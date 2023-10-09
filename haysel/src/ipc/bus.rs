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
        handler::{
            handler_decl_t, method_decl, method_decl_owned, HandlerInit, LocalInterface,
            MethodRegister,
        },
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
                    read: Take::new((read, vec![], 0)),
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
    write: OwnedWriteHalf,
    read: Take<(OwnedReadHalf, Vec<u8>, usize)>,
    addr: SocketAddr,
}

impl IPCConnection {
    pub fn bg_read(&mut self, read: OwnedReadHalf, int: &LocalInterface) {
        // int.bg_spawn(EV_PRIV_READ, async move {read.try_read
        //     let res = mycelium::ipc_recv(&mut read).await;
        // })
    }
}

#[async_trait]
impl HandlerInit for IPCConnection {
    const DECL: crate::bus::msg::HandlerType = handler_decl_t!("IPC Connection Handler");
    // type BgGenerated = Result<IPCMsg, IPCError>;
    // const BG_RUN: bool = true;
    // async fn bg_generate(&mut self) -> Self::BgGenerated {
    //     mycelium::ipc_recv_cancel_safe(
    //         &mut self.buffer,
    //         &mut self.buf_amnt,
    //         &mut self.stream
    //     ).await
    // }
    // async fn bg_consume(&mut self, args: Self::BgGenerated, int: LocalInterface) {
    //     todo!()
    // }
    // description of this handler instance
    fn describe(&self) -> Str {
        Str::Owned(format!("IPC Connection (to: {:?})", self.addr))
    }
    // methods of this handler instance
    fn methods(&self, _register: &mut MethodRegister<Self>) {}
}

method_decl!(EV_PRIV_READ, Result<IPCMsg, IPCError>, ());
