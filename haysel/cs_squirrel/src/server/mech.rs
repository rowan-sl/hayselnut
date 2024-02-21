//! State machine for handling a single server connection

use std::time::{Duration, Instant};

use crate::packet::{
    self,
    uid::{self, Uid},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    #[default]
    Resting,
    // - WE are receiving data, the CLIENT is TRANSMITTING (INIT WITH Tx) -
    RecvStart,
    Recv,
    RecvDone,
    // - WE are sending data, the CLIENT is RECEIVING (INIT WITH Rx) -
    SendStart,
    Send,
    SendDone,
}

#[derive(Debug)]
pub struct Config {
    /// greatest time allowed per transaction
    pub max_trans_time: Duration,
}

/// server connection state machine
#[derive(Debug)]
pub struct ConnState {
    state: State,
    conf: Config,
    responding_to: Uid,
    last_sent: Uid,
    gen: uid::Seq,
    /// time at which this transaction (recv or transmit) was started
    trans_time: Instant,
}

impl ConnState {
    pub fn new(conf: Config) -> Self {
        Self {
            state: State::default(),
            conf,
            responding_to: Uid::null(),
            last_sent: Uid::null(),
            gen: uid::Seq::new(),
            // garbage value, will be overwritten (state is Resting)
            trans_time: Instant::now(),
        }
    }

    pub fn process(&mut self, pkt: packet::Read<'_>) {
        if let State::Recv | State::Send = self.state {
            if self.trans_time.elapsed() > self.conf.max_trans_time {
                self.state = State::Resting;
                todo!("dispatch: timeout");
            }
        }
        match (self.state, pkt) {
            // valid initialization of a transaction (Rest/Done => Tx/Rx)
            (State::Resting | State::RecvDone | State::SendDone, packet::Read::Cmd(cmd))
                if cmd.kind() == Ok(packet::CmdKind::Tx)
                    || cmd.kind() == Ok(packet::CmdKind::Rx) => {}
            // not a valid initialization of a transaction
            (State::Resting, _) => {}

            // -- RECEIVING --
            // this is a REPETITION of the original packet.
            // respond in kind, with a repitition of the original ACK
            (State::RecvStart, packet::Read::Cmd(cmd))
                if cmd.kind() == Ok(packet::CmdKind::Tx)
                    && cmd.head.packet == self.responding_to => {}
            // not a valid continuation
            (State::RecvStart, packet::Read::Cmd(..)) => {}
            // Begin to receive data from the client
            (State::RecvStart, packet::Read::Frame(frame))
                if frame.head.responding_to == self.last_sent => {}
            // this is not the frame we are looking for
            (State::RecvStart, packet::Read::Frame(..)) => {}
            // done receiving
            (State::Recv, packet::Read::Cmd(cmd))
                if cmd.kind() == Ok(packet::CmdKind::Complete)
                    && cmd.head.responding_to == self.last_sent => {}
            // not a valid continuation
            (State::Recv, packet::Read::Cmd(..)) => {}
            // this is a repeat of already received data
            // do not add it to a buffer, but repeat identical ACK
            (State::Recv, packet::Read::Frame(frame))
                if frame.head.packet == self.responding_to => {}
            // receive more data
            (State::Recv, packet::Read::Frame(frame))
                if frame.head.responding_to == self.last_sent => {}
            // received out of order
            (State::Recv, packet::Read::Frame(..)) => {}
            // this is a repeat of a previously received Complete
            // respond with identical ACK
            (State::RecvDone, packet::Read::Cmd(cmd))
                if cmd.kind() == Ok(packet::CmdKind::Complete)
                    && cmd.head.packet == self.responding_to => {}
            // invalid continuation
            (State::RecvDone, _) => {}

            // -- SENDING --
            // this is a repetition of the original client Rx init packet.
            // respond with identical repeat ACK
            (State::SendStart, packet::Read::Cmd(cmd))
                if cmd.kind() == Ok(packet::CmdKind::Rx)
                    && cmd.head.packet == self.responding_to => {}
            // the client ACKs the last sent Frame, we move on to the next one
            (State::SendStart, packet::Read::Cmd(cmd))
                if cmd.kind() == Ok(packet::CmdKind::Confirm)
                    && cmd.head.responding_to == self.last_sent => {}
            // invalid continuation (not confirmation or repeat, doesn't require response)
            (State::SendStart, _) => {}
            // the client ACKs the last sent frame, move on to the next one
            (State::Send, packet::Read::Cmd(cmd))
                if cmd.kind() == Ok(packet::CmdKind::Confirm)
                    && cmd.head.responding_to == self.last_sent => {}
            // this is a repetition of the last received ACK
            // respond with an identical repeat of the last frame
            (State::Send, packet::Read::Cmd(cmd))
                if cmd.kind() == Ok(packet::CmdKind::Confirm)
                    && cmd.head.packet == self.responding_to => {}
            // invalid continuation
            (State::Send, _) => {}
            // the client ACKs the last packet, we have already run out of frames and sent the Complete message
            // respond with an identical Complete message
            (State::SendDone, packet::Read::Cmd(cmd))
                if cmd.kind() == Ok(packet::CmdKind::Confirm)
                    && cmd.head.packet == self.responding_to => {}
            // invalid continuation
            (State::SendDone, _) => {}
        }
    }
}
