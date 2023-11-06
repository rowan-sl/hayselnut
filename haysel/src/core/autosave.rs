use std::{convert::Infallible, time::Duration};

use roundtable::{
    common::EV_BUILTIN_AUTOSAVE,
    handler::{HandlerInit, LocalInterface, MethodRegister},
    handler_decl_t, method_decl_owned,
    msg::{self, Str},
};
use tokio::time::{interval_at, Instant, Interval};

pub struct AutosaveDispatch {
    interval: Duration,
}

impl AutosaveDispatch {
    pub fn new(every: Duration) -> Self {
        Self { interval: every }
    }

    #[instrument(skip(self, interval, int))]
    async fn timer_complete(
        &mut self,
        mut interval: Interval,
        int: &LocalInterface,
    ) -> Result<(), <Self as HandlerInit>::Error> {
        debug!("saving...");
        int.announce(msg::Target::Any, EV_BUILTIN_AUTOSAVE, ())
            .await
            .unwrap();
        int.bg_spawn(EV_PRIV_TIMER_COMPLETED, async move {
            interval.tick().await;
            interval
        });
        Ok(())
    }
}

method_decl_owned!(EV_PRIV_TIMER_COMPLETED, Interval, ());

#[async_trait]
impl HandlerInit for AutosaveDispatch {
    const DECL: roundtable::msg::HandlerType = handler_decl_t!("Autosave event dispatcher");
    type Error = Infallible;
    async fn init(&mut self, int: &LocalInterface) -> Result<(), Self::Error> {
        let mut interval = interval_at(Instant::now() + self.interval, self.interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        self.timer_complete(interval, int).await;
        Ok(())
    }
    fn describe(&self) -> Str {
        Str::Owned(format!(
            "Autosave event dispatch (every: {:?})",
            self.interval
        ))
    }
    fn methods(&self, reg: &mut MethodRegister<Self>) {
        reg.register_owned(Self::timer_complete, EV_PRIV_TIMER_COMPLETED);
    }
}
