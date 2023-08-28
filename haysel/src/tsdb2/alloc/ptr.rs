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
unsafe impl<T> Sync for Ptr<T> {}
unsafe impl<T> Send for Ptr<T> {}
