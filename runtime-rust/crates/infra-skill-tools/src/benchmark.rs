use crate::{Result, RuntimeError, common};
use serde_json::{Map, Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

const BENCHMARK_RACY_NS: i128 = 2_000_000_000;
const BENCHMARK_CACHE_CAP: usize = 64;

fn benchmark_now_ns() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0)
}

fn env_cache_bypassed() -> bool {
    std::env::var("INFRA_CHECK_NO_CACHE").as_deref() == Ok("1")
        || std::env::var("INFRA_NO_CACHE").as_deref() == Ok("1")
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BenchFileStamp {
    len: u64,
    mtime_ns: i128,
    ctime_ns: i128,
    mode: u32,
}

impl BenchFileStamp {
    fn from_metadata(meta: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let mtime_ns = (meta.mtime() as i128) * 1_000_000_000 + (meta.mtime_nsec() as i128);
            let ctime_ns = (meta.ctime() as i128) * 1_000_000_000 + (meta.ctime_nsec() as i128);
            Self {
                len: meta.len(),
                mtime_ns,
                ctime_ns,
                mode: meta.mode(),
            }
        }
        #[cfg(not(unix))]
        {
            let mtime_ns = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as i128)
                .unwrap_or(0);
            Self {
                len: meta.len(),
                mtime_ns,
                ctime_ns: mtime_ns,
                mode: if meta.is_dir() { 0o40755 } else { 0o100644 },
            }
        }
    }

    fn is_settled(&self, now_ns: i128) -> bool {
        self.mtime_ns > 0
            && self.ctime_ns > 0
            && now_ns >= self.mtime_ns + BENCHMARK_RACY_NS
            && now_ns >= self.ctime_ns + BENCHMARK_RACY_NS
    }
}

type BenchStampEntry = (String, bool, BenchFileStamp);

struct BenchmarkStampSnapshot {
    canon: PathBuf,
    stamps: Vec<BenchStampEntry>,
    saw_symlink: bool,
}

#[derive(Clone)]
struct CachedBenchmarkRunResults {
    stamps: Vec<BenchStampEntry>,
    results: Map<String, Value>,
    warnings: String,
    last_used: u64,
}

#[derive(Default)]
struct BenchmarkResultsCache {
    seq: u64,
    entries: HashMap<PathBuf, CachedBenchmarkRunResults>,
}

fn benchmark_results_cache() -> &'static Mutex<BenchmarkResultsCache> {
    static CACHE: OnceLock<Mutex<BenchmarkResultsCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BenchmarkResultsCache::default()))
}

#[cfg(test)]
thread_local! {
    static TEST_BENCHMARK_JSON_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn bump_benchmark_parses() {
    TEST_BENCHMARK_JSON_PARSES.with(|c| c.set(c.get() + 1));
}

#[cfg(test)]
fn benchmark_parse_count() -> usize {
    TEST_BENCHMARK_JSON_PARSES.with(std::cell::Cell::get)
}

fn stat_bench_entry(
    p: &Path,
    label: String,
    stamps: &mut Vec<BenchStampEntry>,
    saw_symlink: &mut bool,
) {
    if let Ok(m) = std::fs::symlink_metadata(p) {
        if m.file_type().is_symlink() {
            *saw_symlink = true;
        } else {
            stamps.push((label, m.is_dir(), BenchFileStamp::from_metadata(&m)));
        }
    }
}

fn collect_benchmark_stamps(directory: &Path) -> Result<BenchmarkStampSnapshot> {
    let canon = infra_core::fs::canonicalize(directory)?;
    let mut stamps = Vec::new();
    let mut saw_symlink = false;

    stat_bench_entry(&canon, ".".into(), &mut stamps, &mut saw_symlink);
    let runs_sub = canon.join("runs");
    stat_bench_entry(&runs_sub, "runs".into(), &mut stamps, &mut saw_symlink);

    let search = if runs_sub.exists() {
        runs_sub
    } else if common::sorted(&canon)?.iter().any(|p| {
        p.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("eval-"))
    }) {
        canon.clone()
    } else {
        stamps.sort_by(|a, b| a.0.cmp(&b.0));
        return Ok(BenchmarkStampSnapshot {
            canon,
            stamps,
            saw_symlink,
        });
    };

    for eval in common::sorted(&search)? {
        let eval_name = eval.file_name().unwrap_or_default().to_string_lossy();
        if !eval_name.starts_with("eval-") {
            continue;
        }
        stat_bench_entry(
            &eval,
            format!("eval/{eval_name}"),
            &mut stamps,
            &mut saw_symlink,
        );
        stat_bench_entry(
            &eval.join("eval_metadata.json"),
            format!("eval/{eval_name}/eval_metadata.json"),
            &mut stamps,
            &mut saw_symlink,
        );
        if eval.is_dir() {
            for config in common::sorted(&eval)? {
                if let Ok(cm) = std::fs::symlink_metadata(&config) {
                    if cm.file_type().is_symlink() {
                        saw_symlink = true;
                        continue;
                    }
                    if !cm.is_dir() {
                        continue;
                    }
                } else {
                    continue;
                }
                let cfg_name = config.file_name().unwrap_or_default().to_string_lossy();
                stat_bench_entry(
                    &config,
                    format!("eval/{eval_name}/{cfg_name}"),
                    &mut stamps,
                    &mut saw_symlink,
                );
                for run in common::sorted(&config)? {
                    let run_name = run.file_name().unwrap_or_default().to_string_lossy();
                    if !run_name.starts_with("run-") {
                        continue;
                    }
                    stat_bench_entry(
                        &run,
                        format!("eval/{eval_name}/{cfg_name}/{run_name}"),
                        &mut stamps,
                        &mut saw_symlink,
                    );
                    for fname in ["grading.json", "timing.json"] {
                        stat_bench_entry(
                            &run.join(fname),
                            format!("eval/{eval_name}/{cfg_name}/{run_name}/{fname}"),
                            &mut stamps,
                            &mut saw_symlink,
                        );
                    }
                }
            }
        }
    }
    stamps.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(BenchmarkStampSnapshot {
        canon,
        stamps,
        saw_symlink,
    })
}

pub fn load_run_results(directory: &Path, log: &mut String) -> Result<Map<String, Value>> {
    load_run_results_with_options(directory, log, benchmark_now_ns(), env_cache_bypassed())
}

pub fn load_run_results_with_options(
    directory: &Path,
    log: &mut String,
    now_ns: i128,
    bypass: bool,
) -> Result<Map<String, Value>> {
    let stamp_snapshot = if !bypass {
        collect_benchmark_stamps(directory).ok()
    } else {
        None
    };
    if let Some(snap) = stamp_snapshot.as_ref()
        && !snap.saw_symlink
        && snap.stamps.iter().all(|(_, _, s)| s.is_settled(now_ns))
        && let Ok(mut cache) = benchmark_results_cache().lock()
    {
        cache.seq = cache.seq.saturating_add(1);
        let seq = cache.seq;
        if let Some(hit) = cache.entries.get_mut(&snap.canon)
            && hit.stamps == snap.stamps
        {
            hit.last_used = seq;
            log.push_str(&hit.warnings);
            return Ok(hit.results.clone());
        }
    }

    #[cfg(test)]
    bump_benchmark_parses();
    let mut captured_warnings = String::new();
    let results = load_run_results_uncached(directory, &mut captured_warnings)?;
    log.push_str(&captured_warnings);

    if !bypass
        && let Some(snap) = stamp_snapshot
        && !snap.saw_symlink
        && snap.stamps.iter().all(|(_, _, s)| s.is_settled(now_ns))
        && let Ok(mut cache) = benchmark_results_cache().lock()
    {
        cache.seq = cache.seq.saturating_add(1);
        let seq = cache.seq;
        if cache.entries.len() >= BENCHMARK_CACHE_CAP
            && !cache.entries.contains_key(&snap.canon)
            && let Some(evict) = cache
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(k, _)| k.clone())
        {
            cache.entries.remove(&evict);
        }
        cache.entries.insert(
            snap.canon,
            CachedBenchmarkRunResults {
                stamps: snap.stamps,
                results: results.clone(),
                warnings: captured_warnings,
                last_used: seq,
            },
        );
    }
    Ok(results)
}

pub fn calculate_stats(values: &[f64]) -> Value {
    if values.is_empty() {
        return json!({"mean":0.0,"stddev":0.0,"min":0.0,"max":0.0});
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let sd = if values.len() > 1 {
        (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
    } else {
        0.0
    };
    json!({"mean":common::rounded(mean),"stddev":common::rounded(sd),"min":common::rounded(values.iter().copied().fold(f64::INFINITY,f64::min)),"max":common::rounded(values.iter().copied().fold(f64::NEG_INFINITY,f64::max))})
}
fn number(v: &Value, key: &str) -> f64 {
    v[key].as_f64().unwrap_or(0.0)
}
fn fallback(v: &Value, key: &str, default: Value) -> Value {
    v.get(key).cloned().unwrap_or(default)
}
fn load_run_results_uncached(directory: &Path, log: &mut String) -> Result<Map<String, Value>> {
    let search = if directory.join("runs").exists() {
        directory.join("runs")
    } else if common::sorted(directory)?.iter().any(|p| {
        p.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("eval-"))
    }) {
        directory.into()
    } else {
        log.push_str(&format!(
            "No eval directories found in {} or {}\n",
            directory.display(),
            directory.join("runs").display()
        ));
        return Ok(Map::new());
    };
    let evals = common::sorted(&search)?
        .into_iter()
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("eval-"))
        })
        .collect::<Vec<_>>();
    let mut results = Map::new();
    for (index, eval) in evals.iter().enumerate() {
        let metadata = eval.join("eval_metadata.json");
        let id = if metadata.exists() {
            common::json_file(&metadata)
                .ok()
                .and_then(|v| v.get("eval_id").cloned())
                .unwrap_or(json!(index))
        } else {
            eval.file_name()
                .and_then(|s| s.to_str())
                .and_then(|s| s.split('-').nth(1))
                .and_then(|s| s.parse::<i64>().ok())
                .map(Value::from)
                .unwrap_or(json!(index))
        };
        for config in common::sorted(eval)? {
            if !config.is_dir() {
                continue;
            }
            let runs = common::sorted(&config)?
                .into_iter()
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("run-"))
                })
                .collect::<Vec<_>>();
            if runs.is_empty() {
                continue;
            }
            let name = config.file_name().unwrap().to_string_lossy().into_owned();
            results.entry(name.clone()).or_insert_with(|| json!([]));
            for run in runs {
                let run_number = run
                    .file_name()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.split('-').nth(1))
                    .and_then(|s| s.parse::<u64>().ok())
                    .ok_or_else(|| {
                        RuntimeError::invalid(format!("Invalid run number in {}", run.display()))
                    })?;
                let path = run.join("grading.json");
                if !path.exists() {
                    log.push_str(&format!(
                        "Warning: grading.json not found in {}\n",
                        run.display()
                    ));
                    continue;
                }
                let grading = match common::json_file(&path) {
                    Ok(v) => v,
                    Err(e) => {
                        log.push_str(&format!(
                            "Warning: Invalid JSON in {}: {e}\n",
                            path.display()
                        ));
                        continue;
                    }
                };
                let summary = &grading["summary"];
                let metrics = &grading["execution_metrics"];
                let mut time = fallback(&grading["timing"], "total_duration_seconds", json!(0.0));
                let mut tokens = json!(0);
                if time.as_f64() == Some(0.0)
                    && run.join("timing.json").exists()
                    && let Ok(t) = common::json_file(&run.join("timing.json"))
                {
                    time = fallback(&t, "total_duration_seconds", json!(0.0));
                    tokens = fallback(&t, "total_tokens", json!(0));
                }
                if tokens.as_f64().is_none_or(|n| n == 0.0) {
                    tokens = fallback(metrics, "output_chars", json!(0));
                }
                let expectations = fallback(&grading, "expectations", json!([]));
                if let Some(exps) = expectations.as_array() {
                    for exp in exps {
                        if exp.get("text").is_none() || exp.get("passed").is_none() {
                            log.push_str(&format!("Warning: expectation in {} missing required fields (text, passed, evidence): {exp}\n",path.display()));
                        }
                    }
                }
                let mut notes = vec![];
                for field in ["uncertainties", "needs_review", "workarounds"] {
                    if let Some(a) = grading["user_notes_summary"][field].as_array() {
                        notes.extend(a.clone());
                    }
                }
                results.get_mut(&name).unwrap().as_array_mut().unwrap().push(json!({"eval_id":id,"run_number":run_number,"pass_rate":fallback(summary,"pass_rate",json!(0.0)),"passed":fallback(summary,"passed",json!(0)),"failed":fallback(summary,"failed",json!(0)),"total":fallback(summary,"total",json!(0)),"time_seconds":time,"tokens":tokens,"tool_calls":fallback(metrics,"total_tool_calls",json!(0)),"errors":fallback(metrics,"errors_encountered",json!(0)),"expectations":expectations,"notes":notes}));
            }
        }
    }
    Ok(results)
}
pub fn aggregate_results(results: &Map<String, Value>) -> Value {
    let mut summary = Map::new();
    for (name, runs) in results {
        let mut metrics = Map::new();
        for key in ["pass_rate", "time_seconds", "tokens"] {
            metrics.insert(
                key.into(),
                calculate_stats(
                    &runs
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|r| number(r, key))
                        .collect::<Vec<_>>(),
                ),
            );
        }
        summary.insert(name.clone(), Value::Object(metrics));
    }
    let configs = summary.values().cloned().collect::<Vec<_>>();
    let zero = Value::Null;
    let a = configs.first().unwrap_or(&zero);
    let b = configs.get(1).unwrap_or(&zero);
    summary.insert("delta".into(),json!({"pass_rate":format!("{:+.2}",number(&a["pass_rate"],"mean")-number(&b["pass_rate"],"mean")),"time_seconds":format!("{:+.1}",number(&a["time_seconds"],"mean")-number(&b["time_seconds"],"mean")),"tokens":format!("{:+.0}",number(&a["tokens"],"mean")-number(&b["tokens"],"mean"))}));
    Value::Object(summary)
}
pub fn generate_benchmark(
    directory: &Path,
    name: &str,
    path: &str,
    log: &mut String,
) -> Result<Value> {
    generate_benchmark_with_options(
        directory,
        name,
        path,
        log,
        benchmark_now_ns(),
        env_cache_bypassed(),
    )
}

pub fn generate_benchmark_with_options(
    directory: &Path,
    name: &str,
    path: &str,
    log: &mut String,
    now_ns: i128,
    bypass: bool,
) -> Result<Value> {
    let results = load_run_results_with_options(directory, log, now_ns, bypass)?;
    let summary = aggregate_results(&results);
    let mut runs = vec![];
    let mut ids = vec![];
    for (config, list) in &results {
        for r in list.as_array().unwrap() {
            let mut metrics = Map::new();
            for key in [
                "pass_rate",
                "passed",
                "failed",
                "total",
                "time_seconds",
                "tokens",
                "tool_calls",
                "errors",
            ] {
                metrics.insert(key.into(), r[key].clone());
            }
            if !ids.contains(&r["eval_id"]) {
                ids.push(r["eval_id"].clone());
            }
            runs.push(json!({"eval_id":r["eval_id"],"configuration":config,"run_number":r["run_number"],"result":metrics,"expectations":r["expectations"],"notes":r["notes"]}));
        }
    }
    ids.sort_by(|a, b| match (a.as_f64(), b.as_f64()) {
        (Some(a), Some(b)) => a.total_cmp(&b),
        _ => common::display(a).cmp(&common::display(b)),
    });
    Ok(
        json!({"metadata":{"skill_name":if name.is_empty(){"<skill-name>"}else{name},"skill_path":if path.is_empty(){"<path/to/skill>"}else{path},"executor_model":"<model-name>","analyzer_model":"<model-name>","timestamp":chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),"evals_run":ids,"runs_per_configuration":3},"runs":runs,"run_summary":summary,"notes":[]}),
    )
}
pub fn generate_markdown(v: &Value) -> String {
    let m = &v["metadata"];
    let s = &v["run_summary"];
    let configs = s
        .as_object()
        .map(|m| {
            m.keys()
                .filter(|k| *k != "delta")
                .map(String::as_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let a = configs.first().copied().unwrap_or("config_a");
    let b = configs.get(1).copied().unwrap_or("config_b");
    let ids = m["evals_run"]
        .as_array()
        .map(|a| a.iter().map(common::display).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    let mut out = format!(
        "# Skill Benchmark: {}\n\n**Model**: {}\n**Date**: {}\n**Evals**: {ids} ({} runs each per configuration)\n\n## Summary\n\n| Metric | {} | {} | Delta |\n|--------|------------|---------------|-------|\n",
        common::display(&m["skill_name"]),
        common::display(&m["executor_model"]),
        common::display(&m["timestamp"]),
        m["runs_per_configuration"],
        common::title(a),
        common::title(b)
    );
    for (key, label, digits, suffix, scale) in [
        ("pass_rate", "Pass Rate", 0, "%", 100.0),
        ("time_seconds", "Time", 1, "s", 1.0),
        ("tokens", "Tokens", 0, "", 1.0),
    ] {
        let av = &s[a][key];
        let bv = &s[b][key];
        let delta = s["delta"][key].as_str().unwrap_or("—");
        out.push_str(&format!("| {label} | {:.*}{suffix} ± {:.*}{suffix} | {:.*}{suffix} ± {:.*}{suffix} | {delta}{} |\n",digits,number(av,"mean")*scale,digits,number(av,"stddev")*scale,digits,number(bv,"mean")*scale,digits,number(bv,"stddev")*scale,if key=="time_seconds"{"s"}else{""}));
    }
    if let Some(notes) = v["notes"].as_array().filter(|n| !n.is_empty()) {
        out.push_str("\n## Notes\n\n");
        for note in notes {
            out.push_str(&format!("- {}\n", common::display(note)));
        }
    }
    out.pop();
    out
}
pub fn execute(v: &Value) -> Result<Value> {
    let path = common::positional(v, "benchmark_dir", 0)?;
    let directory = Path::new(&path);
    if !directory.exists() {
        return Ok(common::cli(
            1,
            format!("Directory not found: {path}\n"),
            "",
            Value::Null,
        ));
    }
    let bypass = common::flag(v, "strict") || common::flag(v, "no_cache") || env_cache_bypassed();
    let mut log = String::new();
    let benchmark = generate_benchmark_with_options(
        directory,
        common::text(v, "skill_name").unwrap_or(""),
        common::text(v, "skill_path").unwrap_or(""),
        &mut log,
        benchmark_now_ns(),
        bypass,
    )?;
    let output = common::text(v, "output")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| directory.join("benchmark.json"));
    let markdown = output.with_extension("md");
    common::write_json(&output, &benchmark, false)?;
    std::fs::write(&markdown, generate_markdown(&benchmark))?;
    log.push_str(&format!(
        "Generated: {}\nGenerated: {}\n\nSummary:\n",
        output.display(),
        markdown.display()
    ));
    for (key, val) in benchmark["run_summary"].as_object().unwrap() {
        if key != "delta" {
            log.push_str(&format!(
                "  {}: {:.1}% pass rate\n",
                common::title(key),
                number(&val["pass_rate"], "mean") * 100.0
            ));
        }
    }
    log.push_str(&format!(
        "  Delta:         {}\n",
        benchmark["run_summary"]["delta"]["pass_rate"]
            .as_str()
            .unwrap_or("—")
    ));
    Ok(common::cli(0, log, "", benchmark))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_run_results_caches_settled_json_reads_and_respects_racy_ns_and_bypass() {
        let temp = tempfile::tempdir().unwrap();
        let ws = temp.path().join("bench");
        let run1 = ws.join("runs/eval-1/with_skill/run-1");
        std::fs::create_dir_all(&run1).unwrap();
        let grading = run1.join("grading.json");
        std::fs::write(
            &grading,
            r#"{"summary":{"pass_rate":1.0,"passed":2,"failed":0,"total":2},"execution_metrics":{"total_tool_calls":3,"errors_encountered":0},"timing":{"total_duration_seconds":1.5}}"#,
        )
        .unwrap();

        let stamp = BenchFileStamp::from_metadata(&std::fs::symlink_metadata(&grading).unwrap());
        let racy_now = stamp.mtime_ns + BENCHMARK_RACY_NS - 1;
        let settled_now = stamp.mtime_ns + BENCHMARK_RACY_NS + 10_000;

        let parses_before = benchmark_parse_count();

        // Inside BENCHMARK_RACY_NS: always re-parses from JSON.
        let mut log = String::new();
        let r_racy1 = load_run_results_with_options(&ws, &mut log, racy_now, false).unwrap();
        let r_racy2 = load_run_results_with_options(&ws, &mut log, racy_now, false).unwrap();
        assert_eq!(r_racy1, r_racy2);
        assert_eq!(benchmark_parse_count(), parses_before + 2);

        // Settled beyond BENCHMARK_RACY_NS: first warms, second skips JSON re-reads completely.
        let r1 = load_run_results_with_options(&ws, &mut log, settled_now, false).unwrap();
        let r2 = load_run_results_with_options(&ws, &mut log, settled_now + 5, false).unwrap();
        assert_eq!(r1, r2);
        assert_eq!(benchmark_parse_count(), parses_before + 3);

        // strict / no_cache bypass forces live JSON parse even when settled.
        let r_bypass =
            load_run_results_with_options(&ws, &mut log, settled_now + 10, true).unwrap();
        assert_eq!(r_bypass, r1);
        assert_eq!(benchmark_parse_count(), parses_before + 4);

        // Same-length grading edit invalidates settled cache cleanly.
        std::fs::write(
            &grading,
            r#"{"summary":{"pass_rate":0.5,"passed":1,"failed":1,"total":2},"execution_metrics":{"total_tool_calls":3,"errors_encountered":0},"timing":{"total_duration_seconds":1.5}}"#,
        )
        .unwrap();
        let updated_stamp =
            BenchFileStamp::from_metadata(&std::fs::symlink_metadata(&grading).unwrap());
        let r3 = load_run_results_with_options(
            &ws,
            &mut log,
            updated_stamp.mtime_ns + BENCHMARK_RACY_NS + 10_000_000,
            false,
        )
        .unwrap();
        assert_eq!(r3["with_skill"][0]["pass_rate"], json!(0.5));
        assert_eq!(benchmark_parse_count(), parses_before + 5);

        // execute honors strict / no_cache flags without error.
        let cli_out = execute(&json!({
            "benchmark_dir": ws,
            "strict": true,
            "output": temp.path().join("out.json")
        }))
        .unwrap();
        assert_eq!(cli_out["_cli"]["code"], json!(0));
    }
}
