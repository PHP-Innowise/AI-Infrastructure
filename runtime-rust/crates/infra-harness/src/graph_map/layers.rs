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
            "valueobjects",
            "events",
            "event",
        ],
    },
];
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
/// Pairs of files against the order, and files of one layer, that are
/// listed at once: the most linked first.
pub const LISTED: usize = 30;
/// A folder is said to be in layers where they hold no fewer files than
/// this, and no less than one in this many of the files of its code: a few
/// files that happen to be named like a layer are no layered design.
const FEWEST: u64 = 4;
const SHARE: u64 = 10;
/// No layer.
const NO: u8 = u8::MAX;
pub(super) const TELLS: &str = "Files filed as entry points, services, data access and model by their names and folders. A layer may use those after it; a link back runs against their order. Counted: calls, inheritance and the types of fields, parameters and results.";
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
/// whole, or by the nearest folder named for a layer. A test is filed in
/// none, nor is a file that is no code.
pub(super) fn layer(path: &str) -> Option<usize> {
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
    // An ending is a word of the name: `OrderService`, `order_service`,
    // `orders.service`, and not `Remodel` for `model`.
    let ends = |end: &str| {
        let Some(at) = stem.len().checked_sub(end.len()) else {
            return false;
        };
        stem.get(at..).is_some_and(|tail| {
            tail.eq_ignore_ascii_case(end)
                && (at == 0
                    || stem[..at].ends_with(['.', '_', '-'])
                    || tail.starts_with(|first: char| first.is_uppercase()))
        })
    };
    let whole = !FOLDERS_ONLY.contains(&low.as_str());
    if let Some(found) = LAYERS.iter().position(|layer| {
        layer.ends.iter().any(|end| ends(end)) || (whole && layer.folders.contains(&low.as_str()))
    }) {
        return Some(found);
    }
    folders.rsplit('/').find_map(|folder| {
        let folder = folder.to_lowercase();
        LAYERS
            .iter()
            .position(|layer| layer.folders.contains(&folder.as_str()))
    })
}
/// What places a file in a layer, in words.
fn signs(layer: &Layer) -> String {
    format!(
        "a name that ends in {}, or a folder named {}",
        layer.ends.join(", "),
        layer.folders.join(", ")
    )
}
/// The layers of a graph: the layer of each file, and every pair of files of
/// two layers of which the first needs the second, with through how many
/// links.
#[derive(Default)]
pub(super) struct Layers {
    of: Vec<u8>,
    cross: Vec<(u32, u32, Flow)>,
}
impl fmt::Debug for Layers {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(
            formatter,
            "layers of {} files, {} pairs across them",
            self.of.len(),
            self.cross.len()
        )
    }
}
/// The layers of one folder.
struct Held {
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
        let of = index
            .files
            .iter()
            .map(|path| layer(path).map_or(NO, |layer| layer as u8))
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
    /// The layers of a folder: `None` where no file of one layer needs a
    /// file of another, which takes two layers, or the layers hold too
    /// little of the folder to say it is built in them. So it is for what
    /// is no folder: nothing lies under it.
    fn held(&self, index: &Index, scope: &str) -> Option<Held> {
        let mut held = Held {
            files: [0; 4],
            apart: 0,
            needs: [[Flow::default(); 4]; 4],
            pairs: Vec::new(),
        };
        for (file, path) in index.files.iter().enumerate() {
            if !under(scope, path) {
                continue;
            }
            match self.of[file] {
                NO => held.apart += u64::from(code(path)),
                layer => held.files[layer as usize] += 1,
            }
        }
        for (from, to, flow) in &self.cross {
            if under(scope, &index.files[*from as usize])
                && under(scope, &index.files[*to as usize])
            {
                let needs = &mut held.needs[self.of[*from as usize] as usize]
                    [self.of[*to as usize] as usize];
                needs.links += flow.links;
                needs.guessed += flow.guessed;
                held.pairs.push((*from, *to, *flow));
            }
        }
        let filed = held.files.iter().sum::<u64>();
        (!held.pairs.is_empty() && filed >= FEWEST && filed * SHARE >= filed + held.apart)
            .then_some(held)
    }
    /// The layers of the open folder as a row of the list of found
    /// patterns.
    pub(super) fn row(&self, index: &Index, scope: &str) -> Option<Value> {
        let held = self.held(index, scope)?;
        let against = held.against().links;
        Some(json!({
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
        }))
    }
    /// The picture of the layers of a folder: each layer that holds files
    /// as one box, and between them through how many links the files of the
    /// one need those of the other. A selected layer lists its files and
    /// what it has with the other layers. `None` where the folder has no
    /// layers, as after the graph was built anew.
    pub(super) fn view(
        &self,
        index: &Index,
        project: &str,
        scope: &str,
        focus: Option<&str>,
        certain: bool,
    ) -> Option<Value> {
        let held = self.held(index, scope)?;
        let key = |layer: usize| format!("layer:{}", LAYERS[layer].id);
        let present = (0..LAYERS.len())
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
                "path": key(layer), "name": LAYERS[layer].name, "folder": true, "layer": true,
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
                    "layer": LAYERS[self.of[file as usize] as usize].name,
                })
            };
            json!({"from": end(from), "to": end(to), "links": flow.links, "guessed": flow.guessed})
        };
        let back = |from: u32, to: u32| self.of[from as usize] > self.of[to as usize];
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
                "tells": TELLS,
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
                        "id": LAYERS[*layer].id, "name": LAYERS[*layer].name,
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
        let Some(chosen) =
            focus.and_then(|focus| present.iter().find(|layer| key(**layer) == focus))
        else {
            return Some(view);
        };
        let chosen = *chosen;
        // The files of the layer, by how much they have with other layers.
        let mut members: HashMap<u32, Flow> = HashMap::new();
        for (from, to, flow) in &held.pairs {
            for file in [from, to] {
                if self.of[*file as usize] as usize == chosen {
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
            .filter(|(file, path)| self.of[*file] as usize == chosen && under(scope, path))
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
                "path": key(other), "name": LAYERS[other].name, "folder": false, "inside": true,
                "links": flow.links, "guessed": flow.guessed, "changed": false,
                "against": back,
            })
        };
        let mut focus = node(chosen);
        focus["inside"] = json!(true);
        focus["signs"] = json!(signs(&LAYERS[chosen]));
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
            .filter(|(from, ..)| self.of[*from as usize] as usize == chosen)
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
