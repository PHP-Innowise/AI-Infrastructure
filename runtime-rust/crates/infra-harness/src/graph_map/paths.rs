use super::*;

/// The links a picture of ways draws, the strongest first, each from the
/// box numbered first to the box numbered second; `keys` names the boxes.
/// Where there are more links than a picture draws, the strongest into each
/// box and the strongest out of it come before the rest: every box still
/// lies on a way that is drawn whole.
pub(super) fn ways_drawn(
    mut edges: Vec<(usize, usize, Flow)>,
    keys: &[String],
) -> Vec<(usize, usize, Flow)> {
    let stronger = |a: &(usize, usize, Flow), b: &(usize, usize, Flow)| {
        b.2.links
            .cmp(&a.2.links)
            .then((&keys[a.0], &keys[a.1]).cmp(&(&keys[b.0], &keys[b.1])))
    };
    edges.sort_by(stronger);
    if edges.len() > DRAWN {
        let (mut led, mut leads) = (HashSet::new(), HashSet::new());
        let (mut needed, mut rest) = (Vec::new(), Vec::new());
        for edge in edges {
            let (into, out) = (led.insert(edge.1), leads.insert(edge.0));
            if into || out {
                needed.push(edge);
            } else {
                rest.push(edge);
            }
        }
        let room = DRAWN.saturating_sub(needed.len());
        needed.extend(rest.into_iter().take(room));
        needed.sort_by(stronger);
        edges = needed;
    }
    edges
}
impl Index {
    /// One end of a path, as the pictures name it.
    pub(super) fn end(&self, place: &str, reach: Reach) -> Value {
        match reach {
            Reach::Symbol(at) => {
                let symbol = &self.symbols[at as usize];
                json!({
                    "path": place,
                    "name": self.shown(at),
                    "what": symbol.kind.word(),
                    "symbol": true,
                    "file": self.files[symbol.file as usize],
                    "line": symbol.line,
                })
            }
            Reach::File(_) => json!({"path": place, "name": name(place), "what": "file"}),
            Reach::Folder(folder) => {
                json!({"path": folder, "name": name(folder), "what": "folder"})
            }
        }
    }
    /// Where a place lies, which is where the way back from a picture of two
    /// places leads: the file of a symbol, the folder of a file or a folder.
    pub(super) fn lies<'a>(&'a self, reach: Reach<'a>) -> &'a str {
        match reach {
            Reach::Symbol(at) => self.files[self.symbols[at as usize].file as usize].as_str(),
            Reach::File(file) => holder(&self.files[file as usize]),
            Reach::Folder(folder) => holder(folder),
        }
    }
    /// One shortest directed cycle through a file or symbol, or across the
    /// boundary of a folder. The origin is drawn once, with a return arrow.
    pub fn cycle(
        &self,
        project: &str,
        place: &str,
        changed: &[(String, char)],
        edited: &dyn Fn(&str) -> Option<Edited>,
        focus: Option<&str>,
        certain: bool,
    ) -> Value {
        let reach = self.reach(place);
        let symbols = matches!(reach, Some(Reach::Symbol(_)));
        let scope = reach.map_or_else(|| holder(place), |reach| self.lies(reach));
        let origin = reach.map_or_else(
            || json!({"path": place, "name": name(place), "missing": true}),
            |reach| self.end(place, reach),
        );
        let mut crumbs = Self::crumbs(project, scope);
        crumbs.push(json!({"name": origin["name"], "path": ""}));
        let mut view = json!({
            "mode": "cycle", "scope": scope, "crumbs": crumbs, "certain": certain,
            "totals": self.totals(),
            "cycle": {"origin": origin, "names": symbols, "steps": 0},
            "nodes": [], "edges": [], "focus": null,
        });
        let Some(reach) = reach else { return view };
        let net = Net::new(self, symbols, certain);
        let members = net.members(reach);
        let mut route = net.cycle(&members);
        if route.is_empty() {
            return view;
        }
        view["cycle"]["steps"] = json!(route.len());
        let folder = matches!(reach, Reach::Folder(_));
        let held = members.iter().copied().collect::<HashSet<_>>();
        if folder {
            // A folder arrow represents all links across that boundary,
            // matching what the link inspector shows.
            let crossing = |other, outgoing| {
                net.beside(other, outgoing)
                    .into_iter()
                    .filter(|(member, _)| held.contains(member))
                    .fold(Flow::default(), |sum, (_, flow)| Flow {
                        links: sum.links + flow.links,
                        guessed: sum.guessed + flow.guessed,
                    })
            };
            let first = &mut route[0];
            first.2 = crossing(first.1, false);
            let last = route.last_mut().expect("nonempty cycle");
            last.2 = crossing(last.0, true);
        }
        let names = Names::new(self, changed, edited);
        let keys = std::iter::once(place.to_owned())
            .chain(
                route
                    .iter()
                    .skip(1)
                    .map(|(from, _, _)| net.key(&names, *from)),
            )
            .collect::<Vec<_>>();
        // Bound the SVG layout, while keeping the entire ordered cycle
        // available as text when it is too long to draw.
        if route.len() > FOLLOWED {
            view["cycle"]["places"] = std::iter::once(json!({"path": keys[0]}))
                .chain(route.iter().enumerate().map(|(at, (_, _, flow))| {
                    json!({
                        "path": keys[(at + 1) % keys.len()],
                        "guessed": flow.guessed,
                        "links": flow.links,
                    })
                }))
                .collect();
            return view;
        }
        let paths = keys.iter().map(String::as_str).collect::<Vec<_>>();
        let node = |at: usize| {
            let mut node = if at == 0 && folder {
                let apart = |file| !held.contains(&file);
                json!({
                    "path": keys[0], "name": origin["name"], "folder": true,
                    "files": members.len(),
                    "out": net.crossing(&members, true, &apart),
                    "in": net.crossing(&members, false, &apart),
                })
            } else {
                net.node(&names, route[at].0, &keys[at], &paths)
            };
            node["start"] = json!(at == 0);
            node
        };
        let len = route.len();
        view["nodes"] = (0..len).map(node).collect();
        view["edges"] = route
            .iter()
            .enumerate()
            .map(|(at, (from, to, flow))| {
                let next = (at + 1) % len;
                let from_places = if at == 0 && folder {
                    members.as_slice()
                } else {
                    std::slice::from_ref(from)
                };
                let to_places = if next == 0 && folder {
                    members.as_slice()
                } else {
                    std::slice::from_ref(to)
                };
                net.edge_json(from_places, to_places, &keys[at], &keys[next], *flow)
            })
            .collect();
        if let Some(at) = focus.and_then(|selected| keys.iter().position(|key| key == selected)) {
            let row = |other: usize, edge: usize| {
                let (user, used, flow) = route[edge];
                if other == 0 && folder {
                    let mut row = node(other);
                    row["inside"] = json!(true);
                    row["links"] = json!(flow.links);
                    row["guessed"] = json!(flow.guessed);
                    row
                } else {
                    net.row(&names, &paths, route[other].0, flow, Some((user, used)))
                }
            };
            let prior = (at + len - 1) % len;
            let mut focus = node(at);
            focus["inside"] = json!(true);
            focus["uses"] = json!([row((at + 1) % len, at)]);
            focus["used_by"] = json!([row(prior, prior)]);
            view["focus"] = focus;
        }
        let mut cut_candidates: Vec<(u64, u64, &str, &str)> = route
            .iter()
            .enumerate()
            .map(|(at, (_, _, flow))| {
                let next = (at + 1) % len;
                (
                    flow.links,
                    flow.guessed,
                    keys[at].as_str(),
                    keys[next].as_str(),
                )
            })
            .collect();
        cut_candidates.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.cmp(&b.1))
                .then(a.2.cmp(b.2))
                .then(a.3.cmp(b.3))
        });
        view["cycle"]["cut_hints"] = cut_candidates
            .into_iter()
            .take(3)
            .map(|(links, guessed, from_key, to_key)| {
                json!({
                    "from": from_key,
                    "to": to_key,
                    "links": links,
                    "weight": links,
                    "guessed": guessed,
                    "breaks_cycles": true,
                    "hint": format!(
                        "Break cycle by extracting an interface from {} or inverting {} -> {} ({} link{}).",
                        name(to_key),
                        name(from_key),
                        name(to_key),
                        links,
                        if links == 1 { "" } else { "s" }
                    ),
                })
            })
            .collect();
        let mut witness_files = keys.clone();
        if let Some(first) = keys.first() {
            witness_files.push(first.clone());
        }
        view["cycle"]["witness"] = json!({
            "length": len,
            "files": witness_files,
        });
        view
    }
    /// Compare a displayed cycle with the full directed base graph. The
    /// shortest route at base may use a different set of dependencies.
    pub(crate) fn compare_cycle(
        &self,
        raw: &Self,
        base: &Self,
        view: &mut Value,
        renames: &HashMap<&str, &str>,
        certain: bool,
    ) -> (&'static str, Option<String>) {
        let origin = view["cycle"]["origin"]["path"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let head_place = self.reach(&origin);
        let hidden = head_place.is_none();
        let original_place = head_place.or_else(|| raw.reach(&origin));
        let symbols =
            view["cycle"]["names"] == true || matches!(original_place, Some(Reach::Symbol(_)));
        let net = Net::new(base, symbols, certain);
        let mut matched = HashMap::new();
        let mut matches = |path: &Value| {
            let path = path.as_str().unwrap_or_default();
            matched
                .entry(path.to_owned())
                .or_insert_with(|| {
                    let identity = if hidden && path == origin { raw } else { self };
                    identity.cycle_members_at_base(&net, path, renames)
                })
                .clone()
        };
        let result = match matches(&view["cycle"]["origin"]["path"]) {
            None => ("ambiguous", None),
            Some(members) if members.is_empty() => ("missing", None),
            Some(members) if net.cycle(&members).is_empty() => ("absent", None),
            Some(members) => {
                let place = match original_place.or_else(|| base.reach(&origin)) {
                    Some(Reach::Folder(path)) => path.to_owned(),
                    _ => net.key(&Names::new(base, &[], &|_| None), members[0]),
                };
                ("present", Some(place))
            }
        };
        // An ambiguous symbol cannot establish whether its edge was added.
        let added = |from: Option<Vec<u32>>, to: Option<Vec<u32>>| {
            let (from, to) = (from?, to?);
            let to = to.into_iter().collect::<HashSet<_>>();
            Some(!from.iter().any(|from| {
                let mut seen = false;
                net.each(*from, true, &mut |used| seen |= to.contains(&used));
                seen
            }))
        };
        if let Some(edges) = view["edges"].as_array_mut() {
            for edge in edges {
                edge["added"] = json!(added(matches(&edge["from"]), matches(&edge["to"])));
            }
        }
        if let Some(places) = view["cycle"]["places"].as_array_mut() {
            let mut previous = places.first().and_then(|place| matches(&place["path"]));
            for place in places.iter_mut().skip(1) {
                let next = matches(&place["path"]);
                place["added"] = json!(added(previous, next.clone()));
                previous = next;
            }
        }
        result
    }
    pub(super) fn cycle_members_at_base(
        &self,
        net: &Net<'_>,
        path: &str,
        renames: &HashMap<&str, &str>,
    ) -> Option<Vec<u32>> {
        let base = net.index;
        let before = |file: u32| {
            let path = self.files[file as usize].as_str();
            renames.get(path).copied().unwrap_or(path)
        };
        let Some(place) = self.reach(path) else {
            return match base.reach(path) {
                Some(Reach::Symbol(_)) => None,
                Some(place) => Some(net.members(place)),
                None if path.contains('#') => None,
                None => Some(Vec::new()),
            };
        };
        let members = match place {
            Reach::Folder(path) => net.members(Reach::Folder(path)),
            Reach::File(file) => base
                .files
                .iter()
                .position(|file_path| file_path == before(file))
                .map(|at| vec![at as u32])
                .unwrap_or_default(),
            Reach::Symbol(at) => {
                let symbol = &self.symbols[at as usize];
                let Some(Reach::File(file)) = base.reach(before(symbol.file)) else {
                    return Some(Vec::new());
                };
                let label = self.label(at);
                let owner = self.owner(at);
                let same = |index: &Self, other: u32| {
                    index.symbols[other as usize].kind == symbol.kind
                        && (symbol.kind == Kind::File || index.label(other) == label)
                        && index.owner(other) == owner
                };
                let mut matches = base.range(file).filter(|other| same(base, *other));
                let Some(first) = matches.next() else {
                    return Some(Vec::new());
                };
                if matches.next().is_some()
                    || self
                        .range(symbol.file)
                        .filter(|other| same(self, *other))
                        .nth(1)
                        .is_some()
                {
                    return None;
                }
                vec![first]
            }
        };
        Some(members)
    }
    pub(crate) fn has_rust_sources(&self) -> bool {
        self.files.iter().any(|file| file.ends_with(".rs"))
    }
    pub(crate) fn has_test_spans(&self) -> bool {
        self.revision_tests.is_some()
    }
    /// The shortest ways from one place to another along the links: every
    /// chain of the fewest links in which each place uses the next. Either
    /// end is a folder, a file or a symbol. Between folders and files the
    /// chain is one of files, a folder standing for every file in it. Where
    /// an end is a symbol the chain is one of symbols, and a file or a
    /// folder at the other end stands for every symbol in it.
    ///
    /// Where nothing leads from `from` to `to`, the ways from `to` to `from`
    /// are given and said to run the other way round; where nothing leads
    /// either way, or one end lies in the other, the answer says so and
    /// draws nothing. A step shows its `WAY` most linked places, fewer on a
    /// long way; each place drawn keeps the strongest of what leads to it,
    /// so every box lies on a way that is drawn whole, and the answer counts
    /// all the places and links that lie on such ways.
    ///
    /// `changed` marks the files of a chosen change and `edited` the symbols
    /// it touched, as among followed links; `focus` lists what one place
    /// drawn leads to on the way and what leads to it. `None` where an end
    /// names nothing in the graph.
    #[allow(clippy::too_many_arguments)]
    pub fn path(
        &self,
        project: &str,
        from: &str,
        to: &str,
        changed: &[(String, char)],
        edited: &dyn Fn(&str) -> Option<Edited>,
        focus: Option<&str>,
        certain: bool,
    ) -> Option<Value> {
        self.path_with(project, from, to, changed, edited, focus, certain, 0)
    }
    /// The shortest ways, and with them the ways that are longer by
    /// `longer` links at most: what else leads from the one place to the
    /// other, which a dependency is not rid of by cutting the shortest way.
    /// The longer ways are drawn whole, each marked as a way round, as far
    /// as `DETOURS` and the room of the picture go; the answer says how
    /// many it draws and whether there are more.
    #[allow(clippy::too_many_arguments)]
    pub fn path_with(
        &self,
        project: &str,
        from: &str,
        to: &str,
        changed: &[(String, char)],
        edited: &dyn Fn(&str) -> Option<Edited>,
        focus: Option<&str>,
        certain: bool,
        longer: usize,
    ) -> Option<Value> {
        let longer = longer.min(LONGER);
        let (start, end) = (self.reach(from)?, self.reach(to)?);
        let symbols = matches!(start, Reach::Symbol(_)) || matches!(end, Reach::Symbol(_));
        let net = Net::new(self, symbols, certain);
        let (starts, ends) = (net.members(start), net.members(end));
        // How far every place lies from the nearest of `first`, as far as
        // the nearest of `last`, and how far that is: the whole graph is
        // walked a step at a time. `None` where nothing leads there.
        let count = net.count();
        let walk = |first: &[u32], last: &[u32]| {
            let goal = last.iter().copied().collect::<HashSet<_>>();
            let mut far = vec![u32::MAX; count];
            for place in first {
                far[*place as usize] = 0;
            }
            let mut front = first.to_vec();
            let mut steps = 0u32;
            while !front.is_empty() {
                steps += 1;
                let mut next = Vec::new();
                let mut there = false;
                let mut reach = |other: u32| {
                    if far[other as usize] == u32::MAX {
                        far[other as usize] = steps;
                        next.push(other);
                        there |= goal.contains(&other);
                    }
                };
                for place in &front {
                    net.each(*place, true, &mut reach);
                }
                if there {
                    return Some((far, steps as usize));
                }
                front = next;
            }
            None
        };
        // One end that lies in the other is no way from the one to the other.
        let within = {
            let inside = starts.iter().copied().collect::<HashSet<_>>();
            ends.iter().any(|place| inside.contains(place))
        };
        let found = if within {
            None
        } else {
            walk(&starts, &ends)
                .map(|way| (way, false))
                .or_else(|| walk(&ends, &starts).map(|way| (way, true)))
        };
        let names = Names::new(self, changed, edited);
        // The way back leads to where the first end lies.
        let scope = self.lies(start);
        let (first, last) = (self.end(from, start), self.end(to, end));
        let mut crumbs = Self::crumbs(project, scope);
        crumbs.push(json!({
            "name": format!(
                "{} → {}",
                first["name"].as_str().unwrap_or(""),
                last["name"].as_str().unwrap_or("")
            ),
            "path": "",
        }));
        let mut view = json!({
            "mode": "path",
            "scope": scope,
            "crumbs": crumbs,
            "certain": certain,
            "totals": self.totals(),
            "path": {
                "from": first,
                "to": last,
                "names": symbols,
                "found": false,
                "reversed": false,
                "within": within,
                "steps": 0,
                "places": 0,
                "shown": 0,
                "links": 0,
                "wide": WAY,
                "longer": longer,
                "detours": 0,
                "detours_more": false,
                "aside": 0,
            },
            "children": 0,
            "matching": 0,
            "nodes": [],
            "between": 0,
            "backbone": false,
            "edges": [],
            "focus": null,
        });
        let Some(((far, steps), reversed)) = found else {
            return Some(view);
        };
        let goal = if reversed { &starts } else { &ends };
        // Every place on a shortest way, a step at a time from the far end
        // back, each with what leads to it on such a way.
        let mut levels = vec![Vec::new(); steps + 1];
        levels[steps] = goal
            .iter()
            .copied()
            .filter(|place| far[*place as usize] == steps as u32)
            .collect();
        let mut before: HashMap<u32, Vec<(u32, Flow)>> = HashMap::new();
        let mut links = 0;
        for step in (1..=steps).rev() {
            let (earlier, later) = levels.split_at_mut(step);
            let mut seen = HashSet::new();
            for place in &later[0] {
                let leading = net
                    .beside(*place, false)
                    .into_iter()
                    .filter(|(other, _)| far[*other as usize] == step as u32 - 1)
                    .collect::<Vec<_>>();
                links += leading.len();
                for (other, _) in &leading {
                    if seen.insert(*other) {
                        earlier[step - 1].push(*other);
                    }
                }
                before.insert(*place, leading);
            }
        }
        let places = levels.iter().map(Vec::len).sum::<usize>();
        let tag = |place: u32| net.tag(place);
        let per = WAY.min(FOLLOWED / (steps + 1)).max(1);
        let mut kept = vec![Vec::new(); steps + 1];
        // The far end keeps the places most linked with the step before.
        let mut ending = levels[steps]
            .iter()
            .map(|place| {
                let links = before[place]
                    .iter()
                    .map(|(_, flow)| flow.links)
                    .sum::<u64>();
                (*place, links)
            })
            .collect::<Vec<_>>();
        ending.sort_by(|a, b| b.1.cmp(&a.1).then(tag(a.0).cmp(&tag(b.0))));
        kept[steps] = ending
            .into_iter()
            .take(per)
            .map(|(place, _)| place)
            .collect();
        for step in (1..=steps).rev() {
            // Each place kept keeps the strongest of what leads to it, and
            // the room that is left goes to the most linked of the rest.
            let mut chosen: Vec<u32> = Vec::new();
            let mut strength: HashMap<u32, u64> = HashMap::new();
            for place in &kept[step] {
                let mut best: Option<(u32, u64)> = None;
                for (other, flow) in &before[place] {
                    *strength.entry(*other).or_default() += flow.links;
                    if best.is_none_or(|(known, links)| {
                        flow.links > links || (flow.links == links && tag(*other) < tag(known))
                    }) {
                        best = Some((*other, flow.links));
                    }
                }
                if let Some((other, _)) = best
                    && !chosen.contains(&other)
                {
                    chosen.push(other);
                }
            }
            let mut rest = strength
                .into_iter()
                .filter(|(other, _)| !chosen.contains(other))
                .collect::<Vec<_>>();
            rest.sort_by(|a, b| b.1.cmp(&a.1).then(tag(a.0).cmp(&tag(b.0))));
            let room = per.saturating_sub(chosen.len());
            chosen.extend(rest.into_iter().take(room).map(|(other, _)| other));
            kept[step - 1] = chosen;
        }
        for level in &mut kept {
            level.sort_by(|a, b| tag(*a).cmp(&tag(*b)));
        }
        let mut shown = kept
            .iter()
            .enumerate()
            .flat_map(|(step, level)| level.iter().map(move |place| (*place, step)))
            .collect::<Vec<_>>();
        // The longer ways, where they were asked for: each adds the places
        // of it that are not drawn yet, at the step they have on it. A
        // place that lies on a shortest way keeps the step it has there.
        let direct = shown.len();
        let (detours, detours_more) = if longer == 0 {
            (Vec::new(), false)
        } else {
            let known = shown.iter().map(|(place, _)| *place).collect();
            net.detours(
                if reversed { &ends } else { &starts },
                goal,
                steps,
                longer,
                FOLLOWED.saturating_sub(direct),
                &known,
            )
        };
        let on_shortest = levels
            .iter()
            .enumerate()
            .flat_map(|(step, level)| level.iter().map(move |place| (*place, step)))
            .collect::<HashMap<_, _>>();
        {
            let mut drawn = shown
                .iter()
                .map(|(place, _)| *place)
                .collect::<HashSet<_>>();
            for way in &detours {
                for (at, place) in way.iter().enumerate() {
                    if drawn.insert(*place) {
                        shown.push((*place, on_shortest.get(place).copied().unwrap_or(at)));
                    }
                }
            }
        }
        // A place off every shortest way is neither end of the path.
        let aside = shown
            .iter()
            .map(|(place, _)| !on_shortest.contains_key(place))
            .collect::<Vec<_>>();
        let keys = shown
            .iter()
            .map(|(place, _)| net.key(&names, *place))
            .collect::<Vec<_>>();
        let slot = shown
            .iter()
            .enumerate()
            .map(|(index, (place, _))| (*place, index))
            .collect::<HashMap<_, _>>();
        let mut edges = Vec::new();
        for place in kept.iter().skip(1).flatten() {
            for (other, flow) in &before[place] {
                if let Some(index) = slot.get(other) {
                    edges.push((*index, slot[place], *flow));
                }
            }
        }
        let between = edges.len();
        let edges = ways_drawn(edges, &keys);
        // The links of the longer ways that the shortest ways do not have.
        let mut round: Vec<(usize, usize, Flow)> = Vec::new();
        {
            let mut drawn = edges
                .iter()
                .map(|(source, target, _)| (*source, *target))
                .collect::<HashSet<_>>();
            for way in &detours {
                for pair in way.windows(2) {
                    let (source, target) = (slot[&pair[0]], slot[&pair[1]]);
                    if drawn.insert((source, target))
                        && let Some((_, flow)) = net
                            .beside(pair[0], true)
                            .into_iter()
                            .find(|(other, _)| *other == pair[1])
                    {
                        round.push((source, target, flow));
                    }
                }
            }
        }
        // A file is named by the end of its path that no other drawn shares.
        let paths = keys.iter().map(String::as_str).collect::<Vec<_>>();
        let node = |index: usize| {
            let (place, step) = shown[index];
            let mut node = net.node(&names, place, &keys[index], &paths);
            for (key, value) in [
                ("start", json!(!aside[index] && step == 0)),
                ("end", json!(!aside[index] && step == steps)),
                ("step", json!(step)),
            ] {
                node[key] = value;
            }
            if aside[index] {
                node["detour"] = json!(true);
            }
            node
        };
        // A place of the way as a row beside a selected one.
        let row = |other: u32, flow: Flow, user: u32, used: u32| {
            net.row(&names, &paths, other, flow, Some((user, used)))
        };
        let focus = focus
            .and_then(|path| keys.iter().position(|key| key == path))
            .map(|index| {
                let (place, step) = shown[index];
                // What the place leads to on the way, and what leads to it:
                // those drawn, and how many more lie on such ways.
                let onward = |level: &[u32]| {
                    level
                        .iter()
                        .filter_map(|other| {
                            before[other]
                                .iter()
                                .find(|(known, _)| *known == place)
                                .map(|(_, flow)| (*other, *flow))
                        })
                        .collect::<Vec<_>>()
                };
                let (next, further) = if step < steps {
                    (onward(&kept[step + 1]), onward(&levels[step + 1]).len())
                } else {
                    (Vec::new(), 0)
                };
                let leading = before.get(&place).map(Vec::as_slice).unwrap_or_default();
                let prior = leading
                    .iter()
                    .filter(|(other, _)| slot.contains_key(other))
                    .copied()
                    .collect::<Vec<_>>();
                let used = next.iter().map(|(other, _)| *other).collect();
                let mut focus = node(index);
                focus["inside"] = json!(true);
                focus["uses"] = next
                    .iter()
                    .map(|(other, flow)| row(*other, *flow, place, *other))
                    .collect();
                focus["uses_more"] = json!(further - next.len());
                focus["used_by"] = prior
                    .iter()
                    .map(|(other, flow)| row(*other, *flow, *other, place))
                    .collect();
                focus["used_by_more"] = json!(leading.len() - prior.len());
                focus["kinds"] = net.kinds(&[place], &used);
                // Among longer ways a place leads on along every link that
                // is drawn from it, and is led to along every one drawn to
                // it: those of the ways round too.
                if longer > 0 {
                    let rows = |outgoing: bool| {
                        edges
                            .iter()
                            .chain(&round)
                            .filter(|(source, target, _)| {
                                index == if outgoing { *source } else { *target }
                            })
                            .map(|(source, target, flow)| {
                                let other = if outgoing { *target } else { *source };
                                row(shown[other].0, *flow, shown[*source].0, shown[*target].0)
                            })
                            .collect::<Vec<_>>()
                    };
                    focus["uses"] = json!(rows(true));
                    focus["used_by"] = json!(rows(false));
                    focus["uses_more"] = json!(0);
                    focus["used_by_more"] = json!(0);
                }
                focus
            });
        for (key, value) in [
            ("found", json!(true)),
            ("reversed", json!(reversed)),
            ("steps", json!(steps)),
            ("places", json!(places)),
            // Of the places on the shortest ways, those drawn; the
            // places that only a longer way has are counted apart.
            (
                "shown",
                json!(aside.iter().filter(|aside| !**aside).count()),
            ),
            ("links", json!(links)),
            ("wide", json!(per)),
            ("detours", json!(detours.len())),
            ("detours_more", json!(detours_more)),
            ("aside", json!(aside.iter().filter(|aside| **aside).count())),
        ] {
            view["path"][key] = value;
        }
        view["children"] = json!(shown.len());
        view["matching"] = json!(shown.len());
        view["nodes"] = (0..shown.len()).map(node).collect();
        view["between"] = json!(between);
        view["edges"] = edges
            .iter()
            .map(|(source, target, flow)| {
                net.edge_json(
                    &[shown[*source].0],
                    &[shown[*target].0],
                    &keys[*source],
                    &keys[*target],
                    *flow,
                )
            })
            .chain(round.iter().map(|(source, target, flow)| {
                let mut edge = net.edge_json(
                    &[shown[*source].0],
                    &[shown[*target].0],
                    &keys[*source],
                    &keys[*target],
                    *flow,
                );
                edge["detour"] = json!(true);
                edge
            }))
            .collect();
        view["focus"] = json!(focus);
        Some(view)
    }
    /// What two places share: what both use, or what uses both. Either
    /// place is a folder, a file or a symbol, and what the two stand for is
    /// told as for a path: files, or, where one of them is a symbol,
    /// symbols. Each of the two is one box of the picture, whatever it
    /// holds.
    ///
    /// Both use a place where a chain of links leads to it from each of the
    /// two without passing through the other: what one of them reaches only
    /// through the other is what the path between them shows. A place uses
    /// both where such chains lead from it to each of them. Both kinds are
    /// counted and one is drawn: `asked` says whether what uses both or
    /// what both use; where it does not say, what both use is drawn, unless
    /// only the other kind has anything.
    ///
    /// The places of a kind stand in the order of nearness: the fewest
    /// links to the two taken together first, and among those as near the
    /// most linked. `filter` keeps those whose name holds it, a file being
    /// named by its path and a symbol as it is shown. Of what is left the
    /// picture passes over as many as `asked` says and shows the next
    /// `SHARED`, so that one picture after another shows them all; a
    /// beginning past the last of them is the last. Each place is drawn
    /// with one shortest way between it and either of the two, the
    /// strongest where several are as short. Where the ways fill the
    /// picture, fewer places are shown, and the next picture begins after
    /// those.
    ///
    /// Where one of the two lies in the other, or there is nothing of the
    /// kind, the answer says so and draws nothing. `changed`, `edited` and
    /// `focus` are as for a path. `None` where a place names nothing in the
    /// graph.
    #[allow(clippy::too_many_arguments)]
    pub fn shared(
        &self,
        project: &str,
        asked: Sharing,
        changed: &[(String, char)],
        edited: &dyn Fn(&str) -> Option<Edited>,
        focus: Option<&str>,
        certain: bool,
        filter: &str,
    ) -> Option<Value> {
        let Sharing {
            from,
            to,
            users,
            skip,
        } = asked;
        let ends = [self.reach(from)?, self.reach(to)?];
        let symbols = ends.iter().any(|end| matches!(end, Reach::Symbol(_)));
        let net = Net::new(self, symbols, certain);
        let held = [net.members(ends[0]), net.members(ends[1])];
        // Which of the two a place belongs to, the first or the second; 0
        // for every other place.
        let mut owner = vec![0u8; net.count()];
        let mut within = false;
        for (end, places) in held.iter().enumerate() {
            for place in places {
                within |= owner[*place as usize] != 0;
                owner[*place as usize] = end as u8 + 1;
            }
        }
        let names = Names::new(self, changed, edited);
        let scope = self.lies(ends[0]);
        let told = [self.end(from, ends[0]), self.end(to, ends[1])];
        let mut crumbs = Self::crumbs(project, scope);
        crumbs.push(json!({
            "name": format!(
                "{} and {}",
                told[0]["name"].as_str().unwrap_or(""),
                told[1]["name"].as_str().unwrap_or("")
            ),
            "path": "",
        }));
        let side = |users: bool| if users { "uses_both" } else { "both_use" };
        let mut view = json!({
            "mode": "shared",
            "scope": scope,
            "crumbs": crumbs,
            "certain": certain,
            "totals": self.totals(),
            "shared": {
                "from": told[0],
                "to": told[1],
                "names": symbols,
                "within": within,
                "side": side(users == Some(true)),
                "both_use": 0,
                "uses_both": 0,
                "matching": 0,
                "skip": 0,
                "shown": 0,
                "wide": SHARED,
            },
            "children": 0,
            "matching": 0,
            "nodes": [],
            "between": 0,
            "backbone": false,
            "edges": [],
            "focus": null,
        });
        // One of the two that lies in the other is not apart from it.
        if within {
            return Some(view);
        }
        // How many links a place lies from one of the two on the ways walked
        // from it: none for what that one holds itself, and nothing for what
        // the other holds, which no way passes.
        let level = |far: &[Vec<u32>; 2], end: usize, place: u32| match owner[place as usize] {
            0 => Some(far[end][place as usize]).filter(|far| *far != u32::MAX),
            own if own as usize == end + 1 => Some(0),
            _ => None,
        };
        // What leads to a place on a shortest way from one of the two, with
        // the links between them. For what both use that is what uses the
        // place; for what uses both, what the place uses.
        let toward = |far: &[Vec<u32>; 2], end: usize, place: u32, outgoing: bool| {
            let step = far[end][place as usize];
            net.beside(place, !outgoing)
                .into_iter()
                .filter(|(other, _)| level(far, end, *other).is_some_and(|near| near + 1 == step))
                .collect::<Vec<_>>()
        };
        // One kind of what the two share: how far every place lies from
        // each of them, along the links for what both use and against them
        // for what uses both, and every place that both are linked with so,
        // with the links to the two taken together.
        let reached = |outgoing: bool| {
            let far = [
                net.far(&held[0], &owner, outgoing),
                net.far(&held[1], &owner, outgoing),
            ];
            let near = (0..net.count() as u32)
                .filter(|place| far.iter().all(|far| far[*place as usize] != u32::MAX))
                .map(|place| (far[0][place as usize] + far[1][place as usize], place))
                .collect::<Vec<_>>();
            (far, near)
        };
        let (used, using) = (reached(true), reached(false));
        // What both use is drawn, unless what uses both was asked for or is
        // all there is.
        let users = users.unwrap_or(used.1.is_empty() && !using.1.is_empty());
        view["shared"]["side"] = json!(side(users));
        view["shared"]["both_use"] = json!(used.1.len());
        view["shared"]["uses_both"] = json!(using.1.len());
        let outgoing = !users;
        let (far, mut near) = if users { using } else { used };
        // Of that kind, only the places whose name holds what the filter
        // says.
        let needle = filter.trim().to_lowercase();
        if !needle.is_empty() {
            near.retain(|(_, place)| net.named(*place, &needle));
        }
        let matching = near.len();
        // A beginning past the last place is the last place.
        let skip = skip.min(matching.saturating_sub(1));
        view["shared"]["matching"] = json!(matching);
        view["shared"]["skip"] = json!(skip);
        // Only the nearest are looked at more closely: those as near as the
        // last that has room in this picture.
        if near.len() > skip + SHARED {
            let cut = near.select_nth_unstable(skip + SHARED - 1).1.0;
            near.retain(|(links, _)| *links <= cut);
        }
        // Among those as near, the most linked with what leads to them.
        let mut near = near
            .into_iter()
            .map(|(links, place)| {
                let strength = (0..2)
                    .flat_map(|end| toward(&far, end, place, outgoing))
                    .map(|(_, flow)| flow.links)
                    .sum::<u64>();
                (links, strength, place)
            })
            .collect::<Vec<_>>();
        near.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(b.1.cmp(&a.1))
                .then(net.tag(a.2).cmp(&net.tag(b.2)))
        });
        // The boxes on the ways drawn from each of the two, the shared
        // places among them: each with what leads to it on its way, none
        // where that is the one of the two itself, and the links between
        // them.
        let mut on: [HashMap<u32, (Option<u32>, Flow)>; 2] = [HashMap::new(), HashMap::new()];
        let mut boxes: HashSet<u32> = HashSet::new();
        let mut shown = Vec::new();
        for (_, _, place) in near.into_iter().skip(skip).take(SHARED) {
            // The way to it from each of the two, back from it a step at a
            // time: the strongest of what leads on, and no further than a
            // box that such a way is drawn through already.
            let mut ways = [Vec::new(), Vec::new()];
            for (end, way) in ways.iter_mut().enumerate() {
                let mut at = place;
                while !on[end].contains_key(&at) {
                    let leading = toward(&far, end, at, outgoing);
                    if far[end][at as usize] == 1 {
                        // What the one of the two holds leads here, and
                        // all its links are one link of the picture.
                        let whole = leading
                            .iter()
                            .fold(Flow::default(), |whole, (_, flow)| Flow {
                                links: whole.links + flow.links,
                                guessed: whole.guessed + flow.guessed,
                            });
                        way.push((at, None, whole));
                        break;
                    }
                    let strongest = leading.into_iter().max_by(|a, b| {
                        a.1.links
                            .cmp(&b.1.links)
                            .then(net.tag(b.0).cmp(&net.tag(a.0)))
                    });
                    let Some((before, flow)) = strongest else {
                        break;
                    };
                    way.push((at, Some(before), flow));
                    at = before;
                }
            }
            let new = ways
                .iter()
                .flatten()
                .map(|(at, ..)| *at)
                .filter(|at| !boxes.contains(at))
                .collect::<HashSet<_>>()
                .len();
            // A picture that is full shows no more: the nearest stand.
            if !shown.is_empty() && boxes.len() + new > SIDE {
                break;
            }
            for (end, way) in ways.into_iter().enumerate() {
                for (at, before, flow) in way {
                    boxes.insert(at);
                    on[end].insert(at, (before, flow));
                }
            }
            shown.push(place);
        }
        view["shared"]["shown"] = json!(shown.len());
        if shown.is_empty() {
            return Some(view);
        }
        // The two come first, then the rest by where they lie.
        let mut drawn = boxes.into_iter().collect::<Vec<_>>();
        drawn.sort_by(|a, b| net.tag(*a).cmp(&net.tag(*b)));
        let mut keys = told
            .iter()
            .map(|end| end["path"].as_str().unwrap_or("").to_owned())
            .collect::<Vec<_>>();
        keys.extend(drawn.iter().map(|place| net.key(&names, *place)));
        let slot = drawn
            .iter()
            .enumerate()
            .map(|(index, place)| (*place, index + 2))
            .collect::<HashMap<_, _>>();
        // The links of the picture are those of the ways, each once: a way
        // from the first of the two and a way from the second may share one.
        let direction = |near, far| if outgoing { (near, far) } else { (far, near) };
        let mut links: HashMap<(usize, usize), Flow> = HashMap::new();
        for (end, way) in on.iter().enumerate() {
            for (at, (before, flow)) in way {
                let near = before.map_or(end, |before| slot[&before]);
                links.insert(direction(near, slot[at]), *flow);
            }
        }
        let edges = links
            .into_iter()
            .map(|((from, to), flow)| (from, to, flow))
            .collect::<Vec<_>>();
        let between = edges.len();
        let edges = ways_drawn(edges, &keys);
        // A file is named by the end of its path that no other drawn shares.
        let paths = keys.iter().map(String::as_str).collect::<Vec<_>>();
        // What a box stands for: one of the two is all it holds.
        let members = |index: usize| match index.checked_sub(2) {
            Some(place) => vec![drawn[place]],
            None => held[index].clone(),
        };
        // The one place a box is, where it is one: a folder is none, nor is
        // a file among symbols.
        let one = |index: usize| match (index.checked_sub(2), ends.get(index)) {
            (Some(place), _) => Some(drawn[place]),
            (None, Some(Reach::Symbol(at))) => Some(*at),
            (None, Some(Reach::File(file))) if !symbols => Some(*file),
            _ => None,
        };
        let node = |index: usize| {
            let key = keys[index].as_str();
            let mut node = match (one(index), ends.get(index)) {
                (Some(place), _) => net.node(&names, place, key, &paths),
                // A file among symbols, or a folder: what it holds, and what
                // leads out of it and into it.
                (None, end) => {
                    let folder = matches!(end, Some(Reach::Folder(_)));
                    let (mut files, mut inside, mut altered, mut status) = (0u64, 0u64, 0u64, None);
                    for (id, file) in self.files.iter().enumerate() {
                        let own = if folder {
                            file.strip_prefix(key)
                                .is_some_and(|rest| rest.starts_with('/'))
                        } else {
                            file == key
                        };
                        if own {
                            files += 1;
                            inside += self.holds(id as u32);
                            if let Some(letter) = names.letter(file) {
                                altered += 1;
                                status = Some(letter);
                            }
                        }
                    }
                    let apart = |other: u32| owner[other as usize] as usize != index + 1;
                    json!({
                        "path": key,
                        "name": if folder { name(key) } else { label(key, &paths) },
                        "folder": folder,
                        "files": files,
                        "symbols": inside,
                        "within": 0,
                        "changed": altered,
                        "status": status.filter(|_| !folder).map(String::from),
                        "out": net.crossing(&held[index], true, &apart),
                        "in": net.crossing(&held[index], false, &apart),
                    })
                }
            };
            // What the two share says how many links each of them is away.
            let shares = index
                .checked_sub(2)
                .map(|place| drawn[place])
                .filter(|place| shown.contains(place))
                .map(|place| [far[0][place as usize], far[1][place as usize]]);
            node["start"] = json!(index == 0);
            node["end"] = json!(index == 1);
            node["shares"] = json!(shares);
            node
        };
        // A box as a row beside a selected one.
        let row = |other: usize, flow: Flow, user: usize, used: usize| match one(other) {
            Some(place) => net.row(&names, &paths, place, flow, one(user).zip(one(used))),
            None => {
                let told = node(other);
                json!({
                    "path": told["path"],
                    "name": told["name"],
                    "folder": told["folder"],
                    "inside": true,
                    "links": flow.links,
                    "guessed": flow.guessed,
                    "changed": told["changed"].as_u64().unwrap_or(0) > 0,
                })
            }
        };
        let focus = focus
            .and_then(|path| keys.iter().position(|key| key == path))
            .map(|index| {
                // What the box leads to on the ways drawn, and what leads to
                // it.
                let next = edges
                    .iter()
                    .filter(|edge| edge.0 == index)
                    .collect::<Vec<_>>();
                let prior = edges
                    .iter()
                    .filter(|edge| edge.1 == index)
                    .collect::<Vec<_>>();
                let used = next
                    .iter()
                    .flat_map(|edge| members(edge.1))
                    .collect::<HashSet<_>>();
                let mut focus = node(index);
                focus["inside"] = json!(true);
                focus["uses"] = next
                    .iter()
                    .map(|edge| row(edge.1, edge.2, index, edge.1))
                    .collect();
                focus["uses_more"] = json!(0);
                focus["used_by"] = prior
                    .iter()
                    .map(|edge| row(edge.0, edge.2, edge.0, index))
                    .collect();
                focus["used_by_more"] = json!(0);
                focus["kinds"] = net.kinds(&members(index), &used);
                if focus["shares"].is_array() {
                    // Keep each end's chosen route. The combined edges can
                    // also form routes drawn for other shared places.
                    let mut shared_edges = BTreeSet::new();
                    for (end, way) in on.iter().enumerate() {
                        let mut at = drawn[index - 2];
                        while let Some((before, _)) = way.get(&at) {
                            let near = before.map_or(end, |before| slot[&before]);
                            shared_edges.insert(direction(near, slot[&at]));
                            let Some(before) = before else { break };
                            at = *before;
                        }
                    }
                    focus["shared_edges"] = shared_edges
                        .into_iter()
                        .map(|(from, to)| json!([keys[from], keys[to]]))
                        .collect();
                }
                focus
            });
        view["children"] = json!(keys.len());
        view["matching"] = json!(keys.len());
        view["nodes"] = (0..keys.len()).map(node).collect();
        view["between"] = json!(between);
        view["edges"] = edges
            .iter()
            .map(|(source, target, flow)| {
                let from_places = members(*source);
                let to_places = members(*target);
                net.edge_json(
                    &from_places,
                    &to_places,
                    &keys[*source],
                    &keys[*target],
                    *flow,
                )
            })
            .collect();
        view["focus"] = json!(focus);
        Some(view)
    }
}
/// One end of a link drawn in an open file: a symbol of the file, or a file
/// around it.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum End {
    Symbol(u32),
    File(u32),
}
/// What an open file has with one file around it.
#[derive(Clone, Copy, Default)]
pub(super) struct Near {
    /// Links through which that file uses this one.
    pub(super) uses: u64,
    /// Links through which this file uses that one.
    pub(super) used: u64,
}
impl Near {
    pub(super) fn role(&self) -> &'static str {
        match (self.uses > 0, self.used > 0) {
            (true, true) => "both",
            (true, false) => "uses",
            _ => "used",
        }
    }
}
/// A symbol listed beside a selected one: how it is linked, or, for the
/// symbols of a file around, through how many links.
pub(super) struct Told {
    pub(super) symbol: u32,
    pub(super) relation: Option<u16>,
    pub(super) links: u64,
    pub(super) guessed: u64,
}
/// What a change touched in one file, by the place of each symbol among
/// the file's: Git's letter for a symbol whose own lines changed, and how
/// many symbols changed inside it.
pub(super) struct Marks {
    pub(super) status: Vec<Option<char>>,
    pub(super) inside: Vec<u64>,
    /// Places between symbols where lines were only removed.
    pub(super) between: u64,
}
/// The symbols that start on one line, and the lines that are theirs: from
/// what leads to them to where their indentation ends.
pub(super) struct Group {
    pub(super) start: u32,
    pub(super) lead: u32,
    pub(super) end: u32,
    pub(super) from: usize,
    pub(super) to: usize,
}
