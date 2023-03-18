use std::mem;

use flume::{Receiver, Sender};
use tokio::{
    fs::File,
    io::{self, AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    select,
    sync::oneshot,
};
use tracing::{debug, error, instrument};
use zerocopy::{AsBytes, FromBytes};

use crate::tsdb::repr::Data;

use super::{
    errors::{AllocReqErr, AllocRunnerErr},
    ptr::Ptr,
    repr::{Header, SegHeader},
    set::SmallSet,
};

pub enum AllocRes {
    None,
    Read { data: Vec<u8> },
    Created { addr: u64 },
}

pub enum AllocReqKind {
    Read {
        addr: u64,
        len: usize,
        mark_used: bool,
    },
    Write {
        addr: u64,
        data: Vec<u8>,
        mark_unused: bool,
        allow_no_response: bool,
    },
    Create {
        size: u64,
    },
    Destroy {
        addr: u64,
    },
}

pub struct AllocReq {
    pub on_done: Sender<Result<AllocRes, AllocReqErr>>,
    pub req: AllocReqKind,
}

#[derive(Debug)]
pub struct AllocRunner {
    file: File,
    req_queue: Receiver<AllocReq>,
    close: oneshot::Receiver<()>,
    do_first_time_init: bool,
    /// addresses of currently existing `Obj` instances
    accesses: SmallSet<u64>,
    /// current header
    header: Header,
}

impl AllocRunner {
    #[instrument(name = "alloc_runner_init")]
    pub fn new(
        file: File,
        req_queue: Receiver<AllocReq>,
        close: oneshot::Receiver<()>,
        // run first-time initialization
        do_first_time_init: bool,
    ) -> Self {
        Self {
            file,
            req_queue,
            close,
            accesses: SmallSet::default(),
            do_first_time_init,
            header: FromBytes::new_zeroed(),
        }
    }

    #[instrument(name = "alloc_runner")]
    pub async fn run(mut self) -> Result<(), AllocRunnerErr> {
        debug!("Alloc runner task started");
        if self.do_first_time_init {
            self.file.set_len(0).await?;
            let header = Header {
                null_byte: 0xAB,
                _pad: [0; 7],
                entrypoint: Ptr::null(),
            };
            self.write_raw(0, &header).await?;
        }
        self.header = self.read_raw::<Header>(0).await?;
        loop {
            select! {
                msg = self.req_queue.recv_async() => {
                    match msg {
                        Ok(msg) => {
                            // self.handle(msg).await;
                        }
                        Err(_) => error!("Alloc runner stopping due to closed message queue"),
                    }
                },
                res = &mut self.close => {
                    match res {
                        Ok(()) => debug!("Stopping alloc runner"),
                        Err(_) => error!("Alloc runner stopping due to closed shutdown channel"),
                    }
                    break;
                }
            }
        }
        debug!("Alloc runner task stopped without errors");
        Ok(())
    }

    #[instrument(name = "alloc_req_handler", skip(on_done, req))]
    async fn handle(&mut self, AllocReq { on_done, req }: AllocReq) -> Result<(), AllocRunnerErr> {
        match req {
            AllocReqKind::Read {
                addr,
                len,
                mark_used,
            } => {
                let seg = self.read_raw::<SegHeader>(addr).await.unwrap();
            }
            AllocReqKind::Write {
                addr,
                data,
                mark_unused,
                allow_no_response,
            } => {}
            AllocReqKind::Create { size } => {}
            AllocReqKind::Destroy { addr } => {}
        }
        Ok(())
    }

    async fn write_raw<T: Data>(&mut self, addr: u64, val: &T) -> io::Result<()> {
        self.file.seek(io::SeekFrom::Start(addr)).await?;
        self.file.write_all(val.as_bytes()).await?;
        Ok(())
    }

    async fn read_raw<T: Data>(&mut self, addr: u64) -> io::Result<T> {
        self.file.seek(io::SeekFrom::Start(addr)).await?;
        let mut buf = vec![0u8; mem::size_of::<T>()];
        self.file.read_exact(&mut buf).await?;
        Ok(T::read_from(buf.as_slice()).unwrap())
    }
}
