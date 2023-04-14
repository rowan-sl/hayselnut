use std::mem::{align_of, size_of};

use num_enum::{IntoPrimitive, TryFromPrimitive};
use static_assertions::{const_assert, const_assert_eq};
use uuid::Uuid;
use zerocopy::{AsBytes, FromBytes};

use super::packet::{extract_packet_type, PACKET_TYPE_CONTROLL, UDP_MAX_SIZE, PacketHeader};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, FromBytes, AsBytes)]
#[repr(C)]
pub struct CmdPacket {
    pub header: PacketHeader,
    // required align: u64,
    pub data: CmdPacketData,
}

impl CmdPacket {
    pub fn new(id: Uuid, next_id: Uuid, cmd: Cmd) -> Self {
        let mut pack = Self {
            header: PacketHeader {
                id: id.into_bytes(),
                next_id: next_id.into_bytes(),
                hash: 0,
                packet_type: PACKET_TYPE_CONTROLL,
                _pad: 0,
            },
            data: CmdPacketData::new(cmd),
        };
        pack.header.hash = pack.calc_hash();
        pack
    }

    /// Validates `hash`, `packet_type`, and checks that `data` contains a valid `Cmd`
    pub fn from_buf_validated(buf: &[u8]) -> Option<Self> {
        if extract_packet_type(buf)? != PACKET_TYPE_CONTROLL {
            None?
        }
        let cmd = CmdPacket::read_from_prefix(buf)?;
        if cmd.header.hash != cmd.calc_hash() {
            None?
        }
        let _ = cmd.data.extract_cmd()?;
        Some(cmd)
    }

    // calculate the hash of this `CmdPacket`
    // calculated with the `hash` field set to zero.
    fn calc_hash(&self) -> u64 {
        let mut packet = *self;
        packet.header.hash = 0;

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
    pub data_id: [u8; 16],
}

impl CmdPacketData {
    pub fn new(c: Cmd) -> Self {
        CmdPacketData {
            cmd: c as _,
            _pad: Default::default(),
            data_id: [0; 16],
        }
    }

    pub fn extract_cmd(&self) -> Option<Cmd> {
        Cmd::try_from(self.cmd).ok()
    }

    pub fn with_data_id(mut self, id: Uuid) -> Self {
        self.data_id = id.into_bytes();
        self
    }
}

const_assert_eq!(align_of::<CmdPacketData>(), align_of::<u64>());

/// Note: some of this documentation refers to sequential IDs (and comparing IDs order) which
/// is no longer used. the logic behind it is basically the same though, just with `id` and `next_id`
///
/// design note: should never repeat packets (without a timeout / max retry limit), to avoid becomming a DDoS vector or similar issue
/// - the re-transmission time should increase over failed repititions (with a limit)
/// - for the server, if the retry limit is hit, the corresponding weather station should be declared "offline".
///        it should cease sending packets to it, untill it receives a ping or a transaction-init packet from the client.
///   - the server is permitted to ping every known station a few times on startup to determine its status.
/// - for the client, if the retry limit is hit it should cease normal transmissions, untill the server responds to a ping or pings it.
///
/// what is known as "pings" here ^ can be done using the pre-existing packets as follows:
/// - s -> c ping: `ServerHintTransaction` -> `AllowTransaction` -> `TransactionUnnecessary` || `ConfirmTransaction` (depending on what the server has to say, bolth are perfectly valid by existing definition)
/// - c -> s ping: `AllowTransaction` ->  `TransactionUnnecessary` || `ConfirmTransaction` (same as server ping, without the first step)
///   - the client should allways ping the server a few times when it is turned on (alternately, it can `RequestTransaction` and send some startup info packets to the server, but this is not really needed)
///
/// for all packets, when re-transmitting the packet's `id` and `next_id` should not change
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
    //
    // the ID of request packets should be repeated for repeat transmission.
    // any responses should use the request's `next_id` as their `id`, and have a random 
    // `next_id` like all other packets.
    //
    // when a transaction is started, it should use a completely new `id`, not
    // the `next_id` of the final packet of the last transmission.

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

    /// (s -> c) Request that the client initiate a server-tx transaction.
    /// It is perfectly valid for client implementations to completely igore these packets,
    /// as long as they preiodically send `AllowTransaction`.
    ///
    /// when not ignored, this is NOT responded to with `Received`, rather with `AllowTransaction`.
    ServerHintTransaction,
}
