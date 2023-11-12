use std::{
    marker::PhantomData,
    ops::{DerefMut, Range},
    ptr::slice_from_raw_parts_mut,
};

use memmap2::MmapMut;

use super::registry::TypeRegistry;

/// memory address of the start of the data access slice
#[derive(Clone, Copy)]
pub struct BaseOffset<'a>(*const u8, PhantomData<&'a ()>);

impl<'a> BaseOffset<'a> {
    pub fn ptr(self) -> *const u8 {
        self.0
    }
}

pub trait SlicePtr<'a> {
    fn ptr(&self) -> *const u8;
}

impl<'a> SlicePtr<'a> for &'a mut [u8] {
    fn ptr(&self) -> *const u8 {
        self.as_ptr()
    }
}

impl<'a> SlicePtr<'a> for MultipleAccess<'a> {
    fn ptr(&self) -> *const u8 {
        self.ptr as *const u8
    }
}

pub fn access_memmap<'a>(
    map: &'a mut MmapMut,
    alloc_t_reg: &TypeRegistry,
) -> (BaseOffset<'a>, &'a mut [u8]) {
    let map = map.deref_mut();
    assert!(map.as_mut_ptr().is_aligned_to(alloc_t_reg.max_align()));
    (BaseOffset(map as *mut [u8] as *const u8, PhantomData), map)
}

#[test]
fn test_memmap_alignment() {
    let mut map = MmapMut::map_anon(1024).unwrap();
    #[repr(align(32))]
    struct A([u8; 32]);
    let mut reg = TypeRegistry::new();
    reg.register::<A>();
    let _ = access_memmap(&mut map, &reg);
}

/// Note: this allows multiple accesses, but only once - when a reference to a sub part is dropped,
///  it may not be referenced again through this struct
pub struct MultipleAccess<'a> {
    len: usize,
    ptr: *mut u8,
    /// (ptr, len)
    access: Vec<Range<*mut u8>>,
    lifetime: PhantomData<&'a mut [u8]>,
}

impl<'a> MultipleAccess<'a> {
    /// To get the original back, one can create the access with new(&mut *original_ref) to use a shorter lifetime, and then
    /// simply use the original slice again after the last use of this struct or any refs given by it
    pub fn new(slice: &'a mut [u8]) -> Self {
        Self {
            len: slice.len(),
            ptr: slice.as_mut_ptr(),
            access: vec![],
            lifetime: PhantomData,
        }
    }

    /// self is borrowed for a different lifetime than the return (self must be modified to insert the new reference, but the return value is unrelated)
    pub fn get<'b>(&'b mut self, range: Range<usize>) -> &'a mut [u8] {
        let Range { start, end } = range;
        assert!(start < end);
        assert!(end < self.len);
        // saftey preconditions
        assert!(range.end < isize::MAX as _);
        assert!(range.end.checked_add(self.ptr as usize).is_some());
        // Saftey: see previous asserts
        let ptr_range = unsafe {
            Range {
                start: self.ptr.add(range.start),
                end: self.ptr.add(range.end),
            }
        };
        assert!(
            !self.is_overlapping(ptr_range.clone()),
            "Attempted to access the same piece of data more than once simulaneously (aliasing is not allowed) - if you meant to use the same element twice, try re-using the old variable"
        );
        self.access.push(ptr_range);
        unsafe {
            // Saftey (for ptr.add): see previous preconditions
            let slice = slice_from_raw_parts_mut(self.ptr.add(range.start), range.len());
            // Saftey: this struct has exclusive ownership over the enclosed range, and has ensured that no other references to this are active
            &mut *slice
        }
    }

    /// the returned slice must be from a current access
    pub fn put<'b>(&'b mut self, slice: &'a mut [u8]) {
        let range = slice.as_mut_ptr_range();
        let idx = self
            .access
            .iter()
            .enumerate()
            .find(|(_, r)| **r == range)
            .map(|(i, _)| i)
            .expect("Returned slice was not taken from this access group");
        let _ = self.access.remove(idx);
    }

    fn is_overlapping(&self, with: Range<*mut u8>) -> bool {
        self.access.iter().any(|range| {
            let Range { start, end } = with;
            if range.contains(&start) || range.contains(&end) {
                eprintln!("Overlap detected between {range:?} and {with:?}");
                true
            } else {
                false
            }
        })
    }
}

#[test]
fn allow_close_access() {
    let mut data = vec![0; 1024];
    let mut access = MultipleAccess::new(&mut data[..]);
    let a = access.get(3..4);
    let b = access.get(4..5);
    a[0] = b[0]
}

#[test]
#[should_panic]
fn disallow_overlapping_access() {
    let mut data = vec![0; 1024];
    let mut access = MultipleAccess::new(&mut data[..]);
    let a = access.get(0..4);
    let b = access.get(3..5);
    // UB (assign a variable its own value, through two references)
    a[3] = b[0]
}
