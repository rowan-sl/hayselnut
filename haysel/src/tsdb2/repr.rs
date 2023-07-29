use mycelium::station::{capabilities::ChannelID, identity::StationID};
use num_enum::IntoPrimitive;
use zerocopy::{AsBytes, FromBytes};

use super::{
    alloc::{ptr::Ptr, util::ChunkedLinkedList},
    tuning,
};

pub mod info;

#[derive(Clone, Copy, AsBytes, FromBytes)]
#[repr(C)]
pub struct DBEntrypoint {
    pub stations: MapStations,
}

#[derive(Clone, Copy, AsBytes, FromBytes)]
#[repr(C)]
pub struct MapStations {
    pub map: Ptr<ChunkedLinkedList<{ tuning::STATION_MAP_CHUNK_SIZE }, Station>>,
}

#[derive(Clone, Copy, AsBytes, FromBytes)]
#[repr(C)]
pub struct Station {
    pub id: StationID,
    pub channels: Ptr<ChunkedLinkedList<{ tuning::CHANNEL_MAP_CHUNK_SIZE }, Channel>>,
}

#[derive(Clone, Copy, AsBytes, FromBytes)]
#[repr(C)]
pub struct Channel {
    pub id: ChannelID,
    pub metadata: ChannelMetadata,
    pub _pad: [u8; 7],
    pub data: ChunkedLinkedList<{ tuning::DATA_INDEX_CHUNK_SIZE }, DataGroupIndex>,
}

#[derive(Clone, Copy, AsBytes, FromBytes)]
#[repr(C)]
pub struct ChannelMetadata {
    /// DataGroupType, as its primitive value
    pub group_type: u8,
}

#[derive(Clone, Copy, AsBytes, FromBytes)]
#[repr(C)]
pub struct DataGroupIndex {
    /// time that this chunk of data is near (unix time, seconds)
    /// all data must be after this time
    pub after: u64,
    /// pointer to the rest of the data, which is not needed for indexing.
    /// this alllows the rest of the data (large) to be allocated/loaded only when needed
    pub ptr: Ptr<DataGroup>,
}

#[derive(Clone, Copy, AsBytes, FromBytes)]
#[repr(C)]
pub union DataGroup {
    pub periodic: Ptr<DataGroupPeriodic>,
    pub sporadic: Ptr<DataGroupSporadic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoPrimitive)]
#[repr(u8)]
pub enum DataGroupType {
    Sporadic,
    Periodic,
}

#[derive(Clone, Copy, AsBytes, FromBytes)]
#[repr(C)]
pub struct DataGroupPeriodic {
    /// average time between events (seconds)
    pub avg_dt: u32, // u16 is not enough for 1 day
    /// number of entries in use
    pub used: u16,
    /// delta-times (deveation from the average) (seconds)
    /// the final computed time may not be before `after`
    pub dt: [i16; tuning::DATA_GROUP_PERIODIC_SIZE - 1],
    /// data
    pub data: [f32; tuning::DATA_GROUP_PERIODIC_SIZE - 1],
}

#[derive(Clone, Copy, AsBytes, FromBytes)]
#[repr(C)]
pub struct DataGroupSporadic {
    /// offset (in seconds) from the start time (can be up to ~136 years, so i think its fine)
    pub dt: [u32; tuning::DATA_GROUP_SPORADIC_SIZE],
    /// data
    pub data: [f32; tuning::DATA_GROUP_SPORADIC_SIZE],
}
