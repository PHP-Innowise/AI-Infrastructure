use crate::{Result, artifact, reconstruction, transcript};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
pub const TIERS: [(&str, f64); 5] = [
    ("input", 1.0),
    ("cache_write_5m", 1.25),
    ("cache_write_1h", 2.0),
    ("cache_read", 0.1),
    ("output", 5.0),
];
pub const DIMENSIONS: [(&str, &str); 5] = [
    ("skill", "attributionSkill"),
    ("mcp", "attributionMcpServer"),
    ("tool", "attributionMcpTool"),
    ("plugin", "attributionPlugin"),
    ("agent", "attributionAgent"),
];
pub fn tiers(usage: &Value) -> BTreeMap<String, u64> {
    let n = |k: &str| usage[k].as_u64().unwrap_or(0);
    let mut a = usage["cache_creation"]["ephemeral_5m_input_tokens"]
        .as_u64()
        .unwrap_or(0);
    let b = usage["cache_creation"]["ephemeral_1h_input_tokens"]
        .as_u64()
        .unwrap_or(0);
    if a == 0 && b == 0 {
        a = n("cache_creation_input_tokens");
    }
    BTreeMap::from([
        ("input".into(), n("input_tokens")),
        ("cache_write_5m".into(), a),
        ("cache_write_1h".into(), b),
        ("cache_read".into(), n("cache_read_input_tokens")),
        ("output".into(), n("output_tokens")),
    ])
}
pub fn bte(tiers: &BTreeMap<String, u64>) -> f64 {
    TIERS
        .iter()
        .map(|(k, w)| tiers.get(*k).copied().unwrap_or(0) as f64 * w)
        .sum()
}
pub fn attribute(root: &Path, project_filter: Option<&str>) -> Result<Value> {
    let mut value = attribute_full(root, project_filter)?;
    for key in [
        "_projects",
        "_project_calls",
        "_stratum_calls",
        "_dimension_calls",
    ] {
        value.as_object_mut().unwrap().remove(key);
    }
    Ok(value)
}
pub fn attribute_full(root: &Path, project_filter: Option<&str>) -> Result<Value> {
    if !root.is_dir() {
        return Err(artifact::invalid("no transcript directory"));
    }
    let mut total = BTreeMap::<String, u64>::new();
    let mut strata = BTreeMap::<String, f64>::new();
    let mut dimensions = BTreeMap::<String, BTreeMap<String, f64>>::new();
    for (k, _) in DIMENSIONS {
        dimensions.insert(k.into(), BTreeMap::new());
    }
    let mut calls = 0;
    let mut projects = BTreeMap::<String, f64>::new();
    let mut project_calls = BTreeMap::<String, u64>::new();
    let mut stratum_calls = BTreeMap::<String, u64>::new();
    let mut dimension_calls = BTreeMap::<String, BTreeMap<String, u64>>::new();
    for (name, _) in DIMENSIONS {
        dimension_calls.insert(name.into(), BTreeMap::new());
    }
    for path in transcript::files(root)? {
        if path.extension().is_some_and(|s| s == "zst") {
            continue;
        }
        let project = path
            .strip_prefix(root)
            .map_err(|_| artifact::invalid("transcript root"))?
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .unwrap_or_default();
        if project_filter.is_some_and(|s| !project.contains(s)) {
            continue;
        }
        let Ok(rows) = transcript::records(&path) else {
            continue;
        };
        let mut seen = BTreeMap::<String, usize>::new();
        let mut unique = vec![];
        let mut retained = vec![];
        for r in rows {
            if !r["message"]["usage"].is_object() {
                continue;
            }
            if !r["message"]["id"].is_null() {
                let id = serde_json::to_string(&r["message"]["id"])?;
                if let Some(i) = seen.get(&id) {
                    unique[*i] = r;
                } else {
                    seen.insert(id, unique.len());
                    unique.push(r);
                }
            } else {
                retained.push(r);
            }
        }
        retained.extend(unique);
        for r in retained {
            calls += 1;
            let t = tiers(&r["message"]["usage"]);
            let cost = bte(&t);
            *projects.entry(project.clone()).or_default() += cost;
            *project_calls.entry(project.clone()).or_default() += 1;
            *stratum_calls
                .entry(
                    if r["isSidechain"] == true {
                        "subagent"
                    } else {
                        "main"
                    }
                    .into(),
                )
                .or_default() += 1;
            for (k, n) in t {
                *total.entry(k).or_default() += n;
            }
            *strata
                .entry(
                    if r["isSidechain"] == true {
                        "subagent"
                    } else {
                        "main"
                    }
                    .into(),
                )
                .or_default() += cost;
            for (name, field) in DIMENSIONS {
                if let Some(value) = r
                    .get(field)
                    .filter(|v| !v.is_null() && **v != false && **v != "")
                {
                    let label = value
                        .as_str()
                        .map(String::from)
                        .unwrap_or_else(|| value.to_string());
                    *dimension_calls
                        .get_mut(name)
                        .unwrap()
                        .entry(label.clone())
                        .or_default() += 1;
                    *dimensions.get_mut(name).unwrap().entry(label).or_default() += cost;
                }
            }
        }
    }
    if calls == 0 {
        return Err(artifact::invalid("no billed calls found"));
    }
    Ok(
        json!({"calls":calls,"weights":TIERS.into_iter().collect::<BTreeMap<_,_>>(),"tiers":total,"total_bte":bte(&total),"strata":strata,"attribution":dimensions,"_projects":projects,"_project_calls":project_calls,"_stratum_calls":stratum_calls,"_dimension_calls":dimension_calls}),
    )
}
pub fn median(values: &[f64]) -> Option<f64> {
    let mut v = values
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .collect::<Vec<_>>();
    v.sort_by(f64::total_cmp);
    if v.is_empty() {
        None
    } else if v.len() % 2 == 1 {
        Some(v[v.len() / 2])
    } else {
        Some((v[v.len() / 2 - 1] + v[v.len() / 2]) / 2.0)
    }
}
pub fn continuation_costs(rows: &[Value], host: &str) -> Vec<Value> {
    #[derive(Clone)]
    struct Point {
        at: DateTime<Utc>,
        output: i64,
        input: i64,
        cached: i64,
    }
    let mut costs = vec![];
    let (mut prompt, mut pending) = (None::<Point>, None::<(Point, Value)>);
    let mut last = None;
    let (mut output, mut input, mut cached) = (0i64, 0i64, 0i64);
    let mut reports = false;
    let mut seen = BTreeSet::<String>::new();
    let close = |pending: &Option<(Point, Value)>,
                 last: Option<DateTime<Utc>>,
                 costs: &mut Vec<Value>,
                 output: i64,
                 input: i64,
                 cached: i64,
                 reports: bool| {
        if let (Some((point, value)), Some(last)) = (pending, last) {
            let mut value = value.clone();
            value["save_seconds"] =
                json!((last - point.at).num_milliseconds().max(0) as f64 / 1000.0);
            value["save_output_tokens"] = json!(output - point.output);
            if reports {
                value["save_input_tokens"] = json!(input - point.input);
                value["save_cached_input_tokens"] = json!(cached - point.cached);
            } else {
                value.as_object_mut().unwrap().remove("answer_input_tokens");
                value
                    .as_object_mut()
                    .unwrap()
                    .remove("answer_cached_input_tokens");
            }
            costs.push(value);
        }
    };
    for r in rows {
        let Some(at) = r["timestamp"].as_str().and_then(reconstruction::instant) else {
            continue;
        };
        let (human, request, asked) = if host == "codex" {
            let p = &r["payload"];
            if p["type"] == "token_count" {
                let u = &p["info"]["total_token_usage"];
                if let Some(n) = u["output_tokens"].as_i64() {
                    output = n;
                }
                if let Some(n) = u["input_tokens"].as_i64() {
                    reports = true;
                    input = n;
                    cached = u["cached_input_tokens"].as_i64().unwrap_or(0);
                }
            }
            let role = p["role"].as_str().unwrap_or("");
            let text = transcript::output_text(&p["content"]);
            let marker = text.contains("INTERNAL_NATIVE_MEMORY_RESULT ");
            (
                role == "user" && !marker && !text.trim_start().starts_with('<'),
                role == "user" && marker,
                text,
            )
        } else {
            let m = &r["message"];
            if r["type"] == "assistant" {
                let id = m["id"].as_str().unwrap_or("").to_string();
                if seen.insert(id) {
                    let u = &m["usage"];
                    if let Some(n) = u["output_tokens"].as_i64() {
                        output += n;
                        if let Some(n) = u["input_tokens"].as_i64() {
                            reports = true;
                            let read = u["cache_read_input_tokens"].as_i64().unwrap_or(0);
                            input +=
                                n + read + u["cache_creation_input_tokens"].as_i64().unwrap_or(0);
                            cached += read;
                        }
                    }
                }
            }
            let text = if r["type"] != "assistant" {
                r.to_string()
            } else {
                String::new()
            };
            let marker = text.contains("INTERNAL_NATIVE_MEMORY_RESULT ");
            (
                r["type"] == "user"
                    && r["toolUseResult"].is_null()
                    && r["isMeta"] != true
                    && m["content"].is_string()
                    && !marker,
                marker,
                text,
            )
        };
        if human {
            close(&pending, last, &mut costs, output, input, cached, reports);
            pending = None;
            prompt = Some(Point {
                at,
                output,
                input,
                cached,
            });
        } else if request
            && pending.is_none()
            && let Some(p) = &prompt
        {
            let ending = if asked.contains("has already read your answer above") {
                "brief"
            } else if asked.contains("return the substantive answer") {
                "repeat"
            } else {
                "earlier"
            };
            pending = Some((
                Point {
                    at,
                    output,
                    input,
                    cached,
                },
                json!({"ending":ending,"answer_seconds":(at-p.at).num_milliseconds().max(0)as f64/1000.0,"answer_output_tokens":output-p.output,"answer_input_tokens":input-p.input,"answer_cached_input_tokens":cached-p.cached}),
            ));
        }
        last = Some(at);
    }
    close(&pending, last, &mut costs, output, input, cached, reports);
    costs
}
pub fn continuation_summary(costs: &[Value]) -> Value {
    if costs.is_empty() {
        return json!({"requests":0});
    }
    let mut out = json!({"requests":costs.len()});
    for key in [
        "save_seconds",
        "save_output_tokens",
        "answer_seconds",
        "answer_output_tokens",
    ] {
        out[key] = json!(
            median(
                &costs
                    .iter()
                    .filter_map(|c| c[key].as_f64())
                    .collect::<Vec<_>>()
            )
            .map(|n| (n * 10.0).round() / 10.0)
        );
    }
    let shares = costs
        .iter()
        .filter_map(|c| {
            c["save_seconds"]
                .as_f64()
                .zip(c["answer_seconds"].as_f64())
                .filter(|(_, a)| *a > 0.0)
                .map(|(s, a)| s / a)
        })
        .collect::<Vec<_>>();
    out["time_share"] = json!(median(&shares).map(|n| (n * 100.0).round() / 100.0));
    let reported = costs
        .iter()
        .filter(|c| c.get("save_input_tokens").is_some())
        .collect::<Vec<_>>();
    if !reported.is_empty() {
        out["requests_with_input"] = json!(reported.len());
        for key in [
            "save_input_tokens",
            "save_cached_input_tokens",
            "answer_input_tokens",
            "answer_cached_input_tokens",
        ] {
            out[key] = json!(
                median(
                    &reported
                        .iter()
                        .filter_map(|c| c[key].as_f64())
                        .collect::<Vec<_>>()
                )
                .map(|n| (n * 10.0).round() / 10.0)
            );
        }
    }
    out
}

fn commas(n: u64) -> String {
    let text = n.to_string();
    let mut out = String::new();
    for (i, c) in text.chars().enumerate() {
        if i > 0 && (text.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
pub fn render_attribution(value: &Value, root: &Path, dimensions: &[String], top: isize) -> String {
    let mut lines = vec![
        format!(
            "{} billed calls under {}",
            commas(value["calls"].as_u64().unwrap_or(0)),
            root.display()
        ),
        format!(
            "BTE = base-token equivalent; weights {}",
            TIERS
                .iter()
                .map(|(name, w)| format!(
                    "{name} x{}",
                    if w.fract() == 0.0 {
                        format!("{w:.1}")
                    } else {
                        w.to_string()
                    }
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        String::new(),
        format!(
            "{:<44}{:>18}{:>18}{:>9}",
            "tier", "raw tokens", "BTE", "share"
        ),
    ];
    let total = value["total_bte"].as_f64().unwrap_or(0.0);
    let mut raw = 0;
    for (name, w) in TIERS {
        let n = value["tiers"][name].as_u64().unwrap_or(0);
        raw += n;
        let cost = n as f64 * w;
        lines.push(format!(
            "{name:<44}{:>18}{:>18}{:>8.1}%",
            commas(n),
            commas(cost.round_ties_even().max(0.0) as u64),
            if total == 0.0 {
                0.0
            } else {
                cost / total * 100.0
            }
        ));
    }
    lines.push(format!(
        "{:<44}{:>18}{:>18}",
        "TOTAL",
        commas(raw),
        commas(total.round_ties_even().max(0.0) as u64)
    ));
    fn report(title: &str, rows: &Value, calls: &Value, top: isize, lines: &mut Vec<String>) {
        let mut rows = rows
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, v)| (k, v.as_f64().unwrap_or(0.0)))
            .collect::<Vec<_>>();
        let total = rows.iter().map(|r| r.1).sum::<f64>();
        if total == 0.0 {
            lines.push(format!("\n{title}: no data"));
            return;
        }
        rows.sort_by(|a, b| b.1.total_cmp(&a.1));
        lines.push(format!("\n{title}"));
        lines.push(format!(
            "{:<44}{:>18}{:>9}{:>10}",
            "", "BTE", "share", "calls"
        ));
        let take = if top >= 0 {
            top as usize
        } else {
            rows.len().saturating_sub(top.unsigned_abs())
        };
        for (name, cost) in rows.into_iter().take(take) {
            let label = if name.chars().count() <= 43 {
                name.clone()
            } else {
                format!("{}...", name.chars().take(40).collect::<String>())
            };
            lines.push(format!(
                "{label:<44}{:>18}{:>8.1}%{:>10}",
                commas(cost.round_ties_even().max(0.0) as u64),
                cost / total * 100.0,
                commas(calls[name].as_u64().unwrap_or(0))
            ));
        }
        lines.push(format!(
            "{:<44}{:>18}",
            "TOTAL",
            commas(total.round_ties_even().max(0.0) as u64)
        ));
    }
    report(
        "BY STRATUM",
        &value["strata"],
        &value["_stratum_calls"],
        top,
        &mut lines,
    );
    report(
        "BY PROJECT",
        &value["_projects"],
        &value["_project_calls"],
        top,
        &mut lines,
    );
    let dimensions = if dimensions.is_empty() {
        let mut names = DIMENSIONS
            .iter()
            .map(|r| r.0.to_string())
            .collect::<Vec<_>>();
        names.sort();
        names
    } else {
        dimensions.to_vec()
    };
    for dimension in dimensions {
        if let Some((_, field)) = DIMENSIONS.iter().find(|(d, _)| *d == dimension) {
            report(
                &format!("BY {} ({field})", dimension.to_uppercase()),
                &value["attribution"][&dimension],
                &value["_dimension_calls"][&dimension],
                top,
                &mut lines,
            );
        }
    }
    lines.join("\n")
}
