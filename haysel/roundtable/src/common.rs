use super::{handler_decl_t, id::Uid, msg::HandlerInstance};
use crate::method_decl;
#[cfg(feature = "bus_dbg")]
use crate::msg::Str;

/// the external handler (used as the sender ID for sending messages from outside a handler)
pub const HDL_EXTERNAL: HandlerInstance = HandlerInstance {
    typ: handler_decl_t!("External event dispatcher"),
    discriminant: Uid::nil(),
    #[cfg(feature = "bus_dbg")]
    discriminant_desc: Str::Borrowed("External event dispatcher"),
};

method_decl!(EV_BUILTIN_AUTOSAVE, (), ());
