//! Unit and integration tests for Omnigraph schema, HTTP 0.14 client,
//! sidecar lifecycle manager, and `OmnigraphDomain`.

use super::{
    client::{
        BranchCreateRequest, BranchDiffQuery, BranchMergeOutcome, BranchMergeRequest,
        ChangeBaselineRequest, ChangeFeedQuery, ChangeRequest, CommitListQuery, DiffQuery,
        MergeConflictKind, OmnigraphClient, QueryRequest, StoredQueryRequest,
    },
    schema, session_branch_name,
    sidecar::{OmnigraphConfig, OmnigraphDomain, OmnigraphManager, OmnigraphMode},
};
use crate::services::Domain;
use axum::{
    Router,
    body::Body,
    extract::{Path as AxumPath, Query as AxumQuery, State},
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::net::TcpListener;

#[derive(Clone, Default)]
struct MockSnapshot {
    sessions: BTreeMap<String, Value>,
    atoms: BTreeMap<String, Value>,
    files: BTreeMap<String, Value>,
    symbols: BTreeMap<String, Value>,
    turn_snapshots: BTreeMap<String, Value>,
    published_by: BTreeMap<String, Value>,
    consumed_by: BTreeMap<String, Value>,
    touches_file: BTreeMap<String, Value>,
    mentions_symbol: BTreeMap<String, Value>,
    relates_to: BTreeMap<String, Value>,
    derived_from: BTreeMap<String, Value>,
    session_follows: BTreeMap<String, Value>,
}

#[derive(Clone)]
struct MockCommit {
    id: String,
    branch: String,
    parent: Option<String>,
    merged_parent: Option<String>,
    actor_id: Option<String>,
    created_at: i64,
    snapshot: MockSnapshot,
    changes: Vec<Value>,
}

#[derive(Clone)]
struct MockBranch {
    name: String,
    head_commit: String,
    from: Option<String>,
    from_commit_id: Option<String>,
    created_at: i64,
    updated_at: i64,
}

struct MockGraphState {
    graph_id: String,
    tokens: BTreeMap<String, String>, // token -> actor_id
    branches: BTreeMap<String, MockBranch>,
    commits: BTreeMap<String, MockCommit>,
    commit_order: Vec<String>,
    next_seq: u64,
    expired_cursor: Option<String>,
}

impl MockGraphState {
    fn new(graph_id: &str) -> Self {
        let mut tokens = BTreeMap::new();
        tokens.insert("tok-harness".into(), "harness".into());
        tokens.insert("tok-bob".into(), "bob".into());

        let init_commit = MockCommit {
            id: "c-0001".into(),
            branch: "main".into(),
            parent: None,
            merged_parent: None,
            actor_id: Some("harness".into()),
            created_at: 1_700_000_000_000_000,
            snapshot: MockSnapshot::default(),
            changes: Vec::new(),
        };
        let mut commits = BTreeMap::new();
        commits.insert(init_commit.id.clone(), init_commit);

        let mut branches = BTreeMap::new();
        branches.insert(
            "main".into(),
            MockBranch {
                name: "main".into(),
                head_commit: "c-0001".into(),
                from: None,
                from_commit_id: None,
                created_at: 1_700_000_000_000_000,
                updated_at: 1_700_000_000_000_000,
            },
        );

        Self {
            graph_id: graph_id.into(),
            tokens,
            branches,
            commits,
            commit_order: vec!["c-0001".into()],
            next_seq: 2,
            expired_cursor: Some("expired-cursor".into()),
        }
    }

    fn alloc_commit_id(&mut self) -> String {
        let id = format!("c-{:04}", self.next_seq);
        self.next_seq += 1;
        id
    }

    fn ancestors(&self, start: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut queue = std::collections::VecDeque::new();
        let mut seen = BTreeSet::new();
        queue.push_back(start.to_string());
        while let Some(cid) = queue.pop_front() {
            if !seen.insert(cid.clone()) {
                continue;
            }
            out.push(cid.clone());
            if let Some(c) = self.commits.get(&cid) {
                if let Some(p) = &c.parent {
                    queue.push_back(p.clone());
                }
                if let Some(mp) = &c.merged_parent {
                    queue.push_back(mp.clone());
                }
            }
        }
        out
    }

    fn lca(&self, a: &str, b: &str) -> String {
        let a_set: BTreeSet<String> = self.ancestors(a).into_iter().collect();
        for anc in self.ancestors(b) {
            if a_set.contains(&anc) {
                return anc;
            }
        }
        "c-0001".into()
    }
}

#[derive(Clone)]
struct MockServer {
    state: Arc<Mutex<MockGraphState>>,
}

fn json_err(status: StatusCode, code: &str, msg: &str, extra: Option<Value>) -> Response {
    let mut body = json!({
        "error": msg,
        "code": code,
    });
    if let (Some(obj), Some(ext)) = (
        body.as_object_mut(),
        extra.and_then(|v| v.as_object().cloned()),
    ) {
        for (k, v) in ext {
            obj.insert(k, v);
        }
    }
    (status, axum::Json(body)).into_response()
}

fn check_auth_and_contract(
    headers: &HeaderMap,
    st: &MockGraphState,
) -> std::result::Result<String, Box<Response>> {
    let api_ver = headers
        .get("Omnigraph-Http-Api")
        .and_then(|v| v.to_str().ok());
    if api_ver != Some("0.14") {
        return Err(Box::new(json_err(
            StatusCode::BAD_REQUEST,
            "api_contract_mismatch",
            "Missing or invalid Omnigraph-Http-Api: 0.14 header.",
            None,
        )));
    }
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    let Some(token) = auth else {
        return Err(Box::new(json_err(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Missing Bearer token.",
            None,
        )));
    };
    let Some(actor_id) = st.tokens.get(token).cloned() else {
        return Err(Box::new(json_err(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Invalid Bearer token.",
            None,
        )));
    };
    if let Some(expected_actor) = headers
        .get("Omnigraph-If-Actor")
        .and_then(|v| v.to_str().ok())
        && expected_actor != actor_id
    {
        return Err(Box::new(json_err(
            StatusCode::PRECONDITION_FAILED,
            "actor_mismatch",
            &format!("Expected actor '{expected_actor}', authenticated as '{actor_id}'."),
            None,
        )));
    }
    Ok(actor_id)
}

async fn mock_healthz() -> Response {
    axum::Json(json!({
        "status": "ok",
        "version": "0.15.0",
        "internal_schema_version": 16,
        "source_version": "test-sha"
    }))
    .into_response()
}

async fn mock_readyz(State(srv): State<MockServer>) -> Response {
    let _st = srv.state.lock().unwrap();
    axum::Json(json!({
        "ready": true,
        "status": "serving",
        "served_graph_count": 1,
        "ready_graph_count": 1
    }))
    .into_response()
}

async fn mock_graphs(State(srv): State<MockServer>, headers: HeaderMap) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(resp) = check_auth_and_contract(&headers, &st) {
        return *resp;
    }
    axum::Json(json!({
        "graphs": [{
            "graph_id": st.graph_id,
            "state": "ready",
            "read_available": true,
            "write_available": true
        }]
    }))
    .into_response()
}

async fn mock_schema(
    State(srv): State<MockServer>,
    AxumPath(graph_id): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(resp) = check_auth_and_contract(&headers, &st) {
        return *resp;
    }
    if graph_id != st.graph_id {
        return json_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    axum::Json(json!({
        "schema_source": schema::HARNESS_CONTEXT_PG,
        "system_columns": { "id": "__id", "src": "__src", "dst": "__dst" }
    }))
    .into_response()
}

async fn mock_branches_list(
    State(srv): State<MockServer>,
    AxumPath(graph_id): AxumPath<String>,
    AxumQuery(q): AxumQuery<BTreeMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(resp) = check_auth_and_contract(&headers, &st) {
        return *resp;
    }
    if graph_id != st.graph_id {
        return json_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    let branches: Vec<String> = st.branches.keys().cloned().collect();
    let detailed = q.get("detailed").is_some_and(|v| v == "true");
    if !detailed {
        return axum::Json(json!({ "branches": branches })).into_response();
    }
    let details: Vec<Value> = st
        .branches
        .values()
        .map(|b| {
            let head = &st.commits[&b.head_commit];
            json!({
                "name": b.name,
                "head": {
                    "graph_commit_id": head.id,
                    "graph_branch": head.branch,
                    "graph_manifest_version": 1,
                    "parent_commit_id": head.parent,
                    "merged_parent_commit_id": head.merged_parent,
                    "actor_id": head.actor_id,
                    "created_at": head.created_at,
                    "published": true,
                    "kind": "published"
                },
                "from": b.from,
                "from_commit_id": b.from_commit_id,
                "created_at": b.created_at,
                "updated_at": b.updated_at
            })
        })
        .collect();
    axum::Json(json!({ "branches": branches, "details": details })).into_response()
}

async fn mock_branches_create(
    State(srv): State<MockServer>,
    AxumPath(graph_id): AxumPath<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let mut st = srv.state.lock().unwrap();
    let actor = match check_auth_and_contract(&headers, &st) {
        Ok(a) => a,
        Err(r) => return *r,
    };
    if graph_id != st.graph_id {
        return json_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    let name = body["name"].as_str().unwrap_or("").trim().to_string();
    if name.is_empty() {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "Missing branch name.",
            None,
        );
    }
    if st.branches.contains_key(&name) {
        return json_err(
            StatusCode::CONFLICT,
            "conflict",
            &format!("Branch '{name}' already exists."),
            None,
        );
    }
    let (from, from_commit_id) = if let Some(cid) = body["from_commit"].as_str() {
        let Some(c) = st.commits.get(cid) else {
            return json_err(
                StatusCode::NOT_FOUND,
                "not_found",
                "Commit not found.",
                None,
            );
        };
        (c.branch.clone(), c.id.clone())
    } else {
        let from_name = body["from"].as_str().unwrap_or("main").to_string();
        let Some(src) = st.branches.get(&from_name) else {
            return json_err(
                StatusCode::NOT_FOUND,
                "not_found",
                "Parent branch not found.",
                None,
            );
        };
        (from_name, src.head_commit.clone())
    };
    let ts = 1_700_000_000_000_000 + (st.next_seq as i64) * 1000;
    st.branches.insert(
        name.clone(),
        MockBranch {
            name: name.clone(),
            head_commit: from_commit_id.clone(),
            from: Some(from.clone()),
            from_commit_id: Some(from_commit_id.clone()),
            created_at: ts,
            updated_at: ts,
        },
    );
    axum::Json(json!({
        "uri": format!("/graphs/{graph_id}/branches/{name}"),
        "from": from,
        "name": name,
        "actor_id": actor,
        "from_commit_id": from_commit_id
    }))
    .into_response()
}

async fn mock_branches_delete(
    State(srv): State<MockServer>,
    AxumPath((graph_id, branch_name)): AxumPath<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let mut st = srv.state.lock().unwrap();
    let actor = match check_auth_and_contract(&headers, &st) {
        Ok(a) => a,
        Err(r) => return *r,
    };
    if graph_id != st.graph_id {
        return json_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    if branch_name == "main" {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "Cannot delete main.",
            None,
        );
    }
    if st.branches.remove(&branch_name).is_none() {
        return json_err(
            StatusCode::NOT_FOUND,
            "not_found",
            "Branch not found.",
            None,
        );
    }
    axum::Json(json!({
        "uri": format!("/graphs/{graph_id}/branches/{branch_name}"),
        "name": branch_name,
        "actor_id": actor
    }))
    .into_response()
}

fn diff_snapshots(base: &MockSnapshot, head: &MockSnapshot) -> Vec<Value> {
    let mut changes = Vec::new();
    for (id, val) in &head.atoms {
        match base.atoms.get(id) {
            None => changes.push(json!({
                "kind": "node",
                "type": { "id": "t-atom", "name": "ContextAtom" },
                "id": id,
                "op": "insert",
                "after": { "properties": val }
            })),
            Some(old) if old != val => changes.push(json!({
                "kind": "node",
                "type": { "id": "t-atom", "name": "ContextAtom" },
                "id": id,
                "op": "update",
                "before": { "properties": old },
                "after": { "properties": val }
            })),
            _ => {}
        }
    }
    for (id, old) in &base.atoms {
        if !head.atoms.contains_key(id) {
            changes.push(json!({
                "kind": "node",
                "type": { "id": "t-atom", "name": "ContextAtom" },
                "id": id,
                "op": "delete",
                "before": { "properties": old }
            }));
        }
    }
    for (edge_type, base_map, head_map) in [
        ("PublishedBy", &base.published_by, &head.published_by),
        ("ConsumedBy", &base.consumed_by, &head.consumed_by),
        ("TouchesFile", &base.touches_file, &head.touches_file),
        ("RelatesTo", &base.relates_to, &head.relates_to),
    ] {
        for (eid, val) in head_map {
            if base_map.get(eid) != Some(val) {
                let op = if base_map.contains_key(eid) {
                    "update"
                } else {
                    "insert"
                };
                changes.push(json!({
                    "kind": "edge",
                    "type": { "id": format!("t-{edge_type}"), "name": edge_type },
                    "id": eid,
                    "op": op,
                    "after": {
                        "properties": val,
                        "endpoints": {
                            "from": val["source_id"],
                            "to": val["target_id"]
                        }
                    }
                }));
            }
        }
        for (eid, old) in base_map {
            if !head_map.contains_key(eid) {
                changes.push(json!({
                    "kind": "edge",
                    "type": { "id": format!("t-{edge_type}"), "name": edge_type },
                    "id": eid,
                    "op": "delete",
                    "before": {
                        "properties": old,
                        "endpoints": {
                            "from": old["source_id"],
                            "to": old["target_id"]
                        }
                    }
                }));
            }
        }
    }
    changes
}

async fn mock_branches_diff(
    State(srv): State<MockServer>,
    AxumPath(graph_id): AxumPath<String>,
    AxumQuery(q): AxumQuery<BTreeMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(resp) = check_auth_and_contract(&headers, &st) {
        return *resp;
    }
    if graph_id != st.graph_id {
        return json_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    // Enforce exact Omnigraph 0.14 query parameter rule (`source` + `target`, NOT `from`!)
    if q.contains_key("from") || q.contains_key("to") {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "unknown query parameter 'from'; use 'source' and 'target'.",
            None,
        );
    }
    let Some(source) = q.get("source") else {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "Missing source.",
            None,
        );
    };
    let target = q.get("target").map(String::as_str).unwrap_or("main");
    let (Some(src_b), Some(tgt_b)) = (st.branches.get(source), st.branches.get(target)) else {
        return json_err(
            StatusCode::NOT_FOUND,
            "not_found",
            "Branch not found.",
            None,
        );
    };
    let base_id = st.lca(&src_b.head_commit, &tgt_b.head_commit);
    let base_snap = &st.commits[&base_id].snapshot;
    let src_snap = &st.commits[&src_b.head_commit].snapshot;
    let changes = diff_snapshots(base_snap, src_snap);
    axum::Json(json!({
        "from_commit_id": base_id,
        "to_commit_id": src_b.head_commit,
        "base_commit_id": base_id,
        "changes": changes
    }))
    .into_response()
}

fn three_way_merge(
    base: &MockSnapshot,
    source: &MockSnapshot,
    target: &MockSnapshot,
) -> std::result::Result<MockSnapshot, Vec<Value>> {
    let mut merged = target.clone();
    let mut conflicts = Vec::new();
    let all_atom_ids: BTreeSet<String> = base
        .atoms
        .keys()
        .chain(source.atoms.keys())
        .chain(target.atoms.keys())
        .cloned()
        .collect();
    for id in all_atom_ids {
        let b = base.atoms.get(&id);
        let s = source.atoms.get(&id);
        let t = target.atoms.get(&id);
        if s == b {
            continue;
        }
        if t == b {
            match s {
                Some(sv) => {
                    merged.atoms.insert(id, sv.clone());
                }
                None => {
                    merged.atoms.remove(&id);
                }
            }
            continue;
        }
        if s == t {
            continue;
        }
        let kind = match (b, s, t) {
            (None, Some(_), Some(_)) => "divergent_insert",
            (Some(_), Some(_), Some(_)) => "divergent_update",
            (Some(_), None, Some(_)) | (Some(_), Some(_), None) => "delete_vs_update",
            _ => "divergent_update",
        };
        conflicts.push(json!({
            "entity_kind": "node",
            "type_name": "ContextAtom",
            "entity_id": id,
            "kind": kind,
            "message": format!("Merge conflict ({kind}) on ContextAtom '{id}'.")
        }));
    }
    if !conflicts.is_empty() {
        return Err(conflicts);
    }
    for (id, v) in &source.sessions {
        merged
            .sessions
            .entry(id.clone())
            .or_insert_with(|| v.clone());
    }
    for (id, v) in &source.files {
        merged.files.entry(id.clone()).or_insert_with(|| v.clone());
    }
    for (id, v) in &source.symbols {
        merged
            .symbols
            .entry(id.clone())
            .or_insert_with(|| v.clone());
    }
    for (id, v) in &source.turn_snapshots {
        merged
            .turn_snapshots
            .entry(id.clone())
            .or_insert_with(|| v.clone());
    }
    for (id, v) in &source.published_by {
        merged.published_by.insert(id.clone(), v.clone());
    }
    for (id, v) in &source.consumed_by {
        merged.consumed_by.insert(id.clone(), v.clone());
    }
    for (id, v) in &source.touches_file {
        merged.touches_file.insert(id.clone(), v.clone());
    }
    for (id, v) in &source.mentions_symbol {
        merged.mentions_symbol.insert(id.clone(), v.clone());
    }
    for (id, v) in &source.relates_to {
        merged.relates_to.insert(id.clone(), v.clone());
    }
    for (id, v) in &source.derived_from {
        merged.derived_from.insert(id.clone(), v.clone());
    }
    for (id, v) in &source.session_follows {
        merged.session_follows.insert(id.clone(), v.clone());
    }
    Ok(merged)
}

async fn mock_branches_merge(
    State(srv): State<MockServer>,
    AxumPath(graph_id): AxumPath<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let mut st = srv.state.lock().unwrap();
    let actor = match check_auth_and_contract(&headers, &st) {
        Ok(a) => a,
        Err(r) => return *r,
    };
    if graph_id != st.graph_id {
        return json_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    let source = body["source"].as_str().unwrap_or("").to_string();
    let target = body["target"].as_str().unwrap_or("main").to_string();
    let dry_run = body["dry_run"].as_bool().unwrap_or(false);
    let delete_branch = body["delete_branch"].as_bool().unwrap_or(false);

    let (Some(src_b), Some(tgt_b)) = (
        st.branches.get(&source).cloned(),
        st.branches.get(&target).cloned(),
    ) else {
        return json_err(
            StatusCode::NOT_FOUND,
            "not_found",
            "Branch not found.",
            None,
        );
    };

    let base_id = st.lca(&src_b.head_commit, &tgt_b.head_commit);
    if src_b.head_commit == tgt_b.head_commit || base_id == src_b.head_commit {
        return axum::Json(json!({
            "source": source,
            "target": target,
            "outcome": "already_up_to_date",
            "commit": Value::Null,
            "actor_id": actor,
            "dry_run": dry_run
        }))
        .into_response();
    }

    let base_snap = st.commits[&base_id].snapshot.clone();
    let src_snap = st.commits[&src_b.head_commit].snapshot.clone();
    let tgt_snap = st.commits[&tgt_b.head_commit].snapshot.clone();

    let outcome = if base_id == tgt_b.head_commit {
        "fast_forward"
    } else {
        "merged"
    };

    let merged_snap = match three_way_merge(&base_snap, &src_snap, &tgt_snap) {
        Ok(snap) => snap,
        Err(conflicts) => {
            return json_err(
                StatusCode::CONFLICT,
                "conflict",
                "Merge conflict detected.",
                Some(json!({ "merge_conflicts": conflicts })),
            );
        }
    };

    if dry_run {
        return axum::Json(json!({
            "source": source,
            "target": target,
            "outcome": outcome,
            "commit": Value::Null,
            "actor_id": actor,
            "dry_run": true
        }))
        .into_response();
    }

    let cid = st.alloc_commit_id();
    let ts = 1_700_000_000_000_000 + (st.next_seq as i64) * 1000;
    let changes = diff_snapshots(&tgt_snap, &merged_snap);
    let commit = MockCommit {
        id: cid.clone(),
        branch: target.clone(),
        parent: Some(tgt_b.head_commit.clone()),
        merged_parent: Some(src_b.head_commit.clone()),
        actor_id: Some(actor.clone()),
        created_at: ts,
        snapshot: merged_snap,
        changes,
    };
    let commit_json = json!({
        "graph_commit_id": commit.id,
        "graph_branch": commit.branch,
        "graph_manifest_version": 1,
        "parent_commit_id": commit.parent,
        "merged_parent_commit_id": commit.merged_parent,
        "actor_id": commit.actor_id,
        "created_at": commit.created_at,
        "published": true,
        "kind": "published"
    });
    st.commits.insert(cid.clone(), commit);
    st.commit_order.push(cid.clone());
    if let Some(tb) = st.branches.get_mut(&target) {
        tb.head_commit = cid;
        tb.updated_at = ts;
    }
    let branch_deleted = if delete_branch && source != "main" {
        Some(st.branches.remove(&source).is_some())
    } else {
        None
    };

    axum::Json(json!({
        "source": source,
        "target": target,
        "outcome": outcome,
        "commit": commit_json,
        "actor_id": actor,
        "branch_deleted": branch_deleted,
        "dry_run": false
    }))
    .into_response()
}

async fn mock_mutate(
    State(srv): State<MockServer>,
    AxumPath(graph_id): AxumPath<String>,
    headers: HeaderMap,
    req: Request<Body>,
) -> Response {
    let cas = req.uri().path().ends_with("/if-graph-commit");
    let bytes = match axum::body::to_bytes(req.into_body(), 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return json_err(StatusCode::BAD_REQUEST, "bad_request", "Bad body.", None),
    };
    let body: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => {
            return json_err(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "Invalid JSON.",
                None,
            );
        }
    };

    let mut st = srv.state.lock().unwrap();
    let actor = match check_auth_and_contract(&headers, &st) {
        Ok(a) => a,
        Err(r) => return *r,
    };
    if graph_id != st.graph_id {
        return json_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }

    let query_text = body["query"].as_str().unwrap_or("");
    if query_text.trim_start().starts_with("mutation") {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "parse error: expected query_file",
            None,
        );
    }
    let branch_name = body["branch"].as_str().unwrap_or("main").to_string();
    let Some(branch) = st.branches.get(&branch_name).cloned() else {
        return json_err(
            StatusCode::NOT_FOUND,
            "not_found",
            "Branch not found.",
            None,
        );
    };

    let if_commit = headers
        .get("Omnigraph-If-Graph-Commit")
        .and_then(|v| v.to_str().ok());
    if cas && if_commit.is_none() {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "Missing Omnigraph-If-Graph-Commit header.",
            None,
        );
    }
    if let Some(expected) = if_commit
        && expected != branch.head_commit
    {
        return json_err(
            StatusCode::PRECONDITION_FAILED,
            "precondition_failed",
            "Graph commit precondition failed.",
            Some(json!({
                "precondition_failure": {
                    "expected": expected,
                    "actual": branch.head_commit
                }
            })),
        );
    }

    let params = &body["params"];
    let mut snap = st.commits[&branch.head_commit].snapshot.clone();
    let mut changes = Vec::new();
    let mut affected_nodes = 0usize;
    let mut affected_edges = 0usize;
    let query_name = body["name"].as_str().unwrap_or("mutate").to_string();

    if query_text.contains("insert SessionFollows") || query_name == "link_session_follows" {
        let src = params["from_session_id"].as_str().unwrap_or("");
        let dst = params["to_session_id"].as_str().unwrap_or("");
        let mut props = params.clone();
        props["source_id"] = json!(src);
        props["target_id"] = json!(dst);
        snap.session_follows.insert(format!("{src}->{dst}"), props);
        affected_edges += 1;
    } else if query_text.contains("insert Session ")
        || query_text.contains("insert Session{")
        || query_name == "upsert_session"
    {
        let sid = params["session_id"].as_str().unwrap_or("s1").to_string();
        snap.sessions.insert(sid.clone(), params.clone());
        affected_nodes += 1;
        changes.push(json!({
            "kind": "node",
            "type": { "id": "t-sess", "name": "Session" },
            "id": sid,
            "op": "insert",
            "after": { "properties": params }
        }));
    } else if query_text.contains("insert ContextAtom") || query_name == "upsert_atom" {
        let aid = params["atom_id"].as_str().unwrap_or("a1").to_string();
        let prev = snap.atoms.insert(aid.clone(), params.clone());
        affected_nodes += 1;
        changes.push(json!({
            "kind": "node",
            "type": { "id": "t-atom", "name": "ContextAtom" },
            "id": aid,
            "op": if prev.is_some() { "update" } else { "insert" },
            "before": prev.map(|p| json!({ "properties": p })),
            "after": { "properties": params }
        }));
    } else if query_text.contains("delete ContextAtom") {
        let aid = params["atom_id"].as_str().unwrap_or("").to_string();
        // As `GQ_DELETE_CANONICAL_ATOM`: every edge naming the atom goes too.
        let named = |key: &String, props: &Value| {
            props["source_id"].as_str() == Some(aid.as_str())
                || props["target_id"].as_str() == Some(aid.as_str())
                || key.starts_with(&format!("{aid}->"))
                || key.starts_with(&format!("{aid}:"))
                || key.ends_with(&format!(":{aid}"))
        };
        for edges in [
            &mut snap.published_by,
            &mut snap.consumed_by,
            &mut snap.touches_file,
            &mut snap.mentions_symbol,
            &mut snap.relates_to,
            &mut snap.derived_from,
        ] {
            let before = edges.len();
            edges.retain(|key, props| !named(key, props));
            affected_edges += before - edges.len();
        }
        if let Some(prev) = snap.atoms.remove(&aid) {
            affected_nodes += 1;
            changes.push(json!({
                "kind": "node",
                "type": { "id": "t-atom", "name": "ContextAtom" },
                "id": aid,
                "op": "delete",
                "before": { "properties": prev }
            }));
        }
    } else if query_text.contains("insert FileRef") || query_name == "upsert_file" {
        let path = params["path"].as_str().unwrap_or("file.rs").to_string();
        snap.files.insert(path.clone(), params.clone());
        affected_nodes += 1;
    } else if query_text.contains("insert TurnSnapshot") {
        let sid = params["snapshot_id"]
            .as_str()
            .unwrap_or("snap1")
            .to_string();
        snap.turn_snapshots.insert(sid, params.clone());
        affected_nodes += 1;
    } else if query_text.contains("insert PublishedBy") {
        let src = params["atom_id"].as_str().unwrap_or("");
        let dst = params["session_id"].as_str().unwrap_or("");
        let eid = format!("{src}->{dst}");
        let mut props = params.clone();
        props["source_id"] = json!(src);
        props["target_id"] = json!(dst);
        snap.published_by.insert(eid.clone(), props.clone());
        affected_edges += 1;
        changes.push(json!({
            "kind": "edge",
            "type": { "id": "t-pub", "name": "PublishedBy" },
            "id": eid,
            "op": "insert",
            "after": { "properties": props, "endpoints": { "from": src, "to": dst } }
        }));
    } else if query_text.contains("insert ConsumedBy") {
        let src = params["atom_id"].as_str().unwrap_or("");
        let dst = params["session_id"].as_str().unwrap_or("");
        let eid = format!("{src}->{dst}");
        let mut props = params.clone();
        props["source_id"] = json!(src);
        props["target_id"] = json!(dst);
        snap.consumed_by.insert(eid.clone(), props.clone());
        affected_edges += 1;
        changes.push(json!({
            "kind": "edge",
            "type": { "id": "t-con", "name": "ConsumedBy" },
            "id": eid,
            "op": "insert",
            "after": { "properties": props, "endpoints": { "from": src, "to": dst } }
        }));
    } else if query_text.contains("insert TouchesFile") {
        let src = params["atom_id"].as_str().unwrap_or("");
        let dst = params["path"].as_str().unwrap_or("");
        let eid = format!("{src}->{dst}");
        let mut props = params.clone();
        props["source_id"] = json!(src);
        props["target_id"] = json!(dst);
        snap.touches_file.insert(eid.clone(), props.clone());
        affected_edges += 1;
        changes.push(json!({
            "kind": "edge",
            "type": { "id": "t-tf", "name": "TouchesFile" },
            "id": eid,
            "op": "insert",
            "after": { "properties": props, "endpoints": { "from": src, "to": dst } }
        }));
    } else if query_text.contains("insert RelatesTo") {
        let src = params["source_id"].as_str().unwrap_or("");
        let dst = params["target_id"].as_str().unwrap_or("");
        let rel = params["relation"].as_str().unwrap_or("supports");
        let eid = format!("{src}:{rel}:{dst}");
        snap.relates_to.insert(eid.clone(), params.clone());
        affected_edges += 1;
        changes.push(json!({
            "kind": "edge",
            "type": { "id": "t-rel", "name": "RelatesTo" },
            "id": eid,
            "op": "insert",
            "after": { "properties": params, "endpoints": { "from": src, "to": dst } }
        }));
    } else if query_text.contains("insert DerivedFrom") {
        let src = params["atom_id"].as_str().unwrap_or("");
        let dst = params["snapshot_id"].as_str().unwrap_or("");
        let mut props = params.clone();
        props["source_id"] = json!(src);
        props["target_id"] = json!(dst);
        snap.derived_from.insert(format!("{src}->{dst}"), props);
        affected_edges += 1;
    } else if query_text.contains("insert SymbolRef") {
        let fqn = params["fqn"].as_str().unwrap_or("sym").to_string();
        snap.symbols.insert(fqn, params.clone());
        affected_nodes += 1;
    } else if query_text.contains("insert MentionsSymbol") {
        let src = params["atom_id"].as_str().unwrap_or("");
        let dst = params["fqn"].as_str().unwrap_or("");
        let mut props = params.clone();
        props["source_id"] = json!(src);
        props["target_id"] = json!(dst);
        snap.mentions_symbol.insert(format!("{src}->{dst}"), props);
        affected_edges += 1;
    } else if query_name == "clear_branch_context" {
        affected_nodes = snap.atoms.len()
            + snap.files.len()
            + snap.symbols.len()
            + snap.sessions.len()
            + snap.turn_snapshots.len();
        affected_edges = snap.published_by.len()
            + snap.consumed_by.len()
            + snap.touches_file.len()
            + snap.mentions_symbol.len()
            + snap.relates_to.len()
            + snap.derived_from.len()
            + snap.session_follows.len();
        snap = MockSnapshot::default();
    }

    let cid = st.alloc_commit_id();
    let ts = 1_700_000_000_000_000 + (st.next_seq as i64) * 1000;
    let commit = MockCommit {
        id: cid.clone(),
        branch: branch_name.clone(),
        parent: Some(branch.head_commit),
        merged_parent: None,
        actor_id: Some(actor.clone()),
        created_at: ts,
        snapshot: snap,
        changes,
    };
    let commit_json = json!({
        "graph_commit_id": commit.id,
        "graph_branch": commit.branch,
        "graph_manifest_version": 1,
        "parent_commit_id": commit.parent,
        "merged_parent_commit_id": Value::Null,
        "actor_id": commit.actor_id,
        "created_at": commit.created_at,
        "published": true,
        "kind": "published"
    });
    st.commits.insert(cid.clone(), commit);
    st.commit_order.push(cid.clone());
    if let Some(b) = st.branches.get_mut(&branch_name) {
        b.head_commit = cid;
        b.updated_at = ts;
    }

    axum::Json(json!({
        "branch": branch_name,
        "query_name": query_name,
        "affected_nodes": affected_nodes,
        "affected_edges": affected_edges,
        "actor_id": actor,
        "commit": commit_json
    }))
    .into_response()
}

fn evaluate_mock_query(
    snap: &MockSnapshot,
    query_name: &str,
    query_text: &str,
    params: &Value,
) -> (Vec<Value>, Option<Value>) {
    let pid = params["project_id"].as_str().unwrap_or("");
    let match_proj = |v: &Value| pid.is_empty() || v["project_id"].as_str().unwrap_or("") == pid;

    if query_name == "context_search" || query_text.contains("context_search") {
        let q = params["q"].as_str().unwrap_or("").to_ascii_lowercase();
        let mut hits = Vec::new();
        for atom in snap.atoms.values().filter(|a| match_proj(a)) {
            let hay = format!(
                "{} {} {}",
                atom["title"].as_str().unwrap_or(""),
                atom["summary"].as_str().unwrap_or(""),
                atom["body"].as_str().unwrap_or("")
            )
            .to_ascii_lowercase();
            if q.is_empty() || hay.contains(&q) {
                let mut row = atom.clone();
                row["score"] = json!(0.95);
                hits.push(row);
            }
        }
        return (
            hits,
            Some(json!({
                "by": "rrf",
                "order": "descending",
                "exhaustive": true,
                "score_columns": ["score"]
            })),
        );
    }

    if query_name == "recent_atoms" || query_text.contains("recent_atoms") {
        let rows: Vec<Value> = snap
            .atoms
            .values()
            .filter(|a| match_proj(a))
            .cloned()
            .collect();
        return (rows, None);
    }
    if query_name == "list_sessions" || query_text.contains("list_sessions") {
        let rows: Vec<Value> = snap
            .sessions
            .values()
            .filter(|s| match_proj(s))
            .cloned()
            .collect();
        return (rows, None);
    }
    if query_name == "list_files" || query_text.contains("list_files") {
        let rows: Vec<Value> = snap
            .files
            .values()
            .filter(|f| match_proj(f))
            .cloned()
            .collect();
        return (rows, None);
    }
    if query_name == "list_published_edges" || query_text.contains("list_published_edges") {
        return (snap.published_by.values().cloned().collect(), None);
    }
    if query_name == "list_consumed_edges" || query_text.contains("list_consumed_edges") {
        return (snap.consumed_by.values().cloned().collect(), None);
    }
    if query_name == "list_file_edges" || query_text.contains("list_file_edges") {
        return (snap.touches_file.values().cloned().collect(), None);
    }
    if query_name == "list_atom_relations" || query_text.contains("list_atom_relations") {
        return (snap.relates_to.values().cloned().collect(), None);
    }
    if query_name == "list_session_follows" || query_text.contains("list_session_follows") {
        return (snap.session_follows.values().cloned().collect(), None);
    }
    if query_name == "contradictions" || query_text.contains("contradictions") {
        let rows: Vec<Value> = snap
            .relates_to
            .values()
            .filter(|e| e["relation"].as_str() == Some("contradicts"))
            .cloned()
            .collect();
        return (rows, None);
    }
    (Vec::new(), None)
}

async fn mock_query(
    State(srv): State<MockServer>,
    AxumPath(graph_id): AxumPath<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(resp) = check_auth_and_contract(&headers, &st) {
        return *resp;
    }
    if graph_id != st.graph_id {
        return json_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    let (commit_id, branch_out, snap_out) = if let Some(snap_id) = body["snapshot"].as_str() {
        (snap_id.to_string(), None, Some(snap_id.to_string()))
    } else {
        let bname = body["branch"].as_str().unwrap_or("main");
        let Some(b) = st.branches.get(bname) else {
            return json_err(
                StatusCode::NOT_FOUND,
                "not_found",
                "Branch not found.",
                None,
            );
        };
        (b.head_commit.clone(), Some(bname.to_string()), None)
    };
    let Some(commit) = st.commits.get(&commit_id) else {
        return json_err(
            StatusCode::NOT_FOUND,
            "not_found",
            "Snapshot commit not found.",
            None,
        );
    };
    let qname = body["name"].as_str().unwrap_or("inline_query");
    let qtext = body["query"].as_str().unwrap_or("");
    let (rows, ranking) = evaluate_mock_query(&commit.snapshot, qname, qtext, &body["params"]);
    axum::Json(json!({
        "query_name": qname,
        "target": { "branch": branch_out, "snapshot": snap_out },
        "row_count": rows.len(),
        "columns": [],
        "rows": rows,
        "graph_commit_id": commit_id,
        "ranking": ranking
    }))
    .into_response()
}

async fn mock_stored_query(
    State(srv): State<MockServer>,
    AxumPath((graph_id, query_name)): AxumPath<(String, String)>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(resp) = check_auth_and_contract(&headers, &st) {
        return *resp;
    }
    if graph_id != st.graph_id {
        return json_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    if !schema::STORED_QUERIES.iter().any(|q| q.name == query_name) {
        return json_err(
            StatusCode::NOT_FOUND,
            "not_found",
            "Stored query not found.",
            None,
        );
    }
    let (commit_id, branch_out, snap_out) = if let Some(snap_id) = body["snapshot"].as_str() {
        (snap_id.to_string(), None, Some(snap_id.to_string()))
    } else {
        let bname = body["branch"].as_str().unwrap_or("main");
        let Some(b) = st.branches.get(bname) else {
            return json_err(
                StatusCode::NOT_FOUND,
                "not_found",
                "Branch not found.",
                None,
            );
        };
        (b.head_commit.clone(), Some(bname.to_string()), None)
    };
    let Some(commit) = st.commits.get(&commit_id) else {
        return json_err(
            StatusCode::NOT_FOUND,
            "not_found",
            "Snapshot commit not found.",
            None,
        );
    };
    let (rows, ranking) = evaluate_mock_query(&commit.snapshot, &query_name, "", &body["params"]);
    axum::Json(json!({
        "query_name": query_name,
        "target": { "branch": branch_out, "snapshot": snap_out },
        "row_count": rows.len(),
        "columns": [],
        "rows": rows,
        "graph_commit_id": commit_id,
        "ranking": ranking
    }))
    .into_response()
}

async fn mock_commits(
    State(srv): State<MockServer>,
    AxumPath(graph_id): AxumPath<String>,
    AxumQuery(q): AxumQuery<BTreeMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(resp) = check_auth_and_contract(&headers, &st) {
        return *resp;
    }
    if graph_id != st.graph_id {
        return json_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    let bname = q.get("branch").map(String::as_str).unwrap_or("main");
    let Some(branch) = st.branches.get(bname) else {
        return json_err(
            StatusCode::NOT_FOUND,
            "not_found",
            "Branch not found.",
            None,
        );
    };
    let mut commits = Vec::new();
    let mut cur = Some(branch.head_commit.clone());
    while let Some(cid) = cur {
        if let Some(c) = st.commits.get(&cid) {
            commits.push(json!({
                "graph_commit_id": c.id,
                "graph_branch": c.branch,
                "graph_manifest_version": 1,
                "parent_commit_id": c.parent,
                "merged_parent_commit_id": c.merged_parent,
                "actor_id": c.actor_id,
                "created_at": c.created_at,
                "published": true,
                "kind": "published"
            }));
            cur = c.parent.clone();
        } else {
            break;
        }
    }
    axum::Json(json!({
        "commits": commits,
        "head_commit_id": branch.head_commit
    }))
    .into_response()
}

async fn mock_commit_changes(
    State(srv): State<MockServer>,
    AxumPath((graph_id, commit_id)): AxumPath<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(resp) = check_auth_and_contract(&headers, &st) {
        return *resp;
    }
    if graph_id != st.graph_id {
        return json_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    let Some(c) = st.commits.get(&commit_id) else {
        return json_err(
            StatusCode::NOT_FOUND,
            "not_found",
            "Commit not found.",
            None,
        );
    };
    if c.parent.is_none() {
        return json_err(
            StatusCode::CONFLICT,
            "conflict",
            "Parentless commit cannot be diffed.",
            Some(json!({
                "change_diff_refusal": {
                    "reason": "parentless_commit",
                    "graph_commit_id": c.id
                }
            })),
        );
    }
    axum::Json(json!({
        "cause": {
            "graph_commit_id": c.id,
            "parent_commit_id": c.parent,
            "merged_parent_commit_id": c.merged_parent,
            "authored_branch": c.branch,
            "actor_id": c.actor_id,
            "authored_at": c.created_at
        },
        "changes": c.changes
    }))
    .into_response()
}

async fn mock_diff(
    State(srv): State<MockServer>,
    AxumPath(graph_id): AxumPath<String>,
    AxumQuery(q): AxumQuery<BTreeMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(resp) = check_auth_and_contract(&headers, &st) {
        return *resp;
    }
    if graph_id != st.graph_id {
        return json_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    let resolve = |target: &str| -> Option<String> {
        st.branches
            .get(target)
            .map(|b| b.head_commit.clone())
            .or_else(|| st.commits.get(target).map(|c| c.id.clone()))
    };
    let (Some(from_c), Some(to_c)) = (
        q.get("from").and_then(|s| resolve(s)),
        q.get("to").and_then(|s| resolve(s)),
    ) else {
        return json_err(
            StatusCode::NOT_FOUND,
            "not_found",
            "Diff target not found.",
            None,
        );
    };
    let changes = diff_snapshots(&st.commits[&from_c].snapshot, &st.commits[&to_c].snapshot);
    axum::Json(json!({
        "from_commit_id": from_c,
        "to_commit_id": to_c,
        "changes": changes
    }))
    .into_response()
}

async fn mock_changes(
    State(srv): State<MockServer>,
    AxumPath(graph_id): AxumPath<String>,
    AxumQuery(q): AxumQuery<BTreeMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(resp) = check_auth_and_contract(&headers, &st) {
        return *resp;
    }
    if graph_id != st.graph_id {
        return json_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    if q.get("cursor") == st.expired_cursor.as_ref() {
        return json_err(
            StatusCode::GONE,
            "change_feed_gap",
            "Change cursor fell behind retained history.",
            Some(json!({
                "change_feed_gap": {
                    "cursor": q.get("cursor"),
                    "first_unreadable_commit_id": "c-0001"
                }
            })),
        );
    }
    let after_idx = q
        .get("cursor")
        .and_then(|c| c.strip_prefix("cur-"))
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(0);
    let mut blocks = Vec::new();
    for cid in st.commit_order.iter().skip(after_idx) {
        let c = &st.commits[cid];
        if c.changes.is_empty() {
            continue;
        }
        blocks.push(json!({
            "cause": {
                "graph_commit_id": c.id,
                "parent_commit_id": c.parent,
                "merged_parent_commit_id": c.merged_parent,
                "authored_branch": c.branch,
                "actor_id": c.actor_id,
                "authored_at": c.created_at
            },
            "changes": c.changes
        }));
    }
    let next_cursor = format!("cur-{}", st.commit_order.len());
    axum::Json(json!({
        "blocks": blocks,
        "cursor": next_cursor,
        "caught_up": true
    }))
    .into_response()
}

async fn mock_changes_baseline(
    State(srv): State<MockServer>,
    AxumPath(graph_id): AxumPath<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(resp) = check_auth_and_contract(&headers, &st) {
        return *resp;
    }
    if graph_id != st.graph_id {
        return json_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    let bname = body["branch"].as_str().unwrap_or("main");
    let head = st
        .branches
        .get(bname)
        .map(|b| b.head_commit.clone())
        .unwrap_or_else(|| "c-0001".into());
    let mut lines = String::new();
    if let Some(c) = st.commits.get(&head) {
        for (aid, atom) in &c.snapshot.atoms {
            lines.push_str(&json!({ "type": "ContextAtom", "id": aid, "data": atom }).to_string());
            lines.push('\n');
        }
    }
    lines.push_str(
        &json!({
            "baseline": {
                "snapshot_commit_id": head,
                "resume_cursor": format!("cur-{}", st.commit_order.len())
            }
        })
        .to_string(),
    );
    lines.push('\n');
    (StatusCode::OK, lines).into_response()
}

fn export_mock_snapshot_ndjson(snap: &MockSnapshot, type_names: &[String]) -> String {
    let want = |name: &str| type_names.is_empty() || type_names.iter().any(|t| t == name);
    let mut lines = Vec::new();
    let strip_alias = |v: &Value| -> Value {
        let mut obj = v.as_object().cloned().unwrap_or_default();
        obj.remove("source_id");
        obj.remove("target_id");
        Value::Object(obj)
    };
    if want("ConsumedBy") {
        for v in snap.consumed_by.values() {
            lines.push(
                json!({
                    "edge": "ConsumedBy",
                    "id": format!("[\"{}\",\"{}\"]", v["source_id"].as_str().unwrap_or(""), v["target_id"].as_str().unwrap_or("")),
                    "from": v["source_id"],
                    "to": v["target_id"],
                    "data": strip_alias(v),
                })
                .to_string(),
            );
        }
    }
    if want("DerivedFrom") {
        for v in snap.derived_from.values() {
            lines.push(
                json!({
                    "edge": "DerivedFrom",
                    "id": format!("[\"{}\",\"{}\"]", v["source_id"].as_str().unwrap_or(""), v["target_id"].as_str().unwrap_or("")),
                    "from": v["source_id"],
                    "to": v["target_id"],
                    "data": strip_alias(v),
                })
                .to_string(),
            );
        }
    }
    if want("MentionsSymbol") {
        for v in snap.mentions_symbol.values() {
            lines.push(
                json!({
                    "edge": "MentionsSymbol",
                    "id": format!("[\"{}\",\"{}\"]", v["source_id"].as_str().unwrap_or(""), v["target_id"].as_str().unwrap_or("")),
                    "from": v["source_id"],
                    "to": v["target_id"],
                    "data": strip_alias(v),
                })
                .to_string(),
            );
        }
    }
    if want("PublishedBy") {
        for v in snap.published_by.values() {
            lines.push(
                json!({
                    "edge": "PublishedBy",
                    "id": format!("[\"{}\",\"{}\"]", v["source_id"].as_str().unwrap_or(""), v["target_id"].as_str().unwrap_or("")),
                    "from": v["source_id"],
                    "to": v["target_id"],
                    "data": strip_alias(v),
                })
                .to_string(),
            );
        }
    }
    if want("RelatesTo") {
        for v in snap.relates_to.values() {
            lines.push(
                json!({
                    "edge": "RelatesTo",
                    "id": format!("[\"{}\",\"{}\",\"{}\"]", v["source_id"].as_str().unwrap_or(""), v["target_id"].as_str().unwrap_or(""), v["relation"].as_str().unwrap_or("supports")),
                    "from": v["source_id"],
                    "to": v["target_id"],
                    "data": strip_alias(v),
                })
                .to_string(),
            );
        }
    }
    if want("SessionFollows") {
        for v in snap.session_follows.values() {
            lines.push(
                json!({
                    "edge": "SessionFollows",
                    "id": format!("[\"{}\",\"{}\"]", v["source_id"].as_str().unwrap_or(""), v["target_id"].as_str().unwrap_or("")),
                    "from": v["source_id"],
                    "to": v["target_id"],
                    "data": strip_alias(v),
                })
                .to_string(),
            );
        }
    }
    if want("TouchesFile") {
        for v in snap.touches_file.values() {
            lines.push(
                json!({
                    "edge": "TouchesFile",
                    "id": format!("[\"{}\",\"{}\"]", v["source_id"].as_str().unwrap_or(""), v["target_id"].as_str().unwrap_or("")),
                    "from": v["source_id"],
                    "to": v["target_id"],
                    "data": strip_alias(v),
                })
                .to_string(),
            );
        }
    }
    if want("ContextAtom") {
        for (id, v) in &snap.atoms {
            lines.push(json!({ "type": "ContextAtom", "id": id, "data": v }).to_string());
        }
    }
    if want("FileRef") {
        for (id, v) in &snap.files {
            lines.push(json!({ "type": "FileRef", "id": id, "data": v }).to_string());
        }
    }
    if want("Session") {
        for (id, v) in &snap.sessions {
            lines.push(json!({ "type": "Session", "id": id, "data": v }).to_string());
        }
    }
    if want("SymbolRef") {
        for (id, v) in &snap.symbols {
            lines.push(json!({ "type": "SymbolRef", "id": id, "data": v }).to_string());
        }
    }
    if want("TurnSnapshot") {
        for (id, v) in &snap.turn_snapshots {
            lines.push(json!({ "type": "TurnSnapshot", "id": id, "data": v }).to_string());
        }
    }
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

async fn mock_export(
    State(srv): State<MockServer>,
    AxumPath(graph_id): AxumPath<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(resp) = check_auth_and_contract(&headers, &st) {
        return *resp;
    }
    if graph_id != st.graph_id {
        return json_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    let bname = body["branch"].as_str().unwrap_or("main");
    let Some(branch) = st.branches.get(bname) else {
        return json_err(
            StatusCode::NOT_FOUND,
            "not_found",
            "Branch not found.",
            None,
        );
    };
    let type_names: Vec<String> = body["type_names"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    let snap = &st.commits[&branch.head_commit].snapshot;
    let body_text = export_mock_snapshot_ndjson(snap, &type_names);
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/x-ndjson")],
        body_text,
    )
        .into_response()
}

fn apply_mock_ndjson_load(
    st: &mut MockGraphState,
    actor: String,
    branch_name: &str,
    from: Option<&str>,
    mode: &str,
    ndjson: &str,
    include_uri: bool,
) -> Response {
    let mut branch_created = false;
    if !st.branches.contains_key(branch_name) {
        let Some(base_name) = from.filter(|s| !s.is_empty()) else {
            return json_err(
                StatusCode::NOT_FOUND,
                "not_found",
                "Target branch not found.",
                None,
            );
        };
        let Some(base_branch) = st.branches.get(base_name).cloned() else {
            return json_err(
                StatusCode::NOT_FOUND,
                "not_found",
                "Base branch not found.",
                None,
            );
        };
        let ts = 1_700_000_000_000_000 + (st.next_seq as i64) * 1000;
        st.branches.insert(
            branch_name.to_string(),
            MockBranch {
                name: branch_name.to_string(),
                head_commit: base_branch.head_commit.clone(),
                from: Some(base_name.to_string()),
                from_commit_id: Some(base_branch.head_commit),
                created_at: ts,
                updated_at: ts,
            },
        );
        branch_created = true;
    }

    let branch = st.branches[branch_name].clone();
    let mut snap = st.commits[&branch.head_commit].snapshot.clone();
    let mut node_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut edge_counts: BTreeMap<String, usize> = BTreeMap::new();

    let mut parsed_records = Vec::new();
    for line in ndjson.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return json_err(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "Invalid NDJSON line.",
                None,
            );
        };
        parsed_records.push(v);
    }

    if mode == "overwrite" {
        let mut present_types = BTreeSet::new();
        for rec in &parsed_records {
            if let Some(t) = rec.get("type").and_then(Value::as_str) {
                present_types.insert(t.to_string());
            } else if let Some(e) = rec.get("edge").and_then(Value::as_str) {
                present_types.insert(e.to_string());
            }
        }
        if present_types.contains("ContextAtom") {
            snap.atoms.clear();
        }
        if present_types.contains("FileRef") {
            snap.files.clear();
        }
        if present_types.contains("SymbolRef") {
            snap.symbols.clear();
        }
        if present_types.contains("Session") {
            snap.sessions.clear();
        }
        if present_types.contains("TurnSnapshot") {
            snap.turn_snapshots.clear();
        }
        if present_types.contains("PublishedBy") {
            snap.published_by.clear();
        }
        if present_types.contains("ConsumedBy") {
            snap.consumed_by.clear();
        }
        if present_types.contains("TouchesFile") {
            snap.touches_file.clear();
        }
        if present_types.contains("MentionsSymbol") {
            snap.mentions_symbol.clear();
        }
        if present_types.contains("RelatesTo") {
            snap.relates_to.clear();
        }
        if present_types.contains("DerivedFrom") {
            snap.derived_from.clear();
        }
        if present_types.contains("SessionFollows") {
            snap.session_follows.clear();
        }
    }

    let mut changes = Vec::new();
    for rec in &parsed_records {
        if let Some(t) = rec.get("type").and_then(Value::as_str) {
            let data = rec["data"].clone();
            match t {
                "ContextAtom" => {
                    let id = data["atom_id"]
                        .as_str()
                        .or_else(|| rec["id"].as_str())
                        .unwrap_or("")
                        .to_string();
                    if mode == "append" && snap.atoms.contains_key(&id) {
                        return json_err(
                            StatusCode::CONFLICT,
                            "conflict",
                            "Entity already exists.",
                            None,
                        );
                    }
                    let prev = snap.atoms.insert(id.clone(), data.clone());
                    changes.push(json!({
                        "kind": "node",
                        "type": { "id": "t-atom", "name": "ContextAtom" },
                        "id": id,
                        "op": if prev.is_some() { "update" } else { "insert" },
                        "after": { "properties": data }
                    }));
                }
                "FileRef" => {
                    let id = data["path"]
                        .as_str()
                        .or_else(|| rec["id"].as_str())
                        .unwrap_or("")
                        .to_string();
                    snap.files.insert(id, data);
                }
                "SymbolRef" => {
                    let id = data["fqn"]
                        .as_str()
                        .or_else(|| rec["id"].as_str())
                        .unwrap_or("")
                        .to_string();
                    snap.symbols.insert(id, data);
                }
                "Session" => {
                    let id = data["session_id"]
                        .as_str()
                        .or_else(|| rec["id"].as_str())
                        .unwrap_or("")
                        .to_string();
                    snap.sessions.insert(id, data);
                }
                "TurnSnapshot" => {
                    let id = data["snapshot_id"]
                        .as_str()
                        .or_else(|| rec["id"].as_str())
                        .unwrap_or("")
                        .to_string();
                    snap.turn_snapshots.insert(id, data);
                }
                _ => {
                    return json_err(
                        StatusCode::BAD_REQUEST,
                        "bad_request",
                        "Unknown node type.",
                        None,
                    );
                }
            }
            *node_counts.entry(t.to_string()).or_insert(0) += 1;
        }
    }

    for rec in &parsed_records {
        if let Some(e) = rec.get("edge").and_then(Value::as_str) {
            let from = rec["from"].as_str().unwrap_or("");
            let to = rec["to"].as_str().unwrap_or("");
            let mut props = rec["data"].as_object().cloned().unwrap_or_default();
            props.insert("source_id".into(), json!(from));
            props.insert("target_id".into(), json!(to));
            let val = Value::Object(props);
            match e {
                "PublishedBy" => {
                    if !snap.atoms.contains_key(from) || !snap.sessions.contains_key(to) {
                        return json_err(
                            StatusCode::BAD_REQUEST,
                            "bad_request",
                            "Dangling PublishedBy.",
                            None,
                        );
                    }
                    snap.published_by.insert(format!("{from}->{to}"), val);
                }
                "ConsumedBy" => {
                    if !snap.atoms.contains_key(from) || !snap.sessions.contains_key(to) {
                        return json_err(
                            StatusCode::BAD_REQUEST,
                            "bad_request",
                            "Dangling ConsumedBy.",
                            None,
                        );
                    }
                    snap.consumed_by.insert(format!("{from}->{to}"), val);
                }
                "TouchesFile" => {
                    if !snap.atoms.contains_key(from) || !snap.files.contains_key(to) {
                        return json_err(
                            StatusCode::BAD_REQUEST,
                            "bad_request",
                            "Dangling TouchesFile.",
                            None,
                        );
                    }
                    snap.touches_file.insert(format!("{from}->{to}"), val);
                }
                "MentionsSymbol" => {
                    if !snap.atoms.contains_key(from) || !snap.symbols.contains_key(to) {
                        return json_err(
                            StatusCode::BAD_REQUEST,
                            "bad_request",
                            "Dangling MentionsSymbol.",
                            None,
                        );
                    }
                    snap.mentions_symbol.insert(format!("{from}->{to}"), val);
                }
                "RelatesTo" => {
                    if !snap.atoms.contains_key(from) || !snap.atoms.contains_key(to) {
                        return json_err(
                            StatusCode::BAD_REQUEST,
                            "bad_request",
                            "Dangling RelatesTo.",
                            None,
                        );
                    }
                    let rel = val["relation"].as_str().unwrap_or("supports");
                    snap.relates_to.insert(format!("{from}:{rel}:{to}"), val);
                }
                "DerivedFrom" => {
                    if !snap.atoms.contains_key(from) || !snap.turn_snapshots.contains_key(to) {
                        return json_err(
                            StatusCode::BAD_REQUEST,
                            "bad_request",
                            "Dangling DerivedFrom.",
                            None,
                        );
                    }
                    snap.derived_from.insert(format!("{from}->{to}"), val);
                }
                "SessionFollows" => {
                    if !snap.sessions.contains_key(from) || !snap.sessions.contains_key(to) {
                        return json_err(
                            StatusCode::BAD_REQUEST,
                            "bad_request",
                            "Dangling SessionFollows.",
                            None,
                        );
                    }
                    snap.session_follows.insert(format!("{from}->{to}"), val);
                }
                _ => {
                    return json_err(
                        StatusCode::BAD_REQUEST,
                        "bad_request",
                        "Unknown edge type.",
                        None,
                    );
                }
            }
            *edge_counts.entry(e.to_string()).or_insert(0) += 1;
        }
    }

    let cid = st.alloc_commit_id();
    let ts = 1_700_000_000_000_000 + (st.next_seq as i64) * 1000;
    let commit = MockCommit {
        id: cid.clone(),
        branch: branch_name.to_string(),
        parent: Some(branch.head_commit),
        merged_parent: None,
        actor_id: Some(actor.clone()),
        created_at: ts,
        snapshot: snap,
        changes,
    };
    let commit_json = json!({
        "graph_commit_id": commit.id,
        "graph_branch": commit.branch,
        "graph_manifest_version": 1,
        "parent_commit_id": commit.parent,
        "merged_parent_commit_id": Value::Null,
        "actor_id": commit.actor_id,
        "created_at": commit.created_at,
        "published": true,
        "kind": "published"
    });
    st.commits.insert(cid.clone(), commit);
    st.commit_order.push(cid.clone());
    if let Some(b) = st.branches.get_mut(branch_name) {
        b.head_commit = cid;
        b.updated_at = ts;
    }

    let nodes: Vec<Value> = node_counts
        .into_iter()
        .map(|(name, entities_loaded)| json!({ "name": name, "entities_loaded": entities_loaded }))
        .collect();
    let edges: Vec<Value> = edge_counts
        .into_iter()
        .map(|(name, entities_loaded)| json!({ "name": name, "entities_loaded": entities_loaded }))
        .collect();
    let mut resp = json!({
        "branch": branch_name,
        "base_branch": from,
        "branch_created": branch_created,
        "mode": mode,
        "nodes": nodes,
        "edges": edges,
        "total_entities": parsed_records.len(),
        "embedding_generation": Value::Null,
        "actor_id": actor,
        "commit": commit_json,
    });
    if include_uri {
        resp["uri"] = json!(format!("mock://{}/graphs/{}", st.graph_id, st.graph_id));
    }
    axum::Json(resp).into_response()
}

async fn mock_load_ndjson(
    State(srv): State<MockServer>,
    AxumPath(graph_id): AxumPath<String>,
    AxumQuery(q): AxumQuery<BTreeMap<String, String>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let mut st = srv.state.lock().unwrap();
    let actor = match check_auth_and_contract(&headers, &st) {
        Ok(a) => a,
        Err(r) => return *r,
    };
    if graph_id != st.graph_id {
        return json_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    let branch = q.get("branch").map(String::as_str).unwrap_or("main");
    let from = q.get("from").map(String::as_str);
    let mode = q.get("mode").map(String::as_str).unwrap_or("merge");
    apply_mock_ndjson_load(&mut st, actor, branch, from, mode, &body, false)
}

async fn mock_load_json(
    State(srv): State<MockServer>,
    AxumPath(graph_id): AxumPath<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let mut st = srv.state.lock().unwrap();
    let actor = match check_auth_and_contract(&headers, &st) {
        Ok(a) => a,
        Err(r) => return *r,
    };
    if graph_id != st.graph_id {
        return json_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    let branch = body["branch"].as_str().unwrap_or("main");
    let from = body["from"].as_str();
    let mode = body["mode"].as_str().unwrap_or("merge");
    let data = body["data"].as_str().unwrap_or("");
    apply_mock_ndjson_load(&mut st, actor, branch, from, mode, data, true)
}

pub(crate) async fn spawn_mock_omnigraph() -> (String, tokio::task::JoinHandle<()>) {
    let srv = MockServer {
        state: Arc::new(Mutex::new(MockGraphState::new(schema::HARNESS_GRAPH_ID))),
    };
    let app = Router::new()
        .route("/healthz", get(mock_healthz))
        .route("/readyz", get(mock_readyz))
        .route("/graphs", get(mock_graphs))
        .route("/graphs/{graph_id}/schema", get(mock_schema))
        .route(
            "/graphs/{graph_id}/branches",
            get(mock_branches_list).post(mock_branches_create),
        )
        .route("/graphs/{graph_id}/branches/diff", get(mock_branches_diff))
        .route(
            "/graphs/{graph_id}/branches/merge",
            post(mock_branches_merge),
        )
        .route(
            "/graphs/{graph_id}/branches/{branch_name}",
            delete(mock_branches_delete),
        )
        .route("/graphs/{graph_id}/mutate", post(mock_mutate))
        .route(
            "/graphs/{graph_id}/mutate/if-graph-commit",
            post(mock_mutate),
        )
        .route("/graphs/{graph_id}/query", post(mock_query))
        .route("/graphs/{graph_id}/queries/{name}", post(mock_stored_query))
        .route("/graphs/{graph_id}/commits", get(mock_commits))
        .route(
            "/graphs/{graph_id}/commits/{commit_id}/changes",
            get(mock_commit_changes),
        )
        .route("/graphs/{graph_id}/diff", get(mock_diff))
        .route("/graphs/{graph_id}/changes", get(mock_changes))
        .route(
            "/graphs/{graph_id}/changes/baseline",
            post(mock_changes_baseline),
        )
        .route("/graphs/{graph_id}/export", post(mock_export))
        .route("/graphs/{graph_id}/load", post(mock_load_json))
        .route("/graphs/{graph_id}/load/ndjson", post(mock_load_ndjson))
        .with_state(srv);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), handle)
}

#[test]
fn schema_bundle_is_valid_and_writes_cleanly() {
    assert!(!schema::HARNESS_GRAPH_ID.contains('_'));
    assert_eq!(schema::CANONICAL_KNOWLEDGE_BRANCH, "knowledge/canonical");
    assert!(schema::HARNESS_CONTEXT_PG.contains("node Session"));
    assert!(schema::HARNESS_CONTEXT_PG.contains("node ContextAtom"));
    for prop in [
        "origin_layer: String?",
        "authority: String?",
        "privacy: String?",
        "brain_record_id: String?",
        "memory_id: String?",
        "record_revision: I64?",
    ] {
        assert!(
            schema::HARNESS_CONTEXT_PG.contains(prop),
            "missing schema property {prop}"
        );
    }
    for q in [
        schema::QUERY_CONTEXT_SEARCH,
        schema::QUERY_SESSION_NEIGHBORS,
        schema::QUERY_FILE_CONTEXT,
        schema::QUERY_RECENT_ATOMS,
        schema::HARNESS_STORED_QUERIES_GQ,
        schema::GQ_UPSERT_ATOM,
    ] {
        for field in [
            "origin_layer",
            "authority",
            "privacy",
            "brain_record_id",
            "memory_id",
            "record_revision",
        ] {
            assert!(q.contains(field), "missing {field} in query:\n{q}");
        }
    }
    assert!(schema::HARNESS_CONTEXT_PG.contains("node FileRef"));
    assert!(schema::HARNESS_CONTEXT_PG.contains("node SymbolRef"));
    assert!(schema::HARNESS_CONTEXT_PG.contains("node TurnSnapshot"));
    assert!(schema::HARNESS_CONTEXT_PG.contains("edge PublishedBy"));
    assert!(schema::HARNESS_CONTEXT_PG.contains("edge ConsumedBy"));
    assert!(schema::HARNESS_CONTEXT_PG.contains("edge TouchesFile"));
    assert!(schema::HARNESS_CONTEXT_PG.contains("edge MentionsSymbol"));
    assert!(schema::HARNESS_CONTEXT_PG.contains("edge RelatesTo"));
    assert!(schema::HARNESS_CONTEXT_PG.contains("edge DerivedFrom"));
    assert!(schema::HARNESS_CONTEXT_PG.contains("edge SessionFollows"));
    assert!(schema::HARNESS_CONTEXT_PG.contains("@key(@src, @dst)"));
    assert!(!schema::HARNESS_CONTEXT_PG.contains("\n#"));

    let names: Vec<&str> = schema::STORED_QUERIES.iter().map(|q| q.name).collect();
    assert_eq!(
        names,
        vec![
            "context_search",
            "session_neighbors",
            "file_context",
            "recent_atoms",
            "contradictions",
        ]
    );
    assert!(schema::validate_graph_id("bad_id").is_err());
    let rendered = schema::render_cluster_yaml("harness-context").unwrap();
    assert!(rendered.contains("harness-context:"));

    let dir = tempfile::tempdir().unwrap();
    schema::write_cluster_bundle(dir.path(), "harness-context").unwrap();
    assert!(dir.path().join("cluster.yaml").is_file());
    assert!(dir.path().join("harness_context.pg").is_file());
    assert!(dir.path().join("queries/harness_context.gq").is_file());
    assert!(dir.path().join("policy.yaml").is_file());
    assert!(dir.path().join("server.policy.yaml").is_file());
    assert_eq!(
        session_branch_name("abc123def456789"),
        "session/abc123def456"
    );
}

#[tokio::test]
async fn client_enforces_contract_auth_cas_branch_merge_conflict_and_feeds() {
    let (base_url, _server) = spawn_mock_omnigraph().await;

    // 1. Missing/invalid contract header check directly on raw reqwest
    let raw_http = reqwest::Client::new();
    let missing_hdr = raw_http
        .get(format!("{base_url}/graphs"))
        .bearer_auth("tok-harness")
        .send()
        .await
        .unwrap();
    assert_eq!(missing_hdr.status(), 400);

    // 2. Invalid token -> 401 unauthorized
    let bad_client = OmnigraphClient::new(
        &base_url,
        Some("wrong-token".into()),
        schema::HARNESS_GRAPH_ID,
    )
    .unwrap();
    let unauth_err = bad_client.list_graphs().await.unwrap_err();
    assert_eq!(unauth_err.status, 401);

    // 3. Actor mismatch -> 412 actor_mismatch
    let actor_mismatch_client = OmnigraphClient::new(
        &base_url,
        Some("tok-harness".into()),
        schema::HARNESS_GRAPH_ID,
    )
    .unwrap()
    .with_actor(Some("bob".into()));
    let mismatch_err = actor_mismatch_client.list_graphs().await.unwrap_err();
    assert!(mismatch_err.is_actor_mismatch());

    // 4. Valid client probes & schema
    let client = OmnigraphClient::new(
        &base_url,
        Some("tok-harness".into()),
        schema::HARNESS_GRAPH_ID,
    )
    .unwrap()
    .with_actor(Some("harness".into()));
    assert_eq!(client.health().await.unwrap().version, "0.15.0");
    assert!(client.ready().await.unwrap().ready);
    assert_eq!(client.list_graphs().await.unwrap().graphs.len(), 1);
    assert!(
        client
            .schema()
            .await
            .unwrap()
            .schema_source
            .contains("ContextAtom")
    );

    // 5. Mutate + CAS `Omnigraph-If-Graph-Commit` check (success and 412 failure)
    let branches = client.list_branches(true).await.unwrap();
    let main_head_1 = branches.details.unwrap()[0].head.graph_commit_id.clone();

    let insert_atom = |id: &str, title: &str| ChangeRequest {
        query: schema::GQ_UPSERT_ATOM.into(),
        name: Some("upsert_atom".into()),
        params: Some(json!({
            "atom_id": id,
            "project_id": "proj-1",
            "kind": "finding",
            "title": title,
            "summary": title,
            "body": format!("Details for {title}"),
            "tags": ["auth"],
            "source_branch": "main",
            "source_session_id": Value::Null,
            "confidence": 0.9,
            "pin_priority": 10,
            "verified": true,
            "origin_layer": Value::Null,
            "authority": Value::Null,
            "privacy": Value::Null,
            "brain_record_id": Value::Null,
            "memory_id": Value::Null,
            "record_revision": Value::Null,
            "created_at": "2026-11-16T18:00:00Z",
            "updated_at": "2026-11-16T18:00:00Z"
        })),
        branch: Some("main".into()),
        settings: None,
    };

    let c2 = client
        .mutate_if_commit(&insert_atom("atom-1", "Initial JWT finding"), &main_head_1)
        .await
        .unwrap();
    let main_head_2 = c2.commit.unwrap().graph_commit_id;
    assert_ne!(main_head_2, main_head_1);

    // Stale commit precondition MUST return 412 with expected vs actual
    let stale_err = client
        .mutate_if_commit(&insert_atom("atom-2", "Stale write"), &main_head_1)
        .await
        .unwrap_err();
    assert!(stale_err.is_precondition_failed());
    let pf = stale_err.precondition_failure.unwrap();
    assert_eq!(pf.expected, main_head_1);
    assert_eq!(pf.actual.as_deref(), Some(main_head_2.as_str()));

    // 6. Branch create, mutate on branch, branch diff, dry-run merge, fast-forward merge, delete
    let created = client
        .create_branch(&BranchCreateRequest {
            name: "session/s1".into(),
            from: Some("main".into()),
            from_commit: None,
        })
        .await
        .unwrap();
    assert_eq!(created.from_commit_id, main_head_2);

    let mut branch_req = insert_atom("atom-b1", "Branch insight on caching");
    branch_req.branch = Some("session/s1".into());
    client.mutate(&branch_req).await.unwrap();

    let diff = client
        .branch_diff(&BranchDiffQuery {
            source: "session/s1".into(),
            target: Some("main".into()),
            limit: None,
            page_token: None,
        })
        .await
        .unwrap();
    assert_eq!(diff.changes.len(), 1);
    assert_eq!(diff.changes[0].id, "atom-b1");

    let dry = client
        .merge_branches(&BranchMergeRequest {
            source: "session/s1".into(),
            target: Some("main".into()),
            delete_branch: false,
            as_commit: false,
            dry_run: true,
            settings: None,
        })
        .await
        .unwrap();
    assert_eq!(dry.outcome, BranchMergeOutcome::FastForward);
    assert!(dry.commit.is_none());

    let merged = client
        .merge_branches(&BranchMergeRequest {
            source: "session/s1".into(),
            target: Some("main".into()),
            delete_branch: true,
            as_commit: false,
            dry_run: false,
            settings: None,
        })
        .await
        .unwrap();
    assert_eq!(merged.outcome, BranchMergeOutcome::FastForward);
    assert_eq!(merged.branch_deleted, Some(true));
    assert!(
        !client
            .list_branches(false)
            .await
            .unwrap()
            .branches
            .contains(&"session/s1".into())
    );

    // 7. Time-travel snapshot query vs live branch query
    let past = client
        .query(&QueryRequest {
            query: schema::QUERY_RECENT_ATOMS.into(),
            name: Some("recent_atoms".into()),
            params: Some(json!({ "project_id": "proj-1" })),
            branch: None,
            snapshot: Some(main_head_2.clone()),
            settings: None,
        })
        .await
        .unwrap();
    assert_eq!(past.row_count, 1);

    let stored = client
        .stored_query(
            "context_search",
            &StoredQueryRequest {
                params: Some(json!({ "q": "caching", "project_id": "proj-1" })),
                branch: Some("main".into()),
                snapshot: None,
                expect_mutation: Some(false),
            },
        )
        .await
        .unwrap();
    assert_eq!(stored.row_count, 1);
    assert_eq!(stored.ranking.unwrap().by, "rrf");

    // 8. 409 merge conflict test (two branches diverge on same atom-1)
    client
        .create_branch(&BranchCreateRequest {
            name: "session/conflict".into(),
            from: Some("main".into()),
            from_commit: None,
        })
        .await
        .unwrap();
    client
        .mutate(&insert_atom("atom-1", "Main edited title"))
        .await
        .unwrap();
    let mut conflict_req = insert_atom("atom-1", "Branch conflicting title");
    conflict_req.branch = Some("session/conflict".into());
    client.mutate(&conflict_req).await.unwrap();

    let conflict_err = client
        .merge_branches(&BranchMergeRequest {
            source: "session/conflict".into(),
            target: Some("main".into()),
            delete_branch: false,
            as_commit: false,
            dry_run: false,
            settings: None,
        })
        .await
        .unwrap_err();
    assert!(conflict_err.is_merge_conflict());
    assert_eq!(
        conflict_err.merge_conflicts[0].kind,
        MergeConflictKind::DivergentUpdate
    );

    // 9. Commits, commit changes, diff, change feed, and 410 baseline recovery
    let commits = client
        .list_commits(&CommitListQuery {
            branch: Some("main".into()),
            limit: Some(10),
            page_token: None,
        })
        .await
        .unwrap();
    assert!(commits.commits.len() >= 3);
    let latest_cid = &commits.commits[0].graph_commit_id;
    let commit_ch = client
        .commit_changes(latest_cid, Some(50), None)
        .await
        .unwrap();
    assert_eq!(commit_ch.changes.len(), 1);

    let point_diff = client
        .diff(&DiffQuery {
            from: main_head_1,
            to: "main".into(),
            limit: None,
            page_token: None,
        })
        .await
        .unwrap();
    assert!(!point_diff.changes.is_empty());

    let feed = client
        .changes(&ChangeFeedQuery {
            branch: Some("main".into()),
            cursor: None,
            start: Some("beginning".into()),
            page_token: None,
            limit: Some(50),
        })
        .await
        .unwrap();
    assert!(!feed.blocks.is_empty());
    assert!(feed.cursor.is_some());

    let gap_err = client
        .changes(&ChangeFeedQuery {
            branch: Some("main".into()),
            cursor: Some("expired-cursor".into()),
            start: None,
            page_token: None,
            limit: None,
        })
        .await
        .unwrap_err();
    assert!(gap_err.is_change_feed_gap());

    let baseline = client
        .changes_baseline(&ChangeBaselineRequest {
            branch: Some("main".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!baseline.resume_cursor.is_empty());
    assert!(!baseline.records.is_empty());
}

#[tokio::test]
async fn sidecar_manager_and_domain_work_end_to_end() {
    let (base_url, _server) = spawn_mock_omnigraph().await;
    let state_dir = tempfile::tempdir().unwrap();
    let mgr = Arc::new(OmnigraphManager::new(state_dir.path().to_path_buf(), None).unwrap());
    let domain = OmnigraphDomain::new(mgr.clone());

    // 1. Disconnected status returns graceful unconfigured state
    let init_status = mgr.status_json("proj-1").await.unwrap();
    assert_eq!(init_status["connected"], false);
    assert_eq!(init_status["state"], "unconfigured");
    assert_eq!(init_status["mode"], "off");

    // 2. Connect via OmnigraphDomain (`omnigraph.setup` -> connect) and verify persisted config
    let connected_status = domain
        .execute(
            "omnigraph.setup",
            json!({
                "params": {},
                "query": {},
                "body": {
                    "action": "connect",
                    "endpoint": base_url,
                    "token": "tok-harness",
                    "graph_id": schema::HARNESS_GRAPH_ID
                }
            }),
        )
        .await
        .unwrap();
    let crate::services::Payload::Json(st_json) = connected_status else {
        panic!("expected json payload");
    };
    assert_eq!(st_json["connected"], true);
    assert_eq!(st_json["mode"], "external");
    assert!(st_json["mcp_url"].as_str().unwrap().ends_with("/mcp"));

    // Reload manager from disk to verify omnigraph.json persistence
    let reloaded = OmnigraphManager::new(state_dir.path().to_path_buf(), None).unwrap();
    let reloaded_cfg: OmnigraphConfig = reloaded.config().await;
    assert_eq!(reloaded_cfg.mode, OmnigraphMode::External);
    assert_eq!(reloaded_cfg.endpoint.as_deref(), Some(base_url.as_str()));

    // 3. Publish atom + files via Domain (`omnigraph.publish`)
    let pub_resp = domain
        .execute(
            "omnigraph.publish",
            json!({
                "params": {},
                "query": {},
                "body": {
                    "project_id": "proj-1",
                    "branch": "main",
                    "kind": "decision",
                    "title": "Use Omnigraph HTTP 0.14 sidecar",
                    "summary": "Avoid Lance/Arrow workspace bloat by using sidecar HTTP.",
                    "body": "Verified sha2 incompatibility; sidecar gives full MCP and branching.",
                    "tags": ["architecture", "omnigraph"],
                    "files": ["runtime-rust/crates/infra-harness/src/omnigraph.rs"],
                    "confidence": 0.96,
                    "pin_priority": 25
                }
            }),
        )
        .await
        .unwrap();
    let crate::services::Payload::Json(pub_json) = pub_resp else {
        panic!("expected json");
    };
    assert_eq!(pub_json["ok"], true);
    let published_atom_id = pub_json["atom"]["atom_id"].as_str().unwrap().to_string();
    assert_eq!(pub_json["id"], published_atom_id);
    assert_eq!(pub_json["atom_id"], published_atom_id);
    assert_eq!(pub_json["atom"]["id"], published_atom_id);
    assert_eq!(pub_json["atom"]["branch"], "main");
    assert_eq!(pub_json["atom"]["source_branch"], "main");
    assert!(
        pub_json["atom"]
            .as_object()
            .unwrap()
            .contains_key("session_id")
    );
    assert!(
        pub_json["atom"]
            .as_object()
            .unwrap()
            .contains_key("source_session_id")
    );

    // 4. `prepare_session_context` creates `session/<sid12>`, upserts Session, returns capsule
    let session = json!({
        "id": "1122334455667788",
        "project_id": "proj-1",
        "title": "Wire Omnigraph UI",
        "provider": "codex",
        "workflow": "native",
        "status": "running",
        "goal": "sidecar HTTP",
        "omnigraph": {
            "enabled": true,
            "branch": "auto",
            "query": "sidecar",
            "auto_merge": true
        }
    });
    let prep = mgr
        .prepare_session_context(&session, state_dir.path())
        .await
        .unwrap();
    assert_eq!(prep["enabled"], true);
    assert_eq!(prep["branch"], "session/112233445566");
    assert!(
        prep["capsule_text"]
            .as_str()
            .unwrap()
            .contains("Use Omnigraph HTTP 0.14 sidecar")
    );
    assert_eq!(prep["consumed_count"], 1);

    // 5. `publish_session_outputs` publishes learnings + files + snapshot + auto_merges into main
    let out = mgr
        .publish_session_outputs(
            &session,
            &[json!({
                "type": "finding",
                "title": "Branches need percent encoding on DELETE",
                "consequence": "Session branches with slashes must encode `/` as `%2F`.",
                "sources": ["runtime-rust/crates/infra-harness/src/omnigraph/client.rs"]
            })],
            &["runtime-rust/crates/infra-harness/src/omnigraph/sidecar.rs".into()],
            "Completed sidecar integration",
        )
        .await
        .unwrap();
    assert_eq!(out["ok"], true);
    assert_eq!(out["merged"], true);
    assert_eq!(out["published_atom_ids"].as_array().unwrap().len(), 1);

    // 6. Explicit share (`omnigraph.share`), graph view (`omnigraph.graph`), search (`omnigraph.search`),
    //    branch actions & diff, changes, commit detail
    let share_res = domain
        .execute(
            "omnigraph.share",
            json!({
                "params": {},
                "query": {},
                "body": {
                    "project_id": "proj-1",
                    "target_session_id": "target-session-99",
                    "atom_ids": [published_atom_id],
                    "source_branch": "main",
                    "mode": "copy"
                }
            }),
        )
        .await
        .unwrap();
    let crate::services::Payload::Json(share_json) = share_res else {
        panic!("expected json");
    };
    assert_eq!(share_json["shared_count"], 1);
    assert_eq!(share_json["target_branch"], "session/target-sessi");

    // 6b. Publish with session provenance and mixed `relates_to` formats (string ID, `rel:id`, and object)
    let pub_rel = domain
        .execute(
            "omnigraph.publish",
            json!({
                "params": {},
                "query": {},
                "body": {
                    "project_id": "proj-1",
                    "branch": "main",
                    "session_id": "sess-author-01",
                    "kind": "finding",
                    "title": "Validate relation shorthands",
                    "summary": "Supports plain string, relation:id, and object entries.",
                    "relates_to": [
                        published_atom_id,
                        format!("refines:{published_atom_id}"),
                        { "target_id": "atom-relates-fallback", "relation": "relates_to" },
                        { "target_id": published_atom_id, "relation": "depends_on" }
                    ]
                }
            }),
        )
        .await
        .unwrap();
    let crate::services::Payload::Json(pub_rel_json) = pub_rel else {
        panic!("expected json");
    };
    assert_eq!(pub_rel_json["atom"]["session_id"], "sess-author-01");
    assert_eq!(pub_rel_json["atom"]["source_session_id"], "sess-author-01");

    let graph_res = domain
        .execute(
            "omnigraph.graph",
            json!({
                "params": {},
                "query": { "project_id": "proj-1", "branch": "main" },
                "body": {}
            }),
        )
        .await
        .unwrap();
    let crate::services::Payload::Json(graph_json) = graph_res else {
        panic!("expected json");
    };
    assert!(graph_json["atoms"].as_array().unwrap().len() >= 2);
    let first_atom = &graph_json["atoms"][0];
    assert_eq!(first_atom["id"], first_atom["atom_id"]);
    assert_eq!(first_atom["branch"], first_atom["source_branch"]);
    assert!(first_atom.as_object().unwrap().contains_key("session_id"));
    assert!(
        first_atom
            .as_object()
            .unwrap()
            .contains_key("source_session_id")
    );
    assert!(!graph_json["files"].as_array().unwrap().is_empty());
    assert!(
        graph_json["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "relates_to")
            .count()
            >= 4
    );
    assert!(
        graph_json["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["to"] == "atom-relates-fallback" && e["relation"] == "supports")
    );
    assert!(!graph_json["branches"].as_array().unwrap().is_empty());
    assert_eq!(
        graph_json["branches"][0]["head_commit"],
        graph_json["branches"][0]["head_commit_id"]
    );
    assert!(!graph_json["commits"].as_array().unwrap().is_empty());
    let first_commit = &graph_json["commits"][0];
    assert!(first_commit["authored_at"].as_i64().is_some());
    assert!(
        first_commit["created_at"]
            .as_str()
            .is_some_and(|s| s.ends_with('Z'))
    );
    assert!(first_commit["added_count"].as_u64().is_some());
    assert!(first_commit["updated_count"].as_u64().is_some());
    assert!(first_commit["deleted_count"].as_u64().is_some());
    assert!(
        first_commit["summary"]
            .as_str()
            .is_some_and(|s| !s.is_empty() && !s.starts_with("Commit "))
    );
    assert!(graph_json["commits"].as_array().unwrap().iter().any(|c| {
        c["added_count"].as_u64().unwrap_or(0) > 0
            && c["summary"]
                .as_str()
                .is_some_and(|s| s.starts_with("Published: ") || s.starts_with("Merged "))
    }));

    let search_res = domain
        .execute(
            "omnigraph.search",
            json!({
                "params": {},
                "query": { "project_id": "proj-1", "q": "percent encoding", "limit": "5" },
                "body": {}
            }),
        )
        .await
        .unwrap();
    let crate::services::Payload::Json(search_json) = search_res else {
        panic!("expected json");
    };
    assert_eq!(search_json["hits"].as_array().unwrap().len(), 1);

    let ch_res = domain
        .execute(
            "omnigraph.changes",
            json!({
                "params": {},
                "query": { "project_id": "proj-1", "branch": "main" },
                "body": {}
            }),
        )
        .await
        .unwrap();
    let crate::services::Payload::Json(ch_json) = ch_res else {
        panic!("expected json");
    };
    let ch_events = ch_json["events"].as_array().unwrap();
    assert!(!ch_events.is_empty());
    let sample_ev = &ch_events[0];
    assert!(sample_ev["kind"].is_string());
    assert!(sample_ev["title"].is_string());
    assert!(sample_ev["summary"].is_string());
    assert!(sample_ev["branch"].is_string());
    assert!(sample_ev["authored_at"].as_i64().is_some());
    assert!(sample_ev["timestamp"].as_i64().is_some());
    assert!(
        sample_ev["created_at"]
            .as_str()
            .is_some_and(|s| s.ends_with('Z'))
    );
    let node_ev = ch_events
        .iter()
        .find(|ev| ev["entity_kind"] == "node")
        .expect("node event");
    assert_eq!(node_ev["node_id"], node_ev["entity_id"]);

    let commit_id = graph_json["commits"][0]["commit_id"].as_str().unwrap();
    let detail_res = domain
        .execute(
            "omnigraph.commit_detail",
            json!({
                "params": { "commit_id": commit_id },
                "query": { "project_id": "proj-1" },
                "body": {}
            }),
        )
        .await
        .unwrap();
    let crate::services::Payload::Json(detail_json) = detail_res else {
        panic!("expected json");
    };
    assert_eq!(detail_json["commit"]["graph_commit_id"], commit_id);
    assert_eq!(detail_json["commit"]["commit_id"], commit_id);
    assert!(detail_json["commit"]["authored_at"].as_i64().is_some());
    assert!(
        detail_json["commit"]["created_at"]
            .as_str()
            .is_some_and(|s| s.ends_with('Z'))
    );
    if let Some(first_ch) = detail_json["changes"].as_array().and_then(|a| a.first()) {
        assert!(first_ch["title"].is_string());
        assert!(first_ch["kind"].is_string());
        assert!(first_ch["type_name"].is_string());
        assert!(first_ch["entity_id"].is_string());
        assert!(first_ch["op"].is_string());
        assert!(first_ch["summary"].is_string());
    }

    // Branch create + diff + 409 conflict surfacing in `branch_diff_json` & `branch_action_json`
    domain
        .execute(
            "omnigraph.branch_act",
            json!({
                "params": {},
                "query": {},
                "body": { "action": "create", "name": "feature/ui", "from": "main" }
            }),
        )
        .await
        .unwrap();
    domain
        .execute(
            "omnigraph.publish",
            json!({
                "params": {},
                "query": {},
                "body": {
                    "project_id": "proj-1",
                    "branch": "main",
                    "atom_id": published_atom_id,
                    "kind": "decision",
                    "title": "Main diverged title",
                    "summary": "Main change"
                }
            }),
        )
        .await
        .unwrap();
    domain
        .execute(
            "omnigraph.publish",
            json!({
                "params": {},
                "query": {},
                "body": {
                    "project_id": "proj-1",
                    "branch": "feature/ui",
                    "atom_id": published_atom_id,
                    "kind": "decision",
                    "title": "Feature diverged title",
                    "summary": "Feature change"
                }
            }),
        )
        .await
        .unwrap();
    let diff_res = domain
        .execute(
            "omnigraph.branch_diff",
            json!({
                "params": {},
                "query": { "project_id": "proj-1", "source": "feature/ui", "target": "main" },
                "body": {}
            }),
        )
        .await
        .unwrap();
    let crate::services::Payload::Json(diff_json) = diff_res else {
        panic!("expected json");
    };
    assert_eq!(diff_json["outcome"], "conflict");
    assert!(!diff_json["conflicts"].as_array().unwrap().is_empty());
    assert!(diff_json["deleted_edges"].is_array());
    for edge in diff_json["added_edges"].as_array().unwrap() {
        assert!(edge["from"].is_string());
        assert!(edge["to"].is_string());
        assert!(edge["relation"].is_string());
        assert!(edge["endpoints"].is_object());
    }

    let merge_conflict = domain
        .execute(
            "omnigraph.branch_act",
            json!({
                "params": {},
                "query": {},
                "body": { "action": "merge", "source": "feature/ui", "target": "main" }
            }),
        )
        .await
        .unwrap();
    let crate::services::Payload::Json(mc_json) = merge_conflict else {
        panic!("expected json");
    };
    assert_eq!(mc_json["ok"], false);
    assert_eq!(mc_json["outcome"], "conflict");
    assert_eq!(mc_json["added_atoms"], json!([]));
    assert_eq!(mc_json["updated_atoms"], json!([]));
    assert_eq!(mc_json["deleted_atoms"], json!([]));
    assert_eq!(mc_json["added_edges"], json!([]));
    assert_eq!(mc_json["deleted_edges"], json!([]));

    // Unknown operation -> 404
    let err = domain
        .execute(
            "omnigraph.unknown",
            json!({"params":{},"query":{},"body":{}}),
        )
        .await;
    assert!(err.is_err());

    mgr.close().await.unwrap();
}

#[tokio::test]
async fn install_local_extracts_tarball_and_local_sidecar_lifecycle_works() {
    // Build a tiny `.tar.gz` containing mock `omnigraph` and `omnigraph-server` executables
    // (implemented in bash) and serve it over local HTTP to verify download, extraction,
    // cluster apply + force-unlock, server startup, /readyz probe, and graceful `close()`.
    let cli_script = r#"#!/usr/bin/env bash
set -e
if [ "$1" = "version" ]; then
  echo '{"version":"0.15.0"}'
  exit 0
fi
if [ "$1" = "cluster" ] && [ "$2" = "apply" ]; then
  echo '{"status":"complete"}'
  exit 0
fi
if [ "$1" = "cluster" ] && [ "$2" = "status" ]; then
  echo '{"lock_id":"lock-123"}'
  exit 0
fi
if [ "$1" = "cluster" ] && [ "$2" = "force-unlock" ]; then
  echo '{"unlocked":true}'
  exit 0
fi
exit 0
"#;
    let server_script = r#"#!/usr/bin/env python3
import http.server, json, sys

bind = "127.0.0.1:0"
for i, arg in enumerate(sys.argv):
    if arg == "--bind" and i + 1 < len(sys.argv):
        bind = sys.argv[i + 1]
host, port = bind.split(":")

class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/healthz":
            b = json.dumps({"status":"ok","version":"0.15.0","internal_schema_version":16}).encode()
        elif self.path == "/readyz":
            b = json.dumps({"ready":True,"status":"serving","served_graph_count":1,"ready_graph_count":1}).encode()
        elif self.path.endswith("/branches?detailed=true") or self.path.endswith("/branches"):
            b = json.dumps({"branches":["main"],"details":[{"name":"main","head":{"graph_commit_id":"c-1","created_at":1,"published":true},"updated_at":1}]}).encode()
        elif "/commits" in self.path:
            b = json.dumps({"commits":[{"graph_commit_id":"c-1","published":true}]}).encode()
        else:
            b = json.dumps({"graphs":[]}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)
    def do_POST(self):
        ln = int(self.headers.get("Content-Length", "0"))
        _ = self.rfile.read(ln)
        b = json.dumps({"query_name":"recent_atoms","target":{"branch":"main"},"row_count":0,"rows":[]}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)
    def log_message(self, *args):
        pass

http.server.HTTPServer((host, int(port)), H).serve_forever()
"#;

    let mut tar_bytes = Vec::new();
    {
        let gz = flate2::write::GzEncoder::new(&mut tar_bytes, flate2::Compression::fast());
        let mut builder = tar::Builder::new(gz);
        for (name, content) in [
            ("omnigraph", cli_script.as_bytes()),
            ("omnigraph-server", server_script.as_bytes()),
        ] {
            let mut hdr = tar::Header::new_gnu();
            hdr.set_size(content.len() as u64);
            hdr.set_mode(0o755);
            hdr.set_cksum();
            builder.append_data(&mut hdr, name, content).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
    }

    let tar_bytes = Arc::new(tar_bytes);
    let tar_app = Router::new().route(
        "/omnigraph-linux-x86_64.tar.gz",
        get({
            let data = tar_bytes.clone();
            move || {
                let bytes = data.clone();
                async move { (StatusCode::OK, bytes.as_ref().clone()) }
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _srv = tokio::spawn(async move {
        let _ = axum::serve(listener, tar_app).await;
    });

    let state_dir = tempfile::tempdir().unwrap();
    let mgr = OmnigraphManager::new(state_dir.path().to_path_buf(), None).unwrap();

    let installed = mgr
        .install_local_from(&format!("http://{addr}/omnigraph-linux-x86_64.tar.gz"))
        .await
        .unwrap();
    assert_eq!(installed["ok"], true);
    assert!(mgr.tools_dir().join("omnigraph").is_file());
    assert!(mgr.tools_dir().join("omnigraph-server").is_file());

    let started = mgr.start_local(None).await.unwrap();
    assert_eq!(started["connected"], true);
    assert_eq!(started["mode"], "local_sidecar");
    assert_eq!(started["binary_installed"], true);
    assert_eq!(started["binary_version"], "0.15.0");

    mgr.close().await.unwrap();
    let after_close = mgr.status_json("").await.unwrap();
    assert_eq!(after_close["connected"], false);

    // Verify path-traversal tar entry rejection (raw GNU header with `../omnigraph`)
    let mut bad_tar = Vec::new();
    {
        let mut raw_tar = Vec::new();
        let content = b"evil";
        let mut hdr = tar::Header::new_gnu();
        hdr.set_size(content.len() as u64);
        hdr.set_mode(0o755);
        let p = b"../omnigraph";
        hdr.as_mut_bytes()[..p.len()].copy_from_slice(p);
        hdr.set_cksum();
        raw_tar.extend_from_slice(hdr.as_bytes());
        let mut block = [0u8; 512];
        block[..content.len()].copy_from_slice(content);
        raw_tar.extend_from_slice(&block);
        raw_tar.extend_from_slice(&[0u8; 1024]);
        let mut gz = flate2::write::GzEncoder::new(&mut bad_tar, flate2::Compression::fast());
        gz.write_all(&raw_tar).unwrap();
        gz.finish().unwrap();
    }
    let bad_app = Router::new().route(
        "/bad.tar.gz",
        get({
            let data = Arc::new(bad_tar);
            move || {
                let bytes = data.clone();
                async move { (StatusCode::OK, bytes.as_ref().clone()) }
            }
        }),
    );
    let bad_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bad_addr = bad_listener.local_addr().unwrap();
    let _bad_srv = tokio::spawn(async move {
        let _ = axum::serve(bad_listener, bad_app).await;
    });
    let res = mgr
        .install_local_from(&format!("http://{bad_addr}/bad.tar.gz"))
        .await;
    assert!(res.is_err());
    assert!(res.unwrap_err().message.contains("Unsafe path traversal"));
}

#[tokio::test]
async fn sidecar_payload_contracts_cover_deleted_edges_and_share_modes() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let change_hits = Arc::new(AtomicUsize::new(0));
    let hit_counter = change_hits.clone();
    let app = Router::new()
        .route("/healthz", get(mock_healthz))
        .route(
            "/readyz",
            get(|| async { axum::Json(json!({ "ready": true, "status": "serving" })) }),
        )
        .route(
            "/graphs",
            get(|| async {
                axum::Json(json!({ "graphs": [{ "id": schema::HARNESS_GRAPH_ID }] }))
            }),
        )
        .route(
            &format!("/graphs/{}/branches", schema::HARNESS_GRAPH_ID),
            get(|| async {
                axum::Json(json!({
                    "branches": ["main", "feature/del-edge"],
                    "details": [
                        {
                            "name": "main",
                            "head": {
                                "graph_commit_id": "c-main-1",
                                "graph_branch": "main",
                                "created_at": 1_700_000_000_000_000i64,
                                "published": true
                            },
                            "created_at": 1_700_000_000_000_000i64,
                            "updated_at": 1_700_000_000_000_000i64
                        },
                        {
                            "name": "feature/del-edge",
                            "head": {
                                "graph_commit_id": "c-feat-2",
                                "graph_branch": "feature/del-edge",
                                "created_at": 1_700_000_100_000_000i64,
                                "published": true
                            },
                            "from": "main",
                            "from_commit_id": "c-main-1",
                            "created_at": 1_700_000_050_000_000i64,
                            "updated_at": 1_700_000_100_000_000i64
                        }
                    ]
                }))
            })
            .post(|| async {
                axum::Json(json!({
                    "uri": "/graphs/harness-context/branches/session",
                    "from": "main",
                    "name": "session/session-share",
                    "from_commit_id": "c-main-1"
                }))
            }),
        )
        .route(
            &format!("/graphs/{}/commits", schema::HARNESS_GRAPH_ID),
            get(|| async {
                axum::Json(json!({
                    "commits": [{
                        "graph_commit_id": "c-main-1",
                        "graph_branch": "main",
                        "created_at": 1_700_000_000_000_000i64,
                        "published": true
                    }]
                }))
            }),
        )
        .route(
            &format!(
                "/graphs/{}/commits/c-main-1/changes",
                schema::HARNESS_GRAPH_ID
            ),
            get(move || {
                let hits = hit_counter.clone();
                async move {
                    hits.fetch_add(1, Ordering::Relaxed);
                    axum::Json(json!({
                        "cause": {
                            "graph_commit_id": "c-main-1",
                            "authored_branch": "main",
                            "authored_at": 1_700_000_000_000_000i64
                        },
                        "changes": [{
                            "kind": "node",
                            "type": { "id": "t-atom", "name": "ContextAtom" },
                            "id": "a0",
                            "op": "insert",
                            "after": { "properties": { "title": "Cached atom title" } }
                        }]
                    }))
                }
            }),
        )
        .route(
            &format!("/graphs/{}/branches/diff", schema::HARNESS_GRAPH_ID),
            get(|| async {
                axum::Json(json!({
                    "from_commit_id": "c-main-1",
                    "to_commit_id": "c-feat-2",
                    "base_commit_id": "c-main-1",
                    "changes": [
                        {
                            "kind": "edge",
                            "type": { "id": "t-rel", "name": "RelatesTo" },
                            "id": "e-add-1",
                            "op": "insert",
                            "after": {
                                "properties": { "relation": "refines", "source_id": "a2", "target_id": "a1" },
                                "endpoints": { "from": "a2", "to": "a1" }
                            }
                        },
                        {
                            "kind": "edge",
                            "type": { "id": "t-con", "name": "ConsumedBy" },
                            "id": "e-con-1",
                            "op": "insert",
                            "after": {
                                "properties": { "injection_mode": "auto_capsule", "atom_id": "a2", "session_id": "s1" },
                                "endpoints": { "from": "a2", "to": "s1" }
                            }
                        },
                        {
                            "kind": "edge",
                            "type": { "id": "t-tch", "name": "TouchesFile" },
                            "id": "e-tch-1",
                            "op": "insert",
                            "after": {
                                "properties": { "access": "referenced", "atom_id": "a2", "path": "src/lib.rs" },
                                "endpoints": { "from": "a2", "to": "src/lib.rs" }
                            }
                        },
                        {
                            "kind": "edge",
                            "type": { "id": "t-pub", "name": "PublishedBy" },
                            "id": "e-pub-1",
                            "op": "insert",
                            "after": {
                                "properties": { "role": "author", "atom_id": "a2", "session_id": "s1" },
                                "endpoints": { "from": "a2", "to": "s1" }
                            }
                        },
                        {
                            "kind": "edge",
                            "type": { "id": "t-der", "name": "DerivedFrom" },
                            "id": "e-der-1",
                            "op": "insert",
                            "after": {
                                "properties": { "atom_id": "a2", "snapshot_id": "snap-1" },
                                "endpoints": { "from": "a2", "to": "snap-1" }
                            }
                        },
                        {
                            "kind": "edge",
                            "type": { "id": "t-rel", "name": "RelatesTo" },
                            "id": "e-del-1",
                            "op": "delete",
                            "before": {
                                "properties": { "relation": "supports", "source_id": "a1", "target_id": "a0" },
                                "endpoints": { "from": "a1", "to": "a0" }
                            }
                        }
                    ]
                }))
            }),
        )
        .route(
            &format!("/graphs/{}/changes", schema::HARNESS_GRAPH_ID),
            get(|AxumQuery(q): AxumQuery<BTreeMap<String, String>>| async move {
                if q.get("cursor").map(String::as_str) == Some("cur-other-branch") {
                    return json_err(
                        StatusCode::BAD_REQUEST,
                        "change_cursor_rejected",
                        "change cursor rejected: change feed continuation was minted for a different branch",
                        None,
                    );
                }
                axum::Json(json!({
                    "blocks": [{
                        "cause": {
                            "graph_commit_id": "c-feat-2",
                            "authored_branch": "feature/del-edge",
                            "authored_at": 1_700_000_100_000_000i64
                        },
                        "changes": [{
                            "kind": "node",
                            "type": { "id": "t-atom", "name": "ContextAtom" },
                            "id": "a2",
                            "op": "insert",
                            "after": { "properties": { "title": "Branch pulse atom" } }
                        }]
                    }],
                    "cursor": "cur-feat-1",
                    "caught_up": true
                }))
                .into_response()
            }),
        )
        .route(
            &format!("/graphs/{}/branches/merge", schema::HARNESS_GRAPH_ID),
            post(|| async {
                axum::Json(json!({
                    "source": "feature/del-edge",
                    "target": "main",
                    "outcome": "fast_forward",
                    "dry_run": true
                }))
            }),
        )
        .route(
            &format!("/graphs/{}/query", schema::HARNESS_GRAPH_ID),
            post(|| async { axum::Json(json!({ "rows": [] })) }),
        )
        .route(
            &format!("/graphs/{}/queries/{{name}}", schema::HARNESS_GRAPH_ID),
            post(|| async { axum::Json(json!({ "rows": [] })) }),
        )
        .route(
            &format!("/graphs/{}/mutate", schema::HARNESS_GRAPH_ID),
            post(|| async {
                axum::Json(json!({
                    "branch": "session/s1",
                    "query_name": "mutate",
                    "affected_nodes": 1,
                    "affected_edges": 1
                }))
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _srv = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let state_dir = tempfile::tempdir().unwrap();
    let mgr = OmnigraphManager::new(state_dir.path().to_path_buf(), None).unwrap();
    mgr.connect_external(&format!("http://{addr}"), Some("tok"), None)
        .await
        .unwrap();

    let diff = mgr
        .branch_diff_json("proj-1", "feature/del-edge", Some("main"))
        .await
        .unwrap();
    let added_edges = diff["added_edges"].as_array().unwrap();
    assert_eq!(added_edges.len(), 5);
    assert_eq!(added_edges[0]["from"], "a2");
    assert_eq!(added_edges[0]["to"], "a1");
    assert_eq!(added_edges[0]["relation"], "refines");
    assert_eq!(added_edges[1]["relation"], "consumed_by");
    assert_eq!(added_edges[2]["relation"], "touches_file");
    assert_eq!(added_edges[3]["relation"], "published_by");
    assert_eq!(added_edges[4]["relation"], "derived_from");
    assert_eq!(diff["deleted_edges"].as_array().unwrap().len(), 1);
    assert_eq!(diff["deleted_edges"][0]["from"], "a1");
    assert_eq!(diff["deleted_edges"][0]["to"], "a0");
    assert_eq!(diff["deleted_edges"][0]["relation"], "supports");
    assert_eq!(diff["deleted_edges"][0]["endpoints"]["from"], "a1");
    assert_eq!(diff["deleted_edges"][0]["endpoints"]["to"], "a0");

    for mode in ["inject", "reference", "inject_and_link", "copy"] {
        let out = mgr
            .share_into_session_json(&json!({
                "project_id": "proj-1",
                "target_session_id": "session-share-check",
                "atom_ids": ["a2"],
                "source_branch": "main",
                "mode": mode,
            }))
            .await
            .unwrap();
        assert_eq!(out["ok"], true);
        assert_eq!(out["mode"], mode);
        assert_eq!(out["source_branch"], "main");
    }

    let invalid = mgr
        .share_into_session_json(&json!({
            "project_id": "proj-1",
            "target_session_id": "session-share-check",
            "atom_ids": ["a2"],
            "mode": "unknown-mode",
        }))
        .await;
    assert!(invalid.is_err());

    let gv1 = mgr
        .graph_view_json("proj-1", Some("main"), None, None, None, None)
        .await
        .unwrap();
    assert_eq!(gv1["commits"][0]["added_count"], 1);
    assert_eq!(gv1["commits"][0]["summary"], "Published: Cached atom title");
    assert_eq!(change_hits.load(Ordering::Relaxed), 1);

    let gv2 = mgr
        .graph_view_json("proj-1", Some("main"), None, None, None, None)
        .await
        .unwrap();
    assert_eq!(gv2["commits"][0]["summary"], "Published: Cached atom title");
    assert_eq!(change_hits.load(Ordering::Relaxed), 1);

    // Rejected cross-branch cursor automatically retries from start on that branch
    let ch_reset = mgr
        .changes_json("proj-1", Some("cur-other-branch"), Some("feature/del-edge"))
        .await
        .unwrap();
    assert_eq!(ch_reset["reset"], true);
    assert_eq!(ch_reset["cursor"], "cur-feat-1");
    assert_eq!(ch_reset["events"].as_array().unwrap().len(), 1);
    assert_eq!(ch_reset["events"][0]["atom_id"], "a2");
}

#[tokio::test]
async fn client_export_branch_and_load_round_trip() {
    let (base_url, _server) = spawn_mock_omnigraph().await;
    let client = OmnigraphClient::new(
        &base_url,
        Some("tok-harness".into()),
        schema::HARNESS_GRAPH_ID,
    )
    .unwrap();

    let seed_ndjson = [
        r#"{"type":"Session","id":"s-seed","data":{"session_id":"s-seed","project_id":"proj-1","branch":"main","title":"Seed","status":"running","updated_at":"2026-11-16T10:00:00.000Z"}}"#,
        r#"{"type":"FileRef","id":"src/auth.rs","data":{"path":"src/auth.rs","project_id":"proj-1","language":"rust","last_touched_at":"2026-11-16T10:00:00.000Z"}}"#,
        r#"{"type":"SymbolRef","id":"sym::login","data":{"fqn":"sym::login","path":"src/auth.rs","name":"login","kind":"function","project_id":"proj-1"}}"#,
        r#"{"type":"ContextAtom","id":"atom-seed-1","data":{"atom_id":"atom-seed-1","project_id":"proj-1","branch":"main","origin_session_id":"s-seed","kind":"decision","title":"Use JWT RS256","summary":"Verify asymmetrically","importance":0.9,"stale":false,"created_at":"2026-11-16T10:00:00.000Z","updated_at":"2026-11-16T10:00:00.000Z"}}"#,
        r#"{"edge":"PublishedBy","id":"pub-1","from":"atom-seed-1","to":"s-seed","data":{"published_at":"2026-11-16T10:00:00.000Z","role":"author"}}"#,
        r#"{"edge":"TouchesFile","id":"tf-1","from":"atom-seed-1","to":"src/auth.rs","data":{"access":"modified"}}"#,
        r#"{"edge":"MentionsSymbol","id":"ms-1","from":"atom-seed-1","to":"sym::login","data":{"role":"defines"}}"#,
    ]
    .join("\n");
    client
        .load_ndjson("main", None, "merge", &seed_ndjson, Some("harness"))
        .await
        .unwrap();

    let full_ndjson = client
        .export_branch(Some("main"), &[], Some("harness"))
        .await
        .unwrap();
    assert!(full_ndjson.contains("\"ContextAtom\""));
    assert!(full_ndjson.contains("\"PublishedBy\""));
    assert!(full_ndjson.contains("\"TouchesFile\""));
    assert!(full_ndjson.contains("\"MentionsSymbol\""));

    let atoms_only = client
        .export_branch(Some("main"), &["ContextAtom"], Some("harness"))
        .await
        .unwrap();
    assert!(atoms_only.contains("\"ContextAtom\""));
    assert!(!atoms_only.contains("\"FileRef\""));
    assert!(!atoms_only.contains("\"PublishedBy\""));

    let loaded_ndjson = client
        .load_ndjson(
            "imported/ndjson",
            Some("main"),
            "merge",
            &full_ndjson,
            Some("harness"),
        )
        .await
        .unwrap();
    assert_eq!(loaded_ndjson["branch"], "imported/ndjson");
    assert_eq!(loaded_ndjson["branch_created"], true);
    assert!(loaded_ndjson["commit"]["graph_commit_id"].is_string());

    let loaded_json = client
        .load_json(
            "imported/json",
            Some("main"),
            "merge",
            &full_ndjson,
            Some("harness"),
        )
        .await
        .unwrap();
    assert_eq!(loaded_json["branch"], "imported/json");
    assert_eq!(loaded_json["branch_created"], true);
    assert!(loaded_json["uri"].is_string());

    // `append` mode rejects duplicate atom IDs already present on `main`.
    let append_err = client
        .load_ndjson("main", None, "append", &atoms_only, Some("harness"))
        .await
        .unwrap_err();
    assert_eq!(append_err.status, 409);
}

#[tokio::test]
async fn sidecar_export_import_round_trip_and_scopes() {
    let (base_url, _server) = spawn_mock_omnigraph().await;
    let state_dir = tempfile::tempdir().unwrap();
    let mgr = OmnigraphManager::new(state_dir.path().to_path_buf(), None).unwrap();
    mgr.connect_external(&base_url, Some("tok-harness"), None)
        .await
        .unwrap();

    let s1 = "session-export-alpha";
    let b1 = crate::omnigraph::session_branch_name(s1);
    let sess1 = json!({
        "id": s1,
        "project_id": "proj-1",
        "title": "Export Alpha",
        "provider": "claude",
        "workflow": "native",
        "omnigraph": { "enabled": true, "mode": "session_branch" }
    });
    mgr.prepare_session_context(&sess1, state_dir.path())
        .await
        .unwrap();
    mgr.publish_atom_json(&json!({
        "project_id": "proj-1",
        "branch": &b1,
        "session_id": s1,
        "atom_id": "atom-alpha-1",
        "kind": "decision",
        "title": "Alpha task auth choice",
        "summary": "Use middleware guard",
        "files": ["src/middleware.rs"]
    }))
    .await
    .unwrap();

    // Publish an inherited atom on `main` and merge `main` into `b1` so `b1` holds it without `s1` consuming it.
    mgr.publish_atom_json(&json!({
        "project_id": "proj-1",
        "branch": "main",
        "session_id": "s-main-seed",
        "atom_id": "atom-inherited",
        "kind": "contract",
        "title": "Shared baseline invariant",
        "summary": "All tokens expire after 1h",
        "files": ["src/config.rs"]
    }))
    .await
    .unwrap();
    mgr.branch_action_json(&json!({
        "action": "merge",
        "source": "main",
        "target": &b1,
        "dry_run": false
    }))
    .await
    .unwrap();

    // 1. `scope: "task"` with `include_inherited: false` excludes `main` baseline `atom-inherited`.
    let task_only = mgr
        .export_context_json(&json!({
            "project_id": "proj-1",
            "session_id": s1,
            "scope": "task",
            "include_inherited": false,
            "format": "bundle"
        }))
        .await
        .unwrap();
    assert_eq!(task_only["ok"], true);
    assert_eq!(task_only["format"], "harness.omnigraph.bundle.v1");
    assert_eq!(task_only["source_branch"], b1);
    assert_eq!(task_only["source_session_id"], s1);
    let task_atom_ids: Vec<&str> = task_only["bundle"]["atoms"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|a| a["id"].as_str())
        .collect();
    assert_eq!(task_atom_ids, vec!["atom-alpha-1"]);
    assert!(
        task_only["ndjson"]
            .as_str()
            .unwrap()
            .contains("atom-alpha-1")
    );

    // 2. `scope: "task"` with `include_inherited: true` includes consumed `atom-inherited`.
    let with_inherited = mgr
        .export_context_json(&json!({
            "project_id": "proj-1",
            "session_id": s1,
            "scope": "task",
            "include_inherited": true,
            "format": "bundle"
        }))
        .await
        .unwrap();
    let inherited_ids: Vec<&str> = with_inherited["bundle"]["atoms"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|a| a["id"].as_str())
        .collect();
    assert!(inherited_ids.contains(&"atom-alpha-1"));
    assert!(inherited_ids.contains(&"atom-inherited"));

    // 3. `export_context_payload` with `download=1` returns downloadable bytes + filename.
    let download_bundle = mgr
        .export_context_payload(
            &json!({
                "project_id": "proj-1",
                "session_id": s1,
                "scope": "task",
                "format": "bundle",
                "download": "1"
            }),
            &json!({}),
        )
        .await
        .unwrap();
    match download_bundle {
        crate::services::Payload::Bytes {
            content_type,
            filename,
            bytes,
            ..
        } => {
            assert_eq!(content_type, "application/json; charset=utf-8");
            let fname = filename.unwrap();
            assert!(fname.starts_with("omnigraph-") && fname.ends_with(".json"));
            let parsed: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(parsed["format"], "harness.omnigraph.bundle.v1");
        }
        crate::services::Payload::Json(_) => panic!("expected Payload::Bytes for download=1"),
    }

    let download_ndjson = mgr
        .export_context_payload(
            &json!({
                "project_id": "proj-1",
                "branch": b1,
                "scope": "branch",
                "format": "ndjson",
                "download": "1"
            }),
            &json!({}),
        )
        .await
        .unwrap();
    match download_ndjson {
        crate::services::Payload::Bytes {
            content_type,
            filename,
            bytes,
            ..
        } => {
            assert_eq!(content_type, "application/x-ndjson; charset=utf-8");
            let fname = filename.unwrap();
            assert!(fname.starts_with("omnigraph-") && fname.ends_with(".ndjson"));
            let text = String::from_utf8(bytes).unwrap();
            assert!(text.contains("\"ContextAtom\""));
        }
        crate::services::Payload::Json(_) => panic!("expected Payload::Bytes for ndjson download"),
    }

    // 4. Import the exported bundle onto a fresh branch and inject into target session `s-recv`.
    let s_recv = "session-import-recv";
    let imported = mgr
        .import_context_json(&json!({
            "project_id": "proj-1",
            "target_branch": "imported/task-bundle",
            "from_branch": "main",
            "create_branch": true,
            "mode": "merge",
            "bundle": with_inherited["bundle"],
            "inject_into_sessions": true,
            "target_session_ids": [s_recv]
        }))
        .await
        .unwrap();
    assert_eq!(imported["ok"], true);
    assert_eq!(imported["target_branch"], "imported/task-bundle");
    assert!(imported["commit_id"].is_string());
    assert!(imported["imported_atoms"].as_u64().unwrap() >= 2);
    assert!(imported["imported_files"].as_u64().unwrap() >= 1);
    assert_eq!(imported["shared_session_ids"], json!([s_recv]));

    // 5. Re-import via raw NDJSON with `mode: "replace"` wipes old atoms on the target branch.
    let replace_ndjson = concat!(
        r#"{"type":"ContextAtom","id":"atom-replaced-only","data":{"project_id":"proj-1","branch":"imported/task-bundle","origin_session_id":"s-ext","kind":"insight","title":"Clean replacement atom","summary":"Only this remains","importance":0.8,"stale":false,"created_at":"2026-11-16T11:00:00.000Z","updated_at":"2026-11-16T11:00:00.000Z"}}"#,
        "\n",
        r#"{"type":"FileRef","id":"src/replaced.rs","data":{"path":"src/replaced.rs","project_id":"proj-1","language":"rust","last_touched_at":"2026-11-16T11:00:00.000Z"}}"#,
        "\n",
        r#"{"edge":"TouchesFile","id":"edge-tf-rep","from":"atom-replaced-only","to":"src/replaced.rs","data":{"access":"modified"}}"#
    );
    let replaced = mgr
        .import_context_json(&json!({
            "project_id": "proj-1",
            "target_branch": "imported/task-bundle",
            "mode": "replace",
            "raw_text": replace_ndjson
        }))
        .await
        .unwrap();
    assert_eq!(replaced["ok"], true);
    assert_eq!(replaced["imported_atoms"], 1);
    assert_eq!(replaced["imported_edges"], 1);

    let after_replace = mgr
        .export_context_json(&json!({
            "project_id": "proj-1",
            "branch": "imported/task-bundle",
            "scope": "branch",
            "format": "bundle"
        }))
        .await
        .unwrap();
    let after_ids: Vec<&str> = after_replace["bundle"]["atoms"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|a| a["id"].as_str())
        .collect();
    assert_eq!(after_ids, vec!["atom-replaced-only"]);

    // 6. Bundle object accepts snake_case `"edge": "relates_to"` with `"relation"`, `{from, to, relation}`
    // without `"edge"`/`"type"`, and preserves ContextAtom fallback `"branch"`/`"session_id"`.
    let flexible_bundle = json!({
        "format": "harness.omnigraph.bundle.v1",
        "atoms": [
            {
                "id": "atom-flex-1",
                "branch": "custom/origin-br",
                "session_id": "s-origin-99",
                "kind": "decision",
                "title": "Flexible bundle atom 1",
                "summary": "Carries branch and session_id fallbacks"
            },
            {
                "atom_id": "atom-flex-2",
                "branch": "custom/origin-br-2",
                "session_id": "s-origin-98",
                "kind": "insight",
                "title": "Flexible bundle atom 2",
                "summary": "Target of simplified edges"
            }
        ],
        "edges": [
            {
                "edge": "relates_to",
                "from": "atom-flex-1",
                "to": "atom-flex-2",
                "relation": "contradicts"
            },
            {
                "from": "atom-flex-2",
                "to": "atom-flex-1",
                "relation": "supports"
            }
        ]
    });
    let flex_imported = mgr
        .import_context_json(&json!({
            "project_id": "proj-1",
            "target_branch": "imported/flex-bundle",
            "from_branch": "main",
            "create_branch": true,
            "mode": "replace",
            "bundle": flexible_bundle
        }))
        .await
        .unwrap();
    assert_eq!(flex_imported["ok"], true);
    assert_eq!(flex_imported["imported_atoms"], 2);
    assert_eq!(flex_imported["imported_edges"], 2);

    let flex_exported = mgr
        .export_context_json(&json!({
            "project_id": "proj-1",
            "branch": "imported/flex-bundle",
            "scope": "branch",
            "format": "bundle"
        }))
        .await
        .unwrap();
    let flex_atoms = flex_exported["bundle"]["atoms"].as_array().unwrap();
    let atom_flex_1 = flex_atoms
        .iter()
        .find(|a| a["id"] == "atom-flex-1")
        .unwrap();
    assert_eq!(atom_flex_1["source_branch"], "custom/origin-br");
    assert_eq!(atom_flex_1["source_session_id"], "s-origin-99");
    let flex_edges = flex_exported["bundle"]["edges"].as_array().unwrap();
    assert!(flex_edges.iter().any(|e| {
        e["from"] == "atom-flex-1" && e["to"] == "atom-flex-2" && e["relation"] == "contradicts"
    }));
    assert!(flex_edges.iter().any(|e| {
        e["from"] == "atom-flex-2" && e["to"] == "atom-flex-1" && e["relation"] == "supports"
    }));

    // 7. Malformed/empty import fails cleanly.
    let empty_err = mgr
        .import_context_json(&json!({
            "project_id": "proj-1",
            "target_branch": "imported/task-bundle",
            "raw_text": "   "
        }))
        .await;
    assert!(empty_err.is_err());

    mgr.close().await.unwrap();
}

#[tokio::test]
async fn sidecar_link_sessions_shared_branch_and_broadcast_via_mock() {
    let (base_url, _server) = spawn_mock_omnigraph().await;
    let state_dir = tempfile::tempdir().unwrap();
    let store = crate::store::SessionStore::open(state_dir.path()).unwrap();
    let proj_dir = state_dir.path().join("project");
    std::fs::create_dir_all(&proj_dir).unwrap();
    let project = store.register(&proj_dir).unwrap();
    let project_id = project["id"].as_str().unwrap().to_string();

    let sess_a = store
        .create(
            &json!({
                "project_id": project_id,
                "provider": "claude",
                "workflow": "native",
                "mode": "plan",
                "agents_enabled": false,
                "title": "Backend auth task",
                "omnigraph": { "enabled": true, "mode": "session_branch" }
            }),
            "design auth",
            &[],
        )
        .unwrap();
    let sess_b = store
        .create(
            &json!({
                "project_id": project_id,
                "provider": "codex",
                "workflow": "native",
                "mode": "plan",
                "agents_enabled": false,
                "title": "Frontend login task",
                "omnigraph": { "enabled": true, "mode": "session_branch" }
            }),
            "build login form",
            &[],
        )
        .unwrap();
    let sess_c = store
        .create(
            &json!({
                "project_id": project_id,
                "provider": "claude",
                "workflow": "native",
                "mode": "plan",
                "agents_enabled": false,
                "title": "E2E suite",
                "omnigraph": { "enabled": true, "mode": "session_branch" }
            }),
            "write tests",
            &[],
        )
        .unwrap();

    let sid_a = sess_a["id"].as_str().unwrap().to_string();
    let sid_b = sess_b["id"].as_str().unwrap().to_string();
    let sid_c = sess_c["id"].as_str().unwrap().to_string();

    let mgr = OmnigraphManager::new(state_dir.path().to_path_buf(), Some(store.clone())).unwrap();
    mgr.connect_external(&base_url, Some("tok-harness"), None)
        .await
        .unwrap();

    // Publish one distinct atom per session branch (`sid_a` and `sid_b`).
    let br_a = crate::omnigraph::session_branch_name(&sid_a);
    let br_b = crate::omnigraph::session_branch_name(&sid_b);
    mgr.prepare_session_context(&sess_a, &proj_dir)
        .await
        .unwrap();
    mgr.prepare_session_context(&sess_b, &proj_dir)
        .await
        .unwrap();
    mgr.publish_atom_json(&json!({
        "project_id": project_id,
        "branch": br_a,
        "session_id": sid_a,
        "atom_id": "atom-sess-a",
        "kind": "decision",
        "title": "Session A endpoint schema",
        "summary": "POST /api/login returns token"
    }))
    .await
    .unwrap();
    mgr.publish_atom_json(&json!({
        "project_id": project_id,
        "branch": br_b,
        "session_id": sid_b,
        "atom_id": "atom-sess-b",
        "kind": "insight",
        "title": "Session B form validation",
        "summary": "Require non-empty password"
    }))
    .await
    .unwrap();

    // Validation check: fewer than 2 sessions rejected.
    let too_few = mgr
        .link_sessions_json(
            &json!({
                "project_id": project_id,
                "session_ids": [sid_a],
                "mode": "shared_branch"
            }),
            &store,
        )
        .await;
    assert!(too_few.is_err());

    // Mutual `shared_branch` sharing between sid_a and sid_b.
    let link_req = json!({
        "project_id": project_id,
        "session_ids": [sid_a, sid_b],
        "mode": "shared_branch",
        "space_name": "Team Auth",
        "inject_live": true
    });
    let shared = mgr.link_sessions_json(&link_req, &store).await.unwrap();
    assert_eq!(shared["ok"], true);
    assert_eq!(shared["mode"], "shared_branch");
    assert_eq!(shared["shared_branch"], "shared/team-auth");
    assert_eq!(shared["shared_space"], "Team Auth");
    assert!(shared["synced_atoms"].as_u64().unwrap() >= 2);
    assert_eq!(shared["conflicts"], json!([]));
    assert_eq!(
        shared["linked_session_ids"],
        json!([sid_a.clone(), sid_b.clone()])
    );

    let synced_rows =
        crate::sessions::sync_omnigraph_link_to_sessions(&store, None, &link_req, &shared).await;
    assert_eq!(synced_rows.len(), 2);

    // Graph view and status reflect `SessionFollows` and `shared_space`.
    let gv = mgr
        .graph_view_json(
            &project_id,
            Some("shared/team-auth"),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let edges = gv["edges"].as_array().unwrap();
    let follow_count = edges
        .iter()
        .filter(|e| e["kind"] == "session_follows")
        .count();
    assert!(
        follow_count >= 2,
        "expected >=2 session_follows edges in graph view: {edges:?}"
    );
    let sessions = gv["sessions"].as_array().unwrap();
    assert!(
        sessions
            .iter()
            .any(|s| s["shared_space"] == json!("Team Auth")),
        "expected shared_space on session in graph view: {sessions:?}"
    );
    let status = mgr.status_json(&project_id).await.unwrap();
    assert!(status["stats"]["branches"].as_u64().unwrap_or(0) >= 3);

    // Broadcast from sid_a to sid_c links SessionFollows and syncs sid_a's branch.
    let broadcast = mgr
        .link_sessions_json(
            &json!({
                "project_id": project_id,
                "mode": "broadcast",
                "source_session_id": sid_a,
                "session_ids": [sid_c],
                "inject_live": true
            }),
            &store,
        )
        .await
        .unwrap();
    assert_eq!(broadcast["ok"], true);
    assert_eq!(broadcast["mode"], "broadcast");
    assert_eq!(broadcast["shared_branch"], json!(br_a));
    assert_eq!(
        broadcast["linked_session_ids"],
        json!([sid_a.clone(), sid_c.clone()])
    );

    // 4. `target_branch` accepts human space names (`"Sprint Space"` -> `"shared/sprint-space"`)
    // and `"shared/Team Alpha"` -> `"shared/team-alpha"`, while rejecting empty/traversal strings.
    let human_target = mgr
        .link_sessions_json(
            &json!({
                "project_id": project_id,
                "session_ids": [sid_a, sid_b],
                "mode": "shared_branch",
                "target_branch": "Sprint Space"
            }),
            &store,
        )
        .await
        .unwrap();
    assert_eq!(human_target["shared_branch"], "shared/sprint-space");

    let prefixed_human_target = mgr
        .link_sessions_json(
            &json!({
                "project_id": project_id,
                "session_ids": [sid_a, sid_b],
                "mode": "shared_branch",
                "target_branch": "shared/Team Alpha"
            }),
            &store,
        )
        .await
        .unwrap();
    assert_eq!(prefixed_human_target["shared_branch"], "shared/team-alpha");

    for bad_branch in [
        "",
        "   ",
        "../escape",
        "shared/..",
        "shared/",
        "bad/with space",
    ] {
        let err = mgr
            .link_sessions_json(
                &json!({
                    "project_id": project_id,
                    "session_ids": [sid_a, sid_b],
                    "mode": "shared_branch",
                    "target_branch": bad_branch
                }),
                &store,
            )
            .await;
        assert!(
            err.is_err(),
            "expected error for invalid target_branch `{bad_branch}`"
        );
    }

    mgr.close().await.unwrap();
}

#[tokio::test]
async fn real_omnigraph_binary_export_import_and_share_round_trip() {
    let bin_cli = std::path::Path::new("/tmp/omnigraph-v0.15.0/omnigraph");
    let bin_srv = std::path::Path::new("/tmp/omnigraph-v0.15.0/omnigraph-server");
    if !bin_cli.is_file() || !bin_srv.is_file() {
        eprintln!(
            "skipping real_omnigraph_binary_export_import_and_share_round_trip: /tmp/omnigraph-v0.15.0 not present"
        );
        return;
    }

    let state_dir = tempfile::tempdir().unwrap();
    let store = crate::store::SessionStore::open(state_dir.path()).unwrap();
    let proj_dir = state_dir.path().join("project");
    std::fs::create_dir_all(&proj_dir).unwrap();
    let project = store.register(&proj_dir).unwrap();
    let project_id = project["id"].as_str().unwrap().to_string();

    let sess_a = store
        .create(
            &json!({
                "project_id": project_id,
                "provider": "claude",
                "workflow": "native",
                "mode": "plan",
                "agents_enabled": false,
                "title": "Real OG Session A",
                "omnigraph": { "enabled": true, "mode": "session_branch" }
            }),
            "task A",
            &[],
        )
        .unwrap();
    let sess_b = store
        .create(
            &json!({
                "project_id": project_id,
                "provider": "codex",
                "workflow": "native",
                "mode": "plan",
                "agents_enabled": false,
                "title": "Real OG Session B",
                "omnigraph": { "enabled": true, "mode": "session_branch" }
            }),
            "task B",
            &[],
        )
        .unwrap();
    let sid_a = sess_a["id"].as_str().unwrap().to_string();
    let sid_b = sess_b["id"].as_str().unwrap().to_string();

    let mgr = OmnigraphManager::new(state_dir.path().to_path_buf(), Some(store.clone())).unwrap();
    std::fs::create_dir_all(mgr.tools_dir()).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(bin_cli, mgr.tools_dir().join("omnigraph")).unwrap();
        std::os::unix::fs::symlink(bin_srv, mgr.tools_dir().join("omnigraph-server")).unwrap();
    }
    #[cfg(not(unix))]
    {
        std::fs::copy(bin_cli, mgr.tools_dir().join("omnigraph")).unwrap();
        std::fs::copy(bin_srv, mgr.tools_dir().join("omnigraph-server")).unwrap();
    }

    let started = mgr.start_local(None).await.unwrap();
    assert_eq!(started["connected"], true);

    let br_a = crate::omnigraph::session_branch_name(&sid_a);
    mgr.prepare_session_context(&sess_a, &proj_dir)
        .await
        .unwrap();
    mgr.prepare_session_context(&sess_b, &proj_dir)
        .await
        .unwrap();
    mgr.publish_atom_json(&json!({
        "project_id": project_id,
        "branch": br_a,
        "session_id": sid_a,
        "atom_id": "atom-real-1",
        "kind": "decision",
        "title": "Real Omnigraph decision",
        "summary": "Persisted into real RocksDB graph",
        "files": ["src/real.rs"]
    }))
    .await
    .unwrap();

    // 1. Real export from session branch
    let exported = mgr
        .export_context_json(&json!({
            "project_id": project_id,
            "session_id": sid_a,
            "scope": "task",
            "include_inherited": false,
            "format": "bundle"
        }))
        .await
        .unwrap();
    assert_eq!(exported["ok"], true);
    assert!(exported["counts"]["atoms"].as_u64().unwrap() >= 1);
    assert!(exported["counts"]["edges"].as_u64().unwrap() >= 1);
    assert!(exported["ndjson"].as_str().unwrap().contains("atom-real-1"));

    // 2. Real import (`create_branch: true`, `mode: "merge"`, then `"replace"`)
    let imported = mgr
        .import_context_json(&json!({
            "project_id": project_id,
            "target_branch": "imported/real-bundle",
            "from_branch": "main",
            "create_branch": true,
            "mode": "merge",
            "bundle": exported["bundle"],
            "inject_into_sessions": true,
            "target_session_ids": [sid_b]
        }))
        .await
        .unwrap();
    assert_eq!(imported["ok"], true);
    assert!(imported["imported_atoms"].as_u64().unwrap() >= 1);
    assert!(imported["imported_edges"].as_u64().unwrap() >= 1);
    assert_eq!(imported["shared_session_ids"], json!([sid_b]));

    let replace_raw = r#"{"type":"ContextAtom","id":"atom-real-replaced","data":{"project_id":"__PROJ__","branch":"imported/real-bundle","origin_session_id":"s-ext","kind":"insight","title":"Replaced atom on real graph","summary":"Previous edges cleared before load","importance":0.7,"stale":false,"created_at":"2026-11-16T12:00:00.000Z","updated_at":"2026-11-16T12:00:00.000Z"}}"#
        .replace("__PROJ__", &project_id);
    let replaced = mgr
        .import_context_json(&json!({
            "project_id": project_id,
            "target_branch": "imported/real-bundle",
            "mode": "replace",
            "raw_text": replace_raw
        }))
        .await
        .unwrap();
    assert_eq!(replaced["ok"], true);
    assert_eq!(replaced["imported_atoms"], 1);

    let re_export = mgr
        .export_context_json(&json!({
            "project_id": project_id,
            "branch": "imported/real-bundle",
            "scope": "branch",
            "format": "bundle"
        }))
        .await
        .unwrap();
    let real_ids: Vec<&str> = re_export["bundle"]["atoms"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|a| a["id"].as_str())
        .collect();
    assert_eq!(real_ids, vec!["atom-real-replaced"]);

    // 3. Real multi-session sharing (`shared_branch`) + graph view verification
    let linked = mgr
        .link_sessions_json(
            &json!({
                "project_id": project_id,
                "session_ids": [sid_a, sid_b],
                "mode": "shared_branch",
                "space_name": "Real Workspace",
                "inject_live": true
            }),
            &store,
        )
        .await
        .unwrap();
    assert_eq!(linked["ok"], true);
    assert_eq!(linked["shared_branch"], "shared/real-workspace");
    assert!(linked["synced_atoms"].as_u64().unwrap() >= 1);

    let gv = mgr
        .graph_view_json(
            &project_id,
            Some("shared/real-workspace"),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(
        gv["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "session_follows")
    );

    // 4. Real Omnigraph provenance, canonical sync, read-only canonical retrieval, fetch_atom, link_brain, and export/import round-trip
    let sync_res = mgr
        .sync_canonical_knowledge(
            &project_id,
            &[
                json!({
                    "atom_id": "brain-real-old",
                    "kind": "finding",
                    "title": "Legacy session token rule",
                    "summary": "Superseded rule",
                    "files": ["src/real.rs"],
                    "verified": true,
                    "origin_layer": "project-brain",
                    "authority": "verified",
                    "privacy": "team",
                    "brain_record_id": "brain-uuid-old",
                    "record_revision": 1
                }),
                json!({
                    "atom_id": "mem-real-1",
                    "kind": "decision",
                    "title": "PKCE required for OAuth flows",
                    "summary": "Verified memory bank decision for Real Omnigraph",
                    "files": ["src/oauth.rs"],
                    "verified": true,
                    "origin_layer": "memory-bank",
                    "authority": "verified",
                    "privacy": "public",
                    "memory_id": "MEM-20261117-aabbccdd-pkce",
                    "record_revision": 4,
                    "supersedes_atom_ids": ["brain-real-old", "missing-target-id"]
                }),
            ],
        )
        .await
        .unwrap();
    assert_eq!(sync_res["ok"], true);
    assert_eq!(sync_res["branch"], schema::CANONICAL_KNOWLEDGE_BRANCH);
    assert_eq!(sync_res["synced"], 2);

    let prep_real = mgr
        .prepare_session_context(&sess_b, &proj_dir)
        .await
        .unwrap();
    let prep_atoms = prep_real["atoms"].as_array().unwrap();
    let canon_hit = prep_atoms
        .iter()
        .find(|a| a["atom_id"] == "mem-real-1")
        .expect("canonical atom surfaced in prepare_session_context");
    assert_eq!(canon_hit["origin_layer"], "memory-bank");
    assert_eq!(canon_hit["authority"], "verified");
    assert_eq!(canon_hit["privacy"], "public");
    assert_eq!(canon_hit["memory_id"], "MEM-20261117-aabbccdd-pkce");
    assert_eq!(canon_hit["record_revision"], 4);
    assert_eq!(canon_hit["files"], json!(["src/oauth.rs"]));
    assert!(
        prep_real["capsule_text"]
            .as_str()
            .unwrap()
            .contains("[memory-bank decision · MEM-20261117-aabbccdd-pkce]")
    );

    let fetched = mgr
        .fetch_atom_with_files(&project_id, "atom-real-1", None)
        .await
        .unwrap();
    assert_eq!(fetched["files"], json!(["src/real.rs"]));
    let linked_brain = mgr
        .link_atom_to_brain(
            &project_id,
            fetched["branch"].as_str().unwrap(),
            &fetched["atom"],
            "brain-uuid-real-99",
            7,
            Some("team"),
        )
        .await
        .unwrap();
    assert_eq!(linked_brain["ok"], true);
    assert_eq!(linked_brain["origin_layer"], "project-brain");
    assert_eq!(linked_brain["authority"], "verified");
    assert_eq!(linked_brain["privacy"], "team");
    assert_eq!(linked_brain["verified"], true);
    assert_eq!(linked_brain["brain_record_id"], "brain-uuid-real-99");
    assert_eq!(linked_brain["record_revision"], 7);
    assert_eq!(linked_brain["files"], json!(["src/real.rs"]));

    let canon_export = mgr
        .export_context_json(&json!({
            "project_id": project_id,
            "branch": schema::CANONICAL_KNOWLEDGE_BRANCH,
            "scope": "branch",
            "format": "bundle"
        }))
        .await
        .unwrap();
    let canon_import = mgr
        .import_context_json(&json!({
            "project_id": project_id,
            "target_branch": "imported/real-provenance",
            "from_branch": "main",
            "create_branch": true,
            "mode": "replace",
            "bundle": canon_export["bundle"]
        }))
        .await
        .unwrap();
    assert_eq!(canon_import["ok"], true);
    let imported_mem = canon_import["atoms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["atom_id"] == "mem-real-1")
        .expect("imported mem-real-1");
    assert_eq!(imported_mem["origin_layer"], "memory-bank");
    assert_eq!(imported_mem["memory_id"], "MEM-20261117-aabbccdd-pkce");
    assert_eq!(imported_mem["record_revision"], 4);

    mgr.close().await.unwrap();
}

#[tokio::test]
async fn sync_canonical_knowledge_deletes_retired_archived_and_stale_atoms() {
    let state_dir = tempfile::tempdir().unwrap();
    let store = crate::store::SessionStore::open(state_dir.path()).unwrap();
    let mut projects = Vec::new();
    for name in ["project", "other"] {
        let dir = state_dir.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        projects.push(
            store.register(&dir).unwrap()["id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    let (project_id, other_id) = (projects[0].clone(), projects[1].clone());
    let (base_url, _server) = spawn_mock_omnigraph().await;
    let mgr = OmnigraphManager::new(state_dir.path().to_path_buf(), Some(store.clone())).unwrap();
    mgr.setup_json(&json!({
        "action": "connect",
        "endpoint": base_url,
        "token": "tok-harness",
        "graph_id": schema::HARNESS_GRAPH_ID
    }))
    .await
    .unwrap();
    let atom = |id: &str, layer: &str, files: &[&str]| json!({"atom_id": id, "kind": "finding", "title": id, "files": files, "verified": true, "origin_layer": layer, "authority": "verified", "privacy": "public"});
    let first = mgr
        .sync_canonical_knowledge_with_options(
            &project_id,
            &[
                atom("brain-kept", "project-brain", &["src/kept.rs"]),
                atom("brain-retired", "project-brain", &["src/retired.rs"]),
                atom("mem-archived", "memory-bank", &[]),
            ],
            &["project-brain".into(), "memory-bank".into()],
        )
        .await
        .unwrap();
    assert_eq!(first["synced"], 3);
    assert_eq!(first["deleted"], 0);
    // Atoms the sync does not own: another project's, and one a session
    // published onto the branch.
    mgr.sync_canonical_knowledge(&other_id, &[atom("brain-elsewhere", "project-brain", &[])])
        .await
        .unwrap();
    mgr.sync_canonical_knowledge(&project_id, &[atom("brain-session-copy", "session", &[])])
        .await
        .unwrap();

    // Without pruning nothing leaves the branch.
    let plain = mgr
        .sync_canonical_knowledge(
            &project_id,
            &[atom("brain-kept", "project-brain", &["src/kept.rs"])],
        )
        .await
        .unwrap();
    assert_eq!(plain["deleted"], 0);
    // A Brain read whole prunes its own gone records only.
    let brain_only = mgr
        .sync_canonical_knowledge_with_options(
            &project_id,
            &[atom("brain-kept", "project-brain", &["src/kept.rs"])],
            &["project-brain".into()],
        )
        .await
        .unwrap();
    assert_eq!(brain_only["deleted_atom_ids"], json!(["brain-retired"]));
    let both = mgr
        .sync_canonical_knowledge_with_options(
            &project_id,
            &[atom("brain-kept", "project-brain", &["src/kept.rs"])],
            &["project-brain".into(), "memory-bank".into()],
        )
        .await
        .unwrap();
    assert_eq!(both["deleted_atom_ids"], json!(["mem-archived"]));

    let view = mgr
        .graph_view_json(
            &project_id,
            Some(schema::CANONICAL_KNOWLEDGE_BRANCH),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let ids = view["atoms"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|atom| atom["atom_id"].as_str())
        .map(String::from)
        .collect::<BTreeSet<_>>();
    assert!(ids.contains("brain-kept"), "{ids:?}");
    assert!(ids.contains("brain-session-copy"), "{ids:?}");
    assert!(!ids.contains("brain-retired"), "{ids:?}");
    assert!(!ids.contains("mem-archived"), "{ids:?}");
    // The retired atom's file edge left with it.
    let edges = view["edges"].as_array().unwrap();
    assert!(
        !edges
            .iter()
            .any(|edge| edge.to_string().contains("brain-retired")),
        "{edges:?}"
    );
    let other = mgr
        .graph_view_json(
            &other_id,
            Some(schema::CANONICAL_KNOWLEDGE_BRANCH),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(
        other["atoms"]
            .as_array()
            .unwrap()
            .iter()
            .any(|atom| atom["atom_id"] == "brain-elsewhere")
    );
    mgr.close().await.unwrap();
}

#[tokio::test]
async fn provenance_defaults_canonical_sync_fetch_and_link_brain_work() {
    let state_dir = tempfile::tempdir().unwrap();
    let store = crate::store::SessionStore::open(state_dir.path()).unwrap();
    let proj_dir = state_dir.path().join("project");
    std::fs::create_dir_all(&proj_dir).unwrap();
    let project = store.register(&proj_dir).unwrap();
    let project_id = project["id"].as_str().unwrap().to_string();

    // 1. OmnigraphMode::Off returns skipped payload gracefully for sync_canonical_knowledge
    let off_mgr =
        OmnigraphManager::new(state_dir.path().to_path_buf(), Some(store.clone())).unwrap();
    let off_sync = off_mgr
        .sync_canonical_knowledge(&project_id, &[json!({"atom_id": "a1", "title": "ignored"})])
        .await
        .unwrap();
    assert_eq!(
        off_sync,
        json!({"ok": false, "skipped": "omnigraph-off", "synced": 0, "atom_ids": []})
    );

    let (base_url, _server) = spawn_mock_omnigraph().await;
    let mgr = Arc::new(
        OmnigraphManager::new(state_dir.path().to_path_buf(), Some(store.clone())).unwrap(),
    );
    mgr.setup_json(&json!({
        "action": "connect",
        "endpoint": base_url,
        "token": "tok-harness",
        "graph_id": schema::HARNESS_GRAPH_ID
    }))
    .await
    .unwrap();
    let domain = OmnigraphDomain::new(mgr.clone());

    // 2. publish_atom_json keeps verified=false by default and echoes the 6 provenance fields
    let pub_unverified = mgr
        .publish_atom_json(&json!({
            "project_id": project_id,
            "branch": "main",
            "atom_id": "atom-default-unverified",
            "kind": "insight",
            "title": "Unverified atom default",
            "summary": "No verified flag provided",
            "origin_layer": "session-draft",
            "authority": "inferred",
            "privacy": "team",
            "brain_record_id": "rec-draft-1",
            "memory_id": "MEM-draft-1",
            "record_revision": 2
        }))
        .await
        .unwrap();
    assert_eq!(pub_unverified["atom"]["verified"], false);
    assert_eq!(pub_unverified["atom"]["origin_layer"], "session-draft");
    assert_eq!(pub_unverified["atom"]["authority"], "inferred");
    assert_eq!(pub_unverified["atom"]["privacy"], "team");
    assert_eq!(pub_unverified["atom"]["brain_record_id"], "rec-draft-1");
    assert_eq!(pub_unverified["atom"]["memory_id"], "MEM-draft-1");
    assert_eq!(pub_unverified["atom"]["record_revision"], 2);

    // 3. publish_session_outputs defaults verified to false and forwards provenance fields
    let sess = store
        .create(
            &json!({
                "project_id": project_id,
                "provider": "codex",
                "workflow": "native",
                "mode": "plan",
                "agents_enabled": false,
                "title": "Session draft provenance",
                "goal": "oauth token refresh",
                "omnigraph": { "enabled": true, "branch": "session/sessprov01", "query": "oauth token refresh" }
            }),
            "oauth token refresh",
            &[],
        )
        .unwrap();
    let _sid = sess["id"].as_str().unwrap();
    let pub_out = mgr
        .publish_session_outputs(
            &sess,
            &[json!({
                "kind": "finding",
                "title": "Session oauth refresh finding",
                "summary": "Observed retry race on refresh token rotation",
                "files": ["src/oauth_client.rs"],
                "origin_layer": "session-draft",
                "authority": "inferred",
                "privacy": "team",
                "record_revision": 1
            })],
            &["src/oauth_client.rs".into()],
            "Investigated refresh token rotation",
        )
        .await
        .unwrap();
    assert_eq!(pub_out["ok"], true);
    let session_atom_id = pub_out["published_atom_ids"][0]
        .as_str()
        .unwrap()
        .to_string();

    let sess_graph_before = mgr
        .graph_view_json(
            &project_id,
            Some("session/sessprov01"),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let published_sess_atom = sess_graph_before["atoms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["atom_id"] == session_atom_id)
        .expect("session atom");
    assert_eq!(published_sess_atom["verified"], false);
    assert_eq!(published_sess_atom["origin_layer"], "session-draft");
    assert_eq!(published_sess_atom["authority"], "inferred");
    assert_eq!(published_sess_atom["privacy"], "team");
    assert_eq!(published_sess_atom["record_revision"], 1);
    assert_eq!(published_sess_atom["files"], json!(["src/oauth_client.rs"]));

    // 4. omnigraph.sync_items creates knowledge/canonical, preserves provenance + files, and adds supersedes only for existing targets
    let sync_payload = match domain
        .execute(
            "omnigraph.sync_items",
            json!({
                "params": {},
                "query": {},
                "body": {
                    "project_id": project_id,
                    "items": [
                        {
                            "atom_id": "brain-101",
                            "kind": "finding",
                            "title": "Legacy oauth refresh workaround",
                            "summary": "Old workaround",
                            "files": ["src/legacy_oauth.rs"],
                            "verified": true,
                            "origin_layer": "project-brain",
                            "authority": "verified",
                            "privacy": "public",
                            "brain_record_id": "550e8400-e29b-41d4-a716-446655440101",
                            "record_revision": 1
                        },
                        {
                            "atom_id": "mem-202",
                            "kind": "decision",
                            "title": "Use atomic oauth refresh lock",
                            "summary": "Serialized refresh prevents double revoke",
                            "body": "Always acquire single-flight lock before refreshing oauth tokens.",
                            "tags": ["oauth", "concurrency"],
                            "files": ["src/oauth_lock.rs", "src/oauth_client.rs"],
                            "verified": true,
                            "origin_layer": "memory-bank",
                            "authority": "verified",
                            "privacy": "team",
                            "memory_id": "MEM-20261117-12345678-oauth-lock",
                            "brain_record_id": "550e8400-e29b-41d4-a716-446655440202",
                            "record_revision": 3,
                            "supersedes_atom_ids": ["brain-101", "does-not-exist-999"]
                        }
                    ]
                }
            }),
        )
        .await
        .unwrap()
    {
        crate::services::Payload::Json(v) => v,
        _ => panic!("expected json"),
    };
    assert_eq!(sync_payload["ok"], true);
    assert_eq!(sync_payload["branch"], schema::CANONICAL_KNOWLEDGE_BRANCH);
    assert_eq!(sync_payload["synced"], 2);
    assert_eq!(sync_payload["atom_ids"], json!(["brain-101", "mem-202"]));

    let canon_graph = mgr
        .graph_view_json(
            &project_id,
            Some(schema::CANONICAL_KNOWLEDGE_BRANCH),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let canon_edges = canon_graph["edges"].as_array().unwrap();
    assert!(canon_edges.iter().any(|e| {
        e["kind"] == "relates_to"
            && e["relation"] == "supersedes"
            && e["from"] == "mem-202"
            && e["to"] == "brain-101"
    }));
    assert!(!canon_edges.iter().any(|e| e["to"] == "does-not-exist-999"));

    // 5. prepare_session_context reads knowledge/canonical read-only without mutating session branch,
    //    attaches `files` from GQ_LIST_FILE_EDGES, preserves provenance, and tags capsule lines clearly.
    let prep = mgr.prepare_session_context(&sess, &proj_dir).await.unwrap();
    let atoms = prep["atoms"].as_array().unwrap();
    let sess_atom = atoms
        .iter()
        .find(|a| a["atom_id"] == session_atom_id)
        .expect("session atom in prepared context");
    assert_eq!(sess_atom["files"], json!(["src/oauth_client.rs"]));
    assert_eq!(sess_atom["verified"], false);
    assert_eq!(sess_atom["origin_layer"], "session-draft");

    let canon_mem = atoms
        .iter()
        .find(|a| a["atom_id"] == "mem-202")
        .expect("canonical memory-bank atom in prepared context");
    let mut canon_files: Vec<String> = canon_mem["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    canon_files.sort();
    assert_eq!(
        canon_files,
        vec!["src/oauth_client.rs", "src/oauth_lock.rs"]
    );
    assert_eq!(canon_mem["memory_id"], "MEM-20261117-12345678-oauth-lock");
    assert_eq!(
        canon_mem["brain_record_id"],
        "550e8400-e29b-41d4-a716-446655440202"
    );
    assert_eq!(canon_mem["record_revision"], 3);

    let capsule = prep["capsule_text"].as_str().unwrap();
    assert!(capsule.contains("subordinate to canonical"));
    assert!(capsule.contains("[memory-bank decision · MEM-20261117-12345678-oauth-lock]"));
    assert!(
        capsule.contains("[project-brain finding · 550e8400]")
            || !atoms.iter().any(|a| a["atom_id"] == "brain-101")
    );
    assert!(capsule.contains("[unverified reference — verify against code & Brain]"));

    // Session branch must NOT have canonical atoms merged/mutated onto it
    let sess_graph_after = mgr
        .graph_view_json(
            &project_id,
            Some("session/sessprov01"),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(
        !sess_graph_after["atoms"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["atom_id"] == "mem-202" || a["atom_id"] == "brain-101")
    );

    // Also verify capsule tags [project-brain ...] when a verified brain atom is matched
    let mut brain_atom_check = canon_graph["atoms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["atom_id"] == "brain-101")
        .unwrap()
        .clone();
    brain_atom_check["verified"] = json!(true);
    let cap_both = super::sidecar::format_context_capsule_for_test(
        schema::HARNESS_GRAPH_ID,
        "session/sessprov01",
        &proj_dir,
        &[brain_atom_check, canon_mem.clone(), sess_atom.clone()],
    );
    assert!(cap_both.contains("[project-brain finding · 550e8400]"));
    assert!(cap_both.contains("[memory-bank decision · MEM-20261117-12345678-oauth-lock]"));
    assert!(cap_both.contains("[unverified reference — verify against code & Brain]"));

    // 6. omnigraph.fetch_atom locates atom across branches with files; missing returns NotFound
    let fetch_res = match domain
        .execute(
            "omnigraph.fetch_atom",
            json!({
                "params": {},
                "query": {},
                "body": {
                    "project_id": project_id,
                    "atom_id": session_atom_id
                }
            }),
        )
        .await
        .unwrap()
    {
        crate::services::Payload::Json(v) => v,
        _ => panic!("expected json"),
    };
    assert_eq!(fetch_res["branch"], "session/sessprov01");
    assert_eq!(fetch_res["files"], json!(["src/oauth_client.rs"]));
    assert_eq!(fetch_res["atom"]["files"], json!(["src/oauth_client.rs"]));

    let Err(missing_err) = domain
        .execute(
            "omnigraph.fetch_atom",
            json!({
                "params": {},
                "query": {},
                "body": {
                    "project_id": project_id,
                    "atom_id": "nonexistent-atom-xyz"
                }
            }),
        )
        .await
    else {
        panic!("expected missing atom error");
    };
    assert_eq!(missing_err.status, 404);

    // 7. omnigraph.link_brain preserves content/tags/files while setting verified Project Brain provenance + optional privacy
    let mut atom_to_link = fetch_res["atom"].clone();
    atom_to_link["tags"] = json!(["oauth", "promoted"]);
    let link_res = match domain
        .execute(
            "omnigraph.link_brain",
            json!({
                "params": {},
                "query": {},
                "body": {
                    "project_id": project_id,
                    "branch": fetch_res["branch"],
                    "atom": atom_to_link,
                    "brain_record_id": "brain-promoted-uuid-77",
                    "revision": 4,
                    "privacy": "public"
                }
            }),
        )
        .await
        .unwrap()
    {
        crate::services::Payload::Json(v) => v,
        _ => panic!("expected json"),
    };
    assert_eq!(link_res["ok"], true);
    assert_eq!(link_res["origin_layer"], "project-brain");
    assert_eq!(link_res["authority"], "verified");
    assert_eq!(link_res["privacy"], "public");
    assert_eq!(link_res["verified"], true);
    assert_eq!(link_res["brain_record_id"], "brain-promoted-uuid-77");
    assert_eq!(link_res["record_revision"], 4);
    assert_eq!(link_res["atom"]["privacy"], "public");
    assert_eq!(link_res["atom"]["tags"], json!(["oauth", "promoted"]));
    assert_eq!(link_res["atom"]["files"], json!(["src/oauth_client.rs"]));
    assert_eq!(link_res["atom"]["title"], "Session oauth refresh finding");

    let refetched = mgr
        .fetch_atom_with_files(&project_id, &session_atom_id, Some("session/sessprov01"))
        .await
        .unwrap();
    assert_eq!(refetched["atom"]["verified"], true);
    assert_eq!(refetched["atom"]["origin_layer"], "project-brain");
    assert_eq!(refetched["atom"]["authority"], "verified");
    assert_eq!(refetched["atom"]["privacy"], "public");
    assert_eq!(
        refetched["atom"]["brain_record_id"],
        "brain-promoted-uuid-77"
    );
    assert_eq!(refetched["atom"]["record_revision"], 4);
    assert_eq!(refetched["atom"]["tags"], json!(["oauth", "promoted"]));
    assert_eq!(refetched["files"], json!(["src/oauth_client.rs"]));

    // 8. fetch_atom_with_files falls back from stale/deleted explicit branch to main/canonical/listed branches
    let fallback_fetch = mgr
        .fetch_atom_with_files(
            &project_id,
            &session_atom_id,
            Some("session/deleted-branch"),
        )
        .await
        .unwrap();
    assert_eq!(fallback_fetch["branch"], "session/sessprov01");
    assert_eq!(fallback_fetch["files"], json!(["src/oauth_client.rs"]));
    assert_eq!(fallback_fetch["atom"]["privacy"], "public");

    let canon_fallback = mgr
        .fetch_atom_with_files(&project_id, "mem-202", Some("session/deleted-branch"))
        .await
        .unwrap();
    assert_eq!(canon_fallback["branch"], schema::CANONICAL_KNOWLEDGE_BRANCH);

    // 9. share_into_session_json (`mode: "copy"`) copies TouchesFile edges + FileRef to target_branch
    let share_copy = mgr
        .share_into_session_json(&json!({
            "project_id": project_id,
            "target_session_id": "target-copy-files-1",
            "atom_ids": [session_atom_id],
            "source_branch": "session/sessprov01",
            "mode": "copy"
        }))
        .await
        .unwrap();
    assert_eq!(share_copy["ok"], true);
    assert_eq!(share_copy["shared_count"], 1);
    let copied_branch = share_copy["target_branch"].as_str().unwrap();
    let copied_fetch = mgr
        .fetch_atom_with_files(&project_id, &session_atom_id, Some(copied_branch))
        .await
        .unwrap();
    assert_eq!(copied_fetch["branch"], copied_branch);
    assert_eq!(copied_fetch["files"], json!(["src/oauth_client.rs"]));
    assert_eq!(copied_fetch["atom"]["privacy"], "public");

    mgr.close().await.unwrap();
}

#[tokio::test]
async fn transfer_round_trip_with_and_without_provenance_fields() {
    let (base_url, _server) = spawn_mock_omnigraph().await;
    let state_dir = tempfile::tempdir().unwrap();
    let mgr = OmnigraphManager::new(state_dir.path().to_path_buf(), None).unwrap();
    mgr.setup_json(&json!({
        "action": "connect",
        "endpoint": base_url,
        "token": "tok-harness",
        "graph_id": schema::HARNESS_GRAPH_ID
    }))
    .await
    .unwrap();

    // Publish one atom WITH all 6 provenance fields onto main
    mgr.publish_atom_json(&json!({
        "project_id": "proj-xfer",
        "branch": "main",
        "atom_id": "atom-prov-full",
        "kind": "decision",
        "title": "Provenance-bearing atom",
        "summary": "Carries all 6 nullable fields",
        "files": ["src/prov.rs"],
        "verified": true,
        "origin_layer": "memory-bank",
        "authority": "verified",
        "privacy": "team",
        "brain_record_id": "brain-xfer-1",
        "memory_id": "MEM-20261117-deadbeef-prov",
        "record_revision": 9
    }))
    .await
    .unwrap();

    let exported = mgr
        .export_context_json(&json!({
            "project_id": "proj-xfer",
            "branch": "main",
            "scope": "branch",
            "format": "bundle"
        }))
        .await
        .unwrap();
    let exp_atom = exported["bundle"]["atoms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["atom_id"] == "atom-prov-full")
        .expect("exported atom");
    assert_eq!(exp_atom["origin_layer"], "memory-bank");
    assert_eq!(exp_atom["authority"], "verified");
    assert_eq!(exp_atom["privacy"], "team");
    assert_eq!(exp_atom["brain_record_id"], "brain-xfer-1");
    assert_eq!(exp_atom["memory_id"], "MEM-20261117-deadbeef-prov");
    assert_eq!(exp_atom["record_revision"], 9);

    // Import bundle WITH provenance fields + legacy bundle WITHOUT provenance fields
    let reimported = mgr
        .import_context_json(&json!({
            "project_id": "proj-xfer",
            "target_branch": "imported/prov-check",
            "from_branch": "main",
            "create_branch": true,
            "mode": "replace",
            "bundle": exported["bundle"]
        }))
        .await
        .unwrap();
    let reimp_atom = reimported["atoms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["atom_id"] == "atom-prov-full")
        .expect("reimported atom");
    assert_eq!(reimp_atom["origin_layer"], "memory-bank");
    assert_eq!(reimp_atom["authority"], "verified");
    assert_eq!(reimp_atom["privacy"], "team");
    assert_eq!(reimp_atom["brain_record_id"], "brain-xfer-1");
    assert_eq!(reimp_atom["memory_id"], "MEM-20261117-deadbeef-prov");
    assert_eq!(reimp_atom["record_revision"], 9);
    assert_eq!(reimp_atom["files"], json!(["src/prov.rs"]));

    let legacy_bundle = json!({
        "format": "harness.omnigraph.bundle.v1",
        "atoms": [
            {
                "id": "atom-legacy-noprovenance",
                "kind": "insight",
                "title": "Legacy atom missing provenance",
                "summary": "Should import cleanly with null provenance"
            }
        ],
        "edges": []
    });
    let legacy_imp = mgr
        .import_context_json(&json!({
            "project_id": "proj-xfer",
            "target_branch": "imported/legacy-noprovenance",
            "from_branch": "main",
            "create_branch": true,
            "mode": "replace",
            "bundle": legacy_bundle
        }))
        .await
        .unwrap();
    assert_eq!(legacy_imp["ok"], true);
    assert_eq!(legacy_imp["imported_atoms"], 1);
    let leg_atom = &legacy_imp["atoms"][0];
    assert_eq!(leg_atom["atom_id"], "atom-legacy-noprovenance");
    for field in [
        "origin_layer",
        "authority",
        "privacy",
        "brain_record_id",
        "memory_id",
        "record_revision",
    ] {
        assert!(leg_atom[field].is_null(), "expected {field} to be null");
    }

    let legacy_reexport = mgr
        .export_context_json(&json!({
            "project_id": "proj-xfer",
            "branch": "imported/legacy-noprovenance",
            "scope": "branch",
            "format": "bundle"
        }))
        .await
        .unwrap();
    assert_eq!(legacy_reexport["counts"]["atoms"], 1);
    assert_eq!(
        legacy_reexport["bundle"]["atoms"][0]["atom_id"],
        "atom-legacy-noprovenance"
    );

    mgr.close().await.unwrap();
}
