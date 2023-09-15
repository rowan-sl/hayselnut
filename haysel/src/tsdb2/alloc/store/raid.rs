//! --redundant-- array of independant disks (store impls)
use std::{
    any::type_name,
    cmp::{max, min},
    convert::identity,
    error::Error,
    fmt::Write,
    mem::{replace, size_of, swap},
};

use tokio::join;
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
    fn type_name(&self) -> &'static str;
}

#[async_trait::async_trait]
impl<T: UntypedStorage<Error = E>, E: Error + Sync + Send + 'static> IsDynStorage
    for DynStorage<T, E>
{
    async fn close_boxed(self: Box<Self>) -> Result<(), <Self as UntypedStorage>::Error> {
        self.close().await
    }

    fn type_name(&self) -> &'static str {
        type_name::<T>()
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
    #[error("attempted to perform operations using an uninitialized array")]
    UseUninitialized,
    #[error("attempted to add a disk to an already configured array")]
    ModifyInitialized,
    #[error("attempted to initialize an array for a second time")]
    DoubleInitialized,
    #[error("one or more elements in the array contains invalid data [in an unexpected manner]")]
    Corrupt,
    #[error("one or more elements in the array are missing")]
    MissingElements,
}

struct Element {
    store: Box<dyn IsDynStorage<Error = DynStorageError>>,
    size: u64,
}

/// Raid zero array
pub struct ArrayR0 {
    /// num_disks = elements.len()
    /// must be in order of disk number
    elements: Vec<Element>,
    array_identifier: Uuid,
    /// has this array been 'built' (validated / initialized)
    /// starts off as false, `add_element` can be called, and then `build` is called.
    /// after this it is set to true, `add_element` may not be called, and the array may be used.
    ready: bool,
}

impl ArrayR0 {
    /// Create a new array with a random identifier and zero disks
    pub fn new() -> Self {
        Self {
            elements: vec![],
            array_identifier: Uuid::nil(),
            ready: false,
        }
    }

    pub async fn add_element<S: Storage + Send>(&mut self, elem: S) -> Result<(), RaidError> {
        if self.ready {
            return Err(RaidError::ModifyInitialized);
        }
        let mut elem = Box::new(DynStorage(elem));
        let elem_size = elem.size().await?;
        self.elements.push(Element {
            store: elem,
            size: elem_size - size_of::<RaidHeader>() as u64,
        });
        Ok(())
    }

    /// # THIS WILL DELETE YOUR DATA!!!
    ///
    /// Called before [or as an alternative to, before closing] `RaidArray::build` (if called after,
    /// it will unset the `ready` flag and you will need to call `RaidArray::build` again)
    /// Deletes *enough* data in store to trigger a rebuild of the array, allocator, and thus database.
    pub async fn wipe_all_your_data_away(&mut self) -> Result<(), RaidError> {
        self.ready = false;
        let mut buf = vec![];
        for elem in &mut self.elements {
            // write enough data to overwrite the raid information, as well as the allocator header.
            // (the alloc header is allways at the start of the store)
            // since the database uses the allocator's entrypoint (stored in it's header) this will reset
            // that as well.
            let amnt = min(
                elem.size,
                (size_of::<RaidHeader>() + size_of::<crate::tsdb2::alloc::repr::AllocHeader>())
                    as u64,
            );
            buf.resize(max(buf.len(), amnt as usize), 0u8);
            elem.store.write_buf(Ptr::null(), amnt, &buf).await?;
        }
        Ok(())
    }

    /// build the raid array. called after all stores
    /// are added, but before this is used [at all] as a storage device
    pub async fn build(&mut self) -> Result<(), RaidError> {
        if self.ready {
            return Err(RaidError::DoubleInitialized);
        }
        let mut headers = vec![];
        let mut valid = vec![];
        for elem in &mut self.elements {
            let header = elem.store.read_typed(Ptr::<RaidHeader>::with(0)).await?;
            if header.magic_bytes == MAGIC_BYTES && header.raid_id == raid_ids::RAID0 {
                valid.push(headers.len())
            }
            headers.push(header);
        }
        if valid.is_empty() {
            self.array_identifier = Uuid::new_v4();
            warn!(
                "array contains no valid elements, creating new array {}",
                self.array_identifier
            );
            let num_disks = self.elements.len();
            for (i, elem) in self.elements.iter_mut().enumerate() {
                elem.store
                    .write_typed(
                        Ptr::with(0),
                        &RaidHeader {
                            magic_bytes: MAGIC_BYTES,
                            disk_num: i as _,
                            num_disks: num_disks
                                .try_into()
                                .expect("Cannot have more than 256 elements in a raid array"),
                            raid_id: raid_ids::RAID0,
                            array_identifier: self.array_identifier,
                        },
                    )
                    .await?;
            }
        } else {
            let conditions = valid
                .iter()
                .copied()
                .map(|idx| {
                    (
                        headers[idx].num_disks == headers[0].num_disks,
                        headers[idx].array_identifier == headers[0].array_identifier,
                    )
                })
                .fold((true, true), |(a, b), (a1, b1)| (a && a1, b && b1));
            if !conditions.1 {
                error!("the stores provided to the raid array contain the headers of multiple different arrays");
                return Err(RaidError::Corrupt);
            }
            if !conditions.0 {
                error!("the store contains headers from the same array that dissagree on the arrays properties");
                return Err(RaidError::Corrupt);
            }
            // each element corresponds to an array element, with the value indicating if it is present or not.
            let mut element_status = vec![false; headers[0].num_disks as usize];
            for &elem in &valid {
                if headers[elem].disk_num >= headers[elem].num_disks {
                    error!("a raid header exists with a element number greater than the total number of elements");
                    return Err(RaidError::Corrupt);
                }
                if replace(&mut element_status[headers[elem].disk_num as usize], true) {
                    error!(
                        "multiple raid elements exist with the same element number in this array!"
                    );
                    return Err(RaidError::Corrupt);
                }
            }
            if !element_status.iter().copied().all(identity) {
                error!(
                    "the raid array is missing disks {:?}",
                    element_status
                        .iter()
                        .enumerate()
                        .filter(|(_, x)| !**x)
                        .map(|(i, _)| i)
                        .collect::<Vec<_>>()
                );
                return Err(RaidError::MissingElements);
            }
            // find the elements that are not part of the array, and build them into it.
            let invalid = (0..headers.len())
                .filter(|idx| !valid.contains(idx))
                .collect::<Vec<_>>();
            if !invalid.is_empty() {
                warn!("Rebuilding array to include {} new elements", invalid.len());
                let num_disks = self.elements.len();
                for idx in invalid {
                    let header = &mut headers[idx];
                    *header = RaidHeader {
                        magic_bytes: MAGIC_BYTES,
                        disk_num: idx as _,
                        num_disks: num_disks
                            .try_into()
                            .expect("Cannot have more than 256 elements in a raid array"),
                        raid_id: raid_ids::RAID0,
                        array_identifier: self.array_identifier,
                    };
                    let element = &mut self.elements[idx];
                    element.store.write_typed(Ptr::null(), &*header).await?;
                }
            }
        }

        self.ready = true;

        Ok(())
    }

    fn ready(&self) -> Result<(), RaidError> {
        if self.ready {
            Ok(())
        } else {
            Err(RaidError::UseUninitialized)
        }
    }

    pub async fn print_info(&mut self) -> Result<(), RaidError> {
        self.ready()?;
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
                "\t- element {}/{}, {}\t(is: {})",
                header.disk_num + 1,
                header.num_disks,
                sfmt(elem.size as _),
                elem.store.type_name(),
            )
            .unwrap();
        }
        writeln!(&mut first_line, "{})", sfmt(total_size as _)).unwrap();
        first_line.push_str(&out);
        swap(&mut first_line, &mut out);
        info!("{out}");
        Ok(())
    }

    /// perform address translation (address in array -> element number,
    /// (address in element, len), (optional address in next element, optional len))
    ///
    /// if the optional second return thing is included, it means the write
    /// was split accross multiple elements (the number provided, and the one after it)
    async fn translate(
        &mut self,
        mut address: u64,
        len: u64,
    ) -> Result<(usize, (u64, u64), Option<(u64, u64)>), RaidError> {
        self.ready()?;
        let mut disk_num = 0;
        for (elem_idx, elem) in self.elements.iter().enumerate() {
            if address < elem.size {
                if len <= (elem.size - address) {
                    return Ok((
                        disk_num,
                        (address + size_of::<RaidHeader>() as u64, len),
                        None,
                    ));
                } else {
                    if self.elements.len() == elem_idx + 1 {
                        return Err(RaidError::OutOfBounds);
                    }
                    if (len - (elem.size - address)) > self.elements[elem_idx + 1].size {
                        unimplemented!("This write would have been split accross more than 2 array elements, which is not implemented at this time")
                    } else {
                        return Ok((
                            disk_num,
                            (
                                address + size_of::<RaidHeader>() as u64,
                                (elem.size - address),
                            ),
                            Some((
                                size_of::<RaidHeader>() as u64,
                                (len - (elem.size - address)),
                            )),
                        ));
                    }
                }
            } else {
                disk_num += 1;
                address -= elem.size;
            }
        }
        return Err(RaidError::OutOfBounds);
    }
}

#[cfg(test)]
#[tokio::test]
#[tracing_test::traced_test]
async fn translation_test() {
    use super::test::TestStore;
    let e0 = TestStore::with_size(100 + size_of::<RaidHeader>());
    let e1 = TestStore::with_size(50 + size_of::<RaidHeader>());
    let mut array = ArrayR0::new();
    array.add_element(e0).await.unwrap();
    array.add_element(e1).await.unwrap();
    array.build().await.unwrap();

    assert_eq!(
        array.translate(0, 10).await.unwrap(),
        (0, (size_of::<RaidHeader>() as u64, 10), None,)
    );

    assert_eq!(
        array.translate(90, 10).await.unwrap(), // (write to 90..=99 (10 values) elem 0 range (0..=99) (100 values))
        (0, (size_of::<RaidHeader>() as u64 + 90, 10), None,)
    );

    assert_eq!(
        array.translate(100, 10).await.unwrap(), // (write to 100..=109 (10 values) elem 0 (0..=99) (100 values) elem 1 (100..149) (50 values))
        (
            1,
            (size_of::<RaidHeader>() as u64, 10), // not +100, bc it is in the second disk
            None,
        )
    );

    assert_eq!(
        array.translate(90, 20).await.unwrap(), // (write to 100..=109 (10 values) elem 0 (0..=99) (100 values) elem 1 (100..149) (50 values))
        (
            0,
            (size_of::<RaidHeader>() as u64 + 90, 10),
            Some((size_of::<RaidHeader>() as u64, 10)),
        )
    );
}

#[async_trait::async_trait]
impl UntypedStorage for ArrayR0 {
    type Error = RaidError;
    async fn read_buf(
        &mut self,
        at: Ptr<Void>,
        amnt: u64,
        into: &mut [u8],
    ) -> Result<(), Self::Error> {
        debug_assert!(into.len() as u64 <= amnt);
        self.ready()?;
        let (elem, (addr0, len0), opt) = self.translate(at.addr, amnt).await?;
        if let Some((addr1, len1)) = opt {
            debug_assert_eq!(len0 + len1, amnt);
            // perform the reads/writes concurrently
            let (buf0, buf1) = into.split_at_mut(len0 as _);
            let (elem0, elem1) = self.elements.split_at_mut(elem + 1);
            let (res0, res1) = join!(
                elem0[elem].store.read_buf(Ptr::with(addr0), len0, buf0),
                elem1[0].store.read_buf(Ptr::with(addr1), len1, buf1),
            );
            res0?;
            res1?;
        } else {
            debug_assert_eq!(len0, amnt);
            self.elements[elem]
                .store
                .read_buf(Ptr::with(addr0), len0, into)
                .await?;
        }
        Ok(())
    }

    async fn write_buf(
        &mut self,
        at: Ptr<Void>,
        amnt: u64,
        from: &[u8],
    ) -> Result<(), Self::Error> {
        debug_assert!(from.len() as u64 <= amnt);
        self.ready()?;
        let (elem, (addr0, len0), opt) = self.translate(at.addr, amnt).await?;
        if let Some((addr1, len1)) = opt {
            debug_assert_eq!(len0 + len1, amnt);
            // perform the reads/writes concurrently
            let (buf0, buf1) = from.split_at(len0 as _);
            let (elem0, elem1) = self.elements.split_at_mut(elem + 1);
            let (res0, res1) = join!(
                elem0[elem].store.write_buf(Ptr::with(addr0), len0, buf0),
                elem1[0].store.write_buf(Ptr::with(addr1), len1, buf1),
            );
            res0?;
            res1?;
        } else {
            debug_assert_eq!(len0, amnt);
            self.elements[elem]
                .store
                .write_buf(Ptr::with(addr0), len0, from)
                .await?;
        }
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
        self.ready()?;
        Ok(self.elements.iter().map(|e| e.size).sum())
    }
    async fn expand_by(&mut self, _amnt: u64) -> Result<(), Self::Error> {
        Err(RaidError::Resize)
    }
    async fn resizeable(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
}
