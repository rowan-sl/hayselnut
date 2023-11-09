// pointers within the database are 40-bit numbers
// (this leaves about up to ~1TB of space)
// this also leaves space for: t a g g e d  p o i n t e r s (24-bits of extra space)
//
// this is bc 32bit would leave max 4GB of size, which is a bit small

const ADDR_MAX: u64 = 2.pow(40);

#[repr(C)]
struct SomeDataStructure<'db> {
    // this is not stored in the database
    // it is used for reading fields from the db using Object<'db, Self>
    // you can impl Object<'db, Self> { fn function_that_needs_db_access(&mut self) } // doesn't take extra args!
    alloc_or_db_reference: &'db AllocOrDb,
    my_addr: Addr,
    // only this is
    data: u64,
}

// implemented for all values stored in the database
trait DBStruct {
    fn db(&self) -> &AllocOrDb;
    fn addr(&self) -> Addr;
    const fn size(&self) -> usize {
        size_of::<Self>() - (size_of::<usize>() + size_of::<Addr>())
    }
}

struct AllocOrDb {}

fn read_some_data_structure<'db>(db: &'db AllocOrDb, from: Addr, buf: &'db mut [u8]) -> &'db mut SomeDataStructure<'db> {
    buf.len == size_of::<SomeDataStructure>];
    write(transmute<&'db T -> [u8]>(db), buf[0..size_of::<usize>()]);
    write(transmute<Addr -> [u8]>(addr), buf[size_of::<usize>..][..size_of::<Addr>]);
    write(db.read(from, size_of::<SomeDataStructure>() - size_of::<usize>()), buf[size_of::<usize>+size_of::<addr>..]);
    // Saftey: SomeDataStruct contains &'db AllocOrDb, which is written in the first write
    // reference is correct to transmute here (and in fact transmuting after writing the ref is the correct thing to do)
    // buf contains the data for the data strucuture
    unsafe {
        transmute<&mut [u8] -> &mut SomeDataStructure>(buf)
    }
}

fn reading_data() {
    // data from the db file should be read directly into a large shared buffer (arena-alloc like)
    // objects are like this
    struct Object<'db, T> {
        ref: &'db mut T,
    }
    // object PartialEq can check if `ref` is the same pointer before checking the content of `ref`
    // object Drop should trigger a write of changes back to the database
    //  - but the database could also know to write back changes to itself if the object is forgotten somehow

    //NOTE: ojbects (behind the pointer) contain a reference to the database, as well as their own address in the database
    // (this could be exposed using a trait)
}
