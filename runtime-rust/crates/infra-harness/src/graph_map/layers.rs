//! The layers the code of a folder is filed in, and what runs against their
//! order.
//!
//! A layered design is told by names: a file is an entry point, a service,
//! data access or a part of the model by how it is named or by the folder it
//! lies in. Nothing in a graph says so, which is why the layers are a
//! finding by name and say what placed each file. What a graph does say is
//! which file needs which, and the order of the layers is checked against
//! that: a layer may use the layers below it, and a link from a layer to
//! one above it runs against the order.
//!
//! The links counted here are fewer than the map's own. An import, or a class
//! named in an attribute, says that one file knows of another; a call, an
//! inheritance, or the type of a field, a parameter or a result says that it
//! needs it. Only the second kind is counted, or every entity that names its
//! repository in an attribute would run against the order.
//!
//! There are two ways of filing. Most projects are read by the four layers
//! above. A design that keeps its domain at the heart is laid out in rings
//! instead, each a folder: what faces the outside, then the application,
//! then the domain. There a repository's interface lies in the domain and
//! what implements it outside, so the four layers would call its every
//! adapter a link against the order. A folder that keeps most of its code
//! in such rings is read by them.
use super::layer_rules::Rules;
use super::*;

const DECLARED: &str = "Layers and allowed dependencies from .harness-architecture.json. Counted: calls, inheritance and the types of fields, parameters and results.";

#[derive(Clone, Copy)]
struct DisplayLayer<'a> {
    id: &'a str,
    name: &'a str,
}

/// One layer, with what places a file in it: an ending of its name, or the
/// name of a folder it lies in. All are in small letters.
pub(super) struct Layer {
    pub(super) id: &'static str,
    pub(super) name: &'static str,
    ends: &'static [&'static str],
    folders: &'static [&'static str],
}
/// The layers from the outermost to the innermost: each may use those after
/// it.
pub(super) const LAYERS: &[Layer] = &[
    Layer {
        id: "entry",
        name: "Entry points",
        ends: &["controller", "middleware", "endpoint"],
        folders: &[
            "controller",
            "controllers",
            "console",
            "middleware",
            "resolver",
            "resolvers",
            "views",
            "routes",
        ],
    },
    Layer {
        id: "services",
        name: "Services",
        ends: &[
            "service",
            "manager",
            "usecase",
            "interactor",
            "handler",
            "listener",
            "subscriber",
            "job",
        ],
        folders: &[
            "services",
            "service",
            "usecases",
            "usecase",
            "handlers",
            "handler",
            "listeners",
            "listener",
            "subscribers",
            "eventsubscriber",
            "eventlistener",
            "jobs",
            "managers",
            "manager",
            "application",
        ],
    },
    Layer {
        id: "data",
        name: "Data access",
        ends: &["repository", "gateway", "mapper", "dao", "client"],
        folders: &[
            "repositories",
            "repository",
            "gateways",
            "mappers",
            "persistence",
            "infrastructure",
            "dao",
            "clients",
        ],
    },
    Layer {
        id: "model",
        name: "Model",
        ends: &["model", "entity", "dto", "valueobject"],
        folders: &[
            "models",
            "model",
            "entities",
            "entity",
            "domain",
            "dto",
            "dtos",
            "valueobject",
            "valueobjects",
            "events",
            "event",
        ],
    },
];
/// The rings of a design that keeps its domain at the heart, from the
/// outermost: each may use those after it. A file is filed by the outermost
/// folder on its way that is named for a ring, whatever it is called itself
/// and whatever a folder inside is called: `Application/Command/Domain/` is
/// of the application, where `Domain` is what the commands are about.
pub(super) const RINGS: &[Layer] = &[
    Layer {
        id: "adapters",
        name: "Adapters",
        ends: &[],
        folders: &[
            "infrastructure",
            "infra",
            "presentation",
            "ui",
            "adapters",
            "adapter",
        ],
    },
    Layer {
        id: "application",
        name: "Application",
        ends: &[],
        folders: &["application"],
    },
    Layer {
        id: "domain",
        name: "Domain",
        ends: &[],
        folders: &["domain"],
    },
];
/// The two ways of filing the files of a folder.
const LAYERED: usize = 0;
const RINGED: usize = 1;
const WAYS: [&[Layer]; 2] = [LAYERS, RINGS];
/// A folder is read by its rings where they hold no less than one in this
/// many of the files of its code, and by the four layers where those do.
const SHARES: [u64; 2] = [10, 2];
/// Folders of wide meaning: among the four layers one files what lies in it
/// only where no folder of a narrower meaning does, and the outermost of
/// them speaks, for one inside it is as often named for what the code is
/// about.
const WIDE: &[&str] = &["application", "domain", "infrastructure"];
/// Names of what passes between the layers and belongs to none of the four:
/// an exception thrown, a request or a response handed on. Without this the
/// folder they lie in would file them, and a client would run against the
/// order by taking the request it sends. Among the model's own classes they
/// are things of the model, as any other noun.
const PASSED: &[&str] = &["exception", "request", "response"];
/// Endings that name a thing as often as a layer: `FindPhoneModel` among the
/// operations of an API is an operation. They file a file only where no
/// folder of a narrower meaning says another layer.
const THINGS: &[&str] = &["model", "entity"];
/// Names that file what lies in a folder so named, and not a file: a file
/// named `Application` or `Console` starts a program more often than it is
/// a layer of one.
const FOLDERS_ONLY: &[&str] = &[
    "application",
    "console",
    "domain",
    "infrastructure",
    "persistence",
];
/// Where the layers named in the code below stand in the order.
const ENTRY: usize = 0;
const MODEL: usize = 3;
/// Folders that hold the operations of a GraphQL API where they lie in a
/// folder named `graphql`: a request comes in at an operation as it does at
/// a resolver. Anywhere else a folder of queries holds messages as often.
const GRAPHQL: &[&str] = &[
    "mutation",
    "mutations",
    "operation",
    "operations",
    "query",
    "queries",
    "subscription",
    "subscriptions",
];
/// Folders of the model's own classes: what lies in one is a thing of the
/// model, whatever it is called.
const MODEL_OWN: &[&str] = &[
    "entity",
    "entities",
    "model",
    "models",
    "dto",
    "dtos",
    "valueobject",
    "valueobjects",
];
/// Endings that are plain nouns as well. In a folder of the model's own
/// classes `OAuthClient`, `ImportJob` and `Subscriber` are things of the
/// model, named for what they are and not for what they do; a repository or
/// a listener there is still what its name says.
const NOUNS: &[&str] = &[
    "client",
    "endpoint",
    "gateway",
    "job",
    "manager",
    "service",
    "subscriber",
];
/// Pairs of files against the order, and files of one layer, that are
/// listed at once: the most linked first.
pub const LISTED: usize = 30;
/// Links against the rules that a picture draws although its own limits
/// left them out: the most links first.
pub const KEPT_VIOLATIONS: usize = 24;
/// A folder is said to be in layers where they hold no fewer files than
/// this, and no less of its code than `SHARES` says: a few files that happen
/// to be named like a layer are no layered design.
const FEWEST: u64 = 4;
/// No layer.
const NO: u8 = u8::MAX;
pub(super) const TELLS: [&str; 2] = [
    "Files filed as entry points, services, data access and model by their names and folders. A layer may use those after it; a link back runs against their order. Counted: calls, inheritance and the types of fields, parameters and results.",
    "Files filed by the ring they lie in: adapters outside, then the application, then the domain at the heart. A ring may use those inside it; a link out of an inner ring runs against their order. Counted: calls, inheritance and the types of fields, parameters and results.",
];
/// Computes minimal cut / decoupling remediation hints for forbidden layer edges.
fn layer_remediation_hints<'a>(
    items: impl Iterator<Item = (bool, bool, &'a str, &'a str, &'a str, &'a str, Flow)>,
) -> Vec<Value> {
    let mut rows: Vec<_> = items.collect();
    rows.sort_by(|a, b| {
        a.6.links
            .cmp(&b.6.links)
            .then(a.6.guessed.cmp(&b.6.guessed))
            .then(a.2.cmp(b.2))
            .then(a.3.cmp(b.3))
    });
    rows.into_iter()
        .take(6)
        .map(
            |(is_declared, is_skip, from_path, to_path, from_layer, to_layer, flow)| {
                let rule_kind = if is_skip {
                    "skip_layer"
                } else if is_declared {
                    "declared"
                } else {
                    "backward"
                };
                let hint = if is_skip {
                    format!(
                        "Bypassed layer in {from_layer} -> {to_layer}: route {} through an allowed intermediate layer or port/interface instead of direct access from {} ({} link{}).",
                        name(to_path),
                        name(from_path),
                        flow.links,
                        if flow.links == 1 { "" } else { "s" }
                    )
                } else {
                    format!(
                        "Invert {from_layer} -> {to_layer}: move {} behind a {from_layer} port/interface consumed by {} ({} link{}).",
                        name(to_path),
                        name(from_path),
                        flow.links,
                        if flow.links == 1 { "" } else { "s" }
                    )
                };
                json!({
                    "from": from_path,
                    "to": to_path,
                    "from_layer": from_layer,
                    "to_layer": to_layer,
                    "rule_kind": rule_kind,
                    "links": flow.links,
                    "weight": flow.links,
                    "guessed": flow.guessed,
                    "hint": hint,
                })
            },
        )
        .collect()
}
/// Whether a file is code a layer can hold: no test, and no template that
/// only wears the ending of a language, as `home.blade.php` does.
/// The layer the project's rules file a file in. As with the layers read
/// from names, a test, a template and what is no code are filed in none: a
/// test reaches into every layer it tests, and its links are no dependency
/// of the layer it lies in.
fn declared(rules: &Rules, path: &str) -> Option<usize> {
    code(path).then(|| rules.layer(path)).flatten()
}
fn code(path: &str) -> bool {
    crate::graph::file_types::is_layer_code_path(path)
}
/// The layer a file is filed in: by the ending of its name, by its name as a
/// whole, or by the nearest folder named for a layer. A name that is a plain
/// noun says nothing in a folder of the model's own classes: the folder
/// files it. A test is filed in none, nor is a file that is no code.
pub(super) fn layer(path: &str) -> Option<usize> {
    filed(path).map(|(layer, _)| layer)
}
/// Whether a file is one of the model's own classes: named for one, as
/// `OrderEntity` is, or lying in a folder of them. What a folder of wider
/// meaning files in the model, as `domain` does, may be anything.
pub(super) fn own(path: &str) -> bool {
    filed(path).is_some_and(|(_, own)| own)
}
/// The layer of a file, and whether it is one of the model's own classes:
/// only what files a file in the model can say so.
fn filed(path: &str) -> Option<(usize, bool)> {
    if !code(path) {
        return None;
    }
    let (folders, file) = path.rsplit_once('/').unwrap_or(("", path));
    let (stem, _) = file.rsplit_once('.')?;
    // `OrderRepositoryInterface` is named for what it stands for.
    let stem = ["Interface", "Impl"]
        .iter()
        .find_map(|end| stem.strip_suffix(end).filter(|rest| !rest.is_empty()))
        .unwrap_or(stem);
    let low = stem.to_lowercase();
    let parts = folders.split('/').collect::<Vec<_>>();
    // An ending is a word of the name: `OrderService`, `order_service`,
    // `orders.service`, and not `Remodel` for `model`. After a dot `client`
    // says where a file runs, as in `analytics.client.ts`, not what it is.
    let worded = |end: &str| {
        let Some(at) = stem.len().checked_sub(end.len()) else {
            return false;
        };
        stem.get(at..).is_some_and(|tail| {
            tail.eq_ignore_ascii_case(end)
                && (at == 0
                    || stem[..at].ends_with(['_', '-'])
                    || (stem[..at].ends_with('.') && end != "client")
                    || tail.starts_with(|first: char| first.is_uppercase()))
        })
    };
    // The nearest folder of a narrow meaning that is named for a layer, and
    // whether it is one of the model's own classes. Inside `graphql` a
    // folder of operations is named for the entry points.
    let narrow = parts.iter().enumerate().rev().find_map(|(at, folder)| {
        let folder = folder.to_lowercase();
        if WIDE.contains(&folder.as_str()) {
            return None;
        }
        if let Some(layer) = LAYERS
            .iter()
            .position(|layer| layer.folders.contains(&folder.as_str()))
        {
            return Some((layer, MODEL_OWN.contains(&folder.as_str())));
        }
        (GRAPHQL.contains(&folder.as_str())
            && parts[..at]
                .iter()
                .any(|above| above.eq_ignore_ascii_case("graphql")))
        .then_some((ENTRY, false))
    });
    // What passes between the layers is of none, wherever it lies; but
    // among the model's own classes `AccessRequest` is a thing of the model.
    if !narrow.is_some_and(|(_, own)| own)
        && (PASSED.iter().any(|end| worded(end))
            || parts.last().is_some_and(|folder| {
                ["exception", "exceptions"].contains(&folder.to_lowercase().as_str())
            }))
    {
        return None;
    }
    let noun = |word: &str| narrow.is_some_and(|(_, own)| own) && NOUNS.contains(&word);
    // A name that is as often a thing gives way to a folder that says
    // another layer.
    let thing = |layer: usize, end: &str| {
        THINGS.contains(&end) && narrow.is_some_and(|(folder, _)| folder != layer)
    };
    if let Some(found) = LAYERS.iter().enumerate().position(|(at, layer)| {
        layer
            .ends
            .iter()
            .any(|end| !noun(end) && !thing(at, end) && worded(end))
    }) {
        return Some((found, found == MODEL));
    }
    let whole = !FOLDERS_ONLY.contains(&low.as_str()) && !noun(&low);
    if whole
        && let Some(found) = LAYERS
            .iter()
            .position(|layer| layer.folders.contains(&low.as_str()))
    {
        return Some((found, MODEL_OWN.contains(&low.as_str())));
    }
    // Then the outermost folder of a wide meaning.
    narrow.or_else(|| {
        parts.iter().find_map(|folder| {
            let folder = folder.to_lowercase();
            WIDE.contains(&folder.as_str())
                .then(|| {
                    LAYERS
                        .iter()
                        .position(|layer| layer.folders.contains(&folder.as_str()))
                })
                .flatten()
                .map(|layer| (layer, false))
        })
    })
}
/// The ring a file lies in: that of the outermost folder on its way that is
/// named for one. A test lies in none, nor does what is no code.
pub(super) fn ring(path: &str) -> Option<usize> {
    if !code(path) {
        return None;
    }
    let (folders, _) = path.rsplit_once('/')?;
    folders.split('/').find_map(|folder| {
        let folder = folder.to_lowercase();
        RINGS
            .iter()
            .position(|ring| ring.folders.contains(&folder.as_str()))
    })
}
/// What places a file in a layer, in words.
fn signs(way: usize, layer: usize) -> String {
    if way == RINGED {
        return format!(
            "the outermost folder named {}",
            RINGS[layer].folders.join(", ")
        );
    }
    let mut signs = format!(
        "a name that ends in {}, or a folder named {}",
        LAYERS[layer].ends.join(", "),
        LAYERS[layer].folders.join(", ")
    );
    if layer == ENTRY {
        signs.push_str(&format!(
            ", or inside graphql a folder named {}",
            GRAPHQL.join(", ")
        ));
    }
    signs
}
/// One way of filing the files of a graph: the layer of each file, and every
/// pair of files of two layers of which the first needs the second, with
/// through how many links.
#[derive(Default)]
struct Way {
    of: Vec<u8>,
    cross: Vec<(u32, u32, EdgeFlow)>,
}
/// The layers of a graph, by the four layers and by the rings.
#[derive(Default)]
pub(super) struct Layers {
    ways: [Way; 2],
}
impl fmt::Debug for Layers {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(
            formatter,
            "layers of {} files, {} and {} pairs across them",
            self.ways[LAYERED].of.len(),
            self.ways[LAYERED].cross.len(),
            self.ways[RINGED].cross.len()
        )
    }
}
/// The layers of one folder.
struct Held {
    /// The way its files are filed: by the four layers, or by the rings.
    way: usize,
    /// How many files each layer holds.
    files: Vec<u64>,
    /// Code under the folder that no layer holds.
    apart: u64,
    /// Through how many links each layer needs each other one.
    needs: Vec<Vec<EdgeFlow>>,
    /// The pairs of files of two layers, the most linked first.
    pairs: Vec<(u32, u32, Flow)>,
}
/// Whether a path lies under a folder; the empty folder is the project.
fn under(scope: &str, path: &str) -> bool {
    scope.is_empty()
        || path
            .strip_prefix(scope)
            .is_some_and(|rest| rest.starts_with('/'))
}
impl Layers {
    /// Reads the layers of a graph. `certain` leaves out the links the tool
    /// only inferred.
    pub(super) fn read(index: &Index, certain: bool) -> Self {
        Self {
            ways: [
                Way::read(index, certain, &layer),
                Way::read(index, certain, &ring),
            ],
        }
    }
}
impl Way {
    /// Files every file of a graph and counts what the files of one layer
    /// need of those of another.
    fn read(index: &Index, certain: bool, filed: &dyn Fn(&str) -> Option<usize>) -> Self {
        let of = index
            .files
            .iter()
            .map(|path| filed(path).map_or(NO, |layer| layer as u8))
            .collect::<Vec<_>>();
        let requiring = ["calls", "inherits", "extends", "implements"]
            .iter()
            .filter_map(|name| index.kinds.iter().position(|kind| kind == name))
            .collect::<Vec<_>>();
        let references = index
            .kinds
            .iter()
            .position(|kind| kind == "references")
            .map(|kind| kind as u16);
        let mut cross: HashMap<(u32, u32), EdgeFlow> = HashMap::new();
        let links = index
            .uses
            .iter()
            .filter(|(.., kind, _)| requiring.contains(&(*kind as usize)))
            .map(|(from, to, kind, guessed)| (*from, *to, Some(*kind), *guessed))
            .chain(
                index
                    .ties
                    .iter()
                    .map(|(from, to, _, guessed)| (*from, *to, references, *guessed)),
            );
        for (from, to, kind, guessed) in links {
            if certain && guessed {
                continue;
            }
            let (from, to) = (
                index.symbols[from as usize].file,
                index.symbols[to as usize].file,
            );
            let (above, below) = (of[from as usize], of[to as usize]);
            if from == to || above == NO || below == NO || above == below {
                continue;
            }
            let entry = cross.entry((from, to)).or_default();
            if let Some(kind) = kind {
                entry.add_link(kind, guessed);
            } else {
                entry.flow.links += 1;
                entry.flow.guessed += u64::from(guessed);
            }
        }
        let mut cross = cross
            .into_iter()
            .map(|((from, to), edge)| (from, to, edge))
            .collect::<Vec<_>>();
        cross.sort_by(|a, b| {
            b.2.flow
                .links
                .cmp(&a.2.flow.links)
                .then_with(|| index.files[a.0 as usize].cmp(&index.files[b.0 as usize]))
                .then_with(|| index.files[a.1 as usize].cmp(&index.files[b.1 as usize]))
        });
        Self { of, cross }
    }
}
impl Layers {
    /// The layers of a folder: `None` where no file of one layer needs a
    /// file of another, which takes two layers, or the layers hold too
    /// little of the folder to say it is built in them. So it is for what
    /// is no folder: nothing lies under it. A folder that keeps most of its
    /// code in rings is read by them, any other by the four layers.
    fn held(&self, index: &Index, scope: &str) -> Option<Held> {
        [RINGED, LAYERED]
            .into_iter()
            .find_map(|way| self.held_by(index, scope, way))
    }
    fn held_by(&self, index: &Index, scope: &str, way: usize) -> Option<Held> {
        let held = self.ways[way].held(index, scope, way, WAYS[way].len());
        let filed = held.files.iter().sum::<u64>();
        (!held.pairs.is_empty() && filed >= FEWEST && filed * SHARES[way] >= filed + held.apart)
            .then_some(held)
    }
}
impl Way {
    fn held(&self, index: &Index, scope: &str, way: usize, layers: usize) -> Held {
        let Way { of, cross } = self;
        let mut held = Held {
            way,
            files: vec![0; layers],
            apart: 0,
            needs: vec![vec![EdgeFlow::default(); layers]; layers],
            pairs: Vec::new(),
        };
        for (file, path) in index.files.iter().enumerate() {
            if !under(scope, path) {
                continue;
            }
            match of[file] {
                NO => held.apart += u64::from(code(path)),
                layer => held.files[layer as usize] += 1,
            }
        }
        for (from, to, edge) in cross {
            if under(scope, &index.files[*from as usize])
                && under(scope, &index.files[*to as usize])
            {
                held.needs[of[*from as usize] as usize][of[*to as usize] as usize].merge(edge);
                held.pairs.push((*from, *to, edge.flow));
            }
        }
        held
    }
}
impl Layers {
    /// The layers of the open folder as a row of the list of found
    /// patterns, with what tells them: that differs by the way the folder
    /// is filed.
    pub(super) fn row(&self, index: &Index, scope: &str) -> Option<(Value, &'static str)> {
        let held = self.held(index, scope)?;
        let against = held.against().links;
        let row = json!({
            "id": format!("layers@{scope}"),
            "sort": "layers",
            "name": "Layers",
            "of": if scope.is_empty() { "the whole project" } else { name(scope) },
            "file": "",
            "line": 0,
            "named": true,
            "guessed": false,
            "parts": format!(
                "{} layers · {} files · {}",
                held.files.iter().filter(|files| **files > 0).count(),
                held.files.iter().sum::<u64>(),
                match against {
                    0 => "nothing against their order".to_owned(),
                    1 => "1 link against their order".to_owned(),
                    many => format!("{many} links against their order"),
                },
            ),
        });
        Some((row, TELLS[held.way]))
    }
    /// The picture of the layers of a folder: each layer that holds files
    /// as one box, and between them through how many links the files of the
    /// one need those of the other. A selected layer (`layer:services`)
    /// lists its files and what it has with the other layers, and a
    /// selected link between two of them (`layer:services>layer:model`) the
    /// pairs of files it is made of. `None` where the folder has no layers,
    /// as after the graph was built anew.
    pub(super) fn view(
        &self,
        index: &Index,
        project: &str,
        scope: &str,
        focus: Option<&str>,
        certain: bool,
    ) -> Option<Value> {
        let held = self.held(index, scope)?;
        let way = &self.ways[held.way];
        let table = WAYS[held.way]
            .iter()
            .map(|layer| DisplayLayer {
                id: layer.id,
                name: layer.name,
            })
            .collect::<Vec<_>>();
        Some(held.view(index, project, scope, focus, certain, &table, &way.of, None))
    }
}

impl Index {
    pub(crate) fn declared_layers(
        &self,
        project: &str,
        scope: &str,
        focus: Option<&str>,
        certain: bool,
        rules: &Rules,
    ) -> Value {
        let way = Way::read(self, certain, &|path| declared(rules, path));
        let held = way.held(self, scope, LAYERED, rules.layers.len());
        let table = rules
            .layers
            .iter()
            .map(|layer| DisplayLayer {
                id: &layer.id,
                name: &layer.name,
            })
            .collect::<Vec<_>>();
        held.view(
            self,
            project,
            scope,
            focus,
            certain,
            &table,
            &way.of,
            Some(rules),
        )
    }

    pub(crate) fn mark_architecture(&self, view: &mut Value, certain: bool, rules: Option<&Rules>) {
        self.mark_architecture_inner(view, certain, rules);
        if let Some(rules) = rules {
            self.mark_extended_architecture(view, certain, rules);
        }
    }

    fn mark_architecture_inner(&self, view: &mut Value, certain: bool, rules: Option<&Rules>) {
        if !matches!(view["mode"].as_str(), None | Some("changes" | "file")) {
            return;
        }
        let asked = view["scope"].as_str().unwrap_or("");
        let scope = self.opens(asked).map_or(asked, holder);
        let (way, table) = if let Some(rules) = rules {
            (
                Way::read(self, certain, &|path| declared(rules, path)),
                rules
                    .layers
                    .iter()
                    .map(|layer| (layer.id.as_str(), layer.name.as_str()))
                    .collect::<Vec<_>>(),
            )
        } else {
            let layers = &self.patterns_of(certain).layers;
            let Some(held) = layers.held(self, scope).or_else(|| layers.held(self, "")) else {
                return;
            };
            let table = WAYS[held.way]
                .iter()
                .map(|layer| (layer.id, layer.name))
                .collect();
            let [layered, ringed] = &layers.ways;
            (
                if held.way == RINGED {
                    Way {
                        of: ringed.of.clone(),
                        cross: ringed.cross.clone(),
                    }
                } else {
                    Way {
                        of: layered.of.clone(),
                        cross: layered.cross.clone(),
                    }
                },
                table,
            )
        };
        let opened = self.opens(asked);
        let within = |path: &str| opened.map_or_else(|| under(scope, path), |file| file == path);
        let path_id: HashMap<_, _> = self
            .files
            .iter()
            .enumerate()
            .map(|(id, path)| (path.as_str(), id))
            .collect();
        let forbidden = |from, to| rules.map_or(from > to, |rules| rules.forbidden(from, to));
        let mut pairs: BTreeMap<(usize, usize), Flow> = BTreeMap::new();
        let mut violations = Vec::new();
        let mut violation_links = 0;
        let mut marked = HashMap::new();
        let mut violating_edges: Vec<(bool, bool, &str, &str, &str, &str, Flow)> = Vec::new();
        for &(from, to, ref edge) in &way.cross {
            let flow = edge.flow;
            let (a, b) = (from as usize, to as usize);
            let (source, target) = (&self.files[a], &self.files[b]);
            let (from_layer, to_layer) = (way.of[a] as usize, way.of[b] as usize);
            if !within(source) && !within(target) {
                continue;
            }
            let pair = pairs.entry((from_layer, to_layer)).or_default();
            pair.links += flow.links;
            pair.guessed += flow.guessed;
            if forbidden(from_layer, to_layer) {
                marked.insert((source.as_str(), target.as_str()), flow.links);
                violation_links += flow.links;
                let is_skip = rules.is_some() && from_layer < to_layer;
                violating_edges.push((
                    rules.is_some(),
                    is_skip,
                    source.as_str(),
                    target.as_str(),
                    table[from_layer].1,
                    table[to_layer].1,
                    flow,
                ));
                if violations.len() < LISTED {
                    let end = |path: &str, layer: usize| json!({"path":path, "name":name(path), "layer":table[layer].1});
                    violations.push(json!({"from":end(source, from_layer), "to":end(target, to_layer), "links":flow.links, "guessed":flow.guessed}));
                }
            }
        }
        let layer_json = |id: usize| json!({"id":table[id].0, "name":table[id].1, "color":id});
        let reciprocal: Vec<_> = pairs
            .iter()
            .filter_map(|(&(from, to), &forward)| {
                if from >= to {
                    return None;
                }
                let reverse = pairs.get(&(to, from))?;
                Some(json!({"from":layer_json(from), "to":layer_json(to),
                "forward":forward.links, "reverse":reverse.links,
                "forward_guessed":forward.guessed, "reverse_guessed":reverse.guessed}))
            })
            .collect();
        let mut visible = BTreeSet::new();
        let mut file_nodes = HashSet::new();
        let mut folder_nodes = HashSet::new();
        // The file each box of an opened file stands in: a symbol in the
        // file it says or in the open one, a file around them in itself.
        let mut file_of: HashMap<String, String> = HashMap::new();
        for node in view["nodes"].as_array_mut().into_iter().flatten() {
            let node_path = node["path"].as_str().unwrap_or("");
            let path = node["file"]
                .as_str()
                .or_else(|| if node["symbol"] == true { opened } else { None })
                .unwrap_or(node_path);
            if node["folder"] != true {
                file_of.insert(node_path.to_owned(), path.to_owned());
            }
            let layer = if node["folder"] == true {
                folder_nodes.insert(node_path.to_owned());
                let scope_path = if path == "." { "" } else { path };
                let mut under = self
                    .files
                    .iter()
                    .zip(&way.of)
                    .filter(|(file, _)| under(scope_path, file))
                    .map(|(_, layer)| *layer);
                under
                    .next()
                    .filter(|first| under.all(|next| next == *first))
            } else {
                if node["symbol"] != true {
                    file_nodes.insert(node_path.to_owned());
                }
                path_id.get(path).map(|&file| way.of[file])
            };
            if let Some(layer) = layer.filter(|&id| id != NO) {
                visible.insert(layer as usize);
                node["architecture_layer"] = layer_json(layer as usize);
            }
        }
        let box_for = |path| {
            if let Some(file) = file_nodes.get(path) {
                return Some(file.as_str());
            }
            let mut parent = holder(path);
            while !parent.is_empty() {
                if let Some(folder) = folder_nodes.get(parent) {
                    return Some(folder.as_str());
                }
                parent = holder(parent);
            }
            folder_nodes.get(".").map(String::as_str)
        };
        let mut drawn = HashMap::new();
        let symbols = view["mode"] == "file" || view["lod"] == "symbol";
        // Folder arrows represent many file links. Attribute a violation only
        // to the boxes representing its two ends.
        if !symbols {
            for (&(source, target), links) in &marked {
                if let Some(ends) = box_for(source).zip(box_for(target))
                    && ends.0 != ends.1
                {
                    *drawn.entry(ends).or_insert(0) += links;
                }
            }
        }
        // A picture draws the strongest links of a dense folder, and no more
        // than its limit. A link that breaks a rule is what the rules are
        // asked for, so it is drawn whatever those limits left out: the
        // most links first, `KEPT_VIOLATIONS` of them.
        let mut kept = 0;
        if !symbols && !drawn.is_empty() {
            let present = view["edges"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|edge| edge["from"].as_str().zip(edge["to"].as_str()))
                .collect::<HashSet<_>>();
            let mut missing: BTreeMap<(&str, &str), EdgeFlow> = drawn
                .keys()
                .filter(|ends| !present.contains(*ends))
                .map(|ends| (*ends, EdgeFlow::default()))
                .collect();
            if !missing.is_empty() {
                for (from, to, pair) in &self.pairs {
                    if pair.weight(certain) == 0 {
                        continue;
                    }
                    let ends = box_for(&self.files[*from as usize])
                        .zip(box_for(&self.files[*to as usize]));
                    if let Some(flow) = ends.and_then(|ends| missing.get_mut(&ends)) {
                        flow.add_pair(pair, certain);
                    }
                }
                let mut missing = missing
                    .into_iter()
                    .filter(|(_, flow)| flow.flow.links > 0)
                    .collect::<Vec<_>>();
                missing.sort_by(|(a, left), (b, right)| {
                    drawn[b]
                        .cmp(&drawn[a])
                        .then(right.flow.links.cmp(&left.flow.links))
                        .then(a.cmp(b))
                });
                missing.truncate(KEPT_VIOLATIONS);
                kept = missing.len();
                let added = missing
                    .iter()
                    .map(|((from, to), flow)| {
                        let mut edge = self.edge_json(from, to, flow);
                        edge["kept"] = json!("violation");
                        edge
                    })
                    .collect::<Vec<_>>();
                if let Some(edges) = view["edges"].as_array_mut() {
                    edges.extend(added);
                }
            }
        }
        for edge in view["edges"].as_array_mut().into_iter().flatten() {
            let endpoints = edge["from"].as_str().zip(edge["to"].as_str());
            if symbols {
                // In an opened file every link from a file of one layer to a
                // file of a layer it may not use breaks the rule.
                let files = endpoints.and_then(|(from, to)| file_of.get(from).zip(file_of.get(to)));
                if let Some((from, to)) = files
                    && from != to
                    && marked.contains_key(&(from.as_str(), to.as_str()))
                {
                    edge["architecture_violation"] = json!(edge["links"].as_u64().unwrap_or(1));
                }
            } else if let Some(links) = endpoints.and_then(|ends| drawn.get(&ends)) {
                edge["architecture_violation"] = json!(links);
            }
        }
        let remediation = layer_remediation_hints(violating_edges.into_iter());
        view["architecture"] = json!({
            "declared":rules.is_some(),
            "layers":visible.into_iter().map(layer_json).collect::<Vec<_>>(),
            "kept":kept,
            "violations":{"links":violation_links, "pairs":marked.len(),
                "more":marked.len().saturating_sub(violations.len()), "rows":violations},
            "remediation": remediation,
            "reciprocal_total":reciprocal.len(),
            "reciprocal":reciprocal.into_iter().take(LISTED).collect::<Vec<_>>(),
        });
    }

    pub(crate) fn mark_declared_layers(&self, view: &mut Value, rules: Option<&Rules>) {
        let asked = view["scope"].as_str().unwrap_or("");
        let scope = self.opens(asked).map_or(asked, holder);
        let mut kinds = view["patterns"]["kinds"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let before = kinds.len();
        kinds.retain(|kind| kind["id"] != "layers");
        let mut found = view["patterns"]["found"]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub((before - kinds.len()) as u64);
        if let Some(rules) = rules {
            let files = self
                .files
                .iter()
                .filter(|path| under(scope, path) && declared(rules, path).is_some())
                .count();
            let row = json!({
                "id": format!("layers@{scope}"), "sort": "layers", "name": "Layers",
                "of": if scope.is_empty() { "the whole project" } else { name(scope) },
                "file": "", "line": 0, "named": false, "guessed": false,
                "parts": format!("Project rules · {files} classified files"),
            });
            kinds.insert(
                0,
                json!({
                    "id": "layers", "name": "Layers", "family": "architecture",
                    "tells": DECLARED, "found": 1, "more": 0, "instances": [row],
                }),
            );
            found += 1;
        }
        view["patterns"] = json!({"found":found, "kinds":kinds});
    }

    pub(crate) fn compare_declared_layers(
        &self,
        base: &Index,
        view: &mut Value,
        renamed: &HashMap<&str, &str>,
        certain: bool,
        rules: &Rules,
    ) {
        let head_way = Way::read(self, certain, &|path| declared(rules, path));
        let base_way = Way::read(base, certain, &|path| declared(rules, path));
        let table: Vec<DisplayLayer<'_>> = rules
            .layers
            .iter()
            .map(|l| DisplayLayer {
                id: &l.id,
                name: &l.name,
            })
            .collect();
        compare_ways(
            self,
            base,
            &head_way,
            &base_way,
            &table,
            &|above, below| rules.forbidden(above, below),
            view,
            renamed,
        );
    }

    /// Lists all forbidden file-level dependencies against `.harness-architecture.json` `rules`.
    pub(crate) fn declared_rule_violations(
        &self,
        certain: bool,
        rules: &Rules,
    ) -> Vec<(String, String, u64, String)> {
        let way = Way::read(self, certain, &|path| declared(rules, path));
        way.forbidden_pairs(&|above, below| rules.forbidden(above, below))
            .into_iter()
            .map(|pair| {
                let from_layer = rules
                    .layers
                    .get(pair.above)
                    .map_or("unknown", |l| l.name.as_str());
                let to_layer = rules
                    .layers
                    .get(pair.below)
                    .map_or("unknown", |l| l.name.as_str());
                (
                    self.files[pair.from as usize].clone(),
                    self.files[pair.to as usize].clone(),
                    pair.flow.links,
                    format!("{from_layer} -> {to_layer}"),
                )
            })
            .collect()
    }
}

impl Held {
    #[allow(clippy::too_many_arguments)]
    fn view(
        &self,
        index: &Index,
        project: &str,
        scope: &str,
        focus: Option<&str>,
        certain: bool,
        table: &[DisplayLayer<'_>],
        of: &[u8],
        rules: Option<&Rules>,
    ) -> Value {
        let held = self;
        let forbidden = |from, to| rules.map_or(from > to, |rules| rules.forbidden(from, to));
        let key = |layer: usize| format!("layer:{}", table[layer].id);
        let present = (0..table.len())
            .filter(|layer| held.files[*layer] > 0)
            .collect::<Vec<_>>();
        let sum = |flows: &mut dyn Iterator<Item = Flow>| {
            flows.fold(Flow::default(), |sum, flow| Flow {
                links: sum.links + flow.links,
                guessed: sum.guessed + flow.guessed,
            })
        };
        let node = |layer: usize| {
            json!({
                "path": key(layer), "name": table[layer].name, "folder": true, "layer": true,
                "files": held.files[layer], "symbols": 0, "within": 0, "changed": 0,
                "out": sum(&mut held.needs[layer].iter().map(|edge| edge.flow)).links,
                "in": sum(&mut held.needs.iter().map(|row| row[layer].flow)).links,
            })
        };
        let pair = |from: u32, to: u32, flow: Flow| {
            let end = |file: u32| {
                let path = index.files[file as usize].as_str();
                json!({
                    "path": path, "name": name(path),
                    "layer": table[of[file as usize] as usize].name,
                })
            };
            json!({"from": end(from), "to": end(to), "links": flow.links, "guessed": flow.guessed})
        };
        let back =
            |from: u32, to: u32| forbidden(of[from as usize] as usize, of[to as usize] as usize);
        let against = held
            .pairs
            .iter()
            .filter(|(from, to, _)| back(*from, *to))
            .collect::<Vec<_>>();
        let total = sum(&mut against.iter().map(|(_, _, flow)| *flow));
        let mut told = Vec::new();
        if held.apart > 0 {
            told.push(format!(
                "{} of code here {} filed in no layer.",
                match held.apart {
                    1 => "1 file".to_owned(),
                    many => format!("{many} files"),
                },
                if held.apart == 1 { "is" } else { "are" },
            ));
        }
        let mut crumbs = Index::crumbs(project, scope);
        crumbs.push(json!({"name": "Layers", "path": ""}));
        let mut view = json!({
            "mode": "pattern", "scope": scope, "crumbs": crumbs, "certain": certain,
            "totals": index.totals(),
            "pattern": {
                "id": format!("layers@{scope}"),
                "sort": "layers",
                "name": "Layers",
                "family": "architecture",
                "tells": if rules.is_some() { DECLARED } else { TELLS[held.way] },
                "declared": rules.is_some(),
                "rings": rules.is_none() && held.way == RINGED,
                "named": rules.is_none(),
                "guessed": total.guessed > 0,
                "of": if scope.is_empty() {
                    json!({"path": "", "name": project, "what": "project"})
                } else {
                    json!({"path": scope, "name": name(scope), "what": "folder"})
                },
                "told": told,
                "layers": present
                    .iter()
                    .map(|layer| json!({
                        "id": table[*layer].id, "name": table[*layer].name,
                        "files": held.files[*layer],
                    }))
                    .collect::<Vec<_>>(),
                "apart": held.apart,
                "against": {
                    "links": total.links,
                    "guessed": total.guessed,
                    "pairs": against.len(),
                    "rows": against
                        .iter()
                        .take(LISTED)
                        .map(|(from, to, flow)| pair(*from, *to, *flow))
                        .collect::<Vec<_>>(),
                    "more": against.len().saturating_sub(LISTED),
                },
                "remediation": layer_remediation_hints(against.iter().map(|&&(from, to, flow)| {
                    let from_idx = of[from as usize] as usize;
                    let to_idx = of[to as usize] as usize;
                    (
                        rules.is_some(),
                        rules.is_some() && from_idx < to_idx,
                        index.files[from as usize].as_str(),
                        index.files[to as usize].as_str(),
                        table[from_idx].name,
                        table[to_idx].name,
                        flow,
                    )
                })),
            },
            "nodes": present.iter().map(|layer| node(*layer)).collect::<Vec<_>>(),
            "edges": present
                .iter()
                .flat_map(|from| present.iter().map(move |to| (*from, *to)))
                .filter(|(from, to)| held.needs[*from][*to].flow.links > 0)
                .map(|(from, to)| {
                    let mut edge = index.edge_json(&key(from), &key(to), &held.needs[from][to]);
                    edge["against"] = json!(forbidden(from, to));
                    edge
                })
                .collect::<Vec<_>>(),
            "focus": null,
        });
        let layer = |asked: &str| present.iter().copied().find(|layer| key(*layer) == asked);
        // A link between two layers is many links of many files: the pairs
        // of files it is made of, the most linked first. A link that is not
        // drawn selects nothing.
        if let Some((from, to)) = focus.and_then(|focus| focus.split_once('>')) {
            if let (Some(from), Some(to)) = (layer(from), layer(to))
                && held.needs[from][to].flow.links > 0
            {
                let flow = held.needs[from][to].flow;
                let made = held
                    .pairs
                    .iter()
                    .filter(|(user, used, _)| {
                        of[*user as usize] as usize == from && of[*used as usize] as usize == to
                    })
                    .collect::<Vec<_>>();
                view["focus"] = json!({
                    "path": format!("{}>{}", key(from), key(to)),
                    "link": true,
                    "from": {"path": key(from), "name": table[from].name},
                    "to": {"path": key(to), "name": table[to].name},
                    "links": flow.links,
                    "guessed": flow.guessed,
                    "against": forbidden(from, to),
                    "pairs": made.len(),
                    "rows": made
                        .iter()
                        .take(LISTED)
                        .map(|(user, used, flow)| pair(*user, *used, *flow))
                        .collect::<Vec<_>>(),
                    "more": made.len().saturating_sub(LISTED),
                });
            }
            return view;
        }
        let Some(chosen) = focus.and_then(layer) else {
            return view;
        };
        // The files of the layer, by how much they have with other layers.
        let mut members: HashMap<u32, Flow> = HashMap::new();
        for (from, to, flow) in &held.pairs {
            for file in [from, to] {
                if of[*file as usize] as usize == chosen {
                    let own = members.entry(*file).or_default();
                    own.links += flow.links;
                    own.guessed += flow.guessed;
                }
            }
        }
        let mut members = index
            .files
            .iter()
            .enumerate()
            .filter(|(file, path)| of[*file] as usize == chosen && under(scope, path))
            .map(|(file, path)| {
                (
                    path.as_str(),
                    members.get(&(file as u32)).copied().unwrap_or_default(),
                )
            })
            .collect::<Vec<_>>();
        members.sort_by(|a, b| b.1.links.cmp(&a.1.links).then(a.0.cmp(b.0)));
        let beside = |other: usize, flow: Flow, back: bool| {
            json!({
                "path": key(other), "name": table[other].name, "folder": false, "inside": true,
                "links": flow.links, "guessed": flow.guessed, "changed": false,
                "against": back,
            })
        };
        let mut focus = node(chosen);
        focus["inside"] = json!(true);
        focus["signs"] = json!(rules.map_or_else(
            || signs(held.way, chosen),
            |rules| rules.layers[chosen].paths.join(", ")
        ));
        focus["uses"] = present
            .iter()
            .filter(|other| held.needs[chosen][**other].flow.links > 0)
            .map(|other| {
                beside(
                    *other,
                    held.needs[chosen][*other].flow,
                    forbidden(chosen, *other),
                )
            })
            .collect();
        focus["used_by"] = present
            .iter()
            .filter(|other| held.needs[**other][chosen].flow.links > 0)
            .map(|other| {
                beside(
                    *other,
                    held.needs[*other][chosen].flow,
                    forbidden(*other, chosen),
                )
            })
            .collect();
        focus["members"] = members
            .iter()
            .take(LISTED)
            .map(|(path, flow)| {
                json!({
                    "path": path, "name": name(path), "folder": false, "inside": false,
                    "links": flow.links, "guessed": flow.guessed, "changed": false,
                })
            })
            .collect();
        focus["members_more"] = json!(members.len().saturating_sub(LISTED));
        // What this layer needs of the layers before it.
        let own = against
            .iter()
            .filter(|(from, ..)| of[*from as usize] as usize == chosen)
            .collect::<Vec<_>>();
        focus["against"] = json!({
            "rows": own
                .iter()
                .take(LISTED)
                .map(|(from, to, flow)| pair(*from, *to, *flow))
                .collect::<Vec<_>>(),
            "more": own.len().saturating_sub(LISTED),
        });
        view["focus"] = focus;
        view
    }
}
/// One pair of files of two layers as a row: the two files with the layers
/// they are filed in, and through how many links the first needs the second.
fn paired(index: &Index, table: &[DisplayLayer<'_>], pair: &Backward) -> Value {
    let end = |file: u32, layer: usize| {
        let path = index.files[file as usize].as_str();
        json!({"path": path, "name": name(path), "layer": table[layer].name})
    };
    json!({
        "from": end(pair.from, pair.above), "to": end(pair.to, pair.below),
        "links": pair.flow.links, "guessed": pair.flow.guessed,
    })
}
/// A pair of files of which the first needs the second against the rules of
/// their layers.
struct Backward {
    from: u32,
    to: u32,
    above: usize,
    below: usize,
    flow: Flow,
}
impl Way {
    fn forbidden_pairs(&self, forbidden: &dyn Fn(usize, usize) -> bool) -> Vec<Backward> {
        self.cross
            .iter()
            .map(|(from, to, edge)| Backward {
                from: *from,
                to: *to,
                above: self.of[*from as usize] as usize,
                below: self.of[*to as usize] as usize,
                flow: edge.flow,
            })
            .filter(|pair| forbidden(pair.above, pair.below))
            .collect()
    }
}
#[allow(clippy::too_many_arguments)]
fn compare_ways(
    index: &Index,
    base: &Index,
    head_way: &Way,
    base_way: &Way,
    table: &[DisplayLayer<'_>],
    forbidden: &dyn Fn(usize, usize) -> bool,
    view: &mut Value,
    renamed: &HashMap<&str, &str>,
) {
    let scope = view["scope"].as_str().unwrap_or("").to_owned();
    let moved = renamed
        .iter()
        .map(|(new, old)| (*old, *new))
        .collect::<HashMap<_, _>>();
    let before = |pair: &Backward| {
        let old = |file: u32| {
            let path = index.files[file as usize].as_str();
            renamed.get(path).copied().unwrap_or(path)
        };
        (old(pair.from), old(pair.to))
    };
    let after = |pair: &Backward| {
        let new = |file: u32| {
            let path = base.files[file as usize].as_str();
            moved.get(path).copied().unwrap_or(path)
        };
        (new(pair.from), new(pair.to))
    };
    fn here<'a>(index: &'a Index, pair: &Backward) -> (&'a str, &'a str) {
        (
            index.files[pair.from as usize].as_str(),
            index.files[pair.to as usize].as_str(),
        )
    }
    let within = |(from, to): (&str, &str)| under(&scope, from) && under(&scope, to);
    let (now, was) = (
        head_way.forbidden_pairs(forbidden),
        base_way.forbidden_pairs(forbidden),
    );
    let had = was
        .iter()
        .map(|pair| here(base, pair))
        .collect::<HashSet<_>>();
    let has = now.iter().map(before).collect::<HashSet<_>>();
    let added = now
        .iter()
        .filter(|pair| within(here(index, pair)) && !had.contains(&before(pair)))
        .collect::<Vec<_>>();
    let removed = was
        .iter()
        .filter(|pair| within(after(pair)) && !has.contains(&here(base, pair)))
        .collect::<Vec<_>>();
    let listed = |idx: &Index, pairs: &[&Backward]| {
        let flow = pairs.iter().fold(Flow::default(), |sum, pair| Flow {
            links: sum.links + pair.flow.links,
            guessed: sum.guessed + pair.flow.guessed,
        });
        json!({
            "pairs": pairs.len(), "links": flow.links, "guessed": flow.guessed,
            "rows": pairs
                .iter()
                .take(LISTED)
                .map(|pair| paired(idx, table, pair))
                .collect::<Vec<_>>(),
            "more": pairs.len().saturating_sub(LISTED),
        })
    };
    let new = added
        .iter()
        .map(|pair| (pair.from, pair.to))
        .collect::<HashSet<_>>();
    let rows = |keep: &dyn Fn(&Backward) -> bool| {
        let (first, rest): (Vec<_>, Vec<_>) = now
            .iter()
            .filter(|pair| within(here(index, pair)) && keep(pair))
            .partition(|pair| new.contains(&(pair.from, pair.to)));
        let told = first.len();
        first
            .into_iter()
            .chain(rest)
            .take(LISTED)
            .enumerate()
            .map(|(at, pair)| {
                let mut row = paired(index, table, pair);
                if at < told {
                    row["added"] = json!(true);
                }
                row
            })
            .collect::<Vec<_>>()
    };
    let layer = |path: &Value| {
        let id = path.as_str()?.strip_prefix("layer:")?;
        table.iter().position(|layer| layer.id == id)
    };
    view["pattern"]["against"]["rows"] = json!(rows(&|_| true));
    let focus = &view["focus"];
    if focus["link"] == true {
        if let Some((from, to)) = layer(&focus["from"]["path"]).zip(layer(&focus["to"]["path"]))
            && forbidden(from, to)
        {
            view["focus"]["rows"] = json!(rows(&|pair| pair.above == from && pair.below == to));
        }
    } else if let Some(chosen) = layer(&focus["path"]) {
        view["focus"]["against"]["rows"] = json!(rows(&|pair| pair.above == chosen));
    }
    let mut through: HashMap<(usize, usize), u64> = HashMap::new();
    for pair in &added {
        *through.entry((pair.above, pair.below)).or_default() += pair.flow.links;
    }
    for edge in view["edges"].as_array_mut().into_iter().flatten() {
        if let Some(links) = layer(&edge["from"])
            .zip(layer(&edge["to"]))
            .and_then(|ends| through.get(&ends))
        {
            edge["added"] = json!(links);
        }
    }
    view["pattern"]["change"] = json!({
        "state": "ready",
        "added": listed(index, &added),
        "removed": listed(base, &removed),
    });
}
impl Layers {
    pub(super) fn compare(
        &self,
        index: &Index,
        then: &Layers,
        base: &Index,
        view: &mut Value,
        renamed: &HashMap<&str, &str>,
    ) {
        let scope = view["scope"].as_str().unwrap_or("");
        let way = self.held(index, scope).map_or(LAYERED, |held| held.way);
        let table: Vec<DisplayLayer<'_>> = WAYS[way]
            .iter()
            .map(|l| DisplayLayer {
                id: l.id,
                name: l.name,
            })
            .collect();
        compare_ways(
            index,
            base,
            &self.ways[way],
            &then.ways[way],
            &table,
            &|above, below| above > below,
            view,
            renamed,
        );
    }
}
impl Held {
    /// Through how many links a layer needs one before it.
    fn against(&self) -> Flow {
        let mut sum = Flow::default();
        for (from, row) in self.needs.iter().enumerate() {
            for edge in &row[..from] {
                sum.links += edge.flow.links;
                sum.guessed += edge.flow.guessed;
            }
        }
        sum
    }
}

/// Structured architectural rule violation covering layer, package/crate, cycle budget, and semantic rules.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct ArchitectureViolation {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    pub from: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<String>,
    pub message: String,
}

impl Index {
    /// Evaluates declared layer, `forbids`, package/crate boundary, `max_cycles`, and semantic-edge rules.
    pub(crate) fn architecture_violations(
        &self,
        scope: &str,
        certain: bool,
        rules: &Rules,
    ) -> Vec<ArchitectureViolation> {
        let mut out = Vec::new();
        let norm_scope = scope.trim_matches('/');

        // 1. Layer violations (allows + forbids).
        if !rules.layers.is_empty() {
            for (from_path, to_path, _, _) in self.declared_rule_violations(certain, rules) {
                if !under(norm_scope, &from_path) && !under(norm_scope, &to_path) {
                    continue;
                }
                let from_layer = rules.layer(&from_path);
                let to_layer = rules.layer(&to_path);
                let from_name =
                    from_layer.map_or(from_path.as_str(), |i| rules.layers[i].name.as_str());
                let to_name = to_layer.map_or(to_path.as_str(), |i| rules.layers[i].name.as_str());
                let message = format!("Layer `{from_name}` forbids dependencies on `{to_name}`.");
                out.push(ArchitectureViolation {
                    kind: "layer".to_owned(),
                    rule_id: from_layer.map(|i| rules.layers[i].id.clone()),
                    from: from_path,
                    to: to_path,
                    relation: None,
                    message,
                });
            }
        }

        // 2. Package & Rust crate boundary violations.
        if rules.enforce_crate_boundaries || !rules.package_rules.is_empty() {
            let manifest_dirs = self.manifest_dirs();
            let mut seen_pkg_pairs = HashSet::new();
            for &(from, to, ref pair) in &self.pairs {
                if pair.weight(certain) == 0 {
                    continue;
                }
                let from_path = &self.files[from as usize];
                let to_path = &self.files[to as usize];
                if from_path.starts_with("__semantic__/") || to_path.starts_with("__semantic__/") {
                    continue;
                }
                if !under(norm_scope, from_path) && !under(norm_scope, to_path) {
                    continue;
                }
                let from_bucket = crate::graph_map::lod::crate_bucket(manifest_dirs, from_path);
                let to_bucket = crate::graph_map::lod::crate_bucket(manifest_dirs, to_path);
                if let Some((rule_id, message)) =
                    rules.check_package_violation(from_path, to_path, &from_bucket, &to_bucket)
                {
                    if seen_pkg_pairs.insert((from_path.clone(), to_path.clone())) {
                        out.push(ArchitectureViolation {
                            kind: "package".to_owned(),
                            rule_id,
                            from: from_path.clone(),
                            to: to_path.clone(),
                            relation: None,
                            message,
                        });
                    }
                    continue;
                }
                if rules.enforce_crate_boundaries
                    && !self.crates.allows_files(&self.files, from_path, to_path)
                    && seen_pkg_pairs.insert((from_path.clone(), to_path.clone()))
                {
                    out.push(ArchitectureViolation {
                        kind: "package".to_owned(),
                        rule_id: Some("crate-boundary".to_owned()),
                        from: from_path.clone(),
                        to: to_path.clone(),
                        relation: None,
                        message: format!(
                            "Crate boundary violated from `{from_bucket}` to `{to_bucket}`."
                        ),
                    });
                }
            }
        }

        // 3. Cycle budgets (`max_cycles` global and per-layer).
        if rules.max_cycles.is_some() || rules.layers.iter().any(|l| l.max_cycles.is_some()) {
            let sccs = self.scc_groups(certain);
            let scoped_sccs: Vec<&Vec<String>> = sccs
                .iter()
                .filter(|g| norm_scope.is_empty() || g.iter().any(|p| under(norm_scope, p)))
                .collect();
            if let Some(budget) = rules.max_cycles
                && scoped_sccs.len() > budget
            {
                let summary = scoped_sccs
                    .iter()
                    .take(3)
                    .map(|g| g.join(" <-> "))
                    .collect::<Vec<_>>()
                    .join("; ");
                let first_file = scoped_sccs
                    .first()
                    .and_then(|g| g.first())
                    .cloned()
                    .unwrap_or_else(|| "workspace".to_owned());
                out.push(ArchitectureViolation {
                    kind: "cycle_budget".to_owned(),
                    rule_id: Some("workspace_max_cycles".to_owned()),
                    from: first_file,
                    to: summary,
                    relation: None,
                    message: format!(
                        "Dependency cycle count ({}) exceeds max_cycles budget ({budget}).",
                        scoped_sccs.len()
                    ),
                });
            }
            for (idx, layer) in rules.layers.iter().enumerate() {
                if let Some(budget) = layer.max_cycles {
                    let touching: Vec<&&Vec<String>> = scoped_sccs
                        .iter()
                        .filter(|g| g.iter().any(|p| rules.layer(p) == Some(idx)))
                        .collect();
                    if touching.len() > budget {
                        let first_file = touching
                            .first()
                            .and_then(|g| g.iter().find(|p| rules.layer(p) == Some(idx)))
                            .cloned()
                            .unwrap_or_else(|| layer.id.clone());
                        let summary = touching
                            .iter()
                            .take(3)
                            .map(|g| g.join(" <-> "))
                            .collect::<Vec<_>>()
                            .join("; ");
                        out.push(ArchitectureViolation {
                            kind: "cycle_budget".to_owned(),
                            rule_id: Some(layer.id.clone()),
                            from: first_file,
                            to: summary,
                            relation: None,
                            message: format!(
                                "Layer `{}` participates in {} dependency cycles (max_cycles budget: {budget}).",
                                layer.name,
                                touching.len()
                            ),
                        });
                    }
                }
            }
        }

        // 4. Semantic-edge rules (layer `forbids_semantic` + top-level `semantic_rules`).
        if !rules.semantic_rules.is_empty()
            || rules.layers.iter().any(|l| !l.forbids_semantic.is_empty())
        {
            let mut seen_sem = HashSet::new();
            for &(from_sym, to_sym, kind_id, guessed) in &self.uses {
                if certain && guessed {
                    continue;
                }
                let to_s = &self.symbols[to_sym as usize];
                let rel = self
                    .kinds
                    .get(kind_id as usize)
                    .map(String::as_str)
                    .unwrap_or("");
                let is_sem_rel = matches!(
                    rel,
                    "writes_table"
                        | "reads_table"
                        | "exposes_route"
                        | "calls_route"
                        | "publishes_event"
                        | "handles_event"
                        | "di_binds"
                );
                if !to_s.is_semantic && !is_sem_rel {
                    continue;
                }
                let from_path = &self.files[self.symbols[from_sym as usize].file as usize];
                if !under(norm_scope, from_path) {
                    continue;
                }
                let target_label =
                    &self.names[to_s.at as usize..to_s.at as usize + to_s.len as usize];
                let from_layer = rules.layer(from_path);
                if let Some((rule_id, message)) =
                    rules.check_semantic_violation(from_path, from_layer, rel, target_label)
                    && seen_sem.insert((from_path.clone(), rel.to_owned(), target_label.to_owned()))
                {
                    out.push(ArchitectureViolation {
                        kind: "semantic".to_owned(),
                        rule_id,
                        from: from_path.clone(),
                        to: target_label.to_owned(),
                        relation: Some(rel.to_owned()),
                        message,
                    });
                }
            }
        }

        out
    }

    fn mark_extended_architecture(&self, view: &mut Value, certain: bool, rules: &Rules) {
        if !rules.has_extended_rules() {
            return;
        }
        let scope = view["scope"].as_str().unwrap_or("").to_owned();
        let all = self.architecture_violations(&scope, certain, rules);
        let package_v: Vec<&ArchitectureViolation> =
            all.iter().filter(|v| v.kind == "package").collect();
        let cycle_v: Vec<&ArchitectureViolation> =
            all.iter().filter(|v| v.kind == "cycle_budget").collect();
        let semantic_v: Vec<&ArchitectureViolation> =
            all.iter().filter(|v| v.kind == "semantic").collect();

        if package_v.is_empty() && cycle_v.is_empty() && semantic_v.is_empty() {
            if let Some(pat) = view["pattern"].as_object_mut() {
                pat.insert("package_violations".to_owned(), json!([]));
                pat.insert(
                    "cycle_budget".to_owned(),
                    json!({ "exceeded": false, "violations": [] }),
                );
                pat.insert("semantic_violations".to_owned(), json!([]));
            }
            return;
        }

        let view_lod = view["lod"].as_str().map(str::to_owned);
        let symbols_mode =
            view["mode"].as_str() == Some("file") || view_lod.as_deref() == Some("symbol");
        let opened = self.opens(&scope);

        let mut drawn_nodes: HashSet<String> = HashSet::new();
        let mut file_nodes: HashSet<String> = HashSet::new();
        let mut folder_nodes: HashSet<String> = HashSet::new();
        let mut file_of_sym: HashMap<String, String> = HashMap::new();
        if let Some(nodes) = view["nodes"].as_array() {
            for node in nodes {
                let Some(path) = node["path"].as_str() else {
                    continue;
                };
                drawn_nodes.insert(path.to_owned());
                if node["folder"] == true {
                    folder_nodes.insert(path.to_owned());
                } else {
                    if node["symbol"] != true {
                        file_nodes.insert(path.to_owned());
                    }
                    let resolved = node["file"]
                        .as_str()
                        .or_else(|| if node["symbol"] == true { opened } else { None })
                        .unwrap_or_else(|| path.split_once('#').map_or(path, |(f, _)| f));
                    file_of_sym.insert(path.to_owned(), resolved.to_owned());
                }
            }
        }

        let manifest_dirs = if view_lod.as_deref() == Some("crate") {
            self.manifest_dirs()
        } else {
            &[]
        };
        let box_for = |file_path: &str| -> String {
            match view_lod.as_deref() {
                Some("crate") => crate::graph_map::lod::node_bucket(
                    manifest_dirs,
                    crate::graph_map::Lod::Crate,
                    file_path,
                ),
                Some("folder") => crate::graph_map::lod::node_bucket(
                    &[],
                    crate::graph_map::Lod::Folder,
                    file_path,
                ),
                _ => {
                    if file_nodes.contains(file_path) || drawn_nodes.contains(file_path) {
                        return file_path.to_owned();
                    }
                    let mut parent = holder(file_path);
                    while !parent.is_empty() {
                        if folder_nodes.contains(parent) {
                            return parent.to_owned();
                        }
                        parent = holder(parent);
                    }
                    if folder_nodes.contains(".") {
                        ".".to_owned()
                    } else {
                        file_path.to_owned()
                    }
                }
            }
        };

        #[derive(Clone, Debug)]
        struct MatchedEdgeRule {
            kind: &'static str,
            rule_id: Option<String>,
            relation: Option<String>,
            target: Option<String>,
        }

        let mut pkg_by_boxes: BTreeMap<(String, String), Vec<MatchedEdgeRule>> = BTreeMap::new();
        let mut sem_by_exact_target: BTreeMap<(String, String), Vec<MatchedEdgeRule>> =
            BTreeMap::new();
        let mut sem_by_bucket: BTreeMap<(String, String), Vec<MatchedEdgeRule>> = BTreeMap::new();
        let mut violated_boxes: HashSet<String> = HashSet::new();

        for v in &package_v {
            let fb = box_for(&v.from);
            let tb = box_for(&v.to);
            violated_boxes.insert(v.from.clone());
            violated_boxes.insert(fb.clone());
            if !fb.is_empty() && !tb.is_empty() && fb != tb {
                pkg_by_boxes
                    .entry((fb, tb))
                    .or_default()
                    .push(MatchedEdgeRule {
                        kind: "package",
                        rule_id: v.rule_id.clone(),
                        relation: None,
                        target: Some(v.to.clone()),
                    });
            }
        }
        for v in &semantic_v {
            let fb = box_for(&v.from);
            violated_boxes.insert(v.from.clone());
            violated_boxes.insert(fb.clone());
            let clean_target = v.to.trim().strip_prefix("sem:").unwrap_or(v.to.trim());
            let sem_bucket = crate::graph_ir::semantic_namespace_bucket(clean_target);
            let m = MatchedEdgeRule {
                kind: "semantic",
                rule_id: v.rule_id.clone(),
                relation: v.relation.clone(),
                target: Some(clean_target.to_owned()),
            };
            sem_by_exact_target
                .entry((fb.clone(), clean_target.to_owned()))
                .or_default()
                .push(m.clone());
            if !fb.is_empty() {
                sem_by_bucket
                    .entry((fb, sem_bucket.to_owned()))
                    .or_default()
                    .push(m);
            }
        }
        if !cycle_v.is_empty() {
            for group in self.scc_groups(certain) {
                if scope.is_empty() || group.iter().any(|p| under(&scope, p)) {
                    for p in group {
                        if let Some(stored) = self.files.iter().find(|f| f.as_str() == p) {
                            violated_boxes.insert(stored.clone());
                            violated_boxes.insert(box_for(stored));
                        }
                    }
                }
            }
        }

        let edge_semantic_target = |raw: &str| -> Option<String> {
            let trimmed = raw.trim();
            if let Some(stripped) = trimmed.strip_prefix("sem:") {
                return Some(stripped.to_owned());
            }
            if let Some((_, frag)) = trimmed.split_once('#') {
                let without_line = frag
                    .strip_prefix('L')
                    .and_then(|rest| {
                        rest.split_once(':').and_then(|(digits, label)| {
                            (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
                                .then_some(label)
                        })
                    })
                    .unwrap_or(frag);
                let candidate = without_line.strip_prefix("sem:").unwrap_or(without_line);
                if candidate.starts_with("route:")
                    || candidate.starts_with("table:")
                    || candidate.starts_with("event:")
                    || candidate.starts_with("service:")
                    || without_line.starts_with("sem:")
                {
                    return Some(candidate.to_owned());
                }
            }
            if trimmed.starts_with("route:")
                || trimmed.starts_with("table:")
                || trimmed.starts_with("event:")
                || trimmed.starts_with("service:")
            {
                return Some(trimmed.to_owned());
            }
            None
        };

        let apply_matches_to_edge = |edge: &mut Value, matches: &[MatchedEdgeRule]| {
            if matches.is_empty() {
                return;
            }
            edge["against"] = json!(true);
            let mut rule_ids: BTreeSet<String> = edge["rule_ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            let mut existing_rules: Vec<Value> = edge["architecture_rules"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let mut seen_keys: BTreeSet<(String, String, String, String)> = existing_rules
                .iter()
                .map(|r| {
                    (
                        r["kind"].as_str().unwrap_or("").to_owned(),
                        r["rule_id"].as_str().unwrap_or("").to_owned(),
                        r["relation"].as_str().unwrap_or("").to_owned(),
                        r["target"].as_str().unwrap_or("").to_owned(),
                    )
                })
                .collect();
            let mut newly_added = 0u64;
            for m in matches {
                if let Some(ref rid) = m.rule_id
                    && !rid.is_empty()
                {
                    rule_ids.insert(rid.clone());
                }
                let key = (
                    m.kind.to_owned(),
                    m.rule_id.clone().unwrap_or_default(),
                    m.relation.clone().unwrap_or_default(),
                    m.target.clone().unwrap_or_default(),
                );
                if seen_keys.insert(key) {
                    newly_added += 1;
                    let mut entry = json!({ "kind": m.kind });
                    if let Some(ref rid) = m.rule_id {
                        entry["rule_id"] = json!(rid);
                    }
                    if let Some(ref rel) = m.relation {
                        entry["relation"] = json!(rel);
                    }
                    if let Some(ref tgt) = m.target {
                        entry["target"] = json!(tgt);
                    }
                    existing_rules.push(entry);
                }
            }
            if newly_added > 0 {
                let prev = edge["architecture_violation"].as_u64().unwrap_or(0);
                edge["architecture_violation"] = json!(prev + newly_added);
            }
            if !rule_ids.is_empty() {
                let ids: Vec<String> = rule_ids.into_iter().collect();
                edge["rule_ids"] = json!(ids);
            }
            if !existing_rules.is_empty() {
                existing_rules.sort_by(|a, b| {
                    a["kind"]
                        .as_str()
                        .cmp(&b["kind"].as_str())
                        .then_with(|| a["rule_id"].as_str().cmp(&b["rule_id"].as_str()))
                        .then_with(|| a["relation"].as_str().cmp(&b["relation"].as_str()))
                        .then_with(|| a["target"].as_str().cmp(&b["target"].as_str()))
                });
                edge["architecture_rules"] = json!(existing_rules);
            }
        };

        let mut present_edges: HashSet<(String, String)> = HashSet::new();
        if let Some(edges) = view["edges"].as_array_mut() {
            for edge in edges {
                let from_raw = edge["from"].as_str().unwrap_or("").to_owned();
                let to_raw = edge["to"].as_str().unwrap_or("").to_owned();
                present_edges.insert((from_raw.clone(), to_raw.clone()));
                let from_key = if symbols_mode {
                    file_of_sym.get(&from_raw).cloned().unwrap_or_else(|| {
                        from_raw
                            .split_once('#')
                            .map_or(from_raw.clone(), |(f, _)| f.to_owned())
                    })
                } else {
                    from_raw.clone()
                };
                let to_key = if symbols_mode {
                    file_of_sym.get(&to_raw).cloned().unwrap_or_else(|| {
                        to_raw
                            .split_once('#')
                            .map_or(to_raw.clone(), |(f, _)| f.to_owned())
                    })
                } else {
                    to_raw.clone()
                };

                let mut matches = Vec::new();
                if let Some(pkg_hits) = pkg_by_boxes.get(&(from_key.clone(), to_key)) {
                    matches.extend(pkg_hits.iter().cloned());
                }
                if let Some(exact_sem) = edge_semantic_target(&to_raw) {
                    if let Some(sem_hits) = sem_by_exact_target.get(&(from_key.clone(), exact_sem))
                    {
                        matches.extend(sem_hits.iter().cloned());
                    }
                } else if !symbols_mode
                    && to_raw.starts_with("__semantic__/")
                    && let Some(sem_hits) = sem_by_bucket.get(&(from_key, to_raw))
                {
                    matches.extend(sem_hits.iter().cloned());
                }
                apply_matches_to_edge(edge, &matches);
            }
        }

        // Preserve package/semantic breaching edges past DRAWN truncation within a dedicated
        // KEPT_VIOLATIONS budget so rolled-up Crate/Folder/File views never silently drop them.
        let mut extended_kept = 0usize;
        if !symbols_mode {
            let mut missing: BTreeMap<(String, String), Vec<MatchedEdgeRule>> = BTreeMap::new();
            for ((fb, tb), hits) in &pkg_by_boxes {
                if fb != tb
                    && drawn_nodes.contains(fb.as_str())
                    && drawn_nodes.contains(tb.as_str())
                    && !present_edges.contains(&(fb.clone(), tb.clone()))
                {
                    missing
                        .entry((fb.clone(), tb.clone()))
                        .or_default()
                        .extend(hits.iter().cloned());
                }
            }
            if matches!(view_lod.as_deref(), Some("crate" | "folder")) {
                for ((fb, tb), hits) in &sem_by_bucket {
                    if fb != tb
                        && drawn_nodes.contains(fb.as_str())
                        && drawn_nodes.contains(tb.as_str())
                        && !present_edges.contains(&(fb.clone(), tb.clone()))
                    {
                        missing
                            .entry((fb.clone(), tb.clone()))
                            .or_default()
                            .extend(hits.iter().cloned());
                    }
                }
            } else {
                for ((fb, clean_target), hits) in &sem_by_exact_target {
                    let candidates = [
                        format!("sem:{clean_target}"),
                        clean_target.clone(),
                        crate::graph_ir::semantic_namespace_bucket(clean_target).to_owned(),
                    ];
                    if drawn_nodes.contains(fb.as_str())
                        && let Some(tb) = candidates
                            .into_iter()
                            .find(|cand| fb != cand && drawn_nodes.contains(cand.as_str()))
                        && !present_edges.contains(&(fb.clone(), tb.clone()))
                    {
                        missing
                            .entry((fb.clone(), tb))
                            .or_default()
                            .extend(hits.iter().cloned());
                    }
                }
            }
            if !missing.is_empty() {
                let mut pair_flows: HashMap<(String, String), EdgeFlow> = HashMap::new();
                for &(from, to, ref pair) in &self.pairs {
                    if pair.weight(certain) == 0 {
                        continue;
                    }
                    let ends = (
                        box_for(&self.files[from as usize]),
                        box_for(&self.files[to as usize]),
                    );
                    if missing.contains_key(&ends) {
                        pair_flows.entry(ends).or_default().add_pair(pair, certain);
                    }
                }
                let mut rows: Vec<((String, String), Vec<MatchedEdgeRule>, EdgeFlow)> = missing
                    .into_iter()
                    .map(|(ends, hits)| {
                        let flow = pair_flows.remove(&ends).unwrap_or_default();
                        (ends, hits, flow)
                    })
                    .collect();
                rows.sort_by(|a, b| {
                    b.2.flow
                        .links
                        .cmp(&a.2.flow.links)
                        .then_with(|| b.1.len().cmp(&a.1.len()))
                        .then_with(|| a.0.cmp(&b.0))
                });
                rows.truncate(KEPT_VIOLATIONS);
                extended_kept = rows.len();
                let mut added_edges = Vec::with_capacity(rows.len());
                for ((from, to), hits, flow) in rows {
                    let eff_flow = if flow.flow.links == 0 {
                        EdgeFlow {
                            flow: Flow {
                                links: hits.len() as u64,
                                guessed: 0,
                            },
                            kinds: BTreeMap::new(),
                        }
                    } else {
                        flow
                    };
                    let mut edge = self.edge_json(&from, &to, &eff_flow);
                    edge["kept"] = json!("violation");
                    apply_matches_to_edge(&mut edge, &hits);
                    added_edges.push(edge);
                }
                if let Some(edges) = view["edges"].as_array_mut() {
                    edges.extend(added_edges);
                }
            }
        }

        if let Some(nodes) = view["nodes"].as_array_mut() {
            for node in nodes {
                let p = node["path"].as_str().unwrap_or("");
                let file_p = p.split_once('#').map_or(p, |(f, _)| f);
                if violated_boxes.contains(p) || violated_boxes.contains(file_p) {
                    node["architecture_violation"] = json!(true);
                }
            }
        }

        if !view["pattern"].is_object() {
            view["pattern"] = json!({});
        }
        if let Some(pat) = view["pattern"].as_object_mut() {
            pat.insert("package_violations".to_owned(), json!(package_v));
            pat.insert(
                "cycle_budget".to_owned(),
                json!({
                    "exceeded": !cycle_v.is_empty(),
                    "max_cycles": rules.max_cycles,
                    "violations": cycle_v,
                }),
            );
            pat.insert("semantic_violations".to_owned(), json!(semantic_v));
            pat.insert("extended_violations".to_owned(), json!(all));
            pat.insert("extended_kept".to_owned(), json!(extended_kept));
        }
    }
}

#[cfg(test)]
mod tests;
