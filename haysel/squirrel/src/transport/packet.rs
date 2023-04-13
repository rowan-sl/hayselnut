use zerocopy::FromBytes;

// https://stackoverflow.com/questions/1098897/what-is-the-largest-safe-udp-packet-size-on-the-internet#1099359
pub const UDP_MAX_SIZE: usize = 508;

// use all bits available to help accuracy
pub const PACKET_TYPE_FRAME: u32 = 0x12233445;
pub const PACKET_TYPE_CONTROLL: u32 = 0xABBCCDDE;

/// extracts the data where the packet type *SHOULD* be.
/// this does not validate the type in any fashion
///
/// returns None if buf does not contain enough bytes to possibly contain a packet type
pub fn extract_packet_type(buf: &[u8]) -> Option<u32> {
    #[derive(FromBytes)]
    #[repr(C)]
    pub struct FrameLike {
        pub id: u64,
        pub hash: u64,
        pub packet_type: u32,
    }

    let FrameLike {
        id: _,
        hash: _,
        packet_type,
    } = FrameLike::read_from_prefix(buf)?;
    Some(packet_type)
}
