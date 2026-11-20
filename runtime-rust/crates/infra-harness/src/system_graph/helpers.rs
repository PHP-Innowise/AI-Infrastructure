use super::resolver::{ResolvedServiceTarget, agent_matches};
use crate::{
    graph::Graphs,
    graph_map::{self, Index},
};
use infra_system::{
    ChangedFileEntry, DispatchEvidence, ServiceCodeSummary, catalog::System,
    collect_run_service_summaries,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub(super) fn collect_service_code_files(root: &Path, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut visited_dirs = 0usize;
    while let Some(dir) = stack.pop() {
        if out.len() >= limit || visited_dirs >= 120 {
            break;
        }
        visited_dirs += 1;
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut sorted: Vec<_> = entries.flatten().collect();
        sorted.sort_by_key(|e| e.file_name());
        for entry in sorted {
            if out.len() >= limit {
                break;
            }
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.')
                || matches!(
                    name_str.as_ref(),
                    "vendor" | "node_modules" | "target" | "dist" | "build" | "storage" | "var"
                )
            {
                continue;
            }
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if matches!(
                    ext.as_str(),
                    "php"
                        | "rs"
                        | "ts"
                        | "tsx"
                        | "js"
                        | "jsx"
                        | "py"
                        | "go"
                        | "java"
                        | "kt"
                        | "rb"
                        | "cs"
                        | "swift"
                        | "scala"
                ) && let Ok(rel) = path.strip_prefix(root)
                {
                    let rel_str = rel.to_string_lossy().replace('\\', "/");
                    if !graph_map::is_test_path(&rel_str) {
                        out.push(rel_str);
                    }
                }
            }
        }
    }
    out
}

pub(super) fn intra_service_caller_paths(
    idx: &Index,
    seed_paths: &[String],
    certain: bool,
) -> Vec<String> {
    let mut visited: BTreeSet<u32> = seed_paths.iter().filter_map(|p| idx.file_id(p)).collect();
    if visited.is_empty() {
        return Vec::new();
    }
    let mut frontier = visited.clone();
    let files = idx.files();
    for _ in 0..2 {
        let mut next = BTreeSet::new();
        for (from, to, pair) in idx.pairs() {
            if frontier.contains(to)
                && !visited.contains(from)
                && pair.weight(certain) > 0
                && let Some(p) = files.get(*from as usize)
                && !graph_map::is_test_path(p)
            {
                visited.insert(*from);
                next.insert(*from);
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    let seed_set: BTreeSet<&str> = seed_paths.iter().map(String::as_str).collect();
    visited
        .into_iter()
        .filter_map(|fid| files.get(fid as usize))
        .filter(|p| !seed_set.contains(p.as_str()))
        .cloned()
        .collect()
}

pub(super) fn augment_summaries_with_intra_callers(
    graphs: &Graphs,
    targets: &[ResolvedServiceTarget],
    summaries: &mut BTreeMap<String, ServiceCodeSummary>,
    certain: bool,
) {
    for target in targets {
        if target.service_id == "__system__" {
            continue;
        }
        let Some(summary) = summaries.get_mut(&target.service_id) else {
            continue;
        };
        if summary.changed_files.is_empty() {
            continue;
        }
        let Some(idx) = graphs.live_index(&target.graph_id) else {
            continue;
        };
        let seeds: Vec<String> = summary
            .changed_files
            .iter()
            .map(|c| c.path.clone())
            .collect();
        let callers = intra_service_caller_paths(&idx, &seeds, certain);
        for caller in callers {
            if !summary.changed_files.iter().any(|c| c.path == caller) {
                summary.changed_files.push(ChangedFileEntry {
                    is_test: graph_map::is_test_path(&caller),
                    path: caller,
                    status: "M".to_owned(),
                    from: None,
                    additions: None,
                    deletions: None,
                });
            }
        }
    }
}

pub(super) fn normalize_graph_action(raw: &str) -> &str {
    match raw.trim() {
        "agents_on" => "offer",
        "agents_off" => "withhold",
        other => other,
    }
}

pub(super) fn is_runtime_artifact_path(raw: &str) -> bool {
    let norm = raw.replace('\\', "/");
    let trimmed = norm.trim_start_matches("./");
    let check = |p: &str| {
        p.starts_with("project-brain/")
            || p.starts_with("memory-bank/")
            || p.starts_with(".agentic-security/")
    };
    if check(trimmed) {
        return true;
    }
    if let Some((_, rest)) = trimmed.split_once('/')
        && check(rest)
    {
        return true;
    }
    false
}

pub(super) fn collect_opened_by_service_for_run(
    dispatches: &[DispatchEvidence],
    agent_filter: Option<&str>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut opened_by_service: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for ev in dispatches {
        if !agent_matches(agent_filter, &ev.phase, &ev.service) {
            continue;
        }
        let bucket = if ev.phase == "verify" {
            "verify".to_owned()
        } else {
            ev.service.clone()
        };
        for p in ev.opened.iter().chain(ev.shell_reads.keys()) {
            let norm = p.replace('\\', "/");
            let trimmed = norm.trim_start_matches("./");
            if ev.service == "__system__"
                && let Some((svc, rest)) = trimmed.split_once('/')
            {
                opened_by_service
                    .entry(svc.to_owned())
                    .or_default()
                    .insert(rest.to_owned());
            }
            opened_by_service
                .entry(bucket.clone())
                .or_default()
                .insert(trimmed.to_owned());
        }
    }
    opened_by_service
}

pub(super) fn build_run_scoped_summaries(
    system: &System,
    dispatches: &[DispatchEvidence],
) -> BTreeMap<String, ServiceCodeSummary> {
    let mut summaries = collect_run_service_summaries(system, dispatches);
    for (sid, svc) in &system.services {
        if svc.access != "available" {
            continue;
        }
        let svc_prefix = format!("{sid}/");
        for ev in dispatches {
            if ev.service == *sid {
                continue;
            }
            for (p, eid) in &ev.shell_reads {
                let norm = p.replace('\\', "/");
                if let Some(rel) = norm.trim_start_matches("./").strip_prefix(&svc_prefix)
                    && !rel.is_empty()
                {
                    let entry = summaries.entry(sid.clone()).or_default();
                    entry.opened_files.insert(rel.to_owned());
                    entry.shell_reads.insert(rel.to_owned(), *eid);
                }
            }
            for ch in ev.git_changed.iter().chain(ev.ledger_edited.iter()) {
                let norm = ch.path.replace('\\', "/");
                if let Some(rel) = norm.trim_start_matches("./").strip_prefix(&svc_prefix)
                    && !rel.is_empty()
                    && !is_runtime_artifact_path(rel)
                {
                    let entry = summaries.entry(sid.clone()).or_default();
                    if !entry.changed_files.iter().any(|f| f.path == rel) {
                        entry.changed_files.push(ChangedFileEntry {
                            is_test: graph_map::is_test_path(rel),
                            path: rel.to_owned(),
                            status: ch.status.clone(),
                            from: ch.from.clone(),
                            additions: None,
                            deletions: None,
                        });
                    }
                }
            }
        }
    }
    summaries
}

pub(super) fn normalize_cross_unopened_entry(raw: &Value) -> Value {
    let mut item = raw.clone();
    if item.is_object()
        && item.get("service").is_none()
        && let Some(consumer) = item["consumer"].as_str().map(str::to_owned)
    {
        item["service"] = json!(consumer);
    }
    item
}

pub(super) fn normalize_cross_unopened_list(raw: &Value) -> Value {
    match raw.as_array() {
        Some(arr) => Value::Array(arr.iter().map(normalize_cross_unopened_entry).collect()),
        None => json!([]),
    }
}
