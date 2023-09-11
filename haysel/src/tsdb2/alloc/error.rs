use std::error::Error;

#[derive(thiserror::Error, Debug, Clone, PartialEq)]
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
        "attempted to use a pointer that does not point to valid data tracked by the allocator"
    )]
    PointerInvalid,
    #[error("attempted to use a pointer that does not match the allocated data it points to")]
    PointerMismatch,
    #[error("the expected status (free or in use) of the given pointer is incorrect for the operation used with it")]
    PointerStatus,
    #[error(
        "the parameters of the allocator in <store> are different than thoes currently configured"
    )]
    MismatchedParameters,
}
