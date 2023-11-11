use std::{
    alloc::Layout,
    cmp::max,
    mem::{align_of, size_of},
};

use crate::tsdb3::repr;

#[inline]
pub(crate) fn round_up_to(n: usize, divisor: usize) -> usize {
    debug_assert!(divisor.is_power_of_two());
    (n + divisor - 1) & !(divisor - 1)
}

pub fn alignment_pad_size<T>() -> usize {
    // align to the alignment of chunk header or T, whichever is greater
    // ensures that the next chunk will have the proper alignment for its header,
    // and that T is properly aligned if its align is greater than that of ChunkHeader
    if align_of::<T>() > align_of::<repr::ChunkHeader>() {
        assert!(align_of::<T>() % align_of::<repr::ChunkHeader>() == 0);
    }
    round_up_to(
        size_of::<T>() + size_of::<repr::ChunkHeader>(),
        max(align_of::<T>(), align_of::<repr::ChunkHeader>()),
    ) - size_of::<T>()
        - size_of::<repr::ChunkHeader>()
}

#[derive(Default)]
pub struct TypeRegistry {
    types: Vec<Layout>,
}

impl TypeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<T>(&mut self) {
        self.types.push(Layout::new::<T>())
    }

    pub fn extend(&mut self, other: &Self) {
        self.types.extend_from_slice(&other.types);
    }

    pub fn num_types(&self) -> usize {
        self.types.len()
    }

    pub fn max_align(&self) -> usize {
        self.types.iter().map(|t| t.align()).max().unwrap_or(1)
    }

    pub fn min_align(&self) -> usize {
        self.types.iter().map(|t| t.align()).min().unwrap_or(1)
    }

    pub fn contains_similar<T>(&self) -> bool {
        self.types.contains(&Layout::new::<T>())
    }
}
