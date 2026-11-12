use crate::{
    HarnessError, Result,
    services::{Payload, Services},
    store,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::State,
    http::{Request, Response, StatusCode, header},
};
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
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
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub etag: String,
    pub version: String,
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
            Ok(Asset {
                bytes,
                content_type: mime.into(),
                etag: format!("\"{}\"", &digest[..32]),
                version: digest[..16].into(),
            })
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
        let boot_text = String::from_utf8(boot.bytes.clone())
            .map_err(|_| HarnessError::invalid("Native UI loader is not UTF-8."))?
            .replace(
                "/assets/infra_ui.js",
                &format!("/assets/infra_ui.js?v={js}"),
            )
            .replace(
                "/assets/infra_ui_bg.wasm",
                &format!("/assets/infra_ui_bg.wasm?v={binary}"),
            );
        boot.bytes = boot_text.into_bytes();
        let digest = store::sha(&boot.bytes);
        boot.etag = format!("\"{}\"", &digest[..32]);
        boot.version = digest[..16].into();
        let kit_bytes = infra_tooling::kit::http_catalog(&source)?;
        let kit_digest = store::sha(&kit_bytes);
        let mut kit = Asset {
            bytes: kit_bytes,
            content_type: "text/html; charset=utf-8".into(),
            etag: format!("\"{}\"", &kit_digest[..32]),
            version: kit_digest[..16].into(),
        };
        let mut kit_text = String::from_utf8(kit.bytes)
            .map_err(|_| HarnessError::invalid("Native Kit 3 catalog is not UTF-8."))?;
        for (path, asset) in &assets {
            for attr in ["src", "href"] {
                kit_text = kit_text.replace(
                    &format!("{attr}=\"{path}\""),
                    &format!("{attr}=\"{path}?v={}\"", asset.version),
                );
            }
        }
        kit.bytes = kit_text.into_bytes();
        let digest = store::sha(&kit.bytes);
        kit.etag = format!("\"{}\"", &digest[..32]);
        kit.version = digest[..16].into();
        assets.insert("/kit3/".into(), kit);
        let mut page = assets
            .remove("/index.html")
            .ok_or_else(|| HarnessError::missing("Harness index.html is missing."))?;
        let mut text = String::from_utf8(page.bytes)
            .map_err(|_| HarnessError::invalid("Harness page is not UTF-8."))?;
        for (path, asset) in &assets {
            for attr in ["src", "href"] {
                text = text.replace(
                    &format!("{attr}=\"{path}\""),
                    &format!("{attr}=\"{path}?v={}\"", asset.version),
                );
            }
        }
        page.bytes = text.into_bytes();
        page.etag = format!("\"{}\"", &store::sha(&page.bytes)[..32]);
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
            "scope", "focus", "certain", "filter", "target", "follow", "out", "into", "notests",
            "spread",
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
        query: &["after"],
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
    let mut out=Response::builder().status(StatusCode::from_u16(status).map_err(|_|HarnessError::invalid("Invalid response status."))?).header(header::CONTENT_TYPE,content_type).header(header::CONTENT_LENGTH,bytes.len().to_string()).header("X-Content-Type-Options","nosniff").header("Referrer-Policy","no-referrer").header("Cross-Origin-Resource-Policy","same-origin").header("Content-Security-Policy","default-src 'self'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-src 'self'; frame-ancestors 'self'; base-uri 'none'; object-src 'none'; form-action 'self'").header(header::CACHE_CONTROL,cache.unwrap_or("no-store".into()));
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
        if request
            .headers()
            .get(header::IF_NONE_MATCH)
            .and_then(|h| h.to_str().ok())
            .is_some_and(|value| {
                value
                    .split(',')
                    .any(|offered| offered.trim() == asset.etag || offered.trim() == "*")
            })
        {
            return Response::builder()
                .status(304)
                .header(header::ETAG, &asset.etag)
                .body(Body::empty())
                .map_err(|_| HarnessError::unavailable("Could not construct HTTP response."));
        }
        let payload = Payload::Bytes {
            bytes: asset.bytes.clone(),
            content_type: asset.content_type.clone(),
            filename: None,
            etag: Some(asset.etag.clone()),
            cache: Some(
                if immutable {
                    "private, max-age=31536000, immutable"
                } else {
                    "no-cache"
                }
                .into(),
            ),
        };
        return response(200, payload, head);
    }
    let (route, params) = route(&method, &path)?;
    let query = query(request.uri().query(), route.query)?;
    if route.token || method == "POST" {
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
    let body = if method == "POST" {
        if request.headers().get(header::TRANSFER_ENCODING).is_some()
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
        let upload = route.operation == "sessions.create" || route.operation == "sessions.send";
        let limit = if upload {
            crate::attachments::MAX_JSON_BYTES
        } else if route.operation == "skills.create_preview" {
            131072
        } else if ["systems.preview", "discovery.start"].contains(&route.operation) {
            2 * 1024 * 1024
        } else {
            65536
        };
        let length = request
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|h| h.to_str().ok())
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
        value
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
    axum::serve(listener, app.router())
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
