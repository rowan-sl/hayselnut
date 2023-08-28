use std::path::PathBuf;

use anyhow::Result;
use flume::Sender;
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
            data,
            recorded_at,
            recorded_by,
        }: &Record,
    ) -> Result<()> {
        Ok(())
    }

    async fn update_station_info(&mut self, updates: &[StationInfoUpdate]) -> Result<()> {
        for update in updates {
            match update {
                StationInfoUpdate::InitialState { stations, channels } => {}
                &StationInfoUpdate::NewStation { id } => {}
                // the database does not track channels independantly of stations
                StationInfoUpdate::NewChannel { id, ch } => {}
                &StationInfoUpdate::StationNewChannel { station, channel } => {}
            }
        }
        Ok(())
    }

    async fn close(self: Box<Self>) {}
}
