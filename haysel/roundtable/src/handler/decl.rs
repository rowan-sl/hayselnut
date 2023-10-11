use std::marker::PhantomData;

use uuid::Uuid;

#[cfg(feature = "bus_dbg")]
use crate::msg::Str;
use crate::{handler::async_fn_ptr::HandlerCallableErased, id::const_uuid_v4};

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
    pub const fn new(desc: &'static str) -> Self {
        Self {
            id: const_uuid_v4(),
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
