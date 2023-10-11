use super::{
    handler_decl_t,
    id::Uid,
    method_decl,
    msg::{HandlerInstance, Str},
};

/// the external handler (used as the sender ID for sending messages from outside a handler)
pub const HDL_EXTERNAL: HandlerInstance = HandlerInstance {
    typ: handler_decl_t!("External event dispatcher"),
    discriminant: Uid::nil(),
    discriminant_desc: Str::Borrowed("External event dispatcher"),
};

method_decl!(EV_SHUTDOWN, (), ());
