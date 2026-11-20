use super::*;

impl Index {
    pub(crate) fn files(&self) -> &[String] {
        &self.files
    }

    pub(crate) fn backend(&self) -> Option<&str> {
        self.backend.as_deref()
    }

    pub(crate) fn ir_db_path(&self) -> Option<&Path> {
        self.ir_db_path.as_deref()
    }

    pub(crate) fn pairs(&self) -> &[(u32, u32, Pair)] {
        &self.pairs
    }

    pub(crate) fn symbol_entries(&self) -> &[Symbol] {
        &self.symbols
    }

    pub(crate) fn uses(&self) -> &[Use] {
        &self.uses
    }

    pub(crate) fn kind_name(&self, id: u16) -> &str {
        self.kinds
            .get(id as usize)
            .map(String::as_str)
            .unwrap_or("")
    }

    /// Sort aggregated link kind counts by descending count, then ascending relation name.
    pub(super) fn kinds_json(&self, kinds: &BTreeMap<u16, u64>) -> Vec<Value> {
        let mut sorted = kinds
            .iter()
            .filter(|(_, count)| **count > 0)
            .map(|(&kind, &count)| (self.kinds[kind as usize].as_str(), count))
            .collect::<Vec<_>>();
        sorted.sort_by(|(a, left), (b, right)| right.cmp(left).then(a.cmp(b)));
        sorted
            .into_iter()
            .map(|(kind, count)| json!({"kind": kind, "links": count}))
            .collect()
    }

    /// Serialize one drawn graph map edge with aggregated dependency kinds.
    pub(super) fn edge_json(&self, from: &str, to: &str, edge: &EdgeFlow) -> Value {
        json!({
            "from": from,
            "to": to,
            "links": edge.flow.links,
            "guessed": edge.flow.guessed,
            "kinds": self.kinds_json(&edge.kinds),
        })
    }

    pub(super) fn collect_symbol_kinds(
        &self,
        from: u32,
        certain: bool,
        kinds: &mut BTreeMap<u16, u64>,
        matches: impl Fn(u32) -> bool,
    ) {
        for (_, other, kind, _) in self.wired(from, true, certain) {
            if matches(*other) {
                *kinds.entry(*kind).or_default() += 1;
            }
        }
    }

    pub(crate) fn file_id(&self, path: &str) -> Option<u32> {
        self.files
            .iter()
            .position(|file| file == path)
            .map(|at| at as u32)
    }

    pub(crate) fn symbol_label(&self, at: u32) -> &str {
        self.label(at)
    }

    pub(crate) fn file_range(&self, file: u32) -> std::ops::Range<u32> {
        self.range(file)
    }

    /// One folder of the map, or, where the scope names a file, that file.
    /// `certain` leaves out what the tool inferred, `filter` keeps the
    /// children whose name contains it, and `focus` adds what one child, or
    /// one place outside the folder, uses and is used by. A child that is
    /// the focus is drawn whatever the filter and the limits say, and the
    /// view says so (`beyond`) where they would have left it out.
    pub fn view(
        &self,
        project: &str,
        scope: &str,
        focus: Option<&str>,
        certain: bool,
        filter: &str,
    ) -> Value {
        self.view_with(project, scope, &[], focus, certain, filter)
    }

    /// Whole-project map view with optional breadth-first unfolding (`whole == true`)
    /// so all modules, files and cross-file edges display in one view.
    #[allow(clippy::too_many_arguments)]
    pub fn view_with_whole(
        &self,
        project: &str,
        scope: &str,
        spread: &[String],
        focus: Option<&str>,
        certain: bool,
        filter: &str,
        whole: bool,
    ) -> Value {
        self.view_with_whole_options(
            project, scope, spread, focus, certain, filter, whole, 0, None, None,
        )
    }

    /// Map view supporting server-side paging (`offset`, `limit`) and hierarchy expansion `depth`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn view_with_whole_options(
        &self,
        project: &str,
        scope: &str,
        spread: &[String],
        focus: Option<&str>,
        certain: bool,
        filter: &str,
        whole: bool,
        offset: usize,
        limit: Option<usize>,
        depth: Option<usize>,
    ) -> Value {
        if whole && scope.trim_matches('/').is_empty() && self.opened(scope).is_none() {
            let expanded = self.whole_spread_with_depth(WHOLE_FOLDERS, depth);
            let mut view = self.view_with_limits_opts(
                project,
                scope,
                &expanded,
                focus,
                certain,
                filter,
                WHOLE_LIMITS,
                offset,
                limit,
                depth,
            );
            view["whole"] = json!(true);
            return view;
        }
        let owned_spread;
        let effective_spread = if spread.is_empty()
            && let Some(max_depth) = depth.filter(|&d| d > 1)
            && self.opened(scope).is_none()
        {
            owned_spread = self.spread_under_scope(scope.trim_matches('/'), max_depth - 1, SPREAD);
            &owned_spread
        } else {
            spread
        };
        let mut view = self.view_with_limits_opts(
            project,
            scope,
            effective_spread,
            focus,
            certain,
            filter,
            NORMAL_LIMITS,
            offset,
            limit,
            depth,
        );
        view["whole"] = json!(whole);
        view
    }

    pub(super) fn spread_under_scope(
        &self,
        scope: &str,
        extra_levels: usize,
        limit: usize,
    ) -> Vec<String> {
        let scope_depth = if scope.is_empty() {
            0
        } else {
            scope.split('/').count()
        };
        let mut folders: BTreeSet<String> = BTreeSet::new();
        for file in &self.files {
            if !scope.is_empty()
                && !file
                    .strip_prefix(scope)
                    .is_some_and(|rest| rest.starts_with('/'))
            {
                continue;
            }
            for (at, ch) in file.char_indices() {
                if ch == '/' {
                    let folder = &file[..at];
                    let segs = folder.split('/').count();
                    if segs > scope_depth && segs <= scope_depth + extra_levels {
                        folders.insert(folder.to_owned());
                        if folders.len() >= limit {
                            return folders.into_iter().collect();
                        }
                    }
                }
            }
        }
        folders.into_iter().collect()
    }

    pub(super) fn whole_spread_with_depth(
        &self,
        limit: usize,
        max_depth: Option<usize>,
    ) -> Vec<String> {
        let mut file_weights = vec![0u64; self.files.len()];
        for (from, to, pair) in &self.pairs {
            let w = pair.weight(false);
            if let Some(slot) = file_weights.get_mut(*from as usize) {
                *slot += w;
            }
            if let Some(slot) = file_weights.get_mut(*to as usize) {
                *slot += w;
            }
        }
        let mut folder_stats: HashMap<&str, (u64, usize)> = HashMap::new();
        for (idx, file) in self.files.iter().enumerate() {
            let w = file_weights[idx];
            for (at, ch) in file.char_indices() {
                if ch == '/' {
                    let folder = &file[..at];
                    if let Some(d) = max_depth
                        && folder.split('/').count() > d
                    {
                        continue;
                    }
                    let entry = folder_stats.entry(folder).or_default();
                    entry.0 += w;
                    entry.1 += 1;
                }
            }
        }
        let mut children_of: HashMap<&str, Vec<&str>> = HashMap::new();
        for &folder in folder_stats.keys() {
            children_of.entry(holder(folder)).or_default().push(folder);
        }
        for kids in children_of.values_mut() {
            kids.sort_by(|a, b| {
                let sa = folder_stats.get(a).copied().unwrap_or_default();
                let sb = folder_stats.get(b).copied().unwrap_or_default();
                sb.0.cmp(&sa.0).then(sb.1.cmp(&sa.1)).then(a.cmp(b))
            });
        }
        let mut result = Vec::new();
        let mut queue = std::collections::VecDeque::from([""]);
        while let Some(parent) = queue.pop_front() {
            if let Some(kids) = children_of.get(parent) {
                for &kid in kids {
                    if result.len() >= limit {
                        return result;
                    }
                    result.push(kid.to_owned());
                    queue.push_back(kid);
                }
            }
        }
        result
    }

    /// One folder of the map with some of the folders in it spread out:
    /// such a folder is not drawn as one place but as the places it holds,
    /// among the others, so what is around it stays in the picture. A
    /// folder inside a spread-out one may be spread out in turn. What a
    /// request names that the picture does not reach is passed over.
    pub fn view_with(
        &self,
        project: &str,
        scope: &str,
        spread: &[String],
        focus: Option<&str>,
        certain: bool,
        filter: &str,
    ) -> Value {
        self.view_with_limits_opts(
            project,
            scope,
            spread,
            focus,
            certain,
            filter,
            NORMAL_LIMITS,
            0,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn view_with_limits_opts(
        &self,
        project: &str,
        scope: &str,
        spread: &[String],
        focus: Option<&str>,
        certain: bool,
        filter: &str,
        limits: ViewLimits,
        offset: usize,
        limit: Option<usize>,
        depth: Option<usize>,
    ) -> Value {
        if let Some(file) = self.opened(scope) {
            return self.file(project, file, &HashMap::new(), None, focus, certain, filter);
        }
        let scope = scope.trim_matches('/');
        let scope = if self.folder(scope) { scope } else { "" };
        let spread = spread
            .iter()
            .map(|folder| folder.trim_matches('/'))
            .collect::<HashSet<_>>();
        let mut tallies: BTreeMap<&str, Tally> = BTreeMap::new();
        for (id, file) in self.files.iter().enumerate() {
            let (key, inside) = place(scope, &spread, file);
            let tally = tallies.entry(key).or_default();
            tally.files += 1;
            tally.inside = inside;
            tally.folder |= key.len() < file.len();
            tally.symbols += self.holds(id as u32);
        }
        let mut flows: HashMap<(&str, &str), EdgeFlow> = HashMap::new();
        let mut kinds: BTreeMap<u16, u64> = BTreeMap::new();
        for (from, to, pair) in &self.pairs {
            let weight = pair.weight(certain);
            if weight == 0 {
                continue;
            }
            let (source, _) = place(scope, &spread, &self.files[*from as usize]);
            let (target, _) = place(scope, &spread, &self.files[*to as usize]);
            if source == target {
                if let Some(tally) = tallies.get_mut(source) {
                    tally.within += weight;
                }
                continue;
            }
            if let Some(tally) = tallies.get_mut(source) {
                tally.out += weight;
            }
            if let Some(tally) = tallies.get_mut(target) {
                tally.into += weight;
            }
            flows
                .entry((source, target))
                .or_default()
                .add_pair(pair, certain);
            if focus == Some(source) {
                add_kind_counts(&mut kinds, pair.kinds_for(certain));
            }
        }
        let needle = filter.trim().to_lowercase();
        let mut children = tallies
            .iter()
            .filter(|(key, tally)| {
                tally.inside && (needle.is_empty() || name(key).to_lowercase().contains(&needle))
            })
            .collect::<Vec<_>>();
        let total = tallies.values().filter(|tally| tally.inside).count();
        let matching = children.len();
        let busiest = |(left, a): &(&&str, &Tally), (right, b): &(&&str, &Tally)| {
            (b.out + b.into + b.within)
                .cmp(&(a.out + a.into + a.within))
                .then(left.cmp(right))
        };
        children.sort_by(busiest);
        // Each folder shows the most linked of what it holds: the open one
        // `SHOWN` of them, a spread-out one `HELD`. What was spread out was
        // asked for, so it is drawn first, and where the picture is full the
        // open folder shows fewer, though never less than `OWN`.
        let mut shown = HashSet::new();
        let mut beyond = false;
        let is_paginated = offset > 0 || limit.is_some();
        if is_paginated {
            let cap = limit.unwrap_or(limits.spread).max(1);
            if let Some(focus_path) = focus
                && let Some(pos) = children.iter().position(|(k, _)| **k == focus_path)
            {
                let was_beyond = pos >= cap;
                let item = children.remove(pos);
                children.insert(0, item);
                if offset == 0 && was_beyond {
                    beyond = true;
                }
            }
            let start = offset.min(matching);
            let end = start.saturating_add(cap).min(matching);
            children = children[start..end].to_vec();
            for (key, _) in &children {
                shown.insert(**key);
            }
        } else {
            let mut drawn: HashMap<&str, usize> = HashMap::new();
            for framed in [true, false] {
                let full = if framed {
                    limits.spread.saturating_sub(limits.own)
                } else {
                    limits.spread
                };
                for (key, _) in &children {
                    let folder = holder(key);
                    if (folder != scope) != framed || shown.len() >= full {
                        continue;
                    }
                    let count = drawn.entry(folder).or_default();
                    if *count < if framed { limits.held } else { limits.shown } {
                        *count += 1;
                        shown.insert(**key);
                    }
                }
            }
            children.retain(|(key, _)| shown.contains(**key));
            // The selected place is drawn whatever the limits and the filter
            // say: it is selected to be seen, and a search of the project leads
            // to places that are not among the most linked of their folder.
            if let Some(place) = focus.and_then(|path| tallies.get_key_value(path))
                && place.1.inside
                && shown.insert(*place.0)
            {
                children.push(place);
                children.sort_by(busiest);
                beyond = true;
            }
        }
        // The folders that are spread out and hold something drawn, each
        // with how many places it holds and how many of them are drawn.
        let mut frames: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        for (key, _) in &children {
            let mut folder = holder(key);
            let mut direct = 1;
            while folder != scope {
                frames.entry(folder).or_default().1 += direct;
                direct = 0;
                folder = holder(folder);
            }
        }
        for (key, tally) in &tallies {
            if tally.inside
                && let Some(frame) = frames.get_mut(holder(key))
            {
                frame.0 += 1;
            }
        }
        let mut cross_page_edges: Vec<Value> = if is_paginated {
            let mut cross: Vec<(&str, &str, u64)> = flows
                .iter()
                .filter_map(|((source, target), edge)| {
                    let s_in = shown.contains(source);
                    let t_in = shown.contains(target);
                    (s_in != t_in
                        && tallies.get(source).is_some_and(|t| t.inside)
                        && tallies.get(target).is_some_and(|t| t.inside))
                    .then_some((*source, *target, edge.flow.links))
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
        cross_page_edges.shrink_to_fit();
        let mut edges = flows
            .iter()
            .filter(|((source, target), _)| shown.contains(source) && shown.contains(target))
            .collect::<Vec<_>>();
        let between = edges.len();
        edges.sort_by(|((a, b), left), ((c, d), right)| {
            right
                .flow
                .links
                .cmp(&left.flow.links)
                .then((a, b).cmp(&(c, d)))
        });
        // Where nearly everything is linked to everything, all links drawn at
        // once say nothing. Each child then keeps its strongest uses; the
        // focus still lists every link.
        let backbone = between > DENSE * shown.len();
        if backbone {
            let mut kept: HashMap<&str, usize> = HashMap::new();
            edges.retain(|((source, _), _)| {
                let count = kept.entry(*source).or_default();
                *count += 1;
                *count <= STRONGEST
            });
        }
        edges.truncate(limits.drawn);
        let side = |outgoing: bool| {
            let Some(focus) = focus else {
                return (Vec::new(), 0);
            };
            let mut rows = flows
                .iter()
                .filter_map(|((source, target), edge)| {
                    let other = if outgoing {
                        (*source == focus).then_some(*target)
                    } else {
                        (*target == focus).then_some(*source)
                    }?;
                    Some((other, edge.flow))
                })
                .collect::<Vec<_>>();
            rows.sort_by(|(a, left), (b, right)| right.links.cmp(&left.links).then(a.cmp(b)));
            let more = rows.len().saturating_sub(LISTED);
            rows.truncate(LISTED);
            let rows = rows
                .into_iter()
                .map(|(path, flow)| {
                    let tally = tallies.get(path);
                    json!({
                        "path": path,
                        "name": if tally.is_some_and(|tally| tally.inside) { name(path) } else { path },
                        "folder": tally.is_some_and(|tally| tally.folder),
                        "inside": tally.is_some_and(|tally| tally.inside),
                        "links": flow.links,
                        "guessed": flow.guessed,
                    })
                })
                .collect::<Vec<_>>();
            (rows, more)
        };
        let focused = focus.and_then(|path| tallies.get(path).map(|tally| (path, tally)));
        let focus = focused.map(|(path, tally)| {
            let (uses, uses_more) = side(true);
            let (used_by, used_by_more) = side(false);
            json!({
                "path": path,
                "name": name(path),
                "folder": tally.folder,
                "inside": tally.inside,
                "files": tally.files,
                "symbols": tally.symbols,
                "within": tally.within,
                "out": tally.out,
                "in": tally.into,
                "uses": uses,
                "uses_more": uses_more,
                "used_by": used_by,
                "used_by_more": used_by_more,
                "kinds": self.kinds_json(&kinds),
            })
        });
        let mut out = json!({
            "scope": scope,
            "crumbs": Self::crumbs(project, scope),
            "certain": certain,
            "totals": self.totals(),
            "children": total,
            "matching": matching,
            "beyond": beyond,
            "nodes": children.iter().map(|(key, tally)| json!({
                "path": key,
                "name": name(key),
                "folder": tally.folder,
                "holder": Some(holder(key)).filter(|folder| *folder != scope),
                "files": tally.files,
                "symbols": tally.symbols,
                "within": tally.within,
                "out": tally.out,
                "in": tally.into,
            })).collect::<Vec<_>>(),
            "frames": frames.iter().map(|(path, (places, drawn))| json!({
                "path": path,
                "name": name(path),
                "holder": Some(holder(path)).filter(|folder| *folder != scope),
                "places": places,
                "drawn": drawn,
            })).collect::<Vec<_>>(),
            "between": between,
            "backbone": backbone,
            "edges": edges.iter().map(|((source, target), edge)| {
                self.edge_json(source, target, edge)
            }).collect::<Vec<_>>(),
            "focus": focus,
            "paging": {
                "offset": offset,
                "limit": limit,
                "total": matching,
                "returned": children.len(),
                "has_more": !children.is_empty() && offset.saturating_add(children.len()) < matching,
                "next_offset": (!children.is_empty() && offset.saturating_add(children.len()) < matching)
                    .then_some(offset.saturating_add(children.len())),
                "cross_page_edges": cross_page_edges,
            },
            "depth": depth,
        });
        if let Some(ref backend) = self.backend {
            out["backend"] = json!(backend);
        }
        out
    }
    /// What a place stands for: a file or a folder by its path, or a symbol
    /// by the name the picture of its file selects it by.
    pub(super) fn reach<'a>(&self, place: &'a str) -> Option<Reach<'a>> {
        if let Some(file) = self.files.iter().position(|file| file == place) {
            return Some(Reach::File(file as u32));
        }
        let folder = place.trim_matches('/');
        if !folder.is_empty() && self.folder(folder) {
            return Some(Reach::Folder(folder));
        }
        // A symbol is named after its file: `src/cart.rs#L12:total()`.
        place.match_indices('#').find_map(|(at, _)| {
            let file = self.files.iter().position(|file| *file == place[..at])? as u32;
            let slot = self.keys(file).iter().position(|key| key == place)?;
            Some(Reach::Symbol(self.range(file).start + slot as u32))
        })
    }
    /// One link of a picture, looked into: the names at its two ends, by
    /// file, and every link of the graph between a name of the one end and a
    /// name of the other. `from` uses `to`; each is a folder, a file, or a
    /// symbol of an opened file. `certain` leaves out what the tool inferred.
    /// An end shows its `LINKED` most linked names; the counts are of the
    /// whole link. An end the graph does not know makes the link missing.
    pub fn link(&self, from: &str, to: &str, certain: bool) -> Value {
        let told = |place: &str, reach: Option<Reach>| match reach {
            Some(Reach::Symbol(at)) => json!({
                "path": place,
                "name": self.shown(at),
                "what": "symbol",
                "file": self.files[self.symbols[at as usize].file as usize],
            }),
            Some(Reach::File(_)) => json!({"path": place, "name": name(place), "what": "file"}),
            _ => json!({
                "path": place,
                "name": name(place.trim_matches('/')),
                "what": "folder",
            }),
        };
        let (user, used) = (self.reach(from), self.reach(to));
        let (Some(user), Some(used)) = (user, used) else {
            return json!({
                "mode": "link",
                "missing": true,
                "certain": certain,
                "from": told(from, user),
                "to": told(to, used),
            });
        };
        // The files of an end and, where it is one symbol, that symbol.
        let reached = |reach: Reach| {
            let files = self
                .files
                .iter()
                .enumerate()
                .map(|(id, file)| match reach {
                    Reach::Folder(folder) => file
                        .strip_prefix(folder)
                        .is_some_and(|rest| rest.starts_with('/')),
                    Reach::File(own) => id as u32 == own,
                    Reach::Symbol(symbol) => id as u32 == self.symbols[symbol as usize].file,
                })
                .collect::<Vec<_>>();
            let only = match reach {
                Reach::Symbol(symbol) => Some(symbol),
                _ => None,
            };
            (files, only)
        };
        let (uses, is_used) = (reached(user), reached(used));
        let of = |(files, only): &(Vec<bool>, Option<u32>), at: u32| {
            files[self.symbols[at as usize].file as usize] && only.is_none_or(|only| only == at)
        };
        let links = self
            .uses
            .iter()
            .filter(|(from, to, _, guessed)| {
                !(certain && *guessed) && of(&uses, *from) && of(&is_used, *to)
            })
            .collect::<Vec<_>>();
        let (mut out, mut into): (HashMap<u32, u64>, HashMap<u32, u64>) = Default::default();
        let mut kinds: BTreeMap<u16, u64> = BTreeMap::new();
        let mut guessed = 0u64;
        for (from, to, kind, inferred) in &links {
            *out.entry(*from).or_default() += 1;
            *into.entry(*to).or_default() += 1;
            *kinds.entry(*kind).or_default() += 1;
            guessed += u64::from(*inferred);
        }
        // The names of one end that are shown: the most linked first.
        let chosen = |counts: &HashMap<u32, u64>| {
            let mut names = counts
                .iter()
                .map(|(at, links)| (*at, *links))
                .collect::<Vec<_>>();
            names.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            names.truncate(LINKED);
            names
        };
        let (users, useds) = (chosen(&out), chosen(&into));
        let shown = users
            .iter()
            .chain(&useds)
            .map(|(at, _)| *at)
            .collect::<HashSet<_>>();
        let home = |at: u32| self.symbols[at as usize].file;
        let paths = shown
            .iter()
            .map(|at| self.files[home(*at) as usize].as_str())
            .collect::<Vec<_>>();
        // Lazily inspect source only for files appearing on either side of this link.
        let mut member_cache = FileMemberCache::new(self);
        let mut user_file_ids: BTreeSet<u32> = users.iter().map(|(at, _)| home(*at)).collect();
        let mut used_file_ids: BTreeSet<u32> = useds.iter().map(|(at, _)| home(*at)).collect();
        match user {
            Reach::File(f) => {
                user_file_ids.insert(f);
            }
            Reach::Symbol(s) => {
                user_file_ids.insert(home(s));
            }
            Reach::Folder(_) => {}
        }
        match used {
            Reach::File(f) => {
                used_file_ids.insert(f);
            }
            Reach::Symbol(s) => {
                used_file_ids.insert(home(s));
            }
            Reach::Folder(_) => {}
        }

        let target_sym_filter: Option<(String, Option<String>)> = match used {
            Reach::Symbol(s)
                if !matches!(
                    self.symbols[s as usize].kind,
                    Kind::Type | Kind::File | Kind::Note
                ) =>
            {
                let lbl = self.label(s).to_owned();
                let own = self
                    .owner(s)
                    .and_then(|o| o.rsplit('.').next().map(str::to_owned));
                Some((lbl, own))
            }
            _ => None,
        };

        let mut pair_analyses: HashMap<(u32, u32), (Vec<DepMemberRef>, Vec<VisibilityWarning>)> =
            HashMap::new();
        let mut link_member_refs: Vec<DepMemberRef> = Vec::new();
        let mut link_warnings: Vec<VisibilityWarning> = Vec::new();
        let mut user_group_refs: HashMap<u32, Vec<DepMemberRef>> = HashMap::new();
        let mut user_group_warnings: HashMap<u32, Vec<VisibilityWarning>> = HashMap::new();
        let mut used_group_refs: HashMap<u32, Vec<DepMemberRef>> = HashMap::new();
        let mut used_group_warnings: HashMap<u32, Vec<VisibilityWarning>> = HashMap::new();

        for &u_fid in &user_file_ids {
            let u_path = self.files[u_fid as usize].as_str();
            for &t_fid in &used_file_ids {
                if u_fid == t_fid {
                    continue;
                }
                let t_path = self.files[t_fid as usize].as_str();
                let (mut refs, mut warns) = member_cache.link_analysis(u_path, t_path);
                if let Some((ref f_name, ref f_owner)) = target_sym_filter {
                    let f_norm = norm_member_name(f_name);
                    let filtered_refs: Vec<DepMemberRef> = refs
                        .iter()
                        .filter(|r| {
                            norm_member_name(&r.name) == f_norm
                                && f_owner
                                    .as_ref()
                                    .is_none_or(|o| r.parent.as_deref().is_none_or(|p| p == o))
                        })
                        .cloned()
                        .collect();
                    if !filtered_refs.is_empty() {
                        refs = filtered_refs;
                        warns.retain(|w| norm_member_name(&w.member) == f_norm);
                    }
                }
                for r in &refs {
                    push_unique_link_member_ref(
                        user_group_refs.entry(u_fid).or_default(),
                        r.clone(),
                    );
                    push_unique_link_member_ref(
                        used_group_refs.entry(t_fid).or_default(),
                        r.clone(),
                    );
                    push_unique_link_member_ref(&mut link_member_refs, r.clone());
                }
                for w in &warns {
                    push_unique_link_warning(
                        user_group_warnings.entry(u_fid).or_default(),
                        w.clone(),
                    );
                    push_unique_link_warning(
                        used_group_warnings.entry(t_fid).or_default(),
                        w.clone(),
                    );
                    push_unique_link_warning(&mut link_warnings, w.clone());
                }
                pair_analyses.insert((u_fid, t_fid), (refs, warns));
            }
        }

        // A name is told as the picture of its file tells it.
        let mut keyed: HashMap<u32, Vec<String>> = HashMap::new();
        let mut key = |at: u32| {
            let file = home(at);
            let keys = keyed.entry(file).or_insert_with(|| self.keys(file));
            keys[(at - self.range(file).start) as usize].clone()
        };
        // One end as it is drawn: its files, the most linked first, each
        // with its names in the order of their lines.
        let mut side = |names: &[(u32, u64)],
                        counts: &HashMap<u32, u64>,
                        group_refs: &HashMap<u32, Vec<DepMemberRef>>,
                        group_warnings: &HashMap<u32, Vec<VisibilityWarning>>,
                        is_used_side: bool| {
            let mut groups: BTreeMap<u32, Vec<(u32, u64)>> = BTreeMap::new();
            for (at, links) in names {
                groups.entry(home(*at)).or_default().push((*at, *links));
            }
            let mut groups = groups.into_iter().collect::<Vec<_>>();
            let weight = |names: &[(u32, u64)]| names.iter().map(|(_, links)| links).sum::<u64>();
            groups.sort_by(|a, b| weight(&b.1).cmp(&weight(&a.1)).then(a.0.cmp(&b.0)));
            let files = counts
                .keys()
                .map(|at| home(*at))
                .collect::<HashSet<_>>()
                .len();
            json!({
                "names": counts.len(),
                "files": files,
                "groups": groups.into_iter().map(|(file, mut names)| {
                    names.sort_by_key(|(at, _)| (self.symbols[*at as usize].line, *at));
                    let path = self.files[file as usize].as_str();
                    let decls = member_cache.declared(path).to_vec();
                    let grefs = group_refs.get(&file).cloned().unwrap_or_default();
                    let gwarns = group_warnings.get(&file).cloned().unwrap_or_default();
                    let name_vals = names.into_iter().map(|(at, links)| {
                        let symbol = &self.symbols[at as usize];
                        let matched_tm = member_cache.symbol_match(at);
                        let lbl_norm = norm_member_name(self.label(at));
                        let risk = if is_used_side {
                            grefs.iter().find_map(|r| {
                                (norm_member_name(&r.name) == lbl_norm)
                                    .then_some(r.access_risk.as_deref())
                                    .flatten()
                            })
                        } else {
                            None
                        };
                        let mut obj = serde_json::Map::new();
                        obj.insert("key".to_owned(), json!(key(at)));
                        obj.insert("name".to_owned(), json!(self.shown(at)));
                        obj.insert("kind".to_owned(), json!(symbol.kind.word()));
                        obj.insert("line".to_owned(), json!(symbol.line));
                        obj.insert("owner".to_owned(), json!(self.owner(at)));
                        obj.insert("links".to_owned(), json!(links));
                        insert_symbol_overlay(&mut obj, matched_tm.as_ref(), risk);
                        Value::Object(obj)
                    }).collect::<Vec<_>>();
                    json!({
                        "path": path,
                        "name": label(path, &paths),
                        "declared_members": decls,
                        "referenced_members": grefs,
                        "visibility_warnings": gwarns,
                        "names": name_vals,
                    })
                }).collect::<Vec<_>>(),
            })
        };
        let (users, useds) = (
            side(&users, &out, &user_group_refs, &user_group_warnings, false),
            side(&useds, &into, &used_group_refs, &used_group_warnings, true),
        );
        let mut edges = links
            .iter()
            .filter(|(from, to, ..)| shown.contains(from) && shown.contains(to))
            .collect::<Vec<_>>();
        edges.sort_by_key(|(from, to, ..)| (*from, *to));
        edges.truncate(WIRED);
        let edge_vals = edges
            .iter()
            .map(|(from_sym, to_sym, kind, inferred)| {
                let u_fid = home(*from_sym);
                let t_fid = home(*to_sym);
                let to_lbl_norm = norm_member_name(self.label(*to_sym));
                let mut access_risk = pair_analyses.get(&(u_fid, t_fid)).and_then(|(refs, _)| {
                    refs.iter().find_map(|r| {
                        (norm_member_name(&r.name) == to_lbl_norm)
                            .then_some(r.access_risk.clone())
                            .flatten()
                    })
                });
                if access_risk.is_none()
                    && u_fid != t_fid
                    && let Some(tm) = member_cache.symbol_match(*to_sym)
                {
                    let u_path = self.files[u_fid as usize].as_str();
                    let t_path = self.files[t_fid as usize].as_str();
                    let t_decls = member_cache.declared(t_path).to_vec();
                    let u_src = member_cache.source(u_path).map(str::to_owned);
                    let root = &self.root;
                    let known = self.known_files_set();
                    let caller_ext = caller_extends_target_in(
                        u_path,
                        u_src.as_deref(),
                        t_path,
                        &t_decls,
                        known,
                        &mut |p| text_at(root, p),
                    );
                    access_risk = crate::graph_impact::assess_member_access_in(
                        Some(root),
                        Some(known),
                        &tm,
                        u_path,
                        t_path,
                        caller_ext,
                        "reference",
                        Some(self.symbols[*from_sym as usize].line),
                    )
                    .access_risk;
                }
                let mut obj = serde_json::Map::new();
                obj.insert("from".to_owned(), json!(key(*from_sym)));
                obj.insert("to".to_owned(), json!(key(*to_sym)));
                obj.insert("kind".to_owned(), json!(self.kinds[*kind as usize]));
                obj.insert("guessed".to_owned(), json!(inferred));
                if let Some(risk) = access_risk {
                    obj.insert("access_risk".to_owned(), json!(risk));
                }
                Value::Object(obj)
            })
            .collect::<Vec<_>>();
        json!({
            "mode": "link",
            "certain": certain,
            "from": told(from, Some(user)),
            "to": told(to, Some(used)),
            "links": links.len(),
            "guessed": guessed,
            "users": users,
            "used": useds,
            "edges": edge_vals,
            "kinds": self.kinds_json(&kinds),
            "member_refs": link_member_refs,
            "visibility_warnings": link_warnings,
        })
    }
}

pub(super) const MAX_CACHED_MEMBER_FILES: usize = 128;
pub(super) const MAX_CACHED_MEMBER_LINKS: usize = 256;
