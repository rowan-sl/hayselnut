#![feature(sync_unsafe_cell)]

#[macro_use]
extern crate log;

pub mod battery;
pub mod conf;
pub mod lightning;
pub mod store;
pub mod wifictl;

use std::{cell::SyncUnsafeCell, collections::HashMap, fmt::Write, thread::sleep, time::Duration};

use anyhow::{anyhow, bail, Result};
use bme280::i2c::BME280;
use esp_idf_hal::{
    delay, gpio::PinDriver, i2c, peripherals::Peripherals, reset::ResetReason, units::FromValueType,
};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    nvs::EspDefaultNvsPartition,
    wifi::{EspWifi, WifiEvent},
};
use esp_idf_sys::{self as _, esp_deep_sleep_start, esp_sleep_disable_wakeup_source}; // allways should be imported if `binstart` feature is enabled.
use futures::{select_biased, FutureExt, StreamExt};
use serde::{Deserialize, Serialize};
use smol::{
    net::{resolve, UdpSocket},
    Timer,
};
use squirrel::{
    api::{
        station::capabilities::{
            Channel, ChannelData, ChannelID, ChannelName, ChannelType, ChannelValue,
        },
        PacketKind, SomeData,
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

use crate::wifictl::Wifi;

fn main() -> Result<()> {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_sys::link_patches();

    {
        // stored in RTC fast memory, not powered off by default even in deep sleep
        // saftey of access: pinky promise that this code is single threadded
        #[link_section = ".rtc.data"]
        static DEEP_SLEEP_CAUSE: SyncUnsafeCell<SleepCause> = SyncUnsafeCell::new(SleepCause::None);

        #[derive(Clone, Copy)]
        enum SleepCause {
            None,
            Panic,
        }

        use ResetReason::*;
        match ResetReason::get() {
            // should be normal reset conditions
            ExternalPin | PowerOn => {}
            // could be used for something in the future
            // see https://github.com/esp-rs/esp-idf-hal/issues/128 for a way of storing info between sleeps
            //  (storing in RTC fast memory, with `#[link_section=".rtc.data"] static mut VAR`)
            DeepSleep => {
                match unsafe { *DEEP_SLEEP_CAUSE.get() } {
                    SleepCause::None => {} // hmmmmm
                    SleepCause::Panic => {
                        // esp docs LIE! (somehow, the chip was woken from deep sleep)
                        // leave the cause as is
                        // sleep forever (or is it)
                        unsafe {
                            esp_deep_sleep_start();
                        }
                    }
                }
            }
            //????
            Software => {}
            // report and wait (?) -- caused by some software issue (Sdio - unknown what it is)
            _reason @ (Watchdog | InterruptWatchdog | TaskWatchdog | Sdio) => {}
            // tentatively continue as normal
            Unknown => {}
            // report and wait for reset
            Panic => {
                // if printing fails, avoid panicing again
                let _ = std::panic::catch_unwind(|| {
                    eprintln!("Chip restarted due to panic -- halting to avoid repeated panicing");
                    eprintln!("restart chip to exit halted mode");
                });
                unsafe {
                    // sleep forever
                    *DEEP_SLEEP_CAUSE.get() = SleepCause::Panic;
                    // set the led indicator
                    let mut indicator =
                        PinDriver::output(Peripherals::take().unwrap().pins.gpio1).unwrap();
                    indicator.set_high().unwrap();

                    sleep(Duration::from_secs(10 * 60));

                    esp_sleep_disable_wakeup_source(
                        esp_idf_sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_ALL,
                    );
                    esp_deep_sleep_start();
                }
            }
            // wait for battery to raise above some level
            Brownout => {}
        }
    }

    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take().unwrap();
    let pins = peripherals.pins;
    let _ = pins.gpio1; // used for other things

    let nvs_partition = EspDefaultNvsPartition::take()?;

    // initializing connectiosn to things
    // battery monitor
    let mut batt_mon = BatteryMonitor::new(peripherals.adc1, pins.gpio0)?;
    // i2c bus (shared with display, and sensors)
    // NOTE: slow baudrate (for lightning sensor compat) will make the display slow
    let i2c_driver = i2c::I2cDriver::new(
        peripherals.i2c0,
        pins.gpio4,
        pins.gpio5,
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
    // IRQ is on pin 6
    // TODO

    let sysloop = EspSystemEventLoop::take()?;
    let (wifi_status_send, wifi_status_recv) =
        smol::channel::unbounded::<wifictl::WifiStatusUpdate>();
    let _wifi_event_sub = sysloop.subscribe(move |event: &WifiEvent| {
        // println!("Wifi event: {event:?}");
        match event {
            WifiEvent::StaDisconnected => wifi_status_send
                .try_send(wifictl::WifiStatusUpdate::Disconnected)
                .expect("Impossible! (unbounded queue is full???? (or main thread dead))"),
            _ => {}
        }
    })?;

    write!(display, "Starting wifi...")?;
    let mut wifi = Wifi::new(
        EspWifi::new(
            peripherals.modem,
            sysloop.clone(),
            Some(nvs_partition.clone()),
        )?,
        sysloop.clone(),
    );
    wifi.start()?;

    // -- NVS station information initialization --
    // performed here since it uses random numbers, and `getrandom` on the esp32
    // requires wifi / bluetooth to be enabled for true random numbers
    // - performed before the wifi is connected, because in the future this might store info on known networks
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

    let chosen = wifi.scan()?.expect("Could not find a network");
    let connected_ssid = chosen.0.ssid.clone().to_string();

    wifi.connect(chosen)?;

    let ip_info = wifi.inner().sta_netif().get_ip_info()?;
    println!("Wifi DHCP info {:?}", ip_info);

    // see docs -- if not present UdpSocket::bind fails
    wifictl::util::fix_networking()?;

    smol::block_on(async {
        println!("Async executor started");
        let _ = display.clear();
        write!(display, "connecting to server")?;
        // if this call fails, (or any other socket binds) try messing with the number in `wifictl::util::fix_networking`
        let sock = UdpSocket::bind("0.0.0.0:0").await?;
        let ips = resolve(conf::SERVER).await?;
        if ips.len() == 0 {
            bail!("Failed to resolve server address -- DNS lookup found nothing");
        } else if ips.len() > 1 {
            bail!("Faild to respolve server address -- multiple results ({ips:?})");
        }

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
            Channel {
                name: "battery".into(),
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

        let mut current_measurements = Observations::default();
        // todo: not hardcode
        let config = MeasureConfig::default();
        let mut timers = MeasureTimers::with_config(&config);
        loop {
            select_biased! {
                status = wifi_status_recv.recv().fuse() => {
                    match status? {
                        wifictl::WifiStatusUpdate::Disconnected => {
                            let _ = display.clear();
                            write!(display, "WIFI disconnected, please restart the device")?;
                            todo!("wifi disconnect not handled currently")
                        }
                    }
                }
                _ = timers.read_timer.next().fuse() => {
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

                    let to_send = PacketKind::Data(SomeData {
                        per_channel: {
                            let mut map = HashMap::<ChannelID, ChannelData>::new();
                            let mut set = |id, val| mappings.map.get(&ChannelName::from(id)).map(|uuid| map.insert(*uuid, val));
                            set("temperature", ChannelData::Float(current_measurements.temperature));
                            set("humidity", ChannelData::Float(current_measurements.humidity));
                            set("pressure", ChannelData::Float(current_measurements.pressure));
                            set("battery", ChannelData::Float(current_measurements.battery));
                            map
                        }
                    });

                    let to_send_raw = rmp_serde::to_vec_named(&to_send)?;

                    mvp_send(&sock, &to_send_raw, &mut uid_gen).await;
                }
                _ = timers.display_update.next().fuse() => {
                    let _ = display.clear();
                    write!(
                        display,
                        "ON:{}\nIP:{}\nBAT:{:.1} TEMP:{:.0}F",
                        connected_ssid,
                        ip_info.ip,
                        current_measurements.battery,
                        current_measurements.temperature * 1.8 + 32.0
                    )?;
                }
            }
        }

        //Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[derive(Debug, Clone)]
pub struct MeasureConfig {
    read_interval: Duration,
    display_interval: Duration,
}

impl Default for MeasureConfig {
    fn default() -> Self {
        //TODO not hardcode values
        Self {
            read_interval: Duration::from_secs(30),
            display_interval: Duration::from_secs(15),
        }
    }
}

#[derive(Debug)]
pub struct MeasureTimers {
    pub read_timer: Timer,
    pub display_update: Timer,
}

impl MeasureTimers {
    pub fn with_config(cfg: &MeasureConfig) -> Self {
        Self {
            read_timer: Timer::interval(cfg.read_interval),
            display_update: Timer::interval(cfg.display_interval),
        }
    }

    pub fn update_new_cfg(&mut self, new_cfg: &MeasureConfig) {
        *self = Self::with_config(new_cfg)
    }
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
