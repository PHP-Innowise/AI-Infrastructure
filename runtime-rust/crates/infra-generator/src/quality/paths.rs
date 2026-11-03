use super::{consts::*, util::*};
use crate::common::{Diagnostic, diag};
use infra_core::Result;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
pub fn normalize_glob(value: &str) -> String {
    let mut x = value.trim().replace('\\', "/");
    while x.starts_with("./") {
        x = x[2..].to_owned()
    }
    rstrip(&replace(r"/+", &x, "/"), "/")
}
pub fn prefix(value: &str) -> String {
    let n = value.find(['*', '?', '[']).unwrap_or(value.len());
    rstrip(&value[..n], "/")
}
fn suffix(value: &str) -> Option<&str> {
    let n = value.rfind(['*', '?']).map(|x| x + 1).unwrap_or(0);
    let s = &value[n..];
    (!s.contains('[')).then_some(s)
}
pub fn wildcard(value: &str) -> bool {
    value.contains(['*', '?', '['])
}
pub fn intersects(left: &str, right: &str) -> bool {
    let a = normalize_glob(left).to_lowercase();
    let b = normalize_glob(right).to_lowercase();
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if crate::common::glob_match_case(&a, &b) || crate::common::glob_match_case(&b, &a) {
        return true;
    }
    let aw = wildcard(&a);
    let bw = wildcard(&b);
    let contains =
        (!aw && b.starts_with(&(a.clone() + "/"))) || (!bw && a.starts_with(&(b.clone() + "/")));
    let bare = |x: &str, w: bool| !w && !x.rsplit('/').next().unwrap_or("").contains('.');
    if !contains
        && !bare(&a, aw)
        && !bare(&b, bw)
        && let (Some(sa), Some(sb)) = (suffix(&a), suffix(&b))
        && !sa.ends_with(sb)
        && !sb.ends_with(sa)
    {
        return false;
    }
    let ap = prefix(&a);
    let bp = prefix(&b);
    ap.is_empty()
        || bp.is_empty()
        || ap == bp
        || ap.starts_with(&(bp.clone() + "/"))
        || bp.starts_with(&(ap + "/"))
}
pub fn collisions(left: &Value, right: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for a in ss(left) {
        for b in ss(right) {
            if intersects(&a, &b) {
                let mut v = [normalize_glob(&a), normalize_glob(&b)];
                v.sort_by(|a, b| (a.len(), a).cmp(&(b.len(), b)));
                out.insert(v[0].clone());
            }
        }
    }
    out
}
fn glob_path(pattern: &[String], path: &[String]) -> Result<bool> {
    fn walk(
        p: &[String],
        v: &[String],
        seen: &mut BTreeSet<(usize, usize)>,
        pi: usize,
        vi: usize,
    ) -> Result<bool> {
        if !seen.insert((pi, vi)) {
            return Ok(false);
        }
        if pi == p.len() {
            return Ok(vi == v.len());
        }
        if p[pi] == "**" {
            Ok(walk(p, v, seen, pi + 1, vi)? || (vi < v.len() && walk(p, v, seen, pi, vi + 1)?))
        } else {
            Ok(vi < v.len()
                && crate::common::glob_match_path(&p[pi], &v[vi], cfg!(windows))?
                && walk(p, v, seen, pi + 1, vi + 1)?)
        }
    }
    walk(pattern, path, &mut BTreeSet::new(), 0, 0)
}
pub fn path_matches(target: &Path, value: &str) -> Result<Vec<PathBuf>> {
    if !wildcard(value) {
        return Ok(confined(target, value)
            .filter(|p| p.exists())
            .into_iter()
            .collect());
    }
    let mut out = Vec::new();
    fn walk(root: &Path, p: &Path, pattern: &[String], out: &mut Vec<PathBuf>) -> Result<()> {
        let Ok(entries) = std::fs::read_dir(p) else {
            return Ok(());
        };
        for entry in entries.flatten() {
            let q = entry.path();
            let rel = relative(root, &q);
            if glob_path(pattern, &segments(&rel))?
                && let Some(c) = confined(root, &rel).filter(|c| c.exists())
            {
                out.push(c)
            }
            if entry
                .file_type()
                .is_ok_and(|t| t.is_dir() && !t.is_symlink())
            {
                walk(root, &q, pattern, out)?
            }
        }
        Ok(())
    }
    walk(target, target, &segments(value), &mut out)?;
    out.sort();
    out.dedup();
    Ok(out)
}
pub fn creatable(target: &Path, value: &str) -> bool {
    confined(target, &prefix(value)).is_some_and(|p| {
        if value.ends_with("/**") {
            p.is_dir()
        } else {
            p.parent().is_some_and(Path::is_dir)
        }
    })
}
pub fn scope(v: &Value) -> BTreeSet<String> {
    flat(v)
        .into_iter()
        .map(|x| rstrip(&x.trim().to_lowercase(), "/"))
        .collect()
}
pub fn scope_collisions(a: &BTreeSet<String>, b: &BTreeSet<String>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for x in a {
        for y in b {
            if x == y || x.starts_with(&(y.clone() + "/")) || y.starts_with(&(x.clone() + "/")) {
                out.insert(if x.len() <= y.len() {
                    x.clone()
                } else {
                    y.clone()
                });
            }
        }
    }
    out
}
pub fn expand(value: &str) -> Vec<String> {
    if let Some(c) = finds(r"\{([^{}]*)\}", value).first() {
        let mut out = Vec::new();
        for option in c.groups[1].as_deref().unwrap_or("").split(',') {
            out.extend(expand(&format!(
                "{}{}{}",
                &value[..c.start],
                option.trim(),
                &value[c.end..]
            )))
        }
        out
    } else {
        vec![value.to_owned()]
    }
}
pub fn segments(value: &str) -> Vec<String> {
    value
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .map(str::to_owned)
        .collect()
}
pub fn segment_match(a: &str, b: &str) -> bool {
    crate::common::glob_match_case(a, b) || crate::common::glob_match_case(b, a)
}
pub fn run(fragment: &[String], contract: &[String]) -> bool {
    !fragment.is_empty()
        && fragment.len() <= contract.len()
        && contract.windows(fragment.len()).any(|window| {
            fragment
                .iter()
                .zip(window)
                .all(|(a, b)| segment_match(a, b))
        })
}
pub fn beneath(fragment: &[String], contract: &[String]) -> bool {
    let c = contract
        .iter()
        .filter(|s| s.as_str() != "**")
        .collect::<Vec<_>>();
    !c.is_empty()
        && fragment.len() >= c.len()
        && fragment.iter().zip(c).all(|(a, b)| segment_match(a, b))
}
pub fn named(span: &str) -> Option<String> {
    let x = span.split(":symbol:").next().unwrap_or(span);
    let x = replace(LINE_ANCHOR_SUFFIX, x, "");
    let x = rstrip(x.trim(), ".,;:");
    if x.is_empty() {
        return None;
    }
    if !full(DOTFILE_SPAN_PATTERN, &x)
        && (!full(PATH_SPAN_PATTERN, &x) || (!x.contains('/') && !has(PATH_EXTENSION_PATTERN, &x)))
    {
        return None;
    }
    let x = rstrip(&x, "/");
    (!x.is_empty()).then_some(x)
}
pub fn covers(surface: &str, kind: &str, path: &str) -> Result<bool> {
    let x = rstrip(surface, "/");
    Ok(if let Some(p) = x.strip_suffix("/**") {
        path == p || path.starts_with(&(p.to_owned() + "/"))
    } else if wildcard(&x) {
        crate::common::glob_match(&x, path)?
    } else if kind == "tree" {
        path == x || path.starts_with(&(x + "/"))
    } else {
        path == x
    })
}
pub fn territory(skill: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for c in arr(&skill["path_contracts"]) {
        if ne(&c["path"]) {
            out.push((s(&c["path"]).trim().to_owned(), "file".into()))
        }
    }
    for c in arr(&skill["ownership"]) {
        for p in ss(&c["paths"]) {
            out.push((p, "tree".into()))
        }
    }
    out
}
pub fn relation<'a>(skill: &'a Value, name: &str) -> Option<&'a Value> {
    let v = skill
        .get("_normalized_siblings")
        .unwrap_or(&skill["nearest_siblings"]);
    arr(v).iter().find(|x| s(&x["name"]) == name)
}
pub fn reciprocal(a: &str, b: &str) -> bool {
    (a == "primary" && ["defer", "fallback"].contains(&b))
        || (b == "primary" && ["defer", "fallback"].contains(&a))
}
pub fn precedence(skill: &Value, name: &str) -> bool {
    relation(skill, name).is_some_and(|r| {
        ROUTING_ROLES.contains(&s(&r["role"]))
            || has(
                r"(?i)\b(primary|defer|deferred|fallback|owns|owner)\b",
                &flat(&r["boundary"]).join(" "),
            )
    })
}
pub fn legacy(skills: &mut SkillMap) {
    for (name, skill) in skills {
        skill["_normalized_ownership"] = json!([{"id":format!("legacy:{name}"),"mode":"exclusive","description":flat(&skill["owned_scope"]).join(" "),"paths":skill["writes"]}]);
        let siblings = arr(&skill["nearest_siblings"])
            .iter()
            .map(|r| {
                let mut r = r.clone();
                let t = flat(&r["boundary"]).join(" ");
                r["role"] = json!(if has(r"(?i)\b(primary|owns|owner|owned here)\b", &t) {
                    Some("primary")
                } else if has(r"(?i)\bfallback\b", &t) {
                    Some("fallback")
                } else if has(r"(?i)\b(defer|deferred|leave|handled by)\b", &t) {
                    Some("defer")
                } else {
                    None
                });
                r["ownership_ids"] = json!([format!("legacy:{name}")]);
                r
            })
            .collect::<Vec<_>>();
        skill["_normalized_siblings"] = json!(siblings);
    }
}
pub fn body_paths(
    name: &str,
    skill: &Value,
    body: &str,
    evidence: &EvidenceMap,
    target: &Path,
    d: &mut Vec<Diagnostic>,
) {
    let mut creatable = set(&skill["writes"]);
    for c in arr(&skill["path_contracts"]) {
        if s(&c["classification"]) != "required-existing" {
            creatable.insert(s(&c["path"]).to_owned());
        }
    }
    let mut quoted = String::new();
    let mut cited = set(&skill["source_paths"]);
    for id in ss(&skill["evidence_ids"]) {
        if let Some((kind, path)) = evidence.get(&id)
            && kind == "path"
        {
            cited.insert(path.clone());
        }
    }
    let mut suspects = BTreeSet::new();
    for block in body.split("\n\n") {
        let paragraph = block.split_whitespace().collect::<Vec<_>>().join(" ");
        for c in finds(CODE_SPAN_CONTENT, &paragraph) {
            let Some(path) = named(c.groups[1].as_deref().unwrap_or("")) else {
                continue;
            };
            let seg = segments(&path);
            if seg.len() < 2
                || path.contains(['*', '?', '[', ']', '{', '}'])
                || BODY_PATH_UNTRACKED_ROOTS.contains(&seg[0].as_str())
                || RUNTIME_ROOTS.contains(&seg[0].as_str())
                || creatable.contains(&path)
                || has(BODY_PATH_CREATION_PATTERN, &paragraph)
                || match path_matches(target, &path) {
                    Ok(matches) => !matches.is_empty(),
                    Err(error) => {
                        diag(d, "QUALITY_GLOB_UNRESOLVED", error.to_string());
                        continue;
                    }
                }
            {
                continue;
            }
            if confined(target, &seg[..seg.len() - 1].join("/")).is_some_and(|p| p.is_dir())
                && confined(target, &seg[0]).is_some_and(|p| p.is_dir())
            {
                suspects.insert(path);
            }
        }
    }
    if suspects.is_empty() {
        return;
    }
    for p in cited {
        if let Some(path) = confined(target, &p)
            && let Ok(bytes) = std::fs::read(path)
        {
            quoted.push_str(
                &String::from_utf8_lossy(&bytes)
                    .chars()
                    .take(200_000)
                    .collect::<String>(),
            );
            quoted.push('\n')
        }
    }
    for p in suspects {
        if !quoted.contains(&p) {
            diag(
                d,
                "SKILL_BODY_PATH_MISSING",
                format!("{name}: body sends the agent to a target path that does not exist: {p}"),
            )
        }
    }
}

#[cfg(test)]
#[path = "path_glob_tests.rs"]
mod path_glob_tests;
