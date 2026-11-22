//! The design and architecture patterns a graph shows signs of.
//!
//! A pattern is told by how types stand to each other, and the graph says
//! that link by link: one type extends or implements another, holds it in a
//! field, takes it as a parameter, returns it, makes it. Where those links
//! alone tell a pattern it is found by them. A repository, a builder, a
//! singleton and the pair of a message and its handler are told by a name
//! as well, and such a finding says so: a name is a promise, not a proof.
//!
//! Nothing here reads a method's body. Two patterns built the same way are
//! one finding under the commoner name, and the words of that finding name
//! the others.
use super::*;

/// What of a type a link names as its type: the third part of a tie.
pub(super) const FIELD: u16 = 1;
pub(super) const PARAMETER: u16 = 2;
pub(super) const RESULT: u16 = 3;
pub(super) const GENERIC: u16 = 4;
pub(super) const MADE: u16 = 5;
pub(super) const ROUTE: u16 = 6;
pub(super) const DI_BINDING: u16 = 7;
pub(super) const EVENT_HINT: u16 = 8;
pub(super) const TABLE_READ: u16 = 9;
pub(super) const TABLE_WRITE: u16 = 10;
/// Findings of one pattern a list names at once, the largest first.
pub const LISTED: usize = 30;
/// Boxes a picture of one finding holds.
pub const DRAWN: usize = 40;
/// Sources read to see what a type is declared as; past that the graph alone
/// is believed.
const READ: usize = 4000;
/// Names of the method a singleton hands itself out by.
const ACCESSORS: &[&str] = &[
    "getinstance",
    "get_instance",
    "instance",
    "sharedinstance",
    "shared_instance",
    "getsingleton",
];
/// Names of the method that makes its own type, where a language has no
/// constructor of the type's name.
const MAKERS: &[&str] = &["new", "default", "create", "make"];
/// Names of the method a type is built by.
const CONSTRUCTORS: &[&str] = &["__construct", "__init__", "constructor", "new", "init"];
/// What stores a link's context for the patterns, where they read it.
pub(super) fn how(context: &str) -> Option<u16> {
    match context {
        "field" => Some(FIELD),
        "parameter_type" => Some(PARAMETER),
        "return_type" => Some(RESULT),
        "generic_arg" => Some(GENERIC),
        "constructor" => Some(MADE),
        "route" | "exposes_route" => Some(ROUTE),
        "di_binding" | "di_binds" => Some(DI_BINDING),
        "event" | "publishes_event" | "handles_event" => Some(EVENT_HINT),
        "table_read" | "reads_table" => Some(TABLE_READ),
        "table_write" | "writes_table" => Some(TABLE_WRITE),
        _ => None,
    }
}
/// What a part is in a finding: for one of them, and for several.
type Role = (&'static str, &'static str);
const ABSTRACTION: Role = ("abstraction", "abstractions");
const IMPLEMENTATION: Role = ("implementation", "implementations");
const USER: Role = ("user", "users");
const DECORATOR: Role = ("decorator", "decorators");
const FACTORY: Role = ("factory", "factories");
const PRODUCT: Role = ("product", "products");
const MADE_TYPE: Role = ("made type", "made types");
const TEMPLATE: Role = ("template", "templates");
const FILLER: Role = ("type that fills the steps", "types that fill the steps");
const BUILDER: Role = ("builder", "builders");
const BUILT: Role = ("built type", "built types");
const SINGLE: Role = ("singleton", "singletons");
const EVENT: Role = ("event", "events");
const LISTENER: Role = ("listener", "listeners");
const RAISER: Role = ("type that raises it", "types that raise it");
const MESSAGE: Role = ("message", "messages");
const HANDLER: Role = ("handler", "handlers");
const SENDER: Role = ("type that sends it", "types that send it");
const REPOSITORY: Role = ("repository", "repositories");
const STORED: Role = ("stored type", "stored types");
const ROUTE_HANDLER: Role = ("route handler", "route handlers");
const ROUTE_ENDPOINT: Role = ("route", "routes");
const TABLE: Role = ("table", "tables");
const TABLE_READER: Role = ("reader", "readers");
const TABLE_WRITER: Role = ("writer", "writers");
const DI_PROVIDER: Role = ("container binding", "container bindings");
const DI_SERVICE: Role = ("bound service", "bound services");
/// One pattern that is looked for.
struct Sort {
    id: &'static str,
    name: &'static str,
    /// An architecture pattern arranges parts of a system, a design
    /// pattern a few types.
    architecture: bool,
    tells: &'static str,
}
const REPOSITORIES: usize = 0;
const EVENTS: usize = 1;
const COMMANDS: usize = 2;
const STRATEGY: usize = 3;
const DECORATORS: usize = 4;
const FACTORIES: usize = 5;
const TEMPLATES: usize = 6;
const BUILDERS: usize = 7;
const SINGLETONS: usize = 8;
const ADAPTERS: usize = 9;
const ROUTES: usize = 10;
const DATA_ACCESS: usize = 11;
const DI_BINDINGS: usize = 12;
const SORTS: &[Sort] = &[
    Sort {
        id: "repository",
        name: "Repository",
        architecture: true,
        tells: "A type named a repository, with the types its methods give and take, and the types that hold it. Found by the name.",
    },
    Sort {
        id: "events",
        name: "Events and listeners",
        architecture: true,
        tells: "A type that others take in a handling method, where its name or theirs says event or listener. Observer and publish-subscribe are built so. Found by names and by the parameter that joins them.",
    },
    Sort {
        id: "commands",
        name: "Commands and handlers",
        architecture: true,
        tells: "A message and the one type named after it as its handler, which takes it. A command bus and CQRS are built so. Found by the name and by the parameter that joins the two.",
    },
    Sort {
        id: "strategy",
        name: "Strategy",
        architecture: false,
        tells: "An abstraction with several implementations that other types hold or take as the abstraction, not as one of them. State and Bridge are built the same way.",
    },
    Sort {
        id: "decorator",
        name: "Decorator",
        architecture: false,
        tells: "A type that is the abstraction it holds, so it can wrap any other of that kind. Proxy and Composite are built the same way.",
    },
    Sort {
        id: "factory",
        name: "Factory",
        architecture: false,
        tells: "A method that returns an abstraction and makes the types that implement it. A type named a factory that makes other types is found by its name.",
    },
    Sort {
        id: "template",
        name: "Template method",
        architecture: false,
        tells: "A method of a base type that calls other methods of that type, which the types under it define.",
    },
    Sort {
        id: "builder",
        name: "Builder",
        architecture: false,
        tells: "A type named a builder, with the type its build method gives. Found by the name.",
    },
    Sort {
        id: "singleton",
        name: "Singleton",
        architecture: false,
        tells: "A type with a method named for handing out its one instance, and the types that ask for it. Found by that name.",
    },
    Sort {
        id: "adapter",
        name: "Adapter",
        architecture: false,
        tells: "A type implements an abstraction and delegates a method to a different type it holds. This is a candidate based on the links shown.",
    },
    Sort {
        id: "routes",
        name: "HTTP & RPC routes",
        architecture: true,
        tells: "Controllers, handlers, or modules that expose HTTP or RPC routes.",
    },
    Sort {
        id: "data_access",
        name: "Database table access",
        architecture: true,
        tells: "Database tables paired with the types or functions that read from or write to them.",
    },
    Sort {
        id: "di_bindings",
        name: "DI container bindings",
        architecture: true,
        tells: "Service providers or containers that bind interfaces and concrete services via dependency injection.",
    },
];
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct SemanticSummary {
    routes: Vec<String>,
    di_bindings: Vec<String>,
    events_published: Vec<String>,
    events_handled: Vec<String>,
    tables_read: Vec<String>,
    tables_written: Vec<String>,
    insights: Vec<String>,
}
impl SemanticSummary {
    fn is_empty(&self) -> bool {
        self.routes.is_empty()
            && self.di_bindings.is_empty()
            && self.events_published.is_empty()
            && self.events_handled.is_empty()
            && self.tables_read.is_empty()
            && self.tables_written.is_empty()
            && self.insights.is_empty()
    }
    fn to_json(&self) -> Option<Value> {
        if self.is_empty() {
            return None;
        }
        Some(json!({
            "routes": self.routes,
            "di_bindings": self.di_bindings,
            "events_published": self.events_published,
            "events_handled": self.events_handled,
            "tables_read": self.tables_read,
            "tables_written": self.tables_written,
            "insights": self.insights,
        }))
    }
}
/// One pattern found in the graph.
#[derive(Debug)]
struct Found {
    sort: usize,
    /// The type the finding is named after.
    anchor: u32,
    /// What takes part, the anchor first, each once, with what it is here.
    parts: Vec<(u32, Role)>,
    /// What joins the parts: the one that needs, the one that is needed,
    /// how, and whether the tool only inferred that.
    links: Vec<(u32, u32, &'static str, bool)>,
    /// What tells it, in words.
    told: Vec<String>,
    /// A name is among what tells it.
    named: bool,
    /// The files its parts lie in, in order, each once.
    files: Vec<u32>,
    /// Optional semantic framework and database summary when semantic edges touch this finding.
    semantic_summary: SemanticSummary,
}
/// The patterns of a graph.
#[derive(Debug, Default)]
pub(super) struct Patterns {
    found: Vec<Found>,
    /// The layers its files are filed in, which a folder is asked for.
    pub(super) layers: layers::Layers,
}
/// The graph as the patterns read it: types, what each lies in, and how
/// they stand to each other.
struct Read<'a> {
    index: &'a Index,
    typed: Vec<bool>,
    /// The type a symbol lies in, the symbol itself where it is one, or
    /// `NONE`.
    owner: Vec<u32>,
    members: HashMap<u32, Vec<u32>>,
    /// The types a type extends or implements, and those that do so to it:
    /// with whether it implements, and whether that was only inferred.
    supers: HashMap<u32, Vec<(u32, bool, bool)>>,
    subs: HashMap<u32, Vec<(u32, bool, bool)>>,
    /// By the symbol that names a type: the type, how, and whether inferred.
    names: HashMap<u32, Vec<(u32, u16, bool)>>,
    /// By the type that is held or taken: the type that does so, how.
    held: HashMap<u32, Vec<(u32, &'static str, bool)>>,
    makes: HashMap<u32, Vec<(u32, bool)>>,
    made: HashMap<u32, Vec<(u32, bool)>>,
    calls: HashMap<u32, Vec<(u32, bool)>>,
    called: HashMap<u32, Vec<(u32, bool)>>,
    text: &'a dyn Fn(&str) -> Option<String>,
    sources: RefCell<HashMap<u32, Option<Rc<String>>>>,
    /// Whether a type stands above others as an abstraction or a base, once
    /// it was asked.
    bases: RefCell<HashMap<u32, bool>>,
    /// Semantic framework & DB edges collected from relation kinds and `ties`.
    routes: BTreeMap<u32, Vec<(u32, bool)>>,
    di_binds: Vec<(u32, u32, bool)>,
    event_publishers: BTreeMap<u32, Vec<(u32, bool)>>,
    event_listeners: BTreeMap<u32, Vec<(u32, bool)>>,
    table_readers: BTreeMap<u32, Vec<(u32, bool)>>,
    table_writers: BTreeMap<u32, Vec<(u32, bool)>>,
    sym_routes: HashMap<u32, Vec<String>>,
    sym_di: HashMap<u32, Vec<String>>,
    sym_events_pub: HashMap<u32, Vec<String>>,
    sym_events_handle: HashMap<u32, Vec<String>>,
    sym_tables_read: HashMap<u32, Vec<String>>,
    sym_tables_write: HashMap<u32, Vec<String>>,
}
fn clean_semantic_label(raw: &str, prefix: &str) -> String {
    let trimmed = raw.trim();
    if !prefix.is_empty()
        && let Some(rest) = trimmed.strip_prefix(prefix)
    {
        return rest.trim().to_owned();
    }
    trimmed.to_owned()
}
fn push_unique_edge(list: &mut Vec<(u32, bool)>, target: u32, guessed: bool) {
    match list.iter_mut().find(|(existing, _)| *existing == target) {
        Some((_, existing_guessed)) => *existing_guessed &= guessed,
        None => list.push((target, guessed)),
    }
}
fn push_unique_text(map: &mut HashMap<u32, Vec<String>>, sym: u32, label: String) {
    if label.is_empty() {
        return;
    }
    let entry = map.entry(sym).or_default();
    if !entry.contains(&label) {
        entry.push(label);
    }
}
fn is_listener_symbol(label: &str, path: &str) -> bool {
    let name = plain(label);
    ["Listener", "Subscriber", "Observer"]
        .iter()
        .any(|end| name.ends_with(end))
        || foldered(
            path,
            &[
                "listener",
                "listeners",
                "subscriber",
                "subscribers",
                "eventsubscriber",
                "eventlistener",
                "observer",
                "observers",
            ],
        )
}
/// What the source says a type is declared as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Declared {
    /// A trait, an interface, a protocol, an abstract class.
    Abstract,
    Plain,
    /// The source could not be read, or the graph has no line for the type.
    Unread,
}
/// The name of a method or a function without what marks it as one.
fn bare(label: &str) -> &str {
    let label = label.strip_prefix('.').unwrap_or(label);
    label.strip_suffix("()").unwrap_or(label)
}
/// A name without what a generic type carries after it.
fn plain(label: &str) -> &str {
    label.split(['<', '(']).next().unwrap_or(label).trim()
}
/// Whether a word stands in a line as a word of its own.
fn worded(line: &str, word: &str) -> bool {
    line.match_indices(word).any(|(at, _)| {
        let before = line[..at].chars().next_back();
        let after = line[at + word.len()..].chars().next();
        !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
            && !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}
/// Whether a path has a folder of one of these names, in any letter case.
fn foldered(path: &str, names: &[&str]) -> bool {
    path.split('/')
        .rev()
        .skip(1)
        .any(|part| names.iter().any(|name| part.eq_ignore_ascii_case(name)))
}
/// Whether a method is named as one that handles what it takes.
fn handles(name: &str) -> bool {
    let stepped = |prefix: &str| {
        name.strip_prefix(prefix).is_some_and(|rest| {
            rest.starts_with(|c: char| c.is_uppercase()) || rest.starts_with('_')
        })
    };
    ["handle", "__invoke", "invoke", "notify", "update", "listen"].contains(&name)
        || stepped("on")
        || stepped("handle")
        || stepped("when")
}
impl<'a> Read<'a> {
    fn new(index: &'a Index, certain: bool, text: &'a dyn Fn(&str) -> Option<String>) -> Self {
        let symbols = &index.symbols;
        let kind = |name: &str| index.kinds.iter().position(|kind| kind == name);
        let over = [kind("inherits"), kind("extends"), kind("implements")];
        let called = kind("calls");
        // A type is what the graph calls one, and what stands as one: a Rust
        // struct with no method of its own still implements and holds.
        let mut typed = symbols
            .iter()
            .map(|symbol| symbol.kind == Kind::Type)
            .collect::<Vec<_>>();
        let stands = |at: u32| matches!(symbols[at as usize].kind, Kind::Type | Kind::Name);
        for (from, to, kind, _) in &index.uses {
            if over.contains(&Some(*kind as usize)) && stands(*from) && stands(*to) {
                typed[*from as usize] = true;
                typed[*to as usize] = true;
            }
        }
        for (from, _, how, _) in &index.ties {
            if *how == FIELD && stands(*from) {
                typed[*from as usize] = true;
            }
        }
        let owner = (0..symbols.len() as u32)
            .map(|at| {
                let mut next = at;
                // A graph can name two symbols as each other's holder.
                for _ in 0..16 {
                    if typed[next as usize] {
                        return next;
                    }
                    next = symbols[next as usize].parent;
                    if next == NONE {
                        break;
                    }
                }
                NONE
            })
            .collect::<Vec<_>>();
        let mut read = Self {
            index,
            typed,
            owner,
            members: HashMap::new(),
            supers: HashMap::new(),
            subs: HashMap::new(),
            names: HashMap::new(),
            held: HashMap::new(),
            makes: HashMap::new(),
            made: HashMap::new(),
            calls: HashMap::new(),
            called: HashMap::new(),
            text,
            sources: RefCell::new(HashMap::new()),
            bases: RefCell::new(HashMap::new()),
            routes: BTreeMap::new(),
            di_binds: Vec::new(),
            event_publishers: BTreeMap::new(),
            event_listeners: BTreeMap::new(),
            table_readers: BTreeMap::new(),
            table_writers: BTreeMap::new(),
            sym_routes: HashMap::new(),
            sym_di: HashMap::new(),
            sym_events_pub: HashMap::new(),
            sym_events_handle: HashMap::new(),
            sym_tables_read: HashMap::new(),
            sym_tables_write: HashMap::new(),
        };
        for (at, symbol) in symbols.iter().enumerate() {
            if matches!(symbol.kind, Kind::Method | Kind::Function)
                && symbol.parent != NONE
                && read.typed[symbol.parent as usize]
            {
                read.members
                    .entry(symbol.parent)
                    .or_default()
                    .push(at as u32);
            }
        }
        let semantic_tag = |relation: &str| -> Option<u16> {
            match relation {
                "exposes_route" | "route" | "calls_route" => Some(ROUTE),
                "di_binds" | "di_binding" => Some(DI_BINDING),
                "publishes_event" => Some(u16::MAX - 1),
                "handles_event" => Some(u16::MAX),
                "event" => Some(EVENT_HINT),
                "reads_table" | "table_read" => Some(TABLE_READ),
                "writes_table" | "table_write" => Some(TABLE_WRITE),
                _ => None,
            }
        };
        for (from, to, kind, guessed) in &index.uses {
            if certain && *guessed {
                continue;
            }
            let (from, to) = (*from, *to);
            if over.contains(&Some(*kind as usize)) {
                if read.typed[from as usize] && read.typed[to as usize] {
                    let implements = over[2] == Some(*kind as usize);
                    read.supers
                        .entry(from)
                        .or_default()
                        .push((to, implements, *guessed));
                    read.subs
                        .entry(to)
                        .or_default()
                        .push((from, implements, *guessed));
                }
            } else if called == Some(*kind as usize) {
                if read.typed[to as usize] {
                    read.make(from, to, *guessed);
                    continue;
                }
                if !matches!(symbols[to as usize].kind, Kind::Method | Kind::Function) {
                    continue;
                }
                read.calls.entry(from).or_default().push((to, *guessed));
                read.called.entry(to).or_default().push((from, *guessed));
                // Calling the method a type is made by makes the type.
                let made = read.owner[to as usize];
                if made != NONE
                    && made != read.owner[from as usize]
                    && MAKERS.contains(&bare(index.label(to)).to_ascii_lowercase().as_str())
                {
                    read.make(from, made, *guessed);
                }
            } else if let Some(tag) = semantic_tag(index.kinds[*kind as usize].as_str()) {
                read.record_semantic(tag, from, to, *guessed);
            }
        }
        for (from, to, how, guessed) in &index.ties {
            if certain && *guessed {
                continue;
            }
            let (from, to) = (*from, *to);
            if matches!(
                *how,
                ROUTE | DI_BINDING | EVENT_HINT | TABLE_READ | TABLE_WRITE
            ) {
                read.record_semantic(*how, from, to, *guessed);
                continue;
            }
            if !read.typed[to as usize] {
                continue;
            }
            if *how == MADE {
                read.make(from, to, *guessed);
                continue;
            }
            read.names
                .entry(from)
                .or_default()
                .push((to, *how, *guessed));
            let holder = read.owner[from as usize];
            if holder == NONE || holder == to {
                continue;
            }
            let way = match *how {
                FIELD => "holds",
                PARAMETER => "takes",
                // A generic argument of the type itself is a field's.
                GENERIC if holder == from => "holds",
                GENERIC => "names",
                _ => continue,
            };
            read.held
                .entry(to)
                .or_default()
                .push((holder, way, *guessed));
        }
        read
    }
    fn actor(&self, sym: u32) -> u32 {
        match self.owner[sym as usize] {
            NONE => sym,
            owner => owner,
        }
    }
    fn record_semantic(&mut self, tag: u16, from: u32, to: u32, guessed: bool) {
        let src = self.actor(from);
        let dst = self.actor(to);
        let resolved_tag = if tag == EVENT_HINT {
            if handles(bare(self.label(from)))
                || is_listener_symbol(self.label(src), self.path(src))
            {
                u16::MAX
            } else {
                u16::MAX - 1
            }
        } else {
            tag
        };
        match resolved_tag {
            ROUTE => {
                push_unique_edge(self.routes.entry(src).or_default(), to, guessed);
                let label = clean_semantic_label(self.label(to), "route:");
                push_unique_text(&mut self.sym_routes, src, label.clone());
                push_unique_text(&mut self.sym_routes, to, label);
            }
            DI_BINDING => {
                match self
                    .di_binds
                    .iter_mut()
                    .find(|(s, d, _)| *s == src && *d == dst)
                {
                    Some((_, _, g)) => *g &= guessed,
                    None => self.di_binds.push((src, dst, guessed)),
                }
                let text = format!(
                    "{} -> {}",
                    self.index.shown(src),
                    clean_semantic_label(self.label(dst), "service:")
                );
                push_unique_text(&mut self.sym_di, src, text.clone());
                push_unique_text(&mut self.sym_di, dst, text);
            }
            t if t == u16::MAX - 1 => {
                push_unique_edge(self.event_publishers.entry(to).or_default(), src, guessed);
                let label = clean_semantic_label(self.label(to), "event:");
                push_unique_text(&mut self.sym_events_pub, src, label.clone());
                push_unique_text(&mut self.sym_events_pub, to, label);
            }
            t if t == u16::MAX => {
                push_unique_edge(self.event_listeners.entry(to).or_default(), src, guessed);
                let label = clean_semantic_label(self.label(to), "event:");
                push_unique_text(&mut self.sym_events_handle, src, label.clone());
                push_unique_text(&mut self.sym_events_handle, to, label);
            }
            TABLE_READ => {
                push_unique_edge(self.table_readers.entry(to).or_default(), src, guessed);
                let label = clean_semantic_label(self.label(to), "table:");
                push_unique_text(&mut self.sym_tables_read, src, label.clone());
                push_unique_text(&mut self.sym_tables_read, to, label);
            }
            TABLE_WRITE => {
                push_unique_edge(self.table_writers.entry(to).or_default(), src, guessed);
                let label = clean_semantic_label(self.label(to), "table:");
                push_unique_text(&mut self.sym_tables_write, src, label.clone());
                push_unique_text(&mut self.sym_tables_write, to, label);
            }
            _ => {}
        }
    }
    /// The types that have methods, in the order of the graph.
    fn types(&self) -> Vec<u32> {
        let mut types = self.members.keys().copied().collect::<Vec<_>>();
        types.sort_unstable();
        types
    }
    fn make(&mut self, from: u32, to: u32, guessed: bool) {
        self.makes.entry(from).or_default().push((to, guessed));
        self.made.entry(to).or_default().push((from, guessed));
    }
    fn label(&self, at: u32) -> &'a str {
        self.index.label(at)
    }
    fn path(&self, at: u32) -> &'a str {
        &self.index.files[self.index.symbols[at as usize].file as usize]
    }
    /// The line a symbol is declared on, as the source has it.
    fn line(&self, at: u32) -> Option<String> {
        let symbol = &self.index.symbols[at as usize];
        if symbol.line == 0 {
            return None;
        }
        let mut sources = self.sources.borrow_mut();
        if !sources.contains_key(&symbol.file) && sources.len() >= READ {
            return None;
        }
        let source = sources
            .entry(symbol.file)
            .or_insert_with(|| (self.text)(self.path(at)).map(Rc::new))
            .clone()?;
        source
            .lines()
            .nth(symbol.line as usize - 1)
            .map(str::to_owned)
    }
    fn declared(&self, at: u32) -> Declared {
        if self.index.symbols[at as usize].is_abstract {
            return Declared::Abstract;
        }
        if self.index.backend.as_deref() == Some(crate::graph_ir::NATIVE_IR_TOOL) {
            return Declared::Plain;
        }
        match self.line(at) {
            None => Declared::Unread,
            Some(line) => {
                let abstraction = [
                    "trait",
                    "interface",
                    "protocol",
                    "abstract",
                    "ABC",
                    "Protocol",
                ]
                .iter()
                .any(|word| worded(&line, word))
                    || line.contains("ABCMeta");
                if abstraction {
                    Declared::Abstract
                } else {
                    Declared::Plain
                }
            }
        }
    }
    /// Whether other types stand under this one as under an abstraction or a
    /// base. In Rust only a trait is implemented; an `impl From<Other>` that
    /// the tool reads as implementing `Other` names a plain type.
    fn above(&self, at: u32) -> bool {
        if let Some(known) = self.bases.borrow().get(&at) {
            return *known;
        }
        let found = self.stands_above(at);
        self.bases.borrow_mut().insert(at, found);
        found
    }
    fn stands_above(&self, at: u32) -> bool {
        let Some(under) = self.subs.get(&at) else {
            return false;
        };
        match self.declared(at) {
            Declared::Abstract => true,
            Declared::Plain if self.path(at).ends_with(".rs") => false,
            Declared::Plain if under.iter().all(|(_, implements, _)| *implements) => {
                // Another language says `interface` on that line.
                false
            }
            _ => self.members.contains_key(&at),
        }
    }
    /// The types that hold or take a type, each once, a holder before one
    /// that only takes or names it.
    fn holders(&self, at: u32) -> Vec<(u32, &'static str, bool)> {
        let mut found: Vec<(u32, &'static str, bool)> = Vec::new();
        for (holder, way, guessed) in self.held.get(&at).into_iter().flatten() {
            match found.iter_mut().find(|(known, ..)| known == holder) {
                Some(known) => {
                    if known.1 != "holds" && *way == "holds" {
                        (known.1, known.2) = (*way, *guessed);
                    }
                }
                None => found.push((*holder, *way, *guessed)),
            }
        }
        found
    }
    /// The types a symbol names in one of these ways.
    fn named(&self, at: u32, ways: &[u16]) -> Vec<(u32, bool)> {
        self.names
            .get(&at)
            .into_iter()
            .flatten()
            .filter(|(_, how, _)| ways.contains(how))
            .map(|(to, _, guessed)| (*to, *guessed))
            .collect()
    }
    /// The types whose methods make a type, each once.
    fn makers(&self, at: u32) -> Vec<(u32, bool)> {
        let mut found: Vec<(u32, bool)> = Vec::new();
        for (from, guessed) in self.made.get(&at).into_iter().flatten() {
            let holder = self.owner[*from as usize];
            if holder != NONE && holder != at && !found.iter().any(|(known, _)| *known == holder) {
                found.push((holder, *guessed));
            }
        }
        found
    }
    fn di_binds_pair(&self, from: u32, to: u32) -> Option<bool> {
        self.di_binds
            .iter()
            .find(|(src, dst, _)| *src == from && *dst == to)
            .map(|(_, _, guessed)| *guessed)
    }
    /// Whether a type keeps the abstraction it stands under: in a field,
    /// taken where it is built, or bound via DI.
    fn wraps(&self, at: u32, abstraction: u32) -> Option<bool> {
        let kept = self
            .named(at, &[FIELD, GENERIC])
            .into_iter()
            .find(|(to, _)| *to == abstraction);
        kept.or_else(|| {
            self.members
                .get(&at)
                .into_iter()
                .flatten()
                .find_map(|method| {
                    let name = bare(self.label(*method));
                    (CONSTRUCTORS.contains(&name) || name == plain(self.label(at)))
                        .then(|| {
                            self.named(*method, &[PARAMETER])
                                .into_iter()
                                .find(|(to, _)| *to == abstraction)
                        })
                        .flatten()
                })
        })
        .map(|(_, guessed)| guessed)
        .or_else(|| self.di_binds_pair(at, abstraction))
    }
    /// `handled` are the messages that have a handler of their own.
    fn strategies(&self, found: &mut Vec<Found>, handled: &BTreeSet<u32>) {
        for (abstraction, under) in &self.subs {
            if under.len() < 2 || !self.above(*abstraction) {
                continue;
            }
            // Entities behind a shared interface are data that something
            // takes, not ways of doing one thing, and so are the commands
            // and queries a bus takes. Where most of the types under the
            // abstraction are the model's own classes or messages with a
            // handler, a test double aside, there is no strategy.
            let made = under
                .iter()
                .map(|(sub, ..)| *sub)
                .filter(|sub| !is_test(self.path(*sub)))
                .collect::<Vec<_>>();
            let data = made
                .iter()
                .filter(|sub| layers::own(self.path(**sub)) || handled.contains(sub))
                .count();
            if data * 2 > made.len() {
                continue;
            }
            let mut users = self
                .holders(*abstraction)
                .into_iter()
                .filter(|(user, ..)| {
                    user != abstraction && !under.iter().any(|(sub, ..)| sub == user)
                })
                .collect::<Vec<_>>();
            for &(binder, target, guessed) in &self.di_binds {
                if binder != *abstraction
                    && !under.iter().any(|(sub, ..)| *sub == binder)
                    && (target == *abstraction || under.iter().any(|(sub, ..)| *sub == target))
                    && !users.iter().any(|(known, ..)| *known == binder)
                {
                    users.push((binder, "binds", guessed));
                }
            }
            if users.is_empty() {
                continue;
            }
            let mut finding = Finding::new(self, STRATEGY, *abstraction, ABSTRACTION);
            for (sub, implements, guessed) in under {
                finding.part(*sub, IMPLEMENTATION);
                finding.link(*sub, *abstraction, over(*implements), *guessed);
            }
            for (user, way, guessed) in &users {
                finding.part(*user, USER);
                finding.link(*user, *abstraction, way, *guessed);
            }
            finding.told(format!(
                "{} stand under {}, and {} it as that.",
                count(under.len(), IMPLEMENTATION),
                self.index.shown(*abstraction),
                match users.len() {
                    1 => "1 other type holds or takes".to_owned(),
                    many => format!("{many} other types hold or take"),
                },
            ));
            found.push(finding.done(false));
        }
    }
    fn decorators(&self, found: &mut Vec<Found>) {
        for (at, above) in &self.supers {
            let mut finding = Finding::new(self, DECORATORS, *at, DECORATOR);
            for (abstraction, implements, guessed) in above {
                if !self.above(*abstraction) {
                    continue;
                }
                let Some(kept) = self.wraps(*at, *abstraction) else {
                    continue;
                };
                finding.part(*abstraction, ABSTRACTION);
                finding.link(*at, *abstraction, over(*implements), *guessed);
                finding.link(*at, *abstraction, "holds", kept);
                for (other, implements, guessed) in &self.subs[abstraction] {
                    if other != at {
                        finding.part(*other, IMPLEMENTATION);
                        finding.link(*other, *abstraction, over(*implements), *guessed);
                    }
                }
                finding.told(format!(
                    "{} stands under {} and keeps one of it.",
                    self.index.shown(*at),
                    self.index.shown(*abstraction),
                ));
            }
            if finding.parts.len() > 1 {
                found.push(finding.done(false));
            }
        }
    }
    fn adapters(&self, found: &mut Vec<Found>) {
        for (&adapter, interfaces) in &self.supers {
            let mut finding = Finding::new(self, ADAPTERS, adapter, ("adapter", "adapters"));
            for &(interface, implements, inferred) in interfaces {
                if !implements && !self.above(interface) {
                    continue;
                }
                for &method in self.members.get(&adapter).into_iter().flatten() {
                    let name = bare(self.label(method));
                    if CONSTRUCTORS.contains(&name)
                        || name == plain(self.label(adapter))
                        || !self
                            .members
                            .get(&interface)
                            .into_iter()
                            .flatten()
                            .any(|&member| bare(self.label(member)) == name)
                    {
                        continue;
                    }
                    for &(target, guessed) in self.calls.get(&method).into_iter().flatten() {
                        let adaptee = self.owner[target as usize];
                        if adaptee == NONE
                            || adaptee == adapter
                            || adaptee == interface
                            || self
                                .supers
                                .get(&adaptee)
                                .is_some_and(|types| types.iter().any(|&(id, ..)| id == interface))
                        {
                            continue;
                        }
                        let Some(kept) = self.wraps(adapter, adaptee) else {
                            continue;
                        };
                        finding.part(interface, ABSTRACTION);
                        finding.part(adaptee, ("adapted type", "adapted types"));
                        finding.link(adapter, interface, over(implements), inferred);
                        finding.link(adapter, adaptee, "delegates to", guessed || kept);
                    }
                }
            }
            if finding.parts.len() > 1 {
                found.push(finding.done(false));
            }
        }
    }

    fn factories(&self, found: &mut Vec<Found>) {
        // By what holds the method: a type, or the function itself.
        let mut by: BTreeMap<u32, Finding> = BTreeMap::new();
        // In the order of the graph, so that a finding is told the same
        // way every time.
        let mut makers = self.makes.keys().copied().collect::<Vec<_>>();
        makers.sort_unstable();
        for method in &makers {
            let made = &self.makes[method];
            for (product, _) in self.named(*method, &[RESULT]) {
                if !self.above(product) {
                    continue;
                }
                let kinds = made
                    .iter()
                    .filter(|(made, _)| {
                        *made != product && self.subs[&product].iter().any(|(sub, ..)| sub == made)
                    })
                    .collect::<Vec<_>>();
                if kinds.is_empty() {
                    continue;
                }
                let holder = match self.owner[*method as usize] {
                    NONE => *method,
                    holder => holder,
                };
                if holder == product {
                    continue;
                }
                let finding = by
                    .entry(holder)
                    .or_insert_with(|| Finding::new(self, FACTORIES, holder, FACTORY));
                finding.part(product, PRODUCT);
                finding.link(holder, product, "returns", false);
                for (made, guessed) in &kinds {
                    finding.part(*made, MADE_TYPE);
                    finding.link(holder, *made, "makes", *guessed);
                    let implements = self.subs[&product]
                        .iter()
                        .find(|(sub, ..)| sub == made)
                        .is_some_and(|(_, implements, _)| *implements);
                    finding.link(*made, product, over(implements), false);
                }
                finding.told(format!(
                    "{} returns {} and makes {}.",
                    self.index.shown(*method),
                    self.index.shown(product),
                    listed(self.index, kinds.iter().map(|(made, _)| *made)),
                ));
            }
        }
        // And by the name, where a type named a factory makes other types.
        for at in self.types() {
            let methods = &self.members[&at];
            let at = &at;
            if by.contains_key(at) || !plain(self.label(*at)).ends_with("Factory") {
                continue;
            }
            let mut finding = Finding::new(self, FACTORIES, *at, FACTORY);
            for method in methods {
                for (made, guessed) in self.makes.get(method).into_iter().flatten() {
                    if made != at {
                        finding.part(*made, MADE_TYPE);
                        finding.link(*at, *made, "makes", *guessed);
                    }
                }
            }
            if finding.parts.len() > 1 {
                finding.told(format!(
                    "{} is named a factory and makes {}.",
                    self.index.shown(*at),
                    listed(
                        self.index,
                        finding.parts.iter().skip(1).map(|(part, _)| *part)
                    ),
                ));
                by.insert(*at, finding.named());
            }
        }
        found.extend(by.into_values().map(|finding| {
            let named = finding.named;
            finding.done(named)
        }));
    }
    fn templates(&self, found: &mut Vec<Found>) {
        for (base, under) in &self.subs {
            let Some(own) = self.members.get(base) else {
                continue;
            };
            // The steps: methods of the base that a type under it has too.
            let mut fillers: HashMap<u32, Vec<u32>> = HashMap::new();
            for (sub, ..) in under {
                for method in self.members.get(sub).into_iter().flatten() {
                    if let Some(step) = own
                        .iter()
                        .find(|step| self.label(**step) == self.label(*method))
                    {
                        fillers.entry(*step).or_default().push(*sub);
                    }
                }
            }
            let mut finding = Finding::new(self, TEMPLATES, *base, TEMPLATE);
            for method in own {
                let steps = self
                    .calls
                    .get(method)
                    .into_iter()
                    .flatten()
                    .filter(|(step, _)| step != method && fillers.contains_key(step))
                    .map(|(step, _)| *step)
                    .collect::<BTreeSet<_>>();
                if steps.is_empty() {
                    continue;
                }
                let mut filling = BTreeSet::new();
                for step in &steps {
                    filling.extend(fillers[step].iter().copied());
                }
                for sub in &filling {
                    finding.part(*sub, FILLER);
                    let (implements, guessed) = under
                        .iter()
                        .find(|(known, ..)| known == sub)
                        .map_or((false, false), |(_, implements, guessed)| {
                            (*implements, *guessed)
                        });
                    finding.link(*sub, *base, over(implements), guessed);
                }
                finding.told(format!(
                    "{} calls {}, which {} {}.",
                    self.index.shown(*method),
                    steps
                        .iter()
                        .map(|step| bare(self.label(*step)).to_owned() + "()")
                        .collect::<Vec<_>>()
                        .join(", "),
                    listed(self.index, filling.iter().copied()),
                    if filling.len() == 1 {
                        "defines"
                    } else {
                        "define"
                    },
                ));
            }
            if finding.parts.len() > 1 {
                found.push(finding.done(false));
            }
        }
    }
    fn builders(&self, found: &mut Vec<Found>) {
        for (at, methods) in &self.members {
            // A method named `build` or `finish` alone says too little: many
            // a type that builds nothing has one.
            if !plain(self.label(*at)).ends_with("Builder") {
                continue;
            }
            let builds = |method: &u32| {
                let name = bare(self.label(*method)).to_ascii_lowercase();
                name == "build" || name.starts_with("build_") || name == "finish"
            };
            let building = methods
                .iter()
                .filter(|method| builds(method))
                .collect::<Vec<_>>();
            // What the build methods give; where it has none, what any of
            // its methods gives.
            let giving = if building.is_empty() {
                methods.iter().collect::<Vec<_>>()
            } else {
                building
            };
            let mut finding = Finding::new(self, BUILDERS, *at, BUILDER);
            for method in giving {
                let given = self
                    .named(*method, &[RESULT])
                    .into_iter()
                    .chain(self.makes.get(method).into_iter().flatten().copied());
                for (built, guessed) in given {
                    if built != *at && !finding.has(built) {
                        finding.part(built, BUILT);
                        finding.link(*at, built, "builds", guessed);
                        finding.told(format!(
                            "{} gives {}.",
                            self.index.shown(*method),
                            self.index.shown(built),
                        ));
                    }
                }
            }
            if finding.parts.len() > 1 {
                found.push(finding.done(true));
            }
        }
    }
    fn singletons(&self, found: &mut Vec<Found>) {
        for (at, methods) in &self.members {
            let Some(accessor) = methods.iter().find(|method| {
                ACCESSORS.contains(&bare(self.label(**method)).to_ascii_lowercase().as_str())
            }) else {
                continue;
            };
            let mut finding = Finding::new(self, SINGLETONS, *at, SINGLE);
            for (caller, guessed) in self.called.get(accessor).into_iter().flatten() {
                let user = self.owner[*caller as usize];
                if user != NONE && user != *at && !finding.has(user) {
                    finding.part(user, USER);
                    finding.link(user, *at, "asks for", *guessed);
                }
            }
            finding.told(format!(
                "{} hands out the one {}.",
                self.index.shown(*accessor),
                self.index.shown(*at),
            ));
            found.push(finding.done(true));
        }
    }
    /// Events with their listeners, and messages with the handler named
    /// after each.
    /// Finds events with their listeners and commands with their handlers,
    /// and gives the messages that have a handler of their own.
    fn messages(&self, found: &mut Vec<Found>) -> BTreeSet<u32> {
        let mut events: BTreeMap<u32, Vec<(u32, bool)>> = BTreeMap::new();
        let mut param_events: BTreeSet<u32> = BTreeSet::new();
        let mut commands: BTreeMap<u32, (u32, bool)> = BTreeMap::new();
        for at in self.types() {
            let methods = &self.members[&at];
            let at = &at;
            let name = plain(self.label(*at));
            let listens = is_listener_symbol(self.label(*at), self.path(*at));
            for method in methods {
                if !handles(bare(self.label(*method))) {
                    continue;
                }
                for (taken, guessed) in self.named(*method, &[PARAMETER]) {
                    if taken == *at {
                        continue;
                    }
                    let message = plain(self.label(taken));
                    if name.strip_suffix("Handler") == Some(message) {
                        commands.entry(taken).or_insert((*at, guessed));
                    } else if listens
                        || message.ends_with("Event")
                        || foldered(self.path(taken), &["event", "events"])
                    {
                        param_events.insert(taken);
                        let listeners = events.entry(taken).or_default();
                        if !listeners.iter().any(|(known, _)| known == at) {
                            listeners.push((*at, guessed));
                        }
                    }
                }
            }
        }
        for (&event, sem_listeners) in &self.event_listeners {
            let listeners = events.entry(event).or_default();
            for &(listener, guessed) in sem_listeners {
                push_unique_edge(listeners, listener, guessed);
            }
        }
        for &event in self.event_publishers.keys() {
            events.entry(event).or_default();
        }
        for (event, listeners) in events {
            let mut finding = Finding::new(self, EVENTS, event, EVENT);
            for (listener, guessed) in &listeners {
                finding.part(*listener, LISTENER);
                finding.link(*listener, event, "handles", *guessed);
            }
            for (raiser, guessed) in self.makers(event) {
                if !finding.has(raiser) {
                    finding.part(raiser, RAISER);
                    finding.link(raiser, event, "raises", guessed);
                }
            }
            let sem_publishers = self.event_publishers.get(&event);
            if let Some(publishers) = sem_publishers {
                for &(publisher, guessed) in publishers {
                    if !finding.has(publisher) {
                        finding.part(publisher, RAISER);
                    }
                    finding.link(publisher, event, "publishes", guessed);
                }
            }
            if finding.parts.len() <= 1 {
                continue;
            }
            let has_sem = self.event_listeners.contains_key(&event) || sem_publishers.is_some();
            if !has_sem {
                finding.told(format!(
                    "{} {} {} in a handling method.",
                    listed(self.index, listeners.iter().map(|(listener, _)| *listener)),
                    if listeners.len() == 1 {
                        "takes"
                    } else {
                        "take"
                    },
                    self.index.shown(event),
                ));
            } else {
                if !listeners.is_empty() {
                    finding.told(format!(
                        "{} {} {}.",
                        listed(self.index, listeners.iter().map(|(listener, _)| *listener)),
                        if listeners.len() == 1 {
                            "handles"
                        } else {
                            "handle"
                        },
                        self.index.shown(event),
                    ));
                }
                if let Some(publishers) = sem_publishers
                    && !publishers.is_empty()
                {
                    finding.told(format!(
                        "{} {} {}.",
                        listed(self.index, publishers.iter().map(|(p, _)| *p)),
                        if publishers.len() == 1 {
                            "publishes"
                        } else {
                            "publish"
                        },
                        self.index.shown(event),
                    ));
                }
            }
            let named_only = param_events.contains(&event) && !has_sem;
            found.push(finding.done(named_only));
        }
        let handled = commands.keys().copied().collect();
        for (message, (handler, guessed)) in commands {
            let mut finding = Finding::new(self, COMMANDS, message, MESSAGE);
            finding.part(handler, HANDLER);
            finding.link(handler, message, "handles", guessed);
            for (sender, guessed) in self.makers(message) {
                if !finding.has(sender) {
                    finding.part(sender, SENDER);
                    finding.link(sender, message, "sends", guessed);
                }
            }
            finding.told(format!(
                "{} is named after {} and takes it.",
                self.index.shown(handler),
                self.index.shown(message),
            ));
            found.push(finding.done(true));
        }
        handled
    }
    fn semantic_patterns(&self, found: &mut Vec<Found>) {
        for (&handler, routes) in &self.routes {
            let mut finding = Finding::new(self, ROUTES, handler, ROUTE_HANDLER);
            for &(route, guessed) in routes {
                finding.part(route, ROUTE_ENDPOINT);
                finding.link(handler, route, "exposes", guessed);
            }
            if finding.parts.len() > 1 {
                finding.told(format!(
                    "{} exposes {}.",
                    self.index.shown(handler),
                    listed(self.index, routes.iter().map(|(route, _)| *route)),
                ));
                found.push(finding.done(false));
            }
        }
        let mut tables: BTreeSet<u32> = BTreeSet::new();
        tables.extend(self.table_readers.keys().copied());
        tables.extend(self.table_writers.keys().copied());
        for table in tables {
            let mut finding = Finding::new(self, DATA_ACCESS, table, TABLE);
            let writers = self.table_writers.get(&table);
            let readers = self.table_readers.get(&table);
            for &(writer, guessed) in writers.into_iter().flatten() {
                finding.part(writer, TABLE_WRITER);
                finding.link(writer, table, "writes", guessed);
            }
            for &(reader, guessed) in readers.into_iter().flatten() {
                finding.part(reader, TABLE_READER);
                finding.link(reader, table, "reads", guessed);
            }
            if finding.parts.len() > 1 {
                if let Some(writers) = writers
                    && !writers.is_empty()
                {
                    finding.told(format!(
                        "{} {} {}.",
                        listed(self.index, writers.iter().map(|(w, _)| *w)),
                        if writers.len() == 1 {
                            "writes"
                        } else {
                            "write"
                        },
                        self.index.shown(table),
                    ));
                }
                if let Some(readers) = readers
                    && !readers.is_empty()
                {
                    finding.told(format!(
                        "{} {} {}.",
                        listed(self.index, readers.iter().map(|(r, _)| *r)),
                        if readers.len() == 1 { "reads" } else { "read" },
                        self.index.shown(table),
                    ));
                }
                found.push(finding.done(false));
            }
        }
        let mut di_by_binder: BTreeMap<u32, Vec<(u32, bool)>> = BTreeMap::new();
        for &(binder, service, guessed) in &self.di_binds {
            if binder != service {
                push_unique_edge(di_by_binder.entry(binder).or_default(), service, guessed);
            }
        }
        for (binder, services) in di_by_binder {
            let mut finding = Finding::new(self, DI_BINDINGS, binder, DI_PROVIDER);
            for &(service, guessed) in &services {
                finding.part(service, DI_SERVICE);
                finding.link(binder, service, "binds", guessed);
            }
            if finding.parts.len() > 1 {
                finding.told(format!(
                    "{} binds {}.",
                    self.index.shown(binder),
                    listed(self.index, services.iter().map(|(s, _)| *s)),
                ));
                found.push(finding.done(false));
            }
        }
    }
    fn attach_semantic_summaries(&self, found: &mut [Found]) {
        let extend_unique = |dest: &mut Vec<String>, map: &HashMap<u32, Vec<String>>, sym: u32| {
            if let Some(items) = map.get(&sym) {
                for item in items {
                    if !dest.contains(item) {
                        dest.push(item.clone());
                    }
                }
            }
        };
        for item in found {
            let mut summary = SemanticSummary::default();
            for &(part, _) in &item.parts {
                extend_unique(&mut summary.routes, &self.sym_routes, part);
                extend_unique(&mut summary.di_bindings, &self.sym_di, part);
                extend_unique(&mut summary.events_published, &self.sym_events_pub, part);
                extend_unique(&mut summary.events_handled, &self.sym_events_handle, part);
                extend_unique(&mut summary.tables_read, &self.sym_tables_read, part);
                extend_unique(&mut summary.tables_written, &self.sym_tables_write, part);
            }
            summary.routes.sort();
            summary.di_bindings.sort();
            summary.events_published.sort();
            summary.events_handled.sort();
            summary.tables_read.sort();
            summary.tables_written.sort();
            if !summary.routes.is_empty() {
                summary.insights.push(format!(
                    "Exposes {} route{}",
                    summary.routes.len(),
                    if summary.routes.len() == 1 { "" } else { "s" }
                ));
            }
            if !summary.di_bindings.is_empty() {
                summary.insights.push(format!(
                    "Participates in {} DI binding{}",
                    summary.di_bindings.len(),
                    if summary.di_bindings.len() == 1 {
                        ""
                    } else {
                        "s"
                    }
                ));
            }
            if !summary.events_published.is_empty() || !summary.events_handled.is_empty() {
                summary.insights.push(format!(
                    "Events: {} published, {} handled",
                    summary.events_published.len(),
                    summary.events_handled.len()
                ));
            }
            if !summary.tables_read.is_empty() || !summary.tables_written.is_empty() {
                summary.insights.push(format!(
                    "Tables: {} read, {} written",
                    summary.tables_read.len(),
                    summary.tables_written.len()
                ));
            }
            item.semantic_summary = summary;
        }
    }
    fn repositories(&self, found: &mut Vec<Found>) {
        for (at, &typed) in self.typed.iter().enumerate() {
            let at = at as u32;
            if !typed || !plain(self.label(at)).ends_with("Repository") {
                continue;
            }
            let mut finding = Finding::new(self, REPOSITORIES, at, REPOSITORY);
            // The types its methods give and take, the most named first.
            let mut stored: BTreeMap<u32, (u64, bool)> = BTreeMap::new();
            for method in self.members.get(&at).into_iter().flatten() {
                for (other, guessed) in self.named(*method, &[RESULT, PARAMETER]) {
                    if other != at {
                        let known = stored.entry(other).or_insert((0, guessed));
                        known.0 += 1;
                        known.1 &= guessed;
                    }
                }
            }
            let mut stored = stored.into_iter().collect::<Vec<_>>();
            stored.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(&b.0)));
            for (other, (_, guessed)) in stored.into_iter().take(3) {
                finding.part(other, STORED);
                finding.link(at, other, "gives or takes", guessed);
            }
            for (user, way, guessed) in self.holders(at) {
                if !finding.has(user) {
                    finding.part(user, USER);
                    finding.link(user, at, way, guessed);
                }
            }
            finding.told(format!("{} is named a repository.", self.index.shown(at)));
            found.push(finding.done(true));
        }
    }
}
/// The word for standing under a type.
fn over(implements: bool) -> &'static str {
    if implements { "implements" } else { "extends" }
}
fn count(many: usize, role: Role) -> String {
    format!("{many} {}", if many == 1 { role.0 } else { role.1 })
}
/// A few names in a row, with how many more there are.
fn listed(index: &Index, places: impl Iterator<Item = u32>) -> String {
    let mut names = places.map(|at| index.shown(at)).collect::<Vec<_>>();
    names.sort();
    names.dedup();
    let more = names.len().saturating_sub(4);
    names.truncate(4);
    let mut text = names.join(", ");
    if more > 0 {
        text.push_str(&format!(" and {more} more"));
    }
    text
}
/// A finding while it is put together.
struct Finding<'a> {
    index: &'a Index,
    sort: usize,
    parts: Vec<(u32, Role)>,
    links: Vec<(u32, u32, &'static str, bool)>,
    told: Vec<String>,
    named: bool,
}
impl<'a> Finding<'a> {
    fn new(read: &Read<'a>, sort: usize, anchor: u32, role: Role) -> Self {
        Self {
            index: read.index,
            sort,
            parts: vec![(anchor, role)],
            links: Vec::new(),
            told: Vec::new(),
            named: false,
        }
    }
    fn has(&self, at: u32) -> bool {
        self.parts.iter().any(|(part, _)| *part == at)
    }
    /// A part takes its first role: what a type is to the anchor comes
    /// before what it is beside that.
    fn part(&mut self, at: u32, role: Role) {
        if !self.has(at) {
            self.parts.push((at, role));
        }
    }
    fn link(&mut self, from: u32, to: u32, how: &'static str, guessed: bool) {
        if from != to
            && !self
                .links
                .iter()
                .any(|link| (link.0, link.1, link.2) == (from, to, how))
        {
            self.links.push((from, to, how, guessed));
        }
    }
    fn told(&mut self, text: String) {
        if self.told.len() < 6 && !self.told.contains(&text) {
            self.told.push(text);
        }
    }
    fn named(mut self) -> Self {
        self.named = true;
        self
    }
    /// The parts in a settled order: the anchor, then role by role as the
    /// roles were met, each role's by where they lie.
    fn done(self, named: bool) -> Found {
        let index = self.index;
        let anchor = self.parts[0];
        let mut roles: Vec<Role> = Vec::new();
        for (_, role) in &self.parts[1..] {
            if !roles.contains(role) {
                roles.push(*role);
            }
        }
        let mut rest = self.parts[1..].to_vec();
        let place = |at: u32| {
            let symbol = &index.symbols[at as usize];
            (index.files[symbol.file as usize].as_str(), symbol.line, at)
        };
        rest.sort_by(|a, b| {
            let role = |part: &(u32, Role)| roles.iter().position(|role| *role == part.1);
            role(a).cmp(&role(b)).then(place(a.0).cmp(&place(b.0)))
        });
        let mut parts = vec![anchor];
        parts.extend(rest);
        let mut files = parts
            .iter()
            .map(|(at, _)| index.symbols[*at as usize].file)
            .collect::<Vec<_>>();
        files.sort_unstable();
        files.dedup();
        let mut links = self.links;
        links.sort_by_key(|link| (place(link.0), place(link.1), link.2));
        Found {
            sort: self.sort,
            anchor: anchor.0,
            parts,
            links,
            told: self.told,
            named: named || self.named,
            files,
            semantic_summary: SemanticSummary::default(),
        }
    }
}
impl Patterns {
    /// Finds the patterns of a graph. `certain` leaves out the links the
    /// tool only inferred, and `text` gives a source where it can be read.
    fn find(index: &Index, certain: bool, text: &dyn Fn(&str) -> Option<String>) -> Self {
        let read = Read::new(index, certain, text);
        let mut found = Vec::new();
        read.repositories(&mut found);
        let handled = read.messages(&mut found);
        read.strategies(&mut found, &handled);
        read.decorators(&mut found);
        read.adapters(&mut found);
        read.factories(&mut found);
        read.templates(&mut found);
        read.builders(&mut found);
        read.singletons(&mut found);
        read.semantic_patterns(&mut found);
        read.attach_semantic_summaries(&mut found);
        // The order of a list: pattern by pattern, the largest finding
        // first, and of two as large the one that lies first.
        found.sort_by(|a, b| {
            let place = |found: &Found| {
                let symbol = &index.symbols[found.anchor as usize];
                (
                    index.files[symbol.file as usize].as_str(),
                    symbol.line,
                    found.anchor,
                )
            };
            a.sort
                .cmp(&b.sort)
                .then(b.parts.len().cmp(&a.parts.len()))
                .then(place(a).cmp(&place(b)))
        });
        Self {
            found,
            layers: layers::Layers::read(index, certain),
        }
    }
}
impl Found {
    /// Whether any of it lies in a file, or under a folder.
    fn within(&self, index: &Index, scope: &str) -> bool {
        scope.is_empty()
            || self.files.iter().any(|file| {
                index.files[*file as usize]
                    .strip_prefix(scope)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
            })
    }
    fn guessed(&self) -> bool {
        self.links.iter().any(|link| link.3)
    }
    /// How many parts it has of each role but the anchor's, in words.
    fn counted(&self) -> String {
        let mut roles: Vec<(Role, usize)> = Vec::new();
        for (_, role) in &self.parts[1..] {
            match roles.iter_mut().find(|(known, _)| known == role) {
                Some((_, many)) => *many += 1,
                None => roles.push((*role, 1)),
            }
        }
        roles
            .iter()
            .map(|(role, many)| count(*many, *role))
            .collect::<Vec<_>>()
            .join(" · ")
    }
}
impl Index {
    pub(super) fn patterns_of(&self, certain: bool) -> &Patterns {
        self.patterns[usize::from(certain)]
            .get_or_init(|| Patterns::find(self, certain, &|path| text_at(&self.root, path)))
    }
    /// The name a finding is asked for by: its pattern and its anchor.
    fn pattern_id(&self, names: &Names, found: &Found) -> String {
        format!("{}@{}", SORTS[found.sort].id, names.key(found.anchor))
    }
    fn pattern_row(&self, names: &Names, found: &Found) -> Value {
        let anchor = &self.symbols[found.anchor as usize];
        let mut row = json!({
            "id": self.pattern_id(names, found),
            "sort": SORTS[found.sort].id,
            "name": SORTS[found.sort].name,
            "of": self.shown(found.anchor),
            "file": self.files[anchor.file as usize],
            "line": anchor.line,
            "named": found.named,
            "guessed": found.guessed(),
            "parts": found.counted(),
        });
        if let Some(summary) = found.semantic_summary.to_json() {
            row["semantic_summary"] = summary;
        }
        row
    }
    /// Marks a picture with the patterns found where it is open: on each box
    /// how many findings it takes part in or holds, beside the picture the
    /// findings pattern by pattern, `LISTED` of each, and with a selected
    /// place those it is part of. `certain` leaves out what rests on links
    /// the tool only inferred.
    pub fn mark_patterns(&self, view: &mut Value, certain: bool) {
        let patterns = self.patterns_of(certain);
        let scope = view["scope"].as_str().unwrap_or("").to_owned();
        let names = Names::new(self, &[], &|_| None);
        let touching = |place: &str| -> Vec<&Found> {
            match self.reach(place) {
                Some(Reach::Symbol(at)) => patterns
                    .found
                    .iter()
                    .filter(|found| found.parts.iter().any(|(part, _)| *part == at))
                    .collect(),
                Some(Reach::File(file)) => patterns
                    .found
                    .iter()
                    .filter(|found| found.files.binary_search(&file).is_ok())
                    .collect(),
                Some(Reach::Folder(folder)) => patterns
                    .found
                    .iter()
                    .filter(|found| found.within(self, folder))
                    .collect(),
                None => Vec::new(),
            }
        };
        if let Some(nodes) = view["nodes"].as_array_mut() {
            for node in nodes {
                let many = node["path"].as_str().map_or(0, |path| touching(path).len());
                if many > 0 {
                    node["patterns"] = json!(many);
                }
            }
        }
        if let Some(path) = view["focus"]["path"].as_str().map(str::to_owned) {
            let rows = touching(&path)
                .into_iter()
                .take(LISTED)
                .map(|found| self.pattern_row(&names, found))
                .collect::<Vec<_>>();
            view["focus"]["patterns"] = json!(rows);
        }
        let mut kinds = Vec::new();
        let mut total = 0;
        // The layers of the open folder are one finding, and come first.
        if let Some((row, tells)) = patterns.layers.row(self, &scope) {
            total += 1;
            kinds.push(json!({
                "id": "layers", "name": "Layers", "family": "architecture",
                "tells": tells, "found": 1, "more": 0, "instances": [row],
            }));
        }
        for (sort, told) in SORTS.iter().enumerate() {
            let here = patterns
                .found
                .iter()
                .filter(|found| found.sort == sort && found.within(self, &scope))
                .collect::<Vec<_>>();
            if here.is_empty() {
                continue;
            }
            total += here.len();
            kinds.push(json!({
                "id": told.id,
                "name": told.name,
                "family": if told.architecture { "architecture" } else { "design" },
                "tells": told.tells,
                "found": here.len(),
                "more": here.len().saturating_sub(LISTED),
                "instances": here
                    .iter()
                    .take(LISTED)
                    .map(|found| self.pattern_row(&names, found))
                    .collect::<Vec<_>>(),
            }));
        }
        view["patterns"] = json!({"found": total, "kinds": kinds});
    }
    /// Says in a picture of a folder's layers what a change did to their
    /// order, against `base`, the graph the change started from: see
    /// `Layers::compare`.
    pub fn compare_layers(
        &self,
        base: &Index,
        view: &mut Value,
        renamed: &HashMap<&str, &str>,
        certain: bool,
    ) {
        self.compare_layers_with_rules(base, view, renamed, certain, None);
    }

    /// Compares layers against `base`, supporting both built-in layers and `.harness-architecture.json` rules.
    pub(crate) fn compare_layers_with_rules(
        &self,
        base: &Index,
        view: &mut Value,
        renamed: &HashMap<&str, &str>,
        certain: bool,
        rules: Option<&layer_rules::Rules>,
    ) {
        if let Some(rules) = rules
            && view["pattern"]["declared"] == true
        {
            self.compare_declared_layers(base, view, renamed, certain, rules);
            return;
        }
        self.patterns_of(certain).layers.compare(
            self,
            &base.patterns_of(certain).layers,
            base,
            view,
            renamed,
        );
    }
    /// The picture of one finding: its parts as boxes, each with what it is
    /// in the pattern, and between them what makes the pattern. `None`
    /// where the graph has no such finding, as after it was built anew.
    pub fn pattern(
        &self,
        project: &str,
        id: &str,
        changed: &[(String, char)],
        edited: &dyn Fn(&str) -> Option<Edited>,
        focus: Option<&str>,
        certain: bool,
    ) -> Option<Value> {
        let (sort, place) = id.split_once('@')?;
        if sort == "layers" {
            // The layers of a folder are asked for by the folder.
            return self
                .patterns_of(certain)
                .layers
                .view(self, project, place, focus, certain);
        }
        let sort = SORTS.iter().position(|known| known.id == sort)?;
        let Some(Reach::Symbol(anchor)) = self.reach(place) else {
            return None;
        };
        let found = self
            .patterns_of(certain)
            .found
            .iter()
            .find(|found| found.sort == sort && found.anchor == anchor)?;
        let told = &SORTS[sort];
        // Of a finding too large for a picture each role keeps its share,
        // and what one role leaves another takes.
        let mut roles: Vec<(Role, usize, usize)> = Vec::new();
        for (_, role) in &found.parts[1..] {
            match roles.iter_mut().find(|(known, ..)| known == role) {
                Some((_, many, _)) => *many += 1,
                None => roles.push((*role, 1, 0)),
            }
        }
        let mut room = DRAWN - 1;
        let mut left = roles.len();
        let mut order = (0..roles.len()).collect::<Vec<_>>();
        order.sort_by_key(|at| roles[*at].1);
        for at in order {
            let share = (room / left.max(1)).max(1);
            roles[at].2 = roles[at].1.min(share).min(room);
            room -= roles[at].2;
            left -= 1;
        }
        let mut taken = vec![0usize; roles.len()];
        let mut drawn = vec![found.parts[0]];
        for part in &found.parts[1..] {
            let at = roles.iter().position(|(role, ..)| *role == part.1)?;
            if taken[at] < roles[at].2 {
                taken[at] += 1;
                drawn.push(*part);
            }
        }
        let names = Names::new(self, changed, edited);
        let net = Net::new(self, true, certain);
        let keys = drawn
            .iter()
            .map(|(at, _)| names.key(*at))
            .collect::<Vec<_>>();
        let paths = keys.iter().map(String::as_str).collect::<Vec<_>>();
        let slot = |at: u32| drawn.iter().position(|(part, _)| *part == at);
        // The parts of one role stand in one frame, the anchor in its own.
        let frame = |at: usize| match at {
            0 => 0,
            _ => {
                1 + roles
                    .iter()
                    .position(|(role, ..)| *role == drawn[at].1)
                    .unwrap_or(0)
            }
        };
        let node = |at: usize| {
            let mut node = net.node(&names, drawn[at].0, &keys[at], &paths);
            node["role"] = json!(drawn[at].1.0);
            node["start"] = json!(at == 0);
            node["holder"] = json!(format!("role:{}", frame(at)));
            node
        };
        let titled = |text: &str| {
            let mut letters = text.chars();
            match letters.next() {
                Some(first) => first.to_uppercase().chain(letters).collect::<String>(),
                None => String::new(),
            }
        };
        let mut frames = vec![json!({
            "path": "role:0", "name": titled(found.parts[0].1.0), "holder": null,
            "places": 1, "drawn": 1, "fixed": true,
        })];
        for (at, (role, many, drawn)) in roles.iter().enumerate() {
            frames.push(json!({
                "path": format!("role:{}", at + 1),
                "name": titled(if *many == 1 { role.0 } else { role.1 }),
                "holder": null, "places": many, "drawn": drawn, "fixed": true,
            }));
        }
        // Two parts are joined once, by all that joins them.
        let mut joined: Vec<(usize, usize, Vec<&str>, bool)> = Vec::new();
        for (from, to, how, guessed) in &found.links {
            let (Some(from), Some(to)) = (slot(*from), slot(*to)) else {
                continue;
            };
            match joined.iter_mut().find(|(a, b, ..)| (*a, *b) == (from, to)) {
                Some((_, _, ways, inferred)) => {
                    ways.push(how);
                    *inferred &= *guessed;
                }
                None => joined.push((from, to, vec![how], *guessed)),
            }
        }
        let symbol = &self.symbols[anchor as usize];
        let scope = self.files[symbol.file as usize].as_str();
        let mut crumbs = Self::crumbs(project, scope);
        crumbs.push(json!({"name": format!("{}: {}", told.name, self.shown(anchor)), "path": ""}));
        let mut view = json!({
            "mode": "pattern", "scope": scope, "crumbs": crumbs, "certain": certain,
            "totals": self.totals(),
            "pattern": {
                "id": id,
                "sort": told.id,
                "name": told.name,
                "family": if told.architecture { "architecture" } else { "design" },
                "tells": told.tells,
                "named": found.named,
                "guessed": found.guessed(),
                "of": self.end(&keys[0], Reach::Symbol(anchor)),
                "role": found.parts[0].1.0,
                "told": found.told,
                "parts": roles
                    .iter()
                    .map(|(role, many, drawn)| {
                        json!({"one": role.0, "many": role.1, "found": many, "drawn": drawn})
                    })
                    .collect::<Vec<_>>(),
            },
            "nodes": (0..drawn.len()).map(node).collect::<Vec<_>>(),
            "frames": frames,
            "edges": joined
                .iter()
                .map(|(from, to, ways, guessed)| {
                    json!({
                        "from": keys[*from], "to": keys[*to],
                        "links": 1, "guessed": u64::from(*guessed),
                        "tie": ways.join(" and "),
                    })
                })
                .collect::<Vec<_>>(),
            "focus": null,
        });
        if let Some(summary) = found.semantic_summary.to_json() {
            view["pattern"]["semantic_summary"] = summary;
        }
        if let Some(at) = focus.and_then(|selected| keys.iter().position(|key| key == selected)) {
            let row = |other: usize, ways: &[&str], guessed: bool| {
                let flow = Flow {
                    links: 1,
                    guessed: u64::from(guessed),
                };
                let mut row = names.row(drawn[other].0, true, flow, None);
                row["relation"] = json!(ways.join(" and "));
                row["role"] = json!(drawn[other].1.0);
                row
            };
            let mut focus = node(at);
            focus["inside"] = json!(true);
            focus["uses"] = joined
                .iter()
                .filter(|(from, ..)| *from == at)
                .map(|(_, to, ways, guessed)| row(*to, ways, *guessed))
                .collect();
            focus["used_by"] = joined
                .iter()
                .filter(|(_, to, ..)| *to == at)
                .map(|(from, _, ways, guessed)| row(*from, ways, *guessed))
                .collect();
            view["focus"] = focus;
        }
        Some(view)
    }
}
#[cfg(test)]
mod tests;

#[cfg(test)]
mod semantic_tests {
    use super::*;

    #[test]
    fn how_recognizes_semantic_framework_and_database_hints() {
        assert_eq!(how("route"), Some(ROUTE));
        assert_eq!(how("exposes_route"), Some(ROUTE));
        assert_eq!(how("di_binding"), Some(DI_BINDING));
        assert_eq!(how("di_binds"), Some(DI_BINDING));
        assert_eq!(how("event"), Some(EVENT_HINT));
        assert_eq!(how("publishes_event"), Some(EVENT_HINT));
        assert_eq!(how("handles_event"), Some(EVENT_HINT));
        assert_eq!(how("table_read"), Some(TABLE_READ));
        assert_eq!(how("reads_table"), Some(TABLE_READ));
        assert_eq!(how("table_write"), Some(TABLE_WRITE));
        assert_eq!(how("writes_table"), Some(TABLE_WRITE));
    }

    #[test]
    fn semantic_events_di_routes_and_tables_participate_in_patterns() {
        let mut sources: HashMap<String, String> = HashMap::new();
        sources.insert(
            "app/Contracts/Notifier.php".into(),
            "<?php\n\ninterface Notifier\n{\n}\n".into(),
        );
        sources.insert(
            "app/Adapters/LegacyMailer.php".into(),
            "<?php\n\nclass LegacyMailer\n{\n}\n".into(),
        );
        sources.insert(
            "app/Adapters/MailerAdapter.php".into(),
            "<?php\n\nclass MailerAdapter implements Notifier\n{\n}\n".into(),
        );
        sources.insert(
            "app/Adapters/NullNotifier.php".into(),
            "<?php\n\nclass NullNotifier implements Notifier\n{\n}\n".into(),
        );
        sources.insert(
            "app/Providers/AppServiceProvider.php".into(),
            "<?php\n\nclass AppServiceProvider\n{\n}\n".into(),
        );
        sources.insert(
            "app/Repositories/OrderRepository.php".into(),
            "<?php\n\nclass OrderRepository\n{\n}\n".into(),
        );
        sources.insert(
            "app/Http/OrderController.php".into(),
            "<?php\n\nclass OrderController\n{\n}\n".into(),
        );
        sources.insert(
            "app/Services/AuditListener.php".into(),
            "<?php\n\nclass AuditListener\n{\n}\n".into(),
        );

        let graph = json!({
            "nodes": [
                {"id": "Notifier", "label": "Notifier", "source_file": "app/Contracts/Notifier.php", "source_location": "L3", "_callable_class": true},
                {"id": "Notifier.send", "label": ".send()", "source_file": "app/Contracts/Notifier.php", "source_location": "L4"},
                {"id": "LegacyMailer", "label": "LegacyMailer", "source_file": "app/Adapters/LegacyMailer.php", "source_location": "L3", "_callable_class": true},
                {"id": "LegacyMailer.deliver", "label": ".deliver()", "source_file": "app/Adapters/LegacyMailer.php", "source_location": "L4"},
                {"id": "MailerAdapter", "label": "MailerAdapter", "source_file": "app/Adapters/MailerAdapter.php", "source_location": "L3", "_callable_class": true},
                {"id": "MailerAdapter.send", "label": ".send()", "source_file": "app/Adapters/MailerAdapter.php", "source_location": "L4"},
                {"id": "NullNotifier", "label": "NullNotifier", "source_file": "app/Adapters/NullNotifier.php", "source_location": "L3", "_callable_class": true},
                {"id": "NullNotifier.send", "label": ".send()", "source_file": "app/Adapters/NullNotifier.php", "source_location": "L4"},
                {"id": "AppServiceProvider", "label": "AppServiceProvider", "source_file": "app/Providers/AppServiceProvider.php", "source_location": "L3", "_callable_class": true},
                {"id": "AppServiceProvider.register", "label": ".register()", "source_file": "app/Providers/AppServiceProvider.php", "source_location": "L4"},
                {"id": "OrderRepository", "label": "OrderRepository", "source_file": "app/Repositories/OrderRepository.php", "source_location": "L3", "_callable_class": true},
                {"id": "OrderRepository.save", "label": ".save()", "source_file": "app/Repositories/OrderRepository.php", "source_location": "L4"},
                {"id": "OrderController", "label": "OrderController", "source_file": "app/Http/OrderController.php", "source_location": "L3", "_callable_class": true},
                {"id": "OrderController.store", "label": ".store()", "source_file": "app/Http/OrderController.php", "source_location": "L4"},
                {"id": "AuditListener", "label": "AuditListener", "source_file": "app/Services/AuditListener.php", "source_location": "L3", "_callable_class": true},
                {"id": "AuditListener.record", "label": ".record()", "source_file": "app/Services/AuditListener.php", "source_location": "L4"},
                {"id": "route_orders", "label": "route:POST /orders", "source_file": "routes/api.php", "source_location": "L2"},
                {"id": "evt_created", "label": "event:order.created", "source_file": "app/Http/OrderController.php", "source_location": "L5"},
                {"id": "tbl_orders", "label": "table:orders", "source_file": "database/migrations/orders.php", "source_location": "L1"},
            ],
            "links": [
                {"source": "Notifier", "target": "Notifier.send", "relation": "method"},
                {"source": "LegacyMailer", "target": "LegacyMailer.deliver", "relation": "method"},
                {"source": "MailerAdapter", "target": "MailerAdapter.send", "relation": "method"},
                {"source": "NullNotifier", "target": "NullNotifier.send", "relation": "method"},
                {"source": "AppServiceProvider", "target": "AppServiceProvider.register", "relation": "method"},
                {"source": "OrderRepository", "target": "OrderRepository.save", "relation": "method"},
                {"source": "OrderController", "target": "OrderController.store", "relation": "method"},
                {"source": "AuditListener", "target": "AuditListener.record", "relation": "method"},
                // Strategy + Adapter via DI bindings (no static field tie needed!)
                {"source": "MailerAdapter", "target": "Notifier", "relation": "implements", "confidence": "EXTRACTED"},
                {"source": "NullNotifier", "target": "Notifier", "relation": "implements", "confidence": "EXTRACTED"},
                {"source": "MailerAdapter.send", "target": "LegacyMailer.deliver", "relation": "calls", "confidence": "EXTRACTED"},
                {"source": "MailerAdapter", "target": "LegacyMailer", "relation": "di_binds", "confidence": "EXTRACTED"},
                {"source": "AppServiceProvider.register", "target": "Notifier", "relation": "di_binds", "confidence": "EXTRACTED"},
                // Semantic routes, events (no *Event/*Listener naming), and tables
                {"source": "OrderController.store", "target": "route_orders", "relation": "exposes_route", "confidence": "EXTRACTED"},
                {"source": "OrderController.store", "target": "evt_created", "relation": "publishes_event", "confidence": "EXTRACTED"},
                {"source": "AuditListener.record", "target": "evt_created", "relation": "handles_event", "confidence": "EXTRACTED"},
                {"source": "OrderRepository.save", "target": "tbl_orders", "relation": "writes_table", "confidence": "EXTRACTED"},
                {"source": "OrderController.store", "target": "tbl_orders", "relation": "reads_table", "confidence": "INFERRED"},
            ]
        });

        let index = Index::read_with(
            graph.to_string().as_bytes(),
            Path::new("/work/shop"),
            &|path| sources.get(path).cloned(),
        )
        .unwrap();

        let all = Patterns::find(&index, false, &|path| sources.get(path).cloned());
        let sorts: Vec<&str> = all.found.iter().map(|f| SORTS[f.sort].id).collect();
        assert!(sorts.contains(&"repository"), "{sorts:?}");
        assert!(sorts.contains(&"events"), "{sorts:?}");
        assert!(sorts.contains(&"strategy"), "{sorts:?}");
        assert!(sorts.contains(&"adapter"), "{sorts:?}");
        assert!(sorts.contains(&"routes"), "{sorts:?}");
        assert!(sorts.contains(&"data_access"), "{sorts:?}");
        assert!(sorts.contains(&"di_bindings"), "{sorts:?}");

        // Events detector consumed publishes_event + handles_event even with arbitrary method/event names.
        let ev = all
            .found
            .iter()
            .find(|f| SORTS[f.sort].id == "events")
            .unwrap();
        assert!(!ev.named);
        assert_eq!(ev.semantic_summary.events_published, vec!["order.created"]);
        assert_eq!(ev.semantic_summary.events_handled, vec!["order.created"]);

        // Strategy detector got AppServiceProvider via di_binds.
        let strat = all
            .found
            .iter()
            .find(|f| SORTS[f.sort].id == "strategy")
            .unwrap();
        assert!(
            strat
                .parts
                .iter()
                .any(|(at, role)| index.shown(*at) == "AppServiceProvider" && *role == USER),
            "{:?}",
            strat.parts
        );
        assert!(!strat.semantic_summary.di_bindings.is_empty());

        // Adapter detector worked using MailerAdapter -> LegacyMailer di_binds without field tie.
        let adapter = all
            .found
            .iter()
            .find(|f| SORTS[f.sort].id == "adapter")
            .unwrap();
        assert_eq!(index.shown(adapter.anchor), "MailerAdapter");

        // Repository carries semantic_summary with table write info.
        let repo = all
            .found
            .iter()
            .find(|f| SORTS[f.sort].id == "repository")
            .unwrap();
        assert_eq!(repo.semantic_summary.tables_written, vec!["orders"]);

        // mark_patterns exposes semantic_summary on rows and omits INFERRED reads_table when certain=true.
        let mut view = index.view("demo", "", None, false, "");
        index.mark_patterns(&mut view, false);
        let kinds = view["patterns"]["kinds"].as_array().unwrap();
        let data_kind = kinds.iter().find(|k| k["id"] == "data_access").unwrap();
        assert_eq!(
            data_kind["instances"][0]["semantic_summary"]["tables_read"],
            json!(["orders"])
        );
        assert_eq!(
            data_kind["instances"][0]["semantic_summary"]["tables_written"],
            json!(["orders"])
        );

        let mut certain_view = index.view("demo", "", None, true, "");
        index.mark_patterns(&mut certain_view, true);
        let certain_kinds = certain_view["patterns"]["kinds"].as_array().unwrap();
        let certain_data = certain_kinds
            .iter()
            .find(|k| k["id"] == "data_access")
            .unwrap();
        assert_eq!(
            certain_data["instances"][0]["semantic_summary"]["tables_read"],
            json!([])
        );
        assert_eq!(
            certain_data["instances"][0]["semantic_summary"]["tables_written"],
            json!(["orders"])
        );
    }

    #[test]
    fn native_ir_abstract_skips_disk_reads_and_includes_calls_route_tag() {
        let missing_disk_root = Path::new("/nonexistent/ir-only-root");
        let ir_nodes = vec![
            crate::graph_ir::IrNodeSpec {
                id: "iface".into(),
                label: "GatewayInterface".into(),
                source_file: "src/Contracts/GatewayInterface.php".into(),
                source_location: Some("L2".into()),
                end_line: 0,
                file_type: Some("php".into()),
                node_kind: Some("class".into()),
                community: None,
                community_name: None,
                callable_class: true,
                is_abstract: true,
                is_test: false,
            },
            crate::graph_ir::IrNodeSpec {
                id: "impl_a".into(),
                label: "StripeGateway".into(),
                source_file: "src/StripeGateway.php".into(),
                source_location: Some("L2".into()),
                end_line: 0,
                file_type: Some("php".into()),
                node_kind: Some("class".into()),
                community: None,
                community_name: None,
                callable_class: true,
                is_abstract: false,
                is_test: false,
            },
            crate::graph_ir::IrNodeSpec {
                id: "impl_b".into(),
                label: "PayPalGateway".into(),
                source_file: "src/PayPalGateway.php".into(),
                source_location: Some("L2".into()),
                end_line: 0,
                file_type: Some("php".into()),
                node_kind: Some("class".into()),
                community: None,
                community_name: None,
                callable_class: true,
                is_abstract: false,
                is_test: false,
            },
            crate::graph_ir::IrNodeSpec {
                id: "ctrl".into(),
                label: "CheckoutController".into(),
                source_file: "src/CheckoutController.php".into(),
                source_location: Some("L2".into()),
                end_line: 0,
                file_type: Some("php".into()),
                node_kind: Some("class".into()),
                community: None,
                community_name: None,
                callable_class: true,
                is_abstract: false,
                is_test: false,
            },
            crate::graph_ir::IrNodeSpec {
                id: "sem:route:POST /checkout".into(),
                label: "route:POST /checkout".into(),
                source_file: "src/CheckoutController.php".into(),
                source_location: Some("L1".into()),
                end_line: 0,
                file_type: None,
                node_kind: Some("semantic".into()),
                community: None,
                community_name: None,
                callable_class: false,
                is_abstract: false,
                is_test: false,
            },
        ];
        let ir_links = vec![
            crate::graph_ir::IrLinkSpec {
                source: "impl_a".into(),
                target: "iface".into(),
                relation: Some("implements".into()),
                confidence: Some("EXTRACTED".into()),
                source_file: Some("src/StripeGateway.php".into()),
                context: None,
                evidence: None,
            },
            crate::graph_ir::IrLinkSpec {
                source: "impl_b".into(),
                target: "iface".into(),
                relation: Some("implements".into()),
                confidence: Some("EXTRACTED".into()),
                source_file: Some("src/PayPalGateway.php".into()),
                context: None,
                evidence: None,
            },
            crate::graph_ir::IrLinkSpec {
                source: "ctrl".into(),
                target: "sem:route:POST /checkout".into(),
                relation: Some("calls_route".into()),
                confidence: Some("EXTRACTED".into()),
                source_file: Some("src/CheckoutController.php".into()),
                context: None,
                evidence: None,
            },
        ];
        let idx = Index::from_ir_parts(
            missing_disk_root,
            None,
            Some(crate::graph_ir::NATIVE_IR_TOOL.to_owned()),
            ir_nodes,
            ir_links,
        )
        .expect("from_ir_parts");
        let read = Read::new(&idx, false, &|_| None);
        let iface_sym = idx
            .symbols
            .iter()
            .position(|s| s.is_abstract)
            .expect("abstract sym") as u32;
        assert_eq!(read.declared(iface_sym), Declared::Abstract);
        let concrete_sym = idx
            .symbols
            .iter()
            .position(|s| !s.is_abstract && !s.is_semantic && s.kind == Kind::Type)
            .expect("concrete sym") as u32;
        assert_eq!(read.declared(concrete_sym), Declared::Plain);
        assert!(
            !read.sym_routes.is_empty(),
            "calls_route should populate sym_routes on NATIVE_IR_TOOL index"
        );
    }
}
