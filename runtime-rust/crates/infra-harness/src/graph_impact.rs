//! Dependency impact analysis and untruncated change catalogs across Git
//! targets, active runs, live focus tracking, and Graphify code graphs.
//!
//! Combines `graph_deps` static extraction/diffing with Graphify `Index`
//! symbol/file edges, bounded live caller scanning, directed cycle overlays,
//! and `.harness-architecture.json` layer rule checks without failing when
//! Graphify is unbuilt or the workspace is not a Git checkout.

use crate::{
    HarnessError, Result,
    graph_changes::Resolved,
    graph_deps::{
        DeclaredMember, DepDiff, DepMemberRef, DepRef, EntityChange, access_kind, diff_entities,
        diff_file_deps, extract_declared_members, extract_file_deps,
        find_matching_member_references, find_matching_references,
    },
    graph_live::{GraphFocus, clean_rel_key, folder_of},
    graph_map::{Index, Kind, check_layer_rule, is_test_path, layer_rules::Rules},
    projects,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    path::{Component, Path},
    time::Duration,
};

const MAX_SOURCE_BYTES: usize = 2 * 1024 * 1024;
const GIT_TIMEOUT_SECS: u64 = 15;
const MAX_WALK_FILES: usize = 5000;
const MAX_WALK_DEPTH: usize = 12;
const MAX_CALLER_SCAN: usize = 300;
const MAX_INDEX_SCAN_BYTES: usize = 64 * 1024 * 1024;
const MAX_RETAINED_SOURCE_BYTES: usize = 32 * 1024 * 1024;
const MAX_TOKENS_PER_FILE: usize = 4096;
const MAX_EXTENDS_HOPS: usize = 8;
const MAX_EXTENDS_VISITS: usize = 32;
const MAX_CYCLE_DEPTH: usize = 6;
const MAX_CYCLES_REPORTED: usize = 24;
const MAX_CYCLE_VISITS: usize = 20_000;
const MAX_EDGE_DIFFS: usize = 500;

const SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "vendor",
    ".next",
    "dist",
    "build",
    "__pycache__",
    ".venv",
    ".c2p",
];

const CODE_EXTENSIONS: &[&str] = &[
    "php", "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "pyi", "go", "java", "kt", "cs",
    "rb", "swift", "scala",
];

enum BlobRead {
    Ok(String),
    Missing,
    Unreadable(&'static str),
}

struct TargetContext<'a> {
    root: &'a Path,
    revision: Option<String>,
    base_commit: Option<String>,
    base_label: Option<String>,
    rev_files: Option<BTreeSet<String>>,
    disk_files: BTreeSet<String>,
    resolution_files: BTreeSet<String>,
    is_git: bool,
}

#[derive(Clone)]
struct MergedChange {
    status: String,
    previous_path: Option<String>,
}

#[derive(Default)]
struct CallerAccum {
    symbols: BTreeSet<String>,
    links: u64,
    guessed: u64,
    from_index: bool,
    source_after: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct VisibilityWarning {
    pub member: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_visibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub reason: String,
}

pub(crate) struct MemberAccessAssessment {
    pub access_risk: Option<String>,
    pub broken_by_change: bool,
    pub warning: Option<VisibilityWarning>,
}

struct TargetMemberProfile {
    members: Vec<DeclaredMember>,
}

struct CallerReport {
    path: String,
    exists: bool,
    impact: &'static str,
    reason: String,
    symbols: Vec<String>,
    changed_symbols: Vec<String>,
    used_members: Vec<DepMemberRef>,
    visibility_warnings: Vec<VisibilityWarning>,
    links: u64,
    guessed: u64,
    opened_by_run: bool,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Violation {
    from: String,
    to: String,
    from_layer_id: String,
    from_layer: String,
    to_layer_id: String,
    to_layer: String,
}

#[derive(Default)]
struct BaselineInfo {
    available: bool,
    label: String,
    reason: Option<String>,
}

impl BaselineInfo {
    fn to_json(&self) -> Value {
        json!({
            "available": self.available,
            "label": self.label,
            "reason": self.reason,
        })
    }
}

/// Lists all changed files across Git, active/historical run ledgers, and live
/// session focus, enriched with entity-level and dependency-level impact counts.
pub(crate) fn changed_catalog(
    root: &Path,
    index: Option<&Index>,
    resolved: Option<&Resolved>,
    live_focus: Option<&GraphFocus>,
    run_edited: &[(String, char)],
    target_label: &str,
    notests: bool,
) -> Value {
    let ctx = TargetContext::build(root, index, resolved, live_focus, run_edited);
    let mut merged = merge_changed_entries(&ctx, index, resolved, live_focus, run_edited);
    if notests {
        merged.retain(|path, _| !is_test_path(path));
    }

    // The callers the graph knows of each changed file, the strongest of
    // every file in turn, so that one much-used file does not take the
    // whole scan from the others.
    let per_file = merged
        .iter()
        .map(|(path, entry)| {
            index_callers(
                index,
                path,
                entry.previous_path.as_deref(),
                false,
                MAX_CALLER_SCAN,
            )
        })
        .collect::<Vec<_>>();
    let mut known_callers = Vec::new();
    let longest = per_file.iter().map(Vec::len).max().unwrap_or(0);
    for at in 0..longest {
        known_callers.extend(
            per_file
                .iter()
                .filter_map(|callers| callers.get(at).cloned()),
        );
        if known_callers.len() >= MAX_CALLER_SCAN * 2 {
            break;
        }
    }
    let scan_index = CallerScanIndex::build(&ctx, index, &merged, live_focus, &known_callers);
    let index_incoming = build_index_incoming(index);

    let mut existing_files = 0usize;
    let mut created_files = 0usize;
    let mut modified_files = 0usize;
    let mut deleted_files = 0usize;
    let mut renamed_files = 0usize;
    let mut created_entities = 0usize;
    let mut modified_entities = 0usize;
    let mut deleted_entities = 0usize;
    let mut files_json = Vec::with_capacity(merged.len());
    let mut added_edges: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    let mut removed_edges: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    let mut risk_callers_map: BTreeMap<(String, String), &'static str> = BTreeMap::new();
    let mut removed_callers_by_target: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut deleted_entries: Vec<(usize, String, Option<String>, BTreeSet<String>)> = Vec::new();

    for (path, entry) in &merged {
        let exists = if entry.status == "deleted" {
            false
        } else {
            ctx.exists_in_target(path)
        };
        if exists {
            existing_files += 1;
        }
        match entry.status.as_str() {
            "created" => created_files += 1,
            "deleted" => deleted_files += 1,
            "renamed" => renamed_files += 1,
            _ => modified_files += 1,
        }
        let in_graph = index.is_some_and(|idx| idx.file_id(path).is_some());

        let (before, _) =
            load_before_for_status(&ctx, path, entry.previous_path.as_deref(), &entry.status);
        let (after, _) = load_after_for_status(&ctx, path, exists, &entry.status);

        let entities = compute_changed_entities(
            path,
            &entry.status,
            before.as_deref(),
            after.as_deref(),
            live_focus,
        );
        for entity in &entities {
            match entity.change.as_str() {
                "created" => created_entities += 1,
                "modified" => modified_entities += 1,
                "deleted" => deleted_entities += 1,
                _ => {}
            }
        }

        let declared_members =
            build_declared_members_with_diff(path, before.as_deref(), after.as_deref(), &entities);

        let dep_diff = compute_uses_diff(
            path,
            &entry.status,
            exists,
            before.as_deref(),
            after.as_deref(),
            &ctx,
            index,
            &merged,
            live_focus,
            false,
        );
        record_edge_diffs(path, &dep_diff.added, &mut added_edges, None);
        record_edge_diffs(
            path,
            &dep_diff.removed,
            &mut removed_edges,
            Some(&mut removed_callers_by_target),
        );

        let target_symbols =
            collect_target_symbols(path, &entities, before.as_deref(), after.as_deref(), index);
        let callers = evaluate_callers_for_file(
            path,
            entry.previous_path.as_deref(),
            &entry.status,
            &entities,
            &declared_members,
            &target_symbols,
            &ctx,
            index,
            live_focus,
            resolved,
            false,
            &index_incoming,
            &scan_index,
        );
        for caller in &callers {
            if caller.path != *path
                && matches!(caller.impact, "broken_risk" | "impacted" | "added_caller")
            {
                risk_callers_map
                    .entry((caller.path.clone(), path.clone()))
                    .and_modify(|existing| {
                        if impact_rank(caller.impact) < impact_rank(existing) {
                            *existing = caller.impact;
                        }
                    })
                    .or_insert(caller.impact);
            }
        }
        let callers_at_risk = callers.iter().filter(|c| c.impact == "broken_risk").count();
        let impacted_callers = callers
            .iter()
            .filter(|c| matches!(c.impact, "broken_risk" | "impacted"))
            .count();
        let visibility_warnings: usize = callers.iter().map(|c| c.visibility_warnings.len()).sum();

        if entry.status == "deleted" {
            let mut del_callers: BTreeSet<String> = callers
                .iter()
                .map(|c| c.path.clone())
                .filter(|caller| caller != path)
                .collect();
            for caller in index_callers(
                index,
                path,
                entry.previous_path.as_deref(),
                false,
                MAX_CALLER_SCAN,
            ) {
                if caller != *path {
                    del_callers.insert(caller);
                }
            }
            deleted_entries.push((
                files_json.len(),
                path.clone(),
                entry.previous_path.clone(),
                del_callers,
            ));
        }

        files_json.push(json!({
            "path": path,
            "previous_path": entry.previous_path,
            "status": entry.status,
            "exists": exists,
            "in_graph": in_graph,
            "summary": {
                "uses_added": dep_diff.added.len(),
                "uses_removed": dep_diff.removed.len(),
                "uses_unchanged": dep_diff.unchanged.len(),
                "impacted_callers": impacted_callers,
                "callers_at_risk": callers_at_risk,
                "visibility_warnings": visibility_warnings,
            },
            "entities": entities,
        }));
    }

    for (idx, path, previous_path, mut del_callers) in deleted_entries {
        for key in [Some(path.as_str()), previous_path.as_deref()]
            .into_iter()
            .flatten()
        {
            if let Some(removed_callers) = removed_callers_by_target.get(key) {
                for caller in removed_callers {
                    if caller != &path {
                        del_callers.insert(caller.clone());
                    }
                }
            }
        }
        files_json[idx]["callers"] = json!(del_callers.into_iter().collect::<Vec<_>>());
    }

    let edge_diffs = build_edge_diffs_json(added_edges, removed_edges, risk_callers_map);

    json!({
        "target": target_label,
        "total_files": merged.len(),
        "existing_files": existing_files,
        "created_files": created_files,
        "modified_files": modified_files,
        "deleted_files": deleted_files,
        "renamed_files": renamed_files,
        "created_entities": created_entities,
        "modified_entities": modified_entities,
        "deleted_entities": deleted_entities,
        "files": files_json,
        "edge_diffs": edge_diffs,
    })
}

/// Accumulates deduplicated resolved outgoing dependency edges from one file's diff.
fn record_edge_diffs(
    from_path: &str,
    deps: &[DepRef],
    edges: &mut BTreeMap<(String, String), BTreeSet<String>>,
    mut callers_by_target: Option<&mut BTreeMap<String, BTreeSet<String>>>,
) {
    for dep in deps {
        if !dep.resolved || dep.target.is_empty() || dep.target == from_path {
            continue;
        }
        let kinds = edges
            .entry((from_path.to_owned(), dep.target.clone()))
            .or_default();
        if !dep.kind.is_empty() {
            kinds.insert(dep.kind.clone());
        }
        if let Some(map) = callers_by_target.as_deref_mut() {
            map.entry(dep.target.clone())
                .or_default()
                .insert(from_path.to_owned());
        }
    }
}

/// Serializes commit-wide `edge_diffs` (`added`, `removed`, `risk_callers`), bounded to `MAX_EDGE_DIFFS`.
fn build_edge_diffs_json(
    added_edges: BTreeMap<(String, String), BTreeSet<String>>,
    removed_edges: BTreeMap<(String, String), BTreeSet<String>>,
    risk_callers_map: BTreeMap<(String, String), &'static str>,
) -> Value {
    let serialize_edges = |map: BTreeMap<(String, String), BTreeSet<String>>| -> Vec<Value> {
        map.into_iter()
            .take(MAX_EDGE_DIFFS)
            .map(|((from, to), kinds)| {
                json!({
                    "from": from,
                    "to": to,
                    "kinds": kinds.into_iter().collect::<Vec<_>>(),
                })
            })
            .collect()
    };
    let mut risk_callers = risk_callers_map
        .into_iter()
        .map(|((from, to), impact)| (impact_rank(impact), from, to, impact))
        .collect::<Vec<_>>();
    risk_callers.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let risk_callers_json = risk_callers
        .into_iter()
        .take(MAX_EDGE_DIFFS)
        .map(|(_, from, to, impact)| {
            json!({
                "from": from,
                "to": to,
                "impact": impact,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "added": serialize_edges(added_edges),
        "removed": serialize_edges(removed_edges),
        "risk_callers": risk_callers_json,
    })
}

/// Computes detailed dependency, reverse-dependent, entity, cycle, and layer
/// violation impact for a single project-relative file path.
pub(crate) fn file_impact(
    root: &Path,
    index: Option<&Index>,
    resolved: Option<&Resolved>,
    live_focus: Option<&GraphFocus>,
    path: &str,
    certain: bool,
) -> Result<Value> {
    let rel_path = normalize_rel_path(path)?;
    let ctx = TargetContext::build(root, index, resolved, live_focus, &[]);
    let merged = merge_changed_entries(&ctx, index, resolved, live_focus, &[]);

    let in_graph = index.is_some_and(|idx| idx.file_id(&rel_path).is_some());
    let (status, previous_path) = match merged.get(&rel_path) {
        Some(entry) => {
            let normalized_status = match entry.status.as_str() {
                "renamed" => "modified",
                other => other,
            };
            (normalized_status.to_owned(), entry.previous_path.clone())
        }
        None => {
            let exists_now = ctx.exists_in_target(&rel_path);
            let exists_at_base = ctx
                .base_commit
                .as_deref()
                .is_some_and(|base| git_blob_exists(ctx.root, base, &rel_path));
            let inferred = if !exists_now && (exists_at_base || in_graph) {
                "deleted"
            } else {
                "unchanged"
            };
            (inferred.to_owned(), None)
        }
    };

    let exists = if status == "deleted" {
        false
    } else {
        ctx.exists_in_target(&rel_path)
    };

    let (before, before_err) =
        load_before_for_status(&ctx, &rel_path, previous_path.as_deref(), &status);
    let (after, after_err) = load_after_for_status(&ctx, &rel_path, exists, &status);

    let baseline = build_baseline_info(&ctx, &status, before.as_ref(), before_err, after_err);

    let entities = compute_changed_entities(
        &rel_path,
        &status,
        before.as_deref(),
        after.as_deref(),
        live_focus,
    );
    let declared_members =
        build_declared_members_with_diff(&rel_path, before.as_deref(), after.as_deref(), &entities);

    let dep_diff = compute_uses_diff(
        &rel_path,
        &status,
        exists,
        before.as_deref(),
        after.as_deref(),
        &ctx,
        index,
        &merged,
        live_focus,
        certain,
    );

    let target_symbols = collect_target_symbols(
        &rel_path,
        &entities,
        before.as_deref(),
        after.as_deref(),
        index,
    );
    let mut scan_scope = merged.clone();
    scan_scope.entry(rel_path.clone()).or_insert(MergedChange {
        status: status.clone(),
        previous_path: previous_path.clone(),
    });
    // The files the graph knows to use this one are read before any file
    // that only might: a scan that stops at its limit keeps the callers.
    let known_callers = index_callers(
        index,
        &rel_path,
        previous_path.as_deref(),
        certain,
        MAX_CALLER_SCAN,
    );
    let scan_index = CallerScanIndex::build(&ctx, index, &scan_scope, live_focus, &known_callers);
    let index_incoming = build_index_incoming(index);

    let used_by = evaluate_callers_for_file(
        &rel_path,
        previous_path.as_deref(),
        &status,
        &entities,
        &declared_members,
        &target_symbols,
        &ctx,
        index,
        live_focus,
        resolved,
        certain,
        &index_incoming,
        &scan_index,
    );

    let arch_caller_cache = scan_index.dep_cache.borrow();
    let (new_cycles, resolved_cycles, new_violations, resolved_violations) =
        compute_architecture_impact(
            &rel_path,
            &status,
            exists,
            &dep_diff,
            &used_by,
            &ctx,
            index,
            certain,
            &arch_caller_cache,
        );

    let callers_at_risk = used_by.iter().filter(|c| c.impact == "broken_risk").count();
    let callers_impacted = used_by.iter().filter(|c| c.impact == "impacted").count();
    let added_callers = used_by
        .iter()
        .filter(|c| c.impact == "added_caller")
        .count();
    let caller_visibility_warnings: usize =
        used_by.iter().map(|c| c.visibility_warnings.len()).sum();
    let uses_total = dep_diff.added.len() + dep_diff.unchanged.len();

    let (uses_added_json, added_vis_warnings) = serialize_uses_deps(&dep_diff.added, true);
    let (uses_removed_json, _) = serialize_uses_deps(&dep_diff.removed, false);
    let (uses_unchanged_json, unchanged_vis_warnings) =
        serialize_uses_deps(&dep_diff.unchanged, true);
    let total_visibility_warnings =
        caller_visibility_warnings + added_vis_warnings + unchanged_vis_warnings;

    let used_by_json = used_by
        .iter()
        .map(|caller| {
            json!({
                "path": caller.path,
                "exists": caller.exists,
                "impact": caller.impact,
                "reason": caller.reason,
                "symbols": caller.symbols,
                "changed_symbols": caller.changed_symbols,
                "used_members": caller.used_members,
                "visibility_warnings": caller.visibility_warnings,
                "links": caller.links,
                "guessed": caller.guessed,
                "opened_by_run": caller.opened_by_run,
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "path": rel_path,
        "previous_path": previous_path,
        "exists": exists,
        "status": status,
        "in_graph": in_graph,
        "baseline": baseline.to_json(),
        "summary": {
            "uses_total": uses_total,
            "uses_added": dep_diff.added.len(),
            "uses_removed": dep_diff.removed.len(),
            "uses_unchanged": dep_diff.unchanged.len(),
            "used_by_total": used_by.len(),
            "callers_at_risk": callers_at_risk,
            "callers_impacted": callers_impacted,
            "added_callers": added_callers,
            "visibility_warnings": total_visibility_warnings,
            "new_cycles": new_cycles.len(),
            "resolved_cycles": resolved_cycles.len(),
            "new_violations": new_violations.len(),
            "resolved_violations": resolved_violations.len(),
        },
        "declared_members": declared_members,
        "entities": entities,
        "uses": {
            "added": uses_added_json,
            "removed": uses_removed_json,
            "unchanged": uses_unchanged_json,
        },
        "used_by": used_by_json,
        "architecture_impact": {
            "new_cycles": new_cycles,
            "resolved_cycles": resolved_cycles,
            "new_violations": new_violations,
            "resolved_violations": resolved_violations,
        },
    }))
}

impl<'a> TargetContext<'a> {
    fn build(
        root: &'a Path,
        index: Option<&Index>,
        resolved: Option<&Resolved>,
        live_focus: Option<&GraphFocus>,
        run_edited: &[(String, char)],
    ) -> Self {
        let is_git = is_git_repo(root);
        let revision = resolved.and_then(|r| r.revision.clone());
        let (base_commit, base_label) = resolve_base(root, resolved, is_git);
        let rev_files = revision
            .as_deref()
            .and_then(|rev| git_tree_files(root, rev));
        let disk_files = if revision.is_none() {
            collect_workspace_files(root, is_git)
        } else {
            BTreeSet::new()
        };

        let mut resolution_files = BTreeSet::new();
        if let Some(tree) = &rev_files {
            resolution_files.extend(tree.iter().cloned());
        } else {
            resolution_files.extend(disk_files.iter().cloned());
        }
        if let Some(idx) = index {
            resolution_files.extend(idx.files().iter().cloned());
        }
        if let Some(res) = resolved {
            for change in &res.files {
                if let Some(p) = normalize_rel_opt(&change.path) {
                    resolution_files.insert(p);
                }
                if let Some(from) = change.from.as_deref().and_then(normalize_rel_opt) {
                    resolution_files.insert(from);
                }
            }
        }
        for (raw, _) in run_edited {
            if let Some(p) = normalize_rel_opt(raw) {
                resolution_files.insert(p);
            }
        }
        if let Some(lf) = live_focus {
            for raw in &lf.target_paths {
                if let Some(p) = normalize_rel_opt(raw) {
                    resolution_files.insert(p);
                }
            }
            for entity in &lf.created_entities {
                if let Some(p) = normalize_rel_opt(&entity.path) {
                    resolution_files.insert(p);
                }
            }
        }

        Self {
            root,
            revision,
            base_commit,
            base_label,
            rev_files,
            disk_files,
            resolution_files,
            is_git,
        }
    }

    fn exists_in_target(&self, rel_path: &str) -> bool {
        if let Some(rev) = self.revision.as_deref() {
            match &self.rev_files {
                Some(tree) => tree.contains(rel_path),
                None => git_blob_exists(self.root, rev, rel_path),
            }
        } else {
            self.disk_files.contains(rel_path) || is_within_root(self.root, rel_path)
        }
    }

    fn existing_files(&self) -> impl Iterator<Item = &str> {
        let source = if let Some(tree) = &self.rev_files {
            tree
        } else {
            &self.disk_files
        };
        source.iter().map(String::as_str)
    }

    fn read_after(&self, rel_path: &str) -> BlobRead {
        match self.revision.as_deref() {
            Some(rev) => git_read_text(self.root, rev, rel_path),
            None => disk_read_text(self.root, rel_path),
        }
    }

    fn read_before(&self, rel_path: &str) -> BlobRead {
        match self.base_commit.as_deref() {
            Some(base) => git_read_text(self.root, base, rel_path),
            None => BlobRead::Missing,
        }
    }

    fn manifest_root(&self) -> Option<&Path> {
        self.revision.is_none().then_some(self.root)
    }
}

fn resolve_base(
    root: &Path,
    resolved: Option<&Resolved>,
    is_git: bool,
) -> (Option<String>, Option<String>) {
    if let Some(side) = resolved.and_then(|r| r.base.as_ref()) {
        let short: String = side.commit.chars().take(12).collect();
        let label = if side.name == side.commit || short == side.name {
            short
        } else {
            format!("{} ({short})", side.name)
        };
        return (Some(side.commit.clone()), Some(label));
    }
    if let Some(rev) = resolved.and_then(|r| r.revision.as_deref()) {
        let parent = format!("{rev}^");
        if is_git && git_commit_exists(root, &parent) {
            return (Some(parent), Some("parent commit".to_owned()));
        }
        return (None, None);
    }
    if is_git && git_commit_exists(root, "HEAD") {
        return (Some("HEAD".to_owned()), Some("HEAD".to_owned()));
    }
    (None, None)
}

fn merge_changed_entries(
    ctx: &TargetContext<'_>,
    index: Option<&Index>,
    resolved: Option<&Resolved>,
    live_focus: Option<&GraphFocus>,
    run_edited: &[(String, char)],
) -> BTreeMap<String, MergedChange> {
    let mut map: BTreeMap<String, MergedChange> = BTreeMap::new();

    if let Some(res) = resolved {
        for change in &res.files {
            let Some(path) = normalize_rel_opt(&change.path) else {
                continue;
            };
            let prev = change.from.as_deref().and_then(normalize_rel_opt);
            let mut status = match change.status {
                'A' | 'C' => "created",
                'D' => "deleted",
                'R' => "renamed",
                _ => "modified",
            }
            .to_owned();
            if ctx.revision.is_none()
                && matches!(status.as_str(), "modified" | "renamed")
                && !ctx.exists_in_target(&path)
            {
                status = "deleted".to_owned();
            }
            map.insert(
                path,
                MergedChange {
                    status,
                    previous_path: prev,
                },
            );
        }
    }

    for (raw_path, ch) in run_edited {
        let Some(path) = normalize_rel_opt(raw_path) else {
            continue;
        };
        if map.contains_key(&path) {
            continue;
        }
        let exists = ctx.exists_in_target(&path);
        let status = match *ch {
            'A' => "created",
            'D' => "deleted",
            _ if ctx.revision.is_none() && !exists => "deleted",
            _ => "modified",
        }
        .to_owned();
        map.insert(
            path,
            MergedChange {
                status,
                previous_path: None,
            },
        );
    }

    if let Some(lf) = live_focus {
        let mut entity_files: BTreeMap<String, bool> = BTreeMap::new();
        for entity in &lf.created_entities {
            let Some(path) = normalize_rel_opt(&entity.path) else {
                continue;
            };
            let is_new_file = entity.kind == "file" && entity.change.as_deref() != Some("modified");
            let entry = entity_files.entry(path).or_insert(false);
            *entry |= is_new_file;
        }

        for (path, is_new_file) in entity_files {
            if map.contains_key(&path) {
                continue;
            }
            let exists = ctx.exists_in_target(&path);
            let in_base = ctx
                .base_commit
                .as_deref()
                .is_some_and(|base| git_blob_exists(ctx.root, base, &path));
            let in_index = index.is_some_and(|idx| idx.file_id(&path).is_some());
            let status = if !exists && (in_base || in_index) {
                "deleted"
            } else if is_new_file || (!in_base && !in_index) {
                "created"
            } else {
                "modified"
            }
            .to_owned();
            map.insert(
                path,
                MergedChange {
                    status,
                    previous_path: None,
                },
            );
        }

        let allow_target_path_fallback = resolved.is_none() || lf.phase == "running";
        if allow_target_path_fallback {
            for raw_path in &lf.target_paths {
                let Some(path) = normalize_rel_opt(raw_path) else {
                    continue;
                };
                if map.contains_key(&path) || !ctx.exists_in_target(&path) {
                    continue;
                }
                if resolved.is_some()
                    && let Some(base) = ctx.base_commit.as_deref()
                    && let (BlobRead::Ok(b), BlobRead::Ok(a)) =
                        (git_read_text(ctx.root, base, &path), ctx.read_after(&path))
                    && b == a
                {
                    continue;
                }
                let in_base = ctx
                    .base_commit
                    .as_deref()
                    .is_some_and(|base| git_blob_exists(ctx.root, base, &path));
                let in_index = index.is_some_and(|idx| idx.file_id(&path).is_some());
                let status = if !in_base && !in_index {
                    "created"
                } else {
                    "modified"
                }
                .to_owned();
                map.insert(
                    path,
                    MergedChange {
                        status,
                        previous_path: None,
                    },
                );
            }
        }
    }

    map
}

fn load_before_for_status(
    ctx: &TargetContext<'_>,
    path: &str,
    previous_path: Option<&str>,
    status: &str,
) -> (Option<String>, Option<&'static str>) {
    if status == "created" && previous_path.is_none() {
        return (None, None);
    }
    let lookup = previous_path.unwrap_or(path);
    match ctx.read_before(lookup) {
        BlobRead::Ok(text) => (Some(text), None),
        BlobRead::Missing => (None, None),
        BlobRead::Unreadable(reason) => (None, Some(reason)),
    }
}

fn load_after_for_status(
    ctx: &TargetContext<'_>,
    path: &str,
    exists: bool,
    status: &str,
) -> (Option<String>, Option<&'static str>) {
    if !exists || status == "deleted" {
        return (None, None);
    }
    match ctx.read_after(path) {
        BlobRead::Ok(text) => (Some(text), None),
        BlobRead::Missing => (None, None),
        BlobRead::Unreadable(reason) => (None, Some(reason)),
    }
}

fn build_baseline_info(
    ctx: &TargetContext<'_>,
    status: &str,
    before: Option<&String>,
    before_err: Option<&'static str>,
    after_err: Option<&'static str>,
) -> BaselineInfo {
    if let Some(reason) = before_err.or(after_err) {
        return BaselineInfo {
            available: false,
            label: ctx
                .base_label
                .clone()
                .unwrap_or_else(|| "Unavailable".to_owned()),
            reason: Some(reason.to_owned()),
        };
    }
    if status == "created" {
        return BaselineInfo {
            available: true,
            label: ctx
                .base_label
                .clone()
                .unwrap_or_else(|| "New file (empty baseline)".to_owned()),
            reason: None,
        };
    }
    if before.is_some() {
        return BaselineInfo {
            available: true,
            label: ctx.base_label.clone().unwrap_or_else(|| "HEAD".to_owned()),
            reason: None,
        };
    }
    if ctx.base_commit.is_some() {
        return BaselineInfo {
            available: false,
            label: ctx.base_label.clone().unwrap_or_else(|| "HEAD".to_owned()),
            reason: Some("File is not present in the baseline commit.".to_owned()),
        };
    }
    BaselineInfo {
        available: false,
        label: if ctx.is_git {
            "No commit yet".to_owned()
        } else {
            "No Git baseline".to_owned()
        },
        reason: Some(if ctx.is_git {
            "Repository has no commits yet; showing current file dependencies.".to_owned()
        } else {
            "Workspace is not a Git repository; showing current file dependencies.".to_owned()
        }),
    }
}

fn compute_changed_entities(
    rel_path: &str,
    status: &str,
    before: Option<&str>,
    after: Option<&str>,
    live_focus: Option<&GraphFocus>,
) -> Vec<EntityChange> {
    match status {
        "created" => diff_entities(rel_path, None, after),
        "deleted" => diff_entities(rel_path, before, None),
        "unchanged" => Vec::new(),
        _ => {
            if let Some(before_src) = before {
                diff_entities(rel_path, Some(before_src), after)
            } else if let Some(lf) = live_focus {
                lf.created_entities
                    .iter()
                    .filter(|e| {
                        e.kind != "file" && normalize_rel_opt(&e.path).as_deref() == Some(rel_path)
                    })
                    .map(|e| EntityChange {
                        id: e.id.clone(),
                        kind: e.kind.clone(),
                        change: e.change.clone().unwrap_or_else(|| "created".to_owned()),
                        name: e.name.clone(),
                        parent: e.parent.clone(),
                        path: rel_path.to_owned(),
                        line: e.line,
                        signature_changed: false,
                        visibility: e.visibility.clone(),
                        previous_visibility: None,
                        visibility_changed: false,
                        visibility_narrowed: false,
                        modifiers: e.modifiers.clone(),
                    })
                    .collect()
            } else {
                Vec::new()
            }
        }
    }
}

/// Normalizes a member name by trimming whitespace, trailing `()`, and a leading `$`.
pub(crate) fn norm_member_name(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches("()");
    trimmed
        .strip_prefix('$')
        .filter(|s| !s.is_empty())
        .unwrap_or(trimmed)
        .to_owned()
}

fn normalized_entity_parent<'a>(parent: Option<&'a str>, rel_path: &str) -> Option<&'a str> {
    let rel = clean_rel_key(rel_path);
    parent
        .map(str::trim)
        .filter(|p| !p.is_empty() && *p != rel_path && clean_rel_key(p) != rel)
}

fn map_entity_change_status(entity: &EntityChange) -> String {
    match entity.change.as_str() {
        "created" => "added".to_owned(),
        "deleted" => "removed".to_owned(),
        "modified" if entity.visibility_changed && !entity.signature_changed => {
            "visibility_changed".to_owned()
        }
        other => other.to_owned(),
    }
}

fn kinds_compatible(a: &str, b: &str) -> bool {
    fn norm(k: &str) -> &str {
        match k {
            "field" | "property" => "property",
            "const" | "constant" => "const",
            "fn" | "function" => "function",
            _ => k,
        }
    }
    a == b || norm(a) == norm(b)
}

fn find_entity_for_member<'a>(
    entities: &'a [EntityChange],
    member: &DeclaredMember,
    rel_path: &str,
    want_deleted: bool,
) -> Option<&'a EntityChange> {
    let m_norm = norm_member_name(&member.name);
    entities.iter().find(|e| {
        e.kind != "file"
            && ((want_deleted && e.change == "deleted") || (!want_deleted && e.change != "deleted"))
            && kinds_compatible(&e.kind, &member.kind)
            && norm_member_name(&e.name) == m_norm
            && normalized_entity_parent(e.parent.as_deref(), rel_path) == member.parent.as_deref()
    })
}

fn build_declared_members_with_diff(
    rel_path: &str,
    before: Option<&str>,
    after: Option<&str>,
    entities: &[EntityChange],
) -> Vec<DeclaredMember> {
    let mut members = after
        .map(|src| extract_declared_members(rel_path, src))
        .unwrap_or_default();

    for member in &mut members {
        if let Some(entity) = find_entity_for_member(entities, member, rel_path, false) {
            member.change = map_entity_change_status(entity);
            member.signature_changed = entity.signature_changed;
            member.visibility_narrowed = entity.visibility_narrowed;
            if entity.previous_visibility.is_some() {
                member.previous_visibility = entity.previous_visibility.clone();
            }
            if entity.visibility.is_some() {
                member.visibility = entity.visibility.clone();
            }
            if !entity.modifiers.is_empty() {
                member.modifiers = entity.modifiers.clone();
            }
        }
    }

    let before_members = before
        .map(|src| extract_declared_members(rel_path, src))
        .unwrap_or_default();

    let push_unique = |list: &mut Vec<DeclaredMember>, item: DeclaredMember| {
        let item_norm = norm_member_name(&item.name);
        let exists = list.iter().any(|existing| {
            norm_member_name(&existing.name) == item_norm
                && kinds_compatible(&existing.kind, &item.kind)
                && existing.parent == item.parent
                && existing.change == item.change
        });
        if !exists {
            list.push(item);
        }
    };

    for entity in entities
        .iter()
        .filter(|e| e.change == "deleted" && e.kind != "file")
    {
        let e_norm = norm_member_name(&entity.name);
        let e_parent = normalized_entity_parent(entity.parent.as_deref(), rel_path);
        let before_match = before_members.iter().find(|bm| {
            norm_member_name(&bm.name) == e_norm
                && kinds_compatible(&bm.kind, &entity.kind)
                && bm.parent.as_deref() == e_parent
        });
        let vis = entity
            .visibility
            .clone()
            .or_else(|| before_match.and_then(|bm| bm.visibility.clone()));
        let mods = if !entity.modifiers.is_empty() {
            entity.modifiers.clone()
        } else {
            before_match
                .map(|bm| bm.modifiers.clone())
                .unwrap_or_default()
        };
        push_unique(
            &mut members,
            DeclaredMember {
                name: before_match
                    .map(|bm| bm.name.clone())
                    .unwrap_or_else(|| entity.name.clone()),
                kind: entity.kind.clone(),
                parent: e_parent.map(str::to_owned),
                line: entity.line.or_else(|| before_match.and_then(|bm| bm.line)),
                signature: before_match.and_then(|bm| bm.signature.clone()),
                previous_visibility: entity.previous_visibility.clone().or_else(|| vis.clone()),
                visibility: vis,
                modifiers: mods,
                change: "removed".to_owned(),
                signature_changed: false,
                visibility_narrowed: false,
                used_by_caller: false,
            },
        );
    }

    if after.is_none() && !before_members.is_empty() {
        for mut bm in before_members {
            bm.previous_visibility = bm.visibility.clone();
            bm.change = "removed".to_owned();
            push_unique(&mut members, bm);
        }
    }

    members
}

/// Finds the best matching [`DeclaredMember`] for a referenced member name, kind, and parent.
pub(crate) fn find_matching_declared_member<'a>(
    target_members: &'a [DeclaredMember],
    name: &str,
    kind: Option<&str>,
    parent: Option<&str>,
) -> Option<&'a DeclaredMember> {
    let idx = find_matching_declared_member_index(target_members, name, kind, parent)?;
    target_members.get(idx)
}

fn find_matching_declared_member_index(
    target_members: &[DeclaredMember],
    name: &str,
    kind: Option<&str>,
    parent: Option<&str>,
) -> Option<usize> {
    let trimmed = name.trim().trim_end_matches("()");
    if trimmed.is_empty() {
        return None;
    }
    let (qual_parent, bare_name) = match trimmed.rsplit_once("::") {
        Some((p, n)) => (Some(p.trim()), n.trim()),
        None => match trimmed.rsplit_once('.') {
            Some((p, n)) => (Some(p.trim()), n.trim()),
            None => (None, trimmed),
        },
    };
    let effective_parent = parent
        .or(qual_parent)
        .map(str::trim)
        .filter(|p| !p.is_empty());
    let ref_norm = norm_member_name(bare_name);

    target_members
        .iter()
        .enumerate()
        .filter_map(|(idx, tm)| {
            let tm_norm = norm_member_name(&tm.name);
            if tm.name != trimmed && tm.name != bare_name && tm_norm != ref_norm {
                return None;
            }
            if let (Some(rp), Some(tp)) = (effective_parent, tm.parent.as_deref())
                && rp != tp
            {
                return None;
            }
            let mut score = 0u16;
            if tm.name == bare_name || tm.name == trimmed {
                score += 8;
            } else {
                score += 4;
            }
            if effective_parent.is_some() && tm.parent.as_deref() == effective_parent {
                score += 6;
            }
            if let Some(k) = kind
                && kinds_compatible(k, &tm.kind)
            {
                score += 4;
            }
            if tm.change != "removed" {
                score += 1;
            }
            Some((score, idx))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, idx)| idx)
}

const PACKAGE_MANIFESTS: &[&str] = &[
    "Cargo.toml",
    "composer.json",
    "package.json",
    "pyproject.toml",
];
const MONOREPO_CONTAINERS: &[&str] = &[
    "crates", "packages", "services", "apps", "modules", "plugins", "libs",
];

#[derive(Clone, Debug, Default)]
struct InheritanceClause {
    class_name: Option<String>,
    parents: Vec<String>,
    traits: Vec<String>,
}

fn strip_angle_generics(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut depth = 0usize;
    for ch in raw.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

fn leaf_type_ident(raw: &str) -> Option<String> {
    let cleaned = raw
        .trim()
        .trim_start_matches(['\\', ':'])
        .split(['{', ';', '(', '=', ' '])
        .next()?
        .trim();
    if cleaned.is_empty() {
        return None;
    }
    let leaf = cleaned
        .rsplit(['\\', '.', ':'])
        .next()
        .unwrap_or(cleaned)
        .trim();
    if leaf.is_empty()
        || !leaf
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_' || b == b'$')
    {
        return None;
    }
    let end = leaf
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
        .unwrap_or(leaf.len());
    let ident = &leaf[..end];
    (!ident.is_empty()).then(|| ident.to_owned())
}

fn find_word_boundary(haystack: &str, word: &str) -> Option<usize> {
    let bytes = haystack.as_bytes();
    let w_bytes = word.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    let mut start = 0usize;
    while let Some(pos) = haystack[start..].find(word) {
        let abs = start + pos;
        let left_ok = abs == 0 || !is_ident(bytes[abs - 1]);
        let right = abs + w_bytes.len();
        let right_ok = right >= bytes.len() || !is_ident(bytes[right]);
        if left_ok && right_ok {
            return Some(abs);
        }
        start = abs + w_bytes.len();
    }
    None
}

fn extract_parents_from_header_fragment(frag: &str, out: &mut Vec<String>) {
    let before_brace = frag.split(['{', ';']).next().unwrap_or(frag);
    if let Some(ext_pos) = find_word_boundary(before_brace, "extends") {
        let after = &before_brace[ext_pos + "extends".len()..];
        let stop = find_word_boundary(after, "implements")
            .into_iter()
            .chain(find_word_boundary(after, "where"))
            .min()
            .unwrap_or(after.len());
        let no_gen = strip_angle_generics(&after[..stop]);
        for part in no_gen.split(',') {
            if let Some(id) = leaf_type_ident(part)
                && !out.contains(&id)
            {
                out.push(id);
            }
        }
        return;
    }
    if let Some(open) = before_brace.find('(') {
        let rest = &before_brace[open + 1..];
        let close = rest.find(')').unwrap_or(rest.len());
        for part in rest[..close].split(',') {
            let trimmed = part.trim();
            if trimmed.contains('=') {
                continue;
            }
            if let Some(id) = leaf_type_ident(trimmed)
                && !out.contains(&id)
            {
                out.push(id);
            }
        }
        return;
    }
    if let Some(colon) = before_brace.find(" : ") {
        let after = &before_brace[colon + 3..];
        let stop = find_word_boundary(after, "where").unwrap_or(after.len());
        let no_gen = strip_angle_generics(&after[..stop]);
        for part in no_gen.split(',') {
            let cleaned = part
                .trim()
                .trim_start_matches("public ")
                .trim_start_matches("protected ")
                .trim_start_matches("private ");
            if let Some(id) = leaf_type_ident(cleaned)
                && !out.contains(&id)
            {
                out.push(id);
            }
        }
        return;
    }
    if let Some(lt) = before_brace.find(" < ") {
        let after = &before_brace[lt + 3..];
        if let Some(id) = leaf_type_ident(after)
            && !out.contains(&id)
        {
            out.push(id);
        }
    }
}

fn extract_use_traits_from_body_fragment(fragment: &str, traits: &mut Vec<String>) {
    for raw_stmt in fragment.split(';') {
        let stmt = raw_stmt.trim().trim_start_matches('{').trim();
        if let Some(rest) = stmt.strip_prefix("use ") {
            let head = rest.split('{').next().unwrap_or(rest);
            for part in head.split(',') {
                if let Some(id) = leaf_type_ident(part)
                    && !traits.contains(&id)
                {
                    traits.push(id);
                }
            }
        }
    }
}

fn extract_inheritance_clauses(src: &str) -> Vec<InheritanceClause> {
    let mut clauses: Vec<InheritanceClause> = Vec::new();
    let mut current: Option<InheritanceClause> = None;
    let mut in_header = false;
    let mut class_body_depth: Option<i32> = None;
    let mut brace_depth: i32 = 0;
    let mut in_block_comment = false;

    for raw_line in src.lines() {
        let mut line = raw_line.trim();
        if in_block_comment {
            if let Some(end) = line.find("*/") {
                in_block_comment = false;
                line = line[end + 2..].trim();
            } else {
                continue;
            }
        }
        if line.starts_with("/*") {
            if let Some(end) = line[2..].find("*/") {
                line = line[2 + end + 2..].trim();
            } else {
                in_block_comment = true;
                continue;
            }
        }
        if line.is_empty()
            || line.starts_with("//")
            || line.starts_with('#')
            || line.starts_with('*')
        {
            continue;
        }
        let code = line.split("//").next().unwrap_or(line).trim();
        if code.is_empty() {
            continue;
        }

        let kw_pos = ["class", "interface", "trait"]
            .into_iter()
            .filter_map(|kw| find_word_boundary(code, kw).map(|pos| (pos, kw)))
            .min_by_key(|(pos, _)| *pos);

        let starts_type_decl = kw_pos.is_some_and(|(pos, _)| {
            let prefix = code[..pos].trim();
            prefix.is_empty()
                || prefix.split_whitespace().all(|tok| {
                    matches!(
                        tok,
                        "abstract"
                            | "final"
                            | "readonly"
                            | "export"
                            | "default"
                            | "public"
                            | "private"
                            | "protected"
                            | "internal"
                            | "open"
                            | "sealed"
                            | "data"
                            | "declare"
                    )
                })
        });

        if starts_type_decl && let Some((pos, kw)) = kw_pos {
            if let Some(prev) = current.take() {
                clauses.push(prev);
            }
            let after_kw = code[pos + kw.len()..].trim_start();
            let class_name = leaf_type_ident(after_kw);
            let mut clause = InheritanceClause {
                class_name,
                parents: Vec::new(),
                traits: Vec::new(),
            };
            extract_parents_from_header_fragment(after_kw, &mut clause.parents);
            if let Some((_, body_after_brace)) = code.split_once('{') {
                extract_use_traits_from_body_fragment(body_after_brace, &mut clause.traits);
            }
            current = Some(clause);
            in_header = !code.contains('{') && !code.ends_with(':') && !code.ends_with(';');
            if code.contains('{') {
                class_body_depth = Some(brace_depth + 1);
            } else if code.ends_with(':') {
                class_body_depth = None;
                if let Some(done) = current.take() {
                    clauses.push(done);
                }
            }
        } else if in_header {
            if let Some(clause) = current.as_mut() {
                extract_parents_from_header_fragment(code, &mut clause.parents);
                if let Some((_, body_after_brace)) = code.split_once('{') {
                    extract_use_traits_from_body_fragment(body_after_brace, &mut clause.traits);
                }
            }
            if code.contains('{') {
                in_header = false;
                class_body_depth = Some(brace_depth + 1);
            } else if code.ends_with(':') || code.ends_with(';') {
                in_header = false;
                if let Some(done) = current.take() {
                    clauses.push(done);
                }
            }
        } else if class_body_depth.is_some_and(|d| brace_depth == d)
            && code.contains("use ")
            && let Some(clause) = current.as_mut()
        {
            extract_use_traits_from_body_fragment(code, &mut clause.traits);
        }

        let mut in_sq = false;
        let mut in_dq = false;
        let mut escaped = false;
        for b in code.bytes() {
            if escaped {
                escaped = false;
                continue;
            }
            match b {
                b'\\' if in_sq || in_dq => escaped = true,
                b'\'' if !in_dq => in_sq = !in_sq,
                b'"' if !in_sq => in_dq = !in_dq,
                b'{' if !in_sq && !in_dq => brace_depth += 1,
                b'}' if !in_sq && !in_dq => {
                    brace_depth -= 1;
                    if class_body_depth.is_some_and(|d| brace_depth < d) {
                        class_body_depth = None;
                        if let Some(done) = current.take() {
                            clauses.push(done);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(rem) = current {
        clauses.push(rem);
    }
    clauses
}

fn target_type_names(target_path: &str, target_members: &[DeclaredMember]) -> HashSet<String> {
    let mut target_types = HashSet::new();
    for m in target_members {
        if matches!(
            m.kind.as_str(),
            "class" | "abstract_class" | "trait" | "interface" | "struct"
        ) && !m.name.is_empty()
        {
            target_types.insert(m.name.clone());
        }
        if let Some(p) = &m.parent
            && !p.is_empty()
        {
            target_types.insert(p.clone());
        }
    }
    if let Some(stem) = file_stem_name(target_path) {
        target_types.insert(stem);
    }
    target_types
}

fn intra_file_extends(clauses: &[InheritanceClause], target_types: &HashSet<String>) -> bool {
    let mut by_name: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut seeds: Vec<&str> = Vec::new();
    for c in clauses {
        for p in c.parents.iter().chain(&c.traits) {
            if target_types.contains(p) {
                return true;
            }
            seeds.push(p.as_str());
        }
        if let Some(name) = c.class_name.as_deref() {
            let entry = by_name.entry(name).or_default();
            for p in c.parents.iter().chain(&c.traits) {
                entry.push(p.as_str());
            }
        }
    }
    let mut visited: HashSet<&str> = HashSet::new();
    let mut queue: VecDeque<(&str, usize)> = seeds.into_iter().map(|s| (s, 1)).collect();
    while let Some((name, hop)) = queue.pop_front() {
        if target_types.contains(name) {
            return true;
        }
        if hop >= MAX_EXTENDS_HOPS || !visited.insert(name) {
            continue;
        }
        if let Some(next_parents) = by_name.get(name) {
            for p in next_parents {
                queue.push_back((p, hop + 1));
            }
        }
    }
    false
}

/// Bounded multi-hop inheritance / trait traversal across files and intra-file class hierarchies
/// with cycle detection.
pub(crate) fn caller_extends_target_in(
    caller_path: &str,
    caller_source: Option<&str>,
    target_path: &str,
    target_members: &[DeclaredMember],
    known_files: &BTreeSet<String>,
    read_source: &mut dyn FnMut(&str) -> Option<String>,
) -> bool {
    let Some(src) = caller_source else {
        return false;
    };
    let target_types = target_type_names(target_path, target_members);
    if target_types.is_empty() {
        return false;
    }
    let caller_key = clean_rel_key(caller_path);
    let target_key = clean_rel_key(target_path);
    let mut source_cache: HashMap<String, Option<String>> = HashMap::new();
    source_cache.insert(caller_key.clone(), Some(src.to_owned()));

    let mut queue: VecDeque<(String, Option<String>, usize)> = VecDeque::new();
    let mut visited: HashSet<(String, String)> = HashSet::new();

    let enqueue_file_parents = |file: &str,
                                file_src: &str,
                                wanted: Option<&str>,
                                hop: usize,
                                queue: &mut VecDeque<(String, Option<String>, usize)>|
     -> bool {
        let clauses = extract_inheritance_clauses(file_src);
        let matching_clauses: Vec<&InheritanceClause> = match wanted {
            Some(w) => {
                let matched: Vec<&InheritanceClause> = clauses
                    .iter()
                    .filter(|c| c.class_name.as_deref() == Some(w))
                    .collect();
                if matched.is_empty() {
                    clauses.iter().collect()
                } else {
                    matched
                }
            }
            None => clauses.iter().collect(),
        };
        if wanted.is_none() && intra_file_extends(&clauses, &target_types) {
            return true;
        }
        let mut local_types: HashMap<&str, &InheritanceClause> = HashMap::new();
        for c in &clauses {
            if let Some(name) = c.class_name.as_deref() {
                local_types.insert(name, c);
            }
        }
        let mut parent_names: Vec<String> = Vec::new();
        let mut trait_names: HashSet<String> = HashSet::new();
        for c in &matching_clauses {
            for p in &c.parents {
                if target_types.contains(p) {
                    return true;
                }
                if !parent_names.contains(p) {
                    parent_names.push(p.clone());
                }
            }
            for t in &c.traits {
                if target_types.contains(t) {
                    return true;
                }
                if !parent_names.contains(t) {
                    parent_names.push(t.clone());
                }
                trait_names.insert(t.clone());
            }
        }
        if parent_names.is_empty() || hop >= MAX_EXTENDS_HOPS {
            return false;
        }

        let deps = extract_file_deps(file, file_src, known_files);
        for dep in &deps {
            if dep.resolved
                && clean_rel_key(&dep.target) == target_key
                && (dep.kind == "extends"
                    || (dep.kind == "uses"
                        && dep.symbols.iter().any(|s| trait_names.contains(s.as_str()))))
            {
                return true;
            }
        }

        for pname in parent_names {
            if local_types.contains_key(pname.as_str()) {
                queue.push_back((file.to_owned(), Some(pname.clone()), hop + 1));
            }
            let mut matched_dep = false;
            for dep in &deps {
                if !dep.resolved {
                    continue;
                }
                let is_inherit =
                    dep.kind == "extends" || (dep.kind == "uses" && trait_names.contains(&pname));
                if !is_inherit {
                    continue;
                }
                let dep_matches =
                    dep.symbols.iter().any(|s| {
                        leaf_type_ident(s).as_deref() == Some(pname.as_str()) || s == &pname
                    }) || file_stem_name(&dep.target).as_deref() == Some(pname.as_str());
                if dep_matches {
                    matched_dep = true;
                    queue.push_back((clean_rel_key(&dep.target), Some(pname.clone()), hop + 1));
                }
            }
            if !matched_dep {
                let dir = folder_of(file);
                for cand in known_files {
                    if folder_of(cand) == dir
                        && file_stem_name(cand).as_deref() == Some(pname.as_str())
                    {
                        let key = clean_rel_key(cand);
                        if key == target_key {
                            return true;
                        }
                        queue.push_back((key, Some(pname.clone()), hop + 1));
                    }
                }
            }
        }
        false
    };

    visited.insert((caller_key.clone(), String::new()));
    if enqueue_file_parents(&caller_key, src, None, 0, &mut queue) {
        return true;
    }

    while let Some((file, wanted, hop)) = queue.pop_front() {
        if file == target_key {
            return true;
        }
        if hop >= MAX_EXTENDS_HOPS || visited.len() >= MAX_EXTENDS_VISITS {
            continue;
        }
        let visit_key = (file.clone(), wanted.clone().unwrap_or_default());
        if !visited.insert(visit_key) {
            continue;
        }
        let next_src = source_cache
            .entry(file.clone())
            .or_insert_with(|| read_source(&file))
            .clone();
        let Some(file_src) = next_src else {
            continue;
        };
        if enqueue_file_parents(&file, &file_src, wanted.as_deref(), hop, &mut queue) {
            return true;
        }
    }
    false
}

fn manifest_root_of(
    rel_path: &str,
    root: Option<&Path>,
    known_files: Option<&BTreeSet<String>>,
) -> Option<String> {
    if root.is_none() && known_files.is_none() {
        return None;
    }
    let rel = clean_rel_key(rel_path);
    let mut folder = folder_of(&rel);
    loop {
        for manifest in PACKAGE_MANIFESTS {
            let candidate = if folder.is_empty() {
                (*manifest).to_owned()
            } else {
                format!("{folder}/{manifest}")
            };
            let in_known = known_files.is_some_and(|files| files.contains(&candidate));
            let on_disk = root.is_some_and(|r| r.join(&candidate).is_file());
            if in_known || on_disk {
                return Some(folder);
            }
        }
        if folder.is_empty() {
            break;
        }
        folder = folder_of(&folder);
    }
    None
}

fn package_scope_root(
    rel_path: &str,
    root: Option<&Path>,
    known_files: Option<&BTreeSet<String>>,
) -> (String, bool) {
    let rel = clean_rel_key(rel_path);
    let is_rust = rel.ends_with(".rs");
    if let Some(pkg_root) = manifest_root_of(&rel, root, known_files) {
        if is_rust {
            let inside = if pkg_root.is_empty() {
                rel.as_str()
            } else {
                rel.strip_prefix(&pkg_root)
                    .and_then(|rest| rest.strip_prefix('/'))
                    .unwrap_or("")
            };
            if inside.starts_with("tests/") {
                return (pkg_root, false);
            }
        }
        return (pkg_root, true);
    }
    if let Some(idx) = rel.find("/src/") {
        return (rel[..idx].to_owned(), true);
    }
    if rel.starts_with("src/") {
        return (String::new(), true);
    }
    if is_rust {
        if let Some(idx) = rel.find("/tests/") {
            return (rel[..idx].to_owned(), false);
        }
        if rel.starts_with("tests/") {
            return (String::new(), false);
        }
    }
    let segs: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    if segs.len() <= 1 {
        return (String::new(), true);
    }
    if segs.len() >= 3 && MONOREPO_CONTAINERS.contains(&segs[0]) {
        return (format!("{}/{}", segs[0], segs[1]), true);
    }
    (segs[0].to_owned(), true)
}

#[cfg(test)]
fn same_scope_crate_or_package(caller_path: &str, target_path: &str) -> bool {
    same_scope_crate_or_package_in(None, None, caller_path, target_path)
}

fn same_scope_crate_or_package_in(
    root: Option<&Path>,
    known_files: Option<&BTreeSet<String>>,
    caller_path: &str,
    target_path: &str,
) -> bool {
    let caller = clean_rel_key(caller_path);
    let target = clean_rel_key(target_path);
    let (c_root, c_in_src) = package_scope_root(&caller, root, known_files);
    let (t_root, t_in_src) = package_scope_root(&target, root, known_files);
    if !c_in_src || !t_in_src {
        return false;
    }
    c_root == t_root
}

fn rust_crate_src_dir_in(
    rel_path: &str,
    root: Option<&Path>,
    known_files: Option<&BTreeSet<String>>,
) -> String {
    let rel = clean_rel_key(rel_path);
    if let Some(idx) = rel.find("/src/") {
        rel[..idx + 4].to_owned()
    } else if rel.starts_with("src/") {
        "src".to_owned()
    } else if let Some(pkg_root) = manifest_root_of(&rel, root, known_files) {
        pkg_root
    } else {
        folder_of(&rel)
    }
}

fn rust_super_dir(target_path: &str) -> String {
    let target = clean_rel_key(target_path);
    let folder = folder_of(&target);
    let file_name = Path::new(&target)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if matches!(file_name, "mod.rs" | "lib.rs" | "main.rs") {
        let parent = folder_of(&folder);
        if parent.is_empty() { folder } else { parent }
    } else {
        folder
    }
}

fn path_in_rust_module_dir(caller_path: &str, mod_dir: &str) -> bool {
    if mod_dir.is_empty() {
        return true;
    }
    let caller = clean_rel_key(caller_path);
    let c_folder = folder_of(&caller);
    c_folder == mod_dir
        || c_folder.starts_with(&format!("{mod_dir}/"))
        || caller.strip_suffix(".rs") == Some(mod_dir)
}

fn within_rust_super_mod_in(
    root: Option<&Path>,
    known_files: Option<&BTreeSet<String>>,
    caller_path: &str,
    target_path: &str,
) -> bool {
    if !same_scope_crate_or_package_in(root, known_files, caller_path, target_path) {
        return false;
    }
    let super_dir = rust_super_dir(target_path);
    path_in_rust_module_dir(caller_path, &super_dir)
}

#[cfg(test)]
fn within_rust_pub_in(vis_lower: &str, caller_path: &str, target_path: &str) -> bool {
    within_rust_pub_in_in(None, None, vis_lower, caller_path, target_path)
}

fn within_rust_pub_in_in(
    root: Option<&Path>,
    known_files: Option<&BTreeSet<String>>,
    vis_lower: &str,
    caller_path: &str,
    target_path: &str,
) -> bool {
    if !same_scope_crate_or_package_in(root, known_files, caller_path, target_path) {
        return false;
    }
    let Some(inner) = vis_lower
        .strip_prefix("pub(in ")
        .and_then(|rest| rest.strip_suffix(')'))
        .map(str::trim)
    else {
        return false;
    };
    if inner.is_empty() {
        return false;
    }
    let raw_segs: Vec<&str> = inner.split("::").map(str::trim).collect();
    if raw_segs.iter().any(|s| s.is_empty()) {
        return false;
    }
    let target = clean_rel_key(target_path);
    let src_dir = rust_crate_src_dir_in(&target, root, known_files);
    let segs = raw_segs;

    if segs[0] == "self" {
        if segs.len() == 1 {
            return clean_rel_key(caller_path) == target;
        }
        if segs[1..]
            .iter()
            .any(|s| matches!(*s, "self" | "super" | "crate"))
        {
            return false;
        }
        let target_mod_dir = target
            .strip_suffix("/mod.rs")
            .or_else(|| target.strip_suffix(".rs"))
            .unwrap_or("");
        let full_dir = if target_mod_dir.is_empty() {
            segs[1..].join("/")
        } else {
            format!("{target_mod_dir}/{}", segs[1..].join("/"))
        };
        return path_in_rust_module_dir(&target, &full_dir)
            && path_in_rust_module_dir(caller_path, &full_dir);
    }

    if segs[0] == "crate" {
        if segs.len() == 1 {
            return true;
        }
        if segs[1..]
            .iter()
            .any(|s| matches!(*s, "self" | "super" | "crate"))
        {
            return false;
        }
        let mod_sub = segs[1..].join("/");
        let full_dir = if src_dir.is_empty() {
            mod_sub
        } else {
            format!("{src_dir}/{mod_sub}")
        };
        return path_in_rust_module_dir(&target, &full_dir)
            && path_in_rust_module_dir(caller_path, &full_dir);
    }

    if segs[0] == "super" {
        let mut dir = rust_super_dir(target_path);
        let in_crate_src =
            |d: &str| src_dir.is_empty() || d == src_dir || d.starts_with(&format!("{src_dir}/"));
        if !in_crate_src(&dir) {
            return false;
        }
        for seg in segs.iter().skip(1) {
            if *seg == "super" {
                if dir == src_dir || dir.is_empty() {
                    return false;
                }
                dir = folder_of(&dir);
                if !in_crate_src(&dir) {
                    return false;
                }
            } else if matches!(*seg, "self" | "crate") {
                return false;
            } else {
                dir = if dir.is_empty() {
                    (*seg).to_owned()
                } else {
                    format!("{dir}/{seg}")
                };
            }
        }
        return path_in_rust_module_dir(&target, &dir)
            && path_in_rust_module_dir(caller_path, &dir);
    }

    // Bare path scopes (e.g. Rust 2015 `pub(in app::internal)`): match only if the
    // visibility module prefix resolves to an enclosing module of `target` under `src_dir`.
    if segs
        .iter()
        .any(|s| matches!(*s, "self" | "super" | "crate"))
    {
        return false;
    }
    let joined = segs.join("/");
    let cand1 = if src_dir.is_empty() {
        joined
    } else {
        format!("{src_dir}/{joined}")
    };
    if path_in_rust_module_dir(&target, &cand1) {
        return path_in_rust_module_dir(caller_path, &cand1);
    }
    if segs.len() > 1 {
        let crate_root = package_scope_root(&target, root, known_files).0;
        let crate_name = crate_root.rsplit('/').next().unwrap_or("");
        if !crate_name.is_empty() && segs[0] == crate_name {
            let sub = segs[1..].join("/");
            let cand2 = if src_dir.is_empty() {
                sub
            } else {
                format!("{src_dir}/{sub}")
            };
            if path_in_rust_module_dir(&target, &cand2) {
                return path_in_rust_module_dir(caller_path, &cand2);
            }
        }
    }
    false
}

/// Evaluates accessibility, risk classification, and visibility warnings for `tm` from `caller_path`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn assess_member_access_in(
    root: Option<&Path>,
    known_files: Option<&BTreeSet<String>>,
    tm: &DeclaredMember,
    caller_path: &str,
    target_path: &str,
    caller_extends: bool,
    access: &str,
    ref_line: Option<u32>,
) -> MemberAccessAssessment {
    if tm.change == "removed" {
        return MemberAccessAssessment {
            access_risk: Some("deleted".to_owned()),
            broken_by_change: true,
            warning: None,
        };
    }

    let same_file = clean_rel_key(caller_path) == clean_rel_key(target_path);
    let is_write = access == "write";
    let mut inaccessible_reason: Option<String> = None;

    if !same_file {
        if is_write
            && tm
                .modifiers
                .iter()
                .any(|m| m.eq_ignore_ascii_case("readonly"))
        {
            inaccessible_reason = Some(format!(
                "Writes to readonly {} '{}' in {}",
                tm.kind, tm.name, target_path
            ));
        }

        if inaccessible_reason.is_none()
            && let Some(vis_raw) = tm.visibility.as_deref()
        {
            let lower = vis_raw.trim().to_ascii_lowercase();
            if lower.contains("private(set)") {
                if is_write {
                    inaccessible_reason = Some(format!(
                        "Writes to private(set) {} '{}' in {}",
                        tm.kind, tm.name, target_path
                    ));
                } else if lower.starts_with("protected") && !caller_extends {
                    inaccessible_reason = Some(format!(
                        "Accesses protected {} '{}' from non-subclass {}",
                        tm.kind, tm.name, caller_path
                    ));
                }
            } else if lower.contains("protected(set)") {
                if is_write && !caller_extends {
                    inaccessible_reason = Some(format!(
                        "Writes to protected(set) {} '{}' from non-subclass {}",
                        tm.kind, tm.name, caller_path
                    ));
                }
            } else if matches!(lower.as_str(), "private" | "#private" | "pub(self)")
                || lower.contains("private")
            {
                inaccessible_reason = Some(format!(
                    "Accesses {} {} '{}' in {} which is inaccessible from {}",
                    vis_raw.trim(),
                    tm.kind,
                    tm.name,
                    target_path,
                    caller_path
                ));
            } else if lower == "protected" || lower.contains("protected") {
                if !caller_extends {
                    inaccessible_reason = Some(format!(
                        "Accesses protected {} '{}' in {} from non-subclass {}",
                        tm.kind, tm.name, target_path, caller_path
                    ));
                }
            } else if matches!(lower.as_str(), "pub(crate)" | "crate" | "internal") {
                if !same_scope_crate_or_package_in(root, known_files, caller_path, target_path) {
                    inaccessible_reason = Some(format!(
                        "Accesses {} {} '{}' in {} across crate boundary from {}",
                        vis_raw.trim(),
                        tm.kind,
                        tm.name,
                        target_path,
                        caller_path
                    ));
                }
            } else if lower == "pub(super)" {
                if !within_rust_super_mod_in(root, known_files, caller_path, target_path) {
                    inaccessible_reason = Some(format!(
                        "Accesses pub(super) {} '{}' in {} outside parent module from {}",
                        tm.kind, tm.name, target_path, caller_path
                    ));
                }
            } else if lower.starts_with("pub(in ") {
                if !within_rust_pub_in_in(root, known_files, &lower, caller_path, target_path) {
                    inaccessible_reason = Some(format!(
                        "Accesses {} {} '{}' in {} outside restricted path from {}",
                        vis_raw.trim(),
                        tm.kind,
                        tm.name,
                        target_path,
                        caller_path
                    ));
                }
            } else if lower == "package"
                && folder_of(&clean_rel_key(caller_path)) != folder_of(&clean_rel_key(target_path))
            {
                inaccessible_reason = Some(format!(
                    "Accesses package-private {} '{}' in {} across package from {}",
                    tm.kind, tm.name, target_path, caller_path
                ));
            }
        }
    }

    let build_warning = |reason: String| VisibilityWarning {
        member: tm.name.clone(),
        kind: tm.kind.clone(),
        parent: tm.parent.clone(),
        visibility: tm.visibility.clone(),
        previous_visibility: tm.previous_visibility.clone(),
        line: ref_line.or(tm.line),
        reason,
    };

    if let Some(inacc_msg) = inaccessible_reason {
        let broken_by_change = tm.visibility_narrowed || tm.signature_changed;
        let reason = if tm.visibility_narrowed {
            format!(
                "Visibility narrowed from {} to {}; {}",
                tm.previous_visibility.as_deref().unwrap_or("unspecified"),
                tm.visibility.as_deref().unwrap_or("unspecified"),
                inacc_msg
            )
        } else {
            inacc_msg
        };
        return MemberAccessAssessment {
            access_risk: Some("inaccessible".to_owned()),
            broken_by_change,
            warning: Some(build_warning(reason)),
        };
    }

    if tm.signature_changed {
        let warning = if tm.visibility_narrowed {
            Some(build_warning(format!(
                "Visibility narrowed from {} to {} (still accessible from {}; non-breaking visibility change)",
                tm.previous_visibility.as_deref().unwrap_or("unspecified"),
                tm.visibility.as_deref().unwrap_or("unspecified"),
                caller_path
            )))
        } else {
            None
        };
        return MemberAccessAssessment {
            access_risk: Some("signature_changed".to_owned()),
            broken_by_change: true,
            warning,
        };
    }

    if tm.visibility_narrowed {
        return MemberAccessAssessment {
            access_risk: Some("visibility_narrowed".to_owned()),
            broken_by_change: false,
            warning: Some(build_warning(format!(
                "Visibility narrowed from {} to {} (still accessible from {}; non-breaking)",
                tm.previous_visibility.as_deref().unwrap_or("unspecified"),
                tm.visibility.as_deref().unwrap_or("unspecified"),
                caller_path
            ))),
        };
    }

    MemberAccessAssessment {
        access_risk: None,
        broken_by_change: false,
        warning: None,
    }
}

fn normalize_raw_access_risk(risk: Option<&str>) -> Option<String> {
    match risk.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("deleted") => Some("deleted".to_owned()),
        Some("signature_changed") => Some("signature_changed".to_owned()),
        Some("visibility_narrowed" | "narrowed") => Some("visibility_narrowed".to_owned()),
        Some("inaccessible" | "private" | "private_set" | "protected" | "readonly") => {
            Some("inaccessible".to_owned())
        }
        Some(other) if !other.is_empty() => Some(other.to_owned()),
        _ => None,
    }
}

/// Enriches a [`DepMemberRef`] using `target_members` and evaluates accessibility for `caller_path`.
pub(crate) fn enrich_member_ref_in(
    root: Option<&Path>,
    known_files: Option<&BTreeSet<String>>,
    mref: &mut DepMemberRef,
    target_members: &[DeclaredMember],
    caller_path: &str,
    target_path: &str,
    caller_extends: bool,
) -> MemberAccessAssessment {
    if let Some(tm) = find_matching_declared_member(
        target_members,
        &mref.name,
        Some(&mref.kind),
        mref.parent.as_deref(),
    ) {
        mref.name = tm.name.clone();
        mref.kind = tm.kind.clone();
        if mref.parent.is_none() {
            mref.parent = tm.parent.clone();
        }
        mref.visibility = tm.visibility.clone();
        mref.previous_visibility = tm.previous_visibility.clone();
        mref.change = Some(tm.change.clone());
        mref.modifiers = tm.modifiers.clone();
        let assess = assess_member_access_in(
            root,
            known_files,
            tm,
            caller_path,
            target_path,
            caller_extends,
            &mref.access,
            mref.line.or(tm.line),
        );
        mref.access_risk = assess.access_risk.clone();
        return assess;
    }

    mref.access_risk = normalize_raw_access_risk(mref.access_risk.as_deref());
    MemberAccessAssessment {
        access_risk: mref.access_risk.clone(),
        broken_by_change: matches!(
            mref.access_risk.as_deref(),
            Some("deleted" | "signature_changed")
        ),
        warning: None,
    }
}

fn deduplicate_and_sort_member_refs(refs: &mut Vec<DepMemberRef>) {
    let mut out: Vec<DepMemberRef> = Vec::with_capacity(refs.len());
    for candidate in refs.drain(..) {
        let c_norm = norm_member_name(&candidate.name);
        if let Some(existing) = out.iter_mut().find(|m| {
            norm_member_name(&m.name) == c_norm
                && kinds_compatible(&m.kind, &candidate.kind)
                && m.parent == candidate.parent
                && m.access == candidate.access
        }) {
            if existing
                .line
                .is_none_or(|prev| candidate.line.is_some_and(|l| l < prev))
            {
                existing.line = candidate.line;
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
            if existing.modifiers.is_empty() {
                existing.modifiers = candidate.modifiers;
            }
            if existing.access_risk.is_none() {
                existing.access_risk = candidate.access_risk;
            }
        } else {
            out.push(candidate);
        }
    }
    out.sort_by(|a, b| {
        a.line
            .unwrap_or(u32::MAX)
            .cmp(&b.line.unwrap_or(u32::MAX))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.access.cmp(&b.access))
    });
    *refs = out;
}

fn load_target_member_profile<'a>(
    target_path: &str,
    ctx: &TargetContext<'_>,
    merged: &BTreeMap<String, MergedChange>,
    live_focus: Option<&GraphFocus>,
    profiles: &'a mut HashMap<String, TargetMemberProfile>,
) -> &'a TargetMemberProfile {
    if !profiles.contains_key(target_path) {
        let entry = merged.get(target_path);
        let t_status = entry.map(|m| m.status.as_str()).unwrap_or_else(|| {
            if ctx.exists_in_target(target_path) {
                "unchanged"
            } else {
                "deleted"
            }
        });
        let t_prev = entry.and_then(|m| m.previous_path.as_deref());
        let (t_before, _) = load_before_for_status(ctx, target_path, t_prev, t_status);
        let t_exists = if t_status == "deleted" {
            false
        } else {
            ctx.exists_in_target(target_path)
        };
        let (t_after, _) = load_after_for_status(ctx, target_path, t_exists, t_status);
        let t_entities = compute_changed_entities(
            target_path,
            t_status,
            t_before.as_deref(),
            t_after.as_deref(),
            live_focus,
        );
        let members = build_declared_members_with_diff(
            target_path,
            t_before.as_deref(),
            t_after.as_deref(),
            &t_entities,
        );
        profiles.insert(target_path.to_owned(), TargetMemberProfile { members });
    }
    &profiles[target_path]
}

/// The kinds of dependency a file has on each of its targets, by target.
fn kinds_by_target<'a>(
    deps: impl Iterator<Item = &'a DepRef>,
) -> HashMap<String, BTreeSet<String>> {
    let mut kinds: HashMap<String, BTreeSet<String>> = HashMap::new();
    for dep in deps.filter(|dep| dep.resolved) {
        kinds
            .entry(clean_rel_key(&dep.target))
            .or_default()
            .insert(dep.kind.clone());
    }
    kinds
}

/// The kind of dependency a member reference makes by itself: a property,
/// a field or a constant is read or written, a method or a function is
/// called. A reference to a type is none of these.
pub(crate) fn member_ref_kind(mref: &DepMemberRef) -> Option<&'static str> {
    let own = access_kind(&mref.access)?;
    let fits = match own {
        "calls" => matches!(mref.kind.as_str(), "method" | "function"),
        _ => matches!(mref.kind.as_str(), "property" | "field" | "const"),
    };
    fits.then_some(own)
}

/// The kinds a member reference falls back to where the file has no
/// dependency of the reference's own kind on a target: the plainest first.
/// An import comes last, since it is given members only where it is the
/// file's one dependency on the target.
const GENERAL_KINDS: [&str; 7] = [
    "uses",
    "extends",
    "implements",
    "calls",
    "reads",
    "writes",
    "imports",
];

/// Whether a member reference is listed under a dependency of `kind`, so
/// that a file's dependencies on one target tell each reference once. A
/// write, a read or a call belongs to the dependency of its own kind; where
/// the file has none on that target, and for a reference to a type, it
/// goes to the plainest dependency there is.
fn member_ref_listed(kind: &str, mref: &DepMemberRef, siblings: Option<&BTreeSet<String>>) -> bool {
    let Some(siblings) = siblings else {
        return true;
    };
    let home = member_ref_kind(mref)
        .filter(|own| siblings.contains(*own))
        .or_else(|| {
            GENERAL_KINDS
                .into_iter()
                .find(|general| siblings.contains(*general))
        });
    home.is_none_or(|home| home == kind)
}

#[allow(clippy::too_many_arguments)]
fn enrich_dep_bucket(
    caller_path: &str,
    caller_src: Option<&str>,
    deps: &mut [DepRef],
    non_import_targets: &HashSet<String>,
    kinds: &HashMap<String, BTreeSet<String>>,
    ctx: &TargetContext<'_>,
    merged: &BTreeMap<String, MergedChange>,
    live_focus: Option<&GraphFocus>,
    profiles: &mut HashMap<String, TargetMemberProfile>,
) {
    for dep in deps.iter_mut() {
        if !dep.resolved {
            continue;
        }
        let target_key = clean_rel_key(&dep.target);
        if target_key.is_empty() {
            continue;
        }
        let target_members =
            load_target_member_profile(&dep.target, ctx, merged, live_focus, profiles)
                .members
                .clone();
        dep.target_members = target_members;

        let caller_extends = caller_extends_target_in(
            caller_path,
            caller_src,
            &dep.target,
            &dep.target_members,
            &ctx.resolution_files,
            &mut |p| ctx.read_after(p).into_opt(),
        );
        let should_supplement = dep.kind != "imports" || !non_import_targets.contains(&target_key);
        if should_supplement && let Some(src) = caller_src {
            let matched = find_matching_member_references(
                caller_path,
                src,
                &dep.target,
                &dep.target_members,
                &ctx.resolution_files,
            );
            dep.member_refs.extend(matched);
        }

        for mref in &mut dep.member_refs {
            enrich_member_ref_in(
                ctx.manifest_root(),
                Some(&ctx.resolution_files),
                mref,
                &dep.target_members,
                caller_path,
                &dep.target,
                caller_extends,
            );
        }
        deduplicate_and_sort_member_refs(&mut dep.member_refs);

        // What the caller uses of the target is marked from every reference;
        // the list of this dependency then keeps the references of its kind.
        for mref in &dep.member_refs {
            if let Some(pos) = find_matching_declared_member_index(
                &dep.target_members,
                &mref.name,
                Some(&mref.kind),
                mref.parent.as_deref(),
            ) {
                dep.target_members[pos].used_by_caller = true;
            }
        }
        let siblings = kinds.get(&target_key);
        let kind = dep.kind.clone();
        dep.member_refs
            .retain(|mref| member_ref_listed(&kind, mref, siblings));
    }
}

#[allow(clippy::too_many_arguments)]
fn enrich_uses_diff(
    rel_path: &str,
    before: Option<&str>,
    after: Option<&str>,
    ctx: &TargetContext<'_>,
    merged: &BTreeMap<String, MergedChange>,
    live_focus: Option<&GraphFocus>,
    diff: &mut DepDiff,
) {
    let mut profiles: HashMap<String, TargetMemberProfile> = HashMap::new();
    let non_import_active: HashSet<String> = diff
        .added
        .iter()
        .chain(diff.unchanged.iter())
        .filter(|d| d.resolved && d.kind != "imports")
        .map(|d| clean_rel_key(&d.target))
        .collect();
    let non_import_removed: HashSet<String> = diff
        .removed
        .iter()
        .filter(|d| d.resolved && d.kind != "imports")
        .map(|d| clean_rel_key(&d.target))
        .collect();

    // What the file has on a target now, and what it had before the change.
    let kinds_after = kinds_by_target(diff.added.iter().chain(&diff.unchanged));
    let kinds_before = kinds_by_target(diff.removed.iter().chain(&diff.unchanged));

    enrich_dep_bucket(
        rel_path,
        after,
        &mut diff.added,
        &non_import_active,
        &kinds_after,
        ctx,
        merged,
        live_focus,
        &mut profiles,
    );
    enrich_dep_bucket(
        rel_path,
        after.or(before),
        &mut diff.unchanged,
        &non_import_active,
        &kinds_after,
        ctx,
        merged,
        live_focus,
        &mut profiles,
    );
    enrich_dep_bucket(
        rel_path,
        before,
        &mut diff.removed,
        &non_import_removed,
        &kinds_before,
        ctx,
        merged,
        live_focus,
        &mut profiles,
    );
}

fn serialize_uses_deps(deps: &[DepRef], emit_warnings: bool) -> (Vec<Value>, usize) {
    let mut out = Vec::with_capacity(deps.len());
    let mut warning_total = 0usize;

    for dep in deps {
        let mut warnings: Vec<VisibilityWarning> = Vec::new();
        if emit_warnings {
            for mref in &dep.member_refs {
                let Some(risk) = mref.access_risk.as_deref() else {
                    continue;
                };
                if !matches!(risk, "inaccessible" | "visibility_narrowed") {
                    continue;
                }
                let tm = find_matching_declared_member(
                    &dep.target_members,
                    &mref.name,
                    Some(&mref.kind),
                    mref.parent.as_deref(),
                );
                let prev = mref
                    .previous_visibility
                    .as_deref()
                    .or_else(|| tm.and_then(|t| t.previous_visibility.as_deref()))
                    .unwrap_or("unspecified");
                let curr = mref
                    .visibility
                    .as_deref()
                    .or_else(|| tm.and_then(|t| t.visibility.as_deref()))
                    .unwrap_or("unspecified");
                let narrowed = tm.is_some_and(|t| t.visibility_narrowed)
                    || (mref.previous_visibility.is_some()
                        && mref.visibility.is_some()
                        && mref.previous_visibility != mref.visibility);
                let reason = if risk == "inaccessible" {
                    if narrowed {
                        format!(
                            "Visibility narrowed from {prev} to {curr}; {} '{}' in {} is inaccessible",
                            mref.kind, mref.name, dep.target
                        )
                    } else if mref.access == "write" {
                        format!(
                            "Writes to restricted ({curr}) {} '{}' in {}",
                            mref.kind, mref.name, dep.target
                        )
                    } else {
                        format!(
                            "Accesses inaccessible ({curr}) {} '{}' in {}",
                            mref.kind, mref.name, dep.target
                        )
                    }
                } else {
                    format!(
                        "Visibility narrowed from {prev} to {curr} for {} '{}' in {} (still accessible)",
                        mref.kind, mref.name, dep.target
                    )
                };
                warnings.push(VisibilityWarning {
                    member: mref.name.clone(),
                    kind: mref.kind.clone(),
                    parent: mref.parent.clone(),
                    visibility: mref.visibility.clone(),
                    previous_visibility: mref.previous_visibility.clone(),
                    line: mref.line,
                    reason,
                });
            }
        }

        warning_total += warnings.len();
        let mut val = serde_json::to_value(dep).unwrap_or_else(|_| json!({}));
        if let Some(obj) = val.as_object_mut() {
            obj.insert("visibility_warnings".to_owned(), json!(warnings));
        }
        out.push(val);
    }

    (out, warning_total)
}

#[allow(clippy::too_many_arguments)]
fn compute_uses_diff(
    rel_path: &str,
    status: &str,
    exists: bool,
    before: Option<&str>,
    after: Option<&str>,
    ctx: &TargetContext<'_>,
    index: Option<&Index>,
    merged: &BTreeMap<String, MergedChange>,
    live_focus: Option<&GraphFocus>,
    certain: bool,
) -> DepDiff {
    let mut diff = if status == "unchanged" {
        if before.is_some() {
            diff_file_deps(rel_path, before, after, &ctx.resolution_files)
        } else {
            diff_file_deps(rel_path, after, after, &ctx.resolution_files)
        }
    } else if status == "modified" && before.is_none() {
        // Non-Git modified file with no baseline: split against Index if available,
        // otherwise report current dependencies as unchanged rather than false added.
        let current = after
            .map(|src| extract_file_deps(rel_path, src, &ctx.resolution_files))
            .unwrap_or_default();
        if let Some(idx) = index
            && let Some(fid) = idx.file_id(rel_path)
        {
            let indexed_targets: HashSet<&str> = idx
                .pairs()
                .iter()
                .filter(|(from, to, pair)| *from == fid && *to != fid && pair.weight(certain) > 0)
                .map(|(_, to, _)| idx.files()[*to as usize].as_str())
                .collect();
            let mut out = DepDiff::default();
            for dep in current {
                if dep.resolved && !indexed_targets.contains(dep.target.as_str()) {
                    out.added.push(dep);
                } else {
                    out.unchanged.push(dep);
                }
            }
            out
        } else {
            DepDiff {
                added: Vec::new(),
                removed: Vec::new(),
                unchanged: current,
            }
        }
    } else {
        diff_file_deps(rel_path, before, after, &ctx.resolution_files)
    };

    for list in [&mut diff.added, &mut diff.removed, &mut diff.unchanged] {
        for dep in list.iter_mut() {
            dep.exists = dep.resolved && ctx.exists_in_target(&dep.target);
        }
    }

    if exists
        && let Some(idx) = index
        && let Some(fid) = idx.file_id(rel_path)
    {
        merge_index_outgoing_edges(
            rel_path, fid, status, before, after, ctx, idx, certain, &mut diff,
        );
    }

    enrich_uses_diff(rel_path, before, after, ctx, merged, live_focus, &mut diff);

    sort_dep_refs(&mut diff.added);
    sort_dep_refs(&mut diff.removed);
    sort_dep_refs(&mut diff.unchanged);
    diff
}

#[allow(clippy::too_many_arguments)]
fn merge_index_outgoing_edges(
    _rel_path: &str,
    fid: u32,
    status: &str,
    before: Option<&str>,
    after: Option<&str>,
    ctx: &TargetContext<'_>,
    idx: &Index,
    certain: bool,
    diff: &mut DepDiff,
) {
    let mut covered: HashSet<String> = HashSet::new();
    for dep in diff
        .added
        .iter()
        .chain(diff.removed.iter())
        .chain(diff.unchanged.iter())
    {
        if dep.resolved {
            covered.insert(dep.target.clone());
        }
    }

    let mut per_target_syms: HashMap<u32, (BTreeSet<String>, Option<u32>)> = HashMap::new();
    let syms = idx.symbol_entries();
    for &(from_sym, to_sym, _, guessed) in idx.uses() {
        if certain && guessed {
            continue;
        }
        let s_from = &syms[from_sym as usize];
        let s_to = &syms[to_sym as usize];
        if s_from.file == fid && s_to.file != fid {
            let entry = per_target_syms.entry(s_to.file).or_default();
            if s_to.kind != Kind::File && s_to.kind != Kind::Note {
                let label = idx.symbol_label(to_sym);
                if !label.is_empty() {
                    entry.0.insert(label.to_owned());
                }
            }
            if s_from.line > 0 {
                entry.1 = Some(entry.1.map_or(s_from.line, |prev| prev.min(s_from.line)));
            }
        }
    }

    for (from, to, pair) in idx.pairs() {
        if *from != fid || *to == fid || pair.weight(certain) == 0 {
            continue;
        }
        let target_path = &idx.files()[*to as usize];
        let (sym_set, first_line) = per_target_syms.get(to).cloned().unwrap_or_default();

        if covered.contains(target_path) {
            if !sym_set.is_empty() {
                for existing in &mut diff.unchanged {
                    if existing.resolved
                        && existing.target == *target_path
                        && existing.symbols.is_empty()
                    {
                        existing.symbols = sym_set.iter().cloned().collect();
                    }
                }
            }
            continue;
        }

        let dominant_kind = pair
            .kinds
            .iter()
            .max_by_key(|(_, count)| *count)
            .map(|(kid, _)| map_index_relation(idx.kind_name(*kid)))
            .unwrap_or("uses");
        let symbols_vec: Vec<String> = sym_set.into_iter().collect();
        let dep_ref = DepRef {
            target: target_path.clone(),
            resolved: true,
            exists: ctx.exists_in_target(target_path),
            kind: dominant_kind.to_owned(),
            symbols: symbols_vec.clone(),
            line: first_line,
            evidence: Some(format!(
                "Graph index ({} link{})",
                pair.weight(certain),
                if pair.weight(certain) == 1 { "" } else { "s" }
            )),
            member_refs: Vec::new(),
            target_members: Vec::new(),
        };

        if status == "unchanged" || before.is_none() {
            diff.unchanged.push(dep_ref);
            continue;
        }

        if let (Some(before_src), Some(after_src)) = (before, after) {
            let mut probe_names = symbols_vec;
            if let Some(stem) = file_stem_name(target_path) {
                probe_names.push(stem);
            }
            let in_after = !probe_names.is_empty()
                && probe_names
                    .iter()
                    .any(|name| contains_identifier(after_src, name));
            let in_before = !probe_names.is_empty()
                && probe_names
                    .iter()
                    .any(|name| contains_identifier(before_src, name));
            if in_after || !in_before {
                diff.unchanged.push(dep_ref);
            } else {
                diff.removed.push(dep_ref);
            }
        }
    }
}

/// The kind of dependency a relation of the graph stands for. The same
/// seven kinds as the map's arrows, so a dependency read from the graph and
/// one read from the source are told alike.
fn map_index_relation(relation: &str) -> &'static str {
    match relation {
        "imports" | "imports_from" | "includes" | "requires" | "dynamic_import" | "re_exports" => {
            "imports"
        }
        "calls" | "instantiates" | "indirect_call" | "dispatches_to" => "calls",
        "inherits" | "extends" => "extends",
        "implements" | "uses_trait" => "implements",
        "writes" | "mutates" | "sets" => "writes",
        "reads" | "references_constant" | "uses_const" | "uses_static_prop" => "reads",
        _ => "uses",
    }
}

fn sort_dep_refs(refs: &mut [DepRef]) {
    refs.sort_by(|a, b| a.target.cmp(&b.target).then(a.kind.cmp(&b.kind)));
}

fn collect_target_symbols(
    rel_path: &str,
    entities: &[EntityChange],
    before: Option<&str>,
    after: Option<&str>,
    index: Option<&Index>,
) -> Vec<String> {
    let mut names = BTreeSet::new();
    for e in entities {
        if e.kind != "file" && !e.name.is_empty() {
            names.insert(e.name.clone());
        }
    }
    for src in [before, after].into_iter().flatten() {
        for e in diff_entities(rel_path, None, Some(src)) {
            if e.kind != "file" && !e.name.is_empty() {
                names.insert(e.name);
            }
        }
    }
    if let Some(idx) = index
        && let Some(fid) = idx.file_id(rel_path)
    {
        for at in idx.file_range(fid) {
            let sym = &idx.symbol_entries()[at as usize];
            if !matches!(sym.kind, Kind::File | Kind::Note) {
                let label = idx.symbol_label(at);
                if !label.is_empty() {
                    names.insert(label.trim_end_matches("()").to_owned());
                }
            }
        }
    }
    names.into_iter().collect()
}

fn build_index_incoming(index: Option<&Index>) -> HashMap<u32, Vec<(u32, String, bool)>> {
    let mut incoming: HashMap<u32, Vec<(u32, String, bool)>> = HashMap::new();
    let Some(idx) = index else {
        return incoming;
    };
    let syms = idx.symbol_entries();
    for &(from_sym, to_sym, _, guessed) in idx.uses() {
        let s_from = &syms[from_sym as usize];
        let s_to = &syms[to_sym as usize];
        if s_from.file != s_to.file {
            let label = if matches!(s_to.kind, Kind::File | Kind::Note) {
                String::new()
            } else {
                idx.symbol_label(to_sym).to_owned()
            };
            incoming
                .entry(s_to.file)
                .or_default()
                .push((s_from.file, label, guessed));
        }
    }
    incoming
}

/// The files the graph links to `rel_path`, or to the path it had before a
/// rename: the most linked first, then by path, at most `limit` of them.
fn index_callers(
    index: Option<&Index>,
    rel_path: &str,
    previous_path: Option<&str>,
    certain: bool,
    limit: usize,
) -> Vec<String> {
    let Some(idx) = index else {
        return Vec::new();
    };
    let targets = [
        idx.file_id(rel_path),
        previous_path.and_then(|old| idx.file_id(old)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    if targets.is_empty() {
        return Vec::new();
    }
    let symbols = idx.symbol_entries();
    let mut links: HashMap<u32, u64> = HashMap::new();
    for &(from_sym, to_sym, _, guessed) in idx.uses() {
        if certain && guessed {
            continue;
        }
        let (from_file, to_file) = (
            symbols[from_sym as usize].file,
            symbols[to_sym as usize].file,
        );
        if targets.contains(&to_file) && !targets.contains(&from_file) {
            *links.entry(from_file).or_default() += 1;
        }
    }
    let mut callers = links
        .into_iter()
        .map(|(file, links)| (links, idx.files()[file as usize].as_str()))
        .filter(|(_, path)| *path != rel_path)
        .collect::<Vec<_>>();
    callers.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    callers
        .into_iter()
        .take(limit)
        .map(|(_, path)| path.to_owned())
        .collect()
}

fn is_scan_stop_word(tok: &str) -> bool {
    matches!(
        tok,
        "php"
            | "use"
            | "class"
            | "struct"
            | "enum"
            | "trait"
            | "interface"
            | "impl"
            | "function"
            | "fn"
            | "pub"
            | "crate"
            | "super"
            | "self"
            | "let"
            | "const"
            | "var"
            | "mut"
            | "static"
            | "return"
            | "true"
            | "false"
            | "null"
            | "none"
            | "some"
            | "void"
            | "int"
            | "bool"
            | "string"
            | "str"
            | "array"
            | "public"
            | "protected"
            | "private"
            | "readonly"
            | "this"
            | "if"
            | "else"
            | "for"
            | "while"
            | "match"
            | "import"
            | "from"
            | "export"
            | "default"
            | "require"
            | "include"
            | "def"
            | "async"
            | "await"
            | "new"
            | "namespace"
            | "mod"
    )
}

fn extract_ident_tokens(src: &str) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut tokens: Vec<String> = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'$' {
            let start = i;
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$')
            {
                i += 1;
            }
            let raw = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
            let trimmed = raw.trim_start_matches('$');
            if trimmed.len() >= 2 && trimmed.len() <= 96 {
                let lower = trimmed.to_ascii_lowercase();
                if !is_scan_stop_word(&lower) && seen.insert(lower.clone()) {
                    tokens.push(lower);
                    if tokens.len() >= MAX_TOKENS_PER_FILE {
                        break;
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    tokens
}

fn push_query_tokens_from_path(path: &str, out: &mut BTreeSet<String>) {
    if let Some(stem) = file_stem_name(path) {
        for tok in extract_ident_tokens(&stem) {
            out.insert(tok);
        }
        let lower_stem = stem.to_ascii_lowercase();
        if lower_stem.len() >= 2 && !is_scan_stop_word(&lower_stem) {
            out.insert(lower_stem.clone());
        }
        if matches!(
            lower_stem.as_str(),
            "mod" | "index" | "__init__" | "lib" | "main"
        ) {
            let folder = folder_of(&clean_rel_key(path));
            if let Some(seg) = folder.rsplit('/').next()
                && !seg.is_empty()
                && !matches!(seg, "src" | "app" | "lib" | "tests" | "test")
            {
                for tok in extract_ident_tokens(seg) {
                    out.insert(tok);
                }
            }
        }
    }
    let cleaned = clean_rel_key(path);
    for seg in cleaned.split('/') {
        if !seg.is_empty() && !matches!(seg, "src" | "app" | "lib" | "tests" | "test") {
            let no_ext = Path::new(seg)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(seg);
            for tok in extract_ident_tokens(no_ext) {
                out.insert(tok);
            }
        }
    }
}

fn parse_batch_blob_payloads(
    stdout: &[u8],
    chunk: &[(&str, &str)],
    out: &mut HashMap<String, String>,
) -> bool {
    let mut pos = 0usize;
    for &(path, _) in chunk {
        let Some(nl_rel) = stdout[pos..].iter().position(|&b| b == b'\n') else {
            return false;
        };
        let header = std::str::from_utf8(&stdout[pos..pos + nl_rel]).unwrap_or("");
        pos += nl_rel + 1;
        let parts: Vec<&str> = header.split_ascii_whitespace().collect();
        if parts.len() == 3 && parts[1] == "blob" {
            let Ok(sz) = parts[2].parse::<usize>() else {
                return false;
            };
            if pos + sz > stdout.len() {
                return false;
            }
            if let BlobRead::Ok(text) = bytes_to_blob(stdout[pos..pos + sz].to_vec(), "File") {
                out.insert(path.to_owned(), text);
            }
            pos += sz;
            if stdout.get(pos) == Some(&b'\n') {
                pos += 1;
            }
        }
    }
    true
}

fn read_revision_blobs_batched(
    root: &Path,
    rev: &str,
    paths: &[String],
    max_total_bytes: usize,
) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let safe_paths: Vec<&str> = paths
        .iter()
        .map(String::as_str)
        .filter(|p| !p.contains('\n') && !p.contains('\r'))
        .collect();
    if safe_paths.is_empty() {
        return out;
    }

    let mut check_input = Vec::with_capacity(safe_paths.len() * 48);
    for path in &safe_paths {
        check_input.extend_from_slice(rev.as_bytes());
        check_input.extend_from_slice(b":./");
        check_input.extend_from_slice(path.as_bytes());
        check_input.push(b'\n');
    }
    let Ok(check_out) = infra_core::git::run(
        root,
        &infra_core::git::strings(&["cat-file", "--batch-check"]),
        Some(check_input),
        Duration::from_secs(GIT_TIMEOUT_SECS),
        8 * 1024 * 1024,
    ) else {
        return out;
    };
    if check_out.code != 0 {
        return out;
    }

    let check_text = String::from_utf8_lossy(&check_out.stdout);
    let mut selected: Vec<(&str, &str, usize)> = Vec::new();
    let mut total_bytes = 0usize;
    for (path, line) in safe_paths.iter().zip(check_text.lines()) {
        let parts: Vec<&str> = line.split_ascii_whitespace().collect();
        if parts.len() == 3
            && parts[1] == "blob"
            && let Ok(sz) = parts[2].parse::<usize>()
            && sz <= MAX_SOURCE_BYTES
        {
            if total_bytes.saturating_add(sz) > max_total_bytes {
                break;
            }
            total_bytes += sz;
            selected.push((path, parts[0], sz));
        }
    }

    const BATCH_MAX_BYTES: usize = 16 * 1024 * 1024;
    let mut idx = 0;
    while idx < selected.len() {
        let mut chunk: Vec<(&str, &str)> = Vec::new();
        let mut chunk_bytes = 0usize;
        let mut batch_input = Vec::new();
        while idx < selected.len() {
            let (path, oid, sz) = selected[idx];
            if !chunk.is_empty() && chunk_bytes + sz + oid.len() + 64 > BATCH_MAX_BYTES {
                break;
            }
            chunk_bytes += sz + oid.len() + 64;
            batch_input.extend_from_slice(oid.as_bytes());
            batch_input.push(b'\n');
            chunk.push((path, oid));
            idx += 1;
        }
        if let Ok(batch_out) = infra_core::git::run(
            root,
            &infra_core::git::strings(&["cat-file", "--batch"]),
            Some(batch_input),
            Duration::from_secs(GIT_TIMEOUT_SECS),
            BATCH_MAX_BYTES + 4 * 1024 * 1024,
        ) && batch_out.code == 0
        {
            let _ = parse_batch_blob_payloads(&batch_out.stdout, &chunk, &mut out);
        }
    }
    out
}

/// Inverted reference/token index over scanned workspace files so callers beyond
/// `MAX_CALLER_SCAN` are ranked and discovered without linear-prefix truncation.
struct CallerScanIndex {
    postings: HashMap<String, Vec<usize>>,
    scanned_files: Vec<String>,
    priority_paths: Vec<String>,
    retained_sources: HashMap<String, String>,
    dep_cache: RefCell<HashMap<String, (String, Vec<DepRef>)>>,
}

impl CallerScanIndex {
    fn build(
        ctx: &TargetContext<'_>,
        index: Option<&Index>,
        changed: &BTreeMap<String, MergedChange>,
        live_focus: Option<&GraphFocus>,
        known_callers: &[String],
    ) -> Self {
        let mut seen = HashSet::new();
        let mut all_paths: Vec<String> = Vec::new();
        let mut priority_paths: Vec<String> = Vec::new();
        {
            let mut push_path = |p: &str, is_priority: bool| {
                if all_paths.len() < MAX_WALK_FILES
                    && is_scannable_code(p)
                    && seen.insert(p.to_owned())
                {
                    let owned = p.to_owned();
                    if is_priority {
                        priority_paths.push(owned.clone());
                    }
                    all_paths.push(owned);
                }
            };

            for (path, entry) in changed {
                if entry.status != "deleted" {
                    push_path(path, true);
                }
            }
            if let Some(lf) = live_focus {
                for raw in &lf.target_paths {
                    if let Some(p) = normalize_rel_opt(raw) {
                        push_path(&p, true);
                    }
                }
                for entity in &lf.created_entities {
                    if let Some(p) = normalize_rel_opt(&entity.path) {
                        push_path(&p, true);
                    }
                }
            }
            for caller in known_callers {
                push_path(caller, false);
            }
            if let Some(idx) = index {
                for path in idx.files() {
                    push_path(path, false);
                }
            }
            for path in ctx.existing_files() {
                push_path(path, false);
            }
        }

        let mut initial_seen: HashSet<String> = priority_paths.iter().cloned().collect();
        let mut initial_count = initial_seen.len();
        for caller in known_callers {
            if initial_count >= MAX_CALLER_SCAN {
                break;
            }
            if is_scannable_code(caller) && initial_seen.insert(caller.clone()) {
                initial_count += 1;
            }
        }

        let mut batched_rev_blobs = match ctx.revision.as_deref() {
            Some(rev) => {
                read_revision_blobs_batched(ctx.root, rev, &all_paths, MAX_INDEX_SCAN_BYTES)
            }
            None => HashMap::new(),
        };

        let mut postings: HashMap<String, Vec<usize>> = HashMap::new();
        let mut scanned_files: Vec<String> = Vec::with_capacity(all_paths.len());
        let mut retained_sources: HashMap<String, String> = HashMap::new();
        let mut dep_cache: HashMap<String, (String, Vec<DepRef>)> = HashMap::new();
        let mut scanned_bytes = 0usize;
        let mut retained_bytes = 0usize;
        let small_project = all_paths.len() <= MAX_CALLER_SCAN;

        for path in &all_paths {
            let src_opt = if ctx.revision.is_some() {
                batched_rev_blobs.remove(path).or_else(|| {
                    if initial_seen.contains(path) || small_project {
                        ctx.read_after(path).into_opt()
                    } else {
                        None
                    }
                })
            } else if scanned_bytes < MAX_INDEX_SCAN_BYTES || initial_seen.contains(path) {
                disk_read_text(ctx.root, path).into_opt()
            } else {
                None
            };
            let Some(src) = src_opt else {
                continue;
            };
            scanned_bytes = scanned_bytes.saturating_add(src.len());
            let file_idx = scanned_files.len();
            scanned_files.push(path.clone());
            for tok in extract_ident_tokens(&src) {
                postings.entry(tok).or_default().push(file_idx);
            }

            if initial_seen.contains(path) || (small_project && dep_cache.len() < MAX_CALLER_SCAN) {
                let deps = extract_file_deps(path, &src, &ctx.resolution_files);
                dep_cache.insert(path.clone(), (src, deps));
            } else if retained_bytes.saturating_add(src.len()) <= MAX_RETAINED_SOURCE_BYTES {
                retained_bytes += src.len();
                retained_sources.insert(path.clone(), src);
            }
        }

        Self {
            postings,
            scanned_files,
            priority_paths,
            retained_sources,
            dep_cache: RefCell::new(dep_cache),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn candidates_for(
        &self,
        rel_path: &str,
        previous_path: Option<&str>,
        target_symbols: &[String],
        declared_members: &[DeclaredMember],
        index: Option<&Index>,
        index_incoming: &HashMap<u32, Vec<(u32, String, bool)>>,
        certain: bool,
    ) -> Vec<String> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut out: Vec<String> = Vec::new();
        let push_unique = |path: &str, seen: &mut HashSet<String>, out: &mut Vec<String>| -> bool {
            if path != rel_path && seen.insert(path.to_owned()) {
                out.push(path.to_owned());
                true
            } else {
                false
            }
        };

        for p in &self.priority_paths {
            push_unique(p, &mut seen, &mut out);
        }
        if let Some(idx) = index {
            let mut idx_added = 0usize;
            for fid in [
                idx.file_id(rel_path),
                previous_path.and_then(|old| idx.file_id(old)),
            ]
            .into_iter()
            .flatten()
            {
                if let Some(rows) = index_incoming.get(&fid) {
                    for (from_fid, _, guessed) in rows {
                        if certain && *guessed {
                            continue;
                        }
                        if idx_added >= MAX_CALLER_SCAN {
                            break;
                        }
                        let caller_path = &idx.files()[*from_fid as usize];
                        if push_unique(caller_path, &mut seen, &mut out) {
                            idx_added += 1;
                        }
                    }
                }
            }
        }

        if self.scanned_files.len() <= MAX_CALLER_SCAN {
            for p in &self.scanned_files {
                push_unique(p, &mut seen, &mut out);
            }
            return out;
        }

        let mut query_tokens: BTreeSet<String> = BTreeSet::new();
        push_query_tokens_from_path(rel_path, &mut query_tokens);
        if let Some(prev) = previous_path {
            push_query_tokens_from_path(prev, &mut query_tokens);
        }
        for sym in target_symbols {
            for tok in extract_ident_tokens(sym) {
                query_tokens.insert(tok);
            }
        }
        for member in declared_members {
            for tok in extract_ident_tokens(&member.name) {
                query_tokens.insert(tok);
            }
            if let Some(parent) = &member.parent {
                for tok in extract_ident_tokens(parent) {
                    query_tokens.insert(tok);
                }
            }
        }

        let mut scores: HashMap<usize, usize> = HashMap::new();
        for tok in &query_tokens {
            if let Some(files) = self.postings.get(tok) {
                let idf_weight = 1000usize / files.len().max(1);
                let weight = idf_weight.max(1);
                for &file_idx in files {
                    *scores.entry(file_idx).or_insert(0) += weight;
                }
            }
        }

        let mut ranked: Vec<(usize, usize)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then(self.scanned_files[a.0].cmp(&self.scanned_files[b.0]))
        });
        let mut added = 0usize;
        for (file_idx, _) in ranked {
            if added >= MAX_CALLER_SCAN {
                break;
            }
            let path = &self.scanned_files[file_idx];
            if path != rel_path && seen.insert(path.clone()) {
                out.push(path.clone());
                added += 1;
            }
        }
        out
    }

    fn ensure_all_cached(&self, ctx: &TargetContext<'_>, candidates: &[String]) {
        let mut cache = self.dep_cache.borrow_mut();
        for path in candidates {
            if cache.contains_key(path) {
                continue;
            }
            let src_opt = self
                .retained_sources
                .get(path)
                .cloned()
                .or_else(|| ctx.read_after(path).into_opt());
            if let Some(src) = src_opt {
                let deps = extract_file_deps(path, &src, &ctx.resolution_files);
                cache.insert(path.clone(), (src, deps));
            }
        }
    }

    fn read_source_for(&self, ctx: &TargetContext<'_>, path: &str) -> Option<String> {
        if let Some((src, _)) = self.dep_cache.borrow().get(path) {
            return Some(src.clone());
        }
        if let Some(src) = self.retained_sources.get(path) {
            return Some(src.clone());
        }
        ctx.read_after(path).into_opt()
    }
}

#[allow(clippy::too_many_arguments)]
fn evaluate_callers_for_file(
    rel_path: &str,
    previous_path: Option<&str>,
    status: &str,
    entities: &[EntityChange],
    declared_members: &[DeclaredMember],
    target_symbols: &[String],
    ctx: &TargetContext<'_>,
    index: Option<&Index>,
    live_focus: Option<&GraphFocus>,
    resolved: Option<&Resolved>,
    certain: bool,
    index_incoming: &HashMap<u32, Vec<(u32, String, bool)>>,
    scan_index: &CallerScanIndex,
) -> Vec<CallerReport> {
    let mut callers: BTreeMap<String, CallerAccum> = BTreeMap::new();

    if let Some(idx) = index {
        for fid in [
            idx.file_id(rel_path),
            previous_path.and_then(|old| idx.file_id(old)),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(rows) = index_incoming.get(&fid) {
                for (from_fid, label, guessed) in rows {
                    if certain && *guessed {
                        continue;
                    }
                    let caller_path = &idx.files()[*from_fid as usize];
                    if caller_path == rel_path {
                        continue;
                    }
                    let acc = callers.entry(caller_path.clone()).or_default();
                    acc.from_index = true;
                    acc.links += 1;
                    if *guessed {
                        acc.guessed += 1;
                    }
                    if !label.is_empty() {
                        acc.symbols.insert(label.clone());
                    }
                }
            }
        }
    }

    let candidates = scan_index.candidates_for(
        rel_path,
        previous_path,
        target_symbols,
        declared_members,
        index,
        index_incoming,
        certain,
    );
    scan_index.ensure_all_cached(ctx, &candidates);
    {
        let dep_cache = scan_index.dep_cache.borrow();
        for caller_path in &candidates {
            if caller_path == rel_path {
                continue;
            }
            let Some((caller_src, caller_deps)) = dep_cache.get(caller_path) else {
                continue;
            };
            let matching_deps: Vec<&DepRef> = caller_deps
                .iter()
                .filter(|d| {
                    d.resolved
                        && (d.target == rel_path
                            || previous_path.is_some_and(|old| d.target == old))
                })
                .collect();
            let matched_refs = find_matching_references(
                caller_path,
                caller_src,
                rel_path,
                target_symbols,
                &ctx.resolution_files,
            );
            let has_live_edge = !matching_deps.is_empty() || !matched_refs.is_empty();

            if let Some(acc) = callers.get_mut(caller_path) {
                acc.source_after = Some(caller_src.clone());
                if has_live_edge {
                    for dep in matching_deps {
                        acc.symbols.extend(dep.symbols.iter().cloned());
                    }
                    acc.symbols.extend(matched_refs);
                } else if is_caller_modified_in_resolved(caller_path, resolved) {
                    // Caller was modified and dropped its dependency on this file.
                    let dropped =
                        ctx.read_before(caller_path)
                            .into_opt()
                            .is_some_and(|before_src| {
                                let before_deps = extract_file_deps(
                                    caller_path,
                                    &before_src,
                                    &ctx.resolution_files,
                                );
                                before_deps.iter().any(|d| {
                                    d.resolved
                                        && (d.target == rel_path
                                            || previous_path.is_some_and(|old| d.target == old))
                                }) || !find_matching_references(
                                    caller_path,
                                    &before_src,
                                    rel_path,
                                    target_symbols,
                                    &ctx.resolution_files,
                                )
                                .is_empty()
                            });
                    if dropped {
                        callers.remove(caller_path);
                    }
                }
            } else if has_live_edge {
                let mut syms = BTreeSet::new();
                let mut link_count = 0u64;
                for dep in &matching_deps {
                    syms.extend(dep.symbols.iter().cloned());
                    link_count += dep.symbols.len().max(1) as u64;
                }
                for r in &matched_refs {
                    syms.insert(r.clone());
                }
                link_count = link_count.max(matched_refs.len().max(1) as u64);
                callers.insert(
                    caller_path.clone(),
                    CallerAccum {
                        symbols: syms,
                        links: link_count,
                        guessed: 0,
                        from_index: false,
                        source_after: Some(caller_src.clone()),
                    },
                );
            }
        }
    }

    let changed_entity_names: Vec<String> = entities
        .iter()
        .filter(|e| e.kind != "file" && !e.name.is_empty())
        .map(|e| e.name.clone())
        .collect();

    let mut reports = Vec::with_capacity(callers.len());
    for (caller_path, mut acc) in callers {
        let caller_exists = ctx.exists_in_target(&caller_path);
        if acc.source_after.is_none()
            && caller_exists
            && let BlobRead::Ok(src) = ctx.read_after(&caller_path)
        {
            acc.source_after = Some(src);
        }

        if let Some(src) = &acc.source_after
            && !changed_entity_names.is_empty()
        {
            let matched_changed = find_matching_references(
                &caller_path,
                src,
                rel_path,
                &changed_entity_names,
                &ctx.resolution_files,
            );
            acc.symbols.extend(matched_changed);
        }

        let is_added_caller = detect_added_caller(
            &caller_path,
            &acc,
            rel_path,
            previous_path,
            status,
            target_symbols,
            ctx,
            resolved,
        );

        let mut read_ext_src = |p: &str| scan_index.read_source_for(ctx, p);
        let caller_extends = caller_extends_target_in(
            &caller_path,
            acc.source_after.as_deref(),
            rel_path,
            declared_members,
            &ctx.resolution_files,
            &mut read_ext_src,
        );
        let mut used_members = match acc.source_after.as_deref() {
            Some(src) => find_matching_member_references(
                &caller_path,
                src,
                rel_path,
                declared_members,
                &ctx.resolution_files,
            ),
            None => Vec::new(),
        };

        // Fallback for symbols discovered from Graphify index when not already captured by AST/syntax refs.
        let initial_syms: Vec<String> = acc.symbols.iter().cloned().collect();
        for sym in &initial_syms {
            if let Some(tm) = find_matching_declared_member(declared_members, sym, None, None) {
                let is_container = matches!(
                    tm.kind.as_str(),
                    "class" | "abstract_class" | "trait" | "interface" | "struct" | "enum"
                );
                if is_container && tm.change == "unchanged" && !used_members.is_empty() {
                    continue;
                }
                let already = used_members.iter().any(|existing| {
                    norm_member_name(&existing.name) == norm_member_name(&tm.name)
                        && kinds_compatible(&existing.kind, &tm.kind)
                        && existing.parent == tm.parent
                });
                if !already {
                    let access = match tm.kind.as_str() {
                        "method" | "function" | "fn" => "call",
                        "property" | "field" | "const" | "constant" => "read",
                        _ => "reference",
                    };
                    used_members.push(DepMemberRef {
                        name: tm.name.clone(),
                        kind: tm.kind.clone(),
                        parent: tm.parent.clone(),
                        access: access.to_owned(),
                        line: None,
                        lines: Vec::new(),
                        visibility: tm.visibility.clone(),
                        previous_visibility: tm.previous_visibility.clone(),
                        change: Some(tm.change.clone()),
                        modifiers: tm.modifiers.clone(),
                        access_risk: None,
                    });
                }
            }
        }

        let mut visibility_warnings: Vec<VisibilityWarning> = Vec::new();
        let mut push_warning = |w: VisibilityWarning| {
            let dup = visibility_warnings.iter().any(|existing| {
                norm_member_name(&existing.member) == norm_member_name(&w.member)
                    && existing.kind == w.kind
                    && existing.parent == w.parent
                    && existing.line == w.line
                    && existing.reason == w.reason
            });
            if !dup {
                visibility_warnings.push(w);
            }
        };

        let mut changed_syms = BTreeSet::new();
        let mut risk_syms = BTreeSet::new();
        let mut vis_narrow_risk_syms = BTreeSet::new();
        let mut mod_syms = BTreeSet::new();

        for mref in &mut used_members {
            acc.symbols.insert(mref.name.clone());
            let assess = enrich_member_ref_in(
                ctx.manifest_root(),
                Some(&ctx.resolution_files),
                mref,
                declared_members,
                &caller_path,
                rel_path,
                caller_extends,
            );
            if mref.change.as_deref().is_some_and(|c| c != "unchanged") {
                changed_syms.insert(mref.name.clone());
            }
            if let Some(w) = assess.warning {
                push_warning(w);
            }
            if assess.broken_by_change {
                risk_syms.insert(mref.name.clone());
                if mref.access_risk.as_deref() == Some("inaccessible")
                    && mref.previous_visibility.is_some()
                {
                    vis_narrow_risk_syms.insert(mref.name.clone());
                }
            } else if matches!(
                mref.change.as_deref(),
                Some("modified" | "visibility_changed")
            ) {
                mod_syms.insert(mref.name.clone());
            }
        }
        deduplicate_and_sort_member_refs(&mut used_members);

        let all_syms: Vec<String> = acc.symbols.iter().cloned().collect();

        for entity in entities {
            if entity.kind == "file" {
                continue;
            }
            let matched = all_syms
                .iter()
                .any(|sym| entity_matches_symbol(entity, sym));
            if !matched {
                continue;
            }
            changed_syms.insert(entity.name.clone());
            let covered_by_member_ref = used_members.iter().any(|m| {
                norm_member_name(&m.name) == norm_member_name(&entity.name)
                    && kinds_compatible(&m.kind, &entity.kind)
                    && (m.parent.as_deref()
                        == normalized_entity_parent(entity.parent.as_deref(), rel_path)
                        || m.parent.is_none()
                        || normalized_entity_parent(entity.parent.as_deref(), rel_path).is_none())
            });

            if entity.change == "deleted" || entity.signature_changed {
                risk_syms.insert(entity.name.clone());
            } else if entity.visibility_narrowed && !covered_by_member_ref {
                let synth_tm = DeclaredMember {
                    name: entity.name.clone(),
                    kind: entity.kind.clone(),
                    parent: normalized_entity_parent(entity.parent.as_deref(), rel_path)
                        .map(str::to_owned),
                    line: entity.line,
                    signature: None,
                    visibility: entity.visibility.clone(),
                    previous_visibility: entity.previous_visibility.clone(),
                    modifiers: entity.modifiers.clone(),
                    change: map_entity_change_status(entity),
                    signature_changed: entity.signature_changed,
                    visibility_narrowed: entity.visibility_narrowed,
                    used_by_caller: true,
                };
                let assess = assess_member_access_in(
                    ctx.manifest_root(),
                    Some(&ctx.resolution_files),
                    &synth_tm,
                    &caller_path,
                    rel_path,
                    caller_extends,
                    "reference",
                    entity.line,
                );
                if let Some(w) = assess.warning {
                    push_warning(w);
                }
                if assess.broken_by_change {
                    risk_syms.insert(entity.name.clone());
                    vis_narrow_risk_syms.insert(entity.name.clone());
                } else {
                    mod_syms.insert(entity.name.clone());
                }
            } else if entity.change == "modified" && !covered_by_member_ref {
                mod_syms.insert(entity.name.clone());
            }
        }

        let (impact, reason) = if !risk_syms.is_empty() {
            let list: Vec<String> = risk_syms.into_iter().collect();
            let reason = if !vis_narrow_risk_syms.is_empty() {
                format!(
                    "References inaccessible (visibility-narrowed), deleted, or signature-changed member(s): {}",
                    list.join(", ")
                )
            } else {
                format!(
                    "References deleted or signature-changed symbol(s): {}",
                    list.join(", ")
                )
            };
            ("broken_risk", reason)
        } else if !mod_syms.is_empty() || status == "deleted" {
            let reason = if status == "deleted" {
                "Target file was deleted".to_owned()
            } else {
                let list: Vec<String> = mod_syms.into_iter().collect();
                format!("References modified symbol(s): {}", list.join(", "))
            };
            ("impacted", reason)
        } else if is_added_caller {
            ("added_caller", "Added dependency on this file".to_owned())
        } else {
            (
                "unchanged",
                "Existing dependency unaffected by symbol changes".to_owned(),
            )
        };

        let opened_by_run = is_opened_by_run(&caller_path, live_focus, resolved);

        reports.push(CallerReport {
            path: caller_path,
            exists: caller_exists,
            impact,
            reason,
            symbols: all_syms,
            changed_symbols: changed_syms.into_iter().collect(),
            used_members,
            visibility_warnings,
            links: acc.links,
            guessed: acc.guessed,
            opened_by_run,
        });
    }

    reports.sort_by(|a, b| {
        impact_rank(a.impact)
            .cmp(&impact_rank(b.impact))
            .then(Reverse(a.links).cmp(&Reverse(b.links)))
            .then(a.path.cmp(&b.path))
    });
    reports
}

fn impact_rank(impact: &str) -> u8 {
    match impact {
        "broken_risk" => 0,
        "impacted" => 1,
        "added_caller" => 2,
        _ => 3,
    }
}

#[allow(clippy::too_many_arguments)]
fn detect_added_caller(
    caller_path: &str,
    acc: &CallerAccum,
    rel_path: &str,
    previous_path: Option<&str>,
    target_status: &str,
    target_symbols: &[String],
    ctx: &TargetContext<'_>,
    resolved: Option<&Resolved>,
) -> bool {
    let caller_changed_entry = resolved.and_then(|r| {
        r.files
            .iter()
            .find(|c| normalize_rel_opt(&c.path).as_deref() == Some(caller_path))
    });
    let caller_is_created = caller_changed_entry.is_some_and(|c| matches!(c.status, 'A' | 'C'));
    if caller_is_created {
        return true;
    }

    let should_check_baseline =
        !acc.from_index || caller_changed_entry.is_some() || target_status == "created";
    if !should_check_baseline {
        return false;
    }

    let lookup = caller_changed_entry
        .and_then(|c| c.from.as_deref())
        .and_then(normalize_rel_opt);
    let lookup_ref = lookup.as_deref().unwrap_or(caller_path);

    if let BlobRead::Ok(before_src) = ctx.read_before(lookup_ref) {
        let before_deps = extract_file_deps(caller_path, &before_src, &ctx.resolution_files);
        let had_dep = before_deps.iter().any(|d| {
            d.resolved && (d.target == rel_path || previous_path.is_some_and(|old| d.target == old))
        });
        let had_ref = !find_matching_references(
            caller_path,
            &before_src,
            rel_path,
            target_symbols,
            &ctx.resolution_files,
        )
        .is_empty();
        return !had_dep && !had_ref;
    }

    target_status == "created"
}

fn is_caller_modified_in_resolved(caller_path: &str, resolved: Option<&Resolved>) -> bool {
    resolved.is_some_and(|r| {
        r.files
            .iter()
            .any(|c| normalize_rel_opt(&c.path).as_deref() == Some(caller_path))
    })
}

fn is_opened_by_run(
    caller_path: &str,
    live_focus: Option<&GraphFocus>,
    resolved: Option<&Resolved>,
) -> bool {
    if let Some(lf) = live_focus
        && (lf
            .target_paths
            .iter()
            .any(|p| normalize_rel_opt(p).as_deref() == Some(caller_path))
            || lf
                .created_entities
                .iter()
                .any(|e| normalize_rel_opt(&e.path).as_deref() == Some(caller_path)))
    {
        return true;
    }
    resolved.is_some_and(|r| {
        r.kind == "run"
            && r.files
                .iter()
                .any(|f| normalize_rel_opt(&f.path).as_deref() == Some(caller_path))
    })
}

fn entity_matches_symbol(entity: &EntityChange, raw_sym: &str) -> bool {
    let clean_sym = raw_sym.trim().trim_end_matches("()");
    let clean_name = entity.name.trim().trim_end_matches("()");
    if clean_sym.is_empty() || clean_name.is_empty() {
        return false;
    }
    if clean_sym == clean_name {
        return true;
    }
    if let Some(parent) = entity.parent.as_deref().filter(|p| !p.is_empty())
        && (clean_sym == format!("{parent}::{clean_name}")
            || clean_sym == format!("{parent}.{clean_name}"))
    {
        return true;
    }
    clean_sym
        .rsplit(['.', ':'])
        .next()
        .is_some_and(|tail| tail == clean_name)
}

#[allow(clippy::too_many_arguments)]
fn compute_architecture_impact(
    rel_path: &str,
    status: &str,
    exists: bool,
    dep_diff: &DepDiff,
    used_by: &[CallerReport],
    ctx: &TargetContext<'_>,
    index: Option<&Index>,
    certain: bool,
    caller_cache: &HashMap<String, (String, Vec<DepRef>)>,
) -> (Vec<Value>, Vec<Value>, Vec<Value>, Vec<Value>) {
    let mut base_adj: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    if let Some(idx) = index {
        let files = idx.files();
        for (from, to, pair) in idx.pairs() {
            if *from != *to && pair.weight(certain) > 0 {
                base_adj
                    .entry(files[*from as usize].clone())
                    .or_default()
                    .insert(files[*to as usize].clone());
            }
        }
    } else {
        for (src_path, (_, deps)) in caller_cache {
            for dep in deps {
                if dep.resolved && dep.target != *src_path {
                    base_adj
                        .entry(src_path.clone())
                        .or_default()
                        .insert(dep.target.clone());
                }
            }
        }
    }

    // Ensure direct workspace targets/callers not yet in Index (e.g. newly created files)
    // contribute their live edges so cycles across new/unindexed files are caught.
    for dep in &dep_diff.added {
        if dep.resolved && !base_adj.contains_key(&dep.target) {
            if let Some((_, target_deps)) = caller_cache.get(&dep.target) {
                for td in target_deps {
                    if td.resolved && td.target != dep.target {
                        base_adj
                            .entry(dep.target.clone())
                            .or_default()
                            .insert(td.target.clone());
                    }
                }
            } else if let BlobRead::Ok(src) = ctx.read_after(&dep.target) {
                for td in extract_file_deps(&dep.target, &src, &ctx.resolution_files) {
                    if td.resolved && td.target != dep.target {
                        base_adj
                            .entry(dep.target.clone())
                            .or_default()
                            .insert(td.target);
                    }
                }
            }
        }
    }

    let mut before_adj = base_adj.clone();
    if status == "created" {
        before_adj.remove(rel_path);
        for targets in before_adj.values_mut() {
            targets.remove(rel_path);
        }
    } else {
        let entry = before_adj.entry(rel_path.to_owned()).or_default();
        for dep in &dep_diff.added {
            if dep.resolved {
                entry.remove(&dep.target);
            }
        }
        for dep in dep_diff.removed.iter().chain(dep_diff.unchanged.iter()) {
            if dep.resolved && dep.target != rel_path {
                entry.insert(dep.target.clone());
            }
        }
        for caller in used_by {
            if caller.impact == "added_caller"
                && let Some(set) = before_adj.get_mut(&caller.path)
            {
                set.remove(rel_path);
            }
        }
    }

    let mut after_adj = base_adj;
    if !exists || status == "deleted" {
        after_adj.remove(rel_path);
        for targets in after_adj.values_mut() {
            targets.remove(rel_path);
        }
    } else {
        let entry = after_adj.entry(rel_path.to_owned()).or_default();
        for dep in &dep_diff.removed {
            if dep.resolved {
                entry.remove(&dep.target);
            }
        }
        for dep in dep_diff.added.iter().chain(dep_diff.unchanged.iter()) {
            if dep.resolved && dep.target != rel_path {
                entry.insert(dep.target.clone());
            }
        }
        for caller in used_by {
            if caller.exists && caller.path != rel_path {
                after_adj
                    .entry(caller.path.clone())
                    .or_default()
                    .insert(rel_path.to_owned());
            }
        }
    }

    let before_cycles = find_cycles_through(&before_adj, rel_path, MAX_CYCLE_DEPTH);
    let after_cycles = find_cycles_through(&after_adj, rel_path, MAX_CYCLE_DEPTH);

    let new_cycles = after_cycles
        .difference(&before_cycles)
        .map(|nodes| json!({ "steps": nodes.len().saturating_sub(1), "nodes": nodes }))
        .collect::<Vec<_>>();
    let resolved_cycles = before_cycles
        .difference(&after_cycles)
        .map(|nodes| json!({ "steps": nodes.len().saturating_sub(1), "nodes": nodes }))
        .collect::<Vec<_>>();

    let (new_violations, resolved_violations) = match Rules::read(ctx.root) {
        Ok(Some(rules)) => {
            let before_v = collect_file_violations(&before_adj, rel_path, &rules);
            let after_v = collect_file_violations(&after_adj, rel_path, &rules);
            let new_v = after_v
                .difference(&before_v)
                .map(violation_json)
                .collect::<Vec<_>>();
            let resolved_v = before_v
                .difference(&after_v)
                .map(violation_json)
                .collect::<Vec<_>>();
            (new_v, resolved_v)
        }
        _ => (Vec::new(), Vec::new()),
    };

    (
        new_cycles,
        resolved_cycles,
        new_violations,
        resolved_violations,
    )
}

fn find_cycles_through(
    adj: &BTreeMap<String, BTreeSet<String>>,
    origin: &str,
    max_depth: usize,
) -> BTreeSet<Vec<String>> {
    let mut found = BTreeSet::new();
    if max_depth < 2 || !adj.contains_key(origin) {
        return found;
    }

    let mut rev: HashMap<&str, Vec<&str>> = HashMap::new();
    for (from, targets) in adj {
        for to in targets {
            rev.entry(to.as_str()).or_default().push(from.as_str());
        }
    }

    let mut dist_to_origin: HashMap<&str, usize> = HashMap::new();
    let mut queue = VecDeque::new();
    dist_to_origin.insert(origin, 0);
    queue.push_back(origin);
    while let Some(node) = queue.pop_front() {
        let d = dist_to_origin[node];
        if d + 1 >= max_depth {
            continue;
        }
        if let Some(preds) = rev.get(node) {
            for &pred in preds {
                if !dist_to_origin.contains_key(pred) {
                    dist_to_origin.insert(pred, d + 1);
                    queue.push_back(pred);
                }
            }
        }
    }

    let mut visits = 0usize;
    for target_len in 2..=max_depth {
        if found.len() >= MAX_CYCLES_REPORTED || visits >= MAX_CYCLE_VISITS {
            break;
        }
        let mut path = vec![origin];
        let mut in_path = HashSet::from([origin]);
        dfs_cycles(
            adj,
            &dist_to_origin,
            origin,
            origin,
            target_len,
            &mut path,
            &mut in_path,
            &mut visits,
            &mut found,
        );
    }
    found
}

#[allow(clippy::too_many_arguments)]
fn dfs_cycles<'a>(
    adj: &'a BTreeMap<String, BTreeSet<String>>,
    dist_to_origin: &HashMap<&'a str, usize>,
    origin: &'a str,
    current: &'a str,
    target_len: usize,
    path: &mut Vec<&'a str>,
    in_path: &mut HashSet<&'a str>,
    visits: &mut usize,
    found: &mut BTreeSet<Vec<String>>,
) {
    if found.len() >= MAX_CYCLES_REPORTED || *visits >= MAX_CYCLE_VISITS {
        return;
    }
    *visits += 1;
    let depth = path.len();
    let Some(neighbors) = adj.get(current) else {
        return;
    };
    for next in neighbors {
        let next_s = next.as_str();
        if next_s == origin {
            if depth == target_len {
                let mut cycle: Vec<String> = path.iter().map(|s| (*s).to_owned()).collect();
                cycle.push(origin.to_owned());
                found.insert(cycle);
            }
            continue;
        }
        if depth >= target_len || in_path.contains(next_s) {
            continue;
        }
        let remaining = target_len - depth;
        if dist_to_origin
            .get(next_s)
            .is_none_or(|&dist| dist > remaining)
        {
            continue;
        }
        in_path.insert(next_s);
        path.push(next_s);
        dfs_cycles(
            adj,
            dist_to_origin,
            origin,
            next_s,
            target_len,
            path,
            in_path,
            visits,
            found,
        );
        path.pop();
        in_path.remove(next_s);
        if found.len() >= MAX_CYCLES_REPORTED || *visits >= MAX_CYCLE_VISITS {
            return;
        }
    }
}

fn collect_file_violations(
    adj: &BTreeMap<String, BTreeSet<String>>,
    rel_path: &str,
    rules: &Rules,
) -> BTreeSet<Violation> {
    let mut violations = BTreeSet::new();
    for (from, targets) in adj {
        for to in targets {
            if from != rel_path && to != rel_path {
                continue;
            }
            if let Some((from_id, from_name, to_id, to_name)) = check_layer_rule(rules, from, to) {
                violations.insert(Violation {
                    from: from.clone(),
                    to: to.clone(),
                    from_layer_id: from_id,
                    from_layer: from_name,
                    to_layer_id: to_id,
                    to_layer: to_name,
                });
            }
        }
    }
    violations
}

fn violation_json(v: &Violation) -> Value {
    json!({
        "from": v.from,
        "to": v.to,
        "from_layer_id": v.from_layer_id,
        "from_layer": v.from_layer,
        "to_layer_id": v.to_layer_id,
        "to_layer": v.to_layer,
    })
}

fn normalize_rel_path(raw: &str) -> Result<String> {
    let trimmed = raw.trim().replace('\\', "/");
    if trimmed.is_empty()
        || trimmed.len() > 1024
        || trimmed.chars().any(char::is_control)
        || trimmed.starts_with('/')
    {
        return Err(HarnessError::invalid(
            "Provide a valid project-relative file path.",
        ));
    }
    let mut out = Vec::new();
    for comp in Path::new(&trimmed).components() {
        match comp {
            Component::Normal(seg) => {
                let s = seg
                    .to_str()
                    .ok_or_else(|| HarnessError::invalid("Provide a valid UTF-8 file path."))?;
                if !s.is_empty() {
                    out.push(s);
                }
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(HarnessError::invalid(
                    "File path cannot escape the project directory.",
                ));
            }
        }
    }
    if out.is_empty() {
        return Err(HarnessError::invalid(
            "Provide a valid project-relative file path.",
        ));
    }
    Ok(out.join("/"))
}

fn normalize_rel_opt(raw: &str) -> Option<String> {
    normalize_rel_path(raw).ok()
}

fn is_within_root(root: &Path, rel: &str) -> bool {
    let full = root.join(rel);
    if !std::fs::metadata(&full).is_ok_and(|m| m.is_file()) {
        return false;
    }
    match (std::fs::canonicalize(root), std::fs::canonicalize(&full)) {
        (Ok(canon_root), Ok(canon_file)) => canon_file.starts_with(canon_root),
        _ => false,
    }
}

fn is_git_repo(root: &Path) -> bool {
    let args = ["rev-parse".to_owned(), "--is-inside-work-tree".to_owned()];
    projects::git(root, &args, 10)
        .is_ok_and(|out| out.code == 0 && String::from_utf8_lossy(&out.stdout).trim() == "true")
}

fn git_commit_exists(root: &Path, rev: &str) -> bool {
    let args = [
        "rev-parse".to_owned(),
        "--verify".to_owned(),
        "--quiet".to_owned(),
        format!("{rev}^{{commit}}"),
    ];
    projects::git(root, &args, 10).is_ok_and(|out| out.code == 0)
}

fn git_blob_exists(root: &Path, rev: &str, rel_path: &str) -> bool {
    let args = [
        "cat-file".to_owned(),
        "-e".to_owned(),
        format!("{rev}:./{rel_path}"),
    ];
    projects::git(root, &args, 10).is_ok_and(|out| out.code == 0)
}

fn git_read_text(root: &Path, rev: &str, rel_path: &str) -> BlobRead {
    let args = ["show".to_owned(), format!("{rev}:./{rel_path}")];
    let Ok(out) = projects::git(root, &args, GIT_TIMEOUT_SECS) else {
        return BlobRead::Missing;
    };
    if out.code != 0 {
        return BlobRead::Missing;
    }
    bytes_to_blob(out.stdout, "Baseline file")
}

fn disk_read_text(root: &Path, rel_path: &str) -> BlobRead {
    if !is_within_root(root, rel_path) {
        return BlobRead::Missing;
    }
    let full = root.join(rel_path);
    let Ok(meta) = std::fs::metadata(&full) else {
        return BlobRead::Missing;
    };
    if meta.len() > MAX_SOURCE_BYTES as u64 {
        return BlobRead::Unreadable("File exceeds size limit.");
    }
    let Ok(bytes) = std::fs::read(&full) else {
        return BlobRead::Missing;
    };
    bytes_to_blob(bytes, "File")
}

fn bytes_to_blob(bytes: Vec<u8>, label: &str) -> BlobRead {
    if bytes.len() > MAX_SOURCE_BYTES {
        return BlobRead::Unreadable(if label == "Baseline file" {
            "Baseline file exceeds size limit."
        } else {
            "File exceeds size limit."
        });
    }
    if bytes.contains(&0) {
        return BlobRead::Unreadable(if label == "Baseline file" {
            "Baseline file is binary."
        } else {
            "File is binary."
        });
    }
    match String::from_utf8(bytes) {
        Ok(text) => BlobRead::Ok(text),
        Err(err) => BlobRead::Ok(String::from_utf8_lossy(err.as_bytes()).into_owned()),
    }
}

impl BlobRead {
    fn into_opt(self) -> Option<String> {
        match self {
            BlobRead::Ok(s) => Some(s),
            _ => None,
        }
    }
}

fn git_tree_files(root: &Path, rev: &str) -> Option<BTreeSet<String>> {
    let args = [
        "ls-tree".to_owned(),
        "-r".to_owned(),
        "--name-only".to_owned(),
        "-z".to_owned(),
        rev.to_owned(),
        "--".to_owned(),
        ".".to_owned(),
    ];
    let out = projects::git(root, &args, GIT_TIMEOUT_SECS).ok()?;
    if out.code != 0 {
        return None;
    }
    let mut set = BTreeSet::new();
    for chunk in out.stdout.split(|b| *b == 0) {
        if chunk.is_empty() {
            continue;
        }
        if let Some(norm) = normalize_rel_opt(&String::from_utf8_lossy(chunk)) {
            set.insert(norm);
            if set.len() >= MAX_WALK_FILES * 4 {
                break;
            }
        }
    }
    Some(set)
}

fn collect_workspace_files(root: &Path, is_git: bool) -> BTreeSet<String> {
    if is_git {
        let ls_args = [
            "ls-files".to_owned(),
            "--cached".to_owned(),
            "--others".to_owned(),
            "--exclude-standard".to_owned(),
            "-z".to_owned(),
        ];
        if let Ok(out) = projects::git(root, &ls_args, GIT_TIMEOUT_SECS)
            && out.code == 0
        {
            let mut deleted = HashSet::new();
            let del_args = [
                "ls-files".to_owned(),
                "--deleted".to_owned(),
                "-z".to_owned(),
            ];
            if let Ok(del_out) = projects::git(root, &del_args, 10)
                && del_out.code == 0
            {
                for chunk in del_out.stdout.split(|b| *b == 0) {
                    if !chunk.is_empty()
                        && let Some(norm) = normalize_rel_opt(&String::from_utf8_lossy(chunk))
                    {
                        deleted.insert(norm);
                    }
                }
            }
            let mut set = BTreeSet::new();
            for chunk in out.stdout.split(|b| *b == 0) {
                if chunk.is_empty() {
                    continue;
                }
                if let Some(norm) = normalize_rel_opt(&String::from_utf8_lossy(chunk))
                    && !deleted.contains(&norm)
                {
                    set.insert(norm);
                    if set.len() >= MAX_WALK_FILES * 4 {
                        break;
                    }
                }
            }
            return set;
        }
    }

    let mut set = BTreeSet::new();
    if root.is_dir() {
        walk_dir_bounded(root, root, 0, &mut set);
    }
    set
}

fn walk_dir_bounded(root: &Path, dir: &Path, depth: usize, out: &mut BTreeSet<String>) {
    if depth > MAX_WALK_DEPTH || out.len() >= MAX_WALK_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths = Vec::new();
    for entry in entries.flatten() {
        paths.push(entry.path());
    }
    paths.sort();

    for path in paths {
        if out.len() >= MAX_WALK_FILES {
            return;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            if SKIP_DIRS.contains(&name) || name.starts_with('.') {
                continue;
            }
            if let Ok(rel) = path.strip_prefix(root) {
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                if rel_str == "memory-bank/local" {
                    continue;
                }
            }
            walk_dir_bounded(root, &path, depth + 1, out);
        } else if meta.is_file()
            && let Ok(rel) = path.strip_prefix(root)
            && let Some(norm) = normalize_rel_opt(&rel.to_string_lossy())
        {
            out.insert(norm);
        }
    }
}

fn is_scannable_code(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| CODE_EXTENSIONS.iter().any(|k| ext.eq_ignore_ascii_case(k)))
}

fn file_stem_name(path: &str) -> Option<String> {
    let base = path.rsplit('/').next().unwrap_or(path);
    let stem = base.split('.').next().unwrap_or("");
    (!stem.is_empty()).then(|| stem.to_owned())
}

fn contains_identifier(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() || needle.len() < 2 {
        return false;
    }
    let bytes = haystack.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        let abs = start + pos;
        let left_ok = abs == 0 || !is_ident(bytes[abs - 1]);
        let right = abs + needle.len();
        let right_ok = right >= bytes.len() || !is_ident(bytes[right]);
        if left_ok && right_ok {
            return true;
        }
        start = abs + needle.len();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_changes::{Change, Side, Target};
    use crate::graph_live::CreatedEntity;
    use std::path::PathBuf;
    use std::process::Command;

    struct TempRepo {
        _dir: tempfile::TempDir,
        root: PathBuf,
    }

    impl TempRepo {
        fn new_git() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().to_path_buf();
            let repo = Self { _dir: dir, root };
            repo.git(&["init", "--quiet", "--initial-branch=main"]);
            repo
        }

        fn new_plain() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().to_path_buf();
            Self { _dir: dir, root }
        }

        fn git(&self, args: &[&str]) -> String {
            let out = Command::new("git")
                .args([
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "core.hooksPath=/dev/null",
                ])
                .args(args)
                .current_dir(&self.root)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "tester")
                .env("GIT_AUTHOR_EMAIL", "tester@example.com")
                .env("GIT_COMMITTER_NAME", "tester")
                .env("GIT_COMMITTER_EMAIL", "tester@example.com")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_owned()
        }

        fn write(&self, rel: &str, content: &str) {
            let full = self.root.join(rel);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(full, content).unwrap();
        }

        fn remove(&self, rel: &str) {
            std::fs::remove_file(self.root.join(rel)).unwrap();
        }

        fn commit(&self, msg: &str) -> String {
            self.git(&["add", "-A"]);
            self.git(&["commit", "--quiet", "-m", msg]);
            self.git(&["rev-parse", "HEAD"])
        }
    }

    fn resolved_work(head: &str, files: Vec<Change>) -> Resolved {
        Resolved {
            target: Target::Work,
            kind: "work",
            name: "main".to_owned(),
            title: None,
            pull: None,
            head: Some(Side {
                name: "main".to_owned(),
                commit: head.to_owned(),
            }),
            base: Some(Side {
                name: "main".to_owned(),
                commit: head.to_owned(),
            }),
            revision: None,
            prefix: String::new(),
            files,
            notes: Vec::new(),
        }
    }

    #[test]
    fn git_repo_detects_added_modified_deleted_deps_entities_and_caller_risk() {
        let repo = TempRepo::new_git();
        repo.write(
            "src/DepOld.php",
            "<?php\nnamespace App;\nclass DepOld {\n    public function ping(): void {}\n}\n",
        );
        repo.write(
            "src/DepKept.php",
            "<?php\nnamespace App;\nclass DepKept {\n    public function keep(): void {}\n}\n",
        );
        repo.write(
            "src/Service.php",
            "<?php\nnamespace App;\nuse App\\DepOld;\nuse App\\DepKept;\nclass Service {\n    public function sign(string $payload): string {\n        $a = new DepOld();\n        $b = new DepKept();\n        return $payload;\n    }\n}\n",
        );
        repo.write(
            "src/Controller.php",
            "<?php\nnamespace App;\nuse App\\Service;\nclass Controller {\n    public function handle(Service $svc): string {\n        return $svc->sign('ok');\n    }\n}\n",
        );
        repo.write("tests/ServiceTest.php", "<?php\nclass ServiceTest {}\n");
        let head = repo.commit("initial");

        // Modify Service: remove DepOld, keep DepKept, add DepNew, change sign() signature
        repo.write(
            "src/DepNew.php",
            "<?php\nnamespace App;\nclass DepNew {\n    public function fresh(): string { return 'x'; }\n}\n",
        );
        repo.write(
            "src/Service.php",
            "<?php\nnamespace App;\nuse App\\DepKept;\nuse App\\DepNew;\nclass Service {\n    public function sign(string $payload, string $algo): string {\n        $b = new DepKept();\n        $n = new DepNew();\n        return $payload . $algo;\n    }\n}\n",
        );
        repo.write(
            "src/Controller.php",
            "<?php\nnamespace App;\nuse App\\Service;\nuse App\\DepNew;\nclass Controller {\n    public function handle(Service $svc): string {\n        $d = new DepNew();\n        return $svc->sign('ok') . $d->fresh();\n    }\n}\n",
        );
        repo.remove("src/DepOld.php");
        repo.write(
            "tests/ServiceTest.php",
            "<?php\nclass ServiceTest { public function testOk(): void {} }\n",
        );

        let index = Index::read(
            json!({
                "nodes": [
                    {"id": "dep_old", "label": "DepOld", "source_file": "src/DepOld.php"},
                    {"id": "dep_kept", "label": "DepKept", "source_file": "src/DepKept.php"},
                    {"id": "svc", "label": "Service", "source_file": "src/Service.php"},
                    {"id": "svc_sign", "label": "sign", "source_file": "src/Service.php", "source_location": "L6"},
                    {"id": "ctrl", "label": "Controller", "source_file": "src/Controller.php"},
                    {"id": "ctrl_handle", "label": "handle", "source_file": "src/Controller.php", "source_location": "L5"}
                ],
                "links": [
                    {"source": "svc_sign", "target": "dep_old", "relation": "calls", "confidence": "EXTRACTED", "source_file": "src/Service.php"},
                    {"source": "svc_sign", "target": "dep_kept", "relation": "calls", "confidence": "EXTRACTED", "source_file": "src/Service.php"},
                    {"source": "ctrl_handle", "target": "svc_sign", "relation": "calls", "confidence": "EXTRACTED", "source_file": "src/Controller.php"}
                ]
            })
            .to_string()
            .as_bytes(),
            &repo.root,
        )
        .unwrap();

        let resolved = resolved_work(
            &head,
            vec![
                Change {
                    path: "src/Service.php".into(),
                    status: 'M',
                    from: None,
                },
                Change {
                    path: "src/Controller.php".into(),
                    status: 'M',
                    from: None,
                },
                Change {
                    path: "src/DepNew.php".into(),
                    status: 'A',
                    from: None,
                },
                Change {
                    path: "src/DepOld.php".into(),
                    status: 'D',
                    from: None,
                },
                Change {
                    path: "tests/ServiceTest.php".into(),
                    status: 'M',
                    from: None,
                },
            ],
        );

        let catalog_no_tests = changed_catalog(
            &repo.root,
            Some(&index),
            Some(&resolved),
            None,
            &[],
            "work",
            true,
        );
        assert_eq!(catalog_no_tests["target"], "work");
        assert_eq!(catalog_no_tests["total_files"], 4);
        assert_eq!(catalog_no_tests["existing_files"], 3);
        assert_eq!(catalog_no_tests["created_files"], 1);
        assert_eq!(catalog_no_tests["modified_files"], 2);
        assert_eq!(catalog_no_tests["deleted_files"], 1);
        assert!(catalog_no_tests["modified_entities"].as_u64().unwrap() >= 1);

        let svc_impact = file_impact(
            &repo.root,
            Some(&index),
            Some(&resolved),
            None,
            "src/Service.php",
            false,
        )
        .unwrap();
        assert_eq!(svc_impact["status"], "modified");
        assert_eq!(svc_impact["exists"], true);
        assert_eq!(svc_impact["in_graph"], true);
        assert_eq!(svc_impact["baseline"]["available"], true);

        let added_targets: Vec<&str> = svc_impact["uses"]["added"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|d| d["target"].as_str())
            .collect();
        let removed_targets: Vec<&str> = svc_impact["uses"]["removed"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|d| d["target"].as_str())
            .collect();
        let unchanged_targets: Vec<&str> = svc_impact["uses"]["unchanged"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|d| d["target"].as_str())
            .collect();
        assert!(added_targets.contains(&"src/DepNew.php"), "{svc_impact}");
        assert!(removed_targets.contains(&"src/DepOld.php"), "{svc_impact}");
        assert!(
            unchanged_targets.contains(&"src/DepKept.php"),
            "{svc_impact}"
        );

        let used_by = svc_impact["used_by"].as_array().unwrap();
        let ctrl = used_by
            .iter()
            .find(|c| c["path"] == "src/Controller.php")
            .expect("Controller caller");
        assert_eq!(ctrl["impact"], "broken_risk");
        assert!(
            ctrl["changed_symbols"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s == "sign")
        );
        assert_eq!(svc_impact["summary"]["callers_at_risk"], 1);

        // Newly created file not in Graphify index should still show added_caller!
        let new_dep_impact = file_impact(
            &repo.root,
            Some(&index),
            Some(&resolved),
            None,
            "src/DepNew.php",
            false,
        )
        .unwrap();
        assert_eq!(new_dep_impact["status"], "created");
        assert_eq!(new_dep_impact["in_graph"], false);
        assert_eq!(new_dep_impact["exists"], true);
        let new_callers = new_dep_impact["used_by"].as_array().unwrap();
        let ctrl_new = new_callers
            .iter()
            .find(|c| c["path"] == "src/Controller.php")
            .expect("Controller added caller");
        assert_eq!(ctrl_new["impact"], "added_caller");

        // Deleted file should report exists=false and affected callers/removed uses
        let old_dep_impact = file_impact(
            &repo.root,
            Some(&index),
            Some(&resolved),
            None,
            "src/DepOld.php",
            false,
        )
        .unwrap();
        assert_eq!(old_dep_impact["status"], "deleted");
        assert_eq!(old_dep_impact["exists"], false);
        assert_eq!(old_dep_impact["summary"]["uses_total"], 0);
    }

    #[test]
    fn detects_new_and_resolved_cycles_and_layer_violations() {
        let repo = TempRepo::new_git();
        repo.write(
            ".harness-architecture.json",
            &json!({
                "version": 1,
                "layers": [
                    {"id": "app", "name": "Application", "paths": ["src/app"], "allows": ["domain"]},
                    {"id": "domain", "name": "Domain", "paths": ["src/domain"], "allows": []}
                ]
            })
            .to_string(),
        );
        repo.write(
            "src/app/Handler.php",
            "<?php\nnamespace App\\App;\nuse App\\Domain\\Core;\nclass Handler {\n    public function run(): void { new Core(); }\n}\n",
        );
        repo.write(
            "src/domain/Core.php",
            "<?php\nnamespace App\\Domain;\nclass Core {\n    public function calc(): int { return 1; }\n}\n",
        );
        let head = repo.commit("base");

        // Edit Core.php to depend back on Handler.php -> creates cycle AND layer violation!
        repo.write(
            "src/domain/Core.php",
            "<?php\nnamespace App\\Domain;\nuse App\\App\\Handler;\nclass Core {\n    public function calc(): int { new Handler(); return 1; }\n}\n",
        );

        let index = Index::read(
            json!({
                "nodes": [
                    {"id": "h", "label": "Handler", "source_file": "src/app/Handler.php"},
                    {"id": "c", "label": "Core", "source_file": "src/domain/Core.php"}
                ],
                "links": [
                    {"source": "h", "target": "c", "relation": "calls", "confidence": "EXTRACTED", "source_file": "src/app/Handler.php"}
                ]
            })
            .to_string()
            .as_bytes(),
            &repo.root,
        )
        .unwrap();

        let resolved = resolved_work(
            &head,
            vec![Change {
                path: "src/domain/Core.php".into(),
                status: 'M',
                from: None,
            }],
        );

        let impact = file_impact(
            &repo.root,
            Some(&index),
            Some(&resolved),
            None,
            "src/domain/Core.php",
            false,
        )
        .unwrap();
        assert_eq!(impact["summary"]["new_cycles"], 1, "{impact}");
        assert_eq!(impact["summary"]["resolved_cycles"], 0, "{impact}");
        assert_eq!(impact["summary"]["new_violations"], 1, "{impact}");
        let cycle = &impact["architecture_impact"]["new_cycles"][0];
        assert_eq!(cycle["steps"], 2);
        assert_eq!(
            cycle["nodes"],
            json!([
                "src/domain/Core.php",
                "src/app/Handler.php",
                "src/domain/Core.php"
            ])
        );
        let violation = &impact["architecture_impact"]["new_violations"][0];
        assert_eq!(violation["from"], "src/domain/Core.php");
        assert_eq!(violation["to"], "src/app/Handler.php");
        assert_eq!(violation["from_layer"], "Domain");
        assert_eq!(violation["to_layer"], "Application");
    }

    #[test]
    fn non_git_workspace_falls_back_gracefully_and_rejects_traversal() {
        let plain = TempRepo::new_plain();
        plain.write(
            "src/lib.rs",
            "mod helper;\npub fn entry() { helper::work(); }\n",
        );
        plain.write("src/helper.rs", "pub fn work() {}\n");

        let focus = GraphFocus {
            phase: "running".to_owned(),
            summary: "Editing helper".to_owned(),
            target_folders: vec!["src".to_owned()],
            target_paths: vec!["src/helper.rs".to_owned(), "src/nonexistent.rs".to_owned()],
            target_nodes: Vec::new(),
            created_entities: vec![CreatedEntity {
                id: "live:file:src/helper.rs".to_owned(),
                kind: "file".to_owned(),
                name: "helper.rs".to_owned(),
                parent: Some("src".to_owned()),
                path: "src/helper.rs".to_owned(),
                folder: "src".to_owned(),
                line: Some(1),
                change: Some("created".to_owned()),
                visibility: None,
                modifiers: Vec::new(),
            }],
        };

        let catalog = changed_catalog(
            &plain.root,
            None,
            None,
            Some(&focus),
            &[("src/lib.rs".to_owned(), 'M')],
            "run:s1",
            false,
        );
        assert_eq!(catalog["target"], "run:s1");
        assert_eq!(catalog["total_files"], 2);
        let paths: Vec<&str> = catalog["files"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|f| f["path"].as_str())
            .collect();
        assert!(paths.contains(&"src/helper.rs"));
        assert!(paths.contains(&"src/lib.rs"));
        assert!(!paths.contains(&"src/nonexistent.rs"));

        let helper_impact = file_impact(
            &plain.root,
            None,
            None,
            Some(&focus),
            "src/helper.rs",
            false,
        )
        .unwrap();
        assert_eq!(helper_impact["exists"], true);
        assert_eq!(helper_impact["status"], "created");
        assert!(
            helper_impact["used_by"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["path"] == "src/lib.rs" && c["opened_by_run"] == false)
        );

        let unchanged_impact =
            file_impact(&plain.root, None, None, None, "src/lib.rs", false).unwrap();
        assert_eq!(unchanged_impact["status"], "unchanged");
        assert_eq!(unchanged_impact["baseline"]["available"], false);
        assert_eq!(unchanged_impact["summary"]["uses_added"], 0);
        assert_eq!(unchanged_impact["summary"]["uses_unchanged"], 2);
        let unchanged = unchanged_impact["uses"]["unchanged"].as_array().unwrap();
        assert!(unchanged.iter().all(|u| u["target"] == "src/helper.rs"));
        assert!(unchanged.iter().any(|u| u["kind"] == "imports"));
        assert!(unchanged.iter().any(|u| u["kind"] == "calls"));

        assert!(file_impact(&plain.root, None, None, None, "../secret.rs", false).is_err());
        assert!(file_impact(&plain.root, None, None, None, "", false).is_err());
    }

    #[test]
    fn file_impact_tracks_properties_consts_visibility_narrowing_and_widening() {
        let repo = TempRepo::new_git();
        repo.write(
            "src/Config.php",
            "<?php\nnamespace App;\nclass Config {\n    public const TTL_MIN = 5;\n    public string $region = 'us';\n    private function legacySecret(): string { return 's'; }\n    public function format(): string { return 'ok'; }\n}\n",
        );
        repo.write(
            "src/Consumer.php",
            "<?php\nnamespace App;\nuse App\\Config;\nclass Consumer {\n    public function run(Config $cfg): string {\n        return $cfg->region . ':' . Config::TTL_MIN;\n    }\n}\n",
        );
        repo.write(
            "src/ChildConfig.php",
            "<?php\nnamespace App;\nuse App\\Config;\nclass ChildConfig extends Config {\n    public function check(): string {\n        return $this->region;\n    }\n}\n",
        );
        repo.write(
            "src/WidenedCaller.php",
            "<?php\nnamespace App;\nuse App\\Config;\nclass WidenedCaller {\n    public function callSecret(Config $cfg): string {\n        return $cfg->legacySecret();\n    }\n}\n",
        );
        let head = repo.commit("initial");

        // Narrow TTL_MIN (public -> private) and $region (public -> protected), widen legacySecret (private -> public).
        repo.write(
            "src/Config.php",
            "<?php\nnamespace App;\nclass Config {\n    private const TTL_MIN = 5;\n    protected string $region = 'us';\n    public function legacySecret(): string { return 's'; }\n    public function format(): string { return 'ok'; }\n}\n",
        );

        let resolved = resolved_work(
            &head,
            vec![Change {
                path: "src/Config.php".into(),
                status: 'M',
                from: None,
            }],
        );

        let cfg_impact = file_impact(
            &repo.root,
            None,
            Some(&resolved),
            None,
            "src/Config.php",
            false,
        )
        .unwrap();

        let declared = cfg_impact["declared_members"].as_array().unwrap();
        let ttl = declared
            .iter()
            .find(|m| m["name"] == "TTL_MIN" && m["kind"] == "const")
            .expect("TTL_MIN declared member");
        assert_eq!(ttl["visibility"], "private");
        assert_eq!(ttl["previous_visibility"], "public");
        assert_eq!(ttl["visibility_narrowed"], true);
        assert_eq!(ttl["change"], "visibility_changed");

        let region = declared
            .iter()
            .find(|m| m["name"] == "$region" && m["kind"] == "property")
            .expect("$region declared member");
        assert_eq!(region["visibility"], "protected");
        assert_eq!(region["previous_visibility"], "public");
        assert_eq!(region["visibility_narrowed"], true);

        let legacy = declared
            .iter()
            .find(|m| m["name"] == "legacySecret" && m["kind"] == "method")
            .expect("legacySecret declared member");
        assert_eq!(legacy["visibility"], "public");
        assert_eq!(legacy["previous_visibility"], "private");
        assert_ne!(legacy["visibility_narrowed"], true);

        let used_by = cfg_impact["used_by"].as_array().unwrap();

        // Non-subclass Consumer accesses private TTL_MIN and protected $region => broken_risk + warnings
        let consumer = used_by
            .iter()
            .find(|c| c["path"] == "src/Consumer.php")
            .expect("Consumer caller");
        assert_eq!(consumer["impact"], "broken_risk");
        let consumer_used = consumer["used_members"].as_array().unwrap();
        assert!(consumer_used.iter().any(|m| {
            m["name"] == "TTL_MIN"
                && m["kind"] == "const"
                && m["visibility"] == "private"
                && m["access_risk"] == "inaccessible"
        }));
        assert!(consumer_used.iter().any(|m| {
            m["name"] == "$region"
                && m["kind"] == "property"
                && m["visibility"] == "protected"
                && m["access_risk"] == "inaccessible"
        }));
        assert!(
            !consumer["visibility_warnings"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            cfg_impact["summary"]["visibility_warnings"]
                .as_u64()
                .unwrap()
                >= 2
        );

        // Subclass ChildConfig extends Config and reads protected $region => non-breaking narrowing warning, NOT broken_risk
        let child = used_by
            .iter()
            .find(|c| c["path"] == "src/ChildConfig.php")
            .expect("ChildConfig caller");
        assert_ne!(child["impact"], "broken_risk");
        assert!(
            child["used_members"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| { m["name"] == "$region" && m["access_risk"] == "visibility_narrowed" })
        );
        assert!(!child["visibility_warnings"].as_array().unwrap().is_empty());

        // WidenedCaller uses legacySecret (private -> public) => NOT broken_risk and no warnings
        let widened = used_by
            .iter()
            .find(|c| c["path"] == "src/WidenedCaller.php")
            .expect("WidenedCaller caller");
        assert_ne!(widened["impact"], "broken_risk");
        assert!(
            widened["visibility_warnings"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        // Outgoing `uses` view on Consumer.php tracks target_members, used_by_caller, member_refs, and warnings
        let consumer_impact = file_impact(
            &repo.root,
            None,
            Some(&resolved),
            None,
            "src/Consumer.php",
            false,
        )
        .unwrap();
        assert!(
            consumer_impact["summary"]["visibility_warnings"]
                .as_u64()
                .unwrap()
                >= 1
        );
        let unchanged_deps = consumer_impact["uses"]["unchanged"].as_array().unwrap();
        let on_config = |kind: &str| {
            unchanged_deps
                .iter()
                .find(|d| d["target"] == "src/Config.php" && d["kind"] == kind)
        };
        // Reading the constant and the property is a dependency of its own
        // kind, and the members read are listed under it alone. The type
        // hint stays a plain use: it lists no member, and still knows what
        // the caller uses of the target.
        let hint = on_config("uses").expect("Consumer -> Config uses dep");
        assert!(
            hint["member_refs"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|m| !matches!(m["kind"].as_str(), Some("property" | "const"))),
            "{hint}"
        );
        assert!(
            hint["target_members"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["name"] == "TTL_MIN" && m["used_by_caller"] == true)
        );
        assert!(on_config("writes").is_none());
        let cfg_dep = on_config("reads").expect("Consumer -> Config reads dep");
        assert_eq!(
            cfg_dep["member_refs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| format!(
                    "{} {} {}",
                    m["name"].as_str().unwrap(),
                    m["access"].as_str().unwrap(),
                    m["access_risk"].as_str().unwrap_or("-")
                ))
                .collect::<Vec<_>>(),
            ["$region read inaccessible", "TTL_MIN const inaccessible"]
        );
        assert!(
            cfg_dep["target_members"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["name"] == "TTL_MIN" && m["used_by_caller"] == true)
        );
        assert!(
            !cfg_dep["visibility_warnings"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn file_impact_handles_rust_pub_crate_narrowing_and_inaccessible_private_warning() {
        let repo = TempRepo::new_git();
        repo.write(
            "crates/core/src/store.rs",
            "pub fn save_record(id: u64) -> u64 { id }\npub fn always_ok() -> bool { true }\nfn crate_only_secret() -> u64 { 42 }\n",
        );
        repo.write(
            "crates/core/src/internal.rs",
            "use crate::store::{save_record, always_ok};\npub fn run() -> u64 { if always_ok() { save_record(1) } else { 0 } }\n",
        );
        repo.write(
            "crates/api/src/handler.rs",
            "use core::store::save_record;\npub fn handle() -> u64 { save_record(2) }\n",
        );
        repo.write(
            "crates/core/src/peeker.rs",
            "use crate::store::crate_only_secret;\npub fn peek() -> u64 { crate_only_secret() }\n",
        );
        let head = repo.commit("initial");

        // Narrow `save_record` from `pub` to `pub(crate)`.
        repo.write(
            "crates/core/src/store.rs",
            "pub(crate) fn save_record(id: u64) -> u64 { id }\npub fn always_ok() -> bool { true }\nfn crate_only_secret() -> u64 { 42 }\n",
        );

        let index = Index::read(
            json!({
                "nodes": [
                    {"id": "store_file", "label": "store", "source_file": "crates/core/src/store.rs"},
                    {"id": "store_save", "label": "save_record", "source_file": "crates/core/src/store.rs", "source_location": "L1"},
                    {"id": "api_handle", "label": "handle", "source_file": "crates/api/src/handler.rs", "source_location": "L2"}
                ],
                "links": [
                    {"source": "api_handle", "target": "store_save", "relation": "calls", "confidence": "EXTRACTED", "source_file": "crates/api/src/handler.rs"}
                ]
            })
            .to_string()
            .as_bytes(),
            &repo.root,
        )
        .unwrap();

        let resolved = resolved_work(
            &head,
            vec![Change {
                path: "crates/core/src/store.rs".into(),
                status: 'M',
                from: None,
            }],
        );

        let impact = file_impact(
            &repo.root,
            Some(&index),
            Some(&resolved),
            None,
            "crates/core/src/store.rs",
            false,
        )
        .unwrap();
        let used_by = impact["used_by"].as_array().unwrap();

        // Inside-crate caller: non-breaking visibility warning, NOT broken_risk
        let internal = used_by
            .iter()
            .find(|c| c["path"] == "crates/core/src/internal.rs")
            .expect("internal caller");
        assert_ne!(internal["impact"], "broken_risk");
        assert!(
            internal["used_members"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| {
                    m["name"] == "save_record" && m["access_risk"] == "visibility_narrowed"
                })
        );
        assert!(
            !internal["visibility_warnings"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        // Cross-crate caller: broken_risk because `pub(crate)` is inaccessible outside `crates/core`
        let api = used_by
            .iter()
            .find(|c| c["path"] == "crates/api/src/handler.rs")
            .expect("api caller");
        assert_eq!(api["impact"], "broken_risk");
        assert!(
            api["used_members"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| { m["name"] == "save_record" && m["access_risk"] == "inaccessible" })
        );

        // Outgoing `uses` edge via Index + static deps on handler.rs flags inaccessible `save_record`
        let handler_impact = file_impact(
            &repo.root,
            Some(&index),
            Some(&resolved),
            None,
            "crates/api/src/handler.rs",
            false,
        )
        .unwrap();
        assert!(
            handler_impact["summary"]["visibility_warnings"]
                .as_u64()
                .unwrap()
                >= 1
        );

        // Unchanged private helper `crate_only_secret` accessed from `peeker.rs` emits visibility warning (without narrowing break)
        let peeker = used_by
            .iter()
            .find(|c| c["path"] == "crates/core/src/peeker.rs")
            .expect("peeker caller");
        assert_ne!(peeker["impact"], "broken_risk");
        assert!(
            peeker["visibility_warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|w| { w["member"] == "crate_only_secret" && w["visibility"] == "private" })
        );
    }

    #[test]
    fn file_impact_distinguishes_deleted_member_and_private_set_read_vs_write() {
        let repo = TempRepo::new_git();
        repo.write(
            "src/Account.php",
            "<?php\nnamespace App;\nclass Account {\n    public int $balance = 0;\n    public function legacyReset(): void {}\n}\n",
        );
        repo.write(
            "src/Reader.php",
            "<?php\nnamespace App;\nuse App\\Account;\nclass Reader {\n    public function show(Account $acc): int { return $acc->balance; }\n}\n",
        );
        repo.write(
            "src/Writer.php",
            "<?php\nnamespace App;\nuse App\\Account;\nclass Writer {\n    public function bump(Account $acc): void { $acc->balance = 10; }\n}\n",
        );
        repo.write(
            "src/LegacyCaller.php",
            "<?php\nnamespace App;\nuse App\\Account;\nclass LegacyCaller {\n    public function wipe(Account $acc): void { $acc->legacyReset(); }\n}\n",
        );
        let head = repo.commit("initial");

        // Narrow `$balance` from `public` to `public private(set)` and remove `legacyReset`.
        repo.write(
            "src/Account.php",
            "<?php\nnamespace App;\nclass Account {\n    public private(set) int $balance = 0;\n}\n",
        );

        let resolved = resolved_work(
            &head,
            vec![Change {
                path: "src/Account.php".into(),
                status: 'M',
                from: None,
            }],
        );

        let impact = file_impact(
            &repo.root,
            None,
            Some(&resolved),
            None,
            "src/Account.php",
            false,
        )
        .unwrap();

        let declared = impact["declared_members"].as_array().unwrap();
        assert!(declared.iter().any(|m| {
            m["name"] == "legacyReset" && m["kind"] == "method" && m["change"] == "removed"
        }));

        let used_by = impact["used_by"].as_array().unwrap();
        let reader = used_by
            .iter()
            .find(|c| c["path"] == "src/Reader.php")
            .expect("Reader caller");
        assert_ne!(reader["impact"], "broken_risk");
        assert!(reader["used_members"].as_array().unwrap().iter().any(|m| {
            m["name"] == "$balance"
                && m["access"] == "read"
                && m["access_risk"] == "visibility_narrowed"
        }));

        let writer = used_by
            .iter()
            .find(|c| c["path"] == "src/Writer.php")
            .expect("Writer caller");
        assert_eq!(writer["impact"], "broken_risk");
        assert!(writer["used_members"].as_array().unwrap().iter().any(|m| {
            m["name"] == "$balance" && m["access"] == "write" && m["access_risk"] == "inaccessible"
        }));

        let legacy = used_by
            .iter()
            .find(|c| c["path"] == "src/LegacyCaller.php")
            .expect("LegacyCaller caller");
        assert_eq!(legacy["impact"], "broken_risk");
        assert!(
            legacy["used_members"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| { m["name"] == "legacyReset" && m["access_risk"] == "deleted" })
        );
    }

    /// A project of more files than one scan reads, where the one file that
    /// calls a removed method stands last, in the graph and by its path.
    fn wide_project(fillers: usize) -> (TempRepo, Resolved, Index) {
        let repo = TempRepo::new_git();
        repo.write(
            "src/Account.php",
            "<?php\nnamespace App;\nclass Account {\n    public function legacyReset(): void {}\n}\n",
        );
        repo.write(
            "zz/LegacyCaller.php",
            "<?php\nnamespace App\\Zz;\nuse App\\Account;\nclass LegacyCaller {\n    public function wipe(Account $acc): void { $acc->legacyReset(); }\n}\n",
        );
        let mut nodes = vec![
            json!({"id": "account", "label": "Account", "source_file": "src/Account.php", "source_location": "L3"}),
            json!({"id": "reset", "label": ".legacyReset()", "source_file": "src/Account.php", "source_location": "L4"}),
        ];
        for at in 0..fillers {
            let path = format!("aa/Filler{at:03}.php");
            repo.write(
                &path,
                &format!("<?php\nnamespace App\\Aa;\nclass Filler{at:03} {{}}\n"),
            );
            nodes.push(json!({"id": format!("filler{at}"), "label": format!("Filler{at:03}"), "source_file": path, "source_location": "L3"}));
        }
        nodes.push(json!({"id": "wipe", "label": ".wipe()", "source_file": "zz/LegacyCaller.php", "source_location": "L5"}));
        let head = repo.commit("initial");
        repo.write(
            "src/Account.php",
            "<?php\nnamespace App;\nclass Account {\n}\n",
        );
        let resolved = resolved_work(
            &head,
            vec![Change {
                path: "src/Account.php".into(),
                status: 'M',
                from: None,
            }],
        );
        let index = Index::read(
            json!({
                "nodes": nodes,
                "links": [
                    {"source": "wipe", "target": "reset", "relation": "calls", "confidence": "EXTRACTED", "source_file": "zz/LegacyCaller.php"}
                ]
            })
            .to_string()
            .as_bytes(),
            &repo.root,
        )
        .unwrap();
        (repo, resolved, index)
    }

    #[test]
    fn a_caller_the_graph_knows_is_read_past_the_limit_of_the_scan() {
        // More files stand before the caller than one scan reads. The
        // caller is known to the graph, so it is read all the same, and the
        // removed method it calls is told as a break. It was so before the
        // callers of the graph were put first in the scan as well: a caller
        // the scan left out is read when it is reported.
        let (repo, resolved, index) = wide_project(MAX_CALLER_SCAN + 12);
        assert_eq!(
            index_callers(Some(&index), "src/Account.php", None, false, 8),
            ["zz/LegacyCaller.php"]
        );
        let impact = file_impact(
            &repo.root,
            Some(&index),
            Some(&resolved),
            None,
            "src/Account.php",
            false,
        )
        .unwrap();
        let caller = impact["used_by"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["path"] == "zz/LegacyCaller.php")
            .expect("the caller the graph knows");
        assert_eq!(caller["impact"], "broken_risk", "{caller}");
        assert!(
            caller["used_members"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["name"] == "legacyReset" && m["access_risk"] == "deleted"),
            "{caller}"
        );
        // The catalog of the whole change reads the same caller.
        let catalog = changed_catalog(
            &repo.root,
            Some(&index),
            Some(&resolved),
            None,
            &[],
            "work",
            false,
        );
        let changed = catalog["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["path"] == "src/Account.php")
            .expect("the changed file");
        assert_eq!(changed["summary"]["callers_at_risk"], 1, "{changed}");
    }

    #[test]
    fn the_callers_of_the_graph_come_strongest_first() {
        let index = Index::read(
            json!({
                "nodes": [
                    {"id": "t", "label": "target()", "source_file": "lib/target.php", "source_location": "L2"},
                    {"id": "a1", "label": "a1()", "source_file": "app/a.php", "source_location": "L2"},
                    {"id": "b1", "label": "b1()", "source_file": "app/b.php", "source_location": "L2"},
                    {"id": "b2", "label": "b2()", "source_file": "app/b.php", "source_location": "L5"},
                    {"id": "c1", "label": "c1()", "source_file": "app/c.php", "source_location": "L2"}
                ],
                "links": [
                    {"source": "a1", "target": "t", "relation": "calls", "confidence": "EXTRACTED", "source_file": "app/a.php"},
                    {"source": "b1", "target": "t", "relation": "calls", "confidence": "EXTRACTED", "source_file": "app/b.php"},
                    {"source": "b2", "target": "t", "relation": "calls", "confidence": "EXTRACTED", "source_file": "app/b.php"},
                    {"source": "c1", "target": "t", "relation": "calls", "confidence": "INFERRED", "source_file": "app/c.php"}
                ]
            })
            .to_string()
            .as_bytes(),
            Path::new("."),
        )
        .unwrap();
        let callers =
            |certain, limit| index_callers(Some(&index), "lib/target.php", None, certain, limit);
        assert_eq!(callers(false, 8), ["app/b.php", "app/a.php", "app/c.php"]);
        assert_eq!(callers(true, 8), ["app/b.php", "app/a.php"]);
        assert_eq!(callers(false, 1), ["app/b.php"]);
        assert!(index_callers(Some(&index), "lib/none.php", None, false, 8).is_empty());
        assert!(index_callers(None, "lib/target.php", None, false, 8).is_empty());
    }

    #[test]
    fn changed_catalog_emits_renamed_files_deleted_callers_and_edge_diffs() {
        let repo = TempRepo::new_git();
        repo.write(
            "src/Legacy.php",
            "<?php\nnamespace App;\nclass Legacy {\n    public function run(): void {}\n}\n",
        );
        repo.write(
            "src/OldName.php",
            "<?php\nnamespace App;\nclass OldName {\n    public function ping(): void {}\n}\n",
        );
        repo.write(
            "src/Service.php",
            "<?php\nnamespace App;\nuse App\\Legacy;\nclass Service {\n    public function work(string $x): void {\n        $l = new Legacy();\n        $l->run();\n    }\n}\n",
        );
        repo.write(
            "src/BrokenCaller.php",
            "<?php\nnamespace App;\nuse App\\Legacy;\nclass BrokenCaller {\n    public function go(): void {\n        $l = new Legacy();\n        $l->run();\n    }\n}\n",
        );
        repo.write(
            "src/SvcCaller.php",
            "<?php\nnamespace App;\nuse App\\Service;\nclass SvcCaller {\n    public function call(Service $s): void {\n        $s->work('a');\n    }\n}\n",
        );
        repo.commit("base");

        // Commit changes: delete Legacy.php, rename OldName -> NewName, create DepNew,
        // and modify Service to drop Legacy, add DepNew, and change work() signature.
        repo.remove("src/Legacy.php");
        let rename = std::process::Command::new("git")
            .args(["mv", "src/OldName.php", "src/NewName.php"])
            .current_dir(&repo.root)
            .output()
            .unwrap();
        assert!(rename.status.success());
        repo.write(
            "src/DepNew.php",
            "<?php\nnamespace App;\nclass DepNew {\n    public function ok(): void {}\n}\n",
        );
        repo.write(
            "src/Service.php",
            "<?php\nnamespace App;\nuse App\\DepNew;\nclass Service {\n    public function work(string $x, int $y): void {\n        $d = new DepNew();\n        $d->ok();\n    }\n}\n",
        );
        let commit_sha = repo.commit("refactor");

        let resolved = crate::graph_changes::resolve(
            &repo.root,
            &crate::graph_changes::Target::Commit(commit_sha.clone()),
            &[],
        )
        .unwrap();
        let catalog = changed_catalog(
            &repo.root,
            None,
            Some(&resolved),
            None,
            &[],
            &format!("commit:{commit_sha}"),
            false,
        );

        assert_eq!(catalog["total_files"], 4, "{catalog}");
        assert_eq!(catalog["created_files"], 1, "{catalog}");
        assert_eq!(catalog["modified_files"], 1, "{catalog}");
        assert_eq!(catalog["deleted_files"], 1, "{catalog}");
        assert_eq!(catalog["renamed_files"], 1, "{catalog}");

        let files = catalog["files"].as_array().unwrap();
        let renamed = files
            .iter()
            .find(|f| f["path"] == "src/NewName.php")
            .expect("renamed entry");
        assert_eq!(renamed["status"], "renamed");
        assert_eq!(renamed["previous_path"], "src/OldName.php");

        let deleted = files
            .iter()
            .find(|f| f["path"] == "src/Legacy.php")
            .expect("deleted entry");
        assert_eq!(deleted["status"], "deleted");
        assert_eq!(deleted["exists"], false);
        let del_callers = deleted["callers"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>();
        assert!(del_callers.contains(&"src/BrokenCaller.php"), "{deleted}");
        assert!(del_callers.contains(&"src/Service.php"), "{deleted}");

        let edge_diffs = &catalog["edge_diffs"];
        let added = edge_diffs["added"].as_array().unwrap();
        let removed = edge_diffs["removed"].as_array().unwrap();
        let risk_callers = edge_diffs["risk_callers"].as_array().unwrap();

        let added_svc_new = added
            .iter()
            .find(|e| e["from"] == "src/Service.php" && e["to"] == "src/DepNew.php")
            .expect("added Service -> DepNew");
        assert!(!added_svc_new["kinds"].as_array().unwrap().is_empty());

        let removed_svc_legacy = removed
            .iter()
            .find(|e| e["from"] == "src/Service.php" && e["to"] == "src/Legacy.php")
            .expect("removed Service -> Legacy");
        assert!(!removed_svc_legacy["kinds"].as_array().unwrap().is_empty());

        assert!(
            risk_callers.iter().any(|r| {
                r["from"] == "src/BrokenCaller.php"
                    && r["to"] == "src/Legacy.php"
                    && r["impact"] == "broken_risk"
            }),
            "{edge_diffs}"
        );
        assert!(
            risk_callers.iter().any(|r| {
                r["from"] == "src/SvcCaller.php"
                    && r["to"] == "src/Service.php"
                    && r["impact"] == "broken_risk"
            }),
            "{edge_diffs}"
        );
    }

    #[test]
    fn same_scope_crate_or_package_walks_to_enclosing_manifest_and_source_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("composer.json"), "{}").unwrap();
        std::fs::create_dir_all(root.join("packages/billing")).unwrap();
        std::fs::write(root.join("packages/billing/composer.json"), "{}").unwrap();

        assert!(same_scope_crate_or_package_in(
            Some(root),
            None,
            "app/A.php",
            "app/B/C.php"
        ));
        assert!(!same_scope_crate_or_package_in(
            Some(root),
            None,
            "app/A.php",
            "packages/billing/Invoice.php"
        ));
        assert!(same_scope_crate_or_package_in(
            Some(root),
            None,
            "packages/billing/src/Model.php",
            "packages/billing/src/Nested/Repo.php"
        ));

        // Without manifests on disk, nested paths under the same source root match while
        // different packages / monorepo containers remain isolated.
        assert!(same_scope_crate_or_package("app/A.php", "app/B/C.php"));
        assert!(!same_scope_crate_or_package("app/A.php", "other/D.php"));
        assert!(!same_scope_crate_or_package(
            "packages/auth/User.php",
            "packages/billing/Invoice.php"
        ));
    }

    #[test]
    fn within_rust_pub_in_rejects_unknown_or_unmatched_scopes() {
        assert!(!within_rust_pub_in(
            "pub(in unknown::scope)",
            "crates/core/src/a.rs",
            "crates/core/src/b.rs"
        ));
        assert!(!within_rust_pub_in(
            "pub(in )",
            "crates/core/src/a.rs",
            "crates/core/src/b.rs"
        ));
        assert!(!within_rust_pub_in(
            "pub(in crate::other)",
            "crates/core/src/api/handler.rs",
            "crates/core/src/api/model.rs"
        ));
        assert!(within_rust_pub_in(
            "pub(in crate::api)",
            "crates/core/src/api/handler.rs",
            "crates/core/src/api/model.rs"
        ));
        assert!(!within_rust_pub_in(
            "pub(in crate::api)",
            "crates/core/src/other/handler.rs",
            "crates/core/src/api/model.rs"
        ));
    }

    #[test]
    fn caller_extends_target_supports_multihop_and_detects_cycles() {
        let mut known = BTreeSet::new();
        for f in [
            "src/Base.php",
            "src/Mid.php",
            "src/Child.php",
            "src/TraitUser.php",
            "src/TargetTrait.php",
            "src/CycleA.php",
            "src/CycleB.php",
        ] {
            known.insert(f.to_owned());
        }
        let sources: HashMap<&str, &str> = HashMap::from([
            (
                "src/Base.php",
                "<?php\nclass Base {\n    protected function secret(): void {}\n}\n",
            ),
            ("src/Mid.php", "<?php\nclass Mid extends Base {}\n"),
            (
                "src/Child.php",
                "<?php\nclass Child extends Mid {\n    public function run() { $this->secret(); }\n}\n",
            ),
            (
                "src/TargetTrait.php",
                "<?php\ntrait TargetTrait {\n    protected function helper(): void {}\n}\n",
            ),
            (
                "src/TraitUser.php",
                "<?php\nclass TraitMid { use TargetTrait; }\nclass TraitUser extends TraitMid {}\n",
            ),
            ("src/CycleA.php", "<?php\nclass CycleA extends CycleB {}\n"),
            ("src/CycleB.php", "<?php\nclass CycleB extends CycleA {}\n"),
        ]);
        let mut reader = |p: &str| sources.get(p).map(|s| (*s).to_owned());
        let base_members = vec![DeclaredMember {
            name: "secret".into(),
            kind: "method".into(),
            parent: Some("Base".into()),
            line: Some(3),
            signature: None,
            visibility: Some("protected".into()),
            previous_visibility: Some("public".into()),
            modifiers: Vec::new(),
            change: "visibility_changed".into(),
            signature_changed: false,
            visibility_narrowed: true,
            used_by_caller: false,
        }];
        assert!(caller_extends_target_in(
            "src/Child.php",
            sources.get("src/Child.php").copied(),
            "src/Base.php",
            &base_members,
            &known,
            &mut reader,
        ));
        assert!(!caller_extends_target_in(
            "src/CycleA.php",
            sources.get("src/CycleA.php").copied(),
            "src/Base.php",
            &base_members,
            &known,
            &mut reader,
        ));
        let trait_members = vec![DeclaredMember {
            name: "TargetTrait".into(),
            kind: "trait".into(),
            parent: None,
            line: Some(2),
            signature: None,
            visibility: Some("public".into()),
            previous_visibility: None,
            modifiers: Vec::new(),
            change: "unchanged".into(),
            signature_changed: false,
            visibility_narrowed: false,
            used_by_caller: false,
        }];
        assert!(caller_extends_target_in(
            "src/TraitUser.php",
            sources.get("src/TraitUser.php").copied(),
            "src/TargetTrait.php",
            &trait_members,
            &known,
            &mut reader,
        ));

        // End-to-end verification in `file_impact`: a 2-hop protected narrowing stays non-breaking
        // for `Child extends Mid` (where `Mid extends Base`), while an unrelated `Outsider` breaks.
        let repo = TempRepo::new_git();
        repo.write(
            "src/Base.php",
            "<?php\nnamespace App;\nclass Base {\n    public static function secret(): void {}\n}\n",
        );
        repo.write(
            "src/Mid.php",
            "<?php\nnamespace App;\nclass Mid extends Base {}\n",
        );
        repo.write(
            "src/Child.php",
            "<?php\nnamespace App;\nclass Child extends Mid {\n    public function run(): void {\n        Base::secret();\n    }\n}\n",
        );
        repo.write(
            "src/Outsider.php",
            "<?php\nnamespace App;\nclass Outsider {\n    public function run(): void {\n        Base::secret();\n    }\n}\n",
        );
        let head = repo.commit("base");
        repo.write(
            "src/Base.php",
            "<?php\nnamespace App;\nclass Base {\n    protected static function secret(): void {}\n}\n",
        );
        let resolved = resolved_work(
            &head,
            vec![Change {
                path: "src/Base.php".into(),
                status: 'M',
                from: None,
            }],
        );
        let impact = file_impact(
            &repo.root,
            None,
            Some(&resolved),
            None,
            "src/Base.php",
            false,
        )
        .unwrap();
        let used_by = impact["used_by"].as_array().unwrap();
        let child = used_by
            .iter()
            .find(|c| c["path"] == "src/Child.php")
            .expect("Child caller");
        assert_ne!(child["impact"], "broken_risk", "{child}");
        let outsider = used_by
            .iter()
            .find(|c| c["path"] == "src/Outsider.php")
            .expect("Outsider caller");
        assert_eq!(outsider["impact"], "broken_risk", "{outsider}");
    }

    #[test]
    fn inverted_caller_scan_finds_unindexed_caller_past_scan_limit() {
        let repo = TempRepo::new_git();
        repo.write(
            "src/BillingTarget.php",
            "<?php\nnamespace App\\Billing;\nclass BillingTarget {\n    public function settleInvoice(): void {}\n}\n",
        );
        for i in 0..(MAX_CALLER_SCAN + 40) {
            repo.write(
                &format!("aa/Filler{i:04}.php"),
                &format!("<?php\nnamespace App\\Fill;\nclass Filler{i:04} {{}}\n"),
            );
        }
        repo.write(
            "zz/LateCaller.php",
            "<?php\nnamespace App\\Zz;\nuse App\\Billing\\BillingTarget;\nclass LateCaller {\n    public function run(): void {\n        $b = new BillingTarget();\n        $b->settleInvoice();\n    }\n}\n",
        );
        let head = repo.commit("many-files");
        repo.write(
            "src/BillingTarget.php",
            "<?php\nnamespace App\\Billing;\nclass BillingTarget {\n    public function settleInvoice(int $cents): void {}\n}\n",
        );
        let resolved = resolved_work(
            &head,
            vec![Change {
                path: "src/BillingTarget.php".into(),
                status: 'M',
                from: None,
            }],
        );

        let impact = file_impact(
            &repo.root,
            None,
            Some(&resolved),
            None,
            "src/BillingTarget.php",
            false,
        )
        .unwrap();
        let used_by = impact["used_by"].as_array().unwrap();
        let late = used_by
            .iter()
            .find(|c| c["path"] == "zz/LateCaller.php")
            .expect("unindexed caller past MAX_CALLER_SCAN should be found");
        assert_eq!(late["impact"], "broken_risk", "{late}");

        // Also check when an incomplete Index exists that lacks the edge to zz/LateCaller.php.
        let partial_index = Index::read(
            json!({
                "nodes": [
                    {"id": "bt", "label": "BillingTarget", "source_file": "src/BillingTarget.php"},
                    {"id": "f0", "label": "Filler0000", "source_file": "aa/Filler0000.php"}
                ],
                "links": []
            })
            .to_string()
            .as_bytes(),
            &repo.root,
        )
        .unwrap();
        let impact_with_idx = file_impact(
            &repo.root,
            Some(&partial_index),
            Some(&resolved),
            None,
            "src/BillingTarget.php",
            false,
        )
        .unwrap();
        assert!(
            impact_with_idx["used_by"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["path"] == "zz/LateCaller.php" && c["impact"] == "broken_risk"),
            "{impact_with_idx}"
        );
    }
}
