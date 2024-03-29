use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::tsdb3::cmd::args::DBCmdArgs;

#[derive(Parser, Debug)]
pub struct ArgsParser {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// run
    Run {
        #[command(flatten)]
        args: RunArgs,
    },
    /// test program for TSDB v3
    DB3 {},
    /// TSDBv3 statistics, testing, maintinance, and more
    DB {
        #[command(flatten)]
        args: DBCmdArgs,
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
