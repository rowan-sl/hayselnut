use std::fs::OpenOptions;

use anyhow::Result;
use memmap2::MmapMut;

use self::alloc::{AllocAccess, TypeRegistry};

mod alloc;

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
        let (_ptr_v, v) = alloc.alloc::<[u8; 13]>();
        *v = *b"Hello, World!";
    }
    file.sync_all()?;
    Ok(())
}
