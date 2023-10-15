//! Utility for loading the registry types (`KnownStations`, `KnownChannels`, etc) from disk

use std::{
    ops::{Deref, DerefMut},
    path::PathBuf,
};

use anyhow::Result;
use serde::{de::DeserializeOwned, Serialize};
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
};

use crate::{
    core::shutdown::{async_drop::AsyncDrop, ShutdownHandle},
    misc::Take,
};

pub struct JsonLoader<R: Serialize + DeserializeOwned> {
    file: Take<File>,
    value: R,
    drop: AsyncDrop,
}

impl<R: Serialize + DeserializeOwned> JsonLoader<R> {
    /// Loads the json at `path`, using `R::default` if it does not exist
    #[instrument(skip(sh_handle))]
    pub async fn open(path: PathBuf, sh_handle: ShutdownHandle) -> Result<Self>
    where
        R: Default,
    {
        let new = !path.exists();
        if path.exists() && !path.is_file() {
            error!("Could not open `{path:?}` -- directory exists here");
            bail!("JsonLoader::open failed - invalid path");
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)
            .await?;
        let value = if new {
            R::default()
        } else {
            let mut buf = String::new();
            file.read_to_string(&mut buf).await?;
            if buf.trim().is_empty() {
                R::default()
            } else {
                serde_json::from_str(&buf)?
            }
        };
        Ok(Self {
            file: Take::new(file),
            value,
            drop: AsyncDrop::new(sh_handle).await,
        })
    }

    #[instrument(skip(self))]
    pub async fn sync(&mut self) -> Result<()> {
        let serialized = serde_json::to_string_pretty(&self.value)?;
        self.file.set_len(0).await?;
        self.file.seek(std::io::SeekFrom::Start(0)).await?;
        self.file.write_all(serialized.as_bytes()).await?;
        Ok(())
    }
}

impl<R: Serialize + DeserializeOwned> Deref for JsonLoader<R> {
    type Target = R;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<R: Serialize + DeserializeOwned> DerefMut for JsonLoader<R> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<R: Serialize + DeserializeOwned> Drop for JsonLoader<R> {
    fn drop(&mut self) {
        let Ok(serialized) = serde_json::to_string_pretty(&self.value) else {
            error!("JsonLoader sync failed - could not serialize");
            return;
        };
        let mut file: File = self.file.take();
        self.drop.run(async move {
            if let Err(e) = file.set_len(0).await {
                error!("JsonLoader sync failed - could not truncate: {e:#?}");
                return;
            }
            if let Err(e) = file.seek(std::io::SeekFrom::Start(0)).await {
                error!("JsonLoader sync failed - could not truncate: {e:#?}");
                return;
            }
            if let Err(e) = file.write_all(serialized.as_bytes()).await {
                error!("JsonLoader sync failed - could not write: {e:#?}");
                return;
            }
            if let Err(e) = file.shutdown().await {
                error!("JsonLoader drop failed - could not shutdown file stream: {e:#?}");
                return;
            }
        });
    }
}
