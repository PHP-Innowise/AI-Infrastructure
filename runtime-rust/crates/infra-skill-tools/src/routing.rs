//! Deliberate, billed model-selection evaluation; never a static route checker.
use crate::{Result, RuntimeError, common};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
pub const EDITIONS: [&str; 4] = ["Laravel", "Symfony", "PHP Core", "Cms/wordpress"];
pub fn label(expected: Option<&str>, observed: Option<&str>) -> &'static str {
    match (expected, observed) {
        (None, None) => "hit",
        (None, Some(_)) => "wrong",
        (Some(_), None) => "miss",
        (Some(a), Some(b)) if a == b => "hit",
        _ => "wrong",
    }
}
pub fn policy_digest(root: &Path, edition: &str) -> String {
    common::json_file(&root.join(edition).join(".accelerator-policy-lock.json"))
        .ok()
        .and_then(|v| v.get("policy_digest").map(common::display))
        .unwrap_or_else(|| "unknown".into())
}
pub fn baseline_path(root: &Path, edition: &str) -> Result<PathBuf> {
    let slug = match edition {
        "Laravel" => "laravel",
        "Symfony" => "symfony",
        "PHP Core" => "php-core",
        "Cms/wordpress" => "wordpress",
        _ => return Err(RuntimeError::invalid("Unsupported edition")),
    };
    Ok(root
        .join("install/policy-lock")
        .join(format!("{slug}-routing-baseline.json")))
}
pub fn selected_skill(
    root: &Path,
    edition: &str,
    request: &str,
    model: Option<&str>,
    timeout: Duration,
) -> Result<Option<String>> {
    let project = root.join(edition);
    let mut skills = common::sorted(&project.join(".agents/skills"))?
        .into_iter()
        .filter(|p| p.is_dir())
        .filter_map(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .collect::<Vec<_>>();
    // Stable lexical tie resolution replaces Python set iteration; persisted baselines identify this detection version.
    skills.sort();
    let mut argv = vec![
        "claude".into(),
        "-p".into(),
        request.into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--include-partial-messages".into(),
    ];
    if let Some(m) = model {
        argv.extend(["--model".into(), m.into()]);
    }
    let mut pending = String::new();
    let mut selected = None;
    let output = common::invoke(
        argv,
        &project,
        common::claude_env(),
        vec![],
        timeout,
        |event| {
            if event["type"] != "stream_event" {
                return None;
            }
            let inner = &event["event"];
            match inner["type"].as_str().unwrap_or("") {
                "content_block_start" => {
                    let block = &inner["content_block"];
                    if block["type"] == "tool_use"
                        && matches!(block["name"].as_str(), Some("Skill" | "Read"))
                    {
                        pending.clear();
                    }
                }
                "content_block_delta" if inner["delta"]["type"] == "input_json_delta" => {
                    pending.push_str(inner["delta"]["partial_json"].as_str().unwrap_or(""));
                    if let Some(slug) = skills.iter().find(|s| pending.contains(s.as_str())) {
                        selected = Some(slug.clone());
                        return Some("routing-observed".into());
                    }
                }
                _ => {}
            }
            None
        },
    )
    .map_err(|e| {
        RuntimeError::unavailable(format!(
            "the claude CLI is required; this command invokes a model: {e}"
        ))
    })?;
    if output.stopped == "interrupted" {
        return Err(RuntimeError::unavailable("Routing evaluation interrupted"));
    }
    Ok(selected)
}
pub fn evaluate(
    root: &Path,
    edition: &str,
    runs: usize,
    model: Option<&str>,
    timeout: Duration,
    dry_run: bool,
) -> Result<Value> {
    let path = root.join(edition).join(".agents/evals/routing.json");
    if !path.is_file() {
        return Err(RuntimeError::not_found(format!(
            "no routing eval data at {}",
            path.display()
        )));
    }
    let fixture = common::json_file(&path)?;
    let cases = fixture["cases"]
        .as_array()
        .ok_or_else(|| RuntimeError::invalid("Routing fixture requires cases"))?;
    let mut per_case = vec![];
    let (mut hit, mut miss, mut wrong) = (0, 0, 0);
    for case in cases {
        let request = case["request"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("Routing case request must be text"))?;
        let expected = if case["expect"].is_null() {
            None
        } else {
            Some(
                case["expect"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("Routing expect must be text or null"))?,
            )
        };
        let mut observed = vec![];
        let mut labels = vec![];
        let mut passed = 0;
        for _ in 0..runs {
            let actual = if dry_run {
                expected.map(str::to_owned)
            } else {
                selected_skill(root, edition, request, model, timeout)?
            };
            let verdict = label(expected, actual.as_deref());
            match verdict {
                "hit" => {
                    hit += 1;
                    passed += 1;
                }
                "miss" => miss += 1,
                _ => wrong += 1,
            };
            labels.push(verdict);
            observed.push(actual);
        }
        per_case.push(json!({"request":request,"expect":expected,"observed":observed,"labels":labels,"pass_rate":common::rounded(passed as f64/runs as f64)}));
    }
    let rates = per_case
        .iter()
        .map(|c| c["pass_rate"].as_f64().unwrap())
        .collect::<Vec<_>>();
    let mean = if rates.is_empty() {
        0.0
    } else {
        rates.iter().sum::<f64>() / rates.len() as f64
    };
    let sd = if rates.len() > 1 {
        (rates.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / rates.len() as f64).sqrt()
    } else {
        0.0
    };
    Ok(
        json!({"edition":edition,"policy_digest":policy_digest(root,edition),"runs":runs,"dry_run":dry_run,"cases":per_case.len(),"pass_rate":common::rounded(mean),"mean":common::rounded(mean),"stddev":common::rounded(sd),"labels":{"hit":hit,"miss":miss,"wrong":wrong},"per_case":per_case,"detection_profile":"streamed-tool-argument-lexical-slug-v1"}),
    )
}
pub fn execute(v: &Value) -> Result<Value> {
    let root = Path::new(
        common::text(v, "source_root")
            .or_else(|| common::text(v, "tooling_root"))
            .ok_or_else(|| RuntimeError::invalid("source_root is required"))?,
    );
    let runs = match common::positive(v, "runs", 3) {
        Ok(n) => n,
        Err(_) => {
            return Ok(common::cli(
                2,
                "",
                "routing_eval: --runs must be positive\n",
                Value::Null,
            ));
        }
    };
    let dry = common::flag(v, "dry_run");
    let write = common::flag(v, "write_baseline");
    if dry && write {
        return Ok(common::cli(
            2,
            "",
            "routing_eval: refusing to record a baseline from a dry run\n",
            Value::Null,
        ));
    }
    let editions = match v.get("edition") {
        None | Some(Value::Null) => EDITIONS.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| RuntimeError::invalid("edition requires strings"))
            })
            .collect::<Result<Vec<_>>>()?,
        _ => return Err(RuntimeError::invalid("edition requires a name or list")),
    };
    let mut results = serde_json::Map::new();
    let mut stdout = String::new();
    for edition in editions {
        if !EDITIONS.contains(&edition.as_str()) {
            return Err(RuntimeError::invalid("Unsupported routing edition"));
        }
        let result = match evaluate(
            root,
            &edition,
            runs,
            common::text(v, "model"),
            Duration::from_secs(common::positive(v, "timeout", 120)? as u64),
            dry,
        ) {
            Ok(r) => r,
            Err(e) => {
                return Ok(common::cli(
                    2,
                    stdout,
                    format!("routing_eval: {edition}: {e}\n"),
                    Value::Object(results),
                ));
            }
        };
        if write {
            let path = baseline_path(root, &edition)?;
            std::fs::create_dir_all(path.parent().unwrap())?;
            common::write_json(&path, &result, true)?;
        }
        if !common::flag(v, "json") {
            stdout.push_str(&format!(
                "{edition}\tpass {}\tstddev {}\thit {} miss {} wrong {}\tpolicy {}{}\n",
                result["pass_rate"],
                result["stddev"],
                result["labels"]["hit"],
                result["labels"]["miss"],
                result["labels"]["wrong"],
                common::chars(result["policy_digest"].as_str().unwrap_or("unknown"), 8),
                if dry { "\t(dry run)" } else { "" }
            ));
            for case in result["per_case"].as_array().unwrap() {
                if case["pass_rate"].as_f64().unwrap_or(0.0) < 1.0 {
                    let expected = case["expect"].as_str().unwrap_or("nothing");
                    let observations = case["observed"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| format!("{:?}", v.as_str().unwrap_or("nothing")))
                        .collect::<Vec<_>>()
                        .join(", ");
                    stdout.push_str(&format!(
                        "  {:>5}  expect {expected:<28} got [{observations}]\n",
                        case["pass_rate"]
                    ));
                }
            }
        }
        results.insert(edition, result);
    }
    let result = Value::Object(results);
    if common::flag(v, "json") {
        stdout = serde_json::to_string(&result)? + "\n";
    }
    Ok(common::cli(0, stdout, "", result))
}
