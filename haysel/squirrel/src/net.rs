//! Async executor-indpendant networking compatability layer

#[cfg(feature = "smol")]
pub use smol::net::UdpSocket;
#[cfg(feature = "tokio")]
pub use tokio::net::UdpSocket;
