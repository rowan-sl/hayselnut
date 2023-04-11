use std::mem::size_of;

use static_assertions::const_assert;
use zerocopy::{FromBytes, AsBytes};

use super::{PACKET_TYPE_FRAME, extract_packet_type};

// https://stackoverflow.com/questions/1098897/what-is-the-largest-safe-udp-packet-size-on-the-internet#1099359
pub const UDP_MAX_SIZE: usize = 508;
pub const FRAME_BUF_SIZE: usize = UDP_MAX_SIZE - 28;

// NOTE:this struct must have the same first few fields (id, hash, packet_type) IN THAT ORDER as the controll packet.
// this is so they can be differentiated before decoding the rest
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, FromBytes, AsBytes)]
#[repr(C)]
pub struct Frame {
    pub id: u64, 
    // for making the hash, hash is set to zero. then it is filled in with the appropreate value
    // hashing algorithm used 
    pub hash: u64,
    // type of packet. 
    pub packet_type: u32,
    // (current, max).
    // max = number of fragments
    // current = where the current fragment is
    pub frag_idx: [u8; 2],
    // number of bytes in `buf`.
    // in range 0..buf.len()
    pub len: u16,
    pub buf: [u8; FRAME_BUF_SIZE],
}

impl Frame {
    /// extracts a frame from `buf` (assumed to contain one frame).
    /// this will ignore any trailing data in `buf`
    /// 
    /// the `packet_type` and `hash` is validated.
    pub fn from_buf_validated(buf: &[u8]) -> Option<Frame> {
        if extract_packet_type(buf)? != PACKET_TYPE_FRAME {
            None?
        }
        let frame = Frame::read_from_prefix(buf)?;
        if frame.hash != frame.calc_hash() {
            None?
        }
        Some(frame)
    }

    /// Generates `ceil(buf / FRAME_BUF_SIZE)` `Frames` that contain the complete data of `buf`
    /// 
    /// each frame will have a unique ID, and a unique frag_idx which indicates its position
    /// if the data is fragmented
    pub fn for_data<F: FnMut() -> u64>(buf: &[u8], mut id: F) -> Vec<Frame> {
        let chunks = buf.chunks(FRAME_BUF_SIZE).collect::<Vec<_>>();
        let num_chunks = chunks.len();
        assert!(num_chunks < 255, "Number of chunks must be less than 255");
        let mut frames = Vec::new();
        for (frag, chunk) in chunks.into_iter().enumerate() {
            let mut arr_chunk= [0u8; FRAME_BUF_SIZE];
            arr_chunk.copy_from_slice(chunk);
            
            let mut frame = Frame {
                id: id(),
                hash: 0,
                packet_type: PACKET_TYPE_FRAME,
                frag_idx: [frag as u8, num_chunks as u8],
                len: chunk.len() as u16,
                buf: arr_chunk,
            };
            frame.hash = frame.calc_hash();
            frames.push(frame);
        }
        frames
    }

    // calculate the hash of this Frame.
    // calculated with the `hash` field set to zero.
    fn calc_hash(&self) -> u64 {
        let mut frame = *self;
        frame.hash = 0;

        let mut buf = [0u8; 8];
        blake3::Hasher::new()
            .update(frame.as_bytes())
            .finalize_xof()
            .fill(&mut buf);
        u64::from_be_bytes(buf)
    }

    /// Returns self::as_bytes, but excluding any unused bytes of `buf` (improve network usage)
    pub fn as_bytes_compact(&self) -> &[u8] {
        &self.as_bytes()[0..(size_of::<Frame>() - self.buf.len() + self.len as usize)]
    }
}

const_assert!(size_of::<Frame>() == UDP_MAX_SIZE - 4); // Frame is 504 bytes

