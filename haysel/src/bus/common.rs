use super::{
    handler::{handler_decl_t, method_decl},
    id::Uid,
    msg::{HandlerInstance, Str},
};

/// the external handler (used as the sender ID for sending messages from outside a handler)
pub const HDL_EXTERNAL: HandlerInstance = HandlerInstance {
    typ: handler_decl_t!("External event dispatcher"),
    discriminant: Uid::nill(),
    discriminant_desc: Str::Borrowed("External event dispatcher"),
};

method_decl!(EV_SHUTDOWN, (), ());
