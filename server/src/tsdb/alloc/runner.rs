use std::mem;

use derivative::Derivative;
use flume::{Receiver, Sender};
use tokio::{
    fs::File,
    io::{self, AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    select,
    sync::oneshot,
};
use tracing::{trace, debug, warn, error, instrument};
use zerocopy::{AsBytes, FromBytes};

use crate::tsdb::repr::Data;

use super::{
    errors::{AllocReqErr, AllocRunnerErr},
    ptr::Ptr,
    repr::{Header, SegHeader},
    set::SmallSet, entrypoint_pointer,
};

pub enum AllocRes {
    None,
    Read { data: Vec<u8> },
    Created { addr: u64 },
}

#[derive(Derivative)]
#[derivative(Debug)]
pub enum AllocReqKind {
    Read {
        addr: u64,
        len: usize,
        mark_used: bool,
    },
    Write {
        addr: u64,
        #[derivative(Debug="ignore")]
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

#[derive(Derivative)]
#[derivative(Debug)]
pub struct AllocReq {
    #[derivative(Debug="ignore")]
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
            self.write(0, &header).await?;
        }
        self.header = self.read::<Header>(0).await?;
        loop {
            select! {
                msg = self.req_queue.recv_async() => {
                    match msg {
                        Ok(msg) => {
                            trace!("Handle: {msg:?}"); 
                            self.handle(msg).await?;
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
                let respond = |res| async {
                    on_done.send_async(res)
                    .await
                    .map_err(|_| AllocRunnerErr::ResFail)
                };
                if addr == 0 {
                    respond(Err(AllocReqErr::NullPointer)).await?;
                    return Ok(());
                }
                if self.accesses.contains(&addr) && mark_used {
                    respond(Err(AllocReqErr::DoubleUse)).await?;
                    return Ok(());
                }
                if addr == entrypoint_pointer::<()>().addr {
                    if len != mem::size_of::<Ptr<()>>() {
                        respond(Err(AllocReqErr::SizeMismatch)).await?;
                        return Ok(());
                    }
                    respond(Ok(AllocRes::Read { data: self.header.entrypoint.as_bytes().to_vec() } )).await?;
                    if mark_used {
                        self.accesses.insert(addr);
                    }
                    return Ok(());
                }
                //TODO: make a table of valid addresses to access
                let seg = self.read::<SegHeader>(addr).await?;
                if seg.free.into() {
                    respond(Err(AllocReqErr::UseAfterFree)).await?;
                    return Ok(());
                }
                if len as u64 != seg.len_this {
                    respond(Err(AllocReqErr::SizeMismatch)).await?;
                    return Ok(());
                }
                // read 
                let buf = match self.read_raw(addr, len).await {
                    Ok(buf) => buf,
                    Err(e) => {
                        let _ = respond(Err(AllocReqErr::InternalError)).await;
                        return Err(e.into());
                    }
                };
                respond(Ok(AllocRes::Read { data: buf })).await?;
                if mark_used {
                    self.accesses.insert(addr);
                }
            }
            AllocReqKind::Write {
                addr,
                data,
                mark_unused,
                allow_no_response,
            } => {
                // FIXME: log errors ignored by allow_no_response
                let respond = |res| async {
                    on_done.send_async(res)
                    .await
                    .map_or_else(|e| if allow_no_response { Ok(()) } else { Err(e) }, |_| Ok(()))
                    .map_err(|_| AllocRunnerErr::ResFail )
                };
                if addr == 0 {
                    respond(Err(AllocReqErr::NullPointer)).await?;
                    return Ok(());
                }
                if addr == entrypoint_pointer::<()>().addr {
                    if data.len() != mem::size_of::<Ptr<()>>() {
                        respond(Err(AllocReqErr::SizeMismatch)).await?;
                        return Ok(());
                    }
                    self.header.entrypoint = Ptr::<()>::read_from(data.as_slice()).unwrap();
                    let h = self.header;
                    match self.write(0, &h).await {
                        Ok(()) => {}
                        Err(e) => {
                            respond(Err(AllocReqErr::InternalError)).await?;
                            return Err(e.into());
                        }
                    }
                    respond(Ok(AllocRes::None)).await?;
                    if mark_unused {
                        self.accesses.remove(&addr);
                    }
                    return Ok(());
                }
                //TODO: make a table of valid addresses to access
                let seg = self.read::<SegHeader>(addr).await?;
                if seg.free.into() {
                    respond(Err(AllocReqErr::UseAfterFree)).await?;
                    return Ok(());
                }
                if data.len() as u64 != seg.len_this {
                    respond(Err(AllocReqErr::SizeMismatch)).await?;
                    return Ok(());
                }
                match self.write_raw(addr, &data).await {
                    Ok(()) => respond(Ok(AllocRes::None)).await?,
                    Err(e) => {
                        respond(Err(AllocReqErr::InternalError)).await?;
                        return Err(e.into());
                    }
                }
                if mark_unused {
                    if !self.accesses.contains(&addr) {
                        warn!("Unneded mark_unused flag")
                    }
                    self.accesses.remove(&addr);
                }
            }
            AllocReqKind::Create { size } => {
                todo!()
            }
            AllocReqKind::Destroy { addr } => {
                let respond = |res| async {
                    on_done.send_async(res)
                    .await
                    .map_err(|_| AllocRunnerErr::ResFail)
                };
                if addr == 0 {
                    respond(Err(AllocReqErr::NullPointer)).await?;
                    return Ok(());
                }
                if self.accesses.contains(&addr) {
                    respond(Err(AllocReqErr::Used)).await?;
                    return Ok(());
                }
                //TODO: make a table of valid addresses to access
                let mut seg = self.read::<SegHeader>(addr).await?;
                if seg.free.into() {
                    respond(Err(AllocReqErr::DoubleFree)).await?;
                    return Ok(());
                }
                // its literally that simple lol
                seg.free = true.into();
                self.write::<SegHeader>(addr, &seg).await?;
            }
        }
        Ok(())
    }

    async fn write<T: Data>(&mut self, addr: u64, val: &T) -> io::Result<()> {
        self.file.seek(io::SeekFrom::Start(addr)).await?;
        self.file.write_all(val.as_bytes()).await?;
        Ok(())
    }

    async fn write_raw(&mut self, addr: u64, buf: &[u8]) -> io::Result<()> {
        self.file.seek(io::SeekFrom::Start(addr)).await?;
        self.file.write_all(&buf).await?;
        Ok(())
    }

    async fn read<T: Data>(&mut self, addr: u64) -> io::Result<T> {
        self.file.seek(io::SeekFrom::Start(addr)).await?;
        let mut buf = vec![0u8; mem::size_of::<T>()];
        self.file.read_exact(&mut buf).await?;
        Ok(T::read_from(buf.as_slice()).unwrap())
    }

    async fn read_raw(&mut self, addr: u64, num_bytes: usize) -> io::Result<Vec<u8>> {
        self.file.seek(io::SeekFrom::Start(addr)).await?;
        let mut buf = vec![0u8; num_bytes];
        self.file.read_exact(&mut buf).await?;
        Ok(buf)
    }
}
