use std::path::PathBuf;

use anyhow::Result;
use serde::Deserialize;

#[cfg(test)]
#[test]
fn load_example_config() {
    let settings = config::Config::builder()
        .add_source(config::File::from_str(
            include_str!("../config.example.toml"),
            config::FileFormat::Toml,
        ))
        .build()
        .unwrap();

    // Print out our settings (as a HashMap)
    println!("{:?}", settings.try_deserialize::<Config>().unwrap());
}

pub async fn open(path: PathBuf) -> Result<self::Config> {
    let config_file = tokio::fs::read_to_string(path).await?;
    let settings = config::Config::builder()
        .add_source(config::File::from_str(
            &config_file,
            config::FileFormat::Toml,
        ))
        .build()?
        .try_deserialize()?;
    Ok(settings)
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Config {
    /// directories to store various things
    pub directory: Directories,
    /// meta server info (url, port)
    pub server: Server,
    /// database configuration
    pub database: Database,
    /// misc
    pub misc: Misc,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Directories {
    /// the directory to store persistant data (e.g. the
    /// station / channel registry, and the default DB location)
    pub data: PathBuf,
    /// the directory to store runtime information (must be
    /// able to delete this *between* server runs, with no consequence)
    ///
    /// e.g. log files, daemon PID files, IPC socket
    pub run: PathBuf,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Server {
    /// the URL of the server this is running on
    pub url: String,
    /// the port to run the server
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Database {
    /// storage mode of the database (single file vs RAID)
    pub storage: StorageMode,
    /// file(s) to use as backing.
    /// not necessary to provide if `StorageMode::DefaultFile` is selected
    #[serde(default)]
    pub files: Vec<File>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub enum StorageMode {
    /// a single file, automatically created inside the data directory
    #[allow(non_camel_case_types)]
    default,
    /// a single file (explicitly specified)
    #[allow(non_camel_case_types)]
    file,
    /// multiple files in raid zero (usefull with block devices)
    #[allow(non_camel_case_types)]
    raid,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct File {
    pub path: PathBuf,
    #[serde(default)]
    pub blockdevice: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Misc {
    /// script to run before starting (e.g. to setup permissions for block devices used in RAID)
    #[serde(default)]
    pub init_script: PathBuf,
}
