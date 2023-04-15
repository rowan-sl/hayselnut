use std::mem::{align_of, size_of};

use static_assertions::const_assert;
use uuid::Uuid;
use zerocopy::{AsBytes, FromBytes};

use super::packet::{extract_packet_type, PacketHeader, PACKET_TYPE_FRAME, UDP_MAX_SIZE};

pub const FRAME_BUF_SIZE: usize =
    4 + (((UDP_MAX_SIZE - (size_of::<PacketHeader>() + 4) - 4) + 7) / 8 - 1) * 8;

/// IMPORTANT FOR DECODERS: VALID FRAME PACKETS MAY BE SMALLER THAN THE FRAME STRUCT!! (if buf is not full, and to_bytes_compact is used)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, FromBytes, AsBytes)]
#[repr(C)]
pub struct Frame {
    pub header: PacketHeader,
    // (current, max).
    // max = number of fragments
    // current = where the current fragment is
    //
    // this is not used to uiquely identify this Frame, `id` does that.
    // this IS used to reconstruct fragmented packets.
    pub frag_idx: [u8; 2],
    // number of bytes in `buf`.
    // in range 0..buf.len()
    pub len: u16,
    // data past [len] in buf is probably going to be garbaje, depending on how the receiver is implemented
    // since valid frame packets can have a size less than the size of frame!!
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
        if frame.header.hash != frame.calc_hash() {
            None?
        }
        Some(frame)
    }

    /// Generates `ceil(buf / FRAME_BUF_SIZE)` `Frames` that contain the complete data of `buf`
    ///
    /// each frame will have a unique ID, and a unique frag_idx which indicates its position
    /// if the data is fragmented
    pub fn for_data<F: FnMut() -> (Uuid, Uuid)>(buf: &[u8], mut id_gen: F) -> Vec<Frame> {
        let chunks = buf.chunks(FRAME_BUF_SIZE).collect::<Vec<_>>();
        let num_chunks = chunks.len();
        assert!(num_chunks < 255, "Number of chunks must be less than 255");
        let mut frames = Vec::new();
        for (frag, chunk) in chunks.into_iter().enumerate() {
            let mut arr_chunk = [0u8; FRAME_BUF_SIZE];
            arr_chunk.copy_from_slice(chunk);

            let (id, next_id) = id_gen();

            let mut frame = Frame {
                header: PacketHeader {
                    id: id.into_bytes(),
                    next_id: next_id.into_bytes(),
                    hash: 0,
                    packet_type: PACKET_TYPE_FRAME,
                    _pad: 0,
                },
                frag_idx: [frag as u8, num_chunks as u8],
                len: chunk.len() as u16,
                buf: arr_chunk,
            };
            frame.header.hash = frame.calc_hash();
            frames.push(frame);
        }
        frames
    }

    // calculate the hash of this Frame.
    // calculated with the `hash` field set to zero.
    fn calc_hash(&self) -> u64 {
        let mut frame = *self;
        frame.header.hash = 0;

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

const_assert!(size_of::<Frame>() == UDP_MAX_SIZE - 4); // Frame is 504 bytes -- since it needs to be aligned to 8 and size is a multiple of align
const_assert!(align_of::<Frame>() == 8);
