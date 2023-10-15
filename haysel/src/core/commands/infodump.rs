//! print info on the database structure (in general, or optionally of the db in `file`)

use std::path::PathBuf;

use anyhow::Result;

use crate::tsdb2::{
    alloc::store::disk::{DiskMode, DiskStore},
    Database,
};

pub async fn main(file: Option<PathBuf>, mode: DiskMode) -> Result<()> {
    warn!(" -------- dumping database info --------");
    Database::<DiskStore>::infodump().await;
    if let Some(file) = file {
        warn!(" -------- dumping database info for {file:?} --------");
        let store = DiskStore::new(&file, true, mode).await?;
        let mut database = Database::new(store, false).await?;
        database.infodump_from().await?;
        database.close().await?;
    }
    warn!(" -------- DB infodump complete  --------");
    return Ok(());
}
