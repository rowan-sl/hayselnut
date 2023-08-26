use std::collections::HashMap;

use anyhow::Result;
use chrono::{DateTime, Utc};

use mycelium::station::capabilities::{ChannelData, ChannelID};
use squirrel::api::station::identity::StationID;

pub mod db;

#[derive(Debug, Clone)]
pub struct Record {
    pub recorded_at: DateTime<Utc>,
    pub recorded_by: StationID,
    pub data: HashMap<ChannelID, ChannelData>,
}

#[async_trait(?Send)]
pub trait RecordConsumer {
    /// handle an observation record.
    async fn handle(&mut self, record: &Record) -> Result<()>;
    /// perform any necessary shutdown, to allow for async execution before drop();
    async fn close(self: Box<Self>);
}
