#[cfg(test)]
use ::{
    chrono::{DateTime, Utc},
    std::collections::HashSet,
    uuid::Uuid,
};

#[cfg(test)]
use super::DB;

#[test]
fn create_new_db() {
    let mut db = DB::new_in_ram(4096).unwrap();
    db.init();
}

/// TOOD: test more things
#[test]
#[should_panic]
fn op_without_init() {
    let mut db = DB::new_in_ram(4096).unwrap();
    let sid = Uuid::new_v4();
    db.insert_station(sid);
}

#[test]
fn create_new_station() {
    let mut db = DB::new_in_ram(4096).unwrap();
    db.init();
    let sid = Uuid::new_v4();
    db.insert_station(sid);
    println!("Station created, verifying");
    let stations = db.get_stations().collect::<Vec<_>>();
    assert_eq!(stations, vec![&sid]);
}

#[test]
fn create_16_new_stations() {
    let mut db = DB::new_in_ram(1_000_000).unwrap();
    db.init();
    let mut set = HashSet::new();
    for _ in 0..16 {
        let sid = Uuid::new_v4();
        set.insert(sid);
        db.insert_station(sid);
    }
    println!("Station created, verifying");
    let stations = db.get_stations().copied().collect::<HashSet<_>>();
    assert_eq!(stations, set);
}

#[test]
fn create_new_channel() {
    // note: need moar bigger
    let mut db = DB::new_in_ram(10_000).unwrap();
    db.init();
    let sid = Uuid::new_v4();
    db.insert_station(sid);
    let cid = Uuid::new_v4();
    db.insert_channels(sid, [cid]);
    println!("Channel created, verifying");
    let channels = db.get_channels_for(sid).map(|x| x.collect::<Vec<_>>());
    assert_eq!(channels, Some(vec![&cid]));
}

#[test]
fn insert_data() {
    // note: need moar bigger
    let mut db = DB::new_in_ram(30_000).unwrap();
    db.init();
    let sid = Uuid::new_v4();
    db.insert_station(sid);
    let cid = Uuid::new_v4();
    db.insert_channels(sid, [cid]);
    let time = Utc::now();
    let reading = 5f32;
    db.insert_data(sid, cid, time, reading);
}

#[test]
fn insert_data_in_order() {
    // note: need moar bigger
    let mut db = DB::new_in_ram(30_000).unwrap();
    db.init();
    let sid = Uuid::new_v4();
    db.insert_station(sid);
    let cid = Uuid::new_v4();
    db.insert_channels(sid, [cid]);
    let time = Utc::now();
    let prev_time = time.checked_sub_days(chrono::Days::new(1)).unwrap();
    let reading = 5f32;
    db.insert_data(sid, cid, prev_time, reading);
    db.insert_data(sid, cid, time, reading);
}

#[test]
#[should_panic]
fn insert_data_backwards() {
    // note: need moar bigger
    let mut db = DB::new_in_ram(30_000).unwrap();
    db.init();
    let sid = Uuid::new_v4();
    db.insert_station(sid);
    let cid = Uuid::new_v4();
    db.insert_channels(sid, [cid]);
    let time = Utc::now();
    let prev_time = time.checked_sub_days(chrono::Days::new(1)).unwrap();
    let reading = 5f32;
    db.insert_data(sid, cid, time, reading);
    // ERROR: data must be in chronological order
    db.insert_data(sid, cid, prev_time, reading);
}

#[test]
fn query_data() {
    // note: need moar bigger
    let mut db = DB::new_in_ram(30_000).unwrap();
    db.init();
    let sid = Uuid::new_v4();
    db.insert_station(sid);
    let cid = Uuid::new_v4();
    db.insert_channels(sid, [cid]);
    let time = Utc::now();
    let reading = 5f32;
    db.insert_data(sid, cid, time, reading);
    let before = time.checked_add_days(chrono::Days::new(1)).unwrap();
    let after = time.checked_sub_days(chrono::Days::new(1)).unwrap();
    let res = db.qery_data_raw(sid, cid, after, before, 10);
    assert_eq!(
        res,
        vec![(
            DateTime::from_timestamp(time.timestamp(), 0).unwrap(),
            reading
        )]
    );
}
