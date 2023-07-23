//! # DB Hierarchy
//! {stations(by ID), chunked linked list} -> channels (by ID, so each one has only one type of simple data, and each sub-event has its own place)
//! {channels(by ID), chunked linked list} -> metadata, first data index chunk
//! metadata: data type (ID of it, each one has an associated size but this is stored elsewhere)
//! [data index chunk: (pointers to n-length chunks, each w/ start time, amnt full) and (pointer to the next chunk)]
//! [data chunk: n number of time offset (small uint seconds) from start time, then n number const(for the channel)-sized data]
//!
//! # allocator:
//!
//! type of data being stored: many repeats of things that are the same size (only a handfull of objects, and they are all const-size)
//! - use a linked list allocator design, but have seperate linked lists for each size of data.
//!
//! each allocated part consists of metadata, then the data. meteadata contains
//! - is this chunk free
//! - the length of the chunk
//! - pointer to the previous free chunk (of this size)
//! - pointer to the next free chunk (of this size) or null if there is none
//!
//! alloc header:
//! - [in chunked linked list, or possibly just have a max number of types]: head pointers to the linked list of free data for each size (and the associated size)

pub mod alloc {
    use std::{error::Error, mem};

    use zerocopy::{AsBytes, FromBytes};

    use self::{
        error::AllocError,
        ptr::{Ptr, Void},
        repr::{AllocCategoryHeader, AllocHeader, ChunkFlags, ChunkHeader},
    };

    /// trait that all storage backings for any allocator must implement.
    #[async_trait::async_trait(?Send)]
    pub trait Storage {
        type Error;
        async fn read_typed<T: FromBytes>(&mut self, at: Ptr<T>) -> Result<T, Self::Error> {
            let mut buf = vec![0; mem::size_of::<T>()];
            self.read_buf(at.cast::<Void>(), buf.len() as u64, &mut buf)
                .await?;
            Ok(T::read_from(buf.as_slice()).unwrap())
        }
        async fn write_typed<T: AsBytes>(
            &mut self,
            at: Ptr<T>,
            from: &T,
        ) -> Result<(), Self::Error> {
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
        async fn write_buf(
            &mut self,
            at: Ptr<Void>,
            amnt: u64,
            from: &[u8],
        ) -> Result<(), Self::Error>;
        async fn close(self) -> Result<(), Self::Error>;
        async fn size(&mut self) -> Result<u64, Self::Error>;
        async fn expand_by(&mut self, amnt: u64) -> Result<(), Self::Error>;
    }

    pub struct Allocator<S: Storage>
    where
        <S as Storage>::Error: Error,
    {
        store: S,
    }

    impl<S: Storage> Allocator<S>
    where
        <S as Storage>::Error: Error,
    {
        pub async fn new(mut store: S) -> Result<Self, AllocError<<S as Storage>::Error>> {
            let size = store.size().await?;

            // if the store is empty, it is probably new and needs to be initialized. do that here
            if size < mem::size_of::<AllocHeader>() as _ {
                warn!("store is empty, intitializing a new database");
                let new_header = AllocHeader::new();
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
                error!("it appears that the store in use contains data that is NOT an allocator's (magic bytes are missing) - refusing to continue to avoid any damage");
                return Err(AllocError::StoreNotAnAllocator);
            }

            // read out the header, listing the available sizes for data
            {
                let sizes = header
                    .free_list
                    .iter()
                    .filter(|x| x.size != 0)
                    .collect::<Vec<_>>();
                trace!("allocator contains free spaces of size {sizes:?}");
            }

            Ok(Self { store })
        }

        /// get the head pointer to the linked list of free spaces of size `size`
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

        pub async fn allocate<T: AsBytes + FromBytes>(
            &mut self,
        ) -> Result<Ptr<T>, AllocError<<S as Storage>::Error>> {
            let allocation_size = mem::size_of::<T>() as u64;
            if let Some(free_list) = self.free_list_for_size(allocation_size).await? {
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
                return Ok(free_list
                    .head
                    .offset(mem::size_of::<ChunkHeader>() as i64)
                    .cast::<T>());
            } else {
                // no free space - must allocate more.
                // allocate more empty space past the current limit, and use it.
                //
                // this would leave space unused if there is unindexed space at the end of the
                // file, but that hopefully wont happen
                let ptr = Ptr::with(self.store.size().await?);
                self.store
                    .expand_by(mem::size_of::<ChunkHeader>() as u64 + allocation_size)
                    .await?;
                // create and write in the new header
                let header = ChunkHeader {
                    flags: ChunkFlags::empty().bits(),
                    len: allocation_size as u32,
                    prev: Ptr::null(),
                    next: Ptr::null(),
                };
                self.store.write_typed(ptr, &header).await?;
                // return a pointer after the header
                return Ok(ptr.offset(mem::size_of::<ChunkHeader>() as i64).cast::<T>());
            }
        }
    }

    mod error {
        use std::error::Error;

        #[derive(thiserror::Error, Debug, Clone)]
        pub enum AllocError<E: Error> {
            #[error("error in underlying storage: {0:#?}")]
            StoreError(#[from] E),
            #[error("the data contained in the store given to this allocator is not valid")]
            StoreNotAnAllocator,
            #[error("data in the store is corrupt or misinterpreted")]
            Corrupt,
            #[error("allocator free list has filled up!")]
            FreeListFull,
        }
    }

    mod repr {
        use bitflags::bitflags;
        use zerocopy::{AsBytes, FromBytes};

        use super::{ptr::Ptr, tuning};

        pub const MAGIC_BYTES: [u8; 12] = *b"Hayselnut DB";

        bitflags! {
            #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
            pub struct ChunkFlags: u32 {
                const FREE = 0b10000000_00000000_00000000_00000000;
            }
        }

        #[derive(Clone, Copy, PartialEq, Eq, Debug, FromBytes, AsBytes)]
        #[repr(C)]
        pub struct ChunkHeader {
            /// (ChunkFlags)
            pub flags: u32,
            /// length of the chunk
            pub len: u32,
            /// pointer to the previous free chunk
            pub prev: Ptr<ChunkHeader>,
            /// pointer to the next free chunk
            pub next: Ptr<ChunkHeader>,
        }

        #[derive(Clone, Copy, FromBytes, AsBytes)]
        #[repr(C)]
        pub struct AllocHeader {
            pub magic_bytes: [u8; 12],
            pub _padding: [u8; 4],
            /// NOTE TO THE VIEWER: this has a hard cap to avoid cursed recursion, where the free
            /// list would contain former entries of itself. it generally makes things much nicer.
            /// also, you are unlikely in this scenario to have more than this many types, and if
            /// you do you have other problems.
            ///
            /// unused entries will have the number set to zero.
            pub free_list: [AllocCategoryHeader; tuning::FREE_LIST_SIZE],
        }

        impl AllocHeader {
            pub fn new() -> Self {
                Self {
                    magic_bytes: MAGIC_BYTES,
                    _padding: [0u8; 4],
                    free_list: <_ as FromBytes>::new_zeroed(),
                }
            }

            pub fn verify(&self) -> bool {
                self.magic_bytes == MAGIC_BYTES
            }
        }

        #[derive(Clone, Copy, PartialEq, Eq, Debug, FromBytes, AsBytes)]
        #[repr(C)]
        pub struct AllocCategoryHeader {
            pub size: u64,
            pub head: Ptr<ChunkHeader>,
        }
    }

    mod tuning {
        /// this determines the maximum number of sizes of allocations that can be kept track of.
        pub const FREE_LIST_SIZE: usize = 1024;
    }

    pub mod util {
        use std::mem;

        use self::comptime_hacks::{Condition, IsTrue};
        use super::ptr::Ptr;
        use zerocopy::{AsBytes, FromBytes};

        pub mod comptime_hacks {
            pub struct Condition<const B: bool>;
            pub trait IsTrue {}
            impl IsTrue for Condition<true> {}
        }

        #[derive(Clone, Copy)]
        #[repr(C)]
        pub struct ChunkedLinkedList<const N: usize, T: AsBytes + FromBytes> {
            pub next: Ptr<Self>,
            pub used: u64,
            pub data: [T; N],
        }

        impl<const N: usize, T: AsBytes + FromBytes> ChunkedLinkedList<N, T>
        where
            Condition<
                {
                    mem::size_of::<Ptr<Self>>() + mem::size_of::<u64>() + mem::size_of::<[T; N]>()
                        == mem::size_of::<Self>()
                },
            >: IsTrue,
        {
            pub fn empty_head() -> Self {
                Self::new_zeroed()
            }
        }

        // dont tell me what to do
        unsafe impl<const N: usize, T: AsBytes + FromBytes> AsBytes for ChunkedLinkedList<N, T>
        where
            Condition<
                {
                    mem::size_of::<Ptr<Self>>() + mem::size_of::<u64>() + mem::size_of::<[T; N]>()
                        == mem::size_of::<Self>()
                },
            >: IsTrue,
        {
            fn only_derive_is_allowed_to_implement_this_trait()
            where
                Self: Sized,
            {
            }
        }

        unsafe impl<const N: usize, T: AsBytes + FromBytes> FromBytes for ChunkedLinkedList<N, T>
        where
            Condition<
                {
                    mem::size_of::<Ptr<Self>>() + mem::size_of::<u64>() + mem::size_of::<[T; N]>()
                        == mem::size_of::<Self>()
                },
            >: IsTrue,
        {
            fn only_derive_is_allowed_to_implement_this_trait()
            where
                Self: Sized,
            {
            }
        }
    }

    pub mod ptr {
        use std::{fmt::Debug, marker::PhantomData};

        use zerocopy::{AsBytes, FromBytes};

        pub enum Void {}

        #[repr(transparent)]
        pub struct Ptr<T> {
            pub addr: u64,
            _ph: PhantomData<T>,
        }

        impl<T> Ptr<T> {
            pub fn with(addr: u64) -> Self {
                Self {
                    addr,
                    _ph: PhantomData,
                }
            }
            pub fn cast<U>(self) -> Ptr<U> {
                Ptr {
                    addr: self.addr,
                    _ph: PhantomData,
                }
            }
            pub fn is_null(&self) -> bool {
                self.addr == 0
            }
            pub fn offset(self, by: i64) -> Self {
                Self::with(self.addr.checked_add_signed(by).unwrap())
            }
            pub fn null() -> Self {
                Self::with(0)
            }
        }

        impl<T> Copy for Ptr<T> {}
        impl<T> Clone for Ptr<T> {
            fn clone(&self) -> Self {
                *self
            }
        }
        impl<T> Debug for Ptr<T> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct("Ptr").field("addr", &self.addr).finish()
            }
        }
        impl<T> PartialEq for Ptr<T> {
            fn eq(&self, other: &Self) -> bool {
                self.addr == other.addr
            }
        }
        impl<T> Eq for Ptr<T> {}
        unsafe impl<T> FromBytes for Ptr<T> {
            fn only_derive_is_allowed_to_implement_this_trait()
            where
                Self: Sized,
            {
            }
        }
        unsafe impl<T> AsBytes for Ptr<T> {
            fn only_derive_is_allowed_to_implement_this_trait()
            where
                Self: Sized,
            {
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{
            ptr::{Ptr, Void},
            Allocator, Storage,
        };

        #[tokio::test]
        async fn initializing_allocator_doesnt_crash() {
            let store = TestStore::default();
            Allocator::new(store)
                .await
                .expect("failed to create allocator");
        }

        #[tokio::test]
        async fn allocate_some_stuff() {
            let mut alloc = Allocator::new(TestStore::default())
                .await
                .expect("failed to create allocator");
            alloc.allocate::<[u8; 512]>().await.unwrap();
            alloc.allocate::<[u128; 16]>().await.unwrap();
        }

        #[derive(thiserror::Error, Debug)]
        enum VoidError {}

        #[derive(Default)]
        struct TestStore {
            backing: Vec<u8>,
        }

        #[async_trait::async_trait(?Send)]
        impl Storage for TestStore {
            type Error = VoidError;
            async fn read_buf(
                &mut self,
                at: Ptr<Void>,
                amnt: u64,
                into: &mut [u8],
            ) -> Result<(), Self::Error> {
                into.copy_from_slice(&self.backing[at.addr as _..(at.addr + amnt) as _]);
                Ok(())
            }
            async fn write_buf(
                &mut self,
                at: Ptr<Void>,
                amnt: u64,
                from: &[u8],
            ) -> Result<(), Self::Error> {
                self.backing[at.addr as _..(at.addr + amnt) as _].copy_from_slice(from);
                Ok(())
            }
            async fn close(self) -> Result<(), Self::Error> {
                Ok(())
            }
            async fn size(&mut self) -> Result<u64, Self::Error> {
                Ok(self.backing.len() as _)
            }
            async fn expand_by(&mut self, amnt: u64) -> Result<(), Self::Error> {
                self.backing
                    .extend_from_slice(vec![0; amnt as _].as_slice());
                Ok(())
            }
        }
    }
}
