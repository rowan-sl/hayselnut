//! network packet definition/serialization/deserialization

pub mod cmd;
pub mod frame;
pub mod uid;

use std::mem::size_of;

use num_enum::{IntoPrimitive, TryFromPrimitive};
use zerocopy::{AsBytes, FromBytes, FromZeroes};

pub use cmd::{Cmd, CmdKind};
pub use frame::Frame;
use uid::Uid;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("the buffer provided was too small (for writing data)")]
    TooSmallWrite,
    #[error("the buffer provided was too small (for reading headers)")]
    TooSmallReadHeader,
    #[error("the buffer provided was too small (for reading the expected length of data)")]
    TooSmallReadData,
    #[error("the packet type was invalid")]
    BadType,
    #[error("the command kind was invalid")]
    BadCommand,
}

pub type Result<T> = ::core::result::Result<T, Error>;

/// network packet types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, IntoPrimitive, TryFromPrimitive, AsBytes)]
#[repr(u8)]
pub enum Type {
    Frame = 0xAA,
    Command = 0xBB,
}

impl Type {
    pub fn extract(from: &[u8]) -> Result<Self> {
        from.get(8)
            .map(|&raw| Self::try_from_primitive(raw).map_err(|_| Error::BadType))
            .ok_or(Error::TooSmallReadHeader)
            .flatten()
    }
}

/// common packet header shared between `Cmd` and `Frame`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, FromZeroes, FromBytes, AsBytes)]
#[repr(C, align(1))]
pub struct Head {
    pub packet: Uid,
    pub responding_to: Uid,
    pub packet_ty: u8,
}

impl Head {
    pub fn ty(&self) -> Result<Type> {
        Type::try_from_primitive(self.packet_ty).map_err(|_| Error::BadType)
    }
}

/// reading a packet from a byte array
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Read<'src> {
    Cmd(&'src Cmd),
    Frame(&'src Frame),
}

impl<'src> Read<'src> {
    pub fn try_read(buf: &'src [u8]) -> Result<Self> {
        Ok(match Type::extract(buf)? {
            Type::Frame => Self::Frame(Frame::ref_buf(buf)?),
            Type::Command => Self::Cmd(Cmd::ref_from_prefix(buf).ok_or(Error::TooSmallReadHeader)?),
        })
    }

    pub fn head(&self) -> &Head {
        match self {
            Self::Cmd(cmd) => &cmd.head,
            Self::Frame(frame) => &frame.head,
        }
    }
}

/// builder API for writing a packet to a buffer
/// this is NOT a value that you should have "hanging around", unlike Read
pub struct Write<'dest> {
    inner: &'dest mut [u8],
}

impl<'dest> Write<'dest> {
    /// returns None if the buffer is not big enough for a header
    /// once you are done with `Write`, simply drop it and continue using the original `buf` reference
    pub fn new(buf: &'dest mut [u8]) -> Option<Self> {
        if buf.len() < size_of::<Head>() {
            None?
        }
        Some(Self { inner: buf })
    }

    /// set the ID of this packet (head.packet)
    pub fn with_packet(self, id: Uid) -> Self {
        Head::mut_from_prefix(self.inner).unwrap().packet = id;
        self
    }

    /// set the ID of the packet this is responding to (head.responding_to)
    pub fn with_responding_to(self, id: Uid) -> Self {
        Head::mut_from_prefix(self.inner).unwrap().responding_to = id;
        self
    }

    /// set packet_ty to Frame, write frame len and data
    pub fn write_frame_with<'other>(self, data: &'other [u8]) -> Result<Self> {
        let frame = Frame::new(self.inner, data)?;
        frame.head.packet_ty = Type::Frame as u8;
        Ok(self)
    }

    /// set packet_ty to Frame, write command
    pub fn write_cmd(self, kind: CmdKind) -> Result<Self> {
        let cmd = Cmd::mut_from_prefix(self.inner).ok_or(Error::TooSmallReadHeader)?;
        cmd.head.packet_ty = Type::Command as u8;
        cmd.command = kind as u8;
        Ok(self)
    }

    // get a reference to the portion of the inner buffer that has actual data in it
    pub fn portion_to_send(&self) -> &[u8] {
        match Type::extract(&self.inner).unwrap() {
            Type::Frame => {
                let f = Frame::ref_buf(&self.inner).unwrap();
                &self.inner[0..frame::SIZE_OF_HEADER + f.data().unwrap().len()]
            }
            Type::Command => &self.inner[0..size_of::<Cmd>()],
        }
    }
}
