use crate::{
    common,
    file_modes::FileModes,
    manifests::{MirrorClass, MirrorManifest, MirrorSpec},
};
use infra_core::{Result, RuntimeError, fs as safe};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub(crate) mod check_receipts {
    use super::*;

    const TREE_RECEIPT_SCHEMA_VERSION: u32 = 1;
    const TREE_RACY_NS: i128 = 2_000_000_000;
    const MAX_TREE_RECEIPT_ENTRIES: usize = 256;
    const MAX_FILE_DIGEST_ENTRIES: usize = 8192;
    const MAX_RECEIPT_FILE_BYTES: u64 = 16 * 1024 * 1024;

    std::thread_local! {
        static CACHE_BYPASS_DEPTH: Cell<usize> = const { Cell::new(0) };
    }

    struct BypassGuard;
    impl Drop for BypassGuard {
        fn drop(&mut self) {
            CACHE_BYPASS_DEPTH.with(|depth| {
                depth.set(depth.get().saturating_sub(1));
            });
        }
    }

    pub(crate) fn with_cache_bypass<T>(bypass: bool, f: impl FnOnce() -> T) -> T {
        if !bypass {
            return f();
        }
        CACHE_BYPASS_DEPTH.with(|depth| depth.set(depth.get().saturating_add(1)));
        let _guard = BypassGuard;
        f()
    }

    pub(crate) fn is_cache_bypassed() -> bool {
        CACHE_BYPASS_DEPTH.with(|depth| depth.get() > 0)
            || std::env::var("INFRA_CHECK_NO_CACHE").as_deref() == Ok("1")
            || std::env::var("INFRA_NO_CACHE").as_deref() == Ok("1")
            || std::env::var("INFRA_TOOLING_NO_CACHE").as_deref() == Ok("1")
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub(crate) struct CachedFileDigest {
        len: u64,
        mtime_ns: i128,
        ctime_ns: i128,
        mode: u32,
        sha256: String,
        last_used: u64,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub(crate) struct TreeReceiptEntry {
        pub(crate) scope: String,
        pub(crate) canonical_tree_sha256: String,
        pub(crate) rules_sha256: String,
        pub(crate) target_tree_sha256: String,
        pub(crate) cached_at: String,
        pub(crate) last_used: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub(crate) report: Option<Value>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub(crate) struct TreeReceiptStore {
        schema_version: u32,
        next_seq: u64,
        receipts: BTreeMap<String, TreeReceiptEntry>,
        file_digests: BTreeMap<String, CachedFileDigest>,
    }

    impl Default for TreeReceiptStore {
        fn default() -> Self {
            Self {
                schema_version: TREE_RECEIPT_SCHEMA_VERSION,
                next_seq: 1,
                receipts: BTreeMap::new(),
                file_digests: BTreeMap::new(),
            }
        }
    }

    impl TreeReceiptStore {
        fn receipt_key(
            scope: &str,
            canonical_tree_sha256: &str,
            rules_sha256: &str,
            target_tree_sha256: &str,
        ) -> String {
            common::sha256(
                format!(
                    "{TREE_RECEIPT_SCHEMA_VERSION}\0{scope}\0{canonical_tree_sha256}\0{rules_sha256}\0{target_tree_sha256}"
                )
                .as_bytes(),
            )
        }

        pub(crate) fn matches_receipt(
            &mut self,
            scope: &str,
            canonical_tree_sha256: &str,
            rules_sha256: &str,
            target_tree_sha256: &str,
        ) -> bool {
            let key = Self::receipt_key(
                scope,
                canonical_tree_sha256,
                rules_sha256,
                target_tree_sha256,
            );
            let hit = self.receipts.get(&key).is_some_and(|entry| {
                entry.scope == scope
                    && entry.canonical_tree_sha256 == canonical_tree_sha256
                    && entry.rules_sha256 == rules_sha256
                    && entry.target_tree_sha256 == target_tree_sha256
            });
            if hit {
                let seq = self.next_seq;
                self.next_seq = self.next_seq.saturating_add(1);
                if let Some(entry) = self.receipts.get_mut(&key) {
                    entry.last_used = seq;
                }
            }
            hit
        }

        pub(crate) fn matches_report(
            &mut self,
            scope: &str,
            canonical_tree_sha256: &str,
            rules_sha256: &str,
            target_tree_sha256: &str,
        ) -> Option<Value> {
            let key = Self::receipt_key(
                scope,
                canonical_tree_sha256,
                rules_sha256,
                target_tree_sha256,
            );
            let report = self.receipts.get(&key).and_then(|entry| {
                (entry.scope == scope
                    && entry.canonical_tree_sha256 == canonical_tree_sha256
                    && entry.rules_sha256 == rules_sha256
                    && entry.target_tree_sha256 == target_tree_sha256)
                    .then(|| entry.report.clone())
                    .flatten()
            });
            if report.is_some() {
                let seq = self.next_seq;
                self.next_seq = self.next_seq.saturating_add(1);
                if let Some(entry) = self.receipts.get_mut(&key) {
                    entry.last_used = seq;
                }
            }
            report
        }

        pub(crate) fn latest_for_scope_and_rules(
            &self,
            scope: &str,
            rules_sha256: &str,
            target_tree_sha256: &str,
        ) -> Option<&TreeReceiptEntry> {
            self.receipts
                .values()
                .filter(|entry| {
                    entry.scope == scope
                        && entry.rules_sha256 == rules_sha256
                        && entry.target_tree_sha256 == target_tree_sha256
                })
                .max_by_key(|entry| entry.last_used)
        }

        pub(crate) fn insert_receipt(
            &mut self,
            scope: &str,
            canonical_tree_sha256: String,
            rules_sha256: String,
            target_tree_sha256: String,
        ) {
            self.insert_report(
                scope,
                canonical_tree_sha256,
                rules_sha256,
                target_tree_sha256,
                None,
            );
        }

        pub(crate) fn insert_report(
            &mut self,
            scope: &str,
            canonical_tree_sha256: String,
            rules_sha256: String,
            target_tree_sha256: String,
            report: Option<Value>,
        ) {
            let key = Self::receipt_key(
                scope,
                &canonical_tree_sha256,
                &rules_sha256,
                &target_tree_sha256,
            );
            let seq = self.next_seq;
            self.next_seq = self.next_seq.saturating_add(1);
            self.receipts.insert(
                key,
                TreeReceiptEntry {
                    scope: scope.to_string(),
                    canonical_tree_sha256,
                    rules_sha256,
                    target_tree_sha256,
                    cached_at: infra_core::domain::utc_now(),
                    last_used: seq,
                    report,
                },
            );
            self.evict_excess();
        }

        fn evict_excess(&mut self) {
            if self.receipts.len() > MAX_TREE_RECEIPT_ENTRIES {
                let mut by_age: Vec<(u64, String)> = self
                    .receipts
                    .iter()
                    .map(|(k, v)| (v.last_used, k.clone()))
                    .collect();
                by_age.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
                let excess = self.receipts.len().saturating_sub(MAX_TREE_RECEIPT_ENTRIES);
                for (_, key) in by_age.into_iter().take(excess) {
                    self.receipts.remove(&key);
                }
            }
            if self.file_digests.len() > MAX_FILE_DIGEST_ENTRIES {
                let mut by_age: Vec<(u64, String)> = self
                    .file_digests
                    .iter()
                    .map(|(k, v)| (v.last_used, k.clone()))
                    .collect();
                by_age.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
                let excess = self
                    .file_digests
                    .len()
                    .saturating_sub(MAX_FILE_DIGEST_ENTRIES);
                for (_, key) in by_age.into_iter().take(excess) {
                    self.file_digests.remove(&key);
                }
            }
        }
    }

    pub(crate) fn time_to_unix_ns(time: std::time::SystemTime) -> Option<i128> {
        match time.duration_since(std::time::UNIX_EPOCH) {
            Ok(delta) => i128::try_from(delta.as_nanos()).ok(),
            Err(err) => i128::try_from(err.duration().as_nanos())
                .ok()
                .and_then(|ns| ns.checked_neg()),
        }
    }

    pub(crate) fn now_unix_ns() -> i128 {
        time_to_unix_ns(std::time::SystemTime::now()).unwrap_or(0)
    }

    pub(crate) fn default_receipt_path(root: &Path, filename: &str) -> PathBuf {
        infra_core::git::GitFrame::discover(root)
            .ok()
            .flatten()
            .map(|frame| frame.admin.join(filename))
            .unwrap_or_else(|| {
                let canonical =
                    infra_core::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
                std::env::temp_dir().join(format!(
                    "{}-{}.json",
                    filename.trim_end_matches(".json"),
                    common::sha256(canonical.to_string_lossy().as_bytes())
                ))
            })
    }

    pub(crate) fn load_store(path: &Path) -> TreeReceiptStore {
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            return TreeReceiptStore::default();
        };
        if !meta.is_file() || meta.len() > MAX_RECEIPT_FILE_BYTES {
            return TreeReceiptStore::default();
        }
        let Ok(bytes) = safe::read_regular(path) else {
            return TreeReceiptStore::default();
        };
        let Ok(mut store) = serde_json::from_slice::<TreeReceiptStore>(&bytes) else {
            return TreeReceiptStore::default();
        };
        if store.schema_version != TREE_RECEIPT_SCHEMA_VERSION {
            return TreeReceiptStore::default();
        }
        store.evict_excess();
        store
    }

    pub(crate) fn save_store(path: &Path, store: &TreeReceiptStore) {
        static SAVE_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        let _guard = SAVE_LOCK.get_or_init(|| std::sync::Mutex::new(())).lock();
        if let Some(parent) = path.parent() {
            let _ = safe::ensure_directory(parent, true);
        }
        let mut merged = load_store(path);
        merged.next_seq = merged.next_seq.max(store.next_seq);
        for (key, entry) in &store.receipts {
            merged
                .receipts
                .entry(key.clone())
                .and_modify(|existing| {
                    if entry.last_used > existing.last_used {
                        *existing = entry.clone();
                    }
                })
                .or_insert_with(|| entry.clone());
        }
        for (key, entry) in &store.file_digests {
            merged
                .file_digests
                .entry(key.clone())
                .and_modify(|existing| {
                    if entry.last_used > existing.last_used {
                        *existing = entry.clone();
                    }
                })
                .or_insert_with(|| entry.clone());
        }
        merged.evict_excess();
        if let Ok(bytes) = serde_json::to_vec(&merged) {
            let _ = safe::atomic_write(path, &bytes);
        }
    }

    fn stat_file(meta: &std::fs::Metadata) -> Option<(u64, i128, i128, u32, bool)> {
        let len = meta.len();
        let mtime_ns = meta.modified().ok().and_then(time_to_unix_ns)?;
        #[cfg(unix)]
        let (ctime_ns, mode, exec_bit) = {
            use std::os::unix::fs::MetadataExt;
            let ctime_ns = i128::from(meta.ctime()) * 1_000_000_000 + i128::from(meta.ctime_nsec());
            let mode = meta.mode();
            (ctime_ns, mode, (mode & 0o111) != 0)
        };
        #[cfg(not(unix))]
        let (ctime_ns, mode, exec_bit) = (mtime_ns, 0u32, false);
        Some((len, mtime_ns, ctime_ns, mode, exec_bit))
    }

    pub(crate) fn hash_regular_file(
        path: &Path,
        store: &mut TreeReceiptStore,
        now_ns: i128,
        dirty: &mut bool,
    ) -> Option<(String, bool)> {
        let meta = std::fs::symlink_metadata(path).ok()?;
        if safe::linked_at(path, &meta).unwrap_or(true) || !meta.is_file() {
            return None;
        }
        let (len, mtime_ns, ctime_ns, mode, exec_bit) = stat_file(&meta)?;
        let settled = now_ns > TREE_RACY_NS
            && mtime_ns <= now_ns - TREE_RACY_NS
            && ctime_ns <= now_ns - TREE_RACY_NS;
        let key = path.to_string_lossy().into_owned();
        if settled
            && let Some(cached) = store.file_digests.get(&key)
            && cached.len == len
            && cached.mtime_ns == mtime_ns
            && cached.ctime_ns == ctime_ns
            && cached.mode == mode
        {
            let sha = cached.sha256.clone();
            let seq = store.next_seq;
            store.next_seq = store.next_seq.saturating_add(1);
            if let Some(entry) = store.file_digests.get_mut(&key) {
                entry.last_used = seq;
            }
            return Some((sha, exec_bit));
        }
        let bytes = safe::read_regular(path).ok()?;
        let sha = common::sha256(&bytes);
        if settled {
            let seq = store.next_seq;
            store.next_seq = store.next_seq.saturating_add(1);
            store.file_digests.insert(
                key,
                CachedFileDigest {
                    len,
                    mtime_ns,
                    ctime_ns,
                    mode,
                    sha256: sha.clone(),
                    last_used: seq,
                },
            );
            store.evict_excess();
            *dirty = true;
        }
        Some((sha, exec_bit))
    }

    /// Captures Git index/config state matching `FileModes::read(dir)` without spawning Git.
    /// Returns `None` if `.git` metadata is broken/linked so the live path reports the error.
    pub(crate) fn git_modes_cue(
        dir: &Path,
        store: &mut TreeReceiptStore,
        now_ns: i128,
        dirty: &mut bool,
    ) -> Option<String> {
        let canonical = infra_core::fs::canonicalize(dir).ok()?;
        if !canonical.is_dir() {
            return None;
        }
        let mut checkout = None;
        for ancestor in canonical.ancestors() {
            let marker = ancestor.join(".git");
            let meta = match std::fs::symlink_metadata(&marker) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return None,
            };
            if safe::linked(&meta) || safe::linked_at(&marker, &meta).unwrap_or(true) {
                return None;
            }
            if meta.is_dir() {
                if std::fs::read_dir(&marker).ok()?.next().is_none() {
                    continue;
                }
            } else if !meta.is_file() {
                return None;
            }
            checkout = Some(ancestor.to_path_buf());
            break;
        }
        let Some(checkout) = checkout else {
            return Some("plain".into());
        };
        let frame = infra_core::git::GitFrame::discover(&canonical).ok()??;
        if frame.top != checkout {
            return None;
        }
        let mut parts = Vec::new();
        for (label, path) in [
            ("index", frame.admin.join("index")),
            ("config", frame.admin.join("config")),
        ] {
            if path.exists() {
                let (sha, _) = hash_regular_file(&path, store, now_ns, dirty)?;
                parts.push(format!("{label}:{sha}"));
            } else {
                parts.push(format!("{label}:absent"));
            }
        }
        if let Ok(common_dir) = frame.common_dir()
            && common_dir != frame.admin
        {
            let common_cfg = common_dir.join("config");
            if common_cfg.exists() {
                let (sha, _) = hash_regular_file(&common_cfg, store, now_ns, dirty)?;
                parts.push(format!("common_config:{sha}"));
            } else {
                parts.push("common_config:absent".into());
            }
        }
        Some(common::sha256(parts.join("\n").as_bytes()))
    }
}

#[derive(Clone, Debug)]
pub struct MirrorOptions {
    pub write: bool,
    pub use_cache: bool,
    pub cache_path: Option<PathBuf>,
}

impl Default for MirrorOptions {
    fn default() -> Self {
        Self {
            write: false,
            use_cache: true,
            cache_path: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MirrorOutcome {
    pub report: Value,
    pub cache_hit: bool,
}
// These are the exact ignored outputs materialized for memory-seed assets.
// They still participate in byte/mode parity, but cannot affect tracked attributes.
fn assembled_seed_output(relative: &str) -> bool {
    [
        "/memory-seed/assets/bin/infra-engine",
        "/memory-seed/assets/bin/infra-engine.exe",
        "/memory-seed/assets/native-artifacts.json",
    ]
    .iter()
    .any(|suffix| relative.ends_with(suffix))
}
fn split(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---\n")?;
    Some((&rest[..end + 1], &rest[end + 5..]))
}
fn replacements(mut text: String, spec: &MirrorSpec) -> String {
    for (old, new) in &spec.replacements {
        text = text.replace(old, new);
    }
    text
}
pub fn transform(relative: &str, text: &str, spec: &MirrorSpec) -> Result<String> {
    if spec.transform == "copy" {
        return Ok(replacements(text.into(), spec));
    }
    let Some((front, body)) = split(text) else {
        return Ok(replacements(text.into(), spec));
    };
    let body = replacements(body.into(), spec);
    if spec.transform == "cursor-agent" {
        let kept = front
            .lines()
            .filter(|line| {
                !["model:", "invokes:", "phase:"]
                    .iter()
                    .any(|key| line.starts_with(key))
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(format!("---\n{kept}\n---\n{body}"));
    }
    if front.lines().any(|line| line.starts_with("name:")) {
        return Ok(format!("---\n{front}---\n{body}"));
    }
    let name = relative.rsplit('/').next().unwrap_or(relative);
    let stem = name.strip_suffix(".md").unwrap_or(name);
    let description = if let Some(value) = spec.description_overrides.get(name) {
        value.clone()
    } else {
        body.lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .ok_or_else(|| {
                RuntimeError::invalid(format!("{relative}: cannot derive a command description"))
            })?
            .into()
    };
    let description = if spec.quote_description {
        format!("\"{description}\"")
    } else {
        description
    };
    Ok(format!(
        "---\nname: {stem}\ndescription: {description}\n---\n{body}"
    ))
}
fn skipped(class: &MirrorClass, spec: &MirrorSpec, framework: Option<&str>, name: &str) -> bool {
    class
        .skip
        .iter()
        .chain(spec.skip.iter())
        .chain(
            framework
                .and_then(|value| class.skip_by_framework.get(value))
                .into_iter()
                .flatten(),
        )
        .any(|entry| {
            if entry.ends_with('/') {
                name.starts_with(entry)
            } else {
                name == entry
            }
        })
}
fn compute_mirror_hashes(
    base: &Path,
    rules: &MirrorManifest,
    store: &mut check_receipts::TreeReceiptStore,
    now_ns: i128,
    dirty: &mut bool,
) -> Option<(String, String, String)> {
    let manifest_rel = if base.join("mirror-rules.json").is_file() {
        "mirror-rules.json"
    } else {
        "memory-bank/config/mirror-rules.json"
    };
    let (manifest_sha, _) =
        check_receipts::hash_regular_file(&base.join(manifest_rel), store, now_ns, dirty)?;
    let runtime_path = base.join("project-brain/config/runtime.json");
    let runtime_sha = if runtime_path.exists() {
        let (sha, _) = check_receipts::hash_regular_file(&runtime_path, store, now_ns, dirty)?;
        sha
    } else {
        "absent".into()
    };
    let git_cue = check_receipts::git_modes_cue(base, store, now_ns, dirty)?;
    let rules_sha256 = common::sha256(
        format!("manifest:{manifest_rel}:{manifest_sha}\nruntime:{runtime_sha}\ngit:{git_cue}")
            .as_bytes(),
    );

    let mut canonical_parts = Vec::new();
    let mut target_parts = Vec::new();
    for class in &rules.classes {
        let canonical_dir = base.join(&class.canonical);
        let names = if let Some(only) = &class.only {
            only.clone()
        } else {
            common::walk(&canonical_dir).ok()?
        };
        for name in &names {
            if common::ignored(name) {
                continue;
            }
            let source = common::under(&canonical_dir, name).ok()?;
            if source.exists() {
                let (sha, exec_bit) =
                    check_receipts::hash_regular_file(&source, store, now_ns, dirty)?;
                canonical_parts.push(format!("{}:{name}:{sha}:{exec_bit}", class.canonical));
            } else {
                canonical_parts.push(format!("{}:{name}:absent", class.canonical));
            }
        }
        for mirror in class.mirrors.keys() {
            let directory = base.join(mirror);
            match std::fs::symlink_metadata(&directory) {
                Ok(meta) => {
                    if safe::linked_at(&directory, &meta).unwrap_or(true) || !meta.is_dir() {
                        return None;
                    }
                    let walked = common::walk(&directory).ok()?;
                    for name in walked {
                        if common::ignored(&name)
                            || class
                                .only
                                .as_ref()
                                .is_some_and(|only| !only.contains(&name))
                        {
                            continue;
                        }
                        let file = common::under(&directory, &name).ok()?;
                        let (sha, exec_bit) =
                            check_receipts::hash_regular_file(&file, store, now_ns, dirty)?;
                        target_parts.push(format!("{mirror}:{name}:{sha}:{exec_bit}"));
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    target_parts.push(format!("{mirror}:absent_dir"));
                }
                Err(_) => return None,
            }
        }
    }
    let gitattributes = base.join(".gitattributes");
    match std::fs::symlink_metadata(&gitattributes) {
        Ok(_) => {
            let (sha, exec_bit) =
                check_receipts::hash_regular_file(&gitattributes, store, now_ns, dirty)?;
            target_parts.push(format!(".gitattributes:{sha}:{exec_bit}"));
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            target_parts.push(".gitattributes:absent".into());
        }
        Err(_) => return None,
    }
    let canonical_tree_sha256 = common::sha256(canonical_parts.join("\n").as_bytes());
    let target_tree_sha256 = common::sha256(target_parts.join("\n").as_bytes());
    Some((canonical_tree_sha256, rules_sha256, target_tree_sha256))
}

pub fn process(root: &Path, edition: &str, write: bool) -> Result<Value> {
    process_with_options(
        root,
        edition,
        MirrorOptions {
            write,
            use_cache: true,
            cache_path: None,
        },
    )
    .map(|outcome| outcome.report)
}

pub fn process_with_options(
    root: &Path,
    edition: &str,
    options: MirrorOptions,
) -> Result<MirrorOutcome> {
    let write = options.write;
    let edition = common::edition(edition)?;
    let base = root.join(edition);
    let rules = MirrorManifest::load(&base)?;
    let effective_cache = !write && options.use_cache && !check_receipts::is_cache_bypassed();
    let cache_path = effective_cache.then(|| {
        options.cache_path.clone().unwrap_or_else(|| {
            check_receipts::default_receipt_path(root, "infra-mirror-receipts.json")
        })
    });
    let mut store = cache_path
        .as_deref()
        .map(check_receipts::load_store)
        .unwrap_or_default();
    let mut dirty = false;
    let now_ns = check_receipts::now_unix_ns();
    let scope = format!("mirrors:{edition}");
    let hashes = if effective_cache {
        compute_mirror_hashes(&base, &rules, &mut store, now_ns, &mut dirty)
    } else {
        None
    };
    if effective_cache
        && let Some((ref canonical_sha, ref rules_sha, ref target_sha)) = hashes
        && store.matches_receipt(&scope, canonical_sha, rules_sha, target_sha)
    {
        if dirty && let Some(path) = cache_path.as_deref() {
            check_receipts::save_store(path, &store);
        }
        return Ok(MirrorOutcome {
            report: json!({"edition":edition,"write":false,"changed":[],"findings":[],"passed":true}),
            cache_hit: true,
        });
    }

    let framework_path = base.join("project-brain/config/runtime.json");
    let framework = if framework_path.is_file() {
        common::read_json(&framework_path)?["framework"]
            .as_str()
            .map(str::to_string)
    } else {
        None
    };
    let mut modes = FileModes::read(&base)?;
    let mut findings = Vec::<String>::new();
    let mut changed = Vec::<String>::new();
    let mut generated = BTreeSet::new();
    for class in &rules.classes {
        let canonical = base.join(&class.canonical);
        let names = if let Some(only) = &class.only {
            only.clone()
        } else {
            common::walk(&canonical)?
        };
        for mirror in class.mirrors.keys() {
            let spec = &class.mirrors[mirror];
            let mut expected = BTreeMap::new();
            for name in &names {
                if skipped(class, spec, framework.as_deref(), name) || common::ignored(name) {
                    continue;
                }
                let source = common::under(&canonical, name)?;
                if !source.is_file() {
                    findings.push(format!(
                        "{edition}: {} is listed but absent",
                        source.display()
                    ));
                    continue;
                }
                let bytes = safe::read_regular(&source)?;
                let output = if let Ok(text) = std::str::from_utf8(&bytes) {
                    transform(name, text, spec)?.into_bytes()
                } else if spec.transform == "copy" && spec.replacements.is_empty() {
                    bytes
                } else {
                    return Err(RuntimeError::invalid(
                        "Text mirror transformation received binary data",
                    ));
                };
                expected.insert(name.clone(), output);
                let relative = format!("{mirror}/{name}");
                if !assembled_seed_output(&relative) {
                    generated.insert(relative);
                }
            }
            for (name, bytes) in &expected {
                let relative = format!("{mirror}/{name}");
                let target = common::under(&base, &relative)?;
                let actual = if target.exists() {
                    Some(safe::read_regular(&target)?)
                } else {
                    None
                };
                let executable = modes.executable(&format!("{}/{name}", class.canonical))?;
                let mode_diff = if let Some(executable) = executable {
                    actual.is_some()
                        && modes
                            .executable(&relative)?
                            .is_some_and(|mode| mode != executable)
                } else {
                    false
                };
                if actual.as_deref() != Some(bytes.as_slice()) || mode_diff {
                    if write {
                        common::write(&base, &relative, bytes)?;
                        if let Some(executable) = executable {
                            modes.set(&relative, executable)?;
                        }
                        changed.push(relative);
                    } else {
                        findings.push(format!(
                            "{edition}/{relative}: mirror content or mode differs"
                        ));
                    }
                }
            }
            let directory = base.join(mirror);
            if directory.is_dir() {
                for name in common::walk(&directory)? {
                    if common::ignored(&name)
                        || skipped(class, spec, framework.as_deref(), &name)
                        || expected.contains_key(&name)
                    {
                        continue;
                    }
                    if class
                        .only
                        .as_ref()
                        .is_some_and(|only| !only.contains(&name))
                    {
                        continue;
                    }
                    let relative = format!("{mirror}/{name}");
                    if write {
                        std::fs::remove_file(common::under(&base, &relative)?)?;
                        changed.push(relative);
                    } else {
                        findings.push(format!("{edition}/{relative}: stray mirror file"));
                    }
                }
            }
        }
    }
    let mut attributes =
        String::from("# Generated by the native mirror engine; do not edit by hand.\n");
    for path in generated {
        attributes.push_str(&format!(
            "{} -diff linguist-generated=true\n",
            path.replace(' ', "?")
        ));
    }
    let current = base.join(".gitattributes");
    if !current.is_file() || safe::read_regular(&current)? != attributes.as_bytes() {
        if write {
            common::write(&base, ".gitattributes", attributes.as_bytes())?;
            changed.push(".gitattributes".into());
        } else {
            findings.push(format!(
                "{edition}/.gitattributes: generated attributes differ"
            ));
        }
    }
    if write {
        modes.flush()?;
    } else if effective_cache
        && findings.is_empty()
        && let Some((canonical_sha, rules_sha, target_sha)) = hashes
    {
        store.insert_receipt(&scope, canonical_sha, rules_sha, target_sha);
        dirty = true;
    }
    if effective_cache
        && dirty
        && let Some(path) = cache_path.as_deref()
    {
        check_receipts::save_store(path, &store);
    }
    Ok(MirrorOutcome {
        report: json!({"edition":edition,"write":write,"changed":changed,"findings":findings,"passed":findings.is_empty()}),
        cache_hit: false,
    })
}

#[cfg(test)]
mod assembled_attribute_tests {
    use super::*;
    use std::{fs, path::PathBuf};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("infra-mirror-attributes-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            let root = infra_core::fs::canonicalize(root).unwrap();
            common::git_ok(&root, &["-c", "init.templateDir=", "init", "--quiet"], None).unwrap();
            #[cfg(unix)]
            common::git_ok(&root, &["config", "core.fileMode", "true"], None).unwrap();
            Self(root)
        }
        fn write(&self, relative: &str, bytes: &[u8]) {
            let destination = self.0.join("PHP Core").join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::write(destination, bytes).unwrap();
        }
        fn process(&self, write: bool) {
            let report = process(&self.0, "PHP Core", write).unwrap();
            assert_eq!(report["passed"], true, "{report}");
        }
        fn attributes(&self) -> Vec<u8> {
            fs::read(self.0.join("PHP Core/.gitattributes")).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn assembly_copies_exact_outputs_without_changing_clean_checkout_attributes() {
        let fixture = Fixture::new();
        fixture.write(
            "memory-bank/config/mirror-rules.json",
            &common::json_bytes(&json!({
                "version":1,
                "classes":[{
                    "name":"skills",
                    "canonical":".agents/skills",
                    "mirrors":{
                        ".claude/skills":{"transform":"copy"},
                        ".cursor/skills":{"transform":"copy"}
                    }
                }]
            }))
            .unwrap(),
        );
        fixture.write(".agents/skills/ordinary/SKILL.md", b"# Ordinary skill\n");
        // Similar names outside memory-seed remain normal tracked mirror inputs.
        fixture.write(
            ".agents/skills/ordinary/assets/bin/infra-engine",
            b"ordinary helper\n",
        );
        fixture.write(".agents/skills/ordinary/native-artifacts.json", b"{}\n");
        fixture.process(true);
        let before = fixture.attributes();
        let text = std::str::from_utf8(&before).unwrap();
        assert!(text.contains(".cursor/skills/ordinary/assets/bin/infra-engine -diff"));
        assert!(text.contains(".claude/skills/ordinary/native-artifacts.json -diff"));

        // This checks binary copying, not capability admission. Assembly and
        // installer integration tests separately admit their actual native engine.
        let outputs: [(&str, &[u8]); 3] = [
            ("bin/infra-engine", b"\x7fELF\0\xffmirror image\n"),
            ("bin/infra-engine.exe", b"MZ\0\xffmirror image\n"),
            ("native-artifacts.json", b"{\"engine\":\"infra-engine\"}\n"),
        ];
        for (name, bytes) in outputs {
            let relative = format!(".agents/skills/memory-seed/assets/{name}");
            fixture.write(&relative, bytes);
            #[cfg(unix)]
            if name.starts_with("bin/") {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(
                    fixture.0.join("PHP Core").join(&relative),
                    fs::Permissions::from_mode(0o755),
                )
                .unwrap();
            }
        }
        fixture.process(true);
        assert_eq!(
            fixture.attributes(),
            before,
            "assembly must not alter tracked attributes"
        );
        for mirror in [".claude/skills", ".cursor/skills"] {
            for (name, bytes) in outputs {
                let destination = fixture
                    .0
                    .join("PHP Core")
                    .join(format!("{mirror}/memory-seed/assets/{name}"));
                assert_eq!(fs::read(&destination).unwrap(), bytes);
                #[cfg(unix)]
                if name.starts_with("bin/") {
                    use std::os::unix::fs::PermissionsExt;
                    // FileModes preserves owner execution. Private publication
                    // need not grant group or other execute permissions.
                    assert_eq!(
                        fs::metadata(destination).unwrap().permissions().mode() & 0o100,
                        0o100
                    );
                }
            }
        }
        fixture.process(false);

        // A clean checkout omits all ignored canonical and mirrored artifacts.
        for prefix in [".agents/skills", ".claude/skills", ".cursor/skills"] {
            for (name, _) in outputs {
                fs::remove_file(
                    fixture
                        .0
                        .join("PHP Core")
                        .join(format!("{prefix}/memory-seed/assets/{name}")),
                )
                .unwrap();
            }
        }
        fixture.process(false);
        assert_eq!(fixture.attributes(), before);
    }

    #[test]
    fn mirror_check_receipt_hits_when_unchanged_and_invalidates_on_any_tree_or_rule_drift() {
        let fixture = Fixture::new();
        let cache_dir = crate::temp::Temporary::new("infra-mirror-cache").unwrap();
        let cache_file = cache_dir.path().join("mirrors.json");
        fixture.write(
            "memory-bank/config/mirror-rules.json",
            &common::json_bytes(&json!({
                "version": 1,
                "classes": [{
                    "name": "skills",
                    "canonical": ".agents/skills",
                    "mirrors": {
                        ".claude/skills": {"transform": "copy"},
                        ".cursor/skills": {"transform": "copy"}
                    }
                }]
            }))
            .unwrap(),
        );
        fixture.write(".agents/skills/ordinary/SKILL.md", b"# Ordinary skill\n");
        fixture.write(".agents/skills/ordinary/run.sh", b"#!/bin/sh\necho ok\n");

        // `--write` never short-circuits.
        let write_run = process_with_options(
            &fixture.0,
            "PHP Core",
            MirrorOptions {
                write: true,
                use_cache: true,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert!(!write_run.cache_hit);
        assert_eq!(write_run.report["passed"], true);

        // First `--check` populates receipt; second `--check` hits receipt.
        let cold = process_with_options(
            &fixture.0,
            "PHP Core",
            MirrorOptions {
                write: false,
                use_cache: true,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert!(!cold.cache_hit);
        assert_eq!(cold.report["passed"], true);

        let warm = process_with_options(
            &fixture.0,
            "PHP Core",
            MirrorOptions {
                write: false,
                use_cache: true,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert!(warm.cache_hit);
        assert_eq!(warm.report, cold.report);

        // Explicit bypass & thread-local strict bypass never hit receipt.
        let bypassed = process_with_options(
            &fixture.0,
            "PHP Core",
            MirrorOptions {
                write: false,
                use_cache: false,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert!(!bypassed.cache_hit);
        assert_eq!(bypassed.report, cold.report);

        let scoped_bypass = check_receipts::with_cache_bypass(true, || {
            process_with_options(
                &fixture.0,
                "PHP Core",
                MirrorOptions {
                    write: false,
                    use_cache: true,
                    cache_path: Some(cache_file.clone()),
                },
            )
        })
        .unwrap();
        assert!(!scoped_bypass.cache_hit);

        // Target file drift invalidates target_tree_sha256 and fails.
        fixture.write(".cursor/skills/ordinary/SKILL.md", b"# Drifted\n");
        let target_drift = process_with_options(
            &fixture.0,
            "PHP Core",
            MirrorOptions {
                write: false,
                use_cache: true,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert!(!target_drift.cache_hit);
        assert_eq!(target_drift.report["passed"], false);

        // Restoring target restores passing receipt hit.
        fixture.write(".cursor/skills/ordinary/SKILL.md", b"# Ordinary skill\n");
        let restored = process_with_options(
            &fixture.0,
            "PHP Core",
            MirrorOptions {
                write: false,
                use_cache: true,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert!(restored.cache_hit);
        assert_eq!(restored.report["passed"], true);

        // Canonical edit invalidates canonical_tree_sha256.
        fixture.write(".agents/skills/ordinary/SKILL.md", b"# Updated\n");
        let canonical_drift = process_with_options(
            &fixture.0,
            "PHP Core",
            MirrorOptions {
                write: false,
                use_cache: true,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert!(!canonical_drift.cache_hit);
        assert_eq!(canonical_drift.report["passed"], false);

        // Re-sync and confirm rules edit invalidates rules_sha256.
        fixture.process(true);
        let warm2 = process_with_options(
            &fixture.0,
            "PHP Core",
            MirrorOptions {
                write: false,
                use_cache: true,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert!(!warm2.cache_hit);
        let warm3 = process_with_options(
            &fixture.0,
            "PHP Core",
            MirrorOptions {
                write: false,
                use_cache: true,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert!(warm3.cache_hit);

        fixture.write(
            "memory-bank/config/mirror-rules.json",
            &common::json_bytes(&json!({
                "version": 1,
                "classes": [{
                    "name": "skills",
                    "canonical": ".agents/skills",
                    "mirrors": {
                        ".claude/skills": {"transform": "copy", "replacements": [["Updated", "Changed"]]},
                        ".cursor/skills": {"transform": "copy"}
                    }
                }]
            }))
            .unwrap(),
        );
        let rules_drift = process_with_options(
            &fixture.0,
            "PHP Core",
            MirrorOptions {
                write: false,
                use_cache: true,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert!(!rules_drift.cache_hit);
        assert_eq!(rules_drift.report["passed"], false);

        // Restore rules and verify Unix executable permission drift invalidates.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fixture.write(
                "memory-bank/config/mirror-rules.json",
                &common::json_bytes(&json!({
                    "version": 1,
                    "classes": [{
                        "name": "skills",
                        "canonical": ".agents/skills",
                        "mirrors": {
                            ".claude/skills": {"transform": "copy"},
                            ".cursor/skills": {"transform": "copy"}
                        }
                    }]
                }))
                .unwrap(),
            );
            fixture.process(true);
            let before_chmod = process_with_options(
                &fixture.0,
                "PHP Core",
                MirrorOptions {
                    write: false,
                    use_cache: true,
                    cache_path: Some(cache_file.clone()),
                },
            )
            .unwrap();
            assert!(before_chmod.cache_hit);
            let run_sh = fixture.0.join("PHP Core/.agents/skills/ordinary/run.sh");
            fs::set_permissions(&run_sh, fs::Permissions::from_mode(0o755)).unwrap();
            let after_chmod = process_with_options(
                &fixture.0,
                "PHP Core",
                MirrorOptions {
                    write: false,
                    use_cache: true,
                    cache_path: Some(cache_file),
                },
            )
            .unwrap();
            assert!(!after_chmod.cache_hit);
            assert_eq!(after_chmod.report["passed"], false);
        }
    }
}
