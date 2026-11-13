use crate::{Result, artifact, executable};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Instant,
};
pub const POLICY: &str = "native-snapshot-known-credentials-v1";
type Files = BTreeMap<String, (Vec<u8>, u32)>;
fn refuse(code: &str) -> infra_core::RuntimeError {
    artifact::invalid(&format!("snapshot-admission:{code}"))
}
fn deadline(d: Option<Instant>) -> Result<()> {
    if d.is_some_and(|d| Instant::now() >= d) {
        Err(refuse("collection-deadline"))
    } else {
        Ok(())
    }
}
fn state(name: &str) -> bool {
    ["memory-bank/INDEX.md", "tasks/.task-counter"].contains(&name)
        || [
            "memory-bank/local/",
            "memory-bank/chunks/",
            "project-brain/dynamic/",
            "project-brain/archive/",
            "project-brain/control/",
            "project-brain/indexes/",
            "project-brain/local/",
            ".context-handoff/",
        ]
        .iter()
        .any(|p| name.starts_with(p) || name == artifact::strip_tail(p, "/"))
}
fn opaque(name: &str) -> bool {
    [
        "memory-bank/local/context.db",
        "project-brain/local/record-images.cache",
        ".git/index",
    ]
    .contains(&name)
        || name.starts_with(".git/objects/")
}
fn safe_scalar(s: &str) -> bool {
    if ["[REDACTED_PRIVATE_VALUE]", "[REDACTED]"].contains(&s) {
        return true;
    }
    if s.len() > 1024 {
        return false;
    }
    regex::Regex::new(
        r"^(?:\$\{[A-Z][A-Z0-9_]*\}|\$[A-Z][A-Z0-9_]*|%env\((?:[a-zA-Z_]\w*:)*[A-Z][A-Z0-9_]*\)%)$",
    )
    .is_ok_and(|r| r.is_match(s))
}
fn credential_key(k: &str) -> bool {
    let k = k.to_lowercase();
    regex::Regex::new(r"^[a-z0-9_.-]*(?:(?:password|passwd|passphrase)(?:[_.-]?(?:hash|digest))?|secret(?:[_.-]?(?:access|key|token|passphrase))*|api[_.-]?key|access[_.-]?(?:key|token)|auth[_.-]?token|refresh[_.-]?token|private[_.-]?key|account[_.-]?key|shared[_.-]?access[_.-]?key|master[_.-]?key|app[_.-]?key|s3[_.-](?:media[_.-])?key|meili(?:search)?[_.-](?:master[_.-])?key|registry[_.-]?credentials)$").is_ok_and(|r|r.is_match(&k))
}
fn known(text: &str) -> Result<()> {
    if regex::Regex::new(r"\bEV[ \t\r\n]*\[").is_ok_and(|r| r.is_match(text))
        || regex::Regex::new(r"(?i)[a-z][a-z0-9+.-]*://[^\s/@:]*:[^\s/@]*@")
            .is_ok_and(|r| r.is_match(text))
    {
        return Err(refuse("known-credential"));
    }
    if infra_core::domain::secret_label(text)?
        .is_some_and(|label| label != "assigned credential" && label != "credential in URL")
    {
        return Err(refuse("known-credential"));
    }
    Ok(())
}
fn credential_field(k: &str, value: &Value) -> bool {
    if credential_key(k) {
        return true;
    }
    if !["authorization", "proxy-authorization"].contains(&k.to_lowercase().as_str()) {
        return false;
    }
    let mut pending = vec![value];
    let mut visited = 0;
    while let Some(v) = pending.pop() {
        visited += 1;
        if visited > 20000 {
            return true;
        }
        if let Some(a) = v.as_array() {
            pending.extend(a);
        } else if let Some(s) = v.as_str() {
            let s = s.trim_start_matches([' ', '\t', '\r', '\n', '\'', '"']);
            if s.starts_with("Basic ") || s.starts_with("Bearer ") {
                return true;
            }
        }
    }
    false
}
fn plain(text: &str, d: Option<Instant>) -> Result<()> {
    let re = regex::Regex::new(
        r#"(?m)(['"`]?)([A-Za-z_][A-Za-z0-9_.-]*)['"`]?[ \t\r\n]*\]?[ \t\r\n]*(:|=>|=)[ \t]*"#,
    )
    .map_err(|_| refuse("field-expression"))?;
    let mut container = false;
    let mut values = vec![];
    for (i, m) in re.captures_iter(text).enumerate() {
        deadline(d)?;
        let hit = m.get(0).unwrap();
        if hit.start() > 0
            && text[..hit.start()]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
        {
            continue;
        }
        if i >= 20000 || m[2].len() > 256 {
            return Err(refuse("field-limit"));
        }
        let tail = &text[hit.end()..];
        let rhs = tail.lines().next().unwrap_or("").trim_matches([' ', '\t']);
        let mut rhs = rhs.to_string();
        if rhs.len() >= 2
            && rhs.starts_with(['\'', '"'])
            && rhs.chars().next() == rhs.chars().next_back()
        {
            rhs = rhs[1..rhs.len() - 1].to_string();
        }
        if rhs.len() > 1024 {
            rhs.clear();
        }
        if &m[3] == ":" && rhs.starts_with(['$', '%']) {
            rhs.clear();
        }
        if credential_field(&m[2], &json!(rhs)) && !safe_scalar(&rhs) {
            return Err(refuse("credential-assignment"));
        }
        let key = m[2].to_lowercase();
        if key == "value" {
            values.push(rhs.clone());
        }
        if ["key", "name", "type"].contains(&key.as_str()) {
            let token = rhs
                .trim_matches(['\'', '"'])
                .split(|c: char| c.is_whitespace() || ",;} ]#".contains(c))
                .next()
                .unwrap_or("");
            if key == "type" && token.eq_ignore_ascii_case("secret")
                || key != "type" && credential_key(token)
            {
                container = true;
            }
        }
    }
    if container && (values.is_empty() || values.iter().any(|s| !safe_scalar(s))) {
        return Err(refuse("credential-container"));
    }
    Ok(())
}
fn fields(value: &Value, d: Option<Instant>) -> Result<()> {
    let mut pending = vec![value];
    let mut count = 0;
    while let Some(v) = pending.pop() {
        deadline(d)?;
        count += 1;
        if count > 20000 {
            return Err(refuse("field-limit"));
        }
        match v {
            Value::Object(m) => {
                let mut secret = false;
                for (k, v) in m {
                    if k.len() > 256 {
                        return Err(refuse("field-limit"));
                    }
                    known(k)?;
                    plain(k, d)?;
                    if credential_field(k, v) && !v.as_str().is_some_and(safe_scalar) {
                        return Err(refuse("credential-assignment"));
                    }
                    if ["key", "name"].contains(&k.to_lowercase().as_str())
                        && v.as_str().is_some_and(credential_key)
                        || k.eq_ignore_ascii_case("type")
                            && v.as_str().is_some_and(|s| s.eq_ignore_ascii_case("secret"))
                    {
                        secret = true;
                    }
                    pending.push(v);
                }
                if secret
                    && !m
                        .get("value")
                        .and_then(Value::as_str)
                        .is_some_and(safe_scalar)
                {
                    return Err(refuse("credential-container"));
                }
            }
            Value::Array(a) => pending.extend(a),
            Value::String(s) => {
                known(s)?;
                plain(s, d)?;
            }
            _ => {}
        }
    }
    Ok(())
}
fn ini(text: &str, d: Option<Instant>) -> Result<()> {
    let section_re = regex::Regex::new(r#"^\[[A-Za-z0-9_. *?-]+(?: "[A-Za-z0-9_. -]+")?\][ \t]*$"#)
        .map_err(|_| refuse("invalid-structured-text"))?;
    let key_re =
        regex::Regex::new(r"^[A-Za-z0-9_.-]+$").map_err(|_| refuse("invalid-structured-text"))?;
    let mut section = String::new();
    let mut sections = BTreeSet::new();
    let mut keys = BTreeSet::new();
    let mut rows = serde_json::Map::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if line.starts_with('[') {
            if !section_re.is_match(line) || !sections.insert(line.to_string()) {
                return Err(refuse("invalid-structured-text"));
            }
            section = line.into();
            continue;
        }
        let Some((k, v)) = line.split_once('=').or_else(|| line.split_once(':')) else {
            return Err(refuse("invalid-structured-text"));
        };
        let k = k.trim();
        if section.is_empty()
            || !key_re.is_match(k)
            || !keys.insert((section.clone(), k.to_lowercase()))
        {
            return Err(refuse("invalid-structured-text"));
        }
        let v = v.trim();
        let v = if v.len() >= 2
            && v.starts_with(['\'', '"'])
            && v.chars().next() == v.chars().next_back()
        {
            &v[1..v.len() - 1]
        } else {
            v
        };
        rows.insert(k.into(), v.into());
    }
    fields(&Value::Object(rows), d)
}
fn text(raw: &[u8], d: Option<Instant>) -> Result<()> {
    let s = std::str::from_utf8(raw).map_err(|_| refuse("unscannable-text"))?;
    if s.chars()
        .any(|c| (c < ' ' && !['\t', '\r', '\n'].contains(&c)) || c == '\u{7f}')
    {
        return Err(refuse("unscannable-text"));
    }
    known(s)?;
    let normalized = s.trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n']);
    if normalized.starts_with(['{', '[']) {
        match artifact::strict(normalized.as_bytes()) {
            Ok(v) => fields(&v, d),
            Err(_) => ini(normalized, d),
        }
    } else {
        plain(s, d)
    }
}
fn engine_authority(canonical: &Files) -> Result<Option<executable::CanonicalEngine<'_>>> {
    let mut engine = None;
    for name in [
        "memory-bank/bin/infra-engine",
        "memory-bank/bin/infra-engine.exe",
    ] {
        if let Some((bytes, mode)) = canonical.get(name) {
            if engine.is_some() {
                return Err(refuse("ambiguous-canonical-engine"));
            }
            engine = Some(executable::CanonicalEngine::new(name, bytes, *mode)?);
        }
    }
    Ok(engine)
}
/// Capture with a separate engine budget only when its exact canonical image is admitted.
pub fn admitted_image(
    root: &Path,
    kind: artifact::ImageKind,
    canonical: &Files,
    d: Option<Instant>,
) -> Result<Vec<artifact::ImageFile>> {
    if let Some(engine) = engine_authority(canonical)? {
        artifact::image_with_canonical_engine(root, kind, d, &engine)
    } else {
        artifact::image(root, kind, d)
    }
}

pub fn admit(
    rows: &[artifact::ImageFile],
    canonical: &Files,
    generated: bool,
    d: Option<Instant>,
) -> Result<Value> {
    let engine = engine_authority(canonical)?;
    let (mut seen, mut bytes, mut exact, mut derived) = (BTreeSet::new(), 0usize, 0, 0);
    let (mut data_bytes, mut engine_bytes) = (0usize, 0usize);
    for f in rows {
        deadline(d)?;
        artifact::relative(&f.path)?;
        if !generated && (!artifact::source_allowed(&f.path) || f.path.starts_with(".git/")) {
            return Err(refuse("private-path"));
        }
        if f.path.split('/').any(|p| {
            p.to_lowercase().starts_with(".env")
                || ["auth.json", "credentials.json", "settings.local.json"]
                    .contains(&p.to_lowercase().as_str())
        }) && !(generated && f.path == "memory-bank/local/native-ab/home/auth.json")
        {
            return Err(refuse("private-path"));
        }
        if !seen.insert(&f.path) || f.mode > 0o777 {
            return Err(refuse("invalid-image"));
        }
        let exact_engine = if let Some(authority) = engine.as_ref().filter(|a| a.path() == f.path) {
            if !authority.matches(&f.path, &f.bytes, f.mode) {
                return Err(refuse("modified-native-engine"));
            }
            true
        } else if executable::is_engine_slot(&f.path) {
            return Err(refuse("unapproved-native-engine"));
        } else {
            false
        };
        bytes = bytes
            .checked_add(f.bytes.len())
            .ok_or_else(|| refuse("image-limit"))?;
        if exact_engine {
            engine_bytes = engine_bytes
                .checked_add(f.bytes.len())
                .ok_or_else(|| refuse("image-limit"))?;
        } else {
            data_bytes = data_bytes
                .checked_add(f.bytes.len())
                .ok_or_else(|| refuse("image-limit"))?;
        }
        let file_limit = if exact_engine {
            executable::LIMIT
        } else {
            artifact::MAX_FILE
        };
        if f.bytes.len() > file_limit
            || data_bytes > artifact::MAX_TREE
            || engine_bytes > executable::LIMIT
            || bytes > artifact::MAX_TREE + executable::LIMIT
            || seen.len() > 20000
        {
            return Err(refuse("image-limit"));
        }
        if generated && f.path == "memory-bank/local/native-ab/home/auth.json" {
            if !f.bytes.is_empty() || f.mode != 0o600 {
                return Err(refuse("invalid-transport-stub"));
            }
            derived += 1;
        } else if generated && opaque(&f.path) {
            derived += 1;
        } else if exact_engine || canonical.get(&f.path) == Some(&(f.bytes.clone(), f.mode)) {
            exact += 1;
        } else {
            if canonical.contains_key(&f.path)
                && (f.path.starts_with("memory-bank/scripts/")
                    || f.path.starts_with("project-brain/scripts/"))
            {
                return Err(refuse("modified-runtime"));
            }
            text(&f.bytes, d)?;
        }
    }
    Ok(
        json!({"policy":POLICY,"image_sha256":artifact::image_digest(rows,true)?,"files":seen.len(),"bytes":bytes,"data_bytes":data_bytes,"native_engine_bytes":engine_bytes,"budget_profile":"canonical-native-engine-v1","exact_canonical_files":exact,"fresh_derived_files":derived}),
    )
}
pub fn canonical(installer: &infra_tooling::installer::Installer) -> Result<Files> {
    let inventory = installer.inventory(
        "PHP Core",
        &crate::reconstruction::TOOLS
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
    )?;
    inventory
        .files
        .into_iter()
        .map(|f| {
            let (bytes, _) = executable::read_inventory_source(&f, artifact::MAX_FILE)?;
            Ok((f.path, (bytes, if f.executable { 0o755 } else { 0o644 })))
        })
        .collect()
}
fn directories(root: &Path) -> Result<Value> {
    let mut rows = vec![];
    fn walk(root: &Path, at: &Path, rows: &mut Vec<Value>) -> Result<()> {
        for e in std::fs::read_dir(at)? {
            let p = e?.path();
            let m = std::fs::symlink_metadata(&p)?;
            if infra_core::fs::linked(&m) {
                return Err(refuse("unsafe-tree"));
            }
            if m.is_dir() {
                let name = artifact::snapshot_name(
                    p.strip_prefix(root).map_err(|_| refuse("unsafe-tree"))?,
                )?;
                artifact::relative(&name)?;
                rows.push(json!([name, artifact::mode(&p)?]));
                walk(root, &p, rows)?;
            }
        }
        Ok(())
    }
    walk(root, root, &mut rows)?;
    rows.sort_by(|a, b| a[0].as_str().cmp(&b[0].as_str()));
    Ok(json!({"root_mode":artifact::mode(root)?,"directories":rows}))
}
fn manifest(rows: &[artifact::ImageFile], dirs: &Value) -> Result<String> {
    artifact::spaced_digest(
        &json!({"files":rows.iter().map(|f|json!([f.path,artifact::sha(&f.bytes),f.mode])).collect::<Vec<_>>(),"folders":dirs}),
        true,
        false,
    )
}
pub struct MaterializedGate {
    root: PathBuf,
    canonical: Files,
    protected: Files,
    initial: BTreeSet<String>,
    dirs: BTreeSet<String>,
    derived: Option<Files>,
    derived_dirs: Option<Value>,
    expected: Option<String>,
    deadline: Option<Instant>,
}
impl MaterializedGate {
    pub fn new(
        root: &Path,
        source: &[artifact::ImageFile],
        expected_overlay: &[artifact::ImageFile],
        canonical: Files,
        d: Option<Instant>,
    ) -> Result<Self> {
        let image = admitted_image(root, artifact::ImageKind::Physical, &canonical, d)?;
        let dirs = directories(root)?;
        if dirs["root_mode"] != 0o700 {
            return Err(refuse("nonprivate-root"));
        }
        let allowed = source
            .iter()
            .map(|f| f.path.clone())
            .chain(canonical.keys().cloned())
            .collect::<BTreeSet<_>>();
        let parents = allowed
            .iter()
            .flat_map(|n| {
                Path::new(n)
                    .ancestors()
                    .skip(1)
                    .filter(|p| !p.as_os_str().is_empty())
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .collect::<Vec<_>>()
            })
            .collect::<BTreeSet<_>>();
        for row in dirs["directories"].as_array().unwrap() {
            let name = row[0].as_str().unwrap();
            if name == ".git"
                || name.starts_with(".git/")
                || !parents.contains(name) && name != ".context-handoff"
            {
                return Err(refuse("unexpected-initial-directory"));
            }
        }
        let actual = image
            .iter()
            .map(|f| (f.path.clone(), (f.bytes.clone(), f.mode)))
            .collect::<Files>();
        for f in &image {
            if !allowed.contains(&f.path) || f.path.starts_with(".git/") || opaque(&f.path) {
                return Err(refuse("nonfresh-initial-tree"));
            }
        }
        for (name, bytes) in &canonical {
            if !state(name)
                && ![
                    "AGENTS.md",
                    "README.md",
                    ".gitignore",
                    ".gitattributes",
                    ".claude/CLAUDE.md",
                ]
                .contains(&name.as_str())
                && if executable::is_engine_slot(name) {
                    !actual.contains_key(name)
                } else {
                    actual.get(name) != Some(bytes)
                }
            {
                return Err(refuse("canonical-overlay-changed"));
            }
        }
        let expected = expected_overlay
            .iter()
            .map(|f| (f.path.clone(), (f.bytes.clone(), f.mode)))
            .collect::<Files>();
        if actual != expected {
            return Err(refuse("held-source-or-canonical-overlay-changed"));
        }
        admit(&image, &canonical, false, d)?;
        let protected = actual.into_iter().filter(|(n, _)| !state(n)).collect();
        Ok(Self {
            root: root.into(),
            canonical,
            protected,
            initial: image.iter().map(|f| f.path.clone()).collect(),
            dirs: dirs["directories"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|r| r[0].as_str().map(String::from))
                .collect(),
            derived: None,
            derived_dirs: None,
            expected: None,
            deadline: d,
        })
    }
    pub fn image(&self, kind: artifact::ImageKind) -> Result<Vec<artifact::ImageFile>> {
        admitted_image(&self.root, kind, &self.canonical, self.deadline)
    }
    fn prepared(&self) -> Result<(Vec<artifact::ImageFile>, Value)> {
        let image = self.image(artifact::ImageKind::Physical)?;
        let dirs = directories(&self.root)?;
        if dirs["root_mode"] != 0o700 {
            return Err(refuse("nonprivate-root"));
        }
        let actual = image
            .iter()
            .map(|f| (f.path.clone(), (f.bytes.clone(), f.mode)))
            .collect::<Files>();
        if self.protected.iter().any(|(n, v)| actual.get(n) != Some(v)) {
            return Err(refuse("protected-image-changed"));
        }
        for n in actual.keys() {
            if !self.initial.contains(n) && !state(n) && !n.starts_with(".git/") {
                return Err(refuse("unexpected-file"));
            }
        }
        for n in dirs["directories"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|r| r[0].as_str())
        {
            if !self.dirs.contains(n) && n != ".git" && !n.starts_with(".git/") && !state(n) {
                return Err(refuse("unexpected-directory"));
            }
        }
        Ok((image, dirs))
    }
    pub fn seal_generated(&mut self) -> Result<()> {
        if self.derived.is_some() {
            return Err(refuse("derived-already-fixed"));
        }
        let (image, dirs) = self.prepared()?;
        admit(&image, &self.canonical, true, self.deadline)?;
        self.derived = Some(
            image
                .iter()
                .filter(|f| opaque(&f.path))
                .map(|f| (f.path.clone(), (f.bytes.clone(), f.mode)))
                .collect(),
        );
        self.derived_dirs = Some(json!(
            dirs["directories"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r[0] == ".git" || r[0].as_str().is_some_and(|s| s.starts_with(".git/")))
                .collect::<Vec<_>>()
        ));
        Ok(())
    }
    pub fn checkpoint(&mut self) -> Result<Value> {
        if self.expected.is_some() || self.derived.is_none() {
            return Err(refuse("invalid-checkpoint-state"));
        }
        let (image, dirs) = self.prepared()?;
        let current = image
            .iter()
            .filter(|f| opaque(&f.path))
            .map(|f| (f.path.clone(), (f.bytes.clone(), f.mode)))
            .collect::<Files>();
        let current_dirs = json!(
            dirs["directories"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r[0] == ".git" || r[0].as_str().is_some_and(|s| s.starts_with(".git/")))
                .collect::<Vec<_>>()
        );
        if self.derived.as_ref() != Some(&current)
            || self.derived_dirs.as_ref() != Some(&current_dirs)
        {
            return Err(refuse("derived-image-changed"));
        }
        let mut report = admit(&image, &self.canonical, true, self.deadline)?;
        let digest = manifest(&image, &dirs)?;
        self.expected = Some(digest.clone());
        report["materialized_sha256"] = digest.into();
        report["derived_origin"] = "fresh-controller-git-and-index-from-admitted-bytes".into();
        Ok(report)
    }
    pub fn verify(&self) -> Result<Value> {
        let expected = self
            .expected
            .as_ref()
            .ok_or_else(|| refuse("checkpoint-required"))?;
        let image = self.image(artifact::ImageKind::Physical)?;
        if manifest(&image, &directories(&self.root)?)? != *expected {
            return Err(refuse("materialized-image-changed"));
        }
        Ok(json!({"policy":POLICY,"materialized_sha256":expected,"verified":true}))
    }
}

#[cfg(test)]
mod canonical_engine_budget_tests {
    use super::*;

    const ENGINE: &str = if cfg!(windows) {
        "memory-bank/bin/infra-engine.exe"
    } else {
        "memory-bank/bin/infra-engine"
    };
    fn image_mode() -> u32 {
        if cfg!(unix) { 0o755 } else { 0o644 }
    }
    // A structurally valid ELF header for byte-admission tests only. This fixture
    // is never executed; the public CLI regression uses the actual Cargo engine.
    fn native_bytes(size: usize) -> Vec<u8> {
        let mut bytes = vec![0; size.max(64)];
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        bytes[16..18].copy_from_slice(&2u16.to_le_bytes());
        bytes[18..20].copy_from_slice(&62u16.to_le_bytes());
        bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
        bytes[52..54].copy_from_slice(&64u16.to_le_bytes());
        bytes
    }
    fn canonical(bytes: Vec<u8>) -> Files {
        BTreeMap::from([(ENGINE.to_string(), (bytes, 0o755))])
    }
    fn row(path: &str, bytes: Vec<u8>, mode: u32) -> artifact::ImageFile {
        artifact::ImageFile {
            path: path.into(),
            bytes,
            mode,
        }
    }
    #[test]
    fn exact_canonical_engine_has_a_separate_raw_budget() {
        let work = artifact::Scratch::new(&std::env::temp_dir(), "engine-budget").unwrap();
        let root = work.0.join("snapshot");
        artifact::private_dir(&root).unwrap();
        let bytes = native_bytes(artifact::MAX_TREE + 1);
        let size = bytes.len();
        artifact::fresh_file(&root.join(ENGINE), &bytes, 0o755).unwrap();
        artifact::fresh_file(&root.join("spec.md"), b"Public fixture.\n", 0o644).unwrap();
        let canonical = canonical(bytes);
        assert!(
            artifact::image(&root, artifact::ImageKind::NativePublic, None).is_err(),
            "the generic snapshot budget must remain unchanged"
        );
        let image =
            admitted_image(&root, artifact::ImageKind::NativePublic, &canonical, None).unwrap();
        let report = admit(&image, &canonical, false, None).unwrap();
        assert_eq!(report["native_engine_bytes"], size);
        assert_eq!(report["data_bytes"], b"Public fixture.\n".len());
        assert_eq!(report["bytes"], size + b"Public fixture.\n".len());
        assert_eq!(report["budget_profile"], "canonical-native-engine-v1");
        assert_eq!(report["exact_canonical_files"], 1);
    }
    #[test]
    fn engine_authority_rejects_substitution_and_preserves_installed_file_policy() {
        let work = artifact::Scratch::new(&std::env::temp_dir(), "engine-authority").unwrap();
        let root = work.0.join("snapshot");
        artifact::private_dir(&root).unwrap();
        let bytes = native_bytes(128);
        let canonical = canonical(bytes.clone());
        let mut changed = bytes.clone();
        changed[127] = 1;
        assert!(
            admit(
                &[row(ENGINE, changed, image_mode())],
                &canonical,
                false,
                None
            )
            .is_err()
        );
        assert!(
            admit(
                &[row(ENGINE, bytes.clone(), 0o444)],
                &canonical,
                false,
                None
            )
            .is_err()
        );
        let alternate = if ENGINE.ends_with(".exe") {
            "memory-bank/bin/infra-engine"
        } else {
            "memory-bank/bin/infra-engine.exe"
        };
        assert!(
            admit(
                &[row(alternate, bytes.clone(), image_mode())],
                &canonical,
                false,
                None
            )
            .is_err()
        );
        assert!(
            admit(
                &[row(ENGINE, bytes.clone(), image_mode())],
                &Files::new(),
                false,
                None
            )
            .is_err()
        );
        assert!(executable::CanonicalEngine::new(ENGINE, &bytes, 0o644).is_err());
        artifact::fresh_file(&root.join(ENGINE), &bytes, 0o755).unwrap();
        assert!(admitted_image(&root, artifact::ImageKind::NativePublic, &canonical, None).is_ok());
        #[cfg(unix)]
        {
            let second = work.0.join("second-image");
            std::fs::hard_link(root.join(ENGINE), &second).unwrap();
            assert!(
                admitted_image(&root, artifact::ImageKind::NativePublic, &canonical, None).is_err(),
                "installed corpus images still require one link"
            );
            std::fs::remove_file(second).unwrap();
        }
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(root.join(ENGINE))
            .unwrap();
        file.set_len(executable::LIMIT as u64 + 1).unwrap();
        drop(file);
        assert!(
            admitted_image(&root, artifact::ImageKind::NativePublic, &canonical, None).is_err(),
            "the raw engine bound is still 512 MiB"
        );
        std::fs::remove_file(root.join(ENGINE)).unwrap();
        std::fs::create_dir(root.join(ENGINE)).unwrap();
        assert!(
            admitted_image(&root, artifact::ImageKind::NativePublic, &canonical, None).is_err()
        );
    }
    #[test]
    fn canonical_engine_does_not_expand_ordinary_data_budgets() {
        let engine = native_bytes(64);
        let mut canonical = canonical(engine.clone());
        let oversized = vec![b'x'; artifact::MAX_FILE + 1];
        canonical.insert("large.md".into(), (oversized.clone(), 0o644));
        let rows = [
            row(ENGINE, engine.clone(), image_mode()),
            row("large.md", oversized, 0o644),
        ];
        assert!(
            admit(&rows, &canonical, false, None).is_err(),
            "even exact canonical data retains the 4 MiB file bound"
        );
        drop(rows);
        canonical.remove("large.md");
        let mut rows = vec![row(ENGINE, engine, image_mode())];
        for index in 0..17 {
            let name = format!("data-{index:02}.md");
            let bytes = vec![b'x'; artifact::MAX_FILE];
            canonical.insert(name.clone(), (bytes.clone(), 0o644));
            rows.push(row(&name, bytes, 0o644));
        }
        assert!(
            admit(&rows, &canonical, false, None).is_err(),
            "ordinary data retains the independent 64 MiB aggregate bound"
        );
    }
    #[cfg(unix)]
    #[test]
    fn materialized_gate_carries_exact_engine_authority_through_checkpoints() {
        let work = artifact::Scratch::new(&std::env::temp_dir(), "engine-gate").unwrap();
        let root = work.0.join("snapshot");
        artifact::private_dir(&root).unwrap();
        let bytes = native_bytes(128);
        artifact::fresh_file(&root.join(ENGINE), &bytes, 0o755).unwrap();
        artifact::fresh_file(&root.join("spec.md"), b"Public fixture.\n", 0o644).unwrap();
        let canonical = canonical(bytes);
        let image =
            admitted_image(&root, artifact::ImageKind::NativePublic, &canonical, None).unwrap();
        let mut gate = MaterializedGate::new(&root, &image, &image, canonical, None).unwrap();
        gate.seal_generated().unwrap();
        let report = gate.checkpoint().unwrap();
        assert_eq!(report["native_engine_bytes"], 128);
        assert_eq!(gate.verify().unwrap()["verified"], true);
        let mut changed = native_bytes(128);
        changed[127] = 1;
        std::fs::write(root.join(ENGINE), changed).unwrap();
        assert!(
            gate.verify().is_err(),
            "checkpoint verification must never adopt changed native engine bytes"
        );
    }
}
