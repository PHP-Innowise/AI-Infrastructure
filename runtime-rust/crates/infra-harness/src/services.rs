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
                "bootstrap" => Ok(Payload::Json((self.bootstrap)()?)),
                "sessions.list" => Ok(Payload::Json(json!({"sessions":self.store.summaries()?}))),
                "sessions.get" => {
                    let after = query["after"]
                        .as_str()
                        .unwrap_or("0")
                        .parse::<i64>()
                        .map_err(|_| HarnessError::invalid("Invalid event cursor."))?;
                    Ok(Payload::Json(
                        json!({"session":self.store.get(sid)?,"events":self.store.events(sid,after)?}),
                    ))
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
                    Ok(Payload::Json(
                        json!({"project":project,"projects":(self.bootstrap)()?["projects"]}),
                    ))
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
