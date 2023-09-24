//! Bussin
#[cfg(feature = "bus_dbg")]
use std::borrow::Cow;
use std::sync::{
    atomic::{self, AtomicPtr, AtomicU64},
    Arc,
};

use const_random::const_random;
use dabus::extras::DynVar;
use futures::FutureExt;
use tokio::{spawn, sync::broadcast, task::JoinHandle};
use uuid::Uuid;

use crate::{flag::Flag, util::Take};

/// size of the inter-handler comm queue.
/// this must be large enough that it will not fill up while a task is busy, because the queue only
/// gets rid of a message once it is received by *all* receivers.
const COMM_QUEUE_CAP: usize = 64;
/// size of the management task communication queue. probably doesn't need to be that large.
const MGMNT_QUEUE_CAP: usize = 64;

/// NON UNIVERSALLY unique identifier
///
/// all Uids that are compared with each other must come from the same `source`
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Uid(u64);

impl Uid {
    /// generates a new Unique identifer by taking the current value in `source` and incrementing
    /// it by 1. this will generate unique ids, as long as they are only compared to values coming
    /// from the same source.
    fn gen_with(source: &AtomicU64) -> Self {
        Self(source.fetch_add(1, atomic::Ordering::Relaxed))
    }
}

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
    /// Queue that is used for communication with the management task. (for example, reporting
    /// errors or misbehavior of other handlers)
    mgmnt_comm: flume::Sender<MgmntMsg>,
    /// Join handle for the management task. this can be checked in on to make sure nothing has
    /// gone too horribly wrong.
    mgmnt_task: Take<JoinHandle<()>>,
}

impl Bus {
    #[instrument]
    pub async fn new() -> Self {
        let (comm, _) = broadcast::channel(COMM_QUEUE_CAP);
        let (mgmnt_comm, mgmnt_comm_recv) = flume::bounded(MGMNT_QUEUE_CAP);
        let mut bus = Self {
            uid_src: Arc::new(AtomicU64::new(0)),
            comm,
            mgmnt_comm,
            mgmnt_task: Take::empty(),
        };
        let mgnmt_task = mgmnt_launch(&bus, mgmnt_comm_recv).await;
        bus.mgmnt_task.put(mgnmt_task);
        bus
    }
}

async fn handler_task_rt_launch(
    // Bus stuff
    uid_src: Arc<AtomicU64>,
    comm: broadcast::Sender<Arc<Msg>>,
    _mgmnt_comm: flume::Sender<MgmntMsg>,
    // handler stuff
    handler_id: Uuid,
    #[cfg(feature = "bus_dbg")] _handler_desc: Str,
) {
    // instance-spacific UID of this handler
    let handler_inst_id = Uid::gen_with(&uid_src);
    #[cfg(feature = "bus_dbg")]
    let _handler_inst_desc = Str::from("todo: instance descriptions");
    spawn(async move {
        let res = async {
            let mut comm_recv = comm.subscribe();
            'recv_next: loop {
                let message = comm_recv.recv().await?;
                match &message.kind {
                    MsgKind::Request {
                        source,
                        target,
                        method,
                        arguments,
                        response,
                    } => {
                        if !match target {
                            Target::Any => true,
                            Target::Type(typ) if typ.id == handler_id => true,
                            Target::Instance(inst)
                                if inst.discriminant == handler_inst_id
                                    && inst.typ.id == handler_id =>
                            {
                                true
                            }
                            _ => false,
                        } {
                            // message is irrelevant
                            continue 'recv_next;
                        }
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
}

async fn mgmnt_launch(bus: &Bus, mgmnt_comm: flume::Receiver<MgmntMsg>) -> JoinHandle<()> {
    let uid_src = bus.uid_src.clone();
    spawn(async {})
}

/// type commonly used in bus_dbg variables. can be &'static str or String
#[cfg(feature = "bus_dbg")]
type Str = Cow<'static, str>;

/// Generates a random Uuid at compile time
const fn const_uuid_v4() -> Uuid {
    uuid::Builder::from_u128(const_random!(u128)).into_uuid()
}

/// the ID used to identify a particular handler on a method (const UUID)
#[derive(Debug)]
struct MethodID {
    /// the UUID of this method
    pub id: Uuid,
    /// debug-only description of the method
    #[cfg(feature = "bus_dbg")]
    pub id_desc: Str,
}

/// describe a type of handler (UUID, a constant associated with that handler) (similar to a struct's type)
#[derive(Debug)]
struct HandlerType {
    /// the UUID of this type
    pub id: Uuid,
    /// debug-only description of the type
    #[cfg(feature = "bus_dbg")]
    pub id_desc: Str,
}

/// describe an instance of a spacific handler type (similar to a struct instance)
/// (UID, associated with an instance)
#[derive(Debug)]
struct HandlerInstance {
    /// the UUID of the handler type
    pub typ: HandlerType,
    /// the UID of this instance
    pub discriminant: Uid,
    /// debug-only description of the instance
    #[cfg(feature = "bus_dbg")]
    pub discriminant_desc: Str,
}

/// a channel used for sending a single response to a query.
#[derive(Debug)]
struct Responder {
    /// the response value. when a handler wants to set this value, it must first box the value,
    /// then use compare_exchange(current = null, new = Box::into_raw, Relaxed, Relaxed).
    /// if this fails, than it is made aware of the fact that some other handler has (erronously,
    /// given that `from` and `discriminant` are specified and can raise an error accordingly)
    ///
    /// After this is done (if successfull) the `response_waker` should be woke
    /// to trigger the requesting task to check for this value
    pub value: AtomicPtr<DynVar>,
    /// see `value`
    pub waker: Flag,
}

/// the target for a request message (instance, any type, or any)
#[derive(Debug)]
enum Target {
    /// this spacific instance of a handler
    Instance(HandlerInstance),
    /// all handlers of this type
    Type(HandlerType),
    /// any handlers
    Any,
}

#[derive(Debug)]
struct Msg {
    /// UID - generated at message send time
    pub id: Uid,
    /// content of the message
    pub kind: MsgKind,
}

#[derive(Debug)]
enum MsgKind {
    /// A request of one or more handlers
    Request {
        /// the handler instance that is sending this request
        source: HandlerInstance,
        /// the handler(s) this request is sent to
        target: Target,
        /// the 'method' on the handler being requested (note that method ids being used across
        /// handlers will imply that bolth handlers implement the given method)
        method: MethodID,
        /// (optional) arguments of the request.
        arguments: Option<DynVar>,
        /// the response channel (if None, no response is desired)
        /// this *must* be None when using Target::(Type | Any)
        response: Option<Responder>,
    },
}

#[derive(Debug)]
struct MgmntMsg {}
