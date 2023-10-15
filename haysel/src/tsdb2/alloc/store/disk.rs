use std::path::Path;

use tokio::{
    fs::{File, OpenOptions},
    io::{self, AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
};

use crate::tsdb2::alloc::{
    ptr::{Ptr, Void},
    UntypedStorage,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskMode {
    /// normal mode.
    /// use with normal files, to allocate space as needed.
    Dynamic,
    /// pre-allocated fixed size mode.
    /// use with *only* block devices (e.g. `/dev/sda`) - linux only
    /// this changes how file size is determined - may (probably not) cause issues if used with normal files.
    BlockDevice,
}

#[derive(Debug, thiserror::Error)]
pub enum DiskError {
    #[error("I/O Error: {0}")]
    IOError(#[from] io::Error),
    #[error("attempted to write to a read-only store")]
    Readonly,
    #[error("DiskStore in BlockDevice mode does not support resizing")]
    FixedSize,
}

pub struct DiskStore {
    file: File,
    mode: DiskMode,
    readonly: bool,
}

impl DiskStore {
    #[instrument]
    pub async fn new(
        path: &Path,
        readonly: bool,
        mode: DiskMode,
    ) -> Result<Self, <Self as UntypedStorage>::Error> {
        Ok(Self {
            file: OpenOptions::new()
                .read(true)
                .write(!readonly)
                .create(!readonly)
                .open(path)
                .await?,
            mode,
            readonly,
        })
    }
}

#[async_trait::async_trait]
impl UntypedStorage for DiskStore {
    type Error = DiskError;
    #[instrument(skip(self, into))]
    async fn read_buf(
        &mut self,
        at: Ptr<Void>,
        amnt: u64,
        into: &mut [u8],
    ) -> Result<(), Self::Error> {
        assert!(into.len() >= amnt as _);
        self.file.seek(io::SeekFrom::Start(at.addr)).await?;
        let mut have_read = 0;
        while have_read < amnt {
            let read = self.file.read(&mut into[have_read as _..]).await? as u64;
            if read == 0 {
                // this means that the pointer/len combination was invalid.
                panic!("attempted to read past the end of the file");
            }
            have_read += read;
        }
        Ok(())
    }

    #[instrument(skip(self, from))]
    async fn write_buf(
        &mut self,
        at: Ptr<Void>,
        amnt: u64,
        from: &[u8],
    ) -> Result<(), Self::Error> {
        if self.readonly {
            return Err(DiskError::Readonly);
        }
        assert!(from.len() >= amnt as _);
        self.file.seek(io::SeekFrom::Start(at.addr)).await?;
        self.file.write_all(&from[..amnt as _]).await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn close(mut self) -> Result<(), Self::Error> {
        self.file.sync_all().await?;
        self.file.shutdown().await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn sync(&mut self) -> Result<(), Self::Error> {
        self.file.sync_all().await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn size(&mut self) -> Result<u64, Self::Error> {
        let guess = self.file.metadata().await?.len();
        if guess != 0 {
            Ok(guess)
        } else {
            // deals with block devices on linux yeilding zero as the size
            // (will also be called if the file size == 0)
            Ok(self.file.seek(io::SeekFrom::End(0)).await?)
        }
    }

    #[instrument(skip(self))]
    async fn expand_by(&mut self, amnt: u64) -> Result<(), Self::Error> {
        if self.readonly {
            return Err(DiskError::Readonly);
        } else if self.mode == DiskMode::BlockDevice {
            return Err(DiskError::FixedSize);
        }
        let size = self.size().await?;
        self.file.set_len(size + amnt).await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn resizeable(&mut self) -> Result<bool, Self::Error> {
        if self.readonly {
            return Ok(false);
        }
        match self.mode {
            DiskMode::Dynamic => Ok(true),
            DiskMode::BlockDevice => Ok(false),
        }
    }
}
