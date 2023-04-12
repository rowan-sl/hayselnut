use tokio::io;

#[derive(Debug, thiserror::Error)]
pub enum AllocRunnerErr {
    #[error("I/O Error: {0:?}")]
    IOError(#[from] io::Error),
    #[error("A reponse could not be sent for the processed request")]
    ResFail,
    #[error("A communication queue was unexpectedly closed")]
    CommQueueClosed,
}

#[derive(Debug, thiserror::Error)]
pub enum AllocReqErr {
    #[error("An unrecoverable error within the allocator occured")]
    InternalError,
    #[error("The size of the read/write does not match the size of the target")]
    SizeMismatch,
    #[error("Only one object can exist for a given piece of data at one time")]
    DoubleUse,
    #[error("This data is currently in use")]
    Used,
    #[error("This data has already been freed")]
    DoubleFree,
    #[error("This object has been deallocated (use after free)")]
    UseAfterFree,
}

#[derive(Debug, thiserror::Error)]
pub enum AllocErr {
    #[error("Runner error: {0:?}")]
    Runner(#[from] AllocRunnerErr),
    #[error("Request error: {0:?}")]
    Request(#[from] AllocReqErr),
}
