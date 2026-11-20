use super::OmnigraphManager;
use super::graph_ops::parse_opt_usize;
use super::helpers::{clean_opt, parse_limit};
use crate::omnigraph::transfer::parse_i64_opt;
use crate::{
    HarnessError,
    services::{Domain, Payload, Reply},
};
use serde_json::Value;
use std::sync::Arc;

/// Harness `Domain` router serving `omnigraph.*` operations.
pub struct OmnigraphDomain {
    pub manager: Arc<OmnigraphManager>,
}

impl OmnigraphDomain {
    pub fn new(manager: Arc<OmnigraphManager>) -> Self {
        Self { manager }
    }
}

impl Domain for OmnigraphDomain {
    fn execute<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a> {
        Box::pin(async move {
            let params = &request["params"];
            let query = &request["query"];
            let body = &request["body"];
            let project_id = query["project_id"]
                .as_str()
                .or_else(|| params["project_id"].as_str())
                .or_else(|| body["project_id"].as_str())
                .unwrap_or("");

            let result = match operation {
                "omnigraph.status" => self.manager.status_json(project_id).await?,
                "omnigraph.setup" => self.manager.setup_json(body).await?,
                "omnigraph.export" => {
                    return self.manager.export_context_payload(query, body).await;
                }
                "omnigraph.import" => self.manager.import_context_json(body).await?,
                "omnigraph.link_sessions" => {
                    let store = self.manager.store.as_ref().ok_or_else(|| {
                        HarnessError::unavailable(
                            "Session store is required for omnigraph.link_sessions.",
                        )
                    })?;
                    self.manager.link_sessions_json(body, store).await?
                }
                "omnigraph.graph" => {
                    let limit = parse_opt_usize(&query["limit"], "limit")?;
                    let offset = parse_opt_usize(&query["offset"], "offset")?;
                    let cursor = clean_opt(query["cursor"].as_str());
                    self.manager
                        .graph_view_json_paginated(
                            project_id,
                            query["branch"].as_str(),
                            query["snapshot"].as_str(),
                            query["kind"].as_str(),
                            query["q"].as_str(),
                            query["session_id"].as_str(),
                            limit,
                            offset,
                            cursor,
                        )
                        .await?
                }
                "omnigraph.search" => {
                    let q = query["q"].as_str().unwrap_or("");
                    let limit = parse_limit(&query["limit"]);
                    self.manager
                        .search_json(
                            project_id,
                            q,
                            query["branch"].as_str(),
                            query["kind"].as_str(),
                            limit,
                        )
                        .await?
                }
                "omnigraph.branch_diff" => {
                    let source = query["source"].as_str().unwrap_or("");
                    self.manager
                        .branch_diff_json(project_id, source, query["target"].as_str())
                        .await?
                }
                "omnigraph.branch_act" => self.manager.branch_action_json(body).await?,
                "omnigraph.publish" => self.manager.publish_atom_json(body).await?,
                "omnigraph.share" => self.manager.share_into_session_json(body).await?,
                "omnigraph.sync_items" => {
                    let pid = body["project_id"]
                        .as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(project_id);
                    let items = body["items"].as_array().ok_or_else(|| {
                        HarnessError::invalid("`items` must be an array for omnigraph.sync_items.")
                    })?;
                    let prune_layers = body["prune_layers"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .filter(|layer| matches!(*layer, "project-brain" | "memory-bank"))
                        .map(String::from)
                        .collect::<Vec<_>>();
                    self.manager
                        .sync_canonical_knowledge_with_options(pid, items, &prune_layers)
                        .await?
                }
                "omnigraph.file_context" => {
                    let pid = query["project_id"]
                        .as_str()
                        .or_else(|| body["project_id"].as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(project_id);
                    let path = query["path"]
                        .as_str()
                        .or_else(|| body["path"].as_str())
                        .unwrap_or("");
                    self.manager.file_context_json(pid, path).await?
                }
                "omnigraph.fetch_atom" => {
                    let pid = body["project_id"]
                        .as_str()
                        .or_else(|| query["project_id"].as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(project_id);
                    let atom_id = body["atom_id"]
                        .as_str()
                        .or_else(|| query["atom_id"].as_str())
                        .or_else(|| params["atom_id"].as_str())
                        .unwrap_or("");
                    let branch = body["branch"].as_str().or_else(|| query["branch"].as_str());
                    self.manager
                        .fetch_atom_with_files(pid, atom_id, branch)
                        .await?
                }
                "omnigraph.link_brain" => {
                    let pid = body["project_id"]
                        .as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(project_id);
                    let branch = body["branch"].as_str().unwrap_or("");
                    let atom = &body["atom"];
                    if !atom.is_object() {
                        return Err(HarnessError::invalid(
                            "`atom` must be an object for omnigraph.link_brain.",
                        ));
                    }
                    let brain_record_id = body["brain_record_id"].as_str().unwrap_or("");
                    let privacy = clean_opt(body["privacy"].as_str());
                    let revision =
                        body.get("revision")
                            .and_then(parse_i64_opt)
                            .ok_or_else(|| {
                                HarnessError::invalid(
                                    "`revision` must be an integer for omnigraph.link_brain.",
                                )
                            })?;
                    self.manager
                        .link_atom_to_brain(pid, branch, atom, brain_record_id, revision, privacy)
                        .await?
                }
                "omnigraph.changes" => {
                    self.manager
                        .changes_json(
                            project_id,
                            query["cursor"].as_str(),
                            query["branch"].as_str(),
                        )
                        .await?
                }
                "omnigraph.commit_detail" => {
                    let commit_id = params["commit_id"]
                        .as_str()
                        .or_else(|| query["commit_id"].as_str())
                        .unwrap_or("");
                    self.manager
                        .commit_detail_json(project_id, commit_id)
                        .await?
                }
                _ => return Err(HarnessError::missing("Unknown Omnigraph operation.")),
            };
            if matches!(
                operation,
                "omnigraph.setup"
                    | "omnigraph.import"
                    | "omnigraph.link_sessions"
                    | "omnigraph.branch_act"
                    | "omnigraph.publish"
                    | "omnigraph.share"
                    | "omnigraph.sync_items"
                    | "omnigraph.link_brain"
            ) {
                self.manager.invalidate_stats_cache();
            }
            Ok(Payload::Json(result))
        })
    }
}
