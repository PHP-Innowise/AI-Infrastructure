use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
};
pub const NATIVE_VERSION: &str = "2.0.0";
pub const LEGACY_VERSION: &str = "1.0.0";
pub const PRIMARY: &[&str] = &[
    "ACCELERATOR-TEST-EVIDENCE-REPORT.md",
    "SHA256SUMS.txt",
    "manifests/historical-runs.json",
    "manifests/historical-testcases.json",
    "manifests/historical-othersheets.json",
    "manifests/defects.json",
    "manifests/cases_map.json",
    "manifests/bwb_cases.json",
    "manifests/workbook-snapshot-20260803-2002.xlsx",
];
pub const FINAL: &[&str] = &[
    "accepted-historical-pass",
    "verified-after-remediation",
    "closed-supported-defect",
    "closed-test-definition-defect",
    "closed-environment-or-setup-artifact",
    "closed-rejected-finding",
    "verified-unmodified-pass",
    "accepted-limitation",
    "accepted-native-host-limitation",
    "external-client-finding-verified",
];
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
fn lower_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        encoded.push(DIGITS[usize::from(byte >> 4)] as char);
        encoded.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    encoded
}
pub fn sha(raw: &[u8]) -> String {
    lower_hex(Sha256::digest(raw).as_ref())
}
pub fn hash_file(path: &Path) -> Result<String> {
    let mut f = fs::File::open(path)?;
    let mut d = Sha256::new();
    let mut b = [0u8; 65536];
    loop {
        let n = f.read(&mut b)?;
        if n == 0 {
            break;
        }
        d.update(&b[..n]);
    }
    Ok(lower_hex(d.finalize().as_ref()))
}
pub fn read_json(path: &Path) -> Result<Value> {
    serde_json::from_slice(&fs::read(path)?)
        .with_context(|| format!("Invalid JSON: {}", path.display()))
}
pub fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
pub fn write_json(path: &Path, v: &Value) -> Result<()> {
    let mut b = serde_json::to_vec_pretty(v)?;
    b.push(b'\n');
    atomic(path, &b)
}
pub fn root() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("INFRA_REPOSITORY_ROOT") {
        return fs::canonicalize(p).context("repository root");
    }
    let cwd = std::env::current_dir()?;
    for p in cwd.ancestors() {
        if p.join("schemas/qa").is_dir() && p.join("qa/tc-ai/cases.json").is_file() {
            return Ok(p.to_owned());
        }
    }
    bail!("run inside the repository or set INFRA_REPOSITORY_ROOT")
}
pub fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git").args(args).current_dir(root).output()?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8(out.stdout)?)
}
pub fn relative(path: &Path, root: &Path) -> Result<String> {
    Ok(fs::canonicalize(path)?
        .strip_prefix(fs::canonicalize(root)?)
        .context("artifact outside repository")?
        .to_string_lossy()
        .replace('\\', "/"))
}
pub fn confined(root: &Path, name: &str) -> Result<PathBuf> {
    let p = Path::new(name);
    if name.is_empty()
        || p.is_absolute()
        || p.components().any(|c| !matches!(c, Component::Normal(_)))
    {
        bail!("unsafe artifact path");
    }
    let base = fs::canonicalize(root)?;
    let mut out = base.clone();
    for c in p.components() {
        out.push(c);
        if fs::symlink_metadata(&out)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            bail!("artifact contains symlink");
        }
    }
    if out.exists() && !fs::canonicalize(&out)?.starts_with(&base) {
        bail!("artifact escapes root");
    }
    Ok(out)
}
pub fn regular(p: &Path) -> bool {
    fs::symlink_metadata(p)
        .map(|m| m.is_file() && !m.file_type().is_symlink())
        .unwrap_or(false)
}
pub fn historical() -> Vec<String> {
    (1..=57).map(|n| format!("RUN-{n:03}")).collect()
}
pub fn codex() -> Vec<String> {
    (1..=18).map(|n| format!("RUN-20260803-{n:03}")).collect()
}
pub fn all_runs() -> BTreeSet<String> {
    historical().into_iter().chain(codex()).collect()
}
pub fn defect(run: &str, case: &str, original: &str) -> Option<&'static str> {
    match (run, case, original) {
        ("RUN-20260803-011", "TC-AI-011", "DEF-001") => Some("DEF-AI-001"),
        ("RUN-20260803-012", "TC-AI-012", "DEF-002") => Some("DEF-AI-002"),
        ("RUN-20260803-013", "TC-AI-013", "DEF-003") => Some("DEF-AI-003"),
        ("RUN-20260803-014", "TC-AI-014", "DEF-004") => Some("DEF-AI-004"),
        _ => None,
    }
}
pub fn primary(root: &Path) -> Result<BTreeMap<String, PathBuf>> {
    if !root.is_dir() {
        bail!("authoritative evidence package incomplete: evidence directory");
    }
    let mut out = BTreeMap::new();
    for n in PRIMARY {
        let p = confined(root, n)?;
        if !regular(&p) {
            bail!("authoritative evidence package incomplete: {n}");
        }
        out.insert(n.to_string(), p);
    }
    Ok(out)
}
pub fn records(v: &Value, keys: &[&str]) -> Result<Vec<Value>> {
    let a = if let Some(a) = v.as_array() {
        a.clone()
    } else if let Some(o) = v.as_object() {
        let mut found = None;
        for k in keys {
            if let Some(a) = v.get(*k).and_then(Value::as_array) {
                if a.first().is_some_and(Value::is_array) {
                    let h = v["header"].as_array().context("matrix header missing")?;
                    let mut rows = vec![];
                    for row in a {
                        let r = row.as_array().context("matrix row")?;
                        let mut m = serde_json::Map::new();
                        for (i, k) in h.iter().enumerate() {
                            m.insert(text(k), r.get(i).cloned().unwrap_or(Value::Null));
                        }
                        rows.push(Value::Object(m));
                    }
                    found = Some(rows);
                } else {
                    found = Some(a.clone());
                }
                break;
            }
        }
        found
            .or_else(|| {
                o.values()
                    .all(Value::is_object)
                    .then(|| o.values().cloned().collect())
            })
            .context("unsupported record envelope")?
    } else {
        bail!("manifest root must be object or array");
    };
    if !a.iter().all(Value::is_object) {
        bail!("record arrays must contain objects");
    }
    Ok(a)
}
pub fn text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        _ => v.to_string(),
    }
}
pub fn key(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .replace(['/', '-'], " ")
        .replace('&', " and ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_")
}
pub fn pick(v: &Value, names: &[&str], default: &str) -> String {
    if let Some(o) = v.as_object() {
        for n in names {
            if let Some((_, v)) = o.iter().find(|(k, _)| key(k) == key(n)) {
                let s = text(v);
                if !s.is_empty() {
                    return s;
                }
            }
        }
    }
    default.into()
}
pub fn nullable(s: String) -> Value {
    if s.is_empty() {
        Value::Null
    } else {
        Value::String(s)
    }
}
pub fn assert_unique(values: &[Value], fields: &[&str]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for row in values {
        let k = fields
            .iter()
            .map(|f| row[*f].to_string())
            .collect::<Vec<_>>();
        if !seen.insert(k) {
            bail!("duplicate identity");
        }
    }
    Ok(())
}
pub type ParsedArgs = (BTreeMap<String, Vec<String>>, Vec<String>);
pub fn parse_args(args: &[String], values: &[&str], flags: &[&str]) -> Result<ParsedArgs> {
    let mut opts = BTreeMap::<String, Vec<String>>::new();
    let mut pos = vec![];
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if values.contains(&a.as_str()) {
            i += 1;
            opts.entry(a.clone())
                .or_default()
                .push(args.get(i).context("missing option value")?.clone());
        } else if flags.contains(&a.as_str()) {
            opts.entry(a.clone()).or_default();
        } else if a.starts_with('-') {
            bail!("unknown option {a}");
        } else {
            pos.push(a.clone());
        }
        i += 1;
    }
    Ok((opts, pos))
}
pub fn option<'a>(o: &'a BTreeMap<String, Vec<String>>, k: &str, d: &'a str) -> &'a str {
    o.get(k)
        .and_then(|v| v.last())
        .map(String::as_str)
        .unwrap_or(d)
}
pub fn schema_version(v: &Value) -> Result<&str> {
    match v["schema_version"].as_str() {
        Some(LEGACY_VERSION) => Ok(LEGACY_VERSION),
        Some(NATIVE_VERSION) => Ok(NATIVE_VERSION),
        _ => bail!("unsupported artifact schema_version"),
    }
}
pub fn schema_errors(instance: &Value, path: &Path, label: &str) -> Result<Vec<String>> {
    let mut schema = read_json(path)?;
    if schema_version(instance)? == NATIVE_VERSION {
        schema["properties"]["schema_version"]["const"] = json!(NATIVE_VERSION);
    }
    let validator = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .should_validate_formats(true)
        .build(&schema)
        .context("invalid QA schema")?;
    Ok(validator
        .iter_errors(instance)
        .map(|e| format!("{label} schema violation at {}: {e}", e.instance_path()))
        .collect())
}
