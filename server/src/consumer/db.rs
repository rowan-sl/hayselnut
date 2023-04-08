use std::path::Path;

use anyhow::Result;

use crate::tsdb::DB;
use super::{Record, RecordConsumer};

type Entry = Record;

pub struct RecordDB {
    db: DB<Entry>,
}

impl RecordDB {
    pub async fn new(path: &Path) -> Result<Self> {
        todo!()
    }
}

#[async_trait]
impl RecordConsumer for RecordDB {
    async fn handle(&mut self, _record: &Record) -> Result<()> {
        todo!()
    }
    async fn close(&mut self) {
        todo!()
    }
}
