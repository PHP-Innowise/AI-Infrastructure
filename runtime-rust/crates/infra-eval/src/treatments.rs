use crate::executable;
use crate::{Result, artifact, process};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::{Duration, Instant},
};
pub fn rules() -> Value {
    json!({"version":2,"historical":["раньше","ранее","прежде","в прошлом","до изменения","previous","previously","earlier","former","formerly","before","used to","old","as of"],"decisions":["отказались","решили","выбрали","приняли решение","rejected","decided","chose","did we choose","did we reject"],"inquiries":["какой","какая","какое","какие","что","как","почему","зачем","когда","найди","вспомни","what","which","how","why","when","recover","recall","find","explain"],"current":["сейчас","теперь","текущий","актуальный","current","currently","today"],"procedural":["как выполнять","порядок действий","процедура","инструкция","how to","workflow","procedure"],"continuations":["дальше","продолжай","продолжи","делай","сделай","continue","go ahead"]})
}
fn folded(s: &str) -> String {
    format!(" {} ", infra_core::query::word_tokens(s).join(" "))
}
pub fn route(query: &str, rules: &Value) -> Result<(&'static str, &'static str)> {
    if query.chars().count() > 24000 {
        return Err(artifact::invalid("invalid routing query"));
    }
    let version = rules["version"]
        .as_u64()
        .filter(|v| *v == 1 || *v == 2)
        .ok_or_else(|| artifact::invalid("invalid routing rules"))?;
    let groups = if version == 1 {
        vec!["historical", "decisions"]
    } else {
        vec![
            "historical",
            "decisions",
            "inquiries",
            "current",
            "procedural",
            "continuations",
        ]
    };
    if rules.as_object().map(|m| m.len()) != Some(groups.len() + 1) {
        return Err(artifact::invalid("invalid routing rule keys"));
    }
    for name in &groups {
        if !rules[*name].as_array().is_some_and(|a| {
            !a.is_empty()
                && a.len() <= 64
                && a.iter().all(|v| {
                    v.as_str().is_some_and(|s| {
                        !s.trim().is_empty()
                            && s.chars().count() <= 80
                            && s.chars()
                                .all(|c| c.is_alphanumeric() || c == '_' || c == ' ' || c == '-')
                    })
                })
        }) {
            return Err(artifact::invalid("invalid bounded phrase list"));
        }
    }
    let q = folded(query);
    let matches = |name: &str| {
        rules[name]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .any(|s| q.contains(&folded(s)))
    };
    if version == 1 {
        return Ok(if matches("historical") && matches("decisions") {
            ("full", "historical-decision")
        } else {
            ("recall", "ordinary")
        });
    }
    if rules["continuations"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .any(|s| q == folded(s))
    {
        return Ok(("recall", "continuation"));
    }
    let inquiry = query.contains('?')
        || rules["inquiries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .any(|s| q.starts_with(&folded(s)));
    Ok(
        if inquiry && (matches("historical") || matches("decisions")) {
            (
                "full",
                if matches("decisions") {
                    "historical-decision"
                } else {
                    "historical-state"
                },
            )
        } else if matches("procedural") {
            ("recall", "procedural")
        } else if inquiry && matches("current") {
            ("recall", "current-state")
        } else {
            ("recall", "ordinary")
        },
    )
}
pub fn observations(raw: &[u8], expected: &Value) -> Value {
    let unknown = || json!({"status":"unknown","records":[],"producer_authenticated":false});
    if raw.len() > 1048576 {
        return unknown();
    }
    let records = raw
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .map(artifact::strict)
        .collect::<Result<Vec<_>>>();
    let Ok(records) = records else {
        return unknown();
    };
    if records.is_empty() || records.len() > 1000 {
        return unknown();
    }
    for r in &records {
        if !artifact::exact(
            r,
            &[
                "schema",
                "profile",
                "reason",
                "rules_digest",
                "router_digest",
                "sanitizer_digest",
                "routing_ns",
                "hook_exit",
            ],
        ) || !["memory-selective-route-v1", "memory-selective-route-v2"]
            .contains(&r["schema"].as_str().unwrap_or(""))
            || !["recall", "full"].contains(&r["profile"].as_str().unwrap_or(""))
            || ![
                "historical-state",
                "historical-decision",
                "current-state",
                "procedural",
                "continuation",
                "ordinary",
                "routing-unavailable",
            ]
            .contains(&r["reason"].as_str().unwrap_or(""))
            || r["routing_ns"].as_u64().is_none()
            || r["hook_exit"].as_i64().is_none()
        {
            return unknown();
        }
        if r["reason"].as_str().unwrap().starts_with("historical-") != (r["profile"] == "full") {
            return unknown();
        }
        if let Some(expected) = expected.as_object() {
            for (k, v) in expected {
                if &r[k] != v
                    && !(r["reason"] == "routing-unavailable"
                        && k != "router_digest"
                        && r[k].is_null())
                {
                    return unknown();
                }
            }
        }
    }
    json!({"status":"observed","records":records,"producer_authenticated":false})
}
pub fn selective_hook(
    runtime: &Path,
    command: &str,
    enabled: bool,
    rule_path: Option<&Path>,
    audit: Option<&Path>,
    raw: &[u8],
) -> Result<i32> {
    let mut env = std::env::vars().collect::<BTreeMap<String, String>>();
    let mut record = None;
    if enabled {
        let started = Instant::now();
        let rule = rule_path.map(|p| artifact::read_json(p, 16384)).transpose();
        let mut r = json!({"schema":"memory-selective-route-v2","profile":"recall","reason":"routing-unavailable","rules_digest":null,"router_digest":artifact::sha(include_bytes!("treatments.rs")),"sanitizer_digest":executable::digest(&std::env::current_exe()?)?});
        if raw.len() <= 1048576
            && let (Ok(payload), Ok(Some(rule))) = (artifact::strict(raw), rule)
            && let Some(query) = payload["prompt"].as_str()
        {
            let chars = query.chars().collect::<Vec<_>>();
            let query = if chars.len() > 24000 {
                format!(
                    "{}\n{}",
                    chars[..15999].iter().collect::<String>(),
                    chars[chars.len() - 8000..].iter().collect::<String>()
                )
            } else {
                query.to_string()
            };
            let safe = infra_core::domain::sanitize_automatic_query(&query)?;
            if let Ok((profile, reason)) = route(&safe, &rule) {
                r["profile"] = profile.into();
                r["reason"] = reason.into();
                r["rules_digest"] = artifact::digest(&rule)?.into();
            }
        }
        env.insert(
            "CONTEXT_CAPSULE_PROFILE".into(),
            r["profile"].as_str().unwrap().into(),
        );
        r["routing_ns"] = json!(started.elapsed().as_nanos() as u64);
        record = Some(r);
    }
    let shell = crate::isolation::which("bash").unwrap_or_else(|| "/bin/sh".into());
    let spec = process::CommandSpec {
        argv: vec![shell.to_string_lossy().into(), "-c".into(), command.into()],
        cwd: std::env::current_dir()?,
        env,
        input: raw.to_vec(),
    };
    let (code, stdout) = process::capture(&spec, Duration::from_secs(900), 8 * 1024 * 1024)?;
    use std::io::Write;
    std::io::stdout().write_all(&stdout)?;
    if let (Some(mut r), Some(path)) = (record, audit) {
        r["hook_exit"] = code.into();
        if artifact::ordinary(path).is_ok()
            && std::fs::metadata(path)
                .map(|m| m.len() <= 1048576)
                .unwrap_or(true)
        {
            let mut raw = infra_core::domain::json::canonical(&r)?.into_bytes();
            raw.push(b'\n');
            let _ = infra_core::fs::append_whole_line(path, &raw);
            let _ = artifact::chmod(path, 0o600);
        }
    }
    let _ = runtime;
    Ok(code)
}
fn configured(root: &Path, variant: &str) -> Result<()> {
    let p = root.join("project-brain/config/runtime.json");
    let mut config = if p.is_file() {
        artifact::read_json(&p, artifact::MAX_FILE)?
    } else {
        json!({})
    };
    config["automatic_consolidation"] = (variant != "baseline").into();
    config["automatic_consolidation_verification"] = (variant == "consolidate-verify").into();
    artifact::write_json(&p, &config)
}
pub fn consolidation(
    source: &Path,
    root: &Path,
    variant: &str,
    owner: &str,
    export: Option<&Path>,
) -> Result<Value> {
    consolidation_inner(source, root, variant, owner, export, None)
}
pub(crate) fn consolidation_with_engine(
    source: &Path,
    root: &Path,
    variant: &str,
    owner: &str,
    export: Option<&Path>,
    engine: &executable::CanonicalEngine<'_>,
) -> Result<Value> {
    consolidation_inner(source, root, variant, owner, export, Some(engine))
}
fn consolidation_inner(
    source: &Path,
    root: &Path,
    variant: &str,
    owner: &str,
    export: Option<&Path>,
    engine: Option<&executable::CanonicalEngine<'_>>,
) -> Result<Value> {
    let snapshot = |path: &Path| {
        if let Some(engine) = engine {
            artifact::image_with_canonical_engine(path, artifact::ImageKind::Public, None, engine)
        } else {
            artifact::image(path, artifact::ImageKind::Public, None)
        }
    };
    if !["baseline", "consolidate", "consolidate-verify"].contains(&variant) {
        return Err(artifact::invalid("unknown consolidation variant"));
    }
    let before = snapshot(root)?;
    let digest = artifact::image_digest(&before, false)?;
    let temp = artifact::Scratch::new(&std::env::temp_dir(), "memory-consolidation")?;
    let copy = temp.0.join("snapshot");
    artifact::copy_image(&before, &copy)?;
    configured(&copy, variant)?;
    let schemas = copy.join("project-brain/schemas");
    if schemas.exists() {
        std::fs::remove_dir_all(&schemas)?;
    }
    let schema_rows = artifact::image(
        &source.join("PHP Core/project-brain/schemas"),
        artifact::ImageKind::Physical,
        None,
    )?;
    artifact::copy_image(&schema_rows, &schemas)?;
    let ctx =
        infra_core::RuntimeContext::new(infra_core::Roots::installed(&copy)?, owner, "governed")?;
    let bank = infra_core::BankService::new(ctx.clone());
    let planned = if variant == "baseline" {
        json!({})
    } else {
        bank.consolidate(false, None)?
    };
    let mut boundaries = vec![];
    let mut last = String::new();
    for _ in 0..3 {
        last = artifact::image_digest(&snapshot(&copy)?, false)?;
        let c = bank.auto_consolidate()?;
        let p = bank.auto_promote(3, None, None)?;
        let compact = bank.auto_compact()?;
        boundaries.push(json!({"created":c["created"],"updated":c["updated"],"superseded":c["superseded"],"promoted":p["promoted"].as_array().map(Vec::len).unwrap_or(0),"promotion_failed":p["failed"].as_array().map(Vec::len).unwrap_or(0),"promotion_blocked":p["blocked"].as_array().map(Vec::len).unwrap_or(0),"compacted":compact["moved"],"failed":!c["error"].is_null()&&!c["error"].as_str().is_some_and(str::is_empty)||!compact["error"].is_null()&&!compact["error"].as_str().is_some_and(str::is_empty)}));
    }
    let brain = infra_core::BrainRepository::new(ctx)?;
    let inventory = brain.inventory(true)?;
    let promotions = infra_core::bank::promotion::list(&bank, false)?;
    let mut mappings = vec![];
    let event_expression = regex::Regex::new(r"\[event ([a-f0-9-]+)\]");
    for r in inventory.records {
        let r = r.record;
        if r["type"] == "finding"
            && r["external_id"]
                .as_str()
                .is_some_and(|id| id.starts_with("CONSOLIDATED-"))
        {
            let chunks = promotions
                .iter()
                .filter(|p| {
                    p["status"] == "applied"
                        && p["source_records"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .any(|s| s["id"] == r["id"] && s["revision"] == r["revision"])
                })
                .map(|p| p["destination_memory_id"].clone())
                .collect::<Vec<_>>();
            let events = event_expression
                .as_ref()
                .map_err(|_| artifact::invalid("event expression"))?
                .captures_iter(r["progress"].as_str().unwrap_or(""))
                .map(|m| m[1].to_string())
                .collect::<BTreeSet<_>>();
            mappings.push(json!({"record":r["id"],"revision":r["revision"],"chunks":chunks,"events":events,"authority":r["authority"],"attestation":r["attestation"]}));
        }
    }
    mappings.sort_by(|a, b| a["record"].as_str().cmp(&b["record"].as_str()));
    let mut errors = vec!["brain-validation"; inventory.problems.len()];
    errors.extend(vec!["bank-validation"; bank.validate()?.len()]);
    if artifact::image_digest(&snapshot(root)?, false)? != digest {
        return Err(artifact::invalid(
            "input changed during consolidation replay",
        ));
    }
    let mut out = json!({"schema":"memory-consolidation-pilot-v1","variant":variant,"input_digest":digest,"boundaries":boundaries,"mappings":mappings,"planned_tickets":planned["tickets"].as_array().map(Vec::len).unwrap_or(0),"idempotent":last==artifact::image_digest(&snapshot(&copy)?,false)?,"validation_errors":errors,"evidence":"lifecycle-replay","task_outcome":"not-measured","human_reviewed":false,"recipe":{"boundaries":3,"owner":owner,"variant":variant},"runtime_digest":artifact::digest(&json!([artifact::sha(include_bytes!("treatments.rs")),schema_rows.iter().map(|f|json!([f.path,artifact::sha(&f.bytes)])).collect::<Vec<_>>(),artifact::sha(include_bytes!("../../infra-core/src/bank/maintenance.rs"))]))?});
    if let Some(export) = export {
        if export.exists()
            || artifact::overlaps(export, root)?
            || artifact::overlaps(export, source)?
        {
            return Err(artifact::invalid(
                "fresh export outside source and input required",
            ));
        }
        let image = snapshot(&copy)?;
        artifact::copy_image(&image, export)?;
        out["export_digest"] = artifact::image_digest(&image, false)?.into();
    }
    Ok(out)
}
fn tokens(s: &str) -> BTreeSet<String> {
    let stop = [
        "the", "and", "for", "with", "from", "that", "this", "how", "does", "what", "where",
        "when", "why", "are", "was", "were", "has", "have", "had", "you", "your", "into", "over",
        "under", "about", "its", "not",
    ];
    regex::Regex::new(r"[a-z0-9]{3,}")
        .map(|r| {
            r.find_iter(&s.to_lowercase())
                .map(|m| m.as_str().to_string())
                .filter(|s| !stop.contains(&s.as_str()))
                .collect()
        })
        .unwrap_or_default()
}
pub fn rerank(candidates: &[Value], query: &str, limit: usize) -> Vec<Value> {
    let wanted = tokens(query);
    let mut scored = candidates
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let path = tokens(item["path"].as_str().unwrap_or(""))
                .into_iter()
                .filter(|t| {
                    ![
                        "php",
                        "json",
                        "yaml",
                        "yml",
                        "html",
                        "htm",
                        "xml",
                        "sql",
                        "java",
                        "python",
                        "javascript",
                        "typescript",
                        "markdown",
                        "toml",
                        "ini",
                    ]
                    .contains(&t.as_str())
                })
                .collect::<BTreeSet<_>>();
            let title = tokens(item["title"].as_str().unwrap_or(""));
            let factor = 1.0
                + 0.15 * wanted.intersection(&path).count().min(3) as f64
                + 0.10 * wanted.intersection(&title).count().min(2) as f64;
            (
                item["adjusted_score"].as_f64().unwrap_or(0.0) * factor,
                index,
                item.clone(),
            )
        })
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    scored.into_iter().take(limit).map(|r| r.2).collect()
}
pub fn query_set(document: &Value) -> Result<Vec<Value>> {
    let rows = if let Some(m) = document.as_object() {
        m.iter().map(|(id,q)|json!({"id":id,"query":q.as_str().map(String::from).unwrap_or_else(||q.to_string())})).collect::<Vec<_>>()
    } else if let Some(a) = document.as_array() {
        a.iter()
            .enumerate()
            .map(|(index, q)| {
                if !q.is_object() {
                    return Err(artifact::invalid("query must be object"));
                }
                let query = q
                    .get("query")
                    .or_else(|| q.get("prompt"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| artifact::invalid("query requires string query/prompt"))?;
                let id = q
                    .get("id")
                    .map(|v| {
                        v.as_str()
                            .map(String::from)
                            .unwrap_or_else(|| v.to_string())
                    })
                    .unwrap_or_else(|| index.to_string());
                Ok(json!({"id":id,"query":query}))
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        return Err(artifact::invalid("queries must be object or array"));
    };
    if rows.is_empty() {
        return Err(artifact::invalid("queries are empty"));
    }
    Ok(rows)
}
pub fn rerank_evaluate(
    database: &Path,
    queries: &Value,
    judgments: &Value,
    limit: usize,
) -> Result<Value> {
    rerank_with(database, queries, judgments, limit, None)
}
pub fn rerank_with(
    database: &Path,
    queries: &Value,
    judgments: &Value,
    limit: usize,
    runtime: Option<&Path>,
) -> Result<Value> {
    rerank_with_at(database, queries, judgments, limit, runtime, None)
}
pub fn rerank_with_at(
    database: &Path,
    queries: &Value,
    judgments: &Value,
    limit: usize,
    runtime: Option<&Path>,
    anchor: Option<&str>,
) -> Result<Value> {
    let _clock = crate::clock::enter(anchor)?;
    let store = infra_core::Store::open_read_only(database, Duration::from_secs(1))?;
    let queries = query_set(queries)?;
    if !judgments.is_object() {
        return Err(artifact::invalid("judgments must be object"));
    }
    for grades in judgments.as_object().unwrap().values() {
        if !grades.is_object() {
            return Err(artifact::invalid("query judgments must be object"));
        }
    }
    let mut rows = vec![];
    let (mut baseline_useful, mut reranked_useful) = (0u64, 0u64);
    for q in queries {
        let id = q["id"].as_str().unwrap();
        let query = q["query"].as_str().unwrap();
        let candidates = if let Some(runtime) = runtime {
            let scratch = artifact::Scratch::new(&std::env::temp_dir(), "rerank")?;
            let input = scratch.0.join("request.json");
            artifact::write_json(
                &input,
                &json!({"database":database,"query":query,"eval_clock":anchor}),
            )?;
            let (code, bytes) = process::capture(
                &process::CommandSpec {
                    argv: vec![
                        runtime.to_string_lossy().into(),
                        "__rank-candidates".into(),
                        input.to_string_lossy().into(),
                    ],
                    cwd: std::env::current_dir()?,
                    env: std::env::vars().collect(),
                    input: vec![],
                },
                Duration::from_secs(30),
                8 * 1024 * 1024,
            )?;
            if code != 0 {
                return Err(artifact::invalid("native candidate runtime failed"));
            }
            artifact::strict(&bytes)?
                .as_array()
                .cloned()
                .ok_or_else(|| artifact::invalid("native candidate list required"))?
        } else {
            candidate_values(&store, query)?
        };
        let baseline = candidates.iter().take(limit).cloned().collect::<Vec<_>>();
        let ranked = rerank(&candidates, query, limit);
        let metrics = |items: &[Value]| {
            let chosen = items
                .iter()
                .filter_map(|i| i["path"].as_str())
                .collect::<BTreeSet<_>>();
            let useful = judgments[id]
                .as_object()
                .into_iter()
                .flat_map(|m| m.iter())
                .filter(|(_, v)| v.as_u64().is_some_and(|g| g > 0))
                .map(|(p, _)| p.as_str())
                .collect::<BTreeSet<_>>();
            let n = chosen.intersection(&useful).count();
            json!({"useful":n,"noise":chosen.difference(&useful).count(),"recall":if useful.is_empty(){None}else{Some((n as f64/useful.len()as f64*10000.0).round()/10000.0)},"paths":chosen})
        };
        let b = metrics(&baseline);
        let r = metrics(&ranked);
        baseline_useful += b["useful"].as_u64().unwrap_or(0);
        reranked_useful += r["useful"].as_u64().unwrap_or(0);
        rows.push(json!({"id":id,"query":query,"changed":b["paths"]!=r["paths"],"baseline":b,"reranked":r}));
    }
    Ok(
        json!({"queries":rows.len(),"limit":limit,"weights":{"path":0.15,"title":0.10},"baseline_useful":baseline_useful,"reranked_useful":reranked_useful,"rows":rows}),
    )
}
fn candidate_values(store: &infra_core::Store, query: &str) -> Result<Vec<Value>> {
    let (candidates, _) = infra_core::query::rank_candidates(&store.conn, query, 100, None)?;
    Ok(candidates
        .into_iter()
        .map(|c| {
            let mut v = c.public();
            v["adjusted_score"] = json!(c.adjusted_score);
            v
        })
        .collect())
}
pub fn worker(args: &[String]) -> Result<Option<i32>> {
    if args.first().map(String::as_str) != Some("__rank-candidates") {
        return Ok(None);
    }
    let input = args
        .get(1)
        .ok_or_else(|| artifact::invalid("candidate request required"))?;
    let v = artifact::read_json(Path::new(input), artifact::MAX_TREE)?;
    let _clock = crate::clock::request(&v)?;
    let store = infra_core::Store::open_read_only(
        Path::new(
            v["database"]
                .as_str()
                .ok_or_else(|| artifact::invalid("database required"))?,
        ),
        Duration::from_secs(1),
    )?;
    println!(
        "{}",
        serde_json::to_string(&candidate_values(
            &store,
            v["query"]
                .as_str()
                .ok_or_else(|| artifact::invalid("query required"))?
        )?)?
    );
    Ok(Some(0))
}

pub fn selective_stream(
    command: &str,
    enabled: bool,
    rules: Option<&Path>,
    audit: Option<&Path>,
) -> Result<i32> {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    let mut first = vec![];
    if enabled {
        std::io::stdin().take(1048577).read_to_end(&mut first)?;
    }
    let started = Instant::now();
    let mut record = json!({"schema":"memory-selective-route-v2","profile":"recall","reason":"routing-unavailable","rules_digest":null,"router_digest":artifact::sha(include_bytes!("treatments.rs")),"sanitizer_digest":executable::digest(&std::env::current_exe()?)?});
    let mut env = std::env::vars().collect::<BTreeMap<_, _>>();
    if enabled {
        if first.len() <= 1048576
            && let (Ok(payload), Some(rules)) = (artifact::strict(&first), rules)
            && let (Ok(rules), Some(prompt)) = (
                artifact::read_json(rules, 16384),
                payload["prompt"].as_str(),
            )
        {
            let chars = prompt.chars().collect::<Vec<_>>();
            let prompt = if chars.len() > 24000 {
                format!(
                    "{}\n{}",
                    chars[..15999].iter().collect::<String>(),
                    chars[chars.len() - 8000..].iter().collect::<String>()
                )
            } else {
                prompt.into()
            };
            if let Ok(query) = infra_core::domain::sanitize_automatic_query(&prompt)
                && let Ok((profile, reason)) = route(&query, &rules)
            {
                record["profile"] = profile.into();
                record["reason"] = reason.into();
                record["rules_digest"] = artifact::digest(&rules)?.into();
            }
        }
        env.insert(
            "CONTEXT_CAPSULE_PROFILE".into(),
            record["profile"].as_str().unwrap().into(),
        );
    }
    record["routing_ns"] = json!(started.elapsed().as_nanos() as u64);
    let shell = crate::isolation::which("bash").unwrap_or_else(|| "/bin/sh".into());
    let mut child = Command::new(shell)
        .arg("-c")
        .arg(command)
        .envs(env)
        .stdin(if enabled {
            Stdio::piped()
        } else {
            Stdio::inherit()
        })
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;
    if enabled && let Some(mut input) = child.stdin.take() {
        let _ = input.write_all(&first);
        let _ = std::io::copy(&mut std::io::stdin(), &mut input);
    }
    let code = child.wait()?.code().unwrap_or(-1);
    if enabled {
        record["hook_exit"] = code.into();
        if let Some(audit) = audit
            && artifact::ordinary(audit).is_ok()
            && std::fs::metadata(audit)
                .map(|m| m.len() <= 1048576)
                .unwrap_or(true)
        {
            let raw = format!("{}\n", infra_core::domain::json::canonical(&record)?);
            let _ = infra_core::fs::append_whole_line(audit, raw.as_bytes());
            let _ = artifact::chmod(audit, 0o600);
        }
    }
    Ok(code)
}
