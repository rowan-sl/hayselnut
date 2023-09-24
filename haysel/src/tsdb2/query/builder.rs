//! Rage Against The State Machine

use chrono::{DateTime, Utc};
use mycelium::station::{capabilities::ChannelID, identity::StationID};

use crate::tsdb2::{
    alloc::{
        store::void::VoidStorage,
        util::comptime_hacks::{Condition, IsTrue},
        Storage,
    },
    Database,
};

// base stages (2 LSB)
const INITIAL: usize = 0b00; // 0
const WITH_STATION: usize = 0b01; // 1
const WITH_CHANNEL: usize = 0b10; // 2
const HAS_BOLTH: usize = 0b11; // 3
                               // flags (next _ LSB)
/// - maximum number of values to return -
const FLAG_MAX_NUM: usize = 0b001_00; // 4
const FLAG_AFTER_T: usize = 0b010_00; // 8
const FLAG_BEFORE_T: usize = 0b100_00; // 16
                                       // final stage. if reached, it means that the struct's contents are verified
const VERIFIED: usize = usize::MAX;
// sanity checks
static_assertions::const_assert_eq!(0b011, 3);
static_assertions::const_assert_eq!(0b11, 3);

pub struct QueryBuilder<'a, Store: Storage + Send, const STEP: usize = INITIAL> {
    pub(super) db: Option<&'a mut Database<Store>>,
    pub(super) station: Option<StationID>,
    pub(super) channel: Option<ChannelID>,
    pub(super) max_results: Option<usize>,
    pub(super) after_time: Option<DateTime<Utc>>,
    pub(super) before_time: Option<DateTime<Utc>>,
}

pub type QueryParams<'a, Store> = QueryBuilder<'a, Store, VERIFIED>;
pub type QueryParamsNoDB = QueryBuilder<'static, VoidStorage, VERIFIED>;

impl QueryParamsNoDB {
    pub fn with_db<'a, Store: Storage + Send>(
        self,
        db: &'a mut Database<Store>,
    ) -> QueryParams<'a, Store> {
        QueryParams {
            db: Some(db),
            station: self.station,
            channel: self.channel,
            max_results: self.max_results,
            after_time: self.after_time,
            before_time: self.before_time,
        }
    }
}

impl QueryBuilder<'static, VoidStorage, INITIAL> {
    pub fn new_nodb() -> Self {
        Self {
            db: None,
            station: None,
            channel: None,
            max_results: None,
            after_time: None,
            before_time: None,
        }
    }
}

impl<'a, Store: Storage + Send> QueryBuilder<'a, Store, INITIAL> {
    pub(in crate::tsdb2) fn new(db: &'a mut Database<Store>) -> Self {
        Self {
            db: Some(db),
            station: None,
            channel: None,
            max_results: None,
            after_time: None,
            before_time: None,
        }
    }
}

impl<'a, const STEP: usize, Store: Storage + Send> QueryBuilder<'a, Store, STEP>
where
    Condition<{ private::has_bolth(STEP) }>: IsTrue,
{
    pub fn verify(self) -> Result<QueryParams<'a, Store>, VerifyError> {
        if self
            .before_time
            .is_some_and(|before| self.after_time.is_some_and(|after| after < before))
        {
            return Err(VerifyError::BeforeAfterAfter);
        }
        Ok(QueryParams {
            ..self.private_into()
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("The provided 'before' time is after the provided 'after' time!")]
    BeforeAfterAfter,
}

impl<'a, const STEP: usize, Store: Storage + Send> QueryBuilder<'a, Store, STEP> {
    fn private_into<const NEW_STEP: usize>(self) -> QueryBuilder<'a, Store, NEW_STEP> {
        let Self {
            db,
            station,
            channel,
            max_results,
            before_time,
            after_time,
        } = self;
        QueryBuilder {
            db,
            station,
            channel,
            max_results,
            before_time,
            after_time,
        }
    }
}

mod private {
    use super::*;

    pub const fn has_bolth(stage: usize) -> bool {
        stage & HAS_BOLTH == HAS_BOLTH
    }

    pub const fn has_flag(stage: usize, flag: usize) -> bool {
        stage & flag == flag
    }

    pub const fn with_channel_out_func(step: usize) -> usize {
        if step == WITH_STATION {
            HAS_BOLTH
        } else {
            WITH_CHANNEL
        }
    }

    pub const fn with_station_out_func(step: usize) -> usize {
        if step == WITH_CHANNEL {
            HAS_BOLTH
        } else {
            WITH_STATION
        }
    }
}

impl<'a, const STEP: usize, Store: Storage + Send> QueryBuilder<'a, Store, STEP>
where
    Condition<{ (STEP == WITH_STATION) | (STEP == INITIAL) }>: IsTrue,
{
    pub fn with_channel(
        self,
        channel: ChannelID,
    ) -> QueryBuilder<'a, Store, { private::with_channel_out_func(STEP) }> {
        debug_assert!(self.channel.is_none());
        QueryBuilder {
            channel: Some(channel),
            ..self.private_into()
        }
    }
}

impl<'a, const STEP: usize, Store: Storage + Send> QueryBuilder<'a, Store, STEP>
where
    Condition<{ (STEP == WITH_CHANNEL) | (STEP == INITIAL) }>: IsTrue,
{
    pub fn with_station(
        self,
        station: StationID,
    ) -> QueryBuilder<'a, Store, { private::with_station_out_func(STEP) }> {
        debug_assert!(self.station.is_none());
        QueryBuilder {
            station: Some(station),
            ..self.private_into()
        }
    }
}

impl<'a, const STEP: usize, Store: Storage + Send> QueryBuilder<'a, Store, STEP>
where
    Condition<{ private::has_bolth(STEP) & !private::has_flag(STEP, FLAG_MAX_NUM) }>: IsTrue,
{
    /// limits the maximum number of responses to return (RECOMMENDED)
    pub fn with_max_results(
        self,
        max_results: usize,
    ) -> QueryBuilder<'a, Store, { STEP | FLAG_MAX_NUM }> {
        debug_assert!(self.max_results.is_none());
        QueryBuilder {
            max_results: Some(max_results),
            ..self.private_into()
        }
    }
}

impl<'a, const STEP: usize, Store: Storage + Send> QueryBuilder<'a, Store, STEP>
where
    Condition<{ private::has_bolth(STEP) & !private::has_flag(STEP, FLAG_AFTER_T) }>: IsTrue,
{
    /// sets the time that results must be after
    pub fn with_after(
        self,
        after: DateTime<Utc>,
    ) -> QueryBuilder<'a, Store, { STEP | FLAG_AFTER_T }> {
        debug_assert!(self.after_time.is_none());
        QueryBuilder {
            after_time: Some(after),
            ..self.private_into()
        }
    }
}

impl<'a, const STEP: usize, Store: Storage + Send> QueryBuilder<'a, Store, STEP>
where
    Condition<{ private::has_bolth(STEP) & !private::has_flag(STEP, FLAG_BEFORE_T) }>: IsTrue,
{
    /// sets the time that results must be before
    pub fn with_before(
        self,
        before: DateTime<Utc>,
    ) -> QueryBuilder<'a, Store, { STEP | FLAG_BEFORE_T }> {
        debug_assert!(self.before_time.is_none());
        QueryBuilder {
            before_time: Some(before),
            ..self.private_into()
        }
    }
}
