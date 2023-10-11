pub mod async_fn_ptr;
pub mod decl;
pub mod dispatch;
pub mod interface;
pub mod macros;
pub mod register;
pub mod runtime;

use crate::msg::{self, Str};

pub use decl::MethodDecl;
pub use interface::{local::LocalInterface, Interface};
pub use register::MethodRegister;

#[async_trait]
pub trait HandlerInit: Send + Sync + 'static {
    const DECL: msg::HandlerType;
    // type BgGenerated: Sync + Send + 'static;
    // const BG_RUN: bool = false;
    // /// NOTE: This function MUST be cancel safe.
    // async fn bg_generate(&mut self) -> Self::BgGenerated { unimplemented!() }
    // async fn bg_consume(&mut self, _args: Self::BgGenerated, _int: LocalInterface) { unimplemented!() }
    async fn init(&mut self, _int: &LocalInterface) {}
    // description of this handler instance
    fn describe(&self) -> Str;
    // methods of this handler instance
    fn methods(&self, register: &mut MethodRegister<Self>);
}
