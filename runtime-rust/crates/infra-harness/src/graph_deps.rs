//! Pure static AST-backed source dependency extraction, declaration diffing, re-export propagation, and reference matching.
use crate::{
    graph_ast::{AstFileExtraction, AstRef, is_graph_manifest_path, parse_file_ir},
    graph_crates::CratePackageMap,
    graph_live::{
        CreatedEntity, ExtractedDecl, Language, clean_rel_key, detect_language, folder_of,
        is_ident_char,
    },
    graph_manifests::{ManifestIndex, join_rel},
};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, VecDeque, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::UNIX_EPOCH,
};

const PHP_EXTS: &[&str] = &["php", "phtml", "inc"];
const RUST_EXTS: &[&str] = &["rs"];
const TS_JS_EXTS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"];
const PYTHON_EXTS: &[&str] = &["py", "pyi"];
const GO_EXTS: &[&str] = &["go"];
const JAVA_EXTS: &[&str] = &["java"];
const KOTLIN_EXTS: &[&str] = &["kt", "kts"];
const CSHARP_EXTS: &[&str] = &["cs"];
const RUBY_EXTS: &[&str] = &["rb"];
const SWIFT_EXTS: &[&str] = &["swift"];
const SCALA_EXTS: &[&str] = &["scala", "sc"];
pub(crate) const MAX_REEXPORT_HOPS: usize = 8;
const MAX_REEXPORT_VISITS: usize = 64;
const MAX_CACHED_MANIFEST_ROOTS: usize = 16;
const MAX_MANIFEST_CACHE_BYTES: u64 = 512 * 1024;

#[derive(Clone)]
struct CachedResolutionBundles {
    stamp: String,
    known_len: usize,
    known_hash: u128,
    manifests: Arc<ManifestIndex>,
    crate_map: Arc<CratePackageMap>,
}

pub(crate) fn known_files_hash(known_files: &BTreeSet<String>) -> u128 {
    let mut hasher = xxhash_rust::xxh3::Xxh3::new();
    for path in known_files {
        hasher.update(&(path.len() as u64).to_le_bytes());
        hasher.update(path.as_bytes());
    }
    hasher.digest128()
}

fn manifest_cache() -> &'static Mutex<Vec<(PathBuf, CachedResolutionBundles)>> {
    static CACHE: OnceLock<Mutex<Vec<(PathBuf, CachedResolutionBundles)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

fn manifest_stamp_for_root(canon_root: &Path, known_files: &BTreeSet<String>) -> String {
    let mut parts: Vec<String> = crate::graph_ir::compute_root_manifest_stamp(canon_root)
        .split('|')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    for rel in known_files {
        let leaf = rel.rsplit('/').next().unwrap_or(rel);
        if is_graph_manifest_path(Path::new(leaf))
            && let Ok(meta) = fs::symlink_metadata(canon_root.join(rel))
            && !meta.file_type().is_symlink()
            && meta.is_file()
        {
            let nanos = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0u128, |d| d.as_nanos());
            parts.push(format!("{rel}:{}:{nanos}", meta.len()));
        }
    }
    parts.sort();
    parts.dedup();
    parts.join("|")
}

/// Returns cached `(ManifestIndex, CratePackageMap)` keyed by `(canonical_root, manifest_stamp, known_len, known_hash)`.
pub(crate) fn cached_manifests_for_root(
    root: &Path,
    known_files: &BTreeSet<String>,
) -> (Arc<ManifestIndex>, Arc<CratePackageMap>) {
    if root.as_os_str().is_empty() || !root.is_dir() {
        return (
            Arc::new(ManifestIndex::default()),
            Arc::new(CratePackageMap::default()),
        );
    }
    let canon_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let stamp = manifest_stamp_for_root(&canon_root, known_files);
    let known_len = known_files.len();
    let known_hash = known_files_hash(known_files);

    if let Ok(mut guard) = manifest_cache().lock()
        && let Some(pos) = guard.iter().position(|(p, entry)| {
            *p == canon_root
                && entry.stamp == stamp
                && entry.known_len == known_len
                && entry.known_hash == known_hash
        })
    {
        let item = guard.remove(pos);
        let res = (item.1.manifests.clone(), item.1.crate_map.clone());
        guard.push(item);
        return res;
    }

    let manifests = Arc::new(ManifestIndex::from_root(&canon_root));
    let known_vec: Vec<String> = known_files.iter().cloned().collect();
    let crate_map = Arc::new(CratePackageMap::from_reader(&known_vec, &|rel| {
        let path = canon_root.join(rel);
        let meta = fs::symlink_metadata(&path).ok()?;
        if meta.file_type().is_symlink() || !meta.is_file() || meta.len() > MAX_MANIFEST_CACHE_BYTES
        {
            return None;
        }
        let mut buf = Vec::new();
        let file = fs::File::open(&path).ok()?;
        file.take(MAX_MANIFEST_CACHE_BYTES + 1)
            .read_to_end(&mut buf)
            .ok()?;
        if buf.len() as u64 > MAX_MANIFEST_CACHE_BYTES {
            return None;
        }
        String::from_utf8(buf).ok()
    }));

    if let Ok(mut guard) = manifest_cache().lock() {
        guard.retain(|(p, _)| *p != canon_root);
        if guard.len() >= MAX_CACHED_MANIFEST_ROOTS {
            guard.remove(0);
        }
        guard.push((
            canon_root,
            CachedResolutionBundles {
                stamp,
                known_len,
                known_hash,
                manifests: manifests.clone(),
                crate_map: crate_map.clone(),
            },
        ));
    }

    (manifests, crate_map)
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// Maps a normalized visibility label to an ordinal exposure rank (`4` = public/export, `0` = private).
pub(crate) fn visibility_rank(vis: Option<&str>) -> u8 {
    let Some(raw) = vis.map(str::trim).filter(|s| !s.is_empty()) else {
        return 4;
    };
    let lower = raw.to_ascii_lowercase();
    if lower.starts_with("pub(in ") {
        return 2;
    }
    match lower.as_str() {
        "public" | "pub" | "export" | "public(set)" => 4,
        "crate"
        | "pub(crate)"
        | "pub(super)"
        | "internal"
        | "protected(set)"
        | "public protected(set)"
        | "package" => 2,
        "protected"
        | "private(set)"
        | "public private(set)"
        | "protected private(set)"
        | "protected protected(set)" => 1,
        "private" | "#private" | "pub(self)" => 0,
        _ if lower.contains("private(set)") => 1,
        _ if lower.contains("protected(set)") => 2,
        _ if lower.contains("private") => 0,
        _ if lower.contains("protected") => 1,
        _ if lower.contains("crate") || lower.contains("internal") || lower.contains("package") => {
            2
        }
        _ => 4,
    }
}

/// Returns `true` when `after` visibility is strictly narrower than `before` visibility.
pub(crate) fn is_visibility_narrowed(before: Option<&str>, after: Option<&str>) -> bool {
    visibility_rank(after) < visibility_rank(before)
}

/// Symbol-level declaration delta between two revisions of a source file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EntityChange {
    pub id: String,
    pub kind: String,
    pub change: String, // "created" | "modified" | "deleted"
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub signature_changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_visibility: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub visibility_changed: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub visibility_narrowed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<String>,
}

/// Member-level usage reference recorded on a dependency edge (`calls`, `reads`, `writes`, `const`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DepMemberRef {
    pub name: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub access: String,
    /// The first line the member is reached in this way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// Every line it is reached in this way, in their order, where there is
    /// more than one: a member read in two methods is one reference told
    /// with both places.
    #[serde(default, skip_serializing_if = "at_most_one")]
    pub lines: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_risk: Option<String>,
}

fn at_most_one(lines: &[u32]) -> bool {
    lines.len() <= 1
}

impl DepMemberRef {
    fn basic(
        name: impl Into<String>,
        kind: &str,
        parent: Option<String>,
        access: &str,
        line: u32,
    ) -> Self {
        Self {
            name: name.into(),
            kind: kind.to_owned(),
            parent,
            access: access.to_owned(),
            line: Some(line),
            lines: vec![line],
            visibility: None,
            previous_visibility: None,
            change: None,
            modifiers: Vec::new(),
            access_risk: None,
        }
    }
}

/// Declared symbol/member summary extracted from a target file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DeclaredMember {
    pub name: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<String>,
    pub change: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub signature_changed: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub visibility_narrowed: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub used_by_caller: bool,
}

/// Extracted dependency reference from one source file to another file or external module.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DepRef {
    pub target: String,
    pub resolved: bool,
    #[serde(default)]
    pub exists: bool,
    /// `imports`, `calls`, `extends`, `implements`, `writes`, `reads` or `uses`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub member_refs: Vec<DepMemberRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_members: Vec<DeclaredMember>,
}

/// Before-vs-after dependency diff for a single file, grouped by canonical `(target, kind)`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DepDiff {
    pub added: Vec<DepRef>,
    pub removed: Vec<DepRef>,
    pub unchanged: Vec<DepRef>,
}

/// Extracts all declarations from a source file into `DeclaredMember` summaries via tree-sitter AST.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn extract_declared_members(rel_path: &str, source: &str) -> Vec<DeclaredMember> {
    let rel = clean_rel_key(rel_path);
    if rel.is_empty() || source.trim().is_empty() {
        return Vec::new();
    }
    crate::graph_ast::extract_declared_members_ast(&rel, source)
}

/// Computes created, modified, and deleted declarations between `before` and `after`.
#[cfg(test)]
pub(crate) fn diff_entities(
    rel_path: &str,
    before: Option<&str>,
    after: Option<&str>,
) -> Vec<EntityChange> {
    let rel = clean_rel_key(rel_path);
    if rel.is_empty() || detect_language(&rel).is_none() {
        return Vec::new();
    }
    let before_ir = before.and_then(|src| parse_file_ir(&rel, src));
    let after_ir = after.and_then(|src| parse_file_ir(&rel, src));
    diff_entities_from_ir(&rel, before_ir.as_ref(), after_ir.as_ref())
}

/// Computes created, modified, and deleted declarations from pre-parsed `AstFileExtraction` records.
pub(crate) fn diff_entities_from_ir(
    rel_path: &str,
    before_ir: Option<&AstFileExtraction>,
    after_ir: Option<&AstFileExtraction>,
) -> Vec<EntityChange> {
    let rel = clean_rel_key(rel_path);
    if rel.is_empty() || detect_language(&rel).is_none() {
        return Vec::new();
    }

    let before_decls: &[ExtractedDecl] = before_ir.map_or(&[], |ir| ir.declarations.as_slice());
    let after_decls: &[ExtractedDecl] = after_ir.map_or(&[], |ir| ir.declarations.as_slice());

    type DeclKey = (String, Option<String>, String);
    let mut before_by_key: BTreeMap<DeclKey, Vec<&ExtractedDecl>> = BTreeMap::new();
    for decl in before_decls {
        let key = (
            decl.entity.kind.clone(),
            decl.entity.parent.clone(),
            decl.entity.name.clone(),
        );
        before_by_key.entry(key).or_default().push(decl);
    }

    let mut used_before: BTreeMap<DeclKey, Vec<bool>> = before_by_key
        .iter()
        .map(|(k, v)| (k.clone(), vec![false; v.len()]))
        .collect();
    let mut unmatched_after: Vec<(DeclKey, &ExtractedDecl)> = Vec::new();

    // Match exact unchanged declarations first so overload insertions/reorders do not produce noise.
    for decl in after_decls {
        let key = (
            decl.entity.kind.clone(),
            decl.entity.parent.clone(),
            decl.entity.name.clone(),
        );
        if let Some(prev_list) = before_by_key.get(&key)
            && let Some(used) = used_before.get_mut(&key)
            && let Some(pos) = prev_list.iter().enumerate().position(|(idx, prev)| {
                !used[idx]
                    && prev.params_signature == decl.params_signature
                    && prev.body_fingerprint == decl.body_fingerprint
                    && prev.visibility == decl.visibility
                    && prev.modifiers == decl.modifiers
            })
        {
            used[pos] = true;
        } else {
            unmatched_after.push((key, decl));
        }
    }

    let mut out = Vec::new();
    let mut seen_ids = BTreeSet::new();

    for (key, decl) in unmatched_after {
        if let Some(prev_list) = before_by_key.get(&key)
            && let Some(used) = used_before.get_mut(&key)
            && let Some(pos) = used.iter().position(|flag| !*flag)
        {
            used[pos] = true;
            let prev = prev_list[pos];
            let sig_changed = prev.params_signature != decl.params_signature;
            let vis_changed = prev.visibility != decl.visibility;
            let vis_narrowed =
                is_visibility_narrowed(prev.visibility.as_deref(), decl.visibility.as_deref());
            out.push(make_entity_change(
                "modified",
                &decl.entity,
                sig_changed,
                decl.visibility.clone(),
                prev.visibility.clone(),
                vis_changed,
                vis_narrowed,
                decl.modifiers.clone(),
                &mut seen_ids,
            ));
        } else {
            out.push(make_entity_change(
                "created",
                &decl.entity,
                false,
                decl.visibility.clone(),
                None,
                false,
                false,
                decl.modifiers.clone(),
                &mut seen_ids,
            ));
        }
    }

    for (key, prev_list) in before_by_key {
        let Some(used) = used_before.get(&key) else {
            continue;
        };
        for (idx, prev) in prev_list.into_iter().enumerate() {
            if !used[idx] {
                out.push(make_entity_change(
                    "deleted",
                    &prev.entity,
                    false,
                    prev.visibility.clone(),
                    None,
                    false,
                    false,
                    prev.modifiers.clone(),
                    &mut seen_ids,
                ));
            }
        }
    }

    out.sort_by(|a, b| {
        a.line
            .unwrap_or(u32::MAX)
            .cmp(&b.line.unwrap_or(u32::MAX))
            .then_with(|| change_rank(&a.change).cmp(&change_rank(&b.change)))
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

fn change_rank(change: &str) -> u8 {
    match change {
        "created" => 0,
        "modified" => 1,
        "deleted" => 2,
        _ => 3,
    }
}

#[allow(clippy::too_many_arguments)]
fn make_entity_change(
    change: &str,
    entity: &CreatedEntity,
    signature_changed: bool,
    visibility: Option<String>,
    previous_visibility: Option<String>,
    visibility_changed: bool,
    visibility_narrowed: bool,
    modifiers: Vec<String>,
    seen_ids: &mut BTreeSet<String>,
) -> EntityChange {
    let base_id = if matches!(
        entity.kind.as_str(),
        "method" | "property" | "field" | "const"
    ) && let Some(owner) = entity
        .parent
        .as_deref()
        .filter(|s| !s.is_empty() && *s != entity.path)
    {
        format!(
            "decl:{}:{}:{owner}::{}",
            entity.kind, entity.path, entity.name
        )
    } else {
        format!("decl:{}:{}:{}", entity.kind, entity.path, entity.name)
    };
    let id = if seen_ids.insert(base_id.clone()) {
        base_id
    } else {
        let disambiguated = format!("{base_id}@{}", entity.line.unwrap_or(0));
        seen_ids.insert(disambiguated.clone());
        disambiguated
    };
    EntityChange {
        id,
        kind: entity.kind.clone(),
        change: change.to_owned(),
        name: entity.name.clone(),
        parent: entity.parent.clone(),
        path: entity.path.clone(),
        line: entity.line,
        signature_changed,
        visibility,
        previous_visibility,
        visibility_changed,
        visibility_narrowed,
        modifiers,
    }
}

/// Single re-export statement (`pub use ...;` in Rust or `export { ... } from '...'` in TS/JS).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReexportEntry {
    pub source_module_path: String,
    pub exported_symbol: String,
    pub original_symbol: String,
    pub is_wildcard: bool,
}

/// Workspace re-export graph supporting cycle-safe multi-hop symbol propagation.
#[derive(Clone, Debug, Default)]
pub(crate) struct ReexportIndex {
    pub by_file: BTreeMap<String, Vec<ReexportEntry>>,
    pub declared_by_file: BTreeMap<String, BTreeSet<String>>,
}

impl ReexportIndex {
    /// Builds a `ReexportIndex` by parsing `(rel_path, source)` pairs via AST.
    #[allow(dead_code)]
    pub(crate) fn from_sources(sources: &[(&str, &str)]) -> Self {
        let extractions: Vec<AstFileExtraction> = sources
            .iter()
            .filter_map(|(path, src)| parse_file_ir(path, src))
            .collect();
        Self::from_extractions(&extractions)
    }

    /// Builds a `ReexportIndex` from pre-parsed `AstFileExtraction` records.
    #[allow(dead_code)]
    pub(crate) fn from_extractions(extractions: &[AstFileExtraction]) -> Self {
        let mut index = Self::default();
        for ir in extractions {
            let rel = clean_rel_key(&ir.rel_path);
            if rel.is_empty() {
                continue;
            }
            let decls = index.declared_by_file.entry(rel.clone()).or_default();
            for d in &ir.declarations {
                if d.entity.parent.as_deref() == Some(rel.as_str())
                    || !matches!(d.entity.kind.as_str(), "method" | "field" | "property")
                {
                    decls.insert(d.entity.name.clone());
                }
            }
            for imp in &ir.imports {
                if !imp.is_reexport {
                    continue;
                }
                if imp.is_wildcard {
                    index
                        .by_file
                        .entry(rel.clone())
                        .or_default()
                        .push(ReexportEntry {
                            source_module_path: imp.path.clone(),
                            exported_symbol: "*".to_owned(),
                            original_symbol: "*".to_owned(),
                            is_wildcard: true,
                        });
                } else {
                    for orig in &imp.symbols {
                        let exported = imp.alias.clone().unwrap_or_else(|| orig.clone());
                        index
                            .by_file
                            .entry(rel.clone())
                            .or_default()
                            .push(ReexportEntry {
                                source_module_path: imp.path.clone(),
                                exported_symbol: exported,
                                original_symbol: orig.clone(),
                                is_wildcard: false,
                            });
                    }
                    if imp.symbols.is_empty()
                        && let Some(ref alias) = imp.alias
                    {
                        index
                            .by_file
                            .entry(rel.clone())
                            .or_default()
                            .push(ReexportEntry {
                                source_module_path: imp.path.clone(),
                                exported_symbol: alias.clone(),
                                original_symbol: alias.clone(),
                                is_wildcard: false,
                            });
                    }
                }
            }
        }
        index
    }

    /// Resolves `(facade_file, symbol)` across bounded cycle-safe re-export hops to `(defining_file, original_symbol)`.
    pub(crate) fn resolve_reexport_target(
        &self,
        facade_file: &str,
        symbol: &str,
        known_files: &BTreeSet<String>,
        manifests_opt: Option<&ManifestIndex>,
        crate_map_opt: Option<&CratePackageMap>,
    ) -> Option<(String, String)> {
        let start_file = clean_rel_key(facade_file);
        if start_file.is_empty() || symbol.is_empty() || symbol == "*" {
            return None;
        }
        // If the facade file itself defines the symbol, do not redirect away from it.
        if self
            .declared_by_file
            .get(&start_file)
            .is_some_and(|decls| decls.contains(symbol))
        {
            return None;
        }
        let lookup = KnownLookup::new(known_files);
        let mut queue = VecDeque::new();
        queue.push_back((start_file.clone(), symbol.to_owned(), 0usize, false));
        let mut visited: BTreeSet<(String, String)> = BTreeSet::new();
        let mut terminal_named: Option<(String, String)> = None;
        let mut named_fallback: Option<(String, String)> = None;
        let mut wildcard_candidates: Vec<(String, String)> = Vec::new();
        let mut visits = 0usize;

        while let Some((cur_file, cur_sym, hops, via_wildcard)) = queue.pop_front() {
            if visits >= MAX_REEXPORT_VISITS {
                break;
            }
            visits += 1;
            if !visited.insert((cur_file.clone(), cur_sym.clone())) {
                continue;
            }
            if hops > 0
                && self
                    .declared_by_file
                    .get(&cur_file)
                    .is_some_and(|decls| decls.contains(&cur_sym))
            {
                return Some((cur_file, cur_sym));
            }
            if hops >= MAX_REEXPORT_HOPS {
                if hops > 0 && cur_file != start_file && !via_wildcard {
                    if terminal_named.is_none() {
                        terminal_named = Some((cur_file.clone(), cur_sym.clone()));
                    }
                    if named_fallback.is_none() {
                        named_fallback = Some((cur_file, cur_sym));
                    }
                }
                continue;
            }
            let Some(entries) = self.by_file.get(&cur_file) else {
                if hops > 0 && cur_file != start_file {
                    if !via_wildcard {
                        if terminal_named.is_none() {
                            terminal_named = Some((cur_file.clone(), cur_sym.clone()));
                        }
                        if named_fallback.is_none() {
                            named_fallback = Some((cur_file, cur_sym));
                        }
                    } else {
                        wildcard_candidates.push((cur_file, cur_sym));
                    }
                }
                continue;
            };

            let mut matched_named = false;
            let mut pushed_any = false;
            for entry in entries
                .iter()
                .filter(|e| !e.is_wildcard && e.exported_symbol == cur_sym)
            {
                if let Some((next_file, next_sym)) = resolve_reexport_step(
                    &cur_file,
                    entry,
                    &cur_sym,
                    &lookup,
                    manifests_opt,
                    crate_map_opt,
                ) {
                    matched_named = true;
                    pushed_any = true;
                    if named_fallback.is_none() && next_file != start_file {
                        named_fallback = Some((next_file.clone(), next_sym.clone()));
                    }
                    queue.push_back((next_file, next_sym, hops + 1, false));
                }
            }

            if !matched_named {
                for entry in entries.iter().filter(|e| e.is_wildcard) {
                    if let Some((next_file, next_sym)) = resolve_reexport_step(
                        &cur_file,
                        entry,
                        &cur_sym,
                        &lookup,
                        manifests_opt,
                        crate_map_opt,
                    ) {
                        pushed_any = true;
                        queue.push_back((next_file, next_sym, hops + 1, true));
                    }
                }
            }

            if !pushed_any && hops > 0 && cur_file != start_file && !via_wildcard {
                if terminal_named.is_none() {
                    terminal_named = Some((cur_file.clone(), cur_sym.clone()));
                }
                if named_fallback.is_none() {
                    named_fallback = Some((cur_file, cur_sym));
                }
            }
        }

        terminal_named.or(named_fallback).or_else(|| {
            if self.declared_by_file.is_empty() && wildcard_candidates.len() == 1 {
                wildcard_candidates.into_iter().next()
            } else {
                None
            }
        })
    }
}

/// Extracts imports, inheritance, includes, and symbol references from `source`, resolving against `known_files`.
#[cfg(test)]
pub(crate) fn extract_file_deps(
    rel_path: &str,
    source: &str,
    known_files: &BTreeSet<String>,
) -> Vec<DepRef> {
    extract_file_deps_with_manifests(rel_path, source, known_files, None, None)
}

/// Extracts dependencies using tree-sitter AST, optional workspace manifests (`composer.json`,
/// `tsconfig.json`, `package.json`, `pyproject.toml`, `go.mod`), and optional re-export propagation.
#[cfg(test)]
pub(crate) fn extract_file_deps_with_manifests(
    rel_path: &str,
    source: &str,
    known_files: &BTreeSet<String>,
    manifests_opt: Option<&ManifestIndex>,
    reexports_opt: Option<&ReexportIndex>,
) -> Vec<DepRef> {
    extract_file_deps_with_context(
        rel_path,
        source,
        known_files,
        manifests_opt,
        reexports_opt,
        None,
    )
}

/// Full dependency extractor also accepting an optional Cargo `CratePackageMap`.
pub(crate) fn extract_file_deps_with_context(
    rel_path: &str,
    source: &str,
    known_files: &BTreeSet<String>,
    manifests_opt: Option<&ManifestIndex>,
    reexports_opt: Option<&ReexportIndex>,
    crate_map_opt: Option<&CratePackageMap>,
) -> Vec<DepRef> {
    let rel = clean_rel_key(rel_path);
    if rel.is_empty() || source.trim().is_empty() {
        return Vec::new();
    }
    let Some(ir) = parse_file_ir(&rel, source) else {
        return Vec::new();
    };
    extract_file_deps_from_ir(
        &rel,
        &ir,
        known_files,
        manifests_opt,
        reexports_opt,
        crate_map_opt,
    )
}

/// Resolves dependencies directly from an already-parsed `AstFileExtraction` without re-parsing.
pub(crate) fn extract_file_deps_from_ir(
    rel_path: &str,
    ir: &AstFileExtraction,
    known_files: &BTreeSet<String>,
    manifests_opt: Option<&ManifestIndex>,
    reexports_opt: Option<&ReexportIndex>,
    crate_map_opt: Option<&CratePackageMap>,
) -> Vec<DepRef> {
    extract_file_deps_from_ir_inner(
        rel_path,
        ir,
        known_files,
        manifests_opt,
        reexports_opt,
        crate_map_opt,
        false,
    )
}

/// Resolves cross-file and intra-file symbol-to-symbol dependencies directly from an
/// already-parsed `AstFileExtraction` (for `GraphIrStore` / `lod=symbol`).
pub(crate) fn extract_file_deps_with_intra_from_ir(
    rel_path: &str,
    ir: &AstFileExtraction,
    known_files: &BTreeSet<String>,
    manifests_opt: Option<&ManifestIndex>,
    reexports_opt: Option<&ReexportIndex>,
    crate_map_opt: Option<&CratePackageMap>,
) -> Vec<DepRef> {
    extract_file_deps_from_ir_inner(
        rel_path,
        ir,
        known_files,
        manifests_opt,
        reexports_opt,
        crate_map_opt,
        true,
    )
}

#[cfg(test)]
pub(crate) fn extract_file_deps_with_intra(
    rel_path: &str,
    source: &str,
    known_files: &BTreeSet<String>,
) -> Vec<DepRef> {
    let rel = clean_rel_key(rel_path);
    if rel.is_empty() {
        return Vec::new();
    }
    let Some(ir) = parse_file_ir(&rel, source) else {
        return Vec::new();
    };
    extract_file_deps_with_intra_from_ir(&rel, &ir, known_files, None, None, None)
}

#[derive(Clone)]
struct IntraLocalSym {
    name: String,
    kind: String,
    parent: Option<String>,
    line: u32,
}

fn strip_generic_and_ns(raw: &str) -> &str {
    let trimmed = raw.trim();
    let without_gen = trimmed.split('<').next().unwrap_or(trimmed).trim();
    let tail = without_gen
        .rsplit(['\\', ':'])
        .next()
        .unwrap_or(without_gen)
        .trim();
    tail.strip_prefix('$').unwrap_or(tail)
}

fn find_enclosing_container(line: u32, containers: &[(String, u32, u32)]) -> Option<String> {
    containers
        .iter()
        .filter(|(_, start, end)| line >= *start && (*end == 0 || line <= *end))
        .min_by_key(|(_, start, end)| {
            if *end >= *start {
                *end - *start
            } else {
                u32::MAX
            }
        })
        .map(|(name, _, _)| name.clone())
}

fn record_intra_sym(
    by_parent_member: &mut BTreeMap<(String, String), IntraLocalSym>,
    by_parent_member_ci: &mut BTreeMap<(String, String), IntraLocalSym>,
    top_by_name: &mut BTreeMap<String, IntraLocalSym>,
    top_by_name_ci: &mut BTreeMap<String, IntraLocalSym>,
    sym: IntraLocalSym,
) {
    let bare = sym.name.strip_prefix('$').unwrap_or(&sym.name).to_owned();
    let bare_lower = bare.to_ascii_lowercase();
    if let Some(ref parent) = sym.parent {
        let parent_clean = strip_generic_and_ns(parent).to_owned();
        if !parent_clean.is_empty() {
            by_parent_member
                .entry((parent_clean.clone(), bare.clone()))
                .or_insert_with(|| sym.clone());
            by_parent_member_ci
                .entry((parent_clean.to_ascii_lowercase(), bare_lower))
                .or_insert(sym);
            return;
        }
    }
    top_by_name.entry(bare).or_insert_with(|| sym.clone());
    top_by_name_ci.entry(bare_lower).or_insert(sym);
}

fn lookup_intra_member<'a>(
    container: &str,
    member_raw: &str,
    by_parent_member: &'a BTreeMap<(String, String), IntraLocalSym>,
    by_parent_member_ci: &'a BTreeMap<(String, String), IntraLocalSym>,
) -> Option<&'a IntraLocalSym> {
    let container_clean = strip_generic_and_ns(container);
    let mem_clean = norm_member_name(member_raw);
    let key = (container_clean.to_owned(), mem_clean.to_owned());
    by_parent_member.get(&key).or_else(|| {
        by_parent_member_ci.get(&(
            container_clean.to_ascii_lowercase(),
            mem_clean.to_ascii_lowercase(),
        ))
    })
}

fn resolve_intra_file_ir_refs(rel_path: &str, ir: &AstFileExtraction, acc: &mut DepAccumulator) {
    let mut by_parent_member: BTreeMap<(String, String), IntraLocalSym> = BTreeMap::new();
    let mut by_parent_member_ci: BTreeMap<(String, String), IntraLocalSym> = BTreeMap::new();
    let mut top_by_name: BTreeMap<String, IntraLocalSym> = BTreeMap::new();
    let mut top_by_name_ci: BTreeMap<String, IntraLocalSym> = BTreeMap::new();
    let mut containers: Vec<(String, u32, u32)> = Vec::new();

    for decl in &ir.declarations {
        let kind_lower = decl.entity.kind.to_ascii_lowercase();
        let is_container = matches!(
            kind_lower.as_str(),
            "class"
                | "struct"
                | "enum"
                | "trait"
                | "interface"
                | "impl"
                | "module"
                | "object"
                | "protocol"
        );
        if is_container && decl.start_line > 0 {
            let short_name = strip_generic_and_ns(&decl.entity.name).to_owned();
            if !short_name.is_empty() {
                containers.push((short_name, decl.start_line, decl.end_line));
            }
        }
        record_intra_sym(
            &mut by_parent_member,
            &mut by_parent_member_ci,
            &mut top_by_name,
            &mut top_by_name_ci,
            IntraLocalSym {
                name: decl.entity.name.clone(),
                kind: decl.entity.kind.clone(),
                parent: decl.entity.parent.clone(),
                line: decl.entity.line.unwrap_or(decl.start_line),
            },
        );
    }
    for dm in &ir.declared_members {
        record_intra_sym(
            &mut by_parent_member,
            &mut by_parent_member_ci,
            &mut top_by_name,
            &mut top_by_name_ci,
            IntraLocalSym {
                name: dm.name.clone(),
                kind: dm.kind.clone(),
                parent: dm.parent.clone(),
                line: dm.line.unwrap_or(0),
            },
        );
    }

    // Track names explicitly imported from external files so local fallback doesn't shadow them.
    let mut imported_locals: BTreeSet<String> = BTreeSet::new();
    for imp in &ir.imports {
        if let Some(ref alias) = imp.alias {
            imported_locals.insert(alias.to_ascii_lowercase());
        }
        for sym in &imp.symbols {
            let s = sym.trim();
            if !s.is_empty() {
                imported_locals.insert(s.to_ascii_lowercase());
            }
        }
    }

    let is_php = ir.language.eq_ignore_ascii_case("php");

    for inh in &ir.inheritance {
        let parent_clean = strip_generic_and_ns(&inh.parent);
        if parent_clean.is_empty() || imported_locals.contains(&parent_clean.to_ascii_lowercase()) {
            continue;
        }
        if let Some(sym) = top_by_name
            .get(parent_clean)
            .or_else(|| top_by_name_ci.get(&parent_clean.to_ascii_lowercase()))
        {
            let m = DepMemberRef::basic(
                &sym.name,
                &sym.kind,
                sym.parent.clone(),
                &inh.kind,
                inh.line,
            );
            acc.add_resolved_members(
                rel_path.to_owned(),
                &inh.kind,
                &[&sym.name],
                &[m],
                inh.line,
                &inh.evidence,
            );
        }
    }

    for r in &ir.calls_and_refs {
        let raw_target = r
            .owner_type
            .as_deref()
            .filter(|_| r.target_name.starts_with('$'))
            .unwrap_or(&r.target_name)
            .trim();
        let is_self_receiver = matches!(raw_target, "self" | "Self" | "static" | "this" | "$this");
        let container_from_self = if is_self_receiver {
            r.enclosing_symbol
                .as_deref()
                .and_then(|enc| {
                    enc.split_once("::")
                        .or_else(|| enc.split_once('.'))
                        .map(|(head, _)| strip_generic_and_ns(head).to_owned())
                })
                .filter(|c| !c.is_empty())
                .or_else(|| find_enclosing_container(r.line, &containers))
        } else {
            None
        };

        if let Some(container) = container_from_self {
            if let Some(ref mem) = r.member {
                if let Some(target_sym) =
                    lookup_intra_member(&container, mem, &by_parent_member, &by_parent_member_ci)
                {
                    let (dep_kind, members) = ref_dep_kind_and_member(
                        r,
                        &container,
                        if target_sym.kind.is_empty() {
                            "field"
                        } else {
                            &target_sym.kind
                        },
                        Some(container.clone()),
                        is_php,
                    );
                    let mut final_members = members;
                    for m in &mut final_members {
                        m.kind = target_sym.kind.clone();
                        m.parent = Some(container.clone());
                    }
                    if !final_members.is_empty() {
                        acc.add_resolved_members(
                            rel_path.to_owned(),
                            dep_kind,
                            &[&container, &target_sym.name],
                            &final_members,
                            r.line,
                            &r.evidence,
                        );
                    }
                }
            } else if let Some(target_sym) = top_by_name
                .get(&container)
                .or_else(|| top_by_name_ci.get(&container.to_ascii_lowercase()))
            {
                let (dep_kind, _) =
                    ref_dep_kind_and_member(r, &target_sym.name, "class", None, is_php);
                let access = if dep_kind == "calls" {
                    "call"
                } else {
                    &r.access
                };
                let m =
                    DepMemberRef::basic(&target_sym.name, &target_sym.kind, None, access, r.line);
                acc.add_resolved_members(
                    rel_path.to_owned(),
                    dep_kind,
                    &[&target_sym.name],
                    &[m],
                    r.line,
                    &r.evidence,
                );
            }
            continue;
        }

        if raw_target.starts_with('$') || is_self_receiver {
            continue;
        }
        let target_clean = strip_generic_and_ns(raw_target);
        if target_clean.is_empty() {
            continue;
        }
        // Only skip external imports when the imported symbol is not declared in this file.
        let is_declared_top = top_by_name.contains_key(target_clean)
            || top_by_name_ci.contains_key(&target_clean.to_ascii_lowercase());
        if !is_declared_top && imported_locals.contains(&target_clean.to_ascii_lowercase()) {
            continue;
        }

        if let Some(ref mem) = r.member {
            if let Some(target_sym) =
                lookup_intra_member(target_clean, mem, &by_parent_member, &by_parent_member_ci)
            {
                let container = target_sym
                    .parent
                    .clone()
                    .unwrap_or_else(|| target_clean.to_owned());
                let (dep_kind, mut members) = ref_dep_kind_and_member(
                    r,
                    &container,
                    if target_sym.kind.is_empty() {
                        "field"
                    } else {
                        &target_sym.kind
                    },
                    Some(container.clone()),
                    is_php,
                );
                for m in &mut members {
                    m.kind = target_sym.kind.clone();
                    m.parent = Some(container.clone());
                }
                if !members.is_empty() {
                    acc.add_resolved_members(
                        rel_path.to_owned(),
                        dep_kind,
                        &[&container, &target_sym.name],
                        &members,
                        r.line,
                        &r.evidence,
                    );
                }
            }
            continue;
        }

        if let Some(target_sym) = top_by_name
            .get(target_clean)
            .or_else(|| top_by_name_ci.get(&target_clean.to_ascii_lowercase()))
        {
            if r.line == target_sym.line && r.context != "call" {
                continue;
            }
            let (dep_kind, _) =
                ref_dep_kind_and_member(r, &target_sym.name, &target_sym.kind, None, is_php);
            let access = match dep_kind {
                "calls" => "call",
                "writes" => "write",
                "reads" => "read",
                _ => "type",
            };
            let m = DepMemberRef::basic(
                &target_sym.name,
                &target_sym.kind,
                target_sym.parent.clone(),
                access,
                r.line,
            );
            acc.add_resolved_members(
                rel_path.to_owned(),
                dep_kind,
                &[&target_sym.name],
                &[m],
                r.line,
                &r.evidence,
            );
        }
    }
}

fn extract_file_deps_from_ir_inner(
    rel_path: &str,
    ir: &AstFileExtraction,
    known_files: &BTreeSet<String>,
    manifests_opt: Option<&ManifestIndex>,
    reexports_opt: Option<&ReexportIndex>,
    crate_map_opt: Option<&CratePackageMap>,
    include_self: bool,
) -> Vec<DepRef> {
    let rel = clean_rel_key(rel_path);
    if rel.is_empty() {
        return Vec::new();
    }
    let Some(lang) = detect_language(&rel) else {
        return Vec::new();
    };
    let lookup = KnownLookup::new(known_files);
    let mut acc = DepAccumulator::new_with_self(&rel, known_files, include_self);

    match lang {
        Language::Php => resolve_php_ir(&rel, ir, &lookup, manifests_opt, reexports_opt, &mut acc),
        Language::Rust => {
            resolve_rust_ir(&rel, ir, &lookup, reexports_opt, crate_map_opt, &mut acc)
        }
        Language::TsJs => {
            resolve_ts_js_ir(&rel, ir, &lookup, manifests_opt, reexports_opt, &mut acc)
        }
        Language::Python => resolve_python_ir(&rel, ir, &lookup, manifests_opt, &mut acc),
        Language::Go => resolve_go_ir(&rel, ir, &lookup, manifests_opt, &mut acc),
        Language::Java | Language::Kotlin | Language::Scala => {
            resolve_jvm_ir(&rel, ir, lang, &lookup, &mut acc)
        }
        Language::CSharp => resolve_csharp_ir(&rel, ir, &lookup, &mut acc),
        Language::Ruby => resolve_ruby_ir(&rel, ir, &lookup, &mut acc),
        Language::Swift => resolve_swift_ir(&rel, ir, &lookup, &mut acc),
    }

    if include_self {
        resolve_intra_file_ir_refs(&rel, ir, &mut acc);
    }

    acc.into_sorted_vec()
}

/// Compares dependencies before and after an edit using optional manifest/reexport/crate indices.
#[cfg(test)]
pub(crate) fn diff_file_deps(
    rel_path: &str,
    before: Option<&str>,
    after: Option<&str>,
    known_files: &BTreeSet<String>,
) -> DepDiff {
    diff_file_deps_with_context(rel_path, before, after, known_files, None, None, None)
}

/// Context-aware version of `diff_file_deps` sharing prebuilt `ManifestIndex`/`ReexportIndex`/`CratePackageMap`.
#[cfg(test)]
pub(crate) fn diff_file_deps_with_context(
    rel_path: &str,
    before: Option<&str>,
    after: Option<&str>,
    known_files: &BTreeSet<String>,
    manifests_opt: Option<&ManifestIndex>,
    reexports_opt: Option<&ReexportIndex>,
    crate_map_opt: Option<&CratePackageMap>,
) -> DepDiff {
    let rel = clean_rel_key(rel_path);
    if rel.is_empty() {
        return DepDiff::default();
    }
    let before_ir = before
        .filter(|s| !s.trim().is_empty())
        .and_then(|src| parse_file_ir(&rel, src));
    let after_ir = after
        .filter(|s| !s.trim().is_empty())
        .and_then(|src| parse_file_ir(&rel, src));
    diff_file_deps_from_ir(
        &rel,
        before_ir.as_ref(),
        after_ir.as_ref(),
        known_files,
        manifests_opt,
        reexports_opt,
        crate_map_opt,
    )
}

/// Computes a file's dependency diff from pre-parsed `AstFileExtraction` records without re-parsing.
pub(crate) fn diff_file_deps_from_ir(
    rel_path: &str,
    before_ir: Option<&AstFileExtraction>,
    after_ir: Option<&AstFileExtraction>,
    known_files: &BTreeSet<String>,
    manifests_opt: Option<&ManifestIndex>,
    reexports_opt: Option<&ReexportIndex>,
    crate_map_opt: Option<&CratePackageMap>,
) -> DepDiff {
    let before_deps = before_ir
        .map(|ir| {
            extract_file_deps_from_ir(
                rel_path,
                ir,
                known_files,
                manifests_opt,
                reexports_opt,
                crate_map_opt,
            )
        })
        .unwrap_or_default();
    let after_deps = after_ir
        .map(|ir| {
            extract_file_deps_from_ir(
                rel_path,
                ir,
                known_files,
                manifests_opt,
                reexports_opt,
                crate_map_opt,
            )
        })
        .unwrap_or_default();

    let mut before_map: BTreeMap<(String, String), DepRef> = BTreeMap::new();
    for dep in before_deps {
        merge_dep_into_map(&mut before_map, dep);
    }
    let mut after_map: BTreeMap<(String, String), DepRef> = BTreeMap::new();
    for dep in after_deps {
        merge_dep_into_map(&mut after_map, dep);
    }

    let mut diff = DepDiff::default();
    for (key, after_dep) in &after_map {
        if let Some(before_dep) = before_map.get(key) {
            let mut merged = after_dep.clone();
            for sym in &before_dep.symbols {
                if !merged.symbols.contains(sym) {
                    merged.symbols.push(sym.clone());
                }
            }
            if merged.line.is_none() {
                merged.line = before_dep.line;
            }
            if merged.evidence.is_none() {
                merged.evidence = before_dep.evidence.clone();
            }
            diff.unchanged.push(merged);
        } else {
            diff.added.push(after_dep.clone());
        }
    }
    for (key, before_dep) in &before_map {
        if !after_map.contains_key(key) {
            diff.removed.push(before_dep.clone());
        }
    }

    sort_dep_refs(&mut diff.added);
    sort_dep_refs(&mut diff.removed);
    sort_dep_refs(&mut diff.unchanged);
    diff
}

/// Finds which `target_symbols` (or file/module stem) `caller_source` references in `target_path`.
#[cfg(test)]
pub(crate) fn find_matching_references(
    caller_path: &str,
    caller_source: &str,
    target_path: &str,
    target_symbols: &[String],
    known_files: &BTreeSet<String>,
) -> Vec<String> {
    find_matching_references_with_context(
        caller_path,
        caller_source,
        target_path,
        target_symbols,
        known_files,
        None,
        None,
        None,
    )
}

/// Context-aware version of `find_matching_references` reusing cached manifests/reexports/crates.
#[allow(clippy::too_many_arguments)]
pub(crate) fn find_matching_references_with_context(
    caller_path: &str,
    caller_source: &str,
    target_path: &str,
    target_symbols: &[String],
    known_files: &BTreeSet<String>,
    manifests_opt: Option<&ManifestIndex>,
    reexports_opt: Option<&ReexportIndex>,
    crate_map_opt: Option<&CratePackageMap>,
) -> Vec<String> {
    let caller = clean_rel_key(caller_path);
    let target = clean_rel_key(target_path);
    if caller.is_empty() || target.is_empty() || caller == target || caller_source.trim().is_empty()
    {
        return Vec::new();
    }
    let Some(ir) = parse_file_ir(&caller, caller_source) else {
        return Vec::new();
    };
    find_matching_references_from_ir(
        &caller,
        caller_source,
        &ir,
        &target,
        target_symbols,
        known_files,
        manifests_opt,
        reexports_opt,
        crate_map_opt,
    )
}

/// Finds symbol references from `caller_source` to `target_path` reusing a pre-parsed `AstFileExtraction`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn find_matching_references_from_ir(
    caller_path: &str,
    caller_source: &str,
    caller_ir: &AstFileExtraction,
    target_path: &str,
    target_symbols: &[String],
    known_files: &BTreeSet<String>,
    manifests_opt: Option<&ManifestIndex>,
    reexports_opt: Option<&ReexportIndex>,
    crate_map_opt: Option<&CratePackageMap>,
) -> Vec<String> {
    let caller = clean_rel_key(caller_path);
    let target = clean_rel_key(target_path);
    if caller.is_empty() || target.is_empty() || caller == target || caller_source.trim().is_empty()
    {
        return Vec::new();
    }

    let augmented_known: Cow<'_, BTreeSet<String>> = if known_files.contains(&target) {
        Cow::Borrowed(known_files)
    } else {
        let mut cloned = known_files.clone();
        cloned.insert(target.clone());
        Cow::Owned(cloned)
    };
    let deps = extract_file_deps_from_ir(
        &caller,
        caller_ir,
        augmented_known.as_ref(),
        manifests_opt,
        reexports_opt,
        crate_map_opt,
    );
    let mut matched_deps = Vec::new();
    for dep in &deps {
        if dep.resolved && clean_rel_key(&dep.target) == target {
            matched_deps.push(dep);
        }
    }

    let target_stem = file_stem(&target);
    let caller_idents: Cow<'_, BTreeSet<String>> = if caller_ir.identifiers.is_empty() {
        Cow::Owned(crate::graph_ast::extract_code_identifiers(
            &caller,
            caller_source,
        ))
    } else {
        Cow::Borrowed(&caller_ir.identifiers)
    };

    let same_folder = folder_of(&caller) == folder_of(&target);
    let stem_referenced =
        !target_stem.is_empty() && target_stem.len() >= 2 && caller_idents.contains(target_stem);
    let has_target_link = !matched_deps.is_empty() || (same_folder && stem_referenced);
    if !has_target_link && !same_folder {
        return Vec::new();
    }

    let mut out = Vec::new();

    if target_symbols.is_empty() {
        for dep in &matched_deps {
            for sym in &dep.symbols {
                push_unique_str(&mut out, sym);
            }
        }
        if out.is_empty() && has_target_link && !target_stem.is_empty() {
            push_unique_str(&mut out, target_stem);
        }
        return out;
    }

    for sym in target_symbols {
        let trimmed = sym.trim();
        if trimmed.is_empty() {
            continue;
        }
        let dep_has_sym = matched_deps
            .iter()
            .any(|dep| dep.symbols.iter().any(|s| s == trimmed));
        if dep_has_sym || (has_target_link && caller_idents.contains(trimmed)) {
            push_unique_str(&mut out, trimmed);
        }
    }

    if out.is_empty() && !matched_deps.is_empty() {
        for dep in &matched_deps {
            for sym in &dep.symbols {
                push_unique_str(&mut out, sym);
            }
        }
        if out.is_empty() && !target_stem.is_empty() {
            push_unique_str(&mut out, target_stem);
        }
    }
    out
}

fn norm_member_name(name: &str) -> &str {
    if name.starts_with('$') && name.len() > 1 {
        &name[1..]
    } else {
        name
    }
}

fn compute_member_access_risk(tm: &DeclaredMember, access: &str) -> Option<String> {
    if tm.visibility_narrowed {
        return Some("narrowed".to_owned());
    }
    let vis = tm.visibility.as_deref().map(str::to_ascii_lowercase);
    match vis.as_deref() {
        Some("private" | "#private" | "pub(self)") => Some("private".to_owned()),
        Some("private(set)" | "public private(set)" | "protected private(set)") => {
            if access == "write" {
                Some("private_set".to_owned())
            } else {
                None
            }
        }
        Some("protected(set)" | "public protected(set)") => {
            if access == "write" {
                Some("protected".to_owned())
            } else {
                None
            }
        }
        Some("protected") => Some("protected".to_owned()),
        _ => {
            if access == "write" && tm.modifiers.iter().any(|m| m == "readonly") {
                Some("readonly".to_owned())
            } else {
                None
            }
        }
    }
}

fn push_unique_member_ref(out: &mut Vec<DepMemberRef>, candidate: DepMemberRef) {
    if let Some(existing) = out.iter_mut().find(|m| {
        m.name == candidate.name
            && m.kind == candidate.kind
            && m.parent == candidate.parent
            && m.access == candidate.access
    }) {
        if existing
            .line
            .is_none_or(|prev| candidate.line.is_some_and(|l| l < prev))
        {
            existing.line = candidate.line;
        }
        for line in candidate.lines {
            if let Err(at) = existing.lines.binary_search(&line) {
                existing.lines.insert(at, line);
            }
        }
        if existing.visibility.is_none() {
            existing.visibility = candidate.visibility;
        }
        if existing.previous_visibility.is_none() {
            existing.previous_visibility = candidate.previous_visibility;
        }
        if existing.change.is_none() {
            existing.change = candidate.change;
        }
        if existing.access_risk.is_none() {
            existing.access_risk = candidate.access_risk;
        }
        for m in candidate.modifiers {
            if !existing.modifiers.contains(&m) {
                existing.modifiers.push(m);
            }
        }
        return;
    }
    out.push(candidate);
}

fn sort_member_refs(refs: &mut [DepMemberRef]) {
    refs.sort_by(|a, b| {
        a.line
            .unwrap_or(u32::MAX)
            .cmp(&b.line.unwrap_or(u32::MAX))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.access.cmp(&b.access))
    });
}

/// Finds member-level references from `caller_source` to `target_path` matched against `target_members`.
pub(crate) fn find_matching_member_references(
    caller_path: &str,
    caller_source: &str,
    target_path: &str,
    target_members: &[DeclaredMember],
    known_files: &BTreeSet<String>,
) -> Vec<DepMemberRef> {
    find_matching_member_references_with_context(
        caller_path,
        caller_source,
        target_path,
        target_members,
        known_files,
        None,
        None,
        None,
    )
}

/// Context-aware version of `find_matching_member_references`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn find_matching_member_references_with_context(
    caller_path: &str,
    caller_source: &str,
    target_path: &str,
    target_members: &[DeclaredMember],
    known_files: &BTreeSet<String>,
    manifests_opt: Option<&ManifestIndex>,
    reexports_opt: Option<&ReexportIndex>,
    crate_map_opt: Option<&CratePackageMap>,
) -> Vec<DepMemberRef> {
    let caller = clean_rel_key(caller_path);
    let target = clean_rel_key(target_path);
    if caller.is_empty() || target.is_empty() || caller == target || caller_source.trim().is_empty()
    {
        return Vec::new();
    }
    let Some(ir) = parse_file_ir(&caller, caller_source) else {
        return Vec::new();
    };
    find_matching_member_references_from_ir(
        &caller,
        caller_source,
        &ir,
        &target,
        target_members,
        known_files,
        manifests_opt,
        reexports_opt,
        crate_map_opt,
    )
}

/// Finds member-level references from `caller_source` to `target_path` reusing a pre-parsed `AstFileExtraction`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn find_matching_member_references_from_ir(
    caller_path: &str,
    caller_source: &str,
    caller_ir: &AstFileExtraction,
    target_path: &str,
    target_members: &[DeclaredMember],
    known_files: &BTreeSet<String>,
    manifests_opt: Option<&ManifestIndex>,
    reexports_opt: Option<&ReexportIndex>,
    crate_map_opt: Option<&CratePackageMap>,
) -> Vec<DepMemberRef> {
    let caller = clean_rel_key(caller_path);
    let target = clean_rel_key(target_path);
    if caller.is_empty() || target.is_empty() || caller == target || caller_source.trim().is_empty()
    {
        return Vec::new();
    }

    let augmented_known: Cow<'_, BTreeSet<String>> = if known_files.contains(&target) {
        Cow::Borrowed(known_files)
    } else {
        let mut cloned = known_files.clone();
        cloned.insert(target.clone());
        Cow::Owned(cloned)
    };
    let deps = extract_file_deps_from_ir(
        &caller,
        caller_ir,
        augmented_known.as_ref(),
        manifests_opt,
        reexports_opt,
        crate_map_opt,
    );
    let mut matched_deps = Vec::new();
    for dep in &deps {
        if dep.resolved && clean_rel_key(&dep.target) == target {
            matched_deps.push(dep);
        }
    }

    let target_stem = file_stem(&target);
    let caller_idents: Cow<'_, BTreeSet<String>> = if caller_ir.identifiers.is_empty() {
        Cow::Owned(crate::graph_ast::extract_code_identifiers(
            &caller,
            caller_source,
        ))
    } else {
        Cow::Borrowed(&caller_ir.identifiers)
    };
    let same_folder = folder_of(&caller) == folder_of(&target);
    let stem_referenced =
        !target_stem.is_empty() && target_stem.len() >= 2 && caller_idents.contains(target_stem);
    let has_target_link = !matched_deps.is_empty() || (same_folder && stem_referenced);
    if !has_target_link && !same_folder {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut raw_refs: Vec<DepMemberRef> = Vec::new();
    for dep in &matched_deps {
        for r in &dep.member_refs {
            push_unique_member_ref(&mut raw_refs, r.clone());
        }
    }

    if target_members.is_empty() {
        sort_member_refs(&mut raw_refs);
        return raw_refs;
    }

    let mut matched_tm_indices = BTreeSet::new();
    for r in raw_refs {
        let r_norm = norm_member_name(&r.name);
        let best = target_members
            .iter()
            .enumerate()
            .filter_map(|(idx, tm)| {
                let tm_norm = norm_member_name(&tm.name);
                if tm.name != r.name && tm_norm != r_norm {
                    return None;
                }
                if let (Some(rp), Some(tp)) = (r.parent.as_deref(), tm.parent.as_deref())
                    && rp != tp
                {
                    return None;
                }
                let mut score = 0u8;
                if tm.name == r.name {
                    score += 4;
                }
                if r.parent.is_some() && tm.parent == r.parent {
                    score += 4;
                }
                if tm.kind == r.kind {
                    score += 2;
                }
                Some((score, idx, tm))
            })
            .max_by_key(|(score, _, _)| *score);

        if let Some((_, idx, tm)) = best {
            matched_tm_indices.insert(idx);
            let risk = compute_member_access_risk(tm, &r.access);
            push_unique_member_ref(
                &mut out,
                DepMemberRef {
                    name: tm.name.clone(),
                    kind: tm.kind.clone(),
                    parent: tm.parent.clone().or(r.parent),
                    access: r.access,
                    line: r.line,
                    lines: r.lines,
                    visibility: tm.visibility.clone(),
                    previous_visibility: tm.previous_visibility.clone(),
                    change: Some(tm.change.clone()),
                    modifiers: tm.modifiers.clone(),
                    access_risk: risk,
                },
            );
        }
    }

    if has_target_link {
        let stripped_caller = strip_comments_for_lines(&caller, caller_source);
        let caller_lines: Vec<&str> = stripped_caller.lines().collect();
        for (idx, tm) in target_members.iter().enumerate() {
            if matched_tm_indices.contains(&idx) {
                continue;
            }
            let bare = norm_member_name(&tm.name);
            if bare.len() < 2
                || (!caller_idents.contains(bare) && !caller_idents.contains(&tm.name))
            {
                continue;
            }
            let owner_ok = match tm.parent.as_deref() {
                None => {
                    matched_deps
                        .iter()
                        .any(|d| d.symbols.iter().any(|s| s == &tm.name || s == bare))
                        || (has_target_link && caller_idents.contains(&tm.name))
                }
                Some(owner) => {
                    matched_deps
                        .iter()
                        .any(|d| d.symbols.iter().any(|s| s == owner))
                        || (same_folder && caller_idents.contains(owner))
                }
            };
            if !owner_ok {
                continue;
            }

            let mut matched_line: Option<(u32, &'static str)> = None;
            for (li, line) in caller_lines.iter().enumerate() {
                if !line.contains(bare) {
                    continue;
                }
                if let Some(access) =
                    find_member_syntax_on_line(line, &tm.name, bare, &tm.kind, tm.parent.is_none())
                {
                    matched_line = Some(((li + 1) as u32, access));
                    break;
                }
            }
            if let Some((line_no, access)) = matched_line {
                let risk = compute_member_access_risk(tm, access);
                push_unique_member_ref(
                    &mut out,
                    DepMemberRef {
                        name: tm.name.clone(),
                        kind: tm.kind.clone(),
                        parent: tm.parent.clone(),
                        access: access.to_owned(),
                        line: Some(line_no),
                        lines: vec![line_no],
                        visibility: tm.visibility.clone(),
                        previous_visibility: tm.previous_visibility.clone(),
                        change: Some(tm.change.clone()),
                        modifiers: tm.modifiers.clone(),
                        access_risk: risk,
                    },
                );
            }
        }
    }

    sort_member_refs(&mut out);
    out
}

fn find_member_syntax_on_line(
    line: &str,
    raw_name: &str,
    bare: &str,
    kind: &str,
    is_top_level: bool,
) -> Option<&'static str> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    for needle in [raw_name, bare] {
        let mut from = 0usize;
        while let Some(rel) = line[from..].find(needle) {
            let pos = from + rel;
            let end = pos + needle.len();
            from = pos + 1;
            let right_ok = end == len || !is_ident_char(bytes[end] as char);
            if !right_ok {
                continue;
            }
            let prefix = line[..pos].trim_end();
            let rest = line[end..].trim_start();
            let has_member_op = prefix.ends_with("->")
                || prefix.ends_with("?->")
                || prefix.ends_with("::")
                || prefix.ends_with('.');
            let left_boundary = pos == 0 || !is_ident_char(bytes[pos - 1] as char);
            if has_member_op {
                if rest.starts_with('(') {
                    return Some("call");
                }
                if prefix.ends_with("::") {
                    return Some(if kind == "const" {
                        "const"
                    } else {
                        "static_read"
                    });
                }
                if rest.starts_with('=') && !rest.starts_with("==") && !rest.starts_with("=>") {
                    return Some("write");
                }
                return Some(if kind == "const" { "const" } else { "read" });
            }
            if is_top_level && left_boundary {
                if kind == "function" && rest.starts_with('(') {
                    return Some("call");
                }
                if kind == "const" {
                    return Some("const");
                }
            }
        }
    }
    None
}

fn push_unique_str(out: &mut Vec<String>, sym: &str) {
    if !sym.is_empty() && !out.iter().any(|existing| existing == sym) {
        out.push(sym.to_owned());
    }
}

struct DepAccumulator<'a> {
    self_path: String,
    known_files: &'a BTreeSet<String>,
    include_self: bool,
    entries: BTreeMap<(String, String), DepRef>,
}

impl<'a> DepAccumulator<'a> {
    fn new_with_self(
        self_path: &str,
        known_files: &'a BTreeSet<String>,
        include_self: bool,
    ) -> Self {
        Self {
            self_path: clean_rel_key(self_path),
            known_files,
            include_self,
            entries: BTreeMap::new(),
        }
    }

    fn add_resolved(
        &mut self,
        target: String,
        kind: &str,
        symbols: &[&str],
        line: u32,
        evidence: &str,
    ) {
        self.add_resolved_members(target, kind, symbols, &[], line, evidence);
    }

    fn add_resolved_members(
        &mut self,
        target: String,
        kind: &str,
        symbols: &[&str],
        members: &[DepMemberRef],
        line: u32,
        evidence: &str,
    ) {
        let cleaned = clean_rel_key(&target);
        if cleaned.is_empty() {
            return;
        }
        let is_self = cleaned == self.self_path;
        if is_self && (!self.include_self || (kind == "imports" && members.is_empty())) {
            return;
        }
        let exists = is_self || self.known_files.contains(&target);
        self.insert_dep(target, true, exists, kind, symbols, members, line, evidence);
    }

    fn add_unresolved(
        &mut self,
        target: &str,
        kind: &str,
        symbols: &[&str],
        line: u32,
        evidence: &str,
    ) {
        let trimmed = target.trim();
        if trimmed.is_empty() {
            return;
        }
        self.insert_dep(
            trimmed.to_owned(),
            false,
            false,
            kind,
            symbols,
            &[],
            line,
            evidence,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_dep(
        &mut self,
        target: String,
        resolved: bool,
        exists: bool,
        kind: &str,
        symbols: &[&str],
        members: &[DepMemberRef],
        line: u32,
        evidence: &str,
    ) {
        let key = (target.clone(), kind.to_owned());
        let trimmed_ev = clip_evidence(evidence);
        let entry = self.entries.entry(key).or_insert_with(|| DepRef {
            target,
            resolved,
            exists,
            kind: kind.to_owned(),
            symbols: Vec::new(),
            line: Some(line),
            evidence: (!trimmed_ev.is_empty()).then_some(trimmed_ev.clone()),
            member_refs: Vec::new(),
            target_members: Vec::new(),
        });
        if resolved && !entry.resolved {
            entry.resolved = true;
            entry.exists = exists;
        }
        if entry.line.is_none_or(|prev| line < prev) {
            entry.line = Some(line);
        }
        if entry.evidence.is_none() && !trimmed_ev.is_empty() {
            entry.evidence = Some(trimmed_ev);
        }
        for sym in symbols {
            let s = sym.trim();
            if !s.is_empty() && !entry.symbols.iter().any(|existing| existing == s) {
                entry.symbols.push(s.to_owned());
            }
        }
        for m in members {
            push_unique_member_ref(&mut entry.member_refs, m.clone());
        }
    }

    fn into_sorted_vec(self) -> Vec<DepRef> {
        let mut out: Vec<DepRef> = self.entries.into_values().collect();
        sort_dep_refs(&mut out);
        out
    }
}

fn merge_dep_into_map(map: &mut BTreeMap<(String, String), DepRef>, dep: DepRef) {
    let key = (dep.target.clone(), dep.kind.clone());
    if let Some(existing) = map.get_mut(&key) {
        for sym in dep.symbols {
            if !existing.symbols.contains(&sym) {
                existing.symbols.push(sym);
            }
        }
        for m in dep.member_refs {
            push_unique_member_ref(&mut existing.member_refs, m);
        }
        if existing
            .line
            .is_none_or(|prev| dep.line.is_some_and(|l| l < prev))
        {
            existing.line = dep.line;
        }
        if existing.evidence.is_none() {
            existing.evidence = dep.evidence;
        }
    } else {
        map.insert(key, dep);
    }
}

fn sort_dep_refs(list: &mut [DepRef]) {
    for dep in list.iter_mut() {
        sort_member_refs(&mut dep.member_refs);
    }
    list.sort_by(|a, b| {
        b.resolved
            .cmp(&a.resolved)
            .then_with(|| a.line.unwrap_or(u32::MAX).cmp(&b.line.unwrap_or(u32::MAX)))
            .then_with(|| a.target.cmp(&b.target))
            .then_with(|| a.kind.cmp(&b.kind))
    });
}

fn clip_evidence(raw: &str) -> String {
    let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.len() <= 140 {
        return compact;
    }
    let mut end = 140usize;
    while end > 0 && !compact.is_char_boundary(end) {
        end -= 1;
    }
    compact[..end].to_owned()
}

const MAX_CACHED_KNOWN_INDICES: usize = 16;

struct KnownIndex {
    len: usize,
    first: Option<String>,
    last: Option<String>,
    h1: u64,
    h2: u64,
    by_cleaned: BTreeMap<String, String>,
    by_lower: BTreeMap<String, Vec<String>>,
    by_stem_lower: BTreeMap<String, Vec<String>>,
    by_dir_stem_lower: BTreeMap<(String, String), Vec<String>>,
    swift_by_dir: BTreeMap<String, String>,
    all_entries: Vec<(String, String, String)>,
}

impl KnownIndex {
    fn digest(known_files: &BTreeSet<String>) -> (usize, u64, u64) {
        let mut h1 = 0xcbf2_9ce4_8422_2325u64;
        let mut h2 = DefaultHasher::new();
        for item in known_files {
            for b in item.as_bytes() {
                h1 ^= u64::from(*b);
                h1 = h1.wrapping_mul(0x100_0000_01b3);
            }
            h1 ^= 0xff;
            h1 = h1.wrapping_mul(0x100_0000_01b3);
            item.hash(&mut h2);
            0xffu8.hash(&mut h2);
        }
        (known_files.len(), h1, h2.finish())
    }

    fn build(known_files: &BTreeSet<String>, len: usize, h1: u64, h2: u64) -> Arc<Self> {
        let mut by_cleaned = BTreeMap::new();
        let mut by_lower: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut by_stem_lower: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut by_dir_stem_lower: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
        let mut swift_by_dir: BTreeMap<String, String> = BTreeMap::new();
        let mut all_entries = Vec::with_capacity(known_files.len());

        for orig in known_files {
            let cleaned = clean_rel_key(orig);
            if cleaned.is_empty() {
                continue;
            }
            let lower = cleaned.to_ascii_lowercase();
            by_cleaned
                .entry(cleaned.clone())
                .or_insert_with(|| orig.clone());
            by_lower
                .entry(lower.clone())
                .or_default()
                .push(orig.clone());

            let stem_lower = file_stem(&cleaned).to_ascii_lowercase();
            let dir = folder_of(&cleaned);
            let dir_lower = dir.to_ascii_lowercase();
            if !stem_lower.is_empty() {
                by_stem_lower
                    .entry(stem_lower.clone())
                    .or_default()
                    .push(orig.clone());
                by_dir_stem_lower
                    .entry((dir_lower, stem_lower))
                    .or_default()
                    .push(orig.clone());
            }
            if has_any_ext(&cleaned, SWIFT_EXTS) && !dir.is_empty() {
                let segs: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
                for start in 0..segs.len() {
                    let mut sub = segs[start].to_owned();
                    swift_by_dir
                        .entry(sub.clone())
                        .or_insert_with(|| orig.clone());
                    for seg in &segs[(start + 1)..] {
                        sub.push('/');
                        sub.push_str(seg);
                        swift_by_dir
                            .entry(sub.clone())
                            .or_insert_with(|| orig.clone());
                    }
                }
            }
            all_entries.push((orig.clone(), cleaned, lower));
        }

        Arc::new(Self {
            len,
            first: known_files.iter().next().cloned(),
            last: known_files.iter().next_back().cloned(),
            h1,
            h2,
            by_cleaned,
            by_lower,
            by_stem_lower,
            by_dir_stem_lower,
            swift_by_dir,
            all_entries,
        })
    }

    fn for_set(known_files: &BTreeSet<String>) -> Arc<Self> {
        static CACHE: OnceLock<Mutex<Vec<Arc<KnownIndex>>>> = OnceLock::new();
        let (len, h1, h2) = Self::digest(known_files);
        let first = known_files.iter().next();
        let last = known_files.iter().next_back();
        let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
        if let Ok(mut entries) = cache.lock()
            && let Some(pos) = entries.iter().position(|e| {
                e.len == len
                    && e.h1 == h1
                    && e.h2 == h2
                    && e.first.as_ref() == first
                    && e.last.as_ref() == last
            })
        {
            let hit = entries.remove(pos);
            let cloned = Arc::clone(&hit);
            entries.push(hit);
            return cloned;
        }
        let built = Self::build(known_files, len, h1, h2);
        if let Ok(mut entries) = cache.lock() {
            if entries.len() >= MAX_CACHED_KNOWN_INDICES {
                entries.remove(0);
            }
            entries.push(Arc::clone(&built));
        }
        built
    }
}

struct KnownLookup<'a> {
    known_files: &'a BTreeSet<String>,
    index: Arc<KnownIndex>,
}

impl<'a> KnownLookup<'a> {
    fn new(known_files: &'a BTreeSet<String>) -> Self {
        Self {
            known_files,
            index: KnownIndex::for_set(known_files),
        }
    }

    fn all_entries(&self) -> &[(String, String, String)] {
        &self.index.all_entries
    }

    fn resolve_swift_module(&self, module: &str) -> Option<String> {
        let clean = module.trim().trim_matches('/');
        if clean.is_empty() {
            return None;
        }
        if let Some(hit) = self.index.swift_by_dir.get(clean) {
            return Some(hit.clone());
        }
        if let Some((head, _)) = clean.split_once('.') {
            let slash = clean.replace('.', "/");
            if let Some(hit) = self.index.swift_by_dir.get(&slash) {
                return Some(hit.clone());
            }
            if let Some(hit) = self.index.swift_by_dir.get(head) {
                return Some(hit.clone());
            }
        }
        None
    }

    fn resolve_exact_or_ci(&self, candidate: &str) -> Option<String> {
        let cleaned = clean_rel_key(candidate);
        if cleaned.is_empty() {
            return None;
        }
        if let Some(exact) = self.index.by_cleaned.get(&cleaned) {
            return Some(exact.clone());
        }
        let lower = cleaned.to_ascii_lowercase();
        self.index
            .by_lower
            .get(&lower)
            .and_then(|matches| pick_best_candidate(matches))
    }

    fn resolve_relative(&self, base_dir: &str, spec: &str) -> Option<String> {
        let joined = resolve_relative_segments(base_dir, spec)?;
        self.resolve_exact_or_ci(&joined)
    }

    fn resolve_suffix(&self, suffix: &str, allowed_exts: Option<&[&str]>) -> Option<String> {
        let cleaned = clean_rel_key(suffix);
        if cleaned.is_empty() {
            return None;
        }
        let slash_suffix = format!("/{cleaned}");
        let exact_matches: Vec<String> = self
            .index
            .all_entries
            .iter()
            .filter(|(_, c, _)| {
                (c == &cleaned || c.ends_with(&slash_suffix))
                    && allowed_exts.is_none_or(|exts| has_any_ext(c, exts))
            })
            .map(|(orig, _, _)| orig.clone())
            .collect();
        if let Some(found) = pick_best_candidate(&exact_matches) {
            return Some(found);
        }

        let lower_cleaned = cleaned.to_ascii_lowercase();
        let lower_slash = format!("/{lower_cleaned}");
        let ci_matches: Vec<String> = self
            .index
            .all_entries
            .iter()
            .filter(|(_, c, l)| {
                (l == &lower_cleaned || l.ends_with(&lower_slash))
                    && allowed_exts.is_none_or(|exts| has_any_ext(c, exts))
            })
            .map(|(orig, _, _)| orig.clone())
            .collect();
        pick_best_candidate(&ci_matches)
    }

    fn resolve_stem_in_dir(&self, dir: &str, stem: &str, allowed_exts: &[&str]) -> Option<String> {
        let dir_lower = dir.to_ascii_lowercase();
        for variant in stem_variants(stem) {
            if let Some(list) = self
                .index
                .by_dir_stem_lower
                .get(&(dir_lower.clone(), variant))
            {
                let filtered: Vec<String> = list
                    .iter()
                    .filter(|p| has_any_ext(p, allowed_exts))
                    .cloned()
                    .collect();
                if filtered.len() == 1 {
                    return Some(filtered[0].clone());
                }
                if let Some(best) = pick_unique_non_test(&filtered) {
                    return Some(best);
                }
            }
        }
        None
    }

    fn resolve_unambiguous_stem(
        &self,
        stem: &str,
        allowed_exts: &[&str],
        scope_prefix: Option<&str>,
    ) -> Option<String> {
        for variant in stem_variants(stem) {
            let Some(list) = self.index.by_stem_lower.get(&variant) else {
                continue;
            };
            let filtered: Vec<String> = list
                .iter()
                .filter(|p| has_any_ext(p, allowed_exts))
                .cloned()
                .collect();
            if filtered.len() == 1 {
                return Some(filtered[0].clone());
            }
            if let Some(prefix) = scope_prefix.filter(|p| !p.is_empty()) {
                let prefix_slash = format!("{prefix}/");
                let scoped: Vec<String> = filtered
                    .iter()
                    .filter(|p| p.starts_with(&prefix_slash))
                    .cloned()
                    .collect();
                if scoped.len() == 1 {
                    return Some(scoped[0].clone());
                }
            }
            if let Some(best) = pick_unique_non_test(&filtered) {
                return Some(best);
            }
        }
        None
    }
}

fn stem_variants(stem: &str) -> Vec<String> {
    let lower = stem.to_ascii_lowercase();
    let snake = camel_to_snake_lower(stem);
    if snake == lower {
        vec![lower]
    } else {
        vec![lower, snake]
    }
}

fn camel_to_snake_lower(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    let chars: Vec<char> = s.chars().collect();
    for (idx, &ch) in chars.iter().enumerate() {
        if ch.is_ascii_uppercase() {
            if idx > 0 {
                let prev = chars[idx - 1];
                let next_lower = chars.get(idx + 1).is_some_and(|c| c.is_ascii_lowercase());
                if prev.is_ascii_lowercase()
                    || prev.is_ascii_digit()
                    || (prev.is_ascii_uppercase() && next_lower)
                {
                    out.push('_');
                }
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch.to_ascii_lowercase());
        }
    }
    out
}

fn has_any_ext(path: &str, exts: &[&str]) -> bool {
    let Some((_, ext)) = path.rsplit_once('.') else {
        return false;
    };
    let lower = ext.to_ascii_lowercase();
    exts.contains(&lower.as_str())
}

fn pick_best_candidate(matches: &[String]) -> Option<String> {
    if matches.is_empty() {
        return None;
    }
    if matches.len() == 1 {
        return Some(matches[0].clone());
    }
    if let Some(unique) = pick_unique_non_test(matches) {
        return Some(unique);
    }
    let mut sorted = matches.to_vec();
    sorted.sort_by(|a, b| {
        candidate_rank(a)
            .cmp(&candidate_rank(b))
            .then_with(|| a.len().cmp(&b.len()))
            .then_with(|| a.cmp(b))
    });
    sorted.into_iter().next()
}

fn pick_unique_non_test(matches: &[String]) -> Option<String> {
    let non_test: Vec<&String> = matches
        .iter()
        .filter(|p| !looks_like_test_or_vendor(p))
        .collect();
    if non_test.len() == 1 {
        Some(non_test[0].clone())
    } else {
        None
    }
}

fn candidate_rank(path: &str) -> u8 {
    let lower = path.to_ascii_lowercase();
    if lower.starts_with("src/") || lower.starts_with("app/") {
        0
    } else if !looks_like_test_or_vendor(path) {
        1
    } else {
        2
    }
}

fn looks_like_test_or_vendor(path: &str) -> bool {
    path.split('/').any(|seg| {
        matches!(
            seg.to_ascii_lowercase().as_str(),
            "test" | "tests" | "__tests__" | "spec" | "fixtures" | "vendor" | "node_modules"
        )
    })
}

fn file_stem(rel_path: &str) -> &str {
    let name = rel_path.rsplit_once('/').map_or(rel_path, |(_, n)| n);
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

fn resolve_relative_segments(base_dir: &str, spec: &str) -> Option<String> {
    let norm_spec = spec.trim().replace('\\', "/");
    if norm_spec.is_empty() {
        return None;
    }
    let mut stack: Vec<&str> = if norm_spec.starts_with('/') {
        Vec::new()
    } else {
        base_dir
            .split('/')
            .filter(|s| !s.is_empty() && *s != ".")
            .collect()
    };
    for seg in norm_spec.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                stack.pop()?;
            }
            other => stack.push(other),
        }
    }
    let joined = stack.join("/");
    (!joined.is_empty()).then_some(joined)
}

// =========================================================================
// Shared Member Classification Helpers
// =========================================================================

/// The kind of dependency a member access is drawn as: writing state,
/// reading it, or calling. Anything else stays a plain use.
pub(crate) fn access_kind(access: &str) -> Option<&'static str> {
    match access {
        "write" => Some("writes"),
        "read" | "static_read" | "const" => Some("reads"),
        "call" => Some("calls"),
        _ => None,
    }
}

// =========================================================================
// Shared Re-export & Line Comment Stripping Helpers
// =========================================================================

fn resolve_reexport_step(
    cur_file: &str,
    entry: &ReexportEntry,
    cur_sym: &str,
    lookup: &KnownLookup,
    manifests_opt: Option<&ManifestIndex>,
    crate_map_opt: Option<&CratePackageMap>,
) -> Option<(String, String)> {
    let next_sym = if entry.is_wildcard {
        cur_sym.to_owned()
    } else {
        entry.original_symbol.clone()
    };
    if cur_file.ends_with(".rs") {
        let crate_src_root = rust_crate_src_root(cur_file);
        let segs: Vec<&str> = entry
            .source_module_path
            .split("::")
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "*")
            .collect();
        if let Some((target, _)) = resolve_rust_path_segments_with_crates(
            cur_file,
            &crate_src_root,
            &segs,
            lookup,
            crate_map_opt,
        ) {
            return Some((target, next_sym));
        }
        // Fallback for `pub use submodule::Thing;` relative to current file.
        if let Some(first) = segs.first()
            && !matches!(*first, "crate" | "super" | "self")
            && let Some(target) = resolve_rust_child_mod(cur_file, first, lookup)
        {
            return Some((target, next_sym));
        }
        return None;
    }
    if cur_file
        .rsplit('.')
        .next()
        .is_some_and(|ext| PHP_EXTS.iter().any(|p| ext.eq_ignore_ascii_case(p)))
    {
        let dir = folder_of(cur_file);
        let cleaned = entry.source_module_path.trim().trim_start_matches('/');
        let target = lookup
            .resolve_exact_or_ci(cleaned)
            .or_else(|| lookup.resolve_relative(&dir, &entry.source_module_path))
            .or_else(|| resolve_php_fqn(&entry.source_module_path, None, &dir, lookup))?;
        return Some((target, next_sym));
    }
    // TS/JS re-export `export { Foo } from './bar'`
    let dir = folder_of(cur_file);
    let target = resolve_ts_js_spec(&dir, &entry.source_module_path, lookup)
        .or_else(|| {
            manifests_opt.and_then(|m| {
                m.resolve_ts_import(cur_file, &entry.source_module_path, lookup.known_files)
            })
        })
        .or_else(|| resolve_ts_js_alias_fallback(&entry.source_module_path, lookup))?;
    Some((target, next_sym))
}

/// Traverses `(barrel_file, orig_path)` re-export links backwards from `seeds` up to
/// `max_hops` (`MAX_REEXPORT_HOPS`), returning every barrel file on any upstream re-export
/// chain (including seed files that themselves declare re-exports) in a cycle-safe BFS.
pub(crate) fn collect_backward_reexport_barrels(
    reexport_rows: &[(String, String)],
    seeds: &BTreeSet<String>,
    known_files: &BTreeSet<String>,
    manifests_opt: Option<&ManifestIndex>,
    crate_map_opt: Option<&CratePackageMap>,
    max_hops: usize,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if reexport_rows.is_empty() || seeds.is_empty() {
        return out;
    }
    let lookup = KnownLookup::new(known_files);
    let mut barrels_with_reexports = BTreeSet::new();
    let mut reverse_adj: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (barrel_file, orig_path) in reexport_rows {
        barrels_with_reexports.insert(barrel_file.as_str());
        let entry = ReexportEntry {
            source_module_path: orig_path.clone(),
            exported_symbol: "*".to_owned(),
            original_symbol: "*".to_owned(),
            is_wildcard: true,
        };
        let resolved_target = resolve_reexport_step(
            barrel_file,
            &entry,
            "*",
            &lookup,
            manifests_opt,
            crate_map_opt,
        )
        .map(|(t, _)| t);
        if let Some(target_file) = resolved_target {
            reverse_adj
                .entry(target_file)
                .or_default()
                .push(barrel_file.clone());
        }
    }

    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    for seed in seeds {
        if barrels_with_reexports.contains(seed.as_str()) {
            out.insert(seed.clone());
        }
        if visited.insert(seed.clone()) {
            queue.push_back((seed.clone(), 0usize));
        }
    }
    while let Some((cur, hops)) = queue.pop_front() {
        if hops >= max_hops {
            continue;
        }
        if let Some(barrels) = reverse_adj.get(&cur) {
            for barrel in barrels {
                out.insert(barrel.clone());
                if visited.insert(barrel.clone()) {
                    queue.push_back((barrel.clone(), hops + 1));
                }
            }
        }
    }
    out
}

fn strip_comments_for_lines(rel_path: &str, source: &str) -> String {
    let Some(lang) = detect_language(rel_path) else {
        return source.to_owned();
    };
    match lang {
        Language::Python | Language::Ruby => source
            .lines()
            .map(|line| {
                if let Some(idx) = line.find('#') {
                    &line[..idx]
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => {
            let mut out = String::with_capacity(source.len());
            let mut in_block = false;
            for line in source.lines() {
                let bytes = line.as_bytes();
                let mut i = 0usize;
                while i < bytes.len() {
                    if in_block {
                        if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                            in_block = false;
                            out.push_str("  ");
                            i += 2;
                        } else {
                            out.push(' ');
                            i += 1;
                        }
                    } else if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                        in_block = true;
                        out.push_str("  ");
                        i += 2;
                    } else if (i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/')
                        || (lang == Language::Php
                            && bytes[i] == b'#'
                            && bytes.get(i + 1) != Some(&b'['))
                    {
                        break;
                    } else if bytes[i].is_ascii() {
                        out.push(bytes[i] as char);
                        i += 1;
                    } else if let Some(ch) = line[i..].chars().next() {
                        out.push(ch);
                        i += ch.len_utf8();
                    } else {
                        break;
                    }
                }
                out.push('\n');
            }
            out
        }
    }
}

fn ref_dep_kind_and_member(
    r: &AstRef,
    canon: &str,
    default_field_kind: &str,
    parent_override: Option<String>,
    php_dollar_prop: bool,
) -> (&'static str, Vec<DepMemberRef>) {
    match r.context.as_str() {
        "constructor" => {
            if r.access == "type" {
                ("uses", Vec::new())
            } else {
                ("calls", Vec::new())
            }
        }
        "parameter_type" | "return_type" | "generic_arg" => ("uses", Vec::new()),
        "call" => {
            let Some(ref mem) = r.member else {
                // Direct function or constructor call.
                let is_type = canon.chars().next().is_some_and(|c| c.is_ascii_uppercase());
                let members = if !is_type && r.member_kind.as_deref() == Some("function") {
                    vec![DepMemberRef::basic(canon, "function", None, "call", r.line)]
                } else {
                    Vec::new()
                };
                return ("calls", members);
            };
            let mk = r.member_kind.as_deref().unwrap_or("method");
            let parent = if mk == "function" {
                None
            } else {
                parent_override
            };
            (
                "calls",
                vec![DepMemberRef::basic(mem, mk, parent, "call", r.line)],
            )
        }
        "field" => {
            if r.access == "type" {
                return ("uses", Vec::new());
            }
            let Some(ref mem) = r.member else {
                return ("uses", Vec::new());
            };
            if r.access == "write_init" {
                return (
                    "uses",
                    vec![DepMemberRef::basic(
                        mem,
                        default_field_kind,
                        parent_override,
                        "write",
                        r.line,
                    )],
                );
            }
            let dep_kind = access_kind(&r.access).unwrap_or("reads");
            let mk = r.member_kind.as_deref().unwrap_or(default_field_kind);
            let mem_name = if php_dollar_prop && mk == "property" && !mem.starts_with('$') {
                format!("${mem}")
            } else {
                mem.clone()
            };
            let parent = if mk == "const" && parent_override.is_none() {
                None
            } else {
                parent_override
            };
            (
                dep_kind,
                vec![DepMemberRef::basic(mem_name, mk, parent, &r.access, r.line)],
            )
        }
        _ => ("uses", Vec::new()),
    }
}

// =========================================================================
// 1. PHP AST Resolution
// =========================================================================

fn resolve_php_ir(
    rel_path: &str,
    ir: &AstFileExtraction,
    lookup: &KnownLookup,
    manifests_opt: Option<&ManifestIndex>,
    reexports_opt: Option<&ReexportIndex>,
    acc: &mut DepAccumulator,
) {
    const PHP_BUILTINS: &[&str] = &[
        "int",
        "integer",
        "float",
        "double",
        "string",
        "bool",
        "boolean",
        "array",
        "object",
        "callable",
        "iterable",
        "void",
        "never",
        "mixed",
        "self",
        "static",
        "parent",
        "null",
        "true",
        "false",
        "stdclass",
        "exception",
        "throwable",
        "errorexception",
        "runtimeexception",
        "invalidargumentexception",
        "logicexception",
        "closure",
        "generator",
        "iterator",
        "traversable",
        "countable",
        "arrayaccess",
        "jsonserializable",
        "stringable",
        "datetime",
        "datetimeimmutable",
        "datetimeinterface",
    ];
    let dir = folder_of(rel_path);
    let current_ns = ir.namespace.as_deref();
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    let resolve_php_target = |fqn: &str, sym_hint: &str| -> (Option<String>, Option<String>) {
        let Some(target) =
            resolve_php_fqn_with_manifests(fqn, current_ns, &dir, lookup, manifests_opt)
        else {
            return (None, None);
        };
        if let Some(reexports) = reexports_opt
            && let Some((rt, orig_sym)) = reexports.resolve_reexport_target(
                &target,
                sym_hint,
                lookup.known_files,
                manifests_opt,
                None,
            )
        {
            return (Some(rt), Some(orig_sym));
        }
        (Some(target), None)
    };

    for imp in &ir.imports {
        match imp.kind.as_str() {
            "include" => {
                let resolved_blade = if let Some(view_name) = imp.path.strip_prefix("blade-view:") {
                    Some(resolve_blade_view_target(view_name, lookup))
                } else if let Some(comp_name) = imp.path.strip_prefix("blade-component:") {
                    Some(resolve_blade_component_target(
                        comp_name,
                        lookup,
                        manifests_opt,
                    ))
                } else {
                    imp.path.strip_prefix("blade-livewire:").map(|lw_name| {
                        resolve_blade_livewire_target(lw_name, lookup, manifests_opt)
                    })
                };
                if let Some(found_opt) = resolved_blade {
                    let sym = imp.symbols.first().map(String::as_str).unwrap_or("");
                    match found_opt {
                        Some(t) => acc.add_resolved(t, "imports", &[sym], imp.line, &imp.evidence),
                        None => acc.add_unresolved(
                            &imp.path,
                            "imports",
                            &[sym],
                            imp.line,
                            &imp.evidence,
                        ),
                    }
                    continue;
                }
                let norm = imp.path.trim().trim_start_matches('/');
                let sym = file_stem(norm);
                let resolved =
                    if imp.evidence.contains("__DIR__") || imp.evidence.contains("dirname(") {
                        lookup.resolve_relative(&dir, norm)
                    } else {
                        lookup
                            .resolve_relative(&dir, &imp.path)
                            .or_else(|| lookup.resolve_exact_or_ci(norm))
                            .or_else(|| lookup.resolve_suffix(norm, Some(PHP_EXTS)))
                    };
                match resolved {
                    Some(t) => acc.add_resolved(t, "imports", &[sym], imp.line, &imp.evidence),
                    None => {
                        acc.add_unresolved(&imp.path, "imports", &[sym], imp.line, &imp.evidence)
                    }
                }
            }
            "trait_use" => {
                let short = imp.symbols.first().map(String::as_str).unwrap_or(&imp.path);
                let fqn = resolve_php_alias(&imp.path, &aliases, current_ns);
                match resolve_php_target(&fqn, short) {
                    (Some(t), orig) => {
                        let resolved_sym = orig.as_deref().unwrap_or(short);
                        acc.add_resolved(t, "uses", &[resolved_sym], imp.line, &imp.evidence);
                    }
                    (None, _) => {
                        acc.add_unresolved(&fqn, "uses", &[short], imp.line, &imp.evidence)
                    }
                }
            }
            _ => {
                let fqn = imp.path.trim_start_matches('\\');
                let short = imp
                    .symbols
                    .first()
                    .cloned()
                    .unwrap_or_else(|| fqn.rsplit('\\').next().unwrap_or(fqn).to_owned());
                let local = imp.alias.clone().unwrap_or_else(|| short.clone());
                aliases.insert(local, fqn.to_owned());
                match resolve_php_target(fqn, &short) {
                    (Some(t), orig) => {
                        let resolved_sym = orig.as_deref().unwrap_or(&short);
                        acc.add_resolved(t, "imports", &[resolved_sym], imp.line, &imp.evidence);
                    }
                    (None, _) => {
                        acc.add_unresolved(fqn, "imports", &[&short], imp.line, &imp.evidence)
                    }
                }
            }
        }
    }

    for inh in &ir.inheritance {
        if inh.kind == "uses" {
            continue;
        }
        let short = inh.parent.rsplit('\\').next().unwrap_or(&inh.parent);
        let fqn = resolve_php_alias(&inh.parent, &aliases, current_ns);
        match resolve_php_target(&fqn, short) {
            (Some(t), orig) => {
                let resolved_sym = orig.as_deref().unwrap_or(short);
                acc.add_resolved(t, &inh.kind, &[resolved_sym], inh.line, &inh.evidence);
            }
            (None, _) => acc.add_unresolved(&fqn, &inh.kind, &[short], inh.line, &inh.evidence),
        }
    }

    for r in &ir.calls_and_refs {
        let raw_target = r
            .owner_type
            .as_deref()
            .filter(|_| r.target_name.starts_with('$'))
            .unwrap_or(&r.target_name);
        if raw_target.starts_with('$') {
            continue;
        }
        let short = raw_target.rsplit('\\').next().unwrap_or(raw_target);
        if PHP_BUILTINS.contains(&short.to_ascii_lowercase().as_str()) {
            continue;
        }
        let fqn = resolve_php_alias(raw_target, &aliases, current_ns);
        let base_canon = fqn.rsplit('\\').next().unwrap_or(short).to_owned();
        let (resolved_target, reexported_sym) = resolve_php_target(&fqn, &base_canon);
        let canon = reexported_sym.unwrap_or(base_canon);
        let parent_opt = r.owner_type.clone().or_else(|| Some(canon.clone()));
        let (dep_kind, members) = ref_dep_kind_and_member(r, &canon, "property", parent_opt, true);
        let mut syms = vec![canon.as_str()];
        for m in &members {
            let clean = norm_member_name(&m.name);
            if !clean.is_empty() {
                syms.push(clean);
            }
        }
        if let Some(target) = resolved_target {
            acc.add_resolved_members(target, dep_kind, &syms, &members, r.line, &r.evidence);
        } else if r.target_name.contains('\\')
            || aliases.contains_key(short)
            || r.owner_type.is_some()
            || r.context == "constructor"
            || short.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        {
            acc.add_unresolved(&fqn, dep_kind, &syms, r.line, &r.evidence);
        }
    }
}

fn resolve_php_alias(
    raw: &str,
    aliases: &BTreeMap<String, String>,
    current_ns: Option<&str>,
) -> String {
    let trimmed = raw.trim();
    if let Some(fqn) = trimmed.strip_prefix('\\') {
        return fqn.to_owned();
    }
    if let Some((head, tail)) = trimmed.split_once('\\') {
        if let Some(aliased) = aliases.get(head) {
            return format!("{aliased}\\{tail}");
        }
    } else if let Some(aliased) = aliases.get(trimmed) {
        return aliased.clone();
    }
    if let Some(ns) = current_ns.filter(|s| !s.is_empty()) {
        format!("{ns}\\{trimmed}")
    } else {
        trimmed.to_owned()
    }
}

fn resolve_php_fqn_with_manifests(
    fqn: &str,
    current_ns: Option<&str>,
    dir: &str,
    lookup: &KnownLookup,
    manifests_opt: Option<&ManifestIndex>,
) -> Option<String> {
    if let Some(manifests) = manifests_opt
        && let Some(found) = manifests.resolve_php_fqn(fqn, lookup.known_files)
    {
        return Some(found);
    }
    resolve_php_fqn(fqn, current_ns, dir, lookup)
}

fn resolve_php_fqn(
    fqn: &str,
    current_ns: Option<&str>,
    dir: &str,
    lookup: &KnownLookup,
) -> Option<String> {
    let cleaned = fqn.trim().trim_start_matches('\\');
    if cleaned.is_empty() {
        return None;
    }
    let slash_path = cleaned.replace('\\', "/");
    let segs: Vec<&str> = slash_path.split('/').filter(|s| !s.is_empty()).collect();
    for ext in PHP_EXTS {
        if let Some(found) = lookup.resolve_exact_or_ci(&format!("{slash_path}.{ext}")) {
            return Some(found);
        }
        for start in 1..segs.len() {
            let suffix = format!("{}.{ext}", segs[start..].join("/"));
            if let Some(found) = lookup.resolve_suffix(&suffix, Some(PHP_EXTS)) {
                return Some(found);
            }
        }
    }
    let short = segs.last().copied().unwrap_or(cleaned);
    if !dir.is_empty()
        && let Some(found) = lookup.resolve_stem_in_dir(dir, short, PHP_EXTS)
    {
        return Some(found);
    }
    if segs.len() == 1 || current_ns.is_some_and(|ns| cleaned.starts_with(ns)) {
        if let Some(found) = lookup.resolve_stem_in_dir(dir, short, PHP_EXTS) {
            return Some(found);
        }
        return lookup.resolve_unambiguous_stem(short, PHP_EXTS, None);
    }
    None
}

fn split_blade_ns_and_rel(raw: &str) -> (Option<String>, String) {
    let cleaned = raw
        .trim()
        .trim_matches(|c| c == '\'' || c == '"' || c == '/');
    let (ns, rest) = if let Some((ns, tail)) = cleaned.split_once("::") {
        let ns_clean = ns.trim();
        (
            (!ns_clean.is_empty()).then(|| ns_clean.to_owned()),
            tail.trim(),
        )
    } else {
        (None, cleaned)
    };
    let rel = rest.replace('.', "/").trim_matches('/').to_owned();
    (ns, rel)
}

fn resolve_blade_template_candidates(
    rel: &str,
    ns: Option<&str>,
    subpath: &str,
    lookup: &KnownLookup,
) -> Option<String> {
    if rel.is_empty() {
        return None;
    }
    let join_sub = |prefix: &str| -> String {
        if subpath.is_empty() {
            format!("{prefix}{rel}")
        } else {
            format!("{prefix}{subpath}/{rel}")
        }
    };
    if let Some(ns_name) = ns {
        for base in [
            format!("resources/views/vendor/{ns_name}/{}", join_sub("")),
            format!("resources/views/{ns_name}/{}", join_sub("")),
            format!("{ns_name}/resources/views/{}", join_sub("")),
        ] {
            for ext in ["blade.php", "index.blade.php"] {
                let sep = if ext.starts_with("index.") { "/" } else { "." };
                if let Some(found) = lookup.resolve_exact_or_ci(&format!("{base}{sep}{ext}")) {
                    return Some(found);
                }
            }
        }
    }
    let views_base = join_sub("resources/views/");
    for cand in [
        format!("{views_base}.blade.php"),
        format!("{views_base}/index.blade.php"),
        format!("{rel}.blade.php"),
    ] {
        if let Some(found) = lookup.resolve_exact_or_ci(&cand) {
            return Some(found);
        }
    }
    let suffix_base = join_sub("views/");
    lookup
        .resolve_suffix(&format!("{suffix_base}.blade.php"), None)
        .or_else(|| lookup.resolve_suffix(&format!("{suffix_base}/index.blade.php"), None))
        .or_else(|| lookup.resolve_suffix(&format!("{rel}.blade.php"), None))
}

fn kebab_to_pascal_path(raw: &str) -> String {
    raw.split(['.', '/', '\\'])
        .filter(|seg| !seg.trim().is_empty())
        .map(|seg| {
            seg.split(['-', '_'])
                .filter(|w| !w.is_empty())
                .map(|w| {
                    let mut chars = w.chars();
                    match chars.next() {
                        Some(first) => {
                            let mut s = first.to_ascii_uppercase().to_string();
                            s.extend(chars);
                            s
                        }
                        None => String::new(),
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Resolves `@include`, `@extends`, `view(...)` template targets (`blade-view:<name>`).
fn resolve_blade_view_target(view_name: &str, lookup: &KnownLookup) -> Option<String> {
    let (ns, rel) = split_blade_ns_and_rel(view_name);
    resolve_blade_template_candidates(&rel, ns.as_deref(), "", lookup)
}

/// Resolves `<x-...>` / `@component` template and class-based Blade components.
fn resolve_blade_component_target(
    comp_name: &str,
    lookup: &KnownLookup,
    manifests_opt: Option<&ManifestIndex>,
) -> Option<String> {
    let stripped = comp_name.trim().strip_prefix("x-").unwrap_or(comp_name);
    let (ns, rel) = split_blade_ns_and_rel(stripped);
    if let Some(found) =
        resolve_blade_template_candidates(&rel, ns.as_deref(), "components", lookup)
    {
        return Some(found);
    }
    let pascal = kebab_to_pascal_path(rel.trim_matches('/'));
    if pascal.is_empty() {
        return None;
    }
    let fqn = format!("App\\View\\Components\\{}", pascal.replace('/', "\\"));
    manifests_opt
        .and_then(|m| m.resolve_php_fqn(&fqn, lookup.known_files))
        .or_else(|| lookup.resolve_exact_or_ci(&format!("app/View/Components/{pascal}.php")))
        .or_else(|| lookup.resolve_suffix(&format!("View/Components/{pascal}.php"), Some(PHP_EXTS)))
}

/// Resolves `@livewire(...)` / `<livewire:...>` PHP component classes and Volt Blade templates.
fn resolve_blade_livewire_target(
    lw_name: &str,
    lookup: &KnownLookup,
    manifests_opt: Option<&ManifestIndex>,
) -> Option<String> {
    let (ns, rel) = split_blade_ns_and_rel(lw_name);
    let pascal = kebab_to_pascal_path(&rel);
    if !pascal.is_empty() {
        let ns_fqn = pascal.replace('/', "\\");
        for prefix in ["App\\Livewire", "App\\Http\\Livewire"] {
            let fqn = format!("{prefix}\\{ns_fqn}");
            if let Some(found) =
                manifests_opt.and_then(|m| m.resolve_php_fqn(&fqn, lookup.known_files))
            {
                return Some(found);
            }
        }
        for exact in [
            format!("app/Livewire/{pascal}.php"),
            format!("app/Http/Livewire/{pascal}.php"),
        ] {
            if let Some(found) = lookup.resolve_exact_or_ci(&exact) {
                return Some(found);
            }
        }
        if let Some(found) = lookup
            .resolve_suffix(&format!("Livewire/{pascal}.php"), Some(PHP_EXTS))
            .or_else(|| {
                lookup.resolve_suffix(&format!("Http/Livewire/{pascal}.php"), Some(PHP_EXTS))
            })
        {
            return Some(found);
        }
    }
    resolve_blade_template_candidates(&rel, ns.as_deref(), "livewire", lookup)
}

// =========================================================================
// 2. Rust AST Resolution (with CratePackageMap & `pub use` propagation)
// =========================================================================

fn resolve_rust_ir(
    rel_path: &str,
    ir: &AstFileExtraction,
    lookup: &KnownLookup,
    reexports_opt: Option<&ReexportIndex>,
    crate_map_opt: Option<&CratePackageMap>,
    acc: &mut DepAccumulator,
) {
    const RUST_BUILTINS: &[&str] = &[
        "std", "core", "alloc", "self", "super", "crate", "Vec", "String", "Option", "Result",
        "Box", "Arc", "Rc", "Mutex", "RwLock", "RefCell", "Cell", "Path", "PathBuf", "BTreeMap",
        "BTreeSet", "HashMap", "HashSet", "Value", "Self", "Default", "Clone", "Debug", "Display",
        "Send", "Sync", "Iterator", "From", "Into", "TryFrom", "TryInto", "Ok", "Err", "Some",
        "None", "Ordering", "Duration", "Instant", "Command", "Stdio", "fs", "io", "mem", "fmt",
        "cmp", "str", "slice", "u8", "u16", "u32", "u64", "usize", "i32", "i64", "f64", "bool",
        "char",
    ];
    let crate_src_root = rust_crate_src_root(rel_path);
    let dir = folder_of(rel_path);
    let mut aliases: BTreeMap<String, (Option<String>, String)> = BTreeMap::new();
    let mut wildcard_facades: Vec<String> = Vec::new();

    for imp in &ir.imports {
        if imp.kind == "mod_decl" {
            let mod_name = imp.symbols.first().map(String::as_str).unwrap_or(&imp.path);
            let resolved = if imp.path.contains('/') || imp.path.ends_with(".rs") {
                lookup.resolve_relative(&dir, &imp.path)
            } else {
                resolve_rust_child_mod(rel_path, mod_name, lookup)
            };
            match resolved {
                Some(t) => {
                    aliases.insert(mod_name.to_owned(), (Some(t.clone()), mod_name.to_owned()));
                    acc.add_resolved(t, "imports", &[mod_name], imp.line, &imp.evidence);
                }
                None => {
                    acc.add_unresolved(mod_name, "imports", &[mod_name], imp.line, &imp.evidence)
                }
            }
            continue;
        }

        let segs: Vec<&str> = imp
            .path
            .split("::")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if segs.is_empty() {
            continue;
        }
        let last = segs.last().copied().unwrap_or("");
        let effective: Vec<&str> = if (last == "self" || last == "*") && segs.len() > 1 {
            segs[..segs.len() - 1].to_vec()
        } else {
            segs.clone()
        };

        if let Some((mut target, split_at)) = resolve_rust_path_segments_with_crates(
            rel_path,
            &crate_src_root,
            &effective,
            lookup,
            crate_map_opt,
        ) {
            let mut orig_sym = if imp.is_wildcard {
                "*".to_owned()
            } else if last == "self" {
                effective.last().copied().unwrap_or("").to_owned()
            } else {
                last.to_owned()
            };
            if imp.is_wildcard {
                if !wildcard_facades.contains(&target) {
                    wildcard_facades.push(target.clone());
                }
            } else if let Some(reexports) = reexports_opt
                && !orig_sym.is_empty()
                && let Some((real_target, real_sym)) = reexports.resolve_reexport_target(
                    &target,
                    &orig_sym,
                    lookup.known_files,
                    None,
                    crate_map_opt,
                )
            {
                target = real_target;
                orig_sym = real_sym;
            }

            let mut syms: Vec<&str> = Vec::new();
            for s in &effective[split_at..] {
                if *s != "self" && *s != "*" {
                    syms.push(*s);
                }
            }
            if syms.is_empty() && !orig_sym.is_empty() && orig_sym != "*" {
                syms.push(&orig_sym);
            }
            let local_name = imp.alias.as_deref().unwrap_or_else(|| {
                if last == "self" {
                    effective.last().copied().unwrap_or("")
                } else {
                    last
                }
            });
            if !local_name.is_empty() && local_name != "*" && local_name != "self" {
                aliases.insert(
                    local_name.to_owned(),
                    (Some(target.clone()), orig_sym.clone()),
                );
            }
            acc.add_resolved(target, "imports", &syms, imp.line, &imp.evidence);
        } else {
            let unres_target = if effective.len() >= 2 {
                format!("{}::{}", effective[0], effective[1])
            } else {
                effective.join("::")
            };
            let syms: Vec<&str> = if !imp.is_wildcard && last != "self" {
                vec![last]
            } else {
                Vec::new()
            };
            acc.add_unresolved(&unres_target, "imports", &syms, imp.line, &imp.evidence);
        }
    }

    let resolve_rs_sym = |raw: &str| -> Option<(String, String)> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.contains("::") {
            let segs: Vec<&str> = trimmed
                .split("::")
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            if matches!(segs.first().copied(), Some("crate" | "super" | "self"))
                && let Some((mut target, _)) = resolve_rust_path_segments_with_crates(
                    rel_path,
                    &crate_src_root,
                    &segs,
                    lookup,
                    crate_map_opt,
                )
            {
                let mut sym = segs.last().copied().unwrap_or("").to_owned();
                if let Some(reexports) = reexports_opt
                    && let Some((rt, rs)) = reexports.resolve_reexport_target(
                        &target,
                        &sym,
                        lookup.known_files,
                        None,
                        crate_map_opt,
                    )
                {
                    target = rt;
                    sym = rs;
                }
                return Some((target, sym));
            }
            if let Some(head) = segs.first()
                && let Some((Some(target), _)) = aliases.get(*head)
            {
                return Some((
                    target.clone(),
                    segs.last().copied().unwrap_or("").to_owned(),
                ));
            }
            return None;
        }
        if let Some((Some(target), orig)) = aliases.get(trimmed) {
            return Some((target.clone(), orig.clone()));
        }
        if RUST_BUILTINS.contains(&trimmed) {
            return None;
        }
        if let Some(reexports) = reexports_opt {
            for facade in &wildcard_facades {
                if let Some((rt, rs)) = reexports.resolve_reexport_target(
                    facade,
                    trimmed,
                    lookup.known_files,
                    None,
                    crate_map_opt,
                ) {
                    return Some((rt, rs));
                }
            }
        }
        let target = lookup
            .resolve_stem_in_dir(&dir, trimmed, RUST_EXTS)
            .or_else(|| {
                lookup.resolve_unambiguous_stem(
                    trimmed,
                    RUST_EXTS,
                    (!crate_src_root.is_empty()).then_some(crate_src_root.as_str()),
                )
            })?;
        Some((target, trimmed.to_owned()))
    };

    for inh in &ir.inheritance {
        if let Some((target, sym)) = resolve_rs_sym(&inh.parent) {
            acc.add_resolved(target, &inh.kind, &[&sym], inh.line, &inh.evidence);
        }
    }

    for r in &ir.calls_and_refs {
        if r.context == "call"
            && let Some(ref mem) = r.member
            && is_rust_std_trait_method(mem)
        {
            continue;
        }
        // For qualified paths like `crate::security::signer::Signer::new`, resolve owner part first.
        let lookup_expr = if r.is_static && r.member.is_some() && r.target_name.contains("::") {
            r.target_name
                .rsplit_once("::")
                .map(|(scope, _)| scope)
                .unwrap_or(&r.target_name)
        } else {
            r.target_name.as_str()
        };
        let Some((target, canon)) = resolve_rs_sym(lookup_expr) else {
            continue;
        };
        let parent_opt = r.owner_type.clone().or_else(|| {
            canon
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_uppercase())
                .then(|| canon.clone())
        });
        let (dep_kind, members) = ref_dep_kind_and_member(r, &canon, "field", parent_opt, false);
        let mut syms = vec![canon.as_str()];
        for m in &members {
            if !m.name.is_empty() {
                syms.push(&m.name);
            }
        }
        acc.add_resolved_members(target, dep_kind, &syms, &members, r.line, &r.evidence);
    }
}

fn rust_crate_src_root(rel_path: &str) -> String {
    if let Some((prefix, _)) = rel_path.rsplit_once("/src/") {
        return format!("{prefix}/src");
    }
    if rel_path.starts_with("src/") {
        return "src".to_owned();
    }
    if let Some((prefix, _)) = rel_path.rsplit_once("/tests/") {
        return format!("{prefix}/src");
    }
    if rel_path.starts_with("tests/") {
        return "src".to_owned();
    }
    String::new()
}

fn resolve_rust_child_mod(rel_path: &str, mod_name: &str, lookup: &KnownLookup) -> Option<String> {
    let dir = folder_of(rel_path);
    let stem = file_stem(rel_path);
    let sub_dir = if matches!(stem, "lib" | "main" | "mod") {
        dir.clone()
    } else if dir.is_empty() {
        stem.to_owned()
    } else {
        format!("{dir}/{stem}")
    };
    for base in [&sub_dir, &dir] {
        for cand in [
            join_rel(base, &format!("{mod_name}.rs")),
            join_rel(base, &format!("{mod_name}/mod.rs")),
        ] {
            if let Some(found) = lookup.resolve_exact_or_ci(&cand) {
                return Some(found);
            }
        }
    }
    None
}

fn resolve_rust_path_segments_with_crates(
    rel_path: &str,
    crate_src_root: &str,
    segs: &[&str],
    lookup: &KnownLookup,
    crate_map_opt: Option<&CratePackageMap>,
) -> Option<(String, usize)> {
    if segs.is_empty() {
        return None;
    }
    let (base_dir, start_idx) = match segs[0] {
        "crate" => (crate_src_root.to_owned(), 1usize),
        "self" => {
            let dir = folder_of(rel_path);
            let stem = file_stem(rel_path);
            let base = if matches!(stem, "lib" | "main" | "mod") {
                dir
            } else {
                join_rel(&dir, stem)
            };
            (base, 1usize)
        }
        "super" => {
            let mut cur = folder_of(rel_path);
            let stem = file_stem(rel_path);
            let mut idx = 0usize;
            while segs.get(idx) == Some(&"super") {
                if idx == 0 && !matches!(stem, "lib" | "main" | "mod") {
                    // In `src/a/b.rs`, first `super` is `src/a`.
                } else {
                    cur = folder_of(&cur);
                }
                idx += 1;
            }
            (cur, idx)
        }
        pkg => {
            let rest = &segs[1..];
            if let Some(crate_map) = crate_map_opt
                && let Some(found) = crate_map.resolve_crate_module(pkg, rest, lookup.known_files)
            {
                return Some(found);
            }
            let pkg_hyphen = pkg.replace('_', "-");
            for k in (1..=rest.len()).rev() {
                let rel_mod = rest[..k].join("/");
                for pkg_name in [&pkg_hyphen, &pkg.to_owned()] {
                    for suffix in [
                        format!("{pkg_name}/src/{rel_mod}.rs"),
                        format!("{pkg_name}/src/{rel_mod}/mod.rs"),
                    ] {
                        if let Some(found) = lookup.resolve_suffix(&suffix, Some(RUST_EXTS)) {
                            return Some((found, 1 + k));
                        }
                    }
                }
            }
            if rest.is_empty() {
                return None;
            }
            for suffix in [
                format!("{pkg_hyphen}/src/lib.rs"),
                format!("{pkg}/src/lib.rs"),
            ] {
                if let Some(found) = lookup.resolve_suffix(&suffix, Some(RUST_EXTS)) {
                    return Some((found, 1));
                }
            }
            return None;
        }
    };
    let rest = &segs[start_idx..];
    for k in (1..=rest.len()).rev() {
        let mod_rel = rest[..k].join("/");
        for cand in [
            join_rel(&base_dir, &format!("{mod_rel}.rs")),
            join_rel(&base_dir, &format!("{mod_rel}/mod.rs")),
        ] {
            if let Some(found) = lookup.resolve_exact_or_ci(&cand) {
                return Some((found, start_idx + k));
            }
        }
    }
    for fallback in [
        join_rel(&base_dir, "mod.rs"),
        join_rel(&base_dir, "lib.rs"),
        format!("{base_dir}.rs"),
    ] {
        if let Some(found) = lookup.resolve_exact_or_ci(&fallback) {
            return Some((found, start_idx));
        }
    }
    None
}

fn is_rust_std_trait_method(name: &str) -> bool {
    matches!(
        name,
        "clone"
            | "into"
            | "to_owned"
            | "to_string"
            | "unwrap"
            | "unwrap_or"
            | "unwrap_or_else"
            | "expect"
            | "is_some"
            | "is_none"
            | "is_ok"
            | "is_err"
            | "as_ref"
            | "as_mut"
            | "fmt"
            | "eq"
            | "cmp"
            | "partial_cmp"
            | "hash"
    )
}

// =========================================================================
// 3. TS / JS AST Resolution
// =========================================================================

fn resolve_ts_js_ir(
    rel_path: &str,
    ir: &AstFileExtraction,
    lookup: &KnownLookup,
    manifests_opt: Option<&ManifestIndex>,
    reexports_opt: Option<&ReexportIndex>,
    acc: &mut DepAccumulator,
) {
    let dir = folder_of(rel_path);
    let mut aliases: BTreeMap<String, (Option<String>, String)> = BTreeMap::new();

    for imp in &ir.imports {
        let mut resolved = resolve_ts_js_spec(&dir, &imp.path, lookup)
            .or_else(|| {
                manifests_opt
                    .and_then(|m| m.resolve_ts_import(rel_path, &imp.path, lookup.known_files))
            })
            .or_else(|| resolve_ts_js_alias_fallback(&imp.path, lookup));

        if imp.is_wildcard {
            if let Some(ns_alias) = imp.alias.as_deref() {
                aliases.insert(ns_alias.to_owned(), (resolved.clone(), "*".to_owned()));
            }
        } else {
            for sym in &imp.symbols {
                let mut target_opt = resolved.clone();
                let mut orig_sym = sym.clone();
                if let Some(ref facade) = target_opt
                    && let Some(reexports) = reexports_opt
                    && let Some((rt, rs)) = reexports.resolve_reexport_target(
                        facade,
                        sym,
                        lookup.known_files,
                        manifests_opt,
                        None,
                    )
                {
                    target_opt = Some(rt.clone());
                    orig_sym = rs;
                    resolved = Some(rt);
                }
                let local = imp.alias.as_deref().unwrap_or(sym);
                aliases.insert(local.to_owned(), (target_opt, orig_sym));
            }
        }
        let sym_refs: Vec<&str> = imp.symbols.iter().map(String::as_str).collect();
        match resolved {
            Some(t) => acc.add_resolved(t, "imports", &sym_refs, imp.line, &imp.evidence),
            None => acc.add_unresolved(&imp.path, "imports", &sym_refs, imp.line, &imp.evidence),
        }
    }

    let resolve_owner = |name: &str| -> Option<(String, String, bool)> {
        let clean = name.split('<').next().unwrap_or(name).trim();
        if clean.is_empty() {
            return None;
        }
        if let Some((Some(t), orig)) = aliases.get(clean) {
            let is_ns = orig == "*";
            let canon = if is_ns { clean } else { orig.as_str() };
            return Some((t.clone(), canon.to_owned(), is_ns));
        }
        lookup
            .resolve_stem_in_dir(&dir, clean, TS_JS_EXTS)
            .map(|t| (t, clean.to_owned(), false))
    };

    for inh in &ir.inheritance {
        if let Some((target, sym, _)) = resolve_owner(&inh.parent) {
            acc.add_resolved(target, &inh.kind, &[&sym], inh.line, &inh.evidence);
        }
    }

    for r in &ir.calls_and_refs {
        let Some((target, canon, is_ns)) = resolve_owner(&r.target_name) else {
            continue;
        };
        let is_class = !is_ns && canon.chars().next().is_some_and(|c| c.is_ascii_uppercase());
        let parent_opt = if is_class {
            r.owner_type.clone().or_else(|| Some(canon.clone()))
        } else {
            None
        };
        let mut adjusted = r.clone();
        if r.member.is_some() && !is_class && r.context == "call" {
            adjusted.member_kind = Some("function".to_owned());
        }
        let (dep_kind, members) =
            ref_dep_kind_and_member(&adjusted, &canon, "property", parent_opt, false);
        let mut syms = Vec::new();
        if is_ns && let Some(ref mem) = r.member {
            syms.push(mem.as_str());
        } else {
            syms.push(canon.as_str());
            if let Some(ref mem) = r.member
                && mem != &canon
            {
                syms.push(mem.as_str());
            }
        }
        acc.add_resolved_members(target, dep_kind, &syms, &members, r.line, &r.evidence);
    }
}

fn resolve_ts_js_spec(dir: &str, spec: &str, lookup: &KnownLookup) -> Option<String> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("./") || trimmed.starts_with("../") {
        let base = resolve_relative_segments(dir, trimmed)?;
        return resolve_ts_js_candidates(&base, lookup);
    }
    None
}

fn resolve_ts_js_alias_fallback(spec: &str, lookup: &KnownLookup) -> Option<String> {
    let trimmed = spec.trim();
    if let Some(alias_rest) = trimmed
        .strip_prefix("@/")
        .or_else(|| trimmed.strip_prefix("~/"))
    {
        for prefix in ["src", "app", ""] {
            let cand = join_rel(prefix, alias_rest);
            if let Some(found) = resolve_ts_js_candidates(&cand, lookup) {
                return Some(found);
            }
        }
        return resolve_ts_js_suffix(alias_rest, lookup);
    }
    None
}

fn resolve_ts_js_candidates(base: &str, lookup: &KnownLookup) -> Option<String> {
    if let Some(found) = lookup.resolve_exact_or_ci(base) {
        return Some(found);
    }
    if let Some((stem_path, ext)) = base.rsplit_once('.') {
        let lower_ext = ext.to_ascii_lowercase();
        let mapped: &[&str] = match lower_ext.as_str() {
            "js" => &["ts", "tsx", "js", "jsx"],
            "jsx" => &["tsx", "ts", "jsx", "js"],
            "mjs" => &["mts", "mjs"],
            "cjs" => &["cts", "cjs"],
            _ => &[],
        };
        for alt_ext in mapped {
            if let Some(found) = lookup.resolve_exact_or_ci(&format!("{stem_path}.{alt_ext}")) {
                return Some(found);
            }
        }
    }
    for ext in TS_JS_EXTS {
        if let Some(found) = lookup.resolve_exact_or_ci(&format!("{base}.{ext}")) {
            return Some(found);
        }
    }
    for ext in TS_JS_EXTS {
        if let Some(found) = lookup.resolve_exact_or_ci(&format!("{base}/index.{ext}")) {
            return Some(found);
        }
    }
    None
}

fn resolve_ts_js_suffix(rest: &str, lookup: &KnownLookup) -> Option<String> {
    if let Some(found) = lookup.resolve_suffix(rest, Some(TS_JS_EXTS)) {
        return Some(found);
    }
    for ext in TS_JS_EXTS {
        if let Some(found) = lookup.resolve_suffix(&format!("{rest}.{ext}"), Some(TS_JS_EXTS)) {
            return Some(found);
        }
        if let Some(found) = lookup.resolve_suffix(&format!("{rest}/index.{ext}"), Some(TS_JS_EXTS))
        {
            return Some(found);
        }
    }
    None
}

// =========================================================================
// 4. Python AST Resolution
// =========================================================================

fn resolve_python_ir(
    rel_path: &str,
    ir: &AstFileExtraction,
    lookup: &KnownLookup,
    manifests_opt: Option<&ManifestIndex>,
    acc: &mut DepAccumulator,
) {
    let dir = folder_of(rel_path);
    let mut aliases: BTreeMap<String, (Option<String>, String)> = BTreeMap::new();

    for imp in &ir.imports {
        let dots = imp.path.bytes().take_while(|b| *b == b'.').count();
        let dotted = &imp.path[dots..];

        if dots > 0 && dotted.is_empty() && !imp.symbols.is_empty() {
            for sym in &imp.symbols {
                if let Some(sub) = resolve_python_module(&dir, dots, sym, lookup) {
                    let local = imp.alias.as_deref().unwrap_or(sym);
                    aliases.insert(local.to_owned(), (Some(sub.clone()), sym.clone()));
                    acc.add_resolved(sub, "imports", &[sym], imp.line, &imp.evidence);
                } else if let Some(init) = resolve_python_module(&dir, dots, "", lookup) {
                    let local = imp.alias.as_deref().unwrap_or(sym);
                    aliases.insert(local.to_owned(), (Some(init.clone()), sym.clone()));
                    acc.add_resolved(init, "imports", &[sym], imp.line, &imp.evidence);
                } else {
                    acc.add_unresolved(
                        &format!("{}{sym}", imp.path),
                        "imports",
                        &[sym],
                        imp.line,
                        &imp.evidence,
                    );
                }
            }
            continue;
        }

        let resolved = if dots > 0 {
            resolve_python_module(&dir, dots, dotted, lookup)
        } else {
            manifests_opt
                .and_then(|m| m.resolve_py_import(rel_path, dotted, lookup.known_files))
                .or_else(|| resolve_python_module(&dir, 0, dotted, lookup))
        };

        if let Some(t) = resolved {
            let sym_refs: Vec<&str> = imp.symbols.iter().map(String::as_str).collect();
            if imp.symbols.is_empty() {
                let last_seg = dotted.rsplit('.').next().unwrap_or(dotted);
                let local = imp
                    .alias
                    .as_deref()
                    .unwrap_or_else(|| dotted.split('.').next().unwrap_or(dotted));
                aliases.insert(local.to_owned(), (Some(t.clone()), last_seg.to_owned()));
            } else {
                for sym in &imp.symbols {
                    let local = imp.alias.as_deref().unwrap_or(sym);
                    aliases.insert(local.to_owned(), (Some(t.clone()), sym.clone()));
                }
            }
            acc.add_resolved(t, "imports", &sym_refs, imp.line, &imp.evidence);
        } else {
            let mut unresolved = Vec::new();
            for sym in &imp.symbols {
                if !dotted.is_empty()
                    && let Some(sub) =
                        resolve_python_module(&dir, dots, &format!("{dotted}.{sym}"), lookup)
                {
                    let local = imp.alias.as_deref().unwrap_or(sym);
                    aliases.insert(local.to_owned(), (Some(sub.clone()), sym.clone()));
                    acc.add_resolved(sub, "imports", &[sym], imp.line, &imp.evidence);
                } else {
                    unresolved.push(sym.as_str());
                }
            }
            if !unresolved.is_empty() || imp.is_wildcard || imp.symbols.is_empty() {
                acc.add_unresolved(&imp.path, "imports", &unresolved, imp.line, &imp.evidence);
            }
        }
    }

    let resolve_py_owner = |raw: &str| -> Option<(String, String, bool)> {
        if let Some((head, tail)) = raw.rsplit_once('.') {
            let root = head.split('.').next().unwrap_or(head);
            if let Some((Some(t), _)) = aliases.get(root) {
                return Some((t.clone(), tail.to_owned(), false));
            }
        }
        if let Some((Some(t), orig)) = aliases.get(raw) {
            let is_mod = orig == file_stem(t);
            let canon = if is_mod { raw.to_owned() } else { orig.clone() };
            return Some((t.clone(), canon, is_mod));
        }
        lookup
            .resolve_stem_in_dir(&dir, raw, PYTHON_EXTS)
            .map(|t| (t, raw.to_owned(), false))
    };

    for inh in &ir.inheritance {
        let head = inh.parent.split('.').next().unwrap_or(&inh.parent);
        if let Some((t, sym, _)) = resolve_py_owner(head) {
            let short = inh.parent.rsplit('.').next().unwrap_or(&sym);
            acc.add_resolved(t, &inh.kind, &[short], inh.line, &inh.evidence);
        }
    }

    for r in &ir.calls_and_refs {
        let Some((target, canon, is_mod)) = resolve_py_owner(&r.target_name) else {
            continue;
        };
        let is_class = !is_mod && canon.chars().next().is_some_and(|c| c.is_ascii_uppercase());
        let parent_opt = if is_class {
            r.owner_type.clone().or_else(|| Some(canon.clone()))
        } else {
            None
        };
        let mut adjusted = r.clone();
        if r.member.is_some() && !is_class && r.context == "call" {
            adjusted.member_kind = Some("function".to_owned());
        }
        let (dep_kind, members) =
            ref_dep_kind_and_member(&adjusted, &canon, "property", parent_opt, false);
        let mut syms = vec![canon.as_str()];
        if let Some(ref mem) = r.member
            && mem != &canon
        {
            syms.push(mem.as_str());
        }
        acc.add_resolved_members(target, dep_kind, &syms, &members, r.line, &r.evidence);
    }
}

fn resolve_python_module(
    dir: &str,
    dots: usize,
    dotted: &str,
    lookup: &KnownLookup,
) -> Option<String> {
    let rel_mod = dotted.replace('.', "/");
    if dots > 0 {
        let mut cur = dir.to_owned();
        for _ in 1..dots {
            cur = folder_of(&cur);
        }
        let base = if rel_mod.is_empty() {
            cur
        } else {
            join_rel(&cur, &rel_mod)
        };
        return resolve_python_candidates(&base, lookup);
    }
    if rel_mod.is_empty() {
        return None;
    }
    if let Some(found) = resolve_python_candidates(&rel_mod, lookup) {
        return Some(found);
    }
    for ext in PYTHON_EXTS {
        if let Some(found) = lookup.resolve_suffix(&format!("{rel_mod}.{ext}"), Some(PYTHON_EXTS)) {
            return Some(found);
        }
        if let Some(found) =
            lookup.resolve_suffix(&format!("{rel_mod}/__init__.{ext}"), Some(PYTHON_EXTS))
        {
            return Some(found);
        }
    }
    None
}

fn resolve_python_candidates(base: &str, lookup: &KnownLookup) -> Option<String> {
    for ext in PYTHON_EXTS {
        if let Some(found) = lookup.resolve_exact_or_ci(&format!("{base}.{ext}")) {
            return Some(found);
        }
    }
    for ext in PYTHON_EXTS {
        if let Some(found) = lookup.resolve_exact_or_ci(&join_rel(base, &format!("__init__.{ext}")))
        {
            return Some(found);
        }
    }
    None
}

// =========================================================================
// 5. Go, Java/Kotlin/Scala, C#, Ruby, Swift AST Resolution
// =========================================================================

const MAX_GO_PACKAGE_FILES: usize = 8;

fn resolve_go_ir(
    rel_path: &str,
    ir: &AstFileExtraction,
    lookup: &KnownLookup,
    manifests_opt: Option<&ManifestIndex>,
    acc: &mut DepAccumulator,
) {
    let dir = folder_of(rel_path);
    let mut pkg_aliases: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for imp in &ir.imports {
        let mut pkg_files = manifests_opt
            .map(|m| m.resolve_go_import_files(&imp.path, lookup.known_files))
            .unwrap_or_default();
        if pkg_files.is_empty() {
            pkg_files = resolve_go_package_suffix_files(&imp.path, lookup);
        }
        let pkg_name = imp
            .alias
            .clone()
            .or_else(|| imp.symbols.first().cloned())
            .unwrap_or_else(|| imp.path.rsplit('/').next().unwrap_or(&imp.path).to_owned());
        if pkg_files.is_empty() {
            acc.add_unresolved(&imp.path, "imports", &[&pkg_name], imp.line, &imp.evidence);
        } else {
            pkg_files.truncate(MAX_GO_PACKAGE_FILES);
            for t in &pkg_files {
                acc.add_resolved(t.clone(), "imports", &[&pkg_name], imp.line, &imp.evidence);
            }
            pkg_aliases.insert(pkg_name, pkg_files);
        }
    }
    for r in &ir.calls_and_refs {
        let targets = if let Some(pkg_files) = pkg_aliases.get(&r.target_name) {
            if let Some(ref mem) = r.member {
                let mem_lower = mem.to_ascii_lowercase();
                let narrowed: Vec<String> = pkg_files
                    .iter()
                    .filter(|p| {
                        let stem = file_stem(p).to_ascii_lowercase();
                        !stem.is_empty() && (mem_lower.contains(&stem) || stem.contains(&mem_lower))
                    })
                    .cloned()
                    .collect();
                if narrowed.is_empty() {
                    pkg_files.clone()
                } else {
                    narrowed
                }
            } else {
                pkg_files.clone()
            }
        } else {
            lookup
                .resolve_stem_in_dir(&dir, &r.target_name, GO_EXTS)
                .into_iter()
                .collect()
        };
        if !targets.is_empty() {
            let (dep_kind, members) =
                ref_dep_kind_and_member(r, &r.target_name, "field", None, false);
            let mut syms = vec![r.target_name.as_str()];
            if let Some(ref mem) = r.member {
                syms.push(mem.as_str());
            }
            for t in targets {
                acc.add_resolved_members(t, dep_kind, &syms, &members, r.line, &r.evidence);
            }
        }
    }
}

fn resolve_go_package_suffix_files(import_path: &str, lookup: &KnownLookup) -> Vec<String> {
    let cleaned = clean_rel_key(import_path);
    if cleaned.is_empty() {
        return Vec::new();
    }
    let segs: Vec<&str> = cleaned.split('/').filter(|s| !s.is_empty()).collect();
    for start in 0..segs.len() {
        let sub_dir = segs[start..].join("/");
        let mut matches: Vec<String> = lookup
            .all_entries()
            .iter()
            .filter(|(_, c, _)| {
                has_any_ext(c, GO_EXTS)
                    && (folder_of(c) == sub_dir || folder_of(c).ends_with(&format!("/{sub_dir}")))
            })
            .map(|(orig, _, _)| orig.clone())
            .collect();
        matches.sort();
        let non_test: Vec<String> = matches
            .iter()
            .filter(|p| !p.ends_with("_test.go"))
            .cloned()
            .collect();
        if !non_test.is_empty() {
            return non_test;
        }
        if !matches.is_empty() {
            return matches;
        }
    }
    Vec::new()
}

fn resolve_jvm_ir(
    rel_path: &str,
    ir: &AstFileExtraction,
    lang: Language,
    lookup: &KnownLookup,
    acc: &mut DepAccumulator,
) {
    let exts = match lang {
        Language::Java => JAVA_EXTS,
        Language::Kotlin => KOTLIN_EXTS,
        Language::Scala => SCALA_EXTS,
        _ => JAVA_EXTS,
    };
    let dir = folder_of(rel_path);
    let mut symbol_targets: BTreeMap<String, String> = BTreeMap::new();
    let mut wildcard_pkgs: Vec<String> = Vec::new();

    for imp in &ir.imports {
        let slash = imp.path.replace('.', "/");
        if imp.is_wildcard {
            wildcard_pkgs.push(slash);
            acc.add_unresolved(&imp.path, "imports", &[], imp.line, &imp.evidence);
            continue;
        }
        let mut resolved = None;
        for ext in exts {
            if let Some(found) = lookup.resolve_suffix(&format!("{slash}.{ext}"), Some(exts)) {
                resolved = Some(found);
                break;
            }
        }
        let sym = imp
            .symbols
            .first()
            .cloned()
            .unwrap_or_else(|| imp.path.rsplit('.').next().unwrap_or(&imp.path).to_owned());
        match resolved {
            Some(t) => {
                let local = imp.alias.clone().unwrap_or_else(|| sym.clone());
                symbol_targets.insert(local, t.clone());
                acc.add_resolved(t, "imports", &[&sym], imp.line, &imp.evidence);
            }
            None => acc.add_unresolved(&imp.path, "imports", &[&sym], imp.line, &imp.evidence),
        }
    }

    let resolve_jvm_sym = |name: &str| -> Option<String> {
        let short = name.split('<').next().unwrap_or(name).trim();
        if let Some(t) = symbol_targets.get(short) {
            return Some(t.clone());
        }
        for pkg in &wildcard_pkgs {
            for ext in exts {
                if let Some(found) =
                    lookup.resolve_suffix(&format!("{pkg}/{short}.{ext}"), Some(exts))
                {
                    return Some(found);
                }
            }
        }
        lookup.resolve_stem_in_dir(&dir, short, exts)
    };

    for inh in &ir.inheritance {
        if let Some(t) = resolve_jvm_sym(&inh.parent) {
            acc.add_resolved(t, &inh.kind, &[&inh.parent], inh.line, &inh.evidence);
        }
    }
    for r in &ir.calls_and_refs {
        if let Some(t) = resolve_jvm_sym(&r.target_name) {
            let (dep_kind, members) = ref_dep_kind_and_member(
                r,
                &r.target_name,
                "field",
                Some(r.target_name.clone()),
                false,
            );
            let mut syms = vec![r.target_name.as_str()];
            if let Some(ref mem) = r.member {
                syms.push(mem.as_str());
            }
            acc.add_resolved_members(t, dep_kind, &syms, &members, r.line, &r.evidence);
        }
    }
}

fn resolve_csharp_ir(
    rel_path: &str,
    ir: &AstFileExtraction,
    lookup: &KnownLookup,
    acc: &mut DepAccumulator,
) {
    let dir = folder_of(rel_path);
    let mut symbol_targets: BTreeMap<String, String> = BTreeMap::new();
    let mut using_ns: Vec<String> = Vec::new();

    for imp in &ir.imports {
        let slash = imp.path.replace('.', "/");
        if let Some(ref alias) = imp.alias {
            if let Some(t) = lookup.resolve_suffix(&format!("{slash}.cs"), Some(CSHARP_EXTS)) {
                symbol_targets.insert(alias.clone(), t.clone());
                acc.add_resolved(t, "imports", &[alias], imp.line, &imp.evidence);
            } else {
                acc.add_unresolved(&imp.path, "imports", &[alias], imp.line, &imp.evidence);
            }
        } else {
            using_ns.push(slash);
            acc.add_unresolved(&imp.path, "imports", &[], imp.line, &imp.evidence);
        }
    }

    let resolve_cs_sym = |name: &str| -> Option<String> {
        let short = name.split('<').next().unwrap_or(name).trim();
        if let Some(t) = symbol_targets.get(short) {
            return Some(t.clone());
        }
        for ns in &using_ns {
            if let Some(t) = lookup.resolve_suffix(&format!("{ns}/{short}.cs"), Some(CSHARP_EXTS)) {
                return Some(t);
            }
        }
        lookup.resolve_stem_in_dir(&dir, short, CSHARP_EXTS)
    };

    for inh in &ir.inheritance {
        if let Some(t) = resolve_cs_sym(&inh.parent) {
            acc.add_resolved(t, &inh.kind, &[&inh.parent], inh.line, &inh.evidence);
        }
    }
    for r in &ir.calls_and_refs {
        if let Some(t) = resolve_cs_sym(&r.target_name) {
            let (dep_kind, members) = ref_dep_kind_and_member(
                r,
                &r.target_name,
                "property",
                Some(r.target_name.clone()),
                false,
            );
            let mut syms = vec![r.target_name.as_str()];
            if let Some(ref mem) = r.member {
                syms.push(mem.as_str());
            }
            acc.add_resolved_members(t, dep_kind, &syms, &members, r.line, &r.evidence);
        }
    }
}

fn resolve_ruby_ir(
    rel_path: &str,
    ir: &AstFileExtraction,
    lookup: &KnownLookup,
    acc: &mut DepAccumulator,
) {
    let dir = folder_of(rel_path);
    for imp in &ir.imports {
        let with_ext = if imp.path.ends_with(".rb") {
            imp.path.clone()
        } else {
            format!("{}.rb", imp.path)
        };
        let resolved = if imp.kind == "include" {
            lookup.resolve_relative(&dir, &with_ext)
        } else {
            lookup.resolve_suffix(&with_ext, Some(RUBY_EXTS))
        };
        let sym = file_stem(&with_ext);
        match resolved {
            Some(t) => acc.add_resolved(t, "imports", &[sym], imp.line, &imp.evidence),
            None => acc.add_unresolved(&imp.path, "imports", &[sym], imp.line, &imp.evidence),
        }
    }
    for inh in &ir.inheritance {
        let short = inh.parent.rsplit("::").next().unwrap_or(&inh.parent);
        if let Some(t) = lookup
            .resolve_stem_in_dir(&dir, short, RUBY_EXTS)
            .or_else(|| lookup.resolve_unambiguous_stem(short, RUBY_EXTS, None))
        {
            acc.add_resolved(t, &inh.kind, &[short], inh.line, &inh.evidence);
        }
    }
    for r in &ir.calls_and_refs {
        let short = r.target_name.rsplit("::").next().unwrap_or(&r.target_name);
        if let Some(t) = lookup
            .resolve_stem_in_dir(&dir, short, RUBY_EXTS)
            .or_else(|| lookup.resolve_unambiguous_stem(short, RUBY_EXTS, None))
        {
            let (dep_kind, members) =
                ref_dep_kind_and_member(r, short, "property", Some(short.to_owned()), false);
            let mut syms = vec![short];
            if let Some(ref mem) = r.member {
                syms.push(mem.as_str());
            }
            acc.add_resolved_members(t, dep_kind, &syms, &members, r.line, &r.evidence);
        }
    }
}

fn resolve_swift_ir(
    rel_path: &str,
    ir: &AstFileExtraction,
    lookup: &KnownLookup,
    acc: &mut DepAccumulator,
) {
    let dir = folder_of(rel_path);
    for imp in &ir.imports {
        match lookup.resolve_swift_module(&imp.path) {
            Some(t) => acc.add_resolved(t, "imports", &[&imp.path], imp.line, &imp.evidence),
            None => acc.add_unresolved(&imp.path, "imports", &[&imp.path], imp.line, &imp.evidence),
        }
    }
    for inh in &ir.inheritance {
        let short = inh.parent.split('<').next().unwrap_or(&inh.parent).trim();
        if let Some(t) = lookup
            .resolve_stem_in_dir(&dir, short, SWIFT_EXTS)
            .or_else(|| lookup.resolve_unambiguous_stem(short, SWIFT_EXTS, None))
        {
            acc.add_resolved(t, &inh.kind, &[short], inh.line, &inh.evidence);
        }
    }
    for r in &ir.calls_and_refs {
        let short = r.target_name.split('.').next().unwrap_or(&r.target_name);
        if let Some(t) = lookup
            .resolve_stem_in_dir(&dir, short, SWIFT_EXTS)
            .or_else(|| lookup.resolve_unambiguous_stem(short, SWIFT_EXTS, None))
        {
            let (dep_kind, members) =
                ref_dep_kind_and_member(r, short, "property", Some(short.to_owned()), false);
            let mut syms = vec![short];
            if let Some(ref mem) = r.member {
                syms.push(mem.as_str());
            }
            acc.add_resolved_members(t, dep_kind, &syms, &members, r.line, &r.evidence);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn diff_entities_tracks_created_modified_signature_and_deleted_across_languages() {
        let before_php = r#"<?php
class JwtGuard {
    public function verify(string $token): bool {
        return $token !== '';
    }
    public function obsolete(): void {}
}
"#;
        let after_php = r#"<?php
// Line-shift comment should not mark untouched classes modified
class JwtGuard {
    public function verify(string $token, bool $strict): bool {
        return $token !== '';
    }
    public function issue(): string {
        return 'tok';
    }
}
"#;
        let changes = diff_entities("src/Auth/JwtGuard.php", Some(before_php), Some(after_php));
        assert_eq!(changes.len(), 3, "{changes:?}");
        assert_eq!(changes[0].name, "verify");
        assert_eq!(changes[0].change, "modified");
        assert!(changes[0].signature_changed);
        assert_eq!(changes[0].parent.as_deref(), Some("JwtGuard"));

        assert_eq!(changes[1].name, "obsolete");
        assert_eq!(changes[1].change, "deleted");
        assert!(!changes[1].signature_changed);

        assert_eq!(changes[2].name, "issue");
        assert_eq!(changes[2].change, "created");
        assert!(!changes[2].signature_changed);

        // Body-only modification keeps `signature_changed == false`.
        let before_rs = "pub fn compute(x: i32) -> i32 {\n    x + 1\n}\n";
        let after_rs = "pub fn compute(x: i32) -> i32 {\n    x * 2\n}\n";
        let rs_changes = diff_entities("src/math.rs", Some(before_rs), Some(after_rs));
        assert_eq!(rs_changes.len(), 1);
        assert_eq!(rs_changes[0].change, "modified");
        assert!(!rs_changes[0].signature_changed);

        // Non-code & empty files never panic and return empty.
        assert!(diff_entities("README.md", Some("# Hi"), Some("# Bye")).is_empty());
        assert!(diff_entities("empty.rs", Some(""), None).is_empty());
    }

    #[test]
    fn diff_entities_detects_ts_and_python_body_and_signature_changes() {
        let before_ts = r#"
export class TokenManager {
    verify(raw: string): boolean {
        return raw.length > 0;
    }
}
export const helper = (n: number): number => n + 1;
"#;
        let after_ts = r#"
export class TokenManager {
    verify(raw: string): boolean {
        return raw.trim().length > 0;
    }
}
export const helper = (n: number, scale: number): number => n + 1;
"#;
        let ts_changes = diff_entities("web/token.ts", Some(before_ts), Some(after_ts));
        assert_eq!(ts_changes.len(), 2, "{ts_changes:?}");
        assert_eq!(ts_changes[0].name, "verify");
        assert_eq!(ts_changes[0].change, "modified");
        assert!(!ts_changes[0].signature_changed);
        assert_eq!(ts_changes[1].name, "helper");
        assert_eq!(ts_changes[1].change, "modified");
        assert!(ts_changes[1].signature_changed);

        let before_py =
            "class Guard:\n    def check(self, tok: str) -> bool:\n        return bool(tok)\n";
        let after_py = "class Guard:\n    def check(self, tok: str, strict: bool) -> bool:\n        return bool(tok)\n";
        let py_changes = diff_entities("auth/guard.py", Some(before_py), Some(after_py));
        assert_eq!(py_changes.len(), 1, "{py_changes:?}");
        assert_eq!(py_changes[0].name, "check");
        assert_eq!(py_changes[0].change, "modified");
        assert!(py_changes[0].signature_changed);
    }

    #[test]
    fn extract_file_deps_resolves_php_psr4_same_ns_inheritance_calls_and_includes() {
        let files = known(&[
            "src/Auth/JwtGuard.php",
            "src/Auth/BaseGuard.php",
            "src/Auth/AuditTrait.php",
            "src/Security/Signer.php",
            "src/Contracts/GuardInterface.php",
            "src/bootstrap.php",
        ]);
        let php = r#"<?php
namespace App\Auth;

require_once __DIR__ . '/../bootstrap.php';

use App\Security\Signer as TokenSigner;
use App\Contracts\GuardInterface;
use External\Vendor\Logger;

class JwtGuard extends BaseGuard implements GuardInterface {
    use AuditTrait;

    public function __construct(private TokenSigner $signer) {}

    public function handle(string $tok): bool {
        $s = new TokenSigner();
        return TokenSigner::verify($tok) && $s->check($tok);
    }
}
"#;
        let deps = extract_file_deps("src/Auth/JwtGuard.php", php, &files);

        let has = |target: &str, kind: &str, resolved: bool| {
            deps.iter()
                .find(|d| d.target == target && d.kind == kind && d.resolved == resolved)
        };
        let bootstrap = has("src/bootstrap.php", "imports", true).expect("bootstrap import");
        assert!(bootstrap.exists);

        let signer_imp = has("src/Security/Signer.php", "imports", true).expect("signer import");
        assert!(signer_imp.symbols.contains(&"Signer".to_owned()));

        let signer_call = has("src/Security/Signer.php", "calls", true).expect("signer call");
        assert!(signer_call.symbols.contains(&"Signer".to_owned()));
        assert!(signer_call.symbols.contains(&"verify".to_owned()));

        assert!(has("src/Auth/BaseGuard.php", "extends", true).is_some());
        assert!(has("src/Contracts/GuardInterface.php", "implements", true).is_some());
        assert!(has("src/Auth/AuditTrait.php", "uses", true).is_some());

        let ext = has("External\\Vendor\\Logger", "imports", false).expect("unresolved external");
        assert!(!ext.exists);
    }

    #[test]
    fn extract_file_deps_resolves_rust_ts_and_python_imports() {
        let files = known(&[
            "crates/app/src/lib.rs",
            "crates/app/src/auth/mod.rs",
            "crates/app/src/auth/jwt.rs",
            "crates/app/src/security/signer.rs",
            "crates/infra-core/src/fs.rs",
            "web/src/components/App.tsx",
            "web/src/utils/auth.ts",
            "web/src/hooks/index.ts",
            "py/auth/__init__.py",
            "py/auth/guard.py",
            "py/auth/tokens.py",
        ]);

        let rust_src = r#"
use crate::security::signer::{Signer, verify_sig};
use super::auth::jwt::JwtService;
use infra_core::fs::python_realpath;
use std::collections::BTreeMap;

pub fn check() -> bool {
    Signer::new();
    verify_sig("a")
}
"#;
        let rs_deps = extract_file_deps("crates/app/src/lib.rs", rust_src, &files);
        assert!(
            rs_deps
                .iter()
                .any(|d| d.target == "crates/app/src/security/signer.rs"
                    && d.resolved
                    && d.exists
                    && d.kind == "imports")
        );
        assert!(
            rs_deps
                .iter()
                .any(|d| d.target == "crates/app/src/security/signer.rs" && d.kind == "calls")
        );
        assert!(
            rs_deps
                .iter()
                .any(|d| d.target == "crates/infra-core/src/fs.rs" && d.resolved)
        );
        assert!(
            rs_deps
                .iter()
                .any(|d| d.target == "std::collections" && !d.resolved && !d.exists)
        );

        let ts_src = r#"
import { verifyToken as checkTok } from '../utils/auth';
import { useAuth } from '@/hooks';
import React from 'react';

export function run() {
    return checkTok('x') && useAuth();
}
"#;
        let ts_deps = extract_file_deps("web/src/components/App.tsx", ts_src, &files);
        assert!(
            ts_deps
                .iter()
                .any(|d| d.target == "web/src/utils/auth.ts" && d.resolved && d.kind == "imports")
        );
        assert!(ts_deps.iter().any(|d| d.target == "web/src/utils/auth.ts"
            && d.kind == "calls"
            && d.symbols.contains(&"verifyToken".to_owned())));
        assert!(
            ts_deps
                .iter()
                .any(|d| d.target == "web/src/hooks/index.ts" && d.resolved)
        );
        assert!(
            ts_deps
                .iter()
                .any(|d| d.target == "react" && !d.resolved && !d.exists)
        );

        let py_src = r#"
from . import tokens
from py.auth.guard import SessionGuard

class CustomGuard(SessionGuard):
    pass

def boot():
    tokens.issue()
    return CustomGuard()
"#;
        let py_deps = extract_file_deps("py/auth/service.py", py_src, &files);
        assert!(
            py_deps
                .iter()
                .any(|d| d.target == "py/auth/tokens.py" && d.resolved && d.kind == "imports")
        );
        assert!(
            py_deps
                .iter()
                .any(|d| d.target == "py/auth/guard.py" && d.resolved && d.kind == "extends")
        );
    }

    #[test]
    fn diff_file_deps_splits_added_removed_and_unchanged() {
        let files = known(&[
            "src/Auth/Signer.php",
            "src/Auth/OldHelper.php",
            "src/Auth/NewCache.php",
        ]);
        let before = "<?php\nuse App\\Auth\\Signer;\nuse App\\Auth\\OldHelper;\n";
        let after = "<?php\nuse App\\Auth\\Signer;\nuse App\\Auth\\NewCache;\n";
        let diff = diff_file_deps("src/Auth/Controller.php", Some(before), Some(after), &files);

        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].target, "src/Auth/NewCache.php");
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.removed[0].target, "src/Auth/OldHelper.php");
        assert_eq!(diff.unchanged.len(), 1);
        assert_eq!(diff.unchanged[0].target, "src/Auth/Signer.php");
    }

    #[test]
    fn find_matching_references_finds_used_symbols_and_skips_unrelated_files() {
        let files = known(&[
            "src/Auth/JwtGuard.php",
            "src/Http/LoginController.php",
            "src/Billing/InvoiceService.php",
        ]);
        let caller = r#"<?php
namespace App\Http;
use App\Auth\JwtGuard;

class LoginController {
    public function login(JwtGuard $guard, string $token): bool {
        // verifyUnused mentioned only in comment
        return $guard->verifyToken($token);
    }
}
"#;
        let unrelated = r#"<?php
namespace App\Billing;
class InvoiceService {
    public function verifyToken(string $t): bool { return true; }
}
"#;
        let syms = vec![
            "JwtGuard".to_owned(),
            "verifyToken".to_owned(),
            "verifyUnused".to_owned(),
        ];

        let refs = find_matching_references(
            "src/Http/LoginController.php",
            caller,
            "src/Auth/JwtGuard.php",
            &syms,
            &files,
        );
        assert_eq!(refs, vec!["JwtGuard", "verifyToken"]);

        let no_refs = find_matching_references(
            "src/Billing/InvoiceService.php",
            unrelated,
            "src/Auth/JwtGuard.php",
            &syms,
            &files,
        );
        assert!(no_refs.is_empty(), "{no_refs:?}");
    }

    #[test]
    fn handles_non_code_grouped_imports_path_mods_and_aliased_references() {
        assert!(diff_entities("README.md", Some("# Old\n"), Some("# New\n")).is_empty());
        assert!(diff_entities("assets/blob.bin", None, Some("\0\x01\x02")).is_empty());

        let files = known(&[
            "src/Domain/Order.php",
            "src/Domain/Money.php",
            "src/Security/Signer.php",
            "src/Service.php",
            "crates/core/src/lib.rs",
            "crates/core/src/alt/custom.rs",
            "crates/core/src/sub.rs",
            "web/src/shared/fmt.ts",
            "web/src/lazy.ts",
            "web/src/app.ts",
            "pkg/common/helpers.py",
            "pkg/api/routes.py",
        ]);

        // PHP grouped `use` + aliased `find_matching_references`
        let php = r#"<?php
namespace App;
use App\Domain\{Order, Money as Cash};
use App\Security\Signer as TokenSigner;
class Service {
    public function total(Order $o): Cash {
        TokenSigner::verify('x');
        return new Cash();
    }
}
"#;
        let php_deps = extract_file_deps("src/Service.php", php, &files);
        assert!(
            php_deps
                .iter()
                .any(|d| d.target == "src/Domain/Order.php" && d.resolved && d.kind == "imports")
        );
        assert!(php_deps.iter().any(|d| d.target == "src/Domain/Money.php"
            && d.resolved
            && d.symbols.contains(&"Money".to_owned())));
        let alias_refs = find_matching_references(
            "src/Service.php",
            php,
            "src/Security/Signer.php",
            &["Signer".to_owned(), "verify".to_owned()],
            &files,
        );
        assert!(alias_refs.contains(&"Signer".to_owned()), "{alias_refs:?}");
        assert!(alias_refs.contains(&"verify".to_owned()), "{alias_refs:?}");

        // Rust #[path = "..."] mod + mod sub;
        let rs = r#"
#[path = "alt/custom.rs"]
mod custom;
mod sub;
"#;
        let rs_deps = extract_file_deps("crates/core/src/lib.rs", rs, &files);
        assert!(
            rs_deps
                .iter()
                .any(|d| d.target == "crates/core/src/alt/custom.rs" && d.resolved)
        );
        assert!(
            rs_deps
                .iter()
                .any(|d| d.target == "crates/core/src/sub.rs" && d.resolved)
        );

        // TS @/ alias + dynamic import()
        let ts = r#"
import { moneyFmt } from '@/shared/fmt';
const loadLazy = () => import('./lazy');
export function show(n: number) { return moneyFmt(n); }
"#;
        let ts_deps = extract_file_deps("web/src/app.ts", ts, &files);
        assert!(
            ts_deps
                .iter()
                .any(|d| d.target == "web/src/shared/fmt.ts" && d.resolved)
        );
        assert!(
            ts_deps
                .iter()
                .any(|d| d.target == "web/src/lazy.ts" && d.resolved)
        );

        // Python parent-relative `from ..common import helpers`
        let py = "from ..common import helpers\nhelpers.run()\n";
        let py_deps = extract_file_deps("pkg/api/routes.py", py, &files);
        assert!(
            py_deps
                .iter()
                .any(|d| d.target == "pkg/common/helpers.py" && d.resolved)
        );
    }

    #[test]
    fn visibility_rank_and_narrowing_rules() {
        assert_eq!(visibility_rank(None), 4);
        assert_eq!(visibility_rank(Some("public")), 4);
        assert_eq!(visibility_rank(Some("export")), 4);
        assert_eq!(visibility_rank(Some("pub(crate)")), 2);
        assert_eq!(visibility_rank(Some("internal")), 2);
        assert_eq!(visibility_rank(Some("protected(set)")), 2);
        assert_eq!(visibility_rank(Some("package")), 2);
        assert_eq!(visibility_rank(Some("protected")), 1);
        assert_eq!(visibility_rank(Some("private")), 0);
        assert_eq!(visibility_rank(Some("#private")), 0);
        assert_eq!(visibility_rank(Some("private(set)")), 1);

        assert!(is_visibility_narrowed(Some("public"), Some("protected")));
        assert!(is_visibility_narrowed(Some("public"), Some("private")));
        assert!(is_visibility_narrowed(Some("public"), Some("private(set)")));
        assert!(is_visibility_narrowed(
            Some("protected(set)"),
            Some("private(set)")
        ));
        assert!(is_visibility_narrowed(Some("protected"), Some("private")));
        assert!(is_visibility_narrowed(Some("pub(crate)"), Some("private")));
        assert!(!is_visibility_narrowed(Some("private"), Some("public")));
        assert!(!is_visibility_narrowed(Some("public"), Some("public")));
        assert!(!is_visibility_narrowed(
            Some("protected"),
            Some("protected")
        ));
    }

    #[test]
    fn diff_entities_visibility_change_excludes_params_signature() {
        let before = r#"<?php
class Signer {
    public function sign(string $payload): string {
        return $payload;
    }
}
"#;
        let narrowed = r#"<?php
class Signer {
    private function sign(string $payload): string {
        return $payload;
    }
}
"#;
        let widened = r#"<?php
class Signer {
    public function sign(string $payload): string {
        return $payload;
    }
}
"#;
        let sig_edit = r#"<?php
class Signer {
    private function sign(int $payload): string {
        return (string)$payload;
    }
}
"#;

        let diff_narrow = diff_entities("src/Signer.php", Some(before), Some(narrowed));
        let sign_narrow = diff_narrow
            .iter()
            .find(|c| c.name == "sign")
            .expect("sign change");
        assert!(sign_narrow.visibility_changed);
        assert!(sign_narrow.visibility_narrowed);
        assert!(!sign_narrow.signature_changed);
        assert_eq!(sign_narrow.previous_visibility.as_deref(), Some("public"));
        assert_eq!(sign_narrow.visibility.as_deref(), Some("private"));

        let diff_widen = diff_entities("src/Signer.php", Some(narrowed), Some(widened));
        let sign_widen = diff_widen
            .iter()
            .find(|c| c.name == "sign")
            .expect("sign widen");
        assert!(sign_widen.visibility_changed);
        assert!(!sign_widen.visibility_narrowed);
        assert!(!sign_widen.signature_changed);
        assert_eq!(sign_widen.previous_visibility.as_deref(), Some("private"));
        assert_eq!(sign_widen.visibility.as_deref(), Some("public"));

        let diff_sig = diff_entities("src/Signer.php", Some(narrowed), Some(sig_edit));
        let sign_sig = diff_sig
            .iter()
            .find(|c| c.name == "sign")
            .expect("sign signature change");
        assert!(!sign_sig.visibility_changed);
        assert!(!sign_sig.visibility_narrowed);
        assert!(sign_sig.signature_changed);
    }

    #[test]
    fn extract_member_refs_across_all_four_languages() {
        let mut files = BTreeSet::new();
        for p in [
            "src/Security/Signer.php",
            "src/Config/TokenConfig.php",
            "src/Auth/TokenService.php",
            "crates/core/src/config.rs",
            "crates/core/src/client.rs",
            "crates/core/src/service.rs",
            "web/src/signer.ts",
            "web/src/config.ts",
            "web/src/helpers.ts",
            "web/src/service.ts",
            "pkg/client.py",
            "pkg/helpers.py",
            "pkg/service.py",
        ] {
            files.insert(p.to_owned());
        }

        // 1. PHP
        let php_src = r#"<?php
namespace App\Auth;
use App\Security\Signer;
use App\Config\TokenConfig;
class TokenService {
    public function __construct(
        private Signer $signer,
        private TokenConfig $config,
    ) {}

    public function issue(TokenConfig $overrideCfg): int {
        $sig = $this->signer->sign('ok');
        $local = new TokenConfig();
        $local->ttl = 5;
        $max = TokenConfig::DEFAULT_TTL;
        $shared = TokenConfig::$shared;
        return $this->config->ttl + $overrideCfg->ttl + $max;
    }
}
"#;
        let php_deps = extract_file_deps("src/Auth/TokenService.php", php_src, &files);
        let signer_dep = php_deps
            .iter()
            .find(|d| d.target == "src/Security/Signer.php" && d.kind == "calls")
            .expect("PHP Signer call dep");
        assert!(signer_dep.member_refs.iter().any(|m| {
            m.name == "sign"
                && m.kind == "method"
                && m.parent.as_deref() == Some("Signer")
                && m.access == "call"
        }));
        // A member access is a dependency of its own kind: an assignment
        // writes, any other access of a property or a constant reads. The
        // type hints stay plain uses and carry no member.
        let held = |deps: &[DepRef], target: &str, kind: &str| {
            deps.iter()
                .find(|d| d.target == target && d.kind == kind)
                .map(|d| {
                    d.member_refs
                        .iter()
                        .map(|m| {
                            format!(
                                "{} {} {} {}",
                                m.name,
                                m.kind,
                                m.parent.as_deref().unwrap_or("-"),
                                m.access
                            )
                        })
                        .collect::<Vec<_>>()
                })
        };
        const PHP_CONFIG: &str = "src/Config/TokenConfig.php";
        assert_eq!(
            held(&php_deps, PHP_CONFIG, "writes"),
            Some(vec!["$ttl property TokenConfig write".to_owned()])
        );
        assert_eq!(
            held(&php_deps, PHP_CONFIG, "reads"),
            Some(vec![
                "DEFAULT_TTL const TokenConfig const".to_owned(),
                "$shared property TokenConfig static_read".to_owned(),
                "$ttl property TokenConfig read".to_owned(),
            ])
        );
        assert_eq!(held(&php_deps, PHP_CONFIG, "uses"), Some(vec![]));

        // 2. Rust
        let rs_src = r#"
use crate::client::Client;
use crate::config::TokenConfig;

pub struct Service {
    client: Client,
}

impl Service {
    pub fn run(&self, cfg: &mut TokenConfig) -> u64 {
        self.client.send();
        let mut owned = TokenConfig { timeout: 30 };
        owned.timeout = 40;
        let base = TokenConfig::DEFAULT_TTL;
        cfg.timeout + base + owned.timeout
    }
}
"#;
        let rs_deps = extract_file_deps("crates/core/src/service.rs", rs_src, &files);
        let rs_client_dep = rs_deps
            .iter()
            .find(|d| d.target == "crates/core/src/client.rs" && d.kind == "calls")
            .expect("Rust Client call dep");
        assert!(rs_client_dep.member_refs.iter().any(|m| {
            m.name == "send"
                && m.kind == "method"
                && m.parent.as_deref() == Some("Client")
                && m.access == "call"
        }));
        // A field named in a struct literal builds a value: it is no write
        // to state, so it stays with the plain use of the type.
        const RS_CONFIG: &str = "crates/core/src/config.rs";
        assert_eq!(
            held(&rs_deps, RS_CONFIG, "uses"),
            Some(vec!["timeout field TokenConfig write".to_owned()])
        );
        assert_eq!(
            held(&rs_deps, RS_CONFIG, "writes"),
            Some(vec!["timeout field TokenConfig write".to_owned()])
        );
        assert_eq!(
            held(&rs_deps, RS_CONFIG, "reads"),
            Some(vec![
                "DEFAULT_TTL const TokenConfig const".to_owned(),
                "timeout field TokenConfig read".to_owned(),
            ])
        );

        // 3. TS/JS
        let ts_src = r#"
import { Signer } from './signer';
import { TokenConfig, dep } from './config';
import * as helpers from './helpers';

export class Service {
    #signer: Signer;
    constructor(private cfg: TokenConfig) {
        this.#signer = new Signer();
    }
    run(): number {
        this.#signer.sign();
        helpers.notify();
        const x = dep.prop;
        dep.prop = 99;
        return this.cfg.timeout + TokenConfig.DEFAULT_TTL + x;
    }
}
"#;
        let ts_deps = extract_file_deps("web/src/service.ts", ts_src, &files);
        let ts_signer = ts_deps
            .iter()
            .find(|d| d.target == "web/src/signer.ts" && d.kind == "calls")
            .expect("TS Signer calls");
        assert!(ts_signer.member_refs.iter().any(|m| {
            m.name == "sign"
                && m.kind == "method"
                && m.parent.as_deref() == Some("Signer")
                && m.access == "call"
        }));
        let ts_helpers = ts_deps
            .iter()
            .find(|d| d.target == "web/src/helpers.ts" && d.kind == "calls")
            .expect("TS helpers call");
        assert!(ts_helpers.member_refs.iter().any(|m| {
            m.name == "notify" && m.kind == "function" && m.parent.is_none() && m.access == "call"
        }));
        const TS_CONFIG: &str = "web/src/config.ts";
        assert_eq!(
            held(&ts_deps, TS_CONFIG, "writes"),
            Some(vec!["prop property - write".to_owned()])
        );
        assert_eq!(
            held(&ts_deps, TS_CONFIG, "reads"),
            Some(vec![
                "prop property - read".to_owned(),
                "DEFAULT_TTL const TokenConfig const".to_owned(),
                "timeout property TokenConfig read".to_owned(),
            ])
        );

        // 4. Python
        let py_src = r#"
from .client import Client, Config, dep
from . import helpers

class Service:
    def __init__(self, client: Client, cfg: Config):
        self.client = client
        self.cfg = cfg

    def run(self) -> int:
        self.client.send()
        helpers.notify()
        val = dep.prop
        return self.cfg.timeout + helpers.MAX_TTL + val
"#;
        let py_deps = extract_file_deps("pkg/service.py", py_src, &files);
        let py_client_call = py_deps
            .iter()
            .find(|d| d.target == "pkg/client.py" && d.kind == "calls")
            .expect("Py client calls");
        assert!(py_client_call.member_refs.iter().any(|m| {
            m.name == "send"
                && m.kind == "method"
                && m.parent.as_deref() == Some("Client")
                && m.access == "call"
        }));
        assert_eq!(
            held(&py_deps, "pkg/client.py", "reads"),
            Some(vec![
                "prop property - read".to_owned(),
                "timeout property Config read".to_owned(),
            ])
        );
        assert_eq!(held(&py_deps, "pkg/client.py", "writes"), None);
        let py_helpers_call = py_deps
            .iter()
            .find(|d| d.target == "pkg/helpers.py" && d.kind == "calls")
            .expect("Py helpers calls");
        assert!(py_helpers_call.member_refs.iter().any(|m| {
            m.name == "notify" && m.kind == "function" && m.parent.is_none() && m.access == "call"
        }));
        assert_eq!(
            held(&py_deps, "pkg/helpers.py", "reads"),
            Some(vec!["MAX_TTL const - const".to_owned()])
        );
    }

    #[test]
    fn extract_declared_members_and_matching_risks() {
        let mut files = BTreeSet::new();
        files.insert("src/Config/TokenConfig.php".to_owned());
        files.insert("src/Auth/Caller.php".to_owned());

        let target_src = r#"<?php
namespace App\Config;
class TokenConfig {
    public private(set) int $ttl = 60;
    private function secret(): void {}
}
"#;
        let members = extract_declared_members("src/Config/TokenConfig.php", target_src);
        let ttl = members
            .iter()
            .find(|m| m.name == "$ttl")
            .expect("$ttl decl");
        assert_eq!(ttl.kind, "property");
        assert_eq!(ttl.visibility.as_deref(), Some("private(set)"));
        let secret = members
            .iter()
            .find(|m| m.name == "secret")
            .expect("secret decl");
        assert_eq!(secret.visibility.as_deref(), Some("private"));

        let caller_src = r#"<?php
namespace App\Auth;
use App\Config\TokenConfig;
class Caller {
    public function mutate(TokenConfig $cfg): void {
        $cfg->ttl = 120;
        $cfg->secret();
    }
}
"#;
        let matched = find_matching_member_references(
            "src/Auth/Caller.php",
            caller_src,
            "src/Config/TokenConfig.php",
            &members,
            &files,
        );
        let ttl_ref = matched
            .iter()
            .find(|r| r.name == "$ttl" && r.access == "write")
            .expect("$ttl write ref");
        assert_eq!(ttl_ref.access_risk.as_deref(), Some("private_set"));
        let secret_ref = matched
            .iter()
            .find(|r| r.name == "secret" && r.access == "call")
            .expect("secret call ref");
        assert_eq!(secret_ref.access_risk.as_deref(), Some("private"));
    }

    #[test]
    fn extract_file_deps_with_manifests_resolves_composer_tsconfig_package_and_pyproject() {
        let files = known(&[
            "modules/Billing/src/Invoice.php",
            "src/App/Controller.php",
            "packages/core/src/crypto/hash.ts",
            "packages/shared/src/index.ts",
            "apps/web/src/main.ts",
            "services/api/src/billing_pkg/ledger.py",
            "services/api/tests/app.py",
        ]);
        let manifests = ManifestIndex::from_texts(&[
            (
                "composer.json",
                r#"{"autoload":{"psr-4":{"Acme\\Billing\\":"modules/Billing/src/"}}}"#,
            ),
            (
                "tsconfig.json",
                r#"{"compilerOptions":{"baseUrl":".","paths":{"@core/*":["packages/core/src/*"]}}}"#,
            ),
            (
                "packages/shared/package.json",
                r#"{"name":"@acme/shared","exports":{".":"./src/index.ts"}}"#,
            ),
            (
                "services/api/pyproject.toml",
                "[project]\nname = \"api\"\n[tool.setuptools.packages.find]\nwhere = [\"src\"]\n",
            ),
        ]);

        let php_deps = extract_file_deps_with_manifests(
            "src/App/Controller.php",
            "<?php\nuse Acme\\Billing\\Invoice;\nclass Controller { public function run(): Invoice { return new Invoice(); } }\n",
            &files,
            Some(&manifests),
            None,
        );
        assert!(
            php_deps
                .iter()
                .any(|d| d.target == "modules/Billing/src/Invoice.php"
                    && d.resolved
                    && d.kind == "imports"),
            "{php_deps:?}"
        );

        let ts_deps = extract_file_deps_with_manifests(
            "apps/web/src/main.ts",
            "import { sha } from '@core/crypto/hash';\nimport { boot } from '@acme/shared';\nexport const run = () => sha() + boot();\n",
            &files,
            Some(&manifests),
            None,
        );
        assert!(
            ts_deps
                .iter()
                .any(|d| d.target == "packages/core/src/crypto/hash.ts" && d.resolved),
            "{ts_deps:?}"
        );
        assert!(
            ts_deps
                .iter()
                .any(|d| d.target == "packages/shared/src/index.ts" && d.resolved),
            "{ts_deps:?}"
        );

        let py_deps = extract_file_deps_with_manifests(
            "services/api/tests/app.py",
            "from billing_pkg.ledger import record\nrecord()\n",
            &files,
            Some(&manifests),
            None,
        );
        assert!(
            py_deps
                .iter()
                .any(|d| d.target == "services/api/src/billing_pkg/ledger.py" && d.resolved),
            "{py_deps:?}"
        );
    }

    #[test]
    fn propagates_rust_pub_use_reexports_across_named_wildcard_alias_and_cyclic_chains() {
        let files = known(&[
            "crates/app/src/inner.rs",
            "crates/app/src/submodule.rs",
            "crates/app/src/facade.rs",
            "crates/app/src/cycle_a.rs",
            "crates/app/src/cycle_b.rs",
            "crates/app/src/caller.rs",
        ]);
        let inner = "pub struct Thing { pub id: u32 }\nimpl Thing { pub fn create() -> Self { Self { id: 1 } } }\n";
        let submodule = "pub struct SubItem;\n";
        let facade = "pub use crate::inner::Thing;\npub use crate::inner::Thing as PublicThing;\npub use crate::submodule::*;\n";
        let cycle_a = "pub use crate::cycle_b::*;\n";
        let cycle_b = "pub use crate::cycle_a::*;\n";
        let caller = "use crate::facade::{Thing, PublicThing, SubItem};\nuse crate::cycle_a::Ghost;\npub fn go() { Thing::create(); PublicThing::create(); }\n";

        let reexports = ReexportIndex::from_sources(&[
            ("crates/app/src/inner.rs", inner),
            ("crates/app/src/submodule.rs", submodule),
            ("crates/app/src/facade.rs", facade),
            ("crates/app/src/cycle_a.rs", cycle_a),
            ("crates/app/src/cycle_b.rs", cycle_b),
        ]);

        let deps = extract_file_deps_with_manifests(
            "crates/app/src/caller.rs",
            caller,
            &files,
            None,
            Some(&reexports),
        );
        assert!(
            deps.iter().any(|d| d.target == "crates/app/src/inner.rs"
                && d.kind == "imports"
                && d.resolved),
            "expected Thing/PublicThing to resolve to inner.rs: {deps:?}"
        );
        assert!(
            deps.iter()
                .any(|d| d.target == "crates/app/src/inner.rs" && d.kind == "calls" && d.resolved),
            "expected Thing::create() call to resolve to inner.rs: {deps:?}"
        );
        assert!(
            deps.iter()
                .any(|d| d.target == "crates/app/src/submodule.rs"
                    && d.kind == "imports"
                    && d.resolved),
            "expected wildcard SubItem re-export to resolve to submodule.rs: {deps:?}"
        );
        // Cyclic pub use terminates cleanly without looping.
        assert!(
            deps.iter()
                .any(|d| d.target == "crates/app/src/cycle_a.rs" && d.kind == "imports"),
            "{deps:?}"
        );
    }

    #[test]
    fn extracts_deps_for_go_java_kotlin_csharp_ruby_swift_and_scala() {
        let files = known(&[
            "internal/auth/guard.go",
            "cmd/main.go",
            "src/main/java/acme/security/Signer.java",
            "src/main/java/acme/app/Controller.java",
            "src/main/kotlin/acme/security/Signer.kt",
            "src/main/kotlin/acme/app/Service.kt",
            "src/Security/Signer.cs",
            "src/App/Handler.cs",
            "lib/auth/signer.rb",
            "lib/app/controller.rb",
            "Sources/AuthKit/Signer.swift",
            "Sources/App/Runner.swift",
            "src/main/scala/acme/security/Signer.scala",
            "src/main/scala/acme/app/App.scala",
        ]);

        let go_deps = extract_file_deps(
            "cmd/main.go",
            "package main\nimport \"github.com/acme/repo/internal/auth\"\nfunc main() { auth.Verify() }\n",
            &files,
        );
        assert!(
            go_deps
                .iter()
                .any(|d| d.target == "internal/auth/guard.go" && d.resolved)
        );

        let java_deps = extract_file_deps(
            "src/main/java/acme/app/Controller.java",
            "package acme.app;\nimport acme.security.Signer;\npublic class Controller extends Signer { void go() { new Signer(); } }\n",
            &files,
        );
        assert!(
            java_deps
                .iter()
                .any(|d| d.target == "src/main/java/acme/security/Signer.java" && d.resolved)
        );

        let kt_deps = extract_file_deps(
            "src/main/kotlin/acme/app/Service.kt",
            "package acme.app\nimport acme.security.Signer\nclass Service : Signer() {}\n",
            &files,
        );
        assert!(
            kt_deps
                .iter()
                .any(|d| d.target == "src/main/kotlin/acme/security/Signer.kt" && d.resolved)
        );

        let cs_deps = extract_file_deps(
            "src/App/Handler.cs",
            "namespace App;\nusing Security;\npublic class Handler { void Run() { new Signer(); } }\n",
            &files,
        );
        assert!(
            cs_deps
                .iter()
                .any(|d| d.target == "src/Security/Signer.cs" && d.resolved)
        );

        let rb_deps = extract_file_deps(
            "lib/app/controller.rb",
            "require_relative '../auth/signer'\nclass Controller < Signer\nend\n",
            &files,
        );
        assert!(
            rb_deps
                .iter()
                .any(|d| d.target == "lib/auth/signer.rb" && d.resolved)
        );

        let swift_deps = extract_file_deps(
            "Sources/App/Runner.swift",
            "import AuthKit\nclass Runner { func go() { Signer.check() } }\n",
            &files,
        );
        assert!(
            swift_deps
                .iter()
                .any(|d| d.target == "Sources/AuthKit/Signer.swift" && d.resolved)
        );

        let scala_deps = extract_file_deps(
            "src/main/scala/acme/app/App.scala",
            "package acme.app\nimport acme.security.Signer\nclass App extends Signer {}\n",
            &files,
        );
        assert!(
            scala_deps
                .iter()
                .any(|d| d.target == "src/main/scala/acme/security/Signer.scala" && d.resolved)
        );
    }

    #[test]
    fn go_package_imports_resolve_all_non_test_files_and_prioritize_member_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("internal/auth")).unwrap();
        std::fs::create_dir_all(root.join("cmd")).unwrap();
        std::fs::write(
            root.join("go.mod"),
            "module github.com/acme/repo\n\ngo 1.23\n",
        )
        .unwrap();
        std::fs::write(
            root.join("internal/auth/guard.go"),
            "package auth\nfunc Guard() {}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("internal/auth/verify.go"),
            "package auth\nfunc Verify() {}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("internal/auth/verify_test.go"),
            "package auth\nfunc TestVerify() {}\n",
        )
        .unwrap();

        let known = BTreeSet::from([
            "go.mod".to_owned(),
            "internal/auth/guard.go".to_owned(),
            "internal/auth/verify.go".to_owned(),
            "internal/auth/verify_test.go".to_owned(),
            "cmd/main.go".to_owned(),
        ]);
        let manifests = ManifestIndex::from_root(root);
        let deps = extract_file_deps_with_manifests(
            "cmd/main.go",
            "package main\nimport \"github.com/acme/repo/internal/auth\"\nfunc main() { auth.Verify() }\n",
            &known,
            Some(&manifests),
            None,
        );
        let targets: BTreeSet<&str> = deps
            .iter()
            .filter(|d| d.resolved)
            .map(|d| d.target.as_str())
            .collect();
        assert!(targets.contains("internal/auth/guard.go"), "{deps:?}");
        assert!(targets.contains("internal/auth/verify.go"), "{deps:?}");
        assert!(
            !targets.contains("internal/auth/verify_test.go"),
            "{deps:?}"
        );
    }

    #[test]
    fn shared_ast_extraction_matches_file_and_member_references_and_caches_known_index() {
        let caller_src = "use crate::service::Widget;\n\npub fn run() {\n    let w = Widget::new();\n    w.render();\n}\n";
        let ir = crate::graph_ast::parse_file_ir("src/caller.rs", caller_src).expect("ast ir");
        let known = BTreeSet::from(["src/caller.rs".to_owned(), "src/service.rs".to_owned()]);
        let syms = vec!["Widget".to_owned(), "render".to_owned()];

        let via_ir = find_matching_references_from_ir(
            "src/caller.rs",
            caller_src,
            &ir,
            "src/service.rs",
            &syms,
            &known,
            None,
            None,
            None,
        );
        let direct =
            find_matching_references("src/caller.rs", caller_src, "src/service.rs", &syms, &known);
        assert_eq!(via_ir, direct);
        assert!(via_ir.contains(&"Widget".to_owned()), "{via_ir:?}");

        let target_members = extract_declared_members(
            "src/service.rs",
            "pub struct Widget {}\nimpl Widget { pub fn new() -> Self { Self {} }\n pub fn render(&self) {} }\n",
        );
        let member_refs = find_matching_member_references_from_ir(
            "src/caller.rs",
            caller_src,
            &ir,
            "src/service.rs",
            &target_members,
            &known,
            None,
            None,
            None,
        );
        assert!(
            member_refs.iter().any(|r| r.name == "render"),
            "{member_refs:?}"
        );

        let lookup = KnownLookup::new(&known);
        assert_eq!(
            lookup.resolve_exact_or_ci("src/service.rs").as_deref(),
            Some("src/service.rs")
        );
        let lookup_cached = KnownLookup::new(&known);
        assert!(Arc::ptr_eq(&lookup.index, &lookup_cached.index));
        assert_eq!(lookup_cached.all_entries().len(), 2);
    }

    #[test]
    fn resolve_php_ir_records_unresolved_calls_and_follows_reexports() {
        let known = known(&[
            "src/Billing/Gateway.php",
            "src/Billing/Facade.php",
            "src/Http/PayController.php",
        ]);
        let php = r#"<?php
namespace App\Http;

use App\Billing\Facade;
use Vendor\Sdk\StripeClient;

class PayController {
    public function pay(StripeClient $client): int {
        Facade::charge();
        $len = strlen('ok');
        return $client->submit() + $len;
    }
}
"#;
        let mut reexports = ReexportIndex::default();
        reexports.by_file.insert(
            "src/Billing/Facade.php".to_owned(),
            vec![ReexportEntry {
                source_module_path: "src/Billing/Gateway.php".to_owned(),
                exported_symbol: "Facade".to_owned(),
                original_symbol: "Gateway".to_owned(),
                is_wildcard: false,
            }],
        );
        let ir = crate::graph_ast::parse_file_ir("src/Http/PayController.php", php).unwrap();
        let deps = extract_file_deps_from_ir(
            "src/Http/PayController.php",
            &ir,
            &known,
            None,
            Some(&reexports),
            None,
        );
        assert!(
            deps.iter()
                .any(|d| d.target == "src/Billing/Gateway.php" && d.resolved),
            "PHP re-export should redirect facade call/import to defining file: {deps:?}"
        );
        assert!(
            deps.iter().any(|d| {
                !d.resolved
                    && d.target.contains("StripeClient")
                    && matches!(d.kind.as_str(), "calls" | "uses")
            }),
            "unresolved external Vendor\\Sdk\\StripeClient call/type should be surfaced: {deps:?}"
        );
        assert!(
            !deps.iter().any(|d| d.target == "strlen"),
            "PHP builtin free functions should stay excluded: {deps:?}"
        );
    }

    #[test]
    fn nested_manifests_invalidate_cached_manifests_even_when_known_files_are_code_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("apps/web/src")).unwrap();
        std::fs::create_dir_all(root.join("apps/api/src")).unwrap();
        std::fs::create_dir_all(root.join("crates/core/src")).unwrap();

        std::fs::write(
            root.join("apps/web/tsconfig.json"),
            "{\"compilerOptions\":{\"baseUrl\":\".\",\"paths\":{\"@web/*\":[\"src/*\"]}}}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("apps/web/package.json"),
            "{\"name\":\"@acme/web\"}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("apps/api/composer.json"),
            "{\"autoload\":{\"psr-4\":{\"Acme\\\\Api\\\\\":\"src/\"}}}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("crates/core/Cargo.toml"),
            "[package]\nname = \"acme-core\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(root.join("apps/web/src/util.ts"), "export const u = 1;\n").unwrap();
        std::fs::write(root.join("apps/web/src/v2.ts"), "export const v = 2;\n").unwrap();
        std::fs::write(
            root.join("apps/api/src/Kernel.php"),
            "<?php class Kernel {}\n",
        )
        .unwrap();
        std::fs::write(root.join("crates/core/src/lib.rs"), "pub fn k() {}\n").unwrap();

        // Simulate GraphIrStore::known_files() filtered by is_supported_code_rel (zero manifest paths).
        let code_only_known = known(&[
            "apps/web/src/util.ts",
            "apps/web/src/v2.ts",
            "apps/api/src/Kernel.php",
            "crates/core/src/lib.rs",
        ]);
        let (m1, c1) = cached_manifests_for_root(root, &code_only_known);
        let (m1_hit, c1_hit) = cached_manifests_for_root(root, &code_only_known);
        assert!(Arc::ptr_eq(&m1, &m1_hit));
        assert!(Arc::ptr_eq(&c1, &c1_hit));

        // Edit nested tsconfig.json and verify ManifestIndex cache invalidates without known_files changing.
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(
            root.join("apps/web/tsconfig.json"),
            "{\"compilerOptions\":{\"baseUrl\":\".\",\"paths\":{\"@web2/*\":[\"src/*\"]}}}\n",
        )
        .unwrap();
        let (m2, _) = cached_manifests_for_root(root, &code_only_known);
        assert!(
            !Arc::ptr_eq(&m1, &m2),
            "nested tsconfig.json edit must invalidate ManifestIndex cache"
        );
        let hits = m2.resolve_ts_import("apps/web/src/main.ts", "@web2/v2", &code_only_known);
        assert_eq!(hits.as_deref(), Some("apps/web/src/v2.ts"), "{hits:?}");

        // Edit nested composer.json, package.json, and Cargo.toml -> each invalidates stamp.
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(
            root.join("apps/api/composer.json"),
            "{\"autoload\":{\"psr-4\":{\"Acme\\\\Api2\\\\\":\"src/\"}}}\n",
        )
        .unwrap();
        let (m3, _) = cached_manifests_for_root(root, &code_only_known);
        assert!(!Arc::ptr_eq(&m2, &m3));

        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(
            root.join("apps/web/package.json"),
            "{\"name\":\"@acme/web-renamed\"}\n",
        )
        .unwrap();
        let (m4, _) = cached_manifests_for_root(root, &code_only_known);
        assert!(!Arc::ptr_eq(&m3, &m4));

        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(
            root.join("crates/core/Cargo.toml"),
            "[package]\nname = \"acme-core-renamed\"\nversion = \"0.2.0\"\n",
        )
        .unwrap();
        let (m5, c5) = cached_manifests_for_root(root, &code_only_known);
        assert!(!Arc::ptr_eq(&m4, &m5));
        assert!(!Arc::ptr_eq(&c1, &c5));
    }

    #[test]
    fn swift_import_resolution_uses_indexed_dir_lookup_without_false_substring_match() {
        let mut files = BTreeSet::from([
            "Sources/AuthKit/Signer.swift".to_owned(),
            "Sources/MyAuthKitExtra/Helper.swift".to_owned(),
            "Sources/App/Runner.swift".to_owned(),
        ]);
        for i in 0..500 {
            files.insert(format!("Sources/Generated/File{i:03}.swift"));
        }
        let lookup = KnownLookup::new(&files);
        assert_eq!(
            lookup.resolve_swift_module("AuthKit").as_deref(),
            Some("Sources/AuthKit/Signer.swift")
        );
        assert_eq!(
            lookup.resolve_swift_module("Sources/AuthKit").as_deref(),
            Some("Sources/AuthKit/Signer.swift")
        );
        assert_eq!(
            lookup.resolve_swift_module("AuthKit.Signer").as_deref(),
            Some("Sources/AuthKit/Signer.swift")
        );
        assert_eq!(lookup.resolve_swift_module("Kit"), None);

        let deps = extract_file_deps(
            "Sources/App/Runner.swift",
            "import AuthKit\nimport Foundation\nclass Runner {}\n",
            &files,
        );
        assert!(
            deps.iter()
                .any(|d| d.target == "Sources/AuthKit/Signer.swift" && d.resolved),
            "{deps:?}"
        );
        assert!(
            deps.iter().any(|d| d.target == "Foundation" && !d.resolved),
            "{deps:?}"
        );
    }

    #[test]
    fn blade_templates_and_php_view_calls_resolve_views_components_and_livewire() {
        let files = known(&[
            "resources/views/layouts/app.blade.php",
            "resources/views/partials/header.blade.php",
            "resources/views/components/forms/text-input.blade.php",
            "app/View/Components/ModalDialog.php",
            "app/Livewire/OrderSummary.php",
            "resources/views/orders/show.blade.php",
            "app/Http/Controllers/OrderController.php",
        ]);
        let blade = r#"
@extends('layouts.app')
@include('partials.header')
<x-forms.text-input name="email" />
<x-modal-dialog />
<livewire:order-summary />
"#;
        let blade_deps = extract_file_deps("resources/views/orders/show.blade.php", blade, &files);
        for expected in [
            "resources/views/layouts/app.blade.php",
            "resources/views/partials/header.blade.php",
            "resources/views/components/forms/text-input.blade.php",
            "app/View/Components/ModalDialog.php",
            "app/Livewire/OrderSummary.php",
        ] {
            assert!(
                blade_deps
                    .iter()
                    .any(|d| d.target == expected && d.resolved && d.exists),
                "expected resolved blade dependency on {expected}, got: {blade_deps:?}"
            );
        }

        let ctrl = r#"<?php
namespace App\Http\Controllers;
class OrderController {
    public function show() {
        return view('orders.show');
    }
}
"#;
        let ctrl_deps = extract_file_deps("app/Http/Controllers/OrderController.php", ctrl, &files);
        assert!(
            ctrl_deps
                .iter()
                .any(|d| d.target == "resources/views/orders/show.blade.php" && d.resolved),
            "PHP view() helper should resolve target blade template: {ctrl_deps:?}"
        );
    }

    #[test]
    fn intra_file_deps_include_self_symbol_edges_only_when_enabled() {
        let files = known(&["src/billing.ts", "src/UserService.php"]);
        let ts_src = r#"
export class Billing {
    private total: number = 0;
    calculate(amount: number): number {
        this.total = helper(amount);
        return this.formatTotal();
    }
    formatTotal(): number {
        return this.total;
    }
}
function helper(v: number): number { return v * 2; }
"#;
        let cross_only = extract_file_deps("src/billing.ts", ts_src, &files);
        assert!(
            !cross_only.iter().any(|d| d.target == "src/billing.ts"),
            "default extract_file_deps must stay strictly cross-file: {cross_only:?}"
        );

        let with_intra = extract_file_deps_with_intra("src/billing.ts", ts_src, &files);
        let self_calls: Vec<&DepMemberRef> = with_intra
            .iter()
            .filter(|d| d.target == "src/billing.ts")
            .flat_map(|d| d.member_refs.iter())
            .collect();
        assert!(
            self_calls
                .iter()
                .any(|m| m.name == "helper" && m.access == "call"),
            "expected intra-file call to helper(): {self_calls:?}"
        );
        assert!(
            self_calls
                .iter()
                .any(|m| m.name == "formatTotal" && m.parent.as_deref() == Some("Billing")),
            "expected intra-file method call via this.formatTotal(): {self_calls:?}"
        );

        let php_src = r#"<?php
class UserService {
    private array $items = [];
    public function add(string $item): void {
        $this->items[] = $item;
        $this->notify();
    }
    private function notify(): void {}
}
"#;
        let php_intra = extract_file_deps_with_intra("src/UserService.php", php_src, &files);
        let php_self_refs: Vec<&DepMemberRef> = php_intra
            .iter()
            .filter(|d| d.target == "src/UserService.php")
            .flat_map(|d| d.member_refs.iter())
            .collect();
        assert!(
            php_self_refs
                .iter()
                .any(|m| m.name == "notify" && m.parent.as_deref() == Some("UserService")),
            "expected PHP $this->notify() intra-file edge: {php_self_refs:?}"
        );
    }

    #[test]
    fn cached_manifests_invalidates_crate_map_on_equal_count_non_manifest_swap() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("crates/alpha/src")).unwrap();
        std::fs::create_dir_all(root.join("crates/beta/src")).unwrap();
        std::fs::write(
            root.join("crates/alpha/Cargo.toml"),
            "[package]\nname = \"alpha-pkg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(
            root.join("crates/beta/Cargo.toml"),
            "[package]\nname = \"beta-pkg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(root.join("crates/alpha/src/lib.rs"), "pub fn a() {}\n").unwrap();
        std::fs::write(root.join("crates/beta/src/lib.rs"), "pub fn b() {}\n").unwrap();

        let alpha_only = known(&["crates/alpha/src/lib.rs"]);
        let beta_only = known(&["crates/beta/src/lib.rs"]);
        assert_eq!(alpha_only.len(), beta_only.len());

        let (_, c_alpha) = cached_manifests_for_root(root, &alpha_only);
        assert!(
            c_alpha
                .resolve_crate_module("alpha_pkg", &[], &alpha_only)
                .is_some()
        );
        assert!(
            c_alpha
                .resolve_crate_module("beta_pkg", &[], &alpha_only)
                .is_none()
        );

        // Swapping to an equal-count non-manifest file set must NOT hit stale CratePackageMap.
        let (_, c_beta) = cached_manifests_for_root(root, &beta_only);
        assert!(!Arc::ptr_eq(&c_alpha, &c_beta));
        assert!(
            c_beta
                .resolve_crate_module("beta_pkg", &[], &beta_only)
                .is_some(),
            "equal-count swap must rebuild CratePackageMap for crates/beta"
        );
        assert!(
            c_beta
                .resolve_crate_module("alpha_pkg", &[], &beta_only)
                .is_none()
        );
    }

    #[test]
    fn backward_reexport_traversal_respects_max_hops_and_cycles() {
        let mut known_files = BTreeSet::from(["src/leaf.ts".to_owned()]);
        let mut rows = Vec::new();
        for i in 1..=10 {
            let cur = format!("src/b{i}/index.ts");
            let prev = if i == 1 {
                "../leaf".to_owned()
            } else {
                format!("../b{}", i - 1)
            };
            known_files.insert(cur.clone());
            rows.push((cur, prev));
        }
        // Add a re-export cycle b2 <-> b1 to prove termination.
        rows.push(("src/b1/index.ts".to_owned(), "../b2".to_owned()));

        let seeds = BTreeSet::from(["src/leaf.ts".to_owned()]);
        let barrels = collect_backward_reexport_barrels(
            &rows,
            &seeds,
            &known_files,
            None,
            None,
            MAX_REEXPORT_HOPS,
        );
        for i in 1..=MAX_REEXPORT_HOPS {
            assert!(
                barrels.contains(&format!("src/b{i}/index.ts")),
                "expected hop {i} barrel in {barrels:?}"
            );
        }
        assert!(
            !barrels.contains("src/b9/index.ts"),
            "hop 9 must stay bounded by MAX_REEXPORT_HOPS={MAX_REEXPORT_HOPS}: {barrels:?}"
        );
    }
}
