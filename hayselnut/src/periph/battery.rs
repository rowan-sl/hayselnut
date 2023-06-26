use anyhow::Result;
use esp_idf_hal::{
    adc::{self, Adc, AdcChannelDriver, AdcDriver},
    gpio::ADCPin,
    peripheral::Peripheral as EspPeripheral,
};
use esp_idf_sys::EspError;

pub struct BatteryMonitor<'a, 'b, ADC: Adc, P: ADCPin>(
    AdcDriver<'a, ADC>,
    AdcChannelDriver<'b, P, adc::Atten11dB<P::Adc>>,
);

impl<'a, 'b, ADC: Adc, P: ADCPin> BatteryMonitor<'a, 'b, ADC, P> {
    pub fn new(adc: impl EspPeripheral<P = ADC> + 'a, pin: P) -> Result<Self, EspError> {
        Ok(Self(
            AdcDriver::new(adc, &adc::config::Config::new().calibration(true))?,
            AdcChannelDriver::new(pin)?,
        ))
    }

    pub fn read(&mut self) -> Result<f32, EspError> {
        // mull by 2, measured through a voltage divider
        Ok((self.0.read(&mut self.1)? * 2 / 10) as f32 / 100.0)
    }
}
