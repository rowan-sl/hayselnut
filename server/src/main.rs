use std::{env, time::Duration};
use tokio::{io::AsyncWriteExt, fs::OpenOptions, net::UdpSocket, time};
use serde::{Serialize, Deserialize};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let addr = env::var("ADDR").expect("Missing ADDR (env) to connect to");
    let delay: u64 = env::var("DELAY").expect("Missing DELAY (env) between readings").parse().expect("Delay is not a number!");
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.connect(addr).await?;
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .write(true)
        .open("readings.csv")
        .await?;
    let mut id = 0u32;
    let mut buf = vec![0u8; 1024];
    let mut wait = false;
    loop {
        if wait {
            time::sleep(Duration::from_secs(delay)).await;
            wait = false;
        }
        id = id.wrapping_add(1);
        socket.send(&bincode::serialize(&RequestPacket {
            magic: REQUEST_PACKET_MAGIC,
            id,
        })?).await?;
        let amnt = tokio::select!{
            amnt = socket.recv(&mut buf) => { amnt? }
            () = time::sleep(Duration::from_secs(1)) => {
                eprintln!("id:{id} timed out");
                continue;
            }
        }; 
        if amnt > buf.len() {
            eprintln!("Received packet {} larger than receiving buffer", amnt-buf.len());
            continue;
        }
        let Ok(pkt) = bincode::deserialize::<DataPacket>(&buf[0..amnt]) else { eprintln!("Failed to deserialize packet"); continue; };
        if pkt.id != id {
            eprintln!("Received packet out of order: expect {} recv {}", id, pkt.id);
           continue; 
        }
        log.write_all(format!(
            "{},{},{},{},{}\n",
            chrono::Utc::now(),
            pkt.observations.temperature,
            pkt.observations.humidity,
            pkt.observations.pressure,
            pkt.observations.battery,
        ).as_bytes()).await?;   
        wait = true;
    }
    // let addr = env::var("ADDR").expect("Missing ADDR env variable");
    // println!("bind");
    // let socket = UdpSocket::bind("0.0.0.0:0")?;
    // socket.connect(addr)?;
    // println!("send");
    // socket.send(&bincode::serialize(&RequestPacket { magic: REQUEST_PACKET_MAGIC, id: 12345})?)?;
    // socket.set_read_timeout(Some(Duration::from_secs(5)))?;
    // let mut buf = [0u8; 1024];
    // let res = socket.recv(&mut buf)?;
    // if res > buf.len() { bail!("Received packet too large") }
    // let decoded = bincode::deserialize::<DataPacket>(&buf[0..res])?;
    // if decoded.id != 12345 { bail!("ID wrong") }
    // println!("{:#?}", decoded.observations);

    // let mut connection = TcpStream::connect(env::var("ADDR").unwrap())?;
    // let mut buf = [0u8; 20];
    // loop {
    //     connection.read_exact(&mut buf)?;
    //     assert!(&buf[0..4] == [0xABu8, 0xCD, 0x00, 0x00].as_slice());
    //     let temperature = f64::from_be_bytes(buf[4..12].try_into().unwrap());
    //     let humidity = f64::from_be_bytes(buf[12..20].try_into().unwrap());
    //     println!("temperature: {temperature}, humidity: {humidity}")
    // }
    // Ok(())
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


