use tokio::io;

#[derive(Debug, thiserror::Error)]
pub enum AllocRunnerErr {
    #[error("I/O Error: {0:?}")]
    IOError(#[from] io::Error),
    #[error("A reponse could not be sent for the processed request")]
    ResponseFailure,
    #[error("A communication queue was unexpectedly closed")]
    CommQueueClosed,
}

#[derive(Debug, thiserror::Error)]
pub enum AllocReqErr {
    #[error("An unrecoverable error within the allocator occured")]
    InternalError,
}

#[derive(Debug, thiserror::Error)]
pub enum AllocErr {
    #[error("Runner error: {0:?}")]
    Runner(#[from] AllocRunnerErr),
    #[error("Request error: {0:?}")]
    Request(#[from] AllocReqErr),
}
