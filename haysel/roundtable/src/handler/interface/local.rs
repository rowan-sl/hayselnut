use futures::{
    future::{pending, BoxFuture},
    Future,
};
use uuid::Uuid;

use crate::{
    dyn_var::DynVar,
    flag::Flag,
    handler::{decl::MethodDecl, dispatch::DispatchErr, interface::Interface},
    msg::{self, HandlerInstance},
};

pub struct LocalInterface {
    pub nonlocal: Interface,
    pub(crate) bg_spawner: flume::Sender<(BoxFuture<'static, DynVar>, Uuid, &'static str)>,
    pub(crate) update_metadata: Flag,
    pub(crate) shutdown: Flag,
    pub(crate) instance: HandlerInstance,
    pub(crate) message_source: Option<HandlerInstance>,
}

impl LocalInterface {
    /// runs `f` to completion, allowing other events to be processed in the meantime. when F completes,
    /// an event (with decl `m`) is generated *for this handler only* containing the results.
    ///
    /// This can be used for a pattern where, for example a socket's receive half is put into a background task,
    /// waits to receive, then returns itself + what it received, and finally the handler spawns the task again.
    pub fn bg_spawn<T: Sync + Send>(
        &self,
        m: MethodDecl<true, T, ()>,
        f: impl Future<Output = T> + Send + 'static,
    ) {
        let dyn_f: BoxFuture<'static, DynVar> = Box::pin(async move { DynVar::new(f.await) });
        let MethodDecl { id, desc, .. } = m;
        if let Err(..) = self.bg_spawner.send((dyn_f, id, desc)) {
            unreachable!("Failed to spawn background runner - handler runtime not listening");
        }
    }

    pub async fn shutdown(&self) -> ! {
        self.shutdown.signal();
        pending().await
    }

    #[allow(dead_code)]
    pub fn update_metadata(&self) {
        self.update_metadata.signal();
    }

    pub fn whoami(&self) -> HandlerInstance {
        self.instance.clone()
    }

    pub fn event_source(&self) -> HandlerInstance {
        self.message_source
            .clone()
            .expect("event_source called in a non-event context")
    }

    pub async fn query<At: Sync + Send + 'static, Rt: 'static>(
        &self,
        target: HandlerInstance,
        method: MethodDecl<false, At, Rt>,
        args: At,
    ) -> Result<Rt, DispatchErr> {
        self.nonlocal
            .query_as(self.whoami(), target, method, args)
            .await
    }

    pub async fn dispatch<At: Sync + Send + 'static, Rt: 'static>(
        &self,
        target: HandlerInstance,
        method: MethodDecl<false, At, Rt>,
        args: At,
    ) -> Result<(), DispatchErr> {
        self.nonlocal
            .dispatch_as(self.whoami(), target, method, args)
            .await
    }

    pub async fn announce<At: Sync + Send + 'static, Rt: 'static>(
        &self,
        target: msg::Target,
        method: MethodDecl<false, At, Rt>,
        args: At,
    ) -> Result<(), DispatchErr> {
        self.nonlocal
            .announce_as(self.whoami(), target, method, args)
            .await
    }
}
