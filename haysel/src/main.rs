#[macro_use]
extern crate async_trait;
#[macro_use]
extern crate tracing;
#[macro_use]
extern crate anyhow;

use clap::Parser;
use squirrel::api::{
    station::{
        capabilities::{ChannelID, ChannelName, KnownChannels},
        identity::{KnownStations, StationInfo},
    },
    ChannelMappings, PacketKind,
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    time::Duration,
};
use tokio::{fs, net::UdpSocket, select, signal::ctrl_c, spawn, sync};
use tracing::metadata::LevelFilter;
use trust_dns_resolver::TokioAsyncResolver;
use trust_dns_resolver::config as resolveconf;

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

        info!("Performing DNS lookup of server's extranal IP (url={})", args.url);
        let resolver = TokioAsyncResolver::tokio(
            resolveconf::ResolverConfig::default(),
            resolveconf::ResolverOpts::default()
        )?;
        let addrs = resolver.lookup_ip(args.url)
            .await?
            .into_iter()
            .map(|addr| {
                debug!("Resolved IP {addr}");
                SocketAddr::new(addr, args.port)
            })
            .collect::<Vec<_>>();

        let mut handle = shutdown.handle();
        let main_task = spawn(async move {
            async move {
            use squirrel::transport::server::{DispatchEvent, ClientInterface, recv_next_packet};
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
                                if let Ok(packet) = rmp_serde::from_slice::<PacketKind>(&data) {
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
                                                info!("connecting to known station [{}] at IP {:?}", data.station_id, ip);
                                                stations.map_info(&data.station_id, |_id, info| info.supports_channels = name_to_id_mappings.values().copied().collect());
                                            } else {
                                                info!("connected to new station [{}] at IP {:?}", data.station_id, ip);
                                                let id = stations.gen_id();
                                                stations.insert_station(id, StationInfo {
                                                    supports_channels: name_to_id_mappings.values().copied().collect(),
                                                }).unwrap();
                                            }
                                            let resp = rmp_serde::to_vec_named(&PacketKind::ChannelMappings(ChannelMappings {
                                                map: name_to_id_mappings,
                                            })).unwrap();
                                            clients.get_mut(&ip).unwrap().queue(resp);
                                        }
                                        _ => debug!("received unexpected packet, ignoring"),
                                    }
                                } else {
                                    debug!("packet was malformed (failed to deserialize)");
                                }
                            }
                        }
                    }
                    packet = recv_next_packet(&sock) => {
                        if let Some((from, packet)) = packet? {
                            debug!("Received {packet:#?} from {from:?}");
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

        // let mut router = Router::new();
        // router.with_consumer(RecordDB::new(&records_dir.path("data.tsdb")).await?);
        //
        // // ... code ...
        //
        // router.close().await;

        info!("running -- press ctrl+c to exit");
        select! { _ = ctrlc.recv() => {} _ = main_task => {} }
        shutdown.trigger_shutdown();
    }
    shutdown.wait_for_completion().await;

    Ok(())

    // #[derive(Debug, Clone, Copy, Serialize, FromBytes, AsBytes)]
    // #[repr(C)]
    // struct TestData {
    //     num1: u32,
    // }
    //
    // let mut db = DB::<TestData>::open(&"test.tsdb".parse::<PathBuf>().unwrap()).await?;
    // // info!("attempting insert");
    // // db.insert(Local::now(), TestData { num1: 100 }).await?;
    // // db.insert(Local::now() - chrono::Duration::days(1), TestData { num1: 50 }).await?;
    // // info!("db.insert ran successfully!");
    // // info!("DB structure debug:\n{}", serde_json::to_string_pretty(&db.debug_structure().await?)?);
    // let records = db.query(NaiveDateTime::new(
    //     Local::now().naive_local().date(),
    //     NaiveTime::from_hms_opt(0, 0, 0).unwrap()
    // ).and_local_timezone(Local).unwrap(),
    // NaiveDateTime::new(
    //     Local::now().naive_local().date(),
    //     NaiveTime::from_hms_opt(23, 59, 59).unwrap()
    // ).and_local_timezone(Local).unwrap()).await?;
    // info!("Query: {records:#?}");
    // db.close().await?;
    // info!("db closed");

    // Ok(())

    // let socket = UdpSocket::bind("0.0.0.0:0").await?;
    // socket.connect(args.addr).await?;
    // let mut log = OpenOptions::new()
    //     .create(true)
    //     .append(true)
    //     .write(true)
    //     .open("readings.csv")
    //     .await?;
    // let mut id = 0u32;
    // let mut buf = vec![0u8; 1024];
    // let mut wait = false;
    // loop {
    //     if wait {
    //         time::sleep(Duration::from_secs_f64(args.delay)).await;
    //         wait = false;
    //     }
    //     id = id.wrapping_add(1);
    //     socket
    //         .send(&bincode::serialize(&RequestPacket {
    //             magic: REQUEST_PACKET_MAGIC,
    //             id,
    //         })?)
    //         .await?;
    //     let amnt = tokio::select! {
    //         amnt = socket.recv(&mut buf) => { amnt? }
    //         () = time::sleep(Duration::from_secs(1)) => {
    //             eprintln!("id:{id} timed out");
    //             continue;
    //         }
    //     };
    //     if amnt > buf.len() {
    //         eprintln!(
    //             "Received packet {} larger than receiving buffer",
    //             amnt - buf.len()
    //         );
    //         continue;
    //     }
    //     let Ok(pkt) = bincode::deserialize::<DataPacket>(&buf[0..amnt]) else { eprintln!("Failed to deserialize packet"); continue; };
    //     if pkt.id != id {
    //         eprintln!(
    //             "Received packet out of order: expect {} recv {}",
    //             id, pkt.id
    //         );
    //         continue;
    //     }
    //     log.write_all(
    //         format!(
    //             "{},{},{},{},{}\n",
    //             chrono::Utc::now(),
    //             pkt.observations.temperature,
    //             pkt.observations.humidity,
    //             pkt.observations.pressure,
    //             pkt.observations.battery,
    //         )
    //         .as_bytes(),
    //     )
    //     .await?;
    //     wait = true;
    // }
    //
}
