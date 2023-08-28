use anyhow::Result;

use crate::args::{ArgsParser, Cmd, RunArgs};

pub mod db2;
pub mod infodump;

pub async fn delegate(args: ArgsParser) -> Delegation {
    match args.cmd {
        Cmd::Infodump { file } => Delegation::SubcommandRan(infodump::main(file).await),
        Cmd::DB2 {
            init_overwrite,
            file,
        } => Delegation::SubcommandRan(db2::main(init_overwrite, file).await),
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
