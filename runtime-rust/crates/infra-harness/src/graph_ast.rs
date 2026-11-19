//! Unified multi-language tree-sitter AST, symbol, reference, and framework/DB semantic extractor.
//!
//! Covers all 14+ `layers::CODE` extensions (`php`, `py`, `rb`, `java`, `kt`, `cs`, `ts`, `tsx`,
//! `js`, `jsx`, `go`, `rs`, `swift`, `scala` plus `.phtml`, `.inc`, `.pyi`, `.mts`, `.cts`, `.mjs`,
//! `.cjs`, `.kts`, `.sc`) using thread-safe per-call `tree_sitter::Parser` instances with bounded
//! file sizes and recursion depth.

use crate::{
    graph_deps::DeclaredMember,
    graph_live::{CreatedEntity, ExtractedDecl, Language, clean_rel_key},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use tree_sitter::{Language as TsLanguage, Node, Parser};

const MAX_AST_BYTES: usize = crate::graph_map::SOURCE_LIMIT as usize;
const MAX_AST_DEPTH: usize = 256;
const MAX_WALK_DEPTH: usize = 64;
const MAX_SQL_LITERAL_BYTES: usize = 8192;

/// Grammar variant selected for tree-sitter parsing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AstGrammar {
    Php,
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
        let leaf = rel_path.rsplit('/').next().unwrap_or(rel_path);
        if leaf.to_ascii_lowercase().ends_with(".blade.php") {
            return None;
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
            Self::Php => Language::Php,
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
    let mut parser = Parser::new();
    let ts_lang = grammar.ts_language(source);
    parser.set_language(&ts_lang).ok()?;
    let tree = parser.parse(source, None)?;

    let mut ctx = ExtractCtx::new(&rel, grammar, source);
    let root = tree.root_node();
    match grammar {
        AstGrammar::Php => extract_php(&mut ctx, root),
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
    // Extract SQL queries from any string literal node across all languages.
    scan_sql_string_literals(&mut ctx, root);
    Some(ctx.into_extraction())
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

/// Extracts all non-comment, non-string code identifiers from `source` via tree-sitter AST.
pub(crate) fn extract_code_identifiers(rel_path: &str, source: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let rel = clean_rel_key(rel_path);
    let Some(grammar) = AstGrammar::from_rel_path(&rel) else {
        return out;
    };
    if source.len() > MAX_AST_BYTES {
        return out;
    }
    let mut parser = Parser::new();
    let ts_lang = grammar.ts_language(source);
    if parser.set_language(&ts_lang).is_err() {
        return out;
    }
    let Some(tree) = parser.parse(source, None) else {
        return out;
    };
    let mut stack = vec![(tree.root_node(), 0usize)];
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
            if matches!(
                name,
                ".git"
                    | ".hg"
                    | ".svn"
                    | "graphify-out"
                    | "node_modules"
                    | "vendor"
                    | "target"
                    | "dist"
                    | "build"
                    | ".venv"
                    | "venv"
                    | "__pycache__"
                    | ".next"
                    | ".nuxt"
                    | ".cargo"
                    | ".cache"
                    | ".pytest_cache"
                    | ".mypy_cache"
                    | ".idea"
                    | ".vscode"
                    | ".c2p"
            ) {
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
        }
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

fn scan_sql_string_literals(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    let mut stack = vec![(root, 0usize)];
    while let Some((node, depth)) = stack.pop() {
        if depth >= MAX_AST_DEPTH {
            continue;
        }
        if is_string_kind(node.kind()) {
            let raw = node_text(node, ctx.source);
            if raw.len() <= MAX_SQL_LITERAL_BYTES
                && let Some(val) = unquote_literal(raw)
            {
                let line = start_line(node);
                emit_sql_table_edges(ctx, &val, None, line);
            }
            continue;
        }
        for i in (0..node.child_count()).rev() {
            if let Some(ch) = node.child(i) {
                stack.push((ch, depth + 1));
            }
        }
    }
}

fn emit_sql_table_edges(
    ctx: &mut ExtractCtx<'_>,
    sql: &str,
    source_symbol: Option<String>,
    line: u32,
) {
    let upper = sql.to_ascii_uppercase();
    let has_select = contains_sql_verb(&upper, "SELECT") && contains_sql_verb(&upper, "FROM");
    let has_insert = contains_sql_verb(&upper, "INSERT") && contains_sql_verb(&upper, "INTO");
    let has_replace = contains_sql_verb(&upper, "REPLACE") && contains_sql_verb(&upper, "INTO");
    let has_merge = contains_sql_verb(&upper, "MERGE") && contains_sql_verb(&upper, "INTO");
    let has_update = contains_sql_verb(&upper, "UPDATE") && contains_sql_verb(&upper, "SET");
    let has_delete = contains_sql_verb(&upper, "DELETE") && contains_sql_verb(&upper, "FROM");
    let has_truncate = contains_sql_verb(&upper, "TRUNCATE");
    if !(has_select
        || has_insert
        || has_replace
        || has_merge
        || has_update
        || has_delete
        || has_truncate)
    {
        return;
    }

    let tokens = tokenize_sql(sql);
    let mut cte_names: BTreeSet<String> = BTreeSet::new();
    for i in 0..tokens.len() {
        let up = tokens[i].to_ascii_uppercase();
        if up == "WITH" {
            let mut j = i + 1;
            if tokens
                .get(j)
                .is_some_and(|t| t.eq_ignore_ascii_case("RECURSIVE"))
            {
                j += 1;
            }
            if let Some(name) = tokens.get(j).and_then(|t| clean_sql_table_name(t))
                && tokens
                    .get(j + 1)
                    .is_some_and(|t| t.eq_ignore_ascii_case("AS"))
            {
                cte_names.insert(name.to_ascii_lowercase());
            }
        } else if up == ","
            && let Some(name) = tokens.get(i + 1).and_then(|t| clean_sql_table_name(t))
            && tokens
                .get(i + 2)
                .is_some_and(|t| t.eq_ignore_ascii_case("AS"))
            && tokens.get(i + 3).is_some_and(|t| *t == "(")
        {
            cte_names.insert(name.to_ascii_lowercase());
        }
    }

    let mut i = 0;
    while i < tokens.len() {
        let tok_up = tokens[i].to_ascii_uppercase();
        if tok_up == "FROM" && (has_select || has_delete) {
            let mut j = i + 1;
            let mut expect_table = true;
            while j < tokens.len() {
                let raw = tokens[j];
                let up = raw.to_ascii_uppercase();
                if matches!(
                    up.as_str(),
                    "WHERE"
                        | "JOIN"
                        | "INNER"
                        | "LEFT"
                        | "RIGHT"
                        | "FULL"
                        | "OUTER"
                        | "CROSS"
                        | "NATURAL"
                        | "ON"
                        | "USING"
                        | "GROUP"
                        | "ORDER"
                        | "HAVING"
                        | "LIMIT"
                        | "OFFSET"
                        | "UNION"
                        | "EXCEPT"
                        | "INTERSECT"
                        | "SET"
                        | "VALUES"
                        | "RETURNING"
                        | "WINDOW"
                        | "FOR"
                        | ")"
                        | ";"
                ) {
                    break;
                }
                if raw == "," {
                    expect_table = true;
                    j += 1;
                    continue;
                }
                if raw == "(" {
                    // Derived table subquery `(SELECT ...)`: let outer loop scan inside.
                    break;
                }
                if expect_table && let Some(tbl) = clean_sql_table_name(raw) {
                    if !cte_names.contains(&tbl.to_ascii_lowercase()) {
                        ctx.push_semantic(
                            "reads_table",
                            format!("table:{tbl}"),
                            source_symbol.clone(),
                            line,
                            sql,
                        );
                    }
                    expect_table = false;
                }
                j += 1;
            }
            i = j;
            continue;
        } else if tok_up == "JOIN" && has_select {
            if let Some(tbl) = tokens.get(i + 1).and_then(|t| clean_sql_table_name(t))
                && !cte_names.contains(&tbl.to_ascii_lowercase())
            {
                ctx.push_semantic(
                    "reads_table",
                    format!("table:{tbl}"),
                    source_symbol.clone(),
                    line,
                    sql,
                );
            }
        } else if ((tok_up == "INTO" && (has_insert || has_replace || has_merge))
            || (tok_up == "UPDATE" && has_update))
            && let Some(tbl) = tokens.get(i + 1).and_then(|t| clean_sql_table_name(t))
            && !cte_names.contains(&tbl.to_ascii_lowercase())
        {
            ctx.push_semantic(
                "writes_table",
                format!("table:{tbl}"),
                source_symbol.clone(),
                line,
                sql,
            );
        } else if tok_up == "TRUNCATE" && has_truncate {
            let next_idx = if tokens
                .get(i + 1)
                .is_some_and(|t| t.eq_ignore_ascii_case("TABLE"))
            {
                i + 2
            } else {
                i + 1
            };
            if let Some(tbl) = tokens.get(next_idx).and_then(|t| clean_sql_table_name(t)) {
                ctx.push_semantic(
                    "writes_table",
                    format!("table:{tbl}"),
                    source_symbol.clone(),
                    line,
                    sql,
                );
            }
        }
        i += 1;
    }
}

fn tokenize_sql(sql: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i];
        if ch.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if matches!(ch, b',' | b'(' | b')' | b';') {
            out.push(&sql[i..i + 1]);
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && !matches!(bytes[i], b',' | b'(' | b')' | b';')
        {
            i += 1;
        }
        out.push(&sql[start..i]);
    }
    out
}

fn contains_sql_verb(upper: &str, verb: &str) -> bool {
    upper
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|tok| tok == verb)
}

fn clean_sql_table_name(raw: &str) -> Option<String> {
    let trimmed = raw
        .trim()
        .trim_matches(|c| matches!(c, '`' | '"' | '[' | ']' | '\'' | ';'));
    if trimmed.is_empty() {
        return None;
    }
    let up = trimmed.to_ascii_uppercase();
    if matches!(
        up.as_str(),
        "SELECT"
            | "WHERE"
            | "SET"
            | "VALUES"
            | "FROM"
            | "JOIN"
            | "INNER"
            | "LEFT"
            | "RIGHT"
            | "OUTER"
            | "CROSS"
            | "ON"
            | "AS"
            | "AND"
            | "OR"
            | "NULL"
            | "TRUE"
            | "FALSE"
            | "DUAL"
            | "LATERAL"
            | "ONLY"
    ) {
        return None;
    }
    if trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        && trimmed
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
    {
        Some(trimmed.to_owned())
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_semantic_from_call(
    ctx: &mut ExtractCtx<'_>,
    receiver: Option<&str>,
    callee: &str,
    args_text: &str,
    string_args: &[String],
    full_chain_text: &str,
    source_symbol: Option<String>,
    line: u32,
) {
    let recv_lower = receiver.unwrap_or("").to_ascii_lowercase();
    let callee_lower = callee.to_ascii_lowercase();
    let evidence = full_chain_text;

    // 1. Routes (`Route::get('/x')`, `app.get('/x')`, `.route('/x')`, `HandleFunc('/x')`, `r.GET('/x')`)
    let mut route_emitted = false;
    let http_verb = match callee_lower.as_str() {
        "get" | "mapget" => Some("GET"),
        "post" | "mappost" => Some("POST"),
        "put" | "mapput" => Some("PUT"),
        "patch" | "mappatch" => Some("PATCH"),
        "delete" | "mapdelete" => Some("DELETE"),
        "options" => Some("OPTIONS"),
        "head" => Some("HEAD"),
        "any" | "match" | "route" | "handle" | "handlefunc" | "resource" | "apiresource" => {
            Some("ANY")
        }
        _ => None,
    };
    if let Some(verb) = http_verb {
        let is_route_receiver = recv_lower.is_empty()
            || recv_lower.contains("route")
            || recv_lower.contains("app")
            || recv_lower.contains("mux")
            || recv_lower.contains("fastify")
            || recv_lower.contains("server")
            || recv_lower.contains("hono")
            || recv_lower.contains("chi")
            || recv_lower.contains("gin")
            || recv_lower.contains("echo")
            || matches!(recv_lower.as_str(), "r" | "e" | "g" | "web")
            || (recv_lower == "http" && matches!(callee_lower.as_str(), "handle" | "handlefunc"));
        if is_route_receiver && let Some(path_arg) = string_args.first() {
            let clean_path = path_arg.trim();
            if !clean_path.is_empty()
                && (clean_path.starts_with('/')
                    || recv_lower.contains("route")
                    || callee_lower == "route"
                    || callee_lower.starts_with("handle")
                    || callee_lower.starts_with("map"))
            {
                let target = if verb == "ANY" {
                    format!("route:{clean_path}")
                } else {
                    format!("route:{verb} {clean_path}")
                };
                ctx.push_semantic(
                    "exposes_route",
                    target,
                    source_symbol.clone(),
                    line,
                    evidence,
                );
                route_emitted = true;
            }
        }
    }

    let args_inner = args_text.trim();
    let args_inner = args_inner.strip_prefix('(').unwrap_or(args_inner);
    let args_inner = args_inner.strip_suffix(')').unwrap_or(args_inner).trim();

    if !route_emitted {
        emit_http_client_route_call(
            ctx,
            &recv_lower,
            &callee_lower,
            args_inner,
            string_args,
            source_symbol.as_deref(),
            line,
            evidence,
        );
    }

    // 2. DI bindings (`bind`, `singleton`, `scoped`, `AddSingleton`, `AddScoped`, `AddTransient`, `provide`)
    if matches!(
        callee_lower.as_str(),
        "bind" | "singleton" | "scoped" | "addsingleton" | "addscoped" | "addtransient" | "provide"
    ) {
        let targets = extract_di_targets(args_inner, args_text, string_args, full_chain_text);
        for svc in targets {
            ctx.push_semantic(
                "di_binds",
                format!("service:{svc}"),
                source_symbol.clone(),
                line,
                evidence,
            );
        }
    }

    // 3. Domain events (`event(...)`, `dispatch(...)`, `emit(...)`, `publish(...)`, `Event::listen(...)`)
    if matches!(
        callee_lower.as_str(),
        "event"
            | "dispatch"
            | "dispatchsync"
            | "dispatchafterresponse"
            | "emit"
            | "publish"
            | "publishevent"
    ) {
        let event_name =
            extract_event_name_from_args(receiver, &callee_lower, args_inner, string_args);
        if let Some(ev) = event_name {
            ctx.push_semantic(
                "publishes_event",
                format!("event:{ev}"),
                source_symbol.clone(),
                line,
                evidence,
            );
        }
    } else if matches!(callee_lower.as_str(), "listen" | "subscribe" | "on")
        && (recv_lower.contains("event")
            || recv_lower.contains("bus")
            || recv_lower.contains("emitter")
            || recv_lower.contains("broker")
            || recv_lower.contains("pubsub")
            || callee_lower == "listen")
        && let Some(ev) =
            extract_event_name_from_args(receiver, &callee_lower, args_inner, string_args)
    {
        ctx.push_semantic(
            "handles_event",
            format!("event:{ev}"),
            source_symbol.clone(),
            line,
            evidence,
        );
    }

    // 4. ORM `DB::table('tbl')`
    if callee_lower == "table"
        && (recv_lower == "db" || recv_lower == "schema" || recv_lower.ends_with("\\db"))
        && let Some(tbl) = string_args.first().and_then(|s| clean_sql_table_name(s))
    {
        let chain_lower = full_chain_text.to_ascii_lowercase();
        let kind = if chain_lower.contains("->insert")
            || chain_lower.contains("->update")
            || chain_lower.contains("->delete")
            || chain_lower.contains("->upsert")
        {
            "writes_table"
        } else {
            "reads_table"
        };
        ctx.push_semantic(kind, format!("table:{tbl}"), source_symbol, line, evidence);
    }
}

fn extract_event_name_from_args(
    receiver: Option<&str>,
    callee_lower: &str,
    args_text: &str,
    string_args: &[String],
) -> Option<String> {
    if callee_lower == "dispatch"
        && let Some(recv) = receiver
    {
        let short = recv.rsplit(['\\', ':', '.']).next().unwrap_or(recv).trim();
        if !short.is_empty()
            && !matches!(
                short.to_ascii_lowercase().as_str(),
                "event" | "bus" | "dispatcher" | "this"
            )
            && short.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        {
            return Some(short.to_owned());
        }
    }
    if let Some(s) = string_args.first()
        && !s.trim().is_empty()
    {
        return Some(s.trim().to_owned());
    }
    for part in args_text.split(',') {
        let trimmed = part.trim();
        if let Some(cls) = extract_class_ref_name(trimmed) {
            return Some(cls);
        }
        if let Some(after_new) = trimmed.strip_prefix("new ") {
            let cls = after_new
                .split(['(', '{', ' '])
                .next()
                .unwrap_or("")
                .rsplit(['\\', ':', '.'])
                .next()
                .unwrap_or("")
                .trim();
            if !cls.is_empty() {
                return Some(cls.to_owned());
            }
        }
    }
    None
}

fn emit_semantic_from_attribute(
    ctx: &mut ExtractCtx<'_>,
    attr_name: &str,
    attr_text: &str,
    string_args: &[String],
    annotated_symbol: Option<&str>,
    param_types: &[String],
    line: u32,
) {
    let short = attr_name
        .rsplit(['\\', ':', '.'])
        .next()
        .unwrap_or(attr_name)
        .trim_start_matches(['@', '#']);
    let lower = short.to_ascii_lowercase();

    // Routes: `Route`, `GetMapping`, `PostMapping`, `PutMapping`, `DeleteMapping`, `PatchMapping`,
    // `RequestMapping`, `HttpGet`, `HttpPost`, `HttpPut`, `HttpDelete`, `HttpPatch`, `get`, `post`, etc.
    let route_verb = match lower.as_str() {
        "getmapping" | "httpget" | "get" => Some("GET"),
        "postmapping" | "httppost" | "post" => Some("POST"),
        "putmapping" | "httpput" | "put" => Some("PUT"),
        "deletemapping" | "httpdelete" | "delete" => Some("DELETE"),
        "patchmapping" | "httppatch" | "patch" => Some("PATCH"),
        "route" | "requestmapping" | "path" => Some("ANY"),
        _ => None,
    };
    if let Some(verb) = route_verb {
        if let Some(path) = string_args.first() {
            let p = path.trim();
            if !p.is_empty() {
                let target = if verb == "ANY" {
                    format!("route:{p}")
                } else {
                    format!("route:{verb} {p}")
                };
                ctx.push_semantic(
                    "exposes_route",
                    target,
                    annotated_symbol.map(str::to_owned),
                    line,
                    attr_text,
                );
            }
        } else if verb != "ANY" {
            ctx.push_semantic(
                "exposes_route",
                format!("route:{verb} /"),
                annotated_symbol.map(str::to_owned),
                line,
                attr_text,
            );
        }
    }

    // DI bindings: `Injectable`, `Service`, `Repository`, `Component`, `Bean`, `AsAlias`
    if matches!(
        lower.as_str(),
        "injectable" | "service" | "repository" | "component" | "bean" | "asalias" | "inject"
    ) {
        let svc = extract_class_ref_name(attr_text)
            .or_else(|| string_args.first().cloned())
            .or_else(|| annotated_symbol.map(str::to_owned));
        if let Some(s) = svc {
            ctx.push_semantic(
                "di_binds",
                format!("service:{s}"),
                annotated_symbol.map(str::to_owned),
                line,
                attr_text,
            );
        }
    }

    // Event handlers: `AsMessageHandler`, `AsEventListener`, `EventListener`, `EventHandler`, `KafkaListener`, `RabbitListener`
    if matches!(
        lower.as_str(),
        "asmessagehandler"
            | "aseventlistener"
            | "eventlistener"
            | "eventhandler"
            | "kafkalistener"
            | "rabbitlistener"
    ) {
        let ev = extract_class_ref_name(attr_text)
            .or_else(|| string_args.first().cloned())
            .or_else(|| param_types.first().cloned());
        if let Some(e) = ev {
            ctx.push_semantic(
                "handles_event",
                format!("event:{e}"),
                annotated_symbol.map(str::to_owned),
                line,
                attr_text,
            );
        }
    }

    // ORM table attributes: `Table` (`#[ORM\Table(name: 'tbl')]`, `@Table(name = "tbl")`)
    if lower == "table"
        && let Some(tbl) = string_args.first().and_then(|s| clean_sql_table_name(s))
    {
        ctx.push_semantic(
            "reads_table",
            format!("table:{tbl}"),
            annotated_symbol.map(str::to_owned),
            line,
            attr_text,
        );
    }

    // Test attributes across languages (`@Test`, `#[Test]`, `[Fact]`, `[Theory]`, `[TestMethod]`)
    if matches!(
        lower.as_str(),
        "test" | "fact" | "theory" | "testmethod" | "parameterizedtest"
    ) {
        ctx.test_lines.insert(line);
    }
}

fn split_top_level_commas(raw: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth_paren = 0usize;
    let mut depth_angle = 0usize;
    let mut depth_bracket = 0usize;
    let mut depth_brace = 0usize;
    let mut in_quote: Option<char> = None;
    let mut escape = false;
    let mut start = 0usize;

    for (idx, ch) in raw.char_indices() {
        if let Some(q) = in_quote {
            if escape {
                escape = false;
                continue;
            }
            if ch == '\\' {
                escape = true;
            } else if ch == q {
                in_quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' | '`' => in_quote = Some(ch),
            '(' => depth_paren += 1,
            ')' => depth_paren = depth_paren.saturating_sub(1),
            '<' => depth_angle += 1,
            '>' => {
                // Skip `=>` closure arrows so they do not decrement angle bracket depth.
                if !raw[..idx].ends_with('=') {
                    depth_angle = depth_angle.saturating_sub(1);
                }
            }
            '[' => depth_bracket += 1,
            ']' => depth_bracket = depth_bracket.saturating_sub(1),
            '{' => depth_brace += 1,
            '}' => depth_brace = depth_brace.saturating_sub(1),
            ',' if depth_paren == 0
                && depth_angle == 0
                && depth_bracket == 0
                && depth_brace == 0 =>
            {
                parts.push(raw[start..idx].trim());
                start = idx + 1;
            }
            _ => {}
        }
    }
    if start <= raw.len() {
        let tail = raw[start..].trim();
        if !tail.is_empty() {
            parts.push(tail);
        }
    }
    parts
}

fn clean_di_type_token(raw: &str) -> Option<String> {
    let cleaned = raw
        .trim()
        .trim_start_matches("typeof")
        .trim()
        .trim_matches(|c: char| {
            matches!(
                c,
                '(' | ')' | '[' | ']' | '{' | '}' | ' ' | '\t' | '\n' | '\r'
            )
        })
        .trim();
    if cleaned.is_empty() || cleaned.starts_with(['\'', '"', '`', '$', '|']) {
        return None;
    }
    if cleaned.starts_with("fn")
        || cleaned.starts_with("function")
        || cleaned.contains("=>")
        || cleaned.starts_with('{')
    {
        return None;
    }
    let candidate = cleaned.strip_prefix("dyn ").unwrap_or(cleaned).trim();
    let short = candidate
        .rsplit(['\\', ':', '.'])
        .next()
        .unwrap_or(candidate)
        .split('<')
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
    if short.is_empty()
        || !short.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        || matches!(short, "Closure" | "Object" | "Array" | "String")
    {
        return None;
    }
    Some(short.to_owned())
}

fn extract_new_type_in_closure(raw: &str) -> Option<String> {
    let idx = raw.find("new ")?;
    let after = raw[idx + 4..].trim_start();
    let tok = after
        .split(|c: char| {
            !c.is_ascii_alphanumeric() && c != '_' && c != '\\' && c != ':' && c != '.'
        })
        .next()
        .unwrap_or("");
    clean_di_type_token(tok)
}

fn extract_di_targets(
    args_inner: &str,
    args_text: &str,
    string_args: &[String],
    full_chain_text: &str,
) -> Vec<String> {
    let mut targets = Vec::new();
    let mut push_unique = |val: String| {
        if !val.is_empty() && !targets.contains(&val) {
            targets.push(val);
        }
    };

    for part in split_top_level_commas(args_inner) {
        let p = part.trim();
        if let Some(ty) = extract_class_ref_name(p) {
            push_unique(ty);
            continue;
        }
        if let Some(inner) = p.strip_prefix("typeof(").and_then(|s| s.strip_suffix(')'))
            && let Some(ty) = clean_di_type_token(inner)
        {
            push_unique(ty);
            continue;
        }
        if (p.contains("provide:") || p.contains("useClass:") || p.contains("useExisting:"))
            && (p.starts_with('{') || p.contains("useClass:"))
        {
            let trimmed_obj = p.trim_matches(|c: char| matches!(c, '{' | '}' | ' '));
            for kv in split_top_level_commas(trimmed_obj) {
                if let Some((k, v)) = kv.split_once(':') {
                    let key = k.trim().trim_matches(|c: char| matches!(c, '{' | ' '));
                    if matches!(key, "provide" | "useClass" | "useExisting")
                        && let Some(ty) = clean_di_type_token(v)
                    {
                        push_unique(ty);
                    }
                }
            }
            continue;
        }
        if let Some(ty) = extract_new_type_in_closure(p) {
            push_unique(ty);
            continue;
        }
        if !p.contains(' ')
            && let Some(ty) = clean_di_type_token(p)
        {
            push_unique(ty);
        }
    }

    // Generic type arguments immediately preceding `args_text` (e.g. `AddSingleton<ITokenCache, RedisTokenCache>()`).
    let chain_prefix = full_chain_text
        .trim_end()
        .strip_suffix(args_text.trim())
        .unwrap_or(full_chain_text)
        .trim_end();
    if let Some(without_gt) = chain_prefix.strip_suffix('>')
        && let Some(lt_idx) = without_gt.rfind('<')
    {
        let generic_inner = &without_gt[lt_idx + 1..];
        for arg in split_top_level_commas(generic_inner) {
            if let Some(ty) = clean_di_type_token(arg) {
                push_unique(ty);
            }
        }
    }

    if targets.is_empty()
        && let Some(s) = string_args.first()
        && !s.trim().is_empty()
    {
        targets.push(s.trim().to_owned());
    }
    targets
}

fn normalize_http_call_path(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains(char::is_whitespace) {
        return None;
    }
    let path_part = if let Some(rest) = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
    {
        match rest.find('/') {
            Some(slash) => &rest[slash..],
            None => "/",
        }
    } else if trimmed.starts_with('/') {
        trimmed
    } else {
        return None;
    };
    let without_query = path_part.split(['?', '#']).next().unwrap_or(path_part);
    if without_query.is_empty() {
        Some("/".to_owned())
    } else {
        Some(without_query.to_owned())
    }
}

fn extract_http_verb_token(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_uppercase().as_str() {
        "GET" => Some("GET"),
        "POST" => Some("POST"),
        "PUT" => Some("PUT"),
        "PATCH" => Some("PATCH"),
        "DELETE" => Some("DELETE"),
        "HEAD" => Some("HEAD"),
        "OPTIONS" => Some("OPTIONS"),
        _ => None,
    }
}

fn extract_explicit_http_verb(args_inner: &str, string_args: &[String]) -> Option<&'static str> {
    for arg in string_args {
        if let Some(v) = extract_http_verb_token(arg) {
            return Some(v);
        }
    }
    let lower = args_inner.to_ascii_lowercase();
    for (needle, verb) in [
        ("method:'post'", "POST"),
        ("method:\"post\"", "POST"),
        ("method:'put'", "PUT"),
        ("method:\"put\"", "PUT"),
        ("method:'patch'", "PATCH"),
        ("method:\"patch\"", "PATCH"),
        ("method:'delete'", "DELETE"),
        ("method:\"delete\"", "DELETE"),
        ("method:'get'", "GET"),
        ("method:\"get\"", "GET"),
    ] {
        let compact: String = lower.chars().filter(|c| !c.is_whitespace()).collect();
        if compact.contains(needle) {
            return Some(verb);
        }
    }
    None
}

fn is_http_client_receiver(recv_lower: &str) -> bool {
    matches!(
        recv_lower,
        "axios"
            | "http"
            | "httpclient"
            | "requests"
            | "httpx"
            | "got"
            | "ky"
            | "superagent"
            | "guzzle"
            | "resttemplate"
            | "webclient"
            | "apiclient"
            | "restclient"
            | "client"
            | "api"
    ) || [
        ".axios",
        ".http",
        "\\http",
        "::http",
        "->http",
        ".httpclient",
        "->httpclient",
        "httpclient",
        ".apiclient",
        "apiclient",
        ".restclient",
        "restclient",
        ".client",
        "->client",
        ".api",
        "->api",
    ]
    .iter()
    .any(|suffix| recv_lower.ends_with(suffix))
}

#[allow(clippy::too_many_arguments)]
fn emit_http_client_route_call(
    ctx: &mut ExtractCtx<'_>,
    recv_lower: &str,
    callee_lower: &str,
    args_inner: &str,
    string_args: &[String],
    source_symbol: Option<&str>,
    line: u32,
    evidence: &str,
) {
    let mut verb: Option<&'static str> = None;
    let is_fetch_or_axios_fn = (callee_lower == "fetch"
        && (recv_lower.is_empty()
            || matches!(
                recv_lower,
                "window" | "globalthis" | "self" | "this" | "$this"
            )))
        || (callee_lower == "axios" && recv_lower.is_empty());

    if is_fetch_or_axios_fn {
        verb = Some(extract_explicit_http_verb(args_inner, string_args).unwrap_or("GET"));
    } else if is_http_client_receiver(recv_lower) {
        verb = match callee_lower {
            "get" | "getasync" | "getjson" | "getforobject" | "getforentity" => Some("GET"),
            "post" | "postasync" | "postjson" | "postforobject" | "postforentity" => Some("POST"),
            "put" | "putasync" => Some("PUT"),
            "patch" | "patchasync" => Some("PATCH"),
            "delete" | "deleteasync" => Some("DELETE"),
            "head" => Some("HEAD"),
            "options" => Some("OPTIONS"),
            "request" | "send" | "newrequest" => {
                Some(extract_explicit_http_verb(args_inner, string_args).unwrap_or("ANY"))
            }
            _ => None,
        };
    }

    let Some(http_method) = verb else {
        return;
    };
    let Some(route_path) = string_args
        .iter()
        .find_map(|arg| normalize_http_call_path(arg))
    else {
        return;
    };
    let target = if http_method == "ANY" {
        format!("route:{route_path}")
    } else {
        format!("route:{http_method} {route_path}")
    };
    ctx.push_semantic(
        "calls_route",
        target,
        source_symbol.map(str::to_owned),
        line,
        evidence,
    );
}

// =========================================================================
// 1. PHP Extraction
// =========================================================================

fn extract_php(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    let mut class_stack: Vec<(String, String)> = Vec::new(); // (kind, name)
    let mut fn_depth = 0usize;
    let mut this_props: BTreeMap<String, String> = BTreeMap::new();
    let mut var_types: BTreeMap<String, String> = BTreeMap::new();

    walk_php(
        ctx,
        root,
        0,
        &mut class_stack,
        &mut fn_depth,
        &mut this_props,
        &mut var_types,
    );
}

fn walk_php(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    class_stack: &mut Vec<(String, String)>,
    fn_depth: &mut usize,
    this_props: &mut BTreeMap<String, String>,
    var_types: &mut BTreeMap<String, String>,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let kind = node.kind();
    let cur_class = class_stack.last().map(|(_, n)| n.clone());

    match kind {
        "namespace_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                ctx.namespace = Some(ctx.text(name_node).trim().trim_matches('\\').to_owned());
            }
        }
        "namespace_use_declaration" => {
            extract_php_namespace_use(ctx, node);
            return;
        }
        "use_declaration" => {
            // Trait use inside class/trait/enum.
            if let Some(owner) = cur_class.as_deref() {
                let line = start_line(node);
                let evidence = ctx.text(node);
                for i in 0..node.child_count() {
                    if let Some(ch) = node.child(i)
                        && matches!(ch.kind(), "name" | "qualified_name")
                    {
                        let trait_path = ctx.text(ch).trim_start_matches('\\').to_owned();
                        let short = trait_path
                            .rsplit('\\')
                            .next()
                            .unwrap_or(&trait_path)
                            .to_owned();
                        ctx.push_import(AstImport {
                            path: trait_path.clone(),
                            symbols: vec![short.clone()],
                            alias: None,
                            is_wildcard: false,
                            is_reexport: false,
                            reexport_visibility: None,
                            kind: "trait_use".to_owned(),
                            line,
                            evidence: clip_str(evidence, 160),
                        });
                        ctx.push_inheritance(owner, &trait_path, "uses", line, evidence);
                    }
                }
            }
            return;
        }
        "include_expression"
        | "include_once_expression"
        | "require_expression"
        | "require_once_expression" => {
            let line = start_line(node);
            let evidence = ctx.text(node);
            let strs = all_string_arg_texts(node, ctx.source);
            if let Some(path_str) = strs.last() {
                let stem = path_str
                    .rsplit('/')
                    .next()
                    .unwrap_or(path_str)
                    .split('.')
                    .next()
                    .unwrap_or("")
                    .to_owned();
                ctx.push_import(AstImport {
                    path: path_str.clone(),
                    symbols: if stem.is_empty() {
                        Vec::new()
                    } else {
                        vec![stem]
                    },
                    alias: None,
                    is_wildcard: false,
                    is_reexport: false,
                    reexport_visibility: None,
                    kind: "include".to_owned(),
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
        }
        "class_declaration"
        | "interface_declaration"
        | "trait_declaration"
        | "enum_declaration" => {
            let decl_kind = match kind {
                "interface_declaration" => "interface",
                "trait_declaration" => "trait",
                "enum_declaration" => "enum",
                _ => "class",
            };
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let e_line = end_line(node);
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let modifiers = collect_php_modifiers(node, ctx.source);

            let mut param_types = Vec::new();
            if let Some(body) = body_node {
                for i in 0..body.child_count() {
                    if let Some(m) = body.child(i)
                        && m.kind() == "method_declaration"
                        && m.child_by_field_name("name")
                            .is_some_and(|n| ctx.text(n) == "__invoke")
                        && let Some(params) = m.child_by_field_name("parameters")
                    {
                        param_types.extend(collect_php_param_type_names(params, ctx.source));
                    }
                }
            }
            emit_php_attributes(ctx, node, Some(&name), &param_types);

            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    cur_class.clone(),
                    line,
                    e_line,
                    None,
                    modifiers,
                    sig.clone(),
                    sig,
                    String::new(),
                );
                // Inheritance: `base_clause` and `class_interface_clause`
                if let Some(base) = child_by_kind(node, "base_clause") {
                    for p in collect_php_type_list(base, ctx.source) {
                        let inherit_kind = if decl_kind == "interface" {
                            "implements"
                        } else {
                            "extends"
                        };
                        ctx.push_inheritance(&name, &p, inherit_kind, line, ctx.text(base));
                    }
                }
                if let Some(ifaces) = child_by_kind(node, "class_interface_clause") {
                    for p in collect_php_type_list(ifaces, ctx.source) {
                        ctx.push_inheritance(&name, &p, "implements", line, ctx.text(ifaces));
                    }
                }
                class_stack.push((decl_kind.to_owned(), name));
                if let Some(body) = body_node {
                    walk_php(
                        ctx,
                        body,
                        depth + 1,
                        class_stack,
                        fn_depth,
                        this_props,
                        var_types,
                    );
                }
                class_stack.pop();
                if class_stack.is_empty() {
                    this_props.clear();
                }
                return;
            }
        }
        "property_declaration" => {
            if let Some(owner) = cur_class.as_deref() {
                let (vis, mods, _has_vis) = parse_php_member_qualifiers(node, ctx.source);
                let vis_final = vis.or_else(|| Some("public".to_owned()));
                let type_node = node.child_by_field_name("type");
                let type_str = type_node.map(|n| ctx.text(n).trim().to_owned());
                if let Some(ty) = type_node {
                    emit_php_type_refs(ctx, ty, cur_class.as_deref(), "field");
                }
                for prop_el in children_by_kind(node, "property_element") {
                    if let Some(var_node) = child_by_kind(prop_el, "variable_name") {
                        let prop_name = ctx.text(var_node).trim().to_owned();
                        let line = start_line(prop_el);
                        let e_line = end_line(prop_el);
                        let init_node = prop_el.child_by_field_name("default_value");
                        let sig_end = init_node.map_or(prop_el.end_byte(), |i| i.start_byte());
                        let sig = header_without_comments(node, sig_end, ctx.source)
                            .trim_end_matches(['=', ';'])
                            .trim()
                            .to_owned();
                        let params_sig = strip_php_vis_from_sig(&sig);
                        let body_fp = init_node
                            .map(|i| ast_body_fingerprint(i, ctx.source))
                            .unwrap_or_default();

                        if let Some(ref ty) = type_str {
                            this_props.insert(
                                prop_name.trim_start_matches('$').to_owned(),
                                ty.trim_start_matches('?').to_owned(),
                            );
                        }
                        if prop_name == "$table"
                            && let Some(init) = init_node
                            && let Some(tbl) = first_string_arg_text(init, ctx.source)
                                .and_then(|s| clean_sql_table_name(&s))
                        {
                            ctx.push_semantic(
                                "reads_table",
                                format!("table:{tbl}"),
                                Some(owner.to_owned()),
                                line,
                                ctx.text(prop_el),
                            );
                        }
                        ctx.push_decl(
                            "property",
                            &prop_name,
                            Some(owner.to_owned()),
                            line,
                            e_line,
                            vis_final.clone(),
                            mods.clone(),
                            sig,
                            params_sig,
                            body_fp,
                        );
                    }
                }
            }
            return;
        }
        "const_declaration" => {
            if *fn_depth == 0 {
                let (vis, mods, _) = parse_php_member_qualifiers(node, ctx.source);
                let vis_final = vis.or_else(|| Some("public".to_owned()));
                for el in children_by_kind(node, "const_element") {
                    if let Some(name_node) = el.child(0) {
                        let name = ctx.text(name_node).trim().to_owned();
                        let line = start_line(el);
                        let e_line = end_line(el);
                        let val_node = el.child(2);
                        let sig_end = val_node.map_or(el.end_byte(), |v| v.start_byte());
                        let sig = header_without_comments(node, sig_end, ctx.source)
                            .trim_end_matches(['=', ';'])
                            .trim()
                            .to_owned();
                        let params_sig = strip_php_vis_from_sig(&sig);
                        let body_fp = val_node
                            .map(|v| ast_body_fingerprint(v, ctx.source))
                            .unwrap_or_default();
                        ctx.push_decl(
                            "const",
                            &name,
                            cur_class.clone(),
                            line,
                            e_line,
                            vis_final.clone(),
                            mods.clone(),
                            sig,
                            params_sig,
                            body_fp,
                        );
                    }
                }
            }
            return;
        }
        "method_declaration" | "function_definition" => {
            let is_method = kind == "method_declaration";
            if !is_method && *fn_depth > 0 {
                return;
            }
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let e_line = end_line(node);
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source)
                .trim_end_matches(';')
                .trim()
                .to_owned();
            let params_sig = strip_php_vis_from_sig(&sig);
            let body_fp = body_node
                .map(|b| ast_body_fingerprint(b, ctx.source))
                .unwrap_or_default();
            let (vis, mods, _) = parse_php_member_qualifiers(node, ctx.source);
            let vis_final = vis.or_else(|| Some("public".to_owned()));

            let params_node = node.child_by_field_name("parameters");
            let param_types = params_node
                .map(|p| collect_php_param_type_names(p, ctx.source))
                .unwrap_or_default();
            let sym_label = if let Some(ref owner) = cur_class {
                format!("{owner}::{name}")
            } else {
                name.clone()
            };
            emit_php_attributes(ctx, node, Some(&sym_label), &param_types);

            if !name.is_empty() {
                ctx.push_decl(
                    if is_method { "method" } else { "function" },
                    &name,
                    if is_method {
                        cur_class.clone()
                    } else {
                        Some(ctx.rel_path.clone())
                    },
                    line,
                    e_line,
                    vis_final,
                    mods,
                    sig,
                    params_sig,
                    body_fp,
                );
            }

            let mut local_vars = var_types.clone();
            if let Some(params) = params_node {
                extract_php_params(
                    ctx,
                    params,
                    cur_class.as_deref(),
                    &name,
                    this_props,
                    &mut local_vars,
                );
            }
            if let Some(ret) = node.child_by_field_name("return_type") {
                emit_php_type_refs(ctx, ret, Some(&sym_label), "return_type");
            }
            if let Some(body) = body_node {
                *fn_depth += 1;
                walk_php(
                    ctx,
                    body,
                    depth + 1,
                    class_stack,
                    fn_depth,
                    this_props,
                    &mut local_vars,
                );
                *fn_depth -= 1;
            }
            return;
        }
        "assignment_expression" => {
            if let (Some(lhs), Some(rhs)) = (
                node.child_by_field_name("left"),
                node.child_by_field_name("right"),
            ) {
                if let Some(ty) = infer_php_expr_type(ctx, rhs, this_props, var_types) {
                    let lhs_txt = ctx.text(lhs).trim();
                    if let Some(prop) = lhs_txt.strip_prefix("$this->") {
                        this_props.insert(prop.to_owned(), ty);
                    } else if let Some(var) = lhs_txt.strip_prefix('$')
                        && var.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        var_types.insert(var.to_owned(), ty);
                    }
                }
                if lhs.kind() == "member_access_expression"
                    || lhs.kind() == "nullsafe_member_access_expression"
                    || lhs.kind() == "scoped_property_access_expression"
                {
                    emit_php_member_access(ctx, lhs, true, this_props, var_types);
                    walk_php(
                        ctx,
                        rhs,
                        depth + 1,
                        class_stack,
                        fn_depth,
                        this_props,
                        var_types,
                    );
                    return;
                }
            }
        }
        "augmented_assignment_expression" => {
            if let Some(lhs) = node.child_by_field_name("left")
                && matches!(
                    lhs.kind(),
                    "member_access_expression"
                        | "nullsafe_member_access_expression"
                        | "scoped_property_access_expression"
                )
            {
                emit_php_member_access(ctx, lhs, true, this_props, var_types);
                if let Some(rhs) = node.child_by_field_name("right") {
                    walk_php(
                        ctx,
                        rhs,
                        depth + 1,
                        class_stack,
                        fn_depth,
                        this_props,
                        var_types,
                    );
                }
                return;
            }
        }
        "object_creation_expression" => {
            let line = start_line(node);
            let evidence = ctx.text(node);
            for i in 0..node.child_count() {
                if let Some(ch) = node.child(i)
                    && matches!(ch.kind(), "name" | "qualified_name")
                {
                    let cls = ctx.text(ch).trim().to_owned();
                    ctx.push_ref(AstRef {
                        target_name: cls,
                        member: None,
                        context: "constructor".to_owned(),
                        access: "call".to_owned(),
                        member_kind: None,
                        owner_type: None,
                        enclosing_symbol: cur_class.clone(),
                        is_static: false,
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                    break;
                }
            }
        }
        "scoped_call_expression" => {
            if let (Some(scope), Some(name_node)) = (
                node.child_by_field_name("scope"),
                node.child_by_field_name("name"),
            ) {
                let scope_txt = ctx.text(scope).trim().to_owned();
                let mem_txt = ctx.text(name_node).trim().to_owned();
                let line = start_line(node);
                let evidence = ctx.text(node);
                let args_node = node.child_by_field_name("arguments");
                let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
                let str_args = args_node
                    .map(|a| all_string_arg_texts(a, ctx.source))
                    .unwrap_or_default();
                let chain_txt = node.parent().map_or(evidence, |p| ctx.text(p));
                emit_semantic_from_call(
                    ctx,
                    Some(&scope_txt),
                    &mem_txt,
                    args_txt,
                    &str_args,
                    chain_txt,
                    cur_class.clone(),
                    line,
                );
                let short_scope = scope_txt
                    .rsplit('\\')
                    .next()
                    .unwrap_or(&scope_txt)
                    .to_owned();
                ctx.push_ref(AstRef {
                    target_name: scope_txt,
                    member: Some(mem_txt),
                    context: "call".to_owned(),
                    access: "call".to_owned(),
                    member_kind: Some("method".to_owned()),
                    owner_type: Some(short_scope),
                    enclosing_symbol: cur_class.clone(),
                    is_static: true,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
        }
        "member_call_expression" | "nullsafe_member_call_expression" => {
            if let (Some(obj), Some(name_node)) = (
                node.child_by_field_name("object"),
                node.child_by_field_name("name"),
            ) {
                let obj_txt = ctx.text(obj).trim();
                let mem_txt = ctx.text(name_node).trim().to_owned();
                let line = start_line(node);
                let evidence = ctx.text(node);
                let args_node = node.child_by_field_name("arguments");
                let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
                let str_args = args_node
                    .map(|a| all_string_arg_texts(a, ctx.source))
                    .unwrap_or_default();
                emit_semantic_from_call(
                    ctx,
                    Some(obj_txt),
                    &mem_txt,
                    args_txt,
                    &str_args,
                    evidence,
                    cur_class.clone(),
                    line,
                );
                if let Some(owner_ty) = resolve_php_receiver_type(obj_txt, this_props, var_types) {
                    let short = owner_ty.rsplit('\\').next().unwrap_or(&owner_ty).to_owned();
                    ctx.push_ref(AstRef {
                        target_name: owner_ty,
                        member: Some(mem_txt),
                        context: "call".to_owned(),
                        access: "call".to_owned(),
                        member_kind: Some("method".to_owned()),
                        owner_type: Some(short),
                        enclosing_symbol: cur_class.clone(),
                        is_static: false,
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                }
            }
        }
        "function_call_expression" => {
            if let Some(func_node) = node.child_by_field_name("function") {
                let fn_name = ctx.text(func_node).trim().to_owned();
                let line = start_line(node);
                let evidence = ctx.text(node);
                let args_node = node.child_by_field_name("arguments");
                let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
                let str_args = args_node
                    .map(|a| all_string_arg_texts(a, ctx.source))
                    .unwrap_or_default();
                emit_semantic_from_call(
                    ctx,
                    None,
                    &fn_name,
                    args_txt,
                    &str_args,
                    evidence,
                    cur_class.clone(),
                    line,
                );
                ctx.push_ref(AstRef {
                    target_name: fn_name,
                    member: None,
                    context: "call".to_owned(),
                    access: "call".to_owned(),
                    member_kind: Some("function".to_owned()),
                    owner_type: None,
                    enclosing_symbol: cur_class.clone(),
                    is_static: false,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
        }
        "member_access_expression"
        | "nullsafe_member_access_expression"
        | "scoped_property_access_expression"
        | "class_constant_access_expression" => {
            emit_php_member_access(ctx, node, false, this_props, var_types);
        }
        "binary_expression" => {
            let txt = ctx.text(node);
            if txt.contains("instanceof")
                && let Some(right) = node.child_by_field_name("right")
                && matches!(right.kind(), "name" | "qualified_name")
            {
                let cls = ctx.text(right).trim().to_owned();
                ctx.push_ref(AstRef {
                    target_name: cls,
                    member: None,
                    context: "parameter_type".to_owned(),
                    access: "type".to_owned(),
                    member_kind: None,
                    owner_type: None,
                    enclosing_symbol: cur_class.clone(),
                    is_static: false,
                    line: start_line(node),
                    evidence: clip_str(txt, 160),
                });
            }
        }
        "catch_clause" => {
            if let Some(ty) = node.child_by_field_name("type") {
                emit_php_type_refs(ctx, ty, cur_class.as_deref(), "parameter_type");
            }
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_php(
                ctx,
                ch,
                depth + 1,
                class_stack,
                fn_depth,
                this_props,
                var_types,
            );
        }
    }
}

fn extract_php_namespace_use(ctx: &mut ExtractCtx<'_>, node: Node<'_>) {
    let line = start_line(node);
    let evidence = ctx.text(node);
    let mut prefix: Option<String> = None;

    for i in 0..node.child_count() {
        let Some(ch) = node.child(i) else {
            continue;
        };
        match ch.kind() {
            "namespace_name" => {
                prefix = Some(ctx.text(ch).trim().trim_matches('\\').to_owned());
            }
            "namespace_use_clause" => {
                emit_php_use_clause(ctx, ch, prefix.as_deref(), line, evidence);
            }
            "namespace_use_group" => {
                for clause in children_by_kind(ch, "namespace_use_clause") {
                    emit_php_use_clause(ctx, clause, prefix.as_deref(), line, evidence);
                }
            }
            _ => {}
        }
    }
}

fn emit_php_use_clause(
    ctx: &mut ExtractCtx<'_>,
    clause: Node<'_>,
    prefix: Option<&str>,
    line: u32,
    evidence: &str,
) {
    let mut raw_path = String::new();
    for i in 0..clause.child_count() {
        if let Some(ch) = clause.child(i)
            && matches!(ch.kind(), "name" | "qualified_name" | "namespace_name")
        {
            raw_path = ctx.text(ch).trim().trim_matches('\\').to_owned();
            break;
        }
    }
    if raw_path.is_empty() {
        return;
    }
    let full = match prefix {
        Some(p) if !p.is_empty() => format!("{p}\\{raw_path}"),
        _ => raw_path,
    };
    let alias = clause
        .child_by_field_name("alias")
        .map(|a| ctx.text(a).trim().to_owned());
    let short = full.rsplit('\\').next().unwrap_or(&full).to_owned();
    ctx.push_import(AstImport {
        path: full,
        symbols: vec![short],
        alias,
        is_wildcard: false,
        is_reexport: false,
        reexport_visibility: None,
        kind: "import".to_owned(),
        line,
        evidence: clip_str(evidence, 160),
    });
}

fn collect_php_modifiers(node: Node<'_>, source: &str) -> Vec<String> {
    let mut mods = Vec::new();
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i)
            && matches!(
                ch.kind(),
                "abstract_modifier" | "final_modifier" | "readonly_modifier" | "static_modifier"
            )
        {
            push_unique_mod(&mut mods, node_text(ch, source).trim());
        }
    }
    mods
}

fn parse_php_member_qualifiers(
    node: Node<'_>,
    source: &str,
) -> (Option<String>, Vec<String>, bool) {
    let mut vis_tokens: Vec<String> = Vec::new();
    let mut mods: Vec<String> = Vec::new();
    let mut has_vis = false;
    for i in 0..node.child_count() {
        let Some(ch) = node.child(i) else {
            continue;
        };
        match ch.kind() {
            "visibility_modifier" => {
                has_vis = true;
                let txt = node_text(ch, source)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                vis_tokens.push(txt);
            }
            "abstract_modifier" | "final_modifier" | "readonly_modifier" | "static_modifier" => {
                push_unique_mod(&mut mods, node_text(ch, source).trim());
            }
            _ => {}
        }
    }
    let vis = if vis_tokens.is_empty() {
        None
    } else if let Some(set_vis) = vis_tokens.iter().find(|v| v.contains("(set)")) {
        Some(set_vis.clone())
    } else {
        Some(vis_tokens[0].clone())
    };
    (vis, mods, has_vis)
}

fn strip_php_vis_from_sig(sig: &str) -> String {
    let mut cur = sig.trim();
    let mut kept = Vec::new();
    loop {
        cur = cur.trim_start();
        let next = cur.split_whitespace().next().unwrap_or("");
        if matches!(next, "public" | "protected" | "private" | "var")
            || next.starts_with("public(")
            || next.starts_with("protected(")
            || next.starts_with("private(")
        {
            cur = cur[next.len()..].trim_start();
            if cur.starts_with("(set)") {
                cur = cur[5..].trim_start();
            }
            continue;
        }
        if matches!(next, "abstract" | "final" | "readonly" | "static") {
            kept.push(next);
            cur = cur[next.len()..].trim_start();
            continue;
        }
        break;
    }
    if kept.is_empty() {
        cur.to_owned()
    } else if cur.is_empty() {
        kept.join(" ")
    } else {
        format!("{} {cur}", kept.join(" "))
    }
}

fn collect_php_type_list(clause: Node<'_>, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0..clause.child_count() {
        if let Some(ch) = clause.child(i)
            && matches!(ch.kind(), "name" | "qualified_name")
        {
            out.push(node_text(ch, source).trim().to_owned());
        }
    }
    out
}

fn collect_php_param_type_names(params_node: Node<'_>, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0..params_node.child_count() {
        if let Some(ch) = params_node.child(i)
            && let Some(ty) = ch.child_by_field_name("type")
        {
            let txt = node_text(ty, source).trim().trim_start_matches('?');
            let short = txt.rsplit('\\').next().unwrap_or(txt);
            if !short.is_empty() {
                out.push(short.to_owned());
            }
        }
    }
    out
}

fn emit_php_attributes(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    annotated_symbol: Option<&str>,
    param_types: &[String],
) {
    let Some(attr_list) = node.child_by_field_name("attributes") else {
        return;
    };
    let mut stack = vec![attr_list];
    while let Some(cur) = stack.pop() {
        if cur.kind() == "attribute" {
            let attr_text = ctx.text(cur);
            let attr_name = cur.child(0).map(|n| ctx.text(n)).unwrap_or(attr_text);
            let strs = cur
                .child_by_field_name("parameters")
                .map(|p| all_string_arg_texts(p, ctx.source))
                .unwrap_or_default();
            emit_semantic_from_attribute(
                ctx,
                attr_name,
                attr_text,
                &strs,
                annotated_symbol,
                param_types,
                start_line(cur),
            );
            continue;
        }
        for i in (0..cur.child_count()).rev() {
            if let Some(ch) = cur.child(i) {
                stack.push(ch);
            }
        }
    }
}

fn emit_php_type_refs(
    ctx: &mut ExtractCtx<'_>,
    ty_node: Node<'_>,
    enclosing: Option<&str>,
    context: &str,
) {
    let line = start_line(ty_node);
    let evidence = ctx.text(ty_node);
    let mut stack = vec![ty_node];
    while let Some(cur) = stack.pop() {
        if matches!(cur.kind(), "name" | "qualified_name" | "named_type") && cur.child_count() == 0
        {
            let txt = ctx.text(cur).trim().trim_start_matches('?');
            if !txt.is_empty() {
                ctx.push_ref(AstRef {
                    target_name: txt.to_owned(),
                    member: None,
                    context: context.to_owned(),
                    access: "type".to_owned(),
                    member_kind: None,
                    owner_type: None,
                    enclosing_symbol: enclosing.map(str::to_owned),
                    is_static: false,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
            continue;
        }
        if cur.kind() == "qualified_name" {
            let txt = ctx.text(cur).trim().to_owned();
            if !txt.is_empty() {
                ctx.push_ref(AstRef {
                    target_name: txt,
                    member: None,
                    context: context.to_owned(),
                    access: "type".to_owned(),
                    member_kind: None,
                    owner_type: None,
                    enclosing_symbol: enclosing.map(str::to_owned),
                    is_static: false,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
            continue;
        }
        for i in (0..cur.child_count()).rev() {
            if let Some(ch) = cur.child(i) {
                stack.push(ch);
            }
        }
    }
}

fn extract_php_params(
    ctx: &mut ExtractCtx<'_>,
    params_node: Node<'_>,
    owner_class: Option<&str>,
    fn_name: &str,
    this_props: &mut BTreeMap<String, String>,
    var_types: &mut BTreeMap<String, String>,
) {
    let is_ctor = fn_name == "__construct";
    for i in 0..params_node.child_count() {
        let Some(p) = params_node.child(i) else {
            continue;
        };
        if !matches!(
            p.kind(),
            "simple_parameter" | "property_promotion_parameter"
        ) {
            continue;
        }
        let ty_node = p.child_by_field_name("type");
        let ty_str = ty_node.map(|t| {
            ctx.text(t)
                .split('|')
                .map(str::trim)
                .find(|part| !part.eq_ignore_ascii_case("null") && !part.is_empty())
                .unwrap_or("")
                .trim_start_matches('?')
                .to_owned()
        });
        let var_name = p
            .child_by_field_name("name")
            .map(|n| ctx.text(n).trim().to_owned())
            .unwrap_or_default();
        if let Some(ty) = ty_node {
            emit_php_type_refs(ctx, ty, owner_class, "parameter_type");
        }
        if let Some(ref ty) = ty_str
            && !ty.is_empty()
            && !var_name.is_empty()
        {
            var_types.insert(var_name.trim_start_matches('$').to_owned(), ty.clone());
        }
        if p.kind() == "property_promotion_parameter"
            && is_ctor
            && let Some(owner) = owner_class
        {
            let (vis, mods, has_vis) = parse_php_member_qualifiers(p, ctx.source);
            let has_readonly =
                p.child_by_field_name("readonly").is_some() || mods.iter().any(|m| m == "readonly");
            if has_vis || has_readonly {
                let mut all_mods = mods;
                if has_readonly {
                    push_unique_mod(&mut all_mods, "readonly");
                }
                let line = start_line(p);
                let e_line = end_line(p);
                let def_val = p.child_by_field_name("default_value");
                let sig_end = def_val.map_or(p.end_byte(), |d| d.start_byte());
                let sig = header_without_comments(p, sig_end, ctx.source)
                    .trim_end_matches('=')
                    .trim()
                    .to_owned();
                let params_sig = strip_php_vis_from_sig(&sig);
                let body_fp = def_val
                    .map(|d| ast_body_fingerprint(d, ctx.source))
                    .unwrap_or_default();
                if let Some(ref ty) = ty_str
                    && !ty.is_empty()
                {
                    this_props.insert(var_name.trim_start_matches('$').to_owned(), ty.clone());
                }
                ctx.push_decl(
                    "property",
                    &var_name,
                    Some(owner.to_owned()),
                    line,
                    e_line,
                    vis,
                    all_mods,
                    sig,
                    params_sig,
                    body_fp,
                );
            }
        }
    }
}

fn infer_php_expr_type(
    ctx: &ExtractCtx<'_>,
    rhs: Node<'_>,
    this_props: &BTreeMap<String, String>,
    var_types: &BTreeMap<String, String>,
) -> Option<String> {
    match rhs.kind() {
        "object_creation_expression" => {
            for i in 0..rhs.child_count() {
                if let Some(ch) = rhs.child(i)
                    && matches!(ch.kind(), "name" | "qualified_name")
                {
                    return Some(ctx.text(ch).trim().to_owned());
                }
            }
            None
        }
        "variable_name" => {
            let v = ctx.text(rhs).trim().trim_start_matches('$');
            var_types.get(v).cloned()
        }
        "member_access_expression" | "nullsafe_member_access_expression" => {
            let txt = ctx.text(rhs).trim();
            if let Some(prop) = txt.strip_prefix("$this->") {
                return this_props.get(prop).cloned();
            }
            None
        }
        _ => None,
    }
}

fn resolve_php_receiver_type(
    recv: &str,
    this_props: &BTreeMap<String, String>,
    var_types: &BTreeMap<String, String>,
) -> Option<String> {
    let trimmed = recv.trim();
    if let Some(prop) = trimmed
        .strip_prefix("$this->")
        .or_else(|| trimmed.strip_prefix("$this?->"))
    {
        return this_props.get(prop.trim()).cloned();
    }
    if let Some(var) = trimmed.strip_prefix('$') {
        return var_types.get(var).cloned();
    }
    None
}

fn emit_php_member_access(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    is_write: bool,
    this_props: &BTreeMap<String, String>,
    var_types: &BTreeMap<String, String>,
) {
    let line = start_line(node);
    let evidence = ctx.text(node);
    match node.kind() {
        "scoped_property_access_expression" => {
            if let (Some(scope), Some(name_node)) = (
                node.child_by_field_name("scope"),
                node.child_by_field_name("name"),
            ) {
                let scope_txt = ctx.text(scope).trim().to_owned();
                let mem_txt = ctx.text(name_node).trim().to_owned();
                let short_scope = scope_txt
                    .rsplit('\\')
                    .next()
                    .unwrap_or(&scope_txt)
                    .to_owned();
                ctx.push_ref(AstRef {
                    target_name: scope_txt,
                    member: Some(mem_txt),
                    context: "field".to_owned(),
                    access: if is_write { "write" } else { "static_read" }.to_owned(),
                    member_kind: Some("property".to_owned()),
                    owner_type: Some(short_scope),
                    enclosing_symbol: None,
                    is_static: true,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
        }
        "class_constant_access_expression" => {
            let scope_txt = node.child(0).map(|n| ctx.text(n).trim()).unwrap_or("");
            let const_txt = node.child(2).map(|n| ctx.text(n).trim()).unwrap_or("");
            if !scope_txt.is_empty() && !const_txt.is_empty() {
                if const_txt == "class" {
                    ctx.push_ref(AstRef {
                        target_name: scope_txt.to_owned(),
                        member: None,
                        context: "parameter_type".to_owned(),
                        access: "type".to_owned(),
                        member_kind: None,
                        owner_type: None,
                        enclosing_symbol: None,
                        is_static: true,
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                } else {
                    let short_scope = scope_txt
                        .rsplit('\\')
                        .next()
                        .unwrap_or(scope_txt)
                        .to_owned();
                    ctx.push_ref(AstRef {
                        target_name: scope_txt.to_owned(),
                        member: Some(const_txt.to_owned()),
                        context: "field".to_owned(),
                        access: "const".to_owned(),
                        member_kind: Some("const".to_owned()),
                        owner_type: Some(short_scope),
                        enclosing_symbol: None,
                        is_static: true,
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                }
            }
        }
        "member_access_expression" | "nullsafe_member_access_expression" => {
            if let (Some(obj), Some(name_node)) = (
                node.child_by_field_name("object"),
                node.child_by_field_name("name"),
            ) {
                let obj_txt = ctx.text(obj).trim();
                let mem_raw = ctx.text(name_node).trim();
                let mem_name = if mem_raw.starts_with('$') {
                    mem_raw.to_owned()
                } else {
                    format!("${mem_raw}")
                };
                if let Some(owner_ty) = resolve_php_receiver_type(obj_txt, this_props, var_types) {
                    let short = owner_ty.rsplit('\\').next().unwrap_or(&owner_ty).to_owned();
                    ctx.push_ref(AstRef {
                        target_name: owner_ty,
                        member: Some(mem_name),
                        context: "field".to_owned(),
                        access: if is_write { "write" } else { "read" }.to_owned(),
                        member_kind: Some("property".to_owned()),
                        owner_type: Some(short),
                        enclosing_symbol: None,
                        is_static: false,
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                }
            }
        }
        _ => {}
    }
}

// =========================================================================
// 2. Rust Extraction
// =========================================================================

fn extract_rust(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    let mut struct_fields: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut var_types: BTreeMap<String, String> = BTreeMap::new();
    walk_rust(
        ctx,
        root,
        0,
        None,
        false,
        0,
        &mut struct_fields,
        &mut var_types,
    );
}

#[allow(clippy::too_many_arguments)]
fn walk_rust(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    owner_name: Option<&str>,
    owner_is_trait_or_trait_impl: bool,
    fn_depth: usize,
    struct_fields: &mut BTreeMap<String, BTreeMap<String, String>>,
    var_types: &mut BTreeMap<String, String>,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let kind = node.kind();

    match kind {
        "use_declaration" => {
            extract_rust_use_decl(ctx, node);
            return;
        }
        "mod_item" => {
            let body = node.child_by_field_name("body");
            if body.is_none()
                && let Some(name_node) = node.child_by_field_name("name")
            {
                let mod_name = ctx.text(name_node).trim().to_owned();
                let attr_path = find_preceding_rust_path_attr(node, ctx.source);
                let line = start_line(node);
                let path = attr_path.unwrap_or_else(|| mod_name.clone());
                ctx.push_import(AstImport {
                    path,
                    symbols: vec![mod_name],
                    alias: None,
                    is_wildcard: false,
                    is_reexport: false,
                    reexport_visibility: None,
                    kind: "mod_decl".to_owned(),
                    line,
                    evidence: clip_str(ctx.text(node), 160),
                });
            } else if let Some(b) = body {
                if let Some(attr_line) = has_preceding_rust_test_attr(node, ctx.source) {
                    ctx.test_spans.push((attr_line, end_line(node)));
                }
                walk_rust(
                    ctx,
                    b,
                    depth + 1,
                    owner_name,
                    false,
                    fn_depth,
                    struct_fields,
                    var_types,
                );
            }
            return;
        }
        "struct_item" | "enum_item" | "trait_item" => {
            if fn_depth > 0 {
                return;
            }
            let decl_kind = match kind {
                "struct_item" => "struct",
                "enum_item" => "enum",
                _ => "trait",
            };
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let e_line = end_line(node);
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source)
                .trim_end_matches(';')
                .trim()
                .to_owned();
            let vis = rust_visibility(node, ctx.source).unwrap_or_else(|| "private".to_owned());
            let params_sig = strip_rust_vis_from_sig(&sig);

            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    Some(ctx.rel_path.clone()),
                    line,
                    e_line,
                    Some(vis),
                    Vec::new(),
                    sig,
                    params_sig,
                    String::new(),
                );
                if let Some(body) = body_node {
                    walk_rust(
                        ctx,
                        body,
                        depth + 1,
                        Some(&name),
                        decl_kind == "trait",
                        fn_depth,
                        struct_fields,
                        var_types,
                    );
                }
            }
            return;
        }
        "impl_item" => {
            if fn_depth > 0 {
                return;
            }
            let type_node = node.child_by_field_name("type");
            let trait_node = node.child_by_field_name("trait");
            let target_ty = type_node
                .map(|n| short_rust_type(ctx.text(n)))
                .unwrap_or_default();
            let line = start_line(node);
            let evidence = ctx.text(node);
            if let Some(tn) = trait_node {
                let trait_ty = short_rust_type(ctx.text(tn));
                ctx.push_inheritance(&target_ty, &trait_ty, "implements", line, evidence);
                ctx.push_ref(AstRef {
                    target_name: ctx.text(tn).trim().to_owned(),
                    member: None,
                    context: "parameter_type".to_owned(),
                    access: "type".to_owned(),
                    member_kind: None,
                    owner_type: None,
                    enclosing_symbol: None,
                    is_static: false,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
            if let Some(ty_n) = type_node {
                ctx.push_ref(AstRef {
                    target_name: unwrap_rust_type(ctx.text(ty_n)),
                    member: None,
                    context: "parameter_type".to_owned(),
                    access: "type".to_owned(),
                    member_kind: None,
                    owner_type: None,
                    enclosing_symbol: None,
                    is_static: false,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
            if let Some(body) = node.child_by_field_name("body") {
                walk_rust(
                    ctx,
                    body,
                    depth + 1,
                    (!target_ty.is_empty()).then_some(target_ty.as_str()),
                    trait_node.is_some(),
                    fn_depth,
                    struct_fields,
                    var_types,
                );
            }
            return;
        }
        "field_declaration" => {
            if let Some(owner) = owner_name {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| ctx.text(n).trim().to_owned())
                    .unwrap_or_default();
                let line = start_line(node);
                let e_line = end_line(node);
                let sig = header_without_comments(node, node.end_byte(), ctx.source)
                    .trim_end_matches(',')
                    .trim()
                    .to_owned();
                let vis = rust_visibility(node, ctx.source).unwrap_or_else(|| "private".to_owned());
                let params_sig = strip_rust_vis_from_sig(&sig);
                if let Some(ty) = node.child_by_field_name("type") {
                    let unwrapped = unwrap_rust_type(ctx.text(ty));
                    if !unwrapped.is_empty() && !name.is_empty() {
                        struct_fields
                            .entry(owner.to_owned())
                            .or_default()
                            .insert(name.clone(), unwrapped.clone());
                        ctx.push_ref(AstRef {
                            target_name: unwrapped,
                            member: None,
                            context: "field".to_owned(),
                            access: "type".to_owned(),
                            member_kind: None,
                            owner_type: None,
                            enclosing_symbol: Some(owner.to_owned()),
                            is_static: false,
                            line,
                            evidence: clip_str(&sig, 160),
                        });
                    }
                }
                if !name.is_empty() {
                    ctx.push_decl(
                        "field",
                        &name,
                        Some(owner.to_owned()),
                        line,
                        e_line,
                        Some(vis),
                        Vec::new(),
                        sig,
                        params_sig,
                        String::new(),
                    );
                }
            }
            return;
        }
        "const_item" | "static_item" => {
            if fn_depth > 0 {
                return;
            }
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let e_line = end_line(node);
            let val_node = node.child_by_field_name("value");
            let sig_end = val_node.map_or(node.end_byte(), |v| v.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source)
                .trim_end_matches(['=', ';'])
                .trim()
                .to_owned();
            let vis = rust_visibility(node, ctx.source).unwrap_or_else(|| {
                if owner_is_trait_or_trait_impl {
                    "public".to_owned()
                } else {
                    "private".to_owned()
                }
            });
            let params_sig = strip_rust_vis_from_sig(&sig);
            let body_fp = val_node
                .map(|v| ast_body_fingerprint(v, ctx.source))
                .unwrap_or_default();
            let mut mods = Vec::new();
            if kind == "static_item" {
                push_unique_mod(&mut mods, "static");
                if child_by_kind(node, "mutable_specifier").is_some() {
                    push_unique_mod(&mut mods, "mut");
                }
            }
            if !name.is_empty() {
                ctx.push_decl(
                    "const",
                    &name,
                    owner_name.map(str::to_owned),
                    line,
                    e_line,
                    Some(vis),
                    mods,
                    sig,
                    params_sig,
                    body_fp,
                );
            }
            if let Some(val) = val_node {
                walk_rust(
                    ctx,
                    val,
                    depth + 1,
                    owner_name,
                    owner_is_trait_or_trait_impl,
                    fn_depth,
                    struct_fields,
                    var_types,
                );
            }
            return;
        }
        "function_item" | "function_signature_item" => {
            if fn_depth > 0 {
                return;
            }
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let e_line = end_line(node);
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source)
                .trim_end_matches(';')
                .trim()
                .to_owned();
            let vis = rust_visibility(node, ctx.source).unwrap_or_else(|| {
                if owner_is_trait_or_trait_impl {
                    "public".to_owned()
                } else {
                    "private".to_owned()
                }
            });
            let params_sig = strip_rust_vis_from_sig(&sig);
            let body_fp = body_node
                .map(|b| ast_body_fingerprint(b, ctx.source))
                .unwrap_or_default();
            let mods = collect_rust_fn_modifiers(node, ctx.source);
            let decl_kind = if owner_name.is_some() {
                "method"
            } else {
                "function"
            };
            if !name.is_empty() {
                if let Some(attr_line) = has_preceding_rust_test_attr(node, ctx.source) {
                    ctx.test_spans.push((attr_line, e_line));
                }
                ctx.push_decl(
                    decl_kind,
                    &name,
                    owner_name.map(str::to_owned),
                    line,
                    e_line,
                    Some(vis),
                    mods,
                    sig,
                    params_sig,
                    body_fp,
                );
            }
            let mut local_vars = var_types.clone();
            if let Some(params) = node.child_by_field_name("parameters") {
                extract_rust_params(ctx, params, owner_name, &mut local_vars);
            }
            if let Some(ret) = node.child_by_field_name("return_type") {
                let ty = unwrap_rust_type(ctx.text(ret).trim_start_matches("->"));
                if !ty.is_empty() {
                    ctx.push_ref(AstRef {
                        target_name: ty,
                        member: None,
                        context: "return_type".to_owned(),
                        access: "type".to_owned(),
                        member_kind: None,
                        owner_type: None,
                        enclosing_symbol: owner_name.map(str::to_owned),
                        is_static: false,
                        line: start_line(ret),
                        evidence: clip_str(ctx.text(ret), 160),
                    });
                }
            }
            if let Some(body) = body_node {
                walk_rust(
                    ctx,
                    body,
                    depth + 1,
                    owner_name,
                    owner_is_trait_or_trait_impl,
                    fn_depth + 1,
                    struct_fields,
                    &mut local_vars,
                );
            }
            return;
        }
        "let_declaration" => {
            if let Some(pat) = node.child_by_field_name("pattern") {
                let var_name = ctx
                    .text(pat)
                    .trim()
                    .trim_start_matches("mut ")
                    .trim()
                    .to_owned();
                let mut inferred = node
                    .child_by_field_name("type")
                    .map(|t| unwrap_rust_type(ctx.text(t)))
                    .filter(|s| !s.is_empty());
                if inferred.is_none()
                    && let Some(val) = node.child_by_field_name("value")
                {
                    inferred = infer_rust_expr_type(
                        ctx,
                        val,
                        owner_name.and_then(|o| struct_fields.get(o)),
                    );
                }
                if let Some(ty) = inferred
                    && !var_name.is_empty()
                {
                    var_types.insert(var_name, ty);
                }
            }
        }
        "macro_invocation" => {
            if let Some(mac) = node.child_by_field_name("macro") {
                let mac_txt = ctx.text(mac).trim();
                let short = mac_txt.rsplit("::").next().unwrap_or(mac_txt);
                if short == "table" {
                    let full = ctx.text(node);
                    if let Some(brace_start) = full.find('{') {
                        let inner = full[brace_start + 1..].trim();
                        if let Some(tbl) = inner
                            .split(|c: char| c.is_whitespace() || c == '(')
                            .find(|s| !s.is_empty())
                            .and_then(clean_sql_table_name)
                        {
                            ctx.push_semantic(
                                "reads_table",
                                format!("table:{tbl}"),
                                owner_name.map(str::to_owned),
                                start_line(node),
                                full,
                            );
                        }
                    }
                }
            }
        }
        "attribute_item" | "inner_attribute_item" => {
            let txt = ctx.text(node);
            if let Some(attr) = child_by_kind(node, "attribute") {
                let name = attr.child(0).map(|n| ctx.text(n)).unwrap_or(txt);
                let strs = all_string_arg_texts(attr, ctx.source);
                emit_semantic_from_attribute(
                    ctx,
                    name,
                    txt,
                    &strs,
                    owner_name,
                    &[],
                    start_line(node),
                );
            }
        }
        "assignment_expression" | "compound_assignment_expr" => {
            if let Some(lhs) = node.child_by_field_name("left")
                && lhs.kind() == "field_expression"
            {
                emit_rust_field_expr(ctx, lhs, true, owner_name, struct_fields, var_types);
                if let Some(rhs) = node.child_by_field_name("right") {
                    walk_rust(
                        ctx,
                        rhs,
                        depth + 1,
                        owner_name,
                        owner_is_trait_or_trait_impl,
                        fn_depth,
                        struct_fields,
                        var_types,
                    );
                }
                return;
            }
        }
        "struct_expression" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let struct_ty = ctx.text(name_node).trim().to_owned();
                let short_ty = short_rust_type(&struct_ty);
                let line = start_line(node);
                let evidence = ctx.text(node);
                ctx.push_ref(AstRef {
                    target_name: struct_ty.clone(),
                    member: None,
                    context: "constructor".to_owned(),
                    access: "type".to_owned(),
                    member_kind: None,
                    owner_type: Some(short_ty.clone()),
                    enclosing_symbol: owner_name.map(str::to_owned),
                    is_static: false,
                    line,
                    evidence: clip_str(evidence, 160),
                });
                if let Some(body) = node.child_by_field_name("body") {
                    for init in children_by_kind(body, "field_initializer") {
                        if let Some(fname) = init.child_by_field_name("field") {
                            ctx.push_ref(AstRef {
                                target_name: struct_ty.clone(),
                                member: Some(ctx.text(fname).trim().to_owned()),
                                context: "field".to_owned(),
                                access: "write_init".to_owned(),
                                member_kind: Some("field".to_owned()),
                                owner_type: Some(short_ty.clone()),
                                enclosing_symbol: owner_name.map(str::to_owned),
                                is_static: false,
                                line: start_line(init),
                                evidence: clip_str(ctx.text(init), 160),
                            });
                        }
                    }
                }
            }
        }
        "call_expression" => {
            if let Some(func_node) = node.child_by_field_name("function") {
                let line = start_line(node);
                let evidence = ctx.text(node);
                let args_node = node.child_by_field_name("arguments");
                let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
                let str_args = args_node
                    .map(|a| all_string_arg_texts(a, ctx.source))
                    .unwrap_or_default();

                match func_node.kind() {
                    "field_expression" => {
                        if let (Some(val_node), Some(field_node)) = (
                            func_node.child_by_field_name("value"),
                            func_node.child_by_field_name("field"),
                        ) {
                            let recv_txt = ctx.text(val_node).trim();
                            let mem_txt = ctx.text(field_node).trim();
                            emit_semantic_from_call(
                                ctx,
                                Some(recv_txt),
                                mem_txt,
                                args_txt,
                                &str_args,
                                evidence,
                                owner_name.map(str::to_owned),
                                line,
                            );
                            if let Some(owner_ty) = resolve_rust_receiver_type(
                                recv_txt,
                                owner_name.and_then(|o| struct_fields.get(o)),
                                var_types,
                            ) {
                                let short = short_rust_type(&owner_ty);
                                ctx.push_ref(AstRef {
                                    target_name: owner_ty,
                                    member: Some(mem_txt.to_owned()),
                                    context: "call".to_owned(),
                                    access: "call".to_owned(),
                                    member_kind: Some("method".to_owned()),
                                    owner_type: Some(short),
                                    enclosing_symbol: owner_name.map(str::to_owned),
                                    is_static: false,
                                    line,
                                    evidence: clip_str(evidence, 160),
                                });
                            }
                        }
                    }
                    "scoped_identifier" | "generic_function" => {
                        let fn_txt = ctx.text(func_node).split("::<").next().unwrap_or("").trim();
                        if let Some((scope, mem)) = fn_txt.rsplit_once("::") {
                            emit_semantic_from_call(
                                ctx,
                                Some(scope),
                                mem,
                                args_txt,
                                &str_args,
                                evidence,
                                owner_name.map(str::to_owned),
                                line,
                            );
                            let short = short_rust_type(scope);
                            let is_type =
                                short.chars().next().is_some_and(|c| c.is_ascii_uppercase());
                            ctx.push_ref(AstRef {
                                target_name: fn_txt.to_owned(),
                                member: Some(mem.to_owned()),
                                context: "call".to_owned(),
                                access: "call".to_owned(),
                                member_kind: Some(
                                    if is_type { "method" } else { "function" }.to_owned(),
                                ),
                                owner_type: is_type.then_some(short),
                                enclosing_symbol: owner_name.map(str::to_owned),
                                is_static: true,
                                line,
                                evidence: clip_str(evidence, 160),
                            });
                        } else if !fn_txt.is_empty() {
                            emit_semantic_from_call(
                                ctx,
                                None,
                                fn_txt,
                                args_txt,
                                &str_args,
                                evidence,
                                owner_name.map(str::to_owned),
                                line,
                            );
                            ctx.push_ref(AstRef {
                                target_name: fn_txt.to_owned(),
                                member: None,
                                context: "call".to_owned(),
                                access: "call".to_owned(),
                                member_kind: Some("function".to_owned()),
                                owner_type: None,
                                enclosing_symbol: owner_name.map(str::to_owned),
                                is_static: false,
                                line,
                                evidence: clip_str(evidence, 160),
                            });
                        }
                    }
                    "identifier" => {
                        let fn_txt = ctx.text(func_node).trim();
                        emit_semantic_from_call(
                            ctx,
                            None,
                            fn_txt,
                            args_txt,
                            &str_args,
                            evidence,
                            owner_name.map(str::to_owned),
                            line,
                        );
                        ctx.push_ref(AstRef {
                            target_name: fn_txt.to_owned(),
                            member: None,
                            context: "call".to_owned(),
                            access: "call".to_owned(),
                            member_kind: Some("function".to_owned()),
                            owner_type: None,
                            enclosing_symbol: owner_name.map(str::to_owned),
                            is_static: false,
                            line,
                            evidence: clip_str(evidence, 160),
                        });
                    }
                    _ => {}
                }
                if let Some(args) = args_node {
                    walk_rust(
                        ctx,
                        args,
                        depth + 1,
                        owner_name,
                        owner_is_trait_or_trait_impl,
                        fn_depth,
                        struct_fields,
                        var_types,
                    );
                }
                // Walk receiver inside `field_expression` in case of chained calls like `Router::new().route(...)`
                if func_node.kind() == "field_expression"
                    && let Some(val_node) = func_node.child_by_field_name("value")
                {
                    walk_rust(
                        ctx,
                        val_node,
                        depth + 1,
                        owner_name,
                        owner_is_trait_or_trait_impl,
                        fn_depth,
                        struct_fields,
                        var_types,
                    );
                }
                return;
            }
        }
        "field_expression" => {
            emit_rust_field_expr(ctx, node, false, owner_name, struct_fields, var_types);
        }
        "scoped_identifier" => {
            // Skip child of call_expression/use_declaration/etc. (already handled by parent)
            let parent_kind = node.parent().map(|p| p.kind()).unwrap_or("");
            if !matches!(
                parent_kind,
                "call_expression"
                    | "generic_function"
                    | "use_declaration"
                    | "scoped_use_list"
                    | "use_as_clause"
                    | "use_wildcard"
                    | "scoped_identifier"
                    | "struct_expression"
            ) {
                let txt = ctx.text(node).trim();
                if let Some((scope, mem)) = txt.rsplit_once("::") {
                    let short = short_rust_type(scope);
                    let is_const = is_upper_snake(mem);
                    ctx.push_ref(AstRef {
                        target_name: txt.to_owned(),
                        member: is_const.then(|| mem.to_owned()),
                        context: "field".to_owned(),
                        access: if is_const { "const" } else { "type" }.to_owned(),
                        member_kind: is_const.then(|| "const".to_owned()),
                        owner_type: Some(short),
                        enclosing_symbol: owner_name.map(str::to_owned),
                        is_static: true,
                        line: start_line(node),
                        evidence: clip_str(txt, 160),
                    });
                }
            }
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_rust(
                ctx,
                ch,
                depth + 1,
                owner_name,
                owner_is_trait_or_trait_impl,
                fn_depth,
                struct_fields,
                var_types,
            );
        }
    }
}

fn extract_rust_use_decl(ctx: &mut ExtractCtx<'_>, node: Node<'_>) {
    let line = start_line(node);
    let evidence = ctx.text(node);
    let vis = rust_visibility(node, ctx.source);
    let is_reexport = vis.is_some();
    let Some(arg) = node.child_by_field_name("argument") else {
        return;
    };
    let mut expanded: Vec<(Vec<String>, Option<String>, bool)> = Vec::new();
    expand_rust_use_node(arg, ctx.source, &[], &mut expanded);
    for (segs, alias, is_wildcard) in expanded {
        if segs.is_empty() {
            continue;
        }
        let path = segs.join("::");
        let symbols = if is_wildcard {
            Vec::new()
        } else {
            let last = segs.last().cloned().unwrap_or_default();
            if last == "self" {
                segs.iter().rev().nth(1).cloned().into_iter().collect()
            } else {
                vec![last]
            }
        };
        ctx.push_import(AstImport {
            path,
            symbols,
            alias,
            is_wildcard,
            is_reexport,
            reexport_visibility: vis.clone(),
            kind: if is_reexport { "reexport" } else { "import" }.to_owned(),
            line,
            evidence: clip_str(evidence, 160),
        });
    }
}

fn expand_rust_use_node(
    node: Node<'_>,
    source: &str,
    prefix: &[String],
    out: &mut Vec<(Vec<String>, Option<String>, bool)>,
) {
    match node.kind() {
        "identifier" | "crate" | "super" | "self" | "metavariable" => {
            let mut segs = prefix.to_vec();
            segs.push(node_text(node, source).trim().to_owned());
            out.push((segs, None, false));
        }
        "scoped_identifier" => {
            let mut segs = prefix.to_vec();
            for part in node_text(node, source).split("::") {
                let p = part.trim();
                if !p.is_empty() {
                    segs.push(p.to_owned());
                }
            }
            out.push((segs, None, false));
        }
        "use_wildcard" => {
            let mut segs = prefix.to_vec();
            let txt = node_text(node, source).trim();
            if let Some(before) = txt.strip_suffix("::*") {
                for part in before.split("::") {
                    let p = part.trim();
                    if !p.is_empty() {
                        segs.push(p.to_owned());
                    }
                }
            }
            if segs.is_empty() {
                segs.push("*".to_owned());
            }
            out.push((segs, None, true));
        }
        "use_as_clause" => {
            let alias = node
                .child_by_field_name("alias")
                .map(|a| node_text(a, source).trim().to_owned());
            if let Some(path_node) = node.child_by_field_name("path") {
                let mut inner = Vec::new();
                expand_rust_use_node(path_node, source, prefix, &mut inner);
                for (segs, _, is_wc) in inner {
                    out.push((segs, alias.clone(), is_wc));
                }
            }
        }
        "scoped_use_list" => {
            let mut next_prefix = prefix.to_vec();
            if let Some(path_node) = node.child_by_field_name("path") {
                for part in node_text(path_node, source).split("::") {
                    let p = part.trim();
                    if !p.is_empty() {
                        next_prefix.push(p.to_owned());
                    }
                }
            }
            if let Some(list_node) = node.child_by_field_name("list") {
                expand_rust_use_node(list_node, source, &next_prefix, out);
            }
        }
        "use_list" => {
            for i in 0..node.child_count() {
                if let Some(ch) = node.child(i)
                    && ch.is_named()
                {
                    expand_rust_use_node(ch, source, prefix, out);
                }
            }
        }
        _ => {}
    }
}

fn rust_visibility(node: Node<'_>, source: &str) -> Option<String> {
    let vis = child_by_kind(node, "visibility_modifier")?;
    let compact = node_text(vis, source)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if compact == "pub" {
        Some("public".to_owned())
    } else {
        Some(compact.replace("pub (", "pub("))
    }
}

fn strip_rust_vis_from_sig(sig: &str) -> String {
    let trimmed = sig.trim();
    if let Some(rest) = trimmed.strip_prefix("pub")
        && !rest
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
    {
        let after_ws = rest.trim_start();
        if let Some(in_paren) = after_ws.strip_prefix('(') {
            let mut depth = 1usize;
            for (idx, ch) in in_paren.char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            return in_paren[idx + 1..].trim_start().to_owned();
                        }
                    }
                    _ => {}
                }
            }
        }
        return after_ws.to_owned();
    }
    trimmed.to_owned()
}

fn collect_rust_fn_modifiers(node: Node<'_>, source: &str) -> Vec<String> {
    let mut mods = Vec::new();
    if let Some(fm) = child_by_kind(node, "function_modifiers") {
        for word in node_text(fm, source).split_whitespace() {
            if matches!(word, "async" | "unsafe" | "const" | "extern") {
                push_unique_mod(&mut mods, word);
            }
        }
    }
    mods
}

fn find_preceding_rust_path_attr(node: Node<'_>, source: &str) -> Option<String> {
    let mut prev = node.prev_sibling();
    while let Some(sib) = prev {
        if is_comment_kind(sib.kind()) {
            prev = sib.prev_sibling();
            continue;
        }
        if sib.kind() != "attribute_item" {
            break;
        }
        let txt = node_text(sib, source);
        if txt.contains("path")
            && txt.contains('=')
            && let Some(val) = first_string_arg_text(sib, source)
        {
            return Some(val);
        }
        prev = sib.prev_sibling();
    }
    None
}

fn has_preceding_rust_test_attr(node: Node<'_>, source: &str) -> Option<u32> {
    let mut prev = node.prev_sibling();
    let mut earliest_test_line: Option<u32> = None;
    while let Some(sib) = prev {
        if is_comment_kind(sib.kind()) {
            prev = sib.prev_sibling();
            continue;
        }
        if sib.kind() != "attribute_item" {
            break;
        }
        let compact: String = node_text(sib, source)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let is_test_attr = compact == "#[test]"
            || compact.starts_with("#[test(")
            || compact.contains("::test]")
            || compact.contains("::test(")
            || compact.contains("cfg(test)")
            || compact == "#[rstest]"
            || compact.starts_with("#[rstest(");
        if is_test_attr {
            earliest_test_line = Some(start_line(sib));
        }
        prev = sib.prev_sibling();
    }
    earliest_test_line
}

fn unwrap_rust_type(raw: &str) -> String {
    let mut ty = raw.trim().trim_end_matches([',', ';', ')']).trim();
    loop {
        let before = ty;
        while let Some(rest) = ty.strip_prefix('&') {
            ty = rest.trim_start();
            if ty.starts_with('\'')
                && let Some((_, after)) = ty.split_once(char::is_whitespace)
            {
                ty = after.trim_start();
            }
            if let Some(after) = ty.strip_prefix("mut")
                && after.starts_with(char::is_whitespace)
            {
                ty = after.trim_start();
            }
        }
        for wrapper in [
            "Arc",
            "Rc",
            "Box",
            "Option",
            "Mutex",
            "RwLock",
            "RefCell",
            "Cell",
            "Pin",
            "std::sync::Arc",
            "alloc::rc::Rc",
            "std::boxed::Box",
            "core::option::Option",
        ] {
            if let Some(after_w) = ty.strip_prefix(wrapper)
                && let Some(inner_start) = after_w.trim_start().strip_prefix('<')
                && let Some(last_gt) = inner_start.rfind('>')
            {
                ty = inner_start[..last_gt].trim();
                break;
            }
        }
        if ty == before {
            break;
        }
    }
    ty.split('<').next().unwrap_or(ty).trim().to_owned()
}

fn short_rust_type(raw: &str) -> String {
    let unwrapped = unwrap_rust_type(raw);
    unwrapped
        .rsplit("::")
        .next()
        .unwrap_or(&unwrapped)
        .trim()
        .to_owned()
}

fn extract_rust_params(
    ctx: &mut ExtractCtx<'_>,
    params_node: Node<'_>,
    owner: Option<&str>,
    var_types: &mut BTreeMap<String, String>,
) {
    for param in children_by_kind(params_node, "parameter") {
        if let (Some(pat), Some(ty_node)) = (
            param.child_by_field_name("pattern"),
            param.child_by_field_name("type"),
        ) {
            let pname = ctx
                .text(pat)
                .trim()
                .trim_start_matches("mut ")
                .trim()
                .to_owned();
            let ty_unwrapped = unwrap_rust_type(ctx.text(ty_node));
            if !ty_unwrapped.is_empty() {
                if !pname.is_empty() && pname != "self" {
                    var_types.insert(pname, ty_unwrapped.clone());
                }
                ctx.push_ref(AstRef {
                    target_name: ty_unwrapped,
                    member: None,
                    context: "parameter_type".to_owned(),
                    access: "type".to_owned(),
                    member_kind: None,
                    owner_type: None,
                    enclosing_symbol: owner.map(str::to_owned),
                    is_static: false,
                    line: start_line(param),
                    evidence: clip_str(ctx.text(param), 160),
                });
            }
        }
    }
}

fn infer_rust_expr_type(
    ctx: &ExtractCtx<'_>,
    rhs: Node<'_>,
    self_fields: Option<&BTreeMap<String, String>>,
) -> Option<String> {
    match rhs.kind() {
        "struct_expression" => rhs
            .child_by_field_name("name")
            .map(|n| ctx.text(n).trim().to_owned()),
        "field_expression" => {
            let txt = ctx.text(rhs).trim();
            if let Some(field) = txt.strip_prefix("self.") {
                return self_fields.and_then(|m| m.get(field.trim()).cloned());
            }
            None
        }
        "call_expression" => {
            let fn_txt = rhs
                .child_by_field_name("function")
                .map(|n| ctx.text(n).trim())?;
            if let Some((ty, ctor)) = fn_txt.rsplit_once("::")
                && matches!(
                    ctor,
                    "new" | "default" | "create" | "open" | "connect" | "build" | "from"
                )
            {
                return Some(ty.to_owned());
            }
            None
        }
        _ => None,
    }
}

fn resolve_rust_receiver_type(
    recv: &str,
    self_fields: Option<&BTreeMap<String, String>>,
    var_types: &BTreeMap<String, String>,
) -> Option<String> {
    let trimmed = recv.trim();
    if let Some(field) = trimmed.strip_prefix("self.") {
        return self_fields.and_then(|m| m.get(field.trim()).cloned());
    }
    var_types.get(trimmed).cloned()
}

fn emit_rust_field_expr(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    is_write: bool,
    owner_name: Option<&str>,
    struct_fields: &BTreeMap<String, BTreeMap<String, String>>,
    var_types: &BTreeMap<String, String>,
) {
    let Some(val) = node.child_by_field_name("value") else {
        return;
    };
    let Some(field) = node.child_by_field_name("field") else {
        return;
    };
    let recv_txt = ctx.text(val).trim();
    let field_txt = ctx.text(field).trim().to_owned();
    let self_fields = owner_name.and_then(|o| struct_fields.get(o));
    if let Some(owner_ty) = resolve_rust_receiver_type(recv_txt, self_fields, var_types) {
        let short = short_rust_type(&owner_ty);
        ctx.push_ref(AstRef {
            target_name: owner_ty,
            member: Some(field_txt),
            context: "field".to_owned(),
            access: if is_write { "write" } else { "read" }.to_owned(),
            member_kind: Some("field".to_owned()),
            owner_type: Some(short),
            enclosing_symbol: owner_name.map(str::to_owned),
            is_static: false,
            line: start_line(node),
            evidence: clip_str(ctx.text(node), 160),
        });
    }
}

// =========================================================================
// 3. TypeScript / TSX / JavaScript / JSX Extraction
// =========================================================================

fn extract_ts_js(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    let mut this_props: BTreeMap<String, String> = BTreeMap::new();
    let mut var_types: BTreeMap<String, String> = BTreeMap::new();
    walk_ts_js(
        ctx,
        root,
        0,
        None,
        0,
        false,
        &mut this_props,
        &mut var_types,
    );
}

#[allow(clippy::too_many_arguments)]
fn walk_ts_js(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    owner_class: Option<&str>,
    fn_depth: usize,
    is_exported: bool,
    this_props: &mut BTreeMap<String, String>,
    var_types: &mut BTreeMap<String, String>,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let kind = node.kind();

    match kind {
        "import_statement" => {
            extract_ts_import(ctx, node, false);
            return;
        }
        "export_statement" => {
            if let Some(src) = node.child_by_field_name("source") {
                let spec = unquote_literal(ctx.text(src)).unwrap_or_default();
                let line = start_line(node);
                let evidence = ctx.text(node);
                let (syms, is_wc) = collect_ts_export_clause(ctx, node);
                if !spec.is_empty() {
                    ctx.push_import(AstImport {
                        path: spec,
                        symbols: syms,
                        alias: None,
                        is_wildcard: is_wc,
                        is_reexport: true,
                        reexport_visibility: Some("export".to_owned()),
                        kind: "reexport".to_owned(),
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                }
                return;
            }
            if let Some(decl) = node.child_by_field_name("declaration") {
                walk_ts_js(
                    ctx,
                    decl,
                    depth + 1,
                    owner_class,
                    fn_depth,
                    true,
                    this_props,
                    var_types,
                );
                return;
            }
        }
        "class_declaration"
        | "abstract_class_declaration"
        | "interface_declaration"
        | "enum_declaration" => {
            if fn_depth > 0 {
                return;
            }
            let decl_kind = match kind {
                "interface_declaration" => "interface",
                "enum_declaration" => "enum",
                _ => "class",
            };
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let e_line = end_line(node);
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let mut sig = header_without_comments(node, sig_end, ctx.source);
            if is_exported && !sig.starts_with("export ") {
                sig = format!("export {sig}");
            }
            let mut mods = Vec::new();
            if kind == "abstract_class_declaration" {
                push_unique_mod(&mut mods, "abstract");
            }
            emit_ts_decorators(ctx, node, Some(&name));

            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    Some(ctx.rel_path.clone()),
                    line,
                    e_line,
                    Some(if is_exported { "export" } else { "private" }.to_owned()),
                    mods,
                    sig.clone(),
                    strip_ts_vis_from_sig(&sig),
                    String::new(),
                );
                extract_ts_heritage(ctx, node, &name);
                if let Some(body) = body_node {
                    walk_ts_js(
                        ctx,
                        body,
                        depth + 1,
                        Some(&name),
                        fn_depth,
                        false,
                        this_props,
                        var_types,
                    );
                }
                this_props.clear();
                return;
            }
        }
        "type_alias_declaration" => {
            if fn_depth > 0 {
                return;
            }
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = ctx.text(name_node).trim().to_owned();
                let line = start_line(node);
                let e_line = end_line(node);
                let val_node = node.child_by_field_name("value");
                let sig_end = val_node.map_or(node.end_byte(), |v| v.start_byte());
                let mut sig = header_without_comments(node, sig_end, ctx.source)
                    .trim_end_matches(['=', ';'])
                    .trim()
                    .to_owned();
                if is_exported && !sig.starts_with("export ") {
                    sig = format!("export {sig}");
                }
                ctx.push_decl(
                    "type",
                    &name,
                    Some(ctx.rel_path.clone()),
                    line,
                    e_line,
                    Some(if is_exported { "export" } else { "private" }.to_owned()),
                    Vec::new(),
                    sig.clone(),
                    strip_ts_vis_from_sig(&sig),
                    val_node
                        .map(|v| ast_body_fingerprint(v, ctx.source))
                        .unwrap_or_default(),
                );
            }
            return;
        }
        "function_declaration" | "generator_function_declaration" | "function_signature" => {
            if fn_depth > 0 {
                return;
            }
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let e_line = end_line(node);
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let mut sig = header_without_comments(node, sig_end, ctx.source)
                .trim_end_matches(';')
                .trim()
                .to_owned();
            if is_exported && !sig.starts_with("export ") {
                sig = format!("export {sig}");
            }
            let mods = collect_ts_modifiers(node, ctx.source);
            let body_fp = body_node
                .map(|b| ast_body_fingerprint(b, ctx.source))
                .unwrap_or_default();
            if !name.is_empty() {
                ctx.push_decl(
                    "function",
                    &name,
                    Some(ctx.rel_path.clone()),
                    line,
                    e_line,
                    Some(if is_exported { "export" } else { "private" }.to_owned()),
                    mods,
                    sig.clone(),
                    strip_ts_vis_from_sig(&sig),
                    body_fp,
                );
            }
            let mut local_vars = var_types.clone();
            if let Some(params) = node.child_by_field_name("parameters") {
                extract_ts_params(ctx, params, owner_class, false, this_props, &mut local_vars);
            }
            if let Some(body) = body_node {
                walk_ts_js(
                    ctx,
                    body,
                    depth + 1,
                    owner_class,
                    fn_depth + 1,
                    false,
                    this_props,
                    &mut local_vars,
                );
            }
            return;
        }
        "method_definition" | "method_signature" | "abstract_method_signature" => {
            if let Some(owner) = owner_class {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| ctx.text(n).trim().to_owned())
                    .unwrap_or_default();
                let line = start_line(node);
                let e_line = end_line(node);
                let body_node = node.child_by_field_name("body");
                let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
                let sig = header_without_comments(node, sig_end, ctx.source)
                    .trim_end_matches(';')
                    .trim()
                    .to_owned();
                let vis = ts_member_visibility(node, &name, ctx.source);
                let mods = collect_ts_modifiers(node, ctx.source);
                let body_fp = body_node
                    .map(|b| ast_body_fingerprint(b, ctx.source))
                    .unwrap_or_default();
                emit_ts_decorators(ctx, node, Some(&format!("{owner}::{name}")));
                if !name.is_empty() {
                    ctx.push_decl(
                        "method",
                        &name,
                        Some(owner.to_owned()),
                        line,
                        e_line,
                        Some(vis),
                        mods,
                        sig.clone(),
                        strip_ts_vis_from_sig(&sig),
                        body_fp,
                    );
                }
                let mut local_vars = var_types.clone();
                if let Some(params) = node.child_by_field_name("parameters") {
                    extract_ts_params(
                        ctx,
                        params,
                        Some(owner),
                        name == "constructor",
                        this_props,
                        &mut local_vars,
                    );
                }
                if let Some(body) = body_node {
                    walk_ts_js(
                        ctx,
                        body,
                        depth + 1,
                        Some(owner),
                        fn_depth + 1,
                        false,
                        this_props,
                        &mut local_vars,
                    );
                }
                return;
            }
        }
        "public_field_definition" | "field_definition" => {
            if let Some(owner) = owner_class
                && let Some(name_node) = node.child_by_field_name("name")
            {
                let name = ctx.text(name_node).trim().to_owned();
                let line = start_line(node);
                let e_line = end_line(node);
                let val_node = node.child_by_field_name("value");
                let sig_end = val_node.map_or(node.end_byte(), |v| v.start_byte());
                let sig = header_without_comments(node, sig_end, ctx.source)
                    .trim_end_matches(['=', ';'])
                    .trim()
                    .to_owned();
                let vis = ts_member_visibility(node, &name, ctx.source);
                let mods = collect_ts_modifiers(node, ctx.source);
                let body_fp = val_node
                    .map(|v| ast_body_fingerprint(v, ctx.source))
                    .unwrap_or_default();
                if let Some(ty) = node.child_by_field_name("type")
                    && let Some(ty_ident) = extract_ts_type_ident(ctx.text(ty))
                {
                    this_props.insert(name.clone(), ty_ident.clone());
                    ctx.push_ref(AstRef {
                        target_name: ty_ident,
                        member: None,
                        context: "field".to_owned(),
                        access: "type".to_owned(),
                        member_kind: None,
                        owner_type: None,
                        enclosing_symbol: Some(owner.to_owned()),
                        is_static: false,
                        line,
                        evidence: clip_str(&sig, 160),
                    });
                }
                if !name.is_empty() {
                    ctx.push_decl(
                        "property",
                        &name,
                        Some(owner.to_owned()),
                        line,
                        e_line,
                        Some(vis),
                        mods,
                        sig.clone(),
                        strip_ts_vis_from_sig(&sig),
                        body_fp,
                    );
                }
                if let Some(v) = val_node {
                    walk_ts_js(
                        ctx,
                        v,
                        depth + 1,
                        Some(owner),
                        fn_depth,
                        false,
                        this_props,
                        var_types,
                    );
                }
                return;
            }
        }
        "lexical_declaration" | "variable_declaration" => {
            let is_const_decl = ctx.text(node).trim_start().starts_with("const ");
            for decl in children_by_kind(node, "variable_declarator") {
                let Some(name_node) = decl.child_by_field_name("name") else {
                    continue;
                };
                let var_name = ctx.text(name_node).trim().to_owned();
                let val_node = decl.child_by_field_name("value");
                if let Some(ty) = decl
                    .child_by_field_name("type")
                    .and_then(|t| extract_ts_type_ident(ctx.text(t)))
                {
                    var_types.insert(var_name.clone(), ty);
                } else if let Some(val) = val_node
                    && let Some(ty) = infer_ts_expr_type(ctx, val, this_props)
                {
                    var_types.insert(var_name.clone(), ty);
                }

                if fn_depth == 0 && owner_class.is_none() {
                    let is_arrow_or_fn = val_node.is_some_and(|v| {
                        matches!(
                            v.kind(),
                            "arrow_function" | "function_expression" | "generator_function"
                        )
                    });
                    if is_arrow_or_fn {
                        let fn_node = val_node.unwrap();
                        let body_node = fn_node.child_by_field_name("body");
                        let sig_end = body_node.map_or(decl.end_byte(), |b| b.start_byte());
                        let mut sig = header_without_comments(node, sig_end, ctx.source)
                            .trim_end_matches("=>")
                            .trim()
                            .to_owned();
                        if is_exported && !sig.starts_with("export ") {
                            sig = format!("export {sig}");
                        }
                        let body_fp = body_node
                            .map(|b| ast_body_fingerprint(b, ctx.source))
                            .unwrap_or_default();
                        ctx.push_decl(
                            "function",
                            &var_name,
                            Some(ctx.rel_path.clone()),
                            start_line(decl),
                            end_line(decl),
                            Some(if is_exported { "export" } else { "private" }.to_owned()),
                            Vec::new(),
                            sig.clone(),
                            strip_ts_vis_from_sig(&sig),
                            body_fp,
                        );
                    } else if is_const_decl && is_upper_snake(&var_name) {
                        let sig_end = val_node.map_or(decl.end_byte(), |v| v.start_byte());
                        let mut sig = header_without_comments(node, sig_end, ctx.source)
                            .trim_end_matches(['=', ';'])
                            .trim()
                            .to_owned();
                        if is_exported && !sig.starts_with("export ") {
                            sig = format!("export {sig}");
                        }
                        let body_fp = val_node
                            .map(|v| ast_body_fingerprint(v, ctx.source))
                            .unwrap_or_default();
                        ctx.push_decl(
                            "const",
                            &var_name,
                            Some(ctx.rel_path.clone()),
                            start_line(decl),
                            end_line(decl),
                            Some(if is_exported { "export" } else { "private" }.to_owned()),
                            Vec::new(),
                            sig.clone(),
                            strip_ts_vis_from_sig(&sig),
                            body_fp,
                        );
                    }
                }
            }
        }
        "assignment_expression" | "augmented_assignment_expression" => {
            if let (Some(lhs), Some(rhs)) = (
                node.child_by_field_name("left"),
                node.child_by_field_name("right"),
            ) {
                if let Some(ty) = infer_ts_expr_type(ctx, rhs, this_props) {
                    let lhs_txt = ctx.text(lhs).trim();
                    if let Some(prop) = lhs_txt.strip_prefix("this.") {
                        this_props.insert(prop.to_owned(), ty);
                    } else if lhs.kind() == "identifier" {
                        var_types.insert(lhs_txt.to_owned(), ty);
                    }
                }
                if lhs.kind() == "member_expression" {
                    emit_ts_member_expr(ctx, lhs, true, owner_class, this_props, var_types);
                    walk_ts_js(
                        ctx,
                        rhs,
                        depth + 1,
                        owner_class,
                        fn_depth,
                        false,
                        this_props,
                        var_types,
                    );
                    return;
                }
            }
        }
        "new_expression" => {
            if let Some(ctor) = node.child_by_field_name("constructor") {
                let name = ctx
                    .text(ctor)
                    .split('<')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                ctx.push_ref(AstRef {
                    target_name: name,
                    member: None,
                    context: "constructor".to_owned(),
                    access: "call".to_owned(),
                    member_kind: None,
                    owner_type: None,
                    enclosing_symbol: owner_class.map(str::to_owned),
                    is_static: false,
                    line: start_line(node),
                    evidence: clip_str(ctx.text(node), 160),
                });
            }
        }
        "call_expression" => {
            if let Some(func_node) = node.child_by_field_name("function") {
                let fn_txt = ctx.text(func_node).trim();
                let line = start_line(node);
                let evidence = ctx.text(node);
                let args_node = node.child_by_field_name("arguments");
                let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
                let str_args = args_node
                    .map(|a| all_string_arg_texts(a, ctx.source))
                    .unwrap_or_default();

                // Dynamic `import('./lazy')` or `require('./mod')`
                if matches!(fn_txt, "import" | "require")
                    && let Some(spec) = str_args.first()
                {
                    let stem = spec
                        .rsplit('/')
                        .next()
                        .unwrap_or(spec)
                        .split('.')
                        .next()
                        .unwrap_or("")
                        .to_owned();
                    ctx.push_import(AstImport {
                        path: spec.clone(),
                        symbols: if stem.is_empty() {
                            Vec::new()
                        } else {
                            vec![stem]
                        },
                        alias: None,
                        is_wildcard: false,
                        is_reexport: false,
                        reexport_visibility: None,
                        kind: "import".to_owned(),
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                }

                if func_node.kind() == "member_expression" {
                    if let (Some(obj), Some(prop)) = (
                        func_node.child_by_field_name("object"),
                        func_node.child_by_field_name("property"),
                    ) {
                        let obj_txt = ctx.text(obj).trim();
                        let prop_txt = ctx.text(prop).trim();
                        emit_semantic_from_call(
                            ctx,
                            Some(obj_txt),
                            prop_txt,
                            args_txt,
                            &str_args,
                            evidence,
                            owner_class.map(str::to_owned),
                            line,
                        );
                        let owner_ty = resolve_ts_receiver_type(obj_txt, this_props, var_types);
                        let target = owner_ty.as_deref().unwrap_or(obj_txt).to_owned();
                        ctx.push_ref(AstRef {
                            target_name: target,
                            member: Some(prop_txt.to_owned()),
                            context: "call".to_owned(),
                            access: "call".to_owned(),
                            member_kind: Some("method".to_owned()),
                            owner_type: owner_ty,
                            enclosing_symbol: owner_class.map(str::to_owned),
                            is_static: false,
                            line,
                            evidence: clip_str(evidence, 160),
                        });
                    }
                } else {
                    emit_semantic_from_call(
                        ctx,
                        None,
                        fn_txt,
                        args_txt,
                        &str_args,
                        evidence,
                        owner_class.map(str::to_owned),
                        line,
                    );
                    ctx.push_ref(AstRef {
                        target_name: fn_txt.to_owned(),
                        member: None,
                        context: "call".to_owned(),
                        access: "call".to_owned(),
                        member_kind: Some("function".to_owned()),
                        owner_type: None,
                        enclosing_symbol: owner_class.map(str::to_owned),
                        is_static: false,
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                }
                if let Some(args) = args_node {
                    walk_ts_js(
                        ctx,
                        args,
                        depth + 1,
                        owner_class,
                        fn_depth,
                        false,
                        this_props,
                        var_types,
                    );
                }
                if func_node.kind() == "member_expression"
                    && let Some(obj) = func_node.child_by_field_name("object")
                {
                    walk_ts_js(
                        ctx,
                        obj,
                        depth + 1,
                        owner_class,
                        fn_depth,
                        false,
                        this_props,
                        var_types,
                    );
                }
                return;
            }
        }
        "member_expression" => {
            emit_ts_member_expr(ctx, node, false, owner_class, this_props, var_types);
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_ts_js(
                ctx,
                ch,
                depth + 1,
                owner_class,
                fn_depth,
                false,
                this_props,
                var_types,
            );
        }
    }
}

fn extract_ts_import(ctx: &mut ExtractCtx<'_>, node: Node<'_>, is_reexport: bool) {
    let Some(src) = node.child_by_field_name("source") else {
        return;
    };
    let Some(spec) = unquote_literal(ctx.text(src)) else {
        return;
    };
    let line = start_line(node);
    let evidence = ctx.text(node);

    let Some(clause) = child_by_kind(node, "import_clause") else {
        ctx.push_import(AstImport {
            path: spec,
            symbols: Vec::new(),
            alias: None,
            is_wildcard: false,
            is_reexport,
            reexport_visibility: None,
            kind: "import".to_owned(),
            line,
            evidence: clip_str(evidence, 160),
        });
        return;
    };

    let mut emitted_any = false;
    for i in 0..clause.child_count() {
        let Some(ch) = clause.child(i) else {
            continue;
        };
        match ch.kind() {
            "identifier" => {
                let name = ctx.text(ch).trim().to_owned();
                emitted_any = true;
                ctx.push_import(AstImport {
                    path: spec.clone(),
                    symbols: vec![name],
                    alias: None,
                    is_wildcard: false,
                    is_reexport,
                    reexport_visibility: None,
                    kind: "import".to_owned(),
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
            "namespace_import" => {
                let ns = child_by_kind(ch, "identifier").map(|n| ctx.text(n).trim().to_owned());
                emitted_any = true;
                ctx.push_import(AstImport {
                    path: spec.clone(),
                    symbols: Vec::new(),
                    alias: ns,
                    is_wildcard: true,
                    is_reexport,
                    reexport_visibility: None,
                    kind: "import".to_owned(),
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
            "named_imports" => {
                for spec_node in children_by_kind(ch, "import_specifier") {
                    let orig = spec_node
                        .child_by_field_name("name")
                        .map(|n| ctx.text(n).trim().to_owned())
                        .unwrap_or_default();
                    let alias = spec_node
                        .child_by_field_name("alias")
                        .map(|n| ctx.text(n).trim().to_owned());
                    if !orig.is_empty() {
                        emitted_any = true;
                        ctx.push_import(AstImport {
                            path: spec.clone(),
                            symbols: vec![orig],
                            alias,
                            is_wildcard: false,
                            is_reexport,
                            reexport_visibility: None,
                            kind: "import".to_owned(),
                            line,
                            evidence: clip_str(evidence, 160),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    if !emitted_any {
        ctx.push_import(AstImport {
            path: spec,
            symbols: Vec::new(),
            alias: None,
            is_wildcard: false,
            is_reexport,
            reexport_visibility: None,
            kind: "import".to_owned(),
            line,
            evidence: clip_str(evidence, 160),
        });
    }
}

fn collect_ts_export_clause(ctx: &ExtractCtx<'_>, node: Node<'_>) -> (Vec<String>, bool) {
    let mut syms = Vec::new();
    let mut is_wc = false;
    for i in 0..node.child_count() {
        let Some(ch) = node.child(i) else {
            continue;
        };
        if ch.kind() == "export_clause" {
            for spec in children_by_kind(ch, "export_specifier") {
                if let Some(n) = spec.child_by_field_name("name") {
                    syms.push(ctx.text(n).trim().to_owned());
                }
            }
        } else if ch.kind() == "namespace_export" || ctx.text(ch).trim() == "*" {
            is_wc = true;
        }
    }
    (syms, is_wc)
}

fn extract_ts_heritage(ctx: &mut ExtractCtx<'_>, class_node: Node<'_>, class_name: &str) {
    let Some(heritage) = child_by_kind(class_node, "class_heritage") else {
        return;
    };
    let line = start_line(heritage);
    let evidence = ctx.text(heritage);
    for i in 0..heritage.child_count() {
        let Some(ch) = heritage.child(i) else {
            continue;
        };
        match ch.kind() {
            "extends_clause" => {
                if let Some(val) = ch.child_by_field_name("value") {
                    let base = ctx
                        .text(val)
                        .split('<')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_owned();
                    ctx.push_inheritance(class_name, &base, "extends", line, evidence);
                }
            }
            "implements_clause" => {
                for j in 0..ch.child_count() {
                    if let Some(item) = ch.child(j)
                        && item.is_named()
                    {
                        let iface = ctx
                            .text(item)
                            .split('<')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_owned();
                        ctx.push_inheritance(class_name, &iface, "implements", line, evidence);
                    }
                }
            }
            _ => {}
        }
    }
}

fn emit_ts_decorators(ctx: &mut ExtractCtx<'_>, node: Node<'_>, sym: Option<&str>) {
    let mut emit_from = |container: Node<'_>| {
        for deco in children_by_kind(container, "decorator") {
            let txt = ctx.text(deco);
            let strs = all_string_arg_texts(deco, ctx.source);
            let name = txt
                .trim_start_matches('@')
                .split('(')
                .next()
                .unwrap_or("")
                .trim();
            emit_semantic_from_attribute(ctx, name, txt, &strs, sym, &[], start_line(deco));
        }
    };
    emit_from(node);
    if let Some(parent) = node.parent()
        && parent.kind() == "export_statement"
    {
        emit_from(parent);
    }
}

fn ts_member_visibility(node: Node<'_>, name: &str, source: &str) -> String {
    if name.starts_with('#') {
        return "#private".to_owned();
    }
    if let Some(acc) = child_by_kind(node, "accessibility_modifier") {
        return node_text(acc, source).trim().to_owned();
    }
    "public".to_owned()
}

fn collect_ts_modifiers(node: Node<'_>, source: &str) -> Vec<String> {
    let mut mods = Vec::new();
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            let txt = node_text(ch, source).trim();
            if matches!(
                txt,
                "static"
                    | "readonly"
                    | "abstract"
                    | "async"
                    | "override"
                    | "declare"
                    | "get"
                    | "set"
            ) {
                push_unique_mod(&mut mods, txt);
            }
        }
    }
    mods
}

fn strip_ts_vis_from_sig(sig: &str) -> String {
    let mut parts = Vec::new();
    let mut skipping_leading = true;
    for tok in sig.split_whitespace() {
        if skipping_leading && matches!(tok, "export" | "public" | "protected" | "private") {
            continue;
        }
        skipping_leading = false;
        parts.push(tok);
    }
    parts.join(" ")
}

fn extract_ts_type_ident(raw: &str) -> Option<String> {
    let cleaned = raw
        .trim()
        .trim_start_matches(':')
        .split(['=', '|', '<', '&'])
        .next()?
        .trim();
    if cleaned
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase())
    {
        Some(cleaned.to_owned())
    } else {
        None
    }
}

fn extract_ts_params(
    ctx: &mut ExtractCtx<'_>,
    params_node: Node<'_>,
    owner_class: Option<&str>,
    is_ctor: bool,
    this_props: &mut BTreeMap<String, String>,
    var_types: &mut BTreeMap<String, String>,
) {
    for i in 0..params_node.child_count() {
        let Some(p) = params_node.child(i) else {
            continue;
        };
        if !matches!(p.kind(), "required_parameter" | "optional_parameter") {
            continue;
        }
        let pname = p
            .child_by_field_name("pattern")
            .or_else(|| p.child_by_field_name("name"))
            .map(|n| ctx.text(n).trim().to_owned())
            .unwrap_or_default();
        let ty_ident = p
            .child_by_field_name("type")
            .and_then(|t| extract_ts_type_ident(ctx.text(t)));
        if let Some(ref ty) = ty_ident {
            if !pname.is_empty() {
                var_types.insert(pname.clone(), ty.clone());
            }
            ctx.push_ref(AstRef {
                target_name: ty.clone(),
                member: None,
                context: "parameter_type".to_owned(),
                access: "type".to_owned(),
                member_kind: None,
                owner_type: None,
                enclosing_symbol: owner_class.map(str::to_owned),
                is_static: false,
                line: start_line(p),
                evidence: clip_str(ctx.text(p), 160),
            });
        }
        if is_ctor && let Some(owner) = owner_class {
            let acc =
                child_by_kind(p, "accessibility_modifier").map(|a| ctx.text(a).trim().to_owned());
            let has_readonly = (0..p.child_count())
                .filter_map(|idx| p.child(idx))
                .any(|c| ctx.text(c).trim() == "readonly");
            if acc.is_some() || has_readonly {
                let vis = acc.unwrap_or_else(|| "public".to_owned());
                let mut mods = Vec::new();
                if has_readonly {
                    mods.push("readonly".to_owned());
                }
                if let Some(ref ty) = ty_ident {
                    this_props.insert(pname.clone(), ty.clone());
                }
                let val_node = p.child_by_field_name("value");
                let sig_end = val_node.map_or(p.end_byte(), |v| v.start_byte());
                let sig = header_without_comments(p, sig_end, ctx.source)
                    .trim_end_matches('=')
                    .trim()
                    .to_owned();
                ctx.push_decl(
                    "property",
                    &pname,
                    Some(owner.to_owned()),
                    start_line(p),
                    end_line(p),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_ts_vis_from_sig(&sig),
                    val_node
                        .map(|v| ast_body_fingerprint(v, ctx.source))
                        .unwrap_or_default(),
                );
            }
        }
    }
}

fn infer_ts_expr_type(
    ctx: &ExtractCtx<'_>,
    rhs: Node<'_>,
    this_props: &BTreeMap<String, String>,
) -> Option<String> {
    match rhs.kind() {
        "new_expression" => rhs
            .child_by_field_name("constructor")
            .map(|c| {
                ctx.text(c)
                    .split('<')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_owned()
            })
            .filter(|s| !s.is_empty()),
        "member_expression" => {
            let txt = ctx.text(rhs).trim();
            if let Some(prop) = txt.strip_prefix("this.") {
                return this_props.get(prop.trim()).cloned();
            }
            None
        }
        _ => None,
    }
}

fn resolve_ts_receiver_type(
    recv: &str,
    this_props: &BTreeMap<String, String>,
    var_types: &BTreeMap<String, String>,
) -> Option<String> {
    let trimmed = recv.trim();
    if let Some(prop) = trimmed
        .strip_prefix("this.")
        .or_else(|| trimmed.strip_prefix("this?."))
    {
        return this_props.get(prop.trim()).cloned();
    }
    var_types.get(trimmed).cloned()
}

fn emit_ts_member_expr(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    is_write: bool,
    owner_class: Option<&str>,
    this_props: &BTreeMap<String, String>,
    var_types: &BTreeMap<String, String>,
) {
    let Some(obj) = node.child_by_field_name("object") else {
        return;
    };
    let Some(prop) = node.child_by_field_name("property") else {
        return;
    };
    let obj_txt = ctx.text(obj).trim();
    let prop_txt = ctx.text(prop).trim();
    let owner_ty = resolve_ts_receiver_type(obj_txt, this_props, var_types);
    let target = owner_ty.clone().unwrap_or_else(|| obj_txt.to_owned());
    let is_static = target
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase())
        && owner_ty.is_none();
    let is_const = !is_write && is_upper_snake(prop_txt);
    ctx.push_ref(AstRef {
        target_name: target,
        member: Some(prop_txt.to_owned()),
        context: "field".to_owned(),
        access: if is_write {
            "write"
        } else if is_const {
            "const"
        } else {
            "read"
        }
        .to_owned(),
        member_kind: Some(if is_const { "const" } else { "property" }.to_owned()),
        owner_type: owner_ty.or_else(|| is_static.then(|| obj_txt.to_owned())),
        enclosing_symbol: owner_class.map(str::to_owned),
        is_static,
        line: start_line(node),
        evidence: clip_str(ctx.text(node), 160),
    });
}

// =========================================================================
// 4. Python Extraction
// =========================================================================

fn extract_python(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    let mut this_attrs: BTreeMap<String, String> = BTreeMap::new();
    let mut var_types: BTreeMap<String, String> = BTreeMap::new();
    walk_python(ctx, root, 0, None, 0, &[], &mut this_attrs, &mut var_types);
}

#[allow(clippy::too_many_arguments)]
fn walk_python(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    owner_class: Option<&str>,
    fn_depth: usize,
    decorators: &[String],
    this_attrs: &mut BTreeMap<String, String>,
    var_types: &mut BTreeMap<String, String>,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let kind = node.kind();

    match kind {
        "import_statement" => {
            let line = start_line(node);
            let evidence = ctx.text(node);
            for i in 0..node.child_count() {
                let Some(ch) = node.child(i) else {
                    continue;
                };
                match ch.kind() {
                    "dotted_name" => {
                        let p = ctx.text(ch).trim().to_owned();
                        let sym = p.rsplit('.').next().unwrap_or(&p).to_owned();
                        ctx.push_import(AstImport {
                            path: p,
                            symbols: vec![sym],
                            alias: None,
                            is_wildcard: false,
                            is_reexport: false,
                            reexport_visibility: None,
                            kind: "import".to_owned(),
                            line,
                            evidence: clip_str(evidence, 160),
                        });
                    }
                    "aliased_import" => {
                        let p = ch
                            .child_by_field_name("name")
                            .map(|n| ctx.text(n).trim().to_owned())
                            .unwrap_or_default();
                        let alias = ch
                            .child_by_field_name("alias")
                            .map(|n| ctx.text(n).trim().to_owned());
                        let sym = p.rsplit('.').next().unwrap_or(&p).to_owned();
                        ctx.push_import(AstImport {
                            path: p,
                            symbols: vec![sym],
                            alias,
                            is_wildcard: false,
                            is_reexport: false,
                            reexport_visibility: None,
                            kind: "import".to_owned(),
                            line,
                            evidence: clip_str(evidence, 160),
                        });
                    }
                    _ => {}
                }
            }
            return;
        }
        "import_from_statement" => {
            let line = start_line(node);
            let evidence = ctx.text(node);
            let mod_name = node
                .child_by_field_name("module_name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let is_wc = child_by_kind(node, "wildcard_import").is_some();
            if is_wc {
                ctx.push_import(AstImport {
                    path: mod_name,
                    symbols: Vec::new(),
                    alias: None,
                    is_wildcard: true,
                    is_reexport: false,
                    reexport_visibility: None,
                    kind: "import".to_owned(),
                    line,
                    evidence: clip_str(evidence, 160),
                });
                return;
            }
            let mut emitted = false;
            for i in 0..node.child_count() {
                let Some(ch) = node.child(i) else {
                    continue;
                };
                if Some(ch) == node.child_by_field_name("module_name") {
                    continue;
                }
                match ch.kind() {
                    "dotted_name" => {
                        let sym = ctx.text(ch).trim().to_owned();
                        emitted = true;
                        ctx.push_import(AstImport {
                            path: mod_name.clone(),
                            symbols: vec![sym],
                            alias: None,
                            is_wildcard: false,
                            is_reexport: false,
                            reexport_visibility: None,
                            kind: "import".to_owned(),
                            line,
                            evidence: clip_str(evidence, 160),
                        });
                    }
                    "aliased_import" => {
                        let sym = ch
                            .child_by_field_name("name")
                            .map(|n| ctx.text(n).trim().to_owned())
                            .unwrap_or_default();
                        let alias = ch
                            .child_by_field_name("alias")
                            .map(|n| ctx.text(n).trim().to_owned());
                        if !sym.is_empty() {
                            emitted = true;
                            ctx.push_import(AstImport {
                                path: mod_name.clone(),
                                symbols: vec![sym],
                                alias,
                                is_wildcard: false,
                                is_reexport: false,
                                reexport_visibility: None,
                                kind: "import".to_owned(),
                                line,
                                evidence: clip_str(evidence, 160),
                            });
                        }
                    }
                    _ => {}
                }
            }
            if !emitted && !mod_name.is_empty() {
                ctx.push_import(AstImport {
                    path: mod_name,
                    symbols: Vec::new(),
                    alias: None,
                    is_wildcard: false,
                    is_reexport: false,
                    reexport_visibility: None,
                    kind: "import".to_owned(),
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
            return;
        }
        "decorated_definition" => {
            let mut decos = Vec::new();
            for d in children_by_kind(node, "decorator") {
                let raw = ctx.text(d);
                let name = raw
                    .trim_start_matches('@')
                    .split('(')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                let strs = all_string_arg_texts(d, ctx.source);
                emit_semantic_from_attribute(
                    ctx,
                    &name,
                    raw,
                    &strs,
                    owner_class,
                    &[],
                    start_line(d),
                );
                let short = name.rsplit('.').next().unwrap_or(&name).to_owned();
                decos.push(short);
            }
            if let Some(def) = node.child_by_field_name("definition") {
                walk_python(
                    ctx,
                    def,
                    depth + 1,
                    owner_class,
                    fn_depth,
                    &decos,
                    this_attrs,
                    var_types,
                );
            }
            return;
        }
        "class_definition" => {
            if fn_depth > 0 {
                return;
            }
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let e_line = end_line(node);
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source)
                .trim_end_matches(':')
                .trim()
                .to_owned();
            if !name.is_empty() {
                ctx.push_decl(
                    "class",
                    &name,
                    Some(ctx.rel_path.clone()),
                    line,
                    e_line,
                    Some(py_visibility(&name)),
                    Vec::new(),
                    sig.clone(),
                    sig,
                    String::new(),
                );
                if let Some(supers) = node.child_by_field_name("superclasses") {
                    for i in 0..supers.child_count() {
                        if let Some(ch) = supers.child(i)
                            && ch.is_named()
                        {
                            let base = ctx
                                .text(ch)
                                .split('[')
                                .next()
                                .unwrap_or("")
                                .trim()
                                .to_owned();
                            ctx.push_inheritance(&name, &base, "extends", line, ctx.text(supers));
                        }
                    }
                }
                if let Some(body) = body_node {
                    walk_python(
                        ctx,
                        body,
                        depth + 1,
                        Some(&name),
                        fn_depth,
                        &[],
                        this_attrs,
                        var_types,
                    );
                }
                this_attrs.clear();
            }
            return;
        }
        "function_definition" => {
            if fn_depth > 0 {
                return;
            }
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let e_line = end_line(node);
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source)
                .trim_end_matches(':')
                .trim()
                .to_owned();
            let body_fp = body_node
                .map(|b| ast_body_fingerprint(b, ctx.source))
                .unwrap_or_default();
            let is_prop = decorators.iter().any(|d| d == "property");
            let mut mods: Vec<String> = decorators
                .iter()
                .filter(|d| {
                    matches!(
                        d.as_str(),
                        "property" | "classmethod" | "staticmethod" | "abstractmethod" | "final"
                    )
                })
                .cloned()
                .collect();
            if sig.starts_with("async ") {
                push_unique_mod(&mut mods, "async");
            }
            let decl_kind = if owner_class.is_some() {
                if is_prop { "property" } else { "method" }
            } else {
                "function"
            };
            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    owner_class.map(str::to_owned),
                    line,
                    e_line,
                    Some(py_visibility(&name)),
                    mods,
                    sig.clone(),
                    sig,
                    body_fp,
                );
            }
            let mut local_vars = var_types.clone();
            if let Some(params) = node.child_by_field_name("parameters") {
                extract_py_params(ctx, params, owner_class, &mut local_vars);
            }
            if let Some(body) = body_node {
                walk_python(
                    ctx,
                    body,
                    depth + 1,
                    owner_class,
                    fn_depth + 1,
                    &[],
                    this_attrs,
                    &mut local_vars,
                );
            }
            return;
        }
        "assignment" => {
            if let Some(lhs) = node.child_by_field_name("left") {
                let lhs_txt = ctx.text(lhs).trim().to_owned();
                let rhs = node.child_by_field_name("right");
                let ty_node = node.child_by_field_name("type");
                let inferred_ty = ty_node
                    .and_then(|t| clean_py_type_str(ctx.text(t)))
                    .or_else(|| {
                        rhs.and_then(|r| infer_py_expr_type(ctx, r, this_attrs, var_types))
                    });

                if let Some(ref ty) = inferred_ty {
                    if let Some(attr) = lhs_txt.strip_prefix("self.") {
                        this_attrs.insert(attr.trim().to_owned(), ty.clone());
                    } else if lhs.kind() == "identifier" {
                        if owner_class.is_some() && fn_depth == 0 {
                            this_attrs.insert(lhs_txt.clone(), ty.clone());
                        } else {
                            var_types.insert(lhs_txt.clone(), ty.clone());
                        }
                    }
                }

                // Class property or module-level constant declarations
                if let Some(owner) = owner_class {
                    if let Some(attr) = lhs_txt.strip_prefix("self.") {
                        let attr_clean = attr.trim();
                        if !attr_clean.is_empty() && !py_decl_exists(ctx, owner, attr_clean) {
                            let line = start_line(node);
                            let sig_end = rhs.map_or(node.end_byte(), |r| r.start_byte());
                            let sig = header_without_comments(node, sig_end, ctx.source)
                                .trim_end_matches('=')
                                .trim()
                                .to_owned();
                            ctx.push_decl(
                                "property",
                                attr_clean,
                                Some(owner.to_owned()),
                                line,
                                end_line(node),
                                Some(py_visibility(attr_clean)),
                                Vec::new(),
                                sig.clone(),
                                sig,
                                rhs.map(|r| ast_body_fingerprint(r, ctx.source))
                                    .unwrap_or_default(),
                            );
                        }
                    } else if fn_depth == 0
                        && lhs.kind() == "identifier"
                        && !py_decl_exists(ctx, owner, &lhs_txt)
                    {
                        let sig_end = rhs.map_or(node.end_byte(), |r| r.start_byte());
                        let sig = header_without_comments(node, sig_end, ctx.source)
                            .trim_end_matches('=')
                            .trim()
                            .to_owned();
                        ctx.push_decl(
                            "property",
                            &lhs_txt,
                            Some(owner.to_owned()),
                            start_line(node),
                            end_line(node),
                            Some(py_visibility(&lhs_txt)),
                            Vec::new(),
                            sig.clone(),
                            sig,
                            rhs.map(|r| ast_body_fingerprint(r, ctx.source))
                                .unwrap_or_default(),
                        );
                    }
                } else if fn_depth == 0 && lhs.kind() == "identifier" && is_upper_snake(&lhs_txt) {
                    let sig_end = rhs.map_or(node.end_byte(), |r| r.start_byte());
                    let sig = header_without_comments(node, sig_end, ctx.source)
                        .trim_end_matches('=')
                        .trim()
                        .to_owned();
                    ctx.push_decl(
                        "const",
                        &lhs_txt,
                        Some(ctx.rel_path.clone()),
                        start_line(node),
                        end_line(node),
                        Some(py_visibility(&lhs_txt)),
                        Vec::new(),
                        sig.clone(),
                        sig,
                        rhs.map(|r| ast_body_fingerprint(r, ctx.source))
                            .unwrap_or_default(),
                    );
                }

                if lhs.kind() == "attribute" {
                    emit_py_attr(ctx, lhs, true, owner_class, this_attrs, var_types);
                    if let Some(r) = rhs {
                        walk_python(
                            ctx,
                            r,
                            depth + 1,
                            owner_class,
                            fn_depth,
                            &[],
                            this_attrs,
                            var_types,
                        );
                    }
                    return;
                }
            }
        }
        "call" => {
            if let Some(func_node) = node.child_by_field_name("function") {
                let line = start_line(node);
                let evidence = ctx.text(node);
                let args_node = node.child_by_field_name("arguments");
                let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
                let str_args = args_node
                    .map(|a| all_string_arg_texts(a, ctx.source))
                    .unwrap_or_default();

                if func_node.kind() == "attribute" {
                    if let (Some(obj), Some(attr)) = (
                        func_node.child_by_field_name("object"),
                        func_node.child_by_field_name("attribute"),
                    ) {
                        let obj_txt = ctx.text(obj).trim();
                        let attr_txt = ctx.text(attr).trim();
                        emit_semantic_from_call(
                            ctx,
                            Some(obj_txt),
                            attr_txt,
                            args_txt,
                            &str_args,
                            evidence,
                            owner_class.map(str::to_owned),
                            line,
                        );
                        let owner_ty = resolve_py_receiver_type(obj_txt, this_attrs, var_types);
                        let target = owner_ty.clone().unwrap_or_else(|| obj_txt.to_owned());
                        ctx.push_ref(AstRef {
                            target_name: target,
                            member: Some(attr_txt.to_owned()),
                            context: "call".to_owned(),
                            access: "call".to_owned(),
                            member_kind: Some("method".to_owned()),
                            owner_type: owner_ty,
                            enclosing_symbol: owner_class.map(str::to_owned),
                            is_static: false,
                            line,
                            evidence: clip_str(evidence, 160),
                        });
                    }
                } else {
                    let fn_txt = ctx.text(func_node).trim();
                    emit_semantic_from_call(
                        ctx,
                        None,
                        fn_txt,
                        args_txt,
                        &str_args,
                        evidence,
                        owner_class.map(str::to_owned),
                        line,
                    );
                    ctx.push_ref(AstRef {
                        target_name: fn_txt.to_owned(),
                        member: None,
                        context: "call".to_owned(),
                        access: "call".to_owned(),
                        member_kind: Some("function".to_owned()),
                        owner_type: None,
                        enclosing_symbol: owner_class.map(str::to_owned),
                        is_static: false,
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                }
                if let Some(args) = args_node {
                    walk_python(
                        ctx,
                        args,
                        depth + 1,
                        owner_class,
                        fn_depth,
                        &[],
                        this_attrs,
                        var_types,
                    );
                }
                if func_node.kind() == "attribute"
                    && let Some(obj) = func_node.child_by_field_name("object")
                {
                    walk_python(
                        ctx,
                        obj,
                        depth + 1,
                        owner_class,
                        fn_depth,
                        &[],
                        this_attrs,
                        var_types,
                    );
                }
                return;
            }
        }
        "attribute" => {
            emit_py_attr(ctx, node, false, owner_class, this_attrs, var_types);
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_python(
                ctx,
                ch,
                depth + 1,
                owner_class,
                fn_depth,
                &[],
                this_attrs,
                var_types,
            );
        }
    }
}

fn py_decl_exists(ctx: &ExtractCtx<'_>, owner: &str, name: &str) -> bool {
    ctx.declarations.iter().any(|d| {
        d.entity.parent.as_deref() == Some(owner)
            && d.entity.name == name
            && d.entity.kind == "property"
    })
}

fn py_visibility(name: &str) -> String {
    if name.starts_with("__") && !name.ends_with("__") {
        "private".to_owned()
    } else if name.starts_with('_') && !name.starts_with("__") {
        "protected".to_owned()
    } else {
        "public".to_owned()
    }
}

fn clean_py_type_str(raw: &str) -> Option<String> {
    let mut s = raw.split(['=', ',']).next().unwrap_or(raw).trim();
    for wrapper in ["Optional[", "Union[", "Annotated["] {
        if let Some(inner) = s.strip_prefix(wrapper) {
            s = inner.trim_end_matches(']').trim();
            if wrapper == "Annotated[" {
                s = s.split(',').next().unwrap_or(s).trim();
            }
        }
    }
    let chosen = s
        .split('|')
        .map(str::trim)
        .find(|part| !part.is_empty() && *part != "None")?;
    let base = chosen.split('[').next().unwrap_or(chosen).trim();
    if matches!(
        base,
        "" | "str"
            | "int"
            | "float"
            | "bool"
            | "bytes"
            | "list"
            | "dict"
            | "set"
            | "tuple"
            | "Any"
            | "None"
            | "Self"
            | "object"
            | "Callable"
    ) {
        return None;
    }
    Some(base.to_owned())
}

fn extract_py_params(
    ctx: &mut ExtractCtx<'_>,
    params_node: Node<'_>,
    owner: Option<&str>,
    var_types: &mut BTreeMap<String, String>,
) {
    for i in 0..params_node.child_count() {
        let Some(p) = params_node.child(i) else {
            continue;
        };
        if !matches!(p.kind(), "typed_parameter" | "typed_default_parameter") {
            continue;
        }
        let pname = p
            .child(0)
            .map(|n| ctx.text(n).trim().trim_start_matches('*').to_owned())
            .unwrap_or_default();
        if let Some(ty_node) = p.child_by_field_name("type")
            && let Some(ty) = clean_py_type_str(ctx.text(ty_node))
        {
            if !pname.is_empty() && !matches!(pname.as_str(), "self" | "cls") {
                var_types.insert(pname, ty.clone());
            }
            ctx.push_ref(AstRef {
                target_name: ty,
                member: None,
                context: "parameter_type".to_owned(),
                access: "type".to_owned(),
                member_kind: None,
                owner_type: None,
                enclosing_symbol: owner.map(str::to_owned),
                is_static: false,
                line: start_line(p),
                evidence: clip_str(ctx.text(p), 160),
            });
        }
    }
}

fn infer_py_expr_type(
    ctx: &ExtractCtx<'_>,
    rhs: Node<'_>,
    this_attrs: &BTreeMap<String, String>,
    var_types: &BTreeMap<String, String>,
) -> Option<String> {
    match rhs.kind() {
        "call" => rhs
            .child_by_field_name("function")
            .map(|f| ctx.text(f).trim().to_owned())
            .filter(|s| {
                s.rsplit('.')
                    .next()
                    .is_some_and(|c| c.chars().next().is_some_and(|ch| ch.is_ascii_uppercase()))
            }),
        "attribute" => {
            let txt = ctx.text(rhs).trim();
            txt.strip_prefix("self.")
                .and_then(|a| this_attrs.get(a.trim()).cloned())
        }
        "identifier" => var_types.get(ctx.text(rhs).trim()).cloned(),
        _ => None,
    }
}

fn resolve_py_receiver_type(
    recv: &str,
    this_attrs: &BTreeMap<String, String>,
    var_types: &BTreeMap<String, String>,
) -> Option<String> {
    let trimmed = recv.trim();
    if let Some(attr) = trimmed.strip_prefix("self.") {
        return this_attrs.get(attr.trim()).cloned();
    }
    var_types.get(trimmed).cloned()
}

fn emit_py_attr(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    is_write: bool,
    owner_class: Option<&str>,
    this_attrs: &BTreeMap<String, String>,
    var_types: &BTreeMap<String, String>,
) {
    let (Some(obj), Some(attr)) = (
        node.child_by_field_name("object"),
        node.child_by_field_name("attribute"),
    ) else {
        return;
    };
    let obj_txt = ctx.text(obj).trim();
    let attr_txt = ctx.text(attr).trim();
    if obj_txt == "self" {
        return;
    }
    let owner_ty = resolve_py_receiver_type(obj_txt, this_attrs, var_types);
    let target = owner_ty.clone().unwrap_or_else(|| obj_txt.to_owned());
    let is_const = !is_write && is_upper_snake(attr_txt);
    ctx.push_ref(AstRef {
        target_name: target,
        member: Some(attr_txt.to_owned()),
        context: "field".to_owned(),
        access: if is_write {
            "write"
        } else if is_const {
            "const"
        } else {
            "read"
        }
        .to_owned(),
        member_kind: Some(if is_const { "const" } else { "property" }.to_owned()),
        owner_type: owner_ty,
        enclosing_symbol: owner_class.map(str::to_owned),
        is_static: false,
        line: start_line(node),
        evidence: clip_str(ctx.text(node), 160),
    });
}

// =========================================================================
// 5. Go Extraction
// =========================================================================

fn extract_go(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    walk_go(ctx, root, 0, None, 0);
}

fn walk_go(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    owner_type: Option<&str>,
    fn_depth: usize,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    match node.kind() {
        "package_clause" => {
            if let Some(pkg) = child_by_kind(node, "package_identifier") {
                ctx.namespace = Some(ctx.text(pkg).trim().to_owned());
            }
            return;
        }
        "import_declaration" => {
            let line = start_line(node);
            let evidence = ctx.text(node);
            let mut stack = vec![node];
            while let Some(cur) = stack.pop() {
                if cur.kind() == "import_spec" {
                    let path = cur
                        .child_by_field_name("path")
                        .and_then(|p| unquote_literal(ctx.text(p)))
                        .unwrap_or_default();
                    let alias = cur
                        .child_by_field_name("name")
                        .map(|n| ctx.text(n).trim().to_owned());
                    let short = path.rsplit('/').next().unwrap_or(&path).to_owned();
                    ctx.push_import(AstImport {
                        path,
                        symbols: if short.is_empty() {
                            vec![]
                        } else {
                            vec![short]
                        },
                        alias,
                        is_wildcard: false,
                        is_reexport: false,
                        reexport_visibility: None,
                        kind: "import".to_owned(),
                        line: start_line(cur),
                        evidence: clip_str(evidence, 160),
                    });
                    continue;
                }
                for i in (0..cur.child_count()).rev() {
                    if let Some(ch) = cur.child(i) {
                        stack.push(ch);
                    }
                }
            }
            let _ = line;
            return;
        }
        "type_spec" => {
            if fn_depth > 0 {
                return;
            }
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let ty_node = node.child_by_field_name("type");
            let decl_kind = match ty_node.map(|t| t.kind()) {
                Some("struct_type") => "struct",
                Some("interface_type") => "interface",
                _ => "type",
            };
            let line = start_line(node);
            let e_line = end_line(node);
            let sig = header_without_comments(
                node,
                ty_node.map_or(node.end_byte(), |t| t.start_byte()),
                ctx.source,
            );
            let vis = go_visibility(&name);
            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    Some(ctx.rel_path.clone()),
                    line,
                    e_line,
                    Some(vis),
                    Vec::new(),
                    format!("type {sig} {decl_kind}"),
                    format!("type {sig} {decl_kind}"),
                    String::new(),
                );
                if let Some(t) = ty_node {
                    walk_go(ctx, t, depth + 1, Some(&name), fn_depth);
                }
            }
            return;
        }
        "field_declaration" => {
            if let Some(owner) = owner_type
                && let Some(name_node) = node.child_by_field_name("name")
            {
                let name = ctx.text(name_node).trim().to_owned();
                let sig = header_without_comments(node, node.end_byte(), ctx.source);
                ctx.push_decl(
                    "field",
                    &name,
                    Some(owner.to_owned()),
                    start_line(node),
                    end_line(node),
                    Some(go_visibility(&name)),
                    Vec::new(),
                    sig.clone(),
                    sig,
                    String::new(),
                );
            }
            return;
        }
        "method_elem" => {
            if let Some(owner) = owner_type
                && let Some(name_node) = node.child_by_field_name("name")
            {
                let name = ctx.text(name_node).trim().to_owned();
                let sig = header_without_comments(node, node.end_byte(), ctx.source);
                ctx.push_decl(
                    "method",
                    &name,
                    Some(owner.to_owned()),
                    start_line(node),
                    end_line(node),
                    Some(go_visibility(&name)),
                    Vec::new(),
                    sig.clone(),
                    sig,
                    String::new(),
                );
            }
            return;
        }
        "const_spec" => {
            if fn_depth == 0
                && let Some(name_node) = node.child_by_field_name("name")
            {
                let name = ctx.text(name_node).trim().to_owned();
                let val_node = node.child_by_field_name("value");
                let sig_end = val_node.map_or(node.end_byte(), |v| v.start_byte());
                let sig = header_without_comments(node, sig_end, ctx.source)
                    .trim_end_matches('=')
                    .trim()
                    .to_owned();
                ctx.push_decl(
                    "const",
                    &name,
                    Some(ctx.rel_path.clone()),
                    start_line(node),
                    end_line(node),
                    Some(go_visibility(&name)),
                    Vec::new(),
                    sig.clone(),
                    sig,
                    val_node
                        .map(|v| ast_body_fingerprint(v, ctx.source))
                        .unwrap_or_default(),
                );
            }
            return;
        }
        "function_declaration" | "method_declaration" => {
            if fn_depth > 0 {
                return;
            }
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let recv_owner = node.child_by_field_name("receiver").and_then(|r| {
                let raw = ctx.text(r);
                raw.trim_matches(|c| matches!(c, '(' | ')' | ' '))
                    .split_whitespace()
                    .last()
                    .map(|t| t.trim_start_matches('*').to_owned())
            });
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let body_fp = body_node
                .map(|b| ast_body_fingerprint(b, ctx.source))
                .unwrap_or_default();
            let decl_kind = if recv_owner.is_some() {
                "method"
            } else {
                "function"
            };
            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    recv_owner.clone(),
                    start_line(node),
                    end_line(node),
                    Some(go_visibility(&name)),
                    Vec::new(),
                    sig.clone(),
                    sig,
                    body_fp,
                );
            }
            if let Some(body) = body_node {
                walk_go(ctx, body, depth + 1, recv_owner.as_deref(), fn_depth + 1);
            }
            return;
        }
        "call_expression" => {
            if let Some(fn_node) = node.child_by_field_name("function") {
                let line = start_line(node);
                let evidence = ctx.text(node);
                let args_node = node.child_by_field_name("arguments");
                let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
                let str_args = args_node
                    .map(|a| all_string_arg_texts(a, ctx.source))
                    .unwrap_or_default();
                if fn_node.kind() == "selector_expression" {
                    if let (Some(op), Some(field)) = (
                        fn_node.child_by_field_name("operand"),
                        fn_node.child_by_field_name("field"),
                    ) {
                        let op_txt = ctx.text(op).trim();
                        let field_txt = ctx.text(field).trim();
                        emit_semantic_from_call(
                            ctx,
                            Some(op_txt),
                            field_txt,
                            args_txt,
                            &str_args,
                            evidence,
                            owner_type.map(str::to_owned),
                            line,
                        );
                        ctx.push_ref(AstRef {
                            target_name: op_txt.to_owned(),
                            member: Some(field_txt.to_owned()),
                            context: "call".to_owned(),
                            access: "call".to_owned(),
                            member_kind: Some("function".to_owned()),
                            owner_type: None,
                            enclosing_symbol: owner_type.map(str::to_owned),
                            is_static: false,
                            line,
                            evidence: clip_str(evidence, 160),
                        });
                    }
                } else {
                    let fn_txt = ctx.text(fn_node).trim();
                    emit_semantic_from_call(
                        ctx,
                        None,
                        fn_txt,
                        args_txt,
                        &str_args,
                        evidence,
                        owner_type.map(str::to_owned),
                        line,
                    );
                    ctx.push_ref(AstRef {
                        target_name: fn_txt.to_owned(),
                        member: None,
                        context: "call".to_owned(),
                        access: "call".to_owned(),
                        member_kind: Some("function".to_owned()),
                        owner_type: None,
                        enclosing_symbol: owner_type.map(str::to_owned),
                        is_static: false,
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                }
            }
        }
        "selector_expression" => {
            if node.parent().is_some_and(|p| p.kind() != "call_expression")
                && let (Some(op), Some(field)) = (
                    node.child_by_field_name("operand"),
                    node.child_by_field_name("field"),
                )
            {
                let op_txt = ctx.text(op).trim();
                let field_txt = ctx.text(field).trim();
                ctx.push_ref(AstRef {
                    target_name: op_txt.to_owned(),
                    member: Some(field_txt.to_owned()),
                    context: "field".to_owned(),
                    access: "read".to_owned(),
                    member_kind: Some("field".to_owned()),
                    owner_type: None,
                    enclosing_symbol: owner_type.map(str::to_owned),
                    is_static: false,
                    line: start_line(node),
                    evidence: clip_str(ctx.text(node), 160),
                });
            }
        }
        _ => {}
    }
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_go(ctx, ch, depth + 1, owner_type, fn_depth);
        }
    }
}

fn go_visibility(name: &str) -> String {
    if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        "public".to_owned()
    } else {
        "package".to_owned()
    }
}

// =========================================================================
// 6. Java Extraction
// =========================================================================

fn extract_java(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    walk_java(ctx, root, 0, None, 0);
}

fn walk_java(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    owner_class: Option<&str>,
    fn_depth: usize,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    match node.kind() {
        "package_declaration" => {
            let pkg = ctx
                .text(node)
                .trim_start_matches("package")
                .trim_end_matches(';')
                .trim()
                .to_owned();
            if !pkg.is_empty() {
                ctx.namespace = Some(pkg);
            }
            return;
        }
        "import_declaration" => {
            let raw = ctx
                .text(node)
                .trim_start_matches("import")
                .trim()
                .trim_start_matches("static")
                .trim()
                .trim_end_matches(';')
                .trim();
            let is_wc = raw.ends_with(".*");
            let path = raw.trim_end_matches(".*").to_owned();
            let sym = path.rsplit('.').next().unwrap_or(&path).to_owned();
            ctx.push_import(AstImport {
                path,
                symbols: if is_wc || sym.is_empty() {
                    Vec::new()
                } else {
                    vec![sym]
                },
                alias: None,
                is_wildcard: is_wc,
                is_reexport: false,
                reexport_visibility: None,
                kind: "import".to_owned(),
                line: start_line(node),
                evidence: clip_str(ctx.text(node), 160),
            });
            return;
        }
        "class_declaration"
        | "interface_declaration"
        | "enum_declaration"
        | "record_declaration"
        | "annotation_type_declaration" => {
            let decl_kind = match node.kind() {
                "interface_declaration" | "annotation_type_declaration" => "interface",
                "enum_declaration" => "enum",
                _ => "class",
            };
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let (vis, mods) = parse_jvm_modifiers(node, ctx.source, "package");
            emit_jvm_annotations(ctx, node, Some(&name), &[]);
            if !name.is_empty() && fn_depth == 0 {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    owner_class.map(str::to_owned),
                    line,
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_leading_vis_word(&sig),
                    String::new(),
                );
                if let Some(sc) = node.child_by_field_name("superclass") {
                    let base = ctx
                        .text(sc)
                        .trim_start_matches("extends")
                        .split('<')
                        .next()
                        .unwrap_or("")
                        .trim();
                    ctx.push_inheritance(&name, base, "extends", line, ctx.text(sc));
                }
                if let Some(ifaces) = node.child_by_field_name("interfaces") {
                    for part in ctx.text(ifaces).trim_start_matches("implements").split(',') {
                        let p = part.split('<').next().unwrap_or("").trim();
                        ctx.push_inheritance(&name, p, "implements", line, ctx.text(ifaces));
                    }
                }
                if let Some(ext_ifaces) = child_by_kind(node, "extends_interfaces") {
                    for part in ctx
                        .text(ext_ifaces)
                        .trim_start_matches("extends")
                        .split(',')
                    {
                        let p = part.split('<').next().unwrap_or("").trim();
                        ctx.push_inheritance(&name, p, "extends", line, ctx.text(ext_ifaces));
                    }
                }
                if let Some(body) = body_node {
                    walk_java(ctx, body, depth + 1, Some(&name), fn_depth);
                }
            }
            return;
        }
        "method_declaration" | "constructor_declaration" => {
            if let Some(owner) = owner_class {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| ctx.text(n).trim().to_owned())
                    .unwrap_or_default();
                let body_node = node.child_by_field_name("body");
                let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
                let sig = header_without_comments(node, sig_end, ctx.source)
                    .trim_end_matches(';')
                    .trim()
                    .to_owned();
                let (vis, mods) = parse_jvm_modifiers(node, ctx.source, "package");
                let param_types = node
                    .child_by_field_name("parameters")
                    .map(|p| collect_java_param_types(p, ctx.source))
                    .unwrap_or_default();
                emit_jvm_annotations(ctx, node, Some(&format!("{owner}::{name}")), &param_types);
                ctx.push_decl(
                    "method",
                    &name,
                    Some(owner.to_owned()),
                    start_line(node),
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_leading_vis_word(&sig),
                    body_node
                        .map(|b| ast_body_fingerprint(b, ctx.source))
                        .unwrap_or_default(),
                );
                if let Some(body) = body_node {
                    walk_java(ctx, body, depth + 1, Some(owner), fn_depth + 1);
                }
                return;
            }
        }
        "field_declaration" | "constant_declaration" => {
            if let Some(owner) = owner_class {
                let (vis, mods) = parse_jvm_modifiers(node, ctx.source, "package");
                let is_const = node.kind() == "constant_declaration"
                    || (mods.iter().any(|m| m == "static") && mods.iter().any(|m| m == "final"));
                for decl in children_by_kind(node, "variable_declarator") {
                    if let Some(n) = decl.child_by_field_name("name") {
                        let name = ctx.text(n).trim().to_owned();
                        let val_node = decl.child_by_field_name("value");
                        let sig = header_without_comments(
                            node,
                            val_node.map_or(decl.end_byte(), |v| v.start_byte()),
                            ctx.source,
                        )
                        .trim_end_matches(['=', ';'])
                        .trim()
                        .to_owned();
                        ctx.push_decl(
                            if is_const { "const" } else { "field" },
                            &name,
                            Some(owner.to_owned()),
                            start_line(decl),
                            end_line(decl),
                            Some(vis.clone()),
                            mods.clone(),
                            sig.clone(),
                            strip_leading_vis_word(&sig),
                            val_node
                                .map(|v| ast_body_fingerprint(v, ctx.source))
                                .unwrap_or_default(),
                        );
                    }
                }
                return;
            }
        }
        "object_creation_expression" => {
            if let Some(ty) = node.child_by_field_name("type") {
                let name = ctx
                    .text(ty)
                    .split('<')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                ctx.push_ref(AstRef {
                    target_name: name,
                    member: None,
                    context: "constructor".to_owned(),
                    access: "call".to_owned(),
                    member_kind: None,
                    owner_type: None,
                    enclosing_symbol: owner_class.map(str::to_owned),
                    is_static: false,
                    line: start_line(node),
                    evidence: clip_str(ctx.text(node), 160),
                });
            }
        }
        "method_invocation" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let mem = ctx.text(name_node).trim().to_owned();
                let obj = node
                    .child_by_field_name("object")
                    .map(|o| ctx.text(o).trim().to_owned());
                let args_node = node.child_by_field_name("arguments");
                let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
                let str_args = args_node
                    .map(|a| all_string_arg_texts(a, ctx.source))
                    .unwrap_or_default();
                let line = start_line(node);
                let evidence = ctx.text(node);
                emit_semantic_from_call(
                    ctx,
                    obj.as_deref(),
                    &mem,
                    args_txt,
                    &str_args,
                    evidence,
                    owner_class.map(str::to_owned),
                    line,
                );
                let target = obj.clone().unwrap_or_else(|| mem.clone());
                ctx.push_ref(AstRef {
                    target_name: target,
                    member: obj.is_some().then_some(mem),
                    context: "call".to_owned(),
                    access: "call".to_owned(),
                    member_kind: Some(if obj.is_some() { "method" } else { "function" }.to_owned()),
                    owner_type: obj,
                    enclosing_symbol: owner_class.map(str::to_owned),
                    is_static: false,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
        }
        _ => {}
    }
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_java(ctx, ch, depth + 1, owner_class, fn_depth);
        }
    }
}

fn parse_jvm_modifiers(node: Node<'_>, source: &str, default_vis: &str) -> (String, Vec<String>) {
    let mut vis = default_vis.to_owned();
    let mut mods = Vec::new();
    if let Some(mnode) = child_by_kind(node, "modifiers") {
        for word in node_text(mnode, source).split_whitespace() {
            if matches!(word, "public" | "protected" | "private" | "internal") {
                vis = word.to_owned();
            } else if !word.starts_with('@') {
                push_unique_mod(&mut mods, word);
            }
        }
    }
    (vis, mods)
}

fn emit_jvm_annotations(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    sym: Option<&str>,
    param_types: &[String],
) {
    let mut stack = vec![node];
    while let Some(cur) = stack.pop() {
        if matches!(cur.kind(), "annotation" | "marker_annotation") {
            let txt = ctx.text(cur);
            let name = cur
                .child_by_field_name("name")
                .map(|n| ctx.text(n))
                .unwrap_or(txt);
            let strs = all_string_arg_texts(cur, ctx.source);
            emit_semantic_from_attribute(ctx, name, txt, &strs, sym, param_types, start_line(cur));
            continue;
        }
        // Only descend into `modifiers` / direct annotation siblings, not body!
        if cur == node || cur.kind() == "modifiers" {
            for i in (0..cur.child_count()).rev() {
                if let Some(ch) = cur.child(i) {
                    stack.push(ch);
                }
            }
        }
    }
}

fn collect_java_param_types(params: Node<'_>, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for p in children_by_kind(params, "formal_parameter") {
        if let Some(ty) = p.child_by_field_name("type") {
            let short = node_text(ty, source)
                .split('<')
                .next()
                .unwrap_or("")
                .rsplit('.')
                .next()
                .unwrap_or("")
                .trim();
            if !short.is_empty() {
                out.push(short.to_owned());
            }
        }
    }
    out
}

fn strip_leading_vis_word(sig: &str) -> String {
    let mut parts = Vec::new();
    let mut skip = true;
    for tok in sig.split_whitespace() {
        if skip && matches!(tok, "public" | "protected" | "private" | "internal") {
            continue;
        }
        skip = false;
        parts.push(tok);
    }
    parts.join(" ")
}

// =========================================================================
// 7. C# Extraction
// =========================================================================

fn extract_csharp(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    walk_csharp(ctx, root, 0, None, 0);
}

fn walk_csharp(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    owner_class: Option<&str>,
    fn_depth: usize,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    match node.kind() {
        "namespace_declaration" | "file_scoped_namespace_declaration" => {
            if let Some(name) = node.child_by_field_name("name") {
                ctx.namespace = Some(ctx.text(name).trim().to_owned());
            }
        }
        "using_directive" => {
            let raw = ctx
                .text(node)
                .trim_start_matches("global")
                .trim()
                .trim_start_matches("using")
                .trim()
                .trim_start_matches("static")
                .trim()
                .trim_end_matches(';')
                .trim();
            let (alias, path) = if let Some((a, p)) = raw.split_once('=') {
                (Some(a.trim().to_owned()), p.trim().to_owned())
            } else {
                (None, raw.to_owned())
            };
            let sym = path.rsplit('.').next().unwrap_or(&path).to_owned();
            let is_wildcard = alias.is_none();
            ctx.push_import(AstImport {
                path,
                symbols: if sym.is_empty() { vec![] } else { vec![sym] },
                alias,
                is_wildcard,
                is_reexport: false,
                reexport_visibility: None,
                kind: "import".to_owned(),
                line: start_line(node),
                evidence: clip_str(ctx.text(node), 160),
            });
            return;
        }
        "class_declaration"
        | "struct_declaration"
        | "interface_declaration"
        | "enum_declaration"
        | "record_declaration" => {
            let decl_kind = match node.kind() {
                "struct_declaration" => "struct",
                "interface_declaration" => "interface",
                "enum_declaration" => "enum",
                _ => "class",
            };
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let (vis, mods) = parse_cs_modifiers(node, ctx.source, "internal");
            emit_cs_attributes(ctx, node, Some(&name));
            if !name.is_empty() && fn_depth == 0 {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    owner_class.map(str::to_owned),
                    line,
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_leading_vis_word(&sig),
                    String::new(),
                );
                if let Some(base_list) = child_by_kind(node, "base_list") {
                    for (idx, part) in ctx
                        .text(base_list)
                        .trim_start_matches(':')
                        .split(',')
                        .enumerate()
                    {
                        let base = part.split(['<', '(']).next().unwrap_or("").trim();
                        let ik = if idx == 0 && !base.starts_with('I') {
                            "extends"
                        } else {
                            "implements"
                        };
                        ctx.push_inheritance(&name, base, ik, line, ctx.text(base_list));
                    }
                }
                if let Some(body) = body_node {
                    walk_csharp(ctx, body, depth + 1, Some(&name), fn_depth);
                }
            }
            return;
        }
        "method_declaration" | "constructor_declaration" => {
            if let Some(owner) = owner_class {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| ctx.text(n).trim().to_owned())
                    .unwrap_or_default();
                let body_node = node.child_by_field_name("body");
                let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
                let sig = header_without_comments(node, sig_end, ctx.source)
                    .trim_end_matches(';')
                    .trim()
                    .to_owned();
                let (vis, mods) = parse_cs_modifiers(node, ctx.source, "private");
                emit_cs_attributes(ctx, node, Some(&format!("{owner}::{name}")));
                ctx.push_decl(
                    "method",
                    &name,
                    Some(owner.to_owned()),
                    start_line(node),
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_leading_vis_word(&sig),
                    body_node
                        .map(|b| ast_body_fingerprint(b, ctx.source))
                        .unwrap_or_default(),
                );
                if let Some(body) = body_node {
                    walk_csharp(ctx, body, depth + 1, Some(owner), fn_depth + 1);
                }
                return;
            }
        }
        "property_declaration" => {
            if let Some(owner) = owner_class
                && let Some(n) = node.child_by_field_name("name")
            {
                let name = ctx.text(n).trim().to_owned();
                let (vis, mods) = parse_cs_modifiers(node, ctx.source, "private");
                let sig = header_without_comments(
                    node,
                    node.child_by_field_name("accessors")
                        .map_or(node.end_byte(), |a| a.start_byte()),
                    ctx.source,
                );
                ctx.push_decl(
                    "property",
                    &name,
                    Some(owner.to_owned()),
                    start_line(node),
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_leading_vis_word(&sig),
                    String::new(),
                );
            }
            return;
        }
        "field_declaration" => {
            if let Some(owner) = owner_class
                && let Some(vd) = child_by_kind(node, "variable_declaration")
            {
                let (vis, mods) = parse_cs_modifiers(node, ctx.source, "private");
                let is_const = mods.iter().any(|m| m == "const");
                for decl in children_by_kind(vd, "variable_declarator") {
                    if let Some(n) = decl.child_by_field_name("name") {
                        let name = ctx.text(n).trim().to_owned();
                        let sig = header_without_comments(node, decl.end_byte(), ctx.source)
                            .split('=')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_owned();
                        ctx.push_decl(
                            if is_const { "const" } else { "field" },
                            &name,
                            Some(owner.to_owned()),
                            start_line(decl),
                            end_line(decl),
                            Some(vis.clone()),
                            mods.clone(),
                            sig.clone(),
                            strip_leading_vis_word(&sig),
                            String::new(),
                        );
                    }
                }
                return;
            }
        }
        "object_creation_expression" => {
            if let Some(ty) = node.child_by_field_name("type") {
                ctx.push_ref(AstRef {
                    target_name: ctx
                        .text(ty)
                        .split('<')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_owned(),
                    member: None,
                    context: "constructor".to_owned(),
                    access: "call".to_owned(),
                    member_kind: None,
                    owner_type: None,
                    enclosing_symbol: owner_class.map(str::to_owned),
                    is_static: false,
                    line: start_line(node),
                    evidence: clip_str(ctx.text(node), 160),
                });
            }
        }
        "invocation_expression" => {
            if let Some(fn_node) = node.child_by_field_name("function") {
                let line = start_line(node);
                let evidence = ctx.text(node);
                let args_node = node.child_by_field_name("arguments");
                let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
                let str_args = args_node
                    .map(|a| all_string_arg_texts(a, ctx.source))
                    .unwrap_or_default();
                if fn_node.kind() == "member_access_expression" {
                    let obj = fn_node
                        .child_by_field_name("expression")
                        .map(|n| ctx.text(n).trim().to_owned())
                        .unwrap_or_default();
                    let mem = fn_node
                        .child_by_field_name("name")
                        .map(|n| ctx.text(n).trim().to_owned())
                        .unwrap_or_default();
                    let mem_clean = mem.split('<').next().unwrap_or(&mem);
                    emit_semantic_from_call(
                        ctx,
                        Some(&obj),
                        mem_clean,
                        args_txt,
                        &str_args,
                        evidence,
                        owner_class.map(str::to_owned),
                        line,
                    );
                    ctx.push_ref(AstRef {
                        target_name: obj.clone(),
                        member: Some(mem_clean.to_owned()),
                        context: "call".to_owned(),
                        access: "call".to_owned(),
                        member_kind: Some("method".to_owned()),
                        owner_type: Some(obj),
                        enclosing_symbol: owner_class.map(str::to_owned),
                        is_static: false,
                        line,
                        evidence: clip_str(evidence, 160),
                    });
                } else {
                    let fn_txt = ctx.text(fn_node).trim();
                    emit_semantic_from_call(
                        ctx,
                        None,
                        fn_txt,
                        args_txt,
                        &str_args,
                        evidence,
                        owner_class.map(str::to_owned),
                        line,
                    );
                }
            }
        }
        _ => {}
    }
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_csharp(ctx, ch, depth + 1, owner_class, fn_depth);
        }
    }
}

fn parse_cs_modifiers(node: Node<'_>, source: &str, default_vis: &str) -> (String, Vec<String>) {
    let mut vis = default_vis.to_owned();
    let mut mods = Vec::new();
    for m in children_by_kind(node, "modifier") {
        let txt = node_text(m, source).trim();
        if matches!(txt, "public" | "private" | "protected" | "internal") {
            vis = txt.to_owned();
        } else {
            push_unique_mod(&mut mods, txt);
        }
    }
    (vis, mods)
}

fn emit_cs_attributes(ctx: &mut ExtractCtx<'_>, node: Node<'_>, sym: Option<&str>) {
    for al in children_by_kind(node, "attribute_list") {
        for attr in children_by_kind(al, "attribute") {
            let txt = ctx.text(attr);
            let name = attr
                .child_by_field_name("name")
                .map(|n| ctx.text(n))
                .unwrap_or(txt);
            let strs = all_string_arg_texts(attr, ctx.source);
            emit_semantic_from_attribute(ctx, name, txt, &strs, sym, &[], start_line(attr));
        }
    }
}

// =========================================================================
// 8. Ruby Extraction
// =========================================================================

fn extract_ruby(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    let mut vis = "public".to_owned();
    walk_ruby(ctx, root, 0, None, 0, &mut vis);
}

fn walk_ruby(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    owner_class: Option<&str>,
    fn_depth: usize,
    cur_vis: &mut String,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    match node.kind() {
        "class" | "module" => {
            let decl_kind = node.kind();
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    owner_class.map(str::to_owned),
                    line,
                    end_line(node),
                    Some("public".to_owned()),
                    Vec::new(),
                    sig.clone(),
                    sig,
                    String::new(),
                );
                if let Some(sc) = node.child_by_field_name("superclass") {
                    let base = ctx.text(sc).trim_start_matches('<').trim();
                    ctx.push_inheritance(&name, base, "extends", line, ctx.text(sc));
                }
                if let Some(body) = body_node {
                    let mut class_vis = "public".to_owned();
                    walk_ruby(ctx, body, depth + 1, Some(&name), fn_depth, &mut class_vis);
                }
            }
            return;
        }
        "method" | "singleton_method" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let mut mods = Vec::new();
            if node.kind() == "singleton_method" {
                mods.push("static".to_owned());
            }
            let decl_kind = if owner_class.is_some() {
                "method"
            } else {
                "function"
            };
            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    owner_class.map(str::to_owned),
                    start_line(node),
                    end_line(node),
                    Some(cur_vis.clone()),
                    mods,
                    sig.clone(),
                    sig,
                    body_node
                        .map(|b| ast_body_fingerprint(b, ctx.source))
                        .unwrap_or_default(),
                );
            }
            if let Some(body) = body_node {
                let mut inner_vis = cur_vis.clone();
                walk_ruby(
                    ctx,
                    body,
                    depth + 1,
                    owner_class,
                    fn_depth + 1,
                    &mut inner_vis,
                );
            }
            return;
        }
        "identifier" if owner_class.is_some() && fn_depth == 0 => {
            let txt = ctx.text(node).trim();
            if matches!(txt, "private" | "protected" | "public") {
                *cur_vis = txt.to_owned();
            }
        }
        "assignment" => {
            if let (Some(lhs), Some(rhs)) = (
                node.child_by_field_name("left"),
                node.child_by_field_name("right"),
            ) {
                let lhs_txt = ctx.text(lhs).trim();
                if fn_depth == 0 && lhs.kind() == "constant" {
                    ctx.push_decl(
                        "const",
                        lhs_txt,
                        owner_class.map(str::to_owned),
                        start_line(node),
                        end_line(node),
                        Some("public".to_owned()),
                        Vec::new(),
                        lhs_txt.to_owned(),
                        lhs_txt.to_owned(),
                        ast_body_fingerprint(rhs, ctx.source),
                    );
                } else if lhs_txt == "self.table_name"
                    && let Some(tbl) =
                        unquote_literal(ctx.text(rhs)).and_then(|s| clean_sql_table_name(&s))
                {
                    ctx.push_semantic(
                        "reads_table",
                        format!("table:{tbl}"),
                        owner_class.map(str::to_owned),
                        start_line(node),
                        ctx.text(node),
                    );
                }
            }
        }
        "call" => {
            let method_name = node
                .child_by_field_name("method")
                .map(|m| ctx.text(m).trim().to_owned())
                .unwrap_or_default();
            let recv = node
                .child_by_field_name("receiver")
                .map(|r| ctx.text(r).trim().to_owned());
            let args_node = node.child_by_field_name("arguments");
            let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
            let str_args = args_node
                .map(|a| all_string_arg_texts(a, ctx.source))
                .unwrap_or_default();
            let line = start_line(node);
            let evidence = ctx.text(node);

            if matches!(method_name.as_str(), "require" | "require_relative")
                && let Some(path) = str_args.first()
            {
                let stem = path.rsplit('/').next().unwrap_or(path).to_owned();
                ctx.push_import(AstImport {
                    path: path.clone(),
                    symbols: vec![stem],
                    alias: None,
                    is_wildcard: false,
                    is_reexport: false,
                    reexport_visibility: None,
                    kind: if method_name == "require_relative" {
                        "include"
                    } else {
                        "import"
                    }
                    .to_owned(),
                    line,
                    evidence: clip_str(evidence, 160),
                });
            } else if matches!(method_name.as_str(), "include" | "extend" | "prepend")
                && let Some(owner) = owner_class
                && let Some(args) = args_node
            {
                for part in ctx.text(args).trim_matches(['(', ')']).split(',') {
                    let mixin = part.trim();
                    if !mixin.is_empty() {
                        ctx.push_inheritance(owner, mixin, "uses", line, evidence);
                    }
                }
            } else {
                emit_semantic_from_call(
                    ctx,
                    recv.as_deref(),
                    &method_name,
                    args_txt,
                    &str_args,
                    evidence,
                    owner_class.map(str::to_owned),
                    line,
                );
                let target = recv.clone().unwrap_or_else(|| method_name.clone());
                ctx.push_ref(AstRef {
                    target_name: target,
                    member: recv.is_some().then_some(method_name),
                    context: "call".to_owned(),
                    access: "call".to_owned(),
                    member_kind: Some(
                        if recv.is_some() { "method" } else { "function" }.to_owned(),
                    ),
                    owner_type: recv,
                    enclosing_symbol: owner_class.map(str::to_owned),
                    is_static: false,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
        }
        _ => {}
    }
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_ruby(ctx, ch, depth + 1, owner_class, fn_depth, cur_vis);
        }
    }
}

// =========================================================================
// 9. Kotlin Extraction
// =========================================================================

fn extract_kotlin(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    walk_kotlin(ctx, root, 0, None, 0);
}

fn walk_kotlin(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    owner_class: Option<&str>,
    fn_depth: usize,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    match node.kind() {
        "package_header" => {
            let pkg = ctx
                .text(node)
                .trim_start_matches("package")
                .trim()
                .to_owned();
            if !pkg.is_empty() {
                ctx.namespace = Some(pkg);
            }
            return;
        }
        "import" => {
            let raw = ctx.text(node).trim_start_matches("import").trim();
            let (path_part, alias) = if let Some((l, r)) = raw.split_once(" as ") {
                (l.trim(), Some(r.trim().to_owned()))
            } else {
                (raw, None)
            };
            let is_wc = path_part.ends_with(".*");
            let clean = path_part.trim_end_matches(".*").to_owned();
            let sym = clean.rsplit('.').next().unwrap_or(&clean).to_owned();
            ctx.push_import(AstImport {
                path: clean,
                symbols: if is_wc || sym.is_empty() {
                    vec![]
                } else {
                    vec![sym]
                },
                alias,
                is_wildcard: is_wc,
                is_reexport: false,
                reexport_visibility: None,
                kind: "import".to_owned(),
                line: start_line(node),
                evidence: clip_str(ctx.text(node), 160),
            });
            return;
        }
        "class_declaration" | "object_declaration" => {
            let raw_txt = ctx.text(node);
            let decl_kind = if raw_txt.contains("interface ") {
                "interface"
            } else if raw_txt.contains("enum ") {
                "enum"
            } else {
                "class"
            };
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let body_node = child_by_kind(node, "class_body")
                .or_else(|| child_by_kind(node, "enum_class_body"));
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let (vis, mods) = parse_jvm_modifiers(node, ctx.source, "public");
            emit_kotlin_annotations(ctx, node, Some(&name));
            let line = start_line(node);
            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    owner_class.map(str::to_owned),
                    line,
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_leading_vis_word(&sig),
                    String::new(),
                );
                if let Some(deleg) = child_by_kind(node, "delegation_specifiers") {
                    for part in ctx.text(deleg).split(',') {
                        let clean = part.split(['(', '<']).next().unwrap_or("").trim();
                        let ik = if part.contains('(') {
                            "extends"
                        } else {
                            "implements"
                        };
                        ctx.push_inheritance(&name, clean, ik, line, ctx.text(deleg));
                    }
                }
                if let Some(pctor) = child_by_kind(node, "primary_constructor")
                    && let Some(cparams) = child_by_kind(pctor, "class_parameters")
                {
                    for cp in children_by_kind(cparams, "class_parameter") {
                        let cp_txt = ctx.text(cp);
                        if (cp_txt.contains("val ") || cp_txt.contains("var "))
                            && let Some(id) = child_by_kind(cp, "identifier")
                        {
                            let pname = ctx.text(id).trim().to_owned();
                            let (pvis, pmods) = parse_jvm_modifiers(cp, ctx.source, "public");
                            ctx.push_decl(
                                "property",
                                &pname,
                                Some(name.clone()),
                                start_line(cp),
                                end_line(cp),
                                Some(pvis),
                                pmods,
                                cp_txt.trim().to_owned(),
                                strip_leading_vis_word(cp_txt.trim()),
                                String::new(),
                            );
                        }
                    }
                }
                if let Some(body) = body_node {
                    walk_kotlin(ctx, body, depth + 1, Some(&name), fn_depth);
                }
            }
            return;
        }
        "function_declaration" => {
            if fn_depth > 0 {
                return;
            }
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let body_node = child_by_kind(node, "function_body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let (vis, mods) = parse_jvm_modifiers(node, ctx.source, "public");
            emit_kotlin_annotations(ctx, node, owner_class);
            if !name.is_empty() {
                ctx.push_decl(
                    if owner_class.is_some() {
                        "method"
                    } else {
                        "function"
                    },
                    &name,
                    owner_class.map(str::to_owned),
                    start_line(node),
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_leading_vis_word(&sig),
                    body_node
                        .map(|b| ast_body_fingerprint(b, ctx.source))
                        .unwrap_or_default(),
                );
            }
            if let Some(body) = body_node {
                walk_kotlin(ctx, body, depth + 1, owner_class, fn_depth + 1);
            }
            return;
        }
        "property_declaration" => {
            if fn_depth == 0
                && let Some(vd) = child_by_kind(node, "variable_declaration")
                && let Some(id) = child_by_kind(vd, "identifier")
            {
                let name = ctx.text(id).trim().to_owned();
                let (vis, mods) = parse_jvm_modifiers(node, ctx.source, "public");
                let is_const = mods.iter().any(|m| m == "const");
                let sig = header_without_comments(node, node.end_byte(), ctx.source)
                    .split('=')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                ctx.push_decl(
                    if is_const { "const" } else { "property" },
                    &name,
                    owner_class.map(str::to_owned),
                    start_line(node),
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_leading_vis_word(&sig),
                    String::new(),
                );
            }
        }
        "call_expression" => {
            let full = ctx.text(node);
            let line = start_line(node);
            let callee_part = full.split('(').next().unwrap_or("").trim();
            let (recv, mem) = match callee_part.rsplit_once('.') {
                Some((r, m)) => (Some(r.trim()), m.trim()),
                None => (None, callee_part),
            };
            let str_args = all_string_arg_texts(node, ctx.source);
            let args_txt = full.split_once('(').map_or("", |(_, r)| r);
            emit_semantic_from_call(
                ctx,
                recv,
                mem,
                args_txt,
                &str_args,
                full,
                owner_class.map(str::to_owned),
                line,
            );
            ctx.push_ref(AstRef {
                target_name: recv.unwrap_or(mem).to_owned(),
                member: recv.is_some().then(|| mem.to_owned()),
                context: "call".to_owned(),
                access: "call".to_owned(),
                member_kind: Some(if recv.is_some() { "method" } else { "function" }.to_owned()),
                owner_type: recv.map(str::to_owned),
                enclosing_symbol: owner_class.map(str::to_owned),
                is_static: false,
                line,
                evidence: clip_str(full, 160),
            });
        }
        _ => {}
    }
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_kotlin(ctx, ch, depth + 1, owner_class, fn_depth);
        }
    }
}

fn emit_kotlin_annotations(ctx: &mut ExtractCtx<'_>, node: Node<'_>, sym: Option<&str>) {
    if let Some(mnode) = child_by_kind(node, "modifiers") {
        for ann in children_by_kind(mnode, "annotation") {
            let txt = ctx.text(ann);
            let name = txt
                .trim_start_matches('@')
                .split('(')
                .next()
                .unwrap_or("")
                .trim();
            let strs = all_string_arg_texts(ann, ctx.source);
            emit_semantic_from_attribute(ctx, name, txt, &strs, sym, &[], start_line(ann));
        }
    }
}

// =========================================================================
// 10. Swift Extraction
// =========================================================================

fn extract_swift(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    walk_swift(ctx, root, 0, None, 0);
}

fn walk_swift(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    owner_class: Option<&str>,
    fn_depth: usize,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    match node.kind() {
        "import_declaration" => {
            let path = ctx
                .text(node)
                .trim_start_matches("import")
                .trim()
                .to_owned();
            let sym = path.rsplit('.').next().unwrap_or(&path).to_owned();
            ctx.push_import(AstImport {
                path,
                symbols: vec![sym],
                alias: None,
                is_wildcard: false,
                is_reexport: false,
                reexport_visibility: None,
                kind: "import".to_owned(),
                line: start_line(node),
                evidence: clip_str(ctx.text(node), 160),
            });
            return;
        }
        "class_declaration" | "protocol_declaration" => {
            let decl_kind = if node.kind() == "protocol_declaration" {
                "protocol"
            } else {
                match node
                    .child_by_field_name("declaration_kind")
                    .map(|k| ctx.text(k).trim())
                {
                    Some("struct") => "struct",
                    Some("enum") => "enum",
                    _ => "class",
                }
            };
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let line = start_line(node);
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let (vis, mods) = parse_swift_modifiers(node, ctx.source);
            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    owner_class.map(str::to_owned),
                    line,
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_swift_vis(&sig),
                    String::new(),
                );
                for inh in children_by_kind(node, "inheritance_specifier") {
                    let parent = ctx.text(inh).split('<').next().unwrap_or("").trim();
                    ctx.push_inheritance(&name, parent, "conforms", line, ctx.text(inh));
                }
                if let Some(body) = body_node {
                    walk_swift(ctx, body, depth + 1, Some(&name), fn_depth);
                }
            }
            return;
        }
        "function_declaration" | "protocol_function_declaration" | "init_declaration" => {
            if fn_depth > 0 {
                return;
            }
            let name = if node.kind() == "init_declaration" {
                "init".to_owned()
            } else {
                node.child_by_field_name("name")
                    .map(|n| ctx.text(n).trim().to_owned())
                    .unwrap_or_default()
            };
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let (vis, mods) = parse_swift_modifiers(node, ctx.source);
            if !name.is_empty() {
                ctx.push_decl(
                    if owner_class.is_some() {
                        "method"
                    } else {
                        "function"
                    },
                    &name,
                    owner_class.map(str::to_owned),
                    start_line(node),
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_swift_vis(&sig),
                    body_node
                        .map(|b| ast_body_fingerprint(b, ctx.source))
                        .unwrap_or_default(),
                );
            }
            if let Some(body) = body_node {
                walk_swift(ctx, body, depth + 1, owner_class, fn_depth + 1);
            }
            return;
        }
        "property_declaration" | "protocol_property_declaration" => {
            if fn_depth == 0
                && let Some(n) = node.child_by_field_name("name")
            {
                let name = ctx.text(n).trim().to_owned();
                let (vis, mods) = parse_swift_modifiers(node, ctx.source);
                let sig = header_without_comments(
                    node,
                    node.child_by_field_name("computed_value")
                        .map_or(node.end_byte(), |c| c.start_byte()),
                    ctx.source,
                )
                .split('=')
                .next()
                .unwrap_or("")
                .trim()
                .to_owned();
                ctx.push_decl(
                    "property",
                    &name,
                    owner_class.map(str::to_owned),
                    start_line(node),
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_swift_vis(&sig),
                    String::new(),
                );
            }
            return;
        }
        "call_expression" => {
            let full = ctx.text(node);
            let line = start_line(node);
            let callee_part = full.split('(').next().unwrap_or("").trim();
            let (recv, mem) = match callee_part.rsplit_once('.') {
                Some((r, m)) => (Some(r.trim()), m.trim()),
                None => (None, callee_part),
            };
            let str_args = all_string_arg_texts(node, ctx.source);
            let args_txt = full.split_once('(').map_or("", |(_, r)| r);
            emit_semantic_from_call(
                ctx,
                recv,
                mem,
                args_txt,
                &str_args,
                full,
                owner_class.map(str::to_owned),
                line,
            );
            ctx.push_ref(AstRef {
                target_name: recv.unwrap_or(mem).to_owned(),
                member: recv.is_some().then(|| mem.to_owned()),
                context: "call".to_owned(),
                access: "call".to_owned(),
                member_kind: Some(if recv.is_some() { "method" } else { "function" }.to_owned()),
                owner_type: recv.map(str::to_owned),
                enclosing_symbol: owner_class.map(str::to_owned),
                is_static: false,
                line,
                evidence: clip_str(full, 160),
            });
        }
        _ => {}
    }
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_swift(ctx, ch, depth + 1, owner_class, fn_depth);
        }
    }
}

fn parse_swift_modifiers(node: Node<'_>, source: &str) -> (String, Vec<String>) {
    let mut vis = "internal".to_owned();
    let mut mods = Vec::new();
    if let Some(mnode) = child_by_kind(node, "modifiers") {
        for word in node_text(mnode, source).split_whitespace() {
            if matches!(
                word,
                "open" | "public" | "package" | "internal" | "fileprivate" | "private"
            ) {
                vis = if word == "fileprivate" {
                    "private".to_owned()
                } else if word == "open" {
                    "public".to_owned()
                } else {
                    word.to_owned()
                };
            } else {
                push_unique_mod(&mut mods, word);
            }
        }
    }
    (vis, mods)
}

fn strip_swift_vis(sig: &str) -> String {
    let mut parts = Vec::new();
    let mut skip = true;
    for tok in sig.split_whitespace() {
        if skip
            && matches!(
                tok,
                "open" | "public" | "package" | "internal" | "fileprivate" | "private"
            )
        {
            continue;
        }
        skip = false;
        parts.push(tok);
    }
    parts.join(" ")
}

// =========================================================================
// 11. Scala Extraction
// =========================================================================

fn extract_scala(ctx: &mut ExtractCtx<'_>, root: Node<'_>) {
    walk_scala(ctx, root, 0, None, 0);
}

fn walk_scala(
    ctx: &mut ExtractCtx<'_>,
    node: Node<'_>,
    depth: usize,
    owner_class: Option<&str>,
    fn_depth: usize,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    match node.kind() {
        "package_clause" => {
            if let Some(name) = node.child_by_field_name("name") {
                ctx.namespace = Some(ctx.text(name).trim().to_owned());
            }
        }
        "import_declaration" | "export_declaration" => {
            let is_reexport = node.kind() == "export_declaration";
            let raw = ctx
                .text(node)
                .trim_start_matches("import")
                .trim_start_matches("export")
                .trim()
                .trim_end_matches(';');
            let is_wc = raw.ends_with("._") || raw.ends_with(".*");
            let clean = raw
                .trim_end_matches("._")
                .trim_end_matches(".*")
                .split('{')
                .next()
                .unwrap_or(raw)
                .trim_end_matches('.');
            let syms: Vec<String> = if let Some((_, inside)) = raw.split_once('{') {
                inside
                    .trim_end_matches('}')
                    .split(',')
                    .map(|s| s.split("=>").next().unwrap_or("").trim().to_owned())
                    .filter(|s| !s.is_empty() && s != "_" && s != "*")
                    .collect()
            } else if !is_wc {
                clean
                    .rsplit('.')
                    .next()
                    .map(|s| vec![s.to_owned()])
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            ctx.push_import(AstImport {
                path: clean.to_owned(),
                symbols: syms,
                alias: None,
                is_wildcard: is_wc,
                is_reexport,
                reexport_visibility: is_reexport.then(|| "public".to_owned()),
                kind: if is_reexport { "reexport" } else { "import" }.to_owned(),
                line: start_line(node),
                evidence: clip_str(ctx.text(node), 160),
            });
            return;
        }
        "class_definition" | "trait_definition" | "object_definition" | "enum_definition" => {
            let decl_kind = match node.kind() {
                "trait_definition" => "trait",
                "enum_definition" => "enum",
                _ => "class",
            };
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let (vis, mods) = parse_scala_modifiers(node, ctx.source);
            let line = start_line(node);
            if !name.is_empty() {
                ctx.push_decl(
                    decl_kind,
                    &name,
                    owner_class.map(str::to_owned),
                    line,
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_leading_vis_word(&sig),
                    String::new(),
                );
                if let Some(ext) = node.child_by_field_name("extend") {
                    let ext_txt = ctx.text(ext).trim_start_matches("extends").trim();
                    for (idx, part) in ext_txt.split(" with ").enumerate() {
                        let base = part.split(['(', '[']).next().unwrap_or("").trim();
                        let ik = if idx == 0 { "extends" } else { "implements" };
                        ctx.push_inheritance(&name, base, ik, line, ctx.text(ext));
                    }
                }
                if let Some(body) = body_node {
                    walk_scala(ctx, body, depth + 1, Some(&name), fn_depth);
                }
            }
            return;
        }
        "function_definition" | "function_declaration" => {
            if fn_depth > 0 {
                return;
            }
            let name = node
                .child_by_field_name("name")
                .map(|n| ctx.text(n).trim().to_owned())
                .unwrap_or_default();
            let body_node = node.child_by_field_name("body");
            let sig_end = body_node.map_or(node.end_byte(), |b| b.start_byte());
            let sig = header_without_comments(node, sig_end, ctx.source);
            let (vis, mods) = parse_scala_modifiers(node, ctx.source);
            if !name.is_empty() {
                ctx.push_decl(
                    if owner_class.is_some() {
                        "method"
                    } else {
                        "function"
                    },
                    &name,
                    owner_class.map(str::to_owned),
                    start_line(node),
                    end_line(node),
                    Some(vis),
                    mods,
                    sig.clone(),
                    strip_leading_vis_word(&sig),
                    body_node
                        .map(|b| ast_body_fingerprint(b, ctx.source))
                        .unwrap_or_default(),
                );
            }
            if let Some(body) = body_node {
                walk_scala(ctx, body, depth + 1, owner_class, fn_depth + 1);
            }
            return;
        }
        "val_definition" | "val_declaration" | "var_definition" | "var_declaration" => {
            if fn_depth == 0 {
                let name = node
                    .child_by_field_name("pattern")
                    .or_else(|| node.child_by_field_name("name"))
                    .map(|n| ctx.text(n).trim().to_owned())
                    .unwrap_or_default();
                let (vis, mods) = parse_scala_modifiers(node, ctx.source);
                let val_node = node.child_by_field_name("value");
                let sig = header_without_comments(
                    node,
                    val_node.map_or(node.end_byte(), |v| v.start_byte()),
                    ctx.source,
                )
                .trim_end_matches('=')
                .trim()
                .to_owned();
                if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    ctx.push_decl(
                        if is_upper_snake(&name) {
                            "const"
                        } else {
                            "property"
                        },
                        &name,
                        owner_class.map(str::to_owned),
                        start_line(node),
                        end_line(node),
                        Some(vis),
                        mods,
                        sig.clone(),
                        strip_leading_vis_word(&sig),
                        val_node
                            .map(|v| ast_body_fingerprint(v, ctx.source))
                            .unwrap_or_default(),
                    );
                }
            }
            return;
        }
        "call_expression" => {
            if let Some(fn_node) = node.child_by_field_name("function") {
                let fn_txt = ctx.text(fn_node).trim();
                let line = start_line(node);
                let evidence = ctx.text(node);
                let args_node = node.child_by_field_name("arguments");
                let args_txt = args_node.map(|a| ctx.text(a)).unwrap_or("");
                let str_args = args_node
                    .map(|a| all_string_arg_texts(a, ctx.source))
                    .unwrap_or_default();
                let (recv, mem) = match fn_txt.rsplit_once('.') {
                    Some((r, m)) => (Some(r.trim()), m.trim()),
                    None => (None, fn_txt),
                };
                emit_semantic_from_call(
                    ctx,
                    recv,
                    mem,
                    args_txt,
                    &str_args,
                    evidence,
                    owner_class.map(str::to_owned),
                    line,
                );
                ctx.push_ref(AstRef {
                    target_name: recv.unwrap_or(mem).to_owned(),
                    member: recv.is_some().then(|| mem.to_owned()),
                    context: "call".to_owned(),
                    access: "call".to_owned(),
                    member_kind: Some(
                        if recv.is_some() { "method" } else { "function" }.to_owned(),
                    ),
                    owner_type: recv.map(str::to_owned),
                    enclosing_symbol: owner_class.map(str::to_owned),
                    is_static: false,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
        }
        _ => {}
    }
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            walk_scala(ctx, ch, depth + 1, owner_class, fn_depth);
        }
    }
}

fn parse_scala_modifiers(node: Node<'_>, source: &str) -> (String, Vec<String>) {
    let mut vis = "public".to_owned();
    let mut mods = Vec::new();
    if let Some(acc) = child_by_kind(node, "access_modifier") {
        let txt = node_text(acc, source).trim();
        vis = if txt.starts_with("private") {
            "private".to_owned()
        } else if txt.starts_with("protected") {
            "protected".to_owned()
        } else {
            txt.to_owned()
        };
    }
    if let Some(mnode) = child_by_kind(node, "modifiers") {
        for word in node_text(mnode, source).split_whitespace() {
            if word.starts_with("private") {
                vis = "private".to_owned();
            } else if word.starts_with("protected") {
                vis = "protected".to_owned();
            } else {
                push_unique_mod(&mut mods, word);
            }
        }
    }
    (vis, mods)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_fourteen_plus_languages_without_panicking() {
        let samples = [
            (
                "src/App.php",
                "<?php\nnamespace App;\nclass App { public function run(): void {} }\n",
            ),
            (
                "src/lib.rs",
                "pub struct Core { pub x: i32 }\npub fn boot() {}\n",
            ),
            ("src/index.ts", "export class Client { fetch(): void {} }\n"),
            (
                "src/Button.tsx",
                "export const Button = () => <button>Ok</button>;\n",
            ),
            (
                "src/util.js",
                "export function add(a, b) { return a + b; }\n",
            ),
            (
                "src/Card.jsx",
                "export function Card() { return <div />; }\n",
            ),
            (
                "app/main.py",
                "class Svc:\n    def handle(self) -> int:\n        return 1\n",
            ),
            (
                "cmd/srv.go",
                "package main\ntype Server struct { Port int }\nfunc (s *Server) Start() {}\n",
            ),
            (
                "src/Main.java",
                "package acme;\npublic class Main { public void run() {} }\n",
            ),
            (
                "src/Auth.kt",
                "package acme\nclass Auth(val key: String) { fun verify() = true }\n",
            ),
            (
                "src/Svc.cs",
                "namespace Acme;\npublic class Svc { public int Count { get; set; } public void Go() {} }\n",
            ),
            (
                "app/user.rb",
                "class User < ActiveRecord::Base\n  def save\n  end\nend\n",
            ),
            (
                "Sources/App.swift",
                "import Foundation\npublic struct Config { public let port: Int }\n",
            ),
            (
                "src/Main.scala",
                "package acme\nclass Main extends Base { def run(): Unit = () }\n",
            ),
        ];
        for (path, src) in samples {
            let ir = parse_file_ir(path, src).unwrap_or_else(|| panic!("failed to parse {path}"));
            assert!(
                !ir.declarations.is_empty(),
                "expected declarations for {path}"
            );
        }
    }

    #[test]
    fn extracts_framework_routes_di_events_and_sql_semantics() {
        let php = r#"<?php
namespace App\Http;
use Symfony\Component\Routing\Annotation\Route;
use Symfony\Component\Messenger\Attribute\AsMessageHandler;

#[Route('/api/orders', methods: ['POST'])]
class OrderController {
    public function store(): void {
        Route::get('/api/orders/health', fn () => 'ok');
        $this->app->bind(OrderRepoInterface::class, SqlOrderRepo::class);
        DB::table('orders')->insert(['total' => 10]);
        $rows = DB::select('SELECT id FROM customers JOIN invoices ON invoices.cust_id = customers.id');
        OrderPlaced::dispatch();
    }
}

#[AsMessageHandler]
class OrderPlacedHandler {
    public function __invoke(OrderPlaced $event): void {}
}
"#;
        let ir = parse_file_ir("src/Http/OrderController.php", php).expect("php ir");
        assert!(
            ir.semantic_edges
                .iter()
                .any(|e| e.kind == "exposes_route" && e.target.contains("/api/orders")),
            "{:?}",
            ir.semantic_edges
        );
        assert!(
            ir.semantic_edges
                .iter()
                .any(|e| e.kind == "exposes_route" && e.target == "route:GET /api/orders/health"),
            "{:?}",
            ir.semantic_edges
        );
        assert!(
            ir.semantic_edges
                .iter()
                .any(|e| e.kind == "di_binds" && e.target == "service:OrderRepoInterface"),
            "{:?}",
            ir.semantic_edges
        );
        assert!(
            ir.semantic_edges
                .iter()
                .any(|e| e.kind == "writes_table" && e.target == "table:orders"),
            "{:?}",
            ir.semantic_edges
        );
        assert!(
            ir.semantic_edges
                .iter()
                .any(|e| e.kind == "reads_table" && e.target == "table:customers"),
            "{:?}",
            ir.semantic_edges
        );
        assert!(
            ir.semantic_edges
                .iter()
                .any(|e| e.kind == "reads_table" && e.target == "table:invoices"),
            "{:?}",
            ir.semantic_edges
        );
        assert!(
            ir.semantic_edges
                .iter()
                .any(|e| e.kind == "publishes_event" && e.target == "event:OrderPlaced"),
            "{:?}",
            ir.semantic_edges
        );
        assert!(
            ir.semantic_edges
                .iter()
                .any(|e| e.kind == "handles_event" && e.target == "event:OrderPlaced"),
            "{:?}",
            ir.semantic_edges
        );
    }

    #[test]
    fn handles_syntax_errors_truncated_inputs_and_oversized_inputs_gracefully() {
        let broken_rs = "pub fn broken(x: i32 { let s = \"unterminated;\n";
        let ir = parse_file_ir("src/broken.rs", broken_rs).expect("partial tree");
        assert_eq!(ir.language, "rust");

        let huge = "x".repeat(MAX_AST_BYTES + 1);
        assert!(parse_file_ir("src/huge.rs", &huge).is_none());

        let deep = format!("fn f() {{ {} }}", "(".repeat(400) + &")".repeat(400));
        assert!(parse_file_ir("src/deep.rs", &deep).is_some());
    }

    #[test]
    fn notify_event_filter_keeps_only_code_paths() {
        let ev = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![
                PathBuf::from("src/app.rs"),
                PathBuf::from("README.md"),
                PathBuf::from("cmd/server.go"),
            ],
            attrs: Default::default(),
        };
        assert_eq!(
            notify_event_touches_code(&ev),
            vec![PathBuf::from("src/app.rs"), PathBuf::from("cmd/server.go")]
        );
    }

    #[test]
    fn extracts_cross_ecosystem_routes_di_events_and_tables() {
        let java = r#"
@Service
public class BillingService {
    @GetMapping("/v1/bills")
    public void list() {}

    @EventListener
    public void onPaid(PaymentSettled evt) {}
}
"#;
        let java_ir = parse_file_ir("src/BillingService.java", java).expect("java ir");
        assert!(
            java_ir
                .semantic_edges
                .iter()
                .any(|e| e.kind == "exposes_route" && e.target == "route:GET /v1/bills"),
            "{:?}",
            java_ir.semantic_edges
        );
        assert!(
            java_ir
                .semantic_edges
                .iter()
                .any(|e| e.kind == "di_binds" && e.target == "service:BillingService"),
            "{:?}",
            java_ir.semantic_edges
        );
        assert!(
            java_ir
                .semantic_edges
                .iter()
                .any(|e| e.kind == "handles_event" && e.target == "event:PaymentSettled"),
            "{:?}",
            java_ir.semantic_edges
        );

        let ts = r#"
@Injectable()
export class GatewayClient {}
app.get('/v1/health', handler);
bus.emit('InvoiceIssued', payload);
"#;
        let ts_ir = parse_file_ir("src/gateway.ts", ts).expect("ts ir");
        assert!(
            ts_ir
                .semantic_edges
                .iter()
                .any(|e| e.kind == "di_binds" && e.target == "service:GatewayClient"),
            "{:?}",
            ts_ir.semantic_edges
        );
        assert!(
            ts_ir
                .semantic_edges
                .iter()
                .any(|e| e.kind == "exposes_route" && e.target == "route:GET /v1/health"),
            "{:?}",
            ts_ir.semantic_edges
        );
        assert!(
            ts_ir
                .semantic_edges
                .iter()
                .any(|e| e.kind == "publishes_event" && e.target == "event:InvoiceIssued"),
            "{:?}",
            ts_ir.semantic_edges
        );

        let py = r#"
@app.get("/py/items")
def list_items():
    return []
"#;
        let py_ir = parse_file_ir("api/items.py", py).expect("py ir");
        assert!(
            py_ir
                .semantic_edges
                .iter()
                .any(|e| e.kind == "exposes_route" && e.target == "route:GET /py/items"),
            "{:?}",
            py_ir.semantic_edges
        );

        let rs = r#"
diesel::table! {
    ledger_entries (id) {
        id -> Int4,
    }
}
fn router() {
    Router::new().route("/rust/ping", get(ping));
}
"#;
        let rs_ir = parse_file_ir("src/routes.rs", rs).expect("rs ir");
        assert!(
            rs_ir
                .semantic_edges
                .iter()
                .any(|e| e.kind == "reads_table" && e.target == "table:ledger_entries"),
            "{:?}",
            rs_ir.semantic_edges
        );
        assert!(
            rs_ir
                .semantic_edges
                .iter()
                .any(|e| e.kind == "exposes_route" && e.target == "route:/rust/ping"),
            "{:?}",
            rs_ir.semantic_edges
        );

        let go = r#"
package web
func register(mux *ServeMux) {
    mux.HandleFunc("/go/ready", okHandler)
}
"#;
        let go_ir = parse_file_ir("web/routes.go", go).expect("go ir");
        assert!(
            go_ir
                .semantic_edges
                .iter()
                .any(|e| e.kind == "exposes_route" && e.target == "route:/go/ready"),
            "{:?}",
            go_ir.semantic_edges
        );

        let cs = r#"
public class Startup {
    public void Configure(IServiceCollection services) {
        services.AddSingleton<ITokenCache, RedisTokenCache>();
    }
}
"#;
        let cs_ir = parse_file_ir("Startup.cs", cs).expect("cs ir");
        assert!(
            cs_ir
                .semantic_edges
                .iter()
                .any(|e| e.kind == "di_binds" && e.target == "service:ITokenCache"),
            "{:?}",
            cs_ir.semantic_edges
        );

        let php = r#"<?php
#[ORM\Table(name: 'audit_logs')]
class AuditLog {}
"#;
        let php_ir = parse_file_ir("src/Entity/AuditLog.php", php).expect("php ir");
        assert!(
            php_ir
                .semantic_edges
                .iter()
                .any(|e| e.kind == "reads_table" && e.target == "table:audit_logs"),
            "{:?}",
            php_ir.semantic_edges
        );
    }

    #[test]
    fn notify_event_filter_includes_manifests_and_architecture_config() {
        let ev = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![
                PathBuf::from("Cargo.toml"),
                PathBuf::from("package.json"),
                PathBuf::from("tsconfig.json"),
                PathBuf::from("composer.json"),
                PathBuf::from("pyproject.toml"),
                PathBuf::from("go.mod"),
                PathBuf::from(".harness-architecture.json"),
                PathBuf::from("docs/notes.md"),
            ],
            attrs: Default::default(),
        };
        assert_eq!(
            notify_event_touches_code(&ev),
            vec![
                PathBuf::from("Cargo.toml"),
                PathBuf::from("package.json"),
                PathBuf::from("tsconfig.json"),
                PathBuf::from("composer.json"),
                PathBuf::from("pyproject.toml"),
                PathBuf::from("go.mod"),
                PathBuf::from(".harness-architecture.json"),
            ]
        );
    }

    #[test]
    fn preserves_di_concrete_targets_calls_route_and_rust_test_spans() {
        let php = r#"<?php
class AppProvider {
    public function register(): void {
        $this->app->bind(OrderRepoInterface::class, SqlOrderRepo::class);
        Http::post('https://billing.internal/v1/charge', ['id' => 1]);
    }
}
"#;
        let php_ir = parse_file_ir("src/Providers/AppProvider.php", php).expect("php ir");
        assert!(
            php_ir
                .semantic_edges
                .iter()
                .any(|e| e.kind == "di_binds" && e.target == "service:OrderRepoInterface"),
            "{:?}",
            php_ir.semantic_edges
        );
        assert!(
            php_ir
                .semantic_edges
                .iter()
                .any(|e| e.kind == "di_binds" && e.target == "service:SqlOrderRepo"),
            "{:?}",
            php_ir.semantic_edges
        );
        assert!(
            php_ir
                .semantic_edges
                .iter()
                .any(|e| e.kind == "calls_route" && e.target == "route:POST /v1/charge"),
            "{:?}",
            php_ir.semantic_edges
        );

        let cs = r#"
public class Startup {
    public void Configure(IServiceCollection services) {
        services.AddSingleton<ITokenCache, RedisTokenCache>();
    }
}
"#;
        let cs_ir = parse_file_ir("Startup.cs", cs).expect("cs ir");
        assert!(
            cs_ir
                .semantic_edges
                .iter()
                .any(|e| e.kind == "di_binds" && e.target == "service:RedisTokenCache"),
            "{:?}",
            cs_ir.semantic_edges
        );

        let ts = r#"
export async function loadBills() {
    await fetch('/v1/bills');
    return axios.post('/v1/bills/pay', {});
}
"#;
        let ts_ir = parse_file_ir("src/client.ts", ts).expect("ts ir");
        assert!(
            ts_ir
                .semantic_edges
                .iter()
                .any(|e| e.kind == "calls_route" && e.target == "route:GET /v1/bills"),
            "{:?}",
            ts_ir.semantic_edges
        );
        assert!(
            ts_ir
                .semantic_edges
                .iter()
                .any(|e| e.kind == "calls_route" && e.target == "route:POST /v1/bills/pay"),
            "{:?}",
            ts_ir.semantic_edges
        );

        let rs = r#"
pub fn helper() {}

#[cfg(test)]
mod tests {
    #[test]
    fn works() {
        super::helper();
    }
}
"#;
        let rs_ir = parse_file_ir("src/lib.rs", rs).expect("rs ir");
        assert!(
            !rs_ir.test_spans.is_empty(),
            "expected #[cfg(test)] span in {:?}",
            rs_ir.declarations
        );
    }

    #[test]
    fn watcher_dir_events_sql_cte_joins_and_blade_exclusion_work() {
        use notify::{
            Event, EventKind,
            event::{CreateKind, ModifyKind, RemoveKind, RenameMode},
        };

        assert!(AstGrammar::from_rel_path("resources/views/home.blade.php").is_none());

        let ev_dir_rm = Event::new(EventKind::Remove(RemoveKind::Folder))
            .add_path(PathBuf::from("/proj/src/legacy"))
            .add_path(PathBuf::from("/proj/target/debug"));
        let kept_rm = notify_event_touches_code(&ev_dir_rm);
        assert_eq!(kept_rm, vec![PathBuf::from("/proj/src/legacy")]);

        let ev_dir_rename = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
            .add_path(PathBuf::from("/proj/src/old_mod"))
            .add_path(PathBuf::from("/proj/src/new_mod"))
            .add_path(PathBuf::from("/proj/node_modules/pkg"));
        let kept_mv = notify_event_touches_code(&ev_dir_rename);
        assert_eq!(
            kept_mv,
            vec![
                PathBuf::from("/proj/src/old_mod"),
                PathBuf::from("/proj/src/new_mod"),
            ]
        );

        let ev_dir_add = Event::new(EventKind::Create(CreateKind::Folder))
            .add_path(PathBuf::from("/proj/src/feature"));
        assert_eq!(
            notify_event_touches_code(&ev_dir_add),
            vec![PathBuf::from("/proj/src/feature")]
        );

        let php_sql = r#"<?php
class ReportRepo {
    public function run($db) {
        return $db->query("WITH recent_orders AS (SELECT * FROM invoices) SELECT o.id FROM recent_orders, payments p JOIN customers c ON c.id = o.customer_id; MERGE INTO audit_log t USING invoices s ON t.id = s.id WHEN MATCHED THEN UPDATE SET n = 1; TRUNCATE TABLE temp_queue");
    }
}
"#;
        let ir = parse_file_ir("src/ReportRepo.php", php_sql).expect("php sql ir");
        let reads: Vec<&str> = ir
            .semantic_edges
            .iter()
            .filter(|e| e.kind == "reads_table")
            .map(|e| e.target.as_str())
            .collect();
        let writes: Vec<&str> = ir
            .semantic_edges
            .iter()
            .filter(|e| e.kind == "writes_table")
            .map(|e| e.target.as_str())
            .collect();
        assert!(reads.contains(&"table:invoices"), "{reads:?}");
        assert!(reads.contains(&"table:payments"), "{reads:?}");
        assert!(reads.contains(&"table:customers"), "{reads:?}");
        assert!(
            !reads.contains(&"table:recent_orders"),
            "CTE alias should not emit table read: {reads:?}"
        );
        assert!(writes.contains(&"table:audit_log"), "{writes:?}");
        assert!(writes.contains(&"table:temp_queue"), "{writes:?}");
    }
}
