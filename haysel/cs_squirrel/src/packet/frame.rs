use std::mem::size_of;

use zerocopy::{byteorder::network_endian::U16, Unalign};

use super::{Error, Result};

#[derive(Debug, PartialEq, Eq, Hash)]
#[repr(C, align(1))]
pub struct Frame {
    pub head: super::Head,
    // NOT the DST len of [data]. this is the number of valid bytes
    pub len: Unalign<U16>,
    // DST metadata of Self is the same as that of [u8]
    pub data: [u8],
}

/// size of the non-data portion of Frame
/// correctness relied on for saftey
pub const SIZE_OF_HEADER: usize = size_of::<super::Head>() + 2;

impl Frame {
    /// create instance of Self in `buf`, copying `data` to the appropreate location in `buf`
    ///
    /// `buf` is borrowed for `self`, `data` is borrowed for the lifetime of this function
    ///
    /// `data` must have a len less than `u16::MAX`
    ///
    /// `buf` must have a length of at least `SIZE_OF_HEADER` + `data.len()`
    /// this function **ONLY sets `len` and `data`, NOT `packet`, `responding_to`, or `packet_ty`**
    pub fn new<'src, 'back>(buf: &'back mut [u8], data: &'src [u8]) -> Result<&'back mut Self> {
        if buf.len() < SIZE_OF_HEADER + data.len() {
            return Err(Error::TooSmallWrite);
        }
        let frame = Self::mut_buf(buf)?;
        frame.len = Unalign::new(U16::new(data.len().try_into().unwrap()));
        frame.data.copy_from_slice(data);
        Ok(frame)
    }

    pub fn ref_buf<'buf>(buf: &'buf [u8]) -> Result<&'buf Self> {
        if buf.len() < SIZE_OF_HEADER {
            return Err(Error::TooSmallReadHeader);
        }
        let (ptr, len): (*const (), usize) = (buf as *const [u8]).to_raw_parts();
        let ptr = ::core::ptr::from_raw_parts::<Frame>(ptr, len - SIZE_OF_HEADER);
        Ok(unsafe { &*ptr })
    }

    pub fn mut_buf<'buf>(buf: &'buf mut [u8]) -> Result<&'buf mut Self> {
        if buf.len() < SIZE_OF_HEADER {
            return Err(Error::TooSmallReadHeader);
        }
        let (ptr, len): (*mut (), usize) = (buf as *mut [u8]).to_raw_parts();
        let ptr = ::core::ptr::from_raw_parts_mut::<Frame>(ptr, len - SIZE_OF_HEADER);
        Ok(unsafe { &mut *ptr })
    }

    /// gets the *valid* portion of data (data[0..self.len])
    /// returns None if self.len is out of the range of self.data
    pub fn data(&self) -> Result<&[u8]> {
        self.data
            .get(0..self.len.get().get().into())
            .ok_or(Error::TooSmallReadData)
    }
}
