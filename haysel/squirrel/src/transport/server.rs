use std::{
    collections::VecDeque,
    mem::swap,
    net::SocketAddr,
    time::{Duration, Instant},
};

use num_enum::TryFromPrimitive;
use tokio::{io, net::UdpSocket};

use super::{
    read_packet, Cmd, CmdKind, Frame, Packet, UidGenerator, FRAME_BUF_SIZE, PACKET_TYPE_COMMAND,
    PACKET_TYPE_FRAME, UDP_MAX_SIZE,
};

pub async fn recv_next_packet(sock: &UdpSocket) -> io::Result<Option<(SocketAddr, Packet)>> {
    let mut buf = [0; UDP_MAX_SIZE];
    let (amnt, from) = sock.recv_from(&mut buf).await?;
    if amnt > buf.len() {
        return Ok(None);
    }
    let Some(p) = read_packet(&buf[0..amnt]) else {
        return Ok(None);
    };
    Ok(Some((from, p)))
}

#[derive(Debug, Clone)]
pub enum DispatchEvent {
    Send(Packet),
    /// connection to SocketAddr timed out (transaction took too long to complete)
    TimedOut,
    /// data has been received
    Received(Vec<u8>),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
enum State {
    #[default]
    Resting,
    ReceivingStart,
    Receiving,
    TheoreticallyDoneReceiving,
    SendingStart,
    Sending,
    TheoreticallyDoneSending,
}

#[derive(Debug)]
pub struct ClientInterface {
    state: State,
    // packet the next packet is responding to
    respond_to: u32,
    last_sent: u32,
    uid_gen: UidGenerator,
    // time since entering `Receiving` or `Sending` state
    transaction_time: Instant,
    max_transaction_time: Duration,
    recev_buf: Vec<u8>,
    send_queue: VecDeque<Vec<u8>>,
    send_buf: Vec<u8>,
    last_sent_send_buf: Vec<u8>,
}

impl ClientInterface {
    /// dispatch must be unbounded
    pub fn new(max_transaction_time: Duration) -> Self {
        Self {
            state: State::default(),
            respond_to: 0,
            last_sent: 0,
            uid_gen: UidGenerator::new(),
            transaction_time: Instant::now(), //never used
            max_transaction_time,
            recev_buf: vec![],
            send_queue: Default::default(),
            send_buf: vec![],
            last_sent_send_buf: vec![],
        }
    }

    pub fn queue(&mut self, to_send: Vec<u8>) {
        self.send_queue.push_front(to_send);
    }

    pub fn handle(&mut self, packet: Packet) -> Vec<DispatchEvent> {
        let mut dispatch = vec![];
        //info!("state: {:?}", self.state);
        if let State::Receiving | State::Sending = self.state {
            if self.transaction_time.elapsed() > self.max_transaction_time {
                self.state = State::Resting;
                dispatch.push(DispatchEvent::TimedOut);
                return dispatch;
            }
        }
        match (self.state, packet) {
            (
                State::Resting
                | State::TheoreticallyDoneReceiving
                | State::TheoreticallyDoneSending,
                Packet::Cmd(Cmd {
                    packet, command, ..
                }),
            ) if command == CmdKind::Tx as _ || command == CmdKind::Rx as _ => {
                self.respond_to = packet;
                match CmdKind::try_from_primitive(command).unwrap() {
                    CmdKind::Tx => {
                        self.state = State::ReceivingStart; // Tx is POV of the CLIENT
                        self.transaction_time = Instant::now();
                        self.recev_buf.clear();
                        dispatch.push(DispatchEvent::Send(Packet::Cmd(Cmd {
                            packet: {
                                self.last_sent = self.uid_gen.next();
                                self.last_sent
                            },
                            responding_to: self.respond_to,
                            packet_ty: PACKET_TYPE_COMMAND,
                            command: CmdKind::Confirm as _,
                            padding: [0; 2],
                        })));
                    }
                    CmdKind::Rx => {
                        self.state = State::SendingStart;
                        self.transaction_time = Instant::now();
                        // send_queue value only removed when sending is done
                        self.send_buf = self.send_queue.back().cloned().unwrap_or(vec![]);
                        self.last_sent_send_buf.clear();
                        dispatch.push(DispatchEvent::Send(Packet::Frame(Frame {
                            packet: {
                                self.last_sent = self.uid_gen.next();
                                self.last_sent
                            },
                            responding_to: self.respond_to,
                            packet_ty: PACKET_TYPE_FRAME,
                            _pad: 0,
                            len: self.send_buf.len().clamp(0, FRAME_BUF_SIZE) as _,
                            data: {
                                let mut buf = [0u8; FRAME_BUF_SIZE];
                                let mut past_buf = self
                                    .send_buf
                                    .split_off(FRAME_BUF_SIZE.clamp(0, self.send_buf.len()));
                                swap(&mut self.send_buf, &mut past_buf);
                                self.last_sent_send_buf = past_buf.clone();
                                buf[0..past_buf.len()].copy_from_slice(&past_buf);
                                buf
                            },
                        })));
                    }
                    CmdKind::Confirm | CmdKind::Complete => unreachable!(),
                }
            }
            (State::Resting, _) => {}
            // receiving
            (State::ReceivingStart, Packet::Cmd(cmd))
                if cmd.command == CmdKind::Tx as _ && cmd.packet == self.respond_to =>
            {
                // this is a repitition of the initial Transmit init packet.
                // respond again, identically to the first time.
                dispatch.push(DispatchEvent::Send(Packet::Cmd(Cmd {
                    packet: self.last_sent,
                    responding_to: self.respond_to,
                    packet_ty: PACKET_TYPE_COMMAND,
                    command: CmdKind::Confirm as _,
                    padding: [0; 2],
                })));
            }
            (State::ReceivingStart, Packet::Cmd(..)) => {}
            (State::ReceivingStart, Packet::Frame(fr)) if fr.responding_to == self.last_sent => {
                self.respond_to = fr.packet;
                let data = &fr.data[0..fr.len as _];
                self.recev_buf.extend_from_slice(data);
                dispatch.push(DispatchEvent::Send(Packet::Cmd(Cmd {
                    packet: {
                        self.last_sent = self.uid_gen.next();
                        self.last_sent
                    },
                    responding_to: self.respond_to,
                    packet_ty: PACKET_TYPE_COMMAND,
                    command: CmdKind::Confirm as _,
                    padding: [0; 2],
                })));
                self.state = State::Receiving;
            }
            (State::ReceivingStart, Packet::Frame(..)) => {}
            (State::Receiving, Packet::Cmd(cmd))
                if cmd.command == CmdKind::Complete as _ && cmd.responding_to == self.last_sent =>
            {
                self.respond_to = cmd.packet;
                // the first end-transaction packet.
                dispatch.push(DispatchEvent::Received(self.recev_buf.clone()));
                dispatch.push(DispatchEvent::Send(Packet::Cmd(Cmd {
                    packet: {
                        self.last_sent = self.uid_gen.next();
                        self.last_sent
                    },
                    responding_to: self.respond_to,
                    packet_ty: PACKET_TYPE_COMMAND,
                    command: CmdKind::Confirm as _,
                    padding: [0; 2],
                })));
                self.state = State::TheoreticallyDoneReceiving;
            }
            (State::Receiving, Packet::Cmd(..)) => {}
            (State::Receiving, Packet::Frame(fr)) if fr.packet == self.respond_to => {
                // already received this data, dont need to add it again
                // could merge with the ReceivingStart branch of this kind?
                dispatch.push(DispatchEvent::Send(Packet::Cmd(Cmd {
                    packet: self.last_sent,
                    responding_to: self.respond_to,
                    packet_ty: PACKET_TYPE_COMMAND,
                    command: CmdKind::Confirm as _,
                    padding: [0; 2],
                })));
            }
            (State::Receiving, Packet::Frame(fr)) if fr.responding_to == self.last_sent => {
                // should be the same code as the ReceivingStart branch of this kind, merge?
                self.respond_to = fr.packet;
                let data = &fr.data[0..fr.len as _];
                self.recev_buf.extend_from_slice(data);
                dispatch.push(DispatchEvent::Send(Packet::Cmd(Cmd {
                    packet: {
                        self.last_sent = self.uid_gen.next();
                        self.last_sent
                    },
                    responding_to: self.respond_to,
                    packet_ty: PACKET_TYPE_COMMAND,
                    command: CmdKind::Confirm as _,
                    padding: [0; 2],
                })));
            }
            (State::Receiving, Packet::Frame(..)) => {}
            (State::TheoreticallyDoneReceiving, Packet::Cmd(cmd))
                if cmd.command == CmdKind::Complete as _ && cmd.packet == self.respond_to =>
            {
                dispatch.push(DispatchEvent::Send(Packet::Cmd(Cmd {
                    packet: self.last_sent,
                    responding_to: self.respond_to,
                    packet_ty: PACKET_TYPE_COMMAND,
                    command: CmdKind::Confirm as _,
                    padding: [0; 2],
                })));
            }
            (State::TheoreticallyDoneReceiving, _) => {}
            // sending
            (State::SendingStart, Packet::Cmd(cmd))
                if cmd.command == CmdKind::Rx as _ && cmd.packet == self.respond_to =>
            {
                // repeat the Rx init packet
                dispatch.push(DispatchEvent::Send(Packet::Frame(Frame {
                    packet: self.last_sent,
                    responding_to: self.respond_to,
                    packet_ty: PACKET_TYPE_FRAME,
                    _pad: 0,
                    len: self.last_sent_send_buf.len() as _,
                    data: {
                        let mut buf = [0u8; FRAME_BUF_SIZE];
                        buf[0..self.last_sent_send_buf.len()]
                            .copy_from_slice(&self.last_sent_send_buf);
                        buf
                    },
                })));
            }
            (State::SendingStart, Packet::Cmd(cmd))
                if cmd.command == CmdKind::Confirm as _ && cmd.responding_to == self.last_sent =>
            {
                self.respond_to = cmd.packet;
                // send the next frame (or end the transaction), go into Sending mode (or done mode)
                if self.send_buf.is_empty() {
                    dispatch.push(DispatchEvent::Send(Packet::Cmd(Cmd {
                        packet: {
                            self.last_sent = self.uid_gen.next();
                            self.last_sent
                        },
                        responding_to: self.respond_to,
                        packet_ty: PACKET_TYPE_COMMAND,
                        command: CmdKind::Complete as _,
                        padding: [0; 2],
                    })));

                    self.state = State::TheoreticallyDoneSending;
                } else {
                    dispatch.push(DispatchEvent::Send(Packet::Frame(Frame {
                        packet: {
                            self.last_sent = self.uid_gen.next();
                            self.last_sent
                        },
                        responding_to: self.respond_to,
                        packet_ty: PACKET_TYPE_FRAME,
                        _pad: 0,
                        len: self.send_buf.len().clamp(0, FRAME_BUF_SIZE) as _,
                        data: {
                            let mut buf = [0u8; FRAME_BUF_SIZE];
                            let mut past_buf = self
                                .send_buf
                                .split_off(FRAME_BUF_SIZE.clamp(0, self.send_buf.len()));
                            swap(&mut self.send_buf, &mut past_buf);
                            self.last_sent_send_buf = past_buf.clone();
                            buf[0..past_buf.len()].copy_from_slice(&past_buf);
                            buf
                        },
                    })));
                    self.state = State::Sending;
                }
            }
            (State::SendingStart, _) => {}
            (State::Sending, Packet::Cmd(cmd))
                if cmd.command == CmdKind::Confirm as _ && cmd.responding_to == self.last_sent =>
            {
                self.respond_to = cmd.packet;
                // send the next frame
                if self.send_buf.is_empty() {
                    dispatch.push(DispatchEvent::Send(Packet::Cmd(Cmd {
                        packet: {
                            self.last_sent = self.uid_gen.next();
                            self.last_sent
                        },
                        responding_to: self.respond_to,
                        packet_ty: PACKET_TYPE_COMMAND,
                        command: CmdKind::Complete as _,
                        padding: [0; 2],
                    })));
                    self.state = State::TheoreticallyDoneSending;
                } else {
                    dispatch.push(DispatchEvent::Send(Packet::Frame(Frame {
                        packet: {
                            self.last_sent = self.uid_gen.next();
                            self.last_sent
                        },
                        responding_to: self.respond_to,
                        packet_ty: PACKET_TYPE_FRAME,
                        _pad: 0,
                        len: self.send_buf.len().clamp(0, FRAME_BUF_SIZE) as _,
                        data: {
                            let mut buf = [0u8; FRAME_BUF_SIZE];
                            let mut past_buf = self
                                .send_buf
                                .split_off(FRAME_BUF_SIZE.clamp(0, self.send_buf.len()));
                            swap(&mut self.send_buf, &mut past_buf);
                            self.last_sent_send_buf = past_buf.clone();
                            buf[0..past_buf.len()].copy_from_slice(&past_buf);
                            buf
                        },
                    })));
                }
            }
            (State::Sending, Packet::Cmd(cmd))
                if cmd.command == CmdKind::Confirm as _ && cmd.packet == self.respond_to =>
            {
                // repeat the last frame
                dispatch.push(DispatchEvent::Send(Packet::Frame(Frame {
                    packet: self.last_sent,
                    responding_to: self.respond_to,
                    packet_ty: PACKET_TYPE_FRAME,
                    _pad: 0,
                    len: self.last_sent_send_buf.len() as _,
                    data: {
                        let mut buf = [0u8; FRAME_BUF_SIZE];
                        buf[0..self.last_sent_send_buf.len()]
                            .copy_from_slice(&self.last_sent_send_buf);
                        buf
                    },
                })));
            }
            (State::Sending, _) => {}
            (State::TheoreticallyDoneSending, Packet::Cmd(cmd))
                if cmd.command == CmdKind::Confirm as _ && cmd.packet == self.respond_to =>
            {
                dispatch.push(DispatchEvent::Send(Packet::Cmd(Cmd {
                    packet: self.last_sent,
                    responding_to: self.respond_to,
                    packet_ty: PACKET_TYPE_COMMAND,
                    command: CmdKind::Complete as _,
                    padding: [0; 2],
                })));
            }
            (State::TheoreticallyDoneSending, _) => {}
        }
        dispatch
    }
}
