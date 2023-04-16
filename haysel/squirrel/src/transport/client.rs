use tokio::net::UdpSocket;

use crate::transport::{
    shared::send_and_wait,
    Cmd, CmdKind, Frame, Packet, UidGenerator, FRAME_BUF_SIZE,
    PACKET_TYPE_COMMAND, PACKET_TYPE_FRAME,
};

pub async fn mvp_send(sock: &UdpSocket, data: &[u8], uid_gen: &mut UidGenerator) {
    assert!(sock.peer_addr().is_ok(), "Socket must be connected");

    let Packet::Cmd(Cmd { packet: mut respond_to, .. }) = send_and_wait(
        sock,
        Packet::Cmd(Cmd {
            packet: uid_gen.next(),
            responding_to: 0,
            packet_ty: PACKET_TYPE_COMMAND,
            command: CmdKind::Tx as _,
            padding: Default::default(),
        }),
        Some(PACKET_TYPE_COMMAND),
        Some(CmdKind::Confirm),
    ).await else { unreachable!() };

    for chunk in data.chunks(FRAME_BUF_SIZE) {
        let mut arr_chunk = [0u8; FRAME_BUF_SIZE];
        arr_chunk.copy_from_slice(chunk);

        let Packet::Cmd(c) = send_and_wait(
            sock,
            Packet::Frame(Frame {
                packet: uid_gen.next(),
                responding_to: respond_to,
                packet_ty: PACKET_TYPE_FRAME,
                _pad: 0,
                len: chunk.len() as u16,
                data: arr_chunk,
            }),
            Some(PACKET_TYPE_COMMAND),
            Some(CmdKind::Confirm),
        ).await else { unreachable!() };

        respond_to = c.packet;
    }

    let Packet::Cmd(Cmd { .. }) = send_and_wait(
        sock,
        Packet::Cmd(Cmd {
            packet: uid_gen.next(),
            responding_to: respond_to,
            packet_ty: PACKET_TYPE_COMMAND,
            command: CmdKind::Complete as _,
            padding: Default::default(),
        }),
        Some(PACKET_TYPE_COMMAND),
        Some(CmdKind::Confirm),
    ).await else { unreachable!() };
}

/// Returns `None` if no frames were received (nothing was ready to send by the server)
pub async fn mvp_recv(sock: &UdpSocket, uid_gen: &mut UidGenerator) -> Option<Vec<u8>> {
    assert!(sock.peer_addr().is_ok(), "Socket must be connected");

    let first_frame = match send_and_wait(
        sock,
        Packet::Cmd(Cmd {
            packet: uid_gen.next(),
            responding_to: 0,
            packet_ty: PACKET_TYPE_COMMAND,
            command: CmdKind::Rx as _,
            padding: Default::default(),
        }),
        None,
        Some(CmdKind::Complete),
    ).await {
        Packet::Cmd(c) => {
            debug_assert_eq!(c.command, CmdKind::Complete as _); // validated in `send_and_wait`
            return None;
        }
        Packet::Frame(f) => f,
    };

    let mut respond_to = first_frame.packet;
    let mut buf = Vec::from(&first_frame.data[0..first_frame.len as _]);

    loop {
        match send_and_wait(
            sock,
            Packet::Cmd(Cmd {
                packet: uid_gen.next(),
                responding_to: respond_to,
                packet_ty: PACKET_TYPE_COMMAND,
                command: CmdKind::Confirm as _,
                padding: Default::default(),
            }),
            None,
            Some(CmdKind::Complete),
        ).await {
            Packet::Cmd(c) => {
                debug_assert_eq!(c.command, CmdKind::Complete as _); // validated in `send_and_wait`
                break;
            }
            Packet::Frame(f) => {
                buf.extend_from_slice(&f.data[0..f.len as _]);
                respond_to = f.packet;
            }
        };
    }

    Some(buf)
}

