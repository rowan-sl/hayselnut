use std::path::PathBuf;

use anyhow::Result;
use flume::Sender;
use mycelium::IPCMsg;
use tokio::spawn;

use crate::{
    ipc::{ipc_task, IPCTaskMsg},
    route::StationInfoUpdate,
    shutdown::{async_drop::AsyncDrop, Shutdown, ShutdownHandle},
    util::Take,
};

use super::{Record, RecordConsumer};

pub struct IPCConsumer {
    /// local shutdown
    shutdown: Take<Shutdown>,
    /// global shutdown
    drop: AsyncDrop,
    ipc_task_tx: Sender<IPCTaskMsg>,
}

impl IPCConsumer {
    pub async fn new(ipc_sock: PathBuf, handle: ShutdownHandle) -> Result<Self> {
        let shutdown = Shutdown::new();
        let (ipc_task_tx, ipc_task_rx) = flume::unbounded::<IPCTaskMsg>();
        spawn(ipc_task(shutdown.handle(), ipc_task_rx, ipc_sock));
        Ok(Self {
            shutdown: Take::new(shutdown),
            drop: AsyncDrop::new(handle).await,
            ipc_task_tx,
        })
    }
}

#[async_trait]
impl RecordConsumer for IPCConsumer {
    async fn handle(
        &mut self,
        Record {
            recorded_at,
            recorded_by,
            data,
        }: &Record,
    ) -> Result<()> {
        self.ipc_task_tx
            .send_async(IPCTaskMsg::Broadcast(IPCMsg {
                kind: mycelium::IPCMsgKind::FreshHotData {
                    from: *recorded_by,
                    recorded_at: *recorded_at,
                    by_channel: data.clone(),
                },
            }))
            .await?;
        Ok(())
    }

    async fn update_station_info(&mut self, updates: &[StationInfoUpdate]) -> Result<()> {
        self.ipc_task_tx
            .send_async(IPCTaskMsg::StationInfoUpdates(updates.to_vec()))
            .await?;
        Ok(())
    }
}

impl Drop for IPCConsumer {
    fn drop(&mut self) {
        let shutdown = self.shutdown.take();
        self.drop.run(async {
            shutdown.trigger_shutdown();
            shutdown.wait_for_completion().await;
        })
    }
}
