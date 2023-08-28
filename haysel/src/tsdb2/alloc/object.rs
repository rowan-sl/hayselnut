//! Optional extension to use with allocator that provides objects that can keep track
//! of modifications done to the data within them, and require explicit action to
//! drop them without error, preventing issues where something is created and modified,
//! but then never written back

use std::ops::{Deref, DerefMut};

use zerocopy::{AsBytes, FromBytes};

use super::{error::AllocError, ptr::Ptr, Allocator, Storage};

pub struct Object<T> {
    val: T,
    modified: bool,
    pointer: Ptr<T>,
    // set to true to make it safe to drop
    drop_flag: bool,
}

impl<T: AsBytes + FromBytes + Sync + Send> Object<T> {
    /// create a new object by allocating space for an existing value.
    #[instrument(skip(alloc, val))]
    pub async fn new_alloc<Store: Storage + Send>(
        alloc: &mut Allocator<Store>,
        val: T,
    ) -> Result<Self, AllocError<<Store as Storage>::Error>> {
        let pointer: Ptr<T> = alloc.allocate().await?;
        let val_copy = T::read_from(val.as_bytes()).unwrap();
        alloc.write(val_copy, pointer).await?;
        Ok(Self {
            val,
            modified: false,
            pointer,
            drop_flag: false,
        })
    }

    /// create a new object by reading it from the allocator
    #[instrument(skip(alloc, ptr))]
    pub async fn new_read<Store: Storage + Send>(
        alloc: &mut Allocator<Store>,
        ptr: Ptr<T>,
    ) -> Result<Self, AllocError<<Store as Storage>::Error>> {
        let read = alloc.read(ptr).await?;
        Ok(Self {
            val: read,
            modified: false,
            pointer: ptr,
            drop_flag: true,
        })
    }

    pub fn pointer(&self) -> Ptr<T> {
        self.pointer
    }

    #[instrument(skip(self, alloc))]
    pub async fn sync<Store: Storage + Send>(
        &mut self,
        alloc: &mut Allocator<Store>,
    ) -> Result<(), AllocError<<Store as Storage>::Error>> {
        alloc
            .write(T::read_from(self.val.as_bytes()).unwrap(), self.pointer)
            .await?;
        self.modified = false;
        Ok(())
    }

    /// dispose of the object, ignoring any changes made (do not sync)
    pub fn dispose_ignore(mut self) {
        self.drop_flag = true;
        drop(self)
    }

    /// dispose of the object, verifying no changes were made (syncing was never needed)
    pub fn dispose_immutated(mut self) {
        assert!(
            !self.modified,
            "attempted to use dispose_immutated to dispose of a modified object!"
        );
        self.drop_flag = true;
        drop(self)
    }

    /// dispose of the object, writing it back to the allocator (sync)
    #[instrument(skip(self, alloc))]
    pub async fn dispose_sync<Store: Storage + Send>(
        mut self,
        alloc: &mut Allocator<Store>,
    ) -> Result<Ptr<T>, AllocError<<Store as Storage>::Error>> {
        let copy = T::read_from(self.val.as_bytes()).unwrap();
        alloc.write(copy, self.pointer).await?;
        self.modified = false;
        self.drop_flag = true;
        let ptr = self.pointer;
        drop(self);
        Ok(ptr)
    }
}

impl<T: AsBytes + FromBytes> Deref for Object<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.val
    }
}

impl<T: AsBytes + FromBytes> DerefMut for Object<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.modified = true;
        &mut self.val
    }
}

impl<T> Drop for Object<T> {
    fn drop(&mut self) {
        if !self.drop_flag {
            panic!("an object was not propperly handled before being dropped: this is a bug");
        }
    }
}
