//! Historical metadata summaries; absent observations remain unknown.
use crate::{
    BankService, Result, RuntimeConfig, RuntimeContext, RuntimeError, Store, checkpoint,
    domain::{self, json},
    fs, observability,
};
use serde_json::{Value, json as object};
use std::{collections::BTreeMap, path::Path, time::Duration};
fn fail(message: &str) -> RuntimeError {
    RuntimeError::invalid(message)
}
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn pystr(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}
fn bump(map: &mut BTreeMap<String, u64>, key: impl Into<String>) {
    *map.entry(key.into()).or_default() += 1;
}
fn numeric(value: &Value, boolean: bool) -> Option<f64> {
    if boolean && let Some(value) = value.as_bool() {
        return Some(if value { 1.0 } else { 0.0 });
    }
    value.as_f64()
}
/// Exact decimal rounding of the represented binary64, with ties to even.
fn rounded(value: f64, digits: u32) -> f64 {
    if !value.is_finite() || digits > 18 {
        return value;
    }
    let bits = value.abs().to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let significand = if exponent == 0 {
        bits & ((1u64 << 52) - 1)
    } else {
        (bits & ((1u64 << 52) - 1)) | (1u64 << 52)
    };
    let power = (if exponent == 0 {
        -1074
    } else {
        exponent - 1023 - 52
    }) + digits as i32;
    if power >= 0 {
        return value;
    }
    let coefficient = significand as u128 * 5u128.pow(digits);
    let shift = (-power) as u32;
    let integer = if shift >= 128 {
        0
    } else {
        let quotient = coefficient >> shift;
        let remainder = coefficient & ((1u128 << shift) - 1);
        let half = 1u128 << (shift - 1);
        quotient + u128::from(remainder > half || remainder == half && quotient & 1 != 0)
    };
    let scale = 10u128.pow(digits);
    let sign = if value.is_sign_negative() { "-" } else { "" };
    let encoded = if digits == 0 {
        format!("{sign}{integer}")
    } else {
        format!(
            "{sign}{}.{:0width$}",
            integer / scale,
            integer % scale,
            width = digits as usize
        )
    };
    encoded.parse().unwrap_or(value)
}
fn percentiles(mut values: Vec<f64>) -> Value {
    if values.is_empty() {
        return object!({"samples":0,"p50":null,"p95":null,"max":null});
    }
    values.sort_by(f64::total_cmp);
    let at = |fraction: f64| {
        let index = (fraction * (values.len() - 1) as f64).round_ties_even() as usize;
        rounded(values[index.min(values.len() - 1)], 6)
    };
    object!({"samples":values.len(),"p50":at(0.5),"p95":at(0.95),"max":rounded(*values.last().unwrap_or(&0.0),6)})
}
fn rate(count: u64, total: u64) -> Value {
    if total == 0 {
        Value::Null
    } else {
        object!(rounded(count as f64 / total as f64, 4))
    }
}
fn lines(source: &str) -> impl Iterator<Item = &str> {
    source.split(|character| {
        [
            '\n', '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}',
            '\u{2029}',
        ]
        .contains(&character)
    })
}
fn json_files(directory: &Path) -> Vec<std::path::PathBuf> {
    if !fs::admit_directory(directory).unwrap_or(false) {
        return Vec::new();
    }
    std::fs::read_dir(directory)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|suffix| suffix == "json"))
        .collect()
}
pub fn read_manifests(
    context: &RuntimeContext,
    scope: &str,
    since: Option<usize>,
) -> Result<Vec<Value>> {
    if !["all", "governed", "local"].contains(&scope) {
        return Err(fail("Invalid manifest scope"));
    }
    let mut result = Vec::new();
    for (name, directory) in [
        (
            "governed",
            context
                .roots
                .brain_root()
                .join("control/retrieval-manifests"),
        ),
        (
            "local",
            context
                .roots
                .state
                .join("memory-bank/local/retrieval-manifests"),
        ),
    ] {
        if scope != "all" && scope != name {
            continue;
        }
        for path in json_files(&directory) {
            if let Ok(raw) = fs::read_text(&path)
                && let Ok(value) = json::parse(&raw)
                && value.is_object()
            {
                result.push(value);
            }
        }
    }
    result.sort_by(|left, right| text(right, "created_at").cmp(text(left, "created_at")));
    if let Some(limit) = since.filter(|limit| *limit > 0) {
        result.truncate(limit);
    }
    Ok(result)
}
pub fn retrieval_report(
    context: &RuntimeContext,
    scope: &str,
    since: Option<usize>,
) -> Result<Value> {
    let manifests = read_manifests(context, scope, since)?;
    let (
        mut reasons,
        mut paths,
        mut sources,
        mut hosts,
        mut entries,
        mut decisions,
        mut skip_reasons,
        mut no_match,
        mut strengths,
        mut versions,
    ) = (
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
    );
    let mut slices = BTreeMap::<(String, String, String, String, String), u64>::new();
    let (mut shadow, mut enforce, mut would_skip, mut withheld, mut empty, mut episode_count) =
        (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    let (mut top_candidate, mut top_delivered) = (Vec::new(), Vec::new());
    let mut tokens = BTreeMap::<String, Vec<f64>>::new();
    let mut phases = BTreeMap::<String, Vec<f64>>::from([
        ("stat".into(), Vec::new()),
        ("index".into(), Vec::new()),
        ("retrieval".into(), Vec::new()),
    ]);
    for manifest in &manifests {
        bump(&mut versions, pystr(&manifest["schema_version"]));
        let selected = manifest["selected"].as_array();
        let episodes = manifest["local_episode_count"].as_u64().unwrap_or(0);
        episode_count = episode_count.saturating_add(episodes);
        if selected.is_none_or(Vec::is_empty) && episodes == 0 {
            empty += 1;
        }
        let mut best = None::<f64>;
        for item in selected.into_iter().flatten() {
            if !item.is_object() {
                continue;
            }
            if let Some(value) = item.get("path") {
                let path = pystr(value);
                if !path.is_empty() {
                    bump(&mut paths, path);
                }
            }
            if let Some(strength) = item["match"].as_str() {
                bump(&mut strengths, strength);
            }
            if let Some(score) = numeric(&item["score"], true) {
                best = Some(best.map(|best| best.max(score)).unwrap_or(score));
            }
        }
        if let Some(best) = best {
            top_delivered.push(best);
        }
        for item in manifest["excluded"].as_array().into_iter().flatten() {
            if item.is_object() {
                bump(
                    &mut reasons,
                    item.get("reason").map(pystr).unwrap_or("unknown".into()),
                );
            }
        }
        let source = manifest["query_source"].as_str();
        if let Some(source) = source {
            bump(&mut sources, source);
        }
        let host = manifest["host"].as_str();
        if let Some(host) = host {
            bump(&mut hosts, host);
        }
        let entry = manifest["entry_point"].as_str();
        if let Some(entry) = entry {
            bump(&mut entries, entry);
        }
        if let Some(estimates) = manifest["token_estimates"].as_object() {
            for (name, value) in estimates {
                if let Some(value) = numeric(value, false) {
                    tokens.entry(name.clone()).or_default().push(value);
                }
            }
        }
        let gate = &manifest["gate"];
        if gate.is_object() {
            let decision = gate.get("decision").map(pystr).unwrap_or("unknown".into());
            let mode = gate.get("mode").map(pystr).unwrap_or("unknown".into());
            if ["shadow", "enforce"].contains(&mode.as_str()) {
                bump(&mut decisions, decision.clone());
                if mode == "shadow" {
                    shadow += 1;
                    if decision == "skip" {
                        would_skip += 1;
                    }
                } else {
                    enforce += 1;
                    if decision == "skip" {
                        withheld += 1;
                    }
                }
                *slices
                    .entry((
                        mode,
                        decision.clone(),
                        source.filter(|s| !s.is_empty()).unwrap_or("unknown").into(),
                        host.filter(|s| !s.is_empty()).unwrap_or("unknown").into(),
                        entry.filter(|s| !s.is_empty()).unwrap_or("unknown").into(),
                    ))
                    .or_default() += 1;
                if decision == "skip" {
                    bump(
                        &mut skip_reasons,
                        gate.get("reason").map(pystr).unwrap_or("unknown".into()),
                    );
                }
            }
            if let Some(score) = numeric(&gate["signals"]["top_score"], true) {
                top_candidate.push(score);
            }
            for layer in gate["signals"]["no_match"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                bump(&mut no_match, layer);
            }
        }
        if let Some(timings) = manifest["phase_seconds"].as_object() {
            for (name, values) in &mut phases {
                if let Some(value) = timings.get(name).and_then(|value| numeric(value, true)) {
                    values.push(value);
                }
            }
        }
    }
    let mut top_paths = paths.into_iter().collect::<Vec<_>>();
    top_paths.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    top_paths.truncate(20);
    let top_paths = top_paths
        .into_iter()
        .map(|(path, count)| (path, object!(count)))
        .collect::<serde_json::Map<_, _>>();
    let slice_values=slices.into_iter().map(|((mode,decision,query_source,host,entry_point),turns)|object!({"mode":mode,"decision":decision,"query_source":query_source,"host":host,"entry_point":entry_point,"turns":turns})).collect::<Vec<_>>();
    Ok(
        object!({"scope":scope,"turns":manifests.len(),"schema_versions":versions,"query_source":sources,"host":hosts,"entry_point":entries,"empty_selection":empty,"local_episode_count":episode_count,"top_paths":top_paths,"excluded_reasons":reasons,"match_strength":strengths,
        "gate":{"decided":decisions.values().sum::<u64>(),"decisions":decisions,"skip_reasons":skip_reasons,"shadow_decided":shadow,"would_skip":would_skip,"would_skip_rate":rate(would_skip,shadow),"enforce_decided":enforce,"withheld":withheld,"withheld_rate":rate(withheld,enforce),"slices":slice_values},
        "token_estimates":tokens.into_iter().map(|(key,values)|(key,percentiles(values))).collect::<BTreeMap<_,_>>(),"no_match":no_match,"top_candidate_score":percentiles(top_candidate),"top_delivered_score":percentiles(top_delivered),"phase_seconds":phases.into_iter().map(|(key,values)|(key,percentiles(values))).collect::<BTreeMap<_,_>>()}),
    )
}
fn summary_counter() -> Value {
    object!({"known_records":0,"unknown_records":0,"saturated_records":0,"sum_known_counts":null,"state":"unknown"})
}
fn slot_paths(template: &Value) -> Vec<Vec<String>> {
    let mut result = Vec::new();
    for group in ["loss_observations", "boundaries"] {
        for key in template[group]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(key, _)| key)
        {
            result.push(vec![group.into(), key.clone()]);
        }
    }
    result.push(vec!["render_projection".into(), "headers".into()]);
    for key in template["render_projection"]["reasons"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, _)| key)
    {
        result.push(vec![
            "render_projection".into(),
            "reasons".into(),
            key.clone(),
        ]);
    }
    result
}
fn slot<'a>(mut value: &'a Value, components: &[String]) -> &'a Value {
    for key in components {
        value = &value[key];
    }
    value
}
fn slot_mut<'a>(mut value: &'a mut Value, components: &[String]) -> &'a mut Value {
    for key in components {
        value = &mut value[key];
    }
    value
}
fn add(value: &mut Value, key: &str, count: u64) {
    value[key] = object!(value[key].as_u64().unwrap_or(0).saturating_add(count));
}
pub fn funnel_diagnosis_summary(records: &[Value]) -> Value {
    let selected = &records[records.len().saturating_sub(500)..];
    let template = observability::unknown_funnel_diagnosis();
    let mut summary = object!({"summary_schema_version":1,"records_considered":selected.len(),"records_omitted_by_cap":records.len()-selected.len(),"availability":{"observed":0,"unavailable":0,"missing":0,"rejected":0},"loss_observations":{},"boundaries":{},"render_projection":{"scope":template["render_projection"]["scope"],"headers":summary_counter(),"reasons":{}}});
    for key in [
        "source_coverage",
        "quality",
        "count_scope",
        "coverage",
        "emission",
        "consumer",
    ] {
        summary[key] = template[key].clone();
    }
    let paths = slot_paths(&template);
    for path in &paths {
        *slot_mut(&mut summary, path) = summary_counter();
    }
    for record in selected {
        let child = &record["funnel_diagnosis"];
        let availability = if record.get("funnel_diagnosis").is_none() {
            "missing"
        } else if observability::valid_funnel_diagnosis(child) {
            child["observation_state"].as_str().unwrap_or("rejected")
        } else {
            "rejected"
        };
        add(&mut summary["availability"], availability, 1);
        let observations = if availability == "observed" {
            child
        } else {
            &template
        };
        for path in &paths {
            let total = slot_mut(&mut summary, path);
            let observation = slot(observations, path);
            if let Some(count) = observation["count"].as_u64() {
                add(total, "known_records", 1);
                add(total, "sum_known_counts", count);
                if observation["state"] == "lower-bound" {
                    add(total, "saturated_records", 1);
                }
            } else {
                add(total, "unknown_records", 1);
            }
        }
    }
    for path in paths {
        let total = slot_mut(&mut summary, &path);
        if total["known_records"].as_u64().unwrap_or(0) > 0 {
            total["state"] = if total["unknown_records"].as_u64().unwrap_or(0) > 0
                || total["saturated_records"].as_u64().unwrap_or(0) > 0
            {
                "lower-bound"
            } else {
                "exact"
            }
            .into();
        }
    }
    summary
}
pub fn ordinary_receipt_summary(records: &[Value]) -> Value {
    let selected = &records[records.len().saturating_sub(500)..];
    let mut summary = object!({"summary_schema_version":1,"records_considered":selected.len(),"observed_records":0,"unavailable_records":0,"no_plan_skipped_records":0,"planned_skipped_records":0,"planned_deferred_records":0,"planned_committed_records":0});
    for record in selected {
        if let Some(child) = observability::normalize_ordinary_receipt(&record["ordinary_receipt"])
            .filter(|child| child["observation_state"] == "observed")
        {
            add(&mut summary, "observed_records", 1);
            let bucket = if child["plans_present"] == true {
                format!("planned_{}_records", text(&child, "returned_status"))
            } else {
                "no_plan_skipped_records".into()
            };
            add(&mut summary, &bucket, 1);
        } else {
            add(&mut summary, "unavailable_records", 1);
        }
    }
    summary
}
pub fn health(context: &RuntimeContext, window: Option<usize>) -> Result<Value> {
    let file = context
        .roots
        .state
        .join("memory-bank/local/refresh-health.ndjson");
    let bytes = fs::read_regular(&file).unwrap_or_default();
    let raw = String::from_utf8_lossy(&bytes);
    let mut records = lines(&raw)
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| json::parse(line).ok())
        .filter(Value::is_object)
        .collect::<Vec<_>>();
    if let Some(window) = window.filter(|window| *window > 0) {
        records = records[records.len().saturating_sub(window)..].to_vec();
    }
    let mut phases = BTreeMap::from([
        ("stat".to_string(), Vec::new()),
        ("index".into(), Vec::new()),
        ("retrieval".into(), Vec::new()),
    ]);
    let (mut timeouts, mut omitted, mut incomplete, mut notice_records) = (0u64, 0u64, 0u64, 0u64);
    let (mut statuses, mut notices) = (BTreeMap::new(), BTreeMap::<String, u64>::new());
    for record in &records {
        let mut seen_notice = false;
        for reason in ["invalid-utf8", "invalid-utf8-path"] {
            if let Some(count) = record["source_notices"][reason]
                .as_u64()
                .filter(|count| *count > 0)
            {
                *notices.entry(reason.into()).or_default() += count;
                seen_notice = true;
            }
        }
        if seen_notice {
            notice_records += 1;
        }
        if let Some(timings) = record["phase_seconds"].as_object() {
            for (name, values) in &mut phases {
                if let Some(value) = timings.get(name).and_then(|value| numeric(value, true)) {
                    values.push(value);
                }
            }
        }
        if let Some(status) = record.get("hook_status").filter(|status| !status.is_null()) {
            bump(&mut statuses, pystr(status));
            if status.as_i64() == Some(124) {
                timeouts += 1;
            }
        }
        if record["omitted"].as_object().is_some_and(|map| {
            map.values().any(|value| {
                value.as_i64().is_some_and(|count| count > 0) || value == &Value::Bool(true)
            })
        }) {
            omitted += 1;
        }
        if record["layers_updated"] == false {
            incomplete += 1;
        }
    }
    let total = records.len() as u64;
    let mut result = object!({"turns":total,"phase_seconds":phases.into_iter().map(|(name,values)|(name,percentiles(values))).collect::<BTreeMap<_,_>>(),"hook_status":statuses,"timeout_rate":rate(timeouts,total),"lossy_capsule_rate":rate(omitted,total),"incomplete_layers_rate":rate(incomplete,total),"funnel_diagnosis":funnel_diagnosis_summary(&records),"ordinary_receipt":ordinary_receipt_summary(&records)});
    if notice_records > 0 {
        result["source_notices"] = object!({"records":notice_records,"totals":notices});
    }
    Ok(result)
}
pub fn append_refresh(context: &RuntimeContext, record: &Value) {
    let operation = (|| -> Result<()> {
        let config = RuntimeConfig::load(&context.roots)?;
        let retention = config.value["refresh_health_retention"]
            .as_u64()
            .filter(|keep| *keep > 0)
            .unwrap_or(500) as usize;
        let file = context
            .roots
            .state
            .join("memory-bank/local/refresh-health.ndjson");
        let mut encoded = json::python_default(record, false, false, true)?.into_bytes();
        encoded.push(b'\n');
        fs::append_whole_line(&file, &encoded)?;
        let mut custody = crate::bank::publication::PublicationFiles::new(&context.roots.state)?;
        let Some(raw) = custody.read(
            "memory-bank/local/refresh-health.ndjson",
            usize::MAX.saturating_sub(1),
        )?
        else {
            return Ok(());
        };
        let text = String::from_utf8_lossy(&raw);
        let records = lines(&text)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        if records.len() > retention {
            let kept = format!("{}\n", records[records.len() - retention..].join("\n"));
            custody.write_expected_limit(
                "memory-bank/local/refresh-health.ndjson",
                kept.as_bytes(),
                Some(&raw),
                raw.len().max(kept.len()),
            )?;
        }
        Ok(())
    })();
    let _ = operation;
}

fn probe(context: &RuntimeContext, args: &[&str]) -> (Option<String>, Option<&'static str>) {
    match crate::git::run(
        &context.roots.project,
        &crate::git::strings(args),
        None,
        Duration::from_secs(2),
        65536,
    ) {
        Ok(output) if output.code == 0 => match String::from_utf8(output.stdout) {
            Ok(value) => (Some(value.trim().into()), None),
            Err(_) => (None, Some("git-probe-failed")),
        },
        Ok(_) => (None, Some("not-a-worktree")),
        Err(error) => (
            None,
            Some(if error.os_code == Some(2) {
                "git-unavailable"
            } else {
                "git-probe-failed"
            }),
        ),
    }
}
fn metadata_text(directory: &Path, relative: &str) -> Option<String> {
    let bytes = crate::result_store::read_bounded(&directory.join(relative), 4096)
        .ok()
        .flatten()?;
    String::from_utf8(bytes).ok()
}
fn special_identity(context: &RuntimeContext) -> (Option<String>, Option<String>, Option<String>) {
    let Some(frame) = crate::git::GitFrame::discover(&context.roots.project)
        .ok()
        .flatten()
    else {
        return (None, None, None);
    };
    for state in ["rebase-merge", "rebase-apply"] {
        if let Some(raw) = metadata_text(&frame.admin, &format!("{state}/head-name")) {
            let name = raw.trim_matches(['\r', '\n']);
            if let Some(branch) = name.strip_prefix("refs/heads/").filter(|branch| {
                !branch.is_empty() && !branch.chars().any(|c| ['\0', '\r', '\n'].contains(&c))
            }) {
                return (Some(branch.into()), None, None);
            }
        }
    }
    if let Some(raw) = metadata_text(&frame.admin, "BISECT_START") {
        let name = raw.trim_matches(['\r', '\n']);
        let oid = regex::Regex::new(r"^[0-9a-f]{40}(?:[0-9a-f]{24})?$")
            .is_ok_and(|pattern| pattern.is_match(name));
        if !name.is_empty() && !oid && !name.chars().any(|c| ['\0', '\r', '\n'].contains(&c)) {
            return (None, Some(name.into()), None);
        }
    }
    if let Some(raw) = metadata_text(&frame.admin, "codex-thread.json") {
        let version = regex::Regex::new(
            r#""version"[ \t\n\x0b\x0c\r]*:[ \t\n\x0b\x0c\r]*1[ \t\n\x0b\x0c\r]*[,}]"#,
        )
        .is_ok_and(|pattern| pattern.is_match(&raw));
        let owner=regex::Regex::new(r#""ownerThreadId"[ \t\n\x0b\x0c\r]*:[ \t\n\x0b\x0c\r]*"([0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12})""#).ok().and_then(|pattern|pattern.captures(&raw).map(|capture|capture[1].to_string()));
        if version {
            return (None, None, owner);
        }
    }
    (None, None, None)
}
pub fn branch_task_id(branch: &str) -> String {
    if crate::results::task_grammar(branch, 128) {
        return branch.into();
    }
    static ALLOWED: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static WORD: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let allowed = ALLOWED.get_or_init(|| {
        regex::Regex::new(r"[\p{L}\p{N}_\u{0300}-\u{036f}./#+-]").expect("constant branch grammar")
    });
    let word =
        WORD.get_or_init(|| regex::Regex::new(r"^[\p{L}\p{N}]$").expect("constant word grammar"));
    let mut stem = String::new();
    let mut refused = false;
    for character in branch.chars() {
        if allowed.is_match(&character.to_string()) {
            stem.push(character);
            refused = false;
        } else if !refused {
            stem.push('-');
            refused = true;
        }
    }
    let mut chars = stem
        .chars()
        .skip_while(|character| !word.is_match(&character.to_string()))
        .take(119)
        .collect::<String>();
    while chars.ends_with('-') {
        chars.pop();
    }
    if chars.is_empty() {
        chars = "branch".into();
    }
    format!("{chars}-{}", &crate::query::sha256(branch.as_bytes())[..8])
}
fn task_identity(source: &str, raw: &str, reason: &str, bad_reason: &str) -> Value {
    match crate::storage::validate_task_id(raw) {
        Ok(id) => {
            object!({"status":"active","source":source,"task_id":id,"reason":reason,"remediation":null})
        }
        Err(_) => {
            object!({"status":"degraded","source":source,"task_id":null,"reason":bad_reason,"remediation":"Set CONTEXT_TASK_ID to a valid task identifier."})
        }
    }
}
pub fn automatic_memory_readiness(context: &RuntimeContext) -> Result<Value> {
    let (worktree, git_error) = probe(context, &["rev-parse", "--show-toplevel"]);
    let mut branch = None;
    let mut state = git_error.unwrap_or("worktree");
    let mut failed_probe = None;
    if git_error.is_none() {
        let (branch_value, branch_error) =
            probe(context, &["symbolic-ref", "--quiet", "--short", "HEAD"]);
        branch = branch_value.filter(|value| !value.is_empty());
        if matches!(branch_error, Some("git-unavailable" | "git-probe-failed")) {
            branch = None;
            state = branch_error.unwrap_or("git-probe-failed");
            failed_probe = Some("branch");
        } else {
            let (head, head_error) = probe(context, &["rev-parse", "--verify", "HEAD"]);
            if matches!(head_error, Some("git-unavailable" | "git-probe-failed")) {
                if branch_error.is_none() {
                    state = "worktree";
                } else {
                    branch = None;
                    state = head_error.unwrap_or("git-probe-failed");
                    failed_probe = Some("HEAD-classification");
                }
            } else if head_error.is_none() && head.is_some_and(|head| !head.is_empty()) {
                state = if branch_error.is_none() {
                    "worktree"
                } else {
                    "detached-head"
                };
            } else {
                state = "unborn-head";
            }
        }
    }
    let (rebased, bisected, owner) = if state == "detached-head" {
        special_identity(context)
    } else {
        (None, None, None)
    };
    let git_metadata = if git_error.is_some() {
        object!({"status":"degraded","state":state,"worktree":null,"branch":null,"reason":"Git changed-path metadata is unavailable; turn checkpointing cannot run.","remediation":"Run the accelerator inside its Git worktree."})
    } else if ["git-unavailable", "git-probe-failed"].contains(&state) {
        object!({"status":"degraded","state":state,"worktree":worktree,"branch":null,"reason":if failed_probe==Some("branch"){"The Git branch probe failed before HEAD could be classified, so branch state is unknown for this report."}else{"The Git HEAD-classification probe failed after no branch was found, so HEAD state is unknown for this report."},"remediation":"Retry when Git responds within the probe timeout, or set CONTEXT_TASK_ID for task-aware automation."})
    } else {
        let reason = match state {
            "worktree" => "Git changed-path metadata is available.",
            "unborn-head" => {
                "Git changed-path metadata is available, but HEAD does not resolve to a commit yet (unborn branch)."
            }
            _ if rebased.is_some() => {
                "Git changed-path metadata is available; HEAD is detached by a rebase, and the branch it rewrites supplies task identity."
            }
            _ if bisected.is_some() => {
                "Git changed-path metadata is available; HEAD is detached by a bisect, and the branch it started from supplies task identity."
            }
            _ if owner.is_some() => {
                "Git changed-path metadata is available; HEAD is detached, and the Codex thread that owns this worktree supplies task identity."
            }
            _ => {
                "Git changed-path metadata is available, but HEAD is detached and cannot supply task identity."
            }
        };
        object!({"status":"active","state":state,"worktree":worktree,"branch":branch,"reason":reason,"remediation":if state=="detached-head"&&rebased.is_none()&&bisected.is_none()&&owner.is_none(){object!("Set CONTEXT_TASK_ID for task-aware automation while HEAD is detached.")}else{Value::Null}})
    };
    let explicit = std::env::var_os("CONTEXT_TASK_ID");
    let task = if explicit
        .as_ref()
        .is_some_and(|value| value.to_str().is_none_or(|value| !value.trim().is_empty()))
    {
        let raw = explicit
            .as_ref()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        task_identity(
            "environment",
            raw,
            "Explicit task identity is available for retrieval and dispatch.",
            "CONTEXT_TASK_ID is present but invalid.",
        )
    } else if let Some(branch) = &branch {
        task_identity(
            "git-branch",
            &branch_task_id(branch),
            "The current Git branch supplies task identity.",
            "The current Git branch is not a valid task identifier.",
        )
    } else if rebased.is_some() || bisected.is_some() {
        let (source, branch_of, ends, value) = if let Some(branch) = rebased.as_ref() {
            (
                "git-rebase",
                "a rebase in progress rewrites",
                "rebase",
                branch,
            )
        } else {
            (
                "git-bisect",
                "a bisect in progress started from",
                "bisect",
                bisected
                    .as_ref()
                    .ok_or_else(|| fail("Missing bisect identity"))?,
            )
        };
        task_identity(
            source,
            &branch_task_id(value),
            &format!("The branch {branch_of} supplies task identity until the {ends} ends."),
            &format!("The branch {branch_of} is not a valid task identifier."),
        )
    } else if let Some(owner) = owner {
        object!({"status":"active","source":"codex-thread","task_id":format!("codex-thread/{owner}"),"reason":"The Codex thread that owns this worktree supplies task identity until a branch does.","remediation":null})
    } else {
        object!({"status":"degraded","source":null,"task_id":null,"reason":"No task identity is available for task-aware retrieval, writes, or dispatch.","remediation":if git_error.is_none(){"Set CONTEXT_TASK_ID explicitly."}else{"Set CONTEXT_TASK_ID, and use a Git worktree to enable turn checkpointing."}})
    };
    let mut overall = if task["status"] == "active" && git_metadata["status"] == "active" {
        object!({"status":"active","reason":"Task identity and Git metadata are available; automatic memory is active.","remediation":null})
    } else if task["status"] == "active" {
        object!({"status":"retrieval-only","reason":"Task identity supports retrieval and dispatch, but Git metadata is unavailable so turn checkpointing is disabled.","remediation":"Run the accelerator inside its Git worktree."})
    } else {
        object!({"status":"degraded","reason":"Automatic task-aware memory is degraded because no usable task identity is available.","remediation":task["remediation"]})
    };
    overall["task_identity"] = task;
    overall["git_metadata"] = git_metadata;
    Ok(overall)
}
pub fn consolidation_counters(context: &RuntimeContext) -> Value {
    let mut result = object!({"promotable":null,"blocked":null,"applied":null,"chunks":null});
    if context.mode == "lightweight" || !context.roots.brain_root().is_dir() {
        return result;
    }
    let bank = BankService::new(context.clone());
    let counts = (|| -> Result<(usize, usize, usize)> {
        let (candidates, blocked) = crate::bank::promotion::status_candidates(&bank)?;
        let applied = crate::bank::promotion::list(&bank, false)?
            .iter()
            .filter(|value| value["status"] == "applied")
            .count();
        Ok((candidates.len(), blocked.len(), applied))
    })();
    let Ok((promotable, blocked, applied)) = counts else {
        return result;
    };
    result["promotable"] = object!(promotable);
    result["blocked"] = object!(blocked);
    result["applied"] = object!(applied);
    result["chunks"] = crate::bank::codec::render_index(&bank)
        .ok()
        .map(|(_, count)| object!(count))
        .unwrap_or(Value::Null);
    result
}
pub fn status(
    context: &RuntimeContext,
    store: &Store,
    database: &Path,
    _task: Option<&str>,
) -> Result<Value> {
    let mut layers = BTreeMap::from([
        ("procedural".to_string(), 0u64),
        ("semantic".into(), 0),
        ("episodic".into(), 0),
    ]);
    let mut query = store
        .conn
        .prepare("SELECT layer,COUNT(*) FROM documents GROUP BY layer")?;
    for row in query.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
    })? {
        let (layer, count) = row?;
        layers.insert(layer, count);
    }
    let count = |table: &str| -> Result<u64> {
        Ok(store
            .conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })?)
    };
    let mut result = object!({"documents":count("documents")?,"episodes":count("episodes")?,"working":count(if context.mode=="lightweight"{"working_tasks"}else{"task_bindings"})?,
        "mode":context.mode,"authority":if context.mode=="lightweight"{"local-sqlite"}else{"project-brain"},"layers":layers,"database":database,
        "automatic_memory":automatic_memory_readiness(context)?,"consolidation":consolidation_counters(context),
        "workspace":{"layout":if context.roots.shared{"shared"}else if context.roots.attached(){"attached"}else{"installed"},"tooling":context.roots.tooling,"project":context.roots.project,"state":context.roots.state}});
    if let Ok(paths) = checkpoint::status_paths(context) {
        let footprint = checkpoint::footprint(context);
        let owned = paths
            .iter()
            .filter(|path| {
                checkpoint::runtime_owned(path)
                    || checkpoint::installed_as_written(context, path, &footprint)
            })
            .count();
        result["uncommitted"] = object!(paths.len() - owned);
        result["accelerator_uncommitted"] = object!(owned);
    }
    Ok(result)
}

const TOKEN_FIELDS: &[&str] = &[
    "input_tokens",
    "output_tokens",
    "context_tokens",
    "cached_input_tokens",
];
const NUMBER_FIELDS: &[&str] = &["cost_usd", "ttfr_ms"];
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
        Value::Number(value) => value.as_f64().is_none_or(|value| value != 0.0),
    }
}
fn load_telemetry_config(path: &Path) -> Result<Value> {
    let value = json::parse(
        &fs::read_text(path).map_err(|_| fail("Telemetry configuration is unavailable"))?,
    )
    .map_err(|_| fail("Telemetry configuration is unavailable"))?;
    if !value.is_object() {
        return Err(fail("Telemetry configuration must be an object"));
    }
    Ok(value)
}
pub fn write_metadata_event(
    context: &RuntimeContext,
    metadata: &Value,
) -> Result<Option<std::path::PathBuf>> {
    let config = load_telemetry_config(
        &context
            .roots
            .state
            .join("project-brain/config/telemetry.json"),
    )?;
    let runtime = load_telemetry_config(
        &context
            .roots
            .state
            .join("project-brain/config/runtime.json"),
    )?;
    if !truthy(&config["enabled"]) || !truthy(&runtime["telemetry_enabled"]) {
        return Ok(None);
    }
    if !truthy(&config["metadata_only"]) {
        return Err(RuntimeError::policy("Telemetry must remain metadata-only"));
    }
    let metadata = metadata
        .as_object()
        .ok_or_else(|| fail("Telemetry metadata must be an object"))?;
    let prohibited = config["prohibited_fields"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let allowed = [
        "provider",
        "model",
        "operation",
        "input_tokens",
        "output_tokens",
        "context_tokens",
        "cached_input_tokens",
        "cost_usd",
        "ttfr_ms",
        "task_id",
    ];
    if metadata
        .keys()
        .any(|key| !allowed.contains(&key.as_str()) || prohibited.contains(key.as_str()))
    {
        return Err(RuntimeError::policy(
            "Telemetry contains prohibited or unknown fields",
        ));
    }
    let get = |key: &str| metadata.get(key).cloned().unwrap_or(Value::Null);
    let required = |key: &str, message: &str| -> Result<String> {
        get(key)
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(String::from)
            .ok_or_else(|| fail(message))
    };
    let provider = required("provider", "Telemetry provider is required")?;
    let operation = required("operation", "Telemetry operation is required")?;
    let raw_model = get("model");
    let model = if raw_model.is_null() {
        Value::Null
    } else {
        object!(
            raw_model
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| fail("Telemetry model must be a string or N/A"))?
        )
    };
    let raw_task = get("task_id");
    let task = if raw_task.is_null() {
        Value::Null
    } else {
        let raw = raw_task
            .as_str()
            .ok_or_else(|| fail("Telemetry task_id must be UUIDv4 or N/A"))?;
        let id = uuid::Uuid::parse_str(raw)
            .map_err(|_| fail("Telemetry task_id must be UUIDv4 or N/A"))?;
        if id.get_version_num() != 4 || id.get_variant() != uuid::Variant::RFC4122 {
            return Err(fail("Telemetry task_id must be UUIDv4 or N/A"));
        }
        object!(id.to_string())
    };
    let mut metrics = serde_json::Map::new();
    for field in TOKEN_FIELDS {
        let value = get(field);
        let integer = value.as_number().is_some_and(|number| {
            let raw = number.to_string();
            !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit())
        });
        if !value.is_null() && !integer {
            return Err(fail(&format!(
                "Telemetry metric {field} must be a nonnegative integer or N/A"
            )));
        }
        metrics.insert((*field).into(), value);
    }
    for field in NUMBER_FIELDS {
        let value = get(field);
        if !value.is_null()
            && !value
                .as_f64()
                .is_some_and(|number| number.is_finite() && number >= 0.0)
        {
            return Err(fail(&format!(
                "Telemetry metric {field} must be a finite nonnegative number or N/A"
            )));
        }
        metrics.insert((*field).into(), value);
    }
    let measured = metrics.values().all(|value| !value.is_null());
    let id = domain::uuid4();
    let mut event = object!({"schema_version":1,"id":id,"timestamp":domain::clock::proof_now().to_rfc3339_opts(chrono::SecondsFormat::Micros,false),"provider":provider,"model":model,"operation":operation,"availability":if measured{"measured"}else{"N/A"},"task_id":task});
    let fields = event
        .as_object_mut()
        .ok_or_else(|| fail("Telemetry envelope"))?;
    for (key, value) in metrics {
        fields.insert(key, value);
    }
    let relative = format!("memory-bank/local/telemetry/{id}.json");
    let encoded = format!("{}\n", json::python_default(&event, true, false, false)?);
    let mut files = crate::bank::publication::PublicationFiles::new(&context.roots.state)?;
    files.write_expected(&relative, encoded.as_bytes(), None)?;
    Ok(Some(context.roots.state.join(relative)))
}
pub fn telemetry(context: &RuntimeContext, capsule: Option<&crate::Capsule>, seconds: f64) {
    let operation = (|| -> Result<()> {
        if !seconds.is_finite() {
            return Ok(());
        }
        let config = RuntimeConfig::load(&context.roots)?;
        let provider = config
            .value
            .get("provider")
            .filter(|value| truthy(value))
            .map(pystr)
            .unwrap_or("sqlite-fts5".into());
        let mut payload = object!({"operation":"refresh","provider":provider,"ttfr_ms":rounded(seconds*1000.0,0).max(0.0) as u64});
        if let Some(capsule) = capsule {
            if let Some(task) = capsule.0["task_uuid"]
                .as_str()
                .filter(|task| !task.is_empty())
            {
                payload["task_id"] = object!(task);
            }
            if let Some(total) = capsule.0["token_estimates"]["total"].as_number() {
                let raw = total.to_string();
                if !raw.contains(['.', 'e', 'E']) {
                    payload["context_tokens"] = Value::Number(total.clone());
                }
            }
        }
        if let Some(file) = write_metadata_event(context, &payload)? {
            let event = json::parse(&fs::read_text(&file)?)?;
            domain::validate_schema_file(&context.roots, "token-usage-event.schema.json", &event)?;
        }
        Ok(())
    })();
    let _ = operation;
}

#[cfg(test)]
mod telemetry_contract_tests {
    use super::{telemetry, write_metadata_event};
    use crate::{Capsule, Roots, RuntimeContext};
    use serde_json::{Value, json};
    use std::{
        collections::{BTreeMap, BTreeSet},
        path::{Path, PathBuf},
    };

    struct Fixture {
        root: PathBuf,
        context: RuntimeContext,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "infra-telemetry-contract-{}",
                crate::domain::uuid4()
            ));
            std::fs::create_dir(&root).unwrap();
            let root = std::fs::canonicalize(root).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            std::fs::create_dir_all(root.join("project-brain/config")).unwrap();
            std::fs::create_dir_all(root.join("project-brain/schemas")).unwrap();
            std::fs::copy(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../PHP Core/project-brain/schemas/token-usage-event.schema.json"),
                root.join("project-brain/schemas/token-usage-event.schema.json"),
            )
            .unwrap();
            let context = RuntimeContext::new(
                Roots::installed(&root).unwrap(),
                "telemetry-fixture",
                "governed",
            )
            .unwrap();
            let fixture = Self { root, context };
            fixture.configure(true, true);
            fixture
        }
        fn configure(&self, adapter: bool, runtime: bool) {
            std::fs::write(
                self.root.join("project-brain/config/runtime.json"),
                serde_json::to_vec(&json!({
                    "mode":"governed", "telemetry_enabled":runtime,
                    "provider":"sqlite-fts5", "automatic_promotion":false
                }))
                .unwrap(),
            )
            .unwrap();
            std::fs::write(
                self.root.join("project-brain/config/telemetry.json"),
                serde_json::to_vec(&json!({
                    "enabled":adapter, "metadata_only":true,
                    "prohibited_fields":[
                        "prompt","response","source_body","tool_payload",
                        "secret","customer_data","raw_log"
                    ],
                    "schema":"../schemas/token-usage-event.schema.json"
                }))
                .unwrap(),
            )
            .unwrap();
        }
        fn image(&self) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
            fn visit(
                root: &Path,
                directory: &Path,
                image: &mut BTreeMap<PathBuf, Option<Vec<u8>>>,
            ) {
                for entry in std::fs::read_dir(directory).unwrap() {
                    let entry = entry.unwrap();
                    let path = entry.path();
                    let metadata = std::fs::symlink_metadata(&path).unwrap();
                    assert!(
                        !metadata.is_symlink(),
                        "fixture unexpectedly acquired a link"
                    );
                    let relative = path.strip_prefix(root).unwrap().to_path_buf();
                    if metadata.is_dir() {
                        image.insert(relative, None);
                        visit(root, &path, image);
                    } else {
                        assert!(metadata.is_file());
                        image.insert(relative, Some(std::fs::read(&path).unwrap()));
                    }
                }
            }
            let mut image = BTreeMap::new();
            visit(&self.root, &self.root, &mut image);
            image
        }
        fn events(&self) -> Vec<Value> {
            let path = self.root.join("memory-bank/local/telemetry");
            match std::fs::read_dir(path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => vec![],
                Err(error) => panic!("telemetry fixture directory: {error}"),
                Ok(entries) => {
                    let mut entries = entries
                        .map(|entry| entry.unwrap().path())
                        .collect::<Vec<_>>();
                    entries.sort();
                    entries
                        .into_iter()
                        .map(|path| {
                            assert_eq!(
                                path.extension().and_then(|value| value.to_str()),
                                Some("json")
                            );
                            let event: Value =
                                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
                            crate::domain::validate_schema_file(
                                &self.context.roots,
                                "token-usage-event.schema.json",
                                &event,
                            )
                            .unwrap();
                            event
                        })
                        .collect()
                }
            }
        }
        fn assert_event(&self, event: &Value) {
            let fields = event
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            assert_eq!(
                fields,
                BTreeSet::from([
                    "schema_version",
                    "id",
                    "timestamp",
                    "provider",
                    "model",
                    "operation",
                    "input_tokens",
                    "output_tokens",
                    "context_tokens",
                    "cached_input_tokens",
                    "cost_usd",
                    "ttfr_ms",
                    "availability",
                    "task_id",
                ])
            );
            assert_eq!(event["schema_version"], 1);
            let id = uuid::Uuid::parse_str(event["id"].as_str().unwrap()).unwrap();
            assert_eq!(id.get_version_num(), 4);
            assert_eq!(id.get_variant(), uuid::Variant::RFC4122);
            chrono::DateTime::parse_from_rfc3339(event["timestamp"].as_str().unwrap()).unwrap();
            crate::domain::validate_schema_file(
                &self.context.roots,
                "token-usage-event.schema.json",
                event,
            )
            .unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    const RAW_PAYLOAD: &str =
        "private_fixture_prompt_response_customer_source_log_must_not_be_written";

    fn capsule() -> Capsule {
        Capsule(
            json!({
                "task_uuid":"F47AC10B-58CC-4372-A567-0E02B2C3D479",
                "task_id":"TASK-EXTERNAL-ALIAS",
                "token_estimates":{"total":512},
                "prompt":RAW_PAYLOAD, "response":RAW_PAYLOAD,
                "source_body":RAW_PAYLOAD, "raw_log":RAW_PAYLOAD,
                "tool_payload":{"customer_data":RAW_PAYLOAD},
                "provider":"capsule-must-not-override-the-configured-provider",
                "operation":"capsule-must-not-override-refresh"
            }),
            None,
            None,
        )
    }

    #[test]
    fn telemetry_configured_off_preserves_state_through_both_gates_and_refresh_adapter() {
        let fixture = Fixture::new();
        for (adapter, runtime) in [(false, false), (true, false), (false, true)] {
            fixture.configure(adapter, runtime);
            let before = fixture.image();
            // Disabled collection returns before payload admission; it must not
            // create a directory or report a payload-validation failure.
            let off = write_metadata_event(
                &fixture.context,
                &json!({
                    "provider":"native", "operation":"retrieve", "prompt":RAW_PAYLOAD
                }),
            )
            .unwrap();
            assert!(off.is_none());
            telemetry(&fixture.context, Some(&capsule()), 0.020);
            assert_eq!(fixture.image(), before);
            assert!(fixture.events().is_empty());
            assert!(!fixture.root.join("memory-bank").exists());
        }
    }

    #[test]
    fn telemetry_sanitized_metadata_preserves_known_metrics_and_refuses_payload_or_invalid_values()
    {
        let fixture = Fixture::new();
        let frozen: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../qa/tc-ai/fixtures/telemetry-events.json"
        )))
        .unwrap();
        let adapter = frozen["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|event| event["id"] == "sanitized-adapter")
            .unwrap();
        assert_eq!(adapter["contains_prompt_content"], false);
        assert_eq!(adapter["input_tokens"], 120);
        assert_eq!(adapter["output_tokens"], 30);
        assert!(adapter["cost"].is_null());
        let partial = json!({
            "provider":" native ", "model":" host-managed ", "operation":" retrieve ",
            "input_tokens":adapter["input_tokens"], "output_tokens":adapter["output_tokens"],
            "cost_usd":adapter["cost"],
            "task_id":"F47AC10B-58CC-4372-A567-0E02B2C3D479"
        });
        let path = write_metadata_event(&fixture.context, &partial)
            .unwrap()
            .unwrap();
        assert_eq!(
            path.parent().unwrap(),
            fixture.root.join("memory-bank/local/telemetry")
        );
        let event: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        fixture.assert_event(&event);
        assert_eq!(event["provider"], "native");
        assert_eq!(event["model"], "host-managed");
        assert_eq!(event["operation"], "retrieve");
        assert_eq!(event["input_tokens"], 120);
        assert_eq!(event["output_tokens"], 30);
        assert_eq!(event["task_id"], "f47ac10b-58cc-4372-a567-0e02b2c3d479");
        // Frozen known counts do not supply unknown cost/context/cached/timing.
        // Both retained and native event profiles require all six metrics for
        // availability=measured; never replace the frozen null with a zero.
        assert_eq!(event["availability"], "N/A");
        for field in [
            "cost_usd",
            "context_tokens",
            "cached_input_tokens",
            "ttfr_ms",
        ] {
            assert!(event[field].is_null(), "{field}");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let before = fixture.image();
        for field in [
            "prompt",
            "response",
            "source_body",
            "tool_payload",
            "secret",
            "customer_data",
            "raw_log",
            "messages",
            "id",
            "timestamp",
            "availability",
        ] {
            let mut rejected = partial.clone();
            rejected[field] = RAW_PAYLOAD.into();
            let error = write_metadata_event(&fixture.context, &rejected).unwrap_err();
            assert!(error.message.contains("prohibited or unknown"));
            assert!(!error.message.contains(RAW_PAYLOAD));
            assert_eq!(fixture.image(), before, "{field}");
        }
        for (field, value) in [
            ("input_tokens", json!(1.5)),
            ("output_tokens", json!(true)),
            ("context_tokens", json!(-1)),
            ("cached_input_tokens", json!("3")),
            ("cost_usd", json!(true)),
            ("ttfr_ms", json!(-0.1)),
            ("cost_usd", serde_json::from_str::<Value>("1e400").unwrap()),
            ("model", json!(false)),
            ("task_id", json!("TASK-EXTERNAL-ALIAS")),
        ] {
            let mut rejected = partial.clone();
            rejected[field] = value;
            let error = write_metadata_event(&fixture.context, &rejected).unwrap_err();
            assert!(!error.message.contains(RAW_PAYLOAD));
            assert_eq!(fixture.image(), before, "{field}");
        }
        let config_path = fixture.root.join("project-brain/config/telemetry.json");
        let mut config: Value =
            serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
        config["metadata_only"] = false.into();
        std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        let unapproved = fixture.image();
        let error = write_metadata_event(&fixture.context, &partial).unwrap_err();
        assert!(error.message.contains("metadata-only"));
        assert!(!error.message.contains(RAW_PAYLOAD));
        assert_eq!(fixture.image(), unapproved);
        fixture.configure(true, true);
        assert_eq!(fixture.image(), before);
        let complete = json!({
            "provider":"native", "operation":"retrieve",
            "input_tokens":120, "output_tokens":30, "context_tokens":512,
            "cached_input_tokens":0, "cost_usd":0.025, "ttfr_ms":20
        });
        let path = write_metadata_event(&fixture.context, &complete)
            .unwrap()
            .unwrap();
        let measured: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        fixture.assert_event(&measured);
        assert_eq!(measured["availability"], "measured");
        assert_eq!(fixture.events().len(), 2);
        assert!(
            !serde_json::to_string(&fixture.events())
                .unwrap()
                .contains(RAW_PAYLOAD)
        );
    }

    #[test]
    fn telemetry_sanitized_refresh_adapter_excludes_capsule_bodies_and_is_best_effort() {
        let fixture = Fixture::new();
        telemetry(&fixture.context, Some(&capsule()), 0.020);
        let events = fixture.events();
        assert_eq!(events.len(), 1);
        let event = &events[0];
        fixture.assert_event(event);
        assert_eq!(event["provider"], "sqlite-fts5");
        assert_eq!(event["operation"], "refresh");
        assert_eq!(event["task_id"], "f47ac10b-58cc-4372-a567-0e02b2c3d479");
        assert_eq!(event["context_tokens"], 512);
        assert_eq!(event["ttfr_ms"], 20);
        assert_eq!(event["availability"], "N/A");
        for field in [
            "input_tokens",
            "output_tokens",
            "cached_input_tokens",
            "cost_usd",
            "model",
        ] {
            assert!(event[field].is_null(), "{field}");
        }
        let encoded = serde_json::to_string(event).unwrap();
        for excluded in [
            RAW_PAYLOAD,
            "TASK-EXTERNAL-ALIAS",
            "capsule-must-not-override-the-configured-provider",
            "capsule-must-not-override-refresh",
        ] {
            assert!(!encoded.contains(excluded));
        }
        let before = fixture.image();
        let mut invalid = capsule();
        invalid.0["task_uuid"] = "TASK-EXTERNAL-ALIAS".into();
        telemetry(&fixture.context, Some(&invalid), 0.020);
        assert_eq!(
            fixture.image(),
            before,
            "invalid metadata must not create a partial event"
        );
        std::fs::remove_file(fixture.root.join("project-brain/config/telemetry.json")).unwrap();
        let missing_config = fixture.image();
        telemetry(&fixture.context, Some(&capsule()), 0.020);
        assert_eq!(
            fixture.image(),
            missing_config,
            "missing optional config must not fail a refresh"
        );
    }
}
