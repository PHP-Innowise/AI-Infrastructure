use super::graph_ops::now_utc_iso_ms;
use super::helpers::{
    AtomProvenance, atom_symbols_for_project, build_upsert_atom_params, clean_opt, detect_language,
    format_atom_line_with_authority, format_context_capsule, format_omnigraph_ts,
    list_atom_symbols, normalize_atom_row, normalize_file_row, normalize_session_row,
    normalize_symbol_row, normalize_turn_row, parse_string_list, parse_tags_opt, pascal_to_snake,
    str_field, symbol_edges, upsert_atom_files, upsert_atom_symbols,
};
use super::{OmnigraphManager, OmnigraphMode, VALID_ATOM_KINDS};
use crate::omnigraph::{
    client::{
        BranchCreateRequest, BranchMergeRequest, ChangeBaselineRequest, ChangeFeedQuery,
        ChangeRequest, CommitOutput, EntityKind, MergeConflict, MergeConflictKind, OmnigraphClient,
        QueryRequest, StoredQueryRequest,
    },
    schema::{
        self, CANONICAL_KNOWLEDGE_BRANCH, GQ_CLEAR_BRANCH_CONTEXT, GQ_DELETE_CANONICAL_ATOM,
        GQ_LINK_CONSUMED_BY, GQ_LINK_DERIVED_FROM, GQ_LINK_RELATES_TO, GQ_LINK_SESSION_FOLLOWS,
        GQ_LINK_TOUCHES_FILE, GQ_LIST_FILE_EDGES, GQ_LIST_SESSION_TURN_SNAPSHOTS, GQ_UPSERT_ATOM,
        GQ_UPSERT_FILE, GQ_UPSERT_SESSION, GQ_UPSERT_TURN_SNAPSHOT, RECENT_ATOMS_LIMIT,
    },
    session_branch_name,
    transfer::{
        self, BUNDLE_FORMAT_V1, filter_graph_for_export, filter_ndjson_excluding_atoms,
        normalize_link_target_branch, normalize_shared_space_branch, parse_bool_param,
        parse_import_body, parse_ndjson_records, prepare_import_ndjson, serialize_graph_ndjson,
        slugify_segment, validate_branch_token,
    },
};
use crate::{
    HarnessError, Result,
    services::Payload,
    store::{self, SessionStore},
};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
struct CanonicalSyncLedgerEntry {
    atom_sha256: String,
    files_sha256: String,
    supersedes_sha256: String,
}

fn open_canonical_sync_ledger(state_dir: &Path) -> Option<rusqlite::Connection> {
    let dir = state_dir.join("cache/omnigraph");
    std::fs::create_dir_all(&dir).ok()?;
    let db = rusqlite::Connection::open(dir.join("sync_ledger.sqlite3")).ok()?;
    db.execute_batch(
        "PRAGMA busy_timeout = 5000;
         CREATE TABLE IF NOT EXISTS canonical_sync_ledger(
             endpoint TEXT NOT NULL,
             graph_id TEXT NOT NULL,
             branch TEXT NOT NULL,
             project_id TEXT NOT NULL,
             atom_id TEXT NOT NULL,
             atom_sha256 TEXT NOT NULL,
             files_sha256 TEXT NOT NULL,
             supersedes_sha256 TEXT NOT NULL DEFAULT '',
             updated_at TEXT NOT NULL,
             PRIMARY KEY(endpoint, graph_id, branch, project_id, atom_id)
         );",
    )
    .ok()?;
    Some(db)
}

fn load_canonical_sync_ledger(
    db: &rusqlite::Connection,
    endpoint: &str,
    graph_id: &str,
    branch: &str,
    project_id: &str,
) -> BTreeMap<String, CanonicalSyncLedgerEntry> {
    let mut out = BTreeMap::new();
    let Ok(mut stmt) = db.prepare(
        "SELECT atom_id, atom_sha256, files_sha256, supersedes_sha256
         FROM canonical_sync_ledger
         WHERE endpoint = ?1 AND graph_id = ?2 AND branch = ?3 AND project_id = ?4",
    ) else {
        return out;
    };
    let Ok(rows) = stmt.query_map(
        rusqlite::params![endpoint, graph_id, branch, project_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                CanonicalSyncLedgerEntry {
                    atom_sha256: row.get(1)?,
                    files_sha256: row.get(2)?,
                    supersedes_sha256: row.get(3)?,
                },
            ))
        },
    ) else {
        return out;
    };
    for (atom_id, entry) in rows.flatten() {
        out.insert(atom_id, entry);
    }
    out
}

#[expect(
    clippy::too_many_arguments,
    reason = "Ledger upsert stores the multi-tenant key plus per-atom, file-edge, and supersedes digests."
)]
fn save_canonical_sync_ledger(
    db: &rusqlite::Connection,
    endpoint: &str,
    graph_id: &str,
    branch: &str,
    project_id: &str,
    atom_id: &str,
    entry: &CanonicalSyncLedgerEntry,
    ts: &str,
) {
    let _ = db.execute(
        "INSERT INTO canonical_sync_ledger(
             endpoint, graph_id, branch, project_id, atom_id,
             atom_sha256, files_sha256, supersedes_sha256, updated_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(endpoint, graph_id, branch, project_id, atom_id) DO UPDATE SET
             atom_sha256 = excluded.atom_sha256,
             files_sha256 = excluded.files_sha256,
             supersedes_sha256 = excluded.supersedes_sha256,
             updated_at = excluded.updated_at",
        rusqlite::params![
            endpoint,
            graph_id,
            branch,
            project_id,
            atom_id,
            entry.atom_sha256,
            entry.files_sha256,
            entry.supersedes_sha256,
            ts
        ],
    );
}

fn delete_canonical_sync_ledger(
    db: &rusqlite::Connection,
    endpoint: &str,
    graph_id: &str,
    branch: &str,
    project_id: &str,
    atom_id: &str,
) {
    let _ = db.execute(
        "DELETE FROM canonical_sync_ledger
         WHERE endpoint = ?1 AND graph_id = ?2 AND branch = ?3 AND project_id = ?4 AND atom_id = ?5",
        rusqlite::params![endpoint, graph_id, branch, project_id, atom_id],
    );
}

#[expect(
    clippy::too_many_arguments,
    reason = "Canonical atom digest hashes every semantic parameter passed to GQ_UPSERT_ATOM without volatile sync clocks."
)]
fn canonical_atom_digest(
    atom_id: &str,
    effective_project_id: &str,
    kind: &str,
    title: &str,
    summary: &str,
    body: &str,
    tags: Option<Vec<String>>,
    branch: &str,
    source_session_id: Option<&str>,
    confidence: f64,
    pin_priority: i32,
    verified: bool,
    prov: &AtomProvenance,
) -> String {
    let mut params_without_clocks = build_upsert_atom_params(
        atom_id,
        effective_project_id,
        kind,
        title,
        summary,
        body,
        tags,
        branch,
        source_session_id,
        confidence,
        pin_priority,
        verified,
        prov,
        "",
        "",
    );
    if let Some(obj) = params_without_clocks.as_object_mut() {
        obj.remove("created_at");
        obj.remove("updated_at");
    }
    let canonical = infra_core::domain::json::canonical(&params_without_clocks)
        .unwrap_or_else(|_| params_without_clocks.to_string());
    store::sha(canonical.as_bytes())
}

fn canonical_sorted_list_digest(values: &[String]) -> String {
    let sorted: BTreeSet<&str> = values.iter().map(String::as_str).collect();
    let encoded = serde_json::to_string(&sorted).unwrap_or_default();
    store::sha(encoded.as_bytes())
}

async fn list_atom_files_map(
    client: &OmnigraphClient,
    project_id: &str,
    branch: &str,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut by_atom: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let edges = query_edges_safe(
        client,
        GQ_LIST_FILE_EDGES,
        "list_file_edges",
        &json!({ "project_id": project_id }),
        Some(branch),
        None,
    )
    .await;
    for edge in edges {
        if let (Some(atom_id), Some(path)) =
            (str_field(&edge, "source_id"), str_field(&edge, "target_id"))
        {
            by_atom.entry(atom_id).or_default().insert(path);
        }
    }
    by_atom
}

impl OmnigraphManager {
    /// Explicitly share selected `atom_ids` into `target_session_id` (recording `ConsumedBy` edges).
    pub async fn share_into_session_json(&self, body: &Value) -> Result<Value> {
        let target_session_id = store::text(&body["target_session_id"], "target_session_id", 128)?;
        let atom_ids: Vec<String> = body["atom_ids"]
            .as_array()
            .ok_or_else(|| HarnessError::invalid("`atom_ids` must be a non-empty array."))?
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if atom_ids.is_empty() {
            return Err(HarnessError::invalid(
                "`atom_ids` must contain at least one atom id.",
            ));
        }
        let source_branch = clean_opt(body["source_branch"].as_str())
            .unwrap_or("main")
            .to_string();
        let mode = clean_opt(body["mode"].as_str())
            .unwrap_or("reference")
            .to_ascii_lowercase();
        if !matches!(
            mode.as_str(),
            "inject" | "reference" | "inject_and_link" | "copy"
        ) {
            return Err(HarnessError::invalid(format!(
                "Unknown share mode '{mode}'."
            )));
        }
        let project_id = clean_opt(body["project_id"].as_str())
            .unwrap_or("")
            .to_string();

        let target_branch = session_branch_name(&target_session_id);
        let client = self.client().await?;
        ensure_branch_exists(&client, &target_branch, Some(&source_branch)).await?;

        let ts = now_utc_iso_ms();
        let session_meta = self
            .store
            .as_ref()
            .and_then(|s| s.get(&target_session_id).ok());
        let sess_project_id = session_meta
            .as_ref()
            .and_then(|s| s["project_id"].as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&project_id);
        let sess_title = session_meta
            .as_ref()
            .and_then(|s| s["title"].as_str())
            .unwrap_or("Harness Session");
        let sess_provider = session_meta
            .as_ref()
            .and_then(|s| s["provider"].as_str())
            .unwrap_or("codex");
        let sess_workflow = session_meta
            .as_ref()
            .and_then(|s| s["workflow"].as_str())
            .unwrap_or("native");
        let sess_status = session_meta
            .as_ref()
            .and_then(|s| s["status"].as_str())
            .unwrap_or("running");

        let _ = client
            .mutate(&ChangeRequest {
                query: GQ_UPSERT_SESSION.into(),
                name: Some("upsert_session".into()),
                params: Some(json!({
                    "session_id": target_session_id,
                    "project_id": sess_project_id,
                    "title": sess_title,
                    "branch": target_branch,
                    "provider": sess_provider,
                    "workflow": sess_workflow,
                    "status": sess_status,
                    "goal": Value::Null,
                    "created_at": ts,
                    "updated_at": ts,
                })),
                branch: Some(target_branch.clone()),
                settings: None,
            })
            .await?;

        let mut shared = Vec::new();
        let mut last_commit = None;
        if mode == "copy" && source_branch != target_branch {
            let source_atoms = run_stored_or_inline(
                &client,
                "recent_atoms",
                schema::QUERY_RECENT_ATOMS,
                json!({ "project_id": sess_project_id }),
                Some(source_branch.clone()),
                None,
            )
            .await?;
            let source_file_edges = query_edges_safe(
                &client,
                GQ_LIST_FILE_EDGES,
                "list_file_edges",
                &json!({ "project_id": sess_project_id }),
                Some(&source_branch),
                None,
            )
            .await;
            for row in &source_atoms.rows {
                let atom = normalize_atom_row(row);
                let aid = str_field(&atom, "atom_id").unwrap_or_default();
                if atom_ids.contains(&aid) {
                    let prov = AtomProvenance::from_value(&atom);
                    let params = build_upsert_atom_params(
                        &aid,
                        sess_project_id,
                        atom["kind"].as_str().unwrap_or("insight"),
                        atom["title"].as_str().unwrap_or(""),
                        atom["summary"].as_str().unwrap_or(""),
                        atom["body"].as_str().unwrap_or(""),
                        parse_tags_opt(&atom["tags"]),
                        &source_branch,
                        atom["source_session_id"].as_str(),
                        atom["confidence"].as_f64().unwrap_or(0.8),
                        atom["pin_priority"].as_i64().unwrap_or(10) as i32,
                        atom["verified"].as_bool().unwrap_or(false),
                        &prov,
                        &ts,
                        &ts,
                    );
                    if let Ok(r) = client
                        .mutate(&ChangeRequest {
                            query: GQ_UPSERT_ATOM.into(),
                            name: Some("upsert_atom".into()),
                            params: Some(params),
                            branch: Some(target_branch.clone()),
                            settings: None,
                        })
                        .await
                    {
                        if r.commit.is_some() {
                            last_commit = r.commit;
                        }
                        let mut copied_paths = BTreeSet::new();
                        for fe in source_file_edges.iter().filter(|fe| {
                            str_field(fe, "source_id").as_deref() == Some(aid.as_str())
                        }) {
                            let Some(path) = str_field(fe, "target_id") else {
                                continue;
                            };
                            if !copied_paths.insert(path.clone()) {
                                continue;
                            }
                            let _ = client
                                .mutate(&ChangeRequest {
                                    query: GQ_UPSERT_FILE.into(),
                                    name: Some("upsert_file".into()),
                                    params: Some(json!({
                                        "path": path,
                                        "project_id": sess_project_id,
                                        "language": detect_language(&path),
                                        "last_touched_at": ts,
                                    })),
                                    branch: Some(target_branch.clone()),
                                    settings: None,
                                })
                                .await;
                            if let Ok(fr) = client
                                .mutate(&ChangeRequest {
                                    query: GQ_LINK_TOUCHES_FILE.into(),
                                    name: Some("link_touches_file".into()),
                                    params: Some(json!({
                                        "atom_id": aid,
                                        "path": path,
                                        "access": str_field(fe, "access")
                                            .unwrap_or_else(|| "referenced".to_string()),
                                        "line_start": fe
                                            .get("line_start")
                                            .cloned()
                                            .unwrap_or(Value::Null),
                                        "line_end": fe
                                            .get("line_end")
                                            .cloned()
                                            .unwrap_or(Value::Null),
                                    })),
                                    branch: Some(target_branch.clone()),
                                    settings: None,
                                })
                                .await
                                && fr.commit.is_some()
                            {
                                last_commit = fr.commit;
                            }
                        }
                    }
                }
            }
        }

        for aid in &atom_ids {
            let res = client
                .mutate(&ChangeRequest {
                    query: GQ_LINK_CONSUMED_BY.into(),
                    name: Some("link_consumed_by".into()),
                    params: Some(json!({
                        "atom_id": aid,
                        "session_id": target_session_id,
                        "consumed_at": ts,
                        "injection_mode": mode,
                        "relevance": 1.0,
                    })),
                    branch: Some(target_branch.clone()),
                    settings: None,
                })
                .await;
            if let Ok(r) = res {
                shared.push(aid.clone());
                if r.commit.is_some() {
                    last_commit = r.commit;
                }
            }
        }

        Ok(json!({
            "ok": true,
            "target_session_id": target_session_id,
            "source_branch": source_branch,
            "target_branch": target_branch,
            "mode": mode,
            "shared_count": shared.len(),
            "atom_ids": shared,
            "commit": last_commit,
        }))
    }

    /// Create `session/<sid12>` branch, upsert `Session`, pull configured branches, and retrieve a ranked context capsule.
    pub async fn prepare_session_context(
        &self,
        session: &Value,
        project_path: &Path,
    ) -> Result<Value> {
        let cfg = self.config().await;
        if cfg.mode == OmnigraphMode::Off {
            return Ok(json!({
                "enabled": false,
                "capsule_text": "",
                "atoms": [],
            }));
        }
        let client = self.client().await?;
        let sid = session["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| HarnessError::invalid("Session object is missing `id`."))?;
        let project_id = session["project_id"].as_str().unwrap_or("");
        let omni_opts = &session["omnigraph"];
        let branch = clean_opt(omni_opts["branch"].as_str())
            .filter(|b| *b != "auto")
            .map(str::to_string)
            .unwrap_or_else(|| session_branch_name(sid));

        ensure_branch_exists(&client, &branch, Some("main")).await?;
        if let Some(raw_space) = clean_opt(omni_opts["shared_space"].as_str())
            && let Ok(space_branch) = normalize_shared_space_branch(raw_space)
            && space_branch != "main"
            && space_branch != branch
        {
            let _ = ensure_branch_exists(&client, &space_branch, Some("main")).await;
        }

        if let Some(pull_branches) = omni_opts["pull_branches"].as_array() {
            for pull in pull_branches.iter().filter_map(Value::as_str) {
                let pull = pull.trim();
                if !pull.is_empty() && pull != branch {
                    let _ = merge_or_sync_branch(&client, pull, &branch, project_id).await;
                }
            }
        }

        let ts = now_utc_iso_ms();
        // The search text is written into the shared graph's Session node, so
        // it is screened as an automatic memory query first.
        let goal = [
            omni_opts["query"].as_str(),
            session["goal"].as_str(),
            session["prompt"].as_str(),
            session["title"].as_str(),
        ]
        .into_iter()
        .flatten()
        .find_map(graph_session_text)
        .unwrap_or_default();
        let goal = goal.as_str();
        let title = session["title"].as_str().unwrap_or("Harness Session");
        let provider = session["provider"].as_str().unwrap_or("codex");
        let workflow = session["workflow"].as_str().unwrap_or("native");
        let status = session["status"].as_str().unwrap_or("running");
        let goal_opt = (!goal.is_empty()).then_some(goal);

        let session_upsert = client
            .mutate(&ChangeRequest {
                query: GQ_UPSERT_SESSION.into(),
                name: Some("upsert_session".into()),
                params: Some(json!({
                    "session_id": sid,
                    "project_id": project_id,
                    "title": title,
                    "branch": branch,
                    "provider": provider,
                    "workflow": workflow,
                    "status": status,
                    "goal": goal_opt,
                    "created_at": ts,
                    "updated_at": ts,
                })),
                branch: Some(branch.clone()),
                settings: None,
            })
            .await?;

        if let Some(peers) = omni_opts["linked_session_ids"].as_array()
            && !peers.is_empty()
        {
            let rel = canonical_session_follows_relation(omni_opts);
            for peer_id in peers.iter().filter_map(Value::as_str) {
                let peer_id = peer_id.trim();
                if peer_id.is_empty() || peer_id == sid {
                    continue;
                }
                let peer_meta = self.store.as_ref().and_then(|s| s.get(peer_id).ok());
                let _ = upsert_session_node(
                    &client,
                    peer_meta.as_ref(),
                    peer_id,
                    project_id,
                    &branch,
                    &ts,
                )
                .await;
                let _ = link_session_follows(&client, sid, peer_id, rel, &branch, &ts).await;
                let _ = link_session_follows(&client, peer_id, sid, rel, &branch, &ts).await;
            }
        }

        let has_canonical_branch = branch != CANONICAL_KNOWLEDGE_BRANCH
            && client.list_branches(false).await.is_ok_and(|list| {
                list.branches
                    .iter()
                    .any(|b| b == CANONICAL_KNOWLEDGE_BRANCH)
            });
        let mut queried_branches = vec![branch.clone()];
        if has_canonical_branch {
            queried_branches.push(CANONICAL_KNOWLEDGE_BRANCH.to_string());
        }

        let mut ranked_atoms: Vec<(Value, f64, String)> = Vec::new();
        let mut seen_ids = BTreeSet::new();
        if !goal.is_empty()
            && let Ok(search) = self
                .search_json(project_id, goal, Some(&branch), None, Some(8))
                .await
            && let Some(hits) = search["hits"].as_array()
        {
            for hit in hits {
                let atom = hit["atom"].clone();
                if let Some(aid) = str_field(&atom, "atom_id")
                    && seen_ids.insert(aid)
                {
                    ranked_atoms.push((atom, hit["score"].as_f64().unwrap_or(0.5), branch.clone()));
                }
            }
        }
        if has_canonical_branch
            && !goal.is_empty()
            && let Ok(search) = self
                .search_json(
                    project_id,
                    goal,
                    Some(CANONICAL_KNOWLEDGE_BRANCH),
                    None,
                    Some(6),
                )
                .await
            && let Some(hits) = search["hits"].as_array()
        {
            for hit in hits {
                let atom = hit["atom"].clone();
                if let Some(aid) = str_field(&atom, "atom_id")
                    && seen_ids.insert(aid)
                {
                    ranked_atoms.push((
                        atom,
                        hit["score"].as_f64().unwrap_or(0.5),
                        CANONICAL_KNOWLEDGE_BRANCH.to_string(),
                    ));
                }
            }
        }
        let branch_recent_cap = if has_canonical_branch { 8 } else { 10 };
        if ranked_atoms.len() < branch_recent_cap
            && let Ok(recent) = run_stored_or_inline(
                &client,
                "recent_atoms",
                schema::QUERY_RECENT_ATOMS,
                json!({ "project_id": project_id }),
                Some(branch.clone()),
                None,
            )
            .await
        {
            for row in &recent.rows {
                let atom = normalize_atom_row(row);
                if let Some(aid) = str_field(&atom, "atom_id")
                    && seen_ids.insert(aid)
                {
                    let prio = atom["pin_priority"].as_f64().unwrap_or(0.0);
                    ranked_atoms.push((atom, prio / 100.0, branch.clone()));
                }
                if ranked_atoms.len() >= branch_recent_cap {
                    break;
                }
            }
        }
        if has_canonical_branch
            && (goal.is_empty() || ranked_atoms.len() < 10)
            && let Ok(recent) = run_stored_or_inline(
                &client,
                "recent_atoms",
                schema::QUERY_RECENT_ATOMS,
                json!({ "project_id": project_id }),
                Some(CANONICAL_KNOWLEDGE_BRANCH.to_string()),
                None,
            )
            .await
        {
            let mut added_canonical = 0usize;
            for row in &recent.rows {
                let atom = normalize_atom_row(row);
                if let Some(aid) = str_field(&atom, "atom_id")
                    && seen_ids.insert(aid)
                {
                    let prio = atom["pin_priority"].as_f64().unwrap_or(0.0);
                    ranked_atoms.push((atom, prio / 100.0, CANONICAL_KNOWLEDGE_BRANCH.to_string()));
                    added_canonical += 1;
                }
                if ranked_atoms.len() >= 12 || added_canonical >= 6 {
                    break;
                }
            }
        }

        let mut files_by_branch_atom: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
        let mut files_by_atom: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for q_branch in &queried_branches {
            let file_edges = query_edges_safe(
                &client,
                GQ_LIST_FILE_EDGES,
                "list_file_edges",
                &json!({ "project_id": project_id }),
                Some(q_branch),
                None,
            )
            .await;
            for edge in &file_edges {
                if let (Some(src), Some(dst)) =
                    (str_field(edge, "source_id"), str_field(edge, "target_id"))
                {
                    let per_branch = files_by_branch_atom
                        .entry((q_branch.clone(), src.clone()))
                        .or_default();
                    if !per_branch.contains(&dst) {
                        per_branch.push(dst.clone());
                    }
                    let all = files_by_atom.entry(src).or_default();
                    if !all.contains(&dst) {
                        all.push(dst);
                    }
                }
            }
        }

        let mut consumed_count = 0usize;
        let mut canonical_consumed = Vec::new();
        let mut atoms_out = Vec::new();
        for (raw_atom, score, origin_query_branch) in ranked_atoms {
            let mut atom = normalize_atom_row(&raw_atom);
            if let Some(aid) = str_field(&atom, "atom_id") {
                let atom_files = files_by_branch_atom
                    .get(&(origin_query_branch.clone(), aid.clone()))
                    .or_else(|| files_by_atom.get(&aid))
                    .cloned()
                    .unwrap_or_else(|| parse_string_list(&atom["files"]));
                atom["files"] = json!(atom_files);
                if origin_query_branch == branch {
                    let _ = client
                        .mutate(&ChangeRequest {
                            query: GQ_LINK_CONSUMED_BY.into(),
                            name: Some("link_consumed_by".into()),
                            params: Some(json!({
                                "atom_id": aid,
                                "session_id": sid,
                                "consumed_at": ts,
                                "injection_mode": "auto_capsule",
                                "relevance": score,
                            })),
                            branch: Some(branch.clone()),
                            settings: None,
                        })
                        .await;
                    consumed_count += 1;
                } else if origin_query_branch == CANONICAL_KNOWLEDGE_BRANCH {
                    canonical_consumed.push((aid, score));
                }
            } else {
                atom["files"] = json!([]);
            }
            atoms_out.push(atom);
        }
        // A canonical atom stays on its branch: the session that consumed it
        // is recorded there, not copied onto the session branch.
        if !canonical_consumed.is_empty()
            && upsert_session_node(
                &client,
                Some(session),
                sid,
                project_id,
                CANONICAL_KNOWLEDGE_BRANCH,
                &ts,
            )
            .await
            .is_ok()
        {
            for (aid, score) in canonical_consumed {
                if client
                    .mutate(&ChangeRequest {
                        query: GQ_LINK_CONSUMED_BY.into(),
                        name: Some("link_consumed_by".into()),
                        params: Some(json!({
                            "atom_id": aid,
                            "session_id": sid,
                            "consumed_at": ts,
                            "injection_mode": "auto_capsule",
                            "relevance": score,
                        })),
                        branch: Some(CANONICAL_KNOWLEDGE_BRANCH.to_string()),
                        settings: None,
                    })
                    .await
                    .is_ok()
                {
                    consumed_count += 1;
                }
            }
        }

        // An injected atom that another contradicts is delivered with a
        // caution; the relation itself is left for a person to resolve.
        let injected_ids = atoms_out
            .iter()
            .filter_map(|atom| str_field(atom, "atom_id"))
            .collect::<BTreeSet<_>>();
        let contradiction_warnings =
            contradictions_touching(&client, project_id, &queried_branches, &injected_ids).await;
        let capsule_text = format_context_capsule(
            &cfg.graph_id,
            &branch,
            project_path,
            &atoms_out,
            &contradiction_warnings,
        );
        Ok(json!({
            "enabled": true,
            "graph_id": cfg.graph_id,
            "branch": branch,
            "head_commit": session_upsert.commit.map(|c| c.graph_commit_id),
            "capsule_text": capsule_text,
            "atoms": atoms_out,
            "consumed_count": consumed_count,
            "contradiction_warnings": contradiction_warnings,
        }))
    }

    /// Publish completed session learnings, touched files, and turn summary onto the session's Omnigraph branch.
    pub async fn publish_session_outputs(
        &self,
        session: &Value,
        learnings: &[Value],
        files: &[String],
        summary: &str,
    ) -> Result<Value> {
        let cfg = self.config().await;
        if cfg.mode == OmnigraphMode::Off {
            return Ok(json!({
                "ok": false,
                "skipped": "omnigraph-off",
                "published_atom_ids": [],
            }));
        }
        let client = self.client().await?;
        let sid = session["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| HarnessError::invalid("Session object is missing `id`."))?;
        let project_id = session["project_id"].as_str().unwrap_or("");
        let omni_opts = &session["omnigraph"];
        let branch = clean_opt(omni_opts["branch"].as_str())
            .filter(|b| *b != "auto")
            .map(str::to_string)
            .unwrap_or_else(|| session_branch_name(sid));
        ensure_branch_exists(&client, &branch, Some("main")).await?;
        // The learnings name declarations of the files in the session's own
        // workspace, which may be a worktree of the project.
        let root = clean_opt(session["project_path"].as_str())
            .map(PathBuf::from)
            .or_else(|| self.project_root(project_id, Some(sid)));

        let ts = now_utc_iso_ms();
        let mut last_commit = None;

        for file_path in files {
            let trimmed = file_path.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(r) = client
                .mutate(&ChangeRequest {
                    query: GQ_UPSERT_FILE.into(),
                    name: Some("upsert_file".into()),
                    params: Some(json!({
                        "path": trimmed,
                        "project_id": project_id,
                        "language": detect_language(trimmed),
                        "last_touched_at": ts,
                    })),
                    branch: Some(branch.clone()),
                    settings: None,
                })
                .await
                && r.commit.is_some()
            {
                last_commit = r.commit;
            }
        }

        let snapshot_id =
            (!summary.trim().is_empty()).then(|| format!("snap-{}", &store::hexid()[..12]));
        let turn_index = if snapshot_id.is_some() {
            next_turn_index(&client, sid, &branch).await
        } else {
            1
        };
        if let Some(ref snap_id) = snapshot_id
            && let Ok(r) = client
                .mutate(&ChangeRequest {
                    query: GQ_UPSERT_TURN_SNAPSHOT.into(),
                    name: Some("upsert_turn_snapshot".into()),
                    params: Some(json!({
                        "snapshot_id": snap_id,
                        "session_id": sid,
                        "turn_index": turn_index,
                        "summary": summary.trim(),
                        "created_at": ts,
                    })),
                    branch: Some(branch.clone()),
                    settings: None,
                })
                .await
            && r.commit.is_some()
        {
            last_commit = r.commit;
        }

        let mut published_atom_ids = Vec::new();
        let mut symbol_count = 0usize;
        for learning in learnings {
            let raw_kind = learning["kind"]
                .as_str()
                .or_else(|| learning["type"].as_str())
                .unwrap_or("finding")
                .to_ascii_lowercase();
            let kind = if VALID_ATOM_KINDS.contains(&raw_kind.as_str()) {
                raw_kind
            } else {
                "insight".to_string()
            };
            let Some(title) = clean_opt(learning["title"].as_str()) else {
                continue;
            };
            let body_text = clean_opt(learning["body"].as_str())
                .or_else(|| clean_opt(learning["consequence"].as_str()))
                .or_else(|| clean_opt(learning["summary"].as_str()))
                .unwrap_or(title);
            let summary_text = clean_opt(learning["summary"].as_str())
                .or_else(|| clean_opt(learning["consequence"].as_str()))
                .unwrap_or(title);
            let mut atom_files: Vec<String> = learning["files"]
                .as_array()
                .or_else(|| learning["sources"].as_array())
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            if atom_files.is_empty() {
                atom_files.extend(files.iter().take(4).cloned());
            }

            let published = self
                .publish_atom_in(&json!({
                    "project_id": project_id,
                    "branch": branch,
                    "session_id": sid,
                    "kind": kind,
                    "title": title,
                    "summary": summary_text,
                    "body": body_text,
                    "tags": learning.get("tags").cloned().unwrap_or(Value::Null),
                    "files": atom_files,
                    "confidence": learning["confidence"].as_f64().unwrap_or(0.9),
                    "pin_priority": learning["pin_priority"].as_i64().unwrap_or(15),
                    "verified": learning["verified"].as_bool().unwrap_or(false),
                    "origin_layer": learning.get("origin_layer").cloned().unwrap_or(Value::Null),
                    "authority": learning.get("authority").cloned().unwrap_or(Value::Null),
                    "privacy": learning.get("privacy").cloned().unwrap_or(Value::Null),
                    "brain_record_id": learning.get("brain_record_id").cloned().unwrap_or(Value::Null),
                    "memory_id": learning.get("memory_id").cloned().unwrap_or(Value::Null),
                    "record_revision": learning.get("record_revision").cloned().unwrap_or(Value::Null),
                }), root.as_deref())
                .await?;
            symbol_count += published["symbols"].as_array().map_or(0, Vec::len);
            if let Some(aid) = published["atom"]["atom_id"].as_str() {
                published_atom_ids.push(aid.to_string());
                if let Some(ref snap_id) = snapshot_id {
                    let _ = client
                        .mutate(&ChangeRequest {
                            query: GQ_LINK_DERIVED_FROM.into(),
                            name: Some("link_derived_from".into()),
                            params: Some(json!({
                                "atom_id": aid,
                                "snapshot_id": snap_id,
                                "recorded_at": ts,
                            })),
                            branch: Some(branch.clone()),
                            settings: None,
                        })
                        .await;
                }
            }
            if published.get("commit").is_some_and(|c| !c.is_null()) {
                last_commit = serde_json::from_value(published["commit"].clone()).ok();
            }
        }

        let mut merged = false;
        let mut merge_outcome = Value::Null;
        if omni_opts["auto_merge"].as_bool().unwrap_or(false)
            && branch != "main"
            && let Ok(resp) = client
                .merge_branches(&BranchMergeRequest {
                    source: branch.clone(),
                    target: Some("main".to_string()),
                    delete_branch: false,
                    as_commit: false,
                    dry_run: false,
                    settings: None,
                })
                .await
        {
            merged = true;
            merge_outcome = json!(resp.outcome.as_str());
            if resp.commit.is_some() {
                last_commit = resp.commit;
            }
        }

        self.invalidate_stats_cache();
        Ok(json!({
            "ok": true,
            "branch": branch,
            "published_atom_ids": published_atom_ids,
            "symbol_count": symbol_count,
            "file_count": files.len(),
            "snapshot_id": snapshot_id,
            "merged": merged,
            "merge_outcome": merge_outcome,
            "commit": last_commit,
        }))
    }

    /// Poll incremental changes for live UI pulse (`GET /api/omnigraph/changes`).
    pub async fn changes_json(
        &self,
        _project_id: &str,
        cursor: Option<&str>,
        branch: Option<&str>,
    ) -> Result<Value> {
        let client = self.client().await?;
        let target_branch = clean_opt(branch).unwrap_or("main").to_string();
        let fresh_query = ChangeFeedQuery {
            branch: Some(target_branch.clone()),
            cursor: None,
            start: Some("beginning".to_string()),
            page_token: None,
            limit: Some(100),
        };
        let mut cursor_reset = false;
        let feed_res = if let Some(cur) = clean_opt(cursor) {
            let query = ChangeFeedQuery {
                branch: Some(target_branch.clone()),
                cursor: Some(cur.to_string()),
                start: None,
                page_token: None,
                limit: Some(100),
            };
            match client.changes(&query).await {
                Ok(feed) => Ok(feed),
                Err(err)
                    if err.is_change_feed_gap()
                        || err.status == 400
                        || err.code.as_deref() == Some("change_cursor_rejected") =>
                {
                    cursor_reset = true;
                    client.changes(&fresh_query).await
                }
                Err(err) => Err(err),
            }
        } else {
            client.changes(&fresh_query).await
        };
        let feed = match feed_res {
            Ok(feed) => feed,
            Err(err) if err.is_change_feed_gap() => {
                let baseline = client
                    .changes_baseline(&ChangeBaselineRequest {
                        branch: Some(target_branch.clone()),
                        kind: Vec::new(),
                        r#type: Vec::new(),
                        op: Vec::new(),
                    })
                    .await?;
                let (baseline_events, baseline_bundle) =
                    build_baseline_recovery_payload(&baseline, &target_branch);
                return Ok(json!({
                    "cursor": baseline.resume_cursor,
                    "caught_up": true,
                    "reset": true,
                    "snapshot_commit_id": baseline.snapshot_commit_id,
                    "events": baseline_events,
                    "baseline": baseline_bundle,
                }));
            }
            Err(err) => return Err(err.into()),
        };

        let mut events = Vec::new();
        for block in &feed.blocks {
            let authored_at = block.cause.authored_at;
            let created_at =
                format_omnigraph_ts(authored_at).unwrap_or_else(|| authored_at.to_string());
            for ch in &block.changes {
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
                let kind = if ch.kind == EntityKind::Node {
                    props["kind"]
                        .as_str()
                        .unwrap_or(ch.entity_type.name.as_str())
                        .to_string()
                } else {
                    "edge".to_string()
                };
                let node_id = if ch.kind == EntityKind::Node {
                    Value::String(ch.id.clone())
                } else {
                    Value::Null
                };
                let atom_id = if ch.kind == EntityKind::Node && ch.entity_type.name == "ContextAtom"
                {
                    Value::String(ch.id.clone())
                } else {
                    Value::Null
                };
                let session_id = if ch.kind == EntityKind::Node && ch.entity_type.name == "Session"
                {
                    Value::String(ch.id.clone())
                } else {
                    props
                        .get("source_session_id")
                        .or_else(|| props.get("session_id"))
                        .cloned()
                        .unwrap_or(Value::Null)
                };
                events.push(json!({
                    "graph_commit_id": block.cause.graph_commit_id,
                    "commit_id": block.cause.graph_commit_id,
                    "branch": block.cause.authored_branch,
                    "actor_id": block.cause.actor_id,
                    "authored_at": authored_at,
                    "timestamp": authored_at,
                    "created_at": created_at,
                    "entity_kind": ch.kind,
                    "type_name": ch.entity_type.name,
                    "entity_id": ch.id,
                    "kind": kind,
                    "node_id": node_id,
                    "atom_id": atom_id,
                    "session_id": session_id,
                    "op": ch.op.as_str(),
                    "title": title,
                    "summary": summary,
                    "properties": props,
                }));
            }
        }

        Ok(json!({
            "cursor": feed.cursor,
            "caught_up": feed.caught_up.unwrap_or(true),
            "reset": cursor_reset,
            "events": events,
        }))
    }

    /// Export a scoped task/branch/graph Omnigraph context bundle + raw NDJSON (`GET|POST /api/omnigraph/export`).
    pub async fn export_context_json(&self, params: &Value) -> Result<Value> {
        let cfg = self.config().await;
        let client = self.client().await?;
        let project_id = clean_opt(params["project_id"].as_str())
            .unwrap_or("")
            .to_string();
        let explicit_branch = clean_opt(params["branch"].as_str())
            .filter(|b| *b != "auto")
            .map(str::to_string);
        let session_id = clean_opt(params["session_id"].as_str()).map(str::to_string);

        let format = clean_opt(params["format"].as_str())
            .unwrap_or("bundle")
            .to_ascii_lowercase();
        if !matches!(format.as_str(), "bundle" | "ndjson") {
            return Err(HarnessError::invalid(format!(
                "Unsupported export format '{format}'; expected `bundle` or `ndjson`."
            )));
        }

        let default_scope = if session_id.is_some() {
            "task"
        } else if explicit_branch.is_some() {
            "branch"
        } else {
            "all"
        };
        let scope = clean_opt(params["scope"].as_str())
            .unwrap_or(default_scope)
            .to_ascii_lowercase();
        if !matches!(scope.as_str(), "task" | "branch" | "all") {
            return Err(HarnessError::invalid(format!(
                "Unsupported export scope '{scope}'; expected `task`, `branch`, or `all`."
            )));
        }

        let include_inherited =
            parse_bool_param(&params["include_inherited"]).unwrap_or(scope != "task");

        let mut source_branch = if scope == "all" && explicit_branch.is_none() {
            "main".to_string()
        } else if let Some(ref b) = explicit_branch {
            validate_branch_token(b)?
        } else if let Some(ref sid) = session_id {
            self.resolve_session_branch(sid)
        } else {
            "main".to_string()
        };

        let raw_ndjson = match client.export_branch(Some(&source_branch), &[], None).await {
            Ok(text) => text,
            Err(err)
                if err.is_not_found() && explicit_branch.is_none() && source_branch != "main" =>
            {
                source_branch = "main".to_string();
                client.export_branch(Some("main"), &[], None).await?
            }
            Err(err) => return Err(err.into()),
        };

        let parsed = parse_ndjson_records(&raw_ndjson)?;
        let filtered = filter_graph_for_export(
            &parsed,
            &project_id,
            &scope,
            session_id.as_deref(),
            &source_branch,
            include_inherited,
        );

        let mut files_by_atom: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for edge in &filtered.edges {
            if edge.edge_type == "TouchesFile" {
                files_by_atom
                    .entry(edge.from.clone())
                    .or_default()
                    .push(edge.to.clone());
            }
        }

        let atoms: Vec<Value> = filtered
            .atoms
            .values()
            .map(|row| {
                let mut atom = normalize_atom_row(row);
                let aid = str_field(&atom, "atom_id").unwrap_or_default();
                let atom_files = files_by_atom.get(&aid).cloned().unwrap_or_default();
                atom["files"] = json!(atom_files);
                atom
            })
            .collect();
        let files: Vec<Value> = filtered.files.values().map(normalize_file_row).collect();
        let symbols: Vec<Value> = filtered
            .symbols
            .values()
            .map(normalize_symbol_row)
            .collect();
        let sessions: Vec<Value> = filtered
            .sessions
            .values()
            .map(normalize_session_row)
            .collect();
        let turns: Vec<Value> = filtered.turns.values().map(normalize_turn_row).collect();
        let edges: Vec<Value> = filtered
            .edges
            .iter()
            .map(transfer::EdgeRecord::to_bundle_json)
            .collect();
        let ndjson = serialize_graph_ndjson(&filtered);

        let exported_at = now_utc_iso_ms();
        let label_slug = session_id
            .as_deref()
            .map(slugify_segment)
            .unwrap_or_else(|| slugify_segment(&source_branch));
        let stamp = exported_at
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(15)
            .collect::<String>();
        let ext = if format == "ndjson" { "ndjson" } else { "json" };
        let filename = format!("omnigraph-{label_slug}-{stamp}.{ext}");

        let bundle = json!({
            "format": BUNDLE_FORMAT_V1,
            "graph_id": cfg.graph_id,
            "project_id": project_id,
            "source_branch": source_branch,
            "source_session_id": session_id,
            "scope": scope,
            "include_inherited": include_inherited,
            "exported_at": exported_at,
            "atoms": atoms,
            "files": files,
            "symbols": symbols,
            "sessions": sessions,
            "turns": turns,
            "edges": edges,
            "ndjson": ndjson,
        });

        let counts = json!({
            "atoms": bundle["atoms"].as_array().map(Vec::len).unwrap_or(0),
            "files": bundle["files"].as_array().map(Vec::len).unwrap_or(0),
            "symbols": bundle["symbols"].as_array().map(Vec::len).unwrap_or(0),
            "sessions": bundle["sessions"].as_array().map(Vec::len).unwrap_or(0),
            "turns": bundle["turns"].as_array().map(Vec::len).unwrap_or(0),
            "edges": bundle["edges"].as_array().map(Vec::len).unwrap_or(0),
        });

        Ok(json!({
            "ok": true,
            "format": if format == "ndjson" { "ndjson" } else { BUNDLE_FORMAT_V1 },
            "requested_format": format,
            "filename": filename,
            "graph_id": cfg.graph_id,
            "project_id": project_id,
            "source_branch": source_branch,
            "source_session_id": session_id,
            "exported_at": exported_at,
            "counts": counts,
            "bundle": bundle,
            "ndjson": ndjson,
        }))
    }

    /// Export context as JSON metadata or downloadable attachment bytes (`download=1`).
    pub async fn export_context_payload(&self, query: &Value, body: &Value) -> Result<Payload> {
        let mut merged = Map::new();
        if let Some(qm) = query.as_object() {
            for (k, v) in qm {
                merged.insert(k.clone(), v.clone());
            }
        }
        if let Some(bm) = body.as_object() {
            for (k, v) in bm {
                merged.insert(k.clone(), v.clone());
            }
        }
        let params = Value::Object(merged);
        let download = parse_bool_param(&params["download"]).unwrap_or(false);
        let out = self.export_context_json(&params).await?;
        if !download {
            return Ok(Payload::Json(out));
        }

        let filename = out["filename"]
            .as_str()
            .unwrap_or("omnigraph-context.json")
            .to_string();
        if out["requested_format"] == "ndjson" {
            let text = out["ndjson"].as_str().unwrap_or("").to_string();
            Ok(Payload::Bytes {
                bytes: text.into_bytes(),
                content_type: "application/x-ndjson; charset=utf-8".into(),
                filename: Some(filename),
                etag: None,
                cache: Some("no-store".into()),
            })
        } else {
            let bytes = serde_json::to_vec_pretty(&out["bundle"])?;
            Ok(Payload::Bytes {
                bytes,
                content_type: "application/json; charset=utf-8".into(),
                filename: Some(filename),
                etag: None,
                cache: Some("no-store".into()),
            })
        }
    }

    /// Import an exported `harness.omnigraph.bundle.v1` JSON bundle or raw NDJSON onto an Omnigraph branch.
    pub async fn import_context_json(&self, body: &Value) -> Result<Value> {
        let client = self.client().await?;
        let project_id = clean_opt(body["project_id"].as_str())
            .unwrap_or("")
            .to_string();
        let mode = clean_opt(body["mode"].as_str())
            .unwrap_or("merge")
            .to_ascii_lowercase();
        if !matches!(mode.as_str(), "merge" | "replace" | "append") {
            return Err(HarnessError::invalid(format!(
                "Unsupported import mode '{mode}'; expected `merge` or `replace`."
            )));
        }

        let session_id = clean_opt(body["session_id"].as_str()).map(str::to_string);
        let target_branch =
            if let Some(b) = clean_opt(body["target_branch"].as_str()).filter(|b| *b != "auto") {
                validate_branch_token(b)?
            } else if let Some(ref sid) = session_id {
                self.resolve_session_branch(sid)
            } else {
                "main".to_string()
            };

        let explicit_from = clean_opt(body["from_branch"].as_str())
            .map(validate_branch_token)
            .transpose()?;
        let from_branch = explicit_from.clone().unwrap_or_else(|| "main".to_string());
        let create_branch_flag = body.get("create_branch").and_then(Value::as_bool);

        let parsed = parse_import_body(body)?;

        if target_branch != "main" && (create_branch_flag != Some(false) || explicit_from.is_some())
        {
            ensure_branch_exists(&client, &target_branch, Some(&from_branch)).await?;
        }

        let existing_on_branch = if mode != "replace" {
            client
                .export_branch(Some(&target_branch), &[], None)
                .await
                .ok()
                .and_then(|txt| parse_ndjson_records(&txt).ok())
        } else {
            None
        };

        let ts = now_utc_iso_ms();
        let (normalized, ndjson_payload) = prepare_import_ndjson(
            &parsed,
            &project_id,
            &target_branch,
            existing_on_branch.as_ref(),
            &ts,
        );

        let mut commit_id: Option<String> = None;
        if mode == "replace" {
            let clear_res = client
                .mutate(&ChangeRequest {
                    query: GQ_CLEAR_BRANCH_CONTEXT.into(),
                    name: Some("clear_branch_context".into()),
                    params: None,
                    branch: Some(target_branch.clone()),
                    settings: None,
                })
                .await?;
            if let Some(c) = clear_res.commit {
                commit_id = Some(c.graph_commit_id);
            }
            if !ndjson_payload.trim().is_empty() {
                let load_res = client
                    .load_ndjson(&target_branch, None, "overwrite", &ndjson_payload, None)
                    .await?;
                if let Some(cid) = load_res["commit"]["graph_commit_id"].as_str() {
                    commit_id = Some(cid.to_string());
                }
            }
        } else if !ndjson_payload.trim().is_empty() {
            let load_mode = if mode == "append" { "append" } else { "merge" };
            let load_res = client
                .load_ndjson(&target_branch, None, load_mode, &ndjson_payload, None)
                .await?;
            if let Some(cid) = load_res["commit"]["graph_commit_id"].as_str() {
                commit_id = Some(cid.to_string());
            }
        }

        let mut target_sids: Vec<String> = Vec::new();
        let explicit_targets = body["target_session_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        let inject_flag = body.get("inject_into_sessions").and_then(Value::as_bool);
        let should_inject = match inject_flag {
            Some(true) => true,
            Some(false) => false,
            None => !explicit_targets.is_empty(),
        };
        if should_inject {
            for sid in explicit_targets {
                if !target_sids.contains(&sid) {
                    target_sids.push(sid);
                }
            }
            if let Some(ref sid) = session_id
                && !target_sids.contains(sid)
            {
                target_sids.push(sid.clone());
            }
        }

        let atom_ids: Vec<String> = normalized.atoms.keys().cloned().collect();
        let mut shared_session_ids = Vec::new();
        for sid in &target_sids {
            let sess_meta = self.store.as_ref().and_then(|s| s.get(sid).ok());
            let sess_proj = sess_meta
                .as_ref()
                .and_then(|s| s["project_id"].as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(&project_id);
            let sess_branch = self.resolve_session_branch(sid);
            upsert_session_node(
                &client,
                sess_meta.as_ref(),
                sid,
                sess_proj,
                &target_branch,
                &ts,
            )
            .await?;

            for aid in &atom_ids {
                if let Ok(r) = client
                    .mutate(&ChangeRequest {
                        query: GQ_LINK_CONSUMED_BY.into(),
                        name: Some("link_consumed_by".into()),
                        params: Some(json!({
                            "atom_id": aid,
                            "session_id": sid,
                            "consumed_at": ts,
                            "injection_mode": "import",
                            "relevance": 1.0,
                        })),
                        branch: Some(target_branch.clone()),
                        settings: None,
                    })
                    .await
                    && let Some(c) = r.commit
                {
                    commit_id = Some(c.graph_commit_id);
                }
            }

            if sess_branch != target_branch {
                ensure_branch_exists(&client, &sess_branch, Some(&target_branch)).await?;
                if !ndjson_payload.trim().is_empty() {
                    let _ = client
                        .load_ndjson(&sess_branch, None, "merge", &ndjson_payload, None)
                        .await;
                }
                let _ = upsert_session_node(
                    &client,
                    sess_meta.as_ref(),
                    sid,
                    sess_proj,
                    &sess_branch,
                    &ts,
                )
                .await;
                for aid in &atom_ids {
                    let _ = client
                        .mutate(&ChangeRequest {
                            query: GQ_LINK_CONSUMED_BY.into(),
                            name: Some("link_consumed_by".into()),
                            params: Some(json!({
                                "atom_id": aid,
                                "session_id": sid,
                                "consumed_at": ts,
                                "injection_mode": "import",
                                "relevance": 1.0,
                            })),
                            branch: Some(sess_branch.clone()),
                            settings: None,
                        })
                        .await;
                }
            }
            shared_session_ids.push(sid.clone());
        }

        let mut files_by_atom: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for edge in &normalized.edges {
            if edge.edge_type == "TouchesFile" {
                let entry = files_by_atom.entry(edge.from.clone()).or_default();
                if !entry.contains(&edge.to) {
                    entry.push(edge.to.clone());
                }
            }
        }
        let atoms_out: Vec<Value> = normalized
            .atoms
            .values()
            .map(|row| {
                let mut atom = normalize_atom_row(row);
                let aid = str_field(&atom, "atom_id").unwrap_or_default();
                let files = files_by_atom.get(&aid).cloned().unwrap_or_default();
                atom["files"] = json!(files);
                atom
            })
            .collect();
        Ok(json!({
            "ok": true,
            "target_branch": target_branch,
            "mode": mode,
            "commit_id": commit_id,
            "imported_atoms": normalized.atoms.len(),
            "imported_files": normalized.files.len(),
            "imported_symbols": normalized.symbols.len(),
            "imported_sessions": normalized.sessions.len(),
            "imported_turns": normalized.turns.len(),
            "imported_edges": normalized.edges.len(),
            "atom_ids": atom_ids,
            "atoms": atoms_out,
            "shared_session_ids": shared_session_ids,
        }))
    }

    /// Link multiple Harness sessions in Omnigraph via mutual `shared_branch` or `broadcast` mode.
    pub async fn link_sessions_json(&self, body: &Value, store: &SessionStore) -> Result<Value> {
        let client = self.client().await?;
        let mode = clean_opt(body["mode"].as_str())
            .unwrap_or("shared_branch")
            .to_ascii_lowercase();
        if !matches!(mode.as_str(), "shared_branch" | "broadcast") {
            return Err(HarnessError::invalid(format!(
                "Unsupported session link mode '{mode}'; expected `shared_branch` or `broadcast`."
            )));
        }

        let requested_project_id = clean_opt(body["project_id"].as_str())
            .unwrap_or("")
            .to_string();
        let source_session_id = clean_opt(body["source_session_id"].as_str()).map(str::to_string);

        let mut selected_ids: Vec<String> = Vec::new();
        if let Some(arr) = body["session_ids"].as_array() {
            for item in arr {
                if let Some(s) = item.as_str().map(str::trim).filter(|s| !s.is_empty())
                    && !selected_ids.contains(&s.to_string())
                {
                    selected_ids.push(s.to_string());
                }
            }
        }

        let mut all_sids = Vec::new();
        if mode == "broadcast" {
            let mut target_ids = Vec::new();
            let src = source_session_id.clone().ok_or_else(|| {
                HarnessError::invalid("`source_session_id` is required in `broadcast` mode.")
            })?;
            all_sids.push(src.clone());
            for sid in selected_ids {
                if sid != src {
                    target_ids.push(sid.clone());
                    all_sids.push(sid);
                }
            }
            if target_ids.is_empty() {
                return Err(HarnessError::invalid(
                    "Select at least one target session distinct from `source_session_id`.",
                ));
            }
        } else {
            if let Some(ref src) = source_session_id {
                all_sids.push(src.clone());
            }
            for sid in selected_ids {
                if !all_sids.contains(&sid) {
                    all_sids.push(sid);
                }
            }
            if all_sids.len() < 2 {
                return Err(HarnessError::invalid(
                    "Select at least 2 sessions to share context.",
                ));
            }
        }

        let mut session_rows: Vec<(String, Value, String)> = Vec::with_capacity(all_sids.len());
        let mut resolved_project_id = requested_project_id.clone();
        for sid in &all_sids {
            let sess = store.get(sid)?;
            let sess_proj = sess["project_id"].as_str().unwrap_or("").to_string();
            if resolved_project_id.is_empty() {
                resolved_project_id = sess_proj.clone();
            } else if sess_proj != resolved_project_id {
                return Err(HarnessError::invalid(format!(
                    "Session `{sid}` does not belong to project `{resolved_project_id}`."
                )));
            }
            let sess_branch = clean_opt(sess["omnigraph"]["branch"].as_str())
                .filter(|b| *b != "auto")
                .map(str::to_string)
                .unwrap_or_else(|| session_branch_name(sid));
            session_rows.push((sid.clone(), sess, sess_branch));
        }

        let space_name_opt = clean_opt(body["space_name"].as_str()).map(str::to_string);
        let target_branch_opt = match body.get("target_branch") {
            None | Some(Value::Null) => None,
            Some(Value::String(raw)) => Some(normalize_link_target_branch(raw)?),
            Some(_) => return Err(HarnessError::invalid("Invalid Omnigraph branch name.")),
        };

        let (shared_branch, shared_space) = if mode == "broadcast" {
            let source_branch = session_rows[0].2.clone();
            let sb = target_branch_opt.unwrap_or(source_branch);
            let sp = space_name_opt.unwrap_or_else(|| sb.clone());
            (sb, sp)
        } else {
            let sb = if let Some(tb) = target_branch_opt {
                tb
            } else if let Some(ref sp) = space_name_opt {
                normalize_shared_space_branch(sp)
                    .map_err(|_| HarnessError::invalid("Invalid Omnigraph `space_name`."))?
            } else {
                let mut sorted = all_sids.clone();
                sorted.sort();
                let digest = store::sha(sorted.join(",").as_bytes());
                format!("shared/{}", &digest[..10])
            };
            let sp = space_name_opt.unwrap_or_else(|| sb.clone());
            (sb, sp)
        };

        let ts = now_utc_iso_ms();
        for (sid, sess, sess_branch) in &session_rows {
            ensure_branch_exists(&client, sess_branch, Some("main")).await?;
            upsert_session_node(
                &client,
                Some(sess),
                sid,
                &resolved_project_id,
                sess_branch,
                &ts,
            )
            .await?;
        }
        ensure_branch_exists(&client, &shared_branch, Some("main")).await?;
        for (sid, sess, _) in &session_rows {
            upsert_session_node(
                &client,
                Some(sess),
                sid,
                &resolved_project_id,
                &shared_branch,
                &ts,
            )
            .await?;
        }

        let mut last_commit: Option<String> = None;
        let mut conflicts: Vec<Value> = Vec::new();

        if mode == "shared_branch" {
            for (_, _, sess_branch) in &session_rows {
                if sess_branch != &shared_branch {
                    let (commit, mut cfl) = merge_or_sync_branch(
                        &client,
                        sess_branch,
                        &shared_branch,
                        &resolved_project_id,
                    )
                    .await?;
                    if let Some(c) = commit {
                        last_commit = Some(c.graph_commit_id);
                    }
                    conflicts.append(&mut cfl);
                }
            }

            let shared_atoms =
                fetch_branch_atoms(&client, &resolved_project_id, &shared_branch).await?;
            for i in 0..all_sids.len() {
                for j in 0..all_sids.len() {
                    if i != j
                        && let Ok(r) = link_session_follows(
                            &client,
                            &all_sids[i],
                            &all_sids[j],
                            "shared_branch",
                            &shared_branch,
                            &ts,
                        )
                        .await
                        && let Some(c) = r.commit
                    {
                        last_commit = Some(c.graph_commit_id);
                    }
                }
            }
            for atom in &shared_atoms {
                if let Some(aid) = str_field(atom, "atom_id") {
                    for sid in &all_sids {
                        if let Ok(r) = link_consumed_by(
                            &client,
                            &aid,
                            sid,
                            "shared_branch",
                            &shared_branch,
                            &ts,
                        )
                        .await
                            && let Some(c) = r.commit
                        {
                            last_commit = Some(c.graph_commit_id);
                        }
                    }
                }
            }

            for (_, _, sess_branch) in &session_rows {
                if sess_branch != &shared_branch {
                    let (commit, mut cfl) = merge_or_sync_branch(
                        &client,
                        &shared_branch,
                        sess_branch,
                        &resolved_project_id,
                    )
                    .await?;
                    if let Some(c) = commit {
                        last_commit = Some(c.graph_commit_id);
                    }
                    conflicts.append(&mut cfl);
                }
            }

            let atom_ids: Vec<String> = shared_atoms
                .iter()
                .filter_map(|a| str_field(a, "atom_id"))
                .collect();
            return Ok(json!({
                "ok": true,
                "mode": mode,
                "shared_branch": shared_branch,
                "shared_space": shared_space,
                "source_session_id": Value::Null,
                "commit_id": last_commit,
                "synced_atoms": shared_atoms.len(),
                "atom_ids": atom_ids,
                "atoms": shared_atoms,
                "linked_session_ids": all_sids,
                "conflicts": conflicts,
            }));
        }

        // Broadcast mode: push `source_session_id`'s branch/atoms into all target sessions
        let (source_sid, _, source_branch) = &session_rows[0];
        if &shared_branch != source_branch {
            let (commit, mut cfl) =
                merge_or_sync_branch(&client, source_branch, &shared_branch, &resolved_project_id)
                    .await?;
            if let Some(c) = commit {
                last_commit = Some(c.graph_commit_id);
            }
            conflicts.append(&mut cfl);
        }
        let source_atoms = fetch_branch_atoms(&client, &resolved_project_id, source_branch).await?;
        for (target_sid, _, target_branch) in session_rows.iter().skip(1) {
            let (commit, mut cfl) =
                merge_or_sync_branch(&client, source_branch, target_branch, &resolved_project_id)
                    .await?;
            if let Some(c) = commit {
                last_commit = Some(c.graph_commit_id);
            }
            conflicts.append(&mut cfl);

            for b in [target_branch.as_str(), shared_branch.as_str()] {
                let _ = link_session_follows(&client, target_sid, source_sid, "broadcast", b, &ts)
                    .await;
            }
            for atom in &source_atoms {
                if let Some(aid) = str_field(atom, "atom_id")
                    && let Ok(r) =
                        link_consumed_by(&client, &aid, target_sid, "broadcast", target_branch, &ts)
                            .await
                    && let Some(c) = r.commit
                {
                    last_commit = Some(c.graph_commit_id);
                }
            }
        }

        let atom_ids: Vec<String> = source_atoms
            .iter()
            .filter_map(|a| str_field(a, "atom_id"))
            .collect();
        Ok(json!({
            "ok": true,
            "mode": mode,
            "shared_branch": shared_branch,
            "shared_space": shared_space,
            "source_session_id": source_sid,
            "commit_id": last_commit,
            "synced_atoms": source_atoms.len(),
            "atom_ids": atom_ids,
            "atoms": source_atoms,
            "linked_session_ids": all_sids,
            "conflicts": conflicts,
        }))
    }

    /// Best-effort helper to create `Session` nodes and `SessionFollows` edges when a new session is
    /// created with `omnigraph.linked_session_ids`.
    pub async fn sync_new_session_links(
        &self,
        store: &SessionStore,
        session: &Value,
    ) -> Result<()> {
        let cfg = self.config().await;
        if cfg.mode == OmnigraphMode::Off {
            return Ok(());
        }
        let peers = session["omnigraph"]["linked_session_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if peers.is_empty() {
            return Ok(());
        }
        let Ok(client) = self.client().await else {
            return Ok(());
        };
        let sid = session["id"].as_str().unwrap_or("");
        if sid.is_empty() {
            return Ok(());
        }
        let project_id = session["project_id"].as_str().unwrap_or("");
        let branch = self.resolve_session_branch(sid);
        let ts = now_utc_iso_ms();
        ensure_branch_exists(&client, &branch, Some("main")).await?;
        upsert_session_node(&client, Some(session), sid, project_id, &branch, &ts).await?;
        let rel = canonical_session_follows_relation(&session["omnigraph"]);
        for peer_id in &peers {
            if peer_id == sid {
                continue;
            }
            let peer_meta = store.get(peer_id).ok();
            let _ = upsert_session_node(
                &client,
                peer_meta.as_ref(),
                peer_id,
                project_id,
                &branch,
                &ts,
            )
            .await;
            let _ = link_session_follows(&client, sid, peer_id, rel, &branch, &ts).await;
            let _ = link_session_follows(&client, peer_id, sid, rel, &branch, &ts).await;
            if let Some(ref pm) = peer_meta {
                let peer_branch = clean_opt(pm["omnigraph"]["branch"].as_str())
                    .filter(|b| *b != "auto")
                    .map(str::to_string)
                    .unwrap_or_else(|| session_branch_name(peer_id));
                if ensure_branch_exists(&client, &peer_branch, Some("main"))
                    .await
                    .is_ok()
                {
                    let _ = upsert_session_node(
                        &client,
                        Some(session),
                        sid,
                        project_id,
                        &peer_branch,
                        &ts,
                    )
                    .await;
                    let _ = upsert_session_node(
                        &client,
                        Some(pm),
                        peer_id,
                        project_id,
                        &peer_branch,
                        &ts,
                    )
                    .await;
                    let _ =
                        link_session_follows(&client, sid, peer_id, rel, &peer_branch, &ts).await;
                    let _ =
                        link_session_follows(&client, peer_id, sid, rel, &peer_branch, &ts).await;
                }
            }
        }
        Ok(())
    }

    /// Sync canonical `Project Brain` and `Memory Bank` knowledge items onto `CANONICAL_KNOWLEDGE_BRANCH`.
    pub async fn sync_canonical_knowledge(
        &self,
        project_id: &str,
        items: &[Value],
    ) -> Result<Value> {
        self.sync_canonical_knowledge_with_options(project_id, items, &[])
            .await
    }

    /// `sync_canonical_knowledge` that also deletes, for each of
    /// `prune_layers` (`project-brain`, `memory-bank`), the project's synced
    /// atoms of that layer that `items` no longer carries: a retired,
    /// archived, superseded or stale record or chunk leaves the branch. Pass
    /// only a layer read whole, with every page of it in `items`.
    pub async fn sync_canonical_knowledge_with_options(
        &self,
        project_id: &str,
        items: &[Value],
        prune_layers: &[String],
    ) -> Result<Value> {
        let cfg = self.config().await;
        if cfg.mode == OmnigraphMode::Off {
            return Ok(json!({
                "ok": false,
                "skipped": "omnigraph-off",
                "synced": 0,
                "atom_ids": [],
            }));
        }
        let client = self.client().await?;
        let branch = CANONICAL_KNOWLEDGE_BRANCH;
        ensure_branch_exists(&client, branch, Some("main")).await?;

        let endpoint_key = cfg.endpoint.as_deref().unwrap_or("").trim().to_owned();
        let graph_key = cfg.graph_id.clone();
        let ledger_db = open_canonical_sync_ledger(&self.state_dir);
        let ledger_rows = ledger_db
            .as_ref()
            .map(|db| load_canonical_sync_ledger(db, &endpoint_key, &graph_key, branch, project_id))
            .unwrap_or_default();

        let mut existing_ids = collect_branch_atom_ids(&client, project_id, branch).await;
        // A record or chunk names declarations of the project's own files;
        // what each synced atom named before is replaced only when it changed.
        let root = self.project_root(project_id, None);
        let mut previous_symbols = list_atom_symbols(&client, project_id, branch).await;
        let mut previous_files = list_atom_files_map(&client, project_id, branch).await;
        let mut symbol_count = 0usize;
        let mut skipped_atoms = 0usize;
        let mut skipped_files = 0usize;
        let mut mutated_atoms = 0usize;
        let ts = now_utc_iso_ms();
        let mut atom_ids = Vec::new();
        let mut pending_supersedes: Vec<(String, Vec<String>)> = Vec::new();

        for item in items {
            let Some(atom_id) = str_field(item, "atom_id").or_else(|| str_field(item, "id")) else {
                continue;
            };
            let raw_title = str_field(item, "title")
                .or_else(|| str_field(item, "summary"))
                .unwrap_or_else(|| atom_id.clone());
            let title: String = raw_title.chars().take(240).collect();
            let raw_kind = str_field(item, "kind")
                .unwrap_or_else(|| "finding".to_string())
                .to_ascii_lowercase();
            let kind = if VALID_ATOM_KINDS.contains(&raw_kind.as_str()) {
                raw_kind
            } else {
                "insight".to_string()
            };
            let summary = str_field(item, "summary").unwrap_or_else(|| title.clone());
            let body = str_field(item, "body").unwrap_or_else(|| summary.clone());
            let tags = parse_tags_opt(&item["tags"]);
            let files = parse_string_list(&item["files"]);
            let confidence = item["confidence"].as_f64().unwrap_or(0.9).clamp(0.0, 1.0);
            let pin_priority = item["pin_priority"].as_i64().unwrap_or(20) as i32;
            let prov = AtomProvenance::from_value(item);
            let verified = item
                .get("verified")
                .and_then(Value::as_bool)
                .unwrap_or_else(|| prov.authority.as_deref() == Some("verified"));
            let effective_project_id = clean_opt(Some(project_id))
                .or_else(|| clean_opt(item["project_id"].as_str()))
                .unwrap_or("");
            let source_session_id =
                str_field(item, "source_session_id").or_else(|| str_field(item, "session_id"));
            let supersedes = parse_string_list(&item["supersedes_atom_ids"]);

            let atom_sha256 = canonical_atom_digest(
                &atom_id,
                effective_project_id,
                &kind,
                &title,
                &summary,
                &body,
                tags.clone(),
                branch,
                source_session_id.as_deref(),
                confidence,
                pin_priority,
                verified,
                &prov,
            );
            let files_sha256 = canonical_sorted_list_digest(&files);
            let supersedes_sha256 = canonical_sorted_list_digest(&supersedes);

            let prev_ledger = ledger_rows.get(&atom_id);
            let on_branch = existing_ids.contains(&atom_id);
            let atom_unchanged =
                on_branch && prev_ledger.is_some_and(|r| r.atom_sha256 == atom_sha256);

            if atom_unchanged {
                skipped_atoms += 1;
            } else {
                let atom_params = build_upsert_atom_params(
                    &atom_id,
                    effective_project_id,
                    &kind,
                    &title,
                    &summary,
                    &body,
                    tags,
                    branch,
                    source_session_id.as_deref(),
                    confidence,
                    pin_priority,
                    verified,
                    &prov,
                    &ts,
                    &ts,
                );
                client
                    .mutate(&ChangeRequest {
                        query: GQ_UPSERT_ATOM.into(),
                        name: Some("upsert_atom".into()),
                        params: Some(atom_params),
                        branch: Some(branch.to_string()),
                        settings: None,
                    })
                    .await?;
                mutated_atoms += 1;
            }

            let prev_files_set = previous_files.remove(&atom_id).unwrap_or_default();
            let wanted_files_set: BTreeSet<String> = files.iter().cloned().collect();
            let files_unchanged = (files.is_empty() && prev_files_set.is_empty())
                || (on_branch
                    && wanted_files_set == prev_files_set
                    && prev_ledger.is_some_and(|r| r.files_sha256 == files_sha256));
            if files_unchanged {
                skipped_files += 1;
            } else if !files.is_empty() {
                let target_files: Vec<String> = if atom_unchanged && on_branch {
                    files
                        .iter()
                        .filter(|f| !prev_files_set.contains(f.as_str()))
                        .cloned()
                        .collect()
                } else {
                    files.clone()
                };
                if !target_files.is_empty() {
                    upsert_atom_files(
                        &client,
                        effective_project_id,
                        branch,
                        &atom_id,
                        &target_files,
                        &ts,
                    )
                    .await;
                }
            }

            let symbols = atom_symbols_for_project(
                Some(&self.state_dir),
                effective_project_id,
                root.as_deref(),
                &files,
                &format!("{title}\n{summary}\n{body}"),
            );
            symbol_count += symbols.len();
            upsert_atom_symbols(
                &client,
                effective_project_id,
                branch,
                &atom_id,
                &symbols,
                &previous_symbols.remove(&atom_id).unwrap_or_default(),
            )
            .await;
            existing_ids.insert(atom_id.clone());
            if !supersedes.is_empty()
                && (!atom_unchanged
                    || prev_ledger.is_none_or(|r| r.supersedes_sha256 != supersedes_sha256))
            {
                pending_supersedes.push((atom_id.clone(), supersedes));
            }
            if let Some(ref db) = ledger_db {
                save_canonical_sync_ledger(
                    db,
                    &endpoint_key,
                    &graph_key,
                    branch,
                    project_id,
                    &atom_id,
                    &CanonicalSyncLedgerEntry {
                        atom_sha256,
                        files_sha256,
                        supersedes_sha256,
                    },
                    &ts,
                );
            }
            atom_ids.push(atom_id);
        }

        for (source_id, targets) in pending_supersedes {
            for target_id in targets {
                if target_id != source_id && existing_ids.contains(&target_id) {
                    let _ = client
                        .mutate(&ChangeRequest {
                            query: GQ_LINK_RELATES_TO.into(),
                            name: Some("link_relates_to".into()),
                            params: Some(json!({
                                "source_id": source_id,
                                "target_id": target_id,
                                "relation": "supersedes",
                                "note": "Superseded in canonical knowledge sync",
                                "confidence": 1.0,
                                "created_at": ts,
                            })),
                            branch: Some(branch.to_string()),
                            settings: None,
                        })
                        .await;
                }
            }
        }

        let mut deleted_atom_ids = Vec::new();
        let mut prune_error = None;
        if !prune_layers.is_empty() {
            let current = atom_ids.iter().cloned().collect::<BTreeSet<_>>();
            match collect_branch_atom_rows(&client, project_id, branch).await {
                Ok(rows) => {
                    for row in rows {
                        let Some(atom_id) = str_field(&row, "atom_id") else {
                            continue;
                        };
                        if current.contains(&atom_id)
                            || !synced_atom_of(&row, &atom_id, project_id, prune_layers)
                        {
                            continue;
                        }
                        match client
                            .mutate(&ChangeRequest {
                                query: GQ_DELETE_CANONICAL_ATOM.into(),
                                name: Some("delete_canonical_atom".into()),
                                params: Some(json!({ "atom_id": atom_id })),
                                branch: Some(branch.to_string()),
                                settings: None,
                            })
                            .await
                        {
                            Ok(_) => {
                                if let Some(ref db) = ledger_db {
                                    delete_canonical_sync_ledger(
                                        db,
                                        &endpoint_key,
                                        &graph_key,
                                        branch,
                                        project_id,
                                        &atom_id,
                                    );
                                }
                                deleted_atom_ids.push(atom_id);
                            }
                            Err(error) => {
                                prune_error = Some(error.to_string());
                                break;
                            }
                        }
                    }
                }
                Err(error) => prune_error = Some(error.to_string()),
            }
        }

        let mut synced = json!({
            "ok": true,
            "branch": branch,
            "synced": atom_ids.len(),
            "mutated_atoms": mutated_atoms,
            "skipped_atoms": skipped_atoms,
            "skipped_files": skipped_files,
            "symbols": symbol_count,
            "atom_ids": atom_ids,
            "deleted": deleted_atom_ids.len(),
            "deleted_atom_ids": deleted_atom_ids,
        });
        if let Some(error) = prune_error {
            synced["prune_error"] = json!(error);
        }
        Ok(synced)
    }

    /// The context atoms that touch one file, on `main` and on the canonical
    /// knowledge branch, each once: what the shared graph knows about it,
    /// the declarations of the file each names (`symbols`), and the
    /// `contradicts` relations with one of them at either end.
    pub async fn file_context_json(&self, project_id: &str, path: &str) -> Result<Value> {
        let path = path.trim().trim_start_matches("./");
        if path.is_empty()
            || path.starts_with('/')
            || path.contains('\\')
            || path.split('/').any(|part| part == "..")
        {
            return Err(HarnessError::invalid("Choose a file of the project."));
        }
        let cfg = self.config().await;
        if cfg.mode == OmnigraphMode::Off {
            return Ok(json!({"ok": false, "skipped": "omnigraph-off", "atoms": []}));
        }
        let client = self.client().await?;
        let branches = ["main".to_string(), CANONICAL_KNOWLEDGE_BRANCH.to_string()];
        let mut atoms = Vec::new();
        let mut seen = BTreeSet::new();
        for branch in &branches {
            let Ok(found) = run_stored_or_inline(
                &client,
                "file_context",
                schema::QUERY_FILE_CONTEXT,
                json!({ "path": path }),
                Some(branch.clone()),
                None,
            )
            .await
            else {
                continue;
            };
            if found.rows.is_empty() {
                continue;
            }
            let mut named: BTreeMap<String, Vec<String>> = BTreeMap::new();
            let prefix = format!("{path}::");
            for edge in symbol_edges(&client, project_id, branch).await {
                if edge["file_path"].as_str() == Some(path)
                    && let (Some(aid), Some(fqn)) =
                        (str_field(&edge, "source_id"), str_field(&edge, "target_id"))
                {
                    let label = fqn.strip_prefix(&prefix).unwrap_or(&fqn).to_string();
                    let labels = named.entry(aid).or_default();
                    if !labels.contains(&label) {
                        labels.push(label);
                    }
                }
            }
            for row in &found.rows {
                let mut atom = normalize_atom_row(row);
                if let Some(aid) = str_field(&atom, "atom_id")
                    && seen.insert(aid.clone())
                {
                    if atom["source_branch"].is_null() {
                        atom["source_branch"] = json!(branch);
                    }
                    atom["capsule_line"] = json!(format_atom_line_with_authority(&atom).trim_end());
                    atom["symbols"] = json!(named.remove(&aid).unwrap_or_default());
                    atoms.push(atom);
                }
            }
        }
        let contradictions = contradictions_touching(&client, project_id, &branches, &seen).await;
        Ok(json!({
            "ok": true,
            "path": path,
            "atoms": atoms,
            "contradictions": contradictions,
        }))
    }

    /// Locate `atom_id` on `branch` (or across `"main"`, `CANONICAL_KNOWLEDGE_BRANCH`, and listed branches)
    /// and attach its `TouchesFile` paths from `GQ_LIST_FILE_EDGES`.
    pub async fn fetch_atom_with_files(
        &self,
        project_id: &str,
        atom_id: &str,
        branch: Option<&str>,
    ) -> Result<Value> {
        let atom_id = atom_id.trim();
        if atom_id.is_empty() {
            return Err(HarnessError::invalid("`atom_id` is required."));
        }
        let client = self.client().await?;
        let mut candidate_branches = Vec::new();
        if let Some(b) = clean_opt(branch).filter(|b| *b != "auto") {
            candidate_branches.push(validate_branch_token(b)?);
        }
        for default_branch in ["main", CANONICAL_KNOWLEDGE_BRANCH] {
            if !candidate_branches.iter().any(|b| b == default_branch) {
                candidate_branches.push(default_branch.to_string());
            }
        }
        if let Ok(list) = client.list_branches(false).await {
            for b in list.branches {
                if !candidate_branches.contains(&b) {
                    candidate_branches.push(b);
                }
            }
        }

        for resolved_branch in candidate_branches {
            let Some(mut atom) =
                find_atom_on_branch(&client, project_id, &resolved_branch, atom_id).await
            else {
                continue;
            };
            let edge_pid = str_field(&atom, "project_id").unwrap_or_else(|| project_id.to_string());
            let files = list_atom_files(&client, &edge_pid, &resolved_branch, atom_id).await;
            atom["files"] = json!(files);
            return Ok(json!({
                "atom": atom,
                "branch": resolved_branch,
                "files": files,
            }));
        }

        Err(HarnessError::missing(format!(
            "Omnigraph ContextAtom '{atom_id}' was not found."
        )))
    }

    /// Re-upsert `atom` on `branch` preserving content, tags, and files while linking it to a
    /// verified `Project Brain` record (`origin_layer: "project-brain"`, `authority: "verified"`, `verified: true`).
    pub async fn link_atom_to_brain(
        &self,
        project_id: &str,
        branch: &str,
        atom: &Value,
        brain_record_id: &str,
        revision: i64,
        privacy: Option<&str>,
    ) -> Result<Value> {
        let atom_id = str_field(atom, "atom_id")
            .or_else(|| str_field(atom, "id"))
            .ok_or_else(|| HarnessError::invalid("`atom` is missing `atom_id`."))?;
        let branch = validate_branch_token(branch)?;
        let brain_record_id = clean_opt(Some(brain_record_id))
            .ok_or_else(|| HarnessError::invalid("`brain_record_id` is required."))?;
        let effective_project_id = clean_opt(Some(project_id))
            .or_else(|| clean_opt(atom["project_id"].as_str()))
            .unwrap_or("");
        let raw_kind = str_field(atom, "kind")
            .unwrap_or_else(|| "insight".to_string())
            .to_ascii_lowercase();
        let kind = if VALID_ATOM_KINDS.contains(&raw_kind.as_str()) {
            raw_kind
        } else {
            "insight".to_string()
        };
        let title = str_field(atom, "title").unwrap_or_else(|| atom_id.clone());
        let summary = str_field(atom, "summary").unwrap_or_else(|| title.clone());
        let body = str_field(atom, "body").unwrap_or_else(|| summary.clone());
        let tags = parse_tags_opt(&atom["tags"]);
        let source_branch = str_field(atom, "source_branch")
            .or_else(|| str_field(atom, "branch"))
            .unwrap_or_else(|| branch.clone());
        let source_session_id =
            str_field(atom, "source_session_id").or_else(|| str_field(atom, "session_id"));
        let confidence = atom["confidence"].as_f64().unwrap_or(0.9).clamp(0.0, 1.0);
        let pin_priority = atom["pin_priority"].as_i64().unwrap_or(15) as i32;
        let created_at = str_field(atom, "created_at").unwrap_or_else(now_utc_iso_ms);
        let updated_at = now_utc_iso_ms();

        let client = self.client().await?;
        ensure_branch_exists(&client, &branch, Some("main")).await?;

        let mut files = parse_string_list(&atom["files"]);
        if files.is_empty() {
            files = list_atom_files(&client, effective_project_id, &branch, &atom_id).await;
        }

        let mut prov = AtomProvenance::from_value(atom);
        prov.origin_layer = Some("project-brain".to_string());
        prov.authority = Some("verified".to_string());
        prov.brain_record_id = Some(brain_record_id.to_string());
        prov.record_revision = Some(revision);
        if let Some(p) = clean_opt(privacy) {
            prov.privacy = Some(p.to_string());
        }

        let atom_params = build_upsert_atom_params(
            &atom_id,
            effective_project_id,
            &kind,
            &title,
            &summary,
            &body,
            tags,
            &source_branch,
            source_session_id.as_deref(),
            confidence,
            pin_priority,
            true,
            &prov,
            &created_at,
            &updated_at,
        );
        let mut last_commit = client
            .mutate(&ChangeRequest {
                query: GQ_UPSERT_ATOM.into(),
                name: Some("upsert_atom".into()),
                params: Some(atom_params.clone()),
                branch: Some(branch.clone()),
                settings: None,
            })
            .await?
            .commit;

        if let Some(commit) = upsert_atom_files(
            &client,
            effective_project_id,
            &branch,
            &atom_id,
            &files,
            &updated_at,
        )
        .await
        {
            last_commit = Some(commit);
        }

        let mut atom_resp = normalize_atom_row(&atom_params);
        atom_resp["files"] = json!(files);
        Ok(json!({
            "ok": true,
            "atom_id": atom_id,
            "branch": branch,
            "brain_record_id": brain_record_id,
            "record_revision": revision,
            "origin_layer": "project-brain",
            "authority": "verified",
            "privacy": prov.privacy,
            "verified": true,
            "atom": atom_resp,
            "files": files,
            "commit": last_commit,
        }))
    }

    fn resolve_session_branch(&self, sid: &str) -> String {
        self.store
            .as_ref()
            .and_then(|s| s.get(sid).ok())
            .and_then(|sess| {
                clean_opt(sess["omnigraph"]["branch"].as_str())
                    .filter(|b| *b != "auto")
                    .map(str::to_string)
            })
            .unwrap_or_else(|| session_branch_name(sid))
    }
}

pub(super) async fn run_stored_or_inline(
    client: &OmnigraphClient,
    stored_name: &str,
    inline_gq: &str,
    params: Value,
    branch: Option<String>,
    snapshot: Option<String>,
) -> Result<crate::omnigraph::client::QueryResponse> {
    match client
        .stored_query(
            stored_name,
            &StoredQueryRequest {
                params: Some(params.clone()),
                branch: branch.clone(),
                snapshot: snapshot.clone(),
                expect_mutation: Some(false),
            },
        )
        .await
    {
        Ok(resp) => Ok(resp),
        Err(err) if err.is_not_found() => client
            .query(&QueryRequest {
                query: inline_gq.to_string(),
                name: Some(stored_name.to_string()),
                params: Some(params),
                branch,
                snapshot,
                settings: None,
            })
            .await
            .map_err(Into::into),
        Err(err) => Err(err.into()),
    }
}

/// Session text the shared graph may hold: screened as an automatic memory
/// query (secrets, personal data and transcript markers removed), or none.
pub(super) fn graph_session_text(raw: &str) -> Option<String> {
    crate::brain_settings::query(raw)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// `SessionFollows.relation` says how two sessions are linked: sessions that
/// share a space are on a shared branch, others are linked. The space's own
/// name stays on the session, never on the edge.
pub(crate) fn canonical_session_follows_relation(omnigraph: &Value) -> &'static str {
    if clean_opt(omnigraph["shared_space"].as_str()).is_some() {
        "shared_branch"
    } else {
        "linked"
    }
}

/// The `turn_index` after the session's latest snapshot on `branch`, or 1.
pub(super) async fn next_turn_index(client: &OmnigraphClient, sid: &str, branch: &str) -> i32 {
    query_edges_safe(
        client,
        GQ_LIST_SESSION_TURN_SNAPSHOTS,
        "list_session_turn_snapshots",
        &json!({ "session_id": sid }),
        Some(branch),
        None,
    )
    .await
    .iter()
    .filter_map(|row| row["turn_index"].as_i64())
    .max()
    .map_or(1, |latest| {
        i32::try_from(latest.saturating_add(1)).unwrap_or(i32::MAX)
    })
}

pub(super) async fn query_edges_safe(
    client: &OmnigraphClient,
    query: &str,
    name: &str,
    params: &Value,
    branch: Option<&str>,
    snapshot: Option<&str>,
) -> Vec<Value> {
    client
        .query(&QueryRequest {
            query: query.to_string(),
            name: Some(name.to_string()),
            params: Some(params.clone()),
            branch: branch.map(str::to_string),
            snapshot: snapshot.map(str::to_string),
            settings: None,
        })
        .await
        .map(|r| r.rows)
        .unwrap_or_default()
}

pub(super) async fn upsert_session_node(
    client: &OmnigraphClient,
    session_meta: Option<&Value>,
    sid: &str,
    project_id: &str,
    branch: &str,
    ts: &str,
) -> Result<crate::omnigraph::client::ChangeResponse> {
    let sess_project_id = session_meta
        .and_then(|s| s["project_id"].as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(project_id);
    let sess_title = session_meta
        .and_then(|s| s["title"].as_str())
        .unwrap_or("Harness Session");
    let sess_provider = session_meta
        .and_then(|s| s["provider"].as_str())
        .unwrap_or("codex");
    let sess_workflow = session_meta
        .and_then(|s| s["workflow"].as_str())
        .unwrap_or("native");
    let sess_status = session_meta
        .and_then(|s| s["status"].as_str())
        .unwrap_or("running");
    let sess_branch = session_meta
        .and_then(|s| clean_opt(s["omnigraph"]["branch"].as_str()))
        .filter(|b| *b != "auto")
        .map(str::to_string)
        .unwrap_or_else(|| session_branch_name(sid));
    let goal = session_meta.and_then(|s| {
        [s["omnigraph"]["query"].as_str(), s["goal"].as_str()]
            .into_iter()
            .flatten()
            .find_map(graph_session_text)
    });
    client
        .mutate(&ChangeRequest {
            query: GQ_UPSERT_SESSION.into(),
            name: Some("upsert_session".into()),
            params: Some(json!({
                "session_id": sid,
                "project_id": sess_project_id,
                "title": sess_title,
                "branch": sess_branch,
                "provider": sess_provider,
                "workflow": sess_workflow,
                "status": sess_status,
                "goal": goal,
                "created_at": ts,
                "updated_at": ts,
            })),
            branch: Some(branch.to_string()),
            settings: None,
        })
        .await
        .map_err(Into::into)
}

pub(super) async fn link_session_follows(
    client: &OmnigraphClient,
    from_sid: &str,
    to_sid: &str,
    relation: &str,
    branch: &str,
    ts: &str,
) -> Result<crate::omnigraph::client::ChangeResponse> {
    client
        .mutate(&ChangeRequest {
            query: GQ_LINK_SESSION_FOLLOWS.into(),
            name: Some("link_session_follows".into()),
            params: Some(json!({
                "from_session_id": from_sid,
                "to_session_id": to_sid,
                "relation": relation,
                "created_at": ts,
            })),
            branch: Some(branch.to_string()),
            settings: None,
        })
        .await
        .map_err(Into::into)
}

pub(super) async fn link_consumed_by(
    client: &OmnigraphClient,
    atom_id: &str,
    session_id: &str,
    injection_mode: &str,
    branch: &str,
    ts: &str,
) -> Result<crate::omnigraph::client::ChangeResponse> {
    client
        .mutate(&ChangeRequest {
            query: GQ_LINK_CONSUMED_BY.into(),
            name: Some("link_consumed_by".into()),
            params: Some(json!({
                "atom_id": atom_id,
                "session_id": session_id,
                "consumed_at": ts,
                "injection_mode": injection_mode,
                "relevance": 1.0,
            })),
            branch: Some(branch.to_string()),
            settings: None,
        })
        .await
        .map_err(Into::into)
}

pub(super) async fn fetch_branch_atoms(
    client: &OmnigraphClient,
    project_id: &str,
    branch: &str,
) -> Result<Vec<Value>> {
    let resp = run_stored_or_inline(
        client,
        "recent_atoms",
        schema::QUERY_RECENT_ATOMS,
        json!({ "project_id": project_id }),
        Some(branch.to_string()),
        None,
    )
    .await?;
    Ok(resp.rows.iter().map(normalize_atom_row).collect())
}

pub(super) async fn merge_or_sync_branch(
    client: &OmnigraphClient,
    source_branch: &str,
    target_branch: &str,
    project_id: &str,
) -> Result<(Option<CommitOutput>, Vec<Value>)> {
    if source_branch == target_branch {
        return Ok((None, Vec::new()));
    }
    match client
        .merge_branches(&BranchMergeRequest {
            source: source_branch.to_string(),
            target: Some(target_branch.to_string()),
            delete_branch: false,
            as_commit: false,
            dry_run: false,
            settings: None,
        })
        .await
    {
        Ok(resp) => Ok((resp.commit, Vec::new())),
        Err(err) if err.is_not_found() => Ok((None, Vec::new())),
        Err(err) if err.is_merge_conflict() => {
            let mut conflicts = Vec::new();
            let mut excluded_atoms = BTreeSet::new();
            for mc in &err.merge_conflicts {
                if mc.type_name == "ContextAtom" {
                    if let Some(eid) = mc.entity_id.as_ref().filter(|s| !s.is_empty()) {
                        excluded_atoms.insert(eid.clone());
                    }
                    if let Ok(v) = serde_json::to_value(mc) {
                        conflicts.push(v);
                    }
                }
            }
            let exported = client
                .export_branch(Some(source_branch), &[], None)
                .await
                .unwrap_or_default();
            let filtered_ndjson =
                filter_ndjson_excluding_atoms(&exported, project_id, &excluded_atoms);
            let mut commit_out = None;
            if !filtered_ndjson.trim().is_empty()
                && let Ok(load_res) = client
                    .load_ndjson(target_branch, None, "merge", &filtered_ndjson, None)
                    .await
                && let Ok(parsed_commit) =
                    serde_json::from_value::<CommitOutput>(load_res["commit"].clone())
            {
                commit_out = Some(parsed_commit);
            }
            Ok((commit_out, conflicts))
        }
        Err(err) => Err(err.into()),
    }
}

pub(super) async fn ensure_branch_exists(
    client: &OmnigraphClient,
    branch: &str,
    from: Option<&str>,
) -> Result<()> {
    if branch == "main" {
        return Ok(());
    }
    match client
        .create_branch(&BranchCreateRequest {
            name: branch.to_string(),
            from: from.map(str::to_string),
            from_commit: None,
        })
        .await
    {
        Ok(_) => Ok(()),
        Err(err) if err.status == 409 => Ok(()),
        Err(err) => Err(err.into()),
    }
}

/// The `contradicts` relations on `branches` with one of `atom_ids` at
/// either end, each pair once: what a person checks before relying on
/// either atom. Nothing is retired or changed.
pub(super) async fn contradictions_touching(
    client: &OmnigraphClient,
    project_id: &str,
    branches: &[String],
    atom_ids: &BTreeSet<String>,
) -> Vec<Value> {
    let mut found = Vec::new();
    if atom_ids.is_empty() {
        return found;
    }
    let mut seen = BTreeSet::new();
    for branch in branches {
        let Ok(rows) = run_stored_or_inline(
            client,
            "contradictions",
            schema::QUERY_CONTRADICTIONS,
            json!({ "project_id": project_id }),
            Some(branch.clone()),
            None,
        )
        .await
        else {
            continue;
        };
        for row in &rows.rows {
            let (Some(source), Some(target)) =
                (str_field(row, "source_id"), str_field(row, "target_id"))
            else {
                continue;
            };
            if source == target
                || !(atom_ids.contains(&source) || atom_ids.contains(&target))
                || !seen.insert((source.clone(), target.clone()))
            {
                continue;
            }
            let source_title = str_field(row, "source_title").unwrap_or_else(|| source.clone());
            let target_title = str_field(row, "target_title").unwrap_or_else(|| target.clone());
            let note = str_field(row, "note");
            let text = match &note {
                Some(note) => format!(
                    "\"{source_title}\" contradicts \"{target_title}\" ({note}); check both against the code before relying on either."
                ),
                None => format!(
                    "\"{source_title}\" contradicts \"{target_title}\"; check both against the code before relying on either."
                ),
            };
            found.push(json!({
                "kind": "atom-contradiction",
                "source_id": source,
                "source_title": source_title,
                "target_id": target,
                "target_title": target_title,
                "note": note,
                "confidence": row["confidence"],
                "branch": branch,
                "recommended_action": "review",
                "text": text,
            }));
        }
    }
    found
}

/// The caution line under an injected atom that a `contradicts` relation
/// links to another atom, in either direction.
pub(super) fn contradiction_caution(atom_id: &str, warnings: &[Value]) -> Option<String> {
    let parts = warnings
        .iter()
        .filter_map(|warning| {
            let (source, target) = (
                warning["source_id"].as_str()?,
                warning["target_id"].as_str()?,
            );
            let (verb, other, title) = if source == atom_id {
                ("contradicts", target, warning["target_title"].as_str())
            } else if target == atom_id {
                (
                    "is contradicted by",
                    source,
                    warning["source_title"].as_str(),
                )
            } else {
                return None;
            };
            let title = title
                .filter(|title| !title.trim().is_empty())
                .unwrap_or(other);
            Some(
                match warning["note"].as_str().filter(|n| !n.trim().is_empty()) {
                    Some(note) => format!("{verb} \"{title}\" (`{other}`: {note})"),
                    None => format!("{verb} \"{title}\" (`{other}`)"),
                },
            )
        })
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| {
        format!(
            "  Caution: this {} — verify both against code and `project-brain/` before relying on either.\n",
            parts.join("; ")
        )
    })
}

pub(super) async fn list_atom_files(
    client: &OmnigraphClient,
    project_id: &str,
    branch: &str,
    atom_id: &str,
) -> Vec<String> {
    let edges = query_edges_safe(
        client,
        GQ_LIST_FILE_EDGES,
        "list_file_edges",
        &json!({ "project_id": project_id }),
        Some(branch),
        None,
    )
    .await;
    let mut files = Vec::new();
    for edge in &edges {
        if str_field(edge, "source_id").as_deref() == Some(atom_id)
            && let Some(path) = str_field(edge, "target_id")
            && !files.contains(&path)
        {
            files.push(path);
        }
    }
    files
}

pub(super) async fn collect_branch_atom_ids(
    client: &OmnigraphClient,
    project_id: &str,
    branch: &str,
) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    let rows = fetch_branch_atoms(client, project_id, branch)
        .await
        .unwrap_or_default();
    for atom in &rows {
        if let Some(aid) = str_field(atom, "atom_id") {
            ids.insert(aid);
        }
    }
    if (rows.len() == 100 || rows.len() >= RECENT_ATOMS_LIMIT)
        && let Ok(ndjson) = client
            .export_branch(Some(branch), &["ContextAtom"], None)
            .await
        && let Ok(parsed) = parse_ndjson_records(&ndjson)
    {
        for (aid, row) in parsed.atoms {
            let pid_ok = project_id.is_empty()
                || str_field(&row, "project_id").is_none_or(|p| p == project_id);
            if pid_ok {
                ids.insert(aid);
            }
        }
    }
    ids
}

/// Every atom of `project_id` on `branch`, or an error where the listing may
/// be partial: pruning acts only on a whole listing.
pub(super) async fn collect_branch_atom_rows(
    client: &OmnigraphClient,
    project_id: &str,
    branch: &str,
) -> Result<Vec<Value>> {
    let mut rows = BTreeMap::new();
    let recent = fetch_branch_atoms(client, project_id, branch).await?;
    // `recent_atoms` stops at 100 on legacy clusters or `RECENT_ATOMS_LIMIT`; past that the export has them all.
    let capped = recent.len() == 100 || recent.len() >= RECENT_ATOMS_LIMIT;
    for mut row in recent {
        if row["project_id"].as_str().is_none() {
            // The query is scoped to the project.
            row["project_id"] = json!(project_id);
        }
        if let Some(atom_id) = str_field(&row, "atom_id") {
            rows.insert(atom_id, row);
        }
    }
    if capped {
        let ndjson = client
            .export_branch(Some(branch), &["ContextAtom"], None)
            .await?;
        for (atom_id, row) in parse_ndjson_records(&ndjson)?.atoms {
            if str_field(&row, "project_id").as_deref() == Some(project_id) {
                rows.insert(atom_id, row);
            }
        }
    }
    Ok(rows.into_values().collect())
}

/// Whether `row` is an atom the canonical sync wrote for `project_id` from
/// one of `layers`: its layer, its id prefix and its project all agree.
pub(super) fn synced_atom_of(
    row: &Value,
    atom_id: &str,
    project_id: &str,
    layers: &[String],
) -> bool {
    let layer = row["origin_layer"].as_str().unwrap_or("");
    let prefix = match layer {
        "project-brain" => "brain-",
        "memory-bank" => "mem-",
        _ => return false,
    };
    layers.iter().any(|held| held == layer)
        && atom_id.starts_with(prefix)
        && row["project_id"].as_str() == Some(project_id)
}

pub(super) async fn find_atom_on_branch(
    client: &OmnigraphClient,
    project_id: &str,
    branch: &str,
    atom_id: &str,
) -> Option<Value> {
    let ids = BTreeSet::from([atom_id.to_string()]);
    atoms_on_branch(client, project_id, branch, &ids)
        .await
        .remove(atom_id)
}

/// The atoms of `ids` as `branch` holds them for `project_id`, by id: one
/// `recent_atoms` read, and one export of the branch's atoms only when an
/// id is missing from a listing that may be partial (100 rows, no project
/// to scope it, or a failed read). An id the branch does not hold is left
/// out.
pub(super) async fn atoms_on_branch(
    client: &OmnigraphClient,
    project_id: &str,
    branch: &str,
    ids: &BTreeSet<String>,
) -> BTreeMap<String, Value> {
    let mut found = BTreeMap::new();
    if ids.is_empty() {
        return found;
    }
    let mut whole = false;
    if let Ok(rows) = fetch_branch_atoms(client, project_id, branch).await {
        whole = rows.len() != 100 && rows.len() < RECENT_ATOMS_LIMIT && !project_id.is_empty();
        for row in rows {
            if let Some(atom_id) = str_field(&row, "atom_id").filter(|id| ids.contains(id)) {
                found.entry(atom_id).or_insert(row);
            }
        }
    }
    if whole || found.len() == ids.len() {
        return found;
    }
    let Ok(ndjson) = client
        .export_branch(Some(branch), &["ContextAtom"], None)
        .await
    else {
        return found;
    };
    let Ok(parsed) = parse_ndjson_records(&ndjson) else {
        return found;
    };
    for atom_id in ids {
        if found.contains_key(atom_id) {
            continue;
        }
        if let Some(row) = parsed.atoms.get(atom_id)
            && (project_id.is_empty()
                || str_field(row, "project_id").is_none_or(|p| p == project_id))
        {
            found.insert(atom_id.clone(), normalize_atom_row(row));
        }
    }
    found
}

/// Omnigraph's merge conflicts, each Context Atom `divergent_update` with
/// the atom as the source and the target branch hold it (`source_value`,
/// `target_value`), which the merge preview's Keep source and Keep target
/// need. The atoms come from one read per branch whatever the number of
/// conflicts. A Keep writes the atom back under the caller's project, so
/// there are values only for an atom the branch holds for a named
/// `project_id`; every other conflict passes through as Omnigraph returned
/// it.
pub(super) async fn conflicts_with_values(
    client: &OmnigraphClient,
    project_id: &str,
    source: &str,
    target: &str,
    conflicts: &[MergeConflict],
) -> Vec<Value> {
    let ids: BTreeSet<String> = if project_id.is_empty() {
        BTreeSet::new()
    } else {
        conflicts
            .iter()
            .filter_map(updated_atom_id)
            .map(str::to_string)
            .collect()
    };
    let (source_atoms, target_atoms) = tokio::join!(
        atoms_on_branch(client, project_id, source, &ids),
        atoms_on_branch(client, project_id, target, &ids),
    );
    conflicts
        .iter()
        .map(|conflict| {
            let mut out = json!(conflict);
            if let Some(atom_id) = updated_atom_id(conflict) {
                for (key, atoms) in [
                    ("source_value", &source_atoms),
                    ("target_value", &target_atoms),
                ] {
                    if let Some(atom) = atoms.get(atom_id) {
                        out[key] = whole_atom(atom, project_id);
                    }
                }
            }
            out
        })
        .collect()
}

/// `atom` with every column a branch may hold, as null where it holds none:
/// Omnigraph leaves null columns out of a row and `recent_atoms` returns no
/// project, and a Keep of the whole atom carries what the kept side holds.
pub(super) fn whole_atom(atom: &Value, project_id: &str) -> Value {
    let mut whole = atom.clone();
    if let Some(columns) = whole.as_object_mut() {
        columns
            .entry("project_id")
            .or_insert_with(|| json!(project_id));
        for key in ["tags", "source_session_id", "session_id"] {
            columns.entry(key).or_insert(Value::Null);
        }
    }
    whole
}

/// Every `ContextAtom` property, as `GQ_UPSERT_ATOM` takes them.
const ATOM_COLUMNS: [&str; 20] = [
    "atom_id",
    "project_id",
    "kind",
    "title",
    "summary",
    "body",
    "tags",
    "source_branch",
    "source_session_id",
    "confidence",
    "pin_priority",
    "verified",
    "origin_layer",
    "authority",
    "privacy",
    "brain_record_id",
    "memory_id",
    "record_revision",
    "created_at",
    "updated_at",
];

/// The upsert parameters that write `atom` back as a branch stores it,
/// every property included and nothing restamped.
pub(super) fn stored_atom_params(atom: &Value, project_id: &str) -> Value {
    let atom = whole_atom(atom, project_id);
    let columns = ATOM_COLUMNS
        .iter()
        .map(|key| {
            (
                key.to_string(),
                atom.get(*key).cloned().unwrap_or(Value::Null),
            )
        })
        .collect();
    Value::Object(columns)
}

/// The Context Atom a merge conflict names when both branches updated it.
pub(super) fn updated_atom_id(conflict: &MergeConflict) -> Option<&str> {
    let both_updated = conflict.entity_kind == EntityKind::Node
        && conflict.type_name == "ContextAtom"
        && conflict.kind == MergeConflictKind::DivergentUpdate;
    clean_opt(conflict.entity_id.as_deref()).filter(|_| both_updated)
}

pub(super) fn build_baseline_recovery_payload(
    baseline: &crate::omnigraph::client::ChangeBaselineResponse,
    branch: &str,
) -> (Vec<Value>, Value) {
    let mut events = Vec::new();
    let mut atoms = Vec::new();
    let mut sessions = Vec::new();
    let mut files = Vec::new();
    let mut symbols = Vec::new();
    let mut edges = Vec::new();

    for rec in &baseline.records {
        let Some(obj) = rec.as_object() else {
            continue;
        };
        let props = obj
            .get("data")
            .or_else(|| obj.get("properties"))
            .filter(|v| v.is_object())
            .cloned()
            .unwrap_or_else(|| rec.clone());

        if let Some(edge_type) = obj
            .get("edge")
            .or_else(|| obj.get("edge_type"))
            .and_then(Value::as_str)
            .or_else(|| {
                (obj.get("entity_kind").and_then(Value::as_str) == Some("edge"))
                    .then(|| obj.get("type_name").and_then(Value::as_str))
                    .flatten()
            })
        {
            let from = obj
                .get("from")
                .or_else(|| obj.get("source"))
                .or_else(|| props.get("source_id"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let to = obj
                .get("to")
                .or_else(|| obj.get("target"))
                .or_else(|| props.get("target_id"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let eid = obj
                .get("id")
                .or_else(|| obj.get("entity_id"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("{edge_type}:{from}->{to}"));
            let relation = if edge_type == "RelatesTo" {
                props["relation"]
                    .as_str()
                    .unwrap_or("relates_to")
                    .to_string()
            } else {
                pascal_to_snake(edge_type)
            };
            edges.push(json!({
                "id": eid,
                "type": edge_type,
                "kind": relation,
                "relation": relation,
                "source": from,
                "target": to,
                "from": from,
                "to": to,
                "properties": props,
            }));
            if events.len() < 200 {
                events.push(json!({
                    "graph_commit_id": baseline.snapshot_commit_id,
                    "commit_id": baseline.snapshot_commit_id,
                    "branch": branch,
                    "entity_kind": "edge",
                    "type_name": edge_type,
                    "entity_id": eid,
                    "kind": "edge",
                    "node_id": Value::Null,
                    "atom_id": Value::Null,
                    "session_id": Value::Null,
                    "op": "baseline",
                    "baseline": true,
                    "title": format!("{from} -> {to}"),
                    "summary": relation,
                    "properties": props,
                }));
            }
            continue;
        }

        let type_name = obj
            .get("type")
            .or_else(|| obj.get("type_name"))
            .and_then(Value::as_str)
            .unwrap_or("ContextAtom");
        let eid = obj
            .get("id")
            .or_else(|| obj.get("entity_id"))
            .or_else(|| props.get("atom_id"))
            .or_else(|| props.get("session_id"))
            .or_else(|| props.get("path"))
            .or_else(|| props.get("fqn"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        let mut normalized_node = match type_name {
            "ContextAtom" => {
                let mut n = normalize_atom_row(&props);
                if !eid.is_empty() && n["atom_id"].is_null() {
                    n["atom_id"] = json!(eid);
                    n["id"] = json!(eid);
                }
                atoms.push(n.clone());
                n
            }
            "Session" => {
                let mut n = normalize_session_row(&props);
                if !eid.is_empty() && n["session_id"].is_null() {
                    n["session_id"] = json!(eid);
                    n["id"] = json!(eid);
                }
                sessions.push(n.clone());
                n
            }
            "FileRef" => {
                let mut n = normalize_file_row(&props);
                if !eid.is_empty() && n["path"].is_null() {
                    n["path"] = json!(eid);
                    n["id"] = json!(eid);
                }
                files.push(n.clone());
                n
            }
            "SymbolRef" => {
                let mut n = normalize_symbol_row(&props);
                if !eid.is_empty() && n["fqn"].is_null() {
                    n["fqn"] = json!(eid);
                    n["id"] = json!(eid);
                }
                symbols.push(n.clone());
                n
            }
            _ => props.clone(),
        };

        let title = normalized_node["title"]
            .as_str()
            .or_else(|| normalized_node["path"].as_str())
            .or_else(|| normalized_node["name"].as_str())
            .unwrap_or(&eid)
            .to_string();
        let summary = normalized_node["summary"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let kind = normalized_node["kind"]
            .as_str()
            .unwrap_or(type_name)
            .to_string();
        let atom_id_val = if type_name == "ContextAtom" {
            Value::String(eid.clone())
        } else {
            Value::Null
        };
        let session_id_val = if type_name == "Session" {
            Value::String(eid.clone())
        } else {
            normalized_node
                .get("source_session_id")
                .or_else(|| normalized_node.get("session_id"))
                .cloned()
                .unwrap_or(Value::Null)
        };
        if let Some(m) = normalized_node.as_object_mut() {
            m.entry("branch").or_insert_with(|| json!(branch));
        }

        if events.len() < 200 {
            events.push(json!({
                "graph_commit_id": baseline.snapshot_commit_id,
                "commit_id": baseline.snapshot_commit_id,
                "branch": branch,
                "entity_kind": "node",
                "type_name": type_name,
                "entity_id": eid,
                "kind": kind,
                "node_id": eid,
                "atom_id": atom_id_val,
                "session_id": session_id_val,
                "op": "baseline",
                "baseline": true,
                "title": title,
                "summary": summary,
                "properties": normalized_node,
            }));
        }
    }

    let bundle = json!({
        "snapshot_commit_id": baseline.snapshot_commit_id,
        "resume_cursor": baseline.resume_cursor,
        "record_count": baseline.records.len(),
        "atoms": atoms,
        "sessions": sessions,
        "files": files,
        "symbols": symbols,
        "edges": edges,
        "records": baseline.records,
    });
    (events, bundle)
}
