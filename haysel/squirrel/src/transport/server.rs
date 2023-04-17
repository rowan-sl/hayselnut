use std::{time::{Instant, Duration}, collections::VecDeque};

use flume::Sender;
use num_enum::TryFromPrimitive;

use crate::net::{UdpSocket, SocketAddr};

use super::{Packet, UDP_MAX_SIZE, read_packet, Cmd, CmdKind, UidGenerator, PACKET_TYPE_COMMAND, Frame, PACKET_TYPE_FRAME, FRAME_BUF_SIZE};

pub async fn recv_next_packet(sock: &UdpSocket) -> Result<Option<(SocketAddr, Packet)>, crate::net::Error> {
    let mut buf = [0; UDP_MAX_SIZE];
    let (amnt, from) = sock.recv_from(&mut buf).await?;
    if amnt > buf.len() {
        return Ok(None);
    }
    let Some(p) = read_packet(&buf[0..amnt]) else { return Ok(None); };
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

#[derive(Debug)]
pub struct ClientInterface {
    state: State,
    addr: SocketAddr,
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
    dispatch: Sender<(SocketAddr, DispatchEvent)>,
}

impl ClientInterface {
    /// dispatch must be unbounded
    pub fn new(max_transaction_time: Duration, addr: SocketAddr, dispatch: Sender<(SocketAddr, DispatchEvent)>) -> Self {
        Self {
            state: State::default(),
            addr,
            respond_to: 0,
            last_sent: 0,
            uid_gen: UidGenerator::new(),
            transaction_time: Instant::now(),//never used
            max_transaction_time,
            recev_buf: vec![],
            send_queue: Default::default(),
            send_buf: vec![],
            dispatch,
        }
    }

    pub fn queue(&mut self, to_send: Vec<u8>) {
        self.send_queue.push_front(to_send);
    }

    pub fn handle(&mut self, packet: Packet) {
        if let State::Receiving | State::Sending = self.state {
            if self.transaction_time.elapsed() > self.max_transaction_time {
                self.state = State::Resting;
                self.dispatch.send((self.addr, DispatchEvent::TimedOut)).unwrap();
                return;
            }
        }
        match (self.state, packet) {
            (State::Resting
                | State::TheoreticallyDoneReceiving
                | State::TheoreticallyDoneSending, Packet::Cmd(Cmd { packet, command, .. }))
                if command == CmdKind::Tx as _ || command == CmdKind::Rx as _ => {
                self.respond_to = packet;
                match CmdKind::try_from_primitive(command).unwrap() {
                    CmdKind::Tx => {
                        self.state = State::ReceivingStart; // Tx is POV of the CLIENT
                        self.transaction_time = Instant::now();
                        self.recev_buf.clear();
                        self.dispatch.send((self.addr, DispatchEvent::Send(Packet::Cmd(Cmd {
                            packet: { self.last_sent = self.uid_gen.next(); self.last_sent },
                            responding_to: self.respond_to,
                            packet_ty: PACKET_TYPE_COMMAND,
                            command: CmdKind::Confirm as _,
                            padding: [0; 2],
                        })))).unwrap();
                    }
                    CmdKind::Rx => {
                        todo!();
                        self.state = State::SendingStart;
                        self.transaction_time = Instant::now();
                        // send_queue value only removed when sending is done
                        self.send_buf = self.send_queue.back().cloned().unwrap_or(vec![]);
                        self.dispatch.send((self.addr, DispatchEvent::Send(Packet::Frame(Frame {
                            packet: { self.last_sent = self.uid_gen.next(); self.last_sent },
                            responding_to: self.respond_to,
                            packet_ty: PACKET_TYPE_FRAME,
                            _pad: 0,
                            len: self.send_buf.len().clamp(0, FRAME_BUF_SIZE) as _,
                            data: {
                                let mut buf = [0u8; FRAME_BUF_SIZE];
                                buf.copy_from_slice(&self.send_buf[0..self.send_buf.len().clamp(0, FRAME_BUF_SIZE)]);
                                buf
                            }
                        })))).unwrap();
                    }
                    CmdKind::Confirm | CmdKind::Complete => unreachable!()
                }
            }
            (State::Resting, _) => {}
            (State::ReceivingStart, Packet::Cmd(cmd))
                if cmd.command == CmdKind::Tx as _
                && cmd.packet == self.respond_to => {
                // this is a repitition of the initial Transmit init packet.
                // respond again, identically to the first time.
                self.dispatch.send((self.addr, DispatchEvent::Send(Packet::Cmd(Cmd {
                    packet: self.last_sent,
                    responding_to: self.respond_to,
                    packet_ty: PACKET_TYPE_COMMAND,
                    command: CmdKind::Confirm as _,
                    padding: [0; 2],
                })))).unwrap();
            }
            (State::ReceivingStart, Packet::Cmd(..)) => {}
            (State::ReceivingStart, Packet::Frame(fr)) if fr.responding_to == self.last_sent => {
                self.respond_to = fr.packet;
                let data = &fr.data[0..fr.len as _];
                self.recev_buf.extend_from_slice(data);
                self.dispatch.send((self.addr, DispatchEvent::Send(Packet::Cmd(Cmd {
                    packet: { self.last_sent = self.uid_gen.next(); self.last_sent },
                    responding_to: self.respond_to,
                    packet_ty: PACKET_TYPE_COMMAND,
                    command: CmdKind::Confirm as _,
                    padding: [0; 2],
                })))).unwrap();
                self.state = State::Receiving;
            }
            (State::ReceivingStart, Packet::Frame(..)) => {}
            (State::Receiving, Packet::Cmd(cmd))
                if cmd.command == CmdKind::Complete as _
                && cmd.packet == self.respond_to => {
                // the first end-transaction packet.
                self.dispatch.send((self.addr, DispatchEvent::Received(self.recev_buf.clone()))).unwrap();
                self.dispatch.send((self.addr, DispatchEvent::Send(Packet::Cmd(Cmd {
                    packet: { self.last_sent = self.uid_gen.next(); self.last_sent },
                    responding_to: self.respond_to,
                    packet_ty: PACKET_TYPE_COMMAND,
                    command: CmdKind::Confirm as _,
                    padding: [0; 2],
                })))).unwrap();
                self.state = State::TheoreticallyDoneReceiving;
            }
            (State::Receiving, Packet::Cmd(..)) => {}
            (State::Receiving, Packet::Frame(fr)) if fr.packet == self.respond_to => {
                // already received this data, dont need to add it again
                // could merge with the ReceivingStart branch of this kind?
                self.dispatch.send((self.addr, DispatchEvent::Send(Packet::Cmd(Cmd {
                    packet: self.last_sent,
                    responding_to: self.respond_to,
                    packet_ty: PACKET_TYPE_COMMAND,
                    command: CmdKind::Confirm as _,
                    padding: [0; 2],
                })))).unwrap();
            }
            (State::Receiving, Packet::Frame(fr)) if fr.responding_to == self.last_sent => {
                // should be the same code as the ReceivingStart branch of this kind, merge?
                self.respond_to = fr.packet;
                let data = &fr.data[0..fr.len as _];
                self.recev_buf.extend_from_slice(data);
                self.dispatch.send((self.addr, DispatchEvent::Send(Packet::Cmd(Cmd {
                    packet: { self.last_sent = self.uid_gen.next(); self.last_sent },
                    responding_to: self.respond_to,
                    packet_ty: PACKET_TYPE_COMMAND,
                    command: CmdKind::Confirm as _,
                    padding: [0; 2],
                })))).unwrap();
            }
            (State::Receiving, Packet::Frame(..)) => {}
            (State::TheoreticallyDoneReceiving, Packet::Cmd(cmd))
                if cmd.command == CmdKind::Complete as _
                && cmd.packet == self.respond_to => {
                self.dispatch.send((self.addr, DispatchEvent::Send(Packet::Cmd(Cmd {
                    packet: self.last_sent,
                    responding_to: self.respond_to,
                    packet_ty: PACKET_TYPE_COMMAND,
                    command: CmdKind::Confirm as _,
                    padding: [0; 2],
                })))).unwrap();
            }
            (State::TheoreticallyDoneReceiving, _) => {}
            // (State::Receiving, Packet::Cmd(cmd)) if cmd.command == CmdKind::Complete as _ => {
            //     todo!()
            // }
            // (State::Receiving, Packet::Cmd(..)) => {}
            // (State::Receiving, Packet::Frame(frame)) if frame.responding_to == self.last_sent => {
            //     self.respond_to = frame.packet;
            //     let data = &frame.data[0..frame.len as _];
            //     self.recev_buf.extend_from_slice(data);
            //     self.dispatch.send((self.addr, DispatchEvent::Send(Packet::Cmd(Cmd {
            //         packet: { self.last_sent = self.uid_gen.next(); self.last_sent },
            //         responding_to: self.respond_to,
            //         packet_ty: PACKET_TYPE_COMMAND,
            //         command: CmdKind::Confirm as _,
            //         padding: [0; 2],
            //     })))).unwrap();
            // }
            _ => todo!()
        }
    }
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

