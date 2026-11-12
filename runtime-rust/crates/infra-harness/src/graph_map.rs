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
use crate::{HarnessError, Result, graph_crates::Crates};
use serde::de::{DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    fmt,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex, OnceLock},
    time::SystemTime,
};
/// Children of one folder drawn at once; the rest are reached by the filter.
pub const SHOWN: usize = 40;
/// Links drawn between those children, strongest first.
pub const DRAWN: usize = 160;
const LISTED: usize = 12;
/// More links than this per child and the folder is drawn as a backbone:
/// each child keeps its `STRONGEST` uses.
const DENSE: usize = 2;
pub const STRONGEST: usize = 3;
const KEPT: usize = 4;
/// Changed files drawn one by one; more of them are drawn as the children of
/// the open folder, which can be opened in turn.
const SINGLE: usize = 24;
/// Places kept for the files linked to a change, however large it is.
const AROUND: usize = 8;
const BYTE_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
/// Symbols of one file drawn at once; the rest are reached by the filter.
const NAMED: usize = SHOWN - AROUND;
/// Symbols listed on each side of a selected one.
const TOLD: usize = 30;
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
const SPREAD: usize = 100;
const OWN: usize = 20;
/// Names each end of a link shows when the link is looked into, the most
/// linked first, and the links drawn between the two ends.
pub const LINKED: usize = 150;
const WIRED: usize = 600;
/// Places a search of the whole graph lists at once, the best first; the
/// rest are counted, and a longer search reaches them.
pub const FOUND: usize = 40;
/// Words of one search that are read.
const WORDS: usize = 8;
/// Folders that hold tests, or what tests are fed with, in the common
/// languages.
const TEST_FOLDERS: &[&str] = &[
    "test",
    "tests",
    "__tests__",
    "spec",
    "testdata",
    "fixtures",
    "__fixtures__",
    "__mocks__",
    "e2e",
    "cypress",
];
/// The largest source that is read: for the tests inside it, and for the
/// other sources it names.
pub(crate) const SOURCE_LIMIT: u64 = 4 * 1024 * 1024;
/// Bytes of a symbol's name that are kept.
const NAME_LIMIT: usize = 160;
/// Relations that say where a symbol lives or why, not what it needs.
const STRUCTURE: &[&str] = &["contains", "method", "defines", "rationale_for"];
/// No symbol: the top of a file.
const NONE: u32 = u32::MAX;
/// A JSON scalar read as text; Graphify writes ids as strings, other writers
/// of the node-link format use numbers.
struct Text(String);
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
struct Loose(Option<String>);
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
struct Node {
    id: Text,
    #[serde(default)]
    source_file: Option<Text>,
    #[serde(default)]
    label: Loose,
    #[serde(default)]
    source_location: Loose,
    #[serde(default)]
    file_type: Loose,
    #[serde(default)]
    node_kind: Loose,
    #[serde(default, rename = "_callable_class")]
    class: Loose,
}
/// What a symbol is, as far as the graph tells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
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
    fn word(self) -> &'static str {
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
struct Symbol {
    file: u32,
    /// The symbol that holds it, or `NONE` at the top of the file.
    parent: u32,
    /// Where it starts, or 0 when the graph does not say.
    line: u32,
    at: u32,
    len: u16,
    kind: Kind,
}
/// A link between two symbols: the one that uses, the one that is used, the
/// relation, and whether the tool only inferred it.
type Use = (u32, u32, u16, bool);
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
enum Row {
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
struct Line {
    indent: u32,
    kind: Row,
    /// The line ends a statement or a block, so nothing of it follows.
    whole: bool,
}
/// Words that go on with a definition at the definition's own depth.
const CONTINUES: &[&str] = &[
    "end", "else", "elif", "elsif", "elseif", "rescue", "ensure", "except", "finally", "catch",
    "where", "begin", "then", "do",
];
fn lines(text: &str) -> Vec<Line> {
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
fn clip(text: &str, limit: usize) -> &str {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
/// The line of `L42`, as the tool writes a place in a file.
fn line(location: Option<&str>) -> u32 {
    let text = location.unwrap_or("").trim_start_matches(['L', 'l']);
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    text[..digits].parse().unwrap_or(0)
}
#[derive(serde::Deserialize)]
struct Link {
    source: Text,
    target: Text,
    #[serde(default)]
    relation: Option<Text>,
    #[serde(default)]
    confidence: Option<Text>,
    #[serde(default)]
    source_file: Option<Text>,
}
#[derive(Clone, Debug, Default, PartialEq)]
struct Pair {
    /// Links the tool read from the code.
    read: u64,
    /// Links it inferred, usually by matching a name.
    guessed: u64,
    kinds: Vec<(u16, u64)>,
}
impl Pair {
    fn weight(&self, certain: bool) -> u64 {
        if certain {
            self.read
        } else {
            self.read + self.guessed
        }
    }
}
struct Builder<'a> {
    root: &'a str,
    files: Vec<String>,
    file_ids: HashMap<String, u32>,
    node_symbol: HashMap<String, u32>,
    symbols: Vec<Symbol>,
    names: String,
    uses: Vec<Use>,
    kinds: Vec<String>,
    kind_ids: HashMap<String, u16>,
    nodes: u64,
    links: u64,
    unplaced: u64,
    /// Links met before the node list; the node-link format puts nodes first.
    early: Vec<Link>,
    listed: bool,
}
impl Builder<'_> {
    fn path(&self, raw: &str) -> Option<String> {
        let mut path = raw.replace('\\', "/");
        if let Some(inside) = path
            .strip_prefix(self.root)
            .and_then(|rest| rest.strip_prefix('/'))
        {
            path = inside.to_owned();
        }
        let path = path.trim_start_matches("./").trim_matches('/');
        (!path.is_empty()).then(|| path.to_owned())
    }
    fn file(&mut self, path: String) -> u32 {
        if let Some(found) = self.file_ids.get(&path) {
            return *found;
        }
        let id = self.files.len() as u32;
        self.files.push(path.clone());
        self.file_ids.insert(path, id);
        id
    }
    fn node(&mut self, node: Node) {
        self.nodes += 1;
        let Some(path) = node.source_file.and_then(|text| self.path(&text.0)) else {
            return;
        };
        let file = self.file(path);
        let label = node.label.0.unwrap_or_default();
        let kind = if node.file_type.0.as_deref() == Some("rationale") {
            Kind::Note
        } else if node.node_kind.0.as_deref() == Some("page") {
            Kind::File
        } else if node.file_type.0.as_deref() == Some("document") {
            Kind::Heading
        } else if node.class.0.as_deref() == Some("true") {
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
        // A note is the text of a comment, which the map never shows.
        let kept = if kind == Kind::Note {
            ""
        } else {
            clip(label.trim(), NAME_LIMIT)
        };
        let symbol = Symbol {
            file,
            parent: NONE,
            line: line(node.source_location.0.as_deref()),
            at: self.names.len() as u32,
            len: kept.len() as u16,
            kind,
        };
        self.names.push_str(kept);
        // Of two nodes that share an id, links go to the later one.
        self.node_symbol
            .insert(node.id.0, self.symbols.len() as u32);
        self.symbols.push(symbol);
    }
    /// Puts a symbol inside the one that holds it. The first holder stays: a
    /// key of a JSON document can be listed under several.
    fn nest(&mut self, holder: u32, held: u32, relation: &str) {
        let inside = &mut self.symbols[held as usize];
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
        let outside = &mut self.symbols[holder as usize];
        if outside.kind == Kind::Name {
            outside.kind = Kind::Type;
        }
    }
    fn link(&mut self, link: Link) {
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
    }
}
struct Whole<'a, 'b>(&'a mut Builder<'b>);
struct Nodes<'a, 'b>(&'a mut Builder<'b>);
struct Links<'a, 'b>(&'a mut Builder<'b>);
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
struct Wires {
    out: Vec<u32>,
    out_first: Vec<u32>,
    into: Vec<u32>,
    into_first: Vec<u32>,
}
/// Files of a graph and the links between them, each pair counted once, and
/// the symbols of each file with the links between symbols.
#[derive(Debug)]
pub struct Index {
    /// Where the files of the graph lie, to read a source for its tests.
    root: PathBuf,
    /// The same graph without its tests, once it was asked for.
    plain: OnceLock<Arc<Index>>,
    /// The links of each symbol, once the links of one were followed.
    wires: OnceLock<Wires>,
    /// What a graph without tests leaves out: test files, and tests inside
    /// the files it keeps.
    left: (u64, u64),
    /// Links the graph holds that cannot be: between two Rust crates of
    /// which the first is not built with the second. They are kept out of
    /// everything that is counted and drawn.
    ruled: Vec<Use>,
    files: Vec<String>,
    pairs: Vec<(u32, u32, Pair)>,
    /// Ordered by file: the symbols of file `n` are `first[n]..first[n + 1]`.
    symbols: Vec<Symbol>,
    first: Vec<u32>,
    names: String,
    uses: Vec<Use>,
    kinds: Vec<String>,
    nodes: u64,
    links: u64,
    unplaced: u64,
}
/// The part of `path` that stands for it when `scope` is open: the child of
/// the scope that holds it, or, outside the scope, the first folder where the
/// two part ways. The flag says whether it is inside.
fn group<'a>(scope: &str, path: &'a str) -> (&'a str, bool) {
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
fn place<'a>(scope: &str, spread: &HashSet<&str>, path: &'a str) -> (&'a str, bool) {
    let (mut key, inside) = group(scope, path);
    while inside && key.len() < path.len() && spread.contains(key) {
        let rest = &path[key.len() + 1..];
        key = &path[..rest.find('/').map_or(path.len(), |at| key.len() + 1 + at)];
    }
    (key, inside)
}
fn name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
/// The folder a place lies in; none for what lies in the project itself.
fn holder(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(folder, _)| folder)
}
/// The part of `path` that stands for it among the links followed from a
/// place `depth` folders deep: the folder at that depth that holds it, or the
/// path itself where it lies no deeper.
fn level(path: &str, depth: usize) -> &str {
    match path.match_indices('/').nth(depth.saturating_sub(1)) {
        Some((at, _)) => &path[..at],
        None => path,
    }
}
/// What a place of a picture stands for when a link between two places is
/// looked into: everything under a folder, one file, or one symbol.
#[derive(Clone, Copy)]
enum Reach<'a> {
    Folder(&'a str),
    File(u32),
    Symbol(u32),
}
#[derive(Default)]
struct Tally {
    files: u64,
    folder: bool,
    inside: bool,
    /// What the files of this place hold: classes, methods, functions.
    symbols: u64,
    within: u64,
    out: u64,
    into: u64,
}
#[derive(Clone, Copy, Default)]
struct Flow {
    links: u64,
    guessed: u64,
}
impl Index {
    /// Reads a node-link graph without holding it in memory: nodes become a
    /// table from symbol to file, links are counted as they pass. The
    /// manifests of the tree under `root` say which links cannot be.
    pub fn read(source: impl std::io::Read, root: &Path) -> Result<Self> {
        Self::read_with(source, root, &|path| text_at(root, path))
    }
    /// Reads a graph, with `text` giving a file of its tree where it can be
    /// read.
    fn read_with(
        source: impl std::io::Read,
        root: &Path,
        text: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self> {
        let base = root.to_path_buf();
        let root = root.to_string_lossy().replace('\\', "/");
        let mut builder = Builder {
            root: root.trim_end_matches('/'),
            files: Vec::new(),
            file_ids: HashMap::new(),
            node_symbol: HashMap::new(),
            symbols: Vec::new(),
            names: String::new(),
            uses: Vec::new(),
            kinds: Vec::new(),
            kind_ids: HashMap::new(),
            nodes: 0,
            links: 0,
            unplaced: 0,
            early: Vec::new(),
            listed: false,
        };
        let mut reader = serde_json::Deserializer::from_reader(std::io::BufReader::new(source));
        Whole(&mut builder)
            .deserialize(&mut reader)
            .map_err(|_| HarnessError::invalid("The code graph file is not a node-link graph."))?;
        for link in std::mem::take(&mut builder.early) {
            builder.link(link);
        }
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
        let symbols = order
            .iter()
            .map(|old| {
                let mut symbol = symbols[*old as usize];
                if symbol.parent != NONE {
                    symbol.parent = place[symbol.parent as usize];
                }
                first[symbol.file as usize + 1] += 1;
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
        Ok(Self {
            root: base,
            plain: OnceLock::new(),
            wires: OnceLock::new(),
            left: (0, 0),
            ruled,
            files: builder.files,
            pairs: pairs_of(&symbols, &uses),
            symbols,
            first,
            names: builder.names,
            uses,
            kinds: builder.kinds,
            nodes: builder.nodes,
            links: builder.links,
            unplaced: builder.unplaced,
        })
    }
    fn folder(&self, scope: &str) -> bool {
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
    fn opened(&self, scope: &str) -> Option<u32> {
        let scope = scope.trim_matches('/');
        if scope.is_empty() || self.folder(scope) {
            return None;
        }
        self.files
            .iter()
            .position(|file| file == scope)
            .map(|at| at as u32)
    }
    fn totals(&self) -> Value {
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
    fn crumbs(project: &str, scope: &str) -> Vec<Value> {
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
    /// One folder of the map, or, where the scope names a file, that file.
    /// `certain` leaves out what the tool inferred, `filter` keeps the
    /// children whose name contains it, and `focus` adds what one child, or
    /// one place outside the folder, uses and is used by. A child that is
    /// the focus is drawn whatever the filter and the limits say, and the
    /// view says so (`beyond`) where they would have left it out.
    pub fn view(
        &self,
        project: &str,
        scope: &str,
        focus: Option<&str>,
        certain: bool,
        filter: &str,
    ) -> Value {
        self.view_with(project, scope, &[], focus, certain, filter)
    }
    /// One folder of the map with some of the folders in it spread out:
    /// such a folder is not drawn as one place but as the places it holds,
    /// among the others, so what is around it stays in the picture. A
    /// folder inside a spread-out one may be spread out in turn. What a
    /// request names that the picture does not reach is passed over.
    pub fn view_with(
        &self,
        project: &str,
        scope: &str,
        spread: &[String],
        focus: Option<&str>,
        certain: bool,
        filter: &str,
    ) -> Value {
        if let Some(file) = self.opened(scope) {
            return self.file(project, file, &HashMap::new(), None, focus, certain, filter);
        }
        let scope = scope.trim_matches('/');
        let scope = if self.folder(scope) { scope } else { "" };
        let spread = spread
            .iter()
            .map(|folder| folder.trim_matches('/'))
            .collect::<HashSet<_>>();
        let mut tallies: BTreeMap<&str, Tally> = BTreeMap::new();
        for (id, file) in self.files.iter().enumerate() {
            let (key, inside) = place(scope, &spread, file);
            let tally = tallies.entry(key).or_default();
            tally.files += 1;
            tally.inside = inside;
            tally.folder |= key.len() < file.len();
            tally.symbols += self.holds(id as u32);
        }
        let mut flows: HashMap<(&str, &str), Flow> = HashMap::new();
        let mut kinds: BTreeMap<u16, u64> = BTreeMap::new();
        for (from, to, pair) in &self.pairs {
            let weight = pair.weight(certain);
            if weight == 0 {
                continue;
            }
            let (source, _) = place(scope, &spread, &self.files[*from as usize]);
            let (target, _) = place(scope, &spread, &self.files[*to as usize]);
            if source == target {
                if let Some(tally) = tallies.get_mut(source) {
                    tally.within += weight;
                }
                continue;
            }
            if let Some(tally) = tallies.get_mut(source) {
                tally.out += weight;
            }
            if let Some(tally) = tallies.get_mut(target) {
                tally.into += weight;
            }
            let flow = flows.entry((source, target)).or_default();
            flow.links += weight;
            flow.guessed += if certain { 0 } else { pair.guessed };
            if focus == Some(source) {
                for (kind, count) in &pair.kinds {
                    *kinds.entry(*kind).or_default() += count;
                }
            }
        }
        let needle = filter.trim().to_lowercase();
        let mut children = tallies
            .iter()
            .filter(|(key, tally)| {
                tally.inside && (needle.is_empty() || name(key).to_lowercase().contains(&needle))
            })
            .collect::<Vec<_>>();
        let total = tallies.values().filter(|tally| tally.inside).count();
        let matching = children.len();
        let busiest = |(left, a): &(&&str, &Tally), (right, b): &(&&str, &Tally)| {
            (b.out + b.into + b.within)
                .cmp(&(a.out + a.into + a.within))
                .then(left.cmp(right))
        };
        children.sort_by(busiest);
        // Each folder shows the most linked of what it holds: the open one
        // `SHOWN` of them, a spread-out one `HELD`. What was spread out was
        // asked for, so it is drawn first, and where the picture is full the
        // open folder shows fewer, though never less than `OWN`.
        let mut shown = HashSet::new();
        let mut drawn: HashMap<&str, usize> = HashMap::new();
        for framed in [true, false] {
            let full = if framed { SPREAD - OWN } else { SPREAD };
            for (key, _) in &children {
                let folder = holder(key);
                if (folder != scope) != framed || shown.len() >= full {
                    continue;
                }
                let count = drawn.entry(folder).or_default();
                if *count < if framed { HELD } else { SHOWN } {
                    *count += 1;
                    shown.insert(**key);
                }
            }
        }
        children.retain(|(key, _)| shown.contains(**key));
        // The selected place is drawn whatever the limits and the filter
        // say: it is selected to be seen, and a search of the project leads
        // to places that are not among the most linked of their folder.
        let mut beyond = false;
        if let Some(place) = focus.and_then(|path| tallies.get_key_value(path))
            && place.1.inside
            && shown.insert(*place.0)
        {
            children.push(place);
            children.sort_by(busiest);
            beyond = true;
        }
        // The folders that are spread out and hold something drawn, each
        // with how many places it holds and how many of them are drawn.
        let mut frames: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        for (key, _) in &children {
            let mut folder = holder(key);
            let mut direct = 1;
            while folder != scope {
                frames.entry(folder).or_default().1 += direct;
                direct = 0;
                folder = holder(folder);
            }
        }
        for (key, tally) in &tallies {
            if tally.inside
                && let Some(frame) = frames.get_mut(holder(key))
            {
                frame.0 += 1;
            }
        }
        let mut edges = flows
            .iter()
            .filter(|((source, target), _)| shown.contains(source) && shown.contains(target))
            .collect::<Vec<_>>();
        let between = edges.len();
        edges.sort_by(|((a, b), left), ((c, d), right)| {
            right.links.cmp(&left.links).then((a, b).cmp(&(c, d)))
        });
        // Where nearly everything is linked to everything, all links drawn at
        // once say nothing. Each child then keeps its strongest uses; the
        // focus still lists every link.
        let backbone = between > DENSE * shown.len();
        if backbone {
            let mut kept: HashMap<&str, usize> = HashMap::new();
            edges.retain(|((source, _), _)| {
                let count = kept.entry(*source).or_default();
                *count += 1;
                *count <= STRONGEST
            });
        }
        edges.truncate(DRAWN);
        let side = |outgoing: bool| {
            let Some(focus) = focus else {
                return (Vec::new(), 0);
            };
            let mut rows = flows
                .iter()
                .filter_map(|((source, target), flow)| {
                    let other = if outgoing {
                        (*source == focus).then_some(*target)
                    } else {
                        (*target == focus).then_some(*source)
                    }?;
                    Some((other, *flow))
                })
                .collect::<Vec<_>>();
            rows.sort_by(|(a, left), (b, right)| right.links.cmp(&left.links).then(a.cmp(b)));
            let more = rows.len().saturating_sub(LISTED);
            rows.truncate(LISTED);
            let rows = rows
                .into_iter()
                .map(|(path, flow)| {
                    let tally = tallies.get(path);
                    json!({
                        "path": path,
                        "name": if tally.is_some_and(|tally| tally.inside) { name(path) } else { path },
                        "folder": tally.is_some_and(|tally| tally.folder),
                        "inside": tally.is_some_and(|tally| tally.inside),
                        "links": flow.links,
                        "guessed": flow.guessed,
                    })
                })
                .collect::<Vec<_>>();
            (rows, more)
        };
        let focused = focus.and_then(|path| tallies.get(path).map(|tally| (path, tally)));
        let focus = focused.map(|(path, tally)| {
            let (uses, uses_more) = side(true);
            let (used_by, used_by_more) = side(false);
            let mut kinds = kinds
                .iter()
                .map(|(kind, count)| (self.kinds[*kind as usize].as_str(), *count))
                .collect::<Vec<_>>();
            kinds.sort_by(|(a, left), (b, right)| right.cmp(left).then(a.cmp(b)));
            json!({
                "path": path,
                "name": name(path),
                "folder": tally.folder,
                "inside": tally.inside,
                "files": tally.files,
                "symbols": tally.symbols,
                "within": tally.within,
                "out": tally.out,
                "in": tally.into,
                "uses": uses,
                "uses_more": uses_more,
                "used_by": used_by,
                "used_by_more": used_by_more,
                "kinds": kinds.into_iter().map(|(kind, count)| json!({"kind": kind, "links": count})).collect::<Vec<_>>(),
            })
        });
        json!({
            "scope": scope,
            "crumbs": Self::crumbs(project, scope),
            "certain": certain,
            "totals": self.totals(),
            "children": total,
            "matching": matching,
            "beyond": beyond,
            "nodes": children.iter().map(|(key, tally)| json!({
                "path": key,
                "name": name(key),
                "folder": tally.folder,
                "holder": Some(holder(key)).filter(|folder| *folder != scope),
                "files": tally.files,
                "symbols": tally.symbols,
                "within": tally.within,
                "out": tally.out,
                "in": tally.into,
            })).collect::<Vec<_>>(),
            "frames": frames.iter().map(|(path, (places, drawn))| json!({
                "path": path,
                "name": name(path),
                "holder": Some(holder(path)).filter(|folder| *folder != scope),
                "places": places,
                "drawn": drawn,
            })).collect::<Vec<_>>(),
            "between": between,
            "backbone": backbone,
            "edges": edges.iter().map(|((source, target), flow)| json!({
                "from": source,
                "to": target,
                "links": flow.links,
                "guessed": flow.guessed,
            })).collect::<Vec<_>>(),
            "focus": focus,
        })
    }
    /// What a place stands for: a file or a folder by its path, or a symbol
    /// by the name the picture of its file selects it by.
    fn reach<'a>(&self, place: &'a str) -> Option<Reach<'a>> {
        if let Some(file) = self.files.iter().position(|file| file == place) {
            return Some(Reach::File(file as u32));
        }
        let folder = place.trim_matches('/');
        if !folder.is_empty() && self.folder(folder) {
            return Some(Reach::Folder(folder));
        }
        // A symbol is named after its file: `src/cart.rs#L12:total()`.
        place.match_indices('#').find_map(|(at, _)| {
            let file = self.files.iter().position(|file| *file == place[..at])? as u32;
            let slot = self.keys(file).iter().position(|key| key == place)?;
            Some(Reach::Symbol(self.range(file).start + slot as u32))
        })
    }
    /// One link of a picture, looked into: the names at its two ends, by
    /// file, and every link of the graph between a name of the one end and a
    /// name of the other. `from` uses `to`; each is a folder, a file, or a
    /// symbol of an opened file. `certain` leaves out what the tool inferred.
    /// An end shows its `LINKED` most linked names; the counts are of the
    /// whole link. An end the graph does not know makes the link missing.
    pub fn link(&self, from: &str, to: &str, certain: bool) -> Value {
        let told = |place: &str, reach: Option<Reach>| match reach {
            Some(Reach::Symbol(at)) => json!({
                "path": place,
                "name": self.shown(at),
                "what": "symbol",
                "file": self.files[self.symbols[at as usize].file as usize],
            }),
            Some(Reach::File(_)) => json!({"path": place, "name": name(place), "what": "file"}),
            _ => json!({
                "path": place,
                "name": name(place.trim_matches('/')),
                "what": "folder",
            }),
        };
        let (user, used) = (self.reach(from), self.reach(to));
        let (Some(user), Some(used)) = (user, used) else {
            return json!({
                "mode": "link",
                "missing": true,
                "certain": certain,
                "from": told(from, user),
                "to": told(to, used),
            });
        };
        // The files of an end and, where it is one symbol, that symbol.
        let reached = |reach: Reach| {
            let files = self
                .files
                .iter()
                .enumerate()
                .map(|(id, file)| match reach {
                    Reach::Folder(folder) => file
                        .strip_prefix(folder)
                        .is_some_and(|rest| rest.starts_with('/')),
                    Reach::File(own) => id as u32 == own,
                    Reach::Symbol(symbol) => id as u32 == self.symbols[symbol as usize].file,
                })
                .collect::<Vec<_>>();
            let only = match reach {
                Reach::Symbol(symbol) => Some(symbol),
                _ => None,
            };
            (files, only)
        };
        let (uses, is_used) = (reached(user), reached(used));
        let of = |(files, only): &(Vec<bool>, Option<u32>), at: u32| {
            files[self.symbols[at as usize].file as usize] && only.is_none_or(|only| only == at)
        };
        let links = self
            .uses
            .iter()
            .filter(|(from, to, _, guessed)| {
                !(certain && *guessed) && of(&uses, *from) && of(&is_used, *to)
            })
            .collect::<Vec<_>>();
        let (mut out, mut into): (HashMap<u32, u64>, HashMap<u32, u64>) = Default::default();
        let mut kinds: BTreeMap<u16, u64> = BTreeMap::new();
        let mut guessed = 0u64;
        for (from, to, kind, inferred) in &links {
            *out.entry(*from).or_default() += 1;
            *into.entry(*to).or_default() += 1;
            *kinds.entry(*kind).or_default() += 1;
            guessed += u64::from(*inferred);
        }
        // The names of one end that are shown: the most linked first.
        let chosen = |counts: &HashMap<u32, u64>| {
            let mut names = counts
                .iter()
                .map(|(at, links)| (*at, *links))
                .collect::<Vec<_>>();
            names.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            names.truncate(LINKED);
            names
        };
        let (users, useds) = (chosen(&out), chosen(&into));
        let shown = users
            .iter()
            .chain(&useds)
            .map(|(at, _)| *at)
            .collect::<HashSet<_>>();
        let home = |at: u32| self.symbols[at as usize].file;
        let paths = shown
            .iter()
            .map(|at| self.files[home(*at) as usize].as_str())
            .collect::<Vec<_>>();
        // A name is told as the picture of its file tells it.
        let mut keyed: HashMap<u32, Vec<String>> = HashMap::new();
        let mut key = |at: u32| {
            let file = home(at);
            let keys = keyed.entry(file).or_insert_with(|| self.keys(file));
            keys[(at - self.range(file).start) as usize].clone()
        };
        // One end as it is drawn: its files, the most linked first, each
        // with its names in the order of their lines.
        let mut side = |names: &[(u32, u64)], counts: &HashMap<u32, u64>| {
            let mut groups: BTreeMap<u32, Vec<(u32, u64)>> = BTreeMap::new();
            for (at, links) in names {
                groups.entry(home(*at)).or_default().push((*at, *links));
            }
            let mut groups = groups.into_iter().collect::<Vec<_>>();
            let weight = |names: &[(u32, u64)]| names.iter().map(|(_, links)| links).sum::<u64>();
            groups.sort_by(|a, b| weight(&b.1).cmp(&weight(&a.1)).then(a.0.cmp(&b.0)));
            let files = counts
                .keys()
                .map(|at| home(*at))
                .collect::<HashSet<_>>()
                .len();
            json!({
                "names": counts.len(),
                "files": files,
                "groups": groups.into_iter().map(|(file, mut names)| {
                    names.sort_by_key(|(at, _)| (self.symbols[*at as usize].line, *at));
                    let path = self.files[file as usize].as_str();
                    json!({
                        "path": path,
                        "name": label(path, &paths),
                        "names": names.into_iter().map(|(at, links)| {
                            let symbol = &self.symbols[at as usize];
                            json!({
                                "key": key(at),
                                "name": self.shown(at),
                                "kind": symbol.kind.word(),
                                "line": symbol.line,
                                "owner": self.owner(at),
                                "links": links,
                            })
                        }).collect::<Vec<_>>(),
                    })
                }).collect::<Vec<_>>(),
            })
        };
        let (users, useds) = (side(&users, &out), side(&useds, &into));
        let mut edges = links
            .iter()
            .filter(|(from, to, ..)| shown.contains(from) && shown.contains(to))
            .collect::<Vec<_>>();
        edges.sort_by_key(|(from, to, ..)| (*from, *to));
        edges.truncate(WIRED);
        let mut kinds = kinds
            .iter()
            .map(|(kind, count)| (self.kinds[*kind as usize].as_str(), *count))
            .collect::<Vec<_>>();
        kinds.sort_by(|(a, left), (b, right)| right.cmp(left).then(a.cmp(b)));
        json!({
            "mode": "link",
            "certain": certain,
            "from": told(from, Some(user)),
            "to": told(to, Some(used)),
            "links": links.len(),
            "guessed": guessed,
            "users": users,
            "used": useds,
            "edges": edges.iter().map(|(from, to, kind, inferred)| json!({
                "from": key(*from),
                "to": key(*to),
                "kind": self.kinds[*kind as usize],
                "guessed": inferred,
            })).collect::<Vec<_>>(),
            "kinds": kinds.into_iter().map(|(kind, count)| json!({"kind": kind, "links": count})).collect::<Vec<_>>(),
        })
    }
}
/// The text of a file of the graph's tree: one that lies inside the tree
/// and is no larger than a source worth reading.
fn text_at(root: &Path, path: &str) -> Option<String> {
    if path.split('/').any(|part| part == "..") {
        return None;
    }
    let file = root.join(path);
    std::fs::metadata(&file)
        .is_ok_and(|meta| meta.is_file() && meta.len() <= SOURCE_LIMIT)
        .then(|| std::fs::read_to_string(&file).ok())
        .flatten()
}
/// The links between files: of the links between symbols, those that cross
/// a file boundary, each pair of files once, with how its links were found
/// and of which kinds they are.
fn pairs_of(symbols: &[Symbol], uses: &[Use]) -> Vec<(u32, u32, Pair)> {
    let mut pairs: HashMap<(u32, u32), Pair> = HashMap::new();
    for (from, to, kind, guessed) in uses {
        let (from, to) = (symbols[*from as usize].file, symbols[*to as usize].file);
        if from == to {
            continue;
        }
        let pair = pairs.entry((from, to)).or_default();
        if *guessed {
            pair.guessed += 1;
        } else {
            pair.read += 1;
        }
        match pair.kinds.iter_mut().find(|(known, _)| known == kind) {
            Some((_, count)) => *count += 1,
            None => pair.kinds.push((*kind, 1)),
        }
    }
    let mut pairs = pairs
        .into_iter()
        .map(|((from, to), pair)| (from, to, pair))
        .collect::<Vec<_>>();
    pairs.sort_by_key(|(from, to, _)| (*from, *to));
    pairs
}
/// Whether a folder is one of tests: by its name, or named for tests after
/// something else, as `App.Tests`, `integration-tests`, `unit_tests` and
/// `UnitTests` are.
fn test_folder(part: &str) -> bool {
    if TEST_FOLDERS.contains(&part) {
        return true;
    }
    ["tests", "test"].iter().any(|end| {
        let Some(at) = part.len().checked_sub(end.len()) else {
            return false;
        };
        if !part.is_char_boundary(at) || !part[at..].eq_ignore_ascii_case(end) {
            return false;
        }
        let before = &part[..at];
        before.is_empty()
            || before.ends_with(['.', '-', '_'])
            || (part[at..].starts_with('T')
                && before.ends_with(|letter: char| {
                    letter.is_ascii_lowercase() || letter.is_ascii_digit()
                }))
    })
}
/// Whether a file is a test by where it lies or how it is named, by the
/// conventions of the common languages: in a folder of tests or of what
/// tests are fed with (`tests/`, `__tests__/`, `spec/`, `fixtures/`,
/// `App.Tests/`), or named as `CartTest.php`, `cart.test.ts`,
/// `cart.spec.js`, `test_cart.py`, `cart_test.go`, `conftest.py`,
/// `tests.rs` and `cart_fixture.rs` are.
fn is_test(path: &str) -> bool {
    let (folders, name) = path.rsplit_once('/').unwrap_or(("", path));
    if folders.split('/').any(test_folder) {
        return true;
    }
    let mut dotted = name.split('.');
    let stem = dotted.next().unwrap_or("");
    let rest = dotted.collect::<Vec<_>>();
    // Between the name and its extension: `cart.test.ts`, `app.e2e-spec.ts`.
    let marked = rest.len() > 1
        && rest[..rest.len() - 1].iter().any(|part| {
            ["test", "tests", "spec", "specs", "cy", "e2e"].contains(part)
                || part.ends_with("-spec")
                || part.ends_with("-test")
        });
    marked
        || ["test", "tests", "conftest", "fixture", "fixtures"].contains(&stem)
        || stem.starts_with("test_")
        || ["_test", "_tests", "_spec", "_fixture", "_fixtures"]
            .iter()
            .any(|end| stem.ends_with(end))
        // A capitalised word that ends a name: `CartTest`, `UserFixtures`.
        || ["TestCase", "Tests", "Test", "Fixtures", "Fixture"]
            .iter()
            .any(|end| {
                stem.strip_suffix(end).is_some_and(|before| {
                    before.ends_with(|letter: char| {
                        letter.is_ascii_lowercase() || letter.is_ascii_digit()
                    })
                })
            })
}
/// The attributes a line of Rust begins with, and what follows them on the
/// line: whether one of them marks test code, and the rest of the line.
/// `#[test]`, a test macro of a runtime or a crate (`#[tokio::test]`,
/// `#[rstest]`) and `#[cfg(test)]` with its combinations mark test code;
/// `#[cfg(not(test))]` and a feature that is merely named `test` do not.
fn attributes(line: &str) -> (bool, &str) {
    let mut rest = line.trim_start();
    let mut marked = false;
    while let Some(inner) = rest.strip_prefix("#[") {
        // The attribute ends at the bracket that closes it; brackets inside
        // a string do not count.
        let (mut depth, mut quoted, mut end) = (1usize, false, None);
        for (at, letter) in inner.char_indices() {
            match letter {
                '"' => quoted = !quoted,
                '[' if !quoted => depth += 1,
                ']' if !quoted => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(at);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            // It goes on over the next lines.
            return (marked || marks_test(inner), "");
        };
        marked |= marks_test(&inner[..end]);
        rest = inner[end + 1..].trim_start();
    }
    (marked, rest)
}
/// Whether the inside of a Rust attribute marks what follows as a test.
fn marks_test(inner: &str) -> bool {
    let named = inner
        .find(|letter: char| !(letter.is_ascii_alphanumeric() || letter == '_' || letter == ':'))
        .unwrap_or(inner.len());
    let (name, told) = inner.split_at(named);
    match name.rsplit("::").next().unwrap_or(name) {
        "test" | "rstest" | "test_case" | "bench" | "wasm_bindgen_test" => true,
        "cfg" => {
            // What a string says is a name of a feature, not the word.
            let mut quoted = false;
            let plain = told
                .chars()
                .filter(|letter| {
                    if *letter == '"' {
                        quoted = !quoted;
                    }
                    !quoted && *letter != '"' && *letter != ' '
                })
                .collect::<String>();
            !plain.contains("not(test")
                && plain
                    .split(|letter: char| !(letter.is_ascii_alphanumeric() || letter == '_'))
                    .any(|word| word == "test")
        }
        _ => false,
    }
}
/// Where a comment, a string or a character that begins at byte `at` ends:
/// the byte after its last. None where none of them begins there.
pub(crate) fn quiet_end(bytes: &[u8], at: usize) -> Option<usize> {
    let next = bytes.get(at + 1).copied();
    // Whether what stands before a byte is no part of a name.
    let free =
        |at: usize| at == 0 || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_');
    match bytes[at] {
        b'/' if next == Some(b'/') => Some(
            bytes[at..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |line| at + line),
        ),
        b'/' if next == Some(b'*') => {
            // Block comments nest.
            let mut depth = 1usize;
            let mut end = at + 2;
            while end < bytes.len() && depth > 0 {
                match (bytes[end], bytes.get(end + 1)) {
                    (b'/', Some(b'*')) => {
                        depth += 1;
                        end += 2;
                    }
                    (b'*', Some(b'/')) => {
                        depth -= 1;
                        end += 2;
                    }
                    _ => end += 1,
                }
            }
            Some(end)
        }
        b'"' => {
            let mut end = at + 1;
            while end < bytes.len() && bytes[end] != b'"' {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            Some(end + 1)
        }
        // A raw string: `r"…"`, `r#"…"#`, also of bytes and for C, `br"…"`
        // and `cr"…"`. It is no part of a name, and `r#name` is a name.
        b'r' if free(at) || (matches!(bytes[at - 1], b'b' | b'c') && free(at - 1)) => {
            let hashes = bytes[at + 1..]
                .iter()
                .take_while(|byte| **byte == b'#')
                .count();
            if bytes.get(at + 1 + hashes) != Some(&b'"') {
                return None;
            }
            let mut close = at + 2 + hashes;
            while close < bytes.len()
                && !(bytes[close] == b'"'
                    && bytes[close + 1..]
                        .iter()
                        .take(hashes)
                        .filter(|byte| **byte == b'#')
                        .count()
                        == hashes)
            {
                close += 1;
            }
            Some(close + hashes + 1)
        }
        b'\'' if next == Some(b'\\') => {
            // An escaped character: to the quote that closes it.
            let mut end = at + 3;
            while end < bytes.len() && bytes[end] != b'\'' {
                end += 1;
            }
            Some(end + 1)
        }
        b'\'' => {
            // One character between quotes; else a lifetime.
            let wide = match next {
                Some(byte) if byte >= 0xf0 => 4,
                Some(byte) if byte >= 0xe0 => 3,
                Some(byte) if byte >= 0xc0 => 2,
                _ => 1,
            };
            (bytes.get(at + 1 + wide) == Some(&b'\'')).then_some(at + 2 + wide)
        }
        _ => None,
    }
}
/// Where the Rust item that begins at byte `from` ends: at the `;` that
/// ends it, or at the brace that closes its body. What stands in a string,
/// a character or a comment does not count: a test often holds a source of
/// its own. The last byte of the text where the item is never closed.
fn item_end(text: &str, from: usize) -> usize {
    let bytes = text.as_bytes();
    let (mut braces, mut groups) = (0usize, 0usize);
    let mut at = from;
    while at < bytes.len() {
        if let Some(end) = quiet_end(bytes, at) {
            at = end;
            continue;
        }
        match bytes[at] {
            b'{' => braces += 1,
            b'}' => {
                braces = braces.saturating_sub(1);
                if braces == 0 {
                    return at;
                }
            }
            b'(' | b'[' => groups += 1,
            b')' | b']' => groups = groups.saturating_sub(1),
            b';' if braces == 0 && groups == 0 => return at,
            _ => {}
        }
        at += 1;
    }
    bytes.len().saturating_sub(1)
}
/// The runs of lines of a Rust source that are tests: an item marked as
/// one, most often `#[cfg(test)] mod tests { … }` or a `#[test]` function,
/// from its attribute to where it ends. Lines count from 1.
fn rust_tests(text: &str) -> Vec<(u32, u32)> {
    let rows = lines(text);
    // Where each line begins, to tell the line of a place in the text.
    let mut starts = Vec::with_capacity(rows.len());
    let mut begun = 0;
    for line in text.split_inclusive('\n') {
        starts.push(begun);
        begun += line.len();
    }
    let raw = text.lines().map(str::trim).collect::<Vec<_>>();
    let mut spans = Vec::new();
    // The line of the attribute that marked what comes next.
    let mut marked: Option<usize> = None;
    let mut at = 0;
    while at < raw.len() {
        let (marks, rest) = attributes(raw[at]);
        if marks && marked.is_none() {
            marked = Some(at);
        }
        // The item itself: what follows the attributes of a line, or a
        // line of code. Comments and other attributes may stand between.
        let item = if raw[at].starts_with("#[") {
            rest
        } else if rows[at].kind == Row::Code {
            raw[at]
        } else {
            if rows[at].kind == Row::Closer {
                marked = None;
            }
            ""
        };
        let Some(first) = marked.filter(|_| !item.is_empty()) else {
            if !item.is_empty() {
                marked = None;
            }
            at += 1;
            continue;
        };
        marked = None;
        // The item is a part of the text, so it says where it stands in it.
        let from = item.as_ptr() as usize - text.as_ptr() as usize;
        let close = item_end(text, from);
        let end = starts
            .partition_point(|start| *start <= close)
            .saturating_sub(1)
            .max(at);
        spans.push((first as u32 + 1, end as u32 + 1));
        at = end + 1;
    }
    spans
}
impl Index {
    /// The same graph without its tests: without the files that are tests
    /// by where they lie or how they are named, and, in a Rust source that
    /// can be read, without the symbols of its test modules and test
    /// functions. The links of what is left are counted anew, so a link
    /// that only a test made is gone from its file too. It is made once and
    /// kept with the graph.
    pub fn without_tests(&self) -> Arc<Index> {
        self.plain
            .get_or_init(|| Arc::new(self.plain_with(&|path| text_at(&self.root, path))))
            .clone()
    }
    /// What a graph without tests leaves out: how many files that are
    /// tests, and how many tests inside the files it keeps.
    pub fn left_out(&self) -> (u64, u64) {
        self.left
    }
    /// The graph without its tests, with `source` giving the text of a file
    /// where it can be read.
    fn plain_with(&self, source: &dyn Fn(&str) -> Option<String>) -> Index {
        let gone = self
            .files
            .iter()
            .map(|path| is_test(path))
            .collect::<Vec<_>>();
        // A symbol goes with its file, with the test it is, or with the
        // test that holds it.
        let mut out = vec![false; self.symbols.len()];
        for (file, path) in self.files.iter().enumerate() {
            let own = self.first[file] as usize..self.first[file + 1] as usize;
            if gone[file] {
                out[own].fill(true);
                continue;
            }
            if !path.ends_with(".rs") || own.is_empty() {
                continue;
            }
            let spans = source(path)
                .map(|text| rust_tests(&text))
                .unwrap_or_default();
            for at in own {
                let line = self.symbols[at].line;
                out[at] = line > 0 && spans.iter().any(|(from, to)| (*from..=*to).contains(&line));
            }
        }
        for at in 0..self.symbols.len() {
            let mut holder = self.symbols[at].parent;
            // A graph may hold two symbols in each other; the walk is short.
            for _ in 0..32 {
                if holder == NONE || out[at] {
                    break;
                }
                out[at] = out[holder as usize];
                holder = self.symbols[holder as usize].parent;
            }
        }
        let mut files = Vec::new();
        let mut file_at = vec![NONE; self.files.len()];
        for (file, path) in self.files.iter().enumerate() {
            if !gone[file] {
                file_at[file] = files.len() as u32;
                files.push(path.clone());
            }
        }
        let mut symbols = Vec::new();
        let mut symbol_at = vec![NONE; self.symbols.len()];
        let mut first = vec![0u32; files.len() + 1];
        let mut inside = 0u64;
        for (at, symbol) in self.symbols.iter().enumerate() {
            if out[at] {
                inside += u64::from(!gone[symbol.file as usize]);
                continue;
            }
            symbol_at[at] = symbols.len() as u32;
            first[file_at[symbol.file as usize] as usize + 1] += 1;
            symbols.push(*symbol);
        }
        for file in 0..files.len() {
            first[file + 1] += first[file];
        }
        for symbol in &mut symbols {
            symbol.file = file_at[symbol.file as usize];
            if symbol.parent != NONE {
                symbol.parent = symbol_at[symbol.parent as usize];
            }
        }
        // Of the links, those between what is left, the ones ruled out as
        // the ones that stand.
        let left = |links: &[Use]| {
            links
                .iter()
                .filter(|(from, to, ..)| !out[*from as usize] && !out[*to as usize])
                .map(|(from, to, kind, guessed)| {
                    (
                        symbol_at[*from as usize],
                        symbol_at[*to as usize],
                        *kind,
                        *guessed,
                    )
                })
                .collect::<Vec<Use>>()
        };
        let (uses, ruled) = (left(&self.uses), left(&self.ruled));
        let unlinked = (self.uses.len() - uses.len()) + (self.ruled.len() - ruled.len());
        let dropped = self.symbols.len() - symbols.len();
        Index {
            root: self.root.clone(),
            plain: OnceLock::new(),
            wires: OnceLock::new(),
            left: (gone.iter().filter(|gone| **gone).count() as u64, inside),
            ruled,
            files,
            pairs: pairs_of(&symbols, &uses),
            symbols,
            first,
            names: self.names.clone(),
            nodes: self.nodes.saturating_sub(dropped as u64),
            links: self.links.saturating_sub(unlinked as u64),
            uses,
            kinds: self.kinds.clone(),
            unplaced: self.unplaced,
        }
    }
}
/// A place of the change map: one changed file, a folder of them, or a file
/// that is linked to the change.
#[derive(Default)]
struct Spot {
    files: u64,
    changed: u64,
    status: Option<char>,
    folder: bool,
    /// What a file holds: classes, methods, functions.
    symbols: u64,
    out: u64,
    into: u64,
    /// A file outside the change that uses it, and so may be affected.
    affected: bool,
    /// A file outside the change that the change uses.
    needed: bool,
    /// Changed as well, outside the open folder.
    altered: bool,
}
impl Spot {
    fn role(&self) -> &'static str {
        match (self.changed > 0, self.affected, self.needed) {
            (true, ..) => "changed",
            (_, true, true) => "both",
            (_, true, false) => "affected",
            _ => "needed",
        }
    }
}
/// The shortest run of trailing path parts that no other drawn place ends
/// with: `Cart.php`, or `Order/Cart.php` where two files share the name.
fn label<'a>(path: &'a str, all: &[&str]) -> &'a str {
    let mut start = path.rfind('/').map_or(0, |at| at + 1);
    loop {
        let tail = &path[start..];
        let shared = all.iter().any(|other| {
            *other != path
                && other
                    .strip_suffix(tail)
                    .is_some_and(|rest| rest.is_empty() || rest.ends_with('/'))
        });
        if !shared || start == 0 {
            return tail;
        }
        start = path[..start - 1].rfind('/').map_or(0, |at| at + 1);
    }
}
impl Index {
    /// The dependency map of a change: the changed files, the files they use
    /// and the files that use them. `changed` names each file with Git's
    /// letter for what happened to it. `scope` narrows the change to one
    /// folder, `filter` keeps the linked files whose path contains it, and
    /// `focus` lists both sides of one place.
    ///
    /// Up to `SINGLE` changed files are drawn one by one. A larger change is
    /// drawn as the children of the open folder that hold changed files, and
    /// a folder is opened to reach its files. Linked files are always files,
    /// strongest first. A scope that names a file opens that file, changed
    /// or not.
    pub fn changes(
        &self,
        project: &str,
        changed: &[(String, char)],
        scope: &str,
        focus: Option<&str>,
        certain: bool,
        filter: &str,
    ) -> Value {
        if let Some(view) = self.symbols(project, scope, changed, None, focus, certain, filter) {
            return view;
        }
        let ids = self
            .files
            .iter()
            .enumerate()
            .map(|(id, file)| (file.as_str(), id as u32))
            .collect::<HashMap<_, _>>();
        let within = |scope: &str, path: &str| {
            scope.is_empty()
                || path
                    .strip_prefix(scope)
                    .is_some_and(|rest| rest.starts_with('/'))
        };
        // The changed files the graph knows, with what happened to each.
        let (mut removed, mut unknown) = (0u64, 0u64);
        let mut known = Vec::new();
        for (path, kind) in changed {
            if *kind == 'D' {
                removed += 1;
                continue;
            }
            match ids.get(path.as_str()) {
                Some(id) => known.push((*id, *kind)),
                None => unknown += 1,
            }
        }
        let here = |scope: &str| {
            known
                .iter()
                .map(|(id, _)| self.files[*id as usize].as_str())
                .filter(|path| within(scope, path))
                .collect::<Vec<_>>()
        };
        let mut scope = scope.trim_matches('/');
        if here(scope).is_empty() {
            scope = "";
        }
        // A folder whose changes all lie in one folder below it says no
        // more than that folder does, so it is passed through.
        loop {
            let inside = here(scope);
            let mut below = None;
            let narrow = inside.len() > SINGLE
                && inside.iter().all(|path| {
                    let (child, _) = group(scope, path);
                    child.len() < path.len() && *below.get_or_insert(child) == child
                });
            match below {
                Some(child) if narrow => scope = child,
                _ => break,
            }
        }
        // Inside the open folder is the change drawn here; what changed
        // outside it is told apart.
        let mut status: HashMap<u32, char> = HashMap::new();
        let mut elsewhere = std::collections::HashSet::new();
        for (id, kind) in &known {
            if within(scope, &self.files[*id as usize]) {
                status.insert(*id, *kind);
            } else {
                elsewhere.insert(*id);
            }
        }
        let single = status.len() <= SINGLE;
        let key = |id: u32| {
            let path = self.files[id as usize].as_str();
            if single { path } else { group(scope, path).0 }
        };
        let mut spots: BTreeMap<&str, Spot> = BTreeMap::new();
        for (id, kind) in &status {
            let place = key(*id);
            let spot = spots.entry(place).or_default();
            spot.files += 1;
            spot.changed += 1;
            spot.symbols += self.holds(*id);
            spot.folder |= place.len() < self.files[*id as usize].len();
            // A folder shows how many changed, even when it is one.
            spot.status = (!spot.folder && spot.changed == 1).then_some(*kind);
        }
        let mut flows: HashMap<(&str, &str), Flow> = HashMap::new();
        // Each linked file: how strongly, and on which side of the change.
        let mut near: HashMap<u32, (u64, bool, bool)> = HashMap::new();
        for (from, to, pair) in &self.pairs {
            let weight = pair.weight(certain);
            let (ours, theirs) = (status.contains_key(from), status.contains_key(to));
            if weight == 0 || !(ours || theirs) {
                continue;
            }
            let source = if ours {
                key(*from)
            } else {
                self.files[*from as usize].as_str()
            };
            let target = if theirs {
                key(*to)
            } else {
                self.files[*to as usize].as_str()
            };
            if source == target {
                continue;
            }
            if !ours {
                let entry = near.entry(*from).or_default();
                entry.0 += weight;
                entry.1 = true;
            }
            if !theirs {
                let entry = near.entry(*to).or_default();
                entry.0 += weight;
                entry.2 = true;
            }
            let flow = flows.entry((source, target)).or_default();
            flow.links += weight;
            flow.guessed += if certain { 0 } else { pair.guessed };
        }
        for ((source, target), flow) in &flows {
            if let Some(spot) = spots.get_mut(source) {
                spot.out += flow.links;
            }
            if let Some(spot) = spots.get_mut(target) {
                spot.into += flow.links;
            }
        }
        // A change too wide to draw keeps its busiest places.
        let places = spots.len();
        if places > SHOWN - AROUND {
            let mut order = spots
                .iter()
                .map(|(place, spot)| (spot.changed, spot.out + spot.into, *place))
                .collect::<Vec<_>>();
            order.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(b.2)));
            for (.., place) in order.into_iter().skip(SHOWN - AROUND) {
                spots.remove(place);
            }
        }
        let hidden = places - spots.len();
        let (affected, needed) = near.values().fold((0u64, 0u64), |(users, used), near| {
            (users + u64::from(near.1), used + u64::from(near.2))
        });
        let needle = filter.trim().to_lowercase();
        let linked = near.len();
        let mut around = near
            .iter()
            .map(|(id, near)| (self.files[*id as usize].as_str(), *id, *near))
            .filter(|(path, ..)| needle.is_empty() || path.to_lowercase().contains(&needle))
            .collect::<Vec<_>>();
        let matching = around.len();
        around.sort_by(|a, b| b.2.0.cmp(&a.2.0).then(a.0.cmp(b.0)));
        around.truncate(SHOWN.saturating_sub(spots.len()).max(AROUND));
        for (path, id, (_, uses, used)) in &around {
            spots.insert(
                path,
                Spot {
                    files: 1,
                    symbols: self.holds(*id),
                    affected: *uses,
                    needed: *used,
                    altered: elsewhere.contains(id),
                    ..Spot::default()
                },
            );
        }
        for ((source, target), flow) in &flows {
            if let Some(spot) = spots.get_mut(source)
                && spot.changed == 0
            {
                spot.out += flow.links;
            }
            if let Some(spot) = spots.get_mut(target)
                && spot.changed == 0
            {
                spot.into += flow.links;
            }
        }
        let mut edges = flows
            .iter()
            .filter(|((source, target), _)| {
                spots.contains_key(source) && spots.contains_key(target)
            })
            .collect::<Vec<_>>();
        let between = edges.len();
        edges.sort_by(|((a, b), left), ((c, d), right)| {
            right.links.cmp(&left.links).then((a, b).cmp(&(c, d)))
        });
        edges.truncate(DRAWN);
        let drawn = spots.keys().copied().collect::<Vec<_>>();
        let named = |place: &str, spot: &Spot| {
            if spot.folder {
                name(place).to_owned()
            } else {
                label(place, &drawn).to_owned()
            }
        };
        let mut nodes = spots.iter().collect::<Vec<_>>();
        nodes.sort_by(|(left, a), (right, b)| {
            (b.changed > 0)
                .cmp(&(a.changed > 0))
                .then((b.out + b.into).cmp(&(a.out + a.into)))
                .then(left.cmp(right))
        });
        let focus = focus.and_then(|place| spots.get_key_value(place));
        let focus = focus.map(|(place, spot)| {
            // The files this place stands for, and whether a link's other
            // end counts: any file for a changed place, the change itself
            // for a file beside it.
            let own = |id: &u32| {
                if spot.changed > 0 {
                    status.contains_key(id) && key(*id) == *place
                } else {
                    self.files[*id as usize] == **place
                }
            };
            let counts = |id: &u32| spot.changed > 0 || status.contains_key(id);
            let mut uses: HashMap<u32, Flow> = HashMap::new();
            let mut used: HashMap<u32, Flow> = HashMap::new();
            let mut kinds: BTreeMap<u16, u64> = BTreeMap::new();
            for (from, to, pair) in &self.pairs {
                let weight = pair.weight(certain);
                if weight == 0 {
                    continue;
                }
                let (other, side) = match (own(from), own(to)) {
                    (true, false) => (to, &mut uses),
                    (false, true) => (from, &mut used),
                    _ => continue,
                };
                if !counts(other) {
                    continue;
                }
                let flow = side.entry(*other).or_default();
                flow.links += weight;
                flow.guessed += if certain { 0 } else { pair.guessed };
                if own(from) {
                    for (kind, count) in &pair.kinds {
                        *kinds.entry(*kind).or_default() += count;
                    }
                }
            }
            let side = |rows: HashMap<u32, Flow>| {
                let mut rows = rows
                    .into_iter()
                    .map(|(id, flow)| (self.files[id as usize].as_str(), id, flow))
                    .collect::<Vec<_>>();
                rows.sort_by(|a, b| b.2.links.cmp(&a.2.links).then(a.0.cmp(b.0)));
                let more = rows.len().saturating_sub(LISTED);
                rows.truncate(LISTED);
                let rows = rows
                    .into_iter()
                    .map(|(path, id, flow)| {
                        json!({
                            "path": path,
                            "name": path,
                            "folder": false,
                            "inside": spots.contains_key(path),
                            "links": flow.links,
                            "guessed": flow.guessed,
                            "changed": status.contains_key(&id) || elsewhere.contains(&id),
                        })
                    })
                    .collect::<Vec<_>>();
                (rows, more)
            };
            let (uses, uses_more) = side(uses);
            let (used_by, used_by_more) = side(used);
            let mut kinds = kinds
                .iter()
                .map(|(kind, count)| (self.kinds[*kind as usize].as_str(), *count))
                .collect::<Vec<_>>();
            kinds.sort_by(|(a, left), (b, right)| right.cmp(left).then(a.cmp(b)));
            json!({
                "path": place,
                "name": named(place, spot),
                "folder": spot.folder,
                "inside": true,
                "files": spot.files,
                "symbols": spot.symbols,
                "within": 0,
                "out": spot.out,
                "in": spot.into,
                "changed": spot.changed,
                "status": spot.status.map(String::from),
                "role": spot.role(),
                "altered": spot.altered,
                "uses": uses,
                "uses_more": uses_more,
                "used_by": used_by,
                "used_by_more": used_by_more,
                "kinds": kinds.into_iter().map(|(kind, count)| json!({"kind": kind, "links": count})).collect::<Vec<_>>(),
            })
        });
        json!({
            "mode": "changes",
            "scope": scope,
            "crumbs": Self::crumbs(project, scope),
            "certain": certain,
            "totals": self.totals(),
            "change": {
                "files": changed.len(),
                "graphed": status.len() + elsewhere.len(),
                "here": status.len(),
                "removed": removed,
                "unknown": unknown,
                "single": single,
                "hidden": hidden,
                "affected": affected,
                "needed": needed,
            },
            "children": linked,
            "matching": matching,
            "nodes": nodes.iter().map(|(place, spot)| json!({
                "path": place,
                "name": named(place, spot),
                "folder": spot.folder,
                "files": spot.files,
                "symbols": spot.symbols,
                "within": 0,
                "out": spot.out,
                "in": spot.into,
                "changed": spot.changed,
                "status": spot.status.map(String::from),
                "role": spot.role(),
                "altered": spot.altered,
            })).collect::<Vec<_>>(),
            "between": between,
            "backbone": false,
            "edges": edges.iter().map(|((source, target), flow)| json!({
                "from": source,
                "to": target,
                "links": flow.links,
                "guessed": flow.guessed,
            })).collect::<Vec<_>>(),
            "focus": focus,
        })
    }
}
/// A place among followed links: a file, or a folder standing for the files
/// in it.
#[derive(Default)]
struct Stop {
    files: u64,
    folder: bool,
    /// What a file holds: classes, methods, functions.
    symbols: u64,
    out: u64,
    into: u64,
    /// Files of a chosen change in it, and Git's letter where it is one file.
    changed: u64,
    status: Option<char>,
}
impl Index {
    /// The links of one place followed step by step. The picture starts with
    /// the place alone. `out` names the places whose uses are unfolded and
    /// `into` those whose users are: each time a place is named there, its
    /// `STEP` strongest that are not drawn yet join the picture, up to
    /// `FOLLOWED` places in all. A link is drawn where the side it leaves or
    /// the side it reaches is unfolded, so the picture holds the steps that
    /// were asked for and nothing else; how many more links run between its
    /// places is counted.
    ///
    /// A file is followed among files. A folder is followed among what lies
    /// as deep as itself: the folders of that depth, and the files that lie
    /// no deeper. A class, a method or a function, named as the picture of
    /// its file selects it, is followed among symbols, as `trace` tells.
    /// `changed` marks the files of a chosen change, and `focus` lists both
    /// sides of one place drawn. `None` where `root` names nothing in the
    /// graph.
    #[allow(clippy::too_many_arguments)]
    pub fn follow(
        &self,
        project: &str,
        root: &str,
        out: &[String],
        into: &[String],
        changed: &[(String, char)],
        focus: Option<&str>,
        certain: bool,
    ) -> Option<Value> {
        self.follow_with(project, root, out, into, changed, &|_| None, focus, certain)
    }
    /// The links of one place followed step by step, with `edited` giving
    /// what a chosen change did to one of its files, where that is known:
    /// among symbols it marks the ones the change touched.
    #[allow(clippy::too_many_arguments)]
    pub fn follow_with(
        &self,
        project: &str,
        root: &str,
        out: &[String],
        into: &[String],
        changed: &[(String, char)],
        edited: &dyn Fn(&str) -> Option<Edited>,
        focus: Option<&str>,
        certain: bool,
    ) -> Option<Value> {
        if let Some(Reach::Symbol(at)) = self.reach(root) {
            return Some(self.trace(project, at, out, into, changed, edited, focus, certain));
        }
        let root = root.trim_matches('/');
        let whole = self.opened(root).is_some();
        if root.is_empty() || !(whole || self.folder(root)) {
            return None;
        }
        let depth = if whole {
            usize::MAX
        } else {
            root.split('/').count()
        };
        let letters = changed
            .iter()
            .filter(|(_, kind)| *kind != 'D')
            .map(|(path, kind)| (path.as_str(), *kind))
            .collect::<HashMap<_, _>>();
        let mut stops: HashMap<&str, Stop> = HashMap::new();
        for (id, file) in self.files.iter().enumerate() {
            let place = level(file, depth);
            let stop = stops.entry(place).or_default();
            stop.files += 1;
            stop.folder |= place.len() < file.len();
            stop.symbols += self.holds(id as u32);
            if let Some(kind) = letters.get(file.as_str()) {
                stop.changed += 1;
                stop.status = Some(*kind);
            }
        }
        let mut flows: HashMap<(&str, &str), Flow> = HashMap::new();
        let mut kinds: BTreeMap<u16, u64> = BTreeMap::new();
        for (from, to, pair) in &self.pairs {
            let weight = pair.weight(certain);
            if weight == 0 {
                continue;
            }
            let source = level(&self.files[*from as usize], depth);
            let target = level(&self.files[*to as usize], depth);
            if source == target {
                continue;
            }
            if let Some(stop) = stops.get_mut(source) {
                stop.out += weight;
            }
            if let Some(stop) = stops.get_mut(target) {
                stop.into += weight;
            }
            let flow = flows.entry((source, target)).or_default();
            flow.links += weight;
            flow.guessed += if certain { 0 } else { pair.guessed };
            if focus == Some(source) {
                for (kind, count) in &pair.kinds {
                    *kinds.entry(*kind).or_default() += count;
                }
            }
        }
        // Both sides of every place, strongest first.
        let mut uses: HashMap<&str, Vec<(&str, Flow)>> = HashMap::new();
        let mut users: HashMap<&str, Vec<(&str, Flow)>> = HashMap::new();
        for ((source, target), flow) in &flows {
            uses.entry(*source).or_default().push((*target, *flow));
            users.entry(*target).or_default().push((*source, *flow));
        }
        for rows in uses.values_mut().chain(users.values_mut()) {
            rows.sort_by(|(a, left), (b, right)| right.links.cmp(&left.links).then(a.cmp(b)));
        }
        let root = stops.get_key_value(root).map(|(place, _)| *place)?;
        let times = |list: &[String], place: &str| {
            list.iter().filter(|named| named.as_str() == place).count()
        };
        // Places join in the order they are reached, the followed one first.
        let mut shown = vec![root];
        let mut seen = HashSet::from([root]);
        let mut full = false;
        let mut at = 0;
        while at < shown.len() {
            let place = shown[at];
            at += 1;
            for (list, side) in [(out, &uses), (into, &users)] {
                let mut left = STEP * times(list, place);
                for (other, _) in side.get(place).into_iter().flatten() {
                    if left == 0 {
                        break;
                    }
                    if seen.contains(other) {
                        continue;
                    }
                    if shown.len() == FOLLOWED {
                        full = true;
                        break;
                    }
                    seen.insert(*other);
                    shown.push(*other);
                    left -= 1;
                }
            }
        }
        // A link between two places drawn is drawn where one of its sides
        // is unfolded: the uses of its user, or the users of what it uses.
        let linked = |source: &str, target: &str| {
            seen.contains(source)
                && seen.contains(target)
                && (times(out, source) > 0 || times(into, target) > 0)
        };
        let among = flows
            .keys()
            .filter(|(source, target)| seen.contains(source) && seen.contains(target))
            .count();
        let mut edges = flows
            .iter()
            .filter(|((source, target), _)| linked(source, target))
            .collect::<Vec<_>>();
        let between = edges.len();
        edges.sort_by(|((a, b), left), ((c, d), right)| {
            right.links.cmp(&left.links).then((a, b).cmp(&(c, d)))
        });
        edges.truncate(DRAWN);
        let count = |side: &HashMap<&str, Vec<(&str, Flow)>>, place: &str| {
            side.get(place).map_or(0, Vec::len)
        };
        // How many of what a place uses, or of what uses it, the picture
        // does not link with it: those not drawn, and those drawn whose link
        // waits for a side to be unfolded.
        let hidden = |place: &str, outgoing: bool| {
            let side = if outgoing { &uses } else { &users };
            side.get(place).map_or(0, |rows| {
                rows.iter()
                    .filter(|(other, _)| {
                        if outgoing {
                            !linked(place, other)
                        } else {
                            !linked(other, place)
                        }
                    })
                    .count()
            })
        };
        let node = |place: &str| {
            let stop = &stops[place];
            json!({
                "path": place,
                "name": label(place, &shown),
                "folder": stop.folder,
                "files": stop.files,
                "symbols": stop.symbols,
                "within": 0,
                "out": stop.out,
                "in": stop.into,
                "start": place == root,
                "more_out": hidden(place, true),
                "more_in": hidden(place, false),
                "unfolded_out": times(out, place) > 0,
                "unfolded_in": times(into, place) > 0,
                "changed": stop.changed,
                "status": stop.status.filter(|_| !stop.folder).map(String::from),
            })
        };
        let focus = focus.filter(|path| seen.contains(path)).map(|path| {
            // A listed place is named as a drawn one is: by the end of its
            // path that no other place drawn or listed shares.
            let mut all = shown.clone();
            for side in [uses.get(path), users.get(path)] {
                all.extend(
                    side.into_iter()
                        .flatten()
                        .take(LISTED)
                        .map(|(other, _)| *other),
                );
            }
            let side = |rows: Option<&Vec<(&str, Flow)>>| {
                let rows = rows.map(Vec::as_slice).unwrap_or_default();
                let more = rows.len().saturating_sub(LISTED);
                let rows = rows
                    .iter()
                    .take(LISTED)
                    .map(|(other, flow)| {
                        json!({
                            "path": other,
                            "name": label(other, &all),
                            "folder": stops[*other].folder,
                            "inside": seen.contains(other),
                            "links": flow.links,
                            "guessed": flow.guessed,
                            "changed": stops[*other].changed > 0,
                        })
                    })
                    .collect::<Vec<_>>();
                (rows, more)
            };
            let (using, using_more) = side(uses.get(path));
            let (used_by, used_by_more) = side(users.get(path));
            let mut kinds = kinds
                .iter()
                .map(|(kind, count)| (self.kinds[*kind as usize].as_str(), *count))
                .collect::<Vec<_>>();
            kinds.sort_by(|(a, left), (b, right)| right.cmp(left).then(a.cmp(b)));
            let mut focus = node(path);
            focus["name"] = json!(name(path));
            focus["inside"] = json!(true);
            focus["uses"] = json!(using);
            focus["uses_more"] = json!(using_more);
            focus["used_by"] = json!(used_by);
            focus["used_by_more"] = json!(used_by_more);
            focus["kinds"] = kinds
                .into_iter()
                .map(|(kind, count)| json!({"kind": kind, "links": count}))
                .collect();
            focus
        });
        Some(json!({
            "mode": "follow",
            "scope": root.rsplit_once('/').map_or("", |(holder, _)| holder),
            "crumbs": Self::crumbs(project, root),
            "certain": certain,
            "totals": self.totals(),
            "followed": {
                "path": root,
                "name": name(root),
                "folder": !whole,
                "uses": count(&uses, root),
                "users": count(&users, root),
                "shown": shown.len(),
                "limit": FOLLOWED,
                "step": STEP,
                "full": full,
                "aside": among - between,
            },
            "children": shown.len(),
            "matching": shown.len(),
            "nodes": shown.iter().map(|place| node(place)).collect::<Vec<_>>(),
            "between": between,
            "backbone": false,
            "edges": edges.iter().map(|((source, target), flow)| json!({
                "from": source,
                "to": target,
                "links": flow.links,
                "guessed": flow.guessed,
            })).collect::<Vec<_>>(),
            "focus": focus,
        }))
    }
}
/// One side of a symbol: each symbol it uses, or each that uses it, with
/// the links between the two and the relation of the first of them.
type Side = Rc<Vec<(u32, Flow, u16)>>;
impl Index {
    /// The links of the graph by the symbol at each end.
    fn wires(&self) -> &Wires {
        self.wires.get_or_init(|| {
            let grouped = |end: fn(&Use) -> u32| {
                let mut first = vec![0u32; self.symbols.len() + 1];
                for link in &self.uses {
                    first[end(link) as usize + 1] += 1;
                }
                for at in 0..self.symbols.len() {
                    first[at + 1] += first[at];
                }
                let mut next = first.clone();
                let mut links = vec![0u32; self.uses.len()];
                for (number, link) in self.uses.iter().enumerate() {
                    let slot = &mut next[end(link) as usize];
                    links[*slot as usize] = number as u32;
                    *slot += 1;
                }
                (links, first)
            };
            let (out, out_first) = grouped(|link| link.0);
            let (into, into_first) = grouped(|link| link.1);
            Wires {
                out,
                out_first,
                into,
                into_first,
            }
        })
    }
    /// The links of one symbol: those through which it uses others, or
    /// those through which others use it. `certain` leaves out what the
    /// tool inferred.
    fn wired(&self, at: u32, outgoing: bool, certain: bool) -> impl Iterator<Item = &Use> {
        let wires = self.wires();
        let (links, first) = if outgoing {
            (&wires.out, &wires.out_first)
        } else {
            (&wires.into, &wires.into_first)
        };
        links[first[at as usize] as usize..first[at as usize + 1] as usize]
            .iter()
            .map(|number| &self.uses[*number as usize])
            .filter(move |link| !(certain && link.3))
    }
    /// What a symbol uses, or what uses it: each other symbol once. What
    /// was read from the code comes before what was only guessed, the
    /// symbol's own file before the others, and then the order is that of
    /// the files and of the lines in them.
    fn beside(&self, at: u32, outgoing: bool, certain: bool) -> Vec<(u32, Flow, u16)> {
        let mut found: Vec<(u32, Flow, u16)> = Vec::new();
        let mut places: HashMap<u32, usize> = HashMap::new();
        for (from, to, kind, guessed) in self.wired(at, outgoing, certain) {
            let other = if outgoing { *to } else { *from };
            // A comment the tool kept is never shown.
            if self.symbols[other as usize].kind == Kind::Note {
                continue;
            }
            let place = *places.entry(other).or_insert_with(|| {
                found.push((other, Flow::default(), *kind));
                found.len() - 1
            });
            found[place].1.links += 1;
            found[place].1.guessed += u64::from(*guessed);
        }
        let home = self.symbols[at as usize].file;
        found.sort_by(|a, b| {
            let key = |(other, flow, _): &(u32, Flow, u16)| {
                let symbol = &self.symbols[*other as usize];
                (
                    flow.guessed == flow.links,
                    symbol.file != home,
                    self.files[symbol.file as usize].as_str(),
                    symbol.line,
                    *other,
                )
            };
            key(a).cmp(&key(b))
        });
        found
    }
    /// The links of one symbol followed step by step among symbols: what a
    /// method calls and what calls it, then the same for any of those, as
    /// `follow` does it for files. `at` is the symbol. `out`, `into` and
    /// `focus` name symbols as the pictures of their files select them, and
    /// so are the places of the answer named. Each time a symbol is named
    /// in `out` or `into`, the next `STEP` of that side join the picture, in
    /// the order `beside` gives them, up to `FOLLOWED` in all, and a link is
    /// drawn where one of its sides is unfolded.
    ///
    /// A class is followed by its own links, not by those of its methods.
    /// Inside a chosen change, `edited` tells what the change did to a file,
    /// and the symbols it touched are marked as in the picture of that file.
    #[allow(clippy::too_many_arguments)]
    fn trace(
        &self,
        project: &str,
        at: u32,
        out: &[String],
        into: &[String],
        changed: &[(String, char)],
        edited: &dyn Fn(&str) -> Option<Edited>,
        focus: Option<&str>,
        certain: bool,
    ) -> Value {
        let letters = changed
            .iter()
            .filter(|(_, kind)| *kind != 'D')
            .map(|(path, kind)| (path.as_str(), *kind))
            .collect::<HashMap<_, _>>();
        // The names the pictures of files select their symbols by, the sides
        // of a symbol and what the change did to a file are worked out as
        // they are asked for, each once.
        let named = RefCell::new(HashMap::<u32, Rc<Vec<String>>>::new());
        let key = |at: u32| {
            let file = self.symbols[at as usize].file;
            let keys = named
                .borrow_mut()
                .entry(file)
                .or_insert_with(|| Rc::new(self.keys(file)))
                .clone();
            keys[(at - self.first[file as usize]) as usize].clone()
        };
        let sides = RefCell::new(HashMap::<(u32, bool), Side>::new());
        let side = |at: u32, outgoing: bool| {
            sides
                .borrow_mut()
                .entry((at, outgoing))
                .or_insert_with(|| Rc::new(self.beside(at, outgoing, certain)))
                .clone()
        };
        let marked = RefCell::new(HashMap::<u32, Option<Rc<Marks>>>::new());
        let marks = |file: u32| {
            marked
                .borrow_mut()
                .entry(file)
                .or_insert_with(|| {
                    let path = self.files[file as usize].as_str();
                    letters
                        .contains_key(path)
                        .then(|| edited(path))
                        .flatten()
                        .map(|edited| Rc::new(self.marks(file, &edited)))
                })
                .clone()
        };
        let times = |list: &[String], place: &str| {
            list.iter().filter(|named| named.as_str() == place).count()
        };
        // Symbols join in the order they are reached, the followed one first.
        let mut shown = vec![at];
        let mut keys = vec![key(at)];
        let mut seen = HashSet::from([at]);
        let mut full = false;
        let mut next = 0;
        while next < shown.len() {
            let place = shown[next];
            for (list, outgoing) in [(out, true), (into, false)] {
                let mut left = STEP * times(list, &keys[next]);
                if left == 0 {
                    continue;
                }
                for (other, ..) in side(place, outgoing).iter() {
                    if left == 0 {
                        break;
                    }
                    if seen.contains(other) {
                        continue;
                    }
                    if shown.len() == FOLLOWED {
                        full = true;
                        break;
                    }
                    seen.insert(*other);
                    shown.push(*other);
                    keys.push(key(*other));
                    left -= 1;
                }
            }
            next += 1;
        }
        let slot = shown
            .iter()
            .enumerate()
            .map(|(index, at)| (*at, index))
            .collect::<HashMap<_, _>>();
        let unfolded = |list: &[String]| {
            keys.iter()
                .map(|key| times(list, key) > 0)
                .collect::<Vec<_>>()
        };
        let (unfolded_out, unfolded_in) = (unfolded(out), unfolded(into));
        // A link between two symbols drawn is drawn where one of its sides
        // is unfolded: the uses of its user, or the users of what it uses.
        let mut edges = Vec::new();
        let mut among = 0;
        for (index, source) in shown.iter().enumerate() {
            for (target, flow, _) in side(*source, true).iter() {
                let Some(other) = slot.get(target) else {
                    continue;
                };
                among += 1;
                if unfolded_out[index] || unfolded_in[*other] {
                    edges.push((index, *other, *flow));
                }
            }
        }
        let between = edges.len();
        edges.sort_by(|a, b| {
            b.2.links
                .cmp(&a.2.links)
                .then((&keys[a.0], &keys[a.1]).cmp(&(&keys[b.0], &keys[b.1])))
        });
        edges.truncate(DRAWN);
        // How many of what a symbol uses, or of what uses it, the picture
        // does not link with it.
        let hidden = |index: usize, outgoing: bool| {
            side(shown[index], outgoing)
                .iter()
                .filter(|(other, ..)| match slot.get(other) {
                    None => true,
                    Some(there) if outgoing => !(unfolded_out[index] || unfolded_in[*there]),
                    Some(there) => !(unfolded_out[*there] || unfolded_in[index]),
                })
                .count()
        };
        let links = |at: u32, outgoing: bool| {
            side(at, outgoing)
                .iter()
                .map(|(_, flow, _)| flow.links)
                .sum::<u64>()
        };
        // What the change did to a symbol: Git's letter where its own lines
        // changed, and how many symbols changed inside it.
        let touched = |at: u32| {
            let symbol = &self.symbols[at as usize];
            let place = (at - self.first[symbol.file as usize]) as usize;
            let marks = marks(symbol.file);
            (
                marks.as_ref().and_then(|marks| marks.status[place]),
                marks.as_ref().map_or(0, |marks| marks.inside[place]),
            )
        };
        let node = |index: usize| {
            let at = shown[index];
            let symbol = &self.symbols[at as usize];
            let (status, inside) = touched(at);
            json!({
                "path": keys[index],
                "name": self.shown(at),
                "kind": symbol.kind.word(),
                "line": symbol.line,
                "owner": self.owner(at),
                "file": self.files[symbol.file as usize],
                "symbol": true,
                "folder": false,
                "files": 1,
                "within": 0,
                "out": links(at, true),
                "in": links(at, false),
                "start": index == 0,
                "more_out": hidden(index, true),
                "more_in": hidden(index, false),
                "unfolded_out": unfolded_out[index],
                "unfolded_in": unfolded_in[index],
                "changed": u64::from(status.is_some() || inside > 0),
                "status": status.map(String::from),
                "touched": inside,
            })
        };
        let focus = focus
            .and_then(|path| keys.iter().position(|key| key == path))
            .map(|index| {
                let at = shown[index];
                let rows = |outgoing: bool| {
                    let all = side(at, outgoing);
                    let rows = all
                        .iter()
                        .take(TOLD)
                        .map(|(other, flow, kind)| {
                            let symbol = &self.symbols[*other as usize];
                            let file = self.files[symbol.file as usize].as_str();
                            let (status, inside) = touched(*other);
                            json!({
                                "path": key(*other),
                                "name": self.shown(*other),
                                "kind": symbol.kind.word(),
                                "line": symbol.line,
                                "owner": self.owner(*other),
                                "file": file,
                                "symbol": true,
                                "folder": false,
                                "inside": seen.contains(other),
                                "relation": (flow.links == 1)
                                    .then(|| self.kinds[*kind as usize].as_str()),
                                "links": flow.links,
                                "guessed": flow.guessed,
                                "changed": status.is_some() || inside > 0,
                            })
                        })
                        .collect::<Vec<_>>();
                    (rows, all.len().saturating_sub(TOLD))
                };
                let (uses, uses_more) = rows(true);
                let (used_by, used_by_more) = rows(false);
                let mut kinds: BTreeMap<u16, u64> = BTreeMap::new();
                for (_, _, kind, _) in self.wired(at, true, certain) {
                    *kinds.entry(*kind).or_default() += 1;
                }
                let mut kinds = kinds
                    .iter()
                    .map(|(kind, count)| (self.kinds[*kind as usize].as_str(), *count))
                    .collect::<Vec<_>>();
                kinds.sort_by(|(a, left), (b, right)| right.cmp(left).then(a.cmp(b)));
                let mut focus = node(index);
                focus["inside"] = json!(true);
                focus["uses"] = json!(uses);
                focus["uses_more"] = json!(uses_more);
                focus["used_by"] = json!(used_by);
                focus["used_by_more"] = json!(used_by_more);
                focus["kinds"] = kinds
                    .into_iter()
                    .map(|(kind, count)| json!({"kind": kind, "links": count}))
                    .collect();
                focus
            });
        let symbol = &self.symbols[at as usize];
        let file = self.files[symbol.file as usize].as_str();
        // The way back leads to the file, and the symbol stands after it.
        let mut crumbs = Self::crumbs(project, file);
        crumbs.push(json!({"name": self.shown(at), "path": keys[0]}));
        json!({
            "mode": "follow",
            "scope": file,
            "crumbs": crumbs,
            "certain": certain,
            "totals": self.totals(),
            "followed": {
                "path": keys[0],
                "name": self.shown(at),
                "folder": false,
                "symbol": true,
                "kind": symbol.kind.word(),
                "file": file,
                "line": symbol.line,
                "uses": side(at, true).len(),
                "users": side(at, false).len(),
                "shown": shown.len(),
                "limit": FOLLOWED,
                "step": STEP,
                "full": full,
                "aside": among - between,
            },
            "children": shown.len(),
            "matching": shown.len(),
            "nodes": (0..shown.len()).map(node).collect::<Vec<_>>(),
            "between": between,
            "backbone": false,
            "edges": edges.iter().map(|(source, target, flow)| json!({
                "from": keys[*source],
                "to": keys[*target],
                "links": flow.links,
                "guessed": flow.guessed,
            })).collect::<Vec<_>>(),
            "focus": focus,
        })
    }
}
/// One end of a link drawn in an open file: a symbol of the file, or a file
/// around it.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum End {
    Symbol(u32),
    File(u32),
}
/// What an open file has with one file around it.
#[derive(Clone, Copy, Default)]
struct Near {
    /// Links through which that file uses this one.
    uses: u64,
    /// Links through which this file uses that one.
    used: u64,
}
impl Near {
    fn role(&self) -> &'static str {
        match (self.uses > 0, self.used > 0) {
            (true, true) => "both",
            (true, false) => "uses",
            _ => "used",
        }
    }
}
/// A symbol listed beside a selected one: how it is linked, or, for the
/// symbols of a file around, through how many links.
struct Told {
    symbol: u32,
    relation: Option<u16>,
    links: u64,
    guessed: u64,
}
/// What a change touched in one file, by the place of each symbol among
/// the file's: Git's letter for a symbol whose own lines changed, and how
/// many symbols changed inside it.
struct Marks {
    status: Vec<Option<char>>,
    inside: Vec<u64>,
    /// Places between symbols where lines were only removed.
    between: u64,
}
/// The symbols that start on one line, and the lines that are theirs: from
/// what leads to them to where their indentation ends.
struct Group {
    start: u32,
    lead: u32,
    end: u32,
    from: usize,
    to: usize,
}
/// Whose a line is: nobody's but the file's, the symbols' of one line, or
/// one symbol's that holds the ones nearer to the line.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Owner {
    Top,
    Group(usize),
    One(u32),
}
impl Index {
    /// Tells which symbols of a file a change touched. The graph says where
    /// a symbol starts, not where it ends, so its lines are told from the
    /// text: blank lines, comments and attributes right above a symbol are
    /// its own, and it ends where the text returns to its indentation with
    /// something that does not close it. A line past that end belongs to
    /// what holds the symbol, and past every end to the file itself. A
    /// blank line touches nothing.
    fn marks(&self, file: u32, edited: &Edited) -> Marks {
        let range = self.range(file);
        let slot = |at: u32| (at - range.start) as usize;
        let mut marks = Marks {
            status: vec![None; range.len()],
            inside: vec![0; range.len()],
            between: 0,
        };
        if edited.new {
            for at in range.clone() {
                if self.symbols[at as usize].kind != Kind::Note {
                    marks.status[slot(at)] = Some('A');
                }
            }
        } else {
            let rows = lines(&edited.text);
            let last = rows.len() as u32;
            let row = |line: u32| &rows[line as usize - 1];
            let mut placed = range
                .clone()
                .filter(|at| {
                    let symbol = &self.symbols[*at as usize];
                    !matches!(symbol.kind, Kind::Note | Kind::File)
                        && symbol.line >= 1
                        && symbol.line <= last
                })
                .collect::<Vec<_>>();
            placed.sort_by_key(|at| (self.symbols[*at as usize].line, *at));
            let mut groups: Vec<Group> = Vec::new();
            let mut home = vec![usize::MAX; range.len()];
            for (index, at) in placed.iter().enumerate() {
                let start = self.symbols[*at as usize].line;
                match groups.last_mut() {
                    Some(group) if group.start == start => group.to = index + 1,
                    _ => groups.push(Group {
                        start,
                        lead: start,
                        end: last + 1,
                        from: index,
                        to: index + 1,
                    }),
                }
                home[slot(*at)] = groups.len() - 1;
            }
            for index in 0..groups.len() {
                let start = groups[index].start;
                let floor = if index == 0 {
                    0
                } else {
                    groups[index - 1].start
                };
                let indent = row(start).indent;
                let mut lead = start;
                while lead - 1 > floor
                    && match row(lead - 1).kind {
                        Row::Blank => true,
                        Row::Comment => row(lead - 1).indent <= indent,
                        _ => false,
                    }
                {
                    lead -= 1;
                }
                groups[index].lead = lead;
                // A heading holds whatever follows it.
                let heading = placed[groups[index].from..groups[index].to]
                    .iter()
                    .any(|at| self.symbols[*at as usize].kind == Kind::Heading);
                if heading {
                    continue;
                }
                // Until something deeper was met, a line at the same depth
                // still belongs to the signature.
                let mut body = row(start).whole;
                for line in start + 1..=last {
                    let here = row(line);
                    if matches!(here.kind, Row::Blank | Row::Comment) || here.indent > indent {
                        body |= here.indent > indent && here.kind != Row::Blank;
                        continue;
                    }
                    if here.indent == indent && (!body || here.kind == Row::Closer) {
                        continue;
                    }
                    groups[index].end = line;
                    break;
                }
            }
            let owner = |line: u32| {
                let Some(index) = groups
                    .partition_point(|group| group.lead <= line)
                    .checked_sub(1)
                else {
                    return Owner::Top;
                };
                let group = &groups[index];
                if line < group.end {
                    return Owner::Group(index);
                }
                let mut next = self.symbols[placed[group.from] as usize].parent;
                let mut hops = 0;
                while next != NONE && range.contains(&next) && hops < 16 {
                    hops += 1;
                    let at = home[slot(next)];
                    if at != usize::MAX && groups[at].lead <= line && line < groups[at].end {
                        return Owner::One(next);
                    }
                    next = self.symbols[next as usize].parent;
                }
                Owner::Top
            };
            // 1 for a line that stands in place of removed ones, 2 for a
            // line that was only added.
            let mut fresh = vec![0u8; rows.len() + 1];
            for hunk in &edited.hunks {
                for line in hunk.at..hunk.at.saturating_add(hunk.added) {
                    if line >= 1 && line <= last {
                        fresh[line as usize] = if hunk.removed == 0 { 2 } else { 1 };
                    }
                }
            }
            let mut hit = vec![false; range.len()];
            let mut top = false;
            let mut touch = |owner: Owner| match owner {
                Owner::Top => top = true,
                Owner::Group(index) => {
                    for at in &placed[groups[index].from..groups[index].to] {
                        hit[slot(*at)] = true;
                    }
                }
                Owner::One(at) => hit[slot(at)] = true,
            };
            for hunk in &edited.hunks {
                if hunk.added > 0 {
                    for line in hunk.at..hunk.at.saturating_add(hunk.added) {
                        if line >= 1 && line <= last && row(line).kind != Row::Blank {
                            touch(owner(line));
                        }
                    }
                } else if !hunk.blank {
                    // Lines were only removed: from a symbol when the lines
                    // on both sides of the gap are its own.
                    let before = match hunk.at {
                        0 => Owner::Top,
                        line => owner(line.min(last)),
                    };
                    let after = if hunk.at < last {
                        owner(hunk.at + 1)
                    } else {
                        before
                    };
                    if before == after {
                        touch(before);
                    } else {
                        marks.between += 1;
                    }
                }
            }
            for (index, group) in groups.iter().enumerate() {
                // New when its first line and every other line of its own,
                // down to the next symbol, were only added.
                let limit = groups
                    .get(index + 1)
                    .map_or(last + 1, |next| next.lead)
                    .min(group.end);
                let new = (group.start..limit)
                    .all(|line| row(line).kind == Row::Blank || fresh[line as usize] == 2);
                for at in &placed[group.from..group.to] {
                    if hit[slot(*at)] {
                        marks.status[slot(*at)] = Some(if new { 'A' } else { 'M' });
                    }
                }
            }
            if top {
                for at in range.clone() {
                    if self.symbols[at as usize].kind == Kind::File {
                        marks.status[slot(at)] = Some('M');
                    }
                }
            }
        }
        for at in range.clone() {
            if marks.status[slot(at)].is_none() || self.symbols[at as usize].kind == Kind::File {
                continue;
            }
            let mut next = self.symbols[at as usize].parent;
            let mut hops = 0;
            while next != NONE && next != at && range.contains(&next) && hops < 16 {
                hops += 1;
                marks.inside[slot(next)] += 1;
                next = self.symbols[next as usize].parent;
            }
        }
        marks
    }
    fn label(&self, at: u32) -> &str {
        let symbol = &self.symbols[at as usize];
        &self.names[symbol.at as usize..symbol.at as usize + symbol.len as usize]
    }
    fn range(&self, file: u32) -> std::ops::Range<u32> {
        self.first[file as usize]..self.first[file as usize + 1]
    }
    /// How many classes, methods, functions and other names a file holds.
    fn holds(&self, file: u32) -> u64 {
        self.range(file)
            .filter(|at| !matches!(self.symbols[*at as usize].kind, Kind::File | Kind::Note))
            .count() as u64
    }
    /// A symbol as it is shown: a method with its class, `Cart.total()`, and
    /// the node of the file itself under the file's name.
    fn shown(&self, at: u32) -> String {
        let mut text = String::new();
        self.show(at, &mut text);
        text
    }
    /// Writes a symbol as it is shown after what `into` holds.
    fn show(&self, at: u32, into: &mut String) {
        let symbol = &self.symbols[at as usize];
        let label = self.label(at);
        match symbol.kind {
            Kind::File => into.push_str(name(&self.files[symbol.file as usize])),
            Kind::Method if symbol.parent != NONE => {
                into.push_str(self.label(symbol.parent));
                if !label.starts_with('.') {
                    into.push('.');
                }
                into.push_str(label);
            }
            _ => into.push_str(label),
        }
    }
    /// What a symbol lies in, outermost first, as one name: `Cart.total()`
    /// for a function declared inside that method.
    fn owner(&self, at: u32) -> Option<String> {
        let mut holders = Vec::new();
        let mut next = self.symbols[at as usize].parent;
        // A graph can name two symbols as each other's holder.
        while next != NONE && next != at && !holders.contains(&next) && holders.len() < 16 {
            holders.push(next);
            next = self.symbols[next as usize].parent;
        }
        let parts = holders
            .iter()
            .map(|holder| self.label(*holder))
            .collect::<Vec<_>>();
        let mut text = String::new();
        for part in parts.iter().rev() {
            if !text.is_empty() && !part.starts_with('.') {
                text.push_str(" › ");
            }
            text.push_str(part);
        }
        (!text.is_empty()).then_some(text)
    }
    /// The names the symbols of a file are selected by, in their order: the
    /// file, the line and the label, with a number where two share all three.
    fn keys(&self, file: u32) -> Vec<String> {
        let path = &self.files[file as usize];
        let mut seen: HashMap<String, u32> = HashMap::new();
        self.range(file)
            .map(|at| {
                let (line, label) = (self.symbols[at as usize].line, self.label(at));
                let mut key = if line > 0 {
                    format!("{path}#L{line}:{label}")
                } else {
                    format!("{path}#{label}")
                };
                let met = seen.entry(key.clone()).or_default();
                *met += 1;
                if *met > 1 {
                    key.push_str(&format!("~{met}"));
                }
                key
            })
            .collect()
    }
    /// One file of the map: the classes, methods and functions the graph
    /// names in it, the links between them, and the files they use or are
    /// used by. `changed` marks the files of a change among those around,
    /// `edited` the symbols that change touched here, `filter` keeps the
    /// symbols whose name contains it, and `focus` lists both sides of one
    /// symbol, or what one file around has with this one.
    ///
    /// A graph links two symbols at most once, so a link between two symbols
    /// counts one, and a link with a file around counts the pairs of symbols
    /// it stands for.
    #[allow(clippy::too_many_arguments)]
    fn file(
        &self,
        project: &str,
        file: u32,
        changed: &HashMap<&str, char>,
        edited: Option<&Edited>,
        focus: Option<&str>,
        certain: bool,
        filter: &str,
    ) -> Value {
        let path = self.files[file as usize].as_str();
        let range = self.range(file);
        let own = |at: u32| range.contains(&at);
        let slot = |at: u32| (at - range.start) as usize;
        let keys = self.keys(file);
        let marks = edited.map(|edited| self.marks(file, edited));
        // What happened to a symbol's own lines, and how many symbols
        // changed inside it.
        let status = |at: u32| marks.as_ref().and_then(|marks| marks.status[slot(at)]);
        let inside = |at: u32| marks.as_ref().map_or(0, |marks| marks.inside[slot(at)]);
        let touched = |at: u32| status(at).is_some() || inside(at) > 0;
        let mut out = vec![0u64; range.len()];
        let mut into = vec![0u64; range.len()];
        // Links between two symbols here, links that leave the file and
        // links that enter it.
        let (mut inner, mut leaving, mut entering) = (Vec::new(), Vec::new(), Vec::new());
        for link in &self.uses {
            let (from, to, _, guessed) = *link;
            if certain && guessed {
                continue;
            }
            match (own(from), own(to)) {
                (true, true) => {
                    out[slot(from)] += 1;
                    into[slot(to)] += 1;
                    inner.push(*link);
                }
                (true, false) => {
                    out[slot(from)] += 1;
                    leaving.push(*link);
                }
                (false, true) => {
                    into[slot(to)] += 1;
                    entering.push(*link);
                }
                (false, false) => {}
            }
        }
        let mut members = vec![0u64; range.len()];
        let (mut types, mut methods, mut functions) = (0u64, 0u64, 0u64);
        let (mut headings, mut others) = (0u64, 0u64);
        for at in range.clone() {
            let symbol = &self.symbols[at as usize];
            match symbol.kind {
                Kind::File | Kind::Note => continue,
                Kind::Type => types += 1,
                Kind::Method => methods += 1,
                Kind::Function => functions += 1,
                Kind::Heading => headings += 1,
                Kind::Case | Kind::Name => others += 1,
            }
            if symbol.parent != NONE && own(symbol.parent) {
                members[slot(symbol.parent)] += 1;
            }
        }
        let weight = |at: u32| out[slot(at)] + into[slot(at)];
        let needle = filter.trim().to_lowercase();
        let mut listed = range
            .clone()
            .filter(|at| match self.symbols[*at as usize].kind {
                Kind::Note => false,
                // The file's own node is worth a box when something is
                // linked to it: its imports, or what imports the file.
                Kind::File => weight(*at) > 0,
                _ => true,
            })
            .collect::<Vec<_>>();
        let children = listed.len();
        // A filter chooses among the symbols, and the file's own node is
        // none: in a file named after its class it would match every time.
        listed.retain(|at| {
            needle.is_empty()
                || (self.symbols[*at as usize].kind != Kind::File
                    && self.shown(*at).to_lowercase().contains(&needle))
        });
        let matching = listed.len();
        // What the change touched comes first, then the most linked.
        listed.sort_by(|a, b| {
            touched(*b)
                .cmp(&touched(*a))
                .then(weight(*b).cmp(&weight(*a)))
                .then(
                    self.symbols[*a as usize]
                        .line
                        .cmp(&self.symbols[*b as usize].line),
                )
                .then(a.cmp(b))
        });
        listed.truncate(NAMED);
        // The selected symbol is drawn whatever the limit and the filter say.
        let chosen = focus.and_then(|focus| {
            range.clone().find(|at| {
                keys[slot(*at)] == focus && self.symbols[*at as usize].kind != Kind::Note
            })
        });
        let mut beyond = false;
        if let Some(at) = chosen
            && !listed.contains(&at)
        {
            listed.push(at);
            beyond = true;
        }
        let mut drawn = vec![false; range.len()];
        for at in &listed {
            drawn[slot(*at)] = true;
        }
        // The files around are those the drawn symbols are linked with.
        let mut flows: HashMap<(End, End), Flow> = HashMap::new();
        let mut near: HashMap<u32, Near> = HashMap::new();
        let mut flow = |from: End, to: End, guessed: bool| {
            let flow = flows.entry((from, to)).or_default();
            flow.links += 1;
            flow.guessed += u64::from(guessed);
        };
        for (from, to, _, guessed) in &inner {
            if drawn[slot(*from)] && drawn[slot(*to)] {
                flow(End::Symbol(*from), End::Symbol(*to), *guessed);
            }
        }
        for (from, to, _, guessed) in &leaving {
            if drawn[slot(*from)] {
                let other = self.symbols[*to as usize].file;
                near.entry(other).or_default().used += 1;
                flow(End::Symbol(*from), End::File(other), *guessed);
            }
        }
        for (from, to, _, guessed) in &entering {
            if drawn[slot(*to)] {
                let other = self.symbols[*from as usize].file;
                near.entry(other).or_default().uses += 1;
                flow(End::File(other), End::Symbol(*to), *guessed);
            }
        }
        let linked = near.len();
        let mut around = near
            .iter()
            .map(|(id, near)| (self.files[*id as usize].as_str(), *id, *near))
            .collect::<Vec<_>>();
        around.sort_by(|a, b| {
            (b.2.uses + b.2.used)
                .cmp(&(a.2.uses + a.2.used))
                .then(a.0.cmp(b.0))
        });
        around.truncate(SHOWN.saturating_sub(listed.len()).max(AROUND));
        let beside = around.iter().map(|(_, id, _)| *id).collect::<HashSet<_>>();
        let place = |end: &End| match end {
            End::Symbol(at) => keys[slot(*at)].as_str(),
            End::File(id) => self.files[*id as usize].as_str(),
        };
        let mut edges = flows
            .iter()
            .filter(|((from, to), _)| {
                [from, to].iter().all(|end| match end {
                    End::Symbol(_) => true,
                    End::File(id) => beside.contains(id),
                })
            })
            .map(|((from, to), flow)| (place(from), place(to), *flow))
            .collect::<Vec<_>>();
        let between = edges.len();
        edges.sort_by(|a, b| b.2.links.cmp(&a.2.links).then((a.0, a.1).cmp(&(b.0, b.1))));
        edges.truncate(DRAWN);
        let mut paths = around.iter().map(|(path, ..)| *path).collect::<Vec<_>>();
        paths.push(path);
        let mut nodes = listed
            .iter()
            .map(|at| {
                let symbol = &self.symbols[*at as usize];
                json!({
                    "path": keys[slot(*at)],
                    "name": self.shown(*at),
                    "kind": symbol.kind.word(),
                    "line": symbol.line,
                    "owner": self.owner(*at),
                    "members": members[slot(*at)],
                    "symbol": true,
                    "folder": false,
                    "files": 1,
                    "within": 0,
                    "out": out[slot(*at)],
                    "in": into[slot(*at)],
                    "changed": u64::from(touched(*at)),
                    "status": status(*at).map(String::from),
                    "touched": inside(*at),
                })
            })
            .collect::<Vec<_>>();
        nodes.extend(around.iter().map(|(other, id, near)| {
            json!({
                "path": other,
                "name": label(other, &paths),
                "kind": "file",
                "symbol": false,
                "folder": false,
                "files": 1,
                "symbols": self.holds(*id),
                "within": 0,
                "out": near.uses,
                "in": near.used,
                "role": near.role(),
                "changed": u64::from(changed.contains_key(other)),
                "status": changed.get(other).map(|kind| kind.to_string()),
            })
        }));
        // The rows of the side panel name symbols of other files too, by the
        // names those files select them by.
        let relations = |links: &[&Use]| {
            let mut kinds: BTreeMap<u16, u64> = BTreeMap::new();
            for (_, _, kind, _) in links {
                *kinds.entry(*kind).or_default() += 1;
            }
            let mut kinds = kinds
                .iter()
                .map(|(kind, count)| (self.kinds[*kind as usize].as_str(), *count))
                .collect::<Vec<_>>();
            kinds.sort_by(|(a, left), (b, right)| right.cmp(left).then(a.cmp(b)));
            kinds
                .into_iter()
                .map(|(kind, count)| json!({"kind": kind, "links": count}))
                .collect::<Vec<_>>()
        };
        let rows = |mut told: Vec<Told>| {
            let more = told.len().saturating_sub(TOLD);
            told.truncate(TOLD);
            let mut foreign: HashMap<u32, Vec<String>> = HashMap::new();
            let rows = told
                .iter()
                .map(|told| {
                    let symbol = &self.symbols[told.symbol as usize];
                    let home = symbol.file;
                    let key = if own(told.symbol) {
                        keys[slot(told.symbol)].clone()
                    } else {
                        let keys = foreign.entry(home).or_insert_with(|| self.keys(home));
                        keys[(told.symbol - self.first[home as usize]) as usize].clone()
                    };
                    let elsewhere = (home != file).then(|| self.files[home as usize].as_str());
                    json!({
                        "path": key,
                        "name": self.shown(told.symbol),
                        "kind": symbol.kind.word(),
                        "line": symbol.line,
                        "owner": self.owner(told.symbol),
                        "file": elsewhere,
                        "symbol": true,
                        "folder": false,
                        "inside": home == file,
                        "relation": told.relation.map(|kind| self.kinds[kind as usize].as_str()),
                        "links": told.links,
                        "guessed": told.guessed,
                        "changed": match elsewhere {
                            Some(other) => changed.contains_key(other),
                            None => touched(told.symbol),
                        },
                    })
                })
                .collect::<Vec<_>>();
            (rows, more)
        };
        // What one symbol is linked with, a row per symbol: what was read
        // first, this file's before the others'.
        let single = |links: Vec<(u32, &Use)>| {
            let mut told = links
                .into_iter()
                .map(|(symbol, link)| Told {
                    symbol,
                    relation: Some(link.2),
                    links: 1,
                    guessed: u64::from(link.3),
                })
                .collect::<Vec<_>>();
            told.sort_by(|a, b| {
                let key = |told: &Told| {
                    let symbol = &self.symbols[told.symbol as usize];
                    (
                        told.guessed,
                        symbol.file != file,
                        self.files[symbol.file as usize].as_str(),
                        symbol.line,
                        told.symbol,
                    )
                };
                key(a).cmp(&key(b))
            });
            told
        };
        // The symbols of a file around that the links with this file end
        // at, the most linked first.
        let grouped = |ends: Vec<(u32, bool)>| {
            let mut counted: BTreeMap<u32, (u64, u64)> = BTreeMap::new();
            for (symbol, guessed) in ends {
                let entry = counted.entry(symbol).or_default();
                entry.0 += 1;
                entry.1 += u64::from(guessed);
            }
            let mut told = counted
                .into_iter()
                .map(|(symbol, (links, guessed))| Told {
                    symbol,
                    relation: None,
                    links,
                    guessed,
                })
                .collect::<Vec<_>>();
            told.sort_by(|a, b| {
                b.links
                    .cmp(&a.links)
                    .then(
                        self.symbols[a.symbol as usize]
                            .line
                            .cmp(&self.symbols[b.symbol as usize].line),
                    )
                    .then(a.symbol.cmp(&b.symbol))
            });
            told
        };
        let focus = match (chosen, focus) {
            (Some(at), _) => {
                let symbol = &self.symbols[at as usize];
                let uses = inner
                    .iter()
                    .chain(&leaving)
                    .filter(|link| link.0 == at)
                    .collect::<Vec<_>>();
                let used = inner
                    .iter()
                    .chain(&entering)
                    .filter(|link| link.1 == at)
                    .collect::<Vec<_>>();
                let kinds = relations(&uses);
                let (uses, uses_more) = rows(single(
                    uses.into_iter().map(|link| (link.1, link)).collect(),
                ));
                let (used_by, used_by_more) = rows(single(
                    used.into_iter().map(|link| (link.0, link)).collect(),
                ));
                let mut held = range
                    .clone()
                    .filter(|other| {
                        self.symbols[*other as usize].parent == at
                            && self.symbols[*other as usize].kind != Kind::Note
                    })
                    .collect::<Vec<_>>();
                held.sort_by_key(|other| (self.symbols[*other as usize].line, *other));
                let held_more = held.len().saturating_sub(TOLD);
                held.truncate(TOLD);
                Some(json!({
                    "path": keys[slot(at)],
                    "name": self.shown(at),
                    "kind": symbol.kind.word(),
                    "line": symbol.line,
                    "owner": self.owner(at),
                    "file": path,
                    "symbol": true,
                    "folder": false,
                    "inside": true,
                    "out": out[slot(at)],
                    "in": into[slot(at)],
                    "changed": u64::from(touched(at)),
                    "status": status(at).map(String::from),
                    "touched": inside(at),
                    "members": held.iter().map(|other| json!({
                        "path": keys[slot(*other)],
                        "name": self.label(*other),
                        "kind": self.symbols[*other as usize].kind.word(),
                        "line": self.symbols[*other as usize].line,
                        "symbol": true,
                        "folder": false,
                        "inside": true,
                        "changed": touched(*other),
                    })).collect::<Vec<_>>(),
                    "members_more": held_more,
                    "uses": uses,
                    "uses_more": uses_more,
                    "used_by": used_by,
                    "used_by_more": used_by_more,
                    "kinds": kinds,
                }))
            }
            (None, Some(other)) => {
                around
                    .iter()
                    .find(|(place, ..)| *place == other)
                    .map(|(other, id, near)| {
                        let theirs = |at: u32| self.symbols[at as usize].file == *id;
                        // Its symbols that use this file, and its symbols that
                        // this file uses.
                        let users = entering
                            .iter()
                            .filter(|link| theirs(link.0))
                            .collect::<Vec<_>>();
                        let kinds = relations(&users);
                        let (uses, uses_more) = rows(grouped(
                            users.into_iter().map(|link| (link.0, link.3)).collect(),
                        ));
                        let (used_by, used_by_more) = rows(grouped(
                            leaving
                                .iter()
                                .filter(|link| theirs(link.1))
                                .map(|link| (link.1, link.3))
                                .collect(),
                        ));
                        json!({
                            "path": other,
                            "name": label(other, &paths),
                            "kind": "file",
                            "symbol": false,
                            "folder": false,
                            "inside": true,
                            "files": 1,
                            "symbols": self.holds(*id),
                            "out": near.uses,
                            "in": near.used,
                            "role": near.role(),
                            "changed": u64::from(changed.contains_key(other)),
                            "status": changed.get(other).map(|kind| kind.to_string()),
                            "uses": uses,
                            "uses_more": uses_more,
                            "used_by": used_by,
                            "used_by_more": used_by_more,
                            "kinds": kinds,
                        })
                    })
            }
            _ => None,
        };
        let files = |links: &[Use], end: fn(&Use) -> u32| {
            links
                .iter()
                .map(|link| self.symbols[end(link) as usize].file)
                .collect::<HashSet<_>>()
                .len()
        };
        // How much of the file the change touched, where that is known.
        let reach = marks.as_ref().map(|marks| {
            let of = |letter: char| {
                range
                    .clone()
                    .filter(|at| {
                        self.symbols[*at as usize].kind != Kind::File
                            && marks.status[slot(*at)] == Some(letter)
                    })
                    .count()
            };
            let (added, modified) = (of('A'), of('M'));
            json!({
                "symbols": added + modified,
                "added": added,
                "modified": modified,
                "between": marks.between,
                "top": range.clone().any(|at| {
                    self.symbols[at as usize].kind == Kind::File
                        && marks.status[slot(at)].is_some()
                }),
            })
        });
        json!({
            "mode": "file",
            "scope": path,
            "crumbs": Self::crumbs(project, path),
            "certain": certain,
            "totals": self.totals(),
            "file": {
                "path": path,
                "name": name(path),
                "status": changed.get(path).map(|kind| kind.to_string()),
                "touched": reach,
                "symbols": types + methods + functions + headings + others,
                "types": types,
                "methods": methods,
                "functions": functions,
                "headings": headings,
                "others": others,
                "inside": inner.len(),
                "out": leaving.len(),
                "in": entering.len(),
                "uses": files(&leaving, |link| link.1),
                "used_by": files(&entering, |link| link.0),
            },
            "children": children,
            "matching": matching,
            "beyond": beyond,
            "around": {"files": linked, "shown": around.len()},
            "nodes": nodes,
            "between": between,
            "backbone": false,
            "edges": edges.iter().map(|(from, to, flow)| json!({
                "from": from,
                "to": to,
                "links": flow.links,
                "guessed": flow.guessed,
            })).collect::<Vec<_>>(),
            "focus": focus,
        })
    }
}
/// How well a word of a search names a place, the best first: it is the
/// whole name, the name starts with it, a word inside the name starts with
/// it, or it only stands somewhere in the name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Named {
    Whole,
    Start,
    Word,
    Part,
}
/// Writes `text` in lower case after what `into` holds: letter by letter,
/// so that a path and the name it ends in stay alike.
fn lower_onto(text: &str, into: &mut String) {
    if text.is_ascii() {
        let from = into.len();
        into.push_str(text);
        into[from..].make_ascii_lowercase();
    } else {
        into.extend(text.chars().flat_map(char::to_lowercase));
    }
}
/// Whether a word begins at byte `at` of a name whose lower case is `lower`:
/// after what is neither a letter nor a digit, or where a capital begins a
/// word of a name written `likeThis`.
fn begins(name: &str, lower: &str, at: usize) -> bool {
    let Some(before) = lower[..at].chars().next_back() else {
        return true;
    };
    if !before.is_alphanumeric() {
        return true;
    }
    // Capitals are told in the name as it is written, which lies letter for
    // letter over its lower case only where the two are equally long.
    if name.len() != lower.len() || !name.is_char_boundary(at) {
        return false;
    }
    let mut after = name[at..].chars();
    let (Some(before), Some(here)) = (name[..at].chars().next_back(), after.next()) else {
        return false;
    };
    here.is_uppercase()
        && (before.is_lowercase()
            || before.is_numeric()
            || after.next().is_some_and(char::is_lowercase))
}
/// How well `word` names `name`, whose lower case is `lower`; the word is
/// in lower case too. `None` where the name does not hold the word.
fn named(name: &str, lower: &str, word: &str) -> Option<Named> {
    if word.is_empty() {
        return None;
    }
    let mut best: Option<Named> = None;
    let mut from = 0;
    while let Some(found) = lower[from..].find(word) {
        let at = from + found;
        let how = if at > 0 {
            if begins(name, lower, at) {
                Named::Word
            } else {
                Named::Part
            }
        } else if lower.len() == word.len() {
            Named::Whole
        } else {
            Named::Start
        };
        best = Some(best.map_or(how, |best| best.min(how)));
        // Further on, the word can at best begin a word again.
        if how <= Named::Word {
            break;
        }
        from = at + lower[at..].chars().next().map_or(1, char::len_utf8);
    }
    best
}
/// How well a word with a slash names a place by its path, both in lower
/// case: what stands before the word's last slash has to end the folder the
/// place lies in, and the rest to begin the place's name. `src/cart` names
/// `lib/src/Cart.php` and a folder `lib/src/cart`, and `src/` all that lies
/// in a folder `src`. A slash at the front is the start of a folder's name.
fn pathed(path: &str, word: &str) -> Option<Named> {
    let (folder, name) = path.rsplit_once('/').unwrap_or(("", path));
    let (head, tail) = word.rsplit_once('/')?;
    let lies = folder.ends_with(head) || head.strip_prefix('/') == Some(folder);
    if !lies || !name.starts_with(tail) {
        return None;
    }
    Some(if tail.is_empty() {
        Named::Part
    } else if tail.len() == name.len() {
        Named::Whole
    } else {
        Named::Start
    })
}
/// How well a word names a place by its name together with what holds it,
/// as `cart.to` names `Cart.total()`: the word has to begin in `held`, which
/// stands before the name, and to end in the name. All are in lower case.
fn spans(held: &str, lower: &str, word: &str) -> Option<Named> {
    if held.is_empty() {
        return None;
    }
    if let Some(rest) = word.strip_prefix(held)
        && !rest.is_empty()
        && lower.starts_with(rest)
    {
        return Some(Named::Start);
    }
    word.char_indices()
        .skip(1)
        .any(|(at, _)| held.ends_with(&word[..at]) && lower.starts_with(&word[at..]))
        .then_some(Named::Part)
}
/// A place as a search looks at it. All but `name` is in lower case.
struct Sought<'a> {
    /// What it is called, as it is written and in lower case.
    name: &'a str,
    lower: &'a str,
    /// Another name it goes by: a file without its ending, a method or a
    /// function without its brackets.
    also: Option<&'a str>,
    /// What stands before the name where it is shown with what holds it:
    /// `cart.` before the `total()` of `Cart.total()`.
    held: &'a str,
    /// Its path, or the path of the file a symbol is in.
    path: &'a str,
    /// The path is its own, so that a word with a slash can name it.
    own: bool,
}
/// How well the words of a search name a place: as well as the best of them
/// does. Each word has to stand in the name or on the way to it, and one at
/// least in the name; otherwise the place is no answer.
fn answers(words: &[String], place: &Sought) -> Option<Named> {
    let mut best: Option<Named> = None;
    for word in words {
        let word = word.as_str();
        let how = if word.contains('/') {
            place.own.then(|| pathed(place.path, word)).flatten()
        } else {
            let other = place.also.and_then(|also| {
                if also == word {
                    Some(Named::Whole)
                } else if also.starts_with(word) {
                    Some(Named::Start)
                } else {
                    None
                }
            });
            match (named(place.name, place.lower, word), other) {
                (Some(one), Some(other)) => Some(one.min(other)),
                (one, other) => one
                    .or(other)
                    .or_else(|| spans(place.held, place.lower, word)),
            }
        };
        match how {
            Some(how) => best = Some(best.map_or(how, |best| best.min(how))),
            None => {
                // Not its name: it may still say where the place lies.
                let way = if place.own {
                    &place.path[..place.path.len() - place.lower.len()]
                } else {
                    place.path
                };
                if !way.contains(word) && !place.held.contains(word) {
                    return None;
                }
            }
        }
    }
    best
}
/// One place a search found: how well it is named, whether it holds what
/// was typed with capitals as it was typed, how much is linked with it, how
/// long its name is, and which folder (0), file (1) or symbol (2) it is, by
/// its number among those.
struct Hit {
    how: Named,
    cased: bool,
    weight: u64,
    len: u32,
    class: u8,
    id: u32,
}
impl Index {
    /// The folders, the files and the names of the whole graph that a
    /// search names, whatever folder a picture has open: the `FOUND` best
    /// of them, and how many there are of each.
    ///
    /// A search is a few words. Each has to stand in the name of a place or
    /// on the way to it, and one at least in the name: `total cart` finds
    /// `total()` in `Cart.php` and not everything under a folder `cart`. A
    /// word with a slash is the end of a path, as `pathed` tells. Letters
    /// match whatever their case.
    ///
    /// The best is what is named most exactly: by its whole name, then by
    /// the start of it, then by the start of a word in it. A file is named
    /// by its name without the ending too, and a method or a function by
    /// its name without its class and its brackets. A word typed with a
    /// capital puts first what holds it as it was typed. Among equals the
    /// most linked comes first, then the shortest name.
    pub fn find(&self, query: &str) -> Value {
        let mut words = Vec::new();
        for word in query.split_whitespace().take(WORDS) {
            let mut lower = String::new();
            lower_onto(word, &mut lower);
            words.push(lower);
        }
        if words.is_empty() {
            return json!({
                "mode": "find",
                "query": "",
                "found": 0,
                "folders": 0,
                "files": 0,
                "names": 0,
                "rows": [],
            });
        }
        // How much is linked with each symbol, each file and each folder.
        let mut linked = vec![0u64; self.symbols.len()];
        for (from, to, ..) in &self.uses {
            linked[*from as usize] += 1;
            linked[*to as usize] += 1;
        }
        let busy = (0..self.files.len() as u32)
            .map(|file| self.range(file).map(|at| linked[at as usize]).sum::<u64>())
            .collect::<Vec<_>>();
        // Every folder a file lies in, with its files and their links.
        let mut folders: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
        for (file, path) in self.files.iter().enumerate() {
            let mut folder = holder(path);
            while !folder.is_empty() {
                let entry = folders.entry(folder).or_default();
                entry.0 += 1;
                entry.1 += busy[file];
                folder = holder(folder);
            }
        }
        let folders = folders.into_iter().collect::<Vec<_>>();
        // The words typed with a capital, each with its lower case, and
        // whether a place holds each of them as it was typed: in its name,
        // where the name holds the word at all, and else on the way to it.
        let capitals = query
            .split_whitespace()
            .take(WORDS)
            .zip(&words)
            .filter(|(typed, _)| typed.chars().any(char::is_uppercase))
            .collect::<Vec<_>>();
        let cased = |name: &str, lower: &str, ways: [&str; 2]| {
            !capitals.is_empty()
                && capitals.iter().all(|(typed, small)| {
                    if lower.contains(small.as_str()) {
                        name.contains(typed)
                    } else {
                        ways.iter().any(|way| way.contains(typed))
                    }
                })
        };
        let mut hits = Vec::new();
        let (mut lower, mut path, mut text) = (String::new(), String::new(), String::new());
        // How well a folder or a file is named, by its name and its own
        // path, whether as it was typed, and how long its name is. A file
        // goes by its name without the ending, too.
        let mut place = |whole: &str, file: bool| {
            let called = name(whole);
            lower.clear();
            lower_onto(called, &mut lower);
            path.clear();
            lower_onto(whole, &mut path);
            let how = answers(
                &words,
                &Sought {
                    name: called,
                    lower: &lower,
                    also: lower
                        .rsplit_once('.')
                        .map(|(stem, _)| stem)
                        .filter(|stem| file && !stem.is_empty()),
                    held: "",
                    path: &path,
                    own: true,
                },
            )?;
            Some((how, cased(called, &lower, [whole, ""]), called.len() as u32))
        };
        for (id, (folder, (_, weight))) in folders.iter().enumerate() {
            if let Some((how, cased, len)) = place(folder, false) {
                hits.push(Hit {
                    how,
                    cased,
                    weight: *weight,
                    len,
                    class: 0,
                    id: id as u32,
                });
            }
        }
        let in_folders = hits.len();
        for (file, whole) in self.files.iter().enumerate() {
            if let Some((how, cased, len)) = place(whole, true) {
                hits.push(Hit {
                    how,
                    cased,
                    weight: busy[file],
                    len,
                    class: 1,
                    id: file as u32,
                });
            }
        }
        let in_files = hits.len() - in_folders;
        for (file, whole) in self.files.iter().enumerate() {
            path.clear();
            lower_onto(whole, &mut path);
            for at in self.range(file as u32) {
                if matches!(self.symbols[at as usize].kind, Kind::File | Kind::Note) {
                    continue;
                }
                // A method is shown with its class before it, `Cart.total()`,
                // and is named by its own name.
                text.clear();
                self.show(at, &mut text);
                let label = self.label(at);
                let own = label.strip_prefix('.').unwrap_or(label);
                let cut = text.len() - own.len();
                lower.clear();
                lower_onto(&text[..cut], &mut lower);
                let held = lower.len();
                lower_onto(own, &mut lower);
                let (held, called) = lower.split_at(held);
                let how = answers(
                    &words,
                    &Sought {
                        name: own,
                        lower: called,
                        also: called.strip_suffix("()"),
                        held,
                        path: &path,
                        own: false,
                    },
                );
                if let Some(how) = how {
                    hits.push(Hit {
                        how,
                        cased: cased(own, called, [&text[..cut], whole]),
                        weight: linked[at as usize],
                        len: own.len() as u32,
                        class: 2,
                        id: at,
                    });
                }
            }
        }
        let found = hits.len();
        let before = |a: &Hit, b: &Hit| {
            a.how
                .cmp(&b.how)
                .then(b.cased.cmp(&a.cased))
                .then(b.weight.cmp(&a.weight))
                .then(a.len.cmp(&b.len))
                .then(a.class.cmp(&b.class))
                .then(a.id.cmp(&b.id))
        };
        if hits.len() > FOUND {
            hits.select_nth_unstable_by(FOUND - 1, before);
            hits.truncate(FOUND);
        }
        hits.sort_by(before);
        // A symbol is told by the name the picture of its file selects it by.
        let mut keys: HashMap<u32, Vec<String>> = HashMap::new();
        let rows = hits
            .iter()
            .map(|hit| match hit.class {
                0 => {
                    let (path, (files, _)) = folders[hit.id as usize];
                    json!({
                        "path": path,
                        "name": name(path),
                        "what": "folder",
                        "in": holder(path),
                        "files": files,
                    })
                }
                1 => {
                    let path = self.files[hit.id as usize].as_str();
                    json!({
                        "path": path,
                        "name": name(path),
                        "what": "file",
                        "in": holder(path),
                        "symbols": self.holds(hit.id),
                    })
                }
                _ => {
                    let symbol = &self.symbols[hit.id as usize];
                    let slot = (hit.id - self.range(symbol.file).start) as usize;
                    let selected = keys
                        .entry(symbol.file)
                        .or_insert_with(|| self.keys(symbol.file));
                    // Where its own name begins in the name it is shown by.
                    let shown = self.shown(hit.id);
                    let label = self.label(hit.id);
                    let own = shown.len() - label.strip_prefix('.').unwrap_or(label).len();
                    json!({
                        "path": selected[slot],
                        "name": shown,
                        "own": own,
                        "what": symbol.kind.word(),
                        "file": self.files[symbol.file as usize],
                        "line": symbol.line,
                    })
                }
            })
            .collect::<Vec<_>>();
        json!({
            "mode": "find",
            "query": query.trim(),
            "found": found,
            "folders": in_folders,
            "files": in_files,
            "names": found - in_folders - in_files,
            "rows": rows,
        })
    }
}
type Kept = (String, u64, Option<SystemTime>, Arc<Index>);
/// Indexes of the graphs looked at most recently, each tied to the size and
/// time of the file it was read from.
#[derive(Default)]
pub struct Maps {
    kept: Mutex<Vec<Kept>>,
}
impl Maps {
    /// The lock is held while a graph is read, so two requests for one graph
    /// read it once.
    pub fn index(&self, project: &str, graph: &Path, root: &Path) -> Result<Arc<Index>> {
        let meta = std::fs::metadata(graph)
            .map_err(|_| HarnessError::missing("This project has no published code graph."))?;
        if meta.len() > BYTE_LIMIT {
            return Err(HarnessError::invalid(
                "The code graph is larger than 2 GiB and cannot be mapped.",
            ));
        }
        let changed = meta.modified().ok();
        let mut kept = self
            .kept
            .lock()
            .map_err(|_| HarnessError::unavailable("The code graph map is unavailable."))?;
        if let Some(at) = kept.iter().position(|(name, ..)| name == project) {
            let entry = kept.remove(at);
            if entry.1 == meta.len() && entry.2 == changed {
                let index = entry.3.clone();
                kept.push(entry);
                return Ok(index);
            }
        }
        let index = Arc::new(Index::read(std::fs::File::open(graph)?, root)?);
        if kept.len() >= KEPT {
            kept.remove(0);
        }
        kept.push((project.to_owned(), meta.len(), changed, index.clone()));
        Ok(index)
    }
    /// Drops what was read for a project: its own graph and, under
    /// `project@commit`, the graphs of its other commits.
    pub fn forget(&self, project: &str) {
        if let Ok(mut kept) = self.kept.lock() {
            kept.retain(|(name, ..)| {
                name != project
                    && name
                        .strip_prefix(project)
                        .is_none_or(|rest| !rest.starts_with('@'))
            });
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn index(graph: Value) -> Index {
        Index::read(graph.to_string().as_bytes(), Path::new("/work/shop")).unwrap()
    }
    fn shop() -> Index {
        index(json!({
            "directed": false,
            "nodes": [
                {"id": "cart", "label": "Cart", "source_file": "src/Cart/Cart.php"},
                {"id": "cart_total", "label": ".total()", "source_file": "src/Cart/Cart.php"},
                {"id": "price", "label": "Price", "source_file": "src/Cart/Price.php"},
                {"id": "order", "label": "Order", "source_file": "src/Order/Order.php"},
                {"id": "mail", "label": "Mailer", "source_file": "/work/shop/src/Mail/Mailer.php"},
                {"id": "test", "label": "CartTest", "source_file": "tests\\CartTest.php"},
                {"id": "readme", "label": "README", "source_file": "README.md"},
                {"id": "idea", "label": "a concept without a file"},
                {"id": 7, "label": "a numeric id", "source_file": "src/Order/Order.php"}
            ],
            "links": [
                {"source": "cart", "target": "cart_total", "relation": "method", "confidence": "EXTRACTED", "source_file": "src/Cart/Cart.php"},
                {"source": "cart", "target": "price", "relation": "imports", "confidence": "EXTRACTED", "source_file": "src/Cart/Cart.php"},
                {"source": "cart_total", "target": "price", "relation": "calls", "confidence": "INFERRED", "source_file": "src/Cart/Cart.php"},
                {"source": "order", "target": "cart", "relation": "imports", "confidence": "EXTRACTED", "source_file": "src/Order/Order.php"},
                {"source": "mail", "target": "order", "relation": "calls", "confidence": "INFERRED", "source_file": "src/Order/Order.php"},
                {"source": "test", "target": "cart", "relation": "calls", "confidence": "INFERRED", "source_file": "tests/CartTest.php"},
                {"source": "test", "target": "price", "relation": "calls", "confidence": "EXTRACTED", "source_file": "tests/CartTest.php"},
                {"source": "readme", "target": "cart", "relation": "contains", "confidence": "EXTRACTED", "source_file": "README.md"},
                {"source": "order", "target": "price", "relation": "calls", "confidence": "INFERRED", "source_file": "src/Elsewhere.php"},
                {"source": "idea", "target": "cart", "relation": "references", "confidence": "EXTRACTED"},
                {"source": "order", "target": "missing", "relation": "calls", "confidence": "EXTRACTED"},
                {"source": 7, "target": "mail", "relation": "_python_import_marker"}
            ]
        }))
    }
    fn rows(view: &Value, key: &str) -> Vec<String> {
        view[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| match key {
                "edges" => format!(
                    "{} -> {} {}/{}",
                    row["from"].as_str().unwrap(),
                    row["to"].as_str().unwrap(),
                    row["links"],
                    row["guessed"]
                ),
                _ => format!(
                    "{} {} files={} out={} in={} within={}",
                    row["path"].as_str().unwrap(),
                    if row["folder"] == true {
                        "folder"
                    } else {
                        "file"
                    },
                    row["files"],
                    row["out"],
                    row["in"],
                    row["within"]
                ),
            })
            .collect()
    }
    #[test]
    fn links_become_file_dependencies_with_a_direction_and_a_confidence() {
        let index = shop();
        // Six files hold symbols; the concept without a file is no file.
        assert_eq!(index.files.len(), 6);
        assert_eq!((index.nodes, index.links, index.unplaced), (9, 12, 1));
        let view = index.view("shop", "", None, false, "");
        assert_eq!(
            view["totals"],
            json!({"files": 6, "symbols": 9, "relations": 12, "links": 6, "read": 3, "guessed": 3, "unplaced": 1, "ruled_out": 0})
        );
        assert_eq!(
            rows(&view, "nodes"),
            [
                "src folder files=4 out=0 in=2 within=4",
                "tests folder files=1 out=2 in=0 within=0",
                "README.md file files=1 out=0 in=0 within=0",
            ]
        );
        assert_eq!(rows(&view, "edges"), ["tests -> src 2/1"]);
        assert_eq!(view["crumbs"], json!([{"name": "shop", "path": ""}]));
    }
    fn change(index: &Index, changed: &[(&str, char)], scope: &str, focus: Option<&str>) -> Value {
        let changed = changed
            .iter()
            .map(|(path, kind)| ((*path).to_owned(), *kind))
            .collect::<Vec<_>>();
        index.changes("shop", &changed, scope, focus, false, "")
    }
    fn places(view: &Value) -> Vec<String> {
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| {
                format!(
                    "{} {} {}{} out={} in={}",
                    node["path"].as_str().unwrap(),
                    node["role"].as_str().unwrap(),
                    node["name"].as_str().unwrap(),
                    match node["status"].as_str() {
                        Some(status) => format!(" [{status}]"),
                        None if node["changed"].as_u64().unwrap() > 0 => {
                            format!(" [{} changed]", node["changed"])
                        }
                        None => String::new(),
                    },
                    node["out"],
                    node["in"]
                )
            })
            .collect()
    }
    #[test]
    fn a_change_is_drawn_with_what_it_uses_and_what_uses_it() {
        let index = shop();
        let changed = [
            ("src/Cart/Cart.php", 'M'),
            ("src/New.php", 'A'),
            ("src/Old.php", 'D'),
        ];
        let view = change(&index, &changed, "", None);
        assert_eq!(view["mode"], "changes");
        assert_eq!(
            places(&view),
            [
                "src/Cart/Cart.php changed Cart.php [M] out=2 in=2",
                "src/Cart/Price.php needed Price.php out=0 in=2",
                "src/Order/Order.php affected Order.php out=1 in=0",
                "tests/CartTest.php affected CartTest.php out=1 in=0",
            ]
        );
        assert_eq!(
            rows(&view, "edges"),
            [
                "src/Cart/Cart.php -> src/Cart/Price.php 2/1",
                "src/Order/Order.php -> src/Cart/Cart.php 1/0",
                "tests/CartTest.php -> src/Cart/Cart.php 1/1",
            ]
        );
        // One file is in the graph; the new one has no symbols there and
        // the deleted one is gone.
        assert_eq!(
            view["change"],
            json!({"files": 3, "graphed": 1, "here": 1, "removed": 1, "unknown": 1, "single": true, "hidden": 0, "affected": 2, "needed": 1})
        );
        assert_eq!(
            (view["children"].as_u64(), view["matching"].as_u64()),
            (Some(3), Some(3))
        );
        // Links the tool only guessed can be left out.
        let certain = index.changes(
            "shop",
            &[("src/Cart/Cart.php".to_owned(), 'M')],
            "",
            None,
            true,
            "",
        );
        assert_eq!(
            rows(&certain, "edges"),
            [
                "src/Cart/Cart.php -> src/Cart/Price.php 1/0",
                "src/Order/Order.php -> src/Cart/Cart.php 1/0",
            ]
        );
        // The filter chooses among the linked files; the change stays.
        let filtered = index.changes(
            "shop",
            &[("src/Cart/Cart.php".to_owned(), 'M')],
            "",
            None,
            false,
            "TEST",
        );
        assert_eq!(
            places(&filtered),
            [
                "src/Cart/Cart.php changed Cart.php [M] out=2 in=2",
                "tests/CartTest.php affected CartTest.php out=1 in=0",
            ]
        );
        assert_eq!(
            (filtered["children"].as_u64(), filtered["matching"].as_u64()),
            (Some(3), Some(1))
        );
        // A change the graph knows nothing of draws nothing.
        let unknown = change(&index, &[("docs/guide.md", 'A')], "", None);
        assert_eq!(unknown["nodes"], json!([]));
        assert_eq!(unknown["change"]["unknown"], 1);
    }
    #[test]
    fn a_selected_place_of_a_change_lists_both_sides() {
        let index = shop();
        let changed = [("src/Cart/Cart.php", 'M'), ("src/Order/Order.php", 'M')];
        let view = change(&index, &changed, "", Some("src/Cart/Cart.php"));
        let focus = &view["focus"];
        let listed = |key: &str| {
            focus[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| {
                    format!(
                        "{} {}/{}{}{}",
                        row["path"].as_str().unwrap(),
                        row["links"],
                        row["guessed"],
                        if row["changed"] == true {
                            " changed"
                        } else {
                            ""
                        },
                        if row["inside"] == true {
                            ""
                        } else {
                            " not drawn"
                        }
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(listed("uses"), ["src/Cart/Price.php 2/1"]);
        assert_eq!(
            listed("used_by"),
            ["src/Order/Order.php 1/0 changed", "tests/CartTest.php 1/1"]
        );
        assert_eq!(
            (focus["role"].as_str(), focus["status"].as_str()),
            (Some("changed"), Some("M"))
        );
        assert_eq!(
            focus["kinds"],
            json!([{"kind": "calls", "links": 1}, {"kind": "imports", "links": 1}])
        );
        // A file beside the change lists its links with the change only:
        // the mailer is used by Order.php and by nothing else that changed.
        let beside = change(&index, &changed, "", Some("src/Mail/Mailer.php"));
        assert_eq!(beside["focus"]["role"], "needed");
        assert_eq!(beside["focus"]["uses"], json!([]));
        assert_eq!(beside["focus"]["used_by"][0]["path"], "src/Order/Order.php");
        // A place that is not drawn is not selected.
        assert!(change(&index, &changed, "", Some("README.md"))["focus"].is_null());
    }
    #[test]
    fn a_wide_change_is_drawn_by_folder_and_a_folder_opens_to_its_files() {
        let mut nodes = vec![
            json!({"id": "core", "source_file": "lib/core.php"}),
            json!({"id": "main", "source_file": "app/main.php"}),
        ];
        let mut links = vec![
            json!({"source": "main", "target": "a0", "relation": "calls", "source_file": "app/main.php"}),
        ];
        let mut changed = Vec::new();
        for folder in ["a", "b"] {
            for number in 0..15 {
                let (id, file) = (
                    format!("{folder}{number}"),
                    format!("{folder}/f{number}.php"),
                );
                nodes.push(json!({"id": id, "source_file": file}));
                links.push(json!({"source": id, "target": "core", "relation": "calls", "source_file": file}));
                changed.push((file, 'M'));
            }
        }
        let index = index(json!({"nodes": nodes, "links": links}));
        let wide = index.changes("shop", &changed, "", None, false, "");
        assert_eq!(
            places(&wide),
            [
                "a changed a [15 changed] out=15 in=1",
                "b changed b [15 changed] out=15 in=0",
                "lib/core.php needed core.php out=0 in=30",
                "app/main.php affected main.php out=1 in=0",
            ]
        );
        assert_eq!(
            rows(&wide, "edges"),
            [
                "a -> lib/core.php 15/0",
                "b -> lib/core.php 15/0",
                "app/main.php -> a 1/0"
            ]
        );
        assert_eq!(wide["change"]["single"], false);
        assert_eq!(wide["nodes"][0]["folder"], true);
        // The opened folder holds few enough files to draw each; what
        // changed outside it no longer counts as the change here.
        let opened = index.changes("shop", &changed, "a", None, false, "");
        assert_eq!(opened["scope"], "a");
        assert_eq!(opened["change"]["here"], 15);
        assert_eq!(opened["change"]["graphed"], 30);
        assert_eq!(opened["nodes"].as_array().unwrap().len(), 17);
        assert_eq!(opened["nodes"][0]["folder"], false);
        assert_eq!(
            opened["crumbs"],
            json!([{"name": "shop", "path": ""}, {"name": "a", "path": "a"}])
        );
        // A folder without changes opens the whole change instead.
        assert_eq!(
            index.changes("shop", &changed, "lib", None, false, "")["scope"],
            ""
        );
    }
    #[test]
    fn a_folder_whose_changes_lie_in_one_folder_below_is_passed_through() {
        let mut nodes = vec![json!({"id": "core", "source_file": "lib/core.php"})];
        let mut links = Vec::new();
        let mut changed = Vec::new();
        for number in 0..30 {
            let (id, file) = (
                format!("f{number}"),
                format!("app/src/Billing/f{number}.php"),
            );
            nodes.push(json!({"id": id, "source_file": file}));
            links.push(
                json!({"source": id, "target": "core", "relation": "calls", "source_file": file}),
            );
            changed.push((file, 'M'));
        }
        let index = index(json!({"nodes": nodes, "links": links}));
        // Thirty files are too many to draw one by one from the top, and
        // `app` and `app/src` each hold nothing but the next folder.
        let view = index.changes("shop", &changed, "", None, false, "");
        assert_eq!(view["scope"], "app/src/Billing");
        assert_eq!(view["change"]["here"], 30);
        assert_eq!(view["nodes"].as_array().unwrap().len(), 31);
        assert_eq!(view["nodes"][0]["folder"], false);
        assert_eq!(view["crumbs"].as_array().unwrap().len(), 4);
        // One changed file in a folder is still a folder with a count.
        changed.push(("lib/core.php".to_owned(), 'M'));
        let top = index.changes("shop", &changed, "", None, false, "");
        assert_eq!(top["scope"], "");
        assert_eq!(
            places(&top),
            [
                "app changed app [30 changed] out=30 in=0",
                "lib changed lib [1 changed] out=0 in=30"
            ]
        );
    }
    #[test]
    fn files_that_share_a_name_are_told_apart_by_their_folder() {
        let all = [
            "src/Cart/Cart.php",
            "src/Order/Cart.php",
            "src/Order/Order.php",
            "README.md",
            "docs/README.md",
        ];
        assert_eq!(label(all[0], &all), "Cart/Cart.php");
        assert_eq!(label(all[1], &all), "Order/Cart.php");
        assert_eq!(label(all[2], &all), "Order.php");
        // A file at the top has nothing to add to its name.
        assert_eq!(label(all[3], &all), "README.md");
        assert_eq!(label(all[4], &all), "docs/README.md");
    }
    #[test]
    fn a_link_points_from_the_file_it_was_found_in() {
        // The undirected graph stored Mailer -> Order; the call sits in Order.php.
        let view = shop().view("shop", "src", None, false, "");
        assert_eq!(
            rows(&view, "edges"),
            ["src/Order -> src/Cart 1/0", "src/Order -> src/Mail 1/1"]
        );
        assert_eq!(
            rows(&view, "nodes"),
            [
                "src/Cart folder files=2 out=0 in=3 within=2",
                "src/Order folder files=1 out=2 in=0 within=0",
                "src/Mail folder files=1 out=0 in=1 within=0",
            ]
        );
        assert_eq!(
            view["crumbs"],
            json!([{"name": "shop", "path": ""}, {"name": "src", "path": "src"}])
        );
    }
    #[test]
    fn a_focus_lists_what_it_uses_and_what_uses_it_inside_and_outside_the_folder() {
        let view = shop().view("shop", "src", Some("src/Cart"), false, "");
        let focus = &view["focus"];
        assert_eq!(focus["name"], "Cart");
        assert_eq!(
            (focus["files"].as_u64(), focus["within"].as_u64()),
            (Some(2), Some(2))
        );
        assert_eq!(focus["uses"], json!([]));
        assert_eq!(
            focus["used_by"],
            json!([
                {"path": "tests", "name": "tests", "folder": true, "inside": false, "links": 2, "guessed": 1},
                {"path": "src/Order", "name": "Order", "folder": true, "inside": true, "links": 1, "guessed": 0},
            ])
        );
        let order = shop().view("shop", "src", Some("src/Order"), false, "");
        assert_eq!(
            order["focus"]["kinds"],
            json!([{"kind": "calls", "links": 1}, {"kind": "imports", "links": 1}])
        );
        // A place outside the folder can be focused too; an unknown one cannot.
        let tests = shop().view("shop", "src", Some("tests"), false, "");
        assert_eq!(tests["focus"]["inside"], false);
        assert_eq!(tests["focus"]["uses"][0]["path"], "src/Cart");
        assert_eq!(
            shop().view("shop", "src", Some("nowhere"), false, "")["focus"],
            Value::Null
        );
    }
    #[test]
    fn certain_leaves_out_what_the_tool_inferred() {
        let view = shop().view("shop", "src", Some("src/Order"), true, "");
        assert_eq!(rows(&view, "edges"), ["src/Order -> src/Cart 1/0"]);
        assert_eq!(view["focus"]["uses"].as_array().unwrap().len(), 1);
        assert_eq!(view["totals"]["guessed"], 3);
        assert_eq!(view["certain"], true);
    }
    #[test]
    fn a_filter_and_the_limit_choose_which_children_are_drawn() {
        let view = shop().view("shop", "src", None, false, "ma");
        assert_eq!(
            rows(&view, "nodes"),
            ["src/Mail folder files=1 out=0 in=1 within=0"]
        );
        assert_eq!(
            (view["children"].as_u64(), view["matching"].as_u64()),
            (Some(3), Some(1))
        );
        let files = (0..SHOWN + 5)
            .map(|at| json!({"id": format!("n{at}"), "source_file": format!("lib/f{at:02}.php")}))
            .collect::<Vec<_>>();
        let links = (1..SHOWN + 5)
            .map(|at| json!({"source": format!("n{at}"), "target": "n0", "relation": "imports", "source_file": format!("lib/f{at:02}.php")}))
            .collect::<Vec<_>>();
        let wide = index(json!({"nodes": files, "edges": links}));
        let view = wide.view("wide", "lib", None, false, "");
        assert_eq!(view["nodes"].as_array().unwrap().len(), SHOWN);
        assert_eq!(view["children"].as_u64(), Some(SHOWN as u64 + 5));
        // The most used file comes first, and a link without a confidence was read.
        assert_eq!(view["nodes"][0]["path"], "lib/f00.php");
        assert_eq!(view["totals"]["read"].as_u64(), Some(SHOWN as u64 + 4));
        assert_eq!(view["between"].as_u64(), Some(SHOWN as u64 - 1));
    }
    #[test]
    fn a_dense_folder_keeps_the_strongest_uses_of_each_child() {
        // Six files that all use each other; file N uses file M through N * 10 + M links.
        let nodes = (0..6)
            .map(|at| json!({"id": format!("n{at}"), "source_file": format!("f{at}.py")}))
            .collect::<Vec<_>>();
        let mut links = vec![];
        for from in 0..6 {
            for to in (0..6).filter(|to| *to != from) {
                for _ in 0..from * 10 + to + 1 {
                    links.push(json!({"source": format!("n{from}"), "target": format!("n{to}"), "relation": "calls", "source_file": format!("f{from}.py")}));
                }
            }
        }
        let dense = index(json!({"nodes": nodes, "links": links}));
        let view = dense.view("dense", "", Some("f0.py"), false, "");
        assert_eq!(
            (view["between"].as_u64(), view["backbone"].as_bool()),
            (Some(30), Some(true))
        );
        let drawn = rows(&view, "edges");
        assert_eq!(drawn.len(), 6 * STRONGEST);
        // File 0 keeps its links to 5, 4 and 3, the three it uses most.
        assert_eq!(
            drawn
                .iter()
                .filter(|row| row.starts_with("f0.py ->"))
                .collect::<Vec<_>>(),
            [
                "f0.py -> f5.py 6/0",
                "f0.py -> f4.py 5/0",
                "f0.py -> f3.py 4/0"
            ]
        );
        // The focus still lists everything.
        assert_eq!(view["focus"]["uses"].as_array().unwrap().len(), 5);
        // A sparse folder is drawn in full.
        assert_eq!(
            shop().view("shop", "src", None, false, "")["backbone"],
            false
        );
    }
    #[test]
    fn an_unknown_folder_opens_the_project_and_a_wrong_file_is_refused() {
        let view = shop().view("shop", "src/Nowhere", None, false, "");
        assert_eq!(view["scope"], "");
        assert_eq!(
            shop().view("shop", "/src/", None, false, "")["scope"],
            "src"
        );
        assert!(Index::read("[1, 2]".as_bytes(), Path::new("/")).is_err());
        assert!(Index::read("{\"nodes\": 5}".as_bytes(), Path::new("/")).is_err());
        // Links before nodes are still counted.
        let early = index(json!({
            "links": [{"source": "a", "target": "b", "relation": "imports", "source_file": "a.py"}],
            "nodes": [{"id": "a", "source_file": "a.py"}, {"id": "b", "source_file": "b.py"}],
        }));
        assert_eq!(
            rows(&early.view("p", "", None, false, ""), "edges"),
            ["a.py -> b.py 1/0"]
        );
    }
    #[test]
    fn a_place_stands_for_a_file_inside_and_outside_the_open_folder() {
        assert_eq!(group("", "src/Cart/Cart.php"), ("src", true));
        assert_eq!(group("", "README.md"), ("README.md", true));
        assert_eq!(group("src", "src/Cart/Cart.php"), ("src/Cart", true));
        assert_eq!(
            group("src/Cart", "src/Cart/Cart.php"),
            ("src/Cart/Cart.php", true)
        );
        assert_eq!(
            group("src/Cart", "src/Order/Order.php"),
            ("src/Order", false)
        );
        assert_eq!(group("src/Cart", "tests/CartTest.php"), ("tests", false));
        assert_eq!(
            group("src/Cart", "src/Cartoon.php"),
            ("src/Cartoon.php", false)
        );
        assert_eq!(group("src/Cart", "README.md"), ("README.md", false));
    }
    /// A small shop as the tool writes it: a file node that holds classes
    /// and functions, classes that hold methods, an enum with its cases.
    /// An application over a library. `main` uses two handlers and both use
    /// `core`; `core` uses `db` through two links and a cache through one;
    /// one handler is only guessed to use `db`; `count` reports use `db`.
    /// The places of a view, each with the frame that holds it.
    fn held(view: &Value) -> Vec<String> {
        let mut places = view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| {
                format!(
                    "{}{}",
                    node["path"].as_str().unwrap(),
                    node["holder"]
                        .as_str()
                        .map(|holder| format!(" in {holder}"))
                        .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>();
        places.sort();
        places
    }
    /// The frames of a view: the folder, the frame that holds it, and how
    /// many of its places are drawn.
    fn framed(view: &Value) -> Vec<String> {
        view["frames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|frame| {
                format!(
                    "{}{} {}/{}",
                    frame["path"].as_str().unwrap(),
                    frame["holder"]
                        .as_str()
                        .map(|holder| format!(" in {holder}"))
                        .unwrap_or_default(),
                    frame["drawn"],
                    frame["places"]
                )
            })
            .collect()
    }
    fn sorted(mut rows: Vec<String>) -> Vec<String> {
        rows.sort();
        rows
    }
    #[test]
    fn a_folder_is_spread_out_among_what_lies_beside_it() {
        let index = shop();
        let spread = |scope: &str, folders: &[&str], focus: Option<&str>, filter: &str| {
            let folders = folders
                .iter()
                .map(|folder| (*folder).to_owned())
                .collect::<Vec<_>>();
            index.view_with("shop", scope, &folders, focus, false, filter)
        };
        // As it is drawn without: the folder is one place.
        let plain = index.view("shop", "", None, false, "");
        assert_eq!(held(&plain), ["README.md", "src", "tests"]);
        assert_eq!(framed(&plain), [""; 0]);
        // Spread out, it gives way to what it holds, and the links are those
        // of what it holds.
        let view = spread("", &["src"], None, "");
        assert_eq!(
            held(&view),
            [
                "README.md",
                "src/Cart in src",
                "src/Mail in src",
                "src/Order in src",
                "tests"
            ]
        );
        assert_eq!(framed(&view), ["src 3/3"]);
        assert_eq!(
            sorted(rows(&view, "edges")),
            [
                "src/Order -> src/Cart 1/0",
                "src/Order -> src/Mail 1/1",
                "tests -> src/Cart 2/1"
            ]
        );
        assert_eq!((&view["scope"], &view["children"]), (&json!(""), &json!(5)));
        // A folder inside it is spread out in turn, down to its files.
        let deep = spread("", &["src/Cart", "src"], None, "");
        assert_eq!(
            held(&deep),
            [
                "README.md",
                "src/Cart/Cart.php in src/Cart",
                "src/Cart/Price.php in src/Cart",
                "src/Mail in src",
                "src/Order in src",
                "tests"
            ]
        );
        assert_eq!(framed(&deep), ["src 2/2", "src/Cart in src 2/2"]);
        assert_eq!(
            sorted(rows(&deep, "edges")),
            [
                "src/Cart/Cart.php -> src/Cart/Price.php 2/1",
                "src/Order -> src/Cart/Cart.php 1/0",
                "src/Order -> src/Mail 1/1",
                "tests -> src/Cart/Cart.php 1/1",
                "tests -> src/Cart/Price.php 1/0"
            ]
        );
        // What is named and not reached is passed over: a folder inside one
        // that is not spread out, a file, a name the graph does not know.
        let unreached = spread("", &["src/Cart", "README.md", "nowhere"], None, "");
        assert_eq!(unreached["nodes"], plain["nodes"]);
        assert_eq!(framed(&unreached), [""; 0]);
        // In an open folder the frame of its own folder has no holder.
        let inside = spread("src", &["src/Cart/"], None, "");
        assert_eq!(
            held(&inside),
            [
                "src/Cart/Cart.php in src/Cart",
                "src/Cart/Price.php in src/Cart",
                "src/Mail",
                "src/Order"
            ]
        );
        assert_eq!(framed(&inside), ["src/Cart 2/2"]);
        // A filter keeps the places of that name, wherever they lie, with
        // the frames that hold them.
        let filtered = spread("", &["src", "src/Cart"], None, "price");
        assert_eq!(held(&filtered), ["src/Cart/Price.php in src/Cart"]);
        assert_eq!(framed(&filtered), ["src 0/2", "src/Cart in src 1/2"]);
        assert_eq!(filtered["matching"], 1);
        // A place in a frame is selected as any other.
        let chosen = spread("", &["src", "src/Cart"], Some("src/Cart/Cart.php"), "");
        assert_eq!(chosen["focus"]["name"], "Cart.php");
        let side = |key: &str| {
            chosen["focus"][key]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["path"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(side("uses"), ["src/Cart/Price.php"]);
        assert_eq!(sorted(side("used_by")), ["src/Order", "tests"]);
        // A folder that is spread out is no place to select.
        assert_eq!(spread("", &["src"], Some("src"), "")["focus"], Value::Null);
    }
    /// A project of `folders` folders of `files` files each, every file of
    /// which uses one hub, as does one more file beside them.
    fn wheel(folders: usize, files: usize) -> Index {
        let mut nodes = vec![
            json!({"id": "hub", "label": "hub()", "source_file": "hub.rs"}),
            json!({"id": "app", "label": "app()", "source_file": "app.rs"}),
        ];
        let mut links = vec![
            json!({"source": "app", "target": "hub", "relation": "calls", "confidence": "EXTRACTED", "source_file": "app.rs"}),
        ];
        for folder in 0..folders {
            for file in 0..files {
                let (id, path) = (
                    format!("f{folder}_{file}"),
                    format!("p{folder}/f{file:02}.rs"),
                );
                nodes.push(json!({"id": id, "label": "spoke()", "source_file": path}));
                links.push(json!({"source": id, "target": "hub", "relation": "calls", "confidence": "EXTRACTED", "source_file": path}));
            }
        }
        index(json!({"directed": false, "nodes": nodes, "links": links}))
    }
    #[test]
    fn a_spread_out_folder_shows_its_most_linked_and_a_full_picture_keeps_the_open_folder() {
        let index = wheel(1, HELD + 5);
        let view = index.view_with("wheel", "", &["p0".to_owned()], None, false, "");
        // The frame holds what it can; the open folder keeps its own.
        assert_eq!(framed(&view), [format!("p0 {HELD}/{}", HELD + 5)]);
        let places = held(&view);
        assert_eq!(places.len(), HELD + 2);
        assert!(places.contains(&"app.rs".to_owned()) && places.contains(&"hub.rs".to_owned()));
        assert!(places.contains(&"p0/f00.rs in p0".to_owned()));
        assert!(!places.contains(&format!("p0/f{:02}.rs in p0", HELD)));
        assert_eq!(
            (&view["children"], &view["matching"]),
            (&json!(HELD + 7), &json!(HELD + 7))
        );
        // More is spread out than a picture holds: the frames fill their
        // share, and what the open folder itself holds is still there.
        let folders = SPREAD / HELD;
        let index = wheel(folders, HELD);
        let all = (0..folders)
            .map(|folder| format!("p{folder}"))
            .collect::<Vec<_>>();
        let view = index.view_with("wheel", "", &all, None, false, "");
        let places = held(&view);
        assert_eq!(places.len(), SPREAD - OWN + 2);
        assert!(places.contains(&"app.rs".to_owned()) && places.contains(&"hub.rs".to_owned()));
        assert_eq!(framed(&view).len(), (SPREAD - OWN) / HELD);
    }
    /// One end of a looked-into link: each file with the names drawn in it.
    fn end(view: &Value, side: &str) -> Vec<String> {
        view[side]["groups"]
            .as_array()
            .unwrap()
            .iter()
            .map(|group| {
                format!(
                    "{}: {}",
                    group["name"].as_str().unwrap(),
                    group["names"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|name| format!(
                            "{} {}x{}",
                            name["kind"].as_str().unwrap(),
                            name["name"].as_str().unwrap(),
                            name["links"]
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
            .collect()
    }
    /// The links of a looked-into link, by the names of their two ends.
    fn wires(view: &Value) -> Vec<String> {
        let named = |key: &Value| {
            ["users", "used"]
                .iter()
                .flat_map(|side| view[*side]["groups"].as_array().unwrap())
                .flat_map(|group| group["names"].as_array().unwrap())
                .find(|name| name["key"] == *key)
                .map(|name| name["name"].as_str().unwrap().to_owned())
                .unwrap()
        };
        view["edges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|edge| {
                format!(
                    "{} -> {} {}{}",
                    named(&edge["from"]),
                    named(&edge["to"]),
                    edge["kind"].as_str().unwrap(),
                    if edge["guessed"] == true {
                        " guessed"
                    } else {
                        ""
                    }
                )
            })
            .collect()
    }
    #[test]
    fn a_link_is_looked_into_down_to_the_names_at_its_two_ends() {
        let index = shop();
        // The arrow of the project's picture: the tests use the source.
        let view = index.link("tests", "src", false);
        assert_eq!(
            (&view["mode"], &view["links"], &view["guessed"]),
            (&json!("link"), &json!(2), &json!(1))
        );
        assert_eq!(
            view["from"],
            json!({"path": "tests", "name": "tests", "what": "folder"})
        );
        assert_eq!(
            view["to"],
            json!({"path": "src", "name": "src", "what": "folder"})
        );
        // What has a method is a type; the graph says no more of the rest.
        assert_eq!(end(&view, "users"), ["CartTest.php: name CartTestx2"]);
        assert_eq!(
            end(&view, "used"),
            ["Cart.php: type Cartx1", "Price.php: name Pricex1"]
        );
        assert_eq!(
            wires(&view),
            ["CartTest -> Cart calls guessed", "CartTest -> Price calls"]
        );
        assert_eq!(
            (&view["users"]["names"], &view["users"]["files"]),
            (&json!(1), &json!(1))
        );
        assert_eq!(
            (&view["used"]["names"], &view["used"]["files"]),
            (&json!(2), &json!(2))
        );
        assert_eq!(view["kinds"], json!([{"kind": "calls", "links": 2}]));
        // Only what the tool read from the code.
        let certain = index.link("tests", "src", true);
        assert_eq!(
            (&certain["links"], &certain["guessed"]),
            (&json!(1), &json!(0))
        );
        assert_eq!(wires(&certain), ["CartTest -> Price calls"]);
        // Between two files the names are their symbols: a method under the
        // name of its type.
        let files = index.link("src/Cart/Cart.php", "src/Cart/Price.php", false);
        assert_eq!(files["from"]["what"], "file");
        assert_eq!(
            end(&files, "users"),
            ["Cart.php: type Cartx1, method Cart.total()x1"]
        );
        assert_eq!(end(&files, "used"), ["Price.php: name Pricex2"]);
        assert_eq!(
            wires(&files),
            [
                "Cart -> Price imports",
                "Cart.total() -> Price calls guessed"
            ]
        );
        // A symbol of an opened file is an end by the name that file's
        // picture gives it.
        let key = files["users"]["groups"][0]["names"][1]["key"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(key, "src/Cart/Cart.php#.total()");
        let symbol = index.link(&key, "src/Cart/Price.php", false);
        assert_eq!(
            symbol["from"],
            json!({"path": key, "name": "Cart.total()", "what": "symbol", "file": "src/Cart/Cart.php"})
        );
        assert_eq!(wires(&symbol), ["Cart.total() -> Price calls guessed"]);
        // The other way round nothing links the two, and that is an answer.
        let back = index.link("src", "tests", false);
        assert_eq!(
            (&back["links"], &back["missing"]),
            (&json!(0), &Value::Null)
        );
        assert_eq!((end(&back, "users"), wires(&back)), (vec![], vec![]));
        // An end the graph does not know makes the link missing: the
        // project itself, a path that is not there, a symbol that is not.
        for (from, to) in [
            ("", "src"),
            ("src", "nowhere"),
            ("src/Cart/Cart.php#.gone()", "src"),
        ] {
            let missing = index.link(from, to, false);
            assert_eq!(missing["missing"], true, "{from} -> {to}");
            assert_eq!(missing["mode"], "link");
        }
    }
    #[test]
    fn a_wide_link_shows_the_most_linked_names_of_each_end_and_counts_them_all() {
        // Every file of one folder uses the hub; the hub is one name.
        let index = wheel(1, LINKED + 7);
        let view = index.link("p0", "hub.rs", false);
        assert_eq!(view["links"], json!(LINKED + 7));
        assert_eq!(
            (&view["users"]["names"], &view["users"]["files"]),
            (&json!(LINKED + 7), &json!(LINKED + 7))
        );
        let drawn = view["users"]["groups"].as_array().unwrap().len();
        assert_eq!(drawn, LINKED);
        // A link is drawn where both of its ends are.
        assert_eq!(view["edges"].as_array().unwrap().len(), LINKED);
        assert_eq!(
            end(&view, "used"),
            [format!("hub.rs: function hub()x{}", LINKED + 7)]
        );
        // Files that share a name are told apart by their folders.
        let index = wheel(2, 1);
        let view = index.link("p0", "hub.rs", false);
        assert_eq!(end(&view, "users"), ["f00.rs: function spoke()x1"]);
        let both = index.link("p1/f00.rs", "hub.rs", false);
        assert_eq!(end(&both, "users"), ["f00.rs: function spoke()x1"]);
    }
    fn office(count: usize) -> Index {
        let call = |from: &str, to: &str, file: &str, read: bool| {
            json!({
                "source": from, "target": to, "relation": "calls",
                "confidence": if read { "EXTRACTED" } else { "INFERRED" },
                "source_file": file
            })
        };
        let mut nodes = vec![
            json!({"id": "main", "label": "main", "source_file": "app/main.php"}),
            json!({"id": "users", "label": "users", "source_file": "app/api/users.php"}),
            json!({"id": "orders", "label": "orders", "source_file": "app/api/orders.php"}),
            json!({"id": "core", "label": "core", "source_file": "lib/core.php"}),
            json!({"id": "boot", "label": "boot", "source_file": "lib/core.php"}),
            json!({"id": "db", "label": "db", "source_file": "lib/db.php"}),
            json!({"id": "store", "label": "store", "source_file": "lib/deep/cache/store.php"}),
        ];
        let mut links = vec![
            call("main", "users", "app/main.php", true),
            call("main", "orders", "app/main.php", true),
            call("users", "core", "app/api/users.php", true),
            call("orders", "core", "app/api/orders.php", true),
            call("orders", "db", "app/api/orders.php", false),
            call("core", "db", "lib/core.php", true),
            call("boot", "db", "lib/core.php", true),
            call("core", "store", "lib/core.php", true),
        ];
        for at in 1..=count {
            let (id, file) = (format!("r{at:02}"), format!("reports/r{at:02}.php"));
            nodes.push(json!({"id": id, "label": id, "source_file": file}));
            links.push(call(&id, "db", &file, true));
        }
        index(json!({"directed": false, "nodes": nodes, "links": links}))
    }
    /// Each box of followed links: its name, how many of what it uses and of
    /// what uses it are not drawn, and which of its sides are unfolded.
    fn steps(view: &Value) -> Vec<String> {
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| {
                format!(
                    "{}{} +{}/{}{}{}",
                    node["name"].as_str().unwrap(),
                    if node["folder"] == true { "/" } else { "" },
                    node["more_out"],
                    node["more_in"],
                    if node["unfolded_out"] == true {
                        " out"
                    } else {
                        ""
                    },
                    if node["unfolded_in"] == true {
                        " in"
                    } else {
                        ""
                    },
                )
            })
            .collect()
    }
    fn named(places: &[&str]) -> Vec<String> {
        places.iter().map(|place| (*place).to_owned()).collect()
    }
    #[test]
    fn the_links_of_a_file_are_followed_step_by_step() {
        let index = office(14);
        let follow = |out: &[&str], into: &[&str]| {
            index
                .follow(
                    "office",
                    "lib/core.php",
                    &named(out),
                    &named(into),
                    &[],
                    None,
                    false,
                )
                .unwrap()
        };
        // The file alone at first, with how many it uses and how many use it.
        let alone = follow(&[], &[]);
        assert_eq!(
            (&alone["mode"], &alone["scope"]),
            (&json!("follow"), &json!("lib"))
        );
        assert_eq!(steps(&alone), ["core.php +2/2"]);
        assert_eq!(alone["nodes"][0]["start"], true);
        assert_eq!(rows(&alone, "edges"), Vec::<String>::new());
        assert_eq!(
            alone["followed"],
            json!({
                "path": "lib/core.php", "name": "core.php", "folder": false, "uses": 2, "users": 2,
                "shown": 1, "limit": FOLLOWED, "step": STEP, "full": false, "aside": 0
            })
        );
        assert_eq!(
            alone["crumbs"],
            json!([
                {"name": "office", "path": ""},
                {"name": "lib", "path": "lib"},
                {"name": "core.php", "path": "lib/core.php"}
            ])
        );
        // What it uses, strongest first: db through two links, then the cache.
        let uses = follow(&["lib/core.php"], &[]);
        assert_eq!(
            steps(&uses),
            ["core.php +0/2 out", "db.php +0/15", "store.php +0/0"]
        );
        assert_eq!(
            rows(&uses, "edges"),
            [
                "lib/core.php -> lib/db.php 2/0",
                "lib/core.php -> lib/deep/cache/store.php 1/0"
            ]
        );
        // Both ways. orders is guessed to use db and both are drawn, but
        // neither that side of orders nor that side of db is unfolded: the
        // link waits, and each of the two counts the other as not shown.
        let both = follow(&["lib/core.php"], &["lib/core.php"]);
        assert_eq!(
            steps(&both),
            [
                "core.php +0/0 out in",
                "db.php +0/15",
                "store.php +0/0",
                "orders.php +1/1",
                "users.php +0/1"
            ]
        );
        assert_eq!(
            rows(&both, "edges"),
            [
                "lib/core.php -> lib/db.php 2/0",
                "app/api/orders.php -> lib/core.php 1/0",
                "app/api/users.php -> lib/core.php 1/0",
                "lib/core.php -> lib/deep/cache/store.php 1/0"
            ]
        );
        assert_eq!(both["followed"]["aside"], 1);
        // Unfolding what orders uses draws it, and adds no place.
        let drawn = follow(&["lib/core.php", "app/api/orders.php"], &["lib/core.php"]);
        assert_eq!(drawn["nodes"].as_array().unwrap().len(), 5);
        assert!(rows(&drawn, "edges").contains(&"app/api/orders.php -> lib/db.php 1/1".to_owned()));
        assert_eq!(steps(&drawn)[3], "orders.php +0/1 out");
        assert_eq!(drawn["followed"]["aside"], 0);
        // A step is twelve places that are not drawn yet: of the fourteen
        // reports that use db, twelve come first and the rest with the next.
        let step = follow(&["lib/core.php"], &["lib/core.php", "lib/db.php"]);
        assert_eq!(step["nodes"].as_array().unwrap().len(), 5 + STEP);
        assert_eq!(steps(&step)[1], "db.php +0/2 in");
        assert_eq!(step["followed"]["shown"], 17);
        let whole = follow(
            &["lib/core.php"],
            &["lib/core.php", "lib/db.php", "lib/db.php"],
        );
        assert_eq!(whole["nodes"].as_array().unwrap().len(), 19);
        assert_eq!(steps(&whole)[1], "db.php +0/0 in");
        // Folding a side takes away what only it brought: the handlers go,
        // and orders comes back as one of those that use db, its link to
        // core waiting again.
        let folded = follow(&["lib/core.php"], &["lib/db.php", "lib/db.php"]);
        assert_eq!(steps(&folded)[0], "core.php +0/2 out");
        assert_eq!(steps(&folded)[3], "orders.php +1/1");
        assert_eq!(folded["nodes"].as_array().unwrap().len(), 3 + 15);
        // What is unfolded behind a folded side is not reached.
        let unreached = follow(&[], &["lib/db.php"]);
        assert_eq!(steps(&unreached), ["core.php +2/2"]);
    }
    #[test]
    fn followed_links_leave_out_guesses_and_stop_at_a_full_picture() {
        let index = office(14);
        let certain = index
            .follow(
                "office",
                "lib/core.php",
                &named(&["lib/core.php"]),
                &named(&["lib/core.php"]),
                &[],
                None,
                true,
            )
            .unwrap();
        assert!(
            !rows(&certain, "edges")
                .iter()
                .any(|edge| edge.starts_with("app/api/orders.php -> lib/db.php"))
        );
        assert_eq!(rows(&certain, "edges").len(), 4);
        // Seventy reports use db; however often its side is unfolded, the
        // picture stops at its limit and says so.
        let crowd = office(70);
        let mut into = named(&["lib/core.php"]);
        into.extend(named(&["lib/db.php"; 8]));
        let full = crowd
            .follow(
                "office",
                "lib/core.php",
                &named(&["lib/core.php"]),
                &into,
                &[],
                None,
                false,
            )
            .unwrap();
        assert_eq!(full["nodes"].as_array().unwrap().len(), FOLLOWED);
        assert_eq!(full["followed"]["full"], true);
        assert_eq!(steps(&full)[1], "db.php +0/15 in");
        // Nothing is followed from what the graph does not know.
        for unknown in ["", "nowhere.php", "lib/core"] {
            assert!(
                index
                    .follow("office", unknown, &[], &[], &[], None, false)
                    .is_none(),
                "{unknown}"
            );
        }
    }
    #[test]
    fn a_folder_is_followed_among_what_lies_as_deep_as_itself() {
        let index = office(14);
        assert_eq!(level("lib/deep/cache/store.php", 1), "lib");
        assert_eq!(level("lib/deep/cache/store.php", 2), "lib/deep");
        assert_eq!(level("lib/db.php", 2), "lib/db.php");
        assert_eq!(level("lib/db.php", usize::MAX), "lib/db.php");
        // A top folder among the top folders: the application and the
        // reports use the library, the reports through more links.
        let top = index
            .follow("office", "lib", &[], &named(&["lib"]), &[], None, false)
            .unwrap();
        assert_eq!(steps(&top), ["lib/ +0/0 in", "reports/ +0/0", "app/ +0/0"]);
        assert_eq!(
            rows(&top, "edges"),
            ["reports -> lib 14/0", "app -> lib 3/1"]
        );
        assert_eq!(
            (
                &top["scope"],
                &top["followed"]["folder"],
                &top["nodes"][0]["files"]
            ),
            (&json!(""), &json!(true), &json!(3))
        );
        // A folder two deep: the files that lie no deeper stand for
        // themselves, and what lies deeper is the folder at that depth.
        let api = index
            .follow(
                "office",
                "app/api",
                &named(&["app/api", "lib/core.php"]),
                &named(&["app/api"]),
                &[],
                None,
                false,
            )
            .unwrap();
        assert_eq!(
            steps(&api),
            [
                "api/ +0/0 out in",
                "core.php +0/0 out",
                "db.php +0/14",
                "main.php +0/0",
                "deep/ +0/0"
            ]
        );
        assert_eq!(api["nodes"][4]["path"], "lib/deep");
    }
    #[test]
    fn followed_links_mark_a_chosen_change_and_list_both_sides_of_a_place() {
        let index = office(14);
        let changed = [
            ("lib/db.php".to_owned(), 'M'),
            ("reports/r01.php".to_owned(), 'A'),
            ("gone.php".to_owned(), 'D'),
        ];
        let view = index
            .follow(
                "office",
                "lib/core.php",
                &named(&["lib/core.php"]),
                &[],
                &changed,
                Some("lib/db.php"),
                false,
            )
            .unwrap();
        let marks = view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| (node["changed"].as_u64().unwrap(), node["status"].as_str()))
            .collect::<Vec<_>>();
        assert_eq!(marks, [(0, None), (1, Some("M")), (0, None)]);
        // The selected place lists every place on each side, drawn or not.
        let focus = &view["focus"];
        assert_eq!(
            (
                &focus["path"],
                &focus["name"],
                &focus["status"],
                &focus["start"]
            ),
            (
                &json!("lib/db.php"),
                &json!("db.php"),
                &json!("M"),
                &json!(false)
            )
        );
        assert_eq!(focus["uses"], json!([]));
        assert_eq!(
            (&focus["uses_more"], &focus["used_by_more"]),
            (&json!(0), &json!(4))
        );
        let users = focus["used_by"].as_array().unwrap();
        assert_eq!(users.len(), LISTED);
        assert_eq!(
            users[0],
            json!({"path": "lib/core.php", "name": "core.php", "folder": false, "inside": true, "links": 2, "guessed": 0, "changed": false})
        );
        assert_eq!(
            users[1],
            json!({"path": "app/api/orders.php", "name": "orders.php", "folder": false, "inside": false, "links": 1, "guessed": 1, "changed": false})
        );
        assert_eq!(users[2]["changed"], true);
        // What the followed place uses, by kind.
        let core = index
            .follow(
                "office",
                "lib/core.php",
                &[],
                &[],
                &[],
                Some("lib/core.php"),
                false,
            )
            .unwrap();
        assert_eq!(
            core["focus"]["kinds"],
            json!([{"kind": "calls", "links": 3}])
        );
        assert_eq!(core["focus"]["start"], true);
        // A folder that holds changed files says how many, not what happened.
        let top = index
            .follow(
                "office",
                "lib",
                &[],
                &named(&["lib"]),
                &changed,
                None,
                false,
            )
            .unwrap();
        let marks = top["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| (node["changed"].as_u64().unwrap(), node["status"].as_str()))
            .collect::<Vec<_>>();
        assert_eq!(marks, [(1, None), (1, None), (0, None)]);
        // A place that is not drawn is not selected.
        let absent = index
            .follow(
                "office",
                "lib/core.php",
                &[],
                &[],
                &[],
                Some("lib/db.php"),
                false,
            )
            .unwrap();
        assert_eq!(absent["focus"], Value::Null);
    }
    #[test]
    fn a_test_is_told_by_where_a_file_lies_and_how_it_is_named() {
        for path in [
            "tests/CartTest.php",
            "tests/Feature/checkout.php",
            "src/Cart/Tests/CartHelper.php",
            "packages/cart/__tests__/total.js",
            "spec/models/cart_spec.rb",
            "internal/cart/testdata/order.go",
            "src/DataFixtures/UserFixtures.php",
            "database/fixtures/users.php",
            "web/__mocks__/api.ts",
            "e2e/login.ts",
            "cypress/support/commands.js",
            "App.Tests/CartTests.cs",
            "services/integration-tests/run.py",
            "crates/core/unit_tests/mod.rs",
            "src/UnitTests/Cart.cs",
            "src/Cart/CartTest.php",
            "src/Cart/CartTests.cs",
            "src/Cart/CartTestCase.php",
            "web/cart.test.tsx",
            "web/cart.spec.js",
            "api/app.e2e-spec.ts",
            "web/cart.cy.ts",
            "shop/test_cart.py",
            "shop/cart_test.py",
            "shop/conftest.py",
            "cart/cart_test.go",
            "src/cart_tests.rs",
            "src/cart/tests.rs",
            "src/cart_fixture.rs",
            "lib/cart_spec.rb",
        ] {
            assert!(is_test(path), "{path}");
        }
        for path in [
            "src/Cart/Cart.php",
            "src/Latest/News.php",
            "src/Contest/Entry.php",
            "src/Contest.php",
            "src/Attest.php",
            "src/LatestNews.php",
            "src/Protest/Vote.php",
            "specs/MANIFEST.md",
            "Task/Epics/Epic-01_Users_SPEC.md",
            "docs/testing.md",
            "web/jquery.min.js",
            "web/vite.config.ts",
            "src/Inspector.php",
            "src/TestimonialController.php",
            "src/fixture_loader.rs",
            "README.md",
            "",
        ] {
            assert!(!is_test(path), "{path}");
        }
    }
    /// A Rust source with what a build keeps and what only tests use.
    const SOURCE: &str = "\
pub fn run() -> u32 {
    helper() + tool()
}

fn helper() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checks the sum.
    #[test]
    fn checks() {
        assert_eq!(run(), probe());
    }

    struct Fake;
    impl Fake {
        fn new() -> Self {
            Fake
        }
    }
}

#[cfg(not(test))]
fn live() {}

#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn standalone(
    seed: u32,
) {
    run();
}

#[cfg(test)] mod extra;
";
    #[test]
    fn the_tests_inside_a_rust_source_are_its_marked_items() {
        // The module from its attribute to its closing brace, the function
        // whose signature runs over lines, and the module on one line.
        assert_eq!(rust_tests(SOURCE), [(9, 25), (30, 36), (38, 38)]);
        assert_eq!(rust_tests("fn plain() {}\n"), Vec::<(u32, u32)>::new());
        assert_eq!(rust_tests(""), Vec::<(u32, u32)>::new());
        // An item that is never closed runs to the end of the text.
        assert_eq!(rust_tests("#[test]\nfn open() {\n    run();\n"), [(1, 3)]);
        // A test often holds a source of its own: a brace in a string, in a
        // raw string, in a character or in a comment is not the module's.
        let holding = "#[cfg(test)]\nmod tests {\n    const A: &str = \"\n}\n\";\n    const B: &str = r#\"\n}\"#;\n    const C: char = '}';\n    const D: char = '\\'';\n    // }\n    /* } /* } */ } */\n    fn f<'a>(x: &'a [u8; 2]) -> &'a [u8; 2] { x }\n}\nfn after() {}\n";
        assert_eq!(rust_tests(holding), [(1, 13)]);
        assert_eq!(item_end("fn a() { b(\"}\"); }", 0), 17);
        assert_eq!(item_end("static X: [u8; 2] = [1, 2]; fn b() {}", 0), 26);
        assert_eq!(item_end("mod r#type;", 0), 10);
        for attribute in [
            "test",
            "tokio::test",
            "tokio::test(flavor = \"multi_thread\")",
            "rstest",
            "test_case(1, 2)",
            "bench",
            "wasm_bindgen_test",
            "cfg(test)",
            "cfg(all(test, feature = \"slow\"))",
            "cfg(any(test, debug_assertions))",
        ] {
            assert!(marks_test(attribute), "{attribute}");
        }
        for attribute in [
            "cfg(not(test))",
            "cfg(all(not(test), unix))",
            "cfg(feature = \"test\")",
            "cfg(feature = \"test-utils\")",
            "cfg_attr(test, derive(Debug))",
            "derive(Debug)",
            "testing",
            "allow(dead_code)",
            "",
        ] {
            assert!(!marks_test(attribute), "{attribute}");
        }
        assert_eq!(attributes("#[cfg(test)] mod extra;"), (true, "mod extra;"));
        assert_eq!(
            attributes("#[doc = \"a ] b\"] #[test] fn x() {}"),
            (true, "fn x() {}")
        );
        assert_eq!(attributes("    fn plain() {}"), (false, "fn plain() {}"));
        assert_eq!(attributes("#[allow("), (false, ""));
    }
    #[test]
    fn a_graph_without_tests_drops_test_files_and_counts_its_links_anew() {
        let index = shop();
        let plain = index.plain_with(&|_| None);
        // The one test file goes, with the two links it made.
        assert_eq!(plain.left_out(), (1, 0));
        assert_eq!(index.left_out(), (0, 0));
        assert_eq!(plain.files.len(), 5);
        assert!(!plain.files.iter().any(|file| file.starts_with("tests/")));
        let view = plain.view("shop", "", None, false, "");
        assert_eq!(
            view["totals"],
            json!({"files": 5, "symbols": 8, "relations": 10, "links": 4, "read": 2, "guessed": 2, "unplaced": 1, "ruled_out": 0})
        );
        assert_eq!(
            rows(&view, "nodes"),
            [
                "src folder files=4 out=0 in=0 within=4",
                "README.md file files=1 out=0 in=0 within=0"
            ]
        );
        // What is left reads as it did: the folder of the cart, its two links.
        let folder = plain.view("shop", "src/Cart", None, false, "");
        assert_eq!(
            rows(&folder, "edges"),
            ["src/Cart/Cart.php -> src/Cart/Price.php 2/1"]
        );
        // A folder of tests is not there to open: the project opens.
        assert_eq!(plain.view("shop", "tests", None, false, "")["scope"], "");
        assert_eq!(
            index.view("shop", "tests", None, false, "")["scope"],
            "tests"
        );
        assert!(
            plain
                .follow("shop", "tests/CartTest.php", &[], &[], &[], None, false)
                .is_none()
        );
        // The graph without tests is made once and kept.
        assert!(Arc::ptr_eq(&index.without_tests(), &index.without_tests()));
        assert_eq!(index.without_tests().left_out(), (1, 0));
    }
    #[test]
    fn a_graph_without_tests_drops_the_tests_inside_a_rust_source() {
        let index = index(json!({
            "directed": false,
            "nodes": [
                {"id": "run", "label": "run()", "source_file": "src/lib.rs", "source_location": "L1"},
                {"id": "helper", "label": "helper()", "source_file": "src/lib.rs", "source_location": "L5"},
                {"id": "checks", "label": "checks()", "source_file": "src/lib.rs", "source_location": "L15"},
                {"id": "fake", "label": "Fake", "source_file": "src/lib.rs", "source_location": "L19"},
                {"id": "fake_new", "label": ".new()", "source_file": "src/lib.rs"},
                {"id": "live", "label": "live()", "source_file": "src/lib.rs", "source_location": "L28"},
                {"id": "standalone", "label": "standalone()", "source_file": "src/lib.rs", "source_location": "L32"},
                {"id": "tool", "label": "tool()", "source_file": "src/util.rs", "source_location": "L1"},
                {"id": "probe", "label": "probe()", "source_file": "src/util.rs", "source_location": "L5"}
            ],
            "links": [
                {"source": "run", "target": "helper", "relation": "calls", "confidence": "EXTRACTED", "source_file": "src/lib.rs"},
                {"source": "run", "target": "tool", "relation": "calls", "confidence": "EXTRACTED", "source_file": "src/lib.rs"},
                {"source": "live", "target": "tool", "relation": "calls", "confidence": "EXTRACTED", "source_file": "src/lib.rs"},
                {"source": "checks", "target": "run", "relation": "calls", "confidence": "EXTRACTED", "source_file": "src/lib.rs"},
                {"source": "checks", "target": "probe", "relation": "calls", "confidence": "EXTRACTED", "source_file": "src/lib.rs"},
                {"source": "standalone", "target": "run", "relation": "calls", "confidence": "INFERRED", "source_file": "src/lib.rs"},
                {"source": "fake", "target": "fake_new", "relation": "method", "confidence": "EXTRACTED", "source_file": "src/lib.rs"}
            ]
        }));
        let edges = |index: &Index| rows(&index.view("lib", "src", None, false, ""), "edges");
        assert_eq!(edges(&index), ["src/lib.rs -> src/util.rs 3/0"]);
        // A source that cannot be read keeps everything it holds.
        let unread = index.plain_with(&|_| None);
        assert_eq!(unread.left_out(), (0, 0));
        assert_eq!(edges(&unread), ["src/lib.rs -> src/util.rs 3/0"]);
        // Read, it loses the test, the test's type with its method, which
        // has no line of its own, and the test outside the module; the link
        // only the test made is gone from the file.
        let plain = index.plain_with(&|path| (path == "src/lib.rs").then(|| SOURCE.to_owned()));
        assert_eq!(plain.left_out(), (0, 4));
        assert_eq!(edges(&plain), ["src/lib.rs -> src/util.rs 2/0"]);
        let opened = plain.view("lib", "src/lib.rs", None, false, "");
        assert_eq!(opened["mode"], "file");
        let names = opened["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node["symbol"] == true)
            .map(|node| node["name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(names, ["run()", "helper()", "live()"]);
        assert_eq!(
            (&opened["file"]["functions"], &opened["file"]["types"]),
            (&json!(3), &json!(0))
        );
        // probe() is used by nothing that is left.
        let util = plain.view("lib", "src/util.rs", None, false, "");
        assert_eq!(util["file"]["used_by"], 1);
    }
    #[test]
    fn a_link_between_crates_that_are_not_built_together_is_left_out() {
        let graph = json!({
            "directed": false,
            "nodes": [
                {"id": "main", "label": "main()", "source_file": "crates/app/src/main.rs"},
                {"id": "core", "label": "Core", "source_file": "crates/core/src/lib.rs"},
                {"id": "core_run", "label": ".run()", "source_file": "crates/core/src/lib.rs"},
                {"id": "tool", "label": "tool()", "source_file": "crates/tool/src/lib.rs"},
                {"id": "check", "label": "check()", "source_file": "crates/core/tests/it.rs"},
                {"id": "cart", "label": "Cart", "source_file": "web/cart.php"},
                {"id": "gen", "label": "gen()", "source_file": "scripts/gen.rs"}
            ],
            "links": [
                {"source": "core", "target": "core_run", "relation": "method", "confidence": "EXTRACTED", "source_file": "crates/core/src/lib.rs"},
                {"source": "main", "target": "core", "relation": "imports_from", "confidence": "EXTRACTED", "source_file": "crates/app/src/main.rs"},
                {"source": "main", "target": "tool", "relation": "calls", "confidence": "INFERRED", "source_file": "crates/app/src/main.rs"},
                {"source": "core", "target": "main", "relation": "imports_from", "confidence": "EXTRACTED", "source_file": "crates/core/src/lib.rs"},
                {"source": "core_run", "target": "tool", "relation": "calls", "confidence": "INFERRED", "source_file": "crates/core/src/lib.rs"},
                {"source": "tool", "target": "main", "relation": "calls", "confidence": "EXTRACTED", "source_file": "crates/tool/src/lib.rs"},
                {"source": "tool", "target": "core", "relation": "references", "confidence": "EXTRACTED", "source_file": "crates/tool/src/lib.rs"},
                {"source": "check", "target": "main", "relation": "calls", "confidence": "INFERRED", "source_file": "crates/core/tests/it.rs"},
                {"source": "check", "target": "core", "relation": "calls", "confidence": "EXTRACTED", "source_file": "crates/core/tests/it.rs"},
                {"source": "cart", "target": "core", "relation": "calls", "confidence": "INFERRED", "source_file": "web/cart.php"},
                {"source": "gen", "target": "tool", "relation": "calls", "confidence": "INFERRED", "source_file": "scripts/gen.rs"}
            ]
        })
        .to_string();
        // The app is built with the core; the tool's tests with the app.
        let manifests = [
            ("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n"),
            (
                "crates/app/Cargo.toml",
                "[package]\nname = \"shop-app\"\n[dependencies]\nshop-core = { path = \"../core\" }\n",
            ),
            (
                "crates/core/Cargo.toml",
                "[package]\nname = \"shop-core\"\n",
            ),
            (
                "crates/tool/Cargo.toml",
                "[package]\nname = \"shop-tool\"\n[dev-dependencies]\nshop-app = { path = \"../app\" }\n",
            ),
        ];
        let read = |manifests: &[(&str, &str)]| {
            Index::read_with(graph.as_bytes(), Path::new("/work/shop"), &|path| {
                manifests
                    .iter()
                    .find(|(at, _)| *at == path)
                    .map(|(_, text)| (*text).to_owned())
            })
            .unwrap()
        };
        // The arrows of a folder, in the order of their names.
        let edges = |index: &Index, scope: &str| {
            let mut edges = rows(&index.view("shop", scope, None, false, ""), "edges");
            edges.sort();
            edges
        };
        // As the tool gave it: the core uses the app and the tool, which no
        // build of it could.
        let whole = read(&[]);
        assert_eq!(
            whole.view("shop", "", None, false, "")["totals"],
            json!({"files": 6, "symbols": 7, "relations": 11, "links": 10, "read": 5, "guessed": 5, "unplaced": 0, "ruled_out": 0})
        );
        assert_eq!(
            edges(&whole, "crates"),
            [
                "crates/app -> crates/core 1/0",
                "crates/app -> crates/tool 1/1",
                "crates/core -> crates/app 2/1",
                "crates/core -> crates/tool 1/1",
                "crates/tool -> crates/app 1/0",
                "crates/tool -> crates/core 1/0",
            ]
        );
        // With the manifests, four links cannot be: the app's into the
        // tool, and the three out of the core, one of them from its test.
        // What is not Rust, and the source of no crate, keep theirs.
        let index = read(&manifests);
        assert_eq!(
            index.view("shop", "", None, false, "")["totals"],
            json!({"files": 6, "symbols": 7, "relations": 11, "links": 6, "read": 4, "guessed": 2, "unplaced": 0, "ruled_out": 4})
        );
        assert_eq!(
            edges(&index, "crates"),
            [
                "crates/app -> crates/core 1/0",
                "crates/tool -> crates/app 1/0",
                "crates/tool -> crates/core 1/0",
            ]
        );
        assert_eq!(
            edges(&index, ""),
            ["scripts -> crates 1/1", "web -> crates 1/1"]
        );
        // The file that was said to use two others uses none.
        let core = index.view("shop", "crates/core/src/lib.rs", None, false, "");
        assert_eq!(
            (&core["file"]["uses"], &core["file"]["used_by"]),
            (&json!(0), &json!(4))
        );
        // Without tests, the link ruled out of a test is not counted twice.
        let plain = index.plain_with(&|_| None);
        assert_eq!(plain.left_out(), (1, 0));
        assert_eq!(
            plain.view("shop", "", None, false, "")["totals"],
            json!({"files": 5, "symbols": 6, "relations": 9, "links": 5, "read": 3, "guessed": 2, "unplaced": 0, "ruled_out": 3})
        );
    }
    fn store() -> Index {
        let cart = "src/Cart/Cart.php";
        let price = "src/Cart/Price.php";
        let notifier = "src/Order/Notifier.php";
        let test = "tests/CartTest.php";
        let node = |id: &str, label: &str, file: &str, line: u32| json!({"id": id, "label": label, "file_type": "code", "source_file": file, "source_location": format!("L{line}")});
        let class = |id: &str, label: &str, file: &str, line: u32| {
            let mut node = node(id, label, file, line);
            node["_callable_class"] = json!(true);
            node
        };
        let link = |source: &str, relation: &str, target: &str, file: &str| json!({"source": source, "target": target, "relation": relation, "confidence": "EXTRACTED", "source_file": file});
        let guess = |source: &str, relation: &str, target: &str, file: &str| json!({"source": source, "target": target, "relation": relation, "confidence": "INFERRED", "source_file": file});
        index(json!({
            "nodes": [
                node("cart_php", "Cart.php", cart, 1),
                class("cart", "Cart", cart, 12),
                node("cart_add", ".add()", cart, 18),
                node("cart_total", ".total()", cart, 19),
                node("cart_checkout", ".checkout()", cart, 26),
                // The tool gives a Rust struct no class flag: what has
                // cases or methods is a type all the same.
                node("status", "Status", cart, 10),
                node("status_open", "Open", cart, 10),
                node("status_paid", "Paid", cart, 10),
                node("format", "format_price()", cart, 29),
                node("limit", "LIMIT", cart, 5),
                {"id": "why", "label": "Totals are kept in cents so that sums never round.", "file_type": "rationale", "source_file": cart, "source_location": "L2"},
                // The node of a file can be named by the end of its path.
                node("price_php", "Cart/Price.php", price, 1),
                class("price", "Price", price, 4),
                node("price_of", ".of()", price, 9),
                node("price_plus", ".plus()", price, 10),
                node("notifier_php", "Notifier.php", notifier, 1),
                class("notifier", "Notifier", notifier, 8),
                node("notifier_paid", ".paid()", notifier, 11),
                node("test_php", "CartTest.php", test, 1),
                class("test", "CartTest", test, 5),
                node("test_total", ".testTotal()", test, 7),
                node("test_empty", ".testEmpty()", test, 15),
                {"id": "external", "label": "Countable", "file_type": "code", "source_file": "", "source_location": ""}
            ],
            "links": [
                link("cart_php", "contains", "cart", cart),
                link("cart_php", "contains", "status", cart),
                link("cart_php", "contains", "format", cart),
                link("cart_php", "contains", "limit", cart),
                link("cart", "method", "cart_add", cart),
                link("cart", "method", "cart_total", cart),
                link("cart", "method", "cart_checkout", cart),
                link("status", "case_of", "status_open", cart),
                link("status", "case_of", "status_paid", cart),
                link("why", "rationale_for", "cart", cart),
                link("price_php", "contains", "price", price),
                link("price", "method", "price_of", price),
                link("price", "method", "price_plus", price),
                link("notifier_php", "contains", "notifier", notifier),
                link("notifier", "method", "notifier_paid", notifier),
                link("test_php", "contains", "test", test),
                link("test", "method", "test_total", test),
                link("test", "method", "test_empty", test),
                // Inside the file.
                link("cart_checkout", "calls", "cart_total", cart),
                link("cart_checkout", "references", "status", cart),
                link("cart_add", "references", "limit", cart),
                link("cart_total", "calls", "cart_total", cart),
                // Out of it.
                link("cart_php", "imports", "notifier", cart),
                link("cart", "implements", "external", cart),
                link("cart_total", "calls", "price_of", cart),
                link("cart_total", "calls", "price_plus", cart),
                link("cart_add", "references", "price", cart),
                guess("cart_checkout", "calls", "notifier_paid", cart),
                guess("format", "calls", "price_plus", cart),
                // Into it.
                link("test_total", "calls", "cart_total", test),
                link("test_total", "calls", "cart_add", test),
                guess("test_empty", "calls", "cart_total", test),
                link("notifier_paid", "calls", "format", notifier)
            ]
        }))
    }
    const CART: &str = "src/Cart/Cart.php";
    /// The symbols of an opened file, or the files around it, as drawn.
    fn drawn(view: &Value, symbols: bool) -> Vec<String> {
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| (node["symbol"] == true) == symbols)
            .map(|node| {
                let path = node["path"].as_str().unwrap();
                if symbols {
                    format!(
                        "{} {} {}{} out={} in={}",
                        path.strip_prefix(CART).unwrap_or(path),
                        node["kind"].as_str().unwrap(),
                        node["name"].as_str().unwrap(),
                        match node["members"].as_u64().unwrap() {
                            0 => String::new(),
                            members => format!(" holds {members}"),
                        },
                        node["out"],
                        node["in"]
                    )
                } else {
                    format!(
                        "{path} {} {}{} holds {} out={} in={}",
                        node["role"].as_str().unwrap(),
                        node["name"].as_str().unwrap(),
                        match node["status"].as_str() {
                            Some(status) => format!(" [{status}]"),
                            None => String::new(),
                        },
                        node["symbols"],
                        node["out"],
                        node["in"]
                    )
                }
            })
            .collect()
    }
    /// The rows of one side of the selection: the symbol, how it is linked,
    /// and where it is when that is another file.
    fn told(focus: &Value, side: &str) -> Vec<String> {
        focus[side]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                format!(
                    "{} {} {}{}{}",
                    row["name"].as_str().unwrap(),
                    row["kind"].as_str().unwrap(),
                    match row["relation"].as_str() {
                        Some(relation) => relation.to_owned(),
                        None => format!("{} links", row["links"]),
                    },
                    if row["guessed"].as_u64().unwrap() > 0 {
                        format!(" guessed={}", row["guessed"])
                    } else {
                        String::new()
                    },
                    match row["file"].as_str() {
                        Some(file) => format!(" in {file}"),
                        None => String::new(),
                    }
                )
            })
            .collect()
    }
    fn short(edges: Vec<String>) -> Vec<String> {
        edges
            .into_iter()
            .map(|edge| edge.replace(CART, ""))
            .collect()
    }
    #[test]
    fn a_file_opens_to_its_classes_methods_and_functions() {
        let view = store().view("shop", CART, None, false, "");
        assert_eq!(
            (view["mode"].as_str(), view["scope"].as_str()),
            (Some("file"), Some(CART))
        );
        // Every symbol is a box, the most linked first: a method under the
        // name of its class, the file's own node under the file's name. The
        // comment the tool kept is not a symbol.
        assert_eq!(
            drawn(&view, true),
            [
                "#L19:.total() method Cart.total() out=2 in=3",
                "#L18:.add() method Cart.add() out=2 in=1",
                "#L26:.checkout() method Cart.checkout() out=3 in=0",
                "#L29:format_price() function format_price() out=1 in=1",
                "#L1:Cart.php file Cart.php out=1 in=0",
                "#L5:LIMIT name LIMIT out=0 in=1",
                "#L10:Status type Status holds 2 out=0 in=1",
                "#L10:Open case Open out=0 in=0",
                "#L10:Paid case Paid out=0 in=0",
                "#L12:Cart type Cart holds 3 out=0 in=0",
            ]
        );
        // The files around are the ones those symbols use or are used by.
        assert_eq!(
            drawn(&view, false),
            [
                "src/Cart/Price.php used Price.php holds 3 out=0 in=4",
                "src/Order/Notifier.php both Notifier.php holds 2 out=1 in=2",
                "tests/CartTest.php uses CartTest.php holds 3 out=3 in=0",
            ]
        );
        assert_eq!(
            short(rows(&view, "edges")),
            [
                "#L19:.total() -> src/Cart/Price.php 2/0",
                "tests/CartTest.php -> #L19:.total() 2/1",
                "#L18:.add() -> #L5:LIMIT 1/0",
                "#L18:.add() -> src/Cart/Price.php 1/0",
                "#L1:Cart.php -> src/Order/Notifier.php 1/0",
                "#L26:.checkout() -> #L10:Status 1/0",
                "#L26:.checkout() -> #L19:.total() 1/0",
                "#L26:.checkout() -> src/Order/Notifier.php 1/1",
                "#L29:format_price() -> src/Cart/Price.php 1/1",
                "src/Order/Notifier.php -> #L29:format_price() 1/0",
                "tests/CartTest.php -> #L18:.add() 1/0",
            ]
        );
        // A link to something outside the project, as an interface of the
        // language, has no place and is not counted.
        assert_eq!(
            view["file"],
            json!({
                "path": CART, "name": "Cart.php", "status": null, "touched": null,
                "symbols": 9, "types": 2, "methods": 3, "functions": 1, "headings": 0, "others": 3,
                "inside": 3, "out": 6, "in": 4, "uses": 2, "used_by": 2,
            })
        );
        assert_eq!(
            (
                view["children"].as_u64(),
                view["matching"].as_u64(),
                &view["around"]
            ),
            (Some(10), Some(10), &json!({"files": 3, "shown": 3}))
        );
        assert_eq!(
            view["crumbs"],
            json!([
                {"name": "shop", "path": ""},
                {"name": "src", "path": "src"},
                {"name": "Cart", "path": "src/Cart"},
                {"name": "Cart.php", "path": CART},
            ])
        );
        // Between files nothing changes: the links inside a file are not
        // links between files.
        let project = store().view("shop", "", None, false, "");
        assert_eq!(project["totals"]["links"], 10);
        assert_eq!(rows(&project, "edges"), ["tests -> src 3/1"]);
        // A folder still opens as a folder, and a path the graph does not
        // know opens the project.
        assert!(store().view("shop", "src/Cart", None, false, "")["mode"].is_null());
        assert_eq!(
            store().view("shop", "src/Cart/Gone.php", None, false, "")["scope"],
            ""
        );
    }
    #[test]
    fn a_selected_symbol_lists_what_it_uses_and_what_uses_it() {
        let index = store();
        let key = format!("{CART}#L19:.total()");
        let view = index.view("shop", CART, Some(&key), false, "");
        let focus = &view["focus"];
        assert_eq!(
            (
                focus["name"].as_str(),
                focus["kind"].as_str(),
                focus["owner"].as_str(),
                focus["line"].as_u64()
            ),
            (Some("Cart.total()"), Some("method"), Some("Cart"), Some(19))
        );
        // A call of itself is no link.
        assert_eq!(
            told(focus, "uses"),
            [
                "Price.of() method calls in src/Cart/Price.php",
                "Price.plus() method calls in src/Cart/Price.php",
            ]
        );
        // What was read comes before what was guessed, this file first.
        assert_eq!(
            told(focus, "used_by"),
            [
                "Cart.checkout() method calls",
                "CartTest.testTotal() method calls in tests/CartTest.php",
                "CartTest.testEmpty() method calls guessed=1 in tests/CartTest.php",
            ]
        );
        assert_eq!(focus["kinds"], json!([{"kind": "calls", "links": 2}]));
        // A symbol of another file is named the way that file selects it.
        assert_eq!(focus["uses"][0]["path"], "src/Cart/Price.php#L9:.of()");
        assert_eq!(
            (&focus["uses"][0]["inside"], &focus["used_by"][0]["inside"]),
            (&json!(false), &json!(true))
        );
        // A type lists what it holds, in the order of the file.
        let key = format!("{CART}#L12:Cart");
        let view = index.view("shop", CART, Some(&key), false, "");
        let held = view["focus"]["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                format!(
                    "{} {} {}",
                    row["name"].as_str().unwrap(),
                    row["kind"].as_str().unwrap(),
                    row["line"]
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            held,
            [
                ".add() method 18",
                ".total() method 19",
                ".checkout() method 26"
            ]
        );
        assert_eq!(view["focus"]["members_more"], 0);
        // Only what the tool read.
        let key = format!("{CART}#L26:.checkout()");
        let certain = index.view("shop", CART, Some(&key), true, "");
        assert_eq!(
            told(&certain["focus"], "uses"),
            ["Status type references", "Cart.total() method calls"]
        );
        assert_eq!(certain["file"]["out"], 4);
        // A name that is not a symbol of the file selects nothing.
        let view = index.view(
            "shop",
            CART,
            Some("src/Cart/Cart.php#L99:.gone()"),
            false,
            "",
        );
        assert!(view["focus"].is_null());
    }
    #[test]
    fn a_selected_file_around_lists_its_symbols_that_are_linked_with_the_open_one() {
        let index = store();
        let view = index.view("shop", CART, Some("src/Order/Notifier.php"), false, "");
        let focus = &view["focus"];
        assert_eq!(
            (
                focus["name"].as_str(),
                focus["kind"].as_str(),
                focus["role"].as_str(),
                focus["symbols"].as_u64()
            ),
            (Some("Notifier.php"), Some("file"), Some("both"), Some(2))
        );
        // Its symbols that use the open file, and its symbols the open file
        // uses, each with the number of links.
        assert_eq!(
            told(focus, "uses"),
            ["Notifier.paid() method 1 links in src/Order/Notifier.php"]
        );
        assert_eq!(
            told(focus, "used_by"),
            [
                "Notifier type 1 links in src/Order/Notifier.php",
                "Notifier.paid() method 1 links guessed=1 in src/Order/Notifier.php",
            ]
        );
        assert_eq!(focus["kinds"], json!([{"kind": "calls", "links": 1}]));
        let view = index.view("shop", CART, Some("src/Cart/Price.php"), false, "");
        assert_eq!(
            told(&view["focus"], "used_by"),
            [
                "Price.plus() method 2 links guessed=1 in src/Cart/Price.php",
                "Price type 1 links in src/Cart/Price.php",
                "Price.of() method 1 links in src/Cart/Price.php",
            ]
        );
        // A file that is not around the open one is not selected.
        assert!(index.view("shop", CART, Some("README.md"), false, "")["focus"].is_null());
    }
    #[test]
    fn a_filter_and_the_limit_choose_which_symbols_of_a_file_are_drawn() {
        let shop = store();
        let view = shop.view("shop", CART, None, false, "cart.");
        // The name a method is shown under includes its class. The file's
        // own node is named after the class too, and stays out.
        assert_eq!(
            drawn(&view, true),
            [
                "#L19:.total() method Cart.total() out=2 in=3",
                "#L18:.add() method Cart.add() out=2 in=1",
                "#L26:.checkout() method Cart.checkout() out=3 in=0",
            ]
        );
        assert_eq!(
            (view["children"].as_u64(), view["matching"].as_u64()),
            (Some(10), Some(3))
        );
        // The selected symbol is drawn whatever the filter says.
        let key = format!("{CART}#L29:format_price()");
        let view = shop.view("shop", CART, Some(&key), false, "status");
        assert_eq!(
            drawn(&view, true),
            [
                "#L10:Status type Status holds 2 out=0 in=1",
                "#L29:format_price() function format_price() out=1 in=1"
            ]
        );
        assert_eq!(view["focus"]["name"], "format_price()");
        // The view says that the filter would have left it out, and says
        // nothing of the kind where the selected symbol is drawn anyway.
        assert_eq!(view["beyond"], true);
        let key = format!("{CART}#L10:Status");
        assert_eq!(
            shop.view("shop", CART, Some(&key), false, "status")["beyond"],
            false
        );
        // A long file keeps its most linked symbols, and at least a few
        // places for the files around.
        let mut nodes = vec![
            json!({"id": "wide", "label": "wide.py", "source_file": "wide.py", "source_location": "L1"}),
        ];
        let mut links = vec![];
        for at in 0..NAMED + 10 {
            nodes.push(json!({"id": format!("f{at}"), "label": format!("f{at}()"), "source_file": "wide.py", "source_location": format!("L{}", at + 2)}));
            links.push(json!({"source": "wide", "target": format!("f{at}"), "relation": "contains", "source_file": "wide.py"}));
            nodes.push(json!({"id": format!("user{at}"), "label": format!("user{at}()"), "source_file": format!("users/u{at:02}.py"), "source_location": "L1"}));
            // The later a function stands, the more it is used.
            for user in 0..=at {
                links.push(json!({"source": format!("user{user}"), "target": format!("f{at}"), "relation": "calls", "source_file": format!("users/u{user:02}.py")}));
            }
        }
        let wide = index(json!({"nodes": nodes, "links": links}));
        let view = wide.view("wide", "wide.py", None, false, "");
        let symbols = drawn(&view, true);
        assert_eq!(symbols.len(), NAMED);
        assert_eq!(
            symbols[0],
            format!(
                "wide.py#L{}:f{}() function f{}() out=0 in={}",
                NAMED + 11,
                NAMED + 9,
                NAMED + 9,
                NAMED + 10
            )
        );
        assert_eq!(
            (view["children"].as_u64(), view["matching"].as_u64()),
            (Some(NAMED as u64 + 10), Some(NAMED as u64 + 10))
        );
        assert_eq!(
            view["around"],
            json!({"files": NAMED + 10, "shown": AROUND})
        );
        assert_eq!(view["nodes"].as_array().unwrap().len(), SHOWN);
        // The least used function is not drawn until it is selected.
        assert!(!symbols.iter().any(|symbol| symbol.contains("f0()")));
        let view = wide.view("wide", "wide.py", Some("wide.py#L2:f0()"), false, "");
        assert_eq!(drawn(&view, true).len(), NAMED + 1);
        assert_eq!(view["focus"]["used_by"][0]["name"], "user0()");
    }
    #[test]
    fn a_change_opens_a_file_too_and_marks_the_changed_files_around_it() {
        let index = store();
        let changed = [
            (CART, 'M'),
            ("src/Cart/Price.php", 'A'),
            ("src/Old.php", 'D'),
        ];
        let view = change(&index, &changed, CART, None);
        assert_eq!(
            (view["mode"].as_str(), view["file"]["status"].as_str()),
            (Some("file"), Some("M"))
        );
        assert_eq!(
            drawn(&view, false),
            [
                "src/Cart/Price.php used Price.php [A] holds 3 out=0 in=4",
                "src/Order/Notifier.php both Notifier.php holds 2 out=1 in=2",
                "tests/CartTest.php uses CartTest.php holds 3 out=3 in=0",
            ]
        );
        assert_eq!(
            view["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|node| node["changed"] == 1)
                .count(),
            1
        );
        // A file that did not change opens as well.
        let beside = change(&index, &changed, "tests/CartTest.php", None);
        assert_eq!(
            (beside["mode"].as_str(), &beside["file"]["status"]),
            (Some("file"), &Value::Null)
        );
        // The map of the change says what each of its files holds.
        let whole = change(&index, &changed, "", None);
        assert_eq!(whole["mode"], "changes");
        assert_eq!(
            whole["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|node| format!("{} {}", node["path"].as_str().unwrap(), node["symbols"]))
                .collect::<Vec<_>>(),
            [
                "src/Cart/Cart.php 9",
                "src/Cart/Price.php 3",
                "src/Order/Notifier.php 2",
                "tests/CartTest.php 3"
            ]
        );
        // And so does a folder of the project's map.
        let folder = index.view("shop", "src/Cart", Some("src/Cart/Cart.php"), false, "");
        assert_eq!(folder["nodes"][0]["symbols"], 9);
        assert_eq!(folder["focus"]["symbols"], 9);
    }
    /// A class with three methods, a function below it and a statement at
    /// the bottom, as a change leaves the file.
    const BASKET: &str = "<?php
namespace Shop;

use Shop\\Notifier;
use Shop\\Price;

/** A cart. */
class Cart
{
    private array $items = [];

    /** Adds a price. */
    public function add(Price $price): void
    {
        $this->items[] = $price;
    }

    public function total(): int
    {
        $sum = 0;
        foreach ($this->items as $item) {
            $sum += $item->cents() * 2;
        }
        return $sum;
    }

    /** Counts the items. */
    #[Pure]
    public function count(): int
    {
        return count($this->items);
    }
}

function format_price(Price $price): string
{
    return number_format($price->cents() / 100, 2);
}

echo format_price(new Price(1));
";
    fn basket() -> Index {
        let file = "src/Cart.php";
        let node = |id: &str, label: &str, line: u32| json!({"id": id, "label": label, "file_type": "code", "source_file": file, "source_location": format!("L{line}")});
        let holds = |holder: &str, relation: &str, held: &str| json!({"source": holder, "target": held, "relation": relation, "source_file": file});
        index(json!({
            "nodes": [
                node("file", "Cart.php", 1),
                node("cart", "Cart", 8),
                node("add", ".add()", 13),
                node("total", ".total()", 18),
                node("count", ".count()", 29),
                node("format", "format_price()", 35),
                {"id": "other", "label": "other.php", "source_file": "src/other.php", "source_location": "L1"},
                {"id": "user", "label": "user()", "source_file": "src/other.php", "source_location": "L3"}
            ],
            "links": [
                holds("file", "contains", "cart"),
                holds("file", "contains", "format"),
                holds("cart", "method", "add"),
                holds("cart", "method", "total"),
                holds("cart", "method", "count"),
                {"source": "other", "target": "user", "relation": "contains", "source_file": "src/other.php"},
                {"source": "user", "target": "format", "relation": "calls", "source_file": "src/other.php"},
                {"source": "file", "target": "user", "relation": "imports", "source_file": file}
            ]
        }))
    }
    fn hunk(at: u32, added: u32, removed: u32) -> Hunk {
        Hunk {
            at,
            added,
            removed,
            blank: false,
        }
    }
    /// The symbols of an opened file with what a change did to each: Git's
    /// letter, and how many symbols changed inside.
    fn marked(index: &Index, scope: &str, text: &str, hunks: &[Hunk]) -> (Vec<String>, Value) {
        let edited = Edited {
            new: false,
            text: text.to_owned(),
            hunks: hunks.to_vec(),
        };
        let changed = [(scope.to_owned(), 'M')];
        let view = index
            .symbols("shop", scope, &changed, Some(&edited), None, false, "")
            .unwrap();
        let symbols = view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node["symbol"] == true)
            .map(|node| {
                format!(
                    "{}{}{}",
                    node["name"].as_str().unwrap(),
                    match node["status"].as_str() {
                        Some(status) => format!(" {status}"),
                        None => String::new(),
                    },
                    match node["touched"].as_u64().unwrap() {
                        0 => String::new(),
                        inside => format!(" holds {inside}"),
                    }
                )
            })
            .collect();
        (symbols, view["file"]["touched"].clone())
    }
    #[test]
    fn a_change_marks_the_symbols_whose_lines_it_touched() {
        let index = basket();
        // An import is added, a line of `total` is rewritten, `count` is
        // inserted with the comment and the attribute above it, and the
        // statement at the bottom of the file is rewritten.
        let (symbols, reach) = marked(
            &index,
            "src/Cart.php",
            BASKET,
            &[
                hunk(5, 1, 0),
                hunk(22, 1, 1),
                hunk(26, 7, 0),
                hunk(40, 1, 1),
            ],
        );
        // What the change touched comes first. The class is touched through
        // its methods, the lines outside every symbol are the file's own,
        // and `add`, right above the inserted method, is not touched.
        assert_eq!(
            symbols,
            [
                "Cart.php M",
                "Cart holds 2",
                "Cart.total() M",
                "Cart.count() A",
                "format_price()",
                "Cart.add()",
            ]
        );
        assert_eq!(
            reach,
            json!({"symbols": 2, "added": 1, "modified": 1, "between": 0, "top": true})
        );
        // A property of the class is the class's own line.
        let (symbols, _) = marked(&index, "src/Cart.php", BASKET, &[hunk(10, 1, 1)]);
        assert_eq!(symbols[0], "Cart M");
        assert_eq!(
            symbols
                .iter()
                .filter(|symbol| symbol.contains(" M"))
                .count(),
            1
        );
        // The comment above a method is the method's, and a blank line is
        // nobody's.
        let (symbols, reach) = marked(
            &index,
            "src/Cart.php",
            BASKET,
            &[hunk(12, 1, 1), hunk(17, 1, 0)],
        );
        assert_eq!(symbols[..2], ["Cart holds 1", "Cart.add() M"]);
        assert_eq!(reach["symbols"], 1);
        // Lines removed inside a method touch it; lines removed between two
        // symbols are counted, and removed blank lines are not.
        let gone = |at: u32, blank: bool| Hunk {
            at,
            added: 0,
            removed: 2,
            blank,
        };
        let (symbols, reach) = marked(
            &index,
            "src/Cart.php",
            BASKET,
            &[gone(15, false), gone(33, false), gone(20, true)],
        );
        assert_eq!(symbols[..2], ["Cart holds 1", "Cart.add() M"]);
        assert_eq!(
            (reach["symbols"].as_u64(), reach["between"].as_u64()),
            (Some(1), Some(1))
        );
        // A function that is new down to its last line is added, and one
        // whose lines replaced others is modified.
        let (symbols, _) = marked(&index, "src/Cart.php", BASKET, &[hunk(34, 5, 0)]);
        assert_eq!(symbols[0], "format_price() A");
        let (symbols, _) = marked(&index, "src/Cart.php", BASKET, &[hunk(34, 5, 2)]);
        assert_eq!(symbols[0], "format_price() M");
    }
    #[test]
    fn a_line_past_the_end_of_a_symbol_belongs_to_what_holds_it() {
        let node = |id: &str, label: &str, line: u32| json!({"id": id, "label": label, "file_type": "code", "source_file": "app.py", "source_location": format!("L{line}")});
        let index = index(json!({
            "nodes": [
                node("app", "app.py", 1),
                node("outer", "outer()", 3),
                node("inner", "inner()", 5),
                node("other", "other()", 11),
                node("where", "bounded()", 15)
            ],
            "links": [
                {"source": "app", "target": "outer", "relation": "contains", "source_file": "app.py"},
                {"source": "outer", "target": "inner", "relation": "contains", "source_file": "app.py"},
                {"source": "app", "target": "other", "relation": "contains", "source_file": "app.py"},
                {"source": "app", "target": "where", "relation": "contains", "source_file": "app.py"}
            ]
        }));
        let text = "import os

def outer():
    x = 1
    def inner():
        return 2
    return inner() + x

# What follows.
@cached
def other(): return answer()
print(other())

# A signature that goes on at its own depth.
fn bounded<T>(x: T) -> u32
where
    T: Debug,
{
    1
}
";
        // The line after the inner function is the outer one's.
        let (symbols, _) = marked(&index, "app.py", text, &[hunk(7, 1, 1)]);
        assert_eq!(symbols[0], "outer() M");
        assert!(!symbols.iter().any(|symbol| symbol.starts_with("inner() ")));
        // A decorator is the function's below it; a statement after a
        // function that is whole on its line is the file's.
        let (symbols, reach) = marked(&index, "app.py", text, &[hunk(10, 1, 1), hunk(12, 1, 1)]);
        assert_eq!(symbols[0], "other() M");
        assert_eq!(
            reach,
            json!({"symbols": 1, "added": 0, "modified": 1, "between": 0, "top": true})
        );
        // A body that opens after a clause at the signature's depth is
        // still the function's.
        let (symbols, reach) = marked(&index, "app.py", text, &[hunk(19, 1, 1)]);
        assert_eq!(symbols[0], "bounded() M");
        assert_eq!(reach["top"], false);
        // A symbol the graph places beyond the text is not marked: the
        // line that changed is the file's own.
        let (symbols, reach) = marked(&index, "app.py", "import os\n", &[hunk(1, 1, 1)]);
        assert!(
            symbols.iter().all(|symbol| !symbol.contains(' ')),
            "{symbols:?}"
        );
        assert_eq!(
            (&reach["symbols"], &reach["top"]),
            (&json!(0), &json!(true))
        );
    }
    #[test]
    fn a_new_file_is_new_throughout_and_an_unread_change_marks_nothing() {
        let index = basket();
        let changed = [("src/Cart.php".to_owned(), 'A')];
        let new = Edited {
            new: true,
            ..Edited::default()
        };
        let view = index
            .symbols(
                "shop",
                "src/Cart.php",
                &changed,
                Some(&new),
                None,
                false,
                "",
            )
            .unwrap();
        assert!(
            view["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|node| node["symbol"] == true)
                .all(|node| node["status"] == "A" && node["changed"] == 1)
        );
        assert_eq!(
            view["file"]["touched"],
            json!({"symbols": 5, "added": 5, "modified": 0, "between": 0, "top": true})
        );
        // The selected symbol and the rows of its side say it too.
        let view = index
            .symbols(
                "shop",
                "src/Cart.php",
                &changed,
                Some(&new),
                Some("src/Cart.php#L8:Cart"),
                false,
                "",
            )
            .unwrap();
        assert_eq!(
            (
                &view["focus"]["status"],
                &view["focus"]["touched"],
                &view["focus"]["members"][0]["changed"]
            ),
            (&json!("A"), &json!(3), &json!(true))
        );
        // Without what the change did, nothing is marked and the view says
        // that it does not know.
        let plain = index
            .symbols("shop", "src/Cart.php", &changed, None, None, false, "")
            .unwrap();
        assert!(plain["file"]["touched"].is_null());
        assert!(
            plain["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .all(|node| node["status"].is_null())
        );
        assert!(
            index
                .symbols("shop", "src", &changed, None, None, false, "")
                .is_none()
        );
        assert_eq!(index.opens("/src/Cart.php/"), Some("src/Cart.php"));
        assert_eq!(index.opens("src"), None);
    }
    #[test]
    fn a_line_is_read_for_its_depth_and_for_what_it_is() {
        let read = lines(
            "fn a() {\n\tlet x = 1;\n  \n    // note\n    #[test]\n}\nwhere\nend_of(x)\n  * a note\n*p = 1;\n",
        );
        let told = read
            .iter()
            .map(|line| (line.indent, line.kind, line.whole))
            .collect::<Vec<_>>();
        assert_eq!(
            told,
            [
                (0, Row::Code, false),
                (4, Row::Code, true),
                (2, Row::Blank, false),
                (4, Row::Comment, false),
                (4, Row::Comment, false),
                (0, Row::Closer, true),
                (0, Row::Closer, false),
                (0, Row::Code, true),
                (2, Row::Comment, false),
                (0, Row::Code, true),
            ]
        );
    }
    #[test]
    fn what_a_symbol_is_comes_from_the_graph_and_odd_graphs_are_read() {
        let graph = index(json!({
            "nodes": [
                // A document: the page is the file, the headings its symbols.
                {"id": "page", "label": "docs/guide.md", "file_type": "document", "node_kind": "page", "source_file": "docs/guide.md", "source_location": "L1"},
                {"id": "h1", "label": "Guide", "file_type": "document", "node_kind": "heading", "source_file": "docs/guide.md", "source_location": "L1"},
                {"id": "h2", "label": "Install", "file_type": "document", "node_kind": "heading", "source_file": "docs/guide.md", "source_location": "L5"},
                // No node for the file, a label that is a number, a place
                // that is a number, fields of an unexpected shape.
                {"id": "a", "label": 42, "source_file": "odd.py", "source_location": 7, "file_type": ["code"], "_callable_class": "yes", "node_kind": {"a": 1}},
                {"id": "b", "label": "run()", "source_file": "odd.py", "source_location": "L9-L12", "_callable_class": null},
                {"id": "c", "label": "inner()", "source_file": "odd.py", "source_location": "L10"},
                // Two symbols of one name on one line, and two that hold
                // each other.
                {"id": "d", "label": "same", "source_file": "odd.py", "source_location": "L20"},
                {"id": "e", "label": "same", "source_file": "odd.py", "source_location": "L20"},
                {"id": "f", "label": "loop", "source_file": "odd.py"},
                {"id": "g", "label": "pool", "source_file": "odd.py"},
                {"id": "long", "label": "ж".repeat(200), "source_file": "odd.py", "source_location": "L30"}
            ],
            "links": [
                {"source": "page", "target": "h1", "relation": "contains", "source_file": "docs/guide.md"},
                {"source": "h1", "target": "h2", "relation": "contains", "source_file": "docs/guide.md"},
                {"source": "h2", "target": "b", "relation": "references", "source_file": "docs/guide.md"},
                {"source": "b", "target": "c", "relation": "contains", "source_file": "odd.py"},
                {"source": "b", "target": "b", "relation": "contains", "source_file": "odd.py"},
                {"source": "f", "target": "g", "relation": "contains", "source_file": "odd.py"},
                {"source": "g", "target": "f", "relation": "contains", "source_file": "odd.py"},
                // A link found in a third file says nothing of these two.
                {"source": "d", "target": "e", "relation": "calls", "source_file": "docs/guide.md"}
            ]
        }));
        let guide = graph.view("p", "docs/guide.md", None, false, "");
        let listed = |view: &Value| {
            view["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|node| node["symbol"] == true)
                .map(|node| {
                    format!(
                        "{} {} {}",
                        node["path"].as_str().unwrap(),
                        node["kind"].as_str().unwrap(),
                        node["owner"].as_str().unwrap_or("-")
                    )
                })
                .collect::<Vec<_>>()
        };
        // The page has no link of its own, so it is no box.
        assert_eq!(
            listed(&guide),
            [
                "docs/guide.md#L5:Install heading Guide",
                "docs/guide.md#L1:Guide heading -"
            ]
        );
        assert_eq!(
            (&guide["file"]["headings"], &guide["file"]["others"]),
            (&json!(2), &json!(0))
        );
        let odd = graph.view("p", "odd.py", None, false, "");
        let long = format!("odd.py#L30:{} name -", "ж".repeat(NAME_LIMIT / 2));
        assert_eq!(
            listed(&odd),
            [
                "odd.py#L9:run() function -",
                "odd.py#loop name pool",
                "odd.py#pool name loop",
                "odd.py#L7:42 name -",
                "odd.py#L10:inner() function run()",
                "odd.py#L20:same name -",
                "odd.py#L20:same~2 name -",
                long.as_str(),
            ]
        );
        assert_eq!(graph.unplaced, 0);
        assert_eq!(odd["file"]["inside"], 0);
        // The second of two that share a name is selected by its own.
        let second = graph.view("p", "odd.py", Some("odd.py#L20:same~2"), false, "");
        assert_eq!(second["focus"]["path"], "odd.py#L20:same~2");
        assert_eq!(line(Some("L42")), 42);
        assert_eq!(
            (line(Some("12-20")), line(Some("top")), line(None)),
            (12, 0, 0)
        );
        assert_eq!(clip("жук", 3), "ж");
    }
    #[test]
    fn a_graph_is_read_once_until_its_file_changes() {
        let folder = tempfile::tempdir().unwrap();
        let graph = folder.path().join("graph.json");
        let maps = Maps::default();
        assert!(maps.index("p", &graph, folder.path()).is_err());
        std::fs::write(
            &graph,
            json!({"nodes": [{"id": "a", "source_file": "a.py"}], "links": []}).to_string(),
        )
        .unwrap();
        let first = maps.index("p", &graph, folder.path()).unwrap();
        let again = maps.index("p", &graph, folder.path()).unwrap();
        assert!(Arc::ptr_eq(&first, &again));
        std::fs::write(&graph, json!({"nodes": [{"id": "a", "source_file": "a.py"}, {"id": "b", "source_file": "b.py"}], "links": []}).to_string()).unwrap();
        let changed = maps.index("p", &graph, folder.path()).unwrap();
        assert!(!Arc::ptr_eq(&first, &changed));
        assert_eq!(changed.files.len(), 2);
        maps.forget("p");
        assert!(!Arc::ptr_eq(
            &changed,
            &maps.index("p", &graph, folder.path()).unwrap()
        ));
    }
    /// The links drawn among followed symbols, by the names of their ends.
    fn calls(view: &Value) -> Vec<String> {
        let name = |key: &Value| {
            view["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|node| &node["path"] == key)
                .map(|node| node["name"].as_str().unwrap().to_owned())
                .unwrap()
        };
        view["edges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|edge| {
                format!(
                    "{} -> {} {}/{}",
                    name(&edge["from"]),
                    name(&edge["to"]),
                    edge["links"],
                    edge["guessed"]
                )
            })
            .collect()
    }
    const TOTAL_KEY: &str = "src/Cart/Cart.php#L19:.total()";
    const CHECKOUT_KEY: &str = "src/Cart/Cart.php#L26:.checkout()";
    #[test]
    fn the_links_of_a_method_are_followed_among_symbols() {
        let index = store();
        let follow = |out: &[&str], into: &[&str], focus: Option<&str>, certain: bool| {
            index
                .follow(
                    "shop",
                    TOTAL_KEY,
                    &named(out),
                    &named(into),
                    &[],
                    focus,
                    certain,
                )
                .unwrap()
        };
        // The method alone at first, with how many it uses and how many use
        // it. The way back leads to its file, where it lies.
        let alone = follow(&[], &[], None, false);
        assert_eq!(
            (&alone["mode"], &alone["scope"]),
            (&json!("follow"), &json!(CART))
        );
        assert_eq!(steps(&alone), ["Cart.total() +2/3"]);
        assert_eq!(
            alone["followed"],
            json!({
                "path": TOTAL_KEY, "name": "Cart.total()", "folder": false, "symbol": true,
                "kind": "method", "file": CART, "line": 19, "uses": 2, "users": 3,
                "shown": 1, "limit": FOLLOWED, "step": STEP, "full": false, "aside": 0
            })
        );
        assert_eq!(
            alone["crumbs"],
            json!([
                {"name": "shop", "path": ""},
                {"name": "src", "path": "src"},
                {"name": "Cart", "path": "src/Cart"},
                {"name": "Cart.php", "path": CART},
                {"name": "Cart.total()", "path": TOTAL_KEY}
            ])
        );
        let start = &alone["nodes"][0];
        assert_eq!(
            (
                &start["path"],
                &start["symbol"],
                &start["kind"],
                &start["line"]
            ),
            (
                &json!(TOTAL_KEY),
                &json!(true),
                &json!("method"),
                &json!(19)
            )
        );
        assert_eq!(
            (
                &start["file"],
                &start["owner"],
                &start["start"],
                &start["changed"]
            ),
            (&json!(CART), &json!("Cart"), &json!(true), &json!(0))
        );
        assert_eq!((&start["out"], &start["in"]), (&json!(2), &json!(3)));
        assert_eq!(calls(&alone), Vec::<String>::new());
        // One step each way: what it calls, in the order of their lines, and
        // what calls it. What was read from the code comes before what was
        // guessed, and its own file before the others.
        let both = follow(&[TOTAL_KEY], &[TOTAL_KEY], None, false);
        assert_eq!(
            steps(&both),
            [
                "Cart.total() +0/0 out in",
                "Price.of() +0/0",
                "Price.plus() +0/1",
                "Cart.checkout() +2/0",
                "CartTest.testTotal() +1/0",
                "CartTest.testEmpty() +0/0",
            ]
        );
        assert_eq!(
            sorted(calls(&both)),
            [
                "Cart.checkout() -> Cart.total() 1/0",
                "Cart.total() -> Price.of() 1/0",
                "Cart.total() -> Price.plus() 1/0",
                "CartTest.testEmpty() -> Cart.total() 1/1",
                "CartTest.testTotal() -> Cart.total() 1/0",
            ]
        );
        assert_eq!(both["followed"]["shown"], 6);
        // Each symbol is named as the picture of its own file selects it.
        assert_eq!(both["nodes"][1]["path"], "src/Cart/Price.php#L9:.of()");
        assert_eq!(both["nodes"][1]["file"], "src/Cart/Price.php");
        // A step further from one of them: what checkout uses besides joins
        // after what was there, its own file first and the guess last.
        let further = follow(&[TOTAL_KEY, CHECKOUT_KEY], &[TOTAL_KEY], None, false);
        assert_eq!(
            steps(&further)[3..],
            [
                "Cart.checkout() +0/0 out",
                "CartTest.testTotal() +1/0",
                "CartTest.testEmpty() +0/0",
                "Status +0/0",
                "Notifier.paid() +1/0",
            ]
        );
        assert!(calls(&further).contains(&"Cart.checkout() -> Notifier.paid() 1/1".to_owned()));
        assert!(calls(&further).contains(&"Cart.checkout() -> Status 1/0".to_owned()));
        // Without what the tool inferred the guessed caller is not there.
        let read = follow(&[TOTAL_KEY], &[TOTAL_KEY], None, true);
        assert_eq!(read["followed"]["users"], 2);
        assert!(!steps(&read).iter().any(|step| step.contains("testEmpty")));
        // A selected symbol lists both sides, each with how it is linked,
        // where it lies and whether the picture draws it.
        let chosen = follow(&[TOTAL_KEY], &[], Some(TOTAL_KEY), false);
        let focus = &chosen["focus"];
        assert_eq!(
            (
                &focus["name"],
                &focus["kind"],
                &focus["file"],
                &focus["start"]
            ),
            (
                &json!("Cart.total()"),
                &json!("method"),
                &json!(CART),
                &json!(true)
            )
        );
        let told = |side: &str| {
            focus[side]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| {
                    format!(
                        "{} {} in {}{}{}",
                        row["relation"].as_str().unwrap(),
                        row["name"].as_str().unwrap(),
                        row["file"].as_str().unwrap(),
                        if row["inside"] == true { " drawn" } else { "" },
                        if row["guessed"] == 1 { " guessed" } else { "" },
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            told("uses"),
            [
                "calls Price.of() in src/Cart/Price.php drawn",
                "calls Price.plus() in src/Cart/Price.php drawn",
            ]
        );
        assert_eq!(
            told("used_by"),
            [
                "calls Cart.checkout() in src/Cart/Cart.php",
                "calls CartTest.testTotal() in tests/CartTest.php",
                "calls CartTest.testEmpty() in tests/CartTest.php guessed",
            ]
        );
        assert_eq!(focus["used_by"][0]["path"], CHECKOUT_KEY);
        assert_eq!(focus["kinds"], json!([{"kind": "calls", "links": 2}]));
        // A symbol the picture does not draw is not selected.
        assert_eq!(
            follow(&[], &[], Some(CHECKOUT_KEY), false)["focus"],
            Value::Null
        );
        // A class is followed by its own links, not by those of its methods,
        // and the node of a file stands for what the file imports.
        let class = index
            .follow(
                "shop",
                "src/Order/Notifier.php#L8:Notifier",
                &[],
                &named(&["src/Order/Notifier.php#L8:Notifier"]),
                &[],
                None,
                false,
            )
            .unwrap();
        assert_eq!(steps(&class), ["Notifier +0/0 in", "Cart.php +0/0"]);
        assert_eq!(calls(&class), ["Cart.php -> Notifier 1/0"]);
        assert_eq!(class["nodes"][1]["kind"], "file");
        // What names no symbol is followed as a file or a folder, or not at
        // all.
        let none = |root: &str| index.follow("shop", root, &[], &[], &[], None, false);
        assert_eq!(none(CART).unwrap()["followed"]["folder"], false);
        assert!(none(CART).unwrap()["followed"].get("symbol").is_none());
        assert!(none("src/Cart/Cart.php#L19:.gone()").is_none());
        assert!(none("src/Cart/Nothing.php#L1:x").is_none());
    }
    #[test]
    fn followed_symbols_stop_at_a_full_picture_and_count_the_links_left_aside() {
        // One function that a hundred others call, each from a file of its
        // own, and that calls two.
        let mut nodes = vec![
            json!({"id": "hub", "label": "hub()", "source_file": "lib/hub.py", "source_location": "L1"}),
            json!({"id": "left", "label": "left()", "source_file": "lib/hub.py", "source_location": "L5"}),
            json!({"id": "right", "label": "right()", "source_file": "lib/hub.py", "source_location": "L9"}),
        ];
        let call = |from: &str, to: &str, file: &str| json!({"source": from, "target": to, "relation": "calls", "confidence": "EXTRACTED", "source_file": file});
        let mut links = vec![
            call("hub", "left", "lib/hub.py"),
            call("hub", "right", "lib/hub.py"),
            call("left", "right", "lib/hub.py"),
        ];
        for at in 0..100 {
            let (id, file) = (format!("u{at:03}"), format!("users/u{at:03}.py"));
            nodes.push(json!({"id": id, "label": format!("user{at:03}()"), "source_file": file, "source_location": "L1"}));
            links.push(call(&id, "hub", &file));
        }
        let index = index(json!({"nodes": nodes, "links": links}));
        let hub = "lib/hub.py#L1:hub()";
        let follow = |out: &[&str], into: &[&str]| {
            index
                .follow("hub", hub, &named(out), &named(into), &[], None, false)
                .unwrap()
        };
        // A step shows the next twelve of a side, and says how many wait.
        let once = follow(&[], &[hub]);
        assert_eq!(steps(&once)[0], format!("hub() +2/{} in", 100 - STEP));
        assert_eq!(once["nodes"].as_array().unwrap().len(), 1 + STEP);
        assert_eq!(once["nodes"][1]["name"], "user000()");
        // Named twice, the side shows twelve more.
        let twice = follow(&[], &[hub, hub]);
        assert_eq!(twice["nodes"].as_array().unwrap().len(), 1 + 2 * STEP);
        // The picture holds so many boxes and says that it is full.
        let all = vec![hub; 10];
        let full = follow(&[hub], &all);
        assert_eq!(full["nodes"].as_array().unwrap().len(), FOLLOWED);
        assert_eq!(full["followed"]["full"], true);
        assert_eq!(full["followed"]["users"], 100);
        // What it calls joined first; the link between the two it calls is
        // not one of its own, and waits for a side that holds it.
        assert_eq!(steps(&full)[1], "left() +1/0");
        assert_eq!(steps(&full)[2], "right() +0/1");
        assert_eq!(full["followed"]["aside"], 1);
        let opened = follow(&[hub, "lib/hub.py#L5:left()"], &[]);
        assert!(calls(&opened).contains(&"left() -> right() 1/0".to_owned()));
        assert_eq!(opened["followed"]["aside"], 0);
    }
    #[test]
    fn followed_symbols_carry_what_a_chosen_change_did_to_them() {
        let index = basket();
        let format = "src/Cart.php#L35:format_price()";
        let changed = [
            ("src/Cart.php".to_owned(), 'M'),
            ("src/other.php".to_owned(), 'A'),
        ];
        let asked = RefCell::new(Vec::new());
        let edited = |path: &str| {
            asked.borrow_mut().push(path.to_owned());
            Some(match path {
                // The lines of the function replaced others.
                "src/Cart.php" => Edited {
                    new: false,
                    text: BASKET.to_owned(),
                    hunks: vec![hunk(34, 5, 2)],
                },
                _ => Edited {
                    new: true,
                    ..Edited::default()
                },
            })
        };
        let view = index
            .follow_with(
                "shop",
                format,
                &[],
                &named(&[format]),
                &changed,
                &edited,
                Some(format),
                false,
            )
            .unwrap();
        let marks = view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| {
                format!(
                    "{} {} {}",
                    node["name"].as_str().unwrap(),
                    node["changed"],
                    node["status"].as_str().unwrap_or("-")
                )
            })
            .collect::<Vec<_>>();
        // The function is modified, and its caller lies in a new file.
        assert_eq!(marks, ["format_price() 1 M", "user() 1 A"]);
        assert_eq!(view["focus"]["status"], "M");
        // A row beside it says the same of the symbol it names.
        assert_eq!(view["focus"]["used_by"][0]["changed"], true);
        // Each file of the change is asked for once, however many of its
        // symbols are drawn and listed.
        assert_eq!(
            sorted(asked.borrow().clone()),
            ["src/Cart.php", "src/other.php"]
        );
        // A function the change left as it was carries no mark, though its
        // file changed; and a file outside the change is not asked for.
        let asked = RefCell::new(Vec::new());
        let elsewhere = |path: &str| {
            asked.borrow_mut().push(path.to_owned());
            Some(Edited {
                new: false,
                text: BASKET.to_owned(),
                hunks: vec![hunk(22, 1, 1)],
            })
        };
        let view = index
            .follow_with(
                "shop",
                format,
                &[],
                &named(&[format]),
                &changed[..1],
                &elsewhere,
                None,
                false,
            )
            .unwrap();
        assert_eq!(
            (&view["nodes"][0]["changed"], &view["nodes"][0]["status"]),
            (&json!(0), &Value::Null)
        );
        assert_eq!(view["nodes"][1]["changed"], 0);
        assert_eq!(*asked.borrow(), ["src/Cart.php"]);
        // Nor does a row that names it, from the other end of the link.
        let from_user = index
            .follow_with(
                "shop",
                "src/other.php#L3:user()",
                &[],
                &[],
                &changed[..1],
                &elsewhere,
                Some("src/other.php#L3:user()"),
                false,
            )
            .unwrap();
        let row = &from_user["focus"]["uses"][0];
        assert_eq!(
            (&row["name"], &row["changed"], &row["inside"]),
            (&json!("format_price()"), &json!(false), &json!(false))
        );
        // Where the change cannot be told, as with a graph older than the
        // files, links are followed without marks.
        let plain = index
            .follow(
                "shop",
                format,
                &[],
                &named(&[format]),
                &changed,
                None,
                false,
            )
            .unwrap();
        assert_eq!(plain["nodes"][0]["changed"], 0);
        assert_eq!(steps(&plain), ["format_price() +0/0 in", "user() +0/1"]);
    }
    /// What a search found, in its order: what each place is, its name and
    /// where it lies.
    fn sought(index: &Index, query: &str) -> Vec<String> {
        index.find(query)["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                let text = |key: &str| row[key].as_str().unwrap_or("").to_owned();
                match row["file"].as_str() {
                    Some(file) => format!(
                        "{} {} in {file}:{}",
                        text("what"),
                        text("name"),
                        row["line"]
                    ),
                    None => format!("{} {} in {}", text("what"), text("name"), text("in")),
                }
            })
            .collect()
    }
    const TOTAL: &str = "method Cart.total() in src/Cart/Cart.php:19";
    const TEST_TOTAL: &str = "method CartTest.testTotal() in tests/CartTest.php:7";
    #[test]
    fn a_search_finds_folders_files_and_names_wherever_they_lie() {
        let index = store();
        // The folder, the file without its ending and the class are all
        // called Cart; what only starts with it comes after them. A method
        // of Cart is not called Cart, and letters match whatever their case.
        let cart = [
            "folder Cart in src",
            "file Cart.php in src/Cart",
            "type Cart in src/Cart/Cart.php:12",
            "file CartTest.php in tests",
            "type CartTest in tests/CartTest.php:5",
        ];
        assert_eq!(sought(&index, "cart"), cart);
        assert_eq!(sought(&index, "  CART "), cart);
        let found = index.find("  CART ");
        assert_eq!(
            (&found["mode"], &found["query"]),
            (&json!("find"), &json!("CART"))
        );
        assert_eq!(
            (
                &found["found"],
                &found["folders"],
                &found["files"],
                &found["names"]
            ),
            (&json!(5), &json!(1), &json!(2), &json!(2))
        );
        // A method is named without its class and its brackets, and a word
        // inside a name written likeThis or like_this is found as well.
        assert_eq!(sought(&index, "total"), [TOTAL, TEST_TOTAL]);
        assert_eq!(
            sought(&index, "price"),
            [
                "file Price.php in src/Cart",
                "type Price in src/Cart/Price.php:4",
                "function format_price() in src/Cart/Cart.php:29",
            ]
        );
        // With its class before it, it is named as it is shown.
        assert_eq!(sought(&index, "cart.to"), [TOTAL]);
        assert_eq!(sought(&index, "t.total"), [TOTAL]);
        // A constant and a case are names; of two called alike the more
        // linked comes first.
        assert_eq!(
            sought(&index, "limit"),
            ["name LIMIT in src/Cart/Cart.php:5"]
        );
        assert_eq!(
            sought(&index, "paid"),
            [
                "method Notifier.paid() in src/Order/Notifier.php:11",
                "case Paid in src/Cart/Cart.php:10",
            ]
        );
        // The node that stands for a file is the file, found once, and a
        // comment the tool kept is no name.
        assert_eq!(sought(&index, "cart.php"), ["file Cart.php in src/Cart"]);
        assert!(sought(&index, "cents").is_empty());
        // Nothing is called that, and nothing was asked.
        assert_eq!(index.find("zzz")["found"], 0);
        let nothing = index.find("   ");
        assert_eq!(
            (&nothing["found"], &nothing["query"], &nothing["rows"]),
            (&json!(0), &json!(""), &json!([]))
        );
    }
    #[test]
    fn further_words_of_a_search_say_where_a_place_lies() {
        let index = store();
        // Each word stands in the name or on the way to it: in the class of
        // a method, in the name of its file, in a folder above.
        assert_eq!(sought(&index, "total cart"), [TOTAL, TEST_TOTAL]);
        assert_eq!(sought(&index, "total test"), [TEST_TOTAL]);
        assert_eq!(sought(&index, "total tests"), [TEST_TOTAL]);
        assert_eq!(sought(&index, "total price.php"), Vec::<String>::new());
        assert_eq!(
            sought(&index, "php order"),
            ["file Notifier.php in src/Order"]
        );
        // A word that only says where names nothing by itself.
        assert_eq!(sought(&index, "order"), ["folder Order in src"]);
        // A word with a slash is the end of a path: the folder a place lies
        // in, and the start of its name.
        assert_eq!(sought(&index, "cart/price"), ["file Price.php in src/Cart"]);
        assert_eq!(
            sought(&index, "CART/PRICE.PHP"),
            ["file Price.php in src/Cart"]
        );
        assert_eq!(
            sought(&index, "src/"),
            ["folder Cart in src", "folder Order in src"]
        );
        assert_eq!(sought(&index, "order/"), ["file Notifier.php in src/Order"]);
        assert!(
            sought(&index, "rder/notifier").len() == 1
                && sought(&index, "/rder/notifier").is_empty()
        );
        // At the front it is the start of a folder's name, or the project.
        assert_eq!(
            sought(&index, "/cart"),
            [
                "folder Cart in src",
                "file Cart.php in src/Cart",
                "file CartTest.php in tests",
            ]
        );
        assert_eq!(sought(&index, "/tests"), ["folder tests in "]);
        // For a name such a word says where its file lies.
        assert_eq!(sought(&index, "src/cart/ total"), [TOTAL]);
        assert!(sought(&index, "src/cart/total").is_empty());
    }
    #[test]
    fn a_search_lists_the_best_named_first_and_counts_what_it_does_not_list() {
        let node = |id: &str, label: &str, file: &str| json!({"id": id, "label": label, "file_type": "code", "source_file": file, "source_location": "L1"});
        let call = |from: &str, to: &str| json!({"source": from, "target": to, "relation": "calls", "confidence": "EXTRACTED", "source_file": "c.py"});
        let index = index(json!({
            "nodes": [
                node("part", "Reprice", "b.py"),
                node("snake", "net_price()", "a.py"),
                node("camel", "netPrice()", "b.py"),
                node("list", "PriceList", "a.py"),
                node("plan", "PricePlan", "b.py"),
                node("whole", "Price", "a.py"),
                node("one", "one()", "c.py"),
                node("two", "two()", "c.py"),
                node("three", "three()", "c.py"),
                node("small", "index()", "a.py"),
                node("capital", "Index", "b.py"),
            ],
            "links": [
                call("one", "part"), call("two", "part"), call("three", "part"),
                call("one", "plan"), call("two", "plan"),
                call("one", "list"),
                call("one", "small"), call("two", "small"),
            ]
        }));
        // The whole name, then what starts with it, then a word inside a
        // name, then any part of one. Reprice is the most linked of all and
        // comes last; among those that start alike the more linked is first,
        // and of two equally linked the shorter name.
        assert_eq!(
            sought(&index, "price"),
            [
                "name Price in a.py:1",
                "name PricePlan in b.py:1",
                "name PriceList in a.py:1",
                "function netPrice() in b.py:1",
                "function net_price() in a.py:1",
                "name Reprice in b.py:1",
            ]
        );
        // Both are called index, and the more linked comes first; typed
        // with a capital, the word puts first what is written that way.
        assert_eq!(
            sought(&index, "index"),
            ["function index() in a.py:1", "name Index in b.py:1"]
        );
        assert_eq!(
            sought(&index, "Index"),
            ["name Index in b.py:1", "function index() in a.py:1"]
        );
        // What a method is called counts, not what its class is called.
        let classes = self::index(json!({
            "nodes": [
                node("tests", "IndexTests", "t.py"),
                node("method", ".index()", "t.py"),
                node("other", "user()", "t.py"),
                node("type", "Index", "i.py"),
            ],
            "links": [
                {"source": "tests", "target": "method", "relation": "method", "source_file": "t.py"},
                {"source": "other", "target": "method", "relation": "calls", "source_file": "t.py"},
            ]
        }));
        assert_eq!(
            sought(&classes, "index"),
            [
                "method IndexTests.index() in t.py:1",
                "name Index in i.py:1",
                "type IndexTests in t.py:1",
            ]
        );
        assert_eq!(
            sought(&classes, "Index"),
            [
                "name Index in i.py:1",
                "method IndexTests.index() in t.py:1",
                "type IndexTests in t.py:1",
            ]
        );
        // A word that says where is looked for, as typed, on the way.
        assert_eq!(
            sought(&classes, "index Tests")[0],
            "method IndexTests.index() in t.py:1"
        );
        // A capital orders what is found and leaves nothing out.
        assert_eq!(sought(&index, "Index a.py"), ["function index() in a.py:1"]);
        // A capital after capitals begins a word where small letters follow.
        assert_eq!(
            super::named("XMLParser", "xmlparser", "parser"),
            Some(Named::Word)
        );
        assert_eq!(
            super::named("XMLParser", "xmlparser", "lparser"),
            Some(Named::Part)
        );
        assert_eq!(super::named("v2Api", "v2api", "api"), Some(Named::Word));
        // Letters of more than one byte are walked whole.
        assert_eq!(super::named("Ééé", "ééé", "é"), Some(Named::Start));
        assert_eq!(super::named("жук", "жук", "ук"), Some(Named::Part));
        // Where the lower case is longer than the name, capitals are not
        // looked for: they do not lie where the lower case says.
        assert_eq!(super::named("aStan", "astan", "stan"), Some(Named::Word));
        assert_eq!(
            super::named("İaStan", "i\u{307}astan", "stan"),
            Some(Named::Part)
        );
        assert_eq!(super::named("any", "any", ""), None);
        // More is found than a list holds: the best are listed, all counted.
        let wide = office(FOUND + 20);
        let found = wide.find("r");
        let rows = found["rows"].as_array().unwrap();
        assert_eq!(rows.len(), FOUND);
        // The reports, each a file and a name in it, and their folder; and
        // `core`, `orders`, `users` and `store` with the files of the four.
        assert_eq!(found["found"], json!(1 + 2 * (FOUND + 20) + 8));
        assert_eq!(
            (&found["folders"], &found["files"], &found["names"]),
            (&json!(1), &json!(FOUND + 24), &json!(FOUND + 24))
        );
        // The folder holds the links of all its files.
        assert_eq!(sought(&wide, "r")[0], "folder reports in ");
        assert_eq!(rows[0]["files"], json!(FOUND + 20));
    }
    #[test]
    fn what_a_search_found_is_selected_in_the_picture_it_lies_in() {
        let index = store();
        // A name is told by what the picture of its file selects it by.
        let found = index.find("total");
        let row = &found["rows"][0];
        assert_eq!(
            (&row["path"], &row["file"], &row["what"], &row["line"]),
            (
                &json!("src/Cart/Cart.php#L19:.total()"),
                &json!(CART),
                &json!("method"),
                &json!(19)
            )
        );
        // It is shown with its class, and says where its own name begins.
        assert_eq!(
            (&row["name"], &row["own"]),
            (&json!("Cart.total()"), &json!(5))
        );
        assert_eq!(index.find("price")["rows"][1]["own"], 0);
        let view = index.view("shop", CART, row["path"].as_str(), false, "");
        assert_eq!(view["focus"]["path"], row["path"]);
        // A file and a folder are told by their paths, with where they lie.
        let found = index.find("cart");
        assert_eq!(
            found["rows"][0],
            json!({"path": "src/Cart", "name": "Cart", "what": "folder", "in": "src", "files": 2})
        );
        assert_eq!(
            found["rows"][1],
            json!({"path": CART, "name": "Cart.php", "what": "file", "in": "src/Cart", "symbols": 9})
        );
        let view = index.view("shop", "src/Cart", Some(CART), false, "");
        assert_eq!(view["focus"]["path"], CART);
        // What is left out with the tests is not found either.
        let plain = index.plain_with(&|_| None);
        assert_eq!(sought(&plain, "total"), [TOTAL]);
        assert_eq!(sought(&plain, "cart").len(), 3);
    }
    #[test]
    fn a_selected_place_is_drawn_whatever_the_limit_and_the_filter_say() {
        let files = (0..SHOWN + 5)
            .map(|at| json!({"id": format!("n{at}"), "source_file": format!("lib/f{at:02}.php")}))
            .collect::<Vec<_>>();
        let links = (1..SHOWN + 5)
            .map(|at| json!({"source": format!("n{at}"), "target": "n0", "relation": "imports", "source_file": format!("lib/f{at:02}.php")}))
            .collect::<Vec<_>>();
        let wide = index(json!({"nodes": files, "edges": links}));
        let last = format!("lib/f{:02}.php", SHOWN + 4);
        let paths = |view: &Value| {
            view["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|node| node["path"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        // The last of the files is no one of the most linked.
        let view = wide.view("wide", "lib", None, false, "");
        assert!(!paths(&view).contains(&last));
        assert_eq!(view["beyond"], false);
        // Selected, it is drawn beside them, with its link, and the view
        // says that one more is drawn than the limit lets.
        let view = wide.view("wide", "lib", Some(&last), false, "");
        assert_eq!(view["beyond"], true);
        assert_eq!(paths(&view).len(), SHOWN + 1);
        assert_eq!(paths(&view).last(), Some(&last));
        assert_eq!(view["focus"]["path"], json!(last));
        assert!(rows(&view, "edges").contains(&format!("{last} -> lib/f00.php 1/0")));
        // A filter that leaves it out does not hide it.
        let view = wide.view("wide", "lib", Some(&last), false, "f0");
        assert_eq!(paths(&view).len(), 11);
        assert_eq!(paths(&view).last(), Some(&last));
        assert_eq!(
            (&view["matching"], &view["beyond"]),
            (&json!(10), &json!(true))
        );
        // One that is drawn anyway is drawn once.
        let view = wide.view("wide", "lib", Some("lib/f01.php"), false, "");
        assert_eq!(paths(&view).len(), SHOWN);
        assert_eq!(view["beyond"], false);
        // So it is in a folder that is spread out and shows a part of what
        // it holds.
        let index = wheel(1, HELD + 5);
        let spoke = format!("p0/f{:02}.rs", HELD + 4);
        let spread = ["p0".to_owned()];
        let view = index.view_with("wheel", "", &spread, Some(&spoke), false, "");
        assert!(held(&view).contains(&format!("{spoke} in p0")));
        assert_eq!(framed(&view), [format!("p0 {}/{}", HELD + 1, HELD + 5)]);
        // A place outside the open folder is listed, not drawn.
        let shop = shop();
        let outside = shop.view("shop", "src", Some("tests"), false, "");
        assert_eq!(outside["focus"]["path"], "tests");
        assert_eq!(outside["beyond"], false);
        assert_eq!(
            rows(&outside, "nodes"),
            rows(&shop.view("shop", "src", None, false, ""), "nodes")
        );
    }
}
