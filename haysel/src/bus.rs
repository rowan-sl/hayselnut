//! Bussin
use std::sync::{atomic::AtomicU64, Arc};

use tokio::sync::broadcast;

pub mod common;
pub mod handler;
pub mod id;
pub mod msg;
#[cfg(test)]
mod test;

use msg::Msg;

/// size of the inter-handler comm queue.
/// this must be large enough that it will not fill up while a task is busy, because the queue only
/// gets rid of a message once it is received by *all* receivers.
const COMM_QUEUE_CAP: usize = 64;

/// bussin
pub struct Bus {
    /// source for generating uids (faster than Uuid::new_v4, since it only requires a single
    /// fetch_add instruction)
    uid_src: Arc<AtomicU64>,
    /// Queue that is used for ALL inter-handler/task communication. ALL of it.
    ///
    /// Arc is used to avoid cloning a (large) Msg value that will never need writing to
    /// TODO: arena allocate Msg?
    comm: broadcast::Sender<Arc<Msg>>,
}

impl Bus {
    #[instrument]
    pub async fn new() -> Self {
        let (comm, _) = broadcast::channel(COMM_QUEUE_CAP);
        Self {
            uid_src: Arc::new(AtomicU64::new(0)),
            comm,
        }
    }

    pub fn interface(&self) -> handler::Interface {
        handler::Interface {
            uid_src: self.uid_src.clone(),
            comm: self.comm.clone(),
        }
    }
}
