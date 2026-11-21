//! A temporary folder for tests that write sealed packages.

/// A temporary folder that is removed although it held sealed packages: a
/// sealed package's folders are read-only, and a plain temporary folder would
/// be left behind on every run.
pub struct Scratch(tempfile::TempDir);

impl Scratch {
    pub fn path(&self) -> &std::path::Path {
        self.0.path()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        infra_skillcache::package::release(self.0.path());
    }
}

pub fn scratch() -> Scratch {
    Scratch(tempfile::tempdir().unwrap())
}
