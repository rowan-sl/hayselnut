use std::{
    collections::HashMap,
    sync::{
        atomic::{self, AtomicPtr, AtomicU64},
        Arc,
    },
    time::Duration,
};

use anyhow::Result;
use dabus::extras::DynVar;
use tokio::{spawn, sync::broadcast, time::timeout};
use uuid::Uuid;

pub mod async_fn_ptr;

use crate::flag::Flag;

use self::async_fn_ptr::HandlerCallableErased;

use super::{
    id::Uid,
    msg::{self, HandlerInstance, Msg, Str},
    MgmntMsg,
};

pub struct Interface;

pub(in crate::bus) async fn bus_dispatch_event(
    uid_src: Arc<AtomicU64>,
    comm: broadcast::Sender<Arc<Msg>>,
    _mgmnt_comm: flume::Sender<MgmntMsg>,
    source: HandlerInstance,
    target: msg::Target,
    method: msg::MethodID,
    arguments: DynVar,
) -> Result<Option<DynVar>> {
    let message_id = Uid::gen_with(&uid_src);
    let mut has_response = false;
    let response = if let msg::Target::Instance(..) = target {
        has_response = true;
        Some(msg::Responder {
            value: AtomicPtr::new(0 as *mut _),
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
    if let Err(..) = comm.send(message.clone()) {
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
            let pointer = responder.value.load(atomic::Ordering::SeqCst);
            if pointer.is_null() {
                error!("Responder waker was triggered, but no response was found");
                bail!("Received null response");
            } else {
                // Saftey: no other (correctly implemented) handlers should read and use a value.
                // TODO: safe code can do things that violate the saftey of this (by putting a
                // dangleing pointer in this field)
                let boxed = unsafe { Box::from_raw(pointer) };
                Ok(Some(*boxed))
            }
        } else {
            error!("Waiting for response timed out");
            bail!("timeout waiting for response");
        }
    } else {
        Ok(None)
    }
}

pub(in crate::bus) async fn handler_task_rt_launch(
    // Bus stuff
    uid_src: Arc<AtomicU64>,
    comm: broadcast::Sender<Arc<Msg>>,
    _mgmnt_comm: flume::Sender<MgmntMsg>,
    // handler stuff
    handler_id: Uuid,
    mut handler: DynVar,
    #[cfg(feature = "bus_dbg")] handler_desc: Str,
    method_map: HashMap<Uuid, Method>,
) -> HandlerInstance {
    // instance-spacific UID of this handler
    let handler_inst_id = Uid::gen_with(&uid_src);
    #[cfg(feature = "bus_dbg")]
    let handler_inst_desc = Str::from("todo: instance descriptions");
    let inst = HandlerInstance {
        typ: msg::HandlerType {
            id: handler_id,
            id_desc: handler_desc.clone(),
        },
        discriminant: handler_inst_id,
        discriminant_desc: handler_inst_desc.clone(),
    };
    // this must be before the task is launched, so that a handler will start receiving
    // as soon as the launch function (this one) is called.
    let mut comm_recv = comm.subscribe();
    spawn(async move {
        let res = async {
            'recv_next: loop {
                let message = comm_recv.recv().await?;
                match &message.kind {
                    msg::MsgKind::Request {
                        source: _source,
                        target,
                        method,
                        arguments,
                        response,
                    } => {
                        let target_matches = match target {
                            msg::Target::Any => true,
                            msg::Target::Type(typ) if typ.id == handler_id => true,
                            msg::Target::Instance(inst)
                                if inst.discriminant == handler_inst_id
                                    && inst.typ.id == handler_id => true,
                            _ => false,
                        };
                        let method_val = method_map.get(&method.id);
                        let method_exists = method_val.is_some_and(|val| {
                            #[cfg(feature = "bus_dbg")]
                            {
                                if method.id_desc != val.handler_desc {
                                    error!("Method description (for method {}) is not consistant between the discription sent with the request, and the stored description! ({} vs {}) - this event will be ignored as if the ID had not matched", method.id, method.id_desc, val.handler_desc);
                                    warn!("TODO: inform the management task about this");
                                    false
                                } else {
                                    true
                                }
                            }
                            #[cfg(not(feature = "bus_dbg"))]
                            { true }
                        });
                        if !(target_matches && method_exists) {
                            // message is irrelevant
                            continue 'recv_next;
                        }
                        let func = &method_val.unwrap().handler_func;
                        match func.call(&mut handler, arguments, Interface) {
                            Ok(future) => {
                                let result = future.await;
                                // if a response is desired, it is sent back.
                                // if not, it is dropped
                                if let msg::Target::Instance(..) = target {
                                    if let Some(responder) = response {
                                        let boxed = Box::new(result);
                                        let pointer = Box::into_raw(boxed);
                                        if let Err(pointer) = responder.value.compare_exchange(
                                            0 as *mut DynVar,
                                            pointer,
                                            atomic::Ordering::SeqCst,
                                            atomic::Ordering::SeqCst,
                                        ) {
                                            // de-allocate fail_pointer to avoid memory leak
                                            // Saftey: if compare_exchange fails, then the pointer could not possibly
                                            // have been seen (much less used) by any other tasks
                                            unsafe {
                                                // value is dropped at the end of the unsafe block (dropbox???)
                                                let _boxed = Box::from_raw(pointer);
                                            }
                                            // now, who tf caused this??
                                            warn!("Spacific instance was targeted, but multiple instances accepted (response already contains a value)");
                                        } else {
                                            // wake the receiving task
                                            responder.waker.signal();
                                        }
                                    }
                                }
                            }
                            Err(err) => {
                                #[cfg(not(feature = "bus_dbg"))]
                                {
                                    error!("Handler type mismatch {err:?} - enable the bus_dbg feature for more details")
                                }
                                #[cfg(feature = "bus_dbg")]
                                {
                                    error!("Hnadler type mismatch {err:?} (method: {}, handler: {}, instance: {})", method_val.unwrap().handler_desc, handler_desc, handler_inst_desc);
                                }
                            }
                        };
                    }
                }
            }
            #[allow(unreachable_code)]
            anyhow::Ok(())
        }
        .await;
        match res {
            Ok(()) => {
                todo!()
            }
            Err(err) => {
                todo!()
            }
        }
    });
    inst
}

/// Describes the (non-ID portion) of a method, incl its handler function
pub struct Method {
    pub handler_func: Box<(dyn HandlerCallableErased + Sync + Send)>,
    #[cfg(feature = "bus_dbg")]
    pub handler_desc: Str,
}
