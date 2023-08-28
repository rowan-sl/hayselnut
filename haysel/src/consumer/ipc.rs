use std::path::PathBuf;

use anyhow::Result;
use flume::Sender;
use mycelium::IPCMsg;
use tokio::spawn;

use crate::{
    ipc::{ipc_task, IPCTaskMsg},
    route::StationInfoUpdate,
    shutdown::Shutdown,
};

use super::{Record, RecordConsumer};

pub struct IPCConsumer {
    shutdown: Shutdown,
    ipc_task_tx: Sender<IPCTaskMsg>,
}

impl IPCConsumer {
    pub async fn new(ipc_sock: PathBuf) -> Result<Self> {
        let shutdown = Shutdown::new();
        let handle = shutdown.handle();
        let (ipc_task_tx, ipc_task_rx) = flume::unbounded::<IPCTaskMsg>();
        spawn(ipc_task(handle, ipc_task_rx, ipc_sock));
        Ok(Self {
            shutdown,
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

    async fn close(mut self: Box<Self>) {
        self.shutdown.trigger_shutdown();
        self.shutdown.wait_for_completion().await;
    }
}
