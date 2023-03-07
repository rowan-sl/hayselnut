use std::{marker::PhantomData, mem::{self, MaybeUninit}, fmt::Debug};
use chrono::{Timelike, NaiveTime};
use zerocopy::{FromBytes, AsBytes};
use static_assertions::const_assert_eq;

/// addr=0 is null, just like normal pointers, and invalid.
#[repr(transparent)]
struct FPtr<T> {
    addr: u64,
    _ty: PhantomData<*const T>
}
impl<T> Debug for FPtr<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FPtr")
            .field("addr", &self.addr)
            .finish()
    }
}
impl<T> Clone for FPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for FPtr<T> {}

#[repr(transparent)]
struct Year<D: FromBytes + AsBytes> {
    /// can be null, null=no data for that day
    days: [FPtr<Day<D>>; 366],
}
impl<T: FromBytes + AsBytes> Debug for Year<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Year")
            .field("days", &self.days)
            .finish()
    }
}
impl<T: FromBytes + AsBytes> Clone for Year<T> {
    fn clone(&self) -> Self {
        Self { days: self.days.clone() }
    }
}

type Day<D> = TimeSegment<D>;

#[repr(C)]
struct TimeSegment<D: FromBytes + AsBytes> {
    start_time: DayTime,
    end_time: DayTime,
    /// can be null, null=no next time segment
    /// there can only be a next time segment
    /// when all entries in this segment are full
    next: FPtr<Day<D>>,
    //number of valid entries 
    len: u16,
    _pad0: [u8; 6],
    //TODO: optimize this quantity 
    /// can be null, null=no more entries, and
    /// once one element is null all following ones must also be null.
    /// 
    /// the position of the first null is determined by `len`
    ///
    /// `[data, data, null, null, null]` <- valid
    ///
    /// `[data, null, null, data, null]` <- invalid
    entries_time: [MaybeUninit<DayTime>; 512],
    /// see entries_time
    entries_data: [MaybeUninit<zerocopy::Unalign<D>>; 512],
} 

const_assert_eq!(
    mem::size_of::<TimeSegment<u128>>(), 
    mem::size_of::<DayTime>()*2
    +mem::size_of::<FPtr<Day<u128>>>()
    +mem::size_of::<u16>()
    +mem::size_of::<[u8; 6]>()
    +mem::size_of::<[MaybeUninit<DayTime>; 512]>()
    +mem::size_of::<[MaybeUninit<zerocopy::Unalign<u128>>; 512]>()
);

// time of day, in seconds since midnight
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct DayTime {
    secs: u32
}

impl DayTime {
    pub fn from_chrono<T: Timelike>(t: &T) -> Self {
        Self {
            secs: t.num_seconds_from_midnight(),
        }
    }

    /// Returns none if `self` contains an invalid number of seconds
    pub fn to_chrono(self) -> Option<NaiveTime> {
        NaiveTime::from_num_seconds_from_midnight_opt(self.secs, 0)
    }
}

