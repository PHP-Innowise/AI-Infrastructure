//! Opt-in private miss tracing. Only supplied judgment names and fixed labels escape.
use crate::{Result, artifact, reconstruction, scoring};
use infra_core::{Roots, RuntimeConfig, RuntimeContext, Store, history, index, query};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};
fn unknown(paths: &[String], stage: &str) -> Value {
    json!({"status":"unknown","items":paths.iter().map(|p|json!({"path":p,"stage":stage,"answer":"unknown","answer_counts":{"eligible":0,"indexed":0,"delivered":0}})).collect::<Vec<_>>()})
}
fn flat(s: &str) -> String {
    query::folded(&scoring::flatten(s))
}
fn answer_flat(s: &str) -> String {
    let body = s
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    flat(if body.is_empty() { s } else { &body })
}
fn sources(root: &Path, path: &str) -> Vec<(String, PathBuf)> {
    let mut names = vec![path.to_string()];
    if let Some(tail) = path.strip_prefix(".agents/skills/") {
        for tool in [".claude/skills/", ".cursor/skills/", ".codex/skills/"] {
            names.push(format!("{tool}{tail}"));
        }
    }
    names
        .into_iter()
        .filter_map(|name| {
            let p = root.join(&name);
            artifact::read_file(&p, artifact::MAX_TREE)
                .ok()
                .map(|_| (name, p))
        })
        .collect()
}
fn reason(reason: &str) -> &'static str {
    match reason {
        "budget" | "hard-budget" => "budget-loss",
        "file-link-limit" | "path-link-limit" => "automatic-link-limit",
        "capsule-limit" => "capsule-loss",
        "layer-limit"
        | "episodic-layer"
        | "skill-subfile"
        | "host-listed"
        | "delivered-this-session" => "rank-loss",
        "privacy"
        | "owner"
        | "authority"
        | "lifecycle"
        | "stale"
        | "map-unverifiable"
        | "map-drift"
        | "source-missing"
        | "working-task"
        | "host-loaded"
        | "promoted-to-chunk"
        | "conflict-unavailable"
        | "ticket-mismatch"
        | "invalid"
        | "explicit-context-only" => "runtime-exclusion",
        _ => "unknown",
    }
}
fn lexical_stage(store: &Store, text: &str, path: &str) -> Result<&'static str> {
    let Some(doc) = index::load_document(&store.conn, path)? else {
        return Ok("not-indexed");
    };
    if doc.kind == "brain-event" {
        return Ok("unknown-history");
    }
    let tokens = query::informative_tokens(
        &store.conn,
        query::expand_query_tokens(&store.conn, query::query_tokens(text)?)?,
    )?;
    let evidence = query::evidence_tokens(&tokens);
    if evidence.is_empty() {
        return Ok("unknown");
    }
    let expression = query::fts_expression(&tokens);
    let hit=store.conn.query_row("SELECT count(*) FROM documents WHERE documents MATCH ?1 AND kind!='brain-event' AND path=?2",rusqlite::params![expression,path],|r|r.get::<_,usize>(0))?;
    if hit == 0 {
        return Ok("no-fts-candidate");
    }
    let mut statement=store.conn.prepare("SELECT d.path,d.layer,d.title,m.authority,m.confidence,m.updated_at,bm25(documents,1.0,1.0,1.0,2.0,8.0,1.0) AS score FROM documents d JOIN document_metadata m ON m.path=d.path WHERE documents MATCH ?1 AND d.kind!='brain-event' ORDER BY score,d.path LIMIT 30")?;
    let rows = statement
        .query_map([&expression], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, f64>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, f64>(6)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if !rows.iter().any(|r| r.0 == path) {
        return Ok("window-limit");
    }
    let total = store
        .conn
        .query_row("SELECT COUNT(*) FROM documents", [], |r| {
            r.get::<_, usize>(0)
        })?
        .max(1);
    let frequencies = query::frequencies(&store.conn, &evidence)?;
    let mut coverage = BTreeMap::<String, usize>::new();
    let mut rare = BTreeMap::<String, Vec<String>>::new();
    for token in &evidence {
        let mut q = store
            .conn
            .prepare("SELECT path FROM documents WHERE documents MATCH ?1")?;
        let names = q
            .query_map([query::fts_expression(std::slice::from_ref(token))], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let distinctive = names.len() as f64 <= (total as f64 * 0.1).max(1.0);
        for p in names {
            *coverage.entry(p.clone()).or_default() += 1;
            if distinctive {
                rare.entry(p).or_default().push(token.clone());
            }
        }
    }
    let minimum = if evidence.len() >= 2 { 2 } else { 1 };
    let mut accepted = vec![];
    for row in &rows {
        let words = query::word_tokens(&format!("{} {}", row.0, row.2));
        let relevant = coverage.get(&row.0).copied().unwrap_or(0) >= minimum
            || rare.get(&row.0).is_some_and(|ts| {
                ts.iter().any(|t| {
                    row.1 == "procedural"
                        || ((frequencies.get(t).copied().unwrap_or(0) <= 3
                            || t.chars().any(char::is_numeric))
                            && query::is_anchor(t, &words))
                })
            });
        if relevant {
            accepted.push((
                row.0.clone(),
                row.1.clone(),
                (-row.6).max(0.0) * query::ranking_weight(&row.3, row.4, &row.5),
            ));
        }
    }
    let Some(target) = accepted.iter().find(|r| r.0 == path) else {
        return Ok("relevance-drop");
    };
    let best = accepted
        .iter()
        .filter(|r| r.1 == target.1)
        .map(|r| r.2)
        .fold(0.0, f64::max);
    Ok(if target.2 < best * query::RELATIVE_SCORE_FLOOR {
        "floor-drop"
    } else {
        "unknown"
    })
}
fn history_stage(
    store: &Store,
    context: &RuntimeContext,
    path: &str,
    text: Option<&str>,
) -> Result<&'static str> {
    let Some(text) = text else {
        return Ok("unknown-history");
    };
    let screened = infra_core::domain::sanitize_automatic_query(text)?;
    let Some(q) = history::history_query(&store.conn, &screened)? else {
        return Ok("history-admission-drop");
    };
    let (events, episodes) = history::candidates(&store.conn, &q)?;
    if !events.iter().any(|e| e.document.path == path) {
        return Ok("history-admission-drop");
    }
    let config = RuntimeConfig::load(&context.roots)?;
    let ranked = history::best_events(context, &config, events, 1, &BTreeSet::new());
    if let Err(error) = &ranked {
        crate::process::record_custody_error(error);
    }
    crate::process::ensure_custody()?;
    let (winners, excluded) = match ranked {
        Ok(value) => value,
        Err(_) => return Ok("unknown-history"),
    };
    if excluded.iter().any(|e| e.path == path) {
        return Ok("runtime-exclusion");
    }
    let Some(winner) = winners.first() else {
        return Ok(if episodes.is_empty() {
            "history-admission-drop"
        } else {
            "history-event-limit"
        });
    };
    if episodes
        .first()
        .is_some_and(|e| e.score > winner.adjusted_score)
        || winner.document.path != path
    {
        return Ok("history-event-limit");
    }
    Ok("unknown-history")
}
struct Seed {
    root: PathBuf,
    created: bool,
}
impl Drop for Seed {
    fn drop(&mut self) {
        if self.created && !crate::process::custody_lost() {
            let _ = std::fs::remove_dir_all(self.root.join(".git"));
        }
    }
}
fn seed(root: &Path, at: Option<&str>) -> Result<Seed> {
    crate::process::ensure_custody()?;
    let mut state = Seed {
        root: root.into(),
        created: false,
    };
    if root.join(".git").exists() {
        return Ok(state);
    }
    let stamp = at
        .and_then(reconstruction::instant)
        .unwrap_or_else(chrono::Utc::now);
    let environment = artifact::Scratch::new(&std::env::temp_dir(), "miss-audit-git-env")?;
    let env = reconstruction::runtime_env(&environment.0, stamp, true);
    if let Err(error) = &env {
        crate::process::record_custody_error(error);
    }
    crate::process::ensure_custody()?;
    if let Ok(env) = env {
        let committed = reconstruction::commit_corpus(root, &env, None, None, "audit/snapshot");
        if let Err(error) = &committed {
            crate::process::record_custody_error(error);
        }
        crate::process::ensure_custody()?;
        if committed.is_ok() {
            state.created = true;
        } else {
            let _ = std::fs::remove_dir_all(root.join(".git"));
        }
    }
    Ok(state)
}
pub struct Audit<'a> {
    pub root: &'a Path,
    pub manifest: Option<&'a str>,
    pub paths: &'a [String],
    pub query_withheld: bool,
    pub passages: &'a BTreeMap<String, Vec<String>>,
    pub capsule: &'a str,
    pub history: Option<&'a str>,
    pub seed_git: bool,
}
pub fn audit(request: Audit<'_>) -> Result<Value> {
    crate::process::ensure_custody()?;
    let paths = request
        .paths
        .iter()
        .filter(|p| artifact::relative(p).is_ok() && !p.contains('\\'))
        .cloned()
        .collect::<Vec<_>>();
    if request.query_withheld {
        return Ok(unknown(&paths, "unknown-query-withheld"));
    }
    let run = (|| -> Result<Value> {
        let reference = request
            .manifest
            .ok_or_else(|| artifact::invalid("manifest-unavailable"))?;
        if !regex::Regex::new(r"^(?:memory-bank/local|project-brain/control)/retrieval-manifests/[0-9a-f-]{36}\.json$").map_err(|_|artifact::invalid("manifest expression"))?.is_match(reference){return Err(artifact::invalid("manifest-unavailable"));}
        let manifest = artifact::read_json(&request.root.join(reference), artifact::MAX_TREE)?;
        let context = RuntimeContext::new(Roots::installed(request.root)?, "local", "governed")?;
        infra_core::domain::validate_schema_file(
            &context.roots,
            "retrieval-manifest.schema.json",
            &manifest,
        )?;
        let text = manifest["query"]
            .as_str()
            .ok_or_else(|| artifact::invalid("manifest-invalid"))?;
        let store = artifact::open_database_read_only(
            &request.root.join("memory-bank/local/context.db"),
            512 * 1024 * 1024,
            Duration::from_secs(1),
        )?;
        let _seed = request
            .seed_git
            .then(|| seed(request.root, manifest["created_at"].as_str()))
            .transpose()?;
        let selected = manifest["selected"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v["path"].as_str())
            .collect::<BTreeSet<_>>();
        let excluded = manifest["excluded"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| Some((v["path"].as_str()?, v["reason"].as_str()?)))
            .collect::<BTreeMap<_, _>>();
        let mut items = vec![];
        for path in &paths {
            crate::process::ensure_custody()?;
            let item = (|| -> Result<Value> {
                let source = sources(request.root, path);
                let indexed = source
                    .iter()
                    .find(|(p, _)| {
                        index::load_document(&store.conn, p)
                            .ok()
                            .flatten()
                            .is_some()
                    })
                    .or_else(|| source.first())
                    .map(|r| r.0.as_str())
                    .unwrap_or(path);
                let doc = index::load_document(&store.conn, indexed)?;
                let mut stage = if source.is_empty() {
                    "absent-source"
                } else if doc.is_none() {
                    "not-indexed"
                } else if selected.contains(indexed) {
                    "selected"
                } else if let Some(r) = excluded.get(indexed) {
                    reason(r)
                } else {
                    lexical_stage(&store, text, indexed)?
                };
                if stage == "unknown-history" {
                    stage = history_stage(&store, &context, indexed, request.history)?;
                }
                let wanted = request
                    .passages
                    .get(path)
                    .into_iter()
                    .flatten()
                    .map(|s| flat(s))
                    .filter(|s| !s.is_empty())
                    .collect::<BTreeSet<_>>();
                let (mut eligible, mut indexed_count, mut delivered) = (0, 0, 0);
                let mut answer = if wanted.is_empty() {
                    "not-applicable"
                } else {
                    "unknown"
                };
                if let (Some((_, p)), Some(doc)) = (source.first(), doc.as_ref()) {
                    let (raw, _) = artifact::read_file(p, artifact::MAX_TREE)?;
                    let source = flat(&String::from_utf8_lossy(&raw));
                    let indexed = flat(&doc.content);
                    let body = answer_flat(request.capsule);
                    for passage in &wanted {
                        if source.contains(passage) {
                            eligible += 1;
                            if indexed.contains(passage) {
                                indexed_count += 1;
                                let aliases = request
                                    .passages
                                    .get(path)
                                    .into_iter()
                                    .flatten()
                                    .filter(|s| flat(s) == *passage)
                                    .map(|s| answer_flat(s));
                                if aliases
                                    .into_iter()
                                    .any(|p| !p.is_empty() && body.contains(&p))
                                {
                                    delivered += 1;
                                }
                            }
                        }
                    }
                    answer = if wanted.is_empty() {
                        "not-applicable"
                    } else if eligible == 0 {
                        "not-in-source"
                    } else if indexed_count == 0 {
                        "not-in-index"
                    } else if indexed_count < eligible {
                        "partial-index"
                    } else if delivered == 0 {
                        "not-in-delivery"
                    } else if delivered < eligible {
                        "partial-delivery"
                    } else {
                        "delivered"
                    };
                }
                if stage == "selected" && ["not-in-delivery", "partial-delivery"].contains(&answer)
                {
                    stage = "selected-answer-not-in-text";
                }
                Ok(
                    json!({"path":path,"stage":stage,"answer":answer,"answer_counts":{"eligible":eligible,"indexed":indexed_count,"delivered":delivered},"history":doc.is_some_and(|d|d.kind=="brain-event")}),
                )
            })();
            if let Err(error) = &item {
                crate::process::record_custody_error(error);
            }
            crate::process::ensure_custody()?;
            items.push(item.unwrap_or_else(|_|json!({"path":path,"stage":"unknown","answer":"unknown","answer_counts":{"eligible":0,"indexed":0,"delivered":0},"history":false})));
        }
        Ok(json!({"status":"ok","items":items}))
    })();
    if let Err(error) = &run {
        crate::process::record_custody_error(error);
    }
    crate::process::ensure_custody()?;
    Ok(run.unwrap_or_else(|_| unknown(&paths, "unknown")))
}

const LOSS: &[&str] = &[
    "term_coverage",
    "relative_score_floor",
    "budget",
    "hard_budget",
    "layer",
    "repeat",
    "gate",
    "pack",
    "authority",
    "lifecycle",
    "stale",
    "source_missing",
    "map_unverifiable",
    "map_drift",
];
const BOUNDARY: &[&str] = &[
    "queue_after_runtime",
    "budget_admitted",
    "layer_pre_novelty",
    "repeat_withheld",
    "gate_input",
    "gate_withheld",
    "pack_input",
    "pack_omitted",
    "selected_post_pack",
];
const RENDER: &[&str] = &["excerpt", "render-limit", "sanitized-empty", "no-excerpt"];
pub fn unavailable_funnel() -> Value {
    let counters = |keys: &[&str]| {
        Value::Object(
            keys.iter()
                .map(|k| (k.to_string(), json!({"count":null,"state":"unknown"})))
                .collect(),
        )
    };
    json!({"schema_version":1,"observation_state":"unavailable","source_coverage":"unknown","quality":"unmeasured","count_scope":"authorized-observation-occurrences","coverage":{"history_pre_admission":"not-instrumented","episodes":"not-instrumented","working_sources":"not-instrumented","path_only_events":"authorized-retained-records-only"},"loss_observations":counters(LOSS),"boundaries":counters(BOUNDARY),"render_projection":{"scope":"authorized-retained-records","headers":{"count":null,"state":"unknown"},"reasons":counters(RENDER)},"emission":"unknown","consumer":"unknown","next_action":"unknown"})
}
fn counter(v: &Value) -> Option<Value> {
    if !artifact::exact(v, &["count", "state"]) {
        return None;
    }
    if v["count"].is_null() && v["state"] == "unknown" {
        return Some(v.clone());
    }
    let n = v["count"].as_u64().filter(|n| *n <= 65535)?;
    if v["state"] != "exact" && !(v["state"] == "lower-bound" && n == 65535) {
        return None;
    }
    Some(json!({"count":n,"state":v["state"]}))
}
pub fn normalize_funnel(v: &Value) -> Option<Value> {
    let mut clean = unavailable_funnel();
    if v.as_object()?.keys().collect::<BTreeSet<_>>()
        != clean.as_object()?.keys().collect::<BTreeSet<_>>()
        || v["schema_version"] != 1
        || !["observed", "unavailable"].contains(&v["observation_state"].as_str()?)
        || !["unknown", "inspect-observed-loss"].contains(&v["next_action"].as_str()?)
    {
        return None;
    }
    for k in [
        "source_coverage",
        "quality",
        "count_scope",
        "coverage",
        "emission",
        "consumer",
    ] {
        if v[k] != clean[k] {
            return None;
        }
    }
    for (k, keys) in [("loss_observations", LOSS), ("boundaries", BOUNDARY)] {
        if !artifact::exact(&v[k], keys) {
            return None;
        }
        for key in keys {
            clean[k][*key] = counter(&v[k][*key])?;
        }
    }
    if !artifact::exact(&v["render_projection"], &["scope", "headers", "reasons"])
        || v["render_projection"]["scope"] != clean["render_projection"]["scope"]
        || !artifact::exact(&v["render_projection"]["reasons"], RENDER)
    {
        return None;
    }
    clean["render_projection"]["headers"] = counter(&v["render_projection"]["headers"])?;
    for k in RENDER {
        clean["render_projection"]["reasons"][*k] =
            counter(&v["render_projection"]["reasons"][*k])?;
    }
    if v["observation_state"] == "unavailable" {
        return (v == &unavailable_funnel()).then_some(clean);
    }
    let loss = LOSS.iter().any(|k| {
        clean["loss_observations"][*k]["count"]
            .as_u64()
            .is_some_and(|n| n > 0)
    });
    let action = if loss {
        "inspect-observed-loss"
    } else {
        "unknown"
    };
    if v["next_action"] != action {
        return None;
    }
    clean["observation_state"] = "observed".into();
    clean["next_action"] = action.into();
    Some(clean)
}
pub fn unavailable_receipt() -> Value {
    json!({"schema_version":1,"observation_state":"unavailable","plans_present":null,"returned_status":"unknown"})
}
pub fn normalize_receipt(v: &Value) -> Option<Value> {
    if !artifact::exact(
        v,
        &[
            "schema_version",
            "observation_state",
            "plans_present",
            "returned_status",
        ],
    ) || v["schema_version"] != 1
    {
        return None;
    }
    match v["observation_state"].as_str()? {
        "unavailable" => {
            if !v["plans_present"].is_null() || v["returned_status"] != "unknown" {
                return None;
            }
        }
        "observed" => {
            if !v["plans_present"].is_boolean()
                || !["skipped", "deferred", "committed"].contains(&v["returned_status"].as_str()?)
                || v["plans_present"] == false && v["returned_status"] != "skipped"
            {
                return None;
            }
        }
        _ => return None,
    }
    Some(v.clone())
}

#[cfg(test)]
mod custody_leaf_tests {
    use super::*;
    use crate::outcome::CorpusPreparation;
    use std::{
        fs::{self, File},
        io::Read,
        process::{Command, Stdio},
        time::Instant,
    };

    const CHILD: &str = "INFRA_EVAL_CUSTODY_LEAF_FIXTURE";
    const NAME: &str =
        "diagnostic::custody_leaf_tests::custody_fatal_is_not_flattened_or_cleaned_up";

    fn witness(error: crate::RuntimeError) {
        assert_eq!(error.kind, infra_core::ErrorKind::Unavailable);
        assert_eq!(error.code.as_deref(), Some("cleanup-unconfirmed"));
        assert_eq!(error.message, "isolated leaf first-cause witness");
        assert_eq!(error.os_code, Some(5));
    }

    #[test]
    fn custody_fatal_is_not_flattened_or_cleaned_up() {
        if let Some(root) = std::env::var_os(CHILD).map(PathBuf::from) {
            let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
            let cache = root.join("cache");
            let runtime = root.join(if cfg!(windows) {
                "selected-engine.exe"
            } else {
                "selected-engine"
            });
            let original = crate::executable::SelectedExecutableSource::capture(
                &std::env::current_exe().unwrap(),
            )
            .unwrap();
            artifact::fresh_file(&runtime, original.bytes(), 0o700).unwrap();
            drop(original);
            let mut preparation = crate::sampling::NativePreparation::new(
                reconstruction::Reconstructor::new(&repository, &cache, &runtime).unwrap(),
                root.join("projects"),
                root.join("claude"),
                root.join("codex"),
                Duration::from_secs(1),
            )
            .unwrap();
            let nested = preparation
                .with_engine(|_| preparation.with_engine(|_| Ok(())))
                .unwrap_err();
            assert_eq!(
                nested.code.as_deref(),
                Some("eval-native-engine-authority-borrowed")
            );
            let seed_root = root.join("seed");
            fs::create_dir(&seed_root).unwrap();
            fs::create_dir(seed_root.join(".git")).unwrap();
            let seeded = Seed {
                root: seed_root.clone(),
                created: true,
            };
            let mut first = crate::RuntimeError::coded(
                infra_core::ErrorKind::Unavailable,
                "cleanup-unconfirmed",
                "isolated leaf first-cause witness",
            );
            first.os_code = Some(5);
            let fatal: crate::Result<()> = preparation.with_engine(|_| {
                fs::remove_file(&runtime).unwrap();
                Err(first)
            });
            witness(fatal.unwrap_err());
            assert!(
                !runtime.exists(),
                "a later source recheck would now fail; the first custody fatal must still win"
            );
            let mut later = crate::RuntimeError::coded(
                infra_core::ErrorKind::Io,
                "cleanup-unconfirmed",
                "later cause must not replace first",
            );
            later.os_code = Some(13);
            crate::process::record_custody_error(&later);
            drop(seeded);
            assert!(
                seed_root.join(".git").is_dir(),
                "fatal custody must preserve owned seed state"
            );

            let paths = Vec::new();
            let passages = BTreeMap::new();
            witness(
                audit(Audit {
                    root: &seed_root,
                    manifest: None,
                    paths: &paths,
                    query_withheld: true,
                    passages: &passages,
                    capsule: "",
                    history: None,
                    seed_git: false,
                })
                .unwrap_err(),
            );
            witness(crate::control::eval_run(&json!({})).unwrap_err());

            let out = root.join("legacy-result.json");
            witness(
                crate::outcome::run_legacy(
                    &json!({"tasks":[]}),
                    &out,
                    &["recall".to_string()],
                    1,
                    1,
                    Duration::from_secs(1),
                    None,
                    "codex",
                    None,
                    false,
                    &mut preparation,
                )
                .unwrap_err(),
            );
            assert!(
                !out.exists(),
                "fatal entry must refuse before writing normal results"
            );
            let settings = crate::outcome::Settings {
                arms: vec!["off".into(), "recall".into()],
                repeat: 1,
                cap: 1,
                timeout: 1.0,
                model: "fixture".into(),
                codex_version: "not-called".into(),
                endpoint: "task-complete".into(),
                retry_policy: "no-retry".into(),
                git_history: "read-only".into(),
            };
            let absent = root.join("intentionally-unread-input");
            let completed = root.join("completion-result.json");
            witness(
                crate::outcome::run_complete(
                    crate::outcome::CompletionRun {
                        set: &absent,
                        verifiers: &absent,
                        protocol: &absent,
                        out: &completed,
                        cache: &cache,
                        private: &root.join("private"),
                        source: &repository,
                        codex: "codex",
                        executable: &runtime,
                        settings: &settings,
                    },
                    &preparation,
                )
                .unwrap_err(),
            );
            assert!(!completed.exists());
            assert!(seed_root.join(".git").is_dir());
            return;
        }

        crate::process::ensure_custody().unwrap();
        let root = artifact::Scratch::new(&std::env::temp_dir(), "custody-leaf").unwrap();
        let home = root.0.join("home");
        let temp = root.0.join("tmp");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&temp).unwrap();
        let stdout = root.0.join("child.stdout");
        let stderr = root.0.join("child.stderr");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", NAME, "--nocapture", "--test-threads=1"])
            .env_clear();
        for (key, value) in std::env::vars_os() {
            if [
                "PATH",
                "SYSTEMROOT",
                "SYSTEMDRIVE",
                "COMSPEC",
                "PATHEXT",
                "WINDIR",
                "LANG",
                "LC_ALL",
            ]
            .contains(&key.to_string_lossy().to_ascii_uppercase().as_str())
            {
                command.env(key, value);
            }
        }
        let mut child = command
            .env(CHILD, &root.0)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("TMPDIR", &temp)
            .env("TMP", &temp)
            .env("TEMP", &temp)
            .stdin(Stdio::null())
            .stdout(File::create(&stdout).unwrap())
            .stderr(File::create(&stderr).unwrap())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        let status = loop {
            match child.try_wait().unwrap() {
                Some(status) => break status,
                None if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                None => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("isolated custody leaf test exceeded 15 seconds");
                }
            }
        };
        let read = |path: &Path| {
            let mut bytes = Vec::new();
            File::open(path)
                .unwrap()
                .take(65_537)
                .read_to_end(&mut bytes)
                .unwrap();
            assert!(bytes.len() <= 65_536);
            String::from_utf8_lossy(&bytes).into_owned()
        };
        assert!(status.success(), "{}\n{}", read(&stdout), read(&stderr));
        assert!(root.0.join("seed/.git").is_dir());
        assert!(
            !crate::process::custody_lost(),
            "child fault injection must not poison this test process"
        );
    }
}
