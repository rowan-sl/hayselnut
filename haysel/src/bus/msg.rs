use std::borrow::Cow;

use super::atomic_cell::AtomicCell;
use super::dyn_var::DynVar;
use uuid::Uuid;

use crate::flag::Flag;

use super::id::Uid;

#[derive(Debug)]
pub struct Msg {
    /// UID - generated at message send time
    pub id: Uid,
    /// content of the message
    pub kind: MsgKind,
}

#[derive(Debug)]
pub enum MsgKind {
    /// A request of one or more handlers
    Request {
        /// the handler instance that is sending this request
        source: HandlerInstance,
        /// the handler(s) this request is sent to
        target: Target,
        /// the 'method' on the handler being requested (note that method ids being used across
        /// handlers will imply that bolth handlers implement the given method)
        ///
        /// if a handler that matches `target` does NOT implement
        /// `method`, it will be ignored and should not handle the request
        method: MethodID,
        /// arguments of the request.
        arguments: DynVar,
        /// the response channel (if None, no response is desired)
        /// this *must* be None when using Target::(Type | Any)
        response: Option<Responder>,
    },
}

/// type commonly used in bus_dbg variables. can be &'static str or String
pub type Str = Cow<'static, str>;

/// the ID used to identify a particular handler on a method (const UUID)
#[derive(Debug)]
pub struct MethodID {
    /// the UUID of this method
    pub id: Uuid,
    /// debug-only description of the method
    #[cfg(feature = "bus_dbg")]
    pub id_desc: Str,
}

/// describe a type of handler (UUID, a constant associated with that handler) (similar to a struct's type)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HandlerType {
    /// the UUID of this type
    pub id: Uuid,
    /// debug-only description of the type
    #[cfg(feature = "bus_dbg")]
    pub id_desc: Str,
}

/// describe an instance of a spacific handler type (similar to a struct instance)
/// (UID, associated with an instance)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HandlerInstance {
    /// the UUID of the handler type
    pub typ: HandlerType,
    /// the UID of this instance
    pub discriminant: Uid,
    /// debug-only description of the instance
    #[cfg(feature = "bus_dbg")]
    pub discriminant_desc: Str,
}

/// a channel used for sending a single response to a query.
#[derive(Debug)]
pub struct Responder {
    /// the response value. when a handler wants to set this value, it must first box the value,
    /// then use compare_exchange(current = null, new = Box::into_raw, Relaxed, Relaxed).
    /// if this fails, than it is made aware of the fact that some other handler has (erronously,
    /// given that `from` and `discriminant` are specified and can raise an error accordingly)
    /// NOTE: AtomicCell now does this for us
    ///
    /// After this is done (if successfull) the `response_waker` should be woke
    /// to trigger the requesting task to check for this value
    pub value: AtomicCell<DynVar>,
    /// see `value`
    pub waker: Flag,
}

/// the target for a request message (instance, any type, or any)
#[derive(Debug)]
pub enum Target {
    /// this spacific instance of a handler
    Instance(HandlerInstance),
    /// all handlers of this type
    Type(HandlerType),
    /// any handlers
    Any,
}
