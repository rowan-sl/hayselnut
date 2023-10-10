//! Communication with clients (weather stations)

use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

use squirrel::transport::{server::recv_next_packet, Packet};
use tokio::{io, net::UdpSocket};

pub mod application;
pub mod transport;

use crate::bus::{
    handler::{
        handler_decl_t, method_decl, method_decl_owned, HandlerInit, LocalInterface, MethodRegister,
    },
    msg::{self, HandlerInstance, Str},
};

pub use transport::{
    TransportClient, EV_TRANS_CLI_DATA_RECVD, EV_TRANS_CLI_QUEUE_DATA, EV_TRANS_CLI_REQ_SEND_PKT,
};

use self::{application::AppClient, transport::EV_TRANS_CLI_IDENT_APP};

pub struct Controller {
    sock: Arc<UdpSocket>,
    active_clients: HashMap<SocketAddr, HandlerInstance>,
    active_clients_inv: HashMap<HandlerInstance, SocketAddr>,
    max_trans_t: Duration,
    registry: HandlerInstance,
}

// sent by `Controller` to the relevant `TransportClient` when it receives a packet
// (target determined using `active_clients`)
method_decl!(EV_CONTROLLER_RECEIVED, Packet, ());

method_decl_owned!(
    EV_PRIV_CONTROLLER_RECEIVED,
    io::Result<Option<(SocketAddr, Packet)>>,
    ()
);

#[async_trait]
impl HandlerInit for Controller {
    const DECL: crate::bus::msg::HandlerType = handler_decl_t!("Weather station interface");
    async fn init(&mut self, int: &LocalInterface) {
        let sock = self.sock.clone();
        int.bg_spawn(EV_PRIV_CONTROLLER_RECEIVED, async move {
            recv_next_packet(&sock).await
        })
    }
    fn describe(&self) -> Str {
        Str::Owned(format!(
            "Weather station socket controller on {:?}",
            self.sock.local_addr()
        ))
    }
    fn methods(&self, reg: &mut MethodRegister<Self>) {
        reg.register_owned(Self::handle_receved, EV_PRIV_CONTROLLER_RECEIVED);
    }
}

impl Controller {
    pub fn new(sock: UdpSocket, max_trans_t: Duration, registry: HandlerInstance) -> Self {
        Self {
            sock: Arc::new(sock),
            active_clients: HashMap::new(),
            active_clients_inv: HashMap::new(),
            max_trans_t,
            registry,
        }
    }

    fn insert_active_client(&mut self, addr: SocketAddr, instance: HandlerInstance) {
        self.active_clients.insert(addr, instance.clone());
        self.active_clients_inv.insert(instance, addr);
    }

    async fn handle_receved(
        &mut self,
        res: io::Result<Option<(SocketAddr, Packet)>>,
        int: &LocalInterface,
    ) {
        match res {
            Ok(Some((addr, pkt))) => {
                let target = if self.active_clients.contains_key(&addr) {
                    self.active_clients.get(&addr).unwrap().clone()
                } else {
                    let trans_cli = TransportClient::new(
                        addr,
                        self.max_trans_t,
                        int.whoami(),
                    );
                    let trans_cli_inst = int.nonlocal.spawn(trans_cli);
                    let appl_cli = AppClient::new(
                        addr,
                        int.whoami(),
                        trans_cli_inst.clone(),
                        self.registry.clone(),
                    );
                    let appl_cli_inst = int.nonlocal.spawn(appl_cli);
                    int.dispatch(
                        msg::Target::Instance(trans_cli_inst.clone()),
                        EV_TRANS_CLI_IDENT_APP,
                        appl_cli_inst,
                    ).await.unwrap();
                    self.insert_active_client(addr, trans_cli_inst.clone());
                    trans_cli_inst
                };
                int.dispatch(msg::Target::Instance(target), EV_CONTROLLER_RECEIVED, pkt).await.unwrap();
            }
            Ok(None) => debug!("Received datagram, but it did not contain a packet"),
            Err(err) => error!("Failed to receive data from udp socket: {err:?} - no further attempts will be made to read data for ANY weather station"),
        }
    }
}
