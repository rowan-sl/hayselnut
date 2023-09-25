//! Bussin
use std::sync::{atomic::AtomicU64, Arc};

use tokio::{spawn, sync::broadcast, task::JoinHandle};
use uuid::Uuid;

use crate::util::Take;

pub mod handler;
pub mod id;
pub mod msg;

use id::Uid;
use msg::{Msg, MsgKind, Str};

/// size of the inter-handler comm queue.
/// this must be large enough that it will not fill up while a task is busy, because the queue only
/// gets rid of a message once it is received by *all* receivers.
const COMM_QUEUE_CAP: usize = 64;
/// size of the management task communication queue. probably doesn't need to be that large.
const MGMNT_QUEUE_CAP: usize = 64;

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

async fn mgmnt_launch(bus: &Bus, mgmnt_comm: flume::Receiver<MgmntMsg>) -> JoinHandle<()> {
    let uid_src = bus.uid_src.clone();
    spawn(async {})
}

#[derive(Debug)]
struct MgmntMsg {}
