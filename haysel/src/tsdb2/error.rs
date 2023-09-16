use std::error::Error;

use super::{alloc::error::AllocError, repr::TuningParams};

#[derive(Debug, thiserror::Error)]
pub enum DBError<StoreError: Error> {
    #[error("error in allocator: {0:?}")]
    AllocError(#[from] AllocError<StoreError>),
    #[error("attempted to create a duplicate station or channel (ID already exists)")]
    Duplicate,
    #[error("mismatched tuning parameters (database was saved using different parameters than currently in use)\nexpected:{expected:?}\nfound:{found:?}")]
    MismatchedParameters {
        expected: TuningParams,
        found: TuningParams,
    },
    #[error("query: station does not exist")]
    NoSuchStation,
    #[error("query: channel does not exist")]
    NoSuchChannel,
}
