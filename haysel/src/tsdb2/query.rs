//! Rage Against The State Machine

use mycelium::station::{capabilities::ChannelID, identity::StationID};

use super::{
    alloc::{
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
const fn has_bolth(stage: usize) -> bool {
    stage & HAS_BOLTH == HAS_BOLTH
}
// flags (next _ LSB)
/// - maximum number of values to return -
const FLAG_MAX_NUM: usize = 0b001_00; // 4
const FLAG_AFTER_T: usize = 0b010_00; // 8
const FLAG_BEFORE_T: usize = 0b100_00; // 16
const fn has_flag(stage: usize, flag: usize) -> bool {
    stage & flag == flag
}
// sanity checks
static_assertions::const_assert_eq!(0b011, 3);
static_assertions::const_assert_eq!(0b11, 3);

pub struct QueryBuilder<'a, Store: Storage + Send, const STEP: usize = INITIAL> {
    db: &'a mut Database<Store>,
    station: Option<StationID>,
    channel: Option<ChannelID>,
}

impl<'a, Store: Storage + Send> QueryBuilder<'a, Store, INITIAL> {
    pub(super) fn new(db: &'a mut Database<Store>) -> Self {
        Self {
            db,
            station: None,
            channel: None,
        }
    }
}

mod private {
    use super::*;

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
            db: self.db,
            channel: Some(channel),
            station: self.station,
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
            db: self.db,
            channel: self.channel,
            station: Some(station),
        }
    }
}
