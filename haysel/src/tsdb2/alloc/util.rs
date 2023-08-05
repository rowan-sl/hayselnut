#[cfg(test)]
pub mod test;

use std::mem::{align_of, size_of};

use self::comptime_hacks::{Condition, IsTrue};
use super::{
    object::Object,
    ptr::{Ptr, Void},
    Allocator, Storage,
};
use zerocopy::{AsBytes, FromBytes};

pub mod comptime_hacks {
    pub struct Condition<const B: bool>;
    pub trait IsTrue {}
    impl IsTrue for Condition<true> {}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CLLIdx {
    pub chunk_num: u64,
    pub chunk_ptr: Ptr<Void>,
    pub data_idx: u64,
}

#[derive(Clone, Copy)]
#[repr(C, align(8))]
pub struct ChunkedLinkedList<const N: usize, T: AsBytes + FromBytes> {
    pub next: Ptr<Self>,
    pub used: u64,
    pub data: [T; N],
}

// const _: &'static dyn IsTrue = &Condition::<
//     {
//         mem::size_of::<Ptr<Void>>()
//             + mem::size_of::<u64>()
//             + mem::size_of::<
//                 [crate::tsdb2::repr::Station; crate::tsdb2::tuning::STATION_MAP_CHUNK_SIZE],
//             >()
//             == mem::size_of::<
//                 ChunkedLinkedList<
//                     { crate::tsdb2::tuning::STATION_MAP_CHUNK_SIZE },
//                     crate::tsdb2::repr::Station,
//                 >,
//             >()
//     },
// >;
//
// this function is a replacement for something like what is above (but in the where clause of ChunkedLinkedList)
// if you actually put this in there (i think it is the fact that the size of self depends on the const generic N)
// it will give a `unconstrained generic constant` error, with a very unhelpfull help message.
//
// this does limit the types that can go in ChunkedLinkedList, but it should work fine
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

    #[instrument(skip(list, alloc, cond))]
    pub async fn find<Store: Storage>(
        list: Ptr<Self>,
        alloc: &mut Allocator<Store>,
        cond: impl Fn(&&T) -> bool,
    ) -> Result<Option<T>, super::error::AllocError<<Store as Storage>::Error>> {
        let mut list = Object::new_read(alloc, list).await?;
        loop {
            if let Some(entry) = list.data[..list.used as usize].iter().find(&cond) {
                let entry = T::read_from(entry.as_bytes()).unwrap();
                list.dispose_immutated();
                break Ok(Some(entry));
            } else if !list.next.is_null() {
                let next = list.next;
                list.dispose_immutated();
                list = Object::new_read(alloc, next).await?;
            } else {
                list.dispose_immutated();
                break Ok(None);
            }
        }
    }

    #[instrument(skip(list, alloc, cond))]
    pub async fn find_best<Store: Storage, C: std::cmp::Ord>(
        list: Ptr<Self>,
        alloc: &mut Allocator<Store>,
        cond: impl Fn(&T) -> Option<C>,
    ) -> Result<Option<(T, CLLIdx)>, super::error::AllocError<<Store as Storage>::Error>> {
        let mut list = Object::new_read(alloc, list).await?;
        let mut chunk_num = 0u64;
        let mut chunk_ptr = list.pointer().cast::<Void>();
        let mut data_idx = 0u64;
        let mut best = None;
        loop {
            if let Some((i, e_cond, entry)) = list.data[..list.used as usize]
                .iter()
                .enumerate()
                .filter_map(|(i, x)| cond(x).map(|c| (i, c, x)))
                .max_by_key(|(_, _, x)| cond(*x))
            {
                let entry = T::read_from(entry.as_bytes()).unwrap();
                if let Some((p_cond, _)) = &best {
                    if p_cond < &e_cond {
                        data_idx = i as u64;
                        best = Some((e_cond, entry));
                    }
                } else {
                    data_idx = i as u64;
                    best = Some((e_cond, entry));
                }
                if !list.next.is_null() {
                    let next = list.next;
                    list.dispose_immutated();
                    list = Object::new_read(alloc, next).await?;
                    chunk_num += 1;
                    chunk_ptr = next.cast::<Void>();
                } else {
                    break;
                }
            } else {
                assert!(list.next.is_null());
                break;
            }
        }
        list.dispose_immutated();
        Ok(best.map(|x| {
            (
                x.1,
                CLLIdx {
                    chunk_num,
                    chunk_ptr,
                    data_idx,
                },
            )
        }))
    }

    #[instrument(skip(list, alloc, item))]
    pub async fn push<Store: Storage>(
        list: Ptr<Self>,
        alloc: &mut Allocator<Store>,
        item: T,
    ) -> Result<(), super::error::AllocError<<Store as Storage>::Error>> {
        let mut list = Object::new_read(alloc, list).await?;
        loop {
            let used = list.used as usize;
            if used < list.data.len() {
                list.data[used] = item;
                list.used += 1;
                list.dispose_sync(alloc).await?;
                break;
            } else if !list.next.is_null() {
                let next = list.next;
                list.dispose_immutated();
                list = Object::new_read(alloc, next).await?;
                continue;
            } else {
                let next = Object::new_alloc(
                    alloc,
                    Self {
                        next: Ptr::null(),
                        used: 1,
                        data: {
                            let mut d = <[T; N]>::new_zeroed();
                            d[0] = item;
                            d
                        },
                    },
                )
                .await?
                .dispose_sync(alloc)
                .await?;
                list.next = next;
                list.dispose_sync(alloc).await?;
                break;
            }
        }
        Ok(())
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
