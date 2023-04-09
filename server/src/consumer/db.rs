use std::path::Path;

use anyhow::Result;

use super::{Record, RecordConsumer};
use crate::{station::api::Observations, tsdb::DB};

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
    async fn handle(&mut self, Record { data, recorded_at }: &Record) -> Result<()> {
        self.db.insert(*recorded_at, data.clone()).await?;
        Ok(())
    }

    async fn close(self: Box<Self>) {
        if let Err(e) = self.db.close().await {
            error!("Error shutting down database: {e:#?}");
        }
    }
}
