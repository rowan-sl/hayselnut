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

use mycelium::station::{capabilities::ChannelID, identity::StationID};
use zerocopy::FromBytes;

use self::{
    alloc::{object::Object, ptr::Ptr, util::ChunkedLinkedList, Allocator, Storage},
    error::DBError,
    repr::DBEntrypoint,
};

pub mod alloc;
pub mod error;
pub mod repr;

mod tuning {
    // low values to force using the list functionality.
    // for real use, set higher
    pub const STATION_MAP_CHUNK_SIZE: usize = 1;
    pub const CHANNEL_MAP_CHUNK_SIZE: usize = 1;
    pub const DATA_INDEX_CHUNK_SIZE: usize = 1;
    // optimize for the largest size (ish) that does not exceed the limit of the delta-time system.
    // must multiply by 2 to get a multiple of 8 (be a multiple of 4) (note: real value is 1 smaller than specified here)
    //
    // if periodic data chunks are consistantly left empty decrease this, or if they are consistantly full increase it.
    // TODO: specify size in a more customizeable way?
    pub const DATA_GROUP_PERIODIC_SIZE: usize = 1024;
    /// honestly probably does not matter, as long as having one of them in the database is not too much of a big deal.
    pub const DATA_GROUP_SPORADIC_SIZE: usize = 1024;
}

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
            let map = Object::new_alloc(
                &mut alloc,
                ChunkedLinkedList::<{ tuning::STATION_MAP_CHUNK_SIZE }, repr::Station> {
                    next: Ptr::null(),
                    used: 0,
                    data: [repr::Station::new_zeroed(); tuning::STATION_MAP_CHUNK_SIZE],
                },
            )
            .await?
            .dispose_sync(&mut alloc)
            .await?;

            let entrypoint = Object::new_alloc(
                &mut alloc,
                DBEntrypoint {
                    stations: repr::MapStations { map },
                },
            )
            .await?;
            alloc.set_entrypoint(entrypoint.pointer().cast()).await?;
            entrypoint.dispose_sync(&mut alloc).await?;
        }
        Ok(Self { alloc })
    }

    #[instrument(skip(self))]
    pub async fn add_station(
        &mut self,
        id: StationID,
    ) -> Result<(), DBError<<Store as Storage>::Error>> {
        warn!("TODO: check that a station does not already exist");
        let eptr = self.alloc.get_entrypoint().await?.cast::<DBEntrypoint>();
        let entry = Object::new_read(&mut self.alloc, eptr).await?;
        ChunkedLinkedList::push(
            entry.stations.map,
            &mut self.alloc,
            repr::Station {
                id,
                channels: Ptr::null(),
            },
        )
        .await?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn add_channel(
        &mut self,
        to: StationID,
        id: ChannelID,
        kind: repr::DataGroupType,
    ) -> Result<(), DBError<<Store as Storage>::Error>> {
        warn!("TODO: check that the channel does not already exist");
        let eptr = self.alloc.get_entrypoint().await?.cast::<DBEntrypoint>();
        let entry = Object::new_read(&mut self.alloc, eptr).await?;
        let station = ChunkedLinkedList::find(entry.stations.map, &mut self.alloc, |s| s.id == id)
            .await?
            .expect("did not find requested station");
        ChunkedLinkedList::push(
            station.channels,
            &mut self.alloc,
            repr::Channel {
                id,
                metadata: repr::ChannelMetadata {
                    group_type: kind as u8,
                },
                _pad: Default::default(),
                data: Ptr::null(),
            },
        )
        .await?;
        entry.dispose_immutated();
        Ok(())
    }

    #[instrument]
    pub async fn infodump() {
        use info::print_inf;
        use repr::*;
        print_inf::<DBEntrypoint>();
    }

    #[instrument(skip(self))]
    pub async fn close(self) -> Result<(), DBError<<Store as Storage>::Error>> {
        self.alloc.close().await?;
        todo!()
    }
}
