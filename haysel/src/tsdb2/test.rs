use chrono::{DateTime, NaiveDateTime};
use tracing_test::traced_test;
use uuid::Uuid;

use super::{alloc::util::test::TestStore, Database};

#[instrument]
async fn get_db() -> Database<TestStore> {
    let store = TestStore::default();
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

#[tokio::test]
#[traced_test]
async fn simple_add_data_periodic() {
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
    .expect("add_channel (periodic) failed");

    // test negative numbers while we are at it
    let mut __time = -500i64;
    let mut get_time = || {
        let x = if rand::random::<f64>() < 0.05 {
            // large, inconsistant delay
            120.0 + rand::random::<f32>() * 120.0
        } else {
            // small, consistant delay (about 30s with +-4s deviation)
            30.0 + (rand::random::<f32>() - 0.5) * 4.0
        };
        __time += x as i64;
        DateTime::from_utc(
            NaiveDateTime::from_timestamp_opt(__time, 0).unwrap(),
            chrono::Utc,
        )
    };

    const ENTRIES: usize = 5_000;
    for _ in 0..ENTRIES {
        db.add_data(station_id, channel_id, get_time(), rand::random::<f32>())
            .await
            .expect("failed to add entry");
    }

    db.close().await.expect("failed to close database")
}
