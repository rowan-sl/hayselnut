//! the place where all of the very cursed workarounds for things like:
//! - cloning things that may not be clone
//! - getting the actual type name of dyn Any
//! - casting dyn T -> dyn Any (not as cursed)

pub mod dyn_downcast;
pub mod dyn_typename;

pub use dyn_downcast::AsAny;
pub use dyn_typename::TypeNamed;

use std::{any::TypeId, fmt::Debug};

/// convenience trait for [`TypeNamed`] + [`AsAny`] + 'static
pub trait GeneralRequirements: TypeNamed + AsAny + 'static {}
impl<T: 'static> GeneralRequirements for T {}

#[repr(transparent)]
pub struct DynVar {
    val: Box<dyn GeneralRequirements + Sync + Send + 'static>,
}

impl DynVar {
    #[must_use]
    pub fn new<T: GeneralRequirements + Sync + Send + 'static>(x: T) -> Self {
        Self { val: Box::new(x) }
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn to_raw(self) -> Box<dyn GeneralRequirements + Send + Sync + 'static> {
        self.val
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn from_raw(val: Box<dyn GeneralRequirements + Sync + Send + 'static>) -> Self {
        Self { val }
    }

    #[must_use]
    pub fn type_name(&self) -> &'static str {
        (*self.val).type_name()
    }

    #[must_use]
    pub fn as_ref<T: GeneralRequirements>(&self) -> Option<&T> {
        (*self.val).as_any().downcast_ref()
    }

    #[must_use]
    pub fn as_mut<T: GeneralRequirements>(&mut self) -> Option<&mut T> {
        (*self.val).mut_any().downcast_mut()
    }

    pub fn try_to<T: GeneralRequirements>(self) -> Result<T, Self> {
        if (*self.val).as_any().type_id() == TypeId::of::<T>() {
            Ok(unsafe { *self.val.to_any().downcast().unwrap_unchecked() })
        } else {
            Err(self)
        }
    }

    #[must_use]
    pub fn is<T: GeneralRequirements>(&self) -> bool {
        (*self.val).as_any().type_id() == TypeId::of::<T>()
    }

    #[must_use]
    pub fn clone_as<T: GeneralRequirements + Clone + Sync + Send + 'static>(&self) -> Option<Self> {
        Some(Self {
            val: Box::new(self.as_ref::<T>()?.clone()),
        })
    }
}

impl Debug for DynVar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynVar").finish_non_exhaustive()
    }
}
