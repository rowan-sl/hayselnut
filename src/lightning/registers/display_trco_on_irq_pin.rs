use crate::lightning::{
    registers::{Mode, Register},
    repr::OutputTRCOOnIRQ,
};

pub(crate) struct DisplayTrcoOnIrqPin;
impl Register for DisplayTrcoOnIrqPin {
    type Repr = OutputTRCOOnIRQ;

    fn name(&self) -> &'static str {
        &"DISP_TRCO"
    }

    fn description(&self) -> &'static str {
        &"Display TRCO on IRQ pin"
    }

    fn address(&self) -> u8 {
        0x08
    }

    fn mode(&self) -> Mode {
        Mode::ReadWrite
    }

    fn mask(&self) -> u8 {
        0b_0010_0000
    }

    fn default_value(&self) -> u8 {
        0b_0
    }
}
