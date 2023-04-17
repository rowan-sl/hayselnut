//! Async executor-indpendant networking compatability layer

#[cfg(feature = "smol")]
pub use smol::{io::Error, net::{UdpSocket, SocketAddr}};
#[cfg(feature = "tokio")]
pub use tokio::{net::UdpSocket, io::Error};
#[cfg(feature = "tokio")]
pub use std::net::SocketAddr;

