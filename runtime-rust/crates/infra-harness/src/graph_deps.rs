//! Pure static source dependency extraction, declaration diffing, and reference matching.
use crate::graph_live::{
    CreatedEntity, ExtractedDecl, Language, clean_rel_key, detect_language,
    extract_declaration_spans, folder_of, is_ident_char, is_ident_start, sanitize_c_like_line,
    strip_comments_keep_strings, strip_leading_words, take_ident,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const PHP_EXTS: &[&str] = &["php", "phtml", "inc"];
const RUST_EXTS: &[&str] = &["rs"];
const TS_JS_EXTS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"];
const PYTHON_EXTS: &[&str] = &["py", "pyi"];

fn is_false(v: &bool) -> bool {
    !*v
}

/// Maps a normalized visibility label to an ordinal exposure rank (`4` = public/export, `0` = private).
pub(crate) fn visibility_rank(vis: Option<&str>) -> u8 {
    let Some(raw) = vis.map(str::trim).filter(|s| !s.is_empty()) else {
        return 4;
    };
    let lower = raw.to_ascii_lowercase();
    if lower.starts_with("pub(in ") {
        return 2;
    }
    match lower.as_str() {
        "public" | "pub" | "export" | "public(set)" => 4,
        "crate"
        | "pub(crate)"
        | "pub(super)"
        | "internal"
        | "protected(set)"
        | "public protected(set)"
        | "package" => 2,
        "protected"
        | "private(set)"
        | "public private(set)"
        | "protected private(set)"
        | "protected protected(set)" => 1,
        "private" | "#private" | "pub(self)" => 0,
        _ if lower.contains("private(set)") => 1,
        _ if lower.contains("protected(set)") => 2,
        _ if lower.contains("private") => 0,
        _ if lower.contains("protected") => 1,
        _ if lower.contains("crate") || lower.contains("internal") || lower.contains("package") => {
            2
        }
        _ => 4,
    }
}

/// Returns `true` when `after` visibility is strictly narrower than `before` visibility.
pub(crate) fn is_visibility_narrowed(before: Option<&str>, after: Option<&str>) -> bool {
    visibility_rank(after) < visibility_rank(before)
}

/// Symbol-level declaration delta between two revisions of a source file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EntityChange {
    pub id: String,
    pub kind: String,
    pub change: String, // "created" | "modified" | "deleted"
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub signature_changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_visibility: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub visibility_changed: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub visibility_narrowed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<String>,
}

/// Member-level usage reference recorded on a dependency edge (`calls`, `reads`, `writes`, `const`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DepMemberRef {
    pub name: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub access: String,
    /// The first line the member is reached in this way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// Every line it is reached in this way, in their order, where there is
    /// more than one: a member read in two methods is one reference told
    /// with both places.
    #[serde(default, skip_serializing_if = "at_most_one")]
    pub lines: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_risk: Option<String>,
}

fn at_most_one(lines: &[u32]) -> bool {
    lines.len() <= 1
}

impl DepMemberRef {
    fn basic(
        name: impl Into<String>,
        kind: &str,
        parent: Option<String>,
        access: &str,
        line: u32,
    ) -> Self {
        Self {
            name: name.into(),
            kind: kind.to_owned(),
            parent,
            access: access.to_owned(),
            line: Some(line),
            lines: vec![line],
            visibility: None,
            previous_visibility: None,
            change: None,
            modifiers: Vec::new(),
            access_risk: None,
        }
    }
}

/// Declared symbol/member summary extracted from a target file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DeclaredMember {
    pub name: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<String>,
    pub change: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub signature_changed: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub visibility_narrowed: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub used_by_caller: bool,
}

/// Extracted dependency reference from one source file to another file or external module.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DepRef {
    pub target: String,
    pub resolved: bool,
    #[serde(default)]
    pub exists: bool,
    /// `imports`, `calls`, `extends`, `implements`, `writes`, `reads` or `uses`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub member_refs: Vec<DepMemberRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_members: Vec<DeclaredMember>,
}

/// Before-vs-after dependency diff for a single file, grouped by canonical `(target, kind)`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DepDiff {
    pub added: Vec<DepRef>,
    pub removed: Vec<DepRef>,
    pub unchanged: Vec<DepRef>,
}

/// Extracts all declarations from a source file into `DeclaredMember` summaries.
pub(crate) fn extract_declared_members(rel_path: &str, source: &str) -> Vec<DeclaredMember> {
    let rel = clean_rel_key(rel_path);
    if rel.is_empty() || source.trim().is_empty() {
        return Vec::new();
    }
    extract_declaration_spans(&rel, source)
        .into_iter()
        .map(|decl| {
            let parent = decl.entity.parent.filter(|p| !p.is_empty() && *p != rel);
            let signature = if decl.signature.is_empty() {
                None
            } else {
                Some(decl.signature)
            };
            DeclaredMember {
                name: decl.entity.name,
                kind: decl.entity.kind,
                parent,
                line: decl.entity.line,
                signature,
                visibility: decl.visibility,
                previous_visibility: None,
                modifiers: decl.modifiers,
                change: "unchanged".to_owned(),
                signature_changed: false,
                visibility_narrowed: false,
                used_by_caller: false,
            }
        })
        .collect()
}

/// Computes created, modified, and deleted declarations between `before` and `after`.
pub(crate) fn diff_entities(
    rel_path: &str,
    before: Option<&str>,
    after: Option<&str>,
) -> Vec<EntityChange> {
    let rel = clean_rel_key(rel_path);
    if rel.is_empty() || detect_language(&rel).is_none() {
        return Vec::new();
    }

    let before_decls = before
        .map(|src| extract_declaration_spans(&rel, src))
        .unwrap_or_default();
    let after_decls = after
        .map(|src| extract_declaration_spans(&rel, src))
        .unwrap_or_default();

    type DeclKey = (String, Option<String>, String);
    let mut before_by_key: BTreeMap<DeclKey, Vec<ExtractedDecl>> = BTreeMap::new();
    for decl in before_decls {
        let key = (
            decl.entity.kind.clone(),
            decl.entity.parent.clone(),
            decl.entity.name.clone(),
        );
        before_by_key.entry(key).or_default().push(decl);
    }

    let mut used_before: BTreeMap<DeclKey, Vec<bool>> = before_by_key
        .iter()
        .map(|(k, v)| (k.clone(), vec![false; v.len()]))
        .collect();
    let mut unmatched_after: Vec<(DeclKey, ExtractedDecl)> = Vec::new();

    // Match exact unchanged declarations first so overload insertions/reorders do not produce noise.
    for decl in after_decls {
        let key = (
            decl.entity.kind.clone(),
            decl.entity.parent.clone(),
            decl.entity.name.clone(),
        );
        if let Some(prev_list) = before_by_key.get(&key)
            && let Some(used) = used_before.get_mut(&key)
            && let Some(pos) = prev_list.iter().enumerate().position(|(idx, prev)| {
                !used[idx]
                    && prev.params_signature == decl.params_signature
                    && prev.body_fingerprint == decl.body_fingerprint
                    && prev.visibility == decl.visibility
                    && prev.modifiers == decl.modifiers
            })
        {
            used[pos] = true;
        } else {
            unmatched_after.push((key, decl));
        }
    }

    let mut out = Vec::new();
    let mut seen_ids = BTreeSet::new();

    for (key, decl) in unmatched_after {
        if let Some(prev_list) = before_by_key.get(&key)
            && let Some(used) = used_before.get_mut(&key)
            && let Some(pos) = used.iter().position(|flag| !*flag)
        {
            used[pos] = true;
            let prev = &prev_list[pos];
            let sig_changed = prev.params_signature != decl.params_signature;
            let vis_changed = prev.visibility != decl.visibility;
            let vis_narrowed =
                is_visibility_narrowed(prev.visibility.as_deref(), decl.visibility.as_deref());
            out.push(make_entity_change(
                "modified",
                &decl.entity,
                sig_changed,
                decl.visibility.clone(),
                prev.visibility.clone(),
                vis_changed,
                vis_narrowed,
                decl.modifiers.clone(),
                &mut seen_ids,
            ));
        } else {
            out.push(make_entity_change(
                "created",
                &decl.entity,
                false,
                decl.visibility.clone(),
                None,
                false,
                false,
                decl.modifiers.clone(),
                &mut seen_ids,
            ));
        }
    }

    for (key, prev_list) in before_by_key {
        let Some(used) = used_before.get(&key) else {
            continue;
        };
        for (idx, prev) in prev_list.into_iter().enumerate() {
            if !used[idx] {
                out.push(make_entity_change(
                    "deleted",
                    &prev.entity,
                    false,
                    prev.visibility,
                    None,
                    false,
                    false,
                    prev.modifiers,
                    &mut seen_ids,
                ));
            }
        }
    }

    out.sort_by(|a, b| {
        a.line
            .unwrap_or(u32::MAX)
            .cmp(&b.line.unwrap_or(u32::MAX))
            .then_with(|| change_rank(&a.change).cmp(&change_rank(&b.change)))
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

fn change_rank(change: &str) -> u8 {
    match change {
        "created" => 0,
        "modified" => 1,
        "deleted" => 2,
        _ => 3,
    }
}

#[allow(clippy::too_many_arguments)]
fn make_entity_change(
    change: &str,
    entity: &CreatedEntity,
    signature_changed: bool,
    visibility: Option<String>,
    previous_visibility: Option<String>,
    visibility_changed: bool,
    visibility_narrowed: bool,
    modifiers: Vec<String>,
    seen_ids: &mut BTreeSet<String>,
) -> EntityChange {
    let base_id = if matches!(
        entity.kind.as_str(),
        "method" | "property" | "field" | "const"
    ) && let Some(owner) = entity
        .parent
        .as_deref()
        .filter(|s| !s.is_empty() && *s != entity.path)
    {
        format!(
            "decl:{}:{}:{owner}::{}",
            entity.kind, entity.path, entity.name
        )
    } else {
        format!("decl:{}:{}:{}", entity.kind, entity.path, entity.name)
    };
    let id = if seen_ids.insert(base_id.clone()) {
        base_id
    } else {
        let disambiguated = format!("{base_id}@{}", entity.line.unwrap_or(0));
        seen_ids.insert(disambiguated.clone());
        disambiguated
    };
    EntityChange {
        id,
        kind: entity.kind.clone(),
        change: change.to_owned(),
        name: entity.name.clone(),
        parent: entity.parent.clone(),
        path: entity.path.clone(),
        line: entity.line,
        signature_changed,
        visibility,
        previous_visibility,
        visibility_changed,
        visibility_narrowed,
        modifiers,
    }
}

/// Extracts imports, inheritance, includes, and symbol references from `source`, resolving against `known_files`.
pub(crate) fn extract_file_deps(
    rel_path: &str,
    source: &str,
    known_files: &BTreeSet<String>,
) -> Vec<DepRef> {
    let rel = clean_rel_key(rel_path);
    if rel.is_empty() || source.trim().is_empty() {
        return Vec::new();
    }
    let Some(lang) = detect_language(&rel) else {
        return Vec::new();
    };
    let lookup = KnownLookup::new(known_files);
    let mut acc = DepAccumulator::new(&rel, known_files);

    match lang {
        Language::Php => extract_php_deps(&rel, source, &lookup, &mut acc),
        Language::Rust => extract_rust_deps(&rel, source, &lookup, &mut acc),
        Language::TsJs => extract_ts_js_deps(&rel, source, &lookup, &mut acc),
        Language::Python => extract_python_deps(&rel, source, &lookup, &mut acc),
    }

    acc.into_sorted_vec()
}

/// Compares dependencies before and after an edit by canonical `(target, kind)`.
pub(crate) fn diff_file_deps(
    rel_path: &str,
    before: Option<&str>,
    after: Option<&str>,
    known_files: &BTreeSet<String>,
) -> DepDiff {
    let before_deps = before
        .map(|src| extract_file_deps(rel_path, src, known_files))
        .unwrap_or_default();
    let after_deps = after
        .map(|src| extract_file_deps(rel_path, src, known_files))
        .unwrap_or_default();

    let mut before_map: BTreeMap<(String, String), DepRef> = BTreeMap::new();
    for dep in before_deps {
        merge_dep_into_map(&mut before_map, dep);
    }
    let mut after_map: BTreeMap<(String, String), DepRef> = BTreeMap::new();
    for dep in after_deps {
        merge_dep_into_map(&mut after_map, dep);
    }

    let mut diff = DepDiff::default();
    for (key, after_dep) in &after_map {
        if let Some(before_dep) = before_map.get(key) {
            let mut merged = after_dep.clone();
            for sym in &before_dep.symbols {
                if !merged.symbols.contains(sym) {
                    merged.symbols.push(sym.clone());
                }
            }
            if merged.line.is_none() {
                merged.line = before_dep.line;
            }
            if merged.evidence.is_none() {
                merged.evidence = before_dep.evidence.clone();
            }
            diff.unchanged.push(merged);
        } else {
            diff.added.push(after_dep.clone());
        }
    }
    for (key, before_dep) in &before_map {
        if !after_map.contains_key(key) {
            diff.removed.push(before_dep.clone());
        }
    }

    sort_dep_refs(&mut diff.added);
    sort_dep_refs(&mut diff.removed);
    sort_dep_refs(&mut diff.unchanged);
    diff
}

/// Finds which `target_symbols` (or file/module stem) `caller_source` references in `target_path`.
pub(crate) fn find_matching_references(
    caller_path: &str,
    caller_source: &str,
    target_path: &str,
    target_symbols: &[String],
    known_files: &BTreeSet<String>,
) -> Vec<String> {
    let caller = clean_rel_key(caller_path);
    let target = clean_rel_key(target_path);
    if caller.is_empty() || target.is_empty() || caller == target || caller_source.trim().is_empty()
    {
        return Vec::new();
    }

    let mut augmented_known = known_files.clone();
    if !known_files.contains(&target) {
        augmented_known.insert(target.clone());
    }
    let deps = extract_file_deps(&caller, caller_source, &augmented_known);
    let mut matched_deps = Vec::new();
    for dep in &deps {
        if dep.resolved && clean_rel_key(&dep.target) == target {
            matched_deps.push(dep);
        }
    }

    let target_stem = file_stem(&target);
    let sanitized_caller = sanitize_whole_source(&caller, caller_source);
    let caller_idents = collect_identifiers(&sanitized_caller);

    let same_folder = folder_of(&caller) == folder_of(&target);
    let stem_referenced =
        !target_stem.is_empty() && target_stem.len() >= 2 && caller_idents.contains(target_stem);
    let has_target_link = !matched_deps.is_empty() || (same_folder && stem_referenced);
    if !has_target_link && !same_folder {
        return Vec::new();
    }

    let mut out = Vec::new();

    if target_symbols.is_empty() {
        for dep in &matched_deps {
            for sym in &dep.symbols {
                push_unique_str(&mut out, sym);
            }
        }
        if out.is_empty() && has_target_link && !target_stem.is_empty() {
            push_unique_str(&mut out, target_stem);
        }
        return out;
    }

    for sym in target_symbols {
        let trimmed = sym.trim();
        if trimmed.is_empty() {
            continue;
        }
        let dep_has_sym = matched_deps
            .iter()
            .any(|dep| dep.symbols.iter().any(|s| s == trimmed));
        if dep_has_sym || (has_target_link && caller_idents.contains(trimmed)) {
            push_unique_str(&mut out, trimmed);
        }
    }

    if out.is_empty() && !matched_deps.is_empty() {
        for dep in &matched_deps {
            for sym in &dep.symbols {
                push_unique_str(&mut out, sym);
            }
        }
        if out.is_empty() && !target_stem.is_empty() {
            push_unique_str(&mut out, target_stem);
        }
    }
    out
}

fn norm_member_name(name: &str) -> &str {
    if name.starts_with('$') && name.len() > 1 {
        &name[1..]
    } else {
        name
    }
}

fn compute_member_access_risk(tm: &DeclaredMember, access: &str) -> Option<String> {
    if tm.visibility_narrowed {
        return Some("narrowed".to_owned());
    }
    let vis = tm.visibility.as_deref().map(str::to_ascii_lowercase);
    match vis.as_deref() {
        Some("private" | "#private" | "pub(self)") => Some("private".to_owned()),
        Some("private(set)" | "public private(set)" | "protected private(set)") => {
            if access == "write" {
                Some("private_set".to_owned())
            } else {
                None
            }
        }
        Some("protected(set)" | "public protected(set)") => {
            if access == "write" {
                Some("protected".to_owned())
            } else {
                None
            }
        }
        Some("protected") => Some("protected".to_owned()),
        _ => {
            if access == "write" && tm.modifiers.iter().any(|m| m == "readonly") {
                Some("readonly".to_owned())
            } else {
                None
            }
        }
    }
}

fn push_unique_member_ref(out: &mut Vec<DepMemberRef>, candidate: DepMemberRef) {
    if let Some(existing) = out.iter_mut().find(|m| {
        m.name == candidate.name
            && m.kind == candidate.kind
            && m.parent == candidate.parent
            && m.access == candidate.access
    }) {
        if existing
            .line
            .is_none_or(|prev| candidate.line.is_some_and(|l| l < prev))
        {
            existing.line = candidate.line;
        }
        for line in candidate.lines {
            if let Err(at) = existing.lines.binary_search(&line) {
                existing.lines.insert(at, line);
            }
        }
        if existing.visibility.is_none() {
            existing.visibility = candidate.visibility;
        }
        if existing.previous_visibility.is_none() {
            existing.previous_visibility = candidate.previous_visibility;
        }
        if existing.change.is_none() {
            existing.change = candidate.change;
        }
        if existing.access_risk.is_none() {
            existing.access_risk = candidate.access_risk;
        }
        for m in candidate.modifiers {
            if !existing.modifiers.contains(&m) {
                existing.modifiers.push(m);
            }
        }
        return;
    }
    out.push(candidate);
}

fn sort_member_refs(refs: &mut [DepMemberRef]) {
    refs.sort_by(|a, b| {
        a.line
            .unwrap_or(u32::MAX)
            .cmp(&b.line.unwrap_or(u32::MAX))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.access.cmp(&b.access))
    });
}

/// Finds member-level references from `caller_source` to `target_path` matched against `target_members`.
pub(crate) fn find_matching_member_references(
    caller_path: &str,
    caller_source: &str,
    target_path: &str,
    target_members: &[DeclaredMember],
    known_files: &BTreeSet<String>,
) -> Vec<DepMemberRef> {
    let caller = clean_rel_key(caller_path);
    let target = clean_rel_key(target_path);
    if caller.is_empty() || target.is_empty() || caller == target || caller_source.trim().is_empty()
    {
        return Vec::new();
    }

    let mut augmented_known = known_files.clone();
    if !known_files.contains(&target) {
        augmented_known.insert(target.clone());
    }
    let deps = extract_file_deps(&caller, caller_source, &augmented_known);
    let mut matched_deps = Vec::new();
    for dep in &deps {
        if dep.resolved && clean_rel_key(&dep.target) == target {
            matched_deps.push(dep);
        }
    }

    let target_stem = file_stem(&target);
    let sanitized_caller = sanitize_whole_source(&caller, caller_source);
    let caller_idents = collect_identifiers(&sanitized_caller);
    let same_folder = folder_of(&caller) == folder_of(&target);
    let stem_referenced =
        !target_stem.is_empty() && target_stem.len() >= 2 && caller_idents.contains(target_stem);
    let has_target_link = !matched_deps.is_empty() || (same_folder && stem_referenced);
    if !has_target_link && !same_folder {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut raw_refs: Vec<DepMemberRef> = Vec::new();
    for dep in &matched_deps {
        for r in &dep.member_refs {
            push_unique_member_ref(&mut raw_refs, r.clone());
        }
    }

    if target_members.is_empty() {
        sort_member_refs(&mut raw_refs);
        return raw_refs;
    }

    let mut matched_tm_indices = BTreeSet::new();
    for r in raw_refs {
        let r_norm = norm_member_name(&r.name);
        let best = target_members
            .iter()
            .enumerate()
            .filter_map(|(idx, tm)| {
                let tm_norm = norm_member_name(&tm.name);
                if tm.name != r.name && tm_norm != r_norm {
                    return None;
                }
                if let (Some(rp), Some(tp)) = (r.parent.as_deref(), tm.parent.as_deref())
                    && rp != tp
                {
                    return None;
                }
                let mut score = 0u8;
                if tm.name == r.name {
                    score += 4;
                }
                if r.parent.is_some() && tm.parent == r.parent {
                    score += 4;
                }
                if tm.kind == r.kind {
                    score += 2;
                }
                Some((score, idx, tm))
            })
            .max_by_key(|(score, _, _)| *score);

        if let Some((_, idx, tm)) = best {
            matched_tm_indices.insert(idx);
            let risk = compute_member_access_risk(tm, &r.access);
            push_unique_member_ref(
                &mut out,
                DepMemberRef {
                    name: tm.name.clone(),
                    kind: tm.kind.clone(),
                    parent: tm.parent.clone().or(r.parent),
                    access: r.access,
                    line: r.line,
                    lines: r.lines,
                    visibility: tm.visibility.clone(),
                    previous_visibility: tm.previous_visibility.clone(),
                    change: Some(tm.change.clone()),
                    modifiers: tm.modifiers.clone(),
                    access_risk: risk,
                },
            );
        }
    }

    if has_target_link {
        let caller_lines: Vec<&str> = sanitized_caller.lines().collect();
        for (idx, tm) in target_members.iter().enumerate() {
            if matched_tm_indices.contains(&idx) {
                continue;
            }
            let bare = norm_member_name(&tm.name);
            if bare.len() < 2 {
                continue;
            }
            let owner_ok = match tm.parent.as_deref() {
                None => {
                    matched_deps
                        .iter()
                        .any(|d| d.symbols.iter().any(|s| s == &tm.name || s == bare))
                        || (has_target_link && caller_idents.contains(&tm.name))
                }
                Some(owner) => {
                    matched_deps
                        .iter()
                        .any(|d| d.symbols.iter().any(|s| s == owner))
                        || (same_folder && caller_idents.contains(owner))
                }
            };
            if !owner_ok {
                continue;
            }

            let mut matched_line: Option<(u32, &'static str)> = None;
            for (li, line) in caller_lines.iter().enumerate() {
                if !line.contains(bare) {
                    continue;
                }
                if let Some(access) =
                    find_member_syntax_on_line(line, &tm.name, bare, &tm.kind, tm.parent.is_none())
                {
                    matched_line = Some(((li + 1) as u32, access));
                    break;
                }
            }
            if let Some((line_no, access)) = matched_line {
                let risk = compute_member_access_risk(tm, access);
                push_unique_member_ref(
                    &mut out,
                    DepMemberRef {
                        name: tm.name.clone(),
                        kind: tm.kind.clone(),
                        parent: tm.parent.clone(),
                        access: access.to_owned(),
                        line: Some(line_no),
                        lines: vec![line_no],
                        visibility: tm.visibility.clone(),
                        previous_visibility: tm.previous_visibility.clone(),
                        change: Some(tm.change.clone()),
                        modifiers: tm.modifiers.clone(),
                        access_risk: risk,
                    },
                );
            }
        }
    }

    sort_member_refs(&mut out);
    out
}

fn find_member_syntax_on_line(
    line: &str,
    raw_name: &str,
    bare: &str,
    kind: &str,
    is_top_level: bool,
) -> Option<&'static str> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    for needle in [raw_name, bare] {
        let mut from = 0usize;
        while let Some(rel) = line[from..].find(needle) {
            let pos = from + rel;
            let end = pos + needle.len();
            from = pos + 1;
            let right_ok = end == len || !is_ident_char(bytes[end] as char);
            if !right_ok {
                continue;
            }
            let prefix = line[..pos].trim_end();
            let rest = line[end..].trim_start();
            let has_member_op = prefix.ends_with("->")
                || prefix.ends_with("?->")
                || prefix.ends_with("::")
                || prefix.ends_with('.');
            let left_boundary = pos == 0 || !is_ident_char(bytes[pos - 1] as char);
            if has_member_op {
                if rest.starts_with('(') {
                    return Some("call");
                }
                if prefix.ends_with("::") {
                    return Some(if kind == "const" {
                        "const"
                    } else {
                        "static_read"
                    });
                }
                if rest.starts_with('=') && !rest.starts_with("==") && !rest.starts_with("=>") {
                    return Some("write");
                }
                return Some(if kind == "const" { "const" } else { "read" });
            }
            if is_top_level && left_boundary {
                if kind == "function" && rest.starts_with('(') {
                    return Some("call");
                }
                if kind == "const" {
                    return Some("const");
                }
            }
        }
    }
    None
}

fn push_unique_str(out: &mut Vec<String>, sym: &str) {
    if !sym.is_empty() && !out.iter().any(|existing| existing == sym) {
        out.push(sym.to_owned());
    }
}

struct DepAccumulator<'a> {
    self_path: String,
    known_files: &'a BTreeSet<String>,
    entries: BTreeMap<(String, String), DepRef>,
}

impl<'a> DepAccumulator<'a> {
    fn new(self_path: &str, known_files: &'a BTreeSet<String>) -> Self {
        Self {
            self_path: clean_rel_key(self_path),
            known_files,
            entries: BTreeMap::new(),
        }
    }

    fn add_resolved(
        &mut self,
        target: String,
        kind: &str,
        symbols: &[&str],
        line: u32,
        evidence: &str,
    ) {
        self.add_resolved_members(target, kind, symbols, &[], line, evidence);
    }

    fn add_resolved_members(
        &mut self,
        target: String,
        kind: &str,
        symbols: &[&str],
        members: &[DepMemberRef],
        line: u32,
        evidence: &str,
    ) {
        let cleaned = clean_rel_key(&target);
        if cleaned.is_empty() || cleaned == self.self_path {
            return;
        }
        let exists = self.known_files.contains(&target);
        self.insert_dep(target, true, exists, kind, symbols, members, line, evidence);
    }

    fn add_unresolved(
        &mut self,
        target: &str,
        kind: &str,
        symbols: &[&str],
        line: u32,
        evidence: &str,
    ) {
        let trimmed = target.trim();
        if trimmed.is_empty() {
            return;
        }
        self.insert_dep(
            trimmed.to_owned(),
            false,
            false,
            kind,
            symbols,
            &[],
            line,
            evidence,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_dep(
        &mut self,
        target: String,
        resolved: bool,
        exists: bool,
        kind: &str,
        symbols: &[&str],
        members: &[DepMemberRef],
        line: u32,
        evidence: &str,
    ) {
        let key = (target.clone(), kind.to_owned());
        let trimmed_ev = clip_evidence(evidence);
        let entry = self.entries.entry(key).or_insert_with(|| DepRef {
            target,
            resolved,
            exists,
            kind: kind.to_owned(),
            symbols: Vec::new(),
            line: Some(line),
            evidence: (!trimmed_ev.is_empty()).then_some(trimmed_ev.clone()),
            member_refs: Vec::new(),
            target_members: Vec::new(),
        });
        if resolved && !entry.resolved {
            entry.resolved = true;
            entry.exists = exists;
        }
        if entry.line.is_none_or(|prev| line < prev) {
            entry.line = Some(line);
        }
        if entry.evidence.is_none() && !trimmed_ev.is_empty() {
            entry.evidence = Some(trimmed_ev);
        }
        for sym in symbols {
            let s = sym.trim();
            if !s.is_empty() && !entry.symbols.iter().any(|existing| existing == s) {
                entry.symbols.push(s.to_owned());
            }
        }
        for m in members {
            push_unique_member_ref(&mut entry.member_refs, m.clone());
        }
    }

    fn into_sorted_vec(self) -> Vec<DepRef> {
        let mut out: Vec<DepRef> = self.entries.into_values().collect();
        sort_dep_refs(&mut out);
        out
    }
}

fn merge_dep_into_map(map: &mut BTreeMap<(String, String), DepRef>, dep: DepRef) {
    let key = (dep.target.clone(), dep.kind.clone());
    if let Some(existing) = map.get_mut(&key) {
        for sym in dep.symbols {
            if !existing.symbols.contains(&sym) {
                existing.symbols.push(sym);
            }
        }
        for m in dep.member_refs {
            push_unique_member_ref(&mut existing.member_refs, m);
        }
        if existing
            .line
            .is_none_or(|prev| dep.line.is_some_and(|l| l < prev))
        {
            existing.line = dep.line;
        }
        if existing.evidence.is_none() {
            existing.evidence = dep.evidence;
        }
    } else {
        map.insert(key, dep);
    }
}

fn sort_dep_refs(list: &mut [DepRef]) {
    for dep in list.iter_mut() {
        sort_member_refs(&mut dep.member_refs);
    }
    list.sort_by(|a, b| {
        b.resolved
            .cmp(&a.resolved)
            .then_with(|| a.line.unwrap_or(u32::MAX).cmp(&b.line.unwrap_or(u32::MAX)))
            .then_with(|| a.target.cmp(&b.target))
            .then_with(|| a.kind.cmp(&b.kind))
    });
}

fn clip_evidence(raw: &str) -> String {
    let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.len() <= 140 {
        return compact;
    }
    let mut end = 140usize;
    while end > 0 && !compact.is_char_boundary(end) {
        end -= 1;
    }
    compact[..end].to_owned()
}

struct KnownLookup {
    by_cleaned: BTreeMap<String, String>,
    by_lower: BTreeMap<String, Vec<String>>,
    by_stem_lower: BTreeMap<String, Vec<String>>,
    by_dir_stem_lower: BTreeMap<(String, String), Vec<String>>,
    all_entries: Vec<(String, String, String)>,
}

impl KnownLookup {
    fn new(known_files: &BTreeSet<String>) -> Self {
        let mut by_cleaned = BTreeMap::new();
        let mut by_lower: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut by_stem_lower: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut by_dir_stem_lower: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
        let mut all_entries = Vec::with_capacity(known_files.len());

        for orig in known_files {
            let cleaned = clean_rel_key(orig);
            if cleaned.is_empty() {
                continue;
            }
            let lower = cleaned.to_ascii_lowercase();
            by_cleaned
                .entry(cleaned.clone())
                .or_insert_with(|| orig.clone());
            by_lower
                .entry(lower.clone())
                .or_default()
                .push(orig.clone());

            let stem_lower = file_stem(&cleaned).to_ascii_lowercase();
            let dir_lower = folder_of(&cleaned).to_ascii_lowercase();
            if !stem_lower.is_empty() {
                by_stem_lower
                    .entry(stem_lower.clone())
                    .or_default()
                    .push(orig.clone());
                by_dir_stem_lower
                    .entry((dir_lower, stem_lower))
                    .or_default()
                    .push(orig.clone());
            }
            all_entries.push((orig.clone(), cleaned, lower));
        }

        Self {
            by_cleaned,
            by_lower,
            by_stem_lower,
            by_dir_stem_lower,
            all_entries,
        }
    }

    fn resolve_exact_or_ci(&self, candidate: &str) -> Option<String> {
        let cleaned = clean_rel_key(candidate);
        if cleaned.is_empty() {
            return None;
        }
        if let Some(exact) = self.by_cleaned.get(&cleaned) {
            return Some(exact.clone());
        }
        let lower = cleaned.to_ascii_lowercase();
        self.by_lower
            .get(&lower)
            .and_then(|matches| pick_best_candidate(matches))
    }

    fn resolve_relative(&self, base_dir: &str, spec: &str) -> Option<String> {
        let joined = resolve_relative_segments(base_dir, spec)?;
        self.resolve_exact_or_ci(&joined)
    }

    fn resolve_suffix(&self, suffix: &str, allowed_exts: Option<&[&str]>) -> Option<String> {
        let cleaned = clean_rel_key(suffix);
        if cleaned.is_empty() {
            return None;
        }
        let slash_suffix = format!("/{cleaned}");
        let exact_matches: Vec<String> = self
            .all_entries
            .iter()
            .filter(|(_, c, _)| {
                (c == &cleaned || c.ends_with(&slash_suffix))
                    && allowed_exts.is_none_or(|exts| has_any_ext(c, exts))
            })
            .map(|(orig, _, _)| orig.clone())
            .collect();
        if let Some(found) = pick_best_candidate(&exact_matches) {
            return Some(found);
        }

        let lower_cleaned = cleaned.to_ascii_lowercase();
        let lower_slash = format!("/{lower_cleaned}");
        let ci_matches: Vec<String> = self
            .all_entries
            .iter()
            .filter(|(_, c, l)| {
                (l == &lower_cleaned || l.ends_with(&lower_slash))
                    && allowed_exts.is_none_or(|exts| has_any_ext(c, exts))
            })
            .map(|(orig, _, _)| orig.clone())
            .collect();
        pick_best_candidate(&ci_matches)
    }

    fn resolve_stem_in_dir(&self, dir: &str, stem: &str, allowed_exts: &[&str]) -> Option<String> {
        let dir_lower = dir.to_ascii_lowercase();
        for variant in stem_variants(stem) {
            if let Some(list) = self.by_dir_stem_lower.get(&(dir_lower.clone(), variant)) {
                let filtered: Vec<String> = list
                    .iter()
                    .filter(|p| has_any_ext(p, allowed_exts))
                    .cloned()
                    .collect();
                if filtered.len() == 1 {
                    return Some(filtered[0].clone());
                }
                if let Some(best) = pick_unique_non_test(&filtered) {
                    return Some(best);
                }
            }
        }
        None
    }

    fn resolve_unambiguous_stem(
        &self,
        stem: &str,
        allowed_exts: &[&str],
        scope_prefix: Option<&str>,
    ) -> Option<String> {
        for variant in stem_variants(stem) {
            let Some(list) = self.by_stem_lower.get(&variant) else {
                continue;
            };
            let filtered: Vec<String> = list
                .iter()
                .filter(|p| has_any_ext(p, allowed_exts))
                .cloned()
                .collect();
            if filtered.len() == 1 {
                return Some(filtered[0].clone());
            }
            if let Some(prefix) = scope_prefix.filter(|p| !p.is_empty()) {
                let prefix_slash = format!("{prefix}/");
                let scoped: Vec<String> = filtered
                    .iter()
                    .filter(|p| p.starts_with(&prefix_slash))
                    .cloned()
                    .collect();
                if scoped.len() == 1 {
                    return Some(scoped[0].clone());
                }
            }
            if let Some(best) = pick_unique_non_test(&filtered) {
                return Some(best);
            }
        }
        None
    }
}

fn stem_variants(stem: &str) -> Vec<String> {
    let lower = stem.to_ascii_lowercase();
    let snake = camel_to_snake_lower(stem);
    if snake == lower {
        vec![lower]
    } else {
        vec![lower, snake]
    }
}

fn camel_to_snake_lower(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    let chars: Vec<char> = s.chars().collect();
    for (idx, &ch) in chars.iter().enumerate() {
        if ch.is_ascii_uppercase() {
            if idx > 0 {
                let prev = chars[idx - 1];
                let next_lower = chars.get(idx + 1).is_some_and(|c| c.is_ascii_lowercase());
                if prev.is_ascii_lowercase()
                    || prev.is_ascii_digit()
                    || (prev.is_ascii_uppercase() && next_lower)
                {
                    out.push('_');
                }
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch.to_ascii_lowercase());
        }
    }
    out
}

fn has_any_ext(path: &str, exts: &[&str]) -> bool {
    let Some((_, ext)) = path.rsplit_once('.') else {
        return false;
    };
    let lower = ext.to_ascii_lowercase();
    exts.contains(&lower.as_str())
}

fn pick_best_candidate(matches: &[String]) -> Option<String> {
    if matches.is_empty() {
        return None;
    }
    if matches.len() == 1 {
        return Some(matches[0].clone());
    }
    if let Some(unique) = pick_unique_non_test(matches) {
        return Some(unique);
    }
    let mut sorted = matches.to_vec();
    sorted.sort_by(|a, b| {
        candidate_rank(a)
            .cmp(&candidate_rank(b))
            .then_with(|| a.len().cmp(&b.len()))
            .then_with(|| a.cmp(b))
    });
    sorted.into_iter().next()
}

fn pick_unique_non_test(matches: &[String]) -> Option<String> {
    let non_test: Vec<&String> = matches
        .iter()
        .filter(|p| !looks_like_test_or_vendor(p))
        .collect();
    if non_test.len() == 1 {
        Some(non_test[0].clone())
    } else {
        None
    }
}

fn candidate_rank(path: &str) -> u8 {
    let lower = path.to_ascii_lowercase();
    if lower.starts_with("src/") || lower.starts_with("app/") {
        0
    } else if !looks_like_test_or_vendor(path) {
        1
    } else {
        2
    }
}

fn looks_like_test_or_vendor(path: &str) -> bool {
    path.split('/').any(|seg| {
        matches!(
            seg.to_ascii_lowercase().as_str(),
            "test" | "tests" | "__tests__" | "spec" | "fixtures" | "vendor" | "node_modules"
        )
    })
}

fn file_stem(rel_path: &str) -> &str {
    let name = rel_path.rsplit_once('/').map_or(rel_path, |(_, n)| n);
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

fn resolve_relative_segments(base_dir: &str, spec: &str) -> Option<String> {
    let norm_spec = spec.trim().replace('\\', "/");
    if norm_spec.is_empty() {
        return None;
    }
    let mut stack: Vec<&str> = if norm_spec.starts_with('/') {
        Vec::new()
    } else {
        base_dir
            .split('/')
            .filter(|s| !s.is_empty() && *s != ".")
            .collect()
    };
    for seg in norm_spec.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                stack.pop()?;
            }
            other => stack.push(other),
        }
    }
    let joined = stack.join("/");
    (!joined.is_empty()).then_some(joined)
}

// =========================================================================
// Shared Member Classification Helpers
// =========================================================================

fn is_upper_snake(name: &str) -> bool {
    let bare = norm_member_name(name);
    bare.len() >= 2
        && bare.chars().any(|c| c.is_ascii_uppercase())
        && bare
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn is_write_operator(rest: &str) -> bool {
    let r = rest.trim_start();
    if r.starts_with("=>") || r.starts_with("==") || r.starts_with("===") {
        return false;
    }
    if r.starts_with('=') {
        return true;
    }
    [
        "+=", "-=", "*=", "/=", "%=", ".=", "&=", "|=", "^=", "??=", "<<=", ">>=",
    ]
    .iter()
    .any(|op| r.starts_with(op))
}

/// The kind of dependency a member access is drawn as: writing state,
/// reading it, or calling. Anything else stays a plain use.
pub(crate) fn access_kind(access: &str) -> Option<&'static str> {
    match access {
        "write" => Some("writes"),
        "read" | "static_read" | "const" => Some("reads"),
        "call" => Some("calls"),
        _ => None,
    }
}

/// What one member access is: the kind of the dependency it makes (`calls`,
/// `writes`, `reads`), what the member is, and how it is reached. An
/// assignment writes; every other access of a field, a static property or a
/// constant reads.
fn classify_member_access(
    member: &str,
    after_member: &str,
    is_static: bool,
    is_call: bool,
    field_kind: &'static str,
    module_level: bool,
) -> (&'static str, &'static str, &'static str) {
    if is_call {
        let mk = if module_level { "function" } else { "method" };
        return ("calls", mk, "call");
    }
    let (member_kind, access) = if is_static {
        if is_upper_snake(member) && !member.starts_with('$') {
            ("const", "const")
        } else if is_write_operator(after_member) {
            (field_kind, "write")
        } else if member.starts_with('$') || field_kind == "field" || field_kind == "property" {
            (field_kind, "static_read")
        } else {
            ("const", "const")
        }
    } else if is_upper_snake(member) && module_level {
        ("const", "const")
    } else if is_write_operator(after_member) {
        (field_kind, "write")
    } else {
        (field_kind, "read")
    };
    (access_kind(access).unwrap_or("uses"), member_kind, access)
}

fn take_ts_js_member_ident(s: &str) -> Option<(&str, &str)> {
    if let Some(after_hash) = s.strip_prefix('#') {
        let (ident, rest) = take_ident(after_hash)?;
        let total = 1 + ident.len();
        return Some((&s[..total], rest));
    }
    take_ident(s)
}

fn contains_word_ascii(haystack: &str, word: &str) -> bool {
    let bytes = haystack.as_bytes();
    let len = bytes.len();
    for i in 0..len {
        if starts_word_at(haystack, i, word) {
            let end = i + word.len();
            if end == len || !is_ident_char(bytes[end] as char) {
                return true;
            }
        }
    }
    false
}

fn split_simple_assign(line: &str) -> Option<(&str, &str)> {
    let bytes = line.as_bytes();
    let mut paren = 0usize;
    let mut bracket = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => paren += 1,
            b')' => paren = paren.saturating_sub(1),
            b'[' => bracket += 1,
            b']' => bracket = bracket.saturating_sub(1),
            b'=' if paren == 0 && bracket == 0 => {
                let prev = i.checked_sub(1).and_then(|k| bytes.get(k)).copied();
                let next = bytes.get(i + 1).copied();
                if matches!(prev, Some(b'=' | b'!' | b'<' | b'>'))
                    || matches!(next, Some(b'=' | b'>'))
                {
                    continue;
                }
                return Some((line[..i].trim(), line[i + 1..].trim()));
            }
            _ => {}
        }
    }
    None
}

// =========================================================================
// PHP Dependency Extraction & Resolution
// =========================================================================

fn extract_php_deps(rel_path: &str, source: &str, lookup: &KnownLookup, acc: &mut DepAccumulator) {
    const PHP_TYPE_BUILTINS: &[&str] = &[
        "int",
        "integer",
        "float",
        "double",
        "string",
        "bool",
        "boolean",
        "array",
        "object",
        "callable",
        "iterable",
        "void",
        "never",
        "mixed",
        "self",
        "static",
        "parent",
        "null",
        "true",
        "false",
        "stdclass",
        "exception",
        "throwable",
        "errorexception",
        "runtimeexception",
        "invalidargumentexception",
        "logicexception",
        "closure",
        "generator",
        "iterator",
        "traversable",
        "countable",
        "arrayaccess",
        "jsonserializable",
        "stringable",
        "datetime",
        "datetimeimmutable",
        "datetimeinterface",
    ];

    let mut current_ns: Option<String> = None;
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    let mut this_props: BTreeMap<String, String> = BTreeMap::new();
    let mut var_types: BTreeMap<String, String> = BTreeMap::new();
    let mut in_block_comment = false;
    let mut depth = 0usize;
    let mut class_stack: Vec<(bool, usize)> = Vec::new(); // (is_class_like, open_depth)
    let mut pending_class_like = false;
    let mut pending_fn = false;
    let mut in_ctor_sig = false;

    let mut use_buf = String::new();
    let mut use_start_line = 0u32;
    let mut class_hdr_buf = String::new();
    let mut class_hdr_line = 0u32;

    let dir = folder_of(rel_path);

    for (idx, raw_line) in source.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let comments_stripped = strip_comments_keep_strings(raw_line, Language::Php);
        let sanitized = sanitize_c_like_line(raw_line, &mut in_block_comment, Language::Php);
        let trimmed = sanitized.trim();
        let clean_stmt = trimmed
            .trim_start_matches("<?php")
            .trim_start_matches("<?")
            .trim();

        // 1. Handle PHP require / include statements using raw comment-stripped text (strings kept).
        extract_php_includes(&dir, &comments_stripped, line_no, lookup, acc);

        let inside_class = class_stack
            .last()
            .is_some_and(|(is_class, open_d)| *is_class && *open_d == depth);
        let inside_fn = class_stack.iter().any(|(is_class, _)| !is_class);

        // 2. Continue accumulated `use ...;` statement (both namespace imports and trait uses).
        if !use_buf.is_empty() {
            use_buf.push(' ');
            use_buf.push_str(clean_stmt);
            if clean_stmt.contains(';') || clean_stmt.contains('{') {
                let stmt = std::mem::take(&mut use_buf);
                process_php_use_stmt(
                    &stmt,
                    inside_class,
                    current_ns.as_deref(),
                    &dir,
                    &mut aliases,
                    use_start_line,
                    lookup,
                    acc,
                );
            }
        } else if !class_hdr_buf.is_empty() {
            class_hdr_buf.push(' ');
            class_hdr_buf.push_str(clean_stmt);
            if clean_stmt.contains('{') || clean_stmt.contains(';') {
                let hdr = std::mem::take(&mut class_hdr_buf);
                extract_php_inheritance(
                    &hdr,
                    current_ns.as_deref(),
                    &dir,
                    &aliases,
                    class_hdr_line,
                    lookup,
                    acc,
                );
            }
        } else if !inside_fn && let Some((kw, after_kw)) = take_ident(clean_stmt) {
            match kw {
                "namespace" => {
                    let ns_part = after_kw.split([';', '{']).next().unwrap_or("").trim();
                    if !ns_part.is_empty() && !ns_part.starts_with('\\') {
                        current_ns = Some(ns_part.trim_matches('\\').to_owned());
                    }
                }
                "use" if !after_kw.trim_start().starts_with('(') => {
                    if clean_stmt.contains(';') || (inside_class && clean_stmt.contains('{')) {
                        process_php_use_stmt(
                            clean_stmt,
                            inside_class,
                            current_ns.as_deref(),
                            &dir,
                            &mut aliases,
                            line_no,
                            lookup,
                            acc,
                        );
                    } else {
                        use_buf = clean_stmt.to_owned();
                        use_start_line = line_no;
                    }
                }
                _ => {
                    let stripped = strip_leading_words(
                        clean_stmt,
                        &[
                            "abstract",
                            "final",
                            "readonly",
                            "public",
                            "protected",
                            "private",
                            "static",
                        ],
                    );
                    if let Some((decl_kw, after_decl)) = take_ident(stripped) {
                        match decl_kw {
                            "class" | "interface" | "trait" | "enum"
                                if !trimmed.starts_with("new ") =>
                            {
                                pending_class_like = true;
                                if clean_stmt.contains('{') || clean_stmt.contains(';') {
                                    extract_php_inheritance(
                                        clean_stmt,
                                        current_ns.as_deref(),
                                        &dir,
                                        &aliases,
                                        line_no,
                                        lookup,
                                        acc,
                                    );
                                } else {
                                    class_hdr_buf = after_decl.to_owned();
                                    class_hdr_line = line_no;
                                }
                            }
                            "function" => {
                                pending_fn = true;
                                if after_decl.trim_start().starts_with("__construct") {
                                    in_ctor_sig = true;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // 3. Extract calls, instantiations, static references, instanceof, catch, typehints, member accesses.
        if !trimmed.is_empty()
            && !clean_stmt.starts_with("use ")
            && !clean_stmt.starts_with("namespace ")
        {
            extract_php_line_refs(
                clean_stmt,
                inside_class,
                inside_fn || pending_fn,
                in_ctor_sig,
                current_ns.as_deref(),
                &dir,
                &aliases,
                PHP_TYPE_BUILTINS,
                line_no,
                lookup,
                &mut this_props,
                &mut var_types,
                acc,
            );
        }

        // 4. Update PHP brace scope.
        for ch in sanitized.chars() {
            match ch {
                '{' => {
                    depth += 1;
                    if pending_class_like {
                        pending_class_like = false;
                        class_stack.push((true, depth));
                    } else if pending_fn {
                        pending_fn = false;
                        in_ctor_sig = false;
                        class_stack.push((false, depth));
                    }
                }
                '}' => {
                    depth = depth.saturating_sub(1);
                    while class_stack.last().is_some_and(|(_, d)| depth < *d) {
                        if let Some((is_class, _)) = class_stack.pop() {
                            if is_class {
                                if !class_stack.iter().any(|(c, _)| *c) {
                                    this_props.clear();
                                }
                            } else {
                                var_types.clear();
                            }
                        }
                    }
                }
                ';' => {
                    pending_class_like = false;
                    if pending_fn {
                        pending_fn = false;
                        in_ctor_sig = false;
                        var_types.clear();
                    }
                }
                _ => {}
            }
        }
    }
}

fn extract_php_includes(
    base_dir: &str,
    comments_stripped: &str,
    line_no: u32,
    lookup: &KnownLookup,
    acc: &mut DepAccumulator,
) {
    for kw in ["require_once", "include_once", "require", "include"] {
        let Some(pos) = find_keyword(comments_stripped, kw) else {
            continue;
        };
        let rest = &comments_stripped[pos + kw.len()..];
        let trimmed = rest.trim_start();
        if !trimmed.starts_with(['\'', '"', '(', '_', 'd']) {
            continue;
        }
        if let Some(quoted) = extract_first_quoted_string(trimmed) {
            let candidate = quoted.trim();
            if candidate.is_empty() {
                continue;
            }
            let normalized = candidate.trim_start_matches('/');
            let resolved = if trimmed.contains("__DIR__") || trimmed.contains("dirname(") {
                lookup.resolve_relative(base_dir, normalized)
            } else {
                lookup
                    .resolve_relative(base_dir, candidate)
                    .or_else(|| lookup.resolve_exact_or_ci(normalized))
                    .or_else(|| lookup.resolve_suffix(normalized, Some(PHP_EXTS)))
            };
            let sym = file_stem(normalized);
            match resolved {
                Some(target) => {
                    acc.add_resolved(target, "imports", &[sym], line_no, comments_stripped)
                }
                None => {
                    acc.add_unresolved(candidate, "imports", &[sym], line_no, comments_stripped)
                }
            }
        }
        break;
    }
}

#[allow(clippy::too_many_arguments)]
fn process_php_use_stmt(
    stmt: &str,
    inside_class: bool,
    current_ns: Option<&str>,
    dir: &str,
    aliases: &mut BTreeMap<String, String>,
    line_no: u32,
    lookup: &KnownLookup,
    acc: &mut DepAccumulator,
) {
    let body = stmt
        .trim_start()
        .strip_prefix("use")
        .unwrap_or(stmt)
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    if body.is_empty() || body.starts_with('(') {
        return;
    }
    let body = if inside_class {
        body.split('{').next().unwrap_or(body).trim()
    } else {
        body
    };
    let kind = if inside_class { "uses" } else { "imports" };

    for item in split_php_use_clauses(body) {
        let mut item = item.trim();
        let mut is_fn_or_const = false;
        if let Some(rest) = item.strip_prefix("function ") {
            item = rest.trim();
            is_fn_or_const = true;
        } else if let Some(rest) = item.strip_prefix("const ") {
            item = rest.trim();
            is_fn_or_const = true;
        }
        if item.is_empty() {
            continue;
        }
        let (fqn_raw, alias) = if let Some((lhs, rhs)) = split_as_alias(item) {
            (lhs.trim(), Some(rhs.trim()))
        } else {
            (item, None)
        };
        let fqn = fqn_raw.trim_start_matches('\\').trim();
        if fqn.is_empty() {
            continue;
        }
        let short_name = fqn.rsplit('\\').next().unwrap_or(fqn);
        let local_name = alias.unwrap_or(short_name);
        aliases.insert(local_name.to_owned(), fqn.to_owned());

        let resolved = resolve_php_fqn(fqn, current_ns, dir, lookup).or_else(|| {
            if is_fn_or_const && let Some((parent_ns, _)) = fqn.rsplit_once('\\') {
                resolve_php_fqn(parent_ns, current_ns, dir, lookup)
            } else {
                None
            }
        });
        match resolved {
            Some(target) => acc.add_resolved(target, kind, &[short_name], line_no, stmt),
            None => acc.add_unresolved(fqn, kind, &[short_name], line_no, stmt),
        }
    }
}

fn split_php_use_clauses(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    // Handle grouped `Prefix\{A, B as C}` as well as comma-separated `A, B`.
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut top_items = Vec::new();
    for (idx, ch) in body.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                top_items.push(body[start..idx].trim());
                start = idx + 1;
            }
            _ => {}
        }
    }
    top_items.push(body[start..].trim());

    for item in top_items {
        if let Some((prefix_raw, brace_rest)) = item.split_once('{') {
            let inner = brace_rest.rsplit_once('}').map_or(brace_rest, |(i, _)| i);
            let prefix = prefix_raw.trim().trim_end_matches('\\');
            for sub in inner.split(',') {
                let mut sub = sub.trim();
                let mut qualifier = "";
                if let Some(rest) = sub.strip_prefix("function ") {
                    qualifier = "function ";
                    sub = rest.trim();
                } else if let Some(rest) = sub.strip_prefix("const ") {
                    qualifier = "const ";
                    sub = rest.trim();
                }
                if !sub.is_empty() {
                    out.push(format!("{qualifier}{prefix}\\{sub}"));
                }
            }
        } else if !item.is_empty() {
            out.push(item.to_owned());
        }
    }
    out
}

fn split_as_alias(clause: &str) -> Option<(&str, &str)> {
    let lower = clause.to_ascii_lowercase();
    let idx = lower.rfind(" as ")?;
    Some((&clause[..idx], &clause[idx + 4..]))
}

fn extract_php_inheritance(
    hdr: &str,
    current_ns: Option<&str>,
    dir: &str,
    aliases: &BTreeMap<String, String>,
    line_no: u32,
    lookup: &KnownLookup,
    acc: &mut DepAccumulator,
) {
    let before_brace = hdr.split(['{', ';']).next().unwrap_or(hdr);
    for (kw, kind) in [("extends", "extends"), ("implements", "implements")] {
        let Some(pos) = find_keyword(before_brace, kw) else {
            continue;
        };
        let after = &before_brace[pos + kw.len()..];
        let stop = if kw == "extends" {
            find_keyword(after, "implements").unwrap_or(after.len())
        } else {
            after.len()
        };
        for item in after[..stop].split(',') {
            let raw_name = item.trim();
            if raw_name.is_empty() {
                continue;
            }
            let short = raw_name.rsplit('\\').next().unwrap_or(raw_name);
            let fqn = resolve_php_alias(raw_name, aliases, current_ns);
            match resolve_php_fqn(&fqn, current_ns, dir, lookup) {
                Some(target) => acc.add_resolved(target, kind, &[short], line_no, hdr),
                None => acc.add_unresolved(&fqn, kind, &[short], line_no, hdr),
            }
        }
    }
}

fn take_php_arrow_ident(s: &str) -> Option<(&str, &str)> {
    let trimmed = s.trim_start();
    let after_op = trimmed
        .strip_prefix("?->")
        .or_else(|| trimmed.strip_prefix("->"))?;
    take_ident(after_op.trim_start())
}

fn is_php_type_candidate(ty: &str, aliases: &BTreeMap<String, String>, builtins: &[&str]) -> bool {
    let short = ty.rsplit('\\').next().unwrap_or(ty);
    if builtins.contains(&short.to_ascii_lowercase().as_str()) {
        return false;
    }
    aliases.contains_key(short)
        || ty.contains('\\')
        || short.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

fn extract_php_typehint_before(
    prefix: &str,
    aliases: &BTreeMap<String, String>,
    builtins: &[&str],
) -> Option<String> {
    let ty = take_trailing_php_name(prefix)?;
    if is_php_type_candidate(ty, aliases, builtins) {
        return Some(ty.to_owned());
    }
    if ty.eq_ignore_ascii_case("null")
        && let Some(pos) = prefix.trim_end().rfind(ty)
        && let Some(before_pipe) = prefix[..pos].trim_end().strip_suffix('|')
        && let Some(alt) = take_trailing_php_name(before_pipe)
        && is_php_type_candidate(alt, aliases, builtins)
    {
        return Some(alt.to_owned());
    }
    None
}

fn extract_php_rhs_type(
    rhs: &str,
    this_props: &BTreeMap<String, String>,
    var_types: &BTreeMap<String, String>,
) -> Option<String> {
    let cleaned = rhs.trim().trim_end_matches(';').trim();
    if let Some(after_new) = cleaned.strip_prefix("new")
        && after_new.starts_with(char::is_whitespace)
        && let Some((cls, _)) = take_php_qualified_name(after_new.trim_start())
    {
        return Some(cls.to_owned());
    }
    if let Some(after_dollar) = cleaned.strip_prefix('$')
        && let Some((ident, rest)) = take_ident(after_dollar)
    {
        if ident == "this"
            && let Some((prop, after_prop)) = take_php_arrow_ident(rest)
            && after_prop.trim().is_empty()
        {
            return this_props.get(prop).cloned();
        }
        if rest.trim().is_empty() {
            return var_types.get(ident).cloned();
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn extract_php_line_refs(
    line: &str,
    inside_class: bool,
    in_fn_or_sig: bool,
    in_ctor_sig: bool,
    current_ns: Option<&str>,
    dir: &str,
    aliases: &BTreeMap<String, String>,
    builtins: &[&str],
    line_no: u32,
    lookup: &KnownLookup,
    this_props: &mut BTreeMap<String, String>,
    var_types: &mut BTreeMap<String, String>,
    acc: &mut DepAccumulator,
) {
    if let Some((lhs, rhs)) = split_simple_assign(line)
        && let Some(ty) = extract_php_rhs_type(rhs, this_props, var_types)
    {
        let clean_lhs = lhs.trim();
        if let Some(after_dollar) = clean_lhs.strip_prefix('$')
            && let Some((ident, rest)) = take_ident(after_dollar)
        {
            if ident == "this"
                && let Some((prop, after_prop)) = take_php_arrow_ident(rest)
                && after_prop.trim().is_empty()
            {
                this_props.insert(prop.to_owned(), ty);
            } else if rest.trim().is_empty() {
                var_types.insert(ident.to_owned(), ty);
            }
        }
    }

    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;

    while i < len {
        // 1. `new ClassName`
        if starts_word_at(line, i, "new")
            && line[i + 3..].starts_with(char::is_whitespace)
            && let Some((cls, _)) = take_php_qualified_name(line[i + 3..].trim_start())
        {
            emit_php_ref(
                cls, "calls", None, None, current_ns, dir, aliases, builtins, line_no, line,
                lookup, acc,
            );
        }
        // 2. `instanceof ClassName`
        if starts_word_at(line, i, "instanceof")
            && line[i + 10..].starts_with(char::is_whitespace)
            && let Some((cls, _)) = take_php_qualified_name(line[i + 10..].trim_start())
        {
            emit_php_ref(
                cls, "uses", None, None, current_ns, dir, aliases, builtins, line_no, line, lookup,
                acc,
            );
        }
        // 3. `ClassName::member` (static method, class constant, or `ClassName::$prop`)
        if i + 1 < len
            && bytes[i] == b':'
            && bytes[i + 1] == b':'
            && let Some(cls) = take_trailing_php_name(line[..i].trim_end())
        {
            let after_colons = line[i + 2..].trim_start();
            if let Some(after_dollar) = after_colons.strip_prefix('$') {
                if let Some((ident, rest)) = take_ident(after_dollar) {
                    let prop_name = format!("${ident}");
                    let access = if is_write_operator(rest) {
                        "write"
                    } else {
                        "static_read"
                    };
                    emit_php_ref(
                        cls,
                        access_kind(access).unwrap_or("uses"),
                        Some(&prop_name),
                        Some(("property", access)),
                        current_ns,
                        dir,
                        aliases,
                        builtins,
                        line_no,
                        line,
                        lookup,
                        acc,
                    );
                }
            } else if let Some((m, rest)) = take_ident(after_colons) {
                if m == "class" {
                    emit_php_ref(
                        cls, "uses", None, None, current_ns, dir, aliases, builtins, line_no, line,
                        lookup, acc,
                    );
                } else {
                    let is_call = rest.trim_start().starts_with('(');
                    let (dep_kind, mem_kind, access) =
                        classify_member_access(m, rest, true, is_call, "property", false);
                    emit_php_ref(
                        cls,
                        dep_kind,
                        Some(m),
                        Some((mem_kind, access)),
                        current_ns,
                        dir,
                        aliases,
                        builtins,
                        line_no,
                        line,
                        lookup,
                        acc,
                    );
                }
            }
        }
        // 4. Typed vars/properties (`Signer $s`) and instance member accesses (`$this->p->m()`, `$svc->ttl`).
        if bytes[i] == b'$' {
            let preceded_by_colons = line[..i].trim_end().ends_with("::");
            if !preceded_by_colons && let Some((var_name, after_var)) = take_ident(&line[i + 1..]) {
                if var_name != "this"
                    && i > 0
                    && let Some(ty) = extract_php_typehint_before(&line[..i], aliases, builtins)
                {
                    emit_php_ref(
                        &ty, "uses", None, None, current_ns, dir, aliases, builtins, line_no, line,
                        lookup, acc,
                    );
                    let decl_chunk = line[..i]
                        .rsplit(['(', ',', '{', ';'])
                        .next()
                        .unwrap_or(&line[..i]);
                    let promoted = in_ctor_sig
                        && (contains_word_ascii(decl_chunk, "public")
                            || contains_word_ascii(decl_chunk, "protected")
                            || contains_word_ascii(decl_chunk, "private")
                            || contains_word_ascii(decl_chunk, "readonly"));
                    if (inside_class && !in_fn_or_sig) || promoted {
                        this_props.insert(var_name.to_owned(), ty.clone());
                    }
                    if in_fn_or_sig || in_ctor_sig || line[..i].contains('(') {
                        var_types.insert(var_name.to_owned(), ty);
                    }
                } else if !preceded_by_colons
                    && let Some((mem, after_mem)) = take_php_arrow_ident(after_var)
                {
                    if var_name == "this" {
                        if let Some((mem2, after_mem2)) = take_php_arrow_ident(after_mem)
                            && let Some(owner_ty) = this_props.get(mem).cloned()
                        {
                            let is_call = after_mem2.trim_start().starts_with('(');
                            let (dep_kind, mem_kind, access) = classify_member_access(
                                mem2, after_mem2, false, is_call, "property", false,
                            );
                            let mem_name = if mem_kind == "property" {
                                format!("${mem2}")
                            } else {
                                mem2.to_owned()
                            };
                            emit_php_ref(
                                &owner_ty,
                                dep_kind,
                                Some(&mem_name),
                                Some((mem_kind, access)),
                                current_ns,
                                dir,
                                aliases,
                                builtins,
                                line_no,
                                line,
                                lookup,
                                acc,
                            );
                        }
                    } else if let Some(owner_ty) = var_types.get(var_name).cloned() {
                        let is_call = after_mem.trim_start().starts_with('(');
                        let (dep_kind, mem_kind, access) = classify_member_access(
                            mem, after_mem, false, is_call, "property", false,
                        );
                        let mem_name = if mem_kind == "property" {
                            format!("${mem}")
                        } else {
                            mem.to_owned()
                        };
                        emit_php_ref(
                            &owner_ty,
                            dep_kind,
                            Some(&mem_name),
                            Some((mem_kind, access)),
                            current_ns,
                            dir,
                            aliases,
                            builtins,
                            line_no,
                            line,
                            lookup,
                            acc,
                        );
                    }
                }
            }
        }
        i += 1;
    }
}

fn resolve_php_target(
    raw_name: &str,
    current_ns: Option<&str>,
    dir: &str,
    aliases: &BTreeMap<String, String>,
    builtins: &[&str],
    lookup: &KnownLookup,
) -> Option<(String, String)> {
    let short = raw_name.rsplit('\\').next().unwrap_or(raw_name);
    if builtins.contains(&short.to_ascii_lowercase().as_str()) {
        return None;
    }
    let fqn = resolve_php_alias(raw_name, aliases, current_ns);
    let canon = fqn.rsplit('\\').next().unwrap_or(short).to_owned();
    let target = resolve_php_fqn(&fqn, current_ns, dir, lookup)?;
    Some((target, canon))
}

#[allow(clippy::too_many_arguments)]
fn emit_php_ref(
    raw_name: &str,
    kind: &str,
    member: Option<&str>,
    member_meta: Option<(&str, &str)>,
    current_ns: Option<&str>,
    dir: &str,
    aliases: &BTreeMap<String, String>,
    builtins: &[&str],
    line_no: u32,
    line: &str,
    lookup: &KnownLookup,
    acc: &mut DepAccumulator,
) {
    if let Some((target, canon)) =
        resolve_php_target(raw_name, current_ns, dir, aliases, builtins, lookup)
    {
        match member {
            Some(m) => {
                let members = member_meta
                    .map(|(mk, acc_mode)| {
                        vec![DepMemberRef::basic(
                            m,
                            mk,
                            Some(canon.clone()),
                            acc_mode,
                            line_no,
                        )]
                    })
                    .unwrap_or_default();
                acc.add_resolved_members(target, kind, &[&canon, m], &members, line_no, line);
            }
            None => acc.add_resolved(target, kind, &[&canon], line_no, line),
        }
    }
}

fn resolve_php_alias(
    raw: &str,
    aliases: &BTreeMap<String, String>,
    current_ns: Option<&str>,
) -> String {
    let trimmed = raw.trim();
    if let Some(fqn) = trimmed.strip_prefix('\\') {
        return fqn.to_owned();
    }
    if let Some((head, tail)) = trimmed.split_once('\\') {
        if let Some(aliased) = aliases.get(head) {
            return format!("{aliased}\\{tail}");
        }
    } else if let Some(aliased) = aliases.get(trimmed) {
        return aliased.clone();
    }
    if let Some(ns) = current_ns.filter(|s| !s.is_empty()) {
        format!("{ns}\\{trimmed}")
    } else {
        trimmed.to_owned()
    }
}

fn resolve_php_fqn(
    fqn: &str,
    current_ns: Option<&str>,
    dir: &str,
    lookup: &KnownLookup,
) -> Option<String> {
    let cleaned = fqn.trim().trim_start_matches('\\');
    if cleaned.is_empty() {
        return None;
    }
    let slash_path = cleaned.replace('\\', "/");
    let segs: Vec<&str> = slash_path.split('/').filter(|s| !s.is_empty()).collect();

    for ext in PHP_EXTS {
        let full = format!("{slash_path}.{ext}");
        if let Some(found) = lookup.resolve_exact_or_ci(&full) {
            return Some(found);
        }
        // Try PSR-4 suffixes (e.g. `App\Security\Signer` -> `src/Security/Signer.php`).
        for start in 1..segs.len() {
            let suffix = format!("{}.{ext}", segs[start..].join("/"));
            if let Some(found) = lookup.resolve_suffix(&suffix, Some(PHP_EXTS)) {
                return Some(found);
            }
        }
    }

    let short = segs.last().copied().unwrap_or(cleaned);
    // Same-directory match (especially for same-namespace PHP classes without `use`).
    if !dir.is_empty()
        && let Some(found) = lookup.resolve_stem_in_dir(dir, short, PHP_EXTS)
    {
        return Some(found);
    }
    if segs.len() == 1 || current_ns.is_some_and(|ns| cleaned.starts_with(ns)) {
        if let Some(found) = lookup.resolve_stem_in_dir(dir, short, PHP_EXTS) {
            return Some(found);
        }
        return lookup.resolve_unambiguous_stem(short, PHP_EXTS, None);
    }
    None
}

fn take_php_qualified_name(s: &str) -> Option<(&str, &str)> {
    let mut end = 0usize;
    for (idx, ch) in s.char_indices() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '\\' {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    let candidate = &s[..end];
    if candidate.is_empty() || candidate == "\\" {
        None
    } else {
        Some((candidate, &s[end..]))
    }
}

fn take_trailing_php_name(s: &str) -> Option<&str> {
    let trimmed = s.trim_end();
    let bytes = trimmed.as_bytes();
    let mut start = bytes.len();
    while start > 0 {
        let b = bytes[start - 1];
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'\\' {
            start -= 1;
        } else {
            break;
        }
    }
    let cand = &trimmed[start..];
    if cand.is_empty() || cand.starts_with(|c: char| c.is_ascii_digit()) {
        None
    } else {
        Some(cand)
    }
}

// =========================================================================
// Rust Dependency Extraction & Resolution
// =========================================================================

enum RustBlockScope {
    Struct(String),
    Impl(String),
    Fn,
    Other,
}

fn unwrap_rust_wrapper_type(raw: &str) -> String {
    let mut ty = raw.trim().trim_end_matches([',', ';', ')']).trim();
    loop {
        let before = ty;
        while let Some(rest) = ty.strip_prefix('&') {
            ty = rest.trim_start();
            if ty.starts_with('\'')
                && let Some((_, after)) = ty.split_once(char::is_whitespace)
            {
                ty = after.trim_start();
            }
            if let Some(after) = ty.strip_prefix("mut")
                && after.starts_with(char::is_whitespace)
            {
                ty = after.trim_start();
            }
        }
        for wrapper in [
            "Arc",
            "Rc",
            "Box",
            "Option",
            "Mutex",
            "RwLock",
            "RefCell",
            "Cell",
            "Pin",
            "std::sync::Arc",
            "alloc::rc::Rc",
            "std::boxed::Box",
            "core::option::Option",
        ] {
            if let Some(after_w) = ty.strip_prefix(wrapper)
                && let Some(inner_start) = after_w.trim_start().strip_prefix('<')
                && let Some(last_gt) = inner_start.rfind('>')
            {
                ty = inner_start[..last_gt].trim();
                break;
            }
        }
        if ty == before {
            break;
        }
    }
    ty.split('<').next().unwrap_or(ty).trim().to_owned()
}

fn parse_rust_impl_target(after_impl: &str) -> Option<String> {
    let hdr = after_impl.split('{').next().unwrap_or(after_impl);
    let no_where = hdr.split(" where ").next().unwrap_or(hdr).trim();
    let ty_part = if let Some((_, rhs)) = no_where.split_once(" for ") {
        rhs.trim()
    } else {
        no_where
    };
    let clean = strip_generics_and_refs(ty_part);
    let short = clean.rsplit("::").next().unwrap_or(clean).trim();
    if short.is_empty() {
        None
    } else {
        Some(short.to_owned())
    }
}

fn collect_rust_param_bindings(line: &str, var_types: &mut BTreeMap<String, String>) {
    // Extract typed parameters `(a: TypeA, b: &mut Option<TypeB>)` or field-like `var: Type`
    let slice = if let Some((_, after_paren)) = line.split_once('(') {
        after_paren
    } else {
        line
    };
    for part in slice.split(',') {
        let p = part.trim();
        let Some((lhs, rhs)) = p.split_once(':') else {
            continue;
        };
        if rhs.starts_with(':') {
            continue;
        }
        let param_name = lhs.trim().trim_start_matches("mut ").trim();
        if !param_name.is_empty()
            && param_name != "self"
            && let Some((ident, rest)) = take_ident(param_name)
            && rest.trim().is_empty()
        {
            let ty = unwrap_rust_wrapper_type(rhs.split(['=', '{']).next().unwrap_or(rhs));
            if !ty.is_empty() {
                var_types.insert(ident.to_owned(), ty);
            }
        }
    }
}

fn collect_rust_let_binding(
    trimmed: &str,
    struct_fields_for_self: Option<&BTreeMap<String, String>>,
    var_types: &mut BTreeMap<String, String>,
) {
    let Some(after_let) = trimmed.strip_prefix("let ") else {
        return;
    };
    let after_mut = after_let
        .trim_start()
        .strip_prefix("mut ")
        .unwrap_or(after_let)
        .trim_start();
    let Some((var_name, after_var)) = take_ident(after_mut) else {
        return;
    };
    let rest = after_var.trim_start();
    if let Some(after_colon) = rest.strip_prefix(':') {
        let ty_raw = after_colon.split(['=', ';']).next().unwrap_or("");
        let ty = unwrap_rust_wrapper_type(ty_raw);
        if !ty.is_empty() {
            var_types.insert(var_name.to_owned(), ty);
            return;
        }
    }
    if let Some(after_eq) = rest.split_once('=').map(|(_, r)| r.trim()) {
        let rhs = after_eq.trim_end_matches(';').trim();
        if let Some(after_self) = rhs.strip_prefix("self.")
            && let Some((field, after_f)) = take_ident(after_self)
            && after_f.trim().is_empty()
            && let Some(ty) = struct_fields_for_self.and_then(|m| m.get(field))
        {
            var_types.insert(var_name.to_owned(), ty.clone());
            return;
        }
        // `Foo::new(...)`, `Foo::default()`, `Foo { ... }`
        let mut prefix_end = 0usize;
        let bytes = rhs.as_bytes();
        while prefix_end < bytes.len()
            && (is_ident_char(bytes[prefix_end] as char) || bytes[prefix_end] == b':')
        {
            prefix_end += 1;
        }
        let head = rhs[..prefix_end].trim();
        let after_head = rhs[prefix_end..].trim_start();
        if after_head.starts_with('{') && !head.is_empty() {
            var_types.insert(var_name.to_owned(), head.to_owned());
        } else if let Some((type_part, ctor)) = head.rsplit_once("::")
            && matches!(
                ctor,
                "new" | "default" | "create" | "open" | "connect" | "build" | "from"
            )
            && !type_part.is_empty()
        {
            var_types.insert(var_name.to_owned(), type_part.to_owned());
        }
    }
}

fn extract_rust_deps(rel_path: &str, source: &str, lookup: &KnownLookup, acc: &mut DepAccumulator) {
    const RUST_BUILTINS: &[&str] = &[
        "std", "core", "alloc", "self", "super", "crate", "Vec", "String", "Option", "Result",
        "Box", "Arc", "Rc", "Mutex", "RwLock", "RefCell", "Cell", "Path", "PathBuf", "BTreeMap",
        "BTreeSet", "HashMap", "HashSet", "Value", "Self", "Default", "Clone", "Debug", "Display",
        "Send", "Sync", "Iterator", "From", "Into", "TryFrom", "TryInto", "Ok", "Err", "Some",
        "None", "Ordering", "Duration", "Instant", "Command", "Stdio", "fs", "io", "mem", "fmt",
        "cmp", "str", "slice", "u8", "u16", "u32", "u64", "usize", "i32", "i64", "f64", "bool",
        "char",
    ];

    let crate_src_root = rust_crate_src_root(rel_path);
    let mut aliases: BTreeMap<String, (Option<String>, String)> = BTreeMap::new();
    let mut struct_fields: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut var_types: BTreeMap<String, String> = BTreeMap::new();
    let mut in_block_comment = false;
    let mut pending_mod_path: Option<String> = None;
    let mut use_buf = String::new();
    let mut use_line = 0u32;
    let mut depth = 0usize;
    let mut scope_stack: Vec<(RustBlockScope, usize)> = Vec::new();
    let mut pending_scope: Option<RustBlockScope> = None;
    let mut struct_lit_stack: Vec<(String, String, usize)> = Vec::new(); // (target, struct_name, open_depth)

    for (idx, raw_line) in source.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let comments_stripped = strip_comments_keep_strings(raw_line, Language::Rust);
        let sanitized = sanitize_c_like_line(raw_line, &mut in_block_comment, Language::Rust);
        let trimmed = sanitized.trim();

        // Capture `#[path = "..."]` attribute before `mod x;`
        if trimmed.starts_with('#') {
            if comments_stripped.contains("path")
                && comments_stripped.contains('=')
                && let Some(path_val) = extract_first_quoted_string(&comments_stripped)
            {
                pending_mod_path = Some(path_val.to_owned());
            }
            continue;
        }

        if trimmed.is_empty() {
            continue;
        }

        // 1. Multi-line `use ...;`
        if !use_buf.is_empty() {
            use_buf.push(' ');
            use_buf.push_str(trimmed);
            if trimmed.contains(';') {
                let full_stmt = std::mem::take(&mut use_buf);
                process_rust_use_stmt(
                    rel_path,
                    &crate_src_root,
                    &full_stmt,
                    use_line,
                    lookup,
                    &mut aliases,
                    acc,
                );
            }
            continue;
        }

        let inside_struct_name = scope_stack.last().and_then(|(sc, d)| match sc {
            RustBlockScope::Struct(name) if *d == depth => Some(name.clone()),
            _ => None,
        });
        let current_impl_name = scope_stack.iter().rev().find_map(|(sc, _)| match sc {
            RustBlockScope::Impl(name) => Some(name.as_str()),
            _ => None,
        });
        let inside_fn = scope_stack
            .iter()
            .any(|(sc, _)| matches!(sc, RustBlockScope::Fn));

        let after_vis = strip_rust_visibility(trimmed);
        let mut is_decl_header = false;
        if let Some((kw, after_kw)) = take_ident(after_vis) {
            if kw == "use" {
                if trimmed.contains(';') {
                    process_rust_use_stmt(
                        rel_path,
                        &crate_src_root,
                        trimmed,
                        line_no,
                        lookup,
                        &mut aliases,
                        acc,
                    );
                } else {
                    use_buf = trimmed.to_owned();
                    use_line = line_no;
                }
                pending_mod_path = None;
                continue;
            }
            if kw == "mod" && trimmed.ends_with(';') {
                if let Some((mod_name, _)) = take_ident(after_kw) {
                    let resolved = if let Some(attr_path) = pending_mod_path.take() {
                        lookup.resolve_relative(&folder_of(rel_path), &attr_path)
                    } else {
                        resolve_rust_child_mod(rel_path, mod_name, lookup)
                    };
                    match resolved {
                        Some(target) => {
                            aliases.insert(
                                mod_name.to_owned(),
                                (Some(target.clone()), mod_name.to_owned()),
                            );
                            acc.add_resolved(target, "imports", &[mod_name], line_no, trimmed);
                        }
                        None => {
                            acc.add_unresolved(mod_name, "imports", &[mod_name], line_no, trimmed)
                        }
                    }
                }
                continue;
            }
            if !inside_fn && matches!(kw, "struct" | "enum" | "trait") {
                is_decl_header = true;
                if kw == "struct"
                    && let Some((sname, _)) = take_ident(after_kw)
                {
                    pending_scope = Some(RustBlockScope::Struct(sname.to_owned()));
                } else {
                    pending_scope = Some(RustBlockScope::Other);
                }
            }
        }
        pending_mod_path = None;

        // Track struct field types inside `struct Name { ... }`
        if let Some(ref sname) = inside_struct_name
            && !is_decl_header
            && !trimmed.starts_with('}')
        {
            let f_after_vis = strip_rust_visibility(trimmed);
            if let Some((field, after_f)) = take_ident(f_after_vis)
                && let Some(after_colon) = after_f.trim_start().strip_prefix(':')
                && !after_colon.starts_with(':')
            {
                let ty = unwrap_rust_wrapper_type(after_colon);
                if !ty.is_empty() {
                    struct_fields
                        .entry(sname.clone())
                        .or_default()
                        .insert(field.to_owned(), ty);
                }
            }
        }

        // 2. `impl Trait for Type` / `impl Type`
        if !inside_fn
            && let Some(after_impl) = after_vis.strip_prefix("impl")
            && after_impl.starts_with([' ', '<'])
        {
            is_decl_header = true;
            if let Some(target_ty) = parse_rust_impl_target(after_impl) {
                pending_scope = Some(RustBlockScope::Impl(target_ty));
            } else {
                pending_scope = Some(RustBlockScope::Other);
            }
            extract_rust_impl_deps(
                rel_path,
                &crate_src_root,
                after_impl,
                line_no,
                trimmed,
                lookup,
                &aliases,
                RUST_BUILTINS,
                acc,
            );
        }

        // Detect `fn` signatures (including `async`/`unsafe`/`const`/`extern`)
        let fn_stripped = strip_leading_words(after_vis, &["async", "unsafe", "const", "default"]);
        let is_fn_header = fn_stripped
            .strip_prefix("fn")
            .is_some_and(|r| r.starts_with(char::is_whitespace) || r.starts_with('<'));
        if is_fn_header && !inside_fn {
            pending_scope = Some(RustBlockScope::Fn);
        }
        if is_fn_header || matches!(pending_scope, Some(RustBlockScope::Fn)) {
            collect_rust_param_bindings(trimmed, &mut var_types);
        }

        // Collect local `let [mut] x [: Type] = ...` bindings
        let self_fields = current_impl_name.and_then(|n| struct_fields.get(n));
        if inside_fn {
            collect_rust_let_binding(trimmed, self_fields, &mut var_types);
        }

        // Track multi-line struct literal field writes
        if let Some((ref target, ref sname, open_d)) = struct_lit_stack.last().cloned()
            && depth == open_d
            && !trimmed.starts_with('}')
            && !trimmed.starts_with("..")
            && let Some((field, after_f)) = take_ident(trimmed)
            && after_f.trim_start().starts_with(':')
            && !after_f.trim_start().starts_with("::")
        {
            acc.add_resolved_members(
                target.clone(),
                "uses",
                &[sname.as_str(), field],
                &[DepMemberRef::basic(
                    field,
                    "field",
                    Some(sname.clone()),
                    "write",
                    line_no,
                )],
                line_no,
                trimmed,
            );
        }

        // 3. Qualified paths, dot member accesses, struct literals, direct calls & consts
        extract_rust_line_refs(
            rel_path,
            &crate_src_root,
            trimmed,
            line_no,
            lookup,
            &aliases,
            RUST_BUILTINS,
            self_fields,
            &var_types,
            acc,
        );

        // Update brace scope + multi-line struct literal tracking
        let open_lit_target = if !is_decl_header
            && !is_fn_header
            && trimmed.ends_with('{')
            && !trimmed.ends_with("{{")
        {
            let before_brace = trimmed.trim_end_matches('{').trim_end();
            let mut start = before_brace.len();
            let bytes = before_brace.as_bytes();
            while start > 0 && (is_ident_char(bytes[start - 1] as char) || bytes[start - 1] == b':')
            {
                start -= 1;
            }
            let cand = before_brace[start..].trim();
            if cand
                .rsplit("::")
                .next()
                .is_some_and(|s| s.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
                && !RUST_BUILTINS.contains(&cand)
            {
                resolve_rust_symbol_ref(
                    rel_path,
                    &crate_src_root,
                    cand,
                    lookup,
                    &aliases,
                    RUST_BUILTINS,
                )
            } else {
                None
            }
        } else {
            None
        };

        for ch in sanitized.chars() {
            match ch {
                '{' => {
                    depth += 1;
                    if let Some(sc) = pending_scope.take() {
                        scope_stack.push((sc, depth));
                    } else if let Some((ref t, ref s)) = open_lit_target {
                        struct_lit_stack.push((t.clone(), s.clone(), depth));
                    }
                }
                '}' => {
                    depth = depth.saturating_sub(1);
                    while struct_lit_stack.last().is_some_and(|(_, _, d)| depth < *d) {
                        struct_lit_stack.pop();
                    }
                    while scope_stack.last().is_some_and(|(_, d)| depth < *d) {
                        if let Some((sc, _)) = scope_stack.pop()
                            && matches!(sc, RustBlockScope::Fn)
                        {
                            var_types.clear();
                        }
                    }
                }
                ';' => {
                    if let Some(sc) = pending_scope.take()
                        && matches!(sc, RustBlockScope::Fn)
                    {
                        var_types.clear();
                    }
                }
                _ => {}
            }
        }
    }
}

fn strip_rust_visibility(mut s: &str) -> &str {
    s = s.trim_start();
    if let Some(rest) = s.strip_prefix("pub") {
        let trimmed = rest.trim_start();
        if let Some(paren_rest) = trimmed.strip_prefix('(')
            && let Some((_, after_paren)) = paren_rest.split_once(')')
        {
            return after_paren.trim_start();
        }
        if rest.is_empty() || rest.chars().next().is_some_and(char::is_whitespace) {
            return trimmed;
        }
    }
    s
}

fn rust_crate_src_root(rel_path: &str) -> String {
    if let Some((prefix, _)) = rel_path.rsplit_once("/src/") {
        return format!("{prefix}/src");
    }
    if rel_path.starts_with("src/") {
        return "src".to_owned();
    }
    if let Some((prefix, _)) = rel_path.rsplit_once("/tests/") {
        return format!("{prefix}/src");
    }
    if rel_path.starts_with("tests/") {
        return "src".to_owned();
    }
    String::new()
}

fn resolve_rust_child_mod(rel_path: &str, mod_name: &str, lookup: &KnownLookup) -> Option<String> {
    let dir = folder_of(rel_path);
    let stem = file_stem(rel_path);
    let sub_dir = if matches!(stem, "lib" | "main" | "mod") {
        dir.clone()
    } else if dir.is_empty() {
        stem.to_owned()
    } else {
        format!("{dir}/{stem}")
    };

    for base in [&sub_dir, &dir] {
        for cand in [
            join_rel(base, &format!("{mod_name}.rs")),
            join_rel(base, &format!("{mod_name}/mod.rs")),
        ] {
            if let Some(found) = lookup.resolve_exact_or_ci(&cand) {
                return Some(found);
            }
        }
    }
    None
}

fn process_rust_use_stmt(
    rel_path: &str,
    crate_src_root: &str,
    stmt: &str,
    line_no: u32,
    lookup: &KnownLookup,
    aliases: &mut BTreeMap<String, (Option<String>, String)>,
    acc: &mut DepAccumulator,
) {
    let after_vis = strip_rust_visibility(stmt);
    let Some(body) = after_vis.strip_prefix("use") else {
        return;
    };
    let body = body.split(';').next().unwrap_or("").trim();
    if body.is_empty() {
        return;
    }

    let mut expanded = Vec::new();
    expand_rust_use_tree(body, &[], &mut expanded);

    for (segs, alias_opt) in expanded {
        if segs.is_empty() {
            continue;
        }
        let last = segs.last().map(String::as_str).unwrap_or("");
        let effective_segs: Vec<&str> = if (last == "self" || last == "*") && segs.len() > 1 {
            segs[..segs.len() - 1].iter().map(String::as_str).collect()
        } else {
            segs.iter().map(String::as_str).collect()
        };

        if let Some((target, split_at)) =
            resolve_rust_path_segments(rel_path, crate_src_root, &effective_segs, lookup)
        {
            let mut syms: Vec<&str> = Vec::new();
            for s in &effective_segs[split_at..] {
                if *s != "self" && *s != "*" {
                    syms.push(*s);
                }
            }
            if syms.is_empty() && last != "*" && last != "self" {
                syms.push(last);
            }
            let local_name = alias_opt.as_deref().unwrap_or(last);
            if !local_name.is_empty() && local_name != "*" && local_name != "self" {
                let orig_sym = if last == "self" {
                    effective_segs.last().copied().unwrap_or(local_name)
                } else {
                    last
                };
                aliases.insert(
                    local_name.to_owned(),
                    (Some(target.clone()), orig_sym.to_owned()),
                );
            }
            acc.add_resolved(target, "imports", &syms, line_no, stmt);
        } else {
            let unres_target = if effective_segs.len() >= 2 {
                format!("{}::{}", effective_segs[0], effective_segs[1])
            } else {
                effective_segs.join("::")
            };
            let syms: Vec<&str> = if last != "*" && last != "self" {
                vec![last]
            } else {
                Vec::new()
            };
            acc.add_unresolved(&unres_target, "imports", &syms, line_no, stmt);
        }
    }
}

fn expand_rust_use_tree(
    tree: &str,
    prefix: &[String],
    out: &mut Vec<(Vec<String>, Option<String>)>,
) {
    let trimmed = tree.trim();
    if trimmed.is_empty() {
        return;
    }
    if let Some((before_brace, after_brace)) = split_top_level_brace(trimmed) {
        let head = before_brace.trim().trim_end_matches(':');
        let mut next_prefix = prefix.to_vec();
        if !head.is_empty() {
            for seg in head.split("::").map(str::trim).filter(|s| !s.is_empty()) {
                next_prefix.push(seg.to_owned());
            }
        }
        let inner = after_brace
            .rsplit_once('}')
            .map_or(after_brace, |(inside, _)| inside);
        for item in split_top_level_commas(inner) {
            expand_rust_use_tree(&item, &next_prefix, out);
        }
        return;
    }

    let (path_part, alias_part) = if let Some((lhs, rhs)) = split_as_alias(trimmed) {
        (lhs.trim(), Some(rhs.trim().to_owned()))
    } else {
        (trimmed, None)
    };

    let mut full = prefix.to_vec();
    for seg in path_part
        .split("::")
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        full.push(seg.to_owned());
    }
    if !full.is_empty() {
        out.push((full, alias_part));
    }
}

fn split_top_level_brace(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    for (idx, ch) in s.char_indices() {
        match ch {
            '{' if depth == 0 => return Some((&s[..idx], &s[idx + 1..])),
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

fn split_top_level_commas(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (idx, ch) in s.char_indices() {
        match ch {
            '{' | '(' | '<' => depth += 1,
            '}' | ')' | '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let item = s[start..idx].trim();
                if !item.is_empty() {
                    out.push(item.to_owned());
                }
                start = idx + 1;
            }
            _ => {}
        }
    }
    let tail = s[start..].trim();
    if !tail.is_empty() {
        out.push(tail.to_owned());
    }
    out
}

fn resolve_rust_path_segments(
    rel_path: &str,
    crate_src_root: &str,
    segs: &[&str],
    lookup: &KnownLookup,
) -> Option<(String, usize)> {
    if segs.is_empty() {
        return None;
    }
    let (base_dir, start_idx) = match segs[0] {
        "crate" => (crate_src_root.to_owned(), 1usize),
        "self" => {
            let dir = folder_of(rel_path);
            let stem = file_stem(rel_path);
            let base = if matches!(stem, "lib" | "main" | "mod") {
                dir
            } else {
                join_rel(&dir, stem)
            };
            (base, 1usize)
        }
        "super" => {
            let mut cur = folder_of(rel_path);
            let stem = file_stem(rel_path);
            let mut idx = 0usize;
            while segs.get(idx) == Some(&"super") {
                if idx == 0 && !matches!(stem, "lib" | "main" | "mod") {
                    // In `src/a/b.rs`, first `super` is `src/a` (folder_of).
                } else {
                    cur = folder_of(&cur);
                }
                idx += 1;
            }
            (cur, idx)
        }
        pkg => {
            // Check workspace crate `<pkg_with_hyphens>/src/...`
            let pkg_hyphen = pkg.replace('_', "-");
            let rest = &segs[1..];
            for k in (1..=rest.len()).rev() {
                let rel_mod = rest[..k].join("/");
                for pkg_name in [&pkg_hyphen, &pkg.to_owned()] {
                    for suffix in [
                        format!("{pkg_name}/src/{rel_mod}.rs"),
                        format!("{pkg_name}/src/{rel_mod}/mod.rs"),
                    ] {
                        if let Some(found) = lookup.resolve_suffix(&suffix, Some(RUST_EXTS)) {
                            return Some((found, 1 + k));
                        }
                    }
                }
            }
            if rest.is_empty() {
                return None;
            }
            for suffix in [
                format!("{pkg_hyphen}/src/lib.rs"),
                format!("{pkg}/src/lib.rs"),
            ] {
                if let Some(found) = lookup.resolve_suffix(&suffix, Some(RUST_EXTS)) {
                    return Some((found, 1));
                }
            }
            return None;
        }
    };

    let rest = &segs[start_idx..];
    for k in (1..=rest.len()).rev() {
        let mod_rel = rest[..k].join("/");
        for cand in [
            join_rel(&base_dir, &format!("{mod_rel}.rs")),
            join_rel(&base_dir, &format!("{mod_rel}/mod.rs")),
        ] {
            if let Some(found) = lookup.resolve_exact_or_ci(&cand) {
                return Some((found, start_idx + k));
            }
        }
    }

    // `use super::*;` or `use crate;` -> lib.rs / mod.rs of `base_dir`
    for fallback in [
        join_rel(&base_dir, "mod.rs"),
        join_rel(&base_dir, "lib.rs"),
        format!("{base_dir}.rs"),
    ] {
        if let Some(found) = lookup.resolve_exact_or_ci(&fallback) {
            return Some((found, start_idx));
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn extract_rust_impl_deps(
    rel_path: &str,
    crate_src_root: &str,
    after_impl: &str,
    line_no: u32,
    raw_line: &str,
    lookup: &KnownLookup,
    aliases: &BTreeMap<String, (Option<String>, String)>,
    builtins: &[&str],
    acc: &mut DepAccumulator,
) {
    let head = after_impl
        .split(['{', ';'])
        .next()
        .unwrap_or("")
        .split(" where ")
        .next()
        .unwrap_or("");
    let Some((trait_part, type_part)) = head.split_once(" for ") else {
        return;
    };
    let trait_clean = strip_generics_and_refs(trait_part);
    if let Some((target, sym)) = resolve_rust_symbol_ref(
        rel_path,
        crate_src_root,
        trait_clean,
        lookup,
        aliases,
        builtins,
    ) {
        acc.add_resolved(target, "implements", &[&sym], line_no, raw_line);
    }
    let type_clean = strip_generics_and_refs(type_part);
    if let Some((target, sym)) = resolve_rust_symbol_ref(
        rel_path,
        crate_src_root,
        type_clean,
        lookup,
        aliases,
        builtins,
    ) {
        acc.add_resolved(target, "uses", &[&sym], line_no, raw_line);
    }
}

fn is_rust_call_after(s: &str) -> bool {
    let t = s.trim_start();
    if t.starts_with('(') {
        return true;
    }
    if let Some(after) = t.strip_prefix("::<") {
        let mut depth = 1usize;
        for (idx, ch) in after.char_indices() {
            match ch {
                '<' => depth += 1,
                '>' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return after[idx + 1..].trim_start().starts_with('(');
                    }
                }
                _ => {}
            }
        }
    }
    false
}

fn is_rust_std_trait_method(name: &str) -> bool {
    matches!(
        name,
        "clone"
            | "into"
            | "to_owned"
            | "to_string"
            | "unwrap"
            | "unwrap_or"
            | "unwrap_or_else"
            | "expect"
            | "is_some"
            | "is_none"
            | "is_ok"
            | "is_err"
            | "as_ref"
            | "as_mut"
            | "fmt"
            | "eq"
            | "cmp"
            | "partial_cmp"
            | "hash"
    )
}

/// A path that names a constant reads it; any other path keeps its kind.
fn path_ref_kind(kind: &'static str, members: &[DepMemberRef]) -> &'static str {
    match members.first().map(|member| member.access.as_str()) {
        Some("const") if kind == "uses" => "reads",
        _ => kind,
    }
}

fn build_rust_path_member_ref(syms: &[&str], is_call: bool, line_no: u32) -> Option<DepMemberRef> {
    match syms.len() {
        0 => None,
        1 => {
            let mem = syms[0];
            if is_call {
                let mk = if mem.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                    "method"
                } else {
                    "function"
                };
                Some(DepMemberRef::basic(mem, mk, None, "call", line_no))
            } else if is_upper_snake(mem) {
                Some(DepMemberRef::basic(mem, "const", None, "const", line_no))
            } else {
                None
            }
        }
        _ => {
            let parent = syms[syms.len() - 2];
            let mem = syms[syms.len() - 1];
            let parent_is_type = parent
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_uppercase());
            let parent_opt = parent_is_type.then(|| parent.to_owned());
            if is_call {
                let mk = if parent_is_type { "method" } else { "function" };
                Some(DepMemberRef::basic(mem, mk, parent_opt, "call", line_no))
            } else if is_upper_snake(mem) {
                Some(DepMemberRef::basic(
                    mem, "const", parent_opt, "const", line_no,
                ))
            } else {
                None
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn extract_rust_line_refs(
    rel_path: &str,
    crate_src_root: &str,
    line: &str,
    line_no: u32,
    lookup: &KnownLookup,
    aliases: &BTreeMap<String, (Option<String>, String)>,
    builtins: &[&str],
    struct_fields_for_self: Option<&BTreeMap<String, String>>,
    var_types: &BTreeMap<String, String>,
    acc: &mut DepAccumulator,
) {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    while i < len {
        let ch = bytes[i] as char;
        if is_ident_start(ch) && (i == 0 || !is_ident_char(bytes[i - 1] as char)) {
            let mut end = i + 1;
            while end < len && is_ident_char(bytes[end] as char) {
                end += 1;
            }
            let first = &line[i..end];
            let preceded_by_dot = line[..i].trim_end().ends_with('.');
            let preceded_by_colons = line[..i].trim_end().ends_with("::");

            if line[end..].starts_with("::") {
                let mut segs = vec![first];
                let mut cursor = end;
                while line[cursor..].starts_with("::") {
                    let after = &line[cursor + 2..];
                    if let Some((seg, rest)) = take_ident(after)
                        && after.starts_with(seg)
                    {
                        segs.push(seg);
                        cursor = line.len() - rest.len();
                    } else {
                        break;
                    }
                }
                let after_path = &line[cursor..];
                let is_call = is_rust_call_after(after_path);
                let kind = if is_call { "calls" } else { "uses" };
                if matches!(segs[0], "crate" | "super" | "self")
                    && let Some((target, split_at)) =
                        resolve_rust_path_segments(rel_path, crate_src_root, &segs, lookup)
                {
                    let syms: Vec<&str> = if split_at < segs.len() {
                        segs[split_at..].to_vec()
                    } else {
                        vec![segs.last().copied().unwrap_or("")]
                    };
                    let mref = build_rust_path_member_ref(&syms, is_call, line_no);
                    let members: Vec<DepMemberRef> = mref.into_iter().collect();
                    let kind = path_ref_kind(kind, &members);
                    acc.add_resolved_members(target, kind, &syms, &members, line_no, line);
                } else if let Some((Some(target), orig_sym)) = aliases.get(segs[0]) {
                    let mut syms = vec![orig_sym.as_str()];
                    if let Some(member) = segs.get(1) {
                        syms.push(*member);
                    }
                    let mref = build_rust_path_member_ref(&syms, is_call, line_no);
                    let members: Vec<DepMemberRef> = mref.into_iter().collect();
                    let kind = path_ref_kind(kind, &members);
                    acc.add_resolved_members(target.clone(), kind, &syms, &members, line_no, line);
                } else if !builtins.contains(&segs[0]) {
                    let dir = folder_of(rel_path);
                    let target_opt = lookup
                        .resolve_stem_in_dir(&dir, segs[0], RUST_EXTS)
                        .or_else(|| {
                            lookup.resolve_unambiguous_stem(
                                segs[0],
                                RUST_EXTS,
                                (!crate_src_root.is_empty()).then_some(crate_src_root),
                            )
                        });
                    if let Some(target) = target_opt {
                        let mut syms = vec![segs[0]];
                        if let Some(member) = segs.get(1) {
                            syms.push(*member);
                        }
                        let mref = build_rust_path_member_ref(&syms, is_call, line_no);
                        let members: Vec<DepMemberRef> = mref.into_iter().collect();
                        let kind = path_ref_kind(kind, &members);
                        acc.add_resolved_members(target, kind, &syms, &members, line_no, line);
                    }
                }
                i = cursor;
                continue;
            }

            let after_first = &line[end..];
            let trimmed_after = after_first.trim_start();

            // Inline single-line struct literal `TypeName { field: val, ... }`
            if !preceded_by_dot
                && !preceded_by_colons
                && first.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && !builtins.contains(&first)
                && let Some(inside_brace) = trimmed_after.strip_prefix('{')
                && let Some(close_rel) = inside_brace.find('}')
                && !line[..i].trim_end().ends_with("struct")
                && !line[..i].trim_end().ends_with("impl")
                && let Some((target, canon)) = resolve_rust_symbol_ref(
                    rel_path,
                    crate_src_root,
                    first,
                    lookup,
                    aliases,
                    builtins,
                )
            {
                let body = &inside_brace[..close_rel];
                let mut members = Vec::new();
                for part in body.split(',') {
                    let pt = part.trim();
                    if pt.starts_with("..") {
                        continue;
                    }
                    if let Some((field, after_f)) = take_ident(pt)
                        && after_f.trim_start().starts_with(':')
                        && !after_f.trim_start().starts_with("::")
                    {
                        members.push(DepMemberRef::basic(
                            field,
                            "field",
                            Some(canon.clone()),
                            "write",
                            line_no,
                        ));
                    }
                }
                acc.add_resolved_members(
                    target,
                    "uses",
                    &[canon.as_str()],
                    &members,
                    line_no,
                    line,
                );
            }

            // `self.field.member` -> resolve `self.field`'s struct type
            if !preceded_by_dot
                && !preceded_by_colons
                && first == "self"
                && let Some(after_dot1) = trimmed_after.strip_prefix('.')
                && let Some((field, after_field)) = take_ident(after_dot1.trim_start())
                && let Some(after_dot2) = after_field.trim_start().strip_prefix('.')
                && let Some((mem2, after_mem2)) = take_ident(after_dot2.trim_start())
                && let Some(owner_ty) = struct_fields_for_self.and_then(|m| m.get(field))
                && let Some((target, canon)) = resolve_rust_symbol_ref(
                    rel_path,
                    crate_src_root,
                    owner_ty,
                    lookup,
                    aliases,
                    builtins,
                )
            {
                let is_call = is_rust_call_after(after_mem2);
                if !(is_call && is_rust_std_trait_method(mem2)) {
                    let (dep_kind, mem_kind, access) =
                        classify_member_access(mem2, after_mem2, false, is_call, "field", false);
                    acc.add_resolved_members(
                        target,
                        dep_kind,
                        &[canon.as_str(), mem2],
                        &[DepMemberRef::basic(
                            mem2,
                            mem_kind,
                            Some(canon.clone()),
                            access,
                            line_no,
                        )],
                        line_no,
                        line,
                    );
                }
            } else if !preceded_by_dot
                && !preceded_by_colons
                && let Some(owner_ty) = var_types.get(first)
                && let Some(after_dot) = trimmed_after.strip_prefix('.')
                && let Some((mem, after_mem)) = take_ident(after_dot.trim_start())
                && let Some((target, canon)) = resolve_rust_symbol_ref(
                    rel_path,
                    crate_src_root,
                    owner_ty,
                    lookup,
                    aliases,
                    builtins,
                )
            {
                let is_call = is_rust_call_after(after_mem);
                if !(is_call && is_rust_std_trait_method(mem)) {
                    let (dep_kind, mem_kind, access) =
                        classify_member_access(mem, after_mem, false, is_call, "field", false);
                    acc.add_resolved_members(
                        target,
                        dep_kind,
                        &[canon.as_str(), mem],
                        &[DepMemberRef::basic(
                            mem,
                            mem_kind,
                            Some(canon.clone()),
                            access,
                            line_no,
                        )],
                        line_no,
                        line,
                    );
                }
            } else if !preceded_by_dot
                && !preceded_by_colons
                && let Some((Some(target), orig_sym)) = aliases.get(first)
            {
                // Direct call or imported constant (`verify_token(...)` or `DEFAULT_TTL`)
                if is_rust_call_after(trimmed_after) {
                    let mk = if orig_sym
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_uppercase())
                    {
                        "method"
                    } else {
                        "function"
                    };
                    acc.add_resolved_members(
                        target.clone(),
                        "calls",
                        &[orig_sym],
                        &[DepMemberRef::basic(
                            orig_sym.as_str(),
                            mk,
                            None,
                            "call",
                            line_no,
                        )],
                        line_no,
                        line,
                    );
                } else if !trimmed_after.starts_with('.') && is_upper_snake(orig_sym) {
                    acc.add_resolved_members(
                        target.clone(),
                        "uses",
                        &[orig_sym],
                        &[DepMemberRef::basic(
                            orig_sym.as_str(),
                            "const",
                            None,
                            "const",
                            line_no,
                        )],
                        line_no,
                        line,
                    );
                }
            }
            i = end;
            continue;
        }
        i += 1;
    }
}

fn resolve_rust_symbol_ref(
    rel_path: &str,
    crate_src_root: &str,
    expr: &str,
    lookup: &KnownLookup,
    aliases: &BTreeMap<String, (Option<String>, String)>,
    builtins: &[&str],
) -> Option<(String, String)> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains("::") {
        let segs: Vec<&str> = trimmed
            .split("::")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if matches!(segs.first().copied(), Some("crate" | "super" | "self"))
            && let Some((target, _)) =
                resolve_rust_path_segments(rel_path, crate_src_root, &segs, lookup)
        {
            return Some((target, segs.last().copied().unwrap_or("").to_owned()));
        }
        if let Some(head) = segs.first()
            && let Some((Some(target), _)) = aliases.get(*head)
        {
            return Some((
                target.clone(),
                segs.last().copied().unwrap_or("").to_owned(),
            ));
        }
        return None;
    }
    if let Some((Some(target), orig_sym)) = aliases.get(trimmed) {
        return Some((target.clone(), orig_sym.clone()));
    }
    if builtins.contains(&trimmed) {
        return None;
    }
    let dir = folder_of(rel_path);
    let target = lookup
        .resolve_stem_in_dir(&dir, trimmed, RUST_EXTS)
        .or_else(|| {
            lookup.resolve_unambiguous_stem(
                trimmed,
                RUST_EXTS,
                (!crate_src_root.is_empty()).then_some(crate_src_root),
            )
        })?;
    Some((target, trimmed.to_owned()))
}

fn strip_generics_and_refs(s: &str) -> &str {
    let mut trimmed = s.trim();
    while let Some(rest) = trimmed.strip_prefix('&') {
        trimmed = rest.trim_start();
        if trimmed.starts_with('\'')
            && let Some((_, after)) = trimmed.split_once(char::is_whitespace)
        {
            trimmed = after.trim_start();
        }
        if let Some(after) = trimmed.strip_prefix("mut ") {
            trimmed = after.trim_start();
        }
    }
    trimmed.split('<').next().unwrap_or(trimmed).trim()
}

// =========================================================================
// TS/JS Dependency Extraction & Resolution
// =========================================================================

fn resolve_ts_owner_type(
    ty_name: &str,
    dir: &str,
    lookup: &KnownLookup,
    aliases: &BTreeMap<String, (Option<String>, String)>,
) -> Option<(String, String)> {
    let clean = ty_name.split('<').next().unwrap_or(ty_name).trim();
    if clean.is_empty() {
        return None;
    }
    if let Some((Some(target), orig)) = aliases.get(clean) {
        let canon = if orig == "*" { clean } else { orig.as_str() };
        return Some((target.clone(), canon.to_owned()));
    }
    lookup
        .resolve_stem_in_dir(dir, clean, TS_JS_EXTS)
        .map(|target| (target, clean.to_owned()))
}

fn collect_ts_param_bindings(
    line: &str,
    in_ctor_sig: bool,
    this_props: &mut BTreeMap<String, String>,
    var_types: &mut BTreeMap<String, String>,
) {
    let slice = line.split_once('(').map(|(_, after)| after).unwrap_or(line);
    let params_part = slice.split(')').next().unwrap_or(slice);
    for raw_param in params_part.split(',') {
        let trimmed = raw_param.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((lhs, rhs)) = trimmed.split_once(':') else {
            continue;
        };
        let has_ctor_vis = in_ctor_sig
            && (contains_word_ascii(lhs, "public")
                || contains_word_ascii(lhs, "protected")
                || contains_word_ascii(lhs, "private")
                || contains_word_ascii(lhs, "readonly"));
        let param_stripped = strip_leading_words(
            lhs,
            &["public", "protected", "private", "readonly", "override"],
        );
        if let Some((param_name, rest)) = take_ts_js_member_ident(param_stripped)
            && (rest.trim().is_empty() || rest.trim() == "?")
        {
            let ty = rhs.split(['=', '|', '<', '&']).next().unwrap_or("").trim();
            if let Some((ty_ident, _)) = take_ident(ty)
                && ty_ident
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_uppercase())
            {
                if has_ctor_vis {
                    this_props.insert(param_name.to_owned(), ty_ident.to_owned());
                }
                var_types.insert(param_name.to_owned(), ty_ident.to_owned());
            }
        }
    }
}

fn update_ts_var_bindings(
    san_trimmed: &str,
    inside_class: bool,
    in_fn: bool,
    in_ctor_sig: bool,
    this_props: &mut BTreeMap<String, String>,
    var_types: &mut BTreeMap<String, String>,
) {
    // Constructor / function typed parameters
    if san_trimmed.contains(':') && (in_fn || san_trimmed.contains('(')) {
        collect_ts_param_bindings(san_trimmed, in_ctor_sig, this_props, var_types);
    }

    // Class field declaration (`private signer: Signer;`, `#signer: Signer;`, `signer = new Signer();`)
    if inside_class && !in_fn && !in_ctor_sig {
        let stripped = strip_leading_words(
            san_trimmed,
            &[
                "public",
                "protected",
                "private",
                "readonly",
                "static",
                "override",
                "declare",
            ],
        );
        if let Some((prop, after_prop)) = take_ts_js_member_ident(stripped) {
            let after_opt = after_prop
                .trim_start()
                .trim_start_matches(['?', '!'])
                .trim_start();
            if let Some(after_colon) = after_opt.strip_prefix(':') {
                let ty_raw = after_colon
                    .split(['=', ';', '<', '|'])
                    .next()
                    .unwrap_or("")
                    .trim();
                if let Some((ty_ident, _)) = take_ident(ty_raw)
                    && ty_ident
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_uppercase())
                {
                    this_props.insert(prop.to_owned(), ty_ident.to_owned());
                }
            }
        }
    }

    // `const|let|var x [: Type] = ...`
    let after_decl = san_trimmed
        .strip_prefix("export ")
        .unwrap_or(san_trimmed)
        .trim_start();
    for kw in ["const ", "let ", "var "] {
        if let Some(rest) = after_decl.strip_prefix(kw)
            && let Some((var_name, after_var)) = take_ident(rest.trim_start())
        {
            let r = after_var.trim_start();
            if let Some(after_colon) = r.strip_prefix(':') {
                let ty_raw = after_colon
                    .split(['=', ';', '<', '|'])
                    .next()
                    .unwrap_or("")
                    .trim();
                if let Some((ty_ident, _)) = take_ident(ty_raw)
                    && ty_ident
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_uppercase())
                {
                    var_types.insert(var_name.to_owned(), ty_ident.to_owned());
                    return;
                }
            }
            if let Some((_, rhs)) = split_simple_assign(san_trimmed) {
                let rhs_c = rhs.trim_end_matches(';').trim();
                if let Some(after_new) = rhs_c.strip_prefix("new")
                    && after_new.starts_with(char::is_whitespace)
                    && let Some((cls, _)) = take_ident(after_new.trim_start())
                {
                    var_types.insert(var_name.to_owned(), cls.to_owned());
                } else if let Some(after_this) = rhs_c.strip_prefix("this.")
                    && let Some((prop, after_p)) = take_ts_js_member_ident(after_this)
                    && after_p.trim().is_empty()
                    && let Some(ty) = this_props.get(prop)
                {
                    var_types.insert(var_name.to_owned(), ty.clone());
                }
            }
            return;
        }
    }

    // `this.prop = new Foo(...)` or `this.prop = param` or `x = new Foo(...)`
    if let Some((lhs, rhs)) = split_simple_assign(san_trimmed) {
        let rhs_c = rhs.trim_end_matches(';').trim();
        let rhs_ty = if let Some(after_new) = rhs_c.strip_prefix("new")
            && after_new.starts_with(char::is_whitespace)
        {
            take_ident(after_new.trim_start()).map(|(c, _)| c.to_owned())
        } else if let Some((var_rhs, rest_rhs)) = take_ident(rhs_c)
            && rest_rhs.trim().is_empty()
        {
            var_types.get(var_rhs).cloned()
        } else {
            None
        };
        if let Some(ty) = rhs_ty {
            if let Some(after_this) = lhs.trim().strip_prefix("this.")
                && let Some((prop, rest_p)) = take_ts_js_member_ident(after_this)
                && rest_p.trim().is_empty()
            {
                this_props.insert(prop.to_owned(), ty);
            } else if let Some((var_lhs, rest_l)) = take_ident(lhs.trim())
                && rest_l.trim().is_empty()
            {
                var_types.insert(var_lhs.to_owned(), ty);
            }
        }
    }
}

fn extract_ts_js_deps(
    rel_path: &str,
    source: &str,
    lookup: &KnownLookup,
    acc: &mut DepAccumulator,
) {
    let dir = folder_of(rel_path);
    let mut aliases: BTreeMap<String, (Option<String>, String)> = BTreeMap::new();
    let mut this_props: BTreeMap<String, String> = BTreeMap::new();
    let mut var_types: BTreeMap<String, String> = BTreeMap::new();
    let mut in_block_comment = false;
    let mut import_buf = String::new();
    let mut import_line = 0u32;
    let mut depth = 0usize;
    let mut scope_stack: Vec<(bool, usize)> = Vec::new(); // (is_class, open_depth)
    let mut pending_scope: Option<bool> = None;
    let mut in_ctor_sig = false;

    for (idx, raw_line) in source.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let comments_stripped = strip_comments_keep_strings(raw_line, Language::TsJs);
        let sanitized = sanitize_c_like_line(raw_line, &mut in_block_comment, Language::TsJs);
        let raw_trimmed = comments_stripped.trim();
        let san_trimmed = sanitized.trim();

        if raw_trimmed.is_empty() && import_buf.is_empty() {
            continue;
        }

        // 1. Multi-line `import ...` or `export ... from '...'`
        if !import_buf.is_empty() {
            import_buf.push(' ');
            import_buf.push_str(raw_trimmed);
            if ts_import_complete(&import_buf) {
                let stmt = std::mem::take(&mut import_buf);
                process_ts_import_stmt(&dir, &stmt, import_line, lookup, &mut aliases, acc);
            }
            continue;
        }

        let starts_import = starts_word_at(raw_trimmed, 0, "import")
            && !raw_trimmed[6..].trim_start().starts_with('(');
        let starts_reexport = starts_word_at(raw_trimmed, 0, "export")
            && (raw_trimmed.contains('{') || raw_trimmed.contains('*'))
            && !starts_word_at(raw_trimmed[6..].trim_start(), 0, "class")
            && !starts_word_at(raw_trimmed[6..].trim_start(), 0, "function")
            && !starts_word_at(raw_trimmed[6..].trim_start(), 0, "const")
            && !starts_word_at(raw_trimmed[6..].trim_start(), 0, "let")
            && !starts_word_at(raw_trimmed[6..].trim_start(), 0, "var")
            && !starts_word_at(raw_trimmed[6..].trim_start(), 0, "interface")
            && !starts_word_at(raw_trimmed[6..].trim_start(), 0, "type")
            && !starts_word_at(raw_trimmed[6..].trim_start(), 0, "enum")
            && !starts_word_at(raw_trimmed[6..].trim_start(), 0, "default")
            && !starts_word_at(raw_trimmed[6..].trim_start(), 0, "async");

        if starts_import || (starts_reexport && raw_trimmed.contains("from")) {
            if ts_import_complete(raw_trimmed) {
                process_ts_import_stmt(&dir, raw_trimmed, line_no, lookup, &mut aliases, acc);
            } else {
                import_buf = raw_trimmed.to_owned();
                import_line = line_no;
            }
            continue;
        }
        if starts_reexport && !raw_trimmed.contains([';', '=']) {
            import_buf = raw_trimmed.to_owned();
            import_line = line_no;
            continue;
        }

        // 2. Dynamic `import('...')` and `require('...')`
        extract_ts_call_imports(&dir, raw_trimmed, line_no, lookup, acc);

        let inside_class = scope_stack
            .last()
            .is_some_and(|(is_c, open_d)| *is_c && *open_d == depth);
        let inside_fn = scope_stack.iter().any(|(is_c, _)| !*is_c);

        if !inside_fn && !san_trimmed.is_empty() {
            let hdr = strip_leading_words(
                san_trimmed,
                &[
                    "export",
                    "default",
                    "declare",
                    "abstract",
                    "async",
                    "public",
                    "protected",
                    "private",
                    "static",
                    "override",
                ],
            );
            if starts_word_at(hdr, 0, "class") {
                pending_scope = Some(true);
            } else if starts_word_at(hdr, 0, "function") {
                pending_scope = Some(false);
            } else if inside_class
                && let Some((mname, after_m)) = take_ts_js_member_ident(hdr)
                && after_m.trim_start().starts_with('(')
                && !matches!(mname, "if" | "for" | "while" | "switch" | "catch")
            {
                pending_scope = Some(false);
                if mname == "constructor" {
                    in_ctor_sig = true;
                }
            }
        }

        // 3. Variable/property type bindings + inheritance/member references
        if !san_trimmed.is_empty() {
            update_ts_var_bindings(
                san_trimmed,
                inside_class,
                inside_fn || matches!(pending_scope, Some(false)),
                in_ctor_sig,
                &mut this_props,
                &mut var_types,
            );
            extract_ts_line_refs(
                san_trimmed,
                &dir,
                line_no,
                lookup,
                &aliases,
                &this_props,
                &var_types,
                acc,
            );
        }

        for ch in sanitized.chars() {
            match ch {
                '{' => {
                    depth += 1;
                    if let Some(is_c) = pending_scope.take() {
                        in_ctor_sig = false;
                        scope_stack.push((is_c, depth));
                    }
                }
                '}' => {
                    depth = depth.saturating_sub(1);
                    while scope_stack.last().is_some_and(|(_, d)| depth < *d) {
                        if let Some((is_c, _)) = scope_stack.pop() {
                            if is_c {
                                if !scope_stack.iter().any(|(c, _)| *c) {
                                    this_props.clear();
                                }
                            } else {
                                var_types.clear();
                            }
                        }
                    }
                }
                ';' if matches!(pending_scope, Some(false)) => {
                    pending_scope = None;
                    in_ctor_sig = false;
                    var_types.clear();
                }
                _ => {}
            }
        }
    }
}

fn ts_import_complete(stmt: &str) -> bool {
    if stmt.contains("from") && extract_last_quoted_string(stmt).is_some() {
        return true;
    }
    let after_import = stmt
        .trim_start()
        .strip_prefix("import")
        .unwrap_or("")
        .trim_start();
    if after_import.starts_with(['\'', '"']) && extract_first_quoted_string(after_import).is_some()
    {
        return true;
    }
    stmt.contains(';') && !stmt.contains('{')
}

fn process_ts_import_stmt(
    dir: &str,
    stmt: &str,
    line_no: u32,
    lookup: &KnownLookup,
    aliases: &mut BTreeMap<String, (Option<String>, String)>,
    acc: &mut DepAccumulator,
) {
    let Some(spec) = extract_last_quoted_string(stmt) else {
        return;
    };
    let resolved = resolve_ts_js_spec(dir, spec, lookup);
    let mut syms: Vec<String> = Vec::new();

    if let Some((head, _)) = stmt.rsplit_once("from") {
        let mut clause = head
            .trim()
            .trim_start_matches("import")
            .trim_start_matches("export")
            .trim();
        if let Some(rest) = clause.strip_prefix("type ") {
            clause = rest.trim();
        }
        // Extract default import before `{` or `,`
        if let Some((before_brace, brace_part)) = clause.split_once('{') {
            let default_part = before_brace.trim().trim_end_matches(',').trim();
            if !default_part.is_empty() && !default_part.starts_with('*') {
                if let Some((name, _)) = take_ident(default_part) {
                    syms.push(name.to_owned());
                    aliases.insert(name.to_owned(), (resolved.clone(), name.to_owned()));
                }
            } else if let Some(after_star) = default_part.strip_prefix('*')
                && let Some((_, ns)) = split_as_alias(&format!("* {after_star}"))
                && let Some((ns_ident, _)) = take_ident(ns)
            {
                aliases.insert(ns_ident.to_owned(), (resolved.clone(), "*".to_owned()));
            }
            let inner = brace_part.rsplit_once('}').map_or(brace_part, |(i, _)| i);
            for item in inner.split(',') {
                let mut item = item.trim();
                if let Some(rest) = item.strip_prefix("type ") {
                    item = rest.trim();
                }
                if item.is_empty() {
                    continue;
                }
                if let Some((orig, alias)) = split_as_alias(item) {
                    if let Some((orig_id, _)) = take_ident(orig) {
                        syms.push(orig_id.to_owned());
                        if let Some((alias_id, _)) = take_ident(alias) {
                            aliases.insert(
                                alias_id.to_owned(),
                                (resolved.clone(), orig_id.to_owned()),
                            );
                        }
                    }
                } else if let Some((id, _)) = take_ident(item) {
                    syms.push(id.to_owned());
                    aliases.insert(id.to_owned(), (resolved.clone(), id.to_owned()));
                }
            }
        } else if let Some((_, ns)) = split_as_alias(clause) {
            if let Some((ns_ident, _)) = take_ident(ns) {
                aliases.insert(ns_ident.to_owned(), (resolved.clone(), "*".to_owned()));
            }
        } else if let Some((default_id, _)) = take_ident(clause) {
            syms.push(default_id.to_owned());
            aliases.insert(
                default_id.to_owned(),
                (resolved.clone(), default_id.to_owned()),
            );
        }
    }

    let sym_refs: Vec<&str> = syms.iter().map(String::as_str).collect();
    match resolved {
        Some(target) => acc.add_resolved(target, "imports", &sym_refs, line_no, stmt),
        None => acc.add_unresolved(spec, "imports", &sym_refs, line_no, stmt),
    }
}

fn extract_ts_call_imports(
    dir: &str,
    line: &str,
    line_no: u32,
    lookup: &KnownLookup,
    acc: &mut DepAccumulator,
) {
    for kw in ["require", "import"] {
        let mut search_from = 0usize;
        while let Some(rel_pos) = find_keyword(&line[search_from..], kw) {
            let pos = search_from + rel_pos;
            let after_kw = line[pos + kw.len()..].trim_start();
            if let Some(after_paren) = after_kw.strip_prefix('(')
                && let Some(spec) = extract_first_quoted_string(after_paren)
            {
                let sym = file_stem(spec);
                match resolve_ts_js_spec(dir, spec, lookup) {
                    Some(target) => acc.add_resolved(target, "imports", &[sym], line_no, line),
                    None => acc.add_unresolved(spec, "imports", &[sym], line_no, line),
                }
            }
            search_from = pos + kw.len();
        }
    }
}

fn strip_ts_dot_op(s: &str) -> Option<&str> {
    let trimmed = s.trim_start();
    trimmed
        .strip_prefix("?.")
        .or_else(|| trimmed.strip_prefix('.'))
        .map(str::trim_start)
}

#[allow(clippy::too_many_arguments)]
fn extract_ts_line_refs(
    line: &str,
    dir: &str,
    line_no: u32,
    lookup: &KnownLookup,
    aliases: &BTreeMap<String, (Option<String>, String)>,
    this_props: &BTreeMap<String, String>,
    var_types: &BTreeMap<String, String>,
    acc: &mut DepAccumulator,
) {
    // 1. `extends Base` / `implements I1, I2`
    let before_brace = line.split('{').next().unwrap_or(line);
    for (kw, kind) in [("extends", "extends"), ("implements", "implements")] {
        if let Some(pos) = find_keyword(before_brace, kw) {
            let after = &before_brace[pos + kw.len()..];
            let stop = if kw == "extends" {
                find_keyword(after, "implements").unwrap_or(after.len())
            } else {
                after.len()
            };
            for item in after[..stop].split(',') {
                let name_raw = item.split('<').next().unwrap_or(item).trim();
                if let Some((ident, _)) = take_ident(name_raw) {
                    if let Some((Some(target), orig)) = aliases.get(ident) {
                        acc.add_resolved(target.clone(), kind, &[orig], line_no, line);
                    } else if let Some(target) = lookup.resolve_stem_in_dir(dir, ident, TS_JS_EXTS)
                    {
                        acc.add_resolved(target, kind, &[ident], line_no, line);
                    }
                }
            }
        }
    }

    // 2. Calls, instantiations, static/namespace member reads & writes, `this.prop.m()`, `var.prop`
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    while i < len {
        let ch = bytes[i] as char;
        if is_ident_start(ch) && (i == 0 || !is_ident_char(bytes[i - 1] as char)) {
            let mut end = i + 1;
            while end < len && is_ident_char(bytes[end] as char) {
                end += 1;
            }
            let ident = &line[i..end];
            let rest = line[end..].trim_start();
            let preceded_by_dot = line[..i].trim_end().ends_with('.');

            if !preceded_by_dot && ident == "this" {
                if let Some(after_dot1) = strip_ts_dot_op(rest)
                    && let Some((prop, after_prop)) = take_ts_js_member_ident(after_dot1)
                    && let Some(after_dot2) = strip_ts_dot_op(after_prop)
                    && let Some((mem2, after_mem2)) = take_ts_js_member_ident(after_dot2)
                    && let Some(owner_ty) = this_props.get(prop)
                    && let Some((target, canon)) =
                        resolve_ts_owner_type(owner_ty, dir, lookup, aliases)
                {
                    let trimmed_after = after_mem2.trim_start();
                    let is_call = trimmed_after.starts_with('(')
                        || (trimmed_after.starts_with('<') && trimmed_after.contains('('));
                    let (dep_kind, mem_kind, access) =
                        classify_member_access(mem2, after_mem2, false, is_call, "property", false);
                    acc.add_resolved_members(
                        target,
                        dep_kind,
                        &[canon.as_str(), mem2],
                        &[DepMemberRef::basic(
                            mem2,
                            mem_kind,
                            Some(canon.clone()),
                            access,
                            line_no,
                        )],
                        line_no,
                        line,
                    );
                }
            } else if !preceded_by_dot && let Some((Some(target), orig)) = aliases.get(ident) {
                if let Some(after_dot) = strip_ts_dot_op(rest) {
                    if let Some((member, after_m)) = take_ts_js_member_ident(after_dot) {
                        let trimmed_after = after_m.trim_start();
                        let is_call = trimmed_after.starts_with('(')
                            || (trimmed_after.starts_with('<') && trimmed_after.contains('('));
                        let is_ns = orig == "*";
                        let orig_is_class =
                            !is_ns && orig.chars().next().is_some_and(|c| c.is_ascii_uppercase());
                        let (dep_kind, mem_kind, access) = classify_member_access(
                            member,
                            after_m,
                            orig_is_class,
                            is_call,
                            "property",
                            is_ns || !orig_is_class,
                        );
                        let sym = if is_ns { member } else { orig.as_str() };
                        let mut syms = vec![sym];
                        if member != sym {
                            syms.push(member);
                        }
                        let parent = orig_is_class.then(|| orig.clone());
                        acc.add_resolved_members(
                            target.clone(),
                            dep_kind,
                            &syms,
                            &[DepMemberRef::basic(
                                member, mem_kind, parent, access, line_no,
                            )],
                            line_no,
                            line,
                        );
                    }
                } else if rest.starts_with('(')
                    || rest.starts_with('<')
                    || (i >= 4 && line[..i].trim_end().ends_with("new"))
                {
                    let sym = if orig == "*" { ident } else { orig.as_str() };
                    let is_ctor_or_type = line[..i].trim_end().ends_with("new")
                        || sym.chars().next().is_some_and(|c| c.is_ascii_uppercase());
                    let members = if !is_ctor_or_type && rest.starts_with('(') {
                        vec![DepMemberRef::basic(sym, "function", None, "call", line_no)]
                    } else {
                        Vec::new()
                    };
                    acc.add_resolved_members(
                        target.clone(),
                        "calls",
                        &[sym],
                        &members,
                        line_no,
                        line,
                    );
                } else if orig != "*" && is_upper_snake(orig) {
                    acc.add_resolved_members(
                        target.clone(),
                        "uses",
                        &[orig.as_str()],
                        &[DepMemberRef::basic(
                            orig.as_str(),
                            "const",
                            None,
                            "const",
                            line_no,
                        )],
                        line_no,
                        line,
                    );
                }
            } else if !preceded_by_dot
                && let Some(owner_ty) = var_types.get(ident)
                && let Some(after_dot) = strip_ts_dot_op(rest)
                && let Some((member, after_m)) = take_ts_js_member_ident(after_dot)
                && let Some((target, canon)) = resolve_ts_owner_type(owner_ty, dir, lookup, aliases)
            {
                let trimmed_after = after_m.trim_start();
                let is_call = trimmed_after.starts_with('(')
                    || (trimmed_after.starts_with('<') && trimmed_after.contains('('));
                let (dep_kind, mem_kind, access) =
                    classify_member_access(member, after_m, false, is_call, "property", false);
                acc.add_resolved_members(
                    target,
                    dep_kind,
                    &[canon.as_str(), member],
                    &[DepMemberRef::basic(
                        member,
                        mem_kind,
                        Some(canon.clone()),
                        access,
                        line_no,
                    )],
                    line_no,
                    line,
                );
            }
            i = end;
            continue;
        }
        i += 1;
    }
}

fn resolve_ts_js_spec(dir: &str, spec: &str, lookup: &KnownLookup) -> Option<String> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("./") || trimmed.starts_with("../") {
        let base = resolve_relative_segments(dir, trimmed)?;
        return resolve_ts_js_candidates(&base, lookup);
    }
    if let Some(alias_rest) = trimmed
        .strip_prefix("@/")
        .or_else(|| trimmed.strip_prefix("~/"))
    {
        for prefix in ["src", "app", ""] {
            let cand = join_rel(prefix, alias_rest);
            if let Some(found) = resolve_ts_js_candidates(&cand, lookup) {
                return Some(found);
            }
        }
        return resolve_ts_js_suffix(alias_rest, lookup);
    }
    None
}

fn resolve_ts_js_candidates(base: &str, lookup: &KnownLookup) -> Option<String> {
    if let Some(found) = lookup.resolve_exact_or_ci(base) {
        return Some(found);
    }
    // TS ESM convention: `./foo.js` -> `./foo.ts` / `./foo.tsx`
    if let Some((stem_path, ext)) = base.rsplit_once('.') {
        let lower_ext = ext.to_ascii_lowercase();
        let mapped: &[&str] = match lower_ext.as_str() {
            "js" => &["ts", "tsx", "js", "jsx"],
            "jsx" => &["tsx", "ts", "jsx", "js"],
            "mjs" => &["mts", "mjs"],
            "cjs" => &["cts", "cjs"],
            _ => &[],
        };
        for alt_ext in mapped {
            let cand = format!("{stem_path}.{alt_ext}");
            if let Some(found) = lookup.resolve_exact_or_ci(&cand) {
                return Some(found);
            }
        }
    }
    for ext in TS_JS_EXTS {
        let cand = format!("{base}.{ext}");
        if let Some(found) = lookup.resolve_exact_or_ci(&cand) {
            return Some(found);
        }
    }
    for ext in TS_JS_EXTS {
        let cand = format!("{base}/index.{ext}");
        if let Some(found) = lookup.resolve_exact_or_ci(&cand) {
            return Some(found);
        }
    }
    None
}

fn resolve_ts_js_suffix(rest: &str, lookup: &KnownLookup) -> Option<String> {
    if let Some(found) = lookup.resolve_suffix(rest, Some(TS_JS_EXTS)) {
        return Some(found);
    }
    for ext in TS_JS_EXTS {
        if let Some(found) = lookup.resolve_suffix(&format!("{rest}.{ext}"), Some(TS_JS_EXTS)) {
            return Some(found);
        }
        if let Some(found) = lookup.resolve_suffix(&format!("{rest}/index.{ext}"), Some(TS_JS_EXTS))
        {
            return Some(found);
        }
    }
    None
}

// =========================================================================
// Python Dependency Extraction & Resolution
// =========================================================================

fn clean_py_type(raw: &str) -> Option<String> {
    let mut s = raw.split(['=', ',']).next().unwrap_or(raw).trim();
    for wrapper in ["Optional[", "Union[", "Annotated["] {
        if let Some(inner) = s.strip_prefix(wrapper) {
            s = inner.trim_end_matches(']').trim();
            if wrapper == "Annotated[" {
                s = s.split(',').next().unwrap_or(s).trim();
            }
        }
    }
    let chosen = s
        .split('|')
        .map(str::trim)
        .find(|part| !part.is_empty() && *part != "None")?;
    let base = chosen.split('[').next().unwrap_or(chosen).trim();
    if base.is_empty()
        || matches!(
            base,
            "str"
                | "int"
                | "float"
                | "bool"
                | "bytes"
                | "list"
                | "dict"
                | "set"
                | "tuple"
                | "Any"
                | "None"
                | "Self"
                | "object"
                | "Callable"
        )
    {
        return None;
    }
    if base.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        Some(base.to_owned())
    } else {
        None
    }
}

fn resolve_py_owner_type(
    ty_name: &str,
    dir: &str,
    lookup: &KnownLookup,
    aliases: &BTreeMap<String, (Option<String>, String)>,
) -> Option<(String, String)> {
    if let Some((head, tail)) = ty_name.rsplit_once('.') {
        let root = head.split('.').next().unwrap_or(head);
        if let Some((Some(target), _)) = aliases.get(root) {
            return Some((target.clone(), tail.to_owned()));
        }
    }
    if let Some((Some(target), orig)) = aliases.get(ty_name) {
        let canon = if orig == file_stem(target) {
            ty_name.to_owned()
        } else {
            orig.clone()
        };
        return Some((target.clone(), canon));
    }
    lookup
        .resolve_stem_in_dir(dir, ty_name, PYTHON_EXTS)
        .map(|target| (target, ty_name.to_owned()))
}

fn collect_py_param_bindings(line: &str, var_types: &mut BTreeMap<String, String>) {
    let slice = line.split_once('(').map(|(_, after)| after).unwrap_or(line);
    let params = slice.split(')').next().unwrap_or(slice);
    for raw_p in params.split(',') {
        let p = raw_p.trim();
        let Some((lhs, rhs)) = p.split_once(':') else {
            continue;
        };
        let param_name = lhs.trim().trim_start_matches('*');
        if param_name.is_empty() || matches!(param_name, "self" | "cls") {
            continue;
        }
        if let Some((ident, rest)) = take_ident(param_name)
            && rest.trim().is_empty()
            && let Some(ty) = clean_py_type(rhs)
        {
            var_types.insert(ident.to_owned(), ty);
        }
    }
}

fn extract_py_call_ctor_type(
    rhs: &str,
    aliases: &BTreeMap<String, (Option<String>, String)>,
) -> Option<String> {
    let r = rhs.trim();
    if let Some((head, after_h)) = take_ident(r) {
        let rest = after_h.trim_start();
        if let Some(after_dot) = rest.strip_prefix('.')
            && let Some((cls, after_c)) = take_ident(after_dot.trim_start())
            && after_c.trim_start().starts_with('(')
            && (cls.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                || aliases.contains_key(head))
        {
            return Some(format!("{head}.{cls}"));
        }
        if rest.starts_with('(')
            && (head.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                || aliases.contains_key(head))
        {
            return Some(head.to_owned());
        }
    }
    None
}

fn update_py_var_bindings(
    clean_code: &str,
    in_class_body: bool,
    in_fn: bool,
    aliases: &BTreeMap<String, (Option<String>, String)>,
    this_attrs: &mut BTreeMap<String, String>,
    var_types: &mut BTreeMap<String, String>,
) {
    // 1. Assignment `lhs = rhs` or annotated `lhs: Type [= rhs]`
    if let Some((lhs, rhs)) = split_simple_assign(clean_code) {
        let lhs_t = lhs.trim();
        let (target_expr, ann_ty) = if let Some((var_part, ty_part)) = lhs_t.split_once(':') {
            (var_part.trim(), clean_py_type(ty_part))
        } else {
            (lhs_t, None)
        };
        let resolved_ty = ann_ty.or_else(|| {
            let rhs_t = rhs.trim();
            if let Some(after_self) = rhs_t.strip_prefix("self.")
                && let Some((attr, rest)) = take_ident(after_self)
                && rest.trim().is_empty()
            {
                return this_attrs.get(attr).cloned();
            }
            if let Some((rhs_var, rest)) = take_ident(rhs_t)
                && rest.trim().is_empty()
            {
                return var_types.get(rhs_var).cloned();
            }
            extract_py_call_ctor_type(rhs_t, aliases)
        });
        if let Some(ty) = resolved_ty {
            if let Some(after_self) = target_expr.strip_prefix("self.")
                && let Some((attr, rest)) = take_ident(after_self)
                && rest.trim().is_empty()
            {
                this_attrs.insert(attr.to_owned(), ty);
            } else if let Some((var_name, rest)) = take_ident(target_expr)
                && rest.trim().is_empty()
            {
                if in_class_body && !in_fn {
                    this_attrs.insert(var_name.to_owned(), ty);
                } else {
                    var_types.insert(var_name.to_owned(), ty);
                }
            }
        }
        return;
    }

    // 2. Annotation-only statement (`self.client: Client` or class field `client: Client`)
    if let Some((lhs_part, rhs_part)) = clean_code.split_once(':')
        && !clean_code.starts_with("if ")
        && !clean_code.starts_with("elif ")
        && !clean_code.starts_with("def ")
        && !clean_code.starts_with("class ")
        && let Some(ty) = clean_py_type(rhs_part)
    {
        let target_expr = lhs_part.trim();
        if let Some(after_self) = target_expr.strip_prefix("self.")
            && let Some((attr, rest)) = take_ident(after_self)
            && rest.trim().is_empty()
        {
            this_attrs.insert(attr.to_owned(), ty);
        } else if let Some((var_name, rest)) = take_ident(target_expr)
            && rest.trim().is_empty()
        {
            if in_class_body && !in_fn {
                this_attrs.insert(var_name.to_owned(), ty);
            } else {
                var_types.insert(var_name.to_owned(), ty);
            }
        }
    }
}

fn extract_python_deps(
    rel_path: &str,
    source: &str,
    lookup: &KnownLookup,
    acc: &mut DepAccumulator,
) {
    let dir = folder_of(rel_path);
    let mut aliases: BTreeMap<String, (Option<String>, String)> = BTreeMap::new();
    let mut this_attrs: BTreeMap<String, String> = BTreeMap::new();
    let mut var_types: BTreeMap<String, String> = BTreeMap::new();
    let mut triple_quote: Option<&'static str> = None;
    let mut import_buf = String::new();
    let mut import_line = 0u32;
    let mut paren_depth = 0usize;
    let mut class_indent: Option<usize> = None;
    let mut fn_stack: Vec<usize> = Vec::new();
    let mut in_fn_sig = false;
    let mut sig_parens = 0usize;

    for (idx, raw_line) in source.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        if let Some(marker) = triple_quote {
            if raw_line.contains(marker) {
                triple_quote = None;
            }
            continue;
        }
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('@') {
            continue;
        }
        for marker in ["\"\"\"", "'''"] {
            if let Some(after) = trimmed.strip_prefix(marker) {
                if !after.contains(marker) {
                    triple_quote = Some(marker);
                }
                break;
            }
        }
        if trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''") {
            continue;
        }

        let code = strip_py_line_comment(trimmed);
        if code.is_empty() {
            continue;
        }

        let indent = raw_line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .count();
        if !in_fn_sig {
            let mut popped_fn = false;
            while fn_stack.last().is_some_and(|fi| indent <= *fi) {
                fn_stack.pop();
                popped_fn = true;
            }
            if popped_fn {
                var_types.clear();
            }
            if class_indent.is_some_and(|ci| indent <= ci) {
                class_indent = None;
                this_attrs.clear();
            }
        }

        if !import_buf.is_empty() {
            let clean_part = code.trim_end_matches('\\').trim();
            import_buf.push(' ');
            import_buf.push_str(clean_part);
            paren_depth = update_paren_depth(paren_depth, clean_part);
            if paren_depth == 0 && !code.trim_end().ends_with('\\') {
                let stmt = std::mem::take(&mut import_buf);
                process_python_import_stmt(&dir, &stmt, import_line, lookup, &mut aliases, acc);
            }
            continue;
        }

        if starts_word_at(code, 0, "import") || starts_word_at(code, 0, "from") {
            let clean_part = code.trim_end_matches('\\').trim();
            paren_depth = update_paren_depth(0, clean_part);
            if paren_depth == 0 && !code.trim_end().ends_with('\\') {
                process_python_import_stmt(&dir, clean_part, line_no, lookup, &mut aliases, acc);
            } else {
                import_buf = clean_part.to_owned();
                import_line = line_no;
            }
            continue;
        }

        let clean_code = strip_py_strings(code);

        // `class Child(Base1, Base2):`
        if let Some(after_class) = clean_code.strip_prefix("class ") {
            if class_indent.is_none_or(|ci| indent <= ci) {
                this_attrs.clear();
            }
            class_indent = Some(indent);
            if let Some((_, after_name)) = take_ident(after_class)
                && let Some(after_open) = after_name.trim_start().strip_prefix('(')
                && let Some((bases_str, _)) = after_open.split_once(')')
            {
                for base in bases_str.split(',') {
                    let base_t = base.trim();
                    let head = base_t.split('.').next().unwrap_or(base_t);
                    if let Some((Some(target), orig)) = aliases.get(head) {
                        let sym = base_t.rsplit('.').next().unwrap_or(orig);
                        acc.add_resolved(target.clone(), "extends", &[sym], line_no, code);
                    } else if let Some(target) = lookup.resolve_stem_in_dir(&dir, head, PYTHON_EXTS)
                    {
                        acc.add_resolved(target, "extends", &[base_t], line_no, code);
                    }
                }
            }
        }

        let after_async = clean_code
            .strip_prefix("async ")
            .unwrap_or(&clean_code)
            .trim_start();
        if let Some(after_def) = after_async.strip_prefix("def ") {
            fn_stack.push(indent);
            var_types.clear();
            in_fn_sig = true;
            sig_parens = update_paren_depth(0, after_def);
            collect_py_param_bindings(after_def, &mut var_types);
            if sig_parens == 0 && after_def.contains(':') {
                in_fn_sig = false;
            }
        } else if in_fn_sig {
            collect_py_param_bindings(&clean_code, &mut var_types);
            sig_parens = update_paren_depth(sig_parens, &clean_code);
            if sig_parens == 0 && clean_code.contains(':') {
                in_fn_sig = false;
            }
        } else {
            update_py_var_bindings(
                &clean_code,
                class_indent.is_some(),
                !fn_stack.is_empty(),
                &aliases,
                &mut this_attrs,
                &mut var_types,
            );
        }

        // Calls, non-call member accesses, and imported constant references
        extract_py_line_calls(
            &clean_code,
            &dir,
            line_no,
            lookup,
            &aliases,
            &this_attrs,
            &var_types,
            acc,
        );
    }
}

fn update_paren_depth(mut depth: usize, s: &str) -> usize {
    for ch in s.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    depth
}

fn strip_py_line_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    while i < len {
        let ch = bytes[i];
        if ch == b'#' {
            return line[..i].trim();
        }
        if ch == b'"' || ch == b'\'' {
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                } else if bytes[i] == ch {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        i += 1;
    }
    line.trim()
}

fn process_python_import_stmt(
    dir: &str,
    stmt: &str,
    line_no: u32,
    lookup: &KnownLookup,
    aliases: &mut BTreeMap<String, (Option<String>, String)>,
    acc: &mut DepAccumulator,
) {
    if let Some(after_import) = stmt.strip_prefix("import")
        && stmt[6..].starts_with(char::is_whitespace)
    {
        for clause in after_import.split(',') {
            let clause = clause.trim();
            if clause.is_empty() {
                continue;
            }
            let (mod_path, alias) = if let Some((lhs, rhs)) = split_as_alias(clause) {
                (lhs.trim(), Some(rhs.trim()))
            } else {
                (clause, None)
            };
            let resolved = resolve_python_module(dir, 0, mod_path, lookup);
            let last_seg = mod_path.rsplit('.').next().unwrap_or(mod_path);
            let local_name =
                alias.unwrap_or_else(|| mod_path.split('.').next().unwrap_or(mod_path));
            aliases.insert(
                local_name.to_owned(),
                (resolved.clone(), last_seg.to_owned()),
            );
            match resolved {
                Some(target) => acc.add_resolved(target, "imports", &[last_seg], line_no, stmt),
                None => acc.add_unresolved(mod_path, "imports", &[last_seg], line_no, stmt),
            }
        }
        return;
    }

    if let Some(after_from) = stmt.strip_prefix("from")
        && let Some((mod_part, import_part)) = split_python_from_import(after_from)
    {
        let mod_trim = mod_part.trim();
        let dots = mod_trim.bytes().take_while(|b| *b == b'.').count();
        let dotted = &mod_trim[dots..];
        let items_raw = import_part
            .trim()
            .trim_start_matches('(')
            .trim_end_matches(')');

        let mut parsed_items: Vec<(&str, Option<&str>)> = Vec::new();
        for clause in items_raw.split(',') {
            let clause = clause.trim();
            if clause.is_empty() {
                continue;
            }
            if let Some((lhs, rhs)) = split_as_alias(clause) {
                parsed_items.push((lhs.trim(), Some(rhs.trim())));
            } else {
                parsed_items.push((clause, None));
            }
        }

        // Handle `from . import submodule` where each imported name may itself be a sibling `.py` file.
        if dots > 0 && dotted.is_empty() {
            for (sym, alias) in parsed_items {
                if let Some(sub_target) = resolve_python_module(dir, dots, sym, lookup) {
                    let local = alias.unwrap_or(sym);
                    aliases.insert(local.to_owned(), (Some(sub_target.clone()), sym.to_owned()));
                    acc.add_resolved(sub_target, "imports", &[sym], line_no, stmt);
                } else if let Some(pkg_init) = resolve_python_module(dir, dots, "", lookup) {
                    let local = alias.unwrap_or(sym);
                    aliases.insert(local.to_owned(), (Some(pkg_init.clone()), sym.to_owned()));
                    acc.add_resolved(pkg_init, "imports", &[sym], line_no, stmt);
                } else {
                    acc.add_unresolved(
                        &format!("{mod_trim}{sym}"),
                        "imports",
                        &[sym],
                        line_no,
                        stmt,
                    );
                }
            }
            return;
        }

        let resolved = resolve_python_module(dir, dots, dotted, lookup);
        if let Some(target) = resolved {
            let syms: Vec<&str> = parsed_items
                .iter()
                .filter_map(|(s, _)| (*s != "*").then_some(*s))
                .collect();
            for (sym, alias) in &parsed_items {
                if *sym != "*" {
                    let local = alias.unwrap_or(sym);
                    aliases.insert(local.to_owned(), (Some(target.clone()), (*sym).to_owned()));
                }
            }
            acc.add_resolved(target, "imports", &syms, line_no, stmt);
            return;
        }

        // Fallback for namespace packages (or `from pkg import submod` without `__init__.py`).
        let mut unresolved_syms = Vec::new();
        for (sym, alias) in &parsed_items {
            if *sym != "*"
                && !dotted.is_empty()
                && let Some(sub_target) =
                    resolve_python_module(dir, dots, &format!("{dotted}.{sym}"), lookup)
            {
                let local = alias.unwrap_or(sym);
                aliases.insert(
                    local.to_owned(),
                    (Some(sub_target.clone()), (*sym).to_owned()),
                );
                acc.add_resolved(sub_target, "imports", &[sym], line_no, stmt);
            } else if *sym != "*" {
                unresolved_syms.push(*sym);
            }
        }
        if !unresolved_syms.is_empty() || parsed_items.iter().any(|(s, _)| *s == "*") {
            acc.add_unresolved(mod_trim, "imports", &unresolved_syms, line_no, stmt);
        }
    }
}

fn split_python_from_import(after_from: &str) -> Option<(&str, &str)> {
    let pos = find_keyword(after_from, "import")?;
    Some((&after_from[..pos], &after_from[pos + 6..]))
}

fn resolve_python_module(
    dir: &str,
    dots: usize,
    dotted: &str,
    lookup: &KnownLookup,
) -> Option<String> {
    let rel_mod = dotted.replace('.', "/");
    if dots > 0 {
        let mut cur = dir.to_owned();
        for _ in 1..dots {
            cur = folder_of(&cur);
        }
        let base = if rel_mod.is_empty() {
            cur
        } else {
            join_rel(&cur, &rel_mod)
        };
        return resolve_python_candidates(&base, lookup);
    }

    if rel_mod.is_empty() {
        return None;
    }
    if let Some(found) = resolve_python_candidates(&rel_mod, lookup) {
        return Some(found);
    }
    for ext in PYTHON_EXTS {
        if let Some(found) = lookup.resolve_suffix(&format!("{rel_mod}.{ext}"), Some(PYTHON_EXTS)) {
            return Some(found);
        }
        if let Some(found) =
            lookup.resolve_suffix(&format!("{rel_mod}/__init__.{ext}"), Some(PYTHON_EXTS))
        {
            return Some(found);
        }
    }
    None
}

fn resolve_python_candidates(base: &str, lookup: &KnownLookup) -> Option<String> {
    for ext in PYTHON_EXTS {
        let file_cand = format!("{base}.{ext}");
        if let Some(found) = lookup.resolve_exact_or_ci(&file_cand) {
            return Some(found);
        }
    }
    for ext in PYTHON_EXTS {
        let init_cand = join_rel(base, &format!("__init__.{ext}"));
        if let Some(found) = lookup.resolve_exact_or_ci(&init_cand) {
            return Some(found);
        }
    }
    None
}

fn strip_py_strings(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut i = 0usize;
    let bytes = code.as_bytes();
    while i < bytes.len() {
        let ch = bytes[i];
        if ch == b'"' || ch == b'\'' {
            out.push(' ');
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                } else if bytes[i] == ch {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            out.push(' ');
        } else if ch.is_ascii() {
            out.push(ch as char);
            i += 1;
        } else if let Some(c) = code[i..].chars().next() {
            out.push(c);
            i += c.len_utf8();
        } else {
            break;
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn extract_py_line_calls(
    line: &str,
    dir: &str,
    line_no: u32,
    lookup: &KnownLookup,
    aliases: &BTreeMap<String, (Option<String>, String)>,
    this_attrs: &BTreeMap<String, String>,
    var_types: &BTreeMap<String, String>,
    acc: &mut DepAccumulator,
) {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    while i < len {
        let ch = bytes[i] as char;
        if is_ident_start(ch) && (i == 0 || !is_ident_char(bytes[i - 1] as char)) {
            let mut end = i + 1;
            while end < len && is_ident_char(bytes[end] as char) {
                end += 1;
            }
            let ident = &line[i..end];
            let preceded_by_dot = line[..i].trim_end().ends_with('.');
            let rest = line[end..].trim_start();

            if !preceded_by_dot && ident == "self" {
                if let Some(after_dot1) = rest.strip_prefix('.')
                    && let Some((attr, after_attr)) = take_ident(after_dot1.trim_start())
                    && let Some(after_dot2) = after_attr.trim_start().strip_prefix('.')
                    && let Some((mem2, after_mem2)) = take_ident(after_dot2.trim_start())
                    && let Some(owner_ty) = this_attrs.get(attr)
                    && let Some((target, canon)) =
                        resolve_py_owner_type(owner_ty, dir, lookup, aliases)
                {
                    let is_call = after_mem2.trim_start().starts_with('(');
                    let (dep_kind, mem_kind, access) =
                        classify_member_access(mem2, after_mem2, false, is_call, "property", false);
                    acc.add_resolved_members(
                        target,
                        dep_kind,
                        &[canon.as_str(), mem2],
                        &[DepMemberRef::basic(
                            mem2,
                            mem_kind,
                            Some(canon.clone()),
                            access,
                            line_no,
                        )],
                        line_no,
                        line,
                    );
                }
            } else if !preceded_by_dot && let Some((Some(target), orig)) = aliases.get(ident) {
                if let Some(after_dot) = rest.strip_prefix('.') {
                    if let Some((member, after_m)) = take_ident(after_dot.trim_start()) {
                        let is_call = after_m.trim_start().starts_with('(');
                        let is_mod = orig == file_stem(target);
                        let orig_is_class =
                            !is_mod && orig.chars().next().is_some_and(|c| c.is_ascii_uppercase());
                        let (dep_kind, mem_kind, access) = classify_member_access(
                            member,
                            after_m,
                            orig_is_class,
                            is_call,
                            "property",
                            is_mod || !orig_is_class,
                        );
                        let mut syms = vec![orig.as_str()];
                        if member != orig {
                            syms.push(member);
                        }
                        let parent = orig_is_class.then(|| orig.clone());
                        acc.add_resolved_members(
                            target.clone(),
                            dep_kind,
                            &syms,
                            &[DepMemberRef::basic(
                                member, mem_kind, parent, access, line_no,
                            )],
                            line_no,
                            line,
                        );
                    }
                } else if rest.starts_with('(') {
                    let members = if orig.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                        Vec::new()
                    } else {
                        vec![DepMemberRef::basic(
                            orig.as_str(),
                            "function",
                            None,
                            "call",
                            line_no,
                        )]
                    };
                    acc.add_resolved_members(
                        target.clone(),
                        "calls",
                        &[orig],
                        &members,
                        line_no,
                        line,
                    );
                } else if is_upper_snake(orig) {
                    acc.add_resolved_members(
                        target.clone(),
                        "uses",
                        &[orig],
                        &[DepMemberRef::basic(
                            orig.as_str(),
                            "const",
                            None,
                            "const",
                            line_no,
                        )],
                        line_no,
                        line,
                    );
                }
            } else if !preceded_by_dot
                && let Some(owner_ty) = var_types.get(ident)
                && let Some(after_dot) = rest.strip_prefix('.')
                && let Some((member, after_m)) = take_ident(after_dot.trim_start())
                && let Some((target, canon)) = resolve_py_owner_type(owner_ty, dir, lookup, aliases)
            {
                let is_call = after_m.trim_start().starts_with('(');
                let (dep_kind, mem_kind, access) =
                    classify_member_access(member, after_m, false, is_call, "property", false);
                acc.add_resolved_members(
                    target,
                    dep_kind,
                    &[canon.as_str(), member],
                    &[DepMemberRef::basic(
                        member,
                        mem_kind,
                        Some(canon.clone()),
                        access,
                        line_no,
                    )],
                    line_no,
                    line,
                );
            }
            i = end;
            continue;
        }
        i += 1;
    }
}

// =========================================================================
// Shared Text & Identifier Helpers
// =========================================================================

fn join_rel(prefix: &str, tail: &str) -> String {
    if prefix.is_empty() {
        tail.trim_start_matches('/').to_owned()
    } else if tail.is_empty() {
        prefix.trim_end_matches('/').to_owned()
    } else {
        format!(
            "{}/{}",
            prefix.trim_end_matches('/'),
            tail.trim_start_matches('/')
        )
    }
}

fn find_keyword(haystack: &str, kw: &str) -> Option<usize> {
    let bytes = haystack.as_bytes();
    let kw_len = kw.len();
    let mut offset = 0usize;
    while let Some(pos) = haystack[offset..].find(kw) {
        let abs = offset + pos;
        let left_ok = abs == 0 || !is_ident_char(bytes[abs - 1] as char);
        let right_ok = abs + kw_len >= bytes.len() || !is_ident_char(bytes[abs + kw_len] as char);
        if left_ok && right_ok {
            return Some(abs);
        }
        offset = abs + kw_len;
    }
    None
}

fn starts_word_at(s: &str, idx: usize, word: &str) -> bool {
    let Some(sub) = s.get(idx..) else {
        return false;
    };
    if !sub.starts_with(word) {
        return false;
    }
    let left_ok = idx == 0 || !is_ident_char(s.as_bytes()[idx - 1] as char);
    let right_ok = sub
        .as_bytes()
        .get(word.len())
        .is_none_or(|b| !is_ident_char(*b as char));
    left_ok && right_ok
}

fn extract_first_quoted_string(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    for (idx, &b) in bytes.iter().enumerate() {
        if b == b'\'' || b == b'"' {
            let mut j = idx + 1;
            while j < bytes.len() {
                if bytes[j] == b'\\' && j + 1 < bytes.len() {
                    j += 2;
                } else if bytes[j] == b {
                    return Some(&s[idx + 1..j]);
                } else {
                    j += 1;
                }
            }
            return None;
        }
    }
    None
}

fn extract_last_quoted_string(s: &str) -> Option<&str> {
    let mut last = None;
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\'' || b == b'"' {
            let start = i + 1;
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                } else if bytes[i] == b {
                    last = Some(&s[start..i]);
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        i += 1;
    }
    last
}

fn sanitize_whole_source(rel_path: &str, source: &str) -> String {
    match detect_language(rel_path) {
        Some(lang @ (Language::Php | Language::Rust | Language::TsJs)) => {
            let mut in_block = false;
            let mut out = String::with_capacity(source.len());
            for line in source.lines() {
                out.push_str(&sanitize_c_like_line(line, &mut in_block, lang));
                out.push('\n');
            }
            out
        }
        Some(Language::Python) => {
            let mut triple: Option<&'static str> = None;
            let mut out = String::with_capacity(source.len());
            for line in source.lines() {
                if let Some(marker) = triple {
                    if line.contains(marker) {
                        triple = None;
                    }
                    out.push('\n');
                    continue;
                }
                let trimmed = line.trim();
                for marker in ["\"\"\"", "'''"] {
                    if let Some(after) = trimmed.strip_prefix(marker)
                        && !after.contains(marker)
                    {
                        triple = Some(marker);
                    }
                }
                if triple.is_some() {
                    out.push('\n');
                    continue;
                }
                let code = strip_py_line_comment(line);
                out.push_str(&strip_py_strings(code));
                out.push('\n');
            }
            out
        }
        None => source.to_owned(),
    }
}

fn collect_identifiers(sanitized: &str) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    let bytes = sanitized.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    while i < len {
        let ch = bytes[i] as char;
        if is_ident_start(ch) {
            let mut end = i + 1;
            while end < len && is_ident_char(bytes[end] as char) {
                end += 1;
            }
            set.insert(sanitized[i..end].to_owned());
            i = end;
        } else {
            i += 1;
        }
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn diff_entities_tracks_created_modified_signature_and_deleted_across_languages() {
        let before_php = r#"<?php
class JwtGuard {
    public function verify(string $token): bool {
        return $token !== '';
    }
    public function obsolete(): void {}
}
"#;
        let after_php = r#"<?php
// Line-shift comment should not mark untouched classes modified
class JwtGuard {
    public function verify(string $token, bool $strict): bool {
        return $token !== '';
    }
    public function issue(): string {
        return 'tok';
    }
}
"#;
        let changes = diff_entities("src/Auth/JwtGuard.php", Some(before_php), Some(after_php));
        assert_eq!(changes.len(), 3, "{changes:?}");
        assert_eq!(changes[0].name, "verify");
        assert_eq!(changes[0].change, "modified");
        assert!(changes[0].signature_changed);
        assert_eq!(changes[0].parent.as_deref(), Some("JwtGuard"));

        assert_eq!(changes[1].name, "obsolete");
        assert_eq!(changes[1].change, "deleted");
        assert!(!changes[1].signature_changed);

        assert_eq!(changes[2].name, "issue");
        assert_eq!(changes[2].change, "created");
        assert!(!changes[2].signature_changed);

        // Body-only modification keeps `signature_changed == false`.
        let before_rs = "pub fn compute(x: i32) -> i32 {\n    x + 1\n}\n";
        let after_rs = "pub fn compute(x: i32) -> i32 {\n    x * 2\n}\n";
        let rs_changes = diff_entities("src/math.rs", Some(before_rs), Some(after_rs));
        assert_eq!(rs_changes.len(), 1);
        assert_eq!(rs_changes[0].change, "modified");
        assert!(!rs_changes[0].signature_changed);

        // Non-code & empty files never panic and return empty.
        assert!(diff_entities("README.md", Some("# Hi"), Some("# Bye")).is_empty());
        assert!(diff_entities("empty.rs", Some(""), None).is_empty());
    }

    #[test]
    fn diff_entities_detects_ts_and_python_body_and_signature_changes() {
        let before_ts = r#"
export class TokenManager {
    verify(raw: string): boolean {
        return raw.length > 0;
    }
}
export const helper = (n: number): number => n + 1;
"#;
        let after_ts = r#"
export class TokenManager {
    verify(raw: string): boolean {
        return raw.trim().length > 0;
    }
}
export const helper = (n: number, scale: number): number => n + 1;
"#;
        let ts_changes = diff_entities("web/token.ts", Some(before_ts), Some(after_ts));
        assert_eq!(ts_changes.len(), 2, "{ts_changes:?}");
        assert_eq!(ts_changes[0].name, "verify");
        assert_eq!(ts_changes[0].change, "modified");
        assert!(!ts_changes[0].signature_changed);
        assert_eq!(ts_changes[1].name, "helper");
        assert_eq!(ts_changes[1].change, "modified");
        assert!(ts_changes[1].signature_changed);

        let before_py =
            "class Guard:\n    def check(self, tok: str) -> bool:\n        return bool(tok)\n";
        let after_py = "class Guard:\n    def check(self, tok: str, strict: bool) -> bool:\n        return bool(tok)\n";
        let py_changes = diff_entities("auth/guard.py", Some(before_py), Some(after_py));
        assert_eq!(py_changes.len(), 1, "{py_changes:?}");
        assert_eq!(py_changes[0].name, "check");
        assert_eq!(py_changes[0].change, "modified");
        assert!(py_changes[0].signature_changed);
    }

    #[test]
    fn extract_file_deps_resolves_php_psr4_same_ns_inheritance_calls_and_includes() {
        let files = known(&[
            "src/Auth/JwtGuard.php",
            "src/Auth/BaseGuard.php",
            "src/Auth/AuditTrait.php",
            "src/Security/Signer.php",
            "src/Contracts/GuardInterface.php",
            "src/bootstrap.php",
        ]);
        let php = r#"<?php
namespace App\Auth;

require_once __DIR__ . '/../bootstrap.php';

use App\Security\Signer as TokenSigner;
use App\Contracts\GuardInterface;
use External\Vendor\Logger;

class JwtGuard extends BaseGuard implements GuardInterface {
    use AuditTrait;

    public function __construct(private TokenSigner $signer) {}

    public function handle(string $tok): bool {
        $s = new TokenSigner();
        return TokenSigner::verify($tok) && $s->check($tok);
    }
}
"#;
        let deps = extract_file_deps("src/Auth/JwtGuard.php", php, &files);

        let has = |target: &str, kind: &str, resolved: bool| {
            deps.iter()
                .find(|d| d.target == target && d.kind == kind && d.resolved == resolved)
        };
        let bootstrap = has("src/bootstrap.php", "imports", true).expect("bootstrap import");
        assert!(bootstrap.exists);

        let signer_imp = has("src/Security/Signer.php", "imports", true).expect("signer import");
        assert!(signer_imp.symbols.contains(&"Signer".to_owned()));

        let signer_call = has("src/Security/Signer.php", "calls", true).expect("signer call");
        assert!(signer_call.symbols.contains(&"Signer".to_owned()));
        assert!(signer_call.symbols.contains(&"verify".to_owned()));

        assert!(has("src/Auth/BaseGuard.php", "extends", true).is_some());
        assert!(has("src/Contracts/GuardInterface.php", "implements", true).is_some());
        assert!(has("src/Auth/AuditTrait.php", "uses", true).is_some());

        let ext = has("External\\Vendor\\Logger", "imports", false).expect("unresolved external");
        assert!(!ext.exists);
    }

    #[test]
    fn extract_file_deps_resolves_rust_ts_and_python_imports() {
        let files = known(&[
            "crates/app/src/lib.rs",
            "crates/app/src/auth/mod.rs",
            "crates/app/src/auth/jwt.rs",
            "crates/app/src/security/signer.rs",
            "crates/infra-core/src/fs.rs",
            "web/src/components/App.tsx",
            "web/src/utils/auth.ts",
            "web/src/hooks/index.ts",
            "py/auth/__init__.py",
            "py/auth/guard.py",
            "py/auth/tokens.py",
        ]);

        let rust_src = r#"
use crate::security::signer::{Signer, verify_sig};
use super::auth::jwt::JwtService;
use infra_core::fs::python_realpath;
use std::collections::BTreeMap;

pub fn check() -> bool {
    Signer::new();
    verify_sig("a")
}
"#;
        let rs_deps = extract_file_deps("crates/app/src/lib.rs", rust_src, &files);
        assert!(
            rs_deps
                .iter()
                .any(|d| d.target == "crates/app/src/security/signer.rs"
                    && d.resolved
                    && d.exists
                    && d.kind == "imports")
        );
        assert!(
            rs_deps
                .iter()
                .any(|d| d.target == "crates/app/src/security/signer.rs" && d.kind == "calls")
        );
        assert!(
            rs_deps
                .iter()
                .any(|d| d.target == "crates/infra-core/src/fs.rs" && d.resolved)
        );
        assert!(
            rs_deps
                .iter()
                .any(|d| d.target == "std::collections" && !d.resolved && !d.exists)
        );

        let ts_src = r#"
import { verifyToken as checkTok } from '../utils/auth';
import { useAuth } from '@/hooks';
import React from 'react';

export function run() {
    return checkTok('x') && useAuth();
}
"#;
        let ts_deps = extract_file_deps("web/src/components/App.tsx", ts_src, &files);
        assert!(
            ts_deps
                .iter()
                .any(|d| d.target == "web/src/utils/auth.ts" && d.resolved && d.kind == "imports")
        );
        assert!(ts_deps.iter().any(|d| d.target == "web/src/utils/auth.ts"
            && d.kind == "calls"
            && d.symbols.contains(&"verifyToken".to_owned())));
        assert!(
            ts_deps
                .iter()
                .any(|d| d.target == "web/src/hooks/index.ts" && d.resolved)
        );
        assert!(
            ts_deps
                .iter()
                .any(|d| d.target == "react" && !d.resolved && !d.exists)
        );

        let py_src = r#"
from . import tokens
from py.auth.guard import SessionGuard

class CustomGuard(SessionGuard):
    pass

def boot():
    tokens.issue()
    return CustomGuard()
"#;
        let py_deps = extract_file_deps("py/auth/service.py", py_src, &files);
        assert!(
            py_deps
                .iter()
                .any(|d| d.target == "py/auth/tokens.py" && d.resolved && d.kind == "imports")
        );
        assert!(
            py_deps
                .iter()
                .any(|d| d.target == "py/auth/guard.py" && d.resolved && d.kind == "extends")
        );
    }

    #[test]
    fn diff_file_deps_splits_added_removed_and_unchanged() {
        let files = known(&[
            "src/Auth/Signer.php",
            "src/Auth/OldHelper.php",
            "src/Auth/NewCache.php",
        ]);
        let before = "<?php\nuse App\\Auth\\Signer;\nuse App\\Auth\\OldHelper;\n";
        let after = "<?php\nuse App\\Auth\\Signer;\nuse App\\Auth\\NewCache;\n";
        let diff = diff_file_deps("src/Auth/Controller.php", Some(before), Some(after), &files);

        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].target, "src/Auth/NewCache.php");
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.removed[0].target, "src/Auth/OldHelper.php");
        assert_eq!(diff.unchanged.len(), 1);
        assert_eq!(diff.unchanged[0].target, "src/Auth/Signer.php");
    }

    #[test]
    fn find_matching_references_finds_used_symbols_and_skips_unrelated_files() {
        let files = known(&[
            "src/Auth/JwtGuard.php",
            "src/Http/LoginController.php",
            "src/Billing/InvoiceService.php",
        ]);
        let caller = r#"<?php
namespace App\Http;
use App\Auth\JwtGuard;

class LoginController {
    public function login(JwtGuard $guard, string $token): bool {
        // verifyUnused mentioned only in comment
        return $guard->verifyToken($token);
    }
}
"#;
        let unrelated = r#"<?php
namespace App\Billing;
class InvoiceService {
    public function verifyToken(string $t): bool { return true; }
}
"#;
        let syms = vec![
            "JwtGuard".to_owned(),
            "verifyToken".to_owned(),
            "verifyUnused".to_owned(),
        ];

        let refs = find_matching_references(
            "src/Http/LoginController.php",
            caller,
            "src/Auth/JwtGuard.php",
            &syms,
            &files,
        );
        assert_eq!(refs, vec!["JwtGuard", "verifyToken"]);

        let no_refs = find_matching_references(
            "src/Billing/InvoiceService.php",
            unrelated,
            "src/Auth/JwtGuard.php",
            &syms,
            &files,
        );
        assert!(no_refs.is_empty(), "{no_refs:?}");
    }

    #[test]
    fn handles_non_code_grouped_imports_path_mods_and_aliased_references() {
        assert!(diff_entities("README.md", Some("# Old\n"), Some("# New\n")).is_empty());
        assert!(diff_entities("assets/blob.bin", None, Some("\0\x01\x02")).is_empty());

        let files = known(&[
            "src/Domain/Order.php",
            "src/Domain/Money.php",
            "src/Security/Signer.php",
            "src/Service.php",
            "crates/core/src/lib.rs",
            "crates/core/src/alt/custom.rs",
            "crates/core/src/sub.rs",
            "web/src/shared/fmt.ts",
            "web/src/lazy.ts",
            "web/src/app.ts",
            "pkg/common/helpers.py",
            "pkg/api/routes.py",
        ]);

        // PHP grouped `use` + aliased `find_matching_references`
        let php = r#"<?php
namespace App;
use App\Domain\{Order, Money as Cash};
use App\Security\Signer as TokenSigner;
class Service {
    public function total(Order $o): Cash {
        TokenSigner::verify('x');
        return new Cash();
    }
}
"#;
        let php_deps = extract_file_deps("src/Service.php", php, &files);
        assert!(
            php_deps
                .iter()
                .any(|d| d.target == "src/Domain/Order.php" && d.resolved && d.kind == "imports")
        );
        assert!(php_deps.iter().any(|d| d.target == "src/Domain/Money.php"
            && d.resolved
            && d.symbols.contains(&"Money".to_owned())));
        let alias_refs = find_matching_references(
            "src/Service.php",
            php,
            "src/Security/Signer.php",
            &["Signer".to_owned(), "verify".to_owned()],
            &files,
        );
        assert!(alias_refs.contains(&"Signer".to_owned()), "{alias_refs:?}");
        assert!(alias_refs.contains(&"verify".to_owned()), "{alias_refs:?}");

        // Rust #[path = "..."] mod + mod sub;
        let rs = r#"
#[path = "alt/custom.rs"]
mod custom;
mod sub;
"#;
        let rs_deps = extract_file_deps("crates/core/src/lib.rs", rs, &files);
        assert!(
            rs_deps
                .iter()
                .any(|d| d.target == "crates/core/src/alt/custom.rs" && d.resolved)
        );
        assert!(
            rs_deps
                .iter()
                .any(|d| d.target == "crates/core/src/sub.rs" && d.resolved)
        );

        // TS @/ alias + dynamic import()
        let ts = r#"
import { moneyFmt } from '@/shared/fmt';
const loadLazy = () => import('./lazy');
export function show(n: number) { return moneyFmt(n); }
"#;
        let ts_deps = extract_file_deps("web/src/app.ts", ts, &files);
        assert!(
            ts_deps
                .iter()
                .any(|d| d.target == "web/src/shared/fmt.ts" && d.resolved)
        );
        assert!(
            ts_deps
                .iter()
                .any(|d| d.target == "web/src/lazy.ts" && d.resolved)
        );

        // Python parent-relative `from ..common import helpers`
        let py = "from ..common import helpers\nhelpers.run()\n";
        let py_deps = extract_file_deps("pkg/api/routes.py", py, &files);
        assert!(
            py_deps
                .iter()
                .any(|d| d.target == "pkg/common/helpers.py" && d.resolved)
        );
    }

    #[test]
    fn visibility_rank_and_narrowing_rules() {
        assert_eq!(visibility_rank(None), 4);
        assert_eq!(visibility_rank(Some("public")), 4);
        assert_eq!(visibility_rank(Some("export")), 4);
        assert_eq!(visibility_rank(Some("pub(crate)")), 2);
        assert_eq!(visibility_rank(Some("internal")), 2);
        assert_eq!(visibility_rank(Some("protected(set)")), 2);
        assert_eq!(visibility_rank(Some("package")), 2);
        assert_eq!(visibility_rank(Some("protected")), 1);
        assert_eq!(visibility_rank(Some("private")), 0);
        assert_eq!(visibility_rank(Some("#private")), 0);
        assert_eq!(visibility_rank(Some("private(set)")), 1);

        assert!(is_visibility_narrowed(Some("public"), Some("protected")));
        assert!(is_visibility_narrowed(Some("public"), Some("private")));
        assert!(is_visibility_narrowed(Some("public"), Some("private(set)")));
        assert!(is_visibility_narrowed(
            Some("protected(set)"),
            Some("private(set)")
        ));
        assert!(is_visibility_narrowed(Some("protected"), Some("private")));
        assert!(is_visibility_narrowed(Some("pub(crate)"), Some("private")));
        assert!(!is_visibility_narrowed(Some("private"), Some("public")));
        assert!(!is_visibility_narrowed(Some("public"), Some("public")));
        assert!(!is_visibility_narrowed(
            Some("protected"),
            Some("protected")
        ));
    }

    #[test]
    fn diff_entities_visibility_change_excludes_params_signature() {
        let before = r#"<?php
class Signer {
    public function sign(string $payload): string {
        return $payload;
    }
}
"#;
        let narrowed = r#"<?php
class Signer {
    private function sign(string $payload): string {
        return $payload;
    }
}
"#;
        let widened = r#"<?php
class Signer {
    public function sign(string $payload): string {
        return $payload;
    }
}
"#;
        let sig_edit = r#"<?php
class Signer {
    private function sign(int $payload): string {
        return (string)$payload;
    }
}
"#;

        let diff_narrow = diff_entities("src/Signer.php", Some(before), Some(narrowed));
        let sign_narrow = diff_narrow
            .iter()
            .find(|c| c.name == "sign")
            .expect("sign change");
        assert!(sign_narrow.visibility_changed);
        assert!(sign_narrow.visibility_narrowed);
        assert!(!sign_narrow.signature_changed);
        assert_eq!(sign_narrow.previous_visibility.as_deref(), Some("public"));
        assert_eq!(sign_narrow.visibility.as_deref(), Some("private"));

        let diff_widen = diff_entities("src/Signer.php", Some(narrowed), Some(widened));
        let sign_widen = diff_widen
            .iter()
            .find(|c| c.name == "sign")
            .expect("sign widen");
        assert!(sign_widen.visibility_changed);
        assert!(!sign_widen.visibility_narrowed);
        assert!(!sign_widen.signature_changed);
        assert_eq!(sign_widen.previous_visibility.as_deref(), Some("private"));
        assert_eq!(sign_widen.visibility.as_deref(), Some("public"));

        let diff_sig = diff_entities("src/Signer.php", Some(narrowed), Some(sig_edit));
        let sign_sig = diff_sig
            .iter()
            .find(|c| c.name == "sign")
            .expect("sign signature change");
        assert!(!sign_sig.visibility_changed);
        assert!(!sign_sig.visibility_narrowed);
        assert!(sign_sig.signature_changed);
    }

    #[test]
    fn extract_member_refs_across_all_four_languages() {
        let mut files = BTreeSet::new();
        for p in [
            "src/Security/Signer.php",
            "src/Config/TokenConfig.php",
            "src/Auth/TokenService.php",
            "crates/core/src/config.rs",
            "crates/core/src/client.rs",
            "crates/core/src/service.rs",
            "web/src/signer.ts",
            "web/src/config.ts",
            "web/src/helpers.ts",
            "web/src/service.ts",
            "pkg/client.py",
            "pkg/helpers.py",
            "pkg/service.py",
        ] {
            files.insert(p.to_owned());
        }

        // 1. PHP
        let php_src = r#"<?php
namespace App\Auth;
use App\Security\Signer;
use App\Config\TokenConfig;
class TokenService {
    public function __construct(
        private Signer $signer,
        private TokenConfig $config,
    ) {}

    public function issue(TokenConfig $overrideCfg): int {
        $sig = $this->signer->sign('ok');
        $local = new TokenConfig();
        $local->ttl = 5;
        $max = TokenConfig::DEFAULT_TTL;
        $shared = TokenConfig::$shared;
        return $this->config->ttl + $overrideCfg->ttl + $max;
    }
}
"#;
        let php_deps = extract_file_deps("src/Auth/TokenService.php", php_src, &files);
        let signer_dep = php_deps
            .iter()
            .find(|d| d.target == "src/Security/Signer.php" && d.kind == "calls")
            .expect("PHP Signer call dep");
        assert!(signer_dep.member_refs.iter().any(|m| {
            m.name == "sign"
                && m.kind == "method"
                && m.parent.as_deref() == Some("Signer")
                && m.access == "call"
        }));
        // A member access is a dependency of its own kind: an assignment
        // writes, any other access of a property or a constant reads. The
        // type hints stay plain uses and carry no member.
        let held = |deps: &[DepRef], target: &str, kind: &str| {
            deps.iter()
                .find(|d| d.target == target && d.kind == kind)
                .map(|d| {
                    d.member_refs
                        .iter()
                        .map(|m| {
                            format!(
                                "{} {} {} {}",
                                m.name,
                                m.kind,
                                m.parent.as_deref().unwrap_or("-"),
                                m.access
                            )
                        })
                        .collect::<Vec<_>>()
                })
        };
        const PHP_CONFIG: &str = "src/Config/TokenConfig.php";
        assert_eq!(
            held(&php_deps, PHP_CONFIG, "writes"),
            Some(vec!["$ttl property TokenConfig write".to_owned()])
        );
        assert_eq!(
            held(&php_deps, PHP_CONFIG, "reads"),
            Some(vec![
                "DEFAULT_TTL const TokenConfig const".to_owned(),
                "$shared property TokenConfig static_read".to_owned(),
                "$ttl property TokenConfig read".to_owned(),
            ])
        );
        assert_eq!(held(&php_deps, PHP_CONFIG, "uses"), Some(vec![]));

        // 2. Rust
        let rs_src = r#"
use crate::client::Client;
use crate::config::TokenConfig;

pub struct Service {
    client: Client,
}

impl Service {
    pub fn run(&self, cfg: &mut TokenConfig) -> u64 {
        self.client.send();
        let mut owned = TokenConfig { timeout: 30 };
        owned.timeout = 40;
        let base = TokenConfig::DEFAULT_TTL;
        cfg.timeout + base + owned.timeout
    }
}
"#;
        let rs_deps = extract_file_deps("crates/core/src/service.rs", rs_src, &files);
        let rs_client_dep = rs_deps
            .iter()
            .find(|d| d.target == "crates/core/src/client.rs" && d.kind == "calls")
            .expect("Rust Client call dep");
        assert!(rs_client_dep.member_refs.iter().any(|m| {
            m.name == "send"
                && m.kind == "method"
                && m.parent.as_deref() == Some("Client")
                && m.access == "call"
        }));
        // A field named in a struct literal builds a value: it is no write
        // to state, so it stays with the plain use of the type.
        const RS_CONFIG: &str = "crates/core/src/config.rs";
        assert_eq!(
            held(&rs_deps, RS_CONFIG, "uses"),
            Some(vec!["timeout field TokenConfig write".to_owned()])
        );
        assert_eq!(
            held(&rs_deps, RS_CONFIG, "writes"),
            Some(vec!["timeout field TokenConfig write".to_owned()])
        );
        assert_eq!(
            held(&rs_deps, RS_CONFIG, "reads"),
            Some(vec![
                "DEFAULT_TTL const TokenConfig const".to_owned(),
                "timeout field TokenConfig read".to_owned(),
            ])
        );

        // 3. TS/JS
        let ts_src = r#"
import { Signer } from './signer';
import { TokenConfig, dep } from './config';
import * as helpers from './helpers';

export class Service {
    #signer: Signer;
    constructor(private cfg: TokenConfig) {
        this.#signer = new Signer();
    }
    run(): number {
        this.#signer.sign();
        helpers.notify();
        const x = dep.prop;
        dep.prop = 99;
        return this.cfg.timeout + TokenConfig.DEFAULT_TTL + x;
    }
}
"#;
        let ts_deps = extract_file_deps("web/src/service.ts", ts_src, &files);
        let ts_signer = ts_deps
            .iter()
            .find(|d| d.target == "web/src/signer.ts" && d.kind == "calls")
            .expect("TS Signer calls");
        assert!(ts_signer.member_refs.iter().any(|m| {
            m.name == "sign"
                && m.kind == "method"
                && m.parent.as_deref() == Some("Signer")
                && m.access == "call"
        }));
        let ts_helpers = ts_deps
            .iter()
            .find(|d| d.target == "web/src/helpers.ts" && d.kind == "calls")
            .expect("TS helpers call");
        assert!(ts_helpers.member_refs.iter().any(|m| {
            m.name == "notify" && m.kind == "function" && m.parent.is_none() && m.access == "call"
        }));
        const TS_CONFIG: &str = "web/src/config.ts";
        assert_eq!(
            held(&ts_deps, TS_CONFIG, "writes"),
            Some(vec!["prop property - write".to_owned()])
        );
        assert_eq!(
            held(&ts_deps, TS_CONFIG, "reads"),
            Some(vec![
                "prop property - read".to_owned(),
                "DEFAULT_TTL const TokenConfig const".to_owned(),
                "timeout property TokenConfig read".to_owned(),
            ])
        );

        // 4. Python
        let py_src = r#"
from .client import Client, Config, dep
from . import helpers

class Service:
    def __init__(self, client: Client, cfg: Config):
        self.client = client
        self.cfg = cfg

    def run(self) -> int:
        self.client.send()
        helpers.notify()
        val = dep.prop
        return self.cfg.timeout + helpers.MAX_TTL + val
"#;
        let py_deps = extract_file_deps("pkg/service.py", py_src, &files);
        let py_client_call = py_deps
            .iter()
            .find(|d| d.target == "pkg/client.py" && d.kind == "calls")
            .expect("Py client calls");
        assert!(py_client_call.member_refs.iter().any(|m| {
            m.name == "send"
                && m.kind == "method"
                && m.parent.as_deref() == Some("Client")
                && m.access == "call"
        }));
        assert_eq!(
            held(&py_deps, "pkg/client.py", "reads"),
            Some(vec![
                "prop property - read".to_owned(),
                "timeout property Config read".to_owned(),
            ])
        );
        assert_eq!(held(&py_deps, "pkg/client.py", "writes"), None);
        let py_helpers_call = py_deps
            .iter()
            .find(|d| d.target == "pkg/helpers.py" && d.kind == "calls")
            .expect("Py helpers calls");
        assert!(py_helpers_call.member_refs.iter().any(|m| {
            m.name == "notify" && m.kind == "function" && m.parent.is_none() && m.access == "call"
        }));
        assert_eq!(
            held(&py_deps, "pkg/helpers.py", "reads"),
            Some(vec!["MAX_TTL const - const".to_owned()])
        );
    }

    #[test]
    fn extract_declared_members_and_matching_risks() {
        let mut files = BTreeSet::new();
        files.insert("src/Config/TokenConfig.php".to_owned());
        files.insert("src/Auth/Caller.php".to_owned());

        let target_src = r#"<?php
namespace App\Config;
class TokenConfig {
    public private(set) int $ttl = 60;
    private function secret(): void {}
}
"#;
        let members = extract_declared_members("src/Config/TokenConfig.php", target_src);
        let ttl = members
            .iter()
            .find(|m| m.name == "$ttl")
            .expect("$ttl decl");
        assert_eq!(ttl.kind, "property");
        assert_eq!(ttl.visibility.as_deref(), Some("private(set)"));
        let secret = members
            .iter()
            .find(|m| m.name == "secret")
            .expect("secret decl");
        assert_eq!(secret.visibility.as_deref(), Some("private"));

        let caller_src = r#"<?php
namespace App\Auth;
use App\Config\TokenConfig;
class Caller {
    public function mutate(TokenConfig $cfg): void {
        $cfg->ttl = 120;
        $cfg->secret();
    }
}
"#;
        let matched = find_matching_member_references(
            "src/Auth/Caller.php",
            caller_src,
            "src/Config/TokenConfig.php",
            &members,
            &files,
        );
        let ttl_ref = matched
            .iter()
            .find(|r| r.name == "$ttl" && r.access == "write")
            .expect("$ttl write ref");
        assert_eq!(ttl_ref.access_risk.as_deref(), Some("private_set"));
        let secret_ref = matched
            .iter()
            .find(|r| r.name == "secret" && r.access == "call")
            .expect("secret call ref");
        assert_eq!(secret_ref.access_risk.as_deref(), Some("private"));
    }
}
