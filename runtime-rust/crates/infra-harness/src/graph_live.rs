//! Live prompt target prediction, incremental declaration scanning, and graph focus tracking.
use crate::{Result, graph_map::Index, store::SessionStore};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

const MAX_BASELINE_FILES: usize = 128;
const MAX_SCAN_BYTES: u64 = 256 * 1024;
const MAX_WALK_ENTRIES: usize = 1500;
const MAX_TARGET_FOLDERS: usize = 8;
const MAX_TARGET_PATHS: usize = 16;
const MAX_TARGET_NODES: usize = 24;
const MAX_CREATED_ENTITIES: usize = 64;
const SESSION_FOCUS_PREFIX: &str = "graph_focus.";

const CODE_EXTENSIONS: &[&str] = &[
    "php", "phtml", "inc", "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts", "py", "pyi",
    "json", "toml", "md", "css", "html", "sql", "yml", "yaml",
];

const STOP_WORDS: &[&str] = &[
    "about",
    "above",
    "after",
    "again",
    "against",
    "all",
    "also",
    "and",
    "any",
    "are",
    "around",
    "because",
    "been",
    "before",
    "being",
    "below",
    "between",
    "both",
    "but",
    "can",
    "change",
    "changes",
    "check",
    "class",
    "code",
    "could",
    "create",
    "created",
    "current",
    "does",
    "done",
    "each",
    "edit",
    "edited",
    "else",
    "ensure",
    "every",
    "file",
    "files",
    "find",
    "fix",
    "fixed",
    "folder",
    "folders",
    "for",
    "from",
    "function",
    "functions",
    "get",
    "give",
    "have",
    "here",
    "how",
    "implement",
    "implemented",
    "include",
    "into",
    "just",
    "keep",
    "like",
    "make",
    "method",
    "methods",
    "module",
    "modules",
    "more",
    "most",
    "move",
    "must",
    "need",
    "needs",
    "new",
    "next",
    "not",
    "now",
    "only",
    "open",
    "other",
    "our",
    "out",
    "over",
    "path",
    "paths",
    "please",
    "project",
    "read",
    "refactor",
    "remove",
    "replace",
    "return",
    "run",
    "same",
    "see",
    "set",
    "should",
    "show",
    "some",
    "start",
    "struct",
    "structs",
    "such",
    "support",
    "take",
    "target",
    "task",
    "test",
    "tests",
    "than",
    "that",
    "the",
    "their",
    "them",
    "then",
    "there",
    "these",
    "they",
    "this",
    "those",
    "through",
    "trait",
    "traits",
    "under",
    "until",
    "update",
    "updated",
    "use",
    "used",
    "using",
    "verify",
    "want",
    "was",
    "were",
    "what",
    "when",
    "where",
    "which",
    "while",
    "will",
    "with",
    "within",
    "without",
    "work",
    "write",
    "written",
    "your",
];

/// Metadata for a file or symbol created or modified during a live run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatedEntity {
    pub id: String,
    pub kind: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub path: String,
    pub folder: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<String>,
}

impl CreatedEntity {
    fn new(
        kind: &str,
        name: impl Into<String>,
        parent: Option<String>,
        path: impl Into<String>,
        line: Option<u32>,
    ) -> Self {
        let name = name.into();
        let path = path.into();
        let folder = folder_of(&path);
        let id = if kind == "file" {
            format!("live:file:{path}")
        } else if matches!(kind, "method" | "property" | "field" | "const")
            && let Some(owner) = parent.as_deref().filter(|s| !s.is_empty() && *s != path)
        {
            format!("live:{kind}:{path}:{owner}::{name}")
        } else {
            format!("live:{kind}:{path}:{name}")
        };
        Self {
            id,
            kind: kind.to_owned(),
            name,
            parent,
            path,
            folder,
            line,
            change: None,
            visibility: None,
            modifiers: Vec::new(),
        }
    }

    pub(crate) fn with_access(
        mut self,
        visibility: Option<String>,
        modifiers: Vec<String>,
    ) -> Self {
        self.visibility = visibility;
        self.modifiers = modifiers;
        self
    }

    pub(crate) fn modified_from_decl(decl: &Self) -> Self {
        let mut out = decl.clone();
        out.id = if matches!(out.kind.as_str(), "method" | "property" | "field" | "const")
            && let Some(owner) = out
                .parent
                .as_deref()
                .filter(|s| !s.is_empty() && *s != out.path)
        {
            format!("live:mod:{}:{}:{owner}::{}", out.kind, out.path, out.name)
        } else {
            format!("live:mod:{}:{}:{}", out.kind, out.path, out.name)
        };
        out.change = Some("modified".to_owned());
        out
    }

    fn identity_key(&self) -> (String, Option<String>, String, String) {
        (
            self.kind.clone(),
            self.parent.clone(),
            self.path.clone(),
            self.name.clone(),
        )
    }
}

/// Live prompt focus, predicted/touched regions, and newly created entities for the graph UI.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphFocus {
    pub phase: String,
    pub summary: String,
    #[serde(default)]
    pub target_folders: Vec<String>,
    #[serde(default)]
    pub target_paths: Vec<String>,
    #[serde(default)]
    pub target_nodes: Vec<String>,
    #[serde(default)]
    pub created_entities: Vec<CreatedEntity>,
}

impl GraphFocus {
    /// Converts this focus state into a live session event payload (`"kind": "graph_focus"`).
    pub fn to_event(&self) -> Value {
        json!({
            "kind": "graph_focus",
            "phase": self.phase,
            "summary": self.summary,
            "target_folders": self.target_folders,
            "target_paths": self.target_paths,
            "target_nodes": self.target_nodes,
            "created_entities": self.created_entities,
        })
    }

    /// Returns `true` when there are no predicted/touched targets or created entities.
    pub fn is_empty(&self) -> bool {
        self.target_folders.is_empty()
            && self.target_paths.is_empty()
            && self.target_nodes.is_empty()
            && self.created_entities.is_empty()
    }
}

/// Saves a session's `GraphFocus` metadata to `harness_settings` (never storing file bodies).
pub fn save_session_focus(store: &SessionStore, sid: &str, focus: &GraphFocus) -> Result<()> {
    let key = format!("{SESSION_FOCUS_PREFIX}{sid}");
    store.set_setting(&key, &serde_json::to_value(focus)?)
}

/// Loads a stored `GraphFocus` for `sid`, if any.
pub fn session_focus(store: &SessionStore, sid: &str) -> Result<Option<GraphFocus>> {
    let key = format!("{SESSION_FOCUS_PREFIX}{sid}");
    let Some(value) = store.setting(&key)? else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    Ok(serde_json::from_value(value).ok())
}

/// Returns the JSON `GraphFocus` object for `sid` (or `Value::Null` when absent).
pub fn session_focus_value(store: &SessionStore, sid: &str) -> Result<Value> {
    let key = format!("{SESSION_FOCUS_PREFIX}{sid}");
    let Some(value) = store.setting(&key)? else {
        return Ok(Value::Null);
    };
    if value.is_null() {
        return Ok(Value::Null);
    }
    Ok(match serde_json::from_value::<GraphFocus>(value) {
        Ok(focus) => serde_json::to_value(focus)?,
        Err(_) => Value::Null,
    })
}

/// Extracts capsule and attachment paths from the session record and predicts initial graph focus.
pub fn predict_session_targets(
    store: &SessionStore,
    sid: &str,
    session: &Value,
    workspace: &Path,
    index: Option<&Index>,
    prompt: &str,
) -> GraphFocus {
    let mut capsule_paths = Vec::new();
    if let Some(hashes) = session["brain"]["source_hashes"].as_object() {
        for key in hashes.keys() {
            capsule_paths.push(key.clone());
        }
    }
    if let Some(text) = session["brain"]["capsule_text"].as_str() {
        for line in text.lines().take(200) {
            let trimmed = line.trim();
            let candidate = trimmed
                .split(" § ")
                .next()
                .unwrap_or("")
                .trim_start_matches(['/', '#', '-', '['])
                .trim_end_matches(']')
                .trim();
            if looks_like_source_path(candidate) {
                capsule_paths.push(candidate.to_owned());
            }
        }
    }
    if session["omnigraph"]["enabled"] == true {
        for atom in session["omnigraph"]["injected_atoms"]
            .as_array()
            .into_iter()
            .flatten()
        {
            for path in atom["files"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                let trimmed = path.trim();
                if !trimmed.is_empty() {
                    capsule_paths.push(trimmed.to_owned());
                }
            }
        }
        if let Some(text) = session["omnigraph"]["capsule_text"].as_str() {
            for line in text.lines().take(200) {
                for token in extract_explicit_paths(line) {
                    let candidate = strip_line_suffix(&token).trim();
                    if looks_like_source_path(candidate) {
                        capsule_paths.push(candidate.to_owned());
                    }
                }
            }
        }
    }

    let mut attachment_paths = Vec::new();
    if let Ok(events) = store.events(sid, 0)
        && let Some(user_event) = events.iter().rev().find(|e| e["kind"] == "user")
        && let Some(items) = user_event["attachments"].as_array()
    {
        for item in items {
            if let Some(path) = item["path"].as_str().or_else(|| item["name"].as_str())
                && !path.is_empty()
            {
                attachment_paths.push(path.to_owned());
            }
        }
    }

    predict_targets(workspace, index, prompt, &capsule_paths, &attachment_paths)
}

/// Marks a stored session focus as `"completed"` if it exists and is still active or predicting.
pub fn complete_stored_session_focus(
    store: &SessionStore,
    sid: &str,
) -> Result<Option<GraphFocus>> {
    let Some(mut focus) = session_focus(store, sid)? else {
        return Ok(None);
    };
    if focus.phase == "completed" {
        return Ok(Some(focus));
    }
    focus.phase = "completed".to_owned();
    focus.summary = summarize_focus(
        &focus.phase,
        &focus.target_folders,
        &focus.target_paths,
        &focus.created_entities,
    );
    save_session_focus(store, sid, &focus)?;
    Ok(Some(focus))
}

/// Returns the live or most recent `GraphFocus` JSON value for a project (or `Value::Null`).
pub fn project_focus_value(
    store: &SessionStore,
    project: &str,
    focus_session: Option<&str>,
) -> Result<Value> {
    if let Some(sid) = focus_session.map(str::trim).filter(|sid| !sid.is_empty()) {
        if let Ok(session) = store.get(sid)
            && session["project_id"].as_str() != Some(project)
        {
            return Ok(Value::Null);
        }
        return session_focus_value(store, sid);
    }
    let sessions = store.summaries()?;
    let active_statuses = ["running", "starting", "queued", "active"];
    for pass_active in [true, false] {
        for session in &sessions {
            if session["project_id"].as_str() != Some(project) {
                continue;
            }
            let status = session["status"].as_str().unwrap_or("");
            if pass_active && !active_statuses.contains(&status) {
                continue;
            }
            let Some(sid) = session["id"].as_str() else {
                continue;
            };
            let focus = session_focus_value(store, sid)?;
            if !focus.is_null() {
                return Ok(focus);
            }
        }
    }
    Ok(Value::Null)
}

/// Predicts target folders, files, and graph nodes at prompt start before spawning the provider CLI.
pub fn predict_targets(
    project_root: &Path,
    index: Option<&Index>,
    prompt: &str,
    capsule_paths: &[String],
    attachment_paths: &[String],
) -> GraphFocus {
    let indexed_set: BTreeSet<&str> = index
        .map(|idx| idx.indexed_files().iter().map(String::as_str).collect())
        .unwrap_or_default();
    let indexed_folders = index.map(collect_indexed_folders).unwrap_or_default();

    let mut folders = UniqueList::new(MAX_TARGET_FOLDERS);
    let mut paths = UniqueList::new(MAX_TARGET_PATHS);
    let mut nodes = UniqueList::new(MAX_TARGET_NODES);

    // 1. Attachments referenced on the turn.
    for raw in attachment_paths {
        if let Some(rel) = normalize_rel_path(project_root, raw) {
            if is_known_file(&rel, project_root, &indexed_set) {
                add_file_target(&rel, &mut folders, &mut paths, &mut nodes);
            } else if let Some(idx) = index {
                add_suffix_or_find_matches(
                    &rel,
                    idx,
                    &indexed_set,
                    &mut folders,
                    &mut paths,
                    &mut nodes,
                );
            }
        }
    }

    // 2. Explicit path tokens directly mentioned in the prompt.
    let explicit_tokens = extract_explicit_paths(prompt);
    for token in &explicit_tokens {
        let Some(rel) = normalize_rel_path(project_root, token) else {
            continue;
        };
        if is_known_file(&rel, project_root, &indexed_set) {
            add_file_target(&rel, &mut folders, &mut paths, &mut nodes);
        } else if is_known_folder(&rel, project_root, &indexed_folders) {
            add_folder_target(&rel, &mut folders, &mut nodes);
        } else if let Some(idx) = index {
            add_suffix_or_find_matches(
                &rel,
                idx,
                &indexed_set,
                &mut folders,
                &mut paths,
                &mut nodes,
            );
            if looks_like_source_path(&rel) {
                add_existing_ancestor_folder(
                    &rel,
                    project_root,
                    &indexed_folders,
                    &mut folders,
                    &mut nodes,
                );
                paths.push(&rel);
                nodes.push(&rel);
            }
        } else if looks_like_source_path(&rel) {
            add_existing_ancestor_folder(
                &rel,
                project_root,
                &indexed_folders,
                &mut folders,
                &mut nodes,
            );
            paths.push(&rel);
            nodes.push(&rel);
        }
    }

    // 3. Symbol / identifier / domain keyword resolution via `Index::find` or workspace paths.
    let queries = extract_search_queries(prompt);
    if let Some(idx) = index {
        for query in &queries {
            let reply = idx.find(query);
            let Some(rows) = reply["rows"].as_array() else {
                continue;
            };
            for row in rows.iter().take(6) {
                let what = row["what"].as_str().unwrap_or("");
                match what {
                    "folder" => {
                        if let Some(path) = row["path"].as_str().filter(|p| !p.is_empty()) {
                            add_folder_target(path, &mut folders, &mut nodes);
                        }
                    }
                    "file" => {
                        if let Some(path) = row["path"].as_str().filter(|p| !p.is_empty()) {
                            add_file_target(path, &mut folders, &mut paths, &mut nodes);
                        }
                    }
                    _ => {
                        if let Some(file) = row["file"].as_str().filter(|p| !p.is_empty()) {
                            add_file_target(file, &mut folders, &mut paths, &mut nodes);
                        }
                        if let Some(node_key) = row["path"].as_str().filter(|p| !p.is_empty()) {
                            nodes.push(node_key);
                        }
                    }
                }
            }
        }
    } else if !queries.is_empty() && project_root.is_dir() {
        match_workspace_paths(project_root, &queries, &mut folders, &mut paths, &mut nodes);
    }

    // 4. Context capsule paths from knowledge prep.
    for raw in capsule_paths {
        let Some(rel) = normalize_rel_path(project_root, raw) else {
            continue;
        };
        if is_known_file(&rel, project_root, &indexed_set) {
            add_file_target(&rel, &mut folders, &mut paths, &mut nodes);
        } else if is_known_folder(&rel, project_root, &indexed_folders) {
            add_folder_target(&rel, &mut folders, &mut nodes);
        }
    }

    let summary = summarize_focus("predicting", &folders.items, &paths.items, &[]);
    GraphFocus {
        phase: "predicting".to_owned(),
        summary,
        target_folders: folders.items,
        target_paths: paths.items,
        target_nodes: nodes.items,
        created_entities: Vec::new(),
    }
}

/// Extracted declaration metadata together with its normalized signature and body fingerprint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExtractedDecl {
    pub entity: CreatedEntity,
    pub signature: String,
    pub params_signature: String,
    pub body_fingerprint: String,
    pub visibility: Option<String>,
    pub modifiers: Vec<String>,
    pub start_line: u32,
    pub end_line: u32,
}

/// Extracts top-level and class/impl declarations with signatures and body fingerprints.
pub(crate) fn extract_declaration_spans(rel_path: &str, source: &str) -> Vec<ExtractedDecl> {
    let rel = clean_rel_key(rel_path);
    if rel.is_empty() {
        return Vec::new();
    }
    match detect_language(&rel) {
        Some(Language::Php) => extract_php_decls(&rel, source),
        Some(Language::Rust) => extract_rust_decls(&rel, source),
        Some(Language::TsJs) => extract_ts_js_decls(&rel, source),
        Some(Language::Python) => extract_python_decls(&rel, source),
        None => Vec::new(),
    }
}

/// Extracts top-level and class/impl declarations from a source file without executing code.
pub fn extract_declarations(rel_path: &str, source: &str) -> Vec<CreatedEntity> {
    extract_declaration_spans(rel_path, source)
        .into_iter()
        .map(|decl| decl.entity)
        .collect()
}

/// Computes declarations modified (signature or own body changed) between `before` and `after`.
pub(crate) fn diff_modified_entities(
    rel_path: &str,
    before: &str,
    after: &str,
) -> Vec<CreatedEntity> {
    let rel = clean_rel_key(rel_path);
    if rel.is_empty() || detect_language(&rel).is_none() {
        return Vec::new();
    }
    let before_decls = extract_declaration_spans(&rel, before);
    let after_decls = extract_declaration_spans(&rel, after);

    type DeclKey = (String, Option<String>, String);
    let mut before_by_key: BTreeMap<DeclKey, Vec<ExtractedDecl>> = BTreeMap::new();
    for decl in before_decls {
        let key = (
            decl.entity.kind.clone(),
            decl.entity.parent.clone(),
            decl.entity.name.clone(),
        );
        before_by_key.entry(key).or_default().push(decl);
    }

    let mut out = Vec::new();
    let mut matched_exact: BTreeMap<DeclKey, Vec<bool>> = before_by_key
        .iter()
        .map(|(k, v)| (k.clone(), vec![false; v.len()]))
        .collect();
    let mut pending_after: Vec<(DeclKey, ExtractedDecl)> = Vec::new();

    for decl in after_decls {
        let key = (
            decl.entity.kind.clone(),
            decl.entity.parent.clone(),
            decl.entity.name.clone(),
        );
        let Some(prev_list) = before_by_key.get(&key) else {
            continue;
        };
        let used = matched_exact.get_mut(&key).expect("matching key");
        if let Some(pos) = prev_list.iter().enumerate().position(|(idx, prev)| {
            !used[idx]
                && prev.signature == decl.signature
                && prev.body_fingerprint == decl.body_fingerprint
                && prev.visibility == decl.visibility
                && prev.modifiers == decl.modifiers
        }) {
            used[pos] = true;
        } else {
            pending_after.push((key, decl));
        }
    }

    for (key, decl) in pending_after {
        if let Some(used) = matched_exact.get_mut(&key)
            && let Some(pos) = used.iter().position(|flag| !*flag)
        {
            used[pos] = true;
            out.push(CreatedEntity::modified_from_decl(&decl.entity));
        }
    }
    out
}

/// Computes declarations newly introduced in `rel_path` by comparing against an optional pre-edit baseline.
///
/// When `is_new_file` is `true`, emits a `"file"` entity plus all extracted declarations.
/// When `is_new_file` is `false` and `before` is `None`, returns empty to prevent false positives on existing code.
pub fn diff_new_entities(
    rel_path: &str,
    before: Option<&str>,
    after: &str,
    is_new_file: bool,
) -> Vec<CreatedEntity> {
    let rel = clean_rel_key(rel_path);
    if rel.is_empty() {
        return Vec::new();
    }
    let after_entities = extract_declarations(&rel, after);
    if is_new_file {
        let folder = folder_of(&rel);
        let file_entity = CreatedEntity::new(
            "file",
            file_name(&rel),
            (!folder.is_empty()).then_some(folder),
            rel,
            Some(1),
        );
        let mut out = Vec::with_capacity(1 + after_entities.len());
        out.push(file_entity);
        out.extend(after_entities);
        return out;
    }
    let Some(before_src) = before else {
        return Vec::new();
    };
    let existing: BTreeSet<(String, Option<String>, String)> =
        extract_declarations(&rel, before_src)
            .into_iter()
            .map(|entity| (entity.kind, entity.parent, entity.name))
            .collect();
    after_entities
        .into_iter()
        .filter(|entity| {
            !existing.contains(&(
                entity.kind.clone(),
                entity.parent.clone(),
                entity.name.clone(),
            ))
        })
        .collect()
}

/// Stateful per-run tracker capturing lazy pre-edit baselines and live created-entity deltas.
#[derive(Clone, Debug, Default)]
pub struct LiveFocusTracker {
    root: PathBuf,
    focus: GraphFocus,
    baselines: BTreeMap<String, Option<String>>,
    entity_keys: BTreeSet<(String, Option<String>, String, String)>,
    dirty: bool,
}

impl LiveFocusTracker {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            focus: GraphFocus {
                phase: "predicting".to_owned(),
                summary: "Analyzing prompt across workspace".to_owned(),
                ..GraphFocus::default()
            },
            baselines: BTreeMap::new(),
            entity_keys: BTreeSet::new(),
            dirty: false,
        }
    }

    pub fn set_root(&mut self, root: PathBuf) {
        self.root = root;
    }

    pub fn focus(&self) -> &GraphFocus {
        &self.focus
    }

    /// Seeds initial predicted targets and lazily snapshots those files if already on disk.
    pub fn seed_focus(&mut self, initial: GraphFocus) {
        let paths = initial.target_paths.clone();
        self.focus = initial;
        self.dirty = false;
        for rel in &paths {
            self.capture_baseline(rel);
        }
    }

    /// Captures pre-edit baseline lazily for `raw_path` if inside workspace and within limits.
    pub fn capture_baseline(&mut self, raw_path: &str) {
        if self.root.as_os_str().is_empty() || self.baselines.len() >= MAX_BASELINE_FILES {
            return;
        }
        let Some(rel) = self.normalize_workspace_path(raw_path) else {
            return;
        };
        if self.baselines.contains_key(&rel) {
            return;
        }
        let full_path = self.root.join(&rel);
        let snapshot = read_bounded_utf8(&full_path);
        self.baselines.insert(rel, snapshot);
    }

    /// Records a file read so we already hold a pre-edit snapshot if edited later.
    pub fn record_read(&mut self, raw_path: &str) {
        self.capture_baseline(raw_path);
    }

    /// Snapshots baseline before a write/edit begins and expands live target highlight immediately.
    pub fn record_mutation_start(&mut self, raw_path: &str) -> bool {
        let Some(rel) = self.normalize_workspace_path(raw_path) else {
            return false;
        };
        self.capture_baseline(&rel);
        let mut changed = false;
        if self.focus.phase == "predicting" || self.focus.phase == "idle" {
            self.focus.phase = "active".to_owned();
            changed = true;
        }
        changed |= add_unique(&mut self.focus.target_paths, &rel, MAX_TARGET_PATHS);
        let folder = folder_of(&rel);
        if !folder.is_empty() {
            changed |= add_unique(&mut self.focus.target_folders, &folder, MAX_TARGET_FOLDERS);
            changed |= add_unique(&mut self.focus.target_nodes, &folder, MAX_TARGET_NODES);
        }
        changed |= add_unique(&mut self.focus.target_nodes, &rel, MAX_TARGET_NODES);
        if changed {
            self.refresh_summary();
            self.dirty = true;
        }
        changed
    }

    /// Scans a completed mutation on `raw_path` and merges any newly created entities and target regions.
    pub fn record_mutation_complete(&mut self, raw_path: &str, change_kind: Option<&str>) -> bool {
        let Some(rel) = self.normalize_workspace_path(raw_path) else {
            return false;
        };
        let kind = change_kind.unwrap_or("").trim().to_ascii_lowercase();
        let mut changed = false;
        if self.focus.phase != "active" {
            self.focus.phase = "active".to_owned();
            changed = true;
        }

        if matches!(kind.as_str(), "d" | "delete" | "deleted") {
            let before_len = self.focus.created_entities.len();
            self.focus.created_entities.retain(|e| e.path != rel);
            if self.focus.created_entities.len() != before_len {
                self.entity_keys = self
                    .focus
                    .created_entities
                    .iter()
                    .map(CreatedEntity::identity_key)
                    .collect();
                changed = true;
            }
            self.baselines.insert(rel.clone(), None);
            if changed {
                self.refresh_summary();
                self.dirty = true;
            }
            return changed;
        }

        changed |= add_unique(&mut self.focus.target_paths, &rel, MAX_TARGET_PATHS);
        let folder = folder_of(&rel);
        if !folder.is_empty() {
            changed |= add_unique(&mut self.focus.target_folders, &folder, MAX_TARGET_FOLDERS);
            changed |= add_unique(&mut self.focus.target_nodes, &folder, MAX_TARGET_NODES);
        }
        changed |= add_unique(&mut self.focus.target_nodes, &rel, MAX_TARGET_NODES);

        if !self.root.as_os_str().is_empty() {
            let full_path = self.root.join(&rel);
            if let Some(after_text) = read_bounded_utf8(&full_path) {
                let (before_opt, is_new_file) = match self.baselines.get(&rel) {
                    Some(None) => (None, true),
                    Some(Some(old)) => (Some(old.clone()), false),
                    None => (None, matches!(kind.as_str(), "a" | "add" | "added")),
                };
                let added =
                    diff_new_entities(&rel, before_opt.as_deref(), &after_text, is_new_file);
                for mut entity in added {
                    if self.focus.created_entities.len() >= MAX_CREATED_ENTITIES {
                        break;
                    }
                    entity.change = Some("created".to_owned());
                    let key = entity.identity_key();
                    if self.entity_keys.insert(key) {
                        self.focus.created_entities.push(entity);
                        changed = true;
                    }
                }
                if !is_new_file && let Some(before_src) = before_opt.as_deref() {
                    for entity in diff_modified_entities(&rel, before_src, &after_text) {
                        if self.focus.created_entities.len() >= MAX_CREATED_ENTITIES {
                            break;
                        }
                        let key = entity.identity_key();
                        if self.entity_keys.insert(key) {
                            self.focus.created_entities.push(entity);
                            changed = true;
                        }
                    }
                }
                if self.baselines.len() < MAX_BASELINE_FILES || self.baselines.contains_key(&rel) {
                    self.baselines.insert(rel, Some(after_text));
                }
            }
        }

        if changed {
            self.refresh_summary();
            self.dirty = true;
        }
        changed
    }

    /// Marks the run focus phase as `"completed"`.
    pub fn complete(&mut self) -> bool {
        if self.focus.phase == "completed" {
            return false;
        }
        self.focus.phase = "completed".to_owned();
        self.refresh_summary();
        self.dirty = true;
        true
    }

    /// Observes a cleaned session `"tool"` event (`started` / `completed`) and returns a live delta event if focus changed.
    pub fn observe_tool_event(&mut self, event: &Value) -> Option<Value> {
        if event["kind"] != "tool" {
            return None;
        }
        let state = event["state"].as_str().unwrap_or("");
        let tool = event["tool"].as_str().unwrap_or("").to_ascii_lowercase();
        let ok = event["ok"] != false;
        let has_changes = event["changes"]
            .as_array()
            .is_some_and(|changes| !changes.is_empty());
        let is_read = tool == "read";
        let is_mutation = matches!(
            tool.as_str(),
            "edit" | "write" | "multiedit" | "notebookedit" | "file_change" | "delete"
        ) || has_changes;

        if state == "started" {
            if is_read
                && event["outside"] != true
                && let Some(path) = event["path"].as_str()
            {
                self.record_read(path);
            }
            if is_mutation {
                if event["outside"] != true
                    && let Some(path) = event["path"].as_str()
                {
                    self.record_mutation_start(path);
                }
                if let Some(changes) = event["changes"].as_array() {
                    for change in changes {
                        if change["outside"] != true
                            && let Some(path) = change["path"].as_str()
                        {
                            self.record_mutation_start(path);
                        }
                    }
                }
            }
        } else if ok {
            if is_read
                && event["outside"] != true
                && let Some(path) = event["path"].as_str()
            {
                self.record_read(path);
            }
            if is_mutation {
                if let Some(changes) = event["changes"].as_array().filter(|arr| !arr.is_empty()) {
                    for change in changes {
                        if change["outside"] != true
                            && let Some(path) = change["path"].as_str()
                        {
                            self.record_mutation_complete(path, change["kind"].as_str());
                        }
                    }
                } else if event["outside"] != true
                    && let Some(path) = event["path"].as_str()
                {
                    let kind = (tool == "delete").then_some("delete");
                    self.record_mutation_complete(path, kind);
                }
            }
        }
        self.take_delta_event()
    }

    /// Returns a `"kind": "graph_focus"` event if state changed since the last delta check.
    pub fn take_delta_event(&mut self) -> Option<Value> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        Some(self.focus.to_event())
    }

    fn normalize_workspace_path(&self, raw: &str) -> Option<String> {
        if self.root.as_os_str().is_empty() {
            let cleaned = clean_rel_key(raw);
            return (!cleaned.is_empty()).then_some(cleaned);
        }
        normalize_rel_path(&self.root, raw)
    }

    fn refresh_summary(&mut self) {
        self.focus.summary = summarize_focus(
            &self.focus.phase,
            &self.focus.target_folders,
            &self.focus.target_paths,
            &self.focus.created_entities,
        );
    }
}

struct UniqueList {
    items: Vec<String>,
    seen: BTreeSet<String>,
    limit: usize,
}

impl UniqueList {
    fn new(limit: usize) -> Self {
        Self {
            items: Vec::new(),
            seen: BTreeSet::new(),
            limit,
        }
    }

    fn push(&mut self, value: &str) -> bool {
        if self.items.len() >= self.limit || value.is_empty() {
            return false;
        }
        if self.seen.insert(value.to_owned()) {
            self.items.push(value.to_owned());
            true
        } else {
            false
        }
    }
}

fn add_unique(target: &mut Vec<String>, value: &str, limit: usize) -> bool {
    if value.is_empty() || target.len() >= limit || target.iter().any(|existing| existing == value)
    {
        return false;
    }
    target.push(value.to_owned());
    true
}

fn add_folder_target(folder: &str, folders: &mut UniqueList, nodes: &mut UniqueList) {
    let cleaned = folder.trim_matches('/');
    if cleaned.is_empty() {
        return;
    }
    folders.push(cleaned);
    nodes.push(cleaned);
}

fn add_file_target(
    rel: &str,
    folders: &mut UniqueList,
    paths: &mut UniqueList,
    nodes: &mut UniqueList,
) {
    paths.push(rel);
    nodes.push(rel);
    let folder = folder_of(rel);
    if !folder.is_empty() {
        folders.push(&folder);
        nodes.push(&folder);
    }
}

fn add_existing_ancestor_folder(
    rel: &str,
    project_root: &Path,
    indexed_folders: &BTreeSet<String>,
    folders: &mut UniqueList,
    nodes: &mut UniqueList,
) {
    let mut cur = folder_of(rel);
    while !cur.is_empty() {
        if is_known_folder(&cur, project_root, indexed_folders) {
            add_folder_target(&cur, folders, nodes);
            break;
        }
        cur = folder_of(&cur);
    }
}

fn add_suffix_or_find_matches(
    rel: &str,
    index: &Index,
    indexed_set: &BTreeSet<&str>,
    folders: &mut UniqueList,
    paths: &mut UniqueList,
    nodes: &mut UniqueList,
) {
    let suffix = format!("/{rel}");
    let mut matched = false;
    for indexed in indexed_set {
        if *indexed == rel || indexed.ends_with(&suffix) {
            add_file_target(indexed, folders, paths, nodes);
            matched = true;
            if paths.items.len() >= MAX_TARGET_PATHS {
                break;
            }
        }
    }
    if matched {
        return;
    }
    let query = rel.replace('/', " ");
    let reply = index.find(&query);
    if let Some(rows) = reply["rows"].as_array() {
        for row in rows.iter().take(4) {
            match row["what"].as_str().unwrap_or("") {
                "folder" => {
                    if let Some(p) = row["path"].as_str() {
                        add_folder_target(p, folders, nodes);
                    }
                }
                "file" => {
                    if let Some(p) = row["path"].as_str() {
                        add_file_target(p, folders, paths, nodes);
                    }
                }
                _ => {
                    if let Some(file) = row["file"].as_str() {
                        add_file_target(file, folders, paths, nodes);
                    }
                    if let Some(p) = row["path"].as_str() {
                        nodes.push(p);
                    }
                }
            }
        }
    }
}

fn collect_indexed_folders(index: &Index) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for path in index.indexed_files() {
        let mut cur = folder_of(path);
        while !cur.is_empty() {
            out.insert(cur.clone());
            cur = folder_of(&cur);
        }
    }
    out
}

fn is_known_file(rel: &str, root: &Path, indexed: &BTreeSet<&str>) -> bool {
    indexed.contains(rel) || (!root.as_os_str().is_empty() && root.join(rel).is_file())
}

fn is_known_folder(rel: &str, root: &Path, indexed_folders: &BTreeSet<String>) -> bool {
    indexed_folders.contains(rel) || (!root.as_os_str().is_empty() && root.join(rel).is_dir())
}

fn looks_like_source_path(rel: &str) -> bool {
    if !rel.contains('/') && !rel.contains('.') {
        return false;
    }
    let lower = rel.to_ascii_lowercase();
    CODE_EXTENSIONS
        .iter()
        .any(|ext| lower.ends_with(&format!(".{ext}")))
}

fn read_bounded_utf8(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_SCAN_BYTES {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    String::from_utf8(bytes).ok()
}

pub(crate) fn clean_rel_key(raw: &str) -> String {
    let trimmed = raw
        .trim()
        .trim_matches(|c| matches!(c, '`' | '"' | '\'' | ',' | ';'));
    let normalized = trimmed.replace('\\', "/");
    let mut parts = Vec::new();
    for part in normalized.split('/') {
        match part {
            "" | "." => {}
            ".." => return String::new(),
            seg => parts.push(seg),
        }
    }
    parts.join("/")
}

fn normalize_rel_path(project_root: &Path, raw: &str) -> Option<String> {
    let cleaned = raw
        .trim()
        .trim_matches(|c| matches!(c, '`' | '"' | '\'' | '(' | ')' | '[' | ']' | '<' | '>'));
    if cleaned.is_empty() {
        return None;
    }
    let cleaned = strip_line_suffix(cleaned).replace('\\', "/");
    let candidate = Path::new(&cleaned);
    let rel_path = if candidate.is_absolute() {
        if project_root.as_os_str().is_empty() {
            return None;
        }
        if let Ok(stripped) = candidate.strip_prefix(project_root) {
            stripped.to_path_buf()
        } else if let Ok(real_root) = infra_core::fs::python_realpath(project_root)
            && let Ok(stripped) = candidate.strip_prefix(&real_root)
        {
            stripped.to_path_buf()
        } else if let Ok(real_cand) = infra_core::fs::python_realpath(candidate)
            && let Ok(real_root) = infra_core::fs::python_realpath(project_root)
            && let Ok(stripped) = real_cand.strip_prefix(&real_root)
        {
            stripped.to_path_buf()
        } else {
            return None;
        }
    } else {
        PathBuf::from(&cleaned)
    };

    let mut out = Vec::new();
    for comp in rel_path.components() {
        match comp {
            Component::Normal(seg) => {
                let s = seg.to_str()?;
                if !s.is_empty() {
                    out.push(s);
                }
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    let joined = out.join("/");
    (!joined.is_empty()).then_some(joined)
}

fn strip_line_suffix(raw: &str) -> &str {
    let no_hash = raw.split('#').next().unwrap_or(raw);
    if let Some((head, tail)) = no_hash.rsplit_once(':')
        && !head.is_empty()
        && tail.chars().all(|c| c.is_ascii_digit())
    {
        return head;
    }
    no_hash
}

pub(crate) fn folder_of(rel_path: &str) -> String {
    rel_path
        .rsplit_once('/')
        .map_or_else(String::new, |(dir, _)| dir.to_owned())
}

pub(crate) fn file_name(rel_path: &str) -> String {
    rel_path
        .rsplit_once('/')
        .map_or(rel_path, |(_, name)| name)
        .to_owned()
}

fn summarize_focus(
    phase: &str,
    folders: &[String],
    paths: &[String],
    entities: &[CreatedEntity],
) -> String {
    if !entities.is_empty() {
        let total = entities.len();
        let modified_count = entities
            .iter()
            .filter(|e| e.change.as_deref() == Some("modified"))
            .count();
        let created_count = total.saturating_sub(modified_count);
        let noun = if total == 1 { "entity" } else { "entities" };
        let scope = if !folders.is_empty() {
            format!(" in {}", format_sample(folders, 2))
        } else if !paths.is_empty() {
            format!(" in {}", file_name(&paths[0]))
        } else {
            String::new()
        };
        if modified_count == 0 {
            return if phase == "completed" {
                format!("Created {total} {noun}{scope}")
            } else {
                format!("Live: {total} new {noun}{scope}")
            };
        }
        if created_count == 0 {
            return if phase == "completed" {
                format!("Modified {modified_count} {noun}{scope}")
            } else {
                format!("Live: {modified_count} modified {noun}{scope}")
            };
        }
        return if phase == "completed" {
            format!(
                "Changed {total} {noun} ({created_count} new, {modified_count} modified){scope}"
            )
        } else {
            format!("Live: {created_count} new, {modified_count} modified {noun}{scope}")
        };
    }
    if !folders.is_empty() {
        let verb = match phase {
            "completed" => "Updated",
            "active" => "Working in",
            _ => "Targeting",
        };
        if folders.len() == 1 {
            return format!("{verb} {}", folders[0]);
        }
        return format!(
            "{verb} {} modules: {}",
            folders.len(),
            format_sample(folders, 3)
        );
    }
    if !paths.is_empty() {
        let verb = match phase {
            "completed" => "Updated",
            "active" => "Working on",
            _ => "Targeting",
        };
        if paths.len() == 1 {
            return format!("{verb} {}", paths[0]);
        }
        return format!("{verb} {} files: {}", paths.len(), format_sample(paths, 2));
    }
    match phase {
        "completed" => "Run completed".to_owned(),
        "active" => "Scanning workspace changes".to_owned(),
        _ => "Analyzing prompt across workspace".to_owned(),
    }
}

fn format_sample(items: &[String], count: usize) -> String {
    let mut head = items.iter().take(count).cloned().collect::<Vec<_>>();
    if items.len() > count {
        head.push("...".to_owned());
    }
    head.join(", ")
}

fn extract_explicit_paths(prompt: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in
        prompt.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '"' | '\'' | '`'))
    {
        let token = raw.trim_matches(|c: char| {
            matches!(c, '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | '.' | ':')
        });
        if token.is_empty() || token.contains("://") {
            continue;
        }
        let has_slash = token.contains('/') || token.contains('\\');
        let lower = strip_line_suffix(token).to_ascii_lowercase();
        let has_ext = CODE_EXTENSIONS
            .iter()
            .any(|ext| lower.ends_with(&format!(".{ext}")));
        if (has_slash || has_ext) && seen.insert(token.to_owned()) {
            out.push(token.to_owned());
        }
    }
    out
}

fn extract_search_queries(prompt: &str) -> Vec<String> {
    let mut high_priority = Vec::new();
    let mut normal_priority = Vec::new();
    let mut seen = BTreeSet::new();

    // Prioritize backticked identifiers first.
    let mut chars = prompt.split('`');
    let _ = chars.next();
    while let Some(span) = chars.next() {
        let trimmed = span.trim();
        if !trimmed.is_empty() && trimmed.len() <= 80 && !trimmed.contains(' ') {
            push_symbol_queries(trimmed, &mut high_priority, &mut seen);
        }
        let _ = chars.next();
    }

    for raw in prompt.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                ',' | ';' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | '`'
            )
    }) {
        let token = raw.trim_matches(|c: char| matches!(c, '.' | ':' | '?' | '!' | '*' | '#'));
        if token.len() < 3 || token.contains("://") {
            continue;
        }
        let is_camel = token.chars().any(|c| c.is_ascii_lowercase())
            && token.chars().any(|c| c.is_ascii_uppercase());
        let is_snake = (token.contains('_') || token.contains('-'))
            && token.chars().any(|c| c.is_ascii_alphabetic());
        let is_qualified = token.contains("::") || token.contains('.');

        if is_camel || is_snake || is_qualified {
            push_symbol_queries(token, &mut high_priority, &mut seen);
        } else if token.chars().all(|c| c.is_ascii_alphanumeric()) {
            let lower = token.to_ascii_lowercase();
            if lower.len() >= 3 && !STOP_WORDS.contains(&lower.as_str()) && seen.insert(lower) {
                normal_priority.push(token.to_owned());
            }
        }
    }

    let mut out = high_priority;
    out.extend(normal_priority);
    out.truncate(14);
    out
}

fn push_symbol_queries(token: &str, out: &mut Vec<String>, seen: &mut BTreeSet<String>) {
    let cleaned = strip_line_suffix(token);
    if cleaned.contains('/') || cleaned.contains('\\') {
        let base = file_name(cleaned);
        let stem = base.split('.').next().unwrap_or(base.as_str());
        if stem.len() >= 3 && seen.insert(stem.to_ascii_lowercase()) {
            out.push(stem.to_owned());
        }
        return;
    }
    if cleaned.contains("::") {
        let spaced = cleaned.replace("::", " ");
        if seen.insert(spaced.to_ascii_lowercase()) {
            out.push(spaced);
        }
        for part in cleaned.split("::").filter(|p| p.len() >= 3) {
            if seen.insert(part.to_ascii_lowercase()) {
                out.push(part.to_owned());
            }
        }
        return;
    }
    if cleaned.contains('_') || cleaned.contains('-') {
        let lower = cleaned.to_ascii_lowercase();
        if seen.insert(lower.clone()) {
            out.push(cleaned.to_owned());
        }
        let spaced = cleaned.replace(['_', '-'], " ");
        if seen.insert(spaced.to_ascii_lowercase()) {
            out.push(spaced);
        }
        return;
    }
    if cleaned.contains('.') {
        let stem = cleaned.split('.').next().unwrap_or(cleaned);
        if stem.len() >= 3 && seen.insert(stem.to_ascii_lowercase()) {
            out.push(stem.to_owned());
        }
        return;
    }
    let lower = cleaned.to_ascii_lowercase();
    if !STOP_WORDS.contains(&lower.as_str()) && seen.insert(lower) {
        out.push(cleaned.to_owned());
    }
}

fn match_workspace_paths(
    project_root: &Path,
    queries: &[String],
    folders: &mut UniqueList,
    paths: &mut UniqueList,
    nodes: &mut UniqueList,
) {
    let tokens: Vec<String> = queries
        .iter()
        .flat_map(|q| q.split_whitespace())
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| s.len() >= 3)
        .collect();
    if tokens.is_empty() {
        return;
    }
    let mut visited = 0usize;
    let mut dir_hits: Vec<(usize, String)> = Vec::new();
    let mut file_hits: Vec<(usize, String)> = Vec::new();
    walk_dir_matching(
        project_root,
        "",
        0,
        &tokens,
        &mut visited,
        &mut dir_hits,
        &mut file_hits,
    );
    dir_hits.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.len().cmp(&b.1.len()))
            .then(a.1.cmp(&b.1))
    });
    file_hits.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.len().cmp(&b.1.len()))
            .then(a.1.cmp(&b.1))
    });
    for (_, file) in file_hits.into_iter().take(8) {
        add_file_target(&file, folders, paths, nodes);
    }
    for (_, dir) in dir_hits.into_iter().take(6) {
        add_folder_target(&dir, folders, nodes);
    }
}

fn walk_dir_matching(
    dir: &Path,
    prefix: &str,
    depth: usize,
    tokens: &[String],
    visited: &mut usize,
    dir_hits: &mut Vec<(usize, String)>,
    file_hits: &mut Vec<(usize, String)>,
) {
    if depth > 7 || *visited >= MAX_WALK_ENTRIES {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut children = entries.flatten().collect::<Vec<_>>();
    children.sort_by_key(|e| e.file_name());
    for entry in children {
        if *visited >= MAX_WALK_ENTRIES {
            break;
        }
        *visited += 1;
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if matches!(
            name_str,
            ".git"
                | "node_modules"
                | "target"
                | "vendor"
                | ".c2p"
                | "memory-bank"
                | ".worktrees"
                | ".idea"
                | ".vscode"
        ) {
            continue;
        }
        let rel = if prefix.is_empty() {
            name_str.to_owned()
        } else {
            format!("{prefix}/{name_str}")
        };
        let lower_name = name_str.to_ascii_lowercase();
        let stem = lower_name.split('.').next().unwrap_or(&lower_name);
        let score = tokens.iter().fold(0usize, |acc, tok| {
            if stem == tok || lower_name == *tok {
                acc + 3
            } else if lower_name.contains(tok.as_str()) {
                acc + 1
            } else {
                acc
            }
        });
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_dir() {
            if score > 0 {
                dir_hits.push((score, rel.clone()));
            }
            walk_dir_matching(
                &entry.path(),
                &rel,
                depth + 1,
                tokens,
                visited,
                dir_hits,
                file_hits,
            );
        } else if ft.is_file() && score > 0 {
            file_hits.push((score, rel));
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Language {
    Php,
    Rust,
    TsJs,
    Python,
}

pub(crate) fn detect_language(rel_path: &str) -> Option<Language> {
    let ext = rel_path.rsplit_once('.')?.1.to_ascii_lowercase();
    match ext.as_str() {
        "php" | "phtml" | "inc" => Some(Language::Php),
        "rs" => Some(Language::Rust),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts" => Some(Language::TsJs),
        "py" | "pyi" => Some(Language::Python),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BraceToken {
    Open,
    Close,
    Semi,
}

#[derive(Clone, Copy, Debug)]
struct BraceEvent {
    raw_byte: usize,
    tok: BraceToken,
    paren_depth: usize,
    bracket_depth: usize,
}

struct SanitizedLine {
    text: String,
    events: Vec<BraceEvent>,
    first_code_raw_byte: Option<usize>,
}

/// Sanitizes one line of C-like source (stripping comments and strings while tracking multi-line `/* ... */`).
pub(crate) fn sanitize_c_like_line(
    line: &str,
    in_block_comment: &mut bool,
    lang: Language,
) -> String {
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    sanitize_c_like_with_events(
        line,
        in_block_comment,
        &mut paren_depth,
        &mut bracket_depth,
        lang,
    )
    .text
}

fn sanitize_c_like_with_events(
    line: &str,
    in_block_comment: &mut bool,
    paren_depth: &mut usize,
    bracket_depth: &mut usize,
    lang: Language,
) -> SanitizedLine {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len);
    let mut events = Vec::new();
    let mut first_code_raw_byte = None;
    let mut i = 0usize;

    while i < len {
        if *in_block_comment {
            if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                *in_block_comment = false;
                out.push_str("  ");
                i += 2;
            } else {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            *in_block_comment = true;
            out.push_str("  ");
            i += 2;
            continue;
        }
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            break;
        }
        // PHP `#` line comment (preserving PHP 8 `#[Attribute]`).
        if lang == Language::Php && bytes[i] == b'#' && bytes.get(i + 1) != Some(&b'[') {
            break;
        }
        let ch = bytes[i];
        if ch == b'"' || (lang == Language::TsJs && ch == b'`') {
            first_code_raw_byte.get_or_insert(i);
            let quote = ch;
            out.push(' ');
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                } else if bytes[i] == quote {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            out.push(' ');
            continue;
        }
        if ch == b'\'' {
            if lang == Language::Rust {
                // Only treat as a Rust char literal (`'a'` or `'\n'`); preserve lifetimes like `'a, 'b`!
                let mut closed = None;
                if bytes.get(i + 1) == Some(&b'\\') {
                    let mut j = i + 3;
                    let limit = len.min(i + 10);
                    while j < limit {
                        if bytes[j] == b'\'' {
                            closed = Some(j);
                            break;
                        }
                        if bytes[j].is_ascii_whitespace() {
                            break;
                        }
                        j += 1;
                    }
                } else if let Some(next_char) = line[i + 1..].chars().next()
                    && next_char != '\''
                {
                    let end_quote_idx = i + 1 + next_char.len_utf8();
                    if bytes.get(end_quote_idx) == Some(&b'\'') {
                        closed = Some(end_quote_idx);
                    }
                }
                if let Some(end_quote) = closed {
                    first_code_raw_byte.get_or_insert(i);
                    out.push(' ');
                    i = end_quote + 1;
                    continue;
                }
            } else {
                first_code_raw_byte.get_or_insert(i);
                out.push(' ');
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' && i + 1 < len {
                        i += 2;
                    } else if bytes[i] == b'\'' {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
                out.push(' ');
                continue;
            }
        }
        // Preserve ASCII bytes directly and copy multi-byte UTF-8 safely by advancing char boundary.
        if ch.is_ascii() {
            if !ch.is_ascii_whitespace() {
                first_code_raw_byte.get_or_insert(i);
            }
            match ch {
                b'(' => *paren_depth += 1,
                b')' => *paren_depth = paren_depth.saturating_sub(1),
                b'[' => *bracket_depth += 1,
                b']' => *bracket_depth = bracket_depth.saturating_sub(1),
                b'{' => events.push(BraceEvent {
                    raw_byte: i,
                    tok: BraceToken::Open,
                    paren_depth: *paren_depth,
                    bracket_depth: *bracket_depth,
                }),
                b'}' => events.push(BraceEvent {
                    raw_byte: i,
                    tok: BraceToken::Close,
                    paren_depth: *paren_depth,
                    bracket_depth: *bracket_depth,
                }),
                b';' => events.push(BraceEvent {
                    raw_byte: i,
                    tok: BraceToken::Semi,
                    paren_depth: *paren_depth,
                    bracket_depth: *bracket_depth,
                }),
                _ => {}
            }
            out.push(ch as char);
            i += 1;
        } else {
            first_code_raw_byte.get_or_insert(i);
            let rest = &line[i..];
            if let Some(c) = rest.chars().next() {
                out.push(c);
                i += c.len_utf8();
            } else {
                break;
            }
        }
    }
    SanitizedLine {
        text: out,
        events,
        first_code_raw_byte,
    }
}

/// Strips comments while preserving string literals verbatim so literal edits inside signatures/bodies are detected.
pub(crate) fn strip_comments_keep_strings(raw: &str, lang: Language) -> String {
    if lang == Language::Python {
        return strip_python_comments_keep_strings(raw);
    }
    let mut out = String::with_capacity(raw.len());
    let mut in_block_comment = false;
    for (line_idx, line) in raw.lines().enumerate() {
        if line_idx > 0 {
            out.push('\n');
        }
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0usize;
        while i < len {
            if in_block_comment {
                if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    in_block_comment = false;
                    out.push(' ');
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                in_block_comment = true;
                out.push(' ');
                i += 2;
                continue;
            }
            if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
                break;
            }
            if lang == Language::Php && bytes[i] == b'#' && bytes.get(i + 1) != Some(&b'[') {
                break;
            }
            let ch = bytes[i];
            if ch == b'"' || (lang == Language::TsJs && ch == b'`') {
                let start = i;
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' && i + 1 < len {
                        i += 2;
                    } else if bytes[i] == ch {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
                out.push_str(&line[start..i]);
                continue;
            }
            if ch == b'\'' {
                if lang == Language::Rust {
                    let mut closed = None;
                    if bytes.get(i + 1) == Some(&b'\\') {
                        let mut j = i + 3;
                        let limit = len.min(i + 10);
                        while j < limit {
                            if bytes[j] == b'\'' {
                                closed = Some(j);
                                break;
                            }
                            if bytes[j].is_ascii_whitespace() {
                                break;
                            }
                            j += 1;
                        }
                    } else if let Some(next_char) = line[i + 1..].chars().next()
                        && next_char != '\''
                    {
                        let end_quote_idx = i + 1 + next_char.len_utf8();
                        if bytes.get(end_quote_idx) == Some(&b'\'') {
                            closed = Some(end_quote_idx);
                        }
                    }
                    if let Some(end_quote) = closed {
                        out.push_str(&line[i..=end_quote]);
                        i = end_quote + 1;
                        continue;
                    }
                } else {
                    let start = i;
                    i += 1;
                    while i < len {
                        if bytes[i] == b'\\' && i + 1 < len {
                            i += 2;
                        } else if bytes[i] == b'\'' {
                            i += 1;
                            break;
                        } else {
                            i += 1;
                        }
                    }
                    out.push_str(&line[start..i]);
                    continue;
                }
            }
            if ch.is_ascii() {
                out.push(ch as char);
                i += 1;
            } else if let Some(c) = line[i..].chars().next() {
                out.push(c);
                i += c.len_utf8();
            } else {
                break;
            }
        }
    }
    out
}

fn strip_python_comments_keep_strings(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut triple: Option<&'static str> = None;
    for (line_idx, line) in raw.lines().enumerate() {
        if line_idx > 0 {
            out.push('\n');
        }
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0usize;
        while i < len {
            if let Some(marker) = triple {
                if line[i..].starts_with(marker) {
                    out.push_str(marker);
                    i += marker.len();
                    triple = None;
                } else if let Some(c) = line[i..].chars().next() {
                    out.push(c);
                    i += c.len_utf8();
                } else {
                    break;
                }
                continue;
            }
            if line[i..].starts_with("\"\"\"") {
                out.push_str("\"\"\"");
                i += 3;
                triple = Some("\"\"\"");
                continue;
            }
            if line[i..].starts_with("'''") {
                out.push_str("'''");
                i += 3;
                triple = Some("'''");
                continue;
            }
            let ch = bytes[i];
            if ch == b'#' {
                break;
            }
            if ch == b'"' || ch == b'\'' {
                let start = i;
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' && i + 1 < len {
                        i += 2;
                    } else if bytes[i] == ch {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
                out.push_str(&line[start..i]);
                continue;
            }
            if ch.is_ascii() {
                out.push(ch as char);
                i += 1;
            } else if let Some(c) = line[i..].chars().next() {
                out.push(c);
                i += c.len_utf8();
            } else {
                break;
            }
        }
    }
    out
}

fn normalize_signature(raw: &str, lang: Language) -> String {
    strip_comments_keep_strings(raw, lang)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_params_signature(raw: &str, lang: Language) -> String {
    let norm = normalize_signature(raw, lang);
    strip_visibility_from_norm(&norm, lang)
}

fn strip_leading_square_attrs(mut s: &str) -> &str {
    loop {
        s = s.trim_start();
        let rest = if let Some(after) = s.strip_prefix("#![") {
            after
        } else if let Some(after) = s.strip_prefix("#[") {
            after
        } else {
            return s;
        };
        let mut depth = 1usize;
        let bytes = rest.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() && depth > 0 {
            let b = bytes[i];
            if b == b'"' || b == b'\'' {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                    } else if bytes[i] == b {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
                continue;
            }
            if b == b'[' {
                depth += 1;
            } else if b == b']' {
                depth -= 1;
            }
            i += 1;
        }
        if depth == 0 {
            s = &rest[i..];
        } else {
            return s;
        }
    }
}

fn skip_leading_rust_pub(s: &str) -> Option<&str> {
    let s = s.trim_start();
    let rest = s.strip_prefix("pub")?;
    if rest.chars().next().is_some_and(is_ident_char) {
        return None;
    }
    let after_ws = rest.trim_start();
    if let Some(in_paren) = after_ws.strip_prefix('(') {
        let mut depth = 1usize;
        for (idx, ch) in in_paren.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(in_paren[idx + 1..].trim_start());
                    }
                }
                _ => {}
            }
        }
        return None;
    }
    Some(after_ws)
}

fn strip_visibility_from_norm(norm: &str, lang: Language) -> String {
    match lang {
        Language::Python => norm.to_owned(),
        Language::Rust => {
            let trimmed = norm.trim();
            let after_attrs = strip_leading_square_attrs(trimmed);
            let prefix_len = trimmed.len() - after_attrs.len();
            if let Some(after_pub) = skip_leading_rust_pub(after_attrs) {
                let prefix = trimmed[..prefix_len].trim_end();
                if prefix.is_empty() {
                    after_pub.to_owned()
                } else if after_pub.is_empty() {
                    prefix.to_owned()
                } else {
                    format!("{prefix} {after_pub}")
                }
            } else {
                trimmed.to_owned()
            }
        }
        Language::Php => {
            let trimmed = norm.trim();
            let after_attrs = strip_leading_square_attrs(trimmed);
            let prefix_len = trimmed.len() - after_attrs.len();
            let mut kept: Vec<&str> = Vec::new();
            let mut cur = after_attrs;
            loop {
                cur = cur.trim_start();
                if let Some((word, rest)) = take_ident(cur) {
                    if matches!(word, "public" | "protected" | "private") {
                        let rest_t = rest.trim_start();
                        if let Some(after_set) = rest_t.strip_prefix("(set)") {
                            cur = after_set;
                        } else {
                            cur = rest;
                        }
                        continue;
                    }
                    if word == "var" {
                        cur = rest;
                        continue;
                    }
                    if matches!(word, "abstract" | "final" | "readonly" | "static") {
                        kept.push(word);
                        cur = rest;
                        continue;
                    }
                }
                break;
            }
            let prefix = trimmed[..prefix_len].trim_end();
            let mut parts: Vec<&str> = Vec::new();
            if !prefix.is_empty() {
                parts.push(prefix);
            }
            parts.extend(kept);
            if !cur.is_empty() {
                parts.push(cur);
            }
            parts.join(" ")
        }
        Language::TsJs => {
            let mut cur = norm.trim();
            let mut kept: Vec<&str> = Vec::new();
            loop {
                cur = cur.trim_start();
                let Some((word, rest)) = take_ident(cur) else {
                    break;
                };
                let rest_t = rest.trim_start();
                let next_ch = rest_t.chars().next();
                let has_member_follower = next_ch.is_some_and(|c| {
                    !matches!(c, '(' | '<' | ':' | '=' | '?' | '!' | ';' | ',' | ')' | '}')
                });
                if !has_member_follower && word != "export" {
                    break;
                }
                if matches!(word, "export" | "public" | "protected" | "private") {
                    cur = rest;
                    continue;
                }
                if matches!(
                    word,
                    "default"
                        | "declare"
                        | "abstract"
                        | "async"
                        | "static"
                        | "readonly"
                        | "override"
                        | "get"
                        | "set"
                ) {
                    kept.push(word);
                    cur = rest;
                    continue;
                }
                break;
            }
            if kept.is_empty() {
                cur.to_owned()
            } else if cur.is_empty() {
                kept.join(" ")
            } else {
                format!("{} {cur}", kept.join(" "))
            }
        }
    }
}

fn split_top_level_assign(raw: &str, lang: Language) -> Option<(&str, &str)> {
    let bytes = raw.as_bytes();
    let len = bytes.len();
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut brace = 0usize;
    let mut angle = 0usize;
    let mut i = 0usize;
    while i < len {
        let ch = bytes[i];
        if ch == b'"' || (lang == Language::TsJs && ch == b'`') {
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                } else if bytes[i] == ch {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if ch == b'\'' {
            if lang == Language::Rust {
                let mut closed = None;
                if bytes.get(i + 1) == Some(&b'\\') {
                    let mut j = i + 3;
                    let limit = len.min(i + 10);
                    while j < limit {
                        if bytes[j] == b'\'' {
                            closed = Some(j);
                            break;
                        }
                        if bytes[j].is_ascii_whitespace() {
                            break;
                        }
                        j += 1;
                    }
                } else if let Some(next_char) = raw[i + 1..].chars().next()
                    && next_char != '\''
                {
                    let end_quote_idx = i + 1 + next_char.len_utf8();
                    if bytes.get(end_quote_idx) == Some(&b'\'') {
                        closed = Some(end_quote_idx);
                    }
                }
                if let Some(end_quote) = closed {
                    i = end_quote + 1;
                    continue;
                }
                i += 1;
                continue;
            }
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                } else if bytes[i] == b'\'' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if i + 1 < len {
            let two = &bytes[i..=i + 1];
            if matches!(two, b"=>" | b"->" | b"==" | b"!=" | b"<=" | b">=") {
                i += 2;
                continue;
            }
        }
        match ch {
            b'(' => paren += 1,
            b')' => paren = paren.saturating_sub(1),
            b'[' => bracket += 1,
            b']' => bracket = bracket.saturating_sub(1),
            b'{' => brace += 1,
            b'}' => brace = brace.saturating_sub(1),
            b'<' => angle += 1,
            b'>' => angle = angle.saturating_sub(1),
            b'=' if paren == 0 && bracket == 0 && brace == 0 && angle == 0 => {
                let prev = i.checked_sub(1).and_then(|p| bytes.get(p).copied());
                let prev_op = prev.is_some_and(|b| {
                    matches!(
                        b,
                        b'!' | b'<'
                            | b'>'
                            | b'+'
                            | b'-'
                            | b'*'
                            | b'/'
                            | b'%'
                            | b'&'
                            | b'|'
                            | b'^'
                            | b'?'
                    )
                });
                if !prev_op {
                    return Some((&raw[..i], &raw[i + 1..]));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn is_upper_snake(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().any(|b| b.is_ascii_uppercase())
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        && !name.bytes().all(|b| b == b'_')
}

fn take_ts_member_ident(s: &str) -> Option<(&str, &str)> {
    let trimmed = s.trim_start();
    if let Some(after_hash) = trimmed.strip_prefix('#')
        && let Some((ident, rest)) = take_ident(after_hash)
        && after_hash.starts_with(ident)
    {
        let total = 1 + ident.len();
        return Some((&trimmed[..total], rest));
    }
    take_ident(trimmed)
}

fn fingerprint_body(raw: &str, lang: Language) -> String {
    let cleaned = strip_comments_keep_strings(raw, lang);
    let mut normalized = String::new();
    for line in cleaned.lines() {
        let compact = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if !compact.is_empty() {
            if !normalized.is_empty() {
                normalized.push('\n');
            }
            normalized.push_str(&compact);
        }
    }
    if normalized.is_empty() {
        return String::new();
    }
    format!(
        "{:032x}",
        xxhash_rust::xxh3::xxh3_128(normalized.as_bytes())
    )
}

pub(crate) fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$'
}

pub(crate) fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

pub(crate) fn take_ident(s: &str) -> Option<(&str, &str)> {
    let trimmed = s.trim_start();
    let mut end = 0usize;
    for (idx, c) in trimmed.char_indices() {
        if idx == 0 {
            if !is_ident_start(c) {
                return None;
            }
        } else if !is_ident_char(c) {
            break;
        }
        end = idx + c.len_utf8();
    }
    if end == 0 {
        None
    } else {
        Some((&trimmed[..end], &trimmed[end..]))
    }
}

pub(crate) fn strip_leading_words<'a>(mut s: &'a str, modifiers: &[&str]) -> &'a str {
    loop {
        s = s.trim_start();
        let Some((word, rest)) = take_ident(s) else {
            return s;
        };
        if modifiers.contains(&word) {
            s = rest;
        } else {
            return s;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum BraceBlock {
    Mod,
    ClassLike {
        name: String,
        vis: Option<String>,
        is_trait: bool,
        is_trait_impl: bool,
    },
    Struct(String),
    Fn,
    Other,
}

fn pop_closed_blocks(stack: &mut Vec<(BraceBlock, usize)>, depth: usize) {
    while stack
        .last()
        .is_some_and(|(_, open_depth)| depth < *open_depth)
    {
        stack.pop();
    }
}

fn active_class_parent(stack: &[(BraceBlock, usize)], current_depth: usize) -> Option<String> {
    match stack.last() {
        Some((BraceBlock::ClassLike { name, .. }, open_depth)) if *open_depth == current_depth => {
            Some(name.clone())
        }
        _ => None,
    }
}

fn active_class_context(
    stack: &[(BraceBlock, usize)],
    current_depth: usize,
) -> Option<(String, Option<String>, bool, bool)> {
    match stack.last() {
        Some((
            BraceBlock::ClassLike {
                name,
                vis,
                is_trait,
                is_trait_impl,
            },
            open_depth,
        )) if *open_depth == current_depth => {
            Some((name.clone(), vis.clone(), *is_trait, *is_trait_impl))
        }
        _ => None,
    }
}

fn active_struct_parent(stack: &[(BraceBlock, usize)], current_depth: usize) -> Option<&str> {
    match stack.last() {
        Some((BraceBlock::Struct(name), open_depth)) if *open_depth == current_depth => {
            Some(name.as_str())
        }
        _ => None,
    }
}

fn inside_fn_scope(stack: &[(BraceBlock, usize)]) -> bool {
    stack
        .iter()
        .any(|(kind, _)| matches!(kind, BraceBlock::Fn | BraceBlock::Other))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeclMode {
    Braced,
    ArrowFn,
    ValueStmt { asi: bool },
}

struct PendingDecl {
    decl_idx: usize,
    decl_depth: usize,
    start_line: usize,
    start_col: usize,
    is_class_like: bool,
    mode: DeclMode,
    struct_owner: Option<(String, String)>,
    php_ctor_owner: Option<(String, String)>,
}

struct ActiveSpan {
    decl_idx: usize,
    open_depth: usize,
    is_class_like: bool,
    decl_start: (usize, usize),
    body_start: (usize, usize),
    excluded_ranges: Vec<((usize, usize), (usize, usize))>,
    struct_owner: Option<(String, String)>,
}

struct BraceSpanTracker {
    decls: Vec<ExtractedDecl>,
    depth: usize,
    stack: Vec<(BraceBlock, usize)>,
    pending_block: Option<BraceBlock>,
    pending_decl: Option<PendingDecl>,
    active_spans: Vec<ActiveSpan>,
}

impl BraceSpanTracker {
    fn new() -> Self {
        Self {
            decls: Vec::new(),
            depth: 0,
            stack: Vec::new(),
            pending_block: None,
            pending_decl: None,
            active_spans: Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn start_decl_mode(
        &mut self,
        entity: CreatedEntity,
        start_line: usize,
        start_col: usize,
        is_class_like: bool,
        mode: DeclMode,
        struct_owner: Option<(String, String)>,
        php_ctor_owner: Option<(String, String)>,
    ) {
        let line_no = (start_line + 1) as u32;
        let decl_idx = self.decls.len();
        let visibility = entity.visibility.clone();
        let modifiers = entity.modifiers.clone();
        self.decls.push(ExtractedDecl {
            entity,
            signature: String::new(),
            params_signature: String::new(),
            body_fingerprint: String::new(),
            visibility,
            modifiers,
            start_line: line_no,
            end_line: line_no,
        });
        self.pending_decl = Some(PendingDecl {
            decl_idx,
            decl_depth: self.depth,
            start_line,
            start_col,
            is_class_like,
            mode,
            struct_owner,
            php_ctor_owner,
        });
    }

    fn flush_pending_asi(&mut self, lines: &[&str], up_to_line: usize, lang: Language) {
        if let Some(pending) = self.pending_decl.as_ref()
            && matches!(
                pending.mode,
                DeclMode::ArrowFn | DeclMode::ValueStmt { asi: true }
            )
            && pending.decl_depth == self.depth
        {
            let pending = self.pending_decl.take().expect("checked");
            self.pending_block = None;
            let end_line = up_to_line.min(lines.len().saturating_sub(1));
            let end_col = lines.get(end_line).map_or(0, |l| l.len());
            let raw_stmt = slice_raw_excluding(
                lines,
                (pending.start_line, pending.start_col),
                (end_line, end_col),
                &[],
            );
            finalize_non_braced_decl(
                &mut self.decls[pending.decl_idx],
                &raw_stmt,
                pending.mode,
                (end_line + 1) as u32,
                lang,
            );
            if let Some(parent) = self.active_spans.last_mut()
                && parent.is_class_like
                && parent.open_depth == pending.decl_depth
            {
                parent
                    .excluded_ranges
                    .push(((pending.start_line, pending.start_col), (end_line, end_col)));
            }
        }
    }

    fn finish_closed_span(
        &mut self,
        lines: &[&str],
        mut span: ActiveSpan,
        end_pos: LineColPos,
        lang: Language,
    ) {
        self.decls[span.decl_idx].end_line = (end_pos.0 + 1) as u32;
        let mut extra_decls = Vec::new();
        if lang == Language::Rust
            && let Some((struct_name, rel_path)) = span.struct_owner.take()
        {
            let (fields, field_ranges) = extract_rust_struct_fields(
                lines,
                span.body_start,
                end_pos,
                &struct_name,
                &rel_path,
            );
            span.excluded_ranges.extend(field_ranges);
            extra_decls = fields;
        }
        let body = slice_raw_excluding(lines, span.body_start, end_pos, &span.excluded_ranges);
        self.decls[span.decl_idx].body_fingerprint = fingerprint_body(&body, lang);
        if let Some(parent) = self.active_spans.last_mut()
            && parent.is_class_like
        {
            parent
                .excluded_ranges
                .push((span.decl_start, (end_pos.0, end_pos.1 + 1)));
        }
        self.decls.extend(extra_decls);
    }

    fn apply_events(
        &mut self,
        lines: &[&str],
        line_idx: usize,
        events: &[BraceEvent],
        lang: Language,
    ) {
        for ev in events {
            match ev.tok {
                BraceToken::Open => {
                    self.depth += 1;
                    if ev.paren_depth == 0 && ev.bracket_depth == 0 {
                        let is_value_stmt = self
                            .pending_decl
                            .as_ref()
                            .is_some_and(|p| matches!(p.mode, DeclMode::ValueStmt { .. }));
                        if !is_value_stmt {
                            if let Some(block) = self.pending_block.take() {
                                self.stack.push((block, self.depth));
                            }
                            if let Some(pending) = self.pending_decl.take() {
                                let header = slice_raw_excluding(
                                    lines,
                                    (pending.start_line, pending.start_col),
                                    (line_idx, ev.raw_byte),
                                    &[],
                                );
                                self.decls[pending.decl_idx].signature =
                                    normalize_signature(&header, lang);
                                self.decls[pending.decl_idx].params_signature =
                                    normalize_params_signature(&header, lang);
                                let promoted_decls =
                                    if let Some((owner, rel_path)) = pending.php_ctor_owner {
                                        match lang {
                                            Language::Php => extract_php_promoted_properties(
                                                lines,
                                                (pending.start_line, pending.start_col),
                                                (line_idx, ev.raw_byte),
                                                &owner,
                                                &rel_path,
                                            ),
                                            Language::TsJs => extract_ts_ctor_properties(
                                                lines,
                                                (pending.start_line, pending.start_col),
                                                (line_idx, ev.raw_byte),
                                                &owner,
                                                &rel_path,
                                            ),
                                            _ => Vec::new(),
                                        }
                                    } else {
                                        Vec::new()
                                    };
                                self.active_spans.push(ActiveSpan {
                                    decl_idx: pending.decl_idx,
                                    open_depth: self.depth,
                                    is_class_like: pending.is_class_like,
                                    decl_start: (pending.start_line, pending.start_col),
                                    body_start: (line_idx, ev.raw_byte + 1),
                                    excluded_ranges: Vec::new(),
                                    struct_owner: pending.struct_owner,
                                });
                                self.decls.extend(promoted_decls);
                            }
                        }
                    }
                }
                BraceToken::Close => {
                    self.depth = self.depth.saturating_sub(1);
                    if self
                        .pending_decl
                        .as_ref()
                        .is_some_and(|p| p.decl_depth > self.depth)
                    {
                        self.pending_block = None;
                        let pending = self.pending_decl.take().expect("checked");
                        let raw_stmt = slice_raw_excluding(
                            lines,
                            (pending.start_line, pending.start_col),
                            (line_idx, ev.raw_byte),
                            &[],
                        );
                        finalize_non_braced_decl(
                            &mut self.decls[pending.decl_idx],
                            &raw_stmt,
                            pending.mode,
                            (line_idx + 1) as u32,
                            lang,
                        );
                        if let Some(parent) = self.active_spans.last_mut()
                            && parent.is_class_like
                        {
                            parent.excluded_ranges.push((
                                (pending.start_line, pending.start_col),
                                (line_idx, ev.raw_byte),
                            ));
                        }
                    }
                    pop_closed_blocks(&mut self.stack, self.depth);
                    while self
                        .active_spans
                        .last()
                        .is_some_and(|span| self.depth < span.open_depth)
                    {
                        let span = self.active_spans.pop().expect("checked");
                        self.finish_closed_span(lines, span, (line_idx, ev.raw_byte), lang);
                    }
                }
                BraceToken::Semi => {
                    if ev.paren_depth == 0
                        && ev.bracket_depth == 0
                        && self
                            .pending_decl
                            .as_ref()
                            .is_some_and(|p| p.decl_depth == self.depth)
                    {
                        self.pending_block = None;
                        let pending = self.pending_decl.take().expect("checked");
                        let stmt = slice_raw_excluding(
                            lines,
                            (pending.start_line, pending.start_col),
                            (line_idx, ev.raw_byte),
                            &[],
                        );
                        finalize_non_braced_decl(
                            &mut self.decls[pending.decl_idx],
                            &stmt,
                            pending.mode,
                            (line_idx + 1) as u32,
                            lang,
                        );
                        if let Some(parent) = self.active_spans.last_mut()
                            && parent.is_class_like
                        {
                            parent.excluded_ranges.push((
                                (pending.start_line, pending.start_col),
                                (line_idx, ev.raw_byte + 1),
                            ));
                        }
                    } else if ev.paren_depth == 0
                        && ev.bracket_depth == 0
                        && self.pending_block.is_some()
                    {
                        let has_open_brace_before = events
                            .iter()
                            .take_while(|other| other.raw_byte < ev.raw_byte)
                            .any(|other| {
                                other.tok == BraceToken::Open
                                    && other.paren_depth == 0
                                    && other.bracket_depth == 0
                            });
                        if !has_open_brace_before {
                            self.pending_block = None;
                        }
                    }
                }
            }
        }
    }

    fn finish_eof(mut self, lines: &[&str], lang: Language) -> Vec<ExtractedDecl> {
        let last_line = lines.len().saturating_sub(1);
        let last_col = lines.get(last_line).map_or(0, |l| l.len());
        if let Some(pending) = self.pending_decl.take() {
            let raw = slice_raw_excluding(
                lines,
                (pending.start_line, pending.start_col),
                (last_line, last_col),
                &[],
            );
            finalize_non_braced_decl(
                &mut self.decls[pending.decl_idx],
                &raw,
                pending.mode,
                (last_line + 1) as u32,
                lang,
            );
            if let Some(parent) = self.active_spans.last_mut()
                && parent.is_class_like
            {
                parent.excluded_ranges.push((
                    (pending.start_line, pending.start_col),
                    (last_line, last_col),
                ));
            }
        }
        while let Some(mut span) = self.active_spans.pop() {
            self.decls[span.decl_idx].end_line = (last_line + 1) as u32;
            let mut extra_decls = Vec::new();
            if lang == Language::Rust
                && let Some((struct_name, rel_path)) = span.struct_owner.take()
            {
                let (fields, field_ranges) = extract_rust_struct_fields(
                    lines,
                    span.body_start,
                    (last_line, last_col),
                    &struct_name,
                    &rel_path,
                );
                span.excluded_ranges.extend(field_ranges);
                extra_decls = fields;
            }
            let body = slice_raw_excluding(
                lines,
                span.body_start,
                (last_line, last_col),
                &span.excluded_ranges,
            );
            self.decls[span.decl_idx].body_fingerprint = fingerprint_body(&body, lang);
            self.decls.extend(extra_decls);
        }
        self.decls
    }
}

fn finalize_non_braced_decl(
    decl: &mut ExtractedDecl,
    raw_stmt: &str,
    mode: DeclMode,
    end_line: u32,
    lang: Language,
) {
    decl.end_line = end_line;
    match mode {
        DeclMode::ArrowFn => {
            if let Some((lhs, rhs)) = split_top_level_arrow(raw_stmt) {
                decl.signature = normalize_signature(lhs, lang);
                decl.params_signature = normalize_params_signature(lhs, lang);
                decl.body_fingerprint = fingerprint_body(rhs, lang);
            } else {
                decl.signature = normalize_signature(raw_stmt, lang);
                decl.params_signature = normalize_params_signature(raw_stmt, lang);
                decl.body_fingerprint = String::new();
            }
        }
        DeclMode::ValueStmt { .. } => {
            if let Some((lhs, rhs)) = split_top_level_assign(raw_stmt, lang) {
                decl.signature = normalize_signature(lhs, lang);
                decl.params_signature = normalize_params_signature(lhs, lang);
                decl.body_fingerprint = fingerprint_body(rhs, lang);
            } else {
                decl.signature = normalize_signature(raw_stmt, lang);
                decl.params_signature = normalize_params_signature(raw_stmt, lang);
                decl.body_fingerprint = String::new();
            }
        }
        DeclMode::Braced => {
            decl.signature = normalize_signature(raw_stmt, lang);
            decl.params_signature = normalize_params_signature(raw_stmt, lang);
            decl.body_fingerprint = String::new();
        }
    }
}

fn split_top_level_arrow(raw: &str) -> Option<(&str, &str)> {
    let bytes = raw.as_bytes();
    let len = bytes.len();
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut i = 0usize;
    while i + 1 < len {
        let ch = bytes[i];
        if ch == b'"' || ch == b'\'' || ch == b'`' {
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                } else if bytes[i] == ch {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        match ch {
            b'(' => paren += 1,
            b')' => paren = paren.saturating_sub(1),
            b'[' => bracket += 1,
            b']' => bracket = bracket.saturating_sub(1),
            b'=' if bytes[i + 1] == b'>' && paren == 0 && bracket == 0 => {
                return Some((&raw[..i], &raw[i + 2..]));
            }
            _ => {}
        }
        i += 1;
    }
    None
}

type LineColPos = (usize, usize);
type LineColSpan = (LineColPos, LineColPos);

fn slice_raw_excluding(
    lines: &[&str],
    start: LineColPos,
    end: LineColPos,
    excluded: &[LineColSpan],
) -> String {
    if lines.is_empty() || start >= end || start.0 >= lines.len() {
        return String::new();
    }
    let last_idx = end.0.min(lines.len().saturating_sub(1));
    let mut out = String::new();
    for (li, line) in lines.iter().enumerate().take(last_idx + 1).skip(start.0) {
        if li > start.0 {
            out.push('\n');
        }
        let len = line.len();
        let mut lo = if li == start.0 { start.1.min(len) } else { 0 };
        let mut hi = if li == end.0 { end.1.min(len) } else { len };
        while lo < len && !line.is_char_boundary(lo) {
            lo += 1;
        }
        while hi > 0 && !line.is_char_boundary(hi) {
            hi -= 1;
        }
        if lo >= hi {
            continue;
        }
        if excluded.is_empty() {
            out.push_str(&line[lo..hi]);
            continue;
        }
        let mut intervals: Vec<(usize, usize)> = Vec::new();
        for &(ex_s, ex_e) in excluded {
            if li < ex_s.0 || li > ex_e.0 {
                continue;
            }
            let ex_lo = if li == ex_s.0 {
                ex_s.1.max(lo).min(hi)
            } else {
                lo
            };
            let ex_hi = if li == ex_e.0 {
                ex_e.1.max(lo).min(hi)
            } else {
                hi
            };
            if ex_lo < ex_hi {
                intervals.push((ex_lo, ex_hi));
            }
        }
        if intervals.is_empty() {
            out.push_str(&line[lo..hi]);
            continue;
        }
        intervals.sort_unstable();
        let mut cur = lo;
        for (ex_lo, ex_hi) in intervals {
            if ex_lo > cur {
                out.push_str(&line[cur..ex_lo]);
            }
            cur = cur.max(ex_hi);
        }
        if cur < hi {
            out.push_str(&line[cur..hi]);
        }
    }
    out
}

fn find_php_decl_col(raw_line: &str, first_code_raw_byte: Option<usize>) -> usize {
    let mut col = first_code_raw_byte.unwrap_or(0).min(raw_line.len());
    if let Some(rest) = raw_line.get(col..) {
        if let Some(after) = rest.strip_prefix("<?php") {
            let ws = after.len() - after.trim_start().len();
            col += 5 + ws;
        } else if let Some(after) = rest.strip_prefix("<?") {
            let ws = after.len() - after.trim_start().len();
            col += 2 + ws;
        }
    }
    col.min(raw_line.len())
}

fn push_unique_modifier(mods: &mut Vec<String>, m: &str) {
    if !mods.iter().any(|existing| existing == m) {
        mods.push(m.to_owned());
    }
}

fn parse_php_qualifiers<'a>(
    s: &'a str,
    default_vis: Option<&str>,
) -> (Option<String>, Vec<String>, bool, &'a str) {
    let mut cur = strip_leading_square_attrs(s);
    let mut read_vis: Option<&str> = None;
    let mut set_vis: Option<&str> = None;
    let mut has_vis = false;
    let mut modifiers = Vec::new();
    loop {
        cur = strip_leading_square_attrs(cur);
        let Some((word, rest)) = take_ident(cur) else {
            break;
        };
        if matches!(word, "public" | "protected" | "private") {
            let rest_t = rest.trim_start();
            if let Some(after_set) = rest_t.strip_prefix("(set)") {
                set_vis = Some(word);
                has_vis = true;
                cur = after_set;
                continue;
            }
            read_vis = Some(word);
            has_vis = true;
            cur = rest;
            continue;
        }
        if word == "var" {
            read_vis = Some("public");
            has_vis = true;
            cur = rest;
            continue;
        }
        if matches!(word, "abstract" | "final" | "readonly" | "static") {
            push_unique_modifier(&mut modifiers, word);
            cur = rest;
            continue;
        }
        break;
    }
    let visibility = match (read_vis, set_vis) {
        (_, Some("private")) => match read_vis {
            Some("protected") => Some("protected private(set)".to_owned()),
            Some("private") => Some("private".to_owned()),
            _ => Some("private(set)".to_owned()),
        },
        (_, Some("protected")) => match read_vis {
            Some("protected") => Some("protected".to_owned()),
            _ => Some("protected(set)".to_owned()),
        },
        (_, Some("public")) => Some(read_vis.unwrap_or("public").to_owned()),
        (Some(v), None) => Some(v.to_owned()),
        _ => default_vis.map(str::to_owned),
    };
    (visibility, modifiers, has_vis, cur)
}

fn last_ident_token(s: &str) -> Option<&str> {
    let mut last: Option<&str> = None;
    let mut cur = s;
    while !cur.is_empty() {
        if let Some((word, rest)) = take_ident(cur) {
            last = Some(word);
            cur = rest;
            continue;
        }
        let mut chars = cur.chars();
        chars.next();
        cur = chars.as_str();
    }
    last
}

fn find_php_dollar_ident(s: &str) -> Option<&str> {
    let mut cur = s;
    while !cur.is_empty() {
        if let Some((word, rest)) = take_ident(cur) {
            if word.starts_with('$') && word.len() > 1 {
                return Some(word);
            }
            cur = rest;
            continue;
        }
        let mut chars = cur.chars();
        chars.next();
        cur = chars.as_str();
    }
    None
}

fn extract_php_promoted_properties(
    lines: &[&str],
    header_start: LineColPos,
    header_end: LineColPos,
    owner: &str,
    rel_path: &str,
) -> Vec<ExtractedDecl> {
    let raw_header = slice_raw_excluding(lines, header_start, header_end, &[]);
    let bytes = raw_header.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut line_off = 0usize;
    let mut open_paren_at: Option<(usize, usize)> = None;
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut brace = 0usize;
    let mut in_block_comment = false;

    let mut params: Vec<(usize, String)> = Vec::new();
    let mut cur_start: Option<(usize, usize)> = None;

    while i < len {
        let ch = bytes[i];
        if in_block_comment {
            if ch == b'\n' {
                line_off += 1;
            }
            if ch == b'*' && i + 1 < len && bytes[i + 1] == b'/' {
                in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if ch == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            in_block_comment = true;
            i += 2;
            continue;
        }
        if (ch == b'/' && i + 1 < len && bytes[i + 1] == b'/')
            || (ch == b'#' && bytes.get(i + 1) != Some(&b'['))
        {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if ch == b'"' || ch == b'\'' {
            i += 1;
            while i < len {
                if bytes[i] == b'\n' {
                    line_off += 1;
                }
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                } else if bytes[i] == ch {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if ch == b'\n' {
            line_off += 1;
            i += 1;
            continue;
        }
        if open_paren_at.is_none() {
            if ch == b'(' {
                paren = 1;
                open_paren_at = Some((i + 1, line_off));
                cur_start = Some((i + 1, line_off));
            }
            i += 1;
            continue;
        }
        match ch {
            b'(' => paren += 1,
            b')' => {
                if paren == 1 && bracket == 0 && brace == 0 {
                    if let Some((start_idx, start_lo)) = cur_start {
                        params.push((start_lo, raw_header[start_idx..i].to_owned()));
                    }
                    break;
                }
                paren = paren.saturating_sub(1);
            }
            b'[' => bracket += 1,
            b']' => bracket = bracket.saturating_sub(1),
            b'{' => brace += 1,
            b'}' => brace = brace.saturating_sub(1),
            b',' if paren == 1 && bracket == 0 && brace == 0 => {
                if let Some((start_idx, start_lo)) = cur_start {
                    params.push((start_lo, raw_header[start_idx..i].to_owned()));
                }
                cur_start = Some((i + 1, line_off));
            }
            _ => {}
        }
        i += 1;
    }

    let mut out = Vec::new();
    for (rel_line, raw_param) in params {
        let cleaned = strip_comments_keep_strings(&raw_param, Language::Php);
        let trimmed = cleaned.trim_start();
        let lead_ws_nl = cleaned[..cleaned.len() - trimmed.len()]
            .bytes()
            .filter(|b| *b == b'\n')
            .count();
        let after_attrs = strip_leading_square_attrs(trimmed).trim_start();
        let attr_nl = trimmed[..trimmed.len() - after_attrs.len()]
            .bytes()
            .filter(|b| *b == b'\n')
            .count();
        if after_attrs.is_empty() {
            continue;
        }
        let (vis, modifiers, has_vis, stripped) = parse_php_qualifiers(after_attrs, None);
        if !has_vis {
            continue;
        }
        let (lhs, rhs) = match split_top_level_assign(stripped, Language::Php) {
            Some((l, r)) => (
                &after_attrs[..after_attrs.len() - stripped.len() + l.len()],
                Some(r),
            ),
            None => (after_attrs.trim(), None),
        };
        let lhs_stripped = split_top_level_assign(stripped, Language::Php)
            .map(|(l, _)| l)
            .unwrap_or(stripped);
        let Some(prop_name) = find_php_dollar_ident(lhs_stripped) else {
            continue;
        };
        let line_no = (header_start.0 + rel_line + lead_ws_nl + attr_nl + 1) as u32;
        let end_nl = after_attrs
            .trim_end()
            .bytes()
            .filter(|b| *b == b'\n')
            .count() as u32;
        out.push(ExtractedDecl {
            entity: CreatedEntity::new(
                "property",
                prop_name,
                Some(owner.to_owned()),
                rel_path,
                Some(line_no),
            )
            .with_access(vis.clone(), modifiers.clone()),
            signature: normalize_signature(lhs, Language::Php),
            params_signature: normalize_params_signature(lhs, Language::Php),
            body_fingerprint: rhs
                .map(|r| fingerprint_body(r, Language::Php))
                .unwrap_or_default(),
            visibility: vis,
            modifiers,
            start_line: line_no,
            end_line: line_no + end_nl,
        });
    }
    out
}

fn extract_php_decls(rel_path: &str, source: &str) -> Vec<ExtractedDecl> {
    let lines: Vec<&str> = source.lines().collect();
    let mut tracker = BraceSpanTracker::new();
    let mut in_block_comment = false;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;

    for (idx, raw_line) in lines.iter().enumerate() {
        let line_no = (idx + 1) as u32;
        let pre_paren = paren_depth;
        let pre_bracket = bracket_depth;
        let san = sanitize_c_like_with_events(
            raw_line,
            &mut in_block_comment,
            &mut paren_depth,
            &mut bracket_depth,
            Language::Php,
        );
        let trimmed = san.text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let clean_stmt = trimmed
            .trim_start_matches("<?php")
            .trim_start_matches("<?")
            .trim();

        if !inside_fn_scope(&tracker.stack)
            && pre_paren == 0
            && pre_bracket == 0
            && tracker.pending_decl.is_none()
        {
            let class_owner = active_class_parent(&tracker.stack, tracker.depth);
            let default_vis = Some("public");
            let (vis, modifiers, has_vis, stripped) = parse_php_qualifiers(clean_stmt, default_vis);
            if !stripped.is_empty() {
                let start_col = find_php_decl_col(raw_line, san.first_code_raw_byte);
                if let Some((kw, after_kw)) = take_ident(stripped) {
                    match kw {
                        "class" | "interface" | "trait" | "enum" => {
                            if !trimmed.starts_with("new ")
                                && let Some((name, _)) = take_ident(after_kw)
                            {
                                let class_vis = Some("public".to_owned());
                                tracker.start_decl_mode(
                                    CreatedEntity::new(
                                        kw,
                                        name,
                                        Some(rel_path.to_owned()),
                                        rel_path,
                                        Some(line_no),
                                    )
                                    .with_access(class_vis.clone(), modifiers),
                                    idx,
                                    start_col,
                                    true,
                                    DeclMode::Braced,
                                    None,
                                    None,
                                );
                                tracker.pending_block = Some(BraceBlock::ClassLike {
                                    name: name.to_owned(),
                                    vis: class_vis,
                                    is_trait: false,
                                    is_trait_impl: false,
                                });
                            }
                        }
                        "function" => {
                            let after_ref = after_kw.trim_start().trim_start_matches('&');
                            if let Some((name, rest)) = take_ident(after_ref)
                                && rest.trim_start().starts_with('(')
                            {
                                let ctor_owner = class_owner
                                    .as_ref()
                                    .filter(|_| name == "__construct")
                                    .map(|owner| (owner.clone(), rel_path.to_owned()));
                                let entity = if let Some(owner) = class_owner {
                                    CreatedEntity::new(
                                        "method",
                                        name,
                                        Some(owner),
                                        rel_path,
                                        Some(line_no),
                                    )
                                } else {
                                    CreatedEntity::new(
                                        "function",
                                        name,
                                        Some(rel_path.to_owned()),
                                        rel_path,
                                        Some(line_no),
                                    )
                                }
                                .with_access(vis, modifiers);
                                tracker.start_decl_mode(
                                    entity,
                                    idx,
                                    start_col,
                                    false,
                                    DeclMode::Braced,
                                    None,
                                    ctor_owner,
                                );
                                tracker.pending_block = Some(BraceBlock::Fn);
                            }
                        }
                        "const" => {
                            if class_owner.is_some() || tracker.stack.is_empty() {
                                let lhs = split_top_level_assign(after_kw, Language::Php)
                                    .map(|(l, _)| l)
                                    .unwrap_or_else(|| after_kw.trim_end_matches(';'));
                                if let Some(const_name) = last_ident_token(lhs)
                                    && !const_name.starts_with('$')
                                {
                                    let parent = class_owner.or_else(|| Some(rel_path.to_owned()));
                                    let entity = CreatedEntity::new(
                                        "const",
                                        const_name,
                                        parent,
                                        rel_path,
                                        Some(line_no),
                                    )
                                    .with_access(vis, modifiers);
                                    tracker.start_decl_mode(
                                        entity,
                                        idx,
                                        start_col,
                                        false,
                                        DeclMode::ValueStmt { asi: false },
                                        None,
                                        None,
                                    );
                                }
                            }
                        }
                        _ if class_owner.is_some()
                            && !matches!(kw, "use" | "case")
                            && (has_vis || !modifiers.is_empty()) =>
                        {
                            let lhs = split_top_level_assign(stripped, Language::Php)
                                .map(|(l, _)| l)
                                .unwrap_or_else(|| stripped.trim_end_matches(';'));
                            if let Some(prop_name) = find_php_dollar_ident(lhs) {
                                let entity = CreatedEntity::new(
                                    "property",
                                    prop_name,
                                    class_owner,
                                    rel_path,
                                    Some(line_no),
                                )
                                .with_access(vis, modifiers);
                                tracker.start_decl_mode(
                                    entity,
                                    idx,
                                    start_col,
                                    false,
                                    DeclMode::ValueStmt { asi: false },
                                    None,
                                    None,
                                );
                            }
                        }
                        _ => {}
                    }
                } else if let Some(owner) = class_owner
                    && (has_vis || !modifiers.is_empty())
                {
                    let lhs = split_top_level_assign(stripped, Language::Php)
                        .map(|(l, _)| l)
                        .unwrap_or_else(|| stripped.trim_end_matches(';'));
                    if let Some(prop_name) = find_php_dollar_ident(lhs) {
                        let entity = CreatedEntity::new(
                            "property",
                            prop_name,
                            Some(owner),
                            rel_path,
                            Some(line_no),
                        )
                        .with_access(vis, modifiers);
                        tracker.start_decl_mode(
                            entity,
                            idx,
                            start_col,
                            false,
                            DeclMode::ValueStmt { asi: false },
                            None,
                            None,
                        );
                    }
                }
            }
        }
        tracker.apply_events(&lines, idx, &san.events, Language::Php);
    }
    tracker.finish_eof(&lines, Language::Php)
}

fn extract_rust_struct_fields(
    lines: &[&str],
    body_start: LineColPos,
    body_end: LineColPos,
    struct_name: &str,
    rel_path: &str,
) -> (Vec<ExtractedDecl>, Vec<LineColSpan>) {
    if lines.is_empty() || body_start >= body_end || body_start.0 >= lines.len() {
        return (Vec::new(), Vec::new());
    }
    let last_li = body_end.0.min(lines.len().saturating_sub(1));
    let mut chunks: Vec<LineColSpan> = Vec::new();
    let mut chunk_start = body_start;
    let mut in_block_comment = false;
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut brace = 0usize;
    let mut angle = 0usize;

    for (li, line) in lines
        .iter()
        .enumerate()
        .take(last_li + 1)
        .skip(body_start.0)
    {
        let len = line.len();
        let lo = if li == body_start.0 {
            body_start.1.min(len)
        } else {
            0
        };
        let hi = if li == body_end.0 {
            body_end.1.min(len)
        } else {
            len
        };
        let bytes = line.as_bytes();
        let mut i = lo;
        while i < hi {
            if in_block_comment {
                if i + 1 < hi && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    in_block_comment = false;
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if i + 1 < hi && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                in_block_comment = true;
                i += 2;
                continue;
            }
            if i + 1 < hi && bytes[i] == b'/' && bytes[i + 1] == b'/' {
                break;
            }
            let ch = bytes[i];
            if ch == b'"' {
                i += 1;
                while i < hi {
                    if bytes[i] == b'\\' && i + 1 < hi {
                        i += 2;
                    } else if bytes[i] == b'"' {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
                continue;
            }
            if ch == b'\'' {
                let mut closed = None;
                if bytes.get(i + 1) == Some(&b'\\') {
                    let mut j = i + 3;
                    let limit = hi.min(i + 10);
                    while j < limit {
                        if bytes[j] == b'\'' {
                            closed = Some(j);
                            break;
                        }
                        if bytes[j].is_ascii_whitespace() {
                            break;
                        }
                        j += 1;
                    }
                } else if let Some(next_char) = line[i + 1..hi].chars().next()
                    && next_char != '\''
                {
                    let end_quote_idx = i + 1 + next_char.len_utf8();
                    if end_quote_idx < hi && bytes[end_quote_idx] == b'\'' {
                        closed = Some(end_quote_idx);
                    }
                }
                if let Some(end_quote) = closed {
                    i = end_quote + 1;
                } else {
                    i += 1;
                }
                continue;
            }
            if i + 1 < hi && matches!(&bytes[i..=i + 1], b"=>" | b"->") {
                i += 2;
                continue;
            }
            match ch {
                b'(' => paren += 1,
                b')' => paren = paren.saturating_sub(1),
                b'[' => bracket += 1,
                b']' => bracket = bracket.saturating_sub(1),
                b'{' => brace += 1,
                b'}' => brace = brace.saturating_sub(1),
                b'<' if paren == 0 && bracket == 0 && brace == 0 => angle += 1,
                b'>' if paren == 0 && bracket == 0 && brace == 0 => angle = angle.saturating_sub(1),
                b',' if paren == 0 && bracket == 0 && brace == 0 && angle == 0 => {
                    chunks.push((chunk_start, (li, i + 1)));
                    chunk_start = (li, i + 1);
                }
                _ => {}
            }
            i += 1;
        }
    }
    if chunk_start < body_end {
        chunks.push((chunk_start, body_end));
    }

    let mut decls = Vec::new();
    let mut excluded_ranges = Vec::new();
    for (c_start, c_end) in chunks {
        let raw_chunk = slice_raw_excluding(lines, c_start, c_end, &[]);
        let cleaned = strip_comments_keep_strings(&raw_chunk, Language::Rust);
        let cleaned_no_comma = cleaned.trim_end().trim_end_matches(',').trim_end();
        let trimmed = cleaned_no_comma.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let lead_nl = cleaned_no_comma[..cleaned_no_comma.len() - trimmed.len()]
            .bytes()
            .filter(|b| *b == b'\n')
            .count();
        let after_attrs = strip_leading_square_attrs(trimmed).trim_start();
        if after_attrs.is_empty() {
            continue;
        }
        let attr_nl = trimmed[..trimmed.len() - after_attrs.len()]
            .bytes()
            .filter(|b| *b == b'\n')
            .count();
        let (parsed_vis, modifiers, rest) = parse_rust_qualifiers(after_attrs, true);
        let Some((field_name, after_name)) = take_ident(rest) else {
            continue;
        };
        let after_name_t = after_name.trim_start();
        if !after_name_t.starts_with(':') || after_name_t.starts_with("::") {
            continue;
        }
        let field_line = (c_start.0 + lead_nl + attr_nl + 1) as u32;
        let end_nl = after_attrs.bytes().filter(|b| *b == b'\n').count() as u32;
        let vis = parsed_vis.unwrap_or_else(|| "private".to_owned());
        decls.push(ExtractedDecl {
            entity: CreatedEntity::new(
                "field",
                field_name,
                Some(struct_name.to_owned()),
                rel_path,
                Some(field_line),
            )
            .with_access(Some(vis.clone()), modifiers.clone()),
            signature: normalize_signature(after_attrs, Language::Rust),
            params_signature: normalize_params_signature(after_attrs, Language::Rust),
            body_fingerprint: String::new(),
            visibility: Some(vis),
            modifiers,
            start_line: field_line,
            end_line: field_line + end_nl,
        });
        excluded_ranges.push((c_start, c_end));
    }
    (decls, excluded_ranges)
}

fn extract_rust_decls(rel_path: &str, source: &str) -> Vec<ExtractedDecl> {
    let lines: Vec<&str> = source.lines().collect();
    let mut tracker = BraceSpanTracker::new();
    let mut in_block_comment = false;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;

    for (idx, raw_line) in lines.iter().enumerate() {
        let line_no = (idx + 1) as u32;
        let pre_paren = paren_depth;
        let pre_bracket = bracket_depth;
        let san = sanitize_c_like_with_events(
            raw_line,
            &mut in_block_comment,
            &mut paren_depth,
            &mut bracket_depth,
            Language::Rust,
        );
        let trimmed = san.text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let after_attrs = strip_leading_square_attrs(trimmed);
        if after_attrs.is_empty() || after_attrs.starts_with('#') {
            tracker.apply_events(&lines, idx, &san.events, Language::Rust);
            continue;
        }

        if !inside_fn_scope(&tracker.stack)
            && active_struct_parent(&tracker.stack, tracker.depth).is_none()
            && pre_paren == 0
            && pre_bracket == 0
            && tracker.pending_decl.is_none()
        {
            let (parsed_vis, mut modifiers, stripped) = parse_rust_qualifiers(after_attrs, false);
            if let Some((kw, after_kw)) = take_ident(stripped) {
                let start_col = san.first_code_raw_byte.unwrap_or(0);
                match kw {
                    "struct" => {
                        if let Some((name, _)) = take_ident(after_kw) {
                            let vis = parsed_vis.unwrap_or_else(|| "private".to_owned());
                            tracker.start_decl_mode(
                                CreatedEntity::new(
                                    "struct",
                                    name,
                                    Some(rel_path.to_owned()),
                                    rel_path,
                                    Some(line_no),
                                )
                                .with_access(Some(vis), modifiers),
                                idx,
                                start_col,
                                true,
                                DeclMode::Braced,
                                Some((name.to_owned(), rel_path.to_owned())),
                                None,
                            );
                            tracker.pending_block = Some(BraceBlock::Struct(name.to_owned()));
                        }
                    }
                    "enum" => {
                        if let Some((name, _)) = take_ident(after_kw) {
                            let vis = parsed_vis.unwrap_or_else(|| "private".to_owned());
                            tracker.start_decl_mode(
                                CreatedEntity::new(
                                    "enum",
                                    name,
                                    Some(rel_path.to_owned()),
                                    rel_path,
                                    Some(line_no),
                                )
                                .with_access(Some(vis), modifiers),
                                idx,
                                start_col,
                                false,
                                DeclMode::Braced,
                                None,
                                None,
                            );
                            tracker.pending_block = Some(BraceBlock::Other);
                        }
                    }
                    "trait" => {
                        if let Some((name, _)) = take_ident(after_kw) {
                            let vis = parsed_vis.unwrap_or_else(|| "private".to_owned());
                            tracker.start_decl_mode(
                                CreatedEntity::new(
                                    "trait",
                                    name,
                                    Some(rel_path.to_owned()),
                                    rel_path,
                                    Some(line_no),
                                )
                                .with_access(Some(vis.clone()), modifiers),
                                idx,
                                start_col,
                                true,
                                DeclMode::Braced,
                                None,
                                None,
                            );
                            tracker.pending_block = Some(BraceBlock::ClassLike {
                                name: name.to_owned(),
                                vis: Some(vis),
                                is_trait: true,
                                is_trait_impl: false,
                            });
                        }
                    }
                    "mod" => {
                        if take_ident(after_kw).is_some() && !trimmed.ends_with(';') {
                            tracker.pending_block = Some(BraceBlock::Mod);
                        }
                    }
                    "impl" => {
                        if let Some((target_ty, is_trait_impl)) =
                            parse_rust_impl_target_meta(after_kw)
                        {
                            tracker.pending_block = Some(BraceBlock::ClassLike {
                                name: target_ty,
                                vis: None,
                                is_trait: false,
                                is_trait_impl,
                            });
                        }
                    }
                    "fn" => {
                        if let Some((name, rest)) = take_ident(after_kw) {
                            let rest_t = rest.trim_start();
                            if rest_t.starts_with('(') || rest_t.starts_with('<') {
                                let (entity, eff_vis) =
                                    if let Some((owner, trait_vis, is_trait, is_trait_impl)) =
                                        active_class_context(&tracker.stack, tracker.depth)
                                    {
                                        let vis = parsed_vis.unwrap_or_else(|| {
                                            if is_trait_impl {
                                                "public".to_owned()
                                            } else if is_trait {
                                                trait_vis.unwrap_or_else(|| "public".to_owned())
                                            } else {
                                                "private".to_owned()
                                            }
                                        });
                                        (
                                            CreatedEntity::new(
                                                "method",
                                                name,
                                                Some(owner),
                                                rel_path,
                                                Some(line_no),
                                            ),
                                            vis,
                                        )
                                    } else if tracker.stack.is_empty()
                                        || matches!(
                                            tracker.stack.last(),
                                            Some((BraceBlock::Mod, _))
                                        )
                                    {
                                        let vis =
                                            parsed_vis.unwrap_or_else(|| "private".to_owned());
                                        (
                                            CreatedEntity::new(
                                                "function",
                                                name,
                                                Some(rel_path.to_owned()),
                                                rel_path,
                                                Some(line_no),
                                            ),
                                            vis,
                                        )
                                    } else {
                                        tracker.apply_events(
                                            &lines,
                                            idx,
                                            &san.events,
                                            Language::Rust,
                                        );
                                        continue;
                                    };
                                tracker.start_decl_mode(
                                    entity.with_access(Some(eff_vis), modifiers),
                                    idx,
                                    start_col,
                                    false,
                                    DeclMode::Braced,
                                    None,
                                    None,
                                );
                                tracker.pending_block = Some(BraceBlock::Fn);
                            }
                        }
                    }
                    "const" | "static" => {
                        let mut ident_src = after_kw;
                        if kw == "static" {
                            push_unique_modifier(&mut modifiers, "static");
                            if let Some(("mut", after_mut)) = take_ident(ident_src) {
                                push_unique_modifier(&mut modifiers, "mut");
                                ident_src = after_mut;
                            }
                        }
                        if let Some((name, after_name)) = take_ident(ident_src)
                            && name != "_"
                        {
                            let after_t = after_name.trim_start();
                            if after_t.starts_with(':') && !after_t.starts_with("::") {
                                let class_ctx = active_class_context(&tracker.stack, tracker.depth);
                                let (parent, eff_vis) =
                                    if let Some((owner, trait_vis, is_trait, is_trait_impl)) =
                                        class_ctx
                                    {
                                        if kw != "const" {
                                            tracker.apply_events(
                                                &lines,
                                                idx,
                                                &san.events,
                                                Language::Rust,
                                            );
                                            continue;
                                        }
                                        let vis = parsed_vis.unwrap_or_else(|| {
                                            if is_trait_impl {
                                                "public".to_owned()
                                            } else if is_trait {
                                                trait_vis.unwrap_or_else(|| "public".to_owned())
                                            } else {
                                                "private".to_owned()
                                            }
                                        });
                                        (Some(owner), vis)
                                    } else if tracker.stack.is_empty()
                                        || matches!(
                                            tracker.stack.last(),
                                            Some((BraceBlock::Mod, _))
                                        )
                                    {
                                        (
                                            Some(rel_path.to_owned()),
                                            parsed_vis.unwrap_or_else(|| "private".to_owned()),
                                        )
                                    } else {
                                        tracker.apply_events(
                                            &lines,
                                            idx,
                                            &san.events,
                                            Language::Rust,
                                        );
                                        continue;
                                    };
                                tracker.start_decl_mode(
                                    CreatedEntity::new(
                                        "const",
                                        name,
                                        parent,
                                        rel_path,
                                        Some(line_no),
                                    )
                                    .with_access(Some(eff_vis), modifiers),
                                    idx,
                                    start_col,
                                    false,
                                    DeclMode::ValueStmt { asi: false },
                                    None,
                                    None,
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        tracker.apply_events(&lines, idx, &san.events, Language::Rust);
    }
    tracker.finish_eof(&lines, Language::Rust)
}

fn parse_rust_qualifiers(s: &str, vis_only: bool) -> (Option<String>, Vec<String>, &str) {
    let mut cur = strip_leading_square_attrs(s);
    let mut vis: Option<String> = None;
    let mut modifiers = Vec::new();
    loop {
        cur = strip_leading_square_attrs(cur);
        if let Some(rest) = cur.strip_prefix("pub")
            && !rest.chars().next().is_some_and(is_ident_char)
        {
            let trimmed = rest.trim_start();
            if let Some(paren_rest) = trimmed.strip_prefix('(') {
                let mut depth = 1usize;
                let mut close_at = None;
                for (idx, c) in paren_rest.char_indices() {
                    match c {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                close_at = Some(idx);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(idx) = close_at {
                    let inside = paren_rest[..idx]
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ");
                    vis = Some(match inside.as_str() {
                        "crate" => "pub(crate)".to_owned(),
                        "super" => "pub(super)".to_owned(),
                        "self" => "private".to_owned(),
                        _ => format!("pub({inside})"),
                    });
                    cur = &paren_rest[idx + 1..];
                    continue;
                }
            }
            vis = Some("public".to_owned());
            cur = trimmed;
            continue;
        }
        if vis_only {
            break;
        }
        let Some((word, rest)) = take_ident(cur) else {
            break;
        };
        match word {
            "async" | "unsafe" | "default" | "extern" => {
                push_unique_modifier(&mut modifiers, word);
                cur = rest;
            }
            "const" => {
                let next_word = take_ident(rest).map(|(w, _)| w);
                if matches!(next_word, Some("fn" | "unsafe" | "extern" | "default")) {
                    push_unique_modifier(&mut modifiers, "const");
                    cur = rest;
                } else {
                    break;
                }
            }
            _ => break,
        }
    }
    (vis, modifiers, cur)
}

fn skip_angle_generics(s: &str) -> &str {
    let trimmed = s.trim_start();
    let Some(rest) = trimmed.strip_prefix('<') else {
        return trimmed;
    };
    let mut depth = 1i32;
    for (idx, c) in rest.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return rest[idx + 1..].trim_start();
                }
            }
            _ => {}
        }
    }
    trimmed
}

fn split_top_level_for(s: &str) -> Option<&str> {
    let mut angle_depth = 0i32;
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => angle_depth += 1,
            b'>' => angle_depth = (angle_depth - 1).max(0),
            b'f' if angle_depth == 0
                && s[i..].starts_with("for")
                && (i == 0 || !is_ident_char(bytes[i - 1] as char))
                && bytes.get(i + 3).is_none_or(|b| !is_ident_char(*b as char)) =>
            {
                return Some(&s[i + 3..]);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn parse_rust_impl_target_meta(after_impl: &str) -> Option<(String, bool)> {
    let mut head = skip_angle_generics(after_impl);
    if let Some((before_brace, _)) = head.split_once('{') {
        head = before_brace;
    }
    if let Some((before_where, _)) = head.split_once(" where ") {
        head = before_where;
    }
    let split_for = split_top_level_for(head);
    let is_trait_impl = split_for.is_some();
    let target_side = split_for.unwrap_or(head);
    let no_generics = target_side.split('<').next().unwrap_or(target_side).trim();
    let last_segment = no_generics
        .rsplit("::")
        .next()
        .unwrap_or(no_generics)
        .trim();
    let cleaned = last_segment.trim_start_matches('&').trim();
    take_ident(cleaned).map(|(ident, _)| (ident.to_owned(), is_trait_impl))
}

fn strip_leading_ts_decorators(mut s: &str) -> &str {
    loop {
        s = s.trim_start();
        let Some(after_at) = s.strip_prefix('@') else {
            return s;
        };
        let mut cur = after_at;
        while let Some((_, rest)) = take_ident(cur) {
            cur = rest;
            if let Some(after_dot) = cur.strip_prefix('.') {
                cur = after_dot;
            } else {
                break;
            }
        }
        cur = cur.trim_start();
        if let Some(in_paren) = cur.strip_prefix('(') {
            let bytes = in_paren.as_bytes();
            let mut i = 0usize;
            let mut depth = 1usize;
            while i < bytes.len() && depth > 0 {
                let ch = bytes[i];
                if ch == b'"' || ch == b'\'' || ch == b'`' {
                    i += 1;
                    while i < bytes.len() {
                        if bytes[i] == b'\\' && i + 1 < bytes.len() {
                            i += 2;
                        } else if bytes[i] == ch {
                            i += 1;
                            break;
                        } else {
                            i += 1;
                        }
                    }
                    continue;
                }
                match ch {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                i += 1;
            }
            if depth == 0 {
                cur = &in_paren[i..];
            } else {
                return "";
            }
        }
        s = cur;
    }
}

fn parse_ts_class_qualifiers(s: &str) -> (Option<String>, Vec<String>, bool, &str) {
    let mut cur = s.trim_start();
    let mut vis: Option<String> = None;
    let mut has_vis = false;
    let mut modifiers = Vec::new();
    loop {
        cur = cur.trim_start();
        let Some((word, rest)) = take_ident(cur) else {
            break;
        };
        let next_ch = rest.trim_start().chars().next();
        let has_follower = next_ch.is_some_and(|c| {
            !matches!(c, '(' | '<' | ':' | '=' | '?' | '!' | ';' | ',' | ')' | '}')
        });
        if !has_follower {
            break;
        }
        match word {
            "public" | "protected" | "private" => {
                vis = Some(word.to_owned());
                has_vis = true;
                cur = rest;
            }
            "static" | "readonly" | "abstract" | "override" | "async" | "declare" | "get"
            | "set" => {
                push_unique_modifier(&mut modifiers, word);
                cur = rest;
            }
            _ => break,
        }
    }
    (vis, modifiers, has_vis, cur)
}

fn parse_ts_top_qualifiers(s: &str) -> (Option<String>, Vec<String>, &str) {
    let mut cur = s.trim_start();
    let mut exported = false;
    let mut modifiers = Vec::new();
    loop {
        cur = cur.trim_start();
        let Some((word, rest)) = take_ident(cur) else {
            break;
        };
        match word {
            "export" => {
                exported = true;
                cur = rest;
            }
            "default" | "declare" | "abstract" | "async" => {
                push_unique_modifier(&mut modifiers, word);
                cur = rest;
            }
            _ => break,
        }
    }
    let vis = if exported {
        "export".to_owned()
    } else {
        "private".to_owned()
    };
    (Some(vis), modifiers, cur)
}

fn is_ts_line_incomplete(trimmed: &str) -> bool {
    let t = trimmed.trim_end();
    if t.is_empty() || t.ends_with(';') || t.ends_with('}') {
        return false;
    }
    t.ends_with("=>")
        || t.ends_with("&&")
        || t.ends_with("||")
        || t.ends_with("??")
        || matches!(
            t.as_bytes().last().copied(),
            Some(
                b'=' | b':'
                    | b','
                    | b'+'
                    | b'-'
                    | b'*'
                    | b'/'
                    | b'?'
                    | b'.'
                    | b'<'
                    | b'('
                    | b'['
                    | b'{'
            )
        )
}

fn extract_ts_ctor_properties(
    lines: &[&str],
    header_start: LineColPos,
    header_end: LineColPos,
    owner: &str,
    rel_path: &str,
) -> Vec<ExtractedDecl> {
    let raw_header = slice_raw_excluding(lines, header_start, header_end, &[]);
    let bytes = raw_header.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut line_off = 0usize;
    let mut open_paren_at: Option<(usize, usize)> = None;
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut brace = 0usize;
    let mut angle = 0usize;
    let mut in_block_comment = false;

    let mut params: Vec<(usize, String)> = Vec::new();
    let mut cur_start: Option<(usize, usize)> = None;

    while i < len {
        let ch = bytes[i];
        if in_block_comment {
            if ch == b'\n' {
                line_off += 1;
            }
            if ch == b'*' && i + 1 < len && bytes[i + 1] == b'/' {
                in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if ch == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            in_block_comment = true;
            i += 2;
            continue;
        }
        if ch == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if ch == b'"' || ch == b'\'' || ch == b'`' {
            i += 1;
            while i < len {
                if bytes[i] == b'\n' {
                    line_off += 1;
                }
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                } else if bytes[i] == ch {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if ch == b'\n' {
            line_off += 1;
            i += 1;
            continue;
        }
        if open_paren_at.is_none() {
            if ch == b'(' {
                paren = 1;
                open_paren_at = Some((i + 1, line_off));
                cur_start = Some((i + 1, line_off));
            }
            i += 1;
            continue;
        }
        if i + 1 < len && matches!(&bytes[i..=i + 1], b"=>" | b"<=" | b">=") {
            i += 2;
            continue;
        }
        match ch {
            b'(' => paren += 1,
            b')' => {
                if paren == 1 && bracket == 0 && brace == 0 && angle == 0 {
                    if let Some((start_idx, start_lo)) = cur_start {
                        params.push((start_lo, raw_header[start_idx..i].to_owned()));
                    }
                    break;
                }
                paren = paren.saturating_sub(1);
            }
            b'[' => bracket += 1,
            b']' => bracket = bracket.saturating_sub(1),
            b'{' => brace += 1,
            b'}' => brace = brace.saturating_sub(1),
            b'<' => angle += 1,
            b'>' => angle = angle.saturating_sub(1),
            b',' if paren == 1 && bracket == 0 && brace == 0 && angle == 0 => {
                if let Some((start_idx, start_lo)) = cur_start {
                    params.push((start_lo, raw_header[start_idx..i].to_owned()));
                }
                cur_start = Some((i + 1, line_off));
            }
            _ => {}
        }
        i += 1;
    }

    let mut out = Vec::new();
    for (rel_line, raw_param) in params {
        let cleaned = strip_comments_keep_strings(&raw_param, Language::TsJs);
        let trimmed = cleaned.trim_start();
        let lead_ws_nl = cleaned[..cleaned.len() - trimmed.len()]
            .bytes()
            .filter(|b| *b == b'\n')
            .count();
        let after_decos = strip_leading_ts_decorators(trimmed).trim_start();
        let deco_nl = trimmed[..trimmed.len() - after_decos.len()]
            .bytes()
            .filter(|b| *b == b'\n')
            .count();
        if after_decos.is_empty() {
            continue;
        }
        let (parsed_vis, modifiers, has_vis, rest) = parse_ts_class_qualifiers(after_decos);
        if !has_vis && !modifiers.iter().any(|m| m == "readonly") {
            continue;
        }
        let Some((name, _)) = take_ident(rest) else {
            continue;
        };
        let vis = Some(parsed_vis.unwrap_or_else(|| "public".to_owned()));
        let (lhs, rhs) = match split_top_level_assign(after_decos, Language::TsJs) {
            Some((l, r)) => (l, Some(r)),
            None => (after_decos.trim(), None),
        };
        let line_no = (header_start.0 + rel_line + lead_ws_nl + deco_nl + 1) as u32;
        let end_nl = after_decos
            .trim_end()
            .bytes()
            .filter(|b| *b == b'\n')
            .count() as u32;
        out.push(ExtractedDecl {
            entity: CreatedEntity::new(
                "property",
                name,
                Some(owner.to_owned()),
                rel_path,
                Some(line_no),
            )
            .with_access(vis.clone(), modifiers.clone()),
            signature: normalize_signature(lhs, Language::TsJs),
            params_signature: normalize_params_signature(lhs, Language::TsJs),
            body_fingerprint: rhs
                .map(|r| fingerprint_body(r, Language::TsJs))
                .unwrap_or_default(),
            visibility: vis,
            modifiers,
            start_line: line_no,
            end_line: line_no + end_nl,
        });
    }
    out
}

fn extract_ts_js_decls(rel_path: &str, source: &str) -> Vec<ExtractedDecl> {
    const CONTROL_KEYWORDS: &[&str] = &[
        "if", "else", "for", "while", "do", "switch", "case", "try", "catch", "finally", "return",
        "throw", "typeof", "new", "delete", "await", "super", "this", "function", "break",
        "continue", "import", "export",
    ];

    let lines: Vec<&str> = source.lines().collect();
    let mut tracker = BraceSpanTracker::new();
    let mut in_block_comment = false;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut last_code_line: Option<usize> = None;
    let mut prev_incomplete = false;

    for (idx, raw_line) in lines.iter().enumerate() {
        let line_no = (idx + 1) as u32;
        let pre_paren = paren_depth;
        let pre_bracket = bracket_depth;
        let san = sanitize_c_like_with_events(
            raw_line,
            &mut in_block_comment,
            &mut paren_depth,
            &mut bracket_depth,
            Language::TsJs,
        );
        let trimmed = san.text.trim();
        if trimmed.is_empty() {
            continue;
        }
        if pre_paren == 0
            && pre_bracket == 0
            && !trimmed.starts_with('{')
            && let Some(prev_idx) = last_code_line
            && (trimmed.starts_with('}') || (!prev_incomplete && !trimmed.starts_with("=>")))
        {
            tracker.flush_pending_asi(&lines, prev_idx, Language::TsJs);
        }

        let after_deco = strip_leading_ts_decorators(trimmed);
        let start_col = san.first_code_raw_byte.unwrap_or(0);
        if !after_deco.is_empty()
            && pre_paren == 0
            && pre_bracket == 0
            && tracker.pending_decl.is_none()
        {
            if let Some(owner) = active_class_parent(&tracker.stack, tracker.depth) {
                let (parsed_vis, modifiers, _, stripped) = parse_ts_class_qualifiers(after_deco);
                let stripped = stripped.trim_start_matches('*').trim_start();
                if stripped.starts_with('{') {
                    tracker.pending_block = Some(BraceBlock::Fn);
                } else if let Some((name, rest)) = take_ts_member_ident(stripped)
                    && !CONTROL_KEYWORDS.contains(&name)
                {
                    let rest_t = rest.trim_start();
                    let rest_after_qp = rest_t.trim_start_matches(['?', '!']).trim_start();
                    let vis = if name.starts_with('#') {
                        Some("#private".to_owned())
                    } else {
                        Some(parsed_vis.unwrap_or_else(|| "public".to_owned()))
                    };
                    if rest_after_qp.starts_with('(') || rest_after_qp.starts_with('<') {
                        let ctor_owner =
                            (name == "constructor").then(|| (owner.clone(), rel_path.to_owned()));
                        tracker.start_decl_mode(
                            CreatedEntity::new(
                                "method",
                                name,
                                Some(owner),
                                rel_path,
                                Some(line_no),
                            )
                            .with_access(vis, modifiers),
                            idx,
                            start_col,
                            false,
                            DeclMode::Braced,
                            None,
                            ctor_owner,
                        );
                        tracker.pending_block = Some(BraceBlock::Fn);
                    } else if rest_after_qp.starts_with(':')
                        || (rest_after_qp.starts_with('=') && !rest_after_qp.starts_with("=="))
                        || rest_after_qp.is_empty()
                        || rest_after_qp.starts_with(';')
                    {
                        let assign_rhs =
                            split_top_level_assign(stripped, Language::TsJs).map(|(_, rhs)| rhs);
                        if let Some(rhs) = assign_rhs
                            && is_js_function_rhs(rhs)
                        {
                            let mode = if rhs.contains("=>") {
                                DeclMode::ArrowFn
                            } else {
                                DeclMode::Braced
                            };
                            tracker.start_decl_mode(
                                CreatedEntity::new(
                                    "property",
                                    name,
                                    Some(owner),
                                    rel_path,
                                    Some(line_no),
                                )
                                .with_access(vis, modifiers),
                                idx,
                                start_col,
                                false,
                                mode,
                                None,
                                None,
                            );
                            tracker.pending_block = Some(BraceBlock::Fn);
                        } else {
                            tracker.start_decl_mode(
                                CreatedEntity::new(
                                    "property",
                                    name,
                                    Some(owner),
                                    rel_path,
                                    Some(line_no),
                                )
                                .with_access(vis, modifiers),
                                idx,
                                start_col,
                                false,
                                DeclMode::ValueStmt { asi: true },
                                None,
                                None,
                            );
                        }
                    }
                }
            } else if !inside_fn_scope(&tracker.stack) {
                let (vis, modifiers, stripped) = parse_ts_top_qualifiers(after_deco);
                if let Some((kw, after_kw)) = take_ident(stripped) {
                    match kw {
                        "class" => {
                            if let Some((name, _)) = take_ident(after_kw) {
                                tracker.start_decl_mode(
                                    CreatedEntity::new(
                                        "class",
                                        name,
                                        Some(rel_path.to_owned()),
                                        rel_path,
                                        Some(line_no),
                                    )
                                    .with_access(vis.clone(), modifiers),
                                    idx,
                                    start_col,
                                    true,
                                    DeclMode::Braced,
                                    None,
                                    None,
                                );
                                tracker.pending_block = Some(BraceBlock::ClassLike {
                                    name: name.to_owned(),
                                    vis,
                                    is_trait: false,
                                    is_trait_impl: false,
                                });
                            }
                        }
                        "interface" | "enum" => {
                            if let Some((name, _)) = take_ident(after_kw) {
                                tracker.start_decl_mode(
                                    CreatedEntity::new(
                                        kw,
                                        name,
                                        Some(rel_path.to_owned()),
                                        rel_path,
                                        Some(line_no),
                                    )
                                    .with_access(vis, modifiers),
                                    idx,
                                    start_col,
                                    false,
                                    DeclMode::Braced,
                                    None,
                                    None,
                                );
                                tracker.pending_block = Some(BraceBlock::Other);
                            }
                        }
                        "function" => {
                            let after_star = after_kw.trim_start().trim_start_matches('*');
                            if let Some((name, rest)) = take_ident(after_star) {
                                let rest_t = rest.trim_start();
                                if rest_t.starts_with('(') || rest_t.starts_with('<') {
                                    tracker.start_decl_mode(
                                        CreatedEntity::new(
                                            "function",
                                            name,
                                            Some(rel_path.to_owned()),
                                            rel_path,
                                            Some(line_no),
                                        )
                                        .with_access(vis, modifiers),
                                        idx,
                                        start_col,
                                        false,
                                        DeclMode::Braced,
                                        None,
                                        None,
                                    );
                                    tracker.pending_block = Some(BraceBlock::Fn);
                                }
                            }
                        }
                        "const" | "let" | "var" => {
                            if kw == "const"
                                && let Some(("enum", after_enum)) = take_ident(after_kw)
                            {
                                if let Some((name, _)) = take_ident(after_enum) {
                                    tracker.start_decl_mode(
                                        CreatedEntity::new(
                                            "enum",
                                            name,
                                            Some(rel_path.to_owned()),
                                            rel_path,
                                            Some(line_no),
                                        )
                                        .with_access(vis, modifiers),
                                        idx,
                                        start_col,
                                        false,
                                        DeclMode::Braced,
                                        None,
                                        None,
                                    );
                                    tracker.pending_block = Some(BraceBlock::Other);
                                }
                            } else if let Some((name, rest)) = take_ident(after_kw) {
                                let assign_rhs = split_top_level_assign(rest, Language::TsJs)
                                    .map(|(_, rhs)| rhs);
                                if let Some(rhs) = assign_rhs
                                    && is_js_function_rhs(rhs)
                                {
                                    let mode = if rhs.contains("=>") {
                                        DeclMode::ArrowFn
                                    } else {
                                        DeclMode::Braced
                                    };
                                    tracker.start_decl_mode(
                                        CreatedEntity::new(
                                            "function",
                                            name,
                                            Some(rel_path.to_owned()),
                                            rel_path,
                                            Some(line_no),
                                        )
                                        .with_access(vis, modifiers),
                                        idx,
                                        start_col,
                                        false,
                                        mode,
                                        None,
                                        None,
                                    );
                                    tracker.pending_block = Some(BraceBlock::Fn);
                                } else if kw == "const" {
                                    let rest_t = rest.trim_start();
                                    if rest_t.starts_with(':')
                                        || rest_t.starts_with('=')
                                        || rest_t.starts_with(';')
                                        || rest_t.is_empty()
                                    {
                                        tracker.start_decl_mode(
                                            CreatedEntity::new(
                                                "const",
                                                name,
                                                Some(rel_path.to_owned()),
                                                rel_path,
                                                Some(line_no),
                                            )
                                            .with_access(vis, modifiers),
                                            idx,
                                            start_col,
                                            false,
                                            DeclMode::ValueStmt { asi: true },
                                            None,
                                            None,
                                        );
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        tracker.apply_events(&lines, idx, &san.events, Language::TsJs);
        last_code_line = Some(idx);
        prev_incomplete = if in_block_comment {
            is_ts_line_incomplete(trimmed)
        } else {
            is_ts_line_incomplete(strip_comments_keep_strings(raw_line, Language::TsJs).trim())
        };
    }
    tracker.finish_eof(&lines, Language::TsJs)
}

fn is_js_function_rhs(rhs: &str) -> bool {
    let trimmed = strip_leading_words(rhs, &["async"]);
    if trimmed.starts_with("function") {
        return true;
    }
    rhs.contains("=>")
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PyBlock {
    Class {
        name: String,
        seen_attrs: BTreeSet<String>,
    },
    Def {
        owner_class: Option<String>,
        is_init: bool,
    },
}

struct PySpan {
    decl_idx: usize,
    start_line: usize,
    start_col: usize,
    is_class: bool,
    excluded_children: Vec<((usize, usize), (usize, usize))>,
}

fn python_paren_delta(line: &str, triple: &mut Option<&'static str>) -> isize {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut delta = 0isize;
    let mut i = 0usize;
    while i < len {
        if let Some(marker) = *triple {
            if line[i..].starts_with(marker) {
                *triple = None;
                i += marker.len();
            } else {
                i += 1;
            }
            continue;
        }
        if line[i..].starts_with("\"\"\"") {
            *triple = Some("\"\"\"");
            i += 3;
            continue;
        }
        if line[i..].starts_with("'''") {
            *triple = Some("'''");
            i += 3;
            continue;
        }
        let ch = bytes[i];
        if ch == b'#' {
            break;
        }
        if ch == b'"' || ch == b'\'' {
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                } else if bytes[i] == ch {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        match ch {
            b'(' | b'[' | b'{' => delta += 1,
            b')' | b']' | b'}' => delta -= 1,
            _ => {}
        }
        i += 1;
    }
    delta
}

fn find_python_header_colon(
    lines: &[&str],
    start_line: usize,
    start_col: usize,
    end_line: usize,
) -> Option<(usize, usize)> {
    let mut triple: Option<&'static str> = None;
    let mut paren = 0usize;
    for (li, raw_line) in lines
        .iter()
        .enumerate()
        .take(end_line.min(lines.len().saturating_sub(1)) + 1)
        .skip(start_line)
    {
        let bytes = raw_line.as_bytes();
        let len = bytes.len();
        let mut i = if li == start_line {
            start_col.min(len)
        } else {
            0
        };
        while i < len {
            if let Some(marker) = triple {
                if raw_line[i..].starts_with(marker) {
                    triple = None;
                    i += marker.len();
                } else {
                    i += 1;
                }
                continue;
            }
            if raw_line[i..].starts_with("\"\"\"") {
                triple = Some("\"\"\"");
                i += 3;
                continue;
            }
            if raw_line[i..].starts_with("'''") {
                triple = Some("'''");
                i += 3;
                continue;
            }
            let ch = bytes[i];
            if ch == b'#' {
                break;
            }
            if ch == b'"' || ch == b'\'' {
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' && i + 1 < len {
                        i += 2;
                    } else if bytes[i] == ch {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
                continue;
            }
            match ch {
                b'(' | b'[' | b'{' => paren += 1,
                b')' | b']' | b'}' => paren = paren.saturating_sub(1),
                b':' if paren == 0 => return Some((li, i)),
                _ => {}
            }
            i += 1;
        }
    }
    None
}

fn trim_python_end_line(lines: &[&str], start_line: usize, candidate_end: usize) -> usize {
    let mut end = candidate_end.min(lines.len().saturating_sub(1));
    while end > start_line {
        let t = lines[end].trim();
        if t.is_empty() || t.starts_with('#') || t.starts_with('@') {
            end -= 1;
        } else {
            break;
        }
    }
    end
}

fn finalize_python_span(
    decls: &mut [ExtractedDecl],
    lines: &[&str],
    span: &PySpan,
    end_idx: usize,
) {
    let end_idx = trim_python_end_line(lines, span.start_line, end_idx);
    decls[span.decl_idx].end_line = (end_idx + 1) as u32;
    let end_col = lines.get(end_idx).map_or(0, |l| l.len());
    if let Some((colon_line, colon_col)) =
        find_python_header_colon(lines, span.start_line, span.start_col, end_idx)
    {
        let header = slice_raw_excluding(
            lines,
            (span.start_line, span.start_col),
            (colon_line, colon_col),
            &[],
        );
        let sig = normalize_signature(&header, Language::Python);
        decls[span.decl_idx].signature = sig.clone();
        decls[span.decl_idx].params_signature = sig;
        let body = slice_raw_excluding(
            lines,
            (colon_line, colon_col + 1),
            (end_idx, end_col),
            &span.excluded_children,
        );
        decls[span.decl_idx].body_fingerprint = fingerprint_body(&body, Language::Python);
    } else {
        let header = slice_raw_excluding(
            lines,
            (span.start_line, span.start_col),
            (end_idx, end_col),
            &[],
        );
        let sig = normalize_signature(&header, Language::Python);
        decls[span.decl_idx].signature = sig.clone();
        decls[span.decl_idx].params_signature = sig;
    }
}

fn pop_python_blocks(
    stack: &mut Vec<(usize, PyBlock, Option<PySpan>)>,
    decls: &mut [ExtractedDecl],
    lines: &[&str],
    min_indent: Option<usize>,
    candidate_end: usize,
) {
    while stack
        .last()
        .is_some_and(|(blk_indent, _, _)| min_indent.is_none_or(|indent| indent <= *blk_indent))
    {
        let (_, _, maybe_span) = stack.pop().expect("checked");
        if let Some(span) = maybe_span {
            let end_idx = trim_python_end_line(lines, span.start_line, candidate_end);
            finalize_python_span(decls, lines, &span, end_idx);
            let end_col = lines.get(end_idx).map_or(0, |l| l.len());
            if let Some((_, _, Some(parent_span))) = stack.last_mut()
                && parent_span.is_class
            {
                parent_span
                    .excluded_children
                    .push(((span.start_line, span.start_col), (end_idx, end_col)));
            }
        }
    }
}

fn python_pep8_visibility(name: &str) -> String {
    if name.starts_with("__") && name.ends_with("__") && name.len() > 4 {
        "public".to_owned()
    } else if name.starts_with("__") {
        "private".to_owned()
    } else if name.starts_with('_') {
        "protected".to_owned()
    } else {
        "public".to_owned()
    }
}

fn parse_python_decorator(trimmed_at_line: &str) -> Option<&str> {
    let rest = trimmed_at_line.strip_prefix('@')?.trim_start();
    let end = rest
        .find(|c: char| !is_ident_char(c) && c != '.')
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

fn python_stmt_end(
    lines: &[&str],
    start_idx: usize,
    post_paren_depth: usize,
    post_paren_triple: Option<&'static str>,
) -> usize {
    let mut depth = post_paren_depth;
    let mut triple = post_paren_triple;
    let mut cur = start_idx;
    while depth > 0 && cur + 1 < lines.len() {
        cur += 1;
        let delta = python_paren_delta(lines[cur], &mut triple);
        depth = (depth as isize + delta).max(0) as usize;
    }
    cur
}

fn extract_python_decls(rel_path: &str, source: &str) -> Vec<ExtractedDecl> {
    const PY_NON_ATTR_KEYWORDS: &[&str] = &[
        "if", "elif", "else", "for", "while", "try", "except", "finally", "with", "match", "case",
        "return", "raise", "assert", "pass", "break", "continue", "import", "from", "global",
        "nonlocal", "del", "yield", "type", "class", "def", "async",
    ];

    let lines: Vec<&str> = source.lines().collect();
    let mut decls = Vec::new();
    let mut stack: Vec<(usize, PyBlock, Option<PySpan>)> = Vec::new();
    let mut pending_decorators: Vec<(usize, String)> = Vec::new();
    let mut triple_quote: Option<&'static str> = None;
    let mut paren_triple: Option<&'static str> = None;
    let mut paren_depth = 0usize;

    for (idx, raw_line) in lines.iter().enumerate() {
        let line_no = (idx + 1) as u32;
        let pre_paren_depth = paren_depth;
        let delta = python_paren_delta(raw_line, &mut paren_triple);
        paren_depth = (paren_depth as isize + delta).max(0) as usize;

        if let Some(marker) = triple_quote {
            if raw_line.contains(marker) {
                triple_quote = None;
            }
            continue;
        }
        let mut indent = 0usize;
        let mut start_col = 0usize;
        let mut rest = "";
        for (at, ch) in raw_line.char_indices() {
            match ch {
                ' ' => indent += 1,
                '\t' => indent = (indent / 4 + 1) * 4,
                _ => {
                    start_col = at;
                    rest = &raw_line[at..];
                    break;
                }
            }
        }
        let trimmed = rest.trim_end();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        for marker in ["\"\"\"", "'''"] {
            if let Some(after) = trimmed.strip_prefix(marker) {
                if !after.contains(marker) {
                    triple_quote = Some(marker);
                }
                rest = "";
                break;
            }
        }
        if rest.is_empty() || pre_paren_depth > 0 {
            continue;
        }

        if idx > 0 {
            pop_python_blocks(&mut stack, &mut decls, &lines, Some(indent), idx - 1);
        }

        if trimmed.starts_with('@') {
            if let Some(deco) = parse_python_decorator(trimmed) {
                pending_decorators.push((indent, deco.to_owned()));
            }
            continue;
        }

        let matched_decorators: Vec<String> = pending_decorators
            .drain(..)
            .filter_map(|(d_indent, raw_deco)| (d_indent == indent).then_some(raw_deco))
            .collect();

        let stmt = trimmed
            .strip_prefix("async ")
            .unwrap_or(trimmed)
            .trim_start();
        let mut matched_kw_decl = false;
        if let Some((kw, after_kw)) = take_ident(stmt) {
            match kw {
                "class" => {
                    matched_kw_decl = true;
                    if let Some((name, after_name)) = take_ident(after_kw) {
                        let tail = after_name.trim_start();
                        if tail.starts_with(':') || tail.starts_with('(') {
                            let mut modifiers = Vec::new();
                            for raw_deco in &matched_decorators {
                                let short = raw_deco.rsplit('.').next().unwrap_or(raw_deco);
                                if short == "final" {
                                    push_unique_modifier(&mut modifiers, short);
                                }
                            }
                            let vis = Some(python_pep8_visibility(name));
                            let decl_idx = decls.len();
                            decls.push(ExtractedDecl {
                                entity: CreatedEntity::new(
                                    "class",
                                    name,
                                    Some(rel_path.to_owned()),
                                    rel_path,
                                    Some(line_no),
                                )
                                .with_access(vis.clone(), modifiers.clone()),
                                signature: String::new(),
                                params_signature: String::new(),
                                body_fingerprint: String::new(),
                                visibility: vis,
                                modifiers,
                                start_line: line_no,
                                end_line: line_no,
                            });
                            stack.push((
                                indent,
                                PyBlock::Class {
                                    name: name.to_owned(),
                                    seen_attrs: BTreeSet::new(),
                                },
                                Some(PySpan {
                                    decl_idx,
                                    start_line: idx,
                                    start_col,
                                    is_class: true,
                                    excluded_children: Vec::new(),
                                }),
                            ));
                        }
                    }
                }
                "def" => {
                    matched_kw_decl = true;
                    if let Some((name, after_name)) = take_ident(after_kw)
                        && after_name.trim_start().starts_with('(')
                    {
                        let mut modifiers = Vec::new();
                        let mut is_property = false;
                        let mut is_setter = false;
                        for raw_deco in &matched_decorators {
                            let short = raw_deco.rsplit('.').next().unwrap_or(raw_deco);
                            match short {
                                "property" => {
                                    push_unique_modifier(&mut modifiers, "property");
                                    is_property = true;
                                }
                                "setter" | "deleter" if raw_deco.contains('.') => {
                                    is_property = true;
                                    if short == "setter" {
                                        is_setter = true;
                                    }
                                }
                                "staticmethod" | "classmethod" | "abstractmethod" | "final" => {
                                    push_unique_modifier(&mut modifiers, short);
                                }
                                _ => {}
                            }
                        }
                        let vis = Some(python_pep8_visibility(name));
                        match stack.last_mut() {
                            Some((
                                _,
                                PyBlock::Class {
                                    name: owner,
                                    seen_attrs,
                                },
                                _,
                            )) => {
                                let owner_name = owner.clone();
                                if is_property && is_setter && seen_attrs.contains(name) {
                                    stack.push((
                                        indent,
                                        PyBlock::Def {
                                            owner_class: None,
                                            is_init: false,
                                        },
                                        None,
                                    ));
                                } else {
                                    let kind = if is_property {
                                        seen_attrs.insert(name.to_owned());
                                        "property"
                                    } else {
                                        "method"
                                    };
                                    let decl_idx = decls.len();
                                    decls.push(ExtractedDecl {
                                        entity: CreatedEntity::new(
                                            kind,
                                            name,
                                            Some(owner_name.clone()),
                                            rel_path,
                                            Some(line_no),
                                        )
                                        .with_access(vis.clone(), modifiers.clone()),
                                        signature: String::new(),
                                        params_signature: String::new(),
                                        body_fingerprint: String::new(),
                                        visibility: vis,
                                        modifiers,
                                        start_line: line_no,
                                        end_line: line_no,
                                    });
                                    stack.push((
                                        indent,
                                        PyBlock::Def {
                                            owner_class: Some(owner_name),
                                            is_init: !is_property && name == "__init__",
                                        },
                                        Some(PySpan {
                                            decl_idx,
                                            start_line: idx,
                                            start_col,
                                            is_class: false,
                                            excluded_children: Vec::new(),
                                        }),
                                    ));
                                }
                            }
                            None => {
                                let decl_idx = decls.len();
                                decls.push(ExtractedDecl {
                                    entity: CreatedEntity::new(
                                        "function",
                                        name,
                                        Some(rel_path.to_owned()),
                                        rel_path,
                                        Some(line_no),
                                    )
                                    .with_access(vis.clone(), modifiers.clone()),
                                    signature: String::new(),
                                    params_signature: String::new(),
                                    body_fingerprint: String::new(),
                                    visibility: vis,
                                    modifiers,
                                    start_line: line_no,
                                    end_line: line_no,
                                });
                                stack.push((
                                    indent,
                                    PyBlock::Def {
                                        owner_class: None,
                                        is_init: false,
                                    },
                                    Some(PySpan {
                                        decl_idx,
                                        start_line: idx,
                                        start_col,
                                        is_class: false,
                                        excluded_children: Vec::new(),
                                    }),
                                ));
                            }
                            Some((_, PyBlock::Def { .. }, _)) => {
                                stack.push((
                                    indent,
                                    PyBlock::Def {
                                        owner_class: None,
                                        is_init: false,
                                    },
                                    None,
                                ));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if !matched_kw_decl {
            let end_idx = python_stmt_end(&lines, idx, paren_depth, paren_triple);
            let end_col = lines.get(end_idx).map_or(0, |l| l.len());
            let raw_stmt = slice_raw_excluding(&lines, (idx, start_col), (end_idx, end_col), &[]);
            let code_stmt = strip_comments_keep_strings(&raw_stmt, Language::Python);
            let code_line = code_stmt.trim();
            let first_line_code = strip_comments_keep_strings(trimmed, Language::Python);
            let first_code = first_line_code.trim();

            if let Some((
                _,
                PyBlock::Class {
                    name: owner,
                    seen_attrs,
                },
                parent_span,
            )) = stack.last_mut()
            {
                if let Some((attr_name, after_name)) = take_ident(first_code)
                    && !PY_NON_ATTR_KEYWORDS.contains(&attr_name)
                {
                    let after_t = after_name.trim_start();
                    if after_t.starts_with(':') && !after_t.starts_with(":=") {
                        if seen_attrs.insert(attr_name.to_owned()) {
                            let (lhs, rhs) =
                                match split_top_level_assign(code_line, Language::Python) {
                                    Some((l, r)) => (l, Some(r)),
                                    None => (code_line, None),
                                };
                            let vis = Some(python_pep8_visibility(attr_name));
                            let sig = normalize_signature(lhs, Language::Python);
                            decls.push(ExtractedDecl {
                                entity: CreatedEntity::new(
                                    "property",
                                    attr_name,
                                    Some(owner.clone()),
                                    rel_path,
                                    Some(line_no),
                                )
                                .with_access(vis.clone(), Vec::new()),
                                signature: sig.clone(),
                                params_signature: sig,
                                body_fingerprint: rhs
                                    .map(|r| fingerprint_body(r, Language::Python))
                                    .unwrap_or_default(),
                                visibility: vis,
                                modifiers: Vec::new(),
                                start_line: line_no,
                                end_line: (end_idx + 1) as u32,
                            });
                            if let Some(ps) = parent_span.as_mut() {
                                ps.excluded_children
                                    .push(((idx, start_col), (end_idx, end_col)));
                            }
                        }
                    } else if is_upper_snake(attr_name)
                        && after_t.starts_with('=')
                        && !after_t.starts_with("==")
                    {
                        let (lhs, rhs) = match split_top_level_assign(code_line, Language::Python) {
                            Some((l, r)) => (l, Some(r)),
                            None => (code_line, None),
                        };
                        let vis = Some(python_pep8_visibility(attr_name));
                        let sig = normalize_signature(lhs, Language::Python);
                        decls.push(ExtractedDecl {
                            entity: CreatedEntity::new(
                                "const",
                                attr_name,
                                Some(owner.clone()),
                                rel_path,
                                Some(line_no),
                            )
                            .with_access(vis.clone(), Vec::new()),
                            signature: sig.clone(),
                            params_signature: sig,
                            body_fingerprint: rhs
                                .map(|r| fingerprint_body(r, Language::Python))
                                .unwrap_or_default(),
                            visibility: vis,
                            modifiers: Vec::new(),
                            start_line: line_no,
                            end_line: (end_idx + 1) as u32,
                        });
                        if let Some(ps) = parent_span.as_mut() {
                            ps.excluded_children
                                .push(((idx, start_col), (end_idx, end_col)));
                        }
                    }
                }
            } else if let Some((
                _,
                PyBlock::Def {
                    owner_class: Some(owner),
                    is_init,
                },
                _,
            )) = stack.last()
            {
                if let Some(after_self) = first_code.strip_prefix("self.")
                    && let Some((attr_name, after_attr)) = take_ident(after_self)
                {
                    let after_t = after_attr.trim_start();
                    let is_annotated = after_t.starts_with(':') && !after_t.starts_with(":=");
                    let is_init_assign =
                        *is_init && after_t.starts_with('=') && !after_t.starts_with("==");
                    if is_annotated || is_init_assign {
                        let owner_name = owner.clone();
                        let inserted = stack
                            .iter_mut()
                            .rev()
                            .find_map(|(_, b, _)| match b {
                                PyBlock::Class { seen_attrs, .. } => {
                                    Some(seen_attrs.insert(attr_name.to_owned()))
                                }
                                _ => None,
                            })
                            .unwrap_or(false);
                        if inserted {
                            let (lhs, rhs) =
                                match split_top_level_assign(code_line, Language::Python) {
                                    Some((l, r)) => (l, Some(r)),
                                    None => (code_line, None),
                                };
                            let vis = Some(python_pep8_visibility(attr_name));
                            let sig = normalize_signature(lhs, Language::Python);
                            decls.push(ExtractedDecl {
                                entity: CreatedEntity::new(
                                    "property",
                                    attr_name,
                                    Some(owner_name),
                                    rel_path,
                                    Some(line_no),
                                )
                                .with_access(vis.clone(), Vec::new()),
                                signature: sig.clone(),
                                params_signature: sig,
                                body_fingerprint: rhs
                                    .map(|r| fingerprint_body(r, Language::Python))
                                    .unwrap_or_default(),
                                visibility: vis,
                                modifiers: Vec::new(),
                                start_line: line_no,
                                end_line: (end_idx + 1) as u32,
                            });
                        }
                    }
                }
            } else if stack.is_empty()
                && let Some((const_name, after_name)) = take_ident(first_code)
                && is_upper_snake(const_name)
            {
                let after_t = after_name.trim_start();
                if (after_t.starts_with(':') && !after_t.starts_with(":="))
                    || (after_t.starts_with('=') && !after_t.starts_with("=="))
                {
                    let (lhs, rhs) = match split_top_level_assign(code_line, Language::Python) {
                        Some((l, r)) => (l, Some(r)),
                        None => (code_line, None),
                    };
                    let vis = Some(python_pep8_visibility(const_name));
                    let sig = normalize_signature(lhs, Language::Python);
                    decls.push(ExtractedDecl {
                        entity: CreatedEntity::new(
                            "const",
                            const_name,
                            Some(rel_path.to_owned()),
                            rel_path,
                            Some(line_no),
                        )
                        .with_access(vis.clone(), Vec::new()),
                        signature: sig.clone(),
                        params_signature: sig,
                        body_fingerprint: rhs
                            .map(|r| fingerprint_body(r, Language::Python))
                            .unwrap_or_default(),
                        visibility: vis,
                        modifiers: Vec::new(),
                        start_line: line_no,
                        end_line: (end_idx + 1) as u32,
                    });
                }
            }
        }
    }
    if !lines.is_empty() {
        pop_python_blocks(&mut stack, &mut decls, &lines, None, lines.len() - 1);
    }
    decls
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn sample_index(root: &Path, graph_json: Value) -> Index {
        let mut cursor = std::io::Cursor::new(graph_json.to_string().into_bytes());
        Index::read(&mut cursor, root).expect("graph index")
    }

    #[test]
    fn predicts_targets_from_index_paths_attachments_and_capsules() {
        let dir = TempDir::new().expect("temp dir");
        let root = dir.path();
        fs::create_dir_all(root.join("src/Auth")).expect("auth dir");
        fs::create_dir_all(root.join("src/Billing")).expect("billing dir");
        fs::write(
            root.join("src/Auth/JwtGuard.php"),
            "<?php\nclass JwtGuard {}\n",
        )
        .expect("php");
        fs::write(
            root.join("src/Billing/Invoice.php"),
            "<?php\nclass Invoice {}\n",
        )
        .expect("invoice");
        fs::write(root.join("docs/SPEC.md"), "# Auth\n").ok();

        let graph = json!({
            "nodes": [
                {"id": "n1", "label": "JwtGuard", "source_file": "src/Auth/JwtGuard.php", "node_kind": "class", "source_location": "line 2"},
                {"id": "n2", "label": "verifyToken", "source_file": "src/Auth/JwtGuard.php", "node_kind": "method", "_callable_class": "JwtGuard", "source_location": "line 8"},
                {"id": "n3", "label": "Invoice", "source_file": "src/Billing/Invoice.php", "node_kind": "class", "source_location": "line 2"}
            ],
            "links": [
                {"source": "n1", "target": "n2", "relation": "contains"}
            ]
        });
        let index = sample_index(root, graph);
        let focus = predict_targets(
            root,
            Some(&index),
            "Update `JwtGuard` and verifyToken inside src/Auth/JwtGuard.php",
            &["src/Billing/Invoice.php".to_owned()],
            &[],
        );

        assert_eq!(focus.phase, "predicting");
        assert!(focus.summary.contains("src/Auth"), "{}", focus.summary);
        assert!(focus.target_folders.contains(&"src/Auth".to_owned()));
        assert!(
            focus
                .target_paths
                .contains(&"src/Auth/JwtGuard.php".to_owned())
        );
        assert!(
            focus
                .target_paths
                .contains(&"src/Billing/Invoice.php".to_owned())
        );
        assert!(
            focus
                .target_nodes
                .contains(&"src/Auth/JwtGuard.php".to_owned())
        );
        assert!(focus.created_entities.is_empty());
    }

    #[test]
    fn predicts_targets_without_index_using_workspace_walk() {
        let dir = TempDir::new().expect("temp dir");
        let root = dir.path();
        fs::create_dir_all(root.join("src/Auth")).expect("dir");
        let mut f = fs::File::create(root.join("src/Auth/token_service.rs")).expect("file");
        writeln!(f, "pub struct TokenService;").expect("write");

        let focus = predict_targets(
            root,
            None,
            "Fix token_service validation in Auth",
            &[],
            &["src/Auth/token_service.rs".to_owned()],
        );
        assert_eq!(focus.phase, "predicting");
        assert!(focus.target_folders.contains(&"src/Auth".to_owned()));
        assert!(
            focus
                .target_paths
                .contains(&"src/Auth/token_service.rs".to_owned())
        );
    }

    #[test]
    fn extracts_php_classes_traits_interfaces_enums_functions_and_methods() {
        let php = r#"<?php
namespace App\Auth;

interface GuardInterface {
    public function authenticate(string $token): bool;
}

trait AuditTrait {
    protected function recordAudit(string $msg): void {}
}

enum TokenStatus: string {
    case Active = 'active';
}

final class JwtGuard implements GuardInterface {
    use AuditTrait;

    public function authenticate(string $token): bool {
        // function insideComment() {}
        $cb = function ($x) { return $x; };
        return true;
    }
}

function helper_verify(string $raw): bool {
    return $raw !== '';
}
"#;
        let entities = extract_declarations("src/Auth/JwtGuard.php", php);
        let summary: Vec<(&str, &str, Option<&str>)> = entities
            .iter()
            .map(|e| (e.kind.as_str(), e.name.as_str(), e.parent.as_deref()))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("interface", "GuardInterface", Some("src/Auth/JwtGuard.php")),
                ("method", "authenticate", Some("GuardInterface")),
                ("trait", "AuditTrait", Some("src/Auth/JwtGuard.php")),
                ("method", "recordAudit", Some("AuditTrait")),
                ("enum", "TokenStatus", Some("src/Auth/JwtGuard.php")),
                ("class", "JwtGuard", Some("src/Auth/JwtGuard.php")),
                ("method", "authenticate", Some("JwtGuard")),
                ("function", "helper_verify", Some("src/Auth/JwtGuard.php")),
            ]
        );
    }

    #[test]
    fn extracts_rust_structs_enums_traits_fns_and_impl_methods_with_lifetimes() {
        let rust = r#"
pub trait Verifier {
    fn verify<'a>(&'a self, token: &'a str) -> bool;
}

pub enum AuthError {
    Expired,
}

pub struct JwtService<'a> {
    pub secret: &'a str,
}

impl<'a> Verifier for JwtService<'a> {
    fn verify<'b>(&'b self, token: &'b str) -> bool {
        fn inner_helper() {}
        !token.is_empty()
    }
}

impl<'a> JwtService<'a> {
    pub async fn issue_token(&self) -> String {
        String::from("{not a block}")
    }
}

pub fn build_service<'a>(secret: &'a str) -> JwtService<'a> {
    JwtService { secret }
}
"#;
        let entities = extract_declarations("src/auth/jwt.rs", rust);
        let summary: Vec<(&str, &str, Option<&str>)> = entities
            .iter()
            .map(|e| (e.kind.as_str(), e.name.as_str(), e.parent.as_deref()))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("trait", "Verifier", Some("src/auth/jwt.rs")),
                ("method", "verify", Some("Verifier")),
                ("enum", "AuthError", Some("src/auth/jwt.rs")),
                ("struct", "JwtService", Some("src/auth/jwt.rs")),
                ("field", "secret", Some("JwtService")),
                ("method", "verify", Some("JwtService")),
                ("method", "issue_token", Some("JwtService")),
                ("function", "build_service", Some("src/auth/jwt.rs")),
            ]
        );
    }

    #[test]
    fn extracts_ts_and_python_declarations() {
        let ts = r#"
export interface Claims { sub: string; }

export class TokenManager {
    private secret: string;
    async verifyToken(raw: string): Promise<boolean> {
        if (raw.length === 0) { return false; }
        return true;
    }
}

export function createManager(secret: string): TokenManager {
    return new TokenManager();
}
"#;
        let ts_entities = extract_declarations("web/token.ts", ts);
        let ts_summary: Vec<(&str, &str, Option<&str>)> = ts_entities
            .iter()
            .map(|e| (e.kind.as_str(), e.name.as_str(), e.parent.as_deref()))
            .collect();
        assert_eq!(
            ts_summary,
            vec![
                ("interface", "Claims", Some("web/token.ts")),
                ("class", "TokenManager", Some("web/token.ts")),
                ("property", "secret", Some("TokenManager")),
                ("method", "verifyToken", Some("TokenManager")),
                ("function", "createManager", Some("web/token.ts")),
            ]
        );

        let py = r#"
class SessionGuard:
    """Docstring with def fake(): inside"""
    async def validate(self, token: str) -> bool:
        def local_helper():
            pass
        return bool(token)

def make_guard() -> SessionGuard:
    return SessionGuard()
"#;
        let py_entities = extract_declarations("auth/guard.py", py);
        let py_summary: Vec<(&str, &str, Option<&str>)> = py_entities
            .iter()
            .map(|e| (e.kind.as_str(), e.name.as_str(), e.parent.as_deref()))
            .collect();
        assert_eq!(
            py_summary,
            vec![
                ("class", "SessionGuard", Some("auth/guard.py")),
                ("method", "validate", Some("SessionGuard")),
                ("function", "make_guard", Some("auth/guard.py")),
            ]
        );
    }

    #[test]
    fn diff_new_entities_respects_baseline_and_ignores_line_shifts() {
        let before = "<?php\nclass Existing {\n    public function oldMethod(): void {}\n}\n";
        let after = "<?php\n// comment inserted at top shifting lines\n\nclass Existing {\n    public function oldMethod(): void {}\n    public function newMethod(): void {}\n}\nclass BrandNew {}\n";

        // Editing existing file without a captured baseline returns no false positives.
        assert!(diff_new_entities("src/Existing.php", None, after, false).is_empty());

        // With baseline, only newly added entities appear despite line-number shift.
        let added = diff_new_entities("src/Existing.php", Some(before), after, false);
        assert_eq!(added.len(), 2);
        assert_eq!(added[0].kind, "method");
        assert_eq!(added[0].name, "newMethod");
        assert_eq!(added[0].parent.as_deref(), Some("Existing"));
        assert_eq!(added[1].kind, "class");
        assert_eq!(added[1].name, "BrandNew");

        // New file creation emits file entity plus extracted symbols.
        let new_file_entities =
            diff_new_entities("src/BrandNew.php", None, "<?php\nclass BrandNew {}\n", true);
        assert_eq!(new_file_entities.len(), 2);
        assert_eq!(new_file_entities[0].kind, "file");
        assert_eq!(new_file_entities[0].path, "src/BrandNew.php");
        assert_eq!(new_file_entities[1].kind, "class");
        assert_eq!(new_file_entities[1].name, "BrandNew");
    }

    #[test]
    fn live_focus_tracker_and_store_persistence_never_store_file_bodies() {
        let state_dir = TempDir::new().expect("state");
        let project_dir = TempDir::new().expect("project");
        let project = project_dir.path();
        fs::create_dir_all(project.join("src/Auth")).expect("auth dir");
        fs::write(
            project.join("src/Auth/TokenService.php"),
            "<?php\nclass TokenService {\n    public function decode(): void {}\n}\n",
        )
        .expect("write existing");

        let store = SessionStore::open(state_dir.path()).expect("store");
        let reg = store.register(project).expect("register");
        let pid = reg["id"].as_str().expect("pid").to_owned();
        let session = store
            .create(
                &json!({"project_id": pid, "provider": "codex", "mode": "edit"}),
                "Focus test",
                &[],
            )
            .expect("session");
        let sid = session["id"].as_str().expect("sid").to_owned();

        let mut tracker = LiveFocusTracker::new(project.to_path_buf());
        let initial = predict_targets(
            project,
            None,
            "Edit src/Auth/TokenService.php to add refresh",
            &[],
            &[],
        );
        tracker.seed_focus(initial);
        assert!(tracker.take_delta_event().is_none());

        // Modify existing file cleanly after baseline was captured.
        assert!(tracker.record_mutation_start("src/Auth/TokenService.php"));
        fs::write(
            project.join("src/Auth/TokenService.php"),
            "<?php\nclass TokenService {\n    public function decode(): void {}\n    public function refresh(): string { return 'ok'; }\n}\n",
        )
        .expect("write updated");
        assert!(tracker.record_mutation_complete("src/Auth/TokenService.php", Some("update")));
        let delta = tracker.take_delta_event().expect("delta event");
        assert_eq!(delta["kind"], "graph_focus");
        assert_eq!(delta["phase"], "active");
        assert!(!delta.to_string().contains("return 'ok'"));
        assert_eq!(delta["created_entities"].as_array().map(Vec::len), Some(1));
        assert_eq!(delta["created_entities"][0]["kind"], "method");
        assert_eq!(delta["created_entities"][0]["name"], "refresh");
        assert_eq!(delta["created_entities"][0]["parent"], "TokenService");

        // Create a brand new file without pre-existing content on disk.
        assert!(tracker.record_mutation_start("src/Auth/JwtGuard.php"));
        fs::write(
            project.join("src/Auth/JwtGuard.php"),
            "<?php\nclass JwtGuard {\n    public function check(): bool { return true; }\n}\n",
        )
        .expect("write new file");
        assert!(tracker.record_mutation_complete("src/Auth/JwtGuard.php", Some("add")));
        assert!(tracker.complete());
        let done_event = tracker.take_delta_event().expect("completed event");
        assert_eq!(done_event["phase"], "completed");
        assert!(!done_event.to_string().contains("return true"));

        save_session_focus(&store, &sid, tracker.focus()).expect("save focus");
        let loaded_session_focus = session_focus_value(&store, &sid).expect("load session focus");
        assert_eq!(loaded_session_focus["phase"], "completed");
        assert_eq!(
            loaded_session_focus["created_entities"]
                .as_array()
                .map(Vec::len),
            Some(4)
        );

        let loaded_project_focus =
            project_focus_value(&store, &pid, None).expect("load project focus");
        assert_eq!(loaded_project_focus, loaded_session_focus);
        let wrong_project =
            project_focus_value(&store, "other-project", Some(&sid)).expect("wrong project");
        assert!(wrong_project.is_null());
    }

    #[test]
    fn created_entity_change_omitted_by_default_and_tagged_by_tracker() {
        let entity = CreatedEntity::new("function", "verify", None, "src/lib.rs", Some(12));
        assert_eq!(entity.change, None);
        let json = serde_json::to_value(&entity).expect("serialize entity");
        assert!(json.get("change").is_none());

        let standalone = diff_new_entities(
            "src/lib.rs",
            Some("pub fn alpha() {}\n"),
            "pub fn alpha() {}\npub fn beta() {}\n",
            false,
        );
        assert_eq!(standalone.len(), 1);
        assert_eq!(standalone[0].name, "beta");
        assert_eq!(standalone[0].change, None);

        let dir = TempDir::new().expect("temp dir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("src dir");
        let file = root.join("src/service.php");
        fs::write(
            &file,
            "<?php\nclass PaymentService {\n    public function charge(int $cents): string {\n        return 'ok';\n    }\n}\n",
        )
        .expect("write initial php");

        let mut tracker = LiveFocusTracker::new(root.to_path_buf());
        assert!(tracker.record_mutation_start("src/service.php"));

        // Modify string literal inside charge() body and add refund(), leaving class body untouched.
        fs::write(
            &file,
            "<?php\n// shifted down\nclass PaymentService {\n    public function charge(int $cents): string {\n        return 'err';\n    }\n    public function refund(int $cents): bool {\n        return true;\n    }\n}\n",
        )
        .expect("write updated php");
        assert!(tracker.record_mutation_complete("src/service.php", Some("edit")));

        let created = &tracker.focus().created_entities;
        let refund = created
            .iter()
            .find(|e| e.name == "refund")
            .expect("refund created");
        assert_eq!(refund.kind, "method");
        assert_eq!(refund.parent.as_deref(), Some("PaymentService"));
        assert_eq!(refund.change.as_deref(), Some("created"));
        assert!(refund.id.starts_with("live:method:"));

        let charge = created
            .iter()
            .find(|e| e.name == "charge")
            .expect("charge modified");
        assert_eq!(charge.kind, "method");
        assert_eq!(charge.parent.as_deref(), Some("PaymentService"));
        assert_eq!(charge.change.as_deref(), Some("modified"));
        assert!(charge.id.starts_with("live:mod:method:"));

        // Enclosing class must not be marked modified just because a child method changed/added.
        assert!(
            !created
                .iter()
                .any(|e| e.kind == "class" && e.name == "PaymentService"),
            "enclosing class should not be emitted as modified when only child methods change: {created:?}"
        );

        // Editing charge again deduplicates cleanly without emitting a second start delta.
        assert!(!tracker.record_mutation_start("src/service.php"));
        fs::write(
            &file,
            "<?php\nclass PaymentService {\n    public function charge(int $cents): string {\n        return 'retry';\n    }\n    public function refund(int $cents): bool {\n        return true;\n    }\n}\n",
        )
        .expect("write second php edit");
        assert!(!tracker.record_mutation_complete("src/service.php", Some("edit")));
        assert_eq!(
            tracker
                .focus()
                .created_entities
                .iter()
                .filter(|e| e.name == "charge")
                .count(),
            1
        );
    }

    #[test]
    fn detects_modified_declarations_across_rust_ts_python_without_comment_noise() {
        // Line shift + comment-only change should not mark symbols modified.
        let rust_before = "\
pub fn unchanged(buf: [u8; 32]) -> usize {
    buf.len()
}

pub fn hash_key(buf: [u8; 32]) -> u8 {
    buf[0]
}
";
        let rust_shifted_comment_only = "\
// header comment shifts lines
pub fn unchanged(buf: [u8; 32]) -> usize {
    /* inline comment */
    buf.len()
}

pub fn hash_key(buf: [u8; 32]) -> u8 {
    buf[1]
}
";
        let rust_mods =
            diff_modified_entities("src/crypto.rs", rust_before, rust_shifted_comment_only);
        assert_eq!(
            rust_mods.len(),
            1,
            "expected only hash_key modified: {rust_mods:?}"
        );
        assert_eq!(rust_mods[0].name, "hash_key");
        assert_eq!(rust_mods[0].change.as_deref(), Some("modified"));
        assert!(rust_mods[0].id.starts_with("live:mod:function:"));

        // TypeScript inline object-type parameter + arrow function signature/body edits.
        let ts_before = "\
export const renderCard = (opts: { title: string; count: number }): string => {
    return `${opts.title}:${opts.count}`;
};

export function keepStable(opts: { ok: boolean }): boolean {
    return opts.ok;
}
";
        let ts_after = "\
export const renderCard = (opts: { title: string; count: number; badge: string }): string => {
    return `${opts.title}:${opts.count}`;
};

export function keepStable(opts: { ok: boolean }): boolean {
    // comment only
    return opts.ok;
}
";
        let ts_mods = diff_modified_entities("web/card.ts", ts_before, ts_after);
        assert_eq!(
            ts_mods.len(),
            1,
            "expected only renderCard modified: {ts_mods:?}"
        );
        assert_eq!(ts_mods[0].name, "renderCard");
        assert_eq!(ts_mods[0].change.as_deref(), Some("modified"));

        // Python multi-line signature & indented body edits.
        let py_before = "\
class Runner:
    def execute(
        self,
        task_id: str,
    ) -> int:
        return 1

    def untouched(self) -> int:
        return 0
";
        let py_after = "\
class Runner:
    def execute(
        self,
        task_id: str,
    ) -> int:
        return 2

    def untouched(self) -> int:
        return 0
";
        let py_mods = diff_modified_entities("app/runner.py", py_before, py_after);
        assert_eq!(
            py_mods.len(),
            1,
            "expected only execute modified: {py_mods:?}"
        );
        assert_eq!(py_mods[0].kind, "method");
        assert_eq!(py_mods[0].name, "execute");
        assert_eq!(py_mods[0].parent.as_deref(), Some("Runner"));
        assert_eq!(py_mods[0].change.as_deref(), Some("modified"));
    }

    #[test]
    fn extracts_php_properties_promoted_params_consts_and_84_visibility() {
        let php = r#"<?php
const GLOBAL_VERSION = 2;

final class MailService {
    public const DEFAULT_TTL = 300;
    private readonly ?string $fallback;
    public private(set) int $count = 0;
    public protected(set) string $mode = 'fast';
    static function boot(): void {}

    public function __construct(
        private readonly Mailer $mailer,
        protected int $ttl = 60,
        string $plainArg,
    ) {}
}
"#;
        let spans = extract_declaration_spans("src/Mail/MailService.php", php);
        let find = |kind: &str, name: &str| {
            spans
                .iter()
                .find(|d| d.entity.kind == kind && d.entity.name == name)
                .unwrap_or_else(|| panic!("missing {kind} {name}: {spans:?}"))
        };

        let global_c = find("const", "GLOBAL_VERSION");
        assert_eq!(
            global_c.entity.parent.as_deref(),
            Some("src/Mail/MailService.php")
        );
        assert_eq!(global_c.visibility.as_deref(), Some("public"));

        let cls = find("class", "MailService");
        assert_eq!(cls.modifiers, vec!["final"]);

        let class_c = find("const", "DEFAULT_TTL");
        assert_eq!(class_c.entity.parent.as_deref(), Some("MailService"));
        assert_eq!(class_c.visibility.as_deref(), Some("public"));
        assert!(class_c.entity.id.contains("MailService::DEFAULT_TTL"));

        let fallback = find("property", "$fallback");
        assert_eq!(fallback.visibility.as_deref(), Some("private"));
        assert_eq!(fallback.modifiers, vec!["readonly"]);
        assert_eq!(fallback.params_signature, "readonly ?string $fallback");

        let count = find("property", "$count");
        assert_eq!(count.visibility.as_deref(), Some("private(set)"));
        assert_eq!(count.params_signature, "int $count");

        let mode = find("property", "$mode");
        assert_eq!(mode.visibility.as_deref(), Some("protected(set)"));

        let boot = find("method", "boot");
        assert_eq!(boot.visibility.as_deref(), Some("public"));
        assert_eq!(boot.modifiers, vec!["static"]);

        let mailer = find("property", "$mailer");
        assert_eq!(mailer.entity.parent.as_deref(), Some("MailService"));
        assert_eq!(mailer.visibility.as_deref(), Some("private"));
        assert_eq!(mailer.modifiers, vec!["readonly"]);
        assert_eq!(mailer.entity.line, Some(12));

        let ttl = find("property", "$ttl");
        assert_eq!(ttl.visibility.as_deref(), Some("protected"));
        assert_eq!(ttl.entity.line, Some(13));

        assert!(
            spans.iter().all(|d| d.entity.name != "$plainArg"),
            "non-promoted constructor parameter must not emit a property"
        );
    }

    #[test]
    fn extracts_rust_fields_consts_statics_and_visibility_modifiers() {
        let rs = r#"
pub(in crate::auth) const TOKEN_VERSION: u8 = 1;
pub static mut ACTIVE_COUNT: usize = 0;

pub struct TokenConfig {
    pub(crate) ttl: u64,
    pub(super) retries: u8,
    signing_key: String,
}

pub trait Signer {
    const ALG: &'static str;
    fn sign(&self, msg: &[u8]) -> Vec<u8>;
}

impl Signer for TokenConfig {
    const ALG: &'static str = "HS256";
    fn sign(&self, msg: &[u8]) -> Vec<u8> {
        msg.to_vec()
    }
}

impl TokenConfig {
    pub async unsafe fn refresh(&self) {}
    fn helper(&self) {}
}
"#;
        let spans = extract_declaration_spans("src/config.rs", rs);
        let find = |kind: &str, name: &str| {
            spans
                .iter()
                .find(|d| d.entity.kind == kind && d.entity.name == name)
                .unwrap_or_else(|| panic!("missing {kind} {name}: {spans:?}"))
        };

        let ver = find("const", "TOKEN_VERSION");
        assert_eq!(ver.visibility.as_deref(), Some("pub(in crate::auth)"));
        assert_eq!(ver.params_signature, "const TOKEN_VERSION: u8");

        let stat = find("const", "ACTIVE_COUNT");
        assert_eq!(stat.visibility.as_deref(), Some("public"));
        assert_eq!(stat.modifiers, vec!["static", "mut"]);

        let ttl = find("field", "ttl");
        assert_eq!(ttl.entity.parent.as_deref(), Some("TokenConfig"));
        assert_eq!(ttl.visibility.as_deref(), Some("pub(crate)"));
        assert_eq!(ttl.params_signature, "ttl: u64");

        let retries = find("field", "retries");
        assert_eq!(retries.visibility.as_deref(), Some("pub(super)"));

        let signing = find("field", "signing_key");
        assert_eq!(signing.visibility.as_deref(), Some("private"));

        let trait_sign = spans
            .iter()
            .find(|d| d.entity.name == "sign" && d.entity.parent.as_deref() == Some("Signer"))
            .expect("trait sign");
        assert_eq!(trait_sign.visibility.as_deref(), Some("public"));

        let impl_sign = spans
            .iter()
            .find(|d| d.entity.name == "sign" && d.entity.parent.as_deref() == Some("TokenConfig"))
            .expect("impl Trait sign");
        assert_eq!(impl_sign.visibility.as_deref(), Some("public"));

        let impl_alg = spans
            .iter()
            .find(|d| d.entity.name == "ALG" && d.entity.parent.as_deref() == Some("TokenConfig"))
            .expect("impl Trait const ALG");
        assert_eq!(impl_alg.visibility.as_deref(), Some("public"));

        let refresh = find("method", "refresh");
        assert_eq!(refresh.visibility.as_deref(), Some("public"));
        assert_eq!(refresh.modifiers, vec!["async", "unsafe"]);

        let helper = find("method", "helper");
        assert_eq!(helper.visibility.as_deref(), Some("private"));
    }

    #[test]
    fn extracts_ts_fields_private_hash_ctor_props_and_consts_and_python_props() {
        let ts = r#"
export const DEFAULT_TTL = 60
const INTERNAL_TAG = 'v1'

export class SessionStore {
    #secret: string
    static readonly version: number = 2
    onExpire = (id: string) => id.trim()

    constructor(
        private readonly signer: Signer,
        public ttl: number = 30,
        plain: string,
    ) {}

    #digest(raw: string): string {
        return raw;
    }
}
"#;
        let ts_spans = extract_declaration_spans("web/session.ts", ts);
        let find_ts = |kind: &str, name: &str| {
            ts_spans
                .iter()
                .find(|d| d.entity.kind == kind && d.entity.name == name)
                .unwrap_or_else(|| panic!("missing ts {kind} {name}: {ts_spans:?}"))
        };

        assert_eq!(
            find_ts("const", "DEFAULT_TTL").visibility.as_deref(),
            Some("export")
        );
        assert_eq!(
            find_ts("const", "INTERNAL_TAG").visibility.as_deref(),
            Some("private")
        );

        let hash_prop = find_ts("property", "#secret");
        assert_eq!(hash_prop.visibility.as_deref(), Some("#private"));
        assert_eq!(hash_prop.entity.parent.as_deref(), Some("SessionStore"));

        let ver = find_ts("property", "version");
        assert_eq!(ver.visibility.as_deref(), Some("public"));
        assert_eq!(ver.modifiers, vec!["static", "readonly"]);

        let on_expire = find_ts("property", "onExpire");
        assert_eq!(on_expire.entity.parent.as_deref(), Some("SessionStore"));

        let signer = find_ts("property", "signer");
        assert_eq!(signer.visibility.as_deref(), Some("private"));
        assert_eq!(signer.modifiers, vec!["readonly"]);

        let ts_ttl = find_ts("property", "ttl");
        assert_eq!(ts_ttl.visibility.as_deref(), Some("public"));

        let hash_method = find_ts("method", "#digest");
        assert_eq!(hash_method.visibility.as_deref(), Some("#private"));

        let py = r#"
MAX_RETRIES = 3
_INTERNAL_MODE = "strict"

class Guard:
    timeout: int = 30
    _cache: dict

    def __init__(self, key: str) -> None:
        self.__key = key
        self.timeout = 30

    @property
    def label(self) -> str:
        return "ok"

    @classmethod
    def create(cls) -> "Guard":
        return cls("x")

    @staticmethod
    @abstractmethod
    @final
    def _helper() -> None:
        pass
"#;
        let py_spans = extract_declaration_spans("auth/guard.py", py);
        let find_py = |kind: &str, name: &str| {
            py_spans
                .iter()
                .find(|d| d.entity.kind == kind && d.entity.name == name)
                .unwrap_or_else(|| panic!("missing py {kind} {name}: {py_spans:?}"))
        };

        assert_eq!(
            find_py("const", "MAX_RETRIES").visibility.as_deref(),
            Some("public")
        );
        assert_eq!(
            find_py("const", "_INTERNAL_MODE").visibility.as_deref(),
            Some("protected")
        );
        assert_eq!(
            py_spans
                .iter()
                .filter(|d| d.entity.kind == "property" && d.entity.name == "timeout")
                .count(),
            1,
            "annotated class attribute and __init__ self.timeout must deduplicate"
        );
        assert_eq!(
            find_py("property", "_cache").visibility.as_deref(),
            Some("protected")
        );
        assert_eq!(
            find_py("property", "__key").visibility.as_deref(),
            Some("private")
        );
        assert_eq!(
            find_py("method", "__init__").visibility.as_deref(),
            Some("public")
        );

        let label = find_py("property", "label");
        assert_eq!(label.modifiers, vec!["property"]);

        let create = find_py("method", "create");
        assert_eq!(create.modifiers, vec!["classmethod"]);

        let helper = find_py("method", "_helper");
        assert_eq!(helper.visibility.as_deref(), Some("protected"));
        assert_eq!(
            helper.modifiers,
            vec!["staticmethod", "abstractmethod", "final"]
        );
    }

    #[test]
    fn params_signature_remains_stable_when_only_visibility_changes() {
        let php_a = extract_declaration_spans(
            "src/Svc.php",
            "<?php\nclass Svc {\n    private readonly string $key;\n    private function run(int $x): void {}\n}\n",
        );
        let php_b = extract_declaration_spans(
            "src/Svc.php",
            "<?php\nclass Svc {\n    public readonly string $key;\n    public function run(int $x): void {}\n}\n",
        );
        assert_eq!(php_a[1].params_signature, php_b[1].params_signature);
        assert_ne!(php_a[1].visibility, php_b[1].visibility);
        assert_eq!(php_a[2].params_signature, php_b[2].params_signature);
    }

    #[test]
    fn predict_session_targets_extracts_omnigraph_atom_files_and_capsule_paths() {
        let dir = TempDir::new().expect("temp dir");
        let root = dir.path();
        fs::create_dir_all(root.join("src/Aaa")).expect("aaa dir");
        for i in 0..20 {
            fs::write(
                root.join(format!("src/Aaa/Pad{i:02}.rs")),
                format!("pub fn pad_{i}() {{}}\n"),
            )
            .expect("pad file");
        }
        fs::create_dir_all(root.join("src/Auth")).expect("auth dir");
        fs::create_dir_all(root.join("src/Cache")).expect("cache dir");
        fs::write(
            root.join("src/Auth/TokenGuard.rs"),
            "pub fn check_token() {}\n",
        )
        .expect("token guard");
        fs::write(
            root.join("src/Cache/RedisStore.rs"),
            "pub fn cache_key() {}\n",
        )
        .expect("redis store");

        let store = SessionStore::open(&root.join("harness-state")).expect("store");
        let enabled_session = json!({
            "omnigraph": {
                "enabled": true,
                "injected_atoms": [
                    {
                        "atom_id": "a1",
                        "files": ["src/Cache/RedisStore.rs"]
                    }
                ],
                "capsule_text": "- [finding] Check TokenGuard in src/Auth/TokenGuard.rs:42\n"
            }
        });
        let focus = predict_session_targets(
            &store,
            "sess-1",
            &enabled_session,
            root,
            None,
            "Review session",
        );
        assert!(
            focus
                .target_paths
                .contains(&"src/Cache/RedisStore.rs".to_owned()),
            "{:?}",
            focus.target_paths
        );
        assert!(
            focus
                .target_paths
                .contains(&"src/Auth/TokenGuard.rs".to_owned()),
            "{:?}",
            focus.target_paths
        );

        let mut disabled_session = enabled_session;
        disabled_session["omnigraph"]["enabled"] = json!(false);
        let disabled_focus = predict_session_targets(
            &store,
            "sess-2",
            &disabled_session,
            root,
            None,
            "Review session",
        );
        assert!(
            !disabled_focus
                .target_paths
                .contains(&"src/Cache/RedisStore.rs".to_owned()),
            "{:?}",
            disabled_focus.target_paths
        );
        assert!(
            !disabled_focus
                .target_paths
                .contains(&"src/Auth/TokenGuard.rs".to_owned()),
            "{:?}",
            disabled_focus.target_paths
        );
    }
}
