use std::path::{Path, PathBuf};

/// Utility struct for generating paths inside the records directory
#[derive(Debug, Clone)]
pub struct RecordsPath {
    records_dir: PathBuf,
}

impl RecordsPath {
    pub fn new(path: PathBuf) -> Self {
        Self { records_dir: path }
    }
    /// Returns the path with the requested file extension.
    /// does not allow for nesting in subdirectories
    ///
    /// # Panics
    /// - if PathBuf does not have a final (file) component, eg the path `foo/..` or `/`
    #[instrument(skip(filename))]
    pub fn path<P: AsRef<Path>>(&self, filename: P) -> PathBuf {
        let p = filename.as_ref();
        if p.parent().is_some() {
            warn!(path=?p, "RecordsPath::path only uses the last segment of a path, the rest will be discarded");
        }
        if let Some(file) = p.file_name() {
            self.records_dir.join(file)
        } else {
            panic!("Invalid filename passed to `RecordsPath::path` (path does not contain a final component)");
        }
    }
}
