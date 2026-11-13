use super::usage::LIMIT;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::collections::BTreeSet;
const DIMENSIONS: [(&str, &str); 3] = [
    ("tokens", "tokens"),
    ("usd_micros", "billing"),
    ("component_ns", "timing"),
];
fn checked(r: &Value) -> Result<&Value> {
    let horizon = r["horizon"]
        .as_u64()
        .filter(|n| (1..=512).contains(n))
        .context("comparison_schema")?;
    if r["profile"] != "memory-benchmark-ledger-v1"
        || r["usage_profile"] != "inclusive-input-output-v1"
        || serde_json::to_vec(r)?.len() > 262144
        || r["phases"].as_object().is_none_or(|m| {
            m.keys().map(String::as_str).collect::<BTreeSet<_>>()
                != ["build", "query", "reader", "judge"].into_iter().collect()
        })
    {
        bail!("comparison_schema");
    }
    if let Some(g) = r.get("grading") {
        let m = g.as_object().context("comparison_grading")?;
        if m.keys().map(String::as_str).collect::<BTreeSet<_>>()
            != [
                "observed",
                "known_correct",
                "correct",
                "complete",
                "conflicting",
            ]
            .into_iter()
            .collect()
        {
            bail!("comparison_grading");
        }
        let observed = g["observed"]
            .as_u64()
            .filter(|n| *n <= horizon)
            .context("comparison_grading")?;
        let known = g["known_correct"]
            .as_u64()
            .filter(|n| *n <= observed)
            .context("comparison_grading")?;
        let conflicts = g["conflicting"]
            .as_u64()
            .filter(|n| *n <= horizon)
            .context("comparison_grading")?;
        if observed + conflicts > horizon
            || g["complete"].as_bool() != Some(observed == horizon)
            || observed == horizon && g["correct"].as_u64() != Some(known)
            || observed != horizon && !g["correct"].is_null()
        {
            bail!("comparison_grading");
        }
    }
    for (phase, row) in r["phases"].as_object().unwrap() {
        let expected = if phase == "build" { 1 } else { horizon };
        let attempts = row["attempts"]
            .as_u64()
            .filter(|n| *n <= 4096)
            .context("comparison_schema")?;
        if row["expected_units"].as_u64() != Some(expected) || !row["overflow"].is_boolean() {
            bail!("comparison_schema");
        }
        let outcomes = row["outcomes"].as_object().context("comparison_schema")?;
        if outcomes.keys().map(String::as_str).collect::<BTreeSet<_>>()
            != ["succeeded", "failed", "cancelled", "pending", "conflict"]
                .into_iter()
                .collect()
            || outcomes
                .values()
                .any(|v| v.as_u64().is_none_or(|n| n > LIMIT))
            || outcomes
                .values()
                .map(|v| v.as_u64().unwrap() as u128)
                .sum::<u128>()
                != attempts as u128
        {
            bail!("comparison_schema");
        }
        for flag in ["coverage", "tokens", "billing", "timing"] {
            if !row["complete"][flag].is_boolean() {
                bail!("comparison_schema");
            }
        }
        for (metric, flag) in DIMENSIONS {
            if !row[metric].is_null() && row[metric].as_u64().is_none_or(|v| v > LIMIT)
                || row["complete"][flag] == true
                    && (row["complete"]["coverage"] != true
                        || row[metric].is_null()
                        || row["overflow"] == true)
            {
                bail!("comparison_schema");
            }
        }
        for k in [
            "input_tokens",
            "output_tokens",
            "cached_input_tokens",
            "reasoning_output_tokens",
        ] {
            if !row[k].is_null() && row[k].as_u64().is_none_or(|n| n > LIMIT) {
                bail!("comparison_schema");
            }
        }
        if row["complete"]["tokens"] == true {
            let i = row["input_tokens"].as_u64().context("comparison_schema")?;
            let o = row["output_tokens"].as_u64().context("comparison_schema")?;
            if i.checked_add(o) != row["tokens"].as_u64() {
                bail!("comparison_schema");
            }
        }
        for (part, total) in [
            ("cached_input_tokens", "input_tokens"),
            ("reasoning_output_tokens", "output_tokens"),
        ] {
            if let Some(p) = row[part].as_u64()
                && row[total].as_u64().is_none_or(|t| p > t)
            {
                bail!("comparison_schema");
            }
        }
        if row["complete"]["coverage"] == true
            && (row["closed_units"].as_u64() != Some(expected)
                || row["finished_attempts"].as_u64() != Some(attempts)
                || row["outcomes"]["pending"] != 0
                || row["outcomes"]["conflict"] != 0)
        {
            bail!("comparison_schema");
        }
    }
    Ok(r)
}
fn summed(rows: &[&Value], metric: &str) -> Option<u64> {
    let flag = DIMENSIONS.iter().find(|(m, _)| *m == metric)?.1;
    let mut sum = 0u128;
    for r in rows {
        if r["complete"]["coverage"] != true
            || r["complete"][flag] != true
            || r["overflow"] != false
        {
            return None;
        }
        sum += r[metric].as_u64()? as u128;
    }
    (sum <= LIMIT as u128).then_some(sum as u64)
}
pub fn compare_incremental(
    baseline: &Value,
    treatment: &Value,
    acquisition_baseline: &Value,
    acquisition_treatment: &Value,
) -> Result<Value> {
    let later = [checked(baseline)?, checked(treatment)?];
    let mut acquisition = vec![];
    for wrapper in [acquisition_baseline, acquisition_treatment] {
        let m = wrapper.as_object().context("acquisition_scope")?;
        if m.keys().map(String::as_str).collect::<BTreeSet<_>>()
            != ["scope", "common_task_excluded", "ledger"]
                .into_iter()
                .collect()
            || wrapper["scope"] != "memory-only"
            || wrapper["common_task_excluded"] != true
        {
            bail!("acquisition_scope");
        }
        acquisition.push(checked(&wrapper["ledger"])?);
    }
    let horizon = baseline["horizon"].as_u64().unwrap();
    if later
        .iter()
        .chain(acquisition.iter())
        .any(|r| r["horizon"].as_u64() != Some(horizon))
    {
        bail!("comparison_horizon");
    }
    let mut arms = json!({});
    for (i, name) in ["baseline", "treatment"].into_iter().enumerate() {
        let source = &acquisition[i]["phases"]["build"];
        let future = [&later[i]["phases"]["query"], &later[i]["phases"]["reader"]];
        let judge = &later[i]["phases"]["judge"];
        let correct = if later[i]["grading"]["complete"] == true {
            later[i]["grading"]["correct"].as_u64()
        } else {
            None
        };
        let mut outcomes = json!({});
        for k in ["succeeded", "failed", "cancelled", "pending", "conflict"] {
            outcomes[k] = json!(
                future
                    .iter()
                    .map(|r| r["outcomes"][k].as_u64().unwrap())
                    .sum::<u64>()
            );
        }
        let mut metrics = json!({});
        for (metric, _) in DIMENSIONS {
            let product = summed(&[source, future[0], future[1]], metric);
            metrics[metric] = json!({"acquisition":summed(&[source],metric),"future":summed(&future,metric),"product":product,"experiment":summed(&[source,future[0],future[1],judge],metric),"per_correct":correct.filter(|n|*n>0).and_then(|n|product.map(|v|v as f64/n as f64))});
        }
        arms[name] = json!({"acquisition_attempts":source["attempts"],"acquisition_outcomes":source["outcomes"],"product_attempts":source["attempts"].as_u64().unwrap()+future.iter().map(|r|r["attempts"].as_u64().unwrap()).sum::<u64>(),"future_outcomes":outcomes,"metrics":metrics});
    }
    let mut metrics = json!({});
    for (metric, _) in DIMENSIONS {
        let left = &arms["baseline"]["metrics"][metric];
        let right = &arms["treatment"]["metrics"][metric];
        let difference = |key: &str| {
            right[key]
                .as_i64()
                .zip(left[key].as_i64())
                .map(|(r, l)| r - l)
        };
        let delta = difference("product");
        metrics[metric] = json!({"baseline":left["product"],"treatment":right["product"],"incremental_acquisition":difference("acquisition"),"incremental_future":difference("future"),"incremental_total":delta,"amortized_incremental":delta.map(|d|d as f64/horizon as f64),"savings":delta.map(|d|-d)});
    }
    Ok(
        json!({"profile":"memory-incremental-comparison-v1","usage_profile":"inclusive-input-output-v1","evidence":"supplied-observations","acquisition_scope":"memory-only","common_task_excluded":true,"horizon":horizon,"arms":arms,"metrics":metrics}),
    )
}
