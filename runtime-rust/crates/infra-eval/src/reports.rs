use crate::{Result, artifact, control, reconstruction, scoring};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
const ROWS: [(&str, &str); 50] = [
    ("prompts", "prompts"),
    ("evaluated", "evaluated"),
    ("skipped", "skipped_total"),
    ("useful turns", "useful"),
    ("could help (a useful document existed)", "could_help"),
    ("useful among could-help", "useful_among_could_help"),
    ("answer in the capsule text", "answer_in_text"),
    ("answer passages existed", "answer_passages_existed"),
    (
        "answer passages heading-only",
        "answer_passages_heading_only",
    ),
    ("answer passages delivered", "answer_passages_delivered"),
    ("answer passage recall", "answer_passage_recall"),
    (
        "answer passages deliverable (in the index)",
        "answer_passages_deliverable",
    ),
    (
        "  of them in documents over 64 KB",
        "answer_passages_long_document",
    ),
    (
        "deliverable passages delivered",
        "answer_passages_delivered_deliverable",
    ),
    ("deliverable passage recall", "deliverable_passage_recall"),
    (
        "could answer (a labelled answer existed)",
        "answer_could_help",
    ),
    (
        "contaminated (today's body of a later edit)",
        "contaminated",
    ),
    (
        "answer in text among could-answer",
        "answer_in_text_among_answer_could_help",
    ),
    ("noise-only turns", "noise_only"),
    ("unjudged-only turns", "unjudged_only"),
    ("silent turns", "silent"),
    ("mean delivered per turn", "mean_delivered"),
    ("mean noise per turn", "mean_noise"),
    ("mean unjudged per turn", "mean_unjudged"),
    ("latency p50 (s)", "latency_p50"),
    ("latency p95 (s)", "latency_p95"),
    ("knowledge: useful turns", "knowledge_useful"),
    ("knowledge: could help", "knowledge_could_help"),
    ("knowledge: noise-only turns", "knowledge_noise_only"),
    ("knowledge: unjudged-only turns", "knowledge_unjudged_only"),
    ("knowledge: silent turns", "knowledge_silent"),
    (
        "knowledge: useful paths delivered",
        "knowledge_useful_paths_delivered",
    ),
    ("knowledge: noise paths", "knowledge_noise_paths"),
    ("skills: a useful one delivered", "skill_hit"),
    ("skills: another one delivered", "skill_wrong"),
    ("skills: none delivered", "skill_empty"),
    ("skills: noise where none was useful", "skill_noise"),
    ("skills: silent where none was useful", "skill_silent"),
    ("useful paths delivered", "useful_paths_delivered"),
    ("noise paths", "noise_paths"),
    ("unjudged paths", "unjudged_paths"),
    ("skill sub-files in procedural", "skill_subfiles_delivered"),
    ("Brain events in episodic", "episodic_brain_events"),
    (
        "changed-citation items delivered",
        "source_changed_delivered",
    ),
    ("snapshots with link rows", "index_snapshots_with_links"),
    ("indexed documents matching a secret", "secret_documents"),
    ("capsules matching a secret", "capsule_secret_matches"),
    ("max capsule chars", "capsule_chars_max"),
    ("index latency p50 (s)", "index_latency_p50"),
    ("index latency p95 (s)", "index_latency_p95"),
];
const SHARES: [&str; 13] = [
    "useful",
    "answer_in_text",
    "noise_only",
    "unjudged_only",
    "silent",
    "could_help",
    "answer_could_help",
    "contaminated",
    "knowledge_useful",
    "knowledge_could_help",
    "knowledge_noise_only",
    "knowledge_unjudged_only",
    "knowledge_silent",
];
fn evaluated(items: &Value) -> BTreeSet<String> {
    items
        .as_object()
        .into_iter()
        .flat_map(|m| m.iter())
        .filter(|(_, i)| i["status"] == "ok")
        .map(|(id, _)| id.clone())
        .collect()
}
fn numeric(v: &Value) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite())
}
fn n(v: &Value) -> u64 {
    v.as_u64().unwrap_or(0)
}
pub fn compatibility(a: &Value, b: &Value) -> Value {
    let mut mismatch = BTreeSet::<String>::new();
    let mut unknown = BTreeSet::<String>::new();
    fn compare(
        field: &str,
        a: Option<&Value>,
        b: Option<&Value>,
        valid: impl Fn(&Value) -> bool,
        m: &mut BTreeSet<String>,
        u: &mut BTreeSet<String>,
    ) {
        match (a.filter(|v| valid(v)), b.filter(|v| valid(v))) {
            (Some(a), Some(b)) => {
                if a != b {
                    m.insert(field.into());
                }
            }
            _ => {
                u.insert(field.into());
            }
        }
    }
    let digest = |v: &Value| v.as_str().is_some_and(|s| artifact::hex(s, 64));
    for key in ["set_sha256", "judgments_sha256"] {
        compare(
            &format!("meta.{key}"),
            a["meta"].get(key),
            b["meta"].get(key),
            digest,
            &mut mismatch,
            &mut unknown,
        );
    }
    compare(
        "meta.passages_sha256",
        a["meta"].get("passages_sha256"),
        b["meta"].get("passages_sha256"),
        |v| v.is_null() || digest(v),
        &mut mismatch,
        &mut unknown,
    );
    for (key, choices) in [
        ("host", &["claude", "codex", "cursor"][..]),
        ("clock", &["pinned", "real"][..]),
        ("as_of", &["prompt", "now"][..]),
        ("updated_after", &["drop", "keep"][..]),
    ] {
        compare(
            &format!("meta.{key}"),
            a["meta"].get(key),
            b["meta"].get(key),
            |v| v.as_str().is_some_and(|s| choices.contains(&s)),
            &mut mismatch,
            &mut unknown,
        );
    }
    let duration = |v: &Value| numeric(v).is_some_and(|n| n > 0.0);
    compare(
        "meta.timeout",
        a["meta"].get("timeout"),
        b["meta"].get("timeout"),
        duration,
        &mut mismatch,
        &mut unknown,
    );
    if a["meta"].get("onnx_excerpts").is_some() || b["meta"].get("onnx_excerpts").is_some() {
        compare(
            "meta.onnx_excerpts.worker_timeout",
            a["meta"]["onnx_excerpts"].get("worker_timeout"),
            b["meta"]["onnx_excerpts"].get("worker_timeout"),
            duration,
            &mut mismatch,
            &mut unknown,
        );
    }
    if a["meta"].get("runtime_under_test").is_some()
        || b["meta"].get("runtime_under_test").is_some()
    {
        compare(
            "meta.runtime_under_test.scorer_sha256",
            a["meta"]["runtime_under_test"].get("scorer_sha256"),
            b["meta"]["runtime_under_test"].get("scorer_sha256"),
            digest,
            &mut mismatch,
            &mut unknown,
        );
    }
    let both = evaluated(&a["items"])
        .intersection(&evaluated(&b["items"]))
        .cloned()
        .collect::<Vec<_>>();
    if both.is_empty() {
        unknown.insert("items.shared_sources".into());
    }
    for id in both {
        let x = &a["items"][&id];
        let y = &b["items"][&id];
        compare(
            "items.project",
            x.get("project"),
            y.get("project"),
            |v| v.as_str().is_some_and(|s| !s.trim().is_empty()),
            &mut mismatch,
            &mut unknown,
        );
        let xa = x["ts"]
            .as_str()
            .and_then(reconstruction::instant)
            .map(|at| json!(at.to_rfc3339()));
        let ya = y["ts"]
            .as_str()
            .and_then(reconstruction::instant)
            .map(|at| json!(at.to_rfc3339()));
        compare(
            "items.ts",
            xa.as_ref(),
            ya.as_ref(),
            Value::is_string,
            &mut mismatch,
            &mut unknown,
        );
        compare(
            "items.commit",
            x.get("commit"),
            y.get("commit"),
            |v| {
                v.as_str()
                    .is_some_and(|s| artifact::hex(s, 40) || artifact::hex(s, 64))
            },
            &mut mismatch,
            &mut unknown,
        );
        compare(
            "items.edition",
            x.get("edition"),
            y.get("edition"),
            |v| {
                v.as_str()
                    .is_some_and(|s| ["PHP Core", "Symfony", "Laravel", "WordPress"].contains(&s))
            },
            &mut mismatch,
            &mut unknown,
        );
        for k in ["history_linked", "contaminated"] {
            compare(
                &format!("items.{k}"),
                x.get(k),
                y.get(k),
                Value::is_boolean,
                &mut mismatch,
                &mut unknown,
            );
        }
        if x["contaminated"] == true || y["contaminated"] == true {
            unknown.insert("items.contaminated".into());
        }
        for (unit, meta) in [(x, &a["meta"]), (y, &b["meta"])] {
            let required = if meta["as_of"] == "prompt" {
                vec![
                    "from_git",
                    "from_worktree",
                    "dropped_future",
                    "undetermined",
                    "updated_after",
                    "unreconstructable_updated",
                    "archive_from_git",
                    "archive_from_worktree",
                    "archive_dropped_future",
                    "archive_undetermined",
                    "archive_unreconstructable_updated",
                    "archive_updated_after",
                    "archive_still_dynamic",
                ]
            } else {
                vec![
                    "from_git",
                    "from_worktree",
                    "dropped_future",
                    "future_present",
                    "undetermined",
                ]
            };
            let p = unit["provenance"].as_object();
            let valid = p.is_some_and(|p| {
                required.iter().all(|k| p.contains_key(*k))
                    && p.iter().all(|(k, v)| {
                        v.as_u64().is_some()
                            && (required.contains(&k.as_str()) || k == "config_from_worktree")
                    })
            });
            if !valid {
                unknown.insert("items.provenance".into());
            }
        }
        if let (Some(x), Some(y)) = (x["provenance"].as_object(), y["provenance"].as_object()) {
            if x.keys().collect::<BTreeSet<_>>() != y.keys().collect() {
                unknown.insert("items.provenance".into());
            }
            for (k, v) in x {
                if let Some(other) = y.get(k)
                    && v.as_u64().is_some()
                    && other.as_u64().is_some()
                    && v != other
                {
                    mismatch.insert("items.provenance".into());
                }
            }
        }
    }
    json!({"profile":"recorded-comparison-v1","status":if !mismatch.is_empty(){"incompatible"}else if !unknown.is_empty(){"unknown"}else{"compatible"},"mismatched_fields":mismatch,"unknown_fields":unknown})
}
fn value(s: &Value, key: &str) -> Option<f64> {
    if key == "skipped_total" {
        return Some(
            s["skipped"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.values())
                .filter_map(Value::as_f64)
                .sum(),
        );
    }
    let mut v = s;
    for part in key.split('/') {
        v = v.get(part)?;
    }
    numeric(v)
}
fn cell(s: &Value, key: &str) -> String {
    let Some(value) = value(s, key) else {
        return "-".into();
    };
    let denominator = if SHARES.contains(&key) {
        Some(n(&s["evaluated"]))
    } else {
        match key {
            "useful_among_could_help" => Some(n(&s["could_help"])),
            "answer_in_text_among_answer_could_help" => Some(n(&s["answer_could_help"])),
            "useful_paths_delivered" => Some(n(&s["useful_paths_existed"])),
            "knowledge_useful_paths_delivered" => Some(n(&s["knowledge_useful_paths_existed"])),
            "skill_hit" | "skill_wrong" | "skill_empty" => Some(n(&s["skill_with_useful"])),
            "skill_noise" | "skill_silent" => Some(n(&s["skill_without_useful"])),
            _ => None,
        }
    };
    if let Some(d) = denominator {
        if SHARES.contains(&key) {
            return if d > 0 {
                format!("{}/{} ({:.1}%)", value as u64, d, 100.0 * value / d as f64)
            } else {
                format!("{}/{}", value as u64, d)
            };
        }
        return format!("{}/{}", value as u64, d);
    }
    if key.ends_with("_recall")
        || key == "miss_audit/candidate_recall"
        || key == "miss_audit/selected_recall"
    {
        format!("{:.1}%", value * 100.0)
    } else if key.starts_with("latency") || key.starts_with("index_latency") {
        format!("{value:.3}")
    } else if key.starts_with("mean") {
        format!("{value:.2}")
    } else {
        format!("{}", value as i64)
    }
}
fn describe(document: &Value) -> String {
    let m = &document["meta"];
    let mut edition = m["edition"]
        .as_str()
        .filter(|s| ["PHP Core", "Symfony", "Laravel", "WordPress", "auto"].contains(s))
        .unwrap_or("unknown")
        .to_string();
    if edition == "auto" {
        let mut used = BTreeMap::<String, usize>::new();
        for item in document["items"]
            .as_object()
            .into_iter()
            .flat_map(|m| m.values())
        {
            if let Some(e) = item["edition"]
                .as_str()
                .filter(|e| ["PHP Core", "Symfony", "Laravel", "WordPress"].contains(e))
            {
                *used.entry(e.into()).or_default() += 1;
            }
        }
        edition.push_str(&format!(
            " ({})",
            used.iter()
                .map(|(e, n)| format!("{e} {n}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let commit = m["clone_commit"]
        .as_str()
        .filter(|s| artifact::hex(s, 40) || artifact::hex(s, 64))
        .map(|s| &s[..10])
        .unwrap_or("?");
    let dirty = m["editions"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
        .any(|v| v["dirty"] == true);
    let choice = |key: &str, allowed: &[&str]| {
        m[key]
            .as_str()
            .filter(|s| allowed.contains(s))
            .unwrap_or("unknown")
    };
    format!(
        "edition={edition} as_of={} clock={} host={} clone={commit}{}",
        choice("as_of", &["prompt", "now"]),
        choice("clock", &["pinned", "real"]),
        choice("host", &["claude", "codex", "cursor"]),
        if dirty { " (edition dirty)" } else { "" }
    )
}
pub fn format(results: &[(&str, &Path, &Value)], show_unjudged: bool) -> Result<String> {
    let summaries = results
        .iter()
        .map(|(_, _, d)| scoring::summarize(&d["items"]))
        .collect::<Vec<_>>();
    let mut lines = vec![];
    for (label, path, d) in results {
        lines.push(format!("{label}: {}  {}", path.display(), describe(d)));
        let w = control::planned_delivery(d);
        let mut line = format!(
            "whole-planned {label}: {}",
            w["status"].as_str().unwrap_or("unknown")
        );
        if w["status"] != "unknown" {
            let percent = |v: &Value| {
                v.as_f64()
                    .map(|n| format!("{:.1}%", n * 100.0))
                    .unwrap_or_else(|| "-".into())
            };
            line.push_str(&format!("; terminal={}/{}; source-known={}/{}; source-excluded={}; unattempted={}; without-terminal={}; unit delivery={}; passage recall={}",w["terminal"],w["planned"],w["source_known"],w["planned"],w["source_excluded"],w["unattempted"],w["without_terminal"],percent(&w["unit_delivery_rate"]),percent(&w["passage_recall"])));
        }
        lines.push(format!("{line}; supplementary per-run delivery only"));
    }
    let mut paired = vec![];
    let mut title = "delta";
    let mut comparison = None;
    let mut both = BTreeSet::new();
    if results.len() == 2 {
        let c = compatibility(results[0].2, results[1].2);
        let status = c["status"].as_str().unwrap_or("unknown");
        lines.push(format!(
            "comparison: {status} recorded metadata{}",
            if status == "incompatible" {
                "; quality/latency deltas withheld"
            } else if status == "unknown" {
                "; ID-overlap diagnostics only"
            } else {
                "; not a corpus-body identity proof"
            }
        ));
        for k in ["mismatched_fields", "unknown_fields"] {
            let fields = c[k]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            if !fields.is_empty() {
                lines.push(format!("{}: {}", k.replace('_', " "), fields.join(", ")));
            }
        }
        let a = evaluated(&results[0].2["items"]);
        let b = evaluated(&results[1].2["items"]);
        both = a.intersection(&b).cloned().collect();
        let only = [a.difference(&both).count(), b.difference(&both).count()];
        if !both.is_empty() && status != "incompatible" {
            for (_, _, d) in results {
                let items = both
                    .iter()
                    .map(|id| (id.clone(), d["items"][id].clone()))
                    .collect::<serde_json::Map<_, _>>();
                paired.push(scoring::summarize(&Value::Object(items)));
            }
        }
        if only.iter().any(|n| *n > 0) {
            title = "paired delta";
            lines.push(format!("note: {} evaluated in both, {} only in A, {} only in B: each measure's delta is over the prompts evaluated in both; prompts, evaluated and skipped compare the whole runs",both.len(),only[0],only[1]));
        }
        if status == "unknown" {
            title = if only.iter().any(|n| *n > 0) {
                "diagnostic paired"
            } else {
                "diagnostic delta"
            };
            if only.iter().any(|n| *n > 0) {
                lines.push("note: diagnostic paired delta does not establish compatible comparison conditions".into());
            }
        } else if status == "incompatible" {
            title = "no paired delta";
        }
        comparison = Some(c);
    }
    let mut rows = ROWS
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect::<Vec<_>>();
    if summaries.iter().any(|s| s.get("miss_audit").is_some()) {
        for (title, key) in [
            ("audit: prompts", "prompts"),
            ("audit: unknown judged paths", "unknown_paths"),
            ("audit: candidate recall", "candidate_recall"),
            ("audit: selected recall", "selected_recall"),
            ("audit: history paths", "history_paths"),
            ("audit: history delivered", "history_delivered"),
            ("audit: history recall", "history_recall"),
        ] {
            rows.push((title.into(), format!("miss_audit/{key}")));
        }
        for group in ["knowledge", "skills"] {
            let stages = summaries
                .iter()
                .flat_map(|s| {
                    s["miss_audit"][group]
                        .as_object()
                        .into_iter()
                        .flat_map(|m| m.keys().cloned())
                })
                .collect::<BTreeSet<_>>();
            for stage in stages {
                rows.push((
                    format!("audit {group}: {stage}"),
                    format!("miss_audit/{group}/{stage}"),
                ));
            }
        }
    }
    for ((label, _, _), s) in results.iter().zip(&summaries) {
        if let (Some(c), Some(selected)) = (
            s["miss_audit"]["candidate_recall"].as_f64(),
            s["miss_audit"]["selected_recall"].as_f64(),
        ) {
            let a = (1.0 - c).max(0.0);
            let b = (c - selected).max(0.0);
            let bound = if a == 0.0 && b == 0.0 {
                "none measured"
            } else if a > b {
                "candidate-bound"
            } else if b > a {
                "selection-bound"
            } else {
                "mixed"
            };
            lines.push(format!(
                "{label}: retrieval loss candidate {:.1}pp, selection {:.1}pp ({bound})",
                100.0 * a,
                100.0 * b
            ));
        }
    }
    let width = rows
        .iter()
        .map(|(t, _)| t.chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    let mut header = format!("{:width$}", "", width = width);
    for (label, _, _) in results {
        header.push_str(&format!("{label:>18}"));
    }
    if results.len() == 2 {
        header.push_str(&format!("{title:>18}"));
    }
    lines.push(String::new());
    lines.push(header);
    for (title, key) in rows {
        let mut row = format!("{title:<width$}", width = width);
        for s in &summaries {
            row.push_str(&format!("{:>18}", cell(s, &key)));
        }
        if results.len() == 2 {
            let measured = if ["prompts", "evaluated", "skipped_total"].contains(&key.as_str()) {
                &summaries
            } else {
                &paired
            };
            let delta = if measured.len() == 2 {
                value(&measured[0], &key)
                    .zip(value(&measured[1], &key))
                    .map(|(a, b)| {
                        let d = b - a;
                        if key.ends_with("_recall")
                            || ["miss_audit/candidate_recall", "miss_audit/selected_recall"]
                                .contains(&key.as_str())
                        {
                            format!("{:+.1}pp", d * 100.0)
                        } else if key.starts_with("mean")
                            || key.starts_with("latency")
                            || key.starts_with("index_latency")
                        {
                            format!("{d:+.3}")
                        } else {
                            format!("{:+}", d as i64)
                        }
                    })
                    .unwrap_or_else(|| "-".into())
            } else {
                "-".into()
            };
            row.push_str(&format!("{delta:>18}"));
        }
        lines.push(row);
    }
    for ((label, _, _), s) in results.iter().zip(&summaries) {
        if let Some(skipped) = s["skipped"].as_object().filter(|m| !m.is_empty()) {
            lines.push(format!(
                "skipped in {label}: {}",
                skipped
                    .iter()
                    .map(|(k, n)| format!("{k} {n}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    if results.len() == 2 {
        let status = comparison
            .as_ref()
            .and_then(|v| v["status"].as_str())
            .unwrap_or("unknown");
        lines.push(String::new());
        lines.push(if status=="compatible"{format!("paired over {} prompt(s) evaluated in both (B against A)",both.len())}else if status=="unknown"{format!("diagnostic ID-overlap over {} prompt(s) evaluated in both (B against A; compatibility unknown)",both.len())}else{format!("ID overlap: {} prompt(s) evaluated in both; paired gains withheld",both.len())});
        for (i, label) in ["A", "B"].iter().enumerate() {
            let only = evaluated(&results[i].2["items"])
                .difference(&both)
                .cloned()
                .collect::<Vec<_>>();
            if !only.is_empty() {
                lines.push(format!("  evaluated only in {label}: {}", only.join(", ")));
            }
        }
        if status != "incompatible" {
            for label in [
                "useful",
                "noise-only",
                "answer",
                "knowledge useful",
                "knowledge noise-only",
                "skill hit",
            ] {
                let test = |i: &Value| match label {
                    "useful" => i["class"] == "useful",
                    "noise-only" => i["class"] == "noise-only",
                    "answer" => i["answer_in_text"] == true,
                    "knowledge useful" | "knowledge noise-only" => {
                        let keep = |key: &str| {
                            i[key]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str)
                                .filter(|p| !scoring::skill(p))
                                .map(String::from)
                                .collect::<Vec<_>>()
                        };
                        scoring::classify(
                            &keep("delivered"),
                            &keep("useful_delivered"),
                            &keep("noise_delivered"),
                            i["query_withheld"] == true,
                        ) == if label == "knowledge useful" {
                            "useful"
                        } else {
                            "noise-only"
                        }
                    }
                    _ => i["useful_delivered"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .any(scoring::skill),
                };
                let gained = both
                    .iter()
                    .filter(|id| {
                        !test(&results[0].2["items"][*id]) && test(&results[1].2["items"][*id])
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                let lost = both
                    .iter()
                    .filter(|id| {
                        test(&results[0].2["items"][*id]) && !test(&results[1].2["items"][*id])
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                lines.push(format!("  {label:<11} +{} -{}", gained.len(), lost.len()));
                if !gained.is_empty() {
                    lines.push(format!("    gained: {}", gained.join(", ")));
                }
                if !lost.is_empty() {
                    lines.push(format!("    lost:   {}", lost.join(", ")));
                }
            }
        }
    }
    if show_unjudged {
        let pairs = results
            .iter()
            .flat_map(|(_, _, d)| {
                d["items"]
                    .as_object()
                    .into_iter()
                    .flat_map(|m| m.iter())
                    .flat_map(|(id, item)| {
                        item["unjudged"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(move |p| (id.clone(), p.to_string()))
                    })
            })
            .collect::<BTreeSet<_>>();
        lines.push(String::new());
        lines.push(format!(
            "{} delivered (id, path) pair(s) without a judgment:",
            pairs.len()
        ));
        lines.extend(pairs.into_iter().map(|(id, path)| format!("{id}\t{path}")));
    }
    Ok(lines.join("\n"))
}
pub fn legacy_outcome(s: &Value) -> String {
    let arms = s["compare"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    if arms.len() != 2 {
        return String::new();
    }
    let (b, t) = (arms[0], arms[1]);
    let target = if s["metric"] == "recall" {
        "the save's subject (needed file or an answer naming it)"
    } else {
        "the first gold file"
    };
    let mut lines = vec![format!(
        "pairs {} of {} tasks; a run that never reached it counts {}",
        s["pairs"],
        s["tasks"],
        n(&s["cap"]) + 1
    )];
    if n(&s["pairs"]) > 0 {
        lines.push(format!(
            "calls to {target}: {b} {}, {t} {}",
            s[format!("mean_{b}")],
            s[format!("mean_{t}")]
        ));
        lines.push(format!(
            "{t} - {b}: {} (95% CI {}), fewer {} / more {}, sign test p {}",
            s["mean_difference"],
            s["ci95"],
            s[format!("{t}_fewer")],
            s[format!("{t}_more")],
            s["sign_test_p"]
        ));
        lines.push(format!(
            "reached within the cap: {b} {}, {t} {}",
            s[format!("touched_{b}")],
            s[format!("touched_{t}")]
        ));
        if s["metric"] == "recall" {
            lines.push(format!(
                "save delivered by the hook: {b} {}, {t} {}",
                s[format!("saved_delivered_{b}")],
                s[format!("saved_delivered_{t}")]
            ));
        }
        if s.get("ticket_files_delivered").is_some() {
            lines.push(format!(
                "ticket files in the capsule: {} of {} pairs",
                s["ticket_files_delivered"], s["pairs"]
            ));
        }
        lines.push("cost, not part of the verdict:".into());
        for arm in arms {
            let c = &s["cost"][arm];
            let tokens = if n(&c["runs_with_usage"]) > 0 {
                format!(
                    "; over those, {} input ({} cached) and {} output tokens",
                    c["input_tokens_mean"], c["cached_input_tokens_mean"], c["output_tokens_mean"]
                )
            } else {
                String::new()
            };
            let printed = if !c["capsule_bytes_mean"].is_null() {
                format!(", printed {} bytes", c["capsule_bytes_mean"])
            } else {
                String::new()
            };
            lines.push(format!("  {arm}: {} s a run; Codex usage on {} of {} runs (a run stopped at the touch has none){tokens}; capsule documents estimate {} tokens{printed}",c["seconds_mean"],c["runs_with_usage"],c["runs"],c["capsule_tokens_mean"]));
        }
    }
    lines.push(format!(
        "verdict: {}",
        s["verdict"].as_str().unwrap_or("insufficient")
    ));
    lines.join("\n")
}

fn scalar(v: &Value) -> String {
    if v.is_null() {
        "-".into()
    } else if let Some(s) = v.as_str() {
        s.into()
    } else {
        v.to_string()
    }
}
fn table(rows: &[(&str, &str)], columns: &[(&str, &Value)], width: usize) -> Vec<String> {
    let mut out = vec![format!(
        "{:width$}{}",
        "",
        columns
            .iter()
            .map(|(s, _)| format!("{s:>9}"))
            .collect::<String>(),
        width = width
    )];
    for (title, key) in rows {
        out.push(format!(
            "{title:<width$}{}",
            columns
                .iter()
                .map(|(_, v)| format!("{:>9}", scalar(&v[*key])))
                .collect::<String>(),
            width = width
        ));
    }
    out
}
const NATIVE_ROWS: &[(&str, &str)] = &[
    ("prompt hook runs", "prompt_hook_runs"),
    ("  armed a save request", "armed"),
    ("    after an unsaved one", "armed_after_incomplete"),
    ("  continuation prompts", "continuations"),
    ("stop hook runs", "stop_hook_runs"),
    ("  timed out", "stop_hook_timeouts"),
    ("  exited non-zero", "stop_hook_failures"),
    ("save requested", "requested"),
    ("  to end without repeating", "requested_brief"),
    ("saved", "saved"),
    ("held", "held"),
    ("omitted", "omitted"),
    ("failed", "failed"),
    ("refused saves", "refused"),
    ("errors", "errors"),
    ("learnings written", "learnings_written"),
    ("chunks promoted", "chunks_promoted"),
    ("capture decision: selected", "capture_selected"),
    ("  no reusable knowledge", "capture_no_reusable_knowledge"),
    ("  insufficiently checked", "capture_insufficiently_checked"),
    ("  unknown", "capture_unknown"),
];
const REALIZED_ROWS: &[(&str, &str)] = &[
    ("transcripts read", "transcripts"),
    ("prompt-hook turns", "hook_turns"),
    (
        "  the refresh assembled no capsule",
        "turns_no_capsule_assembled",
    ),
    ("turns with a capsule", "turns_with_capsule"),
    ("turns with documents", "turns_with_documents"),
    ("delivered items", "delivered_items"),
    ("  touched by the agent", "delivered_touched"),
    ("  mentioned in its text", "delivered_mentioned"),
    (
        "turns touching a delivered item",
        "turns_touching_delivered",
    ),
    (
        "turns mentioning a delivered item",
        "turns_mentioning_delivered",
    ),
    ("other same-kind docs touched", "touched_not_delivered"),
    ("turns touching other docs", "turns_touching_other_docs"),
    ("task record pointers", "record_pointers"),
    ("  touched by the agent", "record_pointers_touched"),
    ("agent retrieve calls, CLI", "agent_retrieve_cli"),
    ("agent retrieve calls, MCP", "agent_retrieve_mcp"),
    ("agent context.py calls", "agent_context_cli"),
];
pub fn realized(v: &Value) -> String {
    let since = v["since"]
        .as_str()
        .map(|s| format!(" since {s}"))
        .unwrap_or_default();
    let mut lines = vec![format!(
        "Realized use of delivered memory: {}{since}",
        v["project"].as_str().unwrap_or("")
    )];
    if n(&v["claude"]["folders_skipped"]) > 0 {
        lines.push(format!("({} Claude Code folder(s) with the same name prefix belong to another project; skipped)",v["claude"]["folders_skipped"]));
    }
    if n(&v["codex"]["compressed_unread"]) > 0 {
        lines.push(format!(
            "({} compressed Codex rollout(s) could not be read: broken Zstandard stream)",
            v["codex"]["compressed_unread"]
        ));
    }
    let width = REALIZED_ROWS.iter().map(|r| r.0.len()).max().unwrap_or(0) + 2;
    let columns = [
        ("claude", &v["claude"]),
        ("codex", &v["codex"]),
        ("total", &v["total"]),
    ];
    lines.extend(table(REALIZED_ROWS, &columns, width));
    lines.extend([
        "".into(),
        "by kind, all hosts: delivered / touched / mentioned".into(),
    ]);
    for kind in crate::transcript::KINDS {
        let c = &v["total"]["by_kind"][kind];
        if n(&c["delivered"]) > 0 {
            lines.push(format!(
                "  {kind:<11} {:>6} {:>6} {:>6}",
                c["delivered"], c["touched"], c["mentioned"]
            ));
        }
    }
    if let Some(hooks) = v["claude"]["hooks"].as_object().filter(|m| !m.is_empty()) {
        lines.extend(["".into(),"Claude Code hook runs it recorded, by event: ok / errors (command not found) / blocked - a tool hook that succeeds quietly is not recorded".into()]);
        for (event, c) in hooks {
            lines.push(format!(
                "  {event:<18} {:>6} {:>6} ({}) {:>6}",
                c["ok"], c["error"], c["not_found"], c["blocked"]
            ));
        }
    }
    lines.extend([
        "".into(),
        "Explicit retrieves in whole sessions (CLI and Memory MCP):".into(),
    ]);
    if n(&v["retrieve"]["total"]["calls"]) == 0 {
        lines.push("  none".into());
    } else {
        let rows = [
            ("sessions that retrieved", "sessions"),
            ("retrieve calls", "calls"),
            ("answers read", "answers"),
            ("  naming a document", "answers_naming"),
            ("documents returned", "items_returned"),
            ("  opened afterwards", "items_opened"),
            ("task records pointed at", "records_pointed"),
            ("  opened afterwards", "records_opened"),
        ];
        let columns = [
            ("claude", &v["retrieve"]["claude"]),
            ("codex", &v["retrieve"]["codex"]),
            ("total", &v["retrieve"]["total"]),
        ];
        lines.extend(table(&rows, &columns, width));
        lines.push("by kind, all hosts: returned / opened afterwards / opened unnamed".into());
        for kind in crate::transcript::KINDS {
            let c = &v["retrieve"]["total"]["by_kind"][kind];
            if n(&c["returned"]) + n(&c["otherwise"]) > 0 {
                lines.push(format!(
                    "  {kind:<11} {:>6} {:>6} {:>6}",
                    c["returned"], c["opened"], c["otherwise"]
                ));
            }
        }
    }
    if v["traces"].is_object() {
        let t = &v["traces"];
        lines.extend([
            "".into(),
            format!("Render traces in the window: {}", t["traces"]),
            format!(
                "  hook renders handed over (seen in a transcript): {}",
                t["hook"]["handed"]
            ),
            format!(
                "  hook renders never seen in a transcript: {}",
                t["hook"]["not_seen"]
            ),
            format!(
                "  retrieve answers matched to a render: {} (retrieve renders unmatched: {})",
                t["retrieve"]["answered"], t["retrieve"]["not_seen"]
            ),
        ]);
        if n(&t["unknown_entry"]) > 0 {
            lines.push(format!(
                "  renders whose manifest is gone: {}",
                t["unknown_entry"]
            ));
        }
        let rows = [
            ("hook capsules in transcripts", "capsules"),
            ("  matched to a render trace", "with_trace"),
            ("  items those renders showed", "trace_items"),
            ("  of them touched", "trace_items_touched"),
        ];
        lines.extend(table(
            &rows,
            &[
                ("claude", &t["hosts"]["claude"]),
                ("codex", &t["hosts"]["codex"]),
            ],
            width,
        ));
    }
    let native = &v["native"];
    lines.push(String::new());
    if native["journal"] != true {
        lines.push("Native result funnel: no native-results.ndjson in memory-bank/local (a runtime before the journal, or no native step ran)".into());
    } else {
        lines.push(format!(
            "Native result funnel: {} journal line(s)",
            native["lines"]
        ));
        let mut columns = vec![
            ("claude", &native["hosts"]["claude"]),
            ("codex", &native["hosts"]["codex"]),
            ("cursor", &native["hosts"]["cursor"]),
        ];
        if native["hosts"]["unknown"]
            .as_object()
            .is_some_and(|m| m.values().any(|v| v.as_u64().is_some_and(|n| n > 0)))
        {
            columns.push(("unknown", &native["hosts"]["unknown"]));
        }
        columns.push(("total", &native["total"]));
        let width = NATIVE_ROWS.iter().map(|r| r.0.len()).max().unwrap_or(0) + 2;
        lines.extend(table(NATIVE_ROWS, &columns, width));
        if let Some(reasons) = native["total"]["reasons"]
            .as_object()
            .filter(|m| !m.is_empty())
        {
            lines.extend(["".into(), "codes, all hosts:".into()]);
            for (step, codes) in reasons {
                for (code, count) in codes.as_object().into_iter().flatten() {
                    lines.push(format!("  {step:<18} {code:<46} {count:>6}"));
                }
            }
        }
    }
    lines.push(format!(
        "prompt hook runs, all hosts (refresh-health.ndjson): {}",
        native["prompt_hook_health_lines"]
    ));
    lines.extend([
        "".into(),
        "Native save requests in the transcripts (median per request):".into(),
    ]);
    for (host, s) in v["continuations"].as_object().into_iter().flatten() {
        if n(&s["requests"]) == 0 {
            lines.push(format!("  {host:<7} none"));
            continue;
        }
        let share = s["time_share"]
            .as_f64()
            .map(|n| format!(", {:.0}% of the answer's time", n * 100.0))
            .unwrap_or_default();
        let input = if s.get("save_input_tokens").is_some() {
            format!(
                " (input {:.0}, {:.0} cached, on {})",
                s["save_input_tokens"].as_f64().unwrap_or(0.0),
                s["save_cached_input_tokens"].as_f64().unwrap_or(0.0),
                s["requests_with_input"]
            )
        } else {
            String::new()
        };
        let answer_input = if s.get("save_input_tokens").is_some() {
            format!(
                " (input {:.0}, {:.0} cached)",
                s["answer_input_tokens"].as_f64().unwrap_or(0.0),
                s["answer_cached_input_tokens"].as_f64().unwrap_or(0.0)
            )
        } else {
            String::new()
        };
        lines.push(format!("  {host:<7} {} request(s): +{} s and +{:.0} output tokens{input} after an answer of {} s and {:.0} tokens{answer_input}{share}",s["requests"],s["save_seconds"],s["save_output_tokens"].as_f64().unwrap_or(0.0),s["answer_seconds"],s["answer_output_tokens"].as_f64().unwrap_or(0.0)));
        for (ending, p) in s["endings"].as_object().into_iter().flatten() {
            lines.push(format!(
                "    {ending:<9} {} request(s): +{} s and +{:.0} output tokens",
                p["requests"],
                p["save_seconds"],
                p["save_output_tokens"].as_f64().unwrap_or(0.0)
            ));
        }
    }
    if let Some(paths) = v["paths"].as_object() {
        lines.extend(["".into(), "by path: delivered / touched / mentioned".into()]);
        let mut paths = paths.iter().collect::<Vec<_>>();
        paths.sort_by_key(|(path, c)| (std::cmp::Reverse(n(&c["delivered"])), (*path).clone()));
        for (path, c) in paths {
            lines.push(format!(
                "  {:>5} {:>5} {:>5}  {path}",
                c["delivered"], c["touched"], c["mentioned"]
            ));
        }
    }
    lines.join("\n")
}
pub fn field_check(v: &Value) -> String {
    let since = v["since"]
        .as_str()
        .map(|s| format!(" since {s}"))
        .unwrap_or_default();
    let mut lines = vec![format!(
        "Field check of the every-turn save{since}, {} project(s), counts only",
        v["projects"]
    )];
    for (host, c) in v["hosts"].as_object().into_iter().flatten() {
        let share = c["save_share"]
            .as_f64()
            .map(|s| format!("{:.0}%", s * 100.0))
            .unwrap_or_else(|| "-".into());
        lines.push(format!("  {host:<7} capsule turns {}, knowledge opened (K) {}, task records opened (T) {}, save requests {}, save share of the answer (S) {share}",c["capsule_turns"],c["knowledge_opened"],c["task_records_opened"],c["save_requests"]));
    }
    let hooks = &v["claude_hooks"];
    lines.push(format!(
        "  Claude Code hook runs recorded: {} ok, {} failed ({} not found), {} blocked",
        hooks["ok"], hooks["error"], hooks["not_found"], hooks["blocked"]
    ));
    lines.push(format!(
        "  prompt-hook turns whose refresh assembled no capsule: claude {}, codex {}",
        v["no_capsule_turns"]["claude"], v["no_capsule_turns"]["codex"]
    ));
    if v["rule"]["ready"] != true {
        lines.push(format!(
            "not ready: a host has fewer than {} capsule turns",
            v["min_turns"]
        ));
    }
    for (host, outcome) in v["rule"]["outcome"].as_object().into_iter().flatten() {
        lines.push(format!("  rule, {host}: {}", scalar(outcome)));
    }
    lines.join("\n")
}
pub fn install_status(v: &Value) -> String {
    let mut lines = vec![];
    for (name, s) in v["projects"].as_object().into_iter().flatten() {
        let release = [
            s["edition"].as_str().unwrap_or(""),
            s["release"].as_str().unwrap_or(""),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
        lines.push(format!(
            "{name} ({}; synced {} from {}: {})",
            if release.is_empty() {
                "no install record"
            } else {
                &release
            },
            scalar(&s["synced_at"]),
            scalar(&s["source"]),
            scalar(&s["clone"])
        ));
        let known = s["profiles_known"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!(
            "  capsule profile: {} (the runtime knows {})",
            scalar(&s["capsule_profile"]),
            if known.is_empty() { "-" } else { &known }
        ));
        lines.push(format!(
            "  last prompt hook: {}; last capsule: {}; last save step: {}{}",
            scalar(&s["last_prompt_hook"]),
            scalar(&s["last_capsule"]),
            scalar(&s["last_save_step"]),
            s["last_save_at"]
                .as_str()
                .map(|s| format!(" at {s}"))
                .unwrap_or_default()
        ));
        lines.push(format!(
            "  Claude Code policy: {}",
            scalar(&s["claude_policy"])
        ));
        let problems = s["problems"].as_array().cloned().unwrap_or_default();
        if problems.is_empty() {
            lines.push("  no problem found".into());
        } else {
            lines.extend(problems.iter().map(|p| format!("  problem: {}", scalar(p))));
        }
    }
    lines.join("\n")
}
pub fn funnel(v: &Value) -> String {
    let rows = [
        ("prompt hook runs", "hook_runs"),
        ("  timed out", "hook_timeouts"),
        ("  exited non-zero", "hook_failures"),
        ("capsules asked for, none built", "no_capsule"),
        ("capsules built", "capsules"),
        ("  shape recorded", "shaped"),
        ("  with the working state", "with_working"),
        ("  with recent files", "with_recent_files"),
        ("  with ticket files", "with_ticket_files"),
        ("  with documents", "with_documents"),
        ("  median tokens", "median_tokens"),
        ("  median bytes printed", "median_bytes"),
        ("save requests armed", "armed"),
        ("save requested at Stop", "requested"),
        ("saves made", "made"),
        ("  refused", "refused"),
        ("  failed or omitted", "failed_or_omitted"),
        ("learnings written", "learnings_written"),
        ("chunks promoted", "chunks_promoted"),
        ("task records", "task_records"),
        ("findings and decisions", "findings_decisions"),
        ("bank chunks", "bank_chunks"),
    ];
    let since = v["since"]
        .as_str()
        .map(|s| format!(" since {s}"))
        .unwrap_or_default();
    let mut lines = vec![format!(
        "Memory funnel{since}, from the journals (counts only)"
    )];
    let mut columns = v["projects"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.iter())
        .map(|(k, v)| (k.as_str(), v))
        .collect::<Vec<_>>();
    columns.push(("total", &v["total"]));
    let width = rows.iter().map(|r| r.0.len()).max().unwrap_or(0) + 2;
    let cell = columns
        .iter()
        .map(|(s, _)| s.len() + 2)
        .max()
        .unwrap_or(9)
        .max(9);
    lines.push(format!(
        "{:width$}{}",
        "",
        columns
            .iter()
            .map(|(s, _)| format!("{s:>cell$}"))
            .collect::<String>(),
        width = width
    ));
    for (title, key) in rows {
        lines.push(format!(
            "{title:<width$}{}",
            columns
                .iter()
                .map(|(_, v)| format!("{:>cell$}", scalar(&v[key])))
                .collect::<String>(),
            width = width
        ));
    }
    for (key, label, names) in [
        (
            "profiles",
            "profile",
            &["full", "navigation", "recall", "unknown"][..],
        ),
        (
            "hosts",
            "host",
            &["claude", "codex", "cursor", "cli", "unknown"][..],
        ),
    ] {
        lines.push(format!(
            "capsules by {label}: {}",
            names
                .iter()
                .map(|name| format!("{name} {}", v["total"][key][*name]))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let missing = columns
        .iter()
        .filter(|(n, v)| *n != "total" && v["save_journal"] != true)
        .map(|(n, _)| *n)
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        lines.push(format!(
            "no save journal (a runtime before it, or no save step ran): {}",
            missing.join(", ")
        ));
    }
    if let Some(reasons) = v["total"]["save_reasons"]
        .as_object()
        .filter(|m| !m.is_empty())
    {
        lines.push("save steps with a code, all projects:".into());
        for (step, codes) in reasons {
            lines.push(format!(
                "  {step}: {}",
                codes
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(code, n)| format!("{code} {n}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    lines.join("\n")
}
