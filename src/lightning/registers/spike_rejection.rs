use crate::lightning::{
    registers::{Mode, Register},
    repr::SpikeRejectionSetting,
};

pub(crate) struct SpikeRejection;
impl Register for SpikeRejection {
    type Repr = SpikeRejectionSetting;

    fn name(&self) -> &'static str {
        &"SREJ"
    }

    fn description(&self) -> &'static str {
        &"Spike rejection"
    }

    fn address(&self) -> u8 {
        0x02
    }

    fn mode(&self) -> Mode {
        Mode::ReadWrite
    }

    fn mask(&self) -> u8 {
        0b_0000_1111
    }

    fn default_value(&self) -> u8 {
        0b_0010
    }
}
