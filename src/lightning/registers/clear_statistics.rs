use crate::lightning::{
    registers::{Mode, Register},
    repr::SetClearStatistics,
};

pub(crate) struct ClearStatistics;
impl Register for ClearStatistics {
    type Repr = SetClearStatistics;

    fn name(&self) -> &'static str {
        &"CL_STAT"
    }

    fn description(&self) -> &'static str {
        &"Clear statistics"
    }

    fn address(&self) -> u8 {
        0x02
    }

    fn mode(&self) -> Mode {
        Mode::ReadWrite
    }

    fn mask(&self) -> u8 {
        0b_0100_0000
    }

    fn default_value(&self) -> u8 {
        0b_1
    }
}
