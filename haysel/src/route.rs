use anyhow::Result;
use futures::future::join_all;

use super::consumer::{Record, RecordConsumer};

#[derive(Default)]
pub struct Router {
    consumers: Vec<Box<dyn RecordConsumer>>,
    properly_dropped: bool,
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_consumer<C: RecordConsumer + 'static>(&mut self, consumer: C) -> &mut Self {
        self.consumers.push(Box::new(consumer));
        self
    }

    pub async fn process(&mut self, record: Record) -> Result<()> {
        if join_all(
            self.consumers
                .iter_mut()
                .map(|consumer| async { consumer.handle(&record).await }),
        )
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

    /// call this to properly shutdown all consumers attached to this Router.
    ///
    /// this MUST be called, you may NOT just drop Router
    pub async fn close(mut self) {
        for c in self.consumers.drain(..) {
            c.close().await;
        }
        self.properly_dropped = true;
    }
}

impl Drop for Router {
    fn drop(&mut self) {
        if !self.properly_dropped {
            error!("Router may NOT be dropped except through `close_consumers`");
        }
    }
}
