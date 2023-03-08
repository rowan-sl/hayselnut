use std::{marker::PhantomData, mem::{self, MaybeUninit}, fmt::Debug};
use chrono::{Timelike, NaiveTime, Datelike};
use zerocopy::{FromBytes, AsBytes};
use static_assertions::const_assert_eq;

/// addr=0 is null, just like normal pointers, and invalid.
#[repr(transparent)]
pub struct FPtr<T> {
    pub addr: u64,
    pub _ty: PhantomData<*const T>
}
impl<T> FPtr<T> {
    pub const fn null() -> Self {
        Self { addr: 0, _ty: PhantomData }
    }
    pub const fn with_addr(addr: u64) -> Self {
        Self { addr, _ty: PhantomData }
    }
    pub const fn is_null(self) -> bool {
        self.addr == 0
    }
    pub const fn pointee_size(self) -> usize {
        mem::size_of::<T>()
    }
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
unsafe impl<T> FromBytes for FPtr<T> { fn only_derive_is_allowed_to_implement_this_trait() where Self: Sized {} }
unsafe impl<T> AsBytes for FPtr<T> { fn only_derive_is_allowed_to_implement_this_trait() where Self: Sized {} }


#[repr(C)]
pub struct Year<D: FromBytes + AsBytes + Clone + Copy> {
    pub year: i32,
    pub _pad0: [u8; 4],
    /// can be null, null=no more years
    pub next: FPtr<Year<D>>,
    /// can be null, null=no data for that day
    pub days: [FPtr<Day<D>>; 366],
}
const_assert_eq!(
    mem::size_of::<Year<u128>>(),
    mem::size_of::<i32>()
    +mem::size_of::<[u8; 4]>()
    +mem::size_of::<FPtr<Year<u128>>>()
    +mem::size_of::<[FPtr<Day<u128>>; 366]>()
);
impl<T: FromBytes + AsBytes + Clone + Copy> Year<T> {
    pub fn with_date(date: impl Datelike) -> Self {
        Self { year: date.year(), _pad0: [0; 4], next: FPtr::null(), days: [FPtr::null(); 366] }
    }
    pub fn has_next(&self) -> bool {
        !self.next.is_null()
    }
}
impl<T: FromBytes + AsBytes + Clone + Copy> Debug for Year<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Year")
            .field("days", &self.days)
            .finish()
    }
}
impl<T: FromBytes + AsBytes + Clone + Copy> Clone for Year<T> {
    fn clone(&self) -> Self {
        Self {
            year: self.year,
            _pad0: self._pad0,
            next: self.next,
            days: self.days,
        }
    }
}
unsafe impl<T: FromBytes + AsBytes + Clone + Copy> FromBytes for Year<T> { fn only_derive_is_allowed_to_implement_this_trait() where Self: Sized {} }
unsafe impl<T: FromBytes + AsBytes + Clone + Copy> AsBytes for Year<T> { fn only_derive_is_allowed_to_implement_this_trait() where Self: Sized {} }

pub type Day<D> = TimeSegment<D>;

#[repr(C)]
pub struct TimeSegment<D: FromBytes + AsBytes + Clone + Copy> {
    pub start_time: DayTime,
    pub end_time: DayTime,
    /// can be null, null=no next time segment
    /// there can only be a next time segment
    /// when all entries in this segment are full
    pub next: FPtr<Day<D>>,
    //number of valid entries 
    pub len: u16,
    pub _pad0: [u8; 6],
    //TODO: optimize this quantity 
    //NOTE: D must be copy to make this safe to drop
    /// can be null, null=no more entries, and
    /// once one element is null all following ones must also be null.
    /// 
    /// the position of the first null is determined by `len`
    ///
    /// `[data, data, null, null, null]` <- valid
    ///
    /// `[data, null, null, data, null]` <- invalid
    pub entries_time: [MaybeUninit<DayTime>; 512],
    /// see entries_time
    pub entries_data: [MaybeUninit<zerocopy::Unalign<D>>; 512],
}
impl<T: FromBytes + AsBytes + Clone + Copy> TimeSegment<T> {
    pub fn new_full_day() -> Self {
        Self {
            start_time: DayTime::from_chrono(&NaiveTime::from_hms_opt(0, 0, 0).unwrap()),
            end_time: DayTime::from_chrono(&NaiveTime::from_hms_opt(23, 59, 59).unwrap()),
            next: FPtr::null(),
            len: 0,
            _pad0: [0; 6],
            entries_time: unsafe { MaybeUninit::uninit().assume_init() },
            entries_data: unsafe { MaybeUninit::uninit().assume_init() },
        }
    }
}
unsafe impl<T: FromBytes + AsBytes + Clone + Copy> FromBytes for TimeSegment<T> { fn only_derive_is_allowed_to_implement_this_trait() where Self: Sized {} }
unsafe impl<T: FromBytes + AsBytes + Clone + Copy> AsBytes for TimeSegment<T> { fn only_derive_is_allowed_to_implement_this_trait() where Self: Sized {} }

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
pub struct DayTime {
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

unsafe impl FromBytes for DayTime { fn only_derive_is_allowed_to_implement_this_trait() where Self: Sized {} }
unsafe impl AsBytes for DayTime { fn only_derive_is_allowed_to_implement_this_trait() where Self: Sized {} }


