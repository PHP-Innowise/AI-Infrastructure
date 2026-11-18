//! Phase 3C System Orchestration permissions (`auto` / `manual` / `bypass`),
//! cross-service & contract-ownership boundary escalation, non-interactive write checks,
//! and `SystemManager` / `native` / `workers` permissions plumbing.

use infra_core::Result;
use infra_host::{
    permission::{Answer, Approvals, Decision, PermissionAsk, PermissionMode},
    provider::PreparedCommand,
};
use infra_system::{
    CodeGraphBridge, EvidenceFileChange, Project, ServiceGraphOffer, SessionBridge, SystemManager,
    boundary::{
        BoundaryScope, SystemBoundaryApprovals, check_step_boundary_writes,
        check_step_boundary_writes_with_extra, detect_boundary_violation,
    },
    catalog::System,
    manager::RUNNER_EVENTS,
    native::{
        CURSOR_PROMPT_BYTES, create_run_with_permissions, prepare_launch_with_permissions,
        prompt_for, validate_state_sync,
    },
    prepare_worker_for_transport, run_system_request_with_adapter_and_approvals,
    run_system_request_with_bridge_and_approvals,
    runner::RunnerGraphHooks,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

struct TempRoot(PathBuf);
impl TempRoot {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "infra-system-perm-boundary-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        infra_core::fs::ensure_directory(&dir, true).unwrap();
        Self(infra_core::fs::canonicalize(dir).unwrap())
    }
}
impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_string_pretty(value).unwrap() + "\n").unwrap();
}

fn write_text(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
}

fn build_two_service_system(root: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let sys_dir = root.join("sys");
    let orders_dir = root.join("orders");
    let billing_dir = root.join("billing");

    write_text(&sys_dir.join("AGENTS.md"), "# System Policy\n");
    write_text(&sys_dir.join("contracts/shared.md"), "# Shared Contract\n");
    let config_path = sys_dir.join("system.json");
    write_json(
        &config_path,
        &json!({
            "schema_version": 1,
            "name": "perm-system",
            "services": [
                { "id": "orders", "root": "../orders", "manifest": "ai-service.json" },
                { "id": "billing", "root": "../billing", "manifest": "ai-service.json" }
            ],
            "shared_sources": [
                { "kind": "policy", "path": "AGENTS.md" },
                { "kind": "contract", "path": "contracts/shared.md" }
            ]
        }),
    );

    write_text(&orders_dir.join("src/order.php"), "<?php // order\n");
    write_text(&orders_dir.join("src/internal.php"), "<?php // internal\n");
    write_json(
        &orders_dir.join("ai-service.json"),
        &json!({
            "schema_version": 1,
            "id": "orders",
            "owner": "@orders",
            "description": "Orders",
            "capabilities": [{ "id": "orders.place", "description": "Place order", "status": "implemented", "sources": ["src/order.php"] }],
            "provides": [{ "id": "order.placed", "kind": "event", "version": "1", "sources": ["src/order.php"] }],
            "consumes": [{ "service": "billing", "contract": "billing.api", "version": "1" }],
            "sources": [{ "kind": "code", "path": "src/order.php" }]
        }),
    );

    write_text(&billing_dir.join("src/api.php"), "<?php // billing api\n");
    write_text(
        &billing_dir.join("src/consumer.php"),
        "<?php // billing consumer\n",
    );
    write_json(
        &billing_dir.join("ai-service.json"),
        &json!({
            "schema_version": 1,
            "id": "billing",
            "owner": "@billing",
            "description": "Billing",
            "capabilities": [{ "id": "billing.charge", "description": "Charge order", "status": "implemented", "sources": ["src/consumer.php"] }],
            "provides": [{ "id": "billing.api", "kind": "http", "version": "1", "sources": ["src/api.php"] }],
            "consumes": [{ "service": "orders", "contract": "order.placed", "version": "1" }],
            "sources": [{ "kind": "code", "path": "src/api.php" }, { "kind": "code", "path": "src/consumer.php" }]
        }),
    );

    (config_path, sys_dir, orders_dir, billing_dir)
}

#[derive(Default)]
struct RecordingApprovals {
    seen: Mutex<Vec<PermissionAsk>>,
    withdrawn: Mutex<Vec<String>>,
    next_decision: Mutex<Option<Decision>>,
}
impl RecordingApprovals {
    fn allowing() -> Arc<Self> {
        Arc::new(Self {
            next_decision: Mutex::new(Some(Decision::Allow)),
            ..Default::default()
        })
    }
    fn denying() -> Arc<Self> {
        Arc::new(Self {
            next_decision: Mutex::new(Some(Decision::Deny)),
            ..Default::default()
        })
    }
}
impl Approvals for RecordingApprovals {
    fn ask(
        &self,
        ask: &PermissionAsk,
    ) -> infra_host::Result<tokio::sync::oneshot::Receiver<Decision>> {
        self.seen.lock().unwrap().push(ask.clone());
        let (tx, rx) = tokio::sync::oneshot::channel();
        let d = self
            .next_decision
            .lock()
            .unwrap()
            .unwrap_or(Decision::Allow);
        let _ = tx.send(d);
        Ok(rx)
    }
    fn withdraw(&self, id: &str) -> infra_host::Result<()> {
        self.withdrawn.lock().unwrap().push(id.to_string());
        Ok(())
    }
}

struct FakeBridge {
    projects: BTreeMap<String, Project>,
    lock_file: Arc<File>,
    provider_exe: PathBuf,
    sessions: Mutex<BTreeMap<String, Value>>,
    waiting: Mutex<BTreeMap<String, Vec<Value>>>,
    next_sid: AtomicU64,
}
impl SessionBridge for FakeBridge {
    fn projects(&self) -> Result<BTreeMap<String, Project>> {
        Ok(self.projects.clone())
    }
    fn provider(&self, _id: &str) -> Result<Option<PathBuf>> {
        Ok(Some(self.provider_exe.clone()))
    }
    fn active(&self) -> Result<bool> {
        Ok(false)
    }
    fn admit(&self) -> Result<()> {
        Ok(())
    }
    fn runner_lock(&self) -> Result<Arc<File>> {
        Ok(self.lock_file.clone())
    }
    fn launch(
        &self,
        request: &Value,
        system_run: Option<&Value>,
        _discovery: Option<&Value>,
    ) -> Result<Value> {
        let n = self.next_sid.fetch_add(1, Ordering::SeqCst) + 1;
        let sid = format!("{n:032x}");
        let session = json!({
            "id": sid,
            "provider": request["provider"],
            "status": "queued",
            "permissions": request["permissions"],
            "created_at": "2026-11-18T12:00:00Z",
            "updated_at": "2026-11-18T12:00:00Z",
            "finished_at": null,
            "request": request,
            "system_run": system_run
        });
        self.sessions
            .lock()
            .unwrap()
            .insert(sid.clone(), session.clone());
        Ok(session)
    }
    fn session(&self, id: &str) -> Result<Value> {
        Ok(self
            .sessions
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .unwrap_or_else(|| json!({"id": id, "status": "completed"})))
    }
    fn cancel(&self, id: &str) -> Result<Value> {
        self.session(id)
    }
    fn events(&self, _id: &str, _kinds: &[&str]) -> Result<Vec<Value>> {
        Ok(vec![])
    }
    fn find_launch(&self, _run_id: &str, _nonce: &str) -> Result<Option<Value>> {
        Ok(None)
    }
    fn waiting_permissions(&self, id: &str) -> Result<Vec<Value>> {
        Ok(self
            .waiting
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .unwrap_or_default())
    }
}

#[test]
fn system_manager_persists_validates_and_views_permissions_and_waiting_permissions() {
    assert!(RUNNER_EVENTS.contains(&"permission"));

    let root = TempRoot::new("mgr");
    let (_cfg, sys_dir, orders_dir, billing_dir) = build_two_service_system(&root.0);
    let state_dir = root.0.join("state");
    fs::create_dir_all(&state_dir).unwrap();
    let lock_file = Arc::new(File::create(root.0.join("runner.lock")).unwrap());
    let fake_cli = root.0.join("fake-cli");
    write_text(&fake_cli, "#!/bin/sh\nexit 0\n");

    let host = Arc::new(FakeBridge {
        projects: BTreeMap::from([
            (
                "sys".into(),
                Project {
                    id: "sys".into(),
                    name: "sys".into(),
                    path: sys_dir,
                },
            ),
            (
                "orders".into(),
                Project {
                    id: "orders".into(),
                    name: "orders".into(),
                    path: orders_dir,
                },
            ),
            (
                "billing".into(),
                Project {
                    id: "billing".into(),
                    name: "billing".into(),
                    path: billing_dir,
                },
            ),
        ]),
        lock_file,
        provider_exe: fake_cli,
        sessions: Mutex::new(BTreeMap::new()),
        waiting: Mutex::new(BTreeMap::new()),
        next_sid: AtomicU64::new(0),
    });

    let mgr = SystemManager::new(root.0.clone(), state_dir.clone(), host.clone()).unwrap();
    let prepared = mgr
        .execute(
            "systems.prepare",
            &json!({
                "project_id": "sys",
                "config_path": "system.json",
                "task": "Place order",
                "change_id": "chg-perm",
                "services": ["orders"]
            }),
        )
        .unwrap();
    assert_eq!(prepared["permissions"], Value::Null);
    assert_eq!(prepared["waiting_permissions"], json!([]));

    let rid = prepared["id"].as_str().unwrap().to_string();
    let rev = prepared["revision"].as_u64().unwrap();

    // Invalid permissions are rejected cleanly.
    let invalid = mgr
        .execute(
            "systems.act",
            &json!({
                "params": { "id": rid },
                "body": {
                    "action": "execute",
                    "revision": rev,
                    "provider": "claude",
                    "mode": "edit",
                    "permissions": "Auto"
                }
            }),
        )
        .unwrap_err();
    assert_eq!(
        invalid.message,
        "Choose Auto, Manual or Bypass permissions."
    );

    // Valid read-only launch preserves `"mode": "plan"` on host.launch even when permissions are passed.
    let ro_prepared = mgr
        .execute(
            "systems.prepare",
            &json!({
                "project_id": "sys",
                "config_path": "system.json",
                "task": "Inspect order",
                "change_id": "chg-ro",
                "services": ["orders"]
            }),
        )
        .unwrap();
    let ro_rid = ro_prepared["id"].as_str().unwrap().to_string();
    let ro_rev = ro_prepared["revision"].as_u64().unwrap();
    let ro_exec = mgr
        .execute(
            "systems.act",
            &json!({
                "params": { "id": ro_rid },
                "body": {
                    "action": "execute",
                    "revision": ro_rev,
                    "provider": "claude",
                    "mode": "read-only",
                    "permissions": "bypass"
                }
            }),
        )
        .unwrap();
    assert_eq!(ro_exec["permissions"], "bypass");
    let ro_sid = ro_exec["session_id"].as_str().unwrap();
    let ro_sess = host.session(ro_sid).unwrap();
    assert_eq!(ro_sess["request"]["mode"], "plan");
    assert_eq!(ro_sess["request"]["permissions"], "bypass");

    // Execute in edit mode with `"auto"`, verify waiting_permissions surfaces on detail view.
    let launched = mgr
        .execute(
            "systems.act",
            &json!({
                "params": { "id": rid },
                "body": {
                    "action": "execute",
                    "revision": rev,
                    "provider": "claude",
                    "mode": "edit",
                    "permissions": "auto"
                }
            }),
        )
        .unwrap();
    assert_eq!(launched["permissions"], "auto");
    let sid1 = launched["session_id"].as_str().unwrap().to_string();
    let sess1 = host.session(&sid1).unwrap();
    assert_eq!(sess1["request"]["mode"], "edit");
    assert_eq!(sess1["request"]["permissions"], "auto");

    let run_dir = state_dir.join("ai-system").join(&rid);
    let req1_file = fs::read_dir(&run_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().starts_with("request-"))
        .unwrap()
        .path();
    let req1: Value = serde_json::from_slice(&fs::read(req1_file).unwrap()).unwrap();
    assert_eq!(req1["permissions"], "auto");

    let pending_ev = json!({
        "kind": "permission",
        "state": "pending",
        "request_id": "claude-1-run",
        "summary": "../billing/src/api.php"
    });
    host.waiting
        .lock()
        .unwrap()
        .insert(sid1.clone(), vec![pending_ev.clone()]);
    let viewed = mgr
        .execute("systems.get", &json!({ "params": { "id": rid } }))
        .unwrap();
    assert_eq!(viewed["waiting_permissions"], json!([pending_ev]));

    // Simulate stopped session, then resume updating permissions to `"manual"`.
    host.sessions.lock().unwrap().get_mut(&sid1).unwrap()["status"] = json!("failed");
    let rev2 = viewed["revision"].as_u64().unwrap();
    let resumed = mgr
        .execute(
            "systems.act",
            &json!({
                "params": { "id": rid },
                "body": {
                    "action": "resume",
                    "revision": rev2,
                    "permissions": "manual"
                }
            }),
        )
        .unwrap();
    assert_eq!(resumed["permissions"], "manual");
    let sid2 = resumed["session_id"].as_str().unwrap().to_string();
    assert_ne!(sid1, sid2);
    assert_eq!(
        host.session(&sid2).unwrap()["request"]["permissions"],
        "manual"
    );

    // Resume omitting permissions preserves saved `"manual"`.
    host.sessions.lock().unwrap().get_mut(&sid2).unwrap()["status"] = json!("failed");
    let rev3 = resumed["revision"].as_u64().unwrap();
    let resumed2 = mgr
        .execute(
            "systems.act",
            &json!({
                "params": { "id": rid },
                "body": {
                    "action": "resume",
                    "revision": rev3
                }
            }),
        )
        .unwrap();
    assert_eq!(resumed2["permissions"], "manual");
}

#[test]
fn boundary_scope_from_run_loads_passport_and_plan_contract_sources() {
    let root = TempRoot::new("scope");
    let (config_path, sys_dir, orders_dir, billing_dir) = build_two_service_system(&root.0);
    let mut system = System::new(&config_path, std::slice::from_ref(&root.0)).unwrap();
    let plan = system
        .plan(
            "Update order",
            "chg-scope",
            &["orders".into()],
            &[],
            16000,
            2,
        )
        .unwrap();
    let state = json!({
        "selected": ["orders", "billing"],
        "access": "service",
        "roots": {
            "__system__": sys_dir,
            "orders": orders_dir,
            "billing": billing_dir
        }
    });
    let step = json!({
        "id": "service-orders",
        "service": "orders",
        "mode": "edit"
    });

    let scope = BoundaryScope::from_run(&system, &state, &plan, &step).unwrap();
    assert_eq!(scope.service, "orders");
    assert_eq!(scope.access, "service");
    assert_eq!(scope.writable_roots, vec![orders_dir.clone()]);
    assert!(
        scope
            .contract_sources
            .get(&orders_dir.join("src/order.php"))
            .is_some_and(|v| v.contains(&("orders".into(), "order.placed".into())))
    );
    assert!(
        scope
            .contract_sources
            .get(&billing_dir.join("src/api.php"))
            .is_some_and(|v| v.contains(&("billing".into(), "billing.api".into())))
    );
    assert!(
        scope
            .contract_sources
            .get(&sys_dir.join("contracts/shared.md"))
            .is_some_and(|v| v.iter().any(|(owner, _)| owner == "__system__"))
    );
    let outside_ask = PermissionAsk {
        provider: "claude".into(),
        tool: "Edit".into(),
        kind: "file_change".into(),
        summary: "../billing/src/api.php".into(),
        ..Default::default()
    };
    assert!(detect_boundary_violation(&outside_ask, &scope).is_some());
}

#[test]
fn system_boundary_approvals_auto_allows_safe_and_escalates_boundary_asks() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let scope = BoundaryScope {
        service: "orders".into(),
        mode: "edit".into(),
        access: "service".into(),
        own_root: PathBuf::from("/work/orders"),
        system_root: PathBuf::from("/work/sys"),
        selected_roots: BTreeMap::from([
            ("orders".into(), PathBuf::from("/work/orders")),
            ("billing".into(), PathBuf::from("/work/billing")),
        ]),
        writable_roots: vec![PathBuf::from("/work/orders")],
        contract_sources: BTreeMap::from([
            (
                PathBuf::from("/work/orders/src/order.php"),
                vec![("orders".into(), "order.placed".into())],
            ),
            (
                PathBuf::from("/work/billing/src/api.php"),
                vec![("billing".into(), "billing.api".into())],
            ),
        ]),
    };

    rt.block_on(async {
        let inner = RecordingApprovals::allowing();
        let auto_approvals = SystemBoundaryApprovals::new(
            Some(inner.clone()),
            Some(PermissionMode::Auto),
            scope.clone(),
        );

        // Safe edit inside own_root (even own contract source) succeeds without prompting inner.
        let safe_edit = PermissionAsk {
            id: "claude-1".into(),
            provider: "claude".into(),
            tool: "Edit".into(),
            kind: "file_change".into(),
            summary: "src/order.php".into(),
            session_option: true,
            ..Default::default()
        };
        let ans = auto_approvals
            .ask_answer(&safe_edit)
            .unwrap()
            .await
            .unwrap();
        assert_eq!(ans, Answer::from_decision(Decision::Allow));
        assert!(inner.seen.lock().unwrap().is_empty());

        // Safe command in own root (with >/dev/null) does not false-positive.
        let safe_cmd = PermissionAsk {
            id: "claude-2".into(),
            provider: "claude".into(),
            tool: "Bash".into(),
            kind: "command".into(),
            summary: "phpunit tests/OrderTest.php 2>/dev/null".into(),
            detail: Some("Run unit tests · in /work/orders".into()),
            session_option: true,
            ..Default::default()
        };
        let ans = auto_approvals.ask_answer(&safe_cmd).unwrap().await.unwrap();
        assert_eq!(ans, Answer::from_decision(Decision::Allow));
        assert!(inner.seen.lock().unwrap().is_empty());

        // Question under Auto declines immediately without prompting.
        let question = PermissionAsk {
            id: "claude-q".into(),
            provider: "claude".into(),
            tool: "AskUserQuestion".into(),
            kind: "question".into(),
            summary: "Pick?".into(),
            ..Default::default()
        };
        let ans = auto_approvals.ask_answer(&question).unwrap().await.unwrap();
        assert_eq!(ans, Answer::from_decision(Decision::Deny));
        assert!(inner.seen.lock().unwrap().is_empty());

        // Cross-service write escalates to Manual with session_option=false and `[Auto -> Manual boundary escalation: ...]`!
        let cross_ask = PermissionAsk {
            id: "claude-3".into(),
            provider: "claude".into(),
            tool: "Edit".into(),
            kind: "file_change".into(),
            summary: "../billing/src/consumer.php".into(),
            detail: Some("Update consumer".into()),
            session_option: true,
            ..Default::default()
        };
        let ans = auto_approvals
            .ask_answer(&cross_ask)
            .unwrap()
            .await
            .unwrap();
        assert_eq!(ans, Answer::from_decision(Decision::Allow));
        let seen = inner.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        assert!(!seen[0].session_option);
        let detail = seen[0].detail.as_deref().unwrap();
        assert!(
            detail.starts_with("[Auto -> Manual boundary escalation:"),
            "{detail}"
        );
        assert!(detail.ends_with("Update consumer"), "{detail}");

        // Withdraw delegates to inner.
        auto_approvals.withdraw("claude-3").unwrap();
        assert_eq!(inner.withdrawn.lock().unwrap().as_slice(), &["claude-3"]);

        // Auto with inner: None denies boundary escalations via ready oneshot.
        let no_inner_auto =
            SystemBoundaryApprovals::new(None, Some(PermissionMode::Auto), scope.clone());
        let ans = no_inner_auto.ask_answer(&cross_ask).unwrap().await.unwrap();
        assert_eq!(ans, Answer::from_decision(Decision::Deny));

        // Bypass allows immediately without prompting.
        let bypass =
            SystemBoundaryApprovals::new(Some(inner.clone()), Some(PermissionMode::Bypass), scope);
        let ans = bypass.ask_answer(&cross_ask).unwrap().await.unwrap();
        assert_eq!(ans, Answer::from_decision(Decision::Allow));
        assert_eq!(inner.seen.lock().unwrap().len(), 1);
    });
}

#[test]
fn system_boundary_approvals_manual_annotates_boundary_asks_and_handles_missing_inner() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let scope = BoundaryScope {
        service: "orders".into(),
        mode: "edit".into(),
        access: "all".into(),
        own_root: PathBuf::from("/work/orders"),
        system_root: PathBuf::from("/work/sys"),
        selected_roots: BTreeMap::from([
            ("orders".into(), PathBuf::from("/work/orders")),
            ("billing".into(), PathBuf::from("/work/billing")),
        ]),
        writable_roots: vec![
            PathBuf::from("/work/orders"),
            PathBuf::from("/work/billing"),
        ],
        contract_sources: BTreeMap::from([(
            PathBuf::from("/work/billing/src/api.php"),
            vec![("billing".into(), "billing.api".into())],
        )]),
    };

    rt.block_on(async {
        let inner = RecordingApprovals::denying();
        let manual = SystemBoundaryApprovals::new(
            Some(inner.clone()),
            Some(PermissionMode::Manual),
            scope.clone(),
        );

        // Safe cross-service non-contract edit in `access == "all"` forwards without boundary escalation note.
        let safe_billing = PermissionAsk {
            id: "codex-1".into(),
            provider: "codex".into(),
            tool: "File change".into(),
            kind: "file_change".into(),
            summary: "update ../billing/src/internal.php".into(),
            detail: Some("Sync handler".into()),
            session_option: true,
            ..Default::default()
        };
        let decision = manual.ask(&safe_billing).unwrap().await.unwrap();
        assert_eq!(decision, Decision::Deny);
        let seen = inner.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].session_option);
        assert_eq!(seen[0].detail.as_deref(), Some("Sync handler"));

        // Foreign contract source even in `access == "all"` gets `[Boundary escalation: ...]` and session_option=false!
        let contract_cross = PermissionAsk {
            id: "codex-2".into(),
            provider: "codex".into(),
            tool: "File change".into(),
            kind: "file_change".into(),
            summary: "update ../billing/src/api.php".into(),
            session_option: true,
            ..Default::default()
        };
        let _ = manual.ask_answer(&contract_cross).unwrap().await.unwrap();
        let seen = inner.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 2);
        assert!(!seen[1].session_option);
        assert!(
            seen[1]
                .detail
                .as_deref()
                .unwrap()
                .starts_with("[Boundary escalation:")
        );

        // Missing inner in Manual mode returns Err.
        let no_inner = SystemBoundaryApprovals::new(None, Some(PermissionMode::Manual), scope);
        assert!(no_inner.ask_answer(&safe_billing).is_err());
    });
}

#[test]
fn check_step_boundary_writes_enforces_non_interactive_boundaries() {
    let scope_service = BoundaryScope {
        service: "orders".into(),
        mode: "edit".into(),
        access: "service".into(),
        own_root: PathBuf::from("/work/orders"),
        system_root: PathBuf::from("/work/sys"),
        selected_roots: BTreeMap::from([
            ("orders".into(), PathBuf::from("/work/orders")),
            ("billing".into(), PathBuf::from("/work/billing")),
        ]),
        writable_roots: vec![PathBuf::from("/work/orders")],
        contract_sources: BTreeMap::from([(
            PathBuf::from("/work/billing/src/api.php"),
            vec![("billing".into(), "billing.api".into())],
        )]),
    };

    // Valid write in own_root passes.
    let ok_git = vec![EvidenceFileChange {
        path: "src/order.php".into(),
        status: "modified".into(),
        from: None,
    }];
    assert!(check_step_boundary_writes(&scope_service, &ok_git, &["src/order.php".into()]).is_ok());

    // System root write fails cleanly with Manual / access="all" hint.
    let sys_err = check_step_boundary_writes(&scope_service, &[], &["__system__/AGENTS.md".into()])
        .unwrap_err();
    assert!(sys_err.message.contains("__system__"));
    assert!(sys_err.message.contains("Manual"));
    assert!(sys_err.message.contains("access = \"all\""));

    // Foreign service write detected via extra dirty worktree paths fails cleanly.
    let foreign_err = check_step_boundary_writes_with_extra(
        &scope_service,
        &ok_git,
        &["src/order.php".into()],
        &[PathBuf::from("/work/billing/src/api.php")],
    )
    .unwrap_err();
    assert!(foreign_err.message.contains("billing"));
    assert!(foreign_err.message.contains("Manual"));

    // Foreign contract source while `access == "service"` fails even if mapped into scope.
    let mut foreign_contract_in_own = scope_service.clone();
    foreign_contract_in_own.contract_sources.insert(
        PathBuf::from("/work/orders/shared/contract.php"),
        vec![("billing".into(), "billing.shared".into())],
    );
    let contract_err = check_step_boundary_writes(
        &foreign_contract_in_own,
        &[],
        &["shared/contract.php".into()],
    )
    .unwrap_err();
    assert!(contract_err.message.contains("billing.shared"));

    // Under `access == "all"`, selected service writes succeed while `__system__` still fails.
    let scope_all = BoundaryScope {
        access: "all".into(),
        writable_roots: vec![
            PathBuf::from("/work/orders"),
            PathBuf::from("/work/billing"),
        ],
        ..scope_service
    };
    let all_git = vec![
        EvidenceFileChange {
            path: "orders/src/order.php".into(),
            status: "modified".into(),
            from: None,
        },
        EvidenceFileChange {
            path: "billing/src/api.php".into(),
            status: "modified".into(),
            from: None,
        },
    ];
    assert!(
        check_step_boundary_writes(
            &scope_all,
            &all_git,
            &["orders/src/order.php".into(), "billing/src/api.php".into()],
        )
        .is_ok()
    );
    assert!(
        check_step_boundary_writes_with_extra(
            &scope_all,
            &all_git,
            &[],
            &[PathBuf::from("/work/sys/system.json")],
        )
        .is_err()
    );
}

#[test]
fn native_create_run_validate_state_and_prepare_launch_respect_permissions_and_read_only() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let root = TempRoot::new("native");
    let (config_path, _sys_dir, orders_dir, _billing_dir) = build_two_service_system(&root.0);
    let mut system = System::new(&config_path, std::slice::from_ref(&root.0)).unwrap();
    let plan = system
        .plan("Update order", "chg-nat", &["orders".into()], &[], 16000, 2)
        .unwrap();

    let fake_cli = root.0.join("fake-cli");
    write_text(&fake_cli, "#!/bin/sh\nexit 0\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake_cli, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let run_dir = root.0.join("run-manual");
    let state = rt
        .block_on(create_run_with_permissions(
            &mut system,
            &plan,
            &run_dir,
            "cursor",
            fake_cli.to_str(),
            "edit",
            120,
            "service",
            Some(true),
            Some("manual"),
        ))
        .unwrap();
    assert_eq!(state["permissions"], "manual");
    assert_eq!(state["steps"][0]["mode"], "read-only");
    assert_eq!(state["steps"][1]["mode"], "edit");
    assert_eq!(state["steps"][2]["mode"], "edit");
    assert_eq!(state["steps"][3]["mode"], "read-only");

    let validated = validate_state_sync(&mut system, &run_dir, true).unwrap();
    assert_eq!(validated["permissions"], "manual");

    // Invalid permissions in run.json are rejected by validate_state_sync.
    let mut bad_state = validated.clone();
    bad_state["permissions"] = json!("Auto");
    fs::write(
        run_dir.join("run.json"),
        serde_json::to_vec(&bad_state).unwrap(),
    )
    .unwrap();
    assert!(validate_state_sync(&mut system, &run_dir, false).is_err());

    // prompt_for skips Cursor's 120KB byte limit ONLY on `edit` steps with `manual` (`CursorAcp`).
    let big_plan = json!({
        "context": {
            "task": "Update order",
            "services": ["orders"],
            "sources": [{"service": "orders", "path": "src/order.php", "kind": "code", "excerpt": "x".repeat(CURSOR_PROMPT_BYTES + 500)}]
        },
        "relationships": [],
        "warnings": [],
        "omitted_sources": []
    });
    let step_edit = json!({"id": "service-orders", "service": "orders", "mode": "edit", "dispatch_id": "d".repeat(32)});
    let step_ro = json!({"id": "verify", "service": "__system__", "mode": "read-only", "dispatch_id": "d".repeat(32)});
    let big_prompt = prompt_for(&state, &big_plan, &step_edit, &[], None).unwrap();
    assert!(big_prompt.len() > CURSOR_PROMPT_BYTES);
    assert!(prompt_for(&state, &big_plan, &step_ro, &[], None).is_err());

    // prepare_launch_with_permissions keeps read-only steps read-only even when Bypass is selected.
    let schema = json!({"type": "object"});
    let schema_path = run_dir.join("result-schema.json");
    let claude_ro = prepare_launch_with_permissions(
        "claude",
        &fake_cli,
        &orders_dir,
        "Inspect",
        &schema,
        &schema_path,
        "read-only",
        &[],
        &[],
        Some(PermissionMode::Bypass),
        None,
    )
    .unwrap();
    assert!(
        claude_ro
            .argv
            .windows(2)
            .any(|w| w == ["--permission-mode", "plan"])
    );
    assert!(
        !claude_ro
            .argv
            .iter()
            .any(|a| a == "--allow-dangerously-skip-permissions")
    );

    let codex_ro = prepare_launch_with_permissions(
        "codex",
        &fake_cli,
        &orders_dir,
        "Inspect",
        &schema,
        &schema_path,
        "read-only",
        &[],
        &[],
        Some(PermissionMode::Manual),
        None,
    )
    .unwrap();
    assert!(
        codex_ro
            .argv
            .windows(2)
            .any(|w| w == ["--sandbox", "read-only"])
    );
    assert!(codex_ro.argv.iter().any(|a| a == "--skip-git-repo-check"));

    let codex_manual_edit = prepare_launch_with_permissions(
        "codex",
        &fake_cli,
        &orders_dir,
        "Edit",
        &schema,
        &schema_path,
        "edit",
        &[root.0.join("billing")],
        &[],
        Some(PermissionMode::Manual),
        None,
    )
    .unwrap();
    assert_eq!(
        codex_manual_edit.argv.last().map(String::as_str),
        Some("app-server")
    );
    assert!(
        !codex_manual_edit
            .argv
            .iter()
            .any(|a| a == "--skip-git-repo-check")
    );
    assert!(!codex_manual_edit.argv.iter().any(|a| a == "--add-dir"));
    assert!(
        !codex_manual_edit
            .argv
            .iter()
            .any(|a| a == "--output-schema")
    );
    assert!(!codex_manual_edit.argv.iter().any(|a| a == "-"));

    let cursor_manual_edit = prepare_launch_with_permissions(
        "cursor",
        &fake_cli,
        &orders_dir,
        &"y".repeat(CURSOR_PROMPT_BYTES + 200),
        &schema,
        &schema_path,
        "edit",
        &[],
        &[],
        Some(PermissionMode::Manual),
        None,
    )
    .unwrap();
    assert_eq!(
        cursor_manual_edit.argv.last().map(String::as_str),
        Some("acp")
    );

    // prepare_worker_for_transport skips `--add-dir` on CodexServer while keeping env and readable paths.
    let offer = ServiceGraphOffer {
        service: "orders".into(),
        command: "graphify".into(),
        graph_path: run_dir.join("orders/graph.json"),
        nodes: 5,
        edges: 3,
        commit: None,
        readable_paths: vec![run_dir.join("orders/graph.json")],
        claude_allow_rules: vec![],
        env: BTreeMap::new(),
        prompt_instructions: None,
    };
    let worker_server = prepare_worker_for_transport(
        "codex",
        "edit",
        infra_host::provider::Transport::CodexServer,
        PreparedCommand {
            argv: vec!["codex".into(), "app-server".into()],
            environment: BTreeMap::new(),
            input: vec![],
            readable: vec![],
            remove: vec![],
            prompt_prefix: String::new(),
        },
        &[offer],
    )
    .unwrap();
    assert!(!worker_server.argv.iter().any(|a| a == "--add-dir"));
    assert_eq!(
        worker_server.environment.get("GRAPHIFY_NO_AUTO_REFRESH"),
        Some(&"1".to_string())
    );
    assert!(
        worker_server
            .readable
            .contains(&run_dir.join("orders/graph.json"))
    );
}

#[test]
fn workers_request_threads_permissions_and_resume_updates_without_clobbering() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let root = TempRoot::new("workers");
    let (config_path, sys_dir, orders_dir, billing_dir) = build_two_service_system(&root.0);
    let mut system = System::new(&config_path, std::slice::from_ref(&root.0)).unwrap();
    let plan = system
        .plan("Update order", "chg-wrk", &["orders".into()], &[], 16000, 2)
        .unwrap();

    let fake_cli = root.0.join("fake-cli");
    write_text(&fake_cli, "#!/bin/sh\nexit 0\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake_cli, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let rid = "0123456789abcdef0123456789abcdef";
    let nonce1 = "fedcba9876543210fedcba9876543210";
    let run_dir = root.0.join("state/ai-system").join(rid);
    fs::create_dir_all(&run_dir).unwrap();
    write_json(&run_dir.join("approved-plan.json"), &plan);

    let req1 = json!({
        "directory": run_dir,
        "system_file": config_path,
        "allowed_roots": [sys_dir, orders_dir, billing_dir],
        "provider": "claude",
        "executable": fake_cli,
        "mode": "edit",
        "timeout": 120,
        "access": "service",
        "retry_step": null,
        "accept_source_changes": false,
        "accept_unrun_checks": false,
        "permissions": "auto"
    });
    let req1_path = run_dir.join(format!("request-{nonce1}.json"));
    write_json(&req1_path, &req1);

    struct NoopAdapter;
    impl infra_host::provider::LaunchAdapter for NoopAdapter {
        fn adapt(
            &self,
            _input: &infra_host::provider::Launch,
            command: PreparedCommand,
        ) -> infra_host::Result<PreparedCommand> {
            Ok(command)
        }
    }
    struct NoopObserver;
    impl infra_host::process::RunObserver for NoopObserver {
        fn boundary(&self, _signal: &str, _details: &Value) -> infra_host::Result<()> {
            Ok(())
        }
    }
    struct NoopBridge;
    impl CodeGraphBridge for NoopBridge {}

    // Pre-cancelled run stops cleanly before invoking guardian while still creating run.json with `permissions: "auto"`.
    let cancel = Arc::new(AtomicBool::new(true));
    let _ = rt.block_on(run_system_request_with_bridge_and_approvals(
        &req1_path,
        cancel.clone(),
        None,
        Arc::new(NoopAdapter),
        Arc::new(NoopObserver),
        Arc::new(NoopBridge),
        Some("sess-1".into()),
        Some(RecordingApprovals::allowing()),
        |_| Ok(()),
        |_| Ok(()),
    ));
    let saved1: Value =
        serde_json::from_slice(&fs::read(run_dir.join("execution/run.json")).unwrap()).unwrap();
    assert_eq!(saved1["permissions"], "auto");

    // Resuming with updated `permissions: "manual"` updates `run.json["permissions"]` to `"manual"`.
    let nonce2 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let mut req2 = req1.clone();
    req2["permissions"] = json!("manual");
    let req2_path = run_dir.join(format!("request-{nonce2}.json"));
    write_json(&req2_path, &req2);

    let _ = rt.block_on(run_system_request_with_adapter_and_approvals(
        &req2_path,
        cancel.clone(),
        None,
        Arc::new(NoopAdapter),
        Arc::new(NoopObserver),
        Some(RecordingApprovals::allowing()),
        |_| Ok(()),
        |_| Ok(()),
    ));
    let saved2: Value =
        serde_json::from_slice(&fs::read(run_dir.join("execution/run.json")).unwrap()).unwrap();
    assert_eq!(saved2["permissions"], "manual");

    // RunnerGraphHooks keeps Default + Clone with `approvals`.
    let hooks = RunnerGraphHooks {
        approvals: Some(RecordingApprovals::allowing()),
        ..Default::default()
    };
    let cloned = hooks.clone();
    assert!(cloned.approvals.is_some());
}
