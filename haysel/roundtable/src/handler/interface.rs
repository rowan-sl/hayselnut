use std::{
    any::type_name,
    sync::{atomic::AtomicU64, Arc},
};

use anyhow::Result;
use tokio::sync::broadcast;

use crate::{
    dyn_var::DynVar,
    handler::{
        decl::MethodDecl, dispatch::bus_dispatch_event, runtime::HandlerTaskRt, HandlerInit,
    },
    msg::{self, HandlerInstance, Msg, Str},
};

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
            }
        });
        inst
    }

    pub async fn dispatch_as<At: Sync + Send + 'static, Rt: 'static>(
        &self,
        source: HandlerInstance,
        target: msg::Target,
        method: MethodDecl<false, At, Rt>,
        args: At,
    ) -> Result<Option<Rt>> {
        if let Some(ret) = bus_dispatch_event(
            self.clone(),
            source,
            target,
            msg::MethodID {
                id: method.id,
                id_desc: Str::Borrowed(method.desc),
            },
            DynVar::new(args),
        )
        .await?
        {
            match ret.try_to() {
                Ok(ret) => Ok(Some(ret)),
                Err(ret) => {
                    error!(
                        "Mismatched return type - expected {}, found {}",
                        type_name::<Rt>(),
                        ret.type_name()
                    );
                    bail!("Mismatched return type");
                }
            }
        } else {
            Ok(None)
        }
    }
}