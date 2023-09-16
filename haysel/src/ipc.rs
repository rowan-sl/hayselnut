use std::path::PathBuf;

use anyhow::Result;
use flume::Receiver;
use mycelium::{
    station::{capabilities::KnownChannels, identity::KnownStations},
    IPCMsg,
};
use tokio::{net::UnixListener, select, spawn, sync};

use crate::{route::StationInfoUpdate, shutdown::ShutdownHandle};

use super::shutdown::Shutdown;

#[derive(Debug)]
pub enum IPCTaskMsg {
    Broadcast(IPCMsg),
    StationInfoUpdates(Vec<StationInfoUpdate>),
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

    let res = async { 'res: {
        select! {
            _ = handle.wait_for_shutdown() => { break 'res Ok(()) }
            recv = ipc_task_rx.recv_async() => {
                let IPCTaskMsg::StationInfoUpdates(updates) = recv? else {
                    unreachable!("First packet was not an info update");
                };
                let [StationInfoUpdate::InitialState { stations, channels }] = &updates[..] else {
                    unreachable!("First packet was not the initial state packet");
                };
                cache_stations = stations.clone();
                cache_channels = channels.clone();
            }
        }
        loop {
            select! {
                _ = handle.wait_for_shutdown() => {
                    break;
                }
                recv = ipc_task_rx.recv_async() => {
                    handle_recv(
                        recv?,
                        &ipc_broadcast_queue,
                        &mut cache_stations,
                        &mut cache_channels
                    ).await?
                }
                res = listener.accept() => {
                    let (mut sock, addr) = res?;
                    let mut recv = ipc_broadcast_queue.subscribe();
                    let mut handle = shutdown_ipc.handle();
                    debug!("Connecting to new IPC client at {addr:?}");
                    let initial_packet = IPCMsg { kind: mycelium::IPCMsgKind::Haiii {
                        stations: cache_stations.clone(),
                        channels: cache_channels.clone(),
                    }};
                    spawn(async move {
                        let res = async move {
                            mycelium::ipc_send(&mut sock, &initial_packet).await?;
                            loop {
                                select! {
                                    // TODO: notify clients of server closure
                                    _ = handle.wait_for_shutdown() => {
                                        mycelium::ipc_send(&mut sock, &IPCMsg { kind: mycelium::IPCMsgKind::Bye }).await?;
                                        break;
                                    }
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
    }}.await;
    drop(listener);
    let _ = tokio::fs::remove_file(ipc_sock_path).await;
    shutdown_ipc.trigger_shutdown();
    shutdown_ipc.wait_for_completion().await;
    res.unwrap();
    Ok(())
}

pub async fn handle_recv(
    recv: IPCTaskMsg,
    ipc_broadcast_queue: &sync::broadcast::Sender<IPCMsg>,
    cache_stations: &mut KnownStations,
    cache_channels: &mut KnownChannels,
) -> Result<()> {
    match recv {
        IPCTaskMsg::Broadcast(msg) => {
            let num = ipc_broadcast_queue.send(msg).unwrap_or(0);
            trace!("Sent IPC message to {num} IPC clients");
        }
        IPCTaskMsg::StationInfoUpdates(updates) => {
            for update in updates {
                match update {
                    StationInfoUpdate::InitialState { .. } => {
                        unreachable!("sent more than one InitialState update")
                    }
                    StationInfoUpdate::NewStation { id } => {
                        cache_stations
                            .insert_station(
                                id,
                                mycelium::station::identity::StationInfo {
                                    supports_channels: vec![],
                                },
                            )
                            .map_err(|_| {
                                anyhow!("NewStation used with station that already exists")
                            })?;
                        ipc_broadcast_queue.send(IPCMsg {
                            kind: mycelium::IPCMsgKind::NewStation { id },
                        })?;
                    }
                    StationInfoUpdate::NewChannel { id, ch } => {
                        cache_channels
                            .insert_channel_with_id(ch.clone(), id)
                            .map_err(|_| {
                                anyhow!("NewChannel was called with a channel that already exists")
                            })?;
                        ipc_broadcast_queue.send(IPCMsg {
                            kind: mycelium::IPCMsgKind::NewChannel { id, ch },
                        })?;
                    }
                    StationInfoUpdate::StationNewChannel { station, channel } => {
                        let _channel_info = cache_channels.get_channel(&channel)
                            .ok_or_else(|| anyhow!("StationNewChannel attempted to associate a station with a channel that does not exist!"))?;
                        cache_stations.map_info(&station, |_id, info| {
                            info.supports_channels.push(channel);
                        }).ok_or_else(|| anyhow!("StationNewChannel attempted to associate a channel with a station that does not exist"))?;
                        ipc_broadcast_queue.send(IPCMsg {
                            kind: mycelium::IPCMsgKind::StationNewChannel { station, channel },
                        })?;
                    }
                }
            }
        }
    };
    Ok(())
}
