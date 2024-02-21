use num_enum::{IntoPrimitive, TryFromPrimitive};
use zerocopy::{AsBytes, FromBytes, FromZeroes};

use super::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, FromZeroes, FromBytes, AsBytes)]
#[repr(C, align(1))]
pub struct Cmd {
    pub head: super::Head,
    pub command: u8,
}

impl Cmd {
    pub fn kind(&self) -> Result<CmdKind> {
        CmdKind::try_from_primitive(self.command).map_err(|_| Error::BadCommand)
    }
}

// c  s       c     s       c     s       c        s
// Tx Confirm Frame Confirm Frame Confirm Complete Confirm
// c  s     c       (timeout)     c       s
// Rx Frame Confirm /* dropped */ Confirm Complete
//
// a note on repeat transmission:
//  - the repeat (from the client) should have the same UID as the original
//  - the response (from the server) should also be identical to the first response
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, IntoPrimitive, TryFromPrimitive, AsBytes)]
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
