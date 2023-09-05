use chrono::{DateTime, NaiveDateTime, Utc};
use num_enum::TryFromPrimitive;
use tracing_test::traced_test;
use uuid::Uuid;

use super::{
    alloc::{
        object::Object,
        util::{test::TestStore, ChunkedLinkedList},
    },
    repr,
    repr::DBEntrypoint,
    Database,
};

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
    let (_station, _channel, _data, db) = do_simple_add_data_periodic().await;
    db.close().await.expect("failed to close database")
}

async fn do_simple_add_data_periodic(
) -> (Uuid, Uuid, Vec<(DateTime<Utc>, f32)>, Database<TestStore>) {
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

// same as simple, but verify that the data is all there
#[tokio::test]
#[traced_test]
async fn verified_add_data_periodic() {
    let (station, channel, expected_data, db) = do_simple_add_data_periodic().await;
    let expected_data = expected_data
        .into_iter()
        .map(|(a, b)| (a.timestamp(), b))
        .collect::<Vec<_>>();
    if verify_database_content(station, channel, expected_data, db).await {
        panic!("Discrepancy detected: see logs");
    }
}

async fn verify_database_content(
    station: Uuid,
    channel: Uuid,
    mut expected_data: Vec<(i64, f32)>,
    mut db: Database<TestStore>,
) -> bool {
    let entrypoint = db
        .alloc
        .get_entrypoint()
        .await
        .unwrap()
        .cast::<DBEntrypoint>();
    let entrypoint = Object::new_read(&mut db.alloc, entrypoint).await.unwrap();
    let station =
        ChunkedLinkedList::find(entrypoint.stations.map, &mut db.alloc, |x| x.id == station)
            .await
            .unwrap()
            .expect("did not find requested station")
            .0;
    let (channel, _channel_idx) =
        ChunkedLinkedList::find(station.channels, &mut db.alloc, |x| x.id == channel)
            .await
            .unwrap()
            .expect("did not find requested channel");
    let gtype = repr::DataGroupType::try_from_primitive(channel.metadata.group_type)
        .expect("invalid group type");

    let mut index = if channel.data.is_null() {
        if expected_data.is_empty() {
            db.close().await.expect("failed to close db");
            return false;
        } else {
            panic!("Expected to find data, but there was none!");
        }
    } else {
        Object::new_read(&mut db.alloc, channel.data).await.unwrap()
    };

    let mut all_data = Vec::with_capacity(expected_data.len() + expected_data.len() / 100);
    loop {
        match gtype {
            repr::DataGroupType::Periodic => {
                let data = Object::new_read(&mut db.alloc, unsafe { index.group.periodic })
                    .await
                    .unwrap();
                for i in 0..index.used {
                    let reading = data.data[i as usize];
                    let time =
                        index.after + (data.avg_dt as i64 * i as i64 + data.dt[i as usize] as i64);
                    all_data.push((time, reading));
                }
            }
            repr::DataGroupType::Sporadic => {
                let data = Object::new_read(&mut db.alloc, unsafe { index.group.sporadic })
                    .await
                    .unwrap();
                for i in 0..index.used {
                    let reading = data.data[i as usize];
                    let time = index.after + data.dt[i as usize] as i64;
                    all_data.push((time, reading));
                }
            }
        }
        if index.next.is_null() {
            break;
        } else {
            let next = index.next;
            index.dispose_immutated();
            index = Object::new_read(&mut db.alloc, next).await.unwrap();
        }
    }

    expected_data.sort_by_key(|x| x.0);
    all_data.sort_by_key(|x| x.0);

    let mut err = false;
    trace!("expected: {expected_data:?}");
    trace!("found:    {all_data:?}");

    loop {
        if expected_data.is_empty() || all_data.is_empty() {
            break;
        }
        let mut expected = vec![expected_data.remove(0)];
        let time = expected[0].0;
        while expected_data
            .get(0)
            .map(|x| expected[0].0 == x.0)
            .unwrap_or(false)
        {
            expected.push(expected_data.remove(0));
        }
        let mut expected: Vec<f32> = expected
            .into_iter()
            .map(|(_time, reading)| reading)
            .collect();
        expected.sort_by(f32::total_cmp);
        let mut found = vec![all_data.remove(0)];
        while all_data.get(0).map(|x| found[0].0 == x.0).unwrap_or(false) {
            found.push(all_data.remove(0));
        }
        let mut found: Vec<f32> = found.into_iter().map(|(_time, reading)| reading).collect();
        found.sort_by(f32::total_cmp);

        for &expected_value in &expected {
            let Some(&found_value) = found.get(0) else {
                error!("Discrepancy found at t={time}\n\texpected: {expected:?}\n\tfound:    {found:?}");
                err = true;
                break;
            };
            found.remove(0);
            if expected_value != found_value {
                error!("Discrepancy found at t={time}\n\texpected: {expected:?}\n\tfound:    {found:?}");
                err = true;
                break;
            }
        }
    }

    if expected_data.is_empty() != all_data.is_empty() {
        error!("Discrepancy found: one of expected data / found data has extra elements!\n\texpected: {expected_data:?}\n\tfound: {all_data:?}");
        err = true;
    }

    db.close().await.expect("failed to close database");
    return err;
}

#[tokio::test]
#[traced_test]
async fn verified_every_step_add_data_periodic() {
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

    const ENTRIES: usize = 5;
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
        if verify_database_content(
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
        }
    }
    if err {
        panic!("Discrepancy detected, see logs for details");
    }
}
