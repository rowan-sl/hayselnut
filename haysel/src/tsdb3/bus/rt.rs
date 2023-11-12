//! async / blocking interface for the database (to bridge roundtable <-> TSDBv3)

use chrono::{DateTime, Utc};
use flume::{Receiver, Sender};
use mycelium::station::{
    capabilities::{Channel, ChannelData, KnownChannels},
    identity::KnownStations,
};
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::{
    dispatch::application::Record,
    tsdb3::{query::QueryParams, DB},
};

pub enum Msg {
    Query {
        params: QueryParams,
        response: oneshot::Sender<Vec<(DateTime<Utc>, f32)>>,
    },
    EnsureExists {
        stations: KnownStations,
        channels: KnownChannels,
    },
    NewStation {
        sid: Uuid,
    },
    NewChannel {
        sid: Uuid,
        cid: Uuid,
        inf: Channel,
    },
    Record {
        record: Record,
    },
}

pub fn launch(db: DB) -> Sender<Msg> {
    let (send, recv) = flume::bounded(64);
    std::thread::spawn(move || runner(db, recv));
    send
}

pub fn runner(mut db: DB, queue: Receiver<Msg>) {
    loop {
        let recv = match queue.recv() {
            Ok(x) => x,
            Err(flume::RecvError::Disconnected) => {
                warn!("TSDBv3 <-> Roundtable comm queue closed, runtime task will now close");
                break;
            }
        };
        match recv {
            Msg::Query { params, response } => {
                let resp = db.query_data(params);
                let _ = response.send(resp);
            }
            Msg::EnsureExists { stations, channels } => {
                for &id in stations.stations() {
                    db.insert_station(id);
                    db.insert_channels(id, channels.channels().map(|(id, _)| *id));
                }
            }
            Msg::NewStation { sid } => db.insert_station(sid),
            Msg::NewChannel { sid, cid, .. } => db.insert_channels(sid, [cid]),
            Msg::Record { record } => {
                for (ch, val) in &record.data {
                    db.insert_data(
                        record.recorded_by,
                        *ch,
                        record.recorded_at,
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
    }
}
