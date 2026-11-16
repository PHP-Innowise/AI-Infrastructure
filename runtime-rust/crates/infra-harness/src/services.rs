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
impl Services for NativeServices {
    fn close<'a>(&'a self) -> CloseFuture<'a> {
        (self.closer)()
    }
    fn request<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a> {
        Box::pin(async move {
            if let Some(domain) = self.overrides.get(operation) {
                return domain.execute(operation, request).await;
            }
            let params = &request["params"];
            let query = &request["query"];
            let body = &request["body"];
            let sid = params["session_id"].as_str().unwrap_or("");
            match operation {
                "bootstrap" => {
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
                    Ok(Payload::Json(data))
                }
                "sessions.list" => {
                    if let Some(project_id) = query["project_id"]
                        .as_str()
                        .or_else(|| params["project_id"].as_str())
                        .filter(|id| !id.is_empty())
                    {
                        self.auto_start_graph(project_id).await;
                    }
                    Ok(Payload::Json(json!({"sessions":self.store.summaries()?})))
                }
                "sessions.get" => {
                    let after = query["after"]
                        .as_str()
                        .unwrap_or("0")
                        .parse::<i64>()
                        .map_err(|_| HarnessError::invalid("Invalid event cursor."))?;
                    let (mut session, events) = self.store.session_events(sid, after)?;
                    session["graph_focus"] =
                        crate::graph_live::session_focus_value(&self.store, sid)?;
                    if after == 0
                        && let Some(project_id) = session["project_id"].as_str()
                    {
                        self.auto_start_graph(project_id).await;
                    }
                    Ok(Payload::Json(json!({"session":session,"events":events})))
                }
                "sessions.merge_archive" => Ok(Payload::Json(self.store.merge_archive(sid)?)),
                "sessions.delete_merge" => {
                    if !crate::store::exact(body, &[], &[]) {
                        return Err(HarnessError::invalid(
                            "Invalid saved context deletion request.",
                        ));
                    }
                    Ok(Payload::Json(
                        json!({"session":self.store.delete_merge(sid)?}),
                    ))
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
                    Ok(Payload::Bytes {
                        bytes: text.as_bytes().to_vec(),
                        content_type: "text/markdown; charset=utf-8".into(),
                        filename: None,
                        etag: None,
                        cache: None,
                    })
                }
                "attachments.read" => {
                    let (item, bytes, _) = crate::attachments::read(
                        &self.store,
                        sid,
                        params["id"].as_str().unwrap_or(""),
                    )?;
                    Ok(Payload::Bytes {
                        bytes,
                        content_type: "application/octet-stream".into(),
                        filename: item["name"].as_str().map(String::from),
                        etag: None,
                        cache: None,
                    })
                }
                "projects.list" => Ok(Payload::Json(
                    json!({"projects":(self.bootstrap)()?["projects"]}),
                )),
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
                    Ok(Payload::Json(
                        json!({"project":project,"projects":(self.bootstrap)()?["projects"]}),
                    ))
                }
                "projects.select" => {
                    let project_id = params["project_id"]
                        .as_str()
                        .or_else(|| body["project_id"].as_str())
                        .ok_or_else(|| HarnessError::invalid("Invalid project selection."))?;
                    let project = self.store.project(project_id)?;
                    self.auto_start_graph(project_id).await;
                    Ok(Payload::Json(json!({"project":project})))
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
                        .execute(operation, request)
                        .await
                }
                "sdd.artifacts" => {
                    let session = self.store.get(sid)?;
                    if session["sdd"].is_null() {
                        return Err(HarnessError::invalid("This session does not use SDD."));
                    }
                    Ok(Payload::Json(
                        json!({"session_id":sid,"sdd":session["sdd"],"files":crate::sdd::artifacts(std::path::Path::new(session["project_path"].as_str().unwrap_or("")),&session["sdd"])?}),
                    ))
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
                    Ok(payload)
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
                    Ok(payload)
                }
                "omnigraph.import" => {
                    let domain = self.domains.get("omnigraph").ok_or_else(|| {
                        HarnessError::unavailable("Required native service is unavailable.")
                    })?;
                    let mut payload = domain.execute(operation, request.clone()).await?;
                    if let Payload::Json(ref mut val) = payload {
                        let sessions_domain = self.domains.get("sessions").map(Arc::as_ref);
                        let updated_sessions = crate::sessions::sync_omnigraph_import_to_sessions(
                            &self.store,
                            sessions_domain,
                            &request["body"],
                            val,
                        )
                        .await;
                        if !updated_sessions.is_empty() {
                            val["session"] = updated_sessions[0].clone();
                            val["sessions"] = Value::Array(updated_sessions);
                        }
                    }
                    Ok(payload)
                }
                "omnigraph.link_sessions" => {
                    let domain = self.domains.get("omnigraph").ok_or_else(|| {
                        HarnessError::unavailable("Required native service is unavailable.")
                    })?;
                    let mut payload = domain.execute(operation, request.clone()).await?;
                    if let Payload::Json(ref mut val) = payload {
                        let sessions_domain = self.domains.get("sessions").map(Arc::as_ref);
                        let updated_sessions = crate::sessions::sync_omnigraph_link_to_sessions(
                            &self.store,
                            sessions_domain,
                            &request["body"],
                            val,
                        )
                        .await;
                        if let Some(first) = updated_sessions.first() {
                            val["session"] = first.clone();
                        }
                        val["sessions"] = Value::Array(updated_sessions);
                    }
                    Ok(payload)
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
                        .execute(operation, request)
                        .await
                }
            }
        })
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
}
