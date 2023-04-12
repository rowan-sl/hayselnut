use anyhow::Result;
use chrono::{DateTime, Utc};

use crate::station::identity::StationID;

// re-add when fixed
//pub mod db;

#[derive(Debug, Clone)]
pub struct Record {
    // TODO: re-add data fields, using new system
    // pub data: Observations,
    pub recorded_at: DateTime<Utc>,
    pub recorded_by: StationID,
}

#[async_trait]
pub trait RecordConsumer {
    /// handle an observation record.
    async fn handle(&mut self, record: &Record) -> Result<()>;
    /// perform any necessary shutdown, to allow for async execution before drop();
    async fn close(self: Box<Self>);
}
