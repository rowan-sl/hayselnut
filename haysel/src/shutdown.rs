use tokio::sync::{broadcast, mpsc};

#[derive(Debug)]
pub struct ShutdownHandle {
    #[allow(unused)]
    inner: mpsc::Sender<()>,
    trigger: broadcast::Receiver<()>,
}

impl ShutdownHandle {
    #[allow(unused)]
    pub async fn wait_for_shutdown(&mut self) {
        let _ = self.trigger.recv().await;
    }
}

#[derive(Debug)]
pub struct Shutdown {
    tx: mpsc::Sender<()>,
    rx: mpsc::Receiver<()>,
    trigger: broadcast::Sender<()>,
}

impl Shutdown {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(1);
        let (trigger, _) = broadcast::channel(1);
        Self { tx, rx, trigger }
    }

    pub fn handle(&self) -> ShutdownHandle {
        ShutdownHandle {
            inner: self.tx.clone(),
            trigger: self.trigger.subscribe(),
        }
    }

    pub async fn wait_for_completion(mut self) {
        drop(self.tx);
        self.rx.recv().await;
    }

    pub fn trigger_shutdown(&mut self) {
        let _ = self.trigger.send(());
    }
}
