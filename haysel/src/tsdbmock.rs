//! bus integration for TSBD2

use std::collections::{HashMap, VecDeque};

use anyhow::Result;
use chrono::{DateTime, Utc};
use mycelium::station::{
    capabilities::{Channel, ChannelData, ChannelID, KnownChannels},
    identity::{KnownStations, StationID},
};
use roundtable::{
    handler::{HandlerInit, LocalInterface},
    handler_decl_t,
    msg::{HandlerType, Str},
};
use uuid::Uuid;

use crate::{
    dispatch::application::{Record, EV_WEATHER_DATA_RECEIVED},
    registry::{EV_META_NEW_STATION, EV_META_STATION_ASSOC_CHANNEL},
    tsdb2::query::builder::QueryParamsNoDB,
};

/// The handler
pub struct TStopDBus2Mock {
    map: HashMap<StationID, HashMap<ChannelID, VecDeque<(DateTime<Utc>, f32)>>>,
}

impl TStopDBus2Mock {
    pub async fn new() -> Self {
        Self {
            map: Default::default(),
        }
    }

    async fn query(
        &mut self,
        args: &QueryParamsNoDB,
        _int: &LocalInterface,
    ) -> Result<Vec<(DateTime<Utc>, f32)>> {
        debug!("DB Mock query");
        let (station, channel, max_results, after_time, before_time) =
            args.clone().to_raw_for_mock_only();
        let data = self
            .map
            .get(&station)
            .ok_or(anyhow!("Query failed: no such station"))?
            .get(&channel)
            .ok_or(anyhow!("Query failed: no such channel"))?;
        let response = data
            .iter()
            .filter(|(time, _)| after_time.as_ref().map(|af_t| time > af_t).unwrap_or(true))
            .filter(|(time, _)| before_time.as_ref().map(|bf_t| time < bf_t).unwrap_or(true))
            .take(max_results.unwrap_or(usize::MAX))
            .cloned()
            .collect::<Vec<_>>();
        debug!("DB query finished");
        Ok(response)
    }

    pub async fn ensure_exists(
        &mut self,
        (stations, channels): &(KnownStations, KnownChannels),
    ) -> Result<()> {
        for &id in stations.stations() {
            self.map.insert(id, {
                let mut inner = HashMap::new();
                for (&id, _) in channels.channels() {
                    inner.insert(id, VecDeque::new());
                }
                inner
            });
        }
        Ok(())
    }

    async fn new_station(&mut self, &id: &Uuid, _int: &LocalInterface) {
        self.map.insert(id, HashMap::new());
    }

    async fn station_new_channel(
        &mut self,
        (station, channel, _channel_info): &(Uuid, Uuid, Channel),
        _int: &LocalInterface,
    ) {
        self.map
            .get_mut(station)
            .expect("StationNewChannel: no such station")
            .insert(*channel, VecDeque::new());
    }

    async fn record_data(&mut self, data: &Record, _int: &LocalInterface) {
        for (channel, reading) in &data.data {
            let ChannelData::Float(reading) = reading else {
                todo!("Event type data is not supported")
            };
            let records = self
                .map
                .get_mut(&data.recorded_by)
                .expect("Insert failed: no such station")
                .get_mut(channel)
                .expect("Insert failed: no such channel");
            records.push_front((data.recorded_at, *reading));
            if records.len() > 256 {
                let _ = records.pop_back();
            }
        }
    }
}

impl HandlerInit for TStopDBus2Mock {
    const DECL: HandlerType = handler_decl_t!("TSDB2 Mock Bus Integration");
    fn describe(&self) -> Str {
        Str::Borrowed("TSDB2 Mock")
    }
    fn methods(&self, r: &mut roundtable::handler::MethodRegister<Self>) {
        r.register(Self::query, crate::tsdb2::bus::EV_DB_QUERY);
        r.register(Self::new_station, EV_META_NEW_STATION);
        r.register(Self::station_new_channel, EV_META_STATION_ASSOC_CHANNEL);
        r.register(Self::record_data, EV_WEATHER_DATA_RECEIVED);
    }
}
