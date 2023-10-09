//! Communication with clients (weather stations)

use std::{collections::HashMap, net::SocketAddr, time::Duration};

use squirrel::transport::{
    server::{ClientInterface, ClientMetadata, DispatchEvent},
    Packet,
};
use tokio::net::UdpSocket;
use uuid::Uuid;

use crate::bus::{
    handler::{handler_decl_t, method_decl, HandlerInit, LocalInterface, MethodRegister},
    msg::{self, HandlerInstance, Str},
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

pub struct TransportClient {
    // controller instance
    ctrl: HandlerInstance,
    // external handler to notify of data received events.
    ext: HandlerInstance,
    addr: SocketAddr,
    inter: ClientInterface,
}

// request by an external handler of `TransportClient` to queue `data` to be
// sent to its associated station
method_decl!(EV_TRANS_CLI_QUEUE_DATA, Vec<u8>, ());

// event sent by a `TransportClient` to an external handler when a full group of data is received.
method_decl!(EV_TRANS_CLI_DATA_RECVD, Vec<u8>, ());

// TransportClient notification of timeout (sent to all)
// method_decl!(EV_TRANS_CLI_TIMED_OUT, (), ());

// TransportClient requests Controller to send `Packet` to the address associeted
// (through `active_clients_inv` with the sending handler)
method_decl!(EV_TRANS_CLI_REQ_SEND_PKT, Packet, ());

#[async_trait]
impl HandlerInit for TransportClient {
    const DECL: crate::bus::msg::HandlerType = handler_decl_t!("Weather station interface");
    async fn init(&mut self, int: &LocalInterface) {}
    fn describe(&self) -> Str {
        Str::Owned(format!("Weather station interface for {:?}", self.addr))
    }
    fn methods(&self, reg: &mut MethodRegister<Self>) {
        reg.register(Self::queue_data, EV_TRANS_CLI_QUEUE_DATA);
        reg.register(Self::handle_pkt, EV_CONTROLLER_RECEIVED);
    }
}

impl TransportClient {
    pub async fn new(
        addr: SocketAddr,
        max_trans_t: Duration,
        controller: HandlerInstance,
        ext: HandlerInstance,
    ) -> Self {
        Self {
            ctrl: controller,
            ext,
            addr,
            inter: ClientInterface::new(max_trans_t, addr, ClientMetadata::default()),
        }
    }

    async fn queue_data(&mut self, data: &Vec<u8>, _int: &LocalInterface) {
        self.inter.queue(data.clone());
    }

    async fn handle_pkt(&mut self, pkt: &Packet, int: &LocalInterface) {
        for ev in self.inter.handle(*pkt) {
            match ev {
                DispatchEvent::TimedOut => {
                    warn!(
                        "Connection to weather station {} at {:?} timed out",
                        self.inter.access_metadata().uuid.unwrap_or(Uuid::nil()),
                        self.addr,
                    );
                }
                DispatchEvent::Send(pkt) => {
                    int.dispatch(
                        msg::Target::Instance(self.ctrl.clone()),
                        EV_TRANS_CLI_REQ_SEND_PKT,
                        pkt,
                    )
                    .await;
                }
                DispatchEvent::Received(pkt) => {
                    int.dispatch(
                        msg::Target::Instance(self.ext.clone()),
                        EV_TRANS_CLI_DATA_RECVD,
                        pkt,
                    )
                    .await;
                }
            }
        }
    }
}
