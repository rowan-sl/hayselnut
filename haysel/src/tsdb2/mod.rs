//! # DB Hierarchy
//! {stations(by ID), chunked linked list} -> channels (by ID, so each one has only one type of simple data, and each sub-event has its own place)
//! {channels(by ID), chunked linked list} -> metadata, first data index chunk
//! metadata: data type (ID of it, each one has an associated size but this is stored elsewhere)
//! [data index chunk: (pointers to n-length chunks, each w/ start time, amnt full) and (pointer to the next chunk)]
//! [data chunk: n number of time offset (small uint seconds) from start time, then n number const(for the channel)-sized data]
//!
//! # allocator:
//!
//! type of data being stored: many repeats of things that are the same size (only a handfull of objects, and they are all const-size)
//! - use a linked list allocator design, but have seperate linked lists for each size of data.
//!
//! each allocated part consists of metadata, then the data. meteadata contains
//! - is this chunk free
//! - the length of the chunk
//! - pointer to the previous free chunk (of this size)
//! - pointer to the next free chunk (of this size) or null if there is none
//!
//! alloc header:
//! - [in chunked linked list, or possibly just have a max number of types]: head pointers to the linked list of free data for each size (and the associated size)

use chrono::{DateTime, Utc};
use mycelium::station::{capabilities::ChannelID, identity::StationID};
use num_enum::TryFromPrimitive;
use zerocopy::FromBytes;

use self::{
    alloc::{object::Object, ptr::Ptr, util::ChunkedLinkedList, Allocator, Storage},
    error::DBError,
    repr::DBEntrypoint,
};

pub mod alloc;
pub mod error;
pub mod helpers;
pub mod repr;
#[cfg(test)]
pub mod test;

mod tuning {
    // low values to force using the list functionality.
    // for real use, set higher
    pub const STATION_MAP_CHUNK_SIZE: usize = 16;
    pub const CHANNEL_MAP_CHUNK_SIZE: usize = 16;
    // pub const DATA_INDEX_CHUNK_SIZE: usize = 1;

    // optimize for the largest size (ish) that does not exceed the limit of the delta-time system.
    // must multiply by 2 to get a multiple of 8 (be a multiple of 4) (note: real value is 1 smaller than specified here)
    //
    // if periodic data chunks are consistantly left empty decrease this, or if they are consistantly full increase it.
    // TODO: specify size in a more customizeable way?
    pub const DATA_GROUP_PERIODIC_SIZE: usize = 4;
    /// honestly probably does not matter, as long as having one of them in the database is not too much of a big deal.
    pub const DATA_GROUP_SPORADIC_SIZE: usize = 4;
}

/// the database
pub struct Database<Store: Storage> {
    alloc: Allocator<Store>,
}

impl<Store: Storage + Send> Database<Store> {
    #[instrument(skip(store))]
    pub async fn new(
        store: Store,
        init_overwrite: bool,
    ) -> Result<Self, DBError<<Store as Storage>::Error>> {
        let mut alloc = Allocator::new(store, init_overwrite).await?;
        if alloc.get_entrypoint().await?.is_null() {
            warn!("initializing a new database");
            // the entrypoint is null, so this is a fresh database.

            // initialize the new entrypoint
            // this is the only thing we get access to when freshly opening
            // the database, and it is used to get at everything else
            let map = Object::new_alloc(
                &mut alloc,
                ChunkedLinkedList::<{ tuning::STATION_MAP_CHUNK_SIZE }, repr::Station> {
                    next: Ptr::null(),
                    used: 0,
                    data: [repr::Station::new_zeroed(); tuning::STATION_MAP_CHUNK_SIZE],
                },
            )
            .await?
            .dispose_sync(&mut alloc)
            .await?;

            let entrypoint = Object::new_alloc(
                &mut alloc,
                DBEntrypoint {
                    stations: repr::MapStations { map },
                },
            )
            .await?;
            alloc.set_entrypoint(entrypoint.pointer().cast()).await?;
            entrypoint.dispose_sync(&mut alloc).await?;
        } else {
            info!("found and opened existing database");
        }
        Ok(Self { alloc })
    }

    #[instrument(skip(self))]
    pub async fn station_exists(
        &mut self,
        id: StationID,
    ) -> Result<bool, DBError<<Store as Storage>::Error>> {
        let eptr = self.alloc.get_entrypoint().await?.cast::<DBEntrypoint>();
        let entry = Object::new_read(&mut self.alloc, eptr).await?;
        Ok(
            ChunkedLinkedList::find(entry.stations.map, &mut self.alloc, |s| s.id == id)
                .await?
                .is_some(),
        )
    }

    #[instrument(skip(self))]
    pub async fn add_station(
        &mut self,
        id: StationID,
    ) -> Result<(), DBError<<Store as Storage>::Error>> {
        let eptr = self.alloc.get_entrypoint().await?.cast::<DBEntrypoint>();
        let entry = Object::new_read(&mut self.alloc, eptr).await?;
        if self.station_exists(id).await? {
            return Err(DBError::Duplicate);
        }
        let channels = Object::new_alloc(&mut self.alloc, ChunkedLinkedList::empty_head())
            .await?
            .dispose_sync(&mut self.alloc)
            .await?;
        ChunkedLinkedList::push(
            entry.stations.map,
            &mut self.alloc,
            repr::Station { id, channels },
        )
        .await?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn add_channel(
        &mut self,
        to: StationID,
        id: ChannelID,
        kind: repr::DataGroupType,
    ) -> Result<(), DBError<<Store as Storage>::Error>> {
        let eptr = self.alloc.get_entrypoint().await?.cast::<DBEntrypoint>();
        let entry = Object::new_read(&mut self.alloc, eptr).await?;
        let station = ChunkedLinkedList::find(entry.stations.map, &mut self.alloc, |s| s.id == to)
            .await?
            .expect("did not find requested station")
            .0;
        if let Some(..) =
            ChunkedLinkedList::find(station.channels, &mut self.alloc, |c| c.id == id).await?
        {
            return Err(DBError::Duplicate);
        }
        ChunkedLinkedList::push(
            station.channels,
            &mut self.alloc,
            repr::Channel {
                id,
                metadata: repr::ChannelMetadata {
                    group_type: kind as u8,
                },
                _pad: Default::default(),
                data: Ptr::<repr::DataGroupIndex>::null(),
            },
        )
        .await?;
        entry.dispose_immutated();
        Ok(())
    }

    #[instrument(skip(self, index))]
    async fn verify_dt(
        &mut self,
        time: i64,
        gtype: repr::DataGroupType,
        index: &Object<repr::DataGroupIndex>,
    ) -> Result<bool, DBError<<Store as Storage>::Error>> {
        match gtype {
            repr::DataGroupType::Periodic => {
                let data =
                    Object::new_read(&mut self.alloc, unsafe { index.group.periodic }).await?;
                let entry_dt =
                    u64::try_from(time - index.after).expect("unreachable: delta-time negative");
                // here we reverse the relative delta calculation, arriving at a
                // individual offset from `entry.after` for each entry in `data`
                let mut abs_dt = helpers::rel_dt_to_abs(&data.dt[0..index.used as _], data.avg_dt);

                // make sure that the delta times are in order
                debug_assert!(abs_dt.is_sorted());

                // insert the new entry
                let ins_idx = abs_dt.binary_search(&entry_dt).map_or_else(|x| x, |x| x);
                if ins_idx as u64 == index.used {
                    abs_dt.push(entry_dt);
                } else {
                    let src = ins_idx..index.used as usize;
                    let dest = ins_idx + 1;
                    abs_dt.push(0); // more space for the slice operation to move into
                    abs_dt.copy_within(src.clone() /* why does Range not impl Copy */, dest);
                    abs_dt[ins_idx] = entry_dt;
                }

                // make sure that the delta times are still in order
                debug_assert!(abs_dt.is_sorted());

                // then calculate the average dt
                let Some(avg_dt) = helpers::calc_avg_dt(&abs_dt) else {
                    return Ok(false);
                };

                // then calculate individual offsets (delta from average)
                let Some(_) = helpers::calc_rel_dt(avg_dt, &abs_dt) else {
                    return Ok(false);
                };

                data.dispose_ignore();
                Ok(true)
            }
            repr::DataGroupType::Sporadic => Ok(u32::try_from(time - index.after).is_ok()),
        }
    }

    #[instrument(skip(self))]
    pub async fn add_data(
        &mut self,
        station_id: StationID,
        channel_id: ChannelID,
        time: DateTime<Utc>,
        reading: f32,
    ) -> Result<(), DBError<<Store as Storage>::Error>> {
        let time = time.timestamp();
        let entrypoint = self.alloc.get_entrypoint().await?.cast::<DBEntrypoint>();
        let entrypoint = Object::new_read(&mut self.alloc, entrypoint).await?;
        let station = ChunkedLinkedList::find(entrypoint.stations.map, &mut self.alloc, |x| {
            x.id == station_id
        })
        .await?
        .expect("did not find requested station")
        .0;
        let (mut channel, channel_idx) =
            ChunkedLinkedList::find(station.channels, &mut self.alloc, |x| x.id == channel_id)
                .await?
                .expect("did not find requested channel");
        let gtype = repr::DataGroupType::try_from_primitive(channel.metadata.group_type)
            .expect("invalid group type");
        let gtype_size = match gtype {
            repr::DataGroupType::Periodic => tuning::DATA_GROUP_PERIODIC_SIZE - 1,
            repr::DataGroupType::Sporadic => tuning::DATA_GROUP_SPORADIC_SIZE,
        } as u64;

        if channel.data.is_null() {
            let group = match gtype {
                repr::DataGroupType::Periodic => {
                    let mut data = repr::DataGroupPeriodic {
                        // only one entry
                        avg_dt: 0,
                        _pad: 0,
                        dt: [0; tuning::DATA_GROUP_PERIODIC_SIZE - 1],
                        data: [0.0; tuning::DATA_GROUP_PERIODIC_SIZE - 1],
                    };
                    data.data[0] = reading;
                    let pointer = Object::new_alloc(&mut self.alloc, data)
                        .await?
                        .dispose_sync(&mut self.alloc)
                        .await?;
                    repr::DataGroup { periodic: pointer }
                }
                repr::DataGroupType::Sporadic => {
                    let mut data = repr::DataGroupSporadic {
                        dt: [0; tuning::DATA_GROUP_SPORADIC_SIZE],
                        data: [0.0; tuning::DATA_GROUP_SPORADIC_SIZE],
                    };
                    data.data[0] = reading;
                    let pointer = Object::new_alloc(&mut self.alloc, data)
                        .await?
                        .dispose_sync(&mut self.alloc)
                        .await?;
                    repr::DataGroup { sporadic: pointer }
                }
            };
            let data = Object::new_alloc(
                &mut self.alloc,
                repr::DataGroupIndex {
                    after: time,
                    used: 1,
                    next: Ptr::null(),
                    group,
                },
            )
            .await?;
            let data_pointer = data.pointer();
            data.dispose_immutated();
            channel.data = data_pointer;
            channel_idx
                .write::<{ tuning::CHANNEL_MAP_CHUNK_SIZE }, _, _>(&mut self.alloc, channel)
                .await?;
            return Ok(());
        }

        let mut prev: Option<Object<repr::DataGroupIndex>> = None;
        let mut index = Object::new_read(&mut self.alloc, channel.data).await?;

        loop {
            match (
                // entry valid for this index or earlier [previously seen index]
                // true = newer, false = older
                time >= index.after,
                // space in this index for the data
                index.used < gtype_size,
                // next index exists?
                !index.next.is_null(),
                // will adding the entry to the current data [regardless if it fits] work (not overflow dt values)
                // ( assuming that time >= index.after )
                if time >= index.after {
                    self.verify_dt(time, gtype, &index).await?
                } else {
                    false
                },
            ) {
                // data goes in this entry
                // likely
                (
                    // time within this or [one that contained data from after `time`]
                    true,
                    // there is space
                    true,
                    // don't care about what came before, it is irrelevant here.
                    _,
                    // must be able to insert data successfully
                    true,
                ) => {
                    match gtype {
                        repr::DataGroupType::Periodic => {
                            let mut data =
                                Object::new_read(&mut self.alloc, unsafe { index.group.periodic })
                                    .await?;
                            let entry_dt = (time - index.after) as u64;
                            debug!("(Periodic, branch {}) - t={time} after={}, entry_dt={entry_dt} reading={reading} initial={:?}", line!(), index.after, &*data);
                            // FIXME: slo cod

                            // instead of calculating the change to avg_dt and then the entry's
                            // relative dt, which is hard, we simply recalculate it for all
                            // entries, which is easy, but slower
                            //
                            // here we reverse the relative delta calculation, arriving at a
                            // individual offset from `entry.after` for each entry in `data`
                            let mut abs_dt =
                                helpers::rel_dt_to_abs(&data.dt[0..index.used as _], data.avg_dt);

                            // make sure that the delta times are in order (to make sure the
                            // rel -> abs isnt completely wrong, and to make sure the
                            // `binary_search` used next works right)
                            debug_assert!(abs_dt.is_sorted());

                            // insert the new entry
                            let ins_idx = abs_dt.binary_search(&entry_dt).map_or_else(|x| x, |x| x);
                            if ins_idx as u64 == index.used {
                                abs_dt.push(entry_dt);
                            } else {
                                let src = ins_idx..index.used as usize;
                                let dest = ins_idx + 1;
                                abs_dt.push(0);
                                abs_dt.copy_within(
                                    src.clone(), /* why does Range not impl Copy */
                                    dest,
                                );
                                abs_dt[ins_idx] = entry_dt;
                                data.data.copy_within(src, dest);
                            }
                            data.data[ins_idx] = reading;
                            // increment `used`
                            index.used += 1;
                            // make sure that we didnt screw up the ordering
                            debug_assert!(abs_dt.is_sorted());
                            // then calculate the average dt
                            let avg_dt = helpers::calc_avg_dt(&abs_dt).unwrap();
                            // then calculate individual offsets (delta from average)
                            let rel_dt = helpers::calc_rel_dt(avg_dt, &abs_dt).unwrap();
                            // store the data
                            data.avg_dt = avg_dt;
                            data.dt[0..index.used as _].copy_from_slice(&rel_dt);
                            debug!("(Periodic, branch {}) - t={time} after={}, entry_dt={entry_dt} reading={reading} final={:?}", line!(), index.after, &*data);
                            data.dispose_sync(&mut self.alloc).await?;
                        }
                        repr::DataGroupType::Sporadic => {
                            let mut data =
                                Object::new_read(&mut self.alloc, unsafe { index.group.sporadic })
                                    .await?;
                            let entry_dt =
                                u32::try_from(time - index.after).expect("delta-time out of range");
                            // just make sure that binary_search wont produce garbage
                            debug_assert!(data.dt[0..index.used as _].is_sorted());
                            let ins_idx = data.dt[0..index.used as _]
                                .binary_search(&entry_dt)
                                .map_or_else(|x| x, |x| x);
                            if ins_idx as u64 != index.used {
                                let src = ins_idx..index.used as usize;
                                let dest = ins_idx + 1;
                                data.dt.copy_within(
                                    src.clone(), /* why does Range not impl Copy */
                                    dest,
                                );
                                data.data.copy_within(src, dest);
                            }
                            data.dt[ins_idx] = entry_dt;
                            data.data[ins_idx] = reading;
                            // increment `used`
                            index.used += 1;
                            // make sure that we didnt screw up the ordering
                            debug_assert!(data.dt[0..index.used as _].is_sorted());
                            data.dispose_sync(&mut self.alloc).await?;
                        }
                    }
                    break;
                }
                // we could have inserted this if there was space, but there isn't
                // likely (when an index fills up)
                (
                    // time within this or [one that contained data from after `time`]
                    true,
                    // there is not space
                    false,
                    // don't care about what came before, it is irrelevant here.
                    _,
                    // must be able to insert data successfully
                    // (otherwise this will be handled by the next thing)
                    true,
                ) => {
                    match gtype {
                        repr::DataGroupType::Periodic => {
                            // this is validated by the match statement, but just in case.
                            // - means that ALL entries in `data.dt` and `data.data` are currently in use.
                            // - this means we DO NOT have to do `data.field[0..index.used]` like in other cases
                            debug_assert_eq!(index.used, gtype_size);
                            let mut data =
                                Object::new_read(&mut self.alloc, unsafe { index.group.periodic })
                                    .await?;
                            let entry_dt = (time - index.after) as u64;
                            debug!("(Periodic, branch {}) - t={time} after={}, entry_dt={entry_dt} reading={reading} initial={:?}", line!(), index.after, &*data);
                            // FIXME: slo cod

                            // instead of calculating the change to avg_dt and then the entry's
                            // relative dt, which is hard, we simply recalculate it for all
                            // entries, which is easy, but slower
                            //
                            // here we reverse the relative delta calculation, arriving at a
                            // individual offset from `entry.after` for each entry in `data`
                            let mut abs_dt = helpers::rel_dt_to_abs(&data.dt, data.avg_dt);

                            // make sure that the delta times are in order (to make sure the
                            // rel -> abs isnt completely wrong, and to make sure the
                            // `binary_search` used next works right)
                            debug_assert!(abs_dt.is_sorted());

                            // insert the new entry
                            let ins_idx = abs_dt.binary_search(&entry_dt).map_or_else(|x| x, |x| x);
                            // this should be fine, since split_off(len) should [keep in abs_dt, leave abs_dt_keep empty] all elements and return Vec[]
                            // split_off(0) means that all data is kept (ends up in abs_dt_move)
                            let mut abs_dt_keep = abs_dt.split_off(ins_idx); // idx -> len
                            let mut abs_dt_move = abs_dt; // 0 -> idx
                            let mut data_move = data.data.to_vec();
                            let mut data_keep = data_move.split_off(ins_idx);
                            // push to the 0 -> idx end, so it ends up at <idx>
                            // if that is full (ins_idx=gtype_size), set it as the first value in the other.
                            // the new data will end up at the end of the array in the new chunk (or the start in the old one)
                            // if ins_idx=0, then the split_off call will do the right thing
                            if ins_idx as u64 == gtype_size {
                                debug_assert_eq!(abs_dt_move.len() as u64, gtype_size);
                                debug_assert_eq!(abs_dt_keep.len(), 0);
                                abs_dt_keep.push(entry_dt);
                                data_keep.push(reading);
                            } else {
                                abs_dt_move.push(entry_dt);
                                data_move.push(reading);
                            }
                            // don't need to index this since it is a vector *that contains only avlid values*
                            let avg_dt_keep = helpers::calc_avg_dt(&abs_dt_keep).unwrap();
                            let avg_dt_move = helpers::calc_avg_dt(&abs_dt_move).unwrap();
                            let rel_dt_keep =
                                helpers::calc_rel_dt(avg_dt_keep, &abs_dt_keep).unwrap();
                            let mut rel_dt_move = [0i16; tuning::DATA_GROUP_PERIODIC_SIZE - 1];
                            rel_dt_move[0..abs_dt_move.len()].copy_from_slice(
                                &helpers::calc_rel_dt(avg_dt_move, &abs_dt_move).unwrap(),
                            );
                            // store the new data [keep]
                            index.used = data_keep.len() as u64;
                            // set the after time equal to the absolute time of the first entry
                            // =comentary= this caused a seriously annoying bug, no clue why i thought this was necessary
                            // index.after += abs_dt_keep[0] as i64;
                            data.avg_dt = avg_dt_keep;
                            data.data[0..data_keep.len()].copy_from_slice(&data_keep);
                            data.dt[0..rel_dt_keep.len()].copy_from_slice(&rel_dt_keep);
                            // store the new data [move]
                            let new_entry = repr::DataGroupIndex {
                                after: index.after + abs_dt_move[0] as i64,
                                used: abs_dt_move.len() as u64,
                                next: index.pointer(),
                                group: {
                                    let mut new_data = repr::DataGroupPeriodic {
                                        // only one entry
                                        avg_dt: avg_dt_move,
                                        _pad: 0,
                                        dt: rel_dt_move,
                                        data: [0.0; tuning::DATA_GROUP_PERIODIC_SIZE - 1],
                                    };
                                    new_data.data[0..data_move.len()].copy_from_slice(&data_move);
                                    debug!("(Periodic, branch {}) - t={time} after(keep)={}, after(move)={} entry_dt={entry_dt} reading={reading} final (keep)={:?} final (move)={:?}", line!(), index.after, index.after + abs_dt_move[0] as i64, &*data, new_data);
                                    let pointer = Object::new_alloc(&mut self.alloc, new_data)
                                        .await?
                                        .dispose_sync(&mut self.alloc)
                                        .await?;
                                    repr::DataGroup { periodic: pointer }
                                },
                            };
                            data.dispose_sync(&mut self.alloc).await?;
                            let new_entry = Object::new_alloc(&mut self.alloc, new_entry).await?;
                            let new_entry_ptr = new_entry.dispose_sync(&mut self.alloc).await?;
                            match &mut prev {
                                Some(prev) => {
                                    prev.next = new_entry_ptr;
                                    prev.sync(&mut self.alloc).await?;
                                }
                                None => {
                                    channel.data = new_entry_ptr;
                                    channel_idx
                                        .write::<{ tuning::CHANNEL_MAP_CHUNK_SIZE }, _, _>(
                                            &mut self.alloc,
                                            channel,
                                        )
                                        .await?;
                                }
                            }
                        }
                        repr::DataGroupType::Sporadic => {
                            // this is validated by the match statement, but just in case.
                            // - means that ALL entries in `data.dt` and `data.data` are currently in use.
                            // - this means we DO NOT have to do `data.field[0..index.used]` like in other cases
                            todo!();
                            debug_assert_eq!(index.used, gtype_size);
                            let mut data =
                                Object::new_read(&mut self.alloc, unsafe { index.group.sporadic })
                                    .await?;
                            let entry_dt =
                                u32::try_from(time - index.after).expect("delta-time out of range");
                            // just make sure that binary_search wont produce garbage
                            debug_assert!(data.dt.is_sorted());
                            let ins_idx =
                                data.dt.binary_search(&entry_dt).map_or_else(|x| x, |x| x);
                            let src = ins_idx..index.used as usize;
                            let dest = ins_idx + 1;
                            data.dt.copy_within(
                                src.clone(), /* why does Range not impl Copy */
                                dest,
                            );
                            data.data.copy_within(src, dest);
                            data.dt[ins_idx] = entry_dt;
                            data.data[ins_idx] = reading;
                            // increment `used`
                            index.used += 1;
                            // make sure that we didnt screw up the ordering
                            debug_assert!(data.dt.is_sorted());
                            data.dispose_sync(&mut self.alloc).await?;
                        }
                    }
                    break;
                }
                // need to split this in (normally 2, never (?) 3) parts
                // likely (if there is a gap in readings, this will happen)
                (
                    // time within this or [one that contained data from after `time`]
                    true,
                    // don't care about space (will split either way)
                    _,
                    // don't care about what came before, it is irrelevant here.
                    _,
                    // must be able to insert data successfully
                    false,
                ) => {
                    todo!()
                }
                // data goes in an entry farther down, which could exist already (unlikely)
                (
                    // the new data is older than this
                    false,
                    // so it doesn't matter if the data fits here
                    _,
                    // because we have something to move on to
                    true,
                    // don't care if it can be inserted here, because we aren't doing that
                    _,
                ) => {
                    let next = index.next;
                    if let Some(prev) = prev {
                        prev.dispose_immutated();
                    }
                    prev = Some(index);
                    index = Object::new_read(&mut self.alloc, next).await?;
                }
                // data can still go in this entry, we just need to change the `before` time backwards
                (
                    // the new data is older than this
                    false,
                    // but there is space [to move the start time of this chunk to match]
                    true,
                    // which is important because there is no chunk after this
                    false,
                    // we must be able to insert data.
                    true,
                ) if {
                    // this will work with the delta-time range of index
                    todo!()
                } =>
                {
                    todo!()
                }
                // data goes in an entry farther down, which does not exist (the data is before any other (very unlikely))
                (
                    // the new data is older than this
                    false,
                    // false = and there is no space [to move the start time back and insert]
                    // true = and moving the start back would have broken the dt system [see above condition]
                    _,
                    // and there are no entries after this [so we need to make a new one]
                    false,
                    // we aren't inserting data, dont care
                    _,
                ) => {
                    todo!()
                }
            }

            // let r = ..chunk.used as usize;
            //     // the first time this is true should be the most recent chunk that works with this data.
            //     if entry.after < time {
            //         // ^ the new data belongs in this chunk
            //         // (`time` is after the start time of data in `entry`)
            //         if entry.used < gtype_size {
            //             // ^ the new data fits in this chunk
            //             match gtype {
            //                 repr::DataGroupType::Periodic => {
            //                     let mut data = Object::new_read(&mut self.alloc, unsafe {
            //                         entry.group.periodic
            //                     })
            //                     .await?;
            //                     let entry_dt = u64::try_from(time - entry.after)
            //                         .expect("unreachable: delta-time negative");
            //                     // FIXME: slo cod
            //
            //                     // instead of calculating the change to avg_dt and then the entry's
            //                     // relative dt, which is hard, we simply recalculate it for all
            //                     // entries, which is easy, but slower
            //                     //
            //                     // here we reverse the relative delta calculation, arriving at a
            //                     // individual offset from `entry.after` for each entry in `data`
            //                     let mut abs_dt = data
            //                         .dt
            //                         .iter()
            //                         .enumerate()
            //                         .map(|(i, dt)| {
            //                             (i as u64 * data.avg_dt as u64)
            //                                 .checked_add_signed(*dt as i64)
            //                                 .unwrap()
            //                         })
            //                         .collect::<Vec<u64>>();
            //
            //                     // make sure that the delta times are in order (to make sure the
            //                     // rel -> abs isnt completely wrong, and to make sure the
            //                     // `binary_search` used next works right)
            //                     debug_assert!(abs_dt.is_sorted());
            //
            //                     // insert the new entry
            //                     let ins_idx =
            //                         abs_dt.binary_search(&entry_dt).map_or_else(|x| x, |x| x);
            //                     let src = ins_idx..entry.used as usize;
            //                     let dest = ins_idx + 1;
            //                     abs_dt.copy_within(
            //                         src.clone(), /* why does Range not impl Copy */
            //                         dest,
            //                     );
            //                     data.data.copy_within(src, dest);
            //                     abs_dt[ins_idx] = entry_dt;
            //                     data.data[ins_idx] = reading;
            //                     // increment `used`
            //                     entry.used += 1;
            //                     // make sure that we didnt screw up the ordering
            //                     debug_assert!(data.dt.is_sorted());
            //
            //                     // then calculate the average dt
            //                     let avg_dt = u32::try_from(
            //                         iter::once(0u64)
            //                             .chain(abs_dt.iter().copied())
            //                             .zip(abs_dt.iter().copied())
            //                             .map(|(last, next)| (next - last) as u128)
            //                             .sum::<u128>()
            //                             / abs_dt.len() as u128,
            //                     )
            //                     .expect("average delta-time too large");
            //
            //                     // then calculate individual offsets (delta from average)
            //                     let rel_dt = abs_dt
            //                         .iter()
            //                         .enumerate()
            //                         .map(|(i, abs_dt)| {
            //                             i16::try_from(
            //                                 *abs_dt as i64 - (avg_dt as u64 * i as u64) as i64,
            //                             )
            //                             .expect("relative delta-time too large")
            //                         })
            //                         .collect::<Vec<i16>>();
            //
            //                     data.avg_dt = avg_dt;
            //                     data.dt = rel_dt.try_into().unwrap();
            //                     data.dispose_sync(&mut self.alloc).await?;
            //                 }
            //                 repr::DataGroupType::Sporadic => {
            //                     let mut data = Object::new_read(&mut self.alloc, unsafe {
            //                         entry.group.sporadic
            //                     })
            //                     .await?;
            //                     let entry_dt = u32::try_from(time - entry.after)
            //                         .expect("delta-time out of range");
            //                     // just make sure that binary_search wont produce garbage
            //                     debug_assert!(data.dt.is_sorted());
            //                     let ins_idx =
            //                         data.dt.binary_search(&entry_dt).map_or_else(|x| x, |x| x);
            //                     let src = ins_idx..entry.used as usize;
            //                     let dest = ins_idx + 1;
            //                     data.dt.copy_within(
            //                         src.clone(), /* why does Range not impl Copy */
            //                         dest,
            //                     );
            //                     data.data.copy_within(src, dest);
            //                     data.dt[ins_idx] = entry_dt;
            //                     data.data[ins_idx] = reading;
            //                     // increment `used`
            //                     entry.used += 1;
            //                     // make sure that we didnt screw up the ordering
            //                     debug_assert!(data.dt.is_sorted());
            //                     data.dispose_sync(&mut self.alloc).await?;
            //                 }
            //             }
            //         } else {
            //             // ^ the new data does not fit in this index, a new one must be created
            //             // after this one, and the data in the old one split around this entry,
            //             // the more recent part moved to the new chunk
            //
            //             if chunk.used < tuning::DATA_INDEX_CHUNK_SIZE as u64 {
            //                 todo!("the data within each final chunk needs to be split at the appropreate time");
            //                 // ^ there is enough space in this chunk for the new index to go
            //                 let used = chunk.used;
            //                 // move this entry, and all the ones (after in list, before in time) it back
            //                 // by 1 to make room for the new entry
            //                 chunk
            //                     .data
            //                     .copy_within(entry_idx..used as usize, entry_idx + 1);
            //                 chunk.used += 1;
            //                 // insert the new entry
            //                 // FIXME: the `after` field is set to the reading time, meaning there is a gap
            //                 // between the end of the last entry, and the start of this one. if something
            //                 // is inserted in that gap, then a new index will be unnecessarily created.
            //                 // this could be fixed in the insertion, or in the vacuum impl. (in insertion
            //                 // would be better tho)
            //                 let new_entry = repr::DataGroupIndex {
            //                     after: time,
            //                     used: 1,
            //                     group: match gtype {
            //                         repr::DataGroupType::Periodic => {
            //                             let mut data = repr::DataGroupPeriodic {
            //                                 // only one entry
            //                                 avg_dt: 0,
            //                                 _pad: 0,
            //                                 dt: [0; tuning::DATA_GROUP_PERIODIC_SIZE - 1],
            //                                 data: [0.0; tuning::DATA_GROUP_PERIODIC_SIZE - 1],
            //                             };
            //                             data.data[0] = reading;
            //                             let pointer = Object::new_alloc(&mut self.alloc, data)
            //                                 .await?
            //                                 .dispose_sync(&mut self.alloc)
            //                                 .await?;
            //                             repr::DataGroup { periodic: pointer }
            //                         }
            //                         repr::DataGroupType::Sporadic => {
            //                             let mut data = repr::DataGroupSporadic {
            //                                 dt: [0; tuning::DATA_GROUP_SPORADIC_SIZE],
            //                                 data: [0.0; tuning::DATA_GROUP_SPORADIC_SIZE],
            //                             };
            //                             data.data[0] = reading;
            //                             let pointer = Object::new_alloc(&mut self.alloc, data)
            //                                 .await?
            //                                 .dispose_sync(&mut self.alloc)
            //                                 .await?;
            //                             repr::DataGroup { sporadic: pointer }
            //                         }
            //                     },
            //                 };
            //                 chunk.data[entry_idx] = new_entry;
            //             } else {
            //                 // ^ we need to create a new entry in the index list
            //                 todo!("unfinished + the data must be split at the appropreate time (see above)");
            //
            //                 // the new entry is farther back in the list, and so will contain older entries
            //                 let mut new_index = Object::new_alloc(
            //                     &mut self.alloc,
            //                     ChunkedLinkedList {
            //                         next: chunk.next,
            //                         used: 0,
            //                         data: <_ as FromBytes>::new_zeroed(),
            //                     },
            //                 )
            //                 .await?;
            //                 chunk.next = new_index.pointer();
            //                 // move all entries older than the new one into the new index
            //                 new_index
            //                     .data
            //                     .get_mut(..(chunk.data[entry_idx..].len()))
            //                     .unwrap()
            //                     .copy_from_slice(&chunk.data[entry_idx..]);
            //                 // insert the new entry into the current chunk
            //             }
            //         }
            //         // the data is inserted
            //         break 'find_chunk;
            //     } else {
            //         // ^ we need to go back to find the right place.
            //         continue 'find_entry;
            //     }
            // if !chunk.next.is_null() {
            //     // go to the next newest chunk
            //     let next = Object::new_read(&mut self.alloc, chunk.next).await?;
            //     chunk.dispose_sync(&mut self.alloc).await?;
            //     chunk = next;
            // } else {
            //     // there is no index old enough ????
            //     if chunk.used < tuning::DATA_INDEX_CHUNK_SIZE as u64 {
            //         // ^ there is enough space in this chunk for the new index to go
            //         todo!()
            //     } else {
            //         // ^ we need to create a new entry in the index list
            //         todo!()
            //     }
            // }
        }
        if let Some(prev) = prev {
            prev.dispose_immutated();
        }
        index.dispose_sync(&mut self.alloc).await?;

        Ok(())
    }

    #[instrument]
    pub async fn infodump() {
        use repr::info::print_inf;
        use repr::*;
        print_inf::<DBEntrypoint>();
    }

    #[instrument(skip(self))]
    pub async fn infodump_from(&mut self) -> Result<(), DBError<<Store as Storage>::Error>> {
        self.alloc.infodump_from().await?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn close(self) -> Result<(), DBError<<Store as Storage>::Error>> {
        self.alloc.close().await?;
        Ok(())
    }
}
