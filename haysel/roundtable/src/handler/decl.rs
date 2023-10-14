use std::{fmt::Debug, marker::PhantomData};

use uuid::Uuid;

use crate::handler::async_fn_ptr::HandlerCallableErased;
#[cfg(feature = "bus_dbg")]
use crate::msg::Str;

pub struct MethodDecl<const OWN: bool, At: 'static, Rt: 'static> {
    pub(crate) id: Uuid,
    pub(crate) desc: &'static str,
    _ph: PhantomData<&'static (At, Rt)>,
}

impl<const OWN: bool, At: 'static, Rt: 'static> Clone for MethodDecl<OWN, At, Rt> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<const OWN: bool, At: 'static, Rt: 'static> Copy for MethodDecl<OWN, At, Rt> {}

impl<const OWN: bool, At: 'static, Rt: 'static> MethodDecl<OWN, At, Rt> {
    #[doc(hidden)]
    pub const fn new(desc: &'static str, id: Uuid) -> Self {
        Self {
            id,
            desc,
            _ph: PhantomData,
        }
    }
}

/// Describes the (non-ID portion) of a method, incl its handler function
pub struct MethodRaw {
    pub handler_func: Box<(dyn HandlerCallableErased + Sync + Send)>,
    #[cfg(feature = "bus_dbg")]
    pub handler_desc: Str,
}

impl Debug for MethodRaw {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MethodRaw")
            .field("handler_desc", &self.handler_desc)
            .finish_non_exhaustive()
    }
}
