//! Native check catalog and bounded concurrent runner. Python is oracle-only.
use crate::{common, process, temp::Temporary};
use infra_core::{Result, RuntimeError, fs as safe};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
static INTERRUPTED: AtomicBool = AtomicBool::new(false);
#[cfg(windows)]
unsafe extern "system" fn console_interrupted(event: u32) -> i32 {
    if [0, 1, 2, 5, 6].contains(&event) {
        INTERRUPTED.store(true, Ordering::Release);
        1
    } else {
        0
    }
}
#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
}
#[cfg(unix)]
extern "C" fn interrupted(_: i32) {
    INTERRUPTED.store(true, Ordering::Release);
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub schema_version: u32,
    pub groups: Vec<Group>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub name: String,
    pub workflow: String,
    pub job: String,
    pub timeout_minutes: u64,
    #[serde(default = "yes")]
    pub default: bool,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default = "stable_toolchain")]
    pub rust_toolchain: String,
    #[serde(default)]
    pub rust_targets: Vec<String>,
    pub gates: Vec<Gate>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Gate {
    Tool {
        name: String,
        operation: String,
        #[serde(default)]
        request: Value,
    },
    Process {
        name: String,
        program: String,
        #[serde(default)]
        arguments: Vec<String>,
        #[serde(default = "dot")]
        cwd: String,
        #[serde(default)]
        tools: Vec<String>,
        /// The same check through programs every checkout has, run instead
        /// of failing or skipping when a listed tool is unavailable.
        #[serde(default)]
        fallback: Option<Fallback>,
        #[serde(default)]
        paths: Vec<String>,
        #[serde(default = "seconds")]
        timeout_seconds: u64,
        #[serde(default)]
        oracle: bool,
    },
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fallback {
    program: String,
    #[serde(default)]
    arguments: Vec<String>,
}
fn yes() -> bool {
    true
}
fn dot() -> String {
    ".".into()
}
fn seconds() -> u64 {
    300
}
fn stable_toolchain() -> String {
    "stable".into()
}
fn valid_rust_toolchain(value: &str) -> bool {
    if value == "stable" {
        return true;
    }
    let mut parts = value.split('.');
    let numbers = [parts.next(), parts.next(), parts.next()].map(|part| {
        part.filter(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (*part == "0" || !part.starts_with('0'))
        })
        .and_then(|part| part.parse::<u16>().ok())
    });
    parts.next().is_none() && matches!(numbers, [Some(1), Some(minor), Some(_)] if minor >= 85)
}
fn validate_rust_configuration(group: &Group) -> Result<()> {
    if !valid_rust_toolchain(&group.rust_toolchain) {
        return Err(RuntimeError::invalid(
            "CI toolchain must be stable or a pinned Rust 1.85+ release",
        ));
    }
    if group.rust_targets.iter().collect::<BTreeSet<_>>().len() != group.rust_targets.len()
        || group.rust_targets.iter().any(|target| {
            target.is_empty()
                || target.len() > 128
                || !target
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
        })
    {
        return Err(RuntimeError::invalid(
            "Native CI Rust targets must be unique target identifiers",
        ));
    }
    Ok(())
}
fn process_argument(value: &str, root: &Path, base: &str, toolchain: &str) -> String {
    // Expand the selected compiler before inserting caller-owned path strings.
    value
        .replace("{toolchain}", toolchain)
        .replace("{root}", &root.to_string_lossy())
        .replace("{base}", base)
        .replace(
            "{engine}",
            &std::env::current_exe()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default(),
        )
}
impl Gate {
    fn name(&self) -> &str {
        match self {
            Self::Tool { name, .. } | Self::Process { name, .. } => name,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
struct Outcome {
    group: String,
    name: String,
    status: String,
    seconds: f64,
    code: Option<i32>,
    reason: Option<String>,
    log: Option<PathBuf>,
    /// The last lines of a failed check's streams. A CI job's retained log
    /// ends with its runner, so the run's own output must show the failure.
    #[serde(skip)]
    excerpt: Vec<(&'static str, String)>,
}
const EXCERPT_LINES: usize = 150;
const EXCERPT_BYTES: usize = 48 * 1024;
/// The end of each nonempty stream, at most `EXCERPT_LINES` lines of its last
/// `EXCERPT_BYTES` bytes.
fn excerpt(streams: &[(&'static str, &[u8])]) -> Vec<(&'static str, String)> {
    streams
        .iter()
        .filter(|(_, bytes)| !bytes.is_empty())
        .map(|(stream, bytes)| {
            let window = &bytes[bytes.len().saturating_sub(EXCERPT_BYTES)..];
            let text = window.strip_suffix(b"\n").unwrap_or(window);
            let start = text
                .iter()
                .enumerate()
                .rev()
                .filter(|(_, byte)| **byte == b'\n')
                .nth(EXCERPT_LINES - 1)
                .map_or(0, |(at, _)| at + 1);
            (
                *stream,
                String::from_utf8_lossy(&text[start..]).into_owned(),
            )
        })
        .collect()
}
/// What each failed check printed last; empty when nothing failed.
fn failure_report(results: &[Outcome]) -> String {
    let mut report = String::new();
    for result in results.iter().filter(|result| result.status == "fail") {
        report.push_str(&format!("FAILED {} / {}", result.group, result.name));
        if let Some(code) = result.code {
            report.push_str(&format!(" (exit {code})"));
        }
        if let Some(reason) = &result.reason {
            report.push_str(&format!(": {reason}"));
        }
        report.push('\n');
        for (stream, text) in &result.excerpt {
            report.push_str(&format!("--- end of {stream} ---\n{text}\n"));
        }
    }
    report
}
fn interpolate(value: &Value, root: &Path, base: &str) -> Value {
    match value {
        Value::String(value) => Value::String(
            value
                .replace("{root}", &root.to_string_lossy())
                .replace("{base}", base),
        ),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| interpolate(value, root, base))
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), interpolate(value, root, base)))
                .collect(),
        ),
        _ => value.clone(),
    }
}
fn available(tool: &str, root: &Path) -> bool {
    let path = Path::new(tool);
    if path.is_absolute() || tool.contains('/') || tool.contains('\\') {
        return root.join(path).is_file();
    }
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|directory| {
            let path = directory.join(tool);
            if path.is_file() {
                return true;
            }
            cfg!(windows)
                && ["exe", "cmd", "bat"]
                    .iter()
                    .any(|extension| directory.join(format!("{tool}.{extension}")).is_file())
        })
    })
}
fn load_catalog(root: &Path, require_workflows: bool) -> Result<Catalog> {
    let path = root.join("install/check-catalog.json");
    let catalog: Catalog = serde_json::from_slice(&safe::read_regular(&path)?)?;
    if catalog.schema_version != 1 || catalog.groups.is_empty() {
        return Err(RuntimeError::invalid(
            "Unsupported or empty native check catalog",
        ));
    }
    let mut names = BTreeSet::new();
    let mut jobs = BTreeSet::new();
    for group in &catalog.groups {
        if group.name.is_empty()
            || !names.insert(&group.name)
            || !jobs.insert((&group.workflow, &group.job))
            || group.gates.is_empty()
            || group.timeout_minutes == 0
        {
            return Err(RuntimeError::invalid(
                "Native check catalog has duplicate or empty groups",
            ));
        }
        validate_rust_configuration(group)?;
        common::relative(&group.workflow)?;
        if require_workflows
            && !root
                .join(".github/workflows")
                .join(&group.workflow)
                .is_file()
        {
            return Err(RuntimeError::invalid(
                "Check catalog names an absent workflow",
            ));
        }
        for gate in &group.gates {
            if gate.name().is_empty() {
                return Err(RuntimeError::invalid("Check catalog gate has no name"));
            }
            if let Gate::Process {
                program,
                fallback,
                oracle,
                ..
            } = gate
                && std::iter::once(program)
                    .chain(fallback.as_ref().map(|fallback| &fallback.program))
                    .any(|program| program == "python" || program.starts_with("python3"))
                && !*oracle
            {
                return Err(RuntimeError::policy(
                    "Python command is allowed only as an explicitly selected verification oracle",
                ));
            }
        }
    }
    Ok(catalog)
}
struct GateContext<'a> {
    root: &'a Path,
    base: &'a str,
    group: &'a Group,
    strict: bool,
    no_cache: bool,
    oracle_allowed: bool,
    logs: &'a Path,
    cancel: Arc<AtomicBool>,
}

const CHECKER_RACY_NS: i128 = 2_000_000_000;
const MAX_CHECK_RECEIPTS: usize = 512;
const MAX_CLOSURE_FILES: usize = 4000;

fn time_to_unix_ns(time: std::time::SystemTime) -> Option<i128> {
    crate::mirrors::check_receipts::time_to_unix_ns(time)
}

fn now_unix_ns() -> i128 {
    crate::mirrors::check_receipts::now_unix_ns()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CheckReceiptStore {
    #[serde(default)]
    next_seq: u64,
    entries: std::collections::BTreeMap<String, CheckReceiptEntry>,
    #[serde(skip)]
    dirty: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CheckReceiptEntry {
    group: String,
    gate: String,
    cached_at: String,
    #[serde(default)]
    last_used: u64,
}

fn load_receipt_store(path: &Path) -> CheckReceiptStore {
    let Ok(bytes) = safe::read_regular(path) else {
        return CheckReceiptStore::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

fn evict_oldest_receipts(store: &mut CheckReceiptStore) {
    if store.entries.len() <= MAX_CHECK_RECEIPTS {
        return;
    }
    let mut by_age: Vec<(u64, String)> = store
        .entries
        .iter()
        .map(|(k, v)| (v.last_used, k.clone()))
        .collect();
    by_age.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let excess = store.entries.len().saturating_sub(MAX_CHECK_RECEIPTS);
    for (_, evict_key) in by_age.into_iter().take(excess) {
        store.entries.remove(&evict_key);
    }
}

fn save_receipt_store(path: &Path, store: &CheckReceiptStore) {
    static SAVE_LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    let _guard = SAVE_LOCK.get_or_init(|| Mutex::new(())).lock();
    if let Some(parent) = path.parent() {
        let _ = safe::ensure_directory(parent, true);
    }
    let mut merged = load_receipt_store(path);
    merged.next_seq = merged.next_seq.max(store.next_seq);
    for (key, entry) in &store.entries {
        merged
            .entries
            .entry(key.clone())
            .and_modify(|existing| {
                if entry.last_used > existing.last_used {
                    *existing = entry.clone();
                }
            })
            .or_insert_with(|| entry.clone());
    }
    evict_oldest_receipts(&mut merged);
    if let Ok(bytes) = serde_json::to_vec(&merged) {
        let _ = safe::atomic_write(path, &bytes);
    }
}

fn resolve_tool_binary_stamp(program: &str, root: &Path) -> Option<String> {
    let direct = Path::new(program);
    let found = if direct.components().count() > 1 {
        let candidate = if direct.is_absolute() {
            direct.to_path_buf()
        } else {
            root.join(direct)
        };
        candidate.is_file().then_some(candidate)
    } else {
        std::env::var_os("PATH").and_then(|path_var| {
            std::env::split_paths(&path_var)
                .map(|dir| dir.join(program))
                .find(|candidate| candidate.is_file())
        })
    }?;
    let meta = std::fs::symlink_metadata(&found).ok()?;
    let mtime = meta.modified().ok().and_then(time_to_unix_ns).unwrap_or(0);
    Some(format!("{}:{}:{mtime}", found.display(), meta.len()))
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ScopeKind {
    ScopedDirs,
    GitTrackedGlobs,
    FullRepo,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ClosureScope {
    kind: ScopeKind,
    dirs: Vec<String>,
    globs: Vec<String>,
    extra_files: Vec<String>,
    git_base: Option<String>,
    ir_revision_hint: Option<u64>,
    graph_ir_db: Option<PathBuf>,
}

fn normalize_rel_path(root: &Path, path: &Path) -> Option<String> {
    let rel = if path.is_absolute() {
        path.strip_prefix(root).ok()?
    } else {
        path
    };
    let mut parts = Vec::new();
    for component in rel.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(part) => parts.push(part.to_str()?),
            _ => return None,
        }
    }
    if parts.is_empty() {
        Some(".".into())
    } else {
        Some(parts.join("/"))
    }
}

fn request_editions(req: &Value) -> Option<Vec<String>> {
    let raw = if let Some(arr) = req.get("editions").and_then(Value::as_array) {
        arr.iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>()
    } else if let Some(one) = req.get("edition").and_then(Value::as_str) {
        vec![one.to_string()]
    } else {
        let arr = req.get("edition").and_then(Value::as_array)?;
        arr.iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    let mut resolved = Vec::new();
    for item in raw {
        let edition = common::edition(&item).ok()?.to_string();
        if !resolved.contains(&edition) {
            resolved.push(edition);
        }
    }
    (!resolved.is_empty()).then_some(resolved)
}

fn cargo_package_scope(
    root: &Path,
    manifest_rel: &str,
    package: &str,
) -> Option<(Vec<String>, Vec<String>)> {
    let manifest_path = root.join(manifest_rel);
    let manifest_dir = Path::new(manifest_rel)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let manifest_text = std::fs::read_to_string(&manifest_path).ok()?;
    let doc = manifest_text.parse::<toml_edit::DocumentMut>().ok()?;
    let members = doc
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())?;

    let mut member_dirs = Vec::new();
    for entry in members.iter().filter_map(|v| v.as_str()) {
        if let Some(prefix) = entry.strip_suffix("/*") {
            let parent = root.join(manifest_dir).join(prefix);
            if let Ok(read_dir) = std::fs::read_dir(&parent) {
                for child in read_dir.flatten() {
                    let path = child.path();
                    if path.join("Cargo.toml").is_file() {
                        member_dirs.push(path);
                    }
                }
            }
        } else {
            let path = root.join(manifest_dir).join(entry);
            if path.join("Cargo.toml").is_file() {
                member_dirs.push(path);
            }
        }
    }

    let mut by_name = BTreeMap::new();
    for dir in member_dirs {
        if let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml"))
            && let Ok(pkg_doc) = text.parse::<toml_edit::DocumentMut>()
            && let Some(name) = pkg_doc
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
        {
            by_name.insert(name.to_string(), dir);
        }
    }
    let start = by_name.get(package)?.clone();
    let mut queue = VecDeque::from([start]);
    let mut seen_dirs = BTreeSet::new();
    let mut dirs = Vec::new();
    while let Some(dir) = queue.pop_front() {
        let canonical_dir = infra_core::fs::canonicalize(&dir).ok()?;
        let rel = normalize_rel_path(root, &canonical_dir)?;
        if !seen_dirs.insert(rel.clone()) {
            continue;
        }
        dirs.push(rel);
        let text = std::fs::read_to_string(dir.join("Cargo.toml")).ok()?;
        let pkg_doc = text.parse::<toml_edit::DocumentMut>().ok()?;
        for table_name in ["dependencies", "dev-dependencies", "build-dependencies"] {
            if let Some(table) = pkg_doc.get(table_name).and_then(|v| v.as_table_like()) {
                for (_, val) in table.iter() {
                    if let Some(dep_path) = val.get("path").and_then(|p| p.as_str()) {
                        let next = dir.join(dep_path);
                        if next.join("Cargo.toml").is_file() {
                            queue.push_back(next);
                        }
                    }
                }
            }
        }
        if let Some(targets) = pkg_doc.get("target").and_then(|v| v.as_table_like()) {
            for (_, target_item) in targets.iter() {
                for table_name in ["dependencies", "dev-dependencies", "build-dependencies"] {
                    if let Some(table) = target_item.get(table_name).and_then(|v| v.as_table_like())
                    {
                        for (_, val) in table.iter() {
                            if let Some(dep_path) = val.get("path").and_then(|p| p.as_str()) {
                                let next = dir.join(dep_path);
                                if next.join("Cargo.toml").is_file() {
                                    queue.push_back(next);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let manifest_prefix = normalize_rel_path(root, &root.join(manifest_dir))?;
    let mut extra = vec![manifest_rel.to_string()];
    for suffix in ["Cargo.lock", ".cargo/config.toml", ".config"] {
        let candidate = if manifest_prefix == "." {
            suffix.to_string()
        } else {
            format!("{manifest_prefix}/{suffix}")
        };
        extra.push(candidate);
    }
    Some((dirs, extra))
}

fn discover_graph_ir_cue(root: &Path, request: &Value) -> (Option<u64>, Option<PathBuf>) {
    let ir_revision_hint = request["ir_revision"]
        .as_u64()
        .or_else(|| request["graph_ir_revision"].as_u64())
        .or_else(|| {
            std::env::var("INFRA_GRAPH_IR_REVISION")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
        });
    let explicit_db = request["graph_ir_db"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| {
            let p = PathBuf::from(s);
            if p.is_absolute() { p } else { root.join(p) }
        });
    let graph_ir_db = explicit_db.or_else(|| {
        let local = root.join(".infra-graph/graph_ir.sqlite3");
        if local.is_file() {
            return Some(local);
        }
        infra_core::git::GitFrame::discover(root)
            .ok()
            .flatten()
            .map(|frame| frame.admin.join("graph_ir.sqlite3"))
            .filter(|p| p.is_file())
    });
    (ir_revision_hint, graph_ir_db)
}

fn scope_for_gate(
    root: &Path,
    request: &Value,
    base: &str,
    group: &Group,
    gate: &Gate,
) -> ClosureScope {
    let (ir_revision_hint, graph_ir_db) = discover_graph_ir_cue(root, request);
    let mut extra_files = vec!["install/check-catalog.json".to_string()];
    match gate {
        Gate::Tool {
            operation,
            request: gate_req,
            ..
        } => {
            let req = interpolate(gate_req, root, base);
            match operation.as_str() {
                "mirrors" | "policy-lock" => {
                    let dirs = request_editions(&req).unwrap_or_else(|| {
                        common::EDITIONS.iter().map(|s| (*s).to_string()).collect()
                    });
                    ClosureScope {
                        kind: ScopeKind::ScopedDirs,
                        dirs,
                        globs: vec![],
                        extra_files,
                        git_base: None,
                        ir_revision_hint,
                        graph_ir_db,
                    }
                }
                "core-sync" => {
                    extra_files.push("install/core-manifest.json".into());
                    let dirs = crate::manifests::CoreManifest::load(root)
                        .map(|m| {
                            let mut d = vec![m.canonical];
                            for ed in m.editions.into_values() {
                                if !d.contains(&ed) {
                                    d.push(ed);
                                }
                            }
                            d
                        })
                        .unwrap_or_else(|_| {
                            common::EDITIONS.iter().map(|s| (*s).to_string()).collect()
                        });
                    ClosureScope {
                        kind: ScopeKind::ScopedDirs,
                        dirs,
                        globs: vec![],
                        extra_files,
                        git_base: None,
                        ir_revision_hint,
                        graph_ir_db,
                    }
                }
                "assets" | "asset-parity" => {
                    extra_files.push(
                        "runtime-rust/crates/infra-generator/resources/native-runtime-contract.json"
                            .into(),
                    );
                    ClosureScope {
                        kind: ScopeKind::ScopedDirs,
                        dirs: vec![
                            "PHP Core/memory-bank".into(),
                            "PHP Core/project-brain".into(),
                            "Infrastructure-Creator/.agents/skills/memory-seed/assets".into(),
                        ],
                        globs: vec![],
                        extra_files,
                        git_base: None,
                        ir_revision_hint,
                        graph_ir_db,
                    }
                }
                "inventory-check" | "inventory-write" => {
                    let mut dirs = vec!["install/inventories".to_string()];
                    let eds = request_editions(&req).unwrap_or_else(|| {
                        common::PHP_EDITIONS
                            .iter()
                            .map(|s| (*s).to_string())
                            .collect()
                    });
                    dirs.extend(eds);
                    ClosureScope {
                        kind: ScopeKind::ScopedDirs,
                        dirs,
                        globs: vec![],
                        extra_files,
                        git_base: None,
                        ir_revision_hint,
                        graph_ir_db,
                    }
                }
                "budget" | "context-budget" => {
                    extra_files.push("scripts/token_budget.json".into());
                    ClosureScope {
                        kind: ScopeKind::ScopedDirs,
                        dirs: common::EDITIONS.iter().map(|s| (*s).to_string()).collect(),
                        globs: vec![],
                        extra_files,
                        git_base: None,
                        ir_revision_hint,
                        graph_ir_db,
                    }
                }
                "php-snippets" => ClosureScope {
                    kind: ScopeKind::GitTrackedGlobs,
                    dirs: vec![],
                    globs: vec!["*.md".into(), ":(glob,exclude)**/Task/**".into()],
                    extra_files,
                    git_base: None,
                    ir_revision_hint,
                    graph_ir_db,
                },
                "json-syntax" => ClosureScope {
                    kind: ScopeKind::GitTrackedGlobs,
                    dirs: vec![],
                    globs: vec!["*.json".into()],
                    extra_files,
                    git_base: None,
                    ir_revision_hint,
                    graph_ir_db,
                },
                "stabilization" | "routes" | "registry" => {
                    let mut dirs = vec!["install".into(), "docs".into(), "scripts".into()];
                    dirs.extend(common::EDITIONS.iter().map(|s| (*s).to_string()));
                    ClosureScope {
                        kind: ScopeKind::ScopedDirs,
                        dirs,
                        globs: vec![],
                        extra_files,
                        git_base: None,
                        ir_revision_hint,
                        graph_ir_db,
                    }
                }
                "changelog" => ClosureScope {
                    kind: ScopeKind::FullRepo,
                    dirs: vec![],
                    globs: vec![],
                    extra_files,
                    git_base: Some(base.to_string()),
                    ir_revision_hint,
                    graph_ir_db,
                },
                _ => ClosureScope {
                    kind: ScopeKind::FullRepo,
                    dirs: vec![],
                    globs: vec![],
                    extra_files,
                    git_base: None,
                    ir_revision_hint,
                    graph_ir_db,
                },
            }
        }
        Gate::Process {
            program,
            arguments,
            cwd,
            paths,
            ..
        } => {
            let expand = |value: &str| process_argument(value, root, base, &group.rust_toolchain);
            let expanded_prog = expand(program);
            let expanded_args: Vec<String> = arguments.iter().map(|v| expand(v)).collect();
            let expanded_cwd = expand(cwd);
            for path in paths {
                let expanded = expand(path);
                if let Some(rel) = normalize_rel_path(root, Path::new(&expanded))
                    && !extra_files.contains(&rel)
                {
                    extra_files.push(rel);
                }
            }
            if let Some(rel_cwd) = normalize_rel_path(root, Path::new(&expanded_cwd))
                && rel_cwd != "."
            {
                return ClosureScope {
                    kind: ScopeKind::ScopedDirs,
                    dirs: vec![rel_cwd],
                    globs: vec![],
                    extra_files,
                    git_base: None,
                    ir_revision_hint,
                    graph_ir_db,
                };
            }
            if expanded_prog == "cargo" {
                let manifest_arg = expanded_args
                    .windows(2)
                    .find(|w| w[0] == "--manifest-path")
                    .and_then(|w| normalize_rel_path(root, Path::new(&w[1])));
                let pkg_arg = expanded_args
                    .windows(2)
                    .find(|w| w[0] == "--package" || w[0] == "-p")
                    .map(|w| w[1].as_str());
                if let (Some(manifest_rel), Some(pkg)) = (manifest_arg.as_deref(), pkg_arg)
                    && let Some((dirs, pkg_extra)) = cargo_package_scope(root, manifest_rel, pkg)
                {
                    for item in pkg_extra {
                        if !extra_files.contains(&item) {
                            extra_files.push(item);
                        }
                    }
                    return ClosureScope {
                        kind: ScopeKind::ScopedDirs,
                        dirs,
                        globs: vec![],
                        extra_files,
                        git_base: None,
                        ir_revision_hint,
                        graph_ir_db,
                    };
                }
                if expanded_args.first().map(String::as_str) == Some("fmt")
                    && let Some(manifest_rel) = manifest_arg
                {
                    let manifest_dir = Path::new(&manifest_rel)
                        .parent()
                        .and_then(|p| p.to_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or(".");
                    extra_files.push("rustfmt.toml".into());
                    extra_files.push(".rustfmt.toml".into());
                    return ClosureScope {
                        kind: ScopeKind::ScopedDirs,
                        dirs: vec![manifest_dir.to_string()],
                        globs: vec![],
                        extra_files,
                        git_base: None,
                        ir_revision_hint,
                        graph_ir_db,
                    };
                }
            }
            ClosureScope {
                kind: ScopeKind::FullRepo,
                dirs: vec![],
                globs: vec![],
                extra_files,
                git_base: None,
                ir_revision_hint,
                graph_ir_db,
            }
        }
    }
}

fn should_skip_walk_entry(name: &str, inside_rust: bool) -> bool {
    matches!(
        name,
        ".git" | "target" | "node_modules" | ".phpunit.cache" | ".c2p"
    ) || (!inside_rust && name == "vendor")
}

fn path_has_ignored_component(rel: &str) -> bool {
    let mut parts = rel.split('/').peekable();
    let first = parts.peek().copied();
    let inside_rust = first == Some("runtime-rust");
    rel.split('/')
        .any(|part| should_skip_walk_entry(part, inside_rust))
}

fn stat_file_entry(path: &Path) -> Option<(u64, i128, i128, u32)> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink()
        || safe::linked_at(path, &meta).unwrap_or(true)
        || !meta.is_file()
    {
        return None;
    }
    let mtime = meta.modified().ok().and_then(time_to_unix_ns)?;
    #[cfg(unix)]
    let (ctime, mode) = {
        use std::os::unix::fs::MetadataExt;
        (
            i128::from(meta.ctime()) * 1_000_000_000 + i128::from(meta.ctime_nsec()),
            meta.mode(),
        )
    };
    #[cfg(not(unix))]
    let (ctime, mode) = (mtime, 0u32);
    Some((meta.len(), mtime, ctime, mode))
}

fn git_head_cue(frame: &infra_core::git::GitFrame, max_mtime_ns: &mut i128) -> String {
    let head_path = frame.admin.join("HEAD");
    if let Ok(meta) = std::fs::symlink_metadata(&head_path)
        && let Ok(modified) = meta.modified()
        && let Some(mtime) = time_to_unix_ns(modified)
    {
        *max_mtime_ns = (*max_mtime_ns).max(mtime);
    }
    let head_txt = std::fs::read_to_string(&head_path).unwrap_or_default();
    let trimmed = head_txt.trim();
    let resolved = if let Some(ref_name) = trimmed.strip_prefix("ref:") {
        let ref_rel = ref_name.trim();
        frame
            .common_dir()
            .ok()
            .and_then(|common| {
                let ref_path = common.join(ref_rel);
                if let Ok(meta) = std::fs::symlink_metadata(&ref_path)
                    && let Ok(modified) = meta.modified()
                    && let Some(mtime) = time_to_unix_ns(modified)
                {
                    *max_mtime_ns = (*max_mtime_ns).max(mtime);
                }
                std::fs::read_to_string(ref_path).ok()
            })
            .unwrap_or_default()
    } else {
        String::new()
    };
    format!("git_head:{trimmed}:{}", resolved.trim())
}

fn graph_ir_db_cue(db_path: &Path, max_mtime_ns: &mut i128) -> Option<String> {
    use std::io::Read;
    let meta = std::fs::symlink_metadata(db_path).ok()?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return None;
    }
    let mtime = meta.modified().ok().and_then(time_to_unix_ns)?;
    *max_mtime_ns = (*max_mtime_ns).max(mtime);
    let mut header = [0u8; 100];
    let header_cue = if meta.len() >= 100
        && let Ok(mut file) = std::fs::File::open(db_path)
        && file.read_exact(&mut header).is_ok()
    {
        let change_counter = u32::from_be_bytes([header[24], header[25], header[26], header[27]]);
        let valid_for = u32::from_be_bytes([header[92], header[93], header[94], header[95]]);
        format!("{change_counter}:{valid_for}")
    } else {
        "nohdr".into()
    };
    let wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
    let wal_cue = if let Ok(wal_meta) = std::fs::symlink_metadata(&wal_path)
        && wal_meta.is_file()
        && let Ok(wal_mod) = wal_meta.modified()
        && let Some(wal_mtime) = time_to_unix_ns(wal_mod)
    {
        *max_mtime_ns = (*max_mtime_ns).max(wal_mtime);
        format!("{}:{wal_mtime}", wal_meta.len())
    } else {
        "none".into()
    };
    Some(format!(
        "graph_ir_db:{}:{mtime}:{header_cue}:{wal_cue}",
        meta.len()
    ))
}

fn walk_dir_closure(
    root: &Path,
    start_dir: &Path,
    inside_rust: bool,
    entries: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
    max_mtime_ns: &mut i128,
) -> Option<()> {
    let start_meta = std::fs::symlink_metadata(start_dir).ok()?;
    if start_meta.file_type().is_symlink() {
        return None;
    }
    if let Ok(modified) = start_meta.modified()
        && let Some(mtime) = time_to_unix_ns(modified)
    {
        *max_mtime_ns = (*max_mtime_ns).max(mtime);
    }
    if start_meta.is_file() {
        let (len, mtime, ctime, mode) = stat_file_entry(start_dir)?;
        *max_mtime_ns = (*max_mtime_ns).max(mtime).max(ctime);
        let rel = start_dir
            .strip_prefix(root)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/");
        if seen.insert(rel.clone()) {
            entries.push(format!("{rel}:{len}:{mtime}:{ctime}:{mode}"));
            if entries.len() >= MAX_CLOSURE_FILES {
                return None;
            }
        }
        return Some(());
    }
    if !start_meta.is_dir() {
        return None;
    }
    let mut queue = VecDeque::from([start_dir.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        let mut children: Vec<_> = std::fs::read_dir(&dir)
            .ok()?
            .filter_map(|e| e.ok())
            .collect();
        children.sort_by_key(|e| e.file_name());
        for entry in children {
            let name = entry.file_name();
            let name_s = name.to_string_lossy();
            let child_inside_rust = inside_rust || name_s.as_ref() == "runtime-rust";
            if should_skip_walk_entry(name_s.as_ref(), child_inside_rust) {
                continue;
            }
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path).ok()?;
            if meta.file_type().is_symlink() {
                return None;
            }
            if meta.is_dir() {
                if let Ok(modified) = meta.modified()
                    && let Some(mtime) = time_to_unix_ns(modified)
                {
                    *max_mtime_ns = (*max_mtime_ns).max(mtime);
                }
                queue.push_back(path);
            } else if meta.is_file() {
                let (len, mtime, ctime, mode) = stat_file_entry(&path)?;
                *max_mtime_ns = (*max_mtime_ns).max(mtime).max(ctime);
                let rel = path
                    .strip_prefix(root)
                    .ok()?
                    .to_string_lossy()
                    .replace('\\', "/");
                if seen.insert(rel.clone()) {
                    entries.push(format!("{rel}:{len}:{mtime}:{ctime}:{mode}"));
                    if entries.len() >= MAX_CLOSURE_FILES {
                        return None;
                    }
                }
            }
        }
    }
    Some(())
}

fn git_status_entries(
    root: &Path,
    frame: &infra_core::git::GitFrame,
    pathspecs: &[String],
    include_untracked: bool,
    entries: &mut Vec<String>,
    max_mtime_ns: &mut i128,
) -> Option<()> {
    let mut args: Vec<String> = vec![
        "status".into(),
        "--porcelain=v1".into(),
        "-z".into(),
        if include_untracked {
            "-uall".into()
        } else {
            "-uno".into()
        },
    ];
    if !pathspecs.is_empty() {
        args.push("--".into());
        args.extend(pathspecs.iter().cloned());
    }
    let raw = common::git_ok(
        root,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        None,
    )
    .ok()?;
    let mut dirty_count = 0usize;
    let mut parts = raw.split(|byte| *byte == 0);
    while let Some(item) = parts.next() {
        if item.is_empty() {
            continue;
        }
        if item.len() < 4 {
            return None;
        }
        let status = std::str::from_utf8(&item[..2]).ok()?;
        let from_top = std::str::from_utf8(&item[3..]).ok()?;
        if status.starts_with('R')
            || status.starts_with('C')
            || status.ends_with('R')
            || status.ends_with('C')
        {
            let _ = parts.next();
        }
        let rel = from_top.strip_prefix(&frame.prefix).unwrap_or(from_top);
        if path_has_ignored_component(rel) {
            continue;
        }
        dirty_count += 1;
        if dirty_count >= MAX_CLOSURE_FILES {
            return None;
        }
        let target = frame.top.join(from_top);
        match std::fs::symlink_metadata(&target) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return None;
                }
                if meta.is_file() {
                    let (len, mtime, ctime, mode) = stat_file_entry(&target)?;
                    *max_mtime_ns = (*max_mtime_ns).max(mtime).max(ctime);
                    entries.push(format!(
                        "git_dirty:{status}:{rel}:{len}:{mtime}:{ctime}:{mode}"
                    ));
                } else {
                    if let Ok(modified) = meta.modified()
                        && let Some(mtime) = time_to_unix_ns(modified)
                    {
                        *max_mtime_ns = (*max_mtime_ns).max(mtime);
                    }
                    entries.push(format!("git_dirty:{status}:{rel}:dir"));
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                entries.push(format!("git_dirty:{status}:{rel}:absent"));
            }
            Err(_) => return None,
        }
    }
    Some(())
}

fn record_git_index_stamp(
    frame: &infra_core::git::GitFrame,
    include_digest: bool,
    entries: &mut Vec<String>,
    max_mtime_ns: &mut i128,
) -> Option<()> {
    let index_path = frame.admin.join("index");
    match std::fs::symlink_metadata(&index_path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() || !meta.is_file() {
                return None;
            }
            let (len, mtime, ctime, mode) = stat_file_entry(&index_path)?;
            *max_mtime_ns = (*max_mtime_ns).max(mtime).max(ctime);
            if include_digest {
                let bytes = safe::read_regular(&index_path).ok()?;
                entries.push(format!(
                    "git_index:{}:{len}:{mtime}:{ctime}:{mode}",
                    common::sha256(&bytes)
                ));
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if include_digest {
                entries.push("git_index:absent".into());
            }
        }
        Err(_) => return None,
    }
    Some(())
}

fn compute_closure_stamp(root: &Path, scope: &ClosureScope) -> Option<(String, i128)> {
    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();
    let mut max_mtime_ns = 0i128;
    let git_frame = infra_core::git::GitFrame::discover(root).ok().flatten();

    if let Some(rev) = scope.ir_revision_hint {
        entries.push(format!("ir_rev:{rev}"));
    }
    if let Some(db_path) = &scope.graph_ir_db
        && let Some(cue) = graph_ir_db_cue(db_path, &mut max_mtime_ns)
    {
        entries.push(cue);
    }

    match scope.kind {
        ScopeKind::ScopedDirs => {
            if let Some(frame) = &git_frame {
                entries.push(git_head_cue(frame, &mut max_mtime_ns));
                if !scope.dirs.is_empty() {
                    let mut ls_args = vec!["ls-files", "--stage", "-z", "--"];
                    ls_args.extend(scope.dirs.iter().map(String::as_str));
                    if let Ok(stage_raw) = common::git_ok(root, &ls_args, None) {
                        entries.push(format!("git_stage:{}", common::sha256(&stage_raw)));
                    }
                    record_git_index_stamp(frame, false, &mut entries, &mut max_mtime_ns)?;
                }
            }
            let mut walked_ok = true;
            let mut snapshot = entries.clone();
            let mut snapshot_mtime = max_mtime_ns;
            for dir_rel in &scope.dirs {
                let target = root.join(dir_rel);
                if target.exists() {
                    let inside_rust =
                        dir_rel == "runtime-rust" || dir_rel.starts_with("runtime-rust/");
                    if walk_dir_closure(
                        root,
                        &target,
                        inside_rust,
                        &mut entries,
                        &mut seen,
                        &mut max_mtime_ns,
                    )
                    .is_none()
                    {
                        walked_ok = false;
                        break;
                    }
                } else {
                    entries.push(format!("dir:{dir_rel}:absent"));
                }
            }
            if !walked_ok {
                let frame = git_frame.as_ref()?;
                entries = std::mem::take(&mut snapshot);
                max_mtime_ns = snapshot_mtime;
                git_status_entries(
                    root,
                    frame,
                    &scope.dirs,
                    true,
                    &mut entries,
                    &mut max_mtime_ns,
                )?;
                let _ = &mut snapshot_mtime;
            }
        }
        ScopeKind::GitTrackedGlobs => {
            if let Some(frame) = &git_frame {
                entries.push(git_head_cue(frame, &mut max_mtime_ns));
                record_git_index_stamp(frame, false, &mut entries, &mut max_mtime_ns)?;
                let mut ls_args = vec!["ls-files", "--stage", "-z", "--"];
                ls_args.extend(scope.globs.iter().map(String::as_str));
                let stage_raw = common::git_ok(root, &ls_args, None).ok()?;
                entries.push(format!("git_stage:{}", common::sha256(&stage_raw)));
                git_status_entries(
                    root,
                    frame,
                    &scope.globs,
                    false,
                    &mut entries,
                    &mut max_mtime_ns,
                )?;
            } else {
                walk_dir_closure(
                    root,
                    root,
                    false,
                    &mut entries,
                    &mut seen,
                    &mut max_mtime_ns,
                )?;
            }
        }
        ScopeKind::FullRepo => {
            if let Ok(meta) = root.metadata()
                && let Ok(modified) = meta.modified()
                && let Some(mtime) = time_to_unix_ns(modified)
            {
                max_mtime_ns = max_mtime_ns.max(mtime);
            }
            if let Some(frame) = &git_frame {
                entries.push(git_head_cue(frame, &mut max_mtime_ns));
                if let Some(base) = &scope.git_base {
                    let rev = common::git_ok(root, &["rev-parse", "--verify", base], None)
                        .map(|raw| String::from_utf8_lossy(&raw).trim().to_string())
                        .unwrap_or_else(|_| "missing".into());
                    entries.push(format!("git_base:{base}:{rev}"));
                }
                record_git_index_stamp(frame, true, &mut entries, &mut max_mtime_ns)?;
                git_status_entries(root, frame, &[], true, &mut entries, &mut max_mtime_ns)?;
            } else {
                walk_dir_closure(
                    root,
                    root,
                    false,
                    &mut entries,
                    &mut seen,
                    &mut max_mtime_ns,
                )?;
            }
        }
    }

    for rel in &scope.extra_files {
        if !seen.contains(rel) {
            let candidate = root.join(rel);
            match std::fs::symlink_metadata(&candidate) {
                Ok(meta) => {
                    if meta.file_type().is_symlink() {
                        return None;
                    }
                    if meta.is_file() {
                        let (len, mtime, ctime, mode) = stat_file_entry(&candidate)?;
                        max_mtime_ns = max_mtime_ns.max(mtime).max(ctime);
                        entries.push(format!("extra:{rel}:{len}:{mtime}:{ctime}:{mode}"));
                    } else {
                        entries.push(format!("extra:{rel}:nonfile"));
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    entries.push(format!("extra:{rel}:absent"));
                }
                Err(_) => return None,
            }
        }
    }

    entries.sort();
    Some((common::sha256(entries.join("\n").as_bytes()), max_mtime_ns))
}

#[cfg(test)]
fn input_closure_stamp(root: &Path, extra_paths: &[String]) -> Option<(String, i128)> {
    let (ir_revision_hint, graph_ir_db) = discover_graph_ir_cue(root, &json!({}));
    compute_closure_stamp(
        root,
        &ClosureScope {
            kind: ScopeKind::FullRepo,
            dirs: vec![],
            globs: vec![],
            extra_files: extra_paths.to_vec(),
            git_base: None,
            ir_revision_hint,
            graph_ir_db,
        },
    )
}

fn gate_receipt_key(
    context: &GateContext<'_>,
    gate_spec: &Gate,
    closure_digest: &str,
) -> Option<String> {
    let GateContext {
        root,
        base,
        group,
        strict,
        no_cache,
        oracle_allowed: _,
        logs: _,
        cancel: _,
    } = context;
    if *strict || *no_cache {
        return None;
    }
    match gate_spec {
        Gate::Tool {
            name,
            operation,
            request,
        } => {
            let interpolated = interpolate(request, root, base);
            let tool_stamp = if operation == "php-snippets" {
                resolve_tool_binary_stamp("php", root)
            } else {
                None
            };
            Some(common::sha256(
                json!({
                    "kind": "tool",
                    "group": group.name,
                    "name": name,
                    "operation": operation,
                    "request": interpolated,
                    "tool": tool_stamp,
                    "base": base,
                    "closure": closure_digest,
                })
                .to_string()
                .as_bytes(),
            ))
        }
        Gate::Process {
            name,
            program,
            arguments,
            cwd,
            tools,
            fallback,
            paths,
            timeout_seconds: _,
            oracle,
        } => {
            if *oracle {
                return None;
            }
            let expand = |value: &str| process_argument(value, root, base, &group.rust_toolchain);
            let expanded_prog = expand(program);
            let expanded_args: Vec<String> = arguments.iter().map(|v| expand(v)).collect();
            let expanded_cwd = expand(cwd);
            let tool_stamp = resolve_tool_binary_stamp(&expanded_prog, root).unwrap_or_default();
            let required_tools: Vec<(String, Option<String>)> = tools
                .iter()
                .map(|tool| (tool.clone(), resolve_tool_binary_stamp(tool, root)))
                .collect();
            let fallback_stamp = fallback.as_ref().map(|fb| {
                let fb_prog = expand(&fb.program);
                let fb_args: Vec<String> = fb.arguments.iter().map(|v| expand(v)).collect();
                (
                    fb_prog.clone(),
                    fb_args,
                    resolve_tool_binary_stamp(&fb_prog, root),
                )
            });
            Some(common::sha256(
                json!({
                    "kind": "process",
                    "group": group.name,
                    "name": name,
                    "program": expanded_prog,
                    "arguments": expanded_args,
                    "cwd": expanded_cwd,
                    "paths": paths,
                    "tool": tool_stamp,
                    "tools": required_tools,
                    "fallback": fallback_stamp,
                    "toolchain": group.rust_toolchain,
                    "base": base,
                    "closure": closure_digest,
                })
                .to_string()
                .as_bytes(),
            ))
        }
    }
}

#[allow(dead_code)]
fn gate(context: GateContext<'_>, gate: &Gate) -> Outcome {
    gate_with_receipts(context, gate, None, None, now_unix_ns())
}

fn gate_with_receipts(
    context: GateContext<'_>,
    gate: &Gate,
    receipt_store: Option<&Arc<Mutex<CheckReceiptStore>>>,
    closure: Option<&(String, i128)>,
    now_ns: i128,
) -> Outcome {
    let gate_cache_key = if let (Some(store), Some((closure_digest, max_mtime))) =
        (receipt_store, closure)
        && !context.strict
        && !context.no_cache
        && *max_mtime > 0
        && now_ns > CHECKER_RACY_NS
        && *max_mtime <= now_ns - CHECKER_RACY_NS
    {
        if let Some(key) = gate_receipt_key(&context, gate, closure_digest) {
            if let Ok(mut guard) = store.lock()
                && guard.entries.contains_key(&key)
            {
                let seq = guard.next_seq;
                guard.next_seq = guard.next_seq.saturating_add(1);
                if let Some(entry) = guard.entries.get_mut(&key) {
                    entry.last_used = seq;
                }
                return Outcome {
                    group: context.group.name.clone(),
                    name: gate.name().into(),
                    status: "pass".into(),
                    seconds: 0.0,
                    code: Some(0),
                    reason: Some("cached local receipt".into()),
                    log: None,
                    excerpt: Vec::new(),
                };
            }
            Some(key)
        } else {
            None
        }
    } else {
        None
    };

    let GateContext {
        root,
        base,
        group,
        strict,
        no_cache,
        oracle_allowed,
        logs,
        cancel,
    } = context;
    let started = Instant::now();
    let mut outcome = Outcome {
        group: group.name.clone(),
        name: gate.name().into(),
        status: "pass".into(),
        seconds: 0.0,
        code: Some(0),
        reason: None,
        log: None,
        excerpt: Vec::new(),
    };
    let executed = (|| -> Result<(Option<i32>, Vec<u8>)> {
        if cancel.load(Ordering::Acquire) || INTERRUPTED.load(Ordering::Acquire) {
            outcome.status = "cancelled".into();
            return Ok((None, vec![]));
        }
        if group
            .platform
            .as_ref()
            .is_some_and(|platform| platform != std::env::consts::OS)
        {
            outcome.status = "skipped".into();
            outcome.reason = Some("Platform-specific check".into());
            return Ok((None, vec![]));
        }
        match gate {
            Gate::Tool {
                operation, request, ..
            } => {
                let mut request = interpolate(request, root, base);
                if request.is_null() {
                    request = json!({});
                }
                request
                    .as_object_mut()
                    .ok_or_else(|| RuntimeError::invalid("Tool check request must be an object"))?
                    .insert("root".into(), json!(root));
                let bypass_inner_caches = strict || no_cache;
                let report = process::with_cancel(cancel.clone(), || {
                    crate::php::with_cache_bypass(bypass_inner_caches, || {
                        crate::mirrors::check_receipts::with_cache_bypass(
                            bypass_inner_caches,
                            || crate::execute(operation, &request),
                        )
                    })
                })?;
                if report["status"] == "skipped" {
                    outcome.status = "skipped".into();
                    outcome.reason = Some(
                        report["reason"]
                            .as_str()
                            .unwrap_or("Explicitly skipped")
                            .into(),
                    );
                } else {
                    let code = crate::dispatch::report_exit_code(&report);
                    if code != 0 {
                        outcome.status = "fail".into();
                        outcome.code = Some(code);
                    }
                }
                let bytes = common::json_bytes(&report)?;
                if outcome.status == "fail" {
                    outcome.excerpt = excerpt(&[("report", bytes.as_slice())]);
                }
                Ok((outcome.code, bytes))
            }
            Gate::Process {
                program,
                arguments,
                cwd,
                tools,
                fallback,
                paths,
                timeout_seconds,
                oracle,
                ..
            } => {
                if *oracle && !oracle_allowed {
                    outcome.status = "skipped".into();
                    outcome.reason = Some("Legacy oracle was not explicitly requested".into());
                    return Ok((None, vec![]));
                }
                let expand =
                    |value: &str| process_argument(value, root, base, &group.rust_toolchain);
                for path in paths {
                    if !root.join(expand(path)).exists() {
                        outcome.status = if strict { "fail" } else { "skipped" }.into();
                        outcome.reason = Some(format!("Required check file is absent: {path}"));
                        return Ok((if strict { Some(1) } else { None }, vec![]));
                    }
                }
                let mut program = expand(program);
                let mut arguments = arguments;
                let missing = std::iter::once(program.as_str())
                    .chain(tools.iter().map(String::as_str))
                    .find(|tool| !available(tool, root))
                    .map(str::to_string);
                if let Some(tool) = missing {
                    match fallback {
                        Some(fallback) if available(&expand(&fallback.program), root) => {
                            outcome.reason =
                                Some(format!("{tool} is unavailable; ran the fallback"));
                            program = expand(&fallback.program);
                            arguments = &fallback.arguments;
                        }
                        _ => {
                            outcome.status = if strict { "fail" } else { "skipped" }.into();
                            outcome.reason =
                                Some(format!("Required executable is unavailable: {tool}"));
                            return Ok((if strict { Some(1) } else { None }, vec![]));
                        }
                    }
                }
                let arguments = arguments
                    .iter()
                    .map(|value| OsString::from(expand(value)))
                    .collect::<Vec<_>>();
                let cwd = root.join(expand(cwd));
                let output = process::with_cancel(cancel, || {
                    process::run(
                        std::ffi::OsStr::new(&program),
                        &arguments,
                        &cwd,
                        None,
                        Duration::from_secs(*timeout_seconds),
                        16 * 1024 * 1024,
                    )
                })?;
                if output.code != 0 {
                    outcome.status = "fail".into();
                    // Separately: a test harness prints its failures to stdout,
                    // while Cargo names the failed target on stderr.
                    outcome.excerpt = excerpt(&[
                        ("stdout", output.stdout.as_slice()),
                        ("stderr", output.stderr.as_slice()),
                    ]);
                }
                let mut bytes = output.stdout;
                bytes.extend_from_slice(&output.stderr);
                Ok((Some(output.code), bytes))
            }
        }
    })();
    let (code, bytes) = match executed {
        Ok(value) => value,
        Err(error) => {
            outcome.status = if INTERRUPTED.load(Ordering::Acquire) {
                "cancelled"
            } else {
                "fail"
            }
            .into();
            outcome.reason = Some(error.to_string());
            (Some(1), error.to_string().into_bytes())
        }
    };
    outcome.code = code;
    outcome.seconds = started.elapsed().as_secs_f64();
    if outcome.status == "fail" {
        let path = logs.join(format!(
            "{}-{}.log",
            group.name,
            uuid::Uuid::new_v4().simple()
        ));
        if safe::atomic_write(&path, &bytes).is_ok() {
            outcome.log = Some(path);
        }
    } else if outcome.status == "pass"
        && let (Some(store), Some(key)) = (receipt_store, gate_cache_key)
        && let Ok(mut guard) = store.lock()
    {
        let seq = guard.next_seq;
        guard.next_seq = guard.next_seq.saturating_add(1);
        guard.entries.insert(
            key,
            CheckReceiptEntry {
                group: group.name.clone(),
                gate: gate.name().into(),
                cached_at: infra_core::domain::utc_now(),
                last_used: seq,
            },
        );
        evict_oldest_receipts(&mut guard);
        guard.dirty = true;
    }
    outcome
}
pub fn execute(request: &Value) -> Result<Value> {
    execute_at(request, now_unix_ns())
}

fn execute_at(request: &Value, now_ns: i128) -> Result<Value> {
    let root = infra_core::fs::canonicalize(request["root"].as_str().unwrap_or("."))?;
    let catalog = load_catalog(&root, true)?;
    let names = request["groups"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(String::from)
                        .ok_or_else(|| RuntimeError::invalid("Check groups must be strings"))
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    for name in &names {
        if !catalog.groups.iter().any(|group| &group.name == name) {
            return Err(RuntimeError::invalid(format!(
                "Unknown native check group: {name}"
            )));
        }
    }
    let oracle = request["include_oracles"].as_bool().unwrap_or(false);
    let groups = catalog
        .groups
        .into_iter()
        .filter(|group| {
            if names.is_empty() {
                group.default
            } else {
                names.contains(&group.name)
            }
        })
        .collect::<Vec<_>>();
    if request["list"].as_bool().unwrap_or(false) {
        return Ok(
            json!({"groups":groups.iter().map(|group|json!({"name":group.name,"workflow":group.workflow,"job":group.job,"timeout_minutes":group.timeout_minutes,"gates":group.gates.iter().map(|gate|gate.name()).collect::<Vec<_>>()})).collect::<Vec<_>>()}),
        );
    }
    let requested_jobs = request["jobs"].as_u64().unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|value| (value.get() / 2).max(2) as u64)
            .unwrap_or(2)
    });
    if requested_jobs == 0 {
        return Err(RuntimeError::invalid("Check worker count must be positive"));
    }
    let jobs = requested_jobs.min(groups.len().max(1) as u64) as usize;
    let strict = request["strict"].as_bool().unwrap_or(false);
    let fail_fast = request["fail_fast"].as_bool().unwrap_or(false);
    let base = request["base"]
        .as_str()
        .unwrap_or("origin/main")
        .to_string();
    let (mut temporary, logs) = match request["log_dir"].as_str() {
        // A named directory keeps failed logs where a CI job can collect them.
        Some(directory) => {
            let directory = PathBuf::from(directory);
            safe::ensure_directory(&directory, false)?;
            (None, directory)
        }
        None => {
            let temporary = Temporary::new("infra-check")?;
            let logs = temporary.path().to_path_buf();
            (Some(temporary), logs)
        }
    };
    let no_cache = strict
        || request["no_cache"] == true
        || request["cache"] == false
        || std::env::var("INFRA_CHECK_NO_CACHE").as_deref() == Ok("1")
        || std::env::var("INFRA_NO_CACHE").as_deref() == Ok("1");
    let use_cache = !no_cache
        && (request["cache"] == true || std::env::var("INFRA_CHECK_CACHE").as_deref() == Ok("1"));
    let receipt_path = use_cache.then(|| {
        request["cache_file"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                infra_core::git::GitFrame::discover(&root)
                    .ok()
                    .flatten()
                    .map(|frame| frame.admin.join("infra-check-receipts.json"))
            })
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!(
                    "infra-check-receipts-{}.json",
                    common::sha256(root.to_string_lossy().as_bytes())
                ))
            })
    });
    let mut closure_stamps =
        std::collections::BTreeMap::<ClosureScope, Option<(String, i128)>>::new();
    if use_cache {
        for group in &groups {
            for gate_spec in &group.gates {
                let scope = scope_for_gate(&root, request, &base, group, gate_spec);
                closure_stamps
                    .entry(scope.clone())
                    .or_insert_with(|| compute_closure_stamp(&root, &scope));
            }
        }
    }
    let receipt_store = receipt_path
        .as_ref()
        .filter(|_| closure_stamps.values().any(Option::is_some))
        .map(|path| Arc::new(Mutex::new(load_receipt_store(path))));

    let queue = Arc::new(Mutex::new(VecDeque::from(groups)));
    let cancelled = Arc::new(AtomicBool::new(false));
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let started = Instant::now();
    INTERRUPTED.store(false, Ordering::Release);
    #[cfg(unix)]
    let signals = unsafe {
        [
            (
                libc::SIGINT,
                libc::signal(libc::SIGINT, interrupted as *const () as libc::sighandler_t),
            ),
            (
                libc::SIGTERM,
                libc::signal(
                    libc::SIGTERM,
                    interrupted as *const () as libc::sighandler_t,
                ),
            ),
            (
                libc::SIGHUP,
                libc::signal(libc::SIGHUP, interrupted as *const () as libc::sighandler_t),
            ),
        ]
    };
    #[cfg(windows)]
    if unsafe { SetConsoleCtrlHandler(Some(console_interrupted), 1) } == 0 {
        return Err(RuntimeError::unavailable(
            "Native check cancellation handler unavailable",
        ));
    }
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            let queue = queue.clone();
            let cancelled = cancelled.clone();
            let outcomes = outcomes.clone();
            let root = &root;
            let base = &base;
            let logs = &logs;
            let receipt_store = receipt_store.as_ref();
            let closure_stamps = &closure_stamps;
            scope.spawn(move || {
                loop {
                    if INTERRUPTED.load(Ordering::Acquire) {
                        cancelled.store(true, Ordering::Release);
                    }
                    let group = queue.lock().ok().and_then(|mut queue| queue.pop_front());
                    let Some(group) = group else {
                        break;
                    };
                    for check in &group.gates {
                        let gate_closure = if use_cache {
                            let gate_scope = scope_for_gate(root, request, base, &group, check);
                            closure_stamps.get(&gate_scope).and_then(Option::as_ref)
                        } else {
                            None
                        };
                        let result = gate_with_receipts(
                            GateContext {
                                root,
                                base,
                                group: &group,
                                strict,
                                no_cache,
                                oracle_allowed: oracle,
                                logs,
                                cancel: cancelled.clone(),
                            },
                            check,
                            receipt_store,
                            gate_closure,
                            now_ns,
                        );
                        if fail_fast && result.status == "fail" {
                            cancelled.store(true, Ordering::Release);
                        }
                        if let Ok(mut output) = outcomes.lock() {
                            output.push(result);
                        }
                    }
                }
            });
        }
    });
    if let (Some(path), Some(store)) = (receipt_path.as_ref(), receipt_store.as_ref())
        && let Ok(guard) = store.lock()
        && guard.dirty
    {
        save_receipt_store(path, &guard);
    }
    #[cfg(unix)]
    unsafe {
        for (signal, previous) in signals {
            libc::signal(signal, previous);
        }
    }
    #[cfg(windows)]
    unsafe {
        SetConsoleCtrlHandler(Some(console_interrupted), 0);
    }
    let mut results = outcomes
        .lock()
        .map_err(|_| RuntimeError::unavailable("Check result collector was poisoned"))?
        .clone();
    results.sort_by(|a, b| (&a.group, &a.name).cmp(&(&b.group, &b.name)));
    let failed = results
        .iter()
        .filter(|result| result.status == "fail")
        .count();
    let skipped = results
        .iter()
        .filter(|result| result.status == "skipped")
        .count();
    let passed = results
        .iter()
        .filter(|result| result.status == "pass")
        .count();
    let interrupted = INTERRUPTED.load(Ordering::Acquire);
    let code = if interrupted {
        130
    } else if failed > 0 {
        1
    } else {
        0
    };
    let retained = if failed > 0 {
        Some(
            temporary
                .as_mut()
                .map_or_else(|| logs.clone(), Temporary::preserve),
        )
    } else {
        None
    };
    Ok(
        json!({"_cli":{"code":code,"stdout":serde_json::to_string_pretty(&json!({"passed":passed,"failed":failed,"skipped":skipped,"interrupted":interrupted,"results":results,"logs":retained,"wall_seconds":started.elapsed().as_secs_f64()}))?+"\n","stderr":failure_report(&results)},"result":{"exit_code":code,"passed":passed,"failed":failed,"skipped":skipped,"interrupted":interrupted,"results":results,"logs":retained}}),
    )
}

// Syntax-only admission keeps json.tool's duplicate-key, nonfinite-number and
// escaped-surrogate acceptance without constructing runtime authority values.
fn validate_json_syntax(bytes: &[u8]) -> std::result::Result<(), String> {
    std::str::from_utf8(bytes).map_err(|error| format!("Invalid UTF-8: {error}"))?;
    let mut normalized = std::borrow::Cow::Borrowed(bytes);
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'"' {
            at += 1;
            while at < bytes.len() {
                match bytes[at] {
                    b'\\' => at += 2,
                    b'"' => {
                        at += 1;
                        break;
                    }
                    _ => at += 1,
                }
            }
        } else if b" \t\r\n,]}:[{".contains(&bytes[at]) {
            at += 1;
        } else {
            let start = at;
            while at < bytes.len() && !b" \t\r\n,]}:[{\"".contains(&bytes[at]) {
                at += 1;
            }
            if [b"NaN".as_slice(), b"Infinity", b"-Infinity"].contains(&&bytes[start..at]) {
                let output = normalized.to_mut();
                output[start] = b'0';
                output[start + 1..at].fill(b' ');
            }
        }
    }
    let mut parser = serde_json::Deserializer::from_slice(normalized.as_ref());
    serde::de::IgnoredAny::deserialize(&mut parser).map_err(|error| error.to_string())?;
    parser.end().map_err(|error| error.to_string())
}

/// Inspect the current worktree images of exactly Git's indexed JSON members.
/// The 512 MiB per-file ceiling is an explicit read admission bound, not a schema.
pub fn check_tracked_json(root: &Path) -> Result<Value> {
    const MAX_JSON_BYTES: usize = 512 * 1024 * 1024;
    let root = infra_core::fs::canonicalize(root)?;
    let names = common::tracked(&root, &["*.json".into()])?;
    let mut files = infra_core::bank::publication::PublicationFiles::new(&root)?;
    let mut findings = Vec::new();
    for name in &names {
        let result = (|| -> std::result::Result<(), String> {
            let (bytes, _) = files
                .capture_source(name, MAX_JSON_BYTES)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "Indexed JSON file is absent from the worktree".to_string())?;
            validate_json_syntax(&bytes)
        })();
        if let Err(message) = result {
            findings.push(json!({"path":name,"message":message}));
        }
    }
    files.check()?;
    Ok(json!({
        "passed":findings.is_empty(),
        "files":names.len(),
        "findings":findings,
        "syntax_only":true,
        "maximum_file_bytes":MAX_JSON_BYTES,
        "selection":"git ls-files --cached -- '*.json'"
    }))
}

fn yaml_string(value: &str) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}
fn workflow(catalog: &Catalog, filename: &str) -> Result<String> {
    let mut groups = catalog
        .groups
        .iter()
        .filter(|group| group.workflow == filename)
        .collect::<Vec<_>>();
    groups.sort_by(|a, b| a.job.cmp(&b.job));
    let mut output = String::from(
        "# Generated by infra-engine write-ci from install/check-catalog.json.\n# Edit the catalog or native emitter; regenerate through the native command.\nname: ",
    );
    output.push_str(&yaml_string(if filename == "rust.yml" {
        "Rust native contracts"
    } else {
        "Native repository checks"
    })?);
    output.push_str("\non:\n  push:\n  pull_request:\n  workflow_dispatch:\n    inputs:\n      legacy_oracles:\n        description: Run retained verification references\n        required: false\n        type: boolean\n        default: false\npermissions:\n  contents: read\njobs:\n");
    for group in groups {
        if group.job.is_empty()
            || !group
                .job
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        {
            return Err(RuntimeError::invalid(
                "Native CI job name is not a GitHub identifier",
            ));
        }
        let oracle = group
            .gates
            .iter()
            .any(|gate| matches!(gate, Gate::Process { oracle: true, .. }));
        if oracle && group.default {
            return Err(RuntimeError::policy("Legacy reference jobs must be opt-in"));
        }
        output.push_str(&format!("  {}:\n", group.job));
        if oracle {
            output.push_str("    if: github.event_name == 'workflow_dispatch' && inputs.legacy_oracles == true\n");
        }
        let platform = group.platform.as_deref().unwrap_or("linux");
        let runner = match platform {
            "linux" => "ubuntu-latest",
            "windows" => "windows-latest",
            "macos" => "macos-latest",
            _ => {
                return Err(RuntimeError::invalid(
                    "Unsupported native CI catalog platform",
                ));
            }
        };
        // Job-level env admits only github, needs, strategy, matrix, vars,
        // secrets and inputs; any runner reference there makes GitHub reject
        // the whole file before a job exists. The target directory is set by
        // a step below instead.
        output.push_str(&format!(
            "    runs-on: {runner}\n    timeout-minutes: {}\n    env:\n      NATIVE_CHECK_GROUP: ",
            group.timeout_minutes
        ));
        output.push_str(&yaml_string(&group.name)?);
        // Match the test profile's debuginfo without changing local dev builds
        // or the debug/ engine path used by the catalog bootstrap. A job
        // compiles each changed crate once, so incremental state would only
        // add writes and cache weight.
        output.push_str("\n      CARGO_PROFILE_DEV_DEBUG: \"0\"\n      CARGO_INCREMENTAL: \"0\"\n    defaults:\n      run:\n        shell: bash\n    steps:\n      - uses: actions/checkout@v4\n        with:\n          fetch-depth: 0\n      - uses: dtolnay/rust-toolchain@stable\n        id: native-toolchain\n        with:\n          components: rustfmt, clippy\n");
        validate_rust_configuration(group)?;
        if group.rust_toolchain != "stable" {
            output.push_str("          toolchain: ");
            output.push_str(&yaml_string(&group.rust_toolchain)?);
            output.push('\n');
        }
        if !group.rust_targets.is_empty() {
            output.push_str("          targets: ");
            output.push_str(&yaml_string(&group.rust_targets.join(", "))?);
            output.push('\n');
        }
        if oracle {
            output.push_str("      - uses: actions/setup-python@v5\n        with:\n          python-version: '3.x'\n");
        }
        if group.gates.iter().any(|gate|matches!(gate,Gate::Tool{operation,request,..}if operation=="php-snippets"&&request["require_php"]==true)){
   output.push_str("      - uses: shivammathur/setup-php@v2\n        with:\n          php-version: '8.3'\n          coverage: none\n          tools: none\n");
  }
        // A prebuilt, checksummed runner; its absence would only send the
        // tests through their sequential fallback, so a missing release fails.
        let uses_nextest = group.gates.iter().any(|gate| {
            matches!(gate, Gate::Process { tools, .. } if tools.iter().any(|tool| tool == "cargo-nextest"))
        });
        if uses_nextest {
            output.push_str(concat!(
                "      - uses: taiki-e/install-action@a4d3f3f761dcd95cce1b72cb61cb6d1963438177 # v2.87.30\n",
                "        with:\n",
                "          tool: cargo-nextest@0.9.149\n",
                "          fallback: none\n",
            ));
        }
        if platform == "linux" && uses_nextest {
            output.push_str(concat!(
                "      - name: Prepare native oracle isolation\n",
                "        run: |\n",
                "          sudo apt-get update\n",
                "          sudo apt-get install --yes bubblewrap\n",
                "          if [ -f /proc/sys/kernel/apparmor_restrict_unprivileged_userns ]; then\n",
                "            sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0\n",
                "          fi\n",
                "          bwrap --unshare-all --die-with-parent --ro-bind / / -- /usr/bin/true\n",
            ));
        }
        // RUNNER_TEMP is a backslash path on Windows; Cargo and Git Bash both
        // accept the forward-slash spelling.
        output.push_str("      - name: Select the native target directory\n        run: echo \"CARGO_TARGET_DIR=${RUNNER_TEMP//\\\\//}/infra-native-target\" >> \"$GITHUB_ENV\"\n");
        // The toolchain action exports CARGO_HOME and an identity of the actual
        // installed compiler. Keep jobs separate so a lightweight engine-only
        // job cannot occupy the immutable cache key of the complete test job.
        // Cached artifacts are inputs to Cargo, never authority to skip it.
        // A missed key starts from the job's newest entry for this compiler,
        // and Cargo rebuilds only what changed since. v2 entries hold no
        // incremental state, so no v1 entry is restored into them.
        let prefix = format!(
            "native-cargo-v2-${{{{ runner.os }}}}-${{{{ runner.arch }}}}-${{{{ steps.native-toolchain.outputs.cachekey }}}}-{}-dev0-",
            group.job
        );
        output.push_str(concat!(
            "      - name: Restore native Cargo artifacts\n",
            "        id: native-cache\n",
            "        uses: actions/cache/restore@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6\n",
            "        with:\n",
        ));
        output.push_str(CARGO_CACHE_PATHS);
        output.push_str("          key: ");
        output.push_str(&prefix);
        output.push_str("${{ hashFiles('runtime-rust/Cargo.lock', 'runtime-rust/**/Cargo.toml', 'install/check-catalog.json', '.cargo/**', 'runtime-rust/.cargo/**') }}\n          restore-keys: ");
        output.push_str(&prefix);
        output.push('\n');
        // The group's own Cargo builds rewrite this engine in the target
        // directory, and Windows cannot replace a running executable: the
        // catalog runs from a copy outside it.
        output.push_str("      - name: Build the native engine\n        run: ");
        if platform == "windows" && uses_nextest {
            // CLI integration tests build their real engine too. Compile once
            // with the workspace's test features instead of first linking the
            // narrower default-member engine and then rebuilding for tests.
            output.push_str("cargo nextest run --workspace --manifest-path runtime-rust/Cargo.toml --locked --no-run");
        } else {
            output.push_str(
                "cargo build --manifest-path runtime-rust/Cargo.toml --locked --bin infra-engine",
            );
        }
        output.push_str("\n      - name: Run the catalog group\n        run: |\n          suffix=''\n          if [ \"$RUNNER_OS\" = 'Windows' ]; then suffix='.exe'; fi\n          engine=\"${RUNNER_TEMP//\\\\//}/infra-check-engine$suffix\"\n          cp \"$CARGO_TARGET_DIR/debug/infra-engine$suffix\" \"$engine\"\n          \"$engine\" check --group \"$NATIVE_CHECK_GROUP\" --strict --base origin/main --log-dir \"${RUNNER_TEMP//\\\\//}/infra-check-logs\"");
        if oracle {
            output.push_str(" --include-oracles");
        }
        output.push('\n');
        // The run's own output shows only the end of each failed stream; the
        // job keeps every failed check's complete output.
        output.push_str(concat!(
            "      - name: Keep the complete output of failed checks\n",
            "        if: failure()\n",
            "        uses: actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2\n",
            "        with:\n",
            "          name: check-logs-${{ github.job }}\n",
            "          path: ${{ runner.temp }}/infra-check-logs\n",
            "          if-no-files-found: ignore\n",
            "          retention-days: 14\n",
        ));
        // A failed check still leaves the dependencies it compiled, and
        // keeping them spares the next run a cold build. An entry found under
        // its exact key is immutable and stays as it is.
        output.push_str(concat!(
            "      - name: Save native Cargo artifacts\n",
            "        if: ${{ !cancelled() && steps.native-cache.outputs.cache-hit != 'true' }}\n",
            "        uses: actions/cache/save@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6\n",
            "        with:\n",
        ));
        output.push_str(CARGO_CACHE_PATHS);
        output.push_str("          key: ${{ steps.native-cache.outputs.cache-primary-key }}\n");
    }
    Ok(output)
}
const CARGO_CACHE_PATHS: &str = concat!(
    "          path: |\n",
    "            ${{ env.CARGO_HOME }}/registry/index\n",
    "            ${{ env.CARGO_HOME }}/registry/cache\n",
    "            ${{ env.CARGO_HOME }}/git/db\n",
    "            ${{ env.CARGO_TARGET_DIR }}\n",
);
/// Emit only catalog-owned workflow files; unrelated workflows are untouched.
pub fn write_ci(root: &Path, write: bool) -> Result<Value> {
    let root = infra_core::fs::canonicalize(root)?;
    let catalog = load_catalog(&root, false)?;
    let mut names = BTreeSet::new();
    for group in &catalog.groups {
        let path = Path::new(&group.workflow);
        if path.components().count() != 1 || !group.workflow.ends_with(".yml") {
            return Err(RuntimeError::invalid(
                "Native catalog workflow must be one .yml filename",
            ));
        }
        names.insert(group.workflow.clone());
    }
    let mut files = infra_core::bank::publication::PublicationFiles::new(&root)?;
    let mut changed = Vec::new();
    let mut prepared = Vec::new();
    for name in names {
        let relative = format!(".github/workflows/{name}");
        let expected = workflow(&catalog, &name)?.into_bytes();
        let previous = files.read(&relative, 8 * 1024 * 1024)?;
        if previous.as_deref() != Some(expected.as_slice()) {
            changed.push(json!({"path":relative,"before_sha256":previous.as_ref().map(|bytes|common::sha256(bytes)),"after_sha256":common::sha256(&expected)}));
            prepared.push((relative, previous, expected));
        }
    }
    if write {
        for (relative, previous, expected) in prepared {
            let mode = files.read_mode(&relative)?.unwrap_or(0o644);
            files.write_expected_mode(
                &relative,
                &expected,
                previous.as_deref(),
                mode,
                8 * 1024 * 1024,
            )?;
        }
    }
    Ok(
        json!({"passed":write||changed.is_empty(),"written":write,"changed":changed,"catalog":"install/check-catalog.json"}),
    )
}

#[cfg(test)]
mod compiler_workflow_tests {
    use super::*;

    fn catalog(groups: Value) -> Catalog {
        serde_json::from_value(json!({"schema_version":1,"groups":groups})).unwrap()
    }
    fn group(name: &str, job: &str) -> Value {
        json!({
            "name":name,"workflow":"rust.yml","job":job,"timeout_minutes":15,
            "gates":[{"kind":"process","name":"native check","program":"cargo","arguments":["check"]}]
        })
    }

    #[test]
    fn workflow_selects_the_catalog_compiler_and_wasm_target_without_changing_stable_defaults() {
        let stable = group("rust", "rust-contract");
        let mut msrv = group("msrv", "rust-msrv");
        msrv["default"] = false.into();
        msrv["rust_toolchain"] = "1.89.0".into();
        msrv["gates"][0]["program"] = "rustup".into();
        msrv["gates"][0]["arguments"] = json!([
            "run",
            "{toolchain}",
            "cargo",
            "check",
            "--workspace",
            "--all-targets",
            "--locked"
        ]);
        let mut wasm = group("wasm", "rust-wasm");
        wasm["rust_targets"] = json!(["wasm32-unknown-unknown"]);
        let source = catalog(json!([stable, msrv, wasm]));
        let rendered = workflow(&source, "rust.yml").unwrap();
        let stable_job = rendered
            .split("  rust-contract:\n")
            .nth(1)
            .unwrap()
            .split("  rust-msrv:\n")
            .next()
            .unwrap();
        assert!(stable_job.contains("uses: dtolnay/rust-toolchain@stable"));
        assert!(!stable_job.contains("          toolchain:"));
        assert!(!stable_job.contains("          targets:"));
        let msrv_job = rendered
            .split("  rust-msrv:\n")
            .nth(1)
            .unwrap()
            .split("  rust-wasm:\n")
            .next()
            .unwrap();
        assert!(msrv_job.contains("          toolchain: \"1.89.0\"\n"));
        assert!(msrv_job.contains("NATIVE_CHECK_GROUP: \"msrv\""));
        let wasm_job = rendered.split("  rust-wasm:\n").nth(1).unwrap();
        assert!(wasm_job.contains("          targets: \"wasm32-unknown-unknown\"\n"));
        assert!(!wasm_job.contains("          toolchain:"));
        // The same catalog value selects the compiler for explicit local runs.
        assert_eq!(
            process_argument(
                "{toolchain}",
                Path::new("root"),
                "main",
                &source.groups[1].rust_toolchain
            ),
            "1.89.0"
        );
        assert_eq!(
            process_argument(
                "{root}/src",
                Path::new("root/{toolchain}"),
                "main",
                "1.89.0"
            ),
            "root/{toolchain}/src"
        );
    }

    #[test]
    fn invalid_compiler_or_target_metadata_is_refused_by_the_emitter() {
        for value in [
            "",
            "1.84.0",
            "1.89",
            "01.89.0",
            "1.89.0\nother",
            "stable; other",
            "nightly",
        ] {
            let mut bad = group("bad", "bad");
            bad["rust_toolchain"] = value.into();
            assert!(
                workflow(&catalog(json!([bad])), "rust.yml").is_err(),
                "{value:?}"
            );
        }
        for targets in [
            json!(["wasm32-unknown-unknown", "wasm32-unknown-unknown"]),
            json!(["wasm32-unknown-unknown\nother"]),
            json!([""]),
        ] {
            let mut bad = group("bad", "bad");
            bad["rust_targets"] = targets;
            assert!(workflow(&catalog(json!([bad])), "rust.yml").is_err());
        }
    }

    /// Context names used by `${{ }}` expressions in each job-level env block.
    fn job_env_contexts(rendered: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut inside = false;
        for line in rendered.lines() {
            if line == "    env:" {
                inside = true;
                continue;
            }
            if !line.starts_with("      ") {
                inside = false;
            }
            if !inside {
                continue;
            }
            for expression in line.split("${{").skip(1) {
                found.push(
                    expression
                        .trim_start()
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect(),
                );
            }
        }
        found
    }

    fn workflow_jobs(rendered: &str) -> Vec<String> {
        let mut jobs: Vec<String> = Vec::new();
        for line in rendered.split("\njobs:\n").nth(1).unwrap().lines() {
            if line.starts_with("  ") && !line.starts_with("   ") {
                jobs.push(String::new());
            }
            if let Some(job) = jobs.last_mut() {
                job.push_str(line);
                job.push('\n');
            }
        }
        jobs
    }

    #[test]
    fn job_level_env_uses_only_contexts_github_provides_there() {
        // GitHub rejects the whole workflow file otherwise, and no job starts.
        let allowed = [
            "github", "needs", "strategy", "matrix", "vars", "secrets", "inputs",
        ];
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let source = load_catalog(&root, false).unwrap();
        let files = source
            .groups
            .iter()
            .map(|group| group.workflow.clone())
            .collect::<BTreeSet<_>>();
        assert!(!files.is_empty());
        for name in files {
            let rendered = workflow(&source, &name).unwrap();
            for context in job_env_contexts(&rendered) {
                assert!(allowed.contains(&context.as_str()), "{name}: {context}");
            }
            let jobs = workflow_jobs(&rendered);
            assert!(!jobs.is_empty(), "{name}");
            for job in jobs {
                let compiler = job.find("id: native-toolchain").unwrap();
                let target = job
                    .find("name: Select the native target directory")
                    .unwrap_or_else(|| panic!("{name}: no target directory step"));
                let cache = job.find("name: Restore native Cargo artifacts").unwrap();
                let build = job.find("name: Build the native engine").unwrap();
                let check = job.find("name: Run the catalog group").unwrap();
                assert!(
                    compiler < target && target < cache && cache < build && build < check,
                    "{name}"
                );
                assert!(job.contains("      CARGO_PROFILE_DEV_DEBUG: \"0\"\n"));
                assert!(job.contains("      CARGO_INCREMENTAL: \"0\"\n"), "{name}");
                // A missed key falls back to the newest entry of this job and
                // compiler, never to another job's or a v1 entry.
                let prefix = job[cache..build]
                    .split("          restore-keys: ")
                    .nth(1)
                    .and_then(|rest| rest.lines().next())
                    .unwrap_or_else(|| panic!("{name}: a missed key restores nothing"));
                let job_name = job.lines().next().unwrap().trim().trim_end_matches(':');
                assert!(prefix.starts_with("native-cargo-v2-"), "{name}");
                assert!(prefix.ends_with(&format!("-{job_name}-dev0-")), "{name}");
                assert!(
                    job[cache..build].contains(&format!("          key: {prefix}${{{{ hashFiles(")),
                    "{name}"
                );
                assert!(job.contains("\"$CARGO_TARGET_DIR/debug/infra-engine$suffix\""));
                // Cargo in the group replaces the built engine; Windows refuses
                // that while the engine runs, so only a copy may run the group.
                let copy = job[check..]
                    .find("          cp \"$CARGO_TARGET_DIR/debug/infra-engine$suffix\" \"$engine\"\n")
                    .unwrap_or_else(|| panic!("{name}: the group runs the built engine"));
                let run = job[check..]
                    .find("          \"$engine\" check --group \"$NATIVE_CHECK_GROUP\"")
                    .unwrap();
                assert!(copy < run, "{name}");
                assert!(
                    job[check..]
                        .contains("engine=\"${RUNNER_TEMP//\\\\//}/infra-check-engine$suffix\"\n")
                );
                // Failed logs go where the job's last step collects them.
                assert!(
                    job[check..].contains(" --log-dir \"${RUNNER_TEMP//\\\\//}/infra-check-logs\""),
                    "{name}"
                );
                let keep = job
                    .find("      - name: Keep the complete output of failed checks\n")
                    .unwrap_or_else(|| panic!("{name}: failed logs are not kept"));
                assert!(check < keep, "{name}");
                assert!(job[keep..].contains("        if: failure()\n"), "{name}");
                assert!(
                    job[keep..].contains("          path: ${{ runner.temp }}/infra-check-logs\n"),
                    "{name}"
                );
                assert!(!job[build..check].contains("        if:"), "{name}");
                // Dependencies compiled before a failure are kept too; only
                // the save step reads whether the exact key was found.
                let save = job
                    .find("      - name: Save native Cargo artifacts\n")
                    .unwrap_or_else(|| panic!("{name}: compiled dependencies are not kept"));
                assert!(keep < save, "{name}");
                assert!(
                    job[save..].contains(
                        "        if: ${{ !cancelled() && steps.native-cache.outputs.cache-hit != 'true' }}\n"
                    ),
                    "{name}"
                );
                assert!(
                    job[save..].contains(
                        "          key: ${{ steps.native-cache.outputs.cache-primary-key }}\n"
                    ),
                    "{name}"
                );
                assert!(!job[..save].contains("cache-hit"), "{name}");
                assert_eq!(job.matches(CARGO_CACHE_PATHS).count(), 2, "{name}");
            }
        }
        assert_eq!(
            job_env_contexts("    env:\n      A: ${{ runner.temp }}/x\n    steps:\n"),
            ["runner"]
        );
    }

    #[test]
    fn native_platform_jobs_keep_their_existing_stable_compiler() {
        for (platform, runner) in [("windows", "windows-latest"), ("macos", "macos-latest")] {
            let mut native = group("native", "native");
            native["platform"] = platform.into();
            let rendered = workflow(&catalog(json!([native])), "rust.yml").unwrap();
            assert!(rendered.contains(&format!("runs-on: {runner}")));
            assert!(rendered.contains("uses: dtolnay/rust-toolchain@stable"));
            assert!(!rendered.contains("          toolchain:"));
        }
    }

    #[test]
    fn only_jobs_that_run_nextest_install_it_before_building() {
        let mut parallel = group("parallel", "parallel");
        parallel["gates"][0]["tools"] = json!(["cargo-nextest"]);
        let rendered = workflow(
            &catalog(json!([parallel, group("plain", "plain")])),
            "rust.yml",
        )
        .unwrap();
        let jobs = workflow_jobs(&rendered);
        let (parallel, plain) = (&jobs[0], &jobs[1]);
        let install = parallel
            .find(concat!(
                "      - uses: taiki-e/install-action@a4d3f3f761dcd95cce1b72cb61cb6d1963438177 # v2.87.30\n",
                "        with:\n",
                "          tool: cargo-nextest@0.9.149\n",
                "          fallback: none\n",
            ))
            .expect("the nextest job installs a pinned release");
        assert!(parallel.find("id: native-toolchain").unwrap() < install);
        assert!(install < parallel.find("name: Build the native engine").unwrap());
        assert!(!plain.contains("install-action"));
    }

    #[test]
    fn linux_nextest_jobs_provision_and_probe_oracle_namespaces() {
        let mut linux = group("linux", "linux");
        linux["gates"][0]["tools"] = json!(["cargo-nextest"]);
        let mut windows = linux.clone();
        windows["name"] = "windows".into();
        windows["job"] = "windows".into();
        windows["platform"] = "windows".into();
        let rendered = workflow(
            &catalog(json!([linux, windows, group("plain", "plain")])),
            "rust.yml",
        )
        .unwrap();
        let jobs = workflow_jobs(&rendered);
        let setup = jobs[0]
            .find("name: Prepare native oracle isolation")
            .unwrap();
        assert!(setup < jobs[0].find("name: Build the native engine").unwrap());
        assert!(jobs[0].contains("sudo apt-get install --yes bubblewrap"));
        assert!(
            jobs[0]
                .contains("if [ -f /proc/sys/kernel/apparmor_restrict_unprivileged_userns ]; then")
        );
        assert!(
            jobs[0]
                .contains("bwrap --unshare-all --die-with-parent --ro-bind / / -- /usr/bin/true")
        );
        assert!(!jobs[1].contains("oracle isolation"));
        assert!(!jobs[2].contains("oracle isolation"));
    }

    #[test]
    fn windows_nextest_bootstrap_compiles_the_workspace_without_running_tests() {
        let mut windows = group("windows", "windows");
        windows["platform"] = "windows".into();
        windows["gates"][0]["tools"] = json!(["cargo-nextest"]);
        let mut linux = windows.clone();
        linux["name"] = "linux".into();
        linux["job"] = "linux".into();
        linux["platform"] = "linux".into();
        let mut plain = windows.clone();
        plain["name"] = "plain".into();
        plain["job"] = "plain".into();
        plain["gates"][0]["tools"] = json!([]);
        let jobs =
            workflow_jobs(&workflow(&catalog(json!([windows, linux, plain])), "rust.yml").unwrap());
        let build = |job: &str| {
            job.split("      - name: Build the native engine\n")
                .nth(1)
                .unwrap()
                .split("      - name: Run the catalog group\n")
                .next()
                .unwrap()
                .to_owned()
        };
        let windows = jobs
            .iter()
            .find(|job| job.starts_with("  windows:\n"))
            .unwrap();
        assert_eq!(
            build(windows),
            "        run: cargo nextest run --workspace --manifest-path runtime-rust/Cargo.toml --locked --no-run\n"
        );
        for name in ["linux", "plain"] {
            let job = jobs
                .iter()
                .find(|job| job.starts_with(&format!("  {name}:\n")))
                .unwrap();
            assert_eq!(
                build(job),
                "        run: cargo build --manifest-path runtime-rust/Cargo.toml --locked --bin infra-engine\n"
            );
        }
    }
}

#[cfg(test)]
mod tracked_json_syntax_tests {
    use super::*;

    fn repository() -> Temporary {
        let temporary = Temporary::new("infra-json-syntax").unwrap();
        common::git_ok(
            temporary.path(),
            &["-c", "init.templateDir=", "init", "--quiet"],
            None,
        )
        .unwrap();
        temporary
    }
    fn indexed(root: &Path, name: &str, bytes: &[u8]) {
        let path = root.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
        common::git_ok(
            root,
            &[
                "-c",
                "core.autocrlf=false",
                "-c",
                "core.attributesFile=",
                "add",
                "--",
                name,
            ],
            None,
        )
        .unwrap();
    }

    #[test]
    fn syntax_accepts_legacy_values_and_consumes_the_complete_input() {
        for value in [
            r#"{"key":1,"key":2}"#,
            r#"[NaN,Infinity,-Infinity,1e9999,{"value":NaN}]"#,
            r#""\uD800""#,
            r#"{"\uDC00":"\uD800"}"#,
            r#""NaN Infinity -Infinity are ordinary text""#,
            r#""escaped quote: \" and slash: \\""#,
            "42",
            "-0",
            "null",
            "true",
            r#"{"unicode":"λ"}"#,
        ] {
            assert!(validate_json_syntax(value.as_bytes()).is_ok(), "{value}");
        }
        for value in [
            "",
            "null trailing",
            "NaNprefix",
            "prefixNaN",
            "InfinityNaN",
            "+Infinity",
            "- Infinity",
            "[NaN,]",
            "{NaN:1}",
            r#""unterminated"#,
            r#""bad \q escape""#,
            r#""bad \u123 escape""#,
            r#"{"x":}"#,
            "[01]",
        ] {
            assert!(validate_json_syntax(value.as_bytes()).is_err(), "{value}");
        }
        assert!(validate_json_syntax(&[b'"', 0xff, b'"']).is_err());
    }

    #[test]
    fn indexed_members_read_worktree_bytes_and_ignore_untracked_json() {
        let temporary = repository();
        let root = temporary.path();
        indexed(root, "nested/valid file.json", b"{\"key\":1,\"key\":2}\n");
        indexed(root, "changed.json", b"{}\n");
        indexed(root, "removed.json", b"{}\n");
        indexed(root, "staged-deletion.json", b"{}\n");
        indexed(root, "invalid-utf8.json", b"{}\n");
        // Selection follows the index; syntax follows each current worktree image.
        std::fs::write(root.join("changed.json"), b"{} trailing").unwrap();
        std::fs::write(root.join("invalid-utf8.json"), [b'"', 0xff, b'"']).unwrap();
        std::fs::remove_file(root.join("removed.json")).unwrap();
        common::git_ok(
            root,
            &["rm", "--cached", "--quiet", "--", "staged-deletion.json"],
            None,
        )
        .unwrap();
        std::fs::write(root.join("staged-deletion.json"), b"invalid").unwrap();
        std::fs::write(root.join("untracked.json"), b"invalid").unwrap();
        let report = crate::execute("json-syntax", &json!({"root":root})).unwrap();
        assert_eq!(report["files"], 4);
        assert_eq!(report["passed"], false);
        assert_eq!(crate::report_exit_code(&report), 1);
        let paths = report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|finding| finding["path"].as_str().unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            paths,
            BTreeSet::from(["changed.json", "invalid-utf8.json", "removed.json"])
        );
        assert_eq!(
            std::fs::read(root.join("changed.json")).unwrap(),
            b"{} trailing"
        );
        assert_eq!(
            std::fs::read(root.join("untracked.json")).unwrap(),
            b"invalid"
        );
    }

    #[test]
    fn a_valid_indexed_repository_passes_without_reformatting() {
        let temporary = repository();
        let root = temporary.path();
        let original = br#" { "duplicate": 1, "duplicate": NaN, "text": "\uD800" } "#;
        indexed(root, "valid.json", original);
        std::fs::write(root.join("ignored.json"), b"not JSON").unwrap();
        let report = check_tracked_json(root).unwrap();
        assert_eq!(report["files"], 1);
        assert_eq!(report["passed"], true);
        assert_eq!(crate::report_exit_code(&report), 0);
        assert_eq!(std::fs::read(root.join("valid.json")).unwrap(), original);
    }

    #[cfg(unix)]
    #[test]
    fn nul_delimited_membership_keeps_newlines_inside_filenames() {
        let temporary = repository();
        let root = temporary.path();
        let name = "line\nbreak.json";
        indexed(root, name, b"{}\n");
        std::fs::write(root.join(name), b"[").unwrap();
        let report = check_tracked_json(root).unwrap();
        assert_eq!(report["files"], 1);
        assert_eq!(report["findings"][0]["path"], name);
        assert_eq!(report["passed"], false);
    }
}

#[cfg(test)]
mod failure_report_tests {
    use super::*;

    fn outcome(status: &str, excerpt: Vec<(&'static str, String)>) -> Outcome {
        Outcome {
            group: "rust".into(),
            name: "Workspace Rust tests".into(),
            status: status.into(),
            seconds: 1.0,
            code: Some(101),
            reason: None,
            log: None,
            excerpt,
        }
    }

    #[test]
    fn excerpts_keep_the_last_lines_of_each_nonempty_stream() {
        let long = (1..=EXCERPT_LINES + 5)
            .map(|line| format!("line {line}\n"))
            .collect::<String>();
        let kept = excerpt(&[
            ("stdout", long.as_bytes()),
            ("stderr", b""),
            ("report", b"no final newline"),
        ]);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].0, "stdout");
        let lines = kept[0].1.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), EXCERPT_LINES);
        assert_eq!(lines[0], "line 6");
        assert_eq!(
            lines[EXCERPT_LINES - 1],
            format!("line {}", EXCERPT_LINES + 5)
        );
        assert_eq!(kept[1], ("report", "no final newline".to_string()));
    }

    #[test]
    fn excerpts_keep_the_last_bytes_of_an_overlong_line() {
        // Two-byte characters put the byte bound inside one of them.
        let mut bytes = "é".repeat(EXCERPT_BYTES).into_bytes();
        bytes.extend_from_slice(b"\nend\n");
        let kept = excerpt(&[("stderr", bytes.as_slice())]);
        let text = &kept[0].1;
        assert!(text.starts_with('\u{FFFD}'), "{}", &text[..8]);
        assert!(text.ends_with("é\nend"));
        // The cut continuation byte becomes one three-byte replacement.
        assert_eq!(text.len(), EXCERPT_BYTES + 1);
    }

    #[test]
    fn the_report_names_only_failed_checks_with_their_streams() {
        assert_eq!(failure_report(&[outcome("pass", vec![])]), "");
        let mut absent = outcome("fail", vec![]);
        absent.name = "PHP snippets".into();
        absent.code = Some(1);
        absent.reason = Some("Required executable is unavailable: php".into());
        let report = failure_report(&[
            outcome("pass", vec![]),
            outcome(
                "fail",
                vec![
                    ("stdout", "test locks ... FAILED".into()),
                    ("stderr", "error: test failed".into()),
                ],
            ),
            absent,
            outcome("skipped", vec![]),
        ]);
        assert_eq!(
            report,
            concat!(
                "FAILED rust / Workspace Rust tests (exit 101)\n",
                "--- end of stdout ---\ntest locks ... FAILED\n",
                "--- end of stderr ---\nerror: test failed\n",
                "FAILED rust / PHP snippets (exit 1): Required executable is unavailable: php\n",
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_process_check_reports_the_end_of_both_streams() {
        let temporary = Temporary::new("infra-check-report").unwrap();
        let group: Group = serde_json::from_value(json!({
            "name":"probe","workflow":"ci.yml","job":"probe","timeout_minutes":1,
            "gates":[{"kind":"process","name":"failing probe","program":"sh",
                "arguments":["-c","echo first; echo last; echo cause >&2; exit 3"]}]
        }))
        .unwrap();
        let outcome = gate(
            GateContext {
                root: temporary.path(),
                base: "origin/main",
                group: &group,
                strict: true,
                no_cache: true,
                oracle_allowed: false,
                logs: temporary.path(),
                cancel: Arc::new(AtomicBool::new(false)),
            },
            &group.gates[0],
        );
        assert_eq!((outcome.status.as_str(), outcome.code), ("fail", Some(3)));
        assert_eq!(
            outcome.excerpt,
            [
                ("stdout", "first\nlast".to_string()),
                ("stderr", "cause".to_string())
            ]
        );
        // The retained log still holds the complete output.
        assert_eq!(
            std::fs::read(outcome.log.as_ref().unwrap()).unwrap(),
            b"first\nlast\ncause\n"
        );
        assert_eq!(
            failure_report(&[outcome]),
            "FAILED probe / failing probe (exit 3)\n--- end of stdout ---\nfirst\nlast\n--- end of stderr ---\ncause\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_named_log_directory_keeps_failed_logs_for_the_job_to_collect() {
        let temporary = Temporary::new("infra-check-named-logs").unwrap();
        let root = temporary.path();
        std::fs::create_dir_all(root.join(".github/workflows")).unwrap();
        std::fs::write(root.join(".github/workflows/probe.yml"), b"name: probe\n").unwrap();
        std::fs::create_dir_all(root.join("install")).unwrap();
        std::fs::write(
            root.join("install/check-catalog.json"),
            serde_json::to_vec(&json!({"schema_version":1,"groups":[{
                "name":"probe","workflow":"probe.yml","job":"probe","timeout_minutes":1,
                "gates":[{"kind":"process","name":"failing probe","program":"sh",
                    "arguments":["-c","echo kept; exit 3"]}]
            }]}))
            .unwrap(),
        )
        .unwrap();
        let logs = root.join("named logs");
        let value = execute(&json!({
            "root":root,"groups":["probe"],"strict":true,"log_dir":logs
        }))
        .unwrap();
        assert_eq!(value["result"]["exit_code"], 1);
        assert_eq!(value["result"]["logs"], json!(logs));
        let kept = std::fs::read_dir(&logs)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(std::fs::read(&kept[0]).unwrap(), b"kept\n");
        assert_eq!(
            value["_cli"]["stderr"],
            "FAILED probe / failing probe (exit 3)\n--- end of stdout ---\nkept\n"
        );
    }
}

#[cfg(all(test, unix))]
mod fallback_tests {
    use super::*;

    fn outcome_of(check: Value) -> Outcome {
        let temporary = Temporary::new("infra-check-fallback").unwrap();
        let group: Group = serde_json::from_value(json!({
            "name":"probe","workflow":"ci.yml","job":"probe","timeout_minutes":1,"gates":[check]
        }))
        .unwrap();
        gate(
            GateContext {
                root: temporary.path(),
                base: "origin/main",
                group: &group,
                strict: true,
                no_cache: true,
                oracle_allowed: false,
                logs: temporary.path(),
                cancel: Arc::new(AtomicBool::new(false)),
            },
            &group.gates[0],
        )
    }

    #[test]
    fn an_unavailable_tool_runs_the_fallback_instead_of_failing() {
        let outcome = outcome_of(json!({"kind":"process","name":"probe","program":"sh",
            "arguments":["-c","exit 3"],"tools":["infra-absent-tool"],
            "fallback":{"program":"sh","arguments":["-c","exit 0"]}}));
        assert_eq!((outcome.status.as_str(), outcome.code), ("pass", Some(0)));
        assert_eq!(
            outcome.reason.as_deref(),
            Some("infra-absent-tool is unavailable; ran the fallback")
        );
        // Without a fallback the same gate still fails a strict run.
        let outcome = outcome_of(json!({"kind":"process","name":"probe","program":"sh",
            "arguments":["-c","exit 3"],"tools":["infra-absent-tool"]}));
        assert_eq!(outcome.status, "fail");
        assert_eq!(
            outcome.reason.as_deref(),
            Some("Required executable is unavailable: infra-absent-tool")
        );
    }

    #[test]
    fn an_available_tool_runs_the_check_itself() {
        let outcome = outcome_of(json!({"kind":"process","name":"probe","program":"sh",
            "arguments":["-c","exit 3"],"tools":["sh"],
            "fallback":{"program":"sh","arguments":["-c","exit 0"]}}));
        assert_eq!((outcome.status.as_str(), outcome.code), ("fail", Some(3)));
        assert_eq!(outcome.reason, None);
    }

    #[test]
    fn a_fallback_cannot_bring_python_back() {
        let temporary = Temporary::new("infra-check-fallback-python").unwrap();
        let root = temporary.path();
        std::fs::create_dir_all(root.join("install")).unwrap();
        std::fs::write(
            root.join("install/check-catalog.json"),
            serde_json::to_vec(&json!({"schema_version":1,"groups":[{
                "name":"probe","workflow":"probe.yml","job":"probe","timeout_minutes":1,
                "gates":[{"kind":"process","name":"probe","program":"cargo",
                    "tools":["cargo-nextest"],"fallback":{"program":"python3"}}]
            }]}))
            .unwrap(),
        )
        .unwrap();
        let error = load_catalog(root, false).unwrap_err();
        assert_eq!(error.kind, infra_core::ErrorKind::Policy);
    }

    #[test]
    fn selective_receipt_cache_hits_only_when_opted_in_settled_and_never_under_strict_or_no_cache()
    {
        let temporary = Temporary::new("infra-check-receipts").unwrap();
        let root = infra_core::fs::canonicalize(temporary.path()).unwrap();
        let marker =
            std::env::temp_dir().join(format!("infra-check-count-{}.txt", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join(".github/workflows")).unwrap();
        std::fs::write(root.join(".github/workflows/probe.yml"), b"name: probe\n").unwrap();
        std::fs::create_dir_all(root.join("install")).unwrap();
        std::fs::write(
            root.join("install/check-catalog.json"),
            serde_json::to_vec(&json!({"schema_version":1,"groups":[{
                "name":"probe","workflow":"probe.yml","job":"probe","timeout_minutes":1,
                "gates":[
                    {"kind":"process","name":"echo probe","program":"sh","arguments":["-c",&format!("printf x >> '{}'", marker.display())]},
                    {"kind":"tool","name":"json syntax","operation":"json-syntax","request":{}}
                ]
            }]}))
            .unwrap(),
        )
        .unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "Test"],
            vec!["add", "."],
            vec!["commit", "--quiet", "-m", "init"],
        ] {
            let st = std::process::Command::new("git")
                .args(&args)
                .current_dir(&root)
                .status()
                .unwrap();
            assert!(st.success(), "git {:?} failed", args);
        }
        let cache_dir = Temporary::new("infra-check-receipts-store").unwrap();
        let cache_file = cache_dir.path().join("receipts.json");
        let (_digest, max_mtime) = input_closure_stamp(&root, &[]).unwrap();

        // Inside 2s RACY_NS window -> executes live even with cache: true.
        let racy = execute_at(
            &json!({"root":&root,"groups":["probe"],"strict":false,"cache":true,"cache_file":&cache_file}),
            max_mtime + CHECKER_RACY_NS - 1,
        )
        .unwrap();
        assert_eq!(racy["result"]["exit_code"], 0);
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "x");

        // Outside RACY_NS window -> run 1 populates receipts, run 2 hits cached receipts cleanly.
        let settled_now = max_mtime + CHECKER_RACY_NS + 100;
        let first = execute_at(
            &json!({"root":&root,"groups":["probe"],"strict":false,"cache":true,"cache_file":&cache_file}),
            settled_now,
        )
        .unwrap();
        assert_eq!(first["result"]["exit_code"], 0);
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "xx");

        let second = execute_at(
            &json!({"root":&root,"groups":["probe"],"strict":false,"cache":true,"cache_file":&cache_file}),
            settled_now + 50,
        )
        .unwrap();
        assert_eq!(second["result"]["exit_code"], 0);
        assert_eq!(
            std::fs::read_to_string(&marker).unwrap(),
            "xx",
            "cached run must not re-run process gate"
        );
        let reasons: Vec<&str> = second["result"]["results"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|g| g["reason"].as_str())
            .collect();
        assert_eq!(
            reasons,
            vec!["cached local receipt", "cached local receipt"]
        );

        // `--strict` MUST disable caching even when cache: true + settled receipts exist!
        let strict_res = execute_at(
            &json!({"root":&root,"groups":["probe"],"strict":true,"cache":true,"cache_file":&cache_file}),
            settled_now + 60,
        )
        .unwrap();
        assert_eq!(strict_res["result"]["exit_code"], 0);
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "xxx");

        // `no_cache: true` MUST disable caching even under non-strict.
        let no_cache_res = execute_at(
            &json!({"root":&root,"groups":["probe"],"strict":false,"cache":true,"no_cache":true,"cache_file":&cache_file}),
            settled_now + 70,
        )
        .unwrap();
        assert_eq!(no_cache_res["result"]["exit_code"], 0);
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "xxxx");

        // Modifying tracked repo file invalidates input closure and runs live again.
        std::fs::write(
            root.join(".github/workflows/probe.yml"),
            b"name: probe-v2\n",
        )
        .unwrap();
        let (_d2, new_mtime) = input_closure_stamp(&root, &[]).unwrap();
        let after_edit = execute_at(
            &json!({"root":&root,"groups":["probe"],"strict":false,"cache":true,"cache_file":&cache_file}),
            new_mtime + CHECKER_RACY_NS + 100,
        )
        .unwrap();
        assert_eq!(after_edit["result"]["exit_code"], 0);
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "xxxxx");

        let _ = std::fs::remove_file(&marker);
    }

    #[test]
    fn scoped_closures_isolate_unrelated_edits_survive_over_4000_files_and_track_ir_cues() {
        // Non-Git repo with >4000 files in an unrelated directory still allows scoped checks to cache!
        let temporary = Temporary::new("infra-check-scoped-4k").unwrap();
        let root = infra_core::fs::canonicalize(temporary.path()).unwrap();
        let marker_a =
            std::env::temp_dir().join(format!("infra-check-scope-a-{}.txt", uuid::Uuid::new_v4()));
        let marker_b =
            std::env::temp_dir().join(format!("infra-check-scope-b-{}.txt", uuid::Uuid::new_v4()));

        std::fs::create_dir_all(root.join(".github/workflows")).unwrap();
        std::fs::write(root.join(".github/workflows/probe.yml"), b"name: probe\n").unwrap();
        std::fs::create_dir_all(root.join("install")).unwrap();
        std::fs::create_dir_all(root.join("sub_a")).unwrap();
        std::fs::create_dir_all(root.join("sub_b")).unwrap();
        std::fs::create_dir_all(root.join("bulk")).unwrap();
        std::fs::write(root.join("sub_a/in.txt"), b"a1\n").unwrap();
        std::fs::write(root.join("sub_b/in.txt"), b"b1\n").unwrap();
        // >4000 files in bulk/ would break an unconditional root walk.
        for i in 0..4050 {
            let bucket = i / 200;
            let dir = root.join(format!("bulk/d{bucket}"));
            if i % 200 == 0 {
                std::fs::create_dir_all(&dir).unwrap();
            }
            std::fs::write(dir.join(format!("f{i}.txt")), b"x").unwrap();
        }
        std::fs::write(
            root.join("install/check-catalog.json"),
            serde_json::to_vec(&json!({"schema_version":1,"groups":[
                {
                    "name":"scope_a","workflow":"probe.yml","job":"scope_a","timeout_minutes":1,
                    "gates":[{"kind":"process","name":"a probe","program":"sh","cwd":"sub_a","paths":["sub_a/in.txt"],
                        "arguments":["-c",&format!("printf a >> '{}'", marker_a.display())]}]
                },
                {
                    "name":"scope_b","workflow":"probe.yml","job":"scope_b","timeout_minutes":1,
                    "gates":[{"kind":"process","name":"b probe","program":"sh","cwd":"sub_b","paths":["sub_b/in.txt"],
                        "arguments":["-c",&format!("printf b >> '{}'", marker_b.display())]}]
                }
            ]}))
            .unwrap(),
        )
        .unwrap();

        let catalog = load_catalog(&root, true).unwrap();
        let scope_a = scope_for_gate(
            &root,
            &json!({}),
            "origin/main",
            &catalog.groups[0],
            &catalog.groups[0].gates[0],
        );
        assert_eq!(scope_a.kind, ScopeKind::ScopedDirs);
        assert_eq!(scope_a.dirs, vec!["sub_a".to_string()]);
        let (_d_a, mtime_a) = compute_closure_stamp(&root, &scope_a).unwrap();
        let scope_b = scope_for_gate(
            &root,
            &json!({}),
            "origin/main",
            &catalog.groups[1],
            &catalog.groups[1].gates[0],
        );
        let (_d_b, mtime_b) = compute_closure_stamp(&root, &scope_b).unwrap();
        let settled_now = mtime_a.max(mtime_b) + CHECKER_RACY_NS + 100;

        let cache_dir = Temporary::new("infra-check-scoped-cache").unwrap();
        let cache_file = cache_dir.path().join("receipts.json");

        // Run 1 populates receipts; run 2 hits cached receipts for both scopes despite >4000 files in bulk/.
        let run1 = execute_at(
            &json!({"root":&root,"groups":["scope_a","scope_b"],"strict":false,"cache":true,"cache_file":&cache_file}),
            settled_now,
        )
        .unwrap();
        assert_eq!(run1["result"]["exit_code"], 0);
        assert_eq!(std::fs::read_to_string(&marker_a).unwrap(), "a");
        assert_eq!(std::fs::read_to_string(&marker_b).unwrap(), "b");

        let run2 = execute_at(
            &json!({"root":&root,"groups":["scope_a","scope_b"],"strict":false,"cache":true,"cache_file":&cache_file}),
            settled_now + 10,
        )
        .unwrap();
        assert_eq!(run2["result"]["exit_code"], 0);
        assert_eq!(std::fs::read_to_string(&marker_a).unwrap(), "a");
        assert_eq!(std::fs::read_to_string(&marker_b).unwrap(), "b");

        // Editing sub_b invalidates ONLY scope_b, leaving scope_a cached!
        std::fs::write(root.join("sub_b/in.txt"), b"b2\n").unwrap();
        let (_d_b2, mtime_b2) = compute_closure_stamp(&root, &scope_b).unwrap();
        let run3 = execute_at(
            &json!({"root":&root,"groups":["scope_a","scope_b"],"strict":false,"cache":true,"cache_file":&cache_file}),
            mtime_b2 + CHECKER_RACY_NS + 100,
        )
        .unwrap();
        assert_eq!(run3["result"]["exit_code"], 0);
        assert_eq!(std::fs::read_to_string(&marker_a).unwrap(), "a");
        assert_eq!(std::fs::read_to_string(&marker_b).unwrap(), "bb");

        // Bumping Graph-IR revision cue invalidates even unchanged scope_a.
        let run4 = execute_at(
            &json!({"root":&root,"groups":["scope_a"],"strict":false,"cache":true,"ir_revision":7,"cache_file":&cache_file}),
            mtime_b2 + CHECKER_RACY_NS + 200,
        )
        .unwrap();
        assert_eq!(run4["result"]["exit_code"], 0);
        assert_eq!(std::fs::read_to_string(&marker_a).unwrap(), "aa");

        let run5 = execute_at(
            &json!({"root":&root,"groups":["scope_a"],"strict":false,"cache":true,"ir_revision":7,"cache_file":&cache_file}),
            mtime_b2 + CHECKER_RACY_NS + 250,
        )
        .unwrap();
        assert_eq!(run5["result"]["exit_code"], 0);
        assert_eq!(std::fs::read_to_string(&marker_a).unwrap(), "aa");

        // `--strict` and `no_cache: true` bypass receipts cleanly.
        let strict_res = execute_at(
            &json!({"root":&root,"groups":["scope_a"],"strict":true,"cache":true,"ir_revision":7,"cache_file":&cache_file}),
            mtime_b2 + CHECKER_RACY_NS + 260,
        )
        .unwrap();
        assert_eq!(strict_res["result"]["exit_code"], 0);
        assert_eq!(std::fs::read_to_string(&marker_a).unwrap(), "aaa");

        let no_cache_res = execute_at(
            &json!({"root":&root,"groups":["scope_a"],"strict":false,"cache":true,"no_cache":true,"ir_revision":7,"cache_file":&cache_file}),
            mtime_b2 + CHECKER_RACY_NS + 270,
        )
        .unwrap();
        assert_eq!(no_cache_res["result"]["exit_code"], 0);
        assert_eq!(std::fs::read_to_string(&marker_a).unwrap(), "aaaa");

        // Now initialize Git in the >4000-file repo and verify FullRepo input_closure_stamp
        // succeeds without hitting MAX_CLOSURE_FILES.
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "Test"],
            vec!["add", "."],
            vec!["commit", "--quiet", "-m", "init"],
        ] {
            let st = std::process::Command::new("git")
                .args(&args)
                .current_dir(&root)
                .status()
                .unwrap();
            assert!(st.success(), "git {:?} failed", args);
        }
        let full_stamp = input_closure_stamp(&root, &[]);
        assert!(
            full_stamp.is_some(),
            "Git-backed full repo closure stamp must succeed on >4000-file repository"
        );

        let _ = std::fs::remove_file(&marker_a);
        let _ = std::fs::remove_file(&marker_b);
    }

    #[test]
    fn real_repo_catalog_narrows_scopes_and_computes_full_repo_git_stamp() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let root = infra_core::fs::canonicalize(root).unwrap();
        let catalog = load_catalog(&root, true).unwrap();

        let wasm = catalog.groups.iter().find(|g| g.name == "wasm").unwrap();
        let wasm_scope = scope_for_gate(&root, &json!({}), "origin/main", wasm, &wasm.gates[0]);
        assert_eq!(wasm_scope.kind, ScopeKind::ScopedDirs);
        assert_eq!(
            wasm_scope.dirs,
            vec!["runtime-rust/crates/infra-ui".to_string()]
        );
        assert!(compute_closure_stamp(&root, &wasm_scope).is_some());

        let mirrors = catalog.groups.iter().find(|g| g.name == "mirrors").unwrap();
        let laravel_scope =
            scope_for_gate(&root, &json!({}), "origin/main", mirrors, &mirrors.gates[0]);
        assert_eq!(laravel_scope.kind, ScopeKind::ScopedDirs);
        assert_eq!(laravel_scope.dirs, vec!["Laravel".to_string()]);
        assert!(compute_closure_stamp(&root, &laravel_scope).is_some());

        let lint = catalog.groups.iter().find(|g| g.name == "lint").unwrap();
        let php_gate = lint
            .gates
            .iter()
            .find(|g| g.name() == "PHP documentation snippets")
            .unwrap();
        let php_scope = scope_for_gate(&root, &json!({}), "origin/main", lint, php_gate);
        assert_eq!(php_scope.kind, ScopeKind::GitTrackedGlobs);
        assert!(compute_closure_stamp(&root, &php_scope).is_some());

        // Full repo closure stamp (e.g. `rust` group) also succeeds on the 4000+ file monorepo!
        assert!(input_closure_stamp(&root, &[]).is_some());
    }

    #[test]
    fn strict_and_no_cache_propagate_bypass_to_inner_tool_gate_receipt_caches() {
        let temporary = Temporary::new("infra-check-tool-bypass").unwrap();
        let root = infra_core::fs::canonicalize(temporary.path()).unwrap();
        std::fs::create_dir_all(root.join(".github/workflows")).unwrap();
        std::fs::write(root.join(".github/workflows/ci.yml"), b"name: ci\n").unwrap();
        std::fs::create_dir_all(root.join("install")).unwrap();
        std::fs::create_dir_all(root.join("PHP Core/memory-bank/config")).unwrap();
        std::fs::create_dir_all(root.join("PHP Core/.agents/skills/demo")).unwrap();
        std::fs::write(
            root.join("PHP Core/memory-bank/config/mirror-rules.json"),
            common::json_bytes(&json!({
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
        )
        .unwrap();
        std::fs::write(
            root.join("PHP Core/.agents/skills/demo/SKILL.md"),
            b"# Demo\n",
        )
        .unwrap();
        crate::mirrors::process(&root, "PHP Core", true).unwrap();
        std::fs::write(
            root.join("install/check-catalog.json"),
            serde_json::to_vec(&json!({"schema_version":1,"groups":[{
                "name":"mirrors","workflow":"ci.yml","job":"mirrors","timeout_minutes":1,
                "gates":[{"kind":"tool","name":"PHP Core mirrors","operation":"mirrors","request":{"edition":"PHP Core","write":false}}]
            }]})).unwrap(),
        )
        .unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "Test"],
            vec!["add", "."],
            vec!["commit", "--quiet", "-m", "init"],
        ] {
            let st = std::process::Command::new("git")
                .args(&args)
                .current_dir(&root)
                .status()
                .unwrap();
            assert!(st.success(), "git {:?} failed", args);
        }
        let mirror_receipt_path = root.join(".git/infra-mirror-receipts.json");
        let check_cache_dir = Temporary::new("infra-check-tool-cache").unwrap();
        let check_cache_file = check_cache_dir.path().join("receipts.json");

        // `--strict` and `no_cache: true` must bypass inner mirrors receipt cache (neither reads nor creates it).
        let strict_res = execute_at(
            &json!({"root":&root,"groups":["mirrors"],"strict":true,"cache":true,"cache_file":&check_cache_file}),
            now_unix_ns() + CHECKER_RACY_NS + 1000,
        )
        .unwrap();
        assert_eq!(strict_res["result"]["exit_code"], 0);
        assert!(!mirror_receipt_path.exists());
        assert!(!check_cache_file.exists());

        let no_cache_res = execute_at(
            &json!({"root":&root,"groups":["mirrors"],"strict":false,"no_cache":true,"cache":true,"cache_file":&check_cache_file}),
            now_unix_ns() + CHECKER_RACY_NS + 1000,
        )
        .unwrap();
        assert_eq!(no_cache_res["result"]["exit_code"], 0);
        assert!(!mirror_receipt_path.exists());
        assert!(!check_cache_file.exists());

        // Settled cached run populates both inner mirror receipt and outer check receipt, then hits outer receipt.
        let catalog = load_catalog(&root, true).unwrap();
        let scope = scope_for_gate(
            &root,
            &json!({}),
            "origin/main",
            &catalog.groups[0],
            &catalog.groups[0].gates[0],
        );
        let (_digest, max_mtime) = compute_closure_stamp(&root, &scope).unwrap();
        let settled = max_mtime + CHECKER_RACY_NS + 100;
        let run1 = execute_at(
            &json!({"root":&root,"groups":["mirrors"],"strict":false,"cache":true,"cache_file":&check_cache_file}),
            settled,
        )
        .unwrap();
        assert_eq!(run1["result"]["exit_code"], 0);
        assert!(mirror_receipt_path.exists());
        assert!(check_cache_file.exists());

        let run2 = execute_at(
            &json!({"root":&root,"groups":["mirrors"],"strict":false,"cache":true,"cache_file":&check_cache_file}),
            settled + 10,
        )
        .unwrap();
        assert_eq!(run2["result"]["exit_code"], 0);
        assert_eq!(
            run2["result"]["results"][0]["reason"],
            "cached local receipt"
        );
    }
}
