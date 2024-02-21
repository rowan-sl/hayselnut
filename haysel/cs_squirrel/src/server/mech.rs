//! State machine for handling a single server connection

use std::{
    future::Future,
    time::{Duration, Instant},
};

use crate::{
    env::Env,
    packet::{
        self,
        uid::{self, Uid},
    },
};

/// "cursor" for indexing data to be sent
///
/// used in [ConnState::process]
pub struct Send<'buf> {
    inner: &'buf [u8],
    last: Option<&'buf [u8]>,
    cursor: usize,
}

impl<'buf> Send<'buf> {
    pub const fn new(buf: &'buf [u8]) -> Self {
        Self {
            inner: buf,
            last: None,
            cursor: 0,
        }
    }

    pub fn done_sending(&self) -> bool {
        self.inner.len() == self.cursor
    }

    /// returns the next (max) `by` bytes of `buf`. None if that would be zero bytes,
    /// and less than `by` if there are only that many left
    ///
    /// advances self.cursor by `by`
    pub(self) fn advance(&mut self, by: usize) -> Option<&[u8]> {
        if self.done_sending() {
            self.last = None;
            None?
        }
        let buf = &self.inner[self.cursor..];
        let amnt = std::cmp::min(buf.len(), by);
        self.cursor += amnt;
        let part = &buf[..amnt];
        self.last = Some(part);
        Some(part)
    }

    /// gets the *last* buffer provided by a call to `advance`
    pub(self) fn prev(&self) -> Option<&[u8]> {
        self.last
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid data packet received (len was larger than the received size)")]
    InvalidData,
    #[error("transaction timed out")]
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

    pub async fn process<'re, 'sc, Fut0: Future, Fut1: Future>(
        &mut self,
        pkt: packet::Read<'re>,
        to_send: Option<&mut Send<'_>>,
        // scratch buffer for sending packets
        scratch: &'sc mut [u8],
        mut send: impl FnMut(packet::Write<'sc>) -> Fut0,
        mut received: impl FnMut(&'re [u8]) -> Fut1,
    ) -> Result<(), Error> {
        if let State::Recv | State::Send = self.state {
            if self.trans_time.elapsed() > self.conf.max_trans_time {
                self.state = State::Resting;
                return Err(Error::Timeout);
            }
        }
        match (self.state, pkt) {
            // -- INITIALIZATION --
            // valid initialization of a transaction (Rest/Done => Tx/Rx)
            (State::Resting | State::RecvDone | State::SendDone, packet::Read::Cmd(cmd))
                if cmd.kind() == Ok(packet::CmdKind::Tx)
                    || cmd.kind() == Ok(packet::CmdKind::Rx) =>
            {
                self.responding_to = cmd.head.packet;
                match cmd.kind().unwrap() {
                    packet::CmdKind::Tx => {
                        self.state = State::RecvStart; // Tx is client POV
                        self.trans_time = Instant::now();
                        let write = packet::Write::new(scratch)
                            .unwrap()
                            .with_packet(self.advance_last_sent())
                            .with_responding_to(self.responding_to)
                            .write_cmd(packet::CmdKind::Confirm)
                            .unwrap();
                        send(write).await;
                    }
                    packet::CmdKind::Rx => {
                        self.state = State::SendStart; // Rx is client POV
                        self.trans_time = Instant::now();
                        let data = to_send
                            .map(|send| send.advance(self.calc_max_data(scratch.len())))
                            .flatten()
                            .unwrap_or(&[]);
                        let write = packet::Write::new(scratch)
                            .unwrap()
                            .with_packet(self.advance_last_sent())
                            .with_responding_to(self.responding_to)
                            .write_frame_with(data)
                            .unwrap();
                        send(write).await;
                    }
                    _ => unreachable!(),
                }
            }
            // not a valid initialization of a transaction
            (State::Resting, _) => {}

            // -- RECEIVING --
            // this is a REPETITION of the original packet.
            // respond in kind, with a repitition of the original ACK
            (State::RecvStart, packet::Read::Cmd(cmd))
                if cmd.kind() == Ok(packet::CmdKind::Tx)
                    && cmd.head.packet == self.responding_to =>
            {
                let write = packet::Write::new(scratch)
                    .unwrap()
                    .with_packet(self.last_sent)
                    .with_responding_to(self.responding_to)
                    .write_cmd(packet::CmdKind::Confirm)
                    .unwrap();
                send(write).await;
            }
            // not a valid continuation
            (State::RecvStart, packet::Read::Cmd(..)) => {}
            // Begin to receive data from the client
            (State::RecvStart, packet::Read::Frame(frame))
                if frame.head.responding_to == self.last_sent =>
            {
                self.state = State::Recv;
                self.responding_to = frame.head.packet;
                let write = packet::Write::new(scratch)
                    .unwrap()
                    .with_packet(self.advance_last_sent())
                    .with_responding_to(self.responding_to)
                    .write_cmd(packet::CmdKind::Confirm)
                    .unwrap();
                send(write).await;
                if let Ok(data) = frame.data() {
                    received(data).await;
                } else {
                    return Err(Error::InvalidData);
                }
            }
            // this is not the frame we are looking for
            (State::RecvStart, packet::Read::Frame(..)) => {}
            // done receiving
            (State::Recv, packet::Read::Cmd(cmd))
                if cmd.kind() == Ok(packet::CmdKind::Complete)
                    && cmd.head.responding_to == self.last_sent =>
            {
                self.state = State::RecvDone;
                self.responding_to = cmd.head.packet;
                let write = packet::Write::new(scratch)
                    .unwrap()
                    .with_packet(self.advance_last_sent())
                    .with_responding_to(self.responding_to)
                    .write_cmd(packet::CmdKind::Confirm)
                    .unwrap();
                send(write).await;
            }
            // not a valid continuation
            (State::Recv, packet::Read::Cmd(..)) => {}
            // this is a repeat of already received data
            // do not add it to a buffer, but repeat identical ACK
            (State::Recv, packet::Read::Frame(frame))
                if frame.head.packet == self.responding_to =>
            {
                // merge with Recv branch
                let write = packet::Write::new(scratch)
                    .unwrap()
                    .with_packet(self.last_sent)
                    .with_responding_to(self.responding_to)
                    .write_cmd(packet::CmdKind::Confirm)
                    .unwrap();
                send(write).await;
            }
            // receive more data
            (State::Recv, packet::Read::Frame(frame))
                if frame.head.responding_to == self.last_sent =>
            {
                // merge with RecvStart branch?
                self.responding_to = frame.head.packet;
                let write = packet::Write::new(scratch)
                    .unwrap()
                    .with_packet(self.advance_last_sent())
                    .with_responding_to(self.responding_to)
                    .write_cmd(packet::CmdKind::Confirm)
                    .unwrap();
                send(write).await;
                if let Ok(data) = frame.data() {
                    received(data).await;
                } else {
                    return Err(Error::InvalidData);
                }
            }
            // received out of order
            (State::Recv, packet::Read::Frame(..)) => {}
            // this is a repeat of a previously received Complete
            // respond with identical ACK
            (State::RecvDone, packet::Read::Cmd(cmd))
                if cmd.kind() == Ok(packet::CmdKind::Complete)
                    && cmd.head.packet == self.responding_to =>
            {
                // merge with first complete branch?
                let write = packet::Write::new(scratch)
                    .unwrap()
                    .with_packet(self.advance_last_sent())
                    .with_responding_to(self.responding_to)
                    .write_cmd(packet::CmdKind::Confirm)
                    .unwrap();
                send(write).await;
            }
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
        Ok(())
    }
}
