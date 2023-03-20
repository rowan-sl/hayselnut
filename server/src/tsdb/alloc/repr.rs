use zerocopy::{AsBytes, FromBytes};

use crate::tsdb::repr::Data;

use super::{types::ByteBool, Ptr};

/// creates a pointer to access the 'entry point' with.
///
/// this is a allways-present place in memory that can store one pointer,
/// and should be used as a way to store information about where the data
/// that is allocated is for the thing using the allocator.
pub const fn entrypoint_pointer<T: Data>() -> Ptr<Ptr<T>> {
    Ptr {
        addr: 1, /* this is special, the allocator will notice reads of 1 and read from the appropreate place */
        _ph0: std::marker::PhantomData,
    }
}

/// header for an entire backing file.
/// will be placed at addr 0 in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromBytes, AsBytes)]
#[repr(C)]
pub struct Header {
    /// the byte at addr 0 (null pointer).
    pub null_byte: u8,
    pub _pad: [u8; 7],
    /// current address of the bump allocator
    pub alloc_addr: u64,
    /// the provided 'entry point', a place where data can go that
    /// indicates the structure of existing allocations to the program using this.
    ///
    /// any reads to the entrypoint, like all others should have the size of the
    /// read verified using the associated SegHeader.
    ///
    /// data written to the entrypoint should be a pointer (to some other data in the file),
    /// and it will be written to THIS LOCATION in the file (in the main header).
    ///
    /// to read and write to this, use the `entrypoint_pointer` function
    ///
    /// this can be null (no entrypoint specified yet), and can be modified (to point at a new object, allocated normally).
    /// the entrypoint is OPTIONAL to use.
    ///
    /// normal one-acccess-at-a-time rules apply here too!
    pub entrypoint: Ptr<()>,
}

/// Header for a segment
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromBytes, AsBytes)]
#[repr(C)]
pub struct SegHeader {
    /// length of this segment
    pub len_this: u64,
    /// is this segment free
    pub free: ByteBool,
    pub _pad: [u8; 7],
}
