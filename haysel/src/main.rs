#![allow(incomplete_features)]
// num_enum!!!! (again)
#![allow(non_upper_case_globals)]
#![feature(trivial_bounds)]
#![feature(generic_const_exprs)]
#![feature(specialization)]
#![feature(is_sorted)]
#![feature(trait_upcasting)]

#[macro_use]
extern crate async_trait;
#[macro_use]
extern crate tracing;
#[macro_use]
extern crate anyhow;

use std::{collections::HashMap, fmt::Write as _, net::SocketAddr, process, time::Duration};

use anyhow::Result;
use clap::Parser;
use nix::{
    sys::signal::{kill, Signal},
    unistd::{daemon, Pid},
};
use paths::RecordsPath;
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
use tokio::{net::UdpSocket, runtime, select};
use trust_dns_resolver::config as resolveconf;
use trust_dns_resolver::TokioAsyncResolver;

mod args;
mod commands;
mod config;
mod consumer;
mod ipc;
mod log;
mod paths;
mod registry;
mod route;
mod shutdown;
// pub mod tsdb;
pub mod tsdb2;
mod util;

use args::{ArgsParser, RunArgs};
use consumer::{db::RecordDB, ipc::IPCConsumer};
use registry::JsonLoader;
use route::{Router, StationInfoUpdate};
use shutdown::Shutdown;
use tsdb2::{
    alloc::store::{disk::DiskStore, raid::ArrayR0 as RaidArray},
    Database,
};

use crate::tsdb2::alloc::store::{
    disk::DiskMode,
    raid::{self, DynStorage, IsDynStorage},
};

fn main() -> anyhow::Result<()> {
    let args = ArgsParser::parse();
    log::init_logging()?;

    let run_args;
    match args {
        ArgsParser {
            cmd: args::Cmd::Run { args },
        } => run_args = args,
        ArgsParser {
            cmd: args::Cmd::Kill { config },
        } => {
            info!("Reading configuration from {:?}", config);
            if !config.exists() {
                error!("Configuration file does not exist!");
                bail!("Configuration file does not exist!");
            }

            let cfg = {
                let buf = std::fs::read_to_string(&config)?;
                self::config::from_str(&buf)?
            };

            let run_dir = paths::RecordsPath::new(cfg.directory.run.clone());
            run_dir.ensure_exists_blocking()?;
            let pid_file = run_dir.path("daemon.lock");
            if !pid_file.try_exists()? {
                warn!("No known haysel daemon is currently running, exiting with no-op");
                return Ok(());
            }
            let pid_txt = std::fs::read_to_string(pid_file)?;
            let pid = pid_txt
                .parse::<u32>()
                .map_err(|e| anyhow!("failed to parse PID: {e:?}"))?;
            info!("Killing process {pid} - sending SIGINT (ctrl+c) to allow for gracefull shutdown\nThe PID file will only be removed when the server has exited");
            kill(Pid::from_raw(pid.try_into()?), Some(Signal::SIGINT))?;
            return Ok(());
        }
        other => {
            let runtime = runtime::Builder::new_multi_thread().enable_all().build()?;
            return runtime.block_on(commands::delegate(other));
        }
    };

    info!("Reading configuration from {:?}", run_args.config);
    if !run_args.config.exists() {
        error!("Configuration file does not exist!");
        bail!("Configuration file does not exist!");
    }

    let cfg = {
        let buf = std::fs::read_to_string(&run_args.config)?;
        self::config::from_str(&buf)?
    };

    let records_dir = paths::RecordsPath::new(cfg.directory.data.clone());
    records_dir.ensure_exists_blocking()?;
    let run_dir = paths::RecordsPath::new(cfg.directory.run.clone());
    run_dir.ensure_exists_blocking()?;

    let pid_file = run_dir.path("daemon.lock");
    if pid_file.try_exists()? {
        error!("A server is already running, refusing to start!");
        info!("If this is incorrect, remove the `daemon.lock` file and try again");
        bail!("Server already started");
    }

    if run_args.daemonize {
        debug!("Forking!");
        daemon(true, true)?;
        info!("[daemon] - copying logs ")
    }
    debug!("Writing PID file {:?}", pid_file);
    let pid = process::id();
    std::fs::write(&pid_file, format!("{pid}").as_bytes())?;

    debug!("Launching async runtime");
    let result = std::panic::catch_unwind(move || {
        let runtime = runtime::Builder::new_multi_thread().enable_all().build()?;
        let mut shutdown = Shutdown::new();
        runtime.block_on(async {
            let result = async_main(cfg, run_args, &mut shutdown, records_dir, run_dir).await;
            if let Err(e) = &result {
                error!("Main task exited with error: {e:?}");
            }
            shutdown.trigger_shutdown();
            info!("shut down - waiting for tasks to stop");
            shutdown.wait_for_completion().await;
            result
        })
    });
    debug!("Deleting PID file {:?}", pid_file);
    std::fs::remove_file(&pid_file)?;
    match result {
        Ok(inner) => inner,
        Err(err) => {
            error!("Main thread panic! - stuff is likely messed up: {err:?}");
            bail!("Main thread panic!");
        }
    }
}

async fn async_main(
    cfg: self::config::Config,
    args: RunArgs,
    shutdown: &mut Shutdown,
    records_dir: RecordsPath,
    run_dir: RecordsPath,
) -> anyhow::Result<()> {
    // trap the ctrl+csignal, will only start listening later in the main loop
    shutdown::util::trap_ctrl_c(shutdown.handle()).await;

    let addrs = lookup_server_ip(cfg.server.url, cfg.server.port).await?;

    info!("Loading info for known stations");
    let mut stations =
        JsonLoader::<KnownStations>::open(records_dir.path("stations.json"), shutdown.handle())
            .await?;
    debug!("Loaded known stations:");

    info!("Loading known channels");
    let mut channels =
        JsonLoader::<KnownChannels>::open(records_dir.path("channels.json"), shutdown.handle())
            .await?;

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
    warn!("TSDB V2 is currently very unstable, bolth in format and in reliablility - things *will* go badly");
    let db_router_client = {
        let store: Box<(dyn IsDynStorage<Error = raid::DynStorageError> + 'static)> = match cfg
            .database
            .storage
        {
            config::StorageMode::default => {
                let path = records_dir.path("data.tsdb2");
                Box::new(DynStorage(
                    DiskStore::new(&path, false, DiskMode::Dynamic).await?,
                ))
            }
            config::StorageMode::file => {
                if cfg.database.files.len() != 1 {
                    if cfg.database.files.is_empty() {
                        error!("Failed to create database storage: file mode is requested, but no file is provided");
                    } else {
                        error!("Failed to create database storage: file mode is requested, but more than one file is proveded (did you mean to enable RAID?)");
                    }
                    bail!("Failed to create database store");
                }
                let config::File { path, blockdevice } = cfg.database.files[0].clone();
                Box::new(DynStorage(
                    DiskStore::new(
                        &path,
                        false,
                        if blockdevice {
                            DiskMode::BlockDevice
                        } else {
                            DiskMode::Dynamic
                        },
                    )
                    .await?,
                ))
            }
            config::StorageMode::raid => {
                if cfg.database.files.is_empty() {
                    error!("Failed to create database storage: RAID mode is requested, but no file(s) are provided");
                    bail!("Failed to create database store");
                }
                if cfg.database.files.len() == 1 {
                    warn!("RAID mode is requested, but only one backing file is specified. this will cause unnecessary overhead, and it is recommended to switch to using single file mode");
                }
                let mut array = RaidArray::new();
                for config::File { path, blockdevice } in cfg.database.files {
                    let store = DiskStore::new(
                        &path,
                        false,
                        if blockdevice {
                            DiskMode::BlockDevice
                        } else {
                            DiskMode::Dynamic
                        },
                    )
                    .await?;
                    array.add_element(store).await?;
                }
                if args.overwrite_reinit {
                    warn!("Deleting and Re-Initializing the RAID storage");
                    array.wipe_all_your_data_away().await?;
                }
                debug!("Building array...");
                array.build().await?;
                array.print_info().await?;
                Box::new(DynStorage(array))
            }
        };
        let database = Database::new(store, args.overwrite_reinit).await?;
        RecordDB::new(database, shutdown.handle()).await?
    };
    info!("Database loaded");

    let ipc_path = run_dir.path("ipc.sock");
    debug!("Setting up IPC at {:?}", ipc_path);
    if tokio::fs::try_exists(&ipc_path).await? {
        tokio::fs::remove_file(&ipc_path).await?;
    }
    let ipc_router_client = IPCConsumer::new(ipc_path, shutdown.handle()).await?;
    info!("IPC configured");

    let mut router = Router::new();
    router.with_consumer(db_router_client);
    router.with_consumer(ipc_router_client);
    // send the initial update with the current state
    router
        .update_station_info(&[StationInfoUpdate::InitialState {
            stations: stations.clone(),
            channels: channels.clone(),
        }])
        .await?;

    info!("running -- press ctrl+c to exit");
    let sock = UdpSocket::bind(addrs.as_slice()).await?;
    let max_transaction_time = Duration::from_secs(30);
    let (dispatch, dispatch_rx) = flume::unbounded::<(SocketAddr, DispatchEvent)>();
    let mut clients = HashMap::<SocketAddr, ClientInterface>::new();
    let mut handle = shutdown.handle();

    loop {
        select! {
            // TODO: complete transactions and inform clients before exit
            _ = handle.wait_for_shutdown() => {
                // shutdown of router clients is handled after the loop exists
                drop(handle);
                break;
            }
            recv = dispatch_rx.recv_async() => {
                let (ip, event) = recv.unwrap();
                match event {
                    DispatchEvent::Send(packet) => {
                        //debug!("Sending {packet:#?} to {ip:?}");
                        sock.send_to(packet.as_bytes(), ip).await?;
                    }
                    DispatchEvent::TimedOut => {
                        error!("Connection to {ip:?} timed out");
                    }
                    DispatchEvent::Received(recv_data) => {
                        debug!("received [packet] from {ip:?}");
                        match rmp_serde::from_slice::<PacketKind>(&recv_data) {
                            Ok(packet) => {
                                handle_packet(
                                    packet,
                                    &mut stations,
                                    &mut channels,
                                    &mut router,
                                    ip,
                                    &mut clients
                                ).await?
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

    trace!("Shutting down - if a deadlock occurs here, it is likely because a shutdown handle was created in the main function and not dropped before this call");
    shutdown.wait_for_completion().await;

    Ok(())
}

/// it is necessary to bind the server to the real external ip address,
/// or risk confusing issues (forgot what, but it's bad)
async fn lookup_server_ip(url: String, port: u16) -> Result<Vec<SocketAddr>> {
    info!(
        "Performing DNS lookup of server's extranal IP (url={})",
        url
    );
    let resolver = TokioAsyncResolver::tokio(
        resolveconf::ResolverConfig::default(),
        resolveconf::ResolverOpts::default(),
    );
    let addrs = resolver
        .lookup_ip(url)
        .await?
        .into_iter()
        .map(|addr| {
            debug!("Resolved IP {addr}");
            SocketAddr::new(addr, port)
        })
        .collect::<Vec<_>>();
    Ok::<_, anyhow::Error>(addrs)
}

async fn handle_packet(
    packet: PacketKind,
    stations: &mut JsonLoader<KnownStations>,
    channels: &mut JsonLoader<KnownChannels>,
    router: &mut Router,
    ip: SocketAddr,
    clients: &mut HashMap<SocketAddr, ClientInterface>,
) -> Result<()> {
    trace!("packet content: {packet:?}");
    match packet {
        PacketKind::Connect(pkt_data) => {
            let name_to_id_mappings = pkt_data
                .channels
                .iter()
                .map(|ch| {
                    (
                        ch.name.clone(),
                        channels
                            .id_by_name(&ch.name)
                            .map(|id| (id, false))
                            .unwrap_or_else(|| {
                                info!("creating new channel: {ch:?}");
                                (channels.insert_channel(ch.clone()).unwrap(), true)
                            }),
                    )
                })
                .collect::<HashMap<ChannelName, (ChannelID, bool)>>();
            router
                .update_station_info(
                    name_to_id_mappings
                        .values()
                        .filter(|(_, is_new)| *is_new)
                        .map(|(ch_id, _)| {
                            let ch = channels.get_channel(&ch_id).expect("unreachable");
                            StationInfoUpdate::NewChannel {
                                id: *ch_id,
                                ch: ch.clone(),
                            }
                        })
                        .collect::<Vec<_>>()
                        .as_slice(),
                )
                .await?;
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
                stations.map_info(&pkt_data.station_id, |_id, info| {
                    info.supports_channels = name_to_id_mappings.values().copied().collect()
                });
            } else {
                info!(
                    "connected to new station [{}] at IP {:?}\n    hayselnut rev {}\n    built on {}",
                    pkt_data.station_id,
                    ip,
                    pkt_data.station_build_rev,
                    pkt_data.station_build_date
                );
                stations
                    .insert_station(
                        pkt_data.station_id,
                        StationInfo {
                            supports_channels: name_to_id_mappings.values().copied().collect(),
                        },
                    )
                    .unwrap();
                let mut updates = name_to_id_mappings
                    .values()
                    .copied()
                    .map(|id| StationInfoUpdate::StationNewChannel {
                        station: pkt_data.station_id,
                        channel: id,
                    })
                    .collect::<Vec<_>>();
                updates.push(StationInfoUpdate::NewStation {
                    id: pkt_data.station_id,
                });
                router.update_station_info(&updates).await?;
            }
            let resp = rmp_serde::to_vec_named(&PacketKind::ChannelMappings(ChannelMappings {
                map: name_to_id_mappings,
            }))
            .unwrap();
            let client = clients.get_mut(&ip).unwrap();
            client.queue(resp);
            client.access_metadata().uuid = Some(pkt_data.station_id);
        }
        PacketKind::Data(pkt_data) => {
            let received_at = chrono::Utc::now();
            let mut buf = String::new();
            for (chid, dat) in pkt_data.per_channel.clone() {
                if let Some(ch) = channels.get_channel(&chid) {
                    //TODO: verify that types match
                    let _ = writeln!(
                        buf,
                        "Channel {chid} ({}) => {:?}",
                        <ChannelName as Into<String>>::into(ch.name.clone()),
                        dat
                    );
                } else {
                    warn!("Data contains channel id {chid} (={dat:?}) which is not known to this server");
                    let _ = writeln!(buf, "Chanenl {chid} (<unknown>) => {:?}", dat);
                }
            }
            info!("Received data:\n{buf}");
            router
                .process(consumer::Record {
                    recorded_at: received_at,
                    recorded_by: clients
                        .get_mut(&ip)
                        .unwrap()
                        .access_metadata()
                        .uuid
                        .unwrap(),
                    data: pkt_data.per_channel,
                })
                .await?;
        }
        _ => warn!("received unexpected packet, ignoring"),
    }
    Ok(())
}
