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
    collections::{BTreeMap, HashMap, VecDeque},
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

fn compress_gzip_level(raw: &[u8], level: Compression) -> Option<Arc<[u8]>> {
    if raw.is_empty() {
        return None;
    }
    let mut encoder = GzEncoder::new(Vec::new(), level);
    encoder.write_all(raw).ok()?;
    let compressed = encoder.finish().ok()?;
    Some(Arc::from(compressed))
}

fn compress_gzip(raw: &[u8]) -> Option<Arc<[u8]>> {
    compress_gzip_level(raw, Compression::best())
}

fn compress_brotli_quality(raw: &[u8], quality: u32) -> Option<Arc<[u8]>> {
    if raw.is_empty() {
        return None;
    }
    let params = brotli::enc::BrotliEncoderParams {
        quality: quality as i32,
        lgwin: 22,
        ..Default::default()
    };
    let mut compressed = Vec::new();
    brotli::BrotliCompress(&mut std::io::Cursor::new(raw), &mut compressed, &params).ok()?;
    Some(Arc::from(compressed))
}

fn compress_brotli(raw: &[u8]) -> Option<Arc<[u8]>> {
    compress_brotli_quality(raw, 6)
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
            "lod",
            "min_weight",
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
        query: &["target", "from", "to", "focus_session", "notests"],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/graph/file-impact",
        operation: "graph.file_impact",
        status: 200,
        token: false,
        query: &[
            "path",
            "target",
            "from",
            "to",
            "focus_session",
            "agent",
            "certain",
        ],
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
            "lod",
            "min_weight",
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
            "from",
            "to",
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
            "from",
            "to",
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
            "from",
            "to",
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
            "target",
            "run_id",
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
            "from",
            "to",
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
        method: "GET",
        pattern: "/api/sessions/{session_id}/skillcache",
        operation: "skillcache.session",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/sessions/{session_id}/skillcache/compile",
        operation: "skillcache.compile",
        status: 202,
        token: true,
        query: &[],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/skillcache/capsules",
        operation: "skillcache.list",
        status: 200,
        token: false,
        query: &["skill", "status", "form", "include_retired", "include_eval"],
    },
    Route {
        method: "GET",
        pattern: "/api/projects/{project_id}/skillcache/capsules/{capsule_id}",
        operation: "skillcache.get",
        status: 200,
        token: false,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/projects/{project_id}/skillcache/capsules/{capsule_id}/requalify",
        operation: "skillcache.requalify",
        status: 202,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/projects/{project_id}/skillcache/capsules/{capsule_id}/invalidate",
        operation: "skillcache.invalidate",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "POST",
        pattern: "/api/projects/{project_id}/skillcache/capsules/{capsule_id}/retire",
        operation: "skillcache.retire",
        status: 200,
        token: true,
        query: &[],
    },
    Route {
        method: "DELETE",
        pattern: "/api/projects/{project_id}/skillcache/capsules/{capsule_id}",
        operation: "skillcache.delete",
        status: 200,
        token: true,
        query: &[],
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

const MIN_COMPRESS_JSON_BYTES: usize = 1024;
const BLOCKING_COMPRESS_JSON_BYTES: usize = 16 * 1024;

struct EncodedPayloadParts {
    bytes: Vec<u8>,
    meta: EncodedResponseMeta,
}

struct EncodedResponseMeta {
    content_type: String,
    filename: Option<String>,
    etag: Option<String>,
    cache: Option<String>,
    is_json: bool,
}

fn split_payload_bytes(payload: Payload) -> Result<EncodedPayloadParts> {
    Ok(match payload {
        Payload::Json(v) => EncodedPayloadParts {
            bytes: serde_json::to_vec(&v)?,
            meta: EncodedResponseMeta {
                content_type: "application/json; charset=utf-8".into(),
                filename: None,
                etag: None,
                cache: None,
                is_json: true,
            },
        },
        Payload::Bytes {
            bytes,
            content_type,
            filename,
            etag,
            cache,
        } => EncodedPayloadParts {
            bytes,
            meta: EncodedResponseMeta {
                content_type,
                filename,
                etag,
                cache,
                is_json: false,
            },
        },
    })
}

fn should_compress_json(is_json: bool, bytes_len: usize, encoding: AssetEncoding) -> bool {
    is_json && bytes_len >= MIN_COMPRESS_JSON_BYTES && encoding != AssetEncoding::Identity
}

fn compress_json_bytes_sync(
    bytes: Vec<u8>,
    encoding: AssetEncoding,
) -> (Vec<u8>, Option<&'static str>) {
    match encoding {
        AssetEncoding::Brotli => compress_brotli_quality(&bytes, 4)
            .filter(|c| c.len() < bytes.len())
            .map_or((bytes, None), |c| (c.to_vec(), Some("br"))),
        AssetEncoding::Gzip => compress_gzip_level(&bytes, Compression::fast())
            .filter(|c| c.len() < bytes.len())
            .map_or((bytes, None), |c| (c.to_vec(), Some("gzip"))),
        AssetEncoding::Identity => (bytes, None),
    }
}

fn finish_encoded_response(
    status: u16,
    body_bytes: Vec<u8>,
    content_encoding: Option<&'static str>,
    meta: EncodedResponseMeta,
    head: bool,
) -> Result<Response<Body>> {
    let mut out = apply_security_headers(
        Response::builder()
            .status(
                StatusCode::from_u16(status)
                    .map_err(|_| HarnessError::invalid("Invalid response status."))?,
            )
            .header(header::CONTENT_TYPE, meta.content_type)
            .header(header::CONTENT_LENGTH, body_bytes.len().to_string())
            .header(
                header::CACHE_CONTROL,
                meta.cache.unwrap_or_else(|| "no-store".into()),
            ),
    );
    if meta.is_json {
        out = out.header(header::VARY, "Accept-Encoding");
    }
    if let Some(enc) = content_encoding {
        out = out.header(header::CONTENT_ENCODING, enc);
    }
    if let Some(tag) = meta.etag {
        out = out.header(header::ETAG, tag);
    }
    if let Some(name) = meta.filename {
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
        Body::from(body_bytes)
    })
    .map_err(|_| HarnessError::unavailable("Could not construct HTTP response."))
}

fn response_with_encoding(
    status: u16,
    payload: Payload,
    encoding: AssetEncoding,
    head: bool,
) -> Result<Response<Body>> {
    let parts = split_payload_bytes(payload)?;
    let (body_bytes, content_encoding) =
        if should_compress_json(parts.meta.is_json, parts.bytes.len(), encoding) {
            compress_json_bytes_sync(parts.bytes, encoding)
        } else {
            (parts.bytes, None)
        };
    finish_encoded_response(status, body_bytes, content_encoding, parts.meta, head)
}

async fn response_with_encoding_async(
    status: u16,
    payload: Payload,
    encoding: AssetEncoding,
    head: bool,
) -> Result<Response<Body>> {
    let parts = split_payload_bytes(payload)?;
    let (body_bytes, content_encoding) =
        if should_compress_json(parts.meta.is_json, parts.bytes.len(), encoding) {
            if parts.bytes.len() >= BLOCKING_COMPRESS_JSON_BYTES {
                let bytes = parts.bytes;
                tokio::task::spawn_blocking(move || compress_json_bytes_sync(bytes, encoding))
                    .await
                    .map_err(|_| HarnessError::unavailable("Compression worker failed."))?
            } else {
                compress_json_bytes_sync(parts.bytes, encoding)
            }
        } else {
            (parts.bytes, None)
        };
    finish_encoded_response(status, body_bytes, content_encoding, parts.meta, head)
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
        && !(frame.event == "resync" && frame.project_id.is_none())
        && frame.project_id.as_deref() != Some(pid)
    {
        return false;
    }
    if let Some(sid) = filter_sid
        && !matches!(
            frame.event,
            "waiting_counts"
                | "graph_status"
                | "system_graph_status"
                | "omnigraph_status"
                | "skillcache_capsule"
                | "resync"
        )
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
    mut tracker_shutdown: tokio::sync::watch::Receiver<bool>,
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
    while !*reg.shutdown.borrow() && !*tracker_shutdown.borrow() {
        tokio::select! {
            changed = reg.shutdown.changed() => {
                if changed.is_err() || *reg.shutdown.borrow() {
                    break;
                }
            }
            changed = tracker_shutdown.changed() => {
                if changed.is_err() || *tracker_shutdown.borrow() {
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

#[derive(Clone, Copy, Debug)]
struct ExpectedResp {
    is_head: bool,
}

#[derive(Clone)]
struct SseTaskTracker {
    tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    cancel: tokio::sync::watch::Sender<bool>,
}

impl Default for SseTaskTracker {
    fn default() -> Self {
        let (cancel, _) = tokio::sync::watch::channel(false);
        Self {
            tasks: Arc::new(Mutex::new(Vec::new())),
            cancel,
        }
    }
}

impl SseTaskTracker {
    fn spawn(&self, stream: tokio::net::TcpStream, clean_head: Vec<u8>, reg: SseRegistration) {
        let cancel_rx = self.cancel.subscribe();
        let handle = tokio::spawn(run_sse_stream(stream, clean_head, reg, cancel_rx));
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.retain(|h| !h.is_finished());
            tasks.push(handle);
        }
    }

    async fn shutdown(self, timeout: std::time::Duration) {
        let _ = self.cancel.send(true);
        let handles = self
            .tasks
            .lock()
            .map(|mut t| std::mem::take(&mut *t))
            .unwrap_or_default();
        if handles.is_empty() {
            return;
        }
        let deadline = tokio::time::Instant::now() + timeout;
        for handle in handles {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                handle.abort();
                continue;
            }
            let wait = deadline.saturating_duration_since(now);
            let abort_handle = handle.abort_handle();
            if tokio::time::timeout(wait, handle).await.is_err() {
                abort_handle.abort();
            }
        }
    }
}

struct SseTaskAbortGuard(Option<SseTaskTracker>);

impl Drop for SseTaskAbortGuard {
    fn drop(&mut self) {
        if let Some(tracker) = self.0.take() {
            let _ = tracker.cancel.send(true);
            if let Ok(mut tasks) = tracker.tasks.lock() {
                for handle in tasks.drain(..) {
                    handle.abort();
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
enum ChunkStage {
    SizeLine { line: Vec<u8> },
    Data { remaining: u64 },
    DataCr,
    DataLf,
    Trailers { line_non_empty: bool },
}

#[derive(Clone, Debug)]
struct ChunkDecoder {
    stage: ChunkStage,
}

enum ChunkStep {
    NeedMore,
    Done { consumed: usize },
    Invalid,
}

impl ChunkDecoder {
    fn new() -> Self {
        Self {
            stage: ChunkStage::SizeLine { line: Vec::new() },
        }
    }

    fn advance(&mut self, data: &[u8]) -> ChunkStep {
        let mut idx = 0;
        while idx < data.len() {
            match &mut self.stage {
                ChunkStage::SizeLine { line } => {
                    let b = data[idx];
                    idx += 1;
                    if b == b'\n' {
                        let raw = std::str::from_utf8(line).unwrap_or("");
                        let size_part = raw
                            .trim_end_matches('\r')
                            .split(';')
                            .next()
                            .unwrap_or("")
                            .trim();
                        let Ok(size) = u64::from_str_radix(size_part, 16) else {
                            return ChunkStep::Invalid;
                        };
                        if size == 0 {
                            self.stage = ChunkStage::Trailers {
                                line_non_empty: false,
                            };
                        } else {
                            self.stage = ChunkStage::Data { remaining: size };
                        }
                    } else {
                        if line.len() >= 4096 {
                            return ChunkStep::Invalid;
                        }
                        line.push(b);
                    }
                }
                ChunkStage::Data { remaining } => {
                    let take = (*remaining).min((data.len() - idx) as u64) as usize;
                    *remaining -= take as u64;
                    idx += take;
                    if *remaining == 0 {
                        self.stage = ChunkStage::DataCr;
                    }
                }
                ChunkStage::DataCr => {
                    let b = data[idx];
                    idx += 1;
                    if b == b'\r' {
                        self.stage = ChunkStage::DataLf;
                    } else if b == b'\n' {
                        self.stage = ChunkStage::SizeLine { line: Vec::new() };
                    } else {
                        return ChunkStep::Invalid;
                    }
                }
                ChunkStage::DataLf => {
                    let b = data[idx];
                    idx += 1;
                    if b == b'\n' {
                        self.stage = ChunkStage::SizeLine { line: Vec::new() };
                    } else {
                        return ChunkStep::Invalid;
                    }
                }
                ChunkStage::Trailers { line_non_empty } => {
                    let b = data[idx];
                    idx += 1;
                    if b == b'\n' {
                        if !*line_non_empty {
                            return ChunkStep::Done { consumed: idx };
                        }
                        *line_non_empty = false;
                    } else if b != b'\r' {
                        *line_non_empty = true;
                    }
                }
            }
        }
        ChunkStep::NeedMore
    }
}

fn trim_ascii_crlf(mut bytes: &[u8]) -> &[u8] {
    while let Some((&b, rest)) = bytes.split_first() {
        if b == b'\r' || b == b'\n' {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}

fn parse_http_headers_meta(head: &[u8]) -> (Option<u16>, bool, Option<u64>) {
    let text = String::from_utf8_lossy(head);
    let mut lines = text.split("\r\n");
    let status = lines.next().and_then(|first| {
        first
            .split_ascii_whitespace()
            .nth(1)
            .and_then(|code| code.parse::<u16>().ok())
    });
    let mut is_chunked = false;
    let mut content_length = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.eq_ignore_ascii_case("transfer-encoding") {
            if value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("chunked"))
            {
                is_chunked = true;
            }
        } else if name.eq_ignore_ascii_case("content-length")
            && content_length.is_none()
            && let Ok(len) = value.trim().parse::<u64>()
        {
            content_length = Some(len);
        }
    }
    (status, is_chunked, content_length)
}

enum ReqParseState {
    Passthrough,
    Headers { buf: Vec<u8> },
    FixedBody { remaining: u64 },
    Chunked(ChunkDecoder),
}

impl ReqParseState {
    fn feed(&mut self, mut bytes: &[u8], expected: &mut VecDeque<ExpectedResp>) -> bool {
        while !bytes.is_empty() {
            match self {
                Self::Passthrough => return false,
                Self::Headers { buf } => {
                    let prev_len = buf.len();
                    let search_from = prev_len.saturating_sub(3);
                    let take = bytes.len().min(65536usize.saturating_sub(prev_len).max(4));
                    buf.extend_from_slice(&bytes[..take]);
                    if buf.len() >= 14 && trim_ascii_crlf(buf).starts_with(b"PRI * HTTP/2.0") {
                        *self = Self::Passthrough;
                        return true;
                    }
                    let Some(rel_end) = find_header_end(&buf[search_from..]) else {
                        if buf.len() >= 65536 {
                            *self = Self::Passthrough;
                            return false;
                        }
                        bytes = &bytes[take..];
                        continue;
                    };
                    let end = search_from + rel_end + 4;
                    let consumed = end - prev_len;
                    bytes = &bytes[consumed..];
                    let trimmed = trim_ascii_crlf(&buf[..end]);
                    let is_head = trimmed.len() >= 5 && trimmed[..5].eq_ignore_ascii_case(b"HEAD ");
                    let (_, is_chunked, content_length) = parse_http_headers_meta(trimmed);
                    expected.push_back(ExpectedResp { is_head });
                    buf.clear();
                    if is_chunked {
                        *self = Self::Chunked(ChunkDecoder::new());
                    } else if let Some(len) = content_length.filter(|&n| n > 0) {
                        *self = Self::FixedBody { remaining: len };
                    }
                }
                Self::FixedBody { remaining } => {
                    let take = (*remaining).min(bytes.len() as u64) as usize;
                    *remaining -= take as u64;
                    bytes = &bytes[take..];
                    if *remaining == 0 {
                        *self = Self::Headers { buf: Vec::new() };
                    }
                }
                Self::Chunked(decoder) => match decoder.advance(bytes) {
                    ChunkStep::NeedMore => return false,
                    ChunkStep::Done { consumed } => {
                        bytes = &bytes[consumed..];
                        *self = Self::Headers { buf: Vec::new() };
                    }
                    ChunkStep::Invalid => {
                        *self = Self::Passthrough;
                        return false;
                    }
                },
            }
        }
        false
    }
}

enum RespState {
    Passthrough,
    ProbingHead { head_probe: Vec<u8> },
    FixedBody { remaining: u64 },
    Chunked(ChunkDecoder),
    CloseDelimited,
}

enum SseIoState {
    Active(tokio::net::TcpStream),
    HandoffDone,
}

struct SseIo {
    state: SseIoState,
    req_parser: ReqParseState,
    expected_responses: VecDeque<ExpectedResp>,
    resp_state: RespState,
    pending_out: Vec<u8>,
    pending_offset: usize,
    sse_tasks: SseTaskTracker,
}

impl SseIo {
    fn new(stream: tokio::net::TcpStream, sse_tasks: SseTaskTracker) -> Self {
        Self {
            state: SseIoState::Active(stream),
            req_parser: ReqParseState::Headers { buf: Vec::new() },
            expected_responses: VecDeque::new(),
            resp_state: RespState::ProbingHead {
                head_probe: Vec::new(),
            },
            pending_out: Vec::new(),
            pending_offset: 0,
            sse_tasks,
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
            let h2 = this
                .req_parser
                .feed(&buf.filled()[before..], &mut this.expected_responses);
            if h2
                && matches!(
                    &this.resp_state,
                    RespState::ProbingHead { head_probe } if head_probe.is_empty()
                )
            {
                this.resp_state = RespState::Passthrough;
            }
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
        if data.is_empty() {
            return Poll::Ready(Ok(0));
        }
        match &mut this.resp_state {
            RespState::Passthrough | RespState::CloseDelimited => {
                let SseIoState::Active(stream) = &mut this.state else {
                    return Poll::Ready(Ok(data.len()));
                };
                Pin::new(stream).poll_write(cx, data)
            }
            RespState::FixedBody { remaining } => {
                let limit = (*remaining).min(data.len() as u64) as usize;
                let SseIoState::Active(stream) = &mut this.state else {
                    return Poll::Ready(Ok(data.len()));
                };
                let poll = Pin::new(stream).poll_write(cx, &data[..limit]);
                if let Poll::Ready(Ok(n)) = &poll {
                    *remaining -= *n as u64;
                    if *remaining == 0 {
                        this.resp_state = RespState::ProbingHead {
                            head_probe: Vec::new(),
                        };
                    }
                }
                poll
            }
            RespState::Chunked(decoder) => {
                let mut probe = decoder.clone();
                let (limit, invalid) = match probe.advance(data) {
                    ChunkStep::NeedMore => (data.len(), false),
                    ChunkStep::Done { consumed } => (consumed, false),
                    ChunkStep::Invalid => (data.len(), true),
                };
                let SseIoState::Active(stream) = &mut this.state else {
                    return Poll::Ready(Ok(data.len()));
                };
                let poll = Pin::new(stream).poll_write(cx, &data[..limit]);
                if let Poll::Ready(Ok(n)) = &poll {
                    if invalid {
                        this.resp_state = RespState::Passthrough;
                    } else if matches!(decoder.advance(&data[..*n]), ChunkStep::Done { .. }) {
                        this.resp_state = RespState::ProbingHead {
                            head_probe: Vec::new(),
                        };
                    }
                }
                poll
            }
            RespState::ProbingHead { head_probe } => {
                let prev_len = head_probe.len();
                let search_from = prev_len.saturating_sub(3);
                head_probe.extend_from_slice(data);
                if head_probe.len() >= 5 && !head_probe.starts_with(b"HTTP/") {
                    this.pending_out = std::mem::take(head_probe);
                    this.pending_offset = 0;
                    this.resp_state = RespState::Passthrough;
                    if let Poll::Ready(Err(err)) = this.drain_pending(cx) {
                        return Poll::Ready(Err(err));
                    }
                    return Poll::Ready(Ok(data.len()));
                }
                if let Some(rel_end) = find_header_end(&head_probe[search_from..]) {
                    let header_end = search_from + rel_end;
                    let end = header_end + 4;
                    let consumed = end - prev_len;
                    if let Some(token) = extract_sse_handoff_token(&head_probe[..header_end])
                        && let Some(reg) = take_sse_handoff(&token)
                    {
                        let _ = this.expected_responses.pop_front();
                        let clean_head = rewrite_sse_response_head(&head_probe[..header_end]);
                        head_probe.clear();
                        this.pending_out.clear();
                        this.pending_offset = 0;
                        this.resp_state = RespState::Passthrough;
                        if let SseIoState::Active(stream) =
                            std::mem::replace(&mut this.state, SseIoState::HandoffDone)
                        {
                            this.sse_tasks.spawn(stream, clean_head, reg);
                        }
                        return Poll::Ready(Ok(data.len()));
                    }
                    let (status, is_chunked, content_length) =
                        parse_http_headers_meta(&head_probe[..header_end]);
                    let is_interim =
                        status.is_some_and(|code| (100..200).contains(&code) && code != 101);
                    let req = if is_interim {
                        ExpectedResp { is_head: false }
                    } else {
                        this.expected_responses
                            .pop_front()
                            .unwrap_or(ExpectedResp { is_head: false })
                    };
                    let next_state = if status == Some(101) {
                        RespState::Passthrough
                    } else if is_interim || req.is_head || matches!(status, Some(204 | 304)) {
                        RespState::ProbingHead {
                            head_probe: Vec::new(),
                        }
                    } else if is_chunked {
                        RespState::Chunked(ChunkDecoder::new())
                    } else if let Some(len) = content_length {
                        if len == 0 {
                            RespState::ProbingHead {
                                head_probe: Vec::new(),
                            }
                        } else {
                            RespState::FixedBody { remaining: len }
                        }
                    } else {
                        RespState::CloseDelimited
                    };
                    head_probe.truncate(end);
                    this.pending_out = std::mem::take(head_probe);
                    this.pending_offset = 0;
                    this.resp_state = next_state;
                    if let Poll::Ready(Err(err)) = this.drain_pending(cx) {
                        return Poll::Ready(Err(err));
                    }
                    return Poll::Ready(Ok(consumed));
                }
                if head_probe.len() >= 8192 {
                    this.pending_out = std::mem::take(head_probe);
                    this.pending_offset = 0;
                    this.resp_state = RespState::Passthrough;
                    if let Poll::Ready(Err(err)) = this.drain_pending(cx) {
                        return Poll::Ready(Err(err));
                    }
                }
                Poll::Ready(Ok(data.len()))
            }
        }
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        if matches!(
            self.resp_state,
            RespState::Passthrough | RespState::CloseDelimited
        ) && self.pending_offset >= self.pending_out.len()
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
        matches!(
            self.resp_state,
            RespState::Passthrough | RespState::CloseDelimited
        ) && self.pending_offset >= self.pending_out.len()
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

struct SseListener {
    listener: tokio::net::TcpListener,
    tasks: SseTaskTracker,
}

impl axum::serve::Listener for SseListener {
    type Io = SseIo;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.listener.accept().await {
                Ok((stream, addr)) => return (SseIo::new(stream, self.tasks.clone()), addr),
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.listener.local_addr()
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
    let encoding = preferred_encoding(request.headers(), true, true);
    match serve(&app, request, head, encoding).await {
        Ok(response) => response,
        Err(error) => response_with_encoding(
            error.status,
            Payload::Json(json!({"error":error.message})),
            encoding,
            head,
        )
        .unwrap_or_else(|_| Response::new(Body::empty())),
    }
}
async fn serve(
    app: &App,
    request: Request<Body>,
    head: bool,
    encoding: AssetEncoding,
) -> Result<Response<Body>> {
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
        let allowed: &[&str] = if path == "/" {
            &["v", "project", "view", "session"]
        } else {
            &["v"]
        };
        let q = query(request.uri().query(), allowed)?;
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
        return response_with_encoding(
            200,
            Payload::Json(json!({"ok":true,"instance":app.instance})),
            encoding,
            head,
        );
    }
    if route.operation == "shutdown" {
        if body.as_object().is_none_or(|m| !m.is_empty()) {
            return Err(HarnessError::invalid("Invalid shutdown request."));
        }
        let _ = app.shutdown.send(true);
        return response_with_encoding(200, Payload::Json(json!({"ok":true})), encoding, head);
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
    response_with_encoding_async(status, payload, encoding, head).await
}
pub async fn serve_listener(listener: tokio::net::TcpListener, app: App) -> Result<()> {
    let mut shutdown = app.shutdown.subscribe();
    let sse_tasks = SseTaskTracker::default();
    let mut abort_guard = SseTaskAbortGuard(Some(sse_tasks.clone()));
    let result = axum::serve(
        SseListener {
            listener,
            tasks: sse_tasks.clone(),
        },
        app.router(),
    )
    .with_graceful_shutdown(async move {
        while !*shutdown.borrow() {
            if shutdown.changed().await.is_err() {
                break;
            }
        }
    })
    .await;
    abort_guard.0 = None;
    sse_tasks.shutdown(std::time::Duration::from_secs(2)).await;
    result.map_err(Into::into)
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

    #[tokio::test]
    async fn json_compression_and_sse_filter_routes_behave_consistently() {
        let small = response_with_encoding(
            200,
            Payload::Json(json!({"ok": true})),
            AssetEncoding::Brotli,
            false,
        )
        .unwrap();
        assert_eq!(small.headers().get(header::CONTENT_ENCODING), None);

        let large_val = json!({"items": vec!["alpha_beta_gamma_payload"; 120]});
        let br_resp = response_with_encoding(
            200,
            Payload::Json(large_val.clone()),
            AssetEncoding::Brotli,
            false,
        )
        .unwrap();
        assert_eq!(
            br_resp
                .headers()
                .get(header::CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok()),
            Some("br")
        );
        assert_eq!(
            br_resp
                .headers()
                .get(header::VARY)
                .and_then(|v| v.to_str().ok()),
            Some("Accept-Encoding")
        );
        let br_bytes = to_bytes(br_resp.into_body(), usize::MAX).await.unwrap();
        let mut br_decoded = Vec::new();
        std::io::Read::read_to_end(
            &mut brotli::Decompressor::new(br_bytes.as_ref(), 4096),
            &mut br_decoded,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&br_decoded).unwrap(),
            large_val
        );

        let gz_resp = response_with_encoding(
            200,
            Payload::Json(large_val.clone()),
            AssetEncoding::Gzip,
            false,
        )
        .unwrap();
        assert_eq!(
            gz_resp
                .headers()
                .get(header::CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok()),
            Some("gzip")
        );
        let gz_bytes = to_bytes(gz_resp.into_body(), usize::MAX).await.unwrap();
        let mut gz_decoded = Vec::new();
        std::io::Read::read_to_end(
            &mut flate2::read::GzDecoder::new(gz_bytes.as_ref()),
            &mut gz_decoded,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&gz_decoded).unwrap(),
            large_val
        );

        let head_resp =
            response_with_encoding(200, Payload::Json(large_val), AssetEncoding::Brotli, true)
                .unwrap();
        assert!(
            to_bytes(head_resp.into_body(), usize::MAX)
                .await
                .unwrap()
                .is_empty()
        );

        for (op, expected) in [
            ("graph.file_impact", &["from", "to", "agent"][..]),
            ("system_graph.service_file_impact", &["from", "to"][..]),
            ("system_graph.changes", &["from", "to"][..]),
            ("system_graph.impact", &["from", "to"][..]),
            ("system_graph.bridge", &["target", "run_id"][..]),
        ] {
            let route = ROUTES.iter().find(|r| r.operation == op).unwrap();
            for q in expected {
                assert!(route.query.contains(q), "{op} missing query param {q}");
            }
        }

        let lag = StreamFrame {
            seq: 1,
            event: "resync",
            session_id: None,
            project_id: None,
            data: json!({"reason": "graph_bridge_lagged"}),
        };
        assert!(matches_sse_filter(&lag, Some("s1"), Some("p1")));
        let sys = StreamFrame {
            seq: 2,
            event: "system_graph_status",
            session_id: None,
            project_id: Some("p1".into()),
            data: json!({"action": "disable"}),
        };
        assert!(matches_sse_filter(&sys, Some("s1"), Some("p1")));
        assert!(!matches_sse_filter(&sys, Some("s1"), Some("p2")));

        // Verify both `graph.map` and `system_graph.service_map` advertise `lod` & `min_weight`.
        let gmap = ROUTES.iter().find(|r| r.operation == "graph.map").unwrap();
        assert!(gmap.query.contains(&"lod"));
        assert!(gmap.query.contains(&"min_weight"));
        let smap = ROUTES
            .iter()
            .find(|r| r.operation == "system_graph.service_map")
            .unwrap();
        assert!(smap.query.contains(&"lod"));
        assert!(smap.query.contains(&"min_weight"));

        // `>= 16 KiB` JSON offloaded to `spawn_blocking` round-trips cleanly via Brotli & Gzip.
        let huge = json!({
            "items": (0..600).map(|i| format!("pkg/module_{i:04}/handler.rs")).collect::<Vec<_>>()
        });
        let br_async = response_with_encoding_async(
            200,
            Payload::Json(huge.clone()),
            AssetEncoding::Brotli,
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            br_async
                .headers()
                .get(header::CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok()),
            Some("br")
        );
        let br_raw = to_bytes(br_async.into_body(), usize::MAX).await.unwrap();
        let mut br_dec = Vec::new();
        std::io::Read::read_to_end(
            &mut brotli::Decompressor::new(br_raw.as_ref(), 4096),
            &mut br_dec,
        )
        .unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&br_dec).unwrap(), huge);

        let gz_async = response_with_encoding_async(
            200,
            Payload::Json(huge.clone()),
            AssetEncoding::Gzip,
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            gz_async
                .headers()
                .get(header::CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok()),
            Some("gzip")
        );
        let gz_raw = to_bytes(gz_async.into_body(), usize::MAX).await.unwrap();
        let mut gz_dec = Vec::new();
        std::io::Read::read_to_end(
            &mut flate2::read::GzDecoder::new(gz_raw.as_ref()),
            &mut gz_dec,
        )
        .unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&gz_dec).unwrap(), huge);
    }

    struct StubServices;

    impl crate::services::Services for StubServices {
        fn request<'a>(
            &'a self,
            _operation: &'a str,
            _request: Value,
        ) -> crate::services::Reply<'a> {
            Box::pin(async { Ok(Payload::Json(json!({}))) })
        }

        fn close<'a>(&'a self) -> crate::services::CloseFuture<'a> {
            Box::pin(async { Ok(()) })
        }
    }

    fn test_http_app(port: u16) -> App {
        let mut assets = BTreeMap::new();
        assets.insert(
            "/".into(),
            Asset::from_bytes(
                b"<!doctype html><html><body>Harness</body></html>".to_vec(),
                "text/html; charset=utf-8",
            ),
        );
        assets.insert(
            "/app.css".into(),
            Asset::from_bytes(b"body{margin:0}".to_vec(), "text/css; charset=utf-8"),
        );
        let (shutdown, _) = tokio::sync::watch::channel(false);
        App {
            services: Arc::new(StubServices),
            token: "test-csrf-token".into(),
            instance: "test-instance".into(),
            port,
            source: PathBuf::from("."),
            shutdown,
            assets: Arc::new(assets),
        }
    }

    #[tokio::test]
    async fn root_route_accepts_spa_deep_link_query_params_while_keeping_assets_strict() {
        let app = test_http_app(8765);
        let root_version = app.assets["/"].version.clone();
        let css_version = app.assets["/app.css"].version.clone();

        let send = |method: &str, uri: &str| {
            let app = app.clone();
            let req = Request::builder()
                .method(method)
                .uri(uri)
                .header(header::HOST, "127.0.0.1:8765")
                .body(Body::empty())
                .unwrap();
            async move { handle(State(app), req).await }
        };

        let root = send("GET", "/?view=sessions&project=foo").await;
        assert_eq!(root.status(), StatusCode::OK);
        assert_eq!(
            root.headers()
                .get(header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("no-cache")
        );
        let body = to_bytes(root.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            body.as_ref(),
            b"<!doctype html><html><body>Harness</body></html>"
        );

        let with_session = send("GET", "/?project=foo&view=sessions&session=s-1").await;
        assert_eq!(with_session.status(), StatusCode::OK);

        let root_head = send("HEAD", "/?view=sessions&project=foo").await;
        assert_eq!(root_head.status(), StatusCode::OK);
        assert!(
            to_bytes(root_head.into_body(), usize::MAX)
                .await
                .unwrap()
                .is_empty()
        );

        let versioned_root = send("GET", &format!("/?v={root_version}&project=foo")).await;
        assert_eq!(versioned_root.status(), StatusCode::OK);
        assert_eq!(
            versioned_root
                .headers()
                .get(header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("private, max-age=31536000, immutable")
        );

        let unknown_root = send("GET", "/?unknown=1").await;
        assert_eq!(unknown_root.status(), StatusCode::BAD_REQUEST);

        let css_with_project = send("GET", "/app.css?project=foo").await;
        assert_eq!(css_with_project.status(), StatusCode::BAD_REQUEST);

        let css_versioned = send("GET", &format!("/app.css?v={css_version}")).await;
        assert_eq!(css_versioned.status(), StatusCode::OK);
        assert_eq!(
            css_versioned
                .headers()
                .get(header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("private, max-age=31536000, immutable")
        );
    }

    #[test]
    fn sse_filter_allows_global_waiting_counts_while_scoping_session_status() {
        let waiting = StreamFrame {
            seq: 10,
            event: "waiting_counts",
            session_id: Some("s-b".into()),
            project_id: Some("proj-b".into()),
            data: json!({"counts": {"s-b": 1}, "total": 1, "session_id": "s-b"}),
        };
        assert!(matches_sse_filter(&waiting, Some("s-a"), Some("proj-a")));
        assert!(matches_sse_filter(&waiting, None, Some("proj-a")));

        let status = StreamFrame {
            seq: 11,
            event: "session_status",
            session_id: Some("s-b".into()),
            project_id: Some("proj-b".into()),
            data: json!({"id": "s-b", "waiting_permissions": 0}),
        };
        assert!(!matches_sse_filter(&status, Some("s-a"), Some("proj-a")));
        assert!(!matches_sse_filter(&status, None, Some("proj-a")));
        assert!(matches_sse_filter(&status, None, Some("proj-b")));
        assert!(!matches_sse_filter(&status, Some("s-a"), Some("proj-b")));
        assert!(matches_sse_filter(&status, Some("s-b"), Some("proj-b")));
    }
}

#[cfg(test)]
mod skillcache_route_tests {
    use super::*;

    #[test]
    fn skillcache_routes_guard_every_change_with_the_token() {
        let route = |operation: &str| {
            ROUTES
                .iter()
                .find(|route| route.operation == operation)
                .unwrap_or_else(|| panic!("no route for {operation}"))
        };
        for (operation, method, token, status) in [
            ("skillcache.session", "GET", false, 200),
            ("skillcache.list", "GET", false, 200),
            ("skillcache.get", "GET", false, 200),
            ("skillcache.compile", "POST", true, 202),
            ("skillcache.requalify", "POST", true, 202),
            ("skillcache.invalidate", "POST", true, 200),
            ("skillcache.retire", "POST", true, 200),
            ("skillcache.delete", "DELETE", true, 200),
        ] {
            let found = route(operation);
            assert_eq!(
                (found.method, found.token, found.status),
                (method, token, status),
                "{operation}"
            );
            assert!(
                found
                    .pattern
                    .starts_with("/api/projects/{project_id}/skillcache/")
                    || found
                        .pattern
                        .starts_with("/api/sessions/{session_id}/skillcache"),
                "{operation} is nested under its project or session"
            );
        }
        assert_eq!(
            route("skillcache.list").query,
            &["skill", "status", "form", "include_retired", "include_eval"]
        );
        // A capsule is compiled from an observed turn: there is no route that
        // compiles "a skill".
        assert!(
            !ROUTES.iter().any(|route| route.pattern.contains("/skills/")
                && route.operation.starts_with("skillcache."))
        );
    }

    #[test]
    fn sse_filter_exempts_skillcache_capsule() {
        let frame = StreamFrame {
            seq: 1,
            event: "skillcache_capsule",
            session_id: None,
            project_id: Some("p1".into()),
            data: json!({"state": "qualified"}),
        };
        // A client that follows one session still hears about its project's
        // capsules; a client of another project does not.
        assert!(matches_sse_filter(&frame, Some("s1"), Some("p1")));
        assert!(matches_sse_filter(&frame, Some("s1"), None));
        assert!(!matches_sse_filter(&frame, Some("s1"), Some("p2")));
    }
}
