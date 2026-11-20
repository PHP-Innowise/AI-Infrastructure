//! The files of a project that use each other in a circle.
//!
//! Files that all reach each other along the links form one group: every
//! file of a group lies on a cycle with every other file of it, and with no
//! file outside it. The groups are found once, over every file of the graph,
//! so a picture can say of a box that it is part of a cycle also where the
//! rest of the cycle is not drawn: in another folder, or past what the
//! picture has room for.

use super::*;

/// No group.
const NONE: u32 = u32::MAX;
/// How many groups a picture lists, and how many files of each it names.
const LISTED: usize = 6;
const NAMED: usize = 4;

#[derive(Debug, Default)]
pub(super) struct Circles {
    /// The files of each group in their order; the largest group first.
    groups: Vec<Vec<u32>>,
    /// Of each group the file with the most links inside the group: a cycle
    /// shown through it is the one most of the group has to do with.
    busiest: Vec<u32>,
}

impl Circles {
    /// The groups among `count` files, of which `links` says which uses
    /// which and through how many links. A file that uses itself is no
    /// circle of files.
    pub(super) fn of(count: usize, links: &[(u32, u32, u64)]) -> Self {
        let mut next = vec![Vec::new(); count];
        for (from, to, _) in links {
            if from != to && (*from as usize) < count && (*to as usize) < count {
                next[*from as usize].push(*to);
            }
        }
        let mut groups = reaching_each_other(&next);
        groups.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
        // For each file its group, to tell the links inside one.
        let mut group = vec![NONE; count];
        for (id, files) in groups.iter().enumerate() {
            for file in files {
                group[*file as usize] = id as u32;
            }
        }
        let mut inside = vec![0u64; count];
        for (from, to, weight) in links {
            if from != to
                && (*from as usize) < count
                && (*to as usize) < count
                && group[*from as usize] != NONE
                && group[*from as usize] == group[*to as usize]
            {
                inside[*from as usize] += weight;
                inside[*to as usize] += weight;
            }
        }
        let busiest = groups
            .iter()
            .map(|files| {
                *files
                    .iter()
                    .max_by(|a, b| {
                        inside[**a as usize]
                            .cmp(&inside[**b as usize])
                            .then(b.cmp(a))
                    })
                    .expect("a group holds files")
            })
            .collect();
        Self { groups, busiest }
    }
}

/// The groups of two places or more that all reach each other, each in the
/// order of its places. Found in one walk that goes as deep as the links
/// lead and remembers its way in a list, so the longest chain of a large
/// project is walked like any other.
fn reaching_each_other(next: &[Vec<u32>]) -> Vec<Vec<u32>> {
    let count = next.len();
    // When a place was first met, and the earliest met place it leads back to.
    let mut met = vec![NONE; count];
    let mut back = vec![0u32; count];
    let mut open = vec![false; count];
    let mut waiting: Vec<u32> = Vec::new();
    let mut found = Vec::new();
    let mut clock = 0u32;
    for start in 0..count as u32 {
        if met[start as usize] != NONE {
            continue;
        }
        let mut way: Vec<(u32, usize)> = vec![(start, 0)];
        met[start as usize] = clock;
        back[start as usize] = clock;
        clock += 1;
        waiting.push(start);
        open[start as usize] = true;
        while let Some((at, tried)) = way.last_mut() {
            let at = *at;
            if let Some(to) = next[at as usize].get(*tried).copied() {
                *tried += 1;
                if met[to as usize] == NONE {
                    met[to as usize] = clock;
                    back[to as usize] = clock;
                    clock += 1;
                    waiting.push(to);
                    open[to as usize] = true;
                    way.push((to, 0));
                } else if open[to as usize] {
                    back[at as usize] = back[at as usize].min(met[to as usize]);
                }
                continue;
            }
            way.pop();
            if let Some((before, _)) = way.last() {
                back[*before as usize] = back[*before as usize].min(back[at as usize]);
            }
            if back[at as usize] == met[at as usize] {
                let mut group = Vec::new();
                while let Some(place) = waiting.pop() {
                    open[place as usize] = false;
                    group.push(place);
                    if place == at {
                        break;
                    }
                }
                if group.len() > 1 {
                    group.sort_unstable();
                    found.push(group);
                }
            }
        }
    }
    found
}

impl Index {
    /// The circles among the files, by every link or by the links the tool
    /// read from the code. Found once for each.
    fn circles(&self, certain: bool) -> &Circles {
        self.circles[usize::from(certain)].get_or_init(|| {
            let links = self
                .pairs
                .iter()
                .map(|(from, to, pair)| (*from, *to, pair.weight(certain)))
                .filter(|(_, _, weight)| *weight > 0)
                .collect::<Vec<_>>();
            Circles::of(self.files.len(), &links)
        })
    }

    /// Returns strongly-connected file groups (each sorted by path) for `certain` or all edges.
    pub(crate) fn scc_groups(&self, certain: bool) -> Vec<Vec<String>> {
        self.circles(certain)
            .groups
            .iter()
            .map(|group| {
                let mut paths = group
                    .iter()
                    .map(|idx| self.files[*idx as usize].clone())
                    .collect::<Vec<_>>();
                paths.sort();
                paths
            })
            .collect()
    }

    /// Says of each file and folder a picture draws whether it is part of
    /// a circle of files, wherever the rest of that circle lies, and of the
    /// picture how many circles the project holds and which touch what is
    /// open. A file is part of one group at most. A folder is told how many
    /// of its files are part of how many groups, and whether one of those
    /// leads out of the folder and back: only then does a cycle pass
    /// through the folder as the place the picture draws.
    pub(crate) fn mark_circles(&self, view: &mut Value) {
        let circles = self.circles(view["certain"] == true);
        if circles.groups.is_empty() {
            return;
        }
        let grouped = circles
            .groups
            .iter()
            .enumerate()
            .flat_map(|(id, files)| {
                files
                    .iter()
                    .map(move |file| (self.files[*file as usize].as_str(), id))
            })
            .collect::<HashMap<_, _>>();
        let held = |folder: &str, files: &[u32]| {
            files
                .iter()
                .filter(|file| {
                    self.files[**file as usize]
                        .strip_prefix(folder)
                        .is_some_and(|rest| rest.starts_with('/'))
                })
                .count()
        };
        if let Some(nodes) = view["nodes"].as_array_mut() {
            for node in nodes {
                if node["symbol"] == true {
                    continue;
                }
                let Some(path) = node["path"].as_str() else {
                    continue;
                };
                let told = if node["folder"] == true {
                    let (mut files, mut groups, mut largest, mut leaves) = (0, 0, 0, false);
                    for group in &circles.groups {
                        let inside = held(path, group);
                        if inside > 0 {
                            files += inside;
                            groups += 1;
                            largest = largest.max(group.len());
                            leaves |= inside < group.len();
                        }
                    }
                    (files > 0).then_some((files, groups, largest, leaves))
                } else {
                    grouped
                        .get(path)
                        .map(|group| (1, 1, circles.groups[*group].len(), true))
                };
                if let Some((files, groups, largest, leaves)) = told {
                    node["circle"] = json!({
                        "files": files, "groups": groups, "largest": largest, "leaves": leaves,
                    });
                }
            }
        }
        // What is open: a folder, or the folder of an opened file.
        let scope = view["scope"].as_str().unwrap_or("").trim_matches('/');
        let scope = if scope.is_empty() || self.folder(scope) {
            scope
        } else {
            holder(scope)
        };
        let here = circles
            .groups
            .iter()
            .enumerate()
            .filter(|(_, files)| scope.is_empty() || held(scope, files) > 0)
            .collect::<Vec<_>>();
        let certain = view["certain"] == true;
        let listed = here
            .iter()
            .take(LISTED)
            .map(|(id, files)| {
                let start = circles.busiest[*id];
                // The file a cycle is shown through first, then the others.
                let names = std::iter::once(start)
                    .chain(files.iter().copied().filter(|file| *file != start))
                    .take(NAMED)
                    .map(|file| name(&self.files[file as usize]))
                    .collect::<Vec<_>>();
                json!({
                    "files": files.len(),
                    "start": self.files[start as usize],
                    "names": names,
                    "more": files.len().saturating_sub(NAMED),
                })
            })
            .collect::<Vec<_>>();
        let remediation = here
            .iter()
            .take(LISTED)
            .filter_map(|(id, files)| {
                let start = circles.busiest[*id];
                let (cuts, feedback_arc_cut) = scc_cut_candidates(self, files, certain);
                if cuts.is_empty() && feedback_arc_cut.is_empty() {
                    return None;
                }
                let mut entry = json!({
                    "files": files.len(),
                    "cuts": cuts,
                    "feedback_arc_cut": feedback_arc_cut,
                });
                if let Some(witness) = shortest_witness_cycle(self, files, start, certain) {
                    entry["witness_cycle"] = witness;
                }
                Some(entry)
            })
            .collect::<Vec<_>>();
        view["circles"] = json!({
            "groups": circles.groups.len(),
            "files": circles.groups.iter().map(Vec::len).sum::<usize>(),
            "largest": circles.groups.first().map_or(0, Vec::len),
            "here": here.len(),
            "listed": listed,
        });
        if !remediation.is_empty() {
            if let Some(first_witness) = remediation.first().and_then(|r| r.get("witness_cycle")) {
                view["cycle_witness"] = first_witness.clone();
            }
            view["circle_remediation"] = json!(remediation);
        }
    }
}

/// Returns up to 5 concrete symbol/import provenance entries explaining why `from_file -> to_file` exists.
fn file_pair_provenance(index: &Index, from_file: u32, to_file: u32, certain: bool) -> Vec<Value> {
    let mut rows: Vec<(u32, String, String, String, bool)> = Vec::new();
    for sym_idx in index.range(from_file) {
        for &(_, to_sym, kind_id, guessed) in index.wired(sym_idx, true, certain) {
            let target_sym = &index.symbols[to_sym as usize];
            if target_sym.file != to_file {
                continue;
            }
            let relation = index.kinds[kind_id as usize].clone();
            let from_sym = &index.symbols[sym_idx as usize];
            rows.push((
                from_sym.line,
                index.shown(sym_idx),
                index.shown(to_sym),
                relation,
                guessed,
            ));
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    rows.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1 && a.2 == b.2 && a.3 == b.3);
    rows.into_iter()
        .take(5)
        .map(|(line, from_symbol, to_symbol, relation, guessed)| {
            json!({
                "from_symbol": from_symbol,
                "to_symbol": to_symbol,
                "relation": relation,
                "line": line,
                "guessed": guessed,
            })
        })
        .collect()
}

/// Computes the shortest directed cycle starting and ending at `start` inside `group`.
fn shortest_witness_cycle(
    index: &Index,
    group: &[u32],
    start: u32,
    certain: bool,
) -> Option<Value> {
    if group.len() < 2 {
        return None;
    }
    let pos_of: HashMap<u32, usize> = group
        .iter()
        .enumerate()
        .map(|(idx, &file)| (file, idx))
        .collect();
    let &start_pos = pos_of.get(&start)?;
    let mut adj: Vec<Vec<(usize, u64)>> = vec![Vec::new(); group.len()];
    let mut edge_weight: HashMap<(usize, usize), u64> = HashMap::new();
    for &(from, to, ref pair) in &index.pairs {
        let weight = pair.weight(certain);
        if weight == 0 || from == to {
            continue;
        }
        if let (Some(&u), Some(&v)) = (pos_of.get(&from), pos_of.get(&to)) {
            adj[u].push((v, weight));
            edge_weight.insert((u, v), weight);
        }
    }
    for neighbors in &mut adj {
        neighbors.sort_by(|a, b| {
            index.files[group[a.0] as usize].cmp(&index.files[group[b.0] as usize])
        });
    }

    let mut parent = vec![usize::MAX; group.len()];
    let mut visited = vec![false; group.len()];
    let mut queue = std::collections::VecDeque::new();
    for &(v, _) in &adj[start_pos] {
        if !visited[v] {
            visited[v] = true;
            parent[v] = start_pos;
            queue.push_back(v);
        }
    }
    let mut back_from = None;
    while let Some(u) = queue.pop_front() {
        for &(v, _) in &adj[u] {
            if v == start_pos {
                back_from = Some(u);
                break;
            }
            if !visited[v] {
                visited[v] = true;
                parent[v] = u;
                queue.push_back(v);
            }
        }
        if back_from.is_some() {
            break;
        }
    }
    let last = back_from?;
    let mut cycle_positions = Vec::new();
    let mut cur = last;
    while cur != start_pos && cur != usize::MAX {
        cycle_positions.push(cur);
        cur = parent[cur];
    }
    if cur != start_pos {
        return None;
    }
    cycle_positions.push(start_pos);
    cycle_positions.reverse();

    let mut files: Vec<&str> = cycle_positions
        .iter()
        .map(|&p| index.files[group[p] as usize].as_str())
        .collect();
    files.push(index.files[start as usize].as_str());
    let mut edges_json = Vec::with_capacity(cycle_positions.len());
    for i in 0..cycle_positions.len() {
        let u = cycle_positions[i];
        let v = cycle_positions[(i + 1) % cycle_positions.len()];
        let from_file = group[u];
        let to_file = group[v];
        let weight = edge_weight.get(&(u, v)).copied().unwrap_or(1);
        let provenance = file_pair_provenance(index, from_file, to_file, certain);
        edges_json.push(json!({
            "from": index.files[from_file as usize],
            "to": index.files[to_file as usize],
            "weight": weight,
            "provenance": provenance,
        }));
    }
    Some(json!({
        "length": cycle_positions.len(),
        "files": files,
        "edges": edges_json,
    }))
}

/// Computes minimal feedback-edge cut candidates + greedy minimal feedback-arc cut with provenance.
fn scc_cut_candidates(index: &Index, group: &[u32], certain: bool) -> (Vec<Value>, Vec<Value>) {
    if group.len() < 2 {
        return (Vec::new(), Vec::new());
    }
    let pos_of: HashMap<u32, usize> = group
        .iter()
        .enumerate()
        .map(|(idx, &file)| (file, idx))
        .collect();
    let mut edges: Vec<(usize, usize, u32, u32, u64, u64)> = Vec::new();
    for &(from, to, ref pair) in &index.pairs {
        let weight = pair.weight(certain);
        if weight == 0 || from == to {
            continue;
        }
        if let (Some(&u), Some(&v)) = (pos_of.get(&from), pos_of.get(&to)) {
            let guessed = if certain { 0 } else { pair.guessed };
            edges.push((u, v, from, to, weight, guessed));
        }
    }
    if edges.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let check_breaks = group.len() <= 32 && edges.len() <= 128;
    let breaks_scc = |removed: &[bool]| -> bool {
        if !check_breaks {
            return false;
        }
        let mut adj = vec![Vec::new(); group.len()];
        for (idx, &(u, v, ..)) in edges.iter().enumerate() {
            if !removed[idx] {
                adj[u].push(v as u32);
            }
        }
        let sccs = reaching_each_other(&adj);
        sccs.iter().all(|comp| comp.len() < group.len())
    };
    let is_acyclic = |removed: &[bool]| -> bool {
        let mut adj = vec![Vec::new(); group.len()];
        for (idx, &(u, v, ..)) in edges.iter().enumerate() {
            if !removed[idx] {
                adj[u].push(v as u32);
            }
        }
        reaching_each_other(&adj).is_empty()
    };

    let mut ranked: Vec<(bool, u64, u64, u32, u32, usize)> = edges
        .iter()
        .enumerate()
        .map(|(idx, &(_, _, from, to, links, guessed))| {
            let mut single_removed = vec![false; edges.len()];
            single_removed[idx] = true;
            (breaks_scc(&single_removed), links, guessed, from, to, idx)
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.cmp(&b.1))
            .then(a.2.cmp(&b.2))
            .then(index.files[a.3 as usize].cmp(&index.files[b.3 as usize]))
            .then(index.files[a.4 as usize].cmp(&index.files[b.4 as usize]))
    });
    let mk_cut_json = |breaks: bool, links: u64, guessed: u64, from: u32, to: u32| {
        let from_path = index.files[from as usize].as_str();
        let to_path = index.files[to as usize].as_str();
        let provenance = file_pair_provenance(index, from, to, certain);
        json!({
            "from": from_path,
            "to": to_path,
            "links": links,
            "weight": links,
            "guessed": guessed,
            "breaks_cycle": breaks,
            "breaks_cycles": breaks,
            "provenance": provenance,
            "hint": format!(
                "Extract an interface/port from {} or invert {} -> {} ({} link{}) to break this cycle.",
                name(to_path),
                name(from_path),
                name(to_path),
                links,
                if links == 1 { "" } else { "s" }
            ),
        })
    };

    let cuts: Vec<Value> = ranked
        .iter()
        .take(3)
        .map(|&(breaks, links, guessed, from, to, _)| mk_cut_json(breaks, links, guessed, from, to))
        .collect();

    let mut fas_cut = Vec::new();
    if group.len() <= 64 && edges.len() <= 256 {
        let mut removed = vec![false; edges.len()];
        let mut by_weight: Vec<usize> = (0..edges.len()).collect();
        by_weight.sort_by(|&a, &b| {
            edges[a]
                .4
                .cmp(&edges[b].4)
                .then(edges[a].5.cmp(&edges[b].5))
                .then(index.files[edges[a].2 as usize].cmp(&index.files[edges[b].2 as usize]))
                .then(index.files[edges[a].3 as usize].cmp(&index.files[edges[b].3 as usize]))
        });
        while !is_acyclic(&removed) {
            let mut chosen = None;
            for &idx in &by_weight {
                if removed[idx] {
                    continue;
                }
                let &(u, v, ..) = &edges[idx];
                let mut adj = vec![Vec::new(); group.len()];
                for (eidx, &(eu, ev, ..)) in edges.iter().enumerate() {
                    if !removed[eidx] && eidx != idx {
                        adj[eu].push(ev);
                    }
                }
                let mut seen = vec![false; group.len()];
                let mut stack = vec![v];
                seen[v] = true;
                let mut in_cycle = false;
                while let Some(cur) = stack.pop() {
                    if cur == u {
                        in_cycle = true;
                        break;
                    }
                    for &nxt in &adj[cur] {
                        if !seen[nxt] {
                            seen[nxt] = true;
                            stack.push(nxt);
                        }
                    }
                }
                if in_cycle {
                    chosen = Some(idx);
                    break;
                }
            }
            let Some(idx) = chosen else {
                break;
            };
            removed[idx] = true;
            let &(_, _, from, to, links, guessed) = &edges[idx];
            fas_cut.push(mk_cut_json(is_acyclic(&removed), links, guessed, from, to));
            if fas_cut.len() >= 16 {
                break;
            }
        }
    }

    (cuts, fas_cut)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups(count: usize, links: &[(u32, u32)]) -> Vec<Vec<u32>> {
        let links = links
            .iter()
            .map(|(from, to)| (*from, *to, 1))
            .collect::<Vec<_>>();
        Circles::of(count, &links).groups
    }

    #[test]
    fn files_that_all_reach_each_other_are_one_group() {
        // 0 → 1 → 2 → 0 is a circle; 3 uses it and 4 is used by it, and
        // neither is part of it. 5 ⇄ 6 is a second, smaller circle.
        let links = [(0, 1), (1, 2), (2, 0), (3, 0), (2, 4), (5, 6), (6, 5)];
        assert_eq!(groups(7, &links), [vec![0, 1, 2], vec![5, 6]]);
        // Two circles that share a file are one group.
        assert_eq!(
            groups(5, &[(0, 1), (1, 0), (1, 2), (2, 3), (3, 1), (3, 4)]),
            [vec![0, 1, 2, 3]]
        );
        // A chain is none, and neither is a file that uses itself.
        assert!(groups(4, &[(0, 1), (1, 2), (2, 3), (3, 3)]).is_empty());
        // A link that names a file the graph does not hold is no link.
        assert!(groups(2, &[(0, 1), (1, 9), (9, 0)]).is_empty());
        assert!(groups(0, &[]).is_empty());
        // Of two groups as large as each other the one of the earlier
        // files comes first.
        assert_eq!(
            groups(4, &[(2, 3), (3, 2), (0, 1), (1, 0)]),
            [vec![0, 1], vec![2, 3]]
        );
    }

    #[test]
    fn a_chain_as_long_as_a_large_project_is_walked() {
        // A hundred thousand files in one chain that closes: a walk that
        // called itself for each would run out of room.
        let count = 100_000u32;
        let links = (0..count)
            .map(|at| (at, (at + 1) % count))
            .collect::<Vec<_>>();
        let found = groups(count as usize, &links);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].len(), count as usize);
        // Without the link that closes it there is no circle at all.
        assert!(groups(count as usize, &links[..links.len() - 1]).is_empty());
    }

    #[test]
    fn the_file_a_group_is_shown_through_has_the_most_links_inside_it() {
        // 1 has links with both others of its group; what 0 has with 3,
        // outside the group, counts for nothing.
        let circles = Circles::of(4, &[(0, 1, 1), (1, 2, 5), (2, 0, 1), (0, 3, 90), (2, 1, 2)]);
        assert_eq!(circles.groups, [vec![0, 1, 2]]);
        assert_eq!(circles.busiest, [1]);
        // As many links: the earlier file.
        assert_eq!(Circles::of(2, &[(0, 1, 1), (1, 0, 1)]).busiest, [0]);
    }

    fn project() -> Index {
        let node = |id: &str, file: &str| json!({"id": id, "label": format!("{id}()"), "source_file": file, "source_location": "L2"});
        let edge = |from: &str, to: &str, confidence: &str| json!({"source": from, "target": to, "relation": "calls", "confidence": confidence});
        // In src/cart, Cart and Price use each other. Order (src/order)
        // uses Cart, which uses Stock (src/stock), which uses Order again:
        // one group of four across three folders. The link from Stock to
        // Order is only guessed. In src/mail, Mailer and Queue use each
        // other and nothing else: a group that stays inside its folder.
        let graph = json!({
            "nodes": [
                node("cart", "src/cart/Cart.php"),
                node("price", "src/cart/Price.php"),
                node("order", "src/order/Order.php"),
                node("stock", "src/stock/Stock.php"),
                node("mailer", "src/mail/Mailer.php"),
                node("queue", "src/mail/Queue.php"),
                node("entry", "web/index.php"),
            ],
            "links": [
                edge("cart", "price", "EXTRACTED"),
                edge("price", "cart", "EXTRACTED"),
                edge("order", "cart", "EXTRACTED"),
                edge("cart", "stock", "EXTRACTED"),
                edge("stock", "order", "INFERRED"),
                edge("mailer", "queue", "EXTRACTED"),
                edge("queue", "mailer", "EXTRACTED"),
                edge("entry", "order", "EXTRACTED"),
                edge("entry", "mailer", "EXTRACTED"),
            ]
        });
        Index::read(graph.to_string().as_bytes(), Path::new("/work/shop")).unwrap()
    }

    fn node<'a>(view: &'a Value, path: &str) -> &'a Value {
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["path"] == path)
            .unwrap_or_else(|| panic!("{path} in {view}"))
    }

    #[test]
    fn a_picture_says_which_of_its_places_are_part_of_a_circle() {
        let index = project();
        // The folders of src: the picture draws four boxes and no file.
        let mut view = index.view("shop", "src", None, false, "");
        index.mark_circles(&mut view);
        // src/cart holds two files of the large group, which leads out of
        // it and back; so do src/order and src/stock, with one file each.
        assert_eq!(
            node(&view, "src/cart")["circle"],
            json!({"files": 2, "groups": 1, "largest": 4, "leaves": true})
        );
        assert_eq!(
            node(&view, "src/order")["circle"],
            json!({"files": 1, "groups": 1, "largest": 4, "leaves": true})
        );
        // The circle of src/mail never leaves it: no cycle passes through
        // the folder as one place.
        assert_eq!(
            node(&view, "src/mail")["circle"],
            json!({"files": 2, "groups": 1, "largest": 2, "leaves": false})
        );
        assert_eq!(
            view["circles"],
            json!({
                "groups": 2, "files": 6, "largest": 4, "here": 2,
                "listed": [
                    {"files": 4, "start": "src/cart/Cart.php", "names": ["Cart.php", "Price.php", "Order.php", "Stock.php"], "more": 0},
                    {"files": 2, "start": "src/mail/Mailer.php", "names": ["Mailer.php", "Queue.php"], "more": 0}
                ]
            })
        );
        // The files of one folder: each is told its own group, though the
        // rest of the group lies in folders this picture does not draw.
        let mut view = index.view("shop", "src/order", None, false, "");
        index.mark_circles(&mut view);
        assert_eq!(
            node(&view, "src/order/Order.php")["circle"],
            json!({"files": 1, "groups": 1, "largest": 4, "leaves": true})
        );
        // Only the group that touches the open folder is listed.
        assert_eq!(view["circles"]["here"], 1);
        assert_eq!(view["circles"]["groups"], 2);
        assert_eq!(view["circles"]["listed"][0]["start"], "src/cart/Cart.php");
        // The top of the project: web holds no file of a circle, and all
        // of both circles lies inside src.
        let mut view = index.view("shop", "", None, false, "");
        index.mark_circles(&mut view);
        assert!(node(&view, "web")["circle"].is_null());
        assert_eq!(
            node(&view, "src")["circle"],
            json!({"files": 6, "groups": 2, "largest": 4, "leaves": false})
        );
    }

    #[test]
    fn a_circle_closed_by_a_guess_is_none_among_the_links_read_from_the_code() {
        let index = project();
        let mut view = index.view("shop", "src", None, true, "");
        index.mark_circles(&mut view);
        // Without the guessed link from Stock to Order only Cart and Price
        // are left of the large group, and they stay inside their folder.
        assert_eq!(
            node(&view, "src/cart")["circle"],
            json!({"files": 2, "groups": 1, "largest": 2, "leaves": false})
        );
        assert!(node(&view, "src/order")["circle"].is_null());
        assert!(node(&view, "src/stock")["circle"].is_null());
        assert_eq!(view["circles"]["groups"], 2);
        assert_eq!(view["circles"]["files"], 4);
    }

    #[test]
    fn a_cycle_can_be_shown_through_every_place_that_is_told_one() {
        let index = project();
        let steps = |place: &str, certain: bool| {
            index.cycle("shop", place, &[], &|_| None, None, certain)["cycle"]["steps"]
                .as_u64()
                .unwrap()
        };
        for certain in [false, true] {
            for scope in ["", "src", "src/cart", "src/order", "src/mail"] {
                let mut view = index.view("shop", scope, None, certain, "");
                index.mark_circles(&mut view);
                for node in view["nodes"].as_array().unwrap() {
                    let path = node["path"].as_str().unwrap();
                    let through = steps(path, certain) > 0;
                    assert_eq!(
                        node["circle"]["leaves"] == true,
                        through,
                        "{path} in {scope:?}, certain {certain}: {}",
                        node["circle"]
                    );
                }
            }
        }
    }

    #[test]
    fn a_project_without_circles_is_told_nothing() {
        let graph = json!({
            "nodes": [
                {"id": "a", "label": "a()", "source_file": "src/a.php", "source_location": "L1"},
                {"id": "b", "label": "b()", "source_file": "src/b.php", "source_location": "L1"}
            ],
            "links": [{"source": "a", "target": "b", "relation": "calls", "confidence": "EXTRACTED"}]
        });
        let index = Index::read(graph.to_string().as_bytes(), Path::new("/work/shop")).unwrap();
        let mut view = index.view("shop", "src", None, false, "");
        index.mark_circles(&mut view);
        assert!(view["circles"].is_null());
        assert!(
            view["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .all(|node| node["circle"].is_null())
        );
    }

    #[test]
    fn circle_remediation_ranks_minimal_breaking_cuts_without_altering_circles() {
        let graph = json!({
            "nodes": [
                {"id": "a", "label": "a()", "source_file": "src/a.php", "source_location": "L1"},
                {"id": "b", "label": "b()", "source_file": "src/b.php", "source_location": "L1"},
                {"id": "c", "label": "c()", "source_file": "src/c.php", "source_location": "L1"}
            ],
            "links": [
                {"source": "a", "target": "b", "relation": "calls", "confidence": "EXTRACTED"},
                {"source": "b", "target": "c", "relation": "calls", "confidence": "EXTRACTED"},
                {"source": "b", "target": "c", "relation": "imports", "confidence": "EXTRACTED"},
                {"source": "c", "target": "a", "relation": "calls", "confidence": "INFERRED"}
            ]
        });
        let index = Index::read(graph.to_string().as_bytes(), Path::new("/work/shop")).unwrap();
        let mut view = index.view("shop", "src", None, false, "");
        index.mark_circles(&mut view);
        let rem = view["circle_remediation"].as_array().expect("remediation");
        assert_eq!(rem.len(), 1);
        let cuts = rem[0]["cuts"].as_array().unwrap();
        assert!(!cuts.is_empty());
        assert_eq!(cuts[0]["breaks_cycles"], true);
        assert_eq!(cuts[0]["weight"], 1);
        assert!(
            cuts[0]["hint"].as_str().unwrap().contains("Break")
                || cuts[0]["hint"].as_str().unwrap().contains("Extract")
        );
        assert!(rem[0]["feedback_arc_cut"].is_array());
        assert!(!rem[0]["feedback_arc_cut"].as_array().unwrap().is_empty());
        let witness = &rem[0]["witness_cycle"];
        assert!(witness["length"].as_u64().unwrap() >= 2);
        let edges = witness["edges"].as_array().unwrap();
        assert!(!edges.is_empty());
        assert!(!edges[0]["provenance"].as_array().unwrap().is_empty());
    }
}
