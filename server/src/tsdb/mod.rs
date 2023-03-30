//! ## We choose to ~~go to the moon~~ make a database from scratch not because it is easy, but because they are hard;
//!
//! This module implements a Time-Series DataBase, (tsdb)
//! - featuring lots of (very necessary) complexity
//! - and a custom async on-disk memory allocator
//!
//! benchmarking TBD

use std::{cmp, path::Path, fmt::Debug};

use chrono::{DateTime, Datelike, Utc, NaiveDateTime, NaiveDate};

mod alloc;
mod repr;

use alloc::{errors::AllocErr, Alloc, Ptr, NonNull};
use repr::Data;
use serde::Serialize;
use tracing::{debug, warn, error, instrument};
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

    #[instrument(skip(self, at))]
    async fn get_year<TZ: chrono::TimeZone>(
        &mut self,
        at: DateTime<TZ>,
        // create a new entry if one does not exist 
        create_if_missing: bool,
    ) -> Result<Option<NonNull<repr::Year<T>>>, self::Error> {
        let at = at.with_timezone(&Utc);
        // retreive the year entry, creating it if it does not allready exist
        let year: Obj<repr::Year<T>> = if let Some(p_head) = NonNull::new(self.cached_head) {
            let head = self.alloc.get(p_head).await?;
            match at.year().cmp(&head.year) {
                cmp::Ordering::Greater => {
                    let mut c_head = head;
                    loop {
                        if let Some(p_next) = NonNull::new(c_head.next) {
                            let n_head = self.alloc.get(p_next).await?;
                            match at.year().cmp(&n_head.year) {
                                cmp::Ordering::Greater => c_head = n_head,
                                cmp::Ordering::Equal => break n_head,
                                cmp::Ordering::Less if create_if_missing => {
                                    // c_head is a preivous year, n_head is a following year.
                                    // we create a new year, and insert it in the middle.
                                    let mut m_head =
                                        self.alloc.alloc(repr::Year::with_date(at)).await?;
                                    m_head.next = Obj::get_ptr(&n_head).downgrade();
                                    c_head.next = Obj::get_ptr(&m_head).downgrade();
                                    break m_head;
                                }
                                cmp::Ordering::Less => {
                                    return Ok(None);
                                }
                            }
                        } else if create_if_missing {
                            let n_year = self.alloc.alloc(repr::Year::with_date(at)).await?;
                            c_head.next = Obj::get_ptr(&n_year).downgrade();
                            break n_year;
                        } else {
                            return Ok(None);
                        }
                    }
                }
                cmp::Ordering::Equal => head,
                cmp::Ordering::Less if create_if_missing => {
                    let mut new_head = self.alloc.alloc(repr::Year::with_date(at)).await?;
                    new_head.next = Obj::get_ptr(&head).downgrade();
                    drop(head);
                    let ptr = Obj::into_ptr(new_head);
                    self.update_head(ptr.downgrade()).await?;
                    self.alloc.get(ptr).await?
                }
                cmp::Ordering::Less => {
                    return Ok(None);
                }
            }
        } else if create_if_missing {
            // TODO: fix this workaround (borrowing self.alloc, then update_head requires a full mutable borrow of self)
            let new_head = Obj::into_ptr(self.alloc.alloc(repr::Year::with_date(at)).await?);
            self.update_head(new_head.downgrade()).await?;
            self.alloc.get(new_head).await?
        } else {
            return Ok(None);
        };
        Ok(Some(Obj::into_ptr(year)))
    }

    #[instrument(skip(self, at, record))]
    pub async fn insert<TZ: chrono::TimeZone>(
        &mut self,
        at: DateTime<TZ>,
        record: T,
    ) -> Result<(), self::Error> {
        let at: DateTime<Utc> = at.with_timezone(&Utc);
        let year_ptr = self.get_year(at, true).await?.unwrap();
        let mut year = self.alloc.get(year_ptr).await?;

        // retreive the day, creating it if it does not allready exist
        let t_day = at.ordinal0();

        // find the appropreate time in the day, and insert the record
        let time = DayTime::from_chrono(&at);

        if let Some(day_ptr) = NonNull::new(year.days[t_day as usize]) {
            // pointer to the previous `c_day`.
            let mut p_ptr: Option<NonNull<TimeSegment<T>>> = None;
            // pointer to current day, to update p_ptr with
            let mut c_ptr: NonNull<TimeSegment<T>> = day_ptr;
            let mut c_day = self.alloc.get(c_ptr).await?;
            // DO NOT USE continue; in this loop unless you intentionally want to skip the code at the end of it!
            loop {
                match (NonNull::new(c_day.next), c_day.contains(time)) {
                    (None, None) => {
                        // its free real estate!
                        //
                        // if we have gotten this far, then we can assume that all previous
                        // segments were full or covered too early of a time range, so take this one.
                        c_day.len += 1;
                        c_day.entries_time[0] = time;
                        c_day.entries_data[0] = Unalign::new(record);
                        break;
                    }
                    (Some(c_day_next), None) => {
                        // once again, if we are this far than we can assume that all previous segments
                        // are not available
                        let n_day = self.alloc.get(c_day_next).await?;
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
                    (next_opt, Some(cmp::Ordering::Greater)) => {
                        if c_day.full().expect("invalid data in DB") {
                            if let Some(c_day_next) = next_opt {
                                // continue to the next day
                                c_day = self.alloc.get(c_day_next).await?;
                            } else {
                                // create a new object, then continue (will go to the `(true, None)` branch where data is inserted)
                                let n_day = self.alloc.alloc(Day::new_empty()).await?;
                                c_day.next = Obj::get_ptr(&n_day).downgrade();
                                c_day = n_day;
                            }
                        } else if let Some(c_day_next) = next_opt {
                            // empty space in this one, but one follows it
                            let n_day = self.alloc.get(c_day_next).await?;
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
                    (next_opt, Some(cmp::Ordering::Equal)) => {
                        // c_day is full
                        if let Some(c_day_next) = next_opt {
                            c_day = self.alloc.get(c_day_next).await?;
                        } else {
                            let n_day = self.alloc.alloc(Day::new_empty()).await?;
                            c_day.next = Obj::get_ptr(&n_day).downgrade();
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
                        new.next = Obj::get_ptr(&c_day).downgrade();
                        new.len = 1;
                        new.entries_time[0] = time;
                        new.entries_data[0] = Unalign::new(record);
                        let new = self.alloc.alloc(new).await?;
                        // new allready points to c_day as the next
                        if let Some(p_ptr) = p_ptr {
                            // get prev day, set this one in between it and c_day
                            let mut p_day = self.alloc.get(p_ptr).await?;
                            p_day.next = Obj::get_ptr(&new).downgrade();
                        } else {
                            // no preivous day, set this one as the first.
                            year.days[t_day as usize] = Obj::get_ptr(&new).downgrade();
                        } 
                        break;
                    }
                }
                p_ptr = Some(c_ptr);
                c_ptr = Obj::get_ptr(&c_day);
            }
        } else {
            let mut day = repr::Day::new_empty();
            day.len += 1;
            day.entries_time[0] = time;
            day.entries_data[0] = Unalign::new(record);
            let day = self.alloc.alloc(day).await?;
            year.days[t_day as usize] = Obj::into_ptr(day).downgrade();
        }
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn query<TZ: chrono::TimeZone>(
        &mut self, 
        from: DateTime<TZ>,
        to: DateTime<TZ>,
    ) -> Result<Vec<(DateTime<Utc>, T)>, self::Error> where T: Copy {
        let from = from.with_timezone(&Utc);
        let to = to.with_timezone(&Utc);
        debug!("from {from} to {to}");
        if to < from {
            return Err(Error::InvalidDateRange);
        }
        let mut records = vec![];
        for year_num in from.year()..=to.year() {
            if let Some(year_ptr) = self.get_year(from.with_year(year_num).unwrap(), false).await? {
                let day_range = if from.year() == to.year() {
                    // only one iteration will be made
                    from.ordinal0() as usize..=to.ordinal0() as usize
                } else if year_num == from.year() {
                    from.ordinal0() as usize..=365
                } else if year_num == to.year() {
                    0..=to.ordinal0() as usize
                } else {
                    0..=365
                };
                debug!("Query day range {day_range:?} in year {year_num}");

                let year = self.alloc.get(year_ptr).await?;
                let days = &year.days[day_range.clone()];
                for (day_num, day_ptr) in days.iter()
                    .copied()
                    .filter(Ptr::not_null)
                    .map(NonNull::new)
                    .map(Option::unwrap)
                    .enumerate() {
                    let day_num = day_num + day_range.start();
                    debug!("Query day {day_num}");
                    let day = self.alloc.get(day_ptr).await?;
                    let entries = day.filled_entries();
                    debug!("{} filled entries", entries.0.len());
                    entries.0.iter()
                        .enumerate()
                        .map(|(i, t)| {
                            if let Some(t_ch) = t.to_chrono() {
                                let d_ch = NaiveDate::from_yo_opt(year_num, day_num as u32+1).unwrap();
                                let r_time = DateTime::<Utc>::from_utc(NaiveDateTime::new(d_ch, t_ch), Utc);
                                Some((r_time, i))
                            } else {
                                warn!("Invalid time entry");
                                None
                            }
                        })
                        .filter(Option::is_some)
                        .map(Option::unwrap)
                        .filter(|(time, _data_idx)| {
                            debug!("{} < {} < {}", from.time(), time, to.time());
                            from <= *time && time <= &to
                        })
                        .for_each(|(t, i)| records.push((t, entries.1[i].into_inner())));
                }
            }
        }
        Ok(records)
    }

    #[instrument(skip(self))]
    pub async fn debug_structure(&mut self) -> Result<serde_json::Value, self::Error> where T: Copy + Debug + Serialize {
        use serde_json::{Value, Map};
        let mut m = Map::new();
        
        if let Some(cached_head) = NonNull::new(self.cached_head) {
            m.insert("head".into(), cached_head.addr().get().into());
            let mut c_year = self.alloc.get(cached_head).await?;
            loop {
                let mut y_map = Map::new();
                y_map.insert(
                    "year".into(),
                    chrono::NaiveDate::from_ymd_opt(c_year.year,1,1)
                        .map_or_else(|| "invalid".into(), |y| y.format("%Y").to_string().into()) 
                );
                y_map.insert("next".into(), if c_year.has_next() { c_year.next.addr.into() } else { "null".into() });
                let mut days_map = Map::new();
                for (i, y_entry) in c_year.days.iter().enumerate() {
                    if let Some(y_entry) = NonNull::new(*y_entry) {
                        let mut c_day = self.alloc.get(y_entry).await?;
                        let mut segments_map = Map::new();
                        loop {
                            let mut d_map = Map::new();
                            d_map.insert("next".into(), if !c_day.next.is_null() { c_day.next.addr.into() } else { "null".into() });
                            let mut entries_map = Map::new();
                            for d_entry in 0..c_day.len as usize {
                                entries_map.insert(
                                    c_day.entries_time[d_entry]
                                        .to_chrono()
                                        .map_or_else(|| "invalid".into(), |y| y.format("%H:%M:%S UTC").to_string().into()),
                                    serde_json::to_value(c_day.entries_data[d_entry].clone().into_inner())
                                        .unwrap_or("{ < error serializing data > }".into())
                                );
                            } 
                            d_map.insert("entries".into(), entries_map.into());
                            segments_map.insert(Obj::get_ptr(&c_day).addr().get().to_string(), d_map.into());
                            if let Some(next) = NonNull::new(c_day.next) {
                                c_day = self.alloc.get(next).await?;
                            } else {
                                break;
                            }
                        }
                        days_map.insert(
                            chrono::NaiveDate::from_yo_opt(c_year.year, i as u32 + 1)
                                .map_or_else(|| "invalid".into(), |y| y.format("%d-%m-%Y").to_string().into()),
                            segments_map.into()
                        );
                    }
                }
                y_map.insert("days".into(), days_map.into());
                m.insert(Obj::get_ptr(&c_year).addr().to_string(), y_map.into());
                if let Some(next) = NonNull::new(c_year.next) {
                    c_year = self.alloc.get(next).await?;
                } else {
                    break;
                }
            }
        } else {
            m.insert("head".into(), "null".into());
        }
        
        Ok(Value::Object(m))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Error in allocator: {0:?}")]
    Alloc(#[from] AllocErr),
    #[error("Query error: time `to` is before time `from`")]
    InvalidDateRange,
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
