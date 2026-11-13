//! One retrieval's current source policy. Cached metadata and held index images
//! aid discovery; each selected document still receives a fresh source read.
use crate::{
    ErrorKind, Result, RuntimeConfig, RuntimeContext, RuntimeError,
    domain::{self, json},
    excerpts::IndexedImage,
    index::{self, Document, Exclusion},
    observability::Trace,
    query::{self, Candidate},
};
use rusqlite::{Connection, params_from_iter};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::Duration,
};
pub const HINT_FILE_BYTES: usize = 131072;
fn source_git() -> RuntimeError {
    RuntimeError::coded(
        ErrorKind::Unavailable,
        "source-git",
        "Git source context unavailable",
    )
}
pub fn project_source_key(context: &RuntimeContext, name: &str) -> Result<Option<String>> {
    if name.is_empty() || name.contains('\0') {
        return Err(source_git());
    }
    if name.starts_with("http://") || name.starts_with("https://") {
        return Ok(None);
    }
    let relative = name.split('#').next().unwrap_or(name);
    let named = context.roots.resolve(relative);
    let lexical = crate::fs::absolute(&named).map_err(|_| source_git())?;
    let physical = crate::fs::canonical_or_missing(&named).map_err(|_| source_git())?;
    let project = crate::fs::canonicalize(&context.roots.project).map_err(|_| source_git())?;
    let relative = match lexical.strip_prefix(&project) {
        Ok(relative) => relative,
        Err(_) => {
            if context.roots.attached() {
                for base in [&context.roots.state, &context.roots.tooling] {
                    let base = crate::fs::canonical_or_missing(base).map_err(|_| source_git())?;
                    if lexical.starts_with(&base) && physical.starts_with(&base) {
                        return Ok(None);
                    }
                }
            }
            return Err(source_git());
        }
    };
    if !physical.starts_with(&project) {
        return Err(source_git());
    }
    let text = relative.to_str().ok_or_else(source_git)?.replace('\\', "/");
    Ok(Some(if text.is_empty() { ".".into() } else { text }))
}
#[derive(Clone, Debug, Default)]
pub struct CurrentSourcePolicy {
    root: Option<PathBuf>,
    project: Option<PathBuf>,
    connection: Option<usize>,
    rows: BTreeMap<String, Option<String>>,
    observed: BTreeMap<String, bool>,
    local_sources: BTreeMap<String, BTreeSet<String>>,
    exists: BTreeMap<String, bool>,
    boundaries: BTreeMap<String, Option<String>>,
    images: BTreeMap<String, (String, IndexedImage)>,
}
impl CurrentSourcePolicy {
    pub fn prime(
        &mut self,
        connection: &Connection,
        context: &RuntimeContext,
        candidates: &mut [Candidate],
    ) -> Result<()> {
        let root = crate::fs::canonical_or_missing(&context.roots.state)?;
        let project = crate::fs::canonicalize(&context.roots.project)?;
        let identity = connection as *const Connection as usize;
        if self.root.as_ref().is_some_and(|held| held != &root)
            || self.project.as_ref().is_some_and(|held| held != &project)
            || self.connection.is_some_and(|held| held != identity)
        {
            return Err(source_git());
        }
        self.root = Some(root);
        self.project = Some(project);
        self.connection = Some(identity);
        for item in candidates.iter_mut() {
            self.images
                .entry(item.document.path.clone())
                .or_insert_with(|| {
                    (
                        item.held_source_hash
                            .clone()
                            .unwrap_or_else(|| item.document.source_hash.clone()),
                        item.image(),
                    )
                });
            if let Some((digest, image)) = self.images.get(&item.document.path) {
                item.held_source_hash = Some(digest.clone());
                item.held_image = Some(image.clone());
            }
        }
        let names = candidates
            .iter()
            .map(|item| item.document.path.clone())
            .filter(|name| !self.rows.contains_key(name))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut pending = BTreeMap::new();
        for batch in names.chunks(400) {
            let marks = vec!["?"; batch.len()].join(",");
            let mut local = batch
                .iter()
                .map(|name| (name.clone(), BTreeSet::new()))
                .collect::<BTreeMap<_, _>>();
            let mut statement = connection.prepare(&format!(
                "SELECT path,eligible_until FROM document_source_state WHERE path IN ({marks})"
            ))?;
            for row in statement.query_map(params_from_iter(batch), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })? {
                let (path, boundary) = row?;
                self.boundaries.insert(path, boundary);
            }
            let mut statement=connection.prepare(&format!("SELECT path,ref_path FROM document_links WHERE ref_kind='source' AND path IN ({marks})"))?;
            for row in statement.query_map(params_from_iter(batch), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })? {
                let (path, reference) = row?;
                if !reference.starts_with("http://") && !reference.starts_with("https://") {
                    local.entry(path).or_default().insert(reference);
                }
            }
            let mut statement = connection.prepare(&format!(
                "SELECT path,source_fingerprints FROM document_metadata WHERE path IN ({marks})"
            ))?;
            for row in statement.query_map(params_from_iter(batch), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })? {
                let (path, fingerprints) = row?;
                let value = json::parse(&fingerprints)?;
                let rows = value
                    .as_array()
                    .ok_or_else(|| RuntimeError::invalid("Invalid indexed source fingerprints"))?;
                for row in rows {
                    let reference = row["path"]
                        .as_str()
                        .ok_or_else(|| RuntimeError::invalid("Invalid indexed source path"))?;
                    if !reference.starts_with("http://") && !reference.starts_with("https://") {
                        local
                            .entry(path.clone())
                            .or_default()
                            .insert(reference.into());
                    }
                }
            }
            for name in batch {
                let own = if crate::workspace::accelerator_owned_source(name)
                    || name.starts_with("project-brain/")
                {
                    None
                } else {
                    project_source_key(context, name)?
                };
                pending.insert(name.clone(), own);
                self.local_sources
                    .insert(name.clone(), local.remove(name).unwrap_or_default());
            }
        }
        let delta = pending
            .values()
            .flatten()
            .filter(|key| !self.observed.contains_key(*key))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let ignored = crate::cursor_rule::source_git_ignored_paths(
            &context.roots.project,
            &delta,
            Duration::from_secs(2),
        )?;
        for key in delta {
            self.observed.insert(key.clone(), ignored.contains(&key));
        }
        self.rows.extend(pending);
        Ok(())
    }
    pub fn image(&self, path: &str) -> Option<&IndexedImage> {
        self.images.get(path).map(|(_, image)| image)
    }
    pub fn image_source_hash(&self, path: &str) -> Option<&str> {
        self.images.get(path).map(|(digest, _)| digest.as_str())
    }
    pub fn reason(&self, item: &Candidate, temporal: bool) -> Result<Option<String>> {
        let own = self
            .rows
            .get(&item.document.path)
            .ok_or_else(|| RuntimeError::unavailable("Current source policy was not primed"))?;
        if own
            .as_ref()
            .is_some_and(|own| self.observed.get(own) == Some(&true))
        {
            return Ok(Some("ignored".into()));
        }
        if item
            .held_source_hash
            .as_deref()
            .is_some_and(|digest| digest != item.document.source_hash)
        {
            return Ok(Some("stale".into()));
        }
        let image = item
            .held_image
            .as_ref()
            .or_else(|| self.image(&item.document.path));
        let Some(image) = image else {
            return Ok(Some("stale".into()));
        };
        if item.selection.as_deref() == Some("source-hint")
            && (image.content.len() > HINT_FILE_BYTES
                || image.content.chars().count() > HINT_FILE_BYTES)
        {
            return Ok(Some("stale".into()));
        }
        if domain::indexed_secret_policy_reason(&[&image.title, &image.summary, &image.content])
            .is_some()
            || domain::indexed_secret_policy_reason(&[
                &item.document.path,
                &item.document.title,
                &item.snippet,
            ])
            .is_some()
        {
            return Ok(Some("secret-policy".into()));
        }
        if temporal
            && item.document.kind == "memory"
            && let Some(Some(boundary)) = self.boundaries.get(&item.document.path)
            && let Some(reason) = index::memory_temporal_reason(
                &serde_json::json!({"status":"active","review_after":boundary}),
            )
        {
            return Ok(Some(reason));
        }
        Ok(None)
    }
    pub fn missing_source(&mut self, context: &RuntimeContext, item: &Candidate) -> Result<bool> {
        let names = self
            .local_sources
            .get(&item.document.path)
            .cloned()
            .ok_or_else(|| RuntimeError::unavailable("Current source policy was not primed"))?;
        for name in names {
            if !self.exists.contains_key(&name) {
                self.exists
                    .insert(name.clone(), citation_exists(context, &name));
            }
            if self.exists.get(&name) != Some(&true) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
fn citation_exists(context: &RuntimeContext, name: &str) -> bool {
    let operation = (|| -> Result<bool> {
        let name = name.split('#').next().unwrap_or(name);
        let path = context.roots.resolve(name);
        let resolved = crate::fs::canonicalize(&path)?;
        if !context.roots.contains(&resolved)? {
            return Ok(false);
        }
        let metadata = std::fs::metadata(&resolved)?;
        if metadata.is_file() {
            return Ok(true);
        }
        if !metadata.is_dir() {
            return Ok(false);
        }
        #[cfg(not(windows))]
        let symlink = std::fs::symlink_metadata(&path)?.file_type().is_symlink();
        #[cfg(windows)]
        let symlink = {
            std::fs::symlink_metadata(&path)?;
            let image = crate::fs::native_image(&path)?
                .ok_or_else(|| RuntimeError::unavailable("Citation image unavailable"))?;
            image.reparse_tag == 0xA000000C
        };
        Ok(!symlink)
    })();
    operation.unwrap_or(false)
}
pub fn raw_hint_read(
    context: &RuntimeContext,
    name: &str,
    expected: &str,
) -> Result<Option<String>> {
    let operation = (|| -> Result<Option<String>> {
        let mut files = crate::bank::publication::cursor::CursorFiles::new(&context.roots.project)?;
        let Some(image) = files.image(name, HINT_FILE_BYTES, 1)? else {
            return Ok(None);
        };
        if query::sha256(&image.bytes) != expected {
            return Ok(None);
        }
        files.check()?;
        let text = String::from_utf8(image.bytes)
            .map_err(|_| RuntimeError::invalid("Hint source is not UTF-8"))?;
        Ok(Some(text.replace("\r\n", "\n").replace('\r', "\n")))
    })();
    match operation {
        Ok(value) => Ok(value),
        Err(error) if error.code.as_deref() == Some("source-git") => Err(error),
        Err(_) => Ok(None),
    }
}
pub fn point_reason(
    context: &RuntimeContext,
    config: &RuntimeConfig,
    item: &Candidate,
    policy_reason: Option<String>,
) -> Result<Option<String>> {
    let document = &item.document;
    if let Some(reason) = metadata_reason(config, document)? {
        return Ok(Some(reason));
    }
    if let Some(reason) = policy_reason {
        return Ok(Some(reason));
    }
    let content = if item.selection.as_deref() == Some("source-hint") {
        let Some(expected) = item.hint_raw_sha256.as_deref() else {
            return Ok(Some("stale".into()));
        };
        raw_hint_read(context, &document.path, expected)?
    } else {
        let path = context.roots.resolve(&document.path);
        let resolved = match crate::fs::canonicalize(&path) {
            Ok(path) => path,
            Err(_) => return Ok(Some("stale".into())),
        };
        if !context.roots.contains(&resolved)? {
            return Ok(Some("outside-project".into()));
        }
        match crate::fs::read_text(&path) {
            Ok(content) => Some(content),
            Err(error) if error.code.as_deref() == Some("source-git") => return Err(error),
            Err(_) => None,
        }
    };
    let Some(content) = content else {
        return Ok(Some("stale".into()));
    };
    if query::sha256(content.as_bytes()) != document.source_hash {
        return Ok(Some("stale".into()));
    }
    if (document.kind == "memory" || document.kind.starts_with("brain-"))
        && domain::indexed_secret_policy_reason(&[&content]).is_some()
    {
        return Ok(Some("secret-policy".into()));
    }
    if document.kind == "memory" {
        let metadata = match index::parse_memory_metadata(&content) {
            Ok(metadata) => metadata,
            Err(_) => return Ok(Some("invalid".into())),
        };
        if let Some(reason) = index::memory_temporal_reason(&metadata) {
            return Ok(Some(reason));
        }
    }
    if document.kind == "codebase" {
        let limit = config.value["codebase_map_max_drift"]
            .as_u64()
            .unwrap_or(25);
        match crate::retrieval::map_drift(context, &content)? {
            None => return Ok(Some("map-unverifiable".into())),
            Some(drift) if drift > limit => return Ok(Some("map-drift".into())),
            _ => {}
        }
    }
    Ok(None)
}
pub fn unscoped_reason(
    context: &RuntimeContext,
    config: &RuntimeConfig,
    item: &Candidate,
) -> Result<Option<String>> {
    let document = &item.document;
    if let Some(reason) = metadata_reason(config, document)? {
        return Ok(Some(reason));
    }
    if item
        .held_source_hash
        .as_deref()
        .is_some_and(|digest| digest != document.source_hash)
    {
        return Ok(Some("stale".into()));
    }
    let own = if crate::workspace::accelerator_owned_source(&document.path)
        || document.path.starts_with("project-brain/")
    {
        None
    } else {
        project_source_key(context, &document.path)?
    };
    if let Some(own) = own
        && crate::cursor_rule::source_git_ignored_paths(
            &context.roots.project,
            std::slice::from_ref(&own),
            Duration::from_secs(2),
        )?
        .contains(&own)
    {
        return Ok(Some("ignored".into()));
    }
    let image = item.image();
    let reason =
        if domain::indexed_secret_policy_reason(&[&image.title, &image.summary, &image.content])
            .is_some()
            || domain::indexed_secret_policy_reason(&[
                &document.path,
                &document.title,
                &item.snippet,
            ])
            .is_some()
        {
            Some("secret-policy".into())
        } else {
            None
        };
    point_reason(context, config, item, reason)
}
pub fn filter_in_scope(
    context: &RuntimeContext,
    config: &RuntimeConfig,
    connection: &Connection,
    policy: &mut CurrentSourcePolicy,
    mut candidates: Vec<Candidate>,
    custody: bool,
    mut trace: Option<&mut Trace>,
) -> Result<(Vec<Candidate>, Vec<Exclusion>)> {
    policy.prime(connection, context, &mut candidates)?;
    let mut selected = vec![];
    let mut excluded = vec![];
    for mut item in candidates {
        if custody {
            item.source_changed.clear();
        }
        let mut reason = metadata_reason(config, &item.document)?;
        if reason.is_none() {
            let policy_reason = policy.reason(&item, false)?;
            reason = point_reason(context, config, &item, policy_reason)?;
        }
        if reason.is_none() {
            let (changed, missing) =
                domain::source_changes(&context.roots, &item.document.source_fingerprints)?;
            if !missing.is_empty() {
                reason = Some("source-missing".into());
            } else if !changed.is_empty() {
                item.source_changed = changed;
                if !custody {
                    item.adjusted_score *= 0.5;
                }
            }
        }
        if let Some(trace) = trace.as_deref_mut() {
            trace.observe(
                &item,
                "runtime",
                reason
                    .as_deref()
                    .unwrap_or(if item.source_changed.is_empty() {
                        "admitted"
                    } else {
                        "source-changed-admitted"
                    }),
                &serde_json::json!({"score":item.adjusted_score}),
            );
        }
        if let Some(reason) = reason {
            excluded.push(Exclusion {
                path: item.document.path,
                reason,
            });
        } else {
            selected.push(item);
        }
    }
    Ok((selected, excluded))
}

pub fn metadata_reason(config: &RuntimeConfig, document: &Document) -> Result<Option<String>> {
    if !config
        .strings("allowed_privacy")?
        .contains(&document.privacy)
    {
        return Ok(Some("privacy".into()));
    }
    let owners = config.strings("owners")?;
    if document.privacy == "restricted"
        && !owners
            .iter()
            .any(|owner| owner == "*" || owner == &document.owner)
    {
        return Ok(Some("owner".into()));
    }
    if !config
        .strings("allowed_authority")?
        .contains(&document.authority)
    {
        return Ok(Some("authority".into()));
    }
    if document.kind == "brain-handoff" && document.lifecycle == "closed" {
        return Ok(Some("lifecycle".into()));
    }
    if let Some(kind) = document.kind.strip_prefix("brain-")
        && kind != "handoff"
        && (domain::lifecycle_states(kind).is_empty()
            || domain::is_terminal(kind, &document.lifecycle))
    {
        return Ok(Some("lifecycle".into()));
    }
    Ok(None)
}
