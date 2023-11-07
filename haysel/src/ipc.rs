//! IPC Bus integration

use std::{convert::Infallible, path::PathBuf, sync::Arc};

use chrono::Utc;
use mycelium::{
    station::{
        capabilities::{Channel, ChannelID, KnownChannels},
        identity::{KnownStations, StationID},
    },
    IPCError, IPCMsg,
};
use roundtable::{
    handler::{DispatchErr, HandlerInit, LocalInterface, MethodRegister},
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
    tsdb2::{bus::EV_DB_QUERY, query::builder::QueryBuilder},
};

pub struct IPCNewConnections {
    listener: Arc<UnixListener>,
    registry: HandlerInstance,
    database: HandlerInstance,
}

impl IPCNewConnections {
    pub async fn new(
        path: PathBuf,
        registry: HandlerInstance,
        database: HandlerInstance,
    ) -> io::Result<Self> {
        Ok(Self {
            listener: Arc::new(UnixListener::bind(path)?),
            registry,
            database,
        })
    }

    async fn handle_new_client(
        &mut self,
        cli: io::Result<(UnixStream, SocketAddr)>,
        int: &LocalInterface,
    ) -> Result<(), Infallible> {
        match cli {
            Ok((stream, addr)) => {
                debug!("New IPC client connected from {addr:?}");
                let (read, write) = stream.into_split();
                let (stations, channels) = match int
                    .query(self.registry.clone(), registry::EV_REGISTRY_QUERY_ALL, ())
                    .await
                {
                    Ok(x) => x,
                    Err(e) => {
                        error!("Failed to query registry ({e:#}) - ipc task will now exit");
                        return int.shutdown().await;
                    }
                };
                let conn = IPCConnection {
                    write,
                    read: Take::new(read),
                    addr,
                    init_known: Take::new((stations, channels)),
                    database: self.database.clone(),
                };
                int.nonlocal.spawn(conn);
                self.bg_handle_new_client(int);
            }
            Err(io_err) => {
                error!("Listening for connections failed: {io_err:#}: ipc task will now exit");
                return int.shutdown().await;
            }
        }
        Ok(())
    }

    fn bg_handle_new_client(&mut self, int: &LocalInterface) {
        let li = self.listener.clone();
        int.bg_spawn(EV_PRIV_NEW_CONNECTION, async move { li.accept().await });
    }
}

#[async_trait]
impl HandlerInit for IPCNewConnections {
    const DECL: msg::HandlerType = handler_decl_t!("IPC New Connection Handler");
    type Error = Infallible;
    async fn init(&mut self, int: &LocalInterface) -> Result<(), Infallible> {
        debug!("Launching IPC client listener");
        self.bg_handle_new_client(int);
        Ok(())
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

#[derive(Debug, thiserror::Error)]
pub enum IPCConnectionErr {
    #[error("IPC comm error: {0:#}")]
    IPC(#[from] IPCError),
    #[error("Dispatch error: {0:#}")]
    Dispatch(#[from] DispatchErr),
    #[error("Failed to query database: {0:#}")]
    DBQuery(anyhow::Error),
}

pub struct IPCConnection {
    write: OwnedWriteHalf,
    read: Take<OwnedReadHalf>,
    addr: SocketAddr,
    init_known: Take<(KnownStations, KnownChannels)>,
    database: HandlerInstance,
}

impl IPCConnection {
    fn bg_read(&mut self, mut read: OwnedReadHalf, int: &LocalInterface) {
        int.bg_spawn(EV_PRIV_READ, async move {
            let res = mycelium::ipc_recv::<IPCMsg>(&mut read).await;
            (read, res)
        })
    }

    async fn handle_read(
        &mut self,
        (read, res): (OwnedReadHalf, Result<IPCMsg, IPCError>),
        int: &LocalInterface,
    ) -> Result<(), IPCConnectionErr> {
        self.read.put(read);
        let msg = res?;
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
            mycelium::IPCMsgKind::QueryLastHourOf { station, channel } => {
                let from_time = Utc::now();
                let data = int
                    .query(
                        self.database.clone(),
                        EV_DB_QUERY,
                        QueryBuilder::new_nodb()
                            .with_station(station)
                            .with_channel(channel)
                            .with_after(from_time - chrono::Duration::minutes(60))
                            .verify()
                            .unwrap(),
                    )
                    .await?
                    .map_err(|e| IPCConnectionErr::DBQuery(e))?;
                self.send(&IPCMsg {
                    kind: mycelium::IPCMsgKind::QueryLastHourResponse { data, from_time },
                })
                .await?;
                let read = self.read.take();
                self.bg_read(read, int);
            }
            _other => {
                let read = self.read.take();
                self.bg_read(read, int);
            }
        }
        Ok(())
    }

    async fn send(&mut self, msg: &IPCMsg) -> Result<(), IPCError> {
        mycelium::ipc_send(&mut self.write, msg).await
    }

    async fn new_station(
        &mut self,
        &id: &StationID,
        _int: &LocalInterface,
    ) -> Result<(), IPCConnectionErr> {
        self.send(&IPCMsg {
            kind: mycelium::IPCMsgKind::NewStation { id },
        })
        .await?;
        Ok(())
    }

    async fn new_channel(
        &mut self,
        (id, ch): &(ChannelID, Channel),
        _int: &LocalInterface,
    ) -> Result<(), IPCConnectionErr> {
        self.send(&IPCMsg {
            kind: mycelium::IPCMsgKind::NewChannel {
                id: *id,
                ch: ch.clone(),
            },
        })
        .await?;
        Ok(())
    }

    async fn station_new_channel(
        &mut self,
        (station, channel, _channel_info): &(StationID, ChannelID, Channel),
        _int: &LocalInterface,
    ) -> Result<(), IPCConnectionErr> {
        self.send(&IPCMsg {
            kind: mycelium::IPCMsgKind::StationNewChannel {
                station: *station,
                channel: *channel,
            },
        })
        .await?;
        Ok(())
    }

    #[allow(dead_code)]
    async fn close(&mut self, _: &(), _int: &LocalInterface) {
        let _ = self
            .send(&IPCMsg {
                kind: mycelium::IPCMsgKind::Bye,
            })
            .await;
    }

    async fn send_data(
        &mut self,
        data: &Record,
        _int: &LocalInterface,
    ) -> Result<(), IPCConnectionErr> {
        self.send(&IPCMsg {
            kind: mycelium::IPCMsgKind::FreshHotData {
                from: data.recorded_by,
                recorded_at: data.recorded_at,
                by_channel: data.data.clone(),
            },
        })
        .await?;
        Ok(())
    }
}

#[async_trait]
impl HandlerInit for IPCConnection {
    const DECL: msg::HandlerType = handler_decl_t!("IPC Connection Handler");
    type Error = IPCConnectionErr;
    async fn init(&mut self, int: &LocalInterface) -> Result<(), IPCConnectionErr> {
        let read = self.read.take();
        self.bg_read(read, int);
        let (stations, channels) = self.init_known.take();
        self.send(&IPCMsg {
            kind: mycelium::IPCMsgKind::Haiii { stations, channels },
        })
        .await?;
        Ok(())
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
    }
    async fn on_error(&mut self, error: IPCConnectionErr, int: &LocalInterface) {
        error!(
            "Error occured in IPC conenction {} - {error:#} - connection will shut down",
            self.describe()
        );
        int.shutdown().await
    }
}

method_decl_owned!(EV_PRIV_READ, (OwnedReadHalf, Result<IPCMsg, IPCError>), ());
