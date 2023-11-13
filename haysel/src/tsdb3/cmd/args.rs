use std::path::PathBuf;

use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub struct DBCmdArgs {
    #[command(subcommand)]
    pub cmd: DBSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum DBSubcommand {
    /// Allocate a new file for a database (zero-initialized with given size)
    Alloc {
        #[arg(help = "path to create the new database at")]
        path: PathBuf,
        #[command(flatten)]
        size: AllocSize,
        #[arg(long, short, help = "overwrite any existing file at this location")]
        force: bool,
    },
    /// Initialize a new database in `file`
    /// This currently has no way to check if a database already exists, so if it does IT WILL BE OVERWRITTEN
    Init {
        #[arg(help = "path of the database to initialize (must already exist)")]
        path: PathBuf,
    },
    /// Report the current usage of the database (amnt, percentage)
    Usage {
        #[arg(help = "path of the database to investigate")]
        path: PathBuf,
    },
}

#[derive(Args, Debug)]
#[group(id = "size", required = true, multiple = true)]
pub struct AllocSize {
    #[arg(
        short,
        default_value = "0",
        help = "size to pre-allocate [in GB] (can be combined with -m, -k, or -b)"
    )]
    pub gigabytes: u64,
    #[arg(
        short,
        default_value = "0",
        help = "size to pre-allocate [in MB] (can be combined with -g, -k, or -b)"
    )]
    pub megabytes: u64,
    #[arg(
        short,
        default_value = "0",
        help = "size to pre-allocate [in KB] (can be combined with -g, -m, or -b)"
    )]
    pub kilobytes: u64,
    #[arg(
        short,
        default_value = "0",
        help = "size to pre-allocate [in B] (can be combined with -g, -m or -k)"
    )]
    pub bytes: u64,
}
