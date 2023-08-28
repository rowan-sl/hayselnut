use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
pub struct ArgsParser {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// dump info about the database
    Infodump {
        #[arg(
            long,
            help = "if provided, will dump information about the database contained in <file>"
        )]
        file: Option<PathBuf>,
    },
    /// run
    Run {
        #[command(flatten)]
        args: RunArgs,
    },
    /// test program for TSDB v2
    DB2 {
        #[arg(
            long,
            help = "allow initializing a database using a file that contains data (may cause silent deletion of corrupted databases, so it is recommended to only use this when running the server for the first time)"
        )]
        init_overwrite: bool,
        #[arg(long, short, help = "database file")]
        file: PathBuf,
    },
}

#[derive(Args, Debug)]
pub struct RunArgs {
    #[arg(
        short,
        long,
        help = "directory for station/channel ID records and the database to be placed"
    )]
    pub data_dir: PathBuf,
    #[arg(short, long, help = "path of the unix socket for the servers IPC API")]
    pub ipc_sock: PathBuf,
    #[arg(short, long, help = "url of the server that this is to be run on")]
    pub url: String,
    #[arg(short, long, help = "port to run the server on")]
    pub port: u16,
    #[arg(
        long,
        help = "allow initiailizing a database using a file that contains data (this may cause silent deletion of corrupt databases, so it is recommended to only use this when running the server for the first time)"
    )]
    pub init_overwrite: bool,
    #[arg(
        long,
        help = "allow using an aternate database file, instead of the default under `data_dir`. this allows for use of *special* files like block devices..."
    )]
    pub alt_db: Option<PathBuf>,
}
