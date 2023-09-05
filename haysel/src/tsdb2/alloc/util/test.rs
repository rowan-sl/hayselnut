use crate::tsdb2::alloc::{
    ptr::{Ptr, Void},
    Allocator, Storage,
};

#[derive(thiserror::Error, Clone, Debug, PartialEq)]
pub enum VoidError {}

#[derive(Default)]
pub struct TestStore {
    backing: Vec<u8>,
}

#[async_trait::async_trait]
impl Storage for TestStore {
    type Error = VoidError;
    async fn read_buf(
        &mut self,
        at: Ptr<Void>,
        amnt: u64,
        into: &mut [u8],
    ) -> Result<(), Self::Error> {
        into.copy_from_slice(&self.backing[at.addr as _..(at.addr + amnt) as _]);
        Ok(())
    }
    async fn write_buf(
        &mut self,
        at: Ptr<Void>,
        amnt: u64,
        from: &[u8],
    ) -> Result<(), Self::Error> {
        self.backing[at.addr as _..(at.addr + amnt) as _].copy_from_slice(from);
        Ok(())
    }
    async fn close(self) -> Result<(), Self::Error> {
        Ok(())
    }
    async fn size(&mut self) -> Result<u64, Self::Error> {
        Ok(self.backing.len() as _)
    }
    async fn expand_by(&mut self, amnt: u64) -> Result<(), Self::Error> {
        self.backing
            .extend_from_slice(vec![0; amnt as _].as_slice());
        Ok(())
    }
}

impl Clone for Allocator<TestStore> {
    fn clone(&self) -> Self {
        Self {
            store: TestStore {
                backing: self.store.backing.clone(),
            },
        }
    }
}
