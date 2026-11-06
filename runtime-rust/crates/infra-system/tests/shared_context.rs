//! Every agent of a system run shares one context: it reads the system folder
//! and every service of the change, and receives the complete reports of the
//! agents before it. The access scope decides only where a service agent
//! writes. A later agent used to see 1000 characters of each earlier summary
//! and ten checks, and an own-service agent only its own folder.
use infra_system::{
    execution::{dispatch_access, extra_directories, read_directories},
    native::{PROMPT_CHARS, claude_read_rule, prompt_for},
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn state(provider: &str, access: &str) -> Value {
    json!({"provider":provider,"mode":"edit","access":access,"selected":["orders","billing"],
        "roots":{"__system__":"/work/system","orders":"/work/orders","billing":"/work/billing"},
        "tasks":{"__system__":{"external_id":"ai-system/run/system"},"orders":{"external_id":"ai-system/run/orders"},"billing":{"external_id":"ai-system/run/billing"}}})
}
fn step(id: &str, service: &str, mode: &str) -> Value {
    json!({"id":id,"service":service,"mode":mode,"dispatch_id":"d".repeat(32)})
}
fn plan(excerpt: &str) -> Value {
    json!({"context":{"task":"Bill every placed order","services":["billing","orders"],
        "sources":[{"service":"orders","path":"contracts/events.yaml","kind":"contract","excerpt":excerpt}]},
        "relationships":[{"provider":"orders","consumer":"billing","contract":"order.placed"}],
        "warnings":[],"omitted_sources":[]})
}
/// A complete earlier report as `execution::reports` hands it on.
fn report(phase: &str, summary: usize, checks: usize, detail: usize) -> Value {
    json!({"phase":phase,"service":"__system__","attempt":1,"status":"completed",
        "summary":"s".repeat(summary),
        "checks":(0..checks).map(|n|json!({"name":format!("check-{n}"),"status":if n%2==0{"passed"}else{"not_run"},"detail":"d".repeat(detail)})).collect::<Vec<_>>(),
        "changed_files":[],"receipt":format!("/state/run/{phase}-a1.json")})
}
fn payload(prompt: &str) -> Value {
    serde_json::from_str(prompt.rsplit_once('\n').unwrap().1).unwrap()
}

#[test]
fn every_agent_reads_the_whole_change_and_writes_only_its_scope() {
    let every = json!(["__system__", "orders", "billing"]);
    for (access, writable) in [
        ("service", json!(["billing"])),
        ("all", json!(["orders", "billing"])),
    ] {
        let state = state("codex", access);
        let service = dispatch_access(&state, &step("service-billing", "billing", "edit"));
        assert_eq!(service["readable"], every, "{access}");
        assert_eq!(service["writable"], writable, "{access}");
        let investigating =
            dispatch_access(&state, &step("service-billing", "billing", "read-only"));
        assert_eq!(investigating["readable"], every);
        assert_eq!(investigating["writable"], json!([]));
        for phase in ["contracts", "verify"] {
            let coordination = dispatch_access(&state, &step(phase, "__system__", "read-only"));
            assert_eq!(coordination["readable"], every);
            assert_eq!(coordination["writable"], json!([]));
        }
    }
}

#[test]
fn a_claude_edit_agent_reads_the_other_roots_without_making_them_writable() {
    let run = Path::new("/state/ai-system/run/execution");
    let own = state("claude", "service");
    let edit = step("service-billing", "billing", "edit");
    // An added folder is writable under acceptEdits, so it adds none...
    assert!(extra_directories(&own, &edit, run).is_empty());
    // ...and reads the system folder, its neighbour and the receipts by rule.
    assert_eq!(
        read_directories(&own, &edit, run),
        [
            PathBuf::from("/work/system"),
            PathBuf::from("/work/orders"),
            run.to_path_buf()
        ]
    );
    let all = state("claude", "all");
    assert_eq!(
        extra_directories(&all, &edit, run),
        [PathBuf::from("/work/orders")]
    );
    assert_eq!(
        read_directories(&all, &edit, run),
        [PathBuf::from("/work/system"), run.to_path_buf()]
    );
    // Plan mode keeps added folders read-only: a read-only agent adds them all.
    let verify = step("verify", "__system__", "read-only");
    assert_eq!(
        extra_directories(&own, &verify, run),
        [
            PathBuf::from("/work/orders"),
            PathBuf::from("/work/billing"),
            run.to_path_buf()
        ]
    );
    assert!(read_directories(&own, &verify, run).is_empty());
    // Codex reads the whole disk already.
    assert!(read_directories(&state("codex", "service"), &edit, run).is_empty());
    assert_eq!(
        claude_read_rule(Path::new("/work/system")).unwrap(),
        "Read(//work/system/**)"
    );
    assert_eq!(
        claude_read_rule(Path::new(r"C:\work\system")).unwrap(),
        "Read(//c/work/system/**)"
    );
}

#[test]
fn every_agent_receives_the_same_shared_context_and_its_own_assignment() {
    let state = state("codex", "service");
    let plan = plan("order.placed: v1");
    let prior = vec![report("contracts", 1500, 12, 300)];
    let billing = payload(
        &prompt_for(
            &state,
            &plan,
            &step("service-billing", "billing", "edit"),
            &prior,
            None,
        )
        .unwrap(),
    );
    let verify = payload(
        &prompt_for(
            &state,
            &plan,
            &step("verify", "__system__", "read-only"),
            &prior,
            None,
        )
        .unwrap(),
    );
    assert_eq!(billing["shared_context"], verify["shared_context"]);
    let shared = &billing["shared_context"];
    assert_eq!(shared["task"], "Bill every placed order");
    assert_eq!(shared["roots"], state["roots"]);
    assert_eq!(shared["relationships"], plan["relationships"]);
    assert_eq!(shared["sources"], plan["context"]["sources"]);
    // The earlier report goes whole: no 1000-character summary, no ten checks.
    assert_eq!(shared["reports"], json!(prior));
    assert_eq!(shared["reports_condensed"], false);
    assert_eq!(billing["phase"], "service-billing");
    assert_eq!(billing["access"]["writable"], json!(["billing"]));
    assert_eq!(billing["native_task"], "ai-system/run/billing");
    assert_eq!(verify["access"]["writable"], json!([]));
    assert_ne!(billing["goal"], verify["goal"]);
}

#[test]
fn earlier_reports_are_shortened_only_when_they_would_not_fit() {
    let state = state("codex", "service");
    let small = plan("order.placed: v1");
    let edit = step("verify", "__system__", "read-only");
    // Ten long reports fit once their summaries and check details are cut.
    let prior = (0..10)
        .map(|n| report(&format!("service-{n}"), 4000, 20, 1000))
        .collect::<Vec<_>>();
    let prompt = prompt_for(&state, &small, &edit, &prior, None).unwrap();
    assert!(prompt.chars().count() <= PROMPT_CHARS);
    let shared = payload(&prompt)["shared_context"].clone();
    assert_eq!(shared["reports_condensed"], true);
    let first = &shared["reports"][0];
    assert_eq!(first["summary"].as_str().unwrap().chars().count(), 1000);
    assert_eq!(first["checks"].as_array().unwrap().len(), 20);
    assert_eq!(
        first["checks"][0]["detail"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        200
    );
    assert_eq!(first["receipt"], "/state/run/service-0-a1.json");
    // Forty reports of a hundred checks keep only counts.
    let prior = (0..40)
        .map(|n| report(&format!("service-{n}"), 4000, 100, 1000))
        .collect::<Vec<_>>();
    let prompt = prompt_for(&state, &small, &edit, &prior, None).unwrap();
    assert!(prompt.chars().count() <= PROMPT_CHARS);
    let first = payload(&prompt)["shared_context"]["reports"][0].clone();
    assert_eq!(first["check_counts"], json!({"not_run":50,"passed":50}));
    assert!(first.get("checks").is_none());
    assert_eq!(first["summary"].as_str().unwrap().chars().count(), 300);
    assert_eq!(first["receipt"], "/state/run/service-0-a1.json");
    // A plan whose own context cannot fit still asks to be narrowed.
    let error = prompt_for(&state, &plan(&"x".repeat(PROMPT_CHARS)), &edit, &[], None)
        .unwrap_err()
        .message;
    assert_eq!(
        error,
        "Worker prompt exceeds 128000 characters; narrow plan"
    );
}
