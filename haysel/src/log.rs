use anyhow::Result;
use tracing::metadata::LevelFilter;
use tracing_log::LogTracer;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

pub fn init_logging() -> Result<()> {
    tracing::subscriber::set_global_default(
        FmtSubscriber::builder()
            .with_env_filter(
                EnvFilter::builder()
                    .with_default_directive(LevelFilter::TRACE.into())
                    .from_env()
                    .expect("Invalid logging config"),
            )
            .pretty()
            .finish(),
    )?;
    LogTracer::init()?;
    Ok(())
}
