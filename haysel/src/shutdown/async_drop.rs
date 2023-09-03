use futures::{pending, Future};
use tokio::runtime;

use crate::shutdown;

use super::ShutdownHandle;

pub struct AsyncDrop {
    handle: runtime::Handle,
    shutdown: Option<shutdown::ShutdownHandle>,
}

impl AsyncDrop {
    pub async fn new(shutdown: ShutdownHandle) -> Self {
        pending!();
        Self {
            handle: runtime::Handle::current(),
            shutdown: Some(shutdown),
        }
    }

    /// may be called ONCE
    pub fn run<Res: Send + 'static, Fut: Future<Output = Res> + Send + 'static>(
        &mut self,
        drop_fn: Fut,
    ) {
        let (handle, shutdown) = (self.handle.clone(), self.shutdown.take());
        // because this could be running on a non-runtime thread, we use the handle provided to
        // enter the runtime with which we can then run the async drop function
        let guard = handle.enter();
        // this relies on the runtime waiting for all tasks to end before shutting down (which
        // should happen)
        handle.spawn(async {
            drop_fn.await;
            drop(shutdown);
        });
        drop(guard);
    }
}
