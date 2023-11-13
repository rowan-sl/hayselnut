use anyhow::Result;

use crate::core::args::{ArgsParser, Cmd};

pub async fn delegate(args: ArgsParser) -> Result<()> {
    match args.cmd {
        Cmd::DB3 {} => tokio::task::spawn_blocking(move || crate::tsdb3::main()).await?,
        Cmd::DB { args } => {
            tokio::task::spawn_blocking(move || crate::tsdb3::cmd::main(args)).await?
        }
        // handled earlier
        Cmd::Kill { .. } | Cmd::Run { .. } => unreachable!(),
    }
}
