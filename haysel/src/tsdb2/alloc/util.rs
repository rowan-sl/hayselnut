#[cfg(test)]
pub mod test;

use std::mem;

use self::comptime_hacks::{Condition, IsTrue};
use super::ptr::Ptr;
use zerocopy::{AsBytes, FromBytes};

pub mod comptime_hacks {
    pub struct Condition<const B: bool>;
    pub trait IsTrue {}
    impl IsTrue for Condition<true> {}
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct ChunkedLinkedList<const N: usize, T: AsBytes + FromBytes> {
    pub next: Ptr<Self>,
    pub used: u64,
    pub data: [T; N],
}

impl<const N: usize, T: AsBytes + FromBytes> ChunkedLinkedList<N, T>
where
    Condition<
        {
            mem::size_of::<Ptr<Self>>() + mem::size_of::<u64>() + mem::size_of::<[T; N]>()
                == mem::size_of::<Self>()
        },
    >: IsTrue,
{
    #[allow(unused)]
    pub fn empty_head() -> Self {
        Self::new_zeroed()
    }
}

// dont tell me what to do
unsafe impl<const N: usize, T: AsBytes + FromBytes> AsBytes for ChunkedLinkedList<N, T>
where
    Condition<
        {
            mem::size_of::<Ptr<Self>>() + mem::size_of::<u64>() + mem::size_of::<[T; N]>()
                == mem::size_of::<Self>()
        },
    >: IsTrue,
{
    fn only_derive_is_allowed_to_implement_this_trait()
    where
        Self: Sized,
    {
    }
}

unsafe impl<const N: usize, T: AsBytes + FromBytes> FromBytes for ChunkedLinkedList<N, T>
where
    Condition<
        {
            mem::size_of::<Ptr<Self>>() + mem::size_of::<u64>() + mem::size_of::<[T; N]>()
                == mem::size_of::<Self>()
        },
    >: IsTrue,
{
    fn only_derive_is_allowed_to_implement_this_trait()
    where
        Self: Sized,
    {
    }
}
