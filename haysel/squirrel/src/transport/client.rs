use std::mem::swap;

use tokio::{
    net::UdpSocket,
    select,
    time::{sleep_until, Duration, Instant},
};
use uuid::Uuid;
use zerocopy::{AsBytes, FromBytes};

use super::{
    controll::{Cmd, CmdPacket},
    frame::Frame,
    packet::{extract_packet_type, PacketHeader, PACKET_TYPE_CONTROLL, UDP_MAX_SIZE},
};

async fn mvp_send(sock: UdpSocket, data: &[u8]) {
    assert!(sock.peer_addr().is_ok(), "socket must be connected");
    let mut id = Uuid::new_v4();
    let mut next_id = Uuid::new_v4();
    fn update_id(id: &mut Uuid, next_id: &mut Uuid) {
        swap(id, next_id);
        *next_id = Uuid::new_v4();
    }

    let wait_dur = 1000;
    let next_wait_end = || Instant::now() + Duration::from_millis(wait_dur);
    let mut wait_end;
    let mut buf = vec![0u8; UDP_MAX_SIZE + 1];

    let mut expected_next_recv_id = 'send: loop {
        sock.send(CmdPacket::new(id, next_id, Cmd::RequestTransaction).as_bytes())
            .await
            .unwrap();

        wait_end = next_wait_end();
        let expected_next_recv_id = 'recv: loop {
            buf.fill(0);
            select! {
                r = sock.recv(&mut buf) => {
                    r.unwrap();
                }
                _ = sleep_until(wait_end) => {
                    continue 'send;// try again
                }
            }

            let Some(t) = extract_packet_type(&buf) else {
                println!("Received data < min packet size");
                continue 'recv; // recv, do not reset timeout
            };
            let h = PacketHeader::read_from_prefix(buf.as_slice()).unwrap();
            // check that this is indeed the packet we want (received ID (transaction init response only) == next_id of request packet)
            // this must happen before any other checks
            // in a proper send fn, this packet should be dropped and receiveing should restart
            let mut expected_next_recv_id = next_id;
            let recved_id = Uuid::from_bytes(h.id);
            if recved_id != expected_next_recv_id {
                println!("Received out of order packet");
                continue 'recv; // do not reset timeout
            }
            expected_next_recv_id = Uuid::from_bytes(h.next_id);
            if t != PACKET_TYPE_CONTROLL {
                println!("Received expected ID, but with invalid data");
                continue 'recv;
            }
            let Some(p) = CmdPacket::from_buf_validated(&buf) else {
                println!("Received invalid packet with expected ID");
                continue 'recv;
            };
            let Cmd::ConfirmTransaction = p.data.extract_cmd().unwrap() else {
                println!("unexpected cmd received");
                continue 'recv;
            };
            update_id(&mut id, &mut next_id);
            break expected_next_recv_id;
        };
        break expected_next_recv_id;
    };

    let frames = Frame::for_data(data, || {
        let t = (id, next_id);
        update_id(&mut id, &mut next_id);
        t
    });
    for frame in frames {
        'send: loop {
            sock.send(frame.as_bytes_compact()).await.unwrap();

            wait_end = next_wait_end();
            'recv: loop {
                buf.fill(0);
                select! {
                    r = sock.recv(&mut buf) => {
                        r.unwrap();
                    }
                    _ = sleep_until(wait_end) => {
                        continue 'send;// try again
                    }
                }

                let Some(t) = extract_packet_type(&buf) else {
                    println!("Received data < min packet size");
                    continue 'recv; // recv, do not reset timeout
                };
                let h = PacketHeader::read_from_prefix(buf.as_slice()).unwrap();
                // check that this is indeed the packet we want (received ID (transaction init response only) == next_id of request packet)
                // this must happen before any other checks
                // in a proper send fn, this packet should be dropped and receiveing should restart
                let recved_id = Uuid::from_bytes(h.id);
                if recved_id != expected_next_recv_id {
                    println!("Received out of order packet");
                    continue 'recv; // do not reset timeout
                }
                expected_next_recv_id = Uuid::from_bytes(h.next_id);
                if t != PACKET_TYPE_CONTROLL {
                    println!("Received expected ID, but with invalid data");
                    continue 'recv;
                }
                let Some(p) = CmdPacket::from_buf_validated(&buf) else {
                    println!("Received invalid packet with expected ID");
                    continue 'recv;
                };
                let Cmd::Received = p.data.extract_cmd().unwrap() else {
                    println!("unexpected cmd received");
                    continue 'recv;
                };
                if p.data.data_id != frame.header.id {
                    println!("Received confirmation packet for wrong `id`");
                    continue 'recv;
                }
                update_id(&mut id, &mut next_id);
                break;
            }
            break;
        }
    }
}

async fn mvp_recv(sock: UdpSocket) {
    assert!(sock.peer_addr().is_ok(), "socket must be connected");
    let mut id = Uuid::new_v4();
    let mut next_id = Uuid::new_v4();
    fn update_id(id: &mut Uuid, next_id: &mut Uuid) {
        swap(id, next_id);
        *next_id = Uuid::new_v4();
    }
}

//
// trait TransactionSig {
//
// }
//
// pub struct Client {
//     sock: UdpSocket,
// }
//
// impl Client {
//     pub async fn transact<'cl, 'tr, FGen, TrType, FHandle, Fut>(&'tr mut self, gen: FGen, handler: FHandle)
//     where
//         'tr: 'cl,
//         FGen: FnOnce(TrArgs<'tr>) -> Transaction<'tr, TrType>,
//         TrType: tr_types::StartTy,
//         FHandle: FnOnce(Transaction<'tr, TrType>) -> Fut,
//         Fut: Future<Output = Transaction<'tr, tr_types::Complete>> + 'cl,
//     {
//         let tr_init = gen(TrArgs { sock: &self.sock });
//         let _ = handler(tr_init).await;
//     }
// }
//
// #[tokio::test]
// async fn test_transact() {
//     let s = UdpSocket::bind("0.0.0.0:0").await.unwrap();
//     let mut c = Client { sock: s };
//     Transaction::ping(
//         |tr| tr.recv()
//     )
//     // c.transact(Transaction::ping, |tr| async move {
//     //     tr.if
//     // });
//     // let mut b = 1;
//     // c.transact(|mut tr| async move {
//     //     tr.b().await;
//     //     b = 2;
//     // }).await;
// }
//
// #[doc(hidden)]
// pub mod tr_types {
//     mod private {
//         pub trait Sealed {}
//     }
//     pub trait StartTy: self::Type {}
//     pub trait Type: private::Sealed {}
//     macro_rules! starttypes {
//         ($($t:ty),+) => {
//             $(
//             impl StartTy for $t {}
//             ),+
//         };
//     }
//     macro_rules! types {
//         ($($t:ty),+) => {
//             $(
//             impl Type for $t {}
//             impl private::Sealed for $t {}
//             )+
//         };
//     }
//     pub struct Default;
//     // starting types
//     pub struct Ping;// AllowTransaction
//     starttypes!(Ping);
//     // intermediate types
//     pub struct ServerTx;// ConfirmTransaction
//     // end types
//     pub struct Complete;
//     types!(Default, Ping, ServerTx, Complete);
// }
//
// pub struct TrArgs<'a> {
//     sock: &'a UdpSocket,
// }
//
// pub struct Transaction<'a, TrType: tr_types::Type = tr_types::Default> {
//     sock: &'a UdpSocket,
//     _type: PhantomData<TrType>
// }
//
// impl<'a> Transaction<'a, tr_types::Default> {
//     pub fn ping(args: TrArgs<'a>) -> Transaction<'a, tr_types::Ping> {
//         let TrArgs { sock } = args;
//         assert!(sock.peer_addr().is_ok(), "Socket must be connected");
//         todo!()
//     }
// }
//
// // allways called for `ping` transactions, the inner function may not get called if the server responds with `TransactionUnnecessary`
// impl<'a> Transaction<'a, tr_types::Ping> {
//     pub fn if_server_transmits<'cl, FHandle, Fut>(self, handler: FHandle) -> Transaction<'a, tr_types::Complete>
//     where
//         'a: 'cl,
//         FHandle: FnOnce(Self) -> Fut,
//         Fut: Future<Output = Transaction<'a, tr_types::Complete>> + 'cl
//     {
//         todo!()
//     }
// }
