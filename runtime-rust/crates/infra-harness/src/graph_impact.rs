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
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    path::{Component, Path},
};

const MAX_SOURCE_BYTES: usize = 2 * 1024 * 1024;
const GIT_TIMEOUT_SECS: u64 = 15;
const MAX_WALK_FILES: usize = 5000;
const MAX_WALK_DEPTH: usize = 12;
const MAX_CALLER_SCAN: usize = 300;
const MAX_CYCLE_DEPTH: usize = 6;
const MAX_CYCLES_REPORTED: usize = 24;
const MAX_CYCLE_VISITS: usize = 20_000;

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
        .filter(|(_, entry)| entry.status != "deleted" || entry.previous_path.is_some())
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
    let candidate_callers =
        collect_workspace_scan_candidates(&ctx, index, &merged, live_focus, &known_callers);
    let caller_cache = build_caller_dep_cache(&ctx, &candidate_callers);
    let index_incoming = build_index_incoming(index);

    let mut existing_files = 0usize;
    let mut created_files = 0usize;
    let mut modified_files = 0usize;
    let mut deleted_files = 0usize;
    let mut created_entities = 0usize;
    let mut modified_entities = 0usize;
    let mut deleted_entities = 0usize;
    let mut files_json = Vec::with_capacity(merged.len());

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
            &caller_cache,
        );
        let callers_at_risk = callers.iter().filter(|c| c.impact == "broken_risk").count();
        let impacted_callers = callers
            .iter()
            .filter(|c| matches!(c.impact, "broken_risk" | "impacted"))
            .count();
        let visibility_warnings: usize = callers.iter().map(|c| c.visibility_warnings.len()).sum();

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

    json!({
        "target": target_label,
        "total_files": merged.len(),
        "existing_files": existing_files,
        "created_files": created_files,
        "modified_files": modified_files,
        "deleted_files": deleted_files,
        "created_entities": created_entities,
        "modified_entities": modified_entities,
        "deleted_entities": deleted_entities,
        "files": files_json,
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
    let candidates =
        collect_workspace_scan_candidates(&ctx, index, &scan_scope, live_focus, &known_callers);
    let caller_cache = build_caller_dep_cache(&ctx, &candidates);
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
        &caller_cache,
    );

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
            &caller_cache,
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

fn line_has_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let bytes = haystack.as_bytes();
    let n_bytes = needle.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        let abs = start + pos;
        let left_ok = abs == 0 || !is_ident(bytes[abs - 1]);
        let right = abs + n_bytes.len();
        let right_ok = right >= bytes.len() || !is_ident(bytes[right]);
        if left_ok && right_ok {
            return true;
        }
        start = abs + n_bytes.len();
    }
    false
}

/// Returns `true` when `caller_source` explicitly extends/subclasses or uses a trait declared in `target_path`.
pub(crate) fn caller_extends_target(
    caller_source: Option<&str>,
    target_path: &str,
    target_members: &[DeclaredMember],
) -> bool {
    let Some(src) = caller_source else {
        return false;
    };
    let mut target_types: Vec<String> = Vec::new();
    for m in target_members {
        if matches!(
            m.kind.as_str(),
            "class" | "abstract_class" | "trait" | "interface" | "struct"
        ) && !m.name.is_empty()
            && !target_types.contains(&m.name)
        {
            target_types.push(m.name.clone());
        }
        if let Some(p) = &m.parent
            && !p.is_empty()
            && !target_types.contains(p)
        {
            target_types.push(p.clone());
        }
    }
    if let Some(stem) = file_stem_name(target_path)
        && !target_types.contains(&stem)
    {
        target_types.push(stem);
    }
    if target_types.is_empty() {
        return false;
    }

    let mut inside_class = false;
    for raw_line in src.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with('#')
            || trimmed.starts_with("/*")
            || trimmed.starts_with('*')
        {
            continue;
        }
        let is_class_header = trimmed.starts_with("class ")
            || trimmed.contains(" class ")
            || trimmed.starts_with("abstract class ")
            || trimmed.starts_with("final class ")
            || trimmed.starts_with("export class ");
        if is_class_header {
            inside_class = true;
            for ty in &target_types {
                if (trimmed.contains(" extends ") && line_has_word(trimmed, ty))
                    || (trimmed.contains('(') && line_has_word(trimmed, ty))
                    || (trimmed.contains(" : ") && line_has_word(trimmed, ty))
                    || (trimmed.contains(" < ") && line_has_word(trimmed, ty))
                {
                    return true;
                }
            }
        } else if trimmed.contains(" extends ")
            || (inside_class && trimmed.starts_with("use ") && trimmed.ends_with(';'))
        {
            for ty in &target_types {
                if line_has_word(trimmed, ty) {
                    return true;
                }
            }
        }
    }
    false
}

fn rust_crate_root(rel_path: &str) -> (String, bool) {
    let rel = rel_path.trim_start_matches("./");
    if let Some(idx) = rel.find("/src/") {
        return (rel[..idx].to_owned(), true);
    }
    if rel.starts_with("src/") {
        return (String::new(), true);
    }
    if let Some(idx) = rel.find("/tests/") {
        return (rel[..idx].to_owned(), false);
    }
    if rel.starts_with("tests/") {
        return (String::new(), false);
    }
    (folder_of(rel), true)
}

fn same_scope_crate_or_package(caller_path: &str, target_path: &str) -> bool {
    let caller = clean_rel_key(caller_path);
    let target = clean_rel_key(target_path);
    let (c_root, c_in_src) = rust_crate_root(&caller);
    let (t_root, t_in_src) = rust_crate_root(&target);
    if !c_in_src || !t_in_src {
        return false;
    }
    c_root == t_root
}

fn rust_crate_src_dir(rel_path: &str) -> String {
    let rel = rel_path.trim_start_matches("./");
    if let Some(idx) = rel.find("/src/") {
        rel[..idx + 4].to_owned()
    } else if rel.starts_with("src/") {
        "src".to_owned()
    } else {
        folder_of(rel)
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

fn within_rust_super_mod(caller_path: &str, target_path: &str) -> bool {
    if !same_scope_crate_or_package(caller_path, target_path) {
        return false;
    }
    let super_dir = rust_super_dir(target_path);
    path_in_rust_module_dir(caller_path, &super_dir)
}

fn within_rust_pub_in(vis_lower: &str, caller_path: &str, target_path: &str) -> bool {
    if !same_scope_crate_or_package(caller_path, target_path) {
        return false;
    }
    let Some(inner) = vis_lower
        .strip_prefix("pub(in ")
        .and_then(|rest| rest.strip_suffix(')'))
        .map(str::trim)
    else {
        return true;
    };
    let segs: Vec<&str> = inner
        .split("::")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if segs.is_empty() {
        return true;
    }
    if segs[0] == "self" && segs.len() == 1 {
        return clean_rel_key(caller_path) == clean_rel_key(target_path);
    }
    if segs[0] == "crate" {
        let target = clean_rel_key(target_path);
        let src_dir = rust_crate_src_dir(&target);
        if segs.len() == 1 {
            return true;
        }
        let mod_sub = segs[1..].join("/");
        let full_dir = if src_dir.is_empty() {
            mod_sub
        } else {
            format!("{src_dir}/{mod_sub}")
        };
        return path_in_rust_module_dir(caller_path, &full_dir);
    }
    if segs[0] == "super" {
        let mut dir = rust_super_dir(target_path);
        for seg in segs.iter().skip(1) {
            if *seg == "super" {
                let p = folder_of(&dir);
                if !p.is_empty() {
                    dir = p.to_owned();
                }
            } else if !seg.is_empty() {
                dir = if dir.is_empty() {
                    (*seg).to_owned()
                } else {
                    format!("{dir}/{seg}")
                };
            }
        }
        return path_in_rust_module_dir(caller_path, &dir);
    }
    true
}

/// Evaluates accessibility, risk classification, and visibility warnings for `tm` from `caller_path`.
pub(crate) fn assess_member_access(
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
                if !same_scope_crate_or_package(caller_path, target_path) {
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
                if !within_rust_super_mod(caller_path, target_path) {
                    inaccessible_reason = Some(format!(
                        "Accesses pub(super) {} '{}' in {} outside parent module from {}",
                        tm.kind, tm.name, target_path, caller_path
                    ));
                }
            } else if lower.starts_with("pub(in ") {
                if !within_rust_pub_in(&lower, caller_path, target_path) {
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
pub(crate) fn enrich_member_ref(
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
        let assess = assess_member_access(
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

        let caller_extends = caller_extends_target(caller_src, &dep.target, &dep.target_members);
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
            enrich_member_ref(
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

/// The files read to find who uses a changed file, at most
/// `MAX_CALLER_SCAN`: the changed files, what the run is working on, the
/// callers the graph knows (`known_callers`, strongest first), and only
/// then the rest of the project in the order of its paths.
fn collect_workspace_scan_candidates(
    ctx: &TargetContext<'_>,
    index: Option<&Index>,
    changed: &BTreeMap<String, MergedChange>,
    live_focus: Option<&GraphFocus>,
    known_callers: &[String],
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut push = |p: &str| {
        if out.len() < MAX_CALLER_SCAN && is_scannable_code(p) && seen.insert(p.to_owned()) {
            out.push(p.to_owned());
        }
    };

    for (path, entry) in changed {
        if entry.status != "deleted" {
            push(path);
        }
    }
    if let Some(lf) = live_focus {
        for raw in &lf.target_paths {
            if let Some(p) = normalize_rel_opt(raw) {
                push(&p);
            }
        }
        for entity in &lf.created_entities {
            if let Some(p) = normalize_rel_opt(&entity.path) {
                push(&p);
            }
        }
    }
    for caller in known_callers {
        push(caller);
    }
    if let Some(idx) = index {
        for path in idx.files() {
            push(path);
        }
    }
    for path in ctx.existing_files() {
        push(path);
    }
    out
}

fn build_caller_dep_cache(
    ctx: &TargetContext<'_>,
    candidates: &[String],
) -> HashMap<String, (String, Vec<DepRef>)> {
    let mut cache = HashMap::new();
    for path in candidates {
        if let BlobRead::Ok(src) = ctx.read_after(path) {
            let deps = extract_file_deps(path, &src, &ctx.resolution_files);
            cache.insert(path.clone(), (src, deps));
        }
    }
    cache
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
    caller_cache: &HashMap<String, (String, Vec<DepRef>)>,
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

    for (caller_path, (caller_src, caller_deps)) in caller_cache {
        if caller_path == rel_path {
            continue;
        }
        let matching_deps: Vec<&DepRef> = caller_deps
            .iter()
            .filter(|d| {
                d.resolved
                    && (d.target == rel_path || previous_path.is_some_and(|old| d.target == old))
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
                let dropped = ctx
                    .read_before(caller_path)
                    .into_opt()
                    .is_some_and(|before_src| {
                        let before_deps =
                            extract_file_deps(caller_path, &before_src, &ctx.resolution_files);
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

        let caller_extends =
            caller_extends_target(acc.source_after.as_deref(), rel_path, declared_members);
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
            let assess = enrich_member_ref(
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
                let assess = assess_member_access(
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
}
