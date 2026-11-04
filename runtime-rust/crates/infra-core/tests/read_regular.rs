//! The admitted-source reader preserves bytes and rejects non-regular names.
#![cfg(any(unix, windows))]

use infra_core::{ErrorKind, domain, fs::read_regular};
use std::{fs, path::PathBuf};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("infra-read-regular-{}", domain::uuid4()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn ordinary_reads_preserve_empty_binary_and_text_bytes_without_retaining_an_old_image() {
    let fixture = Fixture::new();
    let path = fixture.0.join("source.bin");
    for expected in [
        Vec::new(),
        vec![0, 0xff, b'\r', b'\n', 0xc0, 0xaf, 0],
        "Строка\r\nwith original line endings\r".as_bytes().to_vec(),
        (0..6731).map(|index| (index % 256) as u8).collect(),
        vec![0xa5; 64 * 1024],
    ] {
        fs::write(&path, &expected).unwrap();
        assert_eq!(read_regular(&path).unwrap(), expected);
        assert_eq!(read_regular(&path).unwrap(), expected);
    }
    fs::write(&path, b"old\0").unwrap();
    assert_eq!(read_regular(&path).unwrap(), b"old\0");
    fs::write(&path, b"new\xff").unwrap();
    assert_eq!(read_regular(&path).unwrap(), b"new\xff");
}

#[test]
fn directories_are_policy_refusals_and_missing_sources_remain_not_found() {
    let fixture = Fixture::new();
    let error = read_regular(&fixture.0).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Policy);
    assert_eq!(error.message, "Source is linked or not a regular file");
    let error = read_regular(&fixture.0.join("missing")).unwrap_err();
    assert_eq!(error.kind, ErrorKind::NotFound);
}

#[cfg(unix)]
#[test]
fn symlinks_are_refused_while_ordinary_hardlinks_remain_readable() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let source = fixture.0.join("source");
    let linked = fixture.0.join("symbolic");
    let hardlink = fixture.0.join("hardlink");
    fs::write(&source, b"held source bytes\0\xff").unwrap();
    symlink(&source, &linked).unwrap();
    let error = read_regular(&linked).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Policy);
    assert_eq!(error.message, "Source is linked or not a regular file");

    fs::hard_link(&source, &hardlink).unwrap();
    for path in [&source, &hardlink] {
        assert_eq!(read_regular(path).unwrap(), b"held source bytes\0\xff");
    }
    fs::remove_file(&source).unwrap();
    assert_eq!(read_regular(&hardlink).unwrap(), b"held source bytes\0\xff");
    let error = read_regular(&linked).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Policy);
}

#[cfg(unix)]
#[test]
fn a_fifo_without_a_writer_is_refused_as_non_regular() {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let fixture = Fixture::new();
    let fifo = fixture.0.join("source.fifo");
    let encoded = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(encoded.as_ptr(), 0o600) }, 0);
    let error = read_regular(&fifo).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Policy);
    assert_eq!(error.message, "Source is linked or not a regular file");
}
