use std::mem::{align_of, size_of};

use num_enum::{IntoPrimitive, TryFromPrimitive};
use static_assertions::{const_assert, const_assert_eq};
use zerocopy::{AsBytes, FromBytes};

use super::packet::{extract_packet_type, PACKET_TYPE_CONTROLL, UDP_MAX_SIZE};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, FromBytes, AsBytes)]
#[repr(C)]
pub struct CmdPacket {
    // first 3 fields - same as `Frame` struct
    pub id: u64,
    pub hash: u64,
    pub packet_type: u32,
    pub _pad: u32,
    // required align: u64,
    pub data: CmdPacketData,
}

impl CmdPacket {
    pub fn new(id: u64, cmd: Cmd) -> Self {
        let mut pack = Self {
            id,
            hash: 0,
            packet_type: PACKET_TYPE_CONTROLL,
            _pad: 0,
            data: CmdPacketData::new(cmd),
        };
        pack.hash = pack.calc_hash();
        pack
    }

    /// Validates `hash`, `packet_type`, and checks that `data` contains a valid `Cmd`
    pub fn from_buf_validated(buf: &[u8]) -> Option<Self> {
        if extract_packet_type(buf)? != PACKET_TYPE_CONTROLL {
            None?
        }
        let cmd = CmdPacket::read_from_prefix(buf)?;
        if cmd.hash != cmd.calc_hash() {
            None?
        }
        let _ = cmd.data.extract_cmd()?;
        Some(cmd)
    }

    // calculate the hash of this `CmdPacket`
    // calculated with the `hash` field set to zero.
    fn calc_hash(&self) -> u64 {
        let mut packet = *self;
        packet.hash = 0;

        let mut buf = [0u8; 8];
        blake3::Hasher::new()
            .update(packet.as_bytes())
            .finalize_xof()
            .fill(&mut buf);
        u64::from_be_bytes(buf)
    }
}

const_assert!(size_of::<CmdPacket>() < UDP_MAX_SIZE);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, FromBytes, AsBytes)]
#[repr(C, align(8))]
pub struct CmdPacketData {
    pub cmd: u8, // discriminant of `Cmd`, use `Into`/`TryFrom`
    pub _pad: [u8; 7],
    /// optional data field - used by some commands that require sending a message id
    pub data_id: u64,
}

impl CmdPacketData {
    pub fn new(c: Cmd) -> Self {
        CmdPacketData {
            cmd: c as _,
            _pad: Default::default(),
            data_id: 0,
        }
    }

    pub fn extract_cmd(&self) -> Option<Cmd> {
        Cmd::try_from(self.cmd).ok()
    }

    pub fn with_data_id(mut self, id: u64) -> Self {
        self.data_id = id;
        self
    }
}

const_assert_eq!(align_of::<CmdPacketData>(), align_of::<u64>());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, IntoPrimitive, TryFromPrimitive)]
#[repr(u8)]
pub enum Cmd {
    // ---- initiate a transaction ----
    // the reason why all transactions are client-initiated is that
    // the client cannot be assumed to be ready to communicate, or even
    // connected to the internet at all at all times.
    //
    // it is however assumed that the client will not (expect becuase of
    // circumstances beyond its controll) disconnect during a transaction.
    /// (c -> s) request a transaction between the station and the server, with the
    /// intent being (initially) data transfer from the station to the server.
    ///
    /// transaction initiated with client as TX, server as RX.
    RequestTransaction = 1,
    /// (c -> s) inform the server that it may initiate a transaction at this time,
    /// with the intent being that the server can make any pending requests of the station.
    ///
    /// transaction initiated with server as TX, client as RX.
    AllowTransaction = 2,
    /// (s -> c) inform the station that no transaction is needed at this time (response to `AllowTransaction`)
    TransactionUnnecessary = 3,
    /// (s -> c) confirm and begin a transaction. response to `AllowTransaction` or `RequestTransaction`
    ConfirmTransaction = 4,

    // ---- perform a transaction / confirm a command ----
    // - a transaction consists of one or more `Frame` packets being sent, then the confirming `Received` packet.
    //   - !!! the `Received` packet should be sent to confirm commands, too!
    // - during a given transaction, **ONE** of the client / server may be transmitting/receiving at a time. (at the Frame level)
    // - only one (optionally fragmented) `Frame` sequence may be transmitted
    //     - this means one `Frame` if not fragmented
    //     - or the number of `Frame`s listed in the first `Frame` if fragmented.
    /// (rx -> tx) confirm that all packets *up to and including* `data_id` have been received.
    /// this packet informs the other side that it can stop sending packets with ids *less than or equal to*
    /// the id sent with this.
    ///
    /// **this should be sent every time a packet is received (ALL `Frame`, and all `Cmd` except for `Cmd::Received`)**
    ///
    /// **NOTE:** when a packet with an `id` that is more that one greater than the previously received ID,
    /// something must have gone wrong, since the next packet should only be sent after the last one had been received and confiremed.
    /// - Conversely, if an `id` LESS than the last received `id` is received, it should be ignored.
    ///      this would occur normally, if a packet was sent multiple times and one
    ///      repitition took too long to arrive, and can safely be ignored.
    /// - this logic applies to ALL packet repsonses, not just `Received`
    Received = 5,

    // ---- continue or terminate a transaction ----
    // after ONE frame is sent, one of the following packets should be sent *by the current transmitter)
    // after it is confirmed, the listed action should be taken.
    //
    // TBD - conflict of intrest
    /// (tx -> rx) Transmitter requests to send another [set of] `Frame` to the current receiver.
    /// this should be used in the case that the `tx` has another packet immedietally available, and
    /// the receiver is not expecting to respond immedietally.
    ContinueTxAgain = 6,

    /// (tx -> rx) Defer the decision of what to do to the non currently transmitting entity.
    /// [the receiver] can decide to do one of the following:
    /// - switch to being the transmitter (`ContinueSwitchRoles`)
    /// - allow the current transmitter to continue its role (`ContinueSameRoles`)
    ///
    /// after sending this, `tx` and `rx` temporarily switch roles (for one command to be sent).
    /// if `ContinueSwitchRoles` is chosen, this state is maintained. if `ContinueSameRoles`, this is reverted.
    ContinueDeferDecision = 7,

    /// (rx -> tx) When sent by the [temporary transmitter], it becomes the new transmitter on confirmation.
    ContinueSwitchRoles = 8,

    /// (rx -> tx) When sent by the [temporary transmitter], it reverts back to the reciever.
    /// [the receiver] can now decide to
    /// - send `ContinueTxAgain`, and transmit.
    /// - send `Terminate`, and end the transaction upon confirmation.
    ContinueSameRoles = 9,

    /// (tx -> rx) Terminate the connection upon confirmation.
    Terminate = 10,

    // ---- misc ----
    Hint,
}
