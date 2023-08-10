use std::error::Error;

use super::alloc::error::AllocError;

#[derive(Debug, thiserror::Error)]
pub enum DBError<StoreError: Error> {
    #[error("error in allocator: {0:?}")]
    AllocError(#[from] AllocError<StoreError>),
    #[error("attempted to create a duplicate station or channel (ID already exists)")]
    Duplicate,
}
