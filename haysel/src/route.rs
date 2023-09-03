use anyhow::Result;
use futures::{future::join_all, Future};
use mycelium::station::{
    capabilities::{Channel, ChannelID, KnownChannels},
    identity::{KnownStations, StationID},
};

use super::consumer::{Record, RecordConsumer};

#[derive(Default)]
pub struct Router {
    consumers: Vec<Box<(dyn RecordConsumer + Send + Sync + 'static)>>,
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_consumer<C: RecordConsumer + Send + Sync + 'static>(
        &mut self,
        consumer: C,
    ) -> &mut Self {
        self.consumers.push(Box::new(consumer));
        self
    }

    // fun times with l i f e t i m e s!
    async fn consumer_map<'this: 'consumer, 'consumer: 'result, 'result, Fun, Fut>(
        &'this mut self,
        f: Fun,
    ) -> Result<()>
    where
        Fun: FnMut(&'consumer mut Box<(dyn RecordConsumer + Send + Sync + 'static)>) -> Fut,
        Fut: Future<Output = Result<()>> + 'result,
    {
        if join_all(self.consumers.iter_mut().map(f))
            .await
            .into_iter()
            .filter(Result::is_err)
            .map(|res| {
                if let Err(e) = res {
                    error!("Error occured in consumer processing function: {e}");
                }
            })
            .count()
            != 0
        {
            bail!("Error occured in consumer processing function")
        }
        Ok(())
    }

    pub async fn process(&mut self, record: Record) -> Result<()> {
        self.consumer_map(|consumer| consumer.handle(&record))
            .await?;
        Ok(())
    }

    pub async fn update_station_info(&mut self, updates: &[StationInfoUpdate]) -> Result<()> {
        if updates.is_empty() {
            return Ok(());
        }
        self.consumer_map(|consumer| async { consumer.update_station_info(updates).await })
            .await?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum StationInfoUpdate {
    /// event sent when a router is created, gives information on the current state of things
    InitialState {
        stations: KnownStations,
        channels: KnownChannels,
    },
    /// new station registered
    NewStation { id: StationID },
    /// new channel registered
    NewChannel { id: ChannelID, ch: Channel },
    /// new association of channel with a station
    StationNewChannel {
        station: StationID,
        channel: ChannelID,
    },
}
