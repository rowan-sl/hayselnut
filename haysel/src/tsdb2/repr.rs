use zerocopy::{AsBytes, FromBytes};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, AsBytes, FromBytes)]
#[repr(C)]
pub struct DBEntrypoint {}
