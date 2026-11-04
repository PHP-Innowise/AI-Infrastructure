//! Descriptive Kit 3 registry dossiers; a risk status never installs or forbids a tool.
use crate::common;
use infra_core::{Result, RuntimeError};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
pub const BINARY: [&str; 8] = [
    "license",
    "data_egress",
    "pinning",
    "uninstall",
    "collisions",
    "auto_update",
    "maintenance_ownership",
    "measurability",
];
pub const SCORED: [&str; 4] = [
    "automation_depth",
    "token_efficiency",
    "integration_coverage",
    "trust_signals",
];
const REQUIRED: [&str; 16] = [
    "schema_version",
    "id",
    "catalog_id",
    "name",
    "url",
    "reviewed_date",
    "reviewed_by",
    "status",
    "status_summary",
    "tier",
    "default_state",
    "install",
    "binary_gates",
    "scored_gates",
    "intersection_map",
    "lifecycle",
];
pub fn stated(value: &Value) -> bool {
    value.as_str().is_some_and(|value| !value.trim().is_empty())
}
pub fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
pub fn computed_status(gates: &Value) -> &'static str {
    if gates
        .as_object()
        .into_iter()
        .flat_map(|value| value.values())
        .any(|gate| gate["status"] == "fail")
    {
        "known_risks"
    } else if gates
        .as_object()
        .into_iter()
        .flat_map(|value| value.values())
        .any(|gate| gate["status"] == "unknown")
    {
        "open_questions"
    } else {
        "clear"
    }
}
pub fn load_catalog(root: &Path) -> Result<BTreeMap<String, Value>> {
    let data = common::read_json(&root.join("install/open-source-kit/resources.json"))?;
    let resources = data["resources"]
        .as_array()
        .ok_or_else(|| RuntimeError::invalid("Catalog resources must be a list"))?;
    let mut catalog = BTreeMap::new();
    for (index, entry) in resources.iter().enumerate() {
        let id = entry["id"]
            .as_str()
            .filter(|id| safe_id(id))
            .ok_or_else(|| {
                RuntimeError::invalid(format!("Catalog resource #{index} has no usable id"))
            })?;
        if catalog.insert(id.into(), entry.clone()).is_some() {
            return Err(RuntimeError::invalid(format!(
                "Duplicate catalog resource id: {id}"
            )));
        }
    }
    Ok(catalog)
}
pub fn validate_entry(data: &Value, catalog: &BTreeMap<String, Value>) -> Vec<String> {
    let Some(map) = data.as_object() else {
        return vec!["registry entry must contain a JSON object".into()];
    };
    let id = data["id"].as_str().unwrap_or("<no id>");
    let mut errors = Vec::new();
    let mut push = |message: String| errors.push(format!("{id}: {message}"));
    for field in &REQUIRED {
        if !map.contains_key(*field) {
            push(format!("missing required field '{}'", field));
        }
    }
    if !errors.is_empty() {
        return errors;
    }
    let mut raw = Vec::<String>::new();
    for field in ["id", "catalog_id"] {
        if data[field].as_str().is_none_or(|value| !safe_id(value)) {
            raw.push(format!("{field} must be a filename-safe lowercase slug"));
        }
    }
    if data["schema_version"].as_u64() != Some(1) {
        raw.push(format!(
            "unsupported schema_version {}",
            data["schema_version"]
        ));
    }
    if !matches!(data["tier"].as_u64(), Some(1..=3)) {
        raw.push("tier must be one of [1, 2, 3]".into());
    }
    if !matches!(data["default_state"].as_str(), Some("enabled" | "disabled")) {
        raw.push("default_state must be enabled or disabled".into());
    }
    if !matches!(
        data["status"].as_str(),
        Some("clear" | "open_questions" | "known_risks")
    ) {
        raw.push("status must be clear, open_questions or known_risks".into());
    }
    let expected = data["catalog_id"].as_str().and_then(|key| catalog.get(key));
    if let Some(expected) = expected {
        if data["id"] != data["catalog_id"] {
            raw.push("id does not match catalog_id".into());
        }
        for field in ["name", "url"] {
            if data[field] != expected[field] {
                raw.push(format!("{field} does not match catalog value"));
            }
        }
    } else {
        raw.push("catalog_id is not in install/open-source-kit/resources.json".into());
    }
    for block in ["install", "binary_gates", "scored_gates", "lifecycle"] {
        if !data[block].is_object() {
            raw.push(format!("'{block}' must be an object"));
        }
    }
    if !raw.is_empty() {
        return raw
            .into_iter()
            .map(|message| format!("{id}: {message}"))
            .collect();
    }
    for field in ["method", "pinned_ref", "namespace_prefix", "command"] {
        if data["install"].get(field).is_none() {
            raw.push(format!("install block missing '{field}'"));
        }
    }
    for field in ["uninstall_command", "residue", "manifest_tracked"] {
        if data["lifecycle"].get(field).is_none() {
            raw.push(format!("lifecycle block missing '{field}'"));
        }
    }
    if let Some(expected) = expected {
        for (field, binding) in [("method", "install_type"), ("command", "verified_command")] {
            if !stated(&expected[binding]) {
                raw.push(format!(
                    "catalog entry has no '{binding}' to bind install.{field}"
                ));
            } else if data["install"][field] != expected[binding] {
                raw.push(format!("install.{field} does not match catalog value"));
            }
        }
    }
    let binary = data["binary_gates"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let scored = data["scored_gates"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    for (name, expected, actual) in [
        ("binary_gates", BINARY.as_slice(), &binary),
        ("scored_gates", SCORED.as_slice(), &scored),
    ] {
        let wanted = expected.iter().copied().collect::<BTreeSet<_>>();
        let found = actual.keys().map(String::as_str).collect::<BTreeSet<_>>();
        let missing = wanted.difference(&found).copied().collect::<Vec<_>>();
        let extra = found.difference(&wanted).copied().collect::<Vec<_>>();
        if !missing.is_empty() {
            raw.push(format!("{name} missing: {missing:?}"));
        }
        if !extra.is_empty() {
            raw.push(format!("{name} has unknown gates: {extra:?}"));
        }
    }
    for (name, gate) in &binary {
        if !BINARY.contains(&name.as_str()) {
            continue;
        }
        if !gate.is_object() {
            raw.push(format!("gate '{name}' must be an object"));
            continue;
        }
        let status = gate["status"].as_str().unwrap_or("");
        if !["pass", "fail", "unknown"].contains(&status) {
            raw.push(format!(
                "gate '{name}' status must be pass, fail or unknown"
            ));
            continue;
        }
        if !stated(&gate["evidence"]) {
            raw.push(format!("gate '{name}' has no evidence"));
        }
        if ["fail", "unknown"].contains(&status) && !stated(&gate["resolves_by"]) {
            raw.push(format!("gate '{name}' is {status} but has no resolves_by"));
        }
    }
    for (name, gate) in &scored {
        if !SCORED.contains(&name.as_str()) {
            continue;
        }
        if !gate.is_object() {
            raw.push(format!("scored gate '{name}' must be an object"));
            continue;
        }
        if !matches!(gate["score"].as_u64(), Some(0..=5)) {
            raw.push(format!("scored gate '{name}' score must be an int 0-5"));
        }
        if !stated(&gate["evidence"]) {
            raw.push(format!("scored gate '{name}' has no evidence"));
        }
    }
    if let Some(disqualified) = data["scored_gates"]["trust_signals"].get("disqualified_signals")
        && disqualified
            .as_array()
            .is_none_or(|items| !items.iter().all(stated))
    {
        raw.push("trust_signals.disqualified_signals must be a list of non-empty strings".into());
    }
    if let Some(intersections) = data["intersection_map"]
        .as_array()
        .filter(|rows| !rows.is_empty())
    {
        for (index, row) in intersections.iter().enumerate() {
            if !row.is_object() {
                raw.push(format!("intersection_map[{index}] must be an object"));
                continue;
            }
            for field in ["ours", "relation", "detail"] {
                if !stated(&row[field]) {
                    raw.push(format!("intersection_map[{index}] missing '{field}'"));
                }
            }
            if !matches!(
                row["relation"].as_str(),
                Some("duplicates" | "replaces" | "conflicts" | "complements")
            ) {
                raw.push(format!("intersection_map[{index}] has invalid relation"));
            }
        }
    } else {
        raw.push("intersection_map must be a non-empty list".into());
    }
    if (data["scored_gates"]["token_efficiency"]["score"].as_u64() == Some(0)
        || data["scored_gates"]["token_efficiency"]["score"] == false)
        && data["binary_gates"]["measurability"]["status"] == "pass"
    {
        raw.push("token_efficiency scores 0 (unmeasured) but the measurability gate passes - one of the two is wrong".into());
    }
    if raw.is_empty() {
        let status = computed_status(&data["binary_gates"]);
        if data["status"] != status {
            raw.push(format!(
                "stored status disagrees with the gates, which compute '{status}'"
            ));
        }
    }
    raw.into_iter()
        .map(|message| format!("{id}: {message}"))
        .collect()
}
pub fn status_for(root: &Path, resource: &Value) -> Result<Option<String>> {
    let id = resource["id"]
        .as_str()
        .filter(|id| safe_id(id))
        .ok_or_else(|| RuntimeError::invalid("Unsafe resource ID"))?;
    let path = root
        .join("install/open-source-kit/registry")
        .join(format!("{id}.json"));
    if !path.is_file() {
        return Ok(None);
    }
    let data = common::read_json(&path)?;
    let errors = validate_entry(&data, &BTreeMap::from([(id.to_string(), resource.clone())]));
    if !errors.is_empty() {
        return Err(RuntimeError::invalid(format!(
            "Registry entry invalid for {id}: {}",
            errors.join("; ")
        )));
    }
    Ok(data["status"].as_str().map(String::from))
}
pub fn describe(data: &Value) -> String {
    let failing = BINARY
        .iter()
        .filter(|name| data["binary_gates"][**name]["status"] == "fail")
        .copied()
        .collect::<Vec<_>>();
    let unknown = BINARY
        .iter()
        .filter(|name| data["binary_gates"][**name]["status"] == "unknown")
        .copied()
        .collect::<Vec<_>>();
    let scores = SCORED
        .iter()
        .map(|name| format!("{name}={}", data["scored_gates"][*name]["score"]))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{}\t{}\t{}\n  tier {}, default {}, pinned_ref {}\n  risks found: {}\n  open questions: {}\n  scores: {}\n  intersections: {} recorded",
        data["status"].as_str().unwrap_or("").to_uppercase(),
        data["id"].as_str().unwrap_or(""),
        data["name"].as_str().unwrap_or(""),
        data["tier"],
        data["default_state"].as_str().unwrap_or(""),
        data["install"]["pinned_ref"].as_str().unwrap_or("NONE"),
        failing.join(", "),
        unknown.join(", "),
        scores,
        data["intersection_map"]
            .as_array()
            .map(Vec::len)
            .unwrap_or(0)
    )
}
pub fn execute(op: &str, request: &Value) -> Result<Value> {
    let root = PathBuf::from(
        request["root"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("registry root is required"))?,
    );
    let catalog = load_catalog(&root)?;
    if op == "validate-entry" {
        let errors = validate_entry(&request["entry"], &catalog);
        return Ok(
            json!({"valid":errors.is_empty(),"errors":errors,"computed_status":computed_status(&request["entry"]["binary_gates"])}),
        );
    }
    if op == "compute-status" {
        return Ok(json!({"status":computed_status(&request["binary_gates"])}));
    }
    if !["check", "validate", "describe", "list"].contains(&op) {
        return Err(RuntimeError::invalid("Unknown registry operation"));
    }
    let directory = root.join("install/open-source-kit/registry");
    if !directory.is_dir() {
        return Err(RuntimeError::not_found("Registry directory not found"));
    }
    let id = request.get("id").and_then(Value::as_str);
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(&directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("json")
            && id.is_none_or(|id| path.file_stem().and_then(|value| value.to_str()) == Some(id))
        {
            paths.push(path);
        }
    }
    paths.sort();
    if id.is_some() && paths.is_empty() {
        return Err(RuntimeError::not_found(
            "No registry entry with requested id",
        ));
    }
    let (mut entries, mut errors, mut dossiers, mut descriptions) =
        (0, Vec::new(), Vec::new(), Vec::new());
    for path in paths {
        let data = match common::read_json(&path) {
            Ok(data) => data,
            Err(error) => {
                errors.push(format!("{}: unreadable: {error}", path.display()));
                continue;
            }
        };
        entries += 1;
        if !data.is_object() {
            errors.push(format!("{}: must contain a JSON object", path.display()));
            continue;
        }
        if data["id"].as_str() != path.file_stem().and_then(|value| value.to_str()) {
            errors.push(format!(
                "{}: id does not match its filename",
                path.display()
            ));
        }
        let issues = validate_entry(&data, &catalog);
        if issues.is_empty() {
            descriptions.push(describe(&data));
        }
        errors.extend(issues);
        dossiers.push(data);
    }
    if (op == "check" || request["check"] == true) && entries == 0 && id.is_none() {
        errors.push(format!(
            "no registry entries found in {}",
            directory.display()
        ));
    }
    Ok(
        json!({"entries":entries,"valid":errors.is_empty(),"errors":errors,"dossiers":dossiers,"descriptions":descriptions}),
    )
}
