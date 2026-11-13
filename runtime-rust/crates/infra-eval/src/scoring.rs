use crate::{Result, artifact};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
pub fn normalize(path: &str) -> String {
    let path = path.trim().replace('\\', "/");
    let path = path.strip_prefix("./").unwrap_or(&path).to_string();
    for prefix in [".claude/skills/", ".cursor/skills/", ".codex/skills/"] {
        if let Some(p) = path.strip_prefix(prefix) {
            return format!(".agents/skills/{p}");
        }
    }
    path
}
pub fn skill(path: &str) -> bool {
    normalize(path).starts_with(".agents/skills/")
}
pub fn flatten(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
pub fn passage_body(s: &str) -> String {
    s.lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}
pub fn answer_flatten(s: &str) -> String {
    let body = s
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    flatten(if body.is_empty() { s } else { &body })
}
pub fn judgments(raw: &Value) -> Result<BTreeMap<String, BTreeMap<String, u8>>> {
    let mut out = BTreeMap::new();
    for (id, grades) in raw
        .as_object()
        .ok_or_else(|| artifact::invalid("judgments must be object"))?
    {
        let mut clean = BTreeMap::<String, u8>::new();
        for (path, grade) in grades
            .as_object()
            .ok_or_else(|| artifact::invalid("prompt judgments must be object"))?
        {
            if path == "_note" && grade.is_string() {
                continue;
            }
            let grade = grade
                .as_u64()
                .filter(|n| *n <= 2)
                .ok_or_else(|| artifact::invalid("grade must be integer 0, 1 or 2"))?
                as u8;
            let key = normalize(path);
            clean
                .entry(key)
                .and_modify(|v| *v = (*v).max(grade))
                .or_insert(grade);
        }
        out.insert(id.clone(), clean);
    }
    Ok(out)
}
pub fn passages(raw: &Value) -> Result<BTreeMap<String, BTreeMap<String, Value>>> {
    let mut out = BTreeMap::new();
    for (id, docs) in raw
        .as_object()
        .ok_or_else(|| artifact::invalid("passages must be object"))?
    {
        let mut clean = BTreeMap::new();
        for (path, e) in docs
            .as_object()
            .ok_or_else(|| artifact::invalid("passage documents must be object"))?
        {
            if !e.is_object()
                || e.get("useful").is_some_and(|v| !v.is_boolean())
                || e.get("passages")
                    .is_some_and(|v| !v.as_array().is_some_and(|a| a.iter().all(Value::is_string)))
            {
                return Err(artifact::invalid("invalid passage entry"));
            }
            let key = normalize(path);
            if clean.get(&key).is_some_and(|prior| prior != e) {
                return Err(artifact::invalid("conflicting passage aliases"));
            }
            clean.insert(key, e.clone());
        }
        out.insert(id.clone(), clean);
    }
    Ok(out)
}
pub fn delivered(capsule: &Value) -> Vec<String> {
    let mut out = vec![];
    for layer in ["semantic", "episodic", "procedural"] {
        for item in capsule[layer].as_array().into_iter().flatten() {
            if let Some(s) = item["path"].as_str().filter(|s| !s.trim().is_empty()) {
                let s = normalize(s);
                if !out.contains(&s) {
                    out.push(s);
                }
            }
        }
    }
    out
}
pub fn labelled(paths: &[String], passages: &BTreeMap<String, Value>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for path in paths {
        if let Some(e) = passages.get(path) {
            if e["useful"] == false {
                continue;
            }
            for p in e["passages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if !passage_body(p).is_empty() {
                    let p = answer_flatten(p);
                    if !p.is_empty() {
                        out.insert(p);
                    }
                }
            }
        }
    }
    out
}
pub fn classify(
    delivered: &[String],
    useful: &[String],
    noise: &[String],
    withheld: bool,
) -> &'static str {
    if withheld || delivered.is_empty() {
        "silent"
    } else if !useful.is_empty() {
        "useful"
    } else if !noise.is_empty() {
        "noise-only"
    } else {
        "unjudged-only"
    }
}
pub fn score(
    result: &Value,
    grades: &BTreeMap<String, u8>,
    passages: &BTreeMap<String, Value>,
    existed: &[String],
    answers: &[String],
    expected: &BTreeSet<String>,
    deliverable: &BTreeSet<String>,
) -> Result<Value> {
    let withheld = result["query_withheld"] == true;
    let paths = delivered(&result["capsule"]);
    let useful = paths
        .iter()
        .filter(|p| grades.get(*p).is_some_and(|g| *g >= 1))
        .cloned()
        .collect::<Vec<_>>();
    let noise = paths
        .iter()
        .filter(|p| grades.get(*p) == Some(&0))
        .cloned()
        .collect::<Vec<_>>();
    let unjudged = paths
        .iter()
        .filter(|p| !grades.contains_key(*p))
        .cloned()
        .collect::<Vec<_>>();
    let text = result["capsule_text"].as_str().unwrap_or("");
    let hay = answer_flatten(text);
    let labels = labelled(&useful, passages);
    let delivered = labels
        .intersection(expected)
        .filter(|p| hay.contains(p.as_str()))
        .count();
    let possible = deliverable
        .intersection(expected)
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut kinds = json!({"semantic":{},"episodic":{},"procedural":{}});
    let mut selections = json!({"path-link":0,"source-link":0,"prompt-link":0,"touch-link":0});
    let (mut changed, mut subfiles) = (0, 0);
    for layer in ["semantic", "episodic", "procedural"] {
        for item in result["capsule"][layer].as_array().into_iter().flatten() {
            let k = item["kind"]
                .as_str()
                .filter(|s| {
                    s.len() <= 32
                        && s.bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                })
                .unwrap_or("other");
            kinds[layer][k] = json!(kinds[layer][k].as_u64().unwrap_or(0) + 1);
            if let Some(s) = item["selection"].as_str()
                && selections.get(s).is_some()
            {
                selections[s] = json!(selections[s].as_u64().unwrap_or(0) + 1);
            }
            if item["source_changed"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
                || item["source_changed"] == true
            {
                changed += 1;
            }
            if layer == "procedural"
                && item["path"].as_str().is_some_and(|p| {
                    skill(p) && Path::new(p).file_name().is_some_and(|n| n != "SKILL.md")
                })
            {
                subfiles += 1;
            }
        }
    }
    Ok(
        json!({"class":classify(&paths,&useful,&noise,withheld),"delivered":paths,"useful_delivered":useful,"noise_delivered":noise,"unjudged":unjudged,"existed_useful":existed,"could_help":!existed.is_empty(),"answer_existed":answers,"answer_could_help":!answers.is_empty(),"answer_in_text":labels.iter().any(|p|!p.is_empty()&&hay.contains(p)),"answer_passages_existed":expected.len(),"answer_passages_heading_only":answers.iter().filter_map(|p|passages.get(p)).flat_map(|v|v["passages"].as_array().into_iter().flatten()).filter_map(Value::as_str).filter(|p|passage_body(p).is_empty()).count(),"answer_passages_delivered":delivered,"answer_passages_deliverable":possible.len(),"answer_passages_delivered_deliverable":labels.intersection(&possible).filter(|p|hay.contains(p.as_str())).count(),"query_withheld":withheld,"capsule_chars":text.chars().count(),"layer_kinds":kinds,"selections":selections,"source_changed_delivered":changed,"skill_subfiles_delivered":subfiles,"capsule_secret_matches":usize::from(infra_core::domain::secret_label(text)?.is_some())}),
    )
}
pub fn percentile(values: &[f64], share: f64) -> Option<f64> {
    let mut v = values
        .iter()
        .copied()
        .filter(|v| v.is_finite() && *v >= 0.0)
        .collect::<Vec<_>>();
    v.sort_by(f64::total_cmp);
    let i = (share * v.len() as f64).ceil() as usize;
    v.get(i.saturating_sub(1))
        .map(|v| (v * 10000.0).round() / 10000.0)
}
fn paths(v: &Value, k: &str) -> Vec<String> {
    v[k].as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect()
}
pub fn summarize(items: &Value) -> Value {
    let rows = items
        .as_object()
        .map(|m| m.values().collect::<Vec<_>>())
        .unwrap_or_default();
    let evaluated = rows
        .iter()
        .copied()
        .filter(|i| i["status"] == "ok")
        .collect::<Vec<_>>();
    let mut skipped = BTreeMap::<String, usize>::new();
    for row in rows.iter().filter(|i| i["status"] != "ok") {
        *skipped
            .entry(row["reason"].as_str().unwrap_or("unknown").into())
            .or_default() += 1;
    }
    let count = |key: &str| evaluated.iter().filter(|i| i[key] == true).count();
    let class = |name: &str| evaluated.iter().filter(|i| i["class"] == name).count();
    let total = |key: &str| evaluated.iter().map(|i| paths(i, key).len()).sum::<usize>();
    let measured = |key: &str| {
        if !evaluated.is_empty() && evaluated.iter().all(|i| i[key].as_u64().is_some()) {
            Some(
                evaluated
                    .iter()
                    .map(|i| i[key].as_u64().unwrap())
                    .sum::<u64>(),
            )
        } else {
            None
        }
    };
    let existed = measured("answer_passages_existed");
    let delivered = measured("answer_passages_delivered");
    let possible = measured("answer_passages_deliverable");
    let delivered_possible = measured("answer_passages_delivered_deliverable");
    let mut result = json!({"prompts":rows.len(),"evaluated":evaluated.len(),"skipped":skipped,"classes":{"useful":class("useful"),"noise-only":class("noise-only"),"unjudged-only":class("unjudged-only"),"silent":class("silent")},"useful":class("useful"),"noise_only":class("noise-only"),"unjudged_only":class("unjudged-only"),"silent":class("silent"),"could_help":count("could_help"),"answer_could_help":count("answer_could_help"),"answer_in_text":count("answer_in_text"),"contaminated":count("contaminated"),"useful_among_could_help":evaluated.iter().filter(|i|i["could_help"]==true&&i["class"]=="useful").count(),"answer_in_text_among_answer_could_help":evaluated.iter().filter(|i|i["answer_could_help"]==true&&i["answer_in_text"]==true).count(),"answer_passages_existed":existed,"answer_passages_delivered":delivered,"answer_passage_recall":existed.zip(delivered).filter(|(e,_)|*e>0).map(|(e,d)|d as f64/e as f64),"answer_passages_deliverable":possible,"answer_passages_delivered_deliverable":delivered_possible,"deliverable_passage_recall":possible.zip(delivered_possible).filter(|(e,_)|*e>0).map(|(e,d)|d as f64/e as f64),"useful_paths_delivered":total("useful_delivered"),"useful_paths_existed":total("existed_useful"),"noise_paths":total("noise_delivered"),"unjudged_paths":total("unjudged"),"capsule_secret_matches":measured("capsule_secret_matches"),"source_changed_delivered":measured("source_changed_delivered"),"skill_subfiles_delivered":measured("skill_subfiles_delivered"),"capsule_chars_max":evaluated.iter().filter_map(|i|i["capsule_chars"].as_u64()).max()});
    for (out, key) in [
        ("mean_delivered", "delivered"),
        ("mean_noise", "noise_delivered"),
        ("mean_unjudged", "unjudged"),
    ] {
        result[out] = if evaluated.is_empty() {
            Value::Null
        } else {
            json!((total(key) as f64 / evaluated.len() as f64 * 1000.0).round() / 1000.0)
        };
    }
    for (out, key) in [("latency", "seconds"), ("index_latency", "index_seconds")] {
        let samples = evaluated
            .iter()
            .filter_map(|i| i["refresh"][key].as_f64().or_else(|| i[key].as_f64()))
            .collect::<Vec<_>>();
        result[format!("{out}_p50")] = json!(percentile(&samples, 0.5));
        result[format!("{out}_p95")] = json!(percentile(&samples, 0.95));
    }
    let mut knowledge = BTreeMap::<String, usize>::new();
    let mut skills = BTreeMap::<String, usize>::new();
    for item in &evaluated {
        let keep = |key: &str, want: bool| {
            paths(item, key)
                .into_iter()
                .filter(|p| skill(p) == want)
                .collect::<Vec<_>>()
        };
        let d = keep("delivered", false);
        let u = keep("useful_delivered", false);
        let n = keep("noise_delivered", false);
        *knowledge
            .entry(classify(&d, &u, &n, item["query_withheld"] == true).into())
            .or_default() += 1;
        let d = keep("delivered", true);
        let outcome = if !keep("existed_useful", true).is_empty() {
            if !keep("useful_delivered", true).is_empty() {
                "hit"
            } else if !d.is_empty() {
                "wrong"
            } else {
                "empty"
            }
        } else if d.is_empty() {
            "silent"
        } else if !keep("noise_delivered", true).is_empty() {
            "noise"
        } else {
            "unjudged"
        };
        *skills.entry(outcome.into()).or_default() += 1;
    }
    for (out, name) in [
        ("knowledge_useful", "useful"),
        ("knowledge_noise_only", "noise-only"),
        ("knowledge_unjudged_only", "unjudged-only"),
        ("knowledge_silent", "silent"),
    ] {
        result[out] = json!(knowledge.get(name).copied().unwrap_or(0));
    }
    for outcome in ["hit", "wrong", "empty", "silent", "noise", "unjudged"] {
        result[format!("skill_{outcome}")] = json!(skills.get(outcome).copied().unwrap_or(0));
    }
    result["skill_with_useful"] = json!(
        ["hit", "wrong", "empty"]
            .iter()
            .map(|k| skills.get(*k).copied().unwrap_or(0))
            .sum::<usize>()
    );
    result["skill_without_useful"] = json!(
        ["silent", "noise", "unjudged"]
            .iter()
            .map(|k| skills.get(*k).copied().unwrap_or(0))
            .sum::<usize>()
    );
    for (out, key) in [
        ("knowledge_useful_paths_delivered", "useful_delivered"),
        ("knowledge_useful_paths_existed", "existed_useful"),
        ("knowledge_noise_paths", "noise_delivered"),
    ] {
        result[out] = json!(
            evaluated
                .iter()
                .flat_map(|i| paths(i, key))
                .filter(|p| !skill(p))
                .count()
        );
    }
    result["knowledge_could_help"] = json!(
        evaluated
            .iter()
            .filter(|i| paths(i, "existed_useful").iter().any(|p| !skill(p)))
            .count()
    );
    augment(&mut result, &evaluated);
    result
}

pub fn index_stats(store: &infra_core::Store) -> Result<Value> {
    let documents = store
        .conn
        .query_row("SELECT count(*) FROM documents", [], |r| {
            r.get::<_, usize>(0)
        })?;
    let mut links = BTreeMap::<String, u64>::new();
    if let Ok(mut statement) = store
        .conn
        .prepare("SELECT ref_kind,count(*) FROM document_links GROUP BY ref_kind")
    {
        for row in statement.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))? {
            let (kind, n) = row?;
            let key = if kind.len() <= 32
                && kind
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            {
                kind
            } else {
                "other".into()
            };
            *links.entry(key).or_default() += n;
        }
    }
    let mut secrets = 0usize;
    let mut statement = store
        .conn
        .prepare("SELECT title,summary,content FROM documents")?;
    for row in statement.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (a, b, c) = row?;
        secrets +=
            usize::from(infra_core::domain::secret_label(&format!("{a}\n{b}\n{c}"))?.is_some());
    }
    Ok(json!({"documents":documents,"links":links,"secret_documents":secrets}))
}
fn rounded_ratio(a: u64, b: u64) -> Option<f64> {
    (b > 0).then(|| ((a as f64 / b as f64) * 10000.0).round() / 10000.0)
}
fn augment(result: &mut Value, evaluated: &[&Value]) {
    for key in [
        "answer_passages_heading_only",
        "answer_passages_long_document",
    ] {
        result[key] = if evaluated.is_empty() || evaluated.iter().any(|i| i[key].as_u64().is_none())
        {
            Value::Null
        } else {
            json!(
                evaluated
                    .iter()
                    .filter_map(|i| i[key].as_u64())
                    .sum::<u64>()
            )
        };
    }
    for (a, b, out) in [
        (
            "answer_passages_delivered",
            "answer_passages_existed",
            "answer_passage_recall",
        ),
        (
            "answer_passages_delivered_deliverable",
            "answer_passages_deliverable",
            "deliverable_passage_recall",
        ),
    ] {
        result[out] = json!(
            result[a]
                .as_u64()
                .zip(result[b].as_u64())
                .and_then(|(a, b)| rounded_ratio(a, b))
        );
    }
    if !evaluated.is_empty() && evaluated.iter().all(|i| i["layer_kinds"].is_object()) {
        let mut kinds = json!({"semantic":{},"episodic":{},"procedural":{}});
        let mut selections = json!({"path-link":0,"source-link":0,"prompt-link":0,"touch-link":0});
        for i in evaluated {
            for (layer, counts) in i["layer_kinds"].as_object().unwrap() {
                for (kind, n) in counts.as_object().into_iter().flatten() {
                    kinds[layer][kind] =
                        json!(kinds[layer][kind].as_u64().unwrap_or(0) + n.as_u64().unwrap_or(0));
                }
            }
            for (k, n) in i["selections"].as_object().into_iter().flatten() {
                if selections.get(k).is_some() {
                    selections[k] =
                        json!(selections[k].as_u64().unwrap_or(0) + n.as_u64().unwrap_or(0));
                }
            }
        }
        result["episodic_brain_events"] =
            json!(kinds["episodic"]["brain-event"].as_u64().unwrap_or(0));
        result["layer_kinds"] = kinds;
        result["selections"] = selections;
    } else {
        for key in ["layer_kinds", "selections", "episodic_brain_events"] {
            result[key] = Value::Null;
        }
    }
    if !evaluated.is_empty() && evaluated.iter().all(|i| i["refresh"]["index"].is_object()) {
        let mut links = BTreeMap::<String, u64>::new();
        let mut snapshots = 0;
        let mut secret = 0;
        for i in evaluated {
            let index = &i["refresh"]["index"];
            secret += index["secret_documents"].as_u64().unwrap_or(0);
            let mut has = false;
            for (k, n) in index["links"].as_object().into_iter().flatten() {
                let n = n.as_u64().unwrap_or(0);
                has |= n > 0;
                *links.entry(k.clone()).or_default() += n;
            }
            snapshots += usize::from(has);
        }
        result["index_link_rows"] = json!(links);
        result["index_snapshots_with_links"] = json!(snapshots);
        result["secret_documents"] = json!(secret);
    } else {
        for key in [
            "index_link_rows",
            "index_snapshots_with_links",
            "secret_documents",
        ] {
            result[key] = Value::Null;
        }
    }
    let audits = evaluated
        .iter()
        .filter_map(|i| i.get("miss_audit").filter(|v| v.is_object()))
        .collect::<Vec<_>>();
    if audits.is_empty() {
        return;
    }
    let entries = audits
        .iter()
        .flat_map(|a| a["items"].as_array().into_iter().flatten())
        .filter(|e| e["path"].is_string())
        .collect::<Vec<_>>();
    let mut stages = BTreeMap::<String, u64>::new();
    let mut answers = BTreeMap::<String, u64>::new();
    let mut groups = BTreeMap::from([
        ("knowledge", BTreeMap::<String, u64>::new()),
        ("skills", BTreeMap::new()),
    ]);
    let (mut eligible_answers, mut indexed, mut delivered) = (0u64, 0u64, 0u64);
    for e in &entries {
        let stage = e["stage"].as_str().unwrap_or("unknown");
        *stages.entry(stage.into()).or_default() += 1;
        *answers
            .entry(e["answer"].as_str().unwrap_or("unknown").into())
            .or_default() += 1;
        *groups
            .get_mut(if skill(e["path"].as_str().unwrap()) {
                "skills"
            } else {
                "knowledge"
            })
            .unwrap()
            .entry(stage.into())
            .or_default() += 1;
        eligible_answers += e["answer_counts"]["eligible"].as_u64().unwrap_or(0);
        indexed += e["answer_counts"]["indexed"].as_u64().unwrap_or(0);
        delivered += e["answer_counts"]["delivered"].as_u64().unwrap_or(0);
    }
    let count = |k: &str| stages.get(k).copied().unwrap_or(0);
    let unknown = ["unknown", "unknown-history", "unknown-query-withheld"]
        .iter()
        .map(|s| count(s))
        .sum::<u64>();
    let eligible = (entries.len() as u64)
        .saturating_sub(unknown + count("absent-source") + count("not-indexed"));
    let precandidate = count("no-fts-candidate") + count("window-limit");
    let selected = [
        "selected",
        "selected-answer-not-in-text",
        "budget-loss",
        "capsule-loss",
    ]
    .iter()
    .map(|s| count(s))
    .sum::<u64>();
    let history = entries
        .iter()
        .filter(|e| {
            e["history"] == true
                || e.get("history").is_none()
                    && [
                        "unknown-history",
                        "history-admission-drop",
                        "history-event-limit",
                    ]
                    .contains(&e["stage"].as_str().unwrap_or(""))
        })
        .collect::<Vec<_>>();
    let history_delivered = history
        .iter()
        .filter(|e| ["delivered", "partial-delivery"].contains(&e["answer"].as_str().unwrap_or("")))
        .count() as u64;
    result["miss_audit"] = json!({"prompts":audits.len(),"known_paths":entries.len(),"unknown_paths":unknown,"eligible_paths":eligible,"candidate_recall":rounded_ratio(eligible.saturating_sub(precandidate),eligible),"selected_recall":rounded_ratio(selected,eligible),"history_paths":history.len(),"history_delivered":history_delivered,"history_recall":rounded_ratio(history_delivered,history.len()as u64),"stages":stages,"answer_states":answers,"knowledge":groups["knowledge"],"skills":groups["skills"],"per_path_answer_passages":{"eligible":eligible_answers,"indexed":indexed,"delivered":delivered}});
}
