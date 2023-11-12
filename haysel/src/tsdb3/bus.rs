//! bus integration for TSBD2

use chrono::{DateTime, Utc};
use flume::Sender;
use mycelium::station::{
    capabilities::{Channel, KnownChannels},
    identity::KnownStations,
};
use roundtable::{handler::LocalInterface, method_decl};
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::dispatch::application::Record;

use super::{query::QueryParams, DB};

mod rt;

/// The handler
pub struct TStopDBus3 {
    comm: Sender<rt::Msg>,
}

impl TStopDBus3 {
    pub fn new(db: DB) -> Self {
        let comm = rt::launch(db);
        Self { comm }
    }

    async fn query(
        &mut self,
        &params: &QueryParams,
        _int: &LocalInterface,
    ) -> Vec<(DateTime<Utc>, f32)> {
        let (response, recv) = oneshot::channel();
        self.comm
            .send_async(rt::Msg::Query { params, response })
            .await
            .expect("Runtime task closed");
        recv.await
            .expect("Runtime task closed or recv queue dropped")
    }

    pub async fn ensure_exists(&mut self, (stations, channels): &(KnownStations, KnownChannels)) {
        self.comm
            .send_async(rt::Msg::EnsureExists {
                stations: stations.clone(),
                channels: channels.clone(),
            })
            .await
            .expect("Runtime task closed");
    }

    async fn new_station(&mut self, &sid: &Uuid, _int: &LocalInterface) {
        self.comm
            .send_async(rt::Msg::NewStation { sid })
            .await
            .expect("Runtime task closed");
    }

    async fn station_new_channel(
        &mut self,
        (sid, cid, inf): &(Uuid, Uuid, Channel),
        _int: &LocalInterface,
    ) {
        self.comm
            .send_async(rt::Msg::NewChannel {
                sid: *sid,
                cid: *cid,
                inf: inf.clone(),
            })
            .await
            .expect("Runtime task closed");
    }

    async fn record_data(&mut self, record: &Record, _int: &LocalInterface) {
        self.comm
            .send_async(rt::Msg::Record {
                record: record.clone(),
            })
            .await
            .expect("Runtime task closed");
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
