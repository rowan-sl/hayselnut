use zerocopy::{AsBytes, FromBytes, FromZeroes};

use super::alloc::Ptr;

/// Midnight, Jan 1 2020 (unix timestamp, seconds)
pub const EPOCH: i64 = 1577836800;

pub const fn unix_to_htime(time: i64) -> Option<u32> {
    if time <= EPOCH {
        return None;
    }
    let diff = time - EPOCH;
    if diff > u32::MAX as i64 {
        None
    } else {
        Some(diff as u32)
    }
}

pub const fn htime_to_unix(htime: u32) -> i64 {
    htime as i64 + EPOCH
}

#[derive(Debug, Clone, Copy, FromBytes, AsBytes, FromZeroes)]
#[repr(C)]
pub struct DBEntrypoint {
    pub stations: MapStations,
    pub tuning_params: TuningParams,
}

#[derive(Debug, Clone, Copy, FromBytes, AsBytes, FromZeroes)]
#[repr(C)]
pub struct TuningParams {
    pub station_map_chunk_size: u64,
    pub channel_map_chunk_size: u64,
}

#[derive(Debug, Clone, Copy, FromBytes, AsBytes, FromZeroes)]
#[repr(C)]
pub struct MapStations {
    /// To indicate the absence of a station, it has a null id and ptr (from_zeroes does this)
    /// - this may not be sparse (all n valid elements must be the first n elements)
    pub stations: [MapStationsElem; 16],
}

#[derive(Debug, Clone, Copy, FromBytes, AsBytes, FromZeroes)]
#[repr(C)]
pub struct MapStationsElem {
    /// StationID
    pub id: uuid::Bytes,
    pub ptr: Ptr<Station>,
}

#[derive(Debug, Clone, Copy, FromBytes, AsBytes, FromZeroes)]
#[repr(C)]
pub struct Station {
    /// to indicate the absence of a channel, it has a null id and ptr (from_zeroes does this)
    /// - this may not be sparse (all n valid elements must be the first n elements)
    pub channels: [MapChannelsElem; 64],
}

#[derive(Debug, Clone, Copy, FromBytes, AsBytes, FromZeroes)]
#[repr(C)]
pub struct MapChannelsElem {
    /// ChannelID
    pub id: uuid::Bytes,
    pub ptr: Ptr<Channel>,
}

#[derive(Debug, Clone, Copy, FromBytes, AsBytes, FromZeroes)]
#[repr(C)]
pub struct Channel {
    pub num_used: u32,
    /// previous data entry time. (htime fmt)
    pub last_time: u32,
    pub data: ChannelData,
}

impl Channel {
    pub fn is_full(&self) -> bool {
        assert!(self.num_used <= self.data.chunk.len() as u32);
        self.num_used == self.data.chunk.len() as u32
    }
}

/// entry in a linked list (going from most recent to oldest)
/// entries are (time, data)
/// - data is whatever unit this is using
/// - time is in seconds since 2020 (when this breaks in 2156, I'll be dead)
/// - idx 0->len is oldest->newest (0=old, len=new)
/// - only the head can have empty elements, the number of non-empty elements is stored in MapChannelsElem
/// - once the head fills up, a new empty head is created, with its `next` pointing to the previous head
#[derive(Debug, Clone, Copy, FromBytes, AsBytes, FromZeroes)]
#[repr(C)]
pub struct ChannelData {
    pub chunk: [DataEntry; 512],
    pub next: Ptr<ChannelData>,
}

#[derive(Debug, Clone, Copy, FromBytes, AsBytes, FromZeroes)]
#[repr(C)]
pub struct DataEntry {
    pub htime: u32,
    pub data: f32,
}
