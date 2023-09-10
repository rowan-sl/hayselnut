use tracing_test::traced_test;
use uuid::Uuid;

use super::{alloc::util::test::TestStore, Database};

mod duplicate;
mod simple_add;
mod validation;

#[instrument]
async fn get_db() -> Database<TestStore> {
    trace!("creating TestStore");
    let store = TestStore::default();
    trace!("creating DB");
    let db = Database::new(store, false)
        .await
        .expect("error initializing database");
    db
}

#[tokio::test]
#[traced_test]
async fn init_db() {
    let db = get_db().await;
    db.close().await.expect("failed to close database")
}

#[tokio::test]
#[traced_test]
async fn create_structure() {
    let mut db = get_db().await;
    let station_id = Uuid::new_v4();
    db.add_station(station_id)
        .await
        .expect("add_station failed");
    let station_id2 = Uuid::new_v4();
    db.add_station(station_id2)
        .await
        .expect("add_station failed");
    let channel_id1 = Uuid::new_v4();
    let channel_id2 = Uuid::new_v4();
    db.add_channel(
        station_id,
        channel_id1,
        crate::tsdb2::repr::DataGroupType::Periodic,
    )
    .await
    .expect("add_channel (periodic) failed");
    db.add_channel(
        station_id,
        channel_id2,
        crate::tsdb2::repr::DataGroupType::Sporadic,
    )
    .await
    .expect("add_channel (sporadic) failed");
    db.close().await.expect("failed to close database")
}
