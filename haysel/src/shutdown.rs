use tokio::sync::{broadcast, mpsc};

use crate::misc::Take;

pub mod async_drop;
pub mod util;

#[derive(Debug)]
pub struct ShutdownHandle {
    #[allow(unused)]
    inner: mpsc::Sender<()>,
    listener: broadcast::Receiver<()>,
    trigger: broadcast::Sender<()>,
}

impl ShutdownHandle {
    pub async fn wait_for_shutdown(&mut self) {
        let _ = self.listener.recv().await;
    }

    pub fn trigger_shutdown(&mut self) {
        let _ = self.trigger.send(());
    }
}

pub struct Shutdown {
    tx: Take<mpsc::Sender<()>>,
    rx: mpsc::Receiver<()>,
    trigger: broadcast::Sender<()>,
}

impl Shutdown {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(1);
        let (trigger, _) = broadcast::channel(1);
        Self {
            tx: Take::new(tx),
            rx,
            trigger,
        }
    }

    pub fn handle(&self) -> ShutdownHandle {
        ShutdownHandle {
            inner: self.tx.clone(),
            listener: self.trigger.subscribe(),
            trigger: self.trigger.clone(),
        }
    }

    pub async fn wait_for_completion(&mut self) {
        drop(self.tx.take());
        self.rx.recv().await;
    }

    pub fn trigger_shutdown(&self) {
        let _ = self.trigger.send(());
    }
}
