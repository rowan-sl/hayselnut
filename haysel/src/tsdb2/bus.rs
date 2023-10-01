//! bus integration for TSBD2

use anyhow::Result;
use chrono::{DateTime, Utc};
use mycelium::station::{
    capabilities::{Channel, ChannelType, KnownChannels},
    identity::KnownStations,
};
use uuid::Uuid;

use crate::{
    bus::{
        common::EV_SHUTDOWN,
        handler::{handler_decl_t, method_decl, HandlerInit, Interface},
        msg::{HandlerType, Str},
    },
    util::Take,
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

    async fn close(&mut self, _args: &(), _int: Interface) {
        if let Err(e) = self.db.take().close().await {
            error!("Failed to close db: {e:?}")
        }
    }

    async fn query(
        &mut self,
        args: &QueryParamsNoDB,
        _int: Interface,
    ) -> Result<Vec<(DateTime<Utc>, f32)>> {
        Ok(args.clone().with_db(&mut self.db).execute().await?)
    }

    async fn ensure_exists(
        &mut self,
        (_stations, _channels): &(KnownStations, KnownChannels),
        _int: Interface,
    ) -> Result<()> {
        warn!("Initial state verification unimplemented (necessary stations/channels may not exist in the database)");
        Ok(())
    }

    async fn new_station(&mut self, &id: &Uuid, _int: Interface) -> Result<()> {
        self.db.add_station(id).await?;
        Ok(())
    }

    async fn station_new_channel(
        &mut self,
        (station, channel, channel_info): &(Uuid, Uuid, Channel),
        _int: Interface,
    ) -> Result<()> {
        self.db
            .add_channel(
                *station,
                *channel,
                match channel_info.ty {
                    ChannelType::Periodic => DataGroupType::Periodic,
                    ChannelType::Triggered => DataGroupType::Sporadic,
                },
            )
            .await?;
        Ok(())
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
        r.register(Self::ensure_exists, EV_DB_META_ENSURE_EXISTS);
        r.register(Self::new_station, EV_DB_META_NEW_STATION);
        r.register(Self::station_new_channel, EV_DB_META_STATION_ASSOC_CHANNEL);
    }
}

method_decl!(
    EV_DB_QUERY,
    QueryParamsNoDB,
    Result<Vec<(DateTime<Utc>, f32)>>
);
method_decl!(
    EV_DB_META_ENSURE_EXISTS,
    (KnownStations, KnownChannels),
    Result<()>
);
method_decl!(EV_DB_META_NEW_STATION, Uuid, Result<()>);
method_decl!(
    EV_DB_META_STATION_ASSOC_CHANNEL,
    (Uuid, Uuid, Channel),
    Result<()>
);
