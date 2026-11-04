use super::{consts::*, util::*};
use crate::common::{Diagnostic, diag};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};
pub fn valid_falsifier(v: &Value) -> bool {
    let Some(m) = v.as_object() else { return false };
    if !m
        .keys()
        .all(|k| ["surface", "requires", "probes", "at_least"].contains(&k.as_str()))
        || !ne(&v["surface"])
        || !["any", "all"].contains(&s(&v["requires"]))
        || arr(&v["probes"]).is_empty()
    {
        return false;
    }
    if v.get("at_least")
        .is_some_and(|x| x.as_u64().is_none_or(|n| n == 0))
    {
        return false;
    }
    arr(&v["probes"]).iter().all(|p| {
        p.as_object()
            .is_some_and(|m| m.keys().all(|k| ["paths", "pattern"].contains(&k.as_str())))
            && sl(&p["paths"], false)
            && (p.get("pattern").is_none()
                || (ne(&p["pattern"]) && super::regex_adapter::compile(s(&p["pattern"])).is_ok()))
    })
}
pub fn glob_regex(glob: &str) -> String {
    let mut out = String::from(r"\A");
    let mut chars = glob.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '*' {
            if chars.peek() == Some(&'*') {
                chars.next();
                if chars.peek() == Some(&'/') {
                    chars.next();
                    out.push_str("(?:[^/]+/)*")
                } else {
                    out.push_str(".*")
                }
            } else {
                out.push_str("[^/]*")
            }
        } else if c == '?' {
            out.push_str("[^/]")
        } else {
            out.push_str(&super::regex_adapter::escape(&c.to_string()))
        }
    }
    out.push_str(r"\z");
    out
}
pub fn index(target: &Path) -> Vec<String> {
    fn visit(root: &Path, p: &Path, out: &mut Vec<String>) {
        if out.len() >= FALSIFIER_MAX_FILES {
            return;
        }
        let Ok(entries) = std::fs::read_dir(p) else {
            return;
        };
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        for e in entries.flatten() {
            let Ok(t) = e.file_type() else { continue };
            if t.is_symlink() {
                continue;
            }
            if t.is_dir() {
                if !FALSIFIER_IGNORED_DIRECTORIES
                    .contains(&e.file_name().to_string_lossy().as_ref())
                {
                    dirs.push(e.path())
                }
            } else {
                files.push(e.path())
            }
        }
        files.sort();
        dirs.sort();
        for p in files {
            out.push(relative(root, &p));
            if out.len() >= FALSIFIER_MAX_FILES {
                return;
            }
        }
        for dir in dirs {
            visit(root, &dir, out);
            if out.len() >= FALSIFIER_MAX_FILES {
                return;
            }
        }
    }
    let mut out = Vec::new();
    visit(target, target, &mut out);
    for rel in FALSIFIER_GIT_FILES {
        let p = target.join(rel);
        if p.is_file() && !p.is_symlink() {
            out.push((*rel).to_owned())
        }
    }
    out.sort();
    out.dedup();
    out
}
pub fn fires(value: &Value, target: &Path, index: &[String]) -> std::result::Result<bool, String> {
    let mut outcomes = Vec::new();
    for probe in arr(&value["probes"]) {
        let patterns = ss(&probe["paths"])
            .iter()
            .map(|s| glob_regex(s))
            .collect::<Vec<_>>();
        let files = index
            .iter()
            .filter(|path| patterns.iter().any(|r| has(r, path)))
            .collect::<Vec<_>>();
        let mut fire = false;
        if probe.get("pattern").is_none() {
            fire = !files.is_empty()
        } else {
            for path in files.iter().take(FALSIFIER_MAX_READS) {
                let Ok(metadata) = target.join(path).metadata() else {
                    continue;
                };
                if metadata.len() > FALSIFIER_MAX_BYTES as u64 {
                    continue;
                }
                let Ok(bytes) = std::fs::read(target.join(path)) else {
                    continue;
                };
                let text = decode_ignore(&bytes);
                if super::regex_adapter::matching(s(&probe["pattern"]), &text)
                    .map_err(|e| e.to_string())?
                {
                    fire = true;
                    break;
                }
            }
        }
        outcomes.push(fire);
    }
    let count = outcomes.iter().filter(|b| **b).count();
    Ok(
        (s(&value["requires"]) != "all" || outcomes.iter().all(|b| *b))
            && count >= value["at_least"].as_u64().unwrap_or(1) as usize,
    )
}
fn slug(title: &str) -> String {
    let text = replace(r"[\x60*_]", &title.trim().to_lowercase(), "");
    let text = replace(r"[^a-z0-9 -]", &text, "");
    replace(r"[- ]+", &text, "-").trim_matches('-').to_owned()
}
pub fn load(path: &Path, catalog: &Value, d: &mut Vec<Diagnostic>) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let document = match crate::common::read_object(path) {
        Ok(v) => v,
        Err(e) => {
            diag(
                d,
                "REGISTRY_INVALID",
                format!("cannot read candidate registry: {e}"),
            );
            return out;
        }
    };
    if s(&document["schema_version"]) != REGISTRY_SCHEMA_VERSION
        || document["catalog_version"] != *catalog
    {
        diag(
            d,
            "REGISTRY_VERSION",
            "candidate registry schema/catalog version does not match plan",
        );
        return out;
    }
    if !document.as_object().is_some_and(|m| {
        ["schema_version", "catalog_version", "candidates"]
            .iter()
            .all(|k| m.contains_key(*k))
            && m.keys().all(|k| {
                [
                    "schema_version",
                    "catalog_version",
                    "candidates",
                    "runtime_contract",
                    "compilation_requirements",
                ]
                .contains(&k.as_str())
            })
    }) {
        diag(
            d,
            "REGISTRY_INVALID",
            "registry has missing or unknown fields",
        );
        return out;
    }
    let Some(candidates) = document["candidates"].as_array() else {
        diag(
            d,
            "REGISTRY_INVALID",
            "registry candidates must be an array",
        );
        return out;
    };
    for item in candidates {
        let valid = item.as_object().is_some_and(|m| {
            ["id", "catalog", "category", "mode"]
                .iter()
                .all(|k| m.contains_key(*k))
                && m.keys().all(|k| {
                    [
                        "id",
                        "catalog",
                        "category",
                        "mode",
                        "roles",
                        "escalates_on_rejection",
                        "falsifier",
                        "falsifier_absent",
                        "golden",
                    ]
                    .contains(&k.as_str())
                })
        }) && ["id", "catalog", "category"].iter().all(|k| ne(&item[*k]))
            && ["static", "runtime-fixed", "family"].contains(&s(&item["mode"]))
            && item
                .get("roles")
                .is_none_or(|v| v.is_null() || sl(v, false))
            && ["escalates_on_rejection", "golden"]
                .iter()
                .all(|k| item.get(*k).is_none_or(Value::is_boolean))
            && item
                .get("falsifier")
                .is_none_or(|v| v.is_null() || valid_falsifier(v));
        if !valid {
            diag(
                d,
                "REGISTRY_CANDIDATE_INVALID",
                format!("invalid candidate: {item}"),
            );
            continue;
        }
        let id = s(&item["id"]).to_owned();
        if out.contains_key(&id) {
            diag(
                d,
                "REGISTRY_CANDIDATE_DUPLICATE",
                format!("duplicate registry candidate: {id}"),
            );
            continue;
        }
        if item["falsifier"].is_null()
            && item.get("falsifier").is_some()
            && !ne(&item["falsifier_absent"])
        {
            diag(
                d,
                "REGISTRY_CANDIDATE_INVALID",
                format!("{id}: null falsifier requires reason"),
            );
            continue;
        }
        if !item["falsifier"].is_null() && item.get("falsifier_absent").is_some() {
            diag(
                d,
                "REGISTRY_CANDIDATE_INVALID",
                format!("{id}: falsifier and falsifier_absent conflict"),
            );
            continue;
        }
        let catalog = s(&item["catalog"]);
        let Some((file, anchor)) = catalog.split_once('#') else {
            diag(
                d,
                "REGISTRY_CATALOG_INVALID",
                format!("{id}: catalog must include heading anchor"),
            );
            out.insert(id, item.clone());
            continue;
        };
        let base = path.parent().unwrap_or(Path::new("."));
        if !base.join(file).is_file() {
            diag(
                d,
                "REGISTRY_CATALOG_INVALID",
                format!("{id}: catalog file missing or unsafe: {file}"),
            )
        } else if let Ok(text) = read_text(base.join(file)) {
            let headings = words(r"(?m)^#{1,6}\s+(.+?)\s*$", &text);
            if !headings.iter().any(|h| slug(h) == anchor) {
                diag(
                    d,
                    "REGISTRY_CATALOG_ANCHOR",
                    format!("{id}: catalog heading does not exist: {anchor}"),
                )
            }
        }
        out.insert(id, item.clone());
    }
    out
}
pub fn roles(name: &str, skill: &Value, candidate: &Value, d: &mut Vec<Diagnostic>) {
    let declared = arr(&skill["required_procedure_roles"])
        .iter()
        .filter_map(|r| r["role"].as_str())
        .map(|s| s.trim().to_lowercase())
        .collect::<std::collections::BTreeSet<_>>();
    for role in ss(&candidate["roles"]) {
        if !declared.contains(&role.trim().to_lowercase()) {
            diag(
                d,
                "CATALOG_ROLE_UNCOVERED",
                format!("{name} drops catalog role: {role}"),
            )
        }
    }
}
