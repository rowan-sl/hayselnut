#![allow(incomplete_features)]
// warning created by a macro in num_enum
#![allow(non_upper_case_globals)]
#![feature(generic_const_exprs)]
#![feature(specialization)]
#![feature(is_sorted)]

#[macro_use]
extern crate async_trait;
#[macro_use]
extern crate tracing;
#[macro_use]
extern crate anyhow;

use clap::{Args, Parser, Subcommand};
use mycelium::IPCMsg;
use squirrel::{
    api::{
        station::{
            capabilities::{ChannelID, ChannelName, KnownChannels},
            identity::{KnownStations, StationInfo},
        },
        ChannelMappings, PacketKind,
    },
    transport::server::{recv_next_packet, ClientInterface, ClientMetadata, DispatchEvent},
};
use std::{collections::HashMap, fmt::Write as _, net::SocketAddr, path::PathBuf, time::Duration};
use tokio::{
    fs,
    net::{UdpSocket, UnixListener},
    select,
    signal::ctrl_c,
    spawn, sync,
};
use tracing::metadata::LevelFilter;
use trust_dns_resolver::config as resolveconf;
use trust_dns_resolver::TokioAsyncResolver;

mod consumer;
mod paths;
mod registry;
pub mod route;
mod shutdown;
pub mod tsdb;
pub mod tsdb2;

use consumer::db::RecordDB;
use registry::JsonLoader;
use route::{Router, StationInfoUpdate};
use shutdown::Shutdown;
use tsdb2::{alloc::disk_store::DiskStore, Database};

#[derive(Parser, Debug)]
pub struct ArgsParser {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// dump info about the database
    Infodump {
        #[arg(
            long,
            help = "if provided, will dump information about the database contained in <file>"
        )]
        file: Option<PathBuf>,
    },
    /// run
    Run {
        #[command(flatten)]
        args: RunArgs,
    },
    /// test program for TSDB v2
    DB2 {
        #[arg(
            long,
            help = "allow initializing a database using a file that contains data (may cause silent deletion of corrupted databases, so it is recommended to only use this when running the server for the first time)"
        )]
        init_overwrite: bool,
        #[arg(long, short, help = "database file")]
        file: PathBuf,
    },
}

#[derive(Args, Debug)]
pub struct RunArgs {
    #[arg(
        short,
        long,
        help = "directory for station/channel ID records and the database to be placed"
    )]
    data_dir: PathBuf,
    #[arg(short, long, help = "path of the unix socket for the servers IPC API")]
    ipc_sock: PathBuf,
    #[arg(short, long, help = "url of the server that this is to be run on")]
    url: String,
    #[arg(short, long, help = "port to run the server on")]
    port: u16,
    #[arg(
        long,
        help = "allow initiailizing a database using a file that contains data (this may cause silent deletion of corrupt databases, so it is recommended to only use this when running the server for the first time)"
    )]
    init_overwrite: bool,
    #[arg(
        long,
        help = "allow using an aternate database file, instead of the default under `data_dir`. this allows for use of *special* files like block devices..."
    )]
    alt_db: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args_parsed = ArgsParser::parse();
    let args: RunArgs;

    tracing::subscriber::set_global_default(
        tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(
                tracing_subscriber::EnvFilter::builder()
                    .with_default_directive(LevelFilter::TRACE.into())
                    .from_env()
                    .expect("Invalid logging config"),
            )
            .pretty()
            .finish(),
    )
    .expect("Failed to set tracing subscriber");

    info!("Args: {args_parsed:#?}");

    match args_parsed.cmd {
        Cmd::Infodump { file } => {
            error!(" -------- dumping database info --------");
            tsdb2::Database::<tsdb2::alloc::disk_store::DiskStore>::infodump().await;
            if let Some(file) = file {
                error!(" -------- dumping database info for {file:?} --------");
                let store = DiskStore::new(&file, true).await?;
                let mut database = Database::new(store, false).await?;
                database.infodump_from().await?;
                database.close().await?;
            }
            error!(" -------- DB infodump complete  --------");
            return Ok(());
        }
        Cmd::DB2 {
            init_overwrite,
            mut file,
        } => {
            file = file.canonicalize()?;
            debug!("{{file}} resolves to {file:?}");
            let store = DiskStore::new(&file, false).await?;
            let database = Database::new(store, init_overwrite).await?;

            database.close().await?;
            return Ok(());
        }
        Cmd::Run { args: run_args } => args = run_args,
    }

    let mut shutdown = Shutdown::new();
    // trap the signal, will only start listening later in the main loop
    let mut ctrlc = {
        warn!("Trapping ctrl+c, it will be useless until initialization is finished");
        let (shutdown_tx, shutdown_rx) = sync::broadcast::channel::<()>(1);
        tokio::spawn(async move {
            if let Err(_) = ctrl_c().await {
                error!("Failed to listen for ctrl_c signal");
            }
            shutdown_tx.send(()).unwrap();
        });

        shutdown_rx
    };

    // a new scope is opened here so that any item using ShutdownHandles is dropped before
    // the waiting-for-shutdown-handles-to-be-dropped happens, to avoid a deadlock
    {
        if args.data_dir.exists() {
            if !args.data_dir.canonicalize()?.is_dir() {
                error!("records directory path already exists, and is a file!");
                bail!("records dir exists");
            }
        } else {
            info!("Creating new records directory at {:#?}", args.data_dir);
            fs::create_dir(args.data_dir.clone()).await?;
        }

        let records_dir = paths::RecordsPath::new(args.data_dir.canonicalize()?);

        info!("Loading info for known stations");
        let stations_path = records_dir.path("stations.json");
        let mut stations =
            JsonLoader::<KnownStations>::open(stations_path, shutdown.handle()).await?;
        debug!("Loaded known stations:");

        info!("Loading known channels");
        let channels_path = records_dir.path("channels.json");
        let mut channels =
            JsonLoader::<KnownChannels>::open(channels_path, shutdown.handle()).await?;

        debug!(
            "Loaded known channels: {:#?}",
            channels
                .channels()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<Vec<_>>()
        );

        for s in stations.stations() {
            // in the future, station info should be printed
            let info = stations.get_info(s).unwrap();
            debug!(
                "Known station {}\nsupports channels {:#?}",
                s, info.supports_channels
            );
        }

        debug!("Loading database");
        warn!("TSDB2 is currently very unstable, bolth in format and in reliablility - things *will* go badly");
        let db_router_client = {
            let raw_path = if let Some(p) = args.alt_db {
                p
            } else {
                records_dir.path("data.tsdb2")
            };
            let path = raw_path.canonicalize()?;
            debug!("database path ({raw_path:?}) resolves to {path:?}");
            let store = DiskStore::new(&path, false).await?;
            let database = Database::new(store, args.init_overwrite).await?;
            RecordDB::new(database).await?
        };
        info!("Database loaded");

        info!(
            "Performing DNS lookup of server's extranal IP (url={})",
            args.url
        );
        let resolver = TokioAsyncResolver::tokio(
            resolveconf::ResolverConfig::default(),
            resolveconf::ResolverOpts::default(),
        )?;
        let addrs = resolver
            .lookup_ip(args.url)
            .await?
            .into_iter()
            .map(|addr| {
                debug!("Resolved IP {addr}");
                SocketAddr::new(addr, args.port)
            })
            .collect::<Vec<_>>();

        let mut handle = shutdown.handle();

        let mut router = Router::new();
        router.with_consumer(db_router_client);
        // send the initial update with the current state
        router
            .update_station_info(&[StationInfoUpdate::InitialState {
                stations: stations.clone(),
                channels: channels.clone(),
            }])
            .await?;

        //TODO: integrate with the router system
        #[derive(Debug)]
        enum IPCCtrlMsg {
            Broadcast(IPCMsg),
            UpdateInfo {
                stations: KnownStations,
                channels: KnownChannels,
            },
        }
        let (ipc_task_tx, ipc_task_rx) = flume::unbounded::<IPCCtrlMsg>();
        ipc_task_tx
            .send_async(IPCCtrlMsg::UpdateInfo {
                stations: stations.clone(),
                channels: channels.clone(),
            })
            .await
            .unwrap();
        let main_task = spawn(async move {
            async move {
                // test code for the network protcol
                let sock = UdpSocket::bind(addrs.as_slice()).await?;
                let max_transaction_time = Duration::from_secs(30);
                let (dispatch, dispatch_rx) = flume::unbounded::<(SocketAddr, DispatchEvent)>();
                let mut clients = HashMap::<SocketAddr, ClientInterface>::new();

                loop {
                    select! {
                        // TODO: complete transactions and inform clients before exit
                        _ = handle.wait_for_shutdown() => { break; }
                        recv = dispatch_rx.recv_async() => {
                            let (ip, event) = recv.unwrap();
                            match event {
                                DispatchEvent::Send(packet) => {
                                    //debug!("Sending {packet:#?} to {ip:?}");
                                    sock.send_to(packet.as_bytes(), ip).await.unwrap();
                                }
                                DispatchEvent::TimedOut => {
                                    error!("Connection to {ip:?} timed out");
                                }
                                DispatchEvent::Received(recv_data) => {
                                    debug!("received [packet] from {ip:?}");
                                    // if let Ok(packet) = rmp_serde::from_slice::<rmpv::Value>(&data) {
                                    //     trace!("packet content (msgpack value): {packet:#?}");
                                    // }
                                    match rmp_serde::from_slice::<PacketKind>(&recv_data) {
                                        Ok(packet) => {
                                            trace!("packet content: {packet:?}");
                                            match packet {
                                                PacketKind::Connect(pkt_data) => {
                                                    let name_to_id_mappings = pkt_data.channels.iter()
                                                        .map(|ch| (
                                                            ch.name.clone(),
                                                            channels.id_by_name(&ch.name)
                                                                .map(|id| (id, false))
                                                                .unwrap_or_else(|| {
                                                                    info!("creating new channel: {ch:?}");
                                                                    (channels.insert_channel(ch.clone()).unwrap(), true)
                                                                })
                                                        ))
                                                        .collect::<HashMap<ChannelName, (ChannelID, bool)>>();
                                                    router.update_station_info(
                                                        name_to_id_mappings
                                                            .values()
                                                            .filter(|(_, is_new)| *is_new)
                                                            .map(|(ch_id, _)| {
                                                                let ch = channels.get_channel(&ch_id)
                                                                    .expect("unreachable");
                                                                StationInfoUpdate::NewChannel {
                                                                    id: *ch_id,
                                                                    ch: ch.clone(),
                                                                }
                                                            })
                                                            .collect::<Vec<_>>()
                                                            .as_slice()
                                                    ).await?;
                                                    let name_to_id_mappings = name_to_id_mappings
                                                        .into_iter()
                                                        .map(|(k, (v, _))| (k, v))
                                                        .collect::<HashMap<ChannelName, ChannelID>>();
                                                    if let Some(_) = stations.get_info(&pkt_data.station_id) {
                                                        info!(
                                                            "connecting to known station [{}] at IP {:?}\n    hayselnut rev {}\n    built on {}",
                                                            pkt_data.station_id,
                                                            ip,
                                                            pkt_data.station_build_rev,
                                                            pkt_data.station_build_date
                                                        );
                                                        stations.map_info(
                                                            &pkt_data.station_id,
                                                            |_id, info| info.supports_channels = name_to_id_mappings
                                                                .values()
                                                                .copied()
                                                                .collect()
                                                        );
                                                    } else {
                                                        info!(
                                                            "connected to new station [{}] at IP {:?}\n    hayselnut rev {}\n    built on {}",
                                                            pkt_data.station_id,
                                                            ip,
                                                            pkt_data.station_build_rev,
                                                            pkt_data.station_build_date
                                                        );
                                                        stations.insert_station(pkt_data.station_id, StationInfo {
                                                            supports_channels: name_to_id_mappings.values().copied().collect(),
                                                        }).unwrap();
                                                    }
                                                    let resp = rmp_serde::to_vec_named(&PacketKind::ChannelMappings(ChannelMappings {
                                                        map: name_to_id_mappings,
                                                    })).unwrap();
                                                    let client = clients.get_mut(&ip).unwrap();
                                                    client.queue(resp);
                                                    client.access_metadata().uuid = Some(pkt_data.station_id);
                                                    // update info with new station (may not allways be different)
                                                    ipc_task_tx
                                                        .send_async(IPCCtrlMsg::UpdateInfo {
                                                            stations: stations.clone(),
                                                            channels: channels.clone(),
                                                        })
                                                        .await
                                                        .unwrap();
                                                }
                                                PacketKind::Data(pkt_data) => {
                                                    let mut buf = String::new();
                                                    for (chid, dat) in pkt_data.per_channel.clone() {
                                                        if let Some(ch) = channels.get_channel(&chid) {
                                                            //TODO: verify that types match
                                                            let _ = writeln!(
                                                                buf, "Channel {chid} ({}) => {:?}",
                                                                <ChannelName as Into<String>>::into(ch.name.clone()), dat
                                                            );
                                                        } else {
                                                            warn!("Data contains channel id {chid} (={dat:?}) which is not known to this server");
                                                            let _ = writeln!(buf, "Chanenl {chid} (<unknown>) => {:?}", dat);
                                                        }
                                                    }
                                                    info!("Received data:\n{buf}");
                                                    ipc_task_tx.send_async(IPCCtrlMsg::Broadcast(IPCMsg { kind: mycelium::IPCMsgKind::FreshHotData {
                                                        from: clients.get_mut(&ip).unwrap().access_metadata().uuid.unwrap(),
                                                        by_channel: pkt_data.per_channel,
                                                    }})).await?;
                                                }
                                                _ => warn!("received unexpected packet, ignoring"),
                                            }
                                        }
                                        Err(e) => {
                                            warn!("packet was malformed (failed to deserialize)\nerror: {e:?}");
                                        }
                                    }
                                }
                            }
                        }
                        packet = recv_next_packet(&sock) => {
                            if let Some((from, packet)) = packet? {
                                //debug!("Received {packet:#?} from {from:?}");
                                let cl = clients.entry(from)
                                    .or_insert_with(|| ClientInterface::new(max_transaction_time, from, dispatch.clone(), ClientMetadata::default()));
                                cl.handle(packet);
                            }
                        }
                    };
                }

                Ok::<(), anyhow::Error>(())
            }.await.unwrap()
        });

        let mut handle = shutdown.handle();
        let ipc_task = spawn(async move {
            let mut shutdown_ipc = Shutdown::new();
            let listener = UnixListener::bind(args.ipc_sock.clone()).unwrap();
            let (ipc_broadcast_queue, _) = sync::broadcast::channel::<IPCMsg>(10);
            let (mut cache_stations, mut cache_channels) =
                (KnownStations::new(), KnownChannels::new());

            let res = async {
                loop {
                    select! {
                        _ = handle.wait_for_shutdown() => { break; }
                        recv = ipc_task_rx.recv_async() => {
                            match recv? {
                                IPCCtrlMsg::Broadcast(msg) => {
                                    let num = ipc_broadcast_queue.send(msg).unwrap_or(0);
                                    trace!("Sent IPC message to {num} IPC clients");
                                }
                                IPCCtrlMsg::UpdateInfo { stations, channels } => {
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
            let _ = tokio::fs::remove_file(args.ipc_sock).await;
            shutdown_ipc.trigger_shutdown();
            shutdown_ipc.wait_for_completion().await;
            res.unwrap();
        });

        // let mut router = Router::new();
        // router.with_consumer(RecordDB::new(&records_dir.path("data.tsdb")).await?);
        //
        // // ... code ...
        //
        // router.close().await;

        info!("running -- press ctrl+c to exit");
        select! { _ = ctrlc.recv() => {} _ = main_task => {} _ = ipc_task => {} }
        shutdown.trigger_shutdown();
    }
    shutdown.wait_for_completion().await;

    Ok(())
}
