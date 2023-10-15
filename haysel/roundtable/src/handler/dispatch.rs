use std::{sync::Arc, time::Duration};

use anyhow::Result;
use tokio::time::timeout;

use crate::{
    atomic_cell::AtomicCell,
    dyn_var::DynVar,
    flag::Flag,
    handler::interface::Interface,
    id::Uid,
    msg::{self, HandlerInstance},
};

pub async fn bus_dispatch_event(
    int: Interface,
    source: HandlerInstance,
    target: msg::Target,
    method: msg::MethodID,
    arguments: DynVar,
    want_response: bool,
    want_verification: bool,
) -> Result<Option<DynVar>> {
    let message_id = Uid::gen_with(&int.uid_src);
    let response = if let msg::Target::Instance(..) = target {
        if want_response {
            msg::Responder::Respond {
                value: AtomicCell::new(),
                waker: Flag::new(),
            }
        } else if want_verification {
            msg::Responder::Verify { waker: Flag::new() }
        } else {
            msg::Responder::NoVerify
        }
    } else {
        msg::Responder::NoVerify
    };
    let message = Arc::new(msg::Msg {
        id: message_id,
        kind: msg::MsgKind::Request {
            source,
            target,
            method,
            arguments,
            response,
        },
    });
    // avoid erroring when no tasks are watching the channel
    if let Err(..) = int.comm.send(message.clone()) {
        trace!("Sent message, but no one is listening - silently failing");
        return Ok(None);
    }
    #[allow(irrefutable_let_patterns)]
    let msg::MsgKind::Request {
        response: responder,
        ..
    } = &message.kind
    else {
        unreachable!()
    };

    match responder {
        msg::Responder::NoVerify => Ok(None),
        msg::Responder::Verify { waker } => {
            let Ok(..) = timeout(Duration::from_secs(15), waker).await else {
                bail!("No handler handled the message within the given timeout");
            };
            Ok(None)
        }
        msg::Responder::Respond { value, waker } => {
            if let Ok(..) = timeout(Duration::from_secs(15), waker).await {
                let res = value.take();
                if res.is_none() {
                    error!("Responder waker was triggered, but no response was found");
                    bail!("Received null response");
                } else {
                    Ok(res.map(|x| *x))
                }
            } else {
                error!("Waiting for response timed out");
                bail!("timeout waiting for response");
            }
        }
    }
}
