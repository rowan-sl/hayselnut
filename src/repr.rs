

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntType {
    DistanceEstimationChanged,
    NoiseLevelTooHigh,
    DisturberDetected,
    Lightning,
    // remove this once the issues is figured out
    Invalid(u8),
}

impl From<u8> for IntType {
    fn from(byte: u8) -> Self {
        match byte {
            0b_0000 => Self::DistanceEstimationChanged,
            0b_0001 => Self::NoiseLevelTooHigh,
            0b_0100 => Self::DisturberDetected,
            0b_1000 => Self::Lightning,
            other => IntType::Invalid(other)
        }
    }
}

impl Into<u8> for IntType {
    fn into(self) -> u8 {
        unreachable!()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SensorLocation {
    Indoor,
    Outdoor,
}

impl Into<u8> for SensorLocation {
    fn into(self) -> u8 {
        match self {
            SensorLocation::Indoor => 0b_1_0010_u8,
            SensorLocation::Outdoor => 0b_0_1110_u8,
        }
    }
}

impl From<u8> for SensorLocation {
    fn from(byte: u8) -> Self {
        match byte {
            0b_1_0010_u8 => SensorLocation::Indoor,
            0b_0_1110_u8 => SensorLocation::Outdoor,
            _ => unreachable!()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NoiseFloorThreshold(pub u8);

impl NoiseFloorThreshold {
    pub fn new(value: u8) -> ::std::result::Result<Self, &'static str> {
        if value > 11 {
            return Err("Noise level threshold must be in range 0-11");
        }

        Ok(Self(value))
    }
}

impl Into<u8> for NoiseFloorThreshold {
    fn into(self) -> u8 {
        self.0
    }
}

impl From<u8> for NoiseFloorThreshold {
    fn from(v: u8) -> Self {
        Self::new(v).unwrap()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PowerDownStatus {
    /// running
    On,
    /// off
    Off,
}

impl From<u8> for PowerDownStatus {
    fn from(byte: u8) -> Self {
        match byte {
            0b0 => Self::On,
            0b1 => Self::Off,
            _ => unreachable!()
        }
    }
}

impl Into<u8> for PowerDownStatus {
    fn into(self) -> u8 {
        match self {
            Self::Off => 0b1,
            Self::On => 0b0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PresetDefaultCmd;

impl From<u8> for PresetDefaultCmd {
    fn from(_: u8) -> Self {
        unreachable!()
    }
}

impl Into<u8> for PresetDefaultCmd {
    fn into(self) -> u8 {
        0x96 // this is the value in the rpi library, so it probably does something special? idk
    }
}

/// Larger values correspond to more robust disturber rejection, with a decrease of the detection efficiency,
/// Refer to Figure 20 in the datasheet for the relationship between this threshold and its impact.
/// Defaults to 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SignalVerificationThreshold(pub u8);

impl SignalVerificationThreshold {
    pub fn new(value: u8) -> ::std::result::Result<Self, &'static str> {
        if value > 10 {
            return Err("Signal verification threshold must be in range 0-10");
        }

        Ok(Self(value))
    }
}

impl From<u8> for SignalVerificationThreshold {
    fn from(byte: u8) -> Self {
        Self::new(byte).unwrap()
    }
}

impl Into<u8> for SignalVerificationThreshold {
    fn into(self) -> u8 {
        self.0
    }
}

/// increases rejection of events that are likely man-made and not lightning
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpikeRejectionSetting(pub u8);

impl SpikeRejectionSetting {
    pub fn new(value: u8) -> ::std::result::Result<Self, &'static str> {
        if value > 0b00001111 {
            return Err("Spike rejection value must be in range 0-15");
        }

        Ok(Self(value))
    }
}

impl From<u8> for SpikeRejectionSetting {
    fn from(byte: u8) -> Self {
        Self::new(byte).unwrap()
    }
}

impl Into<u8> for SpikeRejectionSetting {
    fn into(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MinimumLightningThreshold {
    One,
    Five,
    Nine,
    Sixteen,
}

impl From<u8> for MinimumLightningThreshold {
    fn from(byte: u8) -> Self {
        match byte {
            0b00 => Self::One,
            0b01 => Self::Five,
            0b10 => Self::Nine,
            0b11 => Self::Sixteen,
            _ => unreachable!()
        }
    }
}

impl Into<u8> for MinimumLightningThreshold {
    fn into(self) -> u8 {
        match self {
            Self::One => 0b00,
            Self::Five => 0b01,
            Self::Nine => 0b10,
            Self::Sixteen => 0b11,
        }
    }
}

/// disable disturber events
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct  MaskDisturberEvent(pub bool);

impl From<u8> for MaskDisturberEvent {
    fn from(byte: u8) -> Self {
        match byte {
            0b0 => Self(false),
            0b1 => Self(true),
            _ => unreachable!()
        }
    }
}

impl Into<u8> for MaskDisturberEvent {
    fn into(self) -> u8 {
        self.0 as u8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TuningCapacitorValue(pub u8);

impl TuningCapacitorValue {
    pub fn new(value: u8) -> ::std::result::Result<Self, &'static str> {
        if value > 0b00001111 {
            return Err("Tunig cap value must be in range 0-15");
        }

        Ok(Self(value))
    }
}

impl From<u8> for TuningCapacitorValue {
    fn from(byte: u8) -> Self {
        Self::new(byte).unwrap()
    }
}

impl Into<u8> for TuningCapacitorValue {
    fn into(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrequencyDivisionRatio {
    R16,
    R32,
    R64,
    R128,
}

impl From<u8> for FrequencyDivisionRatio {
    fn from(byte: u8) -> Self {
        match byte {
            0b00 => Self::R16,
            0b01 => Self::R32,
            0b10 => Self::R64,
            0b11 => Self::R128,
            _ => unreachable!()
        }
    }
}

impl Into<u8> for FrequencyDivisionRatio {
    fn into(self) -> u8 {
        match self {
            Self::R16 => 0b00,
            Self::R32 => 0b01,
            Self::R64 => 0b10,
            Self::R128 => 0b11,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DistanceEstimate {
    OutOfRange,
    InRange(u8),
    Overhead,
}

impl From<u8> for DistanceEstimate {
    fn from(byte: u8) -> Self {
        match byte {
            0b111111 => Self::OutOfRange,
            0b000001 => Self::Overhead,
            dist => Self::InRange(dist)
        }
    }
}

impl Into<u8> for DistanceEstimate {
    fn into(self) -> u8 {
        unreachable!()
    }
}

/// output TRCO clock signal on the IRQ pin
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct  OutputTRCOOnIRQ(pub bool);

impl From<u8> for OutputTRCOOnIRQ {
    fn from(byte: u8) -> Self {
        match byte {
            0b0 => Self(false),
            0b1 => Self(true),
            _ => unreachable!()
        }
    }
}

impl Into<u8> for OutputTRCOOnIRQ {
    fn into(self) -> u8 {
        self.0 as u8
    }
}

/// output SRCO clock signal on the IRQ pin
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct  OutputSRCOOnIRQ(pub bool);

impl From<u8> for OutputSRCOOnIRQ {
    fn from(byte: u8) -> Self {
        match byte {
            0b0 => Self(false),
            0b1 => Self(true),
            _ => unreachable!()
        }
    }
}

impl Into<u8> for OutputSRCOOnIRQ {
    fn into(self) -> u8 {
        self.0 as u8
    }
}


/// output LCO clock signal on the IRQ pin
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct  OutputLCOOnIRQ(pub bool);

impl From<u8> for OutputLCOOnIRQ {
    fn from(byte: u8) -> Self {
        match byte {
            0b0 => Self(false),
            0b1 => Self(true),
            _ => unreachable!()
        }
    }
}

impl Into<u8> for OutputLCOOnIRQ {
    fn into(self) -> u8 {
        self.0 as u8
    }
}

/// output LCO clock signal on the IRQ pin
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct  SetClearStatistics(pub bool);

impl From<u8> for SetClearStatistics {
    fn from(byte: u8) -> Self {
        match byte {
            0b0 => Self(false),
            0b1 => Self(true),
            _ => unreachable!()
        }
    }
}

impl Into<u8> for SetClearStatistics {
    fn into(self) -> u8 {
        self.0 as u8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CalibrateOscilatorsCmd;

impl From<u8> for CalibrateOscilatorsCmd {
    fn from(_: u8) -> Self {
        unreachable!()
    }
}

impl Into<u8> for CalibrateOscilatorsCmd {
    fn into(self) -> u8 {
        0x96 // this is the value in the rpi library, so it probably does something special? idk
    }
}
