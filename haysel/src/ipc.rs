//! IPC Bus integration

use std::{path::PathBuf, sync::Arc};

use mycelium::{
    station::{
        capabilities::{Channel, ChannelID, KnownChannels},
        identity::{KnownStations, StationID},
    },
    IPCError, IPCMsg,
};
use roundtable::{
    common::EV_SHUTDOWN,
    handler::{HandlerInit, LocalInterface, MethodRegister},
    handler_decl_t, method_decl_owned,
    msg::{self, HandlerInstance, Str},
};
use tokio::{
    io,
    net::{
        unix::{OwnedReadHalf, OwnedWriteHalf, SocketAddr},
        UnixListener, UnixStream,
    },
};

use crate::{
    dispatch::application::{Record, EV_WEATHER_DATA_RECEIVED},
    misc::Take,
    registry::{self, EV_META_NEW_CHANNEL, EV_META_NEW_STATION, EV_META_STATION_ASSOC_CHANNEL},
};

pub struct IPCNewConnections {
    listener: Arc<UnixListener>,
    registry: HandlerInstance,
}

impl IPCNewConnections {
    pub async fn new(path: PathBuf, registry: HandlerInstance) -> io::Result<Self> {
        Ok(Self {
            listener: Arc::new(UnixListener::bind(path)?),
            registry,
        })
    }

    async fn handle_new_client(
        &mut self,
        cli: io::Result<(UnixStream, SocketAddr)>,
        int: &LocalInterface,
    ) {
        match cli {
            Ok((stream, addr)) => {
                debug!("New IPC client connected from {addr:?}");
                let (read, write) = stream.into_split();
                let (stations, channels) = int
                    .query(self.registry.clone(), registry::EV_REGISTRY_QUERY_ALL, ())
                    .await
                    .expect("Failed to query registry");
                let conn = IPCConnection {
                    write,
                    read: Take::new(read),
                    addr,
                    init_known: Take::new((stations, channels)),
                };
                int.nonlocal.spawn(conn);
                self.bg_handle_new_client(int);
            }
            Err(io_err) => {
                error!("Listening for connections failed: {io_err:#}: new client connections will not continue to be accepted");
            }
        }
    }

    fn bg_handle_new_client(&mut self, int: &LocalInterface) {
        let li = self.listener.clone();
        int.bg_spawn(EV_PRIV_NEW_CONNECTION, async move { li.accept().await });
    }
}

#[async_trait]
impl HandlerInit for IPCNewConnections {
    const DECL: msg::HandlerType = handler_decl_t!("IPC New Connection Handler");
    async fn init(&mut self, int: &LocalInterface) {
        debug!("Launching IPC client listener");
        self.bg_handle_new_client(int);
    }
    // description of this handler instance
    fn describe(&self) -> Str {
        Str::Borrowed("IPC New Connection Handler")
    }
    // methods of this handler instance
    fn methods(&self, reg: &mut MethodRegister<Self>) {
        reg.register_owned(Self::handle_new_client, EV_PRIV_NEW_CONNECTION);
    }
}

method_decl_owned!(
    EV_PRIV_NEW_CONNECTION,
    io::Result<(UnixStream, SocketAddr)>,
    ()
);

pub struct IPCConnection {
    write: OwnedWriteHalf,
    read: Take<OwnedReadHalf>,
    addr: SocketAddr,
    init_known: Take<(KnownStations, KnownChannels)>,
}

impl IPCConnection {
    fn bg_read(&mut self, mut read: OwnedReadHalf, int: &LocalInterface) {
        int.bg_spawn(EV_PRIV_READ, async move {
            let res = mycelium::ipc_recv(&mut read).await;
            (read, res)
        })
    }

    async fn handle_read(
        &mut self,
        (read, res): (OwnedReadHalf, Result<IPCMsg, IPCError>),
        int: &LocalInterface,
    ) {
        match res {
            Ok(msg) => {
                trace!("IPC: Received {msg:?}");
                match msg.kind {
                    mycelium::IPCMsgKind::ClientDisconnect => {
                        debug!("IPC Client {:?} disconnected", self.addr);
                        warn!("IPC Client disconnected, but the task will not close (TODO/unimplemented)");
                        let _ = self
                            .send(&IPCMsg {
                                kind: mycelium::IPCMsgKind::Bye,
                            })
                            .await;
                    }
                    _other => self.bg_read(read, int),
                }
            }
            Err(e) => {
                error!("Failed to receive IPC message: {e} - no further attempts to read will be performed");
                self.read.put(read);
            }
        }
    }

    async fn send(&mut self, msg: &IPCMsg) -> Result<(), IPCError> {
        mycelium::ipc_send(&mut self.write, msg).await
    }

    async fn new_station(&mut self, &id: &StationID, _int: &LocalInterface) {
        self.send(&IPCMsg {
            kind: mycelium::IPCMsgKind::NewStation { id },
        })
        .await
        .expect("Failed to send `new station` message");
    }

    async fn new_channel(&mut self, (id, ch): &(ChannelID, Channel), _int: &LocalInterface) {
        self.send(&IPCMsg {
            kind: mycelium::IPCMsgKind::NewChannel {
                id: *id,
                ch: ch.clone(),
            },
        })
        .await
        .expect("Failed to send `new channel` message");
    }

    async fn station_new_channel(
        &mut self,
        (station, channel, _channel_info): &(StationID, ChannelID, Channel),
        _int: &LocalInterface,
    ) {
        self.send(&IPCMsg {
            kind: mycelium::IPCMsgKind::StationNewChannel {
                station: *station,
                channel: *channel,
            },
        })
        .await
        .expect("Failed to send `channel assoc` message");
    }

    async fn close(&mut self, _: &(), _int: &LocalInterface) {
        let _ = self
            .send(&IPCMsg {
                kind: mycelium::IPCMsgKind::Bye,
            })
            .await;
    }

    async fn send_data(&mut self, data: &Record, _int: &LocalInterface) {
        self.send(&IPCMsg {
            kind: mycelium::IPCMsgKind::FreshHotData {
                from: data.recorded_by,
                recorded_at: data.recorded_at,
                by_channel: data.data.clone(),
            },
        })
        .await
        .expect("Failed to send `new data received` message");
    }
}

#[async_trait]
impl HandlerInit for IPCConnection {
    const DECL: msg::HandlerType = handler_decl_t!("IPC Connection Handler");

    async fn init(&mut self, int: &LocalInterface) {
        let read = self.read.take();
        self.bg_read(read, int);
        let (stations, channels) = self.init_known.take();
        self.send(&IPCMsg {
            kind: mycelium::IPCMsgKind::Haiii { stations, channels },
        })
        .await
        .expect("Failed to send init packet");
    }
    // description of this handler instance
    fn describe(&self) -> Str {
        Str::Owned(format!("IPC Connection (to: {:?})", self.addr))
    }
    // methods of this handler instance
    fn methods(&self, reg: &mut MethodRegister<Self>) {
        reg.register_owned(Self::handle_read, EV_PRIV_READ);
        reg.register(Self::new_station, EV_META_NEW_STATION);
        reg.register(Self::new_channel, EV_META_NEW_CHANNEL);
        reg.register(Self::station_new_channel, EV_META_STATION_ASSOC_CHANNEL);
        reg.register(Self::send_data, EV_WEATHER_DATA_RECEIVED);
        reg.register(Self::close, EV_SHUTDOWN);
    }
}

method_decl_owned!(EV_PRIV_READ, (OwnedReadHalf, Result<IPCMsg, IPCError>), ());
