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
use crate::{graph_deps::extract_file_deps, graph_impact::member_ref_kind};

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
    fn access_known(&self, file: u32) -> Option<Arc<[AccessRef]>> {
        self.access
            .read
            .lock()
            .ok()
            .and_then(|held| held.get(&file).cloned())
    }
    /// What a file reads and writes of the other files of the graph, from
    /// its source as it lies in the tree. A source that cannot be read, or
    /// is of no language that is read, reaches nothing.
    fn access_of(&self, file: u32) -> Arc<[AccessRef]> {
        if let Some(found) = self.access_known(file) {
            return found;
        }
        let path = &self.files[file as usize];
        let (known, ids) = self.known();
        let mut refs = Vec::new();
        if let Some(source) = text_at(&self.root, path) {
            for dep in extract_file_deps(path, &source, known) {
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
        }
        let refs: Arc<[AccessRef]> = refs.into();
        if let Ok(mut held) = self.access.read.lock() {
            held.insert(file, refs.clone());
        }
        refs
    }
    /// The lines of a symbol of a file as far as the graph tells them: from
    /// where it starts to where the next symbol of the file starts. A type
    /// therefore holds the lines before its first method, and each method
    /// its own.
    fn lines_of(&self, at: u32) -> std::ops::Range<u32> {
        let symbol = &self.symbols[at as usize];
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
        for (edge, parts) in parts.iter().enumerate() {
            if parts.is_empty() {
                continue;
            }
            let fresh = parts
                .iter()
                .map(Part::source)
                .filter(|file| self.access_known(*file).is_none())
                .collect::<BTreeSet<_>>();
            if fresh.len() > room {
                left += 1;
                continue;
            }
            room -= fresh.len();
            read += fresh.len() as u64;
            let mut tally = Tally::default();
            for part in parts {
                let refs = self.access_of(part.source());
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
