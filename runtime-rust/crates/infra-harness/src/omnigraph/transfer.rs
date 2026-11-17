//! Bundle and NDJSON serialization, scoping, normalization, and validation helpers
//! for Omnigraph context export/import and multi-session linking.

use super::{
    schema,
    sidecar::{detect_language, normalize_relation_name, pascal_to_snake},
};
use crate::{HarnessError, Result, store};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const BUNDLE_FORMAT_V1: &str = "harness.omnigraph.bundle.v1";

const VALID_ATOM_KINDS: &[&str] = &[
    "finding", "decision", "warning", "contract", "insight", "artifact", "question",
];

#[derive(Clone, Debug, Default)]
pub(super) struct ParsedGraph {
    pub atoms: BTreeMap<String, Value>,
    pub files: BTreeMap<String, Value>,
    pub symbols: BTreeMap<String, Value>,
    pub sessions: BTreeMap<String, Value>,
    pub turns: BTreeMap<String, Value>,
    pub edges: Vec<EdgeRecord>,
}

#[derive(Clone, Debug)]
pub(super) struct EdgeRecord {
    pub edge_type: String,
    pub id: String,
    pub from: String,
    pub to: String,
    pub data: Map<String, Value>,
}

impl EdgeRecord {
    pub(super) fn dedup_key(&self) -> String {
        if self.edge_type == "RelatesTo" {
            let rel = self
                .data
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or("supports");
            format!("{}:{}:{}:{}", self.edge_type, self.from, rel, self.to)
        } else {
            format!("{}:{}:{}", self.edge_type, self.from, self.to)
        }
    }

    pub(super) fn relation_label(&self) -> String {
        if self.edge_type == "RelatesTo" {
            self.data
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or("supports")
                .to_string()
        } else {
            pascal_to_snake(&self.edge_type)
        }
    }

    pub(super) fn to_bundle_json(&self) -> Value {
        let relation = self.relation_label();
        json!({
            "id": self.id,
            "edge": self.edge_type,
            "type": self.edge_type,
            "from": self.from,
            "to": self.to,
            "source": self.from,
            "target": self.to,
            "relation": relation,
            "data": Value::Object(self.data.clone()),
        })
    }

    pub(super) fn to_ndjson_line(&self) -> String {
        json!({
            "edge": self.edge_type,
            "from": self.from,
            "to": self.to,
            "data": Value::Object(self.data.clone()),
        })
        .to_string()
    }
}

pub(super) fn parse_ndjson_records(ndjson: &str) -> Result<ParsedGraph> {
    let mut graph = ParsedGraph::default();
    for (idx, raw_line) in ndjson.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let val: Value = serde_json::from_str(line).map_err(|err| {
            HarnessError::invalid(format!("Invalid NDJSON at line {}: {err}", idx + 1))
        })?;
        ingest_record_value(&mut graph, &val, idx + 1)?;
    }
    Ok(graph)
}

fn ingest_record_value(graph: &mut ParsedGraph, val: &Value, line_no: usize) -> Result<()> {
    let Some(obj) = val.as_object() else {
        return Err(HarnessError::invalid(format!(
            "NDJSON line {line_no} must be a JSON object."
        )));
    };
    if let Some(node_type) = obj.get("type").and_then(Value::as_str) {
        let data = extract_node_data(obj);
        let explicit_id = obj.get("id").and_then(Value::as_str);
        insert_node_by_type(graph, node_type, explicit_id, data)?;
        return Ok(());
    }
    if let Some(edge_type) = obj.get("edge").and_then(Value::as_str) {
        let from = obj
            .get("from")
            .or_else(|| obj.get("source"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                HarnessError::invalid(format!("Edge line {line_no} is missing `from`."))
            })?
            .to_string();
        let to = obj
            .get("to")
            .or_else(|| obj.get("target"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| HarnessError::invalid(format!("Edge line {line_no} is missing `to`.")))?
            .to_string();
        let data = obj
            .get("data")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let id = obj
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("{edge_type}:{from}->{to}"));
        push_edge_dedup(
            graph,
            EdgeRecord {
                edge_type: normalize_edge_type(edge_type)?,
                id,
                from,
                to,
                data,
            },
        );
        return Ok(());
    }
    Err(HarnessError::invalid(format!(
        "NDJSON line {line_no} must contain `type` or `edge`."
    )))
}

fn extract_node_data(obj: &Map<String, Value>) -> Map<String, Value> {
    if let Some(data) = obj.get("data").and_then(Value::as_object) {
        return data.clone();
    }
    let mut flat = obj.clone();
    flat.remove("type");
    flat.remove("id");
    flat
}

fn insert_node_by_type(
    graph: &mut ParsedGraph,
    node_type: &str,
    explicit_id: Option<&str>,
    mut data: Map<String, Value>,
) -> Result<()> {
    match node_type {
        "ContextAtom" => {
            let aid = str_in_map(&data, "atom_id")
                .or_else(|| str_in_map(&data, "id"))
                .or_else(|| {
                    explicit_id
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| format!("atom-{}", &store::hexid()[..12]));
            data.insert("atom_id".into(), Value::String(aid.clone()));
            graph.atoms.insert(aid, Value::Object(data));
        }
        "FileRef" => {
            let path = str_in_map(&data, "path")
                .or_else(|| {
                    explicit_id
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .ok_or_else(|| HarnessError::invalid("FileRef node is missing `path`."))?;
            data.insert("path".into(), Value::String(path.clone()));
            graph.files.insert(path, Value::Object(data));
        }
        "SymbolRef" => {
            let fqn = str_in_map(&data, "fqn")
                .or_else(|| {
                    explicit_id
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .ok_or_else(|| HarnessError::invalid("SymbolRef node is missing `fqn`."))?;
            data.insert("fqn".into(), Value::String(fqn.clone()));
            graph.symbols.insert(fqn, Value::Object(data));
        }
        "Session" => {
            let sid = str_in_map(&data, "session_id")
                .or_else(|| str_in_map(&data, "id"))
                .or_else(|| {
                    explicit_id
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .ok_or_else(|| HarnessError::invalid("Session node is missing `session_id`."))?;
            data.insert("session_id".into(), Value::String(sid.clone()));
            graph.sessions.insert(sid, Value::Object(data));
        }
        "TurnSnapshot" => {
            let snap_id = str_in_map(&data, "snapshot_id")
                .or_else(|| str_in_map(&data, "id"))
                .or_else(|| {
                    explicit_id
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| format!("snap-{}", &store::hexid()[..12]));
            data.insert("snapshot_id".into(), Value::String(snap_id.clone()));
            graph.turns.insert(snap_id, Value::Object(data));
        }
        other => {
            return Err(HarnessError::invalid(format!(
                "Unsupported Omnigraph node type '{other}'."
            )));
        }
    }
    Ok(())
}

fn push_edge_dedup(graph: &mut ParsedGraph, edge: EdgeRecord) {
    let key = edge.dedup_key();
    if let Some(pos) = graph.edges.iter().position(|e| e.dedup_key() == key) {
        graph.edges[pos] = edge;
    } else {
        graph.edges.push(edge);
    }
}

fn normalize_edge_type(raw: &str) -> Result<String> {
    match raw.trim() {
        "PublishedBy" | "published_by" => Ok("PublishedBy".into()),
        "ConsumedBy" | "consumed_by" => Ok("ConsumedBy".into()),
        "TouchesFile" | "touches_file" => Ok("TouchesFile".into()),
        "MentionsSymbol" | "mentions_symbol" => Ok("MentionsSymbol".into()),
        "RelatesTo" | "relates_to" => Ok("RelatesTo".into()),
        "DerivedFrom" | "derived_from" => Ok("DerivedFrom".into()),
        "SessionFollows" | "session_follows" | "follows" => Ok("SessionFollows".into()),
        other => Err(HarnessError::invalid(format!(
            "Unsupported Omnigraph edge type '{other}'."
        ))),
    }
}

pub(super) fn filter_graph_for_export(
    raw: &ParsedGraph,
    project_id: &str,
    scope: &str,
    session_id: Option<&str>,
    resolved_branch: &str,
    include_inherited: bool,
) -> ParsedGraph {
    let project_matches = |v: &Value| -> bool {
        if project_id.is_empty() {
            return true;
        }
        match v.get("project_id").and_then(Value::as_str).map(str::trim) {
            Some(pid) if !pid.is_empty() => pid == project_id,
            _ => true,
        }
    };

    let proj_atoms: BTreeMap<String, Value> = raw
        .atoms
        .iter()
        .filter(|(_, v)| project_matches(v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let proj_files: BTreeMap<String, Value> = raw
        .files
        .iter()
        .filter(|(_, v)| project_matches(v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let proj_symbols: BTreeMap<String, Value> = raw
        .symbols
        .iter()
        .filter(|(_, v)| project_matches(v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let proj_sessions: BTreeMap<String, Value> = raw
        .sessions
        .iter()
        .filter(|(_, v)| project_matches(v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let proj_turns: BTreeMap<String, Value> = raw
        .turns
        .iter()
        .filter(|(_, v)| {
            if project_id.is_empty() || proj_sessions.is_empty() {
                return true;
            }
            v.get("session_id")
                .and_then(Value::as_str)
                .is_none_or(|sid| {
                    proj_sessions.contains_key(sid) || !raw.sessions.contains_key(sid)
                })
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let mut kept_atom_ids: BTreeSet<String> = BTreeSet::new();
    let mut kept_file_ids: BTreeSet<String> = BTreeSet::new();
    let mut kept_symbol_ids: BTreeSet<String> = BTreeSet::new();
    let mut kept_session_ids: BTreeSet<String> = BTreeSet::new();
    let mut kept_turn_ids: BTreeSet<String> = BTreeSet::new();
    let mut kept_edges: Vec<EdgeRecord> = Vec::new();

    if scope == "task"
        && let Some(sid) = session_id
        && !include_inherited
    {
        if proj_sessions.contains_key(sid) {
            kept_session_ids.insert(sid.to_string());
        }
        for (aid, atom) in &proj_atoms {
            if atom.get("source_session_id").and_then(Value::as_str) == Some(sid)
                || (resolved_branch != "main"
                    && atom.get("source_branch").and_then(Value::as_str) == Some(resolved_branch))
            {
                kept_atom_ids.insert(aid.clone());
            }
        }
        for edge in &raw.edges {
            if matches!(edge.edge_type.as_str(), "PublishedBy" | "ConsumedBy")
                && edge.to == sid
                && proj_atoms.contains_key(&edge.from)
            {
                kept_atom_ids.insert(edge.from.clone());
            }
        }
        for (tid, turn) in &proj_turns {
            if turn.get("session_id").and_then(Value::as_str) == Some(sid) {
                kept_turn_ids.insert(tid.clone());
            }
        }
        for edge in &raw.edges {
            match edge.edge_type.as_str() {
                "PublishedBy" => {
                    if kept_atom_ids.contains(&edge.from) && proj_sessions.contains_key(&edge.to) {
                        kept_session_ids.insert(edge.to.clone());
                        kept_edges.push(edge.clone());
                    }
                }
                "ConsumedBy" => {
                    if kept_atom_ids.contains(&edge.from)
                        && edge.to == sid
                        && proj_sessions.contains_key(&edge.to)
                    {
                        kept_session_ids.insert(edge.to.clone());
                        kept_edges.push(edge.clone());
                    }
                }
                "TouchesFile" => {
                    if kept_atom_ids.contains(&edge.from) && proj_files.contains_key(&edge.to) {
                        kept_file_ids.insert(edge.to.clone());
                        kept_edges.push(edge.clone());
                    }
                }
                "MentionsSymbol" => {
                    if kept_atom_ids.contains(&edge.from) && proj_symbols.contains_key(&edge.to) {
                        kept_symbol_ids.insert(edge.to.clone());
                        kept_edges.push(edge.clone());
                    }
                }
                "DerivedFrom" => {
                    if kept_atom_ids.contains(&edge.from) && proj_turns.contains_key(&edge.to) {
                        kept_turn_ids.insert(edge.to.clone());
                        kept_edges.push(edge.clone());
                    }
                }
                "RelatesTo" => {
                    if kept_atom_ids.contains(&edge.from) && kept_atom_ids.contains(&edge.to) {
                        kept_edges.push(edge.clone());
                    }
                }
                "SessionFollows"
                    if (edge.from == sid || edge.to == sid)
                        && proj_sessions.contains_key(&edge.from)
                        && proj_sessions.contains_key(&edge.to) =>
                {
                    kept_session_ids.insert(edge.from.clone());
                    kept_session_ids.insert(edge.to.clone());
                    kept_edges.push(edge.clone());
                }
                _ => {}
            }
        }
    } else if scope == "branch" && !include_inherited && resolved_branch != "main" {
        for (aid, atom) in &proj_atoms {
            if atom.get("source_branch").and_then(Value::as_str) == Some(resolved_branch) {
                kept_atom_ids.insert(aid.clone());
            }
        }
        for edge in &raw.edges {
            match edge.edge_type.as_str() {
                "PublishedBy" | "ConsumedBy" => {
                    if kept_atom_ids.contains(&edge.from) && proj_sessions.contains_key(&edge.to) {
                        kept_session_ids.insert(edge.to.clone());
                        kept_edges.push(edge.clone());
                    }
                }
                "TouchesFile" => {
                    if kept_atom_ids.contains(&edge.from) && proj_files.contains_key(&edge.to) {
                        kept_file_ids.insert(edge.to.clone());
                        kept_edges.push(edge.clone());
                    }
                }
                "MentionsSymbol" => {
                    if kept_atom_ids.contains(&edge.from) && proj_symbols.contains_key(&edge.to) {
                        kept_symbol_ids.insert(edge.to.clone());
                        kept_edges.push(edge.clone());
                    }
                }
                "DerivedFrom" => {
                    if kept_atom_ids.contains(&edge.from) && proj_turns.contains_key(&edge.to) {
                        kept_turn_ids.insert(edge.to.clone());
                        kept_edges.push(edge.clone());
                    }
                }
                "RelatesTo" => {
                    if kept_atom_ids.contains(&edge.from) && kept_atom_ids.contains(&edge.to) {
                        kept_edges.push(edge.clone());
                    }
                }
                "SessionFollows"
                    if proj_sessions.contains_key(&edge.from)
                        && proj_sessions.contains_key(&edge.to) =>
                {
                    kept_session_ids.insert(edge.from.clone());
                    kept_session_ids.insert(edge.to.clone());
                    kept_edges.push(edge.clone());
                }
                _ => {}
            }
        }
    } else {
        kept_atom_ids.extend(proj_atoms.keys().cloned());
        kept_file_ids.extend(proj_files.keys().cloned());
        kept_symbol_ids.extend(proj_symbols.keys().cloned());
        kept_session_ids.extend(proj_sessions.keys().cloned());
        kept_turn_ids.extend(proj_turns.keys().cloned());
        for edge in &raw.edges {
            let valid = match edge.edge_type.as_str() {
                "PublishedBy" | "ConsumedBy" => {
                    kept_atom_ids.contains(&edge.from) && kept_session_ids.contains(&edge.to)
                }
                "TouchesFile" => {
                    kept_atom_ids.contains(&edge.from) && kept_file_ids.contains(&edge.to)
                }
                "MentionsSymbol" => {
                    kept_atom_ids.contains(&edge.from) && kept_symbol_ids.contains(&edge.to)
                }
                "DerivedFrom" => {
                    kept_atom_ids.contains(&edge.from) && kept_turn_ids.contains(&edge.to)
                }
                "RelatesTo" => {
                    kept_atom_ids.contains(&edge.from) && kept_atom_ids.contains(&edge.to)
                }
                "SessionFollows" => {
                    kept_session_ids.contains(&edge.from) && kept_session_ids.contains(&edge.to)
                }
                _ => false,
            };
            if valid {
                kept_edges.push(edge.clone());
            }
        }
    }

    ParsedGraph {
        atoms: proj_atoms
            .into_iter()
            .filter(|(k, _)| kept_atom_ids.contains(k))
            .collect(),
        files: proj_files
            .into_iter()
            .filter(|(k, _)| kept_file_ids.contains(k))
            .collect(),
        symbols: proj_symbols
            .into_iter()
            .filter(|(k, _)| kept_symbol_ids.contains(k))
            .collect(),
        sessions: proj_sessions
            .into_iter()
            .filter(|(k, _)| kept_session_ids.contains(k))
            .collect(),
        turns: proj_turns
            .into_iter()
            .filter(|(k, _)| kept_turn_ids.contains(k))
            .collect(),
        edges: kept_edges,
    }
}

pub(super) fn serialize_graph_ndjson(graph: &ParsedGraph) -> String {
    let mut lines = Vec::new();
    for (id, data) in &graph.atoms {
        lines.push(json!({"type": "ContextAtom", "id": id, "data": data}).to_string());
    }
    for (id, data) in &graph.files {
        lines.push(json!({"type": "FileRef", "id": id, "data": data}).to_string());
    }
    for (id, data) in &graph.symbols {
        lines.push(json!({"type": "SymbolRef", "id": id, "data": data}).to_string());
    }
    for (id, data) in &graph.sessions {
        lines.push(json!({"type": "Session", "id": id, "data": data}).to_string());
    }
    for (id, data) in &graph.turns {
        lines.push(json!({"type": "TurnSnapshot", "id": id, "data": data}).to_string());
    }
    for edge in &graph.edges {
        lines.push(
            json!({
                "edge": edge.edge_type,
                "id": edge.id,
                "from": edge.from,
                "to": edge.to,
                "data": Value::Object(edge.data.clone()),
            })
            .to_string(),
        );
    }
    lines.join("\n")
}

pub(super) fn parse_import_body(body: &Value) -> Result<ParsedGraph> {
    if let Some(bundle_val) = body.get("bundle").filter(|v| !v.is_null()) {
        if let Some(s) = bundle_val.as_str() {
            return parse_import_text(s);
        }
        if bundle_val.is_object() {
            return parse_bundle_object(bundle_val);
        }
        return Err(HarnessError::invalid(
            "`bundle` must be an export JSON object or string.",
        ));
    }
    if let Some(raw_text) = body.get("raw_text").and_then(Value::as_str) {
        return parse_import_text(raw_text);
    }
    Err(HarnessError::invalid(
        "Provide either `bundle` or `raw_text` to import.",
    ))
}

fn parse_import_text(raw: &str) -> Result<ParsedGraph> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(HarnessError::invalid("Import payload is empty."));
    }
    if trimmed.starts_with('{')
        && let Ok(val) = serde_json::from_str::<Value>(trimmed)
        && val.is_object()
    {
        if val.get("type").is_some() || val.get("edge").is_some() {
            return parse_ndjson_records(trimmed);
        }
        return parse_bundle_object(&val);
    }
    parse_ndjson_records(trimmed)
}

fn parse_bundle_object(val: &Value) -> Result<ParsedGraph> {
    let root = if val.get("bundle").is_some_and(Value::is_object) {
        &val["bundle"]
    } else {
        val
    };
    let mut graph = if let Some(ndjson) = root.get("ndjson").and_then(Value::as_str)
        && !ndjson.trim().is_empty()
    {
        parse_ndjson_records(ndjson)?
    } else {
        ParsedGraph::default()
    };

    for (key, node_type) in [
        ("atoms", "ContextAtom"),
        ("files", "FileRef"),
        ("symbols", "SymbolRef"),
        ("sessions", "Session"),
        ("turns", "TurnSnapshot"),
    ] {
        if let Some(arr) = root.get(key).and_then(Value::as_array) {
            for item in arr {
                if let Some(obj) = item.as_object() {
                    let mut clean = obj.clone();
                    if node_type == "ContextAtom" {
                        clean.remove("id");
                        clean.remove("files");
                    } else if matches!(
                        node_type,
                        "FileRef" | "SymbolRef" | "Session" | "TurnSnapshot"
                    ) {
                        clean.remove("id");
                    }
                    let id_hint = item.get("id").and_then(Value::as_str);
                    if !already_present(&graph, node_type, item) {
                        insert_node_by_type(&mut graph, node_type, id_hint, clean)?;
                    }
                }
            }
        }
    }

    if let Some(edges) = root.get("edges").and_then(Value::as_array) {
        for edge in edges {
            let explicit_type = edge
                .get("edge")
                .or_else(|| edge.get("type"))
                .or_else(|| edge.get("kind"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let rel_hint = edge
                .get("relation")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let (edge_type, inferred_rel): (String, Option<String>) = match explicit_type {
                Some(raw_type) => (normalize_edge_type(raw_type)?, rel_hint.map(str::to_string)),
                None => {
                    let Some(rel) = rel_hint else {
                        continue;
                    };
                    if let Ok(known) = normalize_edge_type(rel) {
                        (known, None)
                    } else if let Some(norm_rel) = normalize_relation_name(rel) {
                        ("RelatesTo".into(), Some(norm_rel))
                    } else {
                        return Err(HarnessError::invalid(format!(
                            "Unsupported Omnigraph edge relation '{rel}'."
                        )));
                    }
                }
            };
            let Some(from) = edge
                .get("from")
                .or_else(|| edge.get("source"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            let Some(to) = edge
                .get("to")
                .or_else(|| edge.get("target"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            let mut data = edge
                .get("data")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if matches!(edge_type.as_str(), "RelatesTo" | "SessionFollows")
                && !data.contains_key("relation")
                && let Some(rel) = inferred_rel.or_else(|| rel_hint.map(str::to_string))
            {
                data.insert("relation".into(), json!(rel));
            }
            let rec = EdgeRecord {
                id: edge
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{edge_type}:{from}->{to}")),
                edge_type,
                from: from.to_string(),
                to: to.to_string(),
                data,
            };
            let key = rec.dedup_key();
            if !graph.edges.iter().any(|e| e.dedup_key() == key) {
                graph.edges.push(rec);
            }
        }
    }

    if graph.atoms.is_empty()
        && graph.files.is_empty()
        && graph.symbols.is_empty()
        && graph.sessions.is_empty()
        && graph.turns.is_empty()
        && graph.edges.is_empty()
    {
        return Err(HarnessError::invalid(
            "Import bundle contains no Omnigraph nodes or edges.",
        ));
    }
    Ok(graph)
}

fn already_present(graph: &ParsedGraph, node_type: &str, item: &Value) -> bool {
    match node_type {
        "ContextAtom" => item
            .get("atom_id")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
            .is_some_and(|id| graph.atoms.contains_key(id)),
        "FileRef" => item
            .get("path")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
            .is_some_and(|id| graph.files.contains_key(id)),
        "SymbolRef" => item
            .get("fqn")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
            .is_some_and(|id| graph.symbols.contains_key(id)),
        "Session" => item
            .get("session_id")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
            .is_some_and(|id| graph.sessions.contains_key(id)),
        "TurnSnapshot" => item
            .get("snapshot_id")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
            .is_some_and(|id| graph.turns.contains_key(id)),
        _ => false,
    }
}

pub(super) fn prepare_import_ndjson(
    raw: &ParsedGraph,
    dest_project_id: &str,
    target_branch: &str,
    existing_on_branch: Option<&ParsedGraph>,
    ts: &str,
) -> (ParsedGraph, String) {
    let mut normalized = ParsedGraph::default();
    let pick_proj = |v: &Value| -> String {
        if !dest_project_id.is_empty() {
            dest_project_id.to_string()
        } else {
            v.get("project_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        }
    };

    for (aid, v) in &raw.atoms {
        let raw_kind = v
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("insight")
            .to_ascii_lowercase();
        let kind = if VALID_ATOM_KINDS.contains(&raw_kind.as_str()) {
            raw_kind
        } else {
            "insight".to_string()
        };
        let title = v
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(aid.as_str())
            .to_string();
        let summary = v
            .get("summary")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&title)
            .to_string();
        let body = v
            .get("body")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&summary)
            .to_string();
        let tags = v
            .get("tags")
            .and_then(Value::as_array)
            .map(|arr| {
                Value::Array(
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(|s| Value::String(s.to_string()))
                        .collect(),
                )
            })
            .unwrap_or(Value::Null);
        let source_branch = v
            .get("source_branch")
            .or_else(|| v.get("branch"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(target_branch)
            .to_string();
        let source_session_id = v
            .get("source_session_id")
            .or_else(|| v.get("session_id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| Value::String(s.to_string()))
            .unwrap_or(Value::Null);
        let confidence = v
            .get("confidence")
            .and_then(Value::as_f64)
            .unwrap_or(0.85)
            .clamp(0.0, 1.0);
        let pin_priority = v.get("pin_priority").and_then(Value::as_i64).unwrap_or(10) as i32;
        let verified = v.get("verified").and_then(Value::as_bool).unwrap_or(false);
        let origin_layer = opt_trimmed_str_val(v, "origin_layer");
        let authority = opt_trimmed_str_val(v, "authority");
        let privacy = opt_trimmed_str_val(v, "privacy");
        let brain_record_id = opt_trimmed_str_val(v, "brain_record_id");
        let memory_id = opt_trimmed_str_val(v, "memory_id");
        let record_revision = v
            .get("record_revision")
            .and_then(parse_i64_opt)
            .map(|n| Value::Number(n.into()))
            .unwrap_or(Value::Null);
        let created_at = normalize_iso_ms(v.get("created_at").and_then(Value::as_str), ts);
        let updated_at = normalize_iso_ms(v.get("updated_at").and_then(Value::as_str), &created_at);

        normalized.atoms.insert(
            aid.clone(),
            json!({
                "atom_id": aid,
                "project_id": pick_proj(v),
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
                "origin_layer": origin_layer,
                "authority": authority,
                "privacy": privacy,
                "brain_record_id": brain_record_id,
                "memory_id": memory_id,
                "record_revision": record_revision,
                "created_at": created_at,
                "updated_at": updated_at,
            }),
        );
    }

    for (path, v) in &raw.files {
        let language = v
            .get("language")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| Value::String(s.to_string()))
            .or_else(|| detect_language(path).map(|s| Value::String(s.into())))
            .unwrap_or(Value::Null);
        let last_touched_at =
            normalize_iso_ms(v.get("last_touched_at").and_then(Value::as_str), ts);
        normalized.files.insert(
            path.clone(),
            json!({
                "path": path,
                "project_id": pick_proj(v),
                "language": language,
                "last_touched_at": last_touched_at,
            }),
        );
    }

    for (fqn, v) in &raw.symbols {
        let file_path = v
            .get("file_path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let kind = v
            .get("kind")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("symbol")
            .to_string();
        let name = v
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(fqn.as_str())
            .to_string();
        normalized.symbols.insert(
            fqn.clone(),
            json!({
                "fqn": fqn,
                "project_id": pick_proj(v),
                "file_path": file_path,
                "kind": kind,
                "name": name,
            }),
        );
    }

    for (sid, v) in &raw.sessions {
        let created_at = normalize_iso_ms(v.get("created_at").and_then(Value::as_str), ts);
        let updated_at = normalize_iso_ms(v.get("updated_at").and_then(Value::as_str), &created_at);
        normalized.sessions.insert(
            sid.clone(),
            json!({
                "session_id": sid,
                "project_id": pick_proj(v),
                "title": v.get("title").and_then(Value::as_str).unwrap_or("Imported Session"),
                "branch": v.get("branch").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(&super::session_branch_name(sid)),
                "provider": v.get("provider").and_then(Value::as_str).unwrap_or("codex"),
                "workflow": v.get("workflow").and_then(Value::as_str).unwrap_or("native"),
                "status": v.get("status").and_then(Value::as_str).unwrap_or("completed"),
                "goal": v.get("goal").cloned().unwrap_or(Value::Null),
                "created_at": created_at,
                "updated_at": updated_at,
            }),
        );
    }

    for (snap_id, v) in &raw.turns {
        let created_at = normalize_iso_ms(v.get("created_at").and_then(Value::as_str), ts);
        normalized.turns.insert(
            snap_id.clone(),
            json!({
                "snapshot_id": snap_id,
                "session_id": v.get("session_id").and_then(Value::as_str).unwrap_or(""),
                "turn_index": v.get("turn_index").and_then(Value::as_i64).unwrap_or(1) as i32,
                "summary": v.get("summary").and_then(Value::as_str).unwrap_or("Turn snapshot"),
                "created_at": created_at,
            }),
        );
    }

    // Ensure referenced FileRef and Session targets exist (in batch or on target_branch) so Omnigraph
    // load/ndjson never fails referential integrity on imported edges.
    fn has_entity(
        in_batch: &BTreeMap<String, Value>,
        on_branch: Option<&BTreeMap<String, Value>>,
        id: &str,
    ) -> bool {
        in_batch.contains_key(id) || on_branch.is_some_and(|ex| ex.contains_key(id))
    }

    for edge in &raw.edges {
        let mut data = Map::new();
        let valid = match edge.edge_type.as_str() {
            "PublishedBy" => {
                if !has_entity(
                    &normalized.atoms,
                    existing_on_branch.map(|ex| &ex.atoms),
                    &edge.from,
                ) {
                    false
                } else {
                    if !has_entity(
                        &normalized.sessions,
                        existing_on_branch.map(|ex| &ex.sessions),
                        &edge.to,
                    ) {
                        normalized.sessions.insert(
                            edge.to.clone(),
                            placeholder_session(&edge.to, dest_project_id, ts),
                        );
                    }
                    if let Some(role) = edge.data.get("role").and_then(Value::as_str) {
                        data.insert("role".into(), json!(role));
                    }
                    data.insert(
                        "published_at".into(),
                        json!(normalize_iso_ms(
                            edge.data.get("published_at").and_then(Value::as_str),
                            ts
                        )),
                    );
                    true
                }
            }
            "ConsumedBy" => {
                if !has_entity(
                    &normalized.atoms,
                    existing_on_branch.map(|ex| &ex.atoms),
                    &edge.from,
                ) {
                    false
                } else {
                    if !has_entity(
                        &normalized.sessions,
                        existing_on_branch.map(|ex| &ex.sessions),
                        &edge.to,
                    ) {
                        normalized.sessions.insert(
                            edge.to.clone(),
                            placeholder_session(&edge.to, dest_project_id, ts),
                        );
                    }
                    data.insert(
                        "consumed_at".into(),
                        json!(normalize_iso_ms(
                            edge.data.get("consumed_at").and_then(Value::as_str),
                            ts
                        )),
                    );
                    if let Some(mode) = edge.data.get("injection_mode").and_then(Value::as_str) {
                        data.insert("injection_mode".into(), json!(mode));
                    }
                    if let Some(rel) = edge.data.get("relevance").and_then(Value::as_f64) {
                        data.insert("relevance".into(), json!(rel));
                    }
                    true
                }
            }
            "TouchesFile" => {
                if !has_entity(
                    &normalized.atoms,
                    existing_on_branch.map(|ex| &ex.atoms),
                    &edge.from,
                ) {
                    false
                } else {
                    if !has_entity(
                        &normalized.files,
                        existing_on_branch.map(|ex| &ex.files),
                        &edge.to,
                    ) {
                        normalized.files.insert(
                            edge.to.clone(),
                            json!({
                                "path": edge.to,
                                "project_id": dest_project_id,
                                "language": detect_language(&edge.to),
                                "last_touched_at": ts,
                            }),
                        );
                    }
                    if let Some(ls) = edge.data.get("line_start").and_then(Value::as_i64) {
                        data.insert("line_start".into(), json!(ls));
                    }
                    if let Some(le) = edge.data.get("line_end").and_then(Value::as_i64) {
                        data.insert("line_end".into(), json!(le));
                    }
                    if let Some(acc) = edge.data.get("access").and_then(Value::as_str) {
                        data.insert("access".into(), json!(acc));
                    }
                    true
                }
            }
            "MentionsSymbol" => {
                if !has_entity(
                    &normalized.atoms,
                    existing_on_branch.map(|ex| &ex.atoms),
                    &edge.from,
                ) || !has_entity(
                    &normalized.symbols,
                    existing_on_branch.map(|ex| &ex.symbols),
                    &edge.to,
                ) {
                    false
                } else {
                    if let Some(role) = edge.data.get("role").and_then(Value::as_str) {
                        data.insert("role".into(), json!(role));
                    }
                    true
                }
            }
            "RelatesTo" => {
                if !has_entity(
                    &normalized.atoms,
                    existing_on_branch.map(|ex| &ex.atoms),
                    &edge.from,
                ) || !has_entity(
                    &normalized.atoms,
                    existing_on_branch.map(|ex| &ex.atoms),
                    &edge.to,
                ) {
                    false
                } else {
                    let raw_rel = edge
                        .data
                        .get("relation")
                        .and_then(Value::as_str)
                        .unwrap_or("supports");
                    let rel = normalize_relation_name(raw_rel).unwrap_or_else(|| "supports".into());
                    data.insert("relation".into(), json!(rel));
                    if let Some(note) = edge.data.get("note").and_then(Value::as_str) {
                        data.insert("note".into(), json!(note));
                    }
                    if let Some(conf) = edge.data.get("confidence").and_then(Value::as_f64) {
                        data.insert("confidence".into(), json!(conf));
                    }
                    data.insert(
                        "created_at".into(),
                        json!(normalize_iso_ms(
                            edge.data.get("created_at").and_then(Value::as_str),
                            ts
                        )),
                    );
                    true
                }
            }
            "DerivedFrom" => {
                if !has_entity(
                    &normalized.atoms,
                    existing_on_branch.map(|ex| &ex.atoms),
                    &edge.from,
                ) || !has_entity(
                    &normalized.turns,
                    existing_on_branch.map(|ex| &ex.turns),
                    &edge.to,
                ) {
                    false
                } else {
                    data.insert(
                        "recorded_at".into(),
                        json!(normalize_iso_ms(
                            edge.data.get("recorded_at").and_then(Value::as_str),
                            ts
                        )),
                    );
                    true
                }
            }
            "SessionFollows" => {
                if !has_entity(
                    &normalized.sessions,
                    existing_on_branch.map(|ex| &ex.sessions),
                    &edge.from,
                ) {
                    normalized.sessions.insert(
                        edge.from.clone(),
                        placeholder_session(&edge.from, dest_project_id, ts),
                    );
                }
                if !has_entity(
                    &normalized.sessions,
                    existing_on_branch.map(|ex| &ex.sessions),
                    &edge.to,
                ) {
                    normalized.sessions.insert(
                        edge.to.clone(),
                        placeholder_session(&edge.to, dest_project_id, ts),
                    );
                }
                data.insert(
                    "relation".into(),
                    json!(
                        edge.data
                            .get("relation")
                            .and_then(Value::as_str)
                            .unwrap_or("linked")
                    ),
                );
                data.insert(
                    "created_at".into(),
                    json!(normalize_iso_ms(
                        edge.data.get("created_at").and_then(Value::as_str),
                        ts
                    )),
                );
                true
            }
            _ => false,
        };
        if valid {
            push_edge_dedup(
                &mut normalized,
                EdgeRecord {
                    edge_type: edge.edge_type.clone(),
                    id: edge.id.clone(),
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                    data,
                },
            );
        }
    }

    let ndjson = build_load_ndjson(&normalized);
    (normalized, ndjson)
}

pub(super) fn build_load_ndjson(graph: &ParsedGraph) -> String {
    let mut lines = Vec::new();
    for data in graph.atoms.values() {
        lines.push(json!({"type": "ContextAtom", "data": data}).to_string());
    }
    for data in graph.files.values() {
        lines.push(json!({"type": "FileRef", "data": data}).to_string());
    }
    for data in graph.symbols.values() {
        lines.push(json!({"type": "SymbolRef", "data": data}).to_string());
    }
    for data in graph.sessions.values() {
        lines.push(json!({"type": "Session", "data": data}).to_string());
    }
    for data in graph.turns.values() {
        lines.push(json!({"type": "TurnSnapshot", "data": data}).to_string());
    }
    for edge in &graph.edges {
        lines.push(edge.to_ndjson_line());
    }
    lines.join("\n")
}

fn placeholder_session(sid: &str, project_id: &str, ts: &str) -> Value {
    json!({
        "session_id": sid,
        "project_id": project_id,
        "title": "Harness Session",
        "branch": super::session_branch_name(sid),
        "provider": "codex",
        "workflow": "native",
        "status": "completed",
        "goal": Value::Null,
        "created_at": ts,
        "updated_at": ts,
    })
}

fn normalize_iso_ms(raw: Option<&str>, fallback: &str) -> String {
    let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return fallback.to_string();
    };
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return dt
            .with_timezone(&chrono::Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(ndt, chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        }
    }
    fallback.to_string()
}

fn str_in_map(map: &Map<String, Value>, key: &str) -> Option<String> {
    map.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn opt_trimmed_str_val(v: &Value, key: &str) -> Value {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| Value::String(s.to_string()))
        .unwrap_or(Value::Null)
}

pub(super) fn parse_i64_opt(val: &Value) -> Option<i64> {
    val.as_i64()
        .or_else(|| val.as_u64().and_then(|n| i64::try_from(n).ok()))
        .or_else(|| val.as_str().and_then(|s| s.trim().parse::<i64>().ok()))
}

pub(super) fn parse_bool_param(val: &Value) -> Option<bool> {
    if let Some(b) = val.as_bool() {
        return Some(b);
    }
    val.as_str()
        .and_then(|s| match s.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" => Some(true),
            "0" | "false" | "no" => Some(false),
            _ => None,
        })
}

pub(super) fn slugify_segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_dash = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "context".into()
    } else {
        out.chars().take(48).collect()
    }
}

pub(super) fn validate_branch_token(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.len() > 128
        || trimmed.chars().any(|c| c.is_control() || c.is_whitespace())
        || trimmed.starts_with('/')
        || trimmed.ends_with('/')
        || trimmed.contains("..")
        || trimmed.contains("//")
    {
        return Err(HarnessError::invalid("Invalid Omnigraph branch name."));
    }
    Ok(trimmed.to_owned())
}

fn check_branch_safety(trimmed: &str) -> Result<()> {
    if trimmed.is_empty()
        || trimmed.chars().count() > 128
        || trimmed.chars().any(char::is_control)
        || trimmed.starts_with('/')
        || trimmed.ends_with('/')
        || trimmed.contains("..")
        || trimmed.contains("//")
    {
        return Err(HarnessError::invalid("Invalid Omnigraph branch name."));
    }
    Ok(())
}

/// Normalize a `shared_space` name or slug into a canonical `shared/<slug>` branch (`main` allowed).
pub fn normalize_shared_space_branch(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    check_branch_safety(trimmed)
        .map_err(|_| HarnessError::invalid("Invalid Omnigraph `shared_space`."))?;
    if trimmed == "main" {
        return Ok("main".to_string());
    }
    let tail = trimmed.strip_prefix("shared/").unwrap_or(trimmed).trim();
    if tail.is_empty() || !tail.chars().any(|c| c.is_ascii_alphanumeric()) {
        return Err(HarnessError::invalid("Invalid Omnigraph `shared_space`."));
    }
    Ok(format!("shared/{}", slugify_segment(tail)))
}

/// Normalize a link target branch or human space name into a valid Omnigraph branch token.
pub(super) fn normalize_link_target_branch(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    check_branch_safety(trimmed)?;
    if trimmed == "main" {
        return Ok("main".to_string());
    }
    if trimmed.starts_with("session/") {
        return validate_branch_token(trimmed);
    }
    if let Some(rest) = trimmed.strip_prefix("shared/") {
        let rest = rest.trim();
        if rest.is_empty() || !rest.chars().any(|c| c.is_ascii_alphanumeric()) {
            return Err(HarnessError::invalid("Invalid Omnigraph branch name."));
        }
        return Ok(format!("shared/{}", slugify_segment(rest)));
    }
    if trimmed.contains('/') {
        return validate_branch_token(trimmed);
    }
    if !trimmed.chars().any(|c| c.is_ascii_alphanumeric()) {
        return Err(HarnessError::invalid("Invalid Omnigraph branch name."));
    }
    Ok(format!("shared/{}", slugify_segment(trimmed)))
}

pub(super) fn filter_ndjson_excluding_atoms(
    ndjson: &str,
    project_id: &str,
    excluded_atom_ids: &BTreeSet<String>,
) -> String {
    let Ok(parsed) = parse_ndjson_records(ndjson) else {
        return String::new();
    };
    let mut filtered = filter_graph_for_export(&parsed, project_id, "all", None, "main", true);
    if !excluded_atom_ids.is_empty() {
        for aid in excluded_atom_ids {
            filtered.atoms.remove(aid);
        }
        filtered.edges.retain(|e| {
            let from_bad = matches!(
                e.edge_type.as_str(),
                "PublishedBy"
                    | "ConsumedBy"
                    | "TouchesFile"
                    | "MentionsSymbol"
                    | "RelatesTo"
                    | "DerivedFrom"
            ) && excluded_atom_ids.contains(&e.from);
            let to_bad = e.edge_type == "RelatesTo" && excluded_atom_ids.contains(&e.to);
            !from_bad && !to_bad
        });
    }
    let _ = schema::HARNESS_GRAPH_ID;
    build_load_ndjson(&filtered)
}
