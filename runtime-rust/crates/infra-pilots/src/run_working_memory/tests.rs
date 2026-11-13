use super::rehearsal::Scenario;
use super::*;
use std::sync::{Arc, Mutex};
fn fixture() -> (tempfile::TempDir, Scenario) {
    let temp = tempfile::tempdir().unwrap();
    let scenario = Scenario::create(temp.path()).unwrap();
    (temp, scenario)
}
fn local(s: &Scenario) -> RunWorkingMemory {
    s.memory(Box::new(
        SqliteStore::open(&s.directory.join("cache")).unwrap(),
    ))
    .unwrap()
}
fn append(memory: &mut RunWorkingMemory, s: &Scenario) {
    memory
        .append(
            &s.state,
            &s.state["steps"][0],
            &s.observations(memory).unwrap(),
            Duration::from_secs(60),
        )
        .unwrap();
}
fn payload(prompt: &str) -> Value {
    serde_json::from_str(prompt.rsplit_once('\n').unwrap().1).unwrap()
}
struct Unavailable;
impl SnapshotStore for Unavailable {
    fn load(&mut self, _: &str) -> Result<Option<Vec<u8>>> {
        bail!("unavailable")
    }
    fn publish(&mut self, _: &str, _: &[u8], _: Duration) -> Result<()> {
        bail!("unavailable")
    }
}
struct Poison {
    writes: Arc<Mutex<usize>>,
}
impl SnapshotStore for Poison {
    fn load(&mut self, _: &str) -> Result<Option<Vec<u8>>> {
        Ok(Some(
            b"forged generation, omitted and replaced claims".to_vec(),
        ))
    }
    fn publish(&mut self, _: &str, _: &[u8], _: Duration) -> Result<()> {
        *self.writes.lock().unwrap() += 1;
        Ok(())
    }
}

#[test]
fn unchanged_control_keeps_reports_and_opt_in_view_reaches_next_dispatch() {
    let (_temp, s) = fixture();
    let mut m = local(&s);
    append(&mut m, &s);
    let prior = s.prior().unwrap();
    let a = native::prompt_for(&s.state, &s.plan, &s.step, &prior, None).unwrap();
    let (b, read) = m.prompt(&s.state, &s.plan, &s.step, &prior).unwrap();
    let a = payload(&a);
    let b = payload(&b);
    assert!(a["shared_context"].get("working_memory").is_none());
    assert_eq!(
        a["shared_context"]["reports"],
        b["shared_context"]["reports"]
    );
    assert!(
        a["shared_context"]["reports"][0]["summary"]
            .as_str()
            .unwrap()
            .contains(rehearsal::SIGNAL)
    );
    assert_eq!(read.snapshot["entries"].as_array().unwrap().len(), 3);
    assert_eq!(read.snapshot["untrusted"], true);
    assert!(
        read.snapshot["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["kind"] == "hypothesis")
    );
}
#[test]
fn journal_acceptance_survives_backend_failure_and_reopen() {
    let (_temp, s) = fixture();
    let before = common::read_file(&s.run, "contracts-a1.json", common::MAX_BYTES).unwrap();
    let mut m = s.memory(Box::new(Unavailable)).unwrap();
    let result = m
        .append(
            &s.state,
            &s.state["steps"][0],
            &s.observations(&m).unwrap(),
            Duration::from_secs(60),
        )
        .unwrap();
    assert!(result.accepted);
    assert_eq!(result.projection, "local_fallback");
    drop(m);
    let mut restored = local(&s);
    assert_eq!(
        restored.snapshot(&s.state).unwrap().snapshot["entries"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        before,
        common::read_file(&s.run, "contracts-a1.json", common::MAX_BYTES).unwrap()
    );
}
#[test]
fn identical_replay_keeps_expiry_and_conflicting_replay_is_refused() {
    let (_temp, s) = fixture();
    let mut m = local(&s);
    let input = s.observations(&m).unwrap();
    let at = now_ms();
    let first = m
        .append_at(
            &s.state,
            &s.state["steps"][0],
            &input,
            Duration::from_secs(60),
            at,
        )
        .unwrap();
    assert!(!first.replayed);
    let before = m.snapshot_at(&s.state, at).unwrap().snapshot;
    let replay = m
        .append_at(
            &s.state,
            &s.state["steps"][0],
            &input,
            Duration::from_secs(60),
            at + 500,
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(before, m.snapshot_at(&s.state, at + 500).unwrap().snapshot);
    let mut changed = input;
    changed[0].text = "Changed claim".into();
    assert!(
        m.append(
            &s.state,
            &s.state["steps"][0],
            &changed,
            Duration::from_secs(60)
        )
        .is_err()
    );
    assert_eq!(m.journal().unwrap().records.len(), 1);
}
#[test]
fn scope_and_generation_cannot_adopt_another_journal() {
    let (_temp, s) = fixture();
    let mut m = local(&s);
    append(&mut m, &s);
    let mut other = s.state.clone();
    other["run_id"] = json!("f".repeat(32));
    assert!(m.snapshot(&other).is_err());
    assert!(
        RunWorkingMemory::open(&s.run, &s.state, &s.workspace, 2, Box::new(Unavailable)).is_err()
    );
    let second = s.directory.join("other-run");
    native::private_directory(&second).unwrap();
    let mut isolated = RunWorkingMemory::open(
        &second,
        &other,
        &s.workspace,
        1,
        Box::new(SqliteStore::open(&s.directory.join("cache")).unwrap()),
    )
    .unwrap();
    assert!(
        isolated.snapshot(&other).unwrap().snapshot["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn narrowing_selected_services_refuses_the_old_view_even_when_roots_remain() {
    let (_temp, s) = fixture();
    let mut m = local(&s);
    append(&mut m, &s);
    let mut narrowed = s.state.clone();
    narrowed["selected"] = json!([]);
    assert!(m.snapshot(&narrowed).is_err());
    assert!(
        m.prompt(&narrowed, &s.plan, &s.step, &s.prior().unwrap())
            .is_err()
    );
    let snapshot = m.snapshot(&s.state).unwrap().snapshot;
    assert!(
        native::prompt_for_with_working_memory(
            &narrowed,
            &s.plan,
            &s.step,
            &s.prior().unwrap(),
            None,
            &snapshot
        )
        .is_err()
    );
}
#[test]
fn changed_sources_and_retried_dispatches_cannot_reuse_old_view() {
    let (_temp, s) = fixture();
    let mut m = local(&s);
    append(&mut m, &s);
    let before = m.snapshot(&s.state).unwrap();
    std::fs::write(
        s.workspace.join("orders/contract.txt"),
        "refund.completed: new shape\n",
    )
    .unwrap();
    let current = m.snapshot(&s.state).unwrap();
    assert!(current.snapshot["entries"].as_array().unwrap().is_empty());
    assert_ne!(before.key, current.key);
    let mut retried = s.state.clone();
    retried["steps"][0]["attempt"] = json!(2);
    retried["steps"][0]["dispatch_id"] = json!("9".repeat(32));
    assert!(
        m.snapshot(&retried).unwrap().snapshot["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn changed_canonical_receipt_invalidates_its_linked_claims() {
    let (_temp, s) = fixture();
    let mut m = local(&s);
    append(&mut m, &s);
    let mut receipt = common::read_json(&s.run, "contracts-a1.json", common::MAX_BYTES).unwrap();
    receipt["report"]["summary"] = json!("Receipt was edited");
    common::save(&s.run, "contracts-a1.json", &receipt, false).unwrap();
    assert!(
        m.snapshot(&s.state).unwrap().snapshot["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn invalid_secret_and_out_of_scope_citations_never_enter_journal() {
    let (_temp, s) = fixture();
    let mut m = local(&s);
    let mut observations = s.observations(&m).unwrap();
    observations[0].sources[0].path = "../outside.txt".into();
    assert!(
        m.append(
            &s.state,
            &s.state["steps"][0],
            &observations,
            Duration::from_secs(60)
        )
        .is_err()
    );
    assert!(m.citation("other", "contract.txt").is_err());
    assert!(m.citation("orders", ".env").is_err());
    std::fs::write(
        s.workspace.join("orders/secret.txt"),
        format!("api_key={}\n", "z".repeat(32)),
    )
    .unwrap();
    assert!(m.citation("orders", "secret.txt").is_err());
    assert!(m.journal().unwrap().records.is_empty());
}
#[cfg(unix)]
#[test]
fn escaping_links_and_replaced_roots_are_refused() {
    let (_temp, s) = fixture();
    let mut m = local(&s);
    append(&mut m, &s);
    std::fs::write(s.directory.join("outside.txt"), "outside\n").unwrap();
    std::os::unix::fs::symlink(
        s.directory.join("outside.txt"),
        s.workspace.join("orders/link.txt"),
    )
    .unwrap();
    assert!(m.citation("orders", "link.txt").is_err());
    std::fs::rename(
        s.workspace.join("orders"),
        s.workspace.join("previous-orders"),
    )
    .unwrap();
    std::fs::create_dir(s.workspace.join("orders")).unwrap();
    assert!(m.snapshot(&s.state).is_err());
}
fn resolve_question(s: &mut Scenario) -> Value {
    let mut service = s.state["steps"][1].clone();
    service["status"] = json!("completed");
    s.receipt(&service, "Service inspected").unwrap();
    s.state["steps"][1] = service;
    let step = json!({"id":"verify","service":"__system__","mode":"read-only","status":"completed","attempt":1,"dispatch_id":"7".repeat(32),"input_sha256":"8".repeat(64)});
    s.receipt(
        &step,
        "Retry ownership was resolved by the worker; this is still a claim.",
    )
    .unwrap();
    s.state["steps"].as_array_mut().unwrap().push(step.clone());
    step
}
#[test]
fn expired_resolution_never_resurrects_the_old_open_question() {
    let (_temp, mut s) = fixture();
    let mut m = local(&s);
    let at = now_ms();
    let original = s.observations(&m).unwrap();
    m.append_at(
        &s.state,
        &s.state["steps"][0],
        &original,
        Duration::from_secs(60),
        at,
    )
    .unwrap();
    let step = resolve_question(&mut s);
    let mut resolved = original[1].clone();
    resolved.status = Status::Resolved;
    resolved.text = "Orders owns retry policy (worker claim).".into();
    m.append_at(
        &s.state,
        &step,
        &[resolved],
        Duration::from_millis(1),
        at + 1,
    )
    .unwrap();
    let read = m.snapshot_at(&s.state, at + 1).unwrap();
    assert!(
        read.snapshot["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["key"] == "retry-owner" && entry["status"] == "resolved")
    );
    let read = m.snapshot_at(&s.state, at + 3).unwrap();
    assert!(
        !read.snapshot["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["key"] == "retry-owner")
    );
    assert!(
        m.snapshot_at(&s.state, at + 60_001).unwrap().snapshot["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    s.state["status"] = json!("completed");
    assert!(
        m.snapshot_at(&s.state, at + 1).unwrap().snapshot["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn backend_poison_is_rebuilt_from_the_exact_allowed_claims() {
    let (_temp, s) = fixture();
    let writes = Arc::new(Mutex::new(0));
    let mut m = s
        .memory(Box::new(Poison {
            writes: writes.clone(),
        }))
        .unwrap();
    append(&mut m, &s);
    let read = m.snapshot(&s.state).unwrap();
    assert_eq!(read.backend, "rebuilt");
    assert_eq!(read.snapshot["entries"].as_array().unwrap().len(), 3);
    assert!(*writes.lock().unwrap() > 0);
}
#[test]
fn snapshot_bound_and_real_prompt_condensation_apply_together() {
    let (_temp, s) = fixture();
    let mut m = local(&s);
    let source = m.citation("orders", "contract.txt").unwrap();
    let observations = (0..12)
        .map(|n| Observation {
            key: format!("topic-{n}"),
            kind: Kind::Hypothesis,
            status: Status::Open,
            text: "hypothesis ".repeat(29),
            sources: vec![source.clone()],
        })
        .collect::<Vec<_>>();
    m.append(
        &s.state,
        &s.state["steps"][0],
        &observations,
        Duration::from_secs(60),
    )
    .unwrap();
    let snapshot = m.snapshot(&s.state).unwrap().snapshot;
    assert!(wire(&snapshot).unwrap().len() <= SNAPSHOT_BYTES);
    assert!(snapshot["omitted"].as_u64().unwrap() > 0);
    let prior=(0..40).map(|n|json!({"phase":format!("phase-{n}"),"service":"__system__","summary":"s".repeat(4000),"checks":(0..100).map(|_|json!({"name":"check","status":"passed","detail":"d".repeat(1000)})).collect::<Vec<_>>(),"changed_files":[],"receipt":"receipt.json"})).collect::<Vec<_>>();
    let prompt =
        native::prompt_for_with_working_memory(&s.state, &s.plan, &s.step, &prior, None, &snapshot)
            .unwrap();
    assert!(prompt.chars().count() <= native::PROMPT_CHARS);
    assert_eq!(
        payload(&prompt)["shared_context"]["reports_condensed"],
        true
    );
    let mut other = snapshot.clone();
    other["run_id"] = json!("wrong-run");
    assert!(
        native::prompt_for_with_working_memory(&s.state, &s.plan, &s.step, &prior, None, &other)
            .is_err()
    );
}
#[test]
fn sqlite_view_expiry_is_not_journal_expiry() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = SqliteStore::open(temp.path()).unwrap();
    store
        .publish("key", b"value", Duration::from_secs(60))
        .unwrap();
    assert_eq!(store.load("key").unwrap(), Some(b"value".to_vec()));
    store
        .connection
        .execute("UPDATE snapshots SET expires_ms=0", [])
        .unwrap();
    assert_eq!(store.load("key").unwrap(), None);
}
#[cfg(feature = "run-memory-redis")]
#[test]
fn redis_pilot_rejects_nonloopback_and_zero_port() {
    assert!(RedisStore::loopback("0.0.0.0:6379".parse().unwrap()).is_err());
    assert!(RedisStore::loopback("127.0.0.1:0".parse().unwrap()).is_err());
}
#[cfg(feature = "run-memory-redis")]
#[test]
#[ignore = "requires explicitly supplied private INFRA_RUN_MEMORY_REDIS_ENDPOINT"]
fn redis_live_matches_sqlite_and_rebuilds_expired_and_corrupt_views() {
    let endpoint = std::env::var("INFRA_RUN_MEMORY_REDIS_ENDPOINT")
        .expect("private endpoint required")
        .parse()
        .unwrap();
    let (_temp, s) = fixture();
    let mut b = local(&s);
    append(&mut b, &s);
    let mut c = s
        .memory(Box::new(RedisStore::loopback(endpoint).unwrap()))
        .unwrap();
    let expected = b.snapshot(&s.state).unwrap().snapshot;
    let actual = c.snapshot(&s.state).unwrap();
    assert_eq!(expected, actual.snapshot);
    assert_ne!(actual.backend, "local_fallback");
    let mut writer = RedisStore::loopback(endpoint).unwrap();
    writer
        .publish(&actual.key, b"forged", Duration::from_secs(60))
        .unwrap();
    assert_eq!(c.snapshot(&s.state).unwrap().backend, "rebuilt");
    writer
        .publish(
            &actual.key,
            &wire(&expected).unwrap(),
            Duration::from_millis(1),
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(10));
    assert_eq!(c.snapshot(&s.state).unwrap().backend, "rebuilt");
    let bp = b
        .prompt(&s.state, &s.plan, &s.step, &s.prior().unwrap())
        .unwrap()
        .0;
    let cp = c
        .prompt(&s.state, &s.plan, &s.step, &s.prior().unwrap())
        .unwrap()
        .0;
    assert_eq!(bp, cp);
}

#[test]
fn late_first_delivery_of_an_old_dispatch_cannot_reopen_a_resolved_topic() {
    let (_temp, mut s) = fixture();
    let mut m = local(&s);
    let original = s.observations(&m).unwrap();
    let step = resolve_question(&mut s);
    let mut resolved = original[1].clone();
    resolved.status = Status::Resolved;
    resolved.text = "Orders owns retry policy (worker claim).".into();
    m.append(&s.state, &step, &[resolved], Duration::from_secs(60))
        .unwrap();
    m.append(
        &s.state,
        &s.state["steps"][0],
        &original,
        Duration::from_secs(60),
    )
    .unwrap();
    let read = m.snapshot(&s.state).unwrap();
    let question = read.snapshot["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["key"] == "retry-owner")
        .unwrap();
    assert_eq!(question["status"], "resolved");
}
#[test]
fn nested_memory_and_brain_paths_are_not_canonical_citations() {
    let (_temp, s) = fixture();
    let m = local(&s);
    for path in [
        "nested/memory-bank/chunks/item.md",
        "nested/project-brain/records/item.md",
    ] {
        let file = s.workspace.join("orders").join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "Not an admitted canonical source.\n").unwrap();
        assert!(m.citation("orders", path).is_err());
    }
}
#[test]
fn secrets_in_topic_keys_are_refused_before_journal_acceptance() {
    let (_temp, s) = fixture();
    let mut m = local(&s);
    let mut observations = s.observations(&m).unwrap();
    for secret in [
        format!("sk-{}", "z".repeat(24)),
        format!("sk-{}", "Z".repeat(24)),
        format!("ghp_{}", "Z".repeat(24)),
    ] {
        observations[0].key = secret.clone();
        let error = m
            .append(
                &s.state,
                &s.state["steps"][0],
                &observations,
                Duration::from_secs(60),
            )
            .unwrap_err();
        assert!(!error.to_string().contains(&secret));
    }
    assert!(m.journal().unwrap().records.is_empty());
}

#[test]
fn citation_validation_errors_do_not_disclose_sensitive_paths() {
    let (_temp, s) = fixture();
    let m = local(&s);
    for secret in [
        format!("sk-{}", "Z".repeat(24)),
        format!("ghp_{}", "Z".repeat(24)),
    ] {
        let error = m.citation("orders", &format!("../{secret}")).unwrap_err();
        assert!(!format!("{error:#}").contains(&secret));
    }
}

#[test]
fn corrupt_journal_schema_errors_do_not_disclose_values() {
    let (_temp, s) = fixture();
    let mut m = local(&s);
    append(&mut m, &s);
    let path = m.directory.join(JOURNAL);
    let original: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let secret = format!("ghp_{}", "Z".repeat(24));
    for unknown_field in [false, true] {
        let mut corrupted = original.clone();
        let observation = &mut corrupted["records"][0]["observations"][0];
        if unknown_field {
            observation[&secret] = json!(true);
        } else {
            observation["kind"] = json!(secret);
        }
        std::fs::write(&path, wire(&corrupted).unwrap()).unwrap();
        let error = m.snapshot(&s.state).unwrap_err();
        assert!(!format!("{error:#}").contains(&secret));
    }
}

fn producer_receipt(s: &Scenario, suffix: &str) {
    let name = common::receipt_name(&s.state["steps"][0]);
    let mut receipt = common::read_json(&s.run, &name, common::MAX_BYTES).unwrap();
    receipt["report"]["summary"] = json!(format!(
        "Actual source inspected.{}{}",
        live::MARKER,
        suffix
    ));
    common::save(&s.run, &name, &receipt, false).unwrap();
}
#[test]
fn real_run_identity_needs_no_extra_project_field() {
    let (_temp, s) = fixture();
    assert!(s.state.get("project_id").is_none());
    let mut memory = local(&s);
    let read = memory.snapshot(&s.state).unwrap();
    assert_eq!(
        read.snapshot["project_id"],
        native::working_memory_project_id(&s.state).unwrap()
    );
    assert!(
        native::prompt_for_with_working_memory(
            &s.state,
            &s.plan,
            &s.step,
            &[],
            None,
            &read.snapshot
        )
        .is_ok()
    );
}
#[test]
fn live_receipt_producer_reaches_next_prompt_and_reopen_preserves_expiry() {
    use infra_system::runner::DispatchContext;
    let (_temp, s) = fixture();
    let memory = local(&s);
    let observations = s.observations(&memory).unwrap();
    producer_receipt(&s, &serde_json::to_string(&observations).unwrap());
    let mut context = live::LiveContext::new(memory, true);
    let prompt = context
        .prompt(&s.state, &s.plan, &s.step, &s.prior().unwrap(), None)
        .unwrap();
    assert!(prompt.contains(live::PRODUCER_INSTRUCTION));
    assert_eq!(
        payload(&prompt)["shared_context"]["reports"],
        json!(s.prior().unwrap())
    );
    assert_eq!(
        payload(&prompt)["shared_context"]["working_memory"]["entries"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(context.imports[0]["status"], "accepted");
    assert_eq!(
        context.deliveries[0]["input_sha256"],
        strict::sha(prompt.as_bytes())
    );
    let before = wire(&context.memory.journal().unwrap()).unwrap();
    drop(context);
    let mut reopened = live::LiveContext::new(local(&s), false);
    let prompt = reopened
        .prompt(&s.state, &s.plan, &s.step, &s.prior().unwrap(), None)
        .unwrap();
    assert!(
        payload(&prompt)["shared_context"]
            .get("working_memory")
            .is_none()
    );
    assert!(prompt.contains(live::PRODUCER_INSTRUCTION));
    assert_eq!(before, wire(&reopened.memory.journal().unwrap()).unwrap());
    assert_eq!(reopened.imports[0]["replayed"], true);
    reopened.reconcile(&s.state).unwrap();
    assert_eq!(reopened.imports.len(), 1);
}
#[test]
fn live_invalid_annotations_and_unfinished_dispatches_are_not_accepted() {
    for case in ["invalid-json", "stale-digest", "unfinished", "missing"] {
        let (_temp, mut s) = fixture();
        let memory = local(&s);
        let mut observations = s.observations(&memory).unwrap();
        if case == "stale-digest" {
            observations[0].sources[0].sha256 = "f".repeat(64);
        }
        if case != "missing" {
            let suffix = if case == "invalid-json" {
                "[malformed".into()
            } else {
                serde_json::to_string(&observations).unwrap()
            };
            producer_receipt(&s, &suffix);
        }
        if case == "unfinished" {
            s.state["steps"][0]["status"] = json!("running");
        }
        let mut context = live::LiveContext::new(memory, true);
        context.reconcile(&s.state).unwrap();
        assert!(
            context.memory.journal().unwrap().records.is_empty(),
            "{case}"
        );
        if case == "unfinished" {
            assert!(context.imports.is_empty());
        } else {
            assert_eq!(context.imports[0]["status"], "rejected", "{case}");
        }
    }
}
#[test]
fn provider_metrics_persist_only_allowlisted_numbers_and_command_hashes() {
    let secret = format!("ghp_{}", "Z".repeat(24));
    let usage = json!({"type":"output","phase":"contracts","line":json!({"type":"turn.completed","usage":{"input_tokens":42,"cached_input_tokens":10,"output_tokens":2,"secret":secret}}).to_string()});
    let metric = live::event_metric(&usage).unwrap();
    assert_eq!(
        metric["usage"],
        json!({"input_tokens":42,"cached_input_tokens":10,"output_tokens":2})
    );
    assert!(!metric.to_string().contains(&secret));
    let command = json!({"type":"output","phase":"contracts","line":json!({"type":"item.completed","item":{"type":"command_execution","command":secret,"aggregated_output":secret,"exit_code":0}}).to_string()});
    let metric = live::event_metric(&command).unwrap();
    assert_eq!(metric["command_sha256"], strict::sha(secret.as_bytes()));
    assert!(!metric.to_string().contains(&secret));
    assert!(live::event_metric(&json!({"type":"output","line":secret})).is_none());
}

#[test]
#[ignore = "requires explicit INFRA_RUN_MEMORY_ARCHIVED_COHORT containing all six real cases"]
fn real_archived_receipts_pass_the_final_privacy_gate() {
    let directory = PathBuf::from(
        std::env::var_os("INFRA_RUN_MEMORY_ARCHIVED_COHORT").expect("explicit cohort required"),
    );
    assert!(directory.is_absolute());
    let mut count = 0;
    for task in ["stock-customer", "refund-recipient"] {
        for arm in ["A", "B", "C"] {
            let run = directory.join(format!("{task}-{arm}/run"));
            let state = common::read_json(&run, "run.json", common::MAX_BYTES).unwrap();
            assert_eq!(state["status"], "completed");
            let mut seen = 0;
            for step in state["steps"].as_array().unwrap() {
                let receipt = execution::dispatch_receipt(&run, &state, step).unwrap();
                assert!(execution::completed_receipt(&receipt, step));
                // Exact new gate predicate, plus whole persisted envelope/error audit.
                assert!(
                    infra_core::domain::secret_label(&common::encoded(&receipt["report"]).unwrap())
                        .unwrap()
                        .is_none()
                );
                assert!(
                    infra_core::domain::secret_label(&common::encoded(&receipt).unwrap())
                        .unwrap()
                        .is_none()
                );
                execution::validate_report(
                    &receipt["report"],
                    step,
                    &common::texts(&state["selected"]),
                    execution::access_scope(&state),
                )
                .unwrap();
                seen += 1;
            }
            assert_eq!(seen, 4, "every real native dispatch must be represented");
            count += seen;
        }
    }
    assert_eq!(count, 24);
    eprintln!(
        "24 real archived native receipts pass the final pre-storage privacy gate and report contract"
    );
}
