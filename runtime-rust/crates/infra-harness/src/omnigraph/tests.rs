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
    turn_snapshots: BTreeMap<String, Value>,
    published_by: BTreeMap<String, Value>,
    consumed_by: BTreeMap<String, Value>,
    touches_file: BTreeMap<String, Value>,
    relates_to: BTreeMap<String, Value>,
    derived_from: BTreeMap<String, Value>,
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
    for (id, v) in &source.published_by {
        merged.published_by.insert(id.clone(), v.clone());
    }
    for (id, v) in &source.consumed_by {
        merged.consumed_by.insert(id.clone(), v.clone());
    }
    for (id, v) in &source.touches_file {
        merged.touches_file.insert(id.clone(), v.clone());
    }
    for (id, v) in &source.relates_to {
        merged.relates_to.insert(id.clone(), v.clone());
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

    if query_text.contains("insert Session") || query_name == "upsert_session" {
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
        snap.derived_from
            .insert(format!("{src}->{dst}"), params.clone());
        affected_edges += 1;
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

async fn spawn_mock_omnigraph() -> (String, tokio::task::JoinHandle<()>) {
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
    assert!(schema::HARNESS_CONTEXT_PG.contains("node Session"));
    assert!(schema::HARNESS_CONTEXT_PG.contains("node ContextAtom"));
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
    assert_eq!(diff["added_edges"].as_array().unwrap().len(), 1);
    assert_eq!(diff["added_edges"][0]["from"], "a2");
    assert_eq!(diff["added_edges"][0]["to"], "a1");
    assert_eq!(diff["added_edges"][0]["relation"], "refines");
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
}
