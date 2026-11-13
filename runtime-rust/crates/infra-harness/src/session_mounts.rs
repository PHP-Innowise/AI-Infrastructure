//! Filesystem coordinates for conservative overlap checks across Linux bind mounts.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MountScope {
    filesystem: (u32, u32),
    coordinate: PathBuf,
}
impl MountScope {
    pub(crate) fn overlaps(&self, other: &Self) -> bool {
        self.filesystem == other.filesystem
            && (self.coordinate.starts_with(&other.coordinate)
                || other.coordinate.starts_with(&self.coordinate))
    }
}

pub(crate) fn scopes(paths: &[&Path]) -> crate::Result<Vec<MountScope>> {
    #[cfg(target_os = "linux")]
    {
        linux::scopes(paths)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = paths;
        Ok(vec![])
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::{HarnessError, Result};
    use std::{
        collections::BTreeSet, ffi::OsString, fs::File, io::Read, os::unix::ffi::OsStringExt,
    };

    const MAX_MOUNTINFO: usize = 4 * 1024 * 1024;

    struct Mount {
        filesystem: (u32, u32),
        root: Option<PathBuf>,
        point: PathBuf,
    }
    fn invalid() -> HarnessError {
        HarnessError::conflict("The session mount identity is unavailable or ambiguous.")
    }
    fn number(raw: &[u8]) -> Result<u32> {
        if raw.is_empty() || !raw.iter().all(u8::is_ascii_digit) {
            return Err(invalid());
        }
        raw.iter().try_fold(0u32, |value, byte| {
            value
                .checked_mul(10)
                .and_then(|value| value.checked_add(u32::from(byte - b'0')))
                .ok_or_else(invalid)
        })
    }
    fn normal(raw: &[u8]) -> bool {
        raw.first() == Some(&b'/')
            && !raw.contains(&0)
            && (raw == b"/"
                || raw[1..]
                    .split(|byte| *byte == b'/')
                    .all(|part| !part.is_empty() && part != b"." && part != b".."))
    }
    fn path(raw: &[u8]) -> Result<PathBuf> {
        let mut decoded = Vec::with_capacity(raw.len());
        let mut cursor = 0;
        while cursor < raw.len() {
            if raw[cursor] == b'\\' {
                let escaped = raw.get(cursor + 1..cursor + 4).ok_or_else(invalid)?;
                decoded.push(match escaped {
                    b"040" => b' ',
                    b"011" => b'\t',
                    b"012" => b'\n',
                    b"134" => b'\\',
                    _ => return Err(invalid()),
                });
                cursor += 4;
            } else {
                decoded.push(raw[cursor]);
                cursor += 1;
            }
        }
        if !normal(&decoded) {
            return Err(invalid());
        }
        Ok(PathBuf::from(OsString::from_vec(decoded)))
    }
    // Kernel field definitions: docs.kernel.org/filesystems/proc.html, section 3.5.
    // Unknown optional fields are deliberately ignored, as the format requires.
    fn parse(raw: &[u8], paths: &[&Path]) -> Result<Vec<Mount>> {
        if raw.is_empty() || raw.len() > MAX_MOUNTINFO || !raw.ends_with(b"\n") {
            return Err(invalid());
        }
        let mut mounts = Vec::new();
        let mut ids = BTreeSet::new();
        for line in raw[..raw.len() - 1].split(|byte| *byte == b'\n') {
            if line.is_empty() || line.iter().any(|byte| matches!(*byte, 0 | b'\t' | b'\r')) {
                return Err(invalid());
            }
            let fields = line.split(|byte| *byte == b' ').collect::<Vec<_>>();
            // The mount point is the fifth field; the kernel never prints the
            // four before it empty. Only mounts that cover or lie inside a
            // declared tree can affect its coordinates, so relevance is decided
            // before the rest of the line is read: an unrelated line need not
            // be interpretable. A replaced bind source has an opaque //deleted
            // root, and a mount made with an empty source prints an empty field.
            let point = path(fields.get(4).ok_or_else(invalid)?)?;
            if !paths
                .iter()
                .any(|root| root.starts_with(&point) || point.starts_with(root))
            {
                continue;
            }
            if fields.len() < 10 || fields.iter().any(|field| field.is_empty()) {
                return Err(invalid());
            }
            let separator = fields[6..]
                .iter()
                .position(|field| *field == b"-")
                .map(|index| index + 6)
                .ok_or_else(invalid)?;
            if fields.len() != separator + 4 {
                return Err(invalid());
            }
            let id = number(fields[0])?;
            if id == 0 || number(fields[1])? == 0 || !ids.insert(id) {
                return Err(invalid());
            }
            let device = fields[2].split(|byte| *byte == b':').collect::<Vec<_>>();
            if device.len() != 2 {
                return Err(invalid());
            }
            mounts.push(Mount {
                filesystem: (number(device[0])?, number(device[1])?),
                root: if fields[3].starts_with(b"/") {
                    Some(path(fields[3])?)
                } else if fields[separator + 1] == b"nsfs" {
                    // Namespace handles have an opaque root such as net:[...].
                    None
                } else {
                    return Err(invalid());
                },
                point,
            });
        }
        Ok(mounts)
    }
    fn locate(path: &Path, mounts: &[Mount]) -> Result<MountScope> {
        use std::os::unix::ffi::OsStrExt;
        if !normal(path.as_os_str().as_bytes()) {
            return Err(invalid());
        }
        let mut selected: Option<&Mount> = None;
        let mut depth = 0;
        let mut ambiguous = false;
        for mount in mounts.iter().filter(|mount| path.starts_with(&mount.point)) {
            let candidate = mount.point.components().count();
            if candidate > depth {
                selected = Some(mount);
                depth = candidate;
                ambiguous = false;
            } else if candidate == depth {
                // Do not guess which stacked mount is visible, even when the
                // entries happen to describe the same filesystem coordinates.
                ambiguous = true;
            }
        }
        let mount = selected.ok_or_else(invalid)?;
        if ambiguous {
            return Err(invalid());
        }
        let relative = path.strip_prefix(&mount.point).map_err(|_| invalid())?;
        Ok(MountScope {
            filesystem: mount.filesystem,
            coordinate: mount.root.as_ref().ok_or_else(invalid)?.join(relative),
        })
    }
    fn trees(
        paths: &[&Path],
        mounts: &[Mount],
        mut verify_directory: impl FnMut(&Path) -> Result<()>,
    ) -> Result<Vec<MountScope>> {
        let mut result = Vec::new();
        for path in paths {
            verify_directory(path)?;
            result.push(locate(path, mounts)?);
            let mut points = BTreeSet::new();
            for mount in mounts
                .iter()
                .filter(|mount| mount.point.as_path() != *path && mount.point.starts_with(path))
            {
                // A declared tree includes filesystems mounted below its root.
                // Duplicate points can be stacked/hidden: never choose one by order.
                if !points.insert(&mount.point) {
                    return Err(invalid());
                }
                verify_directory(&mount.point)?;
                result.push(MountScope {
                    filesystem: mount.filesystem,
                    coordinate: mount.root.clone().ok_or_else(invalid)?,
                });
            }
        }
        Ok(result)
    }
    pub(super) fn scopes(paths: &[&Path]) -> Result<Vec<MountScope>> {
        let mut raw = Vec::new();
        File::open("/proc/self/mountinfo")?
            .take((MAX_MOUNTINFO + 1) as u64)
            .read_to_end(&mut raw)?;
        trees(paths, &parse(&raw, paths)?, |path| {
            if std::fs::canonicalize(path)?.as_path() != path {
                return Err(invalid());
            }
            let metadata = std::fs::symlink_metadata(path)?;
            if !metadata.is_dir() {
                return Err(invalid());
            }
            // Btrfs st_dev can identify a subvolume and differ from mountinfo.
            // The kernel mount table supplies the filesystem coordinate domain.
            Ok(())
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        const ROOT: &str = "1 1 8:1 / / rw - ext4 /dev/root rw\n";
        fn parse(raw: &[u8]) -> Result<Vec<Mount>> {
            super::parse(raw, &[Path::new("/")])
        }
        fn at(mounts: &[Mount], path: &str) -> MountScope {
            locate(Path::new(path), mounts).unwrap()
        }
        #[test]
        fn ordinary_neighboring_roots_do_not_overlap() {
            let mounts = parse(ROOT.as_bytes()).unwrap();
            let a = at(&mounts, "/repos/a");
            assert!(!a.overlaps(&at(&mounts, "/repos/b")));
            assert!(!a.overlaps(&at(&mounts, "/repos/ab")));
            assert!(a.overlaps(&at(&mounts, "/repos/a/nested")));
            assert!(a.overlaps(&at(&mounts, "/repos/a")));
        }
        #[test]
        fn namespace_handles_outside_declared_trees_do_not_disable_parallelism() {
            let text =
                format!("{ROOT}2 1 0:4 net:[4026533374] /run/netns/fixture rw - nsfs nsfs rw\n");
            let mounts = parse(text.as_bytes()).unwrap();
            assert!(trees(&[Path::new("/repos/a")], &mounts, |_| Ok(())).is_ok());
            assert!(trees(&[Path::new("/run")], &mounts, |_| Ok(())).is_err());
        }
        #[test]
        fn deleted_bind_roots_are_rejected_only_when_their_points_affect_a_declared_tree() {
            let paths = [
                Path::new("/repos/a"),
                Path::new("/git/common"),
                Path::new("/git/admin"),
            ];
            let expected = trees(
                &paths,
                &super::parse(ROOT.as_bytes(), &paths).unwrap(),
                |_| Ok(()),
            )
            .unwrap();
            for point in ["/outside/resolv.conf", "/repos/ab/config"] {
                let text =
                    format!("{ROOT}2 1 8:1 /old/source//deleted {point} rw - ext4 /dev/root rw\n");
                let mounts = super::parse(text.as_bytes(), &paths).unwrap();
                assert_eq!(trees(&paths, &mounts, |_| Ok(())).unwrap(), expected);
            }
            for point in [
                "/repos",
                "/repos/a/config",
                "/git/common/config",
                "/git/admin",
            ] {
                let text =
                    format!("{ROOT}2 1 8:1 /old/source//deleted {point} rw - ext4 /dev/root rw\n");
                assert!(super::parse(text.as_bytes(), &paths).is_err(), "{point}");
            }
        }
        #[test]
        fn empty_source_fields_are_rejected_only_when_their_points_affect_a_declared_tree() {
            // `mount -t tmpfs "" <point>` prints its empty source as an empty field.
            let paths = [Path::new("/repos/a"), Path::new("/git/common")];
            let expected = trees(
                &paths,
                &super::parse(ROOT.as_bytes(), &paths).unwrap(),
                |_| Ok(()),
            )
            .unwrap();
            for point in ["/outside/scratch", "/repos/ab"] {
                let text = format!("{ROOT}2 1 0:7 / {point} rw - tmpfs  rw\n");
                let mounts = super::parse(text.as_bytes(), &paths).unwrap();
                assert_eq!(trees(&paths, &mounts, |_| Ok(())).unwrap(), expected);
            }
            for point in ["/repos", "/repos/a/cache", "/git/common"] {
                let text = format!("{ROOT}2 1 0:7 / {point} rw - tmpfs  rw\n");
                assert!(super::parse(text.as_bytes(), &paths).is_err(), "{point}");
            }
        }
        #[test]
        fn current_namespace_accepts_an_actual_disposable_directory() {
            let temporary = tempfile::tempdir().unwrap();
            let directory = std::fs::canonicalize(temporary.path()).unwrap();
            assert!(
                !super::super::scopes(&[directory.as_path()])
                    .unwrap()
                    .is_empty()
            );
        }
        #[test]
        fn exact_and_nested_bind_aliases_share_filesystem_coordinates() {
            let text = format!(
                "{ROOT}2 1 8:1 /repos/a /aliases/a rw shared:7 future:field - ext4 /dev/root rw\n"
            );
            let mounts = parse(text.as_bytes()).unwrap();
            let original = at(&mounts, "/repos/a");
            let alias = at(&mounts, "/aliases/a");
            let nested = at(&mounts, "/aliases/a/subrepository");
            assert_eq!(original, alias);
            assert!(original.overlaps(&nested));
            assert!(nested.overlaps(&original));
            assert!(!at(&mounts, "/repos/b").overlaps(&nested));
        }
        #[test]
        fn non_root_filesystem_roots_and_deepest_mounts_are_preserved() {
            let text = "1 1 8:1 /@ / rw - btrfs /dev/root rw\n2 1 8:1 /@/repos/a /aliases/a rw - btrfs /dev/root rw\n3 1 8:2 / /repos/a/other rw - ext4 /dev/other rw\n";
            let mounts = parse(text.as_bytes()).unwrap();
            assert_eq!(at(&mounts, "/repos/a"), at(&mounts, "/aliases/a"));
            let other = locate(Path::new("/repos/a/other/project"), &mounts).unwrap();
            assert!(!other.overlaps(&at(&mounts, "/repos/a")));
            assert_eq!(other.coordinate, Path::new("/project"));
        }
        #[test]
        fn declared_trees_include_descendant_mounts_exposing_another_repository() {
            let text = format!(
                "{ROOT}2 1 8:2 /other-repository /repos/a/mounted rw - ext4 /dev/other rw\n3 1 8:2 / /external rw - ext4 /dev/other rw\n"
            );
            let mounts = parse(text.as_bytes()).unwrap();
            let roots = trees(&[Path::new("/repos/a")], &mounts, |_| Ok(())).unwrap();
            let external =
                locate(Path::new("/external/other-repository/subrepo"), &mounts).unwrap();
            assert!(roots.iter().any(|root| root.overlaps(&external)));
            assert!(trees(&[Path::new("/repos/a")], &mounts, |_| Err(invalid())).is_err());
            let stacked =
                format!("{text}4 1 8:2 /different /repos/a/mounted rw - ext4 /dev/other rw\n");
            assert!(
                trees(
                    &[Path::new("/repos/a")],
                    &parse(stacked.as_bytes()).unwrap(),
                    |_| Ok(())
                )
                .is_err()
            );
        }
        #[test]
        fn escaped_paths_are_decoded_before_coordinate_matching() {
            let text = format!(
                "{ROOT}2 1 8:1 /repos/a\\040b\\134c\\011d\\012e /aliases/a\\040b rw - ext4 /dev/root rw\n"
            );
            let mounts = parse(text.as_bytes()).unwrap();
            assert_eq!(
                at(&mounts, "/aliases/a b/child"),
                at(&mounts, "/repos/a b\\c\td\ne/child")
            );
        }
        #[test]
        fn malformed_mountinfo_and_paths_fail_closed() {
            for invalid_text in [
                "",
                "1 1 8:1 / / rw - ext4 /dev/root rw",
                "1 1 8:1 / / rw ext4 /dev/root rw\n",
                "1 1 8:1 / / rw - ext4 /dev/root\n",
                "1 1 8:1 / / rw - ext4 /dev/root rw extra\n",
                "1 1 8:1 relative / rw - ext4 /dev/root rw\n",
                "1 1 8:1 / /a/../b rw - ext4 /dev/root rw\n",
                "1 1 8:1 / /a//b rw - ext4 /dev/root rw\n",
                "1 1 8:1 / /a/ rw - ext4 /dev/root rw\n",
                "1 1 8:1 / /a\\057b rw - ext4 /dev/root rw\n",
                "1 1 8:1 / /a\\000b rw - ext4 /dev/root rw\n",
                "1 1 8:1 / /a\\040 rw - ext4 /dev/root rw\n\n",
                "1 1 8:1:2 / / rw - ext4 /dev/root rw\n",
                "1 1 8:x / / rw - ext4 /dev/root rw\n",
                "1 1 4294967296:1 / / rw - ext4 /dev/root rw\n",
                "0 1 8:1 / / rw - ext4 /dev/root rw\n",
                "1 0 8:1 / / rw - ext4 /dev/root rw\n",
                "1 1 8:1 / /  rw - ext4 /dev/root rw\n",
            ] {
                assert!(parse(invalid_text.as_bytes()).is_err(), "{invalid_text:?}");
            }
            assert!(parse(format!("{ROOT}{ROOT}").as_bytes()).is_err());
            assert!(parse(&vec![b'x'; MAX_MOUNTINFO + 1]).is_err());
            let mounts = parse(ROOT.as_bytes()).unwrap();
            for invalid_path in ["relative", "/a/../b", "/a/./b", "/a//b", "/a/", "/a\0b"] {
                assert!(locate(Path::new(invalid_path), &mounts).is_err());
            }
        }
        #[test]
        fn stacked_or_unmapped_mounts_fail_closed() {
            let text = format!(
                "{ROOT}2 1 8:1 /one /alias rw - ext4 /dev/root rw\n3 1 8:1 /two /alias rw - ext4 /dev/root rw\n"
            );
            let mounts = parse(text.as_bytes()).unwrap();
            assert!(locate(Path::new("/alias/project"), &mounts).is_err());
            let mounts = parse(b"2 1 8:1 /one /alias rw - ext4 /dev/root rw\n").unwrap();
            assert!(locate(Path::new("/elsewhere"), &mounts).is_err());
        }
    }
}
