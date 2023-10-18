//! Utilities / smaller functions *core* to the function of haysel (unlike that in src/misc)

use std::net::SocketAddr;

use anyhow::Result;
use trust_dns_resolver::config as resolveconf;
use trust_dns_resolver::TokioAsyncResolver;

pub mod args;
pub mod autosave;
pub mod commands;
pub mod config;
pub mod log;
pub mod rt;
pub mod shutdown;

pub use autosave::AutosaveDispatch;
pub use log::{init_logging_no_file, init_logging_with_file};

/// it is necessary to bind the server to the real external ip address,
/// or risk confusing issues (forgot what, but it's bad)
pub async fn lookup_server_ip(url: String, port: u16) -> Result<Vec<SocketAddr>> {
    info!(
        "Performing DNS lookup of server's extranal IP (url={})",
        url
    );
    let resolver = TokioAsyncResolver::tokio(
        resolveconf::ResolverConfig::default(),
        resolveconf::ResolverOpts::default(),
    );
    let addrs = resolver
        .lookup_ip(url)
        .await?
        .into_iter()
        .map(|addr| {
            debug!("Resolved IP {addr}");
            SocketAddr::new(addr, port)
        })
        .collect::<Vec<_>>();
    Ok::<_, anyhow::Error>(addrs)
}
