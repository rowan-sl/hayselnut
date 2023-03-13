use clap::Parser;
use serde::{Deserialize, Serialize};
use tracing::metadata::LevelFilter;
use std::{env, net::SocketAddr, path::PathBuf, time::Duration};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, net::UdpSocket, time};

pub mod tsdb;

#[derive(Parser)]
pub struct Args {
    #[arg(short, long, help = "IP address of the weather station to connect to")]
    addr: SocketAddr,
    #[arg(short, long, help = "Delay between readings from station (in seconds)")]
    delay: f64,
    #[arg(
        short,
        long,
        help = "Path for unix socket to communicate with the web server on"
    )]
    socket: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    tracing::subscriber::set_global_default(
        tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(
                tracing_subscriber::EnvFilter::builder()
                    .with_default_directive(LevelFilter::INFO.into())
                    .from_env()
                    .expect("Invalid logging config")
            )
            .pretty()
            .finish()
    ).expect("Failed to set tracing subscriber");

    Ok(())

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

//TODO add checksums
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DataPacket {
    id: u32,
    observations: Observations,
}

const REQUEST_PACKET_MAGIC: u32 = 0x3ce9abc2;
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RequestPacket {
    // so random other packets are ignored
    magic: u32,
    // echoed back in the data packet
    id: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Observations {
    /// degrees c
    temperature: f32,
    /// relative humidity (precentage)
    humidity: f32,
    /// pressure (pascals)
    pressure: f32,
    /// battery voltage (volts)
    battery: f32,
}
