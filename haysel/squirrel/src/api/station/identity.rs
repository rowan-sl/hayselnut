//! manages connections to weather stations, station identity, etc;

use serde::{Deserialize, Serialize};
#[cfg(feature = "server-utils")]
use std::collections::HashMap;
use uuid::Uuid;

pub type StationID = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StationInfo {
    pub supports_channels: Vec<super::capabilities::ChannelID>,
}

#[cfg(feature = "server-utils")]
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct KnownStations {
    ids: HashMap<StationID, StationInfo>,
}

#[cfg(feature = "server-utils")]
impl KnownStations {
    pub fn new() -> Self {
        Self {
            ids: HashMap::default(),
        }
    }

    pub fn get_info(&self, id: &StationID) -> Option<&StationInfo> {
        self.ids.get(id)
    }

    pub fn map_info<R, F: FnOnce(&StationID, &mut StationInfo) -> R>(
        &mut self,
        id: &StationID,
        f: F,
    ) -> Option<R> {
        if let Some(inf) = self.ids.get_mut(id) {
            Some(f(id, inf))
        } else {
            None
        }
    }

    pub fn gen_id(&self) -> StationID {
        Uuid::new_v4()
    }

    /// insert station info for a new station, returning Err(new_stations_info) if the info for that station was previously inserted
    pub fn insert_station(
        &mut self,
        id: StationID,
        info: StationInfo,
    ) -> Result<(), (StationID, StationInfo)> {
        if !self.ids.contains_key(&id) {
            self.ids.insert(id, info);
            Ok(())
        } else {
            Err((id, info))
        }
    }

    pub fn stations(&self) -> impl Iterator<Item = &StationID> {
        self.ids.keys()
    }
}
