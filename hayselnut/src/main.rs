#[macro_use]
extern crate log;

pub mod battery;
pub mod conf;
pub mod lightning;
pub mod store;

use std::{
    convert::TryInto,
    fmt::Write,
    net::{Ipv4Addr, SocketAddr},
    time::Duration,
};

use anyhow::{anyhow, bail, Result};
use bme280::i2c::BME280;
use embedded_svc::wifi::{self, AccessPointInfo, AuthMethod, Wifi};
use esp_idf_hal::{delay, i2c, peripherals::Peripherals, reset::ResetReason, units::FromValueType};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    netif::{EspNetif, EspNetifWait},
    nvs::EspDefaultNvsPartition,
    wifi::{EspWifi, WifiEvent, WifiWait},
};
use esp_idf_sys as _; // allways should be imported if `binstart` feature is enabled.
use futures::{select_biased, FutureExt, StreamExt};
use serde::{Deserialize, Serialize};
use smol::{
    net::{resolve, UdpSocket},
    Timer,
};
use squirrel::{
    api::{
        station::capabilities::{Channel, ChannelName, ChannelType, ChannelValue},
        PacketKind,
    },
    transport::{
        client::{mvp_recv, mvp_send},
        UidGenerator,
    },
};
use ssd1306::{prelude::DisplayConfig, I2CDisplayInterface, Ssd1306};

use battery::BatteryMonitor;
use store::{StationStoreAccess, StationStoreData};
use uuid::Uuid;

fn main() -> Result<()> {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    {
        use ResetReason::*;
        match ResetReason::get() {
            // should be normal reset conditions
            ExternalPin | PowerOn => {}
            // could be used for something in the future
            // see https://github.com/esp-rs/esp-idf-hal/issues/128 for a way of storing info between sleeps
            //  (storing in RTC fast memory, with `#[link_section=".rtc.data"] static mut VAR`)
            Software | DeepSleep => {}
            // report and wait (?) -- caused by some software issue (Sdio - unknown what it is)
            _reason @ (Watchdog | InterruptWatchdog | TaskWatchdog | Sdio) => {}
            // tentatively continue as normal
            Unknown => {}
            // report and wait for reset
            Panic => {}
            // wait for battery to raise above some level
            Brownout => {}
        }
    }

    let peripherals = Peripherals::take().unwrap();
    let pins = peripherals.pins;

    let nvs_partition = EspDefaultNvsPartition::take()?;

    // initializing connectiosn to things
    // battery monitor
    let mut batt_mon = BatteryMonitor::new(peripherals.adc1, pins.gpio4)?;
    // i2c bus (shared with display, and sensors)
    // NOTE: slow baudrate (for lightning sensor compat) will make the display slow
    let i2c_driver = i2c::I2cDriver::new(
        peripherals.i2c0,
        pins.gpio6,
        pins.gpio7,
        &i2c::config::Config::new().baudrate(100.kHz().into()),
    )?;
    let i2c_bus = shared_bus::BusManagerSimple::new(i2c_driver);

    // temp/humidity/pressure
    let mut bme280 = BME280::new(i2c_bus.acquire_i2c(), 0x77);
    bme280
        .init(&mut delay::Ets)
        .map_err(|e| anyhow!("Failed to init bme280: {e:?}"))?;

    // oled display used for status on the device
    let display_interface = I2CDisplayInterface::new(i2c_bus.acquire_i2c());
    let mut display = Ssd1306::new(
        display_interface,
        ssd1306::size::DisplaySize128x64,
        ssd1306::rotation::DisplayRotation::Rotate0,
    )
    .into_terminal_mode();
    display
        .init()
        .map_err(|e| anyhow!("Failed to init display: {e:?}"))?;
    let _ = display.clear();
    writeln!(display, "Starting...")?;
    // lightning sensor
    // TODO

    let sysloop = EspSystemEventLoop::take()?;
    let (wifi_status_send, wifi_status_recv) = smol::channel::unbounded::<WifiStatusUpdate>();
    let _wifi_event_sub = sysloop.subscribe(move |event: &WifiEvent| {
        // println!("Wifi event: {event:?}");
        match event {
            WifiEvent::StaDisconnected => wifi_status_send
                .try_send(WifiStatusUpdate::Disconnected)
                .expect("Impossible! (unbounded queue is full???? (or main thread dead))"),
            _ => {}
        }
    })?;

    write!(display, "Starting wifi...")?;
    let mut wifi = Box::new(EspWifi::new(
        peripherals.modem,
        sysloop.clone(),
        Some(nvs_partition.clone()),
    )?);
    wifi.set_configuration(&wifi::Configuration::Client(
        wifi::ClientConfiguration::default(),
    ))?;
    wifi.start()?;
    if !WifiWait::new(&sysloop)?
        .wait_with_timeout(Duration::from_secs(20), || wifi.is_started().unwrap())
    {
        // writeln!(display, "Wifi failed to start")?;
        bail!("Wifi did not start!");
    }

    // wifi.start()?;
    // -- NVS station information initialization --
    // // performed here since it uses random numbers, and `getrandom` on the esp32
    // // requires wifi / bluetooth to be enabled for true random numbers
    let mut store = StationStoreAccess::new(nvs_partition.clone())?;
    let station_info = if !store.exists()? {
        warn!("Performing first-time initialization of station information");
        let default = StationStoreData {
            station_uuid: Uuid::new_v4(),
        };
        warn!("Picked a UUID of {}", default.station_uuid);
        store.write(&default)?;
        default
    } else {
        store.read()?.unwrap()
    };
    info!("Loaded station info: {station_info:#?}");
    // -- end NVS info init --

    // writeln!(display, "Scanning...")?;
    // scan for available networks
    let available_aps = wifi.scan()?;
    // sleep(Duration::from_secs(1));
    // for (i, net) in available_aps.iter().enumerate() {
    //     let _ = display.clear();
    //     writeln!(
    //         display,
    //         "Network ({}/{})\n{}\nSignal: {}dBm\n{}",
    //         i + 1,
    //         available_aps.len(),
    //         net.ssid,
    //         net.signal_strength,
    //         if conf::WIFI_CFG
    //             .iter()
    //             .find(|(ssid, _)| ssid == &net.ssid)
    //             .is_some()
    //         {
    //             "<known>"
    //         } else if net.auth_method == wifi::AuthMethod::None {
    //             "<open>"
    //         } else {
    //             "<locked>"
    //         },
    //     )?;
    //     sleep(Duration::from_secs(4));
    // }
    let mut accessable_aps = filter_networks(available_aps, conf::INCLUDE_OPEN_NETWORKS);
    if accessable_aps.is_empty() {
        // let _ = display.clear();
        // writeln!(display, "No wifi found!")?;
        bail!("No accessable networks!!")
        //TODO eventually this should keep scanning, and not exit
    }
    let chosen_ap = accessable_aps.remove(0);
    drop(accessable_aps);
    // let _ = display.clear();
    // writeln!(
    //     display,
    //     "Connecting to\n{}\nSignal: {}dBm\n{}",
    //     chosen_ap.0.ssid,
    //     chosen_ap.0.signal_strength,
    //     if chosen_ap.1.is_some() {
    //         "<known net>"
    //     } else {
    //         "<open net>"
    //     }
    // )?;
    // sleep(Duration::from_secs(7));

    // let _ = display.clear();
    // writeln!(display, "Configuring...")?;
    wifi.set_configuration(&wifi::Configuration::Client(wifi::ClientConfiguration {
        ssid: chosen_ap.0.ssid.clone(),
        password: chosen_ap.1.unwrap_or_default().into(),
        channel: Some(chosen_ap.0.channel),
        ..Default::default()
    }))?;

    // wifi.start()?;
    // if !WifiWait::new(&sysloop)?
    //     .wait_with_timeout(Duration::from_secs(20), || wifi.is_started().unwrap())
    // {
    //     // writeln!(display, "Wifi failed to start")?;
    //     bail!("Wifi did not start!");
    // }

    wifi.connect()?;
    if !EspNetifWait::new::<EspNetif>(wifi.sta_netif(), &sysloop)?.wait_with_timeout(
        Duration::from_secs(20),
        || {
            wifi.is_connected().unwrap()
                && wifi.sta_netif().get_ip_info().unwrap().ip != Ipv4Addr::new(0, 0, 0, 0)
        },
    ) {
        // writeln!(display, "Wifi did not connect or receive a DHCP lease")?;
        bail!("Wifi did not connect or receive a DHCP lease")
    }

    // let _ = display.clear();
    let ip_info = wifi.sta_netif().get_ip_info()?;
    println!("Wifi DHCP info {:?}", ip_info);

    // black magic
    // if this is not present, the call to UdpSocket::bind fails
    {
        esp_idf_sys::esp!(unsafe {
            esp_idf_sys::esp_vfs_eventfd_register(&esp_idf_sys::esp_vfs_eventfd_config_t {
                max_fds: 5,
                ..Default::default()
            })
        })?;
    }

    smol::block_on(async {
        println!("Async executor started");
        let sock = UdpSocket::bind("0.0.0.0:0").await?;
        let mut ips = resolve(conf::SERVER).await?;
        if ips.len() == 0 {
            bail!("Failed to resolve server address -- DNS lookup found nothing");
        } else if ips.len() > 1 {
            bail!("Faild to respolve server address -- multiple results ({ips:?})");
        }
        //temp hardcoded IP
        ips[0] = SocketAddr::new([10, 1, 10, 9].into(), 43210);
        sock.connect(ips[0]).await?;
        println!("connected to: {:?}", sock.peer_addr()?);

        let mut uid_gen = UidGenerator::new();
        let channels = vec![
            Channel {
                name: "temperature".into(),
                value: ChannelValue::Float,
                ty: ChannelType::Periodic,
            },
            Channel {
                name: "humidity".into(),
                value: ChannelValue::Float,
                ty: ChannelType::Periodic,
            },
            Channel {
                name: "pressure".into(),
                value: ChannelValue::Float,
                ty: ChannelType::Periodic,
            },
        ];

        // send init packet
        info!("sending init info");
        let packet = PacketKind::Connect(squirrel::api::OnConnect {
            station_id: station_info.station_uuid,
            channels,
        });
        mvp_send(
            &sock,
            &rmp_serde::to_vec_named(&packet).unwrap(),
            &mut uid_gen,
        )
        .await;

        info!("receiving channel mappings");
        let recv = loop {
            match mvp_recv(&sock, &mut uid_gen).await {
                Some(p) => break p,
                None => {
                    debug!("retry receive in 5s (got empty response = no packet ready yet)");
                    Timer::after(Duration::from_secs(5)).await;
                }
            }
        };
        let mappings = match rmp_serde::from_slice(&recv) {
            Ok(PacketKind::ChannelMappings(map)) => map,
            Ok(other) => bail!("Expected channel mappings, got {other:?}"),
            Err(..) => bail!("Failed to deserialize received data"),
        };
        info!("channel mappings: {mappings:#?}");

        // println!("attempting to send test data");
        // let mut gen = squirrel::transport::UidGenerator::new();
        // let data = 0xDEADBEEFu32.to_be_bytes();
        // println!("Sending data: {data:?}");
        // squirrel::transport::client::mvp_send(&sock, &data, &mut gen).await;
        // let data = squirrel::transport::client::mvp_recv(&sock, &mut gen)
        //     .await
        //     .unwrap_or(vec![]);
        // println!("Received (echo): {data:?}");
        // println!("done");

        //NOTE (on UDP)
        // - max packet size (?) http://www.tcpipguide.com/free/t_IPDatagramSizetheMaximumTransmissionUnitMTUandFrag-4.htm
        // - this probably means that any transmissions with sizes that are
        // different than the entire data should be ignored
        // - packets can be corrupted
        // - packets (at least small ones) will not be received in multiple parts
        // - packets can be received more than once, or not at all

        //TODO: move sensor readindg into another thread to avoid blocking the main one
        // while reading data

        // batt_mon.read()?;

        // let socket = UdpSocket::bind("0.0.0.0:8080").await?;
        // let mut socket_buf = [0u8; 1024];
        //
        // let mut current_measurements = Observations::default();
        // let config = MeasureConfig::default();
        // let mut timers = MeasureTimers::with_config(&config);
        // //TODO: move future creation outside of select (and loop?)
        // loop {
        //     select_biased! {
        //         status = wifi_status_recv.recv().fuse() => {
        //             match status? {
        //                 WifiStatusUpdate::Disconnected => {
        //                     //TODO: keep scanning, dont exit
        //                     let _ = display.clear();
        //                     writeln!(display, "Wifi Disconnect!\nrestart device")?;
        //                     bail!("Wifi disconnected!");
        //                 }
        //             }
        //         },
        //         res = socket.recv_from(&mut socket_buf).fuse() => {
        //             let (amnt, addr) = res?;
        //             if amnt > socket_buf.len() { continue }
        //             match bincode::deserialize::<RequestPacket>(&socket_buf[0..amnt]) {
        //                 Ok(pkt) => {
        //                     if pkt.magic != REQUEST_PACKET_MAGIC { continue }
        //                     let response = DataPacket {
        //                         id: pkt.id,
        //                         observations: current_measurements.clone(),
        //                     };
        //                     let response_bytes = bincode::serialize(&response)?;
        //                     socket.send_to(&response_bytes, addr).await?;
        //                 }
        //                 Err(..) => continue,
        //             }
        //         }
        //         _ = timers.read_timer.next().fuse() => {
        //             // read sensors
        //             let bme280::Measurements { temperature, pressure, humidity, .. }
        //                 = bme280.measure(&mut delay::Ets)
        //                 .map_err(|e| anyhow!("Failed to read bme280 sensor: {e:?}"))?;
        //             let battery_voltage = batt_mon.read()?;
        //             current_measurements = Observations {
        //                 temperature,
        //                 pressure,
        //                 humidity,
        //                 battery: battery_voltage,
        //             };
        //             let _ = display.clear();
        //             write!(
        //                 display,
        //                 "ON:{}\nIP:{}\nBAT:{:.1} TEMP:{:.0}F",
        //                 chosen_ap.0.ssid,
        //                 ip_info.ip,
        //                 battery_voltage,
        //                 temperature * 1.8 + 32.0
        //             )?;
        //         }
        //     };
        // }

        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[derive(Debug, Clone)]
pub struct MeasureConfig {
    read_interval: Duration,
}

impl Default for MeasureConfig {
    fn default() -> Self {
        Self {
            read_interval: Duration::from_secs(30),
        }
    }
}

#[derive(Debug)]
pub struct MeasureTimers {
    pub read_timer: smol::Timer,
}

impl MeasureTimers {
    pub fn with_config(cfg: &MeasureConfig) -> Self {
        Self {
            read_timer: smol::Timer::interval(cfg.read_interval),
        }
    }

    pub fn update_new_cfg(&mut self, new_cfg: &MeasureConfig) {
        self.read_timer.set_interval(new_cfg.read_interval);
    }
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

/// find and return all known wifi networks, or ones that have no password,
/// in order of signal strength. known networks are prioritized
/// over ones with no password, and networks with no password can be removed entierly
/// with the `include_open_networks` option
///
/// Returns a list of networks and their passwords (None=needs no password)
pub fn filter_networks(
    networks: Vec<AccessPointInfo>,
    include_open_networks: bool,
) -> Vec<(AccessPointInfo, Option<&'static str>)> {
    // signal strength is measured in dBm (Decibls referenced to a miliwatt)
    // larger value = stronger signal
    // should be in the range of ??? (30dBm = 1W transmission power) to -100(min wifi net received)
    let mut found = networks
        .into_iter()
        .filter_map(|net| {
            conf::WIFI_CFG
                .iter()
                .find(|(ssid, _)| ssid == &net.ssid.as_str())
                .map(|(_, pass)| (net.clone(), Some(*pass)))
                .or(
                    if net.auth_method == AuthMethod::None && include_open_networks {
                        Some((net, None))
                    } else {
                        None
                    },
                )
        })
        .collect::<Vec<_>>();
    found.sort_by(|a, b| {
        use std::cmp::Ordering::{Equal, Greater, Less};
        match (a.1, b.1) {
            (Some(..), None) => Greater,
            (None, Some(..)) => Less,
            (..) => Equal,
        }
        .then(a.0.signal_strength.cmp(&b.0.signal_strength))
    });
    found
}

#[derive(Debug, Clone)]
pub enum WifiStatusUpdate {
    Disconnected,
}
