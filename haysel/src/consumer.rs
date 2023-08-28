use std::collections::HashMap;

use anyhow::Result;
use chrono::{DateTime, Utc};

use mycelium::station::capabilities::{ChannelData, ChannelID};
use squirrel::api::station::identity::StationID;

use crate::route::StationInfoUpdate;

pub mod db;
pub mod ipc;

#[derive(Debug, Clone)]
pub struct Record {
    pub recorded_at: DateTime<Utc>,
    pub recorded_by: StationID,
    pub data: HashMap<ChannelID, ChannelData>,
}

#[async_trait]
pub trait RecordConsumer {
    /// handle an observation record.
    async fn handle(&mut self, record: &Record) -> Result<()>;
    /// handle updates to the station/channel lists
    async fn update_station_info(&mut self, updates: &[StationInfoUpdate]) -> Result<()>;
    /// perform any necessary shutdown, to allow for async execution before drop();
    async fn close(self: Box<Self>);
}
