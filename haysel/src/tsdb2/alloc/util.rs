#[cfg(test)]
pub mod test;

use std::mem::{self, align_of, size_of};

use self::comptime_hacks::{Condition, IsTrue};
use super::ptr::{Ptr, Void};
use zerocopy::{AsBytes, FromBytes};

pub mod comptime_hacks {
    pub struct Condition<const B: bool>;
    pub trait IsTrue {}
    impl IsTrue for Condition<true> {}
}

#[derive(Clone, Copy)]
#[repr(C, align(8))]
pub struct ChunkedLinkedList<const N: usize, T: AsBytes + FromBytes> {
    pub next: Ptr<Self>,
    pub used: u64,
    pub data: [T; N],
}

const _: &'static dyn IsTrue = &Condition::<
    {
        mem::size_of::<Ptr<Void>>()
            + mem::size_of::<u64>()
            + mem::size_of::<
                [crate::tsdb2::repr::Station; crate::tsdb2::tuning::STATION_MAP_CHUNK_SIZE],
            >()
            == mem::size_of::<
                ChunkedLinkedList<
                    { crate::tsdb2::tuning::STATION_MAP_CHUNK_SIZE },
                    crate::tsdb2::repr::Station,
                >,
            >()
    },
>;

#[doc(hidden)]
pub const fn works<T>() -> bool {
    align_of::<T>() == 8 && size_of::<T>() % 8 == 0
}

impl<const N: usize, T: AsBytes + FromBytes> ChunkedLinkedList<N, T>
where
    Condition<{ works::<T>() }>: IsTrue,
{
    #[allow(unused)]
    pub fn empty_head() -> Self {
        Self::new_zeroed()
    }
}

// dont tell me what to do
unsafe impl<const N: usize, T: AsBytes + FromBytes> AsBytes for ChunkedLinkedList<N, T>
where
    Condition<{ works::<T>() }>: IsTrue,
{
    fn only_derive_is_allowed_to_implement_this_trait()
    where
        Self: Sized,
    {
    }
}

unsafe impl<const N: usize, T: AsBytes + FromBytes> FromBytes for ChunkedLinkedList<N, T>
where
    Condition<{ works::<T>() }>: IsTrue,
{
    fn only_derive_is_allowed_to_implement_this_trait()
    where
        Self: Sized,
    {
    }
}
