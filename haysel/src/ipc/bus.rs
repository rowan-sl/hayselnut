//! IPC Bus integration

use std::sync::Arc;

use mycelium::{IPCError, IPCMsg};
use tokio::{
    io,
    net::{
        unix::{OwnedReadHalf, OwnedWriteHalf, SocketAddr},
        UnixListener, UnixStream,
    },
};

use crate::{
    bus::{
        handler::{handler_decl_t, method_decl_owned, HandlerInit, LocalInterface, MethodRegister},
        msg::Str,
    },
    util::Take,
};

struct IPCNewConnections {
    listener: Arc<UnixListener>,
}

impl IPCNewConnections {
    async fn handle_new_client(
        &mut self,
        cli: io::Result<(UnixStream, SocketAddr)>,
        int: &LocalInterface,
    ) {
        match cli {
            Ok((stream, addr)) => {
                let (read, write) = stream.into_split();
                let conn = IPCConnection {
                    write,
                    read: Take::new(read),
                    addr,
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
    const DECL: crate::bus::msg::HandlerType = handler_decl_t!("IPC New Connection Handler");
    async fn init(&mut self, int: &LocalInterface) {
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

struct IPCConnection {
    #[allow(unused)]
    write: OwnedWriteHalf,
    read: Take<OwnedReadHalf>,
    addr: SocketAddr,
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
            Ok(_msg) => {
                todo!();
                self.bg_read(read, int);
            }
            Err(e) => {
                error!("Failed to receive IPC message: {e} - no further attempts to read will be performed");
                self.read.put(read);
            }
        }
    }
}

#[async_trait]
impl HandlerInit for IPCConnection {
    const DECL: crate::bus::msg::HandlerType = handler_decl_t!("IPC Connection Handler");

    async fn init(&mut self, int: &LocalInterface) {
        let read = self.read.take();
        self.bg_read(read, int);
    }
    // description of this handler instance
    fn describe(&self) -> Str {
        Str::Owned(format!("IPC Connection (to: {:?})", self.addr))
    }
    // methods of this handler instance
    fn methods(&self, reg: &mut MethodRegister<Self>) {
        reg.register_owned(Self::handle_read, EV_PRIV_READ);
    }
}

method_decl_owned!(EV_PRIV_READ, (OwnedReadHalf, Result<IPCMsg, IPCError>), ());
