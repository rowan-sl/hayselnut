#[macro_use]
extern crate async_trait;
#[macro_use]
extern crate tracing;
#[macro_use]
extern crate anyhow;

use clap::Parser;
use squirrel::{
    api::{
        station::{
            capabilities::{ChannelData, ChannelID, ChannelName, KnownChannels},
            identity::{KnownStations, StationInfo},
        },
        ChannelMappings, PacketKind,
    },
    transport::server::{recv_next_packet, ClientInterface, DispatchEvent},
};
use std::{collections::HashMap, fmt::Write as _, net::SocketAddr, path::PathBuf, time::Duration};
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
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
mod route;
mod shutdown;
pub mod tsdb;

use registry::JsonLoader;
use shutdown::Shutdown;

#[derive(Parser, Debug)]
pub struct Args {
    // #[arg(short, long, help = "IP address of the weather station to connect to")]
    // addr: SocketAddr,
    // #[arg(short, long, help = "Delay between readings from station (in seconds)")]
    // delay: f64,
    // #[arg(
    //     short,
    //     long,
    //     help = "Path for unix socket to communicate with the web server on"
    // )]
    // socket: PathBuf,
    #[arg(
        short,
        long,
        help = "directory for weather + station ID records to be placed"
    )]
    records_path: PathBuf,
    #[arg(short, long, help = "path of the unix socket for the servers IPC API")]
    ipc_sock: PathBuf,
    #[arg(short, long, help = "url of the server that this is to be run on")]
    url: String,
    #[arg(short, long, help = "port to run the server on")]
    port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

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

    info!("Args: {args:#?}");

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
        if args.records_path.exists() {
            if !args.records_path.canonicalize()?.is_dir() {
                error!("records directory path already exists, and is a file!");
                bail!("records dir exists");
            }
        } else {
            info!("Creating new records directory at {:#?}", args.records_path);
            fs::create_dir(args.records_path.clone()).await?;
        }

        let records_dir = paths::RecordsPath::new(args.records_path.canonicalize()?);

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
        let temp_log_path = records_dir.path("data.txt");
        //TODO: integrate IPC with Router system
        #[derive(Debug)]
        enum IPCCtrlMsg {
            Readings(mycelium::LatestReadings),
        }
        let (ipc_task_tx, ipc_task_rx) = flume::unbounded::<IPCCtrlMsg>();
        let main_task = spawn(async move {
            async move {
            // file stuff
                let mut temp_log = OpenOptions::new()
                    .write(true)
                    .append(true)
                    .create(true)
                    .open(temp_log_path)
                    .await?;
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
                            DispatchEvent::Received(data) => {
                                debug!("received [packet] from {ip:?}");
                                // if let Ok(packet) = rmp_serde::from_slice::<rmpv::Value>(&data) {
                                //     trace!("packet content (msgpack value): {packet:#?}");
                                // }
                                match rmp_serde::from_slice::<PacketKind>(&data) {
                                    Ok(packet) => {
                                        trace!("packet content: {packet:?}");
                                        match packet {
                                            PacketKind::Connect(data) => {
                                                let name_to_id_mappings = data.channels.iter()
                                                    .map(|ch| {
                                                        (
                                                            ch.name.clone(),
                                                            channels.id_by_name(&ch.name)
                                                                .unwrap_or_else(|| {
                                                                    info!("creating new channel: {ch:?}");
                                                                    channels.insert_channel(ch.clone()).unwrap()
                                                                })
                                                        )
                                                    })
                                                    .collect::<HashMap<ChannelName, ChannelID>>();
                                                if let Some(_) = stations.get_info(&data.station_id) {
                                                    info!(
                                                        "connecting to known station [{}] at IP {:?}\n    hayselnut rev {}\n    built on {}",
                                                        data.station_id,
                                                        ip,
                                                        data.station_build_rev,
                                                        data.station_build_date
                                                    );
                                                    stations.map_info(
                                                        &data.station_id,
                                                        |_id, info| info.supports_channels = name_to_id_mappings
                                                            .values()
                                                            .copied()
                                                            .collect()
                                                    );
                                                } else {
                                                    info!(
                                                        "connected to new station [{}] at IP {:?}\n    hayselnut rev {}\n    built on {}",
                                                        data.station_id,
                                                        ip,
                                                        data.station_build_rev,
                                                        data.station_build_date
                                                    );
                                                    stations.insert_station(data.station_id, StationInfo {
                                                        supports_channels: name_to_id_mappings.values().copied().collect(),
                                                    }).unwrap();
                                                }
                                                let resp = rmp_serde::to_vec_named(&PacketKind::ChannelMappings(ChannelMappings {
                                                    map: name_to_id_mappings,
                                                })).unwrap();
                                                clients.get_mut(&ip).unwrap().queue(resp);
                                            }
                                            PacketKind::Data(data) => {
                                                let mut buf = String::new();
                                                for (chid, dat) in data.per_channel.clone() {
                                                    if let Some(ch) = channels.get_channel(&chid) {
                                                        //TODO: verify that types match
                                                        let _ = writeln!(
                                                            buf, "Channel {chid} ({}) => {:?}",
                                                            <ChannelName as Into<String>>::into(ch.name.clone()), dat
                                                        );
                                                        if let "battery" | "temperature" | "humidity" | "pressure" = ch.name.as_ref().as_str() {
                                                            temp_log.write_all(format!(
                                                                "[{}]:{}={}",
                                                                chrono::Local::now().to_rfc3339(),
                                                                ch.name.as_ref(),
                                                                match dat {
                                                                    ChannelData::Float(f) => f.to_string(),
                                                                    ChannelData::Event {..} => "null".to_string(),
                                                                }
                                                            ).as_bytes()).await?;
                                                        }
                                                    } else {
                                                        warn!("Data contains channel id {chid} which is not known to this server");
                                                        let _ = writeln!(buf, "Chanenl {chid} (<unknown>) => {:?}", dat);
                                                    }
                                                }
                                                info!("Received data:\n{buf}");
                                                let chv = |name: &str| {
                                                    let ChannelData::Float(f) = data.per_channel.get(
                                                        &channels.id_by_name(
                                                            &name.to_string().into()
                                                        ).unwrap()
                                                    ).unwrap() else {
                                                        panic!()
                                                    };
                                                    *f
                                                };
                                                ipc_task_tx.send_async(IPCCtrlMsg::Readings(mycelium::LatestReadings {
                                                    temperature: chv("temperature"),
                                                    humidity: chv("humidity"),
                                                    pressure: chv("pressure"),
                                                    battery: chv("battery"),
                                                })).await?;
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
                                .or_insert_with(|| ClientInterface::new(max_transaction_time, from, dispatch.clone()));
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
            let (latest_readings_queue, _) = sync::broadcast::channel(10);

            let res = async {
                loop {
                    select! {
                        _ = handle.wait_for_shutdown() => { break; }
                        recv = ipc_task_rx.recv_async() => {
                            match recv? {
                                IPCCtrlMsg::Readings(r) => {
                                    let num = latest_readings_queue.send(r).unwrap_or(0);
                                    trace!("Sent IPC message to {num} tasks");
                                }
                            }
                        }
                        res = listener.accept() => {
                            let (mut sock, addr) = res?;
                            let mut recv = latest_readings_queue.subscribe();
                            let mut handle = shutdown_ipc.handle();
                            debug!("Connecting to new IPC client at {addr:?}");
                            spawn(async move {
                                let res = async move {
                                    loop {
                                        select! {
                                            // TODO: notify clients of server closure
                                            _ = handle.wait_for_shutdown() => { break; }
                                            res = recv.recv() => {
                                                match mycelium::ipc_send(&mut sock, &res?).await {
                                                    Ok(()) => {}
                                                    // Err(mycelium::IPCError::IO(e)) if e.kind() == io::ErrorKind::=> {
                                                    //     debug!("IPC Client {addr:?} disconnected");
                                                    //     break;
                                                    // }
                                                    Err(e) => Err(e)?
                                                }
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
