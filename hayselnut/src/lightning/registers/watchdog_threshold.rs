use crate::lightning::{
    registers::{Mode, Register},
    repr::SignalVerificationThreshold,
};

pub(crate) struct WatchdogThreshold;
impl Register for WatchdogThreshold {
    type Repr = SignalVerificationThreshold;

    fn name(&self) -> &'static str {
        &"WDTH"
    }

    fn description(&self) -> &'static str {
        &"Watchdog threshold"
    }

    fn address(&self) -> u8 {
        0x01
    }

    fn mode(&self) -> Mode {
        Mode::ReadWrite
    }

    fn mask(&self) -> u8 {
        0b_0000_1111
    }

    fn default_value(&self) -> u8 {
        0b_0001
    }
}
