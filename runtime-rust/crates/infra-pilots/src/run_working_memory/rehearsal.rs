//! Fixed mechanics scenario. Its command worker does not model real-agent efficacy.
use super::*;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

pub const SIGNAL: &str = "Refund consumers must deduplicate by refund_id";
pub struct Scenario {
    pub directory: PathBuf,
    pub workspace: PathBuf,
    pub run: PathBuf,
    pub state: Value,
    pub plan: Value,
    pub step: Value,
}
impl Scenario {
    pub fn create(directory: &Path) -> Result<Self> {
        if directory.exists() && std::fs::read_dir(directory)?.next().is_some() {
            bail!("pilot_output_not_empty")
        }
        native::private_directory(directory)?;
        let directory = infra_core::fs::canonicalize(directory)?;
        let workspace = directory.join("workspace");
        let system = workspace.join("system");
        let orders = workspace.join("orders");
        let run = directory.join("run");
        for root in [&system, &orders, &run] {
            native::private_directory(root)?;
        }
        std::fs::write(
            orders.join("contract.txt"),
            "refund.completed: refund_id, order_id\n",
        )?;
        std::fs::write(
            system.join("AGENTS.md"),
            "Reopen canonical sources and verify worker claims.\n",
        )?;
        std::fs::write(system.join("system.json"), "{}\n")?;
        let first = json!({"id":"contracts","service":"__system__","mode":"read-only","status":"completed","attempt":1,"dispatch_id":"b".repeat(32),"input_sha256":"c".repeat(64)});
        let step = json!({"id":"service-orders","service":"orders","mode":"read-only","status":"pending","attempt":1,"dispatch_id":"d".repeat(32),"input_sha256":"e".repeat(64)});
        let state = json!({"run_id":"a".repeat(32),"system_file":common::path_string(&system.join("system.json"))?,"status":"running","provider":"command","mode":"read-only","access":"service","selected":["orders"],"roots":{"__system__":common::path_string(&system)?,"orders":common::path_string(&orders)?},"tasks":{"__system__":{"external_id":"ai-system/run/system"},"orders":{"external_id":"ai-system/run/orders"}},"steps":[first,step]});
        let plan = json!({"context":{"task":"Investigate refund consumers without repeating discovery","services":["orders"],"sources":[{"service":"orders","path":"contract.txt","kind":"contract","excerpt":"refund.completed: refund_id, order_id"}]},"relationships":[],"warnings":[],"omitted_sources":[]});
        let scenario = Self {
            directory,
            workspace,
            run,
            state,
            plan,
            step,
        };
        scenario.receipt(&first,&format!("{SIGNAL}. Open question: who owns the retry policy? Hypothesis: retries may arrive twice."))?;
        common::save(
            &scenario.directory,
            "scenario.json",
            &json!({"state":scenario.state,"plan":scenario.plan,"step":scenario.step}),
            true,
        )?;
        Ok(scenario)
    }
    pub fn reopen(directory: &Path) -> Result<Self> {
        let directory = infra_core::fs::canonicalize(directory)?;
        let value = common::read_json(&directory, "scenario.json", common::MAX_BYTES)?;
        Ok(Self {
            workspace: directory.join("workspace"),
            run: directory.join("run"),
            directory,
            state: value["state"].clone(),
            plan: value["plan"].clone(),
            step: value["step"].clone(),
        })
    }
    pub fn receipt(&self, step: &Value, summary: &str) -> Result<()> {
        let report = json!({"status":"completed","summary":summary,"checks":[{"name":"contract inspected","status":"passed","detail":"Fixture source has refund_id."}],"changed_files":[],"service_order":if step["id"]=="contracts"{json!(["orders"])}else{json!([])}});
        let receipt = json!({"dispatch_id":step["dispatch_id"],"phase":step["id"],"input_sha256":step["input_sha256"],"mode":step["mode"],"returncode":0,"error":null,"duration_seconds":0.0,"ok":true,"report":report});
        common::save(&self.run, &common::receipt_name(step), &receipt, true)?;
        Ok(())
    }
    pub fn observations(&self, memory: &RunWorkingMemory) -> Result<Vec<Observation>> {
        let source = memory.citation("orders", "contract.txt")?;
        Ok(vec![
            Observation {
                key: "refund-deduplication".into(),
                kind: Kind::Progress,
                status: Status::Open,
                text: SIGNAL.into(),
                sources: vec![source.clone()],
            },
            Observation {
                key: "retry-owner".into(),
                kind: Kind::OpenQuestion,
                status: Status::Open,
                text: "Who owns the retry policy?".into(),
                sources: vec![source.clone()],
            },
            Observation {
                key: "duplicate-retries".into(),
                kind: Kind::Hypothesis,
                status: Status::Open,
                text: "Retries may arrive twice; this remains a hypothesis.".into(),
                sources: vec![source],
            },
        ])
    }
    pub fn memory(&self, store: Box<dyn SnapshotStore>) -> Result<RunWorkingMemory> {
        RunWorkingMemory::open(&self.run, &self.state, &self.workspace, 1, store)
    }
    pub fn prior(&self) -> Result<Vec<Value>> {
        Ok(execution::reports(&self.run, &self.state)?)
    }
}

pub fn consumer() -> Result<()> {
    let mut raw = vec![];
    std::io::stdin().take(1_048_577).read_to_end(&mut raw)?;
    if raw.len() > 1_048_576 {
        bail!("pilot_input_bound")
    }
    let prompt = std::str::from_utf8(&raw)?;
    let payload: Value = serde_json::from_str(prompt.rsplit_once('\n').context("pilot_prompt")?.1)?;
    let shared = &payload["shared_context"];
    let in_reports = shared["reports"].as_array().is_some_and(|reports| {
        reports
            .iter()
            .any(|report| common::string(&report["summary"]).contains(SIGNAL))
    });
    let entries = shared["working_memory"]["entries"].as_array();
    let in_snapshot =
        entries.is_some_and(|entries| entries.iter().any(|entry| entry["text"] == SIGNAL));
    let report = json!({"status":"completed","summary":format!("Fixture signal in reports: {in_reports}; in working-memory view: {in_snapshot}. This is signal transfer, not model efficacy."),"checks":[{"name":"known observation available","status":if in_reports||in_snapshot{"passed"}else{"failed"},"detail":"The scripted consumer checks both existing reports and the experimental view."}],"changed_files":[],"service_order":[]});
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}

pub fn trial(
    scenario: &Scenario,
    redis_endpoint: Option<std::net::SocketAddr>,
    recovery: bool,
    rounds: usize,
) -> Result<Value> {
    if rounds == 0 || rounds > 1000 {
        bail!("pilot_rounds")
    }
    let prior = scenario.prior()?;
    let first = scenario.state["steps"][0].clone();
    let mut local = scenario.memory(Box::new(SqliteStore::open(
        &scenario.directory.join("sqlite-view"),
    )?))?;
    let observations = scenario.observations(&local)?;
    let accepted = local.append(
        &scenario.state,
        &first,
        &observations,
        Duration::from_secs(3600),
    )?;
    let mut stores = vec![("B", local)];
    if let Some(endpoint) = redis_endpoint {
        #[cfg(feature = "run-memory-redis")]
        {
            let mut remote = scenario.memory(Box::new(RedisStore::loopback(endpoint)?))?;
            let replay = remote.append(
                &scenario.state,
                &first,
                &observations,
                Duration::from_secs(3600),
            )?;
            if !replay.replayed {
                bail!("pilot_replay")
            }
            stores.push(("C", remote));
        }
        #[cfg(not(feature = "run-memory-redis"))]
        {
            let _ = endpoint;
            bail!("enable_run_memory_redis_feature")
        }
    }
    let baseline = native::prompt_for(
        &scenario.state,
        &scenario.plan,
        &scenario.step,
        &prior,
        None,
    )?;
    let mut inputs = vec![("A", baseline, "disabled")];
    let mut snapshots = vec![];
    for (name, memory) in &mut stores {
        let (prompt, read) =
            memory.prompt(&scenario.state, &scenario.plan, &scenario.step, &prior)?;
        snapshots.push(read.snapshot);
        inputs.push((*name, prompt, read.backend));
    }
    if snapshots.len() == 2 && snapshots[0] != snapshots[1] {
        bail!("pilot_snapshot_parity")
    }
    if inputs.len() == 3 && inputs[1].1 != inputs[2].1 {
        bail!("pilot_prompt_parity")
    }
    let mut results = vec![];
    for (arm, prompt, backend) in &inputs {
        let mut child = Command::new(std::env::current_exe()?)
            .arg("--consumer")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .context("pilot_stdin")?
            .write_all(prompt.as_bytes())?;
        let output = child.wait_with_output()?;
        if !output.status.success() {
            bail!("pilot_consumer_failed")
        }
        let report = common::parse_json(&output.stdout, common::MAX_BYTES)?;
        execution::validate_report(
            &report,
            &scenario.step,
            &common::texts(&scenario.state["selected"]),
            execution::access_scope(&scenario.state),
        )?;
        if report["checks"][0]["status"] != "passed" {
            bail!("pilot_signal_missing")
        }
        let sha = common::digest(prompt.as_bytes());
        let prefix = if recovery { "recovery-" } else { "" };
        common::save(
            &scenario.directory,
            &format!("{prefix}{arm}.input.json"),
            &json!({"input_sha256":sha,"prompt":prompt}),
            true,
        )?;
        common::save(
            &scenario.directory,
            &format!("{prefix}{arm}.receipt.json"),
            &json!({"input_sha256":sha,"report":report}),
            true,
        )?;
        results.push(json!({"arm":arm,"prompt_bytes":prompt.len(),"prompt_chars":prompt.chars().count(),"input_sha256":sha,"backend":backend,"scripted_report":report}));
    }
    let mut timing = BTreeMap::<&str, Vec<f64>>::new();
    for round in 0..rounds {
        let order = if round % 2 == 0 {
            (0..inputs.len()).collect::<Vec<_>>()
        } else {
            (0..inputs.len()).rev().collect()
        };
        for i in order {
            let name = inputs[i].0;
            let start = std::time::Instant::now();
            if name == "A" {
                native::prompt_for(
                    &scenario.state,
                    &scenario.plan,
                    &scenario.step,
                    &prior,
                    None,
                )?;
            } else {
                stores
                    .iter_mut()
                    .find(|(arm, _)| *arm == name)
                    .context("pilot_arm")?
                    .1
                    .prompt(&scenario.state, &scenario.plan, &scenario.step, &prior)?;
            }
            timing
                .entry(name)
                .or_default()
                .push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    Ok(
        json!({"scope":"deterministic mechanics/parity; no live agents or task-outcome efficacy","default_enabled":false,"journal_accepted":accepted.accepted,"journal_replayed":accepted.replayed,"arms":results,"BC_snapshot_equal":if snapshots.len()==2{Some(snapshots[0]==snapshots[1])}else{None},"BC_prompt_equal":if inputs.len()==3{Some(inputs[1].1==inputs[2].1)}else{None},"prompt_build_ms":timing,"rounds":rounds,"recovery":recovery,"adoption":"awaits sealed held-out task-outcome evidence"}),
    )
}
