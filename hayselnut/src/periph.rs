use squirrel::api::station::capabilities::{Channel, ChannelData, ChannelID};
use std::collections::HashMap;

pub mod battery;
pub mod bme280;

#[derive(Debug)]
pub struct PeripheralState<TOk, TErr, E> {
    /// Err if init fails
    /// None if panic occurs while doing `retry_init`
    state: Option<Result<TOk, TErr>>,
    error: Option<E>,
}

impl<TOk, TErr, E> PeripheralState<TOk, TErr, E> {
    pub fn new(init_fn: impl FnOnce() -> Result<TOk, (TErr, E)>) -> Self {
        match init_fn() {
            Ok(state) => Self {
                state: Some(Ok(state)),
                error: None,
            },
            Err((estate, error)) => Self {
                state: Some(Err(estate)),
                error: Some(error),
            },
        }
    }

    /// Retrys failed initialization. if initialization has previously succeded, nothing is done
    pub fn retry_init(&mut self, init_fn: impl FnOnce(TErr, E) -> Result<TOk, (TErr, E)>) {
        match self
            .state
            .take()
            .expect("panic previously occurred in `retry_init`, state has been lost")
        {
            Ok(state) => self.state = Some(Ok(state)),
            Err(estate) => match init_fn(estate, self.error.take().unwrap()) {
                Ok(state) => self.state = Some(Ok(state)),
                Err((estate, err)) => {
                    self.state = Some(Err(estate));
                    self.error = Some(err)
                }
            },
        }
    }

    pub fn map<T>(&mut self, f: impl FnOnce(&mut TOk) -> Result<T, E>) -> Option<T> {
        if self.error.is_some() {
            None
        } else {
            let Some(Ok(state)) = &mut self.state else {
                panic!("Attempted to call `PeripheralState::map` without handling initialization error");
            };
            match f(state) {
                Ok(ret) => Some(ret),
                Err(err) => {
                    self.error = Some(err);
                    None
                }
            }
        }
    }

    pub fn resolve_err(&mut self, f: impl FnOnce(&mut TOk, E) -> Result<(), E>) {
        if let Some(e) = self.error.take() {
            let Some(Ok(state)) = &mut self.state else {
                panic!("Attempted to call `PeripheralState::map` without handling initialization error");
            };
            if let Err(e) = f(state, e) {
                self.error = Some(e);
            }
        }
    }

    pub fn is_init(&self) -> bool {
        self.state.as_ref().map(|v| v.is_ok()).unwrap_or(false)
    }

    pub fn err(&self) -> Option<&E> {
        self.error.as_ref()
    }
}

pub trait Peripheral {
    type Error;
    fn fix(&mut self);
    fn err(&self) -> Option<&Self::Error>;
}

pub trait SensorPeripheral: Peripheral {
    fn channels(&self) -> Vec<Channel>;
    fn read(
        &mut self,
        map_fn: &impl Fn(&str) -> ChannelID,
    ) -> Option<HashMap<ChannelID, ChannelData>>;
}
