//! CLI adaptation for repository services. Domain work stays in infra-tooling.
use crate::output::Output;
use infra_adapters::TailTrim;
use infra_core::{Result, RuntimeError};
use serde_json::{Value, json};
use std::path::Path;
fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key].as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn text(value: &Value) -> String {
    value.as_str().map(String::from).unwrap_or_else(|| {
        if value.is_null() {
            String::new()
        } else {
            value.to_string()
        }
    })
}
pub fn strings(value: &Value) -> Result<Vec<String>> {
    match value {
        Value::Null => Ok(vec![]),
        Value::String(value) => Ok(value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(String::from)
            .collect()),
        Value::Array(values) => {
            let mut result = vec![];
            for value in values {
                result.extend(strings(value)?);
            }
            Ok(result)
        }
        _ => Err(RuntimeError::invalid(
            "Expected a string or list of strings",
        )),
    }
}
pub fn normalise(request: &mut Value) -> Result<()> {
    for (from, to) in [("group", "groups"), ("tool", "tools"), ("path", "paths")] {
        if request[to].is_null() && !request[from].is_null() {
            request[to] = request[from].clone();
        }
    }
    for field in ["groups", "editions"] {
        if !request[field].is_null() {
            request[field] = json!(strings(&request[field])?);
        }
    }
    if request["tools"].is_null() {
        request["tools"] = json!(["claude", "cursor", "codex"]);
    } else {
        request["tools"] = json!(strings(&request["tools"])?);
    }
    Ok(())
}
fn failed(value: &Value) -> bool {
    infra_tooling::report_exit_code(value) != 0
}
pub fn reply(operation: &str, request: &Value, value: Value) -> Result<Output> {
    if value.get("_cli").is_some() || value["stdio_inherited"] == true {
        return Output::envelope(value);
    }
    let code = infra_tooling::report_exit_code(&value);
    if request["json"] == true {
        let value = if operation == "policy-lock" {
            let reports = value["reports"]
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![value.clone()]);
            let mut report = serde_json::Map::new();
            for item in reports {
                let edition = item["edition"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::unavailable("Policy report has no edition"))?;
                let mut data = json!({"policy_digest":item["policy_digest"],"files":item["files"]});
                if item["written"] == true {
                    data["written"] = true.into();
                } else {
                    data["findings"] = item["findings"].clone();
                    if request["write"] == true {
                        data["written"] = false.into();
                    }
                }
                report.insert(edition.into(), data);
            }
            Value::Object(report)
        } else {
            value
        };
        let mut output = Output::json(&value)?;
        output.code = code;
        return Ok(output);
    }
    let mut stdout = String::new();
    let mut stderr = String::new();
    match operation {
        "budget" | "context-budget" => {
            if request["headroom"] == true {
                stdout.push_str(&format!(
                    "{:>10}  {:>9}  {:>9}  category\n",
                    "remaining", "measured", "ceiling"
                ));
                for row in list(&value, "headroom") {
                    stdout.push_str(&format!(
                        "{:>10}  {:>9}  {:>9}  {}.{}\n",
                        text(&row["remaining"]),
                        text(&row["measured"]),
                        text(&row["ceiling"]),
                        text(&row["edition"]),
                        text(&row["category"])
                    ));
                }
                return Ok(Output::text(stdout));
            }
            let categories = [
                "agents_md_bytes",
                "descriptor_bytes",
                "command_bytes",
                "agent_bytes",
                "frontmatter_bytes",
                "body_bytes",
            ];
            let mut editions = [
                "Laravel",
                "Symfony",
                "PHP Core",
                "WordPress",
                "Infrastructure-Creator",
            ]
            .iter()
            .filter(|edition| value["editions"].get(**edition).is_some())
            .map(|edition| edition.to_string())
            .collect::<Vec<_>>();
            if let Some(values) = value["editions"].as_object() {
                for edition in values.keys() {
                    if !editions.contains(edition) {
                        editions.push(edition.clone());
                    }
                }
            }
            if request["check"] == true {
                for edition in &editions {
                    for category in categories {
                        let measured = value["editions"][edition][category].as_u64().unwrap_or(0);
                        let ceiling = value["ceilings"][edition][category].as_u64();
                        match ceiling {
                            Some(ceiling) => stdout.push_str(&format!(
                                "{}  {edition}.{category}: {measured} {} ceiling {ceiling}\n",
                                if measured > ceiling { "FAIL" } else { "ok  " },
                                if measured > ceiling { ">" } else { "<=" }
                            )),
                            None => stdout.push_str(&format!(
                                "FAIL  {edition}.{category}: no ceiling in token_budget.json\n"
                            )),
                        }
                    }
                }
                if code == 0 {
                    stdout.push_str("\nAll context budgets within ceilings.\n");
                } else {
                    stdout.push_str(&format!("\n{} budget violation(s). Either trim the context back\nunder the ceiling, or — for a justified permanent increase —\nraise the ceiling in scripts/token_budget.json in the same change.\n",list(&value,"findings").len()));
                }
            } else {
                let headers = [
                    "Edition",
                    "Skills",
                    "AGENTS.md",
                    "skill descriptors",
                    "command listing",
                    "agent listing",
                    "Startup total",
                    "Bodies (on invocation)",
                ];
                let mut rows = vec![headers.iter().map(|s| s.to_string()).collect::<Vec<_>>()];
                let format_bytes = |measured: &Value, tokens: &Value, category: &str| -> String {
                    format!(
                        "{} B (~{} t)",
                        text(&measured[category]),
                        text(&tokens[category])
                    )
                };
                for edition in &editions {
                    let measured = &value["editions"][edition];
                    let tokens = &value["tokens"][edition];
                    let mut row = vec![edition.clone(), text(&measured["skills"])];
                    for category in &categories[..4] {
                        row.push(format_bytes(measured, tokens, category));
                    }
                    row.push(format!("~{} t", text(&value["startup_tokens"][edition])));
                    row.push(format_bytes(measured, tokens, "body_bytes"));
                    rows.push(row);
                }
                let mut total_tokens = json!({});
                for category in categories {
                    let bytes = value["totals"][category].as_f64().unwrap_or(0.0);
                    let calibration = value["bytes_per_token"][category]
                        .as_f64()
                        .filter(|value| *value > 0.0)
                        .ok_or_else(|| {
                            RuntimeError::unavailable("Native budget calibration unavailable")
                        })?;
                    total_tokens[category] = json!((bytes / calibration).round_ties_even() as u64);
                }
                let mut row = vec![
                    "ALL EDITIONS (monorepo checkout)".into(),
                    text(&value["totals"]["skills"]),
                ];
                for category in &categories[..4] {
                    row.push(format_bytes(&value["totals"], &total_tokens, category));
                }
                row.push(format!("~{} t", text(&value["total_startup_tokens"])));
                row.push(format_bytes(&value["totals"], &total_tokens, "body_bytes"));
                rows.push(row);
                let widths = (0..headers.len())
                    .map(|index| {
                        rows.iter()
                            .map(|row| row[index].chars().count())
                            .max()
                            .unwrap_or(0)
                    })
                    .collect::<Vec<_>>();
                stdout.push_str("Context budget (bytes; token estimates calibrated per class, see BYTES_PER_TOKEN)\n\n");
                for (index, row) in rows.iter().enumerate() {
                    let line = row
                        .iter()
                        .enumerate()
                        .map(|(index, value)| format!("{value:width$}", width = widths[index]))
                        .collect::<Vec<_>>()
                        .join("  ");
                    stdout.push_str(line.tail());
                    stdout.push('\n');
                    if index == 0 {
                        stdout.push_str(
                            &widths
                                .iter()
                                .map(|width| "-".repeat(*width))
                                .collect::<Vec<_>>()
                                .join("  "),
                        );
                        stdout.push('\n');
                    }
                }
                stdout.push_str("\nStartup total = AGENTS.md plus every listing the tool shows the model:\nskill descriptors, commands and agents. All of it is paid on every\nsession of that edition, whether or not any of it is used - which is\nwhy a rarely-invoked skill or agent is expensive rather than cheap.\nBodies are paid per invocation instead, an order of magnitude larger,\nwhich is why they are budgeted separately rather than ignored.\n\nfrontmatter_bytes is gated too but not shown here: it is a superset of\nthe descriptors including orchestration keys the model is never shown.\n\nThe command and agent listings are measured on the .claude tree, the\nrichest of the three: Cursor's differ slightly, and Codex has neither\ndirectory at all, so a Codex session pays only the first two columns.\n\nThe last row is what a monorepo checkout exposes, where every\nedition's listings are visible at once. A consuming project\ninstalls ONE edition and pays only that edition's row.\n");
            }
        }
        "core-sync" => {
            for change in list(&value, "changes") {
                stdout.push_str(&format!(
                    "{}\t{}\t{}\n",
                    text(&change["action"]).to_uppercase(),
                    text(&change["edition"]),
                    text(&change["path"])
                ));
            }
            let count = list(&value, "changes").len();
            if request["write"] == true {
                stdout.push_str(&format!("core_sync: {count} change(s) written\n"));
            } else if code == 0 {
                stdout.push_str("core_sync: every copy matches PHP Core\n");
            } else {
                stderr.push_str(&format!("core_sync: {count} copy difference(s); run infra-engine repo core-sync --write\n"));
            }
        }
        "mirrors" => {
            let reports = value["reports"]
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![value.clone()]);
            let names = reports
                .iter()
                .filter_map(|report| report["edition"].as_str())
                .collect::<Vec<_>>();
            let mut count = 0;
            let mut problems = vec![];
            for report in &reports {
                for path in list(report, "changed") {
                    count += 1;
                    if request["write"] == true {
                        stdout.push_str(&format!("wrote {}\n", text(path)));
                    }
                }
                for finding in list(report, "findings") {
                    problems.push(text(finding));
                }
            }
            if code != 0 {
                stderr.push_str(&format!("Mirror drift ({} finding(s)):\n", problems.len()));
                for problem in problems {
                    stderr.push_str(&format!("  {problem}\n"));
                }
            } else if request["write"] == true {
                stdout.push_str(&format!(
                    "Mirrors up to date for: {} ({count} file(s) written)\n",
                    names.join(", ")
                ));
            } else {
                stdout.push_str(&format!("Mirror check passed for: {}\n", names.join(", ")));
            }
        }
        "policy-lock" => {
            let reports = value["reports"]
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![value.clone()]);
            for report in reports {
                let edition = text(&report["edition"]);
                let findings = list(&report, "findings");
                if !findings.is_empty() {
                    stdout.push_str(&format!(
                        "FAIL\t{edition}\t{} finding(s):\n",
                        findings.len()
                    ));
                    for finding in findings {
                        stdout.push_str(&format!("  {}\n", text(finding)));
                    }
                } else {
                    let digest = text(&report["policy_digest"]);
                    let digest = digest.chars().take(8).collect::<String>();
                    stdout.push_str(&format!(
                        "{}\t{}\t{} file(s)\tpolicy {digest}\n",
                        if report["written"] == true {
                            "WROTE"
                        } else {
                            "ok"
                        },
                        if report["written"] == true {
                            format!("{edition}/.accelerator-policy-lock.json")
                        } else {
                            edition
                        },
                        text(&report["files"])
                    ));
                }
            }
        }
        "links" => {
            let findings = list(&value, "findings");
            if findings.is_empty() {
                stdout.push_str("All relative markdown links resolve. No broken links found.\n");
            } else {
                stdout.push_str(&format!(
                    "Found {} broken relative link(s):\n\n",
                    findings.len()
                ));
                for row in findings {
                    stdout.push_str(&format!(
                        "{}:{}: {} -> {}\n",
                        text(&row["path"]),
                        text(&row["line"]),
                        text(&row["target"]),
                        text(&row["resolved"])
                    ));
                }
            }
        }
        "routes" => {
            let findings = list(&value, "findings");
            let mut groups = list(&value, "editions")
                .iter()
                .map(text)
                .collect::<Vec<_>>();
            if findings.iter().any(|row| row["edition"] == "(allowlist)") {
                groups.push("(allowlist)".into());
            }
            for group in groups {
                let allowed = list(&value, "allowlisted_findings")
                    .iter()
                    .filter(|row| row["edition"] == group)
                    .count();
                let suffix = if allowed > 0 {
                    format!(" ({allowed} allowlisted)")
                } else {
                    String::new()
                };
                let mut rows = findings
                    .iter()
                    .filter(|row| row["edition"] == group)
                    .collect::<Vec<_>>();
                rows.sort_by_key(|row| {
                    (
                        row["severity"] != "error",
                        text(&row["path"]),
                        row["line"].as_u64().unwrap_or(0),
                    )
                });
                stdout.push_str(&format!(
                    "{group}:{}{}\n",
                    if rows.is_empty() { " ok" } else { "" },
                    suffix
                ));
                for row in rows {
                    let location = format!(
                        "{}{}",
                        text(&row["path"]),
                        row["line"]
                            .as_u64()
                            .map(|line| format!(":{line}"))
                            .unwrap_or_default()
                    );
                    stdout.push_str(&format!(
                        "  {:<7} {location}  [{}] {}\n",
                        text(&row["severity"]),
                        text(&row["check"]),
                        text(&row["message"])
                    ));
                }
            }
            stdout.push_str(&format!(
                "check_routes: {} error(s), {} warning(s), {} allowlisted across {} edition(s)\n",
                text(&value["errors"]),
                text(&value["warnings"]),
                text(&value["allowlisted"]),
                list(&value, "editions").len()
            ));
        }
        "php-snippets" => {
            if value["status"] == "skipped" {
                stdout.push_str("SKIPPED\tphp not available; no snippet was checked\n");
            } else {
                for row in list(&value, "findings") {
                    stderr.push_str(&format!(
                        "INVALID\t{} block {}: {}\n",
                        text(&row["path"]),
                        text(&row["block"]),
                        text(&row["problem"])
                    ));
                }
                let counts = value["counts"].as_object();
                let count = counts
                    .map(|values| {
                        values
                            .iter()
                            .filter(|(key, _)| key.as_str() != "declared fragment")
                            .filter_map(|(_, value)| value.as_u64())
                            .sum::<u64>()
                    })
                    .unwrap_or(0);
                let details = counts
                    .map(|values| {
                        values
                            .iter()
                            .filter(|(key, _)| key.as_str() != "declared fragment")
                            .map(|(key, value)| format!("{} {key}", text(value)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                stdout.push_str(&format!("CHECKED\t{count} snippet(s) ({details})\tSKIPPED\t{} declared fragment(s)\tINVALID\t{}\n",value["counts"]["declared fragment"].as_u64().unwrap_or(0),list(&value,"findings").len()));
            }
        }
        "stabilization" => {
            let findings = list(&value, "findings");
            if !findings.is_empty() {
                stdout.push_str(&format!(
                    "Stabilization rule findings ({}):\n",
                    findings.len()
                ));
                for row in findings {
                    stdout.push_str(&format!(
                        "  {}{}: {}\n",
                        text(&row["path"]),
                        row["rule"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(|s| format!(" [{s}]"))
                            .unwrap_or_default(),
                        text(&row["message"])
                    ));
                }
            } else {
                let summary = value["checked"]
                    .as_object()
                    .map(|values| {
                        values
                            .iter()
                            .map(|(name, count)| format!("{name}: {}", text(count)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_else(|| format!("{} rules", text(&value["rules"])));
                stdout.push_str(&format!("Stabilization rules well-formed ({summary}).\n"));
                for edition in list(&value, "skipped") {
                    stdout.push_str(&format!(
                        "  skipped {}: no rule blocks (prose-genre document)\n",
                        text(edition)
                    ));
                }
            }
        }
        "inventory-check" | "inventory-write" => {
            let reports = value["inventories"]
                .as_array()
                .or_else(|| value["reports"].as_array())
                .cloned()
                .unwrap_or_else(|| vec![value.clone()]);
            for report in reports {
                stdout.push_str(&format!(
                    "{}\t{}\n",
                    if operation == "inventory-write" {
                        "WROTE"
                    } else if report["passed"] == false {
                        "INVALID"
                    } else {
                        "VERIFIED"
                    },
                    report["written"]
                        .as_str()
                        .unwrap_or_else(|| report["edition"].as_str().unwrap_or(""))
                ));
            }
        }
        "attach-detect" if value["edition"].is_null() => {
            let mut output = Output::json(&value)?;
            output.code = 1;
            return Ok(output);
        }
        _ => {
            let mut output = Output::json(&value)?;
            output.code = code;
            return Ok(output);
        }
    }
    Ok(Output {
        code,
        stdout: stdout.into_bytes(),
        stderr: stderr.into_bytes(),
        ..Default::default()
    })
}
pub fn execute(operation: &str, mut request: Value) -> Result<Output> {
    normalise(&mut request)?;
    let normalised = operation
        .strip_suffix(".py")
        .unwrap_or(operation)
        .replace('_', "-");
    let operation = match normalised.as_str() {
        "build-mirrors" => "mirrors",
        "core-sync" => "core-sync",
        "asset-parity" => "asset-parity",
        "context-budget" => "context-budget",
        "check-links" => "links",
        "check-routes" => "routes",
        "check-php-snippets" => "php-snippets",
        "check-stabilization" => "stabilization",
        other => other,
    }
    .to_string();
    if ["mirrors", "policy-lock"].contains(&operation.as_str())
        || ["inventory", "inventory-check", "inventory-write"].contains(&operation.as_str())
            && request["edition"].is_array()
    {
        let root = request["root"].as_str().unwrap_or(".");
        let editions = if request["edition"].is_null() {
            infra_tooling::common::EDITIONS
                .iter()
                .filter(|edition| Path::new(root).join(edition).is_dir())
                .map(|edition| edition.to_string())
                .collect::<Vec<_>>()
        } else {
            strings(&request["edition"])?
        };
        if editions.is_empty() {
            return Err(RuntimeError::invalid("No editions selected"));
        }
        let mut reports = vec![];
        for edition in editions {
            let mut local = request.clone();
            local["edition"] = infra_tooling::common::edition(&edition)?.into();
            reports.push(infra_tooling::execute(&operation, &local)?);
        }
        let passed = !reports.iter().any(failed);
        return reply(
            &operation,
            &request,
            json!({"passed":passed,"reports":reports}),
        );
    }
    if ["routes", "stabilization", "artifacts", "assemble"].contains(&operation.as_str())
        && !request["edition"].is_null()
    {
        request["editions"] = json!(strings(&request["edition"])?);
        request
            .as_object_mut()
            .ok_or_else(|| RuntimeError::invalid("Tooling request must be an object"))?
            .remove("edition");
    }
    if request["edition"].is_array() {
        let editions = strings(&request["edition"])?;
        if editions.len() != 1 {
            return Err(RuntimeError::invalid(
                "This operation requires one --edition",
            ));
        }
        request["edition"] = editions[0].clone().into();
    }
    if ["install-apply", "install-preview-images"].contains(&operation.as_str())
        && let Some(path) = request["plan"].as_str()
    {
        use std::io::Read;
        let path = std::fs::canonicalize(path)?;
        let meta = std::fs::symlink_metadata(&path)?;
        if !meta.is_file() || infra_core::fs::linked(&meta) || meta.len() > 16 * 1024 * 1024 {
            return Err(RuntimeError::policy(
                "Reviewed plan must be a bounded regular JSON file",
            ));
        }
        let mut raw = vec![];
        std::fs::File::open(path)?
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut raw)?;
        if raw.len() > 16 * 1024 * 1024 {
            return Err(RuntimeError::invalid("Reviewed plan exceeds 16 MiB"));
        }
        let text = std::str::from_utf8(&raw)
            .map_err(|_| RuntimeError::invalid("Reviewed plan is not UTF-8"))?;
        request["plan"] = infra_core::domain::json::parse(text)?;
    }
    let value = infra_tooling::execute(&operation, &request)?;
    reply(&operation, &request, value)
}
