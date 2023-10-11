use std::{
    ptr,
    sync::atomic::{self, AtomicPtr},
};

// NOTE: `Send` is REQUIRED for saftey
#[derive(Debug)]
pub struct AtomicCell<T: Send + 'static> {
    ptr: AtomicPtr<T>,
}

impl<T: Send + 'static> AtomicCell<T> {
    pub fn new() -> Self {
        Self {
            ptr: AtomicPtr::new(ptr::null_mut()),
        }
    }

    /// see put_boxed (this is a thin wrapper that callsed put_boxed(Box::new(val)))
    pub fn put(&self, val: T) -> Option<Box<T>> {
        self.put_boxed(Box::new(val))
    }

    /// returns Some(val passed to put_boxed()) on failure (AKA there already was a value in `self`)
    pub fn put_boxed(&self, val: Box<T>) -> Option<Box<T>> {
        if let Err(val_ptr) = self.ptr.compare_exchange(
            ptr::null_mut(),
            Box::into_raw(val),
            atomic::Ordering::SeqCst,
            atomic::Ordering::SeqCst,
        ) {
            // return val on failure to avoid memory leak
            // Saftey: if compare_exchange fails, then the pointer could not possibly
            // have been seen (much less used) by any other tasks
            unsafe { Some(Box::from_raw(val_ptr)) }
        } else {
            None
        }
    }

    pub fn take(&self) -> Option<Box<T>> {
        let pointer = self.ptr.swap(ptr::null_mut(), atomic::Ordering::SeqCst);
        if pointer.is_null() {
            None
        } else {
            // Saftey: no other (correctly implemented) handlers should read and use a value.
            // TODO: safe code can do things that violate the saftey of this (by putting a
            // dangleing pointer in this field)
            let boxed = unsafe { Box::from_raw(pointer) };
            Some(boxed)
        }
    }
}

impl<T: Send + 'static> Drop for AtomicCell<T> {
    fn drop(&mut self) {
        let _ = self.take();
    }
}
