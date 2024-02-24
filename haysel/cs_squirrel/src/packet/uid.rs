use zerocopy::{network_endian::U32, AsBytes, FromBytes, FromZeroes, Unalign};

/// unique (NOT GLOBALLY!) ID
///
/// this ID is only expected to be unique for *long enough* for network communication (e.g. the lifetime of a UDP packet.)
/// practically this means that the ID will reset every time the weather station resets, for example
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, FromZeroes, FromBytes, AsBytes)]
#[repr(C, align(1))]
pub struct Uid(Unalign<U32>);

impl Uid {
    pub const fn null() -> Self {
        Self(Unalign::new(U32::ZERO))
    }
}

/// sequential UID generator
#[derive(Debug, Default)]
pub struct Seq {
    current: u32,
}

impl Seq {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn next(&mut self) -> Uid {
        // uid 0 is null
        self.current = self.current.wrapping_add(1);
        Uid(Unalign::new(U32::new(self.current)))
    }
}
