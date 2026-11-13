use infra_core::{Result, RuntimeError};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug)]
pub struct Arguments {
    pub operation: String,
    pub value: Value,
    pub raw: Vec<String>,
}
pub fn key(value: &str) -> String {
    value.replace('-', "_")
}
const SWITCHES: &[&str] = &[
    "accept-source-changes",
    "accept-unrun-checks",
    "allow-downgrade",
    "allow-partial-skills",
    "allow-phase-regression",
    "app-server",
    "apply",
    "approve",
    "attached",
    "audit-git",
    "check",
    "complete",
    "copy",
    "cross-edition",
    "desktop",
    "diagnose-misses",
    "diagnostic",
    "dry-run",
    "enabled",
    "enter-worktree",
    "entered",
    "ephemeral",
    "evidence",
    "fail-fast",
    "flush",
    "force",
    "global",
    "headroom",
    "include-archive",
    "include-generator",
    "include-oracles",
    "include-superseded",
    "include-transcript",
    "incremental",
    "inside",
    "json",
    "keep",
    "list",
    "merge-existing",
    "no-chain",
    "no-retry",
    "no-scripts",
    "offline",
    "overwrite",
    "paths",
    "permissions",
    "plan-only",
    "prefix",
    "provenance",
    "provenance-response",
    "query-withheld",
    "read-only",
    "recorded",
    "refresh",
    "reject",
    "release",
    "remote",
    "replace-next-steps",
    "report-json",
    "require-php",
    "require-ticket-commits",
    "reverify",
    "rewire-codex",
    "sandbox",
    "sanitize",
    "show-unjudged",
    "skills-only",
    "skip",
    "skip-ledger",
    "skip-run-evidence",
    "skip-workbook",
    "standalone",
    "stdout",
    "strict",
    "summary",
    "sync",
    "traces",
    "update-tracked",
    "validate",
    "verbose",
    "verification",
    "verify-inventories",
    "with-mirrors",
    "with-task",
    "worker",
    "worktree",
    "write",
    "write-baseline",
    "write-inventories",
    "yes",
];
const REPEATED: &[&str] = &[
    "exclude",
    "by",
    "group",
    "tool",
    "allow-root",
    "service",
    "contract",
    "pin",
    "chunk",
    "exclude-set",
    "include",
    "requirements",
    "command",
    "source-session",
];
fn repeated(operation: &str, name: &str) -> bool {
    let operation = operation.strip_prefix("eval/").unwrap_or(operation);
    let action = operation.rsplit('/').next().unwrap_or(operation);
    match name {
        "project" => [
            "memory-eval/field-check",
            "memory-eval/funnel",
            "memory-eval/install-status",
        ]
        .contains(&operation),
        "target" => operation == "memory-rollout-rehearsal",
        "path" => operation == "collect",
        "edition" => {
            operation.starts_with("skill-tools/")
                || [
                    "repo/mirrors",
                    "repo/build-mirrors",
                    "repo/policy-lock",
                    "repo/routes",
                    "repo/check-routes",
                    "repo/stabilization",
                    "repo/artifacts",
                    "repo/assemble",
                ]
                .contains(&operation)
        }
        "skill" => ["add", "install"].contains(&action),
        "agent" => ["add", "install", "remove", "uninstall", "update", "list"].contains(&action),
        _ => REPEATED.contains(&name),
    }
}
fn generic_family(operation: &str) -> String {
    let (group, tail) = operation.split_once('/').unwrap_or(("", operation));
    let stem = tail.split('/').next().unwrap_or(tail);
    match group {
        "attach" => "accelerator-attach".into(),
        "kit" => "kit3".into(),
        "system" => "ai-system".into(),
        "creator" => match stem {
            "ownership" => "infra-ownership",
            "publication" => "publish-staging",
            other => other,
        }
        .replace('_', "-"),
        "skill-tools" => stem.replace('_', "-"),
        "release" => format!("release-{stem}"),
        "eval" => stem.replace('_', "-"),
        "repo" => match stem {
            "mirrors" => "build-mirrors",
            "assets" => "asset-parity",
            "budget" => "context-budget",
            "links" => "check-links",
            "routes" => "check-routes",
            "php-snippets" => "check-php-snippets",
            "stabilization" => "check-stabilization",
            "registry" => "validate-registry",
            "inventory"
            | "inventory-check"
            | "inventory-write"
            | "install"
            | "overlay"
            | "install-plan"
            | "install-apply"
            | "install-preview-images" => "install-accelerator",
            "sync" => "core-sync",
            other => other,
        }
        .replace('_', "-"),
        _ => match stem {
            "install" => "install-accelerator",
            "collect" => "collect-context",
            other => other,
        }
        .replace('_', "-"),
    }
}
fn generic_schema(operation: &str) -> Result<Option<BTreeMap<String, Value>>> {
    if let Some(command) = operation.strip_prefix("eval/") {
        if let Some(schema) = infra_eval::cli_contract::schema(command)? {
            return Ok(Some(schema));
        }
        if command.contains('/')
            && infra_eval::cli_contract::known_family(command.split('/').next().unwrap_or(command))?
        {
            return Err(RuntimeError::invalid(format!(
                "Unknown evaluation command: {command}"
            )));
        }
    }
    let registry: BTreeMap<String, BTreeMap<String, Value>> =
        serde_json::from_str(include_str!("../resources/command-options-v1.json"))
            .map_err(|_| RuntimeError::unavailable("Native command option registry is invalid"))?;
    let family = generic_family(operation);
    let mut schema = registry.get(&family).cloned();
    if schema.is_none()
        && (operation.starts_with("skill-tools/")
            || operation.starts_with("creator/")
            || operation.starts_with("bank/")
            || operation == "release/categorize"
            || ["mcp", "hook", "dispatch", "dispatch-install", "continuity"].contains(&operation))
    {
        schema = Some(BTreeMap::new());
    }
    if let Some(schema) = &mut schema {
        let mut extra = vec!["--source-root", "--tooling-root", "--root", "--help", "-h"];
        if operation == "install"
            || operation.starts_with("repo/install")
            || operation == "repo/overlay"
            || operation == "repo/sync"
        {
            extra.push("--engine");
        }
        if operation.starts_with("creator/") {
            extra.extend([
                "--generator-root",
                "--generator-version",
                "--delivery-root",
                "--approved-profile",
                "--approved-profile-sha256",
                "--profile-path",
            ]);
            match family.as_str() {
                "inspect-hook-policy-profile" => extra.push("--profile"),
                "materialize-hook-policy" => extra.extend(["--profile", "--staging"]),
                "validate-hook-policy" => extra.extend(["--target", "--profile"]),
                "seed-assets" => extra.extend([
                    "--assets",
                    "--staging",
                    "--framework",
                    "--editions",
                    "--engine",
                ]),
                "seed-runtime" => {
                    extra.extend(["--source", "--staging", "--edition", "--tools", "--tool"])
                }
                _ => {}
            }
        }
        if operation == "release/categorize" {
            extra.extend(["--input", "--file", "--text", "--json"]);
        }
        if operation.starts_with("bank/") {
            extra.extend([
                "--db",
                "--project",
                "--mode",
                "--owner",
                "--json",
                "--summary",
                "--provenance",
            ]);
        }
        match operation {
            "mcp" => extra.extend(["--host", "--db", "--project", "--mode", "--owner"]),
            "continuity" => extra.extend(["--host", "--event", "--json", "--source-session"]),
            "dispatch" => extra.push("--store"),
            _ => {}
        }
        if operation.starts_with("skill-tools/") {
            extra.extend(["--platform", "--script-path", "--json", "--auto-refresh"]);
            match family.as_str() {
                "init-skill" => extra.extend(["--path", "--resources", "--examples"]),
                "package-skill" => extra.extend(["--skill-path", "--output-dir"]),
                "eval-set-review" => {
                    extra.extend(["--eval-set", "--skill-path", "--output", "--ui-assets"])
                }
                "generate-review" => extra.push("--ui-assets"),
                "quick-validate" | "utils" => extra.push("--skill-path"),
                "platform-cli" => extra.extend([
                    "--action",
                    "--query",
                    "--name",
                    "--description",
                    "--prompt",
                    "--model",
                    "--timeout",
                ]),
                _ => {}
            }
        }
        if operation.starts_with("eval/") {
            extra.extend([
                "--onnx-worker",
                "--onnx-ort-library",
                "--runtime-binary",
                "--runtime",
                "--ort-library",
                "--worker",
                "--assets",
            ]);
            if [
                "eval/memory-miss-audit",
                "eval/memory-onnx-pilot",
                "eval/memory-rerank-pilot",
            ]
            .contains(&operation)
            {
                extra.push("--eval-clock");
            }
        }
        if operation == "collect" {
            extra.push("--path");
        }
        if operation == "install" {
            extra.extend(["--plan", "--max-text-bytes", "--json"]);
        }
        if operation == "repo/build-ui" {
            extra.extend(["--cargo", "--wasm-bindgen", "--timeout-seconds"]);
        }
        if operation.starts_with("repo/") {
            extra.extend([
                "--editions",
                "--include-generator",
                "--include-oracles",
                "--plan",
                "--receipt",
                "--images",
                "--preview-dir",
                "--max-text-bytes",
            ]);
        }
        for flag in extra {
            schema.entry(flag.into()).or_insert_with(||json!({"flags":[flag],"actions":[if ["--help","-h","--examples","--include-generator","--include-oracles","--json","--auto-refresh","--summary","--provenance"].contains(&flag){Value::String("store_true".into())}else{Value::Null}],"types":[null]}));
        }
    }
    if let Some(schema) = &mut schema {
        schema.retain(|flag, _| !flag.contains("python"));
    }
    Ok(schema)
}
pub fn option_help(operation: &str) -> Result<String> {
    let mut output = format!(
        "Usage: infra-engine {} [options]\n",
        operation.replace('/', " ")
    );
    if let Some(schema) = generic_schema(operation)? {
        let mut seen = BTreeSet::new();
        for spec in schema.values() {
            let flags = spec["flags"]
                .as_array()
                .map(|flags| {
                    flags
                        .iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let label = flags.join(", ");
            if label.is_empty() || !seen.insert(label.clone()) {
                continue;
            }
            let takes = !flag_value(Some(spec), "");
            output.push_str(&format!("  {label}{}\n", if takes { " VALUE" } else { "" }));
        }
    }
    Ok(output)
}
fn flag_value(spec: Option<&Value>, name: &str) -> bool {
    spec.and_then(|spec| spec["actions"].as_array())
        .map(|actions| {
            actions
                .iter()
                .all(|value| value == "store_true" || value == "store_false")
        })
        .unwrap_or_else(|| SWITCHES.contains(&name))
}
fn numeric(spec: Option<&Value>, name: &str) -> bool {
    spec.and_then(|spec| spec["types"].as_array())
        .is_some_and(|types| types.iter().any(|value| value == "int" || value == "float"))
        || [
            "limit",
            "window",
            "flush-after",
            "max-files",
            "jobs",
            "timeout",
            "port",
            "revision",
            "confidence",
            "top",
            "context-budget",
            "impact-depth",
            "min-turns",
            "onnx-timeout",
            "max-steps",
            "max-cases",
            "sample-size",
            "minimum",
            "maximum",
            "concurrency",
            "max-text-bytes",
            "timeout-seconds",
        ]
        .contains(&name)
}
fn expand_short(raw: &[String], schema: Option<&BTreeMap<String, Value>>) -> Result<Vec<String>> {
    let mut output = vec![];
    let mut passthrough = false;
    for value in raw {
        if passthrough {
            output.push(value.clone());
            continue;
        }
        if value == "--" {
            passthrough = true;
            output.push(value.clone());
            continue;
        }
        if value.starts_with("--")
            || !value.starts_with('-')
            || value == "-"
            || value == "-h"
            || value.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
        {
            output.push(value.clone());
            continue;
        }
        let chars = value[1..].char_indices().collect::<Vec<_>>();
        let mut at = 0;
        while at < chars.len() {
            let (index, ch) = chars[at];
            let short = format!("-{ch}");
            let (long, takes) = if let Some(spec) = schema.and_then(|schema| schema.get(&short)) {
                let long = spec["flags"]
                    .as_array()
                    .and_then(|flags| {
                        flags
                            .iter()
                            .filter_map(Value::as_str)
                            .find(|flag| flag.starts_with("--"))
                    })
                    .ok_or_else(|| {
                        RuntimeError::invalid(format!("No long option alias for {short}"))
                    })?;
                (
                    long.to_string(),
                    !flag_value(Some(spec), long.trim_start_matches('-')),
                )
            } else {
                let (long, takes) = match ch {
                    'o' => ("--output", true),
                    's' => ("--skill", true),
                    'a' => ("--agent", true),
                    'g' => ("--global", false),
                    'y' => ("--yes", false),
                    _ => return Err(RuntimeError::invalid(format!("Unrecognized option: -{ch}"))),
                };
                (long.to_string(), takes)
            };
            output.push(long);
            if takes {
                let suffix = &value[1 + index + ch.len_utf8()..];
                if !suffix.is_empty() {
                    output.push(suffix.strip_prefix('=').unwrap_or(suffix).into());
                }
                break;
            }
            at += 1;
        }
    }
    Ok(output)
}
fn insert(map: &mut Map<String, Value>, name: &str, value: Value, repeat: bool) -> Result<()> {
    let name = key(name);
    if repeat {
        let entry = map.entry(name).or_insert_with(|| json!([]));
        entry
            .as_array_mut()
            .ok_or_else(|| RuntimeError::invalid("Conflicting repeated argument"))?
            .push(value);
    } else {
        map.insert(name, value);
    }
    Ok(())
}
fn numeric_value(spec: Option<&Value>, name: &str, text: &str) -> Result<Value> {
    if name == "revision" && text == "auto" {
        return Ok("auto".into());
    }
    let types = spec.and_then(|spec| spec["types"].as_array());
    let integer = types.is_some_and(|types| types.iter().any(|value| value == "int"))
        || !types.is_some_and(|types| types.iter().any(|value| value == "float"))
            && !["timeout", "onnx-timeout", "confidence"].contains(&name);
    if integer {
        let text = text.trim();
        let (negative, digits) = if let Some(value) = text.strip_prefix('-') {
            (true, value)
        } else {
            (false, text.strip_prefix('+').unwrap_or(text))
        };
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(RuntimeError::invalid(format!(
                "--{name} requires an integer"
            )));
        }
        let digits = digits.trim_start_matches('0');
        let digits = if digits.is_empty() { "0" } else { digits };
        let encoded = format!(
            "{}{digits}",
            if negative && digits != "0" { "-" } else { "" }
        );
        return serde_json::from_str(&encoded)
            .map_err(|_| RuntimeError::invalid(format!("--{name} integer is invalid")));
    }
    let value = text
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| RuntimeError::invalid(format!("--{name} requires a finite number")))?;
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .ok_or_else(|| RuntimeError::invalid(format!("--{name} is invalid")))
}
fn optional_value(spec: Option<&Value>) -> bool {
    spec.is_some_and(|spec| spec["nargs"] == "?")
}
fn value_token(value: &str) -> bool {
    !value.starts_with("--") && value != "-h"
}
struct ArgumentSegments<'a> {
    options: &'a [String],
    remainder: Option<&'a [String]>,
}
fn argument_segments<'a>(
    raw: &'a [String],
    schema: Option<&BTreeMap<String, Value>>,
) -> Result<ArgumentSegments<'a>> {
    let remainder = schema.is_some_and(|schema| {
        schema
            .values()
            .any(|spec| spec["positional"] == true && spec["nargs"] == "REMAINDER")
    });
    if !remainder {
        return Ok(ArgumentSegments {
            options: raw,
            remainder: None,
        });
    }
    let mut at = 0;
    while at < raw.len() {
        let item = &raw[at];
        if item == "--" {
            return Ok(ArgumentSegments {
                options: &raw[..at],
                remainder: Some(&raw[at + 1..]),
            });
        }
        if item == "-" || !item.starts_with('-') {
            return Ok(ArgumentSegments {
                options: &raw[..at],
                remainder: Some(&raw[at..]),
            });
        }
        let (flag, inline) = item
            .split_once('=')
            .map(|(flag, _)| (flag, true))
            .unwrap_or((item.as_str(), false));
        let spec = schema
            .and_then(|schema| schema.get(flag))
            .ok_or_else(|| RuntimeError::invalid(format!("Unrecognized option: {flag}")))?;
        at += 1;
        if !inline
            && !flag_value(Some(spec), flag.trim_start_matches('-'))
            && (!optional_value(Some(spec)) || raw.get(at).is_some_and(|value| value_token(value)))
        {
            at += 1;
        }
    }
    Ok(ArgumentSegments {
        options: raw,
        remainder: None,
    })
}
pub fn help_requested(raw: &[String], operation: &str) -> Result<bool> {
    let schema = generic_schema(operation)?;
    let segments = argument_segments(raw, schema.as_ref())?;
    Ok(segments
        .options
        .iter()
        .take_while(|argument| argument.as_str() != "--")
        .any(|argument| argument == "--help" || argument == "-h"))
}
fn checked_choice(spec: Option<&Value>, name: &str, value: Value) -> Result<Value> {
    if let Some(choices) = spec.and_then(|spec| spec["choices"].as_array())
        && !choices.contains(&value)
    {
        return Err(RuntimeError::invalid(format!("Invalid value for --{name}")));
    }
    Ok(value)
}
fn apply_scoped_contract(
    operation: &str,
    schema: Option<&BTreeMap<String, Value>>,
    positionals: &[String],
    map: &mut Map<String, Value>,
) -> Result<()> {
    let Some(schema) = schema.filter(|schema| {
        schema
            .get("$contract")
            .is_some_and(|contract| contract["profile"] == "eval-cli-v2")
    }) else {
        return Ok(());
    };
    if map.get("help") == Some(&Value::Bool(true)) {
        return Ok(());
    }
    let positional_specs = schema
        .values()
        .filter(|spec| spec["positional"] == true && spec["nargs"] != "REMAINDER")
        .collect::<Vec<_>>();
    let has_remainder = schema
        .values()
        .any(|spec| spec["positional"] == true && spec["nargs"] == "REMAINDER");
    if !has_remainder && positionals.len() != positional_specs.len() {
        return Err(RuntimeError::invalid(format!(
            "{operation} requires {} positional argument(s)",
            positional_specs.len()
        )));
    }
    for (spec, value) in positional_specs.iter().zip(positionals) {
        let key = infra_eval::cli_contract::key(spec)
            .ok_or_else(|| RuntimeError::unavailable("Invalid positional contract"))?;
        map.insert(key, value.clone().into());
    }
    let mut seen = BTreeSet::new();
    for spec in schema.values() {
        let Some(key) = infra_eval::cli_contract::key(spec) else {
            continue;
        };
        if seen.insert(key.clone())
            && !map.contains_key(&key)
            && let Some(value) = infra_eval::cli_contract::default_value(spec)?
        {
            map.insert(key, value);
        }
    }
    infra_eval::cli_contract::validate(
        operation.strip_prefix("eval/").unwrap_or(operation),
        &Value::Object(map.clone()),
    )
}
pub fn parse(raw: &[String], operation: &str) -> Result<Arguments> {
    let schema = generic_schema(operation)?;
    let original = raw;
    let segments = argument_segments(raw, schema.as_ref())?;
    let expanded = expand_short(segments.options, schema.as_ref())?;
    let raw = &expanded;
    let mut map = Map::new();
    let mut positionals = vec![];
    let mut i = 0;
    let mut ended = false;
    while i < raw.len() {
        let item = &raw[i];
        if ended || item == "-" || !item.starts_with('-') {
            positionals.push(item.clone());
            i += 1;
            continue;
        }
        if item == "--" {
            ended = true;
            i += 1;
            continue;
        }
        if item == "-h" || item == "--help" {
            insert(&mut map, "help", true.into(), false)?;
            i += 1;
            continue;
        }
        if !item.starts_with("--") {
            return Err(RuntimeError::invalid(format!(
                "Unrecognized option: {item}"
            )));
        }
        let flag = &item[2..];
        let (name, inline) = flag
            .split_once('=')
            .map(|(a, b)| (a, Some(b)))
            .unwrap_or((flag, None));
        if name.is_empty() {
            return Err(RuntimeError::invalid("Empty option name"));
        }
        let full = format!("--{name}");
        let spec = schema.as_ref().and_then(|schema| schema.get(&full));
        if schema.is_some() && spec.is_none() {
            return Err(RuntimeError::invalid(format!(
                "Unrecognized option: {full}"
            )));
        }
        if flag_value(spec, name) {
            if inline.is_some() {
                return Err(RuntimeError::invalid(format!(
                    "Option --{name} does not take a value"
                )));
            }
            insert(
                &mut map,
                name,
                Value::Bool(!spec.is_some_and(|spec| {
                    spec["actions"]
                        .as_array()
                        .is_some_and(|values| values.iter().all(|value| value == "store_false"))
                })),
                false,
            )?;
            i += 1;
            continue;
        }
        let value = if let Some(value) = inline {
            Some(value.to_string())
        } else if optional_value(spec) {
            if let Some(next) = raw.get(i + 1).filter(|value| value_token(value)) {
                i += 1;
                Some(next.clone())
            } else {
                None
            }
        } else {
            let next = raw
                .get(i + 1)
                .filter(|value| value_token(value))
                .ok_or_else(|| {
                    RuntimeError::invalid(format!("Option --{name} requires a value"))
                })?;
            i += 1;
            Some(next.clone())
        };
        let converted = if let Some(value) = value {
            if numeric(spec, name) && name != "seed" {
                numeric_value(spec, name, &value)?
            } else {
                Value::String(value)
            }
        } else {
            spec.and_then(|spec| spec.get("const"))
                .cloned()
                .unwrap_or(Value::Null)
        };
        let converted = checked_choice(spec, name, converted)?;
        let repeated = if name == "project"
            || name == "target"
            || name == "skill"
            || name == "agent"
            || name == "path"
        {
            repeated(operation, name)
        } else {
            spec.and_then(|spec| spec["actions"].as_array())
                .is_some_and(|actions| actions.iter().all(|value| value == "append"))
                || repeated(operation, name)
        };
        insert(&mut map, name, converted, repeated)?;
        i += 1;
    }
    if let Some(remainder) = segments.remainder {
        positionals.extend_from_slice(remainder);
        map.insert("_remainder".into(), json!(remainder));
    }
    apply_scoped_contract(operation, schema.as_ref(), &positionals, &mut map)?;
    map.insert("_positionals".into(), json!(positionals));
    map.insert("_arguments".into(), json!(original));
    Ok(Arguments {
        operation: operation.into(),
        value: Value::Object(map),
        raw: original.to_vec(),
    })
}
pub const MEMORY_COMMANDS: &[&str] = &[
    "cursor-rule",
    "cursor-rule-retire",
    "context-save",
    "context-load",
    "index",
    "search",
    "context",
    "retrieve",
    "hook-context",
    "refresh",
    "turn",
    "rebind",
    "record",
    "start",
    "update",
    "get",
    "clear",
    "complete",
    "msg-send",
    "msg-read",
    "msg-dispatch",
    "capsule",
    "brain-create",
    "record-result",
    "native-prompt",
    "native-stop",
    "native-result",
    "prepare-result",
    "replay-result",
    "brain-update",
    "brain-get",
    "source-fingerprints",
    "memory-review",
    "memory-review-controller",
    "retrieval-report",
    "health",
    "links",
    "status",
    "validate",
    "brain-validate",
    "parity",
    "compact",
    "consolidate",
    "bank-audit",
    "bank-reverify",
    "bank-retire",
    "bank-quarantine",
    "reindex-bank",
    "promote-propose",
    "promote-review",
    "promote-apply",
    "promote-auto",
    "export",
];

#[derive(serde::Deserialize)]
struct MemorySpec {
    name: String,
    options: Vec<MemoryOption>,
}
#[derive(Clone, serde::Deserialize)]
struct MemoryOption {
    flags: Vec<String>,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    default: Value,
    choices: Option<Vec<Value>>,
    action: Option<String>,
    dest: Option<String>,
}
fn memory_schema() -> Result<BTreeMap<String, MemorySpec>> {
    serde_json::from_str(include_str!("../resources/memory-cli-v1.json"))
        .map_err(|_| RuntimeError::unavailable("Native memory command registry is invalid"))
}
fn memory_key(spec: &MemoryOption) -> String {
    spec.dest
        .clone()
        .unwrap_or_else(|| spec.flags[0].trim_start_matches('-').replace('-', "_"))
}
fn memory_value(spec: &MemoryOption, value: &str) -> Result<Value> {
    let flag = &spec.flags[0];
    if let Some(choices) = &spec.choices
        && !choices.iter().any(|choice| choice.as_str() == Some(value))
    {
        return Err(RuntimeError::invalid(format!("Invalid value for {flag}")));
    }
    if [
        "--limit",
        "--since",
        "--window",
        "--flush-after",
        "--max-files",
    ]
    .contains(&flag.as_str())
    {
        return value
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| RuntimeError::invalid(format!("Integer required for {flag}")));
    }
    if flag == "--revision" {
        if value == "auto" {
            return Ok("auto".into());
        }
        let revision = value
            .parse::<u64>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| RuntimeError::invalid("Revision must be a positive integer or auto"))?;
        return Ok(revision.into());
    }
    if flag == "--confidence" {
        let value = value
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| RuntimeError::invalid("Confidence must be a finite number"))?;
        return serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| RuntimeError::invalid("Invalid confidence"));
    }
    Ok(value.into())
}
fn parse_memory_options(
    spec: &MemorySpec,
    raw: &[String],
    map: &mut Map<String, Value>,
) -> Result<()> {
    let mut used = BTreeSet::new();
    let mut positionals = vec![];
    let mut i = 0;
    let mut ended = false;
    while i < raw.len() {
        let item = &raw[i];
        if item == "--" && !ended {
            ended = true;
            i += 1;
            continue;
        }
        if !ended && (item == "-h" || item == "--help") {
            map.insert("help".into(), true.into());
            return Ok(());
        }
        if !ended && item.starts_with('-') && item != "-" {
            let (name, inline) = item
                .split_once('=')
                .map(|(a, b)| (a, Some(b)))
                .unwrap_or((item.as_str(), None));
            let option = spec
                .options
                .iter()
                .find(|option| option.flags.iter().any(|flag| flag == name))
                .ok_or_else(|| RuntimeError::invalid(format!("Unrecognized argument: {name}")))?;
            let destination = memory_key(option);
            used.insert(destination.clone());
            if option.action.as_deref() == Some("store_true") {
                if inline.is_some() {
                    return Err(RuntimeError::invalid(format!(
                        "Argument {name} does not take a value"
                    )));
                }
                map.insert(destination, true.into());
                i += 1;
                continue;
            }
            let value = if let Some(value) = inline {
                value.to_string()
            } else {
                let next = raw.get(i + 1).ok_or_else(|| {
                    RuntimeError::invalid(format!("Argument {name} requires a value"))
                })?;
                if next.starts_with("--") {
                    return Err(RuntimeError::invalid(format!(
                        "Argument {name} requires a value"
                    )));
                }
                i += 1;
                next.clone()
            };
            let value = memory_value(option, &value)?;
            if option.action.as_deref() == Some("append") {
                map.entry(destination)
                    .or_insert_with(|| json!([]))
                    .as_array_mut()
                    .ok_or_else(|| RuntimeError::invalid("Invalid repeated argument"))?
                    .push(value);
            } else {
                map.insert(destination, value);
            }
            i += 1;
            continue;
        }
        positionals.push(item.clone());
        i += 1;
    }
    let required_positionals = spec
        .options
        .iter()
        .filter(|option| !option.flags[0].starts_with('-'))
        .collect::<Vec<_>>();
    if positionals.len() != required_positionals.len() {
        return Err(RuntimeError::invalid(format!(
            "{} requires {} positional argument(s)",
            spec.name,
            required_positionals.len()
        )));
    }
    for (option, value) in required_positionals.iter().zip(&positionals) {
        let key = memory_key(option);
        used.insert(key.clone());
        map.insert(key, memory_value(option, value)?);
    }
    for option in &spec.options {
        let destination = memory_key(option);
        if option.required && !used.contains(&destination) {
            return Err(RuntimeError::invalid(format!(
                "Required argument missing: {}",
                option.flags[0]
            )));
        }
        if !map.contains_key(&destination) {
            let default = if option.action.as_deref() == Some("store_true") {
                Value::Bool(false)
            } else {
                option.default.clone()
            };
            map.insert(destination, default);
        }
    }
    map.insert("_positionals".into(), json!(positionals));
    Ok(())
}
pub fn memory(raw: &[String]) -> Result<Arguments> {
    let schema = memory_schema()?;
    let global = schema
        .get("global")
        .ok_or_else(|| RuntimeError::unavailable("Missing global CLI registry"))?;
    let mut at = 0;
    while at < raw.len() {
        if raw[at] == "--help" || raw[at] == "-h" {
            return Ok(Arguments {
                operation: "help".into(),
                value: json!({"help":true}),
                raw: raw.to_vec(),
            });
        }
        if !raw[at].starts_with('-') {
            break;
        }
        let (flag, inline) = raw[at]
            .split_once('=')
            .map(|(flag, _)| (flag, true))
            .unwrap_or((raw[at].as_str(), false));
        if !global
            .options
            .iter()
            .any(|option| option.flags.iter().any(|name| name == flag))
        {
            return Err(RuntimeError::invalid(format!(
                "Unknown global memory argument: {flag}"
            )));
        }
        at += if inline { 1 } else { 2 };
    }
    let operation = raw
        .get(at)
        .ok_or_else(|| RuntimeError::invalid("A memory command is required"))?;
    let spec = schema
        .get(operation)
        .ok_or_else(|| RuntimeError::invalid(format!("Unknown memory command: {operation}")))?;
    let mut map = Map::new();
    parse_memory_options(global, &raw[..at], &mut map)?;
    parse_memory_options(spec, &raw[at + 1..], &mut map)?;
    if operation == "native-result"
        && map.get("skip") == Some(&Value::Bool(true))
        && map.get("input").is_some_and(|value| !value.is_null())
    {
        return Err(RuntimeError::invalid(
            "--input and --skip are mutually exclusive",
        ));
    }
    if operation == "msg-read" {
        map.insert(
            "since_seq".into(),
            map.get("since").cloned().unwrap_or(json!(0)),
        );
    }
    map.insert("_arguments".into(), json!(raw));
    Ok(Arguments {
        operation: if operation == "brain-validate" {
            "validate".into()
        } else {
            operation.clone()
        },
        value: Value::Object(map),
        raw: raw.to_vec(),
    })
}
