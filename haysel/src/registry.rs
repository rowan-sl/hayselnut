pub mod loader;

use std::{collections::HashMap, net::SocketAddr};

pub use loader::JsonLoader;
use mycelium::station::{
    capabilities::{Channel, ChannelID, ChannelName, KnownChannels},
    identity::{KnownStations, StationID, StationInfo},
};
use roundtable::{
    common::EV_BUILTIN_AUTOSAVE,
    handler::{HandlerInit, LocalInterface, MethodRegister},
    handler_decl_t, method_decl,
    msg::{self, Str},
};
use squirrel::api::OnConnect;

use crate::misc::Take;

pub struct Registry {
    stations: Take<JsonLoader<KnownStations>>,
    channels: Take<JsonLoader<KnownChannels>>,
}

method_decl!(EV_REGISTRY_QUERY_ALL, (), (KnownStations, KnownChannels));
method_decl!(EV_REGISTRY_QUERY_CHANNEL, ChannelID, Option<Channel>);
method_decl!(
    EV_REGISTRY_PROCESS_CONNECT,
    (SocketAddr, OnConnect),
    HashMap<ChannelName, ChannelID>
);
method_decl!(EV_META_NEW_STATION, StationID, ());
method_decl!(EV_META_NEW_CHANNEL, (ChannelID, Channel), ());
method_decl!(
    EV_META_STATION_ASSOC_CHANNEL,
    (StationID, ChannelID, Channel),
    ()
);

#[async_trait]
impl HandlerInit for Registry {
    const DECL: msg::HandlerType = handler_decl_t!("Registry interface");
    async fn init(&mut self, _int: &LocalInterface) {}
    fn describe(&self) -> Str {
        Str::Borrowed("Registry interface")
    }
    fn methods(&self, reg: &mut MethodRegister<Self>) {
        reg.register(Self::query_all, EV_REGISTRY_QUERY_ALL);
        reg.register(Self::process_connect, EV_REGISTRY_PROCESS_CONNECT);
        reg.register(Self::sync, EV_BUILTIN_AUTOSAVE);
    }
}

impl Registry {
    pub fn new(stations: JsonLoader<KnownStations>, channels: JsonLoader<KnownChannels>) -> Self {
        Self {
            stations: Take::new(stations),
            channels: Take::new(channels),
        }
    }

    #[instrument(skip(self, _int))]
    async fn sync(&mut self, _: &(), _int: &LocalInterface) {
        self.stations.sync().await.expect("Failed to sync stations");
        self.channels.sync().await.expect("Failed to sync channels");
    }

    async fn query_all(&mut self, _: &(), _int: &LocalInterface) -> (KnownStations, KnownChannels) {
        (self.stations.clone(), self.channels.clone())
    }

    async fn process_connect(
        &mut self,
        (ip, data): &(SocketAddr, OnConnect),
        int: &LocalInterface,
    ) -> HashMap<ChannelName, ChannelID> {
        let (ip, data) = (ip.clone(), data.clone());
        let name_to_id_mappings = data
            .channels
            .iter()
            .map(|ch| {
                (
                    ch.name.clone(),
                    self.channels
                        .id_by_name(&ch.name)
                        .map(|id| (id, false))
                        .unwrap_or_else(|| {
                            info!("creating new channel: {ch:?}");
                            (self.channels.insert_channel(ch.clone()).unwrap(), true)
                        }),
                )
            })
            .collect::<HashMap<ChannelName, (ChannelID, bool)>>();
        for (ch_id, _) in name_to_id_mappings.values().filter(|(_, is_new)| *is_new) {
            let ch = self.channels.get_channel(&ch_id).unwrap();
            int.announce(msg::Target::Any, EV_META_NEW_CHANNEL, (*ch_id, ch.clone()))
                .await
                .unwrap();
        }
        let name_to_id_mappings = name_to_id_mappings
            .into_iter()
            .map(|(k, (v, _))| (k, v))
            .collect::<HashMap<ChannelName, ChannelID>>();
        if let Some(pre_info) = self.stations.get_info(&data.station_id) {
            info!(
                "connecting to known station [{}] at IP {:?}\n    hayselnut rev {}\n    built on {}",
                data.station_id,
                ip,
                data.station_build_rev,
                data.station_build_date
            );
            for new_channel in name_to_id_mappings
                .values()
                .filter(|&id| !pre_info.supports_channels.contains(id))
            {
                let ch = self.channels.get_channel(new_channel).unwrap();
                int.announce(
                    msg::Target::Any,
                    EV_META_STATION_ASSOC_CHANNEL,
                    (data.station_id, *new_channel, ch.clone()),
                )
                .await
                .unwrap();
            }
            self.stations.map_info(&data.station_id, |_id, info| {
                info.supports_channels = name_to_id_mappings.values().copied().collect()
            });
        } else {
            info!(
                "connected to new station [{}] at IP {:?}\n    hayselnut rev {}\n    built on {}",
                data.station_id, ip, data.station_build_rev, data.station_build_date
            );
            self.stations
                .insert_station(
                    data.station_id,
                    StationInfo {
                        supports_channels: name_to_id_mappings.values().copied().collect(),
                    },
                )
                .unwrap();
            int.announce(msg::Target::Any, EV_META_NEW_STATION, data.station_id)
                .await
                .unwrap();
            for new_channel in name_to_id_mappings.values() {
                let ch = self.channels.get_channel(new_channel).unwrap();
                int.announce(
                    msg::Target::Any,
                    EV_META_STATION_ASSOC_CHANNEL,
                    (data.station_id, *new_channel, ch.clone()),
                )
                .await
                .unwrap();
            }
        }
        name_to_id_mappings
    }
}
