use tracing_test::traced_test;

use super::{error::AllocError, ptr::Ptr, store::test::TestStore, Allocator};

#[tokio::test]
#[traced_test]
async fn initializing_allocator_doesnt_crash() {
    let store = TestStore::default();
    let alloc = Allocator::new(store, false)
        .await
        .expect("failed to create allocator");
    alloc.close().await.expect("failed to shutdown allocator");
}

#[tokio::test]
#[traced_test]
async fn allocate_some_stuff() {
    let mut alloc = Allocator::new(TestStore::default(), false)
        .await
        .expect("failed to create allocator");
    alloc.allocate::<[u8; 512]>().await.unwrap();
    alloc.allocate::<[u128; 16]>().await.unwrap();
    alloc.close().await.expect("failed to shutdown allocator")
}

#[instrument(skip(alloc))]
async fn allocate_a_thing(alloc: &mut Allocator<TestStore>) -> (Ptr<[u128; 16]>, [u128; 16]) {
    let ptr = alloc
        .allocate::<[u128; 16]>()
        .await
        .expect("allocating failed");
    let val = [0x928374ABEE1; 16];
    alloc.write(val, ptr).await.expect("writing failed");
    let read = alloc.read(ptr).await.expect("reading failed");
    assert_eq!(val, read, "data written and data read are mismatched");
    (ptr, val)
}

#[tokio::test]
#[traced_test]
async fn allocate_and_use() {
    let mut alloc = Allocator::new(TestStore::default(), false)
        .await
        .expect("failed to create allocator");
    let (ptr, _) = allocate_a_thing(&mut alloc).await;
    alloc.free(ptr).await.expect("failed to free data");
    alloc.close().await.expect("failed to shutdown allocator");
}

#[tokio::test]
#[traced_test]
async fn use_after_free_errors() {
    let mut alloc = Allocator::new(TestStore::default(), false)
        .await
        .expect("failed to create allocator");
    let (ptr, _) = allocate_a_thing(&mut alloc).await;
    alloc.free(ptr).await.expect("failed to free data");
    assert_eq!(
        alloc.free(ptr).await,
        Err(AllocError::PointerStatus),
        "double-free did not error!"
    );
}

#[tokio::test]
#[traced_test]
async fn bad_pointer_errors() {
    let mut alloc = Allocator::new(TestStore::default(), false)
        .await
        .expect("failed to create allocator");
    let (ptr, _) = allocate_a_thing(&mut alloc).await;
    let _ = allocate_a_thing(&mut alloc).await;
    let expected_error = AllocError::PointerInvalid;
    assert_eq!(
        alloc.free(ptr.offset(4)).await,
        Err(expected_error.clone()),
        "`free` did not catch a bad pointer"
    );
    assert_eq!(
        alloc.write(Default::default(), ptr.offset(4)).await,
        Err(expected_error.clone()),
        "`write` did not catch a bad pointer"
    );
    assert_eq!(
        alloc.read(ptr.offset(4)).await,
        Err(expected_error),
        "`read` did not catch a bad pointer"
    );
    alloc.close().await.expect("failed to shutdown allocator");
}
