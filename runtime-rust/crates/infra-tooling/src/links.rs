use crate::{common, mirrors::check_receipts};
use infra_core::{Result, RuntimeError, fs as safe};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, sync::OnceLock};

fn link_re() -> Result<&'static regex::Regex> {
    static RE: OnceLock<std::result::Result<regex::Regex, regex::Error>> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r#"\[[^\]]*\]\(\s*(?:<([^>]*)>|([^()\s]+))(?:\s+"[^"]*")?\s*\)"#)
    })
    .as_ref()
    .map_err(|_| RuntimeError::invalid("Link expression is invalid"))
}

fn inline_re() -> Result<&'static regex::Regex> {
    static RE: OnceLock<std::result::Result<regex::Regex, regex::Error>> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"`[^`]*`"))
        .as_ref()
        .map_err(|_| RuntimeError::invalid("Inline expression is invalid"))
}

fn scheme_re() -> Result<&'static regex::Regex> {
    static RE: OnceLock<std::result::Result<regex::Regex, regex::Error>> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"^[a-zA-Z][a-zA-Z0-9+.-]*:"))
        .as_ref()
        .map_err(|_| RuntimeError::invalid("Scheme expression is invalid"))
}

#[cfg(test)]
fn links_live_runs() -> &'static std::sync::atomic::AtomicUsize {
    static COUNT: OnceLock<std::sync::atomic::AtomicUsize> = OnceLock::new();
    COUNT.get_or_init(|| std::sync::atomic::AtomicUsize::new(0))
}
fn percent_decode(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let nibble = |byte: u8| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                b'A'..=b'F' => Some(byte - b'A' + 10),
                _ => None,
            };
            if let (Some(high), Some(low)) = (nibble(bytes[i + 1]), nibble(bytes[i + 2])) {
                decoded.push(high * 16 + low);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(decoded).map_err(|_| RuntimeError::invalid("Link target is not UTF-8"))
}
pub fn check(root: &Path, standalone: bool) -> Result<Value> {
    check_with_options(root, standalone, true, None).map(|(report, _)| report)
}

pub fn check_with_options(
    root: &Path,
    standalone: bool,
    use_cache: bool,
    cache_path: Option<&Path>,
) -> Result<(Value, bool)> {
    let root = infra_core::fs::canonicalize(root)?;
    let effective_cache = use_cache && !check_receipts::is_cache_bypassed();
    let receipt_path = effective_cache.then(|| {
        cache_path.map(Path::to_path_buf).unwrap_or_else(|| {
            check_receipts::default_receipt_path(&root, "infra-tooling-receipts.json")
        })
    });
    let mut store = receipt_path
        .as_deref()
        .map(check_receipts::load_store)
        .unwrap_or_default();
    let mut dirty = false;
    let now_ns = check_receipts::now_unix_ns();
    let scope = format!(
        "links:{}",
        if standalone { "standalone" } else { "tracked" }
    );
    let pattern_path = root.join("scripts/check_links_ignore.txt");

    let rules_sha = if effective_cache {
        (|| -> Option<String> {
            let ignore_sha = if !standalone && pattern_path.exists() {
                check_receipts::hash_regular_file(&pattern_path, &mut store, now_ns, &mut dirty)?.0
            } else {
                "absent".into()
            };
            let git_cue = if standalone {
                "standalone".into()
            } else {
                check_receipts::git_modes_cue(&root, &mut store, now_ns, &mut dirty)?
            };
            Some(common::sha256(
                format!("links-v1\nstandalone:{standalone}\nignore:{ignore_sha}\ngit:{git_cue}")
                    .as_bytes(),
            ))
        })()
    } else {
        None
    };

    if effective_cache
        && let Some(ref r_sha) = rules_sha
        && let Some(candidate) = store
            .latest_for_scope_and_rules(&scope, r_sha, "links-targets")
            .cloned()
        && let Some(wrapper) = candidate.report.as_ref()
        && let (Some(md_files), Some(targets), Some(cached_report)) = (
            wrapper.get("md_files").and_then(Value::as_array),
            wrapper.get("targets").and_then(Value::as_object),
            wrapper.get("report"),
        )
    {
        let md_names = md_files
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let standalone_files_match = if standalone {
            common::walk(&root)
                .ok()
                .map(|walked| {
                    walked
                        .into_iter()
                        .filter(|n| n.ends_with(".md"))
                        .collect::<Vec<_>>()
                })
                .is_some_and(|live| live == md_names)
        } else {
            true
        };
        let targets_valid = standalone_files_match
            && targets.iter().all(|(rel, expected_exists)| {
                expected_exists.as_bool().is_some_and(|want| {
                    let cand = root.join(rel);
                    safe::canonical_or_missing(&cand)
                        .is_ok_and(|res| res.starts_with(&root) && res.exists() == want)
                })
            });
        let md_tree_sha = if targets_valid {
            (|| -> Option<String> {
                let mut parts = Vec::with_capacity(md_names.len());
                for name in &md_names {
                    let path = common::under(&root, name).ok()?;
                    if path.is_file() {
                        let (sha, _) = check_receipts::hash_regular_file(
                            &path, &mut store, now_ns, &mut dirty,
                        )?;
                        parts.push(format!("{name}:{sha}"));
                    } else {
                        parts.push(format!("{name}:missing"));
                    }
                }
                Some(common::sha256(parts.join("\n").as_bytes()))
            })()
        } else {
            None
        };
        if let Some(c_sha) = md_tree_sha
            && c_sha == candidate.canonical_tree_sha256
        {
            let report = cached_report.clone();
            let _ = store.matches_report(&scope, &c_sha, r_sha, "links-targets");
            if dirty && let Some(path) = receipt_path.as_deref() {
                check_receipts::save_store(path, &store);
            }
            return Ok((report, true));
        }
    }

    #[cfg(test)]
    links_live_runs().fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let files = if standalone {
        common::walk(&root)?
    } else {
        common::tracked(&root, &["*.md".into()])?
    };
    let ignores = if !standalone && pattern_path.is_file() {
        safe::read_text(&pattern_path)?
            .lines()
            .map(|line| line.split('#').next().unwrap_or("").trim().to_string())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let link = link_re()?;
    let inline = inline_re()?;
    let scheme = scheme_re()?;
    let mut findings = Vec::new();
    let mut examined = 0usize;
    let md_names: Vec<String> = files
        .into_iter()
        .filter(|name| name.ends_with(".md"))
        .collect();
    let mut verified_targets = BTreeMap::<String, bool>::new();
    for name in &md_names {
        let path = common::under(&root, name)?;
        if !path.is_file() {
            continue;
        }
        examined += 1;
        let bytes = safe::read_regular(&path)?;
        let text = String::from_utf8_lossy(&bytes);
        let mut fence = None::<&str>;
        for (number, line) in text.lines().enumerate() {
            let left = line.trim_start();
            if left.starts_with("```") || left.starts_with("~~~") {
                let marker = &left[..3];
                if fence == Some(marker) {
                    fence = None;
                } else if fence.is_none() {
                    fence = Some(marker);
                }
                continue;
            }
            if fence.is_some() {
                continue;
            }
            let visible = inline.replace_all(line, "");
            for capture in link.captures_iter(&visible) {
                let raw = capture
                    .get(1)
                    .or_else(|| capture.get(2))
                    .map(|value| value.as_str())
                    .unwrap_or("");
                if scheme.is_match(raw) {
                    continue;
                }
                let target = raw.split('#').next().unwrap_or("");
                if target.is_empty() {
                    continue;
                }
                let decoded = percent_decode(target)?;
                let candidate = if decoded.starts_with('/') {
                    root.join(decoded.strip_prefix('/').unwrap_or(&decoded))
                } else {
                    path.parent().unwrap_or(&root).join(decoded)
                };
                let resolved = safe::canonical_or_missing(&candidate)?;
                let inside = resolved.starts_with(&root);
                if inside && let Ok(rel) = resolved.strip_prefix(&root) {
                    verified_targets
                        .insert(rel.to_string_lossy().replace('\\', "/"), resolved.exists());
                }
                if inside && resolved.exists() {
                    continue;
                }
                let shown = resolved
                    .strip_prefix(&root)
                    .unwrap_or(&resolved)
                    .to_string_lossy();
                let mut ignored = false;
                for pattern in &ignores {
                    if common::matches(pattern, raw, false)?
                        || common::matches(pattern, &shown, false)?
                    {
                        ignored = true;
                        break;
                    }
                }
                if !ignored {
                    findings.push(json!({"path":name,"line":number+1,"target":raw,"resolved":shown,"reason":if inside{"missing"}else{"outside-root"}}));
                }
            }
        }
    }
    let report = json!({"passed":findings.is_empty(),"files":examined,"findings":findings});
    if effective_cache
        && findings.is_empty()
        && let Some(r_sha) = rules_sha
    {
        let mut parts = Vec::with_capacity(md_names.len());
        let mut ok = true;
        for name in &md_names {
            let Ok(path) = common::under(&root, name) else {
                ok = false;
                break;
            };
            if path.is_file() {
                if let Some((sha, _)) =
                    check_receipts::hash_regular_file(&path, &mut store, now_ns, &mut dirty)
                {
                    parts.push(format!("{name}:{sha}"));
                } else {
                    ok = false;
                    break;
                }
            } else {
                parts.push(format!("{name}:missing"));
            }
        }
        if ok {
            let md_tree_sha = common::sha256(parts.join("\n").as_bytes());
            let wrapper = json!({
                "md_files": md_names,
                "targets": verified_targets,
                "report": report,
            });
            store.insert_report(
                &scope,
                md_tree_sha,
                r_sha,
                "links-targets".into(),
                Some(wrapper),
            );
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
    fn links_check_shares_tree_receipt_and_honors_bypass_and_target_changes() {
        let dir = crate::temp::Temporary::new("infra-links-receipt").unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(root.join("docs/target.md"), "# Target\n").unwrap();
        std::fs::write(root.join("README.md"), "[Target](docs/target.md)\n").unwrap();
        let cache = root.join("receipts.json");

        let before = links_live_runs().load(std::sync::atomic::Ordering::Relaxed);
        let (first, hit1) = check_with_options(root, true, true, Some(&cache)).unwrap();
        assert!(!hit1);
        assert_eq!(first["passed"], true);
        assert_eq!(
            links_live_runs().load(std::sync::atomic::Ordering::Relaxed),
            before + 1
        );

        let (second, hit2) = check_with_options(root, true, true, Some(&cache)).unwrap();
        assert!(hit2);
        assert_eq!(second, first);
        assert_eq!(
            links_live_runs().load(std::sync::atomic::Ordering::Relaxed),
            before + 1
        );

        // strict / bypass flag skips cache.
        let (bypassed, hit3) = check_receipts::with_cache_bypass(true, || {
            check_with_options(root, true, true, Some(&cache))
        })
        .unwrap();
        assert!(!hit3);
        assert_eq!(bypassed, first);
        assert_eq!(
            links_live_runs().load(std::sync::atomic::Ordering::Relaxed),
            before + 2
        );

        // Removing a linked target invalidates immediately.
        std::fs::remove_file(root.join("docs/target.md")).unwrap();
        let (broken, hit4) = check_with_options(root, true, true, Some(&cache)).unwrap();
        assert!(!hit4);
        assert_eq!(broken["passed"], false);
    }
}
