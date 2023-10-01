use std::{
    any::type_name,
    collections::HashMap,
    marker::PhantomData,
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

use self::async_fn_ptr::{AsyncFnPtr, HandlerCallableErased, HandlerFn};

use super::{
    id::{const_uuid_v4, Uid},
    msg::{self, HandlerInstance, Msg, Str},
};

pub trait HandlerInit: Send + Sync + 'static {
    const DECL: msg::HandlerType;
    // description of this handler instance
    fn describe(&self) -> Str;
    // methods of this handler instance
    fn methods(&self, register: &mut MethodRegister<Self>);
}

pub struct MethodRegister<H: HandlerInit + ?Sized> {
    methods: HashMap<Uuid, MethodRaw>,
    _ph: PhantomData<H>,
}

impl<H: HandlerInit> MethodRegister<H> {
    pub(in crate::bus) fn new() -> Self {
        Self {
            methods: HashMap::new(),
            _ph: PhantomData,
        }
    }

    pub fn register<
        At: Send + Sync + 'static,
        Rt: Send + Sync + 'static,
        Fn: for<'a> AsyncFnPtr<'a, H, At, Rt> + Copy + Sync + Send + 'static,
    >(
        &mut self,
        func: Fn,
        decl: MethodDecl<At, Rt>,
    ) {
        self.methods.insert(
            decl.id,
            MethodRaw {
                handler_func: Box::new(HandlerFn::new(func)),
                handler_desc: decl.desc.clone(),
            },
        );
    }

    pub(in crate::bus) fn finalize(self) -> HashMap<Uuid, MethodRaw> {
        self.methods
    }
}

pub struct MethodDecl<At: 'static, Rt: 'static> {
    id: Uuid,
    desc: Str,
    _ph: PhantomData<&'static (At, Rt)>,
}

impl<At: 'static, Rt: 'static> MethodDecl<At, Rt> {
    #[doc(hidden)]
    pub const fn new(desc: &'static str) -> Self {
        Self {
            id: const_uuid_v4(),
            desc: Str::Borrowed(desc),
            _ph: PhantomData,
        }
    }
}

macro_rules! method_decl {
    ($name:ident, $arg:ty, $ret:ty) => {
        pub const $name: $crate::bus::handler::MethodDecl<$arg, $ret> =
            $crate::bus::handler::MethodDecl::new(concat!(stringify!($name)));
    };
}

macro_rules! handler_decl_t {
    ($desc:literal) => {
        $crate::bus::msg::HandlerType {
            id: $crate::bus::id::const_uuid_v4(),
            #[cfg(feature = "bus_dbg")]
            id_desc: Str::Borrowed($desc),
        }
    };
}

pub(crate) use handler_decl_t;
pub(crate) use method_decl;

pub const EXTERNAL: HandlerInstance = HandlerInstance {
    typ: handler_decl_t!("External event dispatcher"),
    discriminant: Uid::nill(),
    discriminant_desc: Str::Borrowed("External event dispatcher"),
};

#[derive(Clone)]
pub struct Interface {
    pub(in crate::bus) uid_src: Arc<AtomicU64>,
    pub(in crate::bus) comm: broadcast::Sender<Arc<Msg>>,
}

impl Interface {
    pub async fn spawn<H: HandlerInit>(&mut self, instance: H) -> HandlerInstance {
        let mut register = MethodRegister::new();
        instance.methods(&mut register);
        let desc = instance.describe();
        handler_task_rt_launch(
            self.clone(),
            H::DECL.id,
            DynVar::new(instance),
            #[cfg(feature = "bus_dbg")]
            H::DECL.id_desc,
            register.finalize(),
            #[cfg(feature = "bus_dbg")]
            desc,
        )
        .await
    }

    pub async fn dispatch_as<At: Sync + Send + 'static, Rt: 'static>(
        &mut self,
        source: HandlerInstance,
        target: msg::Target,
        method: MethodDecl<At, Rt>,
        args: At,
    ) -> Result<Option<Rt>> {
        if let Some(ret) = bus_dispatch_event(
            self.clone(),
            source,
            target,
            msg::MethodID {
                id: method.id,
                id_desc: method.desc,
            },
            DynVar::new(args),
        )
        .await?
        {
            match ret.try_to() {
                Ok(ret) => Ok(Some(ret)),
                Err(ret) => {
                    error!(
                        "Mismatched return type - expected {}, found {}",
                        type_name::<Rt>(),
                        ret.type_name()
                    );
                    bail!("Mismatched return type");
                }
            }
        } else {
            Ok(None)
        }
    }
}

pub(in crate::bus) async fn bus_dispatch_event(
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
    int: Interface,
    // handler stuff
    handler_id: Uuid,
    mut handler: DynVar,
    #[cfg(feature = "bus_dbg")] handler_desc: Str,
    method_map: HashMap<Uuid, MethodRaw>,
    #[cfg(feature = "bus_dbg")] handler_inst_desc: Str,
) -> HandlerInstance {
    // instance-spacific UID of this handler
    let handler_inst_id = Uid::gen_with(&int.uid_src);
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
    let mut comm_recv = int.comm.subscribe();
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
                        match func.call(&mut handler, arguments, int.clone()) {
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
pub struct MethodRaw {
    pub handler_func: Box<(dyn HandlerCallableErased + Sync + Send)>,
    #[cfg(feature = "bus_dbg")]
    pub handler_desc: Str,
}
