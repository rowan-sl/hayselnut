use std::time::{Duration, Instant};

use crate::{
    buf::Cursor,
    env::Env,
    packet::{
        self,
        uid::{self, Uid},
    },
};

/// none of these errors may be fatal / indicate a logical error
///
/// fatal errors should be panics
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("transaction timed out")]
    TimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum State {
    #[default]
    Resting,
    Sending,
    /// waiting for Confim from server (need to be able to respond to it)
    /// a paralell `Done` variant does not exist for Receiving
    SendingDone,
    Receiving,
}

#[derive(Debug)]
pub struct Config {
    /// greatest time allowed per transaction
    pub max_trans_time: Duration,
}

#[derive(Debug)]
struct TransientState {
    /// time at which this transaction was started
    time: Instant,
    responding_to: Uid,
}

impl TransientState {
    pub fn new() -> Self {
        Self {
            time: Instant::now(),
            responding_to: Uid::null(),
        }
    }

    pub fn reset(&mut self) {
        // for now
        *self = Self::new();
    }
}

#[derive(Default)]
pub struct DriveResult<'s, 'r> {
    pub written: Option<packet::Write<'s>>,
    pub read: Option<&'r [u8]>,
    pub read_done: bool,
    /// is the full buffer (including the following Complete packet) sent
    pub write_done: bool,
}

#[derive(Debug)]
pub struct ConnState {
    state: State,
    conf: Config,
    env: Env,
    /// state which persists over a single transaction
    transient: TransientState,
    // not transient: avoids packets ariving late having the same ID as newer packets (the server doesn't care either way)
    gen: uid::Seq,
}

impl ConnState {
    pub fn new(conf: Config, env: Env) -> Self {
        Self {
            state: State::default(),
            conf,
            env,
            transient: TransientState::new(),
            gen: uid::Seq::new(),
        }
    }

    fn error_if_timeout(&self) -> Result<(), Error> {
        if self.transient.time.elapsed() > self.conf.max_trans_time {
            Err(Error::TimedOut)
        } else {
            Ok(())
        }
    }

    /// query the current connection state. (can be used to determine, for example, if is an appropreate time to begin a transaction)
    pub fn currently(&self) -> State {
        self.state
    }

    /// cancel the current transaction. this WILL screw up the server state. you WILL have issues communicating with the server after this
    ///
    /// to avoid issues with the server, wait for some time (at minimum the server's timeout duration) before continuing
    pub fn reset(&mut self) {
        self.transient.reset();
        self.gen = uid::Seq::new();
        self.state = State::Resting;
    }

    /// drive the client connection to begin a new send transaction
    ///
    /// # Errors if
    /// - currently in the transmit or receive state
    pub fn drive_send<'s>(
        &mut self,
        scratch: &'s mut [u8],
    ) -> Result<DriveResult<'s, 'static>, Error> {
        if self.state != State::Resting {
            panic!("attempted to initiate tx while not at rest");
        }
        self.transient.reset();
        let write = packet::Write::new(scratch)
            .unwrap()
            .with_packet(self.gen.next())
            .with_responding_to(self.transient.responding_to)
            .write_cmd(packet::CmdKind::Tx)
            .unwrap();
        Ok(DriveResult {
            written: Some(write),
            ..Default::default()
        })
    }

    /// drive the client connection to begin a new receive transaction
    ///
    /// # Errors if
    /// - currently in the transmit or receive state
    pub fn drive_recv(&mut self) -> Result<(), Error> {
        if self.state != State::Resting {
            panic!("attempted to initiate rx while not at rest");
        }
        self.transient.reset();
        unimplemented!()
    }

    /// drive the client connection to continue a transaction with a server packet
    ///
    /// `send` must be Some if currently sending, and contain the same Cursor for the entire duration
    pub fn drive_packet<'s, 'r>(
        &mut self,
        read: packet::Read<'r>,
        send: Option<&mut Cursor<'_>>,
        scratch: &'s mut [u8],
    ) -> Result<DriveResult<'s, 'static>, Error> {
        if self.state != State::Resting {
            self.error_if_timeout()?;
        }
        match (self.state, read) {
            // we have already received the final packets
            // we don't need more
            (State::Resting, _) => Ok(DriveResult::default()),
            (State::Sending, packet::Read::Cmd(cmd))
                if cmd.kind() == Ok(packet::CmdKind::Confirm) =>
            {
                todo!("handle repetitions of server packets");
                let write = if send.as_ref().is_some_and(|send| send.done_sending()) {
                    self.state = State::SendingDone;
                    packet::Write::new(scratch)
                        .unwrap()
                        .with_packet(self.gen.next())
                        .with_responding_to(self.transient.responding_to)
                        .write_cmd(packet::CmdKind::Complete)
                        .unwrap()
                } else {
                    packet::Write::new(scratch)
                        .unwrap()
                        .with_packet(self.gen.next())
                        .with_responding_to(self.transient.responding_to)
                        .write_frame_with(
                            send.map(|send| send.advance(self.env.max_packet_size))
                                .flatten()
                                .unwrap_or(&[]),
                        )
                        .unwrap()
                };
                unimplemented!()
            }
            (State::SendingDone, packet::Read::Cmd(cmd))
                if cmd.kind() == Ok(packet::CmdKind::Confirm) =>
            {
                unimplemented!()
            }
            (State::Sending | State::SendingDone, _) => Ok(DriveResult::default()),
            (State::Receiving, _) => unimplemented!(),
        }
    }
}
