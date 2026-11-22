//! Dependency map of a published code graph: which folders and files use
//! which.
//!
//! A Graphify graph of a real project has hundreds of thousands of symbol
//! nodes, far more than a picture can hold. The map therefore counts the
//! links that cross a file boundary, read in one streaming pass, and answers
//! for one folder at a time: its direct children and the links between them.
//! Of each symbol it keeps a name, a line and what holds it, so one file can
//! be opened in turn: its classes, methods and functions, and the files they
//! use or are used by.
//!
//! The graph is the tool's, and the tool matches names across the whole
//! project. Where the build says otherwise the build is believed: a link
//! from one Rust crate into a crate it is not built with is left out, as
//! `graph_crates` tells.
use crate::{
    HarnessError, Result,
    graph_ast::{AstFileExtraction, parse_file_ir},
    graph_crates::Crates,
    graph_deps::{
        DeclaredMember, DepMemberRef, find_matching_member_references,
        find_matching_member_references_from_ir,
    },
    graph_impact::{
        VisibilityWarning, caller_extends_target_in, enrich_member_ref_in,
        find_matching_declared_member, norm_member_name,
    },
};
use serde::de::{DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fmt,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};
mod access;
mod blast;
mod cache;
mod changes;
mod circles;
mod file_inspect;
mod follow;
mod history;
mod incoming;
pub(crate) mod layer_rules;
mod layers;
mod lod;
mod modules;
mod paths;
mod patterns;
mod runs;
mod scenario;
mod test_reach;
#[cfg(test)]
mod tests;
mod view;
pub use self::cache::Maps;
pub(crate) use self::cache::{attach_confidence_metadata, attach_ir_metadata};
use self::changes::*;
pub(crate) use self::changes::{
    FileMemberCache, PersistentMemberStore, TEST_SPANS, TestSpans, kept_tests, quiet_end,
};
#[cfg(test)]
use self::file_inspect::{Named, named};
use self::follow::*;
pub use self::lod::Lod;
pub(crate) use self::lod::{
    LodFilter, apply_lod_and_min_weight, finalize_telemetry, paginate_lens_view,
    structural_diff_overlay, view_at_lod,
};
use self::paths::*;
use self::view::*;
/// Children of one folder drawn at once; the rest are reached by the filter.
pub const SHOWN: usize = 40;
/// Links drawn between those children, strongest first.
pub const DRAWN: usize = 160;
pub(super) const LISTED: usize = 12;
/// By how many links a way between two places may be longer than the
/// shortest and still be drawn, how many such ways a picture draws, and how
/// many links the search for them tries before it gives up.
pub(crate) const LONGER: usize = 3;
/// How many failing tests a picture of a change may be told of.
pub(crate) const FAILING: usize = 64;
pub(super) const DETOURS: usize = 12;
pub(super) const DETOUR_WALK: usize = 60_000;
/// More links than this per child and the folder is drawn as a backbone:
/// each child keeps its `STRONGEST` uses.
pub(super) const DENSE: usize = 2;
pub const STRONGEST: usize = 3;
pub(super) const KEPT_LIVE: usize = 4;
pub(super) const KEPT_REVISIONS: usize = 8;
/// Changed files drawn one by one; more of them are drawn as the children of
/// the open folder, which can be opened in turn.
pub(super) const SINGLE: usize = 24;
/// Places kept for the files linked to a change, however large it is.
pub(super) const AROUND: usize = 8;
pub(super) const BYTE_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
/// Symbols of one file drawn at once; the rest are reached by the filter.
pub(super) const NAMED: usize = SHOWN - AROUND;
/// Symbols listed on each side of a selected one.
pub(super) const TOLD: usize = 30;
/// Places one unfolding of a side adds at most to the links followed from a
/// place, strongest first.
pub const STEP: usize = 12;
/// Places a picture of followed links holds.
pub const FOLLOWED: usize = 60;
/// Places of a folder that is spread out in the picture of the folder that
/// holds it, drawn at once: the most linked first.
pub const HELD: usize = 20;
/// Boxes a picture with spread-out folders holds, and how many of them are
/// left to what the open folder itself holds, however much is spread out.
pub(super) const SPREAD: usize = 100;
pub(super) const OWN: usize = 20;
pub(super) const WHOLE_SHOWN: usize = 160;
pub(super) const WHOLE_SPREAD: usize = 200;
pub(super) const WHOLE_HELD: usize = 24;
pub(super) const WHOLE_DRAWN: usize = 320;
pub(super) const WHOLE_FOLDERS: usize = 128;
/// Names each end of a link shows when the link is looked into, the most
/// linked first, and the links drawn between the two ends.
pub const LINKED: usize = 150;
pub(super) const WIRED: usize = 600;
/// Places one step of a path shows at most, the most linked first; a long
/// path shows fewer, so that the picture holds no more than `FOLLOWED`.
pub const WAY: usize = 8;
/// Places that two places share which one picture shows at most, the
/// nearest first: of what both use, or of what uses both. With the ways to
/// them and the two themselves a picture holds no more than `FOLLOWED`.
pub const SHARED: usize = 12;
pub(super) const SIDE: usize = FOLLOWED - 2;
/// Places a search of the whole graph lists at once, the best first; the
/// rest are counted, and a longer search reaches them.
pub const FOUND: usize = 40;
/// Words of one search that are read.
pub(super) const WORDS: usize = 8;
/// The largest source that is read: for the tests inside it, and for the
/// other sources it names.
pub(crate) const SOURCE_LIMIT: u64 = 4 * 1024 * 1024;
/// Bytes of a symbol's name that are kept.
pub(super) const NAME_LIMIT: usize = 160;
/// Relations that say where a symbol lives or why, not what it needs.
pub(super) const STRUCTURE: &[&str] = &["contains", "method", "defines", "rationale_for"];
/// No symbol: the top of a file.
pub(super) const NONE: u32 = u32::MAX;
/// A JSON scalar read as text; Graphify writes ids as strings, other writers
/// of the node-link format use numbers.
pub(super) struct Text(String);
impl<'de> serde::Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Scalar;
        impl Visitor<'_> for Scalar {
            type Value = Text;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string or a number")
            }
            fn visit_str<E>(self, value: &str) -> std::result::Result<Text, E> {
                Ok(Text(value.to_owned()))
            }
            fn visit_string<E>(self, value: String) -> std::result::Result<Text, E> {
                Ok(Text(value))
            }
            fn visit_u64<E>(self, value: u64) -> std::result::Result<Text, E> {
                Ok(Text(value.to_string()))
            }
            fn visit_i64<E>(self, value: i64) -> std::result::Result<Text, E> {
                Ok(Text(value.to_string()))
            }
            fn visit_f64<E>(self, value: f64) -> std::result::Result<Text, E> {
                Ok(Text(value.to_string()))
            }
            fn visit_bool<E>(self, value: bool) -> std::result::Result<Text, E> {
                Ok(Text(value.to_string()))
            }
        }
        deserializer.deserialize_any(Scalar)
    }
}
/// A JSON value read as text when it is a scalar and as nothing otherwise:
/// a field that only describes a symbol cannot fail the whole graph.
#[derive(Default)]
pub(super) struct Loose(Option<String>);
impl<'de> serde::Deserialize<'de> for Loose {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Any;
        impl<'de> Visitor<'de> for Any {
            type Value = Loose;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("any value")
            }
            fn visit_str<E>(self, value: &str) -> std::result::Result<Loose, E> {
                Ok(Loose(Some(value.to_owned())))
            }
            fn visit_string<E>(self, value: String) -> std::result::Result<Loose, E> {
                Ok(Loose(Some(value)))
            }
            fn visit_u64<E>(self, value: u64) -> std::result::Result<Loose, E> {
                Ok(Loose(Some(value.to_string())))
            }
            fn visit_i64<E>(self, value: i64) -> std::result::Result<Loose, E> {
                Ok(Loose(Some(value.to_string())))
            }
            fn visit_f64<E>(self, value: f64) -> std::result::Result<Loose, E> {
                Ok(Loose(Some(value.to_string())))
            }
            fn visit_bool<E>(self, value: bool) -> std::result::Result<Loose, E> {
                Ok(Loose(Some(value.to_string())))
            }
            fn visit_unit<E>(self) -> std::result::Result<Loose, E> {
                Ok(Loose(None))
            }
            fn visit_none<E>(self) -> std::result::Result<Loose, E> {
                Ok(Loose(None))
            }
            fn visit_some<S: Deserializer<'de>>(
                self,
                deserializer: S,
            ) -> std::result::Result<Loose, S::Error> {
                serde::Deserialize::deserialize(deserializer)
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Loose, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(Loose(None))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Loose, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(Loose(None))
            }
        }
        deserializer.deserialize_any(Any)
    }
}
#[derive(serde::Deserialize)]
pub(super) struct Node {
    pub(super) id: Text,
    #[serde(default)]
    pub(super) source_file: Option<Text>,
    #[serde(default)]
    pub(super) label: Loose,
    #[serde(default)]
    pub(super) source_location: Loose,
    #[serde(default)]
    pub(super) end_line: Loose,
    #[serde(default)]
    pub(super) file_type: Loose,
    #[serde(default)]
    pub(super) node_kind: Loose,
    #[serde(default)]
    pub(super) community: Loose,
    #[serde(default)]
    pub(super) community_name: Loose,
    #[serde(default, rename = "_callable_class")]
    pub(super) class: Loose,
    #[serde(default, rename = "abstract")]
    pub(super) is_abstract: bool,
    #[serde(default)]
    pub(super) is_test: bool,
}
/// What a symbol is, as far as the graph tells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// The node that stands for the file itself: its imports and the code
    /// outside any function.
    File,
    /// A class, an interface, a trait, a struct or an enum.
    Type,
    Method,
    Function,
    /// A case of an enum.
    Case,
    /// A heading of a document.
    Heading,
    /// Anything else with a name: a constant, a field, a key.
    Name,
    /// A comment the tool kept as the reason for something; never listed.
    Note,
}
impl Kind {
    pub(crate) fn word(self) -> &'static str {
        match self {
            Kind::File => "file",
            Kind::Type => "type",
            Kind::Method => "method",
            Kind::Function => "function",
            Kind::Case => "case",
            Kind::Heading => "heading",
            Kind::Name => "name",
            Kind::Note => "note",
        }
    }
}
/// A node of the graph that lies in a file. Its name is a run of `names`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Symbol {
    pub(crate) file: u32,
    pub(crate) community: Option<u32>,
    /// The symbol that holds it, or `NONE` at the top of the file.
    pub(crate) parent: u32,
    /// Where it starts, or 0 when the graph does not say.
    pub(crate) line: u32,
    /// Where it ends (inclusive), or 0 when the graph does not say.
    pub(crate) end_line: u32,
    pub(crate) at: u32,
    pub(crate) len: u16,
    pub(crate) kind: Kind,
    pub(crate) is_abstract: bool,
    pub(crate) is_test: bool,
    pub(crate) is_semantic: bool,
}
/// A link between two symbols: the one that uses, the one that is used, the
/// relation, and whether the tool only inferred it.
pub(crate) type Use = (u32, u32, u16, bool);
/// One run of lines a change left different in a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hunk {
    /// The first new line, or, where lines were only removed, the line they
    /// followed.
    pub at: u32,
    /// How many new lines stand there.
    pub added: u32,
    /// How many old lines they took the place of.
    pub removed: u32,
    /// The removed lines held nothing but blanks.
    pub blank: bool,
}
/// What a change did to one file, to tell which of its symbols it touched.
#[derive(Clone, Debug, Default)]
pub struct Edited {
    /// The whole file is new, and so is everything in it.
    pub new: bool,
    /// The file as the change leaves it.
    pub text: String,
    /// The lines that differ, as lines of that text.
    pub hunks: Vec<Hunk>,
}
/// A line of a file, as far as telling one symbol's lines from the next
/// one's needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Row {
    Blank,
    /// A comment, an attribute or a decorator: what stands above a symbol
    /// and belongs to it.
    Comment,
    /// A line that goes on with what was begun above at its own depth: a
    /// closing bracket, an opening brace, `end`, `where`.
    Closer,
    Code,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Line {
    pub(super) indent: u32,
    pub(super) kind: Row,
    /// The line ends a statement or a block, so nothing of it follows.
    pub(super) whole: bool,
}
/// Words that go on with a definition at the definition's own depth.
pub(super) const CONTINUES: &[&str] = &[
    "end", "else", "elif", "elsif", "elseif", "rescue", "ensure", "except", "finally", "catch",
    "where", "begin", "then", "do",
];
pub(super) fn lines(text: &str) -> Vec<Line> {
    text.lines()
        .map(|line| {
            let mut indent = 0u32;
            let mut rest = "";
            for (at, letter) in line.char_indices() {
                match letter {
                    ' ' => indent += 1,
                    '\t' => indent = (indent / 4 + 1) * 4,
                    _ => {
                        rest = &line[at..];
                        break;
                    }
                }
            }
            let rest = rest.trim_end();
            let word = &rest[..rest
                .bytes()
                .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                .count()];
            let kind = if rest.is_empty() {
                Row::Blank
            } else if ["//", "/*", "*/", "#", "@", "--", "<!--"]
                .iter()
                .any(|mark| rest.starts_with(mark))
                || rest == "*"
                || rest.starts_with("* ")
            {
                Row::Comment
            } else if rest.starts_with(['}', ')', ']', '{']) || CONTINUES.contains(&word) {
                Row::Closer
            } else {
                Row::Code
            };
            Line {
                indent,
                kind,
                whole: rest.ends_with([';', '}', ')']),
            }
        })
        .collect()
}
/// The start of a text, cut where a character ends.
pub(super) fn clip(text: &str, limit: usize) -> &str {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
/// The line of `L42`, as the tool writes a place in a file.
pub(super) fn line(location: Option<&str>) -> u32 {
    let text = location.unwrap_or("").trim_start_matches(['L', 'l']);
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    text[..digits].parse().unwrap_or(0)
}
#[derive(serde::Deserialize)]
pub(super) struct Link {
    pub(super) source: Text,
    pub(super) target: Text,
    #[serde(default)]
    pub(super) relation: Option<Text>,
    #[serde(default)]
    pub(super) confidence: Option<Text>,
    #[serde(default)]
    pub(super) source_file: Option<Text>,
    /// Where in the code the link was read: a parameter's type, a field.
    #[serde(default)]
    pub(super) context: Option<Text>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Pair {
    /// Links the tool read from the code.
    pub(crate) read: u64,
    /// Links it inferred, usually by matching a name.
    pub(crate) guessed: u64,
    pub(crate) kinds: Vec<(u16, u64)>,
    pub(crate) read_kinds: Vec<(u16, u64)>,
}
impl Pair {
    pub(crate) fn weight(&self, certain: bool) -> u64 {
        if certain {
            self.read
        } else {
            self.read + self.guessed
        }
    }
    pub(crate) fn kinds_for(&self, certain: bool) -> &[(u16, u64)] {
        if certain {
            &self.read_kinds
        } else {
            &self.kinds
        }
    }
}
pub(super) struct Builder<'a> {
    pub(super) root: &'a str,
    pub(super) files: Vec<String>,
    pub(super) file_ids: HashMap<String, u32>,
    pub(super) node_symbol: HashMap<String, u32>,
    pub(super) symbols: Vec<Symbol>,
    pub(super) symbol_ir_ids: Vec<String>,
    pub(super) store_ir_ids: bool,
    pub(super) community_names: BTreeMap<u32, String>,
    pub(super) names: String,
    pub(super) uses: Vec<Use>,
    /// Of the links, those that name a type as the type of something, with
    /// what of: `patterns::how` tells the third part.
    pub(super) ties: Vec<Use>,
    pub(super) kinds: Vec<String>,
    pub(super) kind_ids: HashMap<String, u16>,
    pub(super) nodes: u64,
    pub(super) links: u64,
    pub(super) unplaced: u64,
    /// Links met before the node list; the node-link format puts nodes first.
    pub(super) early: Vec<Link>,
    pub(super) listed: bool,
    pub(super) backend: Option<String>,
}
fn clean_graph_path(root: &str, raw: &str) -> Option<String> {
    let mut path = raw.replace('\\', "/");
    if let Some(inside) = path
        .strip_prefix(root)
        .and_then(|rest| rest.strip_prefix('/'))
    {
        path = inside.to_owned();
    }
    let path = path.trim_start_matches("./").trim_matches('/');
    (!path.is_empty()).then(|| path.to_owned())
}
fn classify_symbol_kind(
    label: &str,
    file_type: Option<&str>,
    node_kind: Option<&str>,
    callable_class: Option<&str>,
) -> (Kind, bool) {
    let kind = if file_type == Some("rationale") {
        Kind::Note
    } else if node_kind == Some("page") {
        Kind::File
    } else if file_type == Some("document") {
        Kind::Heading
    } else if callable_class == Some("true") {
        Kind::Type
    } else if label.ends_with("()") {
        if label.starts_with('.') {
            Kind::Method
        } else {
            Kind::Function
        }
    } else {
        Kind::Name
    };
    (kind, kind == Kind::Note)
}
impl<'a> Builder<'a> {
    pub(super) fn new(root: &'a str, store_ir_ids: bool, backend: Option<String>) -> Self {
        Self {
            root,
            files: Vec::new(),
            file_ids: HashMap::new(),
            node_symbol: HashMap::new(),
            symbols: Vec::new(),
            symbol_ir_ids: Vec::new(),
            store_ir_ids,
            community_names: BTreeMap::new(),
            names: String::new(),
            uses: Vec::new(),
            ties: Vec::new(),
            kinds: Vec::new(),
            kind_ids: HashMap::new(),
            nodes: 0,
            links: 0,
            unplaced: 0,
            early: Vec::new(),
            listed: false,
            backend,
        }
    }
    pub(super) fn path(&self, raw: &str) -> Option<String> {
        clean_graph_path(self.root, raw)
    }
    pub(super) fn file(&mut self, path: String) -> u32 {
        if let Some(found) = self.file_ids.get(&path) {
            return *found;
        }
        let id = self.files.len() as u32;
        self.files.push(path.clone());
        self.file_ids.insert(path, id);
        id
    }
    pub(super) fn node(&mut self, node: Node) {
        self.nodes += 1;
        let Some(path) = node.source_file.and_then(|text| self.path(&text.0)) else {
            return;
        };
        let file = self.file(path);
        let label = node.label.0.unwrap_or_default();
        let (kind, is_note) = classify_symbol_kind(
            &label,
            node.file_type.0.as_deref(),
            node.node_kind.0.as_deref(),
            node.class.0.as_deref(),
        );
        // A note is the text of a comment, which the map never shows.
        let kept = if is_note {
            ""
        } else {
            clip(label.trim(), NAME_LIMIT)
        };
        let community = node.community.0.and_then(|value| value.parse::<u32>().ok());
        if let Some((id, name)) = community.zip(node.community_name.0) {
            let name = clip(name.trim(), NAME_LIMIT);
            if !name.is_empty() {
                self.community_names
                    .entry(id)
                    .or_insert_with(|| name.to_owned());
            }
        }
        let is_semantic =
            node.node_kind.0.as_deref() == Some("semantic") || node.id.0.starts_with("sem:");
        let file_is_test = is_test(&self.files[file as usize]);
        let symbol = Symbol {
            file,
            community,
            parent: NONE,
            line: line(node.source_location.0.as_deref()),
            end_line: line(node.end_line.0.as_deref()),
            at: self.names.len() as u32,
            len: kept.len() as u16,
            kind,
            is_abstract: node.is_abstract,
            is_test: node.is_test || file_is_test,
            is_semantic,
        };
        self.names.push_str(kept);
        if self.store_ir_ids {
            self.symbol_ir_ids.push(node.id.0.clone());
        }
        // Of two nodes that share an id, links go to the later one.
        self.node_symbol
            .insert(node.id.0, self.symbols.len() as u32);
        self.symbols.push(symbol);
    }
}
fn apply_nest(symbols: &mut [Symbol], holder: u32, held: u32, relation: &str) {
    let inside = &mut symbols[held as usize];
    if inside.parent == NONE {
        inside.parent = holder;
    }
    let member = match relation {
        "method" => Kind::Method,
        "case_of" => Kind::Case,
        _ => return,
    };
    if inside.kind == Kind::Name || (member == Kind::Method && inside.kind == Kind::Function) {
        inside.kind = member;
    }
    // Whatever has methods or cases is a type, also where the tool does
    // not say so, as for a Rust struct.
    let outside = &mut symbols[holder as usize];
    if outside.kind == Kind::Name {
        outside.kind = Kind::Type;
    }
}
impl Builder<'_> {
    /// Puts a symbol inside the one that holds it. The first holder stays: a
    /// key of a JSON document can be listed under several.
    pub(super) fn nest(&mut self, holder: u32, held: u32, relation: &str) {
        apply_nest(&mut self.symbols, holder, held, relation);
    }
    pub(super) fn link(&mut self, link: Link) {
        self.links += 1;
        let relation = link.relation.map(|text| text.0).unwrap_or_default();
        if relation.starts_with('_') {
            return;
        }
        let (Some(&source), Some(&target)) = (
            self.node_symbol.get(&link.source.0),
            self.node_symbol.get(&link.target.0),
        ) else {
            return;
        };
        let (from, to) = (
            self.symbols[source as usize].file,
            self.symbols[target as usize].file,
        );
        // Where a symbol lives: its file, its class, its enum.
        if STRUCTURE.contains(&relation.as_str()) || (relation == "case_of" && from == to) {
            if relation != "rationale_for" && from == to && source != target {
                self.nest(source, target, &relation);
            }
            return;
        }
        if source == target {
            return;
        }
        // The link names the file it was found in, which is the side that
        // needs the other. Older releases store links without a direction,
        // and a link whose file is neither end comes from two symbols that
        // share an id.
        let (source, target) = match link.source_file.and_then(|text| self.path(&text.0)) {
            None => (source, target),
            Some(found) if found == self.files[from as usize] => (source, target),
            Some(found) if found == self.files[to as usize] => (target, source),
            // Two symbols of one file cannot be linked from another.
            Some(_) if from == to => return,
            Some(_) => {
                self.unplaced += 1;
                return;
            }
        };
        let named = (relation == "references")
            .then(|| {
                link.context
                    .as_ref()
                    .and_then(|text| patterns::how(&text.0))
            })
            .flatten();
        let kind = match self.kind_ids.get(&relation) {
            Some(kind) => *kind,
            None => {
                let kind = self.kinds.len() as u16;
                self.kinds.push(relation.clone());
                self.kind_ids.insert(relation, kind);
                kind
            }
        };
        let read = link
            .confidence
            .is_none_or(|text| text.0.eq_ignore_ascii_case("EXTRACTED"));
        self.uses.push((source, target, kind, !read));
        if let Some(how) = named {
            self.ties.push((source, target, how, !read));
        }
    }
}
pub(super) struct Whole<'a, 'b>(&'a mut Builder<'b>);
pub(super) struct Nodes<'a, 'b>(&'a mut Builder<'b>);
pub(super) struct Links<'a, 'b>(&'a mut Builder<'b>);
impl<'de> DeserializeSeed<'de> for Whole<'_, '_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<(), D::Error> {
        deserializer.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for Whole<'_, '_> {
    type Value = ();
    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a node-link graph")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "nodes" => map.next_value_seed(Nodes(&mut *self.0))?,
                "links" | "edges" => map.next_value_seed(Links(&mut *self.0))?,
                "backend" => {
                    self.0.backend = map
                        .next_value::<Loose>()?
                        .0
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty());
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(())
    }
}
impl<'de> DeserializeSeed<'de> for Nodes<'_, '_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<(), D::Error> {
        deserializer.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for Nodes<'_, '_> {
    type Value = ();
    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a list of nodes")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<(), A::Error> {
        while let Some(node) = seq.next_element::<Node>()? {
            self.0.node(node);
        }
        self.0.listed = true;
        Ok(())
    }
}
impl<'de> DeserializeSeed<'de> for Links<'_, '_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<(), D::Error> {
        deserializer.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for Links<'_, '_> {
    type Value = ();
    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a list of links")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<(), A::Error> {
        while let Some(link) = seq.next_element::<Link>()? {
            if self.0.listed {
                self.0.link(link);
            } else {
                self.0.early.push(link);
            }
        }
        Ok(())
    }
}
/// The links of a graph by the symbol at each end, to walk from one symbol
/// to those it uses and to those that use it without reading every link:
/// the numbers of the links, in the order of what uses and in the order of
/// what is used. Those of symbol `n` are `out[out_first[n]..out_first[n + 1]]`
/// and `into[into_first[n]..into_first[n + 1]]`.
#[derive(Debug)]
pub(super) struct Wires {
    pub(super) out: Vec<u32>,
    pub(super) out_first: Vec<u32>,
    pub(super) into: Vec<u32>,
    pub(super) into_first: Vec<u32>,
}
/// Files of a graph and the links between them, each pair counted once, and
/// the symbols of each file with the links between symbols.
#[derive(Debug)]
pub struct Index {
    index_id: u64,
    /// Where the files of the graph lie, to read a source for its tests.
    root: PathBuf,
    revision_tests: Option<TestSpans>,
    test_symbols: OnceLock<Vec<bool>>,
    /// The same graph without its tests, once it was asked for.
    plain: OnceLock<Arc<Index>>,
    /// The links of each symbol, once the links of one were followed.
    wires: OnceLock<Wires>,
    /// The patterns the graph shows signs of, once they were asked for: by
    /// every link, and by the links the tool read from the code.
    patterns: [OnceLock<patterns::Patterns>; 2],
    /// What the sources read and write of each other, file by file, as far
    /// as pictures asked for it.
    access: access::Access,
    /// The files that use each other in a circle, once they were asked
    /// for: by every link, and by the links the tool read from the code.
    circles: [OnceLock<circles::Circles>; 2],
    /// What a graph without tests leaves out: test files, and tests inside
    /// the files it keeps.
    left: (u64, u64),
    /// Links the graph holds that cannot be: between two Rust crates of
    /// which the first is not built with the second. They are kept out of
    /// everything that is counted and drawn.
    ruled: Vec<Use>,
    crates: Crates,
    files: Vec<String>,
    pairs: Vec<(u32, u32, Pair)>,
    /// Ordered by file: the symbols of file `n` are `first[n]..first[n + 1]`.
    symbols: Vec<Symbol>,
    community_names: BTreeMap<u32, String>,
    first: Vec<u32>,
    names: String,
    ir_node_ids: String,
    ir_node_id_spans: Vec<(u32, u16)>,
    uses: Vec<Use>,
    /// Of the links, those that name a type as the type of something.
    ties: Vec<Use>,
    kinds: Vec<String>,
    nodes: u64,
    links: u64,
    unplaced: u64,
    backend: Option<String>,
    ir_db_path: Option<PathBuf>,
    known_files_set: OnceLock<BTreeSet<String>>,
    manifest_dirs: OnceLock<Vec<String>>,
    member_store: Mutex<PersistentMemberStore>,
}
/// The part of `path` that stands for it when `scope` is open: the child of
/// the scope that holds it, or, outside the scope, the first folder where the
/// two part ways. The flag says whether it is inside.
pub(super) fn group<'a>(scope: &str, path: &'a str) -> (&'a str, bool) {
    if scope.is_empty() {
        return (path.split('/').next().unwrap_or(path), true);
    }
    if let Some(rest) = path
        .strip_prefix(scope)
        .and_then(|rest| rest.strip_prefix('/'))
    {
        let end = rest.find('/').map_or(path.len(), |at| scope.len() + 1 + at);
        return (&path[..end], true);
    }
    let mut end = 0;
    let mut open = scope.split('/');
    for part in path.split('/') {
        let same = open.next() == Some(part);
        end += part.len();
        if !same {
            break;
        }
        end += 1;
    }
    (&path[..end.min(path.len())], false)
}
/// The part of `path` that stands for it when `scope` is open and some of
/// its folders are spread out in it: as `group` tells, but a folder that is
/// spread out gives way to what it holds, and so on down.
pub(super) fn place<'a>(scope: &str, spread: &HashSet<&str>, path: &'a str) -> (&'a str, bool) {
    let (mut key, inside) = group(scope, path);
    while inside && key.len() < path.len() && spread.contains(key) {
        let rest = &path[key.len() + 1..];
        key = &path[..rest.find('/').map_or(path.len(), |at| key.len() + 1 + at)];
    }
    (key, inside)
}
pub(super) fn name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
/// The folder a place lies in; none for what lies in the project itself.
pub(super) fn holder(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(folder, _)| folder)
}
/// The part of `path` that stands for it among the links followed from a
/// place `depth` folders deep: the folder at that depth that holds it, or the
/// path itself where it lies no deeper.
pub(super) fn level(path: &str, depth: usize) -> &str {
    match path.match_indices('/').nth(depth.saturating_sub(1)) {
        Some((at, _)) => &path[..at],
        None => path,
    }
}
/// What a place of a picture stands for when a link between two places is
/// looked into: everything under a folder, one file, or one symbol.
#[derive(Clone, Copy)]
pub(super) enum Reach<'a> {
    Folder(&'a str),
    File(u32),
    Symbol(u32),
}
#[derive(Default)]
pub(super) struct Tally {
    pub(super) files: u64,
    pub(super) folder: bool,
    pub(super) inside: bool,
    /// What the files of this place hold: classes, methods, functions.
    pub(super) symbols: u64,
    pub(super) within: u64,
    pub(super) out: u64,
    pub(super) into: u64,
}
#[derive(Clone, Copy, Default)]
pub(super) struct Flow {
    pub(super) links: u64,
    pub(super) guessed: u64,
}
/// Link tally and aggregated per-relation counts for one rendered map edge.
#[derive(Clone, Default)]
pub(super) struct EdgeFlow {
    pub(super) flow: Flow,
    pub(super) kinds: BTreeMap<u16, u64>,
}
impl EdgeFlow {
    pub(super) fn add_link(&mut self, kind: u16, guessed: bool) {
        self.flow.links += 1;
        self.flow.guessed += u64::from(guessed);
        *self.kinds.entry(kind).or_default() += 1;
    }
    pub(super) fn add_pair(&mut self, pair: &Pair, certain: bool) {
        let weight = pair.weight(certain);
        if weight == 0 {
            return;
        }
        self.flow.links += weight;
        self.flow.guessed += if certain { 0 } else { pair.guessed };
        add_kind_counts(&mut self.kinds, pair.kinds_for(certain));
    }
    pub(super) fn merge(&mut self, other: &Self) {
        self.flow.links += other.flow.links;
        self.flow.guessed += other.flow.guessed;
        for (&kind, &count) in &other.kinds {
            *self.kinds.entry(kind).or_default() += count;
        }
    }
}
pub(super) fn add_kind_counts(dest: &mut BTreeMap<u16, u64>, counts: &[(u16, u64)]) {
    for &(kind, count) in counts {
        *dest.entry(kind).or_default() += count;
    }
}
/// Two places asked what they share, and which part of that to draw.
#[derive(Clone, Copy, Debug)]
pub struct Sharing<'a> {
    pub from: &'a str,
    pub to: &'a str,
    /// What uses both, or what both use. Where it is not said, what both
    /// use, unless only the other kind has anything.
    pub users: Option<bool>,
    /// How many of the nearest places of that kind are passed over: the
    /// picture begins with the one after them.
    pub skip: usize,
}
static NEXT_INDEX_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub(super) fn next_index_id() -> u64 {
    NEXT_INDEX_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl Index {
    /// Monotonic in-process identity of this `Index` snapshot.
    pub(crate) fn index_id(&self) -> u64 {
        self.index_id
    }

    /// Reads a node-link graph without holding it in memory: nodes become a
    /// table from symbol to file, links are counted as they pass. The
    /// manifests of the tree under `root` say which links cannot be.
    pub fn read(source: impl std::io::Read, root: &Path) -> Result<Self> {
        Self::read_with(source, root, &|path| text_at(root, path))
    }
    /// Reads a graph directly from typed SQLite IR nodes and links without a JSON round-trip.
    pub(crate) fn from_ir_parts(
        root: &Path,
        ir_db_path: Option<PathBuf>,
        backend: Option<String>,
        ir_nodes: Vec<crate::graph_ir::IrNodeSpec>,
        ir_links: Vec<crate::graph_ir::IrLinkSpec>,
    ) -> Result<Self> {
        let base = root.to_path_buf();
        let root_str = root.to_string_lossy().replace('\\', "/");
        let mut builder = Builder::new(root_str.trim_end_matches('/'), true, backend);
        for spec in ir_nodes {
            builder.node(Node {
                id: Text(spec.id),
                source_file: Some(Text(spec.source_file)),
                label: Loose(Some(spec.label)),
                source_location: Loose(spec.source_location),
                end_line: Loose((spec.end_line > 0).then(|| spec.end_line.to_string())),
                file_type: Loose(spec.file_type),
                node_kind: Loose(spec.node_kind),
                community: Loose(spec.community.map(|c| c.to_string())),
                community_name: Loose(spec.community_name),
                class: Loose(spec.callable_class.then(|| "true".to_owned())),
                is_abstract: spec.is_abstract,
                is_test: spec.is_test,
            });
        }
        builder.listed = true;
        for spec in ir_links {
            builder.link(Link {
                source: Text(spec.source),
                target: Text(spec.target),
                relation: spec.relation.map(Text),
                confidence: spec.confidence.map(Text),
                source_file: spec.source_file.map(Text),
                context: spec.context.map(Text),
            });
        }
        Self::finish_builder(builder, base, ir_db_path, &|path| text_at(root, path))
    }

    /// Reads a graph, with `text` giving a file of its tree where it can be
    /// read.
    pub(super) fn read_with(
        source: impl std::io::Read,
        root: &Path,
        text: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self> {
        let base = root.to_path_buf();
        let root = root.to_string_lossy().replace('\\', "/");
        let mut builder = Builder::new(root.trim_end_matches('/'), false, None);
        let mut reader = serde_json::Deserializer::from_reader(std::io::BufReader::new(source));
        Whole(&mut builder)
            .deserialize(&mut reader)
            .map_err(|_| HarnessError::invalid("The code graph file is not a node-link graph."))?;
        for link in std::mem::take(&mut builder.early) {
            builder.link(link);
        }
        Self::finish_builder(builder, base, None, text)
    }

    pub(super) fn finish_builder(
        builder: Builder<'_>,
        base: PathBuf,
        ir_db_path: Option<PathBuf>,
        text: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self> {
        // The node that stands for a file is named after it, by its name or
        // by the end of its path, and what it holds is the top of the file.
        let mut symbols = builder.symbols;
        for symbol in &mut symbols {
            if symbol.parent != NONE || symbol.kind != Kind::Name {
                continue;
            }
            let label =
                &builder.names[symbol.at as usize..symbol.at as usize + symbol.len as usize];
            let path = &builder.files[symbol.file as usize];
            if !label.is_empty()
                && path
                    .strip_suffix(label)
                    .is_some_and(|rest| rest.is_empty() || rest.ends_with('/'))
            {
                symbol.kind = Kind::File;
            }
        }
        for at in 0..symbols.len() {
            let parent = symbols[at].parent;
            if parent != NONE && symbols[parent as usize].kind == Kind::File {
                symbols[at].parent = NONE;
            }
        }
        // Symbols are put in the order of their files, each file's in the
        // order of the graph.
        let mut order = (0..symbols.len() as u32).collect::<Vec<_>>();
        order.sort_by_key(|at| symbols[*at as usize].file);
        let mut place = vec![0u32; symbols.len()];
        for (new, old) in order.iter().enumerate() {
            place[*old as usize] = new as u32;
        }
        let mut first = vec![0u32; builder.files.len() + 1];
        let mut ir_node_ids = String::new();
        let mut ir_node_id_spans = if builder.store_ir_ids {
            Vec::with_capacity(order.len())
        } else {
            Vec::new()
        };
        let symbols = order
            .iter()
            .map(|old| {
                let mut symbol = symbols[*old as usize];
                if symbol.parent != NONE {
                    symbol.parent = place[symbol.parent as usize];
                }
                first[symbol.file as usize + 1] += 1;
                if builder.store_ir_ids {
                    let raw_id = &builder.symbol_ir_ids[*old as usize];
                    if raw_id.len() <= u16::MAX as usize {
                        let start = ir_node_ids.len() as u32;
                        ir_node_ids.push_str(raw_id);
                        ir_node_id_spans.push((start, raw_id.len() as u16));
                    } else {
                        ir_node_id_spans.push((0, 0));
                    }
                }
                symbol
            })
            .collect::<Vec<_>>();
        for file in 0..builder.files.len() {
            first[file + 1] += first[file];
        }
        // A crate is built with the crates its manifest names and with no
        // other, so a link into any other crate is one the tool made up.
        let crates = Crates::find(&builder.files, text);
        let (uses, ruled): (Vec<Use>, Vec<Use>) = builder
            .uses
            .into_iter()
            .map(|(from, to, kind, guessed)| {
                (place[from as usize], place[to as usize], kind, guessed)
            })
            .partition(|(from, to, ..)| {
                !crates.rules_out(symbols[*from as usize].file, symbols[*to as usize].file)
            });
        // What names a type across two crates that are not built together
        // names another type of that name.
        let ties = builder
            .ties
            .into_iter()
            .map(|(from, to, how, guessed)| {
                (place[from as usize], place[to as usize], how, guessed)
            })
            .filter(|(from, to, ..)| {
                !crates.rules_out(symbols[*from as usize].file, symbols[*to as usize].file)
            })
            .collect();
        Ok(Self {
            index_id: next_index_id(),
            root: base,
            revision_tests: None,
            test_symbols: OnceLock::new(),
            plain: OnceLock::new(),
            wires: OnceLock::new(),
            patterns: Default::default(),
            access: Default::default(),
            circles: Default::default(),
            left: (0, 0),
            ruled,
            crates,
            files: builder.files,
            pairs: pairs_of(&symbols, &uses),
            symbols,
            first,
            names: builder.names,
            ir_node_ids,
            ir_node_id_spans,
            community_names: builder.community_names,
            uses,
            ties,
            kinds: builder.kinds,
            nodes: builder.nodes,
            links: builder.links,
            unplaced: builder.unplaced,
            backend: builder.backend,
            ir_db_path,
            known_files_set: OnceLock::new(),
            manifest_dirs: OnceLock::new(),
            member_store: Mutex::new(PersistentMemberStore::default()),
        })
    }
    pub(super) fn folder(&self, scope: &str) -> bool {
        scope.is_empty()
            || self.files.iter().any(|file| {
                file.strip_prefix(scope)
                    .is_some_and(|rest| rest.starts_with('/'))
            })
    }
    /// The path of the file a scope opens, when it names a file.
    pub fn opens(&self, scope: &str) -> Option<&str> {
        self.opened(scope)
            .map(|file| self.files[file as usize].as_str())
    }
    /// The file a scope names, inside a change: the changed files around it
    /// are marked, and with `edited`, what the change did to this file, so
    /// are the symbols it touched. `None` when the scope names no file.
    #[allow(clippy::too_many_arguments)]
    pub fn symbols(
        &self,
        project: &str,
        scope: &str,
        changed: &[(String, char)],
        edited: Option<&Edited>,
        focus: Option<&str>,
        certain: bool,
        filter: &str,
    ) -> Option<Value> {
        let file = self.opened(scope)?;
        let letters = changed
            .iter()
            .filter(|(_, kind)| *kind != 'D')
            .map(|(path, kind)| (path.as_str(), *kind))
            .collect::<HashMap<_, _>>();
        Some(self.file(project, file, &letters, edited, focus, certain, filter))
    }
    /// The file a scope names, when it names one and not a folder.
    pub(super) fn opened(&self, scope: &str) -> Option<u32> {
        let scope = scope.trim_matches('/');
        if scope.is_empty() || self.folder(scope) {
            return None;
        }
        self.files
            .iter()
            .position(|file| file == scope)
            .map(|at| at as u32)
    }
    pub(crate) fn node_link_counts(&self) -> (u64, u64) {
        (self.nodes, self.links)
    }

    fn ir_node_id(&self, sym: u32) -> &str {
        let Some(&(at, len)) = self.ir_node_id_spans.get(sym as usize) else {
            return "";
        };
        let start = at as usize;
        let end = start + len as usize;
        self.ir_node_ids.get(start..end).unwrap_or("")
    }

    /// Builds a new `Index` by updating only dirty files and incident edges on top of `self`.
    /// Returns `None` if `self` lacks IR symbol IDs or an incident neighbor ID cannot be mapped,
    /// allowing the caller to fall back cleanly to a full rebuild.
    pub(crate) fn update_files(
        &self,
        root: &Path,
        delta: crate::graph_ir::IrDeltaElements,
    ) -> Option<Self> {
        if delta.files.is_empty() || self.ir_node_id_spans.len() != self.symbols.len() {
            return None;
        }
        let root_str = root.to_string_lossy().replace('\\', "/");
        let root_trim = root_str.trim_end_matches('/');

        let mut touched = delta.edited_files.clone();
        touched.extend(delta.removed_files.iter().cloned());
        let new_crates =
            self.crates
                .update_files(&self.files, &delta.files, &touched, false, &|path| {
                    text_at(root, path)
                });

        let old_by_path: HashMap<&str, u32> = self
            .files
            .iter()
            .enumerate()
            .map(|(i, p)| (p.as_str(), i as u32))
            .collect();
        let new_by_path: HashMap<&str, u32> = delta
            .files
            .iter()
            .enumerate()
            .map(|(i, p)| (p.as_str(), i as u32))
            .collect();

        let mut community_names = self.community_names.clone();
        let mut delta_symbols: Vec<Symbol> = Vec::with_capacity(delta.node_specs.len());
        let mut delta_labels: Vec<String> = Vec::with_capacity(delta.node_specs.len());
        let mut delta_ir_ids: Vec<String> = Vec::with_capacity(delta.node_specs.len());
        let mut delta_id_to_local: HashMap<String, u32> = HashMap::new();

        for spec in delta.node_specs {
            let Some(clean_file) = clean_graph_path(root_trim, &spec.source_file) else {
                continue;
            };
            let Some(&new_file) = new_by_path.get(clean_file.as_str()) else {
                continue;
            };
            let callable = spec.callable_class.then_some("true");
            let (kind, is_note) = classify_symbol_kind(
                &spec.label,
                spec.file_type.as_deref(),
                spec.node_kind.as_deref(),
                callable,
            );
            let kept = if is_note {
                ""
            } else {
                clip(spec.label.trim(), NAME_LIMIT)
            };
            if let Some((id, name)) = spec.community.zip(spec.community_name) {
                let clipped = clip(name.trim(), NAME_LIMIT);
                if !clipped.is_empty() {
                    community_names
                        .entry(id)
                        .or_insert_with(|| clipped.to_owned());
                }
            }
            let is_semantic =
                spec.node_kind.as_deref() == Some("semantic") || spec.id.starts_with("sem:");
            let file_is_test = is_test(&delta.files[new_file as usize]);
            let sym = Symbol {
                file: new_file,
                community: spec.community,
                parent: NONE,
                line: line(spec.source_location.as_deref()),
                end_line: spec.end_line,
                at: 0,
                len: kept.len() as u16,
                kind,
                is_abstract: spec.is_abstract,
                is_test: spec.is_test || file_is_test,
                is_semantic,
            };
            let local_idx = delta_symbols.len() as u32;
            delta_id_to_local.insert(spec.id.clone(), local_idx);
            delta_symbols.push(sym);
            delta_labels.push(kept.to_owned());
            delta_ir_ids.push(spec.id);
        }

        let delta_total_links = delta.link_specs.len() as u64;
        let mut non_struct_links = Vec::with_capacity(delta.link_specs.len());
        for link in delta.link_specs {
            let rel = link.relation.as_deref().unwrap_or("");
            if rel.starts_with('_') {
                continue;
            }
            let is_structure = STRUCTURE.contains(&rel) || rel == "case_of";
            if is_structure
                && let (Some(&src), Some(&dst)) = (
                    delta_id_to_local.get(&link.source),
                    delta_id_to_local.get(&link.target),
                )
            {
                let from_f = delta_symbols[src as usize].file;
                let to_f = delta_symbols[dst as usize].file;
                if (STRUCTURE.contains(&rel) || (rel == "case_of" && from_f == to_f))
                    && rel != "rationale_for"
                    && from_f == to_f
                    && src != dst
                {
                    apply_nest(&mut delta_symbols, src, dst, rel);
                    continue;
                }
                if STRUCTURE.contains(&rel) {
                    continue;
                }
            }
            if !STRUCTURE.contains(&rel) {
                non_struct_links.push(link);
            }
        }

        for idx in 0..delta_symbols.len() {
            if delta_symbols[idx].parent != NONE || delta_symbols[idx].kind != Kind::Name {
                continue;
            }
            let label = &delta_labels[idx];
            let path = &delta.files[delta_symbols[idx].file as usize];
            if !label.is_empty()
                && path
                    .strip_suffix(label.as_str())
                    .is_some_and(|rest| rest.is_empty() || rest.ends_with('/'))
            {
                delta_symbols[idx].kind = Kind::File;
            }
        }
        for idx in 0..delta_symbols.len() {
            let parent = delta_symbols[idx].parent;
            if parent != NONE && delta_symbols[parent as usize].kind == Kind::File {
                delta_symbols[idx].parent = NONE;
            }
        }

        let mut delta_by_file: HashMap<u32, Vec<u32>> = HashMap::new();
        for (local_idx, sym) in delta_symbols.iter().enumerate() {
            delta_by_file
                .entry(sym.file)
                .or_default()
                .push(local_idx as u32);
        }

        let mut new_symbols: Vec<Symbol> = Vec::with_capacity(self.symbols.len());
        let mut new_first = vec![0u32; delta.files.len() + 1];
        let mut new_names = String::with_capacity(self.names.len());
        let mut new_ir_node_ids = String::with_capacity(self.ir_node_ids.len());
        let mut new_ir_node_id_spans: Vec<(u32, u16)> = Vec::with_capacity(self.symbols.len());
        let mut old_to_new_sym = vec![NONE; self.symbols.len()];
        let mut delta_to_new_sym = vec![NONE; delta_symbols.len()];
        let mut is_new_delta_sym: Vec<bool> = Vec::with_capacity(self.symbols.len());

        let mut push_ir_id = |ir_id: &str| {
            if ir_id.len() <= u16::MAX as usize {
                let start = new_ir_node_ids.len() as u32;
                new_ir_node_ids.push_str(ir_id);
                new_ir_node_id_spans.push((start, ir_id.len() as u16));
            } else {
                new_ir_node_id_spans.push((0, 0));
            }
        };

        for (new_f, path) in delta.files.iter().enumerate() {
            if delta.edited_files.contains(path) {
                if let Some(locals) = delta_by_file.get(&(new_f as u32)) {
                    let base = new_symbols.len();
                    for &local in locals {
                        let mut sym = delta_symbols[local as usize];
                        sym.at = new_names.len() as u32;
                        new_names.push_str(&delta_labels[local as usize]);
                        push_ir_id(&delta_ir_ids[local as usize]);
                        delta_to_new_sym[local as usize] = new_symbols.len() as u32;
                        new_symbols.push(sym);
                        is_new_delta_sym.push(true);
                    }
                    for sym in &mut new_symbols[base..] {
                        if sym.parent != NONE {
                            sym.parent = delta_to_new_sym[sym.parent as usize];
                        }
                    }
                }
            } else if let Some(&old_f) = old_by_path.get(path.as_str()) {
                for old_sym in self.range(old_f) {
                    let mut sym = self.symbols[old_sym as usize];
                    sym.file = new_f as u32;
                    sym.at = new_names.len() as u32;
                    new_names.push_str(self.label(old_sym));
                    let ir_id = self.ir_node_id(old_sym);
                    push_ir_id(ir_id);
                    old_to_new_sym[old_sym as usize] = new_symbols.len() as u32;
                    new_symbols.push(sym);
                    is_new_delta_sym.push(false);
                }
            }
            new_first[new_f + 1] = new_symbols.len() as u32;
        }

        for (idx, sym) in new_symbols.iter_mut().enumerate() {
            if !is_new_delta_sym[idx] && sym.parent != NONE {
                let mapped = old_to_new_sym[sym.parent as usize];
                if mapped == NONE {
                    return None;
                }
                sym.parent = mapped;
            }
        }

        let old_file_dirty: Vec<bool> = self
            .files
            .iter()
            .map(|p| delta.dirty_files.contains(p))
            .collect();
        let mut removed_node_count = 0u64;
        let mut dropped_struct_links = 0u64;
        for (old_f, path) in self.files.iter().enumerate() {
            if delta.edited_files.contains(path) || delta.removed_files.contains(path) {
                let r = self.range(old_f as u32);
                removed_node_count += r.len() as u64;
                for old_sym in r {
                    if self.symbols[old_sym as usize].parent != NONE {
                        dropped_struct_links += 1;
                    }
                }
            }
        }

        let is_dirty_old = |sym: u32| old_file_dirty[self.symbols[sym as usize].file as usize];
        let check_multi_rs = delta.rs_touched && new_crates.is_multi_crate();
        let mut dropped_non_struct = 0u64;
        let mut new_uses = Vec::with_capacity(self.uses.len());
        let mut new_ruled = Vec::with_capacity(self.ruled.len());
        for &(from, to, kind, guessed) in &self.uses {
            if is_dirty_old(from) || is_dirty_old(to) {
                dropped_non_struct += 1;
                continue;
            }
            let nf = old_to_new_sym[from as usize];
            let nt = old_to_new_sym[to as usize];
            if nf == NONE || nt == NONE {
                return None;
            }
            if check_multi_rs
                && new_crates
                    .rules_out(new_symbols[nf as usize].file, new_symbols[nt as usize].file)
            {
                new_ruled.push((nf, nt, kind, guessed));
            } else {
                new_uses.push((nf, nt, kind, guessed));
            }
        }
        for &(from, to, kind, guessed) in &self.ruled {
            if is_dirty_old(from) || is_dirty_old(to) {
                dropped_non_struct += 1;
                continue;
            }
            let nf = old_to_new_sym[from as usize];
            let nt = old_to_new_sym[to as usize];
            if nf == NONE || nt == NONE {
                return None;
            }
            if check_multi_rs
                && !new_crates
                    .rules_out(new_symbols[nf as usize].file, new_symbols[nt as usize].file)
            {
                new_uses.push((nf, nt, kind, guessed));
            } else {
                new_ruled.push((nf, nt, kind, guessed));
            }
        }
        let mut new_ties = Vec::with_capacity(self.ties.len());
        for &(from, to, how, guessed) in &self.ties {
            if is_dirty_old(from) || is_dirty_old(to) {
                continue;
            }
            let nf = old_to_new_sym[from as usize];
            let nt = old_to_new_sym[to as usize];
            if nf == NONE || nt == NONE {
                return None;
            }
            if !check_multi_rs
                || !new_crates
                    .rules_out(new_symbols[nf as usize].file, new_symbols[nt as usize].file)
            {
                new_ties.push((nf, nt, how, guessed));
            }
        }

        let mut endpoint_id_to_sym: HashMap<&str, u32> = HashMap::new();
        for f_path in delta.edited_files.iter().chain(delta.neighbor_files.iter()) {
            let Some(&new_f) = new_by_path.get(f_path.as_str()) else {
                continue;
            };
            let start = new_first[new_f as usize];
            let end = new_first[new_f as usize + 1];
            for sym_idx in start..end {
                let (at, len) = new_ir_node_id_spans[sym_idx as usize];
                if len > 0
                    && let Some(id_str) =
                        new_ir_node_ids.get(at as usize..at as usize + len as usize)
                {
                    endpoint_id_to_sym.insert(id_str, sym_idx);
                }
            }
        }

        let mut new_kinds = self.kinds.clone();
        let mut kind_ids: HashMap<String, u16> = new_kinds
            .iter()
            .enumerate()
            .map(|(i, k)| (k.clone(), i as u16))
            .collect();
        let mut delta_unplaced = 0u64;
        for link in non_struct_links {
            let relation = link.relation.unwrap_or_default();
            if relation.starts_with('_') {
                continue;
            }
            let &source_sym = endpoint_id_to_sym.get(link.source.as_str())?;
            let &target_sym = endpoint_id_to_sym.get(link.target.as_str())?;
            if source_sym == target_sym {
                continue;
            }
            let (from_f, to_f) = (
                new_symbols[source_sym as usize].file,
                new_symbols[target_sym as usize].file,
            );
            let (src, dst) = match link
                .source_file
                .as_deref()
                .and_then(|raw| clean_graph_path(root_trim, raw))
            {
                None => (source_sym, target_sym),
                Some(ref found) if found == &delta.files[from_f as usize] => {
                    (source_sym, target_sym)
                }
                Some(ref found) if found == &delta.files[to_f as usize] => (target_sym, source_sym),
                Some(_) if from_f == to_f => continue,
                Some(_) => {
                    delta_unplaced += 1;
                    continue;
                }
            };
            let named = (relation == "references")
                .then(|| link.context.as_deref().and_then(patterns::how))
                .flatten();
            let kind = match kind_ids.get(&relation) {
                Some(&k) => k,
                None => {
                    let k = new_kinds.len() as u16;
                    new_kinds.push(relation.clone());
                    kind_ids.insert(relation, k);
                    k
                }
            };
            let read = link
                .confidence
                .as_deref()
                .is_none_or(|c| c.eq_ignore_ascii_case("EXTRACTED"));
            let (src_file, dst_file) = (
                new_symbols[src as usize].file,
                new_symbols[dst as usize].file,
            );
            let link_tuple = (src, dst, kind, !read);
            if new_crates.rules_out(src_file, dst_file) {
                new_ruled.push(link_tuple);
            } else {
                new_uses.push(link_tuple);
                if let Some(how) = named {
                    new_ties.push((src, dst, how, !read));
                }
            }
        }

        let new_nodes = self
            .nodes
            .saturating_sub(removed_node_count)
            .saturating_add(delta_symbols.len() as u64);
        let new_links = self
            .links
            .saturating_sub(dropped_non_struct.saturating_add(dropped_struct_links))
            .saturating_add(delta_total_links);

        Some(Self {
            index_id: next_index_id(),
            root: self.root.clone(),
            revision_tests: None,
            test_symbols: OnceLock::new(),
            plain: OnceLock::new(),
            wires: OnceLock::new(),
            patterns: Default::default(),
            access: Default::default(),
            circles: Default::default(),
            left: (0, 0),
            ruled: new_ruled,
            crates: new_crates,
            pairs: pairs_of(&new_symbols, &new_uses),
            files: delta.files,
            symbols: new_symbols,
            community_names,
            first: new_first,
            names: new_names,
            ir_node_ids: new_ir_node_ids,
            ir_node_id_spans: new_ir_node_id_spans,
            uses: new_uses,
            ties: new_ties,
            kinds: new_kinds,
            nodes: new_nodes,
            links: new_links,
            unplaced: self.unplaced.saturating_add(delta_unplaced),
            backend: self.backend.clone(),
            ir_db_path: self.ir_db_path.clone(),
            known_files_set: OnceLock::new(),
            manifest_dirs: OnceLock::new(),
            member_store: Mutex::new(PersistentMemberStore::default()),
        })
    }

    pub(super) fn totals(&self) -> Value {
        let (read, guessed) = self
            .pairs
            .iter()
            .fold((0, 0), |(read, guessed), (_, _, pair)| {
                (read + pair.read, guessed + pair.guessed)
            });
        json!({
            "files": self.files.len(),
            "symbols": self.nodes,
            "relations": self.links,
            "links": read + guessed,
            "read": read,
            "guessed": guessed,
            "unplaced": self.unplaced,
            "ruled_out": self.ruled.len(),
        })
    }
    pub(super) fn crumbs(project: &str, scope: &str) -> Vec<Value> {
        let mut crumbs = vec![json!({"name": project, "path": ""})];
        let mut walked = String::new();
        for part in scope.split('/').filter(|part| !part.is_empty()) {
            if !walked.is_empty() {
                walked.push('/');
            }
            walked.push_str(part);
            crumbs.push(json!({"name": part, "path": walked}));
        }
        crumbs
    }
}

#[derive(Clone, Copy)]
pub(super) struct ViewLimits {
    pub(super) shown: usize,
    pub(super) spread: usize,
    pub(super) held: usize,
    pub(super) own: usize,
    pub(super) drawn: usize,
}

pub(super) const NORMAL_LIMITS: ViewLimits = ViewLimits {
    shown: SHOWN,
    spread: SPREAD,
    held: HELD,
    own: OWN,
    drawn: DRAWN,
};

pub(super) const WHOLE_LIMITS: ViewLimits = ViewLimits {
    shown: WHOLE_SHOWN,
    spread: WHOLE_SPREAD,
    held: WHOLE_HELD,
    own: OWN,
    drawn: WHOLE_DRAWN,
};

pub(crate) fn is_test_path(path: &str) -> bool {
    is_test(path)
}

pub(crate) fn check_layer_rule(
    rules: &layer_rules::Rules,
    from_path: &str,
    to_path: &str,
) -> Option<(String, String, String, String)> {
    if is_test(from_path) || is_test(to_path) {
        return None;
    }
    let from_layer = rules.layer(from_path)?;
    let to_layer = rules.layer(to_path)?;
    if !rules.forbidden(from_layer, to_layer) {
        return None;
    }
    let from = &rules.layers[from_layer];
    let to = &rules.layers[to_layer];
    Some((
        from.id.clone(),
        from.name.clone(),
        to.id.clone(),
        to.name.clone(),
    ))
}
