use super::{consts::*, paths, prose, search, util::*};
use crate::common::{Diagnostic, diag, warn};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
#[derive(Default)]
pub struct Ledger {
    pub locations: EvidenceMap,
    pub claims: BTreeMap<String, Vec<String>>,
}
fn approved_url(value: &str) -> bool {
    let Some((scheme, tail)) = value.split_once("://") else {
        return false;
    };
    if !["http", "https"].contains(&scheme) || tail.contains('#') {
        return false;
    }
    let authority = tail.split(['/', '?']).next().unwrap_or("");
    !authority.is_empty()
        && !authority.contains('@')
        && !authority.chars().any(char::is_whitespace)
        && !authority.starts_with(':')
}
pub fn validate(plan: &Value, target: &Path, schema: &str, d: &mut Vec<Diagnostic>) -> Ledger {
    let mut ledger = Ledger::default();
    if arr(&plan["evidence"]).is_empty() {
        diag(d, "EVIDENCE_EMPTY", "evidence must be a non-empty array");
        return ledger;
    }
    let mut seen = BTreeSet::new();
    for entry in arr(&plan["evidence"]) {
        if !entry.is_object() || !ne(&entry["id"]) {
            diag(
                d,
                "EVIDENCE_INVALID",
                "every evidence item must define a nonempty id",
            );
            continue;
        }
        let id = s(&entry["id"]).trim().to_owned();
        if !seen.insert(id.clone()) {
            diag(
                d,
                "EVIDENCE_ID_DUPLICATE",
                format!("duplicate evidence id: {id}"),
            );
            continue;
        }
        let mut fields = vec![
            "id",
            "path",
            "url",
            "source_type",
            "authority",
            "confidence",
            "line_range",
            "fingerprint",
            "supported_claims",
        ];
        if wired(schema) {
            fields.push("absence")
        }
        for key in entry.as_object().unwrap().keys() {
            if !fields.contains(&key.as_str()) {
                diag(
                    d,
                    "EVIDENCE_FIELD_UNKNOWN",
                    format!("evidence {id} has unknown field: {key}"),
                )
            }
        }
        for key in ["source_type", "authority", "confidence"] {
            if !ne(&entry[key]) {
                diag(d, "EVIDENCE_METADATA", format!("evidence {id} lacks {key}"))
            }
        }
        if !["confirmed", "inferred", "unknown"].contains(&s(&entry["confidence"])) {
            diag(
                d,
                "EVIDENCE_CONFIDENCE",
                format!("evidence {id} confidence must be confirmed, inferred or unknown"),
            )
        }
        let claims = ss(&entry["supported_claims"]);
        if !sl(&entry["supported_claims"], false) {
            diag(
                d,
                "EVIDENCE_CLAIMS",
                format!("evidence {id} must declare supported_claims"),
            )
        }
        ledger.claims.insert(id.clone(), claims.clone());
        let locations = ["path", "url", "source"]
            .iter()
            .filter_map(|k| {
                entry[*k]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .map(|v| ((*k).to_owned(), v.trim().to_owned()))
            })
            .collect::<Vec<_>>();
        if entry.get("absence").is_some() {
            if !wired(schema) || !locations.is_empty() {
                diag(
                    d,
                    "EVIDENCE_LOCATION",
                    format!("evidence {id} must define exactly one location or absence"),
                );
                continue;
            }
            if search::absence(&id, &entry["absence"], target, d) {
                let subject = s(&entry["absence"]["subject"]).to_owned();
                let vocab = prose::vocabulary(s(&entry["absence"]["search"]));
                for claim in &claims {
                    let tokens = prose::meaningful(claim)
                        .difference(&CLAIM_SERVICE_WORDS.iter().map(|x| x.to_string()).collect())
                        .cloned()
                        .collect::<BTreeSet<_>>();
                    if tokens.is_disjoint(&vocab) {
                        diag(
                            d,
                            "EVIDENCE_CLAIM_UNSUPPORTED",
                            format!(
                                "evidence {id} absence claim has no searched vocabulary: {claim}"
                            ),
                        )
                    }
                }
                ledger.locations.insert(id, ("absence".into(), subject));
            }
            continue;
        }
        if locations.len() != 1 {
            diag(
                d,
                "EVIDENCE_LOCATION",
                format!("evidence {id} must identify exactly one path or approved URL"),
            );
            continue;
        }
        let (mut kind, value) = locations[0].clone();
        if kind == "source" {
            kind = if value.starts_with("http://") || value.starts_with("https://") {
                "url"
            } else {
                "path"
            }
            .into()
        }
        if kind == "url" {
            if !approved_url(&value) {
                diag(
                    d,
                    "EVIDENCE_URL_INVALID",
                    format!("evidence {id} URL is not approved"),
                );
                continue;
            }
            ledger.locations.insert(id, (kind, value));
            continue;
        }
        let Some(path) = confined(target, &value).filter(|p| p.is_file()) else {
            diag(
                d,
                "EVIDENCE_PATH_INVALID",
                format!("evidence {id} path is unsafe or missing: {value}"),
            );
            continue;
        };
        if has(r"(?i)(^|/)tasks/TASK-[^/]+/", &value) {
            diag(
                d,
                "EVIDENCE_TASK_PATH",
                format!("evidence {id} cites generator runtime: {value}"),
            )
        }
        let bytes = match std::fs::read(path) {
            Ok(v) => v,
            Err(e) => {
                diag(
                    d,
                    "EVIDENCE_PATH_INVALID",
                    format!("evidence {id} is unreadable: {e}"),
                );
                continue;
            }
        };
        let hash = format!("sha256:{}", crate::common::sha(&bytes));
        if !ne(&entry["fingerprint"]) {
            diag(
                d,
                "EVIDENCE_FINGERPRINT_MISSING",
                format!("evidence {id} requires sha256 fingerprint"),
            )
        } else if s(&entry["fingerprint"]) != hash {
            diag(
                d,
                "EVIDENCE_FINGERPRINT",
                format!("evidence {id} fingerprint differs from target bytes: {value}"),
            )
        }
        let text = decode_lossy(&bytes);
        let lines = py_lines(&text);
        let mut cited = text.to_string();
        if let Some(range) = entry.get("line_range") {
            let start = range["start"].as_u64();
            let end = range["end"].as_u64();
            if !exact(range, &["start", "end"])
                || start.is_none_or(|n| n == 0)
                || end.is_none_or(|n| n < start.unwrap_or(1) || n > lines.len() as u64)
            {
                diag(
                    d,
                    "EVIDENCE_LINE_RANGE",
                    format!("evidence {id} line_range is outside {} lines", lines.len()),
                )
            } else {
                cited = lines[(start.unwrap() - 1) as usize..end.unwrap() as usize].join("\n")
            }
        }
        let vocabulary = prose::vocabulary(&cited);
        for claim in claims {
            let tokens = prose::meaningful(&claim)
                .difference(&CLAIM_SERVICE_WORDS.iter().map(|s| s.to_string()).collect())
                .cloned()
                .collect::<BTreeSet<_>>();
            let overlap = tokens.intersection(&vocabulary).count();
            let affordable = (vocabulary.len() / CLAIM_SENTENCE_TOKENS).max(1);
            let required = if tokens.len() <= 3 { 1 } else { 2 }.min(affordable);
            if overlap < required {
                diag(
                    d,
                    "EVIDENCE_CLAIM_UNSUPPORTED",
                    format!("evidence {id} claim is not grounded in cited vocabulary: {claim}"),
                );
                continue;
            }
            let language = CLAIM_LANGUAGE_LEXICON
                .iter()
                .map(|s| s.to_string())
                .collect::<BTreeSet<_>>();
            let common = CLAIM_COMMON_LEXICON
                .iter()
                .map(|s| s.to_string())
                .collect::<BTreeSet<_>>();
            let specific = tokens
                .difference(&language)
                .cloned()
                .collect::<BTreeSet<_>>();
            if specific.is_disjoint(&vocabulary) {
                diag(
                    d,
                    "EVIDENCE_CLAIM_UNSUPPORTED",
                    format!(
                        "evidence {id} claim shares language words but no project vocabulary: {claim}"
                    ),
                )
            } else {
                let particular = tokens.difference(&common).cloned().collect::<BTreeSet<_>>();
                if particular.is_disjoint(&vocabulary)
                    && particular.len() * 100 <= tokens.len() * CLAIM_GENERIC_SHARE_PERCENT
                {
                    warn(
                        d,
                        "EVIDENCE_CLAIM_GENERIC_SUPPORT",
                        format!(
                            "evidence {id} claim is grounded only in common vocabulary: {claim}"
                        ),
                    )
                }
            }
        }
        ledger.locations.insert(id, (kind, value));
    }
    ledger
}
pub fn resolve_anchor(
    name: &str,
    anchor: &str,
    path: &str,
    target: &Path,
    d: &mut Vec<Diagnostic>,
) {
    let Some(resolved) = confined(target, path).filter(|p| p.is_file()) else {
        diag(
            d,
            "EVIDENCE_ANCHOR_UNRESOLVABLE",
            format!("{name} anchor cites missing or unsafe source: {anchor}"),
        );
        return;
    };
    let Ok(bytes) = std::fs::read(resolved) else {
        diag(
            d,
            "EVIDENCE_ANCHOR_UNRESOLVABLE",
            format!("{name} anchor source cannot be read: {anchor}"),
        );
        return;
    };
    let text = decode_lossy(&bytes);
    let body = &anchor[path.len() + 1..];
    if let Some(symbol) = body.strip_prefix("symbol:") {
        let identifier = symbol
            .split(['.', ':', '\\', '-'])
            .rfind(|s| !s.is_empty())
            .unwrap_or(symbol);
        if !has(
            &format!(
                r"(?i)(?<![A-Za-z0-9_]){}(?![A-Za-z0-9_])",
                super::regex_adapter::escape(identifier)
            ),
            &text,
        ) {
            diag(
                d,
                "EVIDENCE_ANCHOR_SYMBOL_ABSENT",
                format!("{name} anchor symbol absent from cited source: {anchor}"),
            )
        }
        return;
    }
    let bounds = body
        .split('-')
        .filter_map(|s| s.strip_prefix('L').unwrap_or(s).parse::<usize>().ok())
        .collect::<Vec<_>>();
    if bounds.is_empty() {
        return;
    }
    let start = bounds[0];
    let end = *bounds.last().unwrap();
    let count = py_lines(&text).len();
    if end < start || start > count || end > count {
        diag(
            d,
            "EVIDENCE_ANCHOR_RANGE",
            format!("{name} anchor outside cited source ({count} lines): {anchor}"),
        )
    }
}
pub fn dispositions(
    name: &str,
    skill: &Value,
    locations: &EvidenceMap,
    runtime: bool,
    d: &mut Vec<Diagnostic>,
) {
    if !sl(&skill["claim_ids"], runtime) {
        diag(
            d,
            "SKILL_CLAIM_IDS_INVALID",
            format!("{name}.claim_ids must name reconciled claims"),
        )
    }
    if !skill["evidence_dispositions"].is_array() {
        diag(
            d,
            "EVIDENCE_DISPOSITION_INVALID",
            format!("{name}.evidence_dispositions must be an array"),
        );
        return;
    }
    let cited = set(&skill["evidence_ids"]);
    let mut decided = BTreeSet::new();
    for e in arr(&skill["evidence_dispositions"]) {
        if !exact(e, &["evidence_id", "disposition", "reason"])
            || !ne(&e["evidence_id"])
            || s(&e["disposition"]) != "excluded"
            || !ne(&e["reason"])
        {
            diag(
                d,
                "EVIDENCE_DISPOSITION_INVALID",
                format!("{name}: exclusion must name evidence id and reason"),
            );
            continue;
        }
        let id = s(&e["evidence_id"]);
        if !locations.contains_key(id) {
            diag(
                d,
                "EVIDENCE_DISPOSITION_UNKNOWN",
                format!("{name} excludes unknown evidence: {id}"),
            )
        } else if cited.contains(id) {
            diag(
                d,
                "EVIDENCE_DISPOSITION_CONTRADICTED",
                format!("{name} both cites and excludes {id}"),
            )
        } else {
            decided.insert(id.to_owned());
        }
    }
    let surfaces = paths::territory(skill);
    let mut undisposed = Vec::new();
    for (id, (kind, path)) in locations {
        if kind != "path" || cited.contains(id) || decided.contains(id) {
            continue;
        }
        let covered = surfaces.iter().try_fold(
            false,
            |found, (surface, kind)| -> infra_core::Result<bool> {
                Ok(found || paths::covers(surface, kind, path)?)
            },
        );
        match covered {
            Ok(true) => undisposed.push(id.clone()),
            Ok(false) => {}
            Err(error) => {
                diag(d, "QUALITY_GLOB_UNRESOLVED", error.to_string());
                return;
            }
        }
        if undisposed.len() == 6 {
            break;
        }
    }
    if !undisposed.is_empty() {
        diag(
            d,
            "SKILL_EVIDENCE_UNDISPOSED",
            format!(
                "{name} declares paths with evidence it neither cites nor excludes: {}",
                undisposed.join(", ")
            ),
        )
    }
}
