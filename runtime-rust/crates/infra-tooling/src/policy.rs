use crate::{common, mirrors::check_receipts};
use infra_core::{Result, RuntimeError, domain::json as native_json, fs as safe};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, sync::OnceLock};

fn model_re() -> Result<&'static regex::Regex> {
    static RE: OnceLock<std::result::Result<regex::Regex, regex::Error>> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"(?m)^model:\s*(\S+)\s*$"))
        .as_ref()
        .map_err(|_| RuntimeError::invalid("Model expression is invalid"))
}

#[cfg(test)]
fn policy_live_runs() -> &'static std::sync::atomic::AtomicUsize {
    static COUNT: OnceLock<std::sync::atomic::AtomicUsize> = OnceLock::new();
    COUNT.get_or_init(|| std::sync::atomic::AtomicUsize::new(0))
}
const PATHS: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    ".claude/CLAUDE.md",
    ".claude/DOD.md",
    ".claude/GOLDEN-PRINCIPLES.md",
    ".claude/STABILIZATION.md",
    ".agents/skills",
    ".claude/skills/skill-creator/SKILL.md",
    ".cursor/skills/skill-creator/SKILL.md",
    ".claude/skills/SKILL FLOW.md",
    ".agents/skills/SKILL FLOW.md",
    ".claude/agents/*-agent.md",
    ".claude/commands",
    ".claude/settings.json",
    ".cursor/rules",
    ".cursor/hooks.json",
    ".codex/hooks.json",
    ".codex/config.toml",
];
fn retired_policy_source(path: &str) -> bool {
    // The canonical skill subtree includes historical Python helper/oracle
    // names in an unstaged index after retirement. They are not native payload.
    if path.starts_with(".agents/skills/") && path.ends_with(".py") {
        return true;
    }
    path.strip_prefix(".agents/skills/memory-seed/assets/scripts/")
        .is_some_and(|name| !name.contains('/') && name.ends_with(".sh"))
}
pub fn lock(root: &Path, edition: &str, write: bool) -> Result<Value> {
    lock_with_options(root, edition, write, true, None).map(|(report, _)| report)
}

pub fn lock_with_options(
    root: &Path,
    edition: &str,
    write: bool,
    use_cache: bool,
    cache_path: Option<&Path>,
) -> Result<(Value, bool)> {
    let edition = common::edition(edition)?;
    let base = root.join(edition);
    let prefix = format!("{edition}/");
    let lock_relative = format!("{edition}/.accelerator-policy-lock.json");
    let effective_cache = !write && use_cache && !check_receipts::is_cache_bypassed();
    let receipt_path = effective_cache.then(|| {
        cache_path.map(Path::to_path_buf).unwrap_or_else(|| {
            check_receipts::default_receipt_path(root, "infra-tooling-receipts.json")
        })
    });
    let mut store = receipt_path
        .as_deref()
        .map(check_receipts::load_store)
        .unwrap_or_default();
    let mut dirty = false;
    let now_ns = check_receipts::now_unix_ns();
    let scope = format!("policy-lock:{edition}");

    let rules_and_lock = if effective_cache {
        (|| -> Option<(String, String)> {
            let git_cue = check_receipts::git_modes_cue(root, &mut store, now_ns, &mut dirty)?;
            let version_path = base.join("VERSION");
            let version_sha = if version_path.exists() {
                check_receipts::hash_regular_file(&version_path, &mut store, now_ns, &mut dirty)?.0
            } else {
                "absent".into()
            };
            let lock_path = root.join(&lock_relative);
            let lock_sha = if lock_path.exists() {
                check_receipts::hash_regular_file(&lock_path, &mut store, now_ns, &mut dirty)?.0
            } else {
                "absent".into()
            };
            let rules_sha = common::sha256(
                format!("policy-lock-v1\nedition:{edition}\nversion:{version_sha}\ngit:{git_cue}")
                    .as_bytes(),
            );
            Some((rules_sha, lock_sha))
        })()
    } else {
        None
    };

    if effective_cache
        && let Some((ref rules_sha, ref lock_sha)) = rules_and_lock
        && let Some(candidate) = store
            .latest_for_scope_and_rules(&scope, rules_sha, lock_sha)
            .cloned()
        && let Some(wrapper) = candidate.report.as_ref()
        && let (Some(tracked_names), Some(cached_report)) = (
            wrapper.get("tracked_paths").and_then(Value::as_array),
            wrapper.get("report"),
        )
    {
        let names = tracked_names
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let canonical_sha = (|| -> Option<String> {
            let mut parts = Vec::with_capacity(names.len());
            for name in &names {
                let path = common::under(root, name).ok()?;
                let (sha, _) =
                    check_receipts::hash_regular_file(&path, &mut store, now_ns, &mut dirty)?;
                parts.push(format!("{name}:{sha}"));
            }
            Some(common::sha256(parts.join("\n").as_bytes()))
        })();
        if let Some(c_sha) = canonical_sha
            && c_sha == candidate.canonical_tree_sha256
        {
            let report = cached_report.clone();
            let _ = store.matches_report(&scope, &c_sha, rules_sha, lock_sha);
            if dirty && let Some(path) = receipt_path.as_deref() {
                check_receipts::save_store(path, &store);
            }
            return Ok((report, true));
        }
    }

    #[cfg(test)]
    policy_live_runs().fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let specs = PATHS
        .iter()
        .map(|path| format!("{edition}/{path}"))
        .collect::<Vec<_>>();
    let names = common::tracked(root, &specs)?;
    if names.is_empty() {
        return Err(RuntimeError::not_found(
            "No model-facing files tracked for edition",
        ));
    }
    let mut files = BTreeMap::new();
    let mut models = BTreeMap::new();
    let mut findings = Vec::new();
    let mut checked_names = Vec::new();
    let model = model_re()?;
    for name in names {
        let relative = name
            .strip_prefix(&prefix)
            .ok_or_else(|| RuntimeError::policy("Policy path escaped edition"))?;
        if relative == ".accelerator-policy-lock.json" || retired_policy_source(relative) {
            continue;
        }
        checked_names.push(name.clone());
        let bytes = safe::read_regular(&common::under(root, &name)?)?;
        files.insert(relative.to_string(), common::sha256(&bytes));
        if relative.starts_with(".claude/agents/") && relative.ends_with("-agent.md") {
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| RuntimeError::invalid("Agent policy is not UTF-8"))?;
            if let Some(found) = model.captures(text) {
                let value = found[1].to_string();
                let stem = Path::new(relative)
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| RuntimeError::invalid("Agent name is not UTF-8"))?;
                let allowed = if edition == "Infrastructure-Creator" {
                    vec!["opus", "sonnet"]
                } else {
                    vec!["opus", "sonnet", "haiku"]
                };
                if !allowed.contains(&value.as_str()) {
                    findings.push(format!("{relative}: invalid model {value}"));
                }
                models.insert(stem.to_string(), value);
            }
        }
    }
    let file_value = serde_json::to_value(files)?;
    let manifest = native_json::serialize(&file_value, true, true, false, false)?;
    let release = if base.join("VERSION").is_file() {
        safe::read_text(&base.join("VERSION"))?.trim().into()
    } else {
        String::from("unknown")
    };
    let value = json!({"schema_version":1,"edition":edition,"release":release,"policy_digest":common::sha256(manifest.as_bytes()),"agent_models":models,"files":file_value});
    if write {
        if !findings.is_empty() {
            return Ok((
                json!({"edition":edition,"passed":false,"findings":findings,"written":false,"files":file_value.as_object().map_or(0,|files|files.len()),"policy_digest":value["policy_digest"]}),
                false,
            ));
        }
        common::write(root, &lock_relative, &common::json_bytes(&value)?)?;
    } else {
        match common::read_json(&root.join(&lock_relative)) {
            Ok(stored) => {
                for key in [
                    "schema_version",
                    "edition",
                    "policy_digest",
                    "agent_models",
                    "files",
                ] {
                    if stored[key] != value[key] {
                        findings.push(format!("Policy lock field differs: {key}"));
                    }
                }
            }
            Err(error) => findings.push(format!("Policy lock cannot be read: {error}")),
        }
    }
    let report = json!({"edition":edition,"passed":findings.is_empty(),"findings":findings,"written":write,"files":file_value.as_object().map_or(0,|files|files.len()),"policy_digest":value["policy_digest"]});
    if effective_cache
        && findings.is_empty()
        && let Some((rules_sha, lock_sha)) = rules_and_lock
    {
        let mut parts = Vec::with_capacity(checked_names.len());
        let mut ok = true;
        for name in &checked_names {
            let Ok(path) = common::under(root, name) else {
                ok = false;
                break;
            };
            if let Some((sha, _)) =
                check_receipts::hash_regular_file(&path, &mut store, now_ns, &mut dirty)
            {
                parts.push(format!("{name}:{sha}"));
            } else {
                ok = false;
                break;
            }
        }
        if ok {
            let canonical_sha = common::sha256(parts.join("\n").as_bytes());
            let wrapper = json!({
                "tracked_paths": checked_names,
                "report": report,
            });
            store.insert_report(&scope, canonical_sha, rules_sha, lock_sha, Some(wrapper));
            dirty = true;
        }
        if dirty && let Some(path) = receipt_path.as_deref() {
            check_receipts::save_store(path, &store);
        }
    }
    Ok((report, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_lock_shares_tree_receipt_and_honors_write_strict_and_file_drift() {
        let dir = crate::temp::Temporary::new("infra-policy-receipt").unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("PHP Core/.claude/agents")).unwrap();
        std::fs::write(root.join("PHP Core/VERSION"), "1.0.0\n").unwrap();
        std::fs::write(root.join("PHP Core/AGENTS.md"), "# Policy\n").unwrap();
        std::fs::write(
            root.join("PHP Core/.claude/agents/review-agent.md"),
            "---\nmodel: sonnet\n---\n",
        )
        .unwrap();
        let run_git = |args: &[&str]| {
            let out = common::git_ok(root, args, None).unwrap();
            let _ = out;
        };
        run_git(&["init", "-b", "main"]);
        run_git(&["config", "user.email", "test@example.com"]);
        run_git(&["config", "user.name", "Test"]);
        run_git(&["add", "."]);
        run_git(&["commit", "-m", "init"]);

        let cache = root.join(".git/policy-receipts.json");
        let (written, hit_write) =
            lock_with_options(root, "PHP Core", true, true, Some(&cache)).unwrap();
        assert!(!hit_write);
        assert_eq!(written["passed"], true);

        let before = policy_live_runs().load(std::sync::atomic::Ordering::Relaxed);
        let (cold, hit1) = lock_with_options(root, "PHP Core", false, true, Some(&cache)).unwrap();
        assert!(!hit1);
        assert_eq!(cold["passed"], true);
        assert_eq!(
            policy_live_runs().load(std::sync::atomic::Ordering::Relaxed),
            before + 1
        );

        let (warm, hit2) = lock_with_options(root, "PHP Core", false, true, Some(&cache)).unwrap();
        assert!(hit2);
        assert_eq!(warm, cold);
        assert_eq!(
            policy_live_runs().load(std::sync::atomic::Ordering::Relaxed),
            before + 1
        );

        let (strict_run, hit3) = check_receipts::with_cache_bypass(true, || {
            lock_with_options(root, "PHP Core", false, true, Some(&cache))
        })
        .unwrap();
        assert!(!hit3);
        assert_eq!(strict_run, cold);
        assert_eq!(
            policy_live_runs().load(std::sync::atomic::Ordering::Relaxed),
            before + 2
        );

        // Policy edit invalidates immediately.
        std::fs::write(root.join("PHP Core/AGENTS.md"), "# Policy changed\n").unwrap();
        let (drifted, hit4) =
            lock_with_options(root, "PHP Core", false, true, Some(&cache)).unwrap();
        assert!(!hit4);
        assert_eq!(drifted["passed"], false);
    }
}
