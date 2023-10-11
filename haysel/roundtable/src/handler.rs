mod async_fn_ptr;
mod decl;
mod dispatch;
mod interface;
mod macros;
mod register;
mod runtime;

use crate::msg::{self, Str};

pub use decl::MethodDecl;
pub use interface::{local::LocalInterface, Interface};
pub use register::MethodRegister;

/// Trait that describes a handlers functionality.
///
/// All handlers must implement this trait
///
/// Please note that this is an `#[async_trait]`
#[async_trait]
pub trait HandlerInit: Send + Sync + 'static {
    /// handler declaration (the unique ID for this handler type)
    ///
    /// generated using [handler_decl_t][crate::handler_decl_t]
    const DECL: msg::HandlerType;
    /// function run by the handler task runtime on start
    ///
    /// used for dispatching startup events, starting background tasks, etc
    async fn init(&mut self, _int: &LocalInterface) {}
    /// provide a description of this handler instance
    fn describe(&self) -> Str;
    /// the methods of this handler instance
    ///
    /// to register a method, use [`register.register()`][MethodRegister::register]
    fn methods(&self, register: &mut MethodRegister<Self>);
}
