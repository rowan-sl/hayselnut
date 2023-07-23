use tracing_test::traced_test;

use super::{util::test::TestStore, Allocator};

#[tokio::test]
#[traced_test]
async fn initializing_allocator_doesnt_crash() {
    let store = TestStore::default();
    Allocator::new(store)
        .await
        .expect("failed to create allocator");
}

#[tokio::test]
#[traced_test]
async fn allocate_some_stuff() {
    let mut alloc = Allocator::new(TestStore::default())
        .await
        .expect("failed to create allocator");
    alloc.allocate::<[u8; 512]>().await.unwrap();
    alloc.allocate::<[u128; 16]>().await.unwrap();
}
