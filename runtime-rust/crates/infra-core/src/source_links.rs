//! Bounded declared relations: prompt paths, explicit citations and shared
//! current canonical source revisions. A relation is never new authority.
use crate::{
    Result, RuntimeConfig, RuntimeContext, RuntimeError,
    domain::{self, json},
    index::{DOCUMENT_SELECT, Exclusion, document_from_row},
    observability::Trace,
    query::{self, Candidate},
};
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
pub const GRAPH_ANCHOR_LIMIT: usize = 2;
pub const GRAPH_ROW_LIMIT: usize = 32;
pub const GRAPH_SOURCE_BYTE_LIMIT: u64 = 8 * 1024 * 1024;
pub const GRAPH_CANDIDATE_LIMIT: usize = 3;
pub const FILE_LINK_LIMIT: usize = 1;
pub const FILE_LINK_HUB_LIMIT: usize = 5;
pub const FILE_LINK_POOL: usize = 8;
pub const PROMPT_SEED_LIMIT: usize = 6;
pub const TOUCH_SEED_LIMIT: usize = 8;
pub const PROMPT_SCAN_CHARS: usize = 16384;
pub const LINK_PATH_SCAN_LIMIT: usize = 20000;
pub fn file_link_problem(path: &str) -> Option<&'static str> {
    if path.is_empty() || path.contains(['\n', '\r', '\0']) {
        return Some("shape");
    }
    let path = path.strip_prefix("./").unwrap_or(path);
    if posix_normal(path) != strip_all(path, '/') || [".", ""].contains(&path) {
        return Some("shape");
    }
    if domain::source_path_problem(path).is_some() {
        return Some("sensitive");
    }
    if path.starts_with("project-brain/") || path.starts_with("memory-bank/") {
        return Some("state");
    }
    if path.ends_with('/') {
        return Some("directory");
    }
    const PARTS: &[&str] = &[
        "var",
        "node_modules",
        "vendor",
        ".phpunit.cache",
        "coverage",
        ".idea",
        ".vscode",
        ".devcontainer",
        "__pycache__",
    ];
    const PREFIXES: &[&str] = &[
        "public/build/",
        "public/bundles/",
        "public/hot",
        "bootstrap/cache/",
        "storage/framework/",
        "storage/logs/",
    ];
    const NAMES: &[&str] = &[
        "composer.lock",
        "package-lock.json",
        "yarn.lock",
        "pnpm-lock.yaml",
        "symfony.lock",
        ".phpunit.result.cache",
    ];
    if path
        .split('/')
        .any(|part| PARTS.contains(&query::folded(part).as_str()))
        || PREFIXES.iter().any(|prefix| path.starts_with(prefix))
        || NAMES.contains(&path.rsplit('/').next().unwrap_or(""))
    {
        return Some("build");
    }
    if domain::secret_label(path).ok().flatten().is_some() {
        return Some("secret");
    }
    None
}
fn posix_normal(path: &str) -> String {
    let prefix = if path.starts_with("//") && !path.starts_with("///") {
        "//"
    } else if path.starts_with('/') {
        "/"
    } else {
        ""
    };
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|last| *last != "..") {
                    parts.pop();
                } else if prefix.is_empty() {
                    parts.push("..");
                }
            }
            _ => parts.push(part),
        }
    }
    let normalized = format!("{prefix}{}", parts.join("/"));
    if normalized.is_empty() {
        ".".into()
    } else {
        normalized
    }
}
fn inverse_key(value: &str) -> Vec<u32> {
    value.chars().map(|c| 0x10ffff - c as u32).collect()
}
fn strip_all(mut text: &str, suffix: char) -> &str {
    while let Some(previous) = text.strip_suffix(suffix) {
        text = previous;
    }
    text
}
fn py_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
fn prior(text: &str, start: usize) -> Option<char> {
    text[..start].chars().next_back()
}
fn next(text: &str, end: usize) -> Option<char> {
    text[end..].chars().next()
}
fn expression(pattern: &str) -> Result<regex::Regex> {
    regex::Regex::new(pattern)
        .map_err(|_| RuntimeError::invalid("Invalid native prompt path grammar"))
}
fn composer_psr4(project: &Path) -> Vec<(String, Vec<String>)> {
    let operation = (|| -> Result<Vec<(String, Vec<String>)>> {
        let path = project.join("composer.json");
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.is_file()
            || crate::fs::linked_at(&path, &metadata)?
            || metadata.len() > 256 * 1024
        {
            return Ok(vec![]);
        }
        let raw = crate::fs::read_regular(&path)?;
        if raw.len() > 256 * 1024 {
            return Ok(vec![]);
        }
        let value = json::parse(
            std::str::from_utf8(&raw)
                .map_err(|_| RuntimeError::invalid("Composer map is not UTF-8"))?,
        )?;
        let mut prefixes: Vec<(String, Vec<String>)> = vec![];
        for section in ["autoload", "autoload-dev"] {
            let Some(mapping) = value[section]["psr-4"].as_object() else {
                continue;
            };
            for (prefix, directories) in mapping {
                let key = if prefix.is_empty() {
                    String::new()
                } else {
                    format!("{}\\", strip_all(prefix, '\\'))
                };
                let directories = directories
                    .as_array()
                    .map(|values| values.iter().collect::<Vec<_>>())
                    .unwrap_or_else(|| vec![directories]);
                for directory in directories {
                    let Some(directory) = directory.as_str() else {
                        continue;
                    };
                    let directory =
                        strip_all(directory.strip_prefix("./").unwrap_or(directory), '/');
                    if !directory.is_empty()
                        && domain::source_path_problem(&format!("{directory}/x.php")).is_some()
                    {
                        continue;
                    }
                    let position = prefixes.iter().position(|(prefix, _)| prefix == &key);
                    if let Some(position) = position {
                        prefixes[position].1.push(directory.into());
                    } else {
                        prefixes.push((key.clone(), vec![directory.into()]));
                    }
                }
            }
        }
        prefixes.sort_by_key(|(prefix, _)| std::cmp::Reverse(prefix.chars().count()));
        Ok(prefixes)
    })();
    operation.unwrap_or_default()
}
fn linked_paths(connection: &Connection) -> Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT DISTINCT ref_path FROM document_links WHERE ref_kind IN ('source','file') LIMIT ?1",
    )?;
    Ok(statement
        .query_map([LINK_PATH_SCAN_LIMIT], |row| row.get(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?)
}
pub fn prompt_path_seeds(
    connection: &Connection,
    context: &RuntimeContext,
    text: &str,
) -> Result<Vec<String>> {
    let text = query::chars(text, PROMPT_SCAN_CHARS);
    let mut found = vec![];
    let pattern = expression(
        r"(?:\./)?((?:[\p{L}\p{N}_.-]+/)+[\p{L}\p{N}_-]+(?:\.[\p{L}\p{N}_-]+)*\.[A-Za-z0-9]{1,8})",
    )?;
    for capture in pattern.captures_iter(&text) {
        let whole = capture
            .get(0)
            .ok_or_else(|| RuntimeError::invalid("Prompt path capture unavailable"))?;
        if prior(&text, whole.start()).is_some_and(|c| py_word(c) || "@:/\\.-".contains(c))
            || next(&text, whole.end()).is_some_and(|c| py_word(c) || "/-".contains(c))
        {
            continue;
        }
        found.push(capture[1].to_string());
    }
    let native = context.roots.project.to_string_lossy().to_string();
    #[cfg(windows)]
    let native = {
        let mut native = native;
        if let Some(plain) = native.strip_prefix("\\\\?\\") {
            native = if let Some(unc) = plain.strip_prefix("UNC\\") {
                format!("\\\\{unc}")
            } else {
                plain.into()
            };
        }
        native
    };
    let posix = native.replace('\\', "/");
    let mut roots = vec![(format!("{}/", strip_all(&posix, '/')), false)];
    if native != posix {
        roots.push((format!("{}\\", strip_all(&native, '\\')), true));
    }
    for (root, windows) in roots {
        let tail = if windows {
            expression(r"^(?:[\p{L}\p{N}_.-]+\\)*[\p{L}\p{N}_.-]+\.[A-Za-z0-9]{1,8}")?
        } else {
            expression(r"^(?:[\p{L}\p{N}_.-]+/)*[\p{L}\p{N}_.-]+\.[A-Za-z0-9]{1,8}")?
        };
        let mut position = 0usize;
        while let Some(offset) = text[position..].find(&root) {
            let start = position + offset;
            if !prior(&text, start).is_some_and(|c| py_word(c) || ".-".contains(c))
                && let Some(found_tail) = tail.find(&text[start + root.len()..])
            {
                found.push(found_tail.as_str().replace('\\', "/"));
            }
            position = start + root.len();
            if position > text.len() {
                break;
            }
        }
    }
    let pattern = expression(r"((?:[\p{L}\p{N}_.-]+\\)+[\p{L}\p{N}_.-]+\.[A-Za-z0-9]{1,8})")?;
    for capture in pattern.captures_iter(&text) {
        let matched = capture
            .get(0)
            .ok_or_else(|| RuntimeError::invalid("Prompt path capture unavailable"))?;
        if !prior(&text, matched.start()).is_some_and(|c| py_word(c) || c == '\\') {
            found.push(capture[1].replace('\\', "/"));
        }
    }
    let fqcn = expression(r"\\?((?:[A-Z][A-Za-z0-9_]*\\+)+[A-Z][A-Za-z0-9_]*)")?;
    let slash = expression(r"\\+")?;
    let names = fqcn
        .captures_iter(&text)
        .map(|capture| slash.replace_all(&capture[1], "\\").into_owned())
        .collect::<Vec<_>>();
    let bare_pattern = expression(r"([\p{L}\p{N}_-]+\.[A-Za-z0-9]{1,8})")?;
    let bare = bare_pattern
        .captures_iter(&text)
        .filter_map(|capture| {
            let matched = capture.get(0)?;
            if prior(&text, matched.start()).is_some_and(|c| py_word(c) || "/\\.-".contains(c))
                || next(&text, matched.end()).is_some_and(|c| py_word(c) || "/-".contains(c))
            {
                None
            } else {
                Some(capture[1].to_string())
            }
        })
        .collect::<Vec<_>>();
    let symbol_pattern = expression(r"([A-Z][a-z0-9]+(?:[A-Z][a-z0-9]*)+)")?;
    let symbols = symbol_pattern
        .captures_iter(&text)
        .filter_map(|capture| {
            let matched = capture.get(0)?;
            if prior(&text, matched.start()).is_some_and(|c| py_word(c) || "\\$".contains(c))
                || next(&text, matched.end()).is_some_and(|c| py_word(c) || "\\(".contains(c))
            {
                None
            } else {
                Some(capture[1].to_string())
            }
        })
        .collect::<Vec<_>>();
    if !names.is_empty() {
        let prefixes = composer_psr4(&context.roots.project);
        for name in &names {
            for (prefix, directories) in &prefixes {
                let Some(rest) = name.strip_prefix(prefix) else {
                    continue;
                };
                let rest = rest.replace('\\', "/");
                for directory in directories.iter().take(2) {
                    found.push(if directory.is_empty() {
                        format!("{rest}.php")
                    } else {
                        format!("{directory}/{rest}.php")
                    });
                }
                break;
            }
        }
    }
    if !names.is_empty() || !bare.is_empty() || !symbols.is_empty() {
        let paths = linked_paths(connection)?;
        for name in &names {
            let segments = name.split('\\').collect::<Vec<_>>();
            if segments.len() > 1 {
                let suffix = format!("/{}.php", segments[1..].join("/"));
                found.extend(
                    paths
                        .iter()
                        .filter(|path| path.ends_with(&suffix))
                        .take(2)
                        .cloned(),
                );
            }
        }
        for name in bare {
            found.extend(
                paths
                    .iter()
                    .filter(|path| path.rsplit('/').next() == Some(name.as_str()))
                    .take(2)
                    .cloned(),
            );
        }
        for symbol in symbols {
            found.extend(
                paths
                    .iter()
                    .filter(|path| {
                        let name = path.rsplit('/').next().unwrap_or(path);
                        let stem = name
                            .rsplit_once('.')
                            .map(|(stem, _)| if stem.is_empty() { name } else { stem })
                            .unwrap_or(name);
                        stem == symbol
                    })
                    .take(2)
                    .cloned(),
            );
        }
    }
    let mut seen = BTreeSet::new();
    let mut seeds = vec![];
    for path in found {
        let path = path.strip_prefix("./").unwrap_or(&path).to_string();
        if !seen.insert(path.clone()) {
            continue;
        }
        if seeds.len() >= PROMPT_SEED_LIMIT {
            break;
        }
        if file_link_problem(&path).is_some() {
            continue;
        }
        if connection
            .query_row(
                "SELECT 1 FROM document_links WHERE ref_path=?1 LIMIT 1",
                [&path],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            seeds.push(path);
        }
    }
    Ok(seeds)
}
pub fn incoming_conflict(connection: &Connection, record_id: Option<&str>) -> Result<bool> {
    let Some(id) = record_id else {
        return Ok(false);
    };
    Ok(connection.query_row("SELECT 1 FROM document_metadata WHERE conflicts!='[]' AND instr(conflicts,?1)>0 LIMIT 1",[format!("\"{id}\"")],|_|Ok(())).optional()?.is_some())
}
fn hydrate(connection: &Connection, paths: &[String]) -> Result<Vec<Candidate>> {
    if paths.is_empty() {
        return Ok(vec![]);
    }
    let marks = vec!["?"; paths.len()].join(",");
    let sql = format!(
        "SELECT {DOCUMENT_SELECT} FROM documents d JOIN document_metadata m ON m.path=d.path WHERE d.path IN ({marks}) ORDER BY d.path"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(paths), document_from_row)?;
    Ok(rows
        .map(|row| row.map(|document| Candidate::linked(document, "source-link")))
        .collect::<std::result::Result<Vec<_>, _>>()?)
}
#[expect(
    clippy::too_many_arguments,
    reason = "Declared-link admission keeps reference grammar, relation kinds, row cap, current-source policy and trace explicit"
)]
pub fn linked_documents(
    connection: &Connection,
    context: &RuntimeContext,
    config: &RuntimeConfig,
    reference: &str,
    prefix: bool,
    max_rows: Option<usize>,
    ref_kinds: &[&str],
    policy: &mut crate::source_policy::CurrentSourcePolicy,
    mut trace: Option<&mut Trace>,
) -> Result<(Vec<Candidate>, Vec<Exclusion>)> {
    let reference = reference
        .split('#')
        .next()
        .unwrap_or(reference)
        .trim_matches(crate::results::py_space);
    if reference.is_empty() {
        return Ok((vec![], vec![]));
    }
    if max_rows == Some(0) {
        return Err(RuntimeError::invalid(
            "Source-link row limit must be a positive integer",
        ));
    }
    if ref_kinds.is_empty()
        || ref_kinds
            .iter()
            .any(|kind| !["source", "file"].contains(kind))
    {
        return Err(RuntimeError::invalid(
            "Link kinds must be among: source, file",
        ));
    }
    let mut values = vec![rusqlite::types::Value::Text(reference.into())];
    let predicate = if prefix {
        let pattern = strip_all(reference, '/')
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        values.push(rusqlite::types::Value::Text(format!("{pattern}/%")));
        "(l.ref_path=? OR l.ref_path LIKE ? ESCAPE '\\')"
    } else {
        "(l.ref_path=?)"
    };
    values.extend(
        ref_kinds
            .iter()
            .map(|kind| rusqlite::types::Value::Text((*kind).into())),
    );
    let mut sql = format!(
        "SELECT {DOCUMENT_SELECT} FROM document_links l JOIN documents d ON d.path=l.path JOIN document_metadata m ON m.path=l.path WHERE {predicate} AND l.ref_kind IN ({}) ORDER BY d.path,l.ref_path",
        vec!["?"; ref_kinds.len()].join(",")
    );
    if let Some(max_rows) = max_rows {
        sql.push_str(" LIMIT ?");
        values.push(rusqlite::types::Value::Integer(
            i64::try_from(max_rows).unwrap_or(i64::MAX),
        ));
    }
    let mut statement = connection.prepare(&sql)?;
    let mut candidates = vec![];
    for row in statement.query_map(params_from_iter(values.iter()), document_from_row)? {
        let item = Candidate::linked(row?, "source-link");
        if let Some(trace) = trace.as_deref_mut() {
            trace.observe(&item, "source-link", "hydrated", &Value::Null);
        }
        candidates.push(item);
    }
    crate::source_policy::filter_in_scope(
        context, config, connection, policy, candidates, false, trace,
    )
}
#[derive(Clone)]
struct LinkEntry {
    rank: (u8, usize, usize),
    seeds: BTreeSet<String>,
    via: String,
    prompt: bool,
}
#[expect(
    clippy::too_many_arguments,
    reason = "Automatic expansion binds seed relations, filtered candidates, own paths, coverage, source policy and trace"
)]
pub fn automatic_link_candidates(
    connection: &Connection,
    context: &RuntimeContext,
    config: &RuntimeConfig,
    seeds: &[(String, String)],
    filtered: &[Candidate],
    own_paths: &BTreeSet<String>,
    coverage: &BTreeMap<String, usize>,
    policy: &mut crate::source_policy::CurrentSourcePolicy,
    mut trace: Option<&mut Trace>,
) -> Result<(Vec<Candidate>, Vec<Exclusion>)> {
    let blocked = filtered
        .iter()
        .filter(|item| item.match_strength.as_deref() != Some("distinctive"))
        .map(|item| item.document.path.clone())
        .chain(own_paths.iter().cloned())
        .collect::<BTreeSet<_>>();
    let mut found = BTreeMap::<String, LinkEntry>::new();
    for (index, (seed, origin)) in seeds.iter().enumerate() {
        for (kind_rank, kind) in ["source", "file"].into_iter().enumerate() {
            let mut statement=connection.prepare("SELECT path FROM document_links INDEXED BY document_links_source_path WHERE ref_path=?1 AND ref_kind=?2 ORDER BY path LIMIT ?3")?;
            let rows = statement
                .query_map(
                    params![seed, kind, FILE_LINK_HUB_LIMIT + 1 + own_paths.len()],
                    |row| row.get::<_, String>(0),
                )?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let others = rows
                .into_iter()
                .filter(|path| !own_paths.contains(path))
                .collect::<Vec<_>>();
            if others.len() > FILE_LINK_HUB_LIMIT {
                if let Some(trace) = trace.as_deref_mut() {
                    for path in others {
                        trace.known_event(&path, "file-link", "hub-limit");
                    }
                }
                continue;
            }
            for path in others {
                if blocked.contains(&path) {
                    if let Some(trace) = trace.as_deref_mut() {
                        trace.known_event(&path, "file-link", "already-selected-or-own");
                    }
                    continue;
                }
                let rank = (if origin == "prompt" { 0 } else { 1 }, kind_rank, index);
                let entry = found.entry(path).or_insert_with(|| LinkEntry {
                    rank,
                    seeds: BTreeSet::new(),
                    via: seed.clone(),
                    prompt: false,
                });
                if rank < entry.rank {
                    entry.rank = rank;
                    entry.via = seed.clone();
                }
                entry.seeds.insert(seed.clone());
                entry.prompt |= origin == "prompt";
            }
        }
    }
    let mut pool = found.keys().cloned().collect::<Vec<_>>();
    pool.sort_by(|a, b| {
        found[a]
            .rank
            .cmp(&found[b].rank)
            .then_with(|| found[b].seeds.len().cmp(&found[a].seeds.len()))
            .then_with(|| a.cmp(b))
    });
    if let Some(trace) = trace.as_deref_mut() {
        for path in pool.iter().skip(FILE_LINK_POOL) {
            trace.known_event(path, "file-link", "pool-limit");
        }
    }
    pool.truncate(FILE_LINK_POOL);
    if pool.is_empty() {
        return Ok((vec![], vec![]));
    }
    let mut candidates = vec![];
    for item in hydrate(connection, &pool)? {
        if let Some(trace) = trace.as_deref_mut() {
            trace.observe(&item, "file-link", "hydrated", &Value::Null);
        }
        let reason = if item.document.layer != "semantic"
            || !["durable", "dynamic"].contains(&item.document.category.as_str())
        {
            Some("category")
        } else if !item.document.conflicts.is_empty()
            || incoming_conflict(connection, item.document.record_id.as_deref())?
        {
            Some("conflict")
        } else if !found[&item.document.path].prompt
            && coverage.get(&item.document.path).copied().unwrap_or(0) < 1
        {
            Some("touch-without-query-term")
        } else {
            None
        };
        if let Some(reason) = reason {
            if let Some(trace) = trace.as_deref_mut() {
                trace.observe(&item, "file-link", reason, &Value::Null);
            }
            continue;
        }
        candidates.push(item);
    }
    let (mut eligible, _) = crate::source_policy::filter_in_scope(
        context, config, connection, policy, candidates, false, trace,
    )?;
    eligible.sort_by(|a, b| {
        found[&a.document.path]
            .rank
            .cmp(&found[&b.document.path].rank)
            .then_with(|| (!a.source_changed.is_empty()).cmp(&(!b.source_changed.is_empty())))
            .then_with(|| {
                inverse_key(&a.document.updated_at).cmp(&inverse_key(&b.document.updated_at))
            })
            .then_with(|| a.document.path.cmp(&b.document.path))
    });
    for item in &mut eligible {
        let entry = &found[&item.document.path];
        item.selection = Some(
            if entry.prompt {
                "prompt-link"
            } else {
                "touch-link"
            }
            .into(),
        );
        item.via = Some(entry.via.clone());
        item.match_strength = None;
        item.rank = None;
        item.adjusted_score = 0.0;
        item.snippet = query::body_excerpt(&item.image().content, 320, &[], false);
        item.estimated_tokens =
            query::estimated_tokens(&format!("{}{}", item.document.title, item.snippet));
    }
    let excluded = eligible
        .iter()
        .skip(FILE_LINK_LIMIT)
        .map(|item| Exclusion {
            path: item.document.path.clone(),
            reason: "file-link-limit".into(),
        })
        .collect();
    eligible.truncate(FILE_LINK_LIMIT);
    Ok((eligible, excluded))
}
pub fn insert_automatic_links(filtered: Vec<Candidate>, linked: Vec<Candidate>) -> Vec<Candidate> {
    if linked.is_empty() {
        return filtered;
    }
    let moved = linked
        .iter()
        .map(|item| item.document.path.clone())
        .collect::<BTreeSet<_>>();
    let mut rest = filtered
        .into_iter()
        .filter(|item| !moved.contains(&item.document.path))
        .collect::<Vec<_>>();
    let position = rest
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            item.selection.as_deref() == Some("path-link")
                || item.document.category != "policy"
                    && item.document.layer != "episodic"
                    && item.match_strength.as_deref() == Some("covered")
        })
        .map(|(index, _)| index + 1)
        .next_back()
        .unwrap_or(0);
    rest.splice(position..position, linked);
    rest
}

fn declared_sources(item: &Candidate) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    for entry in item
        .document
        .source_fingerprints
        .as_array()
        .into_iter()
        .flatten()
    {
        if let (Some(path), Some(digest)) = (entry["path"].as_str(), entry["sha256"].as_str()) {
            result.insert(path.into(), digest.into());
        }
    }
    result
}
#[expect(
    clippy::too_many_arguments,
    reason = "Shared-source hydration binds anchor, limits, digest cache, current-source policy and metadata trace"
)]
pub fn shared_source_documents(
    connection: &Connection,
    context: &RuntimeContext,
    config: &RuntimeConfig,
    anchor: &Candidate,
    max_rows: usize,
    current_digests: &mut BTreeMap<String, Option<String>>,
    policy: &mut crate::source_policy::CurrentSourcePolicy,
    mut trace: Option<&mut Trace>,
) -> Result<(Vec<Candidate>, usize)> {
    if max_rows == 0 {
        return Ok((vec![], 0));
    }
    let declared = declared_sources(anchor);
    let references = declared
        .iter()
        .filter(|(path, digest)| {
            domain::source_path_problem(path).is_none() && crate::observability::hex(digest)
        })
        .map(|(path, _)| path.clone())
        .take(GRAPH_ROW_LIMIT)
        .collect::<Vec<_>>();
    let mut paths = BTreeSet::new();
    for reference in &references {
        if connection.query_row("SELECT 1 FROM document_links WHERE path=?1 AND ref_path=?2 AND ref_kind='source' LIMIT 1",params![anchor.document.path,reference],|_|Ok(())).optional()?.is_none(){continue;}
        let mut excluded = paths.clone();
        excluded.insert(anchor.document.path.clone());
        let mut values = vec![rusqlite::types::Value::Text(reference.clone())];
        values.extend(
            excluded
                .iter()
                .map(|path| rusqlite::types::Value::Text(path.clone())),
        );
        values.push(rusqlite::types::Value::Integer(
            (max_rows - paths.len()) as i64,
        ));
        let sql = format!(
            "SELECT path FROM document_links INDEXED BY document_links_source_path WHERE ref_path=? AND ref_kind='source' AND path NOT IN ({}) ORDER BY path LIMIT ?",
            vec!["?"; excluded.len()].join(",")
        );
        let mut statement = connection.prepare(&sql)?;
        for row in statement.query_map(params_from_iter(values.iter()), |row| {
            row.get::<_, String>(0)
        })? {
            paths.insert(row?);
        }
        if paths.len() >= max_rows {
            break;
        }
    }
    if paths.is_empty() {
        return Ok((vec![], 0));
    }
    let count = paths.len();
    let mut candidates = vec![];
    for item in hydrate(connection, &paths.into_iter().collect::<Vec<_>>())? {
        if let Some(trace) = trace.as_deref_mut() {
            trace.observe(&item, "shared-source", "hydrated", &Value::Null);
        }
        let peer = declared_sources(&item);
        let mut witnessed = false;
        for reference in &references {
            if peer.get(reference) != declared.get(reference) {
                continue;
            }
            if connection.query_row("SELECT 1 FROM document_links a JOIN document_links b ON b.ref_path=a.ref_path WHERE a.path=?1 AND b.path=?2 AND a.ref_path=?3 AND a.ref_kind='source' AND b.ref_kind='source' LIMIT 1",params![anchor.document.path,item.document.path,reference],|_|Ok(())).optional()?.is_none(){continue;}
            if !current_digests.contains_key(reference) {
                let digest = (|| -> Result<Option<String>> {
                    let mut source = context.roots.project.clone();
                    for component in Path::new(reference).components() {
                        source.push(component);
                        let status = std::fs::symlink_metadata(&source)?;
                        if crate::fs::linked_at(&source, &status)? {
                            return Ok(None);
                        }
                    }
                    let metadata = std::fs::metadata(&source)?;
                    if !metadata.is_file() || metadata.len() > GRAPH_SOURCE_BYTE_LIMIT {
                        return Ok(None);
                    }
                    let fingerprints = domain::source_fingerprints(
                        &context.roots,
                        std::slice::from_ref(reference),
                    )?;
                    Ok(fingerprints
                        .first()
                        .and_then(|entry| entry["sha256"].as_str())
                        .map(String::from))
                })();
                let digest = match digest {
                    Ok(digest) => digest,
                    Err(error) if error.code.as_deref() == Some("source-git") => return Err(error),
                    Err(_) => None,
                };
                current_digests.insert(reference.clone(), digest);
            }
            if current_digests.get(reference).and_then(Option::as_ref) == declared.get(reference) {
                witnessed = true;
                break;
            }
        }
        if witnessed {
            candidates.push(item);
        } else if let Some(trace) = trace.as_deref_mut() {
            trace.observe(
                &item,
                "shared-source",
                "no-current-shared-revision",
                &Value::Null,
            );
        }
    }
    let (eligible, _) = crate::source_policy::filter_in_scope(
        context, config, connection, policy, candidates, false, trace,
    )?;
    Ok((eligible, count))
}
#[derive(Clone, Debug, Default)]
pub struct GraphStats {
    pub anchors: usize,
    pub examined: usize,
    pub eligible: usize,
    pub withheld: usize,
    pub truncated: bool,
}
pub fn source_link_candidates(
    connection: &Connection,
    context: &RuntimeContext,
    config: &RuntimeConfig,
    anchors: &[Candidate],
    seen: &BTreeSet<String>,
    policy: &mut crate::source_policy::CurrentSourcePolicy,
    mut trace: Option<&mut Trace>,
) -> Result<(Vec<Candidate>, GraphStats)> {
    let mut stats = GraphStats::default();
    let mut candidates = vec![];
    let mut seen = seen.clone();
    let mut current_digests = BTreeMap::new();
    for anchor in anchors.iter().take(GRAPH_ANCHOR_LIMIT) {
        if !anchor.document.conflicts.is_empty()
            || incoming_conflict(connection, anchor.document.record_id.as_deref())?
        {
            if let Some(trace) = trace.as_deref_mut() {
                trace.observe(anchor, "source-link", "anchor-conflict", &Value::Null);
            }
            continue;
        }
        let (eligible, _) = crate::source_policy::filter_in_scope(
            context,
            config,
            connection,
            policy,
            vec![anchor.clone()],
            false,
            trace.as_deref_mut(),
        )?;
        if eligible.is_empty() {
            continue;
        }
        let remaining = GRAPH_ROW_LIMIT.saturating_sub(stats.examined);
        if remaining == 0 {
            stats.truncated = true;
            break;
        }
        let (mut linked, withheld) = linked_documents(
            connection,
            context,
            config,
            &anchor.document.path,
            false,
            Some(remaining),
            &["source"],
            policy,
            trace.as_deref_mut(),
        )?;
        stats.anchors += 1;
        let count = linked.len() + withheld.len();
        stats.examined += count;
        stats.withheld += withheld.len();
        stats.truncated |= count == remaining;
        let remaining = remaining.saturating_sub(count);
        if remaining > 0 {
            let (shared, rows) = shared_source_documents(
                connection,
                context,
                config,
                anchor,
                remaining,
                &mut current_digests,
                policy,
                trace.as_deref_mut(),
            )?;
            linked.extend(shared);
            stats.examined += rows;
            stats.truncated |= rows == remaining;
        }
        for mut item in linked {
            let reason = if seen.contains(&item.document.path) {
                Some("already-seen")
            } else if item.document.layer != "semantic"
                || !["durable", "dynamic"].contains(&item.document.category.as_str())
            {
                Some("category")
            } else if !item.document.conflicts.is_empty()
                || incoming_conflict(connection, item.document.record_id.as_deref())?
            {
                Some("conflict")
            } else {
                None
            };
            if let Some(reason) = reason {
                if let Some(trace) = trace.as_deref_mut() {
                    trace.observe(&item, "source-link", reason, &Value::Null);
                }
                continue;
            }
            seen.insert(item.document.path.clone());
            stats.eligible += 1;
            if candidates.len() >= GRAPH_CANDIDATE_LIMIT {
                stats.truncated = true;
                if let Some(trace) = trace.as_deref_mut() {
                    trace.observe(&item, "source-link", "candidate-limit", &Value::Null);
                }
                continue;
            }
            item.snippet = query::body_excerpt(&item.image().content, 320, &[], false);
            item.match_strength = None;
            item.selection = Some("source-link".into());
            item.adjusted_score = 0.0;
            item.rank = None;
            item.estimated_tokens =
                query::estimated_tokens(&format!("{}{}", item.document.title, item.snippet));
            candidates.push(item);
        }
    }
    Ok((candidates, stats))
}
/// Lightweight lexical callers rehydrate their first strong semantic rows;
/// an old cached public SearchItem is never sufficient authorization.
pub fn source_link_candidates_for_paths(
    connection: &Connection,
    context: &RuntimeContext,
    anchors: &[(String, Option<String>)],
    seen: &BTreeSet<String>,
) -> Result<Vec<Candidate>> {
    let paths = anchors
        .iter()
        .filter(|(_, strength)| {
            strength
                .as_deref()
                .is_none_or(|strength| strength == "covered")
        })
        .take(GRAPH_ANCHOR_LIMIT)
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    let mut hydrated = vec![];
    for path in paths {
        hydrated.extend(
            hydrate(connection, &[path])?
                .into_iter()
                .filter(|item| item.document.layer == "semantic"),
        );
    }
    let config = RuntimeConfig::load(&context.roots)?;
    let mut policy = crate::source_policy::CurrentSourcePolicy::default();
    Ok(source_link_candidates(
        connection,
        context,
        &config,
        &hydrated,
        seen,
        &mut policy,
        None,
    )?
    .0)
}

pub fn path_candidates(
    connection: &Connection,
    context: &RuntimeContext,
    config: &RuntimeConfig,
    paths: &[String],
    seen: &BTreeSet<String>,
    policy: &mut crate::source_policy::CurrentSourcePolicy,
    mut trace: Option<&mut Trace>,
) -> Result<(Vec<Candidate>, Vec<Exclusion>)> {
    let mut input_seen = BTreeSet::new();
    let mut seen = seen.clone();
    let mut candidates = vec![];
    let mut excluded = vec![];
    for reference in paths.iter().filter(|path| input_seen.insert(path.as_str())) {
        let (linked, withheld) = linked_documents(
            connection,
            context,
            config,
            reference,
            false,
            None,
            &["source"],
            policy,
            trace.as_deref_mut(),
        )?;
        excluded.extend(withheld);
        for item in linked {
            if seen.insert(item.document.path.clone()) {
                candidates.push(item);
            }
        }
    }
    excluded.extend(
        candidates
            .iter()
            .skip(GRAPH_CANDIDATE_LIMIT)
            .map(|item| Exclusion {
                path: item.document.path.clone(),
                reason: "path-link-limit".into(),
            }),
    );
    candidates.truncate(GRAPH_CANDIDATE_LIMIT);
    for item in &mut candidates {
        item.snippet = query::body_excerpt(&item.image().content, 320, &[], false);
        item.match_strength = None;
        item.selection = Some("path-link".into());
        item.adjusted_score = 0.0;
        item.rank = None;
        item.estimated_tokens =
            query::estimated_tokens(&format!("{}{}", item.document.title, item.snippet));
    }
    Ok((candidates, excluded))
}
