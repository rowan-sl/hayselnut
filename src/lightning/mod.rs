pub mod registers;
pub mod repr;

use std::{fmt::Debug, thread::sleep, time::Duration};

use anyhow::Result;
use embedded_hal::digital::{
    self,
    InputPin, OutputPin,
    PinState,
};

use registers::Register;
use repr::{
    CalibrateOscilatorsCmd, DistanceEstimate, IntType, MaskDisturberEvent,
    MinimumLightningThreshold, NoiseFloorThreshold, OutputTRCOOnIRQ, PowerDownStatus,
    PresetDefaultCmd, SensorLocation, SignalVerificationThreshold,
};

const CLOCK_GENERATION_DELAY: Duration = Duration::from_millis(2);
const IRQ_TRIGGER_TO_READY_DELAY: Duration = Duration::from_millis(2);
const LIGHTNING_CALCULATION_DELAY: Duration = Duration::from_millis(2);
// const DISTURBER_DEACTIVATION_PERIOD: Duration = Duration::from_millis(1500);
// const APPROXIMATE_MINIMUM_LIGHTNING_INTERVAL: Duration = Duration::from_secs(1);

// bit of byte in most significant first byte order
fn bit(byte: u8, bit: u8) -> bool {
    assert!(bit <= 7u8);
    if byte >> (7u8 - bit) & 1 == 1 {
        true
    } else {
        false
    }
}

// bits of a byte in most significant first byte order
fn bits(byte: u8) -> [bool; 8] {
    [
        bit(byte, 0),
        bit(byte, 1),
        bit(byte, 2),
        bit(byte, 3),
        bit(byte, 4),
        bit(byte, 5),
        bit(byte, 6),
        bit(byte, 7),
    ]
}

/// bits are in msb first ordering
fn from_bits(bits: [bool; 8]) -> u8 {
    let mut byte = 0u8;
    for bit in 0..8 {
        byte |= (bits[bit] as u8) << (7u8 - bit as u8);
    }
    byte
}

pub(crate) fn calculate_bitshift(mask: u8) -> u8 {
    for i in 0..7 {
        if (mask & (1 << i)) == 1 {
            return i;
        }
    }

    0
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    DistanceEstimationChanged,
    DisturbanceDetected,
    NoiseLevelTooHigh,
    Lightning { distance: DistanceEstimate },
    InvalidInt(u8),
}

pub struct LightningSensor<CS: OutputPin, CLK: OutputPin, MOSI: OutputPin, MISO: InputPin> {
    cs: CS,
    clk: CLK,
    mosi: MOSI,
    miso: MISO,
    transaction_delay: Duration,
}

impl<CS: OutputPin, CLK: OutputPin, MOSI: OutputPin, MISO: InputPin>
    LightningSensor<CS, CLK, MOSI, MISO>
where
    <CS as digital::ErrorType>::Error: std::error::Error + Sync + Send + 'static,
    <CLK as digital::ErrorType>::Error: std::error::Error + Sync + Send + 'static,
    <MOSI as digital::ErrorType>::Error: std::error::Error + Sync + Send + 'static,
    <MISO as digital::ErrorType>::Error: std::error::Error + Sync + Send + 'static,
{
    pub fn new(cs: CS, clk: CLK, mosi: MOSI, miso: MISO) -> Result<Self> {
        let speed_hz = 100;
        let mut s = Self {
            cs,
            clk,
            mosi,
            miso,
            transaction_delay: Duration::new(1, 0) / speed_hz,
        };
        s.cs.set_high()?;
        s.clk.set_low()?;
        Ok(s)
    }

    /// waiting should happen durnig a function, and not at the start or end
    fn wait(&self) {
        std::thread::sleep(self.transaction_delay);
    }

    fn begin(&mut self) -> Result<(), <CS as digital::ErrorType>::Error> {
        self.cs.set_low()
    }

    fn end(&mut self) -> Result<(), <CS as digital::ErrorType>::Error> {
        self.cs.set_high()
    }

    fn pulse_clk(&mut self) -> Result<(), <CLK as digital::ErrorType>::Error> {
        self.clk.set_high()?;
        self.wait();
        self.clk.set_low()?;
        Ok(())
    }

    /// assuming cs is low and in read mode, read one bit from the sensor
    fn read_bit(&mut self) -> Result<bool> {
        self.pulse_clk()?;
        self.wait();
        Ok(self.miso.is_high()?)
    }

    /// assuming cs is low and in write mode, write one bit to the sensor
    fn write_bit(&mut self, bit: bool) -> Result<()> {
        self.mosi.set_state(PinState::from(bit))?;
        self.wait();
        self.pulse_clk()?;
        Ok(())
    }

    /// doing a complete transaction, write the bits of `data` to `reg`.
    ///
    /// handles appropreate waiting at the start and end
    fn write_reg_raw(&mut self, reg: u8, data: u8) -> Result<()> {
        self.begin()?;
        self.wait();
        self.write_bit(false)?;
        self.wait();
        self.write_bit(false)?;
        self.wait();
        for bit in &bits(reg)[2..]
        /* last 6 bits (cuts off upper 2 bits) */
        {
            self.write_bit(*bit)?;
            self.wait();
        }
        for bit in bits(data) {
            self.write_bit(bit)?;
            self.wait();
        }
        self.end()?;
        self.wait();
        Ok(())
    }

    /// doing a complete transaction, read the bits of `reg`.
    ///
    /// handles appropreate waiting at the start and end
    fn read_reg_raw(&mut self, reg: u8) -> Result<u8> {
        self.begin()?;
        self.wait();
        self.write_bit(false)?;
        self.wait();
        self.write_bit(true)?;
        self.wait();
        for bit in &bits(reg)[2..]
        /* last 6 bits (cuts off upper 2 bits) */
        {
            self.write_bit(*bit)?;
            self.wait();
        }
        let mut bits = [false; 8];
        for i in 0..8 {
            bits[i] = self.read_bit()?;
            self.wait();
        }
        let value = from_bits(bits);
        // println!("raw read,\n addr = {reg:#04x},\n read value = {value:#010b}");
        Ok(value)
    }

    pub fn read_reg<R: Register>(&mut self, register: R) -> Result<<R as Register>::Repr>
    where
        <R as Register>::Repr: Debug,
    {
        let data = self.read_reg_raw(register.address())?;
        let value = (data & register.mask()) >> calculate_bitshift(register.mask());
        let typed_value = <R as Register>::Repr::from(value);
        // println!("reading,\n reg_type = {},\n raw_reg_addr = {:#x} \n typed_value = {typed_value:?}, \n value = {value:#010b},\n mask = {:#010b},\n raw_reg_data = {data:#010b}", std::any::type_name::<R>(), register.address(), register.mask());
        Ok(typed_value)
    }

    pub fn write_reg<R: Register>(
        &mut self,
        register: R,
        payload: <R as Register>::Repr,
    ) -> Result<()>
    where
        <R as Register>::Repr: Clone + Debug,
    {
        let payload_byte: u8 = payload.clone().into();
        let bitshift = calculate_bitshift(register.mask());
        assert!(payload_byte <= (register.mask() >> bitshift));

        let current_data = self.read_reg_raw(register.address())?;
        let to_write = (current_data ^ register.mask()) | (payload_byte << bitshift);
        // println!("writing,\n reg_type = {}, \n payload = {payload:?}, \n payload_bytes = {payload_byte:#010b},\n mask = {:#010b},\n current_data = {current_data:#010b},\n to_write = {to_write:#010b}", std::any::type_name::<R>(),register.mask());
        self.write_reg_raw(register.address(), to_write)?;

        Ok(())
    }

    pub fn reset_int_reg(&mut self) -> Result<()> {
        let reset_int_reg =
            self.read_reg_raw(registers::Interrupt.address())? & (!registers::Interrupt.mask());
        self.write_reg_raw(0x03, reset_int_reg)?;
        Ok(())
    }

    pub fn get_latest_event_and_reset(&mut self) -> Result<Event> {
        let int_type = self.read_reg(registers::Interrupt)?;
        self.reset_int_reg()?;
        Ok(match int_type {
            IntType::DistanceEstimationChanged => Event::DistanceEstimationChanged,
            IntType::DisturberDetected => Event::DisturbanceDetected,
            IntType::NoiseLevelTooHigh => Event::NoiseLevelTooHigh,
            IntType::Lightning => {
                sleep(LIGHTNING_CALCULATION_DELAY);
                let distance = self.read_reg(registers::DistanceEstimation)?;
                // println!("    lighting detected!");
                // println!("    estimated distance: {distance:?}");
                Event::Lightning { distance }
            }
            IntType::Invalid(value) => Event::InvalidInt(value),
        })
    }

    /// needs to be re-run before useage, and after resuming from power off mode
    pub fn perform_initial_configuration(&mut self) -> Result<()> {
        self.set_status(PowerDownStatus::On)?;
        self.configure_oscilators()?;
        self.configure_defaults()?;
        Ok(())
    }

    pub fn set_status(&mut self, status: PowerDownStatus) -> Result<()> {
        self.write_reg(registers::PowerDown, status)?;
        sleep(Duration::from_millis(2));
        Ok(())
    }

    fn configure_oscilators(&mut self) -> Result<()> {
        self.write_reg(registers::CalibrateOscillators, CalibrateOscilatorsCmd)?;
        sleep(Duration::from_millis(2));
        self.write_reg(registers::DisplayTrcoOnIrqPin, OutputTRCOOnIRQ(true))?;
        sleep(CLOCK_GENERATION_DELAY);
        self.write_reg(registers::DisplayTrcoOnIrqPin, OutputTRCOOnIRQ(false))?;
        sleep(Duration::from_millis(2));

        Ok(())
    }

    pub fn configure_defaults(&mut self) -> Result<()> {
        self.write_reg(registers::PresetDefault, PresetDefaultCmd)?;
        Ok(())
    }

    pub fn configure_sensor_placing(&mut self, placing: &SensorLocation) -> Result<()> {
        self.write_reg(registers::AfeGainBoost, *placing)?;
        Ok(())
    }

    pub fn configure_minimum_lightning_threshold(
        &mut self,
        minimum_lightning_threshold: &MinimumLightningThreshold,
    ) -> Result<()> {
        self.write_reg(
            registers::MinimumNumberOfLightning,
            *minimum_lightning_threshold,
        )?;
        Ok(())
    }

    pub fn configure_noise_floor_threshold(
        &mut self,
        noise_floor_threshold: &NoiseFloorThreshold,
    ) -> Result<()> {
        self.write_reg(registers::NoiseFloorLevel, *noise_floor_threshold)?;
        Ok(())
    }

    pub fn configure_signal_verification_threshold(
        &mut self,
        signal_verification_threshold: &SignalVerificationThreshold,
    ) -> Result<()> {
        self.write_reg(registers::WatchdogThreshold, *signal_verification_threshold)?;

        Ok(())
    }

    pub fn configure_ignore_disturbances(
        &mut self,
        ignore_disturbances: &MaskDisturberEvent,
    ) -> Result<()> {
        self.write_reg(registers::MaskDisturber, *ignore_disturbances)?;
        Ok(())
    }
}
