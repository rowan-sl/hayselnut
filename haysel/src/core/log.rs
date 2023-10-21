use std::path::PathBuf;

use anyhow::Result;
use tracing::metadata::LevelFilter;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_log::LogTracer;
use tracing_subscriber::{fmt::Layer, prelude::*, registry, EnvFilter};

#[must_use]
#[allow(unused)]
pub struct Guard {
    inner0: WorkerGuard,
    inner1: Option<WorkerGuard>,
}

pub fn init_logging_no_file() -> Result<Guard> {
    println!("initializing stdout logging");
    let (stdout, guard1) = tracing_appender::non_blocking(std::io::stdout());
    let stdout_layer = Layer::new().with_writer(stdout).pretty();
    let global_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::TRACE.into())
        .from_env()
        .expect("Invalid logging config");
    registry().with(stdout_layer).with(global_filter).init();
    Ok(Guard {
        inner0: guard1,
        inner1: None,
    })
}
pub fn init_logging_with_file(log_dir: PathBuf) -> Result<Guard> {
    println!("initializing stdout+file logging");
    let appender = tracing_appender::rolling::hourly(log_dir, "haysel.log");
    let (logfile, guard0) = tracing_appender::non_blocking(appender);
    let logfile_layer = Layer::new().with_writer(logfile).compact();
    let (stdout, guard1) = tracing_appender::non_blocking(std::io::stdout());
    let stdout_layer = Layer::new().with_writer(stdout).pretty();
    let global_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::TRACE.into())
        .from_env()
        .expect("Invalid logging config");
    registry()
        .with(logfile_layer)
        .with(stdout_layer)
        .with(global_filter)
        .init();
    Ok(Guard {
        inner0: guard0,
        inner1: Some(guard1),
    })
}
