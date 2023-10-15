use crate::tsdb2::alloc::ptr::{Ptr, Void};

use super::UntypedStorage;

#[derive(Debug, thiserror::Error)]
pub enum VoidError {}

pub enum VoidStorage {}

#[async_trait::async_trait]
impl UntypedStorage for VoidStorage {
    type Error = VoidError;
    async fn read_buf(
        &mut self,
        _at: Ptr<Void>,
        _amnt: u64,
        _into: &mut [u8],
    ) -> Result<(), Self::Error> {
        unreachable!()
    }
    async fn write_buf(
        &mut self,
        _at: Ptr<Void>,
        _amnt: u64,
        _from: &[u8],
    ) -> Result<(), Self::Error> {
        unreachable!()
    }
    async fn close(self) -> Result<(), Self::Error> {
        unreachable!()
    }
    async fn sync(&mut self) -> Result<(), Self::Error> {
        unreachable!()
    }
    async fn size(&mut self) -> Result<u64, Self::Error> {
        unreachable!()
    }
    async fn expand_by(&mut self, _amnt: u64) -> Result<(), Self::Error> {
        unreachable!()
    }
    /// is resizing this store permitted.
    /// if not, expand_by is *expected* to error, but may succeed (if it turns out this function is lying)
    /// either way, expand_by is not premitted to panic if this function returns false
    ///
    /// this is not used by the normal database, but is by the raid storage backing
    async fn resizeable(&mut self) -> Result<bool, Self::Error> {
        unreachable!()
    }
}
