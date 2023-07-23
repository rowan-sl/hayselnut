//! # DB Hierarchy
//! {stations(by ID), chunked linked list} -> channels (by ID, so each one has only one type of simple data, and each sub-event has its own place)
//! {channels(by ID), chunked linked list} -> metadata, first data index chunk
//! metadata: data type (ID of it, each one has an associated size but this is stored elsewhere)
//! [data index chunk: (pointers to n-length chunks, each w/ start time, amnt full) and (pointer to the next chunk)]
//! [data chunk: n number of time offset (small uint seconds) from start time, then n number const(for the channel)-sized data]
//!
//! # allocator:
//!
//! type of data being stored: many repeats of things that are the same size (only a handfull of objects, and they are all const-size)
//! - use a linked list allocator design, but have seperate linked lists for each size of data.
//!
//! each allocated part consists of metadata, then the data. meteadata contains
//! - is this chunk free
//! - the length of the chunk
//! - pointer to the previous free chunk (of this size)
//! - pointer to the next free chunk (of this size) or null if there is none
//!
//! alloc header:
//! - [in chunked linked list, or possibly just have a max number of types]: head pointers to the linked list of free data for each size (and the associated size)

use self::{
    alloc::{object::Object, Allocator, Storage},
    error::DBError,
    repr::DBEntrypoint,
};

pub mod alloc;
pub mod error;
pub mod repr;

/// the database
pub struct Database<Store: Storage> {
    alloc: Allocator<Store>,
}

impl<Store: Storage> Database<Store> {
    #[instrument(skip(store))]
    pub async fn new(store: Store) -> Result<Self, DBError<<Store as Storage>::Error>> {
        let mut alloc = Allocator::new(store).await?;
        if alloc.get_entrypoint().await?.is_null() {
            // the entrypoint is null, so this is a fresh database.

            // initialize the new entrypoint
            // this is the only thing we get access to when freshly opening
            // the database, and it is used to get at everything else
            let entrypoint = Object::new_alloc(&mut alloc, DBEntrypoint {}).await?;
            alloc.set_entrypoint(entrypoint.pointer().cast()).await?;
            entrypoint.dispose_sync(&mut alloc).await?;
        }
        todo!()
    }

    #[instrument(skip(self))]
    pub async fn close(self) -> Result<(), DBError<<Store as Storage>::Error>> {
        self.alloc.close().await?;
        todo!()
    }
}
