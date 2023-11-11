use bitflags::bitflags;
use zerocopy::{AsBytes, FromBytes, FromZeroes};

use super::ptr::{Ptr, Void};

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
    /// pointer to the next free chunk (null if last)
    /// - dangling if in use
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
    /// the size of the free list (entries, not bytes)
    /// used to make sure that it is read correctly
    pub free_list_size: u64,
    // not shown: `free_list_size` number of AllocCategoryHeaders
}

impl AllocHeader {
    pub fn new(entrypoint: Ptr<Void>, free_list_size: u64) -> Self {
        Self {
            magic_bytes: MAGIC_BYTES,
            _padding: [0u8; 4],
            entrypoint,
            free_list_size,
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
