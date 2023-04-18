//! Async executor-indpendant networking compatability layer

#[cfg(feature = "smol")]
pub use smol::{
    io::Error,
    net::{SocketAddr, UdpSocket},
};
#[cfg(feature = "tokio")]
pub use std::net::SocketAddr;
#[cfg(feature = "tokio")]
pub use tokio::{io::Error, net::UdpSocket};
