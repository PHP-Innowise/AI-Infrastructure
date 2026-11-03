use infra_core::{Result, fs as safe};
use std::path::{Path, PathBuf};
pub struct Temporary {
    path: PathBuf,
    keep: bool,
}
impl Temporary {
    pub fn new(prefix: &str) -> Result<Self> {
        let path = std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4().simple()));
        safe::ensure_directory(&path, true)?;
        infra_core::security::secure_private_dir(&path)?;
        Ok(Self { path, keep: false })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn preserve(&mut self) -> PathBuf {
        self.keep = true;
        self.path.clone()
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
