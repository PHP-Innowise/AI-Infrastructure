//! Canonical paths in the spelling other programs read. Std only: the QA
//! crate includes this file without depending on infra-core.
use std::path::{Path, PathBuf};

/// `std::fs::canonicalize` in a spelling every program reads. On Windows the
/// standard result always carries the `\\?\` prefix, which Git, a process's
/// working directory and a later `join("a/b")` cannot use; Python's
/// `realpath`, whose results earlier installations stored, drops it as well.
pub fn canonicalize(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    let resolved = std::fs::canonicalize(path)?;
    #[cfg(windows)]
    if let Some(plain) = resolved.to_str().and_then(plain_windows_path) {
        return Ok(PathBuf::from(plain));
    }
    Ok(resolved)
}

/// The plain spelling of a verbatim Windows drive path, when Win32 reads it
/// as the same file: shorter than `MAX_PATH`, with no component it would
/// rewrite or refuse (empty, `.`, `..`, a trailing dot or space, a reserved
/// device name, a forbidden character). Every other form stays verbatim.
pub fn plain_windows_path(path: &str) -> Option<&str> {
    let plain = path.strip_prefix(r"\\?\")?;
    let drive = plain.as_bytes();
    if drive.len() < 3
        || !drive[0].is_ascii_alphabetic()
        || &drive[1..3] != b":\\"
        || plain.encode_utf16().count() >= 260
    {
        return None;
    }
    let rest = &plain[3..];
    (rest.is_empty() || rest.split('\\').all(plain_windows_component)).then_some(plain)
}

fn plain_windows_component(name: &str) -> bool {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.ends_with(['.', ' '])
        || name.chars().any(|c| c < ' ' || "<>:\"/|?*".contains(c))
    {
        return false;
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    let numbered = ["COM", "LPT"].iter().any(|device| {
        stem.strip_prefix(device).is_some_and(|number| {
            let mut digits = number.chars();
            digits
                .next()
                .is_some_and(|digit| digit.is_ascii_digit() || "¹²³".contains(digit))
                && digits.next().is_none()
        })
    });
    !numbered && !["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem.as_str())
}

#[cfg(test)]
mod windows_spelling_tests {
    use super::plain_windows_path;

    #[test]
    fn verbatim_drive_paths_lose_the_prefix_only_where_win32_reads_the_same_file() {
        for (verbatim, plain) in [
            (r"\\?\C:\", r"C:\"),
            (
                r"\\?\c:\Users\runneradmin\Temp",
                r"c:\Users\runneradmin\Temp",
            ),
            (
                r"\\?\D:\a\AI-Infrastructure\linked worktree",
                r"D:\a\AI-Infrastructure\linked worktree",
            ),
            (r"\\?\C:\dir\COM10\console.txt", r"C:\dir\COM10\console.txt"),
            (r"\\?\C:\dir\.hidden\ünïcode", r"C:\dir\.hidden\ünïcode"),
        ] {
            assert_eq!(plain_windows_path(verbatim), Some(plain), "{verbatim}");
        }
        let long = format!(r"\\?\C:\{}", "a".repeat(260));
        for kept in [
            r"C:\already\plain",
            r"\\?\UNC\server\share\file",
            r"\\?\Volume{fixture}\file",
            r"\\?\C:",
            r"\\?\C:relative",
            r"\\?\1:\digit",
            r"\\?\C:\dir\NUL",
            r"\\?\C:\dir\nul.txt",
            r"\\?\C:\dir\Com1",
            r"\\?\C:\dir\lpt²",
            r"\\?\C:\dir\aux .log",
            r"\\?\C:\dir\CONIN$",
            r"\\?\C:\dir\trailing.",
            r"\\?\C:\dir\trailing ",
            r"\\?\C:\dir\..\up",
            r"\\?\C:\dir\.\here",
            r"\\?\C:\dir\\empty",
            r"\\?\C:\dir\a?b",
            r"\\?\C:\dir\a/b",
            long.as_str(),
        ] {
            assert_eq!(plain_windows_path(kept), None, "{kept}");
        }
        let longest = format!(r"\\?\C:\{}", "a".repeat(255));
        assert_eq!(plain_windows_path(&longest).map(str::len), Some(258));
    }
}
