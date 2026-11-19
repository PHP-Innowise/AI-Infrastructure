//! Required native domain ports; the application composes concrete Rust services.
use crate::{HarnessError, Result, store::SessionStore};
use serde_json::{Value, json};
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};
pub enum Payload {
    Json(Value),
    Bytes {
        bytes: Vec<u8>,
        content_type: String,
        filename: Option<String>,
        etag: Option<String>,
        cache: Option<String>,
    },
}
pub type Reply<'a> = Pin<Box<dyn Future<Output = Result<Payload>> + Send + 'a>>;
pub type CloseFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
pub trait Services: Send + Sync {
    fn request<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a>;
    fn close<'a>(&'a self) -> CloseFuture<'a>;
    /// Subscribes to the live multiplexed SSE frame hub, replaying buffered
    /// frames after `last_event_id` when available.
    fn subscribe_stream(&self, _last_event_id: Option<u64>) -> Option<crate::store::ReplayStart> {
        None
    }
}

fn ensure_graph_event_bridge(store: &SessionStore) {
    if !store.stream().claim_graph_bridge() {
        return;
    }
    let mut rx = crate::graph::subscribe_graph_events();
    let store = store.clone();
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        if ev.state_dir == store.state {
                            let pid = ev.project_id.clone();
                            if let Ok(val) = serde_json::to_value(&ev) {
                                store.publish_event("graph_status", None, Some(pid), val);
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }
}
pub trait Domain: Send + Sync {
    fn execute<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a>;
}
pub struct NativeServices {
    pub store: SessionStore,
    pub domains: BTreeMap<String, Arc<dyn Domain>>,
    pub bootstrap: Arc<dyn Fn() -> Result<Value> + Send + Sync>,
    pub overrides: BTreeMap<String, Arc<dyn Domain>>,
    pub closer: Arc<dyn Fn() -> CloseFuture<'static> + Send + Sync>,
}
/// The most `knowledge.sync_collect` pages one sync reads.
const SYNC_PAGES: usize = 64;
/// Every page of `knowledge.sync_collect` as one collection. `prune_layers`
/// names the layers every page reported read whole, and is empty when the
/// pages disagree on the total (knowledge changed between them) or run past
/// `SYNC_PAGES`: a sync prunes only with the whole collection in hand.
pub(crate) async fn collect_sync_items(
    knowledge: &dyn Domain,
    project_id: &str,
    query: &Value,
    body: &Value,
) -> Result<Value> {
    let mut items = Vec::new();
    let mut first = None::<Value>;
    let mut layers = None::<std::collections::BTreeSet<String>>;
    let mut offset = 0u64;
    let mut whole = false;
    for _ in 0..SYNC_PAGES {
        let mut page_body = body.clone();
        if !page_body.is_object() {
            page_body = json!({});
        }
        page_body["offset"] = json!(offset);
        let Payload::Json(page) = knowledge
            .execute(
                "knowledge.sync_collect",
                json!({"params":{"project_id":project_id},"query":query,"body":page_body}),
            )
            .await?
        else {
            return Err(HarnessError::unavailable(
                "Unexpected knowledge sync payload.",
            ));
        };
        items.extend(page["items"].as_array().cloned().unwrap_or_default());
        let reported = page["complete_layers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect::<std::collections::BTreeSet<_>>();
        layers = Some(match layers {
            None => reported,
            Some(held) => held.intersection(&reported).cloned().collect(),
        });
        let consistent = first
            .as_ref()
            .is_none_or(|first| first["total_items"] == page["total_items"]);
        if first.is_none() {
            first = Some(page.clone());
        }
        if !consistent {
            break;
        }
        match page["next_offset"].as_u64() {
            Some(next) if page["has_more"] == true && next > offset => offset = next,
            _ => {
                whole = true;
                break;
            }
        }
    }
    let first = first.unwrap_or_else(|| json!({}));
    let prune_layers = if whole {
        layers.unwrap_or_default().into_iter().collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    Ok(json!({
        "items": items,
        "brain_count": first["brain_count"],
        "memory_count": first["memory_count"],
        "skipped": first["skipped"],
        "total_items": first["total_items"],
        "prune_layers": prune_layers,
    }))
}
impl NativeServices {
    async fn auto_start_graph(&self, project_id: &str) {
        if !project_id.is_empty()
            && let Some(graph) = self.domains.get("graph")
        {
            let _ = graph
                .execute(
                    "graph.auto_start",
                    json!({"params":{"project_id":project_id},"body":{},"query":{}}),
                )
                .await;
        }
    }
    async fn best_effort_sync_knowledge(
        &self,
        project_id: &str,
        bank: Option<&str>,
        session_id: Option<&str>,
    ) {
        if project_id.trim().is_empty() {
            return;
        }
        let (Some(knowledge), Some(omnigraph)) =
            (self.domains.get("knowledge"), self.domains.get("omnigraph"))
        else {
            return;
        };
        if let Ok(Payload::Json(status)) = omnigraph
            .execute(
                "omnigraph.status",
                json!({"params":{},"body":{},"query":{"project_id":project_id}}),
            )
            .await
            && (status["mode"] == "off" || status["state"] == "unconfigured")
        {
            return;
        }
        let mut collect_body = json!({"project_id": project_id});
        if let Some(b) = bank.map(str::trim).filter(|s| !s.is_empty()) {
            collect_body["bank"] = json!(b);
        }
        if let Some(s) = session_id.map(str::trim).filter(|s| !s.is_empty()) {
            collect_body["session_id"] = json!(s);
        }
        let Ok(collected) =
            collect_sync_items(knowledge.as_ref(), project_id, &json!({}), &collect_body).await
        else {
            return;
        };
        let _ = omnigraph
            .execute(
                "omnigraph.sync_items",
                json!({"params":{},"query":{},"body":{"project_id":project_id,"items":collected["items"],"prune_layers":collected["prune_layers"]}}),
            )
            .await;
    }
    async fn attach_waiting_permission_counts(&self, sessions: &mut [Value]) {
        let counts = self.waiting_permission_counts().await;
        for summary in sessions {
            let count = summary["id"]
                .as_str()
                .and_then(|id| counts.get(id))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            summary["waiting_permissions"] = json!(count);
        }
    }
    async fn handle_bootstrap(&self) -> Result<Payload> {
        let mut data = (self.bootstrap)()?;
        if let Some(graph) = self.domains.get("graph") {
            let _ = graph
                .execute(
                    "graph.auto_start_all",
                    json!({"params":{},"body":{},"query":{}}),
                )
                .await;
        }
        if let Some(omnigraph) = self.domains.get("omnigraph")
            && let Ok(Payload::Json(status)) = omnigraph
                .execute(
                    "omnigraph.status",
                    json!({"params":{},"body":{},"query":{}}),
                )
                .await
            && status.get("state").is_some()
        {
            data["omnigraph"] = status;
        }
        if let Some(sessions) = data["sessions"].as_array_mut() {
            self.attach_waiting_permission_counts(sessions).await;
        }
        Ok(Payload::Json(data))
    }
    async fn handle_sessions_list(&self, params: &Value, query: &Value) -> Result<Payload> {
        if let Some(project_id) = query["project_id"]
            .as_str()
            .or_else(|| params["project_id"].as_str())
            .filter(|id| !id.is_empty())
        {
            self.auto_start_graph(project_id).await;
        }
        let mut summaries = self.store.summaries()?;
        self.attach_waiting_permission_counts(&mut summaries).await;
        Ok(Payload::Json(json!({"sessions": summaries})))
    }
    async fn handle_sessions_get(&self, sid: &str, query: &Value) -> Result<Payload> {
        let parse_opt_i64 = |key: &str| -> Result<Option<i64>> {
            match query[key].as_str() {
                None => Ok(None),
                Some(raw) => raw
                    .parse::<i64>()
                    .map(Some)
                    .map_err(|_| HarnessError::invalid("Invalid event cursor.")),
            }
        };
        let after = parse_opt_i64("after")?;
        let before = parse_opt_i64("before")?;
        let limit = match query["limit"].as_str() {
            None => crate::store::DEFAULT_EVENT_PAGE_LIMIT,
            Some(raw) => {
                let parsed = raw
                    .parse::<usize>()
                    .map_err(|_| HarnessError::invalid("Invalid event page limit."))?;
                if !(1..=crate::store::MAX_EVENT_PAGE_LIMIT).contains(&parsed) {
                    return Err(HarnessError::invalid(format!(
                        "Event page limit must be between 1 and {}.",
                        crate::store::MAX_EVENT_PAGE_LIMIT
                    )));
                }
                parsed
            }
        };
        let tail = match query["tail"].as_str() {
            None | Some("0" | "false" | "no" | "") => false,
            Some("1" | "true" | "yes") => true,
            Some(_) => return Err(HarnessError::invalid("Invalid tail pagination flag.")),
        };
        let page = self
            .store
            .session_events_page(sid, after, before, limit, tail)?;
        let mut session = page.session;
        session["graph_focus"] = crate::graph_live::session_focus_value(&self.store, sid)?;
        if after.is_none_or(|v| v == 0)
            && before.is_none()
            && let Some(project_id) = session["project_id"].as_str()
        {
            self.auto_start_graph(project_id).await;
        }
        let waiting = self.waiting_permissions_for(sid).await;
        Ok(Payload::Json(json!({
            "session": session,
            "events": page.events,
            "waiting_permissions": waiting,
            "has_more": page.has_more,
            "has_older": page.has_older,
            "total_events": page.total_events,
            "oldest_event_id": page.oldest_event_id,
            "latest_event_id": page.latest_event_id,
        })))
    }
    fn emit_domain_mutation_sse(
        &self,
        operation: &str,
        params: &Value,
        body: &Value,
        payload: &Payload,
    ) {
        let Payload::Json(val) = payload else {
            return;
        };
        match operation {
            "creator.start" | "creator.act" => {
                let run = val.get("run").filter(|v| v.is_object()).unwrap_or(val);
                let Some(run_id) = run["id"].as_str().filter(|s| !s.is_empty()) else {
                    return;
                };
                let sid = run["session_id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                let project_id = run["project_id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                let status = run["status"].as_str().unwrap_or("new");
                let active = run["active"].as_bool().unwrap_or(matches!(
                    status,
                    "scanning" | "generating" | "applying" | "rolling_back" | "queued" | "running"
                ));
                let waiting = run["waiting_permissions"]
                    .as_u64()
                    .map(|n| n as usize)
                    .unwrap_or_else(|| sid.as_deref().map_or(0, |id| self.store.waiting_count(id)));
                let mut ev = json!({
                    "run_id": run_id,
                    "id": run_id,
                    "project_id": project_id,
                    "status": status,
                    "active": active,
                    "waiting_permissions": waiting,
                    "session_id": sid,
                    "run": run,
                });
                if let Some(rev) = run["revision"].as_u64() {
                    ev["revision"] = json!(rev);
                }
                self.store.publish_event("creator_run", sid, project_id, ev);
            }
            "systems.prepare" | "systems.act" | "systems.apply" | "discovery.start" => {
                let run = val.get("run").filter(|v| v.is_object()).unwrap_or(val);
                let run_id = run["id"]
                    .as_str()
                    .or_else(|| run["run_id"].as_str())
                    .or_else(|| params["id"].as_str())
                    .filter(|s| !s.is_empty());
                let Some(run_id) = run_id else {
                    return;
                };
                let sid = run["session_id"]
                    .as_str()
                    .or_else(|| val["session"]["id"].as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                let project_id = run["project_id"]
                    .as_str()
                    .or_else(|| body["project_id"].as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                let status = run["status"]
                    .as_str()
                    .or_else(|| val["status"].as_str())
                    .unwrap_or("queued");
                let active = run["active"]
                    .as_bool()
                    .unwrap_or(matches!(status, "queued" | "running"));
                let waiting = run["waiting_permissions"]
                    .as_u64()
                    .map(|n| n as usize)
                    .unwrap_or_else(|| sid.as_deref().map_or(0, |id| self.store.waiting_count(id)));
                let ev = json!({
                    "run_id": run_id,
                    "id": run_id,
                    "project_id": project_id,
                    "status": status,
                    "active": active,
                    "waiting_permissions": waiting,
                    "session_id": sid,
                    "run": run,
                });
                self.store.publish_event("system_run", sid, project_id, ev);
            }
            "results.start_check" => {
                let check = val.get("check").filter(|v| v.is_object()).unwrap_or(val);
                let Some(check_id) = check["id"].as_str().filter(|s| !s.is_empty()) else {
                    return;
                };
                let sid = check["session_id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or_else(|| params["session_id"].as_str().filter(|s| !s.is_empty()))
                    .unwrap_or("")
                    .to_owned();
                let project_id = (!sid.is_empty())
                    .then(|| self.store.session_project_id(&sid))
                    .flatten();
                let status = check["status"].as_str().unwrap_or("queued");
                self.store.publish_event(
                    "results_check",
                    (!sid.is_empty()).then_some(sid.clone()),
                    project_id.clone(),
                    json!({
                        "id": check_id,
                        "check_id": check_id,
                        "session_id": sid,
                        "project_id": project_id,
                        "status": status,
                        "check": check,
                    }),
                );
            }
            "graph.act" => {
                let project_id = val["project_id"]
                    .as_str()
                    .or_else(|| params["project_id"].as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("")
                    .to_owned();
                if project_id.is_empty() {
                    return;
                }
                let state = val["state"]
                    .as_str()
                    .or_else(|| body["action"].as_str())
                    .unwrap_or("updated");
                let mut ev = val.clone();
                if ev.get("project_id").is_none() {
                    ev["project_id"] = json!(project_id);
                }
                if ev.get("state").is_none() {
                    ev["state"] = json!(state);
                }
                if ev.get("reason").is_none() {
                    ev["reason"] = json!(body["action"].as_str().unwrap_or("graph_act"));
                }
                if ev.get("revision").is_none() {
                    ev["revision"] = json!(0);
                }
                self.store
                    .publish_event("graph_status", None, Some(project_id), ev);
            }
            _ => {}
        }
    }
    async fn handle_memory_links(&self, operation: &str, request: &Value) -> Result<Payload> {
        let knowledge = self
            .domains
            .get("knowledge")
            .ok_or_else(|| HarnessError::unavailable("Required native service is unavailable."))?;
        let Payload::Json(mut links) = knowledge.execute(operation, request.clone()).await? else {
            return Err(HarnessError::unavailable(
                "Unexpected memory links payload.",
            ));
        };
        let project_id = links
            .get("project_id")
            .filter(|v| v.as_str().is_some_and(|s| !s.is_empty()))
            .cloned()
            .unwrap_or_else(|| request["params"]["project_id"].clone());
        if operation == "knowledge.memory_links_batch" {
            if let Some(items) = links["items"].as_array_mut() {
                for item in items {
                    self.enrich_memory_links_item(&project_id, item).await;
                }
            }
        } else {
            self.enrich_memory_links_item(&project_id, &mut links).await;
        }
        Ok(Payload::Json(links))
    }
    async fn handle_preflight(&self, request: &Value) -> Result<Payload> {
        let knowledge = self
            .domains
            .get("knowledge")
            .ok_or_else(|| HarnessError::unavailable("Required native service is unavailable."))?;
        let Payload::Json(mut preflight) = knowledge
            .execute("knowledge.preflight", request.clone())
            .await?
        else {
            return Err(HarnessError::unavailable("Unexpected preflight payload."));
        };
        let project_id = preflight
            .get("project_id")
            .filter(|v| v.as_str().is_some_and(|s| !s.is_empty()))
            .cloned()
            .unwrap_or_else(|| request["params"]["project_id"].clone());
        let mut contradictions = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        if let Some(files) = preflight["files"].as_array_mut() {
            for item in files {
                self.enrich_memory_links_item(&project_id, item).await;
                collect_unique_contradictions(item, &mut contradictions, &mut seen);
            }
        }
        if preflight.is_object() {
            preflight["contradiction_warnings"] = Value::Array(contradictions);
        }
        Ok(Payload::Json(preflight))
    }
    async fn annotate_delivery_preview(&self, sid: &str, preview: &mut Value) {
        let mut contradiction_warnings = Vec::new();
        let mut hygiene_warnings = Vec::new();
        let mut risk_callers = Vec::new();

        let paths = extract_delivery_preview_paths(preview);
        let project_id = (!sid.is_empty())
            .then(|| self.store.get(sid).ok())
            .flatten()
            .and_then(|s| s["project_id"].as_str().map(str::to_owned));

        if let Some(project_id) = project_id.filter(|id| !id.is_empty())
            && !paths.is_empty()
        {
            let preflight_req = json!({
                "params": { "project_id": project_id },
                "query": {},
                "body": {
                    "paths": paths,
                    "risk_caller_hints": false,
                },
            });
            let mut enriched_via_preflight = false;
            if let Ok(Payload::Json(pf)) = Box::pin(self.handle_preflight(&preflight_req)).await
                && pf["files"].is_array()
            {
                enriched_via_preflight = true;
                if let Some(items) = pf["contradiction_warnings"].as_array() {
                    contradiction_warnings = items.clone();
                }
                if let Some(items) = pf["hygiene_warnings"].as_array() {
                    hygiene_warnings = items.clone();
                }
                if let Some(items) = pf["risk_callers"].as_array() {
                    risk_callers = items.clone();
                }
            }
            if !enriched_via_preflight {
                let mut seen = std::collections::BTreeSet::new();
                for path in &paths {
                    let mut item = json!({ "path": path, "contradiction_warnings": [] });
                    self.enrich_memory_links_item(&json!(project_id), &mut item)
                        .await;
                    collect_unique_contradictions(&item, &mut contradiction_warnings, &mut seen);
                }
            }
        }

        preview["memory_warnings"] = json!({
            "contradiction_warnings": contradiction_warnings,
            "hygiene_warnings": hygiene_warnings,
        });
        preview["risk_callers"] = Value::Array(risk_callers);
    }
    async fn handle_delivery(&self, operation: &str, request: Value) -> Result<Payload> {
        let domain = self
            .domains
            .get("delivery")
            .ok_or_else(|| HarnessError::unavailable("Required native service is unavailable."))?;
        let sid = request["params"]["session_id"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        let action = request["body"]["action"].as_str().unwrap_or("").to_owned();
        let mut payload = domain.execute(operation, request).await?;
        if let Payload::Json(ref mut val) = payload {
            if operation == "delivery.preview_commit"
                || operation == "delivery.preview_transfer"
                || (operation == "delivery.act"
                    && matches!(action.as_str(), "preview_commit" | "preview_transfer"))
            {
                Box::pin(self.annotate_delivery_preview(&sid, val)).await;
            } else if operation == "delivery.get"
                && let Some(preview) = val.get_mut("preview").filter(|v| v.is_object())
            {
                Box::pin(self.annotate_delivery_preview(&sid, preview)).await;
            }
        }
        Ok(payload)
    }
    async fn waiting_permission_counts(&self) -> serde_json::Map<String, Value> {
        let Some(sessions) = self.domains.get("sessions") else {
            return serde_json::Map::new();
        };
        match sessions
            .execute(
                "sessions.waiting_counts",
                json!({"params":{},"query":{},"body":{}}),
            )
            .await
        {
            Ok(Payload::Json(waiting)) => {
                waiting["counts"].as_object().cloned().unwrap_or_default()
            }
            _ => serde_json::Map::new(),
        }
    }
    async fn waiting_permissions_for(&self, sid: &str) -> Vec<Value> {
        let Some(sessions) = self.domains.get("sessions") else {
            return Vec::new();
        };
        match sessions
            .execute(
                "sessions.waiting_permissions",
                json!({"params":{"session_id":sid},"query":{},"body":{}}),
            )
            .await
        {
            Ok(Payload::Json(waiting)) => waiting["items"].as_array().cloned().unwrap_or_default(),
            _ => Vec::new(),
        }
    }
    async fn enrich_memory_links_item(&self, project_id: &Value, links: &mut Value) {
        // The shared graph's atoms are a companion: an unreachable Omnigraph
        // leaves the Brain and Bank links standing.
        links["omnigraph_atoms"] = json!([]);
        let Some(omnigraph) = self.domains.get("omnigraph") else {
            return;
        };
        let file = json!({
            "params": {},
            "body": {},
            "query": {"project_id": project_id, "path": links["path"]},
        });
        match omnigraph.execute("omnigraph.file_context", file).await {
            Ok(Payload::Json(found)) if found["ok"] == true => {
                links["omnigraph_atoms"] = found["atoms"].clone();
                links["contradiction_warnings"] =
                    json!(crate::knowledge::KnowledgeDomain::contradiction_warnings(
                        &found["contradictions"],
                        &found["atoms"],
                    ));
            }
            Ok(Payload::Json(found)) => {
                links["omnigraph_skipped"] = found["skipped"].clone();
            }
            Ok(_) => {}
            Err(error) => {
                links["omnigraph_error"] = json!(error.message);
            }
        }
    }
    pub fn new(
        store: SessionStore,
        domains: BTreeMap<String, Arc<dyn Domain>>,
        bootstrap: Arc<dyn Fn() -> Result<Value> + Send + Sync>,
        overrides: BTreeMap<String, Arc<dyn Domain>>,
        closer: Arc<dyn Fn() -> CloseFuture<'static> + Send + Sync>,
    ) -> Result<Self> {
        let required = [
            "sessions",
            "setup",
            "accelerators",
            "systems",
            "discovery",
            "creator",
            "skills",
            "results",
            "delivery",
            "brain",
            "context",
            "projects",
            "commands",
            "files",
            "memory",
            "updates",
            "providers",
            "knowledge",
            "omnigraph",
        ];
        if required.iter().any(|name| !domains.contains_key(*name)) {
            return Err(HarnessError::invalid(
                "Every native Harness domain must be supplied before serving HTTP.",
            ));
        }
        Ok(Self {
            store,
            domains,
            bootstrap,
            overrides,
            closer,
        })
    }
}
fn extract_delivery_preview_paths(preview: &Value) -> Vec<String> {
    let mut paths = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    if let Some(files) = preview["files"].as_array() {
        for entry in files {
            let Some(clean) = entry
                .as_str()
                .or_else(|| entry["path"].as_str())
                .and_then(crate::knowledge::clean_relative_path)
            else {
                continue;
            };
            if seen.insert(clean.clone()) {
                paths.push(clean);
                if paths.len() >= 24 {
                    break;
                }
            }
        }
    }
    paths
}
fn collect_unique_contradictions(
    item: &Value,
    out: &mut Vec<Value>,
    seen: &mut std::collections::BTreeSet<(String, String)>,
) {
    for warning in item["contradiction_warnings"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let key = (
            warning["source_id"].as_str().unwrap_or("").to_owned(),
            warning["target_id"].as_str().unwrap_or("").to_owned(),
        );
        if seen.insert(key) {
            let mut annotated = warning.clone();
            if annotated.get("path").is_none() && item["path"].is_string() {
                annotated["path"] = item["path"].clone();
            }
            out.push(annotated);
        }
    }
}
impl Services for NativeServices {
    fn close<'a>(&'a self) -> CloseFuture<'a> {
        (self.closer)()
    }
    fn subscribe_stream(&self, last_event_id: Option<u64>) -> Option<crate::store::ReplayStart> {
        ensure_graph_event_bridge(&self.store);
        Some(self.store.stream().subscribe(last_event_id))
    }
    fn request<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a> {
        Box::pin(async move {
            let params = &request["params"];
            let query = &request["query"];
            let body = &request["body"];
            let sid = params["session_id"].as_str().unwrap_or("");
            let payload = if let Some(domain) = self.overrides.get(operation) {
                domain.execute(operation, request.clone()).await?
            } else {
                match operation {
                    "bootstrap" => Box::pin(self.handle_bootstrap()).await?,
                    "sessions.list" => Box::pin(self.handle_sessions_list(params, query)).await?,
                    "sessions.get" => Box::pin(self.handle_sessions_get(sid, query)).await?,
                    "sessions.merge_archive" => Payload::Json(self.store.merge_archive(sid)?),
                    "sessions.delete_merge" => {
                        if !crate::store::exact(body, &[], &[]) {
                            return Err(HarnessError::invalid(
                                "Invalid saved context deletion request.",
                            ));
                        }
                        Payload::Json(json!({"session":self.store.delete_merge(sid)?}))
                    }
                    "sessions.report" => {
                        let session = self.store.get(sid)?;
                        if session["status"] != "completed" {
                            return Err(HarnessError::conflict(
                                "An approved Fleet report is not available.",
                            ));
                        }
                        let text = session["fleet_result"]["report"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .ok_or_else(|| {
                                HarnessError::missing("An approved Fleet report is not available.")
                            })?;
                        Payload::Bytes {
                            bytes: text.as_bytes().to_vec(),
                            content_type: "text/markdown; charset=utf-8".into(),
                            filename: None,
                            etag: None,
                            cache: None,
                        }
                    }
                    "attachments.read" => {
                        let (item, bytes, _) = crate::attachments::read(
                            &self.store,
                            sid,
                            params["id"].as_str().unwrap_or(""),
                        )?;
                        Payload::Bytes {
                            bytes,
                            content_type: "application/octet-stream".into(),
                            filename: item["name"].as_str().map(String::from),
                            etag: None,
                            cache: None,
                        }
                    }
                    "projects.list" => {
                        Payload::Json(json!({"projects":(self.bootstrap)()?["projects"]}))
                    }
                    "projects.add" => {
                        if !crate::store::exact(body, &["path"], &[]) {
                            return Err(HarnessError::invalid(
                                "Invalid project registration request.",
                            ));
                        }
                        let project = self.store.register(std::path::Path::new(
                            body["path"]
                                .as_str()
                                .ok_or_else(|| HarnessError::invalid("Invalid project folder."))?,
                        ))?;
                        self.domains["accelerators"]
                            .execute(
                                "accelerators.registered",
                                json!({"params":{"project_id":project["id"]},"body":{},"query":{}}),
                            )
                            .await?;
                        self.auto_start_graph(project["id"].as_str().unwrap_or(""))
                            .await;
                        Payload::Json(
                            json!({"project":project,"projects":(self.bootstrap)()?["projects"]}),
                        )
                    }
                    "projects.select" => {
                        let project_id = params["project_id"]
                            .as_str()
                            .or_else(|| body["project_id"].as_str())
                            .ok_or_else(|| HarnessError::invalid("Invalid project selection."))?;
                        let project = self.store.project(project_id)?;
                        self.auto_start_graph(project_id).await;
                        Payload::Json(json!({"project":project}))
                    }
                    "graph.status" => {
                        if query["check"].as_str() == Some("1") {
                            self.auto_start_graph(params["project_id"].as_str().unwrap_or(""))
                                .await;
                        }
                        self.domains
                            .get("graph")
                            .ok_or_else(|| {
                                HarnessError::unavailable("Required native service is unavailable.")
                            })?
                            .execute(operation, request.clone())
                            .await?
                    }
                    "sdd.artifacts" => {
                        let session = self.store.get(sid)?;
                        if session["sdd"].is_null() {
                            return Err(HarnessError::invalid("This session does not use SDD."));
                        }
                        Payload::Json(
                            json!({"session_id":sid,"sdd":session["sdd"],"files":crate::sdd::artifacts(std::path::Path::new(session["project_path"].as_str().unwrap_or("")),&session["sdd"])?}),
                        )
                    }
                    "omnigraph.branch_act" => {
                        let domain = self.domains.get("omnigraph").ok_or_else(|| {
                            HarnessError::unavailable("Required native service is unavailable.")
                        })?;
                        let mut payload = domain.execute(operation, request.clone()).await?;
                        if let Payload::Json(ref mut val) = payload
                            && let Some(updated_session) =
                                crate::sessions::sync_omnigraph_merge_to_session(
                                    &self.store,
                                    &request["body"],
                                    val,
                                )
                        {
                            val["session"] = updated_session;
                        }
                        payload
                    }
                    "omnigraph.share" => {
                        let domain = self.domains.get("omnigraph").ok_or_else(|| {
                            HarnessError::unavailable("Required native service is unavailable.")
                        })?;
                        let mut payload = domain.execute(operation, request.clone()).await?;
                        if let Payload::Json(ref mut val) = payload
                            && let Some(updated_session) =
                                crate::sessions::sync_omnigraph_share_to_session(
                                    &self.store,
                                    domain.as_ref(),
                                    &request["body"],
                                    val,
                                )
                                .await
                        {
                            val["session"] = updated_session;
                        }
                        payload
                    }
                    "omnigraph.import" => {
                        let domain = self.domains.get("omnigraph").ok_or_else(|| {
                            HarnessError::unavailable("Required native service is unavailable.")
                        })?;
                        let mut payload = domain.execute(operation, request.clone()).await?;
                        if let Payload::Json(ref mut val) = payload {
                            let sessions_domain = self.domains.get("sessions").map(Arc::as_ref);
                            let synced = crate::sessions::sync_omnigraph_import_to_sessions(
                                &self.store,
                                sessions_domain,
                                &request["body"],
                                val,
                            )
                            .await;
                            if !synced.inbox_notifications.is_empty() {
                                val["inbox_notifications"] =
                                    Value::Array(synced.inbox_notifications);
                            }
                            let updated_sessions = synced.sessions;
                            if !updated_sessions.is_empty() {
                                val["session"] = updated_sessions[0].clone();
                                val["sessions"] = Value::Array(updated_sessions);
                            }
                        }
                        payload
                    }
                    "omnigraph.link_sessions" => {
                        let domain = self.domains.get("omnigraph").ok_or_else(|| {
                            HarnessError::unavailable("Required native service is unavailable.")
                        })?;
                        let mut payload = domain.execute(operation, request.clone()).await?;
                        if let Payload::Json(ref mut val) = payload {
                            let sessions_domain = self.domains.get("sessions").map(Arc::as_ref);
                            let synced = crate::sessions::sync_omnigraph_link_to_sessions(
                                &self.store,
                                sessions_domain,
                                &request["body"],
                                val,
                            )
                            .await;
                            if !synced.inbox_notifications.is_empty() {
                                val["inbox_notifications"] =
                                    Value::Array(synced.inbox_notifications);
                            }
                            let updated_sessions = synced.sessions;
                            if let Some(first) = updated_sessions.first() {
                                val["session"] = first.clone();
                            }
                            val["sessions"] = Value::Array(updated_sessions);
                        }
                        payload
                    }
                    "knowledge.preflight" => Box::pin(self.handle_preflight(&request)).await?,
                    "knowledge.memory_links" | "knowledge.memory_links_batch" => {
                        Box::pin(self.handle_memory_links(operation, &request)).await?
                    }
                    "delivery.get"
                    | "delivery.act"
                    | "delivery.preview_commit"
                    | "delivery.preview_transfer" => {
                        Box::pin(self.handle_delivery(operation, request.clone())).await?
                    }
                    "omnigraph.sync_knowledge" => {
                        let project_id = body["project_id"]
                            .as_str()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .or_else(|| {
                                params["project_id"]
                                    .as_str()
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())
                            })
                            .ok_or_else(|| HarnessError::invalid("Missing project_id."))?;
                        let knowledge = self.domains.get("knowledge").ok_or_else(|| {
                            HarnessError::unavailable("Required native service is unavailable.")
                        })?;
                        let omnigraph = self.domains.get("omnigraph").ok_or_else(|| {
                            HarnessError::unavailable("Required native service is unavailable.")
                        })?;
                        let collected =
                            collect_sync_items(knowledge.as_ref(), project_id, query, body).await?;
                        let Payload::Json(synced) = omnigraph
                            .execute(
                                "omnigraph.sync_items",
                                json!({
                                    "params": {},
                                    "query": {},
                                    "body": {
                                        "project_id": project_id,
                                        "items": collected["items"],
                                        "prune_layers": collected["prune_layers"],
                                    },
                                }),
                            )
                            .await?
                        else {
                            return Err(HarnessError::unavailable(
                                "Unexpected omnigraph sync payload.",
                            ));
                        };
                        let ok = synced["ok"] == true;
                        let skip_reason = (!ok)
                            .then(|| synced["skipped"].as_str().map(str::to_owned))
                            .flatten();
                        let mut out = json!({
                            "ok": ok,
                            "branch": synced["branch"].as_str().unwrap_or("knowledge/canonical"),
                            "synced": synced["synced"].as_u64().unwrap_or(0),
                            "brain_count": collected["brain_count"].as_u64().unwrap_or(0),
                            "memory_count": collected["memory_count"].as_u64().unwrap_or(0),
                            "skipped": skip_reason.as_deref().map(|r| json!(r)).unwrap_or_else(|| json!(collected["skipped"].as_u64().unwrap_or(0))),
                            "atom_ids": synced["atom_ids"].as_array().cloned().unwrap_or_default(),
                            "deleted": synced["deleted"].as_u64().unwrap_or(0),
                            "deleted_atom_ids": synced["deleted_atom_ids"].as_array().cloned().unwrap_or_default(),
                            "symbols": synced["symbols"].as_u64().unwrap_or(0),
                        });
                        if let Some(error) = synced["prune_error"].as_str() {
                            out["prune_error"] = json!(error);
                        }
                        if !ok {
                            let err_msg = synced["error"]
                                .as_str()
                                .or_else(|| synced["message"].as_str())
                                .map(str::to_owned)
                                .unwrap_or_else(|| {
                                    if skip_reason.as_deref() == Some("omnigraph-off") {
                                        "Omnigraph is off. Configure or enable Omnigraph before syncing.".to_owned()
                                    } else if let Some(reason) = &skip_reason {
                                        format!("Omnigraph sync skipped: {reason}")
                                    } else {
                                        "Omnigraph sync failed.".to_owned()
                                    }
                                });
                            out["error"] = json!(err_msg);
                            out["message"] = json!(err_msg);
                        }
                        Payload::Json(out)
                    }
                    "omnigraph.promote" => {
                        let project_id = body["project_id"]
                            .as_str()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .or_else(|| {
                                params["project_id"]
                                    .as_str()
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())
                            })
                            .ok_or_else(|| HarnessError::invalid("Missing project_id."))?;
                        let atom_id = body["atom_id"]
                            .as_str()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .ok_or_else(|| HarnessError::invalid("Missing atom_id."))?;
                        let knowledge = self.domains.get("knowledge").ok_or_else(|| {
                            HarnessError::unavailable("Required native service is unavailable.")
                        })?;
                        let omnigraph = self.domains.get("omnigraph").ok_or_else(|| {
                            HarnessError::unavailable("Required native service is unavailable.")
                        })?;
                        let Payload::Json(fetched) = omnigraph
                            .execute(
                                "omnigraph.fetch_atom",
                                json!({
                                    "params": {},
                                    "query": {},
                                    "body": {
                                        "project_id": project_id,
                                        "atom_id": atom_id,
                                        "branch": body.get("branch").cloned().unwrap_or(Value::Null),
                                    },
                                }),
                            )
                            .await?
                        else {
                            return Err(HarnessError::unavailable(
                                "Unexpected omnigraph atom payload.",
                            ));
                        };
                        let Payload::Json(promoted) = knowledge
                            .execute(
                                "knowledge.promote_atom",
                                json!({
                                    "params": { "project_id": project_id },
                                    "query": {},
                                    "body": {
                                        "project_id": project_id,
                                        "bank": body.get("bank").cloned().unwrap_or(Value::Null),
                                        "session_id": body.get("session_id").cloned().unwrap_or(Value::Null),
                                        "workspace": body.get("workspace").cloned().unwrap_or(Value::Null),
                                        "atom": fetched["atom"],
                                        "files": fetched["files"],
                                        "kind": body.get("kind").cloned().unwrap_or(Value::Null),
                                        "privacy": body.get("privacy").cloned().unwrap_or(Value::Null),
                                    },
                                }),
                            )
                            .await?
                        else {
                            return Err(HarnessError::unavailable(
                                "Unexpected knowledge promote payload.",
                            ));
                        };
                        let branch = fetched["branch"].as_str().unwrap_or("main");
                        let brain_record_id = promoted["record_id"].as_str().unwrap_or("");
                        let revision = promoted["revision"].as_i64().unwrap_or(1);
                        let privacy = promoted
                            .get("privacy")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty());
                        let mut link_body = json!({
                            "project_id": project_id,
                            "branch": branch,
                            "atom": fetched["atom"],
                            "brain_record_id": brain_record_id,
                            "revision": revision,
                        });
                        if let Some(p) = privacy {
                            link_body["privacy"] = json!(p);
                        }
                        let Payload::Json(linked) = omnigraph
                            .execute(
                                "omnigraph.link_brain",
                                json!({
                                    "params": {},
                                    "query": {},
                                    "body": link_body,
                                }),
                            )
                            .await?
                        else {
                            return Err(HarnessError::unavailable(
                                "Unexpected omnigraph link payload.",
                            ));
                        };
                        let linked_atom = if linked["atom"].is_object() {
                            linked["atom"].clone()
                        } else if linked["atom_id"].is_string() {
                            linked
                        } else {
                            let mut patched = fetched["atom"].clone();
                            if let Some(obj) = patched.as_object_mut() {
                                obj.insert("brain_record_id".into(), json!(brain_record_id));
                                obj.insert("record_revision".into(), json!(revision));
                                obj.insert("origin_layer".into(), json!("project-brain"));
                                obj.insert("authority".into(), json!("verified"));
                                obj.insert("verified".into(), json!(true));
                                if let Some(p) = privacy {
                                    obj.insert("privacy".into(), json!(p));
                                }
                            }
                            patched
                        };
                        Payload::Json(json!({
                            "ok": true,
                            "atom_id": atom_id,
                            "branch": branch,
                            "brain_record_id": brain_record_id,
                            "record_type": promoted["record_type"],
                            "revision": revision,
                            "privacy": promoted["privacy"],
                            "origin_layer": "project-brain",
                            "authority": "verified",
                            "verified_files": promoted["verified_files"],
                            "atom": linked_atom,
                        }))
                    }
                    _ => {
                        let (group, _) = operation.split_once('.').ok_or_else(|| {
                            HarnessError::missing("Unknown native service operation.")
                        })?;
                        self.domains
                            .get(group)
                            .ok_or_else(|| {
                                HarnessError::unavailable("Required native service is unavailable.")
                            })?
                            .execute(operation, request.clone())
                            .await?
                    }
                }
            };
            if let Payload::Json(ref val) = payload
                && val["ok"] == true
            {
                if operation == "omnigraph.promote" {
                    let project_id = body["project_id"]
                        .as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .or_else(|| {
                            params["project_id"]
                                .as_str()
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                        })
                        .unwrap_or("");
                    self.best_effort_sync_knowledge(
                        project_id,
                        body["bank"].as_str(),
                        body["session_id"].as_str(),
                    )
                    .await;
                } else if operation == "knowledge.run"
                    && (matches!(
                        body["action"].as_str(),
                        Some(
                            "complete"
                                | "promote-apply"
                                | "promote-auto"
                                | "bank-retire"
                                | "brain-update"
                                | "record-result"
                                | "bank-reverify"
                        )
                    ) || (body["action"].as_str() == Some("brain-create")
                        && (val["result"]["authority"] == "verified"
                            || (val["result"].is_null() && body["authority"] == "verified"))))
                {
                    self.best_effort_sync_knowledge(
                        params["project_id"].as_str().unwrap_or(""),
                        body["bank"].as_str(),
                        None,
                    )
                    .await;
                } else if matches!(operation, "brain.act" | "memory.save")
                    && !sid.is_empty()
                    && let Ok(session) = self.store.get(sid)
                    && session["omnigraph"]["enabled"] == true
                    && let Some(project_id) = session["project_id"].as_str()
                {
                    self.best_effort_sync_knowledge(
                        project_id,
                        session["brain"]["bank"].as_str(),
                        Some(sid),
                    )
                    .await;
                }
            }
            self.emit_domain_mutation_sse(operation, params, body, &payload);
            Ok(payload)
        })
    }
}

#[cfg(test)]
mod sync_page_tests {
    use super::*;

    /// A knowledge domain serving `pages` in turn, by the requested offset.
    struct Pages(Vec<Value>, std::sync::Mutex<Vec<u64>>);
    impl Domain for Pages {
        fn execute<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a> {
            Box::pin(async move {
                assert_eq!(operation, "knowledge.sync_collect");
                let offset = request["body"]["offset"].as_u64().unwrap();
                self.1.lock().unwrap().push(offset);
                assert_eq!(request["body"]["bank"], "team");
                Ok(Payload::Json(
                    self.0
                        .iter()
                        .find(|page| page["offset"] == offset)
                        .cloned()
                        .unwrap(),
                ))
            })
        }
    }
    fn page(offset: u64, ids: &[&str], total: u64, next: Option<u64>, layers: &[&str]) -> Value {
        json!({"items": ids.iter().map(|id| json!({"atom_id": id})).collect::<Vec<_>>(), "offset": offset, "total_items": total, "has_more": next.is_some(), "next_offset": next, "complete_layers": layers, "brain_count": 2, "memory_count": 1, "skipped": 4})
    }

    #[tokio::test]
    async fn collects_every_page_and_prunes_only_whole_consistent_layers() {
        let body = json!({"project_id": "p", "bank": "team"});
        let both = ["project-brain", "memory-bank"];
        let whole = Pages(
            vec![
                page(0, &["brain-a", "brain-b"], 3, Some(2), &both),
                page(2, &["mem-c"], 3, None, &both),
            ],
            Default::default(),
        );
        let collected = collect_sync_items(&whole, "p", &json!({}), &body)
            .await
            .unwrap();
        assert_eq!(*whole.1.lock().unwrap(), vec![0, 2]);
        assert_eq!(
            collected["items"],
            json!([{"atom_id":"brain-a"},{"atom_id":"brain-b"},{"atom_id":"mem-c"}])
        );
        assert_eq!(
            collected["prune_layers"],
            json!(["memory-bank", "project-brain"])
        );
        assert_eq!(collected["brain_count"], 2);
        // A layer one page does not report whole is not pruned.
        let partial = Pages(
            vec![
                page(0, &["brain-a"], 2, Some(1), &both),
                page(1, &["mem-c"], 2, None, &["memory-bank"]),
            ],
            Default::default(),
        );
        let collected = collect_sync_items(&partial, "p", &json!({}), &body)
            .await
            .unwrap();
        assert_eq!(collected["prune_layers"], json!(["memory-bank"]));
        // Knowledge that changed between pages prunes nothing.
        let moved = Pages(
            vec![
                page(0, &["brain-a"], 2, Some(1), &both),
                page(1, &["brain-b", "mem-c"], 3, None, &both),
            ],
            Default::default(),
        );
        let collected = collect_sync_items(&moved, "p", &json!({}), &body)
            .await
            .unwrap();
        assert_eq!(collected["prune_layers"], json!([]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    struct StubDomain;
    impl Domain for StubDomain {
        fn execute<'a>(&'a self, _operation: &'a str, _request: Value) -> Reply<'a> {
            Box::pin(async { Ok(Payload::Json(json!({}))) })
        }
    }

    #[tokio::test]
    async fn graph_status_visit_and_projects_select_auto_start_unset_project() {
        let root = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&root.path().join("state")).unwrap();
        let folder = root.path().join("project");
        std::fs::create_dir_all(&folder).unwrap();
        let project = store.register(&folder).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let graphs = crate::graph::Graphs::with(
            store.clone(),
            Some(root.path().join("no-such-graphify")),
            Duration::ZERO,
        );

        let stub: Arc<dyn Domain> = Arc::new(StubDomain);
        let mut domains: BTreeMap<String, Arc<dyn Domain>> = [
            "sessions",
            "setup",
            "accelerators",
            "systems",
            "discovery",
            "creator",
            "skills",
            "results",
            "delivery",
            "brain",
            "context",
            "projects",
            "commands",
            "files",
            "memory",
            "updates",
            "providers",
            "knowledge",
            "omnigraph",
        ]
        .into_iter()
        .map(|name| (name.to_owned(), Arc::clone(&stub)))
        .collect();
        domains.insert("graph".to_owned(), graphs.clone());

        let close_graphs = graphs.clone();
        let services = NativeServices::new(
            store,
            domains,
            Arc::new(|| Ok(json!({}))),
            BTreeMap::new(),
            Arc::new(move || {
                let g = close_graphs.clone();
                Box::pin(async move {
                    g.close();
                    Ok(())
                })
            }),
        )
        .unwrap();

        // Direct Graphs::status stays "off" on untouched fixtures.
        assert_eq!(graphs.status(&project, true).unwrap()["state"], "off");

        // HTTP/service visit (check=1) auto-starts unset projects before returning status.
        let Payload::Json(status) = services
            .request(
                "graph.status",
                json!({
                    "params": {"project_id": project},
                    "query": {"check": "1"},
                    "body": {},
                }),
            )
            .await
            .unwrap()
        else {
            panic!("expected json status payload");
        };
        assert_eq!(status["enabled"], true, "{status}");
        assert_eq!(status["state"], "missing", "{status}");

        // Explicit disable blocks subsequent check=1 / projects.select auto-start.
        graphs.act(&project, "disable").unwrap();
        let Payload::Json(selected) = services
            .request(
                "projects.select",
                json!({
                    "params": {"project_id": project},
                    "query": {},
                    "body": {},
                }),
            )
            .await
            .unwrap()
        else {
            panic!("expected json select payload");
        };
        assert_eq!(selected["project"]["id"], project);
        assert_eq!(graphs.status(&project, true).unwrap()["state"], "off");
        assert!(crate::http::ROUTES.iter().any(|route| {
            route.method == "POST"
                && route.pattern == "/api/projects/{project_id}/select"
                && route.operation == "projects.select"
        }));

        services.close().await.unwrap();
    }

    #[derive(Clone, Default)]
    struct RecordingDomain {
        calls: Arc<std::sync::Mutex<Vec<(String, Value)>>>,
        promote_error: bool,
        sync_off: bool,
    }
    impl Domain for RecordingDomain {
        fn execute<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a> {
            let op = operation.to_owned();
            let calls = self.calls.clone();
            let promote_error = self.promote_error;
            let sync_off = self.sync_off;
            Box::pin(async move {
                calls.lock().unwrap().push((op.clone(), request.clone()));
                let val = match op.as_str() {
                    "knowledge.sync_collect" => json!({
                        "items": [
                            {"atom_id": "brain-b1", "kind": "finding", "title": "Brain rule"},
                            {"atom_id": "mem-MEM-0001", "kind": "insight", "title": "Bank chunk"}
                        ],
                        "brain_count": 1,
                        "memory_count": 1,
                        "skipped": 2
                    }),
                    "omnigraph.status" => {
                        if sync_off {
                            json!({"state": "unconfigured", "mode": "off"})
                        } else {
                            json!({"state": "connected", "mode": "external"})
                        }
                    }
                    "omnigraph.sync_items" => {
                        if sync_off {
                            json!({
                                "ok": false,
                                "skipped": "omnigraph-off",
                                "branch": "knowledge/canonical",
                                "synced": 0,
                                "atom_ids": []
                            })
                        } else {
                            json!({
                                "ok": true,
                                "branch": "knowledge/canonical",
                                "synced": 2,
                                "atom_ids": ["brain-b1", "mem-MEM-0001"]
                            })
                        }
                    }
                    "omnigraph.fetch_atom" => json!({
                        "branch": "session/s1",
                        "files": ["src/auth.rs"],
                        "atom": {
                            "atom_id": "atom-1",
                            "kind": "finding",
                            "title": "Validate token",
                            "summary": "Check exp claim",
                            "files": ["src/auth.rs"]
                        }
                    }),
                    "knowledge.promote_atom" => {
                        if promote_error {
                            return Err(HarnessError::conflict("Cited file missing."));
                        }
                        json!({
                            "record_id": "11111111-1111-4111-8111-111111111111",
                            "record_type": "finding",
                            "revision": 1,
                            "verified_files": ["src/auth.rs"],
                            "privacy": "team",
                            "authority": "verified",
                            "origin_layer": "project-brain"
                        })
                    }
                    "omnigraph.link_brain" => json!({
                        "atom": {
                            "atom_id": "atom-1",
                            "brain_record_id": "11111111-1111-4111-8111-111111111111",
                            "record_revision": 1,
                            "origin_layer": "project-brain",
                            "authority": "verified",
                            "verified": true
                        }
                    }),
                    "knowledge.run" | "brain.act" | "memory.save" => json!({"ok": true}),
                    _ => json!({}),
                };
                Ok(Payload::Json(val))
            })
        }
    }

    fn build_services_with_recording(
        store: SessionStore,
        rec: Arc<RecordingDomain>,
        override_memory_save: bool,
    ) -> NativeServices {
        let stub: Arc<dyn Domain> = Arc::new(StubDomain);
        let mut domains: BTreeMap<String, Arc<dyn Domain>> = [
            "sessions",
            "setup",
            "accelerators",
            "systems",
            "discovery",
            "creator",
            "skills",
            "results",
            "delivery",
            "brain",
            "context",
            "projects",
            "commands",
            "files",
            "memory",
            "updates",
            "providers",
            "knowledge",
            "omnigraph",
        ]
        .into_iter()
        .map(|name| (name.to_owned(), Arc::clone(&stub)))
        .collect();
        domains.insert("knowledge".into(), rec.clone());
        domains.insert("omnigraph".into(), rec.clone());
        domains.insert("brain".into(), rec.clone());
        let mut overrides = BTreeMap::new();
        if override_memory_save {
            overrides.insert("memory.save".into(), rec as Arc<dyn Domain>);
        }
        NativeServices::new(
            store,
            domains,
            Arc::new(|| Ok(json!({}))),
            overrides,
            Arc::new(|| Box::pin(async { Ok(()) })),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn omnigraph_sync_knowledge_promote_and_post_sync_wire_domains() {
        let root = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&root.path().join("state")).unwrap();
        let project_dir = root.path().join("project");
        std::fs::create_dir_all(&project_dir).unwrap();
        let project = store.register(&project_dir).unwrap();
        let pid = project["id"].as_str().unwrap().to_owned();
        let sess = store
            .create(
                &json!({
                    "project_id": pid,
                    "provider": "claude",
                    "mode": "edit",
                    "workflow": "native",
                    "project_context": false,
                    "omnigraph": {"enabled": true}
                }),
                "Sync test",
                &[],
            )
            .unwrap();
        let sid = sess["id"].as_str().unwrap().to_owned();

        let rec = Arc::new(RecordingDomain::default());
        let services = build_services_with_recording(store.clone(), rec.clone(), true);

        // 1. omnigraph.sync_knowledge
        let Payload::Json(sync_res) = services
            .request(
                "omnigraph.sync_knowledge",
                json!({"params":{},"query":{},"body":{"project_id": pid}}),
            )
            .await
            .unwrap()
        else {
            panic!("expected json");
        };
        assert_eq!(sync_res["ok"], true);
        assert_eq!(sync_res["branch"], "knowledge/canonical");
        assert_eq!(sync_res["synced"], 2);
        assert_eq!(sync_res["brain_count"], 1);
        assert_eq!(sync_res["memory_count"], 1);
        assert_eq!(sync_res["skipped"], 2);
        assert_eq!(sync_res["atom_ids"], json!(["brain-b1", "mem-MEM-0001"]));

        // 2. omnigraph.promote happy path
        let Payload::Json(promote_res) = services
            .request(
                "omnigraph.promote",
                json!({"params":{},"query":{},"body":{"project_id": pid, "atom_id": "atom-1"}}),
            )
            .await
            .unwrap()
        else {
            panic!("expected json");
        };
        assert_eq!(promote_res["ok"], true);
        assert_eq!(
            promote_res["brain_record_id"],
            "11111111-1111-4111-8111-111111111111"
        );
        assert_eq!(promote_res["record_type"], "finding");
        assert_eq!(promote_res["revision"], 1);
        assert_eq!(promote_res["origin_layer"], "project-brain");
        assert_eq!(promote_res["authority"], "verified");
        assert_eq!(promote_res["verified_files"], json!(["src/auth.rs"]));
        assert_eq!(promote_res["privacy"], "team");
        assert_eq!(
            promote_res["atom"]["brain_record_id"],
            "11111111-1111-4111-8111-111111111111"
        );
        {
            let recorded = rec.calls.lock().unwrap();
            let link_call = recorded
                .iter()
                .find(|(op, _)| op == "omnigraph.link_brain")
                .expect("link_brain called");
            assert_eq!(link_call.1["body"]["privacy"], "team");
            assert!(
                recorded.iter().any(|(op, _)| op == "omnigraph.sync_items"),
                "omnigraph.promote must trigger canonical sync"
            );
        }

        // 3. Best-effort sync triggers after knowledge.run(complete|brain-create verified|bank-reverify), brain.act, and memory.save
        rec.calls.lock().unwrap().clear();
        services
            .request(
                "knowledge.run",
                json!({"params":{"project_id": pid},"query":{},"body":{"action":"status"}}),
            )
            .await
            .unwrap();
        assert_eq!(
            rec.calls.lock().unwrap().len(),
            1,
            "status must not trigger sync"
        );

        rec.calls.lock().unwrap().clear();
        services
            .request(
                "knowledge.run",
                json!({"params":{"project_id": pid},"query":{},"body":{"action":"brain-create","authority":"observed"}}),
            )
            .await
            .unwrap();
        assert_eq!(
            rec.calls.lock().unwrap().len(),
            1,
            "unverified brain-create must not trigger sync"
        );

        for body in [
            json!({"action":"complete"}),
            json!({"action":"brain-create","authority":"verified"}),
            json!({"action":"bank-reverify"}),
        ] {
            rec.calls.lock().unwrap().clear();
            services
                .request(
                    "knowledge.run",
                    json!({"params":{"project_id": pid},"query":{},"body": body}),
                )
                .await
                .unwrap();
            let ops: Vec<String> = rec
                .calls
                .lock()
                .unwrap()
                .iter()
                .map(|(op, _)| op.clone())
                .collect();
            assert!(
                ops.contains(&"knowledge.sync_collect".to_string()),
                "{ops:?}"
            );
            assert!(ops.contains(&"omnigraph.sync_items".to_string()), "{ops:?}");
        }

        for op in ["brain.act", "memory.save"] {
            rec.calls.lock().unwrap().clear();
            services
                .request(
                    op,
                    json!({"params":{"session_id": sid},"query":{},"body":{"action":"brain-update"}}),
                )
                .await
                .unwrap();
            let ops: Vec<String> = rec
                .calls
                .lock()
                .unwrap()
                .iter()
                .map(|(op, _)| op.clone())
                .collect();
            assert!(
                ops.contains(&"knowledge.sync_collect".to_string()),
                "{op}: {ops:?}"
            );
            assert!(
                ops.contains(&"omnigraph.sync_items".to_string()),
                "{op}: {ops:?}"
            );
        }

        // 4. omnigraph.sync_knowledge preserves omnigraph-off skip reason and error message
        let off_rec = Arc::new(RecordingDomain {
            sync_off: true,
            ..Default::default()
        });
        let off_services = build_services_with_recording(store.clone(), off_rec, false);
        let Payload::Json(off_res) = off_services
            .request(
                "omnigraph.sync_knowledge",
                json!({"params":{},"query":{},"body":{"project_id": pid}}),
            )
            .await
            .unwrap()
        else {
            panic!("expected json");
        };
        assert_eq!(off_res["ok"], false);
        assert_eq!(off_res["skipped"], "omnigraph-off");
        assert!(
            off_res["error"]
                .as_str()
                .is_some_and(|s| s.contains("Omnigraph is off")),
            "{off_res}"
        );
        assert_eq!(off_res["message"], off_res["error"]);

        // 5. Promote error aborts before omnigraph.link_brain
        let failing_rec = Arc::new(RecordingDomain {
            calls: Arc::new(std::sync::Mutex::new(Vec::new())),
            promote_error: true,
            sync_off: false,
        });
        let failing_services = build_services_with_recording(store, failing_rec.clone(), false);
        let err = failing_services
            .request(
                "omnigraph.promote",
                json!({"params":{},"query":{},"body":{"project_id": pid, "atom_id": "atom-1"}}),
            )
            .await
            .err()
            .expect("promote error");
        assert_eq!(err.status, 409);
        let ops: Vec<String> = failing_rec
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|(op, _)| op.clone())
            .collect();
        assert!(
            !ops.contains(&"omnigraph.link_brain".to_string()),
            "{ops:?}"
        );

        services.close().await.unwrap();
        off_services.close().await.unwrap();
        failing_services.close().await.unwrap();
    }

    struct WaitingAndBatchDomain;
    impl Domain for WaitingAndBatchDomain {
        fn execute<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a> {
            let op = operation.to_owned();
            Box::pin(async move {
                let val = match op.as_str() {
                    "sessions.waiting_counts" => json!({"counts": {"s1": 2}}),
                    "sessions.waiting_permissions" => {
                        let sid = request["params"]["session_id"].as_str().unwrap_or("");
                        if sid == "s1" {
                            json!({"items": [{"kind":"permission","state":"pending","request_id":"req-1"}]})
                        } else {
                            json!({"items": []})
                        }
                    }
                    "knowledge.memory_links" => {
                        let path = request["query"]["path"].as_str().unwrap_or("");
                        json!({
                            "project_id": request["params"]["project_id"],
                            "path": path,
                            "brain_records": [],
                            "memory_chunks": [],
                            "hygiene_warnings": [],
                            "contradiction_warnings": [],
                        })
                    }
                    "knowledge.memory_links_batch" => json!({
                        "project_id": request["params"]["project_id"],
                        "items": [
                            {
                                "project_id": request["params"]["project_id"],
                                "path": "src/ok.rs",
                                "brain_records": [],
                                "memory_chunks": [],
                                "hygiene_warnings": [],
                                "contradiction_warnings": [],
                            },
                            {
                                "project_id": request["params"]["project_id"],
                                "path": "src/skip.rs",
                                "brain_records": [],
                                "memory_chunks": [],
                                "hygiene_warnings": [],
                                "contradiction_warnings": [],
                            },
                            {
                                "project_id": request["params"]["project_id"],
                                "path": "src/err.rs",
                                "brain_records": [],
                                "memory_chunks": [],
                                "hygiene_warnings": [],
                                "contradiction_warnings": [],
                            },
                        ]
                    }),
                    "omnigraph.file_context" => {
                        match request["query"]["path"].as_str().unwrap_or("") {
                            "src/ok.rs" => json!({
                                "ok": true,
                                "atoms": [
                                    {"atom_id": "brain-a1", "title": "Claim A", "brain_record_id": "rec-a"},
                                    {"atom_id": "mem-MEM-1", "title": "Claim B", "memory_id": "MEM-1"}
                                ],
                                "contradictions": [{
                                    "source_id": "brain-a1",
                                    "source_title": "Claim A",
                                    "target_id": "mem-MEM-1",
                                    "target_title": "Claim B",
                                }],
                            }),
                            "src/skip.rs" => json!({"ok": false, "skipped": "omnigraph-off"}),
                            "src/err.rs" => {
                                return Err(HarnessError::unavailable("Omnigraph offline."));
                            }
                            _ => json!({"ok": true, "atoms": [], "contradictions": []}),
                        }
                    }
                    _ => json!({}),
                };
                Ok(Payload::Json(val))
            })
        }
    }

    #[tokio::test]
    async fn sessions_waiting_permissions_and_batch_memory_links_enrich_responses() {
        let root = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&root.path().join("state")).unwrap();
        let project_dir = root.path().join("project");
        std::fs::create_dir_all(&project_dir).unwrap();
        let project = store.register(&project_dir).unwrap();
        let pid = project["id"].as_str().unwrap().to_owned();
        {
            let db = store.connection().unwrap();
            for sid in ["s1", "s2"] {
                db.execute(
                    "INSERT INTO sessions(id,title,project_id,project_path,provider,mode,workflow,project_context,status,created_at,updated_at) VALUES(?1,'Example',?2,?3,'claude','edit','native',0,'running','now','now')",
                    rusqlite::params![sid, pid, project_dir.to_string_lossy().as_ref()],
                )
                .unwrap();
            }
        }
        let domain: Arc<dyn Domain> = Arc::new(WaitingAndBatchDomain);
        let stub: Arc<dyn Domain> = Arc::new(StubDomain);
        let mut domains: BTreeMap<String, Arc<dyn Domain>> = [
            "setup",
            "accelerators",
            "systems",
            "discovery",
            "creator",
            "skills",
            "results",
            "delivery",
            "brain",
            "context",
            "projects",
            "commands",
            "files",
            "memory",
            "updates",
            "providers",
        ]
        .into_iter()
        .map(|name| (name.to_owned(), Arc::clone(&stub)))
        .collect();
        domains.insert("sessions".into(), Arc::clone(&domain));
        domains.insert("knowledge".into(), Arc::clone(&domain));
        domains.insert("omnigraph".into(), Arc::clone(&domain));
        let boot_store = store.clone();
        let services = NativeServices::new(
            store.clone(),
            domains,
            Arc::new(move || Ok(json!({"sessions": boot_store.summaries()?}))),
            BTreeMap::new(),
            Arc::new(|| Box::pin(async { Ok(()) })),
        )
        .unwrap();

        let Payload::Json(bootstrapped) = services
            .request("bootstrap", json!({"params":{},"query":{},"body":{}}))
            .await
            .unwrap()
        else {
            panic!("expected json");
        };
        let boot_sessions = bootstrapped["sessions"].as_array().unwrap();
        let boot_s1 = boot_sessions.iter().find(|s| s["id"] == "s1").unwrap();
        let boot_s2 = boot_sessions.iter().find(|s| s["id"] == "s2").unwrap();
        assert_eq!(boot_s1["waiting_permissions"], 2);
        assert_eq!(boot_s2["waiting_permissions"], 0);

        let Payload::Json(listed) = services
            .request("sessions.list", json!({"params":{},"query":{},"body":{}}))
            .await
            .unwrap()
        else {
            panic!("expected json");
        };
        let summaries = listed["sessions"].as_array().unwrap();
        let s1 = summaries.iter().find(|s| s["id"] == "s1").unwrap();
        let s2 = summaries.iter().find(|s| s["id"] == "s2").unwrap();
        assert_eq!(s1["waiting_permissions"], 2);
        assert_eq!(s2["waiting_permissions"], 0);

        let Payload::Json(got_s1) = services
            .request(
                "sessions.get",
                json!({"params":{"session_id":"s1"},"query":{},"body":{}}),
            )
            .await
            .unwrap()
        else {
            panic!("expected json");
        };
        assert_eq!(got_s1["waiting_permissions"][0]["request_id"], "req-1");
        let Payload::Json(got_s2) = services
            .request(
                "sessions.get",
                json!({"params":{"session_id":"s2"},"query":{},"body":{}}),
            )
            .await
            .unwrap()
        else {
            panic!("expected json");
        };
        assert_eq!(got_s2["waiting_permissions"], json!([]));

        let Payload::Json(single) = services
            .request(
                "knowledge.memory_links",
                json!({"params":{"project_id":pid},"query":{"path":"src/ok.rs"},"body":{}}),
            )
            .await
            .unwrap()
        else {
            panic!("expected json");
        };
        assert_eq!(single["omnigraph_atoms"].as_array().unwrap().len(), 2);
        assert_eq!(
            single["contradiction_warnings"].as_array().unwrap().len(),
            1
        );

        let Payload::Json(batch) = services
            .request(
                "knowledge.memory_links_batch",
                json!({"params":{"project_id":pid},"query":{},"body":{"files":[]}}),
            )
            .await
            .unwrap()
        else {
            panic!("expected json");
        };
        let items = batch["items"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["omnigraph_atoms"], single["omnigraph_atoms"]);
        assert_eq!(
            items[0]["contradiction_warnings"],
            single["contradiction_warnings"]
        );
        assert_eq!(items[1]["omnigraph_atoms"], json!([]));
        assert_eq!(items[1]["omnigraph_skipped"], "omnigraph-off");
        assert_eq!(items[2]["omnigraph_atoms"], json!([]));
        assert_eq!(items[2]["omnigraph_error"], "Omnigraph offline.");

        // Stubbed sessions domain safely degrades waiting_permissions to 0 / [].
        let stub_services =
            build_services_with_recording(store, Arc::new(RecordingDomain::default()), false);
        let Payload::Json(stub_list) = stub_services
            .request("sessions.list", json!({"params":{},"query":{},"body":{}}))
            .await
            .unwrap()
        else {
            panic!("expected json");
        };
        assert_eq!(stub_list["sessions"][0]["waiting_permissions"], 0);
        let Payload::Json(stub_get) = stub_services
            .request(
                "sessions.get",
                json!({"params":{"session_id":"s1"},"query":{},"body":{}}),
            )
            .await
            .unwrap()
        else {
            panic!("expected json");
        };
        assert_eq!(stub_get["waiting_permissions"], json!([]));

        services.close().await.unwrap();
        stub_services.close().await.unwrap();
    }

    struct PreflightAndDeliveryDomain {
        fail_knowledge: bool,
        fail_omnigraph: bool,
    }
    impl Domain for PreflightAndDeliveryDomain {
        fn execute<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a> {
            let op = operation.to_owned();
            let fail_knowledge = self.fail_knowledge;
            let fail_omnigraph = self.fail_omnigraph;
            Box::pin(async move {
                if op == "knowledge.preflight" {
                    if fail_knowledge {
                        return Err(HarnessError::unavailable("Knowledge offline."));
                    }
                    let pid = request["params"]["project_id"].clone();
                    return Ok(Payload::Json(json!({
                        "project_id": pid,
                        "files": [
                            {
                                "path": "src/ok.rs",
                                "brain_records": [],
                                "memory_chunks": [],
                                "hygiene_warnings": [],
                                "contradiction_warnings": [],
                            },
                            {
                                "path": "src/missing.rs",
                                "brain_records": [],
                                "memory_chunks": [],
                                "hygiene_warnings": [{
                                    "kind": "cited-file-missing",
                                    "path": "src/missing.rs",
                                    "linked": ["rec-m"],
                                    "recommended_action": "review",
                                    "text": "Missing file",
                                }],
                                "contradiction_warnings": [],
                            },
                            {
                                "path": "src/err.rs",
                                "brain_records": [],
                                "memory_chunks": [],
                                "hygiene_warnings": [],
                                "contradiction_warnings": [],
                            },
                        ],
                        "contradiction_warnings": [],
                        "hygiene_warnings": [{
                            "kind": "cited-file-missing",
                            "path": "src/missing.rs",
                            "linked": ["rec-m"],
                            "recommended_action": "review",
                            "text": "Missing file",
                        }],
                        "risk_callers": [{
                            "from": "src/caller.rs",
                            "to": "src/ok.rs",
                            "impact": "impacted",
                        }],
                        "blast_summary": {
                            "direct_count": 1,
                            "downstream_count": 0,
                            "files": [{"path": "src/caller.rs", "hop": 1, "left": 0.85}],
                        },
                        "risk_caller_hints": [],
                    })));
                }
                if op == "omnigraph.file_context" {
                    if fail_omnigraph {
                        return Err(HarnessError::unavailable("Omnigraph offline."));
                    }
                    return match request["query"]["path"].as_str().unwrap_or("") {
                        "src/ok.rs" => Ok(Payload::Json(json!({
                            "ok": true,
                            "atoms": [
                                {"atom_id": "brain-a1", "title": "Claim A", "brain_record_id": "rec-a"},
                                {"atom_id": "mem-MEM-1", "title": "Claim B", "memory_id": "MEM-1"}
                            ],
                            "contradictions": [{
                                "source_id": "brain-a1",
                                "source_title": "Claim A",
                                "target_id": "mem-MEM-1",
                                "target_title": "Claim B",
                            }],
                        }))),
                        "src/err.rs" => Err(HarnessError::unavailable("Omnigraph file timeout.")),
                        _ => Ok(Payload::Json(
                            json!({"ok": true, "atoms": [], "contradictions": []}),
                        )),
                    };
                }
                if op == "delivery.preview_commit"
                    || (op == "delivery.act" && request["body"]["action"] == "preview_commit")
                {
                    return Ok(Payload::Json(json!({
                        "kind": "commit",
                        "can_apply": true,
                        "files": [
                            {"status": "M", "path": "src/ok.rs"},
                            {"status": "D", "path": "src/missing.rs"},
                        ],
                        "preview_id": "p-commit",
                    })));
                }
                if op == "delivery.preview_transfer"
                    || (op == "delivery.act" && request["body"]["action"] == "preview_transfer")
                {
                    return Ok(Payload::Json(json!({
                        "kind": "transfer",
                        "can_apply": true,
                        "files": [{"status": "M", "path": "src/ok.rs"}],
                        "preview_id": "p-transfer",
                    })));
                }
                if op == "delivery.get" {
                    return Ok(Payload::Json(json!({
                        "available": true,
                        "preview": {
                            "kind": "transfer",
                            "can_apply": true,
                            "files": [{"status": "M", "path": "src/ok.rs"}],
                            "preview_id": "p-current",
                        },
                    })));
                }
                Ok(Payload::Json(json!({})))
            })
        }
    }

    #[tokio::test]
    async fn knowledge_preflight_and_delivery_previews_enrich_and_degrade_safely() {
        let root = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&root.path().join("state")).unwrap();
        let project_dir = root.path().join("project");
        std::fs::create_dir_all(&project_dir).unwrap();
        let project = store.register(&project_dir).unwrap();
        let pid = project["id"].as_str().unwrap().to_owned();
        {
            let db = store.connection().unwrap();
            db.execute(
                "INSERT INTO sessions(id,title,project_id,project_path,provider,mode,workflow,project_context,status,created_at,updated_at) VALUES('s1','Example',?1,?2,'claude','edit','native',0,'completed','now','now')",
                rusqlite::params![pid, project_dir.to_string_lossy().as_ref()],
            )
            .unwrap();
        }
        let make_services = |fail_knowledge: bool, fail_omnigraph: bool| {
            let domain: Arc<dyn Domain> = Arc::new(PreflightAndDeliveryDomain {
                fail_knowledge,
                fail_omnigraph,
            });
            let stub: Arc<dyn Domain> = Arc::new(StubDomain);
            let mut domains: BTreeMap<String, Arc<dyn Domain>> = [
                "sessions",
                "setup",
                "accelerators",
                "systems",
                "discovery",
                "creator",
                "skills",
                "results",
                "brain",
                "context",
                "projects",
                "commands",
                "files",
                "memory",
                "updates",
                "providers",
            ]
            .into_iter()
            .map(|name| (name.to_owned(), Arc::clone(&stub)))
            .collect();
            domains.insert("knowledge".into(), Arc::clone(&domain));
            domains.insert("omnigraph".into(), Arc::clone(&domain));
            domains.insert("delivery".into(), Arc::clone(&domain));
            NativeServices::new(
                store.clone(),
                domains,
                Arc::new(|| Ok(json!({}))),
                BTreeMap::new(),
                Arc::new(|| Box::pin(async { Ok(()) })),
            )
            .unwrap()
        };

        let services = make_services(false, false);

        // 1. knowledge.preflight enriches files via omnigraph.file_context and aggregates contradiction_warnings.
        let Payload::Json(pf) = services
            .request(
                "knowledge.preflight",
                json!({"params":{"project_id":pid},"query":{},"body":{"paths":["src/ok.rs","src/missing.rs","src/err.rs"]}}),
            )
            .await
            .unwrap()
        else {
            panic!("expected json");
        };
        assert_eq!(
            pf["files"][0]["omnigraph_atoms"].as_array().unwrap().len(),
            2
        );
        assert_eq!(
            pf["files"][0]["contradiction_warnings"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(pf["files"][2]["omnigraph_error"], "Omnigraph file timeout.");
        assert_eq!(pf["contradiction_warnings"].as_array().unwrap().len(), 1);
        assert_eq!(pf["contradiction_warnings"][0]["path"], "src/ok.rs");
        assert_eq!(pf["hygiene_warnings"].as_array().unwrap().len(), 1);

        // 2. delivery.act (preview_commit & preview_transfer), delivery.preview_*, and delivery.get annotate memory_warnings + risk_callers.
        for (op, body) in [
            (
                "delivery.act",
                json!({"action": "preview_commit", "snapshot_id": "s", "paths": ["src/ok.rs"], "message": "m"}),
            ),
            (
                "delivery.act",
                json!({"action": "preview_transfer", "commit": "a", "target_branch": "main"}),
            ),
            ("delivery.preview_commit", json!({})),
            ("delivery.preview_transfer", json!({})),
        ] {
            let Payload::Json(preview) = services
                .request(
                    op,
                    json!({"params":{"session_id":"s1"},"query":{},"body":body}),
                )
                .await
                .unwrap()
            else {
                panic!("expected json");
            };
            assert_eq!(
                preview["memory_warnings"]["contradiction_warnings"]
                    .as_array()
                    .unwrap()
                    .len(),
                1,
                "{op}: {preview}"
            );
            assert_eq!(
                preview["memory_warnings"]["hygiene_warnings"]
                    .as_array()
                    .unwrap()
                    .len(),
                1,
                "{op}: {preview}"
            );
            assert_eq!(preview["risk_callers"][0]["from"], "src/caller.rs");
        }
        let Payload::Json(got_delivery) = services
            .request(
                "delivery.get",
                json!({"params":{"session_id":"s1"},"query":{},"body":{}}),
            )
            .await
            .unwrap()
        else {
            panic!("expected json");
        };
        assert_eq!(
            got_delivery["preview"]["memory_warnings"]["contradiction_warnings"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        // 3. Non-blocking degradation when both Knowledge and Omnigraph fail, or when session is unknown.
        let degraded = make_services(true, true);
        let Payload::Json(preview_ok) = degraded
            .request(
                "delivery.act",
                json!({"params":{"session_id":"s1"},"query":{},"body":{"action":"preview_commit"}}),
            )
            .await
            .unwrap()
        else {
            panic!("expected json");
        };
        assert_eq!(
            preview_ok["memory_warnings"],
            json!({"contradiction_warnings": [], "hygiene_warnings": []})
        );
        assert_eq!(preview_ok["risk_callers"], json!([]));

        let Payload::Json(unknown_sess) = services
            .request(
                "delivery.preview_transfer",
                json!({"params":{"session_id":"missing-sid"},"query":{},"body":{}}),
            )
            .await
            .unwrap()
        else {
            panic!("expected json");
        };
        assert_eq!(
            unknown_sess["memory_warnings"],
            json!({"contradiction_warnings": [], "hygiene_warnings": []})
        );
        assert_eq!(unknown_sess["risk_callers"], json!([]));

        services.close().await.unwrap();
        degraded.close().await.unwrap();
    }
}
