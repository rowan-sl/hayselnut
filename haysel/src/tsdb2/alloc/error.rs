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
    #[error(
        "attempted to free a pointer that does not point to valid data tracked by the allocator"
    )]
    FreeInvalidPointer,
    #[error("attempted to free allocated data using a pointer that does not match it!")]
    FreeMismatch,
    #[error("attempted to free a pointer that points to already free data")]
    DoubleFree,
}
