use std::{
    fs::{self, OpenOptions},
    io,
    mem::ManuallyDrop,
    ptr,
};

use anyhow::Result;
use chrono::{DateTime, Utc};
use memmap2::MmapMut;
use mycelium::station::{capabilities::ChannelID, identity::StationID};
use zerocopy::FromZeroes;

use self::alloc::{AllocAccess, TypeRegistry};

mod alloc;
mod repr;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O Error: {0:#}")]
    Mmap(#[from] io::Error),
}

struct DBStore {
    map: MmapMut,
    alloc_t_reg: TypeRegistry,
}

impl DBStore {
    pub fn access<'a>(&'a mut self, write_header: bool) -> AllocAccess<'a> {
        AllocAccess::new(&mut self.map, &self.alloc_t_reg, write_header)
    }
}

pub struct DB {
    file: *const fs::File,
    store: ManuallyDrop<DBStore>,
    init: bool,
}

impl DB {
    /// Creates an interaface to the database stored in `file`
    ///
    /// ## Initialization
    /// this function does not rely on `file` containing anything in perticular.
    /// before the database may be used, you must initialize it using [`DB::open`] (to open an existing datbase) or [`DB::init`] to initialize a new one
    ///
    /// ## Errors
    /// if memory mapping fails
    ///
    /// ## Saftey
    /// see memmap2::MmapMut::map_mut (file must be appropreatly protected, and it is UB if it is changed externally)
    #[must_use]
    #[forbid(unsafe_op_in_unsafe_fn)]
    pub unsafe fn new(file: fs::File) -> Result<Self, Error> {
        let file = &*Box::leak(Box::new(file));
        // Saftey: forwarded to consumer of this function
        let map = unsafe { MmapMut::map_mut(file) }?;
        let mut alloc_t_reg = TypeRegistry::new();
        // only types that HAVE POINTERS TO THEM need to go here
        alloc_t_reg.register::<repr::DBEntrypoint>();
        alloc_t_reg.register::<repr::Station>();
        alloc_t_reg.register::<repr::Channel>();
        alloc_t_reg.register::<repr::ChannelData>();
        Ok(Self {
            file: file as *const _,
            store: ManuallyDrop::new(DBStore { map, alloc_t_reg }),
            init: false,
        })
    }

    #[must_use]
    #[allow(dead_code)]
    pub(in crate::tsdb3) fn new_in_ram(size: usize) -> Result<Self, Error> {
        let map = MmapMut::map_anon(size)?;
        let mut alloc_t_reg = TypeRegistry::new();
        // only types that HAVE POINTERS TO THEM need to go here
        alloc_t_reg.register::<repr::DBEntrypoint>();
        alloc_t_reg.register::<repr::Station>();
        alloc_t_reg.register::<repr::Channel>();
        alloc_t_reg.register::<repr::ChannelData>();
        Ok(Self {
            file: ptr::null(),
            store: ManuallyDrop::new(DBStore { map, alloc_t_reg }),
            init: false,
        })
    }

    /// Initialize a new database, discarding any previous content.
    ///
    /// This function must only be called once, before any other usage of the db and is the alternative to [`DB::open`]
    pub fn init(&mut self) {
        assert!(!self.init);
        let mut access = self.store.access(true);
        let (entry_ptr, entry) = access.alloc::<repr::DBEntrypoint>();
        *access.entrypoint_pointer() = entry_ptr.cast::<alloc::ptr::Void>();
        entry.tuning_params.station_map_chunk_size =
            repr::MapStations::new_zeroed().stations.len() as u64;
        entry.tuning_params.channel_map_chunk_size =
            repr::Station::new_zeroed().channels.len() as u64;
        self.init = true;
    }

    /// Open an existing database, under the assumption that there is one.
    ///
    /// This function must only be called once, before any other usage of the db and is the alternative to [`DB::init`]
    pub fn open(&mut self) {
        assert!(!self.init);
        // will error if the alloc header is invalid
        let mut access = self.store.access(false);
        let entry = access.entrypoint::<repr::DBEntrypoint>().unwrap();
        assert!(
            entry.tuning_params.station_map_chunk_size
                == repr::MapStations::new_zeroed().stations.len() as u64
        );
        assert!(
            entry.tuning_params.channel_map_chunk_size
                == repr::Station::new_zeroed().channels.len() as u64
        );
        self.init = true;
    }

    /// Get all stations currently known to the database
    pub fn get_stations<'a>(&'a mut self) -> impl Iterator<Item = &'a StationID> + 'a {
        assert!(self.init);
        let mut access = self.store.access(false);
        let entry = access.entrypoint::<repr::DBEntrypoint>().unwrap();
        entry
            .stations
            .stations
            .iter()
            .take_while(|station| !station.ptr.is_null())
            .map(|station| StationID::from_bytes_ref(&station.id))
    }

    /// Get all channels known to a given station (if it exists)
    pub fn get_channels_for<'a>(
        &'a mut self,
        station_id: StationID,
    ) -> Option<impl Iterator<Item = &'a ChannelID> + 'a> {
        assert!(self.init);
        assert!(!station_id.is_nil());
        let mut access = self.store.access(false);
        let entry = access.entrypoint::<repr::DBEntrypoint>().unwrap();
        let ptr = entry
            .stations
            .stations
            .iter()
            .take_while(|station| !station.ptr.is_null())
            .find(|station| &station.id == station_id.as_bytes())?
            .ptr;
        let station = access.read(ptr);
        Some(
            station
                .channels
                .iter()
                .take_while(|ch| !ch.ptr.is_null())
                .map(|ch| ChannelID::from_bytes_ref(&ch.id)),
        )
    }

    pub fn insert_station(&mut self, id: StationID) {
        assert!(self.init);
        assert!(!id.is_nil());
        assert!(self
            .get_stations()
            .find(|station| *station == &id)
            .is_none());
        let mut access = self.store.access(false);
        let entry = access.entrypoint::<repr::DBEntrypoint>().unwrap();
        let first_empty = entry
            .stations
            .stations
            .iter_mut()
            .find(|station| station.ptr.is_null())
            .expect("Station map is full (cannot insert new station)");
        first_empty.id = id.into_bytes();
        // we don't need to add any channel info to the station map, only allocate and set a reference to it
        let (station_ptr, _station) = access.alloc::<repr::Station>();
        first_empty.ptr = station_ptr;
    }

    pub fn insert_channels(
        &mut self,
        station: StationID,
        channels: impl IntoIterator<Item = ChannelID>,
    ) {
        assert!(self.init);
        assert!(!station.is_nil());
        let mut access = self.store.access(false);
        let entry = access.entrypoint::<repr::DBEntrypoint>().unwrap();
        let ptr = entry
            .stations
            .stations
            .iter()
            .take_while(|elem| !elem.ptr.is_null())
            .find(|elem| &elem.id == station.as_bytes())
            .expect("Requested station [for insert_channels] does not exist!")
            .ptr;
        let station = access.read(ptr);
        let mut ins_idx = station
            .channels
            .iter()
            .take_while(|ch| !ch.ptr.is_null())
            .count();
        for ch in channels {
            assert!(!ch.is_nil());
            let elem = station
                .channels
                .get_mut(ins_idx)
                .expect("Channel map is full (cannot insert new channel)");
            elem.id = ch.into_bytes();
            let (data_ptr, _data) = access.alloc::<repr::Channel>();
            elem.ptr = data_ptr;
            ins_idx += 1;
        }
    }

    pub fn insert_data(
        &mut self,
        station_id: StationID,
        channel_id: ChannelID,
        time: DateTime<Utc>,
        reading: f32,
    ) {
        assert!(self.init);
        assert!(self.get_stations().find(|st| *st == &station_id).is_some());
        assert!(self
            .get_channels_for(station_id)
            .is_some_and(|mut chs| chs.find(|ch| *ch == &channel_id).is_some()));
        let timestamp = repr::unix_to_htime(time.timestamp())
            .expect("Cannot create timestamp (date is not between 2020 and 2156)");
        let mut access = self.store.access(false);
        let entry = access.entrypoint::<repr::DBEntrypoint>().unwrap();
        let ptr = entry
            .stations
            .stations
            .iter()
            .take_while(|elem| !elem.ptr.is_null())
            .find(|elem| &elem.id == station_id.as_bytes())
            .expect("Requested station [for insert_data] does not exist!")
            .ptr;
        let station = access.read(ptr);
        let ptr = station
            .channels
            .iter()
            .take_while(|elem| !elem.ptr.is_null())
            .find(|elem| &elem.id == channel_id.as_bytes())
            .expect("Requested channel [for insert_data] does not exist!")
            .ptr;
        let channel = access.read(ptr);
        assert!(channel.last_time <= timestamp);
        channel.last_time = timestamp;
        if channel.is_full() {
            let (new_chunk_ptr, new_chunk) = access.alloc::<repr::ChannelData>();
            *new_chunk = channel.data;
            channel.data.next = new_chunk_ptr;
            channel.num_used = 1;
            let entry = &mut channel.data.chunk[0];
            entry.htime = timestamp;
            entry.data = reading;
        } else {
            let entry = &mut channel.data.chunk[channel.num_used as usize];
            entry.htime = timestamp;
            entry.data = reading;
            channel.num_used += 1;
        }
    }

    pub fn qery_data(
        &mut self,
        station_id: StationID,
        channel_id: ChannelID,
        // lower bound
        after_time: DateTime<Utc>,
        // upper bound
        before_time: DateTime<Utc>,
        // not exactly respected, more of a general max (will be checked once every data chunk)
        max_results: usize,
    ) -> Vec<(DateTime<Utc>, f32)> {
        assert!(self.init);
        assert!(self.get_stations().find(|st| *st == &station_id).is_some());
        assert!(self
            .get_channels_for(station_id)
            .is_some_and(|mut chs| chs.find(|ch| *ch == &channel_id).is_some()));

        let t_lower = repr::unix_to_htime(after_time.timestamp())
            .expect("Cannot create timestamp (date is not between 2020 and 2156)");
        let t_upper = repr::unix_to_htime(before_time.timestamp())
            .expect("Cannot create timestamp (date is not between 2020 and 2156)");
        assert!(t_lower <= t_upper);

        let mut access = self.store.access(false);
        let entry = access.entrypoint::<repr::DBEntrypoint>().unwrap();
        let ptr = entry
            .stations
            .stations
            .iter()
            .take_while(|elem| !elem.ptr.is_null())
            .find(|elem| &elem.id == station_id.as_bytes())
            .expect("Requested station [for insert_data] does not exist!")
            .ptr;
        let station = access.read(ptr);
        let ptr = station
            .channels
            .iter()
            .take_while(|elem| !elem.ptr.is_null())
            .find(|elem| &elem.id == channel_id.as_bytes())
            .expect("Requested channel [for insert_data] does not exist!")
            .ptr;
        let channel = access.read(ptr);
        let mut results = vec![];

        let mut num_vaild = channel.num_used;
        let mut t_newest = channel.last_time;
        let mut t_oldest = channel.data.chunk[0].htime;
        let mut data = &mut channel.data;
        // if not(newest is older than oldest requested || oldest is newer than newest requested || current results < max results)
        while !(t_newest < t_lower || t_oldest > t_upper || results.len() < max_results) {
            results.extend(
                data.chunk[0..num_vaild as usize]
                    .iter()
                    .filter(|entry| entry.htime > t_lower && entry.htime < t_upper)
                    .map(|entry| {
                        (
                            DateTime::from_timestamp(repr::htime_to_unix(entry.htime), 0).unwrap(),
                            entry.data,
                        )
                    }),
            );
            if !data.next.is_null() {
                num_vaild = data.chunk.len() as u32;
                data = access.read(data.next);
                t_newest = data.chunk[data.chunk.len() - 1].htime;
                t_oldest = data.chunk[0].htime
            } else {
                break;
            }
        }
        results
    }
}

impl Drop for DB {
    fn drop(&mut self) {
        // Saftey: self.store not used after this
        unsafe { ManuallyDrop::drop(&mut self.store) };
        if !self.file.is_null() {
            // Saftey: self.file not used after this, no longer referenced by self.store
            let file = unsafe { ptr::read(self.file as *const fs::File) };
            let _ = file.sync_all();
            drop(file)
        }
    }
}

#[test]
fn create_new_db() {
    let mut db = DB::new_in_ram(4096).unwrap();
    db.init();
}

pub fn main() -> Result<()> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open("test.tsdb3")?;
    file.set_len(0)?;
    file.set_len(1024 * 500)?;
    // Saftey: lol. lmao.
    let mut db = unsafe { DB::new(file) }?;
    db.init();
    Ok(())
}
