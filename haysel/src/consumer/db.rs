use anyhow::Result;
use mycelium::station::{
    capabilities::{ChannelData, ChannelType, KnownChannels},
    identity::KnownStations,
};

use crate::{
    route::StationInfoUpdate,
    shutdown::{async_drop::AsyncDrop, ShutdownHandle},
    tsdb2::{alloc::Storage, repr::DataGroupType, Database},
    util::Take,
};

use super::{Record, RecordConsumer};

pub struct RecordDB<S: Storage> {
    db: Take<Database<S>>,
    stations: KnownStations,
    channels: KnownChannels,
    drop: AsyncDrop,
}

impl<S: Storage> RecordDB<S> {
    pub async fn new(db: Database<S>, shutdown: ShutdownHandle) -> Result<Self> {
        Ok(Self {
            db: Take::new(db),
            stations: KnownStations::default(),
            channels: KnownChannels::default(),
            drop: AsyncDrop::new(shutdown).await,
        })
    }
}

#[async_trait]
impl<S: Storage> RecordConsumer for RecordDB<S> {
    async fn handle(
        &mut self,
        Record {
            data,
            recorded_at,
            recorded_by,
        }: &Record,
    ) -> Result<()> {
        for (recorded_from, record) in data {
            let record = match record {
                ChannelData::Float(x) => x,
                ChannelData::Event { .. } => {
                    warn!("Attempted to record event-type data using the database, but that is not supported");
                    continue;
                }
            };
            self.db
                .add_data(*recorded_by, *recorded_from, *recorded_at, *record)
                .await?;
        }
        Ok(())
    }

    async fn update_station_info(&mut self, updates: &[StationInfoUpdate]) -> Result<()> {
        for update in updates {
            match update {
                StationInfoUpdate::InitialState { stations, channels } => {
                    // TODO: verify that the stations in the database match with those given here
                    warn!("Initial state verification unimplemented");
                    self.stations = stations.clone();
                    self.channels = channels.clone();
                }
                &StationInfoUpdate::NewStation { id } => {
                    self.db.add_station(id).await?;
                    self.stations
                        .insert_station(
                            id,
                            mycelium::station::identity::StationInfo {
                                supports_channels: vec![],
                            },
                        )
                        .map_err(|_| {
                            anyhow!("NewStation was called with a station that already existed")
                        })?;
                }
                // the database does not track channels independantly of stations
                StationInfoUpdate::NewChannel { id, ch } => {
                    self.channels
                        .insert_channel_with_id(ch.clone(), *id)
                        .map_err(|_| {
                            anyhow!("NewChannel was called with a channel that already exists")
                        })?;
                }
                &StationInfoUpdate::StationNewChannel { station, channel } => {
                    let channel_info = self.channels.get_channel(&channel)
                        .ok_or_else(|| anyhow!("StationNewChannel attempted to associate a station with a channel that does not exist!"))?;
                    self.stations.map_info(&station, |_id, info| {
                        info.supports_channels.push(channel);
                    }).ok_or_else(|| anyhow!("StationNewChannel attempted to associate a channel with a station that does not exist"))?;
                    self.db
                        .add_channel(
                            station,
                            channel,
                            match channel_info.ty {
                                ChannelType::Periodic => DataGroupType::Periodic,
                                ChannelType::Triggered => DataGroupType::Sporadic,
                            },
                        )
                        .await?;
                }
            }
        }
        Ok(())
    }
}

impl<S: Storage> Drop for RecordDB<S> {
    fn drop(&mut self) {
        let db = self.db.take();
        self.drop.run(async {
            if let Err(e) = db.close().await {
                error!("Failed to close database: {e:#?}")
            }
        })
    }
}
