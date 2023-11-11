//! TSDB v3
//!
//! Keep in Mind: tokio::fs simply uses spawn_blocking(std::fs)

use std::{
    alloc::Layout,
    cmp::max,
    fs::OpenOptions,
    marker::PhantomData,
    mem::{align_of, size_of},
    ops::{DerefMut, Range},
    ptr::slice_from_raw_parts_mut,
};

use anyhow::Result;
use memmap2::MmapMut;
use static_assertions::const_assert;
use zerocopy::{AsBytes, FromBytes, FromZeroes, Ref};

use crate::tsdb3::ptr::Ptr;

mod ptr;
mod repr;

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

fn access_memmap<'a>(
    map: &'a mut MmapMut,
    alloc_t_reg: &TypeRegistry,
) -> (BaseOffset<'a>, &'a mut [u8]) {
    let map = map.deref_mut();
    assert!(map.as_mut_ptr().is_aligned_to(alloc_t_reg.max_align()));
    (BaseOffset(map as *mut [u8] as *const u8, PhantomData), map)
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
            !self.is_overlapping(ptr_range),
            "Attempted to access the same piece of data more than once simulaneously (aliasing is not allowed) - if you meant to use the same element twice, try re-using the old variable"
        );
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
            range.contains(&start) || range.contains(&end)
        })
    }
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

pub struct AllocAccess<'a> {
    alloc_t_reg: &'a TypeRegistry,
    base: BaseOffset<'a>,
    header: &'a mut repr::AllocHeader,
    free_lists: &'a mut [repr::AllocCategoryHeader],
    dat: MultipleAccess<'a>,
}

impl<'a> AllocAccess<'a> {
    pub fn new(map: &'a mut MmapMut, alloc_t_reg: &'a TypeRegistry, write_header: bool) -> Self {
        // make sure that all allocator types are alligned properly
        const_assert!(align_of::<repr::AllocCategoryHeader>() <= align_of::<repr::AllocHeader>());
        const_assert!(align_of::<repr::AllocCategoryHeader>() <= align_of::<repr::ChunkHeader>());
        // -- get memmap content --
        let (base, dat): (BaseOffset, &mut [u8]) = access_memmap(map, &alloc_t_reg);
        // -- get header --
        let (mut header, dat) = Ref::<_, repr::AllocHeader>::new_from_prefix(dat).unwrap();
        if write_header {
            *header = repr::AllocHeader::new(Ptr::null(), alloc_t_reg.num_types() as u64);
        }
        assert!(header.verify());
        // -- get the free lists --
        let (free_lists, dat) = Ref::<_, [repr::AllocCategoryHeader]>::new_slice_from_prefix(
            dat,
            header.free_list_size as _,
        )
        .unwrap();
        Self {
            alloc_t_reg,
            base,
            header: header.into_mut(),
            free_lists: free_lists.into_mut_slice(),
            dat: MultipleAccess::new(dat),
        }
    }

    pub fn get_free_for<T>(&mut self) -> Option<Ptr<repr::ChunkHeader>> {
        // -- find the appropreate list --
        let (list_header, found) = 'found: {
            for list in &mut *self.free_lists {
                if (size_of::<T>() + alignment_pad_size::<T>()) as u64 == list.size
                    && align_of::<T>() as u64 == list.align
                {
                    let head = list.head;
                    break 'found (list, head);
                }
            }
            // no entry (free list) exists for this type
            return None;
        };
        // this entry (free list) exists, but it has no entries (free chunks)
        if found.is_null() {
            return None;
        }
        // -- get the first entry in the free list --
        let first_dat = self
            .dat
            .get(found.localize_to(self.base, &self.dat).to_range_usize());
        let first = Ref::<_, repr::ChunkHeader>::new(&mut *first_dat).unwrap();
        // -- remove `first` from this free list --
        if first.next.is_null() {
            // no `next` element in the list, so set the head to null
            list_header.head = Ptr::null();
        } else {
            // there is an element after `first` in the list, so set the head to that
            list_header.head = first.next;
        }
        self.dat.put(first_dat);
        Some(found)
    }

    /// allocates a new zeroed T, and returns a ref to it
    pub fn alloc<T: AsBytes + FromBytes + FromZeroes>(&mut self) -> &'a mut T {
        assert!(self.alloc_t_reg.contains_similar::<T>());
        if let Some(free_spot) = self.get_free_for::<T>() {
            let dat = self
                .dat
                .get(free_spot.localize_to(self.base, &self.dat).to_range_usize());
            let (mut header, dat) = Ref::<_, repr::ChunkHeader>::new_from_prefix(dat).unwrap();
            let mut flags = repr::ChunkFlags::from_bits(header.flags).unwrap();
            flags.remove(repr::ChunkFlags::FREE);
            header.flags = flags.bits();
            // remove alignment padding
            Ref::<_, T>::new_zeroed(&mut dat[alignment_pad_size::<T>()..])
                .unwrap()
                .into_mut()
        } else {
            let global_ptr = Ptr::<repr::ChunkHeader>::with(self.header.used);
            self.header.used += (size_of::<repr::ChunkHeader>()
                + alignment_pad_size::<T>()
                + size_of::<T>()) as u64;
            // -- write the new header --
            let header_dat = self.dat.get(
                global_ptr
                    .localize_to(self.base, &self.dat)
                    .to_range_usize(),
            );
            let mut header = Ref::<_, repr::ChunkHeader>::new(header_dat).unwrap();
            *header = repr::ChunkHeader {
                flags: repr::ChunkFlags::empty().bits(),
                len: (alignment_pad_size::<T>() + size_of::<T>()) as _,
                // dangling, non null (not required, but it will make detecting errors easier)
                next: Ptr::with(1),
            };
            // -- get and return the body --
            let dat = self.dat.get(
                global_ptr
                    .offset((size_of::<repr::ChunkHeader>() + alignment_pad_size::<T>()) as _)
                    .cast::<T>()
                    .to_range_usize(),
            );
            Ref::<_, T>::new_zeroed(dat).unwrap().into_mut()
        }
    }

    /// returns a ref to an already allocated value
    pub fn read<T: AsBytes + FromBytes + FromZeroes>(&mut self, ptr: Ptr<T>) -> &'a mut T {
        assert!(self.alloc_t_reg.contains_similar::<T>());
        // -- get and return the body --
        let dat = self.dat.get(
            ptr.offset((size_of::<repr::ChunkHeader>() + alignment_pad_size::<T>()) as _)
                .cast::<T>()
                .to_range_usize(),
        );
        Ref::<_, T>::new_zeroed(dat).unwrap().into_mut()
    }
}

pub fn main() -> Result<()> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open("test.tsdb3")?;
    file.set_len(0)?;
    file.set_len(1024 * 500)?;
    // Saftey: lol. lmao.
    let mut map = unsafe { MmapMut::map_mut(&file)? };
    let alloc_t_reg = {
        let mut alloc_t_reg = TypeRegistry::new();
        alloc_t_reg.register::<u64>();
        alloc_t_reg.register::<[u8; 13]>();
        alloc_t_reg
    };
    {
        let mut alloc = AllocAccess::new(&mut map, &alloc_t_reg, true);
        let v = alloc.alloc::<[u8; 13]>();
        *v = *b"Hello, World!";
    }
    file.sync_all()?;
    Ok(())
}
