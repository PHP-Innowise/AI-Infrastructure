use crate::executable;
use crate::outcome::CorpusPreparation;
use crate::{
    Result, artifact, cost, field, native, outcome,
    reconstruction::{self, Reconstructor},
    sampling, scoring, transcript, treatments,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v[key].as_str()
}
fn required<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    s(v, key).ok_or_else(|| artifact::invalid(&format!("--{} is required", key.replace('_', "-"))))
}
fn path(v: &Value, key: &str) -> Result<PathBuf> {
    Ok(expand(required(v, key)?))
}
fn expand(s: &str) -> PathBuf {
    if s == "~" {
        home()
    } else if let Some(tail) = s.strip_prefix("~/") {
        home().join(tail)
    } else {
        PathBuf::from(s)
    }
}
pub(crate) fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}
fn strings(v: &Value, key: &str, default: &[&str]) -> Vec<String> {
    match &v[key] {
        Value::String(s) => s
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        Value::Array(a) => a
            .iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect(),
        _ => default.iter().map(|s| s.to_string()).collect(),
    }
}
fn number(v: &Value, key: &str, default: usize) -> usize {
    v[key]
        .as_u64()
        .map(|n| n as usize)
        .or_else(|| s(v, key).and_then(|s| s.parse().ok()))
        .unwrap_or(default)
}
fn seed(v: &Value) -> Result<String> {
    let raw = match &v["seed"] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return Err(artifact::invalid("--seed required")),
    };
    crate::rng::Random::decimal(&raw)?;
    Ok(raw)
}
fn seconds(v: &Value, key: &str, default: f64) -> Result<Duration> {
    let n = v[key]
        .as_f64()
        .or_else(|| s(v, key).and_then(|s| s.parse().ok()))
        .unwrap_or(default);
    if !n.is_finite() || n <= 0.0 {
        return Err(artifact::invalid("budget must be finite and positive"));
    }
    Ok(Duration::from_secs_f64(n))
}
fn source(v: &Value) -> Result<PathBuf> {
    if let Some(s) = s(v, "tooling_root")
        .or_else(|| s(v, "source_root"))
        .or_else(|| s(v, "source_dir"))
    {
        return artifact::ordinary(&expand(s));
    }
    if let Some(p) = std::env::var_os("INFRA_TOOLING_ROOT") {
        return artifact::ordinary(Path::new(&p));
    }
    for p in std::env::current_dir()?.ancestors() {
        if p.join("install/inventories").is_dir() && p.join("PHP Core").is_dir() {
            return Ok(p.into());
        }
    }
    Err(artifact::invalid(
        "tooling root is required outside an infrastructure checkout",
    ))
}
fn runtime(v: &Value) -> Result<PathBuf> {
    match s(v, "runtime_binary") {
        Some(s) => Ok(expand(s)),
        None => Ok(std::env::current_exe()?),
    }
}
fn cache(v: &Value) -> PathBuf {
    s(v, "cache").map(expand).unwrap_or_else(|| {
        std::env::temp_dir().join(format!(
            "memory-eval-{}",
            std::env::var("USER").unwrap_or_else(|_| "user".into())
        ))
    })
}
fn roots(v: &Value) -> (PathBuf, PathBuf) {
    (
        s(v, "claude_root")
            .or_else(|| s(v, "transcripts"))
            .map(expand)
            .unwrap_or_else(|| home().join(".claude/projects")),
        s(v, "codex_root")
            .map(expand)
            .unwrap_or_else(|| home().join(".codex/sessions")),
    )
}
fn projects(v: &Value) -> Result<Vec<PathBuf>> {
    let raw = strings(v, "project", &[]);
    if raw.is_empty() {
        return Err(artifact::invalid("--project is required"));
    }
    raw.iter()
        .map(|s| {
            let p = artifact::ordinary(&expand(s))?;
            if !p.is_dir() {
                return Err(artifact::invalid("project directory unavailable"));
            }
            Ok(p)
        })
        .collect()
}
fn since(v: &Value, key: &str) -> Result<Option<chrono::DateTime<chrono::Utc>>> {
    let Some(raw) = s(v, key) else {
        return Ok(None);
    };
    if let Some(t) = reconstruction::instant(raw) {
        return Ok(Some(t));
    }
    let day = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map_err(|_| artifact::invalid("date must be YYYY-MM-DD or ISO timestamp"))?;
    use chrono::TimeZone;
    Ok(day
        .and_hms_opt(0, 0, 0)
        .map(|d| chrono::Utc.from_utc_datetime(&d)))
}
fn envelope(value: Value, code: i32, stdout: String) -> Value {
    json!({"_cli":{"code":code,"stdout":stdout,"stderr":""},"result":value})
}
fn json_output(v: Value, code: i32) -> Result<Value> {
    let text = format!(
        "{}\n",
        infra_core::domain::json::serialize(&v, true, false, true, false)?
    );
    Ok(envelope(v, code, text))
}
fn render_output(
    value: Value,
    code: i32,
    request: &Value,
    render: fn(&Value) -> String,
) -> Result<Value> {
    if request["json"] == true {
        json_output(value, code)
    } else {
        let text = format!("{}\n", render(&value));
        Ok(envelope(value, code, text))
    }
}
fn preparation(v: &Value) -> Result<sampling::NativePreparation> {
    let source = source(v)?;
    let cache = cache(v);
    let runtime = runtime(v)?;
    let (claude, codex) = roots(v);
    sampling::NativePreparation::new(
        Reconstructor::new(&source, &cache, &runtime)?,
        s(v, "projects_root")
            .map(expand)
            .unwrap_or_else(|| home().join("Desktop")),
        claude,
        codex,
        seconds(v, "timeout", 900.0)?,
    )
}
fn selected_set(v: &Value) -> Result<(Vec<Value>, Vec<String>)> {
    let raw = artifact::read_json(&path(v, "set")?, artifact::MAX_TREE)?;
    let a = raw
        .as_array()
        .ok_or_else(|| artifact::invalid("evaluation set must be list"))?;
    let mut seen = BTreeSet::new();
    let mut out = vec![];
    for item in a {
        let id = match &item["id"] {
            Value::String(s) if !s.trim().is_empty() => s.clone(),
            Value::Number(n) if n.is_i64() || n.is_u64() => n.to_string(),
            _ => return Err(artifact::invalid("invalid prompt ID")),
        };
        if !seen.insert(id.clone())
            || ["project", "ts", "prompt"]
                .iter()
                .any(|k| !item[*k].is_string())
        {
            return Err(artifact::invalid("invalid or duplicate prompt"));
        }
        let mut item = item.clone();
        item["id"] = id.into();
        out.push(item);
    }
    let ids = strings(v, "ids", &[]);
    let warnings = ids
        .iter()
        .filter(|id| !seen.contains(id.as_str()))
        .map(|id| format!("warning: --ids names {id:?}, which the set does not have\n"))
        .collect::<Vec<_>>();
    if !ids.is_empty() {
        out.retain(|p| {
            p["id"]
                .as_str()
                .is_some_and(|s| ids.iter().any(|id| id == s))
        });
    }
    if v.get("limit").is_some() {
        out.truncate(number(v, "limit", 0));
    }
    Ok((out, warnings))
}
fn validate_eval_run_options(v: &Value) -> Result<()> {
    for (key, choices) in [
        ("host", &["claude", "codex", "cursor"][..]),
        ("as_of", &["prompt", "now"][..]),
        ("clock", &["as-of", "real"][..]),
        ("updated_after", &["drop", "keep"][..]),
    ] {
        if let Some(value) = v.get(key).filter(|value| !value.is_null())
            && !value.as_str().is_some_and(|value| choices.contains(&value))
        {
            return Err(artifact::invalid(&format!(
                "Invalid value for --{}",
                key.replace('_', "-")
            )));
        }
    }
    Ok(())
}
pub fn eval_run(v: &Value) -> Result<Value> {
    crate::process::ensure_custody()?;
    validate_eval_run_options(v)?;
    crate::cli_contract::validate("memory-eval/run", v)?;
    if v["onnx_python"].is_string() {
        return Err(artifact::invalid(
            "--onnx-python is unavailable in the native runtime; use --onnx-worker and --onnx-ort-library",
        ));
    }
    let source = source(v)?;
    let cache = cache(v);
    let runtime = runtime(v)?;
    let r = Reconstructor::new(&source, &cache, &runtime)?;
    let (set, warnings) = selected_set(v)?;
    let grades = scoring::judgments(&artifact::read_json(
        &path(v, "judgments")?,
        artifact::MAX_TREE,
    )?)?;
    let passages = if let Some(p) = s(v, "passages") {
        scoring::passages(&artifact::read_json(&expand(p), artifact::MAX_TREE)?)?
    } else {
        BTreeMap::new()
    };
    let out = path(v, "out")?;
    if artifact::overlaps(&out, &source)? {
        return Err(artifact::invalid(
            "client result must be outside tooling clone",
        ));
    }
    let timeout = seconds(v, "timeout", 30.0)?;
    let work = artifact::Scratch::new(&cache, "run")?;
    let mut units = json!({});
    for p in &set {
        let bound = json!({"id":p["id"],"project":p["project"],"ts":p["ts"],"prompt":p["prompt"]});
        units[p["id"].as_str().unwrap()] = json!({"id":p["id"],"input_sha256":artifact::spaced_digest(&bound,true,false)?,"state":"unattempted","runtime_admitted":false});
    }
    let pinned = s(v, "as_of").unwrap_or("prompt") == "prompt"
        && s(v, "clock").unwrap_or("as-of") == "as-of";
    let clone_commit = reconstruction::git(&source, &["rev-parse", "HEAD"]);
    if let Err(error) = &clone_commit {
        crate::process::record_custody_error(error);
    }
    crate::process::ensure_custody()?;
    let mut result = json!({"meta":{"tool":"scripts/memory_eval.py","python":null,"edition":s(v,"edition").unwrap_or("auto"),"editions":{},"clone_commit":clone_commit.ok(),"host":s(v,"host").unwrap_or("claude"),"as_of":s(v,"as_of").unwrap_or("prompt"),"clock":if pinned{"pinned"}else{"real"},"clock_profile":if pinned{crate::clock::PROFILE}else{"system-v1"},"updated_after":s(v,"updated_after").unwrap_or("drop"),"timeout":timeout.as_secs_f64(),"set_sha256":artifact::sha(&artifact::read_file(&path(v,"set")?,artifact::MAX_TREE)?.0),"judgments_sha256":artifact::sha(&artifact::read_file(&path(v,"judgments")?,artifact::MAX_TREE)?.0),"passages_sha256":s(v,"passages").map(|p|artifact::read_file(&expand(p),artifact::MAX_TREE).map(|r|artifact::sha(&r.0))).transpose()?,"selection":{"ids":if strings(v,"ids",&[]).is_empty(){Value::Null}else{json!(strings(v,"ids",&[]))},"limit":v["limit"]},"runtime_language":"rust","runtime_artifact_profile":"native-executable-image-v1","started":reconstruction::now(),"finished":null},"items":{},"planned":{"profile":"planned-delivery-v1","ids":set.iter().map(|p|p["id"].clone()).collect::<Vec<_>>(),"units":units}});
    if v["diagnose_misses"] == true {
        result["meta"]["diagnose_misses"] = true.into();
    }
    if v["audit_git"] == true {
        if v["diagnose_misses"] != true {
            return Err(artifact::invalid("--audit-git requires --diagnose-misses"));
        }
        result["meta"]["audit_git"] = true.into();
    }
    if let Some(assets) = s(v, "onnx_excerpts") {
        let worker = s(v, "onnx_worker")
            .map(expand)
            .unwrap_or_else(|| runtime.clone());
        let ort = path(v, "onnx_ort_library")?;
        result["meta"]["onnx_excerpts"] = json!({"assets_manifest_sha256":artifact::sha(&artifact::read_file(&expand(assets).join("manifest.json"),artifact::MAX_TREE)?.0),"worker_sha256":executable::digest(&worker)?,"ort_library_sha256":artifact::sha(&artifact::read_file(&ort,512*1024*1024)?.0),"worker_timeout":seconds(v,"onnx_timeout",5.0)?.as_secs_f64(),"worker_language":"rust","scope":"selected-document-excerpts-only"});
    } else if v["onnx_worker"].is_string() || v["onnx_ort_library"].is_string() {
        return Err(artifact::invalid(
            "ONNX worker flags require --onnx-excerpts",
        ));
    }
    persist_eval(&out, &mut result)?;
    let mut evaluated = 0;
    for p in &set {
        crate::process::ensure_custody()?;
        if crate::cancel::interrupted() {
            result["meta"]["interrupted"] = true.into();
            break;
        }
        let id = p["id"].as_str().unwrap();
        result["planned"]["units"][id]["state"] = "preparing".into();
        persist_eval(&out, &mut result)?;
        let corpus = work.0.join(format!(
            "{}-{}",
            reconstruction::safe_name(id),
            &artifact::sha(id.as_bytes())[..8]
        ));
        let mut item = json!({"project":p["project"],"ts":p["ts"],"status":"skipped","planned_id":id,"input_sha256":result["planned"]["units"][id]["input_sha256"]});
        let run = (|| -> Result<()> {
            let stamp = p["ts"]
                .as_str()
                .and_then(reconstruction::instant)
                .ok_or_else(|| artifact::invalid("bad-timestamp"))?;
            let project = expand(p["project"].as_str().unwrap());
            let project = if project.is_absolute() {
                project
            } else {
                PathBuf::from(s(v, "projects_root").ok_or_else(|| artifact::invalid("no-project"))?)
                    .join(project)
            };
            if !project.is_dir() {
                return Err(artifact::invalid("no-project"));
            }
            let edition = r.edition(&project, s(v, "edition"))?;
            item["edition_source"] = if s(v, "edition").is_some_and(|s| s != "auto") {
                "forced"
            } else {
                "detected"
            }
            .into();
            if result["meta"]["editions"].get(&edition).is_none() {
                let inventory = r.installer.inventory(
                    &edition,
                    &reconstruction::TOOLS
                        .iter()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>(),
                )?;
                let inputs = inventory
                    .files
                    .iter()
                    .map(|f| {
                        Ok(json!([
                            f.path,
                            artifact::sha(
                                &executable::read_inventory_source(f, 512 * 1024 * 1024)?.0
                            ),
                            f.executable
                        ]))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let dirty =
                    reconstruction::git(&source, &["status", "--porcelain", "--", &edition]);
                if let Err(error) = &dirty {
                    crate::process::record_custody_error(error);
                }
                crate::process::ensure_custody()?;
                let dirty = dirty.is_ok_and(|value| !value.is_empty());
                result["meta"]["editions"][&edition] = json!({"release":inventory.release,"dirty":dirty,"runtime_sha256":executable::digest(&runtime)?,"resource_sha256":artifact::digest(&json!(inputs))?});
            }
            let built = r.build(
                &project,
                &corpus,
                stamp,
                &edition,
                s(v, "as_of") == Some("now"),
                s(v, "updated_after") == Some("keep"),
                true,
            )?;
            for (k, value) in built.as_object().unwrap() {
                item[k] = value.clone();
            }
            item["contaminated"] = (built["provenance"]["updated_after"].as_u64().unwrap_or(0) > 0
                || built["provenance"]["archive_updated_after"]
                    .as_u64()
                    .unwrap_or(0)
                    > 0)
            .into();
            let g = grades.get(id).cloned().unwrap_or_default();
            let ps = passages.get(id).cloned().unwrap_or_default();
            let existed = g
                .iter()
                .filter(|(p, g)| **g >= 1 && corpus.join(p).is_file())
                .map(|(p, _)| p.clone())
                .collect::<Vec<_>>();
            let answers = existed
                .iter()
                .filter(|p| {
                    ps.get(*p).is_some_and(|e| {
                        e["useful"] != false
                            && e["passages"].as_array().is_some_and(|a| {
                                a.iter()
                                    .filter_map(Value::as_str)
                                    .any(|p| !scoring::passage_body(p).is_empty())
                            })
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            let expected = scoring::labelled(&answers, &ps)
                .into_iter()
                .filter(|answer| {
                    answers.iter().any(|p| {
                        artifact::read_file(&corpus.join(p), 512 * 1024 * 1024)
                            .ok()
                            .and_then(|r| String::from_utf8(r.0).ok())
                            .is_some_and(|text| scoring::answer_flatten(&text).contains(answer))
                    })
                })
                .collect::<BTreeSet<_>>();
            result["planned"]["units"][id]["state"] = "runtime-admitted".into();
            result["planned"]["units"][id]["runtime_admitted"] = true.into();
            result["planned"]["units"][id]["source_opportunity"] = json!({"useful_paths":existed.len(),"answer_paths":answers.len(),"answer_passages":expected.len()});
            persist_eval(&out, &mut result)?;
            let env = reconstruction::runtime_env(&cache, stamp, pinned)?;
            let (index_exit, _, index_seconds) = r.runtime_at(
                &corpus,
                &["index".into(), "--json".into()],
                &env,
                timeout,
                pinned.then_some(stamp),
            )?;
            crate::process::ensure_custody()?;
            let args = vec![
                "refresh".into(),
                "--host".into(),
                s(v, "host").unwrap_or("claude").into(),
                "--query".into(),
                p["prompt"].as_str().unwrap().into(),
                "--task-id".into(),
                reconstruction::git(&corpus, &["branch", "--show-current"])?,
                "--limit".into(),
                "3".into(),
                "--ephemeral".into(),
                "--sanitize".into(),
                "--json".into(),
            ];
            let (exit, raw, seconds) = if let Some(assets) = s(v, "onnx_excerpts") {
                let input = work
                    .0
                    .join(format!("onnx-{}.json", reconstruction::safe_name(id)));
                let request = json!({"eval_clock":if pinned{json!(stamp.to_rfc3339())}else{Value::Null},"root":corpus,"assets":expand(assets),"worker":s(v,"onnx_worker").map(expand).unwrap_or_else(||runtime.clone()),"ort_library":path(v,"onnx_ort_library")?,"worker_timeout":seconds(v,"onnx_timeout",5.0)?.as_secs_f64(),"invocation_timeout":timeout.as_secs_f64(),"request":{"host":s(v,"host").unwrap_or("claude"),"query":p["prompt"],"task_id":reconstruction::git(&corpus,&["branch","--show-current"])?,"limit":3,"ephemeral":true,"sanitize":true}});
                artifact::write_json(&input, &request)?;
                let spec = crate::process::CommandSpec {
                    argv: vec![
                        runtime.to_string_lossy().into(),
                        "__onnx-refresh".into(),
                        input.to_string_lossy().into(),
                    ],
                    cwd: corpus.clone(),
                    env: env.clone(),
                    input: vec![],
                };
                let started = Instant::now();
                let (code, bytes) = crate::process::capture(&spec, timeout, 8 * 1024 * 1024)?;
                let parsed = artifact::strict(&bytes);
                match parsed {
                    Ok(value) if value["refresh"].is_object() => {
                        item["refresh"]["onnx_excerpts"] = value["stats"].clone();
                        (
                            code,
                            serde_json::to_vec(&value["refresh"])?,
                            started.elapsed().as_secs_f64(),
                        )
                    }
                    _ => {
                        item["refresh"]["onnx_excerpts"] = json!({"stats-unavailable":1});
                        (code, bytes, started.elapsed().as_secs_f64())
                    }
                }
            } else {
                r.runtime_at(&corpus, &args, &env, timeout, pinned.then_some(stamp))?
            };
            crate::process::ensure_custody()?;
            let onnx_stats = item["refresh"]["onnx_excerpts"].clone();
            item["refresh"] = json!({"exit":if exit==-1{Value::Null}else{json!(exit)},"seconds":seconds,"index_seconds":index_seconds,"index_exit":index_exit,"stderr_tail":""});
            if !onnx_stats.is_null() {
                item["refresh"]["onnx_excerpts"] = onnx_stats;
            }
            if exit != 0 {
                return Err(artifact::invalid(if exit == -1 {
                    "refresh-timeout"
                } else {
                    "refresh-error"
                }));
            }
            let refresh = artifact::strict(&raw)?;
            let store = infra_core::Store::open_read_only(
                corpus.join("memory-bank/local/context.db"),
                Duration::from_secs(1),
            )?;
            let mut reachable = BTreeSet::new();
            let mut long = BTreeSet::new();
            for path in &answers {
                if let Some(d) = infra_core::index::load_document(&store.conn, path)? {
                    let indexed = scoring::answer_flatten(&d.content);
                    if d.content.chars().count() > 65536 {
                        long.insert(path.clone());
                    }
                    for p in scoring::labelled(std::slice::from_ref(path), &ps) {
                        if indexed.contains(&p) {
                            reachable.insert(p);
                        }
                    }
                }
            }
            item["refresh"]["index"] = scoring::index_stats(&store)?;
            let score =
                scoring::score(&refresh, &g, &ps, &existed, &answers, &expected, &reachable)?;
            for (k, value) in score.as_object().unwrap() {
                item[k] = value.clone();
            }
            item["answer_passages_long_document"] = json!(long.len());
            item["refresh"]["phases"] = refresh["phases"].clone();
            item["refresh"]["warnings"] =
                json!(refresh["warnings"].as_array().map(Vec::len).unwrap_or(0));
            if let Some(d) = crate::diagnostic::normalize_funnel(&refresh["funnel_diagnosis"]) {
                item["refresh"]["funnel_diagnosis"] = d;
            }
            if let Some(d) = crate::diagnostic::normalize_receipt(&refresh["ordinary_receipt"]) {
                item["refresh"]["ordinary_receipt"] = d;
            }
            if v["diagnose_misses"] == true {
                let paths = g
                    .iter()
                    .filter(|(_, grade)| **grade >= 1)
                    .map(|(p, _)| p.clone())
                    .collect::<Vec<_>>();
                let passages = ps
                    .iter()
                    .filter(|(p, e)| g.get(*p).is_some_and(|g| *g >= 1) && e["useful"] != false)
                    .map(|(p, e)| {
                        (
                            p.clone(),
                            e["passages"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str)
                                .map(String::from)
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<BTreeMap<_, _>>();
                if v["audit_git"] == true {
                    let git = corpus.join(".git");
                    if let Ok(m) = std::fs::symlink_metadata(&git) {
                        if m.is_dir() && !m.file_type().is_symlink() {
                            std::fs::remove_dir_all(git)?;
                        } else {
                            std::fs::remove_file(git)?;
                        }
                    }
                }
                let anchor = pinned.then(|| stamp.to_rfc3339());
                let _clock = crate::clock::enter(anchor.as_deref())?;
                item["miss_audit"] = crate::diagnostic::audit(crate::diagnostic::Audit {
                    root: &corpus,
                    manifest: refresh["capsule"]["manifest"].as_str(),
                    paths: &paths,
                    query_withheld: refresh["query_withheld"] == true,
                    passages: &passages,
                    capsule: refresh["capsule_text"].as_str().unwrap_or(""),
                    history: p["prompt"].as_str(),
                    seed_git: v["audit_git"] == true,
                })?;
            }
            item["status"] = "ok".into();
            Ok(())
        })();
        if let Err(error) = &run {
            crate::process::record_custody_error(error);
        }
        if let Err(error) = crate::process::ensure_custody() {
            result["meta"]["cleanup_unconfirmed"] = true.into();
            result["meta"]["failure_code"] = "cleanup-unconfirmed".into();
            // Known evidence may be retained, but failure to persist it cannot
            // replace the first physical-custody error or trigger corpus cleanup.
            let _ = persist_eval(&out, &mut result);
            return Err(error);
        }
        if crate::cancel::interrupted() {
            result["meta"]["interrupted"] = true.into();
            result["planned"]["units"][id]["state"] = "interrupted".into();
            if corpus.exists() && v["keep"] != true {
                std::fs::remove_dir_all(&corpus)?;
            }
            persist_eval(&out, &mut result)?;
            break;
        }
        if let Err(error) = run {
            item["reason"] = error.message.into();
        } else {
            evaluated += 1;
        }
        if v["keep"] == true {
            item["corpus"] = json!(corpus);
        } else if corpus.exists() {
            std::fs::remove_dir_all(&corpus)?;
        }
        result["items"][id] = item;
        result["planned"]["units"][id]["state"] = "finished".into();
        persist_eval(&out, &mut result)?;
    }
    if !crate::cancel::interrupted() {
        result["meta"]["finished"] = reconstruction::now().into();
    }
    persist_eval(&out, &mut result)?;
    if v["keep"] == true {
        std::mem::forget(work);
    }
    let text = format!(
        "{}\nwrote {}\n",
        crate::reports::format(&[("result", out.as_path(), &result)], false)?,
        out.display()
    );
    let mut response = envelope(
        result,
        if crate::cancel::interrupted() {
            128 + crate::cancel::signal()
        } else if evaluated > 0 {
            0
        } else {
            1
        },
        text,
    );
    response["_cli"]["stderr"] = warnings.join("").into();
    Ok(response)
}
fn persist_eval(out: &Path, result: &mut Value) -> Result<()> {
    result["summary"] = scoring::summarize(&result["items"]);
    result["planned_delivery"] = planned_delivery(result);
    artifact::write_json(out, result)
}
pub fn planned_delivery(result: &Value) -> Value {
    let mut out = json!({"profile":"planned-delivery-v1","status":"unknown","unit_delivery_rate":null,"passage_recall":null});
    let Some(ids) = result["planned"]["ids"].as_array() else {
        return out;
    };
    let Some(units) = result["planned"]["units"].as_object() else {
        return out;
    };
    let Some(items) = result["items"].as_object() else {
        return out;
    };
    if result["planned"]["profile"] != "planned-delivery-v1"
        || units.len() != ids.len()
        || ids
            .iter()
            .filter_map(Value::as_str)
            .collect::<BTreeSet<_>>()
            .len()
            != ids.len()
    {
        return out;
    }
    let (
        mut admitted,
        mut source_known,
        mut source_excluded,
        mut useful_eligible,
        mut useful_delivered,
        mut answer_existed,
        mut answer_delivered,
        mut unattempted,
    ) = (0usize, 0usize, 0usize, 0usize, 0usize, 0u64, 0u64, 0usize);
    let mut bindings = BTreeSet::new();
    for id in ids {
        let Some(id) = id.as_str() else {
            return out;
        };
        let Some(u) = units.get(id) else {
            return out;
        };
        if u["id"] != id
            || !u["input_sha256"]
                .as_str()
                .is_some_and(|h| artifact::hex(h, 64) && bindings.insert(h))
            || !u["runtime_admitted"].is_boolean()
        {
            return out;
        }
        let state = u["state"].as_str().unwrap_or("");
        if ![
            "unattempted",
            "preparing",
            "runtime-admitted",
            "finished",
            "interrupted",
        ]
        .contains(&state)
            || (state == "finished") != items.contains_key(id)
        {
            return out;
        }
        unattempted += usize::from(state == "unattempted");
        admitted += usize::from(u["runtime_admitted"] == true);
        if let Some(item) = items.get(id)
            && (item["input_sha256"] != u["input_sha256"] || item["planned_id"] != id)
        {
            return out;
        }
        let opportunity = &u["source_opportunity"];
        if opportunity.is_null() {
            if u["runtime_admitted"] == true {
                return out;
            }
            if items.get(id).is_some_and(|i| {
                ["bad-timestamp", "no-project", "no-history"]
                    .contains(&i["reason"].as_str().unwrap_or(""))
            }) {
                source_excluded += 1;
            }
            continue;
        }
        if !artifact::exact(
            opportunity,
            &["useful_paths", "answer_paths", "answer_passages"],
        ) || u["runtime_admitted"] != true
            || ["useful_paths", "answer_paths", "answer_passages"]
                .iter()
                .any(|k| opportunity[*k].as_u64().is_none())
        {
            return out;
        }
        source_known += 1;
        useful_eligible += usize::from(opportunity["useful_paths"].as_u64().unwrap() > 0);
        answer_existed += opportunity["answer_passages"].as_u64().unwrap();
        if let Some(i) = items.get(id).filter(|i| i["status"] == "ok") {
            if i["existed_useful"].as_array().map(Vec::len)
                != opportunity["useful_paths"].as_u64().map(|n| n as usize)
                || i["answer_passages_existed"] != opportunity["answer_passages"]
                || i["answer_passages_delivered"]
                    .as_u64()
                    .is_none_or(|n| n > opportunity["answer_passages"].as_u64().unwrap())
            {
                return out;
            }
            useful_delivered += usize::from(
                i["useful_delivered"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty()),
            );
            answer_delivered += i["answer_passages_delivered"].as_u64().unwrap();
        }
    }
    let complete = result["meta"]["finished"].is_string()
        && result["meta"]["interrupted"] != true
        && items.len() == ids.len()
        && source_known == ids.len();
    out = json!({"profile":"planned-delivery-v1","status":if complete{"complete"}else{"incomplete"},"planned":ids.len(),"terminal":items.len(),"without_terminal":ids.len().saturating_sub(items.len()),"unattempted":unattempted,"runtime_admitted":admitted,"source_known":source_known,"source_excluded":source_excluded,"useful_eligible":useful_eligible,"useful_units_delivered":useful_delivered,"answer_passages_existed":answer_existed,"answer_passages_delivered":answer_delivered,"unit_delivery_rate":if complete&&useful_eligible>0{Some(useful_delivered as f64/useful_eligible as f64)}else{None},"passage_recall":if complete&&answer_existed>0{Some(answer_delivered as f64/answer_existed as f64)}else{None}});
    out
}
fn settings(v: &Value) -> Result<outcome::Settings> {
    outcome::Settings::new(
        strings(v, "arms", &["off", "capsule"]),
        number(v, "repeat", 1),
        number(v, "cap", 40),
        seconds(v, "timeout", 900.0)?.as_secs_f64(),
        required(v, "model")?.into(),
        s(v, "codex").unwrap_or("codex"),
    )
}
pub fn execute(operation: &str, v: &Value) -> Result<Value> {
    let _signals = crate::cancel::Scope::install()?;
    let op = artifact::strip_tail(operation, ".py").replace('_', "-");
    crate::cli_contract::validate(&op, v)?;
    let parts = op.split('/').collect::<Vec<_>>();
    let family = parts[0];
    let command = parts
        .get(1)
        .copied()
        .or_else(|| s(v, "command"))
        .unwrap_or("");
    let _clock = if v.get("eval_clock").is_some_and(|clock| !clock.is_null()) {
        if ![
            "memory-miss-audit",
            "memory-onnx-pilot",
            "memory-rerank-pilot",
        ]
        .contains(&family)
        {
            return Err(artifact::invalid(
                "explicit eval_clock is unsupported for this facade; use the controlled memory worker route",
            ));
        }
        crate::clock::request(v)?
    } else {
        None
    };
    match (family, command) {
        ("memory-eval", "run") => eval_run(v),
        ("memory-eval", "report") => {
            let file = path(v, "result")?;
            let first = artifact::read_json(&file, artifact::MAX_TREE)?;
            let other = s(v, "compare").map(expand);
            let second = other
                .as_ref()
                .map(|p| artifact::read_json(p, artifact::MAX_TREE))
                .transpose()?;
            let mut results = vec![("A", file.as_path(), &first)];
            if let (Some(p), Some(d)) = (other.as_ref(), second.as_ref()) {
                results.push(("B", p.as_path(), d));
            }
            let text = format!(
                "{}\n",
                crate::reports::format(&results, v["show_unjudged"] == true)?
            );
            let value = json!({"result":first,"comparison":second.as_ref().map(|b|crate::reports::compatibility(&first,b))});
            Ok(envelope(value, 0, text))
        }
        ("memory-eval", "install-status") => {
            let source = source(v)?;
            let mut public = BTreeMap::new();
            let mut failed = false;
            for p in projects(v)? {
                let mut name = p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                while public.contains_key(&name) {
                    name.push('\'');
                }
                let status = field::install_status(&p, &source)?;
                failed |= status["problems"].as_array().is_some_and(|a| !a.is_empty());
                public.insert(name, status);
            }
            render_output(
                json!({"projects":public}),
                if failed { 1 } else { 0 },
                v,
                crate::reports::install_status,
            )
        }
        ("memory-eval", "realized") => {
            let p = projects(v)?.remove(0);
            let state = s(v, "state").map(expand).unwrap_or_else(|| p.clone());
            let (cc, cx) = roots(v);
            render_output(
                field::realized_options(
                    &p,
                    &state,
                    &cc,
                    &cx,
                    since(v, "since")?,
                    v["paths"] == true,
                    v["traces"] == true,
                )?,
                0,
                v,
                crate::reports::realized,
            )
        }
        ("memory-eval", "field-check") => {
            let projects = projects(v)?;
            let (cc, cx) = roots(v);
            let since = since(v, "since")?;
            let mut turns: BTreeMap<String, Vec<transcript::Turn>> =
                BTreeMap::from([("claude".into(), vec![]), ("codex".into(), vec![])]);
            let mut retrieved: BTreeMap<String, Vec<transcript::Retrievals>> =
                BTreeMap::from([("claude".into(), vec![]), ("codex".into(), vec![])]);
            let mut costs: BTreeMap<String, Vec<Value>> =
                BTreeMap::from([("claude".into(), vec![]), ("codex".into(), vec![])]);
            let mut unread = 0;
            let mut hooks = BTreeMap::<String, u64>::new();
            for p in &projects {
                let s = field::sessions(p, &cc, &cx, since)?;
                unread += s.compressed_unread;
                turns.get_mut("claude").unwrap().extend(s.claude);
                turns.get_mut("codex").unwrap().extend(s.codex);
                retrieved
                    .get_mut("claude")
                    .unwrap()
                    .extend(s.claude_retrievals);
                retrieved
                    .get_mut("codex")
                    .unwrap()
                    .extend(s.codex_retrievals);
                costs.get_mut("claude").unwrap().extend(s.claude_costs);
                costs.get_mut("codex").unwrap().extend(s.codex_costs);
                for ((_, outcome), n) in s.hooks {
                    *hooks.entry(outcome).or_default() += n;
                }
            }
            let hosts = json!({"claude":field::field_counts(&turns["claude"],&retrieved["claude"],&costs["claude"]),"codex":field::field_counts(&turns["codex"],&retrieved["codex"],&costs["codex"])});
            for key in ["ok", "error", "not_found", "blocked"] {
                hooks.entry(key.into()).or_default();
            }
            let minimum = number(v, "min_turns", 50);
            render_output(
                json!({"since":v["since"],"projects":projects.len(),"min_turns":minimum,"hosts":hosts,"rule":field::field_rule(&hosts,minimum),"claude_hooks":hooks,"no_capsule_turns":{"claude":turns["claude"].iter().filter(|t|t.unassembled&&!t.output).count(),"codex":turns["codex"].iter().filter(|t|t.unassembled&&!t.output).count()},"compressed_unread":unread}),
                0,
                v,
                crate::reports::field_check,
            )
        }
        ("memory-eval", "funnel") => {
            let mut results = BTreeMap::new();
            for p in projects(v)? {
                let mut name = p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                while results.contains_key(&name) {
                    name.push('\'');
                }
                results.insert(name, field::funnel(&p, since(v, "since")?));
            }
            let mut total = json!({});
            for result in results.values() {
                for (k, value) in result.as_object().unwrap() {
                    if let Some(n) = value.as_u64() {
                        total[k] = json!(total[k].as_u64().unwrap_or(0) + n);
                    }
                }
            }
            for (field, series) in [("median_tokens", "tokens"), ("median_bytes", "bytes")] {
                let values = results
                    .values()
                    .flat_map(|r| r[series].as_array().into_iter().flatten())
                    .filter_map(Value::as_f64)
                    .collect::<Vec<_>>();
                total[field] = json!(cost::median(&values).map(f64::round));
            }
            for key in ["profiles", "hosts"] {
                total[key] = json!({});
                for r in results.values() {
                    for (k, n) in r[key].as_object().into_iter().flatten() {
                        total[key][k] =
                            json!(total[key][k].as_u64().unwrap_or(0) + n.as_u64().unwrap_or(0));
                    }
                }
            }
            total["save_reasons"] = json!({});
            for r in results.values() {
                for (stage, codes) in r["save_reasons"].as_object().into_iter().flatten() {
                    if total["save_reasons"].get(stage).is_none() {
                        total["save_reasons"][stage] = json!({});
                    }
                    for (code, n) in codes.as_object().into_iter().flatten() {
                        total["save_reasons"][stage][code] = json!(
                            total["save_reasons"][stage][code].as_u64().unwrap_or(0)
                                + n.as_u64().unwrap_or(0)
                        );
                    }
                }
            }
            for r in results.values_mut() {
                r.as_object_mut().unwrap().remove("tokens");
                r.as_object_mut().unwrap().remove("bytes");
            }
            render_output(
                json!({"since":v["since"],"projects":results,"total":total}),
                0,
                v,
                crate::reports::funnel,
            )
        }
        ("memory-outcome-ab", "seal") => {
            let mut preparation = preparation(v)?;
            let settings = settings(v)?;
            let design = artifact::read_json(&path(v, "design")?, 1048576)?;
            let source = preparation.reconstruction.source.clone();
            let cache = preparation.reconstruction.cache.clone();
            let value = outcome::seal(
                &path(v, "set")?,
                &path(v, "verifiers")?,
                &design,
                &path(v, "out")?,
                &cache,
                &settings,
                &mut preparation,
                &source,
            )?;
            Ok(envelope(
                value,
                0,
                "sealed protocol and prepared corpora; no model was called\n".into(),
            ))
        }
        ("memory-outcome-ab", "run") => {
            let mut preparation = preparation(v)?;
            let value = if s(v, "endpoint").unwrap_or("first-touch") == "task-complete" {
                if v.get("ids").is_some() || v.get("events").is_some() || v["keep"] == true {
                    return Err(artifact::invalid(
                        "sealed completion refuses ids, raw events and keep",
                    ));
                }
                let settings = settings(v)?;
                let set = path(v, "set")?;
                let verifiers = path(v, "verifiers")?;
                let protocol = path(v, "protocol")?;
                let out = path(v, "out")?;
                let private = common_private(
                    &[&set, &verifiers, &protocol, &out],
                    &preparation.reconstruction.source,
                )?;
                outcome::run_complete(
                    outcome::CompletionRun {
                        set: &set,
                        verifiers: &verifiers,
                        protocol: &protocol,
                        out: &out,
                        cache: &preparation.reconstruction.cache,
                        private: &private,
                        source: &preparation.reconstruction.source,
                        codex: s(v, "codex").unwrap_or("codex"),
                        executable: &preparation.reconstruction.runtime,
                        settings: &settings,
                    },
                    &preparation,
                )?
            } else {
                let document = artifact::read_json(&path(v, "set")?, artifact::MAX_TREE)?;
                let selected = strings(v, "ids", &[]);
                let mut document = document;
                if !selected.is_empty() {
                    document["tasks"]
                        .as_array_mut()
                        .ok_or_else(|| artifact::invalid("tasks required"))?
                        .retain(|t| {
                            t["id"]
                                .as_str()
                                .is_some_and(|id| selected.iter().any(|s| s == id))
                        });
                }
                outcome::run_legacy(
                    &document,
                    &path(v, "out")?,
                    &strings(v, "arms", &["off", "capsule"]),
                    number(v, "repeat", 1),
                    number(v, "cap", 40),
                    seconds(v, "timeout", 900.0)?,
                    s(v, "model"),
                    s(v, "codex").unwrap_or("codex"),
                    s(v, "events").map(expand).as_deref(),
                    v["keep"] == true,
                    &mut preparation,
                )?
            };
            json_output(
                value,
                if crate::cancel::interrupted() {
                    128 + crate::cancel::signal()
                } else {
                    0
                },
            )
        }
        ("memory-outcome-ab", "report") => {
            let result = artifact::read_json(&path(v, "result")?, artifact::MAX_TREE)?;
            let completion = s(v, "metric") == Some("task-complete")
                || result["schema"] == "task-outcome-complete-v1";
            let report = if completion {
                outcome::summarize_complete(&result)?
            } else {
                let compare = strings(v, "compare", &["off", "capsule"]);
                if compare.len() != 2
                    || compare[0] == compare[1]
                    || compare.iter().any(|a| !outcome::ARMS.contains(&a.as_str()))
                {
                    return Err(artifact::invalid("two different comparison arms required"));
                }
                outcome::summarize_legacy(&result, [&compare[0], &compare[1]], s(v, "metric"))?
            };
            if completion || v["json"] == true {
                json_output(report, 0)
            } else {
                let text = format!("{}\n", crate::reports::legacy_outcome(&report));
                Ok(envelope(report, 0, text))
            }
        }
        ("memory-outcome-ab", "sample" | "recall-sources") => {
            let projects_root = s(v, "projects_root")
                .map(expand)
                .unwrap_or_else(|| home().join("Desktop"));
            let projects = strings(v, "projects", &[])
                .iter()
                .map(|p| projects_root.join(p))
                .collect::<Vec<_>>();
            let (cc, cx) = roots(v);
            let hosts = strings(v, "hosts", &["claude"]);
            if hosts.is_empty()
                || hosts
                    .iter()
                    .any(|h| !["claude", "codex"].contains(&h.as_str()))
            {
                return Err(artifact::invalid("hosts are claude and codex"));
            }
            let before =
                since(v, "before")?.ok_or_else(|| artifact::invalid("--before required"))?;
            let after = since(v, "after")?;
            let mut excludes = BTreeMap::<String, Vec<chrono::DateTime<chrono::Utc>>>::new();
            for p in strings(v, "exclude_set", &[]) {
                let set = artifact::read_json(&expand(&p), artifact::MAX_TREE)?;
                for entry in set
                    .as_array()
                    .into_iter()
                    .flatten()
                    .chain(set["tasks"].as_array().into_iter().flatten())
                {
                    if let (Some(project), Some(at)) = (
                        entry["project"].as_str(),
                        entry["ts"].as_str().and_then(reconstruction::instant),
                    ) {
                        excludes.entry(project.into()).or_default().push(at);
                    }
                }
            }
            let selection = sampling::Selection {
                projects: &projects,
                claude: &cc,
                codex: &cx,
                hosts: &hosts,
                before,
                after,
                excludes: &excludes,
            };
            let value = if command == "sample" {
                sampling::sample(
                    &selection,
                    v.get("n")
                        .ok_or_else(|| artifact::invalid("--n required"))?
                        .as_u64()
                        .map(|n| n as usize)
                        .or_else(|| s(v, "n").and_then(|s| s.parse().ok()))
                        .ok_or_else(|| artifact::invalid("invalid sample count"))?,
                    &seed(v)?,
                    v["require_ticket_commits"] == true,
                )?
            } else {
                sampling::recall_sources(&selection)?
            };
            let out = path(v, "out")?;
            if artifact::overlaps(&out, &source(v)?)? {
                return Err(artifact::invalid(
                    "private sample output outside clone required",
                ));
            }
            artifact::write_json(&out, &value)?;
            json_output(value, 0)
        }
        ("memory-outcome-ab", "recall-set") => {
            let value = sampling::recall_set(
                &artifact::read_json(&path(v, "sources")?, artifact::MAX_TREE)?,
                &artifact::read_json(&path(v, "scenarios")?, artifact::MAX_TREE)?,
                &s(v, "projects_root")
                    .map(expand)
                    .unwrap_or_else(|| home().join("Desktop")),
                s(v, "regime").unwrap_or("next"),
                &strings(v, "exclude_id", &[]).into_iter().collect(),
                number(v, "n", 0),
                &seed(v)?,
                &source(v)?,
            )?;
            let out = path(v, "out")?;
            if artifact::overlaps(&out, &source(v)?)? {
                return Err(artifact::invalid(
                    "private recall output outside clone required",
                ));
            }
            artifact::write_json(&out, &value)?;
            json_output(value, 0)
        }
        ("memory-native-ab", "plan" | "run" | "inspect") => {
            let prep = preparation(v)?;
            let value = match command {
                "plan" => {
                    native::plan(&path(v, "config")?, &path(v, "out")?, &prep.reconstruction)?
                }
                "run" => native::run(
                    &path(v, "run")?,
                    s(v, "stage").unwrap_or("both"),
                    &prep.reconstruction,
                )?,
                _ => native::summary(&native::load_run(&path(v, "run")?)?.2)?,
            };
            let code = if crate::cancel::interrupted() {
                128 + crate::cancel::signal()
            } else if command != "run"
                || value["stages"]["served"] == "consulted"
                || s(v, "stage") == Some("source") && value["stages"]["source"] == "captured"
            {
                0
            } else {
                1
            };
            json_output(value, code)
        }
        ("codex-bench", _) => crate::bench_driver::run(&source(v)?, &runtime(v)?, "codex", v),
        ("claude-code-bench", _) => {
            crate::bench_driver::run(&source(v)?, &runtime(v)?, "claude", v)
        }
        ("memory-graph-pilot", _) => crate::graph::run(&source(v)?, &runtime(v)?, v),
        ("memory-selective-retrieve-pilot", _) => {
            let code = treatments::selective_stream(
                required(v, "hook")?,
                v["enabled"] == true,
                s(v, "rules").map(expand).as_deref(),
                s(v, "audit").map(expand).as_deref(),
            )?;
            Ok(envelope(Value::Null, code, String::new()))
        }
        ("memory-throttle-oracle-v2", _) => crate::throttle_oracle::run(v),
        ("memory-onnx-pilot", _) => crate::onnx_adapter::pilot(v),
        ("memory-current-facts", _) => json_output(
            crate::facts::project(
                &path(v, "root")?,
                &artifact::read_json(&path(v, "selectors")?, 1048576)?,
                required(v, "as_of")?,
            )?,
            0,
        ),
        ("memory-fact-consolidation", _) => {
            let result = crate::facts::resolve_facts(&artifact::read_json(
                &path(v, "input")?,
                artifact::MAX_TREE,
            )?)?;
            if v["json"] == true {
                json_output(result, 0)
            } else {
                let lines = result["groups"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|g| {
                        let names = |key: &str| {
                            g[key]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(", ")
                        };
                        if g["ambiguous"] == true {
                            format!(
                                "{}: ambiguous ({})",
                                g["fact"].as_str().unwrap_or(""),
                                names("candidates")
                            )
                        } else {
                            let superseded = names("superseded");
                            format!(
                                "{}: {} (supersedes {})",
                                g["fact"].as_str().unwrap_or(""),
                                g["winner"].as_str().unwrap_or(""),
                                if superseded.is_empty() {
                                    "nothing"
                                } else {
                                    &superseded
                                }
                            )
                        }
                    })
                    .collect::<Vec<_>>();
                Ok(envelope(result, 0, format!("{}\n", lines.join("\n"))))
            }
        }
        ("memory-freshness-plan", _) => {
            let root = path(v, "root")?;
            let source = source(v)?;
            let limit = number(v, "limit", 50);
            let chunks = strings(v, "chunk", &[]);
            let result = if let Some(packet) = s(v, "packet") {
                if !v["decisions"].is_string()
                    || !v["export"].is_string()
                    || v["evidence"] == true
                    || !chunks.is_empty()
                {
                    return Err(artifact::invalid(
                        "replay requires packet, decisions, export",
                    ));
                }
                crate::freshness::replay(
                    &source,
                    &root,
                    &artifact::read_json(&expand(packet), 1048576)?,
                    &artifact::read_json(&path(v, "decisions")?, 131072)?,
                    &path(v, "export")?,
                )?
            } else if v["evidence"] == true {
                if v["decisions"].is_string() || v["export"].is_string() {
                    return Err(artifact::invalid("unexpected replay argument"));
                }
                crate::freshness::capture(
                    &root,
                    (!chunks.is_empty()).then_some(chunks.as_slice()),
                    limit,
                    s(v, "owner").unwrap_or("local"),
                )?
            } else {
                if !chunks.is_empty() || v["decisions"].is_string() || v["export"].is_string() {
                    return Err(artifact::invalid("explicit evidence or replay required"));
                }
                crate::freshness::plan(&root, limit)?
            };
            if let Some(out) = s(v, "out") {
                let out = expand(out);
                if artifact::overlaps(&out, &root)? || artifact::overlaps(&out, &source)? {
                    return Err(artifact::invalid(
                        "private report outside input/tooling required",
                    ));
                }
                artifact::write_json(&out, &result)?;
                Ok(envelope(result, 0, String::new()))
            } else {
                json_output(result, 0)
            }
        }
        ("memory-rollout-rehearsal", _) => crate::rehearsal::run(&source(v)?, &runtime(v)?, v),
        ("memory-lifecycle-fuzz", _) => crate::lifecycle::run(&source(v)?, &runtime(v)?, v),
        ("memory-miss-audit", _) => {
            use std::io::Read;
            let mut raw = vec![];
            if let Some(input) = s(v, "_stdin") {
                raw = input.as_bytes().to_vec();
            } else {
                std::io::stdin()
                    .take(artifact::MAX_TREE as u64 + 1)
                    .read_to_end(&mut raw)?;
            }
            let private = artifact::strict(&raw)?;
            let paths = artifact::strict(required(v, "paths_json")?.as_bytes())?
                .as_array()
                .ok_or_else(|| artifact::invalid("paths list required"))?
                .iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect::<Vec<_>>();
            let passages = private["answer_passages"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.iter())
                .map(|(p, a)| {
                    (
                        p.clone(),
                        a.as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(String::from)
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            let _clock = crate::clock::request(v)?;
            json_output(
                crate::diagnostic::audit(crate::diagnostic::Audit {
                    root: &path(v, "corpus")?,
                    manifest: s(v, "manifest"),
                    paths: &paths,
                    query_withheld: v["query_withheld"] == true,
                    passages: &passages,
                    capsule: private["capsule_text"].as_str().unwrap_or(""),
                    history: private["history_text"].as_str(),
                    seed_git: v["audit_git"] == true,
                })?,
                0,
            )
        }
        ("memory-consolidation-pilot", _) => {
            let preparation = preparation(v)?;
            let value = preparation.with_engine(|engine| {
                treatments::consolidation_with_engine(
                    &preparation.reconstruction.source,
                    &path(v, "root")?,
                    required(v, "variant")?,
                    s(v, "owner").unwrap_or("local"),
                    s(v, "export").map(expand).as_deref(),
                    engine,
                )
            })?;
            artifact::write_json(&path(v, "out")?, &value)?;
            json_output(value, 0)
        }
        ("memory-rerank-pilot", _) => {
            let queries = artifact::read_json(&path(v, "queries")?, artifact::MAX_TREE)?;
            let judgments = s(v, "judgments")
                .map(|p| artifact::read_json(&expand(p), artifact::MAX_TREE))
                .transpose()?
                .unwrap_or_else(|| json!({}));
            let result = treatments::rerank_with_at(
                &path(v, "database")?,
                &queries,
                &judgments,
                number(v, "limit", 3).max(1),
                s(v, "runtime").map(expand).as_deref(),
                s(v, "eval_clock"),
            )?;
            if v["json"] == true {
                json_output(result, 0)
            } else {
                let mut lines = result["rows"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|r| {
                        format!(
                            "{}: baseline {} useful, reranked {} useful{}",
                            r["id"].as_str().unwrap_or(""),
                            r["baseline"]["useful"],
                            r["reranked"]["useful"],
                            if r["changed"] == true {
                                " (changed)"
                            } else {
                                ""
                            }
                        )
                    })
                    .collect::<Vec<_>>();
                lines.push(format!(
                    "{} queries: useful {} -> {} at k={}",
                    result["queries"],
                    result["baseline_useful"],
                    result["reranked_useful"],
                    result["limit"]
                ));
                Ok(envelope(result, 0, format!("{}\n", lines.join("\n"))))
            }
        }
        ("cost-attribution", _) => {
            let root = s(v, "root")
                .map(expand)
                .unwrap_or_else(|| home().join(".claude/projects"));
            let mut result = cost::attribute_full(&root, s(v, "project"))?;
            let dimensions = strings(v, "by", &[]);
            if dimensions
                .iter()
                .any(|d| !cost::DIMENSIONS.iter().any(|(k, _)| *k == d))
            {
                return Err(artifact::invalid("invalid attribution dimension"));
            }
            let rendered = cost::render_attribution(
                &result,
                &root,
                &dimensions,
                v["top"]
                    .as_i64()
                    .or_else(|| s(v, "top").and_then(|s| s.parse().ok()))
                    .unwrap_or(15) as isize,
            );
            for key in [
                "_projects",
                "_project_calls",
                "_stratum_calls",
                "_dimension_calls",
            ] {
                result.as_object_mut().unwrap().remove(key);
            }
            if v["json"] == true {
                json_output(result, 0)
            } else {
                Ok(envelope(result, 0, format!("{rendered}\n")))
            }
        }
        _ => Err(artifact::invalid(
            "operation is outside this evaluation command registry",
        )),
    }
}
fn common_private(paths: &[&Path], source: &Path) -> Result<PathBuf> {
    let mut root = artifact::ordinary(paths[0])?
        .parent()
        .ok_or_else(|| artifact::invalid("private parent"))?
        .to_path_buf();
    for p in paths {
        let p = artifact::ordinary(p)?;
        if artifact::overlaps(&p, source)? {
            return Err(artifact::invalid("private input lies inside tooling clone"));
        }
        while !p.parent().is_some_and(|p| p.starts_with(&root)) {
            if !root.pop() {
                return Err(artifact::invalid(
                    "private inputs must share external directory",
                ));
            }
        }
    }
    if root.parent().is_none() {
        return Err(artifact::invalid(
            "private inputs must share directory below filesystem root",
        ));
    }
    Ok(root)
}
