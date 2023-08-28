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
use tokio::{fs, net::UdpSocket, select};
use trust_dns_resolver::config as resolveconf;
use trust_dns_resolver::TokioAsyncResolver;

mod args;
mod commands;
mod consumer;
mod ipc;
mod log;
mod paths;
mod registry;
pub mod route;
mod shutdown;
pub mod tsdb;
pub mod tsdb2;

use args::ArgsParser;
use consumer::{db::RecordDB, ipc::IPCConsumer};
use registry::JsonLoader;
use route::{Router, StationInfoUpdate};
use shutdown::{Shutdown, ShutdownHandle};
use tsdb2::{alloc::disk_store::DiskStore, Database};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    log::init_logging()?;

    let args = match commands::delegate(ArgsParser::parse()).await {
        commands::Delegation::SubcommandRan(result) => return result,
        commands::Delegation::RunMain(args) => args,
    };

    let shutdown = Shutdown::new();
    // trap the ctrl+csignal, will only start listening later in the main loop
    shutdown::util::trap_ctrl_c(shutdown.handle()).await;

    // a new scope is opened here so that any item using ShutdownHandles is dropped before
    // the waiting-for-shutdown-handles-to-be-dropped happens, to avoid a deadlock
    {
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
        let stations = JsonLoader::<KnownStations>::open(stations_path, shutdown.handle()).await?;
        debug!("Loaded known stations:");

        info!("Loading known channels");
        let channels_path = records_dir.path("channels.json");
        let channels = JsonLoader::<KnownChannels>::open(channels_path, shutdown.handle()).await?;

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

        debug!("Setting up IPC at {:?}", args.ipc_sock);
        let ipc_router_client = IPCConsumer::new(args.ipc_sock).await?;
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
        main_task(router, addrs, shutdown.handle(), stations, channels).await?;
    }
    shutdown.wait_for_completion().await;

    Ok(())
}

async fn main_task(
    mut router: Router,
    addrs: Vec<SocketAddr>,
    mut handle: ShutdownHandle,
    mut stations: JsonLoader<KnownStations>,
    mut channels: JsonLoader<KnownChannels>,
) -> Result<()> {
    let res = async {
        // test code for the network protcol
        let sock = UdpSocket::bind(addrs.as_slice()).await?;
        let max_transaction_time = Duration::from_secs(30);
        let (dispatch, dispatch_rx) = flume::unbounded::<(SocketAddr, DispatchEvent)>();
        let mut clients = HashMap::<SocketAddr, ClientInterface>::new();

        loop {
            select! {
                // TODO: complete transactions and inform clients before exit
                _ = handle.wait_for_shutdown() => {
                    // shutdown of router clients is handled after the loop exists
                    break;
                }
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
        Ok::<(), anyhow::Error>(())
    }.await;
    // takes place here so that shutdown occurs
    // regardless of if an error happened or not
    router.close().await;
    handle.trigger_shutdown();
    res
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
