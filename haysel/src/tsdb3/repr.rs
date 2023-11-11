use mycelium::station::{capabilities::ChannelID, identity::StationID};
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
    pub stations: [(StationID, Ptr<Station>); 16],
}

#[derive(Debug, Clone, Copy, FromBytes, AsBytes, FromZeroes)]
#[repr(C)]
pub struct Station {
    pub channels: [(ChannelID, Ptr<Channel>); 64],
}

/// entry in a linked list (going from most recent to oldest)
/// entries are (time, data)
/// - data is whatever unit this is using
/// - time is in seconds since 2020 (when this breaks in 2156, I'll be dead)
/// - idx 0->len is oldest->newest (0=old, len=new)
/// - only the first element in the list can have empty parts, which [empty elements] are indicated by a timestamp == EPOCH
/// - once the head fills up, a new empty head is created, with its `next` pointing to the previous head
#[derive(Debug, Clone, Copy, FromBytes, AsBytes, FromZeroes)]
#[repr(C)]
pub struct Channel {
    pub chunk: [(u32, f32); 512],
    pub next: Ptr<Channel>,
}
