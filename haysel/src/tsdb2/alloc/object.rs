//! Optional extension to use with allocator that provides objects that can keep track
//! of modifications done to the data within them, and require explicit action to
//! drop them without error, preventing issues where something is created and modified,
//! but then never written back

use std::ops::{Deref, DerefMut};

use zerocopy::{AsBytes, FromBytes};

use super::{error::AllocError, ptr::Ptr, Allocator, Storage};

pub struct Object<T: AsBytes + FromBytes> {
    val: T,
    modified: bool,
    pointer: Ptr<T>,
}

impl<T: AsBytes + FromBytes> Object<T> {
    /// create a new object by allocating space for an existing value.
    pub async fn new_alloc<Store: Storage>(
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
        })
    }

    /// create a new object by reading it from the allocator
    pub async fn new_read<Store: Storage>(
        alloc: &mut Allocator<Store>,
        ptr: Ptr<T>,
    ) -> Result<Self, AllocError<<Store as Storage>::Error>> {
        let read = alloc.read(ptr).await?;
        Ok(Self {
            val: read,
            modified: false,
            pointer: ptr,
        })
    }

    pub fn pointer(&self) -> Ptr<T> {
        self.pointer
    }

    /// dispose of the object, ignoring any changes made (do not sync)
    pub fn dispose_ignore(mut self) {
        self.modified = false;
        drop(self)
    }

    /// dispose of the object, writing it back to the allocator (sync)
    pub async fn dispose_sync<Store: Storage>(
        mut self,
        alloc: &mut Allocator<Store>,
    ) -> Result<(), AllocError<<Store as Storage>::Error>> {
        let copy = T::read_from(self.val.as_bytes()).unwrap();
        alloc.write(copy, self.pointer).await?;
        self.modified = false;
        drop(self);
        Ok(())
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

impl<T: AsBytes + FromBytes> Drop for Object<T> {
    fn drop(&mut self) {
        if self.modified {
            panic!("an object was not propperly handled: this is a bug");
        }
    }
}
