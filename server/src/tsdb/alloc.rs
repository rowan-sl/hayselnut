use std::{
    marker::PhantomData,
    mem,
    ops::{Deref, DerefMut},
    path::Path,
};

use derivative::Derivative;
use flume::{Receiver, Sender};
use tokio::{
    fs::{File, OpenOptions},
    io::{self, AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::oneshot,
    task::{self, JoinHandle},
};
use tracing::{span, instrument, Level, Instrument, info_span};
use zerocopy::{AsBytes, FromBytes};

use super::repr::Data;

#[derive(Debug, thiserror::Error)]
pub enum AllocErr {
    #[error("I/O Error: {0:?}")]
    IOError(#[from] io::Error),
}
enum AllocRes {
    Read { data: Vec<u8> },
}
enum AllocReqKind {
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
}

struct AllocReq {
    on_done: Sender<Result<AllocRes, AllocErr>>,
    req: AllocReqKind,
}

#[derive(Debug)]
pub struct Alloc {
    close: oneshot::Sender<()>,
    runner: JoinHandle<io::Result<()>>,
    req_queue: Sender<AllocReq>,
}

impl Alloc {
    #[instrument]
    pub async fn open_new(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(path.with_extension(".tsdb"))
            .await?;
        let (req_queue, req_queue_recv) = flume::unbounded();
        let (close, close_recv) = oneshot::channel();
        let runner = task::spawn(async move {
            // addresses of currently existing `Obj` instances
            let accesses: Vec<u64> = vec![];
            Ok(())
        }.instrument(info_span!("alloc_runner")));
        Ok(Self { close, runner, req_queue })
    }

    #[instrument]
    pub async fn close(self) -> io::Result<()> {
        let _ = self.close.send(());
        let _res = self.runner.await.unwrap()?;
        Ok(())
    }

    #[instrument]
    pub async fn substantiate<'a, T: Data>(&'a self, ptr: Ptr<T>) -> Result<Obj<'a, T>, AllocErr> {
        let (on_done, recv) = flume::bounded(1);
        // not a bounded channel
        self.req_queue
            .try_send(AllocReq {
                on_done,
                req: AllocReqKind::Read {
                    addr: ptr.addr,
                    len: ptr.pointee_size(),
                    mark_used: true,
                },
            })
            .unwrap();
        #[allow(irrefutable_let_patterns)]
        let AllocRes::Read { data } = recv.recv_async().await.unwrap()? else {
            unreachable!("wrong response for request!");
        };
        Ok(Obj {
            alloc: self,
            addr: ptr.addr,
            val: T::read_from(data.as_slice()).unwrap(),
        })
    }

    /// attempt to drop `Obj`, adding the Write call to the queue
    /// without waiting for a response. if the sync fails, the allocator
    /// will make its best attempt deal with the problem, but there is no guarentees.
    ///
    /// this is only to be called by `Obj::drop`, ONCE.
    #[instrument(skip(obj))]
    fn attempt_drop<'a, T: Data>(obj: &mut Obj<'a, T>) {
        // create a response queue, immedietally drop the receiving end
        let (on_done, _) = flume::bounded(1);
        obj.alloc
            .req_queue
            .try_send(AllocReq {
                on_done,
                req: AllocReqKind::Write {
                    addr: obj.addr,
                    data: obj.val.as_bytes().to_vec(),
                    mark_unused: true,
                    allow_no_response: true,
                },
            })
            .unwrap();
    }
}

#[derive(Derivative)]
#[derivative(Debug(bound=""))]
#[repr(transparent)]
pub struct Ptr<T> {
    pub addr: u64,
    _ph0: PhantomData<*const T>,
}

impl<T> Ptr<T> {
    pub const fn null() -> Self {
        Self {
            addr: 0,
            _ph0: PhantomData,
        }
    }
    pub const fn with_addr(addr: u64) -> Self {
        Self {
            addr,
            _ph0: PhantomData,
        }
    }
    pub const fn is_null(self) -> bool {
        self.addr == 0
    }
    pub const fn pointee_size(self) -> usize {
        mem::size_of::<T>()
    }
}

impl<T> Clone for Ptr<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Ptr<T> {}

// heheheheheheheh
unsafe impl<T> FromBytes for Ptr<T> {
    fn only_derive_is_allowed_to_implement_this_trait()
    where
        Self: Sized,
    {
    }
}
unsafe impl<T> AsBytes for Ptr<T> {
    fn only_derive_is_allowed_to_implement_this_trait()
    where
        Self: Sized,
    {
    }
}

#[derive(Derivative)]
#[derivative(Debug)]
pub struct Obj<'a, T: Data> {
    #[derivative(Debug="ignore")]
    alloc: &'a Alloc,
    addr: u64,
    /// current value (not synced to disk)
    val: T,
}

impl<'a, T: Data> Obj<'a, T> {
    // all function here should not take self, but take Self as a normal param -- like Box

    pub fn get_ptr(obj: &Self) -> Ptr<T> {
        Ptr {
            addr: obj.addr,
            _ph0: PhantomData,
        }
    }

    pub fn into_ptr(obj: Self) -> Ptr<T> {
        let p = Self::get_ptr(&obj);
        // runs sync if necessary
        drop(obj);
        p
    }
}

impl<'a, T: Data> Deref for Obj<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.val
    }
}

impl<'a, T: Data> DerefMut for Obj<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.val
    }
}

impl<'a, T: Data> Drop for Obj<'a, T> {
    fn drop(&mut self) {
        Alloc::attempt_drop(self)
    }
}
