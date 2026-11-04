use crate::{common::*, ownership};
use infra_core::{Result, RuntimeError, fs};
use serde_json::{Value, json as object};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
const PHASES: &[&str] = &[
    "understanding",
    "planning",
    "implementation",
    "verification",
    "finalization",
];
fn exact(value: &Value, names: &[&str]) -> bool {
    keys(value) == names.iter().map(|value| value.to_string()).collect()
}
fn add(errors: &mut Vec<String>, message: impl Into<String>) {
    errors.push(message.into());
}
pub fn validate_plan_graph(plan: &Value) -> Vec<String> {
    let mut errors = vec![];
    if !["1.2", "1.3", "1.4", "1.5", "1.6"].contains(&text(&plan["schema_version"])) {
        return vec![
            "plan: flow validation requires schema_version 1.2 or 1.3 or 1.4 or 1.5 or 1.6".into(),
        ];
    }
    let graph = &plan["flow_contracts"];
    if !graph.is_object() {
        return vec!["plan: a typed plan requires canonical flow_contracts".into()];
    }
    let Some(skills) = plan["skills"].as_array() else {
        return vec!["plan: a typed plan requires skills[]".into()];
    };
    let names = skills
        .iter()
        .filter_map(|skill| skill["name"].as_str())
        .collect::<BTreeSet<_>>();
    let mut adjacency = BTreeMap::<String, BTreeSet<String>>::new();
    let mut observed = BTreeMap::<String, BTreeSet<String>>::new();
    for skill in skills {
        let Some(name) = skill["name"].as_str() else {
            continue;
        };
        let Some(siblings) = skill["nearest_siblings"].as_array() else {
            add(
                &mut errors,
                format!("routing: {name}.nearest_siblings must be a list"),
            );
            continue;
        };
        let mut adjacent = BTreeSet::new();
        for sibling in siblings {
            let adjacent_name = text(&sibling["name"]);
            if adjacent_name == name || !names.contains(adjacent_name) {
                add(
                    &mut errors,
                    format!("routing: {name} has invalid adjacent skill {adjacent_name:?}"),
                );
            } else {
                adjacent.insert(adjacent_name.to_string());
            }
        }
        adjacency.insert(name.to_string(), adjacent);
        observed.insert(name.to_string(), BTreeSet::new());
    }
    for skill in skills {
        let name = text(&skill["name"]);
        let Some(adjacent) = adjacency.get(name) else {
            continue;
        };
        let Some(cases) = skill["routing_cases"]
            .as_array()
            .filter(|cases| !cases.is_empty())
        else {
            add(
                &mut errors,
                format!("routing: {name}.routing_cases must be non-empty"),
            );
            continue;
        };
        for (index, case) in cases.iter().enumerate() {
            let label = format!("{name}.routing_cases[{index}]");
            if !exact(
                case,
                &[
                    "prompt",
                    "expected_primary",
                    "permitted_secondary",
                    "forbidden_skills",
                    "rationale",
                    "evidence_ids",
                ],
            ) {
                add(
                    &mut errors,
                    format!("routing: {label} invalid oracle fields"),
                );
                continue;
            }
            let primary = text(&case["expected_primary"]);
            let secondary = texts(&case["permitted_secondary"]);
            let forbidden = texts(&case["forbidden_skills"]);
            if !case["expected_primary"].is_string()
                || !string_list(&case["permitted_secondary"], true)
                || !string_list(&case["forbidden_skills"], true)
            {
                add(
                    &mut errors,
                    format!("routing: {label} fields must use string skill names"),
                );
                continue;
            }
            if (primary != name && !adjacent.contains(primary))
                || !nonempty(&case["prompt"])
                || secondary.iter().any(|value| !adjacent.contains(value))
                || forbidden
                    .iter()
                    .any(|value| !names.contains(value.as_str()))
                || secondary
                    .iter()
                    .any(|value| value == primary || forbidden.contains(value))
                || forbidden.iter().any(|value| value == primary)
            {
                add(
                    &mut errors,
                    format!("routing: {label} is not a deterministic routing oracle"),
                );
                continue;
            }
            let entry = observed.entry(name.to_string()).or_default();
            if primary != name {
                entry.insert(primary.into());
            }
            entry.extend(secondary);
        }
    }
    for (name, expected) in &adjacency {
        let found = observed.get(name).cloned().unwrap_or_default();
        if found != *expected {
            add(
                &mut errors,
                format!(
                    "routing: {name} adjacency coverage differs: missing {:?}, extra {:?}",
                    expected.difference(&found).collect::<Vec<_>>(),
                    found.difference(expected).collect::<Vec<_>>()
                ),
            );
        }
    }
    if !exact(graph, &["roster", "flows"]) {
        add(
            &mut errors,
            "flow_contracts: expected exactly roster and flows",
        );
        return errors;
    }
    let expected_roster=skills.iter().filter(|skill|skill["name"].is_string()).map(|skill|object!({"skill":skill["name"],"agent":format!("{}-agent",text(&skill["name"])),"phase":skill["phase"],"writes":truth(&skill["writes"])})).collect::<Vec<_>>();
    if graph["roster"] != object!(expected_roster) {
        add(
            &mut errors,
            "flow_contracts: roster must match plan skill order, phases, agents, and writes",
        );
    }
    let Some(flows) = graph["flows"].as_array().filter(|flows| !flows.is_empty()) else {
        add(
            &mut errors,
            "flow_contracts: flows must be a non-empty list",
        );
        return errors;
    };
    let roster = expected_roster
        .iter()
        .map(|entry| (text(&entry["agent"]).to_string(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut flow_names = BTreeSet::new();
    for (index, flow) in flows.iter().enumerate() {
        let label = format!("flow_contracts.flows[{index}]");
        if !exact(flow, &["name", "required_code_review", "stages"]) {
            add(
                &mut errors,
                format!("{label}: expected name, required_code_review, and stages"),
            );
            continue;
        }
        let name = text(&flow["name"]);
        if !matching(r"^[a-z0-9]+(?:-[a-z0-9]+)*$", name) || !flow_names.insert(name.to_string()) {
            add(
                &mut errors,
                format!("{label}: flow name must be valid and unique"),
            );
            continue;
        }
        let Some(stages) = flow["stages"]
            .as_array()
            .filter(|stages| !stages.is_empty())
        else {
            add(
                &mut errors,
                format!("{label}: stages must be a non-empty list"),
            );
            continue;
        };
        let mut previous = 0;
        let mut checkpoints = 0;
        let mut stage_agents = vec![];
        for (stage_index, stage) in stages.iter().enumerate() {
            let stage_label = format!("{label}.stages[{stage_index}]");
            if !exact(stage, &["phase", "agents", "parallel", "checkpoint"]) {
                add(&mut errors, format!("{stage_label}: invalid stage shape"));
                continue;
            }
            let phase = text(&stage["phase"]);
            let agents = texts(&stage["agents"]);
            let Some(order) = PHASES.iter().position(|value| *value == phase) else {
                add(&mut errors, format!("{stage_label}: invalid phase"));
                continue;
            };
            if !string_list(&stage["agents"], false)
                || agents.iter().any(|agent| {
                    !matching(r"^[a-z0-9]+(?:-[a-z0-9]+)*-agent$", agent)
                        || !roster.contains_key(agent)
                })
                || !stage["parallel"].is_boolean()
                || !stage["checkpoint"].is_boolean()
            {
                add(
                    &mut errors,
                    format!("{stage_label}: invalid agents or flags"),
                );
                continue;
            }
            if order < previous {
                add(
                    &mut errors,
                    format!("{stage_label}: phase order moves backwards"),
                );
            }
            previous = order;
            if agents.len() > 1 && stage["parallel"] != true {
                add(
                    &mut errors,
                    format!("{stage_label}: multiple agents require parallel true"),
                );
            }
            let writers = agents
                .iter()
                .filter(|agent| {
                    roster
                        .get(*agent)
                        .is_some_and(|entry| entry["writes"] == true)
                })
                .count();
            if stage["parallel"] == true && writers > 1 {
                add(
                    &mut errors,
                    format!("{stage_label}: write-capable agents are not serialized"),
                );
            }
            checkpoints += usize::from(stage["checkpoint"] == true);
            stage_agents.extend(agents);
        }
        if checkpoints == 0 {
            add(
                &mut errors,
                format!("{label}: multi-stage flow requires checkpoint"),
            );
        }
        if name == "flow-feature" {
            let expected = if roster.contains_key("code-review-agent") {
                object!("code-review-agent")
            } else {
                Value::Null
            };
            if flow["required_code_review"] != expected {
                add(
                    &mut errors,
                    format!("{label}: required_code_review differs from roster"),
                );
            }
            let review = stages
                .iter()
                .enumerate()
                .filter(|(_, stage)| {
                    stage["agents"]
                        .as_array()
                        .is_some_and(|agents| agents.contains(&flow["required_code_review"]))
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            let implementations = stages
                .iter()
                .enumerate()
                .filter(|(_, stage)| stage["phase"] == "implementation")
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            if expected.is_null() {
                if !review.is_empty() {
                    add(
                        &mut errors,
                        format!("{label}: unavailable code review appears in stages"),
                    );
                }
            } else if review.len() != 1
                || stages[review[0]]["phase"] != "verification"
                || implementations.is_empty()
                || review[0] <= *implementations.iter().max().unwrap_or(&usize::MAX)
            {
                add(
                    &mut errors,
                    format!(
                        "{label}: code-review-agent must occur once in verification after implementation"
                    ),
                );
            }
        } else if !flow["required_code_review"].is_null()
            && !stage_agents.contains(&text(&flow["required_code_review"]).to_string())
        {
            add(
                &mut errors,
                format!("{label}: required code review absent from stages"),
            );
        }
    }
    for name in ["flow-feature", "flow-review"] {
        if !flow_names.contains(name) {
            add(
                &mut errors,
                format!("flow_contracts: missing required flow {name}"),
            );
        }
    }
    errors
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
fn truth(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}
pub fn compiled_block(path: &Path) -> Result<Value> {
    let text = fs::read_text(path)?;
    let regex = expression(r"(?s)```json flow-contract\s*\n(.*?)\n```")?;
    let matches = regex.captures_iter(&text).collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(RuntimeError::invalid(format!(
            "{}: expected exactly one flow-contract JSON block",
            path.display()
        )));
    }
    let value = infra_core::domain::json::parse(&matches[0][1])?;
    if !value.is_object() {
        return Err(RuntimeError::invalid("flow-contract block must be object"));
    }
    Ok(value)
}
fn inline_parts(value: &str) -> Vec<String> {
    let mut parts = vec![];
    let mut depth = 0i64;
    let mut current = String::new();
    for character in value.chars() {
        if character == '[' {
            depth += 1;
        } else if character == ']' {
            depth -= 1;
        }
        if character == ',' && depth == 0 {
            if !current.trim().is_empty() {
                parts.push(current.trim().to_string());
            }
            current.clear();
        } else {
            current.push(character);
        }
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts
}
fn assign(stage: &mut Value, key: &str, value: &str, inline: bool) -> Result<()> {
    match key {
        "phase" => stage[key] = value.into(),
        "agents" => {
            let inner = value
                .strip_prefix('[')
                .and_then(|value| value.strip_suffix(']'))
                .ok_or_else(|| RuntimeError::invalid("agents must use inline list"))?;
            stage[key] = object!(
                inner
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>()
            );
        }
        "parallel" | "checkpoint" => {
            if !inline && !["true", "false"].contains(&value) {
                return Err(RuntimeError::invalid("block flags must be true or false"));
            }
            stage[key] = object!(!value.is_empty() && value != "false");
        }
        _ => {}
    }
    Ok(())
}
pub fn frontmatter_stages(path: &Path) -> Result<(String, Vec<Value>)> {
    let text = fs::read_text(path)?;
    let tail = text
        .strip_prefix("---\n")
        .ok_or_else(|| RuntimeError::invalid("Flow command missing frontmatter"))?;
    let (frontmatter, _) = tail
        .split_once("\n---\n")
        .ok_or_else(|| RuntimeError::invalid("Flow command unterminated frontmatter"))?;
    let mut flow = String::new();
    let mut stages = vec![];
    let mut current = None;
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("flow:") {
            flow = value.trim().into();
        } else if let Some(value) = line
            .strip_prefix("-")
            .map(str::trim)
            .and_then(|value| value.strip_prefix('{'))
            .and_then(|value| value.strip_suffix('}'))
        {
            if let Some(stage) = current.take() {
                stages.push(stage);
            }
            let mut stage = object!({});
            for part in inline_parts(value) {
                let (key, value) = part
                    .split_once(':')
                    .ok_or_else(|| RuntimeError::invalid("Stage entry missing colon"))?;
                assign(&mut stage, key.trim(), value.trim(), true)?;
            }
            stages.push(stage);
        } else if let Some(value) = line.strip_prefix("- phase:") {
            if let Some(stage) = current.take() {
                stages.push(stage);
            }
            current = Some(object!({"phase":value.trim()}));
        } else if let Some(stage) = current.as_mut()
            && let Some((key, value)) = line.split_once(':')
        {
            assign(stage, key.trim(), value.trim(), false)?;
        }
    }
    if let Some(stage) = current {
        stages.push(stage);
    }
    Ok((flow, stages))
}
pub fn validate(plan_path: &Path, skill_flow: &Path, commands: &Path) -> Vec<String> {
    let plan = match read_object(plan_path) {
        Ok(value) => value,
        Err(error) => return vec![error.to_string()],
    };
    let mut errors = validate_plan_graph(&plan);
    let graph = &plan["flow_contracts"];
    if !graph.is_object() {
        return errors;
    }
    match compiled_block(skill_flow) {
        Ok(value) if value == *graph => {}
        Ok(_) => add(
            &mut errors,
            "SKILL FLOW: compiled graph differs from flow_contracts",
        ),
        Err(error) => add(&mut errors, error.to_string()),
    }
    let flows = graph["flows"].as_array().cloned().unwrap_or_default();
    let expected = flows
        .iter()
        .filter_map(|flow| flow["name"].as_str())
        .map(|name| format!("{name}.md"))
        .collect::<BTreeSet<_>>();
    let actual = std::fs::read_dir(commands)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .filter(|name| {
            name.ends_with(".md") && (name.starts_with("flow-") || expected.contains(name))
        })
        .collect::<BTreeSet<_>>();
    if actual != expected {
        add(
            &mut errors,
            format!("commands: expected {expected:?}, found {actual:?}"),
        );
    }
    for flow in flows {
        let name = text(&flow["name"]);
        if name.is_empty() {
            continue;
        }
        let filename = format!("{name}.md");
        let path = match ownership::confined(commands, &filename) {
            Ok(path) => path,
            Err(error) => {
                add(&mut errors, error.to_string());
                continue;
            }
        };
        if !path.is_file() {
            continue;
        }
        let expected = object!({"roster":graph["roster"],"flow":flow});
        match compiled_block(&path) {
            Ok(value) if value == expected => {}
            Ok(_) => add(
                &mut errors,
                format!("{filename}: compiled roster/flow differs"),
            ),
            Err(error) => add(&mut errors, error.to_string()),
        }
        match frontmatter_stages(&path) {
            Ok((actual, stages)) => {
                if actual != name {
                    add(&mut errors, format!("{filename}: frontmatter flow differs"));
                }
                if object!(stages) != flow["stages"] {
                    add(
                        &mut errors,
                        format!("{filename}: frontmatter stages differ"),
                    );
                }
            }
            Err(error) => add(&mut errors, error.to_string()),
        }
    }
    errors
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
