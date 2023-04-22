use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use self::station::{
    capabilities::{Channel, ChannelID, ChannelName, ChannelData},
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
    pub channels: Vec<Channel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMappings {
    pub map: HashMap<ChannelName, ChannelID>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SomeData {
    pub per_channel: HashMap<ChannelID, ChannelData>
}

