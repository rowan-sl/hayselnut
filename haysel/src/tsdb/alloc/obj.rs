use std::ops::{Deref, DerefMut};

use derivative::Derivative;

use super::{Alloc, Data, NonNull};

#[derive(Derivative)]
#[derivative(Debug)]
pub struct Obj<'a, T: Data> {
    #[derivative(Debug = "ignore")]
    pub(super) alloc: &'a Alloc,
    pub(super) ptr: NonNull<T>,
    /// current value (not synced to disk)
    pub(super) val: T,
}

impl<'a, T: Data> Obj<'a, T> {
    // all function here should not take self, but take Self as a normal param -- like Box

    pub fn get_ptr(obj: &Self) -> NonNull<T> {
        obj.ptr
    }

    pub fn into_ptr(obj: Self) -> NonNull<T> {
        let p = Self::get_ptr(&obj);
        // runs sync if necessary
        drop(obj);
        p
    }
}

impl<'a, T: Data> Deref for Obj<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.val
    }
}

impl<'a, T: Data> DerefMut for Obj<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.val
    }
}

impl<'a, T: Data> Drop for Obj<'a, T> {
    fn drop(&mut self) {
        Alloc::attempt_sync(self)
    }
}