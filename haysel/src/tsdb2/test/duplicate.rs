use tracing_test::traced_test;
use uuid::Uuid;

use super::get_db;

#[tokio::test]
#[traced_test]
async fn duplicate_station_errors() {
    let mut db = get_db().await;
    let station_id = Uuid::new_v4();
    db.add_station(station_id)
        .await
        .expect("add_station failed");
    db.add_station(station_id)
        .await
        .expect_err("add_station incorrectly allowed creating duplicate entries");
    db.close().await.expect("failed to close database")
}

#[tokio::test]
#[traced_test]
async fn duplicate_channel_errors() {
    let mut db = get_db().await;
    let station_id = Uuid::new_v4();
    db.add_station(station_id)
        .await
        .expect("add_station failed");
    let channel_id = Uuid::new_v4();
    db.add_channel(
        station_id,
        channel_id,
        crate::tsdb2::repr::DataGroupType::Periodic,
    )
    .await
    .expect("add_channel failed");
    db.add_channel(
        station_id,
        channel_id,
        crate::tsdb2::repr::DataGroupType::Periodic,
    )
    .await
    .expect_err("add_channel incorrectly allowed creating duplicate entries of the same type");
    db.add_channel(
        station_id,
        channel_id,
        crate::tsdb2::repr::DataGroupType::Sporadic,
    )
    .await
    .expect_err("add_channel incorrectly allowed creating duplicate entries of different types");
    db.close().await.expect("failed to close database")
}
