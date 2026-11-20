use super::*;

#[test]
fn parses_all_fourteen_plus_languages_without_panicking() {
    let samples = [
        (
            "src/App.php",
            "<?php\nnamespace App;\nclass App { public function run(): void {} }\n",
        ),
        (
            "src/lib.rs",
            "pub struct Core { pub x: i32 }\npub fn boot() {}\n",
        ),
        ("src/index.ts", "export class Client { fetch(): void {} }\n"),
        (
            "src/Button.tsx",
            "export const Button = () => <button>Ok</button>;\n",
        ),
        (
            "src/util.js",
            "export function add(a, b) { return a + b; }\n",
        ),
        (
            "src/Card.jsx",
            "export function Card() { return <div />; }\n",
        ),
        (
            "app/main.py",
            "class Svc:\n    def handle(self) -> int:\n        return 1\n",
        ),
        (
            "cmd/srv.go",
            "package main\ntype Server struct { Port int }\nfunc (s *Server) Start() {}\n",
        ),
        (
            "src/Main.java",
            "package acme;\npublic class Main { public void run() {} }\n",
        ),
        (
            "src/Auth.kt",
            "package acme\nclass Auth(val key: String) { fun verify() = true }\n",
        ),
        (
            "src/Svc.cs",
            "namespace Acme;\npublic class Svc { public int Count { get; set; } public void Go() {} }\n",
        ),
        (
            "app/user.rb",
            "class User < ActiveRecord::Base\n  def save\n  end\nend\n",
        ),
        (
            "Sources/App.swift",
            "import Foundation\npublic struct Config { public let port: Int }\n",
        ),
        (
            "src/Main.scala",
            "package acme\nclass Main extends Base { def run(): Unit = () }\n",
        ),
    ];
    for (path, src) in samples {
        let ir = parse_file_ir(path, src).unwrap_or_else(|| panic!("failed to parse {path}"));
        assert!(
            !ir.declarations.is_empty(),
            "expected declarations for {path}"
        );
    }
}

#[test]
fn extracts_framework_routes_di_events_and_sql_semantics() {
    let php = r#"<?php
namespace App\Http;
use Symfony\Component\Routing\Annotation\Route;
use Symfony\Component\Messenger\Attribute\AsMessageHandler;

#[Route('/api/orders', methods: ['POST'])]
class OrderController {
    public function store(): void {
        Route::get('/api/orders/health', fn () => 'ok');
        $this->app->bind(OrderRepoInterface::class, SqlOrderRepo::class);
        DB::table('orders')->insert(['total' => 10]);
        $rows = DB::select('SELECT id FROM customers JOIN invoices ON invoices.cust_id = customers.id');
        OrderPlaced::dispatch();
    }
}

#[AsMessageHandler]
class OrderPlacedHandler {
    public function __invoke(OrderPlaced $event): void {}
}
"#;
    let ir = parse_file_ir("src/Http/OrderController.php", php).expect("php ir");
    assert!(
        ir.semantic_edges
            .iter()
            .any(|e| e.kind == "exposes_route" && e.target.contains("/api/orders")),
        "{:?}",
        ir.semantic_edges
    );
    assert!(
        ir.semantic_edges
            .iter()
            .any(|e| e.kind == "exposes_route" && e.target == "route:GET /api/orders/health"),
        "{:?}",
        ir.semantic_edges
    );
    assert!(
        ir.semantic_edges
            .iter()
            .any(|e| e.kind == "di_binds" && e.target == "service:OrderRepoInterface"),
        "{:?}",
        ir.semantic_edges
    );
    assert!(
        ir.semantic_edges
            .iter()
            .any(|e| e.kind == "writes_table" && e.target == "table:orders"),
        "{:?}",
        ir.semantic_edges
    );
    assert!(
        ir.semantic_edges
            .iter()
            .any(|e| e.kind == "reads_table" && e.target == "table:customers"),
        "{:?}",
        ir.semantic_edges
    );
    assert!(
        ir.semantic_edges
            .iter()
            .any(|e| e.kind == "reads_table" && e.target == "table:invoices"),
        "{:?}",
        ir.semantic_edges
    );
    assert!(
        ir.semantic_edges
            .iter()
            .any(|e| e.kind == "publishes_event" && e.target == "event:OrderPlaced"),
        "{:?}",
        ir.semantic_edges
    );
    assert!(
        ir.semantic_edges
            .iter()
            .any(|e| e.kind == "handles_event" && e.target == "event:OrderPlaced"),
        "{:?}",
        ir.semantic_edges
    );
}

#[test]
fn handles_syntax_errors_truncated_inputs_and_oversized_inputs_gracefully() {
    let broken_rs = "pub fn broken(x: i32 { let s = \"unterminated;\n";
    let ir = parse_file_ir("src/broken.rs", broken_rs).expect("partial tree");
    assert_eq!(ir.language, "rust");

    let huge = "x".repeat(MAX_AST_BYTES + 1);
    assert!(parse_file_ir("src/huge.rs", &huge).is_none());

    let deep = format!("fn f() {{ {} }}", "(".repeat(400) + &")".repeat(400));
    assert!(parse_file_ir("src/deep.rs", &deep).is_some());
}

#[test]
fn notify_event_filter_keeps_only_code_paths() {
    let ev = notify::Event {
        kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Content,
        )),
        paths: vec![
            PathBuf::from("src/app.rs"),
            PathBuf::from("README.md"),
            PathBuf::from("cmd/server.go"),
        ],
        attrs: Default::default(),
    };
    assert_eq!(
        notify_event_touches_code(&ev),
        vec![PathBuf::from("src/app.rs"), PathBuf::from("cmd/server.go")]
    );
}

#[test]
fn extracts_cross_ecosystem_routes_di_events_and_tables() {
    let java = r#"
@Service
public class BillingService {
    @GetMapping("/v1/bills")
    public void list() {}

    @EventListener
    public void onPaid(PaymentSettled evt) {}
}
"#;
    let java_ir = parse_file_ir("src/BillingService.java", java).expect("java ir");
    assert!(
        java_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "exposes_route" && e.target == "route:GET /v1/bills"),
        "{:?}",
        java_ir.semantic_edges
    );
    assert!(
        java_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "di_binds" && e.target == "service:BillingService"),
        "{:?}",
        java_ir.semantic_edges
    );
    assert!(
        java_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "handles_event" && e.target == "event:PaymentSettled"),
        "{:?}",
        java_ir.semantic_edges
    );

    let ts = r#"
@Injectable()
export class GatewayClient {}
app.get('/v1/health', handler);
bus.emit('InvoiceIssued', payload);
"#;
    let ts_ir = parse_file_ir("src/gateway.ts", ts).expect("ts ir");
    assert!(
        ts_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "di_binds" && e.target == "service:GatewayClient"),
        "{:?}",
        ts_ir.semantic_edges
    );
    assert!(
        ts_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "exposes_route" && e.target == "route:GET /v1/health"),
        "{:?}",
        ts_ir.semantic_edges
    );
    assert!(
        ts_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "publishes_event" && e.target == "event:InvoiceIssued"),
        "{:?}",
        ts_ir.semantic_edges
    );

    let py = r#"
@app.get("/py/items")
def list_items():
    return []
"#;
    let py_ir = parse_file_ir("api/items.py", py).expect("py ir");
    assert!(
        py_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "exposes_route" && e.target == "route:GET /py/items"),
        "{:?}",
        py_ir.semantic_edges
    );

    let rs = r#"
diesel::table! {
    ledger_entries (id) {
        id -> Int4,
    }
}
fn router() {
    Router::new().route("/rust/ping", get(ping));
}
"#;
    let rs_ir = parse_file_ir("src/routes.rs", rs).expect("rs ir");
    assert!(
        rs_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "reads_table" && e.target == "table:ledger_entries"),
        "{:?}",
        rs_ir.semantic_edges
    );
    assert!(
        rs_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "exposes_route" && e.target == "route:* /rust/ping"),
        "{:?}",
        rs_ir.semantic_edges
    );

    let go = r#"
package web
func register(mux *ServeMux) {
    mux.HandleFunc("/go/ready", okHandler)
}
"#;
    let go_ir = parse_file_ir("web/routes.go", go).expect("go ir");
    assert!(
        go_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "exposes_route" && e.target == "route:* /go/ready"),
        "{:?}",
        go_ir.semantic_edges
    );

    let cs = r#"
public class Startup {
    public void Configure(IServiceCollection services) {
        services.AddSingleton<ITokenCache, RedisTokenCache>();
    }
}
"#;
    let cs_ir = parse_file_ir("Startup.cs", cs).expect("cs ir");
    assert!(
        cs_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "di_binds" && e.target == "service:ITokenCache"),
        "{:?}",
        cs_ir.semantic_edges
    );

    let php = r#"<?php
#[ORM\Table(name: 'audit_logs')]
class AuditLog {}
"#;
    let php_ir = parse_file_ir("src/Entity/AuditLog.php", php).expect("php ir");
    assert!(
        php_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "reads_table" && e.target == "table:audit_logs"),
        "{:?}",
        php_ir.semantic_edges
    );
}

#[test]
fn notify_event_filter_includes_manifests_and_architecture_config() {
    let ev = notify::Event {
        kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Content,
        )),
        paths: vec![
            PathBuf::from("Cargo.toml"),
            PathBuf::from("package.json"),
            PathBuf::from("tsconfig.json"),
            PathBuf::from("composer.json"),
            PathBuf::from("pyproject.toml"),
            PathBuf::from("go.mod"),
            PathBuf::from(".harness-architecture.json"),
            PathBuf::from("docs/notes.md"),
        ],
        attrs: Default::default(),
    };
    assert_eq!(
        notify_event_touches_code(&ev),
        vec![
            PathBuf::from("Cargo.toml"),
            PathBuf::from("package.json"),
            PathBuf::from("tsconfig.json"),
            PathBuf::from("composer.json"),
            PathBuf::from("pyproject.toml"),
            PathBuf::from("go.mod"),
            PathBuf::from(".harness-architecture.json"),
        ]
    );
}

#[test]
fn preserves_di_concrete_targets_calls_route_and_rust_test_spans() {
    let php = r#"<?php
class AppProvider {
    public function register(): void {
        $this->app->bind(OrderRepoInterface::class, SqlOrderRepo::class);
        Http::post('https://billing.internal/v1/charge', ['id' => 1]);
    }
}
"#;
    let php_ir = parse_file_ir("src/Providers/AppProvider.php", php).expect("php ir");
    assert!(
        php_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "di_binds" && e.target == "service:OrderRepoInterface"),
        "{:?}",
        php_ir.semantic_edges
    );
    assert!(
        php_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "di_binds" && e.target == "service:SqlOrderRepo"),
        "{:?}",
        php_ir.semantic_edges
    );
    assert!(
        php_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "calls_route" && e.target == "route:POST /v1/charge"),
        "{:?}",
        php_ir.semantic_edges
    );

    let cs = r#"
public class Startup {
    public void Configure(IServiceCollection services) {
        services.AddSingleton<ITokenCache, RedisTokenCache>();
    }
}
"#;
    let cs_ir = parse_file_ir("Startup.cs", cs).expect("cs ir");
    assert!(
        cs_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "di_binds" && e.target == "service:RedisTokenCache"),
        "{:?}",
        cs_ir.semantic_edges
    );

    let ts = r#"
export async function loadBills() {
    await fetch('/v1/bills');
    return axios.post('/v1/bills/pay', {});
}
"#;
    let ts_ir = parse_file_ir("src/client.ts", ts).expect("ts ir");
    assert!(
        ts_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "calls_route" && e.target == "route:GET /v1/bills"),
        "{:?}",
        ts_ir.semantic_edges
    );
    assert!(
        ts_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "calls_route" && e.target == "route:POST /v1/bills/pay"),
        "{:?}",
        ts_ir.semantic_edges
    );

    let rs = r#"
pub fn helper() {}

#[cfg(test)]
mod tests {
    #[test]
    fn works() {
        super::helper();
    }
}
"#;
    let rs_ir = parse_file_ir("src/lib.rs", rs).expect("rs ir");
    assert!(
        !rs_ir.test_spans.is_empty(),
        "expected #[cfg(test)] span in {:?}",
        rs_ir.declarations
    );
}

#[test]
fn watcher_dir_events_sql_cte_joins_and_blade_exclusion_work() {
    use notify::{
        Event, EventKind,
        event::{CreateKind, ModifyKind, RemoveKind, RenameMode},
    };

    assert!(AstGrammar::from_rel_path("resources/views/home.blade.php").is_none());

    let ev_dir_rm = Event::new(EventKind::Remove(RemoveKind::Folder))
        .add_path(PathBuf::from("/proj/src/legacy"))
        .add_path(PathBuf::from("/proj/target/debug"));
    let kept_rm = notify_event_touches_code(&ev_dir_rm);
    assert_eq!(kept_rm, vec![PathBuf::from("/proj/src/legacy")]);

    let ev_dir_rename = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
        .add_path(PathBuf::from("/proj/src/old_mod"))
        .add_path(PathBuf::from("/proj/src/new_mod"))
        .add_path(PathBuf::from("/proj/node_modules/pkg"));
    let kept_mv = notify_event_touches_code(&ev_dir_rename);
    assert_eq!(
        kept_mv,
        vec![
            PathBuf::from("/proj/src/old_mod"),
            PathBuf::from("/proj/src/new_mod"),
        ]
    );

    let ev_dir_add = Event::new(EventKind::Create(CreateKind::Folder))
        .add_path(PathBuf::from("/proj/src/feature"));
    assert_eq!(
        notify_event_touches_code(&ev_dir_add),
        vec![PathBuf::from("/proj/src/feature")]
    );

    let php_sql = r#"<?php
class ReportRepo {
    public function run($db) {
        return $db->query("WITH recent_orders AS (SELECT * FROM invoices) SELECT o.id FROM recent_orders, payments p JOIN customers c ON c.id = o.customer_id; MERGE INTO audit_log t USING invoices s ON t.id = s.id WHEN MATCHED THEN UPDATE SET n = 1; TRUNCATE TABLE temp_queue");
    }
}
"#;
    let ir = parse_file_ir("src/ReportRepo.php", php_sql).expect("php sql ir");
    let reads: Vec<&str> = ir
        .semantic_edges
        .iter()
        .filter(|e| e.kind == "reads_table")
        .map(|e| e.target.as_str())
        .collect();
    let writes: Vec<&str> = ir
        .semantic_edges
        .iter()
        .filter(|e| e.kind == "writes_table")
        .map(|e| e.target.as_str())
        .collect();
    assert!(reads.contains(&"table:invoices"), "{reads:?}");
    assert!(reads.contains(&"table:payments"), "{reads:?}");
    assert!(reads.contains(&"table:customers"), "{reads:?}");
    assert!(
        !reads.contains(&"table:recent_orders"),
        "CTE alias should not emit table read: {reads:?}"
    );
    assert!(writes.contains(&"table:audit_log"), "{writes:?}");
    assert!(writes.contains(&"table:temp_queue"), "{writes:?}");
}

#[test]
fn route_contracts_normalize_params_and_wildcard_verbs_across_frameworks() {
    assert_eq!(
        normalize_route_contract(
            Some("get"),
            "https://api.example.com/API/Users/:id?active=1",
            true
        )
        .as_deref(),
        Some("route:GET /api/users/{param}")
    );
    assert_eq!(
        normalize_route_contract(
            Some("ANY"),
            "/v1/Orders/{orderId}/Items/<int:itemId>/",
            false
        )
        .as_deref(),
        Some("route:* /v1/orders/{param}/items/{param}")
    );
    assert_eq!(
        normalize_route_contract(None, "v1/items/[id]", false).as_deref(),
        Some("route:* /v1/items/{param}")
    );
    let ts = r#"
export async function loadOrder(id: string) {
    return fetch(`/api/orders/${id}/items?full=true`);
}
"#;
    let ts_ir = parse_file_ir("web/client.ts", ts).expect("ts ir");
    assert!(
        ts_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "calls_route" && e.target == "route:GET /api/orders/{param}/items"),
        "{:?}",
        ts_ir.semantic_edges
    );
}
