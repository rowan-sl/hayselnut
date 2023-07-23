use std::error::Error;

use super::alloc::error::AllocError;

#[derive(Debug, thiserror::Error)]
pub enum DBError<StoreError: Error> {
    #[error("error in allocator: {0:?}")]
    AllocError(#[from] AllocError<StoreError>),
}
