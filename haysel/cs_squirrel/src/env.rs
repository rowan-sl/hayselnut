// https://stackoverflow.com/questions/1098897/what-is-the-largest-safe-udp-packet-size-on-the-internet#1099359
pub const UDP_MAX_PACKET_SIZE: usize = 508;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Env {
    /// size of the largest valid packet for the underlying transport (e.g. UDP)
    pub max_packet_size: usize,
}
