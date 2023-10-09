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

use super::dyn_var::DynVar;
use anyhow::Result;
use futures::{future::BoxFuture, Future};
use tokio::{select, sync::broadcast, task::JoinSet, time::timeout};
use uuid::Uuid;

pub mod async_fn_ptr;

use crate::flag::Flag;

use self::async_fn_ptr::{AsyncFnPtr, HandlerCallableErased, HandlerFn};

use super::{
    id::{const_uuid_v4, Uid},
    msg::{self, HandlerInstance, Msg, Str},
};

#[async_trait]
pub trait HandlerInit: Send + Sync + 'static {
    const DECL: msg::HandlerType;
    // type BgGenerated: Sync + Send + 'static;
    // const BG_RUN: bool = false;
    // /// NOTE: This function MUST be cancel safe.
    // async fn bg_generate(&mut self) -> Self::BgGenerated { unimplemented!() }
    // async fn bg_consume(&mut self, _args: Self::BgGenerated, _int: LocalInterface) { unimplemented!() }
    async fn init(&mut self, _int: &LocalInterface) {}
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
                handler_desc: Str::Borrowed(decl.desc),
            },
        );
    }

    pub(in crate::bus) fn finalize(self) -> HashMap<Uuid, MethodRaw> {
        self.methods
    }
}

pub struct MethodDecl<At: 'static, Rt: 'static> {
    id: Uuid,
    desc: &'static str,
    _ph: PhantomData<&'static (At, Rt)>,
}
impl<At: 'static, Rt: 'static> Clone for MethodDecl<At, Rt> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<At: 'static, Rt: 'static> Copy for MethodDecl<At, Rt> {}
impl<At: 'static, Rt: 'static> MethodDecl<At, Rt> {
    #[doc(hidden)]
    pub const fn new(desc: &'static str) -> Self {
        Self {
            id: const_uuid_v4(),
            desc,
            _ph: PhantomData,
        }
    }
}

#[allow(unused_macros)]
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
#[allow(unused_imports)]
pub(crate) use method_decl;

#[derive(Clone)]
pub struct Interface {
    pub(in crate::bus) uid_src: Arc<AtomicU64>,
    pub(in crate::bus) comm: broadcast::Sender<Arc<Msg>>,
}

pub struct LocalInterface {
    pub nonlocal: Interface,
    pub(in crate::bus) bg_spawner: flume::Sender<(BoxFuture<'static, DynVar>, Uuid, &'static str)>,
    pub(in crate::bus) update_metadata: Flag,
    pub(in crate::bus) instance: HandlerInstance,
}

impl LocalInterface {
    /// runs `f` to completion, allowing other events to be processed in the meantime. when F completes,
    /// an event (with decl `m`) is generated *for this handler only* containing the results.
    ///
    /// This can be used for a pattern where, for example a socket's receive half is put into a background task,
    /// waits to receive, then returns itself + what it received, and finally the handler spawns the task again.
    pub fn bg_spawn<T: Sync + Send>(
        &self,
        m: MethodDecl<T, ()>,
        f: impl Future<Output = T> + Send + 'static,
    ) {
        let dyn_f: BoxFuture<'static, DynVar> = Box::pin(async move { DynVar::new(f.await) });
        let MethodDecl { id, desc, .. } = m;
        if let Err(..) = self.bg_spawner.send((dyn_f, id, desc)) {
            unreachable!("Failed to spawn background runner - handler runtime not listening");
        }
    }

    pub fn update_metadata(&self) {
        self.update_metadata.signal();
    }

    pub fn whoami(&self) -> HandlerInstance {
        self.instance.clone()
    }

    pub async fn dispatch<At: Sync + Send + 'static, Rt: 'static>(
        &self,
        target: msg::Target,
        method: MethodDecl<At, Rt>,
        args: At,
    ) -> Result<Option<Rt>> {
        self.nonlocal
            .dispatch_as(self.whoami(), target, method, args)
            .await
    }
}

impl Interface {
    pub fn spawn<H: HandlerInit>(&self, instance: H) -> HandlerInstance {
        let inter = self.clone();
        let rt = HandlerTaskRt::new(inter, instance);
        let inst = rt.id();
        tokio::spawn(async move {
            let res = rt.run().await;
            if let Err(e) = res {
                error!("Runtime task exited with error: {e:#}");
            }
        });
        inst
    }

    pub async fn dispatch_as<At: Sync + Send + 'static, Rt: 'static>(
        &self,
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
                id_desc: Str::Borrowed(method.desc),
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

pub(in crate::bus) struct HandlerTaskRt<H: HandlerInit> {
    inter: LocalInterface,
    bg_spawner_recv: flume::Receiver<(BoxFuture<'static, DynVar>, Uuid, &'static str)>,
    hdl: DynVar,
    inst: HandlerInstance,
    methods: HashMap<Uuid, MethodRaw>,
    comm: broadcast::Receiver<Arc<Msg>>,
    _ph: PhantomData<H>,
}

impl<H: HandlerInit> HandlerTaskRt<H> {
    pub fn new(inter: Interface, instance: H) -> Self {
        let discriminant = Uid::gen_with(&inter.uid_src);
        let (bg_spawner, bg_spawner_recv) = flume::unbounded();
        let comm = inter.comm.subscribe();
        let inst = HandlerInstance {
            typ: H::DECL,
            discriminant,
            discriminant_desc: Str::Owned(String::new()),
        };
        let mut rt = Self {
            inter: LocalInterface {
                nonlocal: inter,
                bg_spawner,
                update_metadata: Flag::new(),
                instance: inst.clone(),
            },
            bg_spawner_recv,
            hdl: DynVar::new(instance),
            inst,
            methods: HashMap::default(),
            comm,
            _ph: PhantomData,
        };
        rt.update_metadata();
        rt
    }

    fn update_metadata(&mut self) {
        let instance = self.hdl.as_ref::<H>().unwrap();
        let mut register = MethodRegister::new();
        instance.methods(&mut register);
        let discriminant_desc = instance.describe();
        self.methods = register.finalize();
        self.inst.discriminant_desc = discriminant_desc;
    }

    pub fn id(&self) -> HandlerInstance {
        self.inst.clone()
    }

    pub async fn run(mut self) -> Result<()> {
        self.hdl.as_mut::<H>().unwrap().init(&self.inter).await;
        let mut background = JoinSet::<(DynVar, Uuid, &'static str)>::new();
        loop {
            select! {
                message = self.comm.recv() => self.handle_message(message?).await?,
                _ = &self.inter.update_metadata => self.update_metadata(),
                // Err is unreachable
                (future, method_id, method_desc) = async { self.bg_spawner_recv.recv_async().await.unwrap() } => {
                    warn!("TODO: Re-implement as a seperate task (like HandlerTaskRt) that sends a normal `comm` message (also consider if the performance vs code size is worth it.)");
                    background.spawn(async move {
                        (future.await, method_id, method_desc)
                    });
                }
                // if None, it will be ignored (good)
                Some(result) = background.join_next() => {
                    let Ok((result, method_id, method_desc)) = result else {
                        error!("Background task panicked! - ignoring would-be return value");
                        continue
                    };
                    let Some(method_val) = self.methods.get(&method_id) else {
                        warn!("Background task would have called method on return that was not registered - its return value will be ignored");
                        continue
                    };
                    if method_val.handler_desc != method_desc {
                        warn!(
                            "method description [registered] vs [called] do not match: ({:?} vs {:?})",
                            method_val.handler_desc,
                            method_desc,
                        );
                    }
                    // TODO: pass result by-value?
                    let _output = method_val.handler_func.call(&mut self.hdl, &result, &self.inter)
                        .expect("unreachable: handler method type mismatch")
                        .await;
                }
            }
        }
        #[allow(unreachable_code)]
        anyhow::Ok(())
    }

    async fn handle_message(&mut self, message: Arc<Msg>) -> Result<()> {
        match &message.kind {
            msg::MsgKind::Request {
                source: _source,
                target,
                method,
                arguments,
                response,
            } => {
                if !self.validate_msg_target(target, method).await {
                    return Ok(());
                }
                let method_val = self.methods.get(&method.id).unwrap();
                let result = method_val
                    .handler_func
                    .call(&mut self.hdl, arguments, &self.inter)
                    .expect("unreachable: handler method type mismatch")
                    .await;
                // if a response is desired, it is sent back.
                // if not, it is dropped
                if let (msg::Target::Instance(..), Some(responder)) = (target, response) {
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
        Ok(())
    }

    async fn validate_msg_target(&mut self, target: &msg::Target, method: &msg::MethodID) -> bool {
        let target_matches = match target {
            msg::Target::Any => true,
            msg::Target::Type(typ) if typ.id == self.inst.typ.id => true,
            msg::Target::Instance(inst)
                if inst.discriminant == self.inst.discriminant
                    && inst.typ.id == self.inst.typ.id =>
            {
                true
            }
            _ => false,
        };
        let method_val = self.methods.get(&method.id);
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
        target_matches && method_exists
    }
}

/// Describes the (non-ID portion) of a method, incl its handler function
pub struct MethodRaw {
    pub handler_func: Box<(dyn HandlerCallableErased + Sync + Send)>,
    #[cfg(feature = "bus_dbg")]
    pub handler_desc: Str,
}
