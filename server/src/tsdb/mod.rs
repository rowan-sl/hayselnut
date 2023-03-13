use std::{cmp, path::Path};

use chrono::{DateTime, Datelike, Utc};
use tokio::{
    fs::{File, OpenOptions},
    io::{self, AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
};
use zerocopy::{AsBytes, FromBytes};

mod alloc;
mod repr;
use repr::{Data, FPtr};

// TODO: Ctrl+C handler to flush data to disk (and allocated objects)
// also make a write-ahead log or similar to catch unexpected shutdowns and recover gracefully
pub struct DB<T: Data> {
    file: File,
    head: FPtr<repr::Year<T>>,
}

impl<T: Data> DB<T> {
    pub async fn open_new(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(path.with_extension(".tsdb"))
            .await?;
        let mut db = Self {
            file,
            head: FPtr::null(), //initialized in Self::init
        };
        db.init().await?;
        Ok(db)
    }

    async fn init(&mut self) -> io::Result<()> {
        let mut buf = vec![];
        buf.extend_from_slice(0xDEADBEEFu32.as_bytes()); // addr 0
        self.file.write_all(&buf).await?;
        Ok(())
    }

    pub async fn close(mut self) -> io::Result<()> {
        self.file.sync_all().await?;
        self.file.shutdown().await?;
        Ok(())
    }

    async fn read<P: FromBytes>(&mut self, ptr: FPtr<P>) -> io::Result<P> {
        let mut buf = vec![0; ptr.pointee_size()];
        self.read_raw(ptr.addr, &mut buf).await?;
        Ok(P::read_from(buf.as_slice()).unwrap())
    }

    async fn read_raw(&mut self, addr: u64, buf: &mut [u8]) -> io::Result<()> {
        self.file.seek(io::SeekFrom::Start(addr)).await?;
        self.file.read_exact(buf).await?;
        Ok(())
    }

    async fn write<P: AsBytes>(&mut self, ptr: FPtr<P>, val: &P) -> io::Result<()> {
        let buf = val.as_bytes();
        self.write_raw(ptr.addr, buf).await?;
        Ok(())
    }

    async fn write_raw(&mut self, addr: u64, buf: &[u8]) -> io::Result<()> {
        self.file.seek(io::SeekFrom::Start(addr)).await?;
        self.file.write_all(buf).await?;
        Ok(())
    }

    /// allocate space for `P`, returning a non-null initialized pointer.
    async fn alloc<P>(&mut self) -> io::Result<FPtr<P>> {
        todo!()
    }

    /// deallocate `ptr`, freeing the space it used.
    ///
    /// Saftey:
    /// - `ptr` must have been obtained from `alloc`
    /// - `ptr` must not have been deallocated before
    async unsafe fn dealloc<P>(&mut self, ptr: FPtr<P>) -> io::Result<()> {
        todo!()
    }

    pub async fn insert_record(&mut self, time: DateTime<Utc>, record: T) -> io::Result<()> {
        let year = time.year();
        let year_val: repr::Year<T> = if self.head.is_null() {
            let head = repr::Year::with_date(time);
            let ptr = self.alloc::<repr::Year<T>>().await?;
            self.write(ptr, &head).await?;
            self.head = ptr;
            head
        } else {
            let head = self.read(self.head).await?;
            match year.cmp(&head.year) {
                cmp::Ordering::Greater => {
                    let mut head = head;
                    loop {
                        if head.has_next() {
                            let next_head = self.read(head.next).await?;
                            match year.cmp(&next_head.year) {
                                cmp::Ordering::Greater => {
                                    head = next_head;
                                    continue;
                                }
                                cmp::Ordering::Equal => {
                                    break next_head;
                                }
                                cmp::Ordering::Less => {
                                    // we are in-between a too small and too large year,
                                    // create a new entry and update poitners on either side
                                    let mut new_entry = repr::Year::<T>::with_date(time);
                                    // update new entry with poitner to next one
                                    new_entry.next = head.next;
                                    let new_entry_ptr = self.alloc().await?;
                                    self.write(new_entry_ptr, &new_entry).await?;
                                    // update entry before with pointer to new entry
                                    let mut updated_head = head;
                                    updated_head.next = new_entry_ptr;
                                    // NEXT THING TODO: API for keeping track of allocated objects
                                    //self.write(updated_head, updated_head_addr).await?;
                                    todo!()
                                }
                            }
                        } else {
                            todo!()
                        }
                    }
                }
                cmp::Ordering::Equal => head,
                cmp::Ordering::Less => {
                    let mut new_head = repr::Year::with_date(time);
                    new_head.next = self.head;
                    let ptr = self.alloc::<repr::Year<T>>().await?;
                    self.write(ptr, &new_head).await?;
                    self.head = ptr;
                    new_head
                }
            }
        };
        let day = time.ordinal0();
        let day_time = time.time();
        todo!()
    }
}
