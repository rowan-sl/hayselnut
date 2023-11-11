use std::{fmt::Debug, marker::PhantomData, mem::size_of, ops::Range};

use zerocopy::{AsBytes, FromBytes, FromZeroes};

use super::{BaseOffset, SlicePtr};

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
        Self::with(self.addr.checked_add_signed(by).expect(&format!(
            "offsetting pointer {}u64 by {by}i64 overflowed!",
            self.addr
        )))
    }
    pub fn to_range_usize(self) -> Range<usize> {
        Range {
            start: self.addr as usize,
            end: self.addr as usize + size_of::<T>(),
        }
    }
    pub fn localize_to<'a>(self, base: BaseOffset<'a>, to: &impl SlicePtr<'a>) -> Self {
        // could use offset_from ptr method, but that would have some saftey implications
        let offset_to = (to.ptr() as usize)
            .checked_sub(base.ptr() as usize)
            .unwrap();
        // value from ^ is negative (base - to)
        self.offset(-(offset_to as i64))
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
unsafe impl<T> FromZeroes for Ptr<T> {
    fn only_derive_is_allowed_to_implement_this_trait()
    where
        Self: Sized,
    {
    }
}
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
