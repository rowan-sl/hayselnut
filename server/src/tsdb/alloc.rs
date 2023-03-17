use std::path::Path;

use flume::Sender;
use tokio::{
    io, fs::OpenOptions,
    sync::oneshot,
    task::{self, JoinHandle},
};
use tracing::{instrument, debug};

mod set; 
mod types;
mod obj;
mod ptr;
mod repr;
pub mod errors;
mod runner;

use super::repr::Data;
use runner::{AllocRunner, AllocReq, AllocReqKind, AllocRes};
use errors::{AllocErr, AllocReqErr, AllocRunnerErr};
pub use ptr::Ptr;
pub use obj::Obj;
pub use repr::entrypoint_pointer;

#[derive(Debug)]
pub struct Alloc {
    close: oneshot::Sender<()>,
    runner: JoinHandle<Result<(), AllocRunnerErr>>,
    req_queue: Sender<AllocReq>,
}

impl Alloc {
    /// Create a new allocator from the given path.
    ///
    /// if the path does not exist, a new database will be created
    #[instrument]
    pub async fn open(path: &Path) -> Result<Alloc, AllocErr> {
        debug!("Opening new db storage file");
        let create_new = !path.try_exists().map_err(AllocRunnerErr::from)?;
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(path.with_extension("tsdb"))
            .await.map_err(AllocRunnerErr::from)?;
        let (req_queue, req_queue_recv) = flume::unbounded();
        let (close, close_recv) = oneshot::channel();
        let runner = task::spawn(AllocRunner::new(
            file,
            req_queue_recv,
            close_recv,
            create_new,
        ).run());
        Ok(Self { close, runner, req_queue })
    }

    #[instrument]
    pub async fn close(self) -> Result<(), AllocErr> {
        let _ = self.close.send(());
        let _res = self.runner.await.unwrap()?;
        Ok(())
    }

    /// Create an `Obj` instance from a pointer.
    /// 
    /// `Obj`s are handles to a allocated piece of data, keeping exlusive access for the `Obj`s lifetime
    /// while allowing reading and writing to the underlying data.
    ///
    /// - the pointer must have come from this allocator        (this *may* error, or will return unspecified data)
    /// - the object must not have been previously deallocated  (this *may* error, or will return unspecified data)
    ///
    /// - no other objects must currently exist for this data   (this will error) 
    /// - the pointer must not be null!                         (this will error)
    #[instrument]
    pub async fn get<'a, T: Data>(&'a self, ptr: Ptr<T>) -> Result<Obj<'a, T>, AllocReqErr> {
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
        let AllocRes::Read { data } = recv.recv_async().await.unwrap()? else {
            unreachable!("wrong response for request!");
        };
        Ok(Obj {
            alloc: self,
            addr: ptr.addr,
            val: T::read_from(data.as_slice()).unwrap(),
        })
    }

    /// Create a new `Obj`, initialized with the specified data.
    /// to obtain a pointer for this allocation, use `Obj::{get,into}_ptr`
    ///
    /// for creating an allocator, see `Alloc::open_{new,existing}`
    pub async fn alloc<'a, T: Data>(&'a self, val: T) -> Result<Obj<'a, T>, AllocReqErr> {
        let (on_done, recv) = flume::bounded(1);
        self.req_queue.try_send(AllocReq {
            on_done,
            req: AllocReqKind::Create { size: std::mem::size_of::<T>() as u64 }
        }).unwrap();
        let AllocRes::Created { addr } = recv.recv_async().await.unwrap()? else {
            unreachable!("wrong response for request");
        };
        Ok(Obj {
            alloc: self,
            addr,
            val
        })
    }

    /// Deallocate the given object (passed by pointer), freeing its memory for future use.
    /// 
    /// - no `Obj` must currently exist for this pointer 
    /// - the pointer must have come from this allocator 
    /// - the pointer must not have been deallocated before.
    pub async fn free<'a, T: Data>(&'a self, ptr: Ptr<T>) -> Result<(), AllocReqErr> {
        let (on_done, recv) = flume::bounded(1);
        self.req_queue.try_send(AllocReq {
            on_done,
            req: AllocReqKind::Destroy { addr: ptr.addr }
        }).unwrap();
        let AllocRes::None = recv.recv_async().await.unwrap()? else {
            unreachable!("wrong response for request");
        };
        Ok(())
    }

    /// attempt to sync an `Obj`, adding the Write call to the queue
    /// without waiting for a response. if the sync fails, the allocator
    /// will make its best attempt deal with the problem, but there is no guarentees.
    ///
    /// this is only to be called by `Obj::drop`, ONCE.
    #[instrument(skip(obj))]
    fn attempt_sync<'a, T: Data>(obj: &mut Obj<'a, T>) {
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

