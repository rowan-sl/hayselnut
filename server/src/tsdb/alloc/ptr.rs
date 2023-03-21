use std::{marker::PhantomData, mem, num::NonZeroU64};

use derivative::Derivative;
use zerocopy::{AsBytes, FromBytes};

#[derive(Derivative)]
#[derivative(Debug(bound = ""), PartialEq(bound = ""), Eq(bound = ""))]
#[repr(transparent)]
pub struct NonNull<T> {
    addr: NonZeroU64,
    _ph0: PhantomData<*const T>,
}

impl<T> NonNull<T> {
    pub const fn new(ptr: Ptr<T>) -> Option<Self> {
        Some(Self {
            addr: match NonZeroU64::new(ptr.addr) {
                None => return None,
                Some(addr) => addr,
            },
            _ph0: PhantomData
        })
    }

    pub const fn with_addr(addr: NonZeroU64) -> Self {
        Self {
            addr,
            _ph0: PhantomData,
        }
    }

    pub const fn pointee_size(self) -> usize {
        mem::size_of::<T>()
    }

    pub const fn addr(self) -> NonZeroU64 {
        self.addr
    }

    pub const fn downgrade(self) -> Ptr<T> {
        Ptr {
            addr: self.addr.get(),
            _ph0: self._ph0
        }
    }

    pub const fn cast<U>(self) -> NonNull<U> {
        NonNull {
            addr: self.addr,
            _ph0: PhantomData,
        }
    }
}


impl<T> Clone for NonNull<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for NonNull<T> {}

unsafe impl<T> Sync for NonNull<T> {}
unsafe impl<T> Send for NonNull<T> {}

#[derive(Derivative)]
#[derivative(Debug(bound = ""), PartialEq(bound = ""), Eq(bound = ""))]
#[repr(transparent)]
pub struct Ptr<T> {
    pub addr: u64,
    pub _ph0: PhantomData<*const T>,
}

impl<T> Ptr<T> {
    pub const fn new(addr: u64) -> Self {
        Self {
            addr,
            _ph0: PhantomData,
        }
    }

    pub const fn null() -> Self {
        Self {
            addr: 0,
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
