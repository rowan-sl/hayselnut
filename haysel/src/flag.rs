use futures::future::Future;
use futures::task::{AtomicWaker, Context, Poll};
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;

#[derive(Debug)]
pub struct Flag {
    waker: AtomicWaker,
    set: AtomicBool,
}

impl Flag {
    pub fn new() -> Self {
        Self {
            waker: AtomicWaker::new(),
            set: AtomicBool::new(false),
        }
    }

    pub fn signal(&self) {
        self.set.store(true, Relaxed);
        self.waker.wake();
    }

    pub fn reset(&self) {
        self.set.store(false, Relaxed);
    }
}

impl Future for Flag {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // quick check to avoid registration if already done.
        if self.set.load(Relaxed) {
            return Poll::Ready(());
        }

        self.waker.register(cx.waker());

        // Need to check condition **after** `register` to avoid a race
        // condition that would result in lost notifications.
        if self.set.load(Relaxed) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}
