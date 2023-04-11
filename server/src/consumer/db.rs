use std::path::Path;

use anyhow::Result;

use super::{Record, RecordConsumer};
use crate::tsdb::DB;

type Entry = Observations;

pub struct RecordDB {
    db: DB<Entry>,
}

impl RecordDB {
    pub async fn new(path: &Path) -> Result<Self> {
        let db = DB::open(path).await?;
        Ok(Self { db })
    }
}

#[async_trait]
impl RecordConsumer for RecordDB {
    async fn handle(&mut self, Record { data: _, recorded_at: _, recorded_by: _ }: &Record) -> Result<()> {
        todo!("need to make the DB store what station the data was recorded by")
        // self.db.insert(*recorded_at, data.clone()).await?;
        // Ok(())
    }

    async fn close(self: Box<Self>) {
        if let Err(e) = self.db.close().await {
            error!("Error shutting down database: {e:#?}");
        }
    }
}
