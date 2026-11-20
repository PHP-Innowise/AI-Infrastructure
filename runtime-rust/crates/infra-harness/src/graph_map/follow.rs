use super::*;

/// A place among followed links: a file, or a folder standing for the files
/// in it.
#[derive(Default)]
pub(super) struct Stop {
    pub(super) files: u64,
    pub(super) folder: bool,
    /// What a file holds: classes, methods, functions.
    pub(super) symbols: u64,
    pub(super) out: u64,
    pub(super) into: u64,
    /// Files of a chosen change in it, and Git's letter where it is one file.
    pub(super) changed: u64,
    pub(super) status: Option<char>,
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
    /// `follow` with caller-provided edit flags and optional multi-hop `depth` + paging (`offset`, `limit`).
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
        self.follow_with_depth(
            project, root, out, into, changed, edited, focus, certain, None, 0, None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn follow_with_depth(
        &self,
        project: &str,
        root: &str,
        out: &[String],
        into: &[String],
        changed: &[(String, char)],
        edited: &dyn Fn(&str) -> Option<Edited>,
        focus: Option<&str>,
        certain: bool,
        depth_limit: Option<usize>,
        offset: usize,
        limit: Option<usize>,
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
        let mut flows: HashMap<(&str, &str), EdgeFlow> = HashMap::new();
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
            flows
                .entry((source, target))
                .or_default()
                .add_pair(pair, certain);
            if focus == Some(source) {
                add_kind_counts(&mut kinds, pair.kinds_for(certain));
            }
        }
        // Both sides of every place, strongest first.
        let mut uses: HashMap<&str, Vec<(&str, Flow)>> = HashMap::new();
        let mut users: HashMap<&str, Vec<(&str, Flow)>> = HashMap::new();
        for ((source, target), edge) in &flows {
            uses.entry(*source).or_default().push((*target, edge.flow));
            users.entry(*target).or_default().push((*source, edge.flow));
        }
        for rows in uses.values_mut().chain(users.values_mut()) {
            rows.sort_by(|(a, left), (b, right)| right.links.cmp(&left.links).then(a.cmp(b)));
        }
        let root = stops.get_key_value(root).map(|(place, _)| *place)?;
        let times = |list: &[String], place: &str| {
            list.iter().filter(|named| named.as_str() == place).count()
        };
        let bfs_depth_cap = depth_limit.unwrap_or(0);
        // Places join in the order they are reached, the followed one first.
        let mut shown = vec![root];
        let mut hops: HashMap<&str, usize> = HashMap::from([(root, 0)]);
        let mut seen = HashSet::from([root]);
        let mut full = false;
        let mut at = 0;
        while at < shown.len() {
            let place = shown[at];
            let hop = hops.get(place).copied().unwrap_or(0);
            at += 1;
            for (list, side) in [(out, &uses), (into, &users)] {
                let explicit = times(list, place);
                let mut left = if explicit > 0 {
                    STEP * explicit
                } else if hop < bfs_depth_cap {
                    STEP
                } else {
                    0
                };
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
                    hops.insert(*other, hop + 1);
                    shown.push(*other);
                    left -= 1;
                }
            }
        }
        let total_shown = shown.len();
        let all_seen: HashSet<&str> = seen.clone();
        let is_paginated = offset > 0 || limit.is_some();
        if is_paginated {
            if let Some(focus_path) = focus
                && focus_path != root
                && let Some(pos) = shown.iter().position(|p| *p == focus_path)
                && pos != 1
            {
                let item = shown.remove(pos);
                shown.insert(1.min(shown.len()), item);
            }
            let cap = limit.unwrap_or(FOLLOWED).max(1);
            let start = offset.min(total_shown);
            let end = start.saturating_add(cap).min(total_shown);
            shown = shown[start..end].to_vec();
            seen = shown.iter().copied().collect();
        }
        // A link between two places drawn is drawn where one of its sides
        // is unfolded: the uses of its user, or the users of what it uses.
        let linked_in = |scope_set: &HashSet<&str>, source: &str, target: &str| {
            scope_set.contains(source)
                && scope_set.contains(target)
                && (times(out, source) > 0 || times(into, target) > 0 || bfs_depth_cap > 0)
        };
        let among = flows
            .keys()
            .filter(|(source, target)| all_seen.contains(source) && all_seen.contains(target))
            .count();
        let cross_page_edges: Vec<Value> = if is_paginated {
            let mut cross: Vec<(&str, &str, u64)> = flows
                .iter()
                .filter_map(|((source, target), edge)| {
                    let s_in = seen.contains(source);
                    let t_in = seen.contains(target);
                    (s_in != t_in && linked_in(&all_seen, source, target)).then_some((
                        *source,
                        *target,
                        edge.flow.links,
                    ))
                })
                .collect();
            cross.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(b.0)).then(a.1.cmp(b.1)));
            cross
                .into_iter()
                .take(64)
                .map(|(from, to, links)| json!({"from": from, "to": to, "links": links}))
                .collect()
        } else {
            Vec::new()
        };
        let mut edges = flows
            .iter()
            .filter(|((source, target), _)| linked_in(&seen, source, target))
            .collect::<Vec<_>>();
        let between = edges.len();
        edges.sort_by(|((a, b), left), ((c, d), right)| {
            right
                .flow
                .links
                .cmp(&left.flow.links)
                .then((a, b).cmp(&(c, d)))
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
                            !linked_in(&all_seen, place, other)
                        } else {
                            !linked_in(&all_seen, other, place)
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
        let focus = focus.filter(|path| all_seen.contains(path)).map(|path| {
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
                            "inside": all_seen.contains(other),
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
            let mut focus = node(path);
            focus["name"] = json!(name(path));
            focus["inside"] = json!(true);
            focus["uses"] = json!(using);
            focus["uses_more"] = json!(using_more);
            focus["used_by"] = json!(used_by);
            focus["used_by_more"] = json!(used_by_more);
            focus["kinds"] = json!(self.kinds_json(&kinds));
            focus
        });
        let has_more = !shown.is_empty() && offset.saturating_add(shown.len()) < total_shown;
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
            "children": total_shown,
            "matching": total_shown,
            "nodes": shown.iter().map(|place| node(place)).collect::<Vec<_>>(),
            "between": between,
            "backbone": false,
            "edges": edges.iter().map(|((source, target), edge)| {
                self.edge_json(source, target, edge)
            }).collect::<Vec<_>>(),
            "focus": focus,
            "paging": {
                "offset": offset,
                "limit": limit,
                "total": total_shown,
                "returned": shown.len(),
                "has_more": has_more,
                "next_offset": has_more.then_some(offset.saturating_add(shown.len())),
                "cross_page_edges": cross_page_edges,
            },
            "depth": depth_limit,
        }))
    }
}
/// What a picture of symbols from many files is drawn with: the names the
/// pictures of their files select them by, and what a chosen change did to
/// each. Both are worked out file by file as they are asked for, each once.
pub(super) struct Names<'a> {
    pub(super) index: &'a Index,
    /// The files of the change, with Git's letter for each.
    pub(super) letters: HashMap<&'a str, char>,
    /// What the change did to one of its files, where that can be told.
    pub(super) edited: &'a dyn Fn(&str) -> Option<Edited>,
    pub(super) keys: RefCell<HashMap<u32, Rc<Vec<String>>>>,
    pub(super) marks: RefCell<HashMap<u32, Option<Rc<Marks>>>>,
}
impl<'a> Names<'a> {
    pub(super) fn new(
        index: &'a Index,
        changed: &'a [(String, char)],
        edited: &'a dyn Fn(&str) -> Option<Edited>,
    ) -> Self {
        Self {
            index,
            letters: changed
                .iter()
                .filter(|(_, kind)| *kind != 'D')
                .map(|(path, kind)| (path.as_str(), *kind))
                .collect(),
            edited,
            keys: RefCell::new(HashMap::new()),
            marks: RefCell::new(HashMap::new()),
        }
    }
    /// The name the picture of its file selects a symbol by.
    pub(super) fn key(&self, at: u32) -> String {
        let index = self.index;
        let file = index.symbols[at as usize].file;
        let keys = self
            .keys
            .borrow_mut()
            .entry(file)
            .or_insert_with(|| Rc::new(index.keys(file)))
            .clone();
        keys[(at - index.first[file as usize]) as usize].clone()
    }
    /// What the change did to a symbol: Git's letter where its own lines
    /// changed, and how many symbols changed inside it.
    pub(super) fn touched(&self, at: u32) -> (Option<char>, u64) {
        let index = self.index;
        let file = index.symbols[at as usize].file;
        let marks = self
            .marks
            .borrow_mut()
            .entry(file)
            .or_insert_with(|| {
                let path = index.files[file as usize].as_str();
                self.letters
                    .contains_key(path)
                    .then(|| (self.edited)(path))
                    .flatten()
                    .map(|edited| Rc::new(index.marks(file, &edited)))
            })
            .clone();
        let place = (at - index.first[file as usize]) as usize;
        (
            marks.as_ref().and_then(|marks| marks.status[place]),
            marks.as_ref().map_or(0, |marks| marks.inside[place]),
        )
    }
    /// A symbol as a picture tells it: by the name its file selects it by,
    /// with what it is and where it lies.
    pub(super) fn told(&self, at: u32, key: &str) -> Value {
        let index = self.index;
        let symbol = &index.symbols[at as usize];
        json!({
            "path": key,
            "name": index.shown(at),
            "kind": symbol.kind.word(),
            "line": symbol.line,
            "owner": index.owner(at),
            "file": index.files[symbol.file as usize],
            "symbol": true,
            "folder": false,
        })
    }
    /// A symbol as a box of a picture, with what the change did to it.
    pub(super) fn node(&self, at: u32, key: &str) -> Value {
        let (status, inside) = self.touched(at);
        let mut node = self.told(at, key);
        node["files"] = json!(1);
        node["within"] = json!(0);
        node["changed"] = json!(u64::from(status.is_some() || inside > 0));
        node["status"] = json!(status.map(String::from));
        node["touched"] = json!(inside);
        node
    }
    /// Git's letter for a file of the change.
    pub(super) fn letter(&self, path: &str) -> Option<char> {
        self.letters.get(path).copied()
    }
    /// A symbol as a row beside a selected one: `drawn` says the picture
    /// draws it, and `flow` and `kind` how the two are linked.
    pub(super) fn row(&self, at: u32, drawn: bool, flow: Flow, kind: Option<u16>) -> Value {
        let (status, inside) = self.touched(at);
        let mut row = self.told(at, &self.key(at));
        row["inside"] = json!(drawn);
        row["relation"] = json!(kind.map(|kind| self.index.kinds[kind as usize].as_str()));
        row["links"] = json!(flow.links);
        row["guessed"] = json!(flow.guessed);
        row["changed"] = json!(status.is_some() || inside > 0);
        row
    }
}
/// One side of a symbol: each symbol it uses, or each that uses it, with
/// the links between the two and the relation of the first of them.
pub(super) type Side = Rc<Vec<(u32, Flow, u16)>>;
impl Index {
    /// The links of the graph by the symbol at each end.
    pub(super) fn wires(&self) -> &Wires {
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
    pub(super) fn wired(
        &self,
        at: u32,
        outgoing: bool,
        certain: bool,
    ) -> impl Iterator<Item = &Use> {
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
    pub(super) fn beside(&self, at: u32, outgoing: bool, certain: bool) -> Vec<(u32, Flow, u16)> {
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
    pub(super) fn trace(
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
        let names = Names::new(self, changed, edited);
        let key = |at: u32| names.key(at);
        // The sides of a symbol are worked out as they are asked for, once.
        let sides = RefCell::new(HashMap::<(u32, bool), Side>::new());
        let side = |at: u32, outgoing: bool| {
            sides
                .borrow_mut()
                .entry((at, outgoing))
                .or_insert_with(|| Rc::new(self.beside(at, outgoing, certain)))
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
        let node = |index: usize| {
            let at = shown[index];
            let mut node = names.node(at, &keys[index]);
            for (key, value) in [
                ("out", json!(links(at, true))),
                ("in", json!(links(at, false))),
                ("start", json!(index == 0)),
                ("more_out", json!(hidden(index, true))),
                ("more_in", json!(hidden(index, false))),
                ("unfolded_out", json!(unfolded_out[index])),
                ("unfolded_in", json!(unfolded_in[index])),
            ] {
                node[key] = value;
            }
            node
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
                            let relation = (flow.links == 1).then_some(*kind);
                            names.row(*other, seen.contains(other), *flow, relation)
                        })
                        .collect::<Vec<_>>();
                    (rows, all.len().saturating_sub(TOLD))
                };
                let (uses, uses_more) = rows(true);
                let (used_by, used_by_more) = rows(false);
                let mut kinds: BTreeMap<u16, u64> = BTreeMap::new();
                self.collect_symbol_kinds(at, certain, &mut kinds, |_| true);
                let mut focus = node(index);
                focus["inside"] = json!(true);
                focus["uses"] = json!(uses);
                focus["uses_more"] = json!(uses_more);
                focus["used_by"] = json!(used_by);
                focus["used_by_more"] = json!(used_by_more);
                focus["kinds"] = json!(self.kinds_json(&kinds));
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
            "edges": edges.iter().map(|(source, target, flow)| {
                let mut kinds = BTreeMap::new();
                self.collect_symbol_kinds(shown[*source], certain, &mut kinds, |other| {
                    other == shown[*target]
                });
                self.edge_json(
                    &keys[*source],
                    &keys[*target],
                    &EdgeFlow { flow: *flow, kinds },
                )
            }).collect::<Vec<_>>(),
            "focus": focus,
        })
    }
}
/// What the ways between two places are worked out among: the files of a
/// graph and the pairs of them that are linked, or, where one of the two is
/// a symbol, its symbols and the links between them.
pub(super) struct Net<'a> {
    pub(super) index: &'a Index,
    pub(super) symbols: bool,
    pub(super) certain: bool,
    /// Between files: what each uses and what uses each, with the links
    /// between the two.
    pub(super) uses: HashMap<u32, Vec<(u32, Flow)>>,
    pub(super) users: HashMap<u32, Vec<(u32, Flow)>>,
}
impl<'a> Net<'a> {
    /// `symbols` says the places are symbols, and `certain` leaves out what
    /// the tool inferred.
    pub(super) fn new(index: &'a Index, symbols: bool, certain: bool) -> Self {
        let mut uses: HashMap<u32, Vec<(u32, Flow)>> = HashMap::new();
        let mut users: HashMap<u32, Vec<(u32, Flow)>> = HashMap::new();
        if !symbols {
            // Between files a link is a pair of files that are linked.
            for (user, used, pair) in &index.pairs {
                let links = pair.weight(certain);
                if links == 0 {
                    continue;
                }
                let flow = Flow {
                    links,
                    guessed: if certain { 0 } else { pair.guessed },
                };
                uses.entry(*user).or_default().push((*used, flow));
                users.entry(*used).or_default().push((*user, flow));
            }
        }
        Self {
            index,
            symbols,
            certain,
            uses,
            users,
        }
    }
    /// How many places there are.
    pub(super) fn count(&self) -> usize {
        if self.symbols {
            self.index.symbols.len()
        } else {
            self.index.files.len()
        }
    }
    /// What an end stands for: files, or, among symbols, symbols. A folder
    /// is every file under it, and among symbols a file or a folder is every
    /// symbol in it.
    pub(super) fn members(&self, reach: Reach) -> Vec<u32> {
        let index = self.index;
        let files = index
            .files
            .iter()
            .enumerate()
            .filter(|(id, file)| match reach {
                Reach::Folder(folder) => file
                    .strip_prefix(folder)
                    .is_some_and(|rest| rest.starts_with('/')),
                Reach::File(own) => *id as u32 == own,
                Reach::Symbol(at) => *id as u32 == index.symbols[at as usize].file,
            })
            .map(|(id, _)| id as u32);
        match (self.symbols, reach) {
            (true, Reach::Symbol(at)) => vec![at],
            (true, _) => files
                .flat_map(|file| index.range(file))
                .filter(|at| index.symbols[*at as usize].kind != Kind::Note)
                .collect(),
            (false, _) => files.collect(),
        }
    }
    /// What a place uses, or what uses it, with the links between the two.
    pub(super) fn beside(&self, place: u32, outgoing: bool) -> Vec<(u32, Flow)> {
        if self.symbols {
            self.index
                .beside(place, outgoing, self.certain)
                .into_iter()
                .map(|(other, flow, _)| (other, flow))
                .collect()
        } else {
            let side = if outgoing { &self.uses } else { &self.users };
            side.get(&place).cloned().unwrap_or_default()
        }
    }
    /// Names to `reach` what a place uses, or what uses it: every one, and
    /// one more than once where more than one link joins the two.
    pub(super) fn each(&self, place: u32, outgoing: bool, reach: &mut dyn FnMut(u32)) {
        if self.symbols {
            for (user, used, ..) in self.index.wired(place, outgoing, self.certain) {
                let other = if outgoing { *used } else { *user };
                // A comment the tool kept is no place.
                if self.index.symbols[other as usize].kind != Kind::Note {
                    reach(other);
                }
            }
        } else {
            let side = if outgoing { &self.uses } else { &self.users };
            for (other, _) in side.get(&place).into_iter().flatten() {
                reach(*other);
            }
        }
    }
    /// How many links every place lies from the nearest of `first`, along
    /// the links or, where `outgoing` is not set, against them; `u32::MAX`
    /// where nothing leads. A place that has an `owner` is not walked into,
    /// so no way passes through one, and none of them is told how far it is.
    pub(super) fn far(&self, first: &[u32], owner: &[u8], outgoing: bool) -> Vec<u32> {
        let mut far = vec![u32::MAX; self.count()];
        let mut front = first.to_vec();
        let mut steps = 0u32;
        while !front.is_empty() {
            steps += 1;
            let mut next = Vec::new();
            for place in &front {
                self.each(*place, outgoing, &mut |other| {
                    if owner[other as usize] == 0 && far[other as usize] == u32::MAX {
                        far[other as usize] = steps;
                        next.push(other);
                    }
                });
            }
            front = next;
        }
        far
    }
    /// Ways from `origin` to `goal` that are longer than the `steps` of the
    /// shortest, by `longer` links at most. Each is a chain of places in
    /// which none comes twice, none but the first is of `origin` and none
    /// but the last of `goal`: a way that passes an end is the shorter way
    /// to it and something else. The shorter ways come first, and of ways
    /// as long those along the stronger links. At most `DETOURS` are told,
    /// which together add at most `room` places to those `known`; the flag
    /// says that there are more, or that the search was given up.
    pub(super) fn detours(
        &self,
        origin: &[u32],
        goal: &[u32],
        steps: usize,
        longer: usize,
        room: usize,
        known: &HashSet<u32>,
    ) -> (Vec<Vec<u32>>, bool) {
        let (first, last) = (
            origin.iter().copied().collect::<HashSet<_>>(),
            goal.iter().copied().collect::<HashSet<_>>(),
        );
        // How far every place lies from the goal: a way is only walked on
        // where what is left of its length still reaches the goal.
        let far = self.far(goal, &vec![0; self.count()], false);
        let mut origin = origin.to_vec();
        origin.sort_by(|a, b| self.tag(*a).cmp(&self.tag(*b)));
        let mut ways: Vec<Vec<u32>> = Vec::new();
        let mut added = known.clone();
        let (mut walked, mut more) = (0usize, false);
        'longer: for length in steps + 1..=steps + longer {
            for start in &origin {
                // The way so far, and for each of its places what it uses
                // that is still to be tried.
                let mut way = vec![*start];
                let mut tries = vec![self.onward(*start)];
                while let Some(open) = tries.last_mut() {
                    let Some(next) = open.pop() else {
                        tries.pop();
                        way.pop();
                        continue;
                    };
                    walked += 1;
                    if walked > DETOUR_WALK {
                        more = true;
                        break 'longer;
                    }
                    if first.contains(&next) || way.contains(&next) {
                        continue;
                    }
                    let left = length - way.len();
                    if last.contains(&next) {
                        if left > 0 {
                            continue;
                        }
                        let fresh = way.iter().filter(|place| !added.contains(place)).count();
                        if ways.len() == DETOURS
                            || fresh > room.saturating_sub(added.len() - known.len())
                        {
                            more = true;
                            break 'longer;
                        }
                        added.extend(way.iter().copied());
                        let mut found = way.clone();
                        found.push(next);
                        ways.push(found);
                        continue;
                    }
                    if left == 0 || far[next as usize] as usize > left {
                        continue;
                    }
                    way.push(next);
                    tries.push(self.onward(next));
                }
            }
        }
        (ways, more)
    }
    /// What a place uses, the weakest link first: taken from the end, the
    /// strongest is tried first, and of links as strong the earlier place.
    pub(super) fn onward(&self, place: u32) -> Vec<u32> {
        let mut uses = self.beside(place, true);
        uses.sort_by(|a, b| {
            a.1.links
                .cmp(&b.1.links)
                .then(self.tag(b.0).cmp(&self.tag(a.0)))
        });
        uses.into_iter().map(|(other, _)| other).collect()
    }
    /// The shortest directed return to the origin. A folder stands for
    /// its files, so a return must leave the folder before re-entering it.
    pub(super) fn cycle(&self, origin: &[u32]) -> Vec<(u32, u32, Flow)> {
        let held = origin.iter().copied().collect::<HashSet<_>>();
        let mut previous = vec![None; self.count()];
        let mut front = origin.to_vec();
        while !front.is_empty() {
            let mut next = Vec::new();
            for from in front {
                let mut closing = None;
                self.each(from, true, &mut |to| {
                    if held.contains(&to) {
                        if !held.contains(&from) {
                            closing.get_or_insert(to);
                        }
                    } else if previous[to as usize].is_none() {
                        previous[to as usize] = Some(from);
                        next.push(to);
                    }
                });
                if let Some(to) = closing {
                    let mut route = vec![(from, to)];
                    let mut at = from;
                    while let Some(before) = previous[at as usize] {
                        route.push((before, at));
                        at = before;
                    }
                    route.reverse();
                    return route
                        .into_iter()
                        .map(|(from, to)| {
                            let flow = self
                                .beside(from, true)
                                .into_iter()
                                .find(|(other, _)| *other == to)
                                .expect("edge visited by cycle search")
                                .1;
                            (from, to, flow)
                        })
                        .collect();
                }
            }
            front = next;
        }
        Vec::new()
    }
    /// Places are told apart by where they lie: the file, then the line.
    pub(super) fn tag(&self, place: u32) -> (&'a str, u32, u32) {
        let index = self.index;
        if self.symbols {
            let symbol = &index.symbols[place as usize];
            (
                index.files[symbol.file as usize].as_str(),
                symbol.line,
                place,
            )
        } else {
            (index.files[place as usize].as_str(), 0, place)
        }
    }
    /// Through how many links a place uses others, or others use it.
    pub(super) fn weight(&self, place: u32, outgoing: bool) -> u64 {
        self.beside(place, outgoing)
            .iter()
            .map(|(_, flow)| flow.links)
            .sum()
    }
    /// Through how many links the places `held` use what is `apart` from
    /// them, or are used by it.
    pub(super) fn crossing(
        &self,
        held: &[u32],
        outgoing: bool,
        apart: &dyn Fn(u32) -> bool,
    ) -> u64 {
        let mut links = 0u64;
        for place in held {
            if self.symbols {
                // Two symbols are linked once.
                self.each(*place, outgoing, &mut |other| {
                    links += u64::from(apart(other))
                });
            } else {
                let side = if outgoing { &self.uses } else { &self.users };
                links += side
                    .get(place)
                    .into_iter()
                    .flatten()
                    .filter(|(other, _)| apart(*other))
                    .map(|(_, flow)| flow.links)
                    .sum::<u64>();
            }
        }
        links
    }
    /// Whether the name of a place holds `needle`, which is in small
    /// letters: a file is named by its path, as where a change is drawn,
    /// and a symbol as it is shown, as in its file.
    pub(super) fn named(&self, place: u32, needle: &str) -> bool {
        if self.symbols {
            self.index.shown(place).to_lowercase().contains(needle)
        } else {
            self.index.files[place as usize]
                .to_lowercase()
                .contains(needle)
        }
    }
    /// The name a picture selects a place by: a file by its path, a symbol
    /// as the picture of its file names it.
    pub(super) fn key(&self, names: &Names, place: u32) -> String {
        if self.symbols {
            names.key(place)
        } else {
            self.index.files[place as usize].clone()
        }
    }
    /// A place as a box of a picture, with what a chosen change did to it
    /// and how much it uses and is used. `key` is the name it is selected
    /// by; a file is named by the end of its path that none of `paths`
    /// shares.
    pub(super) fn node(&self, names: &Names, place: u32, key: &str, paths: &[&str]) -> Value {
        let mut node = if self.symbols {
            names.node(place, key)
        } else {
            let status = names.letter(key);
            json!({
                "path": key,
                "name": label(key, paths),
                "folder": false,
                "files": 1,
                "symbols": self.index.holds(place),
                "within": 0,
                "changed": u64::from(status.is_some()),
                "status": status.map(String::from),
            })
        };
        node["out"] = json!(self.weight(place, true));
        node["in"] = json!(self.weight(place, false));
        node
    }
    /// A place drawn as a row beside a selected one, with the links between
    /// the two. Two symbols linked once are linked in one way, which
    /// `linked` asks for: the one that uses and the one that is used.
    pub(super) fn row(
        &self,
        names: &Names,
        paths: &[&str],
        other: u32,
        flow: Flow,
        linked: Option<(u32, u32)>,
    ) -> Value {
        if self.symbols {
            let relation = linked
                .filter(|_| flow.links == 1)
                .and_then(|(user, used)| {
                    self.index
                        .wired(user, true, self.certain)
                        .find(|link| link.1 == used)
                })
                .map(|link| link.2);
            names.row(other, true, flow, relation)
        } else {
            let path = self.index.files[other as usize].as_str();
            json!({
                "path": path,
                "name": label(path, paths),
                "folder": false,
                "inside": true,
                "links": flow.links,
                "guessed": flow.guessed,
                "changed": names.letter(path).is_some(),
            })
        }
    }
    pub(super) fn edge_kinds(&self, users: &[u32], used: &HashSet<u32>) -> BTreeMap<u16, u64> {
        let index = self.index;
        let mut kinds: BTreeMap<u16, u64> = BTreeMap::new();
        if self.symbols {
            for user in users {
                index.collect_symbol_kinds(*user, self.certain, &mut kinds, |other| {
                    used.contains(&other)
                });
            }
        } else if used.len() == 1 {
            let target = *used.iter().next().unwrap();
            for &user in users {
                if let Ok(pos) = index
                    .pairs
                    .binary_search_by_key(&(user, target), |(from, to, _)| (*from, *to))
                {
                    add_kind_counts(&mut kinds, index.pairs[pos].2.kinds_for(self.certain));
                }
            }
        } else if !used.is_empty() {
            for &user in users {
                let start = index.pairs.partition_point(|(from, ..)| *from < user);
                for (from, other, pair) in &index.pairs[start..] {
                    if *from != user {
                        break;
                    }
                    if used.contains(other) {
                        add_kind_counts(&mut kinds, pair.kinds_for(self.certain));
                    }
                }
            }
        }
        kinds
    }
    /// How the places `users` use the places `used`: each kind of link with
    /// how many there are of it, the most first.
    pub(super) fn kinds(&self, users: &[u32], used: &HashSet<u32>) -> Value {
        json!(self.index.kinds_json(&self.edge_kinds(users, used)))
    }
    /// Serialize one edge between `from_places` and `to_places` with kind breakdown.
    pub(super) fn edge_json(
        &self,
        from_places: &[u32],
        to_places: &[u32],
        from_key: &str,
        to_key: &str,
        flow: Flow,
    ) -> Value {
        let used = to_places.iter().copied().collect::<HashSet<_>>();
        let kinds = self.edge_kinds(from_places, &used);
        self.index
            .edge_json(from_key, to_key, &EdgeFlow { flow, kinds })
    }
}
