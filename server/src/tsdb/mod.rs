use std::{path::Path, cmp};

use chrono::{DateTime, Utc, Datelike};
use tokio::io;

mod alloc;
mod repr;

use repr::Data;
use alloc::{Ptr, Alloc, errors::AllocErr};
use tracing::{instrument, debug};

use self::alloc::{errors::{AllocReqErr, AllocRunnerErr}, Obj};

// TODO: Ctrl+C handler to flush data to disk (and allocated objects)
// also make a write-ahead log or similar to catch unexpected shutdowns and recover gracefully
pub struct DB<T: Data> {
    /// to update this, use `update_head`
    ///
    /// can be null, null=no head (or data in DB)
    cached_head: Ptr<repr::Year<T>>,
    alloc: Alloc,
}

impl<T: Data> DB<T> {
    #[instrument]
    pub async fn open(path: &Path) -> Result<Self, self::Error> {
        let alloc = Alloc::open(path).await?;
        let cached_head = {
            let o = alloc.get::<Ptr<repr::Year<T>>>(alloc::entrypoint_pointer()).await?;
            *o
        };
        Ok(DB { cached_head, alloc })
    }

    #[instrument(skip(self))]
    pub async fn close(self) -> Result<(), self::Error> {
        debug!("Closing DB");
        self.alloc.close().await?;
        Ok(())
    }

    async fn update_head(&mut self, new_head: Ptr<repr::Year<T>>) -> Result<(), AllocReqErr> {
        *self.alloc.get(alloc::entrypoint_pointer()).await? = new_head;
        self.cached_head = new_head;
        Ok(())
    } 

    pub async fn insert<TZ: chrono::TimeZone>(&mut self, at: DateTime<TZ>, record: T) -> Result<(), self::Error> {
        let at: DateTime<Utc> = at.with_timezone(&Utc);
        let year: Obj<repr::Year<T>> = if self.cached_head.is_null() {
            // TODO: fix this workaround (borrowing self.alloc, then update_head requires a full mutable borrow of self)
            let new_head = Obj::into_ptr(self.alloc.alloc(repr::Year::with_date(at)).await?);
            self.update_head(new_head).await?;
            self.alloc.get(new_head).await?
        } else {
            let head = self.alloc.get(self.cached_head).await?;
            match at.year().cmp(&head.year) {
                cmp::Ordering::Greater => {
                    let mut c_head = head;
                    loop {
                        if c_head.has_next() {
                            let n_head = self.alloc.get(c_head.next).await?;
                            match at.year().cmp(&n_head.year) {
                                cmp::Ordering::Greater => c_head = n_head,
                                cmp::Ordering::Equal => break n_head,
                                cmp::Ordering::Less => {
                                    // c_head is a preivous year, n_head is a following year.
                                    // we create a new year, and insert it in the middle.
                                    let mut m_head = self.alloc.alloc(repr::Year::with_date(at)).await?;
                                    m_head.next = Obj::get_ptr(&n_head);
                                    c_head.next = Obj::get_ptr(&m_head);
                                }
                            }
                        }
                    }
                },
                cmp::Ordering::Equal => head,
                cmp::Ordering::Less => {
                    let mut new_head = self.alloc.alloc(repr::Year::with_date(at)).await?;
                    new_head.next = Obj::get_ptr(&head);
                    drop(head);
                    let ptr = Obj::into_ptr(new_head);
                    self.update_head(ptr).await?;
                    self.alloc.get(ptr).await?
                }
            }
        };
        let day = at.ordinal0();
        let time = at.time();
        todo!()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Error in allocator: {0:?}")]
    Alloc(#[from] AllocErr)
}

impl From<AllocReqErr> for Error {
    fn from(value: AllocReqErr) -> Self {
        Self::from(AllocErr::from(value))
    }
}

impl From<AllocRunnerErr> for Error {
    fn from(value: AllocRunnerErr) -> Self {
        Self::from(AllocErr::from(value))
    }
}
