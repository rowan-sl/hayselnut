//! --redundant-- array of independant disks (store impls)
use std::{any::type_name, error::Error, fmt::Write, mem::swap};

use uuid::Uuid;
use zerocopy::{AsBytes, FromBytes};

use crate::tsdb2::{
    alloc::{
        ptr::{Ptr, Void},
        Storage, UntypedStorage,
    },
    repr::info::sfmt,
};

const MAGIC_BYTES: [u8; 13] = *b"HayselnutRAID";

mod raid_ids {
    static_assertions::const_assert_eq!(0x01, 1);
    pub const RAID0: u8 = 0x00;
}

/// Header, present in all raid stores (written by this one)
/// that describes this part's contribution, and the overall array (for verification uses)
#[derive(Clone, Copy, FromBytes, AsBytes)]
#[repr(C)]
struct RaidHeader {
    pub magic_bytes: [u8; 13],
    /// which disk in the array is this (starts at 0)
    pub disk_num: u8,
    /// how many disks are there in the array
    pub num_disks: u8,
    /// identifier of this array's raid version
    pub raid_id: u8,
    /// UUID of this array
    pub array_identifier: Uuid,
}

#[derive(Debug, thiserror::Error)]
#[error("Storage Error (store {0}, error {1}): {2:?}")]
pub struct DynStorageError(
    &'static str,
    &'static str,
    Box<(dyn Error + Sync + Send + 'static)>,
);

struct DynStorage<T: UntypedStorage<Error = E>, E: Error + Sync + Send + 'static>(T);

#[async_trait::async_trait]
trait IsDynStorage: UntypedStorage + Send {
    async fn close_boxed(self: Box<Self>) -> Result<(), <Self as UntypedStorage>::Error>;
}

#[async_trait::async_trait]
impl<T: UntypedStorage<Error = E>, E: Error + Sync + Send + 'static> IsDynStorage
    for DynStorage<T, E>
{
    async fn close_boxed(self: Box<Self>) -> Result<(), <Self as UntypedStorage>::Error> {
        self.close().await
    }
}

#[async_trait::async_trait]
impl<T: UntypedStorage<Error = E>, E: Error + Sync + Send + 'static> UntypedStorage
    for DynStorage<T, E>
{
    type Error = DynStorageError;
    async fn read_buf(
        &mut self,
        at: Ptr<Void>,
        amnt: u64,
        into: &mut [u8],
    ) -> Result<(), Self::Error> {
        self.0
            .read_buf(at, amnt, into)
            .await
            .map_err(|e| DynStorageError(type_name::<T>(), type_name::<E>(), Box::new(e)))
    }
    async fn write_buf(
        &mut self,
        at: Ptr<Void>,
        amnt: u64,
        from: &[u8],
    ) -> Result<(), Self::Error> {
        self.0
            .write_buf(at, amnt, from)
            .await
            .map_err(|e| DynStorageError(type_name::<T>(), type_name::<E>(), Box::new(e)))
    }
    async fn close(self) -> Result<(), Self::Error> {
        self.0
            .close()
            .await
            .map_err(|e| DynStorageError(type_name::<T>(), type_name::<E>(), Box::new(e)))
    }
    async fn size(&mut self) -> Result<u64, Self::Error> {
        self.0
            .size()
            .await
            .map_err(|e| DynStorageError(type_name::<T>(), type_name::<E>(), Box::new(e)))
    }
    async fn expand_by(&mut self, amnt: u64) -> Result<(), Self::Error> {
        self.0
            .expand_by(amnt)
            .await
            .map_err(|e| DynStorageError(type_name::<T>(), type_name::<E>(), Box::new(e)))
    }
    async fn resizeable(&mut self) -> Result<bool, Self::Error> {
        self.0
            .resizeable()
            .await
            .map_err(|e| DynStorageError(type_name::<T>(), type_name::<E>(), Box::new(e)))
    }
}

impl Storage for (dyn UntypedStorage<Error = DynStorageError> + Send) {}

#[derive(Debug, thiserror::Error)]
pub enum RaidError {
    #[error("operation on array element failed: {0}")]
    ElementError(#[from] DynStorageError),
    #[error("address out of bounds")]
    OutOfBounds,
    #[error("multiple [element generated] errors occured: {0:?}")]
    MultiError(Vec<DynStorageError>),
    #[error("attempted to resize a raid array, which is forbidden")]
    Resize,
}

struct Element {
    store: Box<dyn IsDynStorage<Error = DynStorageError>>,
    size: u64,
}

/// Raid zero array
pub struct ArrayR0 {
    // num_disks = elements.len()
    // must be in order of disk number
    elements: Vec<Element>,
    array_identifier: Uuid,
}

impl ArrayR0 {
    /// Create a new array with a random identifier and zero disks
    pub fn new() -> Self {
        Self {
            elements: vec![],
            array_identifier: Uuid::new_v4(),
        }
    }

    pub async fn add_element<S: Storage + Send>(&mut self, elem: S) -> Result<(), RaidError> {
        let mut elem = Box::new(DynStorage(elem));
        let elem_size = elem.size().await?;
        elem.write_typed(
            Ptr::with(0),
            &RaidHeader {
                magic_bytes: MAGIC_BYTES,
                disk_num: self.elements.len() as _,
                num_disks: (self.elements.len() + 1)
                    .try_into()
                    .expect("Cannot have more than 256 elements in a raid array"),
                raid_id: raid_ids::RAID0,
                array_identifier: self.array_identifier,
            },
        )
        .await?;
        self.elements.push(Element {
            store: elem,
            size: elem_size,
        });
        let num = self.elements.len().try_into().unwrap();
        for elem in &mut self.elements {
            let mut header = elem.store.read_typed(Ptr::<RaidHeader>::with(0)).await?;
            // TODO: verify the elements state?
            header.num_disks = num;
            elem.store
                .write_typed(Ptr::<RaidHeader>::with(0), &header)
                .await?;
        }
        Ok(())
    }

    pub async fn print_info(&mut self) -> Result<(), RaidError> {
        let mut first_line = format!(
            "RAID array info (Raid zero, {} elements, ID: {}, size: ",
            self.elements.len(),
            self.array_identifier
        );
        let mut out = String::new();
        let mut total_size = 0;
        for elem in &mut self.elements {
            let header = elem.store.read_typed(Ptr::<RaidHeader>::with(0)).await?;
            total_size += elem.size;
            writeln!(
                &mut out,
                "\t- element {}/{}, {}",
                header.disk_num,
                header.num_disks,
                sfmt(elem.size as _)
            )
            .unwrap();
        }
        writeln!(&mut first_line, "{})", sfmt(total_size as _)).unwrap();
        first_line.push_str(&out);
        swap(&mut first_line, &mut out);
        info!("{out}");
        Ok(())
    }

    /// perform address translation (address in array -> element number, address in element)
    async fn translate(&mut self, mut address: u64) -> Result<(usize, u64), RaidError> {
        let mut disk_num = 0;
        for elem in &self.elements {
            if address < elem.size {
                return Ok((disk_num, address));
            } else {
                disk_num += 1;
                address -= elem.size;
            }
        }
        return Err(RaidError::OutOfBounds);
    }
}

#[async_trait::async_trait]
impl UntypedStorage for ArrayR0 {
    type Error = RaidError;
    async fn read_buf(
        &mut self,
        mut at: Ptr<Void>,
        amnt: u64,
        into: &mut [u8],
    ) -> Result<(), Self::Error> {
        let (elem, addr) = self.translate(at.addr).await?;
        at = Ptr::with(addr);
        self.elements[elem].store.read_buf(at, amnt, into).await?;
        Ok(())
    }

    async fn write_buf(
        &mut self,
        mut at: Ptr<Void>,
        amnt: u64,
        from: &[u8],
    ) -> Result<(), Self::Error> {
        let (elem, addr) = self.translate(at.addr).await?;
        at = Ptr::with(addr);
        self.elements[elem].store.write_buf(at, amnt, from).await?;
        Ok(())
    }

    async fn close(self) -> Result<(), Self::Error> {
        let mut errors = vec![];
        for elem in self.elements {
            if let Err(e) = elem.store.close_boxed().await {
                errors.push(e);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(RaidError::MultiError(errors))
        }
    }
    async fn size(&mut self) -> Result<u64, Self::Error> {
        Ok(self.elements.iter().map(|e| e.size).sum())
    }
    async fn expand_by(&mut self, _amnt: u64) -> Result<(), Self::Error> {
        Err(RaidError::Resize)
    }
    async fn resizeable(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
}
