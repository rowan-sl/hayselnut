use anyhow::Result;

pub mod db;

type Record = super::api::Observations;

#[async_trait]
pub trait RecordConsumer {
    /// handle an observation record.
    async fn handle(&mut self, record: &Record) -> Result<()>;
    /// perform any necessary shutdown, to allow for async execution before drop();
    async fn close(&mut self);
}
