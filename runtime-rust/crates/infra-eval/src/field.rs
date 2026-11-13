use crate::scoring;
use crate::{Result, artifact, cost, reconstruction, transcript};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::{collections::BTreeMap, path::Path};
pub const NATIVE: &str = "memory-bank/local/native-results.ndjson";
pub const HEALTH: &str = "memory-bank/local/refresh-health.ndjson";
fn rows(path: &Path) -> Vec<Value> {
    transcript::records(path).unwrap_or_default()
}
fn within(row: &Value, since: Option<DateTime<Utc>>) -> bool {
    since.is_none_or(|since| {
        row["at"]
            .as_str()
            .and_then(reconstruction::instant)
            .is_some_and(|at| at >= since)
    })
}
const NATIVE_KEYS: [&str; 26] = [
    "prompt_hook_runs",
    "armed",
    "armed_after_incomplete",
    "continuations",
    "stop_hook_runs",
    "stop_hook_timeouts",
    "stop_hook_failures",
    "requested",
    "requested_brief",
    "saved",
    "held",
    "omitted",
    "failed",
    "refused",
    "errors",
    "learnings_written",
    "chunks_promoted",
    "learnings_submitted",
    "learnings_existing",
    "learnings_held",
    "selection_observed",
    "selection_unknown",
    "capture_selected",
    "capture_no_reusable_knowledge",
    "capture_insufficiently_checked",
    "capture_unknown",
];
pub fn native_step(
    counts: &mut BTreeMap<String, u64>,
    reasons: &mut BTreeMap<String, BTreeMap<String, u64>>,
    r: &Value,
) {
    let (Some(stage), Some(event)) = (r["stage"].as_str(), r["event"].as_str()) else {
        return;
    };
    let code = if stage == "stop-hook" && event == "exited" {
        r["status"].as_i64().map(|s| s.to_string())
    } else {
        r.get("code").filter(|v| !v.is_null()).map(|v| {
            v.as_str()
                .filter(|s| {
                    s.len() <= 80
                        && s.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
                })
                .unwrap_or("other")
                .to_string()
        })
    };
    let mut add = |key: &str, n: u64| {
        *counts.entry(key.into()).or_default() += n;
    };
    match stage {
        "prompt" => {
            add("prompt_hook_runs", 1);
            add("armed", u64::from(event == "armed"));
            add(
                "armed_after_incomplete",
                u64::from(
                    event == "armed"
                        && code.as_deref().is_some_and(|s| {
                            !["previous-in-flight", "previous-unreadable"].contains(&s)
                        }),
                ),
            );
            add("continuations", u64::from(event == "continuation"));
        }
        "stop-hook" => {
            add("stop_hook_runs", 1);
            add(
                "stop_hook_timeouts",
                u64::from(code.as_deref() == Some("124")),
            );
            add(
                "stop_hook_failures",
                u64::from(
                    event == "exited"
                        && code.as_deref().is_some_and(|s| !["0", "124"].contains(&s)),
                ),
            );
        }
        "stop" => {
            add("requested", u64::from(event == "requested"));
            add(
                "requested_brief",
                u64::from(event == "requested" && r["answer"] == "brief"),
            );
            add("failed", u64::from(event == "failed"));
            add("held", u64::from(event == "held"));
        }
        "save" => {
            if ["saved", "held", "omitted"].contains(&event) && code.as_deref() != Some("repeat") {
                add(event, 1);
            }
            if ["saved", "held"].contains(&event) {
                add("learnings_written", r["learnings"].as_u64().unwrap_or(0));
                add("chunks_promoted", r["promoted"].as_u64().unwrap_or(0));
                if code.as_deref() != Some("repeat") {
                    let observed = ["submitted", "existing", "held", "learnings"]
                        .iter()
                        .all(|k| r[*k].as_u64().is_some_and(|n| n <= 3))
                        && r["learnings"].as_u64().unwrap_or(4)
                            + r["existing"].as_u64().unwrap_or(4)
                            + r["held"].as_u64().unwrap_or(4)
                            == r["submitted"].as_u64().unwrap_or(99);
                    add(
                        if observed {
                            "selection_observed"
                        } else {
                            "selection_unknown"
                        },
                        1,
                    );
                    if observed {
                        for (field, key) in [
                            ("submitted", "learnings_submitted"),
                            ("existing", "learnings_existing"),
                            ("held", "learnings_held"),
                        ] {
                            add(key, r[field].as_u64().unwrap());
                        }
                    }
                    let decision = r["capture_decision"].as_str().unwrap_or("");
                    let consistent = observed
                        && [
                            "selected",
                            "no_reusable_knowledge",
                            "insufficiently_checked",
                        ]
                        .contains(&decision)
                        && ((decision == "selected") == (r["submitted"].as_u64().unwrap_or(0) > 0));
                    add(
                        &if consistent {
                            format!("capture_{decision}")
                        } else {
                            "capture_unknown".into()
                        },
                        1,
                    );
                }
            }
            add("refused", u64::from(event == "refused"));
        }
        _ => {}
    }
    add("errors", u64::from(event == "error"));
    if let Some(code) = code {
        *reasons
            .entry(format!("{stage} {event}"))
            .or_default()
            .entry(code)
            .or_default() += 1;
    }
}
pub fn native_funnel(state: &Path, since: Option<DateTime<Utc>>) -> Value {
    let mut hosts = BTreeMap::<
        String,
        (
            BTreeMap<String, u64>,
            BTreeMap<String, BTreeMap<String, u64>>,
        ),
    >::new();
    for h in ["claude", "codex", "cursor", "unknown"] {
        hosts.insert(h.into(), (BTreeMap::new(), BTreeMap::new()));
    }
    let mut lines = 0;
    for r in rows(&state.join(NATIVE)) {
        if !within(&r, since) {
            continue;
        }
        lines += 1;
        let host = r["host"]
            .as_str()
            .filter(|h| ["claude", "codex", "cursor"].contains(h))
            .unwrap_or("unknown");
        let (codes, reasons) = hosts.get_mut(host).unwrap();
        native_step(codes, reasons, &r);
    }
    let mut total = BTreeMap::<String, u64>::new();
    let mut reasons = BTreeMap::<String, BTreeMap<String, u64>>::new();
    let mut public = BTreeMap::new();
    for (host, (mut c, r)) in hosts {
        for k in &NATIVE_KEYS {
            c.entry((*k).into()).or_default();
        }
        for (k, n) in &c {
            *total.entry(k.clone()).or_default() += n;
        }
        for (step, codes) in &r {
            for (code, n) in codes {
                *reasons
                    .entry(step.clone())
                    .or_default()
                    .entry(code.clone())
                    .or_default() += n;
            }
        }
        let mut v = json!(c);
        v["reasons"] = json!(r);
        public.insert(host, v);
    }
    let mut total = json!(total);
    total["reasons"] = json!(reasons);
    json!({"journal":state.join(NATIVE).is_file(),"lines":lines,"prompt_hook_health_lines":rows(&state.join(HEALTH)).iter().filter(|r|r["source"]=="hook"&&within(r,since)).count(),"hosts":public,"total":total})
}
fn count_md(state: &Path, folders: &[&str]) -> usize {
    folders
        .iter()
        .map(|folder| {
            fn walk(p: &Path) -> usize {
                if !p.is_dir() {
                    return 0;
                }
                std::fs::read_dir(p)
                    .ok()
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| {
                        let p = e.path();
                        if std::fs::symlink_metadata(&p).is_ok_and(|m| infra_core::fs::linked(&m)) {
                            0
                        } else if p.is_dir() {
                            walk(&p)
                        } else {
                            usize::from(p.extension().is_some_and(|s| s == "md"))
                        }
                    })
                    .sum()
            }
            walk(&state.join(folder))
        })
        .sum()
}
pub fn funnel(state: &Path, since: Option<DateTime<Utc>>) -> Value {
    let mut c = BTreeMap::<String, u64>::new();
    for k in [
        "hook_runs",
        "hook_timeouts",
        "hook_failures",
        "capsules",
        "no_capsule",
        "shaped",
        "with_working",
        "with_recent_files",
        "with_ticket_files",
        "with_documents",
    ] {
        c.insert(k.into(), 0);
    }
    let mut profiles = BTreeMap::from([
        ("full".to_string(), 0u64),
        ("navigation".into(), 0),
        ("recall".into(), 0),
        ("unknown".into(), 0),
    ]);
    let mut hosts = BTreeMap::from([
        ("claude".to_string(), 0u64),
        ("codex".into(), 0),
        ("cursor".into(), 0),
        ("cli".into(), 0),
        ("unknown".into(), 0),
    ]);
    let (mut tokens, mut bytes) = (vec![], vec![]);
    for r in rows(&state.join(HEALTH)) {
        if !within(&r, since) {
            continue;
        }
        match r["source"].as_str() {
            Some("hook") => {
                *c.get_mut("hook_runs").unwrap() += 1;
                *c.get_mut("hook_timeouts").unwrap() += u64::from(r["hook_status"] == 124);
                *c.get_mut("hook_failures").unwrap() += u64::from(
                    r["hook_status"]
                        .as_i64()
                        .is_some_and(|s| s != 0 && s != 124),
                );
            }
            Some("refresh" | "hook-context") => {
                if r.get("capsule").is_some_and(Value::is_null) {
                    *c.get_mut("no_capsule").unwrap() += 1;
                    continue;
                }
                *c.get_mut("capsules").unwrap() += 1;
                let host = r["host"]
                    .as_str()
                    .filter(|s| hosts.contains_key(*s))
                    .unwrap_or("unknown");
                *hosts.get_mut(host).unwrap() += 1;
                let s = &r["capsule"];
                if !s.is_object() {
                    *profiles.get_mut("unknown").unwrap() += 1;
                    continue;
                }
                *c.get_mut("shaped").unwrap() += 1;
                let p = s["profile"]
                    .as_str()
                    .filter(|s| profiles.contains_key(*s))
                    .unwrap_or("unknown");
                *profiles.get_mut(p).unwrap() += 1;
                *c.get_mut("with_working").unwrap() += u64::from(s["working"] == true);
                for (field, key) in [
                    ("recent_files", "with_recent_files"),
                    ("ticket_files", "with_ticket_files"),
                    ("documents", "with_documents"),
                ] {
                    *c.get_mut(key).unwrap() += u64::from(s[field].as_u64().is_some_and(|n| n > 0));
                }
                if let Some(n) = s["tokens"].as_u64() {
                    tokens.push(n as f64);
                }
                if let Some(n) = s["bytes"].as_u64() {
                    bytes.push(n as f64);
                }
            }
            _ => {}
        }
    }
    let native = native_funnel(state, since);
    let save = &native["total"];
    for (key, n) in [
        ("armed", save["armed"].as_u64().unwrap_or(0)),
        ("requested", save["requested"].as_u64().unwrap_or(0)),
        (
            "made",
            save["saved"].as_u64().unwrap_or(0) + save["held"].as_u64().unwrap_or(0),
        ),
        ("refused", save["refused"].as_u64().unwrap_or(0)),
        (
            "failed_or_omitted",
            save["failed"].as_u64().unwrap_or(0) + save["omitted"].as_u64().unwrap_or(0),
        ),
        (
            "learnings_written",
            save["learnings_written"].as_u64().unwrap_or(0),
        ),
        (
            "chunks_promoted",
            save["chunks_promoted"].as_u64().unwrap_or(0),
        ),
    ] {
        c.insert(key.into(), n);
    }
    c.insert(
        "task_records".into(),
        count_md(
            state,
            &["project-brain/dynamic/tasks", "project-brain/archive/task"],
        ) as u64,
    );
    c.insert(
        "findings_decisions".into(),
        count_md(
            state,
            &[
                "project-brain/dynamic/findings",
                "project-brain/dynamic/decisions",
                "project-brain/archive/finding",
                "project-brain/archive/decision",
            ],
        ) as u64,
    );
    c.insert(
        "bank_chunks".into(),
        count_md(state, &["memory-bank/chunks"]) as u64,
    );
    let mut out = json!(c);
    out["median_tokens"] = json!(cost::median(&tokens).map(f64::round));
    out["median_bytes"] = json!(cost::median(&bytes).map(f64::round));
    out["tokens"] = json!(tokens);
    out["bytes"] = json!(bytes);
    out["profiles"] = json!(profiles);
    out["hosts"] = json!(hosts);
    out["save_journal"] = native["journal"].clone();
    out["save_reasons"] = save["reasons"].clone();
    out
}
pub fn field_counts(
    turns: &[transcript::Turn],
    retrievals: &[transcript::Retrievals],
    costs: &[Value],
) -> Value {
    let (mut knowledge, mut records) = (0, 0);
    for t in turns {
        for path in t.delivered.intersection(&t.touched) {
            match transcript::kind(path) {
                "brain" | "chunk" => knowledge += 1,
                "task-record" => records += 1,
                _ => {}
            }
        }
        records += t.records.intersection(&t.touched).count();
    }
    for s in retrievals {
        for (path, how) in &s.returned {
            if !s.opened.contains(path) {
                continue;
            }
            if how == "record" || transcript::kind(path) == "task-record" {
                records += 1;
            } else if ["brain", "chunk"].contains(&transcript::kind(path)) {
                knowledge += 1;
            }
        }
    }
    let answer = cost::median(
        &costs
            .iter()
            .filter_map(|c| c["answer_seconds"].as_f64())
            .collect::<Vec<_>>(),
    );
    let save = cost::median(
        &costs
            .iter()
            .filter_map(|c| c["save_seconds"].as_f64())
            .collect::<Vec<_>>(),
    );
    let share = answer
        .zip(save)
        .filter(|(a, _)| *a > 0.0)
        .map(|(a, s)| (s / a * 100.0).round() / 100.0);
    json!({"capsule_turns":turns.iter().filter(|t|t.output).count(),"knowledge_opened":knowledge,"task_records_opened":records,"save_requests":costs.len(),"save_share":share})
}
pub fn field_rule(hosts: &Value, minimum: usize) -> Value {
    let Some(hosts) = hosts.as_object() else {
        return json!({"ready":false,"counted":[],"outcome":{}});
    };
    let counted = hosts
        .iter()
        .filter(|(_, v)| v["capsule_turns"].as_u64().unwrap_or(0) >= minimum as u64)
        .map(|(k, _)| k.clone())
        .collect::<Vec<_>>();
    let no_knowledge = counted.iter().all(|h| hosts[h]["knowledge_opened"] == 0);
    let mut outcome = BTreeMap::new();
    for h in &counted {
        let c = &hosts[h];
        outcome.insert(
            h,
            if no_knowledge && c["save_share"].as_f64().is_some_and(|s| s >= 0.20) {
                "outcome A/B of the every-turn save on continuation tasks"
            } else if c["task_records_opened"].as_u64().unwrap_or(0) > 0
                && c["knowledge_opened"] == 0
            {
                "working-state path first"
            } else {
                "report as is"
            },
        );
    }
    json!({"ready":counted.len()==hosts.len()&&!counted.is_empty(),"counted":counted,"outcome":outcome})
}
pub struct ProjectSessions {
    pub claude: Vec<transcript::Turn>,
    pub codex: Vec<transcript::Turn>,
    pub claude_retrievals: Vec<transcript::Retrievals>,
    pub codex_retrievals: Vec<transcript::Retrievals>,
    pub claude_costs: Vec<Value>,
    pub codex_costs: Vec<Value>,
    pub hooks: BTreeMap<(String, String), u64>,
    pub files: [usize; 2],
    pub compressed_unread: usize,
}
pub fn sessions(
    project: &Path,
    claude_root: &Path,
    codex_root: &Path,
    since: Option<DateTime<Utc>>,
) -> Result<ProjectSessions> {
    let (cc, cx, unread) = transcript::project_files(project, claude_root, codex_root)?;
    let mut out = ProjectSessions {
        claude: vec![],
        codex: vec![],
        claude_retrievals: vec![],
        codex_retrievals: vec![],
        claude_costs: vec![],
        codex_costs: vec![],
        hooks: BTreeMap::new(),
        files: [cc.len(), cx.len()],
        compressed_unread: unread,
    };
    for (host, files) in [("claude", cc), ("codex", cx)] {
        for file in files {
            let rows = transcript::records(&file)?;
            let parsed = transcript::parse(&rows, host, since);
            let costs = cost::continuation_costs(&rows, host);
            if host == "claude" {
                out.claude.extend(parsed.turns);
                out.claude_retrievals.push(parsed.retrievals);
                out.claude_costs.extend(costs);
                for (k, n) in parsed.hooks {
                    *out.hooks.entry(k).or_default() += n;
                }
            } else {
                out.codex.extend(parsed.turns);
                out.codex_retrievals.push(parsed.retrievals);
                out.codex_costs.extend(costs);
            }
        }
    }
    Ok(out)
}
pub fn realized(
    project: &Path,
    state: &Path,
    claude_root: &Path,
    codex_root: &Path,
    since: Option<DateTime<Utc>>,
) -> Result<Value> {
    realized_options(project, state, claude_root, codex_root, since, false, false)
}
pub fn realized_options(
    project: &Path,
    state: &Path,
    claude_root: &Path,
    codex_root: &Path,
    since: Option<DateTime<Utc>>,
    paths: bool,
    traces: bool,
) -> Result<Value> {
    if !state.is_dir() {
        return Err(artifact::invalid("--state must name an existing directory"));
    }
    let s = sessions(project, claude_root, codex_root, since)?;
    let mut all = s.claude.clone();
    all.extend(s.codex.clone());
    let mut retrieves = s.claude_retrievals.clone();
    retrieves.extend(s.codex_retrievals.clone());
    let mut cc = transcript::tally(&s.claude);
    cc["transcripts"] = s.files[0].into();
    let mut cx = transcript::tally(&s.codex);
    cx["transcripts"] = s.files[1].into();
    cx["compressed_unread"] = s.compressed_unread.into();
    let mut total = transcript::tally(&all);
    total["transcripts"] = (s.files[0] + s.files[1]).into();
    let mut hooks = json!({});
    for ((event, outcome), n) in s.hooks {
        if hooks.get(&event).is_none() {
            hooks[&event] = json!({"ok":0,"error":0,"not_found":0,"blocked":0});
        }
        hooks[&event][&outcome] = n.into();
    }
    cc["hooks"] = hooks;
    let mut report = json!({"project":project,"since":since.map(|s|s.format("%Y-%m-%d").to_string()),"claude":cc,"codex":cx,"total":total,"retrieve":{"claude":transcript::retrieval_summary(&s.claude_retrievals),"codex":transcript::retrieval_summary(&s.codex_retrievals),"total":transcript::retrieval_summary(&retrieves)},"native":native_funnel(state,since),"continuations":{"claude":cost::continuation_summary(&s.claude_costs),"codex":cost::continuation_summary(&s.codex_costs)}});
    if paths {
        let mut counts = BTreeMap::<String, Value>::new();
        for turn in &all {
            let text = turn.text.join("\n");
            for path in &turn.delivered {
                let v = counts
                    .entry(path.clone())
                    .or_insert_with(|| json!({"delivered":0,"touched":0,"mentioned":0}));
                v["delivered"] = json!(v["delivered"].as_u64().unwrap_or(0) + 1);
                v["touched"] = json!(
                    v["touched"].as_u64().unwrap_or(0) + u64::from(turn.touched.contains(path))
                );
                v["mentioned"] = json!(
                    v["mentioned"].as_u64().unwrap_or(0)
                        + u64::from(transcript::mentioned(path, &text))
                );
            }
        }
        report["paths"] = json!(counts);
    }
    if traces {
        let answers = retrieves
            .iter()
            .flat_map(|s| s.answers.iter().cloned())
            .collect::<Vec<_>>();
        report["traces"] = trace_report(
            state,
            since,
            &[("claude", &s.claude), ("codex", &s.codex)],
            &answers,
        )?;
    }
    Ok(report)
}

pub fn install_status(project: &Path, source: &Path) -> Result<Value> {
    let load = |name: &str| {
        artifact::read_json(&project.join(name), 1048576).unwrap_or_else(|_| json!({}))
    };
    let text = |name: &str| std::fs::read_to_string(project.join(name)).unwrap_or_default();
    let record = load("memory-bank/local/accelerator-install.json");
    let commit = record["source_commit"].as_str();
    let head = reconstruction::git(source, &["rev-parse", "HEAD"]).ok();
    let clone = match commit {
        None => "no record of the commit it was synced from",
        Some(commit) if Some(commit) == head.as_deref() => "this clone's commit",
        Some(commit) => {
            let spec = crate::process::CommandSpec {
                argv: vec![
                    "git".into(),
                    "merge-base".into(),
                    "--is-ancestor".into(),
                    commit.into(),
                    "HEAD".into(),
                ],
                cwd: source.into(),
                env: reconstruction::clean_env(),
                input: vec![],
            };
            if crate::process::capture(&spec, std::time::Duration::from_secs(10), 16384)
                .is_ok_and(|r| r.0 == 0)
            {
                "behind this clone"
            } else {
                "not in this clone (newer, or another line of work)"
            }
        }
    };
    let legacy = text("memory-bank/scripts/brain_runtime.py");
    let capabilities = load("memory-bank/local/runtime-capabilities.json");
    let mut known = capabilities["capsule_profiles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect::<Vec<_>>();
    if known.is_empty()
        && let Ok(r) = regex::Regex::new(r"(?m)^CAPSULE_PROFILES = \(([^)]*)\)")
        && let Some(m) = r.captures(&legacy)
        && let Ok(quoted) = regex::Regex::new(r#"["']([a-z-]+)["']"#)
    {
        known = quoted
            .captures_iter(&m[1])
            .map(|m| m[1].to_string())
            .collect();
    }
    let configured = load("project-brain/config/runtime.json")["capsule_profile"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("full")
        .to_string();
    let mut problems = vec![];
    if !known.is_empty() && !known.contains(&configured) {
        problems.push(format!(
            "capsule_profile {:?} is unknown to this project's runtime ({}): {}",
            configured.chars().take(40).collect::<String>(),
            known.join(", "),
            if legacy.contains("def automatic_capsule_profile")
                || capabilities["unknown_profile_fallback"] == "full"
            {
                "it delivers the full capsule instead"
            } else {
                "every prompt goes without a capsule"
            }
        ));
    }
    let health = rows(&project.join(HEALTH));
    let hook = health.iter().rev().find(|r| r["source"] == "hook");
    let refresh = health
        .iter()
        .rev()
        .find(|r| r["source"] == "refresh" || r["source"] == "hook-context");
    let capsule = refresh.map(|r| {
        if r.get("capsule").is_none() {
            "not recorded (an older runtime)"
        } else if r["capsule"].is_null() {
            "none built"
        } else {
            "built"
        }
    });
    if capsule == Some("none built") {
        problems.push("the last prompt that asked for a capsule got none".into());
    }
    let journal = rows(&project.join(NATIVE));
    let save = journal
        .iter()
        .rev()
        .find(|r| r["stage"].as_str().is_some_and(|s| !s.is_empty()));
    let own = project.join("CLAUDE.md").is_file();
    let imported = text(".claude/CLAUDE.md").contains("@../AGENTS.md");
    if own && !imported && project.join(".claude").is_dir() {
        problems.push("Claude Code loads the project's own CLAUDE.md and no AGENTS.md: .claude/CLAUDE.md does not import it (@../AGENTS.md)".into());
    }
    let hooks = load(".claude/settings.json");
    let mut relative = std::collections::BTreeSet::new();
    for groups in hooks["hooks"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
    {
        for group in groups.as_array().into_iter().flatten() {
            for h in group["hooks"].as_array().into_iter().flatten() {
                if let Some(command) = h["command"].as_str() {
                    let words = shell_words::split(command).unwrap_or_default();
                    let first = if words.first().is_some_and(|s| s == "bash" || s == "sh") {
                        words.get(1)
                    } else {
                        words.first()
                    };
                    if first.is_some_and(|s| {
                        s.starts_with(".claude/")
                            || s.starts_with("./.claude/")
                            || s.starts_with("../.claude/")
                    }) {
                        relative.insert(command.to_string());
                    }
                }
            }
        }
    }
    if !relative.is_empty() {
        problems.push(format!("{} Claude Code hook command(s) name .claude/ relative to the folder a session starts in: below the project root they are not found",relative.len()));
    }
    let config = text(".codex/config.toml");
    let cap = regex::Regex::new(r"(?m)^project_doc_max_bytes\s*=\s*(\d+)")
        .ok()
        .and_then(|r| r.captures(&config))
        .and_then(|m| m[1].parse::<usize>().ok())
        .unwrap_or(32768);
    let agents = text("AGENTS.md");
    if project.join(".codex").is_dir() && agents.len() > cap {
        let cut = agents
            .char_indices()
            .find(|(i, _)| *i >= cap)
            .map(|(i, _)| i)
            .unwrap_or(agents.len());
        if agents[cut..].contains("##") && agents[cut..].to_lowercase().contains("memory") {
            problems.push(format!(
                "Codex project_doc_max_bytes {cap} cuts AGENTS.md before the memory policy"
            ));
        }
    }
    Ok(
        json!({"edition":record["edition"],"release":record["release"],"source":commit.map(|s|s.chars().take(12).collect::<String>()),"synced_at":record["synced_at"],"clone":clone,"capsule_profile":configured,"profiles_known":known,"last_prompt_hook":hook.map(|r|r["at"].clone()),"last_capsule":capsule,"last_save_step":save.map(|r|format!("{}:{}",r["stage"].as_str().unwrap_or(""),r["event"].as_str().unwrap_or(""))),"last_save_at":save.map(|r|r["at"].clone()),"claude_policy":if own&&imported{"own CLAUDE.md, AGENTS.md imported"}else if own{"own CLAUDE.md, no import"}else{"AGENTS.md read by Claude Code itself"},"problems":problems}),
    )
}

pub fn trace_report(
    state: &Path,
    since: Option<DateTime<Utc>>,
    turns: &[(&str, &Vec<transcript::Turn>)],
    answers: &[String],
) -> Result<Value> {
    struct Trace {
        sha: String,
        entry: String,
        created: Option<DateTime<Utc>>,
        items: BTreeSet<String>,
    }
    let mut traces = vec![];
    let folder = state.join("memory-bank/local/retrieval-traces");
    if folder.is_dir() {
        for e in std::fs::read_dir(folder)? {
            let p = e?.path();
            if p.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Ok(data) = artifact::read_json(&p, 512 * 1024) else {
                continue;
            };
            let Some(sha) = data["render"]["sha256"].as_str() else {
                continue;
            };
            let created = data["created_at"]
                .as_str()
                .and_then(reconstruction::instant);
            if since.zip(created).is_some_and(|(s, c)| c < s) {
                continue;
            }
            let mut entry = "unknown".to_string();
            for dir in [
                "project-brain/control/retrieval-manifests",
                "memory-bank/local/retrieval-manifests",
            ] {
                if let Ok(m) = artifact::read_json(
                    &state.join(dir).join(p.file_name().unwrap_or_default()),
                    artifact::MAX_FILE,
                ) && let Some(e) = m["entry_point"].as_str()
                {
                    entry = e.into();
                    break;
                }
            }
            let items = data["render"]["records"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| r["path"].as_str().map(scoring::normalize))
                .collect();
            traces.push(Trace {
                sha: sha.into(),
                entry,
                created,
                items,
            });
        }
    }
    traces.sort_by_key(|t| t.created);
    let mut matched = BTreeSet::<usize>::new();
    let take = |digests: &BTreeSet<String>, matched: &mut BTreeSet<usize>| -> Option<usize> {
        for digest in digests {
            if let Some((i, _)) = traces
                .iter()
                .enumerate()
                .find(|(i, t)| t.sha == *digest && !matched.contains(i))
            {
                matched.insert(i);
                return Some(i);
            }
        }
        None
    };
    let mut hosts = json!({});
    for (host, turns) in turns {
        let (mut capsules, mut with_trace, mut items, mut touched) = (0, 0, 0, 0);
        for turn in turns.iter() {
            if !turn.output {
                continue;
            }
            capsules += 1;
            if let Some(index) = take(&turn.digests, &mut matched) {
                with_trace += 1;
                items += traces[index].items.len();
                touched += traces[index].items.intersection(&turn.touched).count();
            }
        }
        hosts[*host] = json!({"capsules":capsules,"with_trace":with_trace,"trace_items":items,"trace_items_touched":touched});
    }
    let answered = answers
        .iter()
        .filter(|s| {
            let body = artifact::strip_tail(s, "\n");
            let digests = BTreeSet::from([
                artifact::sha(body.as_bytes()),
                artifact::sha(format!("{body}\n").as_bytes()),
            ]);
            take(&digests, &mut matched).is_some()
        })
        .count();
    let count = |entries: &[&str], seen: bool| {
        traces
            .iter()
            .enumerate()
            .filter(|(i, t)| entries.contains(&t.entry.as_str()) && matched.contains(i) == seen)
            .count()
    };
    Ok(
        json!({"traces":traces.len(),"hook":{"handed":count(&["refresh","hook-context"],true),"not_seen":count(&["refresh","hook-context"],false)},"retrieve":{"answered":answered,"not_seen":count(&["retrieve","context"],false)},"unknown_entry":traces.iter().filter(|t|!["refresh","hook-context","retrieve","context"].contains(&t.entry.as_str())).count(),"hosts":hosts}),
    )
}
