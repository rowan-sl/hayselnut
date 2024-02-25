//! State machine for handling a single server connection

#[cfg(test)]
mod test;

use std::time::{Duration, Instant};

use crate::{
    buf::Cursor,
    env::Env,
    packet::{
        self,
        uid::{self, Uid},
    },
};

/// No server error may be fatal. everything can be recovered, or else
/// we are vonurable to (at minimum) bad actors screwing with the
/// connection in an unrecoverable way
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid data packet received (len was larger than the received size)")]
    InvalidData,
    /// this error should not be even somewhat fatal becuse of client::ConnState::reset
    /// resetting should cause the server to reset <timout> duration later
    #[error("transaction timed out - this should not be a fatal error")]
    Timeout,
}

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

#[derive(Default)]
pub struct ProcessResult<'sc, 're> {
    pub written: Option<packet::Write<'sc>>,
    pub read: Option<&'re [u8]>,
    pub read_complete: bool,
}

/// server connection state machine
#[derive(Debug)]
pub struct ConnState {
    state: State,
    conf: Config,
    env: Env,
    responding_to: Uid,
    last_sent: Uid,
    gen: uid::Seq,
    /// time at which this transaction (recv or transmit) was started
    trans_time: Instant,
}

impl ConnState {
    pub fn new(conf: Config, env: Env) -> Self {
        Self {
            state: State::default(),
            conf,
            env,
            responding_to: Uid::null(),
            last_sent: Uid::null(),
            gen: uid::Seq::new(),
            // garbage value, will be overwritten (state is Resting)
            trans_time: Instant::now(),
        }
    }

    fn advance_last_sent(&mut self) -> Uid {
        self.last_sent = self.gen.next();
        self.last_sent
    }

    fn calc_max_data(&self, scratch_len: usize) -> usize {
        std::cmp::min(self.env.max_packet_size, scratch_len)
            .checked_sub(packet::frame::SIZE_OF_HEADER)
            .expect("scratch buffer / env max packet size is smaller than the minimum frame size")
    }

    pub fn process<'re, 'sc>(
        &mut self,
        pkt: packet::Read<'re>,
        to_send: Option<&mut Cursor<'_>>,
        // scratch buffer for sending packets
        scratch: &'sc mut [u8],
    ) -> Result<ProcessResult<'sc, 're>, Error> {
        if let State::Recv | State::Send | State::RecvStart | State::SendStart = self.state {
            if self.trans_time.elapsed() > self.conf.max_trans_time {
                self.state = State::Resting;
                return Err(Error::Timeout);
            }
        }

        let pkt_responds_to_last =
            pkt.head().responding_to == self.last_sent && self.last_sent != Uid::null();
        // have we already responded
        let pkt_is_repeat =
            pkt.head().packet == self.responding_to && self.responding_to != Uid::null();

        // for documentation on proper order, see packet::CmdKind
        match (self.state, pkt) {
            // -- INITIALIZATION --
            // valid initialization of a transaction (Rest/Done => Tx/Rx)
            (
                State::Resting | State::RecvDone | State::SendDone | State::RecvStart,
                packet::Read::Cmd(cmd),
            ) if cmd.kind() == Ok(packet::CmdKind::Tx) => 'res: {
                #[cfg(test)]
                dbg!("server: rest || recv_start => recv_start || recv");
                // handles bolth the initial and the repeat
                let pkid = if self.state == State::RecvStart {
                    // on repeat

                    // invalid continuation
                    if !pkt_is_repeat {
                        #[cfg(test)]
                        dbg!("server: recv_start: invalid continuation");
                        break 'res Ok(ProcessResult::default());
                    }
                    self.last_sent
                } else {
                    // initial
                    self.responding_to = cmd.head.packet;
                    self.state = State::RecvStart; // Tx is client POV
                    self.trans_time = Instant::now();
                    self.advance_last_sent()
                };
                let write = packet::Write::new(scratch)
                    .unwrap()
                    .with_packet(pkid)
                    .with_responding_to(self.responding_to)
                    .write_cmd(packet::CmdKind::Confirm)
                    .unwrap();
                Ok(ProcessResult {
                    written: Some(write),
                    ..Default::default()
                })
            }
            (
                State::Resting | State::RecvDone | State::SendDone | State::SendStart,
                packet::Read::Cmd(cmd),
            ) if cmd.kind() == Ok(packet::CmdKind::Rx) => 'res: {
                let (pkid, data) = if let State::SendStart = self.state {
                    // repeat
                    if pkt_is_repeat {
                        (self.last_sent, to_send.map(|send| send.prev()))
                    } else {
                        // invalid continuation
                        break 'res Ok(ProcessResult::default());
                    }
                } else {
                    // initial
                    self.responding_to = cmd.head.packet;
                    self.state = State::SendStart; // Rx is client POV
                    self.trans_time = Instant::now();
                    (
                        self.advance_last_sent(),
                        to_send.map(|send| send.advance(self.calc_max_data(scratch.len()))),
                    )
                };

                let write = packet::Write::new(scratch)
                    .unwrap()
                    .with_packet(pkid)
                    .with_responding_to(self.responding_to);

                // only send a data packet if there is data to send, else send Complete
                let write = if let Some(data) = data.flatten() {
                    write.write_frame_with(data)
                } else {
                    self.state = State::SendDone;
                    write.write_cmd(packet::CmdKind::Complete)
                }
                .unwrap();

                Ok(ProcessResult {
                    written: Some(write),
                    ..Default::default()
                })
            }

            // // -- RECEIVING --
            // receive data from the client
            // this also covers the case of repeat and out-of-order packets
            (State::RecvStart | State::Recv, packet::Read::Frame(frame)) => {
                // redundant in the case that state is Recv
                self.state = State::Recv;
                if pkt_responds_to_last {
                    // used in next if statement
                    self.responding_to = frame.head.packet;
                    let _ = self.advance_last_sent();
                }
                let written = if pkt_responds_to_last || pkt_is_repeat {
                    Some(
                        packet::Write::new(scratch)
                            .unwrap()
                            .with_packet(self.last_sent)
                            .with_responding_to(self.responding_to)
                            .write_cmd(packet::CmdKind::Confirm)
                            .unwrap(),
                    )
                } else {
                    None
                };
                // after responding, reduce latency
                let read = if pkt_responds_to_last {
                    if let Ok(data) = frame.data() {
                        Some(data)
                    } else {
                        return Err(Error::InvalidData);
                    }
                } else {
                    None
                };
                Ok(ProcessResult {
                    written,
                    read,
                    read_complete: false,
                })
            }
            // done receiving
            (State::Recv, packet::Read::Cmd(cmd))
                if cmd.kind() == Ok(packet::CmdKind::Complete) =>
            {
                // redundant in the case that the state is RecvDone
                self.state = State::RecvDone;
                if pkt_responds_to_last {
                    // used in next if statement
                    self.responding_to = cmd.head.packet;
                    let _ = self.advance_last_sent();
                }
                let written = if pkt_responds_to_last || pkt_is_repeat {
                    Some(
                        packet::Write::new(scratch)
                            .unwrap()
                            .with_packet(self.last_sent)
                            .with_responding_to(self.responding_to)
                            .write_cmd(packet::CmdKind::Confirm)
                            .unwrap(),
                    )
                } else {
                    None
                };
                Ok(ProcessResult {
                    written,
                    read_complete: pkt_responds_to_last,
                    ..Default::default()
                })
            }

            // -- SENDING --
            // the client ACKs the last sent frame, move on to the next one
            // also handles the SendStart => Send transition
            // also handles the repeat / Out Of Order condition
            (State::SendStart | State::Send, packet::Read::Cmd(cmd))
                if cmd.kind() == Ok(packet::CmdKind::Confirm) =>
            'res: {
                let (pkid, data) = if pkt_responds_to_last {
                    self.state = State::Send;
                    self.responding_to = cmd.head.packet;
                    (
                        self.advance_last_sent(),
                        to_send.map(|send| send.advance(self.calc_max_data(scratch.len()))),
                    )
                } else if pkt_is_repeat {
                    (self.last_sent, to_send.map(|send| send.prev()))
                } else {
                    break 'res Ok(ProcessResult::default());
                };

                let write = packet::Write::new(scratch)
                    .unwrap()
                    .with_packet(pkid)
                    .with_responding_to(self.responding_to);

                // only send a data packet if there is data to send
                let write = if let Some(data) = data.flatten() {
                    write.write_frame_with(data)
                } else {
                    self.state = State::SendDone;
                    write.write_cmd(packet::CmdKind::Complete)
                }
                .unwrap();

                Ok(ProcessResult {
                    written: Some(write),
                    ..Default::default()
                })
            }
            // the client ACKs the last packet, we have already run out of frames and sent the Complete message
            // respond with an identical Complete message
            (State::SendDone, packet::Read::Cmd(cmd))
                if cmd.kind() == Ok(packet::CmdKind::Confirm) && pkt_is_repeat =>
            {
                let write = packet::Write::new(scratch)
                    .unwrap()
                    .with_packet(self.last_sent)
                    .with_responding_to(self.responding_to)
                    .write_cmd(packet::CmdKind::Complete)
                    .unwrap();
                Ok(ProcessResult {
                    written: Some(write),
                    ..Default::default()
                })
            }
            // invalid continuation (likely an ooo packet)
            _value => {
                #[cfg(test)]
                dbg!("server: invalid continuation", _value);
                dbg!(packet::Type::Frame as u8);
                dbg!(packet::Type::Command as u8);
                Ok(ProcessResult::default())
            }
        }
    }
}
