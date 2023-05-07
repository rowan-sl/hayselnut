use std::collections::HashMap;

use bme280::{i2c::BME280, Measurements};
use embedded_hal::i2c::I2c;
use esp_idf_hal::delay;
use squirrel::api::station::capabilities::{
    Channel, ChannelData, ChannelID, ChannelType, ChannelValue,
};

use super::{Peripheral, PeripheralState, SensorPeripheral};

#[derive(Debug)]
pub struct PeriphBME280<T: I2c> {
    inner: PeripheralState<BME280<T>, BME280<T>, bme280::Error<T::Error>>,
}

impl<T: I2c> PeriphBME280<T> {
    pub fn new(i2c: T) -> Self {
        let mut bme = BME280::new(i2c, 0x77);
        Self {
            inner: PeripheralState::new(move || match bme.init(&mut delay::Ets) {
                Ok(..) => Ok(bme),
                Err(e) => Err((bme, e)),
            }),
        }
    }
}

impl<T: I2c> Peripheral for PeriphBME280<T> {
    type Error = bme280::Error<T::Error>;
    fn fix(&mut self) {
        self.inner
            .retry_init(|mut bme, _err| match bme.init(&mut delay::Ets) {
                Ok(..) => Ok(bme),
                Err(e) => Err((bme, e)),
            });
        self.inner.resolve_err(|bme, _err| {
            // re-init, if connected it will work.
            // this will fix things if it disconencted due to loosing power
            bme.init(&mut delay::Ets)
        });
    }
    fn err(&self) -> Option<&Self::Error> {
        self.inner.err()
    }
}

impl<T: I2c> SensorPeripheral for PeriphBME280<T> {
    fn channels(&self) -> Vec<Channel> {
        vec![
            Channel {
                name: "temperature".into(),
                value: ChannelValue::Float,
                ty: ChannelType::Periodic,
            },
            Channel {
                name: "humidity".into(),
                value: ChannelValue::Float,
                ty: ChannelType::Periodic,
            },
            Channel {
                name: "pressure".into(),
                value: ChannelValue::Float,
                ty: ChannelType::Periodic,
            },
        ]
    }

    fn read(
        &mut self,
        map_fn: &impl Fn(&str) -> ChannelID,
    ) -> Option<HashMap<ChannelID, ChannelData>> {
        self.inner.map(|bme| {
            let mut map = HashMap::new();
            let mut set = |key, val| map.insert(map_fn(key), ChannelData::Float(val));
            let _ = bme.measure(&mut delay::Ets)?;
            let Measurements {
                temperature,
                humidity,
                pressure,
                ..
            } = bme.measure(&mut delay::Ets)?;
            set("temperature", temperature);
            set("humidity", humidity);
            set("pressure", pressure);
            Ok(map)
        })
    }
}
