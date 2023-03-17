use zerocopy::{FromBytes, AsBytes};

#[derive(Clone, Copy, Debug, Default, FromBytes, AsBytes)]
#[repr(transparent)]
pub struct ByteBool(u8);

impl From<bool> for ByteBool {
    fn from(value: bool) -> Self {
        Self(value as u8)
    }
}

impl Into<bool> for ByteBool {
    fn into(self) -> bool {
        self.0 == 0
    }
}

impl PartialEq for ByteBool {
    fn eq(&self, other: &Self) -> bool {
        (self.0 == 0) == (other.0 == 0)
    }
}

impl Eq for ByteBool {}

