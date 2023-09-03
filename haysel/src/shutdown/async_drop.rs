use futures::pending;
use std::ops::{Deref, DerefMut};
use tokio::runtime;

use crate::shutdown;

#[async_trait]
pub trait AsyncDrop {
    async fn drop(self);
}

pub struct AsyncDropBox<T: AsyncDrop + Send + 'static> {
    inner: Option<T>,
    handle: runtime::Handle,
    shutdown: Option<shutdown::ShutdownHandle>,
}

impl<T: AsyncDrop + Send + 'static> AsyncDropBox<T> {
    /// manually run async_drop, ensuring that its execution finishes before this function returns
    pub async fn manual_drop(mut self) {
        <T as AsyncDrop>::drop(self.inner.take().unwrap()).await
    }
}

impl<T: AsyncDrop + Send + 'static> Deref for AsyncDropBox<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().unwrap()
    }
}

impl<T: AsyncDrop + Send + 'static> DerefMut for AsyncDropBox<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut().unwrap()
    }
}

impl<T: AsyncDrop + Send + 'static> Drop for AsyncDropBox<T> {
    fn drop(&mut self) {
        let inner = self.inner.take().unwrap();
        let (handle, shutdown) = (self.handle.clone(), self.shutdown.take());
        // because this could be running on a non-runtime thread, we use the handle provided to
        // enter the runtime with which we can then run the async drop function
        let guard = handle.enter();
        // this relies on the runtime waiting for all tasks to end before shutting down (which
        // should happen)
        handle.spawn(async {
            <T as AsyncDrop>::drop(inner).await;
            drop(shutdown);
        });
        drop(guard);
    }
}

impl super::Shutdown {
    pub async fn wrap_async_drop<T: AsyncDrop + Send + 'static>(&self, val: T) -> AsyncDropBox<T> {
        pending!();
        let shutdown = self.handle();
        let handle = runtime::Handle::current();
        AsyncDropBox {
            inner: Some(val),
            shutdown: Some(shutdown),
            handle,
        }
    }
}
