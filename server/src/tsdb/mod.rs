use std::path::Path;

use tokio::{fs::{File, OpenOptions}, io::AsyncWriteExt};
use zerocopy::{FromBytes, AsBytes};

mod repr;
use repr::FPtr;

pub struct DB<T: FromBytes + AsBytes + Clone + Copy> {
    file: File,
    head: FPtr<repr::Year<T>>,
}

impl<T: FromBytes + AsBytes + Clone + Copy> DB<T> {
    pub async fn open_new(path: &Path) -> tokio::io::Result<Self> {
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(path.with_extension(".tsdb"))
            .await?;
        let mut db = Self {
            file,
            head: FPtr::null(),//initialized in Self::init
        };
        db.init().await?;
        Ok(db)
    }

    async fn init(&mut self) -> tokio::io::Result<()> {
        let mut buf = vec![];
        buf.extend_from_slice(0xDEADBEEFu32.as_bytes());// addr 0
        self.file.write_all(&buf).await?;
        Ok(())
    }

    pub async fn close(mut self) -> tokio::io::Result<()> {
        self.file.sync_all().await?;
        self.file.shutdown().await?;
        Ok(())
    }
}

