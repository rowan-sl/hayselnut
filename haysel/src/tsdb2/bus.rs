//! bus integration for TSBD2

use anyhow::Result;
use chrono::{DateTime, Utc};
use mycelium::station::{
    capabilities::{Channel, ChannelData, ChannelType, KnownChannels},
    identity::KnownStations,
};
use uuid::Uuid;

use crate::{
    bus::{
        common::EV_SHUTDOWN,
        handler::{handler_decl_t, method_decl, HandlerInit, LocalInterface},
        msg::{HandlerType, Str},
    },
    dispatch::application::{Record, EV_WEATHER_DATA_RECEIVED},
    misc::Take,
    registry::{EV_META_NEW_STATION, EV_META_STATION_ASSOC_CHANNEL},
};

use super::{alloc::Storage, query::builder::QueryParamsNoDB, repr::DataGroupType, Database};

/// The handler
pub struct TStopDBus2<S: Storage> {
    db: Take<Database<S>>,
}

impl<S: Storage> TStopDBus2<S> {
    pub async fn new(db: Database<S>) -> Self {
        Self { db: Take::new(db) }
    }

    async fn close(&mut self, _args: &(), _: &LocalInterface) {
        if let Err(e) = self.db.take().close().await {
            error!("Failed to close db: {e:?}")
        }
    }

    async fn query(
        &mut self,
        args: &QueryParamsNoDB,
        _int: &LocalInterface,
    ) -> Result<Vec<(DateTime<Utc>, f32)>> {
        Ok(args.clone().with_db(&mut self.db).execute().await?)
    }

    pub async fn ensure_exists(
        &mut self,
        (_stations, _channels): &(KnownStations, KnownChannels),
    ) -> Result<()> {
        warn!("Initial state verification unimplemented (necessary stations/channels may not exist in the database)");
        Ok(())
    }

    async fn new_station(&mut self, &id: &Uuid, _int: &LocalInterface) {
        if let Err(err) = self.db.add_station(id).await {
            error!("Error occured adding station to DB: {err:?}");
            warn!("Handling of this error is not implemented");
        }
    }

    async fn station_new_channel(
        &mut self,
        (station, channel, channel_info): &(Uuid, Uuid, Channel),
        _int: &LocalInterface,
    ) {
        if let Err(err) = self
            .db
            .add_channel(
                *station,
                *channel,
                match channel_info.ty {
                    ChannelType::Periodic => DataGroupType::Periodic,
                    ChannelType::Triggered => DataGroupType::Sporadic,
                },
            )
            .await
        {
            error!("Error occured adding channel to station in DB: {err:?}");
            warn!("Handling of this error is not implemented");
        }
    }

    async fn record_data(&mut self, data: &Record, _int: &LocalInterface) {
        for (ch, val) in &data.data {
            self.db
                .add_data(
                    data.recorded_by,
                    *ch,
                    data.recorded_at,
                    match val {
                        ChannelData::Float(val) => *val,
                        ChannelData::Event { .. } => {
                            error!("Database does not support recording `event` type events yet");
                            continue;
                        }
                    },
                )
                .await
                .expect("Failed to insert data into database");
        }
    }
}

impl<S: Storage + Sync> HandlerInit for TStopDBus2<S> {
    const DECL: HandlerType = handler_decl_t!("TSDB2 Bus Integration");
    fn describe(&self) -> Str {
        Str::Borrowed("Instance of TSDB2 Bus Integration")
    }
    fn methods(&self, r: &mut crate::bus::handler::MethodRegister<Self>) {
        r.register(Self::close, EV_SHUTDOWN);
        r.register(Self::query, EV_DB_QUERY);
        r.register(Self::new_station, EV_META_NEW_STATION);
        r.register(Self::station_new_channel, EV_META_STATION_ASSOC_CHANNEL);
        r.register(Self::record_data, EV_WEATHER_DATA_RECEIVED);
    }
}

method_decl!(
    EV_DB_QUERY,
    QueryParamsNoDB,
    Result<Vec<(DateTime<Utc>, f32)>>
);
