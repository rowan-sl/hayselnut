//! Bussin

#![allow(incomplete_features)]
#![feature(specialization)]
#![feature(downcast_unchecked)]

#[macro_use]
extern crate tracing;
#[macro_use]
extern crate anyhow;
#[macro_use]
extern crate async_trait;
#[doc(hidden)]
pub extern crate const_random;
#[doc(hidden)]
pub extern crate uuid;

use std::{
    ops::Deref,
    sync::{atomic::AtomicU64, Arc},
};

use tokio::{spawn, sync::broadcast};

mod atomic_cell;
pub mod common;
mod dyn_var;
mod flag;
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
        let mut recv = comm.subscribe();
        spawn(async move {
            loop {
                let msg: Arc<msg::Msg> = recv.recv().await.unwrap();
                match &msg.kind {
                    msg::MsgKind::Request {
                        source,
                        target,
                        method,
                        arguments: _,
                        response: _,
                    } => {
                        trace!(
                            "bus event: request\n\tby: {} - {}\n\ttarget: {}{}\n\tmethod: {}",
                            source.typ.id_desc,
                            source.discriminant_desc,
                            match target {
                                msg::Target::Any => "".to_string(),
                                msg::Target::Type(hdl_typ) => format!("{} - ", hdl_typ.id_desc),
                                msg::Target::Instance(inst) => format!("{} - ", inst.typ.id_desc),
                            },
                            match target {
                                msg::Target::Any => "[any]".to_string(),
                                msg::Target::Type(hdl_typ) => hdl_typ.id_desc.to_string(),
                                msg::Target::Instance(inst) => inst.discriminant_desc.to_string(),
                            },
                            method.id_desc,
                        );
                    }
                }
            }
        });
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
