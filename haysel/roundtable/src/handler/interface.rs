use std::{
    any::type_name,
    sync::{atomic::AtomicU64, Arc},
};

use tokio::sync::broadcast;

#[cfg(feature = "bus_dbg")]
use crate::msg::Str;
use crate::{
    dyn_var::DynVar,
    handler::{
        decl::MethodDecl, dispatch::bus_dispatch_event, runtime::HandlerTaskRt, HandlerInit,
    },
    msg::{self, HandlerInstance, Msg},
};

use super::dispatch::DispatchErr;

pub mod local;

#[derive(Clone)]
pub struct Interface {
    /// source for generating uids (faster than Uuid::new_v4, since it only requires a single
    /// fetch_add instruction)
    pub(crate) uid_src: Arc<AtomicU64>,
    /// Queue that is used for ALL inter-handler/task communication. ALL of it.
    ///
    /// Arc is used to avoid cloning a (large) Msg value that will never need writing to
    /// TODO: arena allocate Msg?
    pub(crate) comm: broadcast::Sender<Arc<Msg>>,
}

impl Interface {
    pub fn spawn<H: HandlerInit>(&self, instance: H) -> HandlerInstance {
        let inter = self.clone();
        let rt = HandlerTaskRt::new(inter, instance);
        let inst = rt.id();
        tokio::spawn(async move {
            let res = rt.run().await;
            if let Err(e) = res {
                error!("Runtime task exited with error: {e:#}");
            } else {
                trace!("Runtime task exited");
            }
        });
        inst
    }

    /// Dispatch, no verification, no response
    pub async fn announce_as<At: Sync + Send + 'static, Rt: 'static>(
        &self,
        source: HandlerInstance,
        target: msg::Target,
        method: MethodDecl<false, At, Rt>,
        args: At,
    ) -> Result<(), DispatchErr> {
        let _ = bus_dispatch_event(
            self.clone(),
            source,
            target,
            msg::MethodID {
                id: method.id,
                #[cfg(feature = "bus_dbg")]
                id_desc: Str::Borrowed(method.desc),
            },
            DynVar::new(args),
            false,
            false,
        )
        .await?;
        Ok(())
    }

    /// Dispatch, verifies that the event was handled, no response
    pub async fn dispatch_as<At: Sync + Send + 'static, Rt: 'static>(
        &self,
        source: HandlerInstance,
        target: HandlerInstance,
        method: MethodDecl<false, At, Rt>,
        args: At,
    ) -> Result<(), DispatchErr> {
        let _ = bus_dispatch_event(
            self.clone(),
            source,
            msg::Target::Instance(target),
            msg::MethodID {
                id: method.id,
                #[cfg(feature = "bus_dbg")]
                id_desc: Str::Borrowed(method.desc),
            },
            DynVar::new(args),
            false,
            true,
        )
        .await?;
        Ok(())
    }

    /// Dispatch, returns the response
    pub async fn query_as<At: Sync + Send + 'static, Rt: 'static>(
        &self,
        source: HandlerInstance,
        target: HandlerInstance,
        method: MethodDecl<false, At, Rt>,
        args: At,
    ) -> Result<Rt, DispatchErr> {
        let Some(ret) = bus_dispatch_event(
            self.clone(),
            source,
            msg::Target::Instance(target),
            msg::MethodID {
                id: method.id,
                #[cfg(feature = "bus_dbg")]
                id_desc: Str::Borrowed(method.desc),
            },
            DynVar::new(args),
            true,
            true,
        )
        .await?
        else {
            unreachable!("Expected, but did not receive a response")
        };
        match ret.try_to() {
            Ok(ret) => Ok(ret),
            Err(ret) => {
                error!(
                    "Mismatched return type - expected {}, found {}",
                    type_name::<Rt>(),
                    ret.type_name()
                );
                unreachable!("Mismatched return type");
            }
        }
    }
}
