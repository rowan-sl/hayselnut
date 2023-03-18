//! ## We choose to ~~go to the moon~~ make a database from scratch not because it is easy, but because they are hard;
//!
//! This module implements a Time-Series DataBase, (tsdb)
//! - featuring lots of (very necessary) complexity
//! - and a custom async on-disk memory allocator
//!
//! benchmarking TBD

use std::{cmp, path::Path};

use chrono::{DateTime, Datelike, Utc};

mod alloc;
mod repr;

use alloc::{errors::AllocErr, Alloc, Ptr};
use repr::Data;
use tracing::{debug, error, instrument};
use zerocopy::Unalign;

use self::{
    alloc::{
        errors::{AllocReqErr, AllocRunnerErr},
        Obj,
    },
    repr::{Day, DayTime, TimeSegment},
};

// TODO: Ctrl+C handler to flush data to disk (and allocated objects)
// also make a write-ahead log or similar to catch unexpected shutdowns and recover gracefully
pub struct DB<T: Data> {
    /// to update this, use `update_head`
    ///
    /// can be null, null=no head (or data in DB)
    cached_head: Ptr<repr::Year<T>>,
    alloc: Alloc,
}

impl<T: Data> DB<T> {
    #[instrument]
    pub async fn open(path: &Path) -> Result<Self, self::Error> {
        let alloc = Alloc::open(path).await?;
        let cached_head = {
            let o = alloc
                .get::<Ptr<repr::Year<T>>>(alloc::entrypoint_pointer())
                .await?;
            *o
        };
        Ok(DB { cached_head, alloc })
    }

    #[instrument(skip(self))]
    pub async fn close(self) -> Result<(), self::Error> {
        debug!("Closing DB");
        self.alloc.close().await?;
        Ok(())
    }

    async fn update_head(&mut self, new_head: Ptr<repr::Year<T>>) -> Result<(), AllocReqErr> {
        *self.alloc.get(alloc::entrypoint_pointer()).await? = new_head;
        self.cached_head = new_head;
        Ok(())
    }

    pub async fn insert<TZ: chrono::TimeZone>(
        &mut self,
        at: DateTime<TZ>,
        record: T,
    ) -> Result<(), self::Error> {
        let at: DateTime<Utc> = at.with_timezone(&Utc);
        // retreive the year entry, creating it if it does not allready exist
        let mut year: Obj<repr::Year<T>> = if self.cached_head.is_null() {
            // TODO: fix this workaround (borrowing self.alloc, then update_head requires a full mutable borrow of self)
            let new_head = Obj::into_ptr(self.alloc.alloc(repr::Year::with_date(at)).await?);
            self.update_head(new_head).await?;
            self.alloc.get(new_head).await?
        } else {
            let head = self.alloc.get(self.cached_head).await?;
            match at.year().cmp(&head.year) {
                cmp::Ordering::Greater => {
                    let mut c_head = head;
                    loop {
                        if c_head.has_next() {
                            let n_head = self.alloc.get(c_head.next).await?;
                            match at.year().cmp(&n_head.year) {
                                cmp::Ordering::Greater => c_head = n_head,
                                cmp::Ordering::Equal => break n_head,
                                cmp::Ordering::Less => {
                                    // c_head is a preivous year, n_head is a following year.
                                    // we create a new year, and insert it in the middle.
                                    let mut m_head =
                                        self.alloc.alloc(repr::Year::with_date(at)).await?;
                                    m_head.next = Obj::get_ptr(&n_head);
                                    c_head.next = Obj::get_ptr(&m_head);
                                }
                            }
                        }
                    }
                }
                cmp::Ordering::Equal => head,
                cmp::Ordering::Less => {
                    let mut new_head = self.alloc.alloc(repr::Year::with_date(at)).await?;
                    new_head.next = Obj::get_ptr(&head);
                    drop(head);
                    let ptr = Obj::into_ptr(new_head);
                    self.update_head(ptr).await?;
                    self.alloc.get(ptr).await?
                }
            }
        };
        // retreive the day, creating it if it does not allready exist
        let t_day = at.ordinal0();

        // find the appropreate time in the day, and insert the record
        let time = DayTime::from_chrono(&at.time());

        if year.days[t_day as usize].is_null() {
            let mut day = repr::Day::new_empty();
            day.len += 1;
            day.entries_time[0] = time;
            day.entries_data[0] = Unalign::new(record);
            let day = self.alloc.alloc(day).await?;
            year.days[t_day as usize] = Obj::into_ptr(day);
        } else {
            // pointer to the previous `c_day`. starts as null
            let mut p_ptr: Ptr<TimeSegment<T>> = Ptr::null();
            // pointer to current day, to update p_ptr with
            let mut c_ptr: Ptr<TimeSegment<T>> = year.days[t_day as usize];
            let mut c_day = self.alloc.get(c_ptr).await?;
            // DO NOT USE continue; in this loop unless you intentionally want to skip the code at the end of it!
            loop {
                match (c_day.next.is_null(), c_day.contains(time)) {
                    (true, None) => {
                        // its free real estate!
                        //
                        // if we have gotten this far, then we can assume that all previous
                        // segments were full or covered too early of a time range, so take this one.
                        c_day.len += 1;
                        c_day.entries_time[0] = time;
                        c_day.entries_data[0] = Unalign::new(record);
                        break;
                    }
                    (false, None) => {
                        // once again, if we are this far than we can assume that all previous segments
                        // are not available
                        let n_day = self.alloc.get(c_day.next).await?;
                        match n_day.contains(time) {
                            None | Some(cmp::Ordering::Greater) | Some(cmp::Ordering::Equal) => {
                                //keep following the trail
                                c_day = n_day;
                            }
                            Some(cmp::Ordering::Less) => {
                                // its free real estate! (next one is too far)
                                c_day.len += 1;
                                c_day.entries_time[0] = time;
                                c_day.entries_data[0] = Unalign::new(record);
                                break;
                            }
                        }
                    }
                    (has_next, Some(cmp::Ordering::Greater)) => {
                        if c_day.full().expect("invalid data in DB") {
                            if !has_next {
                                // create a new object, then continue (will go to the `(true, None)` branch where data is inserted)
                                let n_day = self.alloc.alloc(Day::new_empty()).await?;
                                c_day.next = Obj::get_ptr(&n_day);
                                c_day = n_day;
                            } else {
                                // continue to the next day
                                c_day = self.alloc.get(c_day.next).await?;
                            }
                        } else if has_next {
                            // empty space in this one, but one follows it
                            let n_day = self.alloc.get(c_day.next).await?;
                            match n_day.contains(time) {
                                Some(cmp::Ordering::Less) => {
                                    // one follows, but its time is too large. insert into this one instead
                                    let l = c_day.len as usize;
                                    c_day.entries_time[l] = time;
                                    c_day.entries_data[l] = Unalign::new(record);
                                    c_day.len += 1;
                                    break;
                                }
                                // it fits in in the next one, or otherwise. continue
                                None | Some(cmp::Ordering::Equal | cmp::Ordering::Greater) => {
                                    c_day = n_day
                                }
                            }
                        } else {
                            // there is room, and no following one. just insert into this record (end time will adjust)
                            let l = c_day.len as usize;
                            c_day.entries_time[l] = time;
                            c_day.entries_data[l] = Unalign::new(record);
                            c_day.len += 1;
                            break;
                        }
                    }
                    (_, Some(cmp::Ordering::Equal))
                        if !c_day.full().expect("Invalid data in DB") =>
                    {
                        // it fits, ignore the next one
                        let l = c_day.len as usize;
                        c_day.entries_time[l] = time;
                        c_day.entries_data[l] = Unalign::new(record);
                        c_day.len += 1;
                        break;
                    }
                    (has_next, Some(cmp::Ordering::Equal)) => {
                        // c_day is full
                        if has_next {
                            c_day = self.alloc.get(c_day.next).await?;
                        } else {
                            let n_day = self.alloc.alloc(Day::new_empty()).await?;
                            c_day.next = Obj::get_ptr(&n_day);
                            c_day = n_day;
                        }
                    }
                    (_, Some(cmp::Ordering::Less))
                        if !c_day.full().expect("Invalid data in db") =>
                    {
                        // there is space, but we need to split the data in this one to use it.

                        assert!(c_day.len <= repr::TIMESEG_LEN as u16);
                        let split_idx =
                            c_day.entries_time[..c_day.len as usize].partition_point(|x| x < &time);
                        // shift the elements down (copy_within not used for the second, because it is not copy)
                        let l = c_day.len as usize;
                        c_day.entries_time[..l].copy_within(split_idx.., split_idx + 1);
                        for i in (split_idx..l).rev() {
                            c_day.entries_data[..l + 1].swap(i, i + 1);
                        }
                        // insert
                        c_day.len += 1;
                        c_day.entries_time[split_idx] = time;
                        c_day.entries_data[split_idx] = Unalign::new(record);
                        break;
                    }
                    (_, Some(cmp::Ordering::Less)) => {
                        // no space, insert one between this one and the last one.
                        let mut new = Day::new_empty();
                        new.next = Obj::get_ptr(&c_day);
                        new.len = 1;
                        new.entries_time[0] = time;
                        new.entries_data[0] = Unalign::new(record);
                        let new = self.alloc.alloc(new).await?;
                        // new allready points to c_day as the next
                        if p_ptr.is_null() {
                            // no preivous day, set this one as the first.
                            year.days[t_day as usize] = Obj::get_ptr(&new);
                        } else {
                            // get prev day, set this one in between it and c_day
                            let mut p_day = self.alloc.get(p_ptr).await?;
                            p_day.next = Obj::get_ptr(&new);
                        }
                        break;
                    }
                }
                p_ptr = c_ptr;
                c_ptr = Obj::get_ptr(&c_day);
            }
        }
        Ok(())
    }

    
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Error in allocator: {0:?}")]
    Alloc(#[from] AllocErr),
}

impl From<AllocReqErr> for Error {
    fn from(value: AllocReqErr) -> Self {
        Self::from(AllocErr::from(value))
    }
}

impl From<AllocRunnerErr> for Error {
    fn from(value: AllocRunnerErr) -> Self {
        Self::from(AllocErr::from(value))
    }
}
