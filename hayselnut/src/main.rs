#![feature(sync_unsafe_cell)]
#![feature(io_error_more)]

#[macro_use]
extern crate log;

pub mod conf;
pub mod error;
pub mod lightning;
pub mod periph;
pub mod store;
pub mod wifictl;

use std::{
    cell::SyncUnsafeCell,
    collections::HashMap,
    io,
    thread::sleep,
    time::{Duration, Instant},
};

use embedded_svc::wifi::Wifi as _;
use esp_idf_hal::{
    gpio::PinDriver,
    i2c::{self, I2cDriver},
    peripherals::Peripherals,
    reset::ResetReason,
    units::FromValueType,
};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    log::EspLogger,
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
        shared::SendError,
        UidGenerator,
    },
};

use store::{StationStore, StationStoreCached};

use crate::{
    error::{ErrExt as _, _panic_hwerr},
    periph::{battery::BatteryMonitor, bme280::PeriphBME280, Peripheral, SensorPeripheral},
    wifictl::Wifi,
};

const NO_WIFI_RETRY_INTERVAL: Duration = Duration::from_secs(60);

fn main() {
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

    EspLogger::initialize_default();
    EspLogger.set_target_level("*", log::LevelFilter::Trace);
    trace!("[logger] logging level trace");
    debug!("[logger] logging level debug");
    info!("[logger] logging level info");
    warn!("[logger] logging level warn");
    error!("[logger] logging level error");
    println!("done");

    info!("starting");

    let peripherals = Peripherals::take().unwrap();
    let pins = peripherals.pins;
    let _ = pins.gpio1; // used for other things (error flag)

    // -- initializing core peripherals --
    // battery monitor
    let mut batt_mon = BatteryMonitor::new(peripherals.adc1, pins.gpio0)
        .unwrap_hwerr("failed to initialize battery monitor");
    // i2c bus (shared with display, and sensors)
    // NOTE: slow baudrate (for lightning sensor compat) will make the display slow
    let i2c_driver = i2c::I2cDriver::new(
        peripherals.i2c0,
        pins.gpio4,
        pins.gpio5,
        &i2c::config::Config::new().baudrate(100.kHz().into()),
    )
    .unwrap_hwerr("failed to initialize battery monitor");
    let i2c_bus = shared_bus::new_std!(I2cDriver = i2c_driver)
        .expect("[sanity check] can only create one shared bus instance");

    // -- initializing peripherals --
    // temp/humidity/pressure
    // if this call ever fails (no error, just waiting forever) check the connection with the sensor
    let mut bme280 = PeriphBME280::new(i2c_bus.acquire_i2c());

    // lightning sensor
    // IRQ is on pin 6
    // TODO
    // -- end peripheral initialization --

    let sysloop = EspSystemEventLoop::take().unwrap_hwerr("could not take system event loop");
    let (wifi_status_send, wifi_status_recv) =
        smol::channel::unbounded::<wifictl::WifiStatusUpdate>();
    let _wifi_event_sub = sysloop
        .subscribe(move |event: &WifiEvent| {
            // println!("Wifi event: {event:?}");
            match event {
                WifiEvent::StaDisconnected => wifi_status_send
                    .try_send(wifictl::WifiStatusUpdate::Disconnected)
                    .expect("Impossible! (unbounded queue is full???? (or main thread dead))"),
                _ => {}
            }
        })
        .unwrap_hwerr("could not subscribe to system envent loop");

    let nvs_partition = EspDefaultNvsPartition::take()
        .unwrap_hwerr("could not take default nonvolatile storage partition");
    let mut wifi = Wifi::new(
        EspWifi::new(
            peripherals.modem,
            sysloop.clone(),
            Some(nvs_partition.clone()),
        )
        .unwrap_hwerr("failed to create ESP WIFI instance"),
        sysloop.clone(),
    );
    // this doesn't need to be retried, have never seen this fail
    if let Err(e) = wifi.start() {
        match e {
            wifictl::WifiError::Esp(e) => {
                Err(e).unwrap_hwerr("failed to start ESP WIFI instance")
            }
            wifictl::WifiError::TimedOut => {
                error::_panic_hwerr(error::EmptyError, "Failed to start WIFI: Timed out");
            }
        }
    }


    // -- NVS station information initialization --
    // performed here since it uses random numbers, and `getrandom` on the esp32
    // requires wifi / bluetooth to be enabled for true random numbers
    // - performed before the wifi is connected, because in the future this might store info on known networks
    let store: Box<dyn StationStore> = Box::new(StationStoreCached::init(nvs_partition.clone()).unwrap_hwerr("error accessing NVS"));
    info!("Loaded station info: {:#?}", store.read());

    // -- here is code that needs to go before the error-retry loops --
    // see [fix_networking] docs -- if not present UdpSocket::bind fails
    wifictl::util::fix_networking().unwrap_hwerr("call to wifictl::util::fix_networking failed");

    // init some persistant information for use later
    let mut uid_gen = UidGenerator::new();
    let mut channels = vec![Channel {
        name: "battery".into(),
        value: ChannelValue::Float,
        ty: ChannelType::Periodic,
    }];
    // add channels from sensors
    channels.extend_from_slice(&bme280.channels());
    // setup timers for when to measure things
    // todo: not hardcode
    let config = MeasureConfig::default();
    let mut timers = MeasureTimers::with_config(&config);

    // -- start the executor (before retry loops) --
    smol::block_on(async {
        println!();
        // not an error, just make the message stand out
        error!("----     ----    async executor + main loop started    ----    ----\n");

        'retry_wifi: loop {
            {
                info!("Connecting to WIFI");
                // clear the disconnect queue
                while let Ok(wifictl::WifiStatusUpdate::Disconnected) = wifi_status_recv.try_recv()
                {
                }
                // -- connecting to wifi --
                let before = Instant::now();
                // -- finding a known network --
                let chosen = loop {
                    if let Some(chosen) =
                        wifi.scan().unwrap_hwerr("error scaning for WIFI networks")
                    {
                        break chosen;
                    } else {
                        error!("scan returned no available networks, retrying in {NO_WIFI_RETRY_INTERVAL:?}");
                        sleep(NO_WIFI_RETRY_INTERVAL);
                    }
                };
                'retry: {
                    let max = 5;
                    for i in 0..max {
                        if let Err(e) = wifi.connect(chosen.clone()) {
                            match e {
                                wifictl::WifiError::Esp(e) => {
                                    Err(e).unwrap_hwerr("failed to connect to WIFI")
                                }
                                wifictl::WifiError::TimedOut => {
                                    warn!(
                                        "Failed to connect to WIFI (attempt {i}/{max} timed out), retrying"
                                    );
                                    sleep(Duration::from_millis(1000));
                                    if let Ok(wifictl::WifiStatusUpdate::Disconnected) =
                                        wifi_status_recv.try_recv()
                                    {
                                    } else {
                                        warn!("unexpectedly did not receive a disconnect message after a failed connect attempt");
                                    };
                                }
                            }
                        } else {
                            break 'retry;
                        }
                    }
                    error::_panic_hwerr(error::EmptyError, "Failed to connect to WIFI: Timed out");
                }
                info!("Connected to WIFI in {:?}", before.elapsed());
                let ip_info = wifi.inner()
                    .sta_netif()
                    .get_ip_info()
                    .unwrap_hwerr("failed to get network information (may be caused by TOCTOU error if the wifi network disconnected at the wrong moment)");
                info!("Wifi DHCP info {:?}", ip_info);
            }

            // if this call fails, (or any other socket binds) try messing with the number in `wifictl::util::fix_networking`
            let sock = UdpSocket::bind("0.0.0.0:0")
                .await
                .unwrap_hwerr("call to UdpSocket bind failed [unkwnown cause]");

            'retry_server: loop {
                // re-do DNS in here (not the wifi loop) just in case it changing causing this
                let ips = resolve(conf::SERVER)
                    .await
                    .unwrap_hwerr("call to DNS resolve failed [unknown cause]");
                if ips.len() == 0 {
                    error!("failed to resolve server address (DNS lookup for {:?} returned no IP results)", conf::SERVER);
                    if !wifi
                        .inner()
                        .is_connected()
                        .unwrap_hwerr("error checking wifi status")
                    {
                        warn!("[cause of error]: wifi was not connected");
                        continue 'retry_wifi;
                    }
                } else if ips.len() > 1 {
                    _panic_hwerr(
                        error::EmptyError,
                        "Faild to respolve server address -- multiple results ({ips:?})",
                    );
                }

                // works even if wifi is not connected. only operations that actually use the network will break.
                sock.connect(ips[0])
                    .await
                    .unwrap_hwerr("call to sock.connect failed [unknown cause]");
                info!(
                    "linking socket to server at: {:?} (this does not mean that the server is actually running here)",
                    sock.peer_addr().unwrap_hwerr("socket.peer_addr failed [unknown cause]")
                );

                // macro to handle network errors (needs to be a macro so it can use local loop labels)
                // on scucess, returns the resulting value.
                // on error, if handleable it deals with it, if not it bails
                macro_rules! handle_netres {
                    ($res:expr) => {
                        match $res {
                            Ok(v) => v,
                            Err(SendError::IOError(e)) if e.kind() == io::ErrorKind::HostUnreachable => {
                                error!("I/O Error: host unreachable (the network is down)");
                                error!("attempting to reconnect WIFI");
                                continue 'retry_wifi;
                            }
                            Err(e @ SendError::IOError(..)) => {
                                _panic_hwerr(e, "I/O Error went unhandled (not known to be caused by a fixable problem)");
                            },
                            Err(SendError::TimedOut) => {
                                error!("initial communication with the server failed (connection timed out -- is it running?)");
                                error!("trying to connect with the server [again]");
                                continue 'retry_server;
                            }
                        }
                    };
                }

                macro_rules! send {
                    ($packet:expr) => {
                        handle_netres!(
                            mvp_send(
                                &sock,
                                &rmp_serde::to_vec_named(&$packet)
                                    .unwrap_hwerr("failed to serialize data to send"),
                                &mut uid_gen,
                            )
                            .await
                        )
                    };
                }

                macro_rules! recv {
                    ($kind:path) => {
                        match rmp_serde::from_slice(&loop {
                            match handle_netres!(mvp_recv(&sock, &mut uid_gen).await) {
                                Some(packet) => break packet,
                                None => {
                                    warn!("receive timed out (got empty response, retrying in 5s)");
                                    //TODO: have some sort of failure mode that does not loop forever
                                    Timer::after(Duration::from_secs(5)).await;
                                }
                            }
                        }) {
                            Ok($kind(map)) => map,
                            Ok(other) => {
                                error!("The server is misbehaving! (expected {}, received {other:?})", stringify!($kind));
                                error!("this would be caused by broken server code, or a malicious actor.");
                                error!("we cant do much about this, exiting");
                                //FIXME: mabey try again in a while?
                                panic!()
                            }
                            Err(e) => {
                                error!("The server is misbehaving! (failed to deserialize a packet)");
                                error!("The error is: {e:?}");
                                error!("this would be caused by broken server code, or a malicious actor.");
                                error!("we cant do much about this, exiting");
                                // see above
                                panic!();
                            }
                        }
                    }
                }

                // send init packet
                info!("sending init info");
                send!(&PacketKind::Connect(squirrel::api::OnConnect {
                    station_id: store.read().station_uuid,
                    channels: channels.clone(),
                }));
                info!("server is up");
                info!("requesting channel mappings");
                let mappings = recv!(PacketKind::ChannelMappings);
                info!("received channel mappings: {mappings:#?}");

                loop {
                    select_biased! {
                        status = wifi_status_recv.recv().fuse() => {
                            match status.unwrap_hwerr("wifi status queue broken") {
                                wifictl::WifiStatusUpdate::Disconnected => {
                                    error!("notified of WIFI disconnect, reconnecting...");
                                    continue 'retry_wifi;
                                }
                            }
                        }
                        _ = timers.read_timer.next().fuse() => {
                            info!("reading sensors and sending");
                            let map_fn = |id: &str| *mappings.map.get(&ChannelName::from(id)).expect("could not find mapping for id {id:?}");
                            let bme_readings = match bme280.read(&map_fn) {
                                Some(v) => v,
                                None => {
                                    warn!("BME280 sensor peripheral error: {:?}, fixing...", bme280.err());
                                    bme280.fix();
                                    Default::default()
                                }
                            };

                            let battery_voltage = batt_mon.read().unwrap_hwerr("failed to read battery voltage");

                            send!(PacketKind::Data(SomeData {
                                per_channel: {
                                    let mut map = HashMap::<ChannelID, ChannelData>::new();
                                    let mut set = |id, val| mappings.map.get(&ChannelName::from(id)).map(|uuid| map.insert(*uuid, val));
                                    set("battery", ChannelData::Float(battery_voltage));
                                    bme_readings.into_iter().for_each(|(k, v)| { map.insert(k, v); });
                                    map
                                }
                            }));
                        }
                    }
                }
            }
        }
    });
}

#[derive(Debug, Clone)]
pub struct MeasureConfig {
    read_interval: Duration,
}

impl Default for MeasureConfig {
    fn default() -> Self {
        //TODO not hardcode values
        Self {
            read_interval: Duration::from_secs(30),
        }
    }
}

#[derive(Debug)]
pub struct MeasureTimers {
    pub read_timer: Timer,
}

impl MeasureTimers {
    pub fn with_config(cfg: &MeasureConfig) -> Self {
        Self {
            read_timer: Timer::interval(cfg.read_interval),
        }
    }

    pub fn update_new_cfg(&mut self, new_cfg: &MeasureConfig) {
        *self = Self::with_config(new_cfg)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Observations {
    /// battery voltage (volts)
    battery: f32,
}
