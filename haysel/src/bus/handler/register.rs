use std::{collections::HashMap, marker::PhantomData};

use uuid::Uuid;

use crate::bus::{
    handler::{
        async_fn_ptr::{AsyncFnPtr, HandlerFn, HandlerFnOwnArgs},
        decl::{MethodDecl, MethodRaw},
        HandlerInit,
    },
    msg::Str,
};

pub struct MethodRegister<H: HandlerInit + ?Sized> {
    methods: HashMap<Uuid, MethodRaw>,
    _ph: PhantomData<H>,
}

impl<H: HandlerInit> MethodRegister<H> {
    pub(in crate::bus) fn new() -> Self {
        Self {
            methods: HashMap::new(),
            _ph: PhantomData,
        }
    }

    pub fn register<
        At: Send + Sync + 'static,
        Rt: Send + Sync + 'static,
        Fn: for<'a> AsyncFnPtr<'a, H, &'a At, Rt> + Copy + Sync + Send + 'static,
    >(
        &mut self,
        func: Fn,
        decl: MethodDecl<false, At, Rt>,
    ) {
        self.methods.insert(
            decl.id,
            MethodRaw {
                handler_func: Box::new(HandlerFn::new(func)),
                handler_desc: Str::Borrowed(decl.desc),
            },
        );
    }

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
                handler_desc: Str::Borrowed(decl.desc),
            },
        );
    }

    pub(in crate::bus) fn finalize(self) -> HashMap<Uuid, MethodRaw> {
        self.methods
    }
}
