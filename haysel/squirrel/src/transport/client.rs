use std::{net::SocketAddr, future::Future, marker::PhantomData};

use tokio::net::UdpSocket;
use uuid::Uuid;

use super::{
    frame::Frame,
    controll::{Cmd, CmdPacket},
    packet::{self, extract_packet_type},
};

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
