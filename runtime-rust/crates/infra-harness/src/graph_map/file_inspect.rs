use super::*;

/// Whose a line is: nobody's but the file's, the symbols' of one line, or
/// one symbol's that holds the ones nearer to the line.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Owner {
    Top,
    Group(usize),
    One(u32),
}
impl Index {
    /// Tells which symbols of a file a change touched. The graph says where
    /// a symbol starts, not where it ends, so its lines are told from the
    /// text: blank lines, comments and attributes right above a symbol are
    /// its own, and it ends where the text returns to its indentation with
    /// something that does not close it. A line past that end belongs to
    /// what holds the symbol, and past every end to the file itself. A
    /// blank line touches nothing.
    pub(super) fn marks(&self, file: u32, edited: &Edited) -> Marks {
        let range = self.range(file);
        let slot = |at: u32| (at - range.start) as usize;
        let mut marks = Marks {
            status: vec![None; range.len()],
            inside: vec![0; range.len()],
            between: 0,
        };
        if edited.new {
            for at in range.clone() {
                if self.symbols[at as usize].kind != Kind::Note {
                    marks.status[slot(at)] = Some('A');
                }
            }
        } else {
            let rows = lines(&edited.text);
            let last = rows.len() as u32;
            let row = |line: u32| &rows[line as usize - 1];
            let mut placed = range
                .clone()
                .filter(|at| {
                    let symbol = &self.symbols[*at as usize];
                    !matches!(symbol.kind, Kind::Note | Kind::File)
                        && symbol.line >= 1
                        && symbol.line <= last
                })
                .collect::<Vec<_>>();
            placed.sort_by_key(|at| (self.symbols[*at as usize].line, *at));
            let mut groups: Vec<Group> = Vec::new();
            let mut home = vec![usize::MAX; range.len()];
            for (index, at) in placed.iter().enumerate() {
                let start = self.symbols[*at as usize].line;
                match groups.last_mut() {
                    Some(group) if group.start == start => group.to = index + 1,
                    _ => groups.push(Group {
                        start,
                        lead: start,
                        end: last + 1,
                        from: index,
                        to: index + 1,
                    }),
                }
                home[slot(*at)] = groups.len() - 1;
            }
            for index in 0..groups.len() {
                let start = groups[index].start;
                let floor = if index == 0 {
                    0
                } else {
                    groups[index - 1].start
                };
                let indent = row(start).indent;
                let mut lead = start;
                while lead - 1 > floor
                    && match row(lead - 1).kind {
                        Row::Blank => true,
                        Row::Comment => row(lead - 1).indent <= indent,
                        _ => false,
                    }
                {
                    lead -= 1;
                }
                groups[index].lead = lead;
                // A heading holds whatever follows it.
                let heading = placed[groups[index].from..groups[index].to]
                    .iter()
                    .any(|at| self.symbols[*at as usize].kind == Kind::Heading);
                if heading {
                    continue;
                }
                // Prefer exact AST end_line spans from Graph IR when present, avoiding indentation heuristics.
                if let Some(explicit_end) = placed[groups[index].from..groups[index].to]
                    .iter()
                    .filter_map(|at| {
                        let sym = &self.symbols[*at as usize];
                        (sym.end_line > 0 && sym.end_line >= sym.line).then_some(sym.end_line)
                    })
                    .max()
                {
                    groups[index].end = explicit_end.saturating_add(1).min(last + 1);
                    continue;
                }
                // Until something deeper was met, a line at the same depth
                // still belongs to the signature.
                let mut body = row(start).whole;
                for line in start + 1..=last {
                    let here = row(line);
                    if matches!(here.kind, Row::Blank | Row::Comment) || here.indent > indent {
                        body |= here.indent > indent && here.kind != Row::Blank;
                        continue;
                    }
                    if here.indent == indent && (!body || here.kind == Row::Closer) {
                        continue;
                    }
                    groups[index].end = line;
                    break;
                }
            }
            let owner = |line: u32| {
                let Some(index) = groups
                    .partition_point(|group| group.lead <= line)
                    .checked_sub(1)
                else {
                    return Owner::Top;
                };
                let group = &groups[index];
                if line < group.end {
                    return Owner::Group(index);
                }
                let mut next = self.symbols[placed[group.from] as usize].parent;
                let mut hops = 0;
                while next != NONE && range.contains(&next) && hops < 16 {
                    hops += 1;
                    let at = home[slot(next)];
                    if at != usize::MAX && groups[at].lead <= line && line < groups[at].end {
                        return Owner::One(next);
                    }
                    next = self.symbols[next as usize].parent;
                }
                Owner::Top
            };
            // 1 for a line that stands in place of removed ones, 2 for a
            // line that was only added.
            let mut fresh = vec![0u8; rows.len() + 1];
            for hunk in &edited.hunks {
                for line in hunk.at..hunk.at.saturating_add(hunk.added) {
                    if line >= 1 && line <= last {
                        fresh[line as usize] = if hunk.removed == 0 { 2 } else { 1 };
                    }
                }
            }
            let mut hit = vec![false; range.len()];
            let mut top = false;
            let mut touch = |owner: Owner| match owner {
                Owner::Top => top = true,
                Owner::Group(index) => {
                    for at in &placed[groups[index].from..groups[index].to] {
                        hit[slot(*at)] = true;
                    }
                }
                Owner::One(at) => hit[slot(at)] = true,
            };
            for hunk in &edited.hunks {
                if hunk.added > 0 {
                    for line in hunk.at..hunk.at.saturating_add(hunk.added) {
                        if line >= 1 && line <= last && row(line).kind != Row::Blank {
                            touch(owner(line));
                        }
                    }
                } else if !hunk.blank {
                    // Lines were only removed: from a symbol when the lines
                    // on both sides of the gap are its own.
                    let before = match hunk.at {
                        0 => Owner::Top,
                        line => owner(line.min(last)),
                    };
                    let after = if hunk.at < last {
                        owner(hunk.at + 1)
                    } else {
                        before
                    };
                    if before == after {
                        touch(before);
                    } else {
                        marks.between += 1;
                    }
                }
            }
            for (index, group) in groups.iter().enumerate() {
                // New when its first line and every other line of its own,
                // down to the next symbol, were only added.
                let limit = groups
                    .get(index + 1)
                    .map_or(last + 1, |next| next.lead)
                    .min(group.end);
                let new = (group.start..limit)
                    .all(|line| row(line).kind == Row::Blank || fresh[line as usize] == 2);
                for at in &placed[group.from..group.to] {
                    if hit[slot(*at)] {
                        marks.status[slot(*at)] = Some(if new { 'A' } else { 'M' });
                    }
                }
            }
            if top {
                for at in range.clone() {
                    if self.symbols[at as usize].kind == Kind::File {
                        marks.status[slot(at)] = Some('M');
                    }
                }
            }
        }
        for at in range.clone() {
            if marks.status[slot(at)].is_none() || self.symbols[at as usize].kind == Kind::File {
                continue;
            }
            let mut next = self.symbols[at as usize].parent;
            let mut hops = 0;
            while next != NONE && next != at && range.contains(&next) && hops < 16 {
                hops += 1;
                marks.inside[slot(next)] += 1;
                next = self.symbols[next as usize].parent;
            }
        }
        marks
    }
    pub(super) fn label(&self, at: u32) -> &str {
        let symbol = &self.symbols[at as usize];
        &self.names[symbol.at as usize..symbol.at as usize + symbol.len as usize]
    }
    pub(super) fn range(&self, file: u32) -> std::ops::Range<u32> {
        self.first[file as usize]..self.first[file as usize + 1]
    }
    /// How many classes, methods, functions and other names a file holds.
    pub(super) fn holds(&self, file: u32) -> u64 {
        self.range(file)
            .filter(|at| !matches!(self.symbols[*at as usize].kind, Kind::File | Kind::Note))
            .count() as u64
    }
    /// A symbol as it is shown: a method with its class, `Cart.total()`, and
    /// the node of the file itself under the file's name.
    pub(super) fn shown(&self, at: u32) -> String {
        let mut text = String::new();
        self.show(at, &mut text);
        text
    }
    /// Writes a symbol as it is shown after what `into` holds.
    pub(super) fn show(&self, at: u32, into: &mut String) {
        let symbol = &self.symbols[at as usize];
        let label = self.label(at);
        match symbol.kind {
            Kind::File => into.push_str(name(&self.files[symbol.file as usize])),
            Kind::Method if symbol.parent != NONE => {
                into.push_str(self.label(symbol.parent));
                if !label.starts_with('.') {
                    into.push('.');
                }
                into.push_str(label);
            }
            _ => into.push_str(label),
        }
    }
    /// What a symbol lies in, outermost first, as one name: `Cart.total()`
    /// for a function declared inside that method.
    pub(super) fn owner(&self, at: u32) -> Option<String> {
        let mut holders = Vec::new();
        let mut next = self.symbols[at as usize].parent;
        // A graph can name two symbols as each other's holder.
        while next != NONE && next != at && !holders.contains(&next) && holders.len() < 16 {
            holders.push(next);
            next = self.symbols[next as usize].parent;
        }
        let parts = holders
            .iter()
            .map(|holder| self.label(*holder))
            .collect::<Vec<_>>();
        let mut text = String::new();
        for part in parts.iter().rev() {
            if !text.is_empty() && !part.starts_with('.') {
                text.push_str(" › ");
            }
            text.push_str(part);
        }
        (!text.is_empty()).then_some(text)
    }
    /// The names the symbols of a file are selected by, in their order: the
    /// file, the line and the label, with a number where two share all three.
    pub(super) fn keys(&self, file: u32) -> Vec<String> {
        let path = &self.files[file as usize];
        let mut seen: HashMap<String, u32> = HashMap::new();
        self.range(file)
            .map(|at| {
                let (line, label) = (self.symbols[at as usize].line, self.label(at));
                let mut key = if line > 0 {
                    format!("{path}#L{line}:{label}")
                } else {
                    format!("{path}#{label}")
                };
                let met = seen.entry(key.clone()).or_default();
                *met += 1;
                if *met > 1 {
                    key.push_str(&format!("~{met}"));
                }
                key
            })
            .collect()
    }
    /// One file of the map: the classes, methods and functions the graph
    /// names in it, the links between them, and the files they use or are
    /// used by. `changed` marks the files of a change among those around,
    /// `edited` the symbols that change touched here, `filter` keeps the
    /// symbols whose name contains it, and `focus` lists both sides of one
    /// symbol, or what one file around has with this one.
    ///
    /// A graph links two symbols at most once, so a link between two symbols
    /// counts one, and a link with a file around counts the pairs of symbols
    /// it stands for.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn file(
        &self,
        project: &str,
        file: u32,
        changed: &HashMap<&str, char>,
        edited: Option<&Edited>,
        focus: Option<&str>,
        certain: bool,
        filter: &str,
    ) -> Value {
        let path = self.files[file as usize].as_str();
        let range = self.range(file);
        let own = |at: u32| range.contains(&at);
        let slot = |at: u32| (at - range.start) as usize;
        let keys = self.keys(file);
        let marks = edited.map(|edited| self.marks(file, edited));
        // What happened to a symbol's own lines, and how many symbols
        // changed inside it.
        let status = |at: u32| marks.as_ref().and_then(|marks| marks.status[slot(at)]);
        let inside = |at: u32| marks.as_ref().map_or(0, |marks| marks.inside[slot(at)]);
        let touched = |at: u32| status(at).is_some() || inside(at) > 0;
        let mut out = vec![0u64; range.len()];
        let mut into = vec![0u64; range.len()];
        // Links between two symbols here, links that leave the file and
        // links that enter it.
        let (mut inner, mut leaving, mut entering) = (Vec::new(), Vec::new(), Vec::new());
        for link in &self.uses {
            let (from, to, _, guessed) = *link;
            if certain && guessed {
                continue;
            }
            match (own(from), own(to)) {
                (true, true) => {
                    out[slot(from)] += 1;
                    into[slot(to)] += 1;
                    inner.push(*link);
                }
                (true, false) => {
                    out[slot(from)] += 1;
                    leaving.push(*link);
                }
                (false, true) => {
                    into[slot(to)] += 1;
                    entering.push(*link);
                }
                (false, false) => {}
            }
        }
        let mut members = vec![0u64; range.len()];
        let (mut types, mut methods, mut functions) = (0u64, 0u64, 0u64);
        let (mut headings, mut others) = (0u64, 0u64);
        for at in range.clone() {
            let symbol = &self.symbols[at as usize];
            match symbol.kind {
                Kind::File | Kind::Note => continue,
                Kind::Type => types += 1,
                Kind::Method => methods += 1,
                Kind::Function => functions += 1,
                Kind::Heading => headings += 1,
                Kind::Case | Kind::Name => others += 1,
            }
            if symbol.parent != NONE && own(symbol.parent) {
                members[slot(symbol.parent)] += 1;
            }
        }
        let weight = |at: u32| out[slot(at)] + into[slot(at)];
        let needle = filter.trim().to_lowercase();
        let mut listed = range
            .clone()
            .filter(|at| match self.symbols[*at as usize].kind {
                Kind::Note => false,
                // The file's own node is worth a box when something is
                // linked to it: its imports, or what imports the file.
                Kind::File => weight(*at) > 0,
                _ => true,
            })
            .collect::<Vec<_>>();
        let children = listed.len();
        // A filter chooses among the symbols, and the file's own node is
        // none: in a file named after its class it would match every time.
        listed.retain(|at| {
            needle.is_empty()
                || (self.symbols[*at as usize].kind != Kind::File
                    && self.shown(*at).to_lowercase().contains(&needle))
        });
        let matching = listed.len();
        // What the change touched comes first, then the most linked.
        listed.sort_by(|a, b| {
            touched(*b)
                .cmp(&touched(*a))
                .then(weight(*b).cmp(&weight(*a)))
                .then(
                    self.symbols[*a as usize]
                        .line
                        .cmp(&self.symbols[*b as usize].line),
                )
                .then(a.cmp(b))
        });
        listed.truncate(NAMED);
        // The selected symbol is drawn whatever the limit and the filter say.
        let chosen = focus.and_then(|focus| {
            range.clone().find(|at| {
                keys[slot(*at)] == focus && self.symbols[*at as usize].kind != Kind::Note
            })
        });
        let mut beyond = false;
        if let Some(at) = chosen
            && !listed.contains(&at)
        {
            listed.push(at);
            beyond = true;
        }
        let mut drawn = vec![false; range.len()];
        for at in &listed {
            drawn[slot(*at)] = true;
        }
        // The files around are those the drawn symbols are linked with.
        let mut flows: HashMap<(End, End), EdgeFlow> = HashMap::new();
        let mut near: HashMap<u32, Near> = HashMap::new();
        let mut flow = |from: End, to: End, kind: u16, guessed: bool| {
            flows.entry((from, to)).or_default().add_link(kind, guessed);
        };
        for (from, to, kind, guessed) in &inner {
            if drawn[slot(*from)] && drawn[slot(*to)] {
                flow(End::Symbol(*from), End::Symbol(*to), *kind, *guessed);
            }
        }
        for (from, to, kind, guessed) in &leaving {
            if drawn[slot(*from)] {
                let other = self.symbols[*to as usize].file;
                near.entry(other).or_default().used += 1;
                flow(End::Symbol(*from), End::File(other), *kind, *guessed);
            }
        }
        for (from, to, kind, guessed) in &entering {
            if drawn[slot(*to)] {
                let other = self.symbols[*from as usize].file;
                near.entry(other).or_default().uses += 1;
                flow(End::File(other), End::Symbol(*to), *kind, *guessed);
            }
        }
        let linked = near.len();
        let mut around = near
            .iter()
            .map(|(id, near)| (self.files[*id as usize].as_str(), *id, *near))
            .collect::<Vec<_>>();
        around.sort_by(|a, b| {
            (b.2.uses + b.2.used)
                .cmp(&(a.2.uses + a.2.used))
                .then(a.0.cmp(b.0))
        });
        around.truncate(SHOWN.saturating_sub(listed.len()).max(AROUND));
        let beside = around.iter().map(|(_, id, _)| *id).collect::<HashSet<_>>();
        let place = |end: &End| match end {
            End::Symbol(at) => keys[slot(*at)].as_str(),
            End::File(id) => self.files[*id as usize].as_str(),
        };
        let mut edges = flows
            .iter()
            .filter(|((from, to), _)| {
                [from, to].iter().all(|end| match end {
                    End::Symbol(_) => true,
                    End::File(id) => beside.contains(id),
                })
            })
            .map(|((from, to), edge)| (place(from), place(to), edge))
            .collect::<Vec<_>>();
        let between = edges.len();
        edges.sort_by(|a, b| {
            b.2.flow
                .links
                .cmp(&a.2.flow.links)
                .then((a.0, a.1).cmp(&(b.0, b.1)))
        });
        edges.truncate(DRAWN);
        let mut member_cache = FileMemberCache::new(self);
        let file_declared_members = member_cache.declared(path).to_vec();

        let mut paths = around.iter().map(|(path, ..)| *path).collect::<Vec<_>>();
        paths.push(path);
        let mut nodes = listed
            .iter()
            .map(|at| {
                let symbol = &self.symbols[*at as usize];
                let tm = member_cache.symbol_match(*at);
                let mut obj = serde_json::Map::new();
                obj.insert("path".to_owned(), json!(keys[slot(*at)]));
                obj.insert("name".to_owned(), json!(self.shown(*at)));
                obj.insert("kind".to_owned(), json!(symbol.kind.word()));
                obj.insert("line".to_owned(), json!(symbol.line));
                obj.insert("owner".to_owned(), json!(self.owner(*at)));
                obj.insert("members".to_owned(), json!(members[slot(*at)]));
                obj.insert("symbol".to_owned(), json!(true));
                obj.insert("folder".to_owned(), json!(false));
                obj.insert("files".to_owned(), json!(1));
                obj.insert("within".to_owned(), json!(0));
                obj.insert("out".to_owned(), json!(out[slot(*at)]));
                obj.insert("in".to_owned(), json!(into[slot(*at)]));
                obj.insert("changed".to_owned(), json!(u64::from(touched(*at))));
                obj.insert("status".to_owned(), json!(status(*at).map(String::from)));
                obj.insert("touched".to_owned(), json!(inside(*at)));
                insert_symbol_overlay(&mut obj, tm.as_ref(), None);
                Value::Object(obj)
            })
            .collect::<Vec<_>>();
        nodes.extend(around.iter().map(|(other, id, near)| {
            json!({
                "path": other,
                "name": label(other, &paths),
                "kind": "file",
                "symbol": false,
                "folder": false,
                "files": 1,
                "symbols": self.holds(*id),
                "within": 0,
                "out": near.uses,
                "in": near.used,
                "role": near.role(),
                "changed": u64::from(changed.contains_key(other)),
                "status": changed.get(other).map(|kind| kind.to_string()),
            })
        }));
        // The rows of the side panel name symbols of other files too, by the
        // names those files select them by.
        let relations = |links: &[&Use]| {
            let mut kinds: BTreeMap<u16, u64> = BTreeMap::new();
            for (_, _, kind, _) in links {
                *kinds.entry(*kind).or_default() += 1;
            }
            self.kinds_json(&kinds)
        };
        let mut rows = |mut told: Vec<Told>, is_used_by: bool| {
            let more = told.len().saturating_sub(TOLD);
            told.truncate(TOLD);
            let mut foreign: HashMap<u32, Vec<String>> = HashMap::new();
            let rows = told
                .iter()
                .map(|told| {
                    let symbol = &self.symbols[told.symbol as usize];
                    let home = symbol.file;
                    let key = if own(told.symbol) {
                        keys[slot(told.symbol)].clone()
                    } else {
                        let keys = foreign.entry(home).or_insert_with(|| self.keys(home));
                        keys[(told.symbol - self.first[home as usize]) as usize].clone()
                    };
                    let elsewhere = (home != file).then(|| self.files[home as usize].as_str());
                    let sym_tm = member_cache.symbol_match(told.symbol);
                    let mut obj = serde_json::Map::new();
                    obj.insert("path".to_owned(), json!(key));
                    obj.insert("name".to_owned(), json!(self.shown(told.symbol)));
                    obj.insert("kind".to_owned(), json!(symbol.kind.word()));
                    obj.insert("line".to_owned(), json!(symbol.line));
                    obj.insert("owner".to_owned(), json!(self.owner(told.symbol)));
                    obj.insert("file".to_owned(), json!(elsewhere));
                    obj.insert("symbol".to_owned(), json!(true));
                    obj.insert("folder".to_owned(), json!(false));
                    obj.insert("inside".to_owned(), json!(home == file));
                    obj.insert(
                        "relation".to_owned(),
                        json!(told.relation.map(|kind| self.kinds[kind as usize].as_str())),
                    );
                    obj.insert("links".to_owned(), json!(told.links));
                    obj.insert("guessed".to_owned(), json!(told.guessed));
                    obj.insert(
                        "changed".to_owned(),
                        json!(match elsewhere {
                            Some(other) => changed.contains_key(other),
                            None => touched(told.symbol),
                        }),
                    );
                    let mut sym_risk: Option<String> = None;
                    if let Some(other) = elsewhere {
                        let (r, w, decls) = if is_used_by {
                            let (r, w) = member_cache.link_analysis(other, path);
                            (r, w, file_declared_members.clone())
                        } else {
                            let (r, w) = member_cache.link_analysis(path, other);
                            let lbl_norm = norm_member_name(self.label(told.symbol));
                            sym_risk = r.iter().find_map(|item| {
                                (norm_member_name(&item.name) == lbl_norm)
                                    .then_some(item.access_risk.clone())
                                    .flatten()
                            });
                            (r, w, member_cache.declared(other).to_vec())
                        };
                        obj.insert("declared_members".to_owned(), json!(decls));
                        obj.insert("referenced_members".to_owned(), json!(r));
                        obj.insert("visibility_warnings".to_owned(), json!(w));
                    }
                    insert_symbol_overlay(&mut obj, sym_tm.as_ref(), sym_risk.as_deref());
                    Value::Object(obj)
                })
                .collect::<Vec<_>>();
            (rows, more)
        };
        // What one symbol is linked with, a row per symbol: what was read
        // first, this file's before the others'.
        let single = |links: Vec<(u32, &Use)>| {
            let mut told = links
                .into_iter()
                .map(|(symbol, link)| Told {
                    symbol,
                    relation: Some(link.2),
                    links: 1,
                    guessed: u64::from(link.3),
                })
                .collect::<Vec<_>>();
            told.sort_by(|a, b| {
                let key = |told: &Told| {
                    let symbol = &self.symbols[told.symbol as usize];
                    (
                        told.guessed,
                        symbol.file != file,
                        self.files[symbol.file as usize].as_str(),
                        symbol.line,
                        told.symbol,
                    )
                };
                key(a).cmp(&key(b))
            });
            told
        };
        // The symbols of a file around that the links with this file end
        // at, the most linked first.
        let grouped = |ends: Vec<(u32, bool)>| {
            let mut counted: BTreeMap<u32, (u64, u64)> = BTreeMap::new();
            for (symbol, guessed) in ends {
                let entry = counted.entry(symbol).or_default();
                entry.0 += 1;
                entry.1 += u64::from(guessed);
            }
            let mut told = counted
                .into_iter()
                .map(|(symbol, (links, guessed))| Told {
                    symbol,
                    relation: None,
                    links,
                    guessed,
                })
                .collect::<Vec<_>>();
            told.sort_by(|a, b| {
                b.links
                    .cmp(&a.links)
                    .then(
                        self.symbols[a.symbol as usize]
                            .line
                            .cmp(&self.symbols[b.symbol as usize].line),
                    )
                    .then(a.symbol.cmp(&b.symbol))
            });
            told
        };
        let focus = match (chosen, focus) {
            (Some(at), _) => {
                let symbol = &self.symbols[at as usize];
                let uses = inner
                    .iter()
                    .chain(&leaving)
                    .filter(|link| link.0 == at)
                    .collect::<Vec<_>>();
                let used = inner
                    .iter()
                    .chain(&entering)
                    .filter(|link| link.1 == at)
                    .collect::<Vec<_>>();
                let kinds = relations(&uses);
                let (uses, uses_more) = rows(
                    single(uses.into_iter().map(|link| (link.1, link)).collect()),
                    false,
                );
                let (used_by, used_by_more) = rows(
                    single(used.into_iter().map(|link| (link.0, link)).collect()),
                    true,
                );
                let mut held = range
                    .clone()
                    .filter(|other| {
                        self.symbols[*other as usize].parent == at
                            && self.symbols[*other as usize].kind != Kind::Note
                    })
                    .collect::<Vec<_>>();
                held.sort_by_key(|other| (self.symbols[*other as usize].line, *other));
                let held_more = held.len().saturating_sub(TOLD);
                held.truncate(TOLD);

                let sym_tm = member_cache.symbol_match(at);
                let focus_lbl = self.label(at);
                let focus_decls: Vec<DeclaredMember> = if !held.is_empty()
                    || sym_tm.as_ref().is_some_and(|m| {
                        matches!(
                            m.kind.as_str(),
                            "class" | "abstract_class" | "trait" | "interface" | "struct" | "enum"
                        )
                    }) {
                    let f_norm = norm_member_name(focus_lbl);
                    file_declared_members
                        .iter()
                        .filter(|m| {
                            m.parent
                                .as_deref()
                                .is_some_and(|p| norm_member_name(p) == f_norm)
                        })
                        .cloned()
                        .collect()
                } else {
                    sym_tm.clone().into_iter().collect()
                };
                let member_vals = held
                    .iter()
                    .map(|other| {
                        let m_tm = member_cache.symbol_match(*other);
                        let mut m_obj = serde_json::Map::new();
                        m_obj.insert("path".to_owned(), json!(keys[slot(*other)]));
                        m_obj.insert("name".to_owned(), json!(self.label(*other)));
                        m_obj.insert(
                            "kind".to_owned(),
                            json!(self.symbols[*other as usize].kind.word()),
                        );
                        m_obj.insert("line".to_owned(), json!(self.symbols[*other as usize].line));
                        m_obj.insert("symbol".to_owned(), json!(true));
                        m_obj.insert("folder".to_owned(), json!(false));
                        m_obj.insert("inside".to_owned(), json!(true));
                        m_obj.insert("changed".to_owned(), json!(touched(*other)));
                        insert_symbol_overlay(&mut m_obj, m_tm.as_ref(), None);
                        Value::Object(m_obj)
                    })
                    .collect::<Vec<_>>();
                let mut sym_refs: Vec<DepMemberRef> = Vec::new();
                let mut sym_warns: Vec<VisibilityWarning> = Vec::new();
                let target_sym_norm = norm_member_name(focus_lbl);
                let is_container = !held.is_empty()
                    || sym_tm.as_ref().is_some_and(|m| {
                        matches!(
                            m.kind.as_str(),
                            "class" | "abstract_class" | "trait" | "interface" | "struct" | "enum"
                        )
                    });
                for item in uses.iter().chain(used_by.iter()) {
                    for r in item
                        .get("referenced_members")
                        .and_then(|v| serde_json::from_value::<Vec<DepMemberRef>>(v.clone()).ok())
                        .unwrap_or_default()
                    {
                        let matches_focus = norm_member_name(&r.name) == target_sym_norm
                            || (is_container
                                && r.parent
                                    .as_deref()
                                    .is_some_and(|p| norm_member_name(p) == target_sym_norm));
                        if matches_focus {
                            push_unique_link_member_ref(&mut sym_refs, r);
                        }
                    }
                    for w in item
                        .get("visibility_warnings")
                        .and_then(|v| {
                            serde_json::from_value::<Vec<VisibilityWarning>>(v.clone()).ok()
                        })
                        .unwrap_or_default()
                    {
                        let matches_focus = norm_member_name(&w.member) == target_sym_norm
                            || (is_container
                                && w.parent
                                    .as_deref()
                                    .is_some_and(|p| norm_member_name(p) == target_sym_norm));
                        if matches_focus {
                            push_unique_link_warning(&mut sym_warns, w);
                        }
                    }
                }
                let mut f_obj = serde_json::Map::new();
                f_obj.insert("path".to_owned(), json!(keys[slot(at)]));
                f_obj.insert("name".to_owned(), json!(self.shown(at)));
                f_obj.insert("kind".to_owned(), json!(symbol.kind.word()));
                f_obj.insert("line".to_owned(), json!(symbol.line));
                f_obj.insert("owner".to_owned(), json!(self.owner(at)));
                f_obj.insert("file".to_owned(), json!(path));
                f_obj.insert("symbol".to_owned(), json!(true));
                f_obj.insert("folder".to_owned(), json!(false));
                f_obj.insert("inside".to_owned(), json!(true));
                f_obj.insert("out".to_owned(), json!(out[slot(at)]));
                f_obj.insert("in".to_owned(), json!(into[slot(at)]));
                f_obj.insert("changed".to_owned(), json!(u64::from(touched(at))));
                f_obj.insert("status".to_owned(), json!(status(at).map(String::from)));
                f_obj.insert("touched".to_owned(), json!(inside(at)));
                f_obj.insert("members".to_owned(), json!(member_vals));
                f_obj.insert("members_more".to_owned(), json!(held_more));
                f_obj.insert("declared_members".to_owned(), json!(focus_decls));
                f_obj.insert("referenced_members".to_owned(), json!(sym_refs));
                f_obj.insert("visibility_warnings".to_owned(), json!(sym_warns));
                f_obj.insert("uses".to_owned(), json!(uses));
                f_obj.insert("uses_more".to_owned(), json!(uses_more));
                f_obj.insert("used_by".to_owned(), json!(used_by));
                f_obj.insert("used_by_more".to_owned(), json!(used_by_more));
                f_obj.insert("kinds".to_owned(), json!(kinds));
                insert_symbol_overlay(&mut f_obj, sym_tm.as_ref(), None);
                Some(Value::Object(f_obj))
            }
            (None, Some(other)) => {
                if let Some((other, id, near)) = around.iter().find(|(place, ..)| *place == other) {
                    let theirs = |at: u32| self.symbols[at as usize].file == *id;
                    // Its symbols that use this file, and its symbols that
                    // this file uses.
                    let users = entering
                        .iter()
                        .filter(|link| theirs(link.0))
                        .collect::<Vec<_>>();
                    let kinds = relations(&users);
                    let (uses, uses_more) = rows(
                        grouped(users.into_iter().map(|link| (link.0, link.3)).collect()),
                        true,
                    );
                    let (used_by, used_by_more) = rows(
                        grouped(
                            leaving
                                .iter()
                                .filter(|link| theirs(link.1))
                                .map(|link| (link.1, link.3))
                                .collect(),
                        ),
                        false,
                    );
                    let other_decls = member_cache.declared(other).to_vec();
                    let mut frefs = Vec::new();
                    let mut fwarns = Vec::new();
                    if near.used > 0 {
                        let (r, w) = member_cache.link_analysis(path, other);
                        for item in r {
                            push_unique_link_member_ref(&mut frefs, item);
                        }
                        for item in w {
                            push_unique_link_warning(&mut fwarns, item);
                        }
                    }
                    if near.uses > 0 {
                        let (r, w) = member_cache.link_analysis(other, path);
                        for item in r {
                            push_unique_link_member_ref(&mut frefs, item);
                        }
                        for item in w {
                            push_unique_link_warning(&mut fwarns, item);
                        }
                    }
                    Some(json!({
                        "path": other,
                        "name": label(other, &paths),
                        "kind": "file",
                        "symbol": false,
                        "folder": false,
                        "inside": true,
                        "files": 1,
                        "symbols": self.holds(*id),
                        "out": near.uses,
                        "in": near.used,
                        "role": near.role(),
                        "changed": u64::from(changed.contains_key(other)),
                        "status": changed.get(other).map(|kind| kind.to_string()),
                        "declared_members": other_decls,
                        "referenced_members": frefs,
                        "visibility_warnings": fwarns,
                        "uses": uses,
                        "uses_more": uses_more,
                        "used_by": used_by,
                        "used_by_more": used_by_more,
                        "kinds": kinds,
                    }))
                } else {
                    None
                }
            }
            _ => None,
        };
        let files = |links: &[Use], end: fn(&Use) -> u32| {
            links
                .iter()
                .map(|link| self.symbols[end(link) as usize].file)
                .collect::<HashSet<_>>()
                .len()
        };
        // How much of the file the change touched, where that is known.
        let reach = marks.as_ref().map(|marks| {
            let of = |letter: char| {
                range
                    .clone()
                    .filter(|at| {
                        self.symbols[*at as usize].kind != Kind::File
                            && marks.status[slot(*at)] == Some(letter)
                    })
                    .count()
            };
            let (added, modified) = (of('A'), of('M'));
            json!({
                "symbols": added + modified,
                "added": added,
                "modified": modified,
                "between": marks.between,
                "top": range.clone().any(|at| {
                    self.symbols[at as usize].kind == Kind::File
                        && marks.status[slot(at)].is_some()
                }),
            })
        });
        json!({
            "mode": "file",
            "scope": path,
            "crumbs": Self::crumbs(project, path),
            "certain": certain,
            "totals": self.totals(),
            "file": {
                "path": path,
                "name": name(path),
                "status": changed.get(path).map(|kind| kind.to_string()),
                "touched": reach,
                "symbols": types + methods + functions + headings + others,
                "types": types,
                "methods": methods,
                "functions": functions,
                "headings": headings,
                "others": others,
                "inside": inner.len(),
                "out": leaving.len(),
                "in": entering.len(),
                "uses": files(&leaving, |link| link.1),
                "used_by": files(&entering, |link| link.0),
            },
            "children": children,
            "matching": matching,
            "beyond": beyond,
            "around": {"files": linked, "shown": around.len()},
            "nodes": nodes,
            "between": between,
            "backbone": false,
            "edges": edges.iter().map(|(from, to, edge)| self.edge_json(from, to, edge)).collect::<Vec<_>>(),
            "declared_members": file_declared_members,
            "focus": focus,
        })
    }
}
/// How well a word of a search names a place, the best first: it is the
/// whole name, the name starts with it, a word inside the name starts with
/// it, or it only stands somewhere in the name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Named {
    Whole,
    Start,
    Word,
    Part,
}
/// Writes `text` in lower case after what `into` holds: letter by letter,
/// so that a path and the name it ends in stay alike.
pub(super) fn lower_onto(text: &str, into: &mut String) {
    if text.is_ascii() {
        let from = into.len();
        into.push_str(text);
        into[from..].make_ascii_lowercase();
    } else {
        into.extend(text.chars().flat_map(char::to_lowercase));
    }
}
/// Whether a word begins at byte `at` of a name whose lower case is `lower`:
/// after what is neither a letter nor a digit, or where a capital begins a
/// word of a name written `likeThis`.
pub(super) fn begins(name: &str, lower: &str, at: usize) -> bool {
    let Some(before) = lower[..at].chars().next_back() else {
        return true;
    };
    if !before.is_alphanumeric() {
        return true;
    }
    // Capitals are told in the name as it is written, which lies letter for
    // letter over its lower case only where the two are equally long.
    if name.len() != lower.len() || !name.is_char_boundary(at) {
        return false;
    }
    let mut after = name[at..].chars();
    let (Some(before), Some(here)) = (name[..at].chars().next_back(), after.next()) else {
        return false;
    };
    here.is_uppercase()
        && (before.is_lowercase()
            || before.is_numeric()
            || after.next().is_some_and(char::is_lowercase))
}
/// How well `word` names `name`, whose lower case is `lower`; the word is
/// in lower case too. `None` where the name does not hold the word.
pub(super) fn named(name: &str, lower: &str, word: &str) -> Option<Named> {
    if word.is_empty() {
        return None;
    }
    let mut best: Option<Named> = None;
    let mut from = 0;
    while let Some(found) = lower[from..].find(word) {
        let at = from + found;
        let how = if at > 0 {
            if begins(name, lower, at) {
                Named::Word
            } else {
                Named::Part
            }
        } else if lower.len() == word.len() {
            Named::Whole
        } else {
            Named::Start
        };
        best = Some(best.map_or(how, |best| best.min(how)));
        // Further on, the word can at best begin a word again.
        if how <= Named::Word {
            break;
        }
        from = at + lower[at..].chars().next().map_or(1, char::len_utf8);
    }
    best
}
/// How well a word with a slash names a place by its path, both in lower
/// case: what stands before the word's last slash has to end the folder the
/// place lies in, and the rest to begin the place's name. `src/cart` names
/// `lib/src/Cart.php` and a folder `lib/src/cart`, and `src/` all that lies
/// in a folder `src`. A slash at the front is the start of a folder's name.
pub(super) fn pathed(path: &str, word: &str) -> Option<Named> {
    let (folder, name) = path.rsplit_once('/').unwrap_or(("", path));
    let (head, tail) = word.rsplit_once('/')?;
    let lies = folder.ends_with(head) || head.strip_prefix('/') == Some(folder);
    if !lies || !name.starts_with(tail) {
        return None;
    }
    Some(if tail.is_empty() {
        Named::Part
    } else if tail.len() == name.len() {
        Named::Whole
    } else {
        Named::Start
    })
}
/// How well a word names a place by its name together with what holds it,
/// as `cart.to` names `Cart.total()`: the word has to begin in `held`, which
/// stands before the name, and to end in the name. All are in lower case.
pub(super) fn spans(held: &str, lower: &str, word: &str) -> Option<Named> {
    if held.is_empty() {
        return None;
    }
    if let Some(rest) = word.strip_prefix(held)
        && !rest.is_empty()
        && lower.starts_with(rest)
    {
        return Some(Named::Start);
    }
    word.char_indices()
        .skip(1)
        .any(|(at, _)| held.ends_with(&word[..at]) && lower.starts_with(&word[at..]))
        .then_some(Named::Part)
}
/// A place as a search looks at it. All but `name` is in lower case.
pub(super) struct Sought<'a> {
    /// What it is called, as it is written and in lower case.
    pub(super) name: &'a str,
    pub(super) lower: &'a str,
    /// Another name it goes by: a file without its ending, a method or a
    /// function without its brackets.
    pub(super) also: Option<&'a str>,
    /// What stands before the name where it is shown with what holds it:
    /// `cart.` before the `total()` of `Cart.total()`.
    pub(super) held: &'a str,
    /// Its path, or the path of the file a symbol is in.
    pub(super) path: &'a str,
    /// The path is its own, so that a word with a slash can name it.
    pub(super) own: bool,
}
/// How well the words of a search name a place: as well as the best of them
/// does. Each word has to stand in the name or on the way to it, and one at
/// least in the name; otherwise the place is no answer.
pub(super) fn answers(words: &[String], place: &Sought) -> Option<Named> {
    let mut best: Option<Named> = None;
    for word in words {
        let word = word.as_str();
        let how = if word.contains('/') {
            place.own.then(|| pathed(place.path, word)).flatten()
        } else {
            let other = place.also.and_then(|also| {
                if also == word {
                    Some(Named::Whole)
                } else if also.starts_with(word) {
                    Some(Named::Start)
                } else {
                    None
                }
            });
            match (named(place.name, place.lower, word), other) {
                (Some(one), Some(other)) => Some(one.min(other)),
                (one, other) => one
                    .or(other)
                    .or_else(|| spans(place.held, place.lower, word)),
            }
        };
        match how {
            Some(how) => best = Some(best.map_or(how, |best| best.min(how))),
            None => {
                // Not its name: it may still say where the place lies.
                let way = if place.own {
                    &place.path[..place.path.len() - place.lower.len()]
                } else {
                    place.path
                };
                if !way.contains(word) && !place.held.contains(word) {
                    return None;
                }
            }
        }
    }
    best
}
/// One place a search found: how well it is named, whether it holds what
/// was typed with capitals as it was typed, how much is linked with it, how
/// long its name is, and which folder (0), file (1) or symbol (2) it is, by
/// its number among those.
pub(super) struct Hit {
    pub(super) how: Named,
    pub(super) cased: bool,
    pub(super) weight: u64,
    pub(super) len: u32,
    pub(super) class: u8,
    pub(super) id: u32,
}
impl Index {
    /// The folders, the files and the names of the whole graph that a
    /// search names, whatever folder a picture has open: the `FOUND` best
    /// of them, and how many there are of each.
    ///
    /// A search is a few words. Each has to stand in the name of a place or
    /// on the way to it, and one at least in the name: `total cart` finds
    /// `total()` in `Cart.php` and not everything under a folder `cart`. A
    /// word with a slash is the end of a path, as `pathed` tells. Letters
    /// match whatever their case.
    ///
    /// The best is what is named most exactly: by its whole name, then by
    /// the start of it, then by the start of a word in it. A file is named
    /// by its name without the ending too, and a method or a function by
    /// its name without its class and its brackets. A word typed with a
    /// capital puts first what holds it as it was typed. Among equals the
    /// most linked comes first, then the shortest name.
    pub fn find(&self, query: &str) -> Value {
        let mut words = Vec::new();
        for word in query.split_whitespace().take(WORDS) {
            let mut lower = String::new();
            lower_onto(word, &mut lower);
            words.push(lower);
        }
        if words.is_empty() {
            return json!({
                "mode": "find",
                "query": "",
                "found": 0,
                "folders": 0,
                "files": 0,
                "names": 0,
                "rows": [],
            });
        }
        // How much is linked with each symbol, each file and each folder.
        let mut linked = vec![0u64; self.symbols.len()];
        for (from, to, ..) in &self.uses {
            linked[*from as usize] += 1;
            linked[*to as usize] += 1;
        }
        let busy = (0..self.files.len() as u32)
            .map(|file| self.range(file).map(|at| linked[at as usize]).sum::<u64>())
            .collect::<Vec<_>>();
        // Every folder a file lies in, with its files and their links.
        let mut folders: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
        for (file, path) in self.files.iter().enumerate() {
            let mut folder = holder(path);
            while !folder.is_empty() {
                let entry = folders.entry(folder).or_default();
                entry.0 += 1;
                entry.1 += busy[file];
                folder = holder(folder);
            }
        }
        let folders = folders.into_iter().collect::<Vec<_>>();
        // The words typed with a capital, each with its lower case, and
        // whether a place holds each of them as it was typed: in its name,
        // where the name holds the word at all, and else on the way to it.
        let capitals = query
            .split_whitespace()
            .take(WORDS)
            .zip(&words)
            .filter(|(typed, _)| typed.chars().any(char::is_uppercase))
            .collect::<Vec<_>>();
        let cased = |name: &str, lower: &str, ways: [&str; 2]| {
            !capitals.is_empty()
                && capitals.iter().all(|(typed, small)| {
                    if lower.contains(small.as_str()) {
                        name.contains(typed)
                    } else {
                        ways.iter().any(|way| way.contains(typed))
                    }
                })
        };
        let mut hits = Vec::new();
        let (mut lower, mut path, mut text) = (String::new(), String::new(), String::new());
        // How well a folder or a file is named, by its name and its own
        // path, whether as it was typed, and how long its name is. A file
        // goes by its name without the ending, too.
        let mut place = |whole: &str, file: bool| {
            let called = name(whole);
            lower.clear();
            lower_onto(called, &mut lower);
            path.clear();
            lower_onto(whole, &mut path);
            let how = answers(
                &words,
                &Sought {
                    name: called,
                    lower: &lower,
                    also: lower
                        .rsplit_once('.')
                        .map(|(stem, _)| stem)
                        .filter(|stem| file && !stem.is_empty()),
                    held: "",
                    path: &path,
                    own: true,
                },
            )?;
            Some((how, cased(called, &lower, [whole, ""]), called.len() as u32))
        };
        for (id, (folder, (_, weight))) in folders.iter().enumerate() {
            if let Some((how, cased, len)) = place(folder, false) {
                hits.push(Hit {
                    how,
                    cased,
                    weight: *weight,
                    len,
                    class: 0,
                    id: id as u32,
                });
            }
        }
        let in_folders = hits.len();
        for (file, whole) in self.files.iter().enumerate() {
            if let Some((how, cased, len)) = place(whole, true) {
                hits.push(Hit {
                    how,
                    cased,
                    weight: busy[file],
                    len,
                    class: 1,
                    id: file as u32,
                });
            }
        }
        let in_files = hits.len() - in_folders;
        for (file, whole) in self.files.iter().enumerate() {
            path.clear();
            lower_onto(whole, &mut path);
            for at in self.range(file as u32) {
                if matches!(self.symbols[at as usize].kind, Kind::File | Kind::Note) {
                    continue;
                }
                // A method is shown with its class before it, `Cart.total()`,
                // and is named by its own name.
                text.clear();
                self.show(at, &mut text);
                let label = self.label(at);
                let own = label.strip_prefix('.').unwrap_or(label);
                let cut = text.len() - own.len();
                lower.clear();
                lower_onto(&text[..cut], &mut lower);
                let held = lower.len();
                lower_onto(own, &mut lower);
                let (held, called) = lower.split_at(held);
                let how = answers(
                    &words,
                    &Sought {
                        name: own,
                        lower: called,
                        also: called.strip_suffix("()"),
                        held,
                        path: &path,
                        own: false,
                    },
                );
                if let Some(how) = how {
                    hits.push(Hit {
                        how,
                        cased: cased(own, called, [&text[..cut], whole]),
                        weight: linked[at as usize],
                        len: own.len() as u32,
                        class: 2,
                        id: at,
                    });
                }
            }
        }
        let found = hits.len();
        let before = |a: &Hit, b: &Hit| {
            a.how
                .cmp(&b.how)
                .then(b.cased.cmp(&a.cased))
                .then(b.weight.cmp(&a.weight))
                .then(a.len.cmp(&b.len))
                .then(a.class.cmp(&b.class))
                .then(a.id.cmp(&b.id))
        };
        if hits.len() > FOUND {
            hits.select_nth_unstable_by(FOUND - 1, before);
            hits.truncate(FOUND);
        }
        hits.sort_by(before);
        // A symbol is told by the name the picture of its file selects it by.
        let mut keys: HashMap<u32, Vec<String>> = HashMap::new();
        let rows = hits
            .iter()
            .map(|hit| match hit.class {
                0 => {
                    let (path, (files, _)) = folders[hit.id as usize];
                    json!({
                        "path": path,
                        "name": name(path),
                        "what": "folder",
                        "in": holder(path),
                        "files": files,
                    })
                }
                1 => {
                    let path = self.files[hit.id as usize].as_str();
                    json!({
                        "path": path,
                        "name": name(path),
                        "what": "file",
                        "in": holder(path),
                        "symbols": self.holds(hit.id),
                    })
                }
                _ => {
                    let symbol = &self.symbols[hit.id as usize];
                    let slot = (hit.id - self.range(symbol.file).start) as usize;
                    let selected = keys
                        .entry(symbol.file)
                        .or_insert_with(|| self.keys(symbol.file));
                    // Where its own name begins in the name it is shown by.
                    let shown = self.shown(hit.id);
                    let label = self.label(hit.id);
                    let own = shown.len() - label.strip_prefix('.').unwrap_or(label).len();
                    json!({
                        "path": selected[slot],
                        "name": shown,
                        "own": own,
                        "what": symbol.kind.word(),
                        "file": self.files[symbol.file as usize],
                        "line": symbol.line,
                    })
                }
            })
            .collect::<Vec<_>>();
        json!({
            "mode": "find",
            "query": query.trim(),
            "found": found,
            "folders": in_folders,
            "files": in_files,
            "names": found - in_folders - in_files,
            "rows": rows,
        })
    }

    /// Relative paths of all files indexed by this graph.
    pub fn indexed_files(&self) -> &[String] {
        &self.files
    }
}
