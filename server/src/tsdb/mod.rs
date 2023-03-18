//! ## We choose to ~~go to the moon~~ make a database from scratch not because it is easy, but because they are hard;
//!
//! This module implements a Time-Series DataBase, (tsdb)
//! - featuring lots of (very necessary) complexity
//! - and a custom async on-disk memory allocator
//!
//! benchmarking TBD

use std::{path::Path, cmp, mem};

use chrono::{DateTime, Utc, Datelike, NaiveTime};

mod alloc;
mod repr;

use repr::Data;
use alloc::{Ptr, Alloc, errors::AllocErr};
use tracing::{instrument, debug, warn, error};
use zerocopy::Unalign;

use self::{alloc::{errors::{AllocReqErr, AllocRunnerErr}, Obj}, repr::{DayTime, TimeSegment, Day}};

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
            let o = alloc.get::<Ptr<repr::Year<T>>>(alloc::entrypoint_pointer()).await?;
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

    pub async fn insert<TZ: chrono::TimeZone>(&mut self, at: DateTime<TZ>, record: T) -> Result<(), self::Error> {
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
                                    let mut m_head = self.alloc.alloc(repr::Year::with_date(at)).await?;
                                    m_head.next = Obj::get_ptr(&n_head);
                                    c_head.next = Obj::get_ptr(&m_head);
                                }
                            }
                        }
                    }
                },
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
                            },
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
                                None | Some(cmp::Ordering::Equal | cmp::Ordering::Greater) => c_day = n_day
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
                    (_, Some(cmp::Ordering::Equal)) if !c_day.full().expect("Invalid data in DB") => {
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
                    (_, Some(cmp::Ordering::Less)) if !c_day.full().expect("Invalid data in db") => {
                        // there is space, but we need to split the data in this one to use it.
                        
                        assert!(c_day.len <= repr::TIMESEG_LEN as u16);
                        let split_idx = c_day.entries_time[..c_day.len as usize].partition_point(|x| x < &time);
                        // shift the elements down (copy_within not used for the second, because it is not copy)
                        let l = c_day.len as usize;
                        c_day.entries_time[..l].copy_within(split_idx.., split_idx+1);
                        for i in (split_idx..l).rev() {
                            c_day.entries_data[..l + 1].swap(i, i+1); 
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

        // // end time of the previous day, used to validate the start time of the current day (`c_day`)
        // // starts at midnight/0 seconds from midnight, the earliest representable time. to verify
        // // the first segment starts then
        // // let mut l_day_end_t = DayTime::from_chrono(&NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        // let mut c_day: Obj<repr::Day<T>> = if year.days[t_day as usize].is_null() {
        //     self.alloc.alloc(repr::Day::new_full_day()).await?
        // } else {
        //     self.alloc.get(year.days[t_day as usize]).await?
        // };
        // // find the appropreate time in the day, and insert the record
        // let time = DayTime::from_chrono(&at.time());
        // // retreive the exact day segment, then insert the record into it 
        // // if needed, more space in the DB will be created
        // loop {
        //     match c_day.contains(time).unwrap_or(cmp::Ordering::Equal) {
        //         cmp::Ordering::Greater => {
        //             // this time falls past this segment
        //             if c_day.len < repr::TIMESEG_LEN as u16 {
        //                 // space is available
        //                 // TODO: run repairs on database 
        //                 error!("Invalid database layout detectd: time falls outside a segment with empty space\nthis indicates a bug or corrupted DB");
        //                 todo!("The database is currently incapable of fixing this, and the DB may now be in a even more corrupted state than before. DO NOT attempt to run with the same DB again, attempt recovery");
        //                 // return Err(Error::Invalid);
        //
        //                 // this is old incomplete code for doing repairs
        //                 // if c_day.next.is_null() {
        //                 //     // emit a warning about invalid day end time and expand the day.
        //                 //     warn!("Invalid database layout detected: non all-inclusive day, repairing.\n this is probably a bug");
        //                 //     c_day.end_time = DayTime::from_chrono(&NaiveTime::from_hms_opt(23, 59, 59).unwrap());
        //                 //     // try again
        //                 //     continue
        //                 // } else {
        //                 //     // emit a warning about invalid database layout (empty space followed by segment)
        //                 //     // then do the complicated shuffle of fixing this
        //                 // }
        //             } else {
        //                 // this segment is full
        //                 if c_day.next.is_null() {
        //                     // great! create a new segment and
        //                     // cut off the end time of this one, with the entry to the ne w one
        //
        //                     // Saftey: len check performed above (c_day.len < TIMESEG_LEN check)
        //                     c_day.end_time = c_day.entries_time[repr::TIMESEG_LEN-1]; 
        //                     let mut n_day = Day::<T>::new_full_day();
        //                     n_day.start_time = c_day.end_time;
        //                     let n_day = self.alloc.alloc(n_day).await?;
        //                     c_day.next = Obj::get_ptr(&n_day);
        //
        //                     l_day_end_t = c_day.end_time;
        //                     c_day = n_day;
        //                     continue
        //                 } else {
        //                     // check the next segment (l o o p) 
        //                     // make shure to update l_day_end_t
        //                     let n_day = self.alloc.get(c_day.next).await?;
        //                     l_day_end_t = c_day.end_time;
        //                     c_day = n_day;
        //                     continue
        //                 }
        //             }
        //         }
        //         cmp::Ordering::Equal => {
        //             // we found it! 
        //             // insert the record
        //             if c_day.len >= repr::TIMESEG_LEN as u16 {
        //                 // no space!
        //                 if c_day.len > repr::TIMESEG_LEN as u16 {
        //                     error!("Invalid database layout detected: length of segment greater than capacity.\n this indicates a bug or corrupted DB");
        //                     todo!("The database is currently incapable of fixing this, and the DB may now be in a even more corrupted state than before. DO NOT attempt to run with the same DB again, attempt recovery");
        //                     // return Err(Error::Invalid);
        //                 }
        //                 if c_day.next.is_null() {
        //                     // this is the final segment, and it is full.
        //                     // make a new one, link, and insert into new one.
        //                     let mut n_day = Day::<T>::new_full_day();
        //                     n_day.start_time = c_day.end_time;
        //                     n_day.len = 1;
        //                     n_day.entries_time[0] = time;
        //                     n_day.entries_data[0] = Unalign::new(record);
        //                     let n_day = self.alloc.alloc(n_day).await?;
        //                     c_day.next = Obj::get_ptr(&n_day);
        //                     break; 
        //                 } else {
        //                     // this segment is the right one, but it is full.
        //                     // create a new segment, shift all following
        //                     // elements (BEFORE the record time!) into it, 
        //                     // then insert the record after them, then continue shifting elemnts over.
        //                     //
        //                     // this is the slowest possible path (probably), as insertion out of order should not occur very much
        //
        //                     let mut n_day = self.alloc.get(c_day.next).await?;
        //
        //                     if n_day.len > repr::TIMESEG_LEN as u16 {
        //                         error!("Invalid database layout detected: length of segment greater than capacity.\n this indicates a bug or corrupted DB");
        //                         todo!("The database is currently incapable of fixing this, and the DB may now be in a even more corrupted state than before. DO NOT attempt to run with the same DB again, attempt recovery");
        //                         // return Err(Error::Invalid);
        //                     }
        //
        //                     // Saftey:
        //                     // - only reads up to the length of the list (and even if that is invalid, DayTime is valid for any value) 
        //                     let split_idx = n_day.entries_time[..n_day.len as usize].partition_point(|x| x < &time);
        //
        //                     if n_day.len >= repr::TIMESEG_LEN as u16 {
        //                         // no space!
        //                         // create the new day to insert
        //                         let mut i_day = self.alloc.alloc(Day::<T>::new_full_day()).await?;
        //                         i_day.start_time = c_day.end_time;
        //
        //                         // before goes before `record`, after goes after it, 
        //                         // and remaining goes in the first spot in the following day.
        //                         let n_day_tmp = &mut *n_day;
        //                         // let ... else { unreachable!() } only would error if t_remaining mismatched a zero-length slice, which cannot happen
        //                         let (t_before, t_after) = n_day_tmp.entries_time.split_at_mut(split_idx);
        //                         let (t_after, [t_remaining, ..]) = t_after.split_at_mut(t_after.len()-1) else { unreachable!() };
        //                         let (d_before, d_after) = n_day_tmp.entries_data.split_at_mut(split_idx);
        //                         let (d_after, [d_remaining, ..]) = d_after.split_at_mut(d_after.len()-1) else { unreachable!() };
        //
        //                         i_day.entries_time[..split_idx].swap_with_slice(t_before);
        //                         i_day.entries_data[..split_idx].swap_with_slice(d_before);
        //
        //                         i_day.entries_time[split_idx] = time;
        //                         i_day.entries_data[split_idx] = Unalign::new(record);
        //
        //                         i_day.entries_time[split_idx+1..].swap_with_slice(t_after);
        //                         i_day.entries_data[split_idx+1..].swap_with_slice(d_after);
        //
        //                         mem::swap(&mut t_before[0], t_remaining);
        //                         mem::swap(&mut d_before[0], d_remaining);
        //
        //                         i_day.end_time = *i_day.entries_time.last().unwrap();
        //                         n_day.start_time = i_day.end_time;
        //
        //                         // now, we iterate through the rest of the chain pushing elements down it
        //
        //                         todo!()
        //                     } else {
        //                         if !n_day.next.is_null() {
        //                             error!("Invalid database layout detectd: incomplete non-final segment \nthis indicates a bug or corrupted DB");
        //                             todo!("The database is currently incapable of fixing this, and the DB may now be in a even more corrupted state than before. DO NOT attempt to run with the same DB again, attempt recovery");
        //                         }
        //                         // there is space! move remaining data and insert.
        //                         // NO NEW SEGMENT IS CREATED!!!
        //
        //                         // move the data after the split_idx over by one
        //                         // Saftey: n_day.len must be at least 1 less than repr::TIMESEG_LEN
        //                         n_day.len += 1;
        //                         n_day.entries_time.copy_within(split_idx.., split_idx+1);
        //                         unsafe {
        //                             // Saftey:
        //                             // - bounds validated in above index into entries_time with n_day.len,
        //                             // - all data types are valid for any initialized bytes 
        //                             std::ptr::copy(
        //                                 n_day.entries_data.as_ptr().add(split_idx),
        //                                 n_day.entries_data.as_mut_ptr().add(split_idx+1),
        //                                 n_day.len as usize - (split_idx + 1),
        //                             );
        //                         }
        //                         n_day.entries_time[split_idx] = time;
        //                         n_day.entries_data[split_idx] = Unalign::new(record);
        //
        //                         break;
        //                     }
        //                 }
        //             } else {
        //                 // there is space, insert the element
        //                 let new_idx = c_day.len;
        //                 c_day.len += 1;
        //                 c_day.entries_time[new_idx as usize] = time;
        //                 c_day.entries_data[new_idx as usize] = Unalign::new(record);
        //                 break;
        //             }
        //         },
        //         cmp::Ordering::Less => {
        //             // time falls before this segment
        //             warn!("Invalid database layout detected: non all-inclusive day, repairing.\n this is probably a bug");
        //             c_day.start_time = l_day_end_t;
        //             // try again
        //             continue
        //         }
        //     }
        // };
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
