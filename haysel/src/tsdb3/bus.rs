//! bus integration for TSBD2

use chrono::{DateTime, Utc};
use mycelium::station::{
    capabilities::{Channel, ChannelData, KnownChannels},
    identity::KnownStations,
};
use roundtable::{handler::LocalInterface, method_decl};
use uuid::Uuid;

use crate::dispatch::application::Record;

use super::{query::QueryParams, DB};

/// The handler
pub struct TStopDBus3 {
    db: DB,
}

impl TStopDBus3 {
    pub async fn new(db: DB) -> Self {
        Self { db }
    }

    async fn query(
        &mut self,
        args: &QueryParams,
        _int: &LocalInterface,
    ) -> Vec<(DateTime<Utc>, f32)> {
        todo!()
        // Ok(self.db.qery_data(, , , , ))
    }

    pub async fn ensure_exists(&mut self, (_stations, _channels): &(KnownStations, KnownChannels)) {
        todo!()
        // warn!("Initial state verification unimplemented (necessary stations/channels may not exist in the database)");
        // Ok(())
    }

    async fn new_station(&mut self, &id: &Uuid, _int: &LocalInterface) {
        todo!()
        // if let Err(err) = self.db.add_station(id).await {
        //     error!("Error occured adding station to DB: {err:?}");
        //     warn!("Handling of this error is not implemented");
        // }
    }

    async fn station_new_channel(
        &mut self,
        (station, channel, channel_info): &(Uuid, Uuid, Channel),
        _int: &LocalInterface,
    ) {
        todo!()
        // if let Err(err) = self
        //     .db
        //     .add_channel(
        //         *station,
        //         *channel,
        //         match channel_info.ty {
        //             ChannelType::Periodic => DataGroupType::Periodic,
        //             ChannelType::Triggered => DataGroupType::Sporadic,
        //         },
        //     )
        //     .await
        // {
        //     error!("Error occured adding channel to station in DB: {err:?}");
        //     warn!("Handling of this error is not implemented");
        // }
    }

    async fn record_data(&mut self, data: &Record, _int: &LocalInterface) {
        for (ch, val) in &data.data {
            self.db.insert_data(
                data.recorded_by,
                *ch,
                data.recorded_at,
                match val {
                    ChannelData::Float(val) => *val,
                    ChannelData::Event { .. } => {
                        error!("Database does not support recording `event` type data yet");
                        continue;
                    }
                },
            );
        }
    }
}

// impl<S: Storage + Sync> HandlerInit for TStopDBus2<S> {
//     const DECL: HandlerType = handler_decl_t!("TSDB2 Bus Integration");
//     fn describe(&self) -> Str {
//         Str::Borrowed("Instance of TSDB2 Bus Integration")
//     }
//     fn methods(&self, r: &mut roundtable::handler::MethodRegister<Self>) {
//         r.register(Self::query, EV_DB_QUERY);
//         r.register(Self::new_station, EV_META_NEW_STATION);
//         r.register(Self::station_new_channel, EV_META_STATION_ASSOC_CHANNEL);
//         r.register(Self::record_data, EV_WEATHER_DATA_RECEIVED);
//         r.register(Self::sync, EV_BUILTIN_AUTOSAVE);
//     }
// }

method_decl!(EV_DB_QUERY, QueryParams, Vec<(DateTime<Utc>, f32)>);
