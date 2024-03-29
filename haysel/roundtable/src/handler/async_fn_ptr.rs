//! # generic type+lifetime erased asynchrounous function pointers ftw
//!
//! this took me an indescribable ammount of time to figure out
//!
//! ## A warning to travlers
//!
//! no touchie
//!
//! (copied from dabus, modified to fit this better)

use super::{super::dyn_var::DynVar, HandlerInit};

use core::marker::PhantomData;
use std::any::type_name;

use futures::future::{BoxFuture, Future};

use super::LocalInterface;

pub trait AsyncFnPtr<'a, H: HandlerInit + 'a, At: 'a, Rt> {
    type Fut: Future<Output = Result<Rt, H::Error>> + Send + 'a;
    fn call(self, h: &'a mut H, a: At, i: &'a LocalInterface) -> Self::Fut;
}

impl<
        'a,
        H: HandlerInit + 'a,
        At: 'a,
        Rt,
        Fut: Future<Output = Result<Rt, H::Error>> + Send + 'a,
        F: FnOnce(&'a mut H, At, &'a LocalInterface) -> Fut,
    > AsyncFnPtr<'a, H, At, Rt> for F
{
    type Fut = Fut;
    fn call(self, h: &'a mut H, a: At, i: &'a LocalInterface) -> Self::Fut {
        self(h, a, i)
    }
}

#[derive(Clone)]
pub struct HandlerFn<H: HandlerInit + 'static, At: 'static, Rt: 'static, P>
where
    P: for<'a> AsyncFnPtr<'a, H, &'a At, Rt> + Copy,
{
    f: P,
    _t: PhantomData<&'static (H, At, Rt)>,
}

impl<H: HandlerInit + Send + 'static, At: Sync + Send + 'static, Rt: 'static, P>
    HandlerFn<H, At, Rt, P>
where
    P: for<'a> AsyncFnPtr<'a, H, &'a At, Rt> + Send + Copy + 'static,
{
    #[must_use]
    pub const fn new(f: P) -> Self {
        Self { f, _t: PhantomData }
    }

    pub fn call<'a, 'b>(
        &'b self,
        h: &'a mut H,
        a: &'a At,
        i: &'a LocalInterface,
    ) -> BoxFuture<'a, Result<Rt, H::Error>> {
        let f = self.f;
        Box::pin(async move { f.call(h, a, i).await })
    }
}

#[derive(Clone)]
pub struct HandlerFnOwnArgs<H: HandlerInit + 'static, At: 'static, Rt: 'static, P>
where
    P: for<'a> AsyncFnPtr<'a, H, At, Rt> + Copy,
{
    f: P,
    _t: PhantomData<&'static (H, At, Rt)>,
}

impl<H: HandlerInit + Send + 'static, At: Sync + Send + 'static, Rt: 'static, P>
    HandlerFnOwnArgs<H, At, Rt, P>
where
    P: for<'a> AsyncFnPtr<'a, H, At, Rt> + Send + Copy + 'static,
{
    #[must_use]
    pub const fn new(f: P) -> Self {
        Self { f, _t: PhantomData }
    }

    pub fn call<'a, 'b>(
        &'b self,
        h: &'a mut H,
        a: At,
        i: &'a LocalInterface,
    ) -> BoxFuture<'a, Result<Rt, H::Error>> {
        let f = self.f;
        Box::pin(async move { f.call(h, a, i).await })
    }
}

pub trait HandlerCallableErased {
    fn call<'a>(
        &'a self,
        h: &'a mut DynVar,
        a: &'a DynVar,
        i: &'a LocalInterface,
    ) -> Result<BoxFuture<'a, Result<DynVar, DynVar>>, CallError>;
    fn call_owned<'a>(
        &'a self,
        h: &'a mut DynVar,
        a: DynVar,
        i: &'a LocalInterface,
    ) -> Result<BoxFuture<'a, Result<DynVar, DynVar>>, CallError>;
}

impl<H, At, Rt, P> HandlerCallableErased for HandlerFn<H, At, Rt, P>
where
    P: for<'a> AsyncFnPtr<'a, H, &'a At, Rt> + Send + Sync + Copy + 'static,
    H: HandlerInit + Send + Sync + 'static,
    At: Send + Sync + 'static,
    Rt: Send + Sync + 'static,
{
    fn call<'a>(
        &'a self,
        h: &'a mut DynVar,
        a: &'a DynVar,
        i: &'a LocalInterface,
    ) -> Result<BoxFuture<'a, Result<DynVar, DynVar>>, CallError> {
        let h_name = h.type_name();
        let a_name = h.type_name();
        let h = h
            .as_mut::<H>()
            .ok_or(CallError::MismatchHandler(type_name::<H>(), h_name))?;
        let a = a
            .as_ref::<At>()
            .ok_or(CallError::MismatchArgs(type_name::<At>(), a_name))?;
        Ok(Box::pin(async move {
            let r = self.call(h, a, i).await;
            r.map(|ok| DynVar::new(ok)).map_err(|err| DynVar::new(err))
        }))
    }
    fn call_owned<'a>(
        &'a self,
        _h: &'a mut DynVar,
        _a: DynVar,
        _i: &'a LocalInterface,
    ) -> Result<BoxFuture<'a, Result<DynVar, DynVar>>, CallError> {
        Err(CallError::BorrowInconsistancy)
    }
}

impl<H, At, Rt, P> HandlerCallableErased for HandlerFnOwnArgs<H, At, Rt, P>
where
    P: for<'a> AsyncFnPtr<'a, H, At, Rt> + Send + Sync + Copy + 'static,
    H: HandlerInit + Send + Sync + 'static,
    At: Send + Sync + 'static,
    Rt: Send + Sync + 'static,
{
    fn call<'a>(
        &'a self,
        _h: &'a mut DynVar,
        _a: &'a DynVar,
        _i: &'a LocalInterface,
    ) -> Result<BoxFuture<'a, Result<DynVar, DynVar>>, CallError> {
        Err(CallError::BorrowInconsistancy)
    }
    fn call_owned<'a>(
        &'a self,
        h: &'a mut DynVar,
        a: DynVar,
        i: &'a LocalInterface,
    ) -> Result<BoxFuture<'a, Result<DynVar, DynVar>>, CallError> {
        let h_name = h.type_name();
        let a_name = h.type_name();
        let h = h
            .as_mut::<H>()
            .ok_or(CallError::MismatchHandler(type_name::<H>(), h_name))?;
        let a = a
            .try_to::<At>()
            .map_err(|_| CallError::MismatchArgs(type_name::<At>(), a_name))?;
        Ok(Box::pin(async move {
            let r = self.call(h, a, i).await;
            r.map(|ok| DynVar::new(ok)).map_err(|err| DynVar::new(err))
        }))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CallError {
    #[error("failed to call handler: mismatched type of handler (expected {0}, found {1})")]
    MismatchHandler(&'static str, &'static str),
    #[error("failed to call handler: mismatched type of arguments (expected {0}, found {1})")]
    MismatchArgs(&'static str, &'static str),
    #[error("This handler does not support call_owned / only supports call_owned")]
    BorrowInconsistancy,
}
