use std::ops::{Deref, DerefMut};

pub struct Take<T>(Option<T>);

/// more convenient replacement for when you need to use Option::take once, at the end of a values life
impl<T> Take<T> {
    pub fn new(val: T) -> Self {
        Self(Some(val))
    }

    pub fn take(&mut self) -> T {
        self.0.take().unwrap()
    }
}

impl<T> Deref for Take<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().unwrap()
    }
}

impl<T> DerefMut for Take<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut().unwrap()
    }
}
