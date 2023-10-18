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

use std::time::Duration;

use roundtable::{common::HDL_EXTERNAL, Bus};
use squirrel::api::station::{capabilities::KnownChannels, identity::KnownStations};
use tokio::net::UdpSocket;

mod core;
mod dispatch;
mod ipc;
mod misc;
mod registry;
pub mod tsdb2;
mod tsdbmock;

use core::{
    args::RunArgs,
    shutdown::{util::trap_ctrl_c, Shutdown},
};
use misc::RecordsPath;
use registry::JsonLoader;

use crate::{core::AutosaveDispatch, registry::Registry, tsdbmock::TStopDBus2Mock};

fn main() -> anyhow::Result<()> {
    core::rt::stage0_delegate()
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

    let addrs = core::lookup_server_ip(cfg.server.url, cfg.server.port).await?;
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

    // debug!("Loading database");
    // warn!("TSDB V2 is currently very unstable, bolth in format and in reliablility - things *will* go badly");
    // let db = {
    //     let store: Box<(dyn IsDynStorage<Error = raid::DynStorageError> + 'static)> = match cfg
    //         .database
    //         .storage
    //     {
    //         core::config::StorageMode::default => {
    //             let path = records_dir.path("data.tsdb2");
    //             Box::new(DynStorage(
    //                 DiskStore::new(&path, false, DiskMode::Dynamic).await?,
    //             ))
    //         }
    //         core::config::StorageMode::file => {
    //             if cfg.database.files.len() != 1 {
    //                 if cfg.database.files.is_empty() {
    //                     error!("Failed to create database storage: file mode is requested, but no file is provided");
    //                 } else {
    //                     error!("Failed to create database storage: file mode is requested, but more than one file is proveded (did you mean to enable RAID?)");
    //                 }
    //                 bail!("Failed to create database store");
    //             }
    //             let core::config::File { path, blockdevice } = cfg.database.files[0].clone();
    //             Box::new(DynStorage(
    //                 DiskStore::new(
    //                     &path,
    //                     false,
    //                     if blockdevice {
    //                         DiskMode::BlockDevice
    //                     } else {
    //                         DiskMode::Dynamic
    //                     },
    //                 )
    //                 .await?,
    //             ))
    //         }
    //         core::config::StorageMode::raid => {
    //             if cfg.database.files.is_empty() {
    //                 error!("Failed to create database storage: RAID mode is requested, but no file(s) are provided");
    //                 bail!("Failed to create database store");
    //             }
    //             if cfg.database.files.len() == 1 {
    //                 warn!("RAID mode is requested, but only one backing file is specified. this will cause unnecessary overhead, and it is recommended to switch to using single file mode");
    //             }
    //             let mut array = RaidArray::new();
    //             for core::config::File { path, blockdevice } in cfg.database.files {
    //                 let store = DiskStore::new(
    //                     &path,
    //                     false,
    //                     if blockdevice {
    //                         DiskMode::BlockDevice
    //                     } else {
    //                         DiskMode::Dynamic
    //                     },
    //                 )
    //                 .await?;
    //                 array.add_element(store).await?;
    //             }
    //             if args.overwrite_reinit {
    //                 warn!("Deleting and Re-Initializing the RAID storage");
    //                 array.wipe_all_your_data_away().await?;
    //             }
    //             debug!("Building array...");
    //             array.build().await?;
    //             array.print_info().await?;
    //             Box::new(DynStorage(array))
    //         }
    //     };
    //     let database = Database::new(store, args.overwrite_reinit).await?;
    //     let mut db_stop = tsdb2::bus::TStopDBus2::new(database).await;
    //     let (stations, channels) = bus
    //         .query_as(
    //             HDL_EXTERNAL,
    //             registry.clone(),
    //             registry::EV_REGISTRY_QUERY_ALL,
    //             (),
    //         )
    //         .await?;
    //     db_stop.ensure_exists(&(stations, channels)).await?;
    //     bus.spawn(db_stop)
    // };
    // info!("Database loaded");
    info!("Creating database *mock* - it will store recent data in-memory for testing");
    let db = {
        let mut db = TStopDBus2Mock::new().await;
        let (stations, channels) = bus
            .query_as(
                HDL_EXTERNAL,
                registry.clone(),
                registry::EV_REGISTRY_QUERY_ALL,
                (),
            )
            .await?;
        db.ensure_exists(&(stations, channels)).await?;
        bus.spawn(db)
    };

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
