use anyhow::Result;

use crate::{
    args::{ArgsParser, Cmd, RunArgs},
    tsdb2::alloc::store::disk::DiskMode,
};

pub mod db2;
pub mod infodump;

pub async fn delegate(args: ArgsParser) -> Delegation {
    match args.cmd {
        Cmd::Infodump {
            file,
            is_blockdevice,
        } => {
            let mode = if is_blockdevice {
                DiskMode::BlockDevice
            } else {
                DiskMode::Dynamic
            };
            Delegation::SubcommandRan(infodump::main(file, mode).await)
        }
        Cmd::DB2 {
            init_overwrite,
            file,
            is_blockdevice,
        } => {
            let mode = if is_blockdevice {
                DiskMode::BlockDevice
            } else {
                DiskMode::Dynamic
            };
            Delegation::SubcommandRan(db2::main(init_overwrite, file, mode).await)
        }
        Cmd::Run { args: run_args } => Delegation::RunMain(run_args),
    }
}

#[derive(Debug)]
pub enum Delegation {
    /// a subcommand was selected an ran, no action is required
    /// the result of this is passed as the first argument
    SubcommandRan(Result<()>),
    /// the caller of this function should take these arguments, and run the main task
    RunMain(RunArgs),
}
