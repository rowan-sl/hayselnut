use futures::{select, FutureExt};
use std::{
    io,
    time::{Duration, Instant},
};
use tokio::{net::UdpSocket, time::sleep_until};

use crate::transport::{
    extract_packet_type, read_packet, CmdKind, Packet, PACKET_TYPE_COMMAND, UDP_MAX_SIZE,
};

#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error("I/O Error: {0:?}")]
    IOError(#[from] io::Error),
    #[error("Timed out")]
    TimedOut,
}

pub enum ExpectedResponse {
    FrameOrCommand { cmd: CmdKind },
    Command { cmd: CmdKind },
}

pub async fn send_and_wait(
    sock: &UdpSocket,
    to: Packet,
    expected_response: ExpectedResponse,
    max_attempts: usize,
    wait_dur: Duration,
) -> Result<Packet, SendError> {
    assert!(max_attempts > 0);
    let bytes = to.as_bytes();

    let next_wait_end = || Instant::now() + wait_dur;
    let mut wait_end;
    let mut buf = vec![0u8; UDP_MAX_SIZE];
    let mut attempt = 0usize;

    'send: loop {
        attempt += 1;
        if attempt > max_attempts {
            return Err(SendError::TimedOut);
        }
        sock.send(bytes).await?;
        wait_end = next_wait_end();
        break loop {
            let amnt;
            select! {
                r = sock.recv_from(&mut buf).fuse() => {
                    let (c, f) = r?;
                    if f == sock.peer_addr()? {
                        amnt = c;
                    } else {
                        debug!("send_and_wait: received data from an unknown source (IP: {f:?})");
                        continue;
                    }
                }
                _ = sleep_until(wait_end.into()).fuse() => {
                    warn!("send_and_wait: attempt {attempt}/{max_attempts} timed out after {wait_dur:?} retrying");
                    continue 'send;
                }
            }
            let buf = &buf[0..amnt]; // buf will allways be large enough
            let Some(p) = read_packet(buf) else {
                debug!("send_and_wait: received a corrupt packet (call to read_packet failed)");
                continue;
            };
            if p.responding_to() != to.uid() {
                debug!("send_and_wait: received a [likely out of order] packet (responding_to UID mismatch)");
                continue;
            }
            // calls to .unwrap() here are unreachable
            let expected_command = match expected_response {
                ExpectedResponse::FrameOrCommand { cmd } => cmd,
                ExpectedResponse::Command { cmd } => {
                    if extract_packet_type(buf).unwrap() != PACKET_TYPE_COMMAND {
                        debug!("send_and_wait: expected packet of type command, received packet of type frame (ignoring)");
                        continue;
                    }
                    cmd
                }
            };
            if let Packet::Cmd(c) = p {
                if c.command != expected_command as _ {
                    debug!("send_and_wait: expected packet with command {:?}, received packet with command {:?} (ignoring)", expected_command, c.command);
                    continue;
                }
            }
            break Ok(p);
        };
    }
}
