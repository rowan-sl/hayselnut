use haysel_macro::Info;
use mycelium::station::{capabilities::ChannelID, identity::StationID};
use num_enum::{IntoPrimitive, TryFromPrimitive};
use zerocopy::{AsBytes, FromBytes, FromZeroes};

use super::{
    alloc::{
        ptr::Ptr,
        util::{manual_zerocopy_impl, ChunkedLinkedList},
    },
    c_tuning, tuning,
};

pub mod info;
pub mod time;

#[derive(Clone, Copy, AsBytes, FromZeroes, FromBytes, Info)]
#[repr(C)]
pub struct DBEntrypoint {
    pub stations: MapStations,
    pub tuning_params: TuningParams,
}

#[derive(Clone, Copy, AsBytes, FromZeroes, FromBytes, Info, PartialEq, Eq, Debug)]
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
            station_map_chunk_size: tuning!(STATION_MAP_CHUNK_SIZE) as _,
            channel_map_chunk_size: tuning!(CHANNEL_MAP_CHUNK_SIZE) as _,
            data_group_periodic_size: tuning!(DATA_GROUP_PERIODIC_SIZE) as _,
            data_group_sporadic_size: tuning!(DATA_GROUP_SPORADIC_SIZE) as _,
        }
    }
}

#[derive(Clone, Copy, Info)]
#[repr(C)]
pub struct MapStations {
    pub map: Ptr<ChunkedLinkedList<{ c_tuning::STATION_MAP_CHUNK_SIZE }, Station>>,
}

manual_zerocopy_impl!(MapStations; { true }; ;);

#[derive(Clone, Copy, Info)]
#[repr(C)]
pub struct Station {
    // saftey: Uuid is guarenteed to have the same ABI as [u8; 16]
    pub id: StationID,
    pub channels: Ptr<ChunkedLinkedList<{ c_tuning::CHANNEL_MAP_CHUNK_SIZE }, Channel>>,
}

manual_zerocopy_impl!(Station; { true }; ;);

#[derive(Clone, Copy, Info)]
#[repr(C)]
pub struct Channel {
    // saftey [of zerocopy uuid]: see Station
    pub id: ChannelID,
    pub metadata: ChannelMetadata,
    pub _pad: [u8; 7],
    /// Entry Order: most recent first
    pub data: Ptr<DataGroupIndex>,
}

manual_zerocopy_impl!(Channel; { true }; ;);

#[derive(Clone, Copy, AsBytes, FromZeroes, FromBytes, Info)]
#[repr(C)]
pub struct ChannelMetadata {
    /// DataGroupType, as its primitive value
    pub group_type: u8,
}

#[derive(Clone, Copy, AsBytes, FromZeroes, FromBytes, Info)]
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
#[derive(Clone, Copy, AsBytes, FromZeroes, FromBytes)]
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

#[derive(Clone, Copy, Info, Debug)]
#[repr(C)]
pub struct DataGroupPeriodic {
    /// average time between events (seconds)
    pub avg_dt: u32, // u16 is not enough for 1 day
    pub _pad: u16,
    /// delta-times (deveation from the average) (seconds)
    /// the final computed time may not be before `after`
    pub dt: [i16; c_tuning::DATA_GROUP_PERIODIC_SIZE - 1],
    /// data
    pub data: [f32; c_tuning::DATA_GROUP_PERIODIC_SIZE - 1],
}

manual_zerocopy_impl!(DataGroupPeriodic; { true }; ;);

#[derive(Clone, Copy, Info, Debug)]
#[repr(C)]
pub struct DataGroupSporadic {
    /// offset (in seconds) from the start time (can be up to ~136 years, so i think its fine)
    pub dt: [u32; c_tuning::DATA_GROUP_SPORADIC_SIZE],
    /// data
    pub data: [f32; c_tuning::DATA_GROUP_SPORADIC_SIZE],
}

manual_zerocopy_impl!(DataGroupSporadic; { true }; ;);
