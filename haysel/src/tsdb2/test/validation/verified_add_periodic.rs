use chrono::{DateTime, NaiveDateTime};
use tracing_test::traced_test;
use uuid::Uuid;

use crate::tsdb2::{
    test::{get_db, simple_add, validation},
    Database,
};

// same as simple, but verify that the data is all there (at the end of the test)
#[tokio::test]
#[traced_test]
async fn at_end() {
    let (station, channel, expected_data, db) = simple_add::do_simple_add_periodic().await;
    let expected_data = expected_data
        .into_iter()
        .map(|(a, b)| (a.timestamp(), b))
        .collect::<Vec<_>>();
    if validation::verify_database_content(station, channel, expected_data, db).await {
        panic!("Discrepancy detected: see logs");
    }
}

// same as above, but check for data after every step
#[tokio::test]
#[traced_test]
async fn every_step() {
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
        DateTime::from_naive_utc_and_offset(
            NaiveDateTime::from_timestamp_opt(__time, 0).unwrap(),
            chrono::Utc,
        )
    };

    const ENTRIES: usize = 50;
    let mut data = Vec::with_capacity(ENTRIES);
    let mut err = false;
    for i in 0..ENTRIES {
        let time = get_time();
        let reading = rand::random::<f32>();
        data.push((time, reading));
        db.add_data(station_id, channel_id, time, reading)
            .await
            .expect("failed to add entry");
        let data_clone = data.clone();
        let db_clone = Database {
            alloc: db.alloc.clone(),
        };
        if validation::verify_database_content(
            station_id,
            channel_id,
            data_clone
                .into_iter()
                .map(|(a, b)| (a.timestamp(), b))
                .collect(),
            db_clone,
        )
        .await
        {
            error!("Discrepancy found adding entry {}/{ENTRIES} : see logs for details (if this was not the first one found, this may be innacurate)", i+1);
            err = true;
            break;
        }
    }
    if err {
        panic!("Discrepancy detected, see logs for details");
    }
}
