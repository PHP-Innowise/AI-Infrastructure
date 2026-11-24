use super::context_sync::{contradiction_caution, query_edges_safe};
use super::{MAX_ATOM_SYMBOLS, MAX_SYMBOL_FILES, VALID_RELATIONS};
use crate::omnigraph::{
    client::{ChangeOp, ChangeRequest, CommitOutput, EntityKind, OmnigraphClient},
    schema::{
        CANONICAL_KNOWLEDGE_BRANCH, GQ_CLEAR_ATOM_FILES, GQ_CLEAR_ATOM_SYMBOLS,
        GQ_LINK_MENTIONS_SYMBOL, GQ_LINK_TOUCHES_FILE, GQ_LIST_SYMBOL_EDGES, GQ_UPSERT_FILE,
        GQ_UPSERT_SYMBOL,
    },
    transfer::parse_i64_opt,
};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub(super) fn copy_alias(obj: &mut Map<String, Value>, a: &str, b: &str) {
    let a_val = obj.get(a).filter(|v| !v.is_null()).cloned();
    let b_val = obj.get(b).filter(|v| !v.is_null()).cloned();
    match (a_val, b_val) {
        (Some(v), None) => {
            obj.insert(b.to_string(), v);
        }
        (None, Some(v)) => {
            obj.insert(a.to_string(), v);
        }
        (None, None) => {
            if obj.contains_key(a) && !obj.contains_key(b) {
                obj.insert(b.to_string(), Value::Null);
            } else if obj.contains_key(b) && !obj.contains_key(a) {
                obj.insert(a.to_string(), Value::Null);
            }
        }
        (Some(_), Some(_)) => {}
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct AtomProvenance {
    pub(super) origin_layer: Option<String>,
    pub(super) authority: Option<String>,
    pub(super) privacy: Option<String>,
    pub(super) brain_record_id: Option<String>,
    pub(super) memory_id: Option<String>,
    pub(super) record_revision: Option<i64>,
}

impl AtomProvenance {
    pub(super) fn from_value(v: &Value) -> Self {
        Self {
            origin_layer: str_field(v, "origin_layer"),
            authority: str_field(v, "authority"),
            privacy: str_field(v, "privacy"),
            brain_record_id: str_field(v, "brain_record_id"),
            memory_id: str_field(v, "memory_id"),
            record_revision: v.get("record_revision").and_then(parse_i64_opt),
        }
    }

    pub(super) fn apply_to_map(&self, obj: &mut Map<String, Value>) {
        let str_or_null = |opt: &Option<String>| {
            opt.as_ref()
                .map(|s| Value::String(s.clone()))
                .unwrap_or(Value::Null)
        };
        obj.insert("origin_layer".into(), str_or_null(&self.origin_layer));
        obj.insert("authority".into(), str_or_null(&self.authority));
        obj.insert("privacy".into(), str_or_null(&self.privacy));
        obj.insert("brain_record_id".into(), str_or_null(&self.brain_record_id));
        obj.insert("memory_id".into(), str_or_null(&self.memory_id));
        obj.insert(
            "record_revision".into(),
            self.record_revision
                .map(|n| Value::Number(n.into()))
                .unwrap_or(Value::Null),
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_upsert_atom_params(
    atom_id: &str,
    project_id: &str,
    kind: &str,
    title: &str,
    summary: &str,
    body: &str,
    tags: Option<Vec<String>>,
    source_branch: &str,
    source_session_id: Option<&str>,
    confidence: f64,
    pin_priority: i32,
    verified: bool,
    provenance: &AtomProvenance,
    created_at: &str,
    updated_at: &str,
) -> Value {
    let mut val = json!({
        "atom_id": atom_id,
        "project_id": project_id,
        "kind": kind,
        "title": title,
        "summary": summary,
        "body": body,
        "tags": tags,
        "source_branch": source_branch,
        "source_session_id": source_session_id,
        "confidence": confidence,
        "pin_priority": pin_priority,
        "verified": verified,
        "created_at": created_at,
        "updated_at": updated_at,
    });
    if let Some(obj) = val.as_object_mut() {
        provenance.apply_to_map(obj);
    }
    val
}

pub(super) fn parse_string_list(val: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for item in val.as_array().into_iter().flatten() {
        if let Some(s) = item.as_str().map(str::trim).filter(|s| !s.is_empty())
            && !out.iter().any(|existing| existing == s)
        {
            out.push(s.to_string());
        }
    }
    out
}

pub(super) fn parse_tags_opt(val: &Value) -> Option<Vec<String>> {
    val.as_array().map(|arr| {
        arr.iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    })
}

pub(super) async fn clear_atom_files(
    client: &OmnigraphClient,
    branch: &str,
    atom_id: &str,
) -> bool {
    client
        .mutate(&ChangeRequest {
            query: GQ_CLEAR_ATOM_FILES.into(),
            name: Some("clear_atom_files".into()),
            params: Some(json!({ "atom_id": atom_id })),
            branch: Some(branch.to_string()),
            settings: None,
        })
        .await
        .is_ok()
}

pub(super) async fn upsert_atom_files(
    client: &OmnigraphClient,
    project_id: &str,
    branch: &str,
    atom_id: &str,
    files: &[String],
    ts: &str,
) -> Option<CommitOutput> {
    let mut last_commit = None;
    for file_path in files {
        let lang = detect_language(file_path);
        let _ = client
            .mutate(&ChangeRequest {
                query: GQ_UPSERT_FILE.into(),
                name: Some("upsert_file".into()),
                params: Some(json!({
                    "path": file_path,
                    "project_id": project_id,
                    "language": lang,
                    "last_touched_at": ts,
                })),
                branch: Some(branch.to_string()),
                settings: None,
            })
            .await;
        if let Ok(r) = client
            .mutate(&ChangeRequest {
                query: GQ_LINK_TOUCHES_FILE.into(),
                name: Some("link_touches_file".into()),
                params: Some(json!({
                    "atom_id": atom_id,
                    "path": file_path,
                    "access": "referenced",
                    "line_start": Value::Null,
                    "line_end": Value::Null,
                })),
                branch: Some(branch.to_string()),
                settings: None,
            })
            .await
            && r.commit.is_some()
        {
            last_commit = r.commit;
        }
    }
    last_commit
}

/// The declarations `text` names in the files an atom touches, read from the
/// fresh `GraphIrStore` when available (canonical `symbol_id`), falling back to
/// `graph_live::declarations_named_in` on `root`.
pub(super) fn atom_symbols_for_project(
    state_dir: Option<&Path>,
    project_id: &str,
    root: Option<&Path>,
    files: &[String],
    text: &str,
) -> Vec<Value> {
    let Some(root) = root else {
        return Vec::new();
    };
    let ir_store = state_dir
        .filter(|_| !project_id.is_empty())
        .and_then(|state| {
            crate::graph_ir::GraphIrStore::open_readonly_cached(state, project_id)
                .ok()
                .flatten()
        });
    let mut symbols = Vec::new();
    let format_fqn = |rel: &str, owner: Option<&str>, name: &str| -> String {
        match owner.filter(|o| !o.is_empty() && *o != rel) {
            Some(owner) => format!("{rel}::{owner}::{name}"),
            None => format!("{rel}::{name}"),
        }
    };
    for file in files.iter().take(MAX_SYMBOL_FILES) {
        let rel_key = crate::graph_live::clean_rel_key(file.split('#').next().unwrap_or(file));
        if !rel_key.is_empty()
            && let Some(ref ir) = ir_store
            && let Ok(Some(ir_syms)) = ir.fresh_file_symbols(root, &rel_key)
        {
            let mut seen_names: BTreeSet<(Option<String>, String)> = BTreeSet::new();
            for (symbol_id, kind, name, parent_name, start_line) in ir_syms {
                if symbols.len() >= MAX_ATOM_SYMBOLS {
                    break;
                }
                if kind == "file"
                    || name.chars().count() < 3
                    || !crate::graph_live::names_declaration(text, &name)
                    || !seen_names.insert((parent_name.clone(), name.clone()))
                {
                    continue;
                }
                let fqn = format_fqn(&rel_key, parent_name.as_deref(), &name);
                symbols.push(json!({
                    "fqn": fqn,
                    "symbol_id": symbol_id,
                    "file_path": rel_key,
                    "kind": kind,
                    "name": name,
                    "line": start_line,
                }));
            }
            if symbols.len() >= MAX_ATOM_SYMBOLS {
                break;
            }
            continue;
        }
        let Some((rel, source)) = crate::graph_live::read_workspace_file(root, file) else {
            continue;
        };
        let room = MAX_ATOM_SYMBOLS - symbols.len();
        for entity in crate::graph_live::declarations_named_in(&rel, &source, text, room) {
            let fqn = format_fqn(&rel, entity.parent.as_deref(), &entity.name);
            let mut row = json!({
                "fqn": fqn,
                "file_path": rel,
                "kind": entity.kind,
                "name": entity.name,
            });
            if !entity.id.is_empty() {
                row["symbol_id"] = json!(entity.id);
            }
            if let Some(line) = entity.line {
                row["line"] = json!(line);
            }
            symbols.push(row);
        }
        if symbols.len() >= MAX_ATOM_SYMBOLS {
            break;
        }
    }
    symbols
}

#[cfg(test)]
#[allow(dead_code)]
pub(super) fn atom_symbols(root: Option<&Path>, files: &[String], text: &str) -> Vec<Value> {
    atom_symbols_for_project(None, "", root, files, text)
}

/// Links `atom_id` to `symbols` on `branch`. The mentions it had
/// (`previous`) are replaced when the set differs; the same set writes
/// nothing.
pub(super) async fn upsert_atom_symbols(
    client: &OmnigraphClient,
    project_id: &str,
    branch: &str,
    atom_id: &str,
    symbols: &[Value],
    previous: &BTreeSet<String>,
) -> Option<CommitOutput> {
    let wanted = symbols
        .iter()
        .filter_map(|symbol| str_field(symbol, "fqn"))
        .collect::<BTreeSet<_>>();
    if wanted == *previous {
        return None;
    }
    let mut last_commit = None;
    if !previous.is_empty()
        && let Ok(r) = client
            .mutate(&ChangeRequest {
                query: GQ_CLEAR_ATOM_SYMBOLS.into(),
                name: Some("clear_atom_symbols".into()),
                params: Some(json!({ "atom_id": atom_id })),
                branch: Some(branch.to_string()),
                settings: None,
            })
            .await
        && r.commit.is_some()
    {
        last_commit = r.commit;
    }
    for symbol in symbols {
        let Some(fqn) = str_field(symbol, "fqn") else {
            continue;
        };
        let _ = client
            .mutate(&ChangeRequest {
                query: GQ_UPSERT_SYMBOL.into(),
                name: Some("upsert_symbol".into()),
                params: Some(json!({
                    "fqn": fqn,
                    "project_id": project_id,
                    "file_path": symbol["file_path"],
                    "kind": symbol["kind"],
                    "name": symbol["name"],
                })),
                branch: Some(branch.to_string()),
                settings: None,
            })
            .await;
        if let Ok(r) = client
            .mutate(&ChangeRequest {
                query: GQ_LINK_MENTIONS_SYMBOL.into(),
                name: Some("link_mentions_symbol".into()),
                params: Some(json!({
                    "atom_id": atom_id,
                    "fqn": fqn,
                    "role": "named",
                })),
                branch: Some(branch.to_string()),
                settings: None,
            })
            .await
            && r.commit.is_some()
        {
            last_commit = r.commit;
        }
    }
    last_commit
}

/// The `MentionsSymbol` rows of the project's atoms on `branch`.
pub(super) async fn symbol_edges(
    client: &OmnigraphClient,
    project_id: &str,
    branch: &str,
) -> Vec<Value> {
    query_edges_safe(
        client,
        GQ_LIST_SYMBOL_EDGES,
        "list_symbol_edges",
        &json!({ "project_id": project_id }),
        Some(branch),
        None,
    )
    .await
}

/// The declarations each of the project's atoms names on `branch`.
pub(super) async fn list_atom_symbols(
    client: &OmnigraphClient,
    project_id: &str,
    branch: &str,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut named: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for edge in symbol_edges(client, project_id, branch).await {
        if let (Some(atom_id), Some(fqn)) =
            (str_field(&edge, "source_id"), str_field(&edge, "target_id"))
        {
            named.entry(atom_id).or_default().insert(fqn);
        }
    }
    named
}

pub(super) fn normalize_atom_row(row: &Value) -> Value {
    let mut obj = match row.as_object() {
        Some(m) => m.clone(),
        None => Map::new(),
    };
    copy_alias(&mut obj, "atom_id", "id");
    copy_alias(&mut obj, "source_branch", "branch");
    copy_alias(&mut obj, "source_session_id", "session_id");
    let prov = AtomProvenance::from_value(&Value::Object(obj.clone()));
    prov.apply_to_map(&mut obj);
    Value::Object(obj)
}

pub(super) fn normalize_session_row(row: &Value) -> Value {
    let mut obj = match row.as_object() {
        Some(m) => m.clone(),
        None => Map::new(),
    };
    if let Some(id) = obj
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string)
    {
        obj.entry("id").or_insert_with(|| Value::String(id));
    }
    Value::Object(obj)
}

pub(super) fn normalize_file_row(row: &Value) -> Value {
    let mut obj = match row.as_object() {
        Some(m) => m.clone(),
        None => Map::new(),
    };
    if let Some(path) = obj.get("path").and_then(Value::as_str).map(str::to_string) {
        obj.entry("id").or_insert_with(|| Value::String(path));
    }
    Value::Object(obj)
}

pub(super) fn normalize_symbol_row(row: &Value) -> Value {
    let mut obj = match row.as_object() {
        Some(m) => m.clone(),
        None => Map::new(),
    };
    if let Some(fqn) = obj.get("fqn").and_then(Value::as_str).map(str::to_string) {
        obj.entry("id").or_insert_with(|| Value::String(fqn));
    }
    Value::Object(obj)
}

pub(super) fn normalize_turn_row(row: &Value) -> Value {
    let mut obj = match row.as_object() {
        Some(m) => m.clone(),
        None => Map::new(),
    };
    if let Some(snap_id) = obj
        .get("snapshot_id")
        .and_then(Value::as_str)
        .map(str::to_string)
    {
        obj.entry("id").or_insert_with(|| Value::String(snap_id));
    }
    Value::Object(obj)
}

pub(super) fn change_to_atom_entry(ch: &crate::omnigraph::client::EntityChange) -> Value {
    let props = ch
        .after
        .as_ref()
        .or(ch.before.as_ref())
        .map(|img| img.properties.clone())
        .unwrap_or_else(|| json!({}));
    let mut out = normalize_atom_row(&props);
    if out
        .get("atom_id")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        out["atom_id"] = json!(ch.id);
        out["id"] = json!(ch.id);
    }
    out = normalize_atom_row(&out);
    out["op"] = json!(ch.op.as_str());
    if let Some(before) = &ch.before {
        out["before"] = before.properties.clone();
    }
    if let Some(after) = &ch.after {
        out["after"] = after.properties.clone();
    }
    out
}

pub(super) fn diff_edge_entry(ch: &crate::omnigraph::client::EntityChange) -> Value {
    let img = match ch.op {
        ChangeOp::Delete => ch.before.as_ref().or(ch.after.as_ref()),
        _ => ch.after.as_ref().or(ch.before.as_ref()),
    };
    let endpoints = img.and_then(|i| i.endpoints.clone());
    let props = img.map(|i| i.properties.clone()).unwrap_or(Value::Null);
    let from = endpoints
        .as_ref()
        .map(|e| e.from.clone())
        .or_else(|| str_field(&props, "from"))
        .or_else(|| str_field(&props, "source_id"))
        .unwrap_or_default();
    let to = endpoints
        .as_ref()
        .map(|e| e.to.clone())
        .or_else(|| str_field(&props, "to"))
        .or_else(|| str_field(&props, "target_id"))
        .unwrap_or_default();
    let relation = diff_edge_relation(&ch.entity_type.name, &props);
    json!({
        "id": ch.id,
        "type": ch.entity_type.name,
        "op": ch.op.as_str(),
        "from": from,
        "to": to,
        "relation": relation,
        "endpoints": endpoints,
        "properties": props,
    })
}

pub(super) fn diff_edge_relation(entity_type: &str, props: &Value) -> String {
    if entity_type == "RelatesTo" {
        str_field(props, "relation").unwrap_or_else(|| "relates_to".to_string())
    } else {
        pascal_to_snake(entity_type)
    }
}

pub(in crate::omnigraph) fn pascal_to_snake(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 4);
    for (idx, ch) in raw.chars().enumerate() {
        if ch.is_uppercase() {
            if idx > 0 {
                out.push('_');
            }
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

pub(super) fn enrich_commit_change(ch: &crate::omnigraph::client::EntityChange) -> Value {
    let props = ch
        .after
        .as_ref()
        .or(ch.before.as_ref())
        .map(|img| img.properties.clone())
        .unwrap_or(Value::Null);
    let title = props["title"]
        .as_str()
        .or_else(|| props["path"].as_str())
        .unwrap_or(&ch.id)
        .to_string();
    let summary = props["summary"]
        .as_str()
        .or_else(|| props["relation"].as_str())
        .unwrap_or("")
        .to_string();
    let mut obj = serde_json::to_value(ch)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    obj.insert("entity_kind".into(), json!(ch.kind));
    obj.insert("type_name".into(), json!(ch.entity_type.name));
    obj.insert("entity_id".into(), json!(ch.id));
    obj.insert("title".into(), json!(title));
    obj.insert("summary".into(), json!(summary));
    if ch.kind == EntityKind::Node {
        obj.insert("node_id".into(), json!(ch.id));
        if ch.entity_type.name == "ContextAtom" {
            obj.insert("atom_id".into(), json!(ch.id));
        }
    }
    Value::Object(obj)
}

pub(in crate::omnigraph) fn normalize_relation_name(raw: &str) -> Option<String> {
    let normalized = match raw.trim().to_ascii_lowercase().as_str() {
        "relates_to" | "related" => "supports".to_string(),
        other => other.to_string(),
    };
    VALID_RELATIONS
        .contains(&normalized.as_str())
        .then_some(normalized)
}

pub(super) fn parse_relation_entry(
    rel: &Value,
) -> Option<(String, String, Option<String>, Option<f64>)> {
    if let Some(raw) = rel.as_str().map(str::trim).filter(|s| !s.is_empty()) {
        if let Some((prefix, rest)) = raw.split_once(':') {
            let target = rest.trim();
            if !target.is_empty()
                && let Some(relation) = normalize_relation_name(prefix)
            {
                return Some((target.to_string(), relation, None, None));
            }
        }
        return Some((raw.to_string(), "supports".to_string(), None, None));
    }
    let target_id = clean_opt(rel["target_id"].as_str().or_else(|| rel["id"].as_str()))?;
    let relation = match clean_opt(rel["relation"].as_str()) {
        Some(raw) => normalize_relation_name(raw)?,
        None => "supports".to_string(),
    };
    Some((
        target_id.to_string(),
        relation,
        clean_opt(rel["note"].as_str()).map(str::to_string),
        rel["confidence"].as_f64(),
    ))
}

pub(super) fn atom_matches_query(atom: &Value, q_lower: &str) -> bool {
    for key in ["title", "summary", "body", "source_branch", "kind"] {
        if atom[key]
            .as_str()
            .is_some_and(|s| s.to_ascii_lowercase().contains(q_lower))
        {
            return true;
        }
    }
    for array_key in ["tags", "files", "symbols"] {
        if let Some(items) = atom[array_key].as_array() {
            for item in items.iter().filter_map(Value::as_str) {
                if item.to_ascii_lowercase().contains(q_lower) {
                    return true;
                }
            }
        }
    }
    false
}

pub(super) fn score_atom_fallback(atom: &Value, q_lower: &str) -> f64 {
    let terms: Vec<&str> = q_lower.split_whitespace().collect();
    if terms.is_empty() {
        return 0.0;
    }
    let title = atom["title"].as_str().unwrap_or("").to_ascii_lowercase();
    let summary = atom["summary"].as_str().unwrap_or("").to_ascii_lowercase();
    let body = atom["body"].as_str().unwrap_or("").to_ascii_lowercase();
    let mut score = 0.0;
    for term in &terms {
        if title.contains(term) {
            score += 0.5;
        }
        if summary.contains(term) {
            score += 0.3;
        }
        if body.contains(term) {
            score += 0.2;
        }
    }
    score / (terms.len() as f64)
}

pub(super) fn format_context_capsule(
    graph_id: &str,
    branch: &str,
    project_path: &Path,
    atoms: &[Value],
    contradictions: &[Value],
) -> String {
    if atoms.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "Omnigraph Context Capsule (graph `{graph_id}`, branch `{branch}`, project `{}` — discovery aid only; subordinate to canonical code/config/tests, `project-brain/`, and `memory-bank/`):\n",
        project_path.display()
    );
    for atom in atoms {
        out.push_str(&format_atom_line_with_authority(atom));
        if let Some(atom_id) = str_field(atom, "atom_id")
            && let Some(caution) = contradiction_caution(&atom_id, contradictions)
        {
            out.push_str(&caution);
        }
    }
    out
}

/// One capsule line with the atom's authority tag: a verified Memory Bank or
/// Project Brain atom names its source, and anything else is marked as an
/// unverified reference to check against code and the Brain.
pub(crate) fn format_atom_line_with_authority(atom: &Value) -> String {
    {
        let kind = atom["kind"].as_str().unwrap_or("insight");
        let title = atom["title"].as_str().unwrap_or("Untitled");
        let summary = atom["summary"].as_str().unwrap_or("");
        let src = atom["source_branch"].as_str().unwrap_or("main");
        let verified = atom["verified"].as_bool().unwrap_or(false);
        let origin = clean_opt(atom["origin_layer"].as_str()).unwrap_or("");
        let brain_id = clean_opt(atom["brain_record_id"].as_str());
        let mem_id = clean_opt(atom["memory_id"].as_str());
        let is_imported = origin == "imported" || src.starts_with("imported/");

        let tag = if !is_imported && verified && origin == "memory-bank" {
            match mem_id {
                Some(mid) => format!("[memory-bank {kind} · {mid}]"),
                None => format!("[memory-bank {kind}]"),
            }
        } else if !is_imported && verified && origin == "project-brain" {
            match brain_id {
                Some(bid) => {
                    let short: String = bid.chars().take(8).collect();
                    format!("[project-brain {kind} · {short}]")
                }
                None => format!("[project-brain {kind}]"),
            }
        } else if !is_imported
            && verified
            && src == CANONICAL_KNOWLEDGE_BRANCH
            && (mem_id.is_some() || brain_id.is_some())
        {
            if let Some(mid) = mem_id {
                format!("[memory-bank {kind} · {mid}]")
            } else {
                let short: String = brain_id.unwrap_or("").chars().take(8).collect();
                format!("[project-brain {kind} · {short}]")
            }
        } else {
            format!("[unverified reference — verify against code & Brain] [{kind}]")
        };

        if summary.trim().is_empty() {
            format!("- {tag} {title} (from `{src}`)\n")
        } else {
            format!("- {tag} {title} (from `{src}`): {summary}\n")
        }
    }
}

#[cfg(test)]
pub(in crate::omnigraph) fn format_context_capsule_for_test(
    graph_id: &str,
    branch: &str,
    project_path: &Path,
    atoms: &[Value],
) -> String {
    format_context_capsule(graph_id, branch, project_path, atoms, &[])
}

pub(super) fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(super) fn clean_opt(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|s| !s.is_empty())
}

pub(super) fn parse_limit(v: &Value) -> Option<usize> {
    v.as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<usize>().ok()))
}

pub(super) fn format_omnigraph_ts(ts: i64) -> Option<String> {
    let dt = if ts >= 1_000_000_000_000_000 {
        chrono::DateTime::from_timestamp_micros(ts)
    } else if ts >= 1_000_000_000_000 {
        chrono::DateTime::from_timestamp_millis(ts)
    } else if ts > 0 {
        chrono::DateTime::from_timestamp(ts, 0)
    } else {
        None
    }?;
    Some(dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

pub(in crate::omnigraph) fn detect_language(path: &str) -> Option<&'static str> {
    let ext = Path::new(path).extension().and_then(|e| e.to_str())?;
    match ext.to_ascii_lowercase().as_str() {
        "rs" => Some("rust"),
        "php" | "phtml" | "inc" => Some("php"),
        "ts" | "tsx" | "mts" | "cts" => Some("typescript"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "py" | "pyi" => Some("python"),
        "go" => Some("go"),
        "java" => Some("java"),
        "kt" | "kts" => Some("kotlin"),
        "cs" => Some("csharp"),
        "rb" => Some("ruby"),
        "swift" => Some("swift"),
        "scala" | "sc" => Some("scala"),
        "sql" => Some("sql"),
        "md" => Some("markdown"),
        "css" => Some("css"),
        "html" => Some("html"),
        "json" => Some("json"),
        "yaml" | "yml" => Some("yaml"),
        "toml" => Some("toml"),
        _ => None,
    }
}
