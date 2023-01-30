pub mod conf;
pub mod lightning;

use std::{time::Duration, net::Ipv4Addr};

use anyhow::{anyhow, bail, Result};
use bme280::i2c::BME280;
use embedded_svc::wifi::{self, AccessPointInfo, AuthMethod, Wifi};
use esp_idf_hal::{
    adc::{self, AdcDriver, Adc, AdcChannelDriver}, 
    gpio::ADCPin, i2c, units::FromValueType, delay,
    peripherals::Peripherals, peripheral::Peripheral,
};
use esp_idf_svc::{eventloop::EspSystemEventLoop, wifi::{EspWifi, WifiEvent, WifiWait}, netif::{EspNetifWait, EspNetif}};
use smol::net::TcpListener;
use ssd1306::{I2CDisplayInterface, Ssd1306, prelude::DisplayConfig};
use futures::{select_biased, FutureExt, StreamExt};
use serde::{Serialize, Deserialize};

fn main() -> Result<()> {
    esp_idf_sys::link_patches();
    let peripherals = Peripherals::take().unwrap();
    let pins = peripherals.pins;

    // initializing connectiosn to things
    // battery monitor
    let mut batt_mon = BatteryMonitor::new(peripherals.adc1, pins.gpio4)?;

    // i2c bus (shared with display, and sensors)
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

    // lightning sensor 
    // TODO
    
    let sysloop = EspSystemEventLoop::take()?;

    let (wifi_status_send, wifi_status_recv) = smol::channel::unbounded::<WifiStatusUpdate>();
    let wifi_event_sub = sysloop.subscribe(move |event: &WifiEvent| {
        println!("Wifi event: {event:?}");
        match event {
            WifiEvent::StaDisconnected => wifi_status_send.try_send(WifiStatusUpdate::Disconnected).expect("Impossible! (unbounded queue is full???? (or main thread dead))"), 
            _ => {}
        }
    })?;

    let mut wifi = Box::new(EspWifi::new(
        peripherals.modem,
        sysloop.clone(),
        None,
    )?);

    // scan for available networks
    let available_aps = wifi.scan()?;
    let mut accessable_aps = filter_networks(available_aps, conf::INCLUDE_OPEN_NETWORKS);
    if accessable_aps.is_empty() {
        bail!("No accessable networks!!")
        //TODO eventually this should keep scanning, and not exit
    }
    let chosen_ap = accessable_aps.remove(0);
    drop(accessable_aps);

    wifi.set_configuration(&wifi::Configuration::Client(wifi::ClientConfiguration {
        ssid: chosen_ap.0.ssid,
        password: chosen_ap.1.unwrap_or_default().into(),
        channel: Some(chosen_ap.0.channel),
        ..Default::default()
    }))?;

    wifi.start()?;
    if !WifiWait::new(&sysloop)?
        .wait_with_timeout(Duration::from_secs(20), || wifi.is_started().unwrap()) {
        bail!("Wifi did not start!");
    }

    wifi.connect()?;
    if !EspNetifWait::new::<EspNetif>(wifi.sta_netif(), &sysloop)?
        .wait_with_timeout(Duration::from_secs(20), || wifi.is_connected().unwrap() && wifi.sta_netif().get_ip_info().unwrap().ip != Ipv4Addr::new(0, 0, 0, 0)) {
        bail!("Wifi did not connect or receive a DHCP lease")
    }

    let ip_info = wifi.sta_netif().get_ip_info()?;
    println!("Wifi DHCP info {:?}", ip_info);

    smol::block_on(async {
        println!("Async executor started");
        //TODO: move sensor readindg into another thread to avoid blocking the main one 
        // while reading data 

        // bme280.measure(&mut delay::Ets).map_err(|e| anyhow!("Failed to read bme280 sensor"))?;
        // batt_mon.read()?;

        let listener = TcpListener::bind("0.0.0.0:8080").await?;
        let mut listener_stream = listener.incoming();
        let mut time_before_read = smol::Timer::interval(Duration::from_secs(1));
        //TODO: move future creation outside of select (and loop?)
        select_biased! {
            status = wifi_status_recv.recv().fuse() => {
                match status? {
                    WifiStatusUpdate::Disconnected => {
                        //TODO: keep scanning, dont exit
                        bail!("Wifi disconnected!");
                    }
                }
            },
            new_connection = listener_stream.next().fuse() => {
                // will allways be Some() (see `Incoming` docs)
                let new_connection = new_connection.unwrap()?;
            }
            _ = time_before_read.next().fuse() => {
                // read sensors, put in queue
            }
        };

        Ok(()) 
    })?;
    
    Ok(())
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Packet {

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

