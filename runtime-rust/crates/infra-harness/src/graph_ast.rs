//! Unified multi-language tree-sitter AST, symbol, reference, and framework/DB semantic extractor.
//!
//! Covers all 14+ `layers::CODE` extensions (`php`, `py`, `rb`, `java`, `kt`, `cs`, `ts`, `tsx`,
//! `js`, `jsx`, `go`, `rs`, `swift`, `scala` plus `.phtml`, `.inc`, `.pyi`, `.mts`, `.cts`, `.mjs`,
//! `.cjs`, `.kts`, `.sc`) using thread-safe per-call `tree_sitter::Parser` instances with bounded
//! file sizes and recursion depth.

use crate::{
    graph::file_types::{IGNORED_CODE_DIRS, is_blade_template},
    graph_deps::DeclaredMember,
    graph_live::{CreatedEntity, ExtractedDecl, Language, clean_rel_key},
};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use tree_sitter::{Language as TsLanguage, Node, Parser};

mod php;
mod polyglot;
mod python;
mod rust_lang;
mod semantics;
#[cfg(test)]
mod tests;
mod ts_js;

pub(crate) use self::semantics::normalize_route_contract;
use self::{
    php::{collect_blade_identifiers, extract_blade, extract_php},
    polyglot::{
        extract_csharp, extract_go, extract_java, extract_kotlin, extract_ruby, extract_scala,
        extract_swift,
    },
    python::extract_python,
    rust_lang::extract_rust,
    semantics::*,
    ts_js::extract_ts_js,
};

thread_local! {
    static TL_PARSER: RefCell<Parser> = RefCell::new(Parser::new());
}

fn parse_tree_with_grammar(grammar: AstGrammar, source: &str) -> Option<tree_sitter::Tree> {
    let ts_lang = grammar.ts_language(source);
    TL_PARSER.with(|cell| {
        let mut parser = cell.borrow_mut();
        parser.set_language(&ts_lang).ok()?;
        parser.parse(source, None)
    })
}

const MAX_AST_BYTES: usize = crate::graph_map::SOURCE_LIMIT as usize;
const MAX_AST_DEPTH: usize = 256;
const MAX_WALK_DEPTH: usize = 64;
const MAX_SQL_LITERAL_BYTES: usize = 8192;

/// Grammar variant selected for tree-sitter parsing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AstGrammar {
    Php,
    Blade,
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Go,
    Java,
    CSharp,
    Ruby,
    Kotlin,
    Swift,
    Scala,
}

impl AstGrammar {
    pub(crate) fn from_rel_path(rel_path: &str) -> Option<Self> {
        if is_blade_template(rel_path) {
            return Some(Self::Blade);
        }
        let ext = rel_path.rsplit_once('.')?.1.to_ascii_lowercase();
        match ext.as_str() {
            "php" | "phtml" | "inc" => Some(Self::Php),
            "rs" => Some(Self::Rust),
            "ts" | "mts" | "cts" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "py" | "pyi" => Some(Self::Python),
            "go" => Some(Self::Go),
            "java" => Some(Self::Java),
            "cs" => Some(Self::CSharp),
            "rb" => Some(Self::Ruby),
            "kt" | "kts" => Some(Self::Kotlin),
            "swift" => Some(Self::Swift),
            "scala" | "sc" => Some(Self::Scala),
            _ => None,
        }
    }

    pub(crate) fn to_live_language(self) -> Language {
        match self {
            Self::Php | Self::Blade => Language::Php,
            Self::Rust => Language::Rust,
            Self::TypeScript | Self::Tsx | Self::JavaScript => Language::TsJs,
            Self::Python => Language::Python,
            Self::Go => Language::Go,
            Self::Java => Language::Java,
            Self::CSharp => Language::CSharp,
            Self::Ruby => Language::Ruby,
            Self::Kotlin => Language::Kotlin,
            Self::Swift => Language::Swift,
            Self::Scala => Language::Scala,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Php => "php",
            Self::Blade => "blade",
            Self::Rust => "rust",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::JavaScript => "javascript",
            Self::Python => "python",
            Self::Go => "go",
            Self::Java => "java",
            Self::CSharp => "csharp",
            Self::Ruby => "ruby",
            Self::Kotlin => "kotlin",
            Self::Swift => "swift",
            Self::Scala => "scala",
        }
    }

    fn ts_language(self, source: &str) -> TsLanguage {
        match self {
            Self::Blade => tree_sitter_php::LANGUAGE_PHP.into(),
            Self::Php => {
                if source.contains("<?") {
                    tree_sitter_php::LANGUAGE_PHP.into()
                } else {
                    tree_sitter_php::LANGUAGE_PHP_ONLY.into()
                }
            }
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
            Self::Java => tree_sitter_java::LANGUAGE.into(),
            Self::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
            Self::Ruby => tree_sitter_ruby::LANGUAGE.into(),
            Self::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
            Self::Swift => tree_sitter_swift::LANGUAGE.into(),
            Self::Scala => tree_sitter_scala::LANGUAGE.into(),
        }
    }
}

/// Structured import, include, mod declaration, or re-export item extracted from AST.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AstImport {
    pub path: String,
    pub symbols: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub is_wildcard: bool,
    pub is_reexport: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reexport_visibility: Option<String>,
    /// `"import"` | `"reexport"` | `"trait_use"` | `"include"` | `"mod_decl"`
    pub kind: String,
    pub line: u32,
    pub evidence: String,
}

/// Class/interface/trait/protocol inheritance or mixin relation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AstInheritance {
    pub child: String,
    pub parent: String,
    /// `"extends"` | `"implements"` | `"uses"` | `"conforms"`
    pub kind: String,
    pub line: u32,
    pub evidence: String,
}

/// Symbol call, instantiation, member read/write, constant access, or typed signature reference.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AstRef {
    pub target_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member: Option<String>,
    /// `"call"` | `"constructor"` | `"field"` | `"parameter_type"` | `"return_type"` | `"generic_arg"`
    pub context: String,
    /// `"call"` | `"read"` | `"write"` | `"static_read"` | `"const"` | `"type"`
    pub access: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enclosing_symbol: Option<String>,
    pub is_static: bool,
    pub line: u32,
    pub evidence: String,
}

/// Framework or database semantic relation extracted from AST nodes, attributes, or SQL literals.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SemanticEdge {
    /// `"exposes_route"` | `"calls_route"` | `"di_binds"` | `"publishes_event"` | `"handles_event"` | `"reads_table"` | `"writes_table"`
    pub kind: String,
    /// Canonical target (`route:<METHOD> <PATH>`, `service:<TYPE>`, `event:<NAME>`, `table:<NAME>`).
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_symbol: Option<String>,
    pub line: u32,
    pub evidence: String,
}

/// Full per-file intermediate representation produced by tree-sitter AST extraction.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AstFileExtraction {
    pub rel_path: String,
    pub language: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    pub declarations: Vec<ExtractedDecl>,
    pub declared_members: Vec<DeclaredMember>,
    pub imports: Vec<AstImport>,
    pub inheritance: Vec<AstInheritance>,
    pub calls_and_refs: Vec<AstRef>,
    pub semantic_edges: Vec<SemanticEdge>,
    #[serde(default)]
    pub test_spans: Vec<(u32, u32)>,
    #[serde(default, skip_serializing)]
    pub identifiers: BTreeSet<String>,
}

/// Parses a single source file into `AstFileExtraction`, returning `None` if unsupported or oversized.
///
/// Thread-safe and bounded; never panics on syntax errors or truncated inputs.
pub(crate) fn parse_file_ir(rel_path: &str, source: &str) -> Option<AstFileExtraction> {
    let rel = clean_rel_key(rel_path);
    if rel.is_empty() || source.len() > MAX_AST_BYTES {
        return None;
    }
    let grammar = AstGrammar::from_rel_path(&rel)?;
    let tree = parse_tree_with_grammar(grammar, source)?;

    let mut ctx = ExtractCtx::new(&rel, grammar, source);
    let root = tree.root_node();
    match grammar {
        AstGrammar::Php => extract_php(&mut ctx, root),
        AstGrammar::Blade => extract_blade(&mut ctx, root),
        AstGrammar::Rust => extract_rust(&mut ctx, root),
        AstGrammar::TypeScript | AstGrammar::Tsx | AstGrammar::JavaScript => {
            extract_ts_js(&mut ctx, root)
        }
        AstGrammar::Python => extract_python(&mut ctx, root),
        AstGrammar::Go => extract_go(&mut ctx, root),
        AstGrammar::Java => extract_java(&mut ctx, root),
        AstGrammar::CSharp => extract_csharp(&mut ctx, root),
        AstGrammar::Ruby => extract_ruby(&mut ctx, root),
        AstGrammar::Kotlin => extract_kotlin(&mut ctx, root),
        AstGrammar::Swift => extract_swift(&mut ctx, root),
        AstGrammar::Scala => extract_scala(&mut ctx, root),
    }
    // Extract SQL queries and code identifiers in one combined traversal, anchoring SQL edges
    // to their tightest enclosing declaration from the language extraction above.
    let mut identifiers = scan_sql_and_identifiers(&mut ctx, root);
    if grammar == AstGrammar::Blade {
        collect_blade_identifiers(source, &mut identifiers);
    }
    let mut out = ctx.into_extraction();
    out.identifiers = identifiers;
    Some(out)
}

/// Extracts `ExtractedDecl` spans for `graph_live.rs` via tree-sitter AST.
pub(crate) fn extract_declarations_for_live(rel_path: &str, source: &str) -> Vec<ExtractedDecl> {
    parse_file_ir(rel_path, source)
        .map(|ir| ir.declarations)
        .unwrap_or_default()
}

/// Extracts `DeclaredMember` summaries via tree-sitter AST.
pub(crate) fn extract_declared_members_ast(rel_path: &str, source: &str) -> Vec<DeclaredMember> {
    parse_file_ir(rel_path, source)
        .map(|ir| ir.declared_members)
        .unwrap_or_default()
}

fn collect_code_identifiers_from_root(root: Node<'_>, source: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut stack = vec![(root, 0usize)];
    while let Some((node, depth)) = stack.pop() {
        if depth >= MAX_AST_DEPTH {
            continue;
        }
        let kind = node.kind();
        if is_comment_kind(kind) || is_string_kind(kind) {
            continue;
        }
        if node.child_count() == 0 {
            let text = node_text(node, source);
            let clean = text.strip_prefix('$').unwrap_or(text);
            if !clean.is_empty()
                && clean
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            {
                out.insert(clean.to_owned());
                if text.starts_with('$') {
                    out.insert(text.to_owned());
                }
            }
            continue;
        }
        for i in (0..node.child_count()).rev() {
            if let Some(child) = node.child(i) {
                stack.push((child, depth + 1));
            }
        }
    }
    out
}

/// Extracts all non-comment, non-string code identifiers from `source` via tree-sitter AST.
pub(crate) fn extract_code_identifiers(rel_path: &str, source: &str) -> BTreeSet<String> {
    let rel = clean_rel_key(rel_path);
    let Some(grammar) = AstGrammar::from_rel_path(&rel) else {
        return BTreeSet::new();
    };
    if source.len() > MAX_AST_BYTES {
        return BTreeSet::new();
    }
    let Some(tree) = parse_tree_with_grammar(grammar, source) else {
        return BTreeSet::new();
    };
    let mut out = collect_code_identifiers_from_root(tree.root_node(), source);
    if grammar == AstGrammar::Blade {
        collect_blade_identifiers(source, &mut out);
    }
    out
}

/// Manifest basenames whose edits alter cross-file import / workspace resolution.
pub(crate) const GRAPH_MANIFEST_FILES: &[&str] = &[
    "composer.json",
    "tsconfig.json",
    "jsconfig.json",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "cargo.toml",
];

/// Architecture rules file whose edits alter layer classification without code reparsing.
pub(crate) const GRAPH_ARCHITECTURE_FILE: &str = ".harness-architecture.json";

/// Returns `true` when `path` is a workspace manifest affecting import resolution.
pub(crate) fn is_graph_manifest_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            let lower = name.to_ascii_lowercase();
            GRAPH_MANIFEST_FILES.contains(&lower.as_str())
                || ((lower.starts_with("tsconfig.") || lower.starts_with("jsconfig."))
                    && lower.ends_with(".json"))
        })
}

/// Returns `true` when `path` is the repository architecture rules file.
pub(crate) fn is_graph_architecture_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(GRAPH_ARCHITECTURE_FILE))
}

fn path_in_ignored_dir(path: &Path) -> bool {
    let mut prev = String::new();
    for comp in path.components() {
        if let std::path::Component::Normal(raw) = comp
            && let Some(name) = raw.to_str()
        {
            if IGNORED_CODE_DIRS.contains(&name) {
                return true;
            }
            if prev == "memory-bank" && name == "local" {
                return true;
            }
            prev = name.to_owned();
        }
    }
    false
}

/// Filters `notify::Event` paths to code files, dependency manifests, `.harness-architecture.json`,
/// and non-ignored directory events (create/remove/rename) so directory deletes/renames are tracked.
pub(crate) fn notify_event_touches_code(event: &notify::Event) -> Vec<PathBuf> {
    use notify::{EventKind, event::ModifyKind};
    if !matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) {
        return Vec::new();
    }
    let dir_capable = matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
    );
    event
        .paths
        .iter()
        .filter(|path| {
            if path_in_ignored_dir(path) {
                return false;
            }
            if path
                .to_str()
                .is_some_and(|s| AstGrammar::from_rel_path(s).is_some())
                || is_graph_manifest_path(path)
                || is_graph_architecture_path(path)
            {
                return true;
            }
            dir_capable && (path.extension().is_none() || path.is_dir())
        })
        .cloned()
        .collect()
}

/// Creates a `notify::RecommendedWatcher` delivering changed code paths for Wave 5B incremental re-indexing.
#[allow(dead_code)]
pub(crate) fn create_code_watcher<F>(mut on_paths: F) -> notify::Result<notify::RecommendedWatcher>
where
    F: FnMut(Vec<PathBuf>) + Send + 'static,
{
    notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            let paths = notify_event_touches_code(&event);
            if !paths.is_empty() {
                on_paths(paths);
            }
        }
    })
}

/// Helper to watch a directory with `notify::Watcher`.
#[allow(dead_code)]
pub(crate) fn watch_root_recursive(
    watcher: &mut dyn notify::Watcher,
    root: &Path,
) -> notify::Result<()> {
    watcher.watch(root, notify::RecursiveMode::Recursive)
}

// =========================================================================
// Shared Extraction Context & Builders
// =========================================================================

type TsRouterLeaves = Vec<(String, String, Option<String>, u32, String)>;

struct ExtractCtx<'a> {
    rel_path: String,
    grammar: AstGrammar,
    source: &'a str,
    namespace: Option<String>,
    declarations: Vec<ExtractedDecl>,
    imports: Vec<AstImport>,
    inheritance: Vec<AstInheritance>,
    calls_and_refs: Vec<AstRef>,
    semantic_edges: Vec<SemanticEdge>,
    test_spans: Vec<(u32, u32)>,
    test_lines: BTreeSet<u32>,
    seen_semantic: BTreeSet<(String, String, u32)>,
    seen_decls: BTreeSet<(String, Option<String>, String, u32)>,
    attr_route_prefix_stack: Vec<String>,
    call_route_prefix_stack: Vec<String>,
    ts_router_leaves: BTreeMap<String, TsRouterLeaves>,
}

impl<'a> ExtractCtx<'a> {
    fn new(rel_path: &str, grammar: AstGrammar, source: &'a str) -> Self {
        Self {
            rel_path: rel_path.to_owned(),
            grammar,
            source,
            namespace: None,
            declarations: Vec::new(),
            imports: Vec::new(),
            inheritance: Vec::new(),
            calls_and_refs: Vec::new(),
            semantic_edges: Vec::new(),
            test_spans: Vec::new(),
            test_lines: BTreeSet::new(),
            seen_semantic: BTreeSet::new(),
            seen_decls: BTreeSet::new(),
            attr_route_prefix_stack: Vec::new(),
            call_route_prefix_stack: Vec::new(),
            ts_router_leaves: BTreeMap::new(),
        }
    }

    fn compose_segments(prefixes: &[String], leaf_path: &str) -> String {
        let leaf = leaf_path.trim();
        if prefixes.is_empty() || leaf.starts_with("http://") || leaf.starts_with("https://") {
            return leaf.to_owned();
        }
        let mut segments: Vec<&str> = Vec::new();
        for pref in prefixes {
            for seg in pref.trim().split('/').filter(|s| !s.is_empty()) {
                segments.push(seg);
            }
        }
        let (leaf_route, leaf_suffix) = match leaf.find(['?', '#']) {
            Some(idx) => (&leaf[..idx], &leaf[idx..]),
            None => (leaf, ""),
        };
        for seg in leaf_route.split('/').filter(|s| !s.is_empty()) {
            segments.push(seg);
        }
        if segments.is_empty() {
            format!("/{leaf_suffix}")
        } else {
            format!("/{}{}", segments.join("/"), leaf_suffix)
        }
    }

    fn push_attr_route_prefix(&mut self, prefix: &str) -> usize {
        let mark = self.attr_route_prefix_stack.len();
        let clean = prefix.trim();
        if !clean.is_empty() && clean != "/" {
            self.attr_route_prefix_stack.push(clean.to_owned());
        }
        mark
    }

    fn truncate_attr_route_prefixes(&mut self, len: usize) {
        self.attr_route_prefix_stack.truncate(len);
    }

    fn compose_attr_route_path(&self, leaf_path: &str) -> String {
        Self::compose_segments(&self.attr_route_prefix_stack, leaf_path)
    }

    fn push_call_route_prefix(&mut self, prefix: &str) -> usize {
        let mark = self.call_route_prefix_stack.len();
        let clean = prefix.trim();
        if !clean.is_empty() && clean != "/" {
            self.call_route_prefix_stack.push(clean.to_owned());
        }
        mark
    }

    fn truncate_call_route_prefixes(&mut self, len: usize) {
        self.call_route_prefix_stack.truncate(len);
    }

    fn compose_call_route_path(&self, leaf_path: &str) -> String {
        Self::compose_segments(&self.call_route_prefix_stack, leaf_path)
    }

    fn text(&self, node: Node<'_>) -> &'a str {
        node_text(node, self.source)
    }

    #[allow(clippy::too_many_arguments)]
    fn push_decl(
        &mut self,
        kind: &str,
        name: &str,
        parent: Option<String>,
        line: u32,
        end_line: u32,
        visibility: Option<String>,
        modifiers: Vec<String>,
        signature: String,
        params_signature: String,
        body_fingerprint: String,
    ) {
        let clean_name = name.trim();
        if clean_name.is_empty() {
            return;
        }
        let parent_val = parent.or_else(|| Some(self.rel_path.clone()));
        let key = (
            kind.to_owned(),
            parent_val.clone(),
            clean_name.to_owned(),
            line,
        );
        if !self.seen_decls.insert(key) {
            return;
        }
        let entity = CreatedEntity::new(kind, clean_name, parent_val, &self.rel_path, Some(line))
            .with_access(visibility.clone(), modifiers.clone());
        self.declarations.push(ExtractedDecl {
            entity,
            signature,
            params_signature,
            body_fingerprint,
            visibility,
            modifiers,
            start_line: line,
            end_line: end_line.max(line),
        });
    }

    fn push_import(&mut self, import: AstImport) {
        if !import.path.trim().is_empty() {
            self.imports.push(import);
        }
    }

    fn push_inheritance(
        &mut self,
        child: &str,
        parent: &str,
        kind: &str,
        line: u32,
        evidence: &str,
    ) {
        let c = child.trim();
        let p = parent.trim();
        if !c.is_empty() && !p.is_empty() {
            self.inheritance.push(AstInheritance {
                child: c.to_owned(),
                parent: p.to_owned(),
                kind: kind.to_owned(),
                line,
                evidence: clip_str(evidence, 160),
            });
        }
    }

    fn push_ref(&mut self, r: AstRef) {
        if !r.target_name.trim().is_empty() {
            self.calls_and_refs.push(r);
        }
    }

    fn push_semantic(
        &mut self,
        kind: &str,
        target: String,
        source_symbol: Option<String>,
        line: u32,
        evidence: &str,
    ) {
        if target.is_empty() {
            return;
        }
        let key = (kind.to_owned(), target.clone(), line);
        if self.seen_semantic.insert(key) {
            self.semantic_edges.push(SemanticEdge {
                kind: kind.to_owned(),
                target,
                source_symbol,
                line,
                evidence: clip_str(evidence, 160),
            });
        }
    }

    fn into_extraction(self) -> AstFileExtraction {
        let rel = self.rel_path.clone();
        let declared_members = self
            .declarations
            .iter()
            .map(|decl| {
                let parent = decl
                    .entity
                    .parent
                    .clone()
                    .filter(|p| !p.is_empty() && *p != rel);
                let signature = if decl.signature.is_empty() {
                    None
                } else {
                    Some(decl.signature.clone())
                };
                DeclaredMember {
                    name: decl.entity.name.clone(),
                    kind: decl.entity.kind.clone(),
                    parent,
                    line: decl.entity.line,
                    signature,
                    visibility: decl.visibility.clone(),
                    previous_visibility: None,
                    modifiers: decl.modifiers.clone(),
                    change: "unchanged".to_owned(),
                    signature_changed: false,
                    visibility_narrowed: false,
                    used_by_caller: false,
                }
            })
            .collect();

        let mut test_spans = self.test_spans;
        if !self.test_lines.is_empty() {
            for decl in &self.declarations {
                let near_attr = self
                    .test_lines
                    .range(decl.start_line.saturating_sub(3)..=decl.start_line)
                    .next()
                    .is_some();
                if near_attr {
                    test_spans.push((decl.start_line, decl.end_line.max(decl.start_line)));
                }
            }
            for &line in &self.test_lines {
                test_spans.push((line, line));
            }
        }
        test_spans.sort_unstable();
        test_spans.dedup();

        AstFileExtraction {
            rel_path: self.rel_path,
            language: self.grammar.label().to_owned(),
            namespace: self.namespace,
            declarations: self.declarations,
            declared_members,
            imports: self.imports,
            inheritance: self.inheritance,
            calls_and_refs: self.calls_and_refs,
            semantic_edges: self.semantic_edges,
            test_spans,
            identifiers: BTreeSet::new(),
        }
    }
}

// =========================================================================
// Low-level Tree-sitter Utilities
// =========================================================================

fn node_text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    source.get(node.start_byte()..node.end_byte()).unwrap_or("")
}

fn start_line(node: Node<'_>) -> u32 {
    node.start_position().row as u32 + 1
}

fn end_line(node: Node<'_>) -> u32 {
    node.end_position().row as u32 + 1
}

fn is_comment_kind(kind: &str) -> bool {
    matches!(
        kind,
        "comment" | "line_comment" | "block_comment" | "doc_comment" | "multiline_comment"
    )
}

fn is_string_kind(kind: &str) -> bool {
    kind.contains("string")
        || matches!(
            kind,
            "heredoc"
                | "nowdoc"
                | "character_literal"
                | "char_literal"
                | "line_string_literal"
                | "multi_line_string_literal"
        )
}

fn clip_str(raw: &str, max_len: usize) -> String {
    let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.len() <= max_len {
        return compact;
    }
    let mut end = max_len;
    while end > 0 && !compact.is_char_boundary(end) {
        end -= 1;
    }
    compact[..end].to_owned()
}

/// Extracts text inside a header span `[start_byte..end_byte]`, excluding all descendant comment nodes.
fn header_without_comments(node: Node<'_>, end_byte: usize, source: &str) -> String {
    let start = node.start_byte();
    let end = end_byte.min(node.end_byte()).max(start);
    if start >= end || end > source.len() {
        return String::new();
    }
    let mut comments: Vec<(usize, usize)> = Vec::new();
    let mut stack = vec![(node, 0usize)];
    while let Some((cur, depth)) = stack.pop() {
        if depth >= MAX_AST_DEPTH || cur.start_byte() >= end {
            continue;
        }
        if is_comment_kind(cur.kind()) {
            let cs = cur.start_byte().max(start);
            let ce = cur.end_byte().min(end);
            if cs < ce {
                comments.push((cs, ce));
            }
            continue;
        }
        for i in (0..cur.child_count()).rev() {
            if let Some(ch) = cur.child(i) {
                stack.push((ch, depth + 1));
            }
        }
    }
    comments.sort_by_key(|r| r.0);

    let mut raw = String::with_capacity(end - start);
    let mut cursor = start;
    for (cs, ce) in comments {
        if cs > cursor
            && let Some(slice) = source.get(cursor..cs)
        {
            raw.push_str(slice);
        }
        cursor = cursor.max(ce);
        raw.push(' ');
    }
    if cursor < end
        && let Some(slice) = source.get(cursor..end)
    {
        raw.push_str(slice);
    }
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Computes an AST-normalized body fingerprint excluding comments and normalizing whitespace.
fn ast_body_fingerprint(node: Node<'_>, source: &str) -> String {
    let mut tokens: Vec<&str> = Vec::new();
    let mut stack = vec![(node, 0usize)];
    while let Some((cur, depth)) = stack.pop() {
        if depth >= MAX_AST_DEPTH {
            continue;
        }
        if is_comment_kind(cur.kind()) {
            continue;
        }
        if cur.child_count() == 0 {
            let txt = node_text(cur, source).trim();
            if !txt.is_empty() {
                tokens.push(txt);
            }
            continue;
        }
        for i in (0..cur.child_count()).rev() {
            if let Some(ch) = cur.child(i) {
                stack.push((ch, depth + 1));
            }
        }
    }
    if tokens.is_empty() {
        return String::new();
    }
    let joined = tokens.join(" ");
    format!("{:032x}", xxhash_rust::xxh3::xxh3_128(joined.as_bytes()))
}

fn unquote_literal(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.len() < 2 {
        return None;
    }
    for delim in ["\"\"\"", "'''"] {
        if t.starts_with(delim) && t.ends_with(delim) && t.len() >= delim.len() * 2 {
            return Some(t[delim.len()..t.len() - delim.len()].to_owned());
        }
    }
    if let Some(inner) = t.strip_prefix("r#\"").and_then(|s| s.strip_suffix("\"#")) {
        return Some(inner.to_owned());
    }
    if let Some(inner) = t.strip_prefix("r\"").and_then(|s| s.strip_suffix('"')) {
        return Some(inner.to_owned());
    }
    let first = t.chars().next()?;
    let last = t.chars().next_back()?;
    if matches!(first, '"' | '\'' | '`') && first == last {
        return Some(t[1..t.len() - 1].to_owned());
    }
    None
}

fn is_upper_snake(name: &str) -> bool {
    let bare = name.strip_prefix('$').unwrap_or(name);
    bare.len() >= 2
        && bare.chars().any(|c| c.is_ascii_uppercase())
        && bare
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn push_unique_mod(mods: &mut Vec<String>, m: &str) {
    if !m.is_empty() && !mods.iter().any(|existing| existing == m) {
        mods.push(m.to_owned());
    }
}

fn child_by_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i)
            && ch.kind() == kind
        {
            return Some(ch);
        }
    }
    None
}

fn children_by_kind<'a>(node: Node<'a>, kind: &str) -> Vec<Node<'a>> {
    let mut out = Vec::new();
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i)
            && ch.kind() == kind
        {
            out.push(ch);
        }
    }
    out
}

fn first_string_arg_text(args_node: Node<'_>, source: &str) -> Option<String> {
    let mut stack = vec![(args_node, 0usize)];
    while let Some((cur, depth)) = stack.pop() {
        if depth >= 16 {
            continue;
        }
        if is_string_kind(cur.kind())
            && let Some(val) = unquote_literal(node_text(cur, source))
        {
            return Some(val);
        }
        for i in (0..cur.child_count()).rev() {
            if let Some(ch) = cur.child(i) {
                stack.push((ch, depth + 1));
            }
        }
    }
    None
}

fn all_string_arg_texts(args_node: Node<'_>, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![(args_node, 0usize)];
    while let Some((cur, depth)) = stack.pop() {
        if depth >= 16 {
            continue;
        }
        if is_string_kind(cur.kind())
            && let Some(val) = unquote_literal(node_text(cur, source))
        {
            out.push(val);
            continue;
        }
        for i in (0..cur.child_count()).rev() {
            if let Some(ch) = cur.child(i) {
                stack.push((ch, depth + 1));
            }
        }
    }
    out
}

fn extract_class_ref_name(raw: &str) -> Option<String> {
    let t = raw
        .trim()
        .trim_matches(|c: char| matches!(c, '(' | ')' | '[' | ']' | ' ' | '\t' | '\n' | '\r'))
        .trim();
    for suffix in ["::class", ".class", "::TYPE"] {
        if let Some(base) = t.strip_suffix(suffix) {
            let short = base
                .rsplit(['\\', ':', '.'])
                .next()
                .unwrap_or(base)
                .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
            if !short.is_empty() {
                return Some(short.to_owned());
            }
        }
    }
    None
}

// =========================================================================
// Semantic Extraction Helpers (SQL, Routes, DI, Events)
// =========================================================================

impl ExtractCtx<'_> {
    fn enclosing_symbol_at_line(&self, line: u32) -> Option<String> {
        if line == 0 {
            return None;
        }
        let mut best: Option<&ExtractedDecl> = None;
        let mut best_span = u32::MAX;
        for decl in &self.declarations {
            if line >= decl.start_line && line <= decl.end_line {
                let span = decl.end_line.saturating_sub(decl.start_line);
                let better = match best {
                    None => true,
                    Some(prev) => {
                        span < best_span || (span == best_span && decl.start_line > prev.start_line)
                    }
                };
                if better {
                    best = Some(decl);
                    best_span = span;
                }
            }
        }
        let decl = best?;
        match decl.entity.parent.as_deref() {
            Some(owner) if owner != self.rel_path => Some(format!("{owner}::{}", decl.entity.name)),
            _ => Some(decl.entity.name.clone()),
        }
    }
}

fn scan_sql_and_identifiers(ctx: &mut ExtractCtx<'_>, root: Node<'_>) -> BTreeSet<String> {
    let mut identifiers = BTreeSet::new();
    let mut stack = vec![(root, 0usize)];
    while let Some((node, depth)) = stack.pop() {
        if depth >= MAX_AST_DEPTH {
            continue;
        }
        let kind = node.kind();
        if is_comment_kind(kind) {
            continue;
        }
        if is_string_kind(kind) {
            let raw = node_text(node, ctx.source);
            if raw.len() <= MAX_SQL_LITERAL_BYTES
                && let Some(val) = unquote_literal(raw)
            {
                let line = start_line(node);
                let owner = ctx.enclosing_symbol_at_line(line);
                emit_sql_table_edges(ctx, &val, owner, line);
            }
            continue;
        }
        if node.child_count() == 0 {
            let text = node_text(node, ctx.source);
            let clean = text.strip_prefix('$').unwrap_or(text);
            if !clean.is_empty()
                && clean
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            {
                identifiers.insert(clean.to_owned());
                if text.starts_with('$') {
                    identifiers.insert(text.to_owned());
                }
            }
            continue;
        }
        for i in (0..node.child_count()).rev() {
            if let Some(ch) = node.child(i) {
                stack.push((ch, depth + 1));
            }
        }
    }
    identifiers
}
