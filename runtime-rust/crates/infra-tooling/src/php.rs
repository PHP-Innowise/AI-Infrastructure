use crate::{common, process, temp::Temporary};
use infra_core::{Result, RuntimeError, fs as safe};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    cell::Cell,
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

const PHP_RECEIPT_SCHEMA_VERSION: u32 = 1;
const MAX_PHP_RECEIPT_ENTRIES: usize = 4096;
const MAX_PHP_RECEIPT_BYTES: u64 = 8 * 1024 * 1024;

std::thread_local! {
    static PHP_CACHE_BYPASS_DEPTH: Cell<usize> = const { Cell::new(0) };
}

struct BypassGuard;
impl Drop for BypassGuard {
    fn drop(&mut self) {
        PHP_CACHE_BYPASS_DEPTH.with(|depth| {
            depth.set(depth.get().saturating_sub(1));
        });
    }
}

pub(crate) fn with_cache_bypass<T>(bypass: bool, f: impl FnOnce() -> T) -> T {
    if !bypass {
        return f();
    }
    PHP_CACHE_BYPASS_DEPTH.with(|depth| depth.set(depth.get().saturating_add(1)));
    let _guard = BypassGuard;
    f()
}

fn is_cache_bypassed() -> bool {
    PHP_CACHE_BYPASS_DEPTH.with(|depth| depth.get() > 0)
        || std::env::var("INFRA_CHECK_NO_CACHE").as_deref() == Ok("1")
        || std::env::var("INFRA_NO_CACHE").as_deref() == Ok("1")
        || std::env::var("INFRA_TOOLING_NO_CACHE").as_deref() == Ok("1")
        || std::env::var("INFRA_PHP_NO_CACHE").as_deref() == Ok("1")
}

fn php_block_regex() -> &'static regex::Regex {
    static BLOCK_RE: OnceLock<regex::Regex> = OnceLock::new();
    BLOCK_RE.get_or_init(|| {
        regex::Regex::new(r"(?ms)^```php([^\n]*)\n(.*?)^```")
            .expect("valid PHP fence regular expression")
    })
}

#[derive(Clone, Debug)]
pub struct CheckOptions {
    pub require_php: bool,
    pub use_cache: bool,
    pub cache_path: Option<PathBuf>,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self {
            require_php: false,
            use_cache: true,
            cache_path: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CheckOutcome {
    pub report: Value,
    pub php_version: Option<String>,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub subprocesses: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PhpReceiptEntry {
    php_version: String,
    mode: String,
    snippet_sha256: String,
    ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    problem: Option<String>,
    cached_at: String,
    last_used: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PhpReceiptStore {
    schema_version: u32,
    next_seq: u64,
    entries: BTreeMap<String, PhpReceiptEntry>,
}

impl Default for PhpReceiptStore {
    fn default() -> Self {
        Self {
            schema_version: PHP_RECEIPT_SCHEMA_VERSION,
            next_seq: 1,
            entries: BTreeMap::new(),
        }
    }
}

impl PhpReceiptStore {
    fn receipt_key(php_version: &str, mode: &str, snippet_sha256: &str) -> String {
        common::sha256(
            format!("{PHP_RECEIPT_SCHEMA_VERSION}\0{php_version}\0{mode}\0{snippet_sha256}")
                .as_bytes(),
        )
    }

    fn get(
        &mut self,
        php_version: &str,
        mode: &str,
        snippet_sha256: &str,
    ) -> Option<&PhpReceiptEntry> {
        let key = Self::receipt_key(php_version, mode, snippet_sha256);
        let matches = self.entries.get(&key).is_some_and(|entry| {
            entry.php_version == php_version
                && entry.mode == mode
                && entry.snippet_sha256 == snippet_sha256
        });
        if !matches {
            return None;
        }
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_used = seq;
        }
        self.entries.get(&key)
    }

    fn insert(
        &mut self,
        php_version: &str,
        mode: &str,
        snippet_sha256: &str,
        problem: Option<&str>,
    ) {
        let key = Self::receipt_key(php_version, mode, snippet_sha256);
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        self.entries.insert(
            key,
            PhpReceiptEntry {
                php_version: php_version.to_string(),
                mode: mode.to_string(),
                snippet_sha256: snippet_sha256.to_string(),
                ok: problem.is_none(),
                problem: problem.map(str::to_string),
                cached_at: infra_core::domain::utc_now(),
                last_used: seq,
            },
        );
        self.evict_excess();
    }

    fn evict_excess(&mut self) {
        if self.entries.len() <= MAX_PHP_RECEIPT_ENTRIES {
            return;
        }
        let mut by_age: Vec<(u64, String)> = self
            .entries
            .iter()
            .map(|(k, v)| (v.last_used, k.clone()))
            .collect();
        by_age.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let remove_count = self.entries.len().saturating_sub(MAX_PHP_RECEIPT_ENTRIES);
        for (_, key) in by_age.into_iter().take(remove_count) {
            self.entries.remove(&key);
        }
    }
}

fn default_receipt_path(root: &Path) -> PathBuf {
    infra_core::git::GitFrame::discover(root)
        .ok()
        .flatten()
        .map(|frame| frame.admin.join("infra-php-receipts.json"))
        .unwrap_or_else(|| {
            let canonical =
                infra_core::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
            std::env::temp_dir().join(format!(
                "infra-php-receipts-{}.json",
                common::sha256(canonical.to_string_lossy().as_bytes())
            ))
        })
}

fn load_receipt_store(path: &Path) -> PhpReceiptStore {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return PhpReceiptStore::default();
    };
    if !meta.is_file() || meta.len() > MAX_PHP_RECEIPT_BYTES {
        return PhpReceiptStore::default();
    }
    let Ok(bytes) = safe::read_regular(path) else {
        return PhpReceiptStore::default();
    };
    let Ok(mut store) = serde_json::from_slice::<PhpReceiptStore>(&bytes) else {
        return PhpReceiptStore::default();
    };
    if store.schema_version != PHP_RECEIPT_SCHEMA_VERSION {
        return PhpReceiptStore::default();
    }
    store.evict_excess();
    store
}

fn save_receipt_store(path: &Path, store: &PhpReceiptStore) {
    static SAVE_LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    let _guard = SAVE_LOCK.get_or_init(|| std::sync::Mutex::new(())).lock();
    if let Some(parent) = path.parent() {
        let _ = safe::ensure_directory(parent, true);
    }
    let mut merged = load_receipt_store(path);
    merged.next_seq = merged.next_seq.max(store.next_seq);
    for (key, entry) in &store.entries {
        merged
            .entries
            .entry(key.clone())
            .and_modify(|existing| {
                if entry.last_used > existing.last_used {
                    *existing = entry.clone();
                }
            })
            .or_insert_with(|| entry.clone());
    }
    merged.evict_excess();
    if let Ok(bytes) = serde_json::to_vec(&merged) {
        let _ = safe::atomic_write(path, &bytes);
    }
}

fn parse_php_version(stdout: &[u8]) -> String {
    let joined = String::from_utf8_lossy(stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    if joined.is_empty() {
        "unknown".into()
    } else {
        joined
    }
}

fn lint(root: &Path, file: &Path, source: &str) -> Result<Option<String>> {
    safe::atomic_write(file, source.as_bytes())?;
    let output = process::run(
        std::ffi::OsStr::new("php"),
        &[OsString::from("-l"), file.as_os_str().into()],
        root,
        None,
        Duration::from_secs(15),
        1024 * 1024,
    )?;
    if output.code == 0 {
        return Ok(None);
    }
    let joined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let message = joined
        .lines()
        .map(str::trim)
        .find(|line| line.contains("rror") && !line.contains("Errors parsing"))
        .or_else(|| joined.lines().map(str::trim).find(|line| !line.is_empty()))
        .unwrap_or("php -l failed");
    Ok(Some(
        message.replace(&file.to_string_lossy().to_string(), "<snippet>"),
    ))
}

#[allow(clippy::too_many_arguments)]
fn lint_cached(
    root: &Path,
    temp_slot: &mut Option<(Temporary, PathBuf)>,
    store: &mut PhpReceiptStore,
    use_cache: bool,
    php_version: &str,
    mode: &str,
    snippet_hash: &str,
    source_to_lint: &str,
    dirty: &mut bool,
    cache_hits: &mut usize,
    cache_misses: &mut usize,
    subprocesses: &mut usize,
) -> Result<Option<String>> {
    if use_cache && let Some(entry) = store.get(php_version, mode, snippet_hash) {
        *cache_hits += 1;
        return Ok(if entry.ok {
            None
        } else {
            entry.problem.clone()
        });
    }
    *cache_misses += 1;
    let file = match temp_slot {
        Some((_, path)) => path.as_path(),
        None => {
            let temporary = Temporary::new("infra-php-snippets")?;
            let path = temporary.path().join("snippet.php");
            temp_slot.insert((temporary, path)).1.as_path()
        }
    };
    *subprocesses += 1;
    let result = lint(root, file, source_to_lint)?;
    if use_cache {
        store.insert(php_version, mode, snippet_hash, result.as_deref());
        *dirty = true;
    }
    Ok(result)
}

pub fn check(root: &Path, require_php: bool) -> Result<Value> {
    check_with_options(
        root,
        CheckOptions {
            require_php,
            use_cache: true,
            cache_path: None,
        },
    )
    .map(|outcome| outcome.report)
}

pub fn check_with_options(root: &Path, options: CheckOptions) -> Result<CheckOutcome> {
    let probe = process::run(
        std::ffi::OsStr::new("php"),
        &["--version".into()],
        root,
        None,
        Duration::from_secs(5),
        65536,
    );
    let subprocess_probe = 1;
    let Some(probe_output) = probe.ok().filter(|value| value.code == 0) else {
        if options.require_php {
            return Err(RuntimeError::unavailable(
                "PHP is unavailable; no snippet was checked",
            ));
        }
        return Ok(CheckOutcome {
            report: json!({"status":"skipped","reason":"PHP unavailable","checked":0}),
            php_version: None,
            cache_hits: 0,
            cache_misses: 0,
            subprocesses: subprocess_probe,
        });
    };
    let php_version = parse_php_version(&probe_output.stdout);
    let effective_cache = options.use_cache && !is_cache_bypassed();
    let cache_path = effective_cache.then(|| {
        options
            .cache_path
            .clone()
            .unwrap_or_else(|| default_receipt_path(root))
    });
    let mut store = cache_path
        .as_deref()
        .map(load_receipt_store)
        .unwrap_or_default();
    let mut dirty = false;
    let mut temp_slot: Option<(Temporary, PathBuf)> = None;
    let mut cache_hits = 0usize;
    let mut cache_misses = 0usize;
    let mut subprocesses = subprocess_probe;

    let block = php_block_regex();
    let files = common::tracked(root, &["*.md".into(), ":(glob,exclude)**/Task/**".into()])?;
    let mut modes = BTreeMap::<String, usize>::new();
    let mut failures = Vec::new();
    for name in files {
        let path = common::under(root, &name)?;
        if !path.is_file() {
            continue;
        }
        let text = safe::read_text(&path)?;
        for (index, capture) in block.captures_iter(&text).enumerate() {
            let source = &capture[2];
            if capture[1].split_whitespace().any(|word| word == "fragment") {
                *modes.entry("declared fragment".into()).or_default() += 1;
                continue;
            }
            let snippet_hash = common::sha256(source.as_bytes());
            if source.trim_start().starts_with("<?php") {
                if let Some(problem) = lint_cached(
                    root,
                    &mut temp_slot,
                    &mut store,
                    effective_cache,
                    &php_version,
                    "complete",
                    &snippet_hash,
                    source,
                    &mut dirty,
                    &mut cache_hits,
                    &mut cache_misses,
                    &mut subprocesses,
                )? {
                    failures.push(json!({"path":name,"block":index+1,"problem":problem}));
                } else {
                    *modes.entry("complete".into()).or_default() += 1;
                }
                continue;
            }
            let mut accepted = false;
            let mut first = None;
            for (mode, shape) in [
                ("statements", format!("<?php\n{source}\n")),
                (
                    "class members",
                    format!("<?php\nclass __Snippet\n{{\n{source}\n}}\n"),
                ),
            ] {
                match lint_cached(
                    root,
                    &mut temp_slot,
                    &mut store,
                    effective_cache,
                    &php_version,
                    mode,
                    &snippet_hash,
                    &shape,
                    &mut dirty,
                    &mut cache_hits,
                    &mut cache_misses,
                    &mut subprocesses,
                )? {
                    None => {
                        *modes.entry(mode.into()).or_default() += 1;
                        accepted = true;
                        break;
                    }
                    Some(problem) => {
                        if first.is_none() {
                            first = Some(problem);
                        }
                    }
                }
            }
            if !accepted
                && (source.contains("<?php") || source.contains("<?="))
                && lint_cached(
                    root,
                    &mut temp_slot,
                    &mut store,
                    effective_cache,
                    &php_version,
                    "template",
                    &snippet_hash,
                    source,
                    &mut dirty,
                    &mut cache_hits,
                    &mut cache_misses,
                    &mut subprocesses,
                )?
                .is_none()
            {
                *modes.entry("template".into()).or_default() += 1;
                accepted = true;
            }
            if !accepted {
                failures.push(json!({"path":name,"block":index+1,"problem":first.unwrap_or_else(||"No complete PHP shape parses".into())}));
            }
        }
    }
    if effective_cache
        && dirty
        && let Some(path) = cache_path.as_deref()
    {
        save_receipt_store(path, &store);
    }
    Ok(CheckOutcome {
        report: json!({"status":if failures.is_empty(){"pass"}else{"fail"},"passed":failures.is_empty(),"counts":modes,"findings":failures}),
        php_version: Some(php_version),
        cache_hits,
        cache_misses,
        subprocesses,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_repo() -> Temporary {
        let temp = Temporary::new("infra-php-receipts-test").unwrap();
        common::git_ok(
            temp.path(),
            &["-c", "init.templateDir=", "init", "--quiet"],
            None,
        )
        .unwrap();
        temp
    }

    fn commit_md(root: &Path, name: &str, body: &str) {
        let path = root.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, body.as_bytes()).unwrap();
        common::git_ok(root, &["add", "--", name], None).unwrap();
    }

    #[test]
    fn receipt_store_keys_by_version_mode_snippet_and_evicts_boundedly() {
        let temp = Temporary::new("infra-php-store-unit").unwrap();
        let cache_file = temp.path().join("php-receipts.json");
        let mut store = PhpReceiptStore::default();
        let h1 = common::sha256(b"echo 1;");
        let h2 = common::sha256(b"echo 2;");
        store.insert("PHP 8.3.1", "statements", &h1, None);
        store.insert("PHP 8.3.1", "complete", &h1, Some("syntax error"));
        assert!(
            store
                .get("PHP 8.3.1", "statements", &h1)
                .is_some_and(|e| e.ok)
        );
        assert!(
            store
                .get("PHP 8.3.1", "complete", &h1)
                .is_some_and(|e| !e.ok && e.problem.as_deref() == Some("syntax error"))
        );
        // Different PHP version or different snippet hash must miss.
        assert!(store.get("PHP 8.4.0", "statements", &h1).is_none());
        assert!(store.get("PHP 8.3.1", "statements", &h2).is_none());

        // Bounded eviction keeps capacity <= MAX_PHP_RECEIPT_ENTRIES and preserves recent entries.
        for i in 0..(MAX_PHP_RECEIPT_ENTRIES + 25) {
            let h = common::sha256(format!("snippet-{i}").as_bytes());
            store.insert("PHP 8.3.1", "statements", &h, None);
        }
        assert_eq!(store.entries.len(), MAX_PHP_RECEIPT_ENTRIES);
        save_receipt_store(&cache_file, &store);
        let reloaded = load_receipt_store(&cache_file);
        assert_eq!(reloaded.entries.len(), MAX_PHP_RECEIPT_ENTRIES);
        let newest = common::sha256(format!("snippet-{}", MAX_PHP_RECEIPT_ENTRIES + 24).as_bytes());
        assert!(reloaded.entries.contains_key(&PhpReceiptStore::receipt_key(
            "PHP 8.3.1",
            "statements",
            &newest
        )));
    }

    #[test]
    fn php_snippet_receipt_cache_hits_and_respects_bypass_and_edits() {
        let probe = process::run(
            std::ffi::OsStr::new("php"),
            &["--version".into()],
            Path::new("."),
            None,
            Duration::from_secs(5),
            65536,
        );
        if !probe.is_ok_and(|o| o.code == 0) {
            return;
        }
        let repo = git_repo();
        let root = repo.path();
        let cache_dir = Temporary::new("infra-php-cache-dir").unwrap();
        let cache_file = cache_dir.path().join("receipts.json");

        commit_md(
            root,
            "docs/guide.md",
            concat!(
                "# PHP Examples\n\n",
                "```php\n<?php\necho 'complete';\n```\n\n",
                "```php\n$x = 1 + 2;\n```\n\n",
                "```php\npublic function id(): int { return 1; }\n```\n\n",
                "```php fragment\n->where('a', 1)\n```\n",
            ),
        );

        // Cold run: populates cache across complete + statements + class members (fails statements first).
        let cold = check_with_options(
            root,
            CheckOptions {
                require_php: true,
                use_cache: true,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert_eq!(cold.report["passed"], true, "{}", cold.report);
        assert_eq!(cold.cache_hits, 0);
        assert_eq!(cold.cache_misses, 4); // complete (1) + statements (1) + statements->class members (2)
        assert_eq!(cold.subprocesses, 5); // 1 --version + 4 php -l calls
        assert!(cache_file.is_file());

        // Warm run: 0 php -l subprocesses spawned, identical report output.
        let warm = check_with_options(
            root,
            CheckOptions {
                require_php: true,
                use_cache: true,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert_eq!(warm.report, cold.report);
        assert_eq!(warm.cache_hits, 4);
        assert_eq!(warm.cache_misses, 0);
        assert_eq!(warm.subprocesses, 1); // only php --version

        // Explicit no-cache & thread-local strict bypass both run php -l live.
        let uncached = check_with_options(
            root,
            CheckOptions {
                require_php: true,
                use_cache: false,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert_eq!(uncached.report, cold.report);
        assert_eq!(uncached.cache_hits, 0);
        assert_eq!(uncached.cache_misses, 4);
        assert_eq!(uncached.subprocesses, 5);

        let bypassed = with_cache_bypass(true, || check(root, true)).unwrap();
        assert_eq!(bypassed, cold.report);

        // Modifying one snippet to fail invalidates only that snippet; passing snippets still hit cache.
        commit_md(
            root,
            "docs/guide.md",
            concat!(
                "# PHP Examples\n\n",
                "```php\n<?php\necho 'complete';\n```\n\n",
                "```php\n$x = 1 + ;\n```\n\n",
                "```php\npublic function id(): int { return 1; }\n```\n\n",
                "```php fragment\n->where('a', 1)\n```\n",
            ),
        );
        let after_edit = check_with_options(
            root,
            CheckOptions {
                require_php: true,
                use_cache: true,
                cache_path: Some(cache_file.clone()),
            },
        )
        .unwrap();
        assert_eq!(after_edit.report["passed"], false);
        assert_eq!(after_edit.report["findings"].as_array().unwrap().len(), 1);
        assert_eq!(after_edit.cache_hits, 3); // complete (1) + class members (2)
        assert_eq!(after_edit.cache_misses, 2); // broken snippet tested statements + class members

        // Repeating check on unchanged failing file also avoids spawning php -l subprocesses.
        let warm_fail = check_with_options(
            root,
            CheckOptions {
                require_php: true,
                use_cache: true,
                cache_path: Some(cache_file),
            },
        )
        .unwrap();
        assert_eq!(warm_fail.report, after_edit.report);
        assert_eq!(warm_fail.cache_hits, 5);
        assert_eq!(warm_fail.cache_misses, 0);
        assert_eq!(warm_fail.subprocesses, 1);
    }
}
