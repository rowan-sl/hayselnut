#![allow(incomplete_features)]
#![feature(generic_const_exprs)]
#![feature(specialization)]
#![feature(is_sorted)]

#[macro_use]
extern crate async_trait;
#[macro_use]
extern crate tracing;
#[macro_use]
extern crate anyhow;

use anyhow::Result;
use clap::Parser;
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
use std::{collections::HashMap, fmt::Write as _, net::SocketAddr, time::Duration};
use tokio::{net::UdpSocket, runtime, select};
use trust_dns_resolver::config as resolveconf;
use trust_dns_resolver::TokioAsyncResolver;

mod args;
mod commands;
mod consumer;
mod ipc;
mod log;
pub mod paths;
pub mod registry;
pub mod route;
pub mod shutdown;
pub mod tsdb;
pub mod tsdb2;
pub mod util;

use args::ArgsParser;
use consumer::{db::RecordDB, ipc::IPCConsumer};
use registry::JsonLoader;
use route::{Router, StationInfoUpdate};
use shutdown::Shutdown;
use tsdb2::{alloc::disk_store::DiskStore, Database};

fn main() -> anyhow::Result<()> {
    let runtime = runtime::Builder::new_multi_thread().enable_all().build()?;
    let mut shutdown = Shutdown::new();
    runtime.block_on(async_main(&mut shutdown))?;
    shutdown.trigger_shutdown();
    runtime.shutdown_timeout(Duration::from_secs(60 * 5));
    Ok(())
}

async fn async_main(shutdown: &mut Shutdown) -> anyhow::Result<()> {
    log::init_logging()?;

    let args = match commands::delegate(ArgsParser::parse()).await {
        commands::Delegation::SubcommandRan(result) => return result,
        commands::Delegation::RunMain(args) => args,
    };

    // trap the ctrl+csignal, will only start listening later in the main loop
    shutdown::util::trap_ctrl_c(shutdown.handle()).await;

    let addrs = lookup_server_ip(args.url, args.port).await?;

    let records_dir = paths::RecordsPath::new(args.data_dir.canonicalize()?);
    records_dir.ensure_exists().await?;

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
        let raw_path = if let Some(p) = args.alt_db {
            p
        } else {
            records_dir.path("data.tsdb2")
        };
        let path = raw_path.canonicalize()?;
        debug!("database path ({raw_path:?}) resolves to {path:?}");
        let store = DiskStore::new(&path, false).await?;
        let database = Database::new(store, args.init_overwrite).await?;
        RecordDB::new(database, shutdown.handle()).await?
    };
    info!("Database loaded");

    debug!("Setting up IPC at {:?}", args.ipc_sock);
    let ipc_router_client = IPCConsumer::new(args.ipc_sock, shutdown.handle()).await?;
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
    )?;
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
