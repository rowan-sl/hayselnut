///! test command for tsdb2. does whatever mtnash currently needs
use std::path::PathBuf;

use anyhow::Result;

use crate::tsdb2::{
    alloc::{
        store::{
            disk::{DiskMode, DiskStore},
            raid::ArrayR0 as RaidArray,
        },
        UntypedStorage,
    },
    Database,
};

pub async fn main(init_overwrite: bool, files: Vec<PathBuf>, mode: DiskMode) -> Result<()> {
    info!("Initializing raid array of {} files", files.len());
    let mut store = RaidArray::new();
    for file in files {
        //let canon = file.canonicalize()?;
        //debug!("{file:?} resolves to {canon:?}");
        let mut elem = DiskStore::new(&file, false, mode).await?;
        //elem.expand_by(500_000).await?;
        store.add_element(elem).await?;
    }
    store.print_info().await?;
    let database = Database::new(store, init_overwrite).await?;

    database.close().await?;
    return Ok(());
}
