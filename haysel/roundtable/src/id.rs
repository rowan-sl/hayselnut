use std::sync::atomic::{self, AtomicU64};

/// NON UNIVERSALLY unique identifier
///
/// all Uids that are compared with each other must come from the same `source`
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Uid(u64);

impl Uid {
    /// generates a new Unique identifer by taking the current value in `source` and incrementing
    /// it by 1. this will generate unique ids, as long as they are only compared to values coming
    /// from the same source.
    pub(crate) fn gen_with(source: &AtomicU64) -> Self {
        Self(source.fetch_add(1, atomic::Ordering::Relaxed))
    }
    pub(crate) const fn nil() -> Self {
        Self(0)
    }
}

/// Generates a random Uuid at compile time
#[doc(hidden)]
#[macro_export]
macro_rules! const_uuid_v4 {
    () => {
        $crate::uuid::Builder::from_u128($crate::const_random::const_random!(u128)).into_uuid()
    };
}
