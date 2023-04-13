use zerocopy::{FromBytes, AsBytes};

// https://stackoverflow.com/questions/1098897/what-is-the-largest-safe-udp-packet-size-on-the-internet#1099359
pub const UDP_MAX_SIZE: usize = 508;

// use all bits available to help accuracy
pub const PACKET_TYPE_FRAME: u32 = 0x12233445;
pub const PACKET_TYPE_CONTROLL: u32 = 0xABBCCDDE;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, FromBytes, AsBytes)]
#[repr(C)]
pub struct PacketHeader {
    // packet ids should be generated sequentially
    // if a packet with an id <= the id of the prev packet received, it should be ignored.
    pub id: u64,
    // for making the hash, hash is set to zero. then it is filled in with the appropreate value
    // hashing algorithm used
    pub hash: u64,
    // type of packet.
    pub packet_type: u32,
    pub _pad: u32,
}

/// extracts the data where the packet type *SHOULD* be.
/// this does not validate the type in any fashion
///
/// returns None if buf does not contain enough bytes to possibly contain a packet type
pub fn extract_packet_type(buf: &[u8]) -> Option<u32> {
    let PacketHeader {
        id: _,
        hash: _,
        packet_type,
        _pad: _,
    } = PacketHeader::read_from_prefix(buf)?;
    Some(packet_type)
}
