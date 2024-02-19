#![feature(sync_unsafe_cell)]
#![feature(io_error_more)]

#[macro_use]
extern crate log;

pub mod conf;
pub mod error;
pub mod flag;
pub mod lightning;
pub mod periph;
pub mod store;
pub mod wifictl;

use std::{
    cell::SyncUnsafeCell,
    collections::HashMap,
    io,
    str::FromStr,
    time::{Duration, Instant},
};

use embedded_svc::wifi;
use esp_idf_hal::{
    adc::{self, AdcDriver},
    i2c,
    peripherals::Peripherals,
    reset::ResetReason,
    units::FromValueType,
};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    log::EspLogger,
    nvs::EspDefaultNvsPartition,
    timer::EspTaskTimerService,
    wifi::{AsyncWifi, EspWifi},
};
use esp_idf_sys::{self as _, esp_app_desc, esp_deep_sleep_start, esp_sleep_disable_wakeup_source}; // allways should be imported if `binstart` feature is enabled.
use futures::{select_biased, FutureExt};
use serde::{Deserialize, Serialize};
use tokio::{
    net::{lookup_host as resolve, UdpSocket},
    time::{interval, Interval},
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
};

const NO_WIFI_RETRY_INTERVAL: Duration = Duration::from_secs(60);
/// metadata on the build (passed using `build.rs`)
mod build {
    pub const GIT_REV: &str = env!("BUILD_GIT_REV");
    pub const DATETIME_PRETTY: &str = env!("BUILD_DATETIME_PRETTY");
    pub const DATETIME: &str = env!("BUILD_DATETIME");
}

esp_app_desc!();

fn main() {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_sys::link_patches();

    println!(
        "\n\n {bar} Hayselnut Weather Station {bar} \n\n",
        bar = std::iter::repeat("-~").take(15).collect::<String>()
    );
    println!(
        "hayselnut build metadata:\n    git revision: {}\n    built on: {}",
        build::GIT_REV,
        build::DATETIME_PRETTY
    );

    // handles the reset reason (e.g. does something special if reseting from a panic)
    on_reset();

    println!("setting up logging");
    EspLogger::initialize_default();
    EspLogger
        .set_target_level("*", log::LevelFilter::Trace)
        .unwrap_hwerr("failed to set esp log level");
    // test log levels
    trace!("[logger] logging level trace");
    debug!("[logger] logging level debug");
    info!("[logger] logging level info");
    warn!("[logger] logging level warn");
    error!("[logger] logging level error");

    info!("starting");

    let peripherals = Peripherals::take().unwrap();
    let pins = peripherals.pins;
    // used in wifi and station store
    let sysloop = EspSystemEventLoop::take().unwrap_hwerr("could not take system event loop");
    let nvs_partition = EspDefaultNvsPartition::take()
        .unwrap_hwerr("could not take default nonvolatile storage partition");
    let timer = EspTaskTimerService::new().unwrap_hwerr("failed to create task timer service");

    // -- initializing core peripherals --
    // ADC1
    let mut adc1 = AdcDriver::new(
        peripherals.adc1,
        &adc::AdcConfig::new()
            .resolution(adc::config::Resolution::Resolution12Bit)
            .calibration(true),
    )
    .unwrap_hwerr("failed to initialize ADC");
    // battery monitor
    let mut batt_mon =
        BatteryMonitor::new(pins.gpio0).unwrap_hwerr("failed to initialize battery monitor");
    // i2c bus (shared with display, and sensors)
    // NOTE: slow baudrate (for lightning sensor compat) will make the display slow
    let i2c_driver = i2c::I2cDriver::new(
        peripherals.i2c0,
        pins.gpio1, //sda
        pins.gpio3, //scl
        &i2c::config::Config::new().baudrate(100.kHz().into()),
    )
    .unwrap_hwerr("failed to initialize battery monitor");
    //let i2c_bus = shared_bus::new_std!(I2cDriver = i2c_driver)
    //    .expect("[sanity check] can only create one shared bus instance");
    let i2c_bus = i2c_driver;

    // -- initializing peripherals --
    // lightning
    // CS=7 MISO(MI)=6 MOSI(DA)=5 CLK=10 IRQ=4
    //TODO: intergrate this with the peripheral system for error handleing
    // let (mut lightning_sensor, lightning_setup_interrupt) = {
    //     let (cs, miso, mosi, clk, irq) = (
    //         PinDriver::output(pins.gpio7).unwrap(),
    //         PinDriver::input(pins.gpio6).unwrap(),
    //         PinDriver::output(pins.gpio5).unwrap(),
    //         PinDriver::output(pins.gpio10).unwrap(),
    //         pins.gpio4,
    //     );
    //     let mut sensor = LightningSensor::new(cs, clk, mosi, miso).unwrap();
    //     sensor.perform_initial_configuration().unwrap();
    //     sensor.configure_defaults().unwrap();
    //     let mode = &lightning::repr::SensorLocation::Outdoor;
    //     info!("the lightning sensor currently set to {mode:?} location mode");
    //     sensor.configure_sensor_placing(mode).unwrap();
    //     // DO SOMETHING GOD DAMNIT (remove literally every single anti-noise and false positive protection)
    //     warn!("the lightning sensor currently has all of its noise rejection disabled for testing. this will yield many false positives");
    //     // sensor
    //     //     .configure_ignore_disturbances(&lightning::repr::MaskDisturberEvent(false))
    //     //     .unwrap();
    //     // sensor
    //     //     .configure_noise_floor_threshold(&lightning::repr::NoiseFloorThreshold(0))
    //     //     .unwrap();
    //     // sensor
    //     //     .configure_minimum_lightning_threshold(&lightning::repr::MinimumLightningThreshold::One)
    //     //     .unwrap();
    //     // sensor
    //     //     .configure_signal_verification_threshold(&lightning::repr::SignalVerificationThreshold(
    //     //         0,
    //     //     ))
    //     //     .unwrap();
    //     // sensor
    //     //     .configure_spike_rejection(&lightning::repr::SpikeRejectionSetting(0))
    //     //     .unwrap();
    //     let setup_interrupt = move |flag: Flag| unsafe {
    //         PinDriver::input(irq)
    //             .unwrap()
    //             .subscribe(move || {
    //                 // Saftey (this itself is safe, but its executing in an ISR context)
    //                 // this is only doing atomic memory accesses, which should be fine :shrug:
    //                 flag.signal();
    //             })
    //             .unwrap_hwerr("failed to set interrupt");
    //     };
    //     (sensor, setup_interrupt)
    // };

    // wind{speed,direction}, rainfall quantity
    // {
    //     let speed = pins.gpio6;
    //     let direction = pins.gpio4;
    //     let rainfall = pins.gpio7;
    //
    //     let speed_flag = unsafe {
    //         let flag = Flag::new();
    //         let flag2 = flag.clone();
    //         let driver = Box::leak(Box::new(PinDriver::input(speed).unwrap()));
    //         driver.set_pull(esp_idf_hal::gpio::Pull::Down).unwrap();
    //         driver
    //             .set_interrupt_type(esp_idf_hal::gpio::InterruptType::PosEdge)
    //             .unwrap();
    //         driver
    //             .subscribe(move || flag2.signal())
    //             .unwrap_hwerr("failed to set interrupt");
    //         driver.enable_interrupt().unwrap();
    //         flag
    //     };
    //
    //     let mut direction_reader = {
    //         let mut driver = AdcChannelDriver::<'_, _, adc::Atten11dB<_>>::new(direction)
    //             .unwrap_hwerr("failed to init adc channel");
    //         move |adc1: &mut AdcDriver<'_, adc::ADC1>| {
    //             adc1.read(&mut driver).unwrap_hwerr("failed to read adc")
    //         }
    //     };
    //
    //     let rainfall_flag = unsafe {
    //         let flag = Flag::new();
    //         let flag2 = flag.clone();
    //         let driver = Box::leak(Box::new(PinDriver::input(rainfall).unwrap()));
    //         driver.set_pull(esp_idf_hal::gpio::Pull::Down).unwrap();
    //         driver
    //             .set_interrupt_type(esp_idf_hal::gpio::InterruptType::PosEdge)
    //             .unwrap();
    //         driver
    //             .subscribe(move || flag2.signal())
    //             .unwrap_hwerr("failed to set interrupt");
    //         driver.enable_interrupt().unwrap();
    //         flag
    //     };
    //
    //     wifictl::util::fix_networking().unwrap();
    //     tokio::runtime::Builder::new_current_thread()
    //         .enable_all()
    //         .build()
    //         .unwrap()
    //         .block_on(async {
    //             println!("running");
    //             loop {
    //                 println!("{}", direction_reader(&mut adc1));
    //                 sleep(Duration::from_millis(100)).await;
    //                 // rainfall_flag.clone().await;
    //                 // rainfall_flag.reset();
    //                 // println!("tick");
    //             }
    //         });
    // }
    //
    // temp/humidity/pressure
    // if this call ever fails (no error, just waiting forever) check the connection with the sensor
    warn!("connecting to BME sensor - if it is disconnected this will hang here");
    let mut bme280 = PeriphBME280::new(i2c_bus);

    // see [fix_networking] docs -- if not present UdpSocket::bind fails
    // - also needed for tokio
    wifictl::util::fix_networking().unwrap_hwerr("call to wifictl::util::fix_networking failed");

    // -- wifi initialization --
    let mut wifi = AsyncWifi::wrap(
        EspWifi::new(
            peripherals.modem,
            sysloop.clone(),
            Some(nvs_partition.clone()),
        )
        .unwrap_hwerr("failed to initialize wifi"),
        sysloop.clone(),
        timer,
    )
    .unwrap_hwerr("failed to initialize async wifi");

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap_hwerr("failed to initialize async runtime (tokio)")
        .block_on(async move {
            // -- the rest of wifi initialization --
            wifi.set_configuration(&wifi::Configuration::Client(wifi::ClientConfiguration::default()))
                    .unwrap_hwerr("failed to set wifi configuration");
            wifi.start().await.unwrap_hwerr("failed to start wifi");

            // -- NVS station information initialization --
            // performed here since it uses random numbers, and `getrandom` on the esp32
            // requires wifi / bluetooth to be enabled for true random numbers
            // - performed before the wifi is connected, because in the future this might store info on known networks
            let store: Box<dyn StationStore> = Box::new(
                StationStoreCached::init(nvs_partition.clone()).unwrap_hwerr("error accessing NVS"),
            );
            info!("Loaded station info: {:#?}", store.read());

            // -- here is code that needs to go before the error-retry loops --

            println!();
            // not an error, just make the message stand out
            error!("----     ----    main loop starting    ----    ----\n");

            //info!("setting lightning sensor interrupt");
            //let lightning_flag = Flag::new();
            //lightning_setup_interrupt(lightning_flag.clone());

            // -- init some persistant information for use later --
            let mut uid_gen = UidGenerator::new();
            let mut channels = vec![
                Channel {
                    name: "battery".into(),
                    value: ChannelValue::Float,
                    ty: ChannelType::Periodic,
                },
                Channel {
                    name: "lightning".into(),
                    value: ChannelValue::Event(HashMap::from([
                        ("distance_estimation_changed".into(), vec![]),
                        ("disturbance_detected".into(), vec![]),
                        ("noise_level_too_high".into(), vec![]),
                        ("invalid_interrupt".into(), vec![]),
                        ("lightning".into(), vec!["distance".into()]),
                    ])),
                    ty: ChannelType::Triggered,
                },
            ];
            // add channels from sensors
            channels.extend_from_slice(&bme280.channels());
            // setup timers for when to measure things
            // todo: not hardcode
            let config = MeasureConfig::default();
            let mut timers = MeasureTimers::with_config(&config);

            // if this call fails, (or any other socket binds) try messing with the number in `wifictl::util::fix_networking`
            let sock = UdpSocket::bind("0.0.0.0:0")
                .await
                .unwrap_hwerr("call to UdpSocket bind failed [unkwnown cause]");

            'retry_wifi: loop {
                connect_wifi(&mut wifi).await;

                'retry_server: loop {
                    // re-do DNS in here (not the wifi loop) just in case it changing causing this
                    let ips = resolve(conf::SERVER)
                        .await
                        .unwrap_hwerr("call to DNS resolve failed [unknown cause]")
                        .collect::<Vec<_>>();
                    if ips.len() == 0 {
                        error!("failed to resolve server address (DNS lookup for {:?} returned no IP results)", conf::SERVER);
                        if !wifi
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
                                        tokio::time::sleep(Duration::from_secs(5)).await;
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
                        station_build_rev: build::GIT_REV.to_string(),
                        station_build_date: build::DATETIME.to_string(),
                        channels: channels.clone(),
                    }));
                    info!("server is up");
                    info!("requesting channel mappings");
                    let mappings = recv!(PacketKind::ChannelMappings);
                    info!("received channel mappings: {mappings:#?}");

                    loop {
                        select_biased! {
                            res = wifi.wifi_wait(|wifi| wifi.is_up(), None).fuse() => {
                                res.unwrap_hwerr("failed to check wifi status");
                                info!("WIFI disconnected");
                                continue 'retry_wifi
                            }
                            // _ = lightning_flag.clone().fuse() => {
                            //     lightning_flag.reset();
                            //     Timer::interval(lightning::IRQ_TRIGGER_TO_READY_DELAY).await;
                            //     let event = lightning_sensor.get_latest_event_and_reset().unwrap();
                            //     println!("received a lightning event! {:#?}", event);
                            //     send!(PacketKind::Data(SomeData {
                            //         per_channel: {
                            //             HashMap::<ChannelID, ChannelData>::from([(
                            //                 *mappings.map.get(&ChannelName::from("lightning")).unwrap(),
                            //                 ChannelData::Event {
                            //                     sub: match event {
                            //                         lightning::Event::DistanceEstimationChanged => "distance_estimation_changed",
                            //                         lightning::Event::DisturbanceDetected => "disturbance_detected",
                            //                         lightning::Event::NoiseLevelTooHigh => "noise_level_too_high",
                            //                         lightning::Event::InvalidInt(..) => "invalid_interrupt",
                            //                         lightning::Event::Lightning { .. } => "lightning",
                            //                     }.to_string(),
                            //                     data: match event {
                            //                         lightning::Event::Lightning { distance } => HashMap::from([(
                            //                             "distance".to_string(),
                            //                             match distance {
                            //                                 lightning::repr::DistanceEstimate::OutOfRange => f32::INFINITY,
                            //                                 lightning::repr::DistanceEstimate::InRange(d) => d as f32,
                            //                                 lightning::repr::DistanceEstimate::Overhead => 0f32
                            //                             }
                            //                         )]),
                            //                         _ => HashMap::new()
                            //                     }
                            //                 }
                            //             )])
                            //         }
                            //     }))
                            // }
                            _ = timers.read_timer.tick().fuse() => {
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

                                let battery_voltage = std::iter::repeat_with(|| batt_mon.read(&mut adc1).unwrap_hwerr("failed to read battery voltage"))
                                    .take(50)
                                    .sum::<f32>() / 50.0;

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

// called once on reset to handle any special reset reasons (e.g. panic)
fn on_reset() {
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

                std::thread::sleep(Duration::from_secs(10 * 60));

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

async fn connect_wifi(wifi: &mut AsyncWifi<EspWifi<'_>>) {
    info!("Connecting to WIFI");
    assert!(wifi
        .is_started()
        .unwrap_hwerr("Failed to query wifi status"));
    assert!(!wifi
        .is_connected()
        .unwrap_hwerr("Failed to query wifi status"));
    assert!(!wifi.is_up().unwrap_hwerr("Failed to query wifi status"));
    // // clear the disconnect queue
    // while let Ok(wifictl::WifiStatusUpdate::Disconnected) = wifi_status_recv.try_recv() {}
    // // -- connecting to wifi --
    let before = Instant::now();
    // -- finding a known network --
    let chosen = loop {
        if let Some(chosen) = {
            let aps = wifi
                .scan()
                .await
                .unwrap_hwerr("error scaning for WIFI networks");
            let mut useable = wifictl::filter_networks(aps, conf::INCLUDE_OPEN_NETWORKS);
            useable.sort_by_key(|x| x.0.signal_strength);
            (!useable.is_empty()).then(|| useable.remove(0))
        } {
            break chosen;
        } else {
            error!("scan returned no available networks, retrying in {NO_WIFI_RETRY_INTERVAL:?}");
            std::thread::sleep(NO_WIFI_RETRY_INTERVAL);
        }
    };
    info!("Connecting to: {}", chosen.0.ssid);
    wifi.set_configuration(&wifi::Configuration::Client(wifi::ClientConfiguration {
        ssid: chosen.0.ssid,
        password: <_ as FromStr>::from_str(chosen.1.unwrap_or_default()).unwrap(),
        channel: Some(chosen.0.channel),
        ..Default::default()
    }))
    .unwrap_hwerr("failed to set wifi config");
    for i in 1..=5 {
        info!("Connecting (attempt {i} / 5)");
        if let Err(e) = wifi.connect().await {
            warn!("Attempt {i}/5 failed");
            if i == 5 {
                _panic_hwerr(e, "Failed to connect to wifi (attempt 5 errored)");
            }
        } else {
            break;
        }
    }
    info!("waiting for association");
    wifi.ip_wait_while(|wifi| wifi.is_up().map(|x| !x), None)
        .await
        .unwrap_hwerr("ip_wait_while failed");
    assert!(wifi.is_up().unwrap_hwerr("Failed to query wifi status"));
    info!("Connected to wifi in {:?}", before.elapsed());
    let ip_info = wifi.wifi()
        .sta_netif()
        .get_ip_info()
        .unwrap_hwerr("failed to get DHCP info - may be caused by TOCTOU error if the wifi disconnected immedietally after connecting");
    info!("WIFI DHCP info: {ip_info:?}");
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
    pub read_timer: Interval,
}

impl MeasureTimers {
    pub fn with_config(cfg: &MeasureConfig) -> Self {
        Self {
            read_timer: {
                let mut i = interval(cfg.read_interval);
                i.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                i
            },
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
