///! test command for tsdb2. does whatever mtnash currently needs
use std::path::PathBuf;

use anyhow::Result;

use crate::tsdb2::{alloc::disk_store::DiskStore, Database};

pub async fn main(init_overwrite: bool, file: PathBuf) -> Result<()> {
    let file = file.canonicalize()?;
    debug!("{{file}} resolves to {file:?}");
    let store = DiskStore::new(&file, false).await?;
    let database = Database::new(store, init_overwrite).await?;

    database.close().await?;
    return Ok(());
}
