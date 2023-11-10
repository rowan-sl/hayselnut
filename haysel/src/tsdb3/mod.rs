//! TSDB v3

use std::{
    iter::repeat,
    marker::PhantomData,
    mem::{forget, size_of, transmute, MaybeUninit},
    ops::{Deref, DerefMut},
    ptr,
    sync::atomic::{AtomicPtr, Ordering},
};

use anyhow::Result;
use tokio::io;
use tokio_uring::{
    buf::IoBuf,
    fs::{File, OpenOptions},
};

mod alloc;

//TODO: is this actaully needed (if only single thread, than no)
struct Buffers {
    buffers: Vec<AtomicPtr<u8>>,
    // buffer size
    size: usize,
}

impl Buffers {
    pub fn new(num: usize, size: usize) -> Self {
        let mut buffers = Vec::with_capacity(num);
        for _ in 0..num {
            let buf = repeat(0u8).take(size).collect::<Vec<_>>();
            let ptr = Box::into_raw(buf.into_boxed_slice());
            let atomic = AtomicPtr::new(ptr as *mut u8);
            buffers.push(atomic);
        }
        Self { buffers, size }
    }

    fn get_a_buffer(&self) -> Option<Buffer> {
        for _ in 0..5 {
            for buf in &self.buffers {
                let ptr = buf.swap(ptr::null_mut(), Ordering::Relaxed);
                if !ptr.is_null() {
                    return Some(unsafe { Vec::from_raw_parts(ptr, self.size, self.size) });
                }
            }
        }
        None
    }

    fn put_back_a_buffer(&self, buf: Buffer) {
        assert_eq!(buf.len(), self.size);
        let buf = buf.into_boxed_slice();
        let ptr = Box::into_raw(buf) as *mut u8;
        for buf in &self.buffers {
            if let Ok(..) =
                buf.compare_exchange(ptr::null_mut(), ptr, Ordering::Relaxed, Ordering::Relaxed)
            {
                return;
            }
        }
        // leak buf
        panic!("more buffers were returned than given out!");
    }
}

pub async fn read(mut buf: Buffer, amnt: usize, at: u64, from: &File) -> (io::Result<()>, Buffer) {
    let mut read_total = 0;
    while read_total < amnt {
        let ret = from
            .read_at(buf.slice(read_total..amnt), at + read_total as u64)
            .await;
        buf = ret.1.into_inner();
        read_total += match ret.0 {
            Ok(x) => x,
            Err(e) => return (Err(e.into()), buf),
        }
    }
    (Ok(()), buf)
}

pub async fn write(mut buf: Buffer, amnt: usize, at: u64, into: &File) -> (io::Result<()>, Buffer) {
    let mut write_total = 0;
    while write_total != amnt {
        let ret = into
            .write_at(buf.slice(write_total..amnt), at + write_total as u64)
            .await;
        buf = ret.1.into_inner();

        let written = match ret.0 {
            Ok(x) => x,
            Err(e) => return (Err(e.into()), buf),
        };
        if written != 0 {
            write_total += written;
        } else {
            panic!("Write failed: 0 bytes written from buffer");
        }
    }
    (Ok(()), buf)
}

#[derive(Debug, thiserror::Error)]
pub enum AllocErr {
    #[error("Ran out of buffers to use")]
    OutOfBuffers,
    #[error("Type used was larger than the buffer size")]
    TypeTooLarge,
    #[error("I/O Error: {0:#}")]
    IO(#[from] io::Error),
}

pub struct Alloc {
    bufs: Buffers,
    file: File,
}

impl Alloc {
    pub async fn read<'db, T: DBStruct>(&'db self, at: u64) -> Result<DBRef<'db, T>, AllocErr> {
        // Saftey (for later) - buf must be large enough to contain T
        if size_of::<T>() > self.bufs.size {
            return Err(AllocErr::TypeTooLarge);
        }
        let buf = self.bufs.get_a_buffer().ok_or(AllocErr::OutOfBuffers)?;
        let (res, buf) = read(buf, size_of::<T>(), at, &self.file).await;
        res?;
        Ok(DBRef {
            buf: MaybeUninit::new(buf),
            addr: at,
            alloc: self,
            ty: PhantomData,
        })
    }
}

type Buffer = Vec<u8>;

// safe to transmute to bytes (no padding)
pub unsafe trait DBStruct: Sized {}

pub struct DBRef<'db, T: DBStruct> {
    // will only be de-initialized on Drop::drop being called
    buf: MaybeUninit<Buffer>,
    addr: u64,
    alloc: &'db Alloc,
    ty: PhantomData<T>,
}

impl<'db, T: DBStruct> DBRef<'db, T> {
    /// Write this content back to the file, and return the buffer
    pub async fn sync(self) -> Result<(), io::Error> {
        let addr = self.addr;
        let alloc = self.alloc;
        // Saftey: `self` is forgotten directly after this, so the drop code that would call assume_init_read again is not run
        let buf = unsafe { self.buf.assume_init_read() };
        forget(self);
        let (res, buf) = write(buf, size_of::<T>(), addr, &alloc.file).await;
        res?;
        alloc.bufs.put_back_a_buffer(buf);
        Ok(())
    }

    /// Return the buffer, but do not write the content back to the file
    pub async fn free(self) {
        let alloc = self.alloc;
        // Saftey: `self` is forgotten directly after this, so the drop code that would call assume_init_read again is not run
        let buf = unsafe { self.buf.assume_init_read() };
        forget(self);
        alloc.bufs.put_back_a_buffer(buf);
    }
}

impl<'db, T: DBStruct> Deref for DBRef<'db, T> {
    type Target = T;
    fn deref<'a>(&'a self) -> &'a Self::Target {
        unsafe {
            // only functions that drop self invalidate buf, meaning it is impossible for this to be valid
            let r = self.buf.assume_init_ref();
            debug_assert!(r.capacity() > 0);
            // validated in Alloc::read()
            debug_assert!(r.len() >= size_of::<T>());
            // Buffer contains at least enough space for T
            let ptr = r.as_ptr() as *const T;
            // Buffer is valid for 'a, T impl DBStruct and thus can be safely transmuted from/to bytes
            transmute::<*const T, &'a Self::Target>(ptr)
        }
    }
}

impl<'db, T: DBStruct> DerefMut for DBRef<'db, T> {
    fn deref_mut<'a>(&'a mut self) -> &'a mut Self::Target {
        unsafe {
            // only functions that drop self invalidate buf, meaning it is impossible for this to be valid
            let r = self.buf.assume_init_mut();
            debug_assert!(r.capacity() > 0);
            // validated in Alloc::read()
            debug_assert!(r.len() >= size_of::<T>());
            // Buffer contains at least enough space for T
            let ptr = r.as_mut_ptr() as *mut T;
            // Buffer is valid for 'a, T impl DBStruct and thus can be safely transmuted from/to bytes
            transmute::<*mut T, &'a mut Self::Target>(ptr)
        }
    }
}

impl<'db, T: DBStruct> Drop for DBRef<'db, T> {
    fn drop(&mut self) {
        error!("DBRef dropped improperly (without calling sync or free) - the buffer will be returned to the db, but no data will be written");
        // Saftey: drop() is only called once, and this is the final use of `self.buf`
        let buf = unsafe { self.buf.assume_init_read() };
        self.alloc.bufs.put_back_a_buffer(buf)
    }
}

pub fn main() -> Result<()> {
    tokio_uring::start(async {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open("test.tsdb3")
            .await?;
        const LARGE: [u8; 4096] = [0u8; 4096];
        file.write_at(&LARGE[..], 0).await.0?;
        // 10x 1kB buffers
        let bufs = Buffers::new(10, 1024);
        let alloc = Alloc { bufs, file };

        #[repr(transparent)]
        struct Data {
            val: u32,
        }

        unsafe impl DBStruct for Data {}

        let mut data = alloc.read::<Data>(0).await?;
        data.val = 0xEFBEADDE;
        data.sync().await?;

        // let (res, buf) = read(
        //     bufs.get_a_buffer().expect("ran out of buffers"),
        //     10,
        //     23,
        //     &file,
        // )
        // .await;
        // res?;

        // let (res, buf) = write(buf, 10, 33, &file).await;
        // res?;
        // bufs.put_back_a_buffer(buf);

        alloc.file.close().await?;
        Ok(())
    })
}
