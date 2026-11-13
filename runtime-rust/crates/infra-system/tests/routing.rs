//! Capability routing reads domain terms, not prose: a task written as a
//! sentence must not make every service with a prose description a candidate.
use infra_system::catalog::System;
use serde_json::{Value, json};
use std::{fs, path::Path, path::PathBuf};

struct Root(PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn write_json(path: &Path, value: &Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
fn system() -> (Root, System) {
    let root = std::env::temp_dir().join(format!("infra-system-routing-{}", uuid::Uuid::new_v4()));
    infra_core::fs::ensure_directory(&root, true).unwrap();
    let root = Root(infra_core::fs::canonicalize(root).unwrap());
    let services = [
        (
            "catalog",
            "Lists the products and the prices that vendors set for them.",
            json!(["listing"]),
        ),
        (
            "orders",
            "Places orders and cancels them before payment capture.",
            json!(["checkout"]),
        ),
        (
            "payments",
            "Captures the payment for an order and executes refunds.",
            json!(["refund", "psp"]),
        ),
    ];
    write_json(
        &root.0.join("system.json"),
        &json!({"schema_version":1,"name":"Routing","shared_sources":[],
            "services":services.iter().map(|(sid,_,_)|json!({"id":sid,"root":format!("services/{sid}"),"manifest":"ai-service.json"})).collect::<Vec<_>>()}),
    );
    for (sid, description, keywords) in services {
        write_json(
            &root.0.join(format!("services/{sid}/ai-service.json")),
            &json!({"schema_version":1,"id":sid,"description":description,"owner":"Team","relationships_complete":true,
                "capabilities":[{"id":format!("{sid}-core"),"description":description,"status":"implemented","sources":[],"keywords":keywords}],
                "provides":[],"consumes":[],"sources":[]}),
        );
    }
    let system = System::new(&root.0.join("system.json"), &[]).unwrap();
    (root, system)
}
fn services(located: &Value) -> Vec<String> {
    located["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|candidate| candidate["service"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn function_words_do_not_make_every_service_a_candidate() {
    let (_root, system) = system();
    let located = system
        .locate("Allow the customer to cancel an order and get the refund for it")
        .unwrap();
    let found = services(&located);
    assert!(!found.contains(&"catalog".to_string()), "{located:#}");
    assert_eq!(found, ["orders", "payments"], "{located:#}");
    for candidate in located["candidates"].as_array().unwrap() {
        for matched in candidate["matches"].as_array().unwrap() {
            for term in matched["terms"].as_array().unwrap() {
                assert!(
                    !["the", "and", "for", "get"].contains(&term.as_str().unwrap()),
                    "function word matched: {located:#}"
                );
            }
        }
    }
}

#[test]
fn plurals_match_their_singular_and_candidates_rank_by_matched_terms() {
    let (_root, system) = system();
    let located = system.locate("Refunds through the PSP").unwrap();
    assert_eq!(services(&located)[0], "payments", "{located:#}");
    assert_eq!(located["selection"], "suggested", "{located:#}");
    let located = system
        .locate("refund payments for captured orders")
        .unwrap();
    let ranked = services(&located);
    assert_eq!(ranked[0], "payments", "{located:#}");
    assert!(
        located["candidates"][0]["score"].as_u64() >= located["candidates"][1]["score"].as_u64(),
        "{located:#}"
    );
    assert_eq!(located["selection"], "needs_selection");
}

#[test]
fn a_task_of_only_function_words_matches_nothing() {
    let (_root, system) = system();
    let located = system.locate("and the for with from that").unwrap();
    assert!(services(&located).is_empty(), "{located:#}");
    assert_eq!(located["selection"], "needs_selection");
}

#[test]
fn a_tight_context_budget_keeps_the_sources_the_task_is_about() {
    let root =
        std::env::temp_dir().join(format!("infra-system-relevance-{}", uuid::Uuid::new_v4()));
    infra_core::fs::ensure_directory(&root, true).unwrap();
    let root = Root(infra_core::fs::canonicalize(root).unwrap());
    write_json(
        &root.0.join("system.json"),
        &json!({"schema_version":1,"name":"Relevance","shared_sources":[],
            "services":[{"id":"orders","root":"services/orders","manifest":"ai-service.json"}]}),
    );
    let service = root.0.join("services/orders");
    write_json(
        &service.join("ai-service.json"),
        &json!({"schema_version":1,"id":"orders","description":"Orders","owner":"Team","relationships_complete":true,
            "capabilities":[
                {"id":"placement","description":"Place an order","status":"implemented","sources":["src/AuditTrail.php"]},
                {"id":"cancellation","description":"Customers cancel orders","status":"partial","sources":["src/CancelOrder.php"]}],
            "provides":[],"consumes":[],"sources":[{"path":"AGENTS.md","kind":"policy"}]}),
    );
    fs::create_dir_all(service.join("src")).unwrap();
    fs::write(service.join("AGENTS.md"), "Orders policy.\n").unwrap();
    fs::write(service.join("src/AuditTrail.php"), "<?php // audit trail\n").unwrap();
    fs::write(
        service.join("src/CancelOrder.php"),
        "<?php // cancel order\n",
    )
    .unwrap();
    let mut system = System::new(&root.0.join("system.json"), &[]).unwrap();
    let plan = system
        .plan(
            "Let customers cancel a confirmed order",
            "cancel",
            &["orders".into()],
            &[],
            600,
            32,
        )
        .unwrap();
    let delivered = plan["context"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|source| source["path"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(delivered, ["AGENTS.md", "src/CancelOrder.php"], "{plan:#}");
    assert_eq!(
        plan["omitted_sources"][0]["path"], "src/AuditTrail.php",
        "{plan:#}"
    );
    assert_eq!(plan["omitted_sources"][0]["reason"], "budget", "{plan:#}");
}
