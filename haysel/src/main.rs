#![allow(incomplete_features)]
// num_enum!!!! (again)
#![allow(non_upper_case_globals)]
#![feature(trivial_bounds)]
#![feature(generic_const_exprs)]
#![feature(specialization)]
#![feature(is_sorted)]
#![feature(trait_upcasting)]
#![feature(downcast_unchecked)]

#[macro_use]
extern crate async_trait;
#[macro_use]
extern crate tracing;
#[macro_use]
extern crate anyhow;

use std::{process, time::Duration};

use clap::Parser;
use nix::{
    sys::signal::{kill, Signal},
    unistd::{daemon, Pid},
};
use roundtable::{common::HDL_EXTERNAL, msg, Bus};
use squirrel::api::station::{capabilities::KnownChannels, identity::KnownStations};
use tokio::{net::UdpSocket, runtime};

mod core;
mod dispatch;
mod ipc;
mod misc;
mod registry;
pub mod tsdb2;

use core::lookup_server_ip;
use core::{
    args::{self, ArgsParser, RunArgs},
    commands,
    shutdown::{util::trap_ctrl_c, Shutdown},
};
use misc::RecordsPath;
use registry::JsonLoader;
use tsdb2::{
    alloc::store::{disk::DiskStore, raid::ArrayR0 as RaidArray},
    Database,
};

use crate::{
    core::AutosaveDispatch,
    registry::Registry,
    tsdb2::alloc::store::{
        disk::DiskMode,
        raid::{self, DynStorage, IsDynStorage},
    },
};

fn main() -> anyhow::Result<()> {
    let args = ArgsParser::parse();
    core::init_logging()?;

    let run_args;
    match args {
        ArgsParser {
            cmd: args::Cmd::Run { mut args },
        } => {
            if args.no_safeguards {
                warn!("Running in no-safeguard testing mode: this is NOT what you want for production use");
                if args.daemonize {
                    error!("--no-safeguards and --daemonize are incompatable!");
                    bail!("Invalid Arguments");
                }
                warn!("--overwrite-reinit is implied by --no-safeguards: if this leads to loss of data, please consider the name of the argument and that you may have wanted to RTFM first");
                args.overwrite_reinit = true;
            }
            run_args = args
        }
        ArgsParser {
            cmd: args::Cmd::Kill { config },
        } => {
            info!("Reading configuration from {:?}", config);
            if !config.exists() {
                error!("Configuration file does not exist!");
                bail!("Configuration file does not exist!");
            }

            let cfg = {
                let buf = std::fs::read_to_string(&config)?;
                core::config::from_str(&buf)?
            };

            let run_dir = misc::RecordsPath::new(cfg.directory.run.clone());
            run_dir.ensure_exists_blocking()?;
            let pid_file = run_dir.path("daemon.lock");
            if !pid_file.try_exists()? {
                warn!("No known haysel daemon is currently running, exiting with no-op");
                return Ok(());
            }
            let pid_txt = std::fs::read_to_string(pid_file)?;
            let pid = pid_txt
                .parse::<u32>()
                .map_err(|e| anyhow!("failed to parse PID: {e:?}"))?;
            info!("Killing process {pid} - sending SIGINT (ctrl+c) to allow for gracefull shutdown\nThe PID file will only be removed when the server has exited");
            kill(Pid::from_raw(pid.try_into()?), Some(Signal::SIGINT))?;
            return Ok(());
        }
        other => {
            let runtime = runtime::Builder::new_multi_thread().enable_all().build()?;
            return runtime.block_on(commands::delegate(other));
        }
    };

    info!("Reading configuration from {:?}", run_args.config);
    if !run_args.config.exists() {
        error!("Configuration file does not exist!");
        bail!("Configuration file does not exist!");
    }

    let cfg = {
        let buf = std::fs::read_to_string(&run_args.config)?;
        core::config::from_str(&buf)?
    };

    let records_dir = misc::RecordsPath::new(cfg.directory.data.clone());
    records_dir.ensure_exists_blocking()?;
    let run_dir = misc::RecordsPath::new(cfg.directory.run.clone());
    run_dir.ensure_exists_blocking()?;

    let pid_file = run_dir.path("daemon.lock");
    if pid_file.try_exists()? {
        if run_args.no_safeguards {
            warn!("A PID file exists, continueing anyway (--no-safeguards mode)");
        } else {
            error!("A server is already running, refusing to start!");
            info!("If this is incorrect, remove the `daemon.lock` file and try again");
            bail!("Server already started");
        }
    }

    if run_args.daemonize {
        debug!("Forking!");
        daemon(true, true)?;
        info!("[daemon] - copying logs ")
    }
    if !run_args.no_safeguards {
        debug!("Writing PID file {:?}", pid_file);
        let pid = process::id();
        std::fs::write(&pid_file, format!("{pid}").as_bytes())?;
    }
    let no_safeguards = run_args.no_safeguards;

    debug!("Launching async runtime");
    let result = std::panic::catch_unwind(move || {
        let runtime = runtime::Builder::new_multi_thread().enable_all().build()?;
        let mut shutdown = Shutdown::new();
        runtime.block_on(async {
            let result = async_main(cfg, run_args, &mut shutdown, records_dir, run_dir).await;
            if let Err(e) = &result {
                error!("Main task exited with error: {e:?}");
            }
            shutdown.trigger_shutdown();
            info!("shut down - waiting for tasks to stop");
            shutdown.wait_for_completion().await;
            result
        })
    });
    if !no_safeguards {
        debug!("Deleting PID file {:?}", pid_file);
        std::fs::remove_file(&pid_file)?;
    }
    match result {
        Ok(inner) => inner,
        Err(err) => {
            error!("Main thread panic! - stuff is likely messed up: {err:?}");
            bail!("Main thread panic!");
        }
    }
}

async fn async_main(
    cfg: core::config::Config,
    args: RunArgs,
    shutdown: &mut Shutdown,
    records_dir: RecordsPath,
    run_dir: RecordsPath,
) -> anyhow::Result<()> {
    if !args.no_safeguards {
        // trap the ctrl+csignal, will only start listening later in the main loop
        trap_ctrl_c(shutdown.handle()).await;
    }

    let addrs = lookup_server_ip(cfg.server.url, cfg.server.port).await?;
    let bus = Bus::new().await;

    info!("Loading info for known stations");
    let stations =
        JsonLoader::<KnownStations>::open(records_dir.path("stations.json"), shutdown.handle())
            .await?;
    debug!("Loaded known stations:");

    info!("Loading known channels");
    let channels =
        JsonLoader::<KnownChannels>::open(records_dir.path("channels.json"), shutdown.handle())
            .await?;

    debug!(
        "Loaded known channels: {:#?}",
        channels
            .channels()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Vec<_>>()
    );

    for s in stations.stations() {
        // in the future, station info should be printed
        let info = stations.get_info(s).unwrap();
        debug!(
            "Known station {}\nsupports channels {:#?}",
            s, info.supports_channels
        );
    }

    let registry = bus.spawn(Registry::new(stations, channels));

    debug!("Loading database");
    warn!("TSDB V2 is currently very unstable, bolth in format and in reliablility - things *will* go badly");
    let db = {
        let store: Box<(dyn IsDynStorage<Error = raid::DynStorageError> + 'static)> = match cfg
            .database
            .storage
        {
            core::config::StorageMode::default => {
                let path = records_dir.path("data.tsdb2");
                Box::new(DynStorage(
                    DiskStore::new(&path, false, DiskMode::Dynamic).await?,
                ))
            }
            core::config::StorageMode::file => {
                if cfg.database.files.len() != 1 {
                    if cfg.database.files.is_empty() {
                        error!("Failed to create database storage: file mode is requested, but no file is provided");
                    } else {
                        error!("Failed to create database storage: file mode is requested, but more than one file is proveded (did you mean to enable RAID?)");
                    }
                    bail!("Failed to create database store");
                }
                let core::config::File { path, blockdevice } = cfg.database.files[0].clone();
                Box::new(DynStorage(
                    DiskStore::new(
                        &path,
                        false,
                        if blockdevice {
                            DiskMode::BlockDevice
                        } else {
                            DiskMode::Dynamic
                        },
                    )
                    .await?,
                ))
            }
            core::config::StorageMode::raid => {
                if cfg.database.files.is_empty() {
                    error!("Failed to create database storage: RAID mode is requested, but no file(s) are provided");
                    bail!("Failed to create database store");
                }
                if cfg.database.files.len() == 1 {
                    warn!("RAID mode is requested, but only one backing file is specified. this will cause unnecessary overhead, and it is recommended to switch to using single file mode");
                }
                let mut array = RaidArray::new();
                for core::config::File { path, blockdevice } in cfg.database.files {
                    let store = DiskStore::new(
                        &path,
                        false,
                        if blockdevice {
                            DiskMode::BlockDevice
                        } else {
                            DiskMode::Dynamic
                        },
                    )
                    .await?;
                    array.add_element(store).await?;
                }
                if args.overwrite_reinit {
                    warn!("Deleting and Re-Initializing the RAID storage");
                    array.wipe_all_your_data_away().await?;
                }
                debug!("Building array...");
                array.build().await?;
                array.print_info().await?;
                Box::new(DynStorage(array))
            }
        };
        let database = Database::new(store, args.overwrite_reinit).await?;
        let mut db_stop = tsdb2::bus::TStopDBus2::new(database).await;
        let (stations, channels) = bus
            .query_as(
                HDL_EXTERNAL,
                registry.clone(),
                registry::EV_REGISTRY_QUERY_ALL,
                (),
            )
            .await?;
        db_stop.ensure_exists(&(stations, channels)).await?;
        bus.spawn(db_stop)
    };
    info!("Database loaded");

    let ipc_path = run_dir.path("ipc.sock");
    debug!("Setting up IPC at {:?}", ipc_path);
    if tokio::fs::try_exists(&ipc_path).await? {
        tokio::fs::remove_file(&ipc_path).await?;
    }
    let ipc_stop = ipc::IPCNewConnections::new(ipc_path, registry.clone(), db.clone()).await?;
    bus.spawn(ipc_stop);
    info!("IPC configured");

    let autosave_interval = Duration::from_secs(30);
    info!("Autosaves will be triggered every {autosave_interval:?}");
    bus.spawn(AutosaveDispatch::new(autosave_interval));

    info!("running -- press ctrl+c to exit");
    let sock = UdpSocket::bind(addrs.as_slice()).await?;
    let max_transaction_time = Duration::from_secs(30);

    let dispatch_ctrl = dispatch::Controller::new(sock, max_transaction_time, registry.clone());
    bus.spawn(dispatch_ctrl);

    shutdown.handle().wait_for_shutdown().await;

    // bus.announce_as(HDL_EXTERNAL, msg::Target::Any, EV_SHUTDOWN, ())
    //     .await?;

    trace!("Shutting down - if a deadlock occurs here, it is likely because a shutdown handle was created in the main function and not dropped before this call");
    shutdown.wait_for_completion().await;

    Ok(())
}
