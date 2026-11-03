//! Maintainer-only ONNX injection before session identity. Eligibility and pack order stay in core.
use crate::executable;
use crate::{Result, artifact};
use infra_pilots::excerpt::ExcerptScorer;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
pub struct Adapter<S> {
    pub scorer: S,
    pub deadline: Instant,
    pub stats: BTreeMap<String, u64>,
}
impl<S: ExcerptScorer> infra_core::excerpts::ExcerptPreparer for Adapter<S> {
    fn select(
        &mut self,
        query: &str,
        candidates: &[infra_core::excerpts::PreparationCandidate],
    ) -> Result<BTreeMap<String, usize>> {
        let mut passages = vec![];
        let mut spans = vec![];
        for candidate in candidates {
            if candidate.alternatives.len() < 2 {
                continue;
            }
            let start = passages.len();
            for alternative in &candidate.alternatives {
                passages.push(alternative.text.clone());
            }
            spans.push((candidate.path.clone(), start, passages.len()));
        }
        if spans.is_empty() {
            *self
                .stats
                .entry("fallback:no-alternatives".into())
                .or_default() += 1;
            return Ok(BTreeMap::new());
        }
        if passages.len() > 32
            || serde_json::to_vec(&json!({"query":query,"passages":passages}))?.len() > 48 * 1024
        {
            *self
                .stats
                .entry("fallback:request-limit".into())
                .or_default() += 1;
            return Err(artifact::invalid("onnx request limit"));
        }
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            *self.stats.entry("fallback:deadline".into()).or_default() += 1;
            return Err(artifact::invalid("onnx deadline"));
        }
        *self.stats.entry("scorer_calls".into()).or_default() += 1;
        let scores = match self.scorer.score(query, &passages, remaining) {
            Ok(v) => v,
            Err(e) => {
                *self
                    .stats
                    .entry(format!("fallback:{}", e.reason))
                    .or_default() += 1;
                return Err(artifact::invalid(e.reason));
            }
        };
        if scores.len() != passages.len() || scores.iter().any(|s| !s.is_finite()) {
            *self.stats.entry("fallback:scores".into()).or_default() += 1;
            return Err(artifact::invalid("invalid scorer output"));
        }
        let mut selected = BTreeMap::new();
        for (path, start, end) in spans {
            let mut winner = start;
            for index in start + 1..end {
                if scores[index] > scores[winner] {
                    winner = index;
                }
            }
            if winner != start {
                *self.stats.entry("proposed".into()).or_default() += 1;
            }
            selected.insert(path, winner - start);
        }
        Ok(selected)
    }
}
pub fn refresh(
    root: &Path,
    request: &Value,
    assets: &Path,
    worker: &Path,
    ort: &Path,
    timeout: Duration,
    invocation: Duration,
) -> Result<Value> {
    refresh_at(
        root, request, assets, worker, ort, timeout, invocation, None,
    )
}
#[expect(
    clippy::too_many_arguments,
    reason = "The optional scorer bridge binds roots and request to pinned assets, worker, ORT library, two budgets and an explicit clock."
)]
pub fn refresh_at(
    root: &Path,
    request: &Value,
    assets: &Path,
    worker: &Path,
    ort: &Path,
    timeout: Duration,
    invocation: Duration,
    anchor: Option<&str>,
) -> Result<Value> {
    let _clock = crate::clock::enter(anchor)?;
    let context =
        infra_core::RuntimeContext::new(infra_core::Roots::installed(root)?, "local", "governed")?;
    let service = infra_core::MemoryService::new(context, None);
    let service = if anchor.is_some() {
        service.with_eval_no_child()
    } else {
        service
    };
    let scorer = infra_pilots::onnx::Scorer {
        assets: assets.into(),
        worker: worker.into(),
        ort_library: ort.into(),
        timeout,
    };
    let mut adapter = Adapter {
        scorer,
        deadline: Instant::now() + invocation,
        stats: BTreeMap::new(),
    };
    let refresh = infra_core::application::request_refresh(request)?;
    let value = service.refresh_with_preparer(refresh, &mut adapter)?;
    Ok(json!({"refresh":value,"stats":adapter.stats}))
}
pub fn worker(args: &[String]) -> Result<Option<i32>> {
    if args.first().map(String::as_str) != Some("__onnx-refresh") {
        return Ok(None);
    }
    let file = args
        .get(1)
        .ok_or_else(|| artifact::invalid("ONNX request required"))?;
    let v = artifact::read_json(Path::new(file), artifact::MAX_TREE)?;
    let path = |key: &str| -> Result<PathBuf> {
        v[key]
            .as_str()
            .map(PathBuf::from)
            .ok_or_else(|| artifact::invalid("ONNX worker path required"))
    };
    let timeout = |key: &str, default: f64| -> Result<Duration> {
        let n = v[key].as_f64().unwrap_or(default);
        if !n.is_finite() || n <= 0.0 {
            return Err(artifact::invalid("invalid ONNX budget"));
        }
        Ok(Duration::from_secs_f64(n))
    };
    let result = refresh_at(
        &path("root")?,
        &v["request"],
        &path("assets")?,
        &path("worker")?,
        &path("ort_library")?,
        timeout("worker_timeout", 5.0)?,
        timeout("invocation_timeout", 30.0)?,
        crate::clock::anchor(&v)?,
    )?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(Some(0))
}

pub fn pilot(v: &Value) -> Result<Value> {
    let context = v["context"]
        .as_str()
        .ok_or_else(|| artifact::invalid("--context native executable required"))?;
    let executable = PathBuf::from(context);
    if executable.extension().is_some_and(|e| e == "py") {
        return Err(artifact::invalid(
            "--context must name the native runtime executable",
        ));
    }
    executable::SelectedExecutableSource::capture(&executable)?;
    let root = v["root"]
        .as_str()
        .map(PathBuf::from)
        .or_else(|| {
            executable
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .map(PathBuf::from)
        })
        .ok_or_else(|| artifact::invalid("native context project root unavailable"))?;
    let assets = PathBuf::from(
        v["assets"]
            .as_str()
            .ok_or_else(|| artifact::invalid("--assets required"))?,
    );
    let worker = v["worker"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or(std::env::current_exe()?);
    if v["python"].is_string() {
        return Err(artifact::invalid(
            "ONNX pilot requires native --worker; Python forwarding is unavailable",
        ));
    }
    let ort = PathBuf::from(
        v["ort_library"]
            .as_str()
            .ok_or_else(|| artifact::invalid("--ort-library required"))?,
    );
    let stats = PathBuf::from(
        v["stats"]
            .as_str()
            .ok_or_else(|| artifact::invalid("--stats required"))?,
    );
    let duration = |key: &str, default: f64| -> Result<Duration> {
        let n = v[key]
            .as_f64()
            .or_else(|| v[key].as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(default);
        if !n.is_finite() || n <= 0.0 {
            return Err(artifact::invalid("ONNX duration must be positive finite"));
        }
        Ok(Duration::from_secs_f64(n))
    };
    let timeout = duration("invocation_timeout", 30.0)?;
    #[cfg(unix)]
    let mut timeout = timeout;
    if let Some(deadline) = v["invocation_deadline"].as_f64().or_else(|| {
        v["invocation_deadline"]
            .as_str()
            .and_then(|s| s.parse().ok())
    }) {
        if !deadline.is_finite() {
            return Err(artifact::invalid("invalid invocation deadline"));
        }
        #[cfg(unix)]
        {
            let mut now: libc::timespec = unsafe { std::mem::zeroed() };
            if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) } < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let remaining = (deadline - now.tv_sec as f64 - now.tv_nsec as f64 / 1e9).max(0.0);
            timeout = timeout.min(Duration::from_secs_f64(remaining));
        }
    }
    let mut args = v["arguments"]
        .as_array()
        .or_else(|| v["_remainder"].as_array())
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect::<Vec<_>>();
    if args.first().map(String::as_str) == Some("--") {
        args.remove(0);
    }
    let operation = args
        .first()
        .map(String::as_str)
        .unwrap_or("refresh")
        .to_string();
    let mut request = json!({});
    let mut positionals = vec![];
    let mut index = 1;
    while index < args.len() {
        let arg = &args[index];
        if let Some(key) = arg.strip_prefix("--") {
            let (key, inline) = key
                .split_once('=')
                .map(|(k, v)| (k, Some(v)))
                .unwrap_or((key, None));
            let key = key.replace('-', "_");
            if let Some(value) = inline {
                request[&key] = value.into();
            } else if [
                "json",
                "ephemeral",
                "sanitize",
                "validate",
                "read_only",
                "incremental",
                "provenance_response",
                "automatic_links",
            ]
            .contains(&key.as_str())
            {
                request[&key] = true.into();
            } else {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| artifact::invalid("context option lacks value"))?;
                request[&key] =
                    if ["limit", "window", "max_files", "flush_after"].contains(&key.as_str()) {
                        json!(
                            value
                                .parse::<usize>()
                                .map_err(|_| artifact::invalid("invalid numeric context option"))?
                        )
                    } else {
                        json!(value)
                    };
            }
        } else {
            positionals.push(arg.clone());
        }
        index += 1;
    }
    if !positionals.is_empty() {
        request["query"] = positionals.join(" ").into();
    }
    let _clock = crate::clock::request(v)?;
    let context =
        infra_core::RuntimeContext::new(infra_core::Roots::installed(&root)?, "local", "governed")?;
    let service = infra_core::MemoryService::new(context.clone(), None);
    let service = if crate::clock::anchor(v)?.is_some() {
        service.with_eval_no_child()
    } else {
        service
    };
    let mut adapter = Adapter {
        scorer: infra_pilots::onnx::Scorer {
            assets,
            worker,
            ort_library: ort,
            timeout: duration("worker_timeout", 5.0)?,
        },
        deadline: Instant::now() + timeout,
        stats: BTreeMap::new(),
    };
    let result = match operation.as_str() {
        "refresh" => service.refresh_with_preparer(
            infra_core::application::request_refresh(&request)?,
            &mut adapter,
        ),
        "retrieve" | "context" => {
            let mut store = service.open()?;
            let mut request = infra_core::application::request_retrieval(&request)?;
            request.entry_point = operation.clone();
            request.allow_unprovisioned = true;
            infra_core::retrieve_with_preparer(&mut store, &context, &request, &mut adapter)
                .map(|c| c.0)
        }
        _ => service.execute(&operation, &request, &[]),
    };
    let _ = artifact::write_json(&stats, &json!(adapter.stats));
    let value = result?;
    let text = format!(
        "{}\n",
        infra_core::domain::json::serialize(&value, true, false, true, false)?
    );
    Ok(
        json!({"_cli":{"code":if crate::cancel::interrupted(){128+crate::cancel::signal()}else{0},"stdout":text,"stderr":""},"result":value}),
    )
}
