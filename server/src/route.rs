use super::consumer::RecordConsumer;

#[derive(Default)]
pub struct Router {
    consumers: Vec<Box<dyn RecordConsumer>>,
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_consumer<C: RecordConsumer + 'static>(&mut self, consumer: C) -> &mut Self {
        self.consumers.push(Box::new(consumer));
        self
    }
}
