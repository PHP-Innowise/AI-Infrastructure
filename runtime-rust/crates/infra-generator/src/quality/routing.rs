use super::{consts::*, prose, util::*};
use crate::common::{Diagnostic, diag};
use std::{collections::BTreeSet, path::Path};
pub fn agents(
    dir: &Path,
    plan_path: &Path,
    target: &Path,
    registry: &Path,
    require_invokes: bool,
) -> Vec<Diagnostic> {
    let mut d = super::validate_plan(plan_path, target, registry);
    if d.iter().any(|v| v.severity == "error") {
        return d;
    }
    let plan = load(plan_path, &mut d);
    let contracts = arr(&plan["skills"])
        .iter()
        .filter(|v| ne(&v["name"]))
        .map(|v| (s(&v["name"]).to_owned(), v))
        .collect::<std::collections::BTreeMap<_, _>>();
    let dir = super::plan::absolute(dir);
    if !dir.is_dir() {
        diag(
            &mut d,
            "AGENTS_DIR_INVALID",
            format!("agents directory invalid: {}", dir.display()),
        );
        return finish(d);
    }
    let entries = match std::fs::read_dir(&dir) {
        Ok(v) => v,
        Err(e) => {
            diag(&mut d, "AGENTS_DIR_INVALID", e.to_string());
            return finish(d);
        }
    };
    let mut actual = BTreeSet::new();
    for entry in entries.flatten() {
        let p = entry.path();
        let file = entry.file_name().to_string_lossy().into_owned();
        if !file.ends_with(".md") {
            continue;
        }
        if file == "README.md" {
            continue;
        }
        if !file.ends_with("-agent.md") || p.is_symlink() {
            diag(
                &mut d,
                "AGENT_ROUTING_NONCONFORMING_FILE",
                format!("nonconforming agent roster file: {file}"),
            )
        }
        if file.ends_with("-agent.md") && p.is_file() && !p.is_symlink() {
            actual.insert(file.strip_suffix("-agent.md").unwrap().to_owned());
        }
    }
    for name in actual.iter().filter(|n| !contracts.contains_key(*n)) {
        diag(
            &mut d,
            "AGENT_ROUTING_UNPLANNED",
            format!("{name}: wrapper has no skill contract"),
        )
    }
    for (name, contract) in contracts {
        let path = dir.join(format!("{name}-agent.md"));
        if !path.is_file() || path.is_symlink() {
            diag(
                &mut d,
                "AGENT_ROUTING_MISSING",
                format!("{name}: matching wrapper missing"),
            );
            continue;
        }
        let text = match read_text(path) {
            Ok(v) => v,
            Err(e) => {
                diag(&mut d, "AGENT_ROUTING_UNREADABLE", format!("{name}: {e}"));
                continue;
            }
        };
        let (front, body) = prose::frontmatter(&text);
        if front.get("name") != Some(&format!("{name}-agent")) {
            diag(
                &mut d,
                "AGENT_FRONTMATTER_NAME",
                format!("{name}: wrapper name must be {name}-agent"),
            )
        }
        if (require_invokes && front.get("invokes") != Some(&name))
            || front.get("invokes").is_some_and(|v| v != &name)
        {
            diag(
                &mut d,
                "AGENT_INVOKES_MISMATCH",
                format!("{name}: wrapper must invoke exactly {name}"),
            )
        }
        let description = front.get("description").map(String::as_str).unwrap_or("");
        for (value, haystack, code) in [
            (
                &contract["triggers"]["positive"],
                format!("{description}\n{body}"),
                "AGENT_POSITIVE_ROUTING",
            ),
            (
                &contract["triggers"]["negative"],
                body.clone(),
                "AGENT_NEGATIVE_ROUTING",
            ),
            (
                &contract["output_contract"],
                body.clone(),
                "AGENT_OUTPUT_ROUTING",
            ),
        ] {
            if !prose::matches(value, &haystack) {
                diag(
                    &mut d,
                    code,
                    format!("{name}: routing/output contract not traceable to wrapper"),
                )
            }
        }
        for sibling in arr(&contract["nearest_siblings"]) {
            if !body
                .to_lowercase()
                .contains(&s(&sibling["name"]).to_lowercase())
                || !prose::matches(&sibling["boundary"], &body)
            {
                diag(
                    &mut d,
                    "AGENT_SIBLING_ROUTING",
                    format!("{name}: sibling boundary/deferral not traceable"),
                )
            }
        }
        let writes = front
            .get("writes")
            .is_some_and(|s| s.trim().eq_ignore_ascii_case("true"));
        if writes != (!arr(&contract["writes"]).is_empty()) {
            diag(
                &mut d,
                "AGENT_WRITES_MISMATCH",
                format!("{name}: wrapper writes differs from contract"),
            )
        }
        let pattern = name
            .split('-')
            .map(super::regex_adapter::escape)
            .collect::<Vec<_>>()
            .join(r"[- ]");
        if has(
            &format!(r"(?i)\b(use|select)\b.*\b{pattern}\b.*\b(governed|handled|skill)\b"),
            description,
        ) {
            diag(
                &mut d,
                "AGENT_CIRCULAR_ROUTING",
                format!("{name}: circular wrapper description"),
            )
        }
    }
    finish(d)
}
pub fn flows(dir: &Path, plan_path: &Path, target: &Path, registry: &Path) -> Vec<Diagnostic> {
    let mut d = super::validate_plan(plan_path, target, registry);
    if d.iter().any(|v| v.severity == "error") {
        return d;
    }
    let plan = load(plan_path, &mut d);
    let specialists = arr(&plan["skills"])
        .iter()
        .filter(|v| SPECIALIST_CATEGORIES.contains(&s(&v["category"]).to_lowercase().as_str()))
        .collect::<Vec<_>>();
    let dir = super::plan::absolute(dir);
    if !dir.is_dir() {
        diag(
            &mut d,
            "COMMANDS_DIR_INVALID",
            format!("commands directory invalid: {}", dir.display()),
        );
        return finish(d);
    }
    for name in ["flow-feature", "flow-review"] {
        let path = dir.join(format!("{name}.md"));
        if !path.is_file() || path.is_symlink() {
            diag(
                &mut d,
                "FLOW_ROUTING_MISSING",
                format!("{name}: generated flow command missing"),
            );
            continue;
        }
        let text = match read_text(&path) {
            Ok(v) => v,
            Err(e) => {
                diag(&mut d, "FLOW_ROUTING_UNREADABLE", format!("{name}: {e}"));
                continue;
            }
        };
        let (_, body) = prose::frontmatter(&text);
        if !specialists.is_empty() && !body.to_lowercase().contains("specialist routing") {
            diag(
                &mut d,
                "FLOW_SPECIALIST_SECTION",
                format!("{name}: Specialist Routing section missing"),
            )
        }
        let front = if text.starts_with("---") {
            text.split("---").nth(1).unwrap_or("")
        } else {
            ""
        };
        for specialist in &specialists {
            let agent = format!("{}-agent", s(&specialist["name"]));
            if front.contains(&agent) {
                diag(
                    &mut d,
                    "FLOW_SPECIALIST_UNCONDITIONAL",
                    format!("{name}: specialist {agent} in static stage"),
                )
            }
            let routing = body
                .lines()
                .filter(|l| l.to_lowercase().contains(&agent.to_lowercase()))
                .collect::<Vec<_>>()
                .join("\n");
            if routing.is_empty() {
                diag(
                    &mut d,
                    "FLOW_SPECIALIST_UNROUTED",
                    format!("{name}: specialist {agent} absent from routing"),
                )
            }
            if !prose::matches(&specialist["triggers"]["positive"], &routing)
                || !prose::matches(&specialist["triggers"]["negative"], &routing)
            {
                diag(
                    &mut d,
                    "FLOW_SPECIALIST_TRIGGER",
                    format!("{name}: {agent} lacks positive/negative scope routing"),
                )
            }
            if name == "flow-review"
                && !arr(&specialist["writes"]).is_empty()
                && !has(
                    &format!(
                        r"(?i){}[^\n]*(skip|not selected|write-capable)",
                        super::regex_adapter::escape(&agent)
                    ),
                    &routing,
                )
            {
                diag(
                    &mut d,
                    "FLOW_REVIEW_WRITER",
                    format!("{name}: write-capable {agent} must be explicitly skipped"),
                )
            }
        }
    }
    finish(d)
}
