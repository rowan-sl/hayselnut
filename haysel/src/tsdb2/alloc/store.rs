use std::{error::Error, mem};

use zerocopy::{AsBytes, FromBytes};

use super::ptr::{Ptr, Void};

pub mod disk;
pub mod raid;
#[cfg(test)]
pub mod test;

/// trait that all storage backings for any allocator must implement.
#[async_trait::async_trait]
pub trait UntypedStorage: Send + 'static {
    type Error: Error + Sync + Send + 'static;
    async fn read_buf(
        &mut self,
        at: Ptr<Void>,
        amnt: u64,
        into: &mut [u8],
    ) -> Result<(), Self::Error>;
    async fn write_buf(&mut self, at: Ptr<Void>, amnt: u64, from: &[u8])
        -> Result<(), Self::Error>;
    async fn close(self) -> Result<(), Self::Error>;
    async fn size(&mut self) -> Result<u64, Self::Error>;
    async fn expand_by(&mut self, amnt: u64) -> Result<(), Self::Error>;
    /// is resizing this store permitted.
    /// if not, expand_by is *expected* to error, but may succeed (if it turns out this function is lying)
    /// either way, expand_by is not premitted to panic if this function returns false
    ///
    /// this is not used by the normal database, but is by the raid storage backing
    async fn resizeable(&mut self) -> Result<bool, Self::Error>;
}

#[async_trait::async_trait]
pub trait Storage: UntypedStorage + Send + 'static {
    async fn read_typed<T: FromBytes>(&mut self, at: Ptr<T>) -> Result<T, Self::Error> {
        let mut buf = vec![0; mem::size_of::<T>()];
        self.read_buf(at.cast::<Void>(), buf.len() as u64, &mut buf)
            .await?;
        Ok(T::read_from(buf.as_slice()).unwrap())
    }
    async fn write_typed<T: AsBytes + Sync + Send>(
        &mut self,
        at: Ptr<T>,
        from: &T,
    ) -> Result<(), Self::Error> {
        self.write_buf(
            at.cast::<Void>(),
            mem::size_of::<T>() as u64,
            from.as_bytes(),
        )
        .await
    }
}

#[async_trait::async_trait]
impl<T: ?Sized + UntypedStorage + Send + 'static> Storage for T {}
