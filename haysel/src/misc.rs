pub mod flag;
pub mod paths;
pub mod take;

use std::net::SocketAddr;

use anyhow::Result;
use trust_dns_resolver::config as resolveconf;
use trust_dns_resolver::TokioAsyncResolver;

pub use flag::Flag;
pub use paths::RecordsPath;
pub use take::Take;

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
