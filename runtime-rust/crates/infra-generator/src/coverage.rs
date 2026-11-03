use crate::common::*;
use infra_core::{Result, fs};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
const DISPOSITIONS: &[&str] = &["covered", "excluded", "truncated", "not-permitted"];
fn exact(value: &Value, names: &[&str]) -> bool {
    keys(value) == names.iter().map(|value| value.to_string()).collect()
}
pub fn matches(surface: &str, kind: &str, path: &str) -> Result<bool> {
    let surface = trailing(surface.trim(), &['/']);
    Ok(if ["*", "**"].contains(&surface) {
        true
    } else if let Some(prefix) = surface.strip_suffix("/**") {
        path == prefix || path.starts_with(&format!("{prefix}/"))
    } else if kind == "tree" {
        path == surface || path.starts_with(&format!("{surface}/"))
    } else if surface.contains(['*', '?', '[']) {
        glob_match(surface, path)?
    } else {
        path == surface
    })
}
fn specificity(surface: &str, path: &str) -> usize {
    let surface = trailing(surface.trim(), &['/']);
    if ["*", "**"].contains(&surface) {
        0
    } else if let Some(prefix) = surface.strip_suffix("/**") {
        prefix.split('/').count()
    } else if surface == path {
        path.split('/').count() + 1
    } else {
        surface.split('/').count()
    }
}
fn secret(surface: &str) -> Result<bool> {
    let name = trailing(surface.trim(), &['/'])
        .rsplit('/')
        .next()
        .unwrap_or("");
    [
        ".env",
        ".env.*",
        "*.pem",
        "*.key",
        "*.p12",
        "*.pfx",
        "*.jks",
        "id_rsa",
        "id_ed25519",
        "*.keystore",
        "auth.json",
    ]
    .iter()
    .try_fold(false, |found, pattern| {
        Ok(found || glob_match(pattern, name)?)
    })
}
fn direct(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = std::fs::read_dir(root)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    files.sort();
    Ok(files)
}
pub fn target_surfaces(target: &Path, items: &mut Vec<Diagnostic>) -> Vec<String> {
    let entries = match direct(target) {
        Ok(entries) => entries,
        Err(error) => {
            diag(items, "SCAN_TARGET_UNREADABLE", error.to_string());
            return vec![];
        }
    };
    let mut surfaces = vec![];
    for entry in entries {
        let name = entry
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if name == ".git"
            || std::fs::symlink_metadata(&entry)
                .map(|metadata| fs::linked(&metadata))
                .unwrap_or(true)
        {
            continue;
        }
        surfaces.push(name.to_string());
        if entry.is_dir()
            && [
                "src", "app", "apps", "lib", "libs", "modules", "packages", "bundles", "tests",
                "test", "config",
            ]
            .contains(&name)
        {
            match direct(&entry) {
                Ok(children) => {
                    for child in children {
                        if std::fs::symlink_metadata(&child)
                            .is_ok_and(|metadata| !fs::linked(&metadata))
                        {
                            surfaces.push(format!(
                                "{name}/{}",
                                child
                                    .file_name()
                                    .and_then(|value| value.to_str())
                                    .unwrap_or("")
                            ));
                        }
                    }
                }
                Err(error) => diag(items, "SCAN_TARGET_UNREADABLE", format!("{name}: {error}")),
            }
        }
        if surfaces.len() > 4000 {
            diag(
                items,
                "SCAN_TARGET_TOO_LARGE",
                "target exposes more than 4000 surfaces",
            );
            return vec![];
        }
    }
    surfaces
}
fn locator(entry: &Value) -> String {
    for field in ["path", "url"] {
        if nonempty(&entry[field]) {
            return text(&entry[field]).trim().into();
        }
    }
    text(&entry["absence"]["subject"]).trim().into()
}
fn locators(document: &Value) -> BTreeMap<String, String> {
    document["evidence"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| nonempty(&entry["id"]))
        .map(|entry| (text(&entry["id"]).trim().into(), locator(entry)))
        .collect()
}
fn claim_tokens(value: &str) -> BTreeSet<String> {
    let service = [
        "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "has", "have", "in", "is",
        "it", "its", "must", "never", "no", "not", "of", "on", "once", "only", "or", "that", "the",
        "their", "then", "this", "to", "when", "which", "with",
    ];
    word_tokens(value)
        .into_iter()
        .filter(|word| word.chars().count() > 2 && !service.contains(&word.as_str()))
        .collect()
}
fn same_statement(left: &str, right: &str) -> bool {
    claim_tokens(left)
        .intersection(&claim_tokens(right))
        .count()
        >= 2
}
fn validate_claims(
    task: &Path,
    known: &BTreeMap<String, BTreeSet<String>>,
    items: &mut Vec<Diagnostic>,
) -> Vec<Value> {
    let path = task.join("project-claims.json");
    if !path.is_file() {
        diag(
            items,
            "CLAIMS_MISSING",
            "reconciliation wrote no project-claims.json",
        );
        return vec![];
    }
    let Some(document) = load_diagnostic(&path, "SCAN_ARTIFACT", items) else {
        return vec![];
    };
    if !exact(&document, &["target_root", "claims", "contradictions"]) {
        diag(
            items,
            "CLAIMS_INVALID",
            "project-claims.json must define target_root, claims and contradictions",
        );
        return vec![];
    }
    let evidence = known.values().flatten().cloned().collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut claims = vec![];
    for (index, claim) in document["claims"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if !exact(
            claim,
            &[
                "id",
                "statement",
                "claim_class",
                "priority",
                "evidence_ids",
                "scanners",
                "status",
            ],
        ) || !nonempty(&claim["id"])
            || !nonempty(&claim["statement"])
            || ![
                "invariant",
                "capability",
                "convention",
                "risk",
                "integration",
                "command",
            ]
            .contains(&text(&claim["claim_class"]))
            || !["high", "medium", "low"].contains(&text(&claim["priority"]))
            || !["confirmed", "inferred", "unknown"].contains(&text(&claim["status"]))
            || !string_list(&claim["evidence_ids"], false)
            || !string_list(&claim["scanners"], false)
        {
            diag(
                items,
                "CLAIMS_INVALID",
                format!("claims[{index}] has invalid shape"),
            );
            continue;
        }
        let id = text(&claim["id"]).trim();
        if !seen.insert(id.to_string()) {
            diag(items, "CLAIM_ID_DUPLICATE", format!("duplicate claim {id}"));
            continue;
        }
        if texts(&claim["evidence_ids"])
            .iter()
            .any(|id| !evidence.contains(id.trim()))
        {
            diag(
                items,
                "CLAIM_EVIDENCE_UNKNOWN",
                format!("{id}: rests on unknown evidence"),
            );
            continue;
        }
        if texts(&claim["scanners"])
            .iter()
            .any(|scanner| !known.contains_key(scanner.trim()))
        {
            diag(
                items,
                "CLAIM_SCANNER_UNKNOWN",
                format!("{id}: attributed to unknown scanner"),
            );
            continue;
        }
        claims.push(claim.clone());
    }
    let by_id = claims
        .iter()
        .map(|claim| (text(&claim["id"]).trim().to_string(), claim))
        .collect::<BTreeMap<_, _>>();
    for (index, entry) in document["contradictions"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if !exact(entry, &["id", "claim_ids", "statement", "resolution"])
            || !nonempty(&entry["id"])
            || !nonempty(&entry["statement"])
            || !["unresolved", "resolved", "accepted"].contains(&text(&entry["resolution"]))
            || !string_list(&entry["claim_ids"], false)
            || texts(&entry["claim_ids"]).len() < 2
        {
            diag(
                items,
                "CLAIMS_INVALID",
                format!("contradictions[{index}] has invalid shape"),
            );
            continue;
        }
        let ids = texts(&entry["claim_ids"]);
        if ids.iter().any(|id| !by_id.contains_key(id.trim())) {
            diag(
                items,
                "CONTRADICTION_CLAIM_UNKNOWN",
                format!("{}: names unknown claims", text(&entry["id"])),
            );
            continue;
        }
        if entry["resolution"] != "unresolved" {
            continue;
        }
        if ids
            .iter()
            .filter_map(|id| by_id.get(id.trim()))
            .any(|claim| claim["claim_class"] == "invariant" && claim["priority"] == "high")
        {
            diag(
                items,
                "CONTRADICTION_UNRESOLVED_INVARIANT",
                format!(
                    "{}: high priority invariant in unresolved conflict",
                    text(&entry["id"])
                ),
            );
        } else {
            warn(
                items,
                "CONTRADICTION_UNRESOLVED",
                format!("{}: {}", text(&entry["id"]), text(&entry["statement"])),
            );
        }
    }
    claims
}
fn surface_root(value: &str) -> &str {
    let value = trailing(value.trim(), &['/']);
    value
        .strip_suffix("/**")
        .or_else(|| value.strip_suffix("/*"))
        .unwrap_or(value)
}
fn paths_meet(left: &str, right: &str) -> Result<bool> {
    let (left, right) = (surface_root(left), surface_root(right));
    Ok(!left.is_empty()
        && !right.is_empty()
        && (left == right
            || left.starts_with(&format!("{right}/"))
            || right.starts_with(&format!("{left}/"))
            || glob_match(left, right)?
            || glob_match(right, left)?))
}
fn ownership_coverage(
    declared: &[(String, String, String)],
    task: &Path,
    plan: &Value,
    items: &mut Vec<Diagnostic>,
) -> Result<()> {
    let Some(skills) = plan["skills"]
        .as_array()
        .filter(|skills| !skills.is_empty())
    else {
        return Ok(());
    };
    let mut owned = vec![];
    for skill in skills {
        owned.extend(texts(&skill["writes"]));
        for row in skill["ownership"].as_array().into_iter().flatten() {
            owned.extend(texts(&row["paths"]));
        }
        for row in skill["path_contracts"].as_array().into_iter().flatten() {
            if nonempty(&row["path"]) {
                owned.push(text(&row["path"]).to_string());
            }
        }
    }
    if owned.is_empty() {
        return Ok(());
    }
    let mut needing = BTreeSet::new();
    for (_, surface, disposition) in declared {
        if ["covered", "truncated"].contains(&disposition.as_str()) && !secret(surface)? {
            needing.insert(surface.clone());
        }
    }
    let mut unowned = BTreeSet::new();
    for surface in &needing {
        let covered = owned
            .iter()
            .try_fold(false, |found, path| -> Result<bool> {
                Ok(found || paths_meet(surface, path)?)
            })?;
        if !covered {
            unowned.insert(surface.clone());
        }
    }
    let path = task.join("plan-ownership.json");
    if !path.is_file() {
        if !unowned.is_empty() {
            diag(
                items,
                "OWNERSHIP_COVERAGE_MISSING",
                format!(
                    "{} read surfaces have no skill owner or explicit disposition",
                    unowned.len()
                ),
            );
        }
        return Ok(());
    }
    let Some(record) = load_diagnostic(&path, "SCAN_ARTIFACT", items) else {
        return Ok(());
    };
    if keys(&record)
        .iter()
        .any(|key| !["target_root", "unowned"].contains(&key.as_str()))
        || !record["unowned"].is_array()
    {
        diag(
            items,
            "OWNERSHIP_COVERAGE_INVALID",
            "ownership artifact must carry target_root and unowned",
        );
        return Ok(());
    }
    let mut declared_unowned = BTreeSet::new();
    for entry in record["unowned"].as_array().into_iter().flatten() {
        if !exact(entry, &["surface", "reason"])
            || !nonempty(&entry["surface"])
            || !nonempty(&entry["reason"])
        {
            diag(
                items,
                "OWNERSHIP_COVERAGE_INVALID",
                "unowned record needs surface and reason",
            );
            continue;
        }
        let surface = text(&entry["surface"]).trim();
        if !declared_unowned.insert(surface.to_string()) {
            diag(
                items,
                "OWNERSHIP_COVERAGE_INVALID",
                format!("{surface}: duplicate unowned declaration"),
            );
        }
        if !needing.contains(surface) || !unowned.contains(surface) {
            diag(
                items,
                "OWNERSHIP_DECLARATION_UNKNOWN",
                format!("{surface}: unowned declaration conflicts with scan or selected ownership"),
            );
        }
    }
    for surface in unowned.difference(&declared_unowned) {
        diag(
            items,
            "OWNERSHIP_SURFACE_UNOWNED",
            format!("{surface}: scanned material has no owner or deliberate exclusion"),
        );
    }
    Ok(())
}
fn carry_invariants(
    claims: &[Value],
    ledger: &BTreeMap<String, String>,
    plan: &Value,
    items: &mut Vec<Diagnostic>,
) {
    let known = claims
        .iter()
        .map(|claim| text(&claim["id"]).trim())
        .collect::<BTreeSet<_>>();
    let mut referenced = BTreeSet::new();
    for skill in plan["skills"].as_array().into_iter().flatten() {
        for id in texts(&skill["claim_ids"]) {
            let id = id.trim();
            if !known.contains(id) {
                diag(
                    items,
                    "CLAIM_ID_UNKNOWN",
                    format!("{}: unknown claim {id}", text(&skill["name"])),
                );
            }
            referenced.insert(id.to_string());
        }
    }
    let plan_locators = locators(plan);
    let carried = plan["critical_invariants"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| nonempty(&entry["statement"]))
        .map(|entry| {
            (
                text(&entry["statement"]),
                texts(&entry["evidence_ids"])
                    .into_iter()
                    .filter_map(|id| plan_locators.get(id.trim()).cloned())
                    .filter(|value| !value.is_empty())
                    .collect::<BTreeSet<_>>(),
            )
        })
        .collect::<Vec<_>>();
    for claim in claims {
        if claim["claim_class"] != "invariant"
            || claim["priority"] != "high"
            || referenced.contains(text(&claim["id"]).trim())
        {
            continue;
        }
        let sources = texts(&claim["evidence_ids"])
            .into_iter()
            .filter_map(|id| ledger.get(id.trim()).cloned())
            .filter(|value| !value.is_empty())
            .collect::<BTreeSet<_>>();
        if !carried.iter().any(|(statement, paths)| {
            !sources.is_disjoint(paths) && same_statement(statement, text(&claim["statement"]))
        }) {
            diag(
                items,
                "CLAIM_INVARIANT_LOST",
                format!(
                    "{}: high priority invariant was dropped: {}",
                    text(&claim["id"]),
                    text(&claim["statement"])
                ),
            );
        }
    }
}
pub fn validate(target: &Path, task: &Path, plan: Option<&Path>) -> Vec<Diagnostic> {
    let mut items = vec![];
    if let Err(error) = validate_inner(target, task, plan, &mut items) {
        diag(&mut items, "SCAN_GLOB_UNRESOLVED", error.to_string());
    }
    items
}
fn validate_inner(
    target: &Path,
    task: &Path,
    plan: Option<&Path>,
    items: &mut Vec<Diagnostic>,
) -> Result<()> {
    let files = match direct(task) {
        Ok(files) => files,
        Err(error) => {
            diag(items, "SCAN_ARTIFACT_UNREADABLE", error.to_string());
            return Ok(());
        }
    };
    let coverage_files = files
        .iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("-coverage.json"))
        })
        .collect::<Vec<_>>();
    if coverage_files.is_empty() {
        diag(
            items,
            "SCAN_COVERAGE_MISSING",
            "no scanner coverage artifact",
        );
        return Ok(());
    }
    let surfaces = target_surfaces(target, items);
    let mut accounted = BTreeMap::<String, Vec<(String, String, usize)>>::new();
    let mut declared = vec![];
    let mut scanners = BTreeSet::new();
    let mut known = BTreeMap::new();
    let mut ledger_locators = BTreeMap::new();
    for path in coverage_files {
        let Some(document) = load_diagnostic(path, "SCAN_ARTIFACT", items) else {
            continue;
        };
        let scanner = text(&document["scanner"]).trim().to_string();
        if scanner.is_empty()
            || !exact(&document, &["scanner", "target_root", "surfaces"])
            || !document["surfaces"]
                .as_array()
                .is_some_and(|entries| !entries.is_empty())
        {
            diag(
                items,
                "SCAN_COVERAGE_INVALID",
                format!("{}: invalid coverage document", path.display()),
            );
            continue;
        }
        if !scanners.insert(scanner.clone()) {
            diag(
                items,
                "SCAN_COVERAGE_DUPLICATE",
                format!("{scanner}: duplicate coverage"),
            );
            continue;
        }
        let ledger_path = task.join(format!("{scanner}-evidence.json"));
        let ledger = if ledger_path.is_file() {
            load_diagnostic(&ledger_path, "SCAN_ARTIFACT", items).unwrap_or(Value::Null)
        } else {
            diag(
                items,
                "SCAN_LEDGER_MISSING",
                format!("{scanner}: no evidence ledger"),
            );
            Value::Null
        };
        let mut cited = BTreeMap::new();
        if !ledger.is_null() {
            let mut ids = BTreeSet::new();
            for entry in ledger["evidence"].as_array().into_iter().flatten() {
                if nonempty(&entry["id"]) {
                    let id = text(&entry["id"]).trim().to_string();
                    ids.insert(id.clone());
                    if nonempty(&entry["path"]) {
                        cited.insert(id, text(&entry["path"]).trim().to_string());
                    }
                }
            }
            known.insert(scanner.clone(), ids);
            ledger_locators.extend(locators(&ledger));
        }
        let mut scanned = vec![];
        let entries = document["surfaces"].as_array().cloned().unwrap_or_default();
        if entries.len() > 30 {
            warn(
                items,
                "SCAN_COVERAGE_VERBOSE",
                format!("{scanner}: {} surface records", entries.len()),
            );
        }
        for entry in entries {
            if !exact(
                &entry,
                &["surface", "kind", "disposition", "reason", "evidence_ids"],
            ) || !nonempty(&entry["surface"])
                || !["file", "tree"].contains(&text(&entry["kind"]))
                || !DISPOSITIONS.contains(&text(&entry["disposition"]))
                || !nonempty(&entry["reason"])
                || !string_list(&entry["evidence_ids"], true)
            {
                diag(
                    items,
                    "SCAN_COVERAGE_INVALID",
                    format!("{scanner}: invalid surface entry"),
                );
                continue;
            }
            let surface = text(&entry["surface"]).trim();
            let kind = text(&entry["kind"]);
            let disposition = text(&entry["disposition"]);
            if surface.starts_with('/') || surface.split('/').any(|part| part == "..") {
                diag(
                    items,
                    "SCAN_SURFACE_UNSAFE",
                    format!("{scanner}: surface escapes target: {surface}"),
                );
                continue;
            }
            declared.push((scanner.clone(), surface.into(), disposition.into()));
            if disposition == "covered" && secret(surface)? {
                diag(
                    items,
                    "SCAN_SECRET_COVERED",
                    format!("{scanner}: secret-bearing surface claimed covered: {surface}"),
                );
            }
            if disposition == "covered"
                && !texts(&entry["evidence_ids"]).iter().try_fold(
                    false,
                    |found, id| -> Result<bool> {
                        Ok(found
                            || match cited.get(id) {
                                Some(path) => matches(surface, kind, path)?,
                                None => false,
                            })
                    },
                )?
            {
                diag(
                    items,
                    "SCAN_COVERED_WITHOUT_EVIDENCE",
                    format!("{scanner}: {surface} covered without in-surface evidence"),
                );
            }
            if !["covered", "truncated"].contains(&disposition)
                && !texts(&entry["evidence_ids"]).is_empty()
            {
                diag(
                    items,
                    "SCAN_DISPOSITION_EVIDENCE_CONFLICT",
                    format!("{scanner}: {surface} excluded yet cited"),
                );
            }
            if disposition == "truncated" {
                warn(
                    items,
                    "SCAN_SURFACE_TRUNCATED",
                    format!("{scanner}: {surface}: {}", text(&entry["reason"])),
                );
            }
            for name in &surfaces {
                if matches(surface, kind, name)? {
                    accounted.entry(name.clone()).or_default().push((
                        scanner.clone(),
                        disposition.into(),
                        specificity(surface, name),
                    ));
                }
            }
            if ["covered", "truncated"].contains(&disposition) {
                scanned.push(entry);
            }
        }
        for (id, path) in cited {
            if !scanned
                .iter()
                .try_fold(false, |found, entry| -> Result<bool> {
                    Ok(found || matches(text(&entry["surface"]), text(&entry["kind"]), &path)?)
                })?
            {
                diag(
                    items,
                    "SCAN_EVIDENCE_OUTSIDE_COVERAGE",
                    format!(
                        "{scanner}: evidence {id} from {path} lies outside declared read surfaces"
                    ),
                );
            }
        }
    }
    for surface in surfaces {
        let Some(dispositions) = accounted.get(&surface) else {
            diag(
                items,
                "SCAN_SURFACE_UNACCOUNTED",
                format!("{surface}: no scanner disposition"),
            );
            continue;
        };
        let finest = dispositions
            .iter()
            .map(|(_, _, specificity)| *specificity)
            .max()
            .unwrap_or(0);
        let decisive = dispositions
            .iter()
            .filter(|(_, _, specificity)| *specificity == finest)
            .map(|(_, disposition, _)| disposition.as_str())
            .collect::<BTreeSet<_>>();
        if decisive.contains("covered")
            && (decisive.contains("not-permitted")
                || dispositions.iter().any(|(_, disposition, specificity)| {
                    disposition == "not-permitted" && *specificity < finest
                }))
        {
            diag(
                items,
                "SCAN_DISPOSITION_CONFLICT",
                format!("{surface}: both covered and forbidden"),
            );
        }
    }
    for path in files.iter().filter(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with("-findings.md"))
    }) {
        if fs::read_text(path).is_ok_and(|content| content.lines().count() > 80) {
            warn(
                items,
                "SCAN_REPORT_OVERLONG",
                format!("{}: report exceeds 80 lines", path.display()),
            );
        }
    }
    let claims = validate_claims(task, &known, items);
    if let Some(path) = plan {
        if path.is_file() {
            if let Some(plan) = load_diagnostic(path, "SCAN_ARTIFACT", items) {
                ownership_coverage(&declared, task, &plan, items)?;
                carry_invariants(&claims, &ledger_locators, &plan, items);
            }
        } else {
            diag(
                items,
                "CLAIMS_PLAN_UNREADABLE",
                "plan does not exist; invariants cannot be shown carried",
            );
        }
    }
    Ok(())
}
