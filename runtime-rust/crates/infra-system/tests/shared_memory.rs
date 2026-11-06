//! The memory of a system and of every service of a change is shared by all
//! of its agents: a plan offers each participant's Memory Bank chunks to the
//! one context every agent receives (memory no person reviewed marked as
//! such) and counts those it could not share, and a chunk that stops holding
//! during a run leaves that context instead of stopping the run. Chunks are
//! published through the Memory Bank's own promotion, so they cite the Brain
//! record they came from.
use infra_core::{BankService, BrainRepository, CreateRecord, Roots, RuntimeContext, UpdateRecord};
use infra_system::catalog::System;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static SERIAL: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "infra-system-memory-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(infra_core::fs::canonicalize(path).unwrap())
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn write(root: &Path, name: &str, bytes: impl AsRef<[u8]>) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// `orders` publishes `order.placed`, `billing` consumes it.
fn fixture(temp: &Temp) -> (PathBuf, Vec<PathBuf>) {
    let root = &temp.0;
    write(
        root,
        "system/system.json",
        serde_json::to_vec_pretty(
            &json!({"schema_version":1,"name":"Fixture system","services":[
            {"id":"orders","root":"../services/orders","manifest":"ai-service.json"},
            {"id":"billing","root":"../services/billing","manifest":"ai-service.json"}],
            "shared_sources":[{"path":"AGENTS.md","kind":"policy"}]}),
        )
        .unwrap(),
    );
    write(
        root,
        "system/AGENTS.md",
        "Coordinate contract changes first.\n",
    );
    for (sid, consumes, provides) in [
        (
            "orders",
            json!([]),
            json!([{"id":"order.placed","kind":"event","version":"1","sources":["contracts/events.yaml"]}]),
        ),
        (
            "billing",
            json!([{"service":"orders","contract":"order.placed","version":"1"}]),
            json!([]),
        ),
    ] {
        let service = root.join("services").join(sid);
        write(
            &service,
            "ai-service.json",
            serde_json::to_vec_pretty(&json!({"schema_version":1,"id":sid,"description":format!("{sid} fixture service"),
                "owner":"Fixture team","relationships_complete":true,
                "capabilities":[{"id":"work","description":"Fixture work","status":"implemented","sources":["src/work.txt"]}],
                "provides":provides,"consumes":consumes,
                "sources":[{"path":"AGENTS.md","kind":"policy"}]}))
            .unwrap(),
        );
        write(&service, "AGENTS.md", format!("{sid} policy\n"));
        write(&service, "src/work.txt", "before\n");
        write(&service, "contracts/events.yaml", "order.placed: v1\n");
    }
    (root.join("system/system.json"), vec![root.join("services")])
}
fn context(root: &Path, actor: &str) -> RuntimeContext {
    RuntimeContext::new(
        Roots::installed(root.to_path_buf()).unwrap(),
        actor,
        "governed",
    )
    .unwrap()
}
/// A resolved finding a person verified, citing `sources`.
fn finding(root: &Path, title: &str, content: &str, sources: &[&str]) -> String {
    let brain = BrainRepository::new(context(root, "author")).unwrap();
    let record = brain
        .create(CreateRecord {
            record_type: "finding".into(),
            external_id: format!("MEMORY-{}", SERIAL.fetch_add(1, Ordering::Relaxed)),
            title: title.into(),
            goal: Some(title.into()),
            sources: sources.iter().map(|source| source.to_string()).collect(),
            authority: "verified".into(),
            attestation: Some("person".into()),
            ..CreateRecord::default()
        })
        .unwrap();
    let id = record["id"].as_str().unwrap().to_string();
    brain
        .update(
            &id,
            UpdateRecord {
                expected_revision: record["revision"].as_u64(),
                progress: Some(content.into()),
                transition_to: Some("resolved".into()),
                reason: "Checked against the code".into(),
                attestation: Some("person".into()),
                ..Default::default()
            },
        )
        .unwrap();
    id
}
/// A chunk published after another person approved it.
fn reviewed_memory(root: &Path, title: &str, content: &str, sources: &[&str]) -> String {
    let id = finding(root, title, content, sources);
    let proposal = BankService::new(context(root, "author"))
        .propose(&[id], title, content, false)
        .unwrap();
    let reviewer = BankService::new(context(root, "reviewer"));
    let promotion = proposal["id"].as_str().unwrap();
    reviewer.review(promotion, Some("reviewer"), true).unwrap();
    let applied = reviewer.apply(promotion).unwrap();
    chunk_path(root, applied["destination_memory_id"].as_str().unwrap())
}
/// A chunk published by automatic promotion, which no person reviewed.
fn automatic_memory(root: &Path, title: &str, content: &str, sources: &[&str]) -> String {
    write(
        root,
        "project-brain/config/runtime.json",
        serde_json::to_vec_pretty(&json!({"automatic_promotion":true,
            "allowed_privacy":["public","team"],"allowed_authority":["observed","verified"]}))
        .unwrap(),
    );
    let id = finding(root, title, content, sources);
    let promoted = BankService::new(context(root, "author"))
        .auto_promote(5, None, Some(&[id]))
        .unwrap();
    let memory = promoted["promoted"][0]
        .as_str()
        .or_else(|| promoted["promoted"][0]["memory_id"].as_str())
        .unwrap_or_else(|| panic!("nothing promoted: {promoted:#}"))
        .to_string();
    chunk_path(root, &memory)
}
fn chunk_path(root: &Path, memory: &str) -> String {
    let name = fs::read_dir(root.join("memory-bank/chunks"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .find(|name| name.starts_with(memory))
        .unwrap_or_else(|| panic!("no chunk for {memory}"));
    format!("memory-bank/chunks/{name}")
}
fn shared_memory(plan: &Value) -> Vec<(String, String)> {
    plan["context"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|source| source["kind"] == "memory")
        .map(|source| {
            (
                source["service"].as_str().unwrap().to_string(),
                source["path"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn every_participant_shares_its_memory_and_memory_that_no_longer_holds_is_counted() {
    let temp = Temp::new();
    let (config, allow) = fixture(&temp);
    let system_memory = reviewed_memory(
        &temp.0.join("system"),
        "Order events change through the topic registry",
        "A change to order.placed is agreed in the system folder before any service changes.",
        &["AGENTS.md"],
    );
    let billing_memory = reviewed_memory(
        &temp.0.join("services/billing"),
        "Billing charges each placed order once",
        "Billing deduplicates order.placed deliveries by order id before charging.",
        &["src/work.txt"],
    );
    let orders_memory = automatic_memory(
        &temp.0.join("services/orders"),
        "Orders publish after commit",
        "Orders publish order.placed only after the order row commits.",
        &["src/work.txt"],
    );
    // A chunk whose cited contract changed since no longer holds.
    reviewed_memory(
        &temp.0.join("services/orders"),
        "Order events stay at version one",
        "order.placed stays at version 1 while billing reads it.",
        &["contracts/events.yaml"],
    );
    write(
        &temp.0.join("services/orders"),
        "contracts/events.yaml",
        "order.placed: v2\n",
    );
    let plan = System::new(&config, &allow)
        .unwrap()
        .plan(
            "Bill every placed order",
            "bill-orders",
            &["orders".into()],
            &[],
            16000,
            32,
        )
        .unwrap();
    let trust = |sid: &str, path: &str| {
        plan["context"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|source| source["service"] == sid && source["path"] == path)
            .map(|source| source["trust"].as_str().unwrap().to_string())
    };
    assert_eq!(
        trust("__system__", &system_memory).as_deref(),
        Some("reviewed_memory")
    );
    assert_eq!(
        trust("billing", &billing_memory).as_deref(),
        Some("reviewed_memory")
    );
    // Promoted without a person's review, as Harness sessions save memory:
    // shared, and marked so agents check it.
    assert_eq!(
        trust("orders", &orders_memory).as_deref(),
        Some("automatic_memory")
    );
    assert_eq!(shared_memory(&plan).len(), 3, "{:?}", shared_memory(&plan));
    // The stale chunk is counted, not named.
    assert!(
        plan["warnings"].as_array().unwrap().contains(
            &json!({"service":"orders","reason":"memory_not_shared","count":1,"causes":{"ineligible":1}})
        ),
        "{:#}",
        plan["warnings"]
    );
    assert!(
        !plan["omitted_sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["path"].as_str().unwrap().starts_with("memory-bank/")),
        "{:#}",
        plan["omitted_sources"]
    );
}

#[test]
fn memory_that_stops_holding_leaves_the_shared_context() {
    let temp = Temp::new();
    let (config, allow) = fixture(&temp);
    let billing_memory = reviewed_memory(
        &temp.0.join("services/billing"),
        "Billing charges each placed order once",
        "Billing deduplicates order.placed deliveries by order id before charging.",
        &["src/work.txt"],
    );
    let mut system = System::new(&config, &allow).unwrap();
    let plan = system
        .plan(
            "Bill every placed order",
            "bill-orders",
            &["orders".into()],
            &[],
            16000,
            32,
        )
        .unwrap();
    assert!(shared_memory(&plan).contains(&("billing".into(), billing_memory.clone())));
    // The billing agent changes the source the memory describes.
    write(&temp.0.join("services/billing"), "src/work.txt", "after\n");
    let mut current = System::new(&config, &allow).unwrap();
    let verdict = current.verify(&plan).unwrap();
    assert!(
        verdict["changed"].as_array().unwrap().contains(
            &json!({"service":"billing","path":billing_memory,"reason":"memory_ineligible"})
        ),
        "{verdict:#}"
    );
    let accepted = BTreeSet::from([("billing".to_string(), "src/work.txt".to_string())]);
    let updated = current.refresh_checkpoint(&plan, &accepted).unwrap();
    assert!(shared_memory(&updated).is_empty());
    assert!(updated["omitted_sources"].as_array().unwrap().contains(
        &json!({"service":"billing","path":billing_memory,"reason":"memory_no_longer_holds"})
    ));
    assert_eq!(
        System::new(&config, &allow)
            .unwrap()
            .verify(&updated)
            .unwrap()["fresh"],
        true
    );
}
