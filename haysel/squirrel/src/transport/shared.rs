use futures::{select, FutureExt};
use std::time::{Duration, Instant};

async fn sleep_until(when: Instant) {
    #[cfg(feature = "tokio")]
    tokio::time::sleep_until(when.into()).await;
    #[cfg(feature = "smol")]
    smol::Timer::at(when).await;
}

use crate::{
    net::UdpSocket,
    transport::{extract_packet_type, read_packet, CmdKind, Packet, UDP_MAX_SIZE},
};

pub async fn send_and_wait(
    sock: &UdpSocket,
    to: Packet,
    ty: Option<u8>,
    cmd: Option<CmdKind>,
) -> Packet {
    //info!("Sending packet {to:#?}");
    let bytes = to.as_bytes();

    let wait_dur = 5000;
    let next_wait_end = || Instant::now() + Duration::from_millis(wait_dur);
    let mut wait_end;
    let mut buf = vec![0u8; UDP_MAX_SIZE];
    let mut amnt = 0usize;

    'send: loop {
        //info!("Sending");
        amnt += 1;
        if amnt == 10 {
            panic!("Timed out -- max limit hit");
        }
        sock.send(bytes).await.unwrap();
        wait_end = next_wait_end();
        break loop {
            select! {
                r = sock.recv_from(&mut buf).fuse() => {
                    //info!("Received data");
                    let (c, f) = r.unwrap();
                    if f == sock.peer_addr().unwrap() {
                        amnt = c;
                    } else {
                        warn!("Received data from an unknown source at {f:?}");
                        continue;
                    }
                }
                _ = sleep_until(wait_end).fuse() => {
                    info!("Timed out, resend");
                    continue 'send;
                }
            }
            let buf = &buf[0..amnt];
            let Some(p) = read_packet(buf) else { continue };
            if p.responding_to() != to.uid() {
                continue;
            }
            if ty.is_some() && (extract_packet_type(buf).unwrap() != ty.unwrap()) {
                continue;
            }
            if let Packet::Cmd(c) = p {
                if cmd.is_some() && (c.command != cmd.unwrap() as _) {
                    continue;
                }
            }
            //info!("Received requested packet {p:#?}");
            break p;
        };
    }
}
