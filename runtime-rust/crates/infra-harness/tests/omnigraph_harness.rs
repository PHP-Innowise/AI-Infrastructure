//! Integration tests wiring Omnigraph cross-session context sharing into `infra-harness`:
//! HTTP routes (`/api/omnigraph/*`), `NativeServices`, SQLite `omnigraph` persistence/migration,
//! `POST /api/sessions`, `prompt::build` + `PromptLedger`, post-turn `Results` publishing,
//! `POST /api/omnigraph/share`, and end-to-end cross-session context flow.

use axum::{
    Router,
    extract::{Path as AxumPath, Query as AxumQuery, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use infra_harness::{
    accelerators::Accelerators,
    bindings::{ResultsBridge, SessionLinks},
    commands::CommandCatalog,
    context_usage::PromptLedger,
    http::{self, App},
    knowledge::KnowledgeDomain,
    omnigraph::{OmnigraphDomain, OmnigraphManager, schema},
    prompt,
    provider::{Provider, ProviderDomain},
    results::Results,
    services::{CloseFuture, Domain, NativeServices, Payload, Reply, Services},
    sessions::{BlackboardFactory, SessionLimits, Sessions, SpecialWorkflow},
    store::SessionStore,
};
use reqwest::Method;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex, RwLock, atomic::AtomicBool},
    time::{Duration, Instant},
};
use tokio::net::TcpListener;

// ---------------------------------------------------------------------------
// Mock Omnigraph 0.14 Server (implements contract verified against v0.15.0)
// ---------------------------------------------------------------------------

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
    token: String,
    actor_id: String,
    branches: BTreeMap<String, MockBranch>,
    commits: BTreeMap<String, MockCommit>,
    commit_order: Vec<String>,
    next_seq: u64,
}

impl MockGraphState {
    fn new(graph_id: &str, token: &str) -> Self {
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
            token: token.into(),
            actor_id: "harness".into(),
            branches,
            commits,
            commit_order: vec!["c-0001".into()],
            next_seq: 2,
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

fn mock_err(status: StatusCode, code: &str, msg: &str, extra: Option<Value>) -> Response {
    let mut body = json!({ "error": msg, "code": code });
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

fn check_contract(
    headers: &HeaderMap,
    st: &MockGraphState,
) -> std::result::Result<String, Box<Response>> {
    if headers
        .get("Omnigraph-Http-Api")
        .and_then(|v| v.to_str().ok())
        != Some("0.14")
    {
        return Err(Box::new(mock_err(
            StatusCode::BAD_REQUEST,
            "api_contract_mismatch",
            "Expected Omnigraph-Http-Api: 0.14",
            None,
        )));
    }
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    if bearer != Some(st.token.as_str()) {
        return Err(Box::new(mock_err(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Invalid bearer token.",
            None,
        )));
    }
    if let Some(actor) = headers
        .get("Omnigraph-If-Actor")
        .and_then(|v| v.to_str().ok())
        && actor != st.actor_id
    {
        return Err(Box::new(mock_err(
            StatusCode::PRECONDITION_FAILED,
            "actor_mismatch",
            "Actor mismatch.",
            None,
        )));
    }
    Ok(st.actor_id.clone())
}

async fn mock_healthz() -> Response {
    axum::Json(json!({ "status": "ok", "version": "0.15.0" })).into_response()
}

async fn mock_readyz() -> Response {
    axum::Json(json!({ "ready": true, "status": "serving" })).into_response()
}

async fn mock_graphs(State(srv): State<MockServer>, headers: HeaderMap) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(r) = check_contract(&headers, &st) {
        return *r;
    }
    axum::Json(json!({ "graphs": [{ "graph_id": st.graph_id, "read_available": true, "write_available": true }] }))
        .into_response()
}

async fn mock_schema(
    State(srv): State<MockServer>,
    AxumPath(gid): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(r) = check_contract(&headers, &st) {
        return *r;
    }
    if gid != st.graph_id {
        return mock_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    axum::Json(json!({ "schema_source": schema::HARNESS_CONTEXT_PG })).into_response()
}

async fn mock_branches_list(
    State(srv): State<MockServer>,
    AxumPath(gid): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(r) = check_contract(&headers, &st) {
        return *r;
    }
    if gid != st.graph_id {
        return mock_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    let branches: Vec<String> = st.branches.keys().cloned().collect();
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
                    "parent_commit_id": head.parent,
                    "merged_parent_commit_id": head.merged_parent,
                    "actor_id": head.actor_id,
                    "created_at": head.created_at,
                    "published": true
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
    AxumPath(gid): AxumPath<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let mut st = srv.state.lock().unwrap();
    let actor = match check_contract(&headers, &st) {
        Ok(a) => a,
        Err(r) => return *r,
    };
    if gid != st.graph_id {
        return mock_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    let name = body["name"].as_str().unwrap_or("").trim().to_string();
    if st.branches.contains_key(&name) {
        return mock_err(StatusCode::CONFLICT, "conflict", "Branch exists.", None);
    }
    let from = body["from"].as_str().unwrap_or("main").to_string();
    let Some(src) = st.branches.get(&from).cloned() else {
        return mock_err(
            StatusCode::NOT_FOUND,
            "not_found",
            "Missing parent branch.",
            None,
        );
    };
    let ts = 1_700_000_000_000_000 + (st.next_seq as i64) * 1000;
    st.branches.insert(
        name.clone(),
        MockBranch {
            name: name.clone(),
            head_commit: src.head_commit.clone(),
            from: Some(from.clone()),
            from_commit_id: Some(src.head_commit.clone()),
            created_at: ts,
            updated_at: ts,
        },
    );
    axum::Json(json!({
        "uri": format!("/graphs/{gid}/branches/{name}"),
        "from": from,
        "name": name,
        "actor_id": actor,
        "from_commit_id": src.head_commit
    }))
    .into_response()
}

async fn mock_branches_delete(
    State(srv): State<MockServer>,
    AxumPath((gid, name)): AxumPath<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let mut st = srv.state.lock().unwrap();
    let actor = match check_contract(&headers, &st) {
        Ok(a) => a,
        Err(r) => return *r,
    };
    if gid != st.graph_id {
        return mock_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    st.branches.remove(&name);
    axum::Json(json!({ "name": name, "actor_id": actor })).into_response()
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
    changes
}

async fn mock_branches_diff(
    State(srv): State<MockServer>,
    AxumPath(gid): AxumPath<String>,
    AxumQuery(q): AxumQuery<BTreeMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(r) = check_contract(&headers, &st) {
        return *r;
    }
    if gid != st.graph_id {
        return mock_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    if q.contains_key("from") || q.contains_key("to") {
        return mock_err(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "Use source and target parameters.",
            None,
        );
    }
    let source = q.get("source").map(String::as_str).unwrap_or("");
    let target = q.get("target").map(String::as_str).unwrap_or("main");
    let (Some(src), Some(tgt)) = (st.branches.get(source), st.branches.get(target)) else {
        return mock_err(
            StatusCode::NOT_FOUND,
            "not_found",
            "Branch not found.",
            None,
        );
    };
    let base_id = st.lca(&src.head_commit, &tgt.head_commit);
    let changes = diff_snapshots(
        &st.commits[&base_id].snapshot,
        &st.commits[&src.head_commit].snapshot,
    );
    axum::Json(json!({
        "from_commit_id": base_id,
        "to_commit_id": src.head_commit,
        "base_commit_id": base_id,
        "changes": changes
    }))
    .into_response()
}

async fn mock_branches_merge(
    State(srv): State<MockServer>,
    AxumPath(gid): AxumPath<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let mut st = srv.state.lock().unwrap();
    let actor = match check_contract(&headers, &st) {
        Ok(a) => a,
        Err(r) => return *r,
    };
    if gid != st.graph_id {
        return mock_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    let source = body["source"].as_str().unwrap_or("").to_string();
    let target = body["target"].as_str().unwrap_or("main").to_string();
    let dry_run = body["dry_run"].as_bool().unwrap_or(false);
    let delete_branch = body["delete_branch"].as_bool().unwrap_or(false);

    let (Some(src), Some(tgt)) = (
        st.branches.get(&source).cloned(),
        st.branches.get(&target).cloned(),
    ) else {
        return mock_err(
            StatusCode::NOT_FOUND,
            "not_found",
            "Branch not found.",
            None,
        );
    };

    let base_id = st.lca(&src.head_commit, &tgt.head_commit);
    if src.head_commit == tgt.head_commit || base_id == src.head_commit {
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
    let src_snap = st.commits[&src.head_commit].snapshot.clone();
    let tgt_snap = st.commits[&tgt.head_commit].snapshot.clone();
    let mut merged = tgt_snap.clone();
    for (id, v) in &src_snap.atoms {
        if base_snap.atoms.get(id) != Some(v) {
            merged.atoms.insert(id.clone(), v.clone());
        }
    }
    for (id, v) in &src_snap.sessions {
        merged
            .sessions
            .entry(id.clone())
            .or_insert_with(|| v.clone());
    }
    for (id, v) in &src_snap.files {
        merged.files.entry(id.clone()).or_insert_with(|| v.clone());
    }
    for (id, v) in &src_snap.published_by {
        merged.published_by.insert(id.clone(), v.clone());
    }
    for (id, v) in &src_snap.consumed_by {
        merged.consumed_by.insert(id.clone(), v.clone());
    }
    for (id, v) in &src_snap.touches_file {
        merged.touches_file.insert(id.clone(), v.clone());
    }
    for (id, v) in &src_snap.relates_to {
        merged.relates_to.insert(id.clone(), v.clone());
    }

    let outcome = if base_id == tgt.head_commit {
        "fast_forward"
    } else {
        "merged"
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
    let changes = diff_snapshots(&tgt_snap, &merged);
    let commit = MockCommit {
        id: cid.clone(),
        branch: target.clone(),
        parent: Some(tgt.head_commit),
        merged_parent: Some(src.head_commit),
        actor_id: Some(actor.clone()),
        created_at: ts,
        snapshot: merged,
        changes,
    };
    let commit_json = json!({
        "graph_commit_id": commit.id,
        "graph_branch": commit.branch,
        "parent_commit_id": commit.parent,
        "merged_parent_commit_id": commit.merged_parent,
        "actor_id": commit.actor_id,
        "created_at": commit.created_at,
        "published": true
    });
    st.commits.insert(cid.clone(), commit);
    st.commit_order.push(cid.clone());
    if let Some(tb) = st.branches.get_mut(&target) {
        tb.head_commit = cid;
        tb.updated_at = ts;
    }
    let deleted = delete_branch && source != "main" && st.branches.remove(&source).is_some();
    axum::Json(json!({
        "source": source,
        "target": target,
        "outcome": outcome,
        "commit": commit_json,
        "actor_id": actor,
        "branch_deleted": deleted,
        "dry_run": false
    }))
    .into_response()
}

async fn mock_mutate(
    State(srv): State<MockServer>,
    AxumPath(gid): AxumPath<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let mut st = srv.state.lock().unwrap();
    let actor = match check_contract(&headers, &st) {
        Ok(a) => a,
        Err(r) => return *r,
    };
    if gid != st.graph_id {
        return mock_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    let bname = body["branch"].as_str().unwrap_or("main").to_string();
    let Some(branch) = st.branches.get(&bname).cloned() else {
        return mock_err(
            StatusCode::NOT_FOUND,
            "not_found",
            "Branch not found.",
            None,
        );
    };
    let mut snap = st.commits[&branch.head_commit].snapshot.clone();
    let params = &body["params"];
    let qtext = body["query"].as_str().unwrap_or("");
    let qname = body["name"].as_str().unwrap_or("mutate").to_string();
    let mut changes = Vec::new();
    let mut nodes = 0usize;
    let mut edges = 0usize;

    if qtext.contains("insert Session") || qname == "upsert_session" {
        let sid = params["session_id"].as_str().unwrap_or("s1").to_string();
        snap.sessions.insert(sid.clone(), params.clone());
        nodes += 1;
        changes.push(json!({
            "kind": "node",
            "type": { "id": "t-sess", "name": "Session" },
            "id": sid,
            "op": "insert",
            "after": { "properties": params }
        }));
    } else if qtext.contains("insert ContextAtom") || qname == "upsert_atom" {
        let aid = params["atom_id"].as_str().unwrap_or("a1").to_string();
        let prev = snap.atoms.insert(aid.clone(), params.clone());
        nodes += 1;
        changes.push(json!({
            "kind": "node",
            "type": { "id": "t-atom", "name": "ContextAtom" },
            "id": aid,
            "op": if prev.is_some() { "update" } else { "insert" },
            "before": prev.map(|p| json!({ "properties": p })),
            "after": { "properties": params }
        }));
    } else if qtext.contains("insert FileRef") || qname == "upsert_file" {
        let path = params["path"].as_str().unwrap_or("f.rs").to_string();
        snap.files.insert(path, params.clone());
        nodes += 1;
    } else if qtext.contains("insert TurnSnapshot") {
        let sid = params["snapshot_id"].as_str().unwrap_or("snap").to_string();
        snap.turn_snapshots.insert(sid, params.clone());
        nodes += 1;
    } else if qtext.contains("insert PublishedBy") {
        let src = params["atom_id"].as_str().unwrap_or("");
        let dst = params["session_id"].as_str().unwrap_or("");
        let mut props = params.clone();
        props["source_id"] = json!(src);
        props["target_id"] = json!(dst);
        snap.published_by.insert(format!("{src}->{dst}"), props);
        edges += 1;
    } else if qtext.contains("insert ConsumedBy") {
        let src = params["atom_id"].as_str().unwrap_or("");
        let dst = params["session_id"].as_str().unwrap_or("");
        let mut props = params.clone();
        props["source_id"] = json!(src);
        props["target_id"] = json!(dst);
        snap.consumed_by.insert(format!("{src}->{dst}"), props);
        edges += 1;
    } else if qtext.contains("insert TouchesFile") {
        let src = params["atom_id"].as_str().unwrap_or("");
        let dst = params["path"].as_str().unwrap_or("");
        let mut props = params.clone();
        props["source_id"] = json!(src);
        props["target_id"] = json!(dst);
        snap.touches_file.insert(format!("{src}->{dst}"), props);
        edges += 1;
    } else if qtext.contains("insert RelatesTo") {
        let src = params["source_id"].as_str().unwrap_or("");
        let dst = params["target_id"].as_str().unwrap_or("");
        let rel = params["relation"].as_str().unwrap_or("supports");
        snap.relates_to
            .insert(format!("{src}:{rel}:{dst}"), params.clone());
        edges += 1;
    } else if qtext.contains("insert DerivedFrom") {
        let src = params["atom_id"].as_str().unwrap_or("");
        let dst = params["snapshot_id"].as_str().unwrap_or("");
        snap.derived_from
            .insert(format!("{src}->{dst}"), params.clone());
        edges += 1;
    }

    let cid = st.alloc_commit_id();
    let ts = 1_700_000_000_000_000 + (st.next_seq as i64) * 1000;
    let commit = MockCommit {
        id: cid.clone(),
        branch: bname.clone(),
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
        "parent_commit_id": commit.parent,
        "actor_id": commit.actor_id,
        "created_at": commit.created_at,
        "published": true
    });
    st.commits.insert(cid.clone(), commit);
    st.commit_order.push(cid.clone());
    if let Some(b) = st.branches.get_mut(&bname) {
        b.head_commit = cid;
        b.updated_at = ts;
    }

    axum::Json(json!({
        "branch": bname,
        "query_name": qname,
        "affected_nodes": nodes,
        "affected_edges": edges,
        "actor_id": actor,
        "commit": commit_json
    }))
    .into_response()
}

fn eval_query(snap: &MockSnapshot, qname: &str, params: &Value) -> Vec<Value> {
    let pid = params["project_id"].as_str().unwrap_or("");
    let match_proj = |v: &Value| pid.is_empty() || v["project_id"].as_str().unwrap_or("") == pid;
    match qname {
        "context_search" => {
            let q = params["q"].as_str().unwrap_or("").to_ascii_lowercase();
            snap.atoms
                .values()
                .filter(|a| match_proj(a))
                .filter(|a| {
                    let hay = format!(
                        "{} {} {}",
                        a["title"].as_str().unwrap_or(""),
                        a["summary"].as_str().unwrap_or(""),
                        a["body"].as_str().unwrap_or("")
                    )
                    .to_ascii_lowercase();
                    q.is_empty() || q.split_whitespace().any(|term| hay.contains(term))
                })
                .map(|a| {
                    let mut row = a.clone();
                    row["score"] = json!(0.95);
                    row
                })
                .collect()
        }
        "recent_atoms" => snap
            .atoms
            .values()
            .filter(|a| match_proj(a))
            .cloned()
            .collect(),
        "list_sessions" => snap
            .sessions
            .values()
            .filter(|s| match_proj(s))
            .cloned()
            .collect(),
        "list_files" => snap
            .files
            .values()
            .filter(|f| match_proj(f))
            .cloned()
            .collect(),
        "list_published_edges" => snap.published_by.values().cloned().collect(),
        "list_consumed_edges" => snap.consumed_by.values().cloned().collect(),
        "list_file_edges" => snap.touches_file.values().cloned().collect(),
        "list_atom_relations" => snap.relates_to.values().cloned().collect(),
        _ => Vec::new(),
    }
}

async fn mock_query(
    State(srv): State<MockServer>,
    AxumPath(gid): AxumPath<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(r) = check_contract(&headers, &st) {
        return *r;
    }
    if gid != st.graph_id {
        return mock_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    let cid = if let Some(s) = body["snapshot"].as_str() {
        s.to_string()
    } else {
        let bname = body["branch"].as_str().unwrap_or("main");
        let Some(b) = st.branches.get(bname) else {
            return mock_err(
                StatusCode::NOT_FOUND,
                "not_found",
                "Branch not found.",
                None,
            );
        };
        b.head_commit.clone()
    };
    let Some(commit) = st.commits.get(&cid) else {
        return mock_err(
            StatusCode::NOT_FOUND,
            "not_found",
            "Commit not found.",
            None,
        );
    };
    let qname = body["name"].as_str().unwrap_or("inline");
    let rows = eval_query(&commit.snapshot, qname, &body["params"]);
    axum::Json(json!({
        "query_name": qname,
        "row_count": rows.len(),
        "rows": rows,
        "graph_commit_id": cid
    }))
    .into_response()
}

async fn mock_stored_query(
    State(srv): State<MockServer>,
    AxumPath((gid, qname)): AxumPath<(String, String)>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(r) = check_contract(&headers, &st) {
        return *r;
    }
    if gid != st.graph_id {
        return mock_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    let cid = if let Some(s) = body["snapshot"].as_str() {
        s.to_string()
    } else {
        let bname = body["branch"].as_str().unwrap_or("main");
        let Some(b) = st.branches.get(bname) else {
            return mock_err(
                StatusCode::NOT_FOUND,
                "not_found",
                "Branch not found.",
                None,
            );
        };
        b.head_commit.clone()
    };
    let Some(commit) = st.commits.get(&cid) else {
        return mock_err(
            StatusCode::NOT_FOUND,
            "not_found",
            "Commit not found.",
            None,
        );
    };
    let rows = eval_query(&commit.snapshot, &qname, &body["params"]);
    axum::Json(json!({
        "query_name": qname,
        "row_count": rows.len(),
        "rows": rows,
        "graph_commit_id": cid
    }))
    .into_response()
}

async fn mock_commits(
    State(srv): State<MockServer>,
    AxumPath(gid): AxumPath<String>,
    AxumQuery(q): AxumQuery<BTreeMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(r) = check_contract(&headers, &st) {
        return *r;
    }
    if gid != st.graph_id {
        return mock_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    let bname = q.get("branch").map(String::as_str).unwrap_or("main");
    let Some(b) = st.branches.get(bname) else {
        return mock_err(
            StatusCode::NOT_FOUND,
            "not_found",
            "Branch not found.",
            None,
        );
    };
    let mut commits = Vec::new();
    let mut cur = Some(b.head_commit.clone());
    while let Some(cid) = cur {
        if let Some(c) = st.commits.get(&cid) {
            commits.push(json!({
                "graph_commit_id": c.id,
                "graph_branch": c.branch,
                "parent_commit_id": c.parent,
                "merged_parent_commit_id": c.merged_parent,
                "actor_id": c.actor_id,
                "created_at": c.created_at,
                "published": true
            }));
            cur = c.parent.clone();
        } else {
            break;
        }
    }
    axum::Json(json!({ "commits": commits, "head_commit_id": b.head_commit })).into_response()
}

async fn mock_commit_changes(
    State(srv): State<MockServer>,
    AxumPath((gid, cid)): AxumPath<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(r) = check_contract(&headers, &st) {
        return *r;
    }
    if gid != st.graph_id {
        return mock_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    let Some(c) = st.commits.get(&cid) else {
        return mock_err(
            StatusCode::NOT_FOUND,
            "not_found",
            "Commit not found.",
            None,
        );
    };
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

async fn mock_changes(
    State(srv): State<MockServer>,
    AxumPath(gid): AxumPath<String>,
    AxumQuery(q): AxumQuery<BTreeMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let st = srv.state.lock().unwrap();
    if let Err(r) = check_contract(&headers, &st) {
        return *r;
    }
    if gid != st.graph_id {
        return mock_err(StatusCode::NOT_FOUND, "not_found", "Unknown graph.", None);
    }
    let after = q
        .get("cursor")
        .and_then(|c| c.strip_prefix("cur-"))
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(0);
    let mut blocks = Vec::new();
    for cid in st.commit_order.iter().skip(after) {
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
    axum::Json(json!({
        "blocks": blocks,
        "cursor": format!("cur-{}", st.commit_order.len()),
        "caught_up": true
    }))
    .into_response()
}

async fn spawn_mock_omnigraph(token: &str) -> (String, tokio::task::JoinHandle<()>) {
    let srv = MockServer {
        state: Arc::new(Mutex::new(MockGraphState::new(
            schema::HARNESS_GRAPH_ID,
            token,
        ))),
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
        .route("/graphs/{graph_id}/query", post(mock_query))
        .route("/graphs/{graph_id}/queries/{name}", post(mock_stored_query))
        .route("/graphs/{graph_id}/commits", get(mock_commits))
        .route(
            "/graphs/{graph_id}/commits/{commit_id}/changes",
            get(mock_commit_changes),
        )
        .route("/graphs/{graph_id}/changes", get(mock_changes))
        .with_state(srv);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), handle)
}

// ---------------------------------------------------------------------------
// Harness Test Fixture (`App::router()` + `NativeServices` + `Sessions` + `Results`)
// ---------------------------------------------------------------------------

struct StubDomain;
impl Domain for StubDomain {
    fn execute<'a>(&'a self, _operation: &'a str, _request: Value) -> Reply<'a> {
        Box::pin(async { Ok(Payload::Json(json!({}))) })
    }
}

struct UnusedSpecial;
impl SpecialWorkflow for UnusedSpecial {
    fn run<'a>(
        &'a self,
        _session: Value,
        _prompt: String,
        _generation: String,
        _cancel: Arc<AtomicBool>,
        _emit: infra_harness::sessions::Event,
    ) -> infra_harness::sessions::RunReply<'a> {
        Box::pin(async { Ok(json!({})) })
    }
}
impl BlackboardFactory for UnusedSpecial {
    fn for_session(
        &self,
        _session: &Value,
    ) -> infra_harness::Result<Arc<dyn infra_harness::fleet::Blackboard>> {
        unreachable!()
    }
}

struct HarnessFixture {
    _dir: tempfile::TempDir,
    project_id: String,
    project_path: PathBuf,
    store: SessionStore,
    _sessions: Arc<Sessions>,
    results: Arc<Results>,
    omnigraph: Arc<OmnigraphManager>,
    services: Arc<NativeServices>,
    port: u16,
    token: String,
    shutdown: tokio::sync::watch::Sender<bool>,
    server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
}

impl HarnessFixture {
    async fn new(claude_exe: Option<PathBuf>) -> Self {
        let _ = infra_host::process::use_guardian(PathBuf::from(env!(
            "CARGO_BIN_EXE_infra-harness-inbox-fixture"
        )));
        let dir = tempfile::tempdir().unwrap();
        let root = infra_core::fs::canonicalize(dir.path()).unwrap();
        let project_path = root.join("project");
        let source = root.join("source");
        let state = root.join("state");
        fs::create_dir_all(&project_path).unwrap();
        fs::create_dir_all(&source).unwrap();

        let store = SessionStore::open(&state).unwrap();
        let project = store.register(&project_path).unwrap();
        let project_id = project["id"].as_str().unwrap().to_owned();

        let providers = Arc::new(ProviderDomain {
            providers: Arc::new(RwLock::new(BTreeMap::from([(
                "claude".into(),
                Provider {
                    id: "claude".into(),
                    name: "Mock Claude".into(),
                    available: claude_exe.is_some(),
                    executable: claude_exe,
                    detail: "Test fixture".into(),
                },
            )]))),
        });
        infra_host::model_catalog::model_options("claude", None, None)
            .await
            .unwrap();

        let knowledge = Arc::new(KnowledgeDomain::new(store.clone(), source.clone()));
        let special = ["creator", "system_run", "system_discovery"]
            .into_iter()
            .map(|name| {
                (
                    name.into(),
                    Arc::new(UnusedSpecial) as Arc<dyn SpecialWorkflow>,
                )
            })
            .collect();
        let sessions = Sessions::new(
            store.clone(),
            source.clone(),
            providers.clone(),
            knowledge.clone(),
            Arc::new(UnusedSpecial),
            special,
            SessionLimits {
                seconds: 30,
                concurrency: 1,
            },
        )
        .unwrap();
        let links = Arc::new(SessionLinks::new());
        links.bind(Arc::downgrade(&sessions)).unwrap();
        let results = Arc::new(
            Results::new(
                store.clone(),
                Arc::new(ResultsBridge {
                    links,
                    knowledge: knowledge.clone(),
                    operation: sessions.operation_lock(),
                    cancel: sessions.shutdown_cancel(),
                }),
            )
            .unwrap(),
        );
        let omnigraph =
            Arc::new(OmnigraphManager::new(store.state.clone(), Some(store.clone())).unwrap());
        let omnigraph_domain = Arc::new(OmnigraphDomain::new(omnigraph.clone()));

        sessions.install_omnigraph(omnigraph.clone()).unwrap();
        results.install_omnigraph(omnigraph.clone()).unwrap();
        sessions.install_results(results.clone()).unwrap();
        sessions
            .install_commands(Arc::new(CommandCatalog::new(
                store.clone(),
                source.clone(),
                providers.providers.clone(),
            )))
            .unwrap();
        sessions
            .install_accelerators(Arc::new(
                Accelerators::new(
                    store.clone(),
                    source.clone(),
                    providers,
                    Arc::downgrade(&sessions),
                )
                .unwrap(),
            ))
            .unwrap();
        sessions.activate().unwrap();

        let stub: Arc<dyn Domain> = Arc::new(StubDomain);
        let mut domains: BTreeMap<String, Arc<dyn Domain>> = [
            "setup",
            "accelerators",
            "systems",
            "discovery",
            "creator",
            "skills",
            "delivery",
            "projects",
            "commands",
            "files",
            "memory",
            "updates",
            "providers",
            "knowledge",
        ]
        .into_iter()
        .map(|k| (k.to_owned(), stub.clone()))
        .collect();
        for g in ["sessions", "brain", "context"] {
            domains.insert(g.into(), sessions.clone());
        }
        domains.insert(
            "results".into(),
            Arc::new(infra_harness::results::ResultService {
                results: results.clone(),
            }),
        );
        domains.insert("omnigraph".into(), omnigraph_domain);

        let bs = store.clone();
        let closing = sessions.clone();
        let omni_close = omnigraph.clone();
        let services = Arc::new(
            NativeServices::new(
                store.clone(),
                domains,
                Arc::new(move || {
                    Ok(json!({
                        "projects": bs.projects()?,
                        "sessions": bs.summaries()?,
                        "omnigraph": {
                            "connected": false,
                            "state": "unconfigured",
                            "mode": "off",
                            "api_contract": "0.14"
                        }
                    }))
                }),
                BTreeMap::new(),
                Arc::new(move || {
                    let closing = closing.clone();
                    let omni = omni_close.clone();
                    Box::pin(async move {
                        let req = closing.close();
                        let reaped = closing.wait_closed().await;
                        let closed_omni = omni.close().await;
                        req.and(reaped).and(closed_omni)
                    }) as CloseFuture<'static>
                }),
            )
            .unwrap(),
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (shutdown, _) = tokio::sync::watch::channel(false);
        let token = "harness-omnigraph-test-token".to_owned();
        let app = App {
            services: services.clone(),
            token: token.clone(),
            instance: "test-instance".into(),
            port,
            source,
            shutdown: shutdown.clone(),
            assets: Arc::new(BTreeMap::new()),
        };
        let server = tokio::spawn(async move {
            let _ = http::serve_listener(listener, app).await;
        });

        Self {
            _dir: dir,
            project_id,
            project_path,
            store,
            _sessions: sessions,
            results,
            omnigraph,
            services,
            port,
            token,
            shutdown,
            server,
            client: reqwest::Client::new(),
        }
    }

    async fn http(
        &self,
        method: Method,
        path_and_query: &str,
        body: Option<Value>,
        with_token: bool,
    ) -> (u16, Value) {
        let url = format!("http://127.0.0.1:{}{}", self.port, path_and_query);
        let mut req = self.client.request(method, url);
        if with_token {
            req = req.header("x-harness-token", &self.token);
        }
        if let Some(json_body) = body {
            let raw = json_body.to_string();
            req = req
                .header("content-type", "application/json")
                .header("content-length", raw.len())
                .body(raw);
        }
        let resp = req.send().await.unwrap();
        let status = resp.status().as_u16();
        let bytes = resp.bytes().await.unwrap();
        let parsed: Value = serde_json::from_slice(&bytes).unwrap();
        (status, parsed)
    }

    async fn shutdown(self) {
        let _ = self.shutdown.send(true);
        let _ = self.services.close().await;
        let _ = self.server.await;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn http_omnigraph_routes_end_to_end_and_share_via_app_router() {
    let (mock_url, mock_task) = spawn_mock_omnigraph("tok-test").await;
    let fx = HarnessFixture::new(None).await;

    // 1. Initial bootstrap exposes omnigraph status summary
    let (status, boot) = fx.http(Method::GET, "/api/bootstrap", None, false).await;
    assert_eq!(status, 200);
    assert_eq!(boot["omnigraph"]["state"], "unconfigured");
    assert_eq!(boot["omnigraph"]["connected"], false);

    // 2. GET /api/omnigraph/status before setup
    let (status, st) = fx
        .http(
            Method::GET,
            &format!("/api/omnigraph/status?project_id={}", fx.project_id),
            None,
            false,
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(st["state"], "unconfigured");

    // 3. POST routes enforce X-Harness-Token
    let (status, err_body) = fx
        .http(
            Method::POST,
            "/api/omnigraph/setup",
            Some(json!({ "action": "connect", "endpoint": mock_url, "token": "tok-test" })),
            false,
        )
        .await;
    assert_eq!(status, 403, "{err_body}");

    // 4. POST /api/omnigraph/setup connects to mock Omnigraph 0.14 server
    let (status, setup) = fx
        .http(
            Method::POST,
            "/api/omnigraph/setup",
            Some(json!({
                "action": "connect",
                "endpoint": mock_url,
                "token": "tok-test",
                "graph_id": schema::HARNESS_GRAPH_ID
            })),
            true,
        )
        .await;
    assert_eq!(status, 200, "{setup}");
    assert_eq!(setup["connected"], true);
    assert_eq!(setup["state"], "connected");

    // Refresh bootstrap now reports live connected Omnigraph
    let (status, boot2) = fx.http(Method::GET, "/api/bootstrap", None, false).await;
    assert_eq!(status, 200);
    assert_eq!(boot2["omnigraph"]["connected"], true);
    assert_eq!(boot2["omnigraph"]["state"], "connected");

    // 5. POST /api/omnigraph/atoms works even for large atom bodies (>64 KiB)
    let large_body = "JWT signature verification keeps tenant scopes isolated. ".repeat(1400);
    assert!(large_body.len() > 70_000);
    let (status, pub_main) = fx
        .http(
            Method::POST,
            "/api/omnigraph/atoms",
            Some(json!({
                "project_id": fx.project_id,
                "branch": "main",
                "atom_id": "atom-jwt-1",
                "kind": "contract",
                "title": "JWT tenant isolation contract",
                "summary": "Always verify tenant_id claim before session routing.",
                "body": large_body,
                "tags": ["auth", "jwt"],
                "files": ["src/auth/jwt.rs"],
                "confidence": 0.95,
                "pin_priority": 25
            })),
            true,
        )
        .await;
    assert_eq!(status, 200, "{pub_main}");
    assert_eq!(pub_main["ok"], true);
    assert_eq!(pub_main["id"], "atom-jwt-1");
    assert_eq!(pub_main["atom_id"], "atom-jwt-1");
    assert_eq!(pub_main["atom"]["id"], "atom-jwt-1");
    assert_eq!(pub_main["atom"]["atom_id"], "atom-jwt-1");
    assert_eq!(pub_main["atom"]["branch"], "main");
    assert_eq!(pub_main["atom"]["source_branch"], "main");
    let commit_id = pub_main["commit"]["graph_commit_id"]
        .as_str()
        .unwrap()
        .to_owned();

    // 6. POST /api/omnigraph/branches (create) + publish on branch + GET diff + merge
    let (status, created_b) = fx
        .http(
            Method::POST,
            "/api/omnigraph/branches",
            Some(json!({
                "action": "create",
                "project_id": fx.project_id,
                "name": "session/feat-1",
                "from": "main"
            })),
            true,
        )
        .await;
    assert_eq!(status, 200, "{created_b}");
    assert_eq!(created_b["name"], "session/feat-1");

    let (status, pub_branch) = fx
        .http(
            Method::POST,
            "/api/omnigraph/atoms",
            Some(json!({
                "project_id": fx.project_id,
                "branch": "session/feat-1",
                "atom_id": "atom-cache-2",
                "kind": "finding",
                "title": "Redis key prefix collision",
                "summary": "Include app env prefix in session cache keys.",
                "files": ["src/cache/redis.rs"]
            })),
            true,
        )
        .await;
    assert_eq!(status, 200, "{pub_branch}");

    // 7. GET /api/omnigraph/branches/diff
    let (status, diff) = fx
        .http(
            Method::GET,
            &format!(
                "/api/omnigraph/branches/diff?project_id={}&source=session/feat-1&target=main",
                fx.project_id
            ),
            None,
            false,
        )
        .await;
    assert_eq!(status, 200, "{diff}");
    assert_eq!(diff["outcome"], "fast_forward");
    assert_eq!(diff["added_atoms"].as_array().map(Vec::len), Some(1));

    // Merge branch into main
    let (status, merged) = fx
        .http(
            Method::POST,
            "/api/omnigraph/branches",
            Some(json!({
                "action": "merge",
                "project_id": fx.project_id,
                "source": "session/feat-1",
                "target": "main",
                "dry_run": false,
                "delete_branch": false
            })),
            true,
        )
        .await;
    assert_eq!(status, 200, "{merged}");
    assert_eq!(merged["ok"], true);
    assert_eq!(merged["outcome"], "fast_forward");

    // 8. GET /api/omnigraph/graph & /api/omnigraph/search
    let (status, graph) = fx
        .http(
            Method::GET,
            &format!(
                "/api/omnigraph/graph?project_id={}&branch=main",
                fx.project_id
            ),
            None,
            false,
        )
        .await;
    assert_eq!(status, 200, "{graph}");
    assert_eq!(graph["connected"], true);
    assert!(graph["atoms"].as_array().unwrap().len() >= 2);

    let (status, search) = fx
        .http(
            Method::GET,
            &format!(
                "/api/omnigraph/search?project_id={}&q=Redis&branch=main&limit=5",
                fx.project_id
            ),
            None,
            false,
        )
        .await;
    assert_eq!(status, 200, "{search}");
    assert_eq!(search["hits"][0]["atom"]["atom_id"], "atom-cache-2");

    // 9. GET /api/omnigraph/changes & GET /api/omnigraph/commits/{commit_id}
    let (status, changes) = fx
        .http(
            Method::GET,
            &format!(
                "/api/omnigraph/changes?project_id={}&branch=main",
                fx.project_id
            ),
            None,
            false,
        )
        .await;
    assert_eq!(status, 200, "{changes}");
    let events = changes["events"].as_array().unwrap();
    assert!(!events.is_empty());

    let (status, last_detail) = fx
        .http(
            Method::GET,
            &format!(
                "/api/omnigraph/commits/{commit_id}?project_id={}",
                fx.project_id
            ),
            None,
            false,
        )
        .await;
    assert_eq!(status, 200, "{last_detail}");
    assert_eq!(last_detail["commit"]["graph_commit_id"], commit_id);

    let atom_commit_id = events
        .iter()
        .find(|ev| ev["type_name"] == "ContextAtom" && ev["entity_id"] == "atom-jwt-1")
        .and_then(|ev| ev["graph_commit_id"].as_str())
        .expect("atom-jwt-1 commit event")
        .to_owned();
    let (status, detail) = fx
        .http(
            Method::GET,
            &format!(
                "/api/omnigraph/commits/{atom_commit_id}?project_id={}",
                fx.project_id
            ),
            None,
            false,
        )
        .await;
    assert_eq!(status, 200, "{detail}");
    assert_eq!(detail["commit"]["graph_commit_id"], atom_commit_id);
    assert_eq!(detail["atoms"][0]["atom_id"], "atom-jwt-1");

    // 10. POST /api/omnigraph/share updates target session's omnigraph.injected_atoms, capsule_text, and records event
    let target = fx
        .store
        .create(
            &json!({
                "project_id": fx.project_id,
                "provider": "claude",
                "mode": "plan",
                "workflow": "native",
                "project_context": false
            }),
            "Inspect cache layer",
            &[],
        )
        .unwrap();
    let target_sid = target["id"].as_str().unwrap().to_owned();
    fx.store
        .patch(&target_sid, &json!({ "status": "completed" }))
        .unwrap();

    let (status, shared) = fx
        .http(
            Method::POST,
            "/api/omnigraph/share",
            Some(json!({
                "project_id": fx.project_id,
                "target_session_id": target_sid,
                "atom_ids": ["atom-jwt-1", "atom-cache-2"],
                "source_branch": "main",
                "mode": "reference"
            })),
            true,
        )
        .await;
    assert_eq!(status, 200, "{shared}");
    assert_eq!(shared["ok"], true);
    assert_eq!(shared["shared_count"], 2);
    assert_eq!(
        shared["session"]["omnigraph"]["injected_atoms"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // Verify `inject` & `inject_and_link` share modes return updated `res["session"]` cleanly
    for extra_mode in ["inject", "inject_and_link"] {
        let (st_m, res_m) = fx
            .http(
                Method::POST,
                "/api/omnigraph/share",
                Some(json!({
                    "project_id": fx.project_id,
                    "target_session_id": target_sid,
                    "atom_ids": ["atom-jwt-1"],
                    "source_branch": "main",
                    "mode": extra_mode
                })),
                true,
            )
            .await;
        assert_eq!(st_m, 200, "{res_m}");
        assert_eq!(res_m["mode"], extra_mode);
        assert!(
            res_m["session"]["omnigraph"]["capsule_text"]
                .as_str()
                .is_some()
        );
    }

    let (status, session_after) = fx
        .http(
            Method::GET,
            &format!("/api/sessions/{target_sid}?after=0"),
            None,
            false,
        )
        .await;
    assert_eq!(status, 200);
    let injected = session_after["session"]["omnigraph"]["injected_atoms"]
        .as_array()
        .unwrap();
    assert_eq!(injected.len(), 2);
    let capsule = session_after["session"]["omnigraph"]["capsule_text"]
        .as_str()
        .unwrap();
    assert!(capsule.contains("JWT tenant isolation contract"));
    assert!(capsule.contains("Redis key prefix collision"));
    assert!(
        session_after["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "omnigraph" && e["phase"] == "share" && e["ok"] == true),
        "expected share event in {:?}",
        session_after["events"]
    );

    // Also verify sharing into a running session updates omnigraph state and records an active event
    let active_target = fx
        .store
        .create(
            &json!({
                "project_id": fx.project_id,
                "provider": "claude",
                "mode": "plan",
                "workflow": "native",
                "project_context": false
            }),
            "Active cache debug",
            &[],
        )
        .unwrap();
    let active_sid = active_target["id"].as_str().unwrap().to_owned();
    fx.store
        .patch(&active_sid, &json!({ "status": "running" }))
        .unwrap();
    let (status, shared_active) = fx
        .http(
            Method::POST,
            "/api/omnigraph/share",
            Some(json!({
                "project_id": fx.project_id,
                "target_session_id": active_sid,
                "atom_ids": ["atom-jwt-1"],
                "source_branch": "main",
                "mode": "copy"
            })),
            true,
        )
        .await;
    assert_eq!(status, 200, "{shared_active}");
    let (status, active_after) = fx
        .http(
            Method::GET,
            &format!("/api/sessions/{active_sid}?after=0"),
            None,
            false,
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        active_after["session"]["omnigraph"]["injected_atoms"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        active_after["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "omnigraph"
                && e["phase"] == "share"
                && e["session_status"] == "running"),
        "expected running-session share event in {:?}",
        active_after["events"]
    );

    // 11. Merging a session's Omnigraph branch persists merge_status and returns updated `session`
    let active_branch = active_after["session"]["omnigraph"]["branch"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, pub_active) = fx
        .http(
            Method::POST,
            "/api/omnigraph/atoms",
            Some(json!({
                "project_id": fx.project_id,
                "branch": active_branch,
                "atom_id": "atom-active-merge-1",
                "kind": "finding",
                "title": "Persist merge_status onto session",
                "summary": "Manual branch merge updates session omnigraph metadata."
            })),
            true,
        )
        .await;
    assert_eq!(status, 200, "{pub_active}");
    let (status, merge_res) = fx
        .http(
            Method::POST,
            "/api/omnigraph/branches",
            Some(json!({
                "action": "merge",
                "project_id": fx.project_id,
                "source": active_branch,
                "target": "main",
                "dry_run": false,
                "delete_branch": false
            })),
            true,
        )
        .await;
    assert_eq!(status, 200, "{merge_res}");
    assert_eq!(merge_res["ok"], true);
    assert_eq!(
        merge_res["session"]["omnigraph"]["merge_status"],
        merge_res["outcome"]
    );
    assert_eq!(
        fx.store.get(&active_sid).unwrap()["omnigraph"]["merge_status"],
        merge_res["outcome"]
    );

    fx.shutdown().await;
    mock_task.abort();
}

#[test]
fn sqlite_migration_adds_omnigraph_column_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    fs::create_dir_all(&state).unwrap();
    let db_path = state.join("sessions.sqlite3");
    {
        let db = Connection::open(&db_path).unwrap();
        db.execute_batch(
            "CREATE TABLE sessions(
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                project_id TEXT NOT NULL,
                project_path TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT,
                mode TEXT NOT NULL,
                workflow TEXT NOT NULL,
                project_context INTEGER NOT NULL,
                status TEXT NOT NULL,
                native_session_id TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            INSERT INTO sessions(id,title,project_id,project_path,provider,mode,workflow,project_context,status,created_at,updated_at)
            VALUES('legacy-1','Legacy session','proj','/tmp','claude','plan','native',0,'completed','now','now');",
        )
        .unwrap();
    }

    let store = SessionStore::open(&state).unwrap();
    let legacy = store.get("legacy-1").unwrap();
    assert!(legacy["omnigraph"].is_null());

    let updated = store
        .patch(
            "legacy-1",
            &json!({
                "omnigraph": {
                    "enabled": true,
                    "branch": "session/legacy-1",
                    "injected_atoms": []
                }
            }),
        )
        .unwrap();
    assert_eq!(updated["omnigraph"]["enabled"], true);
    assert_eq!(updated["omnigraph"]["branch"], "session/legacy-1");
}

#[test]
fn prompt_build_injects_bounded_omnigraph_context_and_tracks_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let store = SessionStore::open(&dir.path().join("state")).unwrap();
    let registered = store.register(&project).unwrap();

    let huge_capsule = "Omnigraph insight line: check token expiry.\n".repeat(300);
    assert!(huge_capsule.chars().count() > 6000);
    let created = store
        .create(
            &json!({
                "project_id": registered["id"],
                "provider": "claude",
                "mode": "plan",
                "workflow": "native",
                "project_context": false,
                "omnigraph": {
                    "enabled": true,
                    "branch": "session/sessprompt01"
                }
            }),
            "Review auth logic",
            &[],
        )
        .unwrap();
    let sid = created["id"].as_str().unwrap().to_owned();
    let mut omni_patched = created["omnigraph"].clone();
    omni_patched["capsule_text"] = json!(huge_capsule);
    let session = store
        .patch(&sid, &json!({ "omnigraph": omni_patched }))
        .unwrap();

    // 1. Standard non-slash prompt (`route.mode == "text"`)
    let route_text = infra_harness::commands::ResolvedCommand {
        mode: "text".into(),
        text: "Review auth logic".into(),
        requests: vec![],
        notice: None,
    };
    let prepared = prompt::build(&store, &session, "Review auth logic", &route_text, "").unwrap();
    assert!(prepared.text.contains("Shared Omnigraph Context"));
    assert!(prepared.text.contains("…[truncated]"));
    let omni_chars = prepared.ledger["omnigraph"].as_u64().unwrap();
    assert!(
        omni_chars > 5000 && omni_chars < 6500,
        "ledger={}",
        prepared.ledger
    );
    assert_eq!(
        prepared.ledger["total"].as_u64().unwrap(),
        prepared.text.chars().count() as u64
    );

    // 2. Native Claude slash command (`route.mode == "native"`) with omnigraph.enabled == true
    let route_native = infra_harness::commands::ResolvedCommand {
        mode: "native".into(),
        text: "/review-auth".into(),
        requests: vec![],
        notice: None,
    };
    let native_prepared =
        prompt::build(&store, &session, "/review-auth", &route_native, "").unwrap();
    assert!(native_prepared.text.starts_with("/review-auth"));
    assert!(native_prepared.text.contains("Shared Omnigraph Context"));
    assert!(native_prepared.ledger["omnigraph"].as_u64().unwrap() > 0);
    assert_eq!(
        native_prepared.ledger["total"].as_u64().unwrap(),
        native_prepared.text.chars().count() as u64
    );

    // 3. Native slash command WITHOUT omnigraph enabled remains untouched
    let mut disabled_session = session.clone();
    disabled_session["omnigraph"]["enabled"] = json!(false);
    let plain_native =
        prompt::build(&store, &disabled_session, "/review-auth", &route_native, "").unwrap();
    assert_eq!(plain_native.text, "/review-auth");
    assert!(plain_native.ledger["omnigraph"].is_null());

    // 4. PromptLedger accounting invariant sanity check
    let mut ledger = PromptLedger::new();
    ledger.push("message", "hi");
    ledger.push("omnigraph", "shared");
    let v = ledger.value();
    assert_eq!(v["message"], 2);
    assert_eq!(v["omnigraph"], 6);
    assert_eq!(v["instructions"], 0);
    assert_eq!(v["total"], 8);
}

#[tokio::test]
async fn post_turn_publish_collects_draft_clash_fleet_files_and_auto_merges() {
    let (mock_url, mock_task) = spawn_mock_omnigraph("tok-pub").await;
    let fx = HarnessFixture::new(None).await;
    fx.omnigraph
        .connect_external(&mock_url, Some("tok-pub"), Some(schema::HARNESS_GRAPH_ID))
        .await
        .unwrap();

    let sess = fx
        .store
        .create(
            &json!({
                "project_id": fx.project_id,
                "provider": "claude",
                "mode": "edit",
                "workflow": "native",
                "project_context": false,
                "omnigraph": {
                    "enabled": true,
                    "branch": "session/pubtest001",
                    "pull_branches": ["main"],
                    "auto_publish": true,
                    "auto_merge": true
                }
            }),
            "Harden JWT expiry and session cache",
            &[],
        )
        .unwrap();
    let sid = sess["id"].as_str().unwrap().to_owned();
    let generation = fx.store.generation(&sid).unwrap();
    fx.results.start_launch(&sid, &generation, None).unwrap();
    fx.results
        .save_receipt(
            &generation,
            &json!({
                "edited_paths": ["src/auth/jwt.rs"],
                "opened_paths": ["src/cache/redis.rs"]
            }),
        )
        .unwrap();

    // Emit a memory-draft event and simulate clash_result on session
    let draft_text = "Completed hardening.\n\n```memory-draft\n{\"progress\":\"Hardened JWT validation and prefixed Redis keys.\",\"next_steps\":[],\"learnings\":[{\"type\":\"decision\",\"title\":\"Require explicit leeway in JWT validation\",\"consequence\":\"Rejects expired tokens immediately without clock drift ambiguity.\",\"sources\":[\"src/auth/jwt.rs\"]}]}\n```";
    fx.store
        .guarded_event(
            &sid,
            &generation,
            &json!({ "kind": "result", "text": draft_text }),
        )
        .unwrap();
    fx.store
        .patch(
            &sid,
            &json!({
                "clash_result": {
                    "kind": "clash_state",
                    "summary": "Clash verified JWT guard",
                    "items": [{
                        "id": 1,
                        "kind": "finding",
                        "origin": "challenger",
                        "severity": "high",
                        "file": "src/auth/jwt.rs",
                        "line": 42,
                        "claim": "Missing audience check on refresh token",
                        "evidence": "Line 42 checks exp only",
                        "status": "confirmed"
                    }]
                }
            }),
        )
        .unwrap();

    let pub_out = fx
        .results
        .publish_omnigraph_for_run(
            &sid,
            &generation,
            &json!({ "kind": "result", "ok": true, "text": draft_text }),
            &AtomicBool::new(false),
        )
        .await
        .unwrap()
        .expect("published");
    assert_eq!(pub_out["ok"], true);
    assert_eq!(pub_out["merged"], true);
    assert_eq!(pub_out["published_atom_ids"].as_array().unwrap().len(), 2);

    let updated = fx.store.get(&sid).unwrap();
    assert_eq!(
        updated["omnigraph"]["published_atoms"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(updated["omnigraph"]["merge_status"], "fast_forward");

    // Subsequent session B preparing context from `main` receives both published atoms
    let sess_b = fx
        .store
        .create(
            &json!({
                "project_id": fx.project_id,
                "provider": "claude",
                "mode": "plan",
                "workflow": "native",
                "project_context": false,
                "omnigraph": {
                    "enabled": true,
                    "branch": "session/sessb00001",
                    "pull_branches": ["main"],
                    "query": "JWT audience"
                }
            }),
            "Audit refresh token verification",
            &[],
        )
        .unwrap();
    let prep = fx
        .omnigraph
        .prepare_session_context(&sess_b, &fx.project_path)
        .await
        .unwrap();
    assert_eq!(prep["enabled"], true);
    let capsule = prep["capsule_text"].as_str().unwrap();
    assert!(capsule.contains("Require explicit leeway in JWT validation"));
    assert!(capsule.contains("Missing audience check on refresh token"));

    // Fleet findings collection check
    let fleet_sess = fx
        .store
        .create(
            &json!({
                "project_id": fx.project_id,
                "provider": "claude",
                "mode": "plan",
                "workflow": "fleet-review",
                "project_context": false,
                "omnigraph": {
                    "enabled": true,
                    "branch": "session/fleet0001",
                    "auto_publish": true,
                    "auto_merge": false
                }
            }),
            "Fleet security review",
            &[],
        )
        .unwrap();
    let fsid = fleet_sess["id"].as_str().unwrap().to_owned();
    let fgen = fx.store.generation(&fsid).unwrap();
    let fpub = fx
        .results
        .publish_omnigraph_for_run(
            &fsid,
            &fgen,
            &json!({
                "kind": "fleet_state",
                "status": "completed",
                "findings": [{
                    "lens": "security-reviewer",
                    "severity": "high",
                    "file": "src/api.rs",
                    "line": 10,
                    "claim": "Unbounded query pagination parameter",
                    "evidence": "limit param is not clamped"
                }]
            }),
            &AtomicBool::new(false),
        )
        .await
        .unwrap()
        .expect("fleet published");
    assert_eq!(fpub["published_atom_ids"].as_array().unwrap().len(), 1);
    assert_eq!(
        fx.store.get(&fsid).unwrap()["omnigraph"]["merge_status"],
        "unmerged"
    );

    // Graceful degradation when Omnigraph becomes unreachable
    mock_task.abort();
    let (learnings, _, _) = fx
        .results
        .collect_omnigraph_outputs(&fsid, &fgen, &json!({ "kind": "result", "text": "done" }))
        .await
        .unwrap();
    assert!(!learnings.is_empty());
    let degraded = fx
        .results
        .publish_omnigraph_for_run(
            &fsid,
            &fgen,
            &json!({ "kind": "result", "ok": true, "text": "done" }),
            &AtomicBool::new(false),
        )
        .await
        .unwrap();
    assert!(degraded.is_none());
    let f_after = fx.store.get(&fsid).unwrap();
    assert!(f_after["omnigraph"]["last_error"].as_str().is_some());

    fx.shutdown().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn end_to_end_http_session_create_executes_prepares_prompt_and_publishes_to_next_session() {
    use std::os::unix::fs::PermissionsExt;

    let (mock_url, mock_task) = spawn_mock_omnigraph("tok-e2e").await;
    let bin_dir = tempfile::tempdir().unwrap();
    let fake_claude = bin_dir.path().join("claude");
    let captured_prompt_a = bin_dir.path().join("prompt_a.txt");
    let captured_prompt_b = bin_dir.path().join("prompt_b.txt");
    let result_line = json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "session_id": "native-omni-1",
        "result": "Implemented auth middleware.\n\n```memory-draft\n{\"progress\":\"Added tenant-scoped JWT guard\",\"next_steps\":[],\"learnings\":[{\"type\":\"finding\",\"title\":\"Tenant header must match JWT claim\",\"consequence\":\"Reject cross-tenant requests at gateway middleware.\",\"sources\":[\"src/gateway.rs\"]}]}\n```"
    })
    .to_string();
    let script = format!(
        "#!/bin/sh\nif [ ! -f \"{a}\" ]; then\n  cat > \"{a}\"\nelse\n  cat > \"{b}\"\nfi\ncat <<'EOF'\n{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"native-omni-1\"}}\n{result_line}\nEOF\n",
        a = captured_prompt_a.display(),
        b = captured_prompt_b.display(),
    );
    fs::write(&fake_claude, script).unwrap();
    fs::set_permissions(&fake_claude, fs::Permissions::from_mode(0o755)).unwrap();

    let fx = HarnessFixture::new(Some(fake_claude)).await;
    fs::write(fx.project_path.join("src_marker.txt"), "ok").unwrap();

    let (status, _) = fx
        .http(
            Method::POST,
            "/api/omnigraph/setup",
            Some(json!({
                "action": "connect",
                "endpoint": mock_url,
                "token": "tok-e2e",
                "graph_id": schema::HARNESS_GRAPH_ID
            })),
            true,
        )
        .await;
    assert_eq!(status, 200);

    // Session A via POST /api/sessions with omnigraph enabled & auto_merge: true
    let (status, created_a) = fx
        .http(
            Method::POST,
            "/api/sessions",
            Some(json!({
                "project_id": fx.project_id,
                "provider": "claude",
                "mode": "plan",
                "workflow": "native",
                "project_context": false,
                "prompt": "Implement tenant header check",
                "omnigraph": {
                    "enabled": true,
                    "branch": "auto",
                    "pull_branches": ["main"],
                    "auto_publish": true,
                    "auto_merge": true
                }
            })),
            true,
        )
        .await;
    assert_eq!(status, 201, "{created_a}");
    let sid_a = created_a["session"]["id"].as_str().unwrap().to_owned();
    assert_eq!(created_a["session"]["omnigraph"]["enabled"], true);
    assert!(
        created_a["session"]["omnigraph"]["branch"]
            .as_str()
            .unwrap()
            .starts_with("session/")
    );

    let until = Instant::now() + Duration::from_secs(15);
    let sess_a_done = loop {
        let s = fx.store.get(&sid_a).unwrap();
        if s["status"] == "completed" {
            break s;
        }
        if s["status"] == "failed" {
            let events = fx.store.events(&sid_a, 0).unwrap();
            panic!("Session A failed: {s}\nevents={events:?}");
        }
        assert!(Instant::now() < until, "Session A did not finish: {s}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert_eq!(
        sess_a_done["omnigraph"]["published_atoms"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(sess_a_done["omnigraph"]["merge_status"], "fast_forward");

    // Session B via POST /api/sessions pulls from `main` and receives Session A's finding in its prompt!
    let (status, created_b) = fx
        .http(
            Method::POST,
            "/api/sessions",
            Some(json!({
                "project_id": fx.project_id,
                "provider": "claude",
                "mode": "plan",
                "workflow": "native",
                "project_context": false,
                "prompt": "Check tenant gateway middleware",
                "omnigraph": {
                    "enabled": true,
                    "pull_branches": ["main"],
                    "query": "Tenant header JWT"
                }
            })),
            true,
        )
        .await;
    assert_eq!(status, 201, "{created_b}");
    let sid_b = created_b["session"]["id"].as_str().unwrap().to_owned();

    let until = Instant::now() + Duration::from_secs(15);
    loop {
        let s = fx.store.get(&sid_b).unwrap();
        if s["status"] == "completed" {
            break;
        }
        assert!(Instant::now() < until, "Session B did not finish: {s}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let prompt_b_text = fs::read_to_string(&captured_prompt_b).unwrap();
    assert!(
        prompt_b_text.contains("Shared Omnigraph Context")
            && prompt_b_text.contains("Tenant header must match JWT claim"),
        "Session B prompt did not receive Session A's shared Omnigraph context:\n{prompt_b_text}"
    );

    fx.shutdown().await;
    mock_task.abort();
}
