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

#[test]
fn test_round_up_to() {
    for i in 9..=16 {
        assert_eq!(round_up_to(i, 8), 16);
    }
    assert_eq!(round_up_to(16, 8), 16);
    assert_eq!(round_up_to(8, 8), 8);
    assert_eq!(round_up_to(7, 8), 8);
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

#[test]
fn test_align_pad() {
    assert_eq!(size_of::<repr::ChunkHeader>(), 16);
    assert_eq!(align_of::<repr::ChunkHeader>(), 8);
    //note: size must be a multiple of alignment
    assert_eq!(alignment_pad_size::<u16>(), 6);
    assert_eq!(alignment_pad_size::<repr::ChunkHeader>(), 0);
    struct A([u8; 9]);
    assert_eq!(alignment_pad_size::<A>(), 7);
}

#[test]
fn test_align_pad_unusual_alignment() {
    #[repr(align(32))]
    struct B([u8; 32]);
    assert_eq!(align_of::<B>(), 32);
    assert_eq!(size_of::<B>(), 32);
    assert_eq!(alignment_pad_size::<B>(), 16);
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
