use uuid::Uuid;

use super::super::{alloc::store::test::TestStore, Database};

mod verified_add_periodic;
mod verified_add_sporadic;

pub async fn verify_database_content(
    station: Uuid,
    channel: Uuid,
    mut expected_data: Vec<(i64, f32)>,
    mut db: Database<TestStore>,
) -> bool {
    let mut all_data = db
        .query()
        .with_station(station)
        .with_channel(channel)
        .verify()
        .unwrap()
        .execute()
        .await
        .unwrap()
        .into_iter()
        .map(|(time, data)| (time.timestamp(), data))
        .collect::<Vec<_>>();
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
