//! Folders each dispatch is granted beyond its working folder. A read-only
//! Claude dispatch also reads the run's own receipts, which its prompt names.
use infra_system::execution::extra_directories;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn state(provider: &str, mode: &str) -> Value {
    json!({"provider":provider,"mode":mode,"access":"all","selected":["orders","billing"],
        "roots":{"__system__":"/work/system","orders":"/work/orders","billing":"/work/billing"}})
}
fn step(id: &str, service: &str, mode: &str) -> Value {
    json!({"id":id,"service":service,"mode":mode})
}

#[test]
fn a_read_only_claude_dispatch_may_read_the_runs_receipts() {
    let run = Path::new("/state/ai-system/run/execution");
    let verify = extra_directories(
        &state("claude", "edit"),
        &step("verify", "__system__", "read-only"),
        run,
    );
    assert!(verify.contains(&run.to_path_buf()), "{verify:?}");
    // An edit dispatch's added folders are writable, so the run stays out.
    let edit = extra_directories(
        &state("claude", "edit"),
        &step("service-billing", "billing", "edit"),
        run,
    );
    assert!(!edit.contains(&run.to_path_buf()), "{edit:?}");
    assert!(edit.contains(&PathBuf::from("/work/orders")), "{edit:?}");
    // Codex reads the whole disk already and is granted only writable roots.
    let codex = extra_directories(
        &state("codex", "read-only"),
        &step("verify", "__system__", "read-only"),
        run,
    );
    assert!(codex.is_empty(), "{codex:?}");
}
