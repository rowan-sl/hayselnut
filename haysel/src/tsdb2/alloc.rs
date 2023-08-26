pub mod disk_store;
pub mod error;
pub mod object;
pub mod ptr;
mod repr;
#[cfg(test)]
mod test;
pub mod util;

use std::{error::Error, mem};

use zerocopy::{AsBytes, FromBytes};

use self::{
    error::AllocError,
    ptr::{Ptr, Void},
    repr::{AllocCategoryHeader, AllocHeader, ChunkFlags, ChunkHeader},
};

mod tuning {
    /// this determines the maximum number of sizes of allocations that can be kept track of.
    pub const FREE_LIST_SIZE: usize = 1024;
}

/// trait that all storage backings for any allocator must implement.
#[async_trait::async_trait(?Send)]
pub trait Storage {
    type Error: Error;
    async fn read_typed<T: FromBytes>(&mut self, at: Ptr<T>) -> Result<T, Self::Error> {
        let mut buf = vec![0; mem::size_of::<T>()];
        self.read_buf(at.cast::<Void>(), buf.len() as u64, &mut buf)
            .await?;
        Ok(T::read_from(buf.as_slice()).unwrap())
    }
    async fn write_typed<T: AsBytes>(&mut self, at: Ptr<T>, from: &T) -> Result<(), Self::Error> {
        self.write_buf(
            at.cast::<Void>(),
            mem::size_of::<T>() as u64,
            from.as_bytes(),
        )
        .await
    }
    async fn read_buf(
        &mut self,
        at: Ptr<Void>,
        amnt: u64,
        into: &mut [u8],
    ) -> Result<(), Self::Error>;
    async fn write_buf(&mut self, at: Ptr<Void>, amnt: u64, from: &[u8])
        -> Result<(), Self::Error>;
    async fn close(self) -> Result<(), Self::Error>;
    async fn size(&mut self) -> Result<u64, Self::Error>;
    async fn expand_by(&mut self, amnt: u64) -> Result<(), Self::Error>;
}

pub struct Allocator<S: Storage> {
    store: S,
}

impl<S: Storage> Allocator<S> {
    #[instrument(skip(store))]
    pub async fn new(
        mut store: S,
        init_overwrite: bool,
    ) -> Result<Self, AllocError<<S as Storage>::Error>> {
        let size = store.size().await?;

        // if the store is empty, it is probably new and needs to be initialized. do that here
        if size < mem::size_of::<AllocHeader>() as _ {
            warn!("store is empty / too small, intitializing a new database");
            let new_header = AllocHeader::new(Ptr::null());
            store.expand_by(mem::size_of_val(&new_header) as _).await?;
            store
                .write_typed(Ptr::<AllocHeader>::with(0u64), &new_header)
                .await?;
        }

        let header = store.read_typed(Ptr::<AllocHeader>::with(0u64)).await?;

        // verify that the magic bytes are here (if they arent, then it is possible that the
        // wrong backing [file] was opened instead of a database, and we do not want to
        // overwrite it.)
        if !header.verify() {
            if init_overwrite {
                warn!("overwriting the current contents to initialize the database");
                // there should be enough space
                let new_header = AllocHeader::new(Ptr::null());
                store
                    .write_typed(Ptr::<AllocHeader>::with(0u64), &new_header)
                    .await?;
            } else {
                error!("it appears that the store in use contains data that is NOT an allocator's (magic bytes are missing) - refusing to continue to avoid any damage");
                info!("if you are attempting to initialize a database in a fixed-size file, use the `--init-overwrite` flag to overwrite the current content instead of throwing this error");
                return Err(AllocError::StoreNotAnAllocator);
            }
        }

        Ok(Self { store })
    }

    /// get the head pointer to the linked list of free spaces of size `size`
    #[instrument(skip(self))]
    async fn free_list_for_size(
        &mut self,
        size: u64,
    ) -> Result<Option<AllocCategoryHeader>, AllocError<<S as Storage>::Error>> {
        // perform simple iteration through the list, finding the right entry and returning it
        let header = self.store.read_typed(Ptr::<AllocHeader>::with(0)).await?;
        Ok(header.free_list.iter().find(|x| x.size == size).copied())
    }

    /// set the head pointer for the linked list of free spaces of size `size`
    /// setting it to null will effectively remove it.
    #[instrument(skip(self))]
    async fn set_free_list_for_size(
        &mut self,
        size: u64,
        to: Ptr<ChunkHeader>,
    ) -> Result<(), AllocError<<S as Storage>::Error>> {
        // perform simple iteration through the list, finding the right entry and modifying it
        let mut header = self.store.read_typed(Ptr::<AllocHeader>::null()).await?;
        // new entry to replace the prev one with.
        // if `to` is null then we set size and head to null, removing it from the list.
        let new_entry = AllocCategoryHeader {
            size: if to.is_null() { 0 } else { size },
            head: to, // checking if it's null would be redundant
        };
        if let Some((entry_idx, _)) = header
            .free_list
            .iter_mut()
            .enumerate()
            .find(|x| x.1.size == size)
        {
            header.free_list[entry_idx] = new_entry;
        } else {
            // find an unused entry
            if let Some((new_entry_idx, _)) = header
                .free_list
                .iter_mut()
                .enumerate()
                .find(|x| x.1.size == 0)
            {
                header.free_list[new_entry_idx] = new_entry;
            } else {
                error!("free list is full! since it is hopefully unlikely to have more than {} unique type sizes, this is probably a bug", tuning::FREE_LIST_SIZE);
                return Err(AllocError::FreeListFull);
            }
        }
        self.store.write_typed(Ptr::null(), &header).await?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn set_entrypoint(
        &mut self,
        to: Ptr<Void>,
    ) -> Result<(), AllocError<<S as Storage>::Error>> {
        let mut header: AllocHeader = self.store.read_typed(Ptr::null()).await?;
        header.entrypoint = to;
        self.store.write_typed(Ptr::null(), &header).await?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn get_entrypoint(&mut self) -> Result<Ptr<Void>, AllocError<<S as Storage>::Error>> {
        Ok(self
            .store
            .read_typed::<AllocHeader>(Ptr::null())
            .await?
            .entrypoint)
    }

    #[instrument(skip(self))]
    pub async fn allocate<T: AsBytes + FromBytes>(
        &mut self,
    ) -> Result<Ptr<T>, AllocError<<S as Storage>::Error>> {
        let allocation_size = mem::size_of::<T>() as u64;
        debug!(
            "alloc {} - {}",
            std::any::type_name::<T>(),
            super::repr::info::sfmt(allocation_size as usize).trim(),
        );
        let ptr = if let Some(free_list) = self.free_list_for_size(allocation_size).await? {
            trace!("free list includes entry for this size");
            // there are free spaces - use them!
            // get the header of the chunk that will eventually be used for the allocated data.
            // it is the first entry in the free list
            let mut first_entry = self.store.read_typed(free_list.head).await?;
            // do some verification of the chunk flags
            let flags = if let Some(flags) = ChunkFlags::from_bits(first_entry.flags) {
                if !flags.contains(ChunkFlags::FREE) {
                    error!("corrupt data: in-use chunk on free list");
                    return Err(AllocError::Corrupt);
                }
                flags
            } else {
                error!("corrupt data: chunk flags contained invalid bits");
                return Err(AllocError::Corrupt);
            };
            // handle swapping the head of the list to the next free space (if there is any)
            if first_entry.next.is_null() {
                // no free spaces, remove it.
                self.set_free_list_for_size(allocation_size, Ptr::null())
                    .await?;
            } else {
                // change the head entry to remove the first element of the list (which is getting
                // used for the newly allocated data)
                // ! note that this does not modify free_list.head here, which already had been read !
                self.set_free_list_for_size(allocation_size, first_entry.next)
                    .await?;
            }
            // unset the `free` flag for the now in-use chunk
            first_entry.flags = (flags ^ ChunkFlags::FREE).bits();
            self.store.write_typed(free_list.head, &first_entry).await?;
            // return a pointer that is after the chunk header for the chunk (this is where the
            // data goes)
            free_list
                .head
                .offset(mem::size_of::<ChunkHeader>() as i64)
                .cast::<T>()
        } else {
            trace!("no free list entry - expanding");
            // no free space - must allocate more.
            // allocate more empty space past the current limit, and use it.
            //
            // this would leave space unused if there is unindexed space at the end of the
            // file, but that hopefully wont happen
            let ptr = {
                // expand the store if needed, otherwise just change `used`
                let expand_by = mem::size_of::<ChunkHeader>() as u64 + allocation_size;
                let mut header = self.store.read_typed(Ptr::<AllocHeader>::null()).await?;
                let ptr = Ptr::with(header.used);
                let size = self.store.size().await?;
                // make up the difference if the store is too small
                if size - header.used < expand_by {
                    let delta = expand_by - (size - header.used);
                    self.store.expand_by(delta).await?;
                }
                header.used += expand_by;
                self.store
                    .write_typed(Ptr::<AllocHeader>::null(), &header)
                    .await?;
                ptr
            };
            // create and write in the new header
            let header = ChunkHeader {
                flags: ChunkFlags::empty().bits(),
                len: allocation_size as u32,
                prev: Ptr::null(),
                next: Ptr::null(),
            };
            self.store.write_typed(ptr, &header).await?;
            // return a pointer after the header
            ptr.offset(mem::size_of::<ChunkHeader>() as i64).cast::<T>()
        };
        debug!(
            "alloc'd {} - {} @ {:#X}",
            std::any::type_name::<T>(),
            super::repr::info::sfmt(allocation_size as usize).trim(),
            ptr.addr,
        );
        Ok(ptr)
    }

    #[instrument(skip(self))]
    async fn validate_pointer<T: AsBytes + FromBytes>(
        &mut self,
        ptr: Ptr<T>,
        free: bool,
    ) -> Result<(), AllocError<<S as Storage>::Error>> {
        let alloc_header = self.store.read_typed(Ptr::<AllocHeader>::null()).await?;
        // get the location of the chunk this pointer points to
        let chunk_loc = ptr
            .offset(-(mem::size_of::<ChunkHeader>() as i64))
            .cast::<ChunkHeader>();
        // verify that that actually *is* a valid chunk
        debug!("-- very inneficient code alert --");
        'validate: {
            let mut c_ptr = Ptr::<ChunkHeader>::with(mem::size_of::<AllocHeader>() as _);
            while c_ptr.addr + (mem::size_of::<ChunkHeader>() as u64) < alloc_header.used {
                if c_ptr == chunk_loc {
                    break 'validate;
                }
                let header = self.store.read_typed(c_ptr).await?;
                c_ptr = c_ptr.offset((mem::size_of::<ChunkHeader>() + header.len as usize) as _);
            }
            error!("attempted to use an invalid pointer");
            return Err(AllocError::PointerInvalid);
        }
        // validate that the chunk matches the pointer
        let header = self.store.read_typed(chunk_loc).await?;
        if header.len != mem::size_of::<T>() as u32 {
            error!(
                "the pointer's data type does not match the data type that was used to allocate it"
            );
            return Err(AllocError::PointerMismatch);
        }
        // and that it's free status matches what is requested
        if let Some(flags) = ChunkFlags::from_bits(header.flags) {
            if flags.contains(ChunkFlags::FREE) != free {
                let map = |stat| if stat { "free" } else { "in use" };
                error!(
                    "expected the pointer to point to {} memory, but it actually points to {} memory",
                    map(free),
                    map(!free)
                );
                return Err(AllocError::PointerStatus);
            }
            flags
        } else {
            error!("corrupt data: chunk flags contains invalid bits");
            return Err(AllocError::Corrupt);
        };
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn free<T: AsBytes + FromBytes>(
        &mut self,
        ptr: Ptr<T>,
    ) -> Result<(), AllocError<<S as Storage>::Error>> {
        debug!(
            "free {} - {} @ {:#X}",
            std::any::type_name::<T>(),
            super::repr::info::sfmt(std::mem::size_of::<T>()).trim(),
            ptr.addr
        );
        self.validate_pointer(ptr, false).await?;
        // get the location of the chunk this pointer points to
        let chunk_loc = ptr
            .offset(-(mem::size_of::<ChunkHeader>() as i64))
            .cast::<ChunkHeader>();
        let header = self.store.read_typed(chunk_loc).await?;
        let Some(flags) = ChunkFlags::from_bits(header.flags) else {
            error!("corrupt data: chunk flags contains invalid bits");
            return Err(AllocError::Corrupt);
        };
        // modify the header data
        let mut new_header = header;
        new_header.flags = (flags | ChunkFlags::FREE).bits();
        // find the free list for this size, and insert the newly freed chunk at the start of it.
        if let Some(free_list) = self.free_list_for_size(mem::size_of::<T>() as _).await? {
            new_header.next = free_list.head;
        } else {
            new_header.next = Ptr::null();
        }
        new_header.prev = Ptr::null();
        // write the now free chunk's header, and the updated free list pointer back to the store
        self.store.write_typed(chunk_loc, &new_header).await?;
        self.set_free_list_for_size(mem::size_of::<T>() as _, chunk_loc)
            .await?;
        // and done!
        return Ok(());
    }

    #[instrument(skip(self))]
    pub async fn read<T: AsBytes + FromBytes>(
        &mut self,
        at: Ptr<T>,
    ) -> Result<T, AllocError<<S as Storage>::Error>> {
        self.validate_pointer(at, false).await?;
        Ok(self.store.read_typed(at).await?)
    }

    #[instrument(skip(self, val))]
    pub async fn write<T: AsBytes + FromBytes>(
        &mut self,
        val: T,
        at: Ptr<T>,
    ) -> Result<(), AllocError<<S as Storage>::Error>> {
        self.validate_pointer(at, false).await?;
        Ok(self.store.write_typed(at, &val).await?)
    }

    #[instrument(skip(self))]
    pub async fn infodump_from(&mut self) -> Result<(), AllocError<<S as Storage>::Error>> {
        use super::repr::info::sfmt;
        let header = self.store.read_typed(Ptr::<AllocHeader>::null()).await?;
        let size = self.store.size().await?;
        info!(
            "Allocator tracking {} / {} (includes space that has been freed)",
            sfmt(header.used as _),
            sfmt(size as _)
        );
        // read out the header, listing the available sizes for data
        {
            let sizes = header
                .free_list
                .iter()
                .filter(|x| x.size != 0)
                .collect::<Vec<_>>();
            info!("allocator contains free spaces of size {sizes:?}");
        }
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn close(self) -> Result<(), AllocError<<S as Storage>::Error>> {
        self.store.close().await?;
        Ok(())
    }
}
