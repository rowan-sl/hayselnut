use std::path::PathBuf;

use anyhow::Result;
use flume::Receiver;
use mycelium::{
    station::{capabilities::KnownChannels, identity::KnownStations},
    IPCMsg,
};
use tokio::{net::UnixListener, select, spawn, sync};

use crate::shutdown::ShutdownHandle;

use super::shutdown::Shutdown;

#[derive(Debug)]
pub enum IPCTaskMsg {
    Broadcast(IPCMsg),
    UpdateInfo {
        stations: KnownStations,
        channels: KnownChannels,
    },
}

pub async fn ipc_task(
    mut handle: ShutdownHandle,
    ipc_task_rx: Receiver<IPCTaskMsg>,
    ipc_sock_path: PathBuf,
) -> Result<()> {
    let mut shutdown_ipc = Shutdown::new();
    let listener = UnixListener::bind(ipc_sock_path.clone()).unwrap();
    let (ipc_broadcast_queue, _) = sync::broadcast::channel::<IPCMsg>(10);
    let (mut cache_stations, mut cache_channels) = (KnownStations::new(), KnownChannels::new());

    let res = async {
        loop {
            select! {
                _ = handle.wait_for_shutdown() => { break; }
                recv = ipc_task_rx.recv_async() => {
                    match recv? {
                        IPCTaskMsg::Broadcast(msg) => {
                            let num = ipc_broadcast_queue.send(msg).unwrap_or(0);
                            trace!("Sent IPC message to {num} IPC clients");
                        }
                        IPCTaskMsg::UpdateInfo { stations, channels } => {
                            cache_stations = stations.clone();
                            cache_channels = channels.clone();
                            let num = ipc_broadcast_queue.send(IPCMsg { kind: mycelium::IPCMsgKind::Info {
                                stations,
                                channels,
                            }}).unwrap_or(0);
                            trace!("Sent updated station/channel info to {num} IPC clients");
                        }
                    };
                }
                res = listener.accept() => {
                    let (mut sock, addr) = res?;
                    let mut recv = ipc_broadcast_queue.subscribe();
                    let mut handle = shutdown_ipc.handle();
                    debug!("Connecting to new IPC client at {addr:?}");
                    let initial_packet = IPCMsg { kind: mycelium::IPCMsgKind::Info {
                        stations: cache_stations.clone(),
                        channels: cache_channels.clone(),
                    }};
                    spawn(async move {
                        let res = async move {
                            mycelium::ipc_send(&mut sock, &initial_packet).await?;
                            loop {
                                select! {
                                    // TODO: notify clients of server closure
                                    _ = handle.wait_for_shutdown() => { break; }
                                    res = recv.recv() => {
                                        mycelium::ipc_send(&mut sock, &res?).await?
                                    }
                                }
                            }
                            Ok::<(), anyhow::Error>(())
                        }.await;
                        debug!("IPC client {addr:?} disconnected");
                        match res {
                            Ok(()) => {}
                            Err(e) => error!("IPC task (for: addr:?) exited with error: {e:?}"),
                        }
                    });
                }
            };
        }
        Ok::<(), anyhow::Error>(())
    }.await;
    drop(listener);
    let _ = tokio::fs::remove_file(ipc_sock_path).await;
    shutdown_ipc.trigger_shutdown();
    shutdown_ipc.wait_for_completion().await;
    res.unwrap();
    Ok(())
}
