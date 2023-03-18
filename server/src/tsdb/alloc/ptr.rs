use std::{marker::PhantomData, mem};

use derivative::Derivative;
use zerocopy::{AsBytes, FromBytes};

#[derive(Derivative)]
#[derivative(Debug(bound = ""), PartialEq(bound = ""), Eq(bound = ""))]
#[repr(transparent)]
pub struct Ptr<T> {
    pub addr: u64,
    pub _ph0: PhantomData<*const T>,
}

impl<T> Ptr<T> {
    pub const fn null() -> Self {
        Self {
            addr: 0,
            _ph0: PhantomData,
        }
    }

    #[allow(dead_code)]
    pub const fn with_addr(addr: u64) -> Self {
        Self {
            addr,
            _ph0: PhantomData,
        }
    }

    pub const fn is_null(self) -> bool {
        self.addr == 0
    }

    pub const fn pointee_size(self) -> usize {
        mem::size_of::<T>()
    }

    #[allow(dead_code)]
    pub const fn cast<U>(self) -> Ptr<U> {
        Ptr {
            addr: self.addr,
            _ph0: PhantomData,
        }
    }
}

impl<T> Clone for Ptr<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Ptr<T> {}

// heheheheheheheh
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
