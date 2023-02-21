//TODO: figure this out better
// currently this is unused

use serde::{Serialize, Deserialize};
use semver::Version;

/// Weather station info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StationInfo {
    /// provides these readings
    pub prov: Vec<ReadingType>,
}

/// types of readings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReadingType {
    Battery
}

