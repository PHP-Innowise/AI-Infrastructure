//! Copies of an executable a test then starts. A test binary runs its cases
//! on threads of one process, and a child one case starts inherits every
//! descriptor open in that process until it execs. A case still writing its
//! copy of the engine (130 MB in a debug build) while another starts a
//! process leaves that child holding the copy open for writing, and starting
//! the copy then fails with ETXTBSY ("Text file busy"). A copy written by a
//! process of its own leaves no write descriptor here to inherit.
use std::path::Path;

/// Copies `from` to `to` with `from`'s permissions, as `fs::copy` does.
pub fn copy(from: impl AsRef<Path>, to: impl AsRef<Path>) {
    let (from, to) = (from.as_ref(), to.as_ref());
    #[cfg(unix)]
    {
        let status = std::process::Command::new("cp")
            .arg("--")
            .arg(from)
            .arg(to)
            .status()
            .unwrap_or_else(|error| panic!("start cp: {error}"));
        assert!(status.success(), "cp {} {}", from.display(), to.display());
        std::fs::set_permissions(to, std::fs::metadata(from).unwrap().permissions()).unwrap();
    }
    // Handles are not inherited unless asked for, so a plain copy is safe.
    #[cfg(not(unix))]
    {
        std::fs::copy(from, to).unwrap();
    }
}
