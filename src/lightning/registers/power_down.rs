use crate::lightning::{
    registers::{Mode, Register},
    repr::PowerDownStatus,
};

pub(crate) struct PowerDown;
impl Register for PowerDown {
    type Repr = PowerDownStatus;

    fn name(&self) -> &'static str {
        &"PWD"
    }

    fn description(&self) -> &'static str {
        &"Power-down"
    }

    fn address(&self) -> u8 {
        0x00
    }

    fn mode(&self) -> Mode {
        Mode::ReadWrite
    }

    fn mask(&self) -> u8 {
        0b_0000_0001
    }

    fn default_value(&self) -> u8 {
        0b_0
    }
}
