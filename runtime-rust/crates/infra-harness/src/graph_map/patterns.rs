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
];
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
}
/// The patterns of a graph.
#[derive(Debug, Default)]
pub(super) struct Patterns {
    found: Vec<Found>,
    /// The layers its files are filed in, which a folder is asked for.
    layers: layers::Layers,
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
}
/// What the source says a type is declared as.
#[derive(Clone, Copy, PartialEq, Eq)]
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
            }
        }
        for (from, to, how, guessed) in &index.ties {
            if (certain && *guessed) || !read.typed[*to as usize] {
                continue;
            }
            let (from, to) = (*from, *to);
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
    /// Whether a type keeps the abstraction it stands under: in a field, or
    /// taken where it is built.
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
    }
    fn strategies(&self, found: &mut Vec<Found>) {
        for (abstraction, under) in &self.subs {
            if under.len() < 2 || !self.above(*abstraction) {
                continue;
            }
            let users = self
                .holders(*abstraction)
                .into_iter()
                .filter(|(user, ..)| {
                    user != abstraction && !under.iter().any(|(sub, ..)| sub == user)
                })
                .collect::<Vec<_>>();
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
    fn messages(&self, found: &mut Vec<Found>) {
        let mut events: BTreeMap<u32, Vec<(u32, bool)>> = BTreeMap::new();
        let mut commands: BTreeMap<u32, (u32, bool)> = BTreeMap::new();
        for at in self.types() {
            let methods = &self.members[&at];
            let at = &at;
            let name = plain(self.label(*at));
            let listens = ["Listener", "Subscriber", "Observer"]
                .iter()
                .any(|end| name.ends_with(end))
                || foldered(
                    self.path(*at),
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
                );
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
                        let listeners = events.entry(taken).or_default();
                        if !listeners.iter().any(|(known, _)| known == at) {
                            listeners.push((*at, guessed));
                        }
                    }
                }
            }
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
            found.push(finding.done(true));
        }
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
        read.messages(&mut found);
        read.strategies(&mut found);
        read.decorators(&mut found);
        read.factories(&mut found);
        read.templates(&mut found);
        read.builders(&mut found);
        read.singletons(&mut found);
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
    fn patterns_of(&self, certain: bool) -> &Patterns {
        self.patterns[usize::from(certain)]
            .get_or_init(|| Patterns::find(self, certain, &|path| text_at(&self.root, path)))
    }
    /// The name a finding is asked for by: its pattern and its anchor.
    fn pattern_id(&self, names: &Names, found: &Found) -> String {
        format!("{}@{}", SORTS[found.sort].id, names.key(found.anchor))
    }
    fn pattern_row(&self, names: &Names, found: &Found) -> Value {
        let anchor = &self.symbols[found.anchor as usize];
        json!({
            "id": self.pattern_id(names, found),
            "sort": SORTS[found.sort].id,
            "name": SORTS[found.sort].name,
            "of": self.shown(found.anchor),
            "file": self.files[anchor.file as usize],
            "line": anchor.line,
            "named": found.named,
            "guessed": found.guessed(),
            "parts": found.counted(),
        })
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
        if let Some(row) = patterns.layers.row(self, &scope) {
            total += 1;
            kinds.push(json!({
                "id": "layers", "name": "Layers", "family": "architecture",
                "tells": layers::TELLS, "found": 1, "more": 0, "instances": [row],
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
