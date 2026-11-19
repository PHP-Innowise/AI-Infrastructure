use crate::{
    HarnessError, Result,
    services::{Payload, Services},
    store::{self, ReplayStart, StreamFrame},
};
use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::State,
    http::{HeaderMap, Request, Response, StatusCode, header},
};
use flate2::{Compression, write::GzEncoder};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    io::Write as _,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, Mutex, OnceLock},
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
#[derive(Clone)]
pub struct App {
    pub services: Arc<dyn Services>,
    pub token: String,
    pub instance: String,
    pub port: u16,
    pub source: PathBuf,
    pub shutdown: tokio::sync::watch::Sender<bool>,
    pub assets: Arc<BTreeMap<String, Asset>>,
}
#[derive(Clone)]
pub struct Asset {
    pub bytes: Arc<[u8]>,
    pub gzip_bytes: Option<Arc<[u8]>>,
    pub br_bytes: Option<Arc<[u8]>>,
    pub content_type: String,
    pub etag: String,
    pub gzip_etag: Option<String>,
    pub br_etag: Option<String>,
    pub version: String,
}

fn compress_gzip(raw: &[u8]) -> Option<Arc<[u8]>> {
    if raw.is_empty() {
        return None;
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(raw).ok()?;
    let compressed = encoder.finish().ok()?;
    Some(Arc::from(compressed))
}

fn compress_brotli(raw: &[u8]) -> Option<Arc<[u8]>> {
    if raw.is_empty() {
        return None;
    }
    let params = brotli::enc::BrotliEncoderParams {
        quality: 6,
        lgwin: 22,
        ..Default::default()
    };
    let mut compressed = Vec::new();
    brotli::BrotliCompress(&mut std::io::Cursor::new(raw), &mut compressed, &params).ok()?;
    Some(Arc::from(compressed))
}

impl Asset {
    /// Builds an immutable static HTTP asset alongside its pre-compressed
    /// `br` and `gzip` representations and distinct representation ETags.
    pub fn from_bytes(raw: Vec<u8>, content_type: impl Into<String>) -> Self {
        let digest = store::sha(&raw);
        let etag = format!("\"{}\"", &digest[..32]);
        let version: String = digest[..16].into();
        let gzip_bytes = compress_gzip(&raw);
        let gzip_etag = gzip_bytes
            .as_ref()
            .map(|_| format!("\"{}-gzip\"", &digest[..32]));
        let br_bytes = compress_brotli(&raw);
        let br_etag = br_bytes
            .as_ref()
            .map(|_| format!("\"{}-br\"", &digest[..32]));
        Self {
            bytes: Arc::from(raw),
            gzip_bytes,
            br_bytes,
            content_type: content_type.into(),
            etag,
            gzip_etag,
            br_etag,
            version,
        }
    }

    fn set_bytes(&mut self, raw: Vec<u8>) {
        let updated = Self::from_bytes(raw, self.content_type.clone());
        self.bytes = updated.bytes;
        self.gzip_bytes = updated.gzip_bytes;
        self.br_bytes = updated.br_bytes;
        self.etag = updated.etag;
        self.gzip_etag = updated.gzip_etag;
        self.br_etag = updated.br_etag;
        self.version = updated.version;
    }
}
impl App {
    pub fn new(
        services: Arc<dyn Services>,
        port: u16,
        source: PathBuf,
        shutdown: tokio::sync::watch::Sender<bool>,
    ) -> Result<Self> {
        let token = format!("{}{}", store::hexid(), store::hexid());
        let mut assets = BTreeMap::new();
        let mut pins = vec![];
        let directory = source.join("harness/web");
        type Pin = (
            infra_core::bank::publication::PublicationFiles,
            String,
            Option<(u64, u64, u32, u64, u64, i128, i128)>,
        );
        fn asset(
            path: &std::path::Path,
            mime: &str,
            limit: usize,
            pins: &mut Vec<Pin>,
        ) -> Result<Asset> {
            let parent = path
                .parent()
                .ok_or_else(|| HarnessError::invalid("Static asset has no parent."))?;
            let name = path
                .file_name()
                .and_then(|v| v.to_str())
                .ok_or_else(|| HarnessError::invalid("Static asset name is not UTF-8."))?;
            let mut files = infra_core::bank::publication::PublicationFiles::new(parent)?;
            let before = files.image_identity(name)?;
            let bytes = files.read(name, limit)?.ok_or_else(|| {
                HarnessError::missing(format!(
                    "Required native UI asset is missing: {}",
                    path.display()
                ))
            })?;
            let digest = store::sha(&bytes);
            if files.image_identity(name)? != before {
                return Err(HarnessError::conflict(
                    "Native UI asset changed during startup.",
                ));
            }
            pins.push((files, name.into(), before));
            let _ = digest;
            Ok(Asset::from_bytes(bytes, mime))
        }
        for (name, mime) in [
            ("index.html", "text/html; charset=utf-8"),
            ("app.css", "text/css; charset=utf-8"),
            ("boot.js", "text/javascript; charset=utf-8"),
            ("icons/ai-accelerator.svg", "image/svg+xml"),
        ] {
            assets.insert(
                format!("/{name}"),
                asset(&directory.join(name), mime, 16 * 1024 * 1024, &mut pins)?,
            );
        }
        let wasm = source.join("runtime-rust/target/wasm-ui");
        assets.insert(
            "/assets/infra_ui.js".into(),
            asset(
                &wasm.join("infra_ui.js"),
                "text/javascript; charset=utf-8",
                8 * 1024 * 1024,
                &mut pins,
            )?,
        );
        assets.insert(
            "/assets/infra_ui_bg.wasm".into(),
            asset(
                &wasm.join("infra_ui_bg.wasm"),
                "application/wasm",
                64 * 1024 * 1024,
                &mut pins,
            )?,
        );
        let manifest = asset(
            &wasm.join("abi-manifest.json"),
            "application/json",
            65536,
            &mut pins,
        )?;
        let manifest = crate::strict_json::parse(&manifest.bytes, 65536)?;
        if !crate::store::exact(
            &manifest,
            &["schema_version", "js_sha256", "wasm_sha256"],
            &["source_sha256"],
        ) || manifest["schema_version"] != 1
            || !manifest.get("source_sha256").is_none_or(|digest| {
                digest.as_str().is_some_and(|digest| {
                    digest.len() == 64
                        && digest
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                })
            })
            || manifest["js_sha256"] != store::sha(&assets["/assets/infra_ui.js"].bytes)
            || manifest["wasm_sha256"] != store::sha(&assets["/assets/infra_ui_bg.wasm"].bytes)
        {
            return Err(HarnessError::conflict(
                "The local application assets do not form one published version. Finish the UI build or reinstall this release.",
            ));
        }
        let js = assets["/assets/infra_ui.js"].version.clone();
        let binary = assets["/assets/infra_ui_bg.wasm"].version.clone();
        let boot = assets
            .get_mut("/boot.js")
            .ok_or_else(|| HarnessError::missing("Native UI loader is missing."))?;
        let boot_text = std::str::from_utf8(&boot.bytes)
            .map_err(|_| HarnessError::invalid("Native UI loader is not UTF-8."))?
            .replace(
                "/assets/infra_ui.js",
                &format!("/assets/infra_ui.js?v={js}"),
            )
            .replace(
                "/assets/infra_ui_bg.wasm",
                &format!("/assets/infra_ui_bg.wasm?v={binary}"),
            );
        boot.set_bytes(boot_text.into_bytes());
        let kit_bytes = infra_tooling::kit::http_catalog(&source)?;
        let mut kit = Asset::from_bytes(kit_bytes, "text/html; charset=utf-8");
        let mut kit_text = std::str::from_utf8(&kit.bytes)
            .map_err(|_| HarnessError::invalid("Native Kit 3 catalog is not UTF-8."))?
            .to_owned();
        for (path, asset) in &assets {
            for attr in ["src", "href"] {
                kit_text = kit_text.replace(
                    &format!("{attr}=\"{path}\""),
                    &format!("{attr}=\"{path}?v={}\"", asset.version),
                );
            }
        }
        kit.set_bytes(kit_text.into_bytes());
        assets.insert("/kit3/".into(), kit);
        let mut page = assets
            .remove("/index.html")
            .ok_or_else(|| HarnessError::missing("Harness index.html is missing."))?;
        let mut text = std::str::from_utf8(&page.bytes)
            .map_err(|_| HarnessError::invalid("Harness page is not UTF-8."))?
            .to_owned();
        for (path, asset) in &assets {
            for attr in ["src", "href"] {
                text = text.replace(
                    &format!("{attr}=\"{path}\""),
                    &format!("{attr}=\"{path}?v={}\"", asset.version),
                );
            }
        }
        page.set_bytes(text.into_bytes());
        assets.insert("/".into(), page);
        for (mut files, name, identity) in pins {
            files.check()?;
            if files.image_identity(&name)? != identity {
                return Err(HarnessError::conflict(
                    "Application assets changed while the startup snapshot was being assembled.",
                ));
            }
        }
        Ok(Self {
            services,
            token,
            instance: store::hexid(),
            port,
            source,
            shutdown,
            assets: Arc::new(assets),
        })
    }
    pub fn router(self) -> Router {
        Router::new().fallback(handle).with_state(self)
    }
}
#[derive(Clone, Copy)]
pub struct Route {
    pub method: &'static str,
    pub pattern: &'static str,
    pub operation: &'static str,
    pub status: u16,
    pub token: bool,
    pub query: &'static [&'static str],
}
pub const ROUTES: &[Route] = &[
    Route {
        method: "GET",
        pattern: "/api/health",
        operation: "health",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/bootstrap",
        operation: "bootstrap",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/events/stream",
        operation: "events.stream",
        status: 200,
        token: false,
        query: &["last_event_id", "session_id", "project_id"],
    },
    Route {
        method: "GET",
        pattern: "/api/sessions",
        operation: "sessions.list",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/projects",
        operation: "projects.list",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/setup",
        operation: "setup.status",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/accelerator",
        operation: "accelerators.get",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/accelerator/codex-hooks",
        operation: "accelerators.codex_hooks",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/graph",
        operation: "graph.status",
        status: 200,
        token: false,
        query: &["check"],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/graph/map",
        operation: "graph.map",
        status: 200,
        token: false,
        query: &[
            "scope",
            "focus",
            "certain",
            "filter",
            "target",
            "follow",
            "out",
            "into",
            "notests",
            "spread",
            "from",
            "to",
            "path",
            "longer",
            "failing",
            "shared",
            "side",
            "skip",
            "cycle",
            "base",
            "patterns",
            "pattern",
            "history",
            "modules",
            "no_incoming",
            "launch",
            "compare_launch",
            "scenario",
            "focus_session",
            "whole",
            "offset",
            "limit",
            "depth",
        ],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/graph/find",
        operation: "graph.find",
        status: 200,
        token: false,
        query: &["query", "target", "notests"],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/graph/changes",
        operation: "graph.changes",
        status: 200,
        token: false,
        query: &["target", "focus_session", "notests"],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/graph/file-impact",
        operation: "graph.file_impact",
        status: 200,
        token: false,
        query: &["path", "target", "focus_session", "certain"],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/graph/pulls",
        operation: "graph.pulls",
        status: 200,
        token: false,
        query: &["state", "query"],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/graph/commits",
        operation: "graph.commits",
        status: 200,
        token: false,
        query: &["scope", "query", "skip"],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/graph/branches",
        operation: "graph.branches",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/graph/memory-links",
        operation: "knowledge.memory_links",
        status: 200,
        token: false,
        query: &["path", "previous_path"],
    },
    Route {
        method: "POST",
        pattern: "/api/projects/{project_id}/graph/memory-links/batch",
        operation: "knowledge.memory_links_batch",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/omnigraph/file-context",
        operation: "omnigraph.file_context",
        status: 200,
        token: false,
        query: &["project_id", "path"],
    },
    Route {
        method: "GET",
        pattern: "/api/systems/service-graph",
        operation: "system_graph.service_status",
        status: 200,
        token: false,
        query: &["project_id", "config_path", "service", "check"],
    },
    Route {
        method: "GET",
        pattern: "/api/systems/service-graph/status",
        operation: "system_graph.service_status",
        status: 200,
        token: false,
        query: &["project_id", "config_path", "service", "check"],
    },
    Route {
        method: "GET",
        pattern: "/api/systems/service-graph/map",
        operation: "system_graph.service_map",
        status: 200,
        token: false,
        query: &[
            "project_id",
            "config_path",
            "service",
            "run_id",
            "agent",
            "scope",
            "focus",
            "certain",
            "filter",
            "target",
            "follow",
            "out",
            "into",
            "notests",
            "spread",
            "from",
            "to",
            "path",
            "longer",
            "failing",
            "shared",
            "side",
            "skip",
            "cycle",
            "base",
            "patterns",
            "pattern",
            "history",
            "modules",
            "no_incoming",
            "launch",
            "compare_launch",
            "scenario",
            "focus_session",
            "whole",
            "offset",
            "limit",
            "depth",
        ],
    },
    Route {
        method: "GET",
        pattern: "/api/systems/service-graph/find",
        operation: "system_graph.service_find",
        status: 200,
        token: false,
        query: &[
            "project_id",
            "config_path",
            "service",
            "query",
            "q",
            "target",
            "notests",
            "run_id",
        ],
    },
    Route {
        method: "GET",
        pattern: "/api/systems/service-graph/changes",
        operation: "system_graph.service_changes",
        status: 200,
        token: false,
        query: &[
            "project_id",
            "config_path",
            "service",
            "target",
            "focus_session",
            "notests",
            "run_id",
            "agent",
        ],
    },
    Route {
        method: "GET",
        pattern: "/api/systems/service-graph/file-impact",
        operation: "system_graph.service_file_impact",
        status: 200,
        token: false,
        query: &[
            "project_id",
            "config_path",
            "service",
            "path",
            "target",
            "focus_session",
            "certain",
            "run_id",
            "agent",
        ],
    },
    Route {
        method: "GET",
        pattern: "/api/systems/service-graph/pulls",
        operation: "system_graph.service_pulls",
        status: 200,
        token: false,
        query: &["project_id", "config_path", "service", "state", "query"],
    },
    Route {
        method: "GET",
        pattern: "/api/systems/service-graph/commits",
        operation: "system_graph.service_commits",
        status: 200,
        token: false,
        query: &[
            "project_id",
            "config_path",
            "service",
            "scope",
            "query",
            "skip",
        ],
    },
    Route {
        method: "GET",
        pattern: "/api/systems/service-graph/branches",
        operation: "system_graph.service_branches",
        status: 200,
        token: false,
        query: &["project_id", "config_path", "service"],
    },
    Route {
        method: "GET",
        pattern: "/api/systems/graph",
        operation: "system_graph.status",
        status: 200,
        token: false,
        query: &["project_id", "config_path", "check", "service"],
    },
    Route {
        method: "GET",
        pattern: "/api/systems/graph/status",
        operation: "system_graph.status",
        status: 200,
        token: false,
        query: &["project_id", "config_path", "check", "service"],
    },
    Route {
        method: "GET",
        pattern: "/api/systems/graph/map",
        operation: "system_graph.map",
        status: 200,
        token: false,
        query: &[
            "project_id",
            "config_path",
            "target",
            "run_id",
            "agent",
            "seed_services",
            "contracts",
            "depth",
            "scope",
            "focus",
            "certain",
            "filter",
            "follow",
            "out",
            "into",
            "spread",
            "from",
            "to",
            "path",
            "shared",
            "side",
            "skip",
            "cycle",
            "layers",
            "whole",
        ],
    },
    Route {
        method: "GET",
        pattern: "/api/systems/graph/find",
        operation: "system_graph.find",
        status: 200,
        token: false,
        query: &[
            "project_id",
            "config_path",
            "query",
            "q",
            "service",
            "notests",
        ],
    },
    Route {
        method: "GET",
        pattern: "/api/systems/graph/changes",
        operation: "system_graph.changes",
        status: 200,
        token: false,
        query: &[
            "project_id",
            "config_path",
            "target",
            "run_id",
            "agent",
            "service",
            "notests",
        ],
    },
    Route {
        method: "GET",
        pattern: "/api/systems/graph/bridge",
        operation: "system_graph.bridge",
        status: 200,
        token: false,
        query: &[
            "project_id",
            "config_path",
            "from",
            "to",
            "contract",
            "certain",
        ],
    },
    Route {
        method: "GET",
        pattern: "/api/systems/graph/impact",
        operation: "system_graph.impact",
        status: 200,
        token: false,
        query: &[
            "project_id",
            "config_path",
            "service",
            "path",
            "contract",
            "target",
            "run_id",
            "agent",
            "certain",
            "focus_session",
        ],
    },
    Route {
        method: "GET",
        pattern: "/api/system-runs",
        operation: "systems.list",
        status: 200,
        token: false,
        query: &["project_id"],
    },
    Route {
        method: "GET",
        pattern: "/api/system-discoveries/{id}",
        operation: "discovery.get",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/system-runs/{id}",
        operation: "systems.get",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/creator",
        operation: "creator.list",
        status: 200,
        token: false,
        query: &["project_id"],
    },
    Route {
        method: "GET",
        pattern: "/api/creator/{id}",
        operation: "creator.get",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/skills",
        operation: "skills.catalog",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/skills",
        operation: "skills.installed",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/sessions/{session_id}",
        operation: "sessions.get",
        status: 200,
        token: false,
        query: &["after", "before", "limit", "tail"],
    },
    Route {
        method: "GET",
        pattern: "/api/sessions/{session_id}/permission-rules",
        operation: "sessions.permission_rules_list",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "DELETE",
        pattern: "/api/sessions/{session_id}/permission-rules",
        operation: "sessions.permission_rules_delete",
        status: 200,
        token: true,
        query: &["rule_key", "id", "all"],
    },
    Route {
        method: "DELETE",
        pattern: "/api/sessions/{session_id}/permission-rules/{rule_id}",
        operation: "sessions.permission_rules_delete",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/sessions/{session_id}/attachments/{id}",
        operation: "attachments.read",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/sessions/{session_id}/merge",
        operation: "sessions.merge_archive",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/sessions/{session_id}/sdd",
        operation: "sdd.artifacts",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/sessions/{session_id}/results",
        operation: "results.get",
        status: 200,
        token: false,
        query: &["diff"],
    },
    Route {
        method: "GET",
        pattern: "/api/sessions/{session_id}/delivery",
        operation: "delivery.get",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/sessions/{session_id}/report",
        operation: "sessions.report",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/sessions/{session_id}/brain",
        operation: "brain.info",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/context",
        operation: "context.project",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/git",
        operation: "projects.git",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/commands",
        operation: "commands.project",
        status: 200,
        token: true,
        query: &["provider", "worktree"],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/files",
        operation: "files.project",
        status: 200,
        token: true,
        query: &["q", "worktree"],
    },
    Route {
        method: "GET",
        pattern: "/api/sessions/{session_id}/files",
        operation: "files.session",
        status: 200,
        token: true,
        query: &["q"],
    },
    Route {
        method: "GET",
        pattern: "/api/sessions/{session_id}/commands",
        operation: "commands.session",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/worktrees",
        operation: "projects.worktrees",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/memory",
        operation: "memory.read",
        status: 200,
        token: false,
        query: &["bank", "path"],
    },
    Route {
        method: "GET",
        pattern: "/api/app/update",
        operation: "updates.status",
        status: 200,
        token: false,
        query: &["check"],
    },
    Route {
        method: "GET",
        pattern: "/api/providers/sign-in",
        operation: "providers.sign_in",
        status: 200,
        token: true,
        query: &["refresh"],
    },
    Route {
        method: "GET",
        pattern: "/api/accelerators/startup",
        operation: "accelerators.startup",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/memory-use",
        operation: "memory.usage",
        status: 200,
        token: false,
        query: &["bank"],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/brain",
        operation: "knowledge.brain",
        status: 200,
        token: false,
        query: &["bank", "path"],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/knowledge",
        operation: "knowledge.info",
        status: 200,
        token: false,
        query: &["bank"],
    },
    Route {
        method: "GET",
        pattern: "/api/knowledge/exports/{id}",
        operation: "knowledge.download",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/system-discoveries",
        operation: "discovery.start",
        status: 201,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/systems/editor",
        operation: "systems.editor",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/systems/service",
        operation: "systems.service",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/systems/preview",
        operation: "systems.preview",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/systems/apply",
        operation: "systems.apply",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/systems/catalog",
        operation: "systems.catalog",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/systems/service-graph",
        operation: "system_graph.service_act",
        status: 200,
        token: true,
        query: &["project_id", "config_path", "service"],
    },
    Route {
        method: "POST",
        pattern: "/api/systems/graph",
        operation: "system_graph.act",
        status: 200,
        token: true,
        query: &["project_id", "config_path", "service"],
    },
    Route {
        method: "POST",
        pattern: "/api/system-runs",
        operation: "systems.prepare",
        status: 201,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/system-runs/{id}",
        operation: "systems.act",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/creator",
        operation: "creator.start",
        status: 201,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/creator/{id}",
        operation: "creator.act",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/sessions/merge",
        operation: "sessions.merge",
        status: 201,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/sessions/{session_id}/restart-merge",
        operation: "sessions.restart_merge",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/sessions/{session_id}/delete-merge-context",
        operation: "sessions.delete_merge",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/sessions",
        operation: "sessions.create",
        status: 201,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/projects",
        operation: "projects.add",
        status: 201,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/projects/{project_id}/select",
        operation: "projects.select",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/projects/browse",
        operation: "projects.browse",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/projects/{project_id}/accelerator",
        operation: "accelerators.act",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/projects/{project_id}/accelerator/codex-hooks",
        operation: "accelerators.trust_codex_hooks",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/projects/{project_id}/knowledge",
        operation: "knowledge.run",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/projects/{project_id}/preflight",
        operation: "knowledge.preflight",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/projects/{project_id}/graph",
        operation: "graph.act",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/projects/{project_id}/memory-use/check",
        operation: "memory.check",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/skills/discover",
        operation: "skills.discover",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/skills/change-preview",
        operation: "skills.change_preview",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/skills/apply",
        operation: "skills.apply_change",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/skills/preview",
        operation: "skills.preview",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/skills/create-preview",
        operation: "skills.create_preview",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/skills/install",
        operation: "skills.install",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/sessions/{session_id}/messages",
        operation: "sessions.send",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/sessions/{session_id}/inbox",
        operation: "sessions.inbox_accept",
        status: 202,
        token: true,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/sessions/{session_id}/inbox",
        operation: "sessions.inbox_list",
        status: 200,
        token: false,
        query: &["before"],
    },
    Route {
        method: "POST",
        pattern: "/api/sessions/{session_id}/inbox/{request_id}/cancel",
        operation: "sessions.inbox_cancel",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/sessions/{session_id}/check",
        operation: "results.start_check",
        status: 202,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/sessions/{session_id}/delivery",
        operation: "delivery.act",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/sessions/{session_id}/budgets",
        operation: "sessions.budgets",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/sessions/{session_id}/permissions/{request_id}",
        operation: "sessions.permission",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/sessions/{session_id}/cancel",
        operation: "sessions.cancel",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/sessions/{session_id}/decision",
        operation: "sessions.decide",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/sessions/{session_id}/resume",
        operation: "sessions.resume",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/sessions/{session_id}/context",
        operation: "context.prepare",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/sessions/{session_id}/run",
        operation: "context.run",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/sessions/{session_id}/brain",
        operation: "brain.act",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/sessions/{session_id}/memory",
        operation: "memory.save",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/accelerators/preview",
        operation: "setup.preview",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/accelerators/install",
        operation: "setup.install",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/shutdown",
        operation: "shutdown",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/app/update",
        operation: "updates.apply",
        status: 202,
        token: true,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/omnigraph/status",
        operation: "omnigraph.status",
        status: 200,
        token: false,
        query: &["project_id"],
    },
    Route {
        method: "POST",
        pattern: "/api/omnigraph/setup",
        operation: "omnigraph.setup",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/omnigraph/graph",
        operation: "omnigraph.graph",
        status: 200,
        token: false,
        query: &[
            "project_id",
            "branch",
            "snapshot",
            "kind",
            "q",
            "session_id",
            "limit",
            "offset",
            "cursor",
        ],
    },
    Route {
        method: "GET",
        pattern: "/api/omnigraph/search",
        operation: "omnigraph.search",
        status: 200,
        token: false,
        query: &["project_id", "q", "branch", "kind", "limit"],
    },
    Route {
        method: "GET",
        pattern: "/api/omnigraph/branches/diff",
        operation: "omnigraph.branch_diff",
        status: 200,
        token: false,
        query: &["project_id", "source", "target"],
    },
    Route {
        method: "POST",
        pattern: "/api/omnigraph/branches",
        operation: "omnigraph.branch_act",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/omnigraph/atoms",
        operation: "omnigraph.publish",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/omnigraph/share",
        operation: "omnigraph.share",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/omnigraph/export",
        operation: "omnigraph.export",
        status: 200,
        token: false,
        query: &[
            "project_id",
            "branch",
            "session_id",
            "format",
            "scope",
            "include_inherited",
            "download",
        ],
    },
    Route {
        method: "POST",
        pattern: "/api/omnigraph/export",
        operation: "omnigraph.export",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/omnigraph/import",
        operation: "omnigraph.import",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/omnigraph/sessions/link",
        operation: "omnigraph.link_sessions",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/omnigraph/sync-knowledge",
        operation: "omnigraph.sync_knowledge",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/omnigraph/promote",
        operation: "omnigraph.promote",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/omnigraph/changes",
        operation: "omnigraph.changes",
        status: 200,
        token: false,
        query: &["project_id", "cursor", "branch"],
    },
    Route {
        method: "GET",
        pattern: "/api/omnigraph/commits/{commit_id}",
        operation: "omnigraph.commit_detail",
        status: 200,
        token: false,
        query: &["project_id"],
    },
];
fn equal(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for (i, byte) in b.iter().enumerate() {
        diff |= usize::from(*byte ^ a.get(i).copied().unwrap_or(0));
    }
    diff == 0
}
fn trusted(app: &App, request: &Request<Body>) -> Result<()> {
    let hosts = [
        format!("127.0.0.1:{}", app.port),
        format!("localhost:{}", app.port),
    ];
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if !hosts.iter().any(|h| h == host) {
        return Err(HarnessError::forbidden("Use the localhost server address."));
    }
    if let Some(origin) = request.headers().get(header::ORIGIN) {
        let origin = origin
            .to_str()
            .map_err(|_| HarnessError::forbidden("Invalid Origin."))?;
        if !hosts.iter().any(|h| format!("http://{h}") == origin) {
            return Err(HarnessError::forbidden(
                "Cross-origin requests are disabled.",
            ));
        }
    }
    if request.uri().path().starts_with("/api/")
        && request
            .headers()
            .get("sec-fetch-site")
            .is_some_and(|h| h == "cross-site")
    {
        return Err(HarnessError::forbidden(
            "Cross-site API requests are disabled.",
        ));
    }
    Ok(())
}
fn route(method: &str, path: &str) -> Result<(Route, Map<String, Value>)> {
    for route in ROUTES.iter().filter(|r| r.method == method) {
        let expected = route.pattern.split('/').collect::<Vec<_>>();
        let actual = path.split('/').collect::<Vec<_>>();
        if expected.len() != actual.len() {
            continue;
        }
        let mut params = Map::new();
        let mut matches = true;
        for (a, b) in expected.into_iter().zip(actual) {
            if a.starts_with('{') && a.ends_with('}') && !b.is_empty() {
                params.insert(a[1..a.len() - 1].into(), decode(b)?.into());
            } else if a != b {
                matches = false;
                break;
            }
        }
        if matches {
            return Ok((*route, params));
        }
    }
    Err(HarnessError::missing("Not found."))
}
fn decode(raw: &str) -> Result<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err(HarnessError::invalid("Invalid URL encoding."));
            }
            let hi = (bytes[i + 1] as char)
                .to_digit(16)
                .ok_or_else(|| HarnessError::invalid("Invalid URL encoding."))?;
            let lo = (bytes[i + 2] as char)
                .to_digit(16)
                .ok_or_else(|| HarnessError::invalid("Invalid URL encoding."))?;
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
            i += 1;
        }
    }
    let text = String::from_utf8(out).map_err(|_| HarnessError::invalid("Invalid URL text."))?;
    if text.contains('\0') || text.chars().any(|c| c.is_control()) {
        return Err(HarnessError::invalid("Invalid URL value."));
    }
    Ok(text)
}
fn query(raw: Option<&str>, allowed: &[&str]) -> Result<Value> {
    let mut map = Map::new();
    if let Some(raw) = raw
        && !raw.is_empty()
    {
        for pair in raw.split('&') {
            let (key, value) = pair
                .split_once('=')
                .ok_or_else(|| HarnessError::invalid("Invalid query."))?;
            let key = decode(key)?;
            if !allowed.contains(&key.as_str()) || map.contains_key(&key) {
                return Err(HarnessError::invalid("Invalid query."));
            }
            map.insert(key, decode(value)?.into());
        }
    }
    Ok(map.into())
}
fn apply_security_headers(builder: axum::http::response::Builder) -> axum::http::response::Builder {
    builder
        .header("X-Content-Type-Options", "nosniff")
        .header("Referrer-Policy", "no-referrer")
        .header("Cross-Origin-Resource-Policy", "same-origin")
        .header(
            "Content-Security-Policy",
            "default-src 'self'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-src 'self'; frame-ancestors 'self'; base-uri 'none'; object-src 'none'; form-action 'self'",
        )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AssetEncoding {
    Identity,
    Gzip,
    Brotli,
}

fn preferred_encoding(headers: &HeaderMap, has_br: bool, has_gzip: bool) -> AssetEncoding {
    let mut br_q: Option<f32> = None;
    let mut gzip_q: Option<f32> = None;
    let mut wildcard_q: Option<f32> = None;
    for value in headers.get_all(header::ACCEPT_ENCODING) {
        let Ok(raw) = value.to_str() else {
            continue;
        };
        for item in raw.split(',') {
            let mut parts = item.split(';').map(str::trim);
            let coding = parts.next().unwrap_or("").to_ascii_lowercase();
            if coding.is_empty() {
                continue;
            }
            let mut q = 1.0_f32;
            for param in parts {
                if let Some(rest) = param
                    .strip_prefix("q=")
                    .or_else(|| param.strip_prefix("Q="))
                    && let Ok(parsed) = rest.trim().parse::<f32>()
                    && parsed.is_finite()
                    && parsed >= 0.0
                {
                    q = parsed.min(1.0);
                }
            }
            if coding == "br" {
                br_q = Some(q);
            } else if coding == "gzip" || coding == "x-gzip" {
                gzip_q = Some(q);
            } else if coding == "*" {
                wildcard_q = Some(q);
            }
        }
    }
    let br_eff = if has_br {
        br_q.or(wildcard_q).unwrap_or(0.0)
    } else {
        0.0
    };
    let gzip_eff = if has_gzip {
        gzip_q.or(wildcard_q).unwrap_or(0.0)
    } else {
        0.0
    };
    if br_eff > 0.0 && br_eff >= gzip_eff {
        AssetEncoding::Brotli
    } else if gzip_eff > 0.0 {
        AssetEncoding::Gzip
    } else {
        AssetEncoding::Identity
    }
}

fn asset_response(
    asset: &Asset,
    request: &Request<Body>,
    immutable: bool,
    head: bool,
) -> Result<Response<Body>> {
    let encoding = preferred_encoding(
        request.headers(),
        asset.br_bytes.is_some() && asset.br_etag.is_some(),
        asset.gzip_bytes.is_some() && asset.gzip_etag.is_some(),
    );
    let (selected_bytes, selected_etag, content_encoding): (&Arc<[u8]>, &str, Option<&str>) = match (
        encoding,
        asset.br_bytes.as_ref(),
        asset.br_etag.as_deref(),
        asset.gzip_bytes.as_ref(),
        asset.gzip_etag.as_deref(),
    ) {
        (AssetEncoding::Brotli, Some(br), Some(br_etag), _, _) => (br, br_etag, Some("br")),
        (AssetEncoding::Gzip, _, _, Some(gz), Some(gz_etag)) => (gz, gz_etag, Some("gzip")),
        _ => (&asset.bytes, &asset.etag, None),
    };
    let cache = if immutable {
        "private, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    if request
        .headers()
        .get(header::IF_NONE_MATCH)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|offered| offered.trim() == selected_etag || offered.trim() == "*")
        })
    {
        return apply_security_headers(
            Response::builder()
                .status(304)
                .header(header::ETAG, selected_etag)
                .header(header::VARY, "Accept-Encoding")
                .header(header::CACHE_CONTROL, cache),
        )
        .body(Body::empty())
        .map_err(|_| HarnessError::unavailable("Could not construct HTTP response."));
    }
    let mut out = apply_security_headers(
        Response::builder()
            .status(200)
            .header(header::CONTENT_TYPE, &asset.content_type)
            .header(header::CONTENT_LENGTH, selected_bytes.len().to_string())
            .header(header::ETAG, selected_etag)
            .header(header::VARY, "Accept-Encoding")
            .header(header::CACHE_CONTROL, cache),
    );
    if let Some(enc) = content_encoding {
        out = out.header(header::CONTENT_ENCODING, enc);
    }
    out.body(if head {
        Body::empty()
    } else {
        Body::from(Bytes::from_owner(Arc::clone(selected_bytes)))
    })
    .map_err(|_| HarnessError::unavailable("Could not construct HTTP response."))
}

fn response(status: u16, payload: Payload, head: bool) -> Result<Response<Body>> {
    let (bytes, content_type, filename, etag, cache) = match payload {
        Payload::Json(v) => (
            serde_json::to_vec(&v)?,
            "application/json; charset=utf-8".into(),
            None,
            None,
            None,
        ),
        Payload::Bytes {
            bytes,
            content_type,
            filename,
            etag,
            cache,
        } => (bytes, content_type, filename, etag, cache),
    };
    let mut out = apply_security_headers(
        Response::builder()
            .status(
                StatusCode::from_u16(status)
                    .map_err(|_| HarnessError::invalid("Invalid response status."))?,
            )
            .header(header::CONTENT_TYPE, content_type)
            .header(header::CONTENT_LENGTH, bytes.len().to_string())
            .header(header::CACHE_CONTROL, cache.unwrap_or("no-store".into())),
    );
    if let Some(tag) = etag {
        out = out.header(header::ETAG, tag);
    }
    if let Some(name) = filename {
        let safe = name
            .chars()
            .filter(|c| !c.is_control() && !['"', '\\', '/'].contains(c))
            .collect::<String>();
        out = out.header(
            header::CONTENT_DISPOSITION,
            format!(
                "attachment; filename*=UTF-8\'\'{}",
                safe.bytes()
                    .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                        (b as char).to_string()
                    } else {
                        format!("%{b:02X}")
                    })
                    .collect::<String>()
            ),
        );
    }
    out.body(if head {
        Body::empty()
    } else {
        Body::from(bytes)
    })
    .map_err(|_| HarnessError::unavailable("Could not construct HTTP response."))
}
const SSE_HANDOFF_HEADER: &str = "x-harness-sse-handoff";

struct SseRegistration {
    start: ReplayStart,
    filter_sid: Option<String>,
    filter_pid: Option<String>,
    shutdown: tokio::sync::watch::Receiver<bool>,
}

type HandoffMap = HashMap<String, (std::time::Instant, SseRegistration)>;

fn sse_handoffs() -> &'static Mutex<HandoffMap> {
    static HANDOFFS: OnceLock<Mutex<HandoffMap>> = OnceLock::new();
    HANDOFFS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_sse_handoff(reg: SseRegistration) -> String {
    let token = format!("{}{}", store::hexid(), store::hexid());
    if let Ok(mut map) = sse_handoffs().lock() {
        let now = std::time::Instant::now();
        map.retain(|_, (created, _)| now.duration_since(*created).as_secs() < 15);
        map.insert(token.clone(), (now, reg));
    }
    token
}

fn take_sse_handoff(token: &str) -> Option<SseRegistration> {
    let Ok(mut map) = sse_handoffs().lock() else {
        return None;
    };
    map.remove(token).map(|(_, reg)| reg)
}

fn matches_sse_filter(
    frame: &StreamFrame,
    filter_sid: Option<&str>,
    filter_pid: Option<&str>,
) -> bool {
    if let Some(pid) = filter_pid
        && frame.event != "waiting_counts"
        && frame.project_id.as_deref() != Some(pid)
    {
        return false;
    }
    if let Some(sid) = filter_sid
        && !matches!(frame.event, "waiting_counts" | "graph_status")
        && frame.session_id.as_deref() != Some(sid)
    {
        return false;
    }
    true
}

fn format_sse_frame(id: u64, event: &str, data: &Value) -> String {
    let json = serde_json::to_string(data).unwrap_or_else(|_| "{}".into());
    format!("id: {id}\nevent: {event}\ndata: {json}\n\n")
}

async fn write_http_chunk(
    stream: &mut tokio::net::TcpStream,
    payload: &[u8],
) -> std::io::Result<()> {
    if payload.is_empty() {
        return Ok(());
    }
    let prefix = format!("{:x}\r\n", payload.len());
    stream.write_all(prefix.as_bytes()).await?;
    stream.write_all(payload).await?;
    stream.write_all(b"\r\n").await?;
    stream.flush().await
}

async fn run_sse_stream(
    mut stream: tokio::net::TcpStream,
    clean_headers: Vec<u8>,
    mut reg: SseRegistration,
) {
    if stream.write_all(&clean_headers).await.is_err() || stream.flush().await.is_err() {
        return;
    }
    if write_http_chunk(&mut stream, b": connected\n\n")
        .await
        .is_err()
    {
        return;
    }
    let mut last_sent_seq = 0_u64;
    let mut rx = match reg.start {
        ReplayStart::Resync {
            oldest_id,
            latest_id,
            reason,
            rx,
        } => {
            last_sent_seq = latest_id;
            let frame = format_sse_frame(
                latest_id,
                "resync",
                &json!({
                    "oldest_id": oldest_id,
                    "latest_id": latest_id,
                    "reason": reason,
                }),
            );
            if write_http_chunk(&mut stream, frame.as_bytes())
                .await
                .is_err()
            {
                return;
            }
            rx
        }
        ReplayStart::Frames { frames, rx } => {
            for frame in frames {
                if frame.seq <= last_sent_seq {
                    continue;
                }
                last_sent_seq = frame.seq;
                if !matches_sse_filter(&frame, reg.filter_sid.as_deref(), reg.filter_pid.as_deref())
                {
                    continue;
                }
                let formatted = format_sse_frame(frame.seq, frame.event, &frame.data);
                if write_http_chunk(&mut stream, formatted.as_bytes())
                    .await
                    .is_err()
                {
                    return;
                }
            }
            rx
        }
    };
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(15));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await;
    while !*reg.shutdown.borrow() {
        tokio::select! {
            changed = reg.shutdown.changed() => {
                if changed.is_err() || *reg.shutdown.borrow() {
                    break;
                }
            }
            _ = heartbeat.tick() => {
                if write_http_chunk(&mut stream, b": keep-alive\n\n").await.is_err() {
                    return;
                }
            }
            recv = rx.recv() => {
                match recv {
                    Ok(frame) => {
                        if frame.seq <= last_sent_seq {
                            continue;
                        }
                        last_sent_seq = frame.seq;
                        if !matches_sse_filter(
                            &frame,
                            reg.filter_sid.as_deref(),
                            reg.filter_pid.as_deref(),
                        ) {
                            continue;
                        }
                        let formatted = format_sse_frame(frame.seq, frame.event, &frame.data);
                        if write_http_chunk(&mut stream, formatted.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        let resync = format_sse_frame(
                            last_sent_seq,
                            "resync",
                            &json!({"reason": "lagged", "skipped": skipped}),
                        );
                        if write_http_chunk(&mut stream, resync.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    let _ = stream.write_all(b"0\r\n\r\n").await;
    let _ = stream.shutdown().await;
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn extract_sse_handoff_token(head: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(head).ok()?;
    for line in text.split("\r\n").skip(1) {
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case(SSE_HANDOFF_HEADER)
        {
            let token = value.trim();
            if !token.is_empty() {
                return Some(token.to_owned());
            }
        }
    }
    None
}

fn rewrite_sse_response_head(head: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(head);
    let mut out = Vec::with_capacity(head.len() + 64);
    for (idx, line) in text.split("\r\n").enumerate() {
        if idx == 0 {
            out.extend_from_slice(line.as_bytes());
            out.extend_from_slice(b"\r\n");
            continue;
        }
        if line.is_empty() {
            continue;
        }
        let name = line.split_once(':').map_or("", |(n, _)| n.trim());
        if name.eq_ignore_ascii_case(SSE_HANDOFF_HEADER)
            || name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("connection")
            || name.eq_ignore_ascii_case("transfer-encoding")
        {
            continue;
        }
        out.extend_from_slice(line.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"transfer-encoding: chunked\r\nconnection: close\r\n\r\n");
    out
}

enum SseIoState {
    Active(tokio::net::TcpStream),
    HandoffDone,
}

struct SseIo {
    state: SseIoState,
    expect_response_head: bool,
    head_probe: Vec<u8>,
    pending_out: Vec<u8>,
    pending_offset: usize,
}

impl SseIo {
    fn new(stream: tokio::net::TcpStream) -> Self {
        Self {
            state: SseIoState::Active(stream),
            expect_response_head: false,
            head_probe: Vec::new(),
            pending_out: Vec::new(),
            pending_offset: 0,
        }
    }

    fn drain_pending(&mut self, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let SseIoState::Active(stream) = &mut self.state else {
            return Poll::Ready(Ok(()));
        };
        while self.pending_offset < self.pending_out.len() {
            match Pin::new(&mut *stream).poll_write(cx, &self.pending_out[self.pending_offset..]) {
                Poll::Ready(Ok(0)) => {
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "failed to write pending HTTP bytes",
                    )));
                }
                Poll::Ready(Ok(n)) => {
                    self.pending_offset += n;
                }
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Pending => return Poll::Pending,
            }
        }
        self.pending_out.clear();
        self.pending_offset = 0;
        Poll::Ready(Ok(()))
    }
}

impl AsyncRead for SseIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = &mut *self;
        if matches!(this.state, SseIoState::HandoffDone) {
            return Poll::Ready(Ok(()));
        }
        if this.pending_offset < this.pending_out.len() {
            match this.drain_pending(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Ready(Ok(())) => {}
            }
        }
        let before = buf.filled().len();
        let SseIoState::Active(stream) = &mut this.state else {
            return Poll::Ready(Ok(()));
        };
        let poll = Pin::new(stream).poll_read(cx, buf);
        if matches!(poll, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            this.expect_response_head = true;
        }
        poll
    }
}

impl AsyncWrite for SseIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = &mut *self;
        if matches!(this.state, SseIoState::HandoffDone) {
            return Poll::Ready(Ok(data.len()));
        }
        if this.pending_offset < this.pending_out.len() {
            match this.drain_pending(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Ready(Ok(())) => {}
            }
        }
        if !this.expect_response_head {
            let SseIoState::Active(stream) = &mut this.state else {
                return Poll::Ready(Ok(data.len()));
            };
            return Pin::new(stream).poll_write(cx, data);
        }
        this.head_probe.extend_from_slice(data);
        if this.head_probe.len() >= 5 && !this.head_probe.starts_with(b"HTTP/") {
            this.expect_response_head = false;
            this.pending_out = std::mem::take(&mut this.head_probe);
            this.pending_offset = 0;
            if let Poll::Ready(Err(err)) = this.drain_pending(cx) {
                return Poll::Ready(Err(err));
            }
            return Poll::Ready(Ok(data.len()));
        }
        if let Some(header_end) = find_header_end(&this.head_probe) {
            if let Some(token) = extract_sse_handoff_token(&this.head_probe[..header_end])
                && let Some(reg) = take_sse_handoff(&token)
            {
                let clean_head = rewrite_sse_response_head(&this.head_probe[..header_end]);
                this.expect_response_head = false;
                this.head_probe.clear();
                this.pending_out.clear();
                this.pending_offset = 0;
                if let SseIoState::Active(stream) =
                    std::mem::replace(&mut this.state, SseIoState::HandoffDone)
                {
                    tokio::spawn(run_sse_stream(stream, clean_head, reg));
                }
                return Poll::Ready(Ok(data.len()));
            }
            this.expect_response_head = false;
            this.pending_out = std::mem::take(&mut this.head_probe);
            this.pending_offset = 0;
            if let Poll::Ready(Err(err)) = this.drain_pending(cx) {
                return Poll::Ready(Err(err));
            }
            return Poll::Ready(Ok(data.len()));
        }
        if this.head_probe.len() >= 8192 {
            this.expect_response_head = false;
            this.pending_out = std::mem::take(&mut this.head_probe);
            this.pending_offset = 0;
            if let Poll::Ready(Err(err)) = this.drain_pending(cx) {
                return Poll::Ready(Err(err));
            }
        }
        Poll::Ready(Ok(data.len()))
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        if !self.expect_response_head
            && self.pending_offset >= self.pending_out.len()
            && let SseIoState::Active(stream) = &mut self.as_mut().get_mut().state
        {
            return Pin::new(stream).poll_write_vectored(cx, bufs);
        }
        let Some(first) = bufs.iter().find(|b| !b.is_empty()) else {
            return Poll::Ready(Ok(0));
        };
        self.poll_write(cx, first)
    }

    fn is_write_vectored(&self) -> bool {
        !self.expect_response_head
            && self.pending_offset >= self.pending_out.len()
            && matches!(&self.state, SseIoState::Active(stream) if stream.is_write_vectored())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let this = &mut *self;
        if matches!(this.state, SseIoState::HandoffDone) {
            return Poll::Ready(Ok(()));
        }
        match this.drain_pending(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
            Poll::Ready(Ok(())) => {}
        }
        let SseIoState::Active(stream) = &mut this.state else {
            return Poll::Ready(Ok(()));
        };
        Pin::new(stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let this = &mut *self;
        if matches!(this.state, SseIoState::HandoffDone) {
            return Poll::Ready(Ok(()));
        }
        match this.drain_pending(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
            Poll::Ready(Ok(())) => {}
        }
        let SseIoState::Active(stream) = &mut this.state else {
            return Poll::Ready(Ok(()));
        };
        Pin::new(stream).poll_shutdown(cx)
    }
}

struct SseListener(tokio::net::TcpListener);

impl axum::serve::Listener for SseListener {
    type Io = SseIo;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.0.accept().await {
                Ok((stream, addr)) => return (SseIo::new(stream), addr),
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.0.local_addr()
    }
}

async fn parse_json_body(request: Request<Body>, limit: usize, allow_empty: bool) -> Result<Value> {
    let has_transfer_encoding = request.headers().get(header::TRANSFER_ENCODING).is_some();
    let length_header = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|h| h.to_str().ok())
        .map(str::trim);
    if allow_empty && !has_transfer_encoding && matches!(length_header, None | Some("0")) {
        return Ok(json!({}));
    }
    if has_transfer_encoding
        || request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.split(';').next())
            .map(str::trim)
            != Some("application/json")
    {
        return Err(HarnessError::invalid(
            "Use an application/json body with Content-Length.",
        ));
    }
    let length = length_header
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0 && *n <= limit)
        .ok_or_else(|| {
            HarnessError::invalid("JSON body length is missing or outside its byte limit.")
        })?;
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        to_bytes(request.into_body(), limit),
    )
    .await
    .map_err(|_| HarnessError::invalid("Request body timed out."))?
    .map_err(|_| HarnessError::invalid("Request body exceeds its limit."))?;
    if bytes.len() != length {
        return Err(HarnessError::invalid(
            "Request body length differs from Content-Length.",
        ));
    }
    let value = crate::strict_json::parse(&bytes, limit)?;
    if !value.is_object() {
        return Err(HarnessError::invalid("JSON body must be an object."));
    }
    Ok(value)
}

fn serve_sse_request(
    app: &App,
    request: &Request<Body>,
    query: &Value,
    head: bool,
) -> Result<Response<Body>> {
    let cursor_raw = query["last_event_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            request
                .headers()
                .get("last-event-id")
                .and_then(|h| h.to_str().ok())
                .map(str::trim)
                .filter(|s| !s.is_empty())
        });
    let last_event_id = match cursor_raw {
        None => None,
        Some(raw) => Some(
            raw.parse::<u64>()
                .map_err(|_| HarnessError::invalid("Invalid Last-Event-ID cursor."))?,
        ),
    };
    if head {
        return apply_security_headers(
            Response::builder()
                .status(200)
                .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
                .header(header::CACHE_CONTROL, "no-store"),
        )
        .body(Body::empty())
        .map_err(|_| HarnessError::unavailable("Could not construct HTTP response."));
    }
    let Some(start) = app.services.subscribe_stream(last_event_id) else {
        return Err(HarnessError::unavailable(
            "Real-time event stream is unavailable.",
        ));
    };
    let filter_sid = query["session_id"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let filter_pid = query["project_id"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let token = register_sse_handoff(SseRegistration {
        start,
        filter_sid,
        filter_pid,
        shutdown: app.shutdown.subscribe(),
    });
    apply_security_headers(
        Response::builder()
            .status(200)
            .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-cache")
            .header(SSE_HANDOFF_HEADER, token),
    )
    .body(Body::empty())
    .map_err(|_| HarnessError::unavailable("Could not construct HTTP response."))
}

async fn handle(State(app): State<App>, request: Request<Body>) -> Response<Body> {
    let head = request.method() == axum::http::Method::HEAD;
    match serve(&app, request, head).await {
        Ok(response) => response,
        Err(error) => response(
            error.status,
            Payload::Json(json!({"error":error.message})),
            head,
        )
        .unwrap_or_else(|_| Response::new(Body::empty())),
    }
}
async fn serve(app: &App, request: Request<Body>, head: bool) -> Result<Response<Body>> {
    trusted(app, &request)?;
    let method = if head {
        "GET"
    } else {
        request.method().as_str()
    }
    .to_string();
    let path = request.uri().path().to_string();
    if method == "GET"
        && let Some(asset) = app.assets.get(&path)
    {
        let q = query(request.uri().query(), &["v"])?;
        let immutable = q["v"] == asset.version;
        return asset_response(asset, &request, immutable, head);
    }
    let (route, params) = route(&method, &path)?;
    let query = query(request.uri().query(), route.query)?;
    if route.token || method == "POST" || method == "DELETE" {
        let token = request
            .headers()
            .get("x-harness-token")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        if !token.is_ascii() || !equal(token.as_bytes(), app.token.as_bytes()) {
            return Err(HarnessError::forbidden(
                "Reload the page to renew the session token.",
            ));
        }
    }
    if route.operation == "events.stream" {
        return serve_sse_request(app, &request, &query, head);
    }
    let body = if method == "POST" || method == "DELETE" {
        let upload = route.operation == "sessions.create" || route.operation == "sessions.send";
        let limit = if upload {
            crate::attachments::MAX_JSON_BYTES
        } else if route.operation == "skills.create_preview" {
            131072
        } else if route.operation == "omnigraph.import" {
            4 * 1024 * 1024
        } else if ["omnigraph.publish", "omnigraph.share"].contains(&route.operation) {
            512 * 1024
        } else if ["systems.preview", "discovery.start"].contains(&route.operation) {
            2 * 1024 * 1024
        } else {
            65536
        };
        parse_json_body(request, limit, method == "DELETE").await?
    } else {
        json!({})
    };
    if route.operation == "health" {
        return response(
            200,
            Payload::Json(json!({"ok":true,"instance":app.instance})),
            head,
        );
    }
    if route.operation == "shutdown" {
        if body.as_object().is_none_or(|m| !m.is_empty()) {
            return Err(HarnessError::invalid("Invalid shutdown request."));
        }
        let _ = app.shutdown.send(true);
        return response(200, Payload::Json(json!({"ok":true})), head);
    }
    let mut payload = app
        .services
        .request(
            route.operation,
            json!({"params":params,"query":query,"body":body}),
        )
        .await?;
    if route.operation == "bootstrap"
        && let Payload::Json(value) = &mut payload
    {
        value["csrf"] = app.token.clone().into();
    }
    let status = if route.operation == "updates.apply"
        && matches!(&payload,Payload::Json(value)if value["current"]==true)
    {
        200
    } else {
        route.status
    };
    response(status, payload, head)
}
pub async fn serve_listener(listener: tokio::net::TcpListener, app: App) -> Result<()> {
    let mut shutdown = app.shutdown.subscribe();
    axum::serve(SseListener(listener), app.router())
        .with_graceful_shutdown(async move {
            while !*shutdown.borrow() {
                if shutdown.changed().await.is_err() {
                    break;
                }
            }
        })
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_memory_links_and_omnigraph_file_context_routes_registered() {
        for (pattern, op, query) in [
            (
                "/api/projects/{project_id}/graph/memory-links",
                "knowledge.memory_links",
                &["path", "previous_path"][..],
            ),
            (
                "/api/omnigraph/file-context",
                "omnigraph.file_context",
                &["project_id", "path"][..],
            ),
        ] {
            let found = ROUTES
                .iter()
                .find(|r| r.method == "GET" && r.pattern == pattern)
                .unwrap_or_else(|| panic!("missing route {pattern}"));
            assert_eq!(found.operation, op);
            assert_eq!(found.query, query);
            // Reading links changes nothing.
            assert!(!found.token);
        }
    }

    #[test]
    fn graph_memory_links_batch_route_registered_with_csrf_token() {
        let found = ROUTES
            .iter()
            .find(|r| {
                r.method == "POST"
                    && r.pattern == "/api/projects/{project_id}/graph/memory-links/batch"
            })
            .expect("missing POST memory-links/batch route");
        assert_eq!(found.operation, "knowledge.memory_links_batch");
        assert_eq!(found.status, 200);
        assert!(found.token, "batch memory-links must require CSRF token");
        assert!(found.query.is_empty());

        let (matched, params) =
            route("POST", "/api/projects/demo/graph/memory-links/batch").expect("route matches");
        assert_eq!(matched.operation, "knowledge.memory_links_batch");
        assert_eq!(params["project_id"], "demo");
    }

    #[test]
    fn omnigraph_sync_and_promote_routes_are_registered_with_csrf_tokens() {
        for (method, pattern, op) in [
            (
                "POST",
                "/api/omnigraph/sync-knowledge",
                "omnigraph.sync_knowledge",
            ),
            ("POST", "/api/omnigraph/promote", "omnigraph.promote"),
        ] {
            let found = ROUTES
                .iter()
                .find(|r| r.method == method && r.pattern == pattern)
                .unwrap_or_else(|| panic!("missing route {method} {pattern}"));
            assert_eq!(found.operation, op);
            assert_eq!(found.status, 200);
            assert!(found.token, "{pattern} must require CSRF token");
            assert!(found.query.is_empty());

            let (matched, params) = route(method, pattern).expect("route matches");
            assert_eq!(matched.operation, op);
            assert!(params.is_empty());
        }
    }

    #[test]
    fn omnigraph_graph_route_allows_pagination_query_params() {
        let found = ROUTES
            .iter()
            .find(|r| r.method == "GET" && r.pattern == "/api/omnigraph/graph")
            .expect("missing GET /api/omnigraph/graph route");
        assert_eq!(found.operation, "omnigraph.graph");
        assert_eq!(found.status, 200);
        assert!(!found.token);
        assert_eq!(
            found.query,
            &[
                "project_id",
                "branch",
                "snapshot",
                "kind",
                "q",
                "session_id",
                "limit",
                "offset",
                "cursor",
            ]
        );
        let parsed = query(
            Some("project_id=p1&branch=main&limit=100&offset=50&cursor=offset:50"),
            found.query,
        )
        .expect("pagination query params accepted");
        assert_eq!(parsed["limit"], "100");
        assert_eq!(parsed["offset"], "50");
        assert_eq!(parsed["cursor"], "offset:50");
    }

    #[test]
    fn project_preflight_route_registered_with_csrf_token() {
        let found = ROUTES
            .iter()
            .find(|r| r.method == "POST" && r.pattern == "/api/projects/{project_id}/preflight")
            .expect("missing POST /api/projects/{project_id}/preflight route");
        assert_eq!(found.operation, "knowledge.preflight");
        assert_eq!(found.status, 200);
        assert!(found.token, "preflight must require CSRF token");
        assert!(found.query.is_empty());

        let (matched, params) =
            route("POST", "/api/projects/demo/preflight").expect("route matches");
        assert_eq!(matched.operation, "knowledge.preflight");
        assert_eq!(params["project_id"], "demo");
    }

    #[test]
    fn sse_permission_rules_session_pagination_and_graph_map_routes_registered() {
        let sse = ROUTES
            .iter()
            .find(|r| r.method == "GET" && r.pattern == "/api/events/stream")
            .expect("missing SSE route");
        assert_eq!(sse.operation, "events.stream");
        assert!(!sse.token);
        assert_eq!(sse.query, &["last_event_id", "session_id", "project_id"]);

        let list_rules = ROUTES
            .iter()
            .find(|r| {
                r.method == "GET" && r.pattern == "/api/sessions/{session_id}/permission-rules"
            })
            .expect("missing GET permission-rules route");
        assert_eq!(list_rules.operation, "sessions.permission_rules_list");
        assert!(!list_rules.token);

        let del_rules = ROUTES
            .iter()
            .find(|r| {
                r.method == "DELETE" && r.pattern == "/api/sessions/{session_id}/permission-rules"
            })
            .expect("missing DELETE permission-rules route");
        assert_eq!(del_rules.operation, "sessions.permission_rules_delete");
        assert!(del_rules.token);
        assert_eq!(del_rules.query, &["rule_key", "id", "all"]);

        let del_rule = ROUTES
            .iter()
            .find(|r| {
                r.method == "DELETE"
                    && r.pattern == "/api/sessions/{session_id}/permission-rules/{rule_id}"
            })
            .expect("missing DELETE permission-rules/{rule_id} route");
        assert_eq!(del_rule.operation, "sessions.permission_rules_delete");
        assert!(del_rule.token);

        let s_get = ROUTES
            .iter()
            .find(|r| r.method == "GET" && r.pattern == "/api/sessions/{session_id}")
            .expect("missing sessions.get");
        assert_eq!(s_get.query, &["after", "before", "limit", "tail"]);

        for op in ["graph.map", "system_graph.service_map"] {
            let r = ROUTES
                .iter()
                .find(|r| r.operation == op)
                .unwrap_or_else(|| panic!("missing {op}"));
            assert!(r.query.contains(&"offset"));
            assert!(r.query.contains(&"limit"));
            assert!(r.query.contains(&"depth"));
        }
    }

    #[test]
    fn accept_encoding_negotiation_prefers_br_respects_q_values_and_etags() {
        let mut h = HeaderMap::new();
        assert_eq!(preferred_encoding(&h, true, true), AssetEncoding::Identity);
        h.insert(header::ACCEPT_ENCODING, "deflate, br".parse().unwrap());
        assert_eq!(preferred_encoding(&h, true, true), AssetEncoding::Brotli);
        assert_eq!(preferred_encoding(&h, false, true), AssetEncoding::Identity);
        h.insert(
            header::ACCEPT_ENCODING,
            "gzip, deflate, br".parse().unwrap(),
        );
        assert_eq!(preferred_encoding(&h, true, true), AssetEncoding::Brotli);
        assert_eq!(preferred_encoding(&h, false, true), AssetEncoding::Gzip);
        h.insert(header::ACCEPT_ENCODING, "br;q=0, gzip;q=1".parse().unwrap());
        assert_eq!(preferred_encoding(&h, true, true), AssetEncoding::Gzip);
        h.insert(header::ACCEPT_ENCODING, "gzip;q=0, br;q=1".parse().unwrap());
        assert_eq!(preferred_encoding(&h, true, true), AssetEncoding::Brotli);
        h.insert(
            header::ACCEPT_ENCODING,
            "br;q=0.5, gzip;q=1".parse().unwrap(),
        );
        assert_eq!(preferred_encoding(&h, true, true), AssetEncoding::Gzip);
        h.insert(header::ACCEPT_ENCODING, "*;q=0.5".parse().unwrap());
        assert_eq!(preferred_encoding(&h, true, true), AssetEncoding::Brotli);
        h.insert(header::ACCEPT_ENCODING, "*;q=0".parse().unwrap());
        assert_eq!(preferred_encoding(&h, true, true), AssetEncoding::Identity);

        let payload = "hello wasm ".repeat(200).into_bytes();
        let asset = Asset::from_bytes(payload.clone(), "application/wasm");
        let gz = asset.gzip_bytes.as_ref().expect("gzip bytes compressed");
        let br = asset.br_bytes.as_ref().expect("brotli bytes compressed");
        assert!(gz.len() < asset.bytes.len());
        assert!(br.len() < asset.bytes.len());
        assert_ne!(asset.etag.as_str(), asset.gzip_etag.as_deref().unwrap());
        assert_ne!(asset.etag.as_str(), asset.br_etag.as_deref().unwrap());
        assert_ne!(
            asset.gzip_etag.as_deref().unwrap(),
            asset.br_etag.as_deref().unwrap()
        );
    }
}
