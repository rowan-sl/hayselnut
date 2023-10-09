//! Communication with clients (weather stations)

use std::{collections::HashMap, net::SocketAddr};

use squirrel::transport::Packet;
use tokio::net::UdpSocket;

pub mod transport;

use crate::bus::{
    handler::{handler_decl_t, method_decl, HandlerInit, LocalInterface, MethodRegister},
    msg::{HandlerInstance, Str},
};

pub use transport::{
    TransportClient, EV_TRANS_CLI_DATA_RECVD, EV_TRANS_CLI_QUEUE_DATA, EV_TRANS_CLI_REQ_SEND_PKT,
};

pub struct Controller {
    sock: UdpSocket,
    active_clients: HashMap<SocketAddr, HandlerInstance>,
    active_clients_inv: HashMap<HandlerInstance, SocketAddr>,
}

// sent by `Controller` to the relevant `TransportClient` when it receives a packet
// (target determined using `active_clients`)
method_decl!(EV_CONTROLLER_RECEIVED, Packet, ());

#[async_trait]
impl HandlerInit for Controller {
    const DECL: crate::bus::msg::HandlerType = handler_decl_t!("Weather station interface");
    async fn init(&mut self, int: &LocalInterface) {}
    fn describe(&self) -> Str {
        Str::Owned(format!(
            "Weather station socket controller on {:?}",
            self.sock.local_addr()
        ))
    }
    fn methods(&self, reg: &mut MethodRegister<Self>) {}
}
