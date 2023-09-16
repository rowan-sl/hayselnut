use bitflags::bitflags;
use zerocopy::{AsBytes, FromBytes, FromZeroes};

use super::{
    ptr::{Ptr, Void},
    tuning,
};

pub const MAGIC_BYTES: [u8; 12] = *b"Hayselnut DB";

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
    pub struct ChunkFlags: u32 {
        const FREE = 0b10000000_00000000_00000000_00000000;
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, FromZeroes, FromBytes, AsBytes)]
#[repr(C)]
pub struct ChunkHeader {
    /// (ChunkFlags)
    pub flags: u32,
    /// length of the chunk
    pub len: u32,
    /// pointer to the previous free chunk (null if first)
    pub prev: Ptr<ChunkHeader>,
    /// pointer to the next free chunk (null if last)
    pub next: Ptr<ChunkHeader>,
}

#[derive(Clone, Copy, FromBytes, FromZeroes, AsBytes)]
#[repr(C)]
pub struct AllocHeader {
    pub magic_bytes: [u8; 12],
    pub _padding: [u8; 4],
    /// entrypoint pointer - pointer to something that can be used to get a frame of
    /// reference to the content stored in the allocator
    pub entrypoint: Ptr<Void>,
    /// space used in the store. this allows for backing to be fixed-size and pre-allocated
    pub used: u64,
    /// the size of the free list (entries, not bytes)
    /// used to make sure that it is read correctly
    pub free_list_size: u64,
    /// NOTE TO THE VIEWER: this has a hard cap to avoid cursed recursion, where the free
    /// list would contain former entries of itself. it generally makes things much nicer.
    /// also, you are unlikely in this scenario to have more than this many types, and if
    /// you do you have other problems.
    ///
    /// unused entries will have the number set to zero.
    pub free_list: [AllocCategoryHeader; tuning!(FREE_LIST_SIZE)],
}

impl AllocHeader {
    pub fn new(entrypoint: Ptr<Void>) -> Self {
        Self {
            magic_bytes: MAGIC_BYTES,
            _padding: [0u8; 4],
            entrypoint,
            used: std::mem::size_of::<Self>() as _,
            free_list_size: tuning!(FREE_LIST_SIZE) as _,
            free_list: <_ as FromZeroes>::new_zeroed(),
        }
    }

    pub fn verify(&self) -> bool {
        self.magic_bytes == MAGIC_BYTES
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, FromZeroes, FromBytes, AsBytes)]
#[repr(C)]
pub struct AllocCategoryHeader {
    pub size: u64,
    pub head: Ptr<ChunkHeader>,
}
