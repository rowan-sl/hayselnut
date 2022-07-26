use crate::{registers::{Mode, Register}, repr::MinimumLightningThreshold};

pub(crate) struct MinimumNumberOfLightning;
impl Register for MinimumNumberOfLightning {
    type Repr = MinimumLightningThreshold;

    fn name(&self) -> &'static str {
        &"MIN_NUM_LIGH"
    }

    fn description(&self) -> &'static str {
        &"Minimum number of lightning"
    }

    fn address(&self) -> u8 {
        0x02
    }

    fn mode(&self) -> Mode {
        Mode::ReadWrite
    }

    fn mask(&self) -> u8 {
        0b_0011_0000
    }

    fn default_value(&self) -> u8 {
        0b_00
    }
}
