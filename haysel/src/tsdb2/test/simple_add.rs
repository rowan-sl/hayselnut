use chrono::{DateTime, NaiveDateTime, Utc};
use tracing_test::traced_test;
use uuid::Uuid;

use super::{
    super::{alloc::util::test::TestStore, Database},
    get_db,
};

#[tokio::test]
#[traced_test]
async fn periodic() {
    let (_station, _channel, _data, db) = do_simple_add_periodic().await;
    db.close().await.expect("failed to close database")
}

#[tokio::test]
#[traced_test]
async fn sporadic() {
    let (_station, _channel, _data, db) = do_simple_add_sporadic().await;
    db.close().await.expect("failed to close database")
}

pub async fn do_simple_add_periodic() -> (Uuid, Uuid, Vec<(DateTime<Utc>, f32)>, Database<TestStore>)
{
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

    const ENTRIES: usize = 10;
    let mut data = Vec::with_capacity(ENTRIES);
    for _ in 0..ENTRIES {
        let time = get_time();
        let reading = rand::random::<f32>();
        data.push((time, reading));
        db.add_data(station_id, channel_id, time, reading)
            .await
            .expect("failed to add entry");
    }

    (station_id, channel_id, data, db)
}

pub async fn do_simple_add_sporadic() -> (Uuid, Uuid, Vec<(DateTime<Utc>, f32)>, Database<TestStore>)
{
    let mut db = get_db().await;
    let station_id = Uuid::new_v4();
    db.add_station(station_id)
        .await
        .expect("add_station failed");
    let channel_id = Uuid::new_v4();
    db.add_channel(
        station_id,
        channel_id,
        crate::tsdb2::repr::DataGroupType::Sporadic,
    )
    .await
    .expect("add_channel (sporadic) failed");

    // test negative numbers while we are at it
    let mut __time = -500i64;
    let mut get_time = || {
        let x = 120.0 + rand::random::<f32>() * 1200.0;
        __time += x as i64;
        DateTime::from_utc(
            NaiveDateTime::from_timestamp_opt(__time, 0).unwrap(),
            chrono::Utc,
        )
    };

    const ENTRIES: usize = 10;
    let mut data = Vec::with_capacity(ENTRIES);
    for _ in 0..ENTRIES {
        let time = get_time();
        let reading = rand::random::<f32>();
        data.push((time, reading));
        db.add_data(station_id, channel_id, time, reading)
            .await
            .expect("failed to add entry");
    }

    (station_id, channel_id, data, db)
}
