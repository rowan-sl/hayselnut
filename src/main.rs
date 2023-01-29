// #![allow(unused_imports)]

pub mod conf;
pub mod lightning;

// pub mod lightening_sensor;
// use esp_idf_hal::gpio::{Gpio6, Unknown};
// use lightening_sensor::*;

// datasheet (chip only, different board): https://www.mouser.com/datasheet/2/588/ams_AS3935_Datasheet_EN_v5-1214568.pdf (ish)
// amazon page (the board that i have): https://www.amazon.com/GY-AS3935-Lighting-MA5532-AE-Distance-Detector/dp/B089D2WFKX/ref=sr_1_1?crid=1Y4LV3S3LU7JP&keywords=GY-AS3935+lightning+sensor&qid=1658423717&sprefix=gy-as3935+lightning+sensor%2Caps%2C223&sr=8-1

use std::{time::Duration, thread::sleep, fmt::Write as _, io::{Read as _, Write as _, self}, net::{TcpStream, TcpListener, SocketAddr}, sync::{mpsc, atomic}, thread};

use anyhow::{bail, Result, anyhow};
use embedded_hal::i2c::I2c;
use embedded_svc::{ipv4, wifi::{ClientConfiguration, Configuration, Wifi}};
use esp_idf_hal::{i2c, delay, peripheral::{self, Peripheral}, peripherals::Peripherals, adc::{self, AdcDriver, AdcChannelDriver}, units::FromValueType};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    netif::{EspNetif, EspNetifWait},
    wifi::{EspWifi, WifiWait},
    ping
};
use ssd1306::{prelude::*, I2CDisplayInterface, Ssd1306};

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
    #[allow(unused)]
    let sysloop = EspSystemEventLoop::take()?;

    let mut adc = AdcDriver::new(peripherals.adc1, &adc::config::Config::new().calibration(true))?;
    let mut adc_vbatt: esp_idf_hal::adc::AdcChannelDriver<'_, _, adc::Atten11dB<_>> = AdcChannelDriver::new(pins.gpio4)?;

    println!("Start I2C bus");
    let i2c = i2c::I2cDriver::new(peripherals.i2c0, pins.gpio6, pins.gpio7, &i2c::config::Config::new().baudrate(100.kHz().into()))?;
    let bus = shared_bus::BusManagerSimple::new(i2c);

    println!("Connect BME280");
    let mut sensor = bme280::i2c::BME280::new(bus.acquire_i2c(), 0x77);
    sensor.init(&mut delay::FreeRtos).unwrap();

    println!("Connect OLED display");
    let display_interface = I2CDisplayInterface::new(bus.acquire_i2c());
    let mut display =
        Ssd1306::new(display_interface, DisplaySize128x64, DisplayRotation::Rotate0).into_terminal_mode();
    display.init().unwrap();
    let _ = display.clear();

    println!("Connect AS3935");
    let mut lightning_i2c = bus.acquire_i2c();
    let mut buf = [0u8; 1];
    lightning_i2c.write_read(0x03, &[0x01], &mut buf)?;
    println!("Read: {:#010b}", buf[0]);

    // writeln!(display, "Starting WIFI")?;
    // #[allow(unused)]
    // let wifi = wifi(peripherals.modem, sysloop.clone(), &mut display)?;
    // let _ = display.clear();
    // writeln!(display, "Launch webserver")?;
    //
    // println!("Launch simple TCP server");
    // #[derive(Clone, Copy, Debug)]
    // struct Updates {
    //     // degrees f
    //     temperature: f64,
    //     // relative humidity
    //     humidity: f64,
    // }
    // #[derive(Debug)]
    // enum Status {
    //     Connect(SocketAddr),
    //     Disconnected,
    //     Error(anyhow::Error),
    // }
    // let (data_send, data_recv) = mpsc::sync_channel::<Updates>(5);
    // let (status_send, status_recv) = mpsc::sync_channel::<Status>(5);
    // thread::spawn(move || (|| -> Result<()> {
    //     let listener = TcpListener::bind("0.0.0.0:8080")?;
    //     loop {
    //         let mut client = listener.accept()?;
    //         println!("New connection to {:?}", client.1);
    //         // panic on channel full
    //         // TODO change this, so if a client connects/disconnects too fast it wont crash
    //         status_send.try_send(Status::Connect(client.1)).unwrap();
    //         for recv in &data_recv {
    //             let mut data = [0u8; 4+8+8];
    //             data[0..4].copy_from_slice(&[0xAB, 0xCD, 0x00, 0x00]);
    //             data[4..12].copy_from_slice(&recv.temperature.to_be_bytes());
    //             data[12..20].copy_from_slice(&recv.humidity.to_be_bytes());
    //             match client.0.write_all(&data) {
    //                 Ok(_) => {}
    //                 Err(err) => match err.kind() {
    //                     io::ErrorKind::TimedOut | io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted => { println!("{:?} disconnected", client.1); break; }
    //                     _ => {
    //                         Err(err)?
    //                     }
    //                 }
    //             }
    //         }
    //         status_send.try_send(Status::Disconnected).unwrap();
    //     }
    // })().map_err(|e| { status_send.try_send(Status::Error(e)).unwrap(); anyhow!("Error occured: sent to main thread for handling") }).unwrap());
    // println!("Running...");
    // let mut dropped = 0usize;
    // // none = disconnected
    // let mut client_addr: Option<SocketAddr> = None;
    // loop {
    //     sleep(Duration::from_millis(1000));
    //     let readings = sensor.measure(&mut delay::FreeRtos).map_err(|err| anyhow!("Failed to read sensor: {err:?}"))?;
    //     // println!("Read: {:?}", readings);
    //     let _ = display.clear();
    //     writeln!(display, "t:{:.1}f h:{:.1}%", readings.temperature * 1.8 + 32.0, readings.humidity)?;     
    //
    //     let vbatt = adc.read(&mut adc_vbatt)?;
    //     writeln!(display, "batt:{}v", (vbatt * 2 / 10) as f32 / 100.0)?;
    //
    //     match status_recv.try_recv() {
    //         Ok(Status::Connect(addr)) => {
    //             client_addr = Some(addr);
    //             dropped = 0;
    //         }
    //         Ok(Status::Disconnected) => client_addr = None,
    //         Ok(Status::Error(e)) => {
    //             let _ = display.clear();
    //             println!("NET ERROR: {:?}", e);
    //             write!(display, "NET ERROR: {:?}", e)?;
    //             loop {}
    //         }
    //         Err(..) => {}
    //     }
    //
    //     if data_send.try_send(Updates { temperature: (readings.temperature * 1.8 + 32.0) as f64, humidity: readings.humidity as f64}).is_err() {
    //         dropped += 1;
    //     } else {
    //         dropped = 0;
    //     }
    //
    //     writeln!(
    //         display, "NET:{} {} IP:{:?}",
    //         wifi.get_configuration()?.as_client_conf_ref().unwrap().ssid,
    //         if wifi.is_up()? { "UP" } else { "DOWN" },
    //         wifi.sta_netif().get_ip_info()?.ip
    //     )?;
    //     if let Some(addr) = &client_addr {
    //         writeln!(display, "Srv:{:?}", addr.ip())?;
    //         if dropped < 9999 { 
    //             writeln!(display, "Dropped: {}", dropped)?;
    //         } else {
    //             writeln!(display, "Dropped: >10k")?;
    //         }
    //     } else {
    //         writeln!(display, "Disconnected")?;
    //     }
    //
    // }

    Ok(())

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
    display: &mut Ssd1306<I2CInterface<shared_bus::I2cProxy<shared_bus::NullMutex<i2c::I2cDriver>>>, DisplaySize128x64, ssd1306::mode::TerminalMode>,
) -> Result<Box<EspWifi<'static>>> {
    use std::net::Ipv4Addr;

    let known_networks: &[(
        &'static str, /* ssid */
        &'static str, /* password */
    )] = conf::WIFI_CFG;

    let mut wifi = Box::new(EspWifi::new(modem, sysloop.clone(), None)?);

    println!("Wifi created, about to scan");

    let ap_infos = wifi.scan()?;
    for (i, ap) in ap_infos.iter().enumerate() {
        println!("Network: {}", &ap.ssid);
        let _ = display.clear();
        write!(display, "AP {}/{}\nSSID:{}\nsignal:{}\n{}", 
            i+1, ap_infos.len(),
            ap.ssid,
            ap.signal_strength,
            if true { "known network" } 
            else if let embedded_svc::wifi::AuthMethod::None = ap.auth_method { "<no password>" } 
            else { "" }
        )?;
        sleep(Duration::from_millis(1500));
    }
    let _ = display.clear();
    writeln!(display, "Finding known network...");
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
        writeln!(display, "Found {}\nChannel {}", ours.ssid, ours.channel);
        sleep(Duration::from_millis(1000));
        ours
    } else {
        writeln!(display, "No networks found!");
        bail!("Configured access points not found during scanning");
    };

    let _ = display.clear();
    writeln!(display, "Configure...");

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

    writeln!(display, "Start...")?;
    println!("Starting wifi...");
    wifi.start()?;

    if !WifiWait::new(&sysloop)?
        .wait_with_timeout(Duration::from_secs(20), || wifi.is_started().unwrap())
    {
        writeln!(display, "Wifi did not start");
        bail!("Wifi did not start");
    }

    writeln!(display, "Connecting...");
    println!("Connecting wifi...");

    wifi.connect()?;

    if !EspNetifWait::new::<EspNetif>(wifi.sta_netif(), &sysloop)?.wait_with_timeout(
        Duration::from_secs(20),
        || {
            wifi.is_connected().unwrap()
                && wifi.sta_netif().get_ip_info().unwrap().ip != Ipv4Addr::new(0, 0, 0, 0)
        },
    ) {
        let _ = display.clear();
        writeln!(display, "Did not connect / did not receive a DHCP lease");
        bail!("Wifi did not connect or did not receive a DHCP lease");
    }

    let ip_info = wifi.sta_netif().get_ip_info()?;

    println!("Wifi DHCP info: {:?}", ip_info);

    let _ = display.clear();
    writeln!(display, "Ping gateway \n{:?}", ip_info.subnet.gateway);
    ping(ip_info.subnet.gateway).map_err(|e| {
        writeln!(display, "Ping failure!");
        e
    })?;

    Ok(wifi)
}

fn ping(ip: ipv4::Ipv4Addr) -> Result<()> {
    println!("About to do some pings for {:?}", ip);

    let ping_summary = ping::EspPing::default().ping(ip, &Default::default())?;
    if ping_summary.transmitted != ping_summary.received {
        bail!("Pinging IP {} resulted in timeouts", ip);
    }

    println!("Pinging done");

    Ok(())
}
