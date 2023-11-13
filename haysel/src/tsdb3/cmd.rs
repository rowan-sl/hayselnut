use std::fs::OpenOptions;

use anyhow::Result;

use self::args::{AllocSize, DBSubcommand};

use super::DB;

pub mod args;

pub fn main(args: args::DBCmdArgs) -> Result<()> {
    match args.cmd {
        DBSubcommand::Alloc {
            path,
            size:
                AllocSize {
                    gigabytes,
                    megabytes,
                    kilobytes,
                    bytes,
                },
            force,
        } => {
            if force {
                warn!("--force specified, {path:?} may be overwritten");
            }
            let file = OpenOptions::new()
                .write(true)
                .create(force)
                .truncate(force)
                .create_new(!force)
                .open(&path)?;
            let size = gigabytes
                .checked_mul(1000u64.pow(3))
                .expect("Arithmetic overflow calculating size")
                + megabytes
                    .checked_mul(1000u64.pow(2))
                    .expect("Arithmetic overflow calculating size")
                + kilobytes
                    .checked_mul(1000u64.pow(1))
                    .expect("Arithmetic overflow calculating size")
                + bytes;
            file.set_len(size)?;
            file.sync_all()?;
            info!("Set file size of {path:?} to {size:?} bytes");
        }
        DBSubcommand::Init { path } => {
            let file = OpenOptions::new().read(true).write(true).open(&path)?;
            warn!("Initializing new database in {path:?}...");
            unsafe { DB::new(file) }?.init();
            info!("Initialization complete");
        }
        DBSubcommand::Usage { path } => {
            let file = OpenOptions::new().read(true).write(true).open(&path)?;
            warn!("Opening database {path:?}...");
            let mut db = unsafe { DB::new(file) }?;
            info!("Opened database");
            let size = db.store.map.len() as u64;
            let access = db.store.access(false);
            let used = access.get_size_used();
            let percentage = used as f64 / size as f64;
            info!("Current usage of {path:?} is {used}B / {size}B ({percentage:.4}% full)");
        }
    }
    Ok(())
}
