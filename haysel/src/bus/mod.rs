//! Bussin
use std::{
    ops::Deref,
    sync::{atomic::AtomicU64, Arc},
};

use tokio::sync::broadcast;

pub mod atomic_cell;
pub mod common;
pub mod dyn_var;
pub mod handler;
pub mod id;
pub mod msg;
#[cfg(test)]
mod test;

use self::handler::Interface;

/// size of the inter-handler comm queue.
/// this must be large enough that it will not fill up while a task is busy, because the queue only
/// gets rid of a message once it is received by *all* receivers.
const COMM_QUEUE_CAP: usize = 64;

/// bussin
pub struct Bus {
    int: Interface,
}

impl Bus {
    #[instrument]
    pub async fn new() -> Self {
        let (comm, _) = broadcast::channel(COMM_QUEUE_CAP);
        Self {
            int: Interface {
                uid_src: Arc::new(AtomicU64::new(0)),
                comm,
            },
        }
    }

    #[allow(dead_code)]
    pub fn interface(&self) -> handler::Interface {
        self.int.clone()
    }
}

impl Deref for Bus {
    type Target = handler::Interface;
    fn deref(&self) -> &Self::Target {
        &self.int
    }
}
