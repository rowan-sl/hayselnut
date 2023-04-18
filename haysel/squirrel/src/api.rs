use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use self::station::{
    capabilities::{Channel, ChannelID, ChannelName},
    identity::StationID,
};

pub mod station;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PacketKind {
    Connect(OnConnect),
    // sent to client after it connects.
    // provides mappings of channel names -> uuids
    ChannelMappings(ChannelMappings),
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
