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
    /// pointer to the previous free chunk (null if first)
    pub prev: Ptr<ChunkHeader>,
    /// pointer to the next free chunk (null if last)
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
