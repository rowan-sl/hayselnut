use std::{sync::Arc, time::Duration};

use anyhow::Result;
use tokio::time::timeout;

use crate::{
    atomic_cell::AtomicCell,
    dyn_var::DynVar,
    flag::Flag,
    handler::interface::Interface,
    id::Uid,
    msg::{self, HandlerInstance, ResponseErr},
};

#[derive(Debug, Clone, thiserror::Error)]
pub enum DispatchErr {
    #[error("No handlers handled the message: {0}")]
    NoResponse(&'static str),
    #[error("A response was indicated, but it contained no value")]
    NullResponse,
    #[error("An error occured while handling request: {0:#}")]
    HandlerError(#[from] ResponseErr),
}

pub async fn bus_dispatch_event(
    int: Interface,
    source: HandlerInstance,
    target: msg::Target,
    method: msg::MethodID,
    arguments: DynVar,
    want_response: bool,
    want_verification: bool,
) -> Result<Option<DynVar>, DispatchErr> {
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
        if want_response || want_verification {
            return Err(DispatchErr::NoResponse("no active handlers"));
        }
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
                return Err(DispatchErr::NoResponse("timed out"));
            };
            Ok(None)
        }
        msg::Responder::Respond { value, waker } => {
            if let Ok(..) = timeout(Duration::from_secs(15), waker).await {
                let res = value.take();
                if res.is_none() {
                    error!("Responder waker was triggered, but no response was found");
                    return Err(DispatchErr::NullResponse);
                } else {
                    match res.map(|x| *x) {
                        Some(Ok(ret)) => Ok(Some(ret)),
                        Some(Err(e)) => Err(e)?,
                        None => Ok(None),
                    }
                }
            } else {
                error!("Waiting for response timed out");
                if value.take().is_some() {
                    error!("BUG: Response waker was not woken, but a response was given!");
                }
                return Err(DispatchErr::NoResponse("timed out"));
            }
        }
    }
}
