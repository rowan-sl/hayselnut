use std::{collections::HashMap, marker::PhantomData};

use uuid::Uuid;

use crate::handler::{
    async_fn_ptr::{AsyncFnPtr, HandlerFn, HandlerFnOwnArgs},
    decl::{MethodDecl, MethodRaw},
    HandlerInit,
};
#[cfg(feature = "bus_dbg")]
use crate::msg::Str;

/// Interface for registering methods on a handler.
///
/// Only instantiated by the method register function
pub struct MethodRegister<H: HandlerInit + ?Sized> {
    methods: HashMap<Uuid, MethodRaw>,
    _ph: PhantomData<H>,
}

impl<H: HandlerInit> MethodRegister<H> {
    pub(crate) fn new() -> Self {
        Self {
            methods: HashMap::new(),
            _ph: PhantomData,
        }
    }

    /// Registers that this handler implements the given [`decl`][MethodDecl] with the handler function `func`
    /// (signature: `async fn handler(&mut self, args: &ArgumentType, interface: &LocalInterface) -> ReturnType`)
    ///
    /// For events generated by a background task, use [`register_owned`][MethodRegister::register_owned]
    pub fn register<
        At: Send + Sync + 'static,
        Rt: Send + Sync + 'static,
        Fn: for<'a> AsyncFnPtr<'a, H, &'a At, Rt> + Copy + Sync + Send + 'static,
    >(
        &mut self,
        func: Fn,
        decl: MethodDecl<false, At, Rt>,
    ) {
        debug_assert!(self
            .methods
            .insert(
                decl.id,
                MethodRaw {
                    handler_func: Box::new(HandlerFn::new(func)),
                    #[cfg(feature = "bus_dbg")]
                    handler_desc: Str::Borrowed(decl.desc),
                },
            )
            .is_none());
    }

    /// Registers that this handler implements the given [`decl`][MethodDecl] with the handler function `func`
    /// (signature: `async fn handler(&mut self, args: ArgumentType, interface: &LocalInterface) -> ReturnType`)
    ///
    /// "Owned" methods are different in that they can only be generated as the result
    /// of a background task completing, and the handler function *takes ownership* of its arguments.
    ///
    /// For normal events, use [`register_owned`][MethodRegister::register_owned]
    pub fn register_owned<
        At: Send + Sync + 'static,
        Rt: Send + Sync + 'static,
        Fn: for<'a> AsyncFnPtr<'a, H, At, Rt> + Copy + Sync + Send + 'static,
    >(
        &mut self,
        func: Fn,
        decl: MethodDecl<true, At, Rt>,
    ) {
        self.methods.insert(
            decl.id,
            MethodRaw {
                handler_func: Box::new(HandlerFnOwnArgs::new(func)),
                #[cfg(feature = "bus_dbg")]
                handler_desc: Str::Borrowed(decl.desc),
            },
        );
    }

    pub(crate) fn finalize(self) -> HashMap<Uuid, MethodRaw> {
        self.methods
    }
}
