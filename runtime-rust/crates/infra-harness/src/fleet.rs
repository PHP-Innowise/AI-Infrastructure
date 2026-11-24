use crate::{
    HarnessError, Result, legacy,
    process::Limits,
    provider::{self, Launch, WorkerResult},
    store::{self, SessionStore},
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, atomic::AtomicBool},
};
const MAX_FLEET_PREFIX_MEMO: usize = 128;

#[derive(Clone, Debug)]
struct CachedFleetPrefix {
    prefix: String,
    prefix_sha256: String,
    capsule_sha256: String,
}

#[derive(Default)]
struct FleetPrefixMemoStore {
    entries: HashMap<String, CachedFleetPrefix>,
    order: VecDeque<String>,
}

impl FleetPrefixMemoStore {
    fn get(&self, key: &str) -> Option<&CachedFleetPrefix> {
        self.entries.get(key)
    }

    fn insert(&mut self, key: String, entry: CachedFleetPrefix) {
        if !self.entries.contains_key(&key) {
            if self.entries.len() >= MAX_FLEET_PREFIX_MEMO
                && let Some(evicted) = self.order.pop_front()
            {
                self.entries.remove(&evicted);
            }
            self.order.push_back(key.clone());
        }
        self.entries.insert(key, entry);
    }
}

fn fleet_prefix_memo() -> &'static Mutex<FleetPrefixMemoStore> {
    static MEMO: OnceLock<Mutex<FleetPrefixMemoStore>> = OnceLock::new();
    MEMO.get_or_init(|| Mutex::new(FleetPrefixMemoStore::default()))
}

fn prepare_fleet_reviewer_prefix(
    thread: &str,
    lens: &str,
    capsule: &str,
) -> (String, String, bool) {
    let prefix = format!("Act as {lens}. Return only JSON findings.\n\n{capsule}");
    let capsule_sha = store::sha(capsule.as_bytes());
    let prefix_sha = store::sha(prefix.as_bytes());
    let key = format!("{thread}\0{lens}");
    if let Ok(mut memo) = fleet_prefix_memo().lock() {
        if let Some(cached) = memo.get(&key)
            && cached.capsule_sha256 == capsule_sha
        {
            return (cached.prefix.clone(), cached.prefix_sha256.clone(), true);
        }
        memo.insert(
            key,
            CachedFleetPrefix {
                prefix: prefix.clone(),
                prefix_sha256: prefix_sha.clone(),
                capsule_sha256: capsule_sha,
            },
        );
    }
    (prefix, prefix_sha, false)
}

fn build_fleet_repair_prompt(
    thread: &str,
    lens: &str,
    capsule: &str,
    err: &str,
    snippet: &str,
) -> (String, String, bool) {
    let key = format!("{thread}\0{lens}");
    let capsule_sha = store::sha(capsule.as_bytes());
    let (prefix, prefix_sha, hit) = if let Ok(memo) = fleet_prefix_memo().lock()
        && let Some(cached) = memo.get(&key)
        && cached.capsule_sha256 == capsule_sha
    {
        (cached.prefix.clone(), cached.prefix_sha256.clone(), true)
    } else {
        let (p, sha, _) = prepare_fleet_reviewer_prefix(thread, lens, capsule);
        (p, sha, false)
    };
    let prompt = format!(
        "{prefix}\n\nYour previous response failed strict JSON schema validation ({err}).\nReturn ONLY a single JSON object `{{\"findings\":[{{\"file\":\"...\",\"line\":1,\"severity\":\"high|medium|low\",\"claim\":\"...\",\"evidence\":\"...\"}}]}}`.\nPrevious output:\n{snippet}"
    );
    (prompt, prefix_sha, hit)
}

#[derive(Clone)]
pub struct Config {
    pub project: PathBuf,
    pub provider: String,
    pub executable: PathBuf,
    pub lenses: Vec<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub budget: Option<f64>,
    pub seconds: u64,
    pub concurrency: usize,
    pub dry_run: bool,
    pub reports: PathBuf,
    pub adapter: Option<Arc<dyn provider::LaunchAdapter>>,
    pub runner_lock: Option<Arc<std::fs::File>>,
}
pub trait Blackboard: Send + Sync {
    fn available(&self) -> bool;
    fn ensure_task(&self, task: &str, title: &str) -> Result<()>;
    fn dispatch_spawn(&self, task: &str, lens: &str, capsule: &str) -> Result<()>;
    fn dispatch_complete(&self, task: &str, lens: &str, message: &str) -> Result<()>;
    fn progress(&self, task: &str, message: &str) -> Result<()>;
}

#[derive(Default)]
struct LensRoiStats {
    runs: usize,
    skipped_runs: usize,
    total_cost_usd: f64,
    total_findings: usize,
    accepted_findings: usize,
}

struct FleetHealthReport {
    effective_concurrency: usize,
    warnings: Vec<String>,
    lens_roi: Vec<Value>,
}

fn round_usd_metric(value: f64) -> f64 {
    ((value * 1_000_000_000.0).round()) / 1_000_000_000.0
}

fn resolve_fleet_project_identity(
    store: &SessionStore,
    root_canon: &Path,
    thread: Option<&str>,
    state: &Value,
) -> (String, String) {
    let project_path = root_canon.to_string_lossy().into_owned();
    let mut project_id = state["project_id"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("")
        .to_owned();
    if project_id.is_empty()
        && let Some(tid) = thread.map(str::trim).filter(|s| !s.is_empty())
        && let Ok(session) = store.get(tid)
        && let Some(pid) = session["project_id"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    {
        project_id = pid.to_owned();
    }
    if project_id.is_empty()
        && let Ok(db) = store.connection()
        && let Ok(Some(pid)) = db
            .query_row(
                "SELECT id FROM registered_projects WHERE path = ?1 LIMIT 1",
                [project_path.as_str()],
                |r| r.get::<_, String>(0),
            )
            .optional()
    {
        project_id = pid;
    }
    (project_id, project_path)
}

fn collect_fleet_lens_roi(
    store: &SessionStore,
    current_thread: Option<&str>,
    state: &Value,
    c: &Config,
) -> (Vec<Value>, Vec<String>) {
    let mut per_lens: BTreeMap<String, LensRoiStats> = BTreeMap::new();
    for lens in &c.lenses {
        per_lens.entry(lens.clone()).or_default();
    }
    let root_canon = infra_core::fs::canonicalize(&c.project).unwrap_or_else(|_| c.project.clone());
    let (proj_id, proj_path) =
        resolve_fleet_project_identity(store, &root_canon, current_thread, state);
    let current_thread = current_thread.unwrap_or("");
    let mut current_counted = false;
    if let Ok(db) = store.connection()
        && let Ok(mut stmt) = db.prepare(
            "SELECT t.thread_id, t.lens, t.status, t.output, r.state,
                    COALESCE(r.project_id, ''), COALESCE(r.project_path, ''),
                    COALESCE(s.project_id, ''), COALESCE(s.project_path, '')
             FROM native_fleet_tasks t
             JOIN native_fleet_runs r ON r.thread_id = t.thread_id
             LEFT JOIN sessions s ON s.id = t.thread_id
             ORDER BY r.rowid DESC, t.ordinal ASC
             LIMIT 500",
        )
        && let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
            ))
        })
    {
        for row in rows.flatten() {
            let (
                row_thread,
                lens,
                status,
                output_raw,
                run_state_raw,
                r_pid,
                r_ppath,
                s_pid,
                s_ppath,
            ) = row;
            let run_state = serde_json::from_str::<Value>(&run_state_raw).unwrap_or(Value::Null);
            let row_pid = if !r_pid.is_empty() {
                r_pid.as_str()
            } else if !s_pid.is_empty() {
                s_pid.as_str()
            } else {
                run_state["project_id"].as_str().unwrap_or("")
            };
            let row_ppath = if !r_ppath.is_empty() {
                r_ppath.as_str()
            } else if !s_ppath.is_empty() {
                s_ppath.as_str()
            } else {
                run_state["project_path"].as_str().unwrap_or("")
            };
            let same_thread = !current_thread.is_empty() && row_thread == current_thread;
            let same_project_id = !proj_id.is_empty() && !row_pid.is_empty() && row_pid == proj_id;
            let same_project_path =
                !proj_path.is_empty() && !row_ppath.is_empty() && row_ppath == proj_path;
            if !(same_thread || same_project_id || same_project_path) {
                continue;
            }
            if same_thread {
                current_counted = true;
            }
            let entry = per_lens.entry(lens).or_default();
            let output_val = output_raw
                .as_deref()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or(Value::Null);
            let approved = run_state["approved"] == true;
            let was_skipped = output_val
                .get("skipped")
                .and_then(Value::as_array)
                .is_some_and(|arr| !arr.is_empty());
            if was_skipped {
                entry.skipped_runs += 1;
            } else if matches!(status.as_str(), "completed" | "schema_error" | "failed") {
                entry.runs += 1;
            }
            if let Some(spent) = cost(&output_val["cost_usd"]) {
                entry.total_cost_usd += spent;
            }
            let findings_n = output_val["findings"]
                .as_array()
                .map(|a| a.len())
                .unwrap_or(0);
            entry.total_findings += findings_n;
            if approved {
                entry.accepted_findings += findings_n;
            }
        }
    }
    if !current_counted {
        let approved = state["approved"] == true;
        let skipped_set: BTreeSet<&str> = state["skipped"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        for skipped_lens in &skipped_set {
            per_lens
                .entry((*skipped_lens).to_owned())
                .or_default()
                .skipped_runs += 1;
        }
        for finding in state["findings"].as_array().into_iter().flatten() {
            if let Some(lens) = finding["lens"].as_str().filter(|l| !l.is_empty()) {
                let entry = per_lens.entry(lens.to_owned()).or_default();
                entry.total_findings += 1;
                if approved {
                    entry.accepted_findings += 1;
                }
            }
        }
    }
    let mut warnings = Vec::new();
    let mut out = Vec::with_capacity(per_lens.len());
    for (lens, stats) in per_lens {
        let spent = round_usd_metric(stats.total_cost_usd);
        let findings_per_usd = if spent > 0.0 {
            Some(round_usd_metric(stats.total_findings as f64 / spent))
        } else {
            None
        };
        let accepted_per_usd = if spent > 0.0 {
            Some(round_usd_metric(stats.accepted_findings as f64 / spent))
        } else {
            None
        };
        let cost_per_accepted = if stats.accepted_findings > 0 {
            Some(round_usd_metric(spent / stats.accepted_findings as f64))
        } else {
            None
        };
        if stats.runs >= 3 && spent >= 0.05 && stats.accepted_findings == 0 {
            warnings.push(format!(
                "Low ROI warning for Fleet lens `{lens}`: ${spent:.4} spent across {} runs with 0 accepted findings.",
                stats.runs
            ));
        }
        out.push(json!({
            "lens": lens,
            "runs": stats.runs,
            "skipped_runs": stats.skipped_runs,
            "total_cost_usd": spent,
            "total_findings": stats.total_findings,
            "accepted_findings": stats.accepted_findings,
            "findings_per_usd": findings_per_usd,
            "accepted_findings_per_usd": accepted_per_usd,
            "cost_per_accepted_finding": cost_per_accepted,
        }));
    }
    (out, warnings)
}

fn collect_fleet_health(
    store: &SessionStore,
    current_thread: Option<&str>,
    state: &Value,
    c: &Config,
) -> FleetHealthReport {
    let summary =
        crate::clash::summarize_workflow_health(store, "fleet", &c.provider, c.model.as_deref());
    let requested = c.concurrency.max(1);
    let effective = summary.recommended_fleet_concurrency(requested);
    let mut warnings = Vec::new();
    for existing in state["health_warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if !warnings.iter().any(|w| w == existing) {
            warnings.push(existing.to_owned());
        }
    }
    for w in &summary.warnings {
        if !warnings.contains(w) {
            warnings.push(w.clone());
        }
    }
    if effective < requested {
        let msg = format!(
            "Throttled Fleet concurrency pre-launch from {requested} to {effective} based on smoothed workflow health."
        );
        if !warnings.contains(&msg) {
            warnings.push(msg);
        }
    }
    let (lens_roi, roi_warnings) = collect_fleet_lens_roi(store, current_thread, state, c);
    for w in roi_warnings {
        if !warnings.contains(&w) {
            warnings.push(w);
        }
    }
    FleetHealthReport {
        effective_concurrency: effective,
        warnings,
        lens_roi,
    }
}

struct FleetScopePlan {
    evaluated: bool,
    ir_revision: Option<u64>,
    diff_files: Vec<String>,
    caller_files: Vec<String>,
    fallback: bool,
    fallback_reason: Option<String>,
    lens_globs: BTreeMap<String, Vec<String>>,
}

fn parse_frontmatter_watched_globs(path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Vec::new();
    }
    let mut globs = Vec::new();
    let mut collecting_list = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if collecting_list {
            if let Some(item) = trimmed.strip_prefix('-') {
                let g = item
                    .trim()
                    .trim_matches(|c| c == '"' || c == '\'')
                    .trim_start_matches("./")
                    .replace('\\', "/");
                if !g.is_empty() {
                    globs.push(g);
                }
                continue;
            }
            if trimmed.is_empty() {
                continue;
            }
            collecting_list = false;
        }
        for key in ["watched_globs:", "paths:", "globs:"] {
            if let Some(rest) = trimmed.strip_prefix(key) {
                let rest = rest.trim();
                if rest.starts_with('[') && rest.ends_with(']') {
                    let inner = &rest[1..rest.len() - 1];
                    for part in inner.split(',') {
                        let g = part
                            .trim()
                            .trim_matches(|c| c == '"' || c == '\'')
                            .trim_start_matches("./")
                            .replace('\\', "/");
                        if !g.is_empty() {
                            globs.push(g);
                        }
                    }
                } else if rest.is_empty() {
                    collecting_list = true;
                } else {
                    let g = rest
                        .trim_matches(|c| c == '"' || c == '\'')
                        .trim_start_matches("./")
                        .replace('\\', "/");
                    if !g.is_empty() {
                        globs.push(g);
                    }
                }
            }
        }
    }
    globs
}

fn resolve_lens_watched_globs(
    store: &SessionStore,
    root: &Path,
    project_id: Option<&str>,
    state: &Value,
    lenses: &[String],
) -> BTreeMap<String, Vec<String>> {
    let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    if let Some(obj) = state.get("lens_scopes").and_then(Value::as_object) {
        for (lens, arr) in obj {
            if let Some(items) = arr.as_array() {
                let set = map.entry(lens.clone()).or_default();
                for g in items.iter().filter_map(Value::as_str) {
                    let clean = crate::skillcache::db::normalize_rel_path(g);
                    if !clean.is_empty() {
                        set.insert(clean);
                    }
                }
            }
        }
    }
    let fleet_cfg_path = root.join(".harness-fleet.json");
    if let Ok(meta) = std::fs::symlink_metadata(&fleet_cfg_path)
        && meta.is_file()
        && meta.len() <= 64 * 1024
        && let Ok(bytes) = std::fs::read(&fleet_cfg_path)
        && let Ok(parsed) = serde_json::from_slice::<Value>(&bytes)
    {
        let lens_map = parsed
            .get("lenses")
            .and_then(Value::as_object)
            .or_else(|| parsed.as_object());
        if let Some(obj) = lens_map {
            for lens in lenses {
                if let Some(entry) = obj.get(lens) {
                    let arr = entry
                        .get("watched_globs")
                        .or_else(|| entry.get("paths"))
                        .and_then(Value::as_array)
                        .or_else(|| entry.as_array());
                    if let Some(items) = arr {
                        let set = map.entry(lens.clone()).or_default();
                        for g in items.iter().filter_map(Value::as_str) {
                            let clean = crate::skillcache::db::normalize_rel_path(g);
                            if !clean.is_empty() {
                                set.insert(clean);
                            }
                        }
                    }
                }
            }
        }
    }
    for lens in lenses {
        if map.get(lens).is_some_and(|s| !s.is_empty()) {
            continue;
        }
        let candidates = [
            root.join(".claude/agents").join(format!("{lens}.md")),
            root.join(".claude/skills").join(lens).join("SKILL.md"),
            root.join(".cursor/agents").join(format!("{lens}.md")),
            root.join(".cursor/skills").join(lens).join("SKILL.md"),
            root.join(".cursor/rules").join(format!("{lens}.mdc")),
            root.join(".agents/skills").join(lens).join("SKILL.md"),
        ];
        for cand in candidates {
            let parsed = parse_frontmatter_watched_globs(&cand);
            if !parsed.is_empty() {
                let set = map.entry(lens.clone()).or_default();
                for g in parsed {
                    let clean = crate::skillcache::db::normalize_rel_path(&g);
                    if !clean.is_empty() {
                        set.insert(clean);
                    }
                }
                break;
            }
        }
    }
    if let Some(pid) = project_id.map(str::trim).filter(|s| !s.is_empty())
        && let Ok(db) = store.connection()
    {
        for lens in lenses {
            if map.get(lens).is_some_and(|s| !s.is_empty()) {
                continue;
            }
            if let Ok(Some(raw_manifest)) = db
                .query_row(
                    "SELECT dependency_manifest FROM skillcache_capsules
                     WHERE (mu_name = ?1 OR skill_name = ?1)
                       AND project_id = ?2
                       AND status = 'active'
                     ORDER BY rowid DESC LIMIT 1",
                    params![lens, pid],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                && let Ok(val) = serde_json::from_str::<Value>(&raw_manifest)
            {
                let manifest = crate::skillcache::db::ArtifactDependencyManifest::from_value(&val);
                if !manifest.watched_globs.is_empty() {
                    let set = map.entry(lens.clone()).or_default();
                    for g in manifest.watched_globs {
                        let clean = crate::skillcache::db::normalize_rel_path(&g);
                        if !clean.is_empty() {
                            set.insert(clean);
                        }
                    }
                }
            }
        }
    }
    map.into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect()
}

fn git_rev_oid(root: &Path, rev: &str) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", rev])
        .current_dir(root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let oid = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!oid.is_empty()).then_some(oid)
}

fn resolve_committed_diff_base(root: &Path, base_hint: Option<&str>) -> Option<String> {
    let head = git_rev_oid(root, "HEAD")?;
    let mut candidates: Vec<&str> = Vec::new();
    if let Some(hint) = base_hint.map(str::trim).filter(|s| !s.is_empty()) {
        candidates.push(hint);
    }
    candidates.extend([
        "@{upstream}",
        "origin/main",
        "origin/master",
        "main",
        "master",
    ]);
    for cand in candidates {
        if git_rev_oid(root, cand).is_none() {
            continue;
        }
        let Ok(out) = std::process::Command::new("git")
            .args(["merge-base", "HEAD", cand])
            .current_dir(root)
            .output()
        else {
            continue;
        };
        if out.status.success() {
            let mb = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if !mb.is_empty() && mb != head {
                return Some(mb);
            }
        }
    }
    git_rev_oid(root, "HEAD~1")
}

fn collect_git_diff_files(
    root: &Path,
    base_hint: Option<&str>,
) -> std::result::Result<Vec<String>, &'static str> {
    if !root.join(".git").exists() {
        return Err("no_git_repository");
    }
    let status_out = std::process::Command::new("git")
        .args(["status", "--porcelain=v1", "--untracked-files=all", "-z"])
        .current_dir(root)
        .output()
        .map_err(|_| "git_status_failed")?;
    if !status_out.status.success() {
        return Err("git_status_nonzero");
    }
    let mut files = BTreeSet::new();
    let mut entries = status_out.stdout.split(|b| *b == 0);
    while let Some(raw_entry) = entries.next() {
        if raw_entry.len() < 4 {
            continue;
        }
        let status = &raw_entry[..2];
        let path =
            crate::skillcache::db::normalize_rel_path(&String::from_utf8_lossy(&raw_entry[3..]));
        if !path.is_empty() {
            files.insert(path);
        }
        if (status.starts_with(b"R") || status.starts_with(b"C"))
            && let Some(orig) = entries.next()
        {
            let orig_path =
                crate::skillcache::db::normalize_rel_path(&String::from_utf8_lossy(orig));
            if !orig_path.is_empty() {
                files.insert(orig_path);
            }
        }
    }
    if let Some(diff_base) = resolve_committed_diff_base(root, base_hint) {
        let range = format!("{diff_base}..HEAD");
        let diff_out = std::process::Command::new("git")
            .args(["diff", "--name-only", "-z", &range])
            .current_dir(root)
            .output()
            .map_err(|_| "git_diff_failed")?;
        if !diff_out.status.success() {
            return Err("git_diff_nonzero");
        }
        for raw_path in diff_out.stdout.split(|b| *b == 0) {
            if raw_path.is_empty() {
                continue;
            }
            let path =
                crate::skillcache::db::normalize_rel_path(&String::from_utf8_lossy(raw_path));
            if !path.is_empty() {
                files.insert(path);
            }
        }
    }
    if files.is_empty() {
        return Err("empty_git_diff");
    }
    Ok(files.into_iter().collect())
}

fn is_global_manifest_change(rel: &str) -> bool {
    matches!(
        rel,
        "composer.json"
            | "composer.lock"
            | "package.json"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "Cargo.toml"
            | "Cargo.lock"
            | ".harness-architecture.json"
            | ".harness-fleet.json"
    ) || rel.starts_with(".claude/")
        || rel.starts_with(".cursor/")
        || rel.starts_with(".agents/")
}

fn meta_mtime_ns(meta: &std::fs::Metadata) -> i64 {
    let nanos = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0u128, |d| d.as_nanos());
    i64::try_from(nanos).unwrap_or(i64::MAX)
}

fn verify_graph_ir_fresh_for_scope(
    root: &Path,
    ir: &crate::graph_ir::GraphIrStore,
    diff_files: &[String],
) -> Option<&'static str> {
    let Ok((stamps, manifest_stamp, ir_git_head)) = ir.file_stamps_and_meta() else {
        return Some("graph_ir_meta_unreadable");
    };
    let current_manifest = crate::graph_ir::compute_root_manifest_stamp(root);
    if manifest_stamp.as_deref() != Some(current_manifest.as_str()) {
        return Some("graph_ir_manifest_stale");
    }
    if let Some(current_head) = crate::graph_ir::current_git_head(root)
        && ir_git_head.as_deref() != Some(current_head.as_str())
    {
        return Some("graph_ir_head_stale");
    }
    let mut checked = BTreeSet::new();
    for rel in diff_files {
        checked.insert(rel.as_str());
        let disk_meta = std::fs::symlink_metadata(root.join(rel)).ok();
        if let Some(&(ir_size, ir_mtime_ns)) = stamps.get(rel) {
            let Some(meta) = disk_meta else {
                return Some("graph_ir_stale_for_diff");
            };
            if meta.file_type().is_symlink() || !meta.is_file() {
                return Some("graph_ir_stale_for_diff");
            }
            let disk_size = i64::try_from(meta.len()).unwrap_or(i64::MAX);
            if (disk_size, meta_mtime_ns(&meta)) != (ir_size, ir_mtime_ns) {
                return Some("graph_ir_stale_for_diff");
            }
        } else if crate::graph_ast::AstGrammar::from_rel_path(rel).is_some()
            && disk_meta.is_some_and(|m| !m.file_type().is_symlink() && m.is_file())
        {
            return Some("graph_ir_unindexed_diff_file");
        }
    }
    for (path, &(ir_size, ir_mtime_ns)) in &stamps {
        if checked.contains(path.as_str()) {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(root.join(path)) else {
            return Some("graph_ir_stale_on_disk");
        };
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Some("graph_ir_stale_on_disk");
        }
        let disk_size = i64::try_from(meta.len()).unwrap_or(i64::MAX);
        if (disk_size, meta_mtime_ns(&meta)) != (ir_size, ir_mtime_ns) {
            return Some("graph_ir_stale_on_disk");
        }
    }
    None
}

fn evaluate_fleet_lens_scope(
    store: &SessionStore,
    thread: &str,
    state: &Value,
    c: &Config,
) -> FleetScopePlan {
    let root_canon = infra_core::fs::canonicalize(&c.project).unwrap_or_else(|_| c.project.clone());
    let (project_id, _) = resolve_fleet_project_identity(store, &root_canon, Some(thread), state);
    let pid_opt = (!project_id.is_empty()).then_some(project_id.as_str());
    let lens_globs = resolve_lens_watched_globs(store, &root_canon, pid_opt, state, &c.lenses);
    let session_base_head = store
        .get(thread)
        .ok()
        .and_then(|s| s["result_base"]["head"].as_str().map(str::to_owned))
        .filter(|s| !s.is_empty());
    let base_hint = state["base_head"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(session_base_head.as_deref());
    let diff_files = match collect_git_diff_files(&root_canon, base_hint) {
        Ok(files) => files,
        Err(reason) => {
            return FleetScopePlan {
                evaluated: true,
                ir_revision: None,
                diff_files: Vec::new(),
                caller_files: Vec::new(),
                fallback: true,
                fallback_reason: Some(reason.to_owned()),
                lens_globs,
            };
        }
    };
    if diff_files.iter().any(|f| is_global_manifest_change(f)) {
        return FleetScopePlan {
            evaluated: true,
            ir_revision: crate::clash::resolve_graph_ir_for_workspace(Some(store), &root_canon)
                .map(|ir| ir.current_write_epoch()),
            diff_files,
            caller_files: Vec::new(),
            fallback: true,
            fallback_reason: Some("manifest_or_lens_config_changed".into()),
            lens_globs,
        };
    }
    let Some(ir) = crate::clash::resolve_graph_ir_for_workspace(Some(store), &root_canon) else {
        return FleetScopePlan {
            evaluated: true,
            ir_revision: None,
            diff_files,
            caller_files: Vec::new(),
            fallback: true,
            fallback_reason: Some("graph_ir_unavailable".into()),
            lens_globs,
        };
    };
    let ir_revision = Some(ir.current_write_epoch());
    let ir_files = ir
        .summary_counts()
        .ok()
        .flatten()
        .map(|s| s.files)
        .unwrap_or(0);
    if ir_files == 0 {
        return FleetScopePlan {
            evaluated: true,
            ir_revision,
            diff_files,
            caller_files: Vec::new(),
            fallback: true,
            fallback_reason: Some("graph_ir_empty".into()),
            lens_globs,
        };
    }
    if let Some(stale_reason) = verify_graph_ir_fresh_for_scope(&root_canon, &ir, &diff_files) {
        return FleetScopePlan {
            evaluated: true,
            ir_revision,
            diff_files,
            caller_files: Vec::new(),
            fallback: true,
            fallback_reason: Some(stale_reason.to_owned()),
            lens_globs,
        };
    }
    let caller_files = crate::graph_impact::resolve_incoming_caller_files(&ir, &diff_files, &[]);
    FleetScopePlan {
        evaluated: true,
        ir_revision,
        diff_files,
        caller_files,
        fallback: false,
        fallback_reason: None,
        lens_globs,
    }
}

impl FleetScopePlan {
    fn clean_receipt_if_disjoint(&self, lens: &str) -> Option<Value> {
        if self.fallback {
            return None;
        }
        let globs = self.lens_globs.get(lens).filter(|g| !g.is_empty())?;
        let impacted_overlaps =
            self.diff_files
                .iter()
                .chain(self.caller_files.iter())
                .any(|path| {
                    globs
                        .iter()
                        .any(|g| crate::skillcache::db::glob_matches(g, path))
                });
        if impacted_overlaps {
            return None;
        }
        Some(json!({
            "kind": "fleet_lens_clean_receipt",
            "lens": lens,
            "status": "clean_disjoint",
            "ir_revision": self.ir_revision,
            "watched_globs": globs,
            "diff_files": self.diff_files,
            "caller_files": self.caller_files,
        }))
    }
}
#[derive(Clone)]
pub struct Fleet {
    pub store: SessionStore,
    pub blackboard: Arc<dyn Blackboard>,
}
fn cost(v: &Value) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite() && *n >= 0.)
}
fn reviewer_event_status(ok: bool, schema_error: Option<&str>) -> &'static str {
    if !ok {
        "failed"
    } else if schema_error.is_some() {
        "schema_error"
    } else {
        "completed"
    }
}

/// Strictly validates Fleet JSON findings (`{"findings": [...]}`) without creating
/// heuristic `"severity": "unparsed"` placeholder items.
pub fn parse_findings_strict(text: &str, lens: &str) -> std::result::Result<Vec<Value>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("empty reviewer response".into());
    }
    let root = if let Ok(v) = serde_json::from_str::<Value>(trimmed)
        && v.is_object()
    {
        v
    } else if let Some(fenced) = crate::clash::extract_json_fence(trimmed)
        && let Ok(v) = serde_json::from_str::<Value>(fenced)
        && v.is_object()
    {
        v
    } else {
        return Err("reviewer output is not a valid JSON object or fenced ```json block".into());
    };
    let Some(items) = root.get("findings").and_then(Value::as_array) else {
        return Err("missing `findings` array in JSON output".into());
    };
    let mut validated = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let Some(obj) = item.as_object() else {
            return Err(format!("`findings[{idx}]` must be an object"));
        };
        let Some(claim) = obj
            .get("claim")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return Err(format!(
                "`findings[{idx}].claim` must be a non-empty string"
            ));
        };
        let sev = obj
            .get("severity")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_ascii_lowercase())
            .unwrap_or_default();
        if !["critical", "high", "medium", "low", "info"].contains(&sev.as_str()) {
            return Err(format!(
                "`findings[{idx}].severity` must be critical|high|medium|low|info (got `{sev}`)"
            ));
        }
        let mut entry = Value::Object(obj.clone());
        entry["lens"] = lens.into();
        entry["severity"] = sev.into();
        entry["claim"] = claim.chars().take(2000).collect::<String>().into();
        validated.push(entry);
    }
    Ok(validated)
}

pub fn parse_findings(text: &str, lens: &str) -> Vec<Value> {
    parse_findings_strict(text, lens).unwrap_or_default()
}

pub fn dedupe(items: &[Value]) -> Vec<Value> {
    let mut indexed = items.iter().enumerate().collect::<Vec<_>>();
    indexed.sort_by_key(|(i, v)| {
        (
            match v["severity"].as_str() {
                Some("critical") => 0,
                Some("high") => 1,
                Some("medium") => 2,
                Some("low") => 3,
                Some("info") => 4,
                _ => 5,
            },
            *i,
        )
    });
    let mut seen = BTreeSet::new();
    indexed
        .into_iter()
        .filter_map(|(_, v)| {
            let key = (
                v["file"].as_str().unwrap_or("").to_string(),
                v["claim"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(120)
                    .collect::<String>()
                    .to_lowercase(),
            );
            seen.insert(key).then(|| v.clone())
        })
        .collect()
}
pub fn report(state: &Value) -> String {
    let lenses = state["lenses"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    let label = cost(&state["cost_usd"])
        .map(|v| format!("{}{v:.2}", char::from(36)))
        .unwrap_or("unknown".into());
    let mut out = format!(
        "# Fleet review — {}\n\nScope: {}\nTask: {}; lenses: {}; cost: {}\n\n",
        state["thread_id"].as_str().unwrap_or(""),
        state["scope"].as_str().unwrap_or(""),
        state["task_id"].as_str().unwrap_or(""),
        lenses,
        label
    );
    let items = dedupe(
        state["findings"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]),
    );
    if items.is_empty() {
        out.push_str("No findings.\n");
    }
    for item in items {
        let mut location = item["file"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("-")
            .to_string();
        if let Some(n) = item["line"].as_u64().filter(|n| *n > 0) {
            location.push_str(&format!(":{n}"));
        }
        out.push_str(&format!(
            "- [{}] {} ({}): {}",
            item["severity"].as_str().unwrap_or("?"),
            location,
            item["lens"].as_str().unwrap_or(""),
            item["claim"].as_str().unwrap_or("")
        ));
        if let Some(e) = item["evidence"].as_str().filter(|s| !s.is_empty()) {
            out.push_str(&format!(" — {e}"));
        }
        out.push('\n');
    }
    out
}
impl Fleet {
    pub fn initialize(
        &self,
        thread: &str,
        scope: &str,
        task: &str,
        c: &Config,
        legacy_database: Option<&Path>,
        action: Option<&str>,
    ) -> Result<()> {
        self.initialize_with_precheck(thread, scope, task, c, legacy_database, action, None)
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "Fleet initialization accepts optional precheck settings alongside legacy checkpoint import."
    )]
    pub fn initialize_with_precheck(
        &self,
        thread: &str,
        scope: &str,
        task: &str,
        c: &Config,
        legacy_database: Option<&Path>,
        action: Option<&str>,
        precheck: Option<&Value>,
    ) -> Result<()> {
        if c.lenses.is_empty()
            || c.lenses.len() > 40
            || c.concurrency == 0
            || c.concurrency > 40
            || c.lenses.iter().any(String::is_empty)
            || c.budget.is_some_and(|v| !v.is_finite() || v <= 0.)
        {
            return Err(HarnessError::invalid("Invalid Fleet configuration."));
        }
        {
            let db = self.store.connection()?;
            let _ = crate::clash::ensure_health_table(&db);
            if db
                .query_row(
                    "SELECT 1 FROM native_fleet_runs WHERE thread_id=?1",
                    [thread],
                    |_| Ok(()),
                )
                .optional()?
                .is_some()
            {
                return Ok(());
            }
        }
        if let Some(path) = legacy_database
            && path.exists()
            && let Some(run) = legacy::decode_checkpoint(path, thread, action)?
        {
            return legacy::persist(&self.store, &run);
        }
        let precheck_val = match precheck.filter(|v| !v.is_null()) {
            Some(v) => crate::clash::validate_precheck(Some(v))?,
            None => {
                let stored = self
                    .store
                    .get(thread)
                    .ok()
                    .and_then(|s| s.get("fleet").and_then(|f| f.get("precheck")).cloned())
                    .filter(|v| !v.is_null());
                match stored {
                    Some(v) => crate::clash::validate_precheck(Some(&v))?,
                    None => Value::Null,
                }
            }
        };
        let root_canon =
            infra_core::fs::canonicalize(&c.project).unwrap_or_else(|_| c.project.clone());
        let (project_id, project_path) =
            resolve_fleet_project_identity(&self.store, &root_canon, Some(thread), &Value::Null);
        let mut state = json!({
            "scope": scope,
            "task_id": task,
            "thread_id": thread,
            "project_path": project_path,
            "lenses": c.lenses,
            "findings": [],
            "cost_usd": 0.0,
            "usage_components": {"input_tokens": 0, "output_tokens": 0, "cache_read_input_tokens": 0, "cache_write_5m_input_tokens": 0, "cache_write_1h_input_tokens": 0, "model_calls": 0},
            "skipped": [],
        });
        if !project_id.is_empty() {
            state["project_id"] = json!(project_id);
        }
        if !precheck_val.is_null() {
            state["precheck"] = precheck_val;
        }
        self.store.connection()?.execute(
            "INSERT INTO native_fleet_runs(thread_id,state,stage,project_id,project_path)VALUES(?1,?2,'scope',?3,?4)",
            params![thread, state.to_string(), project_id, project_path],
        )?;
        Ok(())
    }
    pub fn snapshot(&self, thread: &str) -> Result<(Value, String, Option<bool>, i64)> {
        let db = self.store.connection()?;
        let (raw, stage, decision, rev) = db.query_row(
            "SELECT state,stage,decision,revision FROM native_fleet_runs WHERE thread_id=?1",
            [thread],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            },
        )?;
        Ok((
            crate::strict_json::parse(raw.as_bytes(), 16 * 1024 * 1024)?,
            stage,
            decision.map(|v| v != 0),
            rev,
        ))
    }
    fn transition(&self, thread: &str, rev: i64, state: &Value, stage: &str) -> Result<()> {
        if self.store.connection()?.execute("UPDATE native_fleet_runs SET state=?1,stage=?2,revision=revision+1 WHERE thread_id=?3 AND revision=?4",params![state.to_string(),stage,thread,rev])?!=1{return Err(HarnessError::conflict("Fleet state changed."));}
        Ok(())
    }
    pub fn decide(&self, thread: &str, approve: bool) -> Result<()> {
        let mut db = self.store.connection()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (stage, old): (String, Option<i64>) = tx.query_row(
            "SELECT stage,decision FROM native_fleet_runs WHERE thread_id=?1",
            [thread],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if old.is_some_and(|v| v != i64::from(approve))
            || !["gate", "record", "done"].contains(&stage.as_str())
        {
            return Err(HarnessError::conflict(
                "Fleet decision conflicts or the gate is unavailable.",
            ));
        }
        tx.execute("UPDATE native_fleet_runs SET decision=?1,stage=CASE WHEN stage='gate' THEN 'record' ELSE stage END,revision=revision+1 WHERE thread_id=?2",params![i64::from(approve),thread])?;
        tx.commit()?;
        Ok(())
    }
    fn tasks(&self, thread: &str) -> Result<Vec<(String, Value)>> {
        let db = self.store.connection()?;
        let mut q=db.prepare("SELECT task_id,input FROM native_fleet_tasks WHERE thread_id=?1 AND status!='completed' ORDER BY ordinal")?;
        let rows = q.query_map([thread], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, raw) = row?;
            out.push((id, crate::strict_json::parse(raw.as_bytes(), 1024 * 1024)?));
        }
        Ok(out)
    }
    fn finish(
        &self,
        thread: &str,
        tid: &str,
        result: &WorkerResult,
        usage_components: &Value,
        schema_error: Option<&str>,
    ) -> Result<()> {
        let mut db = self.store.connection()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (input, status): (String, String) = tx.query_row(
            "SELECT input,status FROM native_fleet_tasks WHERE thread_id=?1 AND task_id=?2",
            params![thread, tid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if status == "completed" {
            return Ok(());
        }
        if !result.ok {
            tx.execute("UPDATE native_fleet_tasks SET status='pending',error=?1 WHERE thread_id=?2 AND task_id=?3",params![result.error,thread,tid])?;
            tx.commit()?;
            return Err(HarnessError::unavailable(
                result.error.clone().unwrap_or("Reviewer failed.".into()),
            ));
        }
        let input = crate::strict_json::parse(input.as_bytes(), 1024 * 1024)?;
        let raw: String = tx.query_row(
            "SELECT state FROM native_fleet_runs WHERE thread_id=?1",
            [thread],
            |r| r.get(0),
        )?;
        let mut state = crate::strict_json::parse(raw.as_bytes(), 16 * 1024 * 1024)?;
        let lens = input["lens"].as_str().unwrap_or("");
        let (findings, schema_err) = match parse_findings_strict(&result.text, lens) {
            Ok(items) => (items, schema_error.map(str::to_owned)),
            Err(err) => (vec![], Some(schema_error.map(str::to_owned).unwrap_or(err))),
        };
        state["findings"]
            .as_array_mut()
            .ok_or_else(|| HarnessError::invalid("Invalid Fleet findings."))?
            .extend(findings.iter().cloned());
        state["cost_usd"] = json!(
            cost(&state["cost_usd"])
                .zip(result.cost_usd)
                .map(|(a, b)| a + b)
        );
        crate::clash::merge_usage_components(&mut state["usage_components"], usage_components);
        if let Some(err) = &schema_err {
            let mut errs = state["schema_errors"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if errs.len() < 32 {
                errs.push(json!({"lens": lens, "error": err}));
            }
            state["schema_errors"] = json!(errs);
            tx.execute(
                "UPDATE native_fleet_tasks SET status='schema_error',output=?1,error=?2 WHERE thread_id=?3 AND task_id=?4",
                params![
                    json!({"findings":findings,"cost_usd":result.cost_usd,"usage_components":usage_components,"schema_error":err}).to_string(),
                    err,
                    thread,
                    tid
                ],
            )?;
            tx.execute(
                "UPDATE native_fleet_runs SET state=?1,revision=revision+1 WHERE thread_id=?2",
                params![state.to_string(), thread],
            )?;
            tx.commit()?;
            return Err(HarnessError::unavailable(format!(
                "Fleet reviewer `{lens}` failed strict findings schema validation: {err}"
            )));
        }
        if let Some(errs) = state["schema_errors"].as_array_mut() {
            errs.retain(|e| e["lens"] != lens);
        }
        tx.execute("UPDATE native_fleet_tasks SET status='completed',output=?1,error=NULL WHERE thread_id=?2 AND task_id=?3",params![json!({"findings":findings,"cost_usd":result.cost_usd,"usage_components":usage_components,"schema_error":Value::Null}).to_string(),thread,tid])?;
        tx.execute(
            "UPDATE native_fleet_runs SET state=?1,revision=revision+1 WHERE thread_id=?2",
            params![state.to_string(), thread],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub async fn drive(
        &self,
        thread: &str,
        c: &Config,
        cancel: Arc<AtomicBool>,
        emit: Arc<dyn Fn(Value) -> Result<()> + Send + Sync>,
    ) -> Result<Value> {
        loop {
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                return Err(HarnessError::conflict("Cancellation requested."));
            }
            let (mut state, stage, decision, rev) = self.snapshot(thread)?;
            match stage.as_str() {
                "scope" => {
                    emit(json!({"kind":"fleet_stage","stage":"scope","status":"running"}))?;
                    if !c.dry_run {
                        self.blackboard.ensure_task(
                            state["task_id"].as_str().unwrap_or(""),
                            &format!(
                                "Unattended fleet review ({})",
                                state["scope"].as_str().unwrap_or("")
                            ),
                        )?;
                    }
                    let scope_plan = evaluate_fleet_lens_scope(&self.store, thread, &state, c);
                    state["scope_filter"] = json!({
                        "evaluated": scope_plan.evaluated,
                        "ir_revision": scope_plan.ir_revision,
                        "diff_files": scope_plan.diff_files,
                        "caller_files": scope_plan.caller_files,
                        "fallback": scope_plan.fallback,
                        "fallback_reason": scope_plan.fallback_reason,
                    });
                    let mut clean_receipts: Vec<Option<Value>> = Vec::with_capacity(c.lenses.len());
                    let mut active_count = 0usize;
                    for lens in &c.lenses {
                        let receipt = scope_plan.clean_receipt_if_disjoint(lens);
                        if receipt.is_none() {
                            active_count += 1;
                        }
                        clean_receipts.push(receipt);
                    }
                    let allowance = c.budget.map(|budget| {
                        cost(&state["cost_usd"])
                            .map(|spent| {
                                ((budget - spent).max(0.) / active_count.max(1) as f64 * 1_000_000.)
                                    .floor()
                                    / 1_000_000.
                            })
                            .unwrap_or(0.)
                    });
                    let mut skipped_receipts = state["skipped_receipts"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    {
                        let mut db = self.store.connection()?;
                        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
                        for (n, lens) in c.lenses.iter().enumerate() {
                            let tid = store::sha(format!("{thread}\0{n}\0{lens}").as_bytes());
                            let input = json!({"scope":state["scope"],"task_id":state["task_id"],"thread_id":thread,"lens":lens,"budget_usd":allowance});
                            let clean_receipt = clean_receipts[n].clone();
                            let skipped_budget =
                                clean_receipt.is_none() && allowance.is_some_and(|v| v <= 0.);
                            let skipped = clean_receipt.is_some() || skipped_budget;
                            let skip_output = clean_receipt
                                .as_ref()
                                .map(|receipt| {
                                    json!({
                                        "findings": [],
                                        "skipped": [lens],
                                        "skip_reason": "disjoint_scope",
                                        "ir_revision": scope_plan.ir_revision,
                                        "clean_receipt": receipt,
                                        "cost_usd": 0.0,
                                    })
                                    .to_string()
                                })
                                .or_else(|| {
                                    skipped_budget.then(|| {
                                        json!({"skipped":[lens],"cost_usd":0.0}).to_string()
                                    })
                                });
                            tx.execute("INSERT OR IGNORE INTO native_fleet_tasks(thread_id,task_id,lens,ordinal,status,input,output)VALUES(?1,?2,?3,?4,?5,?6,?7)",params![thread,tid,lens,n as i64,if skipped{"completed"}else{"pending"},input.to_string(),skip_output])?;
                            if skipped {
                                state["skipped"]
                                    .as_array_mut()
                                    .ok_or_else(|| HarnessError::invalid("Invalid skipped list."))?
                                    .push(Value::String(lens.clone()));
                            }
                            if let Some(receipt) = clean_receipt {
                                skipped_receipts.push(receipt);
                            }
                        }
                        if !skipped_receipts.is_empty() {
                            state["skipped_receipts"] = Value::Array(skipped_receipts);
                        }
                        tx.execute("UPDATE native_fleet_runs SET state=?1,stage='review',revision=revision+1 WHERE thread_id=?2 AND revision=?3",params![state.to_string(),thread,rev])?;
                        tx.commit()?;
                    }
                    if !c.dry_run
                        && let Some(pre) = state.get("precheck").filter(|v| !v.is_null())
                        && let Some(record) = crate::clash::run_precheck_sandboxed_in_store(
                            &self.store,
                            &c.project,
                            pre,
                        )
                        .await
                    {
                        let (mut st, _, _, r) = self.snapshot(thread)?;
                        st["deterministic_precheck"] = record;
                        self.transition(thread, r, &st, "review")?;
                    }
                    emit(json!({"kind":"fleet_stage","stage":"scope","status":"completed"}))?;
                }
                "review" => {
                    emit(json!({"kind":"fleet_stage","stage":"review","status":"running"}))?;
                    let health = collect_fleet_health(&self.store, Some(thread), &state, c);
                    let effective_concurrency = health.effective_concurrency;
                    let pre_warnings = health.warnings;
                    let current_eff = state.get("effective_concurrency").and_then(Value::as_u64);
                    let desired_eff = Some(effective_concurrency as u64);
                    if current_eff != desired_eff || !pre_warnings.is_empty() {
                        let (mut st, _, _, r) = self.snapshot(thread)?;
                        st["effective_concurrency"] = json!(effective_concurrency);
                        crate::clash::merge_health_warnings(&mut st, &pre_warnings);
                        self.transition(thread, r, &st, "review")?;
                        if !pre_warnings.is_empty() {
                            emit(json!({
                                "kind": "fleet_health",
                                "effective_concurrency": effective_concurrency,
                                "warnings": pre_warnings,
                                "lens_roi": health.lens_roi,
                            }))?;
                        }
                    }
                    let pre_note = match state
                        .get("deterministic_precheck")
                        .filter(|v| v.is_object())
                    {
                        Some(pre) => {
                            let summary = pre["summary"].as_str().unwrap_or("").trim();
                            if !summary.is_empty() {
                                format!("\n**Deterministic pre-stage check** -\n{summary}\n")
                            } else if pre["ok"] == false {
                                let code = pre["exit_code"]
                                    .as_i64()
                                    .map(|c| format!(" (exit code {c})"))
                                    .unwrap_or_default();
                                format!(
                                    "\n**Deterministic pre-stage check** -\nPrecheck failed{code}.\n"
                                )
                            } else {
                                String::new()
                            }
                        }
                        None => String::new(),
                    };
                    let semaphore = Arc::new(tokio::sync::Semaphore::new(effective_concurrency));
                    let mut running = tokio::task::JoinSet::new();
                    for (tid, input) in self.tasks(thread)? {
                        let permit =
                            semaphore.clone().acquire_owned().await.map_err(|_| {
                                HarnessError::unavailable("Fleet scheduler stopped.")
                            })?;
                        let engine = self.clone();
                        let cfg = c.clone();
                        let cancel = cancel.clone();
                        let emit = emit.clone();
                        let thread = thread.to_string();
                        let pre_note = pre_note.clone();
                        running.spawn(async move {
                            let _permit = permit;
                            let lens = input["lens"].as_str().ok_or_else(|| HarnessError::invalid("Missing lens."))?;
                            let task = input["task_id"].as_str().unwrap_or("");
                            emit(json!({"kind":"fleet_reviewer","lens":lens,"status":"running","cost_usd":null}))?;
                            let capsule = format!(
                                "**Objective** - review through the {lens} lens.\n**Output format** - JSON findings with file,line,severity(critical|high|medium|low|info),claim,evidence.\n**Tool and source guidance** - project root; scope {}. Task ID: {task}.\n**Task boundaries** - report only; do not modify files, run fixes or spawn agents.\n**Decisions and assumptions so far** - unattended run {thread}; no user amendments.\n{pre_note}",
                                input["scope"].as_str().unwrap_or("")
                            );
                            if !cfg.dry_run {
                                engine.blackboard.dispatch_spawn(task, lens, &capsule)?;
                            }
                            let (result, usage_components, schema_error) = if cfg.dry_run {
                                (
                                    WorkerResult {
                                        text: json!({"findings":[{"file":"DEMO.md","line":1,"severity":"low","claim":format!("Offline demonstration finding ({lens})"),"evidence":"Synthetic finding; no project code was reviewed."}]}).to_string(),
                                        ok: true,
                                        cost_usd: Some(0.),
                                        session_id: None,
                                        duration_seconds: 0.,
                                        error: None,
                                        limit_reached: None,
                                    },
                                    json!({"input_tokens":0u64,"output_tokens":0u64,"cache_read_input_tokens":0u64,"cache_write_5m_input_tokens":0u64,"cache_write_1h_input_tokens":0u64,"model_calls":0u64,"source":"deterministic"}),
                                    None,
                                )
                            } else {
                                run_reviewer_with_repair(
                                    &cfg,
                                    &thread,
                                    lens,
                                    &capsule,
                                    cost(&input["budget_usd"]),
                                    cancel,
                                    emit.clone(),
                                )
                                .await?
                            };
                            if !cfg.dry_run {
                                let note = if !result.ok {
                                    "Reviewer failed."
                                } else if let Some(err) = &schema_error {
                                    err.as_str()
                                } else {
                                    &result.text
                                };
                                engine.blackboard.dispatch_complete(task, lens, note)?;
                            }
                            let reviewer_status =
                                reviewer_event_status(result.ok, schema_error.as_deref());
                            emit(json!({
                                "kind":"fleet_reviewer",
                                "lens":lens,
                                "status":reviewer_status,
                                "cost_usd":result.cost_usd,
                                "usage_components":usage_components,
                                "schema_error":schema_error,
                                "duration_seconds":result.duration_seconds,
                                "limit_reached":result.limit_reached
                            }))?;
                            let finish_res = engine.finish(&thread, &tid, &result, &usage_components, schema_error.as_deref());
                            if !cfg.dry_run {
                                crate::clash::record_workflow_health(
                                    &engine.store,
                                    "fleet",
                                    &cfg.provider,
                                    cfg.model.as_deref(),
                                    &format!("{thread}:{tid}"),
                                    reviewer_status,
                                    schema_error.is_some() || finish_res.is_err(),
                                    !result.ok || schema_error.is_some(),
                                    result.cost_usd,
                                    result.duration_seconds,
                                );
                            }
                            finish_res
                        });
                    }
                    let mut failure = None;
                    while let Some(v) = running.join_next().await {
                        match v {
                            Ok(Ok(())) => {}
                            Ok(Err(e)) => {
                                failure.get_or_insert(e);
                            }
                            Err(_) => {
                                failure.get_or_insert(HarnessError::unavailable(
                                    "Reviewer task failed.",
                                ));
                            }
                        }
                    }
                    if let Some(e) = failure {
                        return Err(e);
                    }
                    let (state, _, _, rev) = self.snapshot(thread)?;
                    self.transition(thread, rev, &state, "collect")?;
                }
                "collect" => {
                    emit(json!({"kind":"fleet_stage","stage":"review","status":"completed"}))?;
                    self.transition(thread, rev, &state, "gate")?;
                    emit(json!({"kind":"fleet_stage","stage":"gate","status":"waiting"}))?;
                }
                "gate" => {
                    let health = collect_fleet_health(&self.store, Some(thread), &state, c);
                    let eff = state["effective_concurrency"]
                        .as_u64()
                        .unwrap_or(health.effective_concurrency as u64);
                    return Ok(
                        json!({"kind":"fleet_state","status":"awaiting_approval","findings":dedupe(state["findings"].as_array().map(Vec::as_slice).unwrap_or(&[])),"cost_usd":state["cost_usd"],"usage_components":state["usage_components"],"schema_errors":state["schema_errors"],"health_warnings":health.warnings,"lens_roi":health.lens_roi,"effective_concurrency":eff,"deterministic_precheck":state["deterministic_precheck"],"scope_filter":state["scope_filter"],"skipped":state["skipped"],"skipped_receipts":state["skipped_receipts"],"report":"","preview":report(&state),"dry_run":c.dry_run,"brain_available":!c.dry_run&&self.blackboard.available(),"task_id":state["task_id"]}),
                    );
                }
                "record" => {
                    let approve = decision
                        .ok_or_else(|| HarnessError::conflict("Fleet decision is missing."))?;
                    state["approved"] = approve.into();
                    let mut rendered = report(&state);
                    if c.dry_run {
                        rendered = format!(
                            "> Offline dry-run demonstration. No project code was reviewed; no models were called.\n\n{rendered}"
                        );
                    }
                    if approve {
                        infra_core::fs::ensure_directory(&c.reports, true)?;
                        let path = c.reports.join(format!("{thread}.md"));
                        infra_core::fs::admit_descendant(&c.reports, &path)?;
                        infra_core::fs::atomic_write(&path, rendered.as_bytes())?;
                        if !c.dry_run {
                            self.blackboard.progress(
                                state["task_id"].as_str().unwrap_or(""),
                                &format!(
                                    "fleet-review {thread}: approved report {}",
                                    path.display()
                                ),
                            )?;
                        }
                    } else if !c.dry_run {
                        self.blackboard.progress(
                            state["task_id"].as_str().unwrap_or(""),
                            &format!("fleet-review {thread}: rejected at the gate"),
                        )?;
                    }
                    state["report"] = if approve { rendered.into() } else { "".into() };
                    self.transition(thread, rev, &state, "done")?;
                }
                "done" => {
                    let health = collect_fleet_health(&self.store, Some(thread), &state, c);
                    let eff = state["effective_concurrency"]
                        .as_u64()
                        .unwrap_or(health.effective_concurrency as u64);
                    return Ok(
                        json!({"kind":"fleet_state","status":if state["approved"]==true{"completed"}else{"rejected"},"findings":dedupe(state["findings"].as_array().map(Vec::as_slice).unwrap_or(&[])),"cost_usd":state["cost_usd"],"usage_components":state["usage_components"],"schema_errors":state["schema_errors"],"health_warnings":health.warnings,"lens_roi":health.lens_roi,"effective_concurrency":eff,"deterministic_precheck":state["deterministic_precheck"],"scope_filter":state["scope_filter"],"skipped":state["skipped"],"skipped_receipts":state["skipped_receipts"],"report":state["report"],"preview":report(&state),"dry_run":c.dry_run,"brain_available":!c.dry_run,"task_id":state["task_id"]}),
                    );
                }
                _ => return Err(HarnessError::invalid("Invalid native Fleet stage.")),
            }
        }
    }
}

async fn run_reviewer_with_repair(
    cfg: &Config,
    thread: &str,
    lens: &str,
    capsule: &str,
    budget_usd: Option<f64>,
    cancel: Arc<AtomicBool>,
    emit: Arc<dyn Fn(Value) -> Result<()> + Send + Sync>,
) -> Result<(WorkerResult, Value, Option<String>)> {
    let usage = Arc::new(std::sync::Mutex::new(json!({
        "input_tokens": 0u64,
        "output_tokens": 0u64,
        "cache_read_input_tokens": 0u64,
        "cache_write_5m_input_tokens": 0u64,
        "cache_write_1h_input_tokens": 0u64,
        "model_calls": 0u64,
        "source": cfg.provider,
    })));
    let run_once = |prompt: String, usd: Option<f64>| {
        let cfg = cfg.clone();
        let cancel = cancel.clone();
        let emit = emit.clone();
        let usage = usage.clone();
        async move {
            let launch = Launch {
                provider: cfg.provider.clone(),
                executable: cfg.executable.clone(),
                project: cfg.project.clone(),
                prompt,
                mode: "plan".into(),
                model: cfg.model.clone(),
                session: None,
                agents: false,
                count: 1,
                effort: cfg.effort.clone(),
                usd,
                hook_events: false,
                permissions: None,
                approvals: None,
            };
            provider::run_monitored_locked(
                &launch,
                Limits {
                    seconds: Some(cfg.seconds),
                    ..Limits::default()
                },
                cancel,
                None,
                cfg.runner_lock.clone(),
                cfg.adapter.as_deref(),
                &std::collections::BTreeMap::new(),
                None,
                move |e| {
                    if e["kind"] == "usage"
                        && let Ok(mut slot) = usage.lock()
                    {
                        crate::clash::observe_usage_event(&mut slot, &e);
                    }
                    emit(e)
                },
            )
            .await
        }
    };
    let (initial_prompt, prefix_sha, attempt1_hit) =
        prepare_fleet_reviewer_prefix(thread, lens, capsule);
    let _ = emit(json!({
        "kind": "fleet_prefix_memo",
        "lens": lens,
        "attempt": 1,
        "prefix_sha256": prefix_sha,
        "hit": attempt1_hit,
    }));
    let first = run_once(initial_prompt, budget_usd).await?;
    let mut combined = first.clone();
    let mut schema_error = None;
    if first.ok
        && let Err(err) = parse_findings_strict(&first.text, lens)
    {
        let remaining_usd = match (budget_usd, first.cost_usd) {
            (Some(b), Some(used)) => Some((b - used).max(0.0)),
            (b, _) => b,
        };
        let can_repair = !cancel.load(std::sync::atomic::Ordering::Acquire)
            && remaining_usd.is_none_or(|r| r >= 0.01);
        if can_repair {
            let snippet: String = first.text.chars().take(1024).collect();
            let (repair_prompt, repair_prefix_sha, repair_hit) =
                build_fleet_repair_prompt(thread, lens, capsule, &err, &snippet);
            let _ = emit(json!({
                "kind": "fleet_prefix_memo",
                "lens": lens,
                "attempt": 2,
                "prefix_sha256": repair_prefix_sha,
                "hit": repair_hit,
            }));
            if let Ok(repaired) = run_once(repair_prompt, remaining_usd).await {
                combined.cost_usd = match (first.cost_usd, repaired.cost_usd) {
                    (Some(a), Some(b)) => Some(a + b),
                    (a, b) => a.or(b),
                };
                combined.duration_seconds += repaired.duration_seconds;
                if repaired.ok {
                    match parse_findings_strict(&repaired.text, lens) {
                        Ok(_) => combined.text = repaired.text,
                        Err(err2) => schema_error = Some(err2),
                    }
                } else {
                    schema_error = Some(err);
                }
            } else {
                schema_error = Some(err);
            }
        } else {
            schema_error = Some(err);
        }
    }
    let usage_val = usage
        .lock()
        .map(|g| g.clone())
        .unwrap_or_else(|_| json!({}));
    Ok((combined, usage_val, schema_error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_findings_parser_refuses_unparsed_prose_and_normalizes_valid_json() {
        let prose = "No defects found cleanly without JSON.";
        assert!(parse_findings_strict(prose, "security").is_err());
        assert!(parse_findings(prose, "security").is_empty());

        let invalid_sev = r#"{"findings":[{"severity":"bogus","claim":"bad","evidence":"x"}]}"#;
        assert!(parse_findings_strict(invalid_sev, "security").is_err());

        let valid_fenced = "```json\n{\"findings\":[{\"severity\":\"HIGH\",\"claim\":\"SQL injection\",\"file\":\"app.php\",\"line\":10,\"evidence\":\"raw sql\"}]}\n```";
        let findings = parse_findings_strict(valid_fenced, "security").unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0]["severity"], "high");
        assert_eq!(findings[0]["lens"], "security");

        let mut acc = json!({});
        crate::clash::merge_usage_components(
            &mut acc,
            &json!({"input_tokens": 20, "output_tokens": 5, "cache_read_input_tokens": 80, "model_calls": 1}),
        );
        assert_eq!(acc["input_tokens"], 20);
        assert_eq!(acc["output_tokens"], 5);
        assert_eq!(acc["cache_read_input_tokens"], 80);
        assert_eq!(acc["model_calls"], 1);
    }

    struct NoopBlackboard;
    impl Blackboard for NoopBlackboard {
        fn available(&self) -> bool {
            false
        }
        fn ensure_task(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn dispatch_spawn(&self, _: &str, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn dispatch_complete(&self, _: &str, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn progress(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn fleet_initialize_with_precheck_persists_precheck_and_rejects_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&dir.path().join("state")).unwrap();
        let fleet = Fleet {
            store: store.clone(),
            blackboard: Arc::new(NoopBlackboard),
        };
        let cfg = Config {
            project: dir.path().to_path_buf(),
            provider: "claude".into(),
            executable: dir.path().join("fake-cli"),
            lenses: vec!["code-reviewer".into()],
            model: None,
            effort: None,
            budget: None,
            seconds: 30,
            concurrency: 1,
            dry_run: true,
            reports: dir.path().join("reports"),
            adapter: None,
            runner_lock: None,
        };

        assert!(
            fleet
                .initialize_with_precheck(
                    "t-bad",
                    "scope",
                    "harness/t-bad",
                    &cfg,
                    None,
                    None,
                    Some(&json!({"argv": []})),
                )
                .is_err()
        );

        let precheck = json!({"argv": ["composer", "validate"], "timeout_seconds": 20});
        fleet
            .initialize_with_precheck(
                "t-pre",
                "scope",
                "harness/t-pre",
                &cfg,
                None,
                None,
                Some(&precheck),
            )
            .unwrap();
        let (state, stage, _, _) = fleet.snapshot("t-pre").unwrap();
        assert_eq!(stage, "scope");
        assert_eq!(state["precheck"], precheck);
    }

    #[tokio::test]
    async fn fleet_schema_error_never_marks_task_completed_and_retries_cleanly() {
        assert_eq!(reviewer_event_status(false, None), "failed");
        assert_eq!(
            reviewer_event_status(true, Some("bad json")),
            "schema_error"
        );
        assert_eq!(reviewer_event_status(true, None), "completed");

        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&dir.path().join("state")).unwrap();
        let fleet = Fleet {
            store: store.clone(),
            blackboard: Arc::new(NoopBlackboard),
        };
        let cfg = Config {
            project: dir.path().to_path_buf(),
            provider: "claude".into(),
            executable: dir.path().join("fake-cli"),
            lenses: vec!["code-reviewer".into(), "security-reviewer".into()],
            model: None,
            effort: None,
            budget: None,
            seconds: 30,
            concurrency: 2,
            dry_run: true,
            reports: dir.path().join("reports"),
            adapter: None,
            runner_lock: None,
        };
        let thread = "t-schema";
        fleet
            .initialize(thread, "src/", "harness/t-schema", &cfg, None, None)
            .unwrap();
        // Seed both tasks in `review` stage mirroring `scope`.
        let (state, _, _, rev) = fleet.snapshot(thread).unwrap();
        fleet.transition(thread, rev, &state, "review").unwrap();
        let mut tids = Vec::new();
        {
            let db = store.connection().unwrap();
            for (n, lens) in cfg.lenses.iter().enumerate() {
                let tid = store::sha(format!("{thread}\0{n}\0{lens}").as_bytes());
                let input = json!({"scope":"src/","task_id":"harness/t-schema","thread_id":thread,"lens":lens,"budget_usd":null});
                db.execute(
                    "INSERT INTO native_fleet_tasks(thread_id,task_id,lens,ordinal,status,input)VALUES(?1,?2,?3,?4,'pending',?5)",
                    params![thread, tid, lens, n as i64, input.to_string()],
                )
                .unwrap();
                tids.push(tid);
            }
        }

        let usage = json!({"input_tokens": 10, "output_tokens": 5, "model_calls": 1});
        // Lens 0 succeeds with valid structured JSON.
        fleet
            .finish(
                thread,
                &tids[0],
                &WorkerResult {
                    text: r#"{"findings":[{"file":"a.php","line":1,"severity":"medium","claim":"Check","evidence":"e"}]}"#.into(),
                    ok: true,
                    cost_usd: Some(0.02),
                    session_id: None,
                    duration_seconds: 1.0,
                    error: None,
                    limit_reached: None,
                },
                &usage,
                None,
            )
            .unwrap();

        // Lens 1 returns invalid prose after repair -> must NOT mark task `completed`!
        let err = fleet
            .finish(
                thread,
                &tids[1],
                &WorkerResult {
                    text: "No JSON findings here".into(),
                    ok: true,
                    cost_usd: Some(0.03),
                    session_id: None,
                    duration_seconds: 1.0,
                    error: None,
                    limit_reached: None,
                },
                &usage,
                Some("reviewer output is not a valid JSON object"),
            )
            .unwrap_err();
        assert!(err.message.contains("security-reviewer"), "{}", err.message);

        let (task_status, task_err): (String, Option<String>) = store
            .connection()
            .unwrap()
            .query_row(
                "SELECT status, error FROM native_fleet_tasks WHERE thread_id=?1 AND task_id=?2",
                params![thread, tids[1]],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(task_status, "schema_error");
        assert!(task_err.is_some());

        let (state_after_err, stage_after_err, _, _) = fleet.snapshot(thread).unwrap();
        assert_eq!(stage_after_err, "review");
        assert_eq!(state_after_err["findings"].as_array().unwrap().len(), 1);
        assert_eq!(
            state_after_err["schema_errors"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            state_after_err["schema_errors"][0]["lens"],
            "security-reviewer"
        );

        // Pending tasks for resume include ONLY the schema_error lens.
        let pending = fleet.tasks(thread).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, tids[1]);

        // Retry succeeds -> clears schema_error for that lens and advances cleanly via drive().
        fleet
            .finish(
                thread,
                &tids[1],
                &WorkerResult {
                    text: r#"{"findings":[{"file":"b.php","line":4,"severity":"high","claim":"Auth bypass","evidence":"missing check"}]}"#.into(),
                    ok: true,
                    cost_usd: Some(0.02),
                    session_id: None,
                    duration_seconds: 1.0,
                    error: None,
                    limit_reached: None,
                },
                &usage,
                None,
            )
            .unwrap();
        assert!(fleet.tasks(thread).unwrap().is_empty());

        let gated = fleet
            .drive(
                thread,
                &cfg,
                Arc::new(AtomicBool::new(false)),
                Arc::new(|_| Ok(())),
            )
            .await
            .unwrap();
        assert_eq!(gated["status"], "awaiting_approval");
        assert_eq!(gated["findings"].as_array().unwrap().len(), 2);
        assert_eq!(gated["schema_errors"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn fleet_prelaunch_smoothed_health_throttles_concurrency_and_emits_warnings() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&dir.path().join("state")).unwrap();
        let ws = dir.path().join("project");
        std::fs::create_dir_all(&ws).unwrap();
        for i in 0..4 {
            crate::clash::record_workflow_health(
                &store,
                "fleet",
                "codex",
                None,
                &format!("prior-{i}"),
                "schema_error",
                true,
                true,
                Some(0.05),
                1.0,
            );
        }
        let fleet = Fleet {
            store,
            blackboard: Arc::new(NoopBlackboard),
        };
        let cfg = Config {
            project: ws,
            provider: "codex".into(),
            executable: PathBuf::from("codex"),
            lenses: vec!["security-reviewer".into(), "architect".into()],
            model: None,
            effort: None,
            budget: None,
            seconds: 60,
            concurrency: 4,
            dry_run: true,
            reports: dir.path().join("reports"),
            adapter: None,
            runner_lock: None,
        };
        let thread = "fleet-throttle-1";
        fleet
            .initialize(thread, "task-throttle", "Throttle check", &cfg, None, None)
            .unwrap();
        let events = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let events_sink = events.clone();
        let gated = fleet
            .drive(
                thread,
                &cfg,
                Arc::new(AtomicBool::new(false)),
                Arc::new(move |ev| {
                    events_sink.lock().unwrap().push(ev);
                    Ok(())
                }),
            )
            .await
            .unwrap();
        assert_eq!(gated["status"], "awaiting_approval");
        assert_eq!(gated["effective_concurrency"], 1);
        let warnings = gated["health_warnings"].as_array().unwrap();
        assert!(
            warnings.iter().any(|w| w
                .as_str()
                .unwrap_or("")
                .contains("Throttled Fleet concurrency")),
            "{warnings:?}"
        );
        assert!(
            events
                .lock()
                .unwrap()
                .iter()
                .any(|e| e["kind"] == "fleet_health" && e["effective_concurrency"] == 1),
            "fleet_health event must be emitted pre-launch"
        );
    }

    #[tokio::test]
    async fn fleet_dependency_scope_skips_disjoint_lens_with_ir_revision_receipt_and_falls_back_when_uncertain()
     {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&dir.path().join("state")).unwrap();
        let ws = dir.path().join("project");
        std::fs::create_dir_all(ws.join("src/auth")).unwrap();
        std::fs::create_dir_all(ws.join("src/api")).unwrap();
        std::fs::create_dir_all(ws.join("src/billing")).unwrap();
        std::fs::create_dir_all(ws.join(".claude/agents")).unwrap();
        std::fs::write(
            ws.join("Cargo.toml"),
            "[package]\nname = \"fleetscope\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(
            ws.join("src/lib.rs"),
            "pub mod auth;\npub mod api;\npub mod billing;\n",
        )
        .unwrap();
        std::fs::write(ws.join("src/auth/mod.rs"), "pub mod token;\n").unwrap();
        std::fs::write(
            ws.join("src/auth/token.rs"),
            "pub fn verify() -> bool { true }\n",
        )
        .unwrap();
        std::fs::write(ws.join("src/api/mod.rs"), "pub mod handler;\n").unwrap();
        std::fs::write(
            ws.join("src/api/handler.rs"),
            "pub fn check() -> bool { crate::auth::token::verify() }\n",
        )
        .unwrap();
        std::fs::write(ws.join("src/billing/mod.rs"), "pub fn charge() {}\n").unwrap();
        std::fs::write(
            ws.join(".harness-fleet.json"),
            serde_json::to_string_pretty(&json!({
                "lenses": {
                    "api-reviewer": {"watched_globs": ["src/api/**"]},
                    "billing-reviewer": {"watched_globs": ["src/billing/**"]}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            ws.join(".claude/agents/billing-agent.md"),
            "---\nname: billing-agent\nwatched_globs:\n  - src/billing/**\n---\nReview billing.\n",
        )
        .unwrap();

        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&ws)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init"]);
        git(&["add", "."]);
        git(&["commit", "-m", "initial"]);

        let reg = store.register(&ws).unwrap();
        let pid = reg["id"].as_str().unwrap().to_owned();
        let graphs =
            crate::graph::Graphs::with_native_backend(store.clone(), std::time::Duration::ZERO);
        graphs.auto_start(&pid).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if graphs.status(&pid, false).unwrap()["state"] == "ready" {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(graphs.status(&pid, false).unwrap()["state"], "ready");
        // Stop the live filesystem watcher started by `auto_start` so editing `token.rs`
        // below deterministically leaves `GraphIrStore` stale on slower/Windows CI before
        // `evaluate_fleet_lens_scope` runs multiple git checks.
        graphs.close();

        // Edit ONLY src/auth/token.rs:
        // - Before Graph-IR refreshes for dirty `src/auth/token.rs`, `evaluate_fleet_lens_scope`
        //   must refuse to trust stale IR callers and fall back conservatively.
        std::fs::write(
            ws.join("src/auth/token.rs"),
            "pub fn verify() -> bool { false }\n",
        )
        .unwrap();

        let fleet = Fleet {
            store: store.clone(),
            blackboard: Arc::new(NoopBlackboard),
        };
        let cfg = Config {
            project: ws.clone(),
            provider: "claude".into(),
            executable: PathBuf::from("claude"),
            lenses: vec![
                "api-reviewer".into(),
                "billing-reviewer".into(),
                "billing-agent".into(),
                "general-reviewer".into(),
            ],
            model: None,
            effort: None,
            budget: None,
            seconds: 60,
            concurrency: 2,
            dry_run: true,
            reports: dir.path().join("reports"),
            adapter: None,
            runner_lock: None,
        };
        let stale_plan = evaluate_fleet_lens_scope(&store, "fleet-stale", &json!({}), &cfg);
        assert!(stale_plan.fallback);
        assert_eq!(
            stale_plan.fallback_reason.as_deref(),
            Some("graph_ir_stale_for_diff")
        );

        // Once Graph-IR indexes the dirty change cleanly:
        // - `api-reviewer` watches `src/api/**`, which overlaps via Graph-IR 1-hop caller `src/api/handler.rs`!
        // - `billing-reviewer` & `billing-agent` watch `src/billing/**`, strictly disjoint -> skipped with ir_revision receipt!
        // - `general-reviewer` has no watched_globs -> conservative fallback, still runs!
        let ir = crate::graph_ir::GraphIrStore::open(&store.state, &pid).unwrap();
        ir.update_paths_with_delta(&ws, &["src/auth/token.rs".to_owned()])
            .unwrap();

        let thread = "fleet-scope-1";
        fleet
            .initialize(thread, "task-scope-1", "Scoped review", &cfg, None, None)
            .unwrap();
        let gated = fleet
            .drive(
                thread,
                &cfg,
                Arc::new(AtomicBool::new(false)),
                Arc::new(|_| Ok(())),
            )
            .await
            .unwrap();
        assert_eq!(gated["status"], "awaiting_approval");
        let skipped: Vec<String> = gated["skipped"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        assert_eq!(skipped, vec!["billing-reviewer", "billing-agent"]);
        let receipts = gated["skipped_receipts"].as_array().unwrap();
        assert_eq!(receipts.len(), 2);
        assert_eq!(receipts[0]["status"], "clean_disjoint");
        assert!(receipts[0]["ir_revision"].as_u64().unwrap_or(0) >= 1);
        let reviewed_lenses: Vec<String> = gated["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|f| f["lens"].as_str().map(str::to_owned))
            .collect();
        assert!(
            reviewed_lenses.contains(&"api-reviewer".to_owned()),
            "{reviewed_lenses:?}"
        );
        assert!(
            reviewed_lenses.contains(&"general-reviewer".to_owned()),
            "{reviewed_lenses:?}"
        );
        assert!(!reviewed_lenses.contains(&"billing-reviewer".to_owned()));

        // Uncertain project (no .git / no IR) falls back conservatively and runs all lenses.
        let uncertain_ws = dir.path().join("uncertain");
        std::fs::create_dir_all(&uncertain_ws).unwrap();
        let uncertain_cfg = Config {
            project: uncertain_ws,
            lenses: vec!["billing-reviewer".into(), "api-reviewer".into()],
            ..cfg
        };
        fleet
            .initialize(
                "fleet-scope-fallback",
                "task-fb",
                "Fallback",
                &uncertain_cfg,
                None,
                None,
            )
            .unwrap();
        let fb_gated = fleet
            .drive(
                "fleet-scope-fallback",
                &uncertain_cfg,
                Arc::new(AtomicBool::new(false)),
                Arc::new(|_| Ok(())),
            )
            .await
            .unwrap();
        assert_eq!(fb_gated["scope_filter"]["fallback"], true);
        assert!(fb_gated["skipped"].as_array().unwrap().is_empty());
        assert_eq!(fb_gated["findings"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn fleet_prefix_memo_reuses_stable_prefix_across_repair_retries() {
        let capsule = "**Objective** - check SQL queries.\n**Scope** - app/Orders.";
        let (p1, sha1, hit1) =
            prepare_fleet_reviewer_prefix("thread-prefix-1", "security-reviewer", capsule);
        assert!(!hit1);
        assert!(p1.starts_with("Act as security-reviewer. Return only JSON findings."));

        let (p2, sha2, hit2) = build_fleet_repair_prompt(
            "thread-prefix-1",
            "security-reviewer",
            capsule,
            "missing findings key",
            "No defects",
        );
        assert!(hit2, "repair retry must hit FleetPrefixMemoStore");
        assert_eq!(sha1, sha2);
        assert!(
            p2.starts_with(&p1),
            "repair prompt must preserve stable cached prefix"
        );
        assert!(p2.contains("missing findings key"));
    }

    #[test]
    fn fleet_lens_roi_telemetry_aggregates_cost_and_accepted_findings_and_warns_on_zero_roi() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&dir.path().join("state")).unwrap();
        let fleet = Fleet {
            store: store.clone(),
            blackboard: Arc::new(NoopBlackboard),
        };
        let cfg = Config {
            project: dir.path().join("proj"),
            provider: "claude".into(),
            executable: PathBuf::from("claude"),
            lenses: vec!["security".into(), "noisy".into()],
            model: None,
            effort: None,
            budget: None,
            seconds: 60,
            concurrency: 2,
            dry_run: true,
            reports: dir.path().join("reports"),
            adapter: None,
            runner_lock: None,
        };
        std::fs::create_dir_all(&cfg.project).unwrap();
        for i in 0..3 {
            let tid = format!("roi-run-{i}");
            fleet
                .initialize(&tid, "task-roi", "ROI aggregation", &cfg, None, None)
                .unwrap();
            let db = store.connection().unwrap();
            db.execute(
                "UPDATE native_fleet_runs SET state=?1, stage='done', decision=1 WHERE thread_id=?2",
                params![
                    json!({"approved": true, "findings": [], "skipped": []}).to_string(),
                    tid
                ],
            )
            .unwrap();
            db.execute(
                "INSERT INTO native_fleet_tasks(thread_id,task_id,lens,ordinal,status,input,output) VALUES(?1,'t-sec','security',0,'completed','{}',?2)",
                params![
                    tid,
                    json!({
                        "findings": [{"file": "a.php", "line": 1, "severity": "high", "claim": "bug", "evidence": "x"}],
                        "cost_usd": 0.02
                    })
                    .to_string()
                ],
            )
            .unwrap();
            db.execute(
                "INSERT INTO native_fleet_tasks(thread_id,task_id,lens,ordinal,status,input,output) VALUES(?1,'t-noisy','noisy',1,'completed','{}',?2)",
                params![tid, json!({"findings": [], "cost_usd": 0.04}).to_string()],
            )
            .unwrap();
        }
        let report = collect_fleet_health(&store, None, &json!({}), &cfg);
        let sec = report
            .lens_roi
            .iter()
            .find(|v| v["lens"] == "security")
            .unwrap();
        assert_eq!(sec["runs"], 3);
        assert_eq!(sec["accepted_findings"], 3);
        assert_eq!(sec["total_cost_usd"], 0.06);
        assert_eq!(sec["cost_per_accepted_finding"], 0.02);
        let noisy = report
            .lens_roi
            .iter()
            .find(|v| v["lens"] == "noisy")
            .unwrap();
        assert_eq!(noisy["runs"], 3);
        assert_eq!(noisy["accepted_findings"], 0);
        assert_eq!(noisy["total_cost_usd"], 0.12);
        assert!(noisy["cost_per_accepted_finding"].is_null());
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("Low ROI warning") && w.contains("noisy")),
            "{:?}",
            report.warnings
        );
    }

    #[test]
    fn fleet_scope_unions_branch_and_dirty_diffs_and_isolates_project_globs_and_roi() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&dir.path().join("state")).unwrap();
        let ws_a = dir.path().join("proj-a");
        let ws_b = dir.path().join("proj-b");
        std::fs::create_dir_all(ws_a.join("src")).unwrap();
        std::fs::create_dir_all(&ws_b).unwrap();
        let git_a = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&ws_a)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        std::fs::write(ws_a.join("src/base.rs"), "pub fn b() {}\n").unwrap();
        git_a(&["init", "-b", "main"]);
        git_a(&["add", "."]);
        git_a(&["commit", "-m", "main base"]);
        git_a(&["checkout", "-b", "feature/two-commits"]);
        std::fs::write(ws_a.join("src/commit1.rs"), "pub fn c1() {}\n").unwrap();
        git_a(&["add", "."]);
        git_a(&["commit", "-m", "commit 1"]);
        std::fs::write(ws_a.join("src/commit2.rs"), "pub fn c2() {}\n").unwrap();
        git_a(&["add", "."]);
        git_a(&["commit", "-m", "commit 2"]);
        // Dirty working tree file must NOT hide commit1.rs or commit2.rs on feature branch.
        std::fs::write(ws_a.join("src/dirty.rs"), "pub fn d() {}\n").unwrap();
        let diff_files = collect_git_diff_files(&ws_a, None).unwrap();
        for expected in ["src/commit1.rs", "src/commit2.rs", "src/dirty.rs"] {
            assert!(
                diff_files.iter().any(|p| p == expected),
                "missing {expected} from {diff_files:?}"
            );
        }

        // Capsule dependency_manifest globs and ROI telemetry must stay strictly scoped by project.
        let reg_a = store.register(&ws_a).unwrap();
        let reg_b = store.register(&ws_b).unwrap();
        let pid_a = reg_a["id"].as_str().unwrap().to_owned();
        let pid_b = reg_b["id"].as_str().unwrap().to_owned();
        {
            let db = store.connection().unwrap();
            db.execute(
                "INSERT INTO skillcache_capsules(capsule_id,project_id,skill_name,skill_path,mu_name,mu_description,form,status,revision,capsule_sha256,tree_sha256,skill_files,env_fingerprint,schema_version,compile_rules_version,source_session_id,source_generation,compile_id,artifact_dir,created_at,updated_at,dependency_manifest) VALUES('cap-b',?1,'shared-lens','.claude/skills/shared-lens/SKILL.md','shared-lens','desc','hybrid','active',1,'s','t','[]','e',1,'r','sess-b','gen-1','comp-b','dir','2026-01-01','2026-01-01',?2)",
                params![
                    pid_b,
                    json!({"watched_globs": ["src/only-b/**"]}).to_string()
                ],
            )
            .unwrap();
        }
        let globs_a = resolve_lens_watched_globs(
            &store,
            &ws_a,
            Some(&pid_a),
            &json!({}),
            &["shared-lens".into()],
        );
        assert!(
            globs_a.get("shared-lens").is_none_or(|v| v.is_empty()),
            "foreign project capsule globs must not leak: {globs_a:?}"
        );

        let fleet = Fleet {
            store: store.clone(),
            blackboard: Arc::new(NoopBlackboard),
        };
        let cfg_b = Config {
            project: ws_b.clone(),
            provider: "claude".into(),
            executable: PathBuf::from("claude"),
            lenses: vec!["shared-lens".into()],
            model: None,
            effort: None,
            budget: None,
            seconds: 60,
            concurrency: 1,
            dry_run: true,
            reports: dir.path().join("reports-b"),
            adapter: None,
            runner_lock: None,
        };
        let cfg_a = Config {
            project: ws_a.clone(),
            reports: dir.path().join("reports-a"),
            ..cfg_b.clone()
        };
        for i in 0..4 {
            let tid = format!("b-noisy-{i}");
            fleet
                .initialize(&tid, "task-b", "Noisy B", &cfg_b, None, None)
                .unwrap();
            let db = store.connection().unwrap();
            db.execute(
                "INSERT INTO native_fleet_tasks(thread_id,task_id,lens,ordinal,status,input,output) VALUES(?1,'t','shared-lens',0,'completed','{}',?2)",
                params![tid, json!({"findings": [], "cost_usd": 0.5}).to_string()],
            )
            .unwrap();
        }
        let health_a = collect_fleet_health(&store, None, &json!({}), &cfg_a);
        let shared_a = health_a
            .lens_roi
            .iter()
            .find(|v| v["lens"] == "shared-lens")
            .unwrap();
        assert_eq!(shared_a["runs"], 0);
        assert_eq!(shared_a["total_cost_usd"], 0.0);
        assert!(
            !health_a
                .warnings
                .iter()
                .any(|w| w.contains("Low ROI warning")),
            "foreign project ROI must not warn in project A: {:?}",
            health_a.warnings
        );
    }
}
