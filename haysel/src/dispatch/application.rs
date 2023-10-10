//! application-layer packet handling

use std::{collections::HashMap, fmt::Write, net::SocketAddr};

use chrono::{DateTime, Utc};
use mycelium::station::{
    capabilities::{ChannelData, ChannelID, ChannelName},
    identity::StationID,
};
use squirrel::api::{ChannelMappings, OnConnect, PacketKind, SomeData};

use crate::{
    bus::{
        handler::{handler_decl_t, method_decl, HandlerInit, LocalInterface, MethodRegister},
        msg::{self, HandlerInstance, Str},
    },
    registry,
};

use super::{EV_TRANS_CLI_DATA_RECVD, EV_TRANS_CLI_QUEUE_DATA};

pub struct AppClient {
    // controller instance
    #[allow(unused)]
    ctrl: HandlerInstance,
    // associated transport client (used for sending packets to).
    transport: HandlerInstance,
    registry: HandlerInstance,
    addr: SocketAddr,
    meta_station_id: Option<StationID>,
    meta_station_build_rev: Option<String>,
    // chrono rfc3339 timestamp
    meta_station_build_date: Option<String>,
}

method_decl!(EV_WEATHER_DATA_RECEIVED, Record, ());

#[derive(Debug, Clone)]
pub struct Record {
    pub recorded_at: DateTime<Utc>,
    pub recorded_by: StationID,
    pub data: HashMap<ChannelID, ChannelData>,
}

#[async_trait]
impl HandlerInit for AppClient {
    const DECL: crate::bus::msg::HandlerType = handler_decl_t!("Weather station interface");
    async fn init(&mut self, _int: &LocalInterface) {}
    fn describe(&self) -> Str {
        Str::Owned(format!(
            "Weather station interface [application] for {:?}",
            self.addr
        ))
    }
    fn methods(&self, reg: &mut MethodRegister<Self>) {
        reg.register(Self::received, EV_TRANS_CLI_DATA_RECVD);
    }
}

impl AppClient {
    pub fn new(
        addr: SocketAddr,
        controller: HandlerInstance,
        transport: HandlerInstance,
        registry: HandlerInstance,
    ) -> Self {
        Self {
            ctrl: controller,
            transport,
            registry,
            addr,
            meta_station_id: None,
            meta_station_build_rev: None,
            meta_station_build_date: None,
        }
    }

    async fn received(&mut self, data: &Vec<u8>, int: &LocalInterface) {
        match rmp_serde::from_slice::<PacketKind>(&*data) {
            Ok(pkt) => {
                trace!("Received packet from IP: {:?} - {pkt:?}", self.addr);
                match pkt {
                    PacketKind::Connect(data) => self.on_connect(data, int).await,
                    PacketKind::Data(data) => self.on_data(data, int).await,
                    _ => warn!("Received unexpected packet kind"),
                }
            }
            Err(e) => {
                warn!(
                    "Failed to deserialize packet from IP: {:?} - {e:#}",
                    self.addr
                );
            }
        }
    }

    async fn on_connect(&mut self, data: OnConnect, int: &LocalInterface) {
        let name_to_id_mappings = int
            .dispatch(
                msg::Target::Instance(self.registry.clone()),
                registry::EV_REGISTRY_PROCESS_CONNECT,
                (self.addr, data.clone()),
            )
            .await
            .unwrap()
            .expect("Failed to query registry - no reply");
        let resp = rmp_serde::to_vec_named(&PacketKind::ChannelMappings(ChannelMappings {
            map: name_to_id_mappings,
        }))
        .unwrap();
        int.dispatch(
            msg::Target::Instance(self.transport.clone()),
            EV_TRANS_CLI_QUEUE_DATA,
            resp,
        )
        .await
        .unwrap()
        .expect("Failed to send packet back to station");
        self.meta_station_id = Some(data.station_id);
        self.meta_station_build_rev = Some(data.station_build_rev);
        self.meta_station_build_date = Some(data.station_build_date);
    }

    async fn on_data(&mut self, data: SomeData, int: &LocalInterface) {
        let received_at = chrono::Utc::now();
        let mut buf = String::new();
        for (chid, dat) in data.per_channel.clone() {
            if let Some(ch) = int
                .dispatch(
                    msg::Target::Instance(self.registry.clone()),
                    registry::EV_REGISTRY_QUERY_CHANNEL,
                    chid,
                )
                .await
                .unwrap()
                .expect("Failed to query registry")
            {
                //TODO: verify that types match
                let _ = writeln!(
                    buf,
                    "Channel {chid} ({}) => {:?}",
                    <ChannelName as Into<String>>::into(ch.name.clone()),
                    dat
                );
            } else {
                warn!(
                    "Data contains channel id {chid} (={dat:?}) which is not known to this server"
                );
                let _ = writeln!(buf, "Channel {chid} (<unknown>) => {:?}", dat);
            }
        }
        info!("Received data:\n{buf}");
        int.dispatch(
            msg::Target::Any,
            EV_WEATHER_DATA_RECEIVED,
            Record {
                recorded_at: received_at,
                recorded_by: self.meta_station_id.unwrap(),
                data: data.per_channel,
            },
        )
        .await
        .unwrap();
    }
}
