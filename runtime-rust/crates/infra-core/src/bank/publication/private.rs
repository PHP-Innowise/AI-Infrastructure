//! Private attached-state operations through retained native directory handles.
use super::{Pin, PublicationFiles, native};
use crate::{Result, RuntimeError};
use std::{fs::File, path::Path};

#[cfg(test)]
#[path = "private_exclusive_tests.rs"]
mod private_exclusive_tests;

/// Which already-existing directory components the native root may tighten.
/// Every missing component is created privately under the same retained custody.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootProtection {
    /// Preserve the attached-state contract: the selected root and its managed
    /// parent named "attached" are private.
    AttachedState,
    /// Tighten only the selected owned root, preserving every existing ancestor.
    SelectedRoot,
    /// Preserve every existing directory's permissions, including the selected
    /// root. Only newly created components receive private permissions.
    CreatedDirectories,
}

/// A directory created exclusively by this copy, with its original parent and
/// native leaf retained. Mode changes act on this handle, never a pathname.
pub struct CreatedNativeDirectory {
    parent: std::sync::Arc<File>,
    name: std::ffi::OsString,
    file: std::sync::Arc<File>,
    identity: (u64, u64, u32),
}
impl CreatedNativeDirectory {
    pub fn check(&self) -> Result<()> {
        let named = native::directory_native(&self.parent, &self.name)?;
        if native::identity(&named)?.namespace != self.identity
            || native::identity(&self.file)?.namespace != self.identity
        {
            return Err(RuntimeError::conflict(
                "Created native directory custody changed",
            ));
        }
        Ok(())
    }
    pub fn create_private_child(&self, name: &std::ffi::OsStr) -> Result<Self> {
        native_copy_leaf(name)?;
        self.check()?;
        let file = native::private_create_directory_native(&self.file, name)?;
        crate::security::secure_private_handle(&file, true)?;
        let result = Self {
            parent: self.file.clone(),
            name: name.to_os_string(),
            identity: native::identity(&file)?.namespace,
            file: std::sync::Arc::new(file),
        };
        self.check()?;
        result.check()?;
        Ok(result)
    }
    pub fn set_mode(&mut self, mode: u32) -> Result<()> {
        if mode & !0o7777 != 0 {
            return Err(RuntimeError::invalid("Invalid native directory mode"));
        }
        self.check()?;
        let before = native::identity(&self.file)?.namespace;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            self.file
                .set_permissions(std::fs::Permissions::from_mode(mode))?;
        }
        #[cfg(windows)]
        {
            let mut permissions = self.file.metadata()?.permissions();
            permissions.set_readonly(mode & 0o200 == 0);
            self.file.set_permissions(permissions)?;
        }
        let after = native::identity(&self.file)?.namespace;
        if (after.0, after.1) != (before.0, before.1) {
            return Err(RuntimeError::conflict(
                "Created native directory identity changed",
            ));
        }
        self.identity = after;
        self.check()
    }
}
fn native_copy_leaf(name: &std::ffi::OsStr) -> Result<()> {
    let mut components = Path::new(name).components();
    if !matches!(components.next(), Some(std::path::Component::Normal(part)) if part == name)
        || components.next().is_some()
    {
        return Err(RuntimeError::invalid(
            "Native directory needs one ordinary leaf",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        if name.as_bytes().contains(&0) {
            return Err(RuntimeError::invalid("NUL in native directory leaf"));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        if name
            .encode_wide()
            .any(|value| [0, 47, 58, 92].contains(&value))
        {
            return Err(RuntimeError::invalid("Invalid native directory leaf"));
        }
    }
    Ok(())
}
impl PublicationFiles {
    /// Create a UTF-8 target leaf and retain its native mode capability.
    /// Descendants may carry native-only names; serialized names remain strict.
    pub fn create_private_native_directory_capability(
        &mut self,
        name: &std::ffi::OsStr,
    ) -> Result<CreatedNativeDirectory> {
        let identity = self.create_private_native_directory_exclusive(name)?;
        let name_text = name
            .to_str()
            .ok_or_else(|| RuntimeError::invalid("Native target is not UTF-8"))?;
        let key = vec![name_text.to_string()];
        let parent = self
            .directories
            .get(&Vec::<String>::new())
            .ok_or_else(|| RuntimeError::unavailable("Private parent unavailable"))?
            .try_clone()?;
        let file = self
            .directories
            .get(&key)
            .ok_or_else(|| RuntimeError::unavailable("Created private target unavailable"))?
            .try_clone()?;
        let result = CreatedNativeDirectory {
            parent: std::sync::Arc::new(parent),
            name: name.to_os_string(),
            file: std::sync::Arc::new(file),
            identity,
        };
        self.check()?;
        result.check()?;
        Ok(result)
    }
    /// Change only an exclusively created cached child, then refresh that same
    /// device/file's mode projection in this retained publication proof.
    pub fn set_created_native_directory_mode(
        &mut self,
        directory: &mut CreatedNativeDirectory,
        mode: u32,
    ) -> Result<()> {
        self.check()?;
        directory.check()?;
        let root = self
            .directories
            .get(&Vec::<String>::new())
            .ok_or_else(|| RuntimeError::unavailable("Private parent unavailable"))?;
        let parent = native::identity(&directory.parent)?.namespace;
        let wanted_parent = native::identity(root)?.namespace;
        if (parent.0, parent.1) != (wanted_parent.0, wanted_parent.1) {
            return Err(RuntimeError::conflict(
                "Mode capability belongs to another parent",
            ));
        }
        let name = directory
            .name
            .to_str()
            .ok_or_else(|| RuntimeError::invalid("Cached native target is not UTF-8"))?;
        let file = self
            .directories
            .get(&vec![name.to_string()])
            .ok_or_else(|| RuntimeError::conflict("Mode capability was not created here"))?;
        if native::identity(file)?.namespace != directory.identity {
            return Err(RuntimeError::conflict(
                "Created target mode capability changed",
            ));
        }
        let before = directory.identity;
        directory.set_mode(mode)?;
        self.refresh_private_namespace(&directory.file, before)?;
        self.check()
    }
    /// Create one private directory exclusively, with private security at creation.
    /// Existing names are refused without changing their permissions or contents.
    pub fn create_private_directory_exclusive(
        &mut self,
        relative: &str,
    ) -> Result<(u64, u64, u32)> {
        let (parent, name) = self.parent(relative, false)?;
        let key = relative.split('/').map(str::to_owned).collect::<Vec<_>>();
        self.create_private_child_exclusive(parent, name, key)
    }

    /// Create exactly one native UTF-8 leaf under the retained root.
    /// Unix backslash and colon are literal name bytes, never portable keys.
    pub fn create_private_native_directory_exclusive(
        &mut self,
        name: &std::ffi::OsStr,
    ) -> Result<(u64, u64, u32)> {
        let name = name
            .to_str()
            .ok_or_else(|| RuntimeError::invalid("Private directory name is not UTF-8"))?;
        let mut components = Path::new(name).components();
        if name.is_empty()
            || name.contains(['/', '\0'])
            || (!cfg!(unix) && name.contains(['\\', ':']))
            || !matches!(components.next(), Some(std::path::Component::Normal(_)))
            || components.next().is_some()
        {
            return Err(RuntimeError::invalid(
                "Private directory needs one native leaf",
            ));
        }
        let parent = self
            .directories
            .get(&Vec::<String>::new())
            .ok_or_else(|| RuntimeError::unavailable("Private root unavailable"))?
            .try_clone()?;
        let name = name.to_string();
        self.create_private_child_exclusive(parent, name.clone(), vec![name])
    }

    fn create_private_child_exclusive(
        &mut self,
        parent: File,
        name: String,
        key: Vec<String>,
    ) -> Result<(u64, u64, u32)> {
        self.check()?;
        let child = private_mkdir(&parent, &name)?;
        crate::security::secure_private_handle(&child, true)?;
        let identity = native::identity(&child)?.namespace;
        self.pins.push(Pin {
            parent: parent.try_clone()?,
            name,
            child: child.try_clone()?,
            identity,
        });
        self.directories.insert(key, child);
        #[cfg(unix)]
        parent.sync_all()?;
        self.check()?;
        Ok(identity)
    }

    /// Admit the whole existing state chain before creating or tightening anything.
    /// Retain its ancestor descriptors after selecting the state as the local root.
    pub fn open_private_root(path: &Path) -> Result<Self> {
        Self::open_directory_root(path, RootProtection::AttachedState)
    }

    /// Admit and retain native Path components without interpreting them as
    /// portable relative keys. The protection policy is explicit at the caller.
    pub fn open_directory_root(path: &Path, protection: RootProtection) -> Result<Self> {
        let path = crate::fs::absolute(path)?;
        let mut anchor = match path.parent().and_then(Path::parent) {
            Some(anchor) => anchor.to_path_buf(),
            None if protection != RootProtection::AttachedState => {
                path.parent().unwrap_or(path.as_path()).to_path_buf()
            }
            None => {
                return Err(RuntimeError::invalid(
                    "Private state needs an owned directory name",
                ));
            }
        };
        loop {
            match std::fs::symlink_metadata(&anchor) {
                Ok(_) => break,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if !anchor.pop() {
                        return Err(RuntimeError::invalid("Private state ancestor unavailable"));
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        let mut files = Self::new(&anchor)?;
        let parts = path
            .strip_prefix(&anchor)
            .map_err(|_| RuntimeError::policy("Private state escaped its ancestor"))?
            .components()
            .map(|component| {
                let std::path::Component::Normal(name) = component else {
                    return Err(RuntimeError::policy("Invalid private state component"));
                };
                let name = name
                    .to_str()
                    .ok_or_else(|| RuntimeError::invalid("Private state component is not UTF-8"))?;
                if name.is_empty()
                    || name.contains(['/', '\0'])
                    || (cfg!(windows) && name.contains('\\'))
                {
                    return Err(RuntimeError::policy("Invalid private state component"));
                }
                Ok(name.to_string())
            })
            .collect::<Result<Vec<_>>>()?;
        if parts.is_empty() {
            if protection == RootProtection::CreatedDirectories {
                // An existing drive/share/filesystem root has no owned leaf to
                // tighten. Public-root admission retains it without mutation.
                files.check()?;
                return Ok(files);
            }
            return Err(RuntimeError::invalid(
                "Private state needs an owned directory name",
            ));
        }
        // Native Path components preserve a literal Unix backslash. They are
        // never converted to the portable relative-key grammar.
        files.admit_private_root_components(&parts)?;
        let protect_from = match protection {
            RootProtection::AttachedState
                if parts.len() >= 2 && parts[parts.len() - 2] == "attached" =>
            {
                parts.len() - 2
            }
            RootProtection::AttachedState | RootProtection::SelectedRoot => parts.len() - 1,
            RootProtection::CreatedDirectories => parts.len(),
        };
        files.private_directory_components(&parts, true, protect_from)?;
        let root = files
            .directories
            .get(&parts)
            .ok_or_else(|| RuntimeError::unavailable("Private state root unavailable"))?
            .try_clone()?;
        files.check()?;
        let root_identity = native::identity(&root)?.namespace;
        let mut directories = std::collections::BTreeMap::new();
        directories.insert(vec![], root);
        let result = Self {
            root: path,
            root_identity,
            directories,
            root_pins: files.pins.len(),
            pins: files.pins,
        };
        result.check()?;
        Ok(result)
    }

    /// Preflight and pin the complete existing native chain without mutation.
    /// A missing tail is fine; every existing selected component must be a directory.
    fn admit_private_root_components(&mut self, parts: &[String]) -> Result<()> {
        self.check()?;
        for count in 1..=parts.len() {
            let key = parts[..count].to_vec();
            let parent = self
                .directories
                .get(&key[..key.len() - 1])
                .ok_or_else(|| RuntimeError::unavailable("Private ancestor unavailable"))?
                .try_clone()?;
            let child = match native::directory(&parent, &parts[count - 1]) {
                Ok(child) => child,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    self.check()?;
                    return Ok(());
                }
                Err(error) => return Err(error.into()),
            };
            let identity = native::identity(&child)?.namespace;
            if let Some(held) = self.directories.get(&key) {
                if native::identity(held)?.namespace != identity {
                    return Err(RuntimeError::conflict(
                        "Private ancestor changed during admission",
                    ));
                }
            } else {
                self.pins.push(Pin {
                    parent: parent.try_clone()?,
                    name: parts[count - 1].clone(),
                    child: child.try_clone()?,
                    identity,
                });
            }
            self.directories.insert(key, child);
            self.check()?;
        }
        Ok(())
    }

    pub fn root_path(&self) -> &Path {
        &self.root
    }

    // Permission changes affect the namespace tuple. Refresh only observations
    // of this exact held object; never bless a different file/directory ID.
    fn refresh_private_namespace(&mut self, file: &File, before: (u64, u64, u32)) -> Result<()> {
        let after = native::identity(file)?.namespace;
        if (after.0, after.1) != (before.0, before.1) {
            return Err(RuntimeError::conflict(
                "Private state object identity changed",
            ));
        }
        if (self.root_identity.0, self.root_identity.1) == (before.0, before.1) {
            self.root_identity = after;
        }
        for pin in &mut self.pins {
            if (pin.identity.0, pin.identity.1) == (before.0, before.1) {
                pin.identity = after;
            }
        }
        Ok(())
    }

    /// Returns true only when the selected directory was created.
    pub fn secure_private_directory(&mut self, relative: &str, create: bool) -> Result<bool> {
        self.private_directory_chain(relative, create, 0)
    }
    fn private_directory_chain(
        &mut self,
        relative: &str,
        create: bool,
        protect_from: usize,
    ) -> Result<bool> {
        self.check()?;
        if relative.is_empty() {
            let root = self
                .directories
                .get(&Vec::<String>::new())
                .ok_or_else(|| RuntimeError::unavailable("Private root unavailable"))?
                .try_clone()?;
            let before = native::identity(&root)?.namespace;
            crate::security::secure_private_handle(&root, true)?;
            self.refresh_private_namespace(&root, before)?;
            self.check()?;
            return Ok(false);
        }
        // Validate and pin the complete existing chain before permission changes.
        match self.parent(&format!("{relative}/.private-directory-admission"), false) {
            Ok(_) => {}
            Err(error) if error.kind == crate::ErrorKind::NotFound && create => {}
            Err(error) if error.kind == crate::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        }
        let parts = relative.split('/').map(str::to_string).collect::<Vec<_>>();
        self.private_directory_components(&parts, create, protect_from)
    }
    fn private_directory_components(
        &mut self,
        parts: &[String],
        create: bool,
        protect_from: usize,
    ) -> Result<bool> {
        let mut selected_created = false;
        for count in 1..=parts.len() {
            let key = parts[..count]
                .iter()
                .map(|part| part.to_string())
                .collect::<Vec<_>>();
            let parent = self
                .directories
                .get(&key[..key.len() - 1])
                .ok_or_else(|| RuntimeError::unavailable("Private parent unavailable"))?
                .try_clone()?;
            self.check()?;
            let mut created = false;
            let mut child = match native::directory(&parent, &parts[count - 1]) {
                Ok(child) => child,
                Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
                    match private_mkdir(&parent, &parts[count - 1]) {
                        Ok(child) => {
                            created = true;
                            if count == parts.len() {
                                selected_created = true;
                            }
                            child
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                            native::directory(&parent, &parts[count - 1])?
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                Err(error) => return Err(error.into()),
            };
            let before = native::identity(&child)?.namespace;
            if let Some(held) = self.directories.get(&key) {
                if native::identity(held)?.namespace != before {
                    return Err(RuntimeError::conflict(
                        "Private directory changed after admission",
                    ));
                }
            } else {
                self.pins.push(Pin {
                    parent: parent.try_clone()?,
                    name: parts[count - 1].clone(),
                    child: child.try_clone()?,
                    identity: before,
                });
            }
            if created || count > protect_from {
                let elevated = native::private_directory(&parent, &parts[count - 1])?;
                if native::identity(&elevated)?.namespace != before {
                    return Err(RuntimeError::conflict(
                        "Private directory changed before protection",
                    ));
                }
                crate::security::secure_private_handle(&elevated, true)?;
                self.refresh_private_namespace(&elevated, before)?;
                child = elevated;
            }
            self.directories.insert(key, child);
            self.check()?;
        }
        Ok(selected_created)
    }

    /// Capture an ordinary named file's complete image without opening its data.
    /// All lookup occurs relative to the retained parent, including both samples.
    #[cfg(unix)]
    pub fn metadata_image_identity(
        &mut self,
        relative: &str,
    ) -> Result<Option<super::FileImageIdentity>> {
        self.check()?;
        let (parent, name) = match self.parent(relative, false) {
            Ok(value) => value,
            Err(error) if error.kind == crate::ErrorKind::NotFound => {
                self.check()?;
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let before = match entry_stat(&parent, &name) {
            Ok(value) => stat_image(&value)?,
            Err(error) if error.kind == crate::ErrorKind::NotFound => {
                self.check()?;
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        if stat_image(&entry_stat(&parent, &name)?)? != before {
            return Err(RuntimeError::conflict("Named metadata image changed"));
        }
        self.check()?;
        Ok(Some(before))
    }

    /// Restore the fixed private-file mode without an ordinary data descriptor.
    /// SQL may change bytes/timestamps, but the caller's admitted inode must remain.
    /// The named chmod retains the existing pre/post same-user rename boundary.
    #[cfg(unix)]
    pub fn secure_private_file_metadata(
        &mut self,
        relative: &str,
        expected: super::FileImageIdentity,
    ) -> Result<()> {
        let (parent, name) = self.parent(relative, false)?;
        self.check()?;
        let (directory, before) = private_entry(&parent, &name)?;
        let wanted = (expected.0, expected.1, expected.2 & 0o170000);
        if directory || (before.0, before.1, before.2 & 0o170000) != wanted {
            return Err(RuntimeError::conflict(
                "Private file identity changed before protection",
            ));
        }
        if before.2 & 0o7777 != 0o600 {
            repair_private_entry_mode(&parent, &name, before, 0o600)?;
        }
        let (directory, after) = private_entry(&parent, &name)?;
        if directory
            || (after.0, after.1, after.2 & 0o170000) != wanted
            || after.2 & 0o7777 != 0o600
        {
            return Err(RuntimeError::conflict(
                "Private file identity or mode changed during protection",
            ));
        }
        self.check()
    }

    pub fn secure_private_file(&mut self, relative: &str) -> Result<()> {
        let (parent, name) = self.parent(relative, false)?;
        self.check()?;
        protect_private_file(&parent, &name)?;
        self.check()
    }

    /// Keep authority directories private through retained handles. Unix file
    /// repair uses relative nofollow chmod, never an ordinary file descriptor:
    /// closing an unrelated FD can discard a live SQLite connection's locks.
    /// The named chmod has a pre/post metadata identity boundary, like unlink;
    /// it is not an atomic compare-and-swap against a hostile same-user rename.
    ///
    /// Sessions keep writing the state while a launch walks it: SQLite
    /// journals come and go and records are renamed into place. An entry that
    /// is gone by the time it is looked at needs no protection, and one that a
    /// writer replaced while nothing was being repaired is the next pass's to
    /// examine. Walking on keeps one launch from failing for another
    /// session's write; links, foreign owners and a replacement during a
    /// repair are still refused.
    ///
    /// The walk holds every directory it enters beneath the one above it,
    /// apart from the publication's own pins. An entry's custody rests on
    /// the root, the root's ancestors and each held directory down to the
    /// entry's parent: that chain is re-proved before every repair and after
    /// every entry, at the cost of the entry's depth. Re-proving every held
    /// directory per entry made one walk quadratic: a launch over a state
    /// holding 21 Cursor plugin images took 30 s, now 0.15 s. Every held
    /// directory is re-proved once more when the walk ends, and none stays
    /// held after it.
    pub fn secure_private_tree(&mut self, relative: &str) -> Result<()> {
        match self.parent(&format!("{relative}/.private-tree-admission"), false) {
            Ok(_) => {}
            Err(error) if error.kind == crate::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        }
        self.secure_private_directory(relative, false)?;
        let (mut tree, top) = self.hold_tree(relative)?;
        let mut pending = vec![(top, relative.to_string(), 0usize)];
        let mut seen = std::collections::BTreeSet::new();
        let mut nodes = 0usize;
        while let Some((index, directory, depth)) = pending.pop() {
            if depth > 128 {
                return Err(RuntimeError::policy(
                    "Private state tree exceeds its depth boundary",
                ));
            }
            match self.check_tree_chain(&tree, index) {
                Ok(()) => {}
                Err(error) if error.kind == crate::ErrorKind::NotFound && depth > 0 => continue,
                Err(error) => return Err(error),
            }
            let before = native::identity(&tree.held[index].file)?.namespace;
            if !seen.insert((before.0, before.1)) {
                return Err(RuntimeError::policy(
                    "Repeated private state directory identity",
                ));
            }
            let mut names =
                super::cursor::cursor_names(&tree.held[index].file, Some(1_000_000 - nodes))?;
            names.sort();
            for name in names {
                nodes += 1;
                let path = format!("{directory}/{name}");
                let child = self.secure_tree_entry(&mut tree, index, &path, &name)?;
                // Protected or passed over, each entry ends with its custody
                // re-proved, so no later repair follows an unnoticed change.
                self.check_tree_chain(&tree, index)?;
                #[cfg(all(test, unix))]
                AFTER_TREE_ENTRY.with(|hook| {
                    if let Some(hook) = hook.borrow_mut().as_mut() {
                        hook(&path);
                    }
                });
                if let Some(child) = child {
                    pending.push((child, path, depth + 1));
                }
            }
            if native::identity(&tree.held[index].file)?.namespace != before {
                return Err(RuntimeError::conflict(
                    "Private state directory changed during admission",
                ));
            }
            self.check_tree_chain(&tree, index)?;
        }
        tree.check_all()?;
        self.check()
    }

    /// Hold the chain from the root down to the walk's top directory, as
    /// `secure_private_directory` has just admitted and protected it.
    fn hold_tree(&self, relative: &str) -> Result<(HeldTree, usize)> {
        let root = self
            .directories
            .get(&Vec::<String>::new())
            .ok_or_else(|| RuntimeError::unavailable("Private root unavailable"))?
            .try_clone()?;
        let parts = relative.split('/').map(str::to_string).collect::<Vec<_>>();
        let mut held = Vec::with_capacity(parts.len());
        for count in 1..=parts.len() {
            let file = self
                .directories
                .get(&parts[..count])
                .ok_or_else(|| RuntimeError::unavailable("Private tree directory unavailable"))?
                .try_clone()?;
            held.push(HeldDirectory {
                above: count.checked_sub(2),
                name: parts[count - 1].clone(),
                identity: native::identity(&file)?.namespace,
                file,
            });
        }
        Ok((HeldTree { root, held }, parts.len() - 1))
    }

    /// Re-prove the custody one held directory rests on: the root, the
    /// root's own chain and each held directory from the root down to it.
    fn check_tree_chain(&self, tree: &HeldTree, index: usize) -> Result<()> {
        self.check_root()?;
        let mut chain = vec![index];
        while let Some(above) = tree.held[chain[chain.len() - 1]].above {
            chain.push(above);
        }
        for index in chain.into_iter().rev() {
            tree.check(index)?;
        }
        Ok(())
    }

    /// Keep one entry of a held directory private. Returns the directory now
    /// held when the entry was one; an entry gone between looks needs nothing.
    fn secure_tree_entry(
        &mut self,
        tree: &mut HeldTree,
        above: usize,
        path: &str,
        name: &str,
    ) -> Result<Option<usize>> {
        // Entries are named as every publication path is.
        if super::invalid_relative(path) {
            return Err(RuntimeError::policy("Invalid publication path"));
        }
        let (is_directory, entry) = match private_entry(&tree.held[above].file, name) {
            Ok(value) => value,
            Err(error) if error.kind == crate::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if is_directory {
            return self.hold_tree_directory(tree, above, name);
        }
        let parent = &tree.held[above].file;
        #[cfg(unix)]
        let repaired = entry.2 & 0o077 != 0;
        #[cfg(not(unix))]
        let repaired = true;
        // A repair is an effect: prove where it lands just before it.
        if repaired {
            self.check_tree_chain(tree, above)?;
        }
        #[cfg(unix)]
        let protected = repair_private_entry(parent, name, entry);
        #[cfg(not(unix))]
        let protected = protect_private_file(parent, name);
        match protected {
            Ok(()) => {}
            Err(error) if error.kind == crate::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        }
        let (after_directory, after) = match private_entry(parent, name) {
            Ok(value) => value,
            Err(error) if error.kind == crate::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let replaced = after_directory
            || (after.0, after.1, after.2 & 0o170000) != (entry.0, entry.1, entry.2 & 0o170000);
        // A repair checks its own boundary and refuses a replacement;
        // without one, the replacement came after.
        if replaced && repaired {
            return Err(RuntimeError::conflict(
                "Private state file changed during admission",
            ));
        }
        Ok(None)
    }

    /// Protect a directory the walk entered and hold it beneath its held
    /// parent. None when it was gone by the time it was opened.
    fn hold_tree_directory(
        &mut self,
        tree: &mut HeldTree,
        above: usize,
        name: &str,
    ) -> Result<Option<usize>> {
        let file = match native::private_directory(&tree.held[above].file, name) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let before = native::identity(&file)?.namespace;
        #[cfg(unix)]
        let tightened = before.2 & 0o7777 != 0o700;
        #[cfg(not(unix))]
        let tightened = true;
        // Protection is an effect too: prove where it lands just before it.
        if tightened {
            self.check_tree_chain(tree, above)?;
        }
        crate::security::secure_private_handle(&file, true)?;
        let after = native::identity(&file)?.namespace;
        if after != before {
            // Wherever the publication holds this same directory, it now
            // has this mode.
            self.refresh_private_namespace(&file, before)?;
        }
        let named = native::directory(&tree.held[above].file, name)?;
        if native::identity(&named)?.namespace != after {
            return Err(RuntimeError::conflict(
                "Private directory changed during protection",
            ));
        }
        tree.held.push(HeldDirectory {
            above: Some(above),
            name: name.into(),
            file,
            identity: after,
        });
        Ok(Some(tree.held.len() - 1))
    }

    /// Lock the ordinary Brain mutex through the same state custody. The
    /// initializer performs held-custody retirement recovery before seeding.
    pub fn acquire_private_state_lock(&mut self) -> Result<crate::fs::MutationLock> {
        self.secure_private_directory("project-brain/local", true)?;
        let (parent, name) = self.parent("project-brain/local/runtime.lock", false)?;
        self.check()?;
        let file = super::cursor::rooted_lock_file(&parent, &name)?;
        let identity = native::identity(&file)?;
        let named = native::file(&parent, &name, false)?;
        if !file.metadata()?.is_file()
            || identity.links != 1
            || native::identity(&named)? != identity
        {
            return Err(RuntimeError::policy("Private state lock custody changed"));
        }
        self.secure_private_file("project-brain/local/runtime.lock")?;
        self.check()?;
        let wanted = native::identity(&file)?.namespace;
        let guard = crate::fs::MutationLock::acquire_anchored(&self.root, file)?;
        let named = native::file(&parent, &name, false)?;
        let current = native::identity(&named)?;
        if (current.namespace.0, current.namespace.1) != (wanted.0, wanted.1) || current.links != 1
        {
            return Err(RuntimeError::conflict(
                "Private state lock changed while waiting",
            ));
        }
        self.check()?;
        Ok(guard)
    }

    /// Hold an exclusive OS lock on one rooted lock file, created private when
    /// absent, across threads and processes; dropping the file releases it.
    /// Nothing is protected or written before the lock is held, so callers that
    /// serialize on it never observe another holder's effects mid-check.
    pub fn acquire_private_lock(&mut self, relative: &str) -> Result<File> {
        let (parent, name) = self.parent(relative, false)?;
        self.check()?;
        let file = super::cursor::rooted_lock_file(&parent, &name)?;
        let wanted = native::identity(&file)?;
        if !file.metadata()?.is_file() || wanted.links != 1 {
            return Err(RuntimeError::policy(
                "Private lock must have one ordinary link",
            ));
        }
        file.lock()?;
        // A lock taken on a name that was replaced meanwhile excludes no one.
        let named = native::identity(&native::file(&parent, &name, false)?)?;
        if (named.namespace.0, named.namespace.1) != (wanted.namespace.0, wanted.namespace.1)
            || named.links != 1
        {
            return Err(RuntimeError::conflict("Private lock changed while waiting"));
        }
        self.secure_private_file(relative)?;
        self.check()?;
        Ok(file)
    }
}

/// A directory the private walk entered, held until the walk ends. It was
/// opened beneath the directory above it, and holding it keeps its device
/// and file ID from being recycled.
struct HeldDirectory {
    /// The held directory above, or the root.
    above: Option<usize>,
    name: String,
    file: File,
    identity: (u64, u64, u32),
}

/// The private walk's own custody, kept apart from the publication's pins so
/// that neither the walk nor anything after it re-proves all of it per step.
struct HeldTree {
    root: File,
    held: Vec<HeldDirectory>,
}
impl HeldTree {
    /// The held directory is still the one the directory above it names.
    fn check(&self, index: usize) -> Result<()> {
        let held = &self.held[index];
        let above = held
            .above
            .map_or(&self.root, |above| &self.held[above].file);
        super::check_held(above, &held.name, &held.file, held.identity)
    }
    fn check_all(&self) -> Result<()> {
        (0..self.held.len()).try_for_each(|index| self.check(index))
    }
}

/// Called with each entry's path once the private walk has finished with it.
#[cfg(all(test, unix))]
type TreeEntryHook = Box<dyn FnMut(&str)>;
#[cfg(all(test, unix))]
thread_local! {
    /// Lets a test change the state between two entries of the walk.
    static AFTER_TREE_ENTRY: std::cell::RefCell<Option<TreeEntryHook>> =
        const { std::cell::RefCell::new(None) };
}

/// Keep one ordinary file private through a data handle, refusing a name
/// replaced meanwhile.
fn protect_private_file(parent: &File, name: &str) -> Result<()> {
    let file = native::private_file(parent, name)?;
    let before = native::identity(&file)?;
    if !file.metadata()?.is_file() || before.links != 1 {
        return Err(RuntimeError::policy(
            "Private state file must have one ordinary link",
        ));
    }
    crate::security::secure_private_handle(&file, false)?;
    let after = native::identity(&file)?;
    let named = native::file(parent, name, false)?;
    if native::identity(&named)? != after
        || (before.namespace.0, before.namespace.1) != (after.namespace.0, after.namespace.1)
    {
        return Err(RuntimeError::conflict(
            "Private state file changed during protection",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn private_mkdir(parent: &File, name: &str) -> std::io::Result<File> {
    native::mkdir_mode(parent, name, 0o700)?;
    native::directory(parent, name)
}
#[cfg(windows)]
fn private_mkdir(parent: &File, name: &str) -> std::io::Result<File> {
    native::private_create_directory(parent, name)
}
#[cfg(not(any(unix, windows)))]
fn private_mkdir(_: &File, _: &str) -> std::io::Result<File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Private directory creation is unavailable",
    ))
}

#[cfg(unix)]
fn entry_stat(parent: &File, name: &str) -> Result<libc::stat> {
    use std::{ffi::CString, os::fd::AsRawFd};
    let name = CString::new(name)
        .map_err(|_| RuntimeError::invalid("Invalid named metadata component"))?;
    let mut value = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            value.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { value.assume_init() })
}
#[cfg(unix)]
fn stat_times(value: &libc::stat) -> Result<(i128, i128)> {
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "dragonfly",
        target_os = "solaris",
        target_os = "illumos",
        target_os = "haiku",
        target_os = "redox",
    ))]
    {
        Ok((
            value.st_mtime as i128 * 1_000_000_000 + value.st_mtime_nsec as i128,
            value.st_ctime as i128 * 1_000_000_000 + value.st_ctime_nsec as i128,
        ))
    }
    #[cfg(target_os = "netbsd")]
    {
        Ok((
            value.st_mtime as i128 * 1_000_000_000 + value.st_mtimensec as i128,
            value.st_ctime as i128 * 1_000_000_000 + value.st_ctimensec as i128,
        ))
    }
    #[cfg(any(
        target_os = "aix",
        target_os = "hurd",
        target_os = "nto",
        target_os = "nuttx"
    ))]
    {
        Ok((
            value.st_mtim.tv_sec as i128 * 1_000_000_000 + value.st_mtim.tv_nsec as i128,
            value.st_ctim.tv_sec as i128 * 1_000_000_000 + value.st_ctim.tv_nsec as i128,
        ))
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "dragonfly",
        target_os = "solaris",
        target_os = "illumos",
        target_os = "haiku",
        target_os = "redox",
        target_os = "netbsd",
        target_os = "aix",
        target_os = "hurd",
        target_os = "nto",
        target_os = "nuttx",
    )))]
    {
        let _ = value;
        Err(RuntimeError::coded(
            crate::ErrorKind::Unavailable,
            "metadata-image-platform",
            "Exact named metadata timestamps are unavailable on this platform",
        ))
    }
}
#[cfg(unix)]
#[allow(
    clippy::unnecessary_cast,
    reason = "libc stat fields have platform-specific widths"
)]
fn stat_image(value: &libc::stat) -> Result<super::FileImageIdentity> {
    if value.st_mode & libc::S_IFMT != libc::S_IFREG || value.st_nlink != 1 || value.st_size < 0 {
        return Err(RuntimeError::policy(
            "Metadata image must have one ordinary file link",
        ));
    }
    let (modified, changed) = stat_times(value)?;
    Ok((
        value.st_dev as u64,
        value.st_ino as u64,
        value.st_mode as u32,
        value.st_nlink as u64,
        value.st_size as u64,
        modified,
        changed,
    ))
}

#[cfg(unix)]
#[allow(
    clippy::unnecessary_cast,
    reason = "libc stat field widths vary between Unix platforms"
)]
fn private_entry(parent: &File, name: &str) -> Result<(bool, (u64, u64, u32))> {
    let value = entry_stat(parent, name)?;
    let directory = value.st_mode & libc::S_IFMT == libc::S_IFDIR;
    let regular = value.st_mode & libc::S_IFMT == libc::S_IFREG;
    // Looked up just as a writer renamed another file over it or removed it.
    if regular && value.st_nlink == 0 {
        return Err(RuntimeError::not_found("Private state entry was removed"));
    }
    if (!directory && !regular)
        || value.st_uid != unsafe { libc::geteuid() }
        || (regular && value.st_nlink != 1)
    {
        return Err(RuntimeError::policy(
            "Private state requires owned ordinary entries",
        ));
    }
    Ok((
        directory,
        (
            value.st_dev as u64,
            value.st_ino as u64,
            value.st_mode as u32,
        ),
    ))
}
#[cfg(windows)]
fn private_entry(parent: &File, name: &str) -> Result<(bool, (u64, u64, u32))> {
    use std::os::windows::fs::MetadataExt;
    let file = match native::directory(parent, name) {
        Ok(file) => file,
        Err(_) => native::file(parent, name, false)?,
    };
    let metadata = file.metadata()?;
    let identity = native::identity(&file)?;
    if metadata.file_attributes() & 0x400 != 0
        || (!metadata.is_dir() && (!metadata.is_file() || identity.links != 1))
    {
        return Err(RuntimeError::policy(
            "Private state requires ordinary entries",
        ));
    }
    Ok((metadata.is_dir(), identity.namespace))
}
#[cfg(not(any(unix, windows)))]
fn private_entry(_: &File, _: &str) -> Result<(bool, (u64, u64, u32))> {
    Err(RuntimeError::unavailable(
        "Private state inspection is unavailable",
    ))
}

#[cfg(unix)]
fn repair_private_entry(parent: &File, name: &str, before: (u64, u64, u32)) -> Result<()> {
    if before.2 & 0o077 == 0 {
        return Ok(());
    }
    repair_private_entry_mode(parent, name, before, before.2 & 0o700)
}
#[cfg(unix)]
fn repair_private_entry_mode(
    parent: &File,
    name: &str,
    before: (u64, u64, u32),
    mode: u32,
) -> Result<()> {
    use std::{ffi::CString, os::fd::AsRawFd};
    let name =
        CString::new(name).map_err(|_| RuntimeError::invalid("Invalid private state name"))?;
    // glibc uses fchmodat2 or an O_PATH fallback; Darwin changes the resolved
    // vnode. Never add a flags=0 or ordinary File::open fallback here.
    if unsafe {
        libc::fchmodat(
            parent.as_raw_fd(),
            name.as_ptr(),
            mode as libc::mode_t,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        let error = std::io::Error::last_os_error();
        if matches!(
            error.raw_os_error(),
            Some(libc::ENOSYS | libc::EINVAL | libc::ENOTSUP)
        ) {
            return Err(RuntimeError::coded(
                crate::ErrorKind::Unavailable,
                "private-mode-repair",
                "Safe private-file permission repair is unavailable",
            ));
        }
        return Err(error.into());
    }
    let name = name
        .to_str()
        .map_err(|_| RuntimeError::invalid("Invalid private state name"))?;
    let (directory, after) = private_entry(parent, name)?;
    if directory
        || (after.0, after.1, after.2 & 0o170000) != (before.0, before.1, before.2 & 0o170000)
        || after.2 & 0o7777 != mode
    {
        return Err(RuntimeError::conflict(
            "Private state file changed during permission repair",
        ));
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod private_state_tests {
    use rusqlite::{Connection, Error, ErrorCode};
    use std::{path::PathBuf, process::Command, time::Duration};
    struct Fixture {
        root: PathBuf,
        roots: crate::Roots,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("infra-private-state-{}", crate::domain::uuid4()));
            std::fs::create_dir(&root).unwrap();
            let root = crate::fs::canonicalize(root).unwrap();
            let tooling = root.join("tooling");
            let project = root.join("project");
            let state = root.join("state");
            std::fs::create_dir(&tooling).unwrap();
            std::fs::create_dir(&project).unwrap();
            let roots = crate::Roots {
                tooling,
                project,
                state,
                shared: false,
            };
            roots.ensure_attached_state().unwrap();
            Self { root, roots }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn sqlite_reservation_probe_child() {
        let Some(path) = std::env::var_os("INFRA_PRIVATE_STATE_SQLITE_PROBE") else {
            return;
        };
        let database = Connection::open(path).unwrap();
        database.busy_timeout(Duration::ZERO).unwrap();
        let error = database
            .execute_batch("BEGIN IMMEDIATE")
            .expect_err("another process acquired a live SQLite reservation");
        assert!(matches!(error,Error::SqliteFailure(failure,_)
            if failure.code==ErrorCode::DatabaseBusy || failure.code==ErrorCode::DatabaseLocked));
    }
    #[test]
    fn repeated_admission_retains_live_sqlite_locks_for_a_custom_database_outside_local() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let fixture = Fixture::new();
        let path = fixture
            .roots
            .state
            .join("project-brain/local/custom-name.sqlite");
        let database = Connection::open(&path).unwrap();
        database
            .execute_batch(
                "CREATE TABLE retained(value TEXT);INSERT INTO retained VALUES('unfinished data');",
            )
            .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        database
            .execute_batch("BEGIN IMMEDIATE;INSERT INTO retained VALUES('pending');")
            .unwrap();
        let probe = || {
            let outcome=Command::new(std::env::current_exe().unwrap())
                .args(["--exact","bank::publication::private::private_state_tests::sqlite_reservation_probe_child","--nocapture"])
                .env("INFRA_PRIVATE_STATE_SQLITE_PROBE",&path).output().unwrap();
            assert!(
                outcome.status.success(),
                "stdout={} stderr={}",
                String::from_utf8_lossy(&outcome.stdout),
                String::from_utf8_lossy(&outcome.stderr)
            );
            assert!(
                String::from_utf8_lossy(&outcome.stdout).contains("1 passed"),
                "child did not execute its SQLite lock assertion"
            );
        };
        probe();
        fixture.roots.ensure_attached_state().unwrap();
        probe();
        assert_eq!(
            std::fs::symlink_metadata(&path).unwrap().mode() & 0o777,
            0o600,
            "legacy loose regular-file mode was not tightened"
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap()).unwrap().mode() & 0o777,
            0o700
        );
        database.execute_batch("ROLLBACK").unwrap();
        let retained: String = database
            .query_row("SELECT value FROM retained", [], |row| row.get(0))
            .unwrap();
        assert_eq!(retained, "unfinished data");
    }
    /// Custody checks compare change times, so a protection that changes no
    /// mode must not move them: two launches protecting one already-private
    /// state at once refused each other's files as changed.
    #[test]
    fn protecting_already_private_state_keeps_its_change_time() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let fixture = Fixture::new();
        let directory = fixture.roots.state.join("project-brain/local");
        let file = directory.join("custom.json");
        std::fs::write(&file, b"{}").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        let changed = |path: &std::path::Path| {
            let image = std::fs::symlink_metadata(path).unwrap();
            (image.ctime(), image.ctime_nsec(), image.mode() & 0o7777)
        };
        let (file_before, directory_before) = (changed(&file), changed(&directory));
        assert_eq!((file_before.2, directory_before.2), (0o600, 0o700));
        // Past any coarse filesystem clock tick, so a chmod would show.
        std::thread::sleep(Duration::from_millis(50));
        let mut files = super::PublicationFiles::open_private_root(&fixture.roots.state).unwrap();
        files
            .secure_private_file("project-brain/local/custom.json")
            .unwrap();
        files
            .secure_private_directory("project-brain/local", false)
            .unwrap();
        assert_eq!(changed(&file), file_before, "private file was re-chmodded");
        assert_eq!(
            changed(&directory),
            directory_before,
            "private directory was re-chmodded"
        );
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        files
            .secure_private_file("project-brain/local/custom.json")
            .unwrap();
        assert_eq!(changed(&file).2, 0o600, "a loose mode was not repaired");
    }
    /// Run `walk` with `hook` called after each entry the walk finishes.
    fn with_tree_hook<T>(hook: impl FnMut(&str) + 'static, walk: impl FnOnce() -> T) -> T {
        struct Clear;
        impl Drop for Clear {
            fn drop(&mut self) {
                super::AFTER_TREE_ENTRY.with(|slot| drop(slot.take()));
            }
        }
        super::AFTER_TREE_ENTRY.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
        let _clear = Clear;
        walk()
    }
    fn loose_files(directory: &std::path::Path, names: &[&str]) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(directory).unwrap();
        for name in names {
            std::fs::write(directory.join(name), b"private").unwrap();
            std::fs::set_permissions(directory.join(name), std::fs::Permissions::from_mode(0o644))
                .unwrap();
        }
    }
    fn mode(path: &std::path::Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::symlink_metadata(path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    }
    /// A walk re-proves an entry's custody along that entry's own chain, so
    /// what it costs per entry stays flat however many directories the state
    /// holds, and it leaves none of them held. Re-proving every directory
    /// held so far per entry made a launch over 21 Cursor plugin images take
    /// 30 s instead of 0.2 s.
    #[test]
    fn the_walk_costs_each_entry_its_depth_not_the_directories_it_holds() {
        use crate::bank::publication::revalidations;
        let walk = |images: usize| {
            let fixture = Fixture::new();
            for image in 0..images {
                let skills = format!("launch/image-{image}/skills");
                loose_files(
                    &fixture.roots.state.join(skills),
                    &["a.md", "b.md", "c.md", "d.md"],
                );
            }
            let mut files =
                super::PublicationFiles::open_private_root(&fixture.roots.state).unwrap();
            let before = revalidations();
            files.secure_private_tree("launch").unwrap();
            // Each image is its directory, its skills folder and four files.
            let per_entry = (revalidations() - before) as f64 / (images * 6) as f64;
            let before = revalidations();
            files.check().unwrap();
            let skill = fixture.roots.state.join("launch/image-0/skills/a.md");
            assert_eq!(mode(&skill), 0o600, "the walk left a loose file");
            (per_entry, revalidations() - before)
        };
        let (few, held_after_few) = walk(4);
        let (many, held_after_many) = walk(64);
        assert!(
            many <= few * 1.1,
            "custody re-proofs per entry grew with the directories: {few:.1} -> {many:.1}"
        );
        assert_eq!(
            held_after_many, held_after_few,
            "the walk left the directories it went through held"
        );
    }
    /// The walk's effects stay under custody: a directory moved out of the
    /// state while the walk is inside it is refused before a repair lands
    /// outside.
    #[test]
    fn a_directory_moved_out_mid_walk_is_refused_before_a_repair_lands_outside() {
        let fixture = Fixture::new();
        let moved = fixture.roots.state.join("launch/moved");
        loose_files(&moved, &["a.md", "b.md", "c.md"]);
        let outside = fixture.root.join("outside");
        let (from, to) = (moved.clone(), outside.clone());
        let mut files = super::PublicationFiles::open_private_root(&fixture.roots.state).unwrap();
        let error = with_tree_hook(
            move |path| {
                if path == "launch/moved/a.md" {
                    std::fs::rename(&from, &to).unwrap();
                }
            },
            || files.secure_private_tree("launch"),
        )
        .unwrap_err();
        assert!(
            matches!(
                error.kind,
                crate::ErrorKind::NotFound | crate::ErrorKind::Conflict
            ),
            "{error}"
        );
        assert_eq!(mode(&outside.join("a.md")), 0o600, "{error}");
        assert_eq!(
            (mode(&outside.join("b.md")), mode(&outside.join("c.md"))),
            (0o644, 0o644),
            "a repair landed outside the state: {error}"
        );
    }
    /// An ancestor replaced by another directory of the same name is refused
    /// though the entry's own directory still answers beneath it.
    #[test]
    fn an_ancestor_swapped_mid_walk_is_refused_before_a_repair_lands_outside() {
        let fixture = Fixture::new();
        let outer = fixture.roots.state.join("launch/outer");
        loose_files(&outer.join("inner"), &["a.md", "b.md"]);
        let outside = fixture.root.join("outside");
        let (from, to) = (outer.clone(), outside.clone());
        let mut files = super::PublicationFiles::open_private_root(&fixture.roots.state).unwrap();
        let error = with_tree_hook(
            move |path| {
                if path == "launch/outer/inner/a.md" {
                    std::fs::rename(&from, &to).unwrap();
                    std::fs::create_dir_all(from.join("inner")).unwrap();
                }
            },
            || files.secure_private_tree("launch"),
        )
        .unwrap_err();
        assert_eq!(error.kind, crate::ErrorKind::Conflict, "{error}");
        assert_eq!(
            mode(&outside.join("inner/b.md")),
            0o644,
            "a repair landed outside the state: {error}"
        );
    }
    /// Every directory the walk went through is re-proved when it ends: one
    /// replaced after the walk left it is refused, as the check of every
    /// held directory after each entry refused it before.
    #[test]
    fn a_directory_replaced_after_the_walk_left_it_is_refused_when_the_walk_ends() {
        let fixture = Fixture::new();
        let launch = fixture.roots.state.join("launch");
        // The walk takes the last name first: "z-left" before "a-later".
        loose_files(&launch.join("z-left"), &["a.md"]);
        loose_files(&launch.join("a-later"), &["a.md"]);
        let (left, outside) = (launch.join("z-left"), fixture.root.join("outside"));
        let mut files = super::PublicationFiles::open_private_root(&fixture.roots.state).unwrap();
        let error = with_tree_hook(
            move |path| {
                if path == "launch/a-later/a.md" {
                    std::fs::rename(&left, &outside).unwrap();
                    std::fs::create_dir(&left).unwrap();
                }
            },
            || files.secure_private_tree("launch"),
        )
        .unwrap_err();
        assert_eq!(error.kind, crate::ErrorKind::Conflict, "{error}");
    }
    #[test]
    fn metadata_only_file_admission_still_refuses_symlinks_and_hardlinks() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let outside = fixture.root.join("outside.sqlite");
        std::fs::write(&outside, b"unrelated bytes").unwrap();
        let target = fixture
            .roots
            .state
            .join("memory-bank/local/custom-name.sqlite");
        symlink(&outside, &target).unwrap();
        assert!(fixture.roots.secure_attached_state().is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"unrelated bytes");
        std::fs::remove_file(&target).unwrap();
        std::fs::hard_link(&outside, &target).unwrap();
        assert!(fixture.roots.secure_attached_state().is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"unrelated bytes");
    }
}

#[cfg(all(test, unix))]
mod directory_root_policy_tests {
    use super::{PublicationFiles, RootProtection};
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
    };
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("infra-directory-policy-{}", crate::domain::uuid4()));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(crate::fs::canonicalize(path).unwrap())
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn mode(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
    }
    #[test]
    fn selected_root_preserves_an_existing_parent_named_attached() {
        let fixture = Directory::new();
        let parent = fixture.0.join("attached");
        let config = parent.join("ai-infrastructure-harness");
        fs::create_dir_all(&config).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o775)).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o755)).unwrap();
        let custody =
            PublicationFiles::open_directory_root(&config, RootProtection::SelectedRoot).unwrap();
        custody.check().unwrap();
        assert_eq!(
            mode(&parent),
            0o775,
            "selected config changed an unrelated attached-named parent"
        );
        assert_eq!(mode(&config), 0o700);
    }
    #[test]
    fn created_only_preserves_existing_modes_and_uses_literal_native_components() {
        let fixture = Directory::new();
        let public = fixture.0.join("existing public base");
        fs::create_dir(&public).unwrap();
        fs::set_permissions(&public, fs::Permissions::from_mode(0o755)).unwrap();
        let existing =
            PublicationFiles::open_directory_root(&public, RootProtection::CreatedDirectories)
                .unwrap();
        existing.check().unwrap();
        assert_eq!(mode(&public), 0o755, "existing public root was tightened");
        let literal = public.join(r"new\literal base");
        let selected = literal.join("nested");
        let created =
            PublicationFiles::open_directory_root(&selected, RootProtection::CreatedDirectories)
                .unwrap();
        created.check().unwrap();
        assert_eq!(
            mode(&public),
            0o755,
            "new root creation tightened an existing ancestor"
        );
        assert_eq!(mode(&literal), 0o700);
        assert_eq!(mode(&selected), 0o700);
        assert!(
            !public.join("new").exists(),
            "native backslash was interpreted as a separator"
        );
    }
}
