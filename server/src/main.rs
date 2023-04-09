#[macro_use]
extern crate async_trait;
#[macro_use]
extern crate tracing;
#[macro_use]
extern crate anyhow;

use clap::Parser;
use std::path::PathBuf;
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncReadExt,
};
use tracing::metadata::LevelFilter;

mod consumer;
mod paths;
mod route;
mod station;
pub mod tsdb;

use consumer::db::RecordDB;
use route::Router;

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
    let stations = if stations_path.exists() {
        if !stations_path.is_file() {
            error!("stations.json exists, but it is not a file");
            bail!("error loading station info");
        }
        let mut f = OpenOptions::new().read(true).open(stations_path).await?;
        let mut buf = String::new();
        f.read_to_string(&mut buf).await?;
        station::identity::KnownStations::from_json(&buf).map_err(|e| {
            error!("Failed to load stations.json");
            e
        })?
    } else {
        station::identity::KnownStations::new()
    };

    debug!("Loaded known stations:");
    for s in stations.stations() {
        // in the future, station info should be printed
        let _info = stations.get_info(s).unwrap();
        debug!("Known station {}", s);
    }

    let mut router = Router::new();
    router.with_consumer(RecordDB::new(&records_dir.path("data.tsdb")).await?);

    // ... code ...

    router.close().await;

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
