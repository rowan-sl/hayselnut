use anyhow::Result;

use crate::{
    args::{ArgsParser, Cmd},
    tsdb2::alloc::store::disk::DiskMode,
};

pub mod db2;
pub mod infodump;

pub async fn delegate(args: ArgsParser) -> Result<()> {
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
            infodump::main(file, mode).await
        }
        Cmd::DB2 {
            init_overwrite,
            files,
            is_blockdevice,
        } => {
            let mode = if is_blockdevice {
                DiskMode::BlockDevice
            } else {
                DiskMode::Dynamic
            };
            db2::main(init_overwrite, files, mode).await
        }
        // handled earlier
        Cmd::Kill { .. } | Cmd::Run { .. } => unreachable!(),
    }
}
