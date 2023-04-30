#[macro_use] extern crate serde;

pub use squirrel;
pub use squirrel::api::station;

// temporary API for sending updates of the latest readings
// final version will not use hardcoded fields
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatestReadings {
    pub temperature: f32,
    pub humidity: f32,
    pub pressure: f32,
    pub battery: f32,
}

