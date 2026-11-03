use super::{consts::*, paths, prose, search, util::*};
use crate::common::{Diagnostic, diag, warn};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::Path,
};
pub fn outcomes() -> BTreeMap<String, Vec<Value>> {
    crate::assets::runtime_contract()["commands"]["outcomes"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.iter())
        .filter_map(|(k, v)| {
            let valid = arr(v)
                .iter()
                .filter(|e| {
                    (e["exit"].as_i64().is_some() || e["exit"].as_u64().is_some())
                        && ne(&e["meaning"])
                })
                .cloned()
                .collect::<Vec<_>>();
            (!valid.is_empty()).then(|| (prose::normalize_command(k).trim().to_owned(), valid))
        })
        .collect()
}
fn purposes() -> BTreeMap<String, String> {
    crate::assets::runtime_contract()["commands"]["purposes"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.iter())
        .filter_map(|(k, v)| {
            v.as_str()
                .map(|s| (prose::normalize_command(k).trim().to_owned(), s.to_owned()))
        })
        .collect()
}
pub fn attested(command: &str, analysis: &crate::commands::CommandAnalysis) -> bool {
    if analysis
        .categories
        .iter()
        .any(|c| !["non_mutating", "verification_blocker"].contains(&c.as_str()))
    {
        return false;
    }
    flat(&crate::assets::runtime_contract()["commands"]["read_health"])
        .iter()
        .any(|c| prose::normalize_command(c).trim() == prose::normalize_command(command).trim())
}
pub fn baseline(
    name: &str,
    check: &Value,
    target: &Path,
    capability: &str,
    runtime: bool,
    d: &mut Vec<Diagnostic>,
) {
    let label = format!("{name}.{}", s(&check["id"]));
    let command = s(&check["command"]);
    let baseline = &check["baseline"];
    let parsed = search::parse(command, s(&check["skip_condition"]), target);
    let declared = if runtime {
        outcomes().remove(prose::normalize_command(command).trim())
    } else {
        None
    };
    if let Some(declared) = declared {
        let exits = declared
            .iter()
            .filter_map(|v| v["exit"].as_i64())
            .collect::<BTreeSet<_>>();
        if exits.iter().any(|n| *n != 0)
            && has(ABSOLUTE_SUCCESS_PATTERN, s(&check["expected_result"]))
            && !has(
                DIFFERENTIAL_EXPECTATION_PATTERN,
                s(&check["expected_result"]),
            )
        {
            diag(
                d,
                "RUNTIME_EXPECTATION_CONTRADICTED",
                format!(
                    "{label} promises absolute success despite declared runtime failure outcomes"
                ),
            )
        }
        if !baseline.is_null() {
            if !exact(baseline, &["command", "observed", "outcome"])
                || !ne(&baseline["command"])
                || !ne(&baseline["observed"])
                || !VERIFICATION_BASELINE_OUTCOMES.contains(&s(&baseline["outcome"]))
            {
                diag(
                    d,
                    "VERIFICATION_BASELINE_INVALID",
                    format!("{label} has malformed runtime baseline"),
                )
            } else if (s(&baseline["outcome"]) == "passing" && !exits.contains(&0))
                || (s(&baseline["outcome"]) != "passing" && exits == BTreeSet::from([0]))
            {
                diag(
                    d,
                    "RUNTIME_BASELINE_CONTRADICTED",
                    format!("{label} baseline contradicts runtime exit contract"),
                )
            }
        }
        return;
    }
    if baseline.is_null() {
        if s(&check["mode"]) == "command" && parsed.is_none() {
            diag(
                d,
                "VERIFICATION_BASELINE_MISSING",
                format!("{label} expectation requires an observed unmodified-target baseline"),
            )
        }
        return;
    }
    if !exact(baseline, &["command", "observed", "outcome"])
        || !ne(&baseline["command"])
        || !ne(&baseline["observed"])
        || !VERIFICATION_BASELINE_OUTCOMES.contains(&s(&baseline["outcome"]))
    {
        diag(
            d,
            "VERIFICATION_BASELINE_INVALID",
            format!("{label} baseline must record command, observation and outcome"),
        );
        return;
    }
    if s(&check["mode"]) != "command" {
        diag(
            d,
            "VERIFICATION_BASELINE_INVALID",
            format!("{label} manual check cannot carry an executed baseline"),
        );
        return;
    }
    if s(&baseline["command"]).trim() != command.trim() {
        diag(
            d,
            "VERIFICATION_BASELINE_COMMAND_MISMATCH",
            format!("{label} baseline records a different command"),
        );
        return;
    }
    if s(&baseline["outcome"]) == "failing"
        && has(ABSOLUTE_SUCCESS_PATTERN, s(&check["expected_result"]))
        && !has(
            DIFFERENTIAL_EXPECTATION_PATTERN,
            s(&check["expected_result"]),
        )
    {
        diag(
            d,
            "VERIFICATION_BASELINE_CONTRADICTED",
            format!("{label} promises outright success while unmodified target baseline fails"),
        )
    }
    if s(&baseline["outcome"]) == "failing-remediated" && capability == "read-only" {
        diag(
            d,
            "VERIFICATION_BASELINE_REMEDIATION_CONFLICT",
            format!("{label} read-only skill claims to remediate a failure"),
        )
    }
    if let Some(query) = parsed {
        let resolved = match search::resolve(target, &query) {
            Ok(value) => value,
            Err(error) => {
                diag(d, "QUALITY_GLOB_UNRESOLVED", error.to_string());
                return;
            }
        };
        if let Some((_, matched)) = resolved
            && !matched.is_empty() != (s(&baseline["outcome"]) == "passing")
        {
            diag(
                d,
                "VERIFICATION_BASELINE_CONTRADICTED",
                format!(
                    "{label} recorded baseline contradicts {} resolved matches",
                    matched.len()
                ),
            )
        }
    }
}
fn base(command: &str) -> String {
    prose::normalize_command(command)
        .split_whitespace()
        .filter(|s| !s.starts_with('-'))
        .collect::<Vec<_>>()
        .join(" ")
}
fn singular(word: &str) -> String {
    if word.len() > 3 && word.ends_with('s') && !word.ends_with("ss") {
        word[..word.len() - 1].to_owned()
    } else {
        word.into()
    }
}
fn subcommand_words(command: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for raw in command.split_whitespace() {
        let token = if raw.starts_with('-') {
            raw.trim_start_matches('-')
        } else if raw.contains('/') || raw.ends_with(".exe") {
            ""
        } else {
            raw
        };
        let token = token.trim().to_lowercase();
        if !token.is_empty()
            && !["infra-engine", "infra-engine.exe", "json", "memory"].contains(&token.as_str())
        {
            out.insert(token);
        }
    }
    out
}
pub fn description(name: &str, check: &Value, command: &str, d: &mut Vec<Diagnostic>) {
    let purposes = purposes();
    let normalized = prose::normalize_command(command);
    let Some(own) = purposes.get(normalized.trim()) else {
        return;
    };
    let language = CLAIM_LANGUAGE_LEXICON
        .iter()
        .map(|s| s.to_string())
        .collect::<BTreeSet<_>>();
    let described = prose::meaningful(&format!(
        "{} {}",
        s(&check["instruction"]),
        s(&check["expected_result"])
    ))
    .difference(&language)
    .map(|s| singular(s))
    .collect::<BTreeSet<_>>();
    let score = |purpose: &str, command: &str| {
        let mut words = prose::meaningful(purpose);
        words.extend(subcommand_words(command));
        words
            .difference(&language)
            .map(|s| singular(s))
            .collect::<BTreeSet<_>>()
            .intersection(&described)
            .count()
    };
    let own_score = score(own, command);
    if purposes
        .iter()
        .filter(|(k, _)| base(k) != base(command))
        .any(|(k, v)| score(v, k) > own_score)
    {
        warn(
            d,
            "RUNTIME_COMMAND_DESCRIPTION",
            format!(
                "{name}.{} describes another command's job; canonical purpose: {own}",
                s(&check["id"])
            ),
        )
    }
}
pub fn signature(command: &str) -> Option<(String, String, BTreeSet<String>)> {
    let tokens = command
        .split_whitespace()
        .map(|s| s.trim_matches(['.', ',', ';', ':']))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    if tokens.len() < 2
        || !["infra-engine", "infra-engine.exe"]
            .contains(&tokens[0].rsplit('/').next().unwrap_or(tokens[0]))
        || !["memory", "bank", "continuity"].contains(&tokens[1])
    {
        return None;
    }
    let mut sub = tokens[1].to_owned();
    let mut flags = BTreeSet::new();
    let mut selected = false;
    for token in &tokens[2..] {
        if token.starts_with('-') {
            flags.insert(token.split('=').next().unwrap().to_owned());
        } else if !selected && flags.is_empty() {
            sub.push(' ');
            sub.push_str(token);
            selected = true
        }
    }
    Some((tokens[0].to_owned(), sub, flags))
}
fn forms(value: &Value) -> Vec<String> {
    match value {
        Value::String(s) => s
            .contains("memory-bank/bin/infra-engine")
            .then(|| s.clone())
            .into_iter()
            .collect(),
        Value::Array(v) => v.iter().flat_map(forms).collect(),
        Value::Object(v) => v
            .iter()
            .flat_map(|(k, v)| forms(&Value::String(k.clone())).into_iter().chain(forms(v)))
            .collect(),
        _ => Vec::new(),
    }
}
pub fn body(
    name: &str,
    skill: &Value,
    body: &str,
    evidence: &EvidenceMap,
    d: &mut Vec<Diagnostic>,
) {
    let doc = crate::assets::runtime_contract();
    let mut declared = Vec::new();
    for value in ss(&doc["path_contracts"]["required_skeleton"])
        .into_iter()
        .chain(
            arr(&doc["path_contracts"]["creatable"])
                .iter()
                .map(|v| s(&v["path"]).to_owned()),
        )
    {
        for value in paths::expand(&value) {
            declared.push(paths::segments(&value));
        }
    }
    let forbidden = ss(&doc["path_contracts"]["forbidden_invented_paths"])
        .iter()
        .flat_map(|p| paths::expand(p))
        .map(|p| paths::segments(&p))
        .collect::<Vec<_>>();
    let mut commands: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for form in forms(&doc["commands"]) {
        if let Some((exe, sub, flags)) = signature(&form) {
            commands.entry((exe, sub)).or_default().extend(flags);
        }
    }
    for raw in words(RUNTIME_BODY_PATH, body)
        .into_iter()
        .collect::<BTreeSet<_>>()
    {
        let path = rstrip(raw.trim(), ".,;:)]/");
        if path.is_empty() {
            continue;
        }
        let alternatives = paths::expand(&path);
        if alternatives.len() > 1
            && alternatives
                .iter()
                .all(|p| declared.iter().any(|c| paths::run(&paths::segments(p), c)))
        {
            continue;
        }
        let seg = paths::segments(&path);
        if forbidden.iter().any(|c| paths::beneath(&seg, c)) {
            diag(
                d,
                "RUNTIME_PATH_FORBIDDEN",
                format!("{name}: invented runtime path: {path}"),
            )
        } else if !declared.iter().any(|c| paths::run(&seg, c)) {
            diag(
                d,
                "RUNTIME_PATH_UNSUPPORTED",
                format!("{name}: runtime path absent from trusted contract: {path}"),
            )
        }
    }
    for raw in words(
        r"memory-bank/bin/infra-engine(?:\.exe)?(?:[ \t]+[^\s\x60|;&<>()]+)+",
        body,
    ) {
        if let Some((exe, sub, flags)) = signature(&raw)
            && !commands
                .get(&(exe, sub))
                .is_some_and(|allowed| flags.is_subset(allowed))
        {
            diag(
                d,
                "RUNTIME_COMMAND_UNSUPPORTED",
                format!(
                    "{name}: runtime command absent from trusted contract: {}",
                    prose::excerpt(&raw)
                ),
            )
        }
    }
    let mut target = ss(&skill["source_paths"]);
    for id in ss(&skill["evidence_ids"]) {
        if let Some((kind, path)) = evidence.get(&id)
            && kind == "path"
        {
            target.push(path.clone())
        }
    }
    for span in words(CODE_SPAN_CONTENT, body) {
        let Some(path) = paths::named(&span) else {
            continue;
        };
        let seg = paths::segments(&path);
        if declared.iter().any(|c| paths::run(&seg, c))
            || seg
                .first()
                .is_some_and(|p| RUNTIME_ROOTS.contains(&p.as_str()))
        {
            continue;
        }
        if !target.iter().any(|declared| {
            paths::intersects(&path, declared) || declared.starts_with(&(path.clone() + "/"))
        }) {
            diag(
                d,
                "RUNTIME_PROJECT_CLAIM_UNSUPPORTED",
                format!("{name}: runtime-fixed body claims undeclared target knowledge: {path}"),
            )
        }
    }
}
pub fn command_segments(command: &str) -> Vec<String> {
    let (mut out, mut current, mut quote) = (Vec::new(), String::new(), '\0');
    for c in command.chars() {
        if quote != '\0' {
            current.push(c);
            if c == quote {
                quote = '\0'
            }
            continue;
        }
        if c == '\'' || c == '"' {
            quote = c;
            current.push(c)
        } else if COMMAND_SEGMENT_SEPARATORS.contains(&c.to_string().as_str()) {
            if !current.trim().is_empty() {
                out.push(current.trim().to_owned())
            }
            current.clear()
        } else {
            current.push(c)
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_owned())
    }
    out
}
pub fn body_commands(name: &str, body: &str, target: &Path, d: &mut Vec<Diagnostic>) {
    let mut commands = prose::shell_blocks(body)
        .iter()
        .flat_map(|block| prose::shell_commands(block))
        .map(|s| (s, true))
        .collect::<Vec<_>>();
    for c in finds(INLINE_CODE_PATTERN, body) {
        if !prose::forbids(body, c.start, c.end - c.start, false) {
            let command = prose::normalize_command(c.groups[1].as_deref().unwrap_or(""));
            if !command.trim().is_empty() {
                commands.push((command.trim().into(), false))
            }
        }
    }
    if commands.is_empty() {
        return;
    }
    let analyzer = match crate::commands::CommandAnalyzer::new(target) {
        Ok(v) => v,
        Err(e) => {
            diag(
                d,
                "COMMAND_ANALYZER_UNAVAILABLE",
                format!("{name} target command analysis unavailable: {e}"),
            );
            return;
        }
    };
    for (command, strict) in commands {
        let mut pending = command_segments(&command)
            .into_iter()
            .map(|s| (s, strict))
            .collect::<VecDeque<_>>();
        let mut seen = BTreeSet::new();
        while let Some((segment, strict)) = pending.pop_front() {
            if seen.len() >= MAX_BODY_COMMAND_SEGMENTS {
                diag(
                    d,
                    "SKILL_BODY_COMMAND_UNSCANNED",
                    format!(
                        "{name} prose command exceeds {MAX_BODY_COMMAND_SEGMENTS}-segment cap: {command}"
                    ),
                );
                break;
            }
            if !seen.insert((segment.clone(), strict)) {
                continue;
            }
            let tokens = segment
                .split_whitespace()
                .skip_while(|s| has(ENV_ASSIGNMENT_TOKEN, s))
                .collect::<Vec<_>>();
            let Some(head) = tokens.first() else { continue };
            let executable = head.rsplit('/').next().unwrap_or(head).to_lowercase();
            if !crate::commands::known_executable(&executable)
                || (!strict
                    && (tokens.len() < 2
                        || (AMBIGUOUS_BARE_EXECUTABLES.contains(&executable.as_str())
                            && !head.contains('/'))))
            {
                continue;
            }
            if [
                "bash", "sh", "dash", "zsh", "fish", "ksh", "python", "python3", "php", "node",
                "ruby", "perl",
            ]
            .contains(&executable.as_str())
                && let Ok(parsed) = crate::commands::tokenize(&segment)
            {
                for token in parsed
                    .into_iter()
                    .skip(1)
                    .filter(|t| !t.trim().is_empty() && !t.starts_with('-'))
                {
                    pending.extend(command_segments(&token).into_iter().map(|s| (s, true)));
                }
            }
            if let Ok(analysis) = analyzer.analyze(&segment, false, None) {
                let codes = analysis
                    .findings
                    .iter()
                    .filter(|f| {
                        COMMAND_RISK_CATEGORIES.contains(&f.category.as_str())
                            || COMMAND_BLOCKING_FINDINGS.contains(&f.code.as_str())
                    })
                    .map(|f| f.code.clone())
                    .collect::<BTreeSet<_>>();
                if !codes.is_empty() {
                    diag(
                        d,
                        "SKILL_BODY_COMMAND_RISK",
                        format!(
                            "{name}: body prescribes unsafe command ({}): {segment}",
                            codes.into_iter().collect::<Vec<_>>().join(", ")
                        ),
                    )
                }
            }
        }
    }
}
