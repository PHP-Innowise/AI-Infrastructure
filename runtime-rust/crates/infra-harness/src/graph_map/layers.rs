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
use super::*;

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
/// Files of these kinds are code that can be filed in a layer.
const CODE: &[&str] = &[
    "php", "py", "rb", "java", "kt", "cs", "ts", "tsx", "js", "jsx", "go", "rs", "swift", "scala",
];
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
/// Whether a file is code a layer can hold: no test, and no template that
/// only wears the ending of a language, as `home.blade.php` does.
fn code(path: &str) -> bool {
    let file = name(path);
    !is_test(path)
        && file.rsplit_once('.').is_some_and(|(stem, kind)| {
            CODE.iter().any(|code| kind.eq_ignore_ascii_case(code))
                && !stem.to_lowercase().ends_with(".blade")
        })
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
    cross: Vec<(u32, u32, Flow)>,
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
    files: [u64; 4],
    /// Code under the folder that no layer holds.
    apart: u64,
    /// Through how many links each layer needs each other one.
    needs: [[Flow; 4]; 4],
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
        let needing = ["calls", "inherits", "extends", "implements"]
            .iter()
            .filter_map(|name| index.kinds.iter().position(|kind| kind == name))
            .collect::<Vec<_>>();
        let mut cross: HashMap<(u32, u32), Flow> = HashMap::new();
        let links = index
            .uses
            .iter()
            .filter(|(.., kind, _)| needing.contains(&(*kind as usize)))
            .chain(index.ties.iter());
        for (from, to, _, guessed) in links {
            if certain && *guessed {
                continue;
            }
            let (from, to) = (
                index.symbols[*from as usize].file,
                index.symbols[*to as usize].file,
            );
            let (above, below) = (of[from as usize], of[to as usize]);
            if from == to || above == NO || below == NO || above == below {
                continue;
            }
            let flow = cross.entry((from, to)).or_default();
            flow.links += 1;
            flow.guessed += u64::from(*guessed);
        }
        let mut cross = cross
            .into_iter()
            .map(|((from, to), flow)| (from, to, flow))
            .collect::<Vec<_>>();
        cross.sort_by(|a, b| {
            b.2.links
                .cmp(&a.2.links)
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
        let Way { of, cross } = &self.ways[way];
        let mut held = Held {
            way,
            files: [0; 4],
            apart: 0,
            needs: [[Flow::default(); 4]; 4],
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
        for (from, to, flow) in cross {
            if under(scope, &index.files[*from as usize])
                && under(scope, &index.files[*to as usize])
            {
                let needs = &mut held.needs[of[*from as usize] as usize][of[*to as usize] as usize];
                needs.links += flow.links;
                needs.guessed += flow.guessed;
                held.pairs.push((*from, *to, *flow));
            }
        }
        let filed = held.files.iter().sum::<u64>();
        (!held.pairs.is_empty() && filed >= FEWEST && filed * SHARES[way] >= filed + held.apart)
            .then_some(held)
    }
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
        let (table, of) = (WAYS[held.way], &self.ways[held.way].of);
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
                "out": sum(&mut held.needs[layer].iter().copied()).links,
                "in": sum(&mut held.needs.iter().map(|row| row[layer])).links,
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
        let back = |from: u32, to: u32| of[from as usize] > of[to as usize];
        let against = held
            .pairs
            .iter()
            .filter(|(from, to, _)| back(*from, *to))
            .collect::<Vec<_>>();
        let total = held.against();
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
                "tells": TELLS[held.way],
                "rings": held.way == RINGED,
                "named": true,
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
            },
            "nodes": present.iter().map(|layer| node(*layer)).collect::<Vec<_>>(),
            "edges": present
                .iter()
                .flat_map(|from| present.iter().map(move |to| (*from, *to)))
                .filter(|(from, to)| held.needs[*from][*to].links > 0)
                .map(|(from, to)| {
                    let flow = held.needs[from][to];
                    json!({
                        "from": key(from), "to": key(to),
                        "links": flow.links, "guessed": flow.guessed,
                        "against": from > to,
                    })
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
                && held.needs[from][to].links > 0
            {
                let flow = held.needs[from][to];
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
                    "against": from > to,
                    "pairs": made.len(),
                    "rows": made
                        .iter()
                        .take(LISTED)
                        .map(|(user, used, flow)| pair(*user, *used, *flow))
                        .collect::<Vec<_>>(),
                    "more": made.len().saturating_sub(LISTED),
                });
            }
            return Some(view);
        }
        let Some(chosen) = focus.and_then(layer) else {
            return Some(view);
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
        focus["signs"] = json!(signs(held.way, chosen));
        focus["uses"] = present
            .iter()
            .filter(|other| held.needs[chosen][**other].links > 0)
            .map(|other| beside(*other, held.needs[chosen][*other], chosen > *other))
            .collect();
        focus["used_by"] = present
            .iter()
            .filter(|other| held.needs[**other][chosen].links > 0)
            .map(|other| beside(*other, held.needs[*other][chosen], *other > chosen))
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
        Some(view)
    }
}
impl Held {
    /// Through how many links a layer needs one before it.
    fn against(&self) -> Flow {
        let mut sum = Flow::default();
        for (from, row) in self.needs.iter().enumerate() {
            for flow in &row[..from] {
                sum.links += flow.links;
                sum.guessed += flow.guessed;
            }
        }
        sum
    }
}
#[cfg(test)]
mod tests;
