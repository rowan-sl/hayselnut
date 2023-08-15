use std::mem::size_of;

use esp_idf_svc::nvs::{EspNvs, EspNvsPartition, NvsPartitionId};
use esp_idf_sys::EspError;
use serde::{Deserialize, Serialize};
use static_assertions::const_assert;
use uuid::Uuid;

//TODO: implement a way of upgrading prev versions
pub const CURRENT_VERSION: u64 = 1;

pub const NAMESPACE: &str = "haysel_store";
pub const STATION_STORE_ID: &str = "data";
pub const STATION_STORE_VERSION_ID: &str = "id";
// might need to increase if StationStoreData gets too large
pub const STORE_DATA_SIZE: usize = 48;

const_assert!(NAMESPACE.len() <= 15); // namespace must be <15 chars

pub struct StationStoreCached<T: NvsPartitionId> {
    access: StationStoreAccess<T>,
    cache: StationStoreData,
}

impl<T: NvsPartitionId> StationStoreCached<T> {
    pub fn init(partition: EspNvsPartition<T>) -> Result<Self, EspError> {
        let mut store = StationStoreAccess::new(partition)?;
        let station_info = if !store.exists()? {
            warn!("Performing first-time initialization of station information");
            let default = StationStoreData {
                station_uuid: Uuid::new_v4(),
            };
            warn!("Picked a UUID of {}", default.station_uuid);
            store.write(&default)?;
            default
        } else {
            store.read()?.unwrap()
        };
        Ok(Self {
            access: store,
            cache: station_info,
        })
    }
}

impl<T: NvsPartitionId> StationStore for StationStoreCached<T> {
    fn read(&self) -> &StationStoreData {
        &self.cache
    }
    #[doc(hidden)]
    fn write(&mut self, new: StationStoreData) -> Result<(), EspError> {
        self.access.write(&new)?;
        self.cache = new;
        Ok(())
    }
}

// trait objects cant use generics, you say?
impl dyn StationStore {
    #[allow(unused)]
    fn modify(&mut self, f: impl FnOnce(&mut StationStoreData)) -> Result<(), EspError> {
        let mut v = *self.read(); // copy
        f(&mut v);
        if v != *self.read() {
            self.write(v)?;
        }
        Ok(())
    }
}

pub trait StationStore {
    fn read(&self) -> &StationStoreData;
    fn write(&mut self, new: StationStoreData) -> Result<(), EspError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationStoreData {
    pub station_uuid: Uuid,
}

pub struct StationStoreAccess<T: NvsPartitionId> {
    nvs: EspNvs<T>,
}

impl<T: NvsPartitionId> StationStoreAccess<T> {
    pub fn new(partition: EspNvsPartition<T>) -> Result<Self, EspError> {
        Ok(Self {
            nvs: EspNvs::new(partition, NAMESPACE, true)?,
        })
    }

    pub fn exists(&mut self) -> Result<bool, EspError> {
        Ok(
            match (
                self.nvs.contains(STATION_STORE_VERSION_ID)?,
                self.nvs.contains(STATION_STORE_ID)?,
            ) {
                (false, false) => false,
                (true, false) | (false, true) => {
                    panic!("[one of] StationStore version/data is in NVS flash, but not the other!")
                }
                (true, true) => true,
            },
        )
    }

    pub fn read(&mut self) -> Result<Option<StationStoreData>, EspError> {
        let mut id_buf = [0u8; size_of::<u64>()];
        let Some(version) = self.nvs.get_raw(STATION_STORE_VERSION_ID, &mut id_buf)? else {
            return Ok(None);
        };
        assert_eq!(
            version.len(),
            size_of::<u64>(),
            "Size of stored version ID is too small/large!"
        );

        let mut id_buf2 = [0u8; size_of::<u64>()];
        id_buf2.copy_from_slice(version);
        let version = u64::from_be_bytes(id_buf2);
        assert_eq!(
            version, CURRENT_VERSION,
            "Version of stored NVS data is mismatched (expected {CURRENT_VERSION} found {version})"
        );

        let mut store_buf = [0u8; STORE_DATA_SIZE];
        let Some(store) = self.nvs.get_raw(STATION_STORE_ID, &mut store_buf)? else {
            return Ok(None);
        };
        let store = rmp_serde::from_slice(store).expect("Faild to deserialize NVS store");
        Ok(Some(store))
    }

    pub fn write(&mut self, store: &StationStoreData) -> Result<(), EspError> {
        let mut id_buf = [0u8; size_of::<u64>()];
        let version =
            if let Some(version) = self.nvs.get_raw(STATION_STORE_VERSION_ID, &mut id_buf)? {
                version
            } else {
                log::warn!(
                    "Performing first-time initialization of StationStore NVS version information"
                );
                self.nvs
                    .set_raw(STATION_STORE_VERSION_ID, &CURRENT_VERSION.to_be_bytes())?;
                id_buf = CURRENT_VERSION.to_be_bytes();
                &id_buf
            };
        assert_eq!(
            version.len(),
            size_of::<u64>(),
            "Size of stored version ID is too small/large!"
        );

        let mut id_buf2 = [0u8; size_of::<u64>()];
        id_buf2.copy_from_slice(version);
        let version = u64::from_be_bytes(id_buf2);
        assert_eq!(
            version, CURRENT_VERSION,
            "Version of stored NVS data is mismatched (expected {CURRENT_VERSION} found {version})"
        );

        let ser = rmp_serde::to_vec(store).expect("Failed to serialize");
        let mut store_buf = [0u8; STORE_DATA_SIZE];
        store_buf[0..ser.len()].copy_from_slice(&ser);
        self.nvs.set_raw(STATION_STORE_ID, &store_buf)?;
        Ok(())
    }
}
