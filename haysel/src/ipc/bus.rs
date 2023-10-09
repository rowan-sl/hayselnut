//! IPC Bus integration

use std::{cell::Cell, sync::Arc};

use mycelium::{IPCError, IPCMsg};
use tokio::{
    io,
    net::{unix::SocketAddr, UnixListener, UnixStream},
};

use crate::bus::{
    handler::{handler_decl_t, method_decl, HandlerInit, LocalInterface, MethodRegister},
    msg::Str,
};

struct IPCNewConnections {
    listener: Arc<UnixListener>,
}

impl IPCNewConnections {
    async fn handle_new_client(
        &mut self,
        cli: &Cell<(UnixStream, SocketAddr)>,
        int: &LocalInterface,
    ) {
        todo!()
    }
}

#[async_trait]
impl HandlerInit for IPCNewConnections {
    const DECL: crate::bus::msg::HandlerType = handler_decl_t!("IPC New Connection Handler");
    // type BgGenerated = io::Result<(UnixStream, SocketAddr)>;
    // const BG_RUN: bool = true;
    // async fn bg_generate(&mut self) -> Self::BgGenerated {
    //     self.listener.accept().await
    // }
    // async fn bg_consume(&mut self, args: Self::BgGenerated, mut int: LocalInterface) {
    //     match args {
    //         Ok((stream, addr)) => {
    //             let conn = IPCConnection {
    //                 stream,
    //                 addr,
    //                 buffer: vec![],
    //                 buf_amnt: 0usize,
    //             };
    //             int.nonlocal.spawn(conn).await;
    //         }
    //         Err(io_err) => {
    //             error!("Listening for connections failed: {io_err:#}: new client connections will not continue to be accepted");
    //             int.bg_pause();
    //         }
    //     }
    // }
    async fn init(&mut self, int: &LocalInterface) {}
    // description of this handler instance
    fn describe(&self) -> Str {
        Str::Borrowed("IPC New Connection Handler")
    }
    // methods of this handler instance
    fn methods(&self, _register: &mut MethodRegister<Self>) {}
}

method_decl!(EV_PRIV_NEW_CONNECTION, Cell<(UnixStream, SocketAddr)>, ());

struct IPCConnection {
    stream: UnixStream,
    addr: SocketAddr,
    buffer: Vec<u8>,
    buf_amnt: usize,
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
