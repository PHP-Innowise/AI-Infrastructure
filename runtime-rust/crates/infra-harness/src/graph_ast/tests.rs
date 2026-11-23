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
fn watcher_dir_events_sql_cte_joins_and_blade_support_work() {
    use notify::{
        Event, EventKind,
        event::{CreateKind, ModifyKind, RemoveKind, RenameMode},
    };

    assert_eq!(
        AstGrammar::from_rel_path("resources/views/home.blade.php"),
        Some(AstGrammar::Blade)
    );

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
export async function loadOrder(id: string, a: string, b: string) {
    const a1 = fetch(`/api/orders/${id}/items?full=true`);
    const a2 = fetch(`${BASE}/orders/${id}`);
    const a3 = fetch(`/api/prefix-${id}/${a + b}?mode=${id ? 'x' : 'y'}`);
    return [a1, a2, a3];
}
"#;
    let ts_ir = parse_file_ir("web/client.ts", ts).expect("ts ir");
    for expected in [
        "route:GET /api/orders/{param}/items",
        "route:GET /orders/{param}",
        "route:GET /api/prefix-{param}/{param}",
    ] {
        assert!(
            ts_ir
                .semantic_edges
                .iter()
                .any(|e| e.kind == "calls_route" && e.target == expected),
            "missing `{expected}` in {:?}",
            ts_ir.semantic_edges
        );
    }
    assert_eq!(
        normalize_route_contract(Some("get"), "${BASE}/orders/${id}", true).as_deref(),
        Some("route:GET /orders/{param}")
    );
    assert_eq!(
        normalize_route_contract(None, "prefix-${id}", false).as_deref(),
        Some("route:* /prefix-{param}")
    );
    assert_eq!(
        normalize_route_contract(None, "${a + b}", false).as_deref(),
        Some("route:* /{param}")
    );
    assert_eq!(
        normalize_route_contract(
            Some("post"),
            "https://${cond ? 'a/b' : 'c'}/api/prefix-${orderId}/${a + b}?q=${x ? 1 : 0}",
            true
        )
        .as_deref(),
        Some("route:POST /api/prefix-{param}/{param}")
    );
    assert_eq!(
        normalize_route_contract(Some("get"), "${`foo/${bar}`}/orders/${id}", true).as_deref(),
        Some("route:GET /orders/{param}")
    );
    assert_eq!(
        normalize_route_contract(Some("get"), "${PROTO}${HOST}/orders/${id}", true).as_deref(),
        Some("route:GET /orders/{param}")
    );
    assert_eq!(
        normalize_route_contract(Some("get"), "/users/{id:[0-9]{2,4}}", true).as_deref(),
        Some("route:GET /users/{param}")
    );
    // Bare template expression without `/` and prose whitespace outside `${...}` must stay rejected.
    assert_eq!(normalize_route_contract(Some("get"), "${url}", true), None);
    assert_eq!(
        normalize_route_contract(Some("get"), "/api/orders with space", true),
        None
    );
}

#[test]
fn ts_js_walks_nested_functions_closures_and_local_classes_without_leaking_decls() {
    let ts = r#"
import { BillingClient } from './billing';
import { AuditLog } from './audit';

export function orchestrate(client: BillingClient) {
    function innerStep(audit: AuditLog) {
        audit.record('nested');
        return fetch('/api/internal/sync');
    }
    class LocalHelper {
        run() {
            client.charge();
        }
    }
    return [innerStep, new LocalHelper()];
}
"#;
    let ir = parse_file_ir("src/orchestrate.ts", ts).expect("ts ir");
    assert!(
        ir.declarations
            .iter()
            .any(|d| d.entity.name == "orchestrate"),
        "{:?}",
        ir.declarations
    );
    assert!(
        !ir.declarations
            .iter()
            .any(|d| d.entity.name == "innerStep" || d.entity.name == "LocalHelper"),
        "nested declarations should stay scoped inside outer fn: {:?}",
        ir.declarations
    );
    assert!(
        ir.calls_and_refs
            .iter()
            .any(|r| r.target_name == "AuditLog" && r.member.as_deref() == Some("record")),
        "expected nested function param + call captured: {:?}",
        ir.calls_and_refs
    );
    assert!(
        ir.calls_and_refs
            .iter()
            .any(|r| r.target_name == "BillingClient" && r.member.as_deref() == Some("charge")),
        "expected nested class method call with captured outer param type: {:?}",
        ir.calls_and_refs
    );
    assert!(
        ir.semantic_edges
            .iter()
            .any(|e| e.kind == "calls_route" && e.target == "route:GET /api/internal/sync"),
        "expected semantic call inside nested function: {:?}",
        ir.semantic_edges
    );
}

#[test]
fn php_interface_extends_queue_events_and_sql_enclosing_symbol() {
    let php = r#"<?php
namespace App\Events;

interface BaseEvent {}
interface Auditable {}
interface PaymentEvent extends BaseEvent, Auditable {}

class OrderDispatcher {
    public function publishOrder() {
        $bus->enqueue('order.queued');
        broadcast(new OrderCompleted());
        DB::table('orders')->where('id', 1)->get();
        return "SELECT * FROM invoices WHERE total > 0";
    }
}

class OrderHandler {
    public function handle(OrderCompleted $event) {}
}

#[AsMessageHandler]
class AsyncHandler {
    public function handle(PaymentEvent $msg) {}
}
"#;
    let ir = parse_file_ir("app/Events/OrderEvents.php", php).expect("php ir");
    assert!(
        ir.inheritance.iter().any(|inh| {
            inh.child == "PaymentEvent" && inh.parent == "BaseEvent" && inh.kind == "extends"
        }),
        "expected PHP interface extends (not implements): {:?}",
        ir.inheritance
    );
    assert!(
        !ir.inheritance
            .iter()
            .any(|inh| inh.child == "PaymentEvent" && inh.kind == "implements"),
        "PHP interface must not emit implements: {:?}",
        ir.inheritance
    );
    assert!(
        ir.semantic_edges
            .iter()
            .any(|e| e.kind == "publishes_event" && e.target == "event:order.queued"),
        "expected enqueue publishes_event: {:?}",
        ir.semantic_edges
    );
    assert!(
        ir.semantic_edges
            .iter()
            .any(|e| e.kind == "publishes_event" && e.target == "event:OrderCompleted"),
        "expected broadcast publishes_event: {:?}",
        ir.semantic_edges
    );
    assert!(
        ir.semantic_edges.iter().any(|e| {
            e.kind == "handles_event"
                && e.target == "event:OrderCompleted"
                && e.source_symbol.as_deref() == Some("OrderHandler::handle")
        }),
        "expected typed PHP handle() listener: {:?}",
        ir.semantic_edges
    );
    assert!(
        ir.semantic_edges.iter().any(|e| {
            e.kind == "handles_event"
                && e.target == "event:PaymentEvent"
                && e.source_symbol.as_deref() == Some("AsyncHandler")
        }),
        "expected class-level AsMessageHandler with handle(): {:?}",
        ir.semantic_edges
    );
    assert!(
        ir.semantic_edges.iter().any(|e| {
            e.kind == "reads_table"
                && e.target == "table:invoices"
                && e.source_symbol.as_deref() == Some("OrderDispatcher::publishOrder")
        }),
        "expected SQL string edge anchored to enclosing symbol: {:?}",
        ir.semantic_edges
    );
    assert!(ir.identifiers.contains("OrderCompleted"));
    assert!(!ir.identifiers.contains("invoices"));

    let ts = r#"
export async function syncEvents(broker: any) {
    await broker.produce('billing.settled');
    broker.consume('billing.refund');
}
"#;
    let ts_ir = parse_file_ir("src/events.ts", ts).expect("ts ir");
    assert!(
        ts_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "publishes_event" && e.target == "event:billing.settled"),
        "expected produce publishes_event: {:?}",
        ts_ir.semantic_edges
    );
    assert!(
        ts_ir
            .semantic_edges
            .iter()
            .any(|e| e.kind == "handles_event" && e.target == "event:billing.refund"),
        "expected consume handles_event: {:?}",
        ts_ir.semantic_edges
    );
}

#[test]
fn parses_blade_templates_directives_components_livewire_and_routes() {
    let blade = r#"{{-- @include('ignored.comment') --}}
@extends('layouts.app')
@inject('metrics', 'App\Services\MetricsService')
@include('partials.header')
@includeWhen($show, 'partials.banner')
@component('components.modal')
    <x-forms.input name="email" />
    <x-slot:footer>ignored slot</x-slot:footer>
    @livewire('users.profile-card')
    <livewire:orders.summary-table />
    <a href="{{ route('orders.show') }}">View</a>
    <form action="{{ action([App\Http\Controllers\OrderController::class, 'store']) }}">
        {{ App\Helpers\Format::currency($total) }}
    </form>
@endcomponent
@php
    $svc = new \App\Services\QuoteService();
    $svc->calculate();
@endphp
"#;
    let ir = parse_file_ir("resources/views/orders/show.blade.php", blade).expect("blade ir");
    assert_eq!(ir.language, "blade");
    let imp_paths: Vec<&str> = ir.imports.iter().map(|i| i.path.as_str()).collect();
    assert!(
        imp_paths.contains(&"blade-view:layouts.app"),
        "{imp_paths:?}"
    );
    assert!(
        imp_paths.contains(&"blade-view:partials.header"),
        "{imp_paths:?}"
    );
    assert!(
        imp_paths.contains(&"blade-view:partials.banner"),
        "{imp_paths:?}"
    );
    assert!(
        imp_paths.contains(&"blade-view:components.modal"),
        "{imp_paths:?}"
    );
    assert!(
        imp_paths.contains(&"blade-component:forms.input"),
        "{imp_paths:?}"
    );
    assert!(
        !imp_paths.iter().any(|p| p.contains("slot")),
        "{imp_paths:?}"
    );
    assert!(
        imp_paths.contains(&"blade-livewire:users.profile-card"),
        "{imp_paths:?}"
    );
    assert!(
        imp_paths.contains(&"blade-livewire:orders.summary-table"),
        "{imp_paths:?}"
    );
    assert!(
        imp_paths.contains(&"App\\Services\\MetricsService"),
        "{imp_paths:?}"
    );
    assert!(
        !imp_paths.contains(&"blade-view:ignored.comment"),
        "{imp_paths:?}"
    );

    assert!(
        ir.semantic_edges
            .iter()
            .any(|e| e.kind == "calls_route" && e.target == "route:NAME orders.show"),
        "expected route() semantic edge: {:?}",
        ir.semantic_edges
    );
    assert!(
        ir.calls_and_refs
            .iter()
            .any(|r| r.target_name.contains("OrderController")
                && r.member.as_deref() == Some("store")),
        "expected action() controller ref: {:?}",
        ir.calls_and_refs
    );
    assert!(
        ir.calls_and_refs
            .iter()
            .any(|r| r.target_name.contains("Format") && r.member.as_deref() == Some("currency")),
        "expected static call ref: {:?}",
        ir.calls_and_refs
    );
    assert!(
        ir.calls_and_refs
            .iter()
            .any(|r| r.target_name.contains("QuoteService")
                && r.member.as_deref() == Some("calculate")),
        "expected @php block ref: {:?}",
        ir.calls_and_refs
    );
}

#[test]
fn route_prefix_composition_symfony_laravel_and_express() {
    // Symfony class-level #[Route('/api/v2')] composes with method #[Route('/orders/{id}')],
    // without polluting inner Route::get static calls or outgoing calls_route edges.
    let symfony_src = r#"<?php
namespace App\Controller;

use Symfony\Component\Routing\Attribute\Route;

#[Route('/api/v2')]
class OrderController {
    #[Route('/orders/{id}', methods: ['GET'])]
    public function show(): void {}

    #[Route('health')]
    public function health(): void {}
}
"#;
    let sym_ir = parse_file_ir("src/Controller/OrderController.php", symfony_src)
        .expect("parse symfony controller");
    let sym_routes: Vec<&str> = sym_ir
        .semantic_edges
        .iter()
        .filter(|e| e.kind == "exposes_route")
        .map(|e| e.target.as_str())
        .collect();
    assert!(
        sym_routes
            .iter()
            .any(|t| t.ends_with(" /api/v2/orders/{param}")),
        "expected composed Symfony class+method route: {sym_routes:?}"
    );
    assert!(
        sym_routes.iter().any(|t| t.ends_with(" /api/v2/health")),
        "expected relative method route composed with class prefix: {sym_routes:?}"
    );

    // Laravel nested Route::prefix(...)->group(...) + Route::group(['prefix' => ...]).
    let laravel_src = r#"<?php
Route::prefix('/api')->group(function () {
    Route::prefix('v1')->group(function () {
        Route::get('/invoices', [InvoiceController::class, 'index']);
    });
    Route::group(['prefix' => 'admin'], function () {
        Route::post('/audit', [AuditController::class, 'store']);
    });
});
"#;
    let lar_ir = parse_file_ir("routes/api.php", laravel_src).expect("parse laravel routes");
    let lar_routes: Vec<&str> = lar_ir
        .semantic_edges
        .iter()
        .filter(|e| e.kind == "exposes_route")
        .map(|e| e.target.as_str())
        .collect();
    assert!(
        lar_routes.contains(&"route:GET /api/v1/invoices"),
        "expected nested Route::prefix composition: {lar_routes:?}"
    );
    assert!(
        lar_routes.contains(&"route:POST /api/admin/audit"),
        "expected Route::group prefix composition: {lar_routes:?}"
    );

    // Express router.use('/prefix', ...) inline + mounted router + chained .route(...).get(...).
    let express_src = r#"
import express from 'express';
const app = express();
const users = express.Router();
users.get('/:id', showUser);
users.route('/profile').get(getProfile).post(updateProfile);
app.use('/api/users', users);
app.use('/api/v1', (req, res, next) => {
  const sub = express.Router();
  sub.get('/ping', pingHandler);
});
"#;
    let exp_ir = parse_file_ir("src/server.ts", express_src).expect("parse express routes");
    let exp_routes: Vec<&str> = exp_ir
        .semantic_edges
        .iter()
        .filter(|e| e.kind == "exposes_route")
        .map(|e| e.target.as_str())
        .collect();
    assert!(
        exp_routes.contains(&"route:GET /api/users/{param}"),
        "expected mounted router GET route: {exp_routes:?}"
    );
    assert!(
        exp_routes.contains(&"route:GET /api/users/profile")
            && exp_routes.contains(&"route:POST /api/users/profile"),
        "expected chained .route().get().post() composition: {exp_routes:?}"
    );
    assert!(
        exp_routes.contains(&"route:GET /api/v1/ping"),
        "expected inline app.use('/api/v1', ...) callback composition: {exp_routes:?}"
    );
}

#[test]
fn extracts_blade_inline_php_expressions_and_di_container_resolutions() {
    let blade = r#"
<div>{{ blade_inline_helper($order->total) }}</div>
<div>{!! app()->make(\App\Services\HtmlSanitizer::class)->clean($body) !!}</div>
<script>const raw = @{{ IgnoredClientExpr::run() }};</script>
{{-- {{ IgnoredComment::skip() }} --}}
"#;
    let blade_ir = parse_file_ir("resources/views/orders/receipt.blade.php", blade)
        .expect("parse blade inline expressions");
    assert!(
        blade_ir
            .calls_and_refs
            .iter()
            .any(|r| r.target_name == "blade_inline_helper"),
        "expected inline {{ ... }} PHP call ref: {:?}",
        blade_ir.calls_and_refs
    );
    assert!(
        blade_ir
            .calls_and_refs
            .iter()
            .any(|r| r.target_name.contains("HtmlSanitizer")),
        "expected inline {{!! app()->make(...) !!}} DI resolution: {:?}",
        blade_ir.calls_and_refs
    );
    assert!(
        !blade_ir
            .calls_and_refs
            .iter()
            .any(|r| r.target_name.contains("IgnoredClientExpr")
                || r.target_name.contains("IgnoredComment")),
        "escaped @{{ }} and Blade comment {{----}} must stay ignored: {:?}",
        blade_ir.calls_and_refs
    );

    let php_di = r#"<?php
namespace App\Services;

use App\Contracts\PaymentGateway;
use App\Support\Ledger;
use App\Support\Mailer;
use App\Support\Metrics;
use Psr\Container\ContainerInterface;

class CheckoutHandler {
    public function handle(ContainerInterface $container): void {
        $gw = app()->make(PaymentGateway::class);
        $ledger = Container::get(Ledger::class);
        $mailer = resolve(Mailer::class);
        $metrics = app(Metrics::class);
    }
}
"#;
    let di_ir = parse_file_ir("app/Services/CheckoutHandler.php", php_di).expect("parse php DI");
    let refs: Vec<&str> = di_ir
        .calls_and_refs
        .iter()
        .map(|r| r.target_name.as_str())
        .collect();
    for expected in ["PaymentGateway", "Ledger", "Mailer", "Metrics"] {
        assert!(
            refs.iter().any(|r| r.contains(expected)),
            "expected DI reference to {expected} in {refs:?}"
        );
    }
    let di_binds: Vec<(Option<&str>, &str)> = di_ir
        .semantic_edges
        .iter()
        .filter(|e| e.kind == "di_binds")
        .map(|e| (e.source_symbol.as_deref(), e.target.as_str()))
        .collect();
    assert!(
        di_binds.iter().any(|(_, t)| t.contains("PaymentGateway")),
        "expected di_binds semantic edge for app()->make: {di_binds:?}"
    );
}

#[test]
fn route_di_blade_escaping_nested_routers_and_named_php_attributes_regression() {
    // 1. Express app.get('/users', UserController) and app.get('port') must not emit DI bindings/resolutions,
    // while NestJS app.get(UsersService) still works. Also multi-level router propagation must emit /api/u/profile.
    let ts = r#"
const uRouter = express.Router();
uRouter.get('/profile', getProfile);
const v1Router = express.Router();
v1Router.use('/u', uRouter);
app.use('/api', v1Router);

app.get('/users', UserController);
const port = app.get('port');
const svc = app.get(UsersService);
"#;
    let ts_ir = parse_file_ir("src/routes/multi.ts", ts).expect("parse multi router");
    let ts_di: Vec<&str> = ts_ir
        .semantic_edges
        .iter()
        .filter(|e| e.kind == "di_binds")
        .map(|e| e.target.as_str())
        .collect();
    assert!(
        !ts_di.iter().any(|t| *t == "service:/users"
            || *t == "service:port"
            || *t == "service:UserController"),
        "Express route/config app.get must not emit false DI bindings: {ts_di:?}"
    );
    assert!(
        !ts_ir
            .calls_and_refs
            .iter()
            .any(|r| r.context == "di_resolve" && r.target_name == "UserController"),
        "Express handler UserController must not become di_resolve: {:?}",
        ts_ir.calls_and_refs
    );
    assert!(
        ts_di.contains(&"service:UsersService"),
        "typed container app.get(UsersService) should still emit DI: {ts_di:?}"
    );
    let ts_routes: Vec<&str> = ts_ir
        .semantic_edges
        .iter()
        .filter(|e| e.kind == "exposes_route")
        .map(|e| e.target.as_str())
        .collect();
    assert!(
        ts_routes.contains(&"route:GET /api/u/profile"),
        "expected 2-level mounted router route GET /api/u/profile: {ts_routes:?}"
    );
    assert!(
        ts_routes.contains(&"route:GET /users"),
        "expected GET /users route: {ts_routes:?}"
    );

    // 2. Blade mask_blade_comments handles quoted ' }}', nested {a:{b:1}}, and @@{{ / @@{!! live PHP echoes.
    let blade = r#"
<div x-data="@{{ { quoted: ' }}', nested: { fn: IgnoredEscaped::skip() } } }}"></div>
<div>@{!! { raw: '!!}', call: IgnoredRawEsc::skip() } !!}</div>
<div>@@{{ LiveEcho::run() }}</div>
<div>@@{!! LiveRaw::exec() !!}</div>
"#;
    let blade_ir = parse_file_ir("resources/views/escaped.blade.php", blade)
        .expect("parse blade escaping edge cases");
    let blade_refs: Vec<&str> = blade_ir
        .calls_and_refs
        .iter()
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        !blade_refs
            .iter()
            .any(|r| r.contains("IgnoredEscaped") || r.contains("IgnoredRawEsc")),
        "nested/quoted @{{ }} and @{{!! !!}} must remain masked: {blade_refs:?}"
    );
    assert!(
        blade_refs.iter().any(|r| r.contains("LiveEcho")),
        "@@{{ }} must remain live PHP: {blade_refs:?}"
    );
    assert!(
        blade_refs.iter().any(|r| r.contains("LiveRaw")),
        "@@{{!! !!}} must remain live PHP: {blade_refs:?}"
    );

    // 3. Symfony PHP named #[Route(name: ..., path: ...)] attributes extract `path:`, not `name:`.
    let php = r#"<?php
namespace App\Controller;

use Symfony\Component\Routing\Annotation\Route;

#[Route(name: 'admin_', path: '/admin')]
class AdminController {
    #[Route(name: 'users_list', path: '/users', methods: ['GET'])]
    public function list(): void {}
}
"#;
    let php_ir =
        parse_file_ir("src/Controller/AdminController.php", php).expect("parse php named route");
    let php_routes: Vec<&str> = php_ir
        .semantic_edges
        .iter()
        .filter(|e| e.kind == "exposes_route")
        .map(|e| e.target.as_str())
        .collect();
    assert!(
        php_routes.contains(&"route:* /admin/users"),
        "expected composed named Symfony Route /admin/users: {php_routes:?}"
    );
    assert!(
        !php_routes
            .iter()
            .any(|r| r.contains("admin_") || r.contains("users_list")),
        "attribute name: arg must never be mistaken for route path: {php_routes:?}"
    );
}
