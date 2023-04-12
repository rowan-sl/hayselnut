use crate::lightning::{
    registers::{Mode, Register},
    repr::NoiseFloorThreshold,
};

pub(crate) struct NoiseFloorLevel;
impl Register for NoiseFloorLevel {
    type Repr = NoiseFloorThreshold;

    fn name(&self) -> &'static str {
        &"NF_LEV"
    }

    fn description(&self) -> &'static str {
        &"unimplemented!()"
    }

    fn address(&self) -> u8 {
        0x01
    }

    fn mode(&self) -> Mode {
        Mode::ReadWrite
    }

    fn mask(&self) -> u8 {
        0b_0111_0000
    }

    fn default_value(&self) -> u8 {
        0b_010
    }
}
