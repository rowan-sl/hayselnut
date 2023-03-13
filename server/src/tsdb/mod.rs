use std::path::Path;

use tokio::io;

mod alloc;
mod repr;

use repr::Data;
use alloc::{Ptr, Alloc};

// TODO: Ctrl+C handler to flush data to disk (and allocated objects)
// also make a write-ahead log or similar to catch unexpected shutdowns and recover gracefully
pub struct DB<T: Data> {
    head: Ptr<repr::Year<T>>,
    alloc: Alloc,
}

impl<T: Data> DB<T> {
    pub async fn open_new(path: &Path) -> io::Result<Self> {
        let alloc = Alloc::open_new(path).await?;
        Ok(DB { head: Ptr::null(), alloc })
    }

    pub async fn close(self) -> io::Result<()> {
        self.alloc.close().await?;
        Ok(())
    }

    // pub async fn insert_record(&mut self, time: DateTime<Utc>, record: T) -> io::Result<()> {
    //     let year = time.year();
    //     let year_val: repr::Year<T> = if self.head.is_null() {
    //         let head = repr::Year::with_date(time);
    //         let ptr = self.alloc::<repr::Year<T>>().await?;
    //         self.write(ptr, &head).await?;
    //         self.head = ptr;
    //         head
    //     } else {
    //         let head = self.read(self.head).await?;
    //         match year.cmp(&head.year) {
    //             cmp::Ordering::Greater => {
    //                 let mut head = head;
    //                 loop {
    //                     if head.has_next() {
    //                         let next_head = self.read(head.next).await?;
    //                         match year.cmp(&next_head.year) {
    //                             cmp::Ordering::Greater => {
    //                                 head = next_head;
    //                                 continue;
    //                             }
    //                             cmp::Ordering::Equal => {
    //                                 break next_head;
    //                             }
    //                             cmp::Ordering::Less => {
    //                                 // we are in-between a too small and too large year,
    //                                 // create a new entry and update poitners on either side
    //                                 let mut new_entry = repr::Year::<T>::with_date(time);
    //                                 // update new entry with poitner to next one
    //                                 new_entry.next = head.next;
    //                                 let new_entry_ptr = self.alloc().await?;
    //                                 self.write(new_entry_ptr, &new_entry).await?;
    //                                 // update entry before with pointer to new entry
    //                                 let mut updated_head = head;
    //                                 updated_head.next = new_entry_ptr;
    //                                 // NEXT THING TODO: API for keeping track of allocated objects
    //                                 //self.write(updated_head, updated_head_addr).await?;
    //                                 todo!()
    //                             }
    //                         }
    //                     } else {
    //                         todo!()
    //                     }
    //                 }
    //             }
    //             cmp::Ordering::Equal => head,
    //             cmp::Ordering::Less => {
    //                 let mut new_head = repr::Year::with_date(time);
    //                 new_head.next = self.head;
    //                 let ptr = self.alloc::<repr::Year<T>>().await?;
    //                 self.write(ptr, &new_head).await?;
    //                 self.head = ptr;
    //                 new_head
    //             }
    //         }
    //     };
    //     let day = time.ordinal0();
    //     let day_time = time.time();
    //     todo!()
    // }
}
