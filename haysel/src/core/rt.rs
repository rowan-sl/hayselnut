use std::process;

use anyhow::Result;
use clap::Parser;
use nix::{
    sys::signal::{kill, Signal},
    unistd::{daemon, Pid},
};
use tokio::runtime;

use crate::{
    core::{
        self,
        args::{self, ArgsParser, RunArgs},
        commands,
        shutdown::Shutdown,
    },
    misc,
};

pub fn stage0_delegate() -> Result<()> {
    let args = ArgsParser::parse();
    match args {
        ArgsParser {
            cmd: args::Cmd::Run { args },
        } => stage1_daemon(args),
        ArgsParser {
            cmd: args::Cmd::Kill { config },
        } => {
            let _guard = core::init_logging_no_file()?;
            info!("Reading configuration from {:?}", config);
            if !config.exists() {
                error!("Configuration file does not exist!");
                bail!("Configuration file does not exist!");
            }

            let cfg = {
                let buf = std::fs::read_to_string(&config)?;
                core::config::from_str(&buf)?
            };

            let run_dir = misc::RecordsPath::new(cfg.directory.run.clone());
            run_dir.ensure_exists_blocking()?;
            let pid_file = run_dir.path("daemon.lock");
            if !pid_file.try_exists()? {
                warn!("No known haysel daemon is currently running, exiting with no-op");
                return Ok(());
            }
            let pid_txt = std::fs::read_to_string(pid_file)?;
            let pid = pid_txt
                .parse::<u32>()
                .map_err(|e| anyhow!("failed to parse PID: {e:?}"))?;
            info!("Killing process {pid} - sending SIGINT (ctrl+c) to allow for gracefull shutdown\nThe PID file will only be removed when the server has exited");
            kill(Pid::from_raw(pid.try_into()?), Some(Signal::SIGINT))?;
            return Ok(());
        }
        #[allow(unreachable_code)]
        other => {
            let _guard = core::init_logging_no_file()?;
            let runtime = runtime::Builder::new_multi_thread().enable_all().build()?;
            return runtime.block_on(commands::delegate(other));
            drop(_guard);
        }
    }
}

pub fn stage1_daemon(mut args: RunArgs) -> Result<()> {
    if args.daemonize && args.no_safeguards {
        eprintln!("ERROR: --no-safeguards and --daemonize are incompatable!");
        bail!("Invalid Arguments");
    }
    println!("Reading configuration from {:?}", args.config);
    if !args.config.exists() {
        bail!("Configuration file does not exist!");
    }

    let cfg = {
        let buf = std::fs::read_to_string(&args.config)?;
        core::config::from_str(&buf)?
    };

    let records_dir = misc::RecordsPath::new(cfg.directory.data.clone());
    records_dir.ensure_exists_blocking()?;
    let run_dir = misc::RecordsPath::new(cfg.directory.run.clone());
    run_dir.ensure_exists_blocking()?;
    let log_dir = misc::RecordsPath::new(run_dir.path("log"));
    log_dir.ensure_exists_blocking()?;

    let pid_file = run_dir.path("daemon.lock");
    if pid_file.try_exists()? {
        if !args.no_safeguards {
            println!("ERROR: A server is already running, refusing to start!");
            println!("     | If this is incorrect, remove the `daemon.lock` file and try again");
            bail!("Server already started");
        }
    }

    if args.daemonize {
        println!("Forking!");
        daemon(true, true)?;
    }

    println!("Init logging");
    let guard = core::init_logging_with_file(run_dir.path("log"))?;
    if args.no_safeguards {
        warn!("Running in no-safeguard testing mode: this is NOT what you want for production use");
        warn!("--overwrite-reinit is implied by --no-safeguards: if this leads to loss of data, please consider the name of the argument and that you may have wanted to RTFM first");
        args.overwrite_reinit = true;
    }
    if args.no_safeguards && pid_file.try_exists()? {
        warn!("A PID file exists, continuing anyway (--no-safeguards mode)");
    }

    if !args.no_safeguards {
        debug!("Writing PID file {:?}", pid_file);
        let pid = process::id();
        std::fs::write(&pid_file, format!("{pid}").as_bytes())?;
    }
    let no_safeguards = args.no_safeguards;

    let result = std::panic::catch_unwind(move || stage2_async(cfg, args, records_dir, run_dir));

    if !no_safeguards {
        debug!("Deleting PID file {:?}", pid_file);
        std::fs::remove_file(&pid_file)?;
    }
    match result {
        Ok(inner) => {
            drop(guard);
            inner
        }
        Err(err) => {
            error!("Main thread panic! - stuff is likely messed up: {err:?}");
            bail!("Main thread panic!");
        }
    }
}

pub fn stage2_async(
    cfg: core::config::Config,
    args: RunArgs,
    records_dir: misc::RecordsPath,
    run_dir: misc::RecordsPath,
) -> Result<()> {
    debug!("Launching async runtime");
    let runtime = runtime::Builder::new_multi_thread().enable_all().build()?;
    let mut shutdown = Shutdown::new();
    runtime.block_on(async {
        let result = crate::async_main(cfg, args, &mut shutdown, records_dir, run_dir).await;
        if let Err(e) = &result {
            error!("Main task exited with error: {e:?}");
        }
        shutdown.trigger_shutdown();
        info!("shut down - waiting for tasks to stop");
        shutdown.wait_for_completion().await;
        result
    })
}
