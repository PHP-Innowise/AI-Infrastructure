use super::*;
use std::{
    collections::VecDeque,
    fs,
    io::Read,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant, UNIX_EPOCH},
};

const LISTED: usize = 8;
const VISITS: usize = 250_000;
const MAX_COVERAGE_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_COVERAGE_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
const MAX_COVERAGE_FILES: usize = 16;
const MAX_MONOREPO_COVERAGE_DIRS: usize = 32;
const MAX_CACHED_COVERAGE_ROOTS: usize = 8;
const MAX_COVERAGE_TESTS_PER_FILE: usize = 64;
const MAX_COVERAGE_FILE_ENTRIES: usize = 20_000;
const MAX_LCOV_LINES: usize = 500_000;
const MAX_XML_ATTRS: usize = 64;
const MAX_XML_TAG_BYTES: usize = 16 * 1024;

const MONOREPO_WORKSPACES: &[&str] = &["packages", "services", "apps", "crates", "modules"];
const MONOREPO_COVERAGE_REL: &[(&str, CoverageFormat)] = &[
    ("coverage/lcov.info", CoverageFormat::Lcov),
    ("lcov.info", CoverageFormat::Lcov),
    ("coverage/clover.xml", CoverageFormat::Clover),
    ("clover.xml", CoverageFormat::Clover),
];

const COVERAGE_CANDIDATES_LCOV: &[&str] = &[
    "lcov.info",
    "coverage/lcov.info",
    "coverage.info",
    ".coverage/lcov.info",
    "target/coverage/lcov.info",
    "reports/lcov.info",
    "build/logs/lcov.info",
    "var/coverage/lcov.info",
];

const COVERAGE_CANDIDATES_CLOVER: &[&str] = &[
    "clover.xml",
    "coverage/clover.xml",
    "build/logs/clover.xml",
    "target/coverage/clover.xml",
    "reports/clover.xml",
    "coverage/coverage.xml",
    "coverage.xml",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CoverageFormat {
    Lcov,
    Clover,
}

impl CoverageFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Lcov => "lcov",
            Self::Clover => "clover",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileCoverage {
    format: CoverageFormat,
    covered_lines: usize,
    total_lines: usize,
    tests: Vec<String>,
    test_hits: BTreeSet<String>,
}

impl FileCoverage {
    fn rate(&self) -> f64 {
        if self.total_lines == 0 {
            0.0
        } else {
            ((self.covered_lines as f64 / self.total_lines as f64) * 10_000.0).round() / 10_000.0
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "format": self.format.as_str(),
            "covered_lines": self.covered_lines,
            "total_lines": self.total_lines,
            "rate": self.rate(),
            "tests": self.tests,
        })
    }
}

#[derive(Clone, Debug, Default)]
struct CoverageReport {
    files: BTreeMap<String, FileCoverage>,
    all_tests: BTreeSet<String>,
}

fn normalize_root_text(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let trimmed = raw.trim_end_matches('/');
    if trimmed.is_empty() && raw.starts_with('/') {
        "/".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn is_absolute_coverage_path(norm: &str) -> bool {
    norm.starts_with('/')
        || norm.starts_with("//")
        || (norm.len() >= 3
            && norm.as_bytes()[0].is_ascii_alphabetic()
            && norm.as_bytes()[1] == b':'
            && norm.as_bytes()[2] == b'/')
}

fn strip_root_prefix<'a>(path: &'a str, root: &str) -> Option<&'a str> {
    if root.is_empty() {
        return None;
    }
    if root == "/" {
        return path.strip_prefix('/');
    }
    path.strip_prefix(root)
        .and_then(|rest| rest.strip_prefix('/'))
}

/// Normalize a source file path from an LCOV or Clover report relative to `root`,
/// rejecting absolute paths outside `root` and any `..` traversal components.
fn normalize_coverage_path(raw: &str, root: &Path, canon_root: Option<&Path>) -> Option<String> {
    let cleaned = raw.trim().replace('\\', "/");
    if cleaned.is_empty() || cleaned.contains('\0') {
        return None;
    }
    let rel = if is_absolute_coverage_path(&cleaned) {
        let root_str = normalize_root_text(root);
        let canon_str = canon_root.map(normalize_root_text);
        strip_root_prefix(&cleaned, &root_str).or_else(|| {
            canon_str
                .as_deref()
                .and_then(|canon| strip_root_prefix(&cleaned, canon))
        })?
    } else {
        cleaned.as_str()
    };

    let mut parts = Vec::new();
    for segment in rel.split('/') {
        match segment {
            "" | "." => continue,
            ".." => return None,
            part => parts.push(part),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

#[derive(Clone)]
struct CachedCoverageEntry {
    canon_root: PathBuf,
    stamps: Vec<(String, u64, u128)>,
    report: CoverageReport,
}

#[derive(Clone)]
struct CachedNegativeCoverageEntry {
    canon_root: PathBuf,
    dir_stamps: Vec<(String, u128)>,
    checked_at: Instant,
}

fn coverage_report_cache() -> &'static Mutex<Vec<CachedCoverageEntry>> {
    static CACHE: OnceLock<Mutex<Vec<CachedCoverageEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

fn negative_coverage_cache() -> &'static Mutex<Vec<CachedNegativeCoverageEntry>> {
    static CACHE: OnceLock<Mutex<Vec<CachedNegativeCoverageEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

const COVERAGE_PROBE_PARENTS: &[&str] = &[
    "",
    "coverage",
    ".coverage",
    "reports",
    "target/coverage",
    "build/logs",
    "var/coverage",
    "packages",
    "services",
    "apps",
    "crates",
    "modules",
];
const NEGATIVE_COVERAGE_TTL: Duration = Duration::from_secs(5);

fn coverage_parent_dir_stamps(root: &Path) -> Vec<(String, u128)> {
    COVERAGE_PROBE_PARENTS
        .iter()
        .filter_map(|&rel| {
            let dir = if rel.is_empty() {
                root.to_path_buf()
            } else {
                root.join(rel)
            };
            let meta = fs::symlink_metadata(&dir).ok()?;
            if !meta.is_dir() {
                return None;
            }
            let mtime_ns = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0u128, |d| d.as_nanos());
            Some((rel.to_owned(), mtime_ns))
        })
        .collect()
}

fn stat_bounded_coverage_file(root: &Path, canon_root: &Path, rel: &str) -> Option<(u64, u128)> {
    if rel.is_empty()
        || rel.starts_with('/')
        || rel.contains('\\')
        || rel.split('/').any(|part| part.is_empty() || part == "..")
    {
        return None;
    }
    let candidate = root.join(rel);
    let canon_file = infra_core::fs::canonicalize(&candidate).ok()?;
    if !canon_file.starts_with(canon_root) {
        return None;
    }
    let meta = fs::symlink_metadata(&canon_file).ok()?;
    if !meta.is_file() || meta.len() > MAX_COVERAGE_FILE_BYTES {
        return None;
    }
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0u128, |d| d.as_nanos());
    Some((meta.len(), mtime_ns))
}

fn read_bounded_coverage_file(root: &Path, canon_root: &Path, rel: &str) -> Option<(String, u64)> {
    stat_bounded_coverage_file(root, canon_root, rel)?;
    let canon_file = infra_core::fs::canonicalize(root.join(rel)).ok()?;
    let meta = fs::symlink_metadata(&canon_file).ok()?;
    let file = fs::File::open(&canon_file).ok()?;
    let mut buf = Vec::with_capacity(meta.len().min(MAX_COVERAGE_FILE_BYTES) as usize);
    file.take(MAX_COVERAGE_FILE_BYTES + 1)
        .read_to_end(&mut buf)
        .ok()?;
    if buf.len() as u64 > MAX_COVERAGE_FILE_BYTES {
        return None;
    }
    let bytes_read = buf.len() as u64;
    let text = String::from_utf8(buf).ok()?;
    Some((text, bytes_read))
}

#[derive(Default)]
struct LcovFileAccum {
    instrumented: BTreeSet<u32>,
    covered: BTreeSet<u32>,
    max_lf: usize,
    max_lh: usize,
    saw_da: bool,
    test_hits: BTreeSet<String>,
}

fn parse_lcov(text: &str, root: &Path, canon_root: Option<&Path>, report: &mut CoverageReport) {
    let mut accum: BTreeMap<String, LcovFileAccum> = BTreeMap::new();
    let mut current_tn: Option<String> = None;
    let mut current_sf: Option<String> = None;
    let mut rec_da: Vec<(u32, u64)> = Vec::new();
    let mut rec_lf: Option<usize> = None;
    let mut rec_lh: Option<usize> = None;

    let flush = |accum: &mut BTreeMap<String, LcovFileAccum>,
                 all_tests: &mut BTreeSet<String>,
                 current_tn: &mut Option<String>,
                 current_sf: &mut Option<String>,
                 rec_da: &mut Vec<(u32, u64)>,
                 rec_lf: &mut Option<usize>,
                 rec_lh: &mut Option<usize>,
                 reset_tn: bool| {
        if let Some(rel) = current_sf.take()
            && (accum.contains_key(&rel) || accum.len() < MAX_COVERAGE_FILE_ENTRIES)
        {
            let entry = accum.entry(rel).or_default();
            let mut rec_hit = false;
            if !rec_da.is_empty() {
                entry.saw_da = true;
                for &(line, hits) in rec_da.iter() {
                    entry.instrumented.insert(line);
                    if hits > 0 {
                        entry.covered.insert(line);
                        rec_hit = true;
                    }
                }
            }
            if let Some(lf) = *rec_lf {
                entry.max_lf = entry.max_lf.max(lf);
            }
            if let Some(lh) = *rec_lh {
                entry.max_lh = entry.max_lh.max(lh);
                if rec_da.is_empty() && lh > 0 {
                    rec_hit = true;
                }
            }
            if let Some(tn) = current_tn.as_ref() {
                all_tests.insert(tn.clone());
                if rec_hit && entry.test_hits.len() < 512 {
                    entry.test_hits.insert(tn.clone());
                }
            }
        }
        rec_da.clear();
        *rec_lf = None;
        *rec_lh = None;
        if reset_tn {
            *current_tn = None;
        }
    };

    for (idx, raw_line) in text.lines().enumerate() {
        if idx >= MAX_LCOV_LINES {
            break;
        }
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "end_of_record" {
            flush(
                &mut accum,
                &mut report.all_tests,
                &mut current_tn,
                &mut current_sf,
                &mut rec_da,
                &mut rec_lf,
                &mut rec_lh,
                true,
            );
            continue;
        }
        let Some((tag, rest)) = line.split_once(':') else {
            continue;
        };
        match tag {
            "TN" => {
                let tn = clip(rest.trim(), 160).to_owned();
                if tn.is_empty() {
                    current_tn = None;
                } else {
                    report.all_tests.insert(tn.clone());
                    current_tn = Some(tn);
                }
            }
            "SF" => {
                if current_sf.is_some() {
                    flush(
                        &mut accum,
                        &mut report.all_tests,
                        &mut current_tn,
                        &mut current_sf,
                        &mut rec_da,
                        &mut rec_lf,
                        &mut rec_lh,
                        false,
                    );
                }
                current_sf = normalize_coverage_path(rest, root, canon_root);
            }
            "DA" if current_sf.is_some() => {
                let mut parts = rest.split(',');
                if let (Some(line_str), Some(hits_str)) = (parts.next(), parts.next())
                    && let (Ok(line_no), Ok(hits)) = (
                        line_str.trim().parse::<u32>(),
                        hits_str.trim().parse::<u64>(),
                    )
                {
                    rec_da.push((line_no, hits));
                }
            }
            "LF" if current_sf.is_some() => {
                if let Ok(lf) = rest.trim().parse::<usize>() {
                    rec_lf = Some(lf);
                }
            }
            "LH" if current_sf.is_some() => {
                if let Ok(lh) = rest.trim().parse::<usize>() {
                    rec_lh = Some(lh);
                }
            }
            _ => {}
        }
    }

    flush(
        &mut accum,
        &mut report.all_tests,
        &mut current_tn,
        &mut current_sf,
        &mut rec_da,
        &mut rec_lf,
        &mut rec_lh,
        true,
    );

    for (path, acc) in accum {
        let total_lines = if acc.saw_da {
            acc.instrumented.len().max(acc.max_lf)
        } else {
            acc.max_lf
        };
        let covered_lines = if acc.saw_da {
            acc.covered.len().min(total_lines)
        } else {
            acc.max_lh.min(total_lines)
        };
        let tests = acc
            .test_hits
            .iter()
            .take(MAX_COVERAGE_TESTS_PER_FILE)
            .cloned()
            .collect::<Vec<_>>();
        match report.files.get_mut(&path) {
            Some(existing) if existing.format == CoverageFormat::Lcov => {
                existing.total_lines = existing.total_lines.max(total_lines);
                existing.covered_lines = existing
                    .covered_lines
                    .max(covered_lines)
                    .min(existing.total_lines);
                for tn in acc.test_hits {
                    if existing.test_hits.len() < 512 {
                        existing.test_hits.insert(tn);
                    }
                }
                existing.tests = existing
                    .test_hits
                    .iter()
                    .take(MAX_COVERAGE_TESTS_PER_FILE)
                    .cloned()
                    .collect();
            }
            _ => {
                report.files.insert(
                    path,
                    FileCoverage {
                        format: CoverageFormat::Lcov,
                        covered_lines,
                        total_lines,
                        tests,
                        test_hits: acc.test_hits,
                    },
                );
            }
        }
    }
}

fn unescape_xml(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_owned();
    }
    raw.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn find_tag_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut quote: Option<u8> = None;
    let limit = (start + MAX_XML_TAG_BYTES).min(bytes.len());
    let mut idx = start;
    while idx < limit {
        let b = bytes[idx];
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'>' => return Some(idx),
            None => {}
        }
        idx += 1;
    }
    None
}

fn parse_xml_attrs(raw: &str) -> Vec<(String, String)> {
    let bytes = raw.as_bytes();
    let mut attrs = Vec::new();
    let mut idx = 0usize;
    while idx < bytes.len() && attrs.len() < MAX_XML_ATTRS {
        while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx >= bytes.len() {
            break;
        }
        let key_start = idx;
        while idx < bytes.len()
            && !bytes[idx].is_ascii_whitespace()
            && bytes[idx] != b'='
            && bytes[idx] != b'/'
        {
            idx += 1;
        }
        if idx == key_start {
            idx += 1;
            continue;
        }
        let key = raw[key_start..idx].to_ascii_lowercase();
        while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx >= bytes.len() || bytes[idx] != b'=' {
            continue;
        }
        idx += 1;
        while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx >= bytes.len() {
            break;
        }
        let quote = bytes[idx];
        if quote != b'"' && quote != b'\'' {
            continue;
        }
        idx += 1;
        let val_start = idx;
        while idx < bytes.len() && bytes[idx] != quote {
            idx += 1;
        }
        if idx >= bytes.len() {
            break;
        }
        let val = unescape_xml(&raw[val_start..idx]);
        attrs.push((key, val));
        idx += 1;
    }
    attrs
}

fn xml_attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

#[derive(Default)]
struct ActiveCloverFile {
    rel_path: Option<String>,
    depth: usize,
    stmt_lines: BTreeMap<u32, u64>,
    all_lines: BTreeMap<u32, u64>,
    metrics_statements: Option<usize>,
    metrics_covered: Option<usize>,
}

fn finalize_clover_file(active: ActiveCloverFile, report: &mut CoverageReport) {
    let Some(rel_path) = active.rel_path else {
        return;
    };
    if report.files.contains_key(&rel_path) || report.files.len() >= MAX_COVERAGE_FILE_ENTRIES {
        return;
    }
    let (total_lines, covered_lines) = match (active.metrics_statements, active.metrics_covered) {
        (Some(total), Some(covered)) if total > 0 || active.all_lines.is_empty() => {
            (total, covered.min(total))
        }
        _ if !active.stmt_lines.is_empty() => {
            let total = active.stmt_lines.len();
            let covered = active.stmt_lines.values().filter(|c| **c > 0).count();
            (total, covered)
        }
        _ => {
            let total = active.all_lines.len();
            let covered = active.all_lines.values().filter(|c| **c > 0).count();
            (total, covered)
        }
    };
    report.files.insert(
        rel_path,
        FileCoverage {
            format: CoverageFormat::Clover,
            covered_lines,
            total_lines,
            tests: Vec::new(),
            test_hits: BTreeSet::new(),
        },
    );
}

fn parse_clover(text: &str, root: &Path, canon_root: Option<&Path>, report: &mut CoverageReport) {
    let bytes = text.as_bytes();
    let mut pos = 0usize;
    let mut active: Option<ActiveCloverFile> = None;

    while pos < bytes.len() {
        let Some(rel_lt) = text[pos..].find('<') else {
            break;
        };
        let lt = pos + rel_lt;
        if text[lt..].starts_with("<!--") {
            pos = text[lt + 4..]
                .find("-->")
                .map_or(bytes.len(), |end| lt + 4 + end + 3);
            continue;
        }
        if text[lt..].starts_with("<![CDATA[") {
            pos = text[lt + 9..]
                .find("]]>")
                .map_or(bytes.len(), |end| lt + 9 + end + 3);
            continue;
        }
        if text[lt..].starts_with("<?") {
            pos = text[lt + 2..]
                .find("?>")
                .map_or(bytes.len(), |end| lt + 2 + end + 2);
            continue;
        }
        let Some(gt) = find_tag_end(bytes, lt + 1) else {
            break;
        };
        pos = gt + 1;
        let inner = text[lt + 1..gt].trim();
        if inner.is_empty() || inner.starts_with('!') {
            continue;
        }
        let closing = inner.starts_with('/');
        let self_closing = !closing && inner.ends_with('/');
        let body = inner.trim_start_matches('/').trim_end_matches('/').trim();
        let name_end = body
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(body.len());
        let tag_name = body[..name_end].to_ascii_lowercase();
        let attrs_str = &body[name_end..];

        if closing {
            if let Some(mut cur) = active.take() {
                if cur.depth == 0 && tag_name == "file" {
                    finalize_clover_file(cur, report);
                } else {
                    cur.depth = cur.depth.saturating_sub(1);
                    active = Some(cur);
                }
            }
            continue;
        }

        match tag_name.as_str() {
            "file" if active.is_none() => {
                let attrs = parse_xml_attrs(attrs_str);
                let raw_path = xml_attr(&attrs, "path")
                    .filter(|s| !s.trim().is_empty())
                    .or_else(|| xml_attr(&attrs, "name"));
                let rel_path =
                    raw_path.and_then(|raw| normalize_coverage_path(raw, root, canon_root));
                if !self_closing {
                    active = Some(ActiveCloverFile {
                        rel_path,
                        ..Default::default()
                    });
                }
            }
            "line" => {
                if let Some(cur) = active.as_mut() {
                    let attrs = parse_xml_attrs(attrs_str);
                    if let (Some(num_str), Some(count_str)) =
                        (xml_attr(&attrs, "num"), xml_attr(&attrs, "count"))
                        && let (Ok(num), Ok(count)) = (
                            num_str.trim().parse::<u32>(),
                            count_str.trim().parse::<u64>(),
                        )
                    {
                        cur.all_lines
                            .entry(num)
                            .and_modify(|existing| *existing = (*existing).max(count))
                            .or_insert(count);
                        let line_type = xml_attr(&attrs, "type").unwrap_or("stmt");
                        if line_type.eq_ignore_ascii_case("stmt") {
                            cur.stmt_lines
                                .entry(num)
                                .and_modify(|existing| *existing = (*existing).max(count))
                                .or_insert(count);
                        }
                    }
                    if !self_closing {
                        cur.depth += 1;
                    }
                }
            }
            "metrics" => {
                if let Some(cur) = active.as_mut() {
                    if cur.depth == 0 {
                        let attrs = parse_xml_attrs(attrs_str);
                        let stmts = xml_attr(&attrs, "statements")
                            .and_then(|v| v.trim().parse::<usize>().ok());
                        let cov_stmts = xml_attr(&attrs, "coveredstatements")
                            .and_then(|v| v.trim().parse::<usize>().ok());
                        let (total, covered) = match (stmts, cov_stmts) {
                            (Some(s), Some(c)) => (Some(s), Some(c)),
                            _ => (
                                xml_attr(&attrs, "elements")
                                    .and_then(|v| v.trim().parse::<usize>().ok()),
                                xml_attr(&attrs, "coveredelements")
                                    .and_then(|v| v.trim().parse::<usize>().ok()),
                            ),
                        };
                        if let (Some(total), Some(covered)) = (total, covered) {
                            cur.metrics_statements = Some(total);
                            cur.metrics_covered = Some(covered);
                        }
                    }
                    if !self_closing {
                        cur.depth += 1;
                    }
                }
            }
            _ => {
                if let Some(cur) = active.as_mut()
                    && !self_closing
                {
                    cur.depth += 1;
                }
            }
        }
    }
}

fn discover_workspace_coverage(root: &Path) -> Option<CoverageReport> {
    if root.as_os_str().is_empty() {
        return None;
    }
    let canon_root = infra_core::fs::canonicalize(root).ok()?;
    let dir_stamps = coverage_parent_dir_stamps(root);
    if let Ok(mut neg) = negative_coverage_cache().lock()
        && let Some(pos) = neg.iter().position(|entry| entry.canon_root == canon_root)
    {
        let hit = &neg[pos];
        if hit.dir_stamps == dir_stamps && hit.checked_at.elapsed() < NEGATIVE_COVERAGE_TTL {
            let entry = neg.remove(pos);
            neg.push(entry);
            return None;
        }
        neg.remove(pos);
    }

    let mut matched: Vec<(String, CoverageFormat, u64, u128)> = Vec::new();
    let mut seen_rels: BTreeSet<String> = BTreeSet::new();
    let push_candidate =
        |rel: String,
         fmt: CoverageFormat,
         seen_rels: &mut BTreeSet<String>,
         matched: &mut Vec<(String, CoverageFormat, u64, u128)>| {
            if matched.len() >= MAX_COVERAGE_FILES || !seen_rels.insert(rel.clone()) {
                return;
            }
            if let Some((len, mtime_ns)) = stat_bounded_coverage_file(root, &canon_root, &rel) {
                matched.push((rel, fmt, len, mtime_ns));
            }
        };

    for &rel in COVERAGE_CANDIDATES_LCOV {
        push_candidate(
            rel.to_owned(),
            CoverageFormat::Lcov,
            &mut seen_rels,
            &mut matched,
        );
    }
    for &rel in COVERAGE_CANDIDATES_CLOVER {
        push_candidate(
            rel.to_owned(),
            CoverageFormat::Clover,
            &mut seen_rels,
            &mut matched,
        );
    }

    let mut scanned_dirs = 0usize;
    for &parent in MONOREPO_WORKSPACES {
        if matched.len() >= MAX_COVERAGE_FILES || scanned_dirs >= MAX_MONOREPO_COVERAGE_DIRS {
            break;
        }
        let parent_dir = root.join(parent);
        let Ok(entries) = fs::read_dir(&parent_dir) else {
            continue;
        };
        let mut children: Vec<_> = entries.flatten().collect();
        children.sort_by_key(|e| e.file_name());
        for entry in children {
            if matched.len() >= MAX_COVERAGE_FILES || scanned_dirs >= MAX_MONOREPO_COVERAGE_DIRS {
                break;
            }
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if !ft.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let Some(name_str) = name.to_str() else {
                continue;
            };
            scanned_dirs += 1;
            for &(leaf, fmt) in MONOREPO_COVERAGE_REL {
                push_candidate(
                    format!("{parent}/{name_str}/{leaf}"),
                    fmt,
                    &mut seen_rels,
                    &mut matched,
                );
            }
        }
    }

    if matched.is_empty() {
        if let Ok(mut neg) = negative_coverage_cache().lock() {
            neg.retain(|entry| entry.canon_root != canon_root);
            if neg.len() >= MAX_CACHED_COVERAGE_ROOTS {
                neg.remove(0);
            }
            neg.push(CachedNegativeCoverageEntry {
                canon_root,
                dir_stamps,
                checked_at: Instant::now(),
            });
        }
        return None;
    }

    let stamps: Vec<(String, u64, u128)> = matched
        .iter()
        .map(|(rel, _, len, mtime_ns)| (rel.clone(), *len, *mtime_ns))
        .collect();
    if let Ok(mut guard) = coverage_report_cache().lock()
        && let Some(pos) = guard
            .iter()
            .position(|entry| entry.canon_root == canon_root && entry.stamps == stamps)
    {
        let hit = guard.remove(pos);
        let out = hit.report.clone();
        guard.push(hit);
        return (!out.files.is_empty() || !out.all_tests.is_empty()).then_some(out);
    }

    let mut report = CoverageReport::default();
    let mut bytes_loaded = 0u64;
    for (rel, fmt, _, _) in &matched {
        if bytes_loaded >= MAX_COVERAGE_TOTAL_BYTES {
            break;
        }
        if let Some((text, size)) = read_bounded_coverage_file(root, &canon_root, rel) {
            match fmt {
                CoverageFormat::Lcov => parse_lcov(&text, root, Some(&canon_root), &mut report),
                CoverageFormat::Clover => parse_clover(&text, root, Some(&canon_root), &mut report),
            }
            bytes_loaded += size;
        }
    }

    if let Ok(mut guard) = coverage_report_cache().lock() {
        guard.retain(|entry| entry.canon_root != canon_root);
        if guard.len() >= MAX_CACHED_COVERAGE_ROOTS {
            guard.remove(0);
        }
        guard.push(CachedCoverageEntry {
            canon_root,
            stamps,
            report: report.clone(),
        });
    }

    (!report.files.is_empty() || !report.all_tests.is_empty()).then_some(report)
}

struct Candidate {
    steps: u32,
    file: u32,
    symbol: Option<(u32, u32)>,
}

fn build_semantic_bridges(
    index: &Index,
    certain: bool,
) -> (HashMap<u32, Vec<u32>>, HashMap<u32, Vec<u32>>) {
    let mut hub_groups: HashMap<u32, (Vec<u32>, Vec<u32>)> = HashMap::new();
    let mut sym_bridges: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut file_bridges: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut add_pair = |prod: u32, cons: u32| {
        if prod == cons {
            return;
        }
        sym_bridges.entry(prod).or_default().push(cons);
        let prod_file = index.symbols[prod as usize].file;
        let cons_file = index.symbols[cons as usize].file;
        if prod_file != cons_file {
            file_bridges.entry(prod_file).or_default().push(cons_file);
        }
    };

    for (from, to, kind, guessed) in &index.uses {
        if certain && *guessed {
            continue;
        }
        let rel = index.kinds[*kind as usize].as_str();
        if index.symbols[*to as usize].is_semantic {
            let entry = hub_groups.entry(*to).or_default();
            match rel {
                "writes_table" | "table_write" | "publishes_event" | "exposes_route" | "route" => {
                    entry.0.push(*from);
                }
                "reads_table" | "table_read" | "handles_event" | "calls_route" => {
                    entry.1.push(*from);
                }
                "di_binds" | "di_binding" => {
                    entry.0.push(*from);
                    entry.1.push(*from);
                }
                _ => {}
            }
        } else if matches!(rel, "di_binds" | "di_binding") {
            // Direct Interface -> Impl link: edits to Interface can affect direct tests of Impl.
            add_pair(*from, *to);
        }
    }

    for (_, (prods, cons)) in hub_groups {
        for &p in &prods {
            for &c in &cons {
                add_pair(p, c);
            }
        }
    }
    for vals in sym_bridges.values_mut() {
        vals.sort_unstable();
        vals.dedup();
    }
    for vals in file_bridges.values_mut() {
        vals.sort_unstable();
        vals.dedup();
    }
    (sym_bridges, file_bridges)
}

fn walk_visit(
    other: u32,
    steps: u32,
    goal: u32,
    budget: &mut usize,
    seen: &mut HashSet<u32>,
    pending: &mut VecDeque<(u32, u32, u32)>,
) -> bool {
    if let Some(left) = budget.checked_sub(1) {
        *budget = left;
        if seen.insert(other) {
            pending.push_back((other, steps + 1, goal));
        }
        true
    } else {
        false
    }
}

/// Breadth-first along incoming edges (plus semantic producer->consumer bridges),
/// stopping if this response has used its traversal budget. A place is visited only
/// once per changed file.
fn walk_bridged(
    net: &Net<'_>,
    starts: Vec<u32>,
    bridges: Option<&HashMap<u32, Vec<u32>>>,
    budget: &mut usize,
    mut found: impl FnMut(u32, u32, u32),
) -> bool {
    let mut seen: HashSet<_> = starts.iter().copied().collect();
    let mut pending: VecDeque<_> = starts.into_iter().map(|at| (at, 0, at)).collect();
    while let Some((place, steps, goal)) = pending.pop_front() {
        let Some(left) = budget.checked_sub(1) else {
            return false;
        };
        *budget = left;
        if steps > 0 {
            found(place, steps, goal);
        }
        let mut complete = true;
        {
            let budget_ref = &mut *budget;
            let seen_ref = &mut seen;
            let pending_ref = &mut pending;
            net.each(place, false, &mut |other| {
                if complete && !walk_visit(other, steps, goal, budget_ref, seen_ref, pending_ref) {
                    complete = false;
                }
            });
        }
        if !complete {
            return false;
        }
        if let Some(extra) = bridges.and_then(|b| b.get(&place)) {
            for &other in extra {
                if !walk_visit(other, steps, goal, budget, &mut seen, &mut pending) {
                    return false;
                }
            }
        }
    }
    true
}

#[cfg(test)]
fn walk(
    net: &Net<'_>,
    starts: Vec<u32>,
    budget: &mut usize,
    found: impl FnMut(u32, u32, u32),
) -> bool {
    walk_bridged(net, starts, None, budget, found)
}

fn is_candidate_test_file(path: &str, test: bool) -> bool {
    test && !path.split('/').any(|part| {
        matches!(
            part,
            "fixtures" | "__fixtures__" | "__mocks__" | "testdata" | "conftest.py"
        ) || part.ends_with("_fixture.rs")
            || part.ends_with("_fixtures.rs")
    })
}

fn match_coverage_test_file(
    files: &[String],
    candidates: &[bool],
    tn: &str,
    root: &Path,
    canon_root: Option<&Path>,
) -> Option<usize> {
    let norm = normalize_coverage_path(tn, root, canon_root);
    files
        .iter()
        .enumerate()
        .find(|(id, path)| {
            candidates[*id] && (norm.as_deref() == Some(path.as_str()) || names_test(tn, path))
        })
        .map(|(id, _)| id)
}

impl Index {
    pub(crate) fn tests_reaching_changes(
        &self,
        view: &mut Value,
        changed: &[(String, char)],
        scope: &str,
        certain: bool,
    ) {
        if !matches!(view["mode"].as_str(), Some("changes" | "file")) || changed.is_empty() {
            return;
        }
        let ids: HashMap<_, _> = self
            .files
            .iter()
            .enumerate()
            .map(|(id, path)| (path.as_str(), id as u32))
            .collect();
        let mut paths: Vec<_> = changed
            .iter()
            .filter(|(path, _)| {
                scope.is_empty()
                    || path == scope
                    || path
                        .strip_prefix(scope)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            })
            .collect();
        if paths.is_empty() {
            return;
        }
        // A map sometimes opens a common folder automatically. Keep the
        // list sorted and prefer the selected file if work has to be capped.
        let focused = view["focus"]["path"].as_str();
        paths.sort_by_key(|(path, _)| (focused != Some(path.as_str()), path));
        let test_files: Vec<_> = self.files.iter().map(|path| is_test(path)).collect();
        let candidate_files: Vec<_> = self
            .files
            .iter()
            .zip(&test_files)
            .map(|(path, test)| is_candidate_test_file(path, *test))
            .collect();
        let coverage = discover_workspace_coverage(&self.root);
        let canon_root = coverage
            .as_ref()
            .and_then(|_| infra_core::fs::canonicalize(&self.root).ok());
        let match_coverage_test = |tn: &str| -> Option<usize> {
            match_coverage_test_file(
                &self.files,
                &candidate_files,
                tn,
                &self.root,
                canon_root.as_deref(),
            )
        };
        let test_symbols = self
            .test_symbols
            .get_or_init(|| self.test_symbols(&test_files, &|path| text_at(&self.root, path)));
        let inline = |at: u32| {
            let symbol = &self.symbols[at as usize];
            !test_files[symbol.file as usize]
                && test_symbols[at as usize]
                && matches!(symbol.kind, Kind::Function | Kind::Method)
        };
        let has_inline = (0..self.symbols.len() as u32).any(inline);
        let files = Net::new(self, false, certain);
        let symbols = Net::new(self, true, certain);
        let (sym_bridges, file_bridges) = build_semantic_bridges(self, certain);
        let names = Names::new(self, &[], &|_| None);
        let per_file_slice = (VISITS / paths.len().max(1)).clamp(1_024, VISITS);
        let rows: Vec<_> = paths.into_iter().map(|(path, status)| {
            let mut budget = per_file_slice;
            let cov = coverage.as_ref().and_then(|report| report.files.get(path.as_str()));
            let Some(file) = ids.get(path.as_str()).copied().filter(|_| *status != 'D') else {
                let mut row = json!({"path": path, "state": if *status == 'D' { "deleted" } else { "missing" }});
                if *status != 'D' && !is_test(path) && let Some(cov) = cov {
                    row["coverage"] = cov.to_json();
                }
                return row;
            };
            if test_files[file as usize] {
                return json!({"path": path, "state": "test_file"});
            }
            let mut found = Vec::new();
            let files_done = walk_bridged(&files, vec![file], Some(&file_bridges), &mut budget, |at, steps, _| {
                if candidate_files[at as usize] {
                    found.push(Candidate { steps, file: at, symbol: None });
                }
            });
            let symbols_done = !has_inline || (files_done && walk_bridged(
                &symbols,
                self.range(file).filter(|at| {
                    !test_symbols[*at as usize] && self.symbols[*at as usize].kind != Kind::Note
                }).collect(),
                Some(&sym_bridges),
                &mut budget,
                |at, steps, goal| {
                    if inline(at) {
                        found.push(Candidate {
                            steps,
                            file: self.symbols[at as usize].file,
                            symbol: Some((at, goal)),
                        });
                    }
                },
            ));
            found.sort_by(|a, b| a.steps.cmp(&b.steps)
                .then(self.files[a.file as usize].cmp(&self.files[b.file as usize]))
                .then(a.symbol.cmp(&b.symbol)));
            struct ReachEntry {
                steps: u32,
                verified: bool,
                file: String,
                path: String,
                name: String,
                value: Value,
            }
            let mut merged = found
                .iter()
                .map(|test| {
                    let test_file = &self.files[test.file as usize];
                    let (from, to, label) = test.symbol.map_or_else(
                        || (test_file.clone(), path.clone(), test_file.clone()),
                        |(at, goal)| (names.key(at), names.key(goal), self.shown(at)),
                    );
                    ReachEntry {
                        steps: test.steps,
                        verified: false,
                        file: test_file.clone(),
                        path: from.clone(),
                        name: label.clone(),
                        value: json!({
                            "path": from,
                            "to": to,
                            "file": test_file,
                            "name": label,
                            "steps": test.steps,
                        }),
                    }
                })
                .collect::<Vec<_>>();
            if let Some(cov) = cov {
                for tn in &cov.test_hits {
                    let matched_file = match_coverage_test(tn).map(|id| self.files[id].as_str());
                    let existing = merged.iter_mut().find(|entry| {
                        matched_file.is_some_and(|tf| entry.file == tf)
                            || entry.file == tn.as_str()
                            || entry.name == tn.as_str()
                    });
                    if let Some(entry) = existing {
                        entry.verified = true;
                        entry.value["coverage_verified"] = json!(true);
                    } else {
                        let file_label = matched_file.unwrap_or(tn.as_str()).to_owned();
                        merged.push(ReachEntry {
                            steps: 1,
                            verified: true,
                            file: file_label.clone(),
                            path: file_label.clone(),
                            name: tn.clone(),
                            value: json!({
                                "path": file_label,
                                "to": path,
                                "file": file_label,
                                "name": tn,
                                "steps": 1,
                                "source": "coverage",
                                "coverage_verified": true,
                            }),
                        });
                    }
                }
                merged.sort_by(|a, b| {
                    a.steps
                        .cmp(&b.steps)
                        .then(b.verified.cmp(&a.verified))
                        .then(a.file.cmp(&b.file))
                        .then(a.path.cmp(&b.path))
                        .then(a.name.cmp(&b.name))
                });
            }
            let total = merged.len();
            let tests = merged
                .into_iter()
                .take(LISTED)
                .map(|entry| entry.value)
                .collect::<Vec<_>>();
            let mut row = json!({
                "path": path,
                "state": if files_done && symbols_done { "ready" } else { "partial" },
                "tests": tests,
                "total": total,
            });
            if let Some(cov) = cov {
                row["coverage"] = cov.to_json();
            }
            row
        }).collect();
        view["test_reach"] = Value::Array(rows);
    }

    /// Collects reaching test files (dedicated test files, inline `#[test]` / `#[cfg(test)]`
    /// source files, and LCOV/Clover coverage-verified tests) across `target_paths`.
    pub(crate) fn collect_tests_for_paths(
        &self,
        target_paths: &[&str],
        certain: bool,
    ) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        if target_paths.is_empty() {
            return out;
        }
        let ids: HashMap<_, _> = self
            .files
            .iter()
            .enumerate()
            .map(|(id, path)| (path.as_str(), id as u32))
            .collect();
        let test_files: Vec<_> = self.files.iter().map(|path| is_test(path)).collect();
        let candidate_files: Vec<_> = self
            .files
            .iter()
            .zip(&test_files)
            .map(|(path, test)| is_candidate_test_file(path, *test))
            .collect();
        let coverage = discover_workspace_coverage(&self.root);
        let canon_root = coverage
            .as_ref()
            .and_then(|_| infra_core::fs::canonicalize(&self.root).ok());
        let match_coverage_test = |tn: &str| -> Option<usize> {
            match_coverage_test_file(
                &self.files,
                &candidate_files,
                tn,
                &self.root,
                canon_root.as_deref(),
            )
        };
        let test_symbols = self
            .test_symbols
            .get_or_init(|| self.test_symbols(&test_files, &|path| text_at(&self.root, path)));
        let inline = |at: u32| {
            let symbol = &self.symbols[at as usize];
            !test_files[symbol.file as usize]
                && test_symbols[at as usize]
                && matches!(symbol.kind, Kind::Function | Kind::Method)
        };
        let has_inline = (0..self.symbols.len() as u32).any(inline);
        let files = Net::new(self, false, certain);
        let symbols = Net::new(self, true, certain);
        let (sym_bridges, file_bridges) = build_semantic_bridges(self, certain);
        let per_file_slice = (VISITS / target_paths.len().max(1)).clamp(1_024, VISITS);

        for &path in target_paths {
            if let Some(&file) = ids.get(path)
                && !test_files[file as usize]
            {
                let mut budget = per_file_slice;
                let files_done = walk_bridged(
                    &files,
                    vec![file],
                    Some(&file_bridges),
                    &mut budget,
                    |at, _, _| {
                        if candidate_files[at as usize] {
                            out.insert(self.files[at as usize].clone());
                        }
                    },
                );
                if has_inline && files_done {
                    let starts: Vec<u32> = self
                        .range(file)
                        .filter(|at| {
                            !test_symbols[*at as usize]
                                && self.symbols[*at as usize].kind != Kind::Note
                        })
                        .collect();
                    let _ = walk_bridged(
                        &symbols,
                        starts,
                        Some(&sym_bridges),
                        &mut budget,
                        |at, _, _| {
                            if inline(at) {
                                let file_idx = self.symbols[at as usize].file as usize;
                                out.insert(self.files[file_idx].clone());
                            }
                        },
                    );
                }
            }
            if let Some(cov) = coverage.as_ref().and_then(|report| report.files.get(path)) {
                for tn in &cov.test_hits {
                    let file_label = match_coverage_test(tn)
                        .map(|id| self.files[id].clone())
                        .unwrap_or_else(|| tn.clone());
                    if !file_label.is_empty() {
                        out.insert(file_label);
                    }
                }
            }
        }
        out
    }
}

/// How many of the changed files that failing tests reach are listed.
const RANKED: usize = 12;

fn has_word_match(haystack: &str, needle: &str) -> bool {
    let word = |letter: char| letter.is_alphanumeric() || letter == '_';
    haystack.match_indices(needle).any(|(at, _)| {
        !haystack[..at].chars().next_back().is_some_and(word)
            && !haystack[at + needle.len()..]
                .chars()
                .next()
                .is_some_and(word)
    })
}

/// Whether what a person named, or pasted from the output of a test run,
/// names a test file: by its path, or by its name without the extension
/// standing as a word of its own, so that `Cart` does not name `CartTest`.
fn names_test(told: &str, path: &str) -> bool {
    if told.contains(path) {
        return true;
    }
    let stem = name(path);
    let stem = stem.split('.').next().unwrap_or(stem);
    stem.len() >= 4 && has_word_match(told, stem)
}

fn names_tn_token(told: &str, tn: &str) -> bool {
    let told = told.trim();
    if told.is_empty() || tn.is_empty() {
        return false;
    }
    if told == tn || told.contains(tn) {
        return true;
    }
    told.len() >= 4 && has_word_match(tn, told)
}

impl Index {
    /// Ranks the changed files of a picture by how likely each is behind
    /// the tests that fail. `failing` names those tests as a person would:
    /// paths, names of test files, or lines of the output of a test run.
    ///
    /// The graph stands in for what a test runs: a test reaches a file
    /// where a chain of links leads from the test to it. A changed file is
    /// the more suspect the more of the failing tests reach it and the
    /// fewer of the passing ones do: the share of the failing tests that
    /// reach it, times the share of failing among the tests that reach it,
    /// under a root, which is 1 where exactly the failing tests reach it
    /// and 0 where none does. Of files as suspect, the one nearer to the
    /// failing tests comes first.
    ///
    /// When an LCOV or Clover workspace coverage report exists under
    /// `self.root`, observed coverage metrics and LCOV `TN:` per-test hits
    /// enrich each suspect row and break ties / surface dynamically covered
    /// files cleanly without altering static behavior when absent.
    pub(crate) fn suspects(
        &self,
        view: &mut Value,
        changed: &[(String, char)],
        failing: &[String],
        certain: bool,
    ) {
        if view["mode"] != "changes" || changed.is_empty() || failing.is_empty() {
            return;
        }
        let candidates = self
            .files
            .iter()
            .map(|path| is_candidate_test_file(path, is_test(path)))
            .collect::<Vec<_>>();
        let coverage = discover_workspace_coverage(&self.root);
        let canon_root = coverage
            .as_ref()
            .and_then(|_| infra_core::fs::canonicalize(&self.root).ok());
        let match_tn_file = |tn: &str| -> Option<usize> {
            match_coverage_test_file(
                &self.files,
                &candidates,
                tn,
                &self.root,
                canon_root.as_deref(),
            )
        };
        let mut virtual_tns = BTreeSet::new();
        if let Some(cov) = &coverage {
            for tn in &cov.all_tests {
                if match_tn_file(tn).is_none() {
                    virtual_tns.insert(tn.clone());
                }
            }
        }
        // The tests that are named, and what was named that is no test.
        let mut failed = vec![false; self.files.len()];
        let mut failed_virtual_tns = BTreeSet::new();
        let mut unmatched = Vec::new();
        for told in failing {
            let mut any = false;
            for (file, path) in self.files.iter().enumerate() {
                if candidates[file] && names_test(told, path) {
                    failed[file] = true;
                    any = true;
                }
            }
            for tn in &virtual_tns {
                if names_tn_token(told, tn) {
                    failed_virtual_tns.insert(tn.clone());
                    any = true;
                }
            }
            if !any {
                unmatched.push(clip(told.trim(), 120).to_owned());
            }
        }
        let failing_files = (0..self.files.len())
            .filter(|file| failed[*file])
            .collect::<Vec<_>>();
        let total = failing_files.len() + failed_virtual_tns.len();
        let net = Net::new(self, false, certain);
        let (_, file_bridges) = build_semantic_bridges(self, certain);
        let per_file_slice = (VISITS / changed.len().max(1)).clamp(1_024, VISITS);
        let mut complete = true;
        struct SuspectRow<'a> {
            path: &'a str,
            status: char,
            score: f64,
            fail: usize,
            pass: usize,
            nearest: u32,
            far: u64,
            cov: Option<&'a FileCoverage>,
            cov_failing_hit: bool,
            cov_any_hit: bool,
            bfs_fail: usize,
        }
        let mut rows = Vec::new();
        let mut clear = 0usize;
        for (path, status) in changed {
            let Some(file) = self.file_id(path).filter(|_| *status != 'D') else {
                continue;
            };
            if candidates[file as usize] {
                continue;
            }
            // Which tests reach it, and from how near the failing ones do.
            let mut budget = per_file_slice;
            let mut bfs_failing_files = BTreeSet::new();
            let mut bfs_passing_files = BTreeSet::new();
            let (mut nearest, mut far) = (u32::MAX, 0u64);
            complete &= walk_bridged(
                &net,
                vec![file],
                Some(&file_bridges),
                &mut budget,
                |at, steps, _| {
                    if !candidates[at as usize] {
                        return;
                    }
                    if failed[at as usize] {
                        if bfs_failing_files.insert(at as usize) {
                            nearest = nearest.min(steps);
                            far += u64::from(steps);
                        }
                    } else {
                        bfs_passing_files.insert(at as usize);
                    }
                },
            );
            let bfs_fail = bfs_failing_files.len();
            let cov = coverage
                .as_ref()
                .and_then(|report| report.files.get(path.as_str()));
            let mut cov_failing_hit = false;
            let mut failing_virtual_hits = BTreeSet::new();
            let mut passing_virtual_hits = BTreeSet::new();
            if let Some(cov) = cov {
                for tn in &cov.test_hits {
                    if let Some(tf) = match_tn_file(tn) {
                        if failed[tf] {
                            cov_failing_hit = true;
                            if bfs_failing_files.insert(tf) {
                                nearest = nearest.min(1);
                                far += 1;
                            }
                        } else if !bfs_failing_files.contains(&tf) {
                            bfs_passing_files.insert(tf);
                        }
                    } else if failed_virtual_tns.contains(tn) {
                        cov_failing_hit = true;
                        if failing_virtual_hits.insert(tn.as_str()) {
                            nearest = nearest.min(1);
                            far += 1;
                        }
                    } else {
                        passing_virtual_hits.insert(tn.as_str());
                    }
                }
            }
            let fail = bfs_failing_files.len() + failing_virtual_hits.len();
            let pass = bfs_passing_files.len() + passing_virtual_hits.len();
            if fail == 0 {
                clear += 1;
                continue;
            }
            let cov_any_hit = cov.is_some_and(|c| c.covered_lines > 0);
            let score = fail as f64 / ((total * (fail + pass)) as f64).sqrt();
            rows.push(SuspectRow {
                path: path.as_str(),
                status: *status,
                score,
                fail,
                pass,
                nearest,
                far,
                cov,
                cov_failing_hit,
                cov_any_hit,
                bfs_fail,
            });
        }
        rows.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then(b.cov_failing_hit.cmp(&a.cov_failing_hit))
                .then(b.cov_any_hit.cmp(&a.cov_any_hit))
                .then((a.far * b.fail as u64).cmp(&(b.far * a.fail as u64)))
                .then(a.path.cmp(b.path))
        });
        let ranked = rows.len();
        let listed = rows
            .iter()
            .take(RANKED)
            .enumerate()
            .map(|(at, row)| {
                let mut item = json!({
                    "path": row.path,
                    "status": row.status.to_string(),
                    "rank": at + 1,
                    "score": (row.score * 100.0 + 1e-6).round() / 100.0,
                    "failing": row.fail,
                    "passing": row.pass,
                    "nearest": row.nearest,
                });
                if let Some(cov) = row.cov {
                    item["coverage"] = cov.to_json();
                }
                if row.cov_failing_hit {
                    item["coverage_verified"] = json!(true);
                    if row.bfs_fail == 0 {
                        item["source"] = json!("coverage");
                    }
                }
                item
            })
            .collect::<Vec<_>>();
        // The boxes of the picture carry their rank, and a folder the best
        // rank among the ranked files it holds: a change too wide to draw
        // file by file is drawn by folder.
        if let Some(nodes) = view["nodes"].as_array_mut() {
            for node in nodes {
                let Some(path) = node["path"].as_str().map(str::to_owned) else {
                    continue;
                };
                if let Some(row) = listed.iter().find(|row| row["path"] == path.as_str()) {
                    node["suspect"] = json!({"rank": row["rank"], "score": row["score"]});
                } else if node["folder"] == true {
                    let held = listed
                        .iter()
                        .filter(|row| {
                            row["path"].as_str().is_some_and(|file| {
                                file.strip_prefix(path.as_str())
                                    .is_some_and(|rest| rest.starts_with('/'))
                            })
                        })
                        .collect::<Vec<_>>();
                    if let Some(first) = held.first() {
                        node["suspect"] = json!({
                            "rank": first["rank"], "score": first["score"], "files": held.len(),
                        });
                    }
                }
            }
        }
        let mut failing_list = failing_files
            .iter()
            .map(|file| self.files[*file].as_str())
            .collect::<Vec<_>>();
        for tn in &failed_virtual_tns {
            failing_list.push(tn.as_str());
        }
        view["suspects"] = json!({
            "failing": failing_list,
            "unmatched": unmatched,
            "tests": candidates.iter().filter(|test| **test).count() + virtual_tns.len(),
            "rows": listed,
            "more": ranked.saturating_sub(RANKED),
            "clear": clear,
            "complete": complete,
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_only_incoming_paths_and_distinguishes_unavailable_files() {
        let node = |id: &str, file: &str| json!({"id":id, "label":id, "source_file":file});
        let edge = |from: &str, to: &str, confidence: &str| json!({"source":from, "target":to, "relation":"calls", "confidence":confidence});
        let graph = json!({
            "nodes": [
                node("changed", "src/changed.php"),
                node("alone", "src/alone.php"),
                node("bridge", "src/bridge.php"),
                node("test", "tests/CartTest.php"),
                node("reverse", "tests/ReverseTest.php"),
                node("direct", "tests/DirectTest.php"),
                node("fixture", "tests/fixtures/data.php"),
            ],
            "links": [
                edge("test", "bridge", "EXTRACTED"),
                edge("bridge", "changed", "EXTRACTED"),
                edge("changed", "bridge", "EXTRACTED"),
                edge("changed", "reverse", "EXTRACTED"),
                edge("direct", "changed", "INFERRED"),
                edge("fixture", "changed", "EXTRACTED"),
            ]
        });
        let index = Index::read(graph.to_string().as_bytes(), Path::new("/work/shop")).unwrap();
        let changed = [
            ("src/alone.php", 'M'),
            ("src/changed.php", 'M'),
            ("src/deleted.php", 'D'),
            ("src/missing.php", 'A'),
            ("tests/CartTest.php", 'M'),
        ]
        .map(|(path, status)| (path.to_owned(), status));
        let run = |certain| {
            let mut view = json!({"mode":"changes", "scope":"src"});
            index.tests_reaching_changes(&mut view, &changed, "", certain);
            view["test_reach"].clone()
        };
        let rows = run(false);
        assert_eq!(rows[0]["total"], 0);
        assert_eq!(rows[1]["total"], 2);
        assert_eq!(rows[1]["tests"][0]["path"], "tests/DirectTest.php");
        assert_eq!(rows[1]["tests"][0]["steps"], 1);
        assert_eq!(rows[1]["tests"][1]["path"], "tests/CartTest.php");
        assert_eq!(rows[1]["tests"][1]["steps"], 2);
        assert_eq!(rows[2]["state"], "deleted");
        assert_eq!(rows[3]["state"], "missing");
        assert_eq!(rows[4]["state"], "test_file");
        assert_eq!(run(true)[1]["total"], 1);

        let net = Net::new(&index, false, false);
        let file = index.opened("src/changed.php").unwrap();
        assert!(!walk(&net, vec![file], &mut 1, |_, _, _| {}));
    }

    #[test]
    fn opens_an_inline_test_path_inside_the_changed_file() {
        let graph = json!({
            "nodes": [
                {"id":"work", "label":"work()", "source_file":"src/lib.rs", "source_location":"L1"},
                {"id":"test", "label":"exercise()", "source_file":"src/lib.rs", "source_location":"L4"}
            ],
            "links": [{"source":"test", "target":"work", "relation":"calls", "confidence":"EXTRACTED"}]
        });
        let mut index = Index::read(graph.to_string().as_bytes(), Path::new("/work/shop")).unwrap();
        index.revision_tests = Some(BTreeMap::from([("src/lib.rs".into(), vec![(3, 4)])]));
        let mut view = json!({"mode":"file","scope":"src/lib.rs"});
        index.tests_reaching_changes(&mut view, &[("src/lib.rs".into(), 'M')], "src/lib.rs", true);
        let test = &view["test_reach"][0]["tests"][0];
        assert_eq!(test["steps"], 1, "{view}");
        let route = index
            .path(
                "shop",
                test["path"].as_str().unwrap(),
                test["to"].as_str().unwrap(),
                &[],
                &|_| None,
                None,
                true,
            )
            .unwrap();
        assert_eq!(route["path"]["found"], true, "{route}");
        assert_eq!(route["path"]["reversed"], false);
    }

    #[test]
    fn the_changed_files_that_failing_tests_reach_come_first() {
        // cart uses price; CartTest uses cart, PriceTest uses price,
        // MailTest uses mail. All three of cart, price and mail changed,
        // and a fourth file was deleted.
        let node = |id: &str, file: &str| json!({"id": id, "label": id, "source_file": file});
        let edge = |from: &str, to: &str| json!({"source": from, "target": to, "relation": "calls", "confidence": "EXTRACTED"});
        let graph = json!({
            "nodes": [
                node("price", "src/price.php"), node("cart", "src/cart.php"),
                node("mail", "src/mail.php"), node("carttest", "tests/CartTest.php"),
                node("pricetest", "tests/PriceTest.php"), node("mailtest", "tests/MailTest.php"),
                node("helper", "tests/fixtures/CartFixture.php"),
            ],
            "links": [
                edge("cart", "price"), edge("carttest", "cart"), edge("pricetest", "price"),
                edge("mailtest", "mail"), edge("helper", "cart"),
            ]
        });
        let index = Index::read(graph.to_string().as_bytes(), Path::new("/work/shop")).unwrap();
        let changed = [
            ("src/price.php", 'M'),
            ("src/mail.php", 'M'),
            ("src/cart.php", 'M'),
            ("src/gone.php", 'D'),
        ]
        .map(|(path, status)| (path.to_owned(), status));
        let rank = |failing: &[&str]| {
            let mut view = json!({
                "mode": "changes",
                "nodes": [{"path": "src/price.php"}, {"path": "src/cart.php"}, {"path": "src/mail.php"}]
            });
            let failing = failing
                .iter()
                .map(|told| (*told).to_owned())
                .collect::<Vec<_>>();
            index.suspects(&mut view, &changed, &failing, false);
            view
        };
        // CartTest fails, named as a test runner names it. It reaches cart
        // in one link and price in two; PriceTest, which passes, reaches
        // price as well, so price comes second. MailTest reaches nothing
        // that fails.
        let view = rank(&["1) Tests\\Unit\\CartTest::testTotal"]);
        let suspects = &view["suspects"];
        assert_eq!(suspects["failing"], json!(["tests/CartTest.php"]));
        assert_eq!(suspects["tests"], 3);
        assert_eq!(
            suspects["rows"],
            json!([
                {"path": "src/cart.php", "status": "M", "rank": 1, "score": 1.0, "failing": 1, "passing": 0, "nearest": 1},
                {"path": "src/price.php", "status": "M", "rank": 2, "score": 0.71, "failing": 1, "passing": 1, "nearest": 2}
            ])
        );
        assert_eq!(suspects["clear"], 1);
        assert_eq!(suspects["more"], 0);
        assert_eq!(suspects["unmatched"], json!([]));
        assert_eq!(suspects["complete"], true);
        // The boxes of the picture carry their rank.
        assert_eq!(
            view["nodes"][1]["suspect"],
            json!({"rank": 1, "score": 1.0})
        );
        assert_eq!(view["nodes"][0]["suspect"]["rank"], 2);
        assert!(view["nodes"][2]["suspect"].is_null());
        // A change drawn by folder: the folder carries the best rank of the
        // ranked files it holds, and how many they are.
        let mut folders = json!({
            "mode": "changes",
            "nodes": [{"path": "src", "folder": true}, {"path": "tests", "folder": true}]
        });
        index.suspects(&mut folders, &changed, &["CartTest".to_owned()], false);
        assert_eq!(
            folders["nodes"][0]["suspect"],
            json!({"rank": 1, "score": 1.0, "files": 2})
        );
        assert!(folders["nodes"][1]["suspect"].is_null());
        // A fixture is no test, a path names its test, and what names no
        // test file is said to.
        let view = rank(&["tests/MailTest.php", "CartFixture", "Cart"]);
        assert_eq!(view["suspects"]["failing"], json!(["tests/MailTest.php"]));
        assert_eq!(
            view["suspects"]["unmatched"],
            json!(["CartFixture", "Cart"])
        );
        assert_eq!(view["suspects"]["rows"][0]["path"], "src/mail.php");
        assert_eq!(view["suspects"]["rows"][0]["score"], 1.0);
        assert_eq!(view["suspects"]["clear"], 2);
        // Both fail: cart is reached by one of the two, price by one of
        // the two and by a passing test, mail by the other alone.
        let view = rank(&["CartTest", "MailTest"]);
        let rows = view["suspects"]["rows"].as_array().unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| (
                    row["path"].as_str().unwrap(),
                    row["score"].as_f64().unwrap()
                ))
                .collect::<Vec<_>>(),
            [
                ("src/cart.php", 0.71),
                ("src/mail.php", 0.71),
                ("src/price.php", 0.5)
            ]
        );
        // Nothing failing, or no picture of a change: nothing is told.
        assert!(rank(&[])["suspects"].is_null());
        let mut folder = json!({"mode": "folder", "nodes": []});
        index.suspects(&mut folder, &changed, &["CartTest".to_owned()], false);
        assert!(folder["suspects"].is_null());
    }

    #[test]
    fn a_test_is_named_by_its_path_or_by_its_name_as_a_word_of_its_own() {
        let path = "tests/Unit/CartTest.php";
        for told in [
            "tests/Unit/CartTest.php",
            "FAILED tests/Unit/CartTest.php::test_total - AssertionError",
            "1) Tests\\Unit\\CartTest::testTotal",
            "CartTest",
            "  CartTest.testTotal",
        ] {
            assert!(names_test(told, path), "{told}");
        }
        for told in ["Cart", "CartTestHelper", "MyCartTest", "", "tests/Unit"] {
            assert!(!names_test(told, path), "{told}");
        }
        // A name too short to tell is matched by its path only.
        assert!(!names_test("ab", "tests/ab.py"));
        assert!(names_test("see tests/ab.py", "tests/ab.py"));
        // The name of a Rust or Python test file is its stem.
        assert!(names_test(
            "test_cart::total ... FAILED",
            "tests/test_cart.py"
        ));
        assert!(names_test("cart_test", "src/cart_test.go"));
    }

    #[test]
    fn parses_lcov_with_multiple_tn_tests_and_normalizes_paths() {
        let root = Path::new("/work/shop");
        let lcov = "\
TN:CartTest::testTotal
SF:/work/shop/src/cart.php
DA:10,2
DA:11,0
LF:2
LH:1
end_of_record
TN:CheckoutTest::testPay
SF:./src\\cart.php
DA:10,0
DA:11,3
LF:2
LH:1
end_of_record
TN:ZeroHitTest::testUnused
SF:src/cart.php
DA:10,0
DA:11,0
LF:2
LH:0
end_of_record
TN:SummaryOnlyTest::testTax
SF:src/tax.php
LF:10
LH:7
end_of_record
";
        let mut report = CoverageReport::default();
        parse_lcov(lcov, root, None, &mut report);

        let cart = report.files.get("src/cart.php").expect("src/cart.php");
        assert_eq!(cart.format, CoverageFormat::Lcov);
        assert_eq!(cart.covered_lines, 2);
        assert_eq!(cart.total_lines, 2);
        assert_eq!(cart.rate(), 1.0);
        assert_eq!(
            cart.tests,
            vec![
                "CartTest::testTotal".to_owned(),
                "CheckoutTest::testPay".to_owned()
            ]
        );

        let tax = report.files.get("src/tax.php").expect("src/tax.php");
        assert_eq!(tax.covered_lines, 7);
        assert_eq!(tax.total_lines, 10);
        assert_eq!(tax.rate(), 0.7);
        assert_eq!(tax.tests, vec!["SummaryOnlyTest::testTax".to_owned()]);
        assert!(report.all_tests.contains("ZeroHitTest::testUnused"));
    }

    #[test]
    fn parses_clover_xml_variants_and_prefers_file_metrics_over_nested_class_metrics() {
        let root = Path::new("/work/shop");
        let clover = r#"<?xml version="1.0" encoding="UTF-8"?>
<coverage generated="1700000000">
  <!-- PHPUnit clover with nested class metrics -->
  <project timestamp="1700000000">
    <package name="App">
      <file name="/work/shop/src/Order.php">
        <class name="App\Order">
          <metrics statements="2" coveredstatements="1"/>
        </class>
        <line num="10" type="method" count="1"/>
        <line num="12" type="stmt" count="2"/>
        <line num="13" type="stmt" count="2"/>
        <line num="14" type="stmt" count="0"/>
        <metrics statements="8" coveredstatements="6"/>
      </file>
      <file name="Client.ts" path="/work/shop/web/Client.ts">
        <line num="1" type="stmt" count="3"/>
        <line num="2" type="stmt" count="0"/>
        <line num="3" type="stmt" count="1"/>
      </file>
      <file name="Empty.php"/>
    </package>
  </project>
</coverage>"#;
        let mut report = CoverageReport::default();
        parse_clover(clover, root, None, &mut report);

        let order = report.files.get("src/Order.php").expect("src/Order.php");
        assert_eq!(order.format, CoverageFormat::Clover);
        assert_eq!(order.covered_lines, 6);
        assert_eq!(order.total_lines, 8);
        assert_eq!(order.rate(), 0.75);
        assert!(order.tests.is_empty());

        let client = report.files.get("web/Client.ts").expect("web/Client.ts");
        assert_eq!(client.format, CoverageFormat::Clover);
        assert_eq!(client.covered_lines, 2);
        assert_eq!(client.total_lines, 3);
        assert!(!report.files.contains_key("Empty.php"));
    }

    #[test]
    fn rejects_path_traversal_outside_root_and_oversized_or_symlinked_coverage_files() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("workspace");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(ws.join("src")).unwrap();
        fs::create_dir_all(ws.join("coverage")).unwrap();
        fs::create_dir_all(&outside).unwrap();

        let secret = outside.join("secret.info");
        fs::write(&secret, "TN:Evil\nSF:src/a.php\nDA:1,1\nend_of_record\n").unwrap();

        // Direct path normalization rejections.
        let canon = infra_core::fs::canonicalize(&ws).unwrap();
        assert_eq!(
            normalize_coverage_path("../../etc/passwd", &ws, Some(&canon)),
            None
        );
        assert_eq!(
            normalize_coverage_path("/etc/passwd", &ws, Some(&canon)),
            None
        );
        assert_eq!(
            normalize_coverage_path("src/../../outside.php", &ws, Some(&canon)),
            None
        );
        assert_eq!(
            normalize_coverage_path(
                &outside.join("secret.php").to_string_lossy(),
                &ws,
                Some(&canon)
            ),
            None
        );
        assert_eq!(
            normalize_coverage_path(&ws.join("src/a.php").to_string_lossy(), &ws, Some(&canon)),
            Some("src/a.php".to_owned())
        );
        assert_eq!(
            normalize_coverage_path(
                &canon.join("src/a.php").to_string_lossy(),
                &ws,
                Some(&canon)
            ),
            Some("src/a.php".to_owned())
        );

        // Symlink outside root must be rejected.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&secret, ws.join("lcov.info")).unwrap();
            std::os::unix::fs::symlink(&outside, ws.join("build")).unwrap();
            assert!(discover_workspace_coverage(&ws).is_none());
            fs::remove_file(ws.join("lcov.info")).unwrap();
        }

        // Oversized (> 8 MiB) file must be skipped safely.
        let big = vec![b'#'; (MAX_COVERAGE_FILE_BYTES + 1024) as usize];
        fs::write(ws.join("coverage/lcov.info"), big).unwrap();
        assert!(discover_workspace_coverage(&ws).is_none());
        fs::remove_file(ws.join("coverage/lcov.info")).unwrap();

        // Internal traversal entries inside an otherwise valid report are dropped.
        let report_text = format!(
            "TN:Bad\nSF:../../etc/passwd\nDA:1,1\nend_of_record\nTN:BadAbs\nSF:/other/repo/src/a.php\nDA:1,1\nend_of_record\nTN:Good\nSF:{}/src/a.php\nDA:1,1\nend_of_record\n",
            ws.display()
        );
        fs::write(ws.join("lcov.info"), report_text).unwrap();
        let discovered = discover_workspace_coverage(&ws).expect("good file kept");
        assert_eq!(discovered.files.len(), 1);
        assert!(discovered.files.contains_key("src/a.php"));
    }

    #[test]
    fn tests_reaching_changes_and_suspects_integrate_lcov_and_clover_coverage() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("coverage")).unwrap();
        fs::create_dir_all(root.join("build/logs")).unwrap();

        // Static graph:
        // - tests/CartTest.php calls src/cart.php
        // - tests/PriceTest.php calls src/price.php
        // - tests/DynamicTest.php has NO static edge to src/dynamic.php
        // - tests/CartTest.php also statically reaches src/cold.php (1 hop) and src/hot.php (2 hops)
        let node = |id: &str, file: &str| json!({"id": id, "label": id, "source_file": file});
        let edge = |from: &str, to: &str| json!({"source": from, "target": to, "relation": "calls", "confidence": "EXTRACTED"});
        let graph = json!({
            "nodes": [
                node("cart", "src/cart.php"),
                node("price", "src/price.php"),
                node("dynamic", "src/dynamic.php"),
                node("cold", "src/cold.php"),
                node("hot", "src/hot.php"),
                node("hot_bridge", "src/hot_bridge.php"),
                node("carttest", "tests/CartTest.php"),
                node("pricetest", "tests/PriceTest.php"),
                node("dynamictest", "tests/DynamicTest.php"),
            ],
            "links": [
                edge("carttest", "cart"),
                edge("pricetest", "price"),
                edge("carttest", "cold"),
                edge("carttest", "hot_bridge"),
                edge("hot_bridge", "hot"),
            ]
        });
        let index = Index::read(graph.to_string().as_bytes(), root).unwrap();

        // LCOV:
        // - CartTest::testTotal hits src/cart.php (confirming BFS) AND src/hot.php (confirming BFS) AND src/dynamic.php (missed by BFS!)
        // - CartTest::testTotal did NOT cover src/cold.php (0 hits)
        // - ExternalIntegrationTest hits src/dynamic.php (virtual TN not in graph)
        let lcov = format!(
            "\
TN:CartTest::testTotal
SF:{}/src/cart.php
DA:1,1
DA:2,1
LF:2
LH:2
end_of_record
TN:CartTest::testTotal
SF:src/dynamic.php
DA:5,2
DA:6,0
LF:2
LH:1
end_of_record
TN:ExternalIntegrationTest
SF:src/dynamic.php
DA:6,1
LF:2
LH:1
end_of_record
TN:CartTest::testTotal
SF:src/hot.php
DA:3,4
LF:1
LH:1
end_of_record
TN:CartTest::testTotal
SF:src/cold.php
DA:1,0
DA:2,0
LF:2
LH:0
end_of_record
",
            root.display()
        );
        fs::write(root.join("coverage/lcov.info"), lcov).unwrap();
        // Clover covers src/price.php (LCOV takes precedence on files it already parsed).
        let clover = r#"<?xml version="1.0" encoding="UTF-8"?>
<coverage>
  <project>
    <file name="src/price.php">
      <metrics statements="4" coveredstatements="3"/>
    </file>
  </project>
</coverage>"#;
        fs::write(root.join("build/logs/clover.xml"), clover).unwrap();

        let changed = [
            ("src/cart.php", 'M'),
            ("src/dynamic.php", 'M'),
            ("src/price.php", 'M'),
            ("src/cold.php", 'M'),
            ("src/hot.php", 'M'),
        ]
        .map(|(p, s)| (p.to_owned(), s));

        let mut view = json!({"mode": "changes", "scope": "src"});
        index.tests_reaching_changes(&mut view, &changed, "", false);
        let reach = view["test_reach"].as_array().unwrap();

        let cart_row = reach.iter().find(|r| r["path"] == "src/cart.php").unwrap();
        assert_eq!(cart_row["coverage"]["format"], "lcov");
        assert_eq!(cart_row["coverage"]["covered_lines"], 2);
        assert_eq!(cart_row["coverage"]["total_lines"], 2);
        assert_eq!(cart_row["coverage"]["rate"], 1.0);
        assert_eq!(
            cart_row["coverage"]["tests"],
            json!(["CartTest::testTotal"])
        );
        assert_eq!(cart_row["tests"][0]["path"], "tests/CartTest.php");
        assert_eq!(cart_row["tests"][0]["coverage_verified"], true);

        let dyn_row = reach
            .iter()
            .find(|r| r["path"] == "src/dynamic.php")
            .unwrap();
        assert_eq!(dyn_row["coverage"]["covered_lines"], 2);
        assert_eq!(dyn_row["coverage"]["total_lines"], 2);
        assert_eq!(dyn_row["total"], 2);
        let dyn_tests = dyn_row["tests"].as_array().unwrap();
        assert!(dyn_tests.iter().any(|t| {
            t["path"] == "tests/CartTest.php"
                && t["source"] == "coverage"
                && t["coverage_verified"] == true
        }));
        assert!(dyn_tests.iter().any(|t| {
            t["path"] == "ExternalIntegrationTest"
                && t["source"] == "coverage"
                && t["coverage_verified"] == true
        }));

        let price_row = reach.iter().find(|r| r["path"] == "src/price.php").unwrap();
        assert_eq!(price_row["coverage"]["format"], "clover");
        assert_eq!(price_row["coverage"]["covered_lines"], 3);
        assert_eq!(price_row["coverage"]["total_lines"], 4);
        assert_eq!(price_row["coverage"]["rate"], 0.75);

        // Suspects with CartTest failing:
        // - src/cart.php and src/hot.php have coverage_verified + passing=0 + score=1.0
        // - src/dynamic.php is surfaced via LCOV TN:CartTest::testTotal (passing=1 from ExternalIntegrationTest -> score 0.71)
        // - src/cold.php has 0 covered lines, so src/cart.php & src/hot.php rank ahead of src/cold.php despite cold.php being 1 static hop!
        let mut s_view = json!({
            "mode": "changes",
            "nodes": [
                {"path": "src/cart.php"},
                {"path": "src/dynamic.php"},
                {"path": "src/cold.php"},
                {"path": "src/hot.php"},
                {"path": "src/price.php"},
            ]
        });
        index.suspects(&mut s_view, &changed, &["CartTest".to_owned()], false);
        let s_rows = s_view["suspects"]["rows"].as_array().unwrap();
        let rank_of = |p: &str| {
            s_rows
                .iter()
                .find(|r| r["path"] == p)
                .unwrap_or_else(|| panic!("missing suspect {p}: {s_view}"))
        };
        assert_eq!(rank_of("src/cart.php")["coverage_verified"], true);
        assert_eq!(rank_of("src/cart.php")["score"], 1.0);
        assert_eq!(rank_of("src/cart.php")["failing"], 1);
        assert_eq!(rank_of("src/hot.php")["coverage_verified"], true);
        assert!(
            rank_of("src/hot.php")["rank"].as_u64().unwrap()
                < rank_of("src/cold.php")["rank"].as_u64().unwrap()
        );
        assert_eq!(rank_of("src/dynamic.php")["source"], "coverage");
        assert_eq!(rank_of("src/dynamic.php")["coverage_verified"], true);
        assert_eq!(rank_of("src/dynamic.php")["failing"], 1);
        assert_eq!(rank_of("src/dynamic.php")["passing"], 1);
        assert!(rank_of("src/dynamic.php")["coverage"].is_object());
    }

    #[test]
    fn discovers_monorepo_coverage_and_invalidates_cache_on_update() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("mono");
        std::fs::create_dir_all(ws.join("packages/billing/coverage")).unwrap();
        std::fs::create_dir_all(ws.join("packages/billing/src")).unwrap();
        std::fs::write(
            ws.join("packages/billing/src/tax.ts"),
            "export const t = 1;\n",
        )
        .unwrap();
        std::fs::write(
            ws.join("packages/billing/coverage/lcov.info"),
            "SF:packages/billing/src/tax.ts\nDA:1,1\nLF:1\nLH:1\nend_of_record\n",
        )
        .unwrap();

        let first = discover_workspace_coverage(&ws).expect("monorepo coverage");
        assert_eq!(first.files["packages/billing/src/tax.ts"].covered_lines, 1);

        // Second call hits cache and preserves report; editing content invalidates via stamp.
        let cached = discover_workspace_coverage(&ws).expect("cached coverage");
        assert_eq!(cached.files["packages/billing/src/tax.ts"].covered_lines, 1);

        std::fs::write(
            ws.join("packages/billing/coverage/lcov.info"),
            "SF:packages/billing/src/tax.ts\nDA:1,0\nDA:2,0\nLF:2\nLH:0\nend_of_record\n",
        )
        .unwrap();
        let updated = discover_workspace_coverage(&ws).expect("updated coverage");
        assert_eq!(updated.files["packages/billing/src/tax.ts"].total_lines, 2);
        assert_eq!(
            updated.files["packages/billing/src/tax.ts"].covered_lines,
            0
        );
    }

    #[test]
    fn tests_reaching_changes_crosses_semantic_bridges_and_direct_di_binds() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("shop");
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::create_dir_all(ws.join("tests")).unwrap();
        for rel in [
            "src/CheckoutService.php",
            "src/OrderService.php",
            "src/InvoiceListener.php",
            "src/OrderController.php",
            "src/GatewayInterface.php",
            "src/StripeGateway.php",
            "tests/InvoiceListenerTest.php",
            "tests/OrderApiTest.php",
            "tests/StripeGatewayTest.php",
            "tests/OrderControllerTest.php",
        ] {
            std::fs::write(ws.join(rel), "<?php\n").unwrap();
        }
        // `sem:event:OrderCreated` is anchored in `src/CheckoutService.php`, NOT `src/OrderService.php`.
        // Semantic bridge must still carry changes to `src/OrderService.php` through the shared
        // event hub to `src/InvoiceListener.php` -> `tests/InvoiceListenerTest.php`.
        let graph = json!({
            "nodes": [
                {"id": "checkout_pub", "label": "CheckoutService.submit()", "source_file": "src/CheckoutService.php", "node_kind": "method"},
                {"id": "order_pub", "label": "OrderService.create()", "source_file": "src/OrderService.php", "node_kind": "method"},
                {"id": "evt_created", "label": "event:OrderCreated", "source_file": "src/CheckoutService.php", "node_kind": "semantic"},
                {"id": "listener", "label": "InvoiceListener.handle()", "source_file": "src/InvoiceListener.php", "node_kind": "method"},
                {"id": "listener_test", "label": "InvoiceListenerTest.testMail()", "source_file": "tests/InvoiceListenerTest.php", "node_kind": "method"},
                {"id": "ctrl_route", "label": "OrderController.store()", "source_file": "src/OrderController.php", "node_kind": "method"},
                {"id": "route_orders", "label": "route:POST /orders", "source_file": "src/OrderController.php", "node_kind": "semantic"},
                {"id": "api_test", "label": "OrderApiTest.testCreate()", "source_file": "tests/OrderApiTest.php", "node_kind": "method"},
                {"id": "iface", "label": "GatewayInterface", "source_file": "src/GatewayInterface.php", "node_kind": "interface"},
                {"id": "impl", "label": "StripeGateway", "source_file": "src/StripeGateway.php", "node_kind": "class"},
                {"id": "impl_test", "label": "StripeGatewayTest.testCharge()", "source_file": "tests/StripeGatewayTest.php", "node_kind": "method"},
                {"id": "ctrl_type", "label": "OrderController", "source_file": "src/OrderController.php", "node_kind": "class"},
                {"id": "ctrl_test", "label": "OrderControllerTest.testFlow()", "source_file": "tests/OrderControllerTest.php", "node_kind": "method"}
            ],
            "links": [
                {"source": "checkout_pub", "target": "evt_created", "relation": "publishes_event", "confidence": "EXTRACTED"},
                {"source": "order_pub", "target": "evt_created", "relation": "publishes_event", "confidence": "EXTRACTED"},
                {"source": "listener", "target": "evt_created", "relation": "handles_event", "confidence": "EXTRACTED"},
                {"source": "listener_test", "target": "listener", "relation": "calls", "confidence": "EXTRACTED"},
                {"source": "ctrl_route", "target": "route_orders", "relation": "exposes_route", "confidence": "EXTRACTED"},
                {"source": "api_test", "target": "route_orders", "relation": "calls_route", "confidence": "EXTRACTED"},
                {"source": "iface", "target": "impl", "relation": "di_binds", "confidence": "EXTRACTED"},
                {"source": "impl_test", "target": "impl", "relation": "calls", "confidence": "EXTRACTED"},
                {"source": "ctrl_type", "target": "iface", "relation": "references", "confidence": "EXTRACTED"},
                {"source": "ctrl_test", "target": "ctrl_type", "relation": "calls", "confidence": "EXTRACTED"}
            ]
        });
        let idx = Index::read(graph.to_string().as_bytes(), &ws).unwrap();
        let mut view = json!({"mode": "changes"});
        idx.tests_reaching_changes(
            &mut view,
            &[
                ("src/OrderService.php".to_owned(), 'M'),
                ("src/OrderController.php".to_owned(), 'M'),
                ("src/GatewayInterface.php".to_owned(), 'M'),
                ("src/StripeGateway.php".to_owned(), 'M'),
            ],
            "",
            true,
        );
        let rows = view["test_reach"].as_array().unwrap();
        let tests_for = |rel: &str| -> Vec<String> {
            rows.iter()
                .find(|r| r["path"] == rel)
                .and_then(|r| r["tests"].as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|t| t["file"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };

        let order_tests = tests_for("src/OrderService.php");
        assert!(
            order_tests.contains(&"tests/InvoiceListenerTest.php".to_owned()),
            "event publisher should reach listener test across non-local anchor: {order_tests:?}"
        );

        let ctrl_tests = tests_for("src/OrderController.php");
        assert!(
            ctrl_tests.contains(&"tests/OrderApiTest.php".to_owned()),
            "route producer should reach route caller test: {ctrl_tests:?}"
        );

        let iface_tests = tests_for("src/GatewayInterface.php");
        assert!(
            iface_tests.contains(&"tests/StripeGatewayTest.php".to_owned())
                && iface_tests.contains(&"tests/OrderControllerTest.php".to_owned()),
            "interface change should reach both caller tests and bound impl tests: {iface_tests:?}"
        );

        let impl_tests = tests_for("src/StripeGateway.php");
        assert!(
            impl_tests.contains(&"tests/OrderControllerTest.php".to_owned()),
            "impl change should reach interface caller tests via di_binds: {impl_tests:?}"
        );
    }
}
