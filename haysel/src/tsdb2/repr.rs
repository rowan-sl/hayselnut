use haysel_macro::Info;
use mycelium::station::{capabilities::ChannelID, identity::StationID};
use num_enum::{IntoPrimitive, TryFromPrimitive};
use zerocopy::{AsBytes, FromBytes};

use super::{
    alloc::{ptr::Ptr, util::ChunkedLinkedList},
    tuning,
};

pub mod info;
pub mod time;

#[derive(Clone, Copy, AsBytes, FromBytes, Info)]
#[repr(C)]
pub struct DBEntrypoint {
    pub stations: MapStations,
    pub tuning_params: TuningParams,
}

#[derive(Clone, Copy, AsBytes, FromBytes, Info, PartialEq, Eq, Debug)]
#[repr(C)]
pub struct TuningParams {
    pub station_map_chunk_size: u64,
    pub channel_map_chunk_size: u64,
    pub data_group_periodic_size: u64,
    pub data_group_sporadic_size: u64,
}

impl TuningParams {
    pub const fn current() -> Self {
        Self {
            station_map_chunk_size: tuning::STATION_MAP_CHUNK_SIZE as _,
            channel_map_chunk_size: tuning::CHANNEL_MAP_CHUNK_SIZE as _,
            data_group_periodic_size: tuning::DATA_GROUP_PERIODIC_SIZE as _,
            data_group_sporadic_size: tuning::DATA_GROUP_SPORADIC_SIZE as _,
        }
    }
}

#[derive(Clone, Copy, AsBytes, FromBytes, Info)]
#[repr(C)]
pub struct MapStations {
    pub map: Ptr<ChunkedLinkedList<{ tuning::STATION_MAP_CHUNK_SIZE }, Station>>,
}

#[derive(Clone, Copy, AsBytes, FromBytes, Info)]
#[repr(C)]
pub struct Station {
    pub id: StationID,
    pub channels: Ptr<ChunkedLinkedList<{ tuning::CHANNEL_MAP_CHUNK_SIZE }, Channel>>,
}

#[derive(Clone, Copy, AsBytes, FromBytes, Info)]
#[repr(C)]
pub struct Channel {
    pub id: ChannelID,
    pub metadata: ChannelMetadata,
    pub _pad: [u8; 7],
    /// Entry Order: most recent first
    pub data: Ptr<DataGroupIndex>,
}

#[derive(Clone, Copy, AsBytes, FromBytes, Info)]
#[repr(C)]
pub struct ChannelMetadata {
    /// DataGroupType, as its primitive value
    pub group_type: u8,
}

#[derive(Clone, Copy, AsBytes, FromBytes, Info)]
#[repr(C)]
pub struct DataGroupIndex {
    /// time that this chunk of data is near (unix time, seconds)
    /// all data must be after OR EQUAL TO this time
    ///
    /// !IMPORTANTLY! all data in this chunk must be from BEFORE the `after` time of the [adjacent entry, closer to the start]. no overlaps allowed
    pub after: i64,
    /// number of entries in use
    pub used: u64, // (only needs to be u16 probably, but this works for alignment reasons)
    /// pointer to the next index in the list
    pub next: Ptr<DataGroupIndex>,
    /// pointer to the rest of the data, which is not needed for indexing.
    /// this alllows the rest of the data (large) to be allocated/loaded only when needed
    pub group: DataGroup,
}

/// Info impl for this is done manually in the impl module, bc it works differently than most other things
#[derive(Clone, Copy, AsBytes, FromBytes)]
#[repr(C)]
pub union DataGroup {
    pub periodic: Ptr<DataGroupPeriodic>,
    pub sporadic: Ptr<DataGroupSporadic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoPrimitive, TryFromPrimitive)]
#[repr(u8)]
pub enum DataGroupType {
    Sporadic,
    Periodic,
}

#[derive(Clone, Copy, AsBytes, FromBytes, Info, Debug)]
#[repr(C)]
pub struct DataGroupPeriodic {
    /// average time between events (seconds)
    pub avg_dt: u32, // u16 is not enough for 1 day
    pub _pad: u16,
    /// delta-times (deveation from the average) (seconds)
    /// the final computed time may not be before `after`
    pub dt: [i16; tuning::DATA_GROUP_PERIODIC_SIZE - 1],
    /// data
    pub data: [f32; tuning::DATA_GROUP_PERIODIC_SIZE - 1],
}

#[derive(Clone, Copy, AsBytes, FromBytes, Info, Debug)]
#[repr(C)]
pub struct DataGroupSporadic {
    /// offset (in seconds) from the start time (can be up to ~136 years, so i think its fine)
    pub dt: [u32; tuning::DATA_GROUP_SPORADIC_SIZE],
    /// data
    pub data: [f32; tuning::DATA_GROUP_SPORADIC_SIZE],
}
