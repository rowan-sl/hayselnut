// #![allow(unused_imports)]

pub mod conf;
//pub mod lightning;

// pub mod lightening_sensor;
// use esp_idf_hal::gpio::{Gpio6, Unknown};
// use lightening_sensor::*;

// datasheet (chip only, different board): https://www.mouser.com/datasheet/2/588/ams_AS3935_Datasheet_EN_v5-1214568.pdf (ish)
// amazon page (the board that i have): https://www.amazon.com/GY-AS3935-Lighting-MA5532-AE-Distance-Detector/dp/B089D2WFKX/ref=sr_1_1?crid=1Y4LV3S3LU7JP&keywords=GY-AS3935+lightning+sensor&qid=1658423717&sprefix=gy-as3935+lightning+sensor%2Caps%2C223&sr=8-1

use std::{time::Duration, thread::sleep};

use anyhow::{bail, Result};
use embedded_svc::wifi::{ClientConfiguration, Configuration, Wifi};
use esp_idf_hal::{i2c, delay, peripheral, peripherals::Peripherals};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    netif::{EspNetif, EspNetifWait},
    wifi::{EspWifi, WifiWait},
};

// use crate::lightening_sensor::interface::i2c::{I2cInterface, I2cAddress};

// const SENSOR_I2C_ADDR: u8 = 0x03;

// struct Reg {
//     addr: u8,
//     mask: u8,
// }

fn main() -> Result<()> {
    esp_idf_sys::link_patches();

    let peripherals = Peripherals::take().unwrap();
    #[allow(unused)]
    let pins = peripherals.pins;
    // #[allow(unused)]
    // let sysloop = EspSystemEventLoop::take()?;
    // #[allow(unused)]
    // let wifi = wifi(peripherals.modem, sysloop.clone())?;

    let i2c = i2c::I2cDriver::new(peripherals.i2c0, pins.gpio7, pins.gpio6, &i2c::config::Config::new())?;
    let bus = shared_bus::BusManagerSimple::new(i2c);
    
    let mut sensor = bme280::i2c::BME280::new(bus.acquire_i2c(), 0x77);
    sensor.init(&mut delay::FreeRtos).unwrap();

    loop {
        sleep(Duration::from_millis(1000));
        println!("Read: {:?}", sensor.measure(&mut delay::FreeRtos))
    }

    // Ok(())

    //? blinky
    // let mut led = p.pins.gpio0.into_output()?;

    // loop {
    //     led.toggle()?;
    //     sleep(Duration::from_millis(1_000));
    // }

    //? lightning sensor / stuff
    // // println!("{} -> {}", 143, from_bits(bits(143)));

    // println!("starting");
    // let peripherals = Peripherals::take().unwrap();

    // // let sck = peripherals.pins.gpio6;
    // // let sdi = peripherals.pins.gpio7;

    // // let i2c = i2c::Master::new(peripherals.i2c0, i2c::MasterPins { sda: sdi, scl: sck }, i2c::config::MasterConfig::default().baudrate(100.kHz().into()))?;

    // let cs = peripherals.pins.gpio4.into_output()?;//chip select, input of sensor
    // let clk = peripherals.pins.gpio5.into_output()?;//clock, input of sensor
    // let mosi = peripherals.pins.gpio18.into_output()?;//data input of sensor
    // let miso = peripherals.pins.gpio19.into_input()?;// data output of sensor
    // let irq = peripherals.pins.gpio3.into_input()?;// interrupt

    // println!("setting up sensor");
    // let mut sensor = LightningSensor::new(cs, clk, mosi, miso)?;
    // sensor.perform_initial_configuration()?;
    // sensor.configure_minimum_lightning_threshold(&MinimumLightningThreshold::One)?;
    // sensor.configure_sensor_placing(&SensorLocation::Indoor)?;

    // // println!("configuring interupt");
    // // let (send, recv) = std::sync::mpsc::channel::<()>();
    // // let irq = unsafe {
    // //     peripherals.pins.gpio3.into_subscribed(move || {
    // //         let _ = send.send(());
    // //     }, esp_idf_hal::gpio::InterruptType::HighLevel)?
    // // };

    // println!("running");
    // // while let Ok(()) = recv.recv() {
    // loop {
    //     while !irq.is_high()? { sleep(Duration::from_millis(10)); }
    //     sleep(IRQ_TRIGGER_TO_READY_DELAY);// at least 2 is required, per the datasheet
    //     println!("{:?}", sensor.get_latest_event_and_reset()?);
    // }
}

#[allow(unused)]
fn wifi(
    modem: impl peripheral::Peripheral<P = esp_idf_hal::modem::Modem> + 'static,
    sysloop: EspSystemEventLoop,
) -> Result<Box<EspWifi<'static>>> {
    use std::net::Ipv4Addr;

    let known_networks: &[(
        &'static str, /* ssid */
        &'static str, /* password */
    )] = conf::WIFI_CFG;

    let mut wifi = Box::new(EspWifi::new(modem, sysloop.clone(), None)?);

    println!("Wifi created, about to scan");

    let ap_infos = wifi.scan()?;

    ap_infos
        .iter()
        .for_each(|ap| println!("Network: {}", &ap.ssid));
    let ours = ap_infos.into_iter().find(|a| {
        known_networks
            .iter()
            .find(|(ssid, _)| ssid == &a.ssid.as_str())
            .is_some()
    });

    let network = if let Some(ours) = ours {
        println!(
            "Found configured access point {} on channel {}",
            ours.ssid, ours.channel
        );
        ours
    } else {
        bail!("Configured access points not found during scanning");
    };

    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: network.ssid.clone().into(),
        password: (*known_networks
            .iter()
            .find(|(ssid, _)| ssid == &network.ssid.as_str())
            .unwrap())
        .1
        .into(),
        channel: Some(network.channel),
        ..Default::default()
    }))?;

    wifi.start()?;

    println!("Starting wifi...");

    if !WifiWait::new(&sysloop)?
        .wait_with_timeout(Duration::from_secs(20), || wifi.is_started().unwrap())
    {
        bail!("Wifi did not start");
    }

    println!("Connecting wifi...");

    wifi.connect()?;

    if !EspNetifWait::new::<EspNetif>(wifi.sta_netif(), &sysloop)?.wait_with_timeout(
        Duration::from_secs(20),
        || {
            wifi.is_connected().unwrap()
                && wifi.sta_netif().get_ip_info().unwrap().ip != Ipv4Addr::new(0, 0, 0, 0)
        },
    ) {
        bail!("Wifi did not connect or did not receive a DHCP lease");
    }

    let ip_info = wifi.sta_netif().get_ip_info()?;

    println!("Wifi DHCP info: {:?}", ip_info);

    // ping(ip_info.subnet.gateway)?;

    Ok(wifi)
}
