#[macro_use]
extern crate async_trait;
#[macro_use]
extern crate tracing;

use clap::Parser;
use std::path::PathBuf;
use tracing::metadata::LevelFilter;

mod consumer;
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
    #[arg(short, long, help = "Path for the database")]
    db_path: PathBuf,
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

    let mut router = Router::new();
    router.with_consumer(RecordDB::new(&args.db_path).await?);

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
