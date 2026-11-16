//! Comprehensive tests for `SystemGraph`, cross-service contract provenance & risks,
//! architecture layers, cross-service change impact (`cross_service_unopened`),
//! `CodeGraphBridge` hooks, worker sandbox/prompt grants, and `DispatchEvidence`.

use infra_host::provider::PreparedCommand;
use infra_system::{
    ChangedEntityEntry, CodeGraphBridge, CrossServiceImpactQuery, DispatchEvidence,
    DispatchGraphContext, NoopCodeGraphBridge, Project, ServiceGraphOffer, SessionBridge,
    SystemGraph, SystemManager, SystemMapQuery, bwrap_ro_bind_args, catalog::System,
    collect_git_service_summaries, enrich_summaries_from_graph_store, load_run_evidence,
    macos_seatbelt_read_rules, native::prompt_for_with_graphs, offers_for_step, prepare_worker,
    summarize_run_evidence, system_graph_status,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "infra-system-graph-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
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

fn write_text(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn git_init_commit(root: &Path) {
    let run = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(root)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    };
    run(&["init", "-q"]);
    run(&["add", "."]);
    run(&["commit", "-q", "-m", "init"]);
}

/// Creates a polyglot system mixing monorepo subfolders (`monorepo/services/{orders,catalog}`),
/// standalone repositories (`gateway`, `payments`, `notifications`), and an external unallowed
/// `shipping` root (`unavailable_root/shipping`) to exercise every topology & risk edge case.
fn build_polyglot_fixture(base: &Path) -> (PathBuf, Vec<PathBuf>) {
    let sys_dir = base.join("harbor-like-system");
    let mono_dir = base.join("services/monorepo");
    let orders_dir = mono_dir.join("services/orders");
    let catalog_dir = mono_dir.join("services/catalog");
    let gateway_dir = base.join("services/gateway");
    let payments_dir = base.join("services/payments");
    let notifications_dir = base.join("services/notifications");
    let shipping_dir = base.join("outside-unallowed/shipping");

    // System shared sources
    write_text(&sys_dir.join("AGENTS.md"), "# System coordination policy\n");
    write_text(
        &sys_dir.join("contracts/topics.md"),
        "Topics: order.placed, order.cancelled, payment.captured\n",
    );
    write_json(
        &sys_dir.join("system.json"),
        &json!({
            "schema_version": 1,
            "name": "polyglot-commerce",
            "services": [
                { "id": "gateway", "root": "../services/gateway", "manifest": "ai-service.json" },
                { "id": "orders", "root": "../services/monorepo/services/orders", "manifest": "ai-service.json" },
                { "id": "catalog", "root": "../services/monorepo/services/catalog", "manifest": "ai-service.json" },
                { "id": "payments", "root": "../services/payments", "manifest": "ai-service.json" },
                { "id": "notifications", "root": "../services/notifications", "manifest": "ai-service.json" },
                { "id": "shipping", "root": "../outside-unallowed/shipping", "manifest": "ai-service.json" }
            ],
            "shared_sources": [
                { "kind": "policy", "path": "AGENTS.md" },
                { "kind": "contract", "path": "contracts/topics.md" }
            ]
        }),
    );

    // Monorepo root has composer.json -> PHP stack inherited by subfolder services!
    write_json(
        &mono_dir.join("composer.json"),
        &json!({ "name": "acme/monorepo" }),
    );

    // Catalog service (provides catalog.products v2)
    write_text(
        &catalog_dir.join("src/CatalogController.php"),
        "<?php\nfinal class CatalogController { public function list(): array { return []; } }\n",
    );
    write_json(
        &catalog_dir.join("ai-service.json"),
        &json!({
            "schema_version": 1,
            "id": "catalog",
            "owner": "@catalog-team",
            "description": "Product catalog and pricing v2",
            "relationships_complete": true,
            "capabilities": [
                {
                    "id": "catalog.pricing",
                    "description": "Serves product snapshots",
                    "status": "implemented",
                    "sources": ["src/CatalogController.php"],
                    "keywords": ["pricing", "products"]
                }
            ],
            "provides": [
                {
                    "id": "catalog.products",
                    "kind": "http",
                    "version": "2",
                    "sources": ["src/CatalogController.php"]
                }
            ],
            "consumes": [],
            "sources": [
                { "kind": "code", "path": "src/CatalogController.php" }
            ]
        }),
    );

    // Orders service:
    // - consumes catalog.products v1 (MISMATCH against catalog's v2!)
    // - consumes payments:payment.captured v1 (cycle with payments!)
    // - consumes shipping:shipment.quote v1 (UNAVAILABLE provider!)
    // - code file ALSO mentions "email.send" without declaring notifications in `consumes` -> undeclared_in_code!
    write_text(
        &orders_dir.join("src/OrderService.php"),
        "<?php\n// Publishes order.placed and order.cancelled; invokes payment.captured handler and email.send RPC\nfinal class OrderService {}\n",
    );
    write_text(
        &orders_dir.join("tests/OrderTest.php"),
        "<?php\nfinal class OrderTest {}\n",
    );
    write_json(
        &orders_dir.join("ai-service.json"),
        &json!({
            "schema_version": 1,
            "id": "orders",
            "owner": "@checkout-team",
            "description": "Order lifecycle, placement and cancellation",
            "relationships_complete": false,
            "capabilities": [
                {
                    "id": "orders.checkout",
                    "description": "Places and cancels customer orders",
                    "status": "implemented",
                    "sources": ["src/OrderService.php", "tests/OrderTest.php"],
                    "keywords": ["checkout", "cancelled"]
                }
            ],
            "provides": [
                {
                    "id": "orders.api",
                    "kind": "http",
                    "version": "1",
                    "sources": ["src/OrderService.php"]
                },
                {
                    "id": "order.placed",
                    "kind": "event",
                    "version": "2",
                    "sources": ["src/OrderService.php"]
                },
                {
                    "id": "order.cancelled",
                    "kind": "event",
                    "version": "1",
                    "sources": ["src/OrderService.php"]
                }
            ],
            "consumes": [
                { "service": "catalog", "contract": "catalog.products", "version": "1" },
                { "service": "payments", "contract": "payment.captured", "version": "1" },
                { "service": "shipping", "contract": "shipment.quote", "version": "1" }
            ],
            "sources": [
                { "kind": "code", "path": "src/OrderService.php" },
                { "kind": "test", "path": "tests/OrderTest.php" }
            ]
        }),
    );

    // Gateway service (Node.js)
    write_json(
        &gateway_dir.join("package.json"),
        &json!({ "name": "gateway", "version": "1.0.0" }),
    );
    write_text(
        &gateway_dir.join("src/routes.ts"),
        "export const proxy = ['orders.api', 'catalog.products'];\n",
    );
    write_json(
        &gateway_dir.join("ai-service.json"),
        &json!({
            "schema_version": 1,
            "id": "gateway",
            "owner": "@edge-team",
            "description": "Public edge HTTP gateway",
            "relationships_complete": true,
            "capabilities": [
                {
                    "id": "gateway.routing",
                    "description": "Proxies checkout and catalog HTTP APIs",
                    "status": "implemented",
                    "sources": ["src/routes.ts"]
                }
            ],
            "provides": [],
            "consumes": [
                { "service": "orders", "contract": "orders.api", "version": "1" },
                { "service": "catalog", "contract": "catalog.products", "version": "2" }
            ],
            "sources": [
                { "kind": "code", "path": "src/routes.ts" }
            ]
        }),
    );

    // Payments service (Go)
    write_text(
        &payments_dir.join("go.mod"),
        "module example.com/payments\n\ngo 1.22\n",
    );
    write_text(
        &payments_dir.join("internal/consumer/order_placed.go"),
        "package consumer\n// Handles order.placed v2 and emits payment.captured\nfunc HandleOrderPlaced() {}\n",
    );
    write_json(
        &payments_dir.join("ai-service.json"),
        &json!({
            "schema_version": 1,
            "id": "payments",
            "owner": "@payments-team",
            "description": "Captures and refunds order payments",
            "relationships_complete": true,
            "capabilities": [
                {
                    "id": "payments.capture",
                    "description": "Captures payment on order.placed",
                    "status": "implemented",
                    "sources": ["internal/consumer/order_placed.go"]
                }
            ],
            "provides": [
                {
                    "id": "payment.captured",
                    "kind": "event",
                    "version": "1",
                    "sources": ["internal/consumer/order_placed.go"]
                }
            ],
            "consumes": [
                { "service": "orders", "contract": "order.placed", "version": "2" }
            ],
            "sources": [
                { "kind": "code", "path": "internal/consumer/order_placed.go" }
            ]
        }),
    );

    // Notifications service (Python)
    write_text(
        &notifications_dir.join("pyproject.toml"),
        "[project]\nname = \"notifications\"\nversion = \"0.1.0\"\n",
    );
    write_text(
        &notifications_dir.join("app/handlers/cancelled.py"),
        "def on_order_cancelled(evt):\n    return 'order.cancelled'\n",
    );
    write_json(
        &notifications_dir.join("ai-service.json"),
        &json!({
            "schema_version": 1,
            "id": "notifications",
            "owner": "@comms-team",
            "description": "Sends transactional customer emails",
            "relationships_complete": true,
            "capabilities": [
                {
                    "id": "notifications.email",
                    "description": "Emails customer when order.cancelled fires",
                    "status": "implemented",
                    "sources": ["app/handlers/cancelled.py"]
                }
            ],
            "provides": [
                {
                    "id": "email.send",
                    "kind": "rpc",
                    "version": "1",
                    "sources": ["app/handlers/cancelled.py"]
                }
            ],
            "consumes": [
                { "service": "orders", "contract": "order.cancelled", "version": "1" }
            ],
            "sources": [
                { "kind": "code", "path": "app/handlers/cancelled.py" }
            ]
        }),
    );

    // Shipping folder exists on disk, but is NOT in allow_roots!
    write_json(
        &shipping_dir.join("ai-service.json"),
        &json!({
            "schema_version": 1,
            "id": "shipping",
            "owner": "@Fulfillment",
            "description": "Shipping quotes",
            "capabilities": [],
            "provides": [],
            "consumes": [],
            "sources": []
        }),
    );

    git_init_commit(&sys_dir);
    git_init_commit(&mono_dir);
    git_init_commit(&gateway_dir);
    git_init_commit(&payments_dir);
    git_init_commit(&notifications_dir);

    let config = sys_dir.join("system.json");
    let allow = vec![base.join("services")];
    (config, allow)
}

#[test]
fn topology_spread_whole_provenance_and_contract_risks() {
    let root = TempRoot::new("topology");
    let (config, allow) = build_polyglot_fixture(&root.0);
    let system = System::new(&config, &allow).unwrap();

    let summaries = collect_git_service_summaries(&system, &[]);
    let graph = SystemGraph::from_system(&system, summaries.clone());
    let full = graph.to_json();

    assert_eq!(full["system"], "polyglot-commerce");
    assert_eq!(full["summary"]["service_count"], 6);

    // Check stack detection across monorepo subfolders and standalone repos.
    let nodes = full["nodes"].as_array().unwrap();
    let find_node = |id: &str| nodes.iter().find(|n| n["id"] == id).unwrap();
    assert_eq!(find_node("orders")["stack"], "php");
    assert_eq!(find_node("catalog")["stack"], "php");
    assert_eq!(find_node("gateway")["stack"], "node");
    assert_eq!(find_node("payments")["stack"], "go");
    assert_eq!(find_node("notifications")["stack"], "python");
    assert_eq!(find_node("shipping")["access"], "denied");
    assert_eq!(find_node("orders")["broken_risk"], true);

    // Check edge provenance & risks:
    let edges = full["edges"].as_array().unwrap();
    let find_edge = |from: &str, to: &str| {
        edges
            .iter()
            .find(|e| e["from"] == from && e["to"] == to && e["level"] == "service")
            .unwrap_or_else(|| panic!("missing edge {from} -> {to} in {edges:#?}"))
    };

    // gateway -> orders is verified (src/routes.ts cites 'orders.api')
    let gw_orders = find_edge("gateway", "orders");
    assert_eq!(gw_orders["provenance"], "verified");
    assert_eq!(gw_orders["kinds"], json!(["http"]));
    assert_eq!(gw_orders["broken_risk"], false);

    // orders -> catalog has version mismatch (consumes v1 vs provides v2)
    let ord_cat = find_edge("orders", "catalog");
    assert_eq!(ord_cat["broken_risk"], true);
    assert_eq!(
        ord_cat["contracts"][0]["risk_reason"],
        "contract_version_mismatch"
    );

    // orders -> shipping has unavailable provider risk
    let ord_ship = find_edge("orders", "shipping");
    assert_eq!(ord_ship["broken_risk"], true);
    assert_eq!(
        ord_ship["contracts"][0]["risk_reason"],
        "provider_contract_unavailable"
    );

    // orders -> notifications is undeclared_in_code (OrderService.php references email.send)
    let ord_notif = find_edge("orders", "notifications");
    assert_eq!(ord_notif["provenance"], "undeclared_in_code");
    assert_eq!(ord_notif["kinds"], json!(["code", "rpc"]));

    // Spread expansion on `orders` exposes capability, contract, and source child nodes + frame.
    let spread_view = graph.view(&SystemMapQuery {
        spread: vec!["orders".into()],
        ..Default::default()
    });
    let spread_nodes = spread_view["nodes"].as_array().unwrap();
    assert!(
        spread_nodes
            .iter()
            .any(|n| n["id"] == "cap:orders:orders.checkout")
    );
    assert!(
        spread_nodes
            .iter()
            .any(|n| n["id"] == "contract:orders:order.placed")
    );
    assert!(
        spread_nodes
            .iter()
            .any(|n| n["id"] == "source:orders:src/OrderService.php")
    );
    assert_eq!(spread_view["frames"][0]["id"], "orders");

    // Whole expansion unfolds every service + __system__ shared sources.
    let whole_view = graph.view(&SystemMapQuery {
        whole: true,
        ..Default::default()
    });
    let whole_nodes = whole_view["nodes"].as_array().unwrap();
    assert!(
        whole_nodes
            .iter()
            .any(|n| n["id"] == "source:__system__:contracts/topics.md")
    );
    assert!(
        whole_nodes
            .iter()
            .any(|n| n["id"] == "contract:payments:payment.captured")
    );

    // Status helper reflects published Graphify live/graph.json when present in state_dir.
    let state_dir = root.0.join("harness-state");
    write_json(
        &state_dir.join("graphs/orders/live/graph.json"),
        &json!({
            "nodes": [{"id": "a"}, {"id": "b"}, {"id": "c"}],
            "links": [{"source": "a", "target": "b"}]
        }),
    );
    let mut enriched = summaries;
    enrich_summaries_from_graph_store(&system, Some(&state_dir), &mut enriched);
    assert_eq!(enriched["orders"].state, "ready");
    assert_eq!(enriched["orders"].nodes, 3);
    assert_eq!(enriched["orders"].edges, 1);
    let status = system_graph_status(&system, &enriched, &graph);
    assert_eq!(status["ready_count"], 1);
    assert_eq!(status["services"]["orders"]["state"], "ready");
}

#[test]
fn link_shortest_path_shared_cycles_blast_radius_layers_and_find() {
    let root = TempRoot::new("queries");
    let (config, allow) = build_polyglot_fixture(&root.0);

    // Declare service-tier architecture rules in `system-architecture.json` so `orders -> notifications`
    // or `payments -> orders` is checked against explicit layer rules.
    write_json(
        &config.parent().unwrap().join("system-architecture.json"),
        &json!({
            "layers": [
                { "name": "edge", "services": ["gateway"], "allows": ["domain", "foundation"] },
                { "name": "domain", "services": ["orders", "notifications"], "allows": ["foundation", "payments-tier"] },
                { "name": "payments-tier", "services": ["payments"], "allows": ["foundation"] },
                { "name": "foundation", "services": ["catalog", "shipping"], "allows": [] }
            ]
        }),
    );

    let system = System::new(&config, &allow).unwrap();
    let summaries = collect_git_service_summaries(&system, &[]);
    let graph = SystemGraph::from_system(&system, summaries);

    // Bidirectional link between `orders` and `payments`
    let link = graph.link("orders", "payments", false);
    assert_eq!(link["direction"], "bidirectional");
    assert!(link["forward"].is_object());
    assert!(link["reverse"].is_object());

    // Forward shortest path: gateway -> orders -> notifications
    let fwd = graph.shortest_path("gateway", "notifications", Some("orders"), false);
    assert_eq!(fwd["found"], true);
    assert_eq!(fwd["direction"], "forward");
    assert_eq!(fwd["hops"], 2);
    assert_eq!(fwd["steps"], json!(["gateway", "orders", "notifications"]));
    assert_eq!(fwd["focus"]["node"]["id"], "orders");

    // Reverse shortest path when queried from downstream provider back to upstream edge consumer:
    // catalog has no outgoing dependencies, so shortest_path(catalog, gateway) finds reverse gateway -> catalog!
    let rev = graph.shortest_path("catalog", "gateway", None, false);
    assert_eq!(rev["found"], true);
    assert_eq!(rev["direction"], "reverse");
    assert_eq!(rev["steps"], json!(["gateway", "catalog"]));

    // Shared dependencies: gateway and orders both use `catalog`.
    let shared = graph.shared("gateway", "orders", None, 0, false, "all");
    assert_eq!(shared["both_use"], json!(["catalog"]));

    // Shared consumers: payments and notifications are both used by `orders` (and both use `orders`!).
    let shared_consumers = graph.shared("payments", "notifications", None, 0, false, "all");
    assert_eq!(shared_consumers["both_use"], json!(["orders"]));
    assert_eq!(shared_consumers["uses_both"], json!(["orders"]));

    // Cross-service cycles: `orders <-> payments` (and `orders <-> notifications` via undeclared_in_code).
    let cycles = graph.cycles(false);
    assert!(
        cycles.contains(&vec!["orders".into(), "payments".into(), "orders".into()]),
        "cycles: {cycles:?}"
    );

    // Declared architecture layers: `payments` (in `payments-tier`, allows `["foundation"]`)
    // depends on `orders` (in `domain`), which violates `allows`!
    let layers = graph.layers();
    assert_eq!(layers["declared"], true);
    assert_eq!(layers["source_file"], "system-architecture.json");
    assert!(
        layers["violations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["from"] == "payments" && v["to"] == "orders"),
        "expected payments -> orders layer violation: {layers:#}"
    );

    // Blast radius from `catalog`: Wave 1 = gateway, orders; Wave 2 = payments, notifications.
    let blast = graph.blast_radius(&["catalog".into()], &[], 32);
    assert_eq!(blast["waves"][0]["depth"], 1);
    assert_eq!(blast["waves"][0]["services"], json!(["gateway", "orders"]));
    assert_eq!(
        blast["waves"][1]["services"],
        json!(["notifications", "payments"])
    );

    // Find searches across services, contracts, capabilities, and sources.
    let found = graph.find("cancelled");
    let ids: Vec<&str> = found["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert!(ids.contains(&"contract:orders:order.cancelled"), "{ids:?}");
}

#[test]
fn changes_and_cross_service_impact_track_unopened_consumers_and_sed_shell_reads() {
    let root = TempRoot::new("impact");
    let (config, allow) = build_polyglot_fixture(&root.0);

    // Modify both code and test files inside monorepo `orders` subfolder ONLY
    // (leaving sibling `catalog` clean in the same git repository!).
    let orders_root = root.0.join("services/monorepo/services/orders");
    write_text(
        &orders_root.join("src/OrderService.php"),
        "<?php\n// Updated order.placed v2 payload and order.cancelled reason\nfinal class OrderService { public int $v = 2; }\n",
    );
    write_text(
        &orders_root.join("tests/OrderTest.php"),
        "<?php\n// Updated test\nfinal class OrderTest {}\n",
    );

    let system = System::new(&config, &allow).unwrap();
    let mut summaries = collect_git_service_summaries(&system, &[]);
    summaries
        .get_mut("orders")
        .unwrap()
        .changed_entities
        .push(ChangedEntityEntry {
            path: "src/OrderService.php".into(),
            name: "OrderService".into(),
            kind: "class".into(),
            change: "modified".into(),
            visibility: Some("public".into()),
            visibility_narrowed: false,
        });

    // Sibling monorepo subfolder `catalog` must remain clean!
    assert!(!summaries["catalog"].dirty);
    assert!(summaries["catalog"].changed_files.is_empty());
    assert!(summaries["orders"].dirty);
    assert_eq!(summaries["orders"].changed_files.len(), 2);

    let graph = SystemGraph::from_system(&system, summaries);

    // `notests = true` filters out `tests/OrderTest.php`.
    let changes = graph.changes_across_services(None, true);
    assert_eq!(changes["total_files"], 1);
    assert_eq!(changes["total_entities"], 1);
    assert_eq!(changes["services"][0]["service"], "orders");
    assert_eq!(
        changes["downstream_services"],
        json!(["gateway", "notifications", "payments"])
    );

    // 1) When downstream consumer services left their consumer handlers unopened,
    // `cross_service_unopened` flags them!
    let impact_unopened = graph.impact_across_services(&CrossServiceImpactQuery {
        service: Some("orders".into()),
        path: Some("src/OrderService.php".into()),
        ..Default::default()
    });
    let unopened = impact_unopened["cross_service_unopened"]
        .as_array()
        .unwrap();
    assert!(
        unopened.iter().any(|u| {
            u["consumer"] == "payments"
                && u["contract"] == "order.placed"
                && u["path"] == "internal/consumer/order_placed.go"
        }),
        "expected payments consumer handler in cross_service_unopened: {unopened:#?}"
    );
    assert!(
        unopened.iter().any(|u| {
            u["consumer"] == "notifications"
                && u["contract"] == "order.cancelled"
                && u["path"] == "app/handlers/cancelled.py"
        }),
        "expected notifications consumer handler in cross_service_unopened: {unopened:#?}"
    );

    // 2) When `payments` opened its handler (e.g. via `sed -n` shell_reads) and `verify` opened
    // `notifications/app/handlers/cancelled.py`, both clear from `cross_service_unopened`!
    let mut opened_by_service = BTreeMap::<String, BTreeSet<String>>::new();
    opened_by_service
        .entry("payments".into())
        .or_default()
        .insert("internal/consumer/order_placed.go".into());
    opened_by_service
        .entry("verify".into())
        .or_default()
        .insert("notifications/app/handlers/cancelled.py".into());
    opened_by_service
        .entry("gateway".into())
        .or_default()
        .insert("src/routes.ts".into());

    let impact_opened = graph.impact_across_services(&CrossServiceImpactQuery {
        service: Some("orders".into()),
        path: Some("src/OrderService.php".into()),
        opened_by_service,
        ..Default::default()
    });
    assert_eq!(impact_opened["cross_service_unopened"], json!([]));
}

#[test]
fn worker_offers_sandbox_mounts_claude_rules_codex_dirs_and_prompt_injection() {
    let offers = vec![
        ServiceGraphOffer {
            service: "orders".into(),
            command: "graphify".into(),
            graph_path: PathBuf::from("/state/graphs/orders/live/graph.json"),
            nodes: 120,
            edges: 340,
            commit: Some("abc1234".into()),
            readable_paths: vec![PathBuf::from("/state/graphs/orders/live/graph.json")],
            claude_allow_rules: vec![
                "Bash(graphify query * --graph /state/graphs/orders/live/graph.json)".into(),
            ],
            env: BTreeMap::new(),
            prompt_instructions: None,
        },
        ServiceGraphOffer {
            service: "payments".into(),
            command: "graphify".into(),
            graph_path: PathBuf::from("/state/graphs/payments/live/graph.json"),
            nodes: 80,
            edges: 190,
            commit: Some("def5678".into()),
            readable_paths: vec![PathBuf::from("/state/graphs/payments/live/graph.json")],
            claude_allow_rules: vec![
                "Bash(graphify query * --graph /state/graphs/payments/live/graph.json)".into(),
            ],
            env: BTreeMap::new(),
            prompt_instructions: None,
        },
    ];

    let state_own = json!({
        "provider": "claude",
        "mode": "edit",
        "access": "service",
        "selected": ["orders", "payments"],
        "roots": {
            "__system__": "/work/sys",
            "orders": "/work/orders",
            "payments": "/work/payments"
        },
        "tasks": {
            "__system__": {"external_id": "run/sys"},
            "orders": {"external_id": "run/orders"},
            "payments": {"external_id": "run/payments"}
        }
    });

    // contracts & verify receive all selected offers; service-orders (access=service) receives only `orders`.
    let contracts_step = json!({"id": "contracts", "service": "__system__", "mode": "read-only", "dispatch_id": "d".repeat(32)});
    let orders_step = json!({"id": "service-orders", "service": "orders", "mode": "edit", "dispatch_id": "e".repeat(32)});
    let verify_step = json!({"id": "verify", "service": "__system__", "mode": "read-only", "dispatch_id": "f".repeat(32)});

    assert_eq!(
        offers_for_step(&state_own, &contracts_step, &offers).len(),
        2
    );
    assert_eq!(offers_for_step(&state_own, &verify_step, &offers).len(), 2);
    let own_filtered = offers_for_step(&state_own, &orders_step, &offers);
    assert_eq!(own_filtered.len(), 1);
    assert_eq!(own_filtered[0].service, "orders");

    // When `access == "all"`, `service-orders` gets all selected offers.
    let mut state_all = state_own.clone();
    state_all["access"] = json!("all");
    assert_eq!(offers_for_step(&state_all, &orders_step, &offers).len(), 2);

    // When `tell_agents_graph == false`, no offers are granted and prompt omits graph lines.
    let mut state_off = state_all.clone();
    state_off["tell_agents_graph"] = json!(false);
    assert!(offers_for_step(&state_off, &orders_step, &offers).is_empty());

    // Linux bwrap read-only bind args & macOS Seatbelt read rules:
    let bwrap = bwrap_ro_bind_args(&own_filtered).unwrap();
    assert_eq!(
        bwrap,
        vec![
            "--ro-bind",
            "/state/graphs/orders/live/graph.json",
            "/state/graphs/orders/live/graph.json"
        ]
    );
    let seatbelt = macos_seatbelt_read_rules(&own_filtered).unwrap();
    assert!(
        seatbelt.contains("(allow file-read* (literal \"/state/graphs/orders/live/graph.json\"))")
    );

    // Codex `prepare_worker`: sets GRAPHIFY_NO_AUTO_REFRESH=1, adds readable path, inserts `--add-dir` before `-`.
    let codex_cmd = PreparedCommand {
        argv: vec!["codex".into(), "exec".into(), "-".into()],
        environment: BTreeMap::new(),
        remove: vec!["GRAPHIFY_NO_AUTO_REFRESH".into()],
        input: vec![],
        readable: vec![],
    };
    let codex_prepared = prepare_worker("codex", "edit", codex_cmd, &own_filtered).unwrap();
    assert_eq!(
        codex_prepared.environment.get("GRAPHIFY_NO_AUTO_REFRESH"),
        Some(&"1".to_string())
    );
    assert!(
        !codex_prepared
            .remove
            .contains(&"GRAPHIFY_NO_AUTO_REFRESH".to_string())
    );
    assert!(
        codex_prepared
            .readable
            .contains(&PathBuf::from("/state/graphs/orders/live/graph.json"))
    );
    assert_eq!(
        codex_prepared.argv,
        vec![
            "codex",
            "exec",
            "--add-dir",
            "/state/graphs/orders/live",
            "-"
        ]
    );

    // Program and virtualenv grants stay read-only sandbox paths; Codex
    // `--add-dir` (writable in workspace-write) names only the graph folder.
    let mut with_program = own_filtered.clone();
    with_program[0].readable_paths.extend([
        PathBuf::from("/home/u/.local/bin/graphify"),
        PathBuf::from("/home/u/.local/bin"),
        PathBuf::from("/home/u/.venvs/graphify"),
    ]);
    let codex_program = prepare_worker(
        "codex",
        "edit",
        PreparedCommand {
            argv: vec!["codex".into(), "exec".into(), "-".into()],
            environment: BTreeMap::new(),
            remove: vec![],
            input: vec![],
            readable: vec![],
        },
        &with_program,
    )
    .unwrap();
    assert_eq!(
        codex_program.argv,
        vec![
            "codex",
            "exec",
            "--add-dir",
            "/state/graphs/orders/live",
            "-"
        ]
    );
    assert!(
        codex_program
            .readable
            .contains(&PathBuf::from("/home/u/.local/bin"))
    );

    // Claude `prepare_worker`: preserves existing `permissions.allow` and appends Read(...) + Bash(graphify ...).
    let claude_cmd = PreparedCommand {
        argv: vec![
            "claude".into(),
            "-p".into(),
            "--settings".into(),
            json!({"permissions": {"allow": ["Read(//work/sys/**)"]}}).to_string(),
        ],
        environment: BTreeMap::new(),
        remove: vec![],
        input: vec![],
        readable: vec![],
    };
    let claude_prepared = prepare_worker("claude", "edit", claude_cmd, &own_filtered).unwrap();
    let settings_idx = claude_prepared
        .argv
        .iter()
        .position(|a| a == "--settings")
        .unwrap()
        + 1;
    let settings_val: Value = serde_json::from_str(&claude_prepared.argv[settings_idx]).unwrap();
    let allow_rules = settings_val["permissions"]["allow"].as_array().unwrap();
    assert!(allow_rules.contains(&json!("Read(//work/sys/**)")));
    assert!(allow_rules.contains(&json!("Read(//state/graphs/orders/live/**)")));
    assert!(allow_rules.contains(&json!(
        "Bash(graphify query * --graph /state/graphs/orders/live/graph.json)"
    )));

    // Prompt injection when offered vs suppressed when `tell_agents_graph == false`:
    let sample_plan = json!({
        "context": { "task": "Update checkout", "services": ["orders", "payments"], "sources": [] },
        "relationships": [],
        "warnings": [],
        "omitted_sources": []
    });
    let prompt_on = prompt_for_with_graphs(
        &state_own,
        &sample_plan,
        &orders_step,
        &[],
        None,
        &own_filtered,
    )
    .unwrap();
    assert!(
        prompt_on
            .contains("graphify query \"<question>\" --graph /state/graphs/orders/live/graph.json")
    );
    let payload_on: Value = serde_json::from_str(prompt_on.rsplit_once('\n').unwrap().1).unwrap();
    assert_eq!(payload_on["code_graphs"][0]["service"], "orders");

    let prompt_off = prompt_for_with_graphs(
        &state_off,
        &sample_plan,
        &orders_step,
        &[],
        None,
        &own_filtered,
    )
    .unwrap();
    assert!(!prompt_off.contains("graphify query"));
}

#[test]
fn code_graph_bridge_and_dispatch_evidence_persist_and_enrich_system_manager_view() {
    #[derive(Default)]
    struct RecordingBridge {
        events: Mutex<Vec<String>>,
    }

    impl CodeGraphBridge for RecordingBridge {
        fn on_run_start(
            &self,
            run_id: &str,
            _state: &Value,
            _system: &System,
        ) -> infra_core::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("run_start:{run_id}"));
            Ok(())
        }

        fn offers_for_dispatch(
            &self,
            ctx: &DispatchGraphContext,
        ) -> infra_core::Result<Vec<ServiceGraphOffer>> {
            self.events
                .lock()
                .unwrap()
                .push(format!("offers:{}", ctx.phase));
            Ok(vec![ServiceGraphOffer {
                service: "orders".into(),
                command: "graphify".into(),
                graph_path: ctx.execution_dir.join("mock-orders-graph.json"),
                nodes: 42,
                edges: 84,
                commit: ctx.base_head.clone(),
                readable_paths: vec![ctx.execution_dir.join("mock-orders-graph.json")],
                claude_allow_rules: vec![],
                env: BTreeMap::new(),
                prompt_instructions: None,
            }])
        }

        fn on_dispatch_start(&self, ctx: &DispatchGraphContext) -> infra_core::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("start:{}", ctx.phase));
            Ok(())
        }

        fn on_dispatch_signal(
            &self,
            ctx: &DispatchGraphContext,
            raw_line: &[u8],
        ) -> infra_core::Result<()> {
            if std::str::from_utf8(raw_line).is_ok_and(|s| s.contains("completed")) {
                self.events
                    .lock()
                    .unwrap()
                    .push(format!("signal:{}", ctx.phase));
            }
            Ok(())
        }

        fn on_dispatch_finish(
            &self,
            ctx: &DispatchGraphContext,
            _receipt: &Value,
        ) -> infra_core::Result<Option<DispatchEvidence>> {
            self.events
                .lock()
                .unwrap()
                .push(format!("finish:{}", ctx.phase));
            // Let `service-orders` exercise the native JSONL fallback (`None`), and return custom
            // bridge evidence for `contracts` and `verify`!
            if ctx.phase == "service-orders" {
                return Ok(None);
            }
            Ok(Some(DispatchEvidence {
                phase: ctx.phase.clone(),
                attempt: ctx.attempt,
                launch_id: ctx.launch_id.clone(),
                service: ctx.service.clone(),
                root: ctx.root.to_string_lossy().into_owned(),
                graph_asked: ctx.phase == "contracts",
                steps: 2,
                ..Default::default()
            }))
        }

        fn on_run_finish(
            &self,
            run_id: &str,
            state: &Value,
            _system: &System,
        ) -> infra_core::Result<()> {
            self.events.lock().unwrap().push(format!(
                "run_finish:{run_id}:{}",
                state["status"].as_str().unwrap_or("")
            ));
            Ok(())
        }
    }

    struct FakeHost {
        projects: BTreeMap<String, Project>,
        lock_file: Arc<fs::File>,
        sessions: Mutex<BTreeMap<String, Value>>,
    }

    impl SessionBridge for FakeHost {
        fn projects(&self) -> infra_core::Result<BTreeMap<String, Project>> {
            Ok(self.projects.clone())
        }
        fn provider(&self, _id: &str) -> infra_core::Result<Option<PathBuf>> {
            Ok(Some(PathBuf::from("/bin/sh")))
        }
        fn active(&self) -> infra_core::Result<bool> {
            Ok(false)
        }
        fn admit(&self) -> infra_core::Result<()> {
            Ok(())
        }
        fn runner_lock(&self) -> infra_core::Result<Arc<fs::File>> {
            Ok(self.lock_file.clone())
        }
        fn launch(
            &self,
            request: &Value,
            system_run: Option<&Value>,
            _discovery: Option<&Value>,
        ) -> infra_core::Result<Value> {
            let sid = "a".repeat(32);
            let session = json!({
                "id": sid,
                "status": "completed",
                "created_at": "2026-11-15T18:00:00Z",
                "updated_at": "2026-11-15T18:01:00Z",
                "finished_at": "2026-11-15T18:01:00Z",
                "request": request,
                "system_run": system_run
            });
            self.sessions
                .lock()
                .unwrap()
                .insert(sid.clone(), session.clone());
            Ok(session)
        }
        fn session(&self, id: &str) -> infra_core::Result<Value> {
            Ok(self
                .sessions
                .lock()
                .unwrap()
                .get(id)
                .cloned()
                .unwrap_or_else(|| json!({"id": id, "status": "completed"})))
        }
        fn cancel(&self, id: &str) -> infra_core::Result<Value> {
            self.session(id)
        }
        fn events(&self, _id: &str, _kinds: &[&str]) -> infra_core::Result<Vec<Value>> {
            Ok(vec![])
        }
        fn find_launch(&self, _run_id: &str, _nonce: &str) -> infra_core::Result<Option<Value>> {
            Ok(None)
        }
    }

    // Ensure NoopCodeGraphBridge compiles and implements CodeGraphBridge cleanly.
    let _noop: Arc<dyn CodeGraphBridge> = Arc::new(NoopCodeGraphBridge);

    let root = TempRoot::new("bridge");
    let sys_dir = root.0.join("sys");
    let orders_dir = root.0.join("orders");
    let billing_dir = root.0.join("billing");

    write_text(&sys_dir.join("AGENTS.md"), "# Policy\n");
    write_json(
        &sys_dir.join("system.json"),
        &json!({
            "schema_version": 1,
            "name": "bridge-system",
            "services": [
                { "id": "orders", "root": "../orders", "manifest": "ai-service.json" },
                { "id": "billing", "root": "../billing", "manifest": "ai-service.json" }
            ],
            "shared_sources": [{ "kind": "policy", "path": "AGENTS.md" }]
        }),
    );
    write_text(&orders_dir.join("src/order.php"), "<?php // v1\n");
    write_json(
        &orders_dir.join("ai-service.json"),
        &json!({
            "schema_version": 1,
            "id": "orders",
            "owner": "@orders",
            "description": "Orders",
            "capabilities": [{ "id": "orders.place", "description": "Place order", "status": "implemented", "sources": ["src/order.php"] }],
            "provides": [{ "id": "order.placed", "kind": "event", "version": "1", "sources": ["src/order.php"] }],
            "consumes": [],
            "sources": [{ "kind": "code", "path": "src/order.php" }]
        }),
    );
    write_text(
        &billing_dir.join("src/consumer.php"),
        "<?php // handles order.placed\n",
    );
    write_json(
        &billing_dir.join("ai-service.json"),
        &json!({
            "schema_version": 1,
            "id": "billing",
            "owner": "@billing",
            "description": "Billing",
            "capabilities": [{ "id": "billing.charge", "description": "Charge order", "status": "implemented", "sources": ["src/consumer.php"] }],
            "provides": [],
            "consumes": [{ "service": "orders", "contract": "order.placed", "version": "1" }],
            "sources": [{ "kind": "code", "path": "src/consumer.php" }]
        }),
    );
    git_init_commit(&sys_dir);
    git_init_commit(&orders_dir);
    git_init_commit(&billing_dir);

    let repo_source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let bridge = Arc::new(RecordingBridge::default());

    // Verify `load_run_evidence` and `summarize_run_evidence` directly over persisted `.evidence.json`
    // artifacts (live subprocess execution with `drive_run_with_bridge` is covered in `infra-cli/tests/system_execution.rs`
    // where the `CARGO_BIN_EXE_infra-engine` watchdog guardian is compiled).
    let run_dir = root.0.join("direct-run");
    let system = System::new(&sys_dir.join("system.json"), std::slice::from_ref(&root.0)).unwrap();
    let state = json!({
        "status": "completed",
        "steps": [
            { "id": "contracts", "service": "__system__", "attempt": 1 },
            { "id": "service-orders", "service": "orders", "attempt": 1 },
            { "id": "service-billing", "service": "billing", "attempt": 1 },
            { "id": "verify", "service": "__system__", "attempt": 1 }
        ]
    });
    let orders_ev = DispatchEvidence {
        phase: "service-orders".into(),
        attempt: 1,
        launch_id: Some("launch-1".into()),
        service: "orders".into(),
        root: orders_dir.to_string_lossy().into_owned(),
        git_changed: vec![infra_system::EvidenceFileChange {
            path: "src/order.php".into(),
            status: "modified".into(),
            from: None,
        }],
        shell_reads: BTreeMap::from([("src/order.php".into(), 1)]),
        graph_asked: true,
        steps: 3,
        ..Default::default()
    };
    write_json(
        &run_dir.join("service-orders-a1.evidence.json"),
        &serde_json::to_value(&orders_ev).unwrap(),
    );

    let dispatches = load_run_evidence(&run_dir, &state).unwrap();
    assert_eq!(dispatches.len(), 1);
    assert_eq!(dispatches[0].launch_id.as_deref(), Some("launch-1"));
    assert!(dispatches[0].graph_asked);
    assert!(dispatches[0].shell_reads.contains_key("src/order.php"));

    let summary = summarize_run_evidence(Some(&system), &state, &dispatches);
    assert_eq!(summary["graph_asked"], true);
    assert_eq!(summary["shell_opened"], 1);
    // Billing never opened `src/consumer.php`, nor did `verify`, so `cross_service_unopened` flags it!
    assert!(
        summary["cross_service_unopened"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["consumer"] == "billing" && u["path"] == "src/consumer.php"),
        "summary: {summary:#}"
    );

    // Also verify `SystemManager` `tell_agents_graph`, `load_system`, and `view` enrichment!
    let state_dir = root.0.join("mgr-state");
    fs::create_dir_all(&state_dir).unwrap();
    let lock_file = Arc::new(fs::File::create(root.0.join("runner.lock")).unwrap());
    let mut projects = BTreeMap::new();
    projects.insert(
        "proj-sys".into(),
        Project {
            id: "proj-sys".into(),
            name: "sys".into(),
            path: sys_dir.clone(),
        },
    );
    projects.insert(
        "proj-orders".into(),
        Project {
            id: "proj-orders".into(),
            name: "orders".into(),
            path: orders_dir.clone(),
        },
    );
    projects.insert(
        "proj-billing".into(),
        Project {
            id: "proj-billing".into(),
            name: "billing".into(),
            path: billing_dir.clone(),
        },
    );
    let host = Arc::new(FakeHost {
        projects,
        lock_file,
        sessions: Mutex::new(BTreeMap::new()),
    });
    let mgr = SystemManager::new(repo_source, state_dir, host)
        .unwrap()
        .with_code_graph_bridge(bridge)
        .unwrap();

    let loaded_sys = mgr.load_system("proj-sys", "system.json").unwrap();
    assert_eq!(loaded_sys.config_data["name"], "bridge-system");

    let created = mgr
        .execute(
            "systems.prepare",
            &json!({
                "project_id": "proj-sys",
                "config_path": "system.json",
                "task": "Check order placement",
                "change_id": "chg-mgr",
                "services": ["orders"]
            }),
        )
        .unwrap();
    let rid = created["id"].as_str().unwrap().to_string();
    let rev = created["revision"].as_u64().unwrap();

    let launched = mgr
        .execute(
            "systems.act",
            &json!({
                "params": { "id": rid },
                "body": {
                    "action": "execute",
                    "revision": rev,
                    "provider": "codex",
                    "mode": "read-only",
                    "tell_agents_graph": true
                }
            }),
        )
        .unwrap();
    assert_eq!(launched["tell_agents_graph"], true);
    assert!(launched["evidence"].is_object());
    assert_eq!(launched["launches"][0]["launch_number"], 1);
}

#[test]
fn synthetic_graph_ids_runtime_records_and_run_scoped_summaries() {
    let root = TempRoot::new("scoped");
    let sys_dir = root.0.join("sys");
    let orders_dir = root.0.join("orders");
    let billing_dir = root.0.join("billing");
    write_text(&sys_dir.join("AGENTS.md"), "# Policy\n");
    write_json(
        &sys_dir.join("system.json"),
        &json!({
            "schema_version": 1,
            "name": "scoped-system",
            "services": [
                { "id": "orders", "root": "../orders", "manifest": "ai-service.json" },
                { "id": "billing", "root": "../billing", "manifest": "ai-service.json" }
            ],
            "shared_sources": [{ "kind": "policy", "path": "AGENTS.md" }]
        }),
    );
    write_text(&orders_dir.join("src/order.php"), "<?php // v1\n");
    write_json(
        &orders_dir.join("ai-service.json"),
        &json!({
            "schema_version": 1, "id": "orders", "owner": "@orders", "description": "Orders",
            "capabilities": [{ "id": "orders.place", "description": "Place order", "status": "implemented", "sources": ["src/order.php"] }],
            "provides": [{ "id": "order.placed", "kind": "event", "version": "1", "sources": ["src/order.php"] }],
            "consumes": [],
            "sources": [{ "kind": "code", "path": "src/order.php" }]
        }),
    );
    write_text(
        &billing_dir.join("src/consumer.php"),
        "<?php // order.placed\n",
    );
    write_text(&billing_dir.join("src/unrelated.php"), "<?php // v1\n");
    write_json(
        &billing_dir.join("ai-service.json"),
        &json!({
            "schema_version": 1, "id": "billing", "owner": "@billing", "description": "Billing",
            "capabilities": [{ "id": "billing.charge", "description": "Charge order", "status": "implemented", "sources": ["src/consumer.php"] }],
            "provides": [],
            "consumes": [{ "service": "orders", "contract": "order.placed", "version": "1" }],
            "sources": [{ "kind": "code", "path": "src/consumer.php" }]
        }),
    );
    git_init_commit(&sys_dir);
    git_init_commit(&orders_dir);
    git_init_commit(&billing_dir);

    let system = System::new(&sys_dir.join("system.json"), std::slice::from_ref(&root.0)).unwrap();

    // Runtime records written by the Brain or Memory Bank are not source edits.
    write_text(&billing_dir.join("project-brain/tasks/T-1.json"), "{}\n");
    write_text(&billing_dir.join("memory-bank/MEM-1.md"), "note\n");
    write_text(&billing_dir.join(".agentic-security/scan.json"), "{}\n");
    // A pre-existing edit in a service the run never touched.
    write_text(&billing_dir.join("src/unrelated.php"), "<?php // v2\n");
    let worktree = infra_system::git_worktree_changes(&billing_dir);
    let paths: Vec<&str> = worktree.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(paths, vec!["src/unrelated.php"], "{paths:?}");

    // Worktree summaries see billing's own uncommitted edit; run-scoped
    // summaries attribute only what the run's dispatches changed.
    let orders_ev = DispatchEvidence {
        phase: "service-orders".into(),
        attempt: 1,
        service: "orders".into(),
        root: orders_dir.to_string_lossy().into_owned(),
        git_changed: vec![infra_system::EvidenceFileChange {
            path: "src/order.php".into(),
            status: "modified".into(),
            from: None,
        }],
        ..Default::default()
    };
    let dispatches = vec![orders_ev];
    let worktree_summaries = collect_git_service_summaries(&system, &dispatches);
    assert_eq!(worktree_summaries["billing"].changed_files.len(), 1);
    let run_summaries = infra_system::collect_run_service_summaries(&system, &dispatches);
    assert!(run_summaries["billing"].changed_files.is_empty());
    assert_eq!(run_summaries["orders"].changed_files.len(), 1);
    let state = json!({ "status": "completed", "selected": ["orders"], "access": "service" });
    let summary = summarize_run_evidence(Some(&system), &state, &dispatches);
    let unopened = summary["cross_service_unopened"].as_array().unwrap();
    assert!(
        unopened.iter().all(|u| u["provider"] == "orders"),
        "{summary:#}"
    );
    assert!(
        unopened
            .iter()
            .any(|u| u["consumer"] == "billing" && u["path"] == "src/consumer.php"),
        "{summary:#}"
    );

    // Members inside a registered parent workspace use Harness's synthetic
    // `sys-<sha256(canonical root)[..16]>` graph id.
    let state_dir = root.0.join("harness-state");
    let canon = infra_core::fs::canonicalize(&orders_dir).unwrap();
    let synth = format!(
        "sys-{}",
        &infra_core::query::sha256(canon.to_string_lossy().as_bytes())[..16]
    );
    write_json(
        &state_dir
            .join("graphs")
            .join(&synth)
            .join("live/graph.json"),
        &json!({ "nodes": [{"id": "a"}, {"id": "b"}], "links": [{"source": "a", "target": "b"}] }),
    );
    let mut enriched = collect_git_service_summaries(&system, &[]);
    enrich_summaries_from_graph_store(&system, Some(&state_dir), &mut enriched);
    assert_eq!(enriched["orders"].state, "ready");
    assert_eq!(enriched["orders"].nodes, 2);
    assert_eq!(enriched["billing"].state, "unindexed");
}
