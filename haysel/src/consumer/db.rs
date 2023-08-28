use anyhow::Result;
use mycelium::station::capabilities::ChannelData;

use crate::{
    route::StationInfoUpdate,
    tsdb2::{alloc::Storage, Database},
};

use super::{Record, RecordConsumer};

pub struct RecordDB<S: Storage> {
    db: Database<S>,
}

impl<S: Storage> RecordDB<S> {
    pub async fn new(db: Database<S>) -> Result<Self> {
        Ok(Self { db })
    }
}

#[async_trait]
impl RecordConsumer for RecordDB<crate::tsdb2::alloc::disk_store::DiskStore> {
    async fn handle(
        &mut self,
        Record {
            data,
            recorded_at,
            recorded_by,
        }: &Record,
    ) -> Result<()> {
        for (recorded_from, record) in data {
            let record = match record {
                ChannelData::Float(x) => x,
                ChannelData::Event { .. } => {
                    warn!("Attempted to record event-type data using the database, but that is not supported");
                    continue;
                }
            };
            self.db
                .add_data(*recorded_by, *recorded_from, *recorded_at, *record)
                .await?;
        }
        Ok(())
    }

    async fn update_station_info(&mut self, _updates: &[StationInfoUpdate]) -> Result<()> {
        todo!()
    }

    async fn close(self: Box<Self>) {
        if let Err(e) = self.db.close().await {
            error!("RecordDB: failed to shutdown database: {e:#?}")
        }
    }
}
