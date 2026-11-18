use super::*;
use crate::{
    graph::{Graphs, publish_for_tests},
    http::ROUTES,
    provider::ProviderDomain,
    services::{Domain, Payload},
    sessions::{SessionLimits, Sessions, SpecialWorkflow},
    store::SessionStore,
};
use infra_host::provider::{Launch, PreparedCommand};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("git command");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write_passport(root: &Path, id: &str, provides: Value, consumes: Value, sources: Value) {
    fs::create_dir_all(root).unwrap();
    let passport = json!({
        "schema_version": 1,
        "id": id,
        "description": format!("{id} service"),
        "owner": "platform",
        "relationships_complete": true,
        "capabilities": [{
            "id": format!("{id}-core"),
            "description": format!("Core capability of {id}"),
            "status": "implemented",
            "sources": sources.as_array().map(|arr| {
                arr.iter().filter_map(|s| s["path"].as_str().map(Value::from)).collect::<Vec<_>>()
            }).unwrap_or_default(),
        }],
        "provides": provides,
        "consumes": consumes,
        "sources": sources,
    });
    fs::write(
        root.join("ai-service.json"),
        serde_json::to_vec_pretty(&passport).unwrap(),
    )
    .unwrap();
}

fn write_sample_graphify_json(
    store: &SessionStore,
    graph_id: &str,
    program: &Path,
    nodes: Value,
    links: Value,
) {
    publish_for_tests(store, graph_id, program);
    let graph_file = store
        .state
        .join("graphs")
        .join(graph_id)
        .join("live")
        .join("graph.json");
    fs::create_dir_all(graph_file.parent().unwrap()).unwrap();
    let doc = json!({
        "directed": true,
        "multigraph": false,
        "graph": {},
        "nodes": nodes,
        "links": links,
    });
    fs::write(&graph_file, serde_json::to_vec(&doc).unwrap()).unwrap();
}

struct HarborFixture {
    _tmp: TempDir,
    store: SessionStore,
    graphs: Arc<Graphs>,
    bridge: Arc<HarnessCodeGraphBridge>,
    domain: SystemGraphDomain,
    monorepo_root: PathBuf,
    orders_root: PathBuf,
    billing_root: PathBuf,
    standalone_web_root: PathBuf,
    monorepo_pid: String,
    web_pid: String,
    fake_graphify: PathBuf,
}

fn setup_harbor_fixture() -> HarborFixture {
    let tmp = tempfile::tempdir().unwrap();
    let state_dir = tmp.path().join("state");
    let store = SessionStore::open(&state_dir).unwrap();

    let monorepo_root = tmp.path().join("harbor-core");
    let orders_root = monorepo_root.join("services/orders");
    let billing_root = monorepo_root.join("services/billing");
    let standalone_web_root = tmp.path().join("harbor-web");
    let outside_root = tmp.path().join("outside-unregistered");

    fs::create_dir_all(orders_root.join("src")).unwrap();
    fs::create_dir_all(billing_root.join("src")).unwrap();
    fs::create_dir_all(standalone_web_root.join("src")).unwrap();
    fs::create_dir_all(outside_root.join("src")).unwrap();

    // Source files in orders, billing, and web (including an undeclared call from billing -> orders!).
    fs::write(
        orders_root.join("src/OrderService.php"),
        "<?php\nclass OrderService {\n    public function createOrder(): array { return []; }\n}\n",
    )
    .unwrap();
    fs::write(
        orders_root.join("src/OrderController.php"),
        "<?php\nclass OrderController {\n    public function store(OrderService $svc): array { return $svc->createOrder(); }\n}\n",
    )
    .unwrap();
    fs::write(
        billing_root.join("src/InvoiceWorker.php"),
        "<?php\n// Undeclared direct call to orders-http-v1 endpoint!\nclass InvoiceWorker {\n    public function sync(): string { return 'orders-http-v1 /api/v1/orders'; }\n}\n",
    )
    .unwrap();
    fs::write(
        standalone_web_root.join("src/OrdersClient.ts"),
        "export async function fetchOrders() {\n  return fetch('/api/v1/orders'); // orders-http-v1\n}\n",
    )
    .unwrap();
    fs::write(outside_root.join("src/Secret.php"), "<?php\n").unwrap();

    write_passport(
        &orders_root,
        "orders",
        json!([{
            "id": "orders-http-v1",
            "kind": "http",
            "version": "1.0.0",
            "sources": ["src/OrderController.php"]
        }]),
        json!([]),
        json!([
            {"path": "src/OrderService.php", "kind": "code"},
            {"path": "src/OrderController.php", "kind": "contract"}
        ]),
    );

    write_passport(
        &billing_root,
        "billing",
        json!([{
            "id": "billing-events-v1",
            "kind": "event",
            "version": "1.0.0",
            "sources": ["src/InvoiceWorker.php"]
        }]),
        json!([]),
        json!([{"path": "src/InvoiceWorker.php", "kind": "code"}]),
    );

    write_passport(
        &standalone_web_root,
        "web",
        json!([]),
        json!([{
            "service": "orders",
            "contract": "orders-http-v1",
            "version": "1.0.0"
        }]),
        json!([{"path": "src/OrdersClient.ts", "kind": "code"}]),
    );

    write_passport(&outside_root, "secret", json!([]), json!([]), json!([]));

    let sys_cfg = json!({
        "schema_version": 1,
        "name": "harbor-market",
        "shared_sources": [],
        "services": [
            {"id": "orders", "root": "services/orders", "manifest": "ai-service.json"},
            {"id": "billing", "root": "services/billing", "manifest": "ai-service.json"},
            {"id": "web", "root": "../harbor-web", "manifest": "ai-service.json"},
            {"id": "secret", "root": "../outside-unregistered", "manifest": "ai-service.json"}
        ]
    });
    fs::write(
        monorepo_root.join("system.json"),
        serde_json::to_vec_pretty(&sys_cfg).unwrap(),
    )
    .unwrap();

    // Initialize git repos: one monorepo (`harbor-core`) with distinct commits per subfolder, plus standalone `harbor-web`.
    git(&monorepo_root, &["init"]);
    git(&monorepo_root, &["add", "system.json", "services/orders"]);
    git(&monorepo_root, &["commit", "-m", "orders: initial service"]);
    git(&monorepo_root, &["add", "services/billing"]);
    git(&monorepo_root, &["commit", "-m", "billing: initial worker"]);

    git(&standalone_web_root, &["init"]);
    git(&standalone_web_root, &["add", "."]);
    git(
        &standalone_web_root,
        &["commit", "-m", "web: initial client"],
    );

    // Register ONLY `harbor-core` and `harbor-web` (leaving `outside-unregistered` disallowed!).
    let monorepo_pid = store.register(&monorepo_root).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let web_pid = store.register(&standalone_web_root).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let fake_graphify = tmp.path().join("fake-graphify");
    fs::write(&fake_graphify, "#!/bin/sh\nexit 0\n").unwrap();

    // The fixture's own Graphify stands in for an installed one, so the
    // states it reports do not depend on what the machine has on its PATH;
    // the gap is the automatic one `Graphs::new` uses.
    let graphs = Graphs::with(
        store.clone(),
        Some(fake_graphify.clone()),
        std::time::Duration::from_secs(30),
    );
    let bridge = HarnessCodeGraphBridge::new(store.clone(), graphs.clone());
    let domain = SystemGraphDomain::new(store.clone(), graphs.clone(), bridge.clone(), None);

    HarborFixture {
        _tmp: tmp,
        store,
        graphs,
        bridge,
        domain,
        monorepo_root,
        orders_root,
        billing_root,
        standalone_web_root,
        monorepo_pid,
        web_pid,
        fake_graphify,
    }
}

fn unwrap_json(payload: Payload) -> Value {
    match payload {
        Payload::Json(v) => v,
        Payload::Bytes { .. } => panic!("expected json payload"),
    }
}

#[test]
fn all_system_graph_routes_and_28_service_map_params_are_registered() {
    let svc_map = ROUTES
        .iter()
        .find(|r| r.operation == "system_graph.service_map")
        .expect("service_map route");
    assert_eq!(svc_map.pattern, "/api/systems/service-graph/map");
    for param in [
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
    ] {
        assert!(
            svc_map.query.contains(&param),
            "missing query param {param}"
        );
    }
    // What no handler reads is refused, so that a request naming it does
    // not look answered.
    for (operation, dead) in [
        ("system_graph.service_map", &["phase", "attempt"][..]),
        ("system_graph.service_find", &["agent", "focus_session"]),
        (
            "system_graph.service_changes",
            &["phase", "attempt", "launch", "compare_launch"],
        ),
        (
            "system_graph.service_file_impact",
            &["phase", "attempt", "launch", "compare_launch"],
        ),
        (
            "system_graph.map",
            &[
                "phase",
                "compare_run_id",
                "compare_launch",
                "notests",
                "pattern",
                "longer",
                "failing",
            ],
        ),
        ("system_graph.find", &["target", "run_id", "agent"]),
        (
            "system_graph.changes",
            &["phase", "launch", "focus_session"],
        ),
        (
            "system_graph.impact",
            &["contracts", "seed_services", "depth"],
        ),
    ] {
        let route = ROUTES
            .iter()
            .find(|route| route.operation == operation)
            .expect(operation);
        for key in dead {
            assert!(!route.query.contains(key), "{operation} accepts {key}");
        }
    }

    for op in [
        "system_graph.service_status",
        "system_graph.service_act",
        "system_graph.service_find",
        "system_graph.service_changes",
        "system_graph.service_file_impact",
        "system_graph.service_pulls",
        "system_graph.service_commits",
        "system_graph.service_branches",
        "system_graph.status",
        "system_graph.act",
        "system_graph.map",
        "system_graph.find",
        "system_graph.changes",
        "system_graph.impact",
    ] {
        assert!(
            ROUTES.iter().any(|r| r.operation == op),
            "missing route for {op}"
        );
    }
}

#[test]
fn resolver_reuses_registered_project_id_and_scopes_monorepo_subfolders_and_denies_outside_roots() {
    let fx = setup_harbor_fixture();

    // 1. `__system__` root equals registered `monorepo_pid` -> reuses exact `monorepo_pid`!
    let (_, sys_target) = resolve_service_target(
        &fx.store,
        Some(&fx.graphs),
        None,
        &fx.monorepo_pid,
        None,
        "__system__",
    )
    .unwrap();
    assert!(sys_target.reused_project);
    assert_eq!(sys_target.graph_id, fx.monorepo_pid);

    // 2. `web` root equals registered `web_pid` -> reuses exact `web_pid` so Sessions & Systems share cache!
    let (_, web_target) = resolve_service_target(
        &fx.store,
        Some(&fx.graphs),
        None,
        &fx.monorepo_pid,
        Some("system.json"),
        "web",
    )
    .unwrap();
    assert!(web_target.reused_project);
    assert_eq!(web_target.graph_id, fx.web_pid);

    // 3. Monorepo subfolders `orders` and `billing` get deterministic `sys-<hash>` IDs!
    let (_, orders_target) = resolve_service_target(
        &fx.store,
        Some(&fx.graphs),
        None,
        &fx.monorepo_pid,
        None,
        "orders",
    )
    .unwrap();
    assert!(!orders_target.reused_project);
    assert!(orders_target.graph_id.starts_with("sys-"));
    assert_eq!(orders_target.graph_id.len(), 20);
    assert_eq!(
        orders_target.root,
        infra_core::fs::canonicalize(&fx.orders_root).unwrap()
    );

    // 4. Disallowed `secret` service outside registered roots returns 403!
    let secret_err = resolve_service_target(
        &fx.store,
        Some(&fx.graphs),
        None,
        &fx.monorepo_pid,
        None,
        "secret",
    )
    .unwrap_err();
    assert_eq!(secret_err.status, 403);

    // 5. Path traversal in `config_path` returns 403, unknown service returns 404!
    let trav_err = resolve_service_target(
        &fx.store,
        Some(&fx.graphs),
        None,
        &fx.monorepo_pid,
        Some("../outside-unregistered/system.json"),
        "orders",
    )
    .unwrap_err();
    assert_eq!(trav_err.status, 403);

    let missing_err = resolve_service_target(
        &fx.store,
        Some(&fx.graphs),
        None,
        &fx.monorepo_pid,
        None,
        "nonexistent",
    )
    .unwrap_err();
    assert_eq!(missing_err.status, 404);

    // 6. Monorepo subfolder `commits` for `orders` only lists the commit touching `services/orders`, NOT `services/billing`!
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let commits_val = rt.block_on(async {
        unwrap_json(
            fx.domain
                .execute(
                    "system_graph.service_commits",
                    json!({
                        "query": {
                            "project_id": fx.monorepo_pid,
                            "service": "orders"
                        },
                        "body": {}
                    }),
                )
                .await
                .unwrap(),
        )
    });
    let commits = commits_val["commits"].as_array().expect("commits array");
    assert_eq!(commits.len(), 1);
    assert!(
        commits[0]["subject"]
            .as_str()
            .unwrap_or("")
            .contains("orders: initial service")
    );
}

#[test]
fn system_and_member_graph_endpoints_overlay_sysrun_evidence_shell_reads_and_cross_service_impact()
{
    let fx = setup_harbor_fixture();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    // Resolve targets and publish real Graphify graphs for `orders` and `web`.
    let (_, orders_t) = resolve_service_target(
        &fx.store,
        Some(&fx.graphs),
        None,
        &fx.monorepo_pid,
        None,
        "orders",
    )
    .unwrap();
    write_sample_graphify_json(
        &fx.store,
        &orders_t.graph_id,
        &fx.fake_graphify,
        json!([
            {"id": "n_svc", "label": "OrderService", "file_type": "code", "source_file": "src/OrderService.php", "source_location": "L2"},
            {"id": "n_ctrl", "label": "OrderController", "file_type": "code", "source_file": "src/OrderController.php", "source_location": "L2"}
        ]),
        json!([
            {"source": "n_ctrl", "target": "n_svc", "relation": "calls", "confidence": "EXTRACTED"}
        ]),
    );
    write_sample_graphify_json(
        &fx.store,
        &fx.web_pid,
        &fx.fake_graphify,
        json!([
            {"id": "n_web", "label": "fetchOrders", "file_type": "code", "source_file": "src/OrdersClient.ts", "source_location": "L1"}
        ]),
        json!([]),
    );

    // Seed real `sessions` rows for `sess-launch-1` and `sess-launch-2` plus a real SQLite `Shell` event in `sess-launch-2`!
    let run_id = "0123456789abcdef0123456789abcdef";
    let gen_l1 = "11111111111111111111111111111111";
    let gen_l2 = "22222222222222222222222222222222";
    let launch2_shell_eid = {
        let project_rec = fx.store.project(&fx.monorepo_pid).unwrap();
        for (lsid, lgen) in [("sess-launch-1", gen_l1), ("sess-launch-2", gen_l2)] {
            store::insert_session(
                &fx.store.connection().unwrap(),
                &json!({
                    "provider": "claude",
                    "mode": "edit",
                    "workflow": "system_run",
                    "agents_enabled": false,
                    "agent_count": 1,
                    "budgets": {"seconds": 300}
                }),
                &project_rec,
                "Update OrderService payload",
                &[],
                lsid,
                &store::now(),
                lgen,
            )
            .unwrap();
            fx.store
                .patch(lsid, &json!({"status": "completed"}))
                .unwrap();
        }
        let shell_ev = json!({
            "kind": "agent_activity",
            "type": "tool",
            "tool": "Shell",
            "state": "completed",
            "ok": true,
            "agent": "service-orders",
            "attempt": 2,
            "detail": "sed -n '1,10p' src/OrderController.php"
        });
        let eid = fx
            .store
            .guarded_event_id("sess-launch-2", gen_l2, &shell_ev)
            .unwrap()
            .expect("launch-2 shell event id");

        let conn = fx.store.connection().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS system_runs(id TEXT PRIMARY KEY,data TEXT NOT NULL,project_id TEXT);",
        )
        .unwrap();
        let record = json!({
            "id": run_id,
            "project_id": fx.monorepo_pid,
            "config_path": "system.json",
            "goal": "Update OrderService payload",
            "provider": "claude",
            "status": "completed",
            "session_id": "sess-launch-2",
            "launches": ["sess-launch-1", "sess-launch-2"],
        });
        conn.execute(
            "INSERT INTO system_runs(id, data, project_id) VALUES (?1, ?2, ?3)",
            rusqlite::params![run_id, record.to_string(), fx.monorepo_pid],
        )
        .unwrap();
        eid
    };
    let exec_dir = fx
        .store
        .state
        .join("ai-system")
        .join(run_id)
        .join("execution");
    fs::create_dir_all(&exec_dir).unwrap();
    let run_state = json!({
        "status": "completed",
        "selected": ["orders", "web"],
        "steps": [
            {"id": "contracts", "service": "__system__", "role": "read", "status": "completed", "attempt": 1},
            {"id": "service-orders", "service": "orders", "role": "edit", "status": "completed", "attempt": 2},
            {"id": "verify", "service": "__system__", "role": "read", "status": "completed", "attempt": 1}
        ]
    });
    fs::write(
        exec_dir.join("run.json"),
        serde_json::to_vec_pretty(&run_state).unwrap(),
    )
    .unwrap();

    // Attempt 1 (`sess-launch-1`) edited `src/OrderService.php` WITHOUT opening `src/OrderController.php` or `web/src/OrdersClient.ts`.
    let ev1 = DispatchEvidence {
        phase: "service-orders".to_owned(),
        attempt: 1,
        launch_id: Some("sess-launch-1".to_owned()),
        service: "orders".to_owned(),
        root: fx.orders_root.to_string_lossy().into_owned(),
        git_changed: vec![EvidenceFileChange {
            path: "src/OrderService.php".to_owned(),
            status: "M".to_owned(),
            from: None,
        }],
        ledger_edited: vec![EvidenceFileChange {
            path: "src/OrderService.php".to_owned(),
            status: "M".to_owned(),
            from: None,
        }],
        opened: vec!["src/OrderService.php".to_owned()],
        steps: 3,
        ..Default::default()
    };
    fs::write(
        exec_dir.join("service-orders-a1.evidence.json"),
        serde_json::to_vec_pretty(&ev1).unwrap(),
    )
    .unwrap();

    // Attempt 2 (`sess-launch-2`) opened `src/OrderController.php` via `sed -n` (`shell_reads` SQLite event id)!
    let ev2 = DispatchEvidence {
        phase: "service-orders".to_owned(),
        attempt: 2,
        launch_id: Some("sess-launch-2".to_owned()),
        service: "orders".to_owned(),
        root: fx.orders_root.to_string_lossy().into_owned(),
        git_changed: vec![EvidenceFileChange {
            path: "src/OrderService.php".to_owned(),
            status: "M".to_owned(),
            from: None,
        }],
        opened: vec!["src/OrderService.php".to_owned()],
        shell_reads: BTreeMap::from([(
            "src/OrderController.php".to_owned(),
            launch2_shell_eid as i64,
        )]),
        steps: 5,
        graph_asked: true,
        ..Default::default()
    };
    fs::write(
        exec_dir.join("service-orders-a2.evidence.json"),
        serde_json::to_vec_pretty(&ev2).unwrap(),
    )
    .unwrap();

    rt.block_on(async {
        // 1. `GET /api/systems/graph/status` reports UI-compatible states and member counts.
        let sys_status = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.status",
                    json!({"query": {"project_id": fx.monorepo_pid}, "body": {}}),
                )
                .await
                .unwrap(),
        );
        assert!(
            ["ready", "stale"].contains(&sys_status["state"].as_str().unwrap()),
            "unexpected state: {sys_status}"
        );
        assert_eq!(sys_status["ready_count"], 2);
        assert_eq!(sys_status["total_count"], 4); // orders, billing, web + unavailable secret
        assert_eq!(sys_status["tell_agents"], true);

        // 2. `GET /api/systems/service-graph/map` with `target=sysrun:<run_id>` & `launch`/`compare_launch`!
        let svc_map = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.service_map",
                    json!({
                        "query": {
                            "project_id": fx.monorepo_pid,
                            "service": "orders",
                            "target": format!("sysrun:{run_id}"),
                            "launch": "sess-launch-1",
                            "compare_launch": "sess-launch-2",
                            "agent": "service-orders"
                        },
                        "body": {}
                    }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(svc_map["service"], "orders");
        assert_eq!(svc_map["graph_id"], orders_t.graph_id);
        assert_eq!(svc_map["run"]["run_id"], run_id);
        assert_eq!(svc_map["run"]["session"], "sess-launch-1");
        assert_eq!(svc_map["run"]["launch"], "sess-launch-1");
        assert_eq!(svc_map["run"]["compare_launch"], "sess-launch-2");
        assert_eq!(svc_map["run"]["launches"].as_array().unwrap().len(), 2);
        // `web` consumes `orders-http-v1` and was never opened in `sess-launch-1`, so `cross_service_unopened` flags `web`!
        let cross_unopened = svc_map["cross_service_unopened"]
            .as_array()
            .expect("cross_service_unopened");
        assert!(
            cross_unopened.iter().any(|c| c["service"] == "web"),
            "expected web in cross_service_unopened: {cross_unopened:?}"
        );

        // Numeric launch selector (`launch=1`) + latest launch (`launch` omitted) with UI-verified `shell_reads` SQLite event!
        let svc_map_num = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.service_map",
                    json!({
                        "query": {
                            "project_id": fx.monorepo_pid,
                            "service": "orders",
                            "target": format!("sysrun:{run_id}"),
                            "launch": "1"
                        },
                        "body": {}
                    }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(svc_map_num["run"]["launch"], "sess-launch-1");
        assert_eq!(svc_map_num["run"]["session"], "sess-launch-1");

        let svc_map_latest = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.service_map",
                    json!({
                        "query": {
                            "project_id": fx.monorepo_pid,
                            "service": "orders",
                            "target": format!("sysrun:{run_id}")
                        },
                        "body": {}
                    }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(svc_map_latest["run"]["session"], "sess-launch-2");
        assert!(svc_map_latest["run"]["launch"].is_null());
        let shell_rows = svc_map_latest["run"]["shell_reads"]["rows"]
            .as_array()
            .expect("shell_reads rows");
        assert!(
            shell_rows
                .iter()
                .any(|r| r["path"] == "src/OrderController.php" && r["event"] == launch2_shell_eid),
            "expected verified SQLite shell_reads row: {shell_rows:?}"
        );
        let page_events = fx
            .store
            .events("sess-launch-2", (launch2_shell_eid - 1) as i64)
            .unwrap();
        assert_eq!(page_events[0]["id"], launch2_shell_eid);
        assert_eq!(page_events[0]["tool"], "Shell");
        assert_eq!(page_events[0]["state"], "completed");

        // 3. `GET /api/systems/service-graph/file-impact` enriches intra-service impact with `contract_consumers`!
        let impact = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.service_file_impact",
                    json!({
                        "query": {
                            "project_id": fx.monorepo_pid,
                            "service": "orders",
                            "path": "src/OrderController.php",
                            "run_id": run_id
                        },
                        "body": {}
                    }),
                )
                .await
                .unwrap(),
        );
        assert!(
            impact["contract_consumers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["consumer"] == "web"),
            "expected web in contract_consumers: {impact}"
        );

        // 4. `GET /api/systems/graph/map` detects both `verified` (`web -> orders`) AND `undeclared_in_code` (`billing -> orders`)!
        let sys_map = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.map",
                    json!({
                        "query": {
                            "project_id": fx.monorepo_pid,
                            "run_id": run_id,
                            "seed_services": "orders",
                            "depth": "1"
                        },
                        "body": {}
                    }),
                )
                .await
                .unwrap(),
        );
        let edges = sys_map["edges"].as_array().expect("system edges");
        assert!(
            edges.iter().any(|e| {
                e["consumer"] == "web" && e["provider"] == "orders" && e["provenance"] == "verified"
            }),
            "expected verified edge web->orders: {edges:?}"
        );
        assert!(
            edges.iter().any(|e| {
                e["consumer"] == "billing"
                    && e["provider"] == "orders"
                    && e["provenance"] == "undeclared_in_code"
            }),
            "expected undeclared_in_code edge billing->orders: {edges:?}"
        );
        assert!(sys_map["blast_radius"].is_object());
        assert!(sys_map["run"].is_object());

        // 5. `GET /api/systems/graph/find` merges `SystemGraph` + Graphify `Index::find` symbol hits!
        let found = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.find",
                    json!({
                        "query": {
                            "project_id": fx.monorepo_pid,
                            "q": "OrderController"
                        },
                        "body": {}
                    }),
                )
                .await
                .unwrap(),
        );
        let results = found["results"].as_array().unwrap();
        assert!(
            results.iter().any(|r| r["service"] == "orders"
                && r["name"].as_str().unwrap_or("").contains("OrderController")),
            "expected Graphify symbol hit for OrderController: {results:?}"
        );

        // 6. `POST /api/systems/graph` bulk `withhold` -> `offer` -> `disable` -> `enable`.
        let withheld = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.act",
                    json!({
                        "query": {},
                        "body": {
                            "project_id": fx.monorepo_pid,
                            "action": "withhold"
                        }
                    }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(withheld["tell_agents"], false);
    });
}

#[test]
fn code_graph_bridge_captures_live_focus_shell_read_event_ids_and_enriches_run_view() {
    let fx = setup_harbor_fixture();
    let (_, orders_t) = resolve_service_target(
        &fx.store,
        Some(&fx.graphs),
        None,
        &fx.monorepo_pid,
        None,
        "orders",
    )
    .unwrap();
    write_sample_graphify_json(
        &fx.store,
        &orders_t.graph_id,
        &fx.fake_graphify,
        json!([
            {"id": "n_svc", "label": "OrderService", "file_type": "code", "source_file": "src/OrderService.php", "source_location": "L2"},
            {"id": "n_ctrl", "label": "OrderController", "file_type": "code", "source_file": "src/OrderController.php", "source_location": "L2"}
        ]),
        json!([
            {"source": "n_ctrl", "target": "n_svc", "relation": "calls", "confidence": "EXTRACTED"}
        ]),
    );

    // Create a real session row in `sessions` so `guarded_event` records live `graph_focus` events!
    let sid = "sess-bridge-live";
    let generation = "0123456789abcdef0123456789abcdef";
    let run_id = "fedcba9876543210fedcba9876543210";
    {
        let project_rec = fx.store.project(&fx.monorepo_pid).unwrap();
        store::insert_session(
            &fx.store.connection().unwrap(),
            &json!({
                "provider": "claude",
                "mode": "edit",
                "workflow": "native",
                "agents_enabled": false,
                "agent_count": 1,
                "budgets": {"seconds": 300}
            }),
            &project_rec,
            "sys",
            &[],
            sid,
            &store::now(),
            generation,
        )
        .unwrap();
        fx.store.patch(sid, &json!({"status": "running"})).unwrap();
    }
    fx.bridge.register_session(run_id, sid, generation);

    let ctx = DispatchGraphContext {
        run_id: run_id.to_owned(),
        journal_run_id: run_id.to_owned(),
        launch_id: Some(sid.to_owned()),
        execution_dir: fx
            .store
            .state
            .join("ai-system")
            .join(run_id)
            .join("execution"),
        system_file: fx.monorepo_root.join("system.json"),
        provider: "claude".to_owned(),
        mode: "edit".to_owned(),
        access: "service".to_owned(),
        tell_agents_graph: Some(true),
        phase: "service-orders".to_owned(),
        dispatch_id: "service-orders-a1".to_owned(),
        attempt: 1,
        service: "orders".to_owned(),
        root: fx.orders_root.clone(),
        roots: BTreeMap::from([
            ("__system__".to_owned(), fx.monorepo_root.clone()),
            ("orders".to_owned(), fx.orders_root.clone()),
            ("web".to_owned(), fx.standalone_web_root.clone()),
        ]),
        selected: vec!["orders".to_owned()],
        readable: vec!["orders".to_owned()],
        writable: vec!["orders".to_owned()],
        base_head: None,
        head_after: None,
        plan: json!({"goal": "Update OrderService in src/OrderService.php"}),
        state: json!({"goal": "Update OrderService in src/OrderService.php", "selected": ["orders"]}),
    };

    // 1. `offers_for_dispatch` returns Graphify offer when `tell_agents_graph != Some(false)`, and empty when `Some(false)`.
    let offers = fx.bridge.offers_for_dispatch(&ctx).unwrap();
    assert_eq!(offers.len(), 1);
    assert_eq!(offers[0].service, "orders");
    assert!(
        offers[0]
            .prompt_instructions
            .as_deref()
            .unwrap()
            .contains("orders")
    );

    let mut muted_ctx = ctx.clone();
    muted_ctx.tell_agents_graph = Some(false);
    assert!(
        fx.bridge
            .offers_for_dispatch(&muted_ctx)
            .unwrap()
            .is_empty()
    );

    // 2. Start dispatch -> feed Claude tool stream (Bash graphify query + Bash `sed -n` + Edit) -> finish!
    fx.bridge.on_dispatch_start(&ctx).unwrap();

    let bash_graphify_start = json!({
        "type": "assistant",
        "message": {
            "content": [{
                "type": "tool_use",
                "id": "call_g",
                "name": "Bash",
                "input": {"command": "graphify query \"OrderService\" --graph /tmp/g.json"}
            }]
        }
    });
    let bash_graphify_end = json!({
        "type": "user",
        "message": {
            "content": [{"type": "tool_result", "tool_use_id": "call_g", "content": "ok"}]
        }
    });
    let sed_cmd = "sed -n '1,20p' src/OrderService.php";
    let bash_sed_start = json!({
        "type": "assistant",
        "message": {
            "content": [{
                "type": "tool_use",
                "id": "call_sed",
                "name": "Bash",
                "input": {"command": sed_cmd}
            }]
        }
    });
    let bash_sed_end = json!({
        "type": "user",
        "message": {
            "content": [{"type": "tool_result", "tool_use_id": "call_sed", "content": "<?php"}]
        }
    });
    let edit_path = fx.orders_root.join("src/OrderService.php");
    let edit_start = json!({
        "type": "assistant",
        "message": {
            "content": [{
                "type": "tool_use",
                "id": "call_edit",
                "name": "Edit",
                "input": {"file_path": edit_path.to_string_lossy()}
            }]
        }
    });
    let edit_end = json!({
        "type": "user",
        "message": {
            "content": [{"type": "tool_result", "tool_use_id": "call_edit", "content": "updated"}]
        }
    });

    let store_emit = fx.store.clone();
    let bridge_obs = fx.bridge.clone();
    let sid_owned = sid.to_owned();
    let gen_owned = generation.to_owned();
    let rid_owned = run_id.to_owned();
    let mut activity = crate::agent_activity::ActivityStream::new_with_ids(
        move |mut ev| {
            ev["phase_id"] = json!("service-orders");
            ev["attempt"] = json!(1);
            store_emit.guarded_event_id(&sid_owned, &gen_owned, &ev)
        },
        move |eid, ev| {
            bridge_obs.on_emitted_session_event(&rid_owned, eid, ev);
        },
        "claude",
        BTreeMap::from([(
            "orders".to_owned(),
            fx.orders_root.to_string_lossy().into_owned(),
        )]),
        BTreeMap::new(),
    );
    activity.start(
        "service-orders",
        1,
        &fx.orders_root.to_string_lossy(),
        &json!({"phase_id": "service-orders", "attempt": 1}),
    );

    for ev in [
        &bash_graphify_start,
        &bash_graphify_end,
        &bash_sed_start,
        &bash_sed_end,
        &edit_start,
    ] {
        let raw = serde_json::to_vec(ev).unwrap();
        activity.line(&raw);
        fx.bridge.on_dispatch_signal(&ctx, &raw).unwrap();
    }
    // Simulate file modification on disk before `edit_end` completes so `LiveFocusTracker` extracts declarations!
    fs::write(
        &edit_path,
        "<?php\nclass OrderService {\n    public function createOrder(): array { return ['v' => 2]; }\n    public function cancelOrder(): void {}\n}\n",
    )
    .unwrap();
    let raw_edit_end = serde_json::to_vec(&edit_end).unwrap();
    activity.line(&raw_edit_end);
    fx.bridge.on_dispatch_signal(&ctx, &raw_edit_end).unwrap();
    activity.finish(
        "completed",
        &json!({"phase_id": "service-orders", "attempt": 1}),
    );

    let evidence = fx
        .bridge
        .on_dispatch_finish(&ctx, &json!({"status": "completed"}))
        .unwrap()
        .expect("dispatch evidence");
    assert!(evidence.graph_asked);
    let sed_sql_eid = evidence
        .shell_reads
        .get("src/OrderService.php")
        .copied()
        .expect("recorded shell_read event id");
    assert!(sed_sql_eid > 0);
    let paged = fx.store.events(sid, sed_sql_eid - 1).unwrap();
    assert_eq!(paged[0]["id"], sed_sql_eid as u64);
    assert_eq!(paged[0]["tool"], "Shell");
    assert_eq!(paged[0]["state"], "completed");
    assert!(
        paged[0]["detail"]
            .as_str()
            .unwrap_or("")
            .contains("sed -n '1,20p' src/OrderService.php"),
        "expected restored Claude detail on completed Shell event: {:?}",
        paged[0]
    );
    assert!(
        evidence
            .ledger_edited
            .iter()
            .any(|e| e.path == "src/OrderService.php")
    );
    assert!(
        evidence
            .created_entities
            .iter()
            .any(|e| e["name"] == "cancelOrder"),
        "expected new method cancelOrder in created_entities: {:?}",
        evidence.created_entities
    );

    // Verify `graph_focus` session events were emitted!
    let events = fx.store.events(sid, 0).unwrap();
    assert!(
        events
            .iter()
            .any(|e| e["kind"] == "graph_focus" && e["service"] == "orders"),
        "expected live graph_focus event on session: {events:?}"
    );

    // Verify `enrich_run_view` populates `graph_focus`, `unopened_callers` (`src/OrderController.php`), and `cross_service_unopened` (`web`)!
    // Persist `system_runs` row + evidence so `Graphs::map(Target::SysRun)` inside `enrich_run_view` resolves cleanly!
    {
        let conn = fx.store.connection().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS system_runs(id TEXT PRIMARY KEY,data TEXT NOT NULL,project_id TEXT);",
        )
        .unwrap();
        let rec = json!({
            "id": run_id,
            "project_id": fx.monorepo_pid,
            "config_path": "system.json",
            "goal": "Update OrderService",
            "provider": "claude",
            "status": "completed",
            "session_id": sid,
            "launches": [sid],
        });
        conn.execute(
            "INSERT INTO system_runs(id,data,project_id) VALUES (?1,?2,?3)",
            rusqlite::params![run_id, rec.to_string(), fx.monorepo_pid],
        )
        .unwrap();
    }
    let (system, _, _) = resolve_system(&fx.store, None, &fx.monorepo_pid, None).unwrap();
    let mut run_view = json!({
        "id": run_id,
        "project_id": fx.monorepo_pid,
        "config_path": "system.json",
    });
    fx.bridge
        .enrich_run_view(
            run_id,
            Some(&system),
            Some(&ctx.state),
            std::slice::from_ref(&evidence),
            &mut run_view,
        )
        .unwrap();
    assert!(run_view["graph_focus"]["services"]["orders"].is_object());
    assert!(
        run_view["evidence"]["unopened_callers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["service"] == "orders" && u["path"] == "src/OrderController.php"),
        "expected intra-service unopened caller src/OrderController.php: {run_view}"
    );
    assert!(
        run_view["cross_service_unopened"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["service"] == "web"),
        "expected cross-service unopened web: {run_view}"
    );

    // A run that is going on is asked for its view about once a second.
    // Its evidence is summed up, and what it reaches is worked out, once
    // for the same evidence over the same graphs; the views asked for
    // meanwhile are the same.
    use std::sync::atomic::Ordering;
    let asked = |evidence: &DispatchEvidence| {
        let mut view = json!({
            "id": run_id,
            "project_id": fx.monorepo_pid,
            "config_path": "system.json",
        });
        fx.bridge
            .enrich_run_view(
                run_id,
                Some(&system),
                Some(&ctx.state),
                std::slice::from_ref(evidence),
                &mut view,
            )
            .unwrap();
        view
    };
    let reached = || fx.bridge.reached.load(Ordering::Relaxed);
    let (_, _, members) =
        resolve_all_targets(&fx.store, Some(&fx.graphs), None, &fx.monorepo_pid, None).unwrap();
    // Looking at a run brings the graph of an edited member up to date,
    // which publishes it anew. The graphs stand still once nothing is built.
    let rested = || {
        let started = std::time::Instant::now();
        while members.iter().any(|member| {
            fx.graphs
                .status(&member.graph_id, false)
                .is_ok_and(|status| status["state"] == "building")
        }) {
            assert!(started.elapsed() < std::time::Duration::from_secs(20));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    };
    // An answer worked out on a graph that is then published anew is
    // worked out once more: ask until an answer comes from what was kept.
    let mut settled = Value::Null;
    for round in 0.. {
        assert!(round < 10, "the graphs never stood still");
        rested();
        let before = reached();
        settled = asked(&evidence);
        rested();
        if reached() == before {
            break;
        }
    }
    let base = reached();
    for _ in 0..20 {
        assert_eq!(asked(&evidence), settled);
    }
    assert_eq!(reached(), base);
    // Other evidence is another question: the caller that was not opened
    // is opened now, and is no longer told.
    let mut opened = evidence.clone();
    opened.opened.push("src/OrderController.php".to_owned());
    let after = asked(&opened);
    assert_eq!(reached(), base + 1);
    assert!(
        !after["evidence"]["unopened_callers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|caller| caller["path"] == "src/OrderController.php"),
        "{after}"
    );
    assert_eq!(asked(&opened), after);
    assert_eq!(reached(), base + 1);
    // So is an event of the run that passed the bridge, whatever it held.
    fx.bridge
        .on_emitted_session_event(run_id, None, &json!({"kind": "agent_activity"}));
    assert_eq!(asked(&opened), after);
    assert_eq!(reached(), base + 2);
    // And a graph of a member that was published anew: what the answer is
    // kept by tells one published graph from the next.
    let key = || {
        fx.bridge.reach_key(
            run_id,
            &system,
            &ctx.state,
            std::slice::from_ref(&opened),
            &members,
            &Value::Null,
        )
    };
    let before = key();
    assert_eq!(before, key());
    let graph = crate::graph::live(&fx.store.state, &orders_t.graph_id);
    let text = fs::read_to_string(&graph).unwrap();
    fs::write(&graph, format!("{text}\n")).unwrap();
    assert_ne!(before, key());
}

#[test]
fn multi_root_system_explain_validates_plan_budget_allowed_roots_and_adapts_codex_and_claude() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(run_multi_root_system_explain_test)
        .unwrap()
        .join()
        .unwrap();
}

fn run_multi_root_system_explain_test() {
    let fx = setup_harbor_fixture();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    // Publish Graphify graph on `web` so `system_explain` includes its Graphify offer and allow rules.
    publish_for_tests(&fx.store, &fx.web_pid, &fx.fake_graphify);

    struct DummySpecial;
    impl SpecialWorkflow for DummySpecial {
        fn run<'a>(
            &'a self,
            _: Value,
            _: String,
            _: String,
            _: Arc<std::sync::atomic::AtomicBool>,
            _: crate::sessions::Event,
        ) -> crate::sessions::RunReply<'a> {
            Box::pin(async { Ok(json!({})) })
        }
    }
    struct DummyBlackboards;
    impl crate::sessions::BlackboardFactory for DummyBlackboards {
        fn for_session(&self, _: &Value) -> Result<Arc<dyn crate::fleet::Blackboard>> {
            Err(HarnessError::unavailable("unused"))
        }
    }

    let sessions = rt.block_on(async {
        let providers = Arc::new(ProviderDomain::new(&BTreeMap::new()).await.unwrap());
        if let Some(p) = providers.providers.write().unwrap().get_mut("codex") {
            p.available = true;
            p.executable = Some(PathBuf::from("codex"));
        }
        let knowledge = Arc::new(crate::knowledge::KnowledgeDomain::new(
            fx.store.clone(),
            fx.monorepo_root.clone(),
        ));
        let mut special: BTreeMap<String, Arc<dyn SpecialWorkflow>> = BTreeMap::new();
        for k in ["creator", "system_run", "system_discovery"] {
            special.insert(k.to_owned(), Arc::new(DummySpecial));
        }
        let s = Sessions::new(
            fx.store.clone(),
            fx.monorepo_root.clone(),
            providers.clone(),
            knowledge,
            Arc::new(DummyBlackboards),
            special,
            SessionLimits {
                seconds: 1800,
                concurrency: 1,
            },
        )
        .unwrap();
        let commands = Arc::new(crate::commands::CommandCatalog::new(
            fx.store.clone(),
            fx.monorepo_root.clone(),
            providers.providers.clone(),
        ));
        s.install_commands(commands).unwrap();
        s
    });

    // 1. Rejects `mode: "edit"` (400), `project_context: true` (400), `budgets.seconds > 300` (400), and disallowed `secret` service (403)!
    let edit_err = sessions
        .create_options_for_test(&json!({
            "project_id": fx.monorepo_pid,
            "provider": "codex",
            "prompt": "Explain link",
            "mode": "edit",
            "project_context": false,
            "system_explain": {
                "services": ["orders", "web"]
            }
        }))
        .unwrap_err();
    assert_eq!(edit_err.status, 400);

    let ctx_err = sessions
        .create_options_for_test(&json!({
            "project_id": fx.monorepo_pid,
            "provider": "codex",
            "prompt": "Explain link",
            "mode": "plan",
            "project_context": true,
            "system_explain": {
                "services": ["orders", "web"]
            }
        }))
        .unwrap_err();
    assert_eq!(ctx_err.status, 400);

    let budget_err = sessions
        .create_options_for_test(&json!({
            "project_id": fx.monorepo_pid,
            "provider": "codex",
            "prompt": "Explain link",
            "mode": "plan",
            "project_context": false,
            "budgets": {"seconds": 600},
            "system_explain": {
                "services": ["orders", "web"]
            }
        }))
        .unwrap_err();
    assert_eq!(budget_err.status, 400);

    let forbidden_err = sessions
        .create_options_for_test(&json!({
            "project_id": fx.monorepo_pid,
            "provider": "codex",
            "prompt": "Explain secret",
            "mode": "plan",
            "project_context": false,
            "system_explain": {
                "services": ["secret"]
            }
        }))
        .unwrap_err();
    assert_eq!(forbidden_err.status, 403);

    // 2. Valid `system_explain` launch defaults `budgets.seconds` to 300 and records normalized multi-root metadata!
    let opts = sessions
        .create_options_for_test(&json!({
            "project_id": fx.monorepo_pid,
            "provider": "codex",
            "prompt": "Explain web -> orders contract link",
            "mode": "plan",
            "project_context": false,
            "system_explain": {
                "project_id": fx.monorepo_pid,
                "config_path": "system.json",
                "services": ["orders", "web"]
            }
        }))
        .unwrap();
    let created = fx
        .store
        .create_uploaded(&opts, opts["prompt"].as_str().unwrap(), &[])
        .unwrap();
    assert_eq!(created["budgets"]["seconds"], 300);
    assert_eq!(
        created["system_explain"]["services"],
        json!(["orders", "web"])
    );
    assert_eq!(
        created["system_explain"]["roots"].as_array().unwrap().len(),
        3 // __system__, orders, web
    );

    let prefix = system_explain_prompt_prefix(&fx.store, &created["system_explain"]);
    assert!(prefix.contains("harbor-market"));
    assert!(prefix.contains("`orders`"));
    assert!(prefix.contains("`web`"));
    assert!(prefix.contains("Graphify indexed `web`"));

    // 3. Verify Codex & Claude launch adaptation (`--skip-git-repo-check`, `--add-dir` before `-`, Claude `Read(//dir/**)`).
    let base_launch = Launch {
        provider: "codex".to_owned(),
        executable: PathBuf::from("codex"),
        project: PathBuf::from(created["system_explain"]["primary_root"].as_str().unwrap()),
        prompt: "Explain".to_owned(),
        mode: "plan".to_owned(),
        model: None,
        effort: None,
        session: None,
        usd: None,
        agents: false,
        count: 0,
        hook_events: false,
        permissions: None,
        approvals: None,
    };
    let codex_cmd = adapt_system_explain_launch(
        &fx.store,
        &created,
        &base_launch,
        PreparedCommand {
            argv: vec!["codex".into(), "exec".into(), "-".into()],
            environment: BTreeMap::new(),
            input: Vec::new(),
            readable: Vec::new(),
            remove: Vec::new(),
            prompt_prefix: String::new(),
        },
    )
    .unwrap();
    assert_eq!(codex_cmd.argv.last().unwrap(), "-");
    assert!(codex_cmd.argv.contains(&"--skip-git-repo-check".to_owned()));
    assert!(codex_cmd.argv.contains(&"--add-dir".to_owned()));
    assert_eq!(
        codex_cmd
            .environment
            .get("GRAPHIFY_NO_AUTO_REFRESH")
            .map(String::as_str),
        Some("1")
    );
    assert!(codex_cmd.readable.len() >= 3);

    let mut claude_launch = base_launch;
    claude_launch.provider = "claude".to_owned();
    let claude_cmd = adapt_system_explain_launch(
        &fx.store,
        &created,
        &claude_launch,
        PreparedCommand {
            argv: vec!["claude".into(), "--print".into()],
            environment: BTreeMap::new(),
            input: Vec::new(),
            readable: Vec::new(),
            remove: Vec::new(),
            prompt_prefix: String::new(),
        },
    )
    .unwrap();
    assert!(claude_cmd.argv.contains(&"--add-dir".to_owned()));
    let settings_idx = claude_cmd
        .argv
        .iter()
        .position(|a| a == "--settings")
        .expect("--settings");
    let settings_val: Value = serde_json::from_str(&claude_cmd.argv[settings_idx + 1]).unwrap();
    let allow = settings_val["permissions"]["allow"].as_array().unwrap();
    assert!(
        allow
            .iter()
            .any(|r| r.as_str().unwrap_or("").starts_with("Read(//")),
        "expected Read(//...) rule in Claude settings: {allow:?}"
    );
    assert!(
        allow
            .iter()
            .any(|r| r.as_str().unwrap_or("").starts_with("Bash(")),
        "expected Graphify Bash(...) rule in Claude settings: {allow:?}"
    );
}

#[test]
fn audit_regressions_h1_through_h7_boolean_path_shared_depth_aliases_dirty_isolation_sysrun_cfg_rel_and_agent()
 {
    let fx = setup_harbor_fixture();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let (_, orders_t) = resolve_service_target(
        &fx.store,
        Some(&fx.graphs),
        None,
        &fx.monorepo_pid,
        None,
        "orders",
    )
    .unwrap();
    let (_, billing_t) = resolve_service_target(
        &fx.store,
        Some(&fx.graphs),
        None,
        &fx.monorepo_pid,
        None,
        "billing",
    )
    .unwrap();
    write_sample_graphify_json(
        &fx.store,
        &orders_t.graph_id,
        &fx.fake_graphify,
        json!([
            {"id": "n_svc", "label": "OrderService", "file_type": "code", "source_file": "src/OrderService.php", "source_location": "L2"},
            {"id": "n_ctrl", "label": "OrderController", "file_type": "code", "source_file": "src/OrderController.php", "source_location": "L2"}
        ]),
        json!([
            {"source": "n_ctrl", "target": "n_svc", "relation": "calls", "confidence": "EXTRACTED"}
        ]),
    );
    write_sample_graphify_json(
        &fx.store,
        &billing_t.graph_id,
        &fx.fake_graphify,
        json!([
            {"id": "n_inv", "label": "InvoiceWorker", "file_type": "code", "source_file": "src/InvoiceWorker.php", "source_location": "L3"}
        ]),
        json!([]),
    );
    write_sample_graphify_json(
        &fx.store,
        &fx.web_pid,
        &fx.fake_graphify,
        json!([
            {"id": "n_web", "label": "fetchOrders", "file_type": "code", "source_file": "src/OrdersClient.ts", "source_location": "L1"}
        ]),
        json!([]),
    );

    // H6: Register parent workspace above `harbor-core` in a separate `SessionStore` and verify
    // `resolve_targets_from_system_file` yields `"harbor-core/system.json"` (not bare `"system.json"`).
    {
        let parent_state = fx._tmp.path().join("parent-state");
        let parent_store = SessionStore::open(&parent_state).unwrap();
        let workspace_root = fx._tmp.path();
        let ws_pid = parent_store.register(workspace_root).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let (_, targets) = resolve_targets_from_system_file(
            &parent_store,
            None,
            &fx.monorepo_root.join("system.json"),
        )
        .unwrap();
        let orders_parent_t = targets
            .iter()
            .find(|t| t.service_id == "orders")
            .expect("orders target");
        assert_eq!(orders_parent_t.project_id, ws_pid);
        assert_eq!(orders_parent_t.config_path, "harbor-core/system.json");
    }

    // H3 setup: Dirty `billing` worktree with an unrelated uncommitted edit + create a runtime artifact in `orders`,
    // then run a read-only dispatch in `billing` and verify `on_dispatch_finish` returns EMPTY `git_changed`!
    fs::write(
        fx.billing_root.join("src/InvoiceWorker.php"),
        "<?php\nclass InvoiceWorker {\n    public function sync(): string { return 'dirty-uncommitted orders-http-v1 /api/v1/orders'; }\n}\n",
    )
    .unwrap();
    fs::create_dir_all(fx.orders_root.join("project-brain")).unwrap();
    fs::write(
        fx.orders_root.join("project-brain/findings.md"),
        "# runtime artifact\n",
    )
    .unwrap();

    let read_ctx = DispatchGraphContext {
        run_id: "aaaaaaaa11111111aaaaaaaa11111111".to_owned(),
        journal_run_id: "aaaaaaaa11111111aaaaaaaa11111111".to_owned(),
        launch_id: None,
        execution_dir: fx
            .store
            .state
            .join("ai-system/aaaaaaaa11111111aaaaaaaa11111111/execution"),
        system_file: fx.monorepo_root.join("system.json"),
        provider: "claude".to_owned(),
        mode: "read".to_owned(),
        access: "service".to_owned(),
        tell_agents_graph: Some(true),
        phase: "verify".to_owned(),
        dispatch_id: "verify-a1".to_owned(),
        attempt: 1,
        service: "billing".to_owned(),
        root: fx.billing_root.clone(),
        roots: BTreeMap::from([("billing".to_owned(), fx.billing_root.clone())]),
        selected: vec!["billing".to_owned()],
        readable: vec!["billing".to_owned()],
        writable: Vec::new(),
        base_head: None,
        head_after: None,
        plan: json!({}),
        state: json!({}),
    };
    fx.bridge.on_dispatch_start(&read_ctx).unwrap();
    let read_ev = fx
        .bridge
        .on_dispatch_finish(&read_ctx, &json!({"status": "completed"}))
        .unwrap()
        .expect("read dispatch evidence");
    assert!(
        read_ev.git_changed.is_empty(),
        "read-mode dispatch must never capture pre-existing dirty worktree files: {:?}",
        read_ev.git_changed
    );

    // Seed run `rid` (headless CLI run: `launches: []`, no `session_id`) that ONLY edited `orders/src/OrderService.php`
    // in `service-orders` (plus read-only `contracts` phase in `__system__` reading `orders/src/OrderController.php`).
    let rid = "bbbbbbbb22222222bbbbbbbb22222222";
    {
        let conn = fx.store.connection().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS system_runs(id TEXT PRIMARY KEY,data TEXT NOT NULL,project_id TEXT);",
        )
        .unwrap();
        let rec = json!({
            "id": rid,
            "project_id": fx.monorepo_pid,
            "config_path": "system.json",
            "goal": "Headless CLI orders update",
            "provider": "codex",
            "status": "completed",
            "launches": [],
        });
        conn.execute(
            "INSERT INTO system_runs(id,data,project_id) VALUES (?1,?2,?3)",
            rusqlite::params![rid, rec.to_string(), fx.monorepo_pid],
        )
        .unwrap();
    }
    let rid_exec = fx.store.state.join("ai-system").join(rid).join("execution");
    fs::create_dir_all(&rid_exec).unwrap();
    fs::write(
        rid_exec.join("run.json"),
        serde_json::to_vec_pretty(&json!({
            "status": "completed",
            "selected": ["orders"],
            "steps": [
                {"id": "contracts", "service": "__system__", "role": "read", "status": "completed", "attempt": 1},
                {"id": "service-orders", "service": "orders", "role": "edit", "status": "completed", "attempt": 1}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    let ev_contracts = DispatchEvidence {
        phase: "contracts".to_owned(),
        attempt: 1,
        service: "__system__".to_owned(),
        root: fx.monorepo_root.to_string_lossy().into_owned(),
        opened: vec!["orders/src/OrderController.php".to_owned()],
        steps: 1,
        ..Default::default()
    };
    let ev_orders = DispatchEvidence {
        phase: "service-orders".to_owned(),
        attempt: 1,
        service: "orders".to_owned(),
        root: fx.orders_root.to_string_lossy().into_owned(),
        git_changed: vec![
            EvidenceFileChange {
                path: "src/OrderService.php".to_owned(),
                status: "M".to_owned(),
                from: None,
            },
            EvidenceFileChange {
                path: "project-brain/findings.md".to_owned(),
                status: "A".to_owned(),
                from: None,
            },
        ],
        ledger_edited: vec![EvidenceFileChange {
            path: "src/OrderService.php".to_owned(),
            status: "M".to_owned(),
            from: None,
        }],
        opened: vec!["src/OrderService.php".to_owned()],
        shell_reads: BTreeMap::from([("src/OrderService.php".to_owned(), 1)]),
        steps: 2,
        ..Default::default()
    };
    fs::write(
        rid_exec.join("contracts-a1.evidence.json"),
        serde_json::to_vec_pretty(&ev_contracts).unwrap(),
    )
    .unwrap();
    fs::write(
        rid_exec.join("service-orders-a1.evidence.json"),
        serde_json::to_vec_pretty(&ev_orders).unwrap(),
    )
    .unwrap();

    // H3: `enrich_run_view` must ignore `billing` dirty worktree files AND ignore `project-brain/findings.md`!
    let (system, _, _) = resolve_system(&fx.store, None, &fx.monorepo_pid, None).unwrap();
    let mut run_view = json!({
        "id": rid,
        "project_id": fx.monorepo_pid,
        "config_path": "system.json",
    });
    fx.bridge
        .enrich_run_view(
            rid,
            Some(&system),
            Some(&json!({"selected": ["orders"]})),
            &[ev_contracts, ev_orders],
            &mut run_view,
        )
        .unwrap();
    let git_changed_paths: Vec<&str> = run_view["evidence"]["git_changed"]
        .as_array()
        .expect("evidence.git_changed")
        .iter()
        .filter_map(|c| c["path"].as_str())
        .collect();
    assert_eq!(
        git_changed_paths,
        vec!["orders/src/OrderService.php"],
        "dirty billing worktree and project-brain runtime artifacts must not leak into run evidence: {run_view}"
    );

    rt.block_on(async {
        // H1: Boolean `from` + `to` + `path=1` / `shared=1` AND 400 when `from`/`to` omitted; plus 2-item CSV `path=web,orders`.
        let path_bool = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.map",
                    json!({
                        "query": {
                            "project_id": fx.monorepo_pid,
                            "from": "web",
                            "to": "orders",
                            "path": "1"
                        },
                        "body": {}
                    }),
                )
                .await
                .unwrap(),
        );
        let path_node_ids: Vec<&str> = path_bool["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|n| n["id"].as_str())
            .collect();
        assert!(
            path_node_ids.contains(&"web") && path_node_ids.contains(&"orders"),
            "expected web->orders path nodes: {path_node_ids:?}"
        );

        let shared_bool = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.map",
                    json!({
                        "query": {
                            "project_id": fx.monorepo_pid,
                            "from": "web",
                            "to": "billing",
                            "shared": "1"
                        },
                        "body": {}
                    }),
                )
                .await
                .unwrap(),
        );
        assert!(
            shared_bool["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n["id"] == "orders"),
            "expected shared neighbor orders between web and billing: {shared_bool}"
        );

        let missing_endpoints = fx
            .domain
            .execute(
                "system_graph.map",
                json!({"query": {"project_id": fx.monorepo_pid, "path": "1"}, "body": {}}),
            )
            .await
            .err()
            .expect("missing from/to for path=1");
        assert_eq!(missing_endpoints.status, 400);

        let missing_shared_endpoints = fx
            .domain
            .execute(
                "system_graph.map",
                json!({"query": {"project_id": fx.monorepo_pid, "shared": "true"}, "body": {}}),
            )
            .await
            .err()
            .expect("missing from/to for shared=true");
        assert_eq!(missing_shared_endpoints.status, 400);

        let path_csv = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.map",
                    json!({"query": {"project_id": fx.monorepo_pid, "path": "web,orders"}, "body": {}}),
                )
                .await
                .unwrap(),
        );
        assert!(!path_csv["nodes"].as_array().unwrap().is_empty());

        // H2: `depth`, `out`, `into` accept 1..=64 (reject 65) AND `agents_on` / `agents_off` aliases work!
        let deep_ok = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.map",
                    json!({
                        "query": {
                            "project_id": fx.monorepo_pid,
                            "seed_services": "orders",
                            "depth": "32",
                            "out": "32",
                            "into": "32"
                        },
                        "body": {}
                    }),
                )
                .await
                .unwrap(),
        );
        assert!(!deep_ok["nodes"].as_array().unwrap().is_empty());

        let depth_too_high = fx
            .domain
            .execute(
                "system_graph.map",
                json!({"query": {"project_id": fx.monorepo_pid, "depth": "65"}, "body": {}}),
            )
            .await
            .err()
            .expect("depth=65 out of range");
        assert_eq!(depth_too_high.status, 400);

        let out_too_high = fx
            .domain
            .execute(
                "system_graph.map",
                json!({"query": {"project_id": fx.monorepo_pid, "out": "65"}, "body": {}}),
            )
            .await
            .err()
            .expect("out=65 out of range");
        assert_eq!(out_too_high.status, 400);

        let sys_off = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.act",
                    json!({"query": {}, "body": {"project_id": fx.monorepo_pid, "action": "agents_off"}}),
                )
                .await
                .unwrap(),
        );
        assert_eq!(sys_off["tell_agents"], false);

        let sys_on = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.act",
                    json!({"query": {}, "body": {"project_id": fx.monorepo_pid, "action": "agents_on"}}),
                )
                .await
                .unwrap(),
        );
        assert_eq!(sys_on["tell_agents"], true);

        let svc_off = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.service_act",
                    json!({"query": {}, "body": {"project_id": fx.monorepo_pid, "service": "orders", "action": "agents_off"}}),
                )
                .await
                .unwrap(),
        );
        assert_eq!(svc_off["agents"], false);

        let svc_on = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.service_act",
                    json!({"query": {}, "body": {"project_id": fx.monorepo_pid, "service": "orders", "action": "agents_on"}}),
                )
                .await
                .unwrap(),
        );
        assert_eq!(svc_on["agents"], true);

        // H4 (headless CLI) + H5 (`target=sysrun` / `target=system-run` + `run_id`):
        // Headless run has no Harness session -> `run.session` MUST be `null` (never `"sysrun:..."`) and unverified event id `null`.
        let headless_map = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.service_map",
                    json!({
                        "query": {
                            "project_id": fx.monorepo_pid,
                            "service": "orders",
                            "target": "sysrun",
                            "run_id": rid
                        },
                        "body": {}
                    }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(headless_map["run"]["run_id"], rid);
        assert!(
            headless_map["run"]["session"].is_null(),
            "headless CLI run must report run.session == null, got: {:?}",
            headless_map["run"]["session"]
        );
        let headless_shell_rows = headless_map["run"]["shell_reads"]["rows"]
            .as_array()
            .unwrap();
        assert_eq!(headless_shell_rows.len(), 1);
        assert_eq!(headless_shell_rows[0]["path"], "src/OrderService.php");

        let alias_map = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.service_map",
                    json!({
                        "query": {
                            "project_id": fx.monorepo_pid,
                            "service": "orders",
                            "target": "system-run",
                            "run_id": rid
                        },
                        "body": {}
                    }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(alias_map["run"]["run_id"], rid);

        // H3: `billing` service_map for `rid` must NOT report cross-service impact from its dirty worktree,
        // and `system_graph.changes` with `run_id=rid` must ONLY include `orders` (NOT dirty `billing`)!
        let billing_run_map = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.service_map",
                    json!({
                        "query": {
                            "project_id": fx.monorepo_pid,
                            "service": "billing",
                            "target": "sysrun",
                            "run_id": rid
                        },
                        "body": {}
                    }),
                )
                .await
                .unwrap(),
        );
        assert!(
            billing_run_map["cross_service_unopened"]
                .as_array()
                .unwrap()
                .is_empty(),
            "unselected dirty service must have empty cross_service_unopened: {billing_run_map}"
        );

        // H7: `/api/systems/graph/{map,changes,impact}` honor `target=sysrun` + `run_id` + `agent` filter,
        // and `impact` populates `intra_service`!
        let changes_all = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.changes",
                    json!({
                        "query": {
                            "project_id": fx.monorepo_pid,
                            "target": "sysrun",
                            "run_id": rid
                        },
                        "body": {}
                    }),
                )
                .await
                .unwrap(),
        );
        let changed_sids: Vec<&str> = changes_all["services"]
            .as_array()
            .expect("services array")
            .iter()
            .filter_map(|s| s["service"].as_str())
            .collect();
        assert_eq!(
            changed_sids,
            vec!["orders"],
            "dirty billing worktree must not appear in run-scoped system changes: {changes_all}"
        );

        let changes_contracts_only = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.changes",
                    json!({
                        "query": {
                            "project_id": fx.monorepo_pid,
                            "target": "sysrun",
                            "run_id": rid,
                            "agent": "contracts"
                        },
                        "body": {}
                    }),
                )
                .await
                .unwrap(),
        );
        assert!(
            changes_contracts_only["services"]
                .as_array()
                .unwrap()
                .is_empty(),
            "contracts phase made no edits so agent=contracts changes must exclude orders: {changes_contracts_only}"
        );

        let map_contracts_only = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.map",
                    json!({
                        "query": {
                            "project_id": fx.monorepo_pid,
                            "target": "sysrun",
                            "run_id": rid,
                            "agent": "contracts"
                        },
                        "body": {}
                    }),
                )
                .await
                .unwrap(),
        );
        assert!(
            map_contracts_only["run"]["git_changed"]
                .as_array()
                .unwrap()
                .is_empty(),
            "agent=contracts system map must exclude service-orders edits: {map_contracts_only}"
        );
        assert!(
            map_contracts_only["blast_radius"]["changed_services"]
                .as_array()
                .unwrap_or(&Vec::new())
                .is_empty(),
            "agent=contracts blast_radius must exclude service-orders edits: {map_contracts_only}"
        );

        let sys_impact = unwrap_json(
            fx.domain
                .execute(
                    "system_graph.impact",
                    json!({
                        "query": {
                            "project_id": fx.monorepo_pid,
                            "service": "orders",
                            "path": "src/OrderService.php",
                            "target": "sysrun",
                            "run_id": rid,
                            "agent": "service-orders"
                        },
                        "body": {}
                    }),
                )
                .await
                .unwrap(),
        );
        assert!(
            sys_impact["downstream_services"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["service"] == "web" || s == "web"),
            "expected web in 2-hop cross-service impact of src/OrderService.php: {sys_impact}"
        );
        assert!(
            sys_impact["intra_service"].is_object(),
            "expected intra_service Graphify file_impact overlay: {sys_impact}"
        );
    });
}

#[test]
fn a_link_between_two_services_is_bridged_to_the_code_on_both_sides() {
    let fx = setup_harbor_fixture();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let bridge = |from: &str, to: &str, contract: &str| {
        let mut query = json!({"project_id": fx.monorepo_pid, "from": from, "to": to});
        if !contract.is_empty() {
            query["contract"] = json!(contract);
        }
        rt.block_on(
            fx.domain
                .execute("system_graph.bridge", json!({"query": query, "body": {}})),
        )
    };
    let paths = |side: &Value| {
        side["sites"]
            .as_array()
            .unwrap()
            .iter()
            .map(|site| site["path"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    // web uses orders-http-v1 of orders: its client names it, and the
    // passport of orders says where the contract is made.
    let view = unwrap_json(bridge("web", "orders", "").unwrap());
    assert_eq!(view["kind"], "system_bridge");
    assert_eq!(view["contracts"], json!(["orders-http-v1"]));
    assert_eq!(paths(&view["consumer"]), ["src/OrdersClient.ts"]);
    assert_eq!(
        view["consumer"]["sites"][0]["contracts"],
        json!(["orders-http-v1"])
    );
    assert_eq!(paths(&view["provider"]), ["src/OrderController.php"]);
    let made = &view["provider"]["sites"][0];
    assert_eq!(
        (made["declared"].clone(), made["found"].clone()),
        (json!(true), json!(false))
    );
    // Without a code graph nothing is told of how far a file reaches.
    assert_eq!(view["provider"]["graph"], false);
    assert!(made["reach"].is_null());
    assert_eq!(made["in_graph"], false);
    // billing calls orders without declaring it: its code is the bridge.
    let undeclared = unwrap_json(bridge("billing", "orders", "").unwrap());
    assert_eq!(paths(&undeclared["consumer"]), ["src/InvoiceWorker.php"]);
    // A contract the link does not hold has no code on either side.
    let other = unwrap_json(bridge("web", "orders", "billing-events-v1").unwrap());
    assert!(other["consumer"]["sites"].as_array().unwrap().is_empty());
    assert!(other["provider"]["sites"].as_array().unwrap().is_empty());
    // A link needs two different services.
    assert!(bridge("web", "web", "").is_err());
    assert!(bridge("", "orders", "").is_err());
    // With the code graph of orders, a file of the contract is told how
    // far a change of it reaches inside orders: the routes call the
    // controller.
    let (_, orders_t) = resolve_service_target(
        &fx.store,
        Some(&fx.graphs),
        None,
        &fx.monorepo_pid,
        None,
        "orders",
    )
    .unwrap();
    write_sample_graphify_json(
        &fx.store,
        &orders_t.graph_id,
        &fx.fake_graphify,
        json!([
            {"id": "n_svc", "label": "OrderService", "file_type": "code", "source_file": "src/OrderService.php", "source_location": "L2"},
            {"id": "n_ctrl", "label": "OrderController", "file_type": "code", "source_file": "src/OrderController.php", "source_location": "L2"},
            {"id": "n_routes", "label": "routes()", "file_type": "code", "source_file": "src/routes.php", "source_location": "L1"}
        ]),
        json!([
            {"source": "n_ctrl", "target": "n_svc", "relation": "calls", "confidence": "EXTRACTED"},
            {"source": "n_routes", "target": "n_ctrl", "relation": "calls", "confidence": "EXTRACTED"}
        ]),
    );
    let view = unwrap_json(bridge("web", "orders", "orders-http-v1").unwrap());
    assert_eq!(view["provider"]["graph"], true);
    let made = &view["provider"]["sites"][0];
    assert_eq!(made["in_graph"], true);
    assert_eq!(made["reach"], json!({"files": 1, "tests": 0}));
    assert_eq!(view["contract"], "orders-http-v1");
}
