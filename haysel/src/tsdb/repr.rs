use super::alloc::Ptr;
use chrono::{DateTime, Datelike, NaiveTime, Timelike, Utc};
use static_assertions::const_assert_eq;
use std::{fmt::Debug, mem};
use zerocopy::{AsBytes, FromBytes, FromZeroes};

pub trait Data: FromBytes + AsBytes + Clone {}
impl<T: FromBytes + AsBytes + Clone> Data for T {}

#[repr(C)]
pub struct Year<D: Data> {
    pub year: i32,
    pub _pad0: [u8; 4],
    /// can be null, null=no more years
    pub next: Ptr<Year<D>>,
    /// can be null, null=no previous year
    pub prev: Ptr<Year<D>>,
    /// can be null, null=no data for that day
    ///
    /// use oridnal0, gives the day starting at 0, to 365
    pub days: [Ptr<Day<D>>; 366],
}
const_assert_eq!(
    mem::size_of::<Year<u128>>(),
    mem::size_of::<i32>()
        + mem::size_of::<[u8; 4]>()
        + mem::size_of::<Ptr<Year<u128>>>() * 2
        + mem::size_of::<[Ptr<Day<u128>>; 366]>()
);
impl<T: Data> Year<T> {
    pub fn with_date(date: impl Datelike, next: Ptr<Year<T>>, prev: Ptr<Year<T>>) -> Self {
        Self {
            year: date.year(),
            _pad0: [0; 4],
            next,
            prev,
            days: [Ptr::null(); 366],
        }
    }

    pub fn has_next(&self) -> bool {
        !self.next.is_null()
    }
}
impl<T: Data> Debug for Year<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Year").field("days", &self.days).finish()
    }
}
impl<T: Data> Clone for Year<T> {
    fn clone(&self) -> Self {
        Self {
            year: self.year,
            _pad0: self._pad0,
            next: self.next,
            prev: self.prev,
            days: self.days,
        }
    }
}
unsafe impl<T: Data> FromZeroes for Year<T> {
    fn only_derive_is_allowed_to_implement_this_trait()
    where
        Self: Sized,
    {
    }
}unsafe impl<T: Data> FromBytes for Year<T> {
    fn only_derive_is_allowed_to_implement_this_trait()
    where
        Self: Sized,
    {
    }
}
unsafe impl<T: Data> AsBytes for Year<T> {
    fn only_derive_is_allowed_to_implement_this_trait()
    where
        Self: Sized,
    {
    }
}

pub type Day<D> = TimeSegment<D>;

pub const TIMESEG_LEN: usize = 512;

//TODO: possible optimization: delta compression of timestamps, and possibly data?
#[repr(C)]
pub struct TimeSegment<D: Data> {
    /// can be null, null=no next time segment
    /// there can only be a next time segment
    /// when all entries in this segment are full
    pub next: Ptr<Day<D>>,
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
    pub entries_time: [DayTime; TIMESEG_LEN],
    /// see entries_time
    pub entries_data: [zerocopy::Unalign<D>; TIMESEG_LEN],
}
impl<T: Data> TimeSegment<T> {
    pub fn new_empty() -> Self {
        Self {
            next: Ptr::null(),
            len: 0,
            _pad0: [0; 6],
            entries_time: FromBytes::new_zeroed(),
            entries_data: FromBytes::new_zeroed(),
        }
    }

    pub fn filled_entries(&self) -> (&[DayTime], &[zerocopy::Unalign<T>]) {
        (
            &self.entries_time[..=self.len as usize - 1],
            &self.entries_data[..=self.len as usize - 1],
        )
    }

    // returns None if the length is invalid
    pub fn full(&self) -> Option<bool> {
        if self.len > TIMESEG_LEN as u16 {
            None?
        }
        Some(self.len == TIMESEG_LEN as u16)
    }

    /// get the start time of the day (assumes sorted order), returning None if self.len==0
    pub fn start_time(&self) -> Option<DayTime> {
        if self.len == 0 {
            None?
        }
        Some(self.entries_time[0])
    }

    /// get the end time of the day (assumes sorted order), returning None if self.len==0
    pub fn end_time(&self) -> Option<DayTime> {
        if self.len == 0 {
            None?
        }
        Some(self.entries_time[self.len as usize - 1])
    }

    /// does this time segment contain the specified time. returns Less if the time is too early,
    /// Equal if it falls in the range, and Greater if it occurs after this range
    ///
    /// assumes that this is in sorted order (it allways should be)
    ///
    /// returns None if this is self.len==0
    pub fn contains(&self, time: DayTime) -> Option<std::cmp::Ordering> {
        Some(if self.start_time()? > time {
            std::cmp::Ordering::Less
        } else if self.end_time()? >= time {
            std::cmp::Ordering::Equal
        } else {
            std::cmp::Ordering::Greater
        })
    }
}
impl<T: Data> Clone for TimeSegment<T> {
    fn clone(&self) -> Self {
        Self {
            next: self.next,
            len: self.len,
            _pad0: self._pad0,
            entries_time: self.entries_time,
            // Saftey: zerocopy::Unalign<T> where T: Data **is** effectively copy (since it implements AsBytes+FromBytes).
            // this effectively copies the data to the new clone
            entries_data: unsafe { std::ptr::read(&self.entries_data as _) },
        }
    }
}
unsafe impl<T: Data> FromZeroes for TimeSegment<T> {
    fn only_derive_is_allowed_to_implement_this_trait()
    where
        Self: Sized,
    {
    }
}
unsafe impl<T: Data> FromBytes for TimeSegment<T> {
    fn only_derive_is_allowed_to_implement_this_trait()
    where
        Self: Sized,
    {
    }
}
unsafe impl<T: Data> AsBytes for TimeSegment<T> {
    fn only_derive_is_allowed_to_implement_this_trait()
    where
        Self: Sized,
    {
    }
}

const_assert_eq!(
    mem::size_of::<TimeSegment<u128>>(),
    mem::size_of::<Ptr<Day<u128>>>()
        + mem::size_of::<u16>()
        + mem::size_of::<[u8; 6]>()
        + mem::size_of::<[DayTime; 512]>()
        + mem::size_of::<[zerocopy::Unalign<u128>; 512]>()
);

/// time of day, in seconds since midnight
///
/// this uses the UTC timezone!
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DayTime {
    secs: u32,
}

impl DayTime {
    pub fn from_chrono(t: &DateTime<Utc>) -> Self {
        Self {
            secs: t.num_seconds_from_midnight(),
        }
    }

    /// get time (in UTC)
    ///
    /// Returns none if `self` contains an invalid number of seconds
    #[allow(dead_code)]
    pub fn to_chrono(self) -> Option<NaiveTime> {
        NaiveTime::from_num_seconds_from_midnight_opt(self.secs, 0)
    }
}
unsafe impl FromZeroes for DayTime {
    fn only_derive_is_allowed_to_implement_this_trait()
    where
        Self: Sized,
    {
    }
}
unsafe impl FromBytes for DayTime {
    fn only_derive_is_allowed_to_implement_this_trait()
    where
        Self: Sized,
    {
    }
}
unsafe impl AsBytes for DayTime {
    fn only_derive_is_allowed_to_implement_this_trait()
    where
        Self: Sized,
    {
    }
}
