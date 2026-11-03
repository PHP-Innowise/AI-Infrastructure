use crate::{
    Result, RuntimeError, common,
    platform_cli::{self, PlatformConfig},
    utils,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};
struct ProbeFile(std::path::PathBuf);
impl Drop for ProbeFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
#[derive(Default)]
pub struct TriggerDetector {
    pending: bool,
    accumulated: String,
    pub observed: bool,
}
impl TriggerDetector {
    pub fn feed(&mut self, event: &Value, name: &str) -> Option<bool> {
        match event["type"].as_str().unwrap_or("") {
            "stream_event" => {
                let inner = &event["event"];
                match inner["type"].as_str().unwrap_or("") {
                    "content_block_start" => {
                        let block = &inner["content_block"];
                        if block["type"] == "tool_use" {
                            if matches!(block["name"].as_str(), Some("Skill" | "Read")) {
                                self.pending = true;
                                self.accumulated.clear();
                            } else {
                                return Some(false);
                            }
                        }
                    }
                    "content_block_delta" if self.pending => {
                        if inner["delta"]["type"] == "input_json_delta" {
                            self.accumulated
                                .push_str(inner["delta"]["partial_json"].as_str().unwrap_or(""));
                            if self.accumulated.contains(name) {
                                self.observed = true;
                                return Some(true);
                            }
                        }
                    }
                    "content_block_stop" | "message_stop" => {
                        if self.pending {
                            return Some(self.accumulated.contains(name));
                        }
                        if inner["type"] == "message_stop" {
                            return Some(false);
                        }
                    }
                    _ => {}
                }
            }
            "assistant" => {
                if let Some(content) = event["message"]["content"].as_array() {
                    for item in content {
                        if item["type"] != "tool_use" {
                            continue;
                        }
                        let input = &item["input"];
                        let found = match item["name"].as_str() {
                            Some("Skill") => {
                                input["skill"].as_str().is_some_and(|s| s.contains(name))
                            }
                            Some("Read") => input["file_path"]
                                .as_str()
                                .is_some_and(|s| s.contains(name)),
                            _ => false,
                        };
                        self.observed = found;
                        return Some(found);
                    }
                }
            }
            "result" => return Some(self.observed),
            _ => {}
        }
        None
    }
}
#[expect(
    clippy::too_many_arguments,
    reason = "Preserve the public trigger-evaluation API with independently supplied platform, query, skill metadata, timeout, project, model and environment."
)]
pub fn run_single_query(
    config: &PlatformConfig,
    query: &str,
    name: &str,
    description: &str,
    timeout: Duration,
    project: &Path,
    model: Option<&str>,
    environment: &BTreeMap<String, String>,
) -> Result<bool> {
    if config.key != "claude" {
        return platform_cli::evaluate_trigger(
            config,
            query,
            name,
            description,
            timeout,
            project,
            model,
            environment,
        );
    }
    let unique = uuid::Uuid::new_v4().simple().to_string();
    let clean = format!("{name}-skill-{}", &unique[..8]);
    let directory = project.join(config.skills_path);
    std::fs::create_dir_all(&directory)?;
    let file = ProbeFile(directory.join(format!("{clean}.md")));
    let indented = description.split('\n').collect::<Vec<_>>().join("\n  ");
    let content = format!(
        "---\ndescription: |\n  {indented}\n---\n\n# {name}\n\nThis skill handles: {description}\n"
    );
    use std::io::Write;
    let mut writer = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&file.0)?;
    writer.write_all(content.as_bytes())?;
    drop(writer);
    let mut env = environment.clone();
    env.remove("CLAUDECODE");
    let executable = platform_cli::executable(config, &env)?;
    let mut argv = vec![
        executable,
        "-p".into(),
        query.into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--include-partial-messages".into(),
    ];
    if let Some(m) = model {
        argv.extend(["--model".into(), m.into()]);
    }
    let mut detector = TriggerDetector::default();
    let mut selected = None;
    let output = common::invoke(argv, project, env, vec![], timeout, |event| {
        if let Some(hit) = detector.feed(event, &clean) {
            selected = Some(hit);
            Some("probe-observed".into())
        } else {
            None
        }
    })?;
    if output.stopped == "interrupted" {
        return Err(RuntimeError::unavailable("Trigger evaluation interrupted"));
    }
    Ok(selected.unwrap_or(false))
}
#[derive(Clone)]
pub struct EvalOptions {
    pub config: PlatformConfig,
    pub workers: usize,
    pub timeout: Duration,
    pub runs: usize,
    pub threshold: f64,
    pub model: Option<String>,
}
pub fn run_eval(
    eval_set: &[Value],
    name: &str,
    description: &str,
    project: &Path,
    options: &EvalOptions,
    log: &mut String,
) -> Result<Value> {
    let native = options.config.key != "claude";
    if options.workers == 0 || options.runs == 0 {
        return Err(RuntimeError::invalid(
            "workers and runs_per_query must be positive",
        ));
    }
    if native && (!(0.0 < options.threshold && options.threshold <= 1.0) || eval_set.is_empty()) {
        return Err(RuntimeError::invalid(
            "Native evaluation requires a nonempty set and threshold greater than 0 and at most 1",
        ));
    }
    let mut query_order = vec![];
    let mut query_items = BTreeMap::new();
    let mut queue = VecDeque::new();
    for item in eval_set {
        let query = item["query"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("Every eval item requires a string query"))?;
        if !item["should_trigger"].is_boolean() || native && query.trim().is_empty() {
            return Err(RuntimeError::invalid(
                "Every eval item requires a non-empty query and boolean should_trigger",
            ));
        }
        if native && query_items.contains_key(query) {
            return Err(RuntimeError::invalid(format!(
                "Duplicate eval query: {query}"
            )));
        }
        if native {
            query_order.push(query.to_owned());
        }
        query_items.insert(query.to_owned(), item.clone());
        for _ in 0..options.runs {
            queue.push_back((query.to_owned(), item.clone()));
        }
    }
    if queue.len() > 1_000_000 {
        return Err(RuntimeError::invalid(
            "Evaluation exceeds the bounded job limit",
        ));
    }
    let queue = Arc::new(Mutex::new(queue));
    let (tx, rx) = mpsc::channel();
    let environment = common::env();
    let mut completed = vec![];
    std::thread::scope(|scope| {
        for _ in 0..options.workers.min(128) {
            let queue = queue.clone();
            let tx = tx.clone();
            let environment = environment.clone();
            scope.spawn(move || {
                loop {
                    if infra_eval::cancel::interrupted() {
                        break;
                    }
                    let job = queue.lock().unwrap().pop_front();
                    let Some((query, item)) = job else {
                        break;
                    };
                    let result = run_single_query(
                        &options.config,
                        &query,
                        name,
                        description,
                        options.timeout,
                        project,
                        options.model.as_deref(),
                        &environment,
                    );
                    if tx.send((query, item, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        for result in rx {
            completed.push(result);
        }
    });
    if infra_eval::cancel::interrupted() {
        return Err(RuntimeError::unavailable("Trigger evaluation interrupted"));
    }
    let mut triggers: BTreeMap<String, Vec<bool>> = BTreeMap::new();
    let mut first_error = None;
    for (query, item, result) in completed {
        if !native && !triggers.contains_key(&query) {
            query_order.push(query.clone());
        }
        query_items.insert(query.clone(), item);
        let hit = match result {
            Ok(hit) => hit,
            Err(e) => {
                if native {
                    if first_error.is_none() {
                        first_error = Some(RuntimeError::unavailable(format!(
                            "Trigger probe failed for {query:?}: {e}"
                        )));
                    }
                    false
                } else {
                    log.push_str(&format!("Warning: query failed: {e}\n"));
                    false
                }
            }
        };
        triggers.entry(query).or_default().push(hit);
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    let mut results = vec![];
    for query in query_order {
        let values = triggers.remove(&query).unwrap_or_default();
        if values.is_empty() {
            return Err(RuntimeError::unavailable(
                "Evaluation ended before every scheduled probe completed",
            ));
        }
        let count = values.iter().filter(|b| **b).count();
        let rate = count as f64 / values.len() as f64;
        let expected = query_items[&query]["should_trigger"].as_bool().unwrap();
        let passed = if expected {
            rate >= options.threshold
        } else {
            rate < options.threshold
        };
        results.push(json!({"query":query,"should_trigger":expected,"trigger_rate":rate,"triggers":count,"runs":values.len(),"pass":passed}));
    }
    let passed = results.iter().filter(|r| r["pass"] == true).count();
    let mut output = json!({"skill_name":name,"description":description,"results":results,"summary":{"total":results.len(),"passed":passed,"failed":results.len()-passed}});
    if native {
        output["platform"] = json!(options.config.key);
    }
    Ok(output)
}
pub fn options(v: &Value) -> Result<EvalOptions> {
    let config = platform_cli::platform_config(v)?;
    let native = config.key != "claude";
    let workers = common::positive(v, "num_workers", if native { 4 } else { 10 })?;
    let runs = common::positive(v, "runs_per_query", 3)?;
    let threshold = common::number(v, "trigger_threshold", 0.5)?;
    if native && !(0.0 < threshold && threshold <= 1.0) {
        return Err(RuntimeError::invalid(
            "trigger_threshold must be greater than 0 and at most 1",
        ));
    }
    Ok(EvalOptions {
        config,
        workers,
        timeout: Duration::from_secs(
            common::positive(v, "timeout", if native { 60 } else { 30 })? as u64,
        ),
        runs,
        threshold,
        model: common::text(v, "model").map(str::to_owned),
    })
}
pub fn stats(label: &str, results: &[Value], elapsed: f64) -> String {
    let (mut tp, mut tn, mut fp, mut fn_) = (0u64, 0u64, 0u64, 0u64);
    for r in results {
        let runs = r["runs"].as_u64().unwrap_or(0);
        let triggers = r["triggers"].as_u64().unwrap_or(0);
        if r["should_trigger"] == true {
            tp += triggers;
            fn_ += runs.saturating_sub(triggers);
        } else {
            fp += triggers;
            tn += runs.saturating_sub(triggers);
        }
    }
    let total = tp + tn + fp + fn_;
    let precision = if tp + fp > 0 {
        tp as f64 / (tp + fp) as f64
    } else {
        1.0
    };
    let recall = if tp + fn_ > 0 {
        tp as f64 / (tp + fn_) as f64
    } else {
        1.0
    };
    let accuracy = if total > 0 {
        (tp + tn) as f64 / total as f64
    } else {
        0.0
    };
    let mut out = format!(
        "{label}: {}/{total} correct, precision={:.0}% recall={:.0}% accuracy={:.0}% ({elapsed:.1}s)\n",
        tp + tn,
        precision * 100.0,
        recall * 100.0,
        accuracy * 100.0
    );
    for r in results {
        out.push_str(&format!(
            "  [{}] rate={}/{} expected={}: {}\n",
            if r["pass"] == true { "PASS" } else { "FAIL" },
            r["triggers"],
            r["runs"],
            r["should_trigger"],
            common::chars(r["query"].as_str().unwrap_or(""), 60)
        ));
    }
    out
}
pub fn execute(v: &Value) -> Result<Value> {
    let skill = utils::parse_skill_md(Path::new(common::required(v, "skill_path")?))?;
    let eval = common::json_file(Path::new(common::required(v, "eval_set")?))?;
    let list = eval
        .as_array()
        .ok_or_else(|| RuntimeError::invalid("Eval set must be a JSON array"))?;
    let options = options(v)?;
    let project = platform_cli::find_project_root(
        &options.config,
        Path::new(common::text(v, "project_root").unwrap_or(".")),
    )?;
    let description = common::text(v, "description")
        .filter(|s| !s.is_empty())
        .unwrap_or(&skill.description);
    let mut log = String::new();
    if common::flag(v, "verbose") {
        log.push_str(&format!("Testing description: {description}\n"));
    }
    let output = run_eval(list, &skill.name, description, &project, &options, &mut log)?;
    if common::flag(v, "verbose") {
        log.push_str(&format!(
            "Summary: {}/{} passed\n",
            output["summary"]["passed"], output["summary"]["total"]
        ));
        log.push_str(&stats("Eval", output["results"].as_array().unwrap(), 0.0));
    }
    common::json_cli(output, log)
}
