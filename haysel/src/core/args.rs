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
        #[arg(
            long,
            help = "Use the disk storage's block device mode. required (and exclusively used for) using block devices as databases"
        )]
        is_blockdevice: bool,
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
        #[arg(long, short, help = "database file(s)")]
        files: Vec<PathBuf>,
        #[arg(
            long,
            help = "Use the disk storage's block device mode. required (and exclusively used for) using block devices as databases"
        )]
        is_blockdevice: bool,
    },
    /// kill the currently running haysel daemon (if there is one)
    Kill {
        #[arg(long, short, help = "config filepath")]
        config: PathBuf,
    },
}

#[derive(Args, Debug)]
pub struct RunArgs {
    #[arg(long, short, help = "config filepath")]
    pub config: PathBuf,
    #[arg(
        long,
        help = "Delete the current database contents (if they exist) and re-initialize (must be passed on the first run if the database contains anything (e.g. is a physical disk) to prevent first-time init occurring accidentally)"
    )]
    pub overwrite_reinit: bool,
    #[arg(
        long,
        help = "Start and then immedietally fork, running in the background untill killed with `haysel kill`"
    )]
    pub daemonize: bool,
    #[arg(
        long,
        help = "do not write a PID file, do not check for a PID file, do not trap ctrl+c, imply --overwrite-reinit (incompatable with --daemonize)"
    )]
    pub no_safeguards: bool,
}
