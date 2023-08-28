use std::path::Path;

use tokio::{
    fs::{File, OpenOptions},
    io::{self, AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
};

use super::{
    ptr::{Ptr, Void},
    Storage,
};

pub struct DiskStore {
    file: File,
}

impl DiskStore {
    #[instrument]
    pub async fn new(path: &Path, readonly: bool) -> Result<Self, <Self as Storage>::Error> {
        Ok(Self {
            file: OpenOptions::new()
                .read(true)
                .write(!readonly)
                .create(!readonly)
                .open(path)
                .await?,
        })
    }
}

#[async_trait::async_trait]
impl Storage for DiskStore {
    type Error = io::Error;
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
    async fn size(&mut self) -> Result<u64, Self::Error> {
        let guess = self.file.metadata().await?.len();
        if guess != 0 {
            Ok(guess)
        } else {
            // deals with block devices on linux yeilding zero as the size
            Ok(self.file.seek(io::SeekFrom::End(0)).await?)
        }
    }

    #[instrument(skip(self))]
    async fn expand_by(&mut self, amnt: u64) -> Result<(), Self::Error> {
        let size = self.size().await?;
        self.file.set_len(size + amnt).await
    }
}
