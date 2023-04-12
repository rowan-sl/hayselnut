use anyhow::Result;

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
        for c in &mut self.consumers {
            c.handle(&record).await?;
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
