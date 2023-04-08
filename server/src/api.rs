use serde::{Deserialize, Serialize};
use zerocopy::{AsBytes, FromBytes};

//TODO add checksums
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DataPacket {
    id: u32,
    observations: Observations,
}

const REQUEST_PACKET_MAGIC: u32 = 0x3ce9abc2;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RequestPacket {
    // so random other packets are ignored
    magic: u32,
    // echoed back in the data packet
    id: u32,
}

impl RequestPacket {
    pub fn validate(&self) -> bool {
        self.magic == REQUEST_PACKET_MAGIC
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, FromBytes, AsBytes)]
#[repr(C)]
pub struct Observations {
    /// degrees c
    temperature: f32,
    /// relative humidity (precentage)
    humidity: f32,
    /// pressure (pascals)
    pressure: f32,
    /// battery voltage (volts)
    battery: f32,
}
