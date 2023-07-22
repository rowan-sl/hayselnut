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
        repr::AllocHeader,
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
                let mut list = header.free_by_size;
                let mut sizes = vec![];
                loop {
                    sizes.extend_from_slice(&list.data[0..list.used as _]);
                    if list.next.is_null() {
                        break;
                    }
                    list = store.read_typed(list.next).await?;
                }
                trace!("allocator contains free spaces of size {sizes:?}");
            }

            Ok(Self { store })
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
        }
    }

    mod repr {
        use bitflags::bitflags;
        use zerocopy::{AsBytes, FromBytes};

        use super::{ptr::Ptr, tuning, util::ChunkedLinkedList};

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
            /// pointer to the previous free chunk (garbage if this chunk is in use, check the flags field)
            pub prev: Ptr<ChunkHeader>,
            /// pointer to the next free chunk (also garbage if this chunk is in use)
            pub next: Ptr<ChunkHeader>,
        }

        #[derive(Clone, Copy, FromBytes, AsBytes)]
        #[repr(C)]
        pub struct AllocHeader {
            pub magic_bytes: [u8; 12],
            pub _padding: [u8; 4],
            pub free_by_size:
                ChunkedLinkedList<{ tuning::ALLOC_HEADER_LIST_CHUNK_SIZE }, AllocCategoryHeader>,
        }

        impl AllocHeader {
            pub fn new() -> Self {
                Self {
                    magic_bytes: MAGIC_BYTES,
                    _padding: [0u8; 4],
                    free_by_size: ChunkedLinkedList::empty_head(),
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
            pub head: u64,
        }
    }

    mod tuning {
        pub const ALLOC_HEADER_LIST_CHUNK_SIZE: usize = 8;
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
}
