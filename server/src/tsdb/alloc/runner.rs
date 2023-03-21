use std::{mem, num::NonZeroU64};

use derivative::Derivative;
use flume::{Receiver, Sender};
use tokio::{
    fs::File,
    io::{self, AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    select,
    sync::oneshot,
};
use tracing::{debug, info, error, instrument, trace, warn};
use zerocopy::{AsBytes, FromBytes};

use crate::tsdb::repr::Data;

use super::{
    entrypoint_pointer,
    errors::{AllocReqErr, AllocRunnerErr},
    ptr::{Ptr, NonNull},
    repr::{Header, SegHeader},
    set::SmallSet,
};

#[derive(Derivative)]
#[derivative(Debug)]
pub enum AllocRes {
    None,
    Read {
        #[derivative(Debug = "ignore")]
        data: Vec<u8>,
    },
    Created {
        ptr: NonNull<()>,
    },
}

/// all addr fields here are the address of the HEADER in the file, not the data
#[derive(Derivative)]
#[derivative(Debug)]
pub enum AllocReqKind {
    Read {
        ptr: NonNull<()>,
        len: usize,
        mark_used: bool,
    },
    Write {
        ptr: NonNull<()>,
        #[derivative(Debug = "ignore")]
        data: Vec<u8>,
        mark_unused: bool,
        allow_no_response: bool,
    },
    Create {
        size: u64,
    },
    Destroy {
        ptr: NonNull<()>,
    },
}

#[derive(Derivative)]
#[derivative(Debug)]
pub struct AllocReq {
    #[derivative(Debug = "ignore")]
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
    accesses: SmallSet<NonZeroU64>,
    /// current location of the bump allocator
    alloc_addr: u64,
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
            // starts after the header
            // updated at the start of the run function
            alloc_addr: 0,
            header: FromBytes::new_zeroed(),
        }
    }

    #[instrument(name = "alloc_runner", skip(self))]
    pub async fn run(mut self) -> Result<(), AllocRunnerErr> {
        debug!("Alloc runner task started");
        if self.do_first_time_init {
            self.file.set_len(0).await?;
            let header = Header {
                null_byte: 0xAB,
                _pad: [0; 7],
                alloc_addr: mem::size_of::<Header>() as u64,
                entrypoint: Ptr::null(),
            };
            self.write(0, &header).await?;
        }
        self.header = self.read::<Header>(0).await?;
        self.alloc_addr = self.header.alloc_addr;
        match self.run_inner().await {
            Ok(()) => {
                info!("Running shutdown code");
                if let Err(w_err) = self.write(0, &self.header.clone()).await {
                    error!("Failed to write header to disk, DB may be corrupt:\n{w_err:?}");
                }
            }
            Err(e) => {
                info!("Attempting to run shutdown code");
                if let Err(w_err) = self.write(0, &self.header.clone()).await {
                    error!("Failed to write header to disk, DB may be corrupt:\n{w_err:?}");
                }
                return Err(e);
            }
        }
        debug!("Alloc runner task stopped without errors");
        Ok(())
    }

    async fn run_inner(&mut self) -> Result<(), AllocRunnerErr> {
        loop {
            select! {
                msg = self.req_queue.recv_async() => {
                    match msg {
                        Ok(msg) => {
                            debug!("Handle: {msg:?}");
                            match self.handle(msg).await {
                                Ok(()) => {}
                                Err(e) => {
                                    error!("Runner error in handling: {e:?}");
                                    return Err(e.into());
                                }
                            }
                        }
                        Err(_) => {
                            error!("Alloc runner stopping due to closed message queue");
                            return Err(AllocRunnerErr::CommQueueClosed);
                        },
                    }
                },
                res = &mut self.close => {
                    match res {
                        Ok(()) => {
                            debug!("Stopping alloc runner");
                            break Ok(());
                        }
                        Err(_) => {
                            error!("Alloc runner stopping due to closed shutdown channel");
                            return Err(AllocRunnerErr::CommQueueClosed);
                        }
                    }
                }
            }
        }
    }

    #[instrument(name = "alloc_req_handler", skip(on_done, self))]
    async fn handle(&mut self, AllocReq { on_done, req }: AllocReq) -> Result<(), AllocRunnerErr> {
        match req {
            AllocReqKind::Read {
                ptr,
                len,
                mark_used,
            } => {
                let respond = |res| async {
                    match &res {
                        Ok(ok) => trace!("Response: {ok:?}"),
                        Err(err) => error!("Error: {err:?}"),
                    }
                    on_done
                        .send_async(res)
                        .await
                        .map_err(|_| AllocRunnerErr::ResFail)
                };
                if self.accesses.contains(&ptr.addr()) && mark_used {
                    respond(Err(AllocReqErr::DoubleUse)).await?;
                    return Ok(());
                }
                if ptr.addr() == entrypoint_pointer::<()>().addr() {
                    if len != mem::size_of::<Ptr<()>>() {
                        respond(Err(AllocReqErr::SizeMismatch)).await?;
                        return Ok(());
                    }
                    respond(Ok(AllocRes::Read {
                        data: self.header.entrypoint.as_bytes().to_vec(),
                    }))
                    .await?;
                    if mark_used {
                        self.accesses.insert(ptr.addr());
                    }
                    return Ok(());
                }
                //TODO: make a table of valid addresses to access
                let seg = self.read::<SegHeader>(ptr.addr().into()).await?;
                trace!("Read header: {seg:?}");
                if seg.free.into() {
                    respond(Err(AllocReqErr::UseAfterFree)).await?;
                    return Ok(());
                }
                if len as u64 != seg.len_this {
                    respond(Err(AllocReqErr::SizeMismatch)).await?;
                    return Ok(());
                }
                // read
                let buf = match self
                    .read_raw(ptr.addr().get() + mem::size_of::<SegHeader>() as u64, len)
                    .await
                {
                    Ok(buf) => buf,
                    Err(e) => {
                        let _ = respond(Err(AllocReqErr::InternalError)).await;
                        return Err(e.into());
                    }
                };
                respond(Ok(AllocRes::Read { data: buf })).await?;
                if mark_used {
                    self.accesses.insert(ptr.addr());
                }
            }
            AllocReqKind::Write {
                ptr,
                data,
                mark_unused,
                allow_no_response,
            } => {
                // FIXME: log errors ignored by allow_no_response
                let respond = |res| async {
                    on_done
                        .send_async(res)
                        .await
                        .map_or_else(
                            |e| if allow_no_response { Ok(()) } else { Err(e) },
                            |_| Ok(()),
                        )
                        .map_err(|_| AllocRunnerErr::ResFail)
                };
                if ptr.addr() == entrypoint_pointer::<()>().addr() {
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
                        self.accesses.remove(&ptr.addr());
                    }
                    return Ok(());
                }
                //TODO: make a table of valid addresses to access
                let seg = self.read::<SegHeader>(ptr.addr().into()).await?;
                trace!("Read header: {seg:?}");
                if seg.free.into() {
                    respond(Err(AllocReqErr::UseAfterFree)).await?;
                    return Ok(());
                }
                if data.len() as u64 != seg.len_this {
                    respond(Err(AllocReqErr::SizeMismatch)).await?;
                    return Ok(());
                }
                match self
                    .write_raw(ptr.addr().get() + mem::size_of::<SegHeader>() as u64, &data)
                    .await
                {
                    Ok(()) => respond(Ok(AllocRes::None)).await?,
                    Err(e) => {
                        respond(Err(AllocReqErr::InternalError)).await?;
                        return Err(e.into());
                    }
                }
                if mark_unused {
                    if !self.accesses.contains(&ptr.addr()) {
                        warn!("Unneded mark_unused flag")
                    }
                    self.accesses.remove(&ptr.addr());
                }
            }
            AllocReqKind::Create { size } => {
                let respond = |res| async {
                    on_done
                        .send_async(res)
                        .await
                        .map_err(|_| AllocRunnerErr::ResFail)
                };
                let seg_addr = self.alloc_addr;
                let val_addr = self.alloc_addr + mem::size_of::<SegHeader>() as u64;
                self.alloc_addr = val_addr + size;
                let seg = SegHeader {
                    len_this: size,
                    free: false.into(),
                    _pad: [0u8; 7],
                };
                match self.write(seg_addr, &seg).await {
                    Ok(()) => {
                        trace!("created allocation of size {size} at {seg_addr}.\nnew allocation marked as used, assuming Obj creation");
                        let addr = NonZeroU64::new(seg_addr).unwrap();
                        self.accesses.insert(addr);
                        respond(Ok(AllocRes::Created { ptr: NonNull::with_addr(addr) })).await?;
                    }
                    Err(e) => {
                        respond(Err(AllocReqErr::InternalError)).await?;
                        return Err(e.into());
                    }
                }
            }
            AllocReqKind::Destroy { ptr } => {
                let respond = |res| async {
                    on_done
                        .send_async(res)
                        .await
                        .map_err(|_| AllocRunnerErr::ResFail)
                };
                if self.accesses.contains(&ptr.addr()) {
                    respond(Err(AllocReqErr::Used)).await?;
                    return Ok(());
                }
                //TODO: make a table of valid addresses to access
                let mut seg = self.read::<SegHeader>(ptr.addr().into()).await?;
                if seg.free.into() {
                    respond(Err(AllocReqErr::DoubleFree)).await?;
                    return Ok(());
                }
                // its literally that simple lol
                seg.free = true.into();
                self.write::<SegHeader>(ptr.addr().into(), &seg).await?;
                respond(Ok(AllocRes::None)).await?;
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
