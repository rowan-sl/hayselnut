//! Communication with clients (weather stations)

use std::{collections::HashMap, convert::Infallible, net::SocketAddr, sync::Arc, time::Duration};

use squirrel::transport::{server::recv_next_packet, Packet};
use tokio::{io, net::UdpSocket};

pub mod application;
pub mod transport;

use roundtable::{
    handler::{HandlerInit, LocalInterface, MethodRegister},
    handler_decl_t, method_decl, method_decl_owned,
    msg::{self, HandlerInstance, Str},
};

pub use transport::{
    TransportClient, EV_TRANS_CLI_DATA_RECVD, EV_TRANS_CLI_QUEUE_DATA, EV_TRANS_CLI_REQ_SEND_PKT,
};

use application::AppClient;
use transport::EV_TRANS_CLI_IDENT_APP;

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
    const DECL: msg::HandlerType = handler_decl_t!("Weather station interface [controller]");
    type Error = Infallible;
    async fn init(&mut self, int: &LocalInterface) -> Result<(), Self::Error> {
        self.recv_next(int);
        Ok(())
    }
    fn describe(&self) -> Str {
        Str::Owned(format!(
            "Weather station socket controller on {:?}",
            self.sock.local_addr()
        ))
    }
    fn methods(&self, reg: &mut MethodRegister<Self>) {
        reg.register_owned(Self::handle_receved, EV_PRIV_CONTROLLER_RECEIVED);
        reg.register(Self::send_packet, EV_TRANS_CLI_REQ_SEND_PKT);
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

    #[instrument(skip(self, int))]
    fn recv_next(&mut self, int: &LocalInterface) {
        let sock = self.sock.clone();
        int.bg_spawn(EV_PRIV_CONTROLLER_RECEIVED, async move {
            let pkt = recv_next_packet(&sock).await;
            trace!("controller: received [transport] packet");
            pkt
        })
    }

    #[instrument(skip(self, pkt, int))]
    async fn send_packet(
        &mut self,
        pkt: &Packet,
        int: &LocalInterface,
    ) -> Result<(), <Self as HandlerInit>::Error> {
        let Some(addr) = self.active_clients_inv.get(&int.event_source()) else {
            error!("Controller::send_packet used by a handler that was not one of its clients - the event will be ignored");
            return Ok(());
        };
        trace!("Sending packet {pkt:?} to {addr:?}");
        self.sock
            .send_to(pkt.as_bytes(), addr)
            .await
            .expect("Failed to send data");
        Ok(())
    }

    fn insert_active_client(&mut self, addr: SocketAddr, instance: HandlerInstance) {
        self.active_clients.insert(addr, instance.clone());
        self.active_clients_inv.insert(instance, addr);
    }

    #[instrument(skip(self, res, int))]
    async fn handle_receved(
        &mut self,
        res: io::Result<Option<(SocketAddr, Packet)>>,
        int: &LocalInterface,
    ) -> Result<(), <Self as HandlerInit>::Error> {
        match res {
            Ok(Some((addr, pkt))) => {
                trace!("Received packet {pkt:?} from {addr:?}");
                let target = if self.active_clients.contains_key(&addr) {
                    self.active_clients.get(&addr).unwrap().clone()
                } else {
                    debug!("New client interfaces created for {addr:?}");
                    let trans_cli = TransportClient::new(addr, self.max_trans_t, int.whoami());
                    let trans_cli_inst = int.nonlocal.spawn(trans_cli);
                    let appl_cli = AppClient::new(
                        addr,
                        int.whoami(),
                        trans_cli_inst.clone(),
                        self.registry.clone(),
                    );
                    let appl_cli_inst = int.nonlocal.spawn(appl_cli);
                    int.dispatch(
                        trans_cli_inst.clone(),
                        EV_TRANS_CLI_IDENT_APP,
                        appl_cli_inst,
                    )
                    .await
                    .unwrap();
                    self.insert_active_client(addr, trans_cli_inst.clone());
                    trans_cli_inst
                };
                int.dispatch(target, EV_CONTROLLER_RECEIVED, pkt)
                    .await
                    .unwrap();
                self.recv_next(int);
                Ok(())
            }
            Ok(None) => {
                debug!("Received datagram, but it did not contain a packet");
                self.recv_next(int);
                Ok(())
            }
            Err(err) => {
                error!("Failed to receive data from udp socket: {err:?} - no further attempts will be made to read data for ANY weather station");
                int.shutdown().await
            }
        }
    }
}
