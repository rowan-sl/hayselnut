#![allow(incomplete_features)]
// num_enum!!!! (again)
// #![allow(non_upper_case_globals)]
#![feature(generic_const_exprs)]
#![feature(pointer_is_aligned)]

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
pub mod tsdb3;

use core::{
    args::RunArgs,
    shutdown::{util::trap_ctrl_c, Shutdown},
};
use misc::RecordsPath;
use registry::JsonLoader;

use crate::{core::AutosaveDispatch, registry::Registry};

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

    debug!("Loading database [TSDB v3]");
    let db = {
        let path = match cfg.database.storage {
            core::config::StorageMode::default => records_dir.path("data.tsdb3"),
            core::config::StorageMode::file => {
                if cfg.database.files.is_empty() {
                    error!(
                        "storage mode: 'file' was selected, but no files were given for storage"
                    );
                    bail!("Invalid config");
                } else if cfg.database.files.len() == 1 {
                    cfg.database.files[0].path.clone()
                } else {
                    error!("storage mode: 'file' was selected, but multiple files were provided. TSDB v3 does not yet support this");
                    bail!("Invalid config");
                }
            }
        };
        let file = tokio::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .await?
            .into_std()
            .await;
        // Saftey: YOLO
        let mut db = unsafe { tsdb3::DB::new(file) }?;
        db.open();
        let mut stop = tsdb3::bus::TStopDBus3::new(db);
        let (stations, channels) = bus
            .query_as(
                HDL_EXTERNAL,
                registry.clone(),
                registry::EV_REGISTRY_QUERY_ALL,
                (),
            )
            .await?;
        stop.ensure_exists(&(stations, channels)).await;
        bus.spawn(stop)
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
