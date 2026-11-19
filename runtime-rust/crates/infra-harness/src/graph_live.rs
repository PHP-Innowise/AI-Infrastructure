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
    "go", "java", "kt", "kts", "cs", "rb", "swift", "scala", "sc", "json", "toml", "md", "css",
    "html", "sql", "yml", "yaml",
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
    pub(crate) fn new(
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// Extracts top-level and class/impl declarations with signatures and body fingerprints via AST.
pub(crate) fn extract_declaration_spans(rel_path: &str, source: &str) -> Vec<ExtractedDecl> {
    let rel = clean_rel_key(rel_path);
    if rel.is_empty() {
        return Vec::new();
    }
    crate::graph_ast::extract_declarations_for_live(&rel, source)
}

/// Extracts top-level and class/impl declarations from a source file without executing code.
pub fn extract_declarations(rel_path: &str, source: &str) -> Vec<CreatedEntity> {
    extract_declaration_spans(rel_path, source)
        .into_iter()
        .map(|decl| decl.entity)
        .collect()
}

/// The declarations of `source` (the file `rel_path`) that `text` names, in
/// file order, at most `limit`. A name with a capital, an underscore or a
/// digit in it counts wherever it stands as a whole word; a plain lowercase
/// one (`check`, `render`) only where the text writes it as code: in
/// backticks, called, or after `::`, `->` or `.`. Matching is case-sensitive.
pub(crate) fn declarations_named_in(
    rel_path: &str,
    source: &str,
    text: &str,
    limit: usize,
) -> Vec<CreatedEntity> {
    if limit == 0 || text.trim().is_empty() {
        return Vec::new();
    }
    let mut out: Vec<CreatedEntity> = Vec::new();
    for decl in extract_declaration_spans(rel_path, source) {
        let entity = decl.entity;
        if entity.kind == "file"
            || entity.name.chars().count() < 3
            || !names_declaration(text, &entity.name)
        {
            continue;
        }
        if out
            .iter()
            .any(|seen| seen.name == entity.name && seen.parent == entity.parent)
        {
            continue;
        }
        out.push(entity);
        if out.len() >= limit {
            break;
        }
    }
    out
}

pub(crate) fn names_declaration(text: &str, name: &str) -> bool {
    let distinctive = name.contains('_')
        || name.chars().any(|c| c.is_ascii_digit())
        || (name.chars().any(|c| c.is_ascii_uppercase())
            && name.chars().any(|c| c.is_ascii_lowercase()));
    let mut from = 0;
    while let Some(at) = text[from..].find(name) {
        let start = from + at;
        let end = start + name.len();
        let head = &text[..start];
        let tail = &text[end..];
        let before = head.chars().next_back();
        let after = tail.chars().next();
        if !before.is_some_and(is_ident_char)
            && !after.is_some_and(is_ident_char)
            && (distinctive
                || (before == Some('`') && after == Some('`'))
                || after == Some('(')
                || head.ends_with("::")
                || head.ends_with("->")
                || before == Some('.'))
        {
            return true;
        }
        from = end;
    }
    false
}

/// A project file read for its declarations: inside `root` once links are
/// followed, a regular file, at most the live scan's size, UTF-8. `rel_path`
/// may carry a `#L12` anchor.
pub(crate) fn read_workspace_file(root: &Path, rel_path: &str) -> Option<(String, String)> {
    let rel = clean_rel_key(rel_path.split('#').next().unwrap_or(rel_path));
    if rel.is_empty() || root.as_os_str().is_empty() {
        return None;
    }
    let real_root = fs::canonicalize(root).ok()?;
    let real = fs::canonicalize(root.join(&rel)).ok()?;
    if !real.starts_with(&real_root) {
        return None;
    }
    read_bounded_utf8(&real).map(|source| (rel, source))
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Language {
    Php,
    Rust,
    TsJs,
    Python,
    Go,
    Java,
    CSharp,
    Ruby,
    Kotlin,
    Swift,
    Scala,
}

pub(crate) fn detect_language(rel_path: &str) -> Option<Language> {
    crate::graph_ast::AstGrammar::from_rel_path(rel_path)
        .map(crate::graph_ast::AstGrammar::to_live_language)
}

pub(crate) fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
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

    #[test]
    fn detects_and_extracts_declarations_across_all_fourteen_plus_languages() {
        for (ext, expected) in [
            ("a.php", Language::Php),
            ("a.py", Language::Python),
            ("a.rb", Language::Ruby),
            ("a.java", Language::Java),
            ("a.kt", Language::Kotlin),
            ("a.cs", Language::CSharp),
            ("a.ts", Language::TsJs),
            ("a.tsx", Language::TsJs),
            ("a.js", Language::TsJs),
            ("a.jsx", Language::TsJs),
            ("a.go", Language::Go),
            ("a.rs", Language::Rust),
            ("a.swift", Language::Swift),
            ("a.scala", Language::Scala),
        ] {
            assert_eq!(detect_language(ext), Some(expected), "ext={ext}");
        }

        let go_before = "package auth\ntype Guard struct { Secret string }\nfunc (g *Guard) Check() bool { return true }\n";
        let go_after = "package auth\ntype Guard struct { Secret string }\nfunc (g *Guard) Check() bool { return false }\n";
        let go_spans = extract_declaration_spans("auth/guard.go", go_before);
        assert!(
            go_spans
                .iter()
                .any(|d| d.entity.kind == "struct" && d.entity.name == "Guard")
        );
        assert!(go_spans.iter().any(|d| d.entity.kind == "method"
            && d.entity.name == "Check"
            && d.entity.parent.as_deref() == Some("Guard")));
        let go_mods = diff_modified_entities("auth/guard.go", go_before, go_after);
        assert_eq!(go_mods.len(), 1);
        assert_eq!(go_mods[0].name, "Check");

        let java_spans = extract_declaration_spans(
            "src/Guard.java",
            "public class Guard { private int ttl; public boolean ok() { return true; } }\n",
        );
        assert!(
            java_spans
                .iter()
                .any(|d| d.entity.kind == "class" && d.entity.name == "Guard")
        );
        assert!(java_spans.iter().any(|d| d.entity.kind == "field"
            && d.entity.name == "ttl"
            && d.visibility.as_deref() == Some("private")));
        assert!(java_spans.iter().any(|d| d.entity.kind == "method"
            && d.entity.name == "ok"
            && d.visibility.as_deref() == Some("public")));
    }
}
