use std::error::Error;

#[derive(thiserror::Error, Debug, Clone)]
pub enum AllocError<E: Error> {
    #[error("error in underlying storage: {0:#?}")]
    StoreError(#[from] E),
    #[error("the data contained in the store given to this allocator is not valid")]
    StoreNotAnAllocator,
    #[error("data in the store is corrupt or misinterpreted")]
    Corrupt,
    #[error("allocator free list has filled up!")]
    FreeListFull,
}
