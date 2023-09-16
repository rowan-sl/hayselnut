use chrono::{DateTime, NaiveDateTime, Utc};
use num_enum::TryFromPrimitive;

use self::builder::QueryParams;

use super::{
    alloc::{object::Object, util::ChunkedLinkedList, Storage, UntypedStorage},
    error::DBError,
    repr::{self, DBEntrypoint, DataGroupIndex},
};

pub mod builder;

impl<'a, Store: Storage + Send> QueryParams<'a, Store> {
    #[instrument(skip(self))]
    pub async fn execute(
        self,
    ) -> Result<Vec<(DateTime<Utc>, f32)>, DBError<<Store as UntypedStorage>::Error>> {
        let QueryParams {
            db,
            station,
            channel,
            max_results,
            before_time,
            after_time,
        } = self;
        let (station, channel) = (station.unwrap(), channel.unwrap());
        let mut readings = Vec::new();

        let entrypoint = db.alloc.get_entrypoint().await?.cast::<DBEntrypoint>();
        let entrypoint = Object::new_read(&mut db.alloc, entrypoint).await?;
        let (station, _) =
            ChunkedLinkedList::find(entrypoint.stations.map, &mut db.alloc, |x| x.id == station)
                .await?
                .ok_or(DBError::NoSuchStation)?;
        let (channel, _) =
            ChunkedLinkedList::find(station.channels, &mut db.alloc, |x| x.id == channel)
                .await?
                .ok_or(DBError::NoSuchChannel)?;
        let gtype = repr::DataGroupType::try_from_primitive(channel.metadata.group_type)
            .expect("invalid group type");

        let mut prev_index = None::<Object<DataGroupIndex>>;
        let mut index = if channel.data.is_null() {
            return Ok(readings);
        } else {
            readings.reserve(max_results.unwrap_or(0));
            Object::new_read(&mut db.alloc, channel.data).await?
        };

        if after_time.is_none() && before_time.is_none() && max_results.is_none() {
            warn!(
                "A query has been made for *all* values in the database. This may take a moment..."
            );
        }

        'read: loop {
            'read_index: {
                // note: makes HEAVY use of operator short-circuting (and true=ok, false=problem)
                // is this index after the `after` time (if specified)
                let cond_this_after =
                    after_time.is_none() || after_time.unwrap().timestamp() < index.after;
                // is the previous index after the `after` time (if specified)
                let cond_prev_after = after_time.is_none()
                    || (prev_index.is_none()
                        || (after_time.unwrap().timestamp() < prev_index.as_ref().unwrap().after));
                // only if the previous AND current indexes are BEFORE the after time
                // can we exit because of this (bolth false) (to catch any elements that are in-between in the boundary index)
                if !(cond_this_after || cond_prev_after) {
                    break 'read;
                }
                // is this index before the `before` time (if specified)
                // if this fails, we do not exit, but rather skip to the next (if it exists)
                // because it could be before the `before` time
                if !(before_time.is_none() || before_time.unwrap().timestamp() > index.after) {
                    break 'read_index;
                }

                let validate = |time, len| {
                    (before_time.is_none() || before_time.unwrap().timestamp() > time)
                        && (after_time.is_none() || after_time.unwrap().timestamp() < time)
                        && (max_results.is_none() || max_results.unwrap() < len)
                };

                match gtype {
                    repr::DataGroupType::Periodic => {
                        let data = Object::new_read(&mut db.alloc, unsafe { index.group.periodic })
                            .await
                            .unwrap();
                        for i in 0..index.used {
                            let reading = data.data[i as usize];
                            let time = index.after
                                + (data.avg_dt as i64 * i as i64 + data.dt[i as usize] as i64);
                            if validate(time, readings.len()) {
                                readings.push((
                                    DateTime::from_utc(
                                        NaiveDateTime::from_timestamp_opt(time, 0).unwrap(),
                                        Utc,
                                    ),
                                    reading,
                                ))
                            } else {
                                continue;
                            }
                        }
                    }
                    repr::DataGroupType::Sporadic => {
                        let data = Object::new_read(&mut db.alloc, unsafe { index.group.sporadic })
                            .await
                            .unwrap();
                        for i in 0..index.used {
                            let reading = data.data[i as usize];
                            let time = index.after + data.dt[i as usize] as i64;
                            if validate(time, readings.len()) {
                                readings.push((
                                    DateTime::from_utc(
                                        NaiveDateTime::from_timestamp_opt(time, 0).unwrap(),
                                        Utc,
                                    ),
                                    reading,
                                ))
                            } else {
                                continue;
                            }
                        }
                    }
                }
            }
            if index.next.is_null() {
                break;
            } else {
                let next = index.next;
                prev_index.map(|prev| prev.dispose_immutated());
                prev_index = Some(index);
                index = Object::new_read(&mut db.alloc, next).await.unwrap();
            }
        }

        Ok(readings)
    }
}
