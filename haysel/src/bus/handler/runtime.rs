use std::{any::type_name, collections::HashMap, marker::PhantomData, sync::Arc};

use anyhow::Result;
use futures::future::BoxFuture;
use tokio::{select, sync::broadcast, task::JoinSet};
use uuid::Uuid;

use crate::{
    bus::{
        dyn_var::DynVar,
        handler::{
            decl::MethodRaw,
            interface::{local::LocalInterface, Interface},
            register::MethodRegister,
            HandlerInit,
        },
        id::Uid,
        msg::{self, HandlerInstance, Msg, Str},
    },
    misc::Flag,
};

pub(in crate::bus) struct HandlerTaskRt<H: HandlerInit> {
    inter: LocalInterface,
    bg_spawner_recv: flume::Receiver<(BoxFuture<'static, DynVar>, Uuid, &'static str)>,
    hdl: DynVar,
    inst: HandlerInstance,
    methods: HashMap<Uuid, MethodRaw>,
    comm_filtered: flume::Receiver<Arc<Msg>>,
    _ph: PhantomData<H>,
}

impl<H: HandlerInit> HandlerTaskRt<H> {
    pub fn new(inter: Interface, instance: H) -> Self {
        let discriminant = Uid::gen_with(&inter.uid_src);
        let (bg_spawner, bg_spawner_recv) = flume::unbounded();
        let mut comm = inter.comm.subscribe();
        let (cf_send, comm_filtered) = flume::bounded(512);
        let inst = HandlerInstance {
            typ: H::DECL,
            discriminant,
            discriminant_desc: Str::Owned(String::new()),
        };
        let inst2 = inst.clone();
        tokio::spawn(async move {
            let name = type_name::<H>();
            loop {
                let recvd = match comm.recv().await {
                    Ok(recvd) => recvd,
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(num_missed)) => {
                        error!("Handler task for handler {} lagged, skipped {num_missed} messages. beware!", name);
                        continue;
                    }
                };
                if match &recvd.kind {
                    msg::MsgKind::Request { target, .. } => Self::msg_target_match(&inst2, target),
                } {
                    match cf_send.try_send(recvd) {
                        Ok(()) => {}
                        Err(flume::TrySendError::Disconnected(..)) => break,
                        Err(flume::TrySendError::Full(value)) => {
                            warn!("Buffer queue for task {} is full! if this continues, the bus receiver may lag!", name);
                            if let Err(..) = cf_send.send_async(value).await {
                                break;
                            }
                        }
                    }
                }
            }
        });
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
            comm_filtered,
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
                message = self.comm_filtered.recv_async() => self.handle_message(message?).await?,
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
                if !self.msg_method_validate(method) {
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
                    if let Some(..) = responder.value.put(result) {
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

    fn msg_method_validate(&self, method: &msg::MethodID) -> bool {
        let method_val = self.methods.get(&method.id);
        method_val.is_some_and(|val| {
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
        })
    }

    fn msg_target_match(this: &HandlerInstance, target: &msg::Target) -> bool {
        match target {
            msg::Target::Any => true,
            msg::Target::Type(typ) if typ.id == this.typ.id => true,
            msg::Target::Instance(inst)
                if inst.discriminant == this.discriminant && inst.typ.id == this.typ.id =>
            {
                true
            }
            _ => false,
        }
    }
}
