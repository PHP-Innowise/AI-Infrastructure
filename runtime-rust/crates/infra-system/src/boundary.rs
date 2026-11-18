use crate::{
    catalog::System,
    common::*,
    execution::{access_scope, dispatch_access},
    system_graph::EvidenceFileChange,
};
use infra_core::Result;
use infra_host::{
    HarnessError,
    permission::{Answer, Approvals, Decision, PermissionAsk, PermissionMode},
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use tokio::sync::oneshot;

/// Captures the filesystem and contract-ownership boundary for one system dispatch step.
#[derive(Clone, Debug)]
pub struct BoundaryScope {
    pub service: String,
    pub mode: String,
    pub access: String,
    pub own_root: PathBuf,
    pub system_root: PathBuf,
    pub selected_roots: BTreeMap<String, PathBuf>,
    pub writable_roots: Vec<PathBuf>,
    pub contract_sources: BTreeMap<PathBuf, Vec<(String, String)>>,
}

impl BoundaryScope {
    /// Builds a [`BoundaryScope`] from a loaded [`System`], run state, checkpoint plan, and dispatch step.
    pub fn from_run(system: &System, state: &Value, plan: &Value, step: &Value) -> Result<Self> {
        let service = string(&step["service"]).to_string();
        let mode = string(&step["mode"]).to_string();
        let access = access_scope(state).to_string();
        let system_root = normalize_path(&system.root);

        let mut selected_roots = BTreeMap::new();
        for sid in texts(&state["selected"]) {
            let raw_root = state["roots"]
                .get(&sid)
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .or_else(|| system.services.get(&sid).map(|svc| svc.root.clone()));
            if let Some(root) = raw_root {
                selected_roots.insert(sid, normalize_path(&root));
            }
        }

        let own_root = if service == "__system__" {
            system_root.clone()
        } else if let Some(root) = state["roots"].get(&service).and_then(Value::as_str) {
            normalize_path(Path::new(root))
        } else if let Some(root) = selected_roots.get(&service) {
            root.clone()
        } else if let Some(svc) = system.services.get(&service) {
            normalize_path(&svc.root)
        } else {
            return Err(fail(format!("Missing root for service {service}")));
        };

        let step_access = dispatch_access(state, step);
        let mut writable_roots = Vec::new();
        for sid in texts(&step_access["writable"]) {
            let root = if sid == "__system__" {
                None
            } else {
                selected_roots.get(&sid).cloned().or_else(|| {
                    state["roots"]
                        .get(&sid)
                        .and_then(Value::as_str)
                        .map(|p| normalize_path(Path::new(p)))
                })
            };
            if let Some(root) = root
                && !writable_roots.contains(&root)
            {
                writable_roots.push(root);
            }
        }

        let mut contract_sources: BTreeMap<PathBuf, Vec<(String, String)>> = BTreeMap::new();
        for (sid, svc) in &system.services {
            let Some(manifest) = &svc.manifest else {
                continue;
            };
            let svc_root = normalize_path(&svc.root);
            for contract in manifest["provides"].as_array().into_iter().flatten() {
                let cid = string(&contract["id"]).to_string();
                if cid.is_empty() {
                    continue;
                }
                for rel in texts(&contract["sources"]) {
                    let full = normalize_path(&svc_root.join(&rel));
                    let entry = contract_sources.entry(full).or_default();
                    let pair = (sid.clone(), cid.clone());
                    if !entry.contains(&pair) {
                        entry.push(pair);
                    }
                }
            }
        }

        for src in plan["context"]["sources"].as_array().into_iter().flatten() {
            if src["kind"] != "contract" {
                continue;
            }
            let sid = string(&src["service"]).to_string();
            let rel = string(&src["path"]);
            if sid.is_empty() || rel.is_empty() {
                continue;
            }
            let root = if sid == "__system__" {
                Some(system_root.clone())
            } else {
                selected_roots.get(&sid).cloned().or_else(|| {
                    system
                        .services
                        .get(&sid)
                        .map(|svc| normalize_path(&svc.root))
                })
            };
            if let Some(root) = root {
                let full = normalize_path(&root.join(rel));
                let entry = contract_sources.entry(full).or_default();
                if !entry.iter().any(|(owner, _)| owner == &sid) {
                    entry.push((sid, "contract".to_string()));
                }
            }
        }

        Ok(Self {
            service,
            mode,
            access,
            own_root,
            system_root,
            selected_roots,
            writable_roots,
            contract_sources,
        })
    }
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    None | Some(Component::RootDir | Component::Prefix(_))
                ) {
                    out.pop();
                }
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                out.push(component.as_os_str());
            }
        }
    }
    out
}

fn normalize_path(path: &Path) -> PathBuf {
    if path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    infra_core::fs::canonical_or_missing(path).unwrap_or_else(|_| normalize_lexical(path))
}

fn norm_starts_with(path: &Path, root: &Path) -> bool {
    !root.as_os_str().is_empty() && path.starts_with(root)
}

#[derive(Debug)]
enum Candidate {
    Home(String),
    Path(PathBuf),
}

fn is_virtual_device(raw: &str) -> bool {
    matches!(
        raw,
        "/dev/null" | "/dev/stdin" | "/dev/stdout" | "/dev/stderr" | "/dev/zero" | "/dev/urandom"
    )
}

fn resolve_raw_candidate(raw: &str, own_root: &Path) -> Option<Candidate> {
    let trimmed = raw
        .trim()
        .trim_matches(|c| matches!(c, '\'' | '"' | '`' | ',' | ';' | ')'));
    if trimmed.is_empty() || trimmed.contains("://") || is_virtual_device(trimmed) {
        return None;
    }
    if trimmed == "~" || trimmed.starts_with("~/") || trimmed.starts_with("~\\") {
        return Some(Candidate::Home(trimmed.to_string()));
    }
    let normalized_slashes = trimmed.replace('\\', "/");
    let candidate_path = Path::new(&normalized_slashes);
    let joined = if candidate_path.is_absolute()
        || (normalized_slashes.len() >= 3
            && normalized_slashes.as_bytes()[0].is_ascii_alphabetic()
            && normalized_slashes.as_bytes()[1] == b':'
            && normalized_slashes.as_bytes()[2] == b'/')
    {
        candidate_path.to_path_buf()
    } else {
        own_root.join(candidate_path)
    };
    Some(Candidate::Path(normalize_path(&joined)))
}

fn looks_like_path_token(tok: &str) -> bool {
    if tok.is_empty() || tok.contains("://") {
        return false;
    }
    let norm = tok.replace('\\', "/");
    if norm == "~"
        || norm.starts_with("~/")
        || norm.starts_with('/')
        || norm.starts_with("./")
        || norm.starts_with("../")
        || norm == ".."
        || norm.contains("../")
        || norm.ends_with("/..")
        || norm.contains('/')
    {
        return true;
    }
    let bytes = norm.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

fn push_tokens_from_text(text: &str, own_root: &Path, out: &mut Vec<Candidate>) {
    for segment in text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                ';' | '|' | '&' | '(' | ')' | '<' | '>' | ',' | '=' | '\'' | '"' | '`'
            )
    }) {
        let tok = segment.trim();
        if tok.is_empty() || (tok.starts_with('-') && !tok.starts_with("-/")) {
            continue;
        }
        if looks_like_path_token(tok)
            && let Some(candidate) = resolve_raw_candidate(tok, own_root)
        {
            out.push(candidate);
        }
    }
}

fn extract_candidates(ask: &PermissionAsk, own_root: &Path) -> Vec<Candidate> {
    if ask.kind == "question" {
        return vec![];
    }
    let mut out = Vec::new();
    if let Some(diff) = ask.diff_preview.as_deref() {
        for line in diff.lines() {
            let extracted = line
                .strip_prefix("+++ b/")
                .or_else(|| line.strip_prefix("--- a/"))
                .or_else(|| line.strip_prefix("+++ "))
                .or_else(|| line.strip_prefix("--- "))
                .or_else(|| line.strip_prefix("*** Update File: "))
                .or_else(|| line.strip_prefix("*** Add File: "))
                .or_else(|| line.strip_prefix("*** Delete File: "));
            if let Some(rest) = extracted {
                let raw = rest.split('\t').next().unwrap_or("").trim();
                if let Some(candidate) = resolve_raw_candidate(raw, own_root) {
                    out.push(candidate);
                }
            } else if let Some(rest) = line.strip_prefix("diff --git ") {
                for part in rest.split_whitespace() {
                    let trimmed = part
                        .strip_prefix("a/")
                        .or_else(|| part.strip_prefix("b/"))
                        .unwrap_or(part);
                    if let Some(candidate) = resolve_raw_candidate(trimmed, own_root) {
                        out.push(candidate);
                    }
                }
            }
        }
    }

    let summary = ask.summary.trim();
    if (ask.kind == "file_change"
        || matches!(
            ask.tool.as_str(),
            "Edit" | "MultiEdit" | "Write" | "NotebookEdit" | "File change" | "Delete" | "Read"
        ))
        && !summary.is_empty()
        && looks_like_path_token(summary)
        && let Some(candidate) = resolve_raw_candidate(summary, own_root)
    {
        out.push(candidate);
    }
    push_tokens_from_text(summary, own_root, &mut out);
    if let Some(detail) = ask.detail.as_deref() {
        push_tokens_from_text(detail, own_root, &mut out);
    }
    out
}

fn best_owner<'a>(
    norm_target: &Path,
    scope: &'a BoundaryScope,
    system_root: &Path,
    selected_roots: &'a [(String, PathBuf)],
) -> Option<&'a str> {
    let mut best: Option<(&str, usize, bool)> = None;
    let consider =
        |best: &mut Option<(&'a str, usize, bool)>, sid: &'a str, root: &Path, is_system: bool| {
            if !norm_starts_with(norm_target, root) {
                return;
            }
            let depth = root.components().count();
            let replace = match best {
                None => true,
                Some((_, best_depth, best_sys)) => {
                    depth > *best_depth || (depth == *best_depth && *best_sys && !is_system)
                }
            };
            if replace {
                *best = Some((sid, depth, is_system));
            }
        };

    consider(&mut best, "__system__", system_root, true);
    for (sid, root) in selected_roots {
        consider(&mut best, sid.as_str(), root, false);
    }
    if scope.service != "__system__" {
        consider(&mut best, scope.service.as_str(), &scope.own_root, false);
    }
    best.map(|(sid, _, _)| sid)
}

fn check_target_path(
    target: &Path,
    scope: &BoundaryScope,
    enforce_foreign_contract_when_all: bool,
) -> Option<String> {
    let norm_target = normalize_path(target);
    let system_root = normalize_path(&scope.system_root);
    let own_root = normalize_path(&scope.own_root);
    let writable_roots: Vec<PathBuf> = scope
        .writable_roots
        .iter()
        .map(|p| normalize_path(p))
        .collect();
    let selected_roots: Vec<(String, PathBuf)> = scope
        .selected_roots
        .iter()
        .map(|(sid, root)| (sid.clone(), normalize_path(root)))
        .collect();
    let own_scope = BoundaryScope {
        own_root: own_root.clone(),
        ..scope.clone()
    };

    let owner = best_owner(&norm_target, &own_scope, &system_root, &selected_roots);
    match owner {
        Some("__system__") => {
            return Some(format!(
                "targets system root '__system__' ({}) outside service scope '{}'",
                norm_target.display(),
                scope.service
            ));
        }
        Some(sid) if sid != scope.service => {
            if scope.access != "all"
                || !writable_roots
                    .iter()
                    .any(|w| norm_starts_with(&norm_target, w))
            {
                return Some(format!(
                    "targets service '{sid}' ({}) outside writable scope for '{}'",
                    norm_target.display(),
                    scope.service
                ));
            }
        }
        Some(_) => {}
        None => {
            return Some(format!(
                "targets path '{}' outside allowed writable roots",
                norm_target.display()
            ));
        }
    }

    if (scope.access == "service" || enforce_foreign_contract_when_all)
        && let Some((contract_owner, contract_id)) =
            scope.contract_sources.iter().find_map(|(p, owners)| {
                (normalize_path(p) == norm_target)
                    .then(|| {
                        owners
                            .iter()
                            .find(|(owner, _)| owner != &scope.service)
                            .map(|(owner, cid)| (owner.as_str(), cid.as_str()))
                    })
                    .flatten()
            })
    {
        return Some(format!(
            "targets contract '{contract_id}' owned by service '{contract_owner}' ({})",
            norm_target.display()
        ));
    }

    if !writable_roots
        .iter()
        .any(|w| norm_starts_with(&norm_target, w))
    {
        return Some(format!(
            "targets path '{}' outside writable scope for '{}'",
            norm_target.display(),
            scope.service
        ));
    }

    None
}

/// Detects whether a host [`PermissionAsk`] crosses a service, system, or contract ownership boundary.
pub fn detect_boundary_violation(ask: &PermissionAsk, scope: &BoundaryScope) -> Option<String> {
    let own_root = normalize_path(&scope.own_root);
    for candidate in extract_candidates(ask, &own_root) {
        match candidate {
            Candidate::Home(raw) => {
                return Some(format!(
                    "targets user home path '{raw}' outside writable scope for '{}'",
                    scope.service
                ));
            }
            Candidate::Path(path) => {
                if let Some(reason) = check_target_path(&path, scope, true) {
                    return Some(reason);
                }
            }
        }
    }
    None
}

fn prepend_detail_note(existing: Option<&str>, note: &str) -> String {
    match existing.map(str::trim).filter(|s| !s.is_empty()) {
        Some(detail) => format!("{note} {detail}"),
        None => note.to_owned(),
    }
}

fn ready_answer(answer: Answer) -> oneshot::Receiver<Answer> {
    let (tx, rx) = oneshot::channel();
    let _ = tx.send(answer);
    rx
}

/// Wraps a Harness [`Approvals`] channel with cross-service and contract-ownership boundary checks.
pub struct SystemBoundaryApprovals {
    pub inner: Option<Arc<dyn Approvals>>,
    pub mode: Option<PermissionMode>,
    pub scope: BoundaryScope,
}

impl SystemBoundaryApprovals {
    pub fn new(
        inner: Option<Arc<dyn Approvals>>,
        mode: Option<PermissionMode>,
        scope: BoundaryScope,
    ) -> Self {
        Self { inner, mode, scope }
    }
}

impl Approvals for SystemBoundaryApprovals {
    fn ask(&self, ask: &PermissionAsk) -> infra_host::Result<oneshot::Receiver<Decision>> {
        let mut rx = self.ask_answer(ask)?;
        let (mut tx, decision_rx) = oneshot::channel();
        match rx.try_recv() {
            Ok(answer) => {
                let _ = tx.send(answer.decision);
            }
            Err(oneshot::error::TryRecvError::Closed) => {}
            Err(oneshot::error::TryRecvError::Empty) => {
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    handle.spawn(async move {
                        tokio::select! {
                            res = &mut rx => {
                                if let Ok(answer) = res {
                                    let _ = tx.send(answer.decision);
                                }
                            }
                            _ = tx.closed() => {}
                        }
                    });
                } else {
                    std::thread::spawn(move || {
                        if let Ok(answer) = rx.blocking_recv() {
                            let _ = tx.send(answer.decision);
                        }
                    });
                }
            }
        }
        Ok(decision_rx)
    }

    fn ask_answer(&self, ask: &PermissionAsk) -> infra_host::Result<oneshot::Receiver<Answer>> {
        match self.mode {
            Some(PermissionMode::Manual) => {
                let mut forwarded = ask.clone();
                if let Some(reason) = detect_boundary_violation(ask, &self.scope) {
                    forwarded.session_option = false;
                    forwarded.detail = Some(prepend_detail_note(
                        ask.detail.as_deref(),
                        &format!("[Boundary escalation: {reason}]"),
                    ));
                }
                match &self.inner {
                    Some(inner) => inner.ask_answer(&forwarded),
                    None => Err(HarnessError::invalid(
                        "Manual permissions need someone to answer the requests.",
                    )),
                }
            }
            Some(PermissionMode::Auto) => {
                if let Some(reason) = detect_boundary_violation(ask, &self.scope) {
                    let mut escalated = ask.clone();
                    escalated.session_option = false;
                    escalated.detail = Some(prepend_detail_note(
                        ask.detail.as_deref(),
                        &format!("[Auto -> Manual boundary escalation: {reason}]"),
                    ));
                    match &self.inner {
                        Some(inner) => inner.ask_answer(&escalated),
                        None => Ok(ready_answer(Answer::from_decision(Decision::Deny))),
                    }
                } else if ask.kind == "question" {
                    Ok(ready_answer(Answer::from_decision(Decision::Deny)))
                } else {
                    Ok(ready_answer(Answer::from_decision(Decision::Allow)))
                }
            }
            Some(PermissionMode::Bypass) => {
                Ok(ready_answer(Answer::from_decision(Decision::Allow)))
            }
            None => match &self.inner {
                Some(inner) => inner.ask_answer(ask),
                None => Ok(ready_answer(Answer::from_decision(Decision::Deny))),
            },
        }
    }

    fn withdraw(&self, id: &str) -> infra_host::Result<()> {
        match &self.inner {
            Some(inner) => inner.withdraw(id),
            None => Ok(()),
        }
    }
}

fn resolve_written_entry(raw: &str, scope: &BoundaryScope) -> PathBuf {
    let clean = raw.trim().replace('\\', "/");
    if let Some(rest) = clean.strip_prefix("__system__/") {
        return normalize_path(&scope.system_root.join(rest));
    }
    if clean == "__system__" {
        return normalize_path(&scope.system_root);
    }
    let p = Path::new(&clean);
    if p.is_absolute() {
        return normalize_path(p);
    }
    if scope.access == "all"
        && let Some((sid, rest)) = clean.split_once('/')
        && let Some(root) = scope.selected_roots.get(sid)
    {
        return normalize_path(&root.join(rest));
    }
    normalize_path(&scope.own_root.join(p))
}

/// Verifies post-step writes from `git_changed` and `report["changed_files"]` stayed within the step's allowed boundary.
pub fn check_step_boundary_writes(
    scope: &BoundaryScope,
    git_changed: &[EvidenceFileChange],
    reported_changed_files: &[String],
) -> Result<()> {
    check_step_boundary_writes_with_extra(scope, git_changed, reported_changed_files, &[])
}

/// Verifies post-step writes including any additional observed paths (e.g. dirty worktree files in `__system__` or foreign services).
pub fn check_step_boundary_writes_with_extra(
    scope: &BoundaryScope,
    git_changed: &[EvidenceFileChange],
    reported_changed_files: &[String],
    extra_written_paths: &[PathBuf],
) -> Result<()> {
    let mut paths = Vec::new();
    for ch in git_changed {
        paths.push(resolve_written_entry(&ch.path, scope));
    }
    for entry in reported_changed_files {
        paths.push(resolve_written_entry(entry, scope));
    }
    for extra in extra_written_paths {
        paths.push(normalize_path(extra));
    }

    for path in paths {
        if let Some(reason) = check_target_path(&path, scope, false) {
            return Err(fail(format!(
                "Step boundary violation: non-interactive worker {reason}. Re-run with Manual permissions for explicit approval or use access = \"all\" for cross-service edits."
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_scope(access: &str) -> BoundaryScope {
        let own = PathBuf::from("/work/orders");
        let billing = PathBuf::from("/work/billing");
        let sys = PathBuf::from("/work/sys");
        let mut writable_roots = vec![own.clone()];
        if access == "all" {
            writable_roots.push(billing.clone());
        }
        BoundaryScope {
            service: "orders".into(),
            mode: "edit".into(),
            access: access.into(),
            own_root: own.clone(),
            system_root: sys,
            selected_roots: BTreeMap::from([("orders".into(), own), ("billing".into(), billing)]),
            writable_roots,
            contract_sources: BTreeMap::from([
                (
                    PathBuf::from("/work/orders/src/order.php"),
                    vec![("orders".into(), "order.placed".into())],
                ),
                (
                    PathBuf::from("/work/billing/src/api.php"),
                    vec![("billing".into(), "billing.api".into())],
                ),
            ]),
        }
    }

    #[test]
    fn detect_boundary_violation_catches_system_foreign_and_contract_crossings() {
        let scope = sample_scope("service");

        // Own service file (even its own contract source) is safe.
        let own_ask = PermissionAsk {
            provider: "claude".into(),
            tool: "Edit".into(),
            kind: "file_change".into(),
            summary: "src/order.php".into(),
            diff_preview: Some(
                "--- a/src/order.php\n+++ b/src/order.php\n@@ -1 +1 @@\n-a\n+b".into(),
            ),
            ..Default::default()
        };
        assert_eq!(detect_boundary_violation(&own_ask, &scope), None);

        // New file write with `--- /dev/null` in own root must not false-positive.
        let new_own = PermissionAsk {
            provider: "claude".into(),
            tool: "Write".into(),
            kind: "file_change".into(),
            summary: "src/new_helper.php".into(),
            diff_preview: Some(
                "--- /dev/null\n+++ b/src/new_helper.php\n@@ -0,0 +1 @@\n+<?php".into(),
            ),
            ..Default::default()
        };
        assert_eq!(detect_boundary_violation(&new_own, &scope), None);

        // System root write is always a violation.
        let sys_ask = PermissionAsk {
            provider: "claude".into(),
            tool: "Edit".into(),
            kind: "file_change".into(),
            summary: "../sys/AGENTS.md".into(),
            ..Default::default()
        };
        assert!(
            detect_boundary_violation(&sys_ask, &scope)
                .unwrap()
                .contains("__system__")
        );

        // Foreign service edit under `access == "service"` is caught from diff_preview or command.
        let foreign_diff = PermissionAsk {
            provider: "codex".into(),
            tool: "File change".into(),
            kind: "file_change".into(),
            summary: "apply a change to files".into(),
            diff_preview: Some(
                "--- a/../billing/src/consumer.php\n+++ b/../billing/src/consumer.php\n@@".into(),
            ),
            ..Default::default()
        };
        assert!(
            detect_boundary_violation(&foreign_diff, &scope)
                .unwrap()
                .contains("billing")
        );

        let foreign_cmd = PermissionAsk {
            provider: "claude".into(),
            tool: "Bash".into(),
            kind: "command".into(),
            summary: "sed -i 's/a/b/' /work/billing/src/consumer.php".into(),
            ..Default::default()
        };
        assert!(
            detect_boundary_violation(&foreign_cmd, &scope)
                .unwrap()
                .contains("billing")
        );

        // Under `access == "all"`, non-contract file in `billing` is allowed,
        // while `billing`'s declared contract source still triggers boundary escalation!
        let all_scope = sample_scope("all");
        let billing_internal = PermissionAsk {
            provider: "claude".into(),
            tool: "Edit".into(),
            kind: "file_change".into(),
            summary: "/work/billing/src/internal.php".into(),
            ..Default::default()
        };
        assert_eq!(
            detect_boundary_violation(&billing_internal, &all_scope),
            None
        );

        let billing_contract = PermissionAsk {
            provider: "claude".into(),
            tool: "Edit".into(),
            kind: "file_change".into(),
            summary: "../billing/src/api.php".into(),
            ..Default::default()
        };
        assert!(
            detect_boundary_violation(&billing_contract, &all_scope)
                .unwrap()
                .contains("billing.api")
        );
    }

    #[test]
    fn monorepo_topology_allows_own_service_and_flags_system_root_and_sibling_services() {
        let sys_root = PathBuf::from("/repo");
        let orders_root = PathBuf::from("/repo/services/orders");
        let billing_root = PathBuf::from("/repo/services/billing");
        let scope = BoundaryScope {
            service: "orders".into(),
            mode: "edit".into(),
            access: "service".into(),
            own_root: orders_root.clone(),
            system_root: sys_root,
            selected_roots: BTreeMap::from([
                ("orders".into(), orders_root),
                ("billing".into(), billing_root),
            ]),
            writable_roots: vec![PathBuf::from("/repo/services/orders")],
            contract_sources: BTreeMap::new(),
        };

        let own = PermissionAsk {
            tool: "Edit".into(),
            kind: "file_change".into(),
            summary: "src/Order.php".into(),
            ..Default::default()
        };
        assert_eq!(detect_boundary_violation(&own, &scope), None);

        let sys_cfg = PermissionAsk {
            tool: "Edit".into(),
            kind: "file_change".into(),
            summary: "../../system.json".into(),
            ..Default::default()
        };
        assert!(
            detect_boundary_violation(&sys_cfg, &scope)
                .unwrap()
                .contains("__system__")
        );

        let sibling = PermissionAsk {
            tool: "Edit".into(),
            kind: "file_change".into(),
            summary: "../billing/src/Billing.php".into(),
            ..Default::default()
        };
        assert!(
            detect_boundary_violation(&sibling, &scope)
                .unwrap()
                .contains("billing")
        );
    }
}
