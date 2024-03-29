//! TSDB v3
//!
//! Keep in Mind: tokio::fs simply uses spawn_blocking(std::fs)

use std::mem::{align_of, size_of};

use memmap2::MmapMut;
use static_assertions::const_assert;
use zerocopy::{AsBytes, FromBytes, FromZeroes, Ref};

use self::{
    access::{access_memmap, BaseOffset, MultipleAccess},
    registry::alignment_pad_size,
};

pub use ptr::Ptr;
pub use registry::TypeRegistry;

mod access;
pub mod ptr;
mod registry;
mod repr;

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

    pub fn get_size_used(&self) -> u64 {
        self.header.used
    }

    pub fn entrypoint_pointer(&mut self) -> &mut Ptr<ptr::Void> {
        &mut self.header.entrypoint
    }

    pub fn entrypoint<'b, T: FromBytes + AsBytes + 'a>(&'b mut self) -> Option<&'a mut T> {
        if self.header.entrypoint.is_null() {
            None
        } else {
            Some(self.read(self.header.entrypoint.cast::<T>()))
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
    pub fn alloc<T: AsBytes + FromBytes + FromZeroes>(&mut self) -> (Ptr<T>, &'a mut T) {
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
            let ref0 = Ref::<_, T>::new_zeroed(&mut dat[alignment_pad_size::<T>()..])
                .unwrap()
                .into_mut();
            (
                free_spot
                    .offset((size_of::<repr::ChunkHeader>() + alignment_pad_size::<T>()) as _)
                    .cast::<T>(),
                ref0,
            )
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
            let mut header = Ref::<_, repr::ChunkHeader>::new(&mut *header_dat).unwrap();
            *header = repr::ChunkHeader {
                flags: repr::ChunkFlags::empty().bits(),
                len: (alignment_pad_size::<T>() + size_of::<T>()) as _,
                // dangling, non null (not required, but it will make detecting errors easier)
                next: Ptr::with(1),
            };
            self.dat.put(header_dat);
            // -- get and return the body --
            let ptr_t = global_ptr
                .offset((size_of::<repr::ChunkHeader>() + alignment_pad_size::<T>()) as _)
                .cast::<T>();
            let dat = self
                .dat
                .get(ptr_t.localize_to(self.base, &self.dat).to_range_usize());
            (ptr_t, Ref::<_, T>::new_zeroed(dat).unwrap().into_mut())
        }
    }

    /// returns a ref to an already allocated value
    pub fn read<T: AsBytes + FromBytes + FromZeroes>(&mut self, ptr: Ptr<T>) -> &'a mut T {
        assert!(self.alloc_t_reg.contains_similar::<T>());
        // -- get and return the body --
        let dat = self
            .dat
            .get(ptr.localize_to(self.base, &self.dat).to_range_usize());
        Ref::<_, T>::new(dat).unwrap().into_mut()
    }
}

#[test]
fn test_new_map_basic_types() {
    let mut map = MmapMut::map_anon(4096).unwrap();
    let alloc_t_reg = {
        let mut alloc_t_reg = TypeRegistry::new();
        alloc_t_reg.register::<u64>();
        alloc_t_reg.register::<[u8; 13]>();
        alloc_t_reg
    };
    let mut alloc = AllocAccess::new(&mut map, &alloc_t_reg, true);
    let (_ptr_v, v) = alloc.alloc::<[u8; 13]>();
    *v = *b"Hello, World!";
}

#[test]
fn test_alloc_twice() {
    let mut map = MmapMut::map_anon(4096).unwrap();
    let alloc_t_reg = {
        let mut alloc_t_reg = TypeRegistry::new();
        alloc_t_reg.register::<u64>();
        alloc_t_reg.register::<[u8; 13]>();
        alloc_t_reg
    };
    let mut alloc = AllocAccess::new(&mut map, &alloc_t_reg, true);
    let (ptr_v, v) = alloc.alloc::<[u8; 13]>();
    *v = *b"Hello, World!";
    drop(alloc);
    let mut alloc = AllocAccess::new(&mut map, &alloc_t_reg, false);
    let v = alloc.read(ptr_v);
    assert_eq!(v, &b"Hello, World!"[..]);
    let _ = alloc.alloc::<u64>();
}

#[test]
#[should_panic]
fn test_new_map_not_enough_space() {
    let mut map = MmapMut::map_anon(20).unwrap();
    let alloc_t_reg = {
        let mut alloc_t_reg = TypeRegistry::new();
        alloc_t_reg.register::<u64>();
        alloc_t_reg.register::<[u8; 13]>();
        alloc_t_reg
    };
    // missing space, will panic
    let _alloc = AllocAccess::new(&mut map, &alloc_t_reg, true);
}

#[test]
#[should_panic]
fn test_alloc_access_access_twice() {
    let mut map = MmapMut::map_anon(4096).unwrap();
    let alloc_t_reg = {
        let mut alloc_t_reg = TypeRegistry::new();
        alloc_t_reg.register::<u64>();
        alloc_t_reg.register::<[u8; 13]>();
        alloc_t_reg
    };
    let mut alloc = AllocAccess::new(&mut map, &alloc_t_reg, true);
    let (ptr_v, v) = alloc.alloc::<[u8; 13]>();
    *v = *b"Hello, World!";
    // panic
    let _v_again = alloc.read(ptr_v);
}

#[test]
fn test_alloc_access_again() {
    let mut map = MmapMut::map_anon(4096).unwrap();
    let alloc_t_reg = {
        let mut alloc_t_reg = TypeRegistry::new();
        alloc_t_reg.register::<u64>();
        alloc_t_reg.register::<[u8; 13]>();
        alloc_t_reg
    };
    let mut alloc = AllocAccess::new(&mut map, &alloc_t_reg, true);
    let (ptr_v, v) = alloc.alloc::<[u8; 13]>();
    *v = *b"Hello, World!";
    drop(alloc);
    let mut alloc = AllocAccess::new(&mut map, &alloc_t_reg, false);
    let v = alloc.read(ptr_v);
    assert_eq!(v, &b"Hello, World!"[..]);
}

#[test]
fn test_alloc_tricky_types() {
    let mut map = MmapMut::map_anon(4096).unwrap();
    let alloc_t_reg = {
        let mut alloc_t_reg = TypeRegistry::new();
        alloc_t_reg.register::<super::repr::DBEntrypoint>();
        alloc_t_reg.register::<super::repr::Station>();
        alloc_t_reg
    };
    let mut alloc = AllocAccess::new(&mut map, &alloc_t_reg, true);
    let (entry, _) = alloc.alloc::<super::repr::DBEntrypoint>();
    drop(alloc);
    let mut alloc = AllocAccess::new(&mut map, &alloc_t_reg, false);
    let _v = alloc.read(entry);
    let _a = alloc.alloc::<super::repr::Station>();
}
