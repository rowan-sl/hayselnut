use tokio::{
    net::UdpSocket,
    select,
    time::{sleep_until, Duration, Instant},
};

use crate::transport::{
    extract_packet_type, read_packet, CmdKind, Packet, UDP_MAX_SIZE,
};

pub async fn send_and_wait(sock: &UdpSocket, to: Packet, ty: Option<u8>, cmd: Option<CmdKind>) -> Packet {
    let bytes = to.as_bytes();

    let wait_dur = 1000;
    let next_wait_end = || Instant::now() + Duration::from_millis(wait_dur);
    let mut wait_end;
    let mut buf = vec![0u8; UDP_MAX_SIZE];
    let mut amnt = 0usize;

    'send: loop {
        amnt += 1;
        if amnt == 10 {
            panic!("Timed out");
        }
        sock.send(bytes).await.unwrap();
        wait_end = next_wait_end();
        break loop {
            select! {
                r = sock.recv(&mut buf) => {
                    println!("Received packet");
                    amnt = r.unwrap();
                }
                _ = sleep_until(wait_end) => {
                    println!("Timed out, resend");
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
            break p;
        };
    }
}


