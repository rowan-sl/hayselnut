use std::{
    fs::{self, OpenOptions},
    io,
    mem::ManuallyDrop,
    ptr,
};

use anyhow::Result;
use memmap2::MmapMut;
use zerocopy::FromZeroes;

use self::alloc::{AllocAccess, TypeRegistry};

mod alloc;
mod repr;

#[derive(Debug, thiserror::Error)]
enum Error {
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

struct DB {
    file: *const fs::File,
    store: ManuallyDrop<DBStore>,
    init: bool,
}

impl DB {
    /// Creates an interaface to the database stored in `file`
    ///
    /// ## Initialization
    /// this function does not rely on `file` containing anything in perticular.
    /// before the database may be used, you must initialize it using [`open`] (to open an existing datbase) or [`init`] to initialize a new one
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
        alloc_t_reg.register::<repr::DBEntrypoint>();
        alloc_t_reg.register::<repr::Channel>();
        alloc_t_reg.register::<repr::MapStations>();
        alloc_t_reg.register::<repr::Station>();
        alloc_t_reg.register::<repr::Channel>();
        Ok(Self {
            file: file as *const _,
            store: ManuallyDrop::new(DBStore { map, alloc_t_reg }),
            init: false,
        })
    }

    /// Initialize a new database, discarding any previous content.
    ///
    /// This function must only be called once, before any other usage of the db and is the alternative to [`open`]
    pub fn init(&mut self) {
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
    /// This function must only be called once, before any other usage of the db and is the alternative to [`init`]
    pub fn open(&mut self) {
        // will error if the alloc header is invalid
        let mut access = self.store.access(false);
        let entry_ptr = *access.entrypoint_pointer();
        let entry = access.read::<repr::DBEntrypoint>(entry_ptr.cast());
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
}

impl Drop for DB {
    fn drop(&mut self) {
        // Saftey: self.store not used after this
        unsafe { ManuallyDrop::drop(&mut self.store) };
        // Saftey: self.file not used after this, no longer referenced by self.store
        let file = unsafe { ptr::read(self.file as *const fs::File) };
        let _ = file.sync_all();
        drop(file)
    }
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
    // let mut map = unsafe { MmapMut::map_mut(&file)? };
    // let alloc_t_reg = {
    //     let mut alloc_t_reg = TypeRegistry::new();
    //     alloc_t_reg.register::<u64>();
    //     alloc_t_reg.register::<[u8; 13]>();
    //     alloc_t_reg
    // };
    // {
    //     let mut alloc = AllocAccess::new(&mut map, &alloc_t_reg, true);
    //     let (_ptr_v, v) = alloc.alloc::<[u8; 13]>();
    //     *v = *b"Hello, World!";
    // }
    // file.sync_all()?;
    Ok(())
}
