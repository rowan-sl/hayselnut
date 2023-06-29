use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use self::station::{
    capabilities::{Channel, ChannelData, ChannelID, ChannelName},
    identity::StationID,
};

pub mod station;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PacketKind {
    Connect(OnConnect),
    // sent to client after it connects.
    // provides mappings of channel names -> uuids
    ChannelMappings(ChannelMappings),
    Data(SomeData),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnConnect {
    pub station_id: StationID,
    pub station_build_rev: String,
    // chrono rfc3339 timestamp
    pub station_build_date: String,
    pub channels: Vec<Channel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMappings {
    pub map: HashMap<ChannelName, ChannelID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SomeData {
    pub per_channel: HashMap<ChannelID, ChannelData>,
}
