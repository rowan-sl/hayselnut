use std::path::{Path, PathBuf};

use anyhow::Result;

/// Utility struct for generating paths inside the records directory
#[derive(Debug, Clone)]
pub struct RecordsPath {
    records_dir: PathBuf,
}

impl RecordsPath {
    pub fn new(path: PathBuf) -> Self {
        Self { records_dir: path }
    }

    pub fn ensure_exists_blocking(&self) -> Result<()> {
        if self.records_dir.exists() {
            if !self.records_dir.canonicalize()?.is_dir() {
                error!("records directory path already exists, and is a file!");
                bail!("records dir exists");
            }
        } else {
            info!("Creating new records directory at {:#?}", self.records_dir);
            std::fs::create_dir(self.records_dir.clone())?;
        }
        Ok(())
    }

    /// Returns the path with the requested file extension.
    /// does not allow for nesting in subdirectories
    ///
    /// # Panics
    /// - if PathBuf does not have a final (file) component, eg the path `foo/..` or `/`
    #[instrument(skip(filename))]
    pub fn path<P: AsRef<Path>>(&self, filename: P) -> PathBuf {
        let p = filename.as_ref();
        if p.parent().is_some() && p.parent() != Some(Path::new("")) {
            warn!(path=?p, "RecordsPath::path only uses the last segment of a path, the rest will be discarded");
        }
        if let Some(file) = p.file_name() {
            self.records_dir.join(file)
        } else {
            panic!("Invalid filename passed to `RecordsPath::path` (path does not contain a final component)");
        }
    }
}
