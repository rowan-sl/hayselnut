use zerocopy::FromBytes;

pub mod frame;

pub const PACKET_TYPE_FRAME: u32 = 1;

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

    let FrameLike { id: _, hash: _, packet_type } = FrameLike::read_from_prefix(buf)?;
    Some(packet_type)
}
