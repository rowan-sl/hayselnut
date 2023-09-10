use num_enum::TryFromPrimitive;
use uuid::Uuid;

use super::super::{
    alloc::{
        object::Object,
        util::{test::TestStore, ChunkedLinkedList},
    },
    repr,
    repr::DBEntrypoint,
    Database,
};

mod verified_add_periodic;
mod verified_add_sporadic;

pub async fn verify_database_content(
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
                info!("{:?}\n{:?}", index.after, &*data);
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
                info!("{:?}\n{:?}", index.after, &*data);
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
