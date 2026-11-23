use crate::{
    Result, RuntimeError, common,
    platform_cli::{self, PlatformConfig},
    utils,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, mpsc},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_DESCRIPTION_CHARS: usize = 4096;
const MAX_QUERY_MEMO_ENTRIES: usize = 4096;
const MAX_TREE_DIGEST_FILES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct QueryMemoKey {
    platform: String,
    skill_name: String,
    tree_digest: String,
    timeout_ms: u64,
    description_sha256: String,
    query_sha256: String,
    model: String,
    sample_index: usize,
}

impl QueryMemoKey {
    fn digest_hex(&self) -> String {
        let payload = format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
            self.platform,
            self.skill_name,
            self.tree_digest,
            self.timeout_ms,
            self.description_sha256,
            self.query_sha256,
            self.model,
            self.sample_index
        );
        infra_core::query::sha256(payload.as_bytes())
    }
}

#[derive(Default)]
struct QueryMemoCache {
    entries: HashMap<QueryMemoKey, bool>,
    order: VecDeque<QueryMemoKey>,
    hits: u64,
    misses: u64,
    conn: Option<Connection>,
}

impl QueryMemoCache {
    fn open(path: &Path) -> std::result::Result<Self, rusqlite::Error> {
        if let Some(parent) = path.parent()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(err)));
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS query_memo (
                memo_key TEXT PRIMARY KEY,
                platform TEXT NOT NULL,
                skill_name TEXT NOT NULL,
                tree_digest TEXT NOT NULL,
                timeout_ms INTEGER NOT NULL,
                description_sha256 TEXT NOT NULL,
                query_sha256 TEXT NOT NULL,
                model TEXT NOT NULL,
                sample_index INTEGER NOT NULL,
                result INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_query_memo_updated ON query_memo(updated_at);",
        )?;
        Ok(Self {
            conn: Some(conn),
            ..Self::default()
        })
    }

    fn open_best_effort(path: &Path) -> Self {
        Self::open(path).unwrap_or_default()
    }

    fn unix_now_nanos() -> i64 {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        i64::try_from(nanos).unwrap_or(i64::MAX)
    }

    fn db_get(conn: &Connection, memo_key: &str) -> Option<bool> {
        let row: Option<i64> = conn
            .query_row(
                "SELECT result FROM query_memo WHERE memo_key = ?1",
                params![memo_key],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten();
        if row.is_some() {
            let _ = conn.execute(
                "UPDATE query_memo SET updated_at = ?2 WHERE memo_key = ?1",
                params![memo_key, Self::unix_now_nanos()],
            );
        }
        row.map(|value| value != 0)
    }

    fn db_put(conn: &Connection, key: &QueryMemoKey, result: bool) {
        let memo_key = key.digest_hex();
        let timeout_ms = i64::try_from(key.timeout_ms).unwrap_or(i64::MAX);
        let sample_index = i64::try_from(key.sample_index).unwrap_or(i64::MAX);
        let stored = i64::from(result);
        if conn
            .execute(
                "INSERT OR REPLACE INTO query_memo (
                    memo_key, platform, skill_name, tree_digest, timeout_ms,
                    description_sha256, query_sha256, model, sample_index, result, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    memo_key,
                    key.platform,
                    key.skill_name,
                    key.tree_digest,
                    timeout_ms,
                    key.description_sha256,
                    key.query_sha256,
                    key.model,
                    sample_index,
                    stored,
                    Self::unix_now_nanos(),
                ],
            )
            .is_ok()
        {
            let _ = conn.execute(
                "DELETE FROM query_memo
                 WHERE memo_key IN (
                     SELECT memo_key FROM query_memo
                     ORDER BY updated_at DESC, rowid DESC
                     LIMIT -1 OFFSET ?1
                 )",
                params![i64::try_from(MAX_QUERY_MEMO_ENTRIES).unwrap_or(i64::MAX)],
            );
        }
    }

    fn remember_l1(&mut self, key: QueryMemoKey, result: bool) {
        if let std::collections::hash_map::Entry::Occupied(mut entry) =
            self.entries.entry(key.clone())
        {
            entry.insert(result);
            return;
        }
        if self.entries.len() >= MAX_QUERY_MEMO_ENTRIES
            && let Some(evicted) = self.order.pop_front()
        {
            self.entries.remove(&evicted);
        }
        self.order.push_back(key.clone());
        self.entries.insert(key, result);
    }

    fn get(&mut self, key: &QueryMemoKey) -> Option<bool> {
        if let Some(&hit) = self.entries.get(key) {
            self.hits += 1;
            return Some(hit);
        }
        let db_hit = self
            .conn
            .as_ref()
            .and_then(|conn| Self::db_get(conn, &key.digest_hex()));
        if let Some(hit) = db_hit {
            self.hits += 1;
            self.remember_l1(key.clone(), hit);
            return Some(hit);
        }
        None
    }

    fn insert(&mut self, key: QueryMemoKey, result: bool) {
        self.misses += 1;
        if let Some(conn) = &self.conn {
            Self::db_put(conn, &key, result);
        }
        self.remember_l1(key, result);
    }
}

fn default_memo_db_path() -> PathBuf {
    if let Some(custom) = std::env::var_os("INFRA_SKILL_TOOLS_MEMO_DB")
        && !custom.is_empty()
    {
        return PathBuf::from(custom);
    }
    #[cfg(test)]
    {
        static TEST_DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
        let dir = TEST_DIR.get_or_init(|| {
            tempfile::Builder::new()
                .prefix("infra-skill-tools-memo-")
                .tempdir()
                .expect("create test memo tempdir")
        });
        dir.path().join("query-memo.sqlite3")
    }
    #[cfg(not(test))]
    {
        let base = std::env::var_os("XDG_CACHE_HOME")
            .filter(|val| !val.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|val| !val.is_empty())
                    .map(|home| PathBuf::from(home).join(".cache"))
            })
            .unwrap_or_else(std::env::temp_dir);
        base.join("infra-skill-tools").join("query-memo.sqlite3")
    }
}

fn query_memo() -> &'static Mutex<QueryMemoCache> {
    static MEMO: OnceLock<Mutex<QueryMemoCache>> = OnceLock::new();
    MEMO.get_or_init(|| Mutex::new(QueryMemoCache::open_best_effort(&default_memo_db_path())))
}

fn is_transient_probe_file(file_name: &str, skill_name: &str) -> bool {
    let prefix = format!("{skill_name}-skill-");
    file_name
        .strip_prefix(&prefix)
        .and_then(|rest| rest.strip_suffix(".md"))
        .is_some_and(|hex| hex.len() == 8 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn collect_tree_digests(
    base: &Path,
    dir: &Path,
    skill_name: &str,
    records: &mut Vec<(String, String)>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    children.sort();
    for path in children {
        if records.len() >= MAX_TREE_DIGEST_FILES {
            break;
        }
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if meta.is_dir() {
            if matches!(name, ".git" | "node_modules" | "target" | "vendor") {
                continue;
            }
            collect_tree_digests(base, &path, skill_name, records);
        } else if meta.is_file() {
            if is_transient_probe_file(name, skill_name) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            records.push((rel, infra_core::query::sha256(&bytes)));
        }
    }
}

fn skill_tree_digest(project: &Path, skills_path: Option<&str>, skill_name: &str) -> String {
    let canonical = std::fs::canonicalize(project).unwrap_or_else(|_| project.to_path_buf());
    let mut records: Vec<(String, String)> = Vec::new();
    if let Some(rel) = skills_path
        && !rel.is_empty()
    {
        let skills_dir = canonical.join(rel);
        if skills_dir.is_dir() {
            collect_tree_digests(&canonical, &skills_dir, skill_name, &mut records);
        }
    }
    for root_file in ["SKILL.md", "AGENTS.md", "CLAUDE.md"] {
        if records.len() >= MAX_TREE_DIGEST_FILES {
            break;
        }
        let path = canonical.join(root_file);
        if path.is_file()
            && let Ok(bytes) = std::fs::read(&path)
        {
            records.push((root_file.to_string(), infra_core::query::sha256(&bytes)));
        }
    }
    records.sort();
    let mut buf = String::new();
    buf.push_str(&canonical.to_string_lossy());
    buf.push('\n');
    for (rel, digest) in records {
        buf.push_str(&rel);
        buf.push(':');
        buf.push_str(&digest);
        buf.push('\n');
    }
    infra_core::query::sha256(buf.as_bytes())
}

#[expect(
    clippy::too_many_arguments,
    reason = "Explicit query memo key constructor over platform, skill, tree digest, timeout, description, query, model and sample index."
)]
fn query_memo_key_with_digest(
    platform: &str,
    skill_name: &str,
    tree_digest: &str,
    timeout: Duration,
    description: &str,
    query: &str,
    model: Option<&str>,
    sample_index: usize,
) -> QueryMemoKey {
    let timeout_ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
    QueryMemoKey {
        platform: platform.to_string(),
        skill_name: skill_name.to_string(),
        tree_digest: tree_digest.to_string(),
        timeout_ms,
        description_sha256: infra_core::query::sha256(description.as_bytes()),
        query_sha256: infra_core::query::sha256(query.as_bytes()),
        model: model.unwrap_or("").to_string(),
        sample_index,
    }
}

#[cfg(test)]
#[expect(
    clippy::too_many_arguments,
    reason = "Explicit query memo key constructor over platform, project, skills path, skill name, timeout, description, query, model and sample index."
)]
fn query_memo_key(
    platform: &str,
    project: &Path,
    skills_path: Option<&str>,
    skill_name: &str,
    timeout: Duration,
    description: &str,
    query: &str,
    model: Option<&str>,
    sample_index: usize,
) -> QueryMemoKey {
    let tree_digest = skill_tree_digest(project, skills_path, skill_name);
    query_memo_key_with_digest(
        platform,
        skill_name,
        &tree_digest,
        timeout,
        description,
        query,
        model,
        sample_index,
    )
}

/// Static Gate-1-style pre-filters applied before probe execution or optimization rounds.
pub fn static_prefilter_description(name: &str, description: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(RuntimeError::invalid("Skill name must not be empty"));
    }
    let trimmed = description.trim();
    if trimmed.is_empty() {
        return Err(RuntimeError::invalid("Skill description must not be empty"));
    }
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(RuntimeError::invalid(format!(
            "Skill description exceeds {MAX_DESCRIPTION_CHARS} characters"
        )));
    }
    if let Some(label) = infra_core::domain::sensitive_label(description)? {
        return Err(RuntimeError::invalid(format!(
            "Skill description contains sensitive content ({label})"
        )));
    }
    let (_, masked_count) = infra_core::domain::mask_secrets(description)?;
    if masked_count > 0 {
        return Err(RuntimeError::invalid(
            "Skill description contains masked secret material",
        ));
    }
    Ok(())
}

/// Static pre-filter validating both the candidate description and evaluation queries.
pub fn static_prefilter(name: &str, description: &str, eval_set: &[Value]) -> Result<()> {
    static_prefilter_description(name, description)?;
    for item in eval_set {
        let Some(query) = item["query"].as_str() else {
            continue;
        };
        if let Some(label) = infra_core::domain::sensitive_label(query)? {
            return Err(RuntimeError::invalid(format!(
                "Eval query contains sensitive content ({label})"
            )));
        }
        let (_, masked_count) = infra_core::domain::mask_secrets(query)?;
        if masked_count > 0 {
            return Err(RuntimeError::invalid(
                "Eval query contains masked secret material",
            ));
        }
    }
    Ok(())
}
struct ProbeFile(std::path::PathBuf);
impl Drop for ProbeFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
#[derive(Default)]
pub struct TriggerDetector {
    pending: bool,
    accumulated: String,
    pub observed: bool,
}
impl TriggerDetector {
    pub fn feed(&mut self, event: &Value, name: &str) -> Option<bool> {
        match event["type"].as_str().unwrap_or("") {
            "stream_event" => {
                let inner = &event["event"];
                match inner["type"].as_str().unwrap_or("") {
                    "content_block_start" => {
                        let block = &inner["content_block"];
                        if block["type"] == "tool_use" {
                            if matches!(block["name"].as_str(), Some("Skill" | "Read")) {
                                self.pending = true;
                                self.accumulated.clear();
                            } else {
                                return Some(false);
                            }
                        }
                    }
                    "content_block_delta" if self.pending => {
                        if inner["delta"]["type"] == "input_json_delta" {
                            self.accumulated
                                .push_str(inner["delta"]["partial_json"].as_str().unwrap_or(""));
                            if self.accumulated.contains(name) {
                                self.observed = true;
                                return Some(true);
                            }
                        }
                    }
                    "content_block_stop" | "message_stop" => {
                        if self.pending {
                            return Some(self.accumulated.contains(name));
                        }
                        if inner["type"] == "message_stop" {
                            return Some(false);
                        }
                    }
                    _ => {}
                }
            }
            "assistant" => {
                if let Some(content) = event["message"]["content"].as_array() {
                    for item in content {
                        if item["type"] != "tool_use" {
                            continue;
                        }
                        let input = &item["input"];
                        let found = match item["name"].as_str() {
                            Some("Skill") => {
                                input["skill"].as_str().is_some_and(|s| s.contains(name))
                            }
                            Some("Read") => input["file_path"]
                                .as_str()
                                .is_some_and(|s| s.contains(name)),
                            _ => false,
                        };
                        self.observed = found;
                        return Some(found);
                    }
                }
            }
            "result" => return Some(self.observed),
            _ => {}
        }
        None
    }
}
fn run_single_query_with_memo_in(
    cache: &Mutex<QueryMemoCache>,
    key: QueryMemoKey,
    name: &str,
    description: &str,
    exec: impl FnOnce() -> Result<bool>,
) -> Result<bool> {
    static_prefilter_description(name, description)?;
    if let Ok(mut guard) = cache.lock()
        && let Some(hit) = guard.get(&key)
    {
        return Ok(hit);
    }
    let result = exec()?;
    if let Ok(mut guard) = cache.lock() {
        guard.insert(key, result);
    }
    Ok(result)
}

fn run_single_query_with_memo(
    key: QueryMemoKey,
    name: &str,
    description: &str,
    exec: impl FnOnce() -> Result<bool>,
) -> Result<bool> {
    run_single_query_with_memo_in(query_memo(), key, name, description, exec)
}

#[expect(
    clippy::too_many_arguments,
    reason = "Preserve the public trigger-evaluation API with independently supplied platform, query, skill metadata, timeout, project, model and environment."
)]
pub fn run_single_query(
    config: &PlatformConfig,
    query: &str,
    name: &str,
    description: &str,
    timeout: Duration,
    project: &Path,
    model: Option<&str>,
    environment: &BTreeMap<String, String>,
) -> Result<bool> {
    static_prefilter_description(name, description)?;
    execute_single_query(
        config,
        query,
        name,
        description,
        timeout,
        project,
        model,
        environment,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "Preserve the public trigger-evaluation API with independently supplied platform, query, skill metadata, timeout, project, model and environment."
)]
fn execute_single_query(
    config: &PlatformConfig,
    query: &str,
    name: &str,
    description: &str,
    timeout: Duration,
    project: &Path,
    model: Option<&str>,
    environment: &BTreeMap<String, String>,
) -> Result<bool> {
    if config.key != "claude" {
        return platform_cli::evaluate_trigger(
            config,
            query,
            name,
            description,
            timeout,
            project,
            model,
            environment,
        );
    }
    let unique = uuid::Uuid::new_v4().simple().to_string();
    let clean = format!("{name}-skill-{}", &unique[..8]);
    let directory = project.join(config.skills_path);
    std::fs::create_dir_all(&directory)?;
    let file = ProbeFile(directory.join(format!("{clean}.md")));
    let indented = description.split('\n').collect::<Vec<_>>().join("\n  ");
    let content = format!(
        "---\ndescription: |\n  {indented}\n---\n\n# {name}\n\nThis skill handles: {description}\n"
    );
    use std::io::Write;
    let mut writer = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&file.0)?;
    writer.write_all(content.as_bytes())?;
    drop(writer);
    let mut env = environment.clone();
    env.remove("CLAUDECODE");
    let executable = platform_cli::executable(config, &env)?;
    let mut argv = vec![
        executable,
        "-p".into(),
        query.into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--include-partial-messages".into(),
    ];
    if let Some(m) = model {
        argv.extend(["--model".into(), m.into()]);
    }
    let mut detector = TriggerDetector::default();
    let mut selected = None;
    let output = common::invoke(argv, project, env, vec![], timeout, |event| {
        if let Some(hit) = detector.feed(event, &clean) {
            selected = Some(hit);
            Some("probe-observed".into())
        } else {
            None
        }
    })?;
    if output.stopped == "interrupted" {
        return Err(RuntimeError::unavailable("Trigger evaluation interrupted"));
    }
    Ok(selected.unwrap_or(false))
}
#[derive(Clone)]
pub struct EvalOptions {
    pub config: PlatformConfig,
    pub workers: usize,
    pub timeout: Duration,
    pub runs: usize,
    pub threshold: f64,
    pub model: Option<String>,
}
struct PreparedJobs<'a> {
    items: &'a [Value],
    query_order: Vec<&'a str>,
    query_items: BTreeMap<&'a str, &'a Value>,
    job_count: usize,
}

fn prepare_jobs<'a>(eval_set: &'a [Value], options: &EvalOptions) -> Result<PreparedJobs<'a>> {
    let native = options.config.key != "claude";
    if options.workers == 0 || options.runs == 0 {
        return Err(RuntimeError::invalid(
            "workers and runs_per_query must be positive",
        ));
    }
    if native && (!(0.0 < options.threshold && options.threshold <= 1.0) || eval_set.is_empty()) {
        return Err(RuntimeError::invalid(
            "Native evaluation requires a nonempty set and threshold greater than 0 and at most 1",
        ));
    }
    let mut query_order = vec![];
    let mut query_items = BTreeMap::new();
    for item in eval_set {
        let query = item["query"]
            .as_str()
            .ok_or_else(|| RuntimeError::invalid("Every eval item requires a string query"))?;
        if !item["should_trigger"].is_boolean() || native && query.trim().is_empty() {
            return Err(RuntimeError::invalid(
                "Every eval item requires a non-empty query and boolean should_trigger",
            ));
        }
        if native && query_items.contains_key(query) {
            return Err(RuntimeError::invalid(format!(
                "Duplicate eval query: {query}"
            )));
        }
        if native {
            query_order.push(query);
        }
        query_items.insert(query, item);
    }
    let job_count = eval_set
        .len()
        .checked_mul(options.runs)
        .filter(|count| *count <= 1_000_000)
        .ok_or_else(|| RuntimeError::invalid("Evaluation exceeds the bounded job limit"))?;
    Ok(PreparedJobs {
        items: eval_set,
        query_order,
        query_items,
        job_count,
    })
}

fn finish_eval(
    jobs: PreparedJobs<'_>,
    completed: Vec<(usize, Result<bool>)>,
    name: &str,
    description: &str,
    options: &EvalOptions,
    cancelled: bool,
    log: &mut String,
) -> Result<Value> {
    if cancelled {
        return Err(RuntimeError::unavailable("Trigger evaluation interrupted"));
    }
    let native = options.config.key != "claude";
    let PreparedJobs {
        items,
        mut query_order,
        mut query_items,
        ..
    } = jobs;
    let mut triggers: BTreeMap<&str, Vec<bool>> = BTreeMap::new();
    let mut first_error = None;
    for (index, result) in completed {
        let item = &items[index];
        let query = item["query"].as_str().expect("validated evaluation query");
        if !native && !triggers.contains_key(query) {
            query_order.push(query);
        }
        query_items.insert(query, item);
        let hit = match result {
            Ok(hit) => hit,
            Err(e) => {
                if native {
                    if first_error.is_none() {
                        first_error = Some(RuntimeError::unavailable(format!(
                            "Trigger probe failed for {query:?}: {e}"
                        )));
                    }
                    false
                } else {
                    log.push_str(&format!("Warning: query failed: {e}\n"));
                    false
                }
            }
        };
        triggers.entry(query).or_default().push(hit);
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    let mut results = vec![];
    for query in query_order {
        let values = triggers.remove(query).unwrap_or_default();
        if values.is_empty() {
            return Err(RuntimeError::unavailable(
                "Evaluation ended before every scheduled probe completed",
            ));
        }
        let count = values.iter().filter(|b| **b).count();
        let rate = count as f64 / values.len() as f64;
        let expected = query_items[query]["should_trigger"].as_bool().unwrap();
        let passed = if expected {
            rate >= options.threshold
        } else {
            rate < options.threshold
        };
        results.push(json!({"query":query,"should_trigger":expected,"trigger_rate":rate,"triggers":count,"runs":values.len(),"pass":passed}));
    }
    let passed = results.iter().filter(|r| r["pass"] == true).count();
    let mut output = json!({"skill_name":name,"description":description,"results":results,"summary":{"total":results.len(),"passed":passed,"failed":results.len()-passed}});
    if native {
        output["platform"] = json!(options.config.key);
    }
    Ok(output)
}

pub fn run_eval(
    eval_set: &[Value],
    name: &str,
    description: &str,
    project: &Path,
    options: &EvalOptions,
    log: &mut String,
) -> Result<Value> {
    static_prefilter(name, description, eval_set)?;
    let jobs = prepare_jobs(eval_set, options)?;
    let mut sample_counts: HashMap<&str, usize> = HashMap::new();
    let mut queue = VecDeque::with_capacity(jobs.job_count);
    for (index, item) in eval_set.iter().enumerate() {
        let query = item["query"].as_str().expect("validated evaluation query");
        let sample = sample_counts.entry(query).or_default();
        for _ in 0..options.runs {
            queue.push_back((index, *sample));
            *sample += 1;
        }
    }
    let queue = Arc::new(Mutex::new(queue));
    let (tx, rx) = mpsc::channel();
    let environment = common::env();
    let tree_digest = skill_tree_digest(project, Some(options.config.skills_path), name);
    let mut completed = vec![];
    std::thread::scope(|scope| {
        for _ in 0..options.workers.min(128) {
            let queue = queue.clone();
            let tx = tx.clone();
            let environment = environment.clone();
            let tree_digest = tree_digest.clone();
            scope.spawn(move || {
                loop {
                    if infra_eval::cancel::interrupted() {
                        break;
                    }
                    let job = queue.lock().unwrap().pop_front();
                    let Some((index, sample_index)) = job else {
                        break;
                    };
                    let query = eval_set[index]["query"]
                        .as_str()
                        .expect("validated evaluation query");
                    let key = query_memo_key_with_digest(
                        options.config.key,
                        name,
                        &tree_digest,
                        options.timeout,
                        description,
                        query,
                        options.model.as_deref(),
                        sample_index,
                    );
                    let result = run_single_query_with_memo(key, name, description, || {
                        execute_single_query(
                            &options.config,
                            query,
                            name,
                            description,
                            options.timeout,
                            project,
                            options.model.as_deref(),
                            &environment,
                        )
                    });
                    if tx.send((index, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        for result in rx {
            completed.push(result);
        }
    });
    finish_eval(
        jobs,
        completed,
        name,
        description,
        options,
        infra_eval::cancel::interrupted(),
        log,
    )
}
pub fn options(v: &Value) -> Result<EvalOptions> {
    let config = platform_cli::platform_config(v)?;
    let native = config.key != "claude";
    let workers = common::positive(v, "num_workers", if native { 4 } else { 10 })?;
    let runs = common::positive(v, "runs_per_query", 3)?;
    let threshold = common::number(v, "trigger_threshold", 0.5)?;
    if native && !(0.0 < threshold && threshold <= 1.0) {
        return Err(RuntimeError::invalid(
            "trigger_threshold must be greater than 0 and at most 1",
        ));
    }
    Ok(EvalOptions {
        config,
        workers,
        timeout: Duration::from_secs(
            common::positive(v, "timeout", if native { 60 } else { 30 })? as u64,
        ),
        runs,
        threshold,
        model: common::text(v, "model").map(str::to_owned),
    })
}
pub fn stats(label: &str, results: &[Value], elapsed: f64) -> String {
    let (mut tp, mut tn, mut fp, mut fn_) = (0u64, 0u64, 0u64, 0u64);
    for r in results {
        let runs = r["runs"].as_u64().unwrap_or(0);
        let triggers = r["triggers"].as_u64().unwrap_or(0);
        if r["should_trigger"] == true {
            tp += triggers;
            fn_ += runs.saturating_sub(triggers);
        } else {
            fp += triggers;
            tn += runs.saturating_sub(triggers);
        }
    }
    let total = tp + tn + fp + fn_;
    let precision = if tp + fp > 0 {
        tp as f64 / (tp + fp) as f64
    } else {
        1.0
    };
    let recall = if tp + fn_ > 0 {
        tp as f64 / (tp + fn_) as f64
    } else {
        1.0
    };
    let accuracy = if total > 0 {
        (tp + tn) as f64 / total as f64
    } else {
        0.0
    };
    let mut out = format!(
        "{label}: {}/{total} correct, precision={:.0}% recall={:.0}% accuracy={:.0}% ({elapsed:.1}s)\n",
        tp + tn,
        precision * 100.0,
        recall * 100.0,
        accuracy * 100.0
    );
    for r in results {
        out.push_str(&format!(
            "  [{}] rate={}/{} expected={}: {}\n",
            if r["pass"] == true { "PASS" } else { "FAIL" },
            r["triggers"],
            r["runs"],
            r["should_trigger"],
            common::chars(r["query"].as_str().unwrap_or(""), 60)
        ));
    }
    out
}
pub fn execute(v: &Value) -> Result<Value> {
    let skill = utils::parse_skill_md(Path::new(common::required(v, "skill_path")?))?;
    let eval = common::json_file(Path::new(common::required(v, "eval_set")?))?;
    let list = eval
        .as_array()
        .ok_or_else(|| RuntimeError::invalid("Eval set must be a JSON array"))?;
    let options = options(v)?;
    let project = platform_cli::find_project_root(
        &options.config,
        Path::new(common::text(v, "project_root").unwrap_or(".")),
    )?;
    let description = common::text(v, "description")
        .filter(|s| !s.is_empty())
        .unwrap_or(&skill.description);
    let mut log = String::new();
    if common::flag(v, "verbose") {
        log.push_str(&format!("Testing description: {description}\n"));
    }
    let output = run_eval(list, &skill.name, description, &project, &options, &mut log)?;
    if common::flag(v, "verbose") {
        log.push_str(&format!(
            "Summary: {}/{} passed\n",
            output["summary"]["passed"], output["summary"]["total"]
        ));
        log.push_str(&stats("Eval", output["results"].as_array().unwrap(), 0.0));
    }
    common::json_cli(output, log)
}

#[cfg(test)]
mod job_tests {
    use super::*;

    fn config(platform: &str, runs: usize) -> EvalOptions {
        EvalOptions {
            config: platform_cli::platform_config(&json!({"platform":platform})).unwrap(),
            workers: 2,
            timeout: Duration::from_secs(1),
            runs,
            threshold: 0.5,
            model: None,
        }
    }

    #[test]
    fn job_limit_is_checked_without_expanding_jobs_and_keeps_validation_priority() {
        let items = [json!({"query":"first","should_trigger":true})];
        assert_eq!(
            prepare_jobs(&items, &config("codex", 1_000_000))
                .unwrap()
                .job_count,
            1_000_000
        );
        let error = prepare_jobs(&items, &config("codex", 1_000_001))
            .err()
            .unwrap();
        assert_eq!(error.message, "Evaluation exceeds the bounded job limit");
        let two = [
            items[0].clone(),
            json!({"query":"second","should_trigger":false}),
        ];
        assert_eq!(
            prepare_jobs(&two, &config("codex", usize::MAX))
                .err()
                .unwrap()
                .message,
            error.message
        );
        let duplicate = [items[0].clone(), items[0].clone()];
        assert_eq!(
            prepare_jobs(&duplicate, &config("codex", usize::MAX))
                .err()
                .unwrap()
                .message,
            "Duplicate eval query: first"
        );
        let invalid = [
            items[0].clone(),
            json!({"query":false,"should_trigger":true}),
        ];
        assert_eq!(
            prepare_jobs(&invalid, &config("codex", usize::MAX))
                .err()
                .unwrap()
                .message,
            "Every eval item requires a string query"
        );
        for (workers, runs) in [(0, 1), (1, 0)] {
            let mut options = config("codex", runs);
            options.workers = workers;
            assert_eq!(
                prepare_jobs(&items, &options).err().unwrap().message,
                "workers and runs_per_query must be positive"
            );
        }
    }

    #[test]
    fn native_results_keep_input_order_and_first_receiver_error() {
        let items = [
            json!({"query":"first","should_trigger":true}),
            json!({"query":"second","should_trigger":false}),
        ];
        let options = config("codex", 1);
        let mut log = String::from("seed\n");
        let result = finish_eval(
            prepare_jobs(&items, &options).unwrap(),
            vec![(1, Ok(false)), (0, Ok(true))],
            "name",
            "description",
            &options,
            false,
            &mut log,
        )
        .unwrap();
        assert_eq!(result["results"][0]["query"], "first");
        assert_eq!(result["results"][1]["query"], "second");
        assert_eq!(result["summary"], json!({"total":2,"passed":2,"failed":0}));
        assert_eq!(result["platform"], "codex");
        let error = finish_eval(
            prepare_jobs(&items, &options).unwrap(),
            vec![
                (1, Err(RuntimeError::unavailable("second error"))),
                (0, Err(RuntimeError::unavailable("first error"))),
            ],
            "name",
            "description",
            &options,
            false,
            &mut log,
        )
        .unwrap_err();
        assert_eq!(
            error.message,
            "Trigger probe failed for \"second\": second error"
        );
        assert_eq!(log, "seed\n");
    }

    #[test]
    fn legacy_duplicates_use_receiver_order_and_last_completed_item() {
        let items = [
            json!({"query":"duplicate","should_trigger":true}),
            json!({"query":"other","should_trigger":false}),
            json!({"query":"duplicate","should_trigger":false}),
        ];
        let options = config("claude", 1);
        let mut log = String::new();
        let result = finish_eval(
            prepare_jobs(&items, &options).unwrap(),
            vec![(2, Ok(false)), (1, Ok(false)), (0, Ok(true))],
            "name",
            "description",
            &options,
            false,
            &mut log,
        )
        .unwrap();
        assert_eq!(
            result["results"],
            json!([
                {"query":"duplicate","should_trigger":true,"trigger_rate":0.5,"triggers":1,"runs":2,"pass":true},
                {"query":"other","should_trigger":false,"trigger_rate":0.0,"triggers":0,"runs":1,"pass":true}
            ])
        );
        assert!(result.get("platform").is_none());
        assert!(log.is_empty());
    }

    #[test]
    fn warnings_follow_receiver_order_but_cancellation_does_not_change_log() {
        let items = [
            json!({"query":"first","should_trigger":false}),
            json!({"query":"second","should_trigger":false}),
        ];
        let options = config("claude", 1);
        let completions = || {
            vec![
                (1, Err(RuntimeError::unavailable("second error"))),
                (0, Err(RuntimeError::unavailable("first error"))),
            ]
        };
        let mut log = String::from("seed\n");
        let error = finish_eval(
            prepare_jobs(&items, &options).unwrap(),
            completions(),
            "name",
            "description",
            &options,
            true,
            &mut log,
        )
        .unwrap_err();
        assert_eq!(error.message, "Trigger evaluation interrupted");
        assert_eq!(log, "seed\n");
        let result = finish_eval(
            prepare_jobs(&items, &options).unwrap(),
            completions(),
            "name",
            "description",
            &options,
            false,
            &mut log,
        )
        .unwrap();
        assert_eq!(
            log,
            "seed\nWarning: query failed: second error\nWarning: query failed: first error\n"
        );
        assert_eq!(result["results"][0]["query"], "second");
        assert_eq!(result["summary"]["passed"], 2);
    }

    #[test]
    fn query_memoization_reuses_result_by_description_query_and_model() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = AtomicUsize::new(0);
        let project = Path::new("/tmp/memo-project-a");
        let desc = "memo-test-desc-v1";
        let q = "memo-test-query-alpha";
        let timeout = Duration::from_secs(30);
        let first = run_single_query_with_memo(
            query_memo_key(
                "claude",
                project,
                Some(".claude/skills"),
                "memo-skill",
                timeout,
                desc,
                q,
                Some("sonnet"),
                0,
            ),
            "memo-skill",
            desc,
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(true)
            },
        )
        .unwrap();
        let second = run_single_query_with_memo(
            query_memo_key(
                "claude",
                project,
                Some(".claude/skills"),
                "memo-skill",
                timeout,
                desc,
                q,
                Some("sonnet"),
                0,
            ),
            "memo-skill",
            desc,
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(false)
            },
        )
        .unwrap();
        let next_sample = run_single_query_with_memo(
            query_memo_key(
                "claude",
                project,
                Some(".claude/skills"),
                "memo-skill",
                timeout,
                desc,
                q,
                Some("sonnet"),
                1,
            ),
            "memo-skill",
            desc,
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(false)
            },
        )
        .unwrap();
        let diff_model = run_single_query_with_memo(
            query_memo_key(
                "claude",
                project,
                Some(".claude/skills"),
                "memo-skill",
                timeout,
                desc,
                q,
                Some("opus"),
                0,
            ),
            "memo-skill",
            desc,
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(false)
            },
        )
        .unwrap();
        let diff_desc = run_single_query_with_memo(
            query_memo_key(
                "claude",
                project,
                Some(".claude/skills"),
                "memo-skill",
                timeout,
                "memo-test-desc-v2",
                q,
                Some("sonnet"),
                0,
            ),
            "memo-skill",
            "memo-test-desc-v2",
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(false)
            },
        )
        .unwrap();
        assert!(first);
        assert!(second);
        assert!(!next_sample);
        assert!(!diff_model);
        assert!(!diff_desc);
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn query_memo_persists_in_sqlite_and_invalidates_on_skill_tree_or_timeout() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("memo.sqlite3");
        let project = dir.path().join("workspace");
        let skills_dir = project.join(".claude/skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(skills_dir.join("alpha.md"), "# alpha v1").unwrap();

        let desc = "persist-desc";
        let query = "check migrations";
        let t1 = Duration::from_secs(30);
        let t2 = Duration::from_secs(60);

        let calls = AtomicUsize::new(0);
        {
            let store = Mutex::new(QueryMemoCache::open(&db_path).unwrap());
            let k1 = query_memo_key(
                "claude",
                &project,
                Some(".claude/skills"),
                "alpha",
                t1,
                desc,
                query,
                Some("sonnet"),
                0,
            );
            let out = run_single_query_with_memo_in(&store, k1, "alpha", desc, || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(true)
            })
            .unwrap();
            assert!(out);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }

        // Reopen from disk to prove durability across process/cache restarts.
        let store_reopened = Mutex::new(QueryMemoCache::open(&db_path).unwrap());
        let same_key = query_memo_key(
            "claude",
            &project,
            Some(".claude/skills"),
            "alpha",
            t1,
            desc,
            query,
            Some("sonnet"),
            0,
        );
        let hit = run_single_query_with_memo_in(&store_reopened, same_key, "alpha", desc, || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(false)
        })
        .unwrap();
        assert!(hit);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Changing skill_name misses.
        let diff_skill = query_memo_key(
            "claude",
            &project,
            Some(".claude/skills"),
            "beta",
            t1,
            desc,
            query,
            Some("sonnet"),
            0,
        );
        let out_skill =
            run_single_query_with_memo_in(&store_reopened, diff_skill, "beta", desc, || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(false)
            })
            .unwrap();
        assert!(!out_skill);
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        // Changing timeout misses.
        let diff_timeout = query_memo_key(
            "claude",
            &project,
            Some(".claude/skills"),
            "alpha",
            t2,
            desc,
            query,
            Some("sonnet"),
            0,
        );
        let out_timeout =
            run_single_query_with_memo_in(&store_reopened, diff_timeout, "alpha", desc, || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(false)
            })
            .unwrap();
        assert!(!out_timeout);
        assert_eq!(calls.load(Ordering::SeqCst), 3);

        // Editing a skill file alters tree_digest and invalidates persisted cache.
        std::fs::write(skills_dir.join("alpha.md"), "# alpha v2").unwrap();
        let diff_tree = query_memo_key(
            "claude",
            &project,
            Some(".claude/skills"),
            "alpha",
            t1,
            desc,
            query,
            Some("sonnet"),
            0,
        );
        let out_tree =
            run_single_query_with_memo_in(&store_reopened, diff_tree, "alpha", desc, || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(false)
            })
            .unwrap();
        assert!(!out_tree);
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn static_prefilter_rejects_empty_oversize_and_secret_inputs() {
        let valid = vec![json!({"query": "check migrations", "should_trigger": true})];
        assert!(static_prefilter("skill", "", &valid).is_err());
        let oversize = "a".repeat(MAX_DESCRIPTION_CHARS + 1);
        assert!(static_prefilter("skill", &oversize, &valid).is_err());
        let with_secret = "Use token ghp_1234567890abcdefghij1234567890abcdef";
        assert!(static_prefilter("skill", with_secret, &valid).is_err());
        assert!(static_prefilter("skill", "Clean skill description", &valid).is_ok());
    }
}
