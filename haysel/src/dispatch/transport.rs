use std::{net::SocketAddr, time::Duration};

use squirrel::transport::{
    server::{ClientInterface, DispatchEvent},
    Packet,
};

use roundtable::{
    handler::{DispatchErr, HandlerInit, LocalInterface, MethodRegister},
    handler_decl_t, method_decl,
    msg::{self, HandlerInstance, Str},
};

pub struct TransportClient {
    // controller instance
    ctrl: HandlerInstance,
    // external handler to notify of data received events.
    ext: Option<HandlerInstance>,
    addr: SocketAddr,
    inter: ClientInterface,
    // data received while `self.ext` is None
    missed_events: Vec<Vec<u8>>,
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

// Controller notifies TransportClient of the identity of its
// associated Application level client
method_decl!(EV_TRANS_CLI_IDENT_APP, HandlerInstance, ());

#[async_trait]
impl HandlerInit for TransportClient {
    const DECL: msg::HandlerType = handler_decl_t!("Weather station interface [transport]");
    type Error = DispatchErr;
    async fn init(&mut self, _int: &LocalInterface) -> Result<(), DispatchErr> {
        Ok(())
    }
    fn describe(&self) -> Str {
        Str::Owned(format!(
            "Weather station interface [transport] for {:?}",
            self.addr
        ))
    }
    fn methods(&self, reg: &mut MethodRegister<Self>) {
        reg.register(Self::queue_data, EV_TRANS_CLI_QUEUE_DATA);
        reg.register(Self::handle_pkt, super::EV_CONTROLLER_RECEIVED);
        reg.register(Self::ident_appl, EV_TRANS_CLI_IDENT_APP);
    }
    async fn on_error(&mut self, error: DispatchErr, int: &LocalInterface) {
        error!(
            "Handler {} experienced an error - failed to dispatch: {error:#?} (exiting)",
            self.describe()
        );
        int.shutdown().await;
    }
}

impl TransportClient {
    pub fn new(addr: SocketAddr, max_trans_t: Duration, controller: HandlerInstance) -> Self {
        Self {
            ctrl: controller,
            ext: None,
            addr,
            inter: ClientInterface::new(max_trans_t),
            missed_events: vec![],
        }
    }

    async fn ident_appl(
        &mut self,
        app: &HandlerInstance,
        int: &LocalInterface,
    ) -> Result<(), <Self as HandlerInit>::Error> {
        debug!(
            "sending {} missed events to newly received application client",
            self.missed_events.len()
        );
        self.ext = Some(app.clone());
        for pkt in self.missed_events.drain(..) {
            int.dispatch(app.clone(), EV_TRANS_CLI_DATA_RECVD, pkt)
                .await?;
        }
        Ok(())
    }

    async fn queue_data(
        &mut self,
        data: &Vec<u8>,
        _int: &LocalInterface,
    ) -> Result<(), <Self as HandlerInit>::Error> {
        self.inter.queue(data.clone());
        Ok(())
    }

    async fn handle_pkt(
        &mut self,
        pkt: &Packet,
        int: &LocalInterface,
    ) -> Result<(), <Self as HandlerInit>::Error> {
        for ev in self.inter.handle(*pkt) {
            match ev {
                DispatchEvent::TimedOut => {
                    warn!("Connection to weather station at {:?} timed out", self.addr,);
                }
                DispatchEvent::Send(pkt) => {
                    int.dispatch(self.ctrl.clone(), EV_TRANS_CLI_REQ_SEND_PKT, pkt)
                        .await?;
                }
                DispatchEvent::Received(pkt) => {
                    if let Some(ext) = self.ext.clone() {
                        int.dispatch(ext, EV_TRANS_CLI_DATA_RECVD, pkt).await?;
                    } else {
                        warn!("Transport received message, but has no assocated application client to send to");
                        self.missed_events.push(pkt);
                    }
                }
            }
        }
        Ok(())
    }
}
