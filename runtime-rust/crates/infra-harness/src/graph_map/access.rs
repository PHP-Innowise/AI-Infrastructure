//! What the sources say of reading and writing state.
//!
//! The graph tells which symbol calls, imports or names which. It does not
//! tell that a method reads a property of another class, or assigns to it;
//! the sources do, and `graph_deps` reads them. A picture drawn on the
//! checkout's own graph therefore adds, to each link it draws, how many
//! members of what stands at its far end the files at its near end read,
//! and how many they write. The counts refine a link the graph holds: a
//! read or a write between two files the graph does not link draws nothing.
//!
//! A source is read once for a graph and kept. One picture reads no more
//! than `ACCESS_SOURCES` sources anew, the links of the most weight first,
//! and a link is told whole or not at all: where its sources do not fit,
//! it stays as the graph gave it and the picture says how many were left.
use super::*;
use crate::{
    graph_deps::{cached_manifests_for_root, extract_file_deps_with_context},
    graph_impact::member_ref_kind,
    graph_ir::GraphIrStore,
};

/// Sources one picture reads that were not read for this graph before.
pub const ACCESS_SOURCES: usize = 160;

/// One member of another file that a file reads or writes.
#[derive(Clone, Debug)]
struct AccessRef {
    target: u32,
    /// Every line the member is reached in.
    lines: Box<[u32]>,
    write: bool,
    /// The type the member belongs to, where the source says.
    parent: Option<Box<str>>,
}

/// What the sources of a graph were found to read and write, by file.
#[derive(Debug, Default)]
pub(super) struct Access {
    read: Mutex<HashMap<u32, Arc<[AccessRef]>>>,
    /// The files of the graph, as the sources are resolved against them.
    known: OnceLock<(BTreeSet<String>, HashMap<String, u32>)>,
}

/// Reads and writes counted for one link.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Tally {
    reads: u64,
    writes: u64,
}
impl Tally {
    fn add(&mut self, write: bool) {
        if write {
            self.writes += 1;
        } else {
            self.reads += 1;
        }
    }
}

/// What a link of a picture stands for: pairs of files, and, in an opened
/// file, which of the references between them are the link's own.
enum Part {
    /// Every member the first file reaches in the second.
    Whole(u32, u32),
    /// What the lines of one symbol of the first file reach in the second.
    From(u32, u32, std::ops::Range<u32>),
    /// What the first file reaches of one type of the second.
    Into(u32, u32, String),
}
impl Part {
    fn source(&self) -> u32 {
        match self {
            Self::Whole(from, _) | Self::From(from, ..) | Self::Into(from, ..) => *from,
        }
    }
}

impl Index {
    fn known(&self) -> &(BTreeSet<String>, HashMap<String, u32>) {
        self.access.known.get_or_init(|| {
            (
                self.files.iter().cloned().collect(),
                self.files
                    .iter()
                    .enumerate()
                    .map(|(id, path)| (path.clone(), id as u32))
                    .collect(),
            )
        })
    }
    fn access_refs_from_deps(
        &self,
        deps: Vec<crate::graph_deps::DepRef>,
        ids: &HashMap<String, u32>,
    ) -> Arc<[AccessRef]> {
        let mut refs = Vec::new();
        for dep in deps {
            let write = match dep.kind.as_str() {
                "writes" => true,
                "reads" => false,
                _ => continue,
            };
            let Some(&target) = ids.get(&dep.target).filter(|_| dep.resolved) else {
                continue;
            };
            for member in &dep.member_refs {
                if member_ref_kind(member) == Some(dep.kind.as_str()) {
                    let lines = if member.lines.is_empty() {
                        member.line.into_iter().collect()
                    } else {
                        member.lines.clone()
                    };
                    refs.push(AccessRef {
                        target,
                        lines: lines.into(),
                        write,
                        parent: member.parent.as_deref().map(Box::from),
                    });
                }
            }
        }
        refs.into()
    }
    /// Batch-loads `AccessRef`s for multiple uncached file IDs using a single read-only
    /// `GraphIrStore` connection and SQL query, falling back to disk parsing only for
    /// missing or stale files.
    fn load_access_batch(&self, files: &[u32]) -> HashMap<u32, Arc<[AccessRef]>> {
        let mut loaded = HashMap::new();
        if files.is_empty() {
            return loaded;
        }
        let (known, ids) = self.known();
        let rel_paths: Vec<&str> = files
            .iter()
            .map(|&f| self.files[f as usize].as_str())
            .collect();
        let mut ir_deps = self
            .ir_db_path
            .as_deref()
            .and_then(|db_path| GraphIrStore::open_at_readonly(db_path).ok().flatten())
            .and_then(|ir| ir.batch_access_deps_if_fresh(&self.root, &rel_paths).ok())
            .unwrap_or_default();

        let mut manifests_cache = None;
        for &file in files {
            let path = &self.files[file as usize];
            let deps = if let Some(cached_ir) = ir_deps.remove(path) {
                cached_ir
            } else if let Some(source) = text_at(&self.root, path) {
                let (manifests, crate_map) = manifests_cache
                    .get_or_insert_with(|| cached_manifests_for_root(&self.root, known));
                extract_file_deps_with_context(
                    path,
                    &source,
                    known,
                    Some(manifests),
                    None,
                    Some(crate_map),
                )
            } else {
                Vec::new()
            };
            loaded.insert(file, self.access_refs_from_deps(deps, ids));
        }
        if let Ok(mut held) = self.access.read.lock() {
            for (&file, refs) in &loaded {
                held.insert(file, refs.clone());
            }
        }
        loaded
    }
    /// The lines of a symbol of a file: exact `[start, end_line + 1)` from IR
    /// when recorded, else falling back to `[start, next_symbol)`.
    pub(super) fn lines_of(&self, at: u32) -> std::ops::Range<u32> {
        let symbol = &self.symbols[at as usize];
        if symbol.line > 0 && symbol.end_line >= symbol.line {
            return symbol.line..symbol.end_line.saturating_add(1);
        }
        let next = self
            .range(symbol.file)
            .filter(|other| *other != at)
            .map(|other| self.symbols[other as usize].line)
            .filter(|line| *line > symbol.line)
            .min()
            .unwrap_or(u32::MAX);
        symbol.line..next
    }
    /// Adds to the links of a picture of folders and files, of a change, or
    /// of an opened file, what the sources read and write through them:
    /// `{"kind": "reads" | "writes", "links": members, "by": "source"}`
    /// among the kinds of a link. `view["access"]` says how many links were
    /// told and how many were left for want of room.
    pub(crate) fn mark_access(&self, view: &mut Value) {
        if !matches!(view["mode"].as_str(), None | Some("changes" | "file")) {
            return;
        }
        let symbols = view["mode"] == "file";
        let (_, ids) = self.known();
        // What each box stands for.
        let nodes = view["nodes"].as_array().cloned().unwrap_or_default();
        let mut file_boxes: HashMap<&str, usize> = HashMap::new();
        let mut folder_boxes: HashMap<&str, usize> = HashMap::new();
        let mut symbol_boxes: HashMap<usize, u32> = HashMap::new();
        for (at, node) in nodes.iter().enumerate() {
            let Some(path) = node["path"].as_str() else {
                continue;
            };
            if node["symbol"] == true {
                if let Some(Reach::Symbol(symbol)) = self.reach(path) {
                    symbol_boxes.insert(at, symbol);
                }
            } else if node["folder"] == true {
                folder_boxes.insert(path, at);
            } else if ids.contains_key(path) {
                file_boxes.insert(path, at);
            }
        }
        let place: HashMap<&str, usize> = nodes
            .iter()
            .enumerate()
            .filter_map(|(at, node)| Some((node["path"].as_str()?, at)))
            .collect();
        let Some(edges) = view["edges"].as_array() else {
            return;
        };
        let drawn: HashMap<(usize, usize), usize> = edges
            .iter()
            .enumerate()
            .filter_map(|(at, edge)| {
                let from = place.get(edge["from"].as_str()?)?;
                let to = place.get(edge["to"].as_str()?)?;
                Some(((*from, *to), at))
            })
            .collect();
        let mut parts: Vec<Vec<Part>> = (0..edges.len()).map(|_| Vec::new()).collect();
        if symbols {
            // Around an opened file: a symbol of it reaches another file,
            // or another file reaches a type of it.
            let file_at = |at: usize| {
                nodes[at]["path"]
                    .as_str()
                    .filter(|path| file_boxes.contains_key(path))
                    .and_then(|path| ids.get(path).copied())
            };
            for (&(from, to), &edge) in &drawn {
                match (symbol_boxes.get(&from), symbol_boxes.get(&to)) {
                    (Some(&symbol), None) => {
                        let own = self.symbols[symbol as usize].file;
                        if let Some(other) = file_at(to).filter(|other| *other != own) {
                            parts[edge].push(Part::From(own, other, self.lines_of(symbol)));
                        }
                    }
                    (None, Some(&symbol)) => {
                        let own = self.symbols[symbol as usize].file;
                        if let Some(other) = file_at(from).filter(|other| *other != own) {
                            parts[edge].push(Part::Into(other, own, self.label(symbol).to_owned()));
                        }
                    }
                    _ => {}
                }
            }
        } else {
            // A folder stands for every file under it that no nearer box
            // stands for.
            let box_of = |path: &str| {
                if let Some(at) = file_boxes.get(path) {
                    return Some(*at);
                }
                let mut parent = holder(path);
                while !parent.is_empty() {
                    if let Some(at) = folder_boxes.get(parent) {
                        return Some(*at);
                    }
                    parent = holder(parent);
                }
                None
            };
            let boxes = self
                .files
                .iter()
                .map(|path| box_of(path))
                .collect::<Vec<_>>();
            for (from, to, _) in &self.pairs {
                let ends = boxes[*from as usize].zip(boxes[*to as usize]);
                if let Some(edge) = ends.and_then(|ends| drawn.get(&ends)) {
                    parts[*edge].push(Part::Whole(*from, *to));
                }
            }
        }
        // The links in the order the picture gives them, the strongest
        // first; each is told whole, or left where its sources do not fit.
        let mut room = ACCESS_SOURCES;
        let mut tallies: Vec<Option<Tally>> = vec![None; parts.len()];
        let (mut told, mut left, mut read) = (0u64, 0u64, 0u64);
        let mut known_sources: HashMap<u32, Arc<[AccessRef]>> = self
            .access
            .read
            .lock()
            .ok()
            .map(|held| held.iter().map(|(&k, v)| (k, v.clone())).collect())
            .unwrap_or_default();
        let mut planned_fresh: HashSet<u32> = HashSet::new();
        let mut admitted = vec![false; parts.len()];
        let mut files_to_load = Vec::new();
        for (edge, edge_parts) in parts.iter().enumerate() {
            if edge_parts.is_empty() {
                continue;
            }
            let fresh = edge_parts
                .iter()
                .map(Part::source)
                .filter(|file| !known_sources.contains_key(file) && !planned_fresh.contains(file))
                .collect::<BTreeSet<_>>();
            if fresh.len() > room {
                left += 1;
                continue;
            }
            room -= fresh.len();
            read += fresh.len() as u64;
            admitted[edge] = true;
            for file in fresh {
                planned_fresh.insert(file);
                files_to_load.push(file);
            }
        }
        if !files_to_load.is_empty() {
            known_sources.extend(self.load_access_batch(&files_to_load));
        }
        for (edge, edge_parts) in parts.iter().enumerate() {
            if !admitted[edge] {
                continue;
            }
            let mut tally = Tally::default();
            for part in edge_parts {
                let Some(refs) = known_sources.get(&part.source()) else {
                    continue;
                };
                for access in refs.iter() {
                    let own = match part {
                        Part::Whole(_, to) => access.target == *to,
                        Part::From(_, to, lines) => {
                            access.target == *to
                                && access.lines.iter().any(|line| lines.contains(line))
                        }
                        Part::Into(_, to, name) => {
                            access.target == *to && access.parent.as_deref() == Some(name.as_str())
                        }
                    };
                    if own {
                        tally.add(access.write);
                    }
                }
            }
            told += 1;
            tallies[edge] = Some(tally);
        }
        let Some(edges) = view["edges"].as_array_mut() else {
            return;
        };
        for (edge, tally) in edges.iter_mut().zip(tallies) {
            let Some(tally) = tally.filter(|tally| *tally != Tally::default()) else {
                continue;
            };
            if !edge["kinds"].is_array() {
                edge["kinds"] = json!([]);
            }
            let kinds = edge["kinds"].as_array_mut().expect("an array of kinds");
            for (kind, members) in [("writes", tally.writes), ("reads", tally.reads)] {
                if members > 0 {
                    kinds.push(json!({"kind": kind, "links": members, "by": "source"}));
                }
            }
        }
        view["access"] = json!({"links": told, "left": left, "sources": read});
    }
}

#[cfg(test)]
mod tests;
