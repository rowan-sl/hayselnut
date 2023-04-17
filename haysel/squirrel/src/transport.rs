use std::mem::size_of;

use num_enum::{IntoPrimitive, TryFromPrimitive};
use static_assertions::const_assert_eq;
use zerocopy::{AsBytes, FromBytes};

pub mod client;
pub mod shared;

// https://stackoverflow.com/questions/1098897/what-is-the-largest-safe-udp-packet-size-on-the-internet#1099359
pub const UDP_MAX_SIZE: usize = 508;

pub const PACKET_TYPE_FRAME: u8 = 0xAA;
pub const PACKET_TYPE_COMMAND: u8 = 0xBB;

pub fn extract_packet_type(bytes: &[u8]) -> Option<u8> {
    bytes.get(8).copied()
}

pub struct UidGenerator(u32);

impl UidGenerator {
    /// all generators start at zero, so only one long-lived one should be used to give out IDs
    pub fn new() -> Self {
        Self(0)
    }

    pub fn next(&mut self) -> u32 {
        self.0.wrapping_add(1);
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, FromBytes, AsBytes)]
#[repr(C)]
pub struct Frame {
    pub packet: u32,
    pub responding_to: u32,
    pub packet_ty: u8,
    pub _pad: u8,
    pub len: u16,
    pub data: [u8; FRAME_BUF_SIZE],
}

const FRAME_NON_DATA_SIZE: usize = 4 + 4 + 1 + 1 + 2;
const FRAME_BUF_SIZE: usize = UDP_MAX_SIZE - FRAME_NON_DATA_SIZE;

const_assert_eq!(size_of::<Frame>(), UDP_MAX_SIZE);

impl Frame {
    pub fn from_bytes_compact(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < FRAME_NON_DATA_SIZE || bytes.len() > size_of::<Self>() {
            None
        } else {
            let mut larger = [0u8; size_of::<Self>()];
            larger.copy_from_slice(bytes);
            Some(Self::read_from(larger.as_slice()).unwrap())
        }
    }

    pub fn as_bytes_compact(&self) -> &[u8] {
        &self.as_bytes()[0..FRAME_NON_DATA_SIZE + self.len as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, FromBytes, AsBytes)]
#[repr(C)]
pub struct Cmd {
    pub packet: u32,
    pub responding_to: u32,
    pub packet_ty: u8,
    pub command: u8,
    pub padding: [u8; 2],
}

// c  s       c     s       c     s       c        s
// Tx Confirm Frame Confirm Frame Confirm Complete Confirm
// c  s     c       (timeout)     c       s
// Rx Frame Confirm /* dropped */ Confirm Complete
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, IntoPrimitive, TryFromPrimitive)]
#[repr(u8)]
pub enum CmdKind {
    // c-> s inform transmit
    Tx,
    // c -> s inform receive
    Rx,
    // s ->/<- c inform received
    Confirm,
    // s ->/<- c inform complete
    Complete,
}

pub fn read_packet(buf: &[u8]) -> Option<Packet> {
    Some(match extract_packet_type(buf)? {
        PACKET_TYPE_FRAME => Packet::Frame(Frame::from_bytes_compact(buf)?),
        PACKET_TYPE_COMMAND => Packet::Cmd(Cmd::read_from(buf)?),
        _ => None?,
    })
}

pub enum Packet {
    Cmd(Cmd),
    Frame(Frame),
}

impl Packet {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Cmd(c) => c.as_bytes(),
            Self::Frame(f) => f.as_bytes_compact(),
        }
    }

    pub fn uid(&self) -> u32 {
        match self {
            Packet::Cmd(Cmd { packet, .. }) => *packet,
            Packet::Frame(Frame { packet, .. }) => *packet,
        }
    }

    pub fn responding_to(&self) -> u32 {
        match self {
            Packet::Cmd(Cmd { responding_to, .. }) => *responding_to,
            Packet::Frame(Frame { responding_to, .. }) => *responding_to,
        }
    }
}
