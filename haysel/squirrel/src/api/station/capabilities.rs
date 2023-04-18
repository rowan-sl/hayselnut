#[cfg(feature = "server-utils")]
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")] //internally tagged
pub enum ChannelValue {
    /// f32 value
    Float,
    /// Event with no data
    Event,
    // in the future this might include something for custom json events?
    // this could be used for the lightning sensor or similar
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")] //internally tagged
pub enum ChannelType {
    /// value can be read at any time, and can be expected to change smoothly over time (EX: temperature, humidity)
    Periodic,
    /// event that occurs based on an external trigger. (EX: lightning)
    /// the fact that the event occured at this time is significant
    Triggered,
}

/// Name of a reading channel (temperature, humidity, lightning, etc)
#[derive(Debug, Clone, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct ChannelName {
    name: String,
}

impl ChannelName {
    pub fn new(name: String) -> Self {
        Self { name }
    }
}

impl<T: ToString> From<T> for ChannelName {
    fn from(value: T) -> Self {
        Self::new(value.to_string())
    }
}

impl Into<String> for ChannelName {
    fn into(self) -> String {
        self.name
    }
}

impl AsRef<String> for ChannelName {
    fn as_ref(&self) -> &String {
        &self.name
    }
}

/// A reading channel's associated information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub name: ChannelName,
    pub value: ChannelValue,
    pub ty: ChannelType,
}

pub type ChannelID = Uuid;

#[cfg(feature = "server-utils")]
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct KnownChannels {
    channels: HashMap<ChannelID, Channel>,
}

#[cfg(feature = "server-utils")]
impl KnownChannels {
    pub fn new() -> Self {
        KnownChannels {
            channels: HashMap::default(),
        }
    }

    pub fn get_channel(&self, id: &ChannelID) -> Option<&Channel> {
        self.channels.get(id)
    }

    pub fn id_by_name(&self, name: &ChannelName) -> Option<ChannelID> {
        self.channels
            .iter()
            .find(|(_, n)| &n.name == name)
            .map(|(id, _)| id.clone())
    }

    /// Returns Err(new_channel) if a channel with the new channels name already exists
    pub fn insert_channel(&mut self, channel: Channel) -> Result<ChannelID, Channel> {
        if self.id_by_name(&channel.name).is_some() {
            Err(channel)
        } else {
            let id = ChannelID::new_v4();
            self.channels.insert(id, channel);
            Ok(id)
        }
    }

    pub fn channels(&self) -> impl Iterator<Item = (&ChannelID, &ChannelName)> {
        self.channels.iter().map(|(k, v)| (k, &v.name))
    }
}
