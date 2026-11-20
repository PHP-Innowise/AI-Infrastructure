//! Multi-ecosystem workspace manifest parsers and import path resolvers.
//!
//! Reads bounded `composer.json`, `tsconfig.json` / `jsconfig.json`, `package.json`,
//! `pyproject.toml`, and `go.mod` manifests either from disk (`ManifestIndex::from_root`)
//! or from in-memory `(rel_path, content)` pairs (`ManifestIndex::from_texts`), and
//! resolves cross-file imports against `known_files`.

use crate::graph_live::clean_rel_key;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    io::Read,
    path::{Component, Path},
};

const MAX_MANIFEST_BYTES: u64 = 512 * 1024;
#[allow(dead_code)]
const MAX_WALK_DIRS: usize = 2000;
#[allow(dead_code)]
const MAX_MANIFEST_COUNT: usize = 256;
const MAX_TSCONFIG_EXTENDS_HOPS: usize = 6;

const PHP_EXTS: &[&str] = &["php", "phtml", "inc"];
const TS_JS_EXTS: &[&str] = &["ts", "tsx", "d.ts", "mts", "cts", "js", "jsx", "mjs", "cjs"];
const PYTHON_EXTS: &[&str] = &["py", "pyi"];

#[allow(dead_code)]
const IGNORED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "vendor",
    "target",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    ".nuxt",
    ".cargo",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ComposerMapping {
    base_dir: String,
    psr4: Vec<(String, String)>, // (namespace_prefix_with_trailing_slash_removed, dir_rel_path)
    classmap_dirs: Vec<String>,
    classmap_files: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TsConfig {
    rel_path: String,
    dir: String,
    extends: Option<String>,
    base_url: Option<String>,
    paths: Vec<(String, Vec<String>)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PackageManifest {
    dir: String,
    name: Option<String>,
    main: Option<String>,
    module: Option<String>,
    types: Option<String>,
    exports: Vec<(String, Vec<String>)>,
    imports: Vec<(String, Vec<String>)>,
    workspaces: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PyProjectManifest {
    dir: String,
    name: Option<String>,
    source_roots: Vec<String>,
    package_dirs: Vec<(String, String)>, // (dotted_pkg, dir_rel_path)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct GoModManifest {
    dir: String,
    module_path: String,
}

/// Aggregated multi-ecosystem workspace manifests for deterministic import resolution.
#[derive(Clone, Debug, Default)]
pub(crate) struct ManifestIndex {
    composers: Vec<ComposerMapping>,
    tsconfigs: Vec<TsConfig>,
    packages: Vec<PackageManifest>,
    pyprojects: Vec<PyProjectManifest>,
    gomods: Vec<GoModManifest>,
}

impl ManifestIndex {
    /// Builds a `ManifestIndex` from in-memory `(rel_path, content)` pairs.
    #[allow(dead_code)]
    pub(crate) fn from_texts(manifests: &[(&str, &str)]) -> Self {
        let mut index = Self::default();
        for &(raw_path, text) in manifests {
            if text.len() as u64 > MAX_MANIFEST_BYTES {
                continue;
            }
            let rel = clean_rel_key(raw_path);
            if rel.is_empty() {
                continue;
            }
            index.ingest(&rel, text);
        }
        index.finalize();
        index
    }

    /// Builds a `ManifestIndex` by walking `root` safely within size and directory bounds.
    #[allow(dead_code)]
    pub(crate) fn from_root(root: &Path) -> Self {
        if root.as_os_str().is_empty() || !root.is_dir() {
            return Self::default();
        }
        let Ok(real_root) = fs::canonicalize(root) else {
            return Self::default();
        };
        let mut index = Self::default();
        let mut queue = VecDeque::new();
        queue.push_back(real_root.clone());
        let mut visited_dirs = 0usize;
        let mut ingested = 0usize;

        while let Some(dir) = queue.pop_front() {
            if visited_dirs >= MAX_WALK_DIRS || ingested >= MAX_MANIFEST_COUNT {
                break;
            }
            visited_dirs += 1;
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            let mut sorted: Vec<_> = entries.flatten().collect();
            sorted.sort_by_key(|e| e.file_name());
            for entry in sorted {
                if ingested >= MAX_MANIFEST_COUNT {
                    break;
                }
                let path = entry.path();
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                let Ok(meta) = fs::symlink_metadata(&path) else {
                    continue;
                };
                if meta.file_type().is_symlink() {
                    continue;
                }
                if meta.is_dir() {
                    if !IGNORED_DIRS.contains(&name_str.as_ref()) {
                        queue.push_back(path);
                    }
                    continue;
                }
                if !meta.is_file()
                    || meta.len() > MAX_MANIFEST_BYTES
                    || !is_manifest_filename(&name_str)
                {
                    continue;
                }
                let Ok(rel_buf) = path.strip_prefix(&real_root) else {
                    continue;
                };
                let Some(rel) = normalize_path_buf(rel_buf) else {
                    continue;
                };
                let Some(text) = read_bounded_manifest_file(&path, MAX_MANIFEST_BYTES) else {
                    continue;
                };
                index.ingest(&rel, &text);
                ingested += 1;
            }
        }
        index.finalize();
        index
    }

    /// Builds a `ManifestIndex` from a known file list and reader callback.
    #[allow(dead_code)]
    pub(crate) fn from_reader(paths: &[String], reader: &dyn Fn(&str) -> Option<String>) -> Self {
        let mut index = Self::default();
        for path in paths {
            let clean = clean_rel_key(path);
            let fname = file_name(&clean);
            if !is_manifest_filename(fname) {
                continue;
            }
            if let Some(text) = reader(&clean)
                && text.len() as u64 <= MAX_MANIFEST_BYTES
            {
                index.ingest(&clean, &text);
            }
        }
        index.finalize();
        index
    }

    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.composers.is_empty()
            && self.tsconfigs.is_empty()
            && self.packages.is_empty()
            && self.pyprojects.is_empty()
            && self.gomods.is_empty()
    }

    fn ingest(&mut self, rel_path: &str, text: &str) {
        let fname = file_name(rel_path);
        let dir = dir_of(rel_path);
        if fname.eq_ignore_ascii_case("composer.json") {
            if let Some(parsed) = parse_composer_json(&dir, text) {
                self.composers.push(parsed);
            }
        } else if is_tsconfig_name(fname) {
            if let Some(parsed) = parse_tsconfig(rel_path, &dir, text) {
                self.tsconfigs.push(parsed);
            }
        } else if fname.eq_ignore_ascii_case("package.json") {
            if let Some(parsed) = parse_package_json(&dir, text) {
                self.packages.push(parsed);
            }
        } else if fname.eq_ignore_ascii_case("pyproject.toml") {
            if let Some(parsed) = parse_pyproject_toml(&dir, text) {
                self.pyprojects.push(parsed);
            }
        } else if fname.eq_ignore_ascii_case("go.mod")
            && let Some(parsed) = parse_go_mod(&dir, text)
        {
            self.gomods.push(parsed);
        }
    }

    fn finalize(&mut self) {
        self.resolve_tsconfig_extends();
        for c in &mut self.composers {
            c.psr4
                .sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        }
        for tc in &mut self.tsconfigs {
            tc.paths
                .sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        }
        for pkg in &mut self.packages {
            pkg.exports
                .sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
            pkg.imports
                .sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        }
        for py in &mut self.pyprojects {
            py.package_dirs
                .sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        }
    }

    fn resolve_tsconfig_extends(&mut self) {
        let snapshot: BTreeMap<String, TsConfig> = self
            .tsconfigs
            .iter()
            .map(|tc| (tc.rel_path.clone(), tc.clone()))
            .collect();

        for tc in &mut self.tsconfigs {
            let mut chain: Vec<TsConfig> = Vec::new();
            let mut seen = BTreeSet::from([tc.rel_path.clone()]);
            let mut cur_extends = tc.extends.clone();
            let mut cur_dir = tc.dir.clone();

            for _ in 0..MAX_TSCONFIG_EXTENDS_HOPS {
                let Some(ext_spec) = cur_extends.take() else {
                    break;
                };
                let Some(target_rel) = resolve_extends_rel(&cur_dir, &ext_spec, &snapshot) else {
                    break;
                };
                if !seen.insert(target_rel.clone()) {
                    break;
                }
                if let Some(parent) = snapshot.get(&target_rel) {
                    cur_dir = parent.dir.clone();
                    cur_extends = parent.extends.clone();
                    chain.push(parent.clone());
                } else {
                    break;
                }
            }

            // Inherit `base_url` and `paths` from ancestors when not explicitly overridden.
            for parent in chain {
                if tc.base_url.is_none()
                    && let Some(parent_base) = &parent.base_url
                {
                    tc.base_url = Some(parent_base.clone());
                }
                for (pat, targets) in &parent.paths {
                    if !tc.paths.iter().any(|(existing, _)| existing == pat) {
                        let rebased: Vec<String> = targets
                            .iter()
                            .map(|t| {
                                rebase_tsconfig_target(
                                    parent.base_url.as_deref(),
                                    &parent.dir,
                                    t,
                                    tc.base_url.as_deref(),
                                    &tc.dir,
                                )
                            })
                            .collect();
                        tc.paths.push((pat.clone(), rebased));
                    }
                }
            }
        }
    }

    /// Resolves a PHP FQN (e.g. `App\Billing\Invoice`) via `composer.json` PSR-4 & classmap.
    pub(crate) fn resolve_php_fqn(
        &self,
        fqn: &str,
        known_files: &BTreeSet<String>,
    ) -> Option<String> {
        let cleaned = fqn.trim().trim_start_matches('\\');
        if cleaned.is_empty() {
            return None;
        }
        let lookup = KnownFileLookup::new(known_files);

        for composer in &self.composers {
            for (ns_prefix, dir_rel) in &composer.psr4 {
                if let Some(rest) = match_php_ns_prefix(cleaned, ns_prefix) {
                    let slash_rest = rest.replace('\\', "/");
                    for ext in PHP_EXTS {
                        let cand = join_rel(dir_rel, &format!("{slash_rest}.{ext}"));
                        if let Some(found) = lookup.find_exact_or_ci(&cand) {
                            return Some(found);
                        }
                    }
                }
            }
        }

        let short = cleaned.rsplit('\\').next().unwrap_or(cleaned);
        let full_slash = cleaned.replace('\\', "/");
        for composer in &self.composers {
            for file in &composer.classmap_files {
                let stem = file_stem(file);
                if stem.eq_ignore_ascii_case(short)
                    && let Some(found) = lookup.find_exact_or_ci(file)
                {
                    return Some(found);
                }
            }
            for dir_rel in &composer.classmap_dirs {
                for ext in PHP_EXTS {
                    let direct = join_rel(dir_rel, &format!("{short}.{ext}"));
                    if let Some(found) = lookup.find_exact_or_ci(&direct) {
                        return Some(found);
                    }
                    let nested = join_rel(dir_rel, &format!("{full_slash}.{ext}"));
                    if let Some(found) = lookup.find_exact_or_ci(&nested) {
                        return Some(found);
                    }
                }
            }
        }
        None
    }

    /// Resolves a TS/JS import specifier via `package.json` (`#imports`, workspace packages)
    /// and `tsconfig.json` / `jsconfig.json` (`paths` & `baseUrl`).
    pub(crate) fn resolve_ts_import(
        &self,
        caller_rel_path: &str,
        spec: &str,
        known_files: &BTreeSet<String>,
    ) -> Option<String> {
        let trimmed = spec.trim();
        if trimmed.is_empty() || trimmed.starts_with("./") || trimmed.starts_with("../") {
            return None;
        }
        let caller = clean_rel_key(caller_rel_path);
        let lookup = KnownFileLookup::new(known_files);

        // 1. Subpath `#imports` and workspace package imports from `package.json`.
        if let Some(found) = self.resolve_package_import_with(&caller, trimmed, &lookup) {
            return Some(found);
        }

        // 2. `tsconfig.json` / `jsconfig.json` `paths` and `baseUrl`, ordered nearest-ancestor first.
        let mut configs: Vec<&TsConfig> = self.tsconfigs.iter().collect();
        configs.sort_by_key(|tc| ancestor_rank(&caller, &tc.dir));

        for tc in configs {
            let base = tc.base_url.as_deref().unwrap_or(&tc.dir);
            for (pattern, targets) in &tc.paths {
                if let Some(captured) = match_wildcard_pattern(pattern, trimmed) {
                    for target_pat in targets {
                        let sub = target_pat.replace('*', captured);
                        if let Some(resolved_base) = resolve_rel_join(base, &sub)
                            && let Some(found) = resolve_ts_candidates(&resolved_base, &lookup)
                        {
                            return Some(found);
                        }
                    }
                }
            }
            if let Some(base_url) = tc.base_url.as_deref()
                && let Some(resolved_base) = resolve_rel_join(base_url, trimmed)
                && let Some(found) = resolve_ts_candidates(&resolved_base, &lookup)
            {
                return Some(found);
            }
        }
        None
    }

    /// Resolves `package.json` `#imports` or workspace `@org/pkg[/subpath]` imports.
    #[allow(dead_code)]
    pub(crate) fn resolve_package_import(
        &self,
        caller_rel_path: &str,
        spec: &str,
        known_files: &BTreeSet<String>,
    ) -> Option<String> {
        let trimmed = spec.trim();
        if trimmed.is_empty() {
            return None;
        }
        let caller = clean_rel_key(caller_rel_path);
        let lookup = KnownFileLookup::new(known_files);
        self.resolve_package_import_with(&caller, trimmed, &lookup)
    }

    fn resolve_package_import_with(
        &self,
        caller: &str,
        spec: &str,
        lookup: &KnownFileLookup,
    ) -> Option<String> {
        if spec.starts_with('#') {
            let mut pkgs: Vec<&PackageManifest> = self.packages.iter().collect();
            pkgs.sort_by_key(|pkg| ancestor_rank(caller, &pkg.dir));
            for pkg in pkgs {
                for (pattern, targets) in &pkg.imports {
                    if let Some(captured) = match_wildcard_pattern(pattern, spec) {
                        for t in targets {
                            let sub = t.replace('*', captured);
                            if let Some(base) = resolve_rel_join(&pkg.dir, &sub)
                                && let Some(found) = resolve_ts_candidates(&base, lookup)
                            {
                                return Some(found);
                            }
                        }
                    }
                }
            }
            return None;
        }

        let workspace_globs: Vec<&str> = self
            .packages
            .iter()
            .flat_map(|p| p.workspaces.iter().map(String::as_str))
            .collect();

        let mut matching: Vec<(&PackageManifest, String)> = Vec::new();
        for pkg in &self.packages {
            let Some(name) = pkg.name.as_deref() else {
                continue;
            };
            if spec == name {
                matching.push((pkg, ".".to_owned()));
            } else if let Some(rest) = spec.strip_prefix(name).and_then(|r| r.strip_prefix('/')) {
                matching.push((pkg, format!("./{rest}")));
            }
        }

        // Prefer packages matched by declared `workspaces` globs, then shorter directory.
        matching.sort_by_key(|(pkg, _)| {
            let in_ws = workspace_globs
                .iter()
                .any(|glob| matches_workspace_glob(&pkg.dir, glob));
            (!in_ws, pkg.dir.len())
        });

        for (pkg, subpath) in matching {
            // 1. Check `exports` map.
            for (exp_pat, targets) in &pkg.exports {
                if let Some(captured) = match_wildcard_pattern(exp_pat, &subpath) {
                    for t in targets {
                        let sub = t.replace('*', captured);
                        if let Some(found) =
                            resolve_pkg_target_with_src_fallback(&pkg.dir, &sub, lookup)
                        {
                            return Some(found);
                        }
                    }
                }
            }

            // 2. Root import (`spec == pkg.name`): `module`, `main`, `types`, `src/index`, `index`.
            if subpath == "." {
                for entry in [
                    pkg.module.as_deref(),
                    pkg.main.as_deref(),
                    pkg.types.as_deref(),
                    Some("src/index"),
                    Some("index"),
                ]
                .into_iter()
                .flatten()
                {
                    if let Some(found) =
                        resolve_pkg_target_with_src_fallback(&pkg.dir, entry, lookup)
                    {
                        return Some(found);
                    }
                }
            } else {
                // 3. Direct subpath fallback `<pkg.dir>/<subpath>` or `<pkg.dir>/src/<subpath>`.
                let rel_sub = subpath.trim_start_matches("./");
                for cand in [
                    resolve_rel_join(&pkg.dir, rel_sub),
                    resolve_rel_join(&pkg.dir, &format!("src/{rel_sub}")),
                ]
                .into_iter()
                .flatten()
                {
                    if let Some(found) = resolve_ts_candidates(&cand, lookup) {
                        return Some(found);
                    }
                }
            }
        }
        None
    }

    /// Resolves a Python dotted module import (e.g. `acme.billing.invoice`) via `pyproject.toml`.
    pub(crate) fn resolve_py_import(
        &self,
        caller_rel_path: &str,
        module: &str,
        known_files: &BTreeSet<String>,
    ) -> Option<String> {
        let trimmed = module.trim().trim_start_matches('.');
        if trimmed.is_empty() {
            return None;
        }
        let caller = clean_rel_key(caller_rel_path);
        let lookup = KnownFileLookup::new(known_files);
        let mod_slash = trimmed.replace('.', "/");

        let mut projects: Vec<&PyProjectManifest> = self.pyprojects.iter().collect();
        projects.sort_by_key(|p| ancestor_rank(&caller, &p.dir));

        for py in projects {
            for (pkg_dotted, dir_rel) in &py.package_dirs {
                if trimmed == pkg_dotted {
                    if let Some(found) = resolve_py_candidates(dir_rel, &lookup) {
                        return Some(found);
                    }
                } else if let Some(rest) = trimmed
                    .strip_prefix(pkg_dotted)
                    .and_then(|r| r.strip_prefix('.'))
                {
                    let base = join_rel(dir_rel, &rest.replace('.', "/"));
                    if let Some(found) = resolve_py_candidates(&base, &lookup) {
                        return Some(found);
                    }
                }
            }

            for root in &py.source_roots {
                let base = join_rel(root, &mod_slash);
                if let Some(found) = resolve_py_candidates(&base, &lookup) {
                    return Some(found);
                }
            }

            let direct_base = join_rel(&py.dir, &mod_slash);
            if let Some(found) = resolve_py_candidates(&direct_base, &lookup) {
                return Some(found);
            }
        }
        None
    }

    /// Resolves a Go import path via `go.mod` module prefix against `known_files`.
    #[cfg(test)]
    pub(crate) fn resolve_go_import(
        &self,
        import_path: &str,
        known_files: &BTreeSet<String>,
    ) -> Option<String> {
        self.resolve_go_import_files(import_path, known_files)
            .into_iter()
            .next()
    }

    /// Resolves a Go import path via `go.mod` to all `.go` source files in the target package directory.
    pub(crate) fn resolve_go_import_files(
        &self,
        import_path: &str,
        known_files: &BTreeSet<String>,
    ) -> Vec<String> {
        let cleaned = import_path.trim().trim_matches('"');
        if cleaned.is_empty() {
            return Vec::new();
        }
        for gm in &self.gomods {
            let sub = if cleaned == gm.module_path {
                ""
            } else if let Some(rest) = cleaned
                .strip_prefix(&gm.module_path)
                .and_then(|r| r.strip_prefix('/'))
            {
                rest
            } else {
                continue;
            };
            let target_dir = join_rel(&gm.dir, sub);
            let files = pick_go_files_in_dir(&target_dir, known_files);
            if !files.is_empty() {
                return files;
            }
        }
        Vec::new()
    }
}

fn is_manifest_filename(name: &str) -> bool {
    name.eq_ignore_ascii_case("composer.json")
        || name.eq_ignore_ascii_case("package.json")
        || name.eq_ignore_ascii_case("pyproject.toml")
        || name.eq_ignore_ascii_case("go.mod")
        || is_tsconfig_name(name)
}

fn is_tsconfig_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "tsconfig.json"
        || lower == "jsconfig.json"
        || (lower.starts_with("tsconfig.") && lower.ends_with(".json"))
        || (lower.starts_with("jsconfig.") && lower.ends_with(".json"))
}

fn parse_composer_json(dir: &str, text: &str) -> Option<ComposerMapping> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(text) else {
        return None;
    };
    let mut out = ComposerMapping {
        base_dir: dir.to_owned(),
        ..ComposerMapping::default()
    };
    for section in ["autoload", "autoload-dev"] {
        let Some(obj) = val.get(section).and_then(|v| v.as_object()) else {
            continue;
        };
        if let Some(psr4) = obj.get("psr-4").and_then(|v| v.as_object()) {
            for (ns_raw, paths_val) in psr4 {
                let ns = ns_raw.trim().trim_matches('\\').to_owned();
                let dirs = json_string_or_array(paths_val);
                for raw_dir in dirs {
                    if let Some(joined) = resolve_rel_join(dir, &raw_dir) {
                        out.psr4.push((ns.clone(), joined));
                    } else if raw_dir.trim().is_empty() || raw_dir.trim() == "." {
                        out.psr4.push((ns.clone(), dir.to_owned()));
                    }
                }
            }
        }
        if let Some(classmap) = obj.get("classmap").and_then(|v| v.as_array()) {
            for item in classmap.iter().filter_map(|v| v.as_str()) {
                if let Some(joined) = resolve_rel_join(dir, item) {
                    if joined.to_ascii_lowercase().ends_with(".php") {
                        out.classmap_files.push(joined);
                    } else {
                        out.classmap_dirs.push(joined);
                    }
                }
            }
        }
        if let Some(files) = obj.get("files").and_then(|v| v.as_array()) {
            for item in files.iter().filter_map(|v| v.as_str()) {
                if let Some(joined) = resolve_rel_join(dir, item) {
                    out.classmap_files.push(joined);
                }
            }
        }
    }
    Some(out)
}

fn parse_tsconfig(rel_path: &str, dir: &str, text: &str) -> Option<TsConfig> {
    let stripped = strip_jsonc_comments_and_trailing_commas(text);
    let Ok(val) = serde_json::from_str::<serde_json::Value>(&stripped) else {
        return None;
    };
    let extends = val
        .get("extends")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());

    let opts = val.get("compilerOptions").and_then(|v| v.as_object());
    let base_url = opts
        .and_then(|o| o.get("baseUrl"))
        .and_then(|v| v.as_str())
        .and_then(|b| {
            let trimmed = b.trim();
            if trimmed.is_empty() || trimmed == "." {
                Some(dir.to_owned())
            } else {
                resolve_rel_join(dir, trimmed)
            }
        });

    let mut paths = Vec::new();
    if let Some(paths_obj) = opts
        .and_then(|o| o.get("paths"))
        .and_then(|v| v.as_object())
    {
        for (pat, targets_val) in paths_obj {
            let targets = json_string_or_array(targets_val);
            if !targets.is_empty() {
                paths.push((pat.clone(), targets));
            }
        }
    }

    Some(TsConfig {
        rel_path: rel_path.to_owned(),
        dir: dir.to_owned(),
        extends,
        base_url,
        paths,
    })
}

fn parse_package_json(dir: &str, text: &str) -> Option<PackageManifest> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(text) else {
        return None;
    };
    let name = val
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let main = val
        .get("main")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let module = val
        .get("module")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let types = val
        .get("types")
        .or_else(|| val.get("typings"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());

    let mut exports = Vec::new();
    if let Some(exp_val) = val.get("exports") {
        collect_package_subpath_map(".", exp_val, &mut exports);
    }

    let mut imports = Vec::new();
    if let Some(imp_obj) = val.get("imports").and_then(|v| v.as_object()) {
        for (k, v) in imp_obj {
            let mut targets = Vec::new();
            collect_conditional_targets(v, &mut targets);
            if !targets.is_empty() {
                imports.push((k.clone(), targets));
            }
        }
    }

    let mut workspaces = Vec::new();
    if let Some(ws) = val.get("workspaces") {
        if let Some(arr) = ws.as_array() {
            for item in arr.iter().filter_map(|v| v.as_str()) {
                if let Some(joined) = resolve_rel_join(dir, item) {
                    workspaces.push(joined);
                }
            }
        } else if let Some(arr) = ws.get("packages").and_then(|v| v.as_array()) {
            for item in arr.iter().filter_map(|v| v.as_str()) {
                if let Some(joined) = resolve_rel_join(dir, item) {
                    workspaces.push(joined);
                }
            }
        }
    }

    Some(PackageManifest {
        dir: dir.to_owned(),
        name,
        main,
        module,
        types,
        exports,
        imports,
        workspaces,
    })
}

fn collect_package_subpath_map(
    default_key: &str,
    val: &serde_json::Value,
    out: &mut Vec<(String, Vec<String>)>,
) {
    match val {
        serde_json::Value::String(s) => {
            out.push((default_key.to_owned(), vec![s.trim().to_owned()]));
        }
        serde_json::Value::Array(arr) => {
            let mut targets = Vec::new();
            for item in arr {
                collect_conditional_targets(item, &mut targets);
            }
            if !targets.is_empty() {
                out.push((default_key.to_owned(), targets));
            }
        }
        serde_json::Value::Object(map) => {
            let has_subpath_keys = map.keys().any(|k| k.starts_with('.'));
            if has_subpath_keys {
                for (k, v) in map {
                    let mut targets = Vec::new();
                    collect_conditional_targets(v, &mut targets);
                    if !targets.is_empty() {
                        out.push((k.clone(), targets));
                    }
                }
            } else {
                let mut targets = Vec::new();
                collect_conditional_targets(val, &mut targets);
                if !targets.is_empty() {
                    out.push((default_key.to_owned(), targets));
                }
            }
        }
        _ => {}
    }
}

fn collect_conditional_targets(val: &serde_json::Value, out: &mut Vec<String>) {
    match val {
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            if !trimmed.is_empty() && !out.iter().any(|existing| existing == trimmed) {
                out.push(trimmed.to_owned());
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                collect_conditional_targets(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for key in [
                "import", "types", "module", "browser", "node", "default", "require",
            ] {
                if let Some(sub) = map.get(key) {
                    collect_conditional_targets(sub, out);
                }
            }
            for (key, sub) in map {
                if !matches!(
                    key.as_str(),
                    "import" | "types" | "module" | "browser" | "node" | "default" | "require"
                ) {
                    collect_conditional_targets(sub, out);
                }
            }
        }
        _ => {}
    }
}

fn parse_pyproject_toml(dir: &str, text: &str) -> Option<PyProjectManifest> {
    let Ok(table) = text.parse::<toml::Table>() else {
        return None;
    };
    let name = table
        .get("project")
        .and_then(|p| p.get("name"))
        .or_else(|| {
            table
                .get("tool")
                .and_then(|t| t.get("poetry"))
                .and_then(|p| p.get("name"))
        })
        .and_then(toml::Value::as_str)
        .map(str::to_owned);

    let mut source_roots: Vec<String> = Vec::new();
    let mut package_dirs: Vec<(String, String)> = Vec::new();
    let mut push_root = |raw: &str| {
        let joined = if raw.trim().is_empty() || raw.trim() == "." {
            dir.to_owned()
        } else {
            resolve_rel_join(dir, raw).unwrap_or_else(|| join_rel(dir, raw))
        };
        if !source_roots.contains(&joined) {
            source_roots.push(joined);
        }
    };

    if let Some(tool) = table.get("tool").and_then(toml::Value::as_table) {
        // Poetry `packages = [{ include = "pkg", from = "src" }]`
        if let Some(poetry) = tool.get("poetry").and_then(toml::Value::as_table)
            && let Some(pkgs) = poetry.get("packages").and_then(toml::Value::as_array)
        {
            for item in pkgs.iter().filter_map(toml::Value::as_table) {
                let from = item.get("from").and_then(toml::Value::as_str).unwrap_or("");
                if !from.is_empty() {
                    push_root(from);
                }
                if let Some(inc) = item.get("include").and_then(toml::Value::as_str) {
                    let root = if from.is_empty() {
                        dir.to_owned()
                    } else {
                        join_rel(dir, from)
                    };
                    let pkg_dir = join_rel(&root, inc);
                    package_dirs.push((inc.replace('/', "."), pkg_dir));
                }
            }
        }
        // Setuptools `packages.find.where = ["src"]` and `package-dir`
        if let Some(setuptools) = tool.get("setuptools").and_then(toml::Value::as_table) {
            if let Some(where_arr) = setuptools
                .get("packages")
                .and_then(|p| p.get("find"))
                .and_then(|f| f.get("where"))
                .and_then(toml::Value::as_array)
            {
                for item in where_arr.iter().filter_map(toml::Value::as_str) {
                    push_root(item);
                }
            }
            if let Some(pkg_dir_tbl) = setuptools
                .get("package-dir")
                .and_then(toml::Value::as_table)
            {
                for (k, v) in pkg_dir_tbl {
                    if let Some(path_str) = v.as_str() {
                        if k.is_empty() {
                            push_root(path_str);
                        } else if let Some(joined) = resolve_rel_join(dir, path_str) {
                            package_dirs.push((k.clone(), joined));
                        }
                    }
                }
            }
        }
        // Hatch `tool.hatch.build.targets.wheel.packages = ["src/my_pkg"]`
        if let Some(wheel_pkgs) = tool
            .get("hatch")
            .and_then(|h| h.get("build"))
            .and_then(|b| b.get("targets"))
            .and_then(|t| t.get("wheel"))
            .and_then(|w| w.get("packages"))
            .and_then(toml::Value::as_array)
        {
            for item in wheel_pkgs.iter().filter_map(toml::Value::as_str) {
                let cleaned = clean_rel_key(item);
                if let Some((parent, pkg)) = cleaned.rsplit_once('/') {
                    push_root(parent);
                    package_dirs.push((pkg.to_owned(), join_rel(dir, &cleaned)));
                } else if !cleaned.is_empty() {
                    package_dirs.push((cleaned.clone(), join_rel(dir, &cleaned)));
                }
            }
        }
    }

    // Standard `src` layout fallback under this pyproject's directory.
    push_root("src");
    push_root("");

    Some(PyProjectManifest {
        dir: dir.to_owned(),
        name,
        source_roots,
        package_dirs,
    })
}

fn parse_go_mod(dir: &str, text: &str) -> Option<GoModManifest> {
    for line in text.lines() {
        let trimmed = line.split("//").next().unwrap_or("").trim();
        if let Some(rest) = trimmed.strip_prefix("module ") {
            let module_path = rest.trim().trim_matches('"').to_owned();
            if !module_path.is_empty() {
                return Some(GoModManifest {
                    dir: dir.to_owned(),
                    module_path,
                });
            }
        }
    }
    None
}

/// Strips JSONC `//` line comments, `/* ... */` block comments, and trailing commas `,}` / `,]`.
fn strip_jsonc_comments_and_trailing_commas(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let len = bytes.len();
    let mut no_comments = String::with_capacity(len);
    let mut i = 0usize;
    while i < len {
        let ch = bytes[i];
        if ch == b'"' {
            let start = i;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            no_comments.push_str(&raw[start..i]);
            continue;
        }
        if ch == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if ch == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                if bytes[i] == b'\n' {
                    no_comments.push('\n');
                }
                i += 1;
            }
            i = (i + 2).min(len);
            continue;
        }
        if ch.is_ascii() {
            no_comments.push(ch as char);
            i += 1;
        } else if let Some(c) = raw[i..].chars().next() {
            no_comments.push(c);
            i += c.len_utf8();
        } else {
            break;
        }
    }

    // Remove trailing commas before `}` or `]` outside strings.
    let mut out = String::with_capacity(no_comments.len());
    let nb = no_comments.as_bytes();
    let nlen = nb.len();
    let mut j = 0usize;
    while j < nlen {
        let ch = nb[j];
        if ch == b'"' {
            let start = j;
            j += 1;
            while j < nlen {
                if nb[j] == b'\\' && j + 1 < nlen {
                    j += 2;
                } else if nb[j] == b'"' {
                    j += 1;
                    break;
                } else {
                    j += 1;
                }
            }
            out.push_str(&no_comments[start..j]);
            continue;
        }
        if ch == b',' {
            let mut k = j + 1;
            while k < nlen && nb[k].is_ascii_whitespace() {
                k += 1;
            }
            if k < nlen && (nb[k] == b'}' || nb[k] == b']') {
                j += 1;
                continue;
            }
        }
        if ch.is_ascii() {
            out.push(ch as char);
            j += 1;
        } else if let Some(c) = no_comments[j..].chars().next() {
            out.push(c);
            j += c.len_utf8();
        } else {
            break;
        }
    }
    out
}

fn json_string_or_array(val: &serde_json::Value) -> Vec<String> {
    match val {
        serde_json::Value::String(s) => vec![s.trim().to_owned()],
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.trim().to_owned())
            .collect(),
        _ => Vec::new(),
    }
}

fn resolve_extends_rel(
    cur_dir: &str,
    ext_spec: &str,
    snapshot: &BTreeMap<String, TsConfig>,
) -> Option<String> {
    let trimmed = ext_spec.trim();
    if trimmed.is_empty() {
        return None;
    }
    let direct = resolve_rel_join(cur_dir, trimmed)?;
    if snapshot.contains_key(&direct) {
        return Some(direct);
    }
    let with_json = format!("{direct}.json");
    if snapshot.contains_key(&with_json) {
        return Some(with_json);
    }
    None
}

fn rebase_tsconfig_target(
    parent_base: Option<&str>,
    parent_dir: &str,
    target: &str,
    child_base: Option<&str>,
    child_dir: &str,
) -> String {
    let p_base = parent_base.unwrap_or(parent_dir);
    let c_base = child_base.unwrap_or(child_dir);
    if p_base == c_base {
        return target.to_owned();
    }
    let Some(abs_rel) = resolve_rel_join(p_base, target) else {
        return target.to_owned();
    };
    if c_base.is_empty() {
        return abs_rel;
    }
    if let Some(stripped) = abs_rel
        .strip_prefix(c_base)
        .and_then(|r| r.strip_prefix('/'))
    {
        return stripped.to_owned();
    }
    let up_count = c_base.split('/').filter(|s| !s.is_empty()).count();
    let mut prefix = String::new();
    for _ in 0..up_count {
        prefix.push_str("../");
    }
    format!("{prefix}{abs_rel}")
}

fn match_php_ns_prefix<'a>(fqn: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() {
        return Some(fqn);
    }
    if let Some(rest) = fqn.strip_prefix(prefix) {
        return rest.strip_prefix('\\');
    }
    let fqn_lower = fqn.to_ascii_lowercase();
    let prefix_lower = prefix.to_ascii_lowercase();
    if fqn_lower.starts_with(&prefix_lower) && fqn.as_bytes().get(prefix.len()) == Some(&b'\\') {
        return Some(&fqn[prefix.len() + 1..]);
    }
    None
}

fn match_wildcard_pattern<'a>(pattern: &str, spec: &'a str) -> Option<&'a str> {
    if let Some((prefix, suffix)) = pattern.split_once('*') {
        if spec.len() >= prefix.len() + suffix.len()
            && spec.starts_with(prefix)
            && spec.ends_with(suffix)
        {
            return Some(&spec[prefix.len()..spec.len() - suffix.len()]);
        }
        None
    } else if pattern == spec {
        Some("")
    } else {
        None
    }
}

fn matches_workspace_glob(pkg_dir: &str, glob: &str) -> bool {
    let g = clean_rel_key(glob);
    if g.is_empty() || pkg_dir.is_empty() {
        return false;
    }
    if let Some(prefix) = g.strip_suffix("/*") {
        if let Some(rest) = pkg_dir
            .strip_prefix(prefix)
            .and_then(|r| r.strip_prefix('/'))
        {
            return !rest.is_empty() && !rest.contains('/');
        }
        return false;
    }
    if let Some(prefix) = g.strip_suffix("/**") {
        return pkg_dir == prefix
            || pkg_dir
                .strip_prefix(prefix)
                .is_some_and(|r| r.starts_with('/'));
    }
    pkg_dir == g
}

fn ancestor_rank(caller: &str, manifest_dir: &str) -> (u8, usize) {
    if manifest_dir.is_empty() {
        return (1, usize::MAX);
    }
    let caller_dir = dir_of(caller);
    if caller_dir == manifest_dir
        || caller_dir
            .strip_prefix(manifest_dir)
            .is_some_and(|r| r.starts_with('/'))
    {
        (0, usize::MAX - manifest_dir.len())
    } else {
        (2, manifest_dir.len())
    }
}

struct KnownFileLookup<'a> {
    by_cleaned: BTreeMap<String, &'a String>,
    by_lower: BTreeMap<String, &'a String>,
}

impl<'a> KnownFileLookup<'a> {
    fn new(known_files: &'a BTreeSet<String>) -> Self {
        let mut by_cleaned = BTreeMap::new();
        let mut by_lower = BTreeMap::new();
        for orig in known_files {
            let cleaned = clean_rel_key(orig);
            if cleaned.is_empty() {
                continue;
            }
            by_lower.entry(cleaned.to_ascii_lowercase()).or_insert(orig);
            by_cleaned.entry(cleaned).or_insert(orig);
        }
        Self {
            by_cleaned,
            by_lower,
        }
    }

    fn find_exact_or_ci(&self, candidate: &str) -> Option<String> {
        let cleaned = clean_rel_key(candidate);
        if cleaned.is_empty() {
            return None;
        }
        if let Some(exact) = self.by_cleaned.get(&cleaned) {
            return Some((*exact).clone());
        }
        let lower = cleaned.to_ascii_lowercase();
        self.by_lower.get(&lower).map(|s| (*s).clone())
    }
}

fn resolve_pkg_target_with_src_fallback(
    pkg_dir: &str,
    target: &str,
    lookup: &KnownFileLookup<'_>,
) -> Option<String> {
    if let Some(base) = resolve_rel_join(pkg_dir, target)
        && let Some(found) = resolve_ts_candidates(&base, lookup)
    {
        return Some(found);
    }
    // Common compiled output -> source remap (`./dist/foo.js` -> `./src/foo.ts`) when `dist/` is ignored.
    let norm = target.trim().trim_start_matches("./");
    for out_dir in ["dist/", "build/", "lib/", "out/"] {
        if let Some(rest) = norm.strip_prefix(out_dir) {
            for src_dir in ["src/", ""] {
                if let Some(base) = resolve_rel_join(pkg_dir, &format!("{src_dir}{rest}"))
                    && let Some(found) = resolve_ts_candidates(&base, lookup)
                {
                    return Some(found);
                }
            }
        }
    }
    None
}

fn resolve_ts_candidates(base: &str, lookup: &KnownFileLookup<'_>) -> Option<String> {
    if let Some(found) = lookup.find_exact_or_ci(base) {
        return Some(found);
    }
    if let Some((stem_path, ext)) = base.rsplit_once('.') {
        let lower_ext = ext.to_ascii_lowercase();
        let mapped: &[&str] = match lower_ext.as_str() {
            "js" => &["ts", "tsx", "d.ts", "js", "jsx"],
            "jsx" => &["tsx", "ts", "jsx", "js"],
            "mjs" => &["mts", "mjs"],
            "cjs" => &["cts", "cjs"],
            _ => &[],
        };
        for alt_ext in mapped {
            if let Some(found) = lookup.find_exact_or_ci(&format!("{stem_path}.{alt_ext}")) {
                return Some(found);
            }
        }
    }
    for ext in TS_JS_EXTS {
        if let Some(found) = lookup.find_exact_or_ci(&format!("{base}.{ext}")) {
            return Some(found);
        }
    }
    for ext in TS_JS_EXTS {
        if let Some(found) = lookup.find_exact_or_ci(&format!("{base}/index.{ext}")) {
            return Some(found);
        }
    }
    None
}

fn resolve_py_candidates(base: &str, lookup: &KnownFileLookup<'_>) -> Option<String> {
    for ext in PYTHON_EXTS {
        if let Some(found) = lookup.find_exact_or_ci(&format!("{base}.{ext}")) {
            return Some(found);
        }
    }
    for ext in PYTHON_EXTS {
        if let Some(found) = lookup.find_exact_or_ci(&join_rel(base, &format!("__init__.{ext}"))) {
            return Some(found);
        }
    }
    None
}

fn pick_go_files_in_dir(dir: &str, known_files: &BTreeSet<String>) -> Vec<String> {
    let mut matches: Vec<String> = known_files
        .iter()
        .filter(|path| {
            let clean = clean_rel_key(path);
            clean.ends_with(".go") && dir_of(&clean) == dir
        })
        .cloned()
        .collect();
    matches.sort();
    let non_test: Vec<String> = matches
        .iter()
        .filter(|p| !p.ends_with("_test.go"))
        .cloned()
        .collect();
    if non_test.is_empty() {
        matches
    } else {
        non_test
    }
}

fn resolve_rel_join(base_dir: &str, spec: &str) -> Option<String> {
    let norm = spec.trim().replace('\\', "/");
    if norm.is_empty() {
        return None;
    }
    let mut stack: Vec<&str> = if norm.starts_with('/') {
        Vec::new()
    } else {
        base_dir
            .split('/')
            .filter(|s| !s.is_empty() && *s != ".")
            .collect()
    };
    for seg in norm.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                stack.pop()?;
            }
            s => stack.push(s),
        }
    }
    let joined = stack.join("/");
    (!joined.is_empty()).then_some(joined)
}

pub(crate) fn join_rel(prefix: &str, tail: &str) -> String {
    if prefix.is_empty() {
        tail.trim_start_matches('/').to_owned()
    } else if tail.is_empty() {
        prefix.trim_end_matches('/').to_owned()
    } else {
        format!(
            "{}/{}",
            prefix.trim_end_matches('/'),
            tail.trim_start_matches('/')
        )
    }
}

fn dir_of(rel_path: &str) -> String {
    rel_path
        .rsplit_once('/')
        .map_or_else(String::new, |(d, _)| d.to_owned())
}

fn file_name(rel_path: &str) -> &str {
    rel_path.rsplit_once('/').map_or(rel_path, |(_, f)| f)
}

fn file_stem(rel_path: &str) -> &str {
    let fname = file_name(rel_path);
    fname.rsplit_once('.').map_or(fname, |(s, _)| s)
}

#[allow(dead_code)]
fn normalize_path_buf(path: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for comp in path.components() {
        match comp {
            Component::Normal(s) => {
                let st = s.to_str()?;
                if !st.is_empty() {
                    parts.push(st);
                }
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    let joined = parts.join("/");
    (!joined.is_empty()).then_some(joined)
}

#[allow(dead_code)]
fn read_bounded_manifest_file(path: &Path, max_bytes: u64) -> Option<String> {
    let mut opts = fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = opts.open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > max_bytes {
        return None;
    }
    read_bounded_utf8_stream(file, max_bytes, meta.len().min(max_bytes) as usize)
}

#[allow(dead_code)]
fn read_bounded_utf8_stream<R: Read>(
    reader: R,
    max_bytes: u64,
    capacity_hint: usize,
) -> Option<String> {
    let mut buf = Vec::with_capacity(capacity_hint);
    reader
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut buf)
        .ok()?;
    if buf.len() as u64 > max_bytes {
        return None;
    }
    String::from_utf8(buf).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn known(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn resolves_php_psr4_and_classmap_from_composer_json() {
        let files = known(&[
            "src/Billing/Invoice.php",
            "modules/Auth/src/TokenGuard.php",
            "legacy/LegacyMailer.php",
            "tests/Billing/InvoiceTest.php",
        ]);
        let composer = r#"{
            "autoload": {
                "psr-4": {
                    "App\\": "src/",
                    "Acme\\Auth\\": ["modules/Auth/src/", "modules/Auth/fallback/"]
                },
                "classmap": ["legacy/"]
            },
            "autoload-dev": {
                "psr-4": {
                    "Tests\\": "tests/"
                }
            }
        }"#;
        let idx = ManifestIndex::from_texts(&[("composer.json", composer)]);
        assert_eq!(
            idx.resolve_php_fqn(r"\App\Billing\Invoice", &files)
                .as_deref(),
            Some("src/Billing/Invoice.php")
        );
        assert_eq!(
            idx.resolve_php_fqn("Acme\\Auth\\TokenGuard", &files)
                .as_deref(),
            Some("modules/Auth/src/TokenGuard.php")
        );
        assert_eq!(
            idx.resolve_php_fqn("Tests\\Billing\\InvoiceTest", &files)
                .as_deref(),
            Some("tests/Billing/InvoiceTest.php")
        );
        assert_eq!(
            idx.resolve_php_fqn("LegacyMailer", &files).as_deref(),
            Some("legacy/LegacyMailer.php")
        );
        assert_eq!(
            idx.resolve_php_fqn("Vendor\\External\\Missing", &files),
            None
        );
    }

    #[test]
    fn resolves_tsconfig_paths_jsonc_extends_and_base_url() {
        let files = known(&[
            "packages/core/src/auth/jwt.ts",
            "packages/ui/src/components/Button.tsx",
            "packages/ui/src/utils/index.ts",
            "apps/web/src/config.ts",
        ]);
        let base_tsconfig = r#"{
            // Base monorepo paths
            "compilerOptions": {
                "baseUrl": ".",
                "paths": {
                    "@core/*": ["packages/core/src/*"],
                },
            }
        }"#;
        let web_tsconfig = r#"{
            "extends": "../../tsconfig.base.json",
            /* App overrides */
            "compilerOptions": {
                "baseUrl": "src",
                "paths": {
                    "@ui/*": ["../../../packages/ui/src/*"],
                    "@config": ["config.ts"],
                }
            }
        }"#;
        let idx = ManifestIndex::from_texts(&[
            ("tsconfig.base.json", base_tsconfig),
            ("apps/web/tsconfig.json", web_tsconfig),
        ]);
        assert_eq!(
            idx.resolve_ts_import("apps/web/src/main.ts", "@core/auth/jwt", &files)
                .as_deref(),
            Some("packages/core/src/auth/jwt.ts")
        );
        assert_eq!(
            idx.resolve_ts_import("apps/web/src/main.ts", "@ui/components/Button", &files)
                .as_deref(),
            Some("packages/ui/src/components/Button.tsx")
        );
        assert_eq!(
            idx.resolve_ts_import("apps/web/src/main.ts", "@ui/utils", &files)
                .as_deref(),
            Some("packages/ui/src/utils/index.ts")
        );
        assert_eq!(
            idx.resolve_ts_import("apps/web/src/main.ts", "@config", &files)
                .as_deref(),
            Some("apps/web/src/config.ts")
        );
        assert_eq!(
            idx.resolve_ts_import("apps/web/src/main.ts", "config", &files)
                .as_deref(),
            Some("apps/web/src/config.ts")
        );
    }

    #[test]
    fn resolves_package_json_workspaces_exports_and_subpath_imports() {
        let files = known(&[
            "packages/shared/src/index.ts",
            "packages/shared/src/crypto/hash.ts",
            "apps/api/src/internal/logger.ts",
            "apps/api/src/server.ts",
        ]);
        let root_pkg = r#"{
            "name": "monorepo",
            "workspaces": { "packages": ["packages/*", "apps/*"] }
        }"#;
        let shared_pkg = r#"{
            "name": "@acme/shared",
            "exports": {
                ".": { "import": "./dist/index.js", "types": "./src/index.ts" },
                "./crypto/*": "./src/crypto/*.ts"
            }
        }"#;
        let api_pkg = r##"{
            "name": "@acme/api",
            "imports": {
                "#internal/*": "./src/internal/*.ts"
            }
        }"##;
        let idx = ManifestIndex::from_texts(&[
            ("package.json", root_pkg),
            ("packages/shared/package.json", shared_pkg),
            ("apps/api/package.json", api_pkg),
        ]);
        assert_eq!(
            idx.resolve_ts_import("apps/api/src/server.ts", "@acme/shared", &files)
                .as_deref(),
            Some("packages/shared/src/index.ts")
        );
        assert_eq!(
            idx.resolve_ts_import("apps/api/src/server.ts", "@acme/shared/crypto/hash", &files)
                .as_deref(),
            Some("packages/shared/src/crypto/hash.ts")
        );
        assert_eq!(
            idx.resolve_ts_import("apps/api/src/server.ts", "#internal/logger", &files)
                .as_deref(),
            Some("apps/api/src/internal/logger.ts")
        );
    }

    #[test]
    fn resolves_pyproject_source_roots_and_from_root_skips_ignored_dirs() {
        let files = known(&[
            "services/billing/src/billing_core/invoice.py",
            "services/billing/src/billing_core/__init__.py",
            "libs/shared/custom_dir/helpers.py",
        ]);
        let billing_pyproject = r#"
[project]
name = "billing-core"

[tool.setuptools.packages.find]
where = ["src"]
"#;
        let shared_pyproject = r#"
[tool.poetry]
name = "shared-helpers"
packages = [{ include = "helpers", from = "custom_dir" }]
"#;
        let idx = ManifestIndex::from_texts(&[
            ("services/billing/pyproject.toml", billing_pyproject),
            ("libs/shared/pyproject.toml", shared_pyproject),
        ]);
        assert_eq!(
            idx.resolve_py_import(
                "services/billing/tests/test_inv.py",
                "billing_core.invoice",
                &files
            )
            .as_deref(),
            Some("services/billing/src/billing_core/invoice.py")
        );
        assert_eq!(
            idx.resolve_py_import("services/billing/tests/test_inv.py", "billing_core", &files)
                .as_deref(),
            Some("services/billing/src/billing_core/__init__.py")
        );
        assert_eq!(
            idx.resolve_py_import("libs/shared/app.py", "helpers", &files)
                .as_deref(),
            Some("libs/shared/custom_dir/helpers.py")
        );

        let dir = TempDir::new().expect("temp dir");
        let root = dir.path();
        fs::create_dir_all(root.join("node_modules/fake")).unwrap();
        fs::write(
            root.join("node_modules/fake/package.json"),
            r#"{"name": "fake-pkg"}"#,
        )
        .unwrap();
        fs::write(
            root.join("composer.json"),
            r#"{"autoload":{"psr-4":{"App\\":"src/"}}}"#,
        )
        .unwrap();
        let disk_idx = ManifestIndex::from_root(root);
        assert_eq!(disk_idx.packages.len(), 0);
        assert_eq!(disk_idx.composers.len(), 1);
    }

    #[test]
    fn resolves_go_mod_from_reader_and_direct_package_imports() {
        let files = known(&[
            "go.mod",
            "internal/auth/guard.go",
            "internal/auth/guard_test.go",
            "packages/ui/package.json",
            "packages/ui/src/index.ts",
        ]);
        let reader = |p: &str| match p {
            "go.mod" => Some("module github.com/acme/api\n\ngo 1.23\n".to_owned()),
            "packages/ui/package.json" => {
                Some(r#"{"name":"@acme/ui","main":"./src/index.ts"}"#.to_owned())
            }
            _ => None,
        };
        let path_vec: Vec<String> = files.iter().cloned().collect();
        let idx = ManifestIndex::from_reader(&path_vec, &reader);
        assert!(!idx.is_empty());
        assert_eq!(
            idx.resolve_go_import("github.com/acme/api/internal/auth", &files)
                .as_deref(),
            Some("internal/auth/guard.go")
        );
        assert_eq!(
            idx.resolve_package_import("apps/web/main.ts", "@acme/ui", &files)
                .as_deref(),
            Some("packages/ui/src/index.ts")
        );
    }

    #[test]
    fn from_root_and_bounded_reader_reject_oversized_and_symlink_manifests() {
        let dir = TempDir::new().expect("temp dir");
        let root = dir.path();

        let ok_manifest = r#"{"autoload":{"psr-4":{"App\\":"src/"}}}"#;
        fs::write(root.join("composer.json"), ok_manifest).unwrap();

        let oversized_len = (MAX_MANIFEST_BYTES as usize) + 1;
        let padding = " ".repeat(oversized_len.saturating_sub(2));
        let oversized_pkg = format!("{{{padding}}}");
        assert_eq!(oversized_pkg.len(), oversized_len);
        fs::write(root.join("package.json"), oversized_pkg).unwrap();

        let disk_idx = ManifestIndex::from_root(root);
        assert_eq!(disk_idx.composers.len(), 1);
        assert_eq!(disk_idx.packages.len(), 0);

        // Enforce `.take(max_bytes + 1)` rejection even without prior metadata size knowledge
        // (guards against files growing between `symlink_metadata` and read).
        let over_stream = std::io::Cursor::new(b"{\"name\":\"too-long\"}".to_vec());
        assert!(read_bounded_utf8_stream(over_stream, 8, 8).is_none());

        let exact_stream = std::io::Cursor::new(b"{\"a\":1}".to_vec());
        assert_eq!(
            read_bounded_utf8_stream(exact_stream, 7, 7).as_deref(),
            Some("{\"a\":1}")
        );

        #[cfg(unix)]
        {
            let target = root.join("real_package.json");
            fs::write(&target, r#"{"name":"@acme/sym"}"#).unwrap();
            let link = root.join("sub_package.json");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert!(read_bounded_manifest_file(&link, MAX_MANIFEST_BYTES).is_none());
        }
    }
}
