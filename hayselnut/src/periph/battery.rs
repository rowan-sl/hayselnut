use anyhow::Result;
use esp_idf_hal::{
    adc::{self, Adc, AdcChannelDriver, AdcDriver},
    gpio::ADCPin,
};
use esp_idf_sys::EspError;

pub struct BatteryMonitor<'a, P: ADCPin>(AdcChannelDriver<'a, P, adc::Atten11dB<P::Adc>>);

impl<'a, P: ADCPin> BatteryMonitor<'a, P> {
    pub fn new(pin: P) -> Result<Self, EspError> {
        Ok(Self(AdcChannelDriver::new(pin)?))
    }

    pub fn read<'b, ADC: Adc>(&mut self, driver: &mut AdcDriver<'b, ADC>) -> Result<f32, EspError> {
        // mull by 2, measured through a voltage divider
        Ok((driver.read(&mut self.0)? * 2 / 10) as f32 / 100.0)
    }
}
