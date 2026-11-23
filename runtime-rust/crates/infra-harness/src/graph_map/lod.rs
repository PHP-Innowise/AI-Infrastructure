use super::*;

/// Paginates any lens response object deterministically while pinning anchors (`start`, `end`,
/// `focus`, or caller hints), never slicing past `nodes.len()`, never setting `has_more` when
/// `returned == 0`, and sorting `cross_page_edges` by descending weight.
pub(crate) fn paginate_lens_view(
    obj: &mut serde_json::Map<String, Value>,
    offset: usize,
    limit: Option<usize>,
    anchor_hints: &[&str],
) {
    let mut focus_anchors: Vec<String> = anchor_hints.iter().map(|s| (*s).to_owned()).collect();
    if let Some(focus_path) = obj
        .get("focus")
        .and_then(|f| f.get("path").or_else(|| f.get("id")))
        .and_then(Value::as_str)
        && !focus_path.is_empty()
        && !focus_anchors.iter().any(|s| s == focus_path)
    {
        focus_anchors.push(focus_path.to_owned());
    }
    if let Some(origin_path) = obj
        .get("cycle")
        .and_then(|c| c.get("origin"))
        .and_then(|o| o.get("path"))
        .and_then(Value::as_str)
        && !origin_path.is_empty()
        && !focus_anchors.iter().any(|s| s == origin_path)
    {
        focus_anchors.push(origin_path.to_owned());
    }

    let is_anchor = |node: &Value| -> bool {
        if node.get("start").and_then(Value::as_bool) == Some(true)
            || node.get("end").and_then(Value::as_bool) == Some(true)
        {
            return true;
        }
        node.get("path")
            .or_else(|| node.get("id"))
            .and_then(Value::as_str)
            .is_some_and(|key| focus_anchors.iter().any(|a| a == key))
    };

    let mut total = 0usize;
    let is_paginated = offset > 0 || limit.is_some();
    if let Some(nodes) = obj.get_mut("nodes").and_then(Value::as_array_mut) {
        total = nodes.len();
        if is_paginated {
            let mut anchored = Vec::new();
            let mut rest = Vec::with_capacity(nodes.len());
            for node in nodes.drain(..) {
                if is_anchor(&node) {
                    anchored.push(node);
                } else {
                    rest.push(node);
                }
            }
            anchored.extend(rest);
            let start = offset.min(anchored.len());
            let end = limit.map_or(anchored.len(), |lim| {
                start.saturating_add(lim).min(anchored.len())
            });
            *nodes = anchored[start..end].to_vec();
        }
    }

    let kept_keys: Option<BTreeSet<String>> = is_paginated.then(|| {
        obj.get("nodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|n| {
                n.get("path")
                    .or_else(|| n.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect()
    });

    let mut cross_page_edges = Vec::new();
    if let Some(ref kept) = kept_keys
        && let Some(edges) = obj.get_mut("edges").and_then(Value::as_array_mut)
    {
        let mut filtered = Vec::with_capacity(edges.len());
        let mut cross: Vec<(String, String, u64)> = Vec::new();
        for edge in edges.drain(..) {
            let from = edge
                .get("from")
                .or_else(|| edge.get("source"))
                .and_then(Value::as_str);
            let to = edge
                .get("to")
                .or_else(|| edge.get("target"))
                .and_then(Value::as_str);
            match (from, to) {
                (Some(f), Some(t)) if kept.contains(f) && kept.contains(t) => {
                    filtered.push(edge);
                }
                (Some(f), Some(t)) if kept.contains(f) != kept.contains(t) => {
                    let links = edge
                        .get("links")
                        .or_else(|| edge.get("weight"))
                        .and_then(Value::as_u64)
                        .unwrap_or(1);
                    cross.push((f.to_owned(), t.to_owned(), links));
                }
                _ => {}
            }
        }
        cross.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
        cross_page_edges = cross
            .into_iter()
            .take(64)
            .map(|(from, to, links)| json!({"from": from, "to": to, "links": links}))
            .collect();
        *edges = filtered;
    }

    let returned = obj
        .get("nodes")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let has_more = returned > 0 && offset.saturating_add(returned) < total;
    obj.insert(
        "paging".into(),
        json!({
            "offset": offset,
            "limit": limit,
            "total": total,
            "returned": returned,
            "has_more": has_more,
            "next_offset": has_more.then_some(offset.saturating_add(returned)),
            "cross_page_edges": cross_page_edges,
        }),
    );
}

/// Progressive level of detail supported by `graph.map`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lod {
    Crate,
    Folder,
    File,
    Symbol,
}

impl Lod {
    /// Parses query parameter values `crate`, `folder`, `file`, or `symbol`.
    pub fn parse(raw: Option<&str>) -> Result<Option<Self>> {
        let Some(text) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        match text.to_ascii_lowercase().as_str() {
            "crate" => Ok(Some(Self::Crate)),
            "folder" => Ok(Some(Self::Folder)),
            "file" => Ok(Some(Self::File)),
            "symbol" => Ok(Some(Self::Symbol)),
            _ => Err(HarnessError::invalid("Invalid code graph lod.")),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Crate => "crate",
            Self::Folder => "folder",
            Self::File => "file",
            Self::Symbol => "symbol",
        }
    }
}

pub(super) fn path_matches_scope(scope: &str, path: &str) -> bool {
    let scope = scope.trim_matches('/');
    scope.is_empty()
        || path == scope
        || path
            .strip_prefix(scope)
            .is_some_and(|rest| rest.starts_with('/'))
}

pub(super) fn discover_manifest_dirs(root: &Path, files: &[String]) -> Vec<String> {
    const MANIFESTS: &[&str] = &[
        "Cargo.toml",
        "package.json",
        "tsconfig.json",
        "jsconfig.json",
        "composer.json",
        "go.mod",
        "pyproject.toml",
        "setup.py",
        "setup.cfg",
        "Package.swift",
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "Gemfile",
        "deno.json",
        "deno.jsonc",
    ];
    let mut dirs = BTreeSet::new();
    // Always inspect the repository root `""` so single-crate or workspace-root files group under `"."`.
    dirs.insert(String::new());
    for file in files {
        let mut cursor = file.as_str();
        while let Some((dir, _)) = cursor.rsplit_once('/') {
            dirs.insert(dir.to_owned());
            cursor = dir;
        }
    }
    let mut roots: Vec<String> = dirs
        .into_iter()
        .filter(|dir| {
            let abs = if dir.is_empty() {
                root.to_path_buf()
            } else {
                root.join(dir)
            };
            MANIFESTS.iter().any(|m| abs.join(m).is_file())
        })
        .collect();
    // Match deepest crate/package directory first (root `""` sorts last).
    roots.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
    roots
}

impl Index {
    pub(super) fn manifest_dirs(&self) -> &[String] {
        self.manifest_dirs
            .get_or_init(|| discover_manifest_dirs(&self.root, &self.files))
    }
}

/// Maps a semantic hub path (`__semantic__/*`, `sem:*`, or `file#L1:table:*`) into its dedicated
/// virtual semantic namespace bucket so semantic hubs never fold into an arbitrary code folder or
/// root `"."` crate.
pub(super) fn semantic_namespace_bucket_for(raw: &str) -> Option<&'static str> {
    let trimmed = raw.trim();
    if let Some(sub) = trimmed.strip_prefix("__semantic__/") {
        let head = sub
            .split('/')
            .next()
            .unwrap_or(sub)
            .split('#')
            .next()
            .unwrap_or(sub);
        return Some(match head {
            "routes" => "__semantic__/routes",
            "tables" => "__semantic__/tables",
            "events" => "__semantic__/events",
            "services" => "__semantic__/services",
            _ => "__semantic__/contracts",
        });
    }
    if trimmed.starts_with("sem:") {
        return Some(crate::graph_ir::semantic_namespace_bucket(trimmed));
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
            return Some(crate::graph_ir::semantic_namespace_bucket(candidate));
        }
    }
    None
}

pub(super) fn crate_bucket(manifest_dirs: &[String], path: &str) -> String {
    if let Some(sem_bucket) = semantic_namespace_bucket_for(path) {
        return sem_bucket.to_owned();
    }
    if let Some(dir) = manifest_dirs.iter().find(|dir| {
        dir.is_empty()
            || path
                .strip_prefix(dir.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    }) {
        return if dir.is_empty() {
            ".".to_owned()
        } else {
            dir.clone()
        };
    }
    match path.split_once('/') {
        Some((top, _)) if !top.is_empty() => top.to_owned(),
        _ => ".".to_owned(),
    }
}

pub(super) fn folder_bucket(path: &str) -> String {
    if let Some(sem_bucket) = semantic_namespace_bucket_for(path) {
        return sem_bucket.to_owned();
    }
    match path.rsplit_once('/') {
        Some((dir, _)) if !dir.is_empty() => dir.to_owned(),
        _ => ".".to_owned(),
    }
}

pub(super) fn node_bucket(manifest_dirs: &[String], lod: Lod, path: &str) -> String {
    match lod {
        Lod::Crate => crate_bucket(manifest_dirs, path),
        Lod::Folder => folder_bucket(path),
        Lod::File | Lod::Symbol => path.to_owned(),
    }
}

pub(crate) struct LodFilter<'a> {
    pub(crate) scope: &'a str,
    pub(crate) focus: Option<&'a str>,
    pub(crate) filter: &'a str,
    pub(crate) certain: bool,
    pub(crate) offset: usize,
    pub(crate) limit: Option<usize>,
}

pub(crate) fn rebuild_default_map_at_lod(
    index: &Index,
    view: &mut Value,
    lod: Lod,
    opts: &LodFilter<'_>,
) -> bool {
    let LodFilter {
        scope,
        focus,
        filter,
        certain,
        offset,
        limit,
    } = *opts;
    let paginated = offset > 0 || limit.is_some();
    let scope = scope.trim_matches('/');
    let scope = if scope.is_empty() || index.folder(scope) || index.opens(scope).is_some() {
        scope
    } else {
        ""
    };
    let needle = filter.trim().to_ascii_lowercase();
    let max_default = NORMAL_LIMITS.shown;

    if lod == Lod::Symbol {
        let mut sym_in = vec![0u64; index.symbols.len()];
        let mut sym_out = vec![0u64; index.symbols.len()];
        for &(from, to, _, guessed) in &index.uses {
            if (certain && guessed) || from == to {
                continue;
            }
            sym_out[from as usize] += 1;
            sym_in[to as usize] += 1;
        }
        let mut file_keys_cache: HashMap<u32, Vec<String>> = HashMap::new();
        let focus_trim = focus.map(str::trim).filter(|s| !s.is_empty());
        let mut seed_syms: BTreeSet<u32> = BTreeSet::new();
        let mut focus_is_path_scope = false;
        if let Some(f) = focus_trim {
            for (idx, sym) in index.symbols.iter().enumerate() {
                if matches!(sym.kind, Kind::File | Kind::Note) {
                    continue;
                }
                let file_path = index.files[sym.file as usize].as_str();
                if path_matches_scope(f, file_path) {
                    seed_syms.insert(idx as u32);
                    focus_is_path_scope = true;
                } else {
                    let keys = file_keys_cache
                        .entry(sym.file)
                        .or_insert_with(|| index.keys(sym.file));
                    let k = &keys[idx - index.first[sym.file as usize] as usize];
                    if k == f {
                        seed_syms.insert(idx as u32);
                    }
                }
            }
        }
        let allowed_focus: Option<HashSet<u32>> = focus_trim.map(|_| {
            let mut allowed: HashSet<u32> = seed_syms.iter().copied().collect();
            if !focus_is_path_scope {
                for &(from, to, _, guessed) in &index.uses {
                    if (certain && guessed) || from == to {
                        continue;
                    }
                    if seed_syms.contains(&from) {
                        allowed.insert(to);
                    }
                    if seed_syms.contains(&to) {
                        allowed.insert(from);
                    }
                }
            }
            allowed
        });

        let mut candidates: Vec<(u32, String, u64, u64)> = Vec::new();
        for (idx, sym) in index.symbols.iter().enumerate() {
            if matches!(sym.kind, Kind::File | Kind::Note) {
                continue;
            }
            let sym_id = idx as u32;
            let file_path = index.files[sym.file as usize].as_str();
            let is_seed = seed_syms.contains(&sym_id);
            let allowed = match &allowed_focus {
                Some(set) => set.contains(&sym_id),
                None => path_matches_scope(scope, file_path),
            };
            if !allowed {
                continue;
            }
            let keys = file_keys_cache
                .entry(sym.file)
                .or_insert_with(|| index.keys(sym.file));
            let key = keys[idx - index.first[sym.file as usize] as usize].clone();
            if !is_seed && !needle.is_empty() && !key.to_ascii_lowercase().contains(&needle) {
                continue;
            }
            candidates.push((sym_id, key, sym_out[idx], sym_in[idx]));
        }
        let total_matching = candidates.len();
        candidates.sort_by(|a, b| {
            seed_syms
                .contains(&b.0)
                .cmp(&seed_syms.contains(&a.0))
                .then_with(|| (b.2 + b.3).cmp(&(a.2 + a.3)))
                .then_with(|| a.1.cmp(&b.1))
        });
        if !paginated {
            candidates.truncate(max_default);
        }
        let kept_set: HashMap<u32, String> = candidates
            .iter()
            .map(|(id, key, _, _)| (*id, key.clone()))
            .collect();
        let mut edge_flows: BTreeMap<(String, String), EdgeFlow> = BTreeMap::new();
        for &(from, to, kind, guessed) in &index.uses {
            if (certain && guessed) || from == to {
                continue;
            }
            if let (Some(fk), Some(tk)) = (kept_set.get(&from), kept_set.get(&to)) {
                edge_flows
                    .entry((fk.clone(), tk.clone()))
                    .or_default()
                    .add_link(kind, guessed);
            }
        }
        let mut edges: Vec<((String, String), EdgeFlow)> = edge_flows.into_iter().collect();
        edges.sort_by(|a, b| b.1.flow.links.cmp(&a.1.flow.links).then(a.0.cmp(&b.0)));
        let between = edges.len();
        if !paginated {
            edges.truncate(NORMAL_LIMITS.drawn);
        }
        view["children"] = json!(total_matching);
        view["matching"] = json!(total_matching);
        view["between"] = json!(between);
        view["nodes"] = Value::Array(
            candidates
                .into_iter()
                .map(|(id, key, out_c, in_c)| {
                    let sym = &index.symbols[id as usize];
                    json!({
                        "path": key,
                        "name": index.shown(id),
                        "file": index.files[sym.file as usize],
                        "line": sym.line,
                        "kind": sym.kind.word(),
                        "folder": false,
                        "symbol": true,
                        "out": out_c,
                        "in": in_c,
                    })
                })
                .collect(),
        );
        view["edges"] = Value::Array(
            edges
                .iter()
                .map(|((f, t), flow)| index.edge_json(f, t, flow))
                .collect(),
        );
        if let Some(f) = focus_trim
            && !seed_syms.is_empty()
        {
            let mut key_for_sym = |sym_id: u32| -> String {
                let s = &index.symbols[sym_id as usize];
                let keys = file_keys_cache
                    .entry(s.file)
                    .or_insert_with(|| index.keys(s.file));
                keys[sym_id as usize - index.first[s.file as usize] as usize].clone()
            };
            let format_side =
                |flows_map: BTreeMap<(u32, String), EdgeFlow>| -> (Vec<Value>, usize) {
                    let mut rows: Vec<((u32, String), EdgeFlow)> = flows_map.into_iter().collect();
                    rows.sort_by(|a, b| {
                        b.1.flow.links.cmp(&a.1.flow.links).then(a.0.1.cmp(&b.0.1))
                    });
                    let more = rows.len().saturating_sub(LISTED);
                    rows.truncate(LISTED);
                    let list = rows
                        .into_iter()
                        .map(|((other_id, other_key), flow)| {
                            let other = &index.symbols[other_id as usize];
                            json!({
                                "path": other_key,
                                "name": index.shown(other_id),
                                "file": index.files[other.file as usize],
                                "line": other.line,
                                "kind": other.kind.word(),
                                "folder": false,
                                "symbol": true,
                                "inside": kept_set.contains_key(&other_id),
                                "links": flow.flow.links,
                                "guessed": flow.flow.guessed,
                            })
                        })
                        .collect();
                    (list, more)
                };
            let mut out_count = 0u64;
            let mut in_count = 0u64;
            let mut out_flows: BTreeMap<(u32, String), EdgeFlow> = BTreeMap::new();
            let mut in_flows: BTreeMap<(u32, String), EdgeFlow> = BTreeMap::new();
            let mut focus_kinds: BTreeMap<u16, u64> = BTreeMap::new();
            for &(from, to, kind, guessed) in &index.uses {
                if (certain && guessed) || from == to {
                    continue;
                }
                let from_in = seed_syms.contains(&from);
                let to_in = seed_syms.contains(&to);
                if from_in && !to_in {
                    out_count += 1;
                    *focus_kinds.entry(kind).or_default() += 1;
                    let tk = key_for_sym(to);
                    out_flows
                        .entry((to, tk))
                        .or_default()
                        .add_link(kind, guessed);
                } else if to_in && !from_in {
                    in_count += 1;
                    let fk = key_for_sym(from);
                    in_flows
                        .entry((from, fk))
                        .or_default()
                        .add_link(kind, guessed);
                }
            }
            let (uses, uses_more) = format_side(out_flows);
            let (used_by, used_by_more) = format_side(in_flows);
            if !focus_is_path_scope && let Some(&at) = seed_syms.iter().next() {
                let focus_key = key_for_sym(at);
                let sym = &index.symbols[at as usize];
                view["focus"] = json!({
                    "path": focus_key,
                    "name": index.shown(at),
                    "kind": sym.kind.word(),
                    "line": sym.line,
                    "file": index.files[sym.file as usize],
                    "symbol": true,
                    "folder": false,
                    "inside": true,
                    "out": sym_out[at as usize],
                    "in": sym_in[at as usize],
                    "uses": uses,
                    "uses_more": uses_more,
                    "used_by": used_by,
                    "used_by_more": used_by_more,
                    "kinds": index.kinds_json(&focus_kinds),
                });
            } else if focus_is_path_scope {
                let is_folder = matches!(index.reach(f), Some(Reach::Folder(_)));
                let mut focus_files: HashSet<u32> = HashSet::new();
                for &sid in &seed_syms {
                    focus_files.insert(index.symbols[sid as usize].file);
                }
                view["focus"] = json!({
                    "path": f,
                    "name": if f == "." { "." } else { name(f) },
                    "folder": is_folder,
                    "inside": true,
                    "files": focus_files.len(),
                    "symbols": seed_syms.len(),
                    "out": out_count,
                    "in": in_count,
                    "uses": uses,
                    "uses_more": uses_more,
                    "used_by": used_by,
                    "used_by_more": used_by_more,
                    "kinds": index.kinds_json(&focus_kinds),
                });
            }
        }
        if let Some(obj) = view.as_object_mut() {
            obj.remove("paging");
        }
        return true;
    }

    if !matches!(lod, Lod::Crate | Lod::Folder | Lod::File) {
        return false;
    }
    let manifest_dirs: &[String] = if lod == Lod::Crate {
        index.manifest_dirs()
    } else {
        &[]
    };
    let bucket_for_file = |path: &str| -> String { node_bucket(manifest_dirs, lod, path) };

    #[derive(Default)]
    struct BucketTally {
        files: u64,
        symbols: u64,
        within: u64,
        out: u64,
        into: u64,
        is_semantic: bool,
    }
    let mut tallies: BTreeMap<String, BucketTally> = BTreeMap::new();
    let mut file_to_bucket: HashMap<u32, String> = HashMap::new();
    for (id, file) in index.files.iter().enumerate() {
        if !path_matches_scope(scope, file) {
            continue;
        }
        let b = bucket_for_file(file);
        let tally = tallies.entry(b.clone()).or_default();
        tally.files += 1;
        tally.symbols += index.holds(id as u32);
        tally.is_semantic |= semantic_namespace_bucket_for(file).is_some();
        file_to_bucket.insert(id as u32, b);
    }
    let mut flows: BTreeMap<(String, String), EdgeFlow> = BTreeMap::new();
    let mut focus_kinds: BTreeMap<u16, u64> = BTreeMap::new();
    for (from, to, pair) in &index.pairs {
        let w = pair.weight(certain);
        if w == 0 {
            continue;
        }
        let (Some(sb), Some(tb)) = (file_to_bucket.get(from), file_to_bucket.get(to)) else {
            continue;
        };
        if sb == tb {
            if let Some(t) = tallies.get_mut(sb) {
                t.within += w;
            }
            continue;
        }
        if let Some(t) = tallies.get_mut(sb) {
            t.out += w;
        }
        if let Some(t) = tallies.get_mut(tb) {
            t.into += w;
        }
        if focus == Some(sb.as_str()) {
            add_kind_counts(&mut focus_kinds, pair.kinds_for(certain));
        }
        flows
            .entry((sb.clone(), tb.clone()))
            .or_default()
            .add_pair(pair, certain);
    }

    let total = tallies.len();
    let mut rows: Vec<(String, BucketTally)> = tallies
        .into_iter()
        .filter(|(k, _)| needle.is_empty() || k.to_ascii_lowercase().contains(&needle))
        .collect();
    let matching = rows.len();
    rows.sort_by(|(a_k, a), (b_k, b)| {
        (b.out + b.into + b.within)
            .cmp(&(a.out + a.into + a.within))
            .then(a_k.cmp(b_k))
    });
    let mut beyond = false;
    let focus_bucket = focus.filter(|f| !f.is_empty()).map(|f| {
        let trimmed = f.split('#').next().unwrap_or(f);
        if rows.iter().any(|(k, _)| k == trimmed) {
            trimmed.to_owned()
        } else {
            node_bucket(manifest_dirs, lod, trimmed)
        }
    });
    if !paginated {
        if let Some(ref fb) = focus_bucket
            && let Some(pos) = rows.iter().position(|(k, _)| k == fb)
            && pos >= max_default
        {
            let focused_row = rows.remove(pos);
            rows.truncate(max_default.saturating_sub(1));
            rows.push(focused_row);
            rows.sort_by(|(a_k, a), (b_k, b)| {
                (b.out + b.into + b.within)
                    .cmp(&(a.out + a.into + a.within))
                    .then(a_k.cmp(b_k))
            });
            beyond = true;
        } else {
            rows.truncate(max_default);
        }
    }
    let kept: HashSet<String> = rows.iter().map(|(k, _)| k.clone()).collect();
    let mut edges: Vec<((String, String), EdgeFlow)> = flows
        .iter()
        .filter(|((s, t), _)| kept.contains(s) && kept.contains(t))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    edges.sort_by(|a, b| b.1.flow.links.cmp(&a.1.flow.links).then(a.0.cmp(&b.0)));
    let between = edges.len();
    if !paginated {
        edges.truncate(NORMAL_LIMITS.drawn);
    }
    let focused_entry = focus_bucket
        .as_deref()
        .and_then(|fb| rows.iter().find(|(k, _)| k == fb));
    if let Some((fb_path, fb_tally)) = focused_entry {
        let side = |outgoing: bool| -> (Vec<Value>, usize) {
            let mut side_rows: Vec<(&str, Flow)> = flows
                .iter()
                .filter_map(|((s, t), edge)| {
                    let other = if outgoing {
                        (s == fb_path).then_some(t.as_str())
                    } else {
                        (t == fb_path).then_some(s.as_str())
                    }?;
                    Some((other, edge.flow))
                })
                .collect();
            side_rows.sort_by(|(a, l), (b, r)| r.links.cmp(&l.links).then(a.cmp(b)));
            let more = side_rows.len().saturating_sub(LISTED);
            side_rows.truncate(LISTED);
            let out = side_rows
                .into_iter()
                .map(|(p, flow)| {
                    json!({
                        "path": p,
                        "name": if p == "." { "." } else { name(p) },
                        "folder": matches!(lod, Lod::Crate | Lod::Folder),
                        "inside": true,
                        "links": flow.links,
                        "guessed": flow.guessed,
                    })
                })
                .collect();
            (out, more)
        };
        let (uses, uses_more) = side(true);
        let (used_by, used_by_more) = side(false);
        view["focus"] = json!({
            "path": fb_path,
            "name": if fb_path == "." { "." } else { name(fb_path) },
            "folder": matches!(lod, Lod::Crate | Lod::Folder),
            "inside": true,
            "files": fb_tally.files,
            "symbols": fb_tally.symbols,
            "within": fb_tally.within,
            "out": fb_tally.out,
            "in": fb_tally.into,
            "uses": uses,
            "uses_more": uses_more,
            "used_by": used_by,
            "used_by_more": used_by_more,
            "kinds": index.kinds_json(&focus_kinds),
        });
    }
    if view.get("frames").is_none() {
        view["frames"] = json!([]);
    }
    if view.get("backbone").is_none() {
        view["backbone"] = json!(false);
    }
    view["beyond"] = json!(beyond);
    view["children"] = json!(total);
    view["matching"] = json!(matching);
    view["between"] = json!(between);
    view["nodes"] = Value::Array(
        rows.into_iter()
            .map(|(k, t)| {
                let is_folder = matches!(lod, Lod::Crate | Lod::Folder);
                let mut obj = json!({
                    "path": k,
                    "name": if k == "." { ".".to_owned() } else { name(&k).to_owned() },
                    "folder": is_folder,
                    "files": t.files,
                    "symbols": t.symbols,
                    "within": t.within,
                    "out": t.out,
                    "in": t.into,
                });
                if t.is_semantic {
                    obj["node_kind"] = json!("semantic");
                    obj["semantic"] = json!(true);
                }
                obj
            })
            .collect(),
    );
    view["edges"] = Value::Array(
        edges
            .iter()
            .map(|((f, t), flow)| index.edge_json(f, t, flow))
            .collect(),
    );
    if let Some(obj) = view.as_object_mut() {
        obj.remove("paging");
    }
    true
}

pub(super) fn rollup_view_to_lod(index: &Index, view: &mut Value, lod: Lod) -> bool {
    if !matches!(lod, Lod::Crate | Lod::Folder) {
        return false;
    }
    let Some(nodes) = view.get("nodes").and_then(Value::as_array) else {
        return false;
    };
    let manifest_dirs: &[String] = if lod == Lod::Crate {
        index.manifest_dirs()
    } else {
        &[]
    };

    #[derive(Default)]
    struct Bucket {
        files: u64,
        symbols: u64,
        within: u64,
        out: u64,
        into: u64,
        folder: bool,
        is_semantic: bool,
        changed: u64,
        touched: u64,
        patterns: u64,
        shell_read: u64,
        opened: bool,
        start: bool,
        end: bool,
        focus_target: bool,
        statuses: BTreeSet<String>,
        suspect: Option<u64>,
        blast: Option<(u64, f64)>,
        circle: Option<Value>,
        arch_layer: Option<Option<Value>>,
        role: Option<Option<String>>,
    }
    let mut buckets: BTreeMap<String, Bucket> = BTreeMap::new();
    let mut path_to_bucket: HashMap<String, String> = HashMap::new();
    for node in nodes {
        let Some(raw_path) = node
            .get("path")
            .or_else(|| node.get("id"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let node_is_sem = node.get("node_kind").and_then(Value::as_str) == Some("semantic")
            || node.get("semantic").and_then(Value::as_bool) == Some(true)
            || semantic_namespace_bucket_for(raw_path).is_some();
        let file_part = raw_path.split('#').next().unwrap_or(raw_path);
        let b = if node_is_sem {
            semantic_namespace_bucket_for(raw_path)
                .unwrap_or("__semantic__/contracts")
                .to_owned()
        } else {
            node_bucket(manifest_dirs, lod, file_part)
        };
        path_to_bucket.insert(raw_path.to_owned(), b.clone());
        let entry = buckets.entry(b.clone()).or_default();
        entry.is_semantic |= node_is_sem;
        entry.files += node.get("files").and_then(Value::as_u64).unwrap_or(1);
        entry.symbols += node.get("symbols").and_then(Value::as_u64).unwrap_or(0);
        entry.within += node.get("within").and_then(Value::as_u64).unwrap_or(0);
        entry.folder |= matches!(lod, Lod::Crate | Lod::Folder)
            || (b != "."
                && (node.get("folder").and_then(Value::as_bool).unwrap_or(false)
                    || file_part.contains('/')));
        entry.changed += node
            .get("changed")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| u64::from(node.get("status").is_some()));
        entry.touched += node.get("touched").and_then(Value::as_u64).unwrap_or(0);
        entry.patterns += node.get("patterns").and_then(Value::as_u64).unwrap_or(0);
        entry.shell_read += node.get("shell_read").and_then(Value::as_u64).unwrap_or(0);
        entry.opened |= node.get("opened").and_then(Value::as_bool) == Some(true);
        entry.start |= node.get("start").and_then(Value::as_bool) == Some(true);
        entry.end |= node.get("end").and_then(Value::as_bool) == Some(true);
        entry.focus_target |= node.get("focus_target").and_then(Value::as_bool) == Some(true);
        if let Some(st) = node.get("status").and_then(Value::as_str) {
            entry.statuses.insert(st.to_owned());
        }
        if let Some(rank) = node.get("suspect").and_then(Value::as_u64) {
            entry.suspect = Some(entry.suspect.map_or(rank, |prev| prev.min(rank)));
        }
        if let Some(blast_obj) = node.get("blast").filter(|v| v.is_object()) {
            let hop = blast_obj.get("hop").and_then(Value::as_u64).unwrap_or(1);
            let left = blast_obj.get("left").and_then(Value::as_f64).unwrap_or(0.0);
            entry.blast = Some(match entry.blast {
                Some((ph, pl)) => (ph.min(hop), pl.max(left)),
                None => (hop, left),
            });
        }
        if let Some(c) = node.get("circle").filter(|v| !v.is_null())
            && (entry.circle.is_none() || c.is_object())
        {
            entry.circle = Some(c.clone());
        }
        if let Some(layer) = node.get("architecture_layer").filter(|v| v.is_object()) {
            match &mut entry.arch_layer {
                None => entry.arch_layer = Some(Some(layer.clone())),
                Some(Some(prev)) if prev.get("id") != layer.get("id") => {
                    entry.arch_layer = Some(None);
                }
                _ => {}
            }
        }
        if let Some(role) = node.get("role").and_then(Value::as_str) {
            match &mut entry.role {
                None => entry.role = Some(Some(role.to_owned())),
                Some(Some(prev)) if prev != role => entry.role = Some(None),
                _ => {}
            }
        }
    }

    #[derive(Default)]
    struct AggEdge {
        links: u64,
        guessed: u64,
        kinds: BTreeMap<String, u64>,
        against: bool,
        added: bool,
        detour: bool,
        kept_violation: bool,
        arch_violation: u64,
        rule_ids: BTreeSet<String>,
        arch_rules: Vec<Value>,
        seen_arch_rules: BTreeSet<(String, String, String, String)>,
    }
    let mut aggregated: BTreeMap<(String, String), AggEdge> = BTreeMap::new();
    if let Some(edges) = view.get("edges").and_then(Value::as_array) {
        for edge in edges {
            let from = edge
                .get("from")
                .or_else(|| edge.get("source"))
                .and_then(Value::as_str);
            let to = edge
                .get("to")
                .or_else(|| edge.get("target"))
                .and_then(Value::as_str);
            let (Some(f), Some(t)) = (from, to) else {
                continue;
            };
            let fb = path_to_bucket
                .get(f)
                .cloned()
                .unwrap_or_else(|| node_bucket(manifest_dirs, lod, f));
            let tb = path_to_bucket
                .get(t)
                .cloned()
                .unwrap_or_else(|| node_bucket(manifest_dirs, lod, t));
            let links = edge
                .get("links")
                .or_else(|| edge.get("weight"))
                .and_then(Value::as_u64)
                .unwrap_or(1);
            let guessed = edge.get("guessed").and_then(Value::as_u64).unwrap_or(0);
            if fb == tb {
                if let Some(b) = buckets.get_mut(&fb) {
                    b.within += links;
                }
            } else {
                if let Some(b) = buckets.get_mut(&fb) {
                    b.out += links;
                }
                if let Some(b) = buckets.get_mut(&tb) {
                    b.into += links;
                }
                let entry = aggregated.entry((fb, tb)).or_default();
                entry.links += links;
                entry.guessed += guessed;
                entry.against |= edge.get("against").and_then(Value::as_bool) == Some(true);
                entry.added |= edge.get("added").and_then(Value::as_bool) == Some(true);
                entry.detour |= edge.get("detour").and_then(Value::as_bool) == Some(true);
                entry.kept_violation |=
                    edge.get("kept").and_then(Value::as_str) == Some("violation");
                entry.arch_violation += edge
                    .get("architecture_violation")
                    .and_then(|v| {
                        v.as_u64()
                            .or_else(|| (v.as_bool() == Some(true)).then_some(links))
                    })
                    .unwrap_or(0);
                if let Some(ids) = edge.get("rule_ids").and_then(Value::as_array) {
                    for id in ids.iter().filter_map(Value::as_str) {
                        if !id.is_empty() {
                            entry.rule_ids.insert(id.to_owned());
                        }
                    }
                }
                if let Some(rules_arr) = edge.get("architecture_rules").and_then(Value::as_array) {
                    for rule_val in rules_arr {
                        let r_kind = rule_val
                            .get("kind")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        let r_id = rule_val
                            .get("rule_id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        if !r_id.is_empty() {
                            entry.rule_ids.insert(r_id.clone());
                        }
                        let r_rel = rule_val
                            .get("relation")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        let r_target = rule_val
                            .get("target")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        if entry
                            .seen_arch_rules
                            .insert((r_kind, r_id, r_rel, r_target))
                        {
                            entry.arch_rules.push(rule_val.clone());
                        }
                    }
                }
                if let Some(kinds) = edge.get("kinds").and_then(Value::as_array) {
                    for k in kinds {
                        if let Some(name) = k.get("kind").and_then(Value::as_str) {
                            let count = k.get("links").and_then(Value::as_u64).unwrap_or(1);
                            *entry.kinds.entry(name.to_owned()).or_default() += count;
                        }
                    }
                }
            }
        }
    }

    let bucket_total = buckets.len() as u64;
    view["children"] = json!(bucket_total);
    view["matching"] = json!(bucket_total);
    if let Some(frames) = view.get_mut("frames") {
        *frames = json!([]);
    }
    view["nodes"] = Value::Array(
        buckets
            .into_iter()
            .map(|(k, b)| {
                let mut obj = json!({
                    "path": k,
                    "name": if k == "." { ".".to_owned() } else { name(&k).to_owned() },
                    "folder": b.folder,
                    "files": b.files,
                    "symbols": b.symbols,
                    "within": b.within,
                    "out": b.out,
                    "in": b.into,
                });
                if b.is_semantic {
                    obj["node_kind"] = json!("semantic");
                    obj["semantic"] = json!(true);
                }
                if b.changed > 0 {
                    obj["changed"] = json!(b.changed);
                }
                if b.touched > 0 {
                    obj["touched"] = json!(b.touched);
                }
                if b.statuses.len() == 1
                    && let Some(only) = b.statuses.into_iter().next()
                {
                    obj["status"] = json!(only);
                }
                if b.patterns > 0 {
                    obj["patterns"] = json!(b.patterns);
                }
                if b.shell_read > 0 {
                    obj["shell_read"] = json!(b.shell_read);
                }
                if b.opened {
                    obj["opened"] = json!(true);
                }
                if b.start {
                    obj["start"] = json!(true);
                }
                if b.end {
                    obj["end"] = json!(true);
                }
                if b.focus_target {
                    obj["focus_target"] = json!(true);
                }
                if let Some(rank) = b.suspect {
                    obj["suspect"] = json!(rank);
                }
                if let Some((hop, left)) = b.blast {
                    obj["blast"] = json!({"hop": hop, "left": left});
                }
                if let Some(circle) = b.circle {
                    obj["circle"] = circle;
                }
                if let Some(Some(layer)) = b.arch_layer {
                    obj["architecture_layer"] = layer;
                }
                if let Some(Some(role)) = b.role {
                    obj["role"] = json!(role);
                }
                obj
            })
            .collect(),
    );
    let mut edge_rows: Vec<_> = aggregated.into_iter().collect();
    edge_rows.sort_by(|a, b| b.1.links.cmp(&a.1.links).then(a.0.cmp(&b.0)));
    view["between"] = json!(edge_rows.len());
    // Truncate rolled-up edges past DRAWN while reserving up to KEPT_VIOLATIONS slots for
    // architecture/package/semantic breaches so dense rollups never drop violating edges.
    if edge_rows.len() > super::DRAWN {
        let is_violating = |agg: &AggEdge| {
            agg.against
                || agg.arch_violation > 0
                || !agg.rule_ids.is_empty()
                || !agg.arch_rules.is_empty()
        };
        let mut kept_rows = Vec::with_capacity(super::DRAWN + super::layers::KEPT_VIOLATIONS);
        let mut extra_violations = 0usize;
        for (idx, (ends, mut agg)) in edge_rows.into_iter().enumerate() {
            if idx < super::DRAWN {
                kept_rows.push((ends, agg));
            } else if is_violating(&agg) && extra_violations < super::layers::KEPT_VIOLATIONS {
                agg.kept_violation = true;
                extra_violations += 1;
                kept_rows.push((ends, agg));
            }
        }
        edge_rows = kept_rows;
    }
    view["edges"] = Value::Array(
        edge_rows
            .into_iter()
            .map(|((from, to), mut agg)| {
                let mut obj = json!({
                    "from": from,
                    "to": to,
                    "links": agg.links,
                    "guessed": agg.guessed,
                });
                if !agg.kinds.is_empty() {
                    let mut kinds_sorted: Vec<(String, u64)> = agg.kinds.into_iter().collect();
                    kinds_sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                    obj["kinds"] = Value::Array(
                        kinds_sorted
                            .into_iter()
                            .map(|(kind, links)| json!({"kind": kind, "links": links}))
                            .collect(),
                    );
                }
                if agg.against {
                    obj["against"] = json!(true);
                }
                if agg.added {
                    obj["added"] = json!(true);
                }
                if agg.detour {
                    obj["detour"] = json!(true);
                }
                if agg.kept_violation {
                    obj["kept"] = json!("violation");
                }
                if agg.arch_violation > 0 {
                    obj["architecture_violation"] = json!(agg.arch_violation);
                }
                if !agg.rule_ids.is_empty() {
                    let ids: Vec<String> = agg.rule_ids.into_iter().collect();
                    obj["rule_ids"] = json!(ids);
                }
                if !agg.arch_rules.is_empty() {
                    agg.arch_rules.sort_by(|a, b| {
                        a.get("kind")
                            .and_then(Value::as_str)
                            .cmp(&b.get("kind").and_then(Value::as_str))
                            .then_with(|| {
                                a.get("rule_id")
                                    .and_then(Value::as_str)
                                    .cmp(&b.get("rule_id").and_then(Value::as_str))
                            })
                            .then_with(|| {
                                a.get("relation")
                                    .and_then(Value::as_str)
                                    .cmp(&b.get("relation").and_then(Value::as_str))
                            })
                            .then_with(|| {
                                a.get("target")
                                    .and_then(Value::as_str)
                                    .cmp(&b.get("target").and_then(Value::as_str))
                            })
                    });
                    obj["architecture_rules"] = Value::Array(agg.arch_rules);
                }
                obj
            })
            .collect(),
    );
    if let Some(obj) = view.as_object_mut() {
        obj.remove("paging");
    }
    true
}

impl Index {
    /// Builds a default-mode view directly at `lod` without yet applying `min_weight` filtering
    /// or `offset`/`limit` pagination, so downstream `mark_*` passes annotate the un-truncated view.
    pub(crate) fn view_at_lod_unfiltered(
        &self,
        project: &str,
        lod: Lod,
        opts: &LodFilter<'_>,
    ) -> Value {
        let scope_norm = opts.scope.trim_matches('/');
        let scope_val = if self.folder(scope_norm) {
            scope_norm
        } else {
            ""
        };
        let mut out = json!({
            "scope": scope_val,
            "crumbs": Index::crumbs(project, scope_val),
            "certain": opts.certain,
            "totals": self.totals(),
            "beyond": false,
            "nodes": [],
            "frames": [],
            "backbone": false,
            "focus": null,
        });
        if let Some(ref backend) = self.backend {
            out["backend"] = json!(backend);
        }
        self.apply_lod_shape_only(&mut out, Some(lod), opts);
        out
    }

    /// Applies progressive `lod` bucketing/expansion without `min_weight` filtering or pagination.
    pub(crate) fn apply_lod_shape_only(
        &self,
        view: &mut Value,
        lod: Option<Lod>,
        opts: &LodFilter<'_>,
    ) -> bool {
        let Some(level) = lod else {
            return false;
        };
        let mode = view.get("mode").and_then(Value::as_str).map(str::to_owned);
        let reshaped = if mode.is_none() {
            rebuild_default_map_at_lod(self, view, level, opts)
        } else if mode.as_deref() != Some("file") {
            rollup_view_to_lod(self, view, level)
        } else {
            false
        };
        if let Some(obj) = view.as_object_mut() {
            obj.insert("lod".into(), json!(level.as_str()));
        }
        reshaped
    }

    /// Applies minimum edge weight (`min_weight`) filtering and pagination onto an annotated view.
    pub(crate) fn apply_min_weight_and_pagination(
        view: &mut Value,
        min_weight: Option<u64>,
        focus: Option<&str>,
        offset: usize,
        limit: Option<usize>,
        reshaped: bool,
    ) {
        if let Some(threshold) = min_weight {
            let keep_edge = |edge: &Value| {
                edge.get("against").and_then(Value::as_bool) == Some(true)
                    || edge.get("kept").and_then(Value::as_str) == Some("violation")
                    || edge
                        .get("links")
                        .or_else(|| edge.get("weight"))
                        .and_then(Value::as_u64)
                        .unwrap_or(1)
                        >= threshold
            };
            if let Some(edges) = view.get_mut("edges").and_then(Value::as_array_mut) {
                edges.retain(keep_edge);
                let new_len = edges.len() as u64;
                if view.get("between").is_some()
                    && view.get("backbone").and_then(Value::as_bool) != Some(true)
                {
                    view["between"] = json!(new_len);
                }
            }
            if let Some(cross) = view
                .get_mut("paging")
                .and_then(|p| p.get_mut("cross_page_edges"))
                .and_then(Value::as_array_mut)
            {
                cross.retain(keep_edge);
            }
            if let Some(obj) = view.as_object_mut() {
                obj.insert("min_weight".into(), json!(threshold));
            }
        }
        if (reshaped || offset > 0 || limit.is_some())
            && let Value::Object(obj) = view
            && (reshaped || !obj.get("paging").is_some_and(Value::is_object))
        {
            let mut anchors = Vec::new();
            if let Some(f) = focus.filter(|s| !s.is_empty()) {
                anchors.push(f);
            }
            paginate_lens_view(obj, offset, limit, &anchors);
        }
    }
}

/// Builds a default-mode view directly at `lod` (avoiding building a discarded folder view first).
#[allow(clippy::too_many_arguments)]
pub(crate) fn view_at_lod(
    index: &Index,
    project: &str,
    lod: Lod,
    min_weight: Option<u64>,
    opts: &LodFilter<'_>,
) -> Value {
    let mut out = index.view_at_lod_unfiltered(project, lod, opts);
    Index::apply_min_weight_and_pagination(
        &mut out,
        min_weight,
        opts.focus,
        opts.offset,
        opts.limit,
        true,
    );
    out
}

/// Applies optional progressive level-of-detail (`lod`) re-bucketing/expansion and minimum edge
/// weight (`min_weight`) filtering on a `graph.map` response. Returns `true` when `nodes` were
/// reshaped and need paging re-applied.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_lod_and_min_weight(
    index: &Index,
    view: &mut Value,
    lod: Option<Lod>,
    min_weight: Option<u64>,
    scope: &str,
    focus: Option<&str>,
    filter: &str,
    certain: bool,
    offset: usize,
    limit: Option<usize>,
) -> bool {
    let opts = LodFilter {
        scope,
        focus,
        filter,
        certain,
        offset,
        limit,
    };
    let reshaped = index.apply_lod_shape_only(view, lod, &opts);
    Index::apply_min_weight_and_pagination(view, min_weight, focus, offset, limit, reshaped);
    reshaped
}

/// Computes a compact structural diff overlay (`edges_added`, `edges_removed`,
/// `sccs_introduced`, `sccs_resolved`, and declared `.harness-architecture.json` `rule_violations`)
/// across two `Index` snapshots or over `changed` files when only the head snapshot is available.
pub(crate) fn structural_diff_overlay(
    head: &Index,
    base: Option<&Index>,
    changed: &[String],
    renamed: &HashMap<&str, &str>,
    certain: bool,
    rules: Option<&layer_rules::Rules>,
) -> Value {
    let head_pairs: BTreeMap<(&str, &str), u64> = head
        .pairs
        .iter()
        .filter_map(|(from, to, pair)| {
            let w = pair.weight(certain);
            (w > 0).then_some((
                (
                    head.files[*from as usize].as_str(),
                    head.files[*to as usize].as_str(),
                ),
                w,
            ))
        })
        .collect();
    let changed_set: HashSet<&str> = changed.iter().map(String::as_str).collect();
    let cap = |mut rows: Vec<(String, String, u64)>, limit: usize| -> Vec<Value> {
        rows.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
        rows.into_iter()
            .take(limit)
            .map(|(from, to, links)| json!({"from": from, "to": to, "links": links}))
            .collect()
    };

    let head_sccs = head.scc_groups(certain);
    let mut overlay = json!({
        "base_available": base.is_some(),
        "head_scc_count": head_sccs.len(),
    });

    if let Some(base_idx) = base {
        let base_pairs: BTreeMap<(&str, &str), u64> = base_idx
            .pairs
            .iter()
            .filter_map(|(from, to, pair)| {
                let w = pair.weight(certain);
                (w > 0).then_some((
                    (
                        base_idx.files[*from as usize].as_str(),
                        base_idx.files[*to as usize].as_str(),
                    ),
                    w,
                ))
            })
            .collect();
        let mut added = Vec::new();
        for (&(u, v), &w) in &head_pairs {
            let bu = renamed.get(u).copied().unwrap_or(u);
            let bv = renamed.get(v).copied().unwrap_or(v);
            if !base_pairs.contains_key(&(bu, bv)) {
                added.push((u.to_owned(), v.to_owned(), w));
            }
        }
        let reverse_rename: HashMap<&str, &str> = renamed
            .iter()
            .map(|(&new_p, &old_p)| (old_p, new_p))
            .collect();
        let mut removed = Vec::new();
        for (&(u, v), &w) in &base_pairs {
            let hu = reverse_rename.get(u).copied().unwrap_or(u);
            let hv = reverse_rename.get(v).copied().unwrap_or(v);
            if !head_pairs.contains_key(&(hu, hv)) {
                removed.push((u.to_owned(), v.to_owned(), w));
            }
        }
        let added_count = added.len();
        let removed_count = removed.len();
        overlay["added_edges"] = json!({"count": added_count, "rows": cap(added, 32)});
        overlay["removed_edges"] = json!({"count": removed_count, "rows": cap(removed, 32)});

        let base_sccs = base_idx.scc_groups(certain);
        let normalize_group = |g: &[String], use_rename: bool| -> Vec<String> {
            let mut out: Vec<String> = g
                .iter()
                .map(|p| {
                    if use_rename {
                        renamed
                            .get(p.as_str())
                            .copied()
                            .unwrap_or(p.as_str())
                            .to_owned()
                    } else {
                        p.clone()
                    }
                })
                .collect();
            out.sort();
            out
        };
        let base_scc_set: BTreeSet<Vec<String>> = base_sccs
            .iter()
            .map(|g| normalize_group(g, false))
            .collect();
        let head_scc_norm_set: BTreeSet<Vec<String>> =
            head_sccs.iter().map(|g| normalize_group(g, true)).collect();
        let mut sccs_introduced: Vec<&Vec<String>> = head_sccs
            .iter()
            .filter(|g| !base_scc_set.contains(&normalize_group(g, true)))
            .collect();
        sccs_introduced.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
        let mut sccs_resolved: Vec<&Vec<String>> = base_sccs
            .iter()
            .filter(|g| !head_scc_norm_set.contains(&normalize_group(g, false)))
            .collect();
        sccs_resolved.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
        overlay["base_scc_count"] = json!(base_sccs.len());
        overlay["sccs_introduced"] = Value::Array(
            sccs_introduced
                .into_iter()
                .take(16)
                .map(
                    |g| json!({"files": g.len(), "members": g.iter().take(12).collect::<Vec<_>>()}),
                )
                .collect(),
        );
        overlay["sccs_resolved"] = Value::Array(
            sccs_resolved
                .into_iter()
                .take(16)
                .map(
                    |g| json!({"files": g.len(), "members": g.iter().take(12).collect::<Vec<_>>()}),
                )
                .collect(),
        );
    } else {
        let touching: Vec<(String, String, u64)> = head_pairs
            .iter()
            .filter(|((u, v), _)| changed_set.contains(*u) || changed_set.contains(*v))
            .map(|((u, v), w)| ((*u).to_owned(), (*v).to_owned(), *w))
            .collect();
        let count = touching.len();
        overlay["changed_edges"] = json!({"count": count, "rows": cap(touching, 32)});
    }

    if let Some(rules) = rules {
        let head_viol = head.declared_rule_violations(certain, rules);
        let head_set: BTreeSet<(String, String, String)> = head_viol
            .iter()
            .map(|(f, t, _, r)| {
                (
                    renamed
                        .get(f.as_str())
                        .copied()
                        .unwrap_or(f.as_str())
                        .to_owned(),
                    renamed
                        .get(t.as_str())
                        .copied()
                        .unwrap_or(t.as_str())
                        .to_owned(),
                    r.clone(),
                )
            })
            .collect();
        let row_json = |v: &[(String, String, u64, String)]| -> Vec<Value> {
            let mut sorted = v.to_vec();
            sorted.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
            sorted
                .into_iter()
                .take(32)
                .map(|(from, to, links, rule)| {
                    json!({"from": from, "to": to, "links": links, "rule": rule})
                })
                .collect()
        };
        if let Some(base_idx) = base {
            let base_viol = base_idx.declared_rule_violations(certain, rules);
            let base_set: BTreeSet<(String, String, String)> = base_viol
                .iter()
                .map(|(f, t, _, r)| (f.clone(), t.clone(), r.clone()))
                .collect();
            let introduced: Vec<_> = head_viol
                .iter()
                .filter(|(f, t, _, r)| {
                    !base_set.contains(&(
                        renamed
                            .get(f.as_str())
                            .copied()
                            .unwrap_or(f.as_str())
                            .to_owned(),
                        renamed
                            .get(t.as_str())
                            .copied()
                            .unwrap_or(t.as_str())
                            .to_owned(),
                        r.clone(),
                    ))
                })
                .cloned()
                .collect();
            let resolved_viol: Vec<_> = base_viol
                .iter()
                .filter(|(f, t, _, r)| !head_set.contains(&(f.clone(), t.clone(), r.clone())))
                .cloned()
                .collect();
            overlay["rule_violations"] = json!({
                "head_count": head_viol.len(),
                "base_count": base_viol.len(),
                "introduced": {"count": introduced.len(), "rows": row_json(&introduced)},
                "resolved": {"count": resolved_viol.len(), "rows": row_json(&resolved_viol)},
            });
        } else {
            overlay["rule_violations"] = json!({
                "head_count": head_viol.len(),
                "rows": row_json(&head_viol),
            });
        }
    }

    overlay
}

/// Computes `truncated`, `limited`, `shown`, `drawn`, `cache_hit`, and `elapsed_ms`
/// and attaches them as non-breaking `"telemetry"` metadata on a `graph.map` response.
pub(crate) fn finalize_telemetry(view: &mut Value, elapsed_ms: u64, cache_hit: bool) {
    let u64_at = |obj: &Value, key: &str| obj.get(key).and_then(Value::as_u64).unwrap_or(0);
    let bool_at = |obj: &Value, key: &str| obj.get(key).and_then(Value::as_bool);

    let shown = view
        .get("nodes")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let drawn = view
        .get("edges")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);

    let mut truncated = u64_at(view, "between") > drawn as u64;
    let mode = view.get("mode").and_then(Value::as_str);
    match mode {
        None => {
            if u64_at(view, "matching") > shown as u64 {
                truncated = true;
            }
            if let Some(frames) = view.get("frames").and_then(Value::as_array)
                && frames
                    .iter()
                    .any(|f| u64_at(f, "places") > u64_at(f, "drawn"))
            {
                truncated = true;
            }
        }
        Some("file") => {
            let around_shown = view.get("around").map_or(0, |a| u64_at(a, "shown"));
            let around_files = view.get("around").map_or(0, |a| u64_at(a, "files"));
            let file_shown = (shown as u64).saturating_sub(around_shown);
            if u64_at(view, "matching") > file_shown || around_files > around_shown {
                truncated = true;
            }
        }
        Some("changes") => {
            if view.get("change").is_some_and(|c| u64_at(c, "hidden") > 0) {
                truncated = true;
            }
            let non_changed_shown = view
                .get("nodes")
                .and_then(Value::as_array)
                .map_or(0, |nodes| {
                    nodes.iter().filter(|n| u64_at(n, "changed") == 0).count() as u64
                });
            if u64_at(view, "matching") > non_changed_shown {
                truncated = true;
            }
        }
        Some("follow") => {
            if let Some(followed) = view.get("followed")
                && (bool_at(followed, "full") == Some(false) || u64_at(followed, "aside") > 0)
            {
                truncated = true;
            }
        }
        Some("path") => {
            if let Some(path) = view.get("path")
                && (u64_at(path, "shown") < u64_at(path, "places")
                    || u64_at(path, "aside") > 0
                    || u64_at(path, "detours_more") > 0)
            {
                truncated = true;
            }
        }
        Some("shared") => {
            if let Some(shared) = view.get("shared")
                && u64_at(shared, "shown") < u64_at(shared, "matching")
            {
                truncated = true;
            }
        }
        Some("cycle") => {
            if let Some(cycle) = view.get("cycle")
                && cycle.get("places").is_some_and(Value::is_array)
                && shown == 0
            {
                truncated = true;
            }
        }
        Some("link") => {
            if u64_at(view, "links") > drawn as u64 {
                truncated = true;
            }
            for dir_key in ["users", "used"] {
                if let Some(dir) = view.get(dir_key) {
                    let total_names = u64_at(dir, "names");
                    let shown_names: u64 =
                        dir.get("groups")
                            .and_then(Value::as_array)
                            .map_or(0, |groups| {
                                groups
                                    .iter()
                                    .map(|g| {
                                        g.get("names").and_then(Value::as_array).map_or(0, Vec::len)
                                            as u64
                                    })
                                    .sum()
                            });
                    if total_names > shown_names {
                        truncated = true;
                    }
                }
            }
        }
        Some("modules") => {
            if u64_at(view, "matching") > shown as u64 {
                truncated = true;
            }
        }
        Some("scenario") => {
            if let Some(scenario) = view.get("scenario")
                && u64_at(scenario, "reachable") + 1 > shown as u64
            {
                truncated = true;
            }
        }
        Some("pattern") => {
            if let Some(parts) = view
                .get("pattern")
                .and_then(|p| p.get("parts"))
                .and_then(Value::as_array)
                && parts.iter().any(|p| u64_at(p, "found") > drawn as u64)
            {
                truncated = true;
            }
            if let Some(frames) = view.get("frames").and_then(Value::as_array)
                && frames
                    .iter()
                    .any(|f| u64_at(f, "places") > u64_at(f, "drawn"))
            {
                truncated = true;
            }
        }
        Some(_) => {}
    }

    let limited = truncated
        || bool_at(view, "backbone") == Some(true)
        || view.get("run").and_then(|r| bool_at(r, "limited")) == Some(true)
        || view.get("access").is_some_and(|a| u64_at(a, "left") > 0)
        || view
            .get("architecture")
            .and_then(|a| a.get("violations"))
            .is_some_and(|v| u64_at(v, "more") > 0)
        || view
            .get("pattern")
            .and_then(|p| p.get("against"))
            .is_some_and(|a| u64_at(a, "more") > 0)
        || view.get("git_history").and_then(|h| bool_at(h, "skipped")) == Some(true)
        || view
            .get("focus")
            .is_some_and(|f| u64_at(f, "uses_more") > 0 || u64_at(f, "used_by_more") > 0);

    if let Value::Object(map) = view {
        map.insert(
            "telemetry".into(),
            json!({
                "elapsed_ms": elapsed_ms,
                "cache_hit": cache_hit,
                "shown": shown,
                "drawn": drawn,
                "limited": limited,
                "truncated": truncated,
            }),
        );
    }
}
