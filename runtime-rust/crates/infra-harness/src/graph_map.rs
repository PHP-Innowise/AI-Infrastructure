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
use crate::{HarnessError, Result};
use serde::de::{DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt,
    path::Path,
    sync::{Arc, Mutex},
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
    pairs: HashMap<(u32, u32), Pair>,
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
        let (from, to) = (
            self.symbols[source as usize].file,
            self.symbols[target as usize].file,
        );
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
        // Between files only what crosses a file boundary counts.
        if from == to {
            return;
        }
        let pair = self.pairs.entry((from, to)).or_default();
        if read {
            pair.read += 1;
        } else {
            pair.guessed += 1;
        }
        match pair.kinds.iter_mut().find(|(known, _)| *known == kind) {
            Some((_, count)) => *count += 1,
            None => pair.kinds.push((kind, 1)),
        }
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
/// Files of a graph and the links between them, each pair counted once, and
/// the symbols of each file with the links between symbols.
#[derive(Debug)]
pub struct Index {
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
fn name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
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
    /// table from symbol to file, links are counted as they pass.
    pub fn read(source: impl std::io::Read, root: &Path) -> Result<Self> {
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
            pairs: HashMap::new(),
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
        let mut pairs = builder
            .pairs
            .into_iter()
            .map(|((from, to), pair)| (from, to, pair))
            .collect::<Vec<_>>();
        pairs.sort_by_key(|(from, to, _)| (*from, *to));
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
        let uses = builder
            .uses
            .into_iter()
            .map(|(from, to, kind, guessed)| {
                (place[from as usize], place[to as usize], kind, guessed)
            })
            .collect();
        Ok(Self {
            files: builder.files,
            pairs,
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
    /// one place outside the folder, uses and is used by.
    pub fn view(
        &self,
        project: &str,
        scope: &str,
        focus: Option<&str>,
        certain: bool,
        filter: &str,
    ) -> Value {
        if let Some(file) = self.opened(scope) {
            return self.file(project, file, &HashMap::new(), None, focus, certain, filter);
        }
        let scope = scope.trim_matches('/');
        let scope = if self.folder(scope) { scope } else { "" };
        let mut tallies: BTreeMap<&str, Tally> = BTreeMap::new();
        for (id, file) in self.files.iter().enumerate() {
            let (key, inside) = group(scope, file);
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
            let (source, _) = group(scope, &self.files[*from as usize]);
            let (target, _) = group(scope, &self.files[*to as usize]);
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
        children.sort_by(|(left, a), (right, b)| {
            (b.out + b.into + b.within)
                .cmp(&(a.out + a.into + a.within))
                .then(left.cmp(right))
        });
        children.truncate(SHOWN);
        let shown = children
            .iter()
            .map(|(key, _)| **key)
            .collect::<std::collections::BTreeSet<_>>();
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
            "nodes": children.iter().map(|(key, tally)| json!({
                "path": key,
                "name": name(key),
                "folder": tally.folder,
                "files": tally.files,
                "symbols": tally.symbols,
                "within": tally.within,
                "out": tally.out,
                "in": tally.into,
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
        let symbol = &self.symbols[at as usize];
        let label = self.label(at);
        match symbol.kind {
            Kind::File => name(&self.files[symbol.file as usize]).to_owned(),
            Kind::Method if symbol.parent != NONE => {
                let holder = self.label(symbol.parent);
                if label.starts_with('.') {
                    format!("{holder}{label}")
                } else {
                    format!("{holder}.{label}")
                }
            }
            _ => label.to_owned(),
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
        if let Some(at) = chosen
            && !listed.contains(&at)
        {
            listed.push(at);
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
            json!({"files": 6, "symbols": 9, "relations": 12, "links": 6, "read": 3, "guessed": 3, "unplaced": 1})
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
}
