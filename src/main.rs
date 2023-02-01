pub mod conf;
pub mod lightning;

use std::{time::Duration, net::Ipv4Addr, fmt::Write, thread::sleep};

use anyhow::{anyhow, bail, Result};
use bme280::i2c::BME280;
use embedded_svc::wifi::{self, AccessPointInfo, AuthMethod, Wifi};
use esp_idf_hal::{
    adc::{self, AdcDriver, Adc, AdcChannelDriver}, 
    gpio::ADCPin, i2c, units::FromValueType, delay,
    peripherals::Peripherals, peripheral::Peripheral,
};
use esp_idf_svc::{eventloop::EspSystemEventLoop, wifi::{EspWifi, WifiEvent, WifiWait}, netif::{EspNetifWait, EspNetif}};
use ssd1306::{I2CDisplayInterface, Ssd1306, prelude::DisplayConfig};
use futures::{select_biased, FutureExt, StreamExt};
use serde::{Serialize, Deserialize};
use smol::net::UdpSocket;

fn main() -> Result<()> {
    esp_idf_sys::link_patches();
    let peripherals = Peripherals::take().unwrap();
    let pins = peripherals.pins;

    // initializing connectiosn to things
    // battery monitor
    let mut batt_mon = BatteryMonitor::new(peripherals.adc1, pins.gpio4)?;

    // i2c bus (shared with display, and sensors)
    // NOTE: slow baudrate (for lightning sensor compat) will make the display slow
    let i2c_driver = i2c::I2cDriver::new(peripherals.i2c0, pins.gpio6, pins.gpio7, &i2c::config::Config::new().baudrate(100.kHz().into()))?;
    let i2c_bus = shared_bus::BusManagerSimple::new(i2c_driver); 

    // temp/humidity/pressure
    let mut bme280 = BME280::new(i2c_bus.acquire_i2c(), 0x77);
    bme280.init(&mut delay::Ets).map_err(|e| anyhow!("Failed to init bme280: {e:?}"))?;

    // oled display used for status on the device
    let display_interface = I2CDisplayInterface::new(i2c_bus.acquire_i2c());
    let mut display = Ssd1306::new(
        display_interface,
        ssd1306::size::DisplaySize128x64,
        ssd1306::rotation::DisplayRotation::Rotate0,
    ).into_terminal_mode();
    display.init().map_err(|e| anyhow!("Failed to init display: {e:?}"))?;
    let _ = display.clear();
    writeln!(display, "Starting...")?;
    // lightning sensor 
    // TODO
    
    let sysloop = EspSystemEventLoop::take()?;

    let (wifi_status_send, wifi_status_recv) = smol::channel::unbounded::<WifiStatusUpdate>();
    let _wifi_event_sub = sysloop.subscribe(move |event: &WifiEvent| {
        println!("Wifi event: {event:?}");
        match event {
            WifiEvent::StaDisconnected => wifi_status_send.try_send(WifiStatusUpdate::Disconnected).expect("Impossible! (unbounded queue is full???? (or main thread dead))"), 
            _ => {}
        }
    })?;
    
    write!(display, "Starting wifi...")?;
    let mut wifi = Box::new(EspWifi::new(
        peripherals.modem,
        sysloop.clone(),
        None,
    )?);

    writeln!(display, "Scanning...")?;
    // scan for available networks
    let available_aps = wifi.scan()?;
    sleep(Duration::from_secs(1));
    for (i, net) in available_aps.iter().enumerate() {
        let _ = display.clear();
        writeln!(
            display, 
            "Network ({}/{})\n{}\nSignal: {}dBm\n{}", 
            i+1, available_aps.len(),
            net.ssid,
            net.signal_strength,
            if conf::WIFI_CFG.iter().find(|(ssid, _)| ssid == &net.ssid).is_some() { "<known>" }
            else if net.auth_method == wifi::AuthMethod::None { "<open>" }
            else { "<locked>" },
        )?;
        sleep(Duration::from_secs(4));
    }
    let mut accessable_aps = filter_networks(available_aps, conf::INCLUDE_OPEN_NETWORKS);
    if accessable_aps.is_empty() {
        let _ = display.clear();
        writeln!(display, "No wifi found!")?;
        bail!("No accessable networks!!")
        //TODO eventually this should keep scanning, and not exit
    }
    let chosen_ap = accessable_aps.remove(0);
    drop(accessable_aps);
    let _ = display.clear();
    writeln!(
        display, 
        "Connecting to\n{}\nSignal: {}dBm\n{}", 
        chosen_ap.0.ssid,
        chosen_ap.0.signal_strength,
        if chosen_ap.1.is_some() { "<known net>" } else { "<open net>" }
    )?;
    sleep(Duration::from_secs(7));
    
    let _ = display.clear();
    writeln!(display, "Configuring...")?;
    wifi.set_configuration(&wifi::Configuration::Client(wifi::ClientConfiguration {
        ssid: chosen_ap.0.ssid.clone(),
        password: chosen_ap.1.unwrap_or_default().into(),
        channel: Some(chosen_ap.0.channel),
        ..Default::default()
    }))?;

    wifi.start()?;
    if !WifiWait::new(&sysloop)?
        .wait_with_timeout(Duration::from_secs(20), || wifi.is_started().unwrap()) {
        writeln!(display, "Wifi failed to start")?;
        bail!("Wifi did not start!");
    }

    wifi.connect()?;
    if !EspNetifWait::new::<EspNetif>(wifi.sta_netif(), &sysloop)?
        .wait_with_timeout(Duration::from_secs(20), || wifi.is_connected().unwrap() && wifi.sta_netif().get_ip_info().unwrap().ip != Ipv4Addr::new(0, 0, 0, 0)) {
        writeln!(display, "Wifi did not connect or receive a DHCP lease")?;
        bail!("Wifi did not connect or receive a DHCP lease")
    }

    let _ = display.clear();
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

        let mut socket = UdpSocket::bind("0.0.0.0:8080").await?;
        let mut socket_buf = [0u8; 1024];
        
        let mut time_before_read = smol::Timer::interval(Duration::from_secs(1));
        
        let mut current_measurements = Observations::default();
        //TODO: move future creation outside of select (and loop?)
        loop {
            select_biased! {
                status = wifi_status_recv.recv().fuse() => {
                    match status? {
                        WifiStatusUpdate::Disconnected => {
                            //TODO: keep scanning, dont exit
                            let _ = display.clear();
                            writeln!(display, "Wifi Disconnect!\nrestart device")?;
                            bail!("Wifi disconnected!");
                        }
                    }
                },
                res = socket.recv_from(&mut socket_buf).fuse() => {
                    let (amnt, addr) = res?;
                    if amnt > socket_buf.len() { continue }
                    match bincode::deserialize::<RequestPacket>(&socket_buf[0..amnt]) {
                        Ok(pkt) => {
                            if pkt.magic != REQUEST_PACKET_MAGIC { continue }
                            let response = DataPacket {
                                id: pkt.id,
                                observations: current_measurements.clone(),
                            };
                            let response_bytes = bincode::serialize(&response)?;
                            socket.send_to(&response_bytes, addr).await?;
                        }
                        Err(..) => continue,
                    }
                }
                _ = time_before_read.next().fuse() => {
                    // read sensors
                    let bme280::Measurements { temperature, pressure, humidity, .. }
                        = bme280.measure(&mut delay::Ets)
                        .map_err(|e| anyhow!("Failed to read bme280 sensor: {e:?}"))?;
                    let battery_voltage = batt_mon.read()?;
                    current_measurements = Observations {
                        temperature,
                        pressure,
                        humidity,
                        battery: battery_voltage,
                    };
                    let _ = display.clear();
                    write!(
                        display, 
                        "ON:{}\nIP:{}\nBAT:{:.1} TEMP:{:.0}F", 
                        chosen_ap.0.ssid,
                        ip_info.ip,
                        battery_voltage,
                        temperature * 1.8 + 32.0
                    )?;
                }
            };
        }

        // Ok(()) 
    })?;

    Ok(())
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
    include_open_networks: bool
) -> Vec<(AccessPointInfo, Option<&'static str>)> {
    // signal strength is measured in dBm (Decibls referenced to a miliwatt)
    // larger value = stronger signal 
    // should be in the range of ??? (30dBm = 1W transmission power) to -100(min wifi net received)
    let mut found = networks.into_iter()
        .filter_map(|net|
            conf::WIFI_CFG
                .iter()
                .find(|(ssid, _)| ssid == &net.ssid.as_str())
                .map(|(_, pass)| (net.clone(), Some(*pass)))
                .or(if net.auth_method == AuthMethod::None && include_open_networks { Some((net, None)) } else { None })
        ).collect::<Vec<_>>();
    found.sort_by(|a, b| {
        use std::cmp::Ordering::{Less, Equal, Greater};
        match (a.1, b.1) {
            (Some(..), None) => Greater,
            (None, Some(..)) => Less,
            (..) => Equal,
        }.then(a.0.signal_strength.cmp(&b.0.signal_strength))
    });
    found
}

#[derive(Debug, Clone)]
pub enum WifiStatusUpdate {
    Disconnected,
}

pub struct BatteryMonitor<'a, 'b, ADC: Adc, P: ADCPin>(AdcDriver<'a, ADC>, AdcChannelDriver<'b, P, adc::Atten11dB<P::Adc>>);

impl<'a, 'b, ADC: Adc, P: ADCPin> BatteryMonitor<'a, 'b, ADC, P> {
    pub fn new(adc: impl Peripheral<P=ADC> + 'a, pin: P) -> Result<Self> {
        Ok(Self (
            AdcDriver::new(adc, &adc::config::Config::new().calibration(true))?,
            AdcChannelDriver::new(pin)?
        ))
    }

    pub fn read(&mut self) -> Result<f32> {
        // mull by 2, measured through a voltage divider
        Ok((self.0.read(&mut self.1)? * 2 / 10) as f32 / 100.0)
    }
}

