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

pub(crate) async fn bus_dispatch_event(
    int: Interface,
    source: HandlerInstance,
    target: msg::Target,
    method: msg::MethodID,
    arguments: DynVar,
) -> Result<Option<DynVar>> {
    let message_id = Uid::gen_with(&int.uid_src);
    let mut has_response = false;
    let response = if let msg::Target::Instance(..) = target {
        has_response = true;
        Some(msg::Responder {
            value: AtomicCell::new(),
            waker: Flag::new(),
        })
    } else {
        None
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
    if has_response {
        let msg::MsgKind::Request {
            response: Some(responder),
            ..
        } = &message.kind
        else {
            unreachable!()
        };
        if let Ok(..) = timeout(Duration::from_secs(60), &responder.waker).await {
            let res = responder.value.take();
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
    } else {
        Ok(None)
    }
}
