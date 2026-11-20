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
        "composer.json",
        "go.mod",
        "pyproject.toml",
    ];
    let mut dirs = BTreeSet::new();
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
            let abs = root.join(dir);
            MANIFESTS.iter().any(|m| abs.join(m).is_file())
        })
        .collect();
    // Match deepest crate/package directory first.
    roots.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
    roots
}

pub(super) fn crate_bucket(manifest_dirs: &[String], path: &str) -> String {
    if let Some(dir) = manifest_dirs.iter().find(|dir| {
        path.strip_prefix(dir.as_str())
            .is_some_and(|rest| rest.starts_with('/'))
    }) {
        return dir.clone();
    }
    match path.split_once('/') {
        Some((top, _)) if !top.is_empty() => top.to_owned(),
        _ => ".".to_owned(),
    }
}

pub(super) fn folder_bucket(path: &str) -> String {
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

pub(super) struct LodFilter<'a> {
    pub(super) scope: &'a str,
    pub(super) focus: Option<&'a str>,
    pub(super) filter: &'a str,
    pub(super) certain: bool,
    pub(super) paginated: bool,
}

pub(super) fn rebuild_default_map_at_lod(
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
        paginated,
    } = *opts;
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
        let mut candidates: Vec<(u32, String, u64, u64)> = Vec::new();
        for (idx, sym) in index.symbols.iter().enumerate() {
            if matches!(sym.kind, Kind::File | Kind::Note) {
                continue;
            }
            let file_path = index.files[sym.file as usize].as_str();
            if !path_matches_scope(scope, file_path) {
                continue;
            }
            if let Some(f) = focus.filter(|s| !s.is_empty())
                && !path_matches_scope(f, file_path)
            {
                let keys = file_keys_cache
                    .entry(sym.file)
                    .or_insert_with(|| index.keys(sym.file));
                let k = &keys[idx - index.first[sym.file as usize] as usize];
                if k != f {
                    continue;
                }
            }
            let keys = file_keys_cache
                .entry(sym.file)
                .or_insert_with(|| index.keys(sym.file));
            let key = keys[idx - index.first[sym.file as usize] as usize].clone();
            if !needle.is_empty() && !key.to_ascii_lowercase().contains(&needle) {
                continue;
            }
            candidates.push((idx as u32, key, sym_out[idx], sym_in[idx]));
        }
        let total_matching = candidates.len();
        candidates.sort_by(|a, b| (b.2 + b.3).cmp(&(a.2 + a.3)).then(a.1.cmp(&b.1)));
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
        if let Some(obj) = view.as_object_mut() {
            obj.remove("paging");
        }
        return true;
    }

    if !matches!(lod, Lod::Crate | Lod::File) {
        return false;
    }
    let manifest_dirs = if lod == Lod::Crate {
        discover_manifest_dirs(&index.root, &index.files)
    } else {
        Vec::new()
    };
    let bucket_for_file = |path: &str| -> String {
        if lod == Lod::Crate {
            crate_bucket(&manifest_dirs, path)
        } else {
            path.to_owned()
        }
    };

    #[derive(Default)]
    struct BucketTally {
        files: u64,
        symbols: u64,
        within: u64,
        out: u64,
        into: u64,
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
        file_to_bucket.insert(id as u32, b);
    }
    let mut flows: BTreeMap<(String, String), EdgeFlow> = BTreeMap::new();
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
    if !paginated {
        rows.truncate(max_default);
    }
    let kept: HashSet<String> = rows.iter().map(|(k, _)| k.clone()).collect();
    let mut edges: Vec<((String, String), EdgeFlow)> = flows
        .into_iter()
        .filter(|((s, t), _)| kept.contains(s) && kept.contains(t))
        .collect();
    edges.sort_by(|a, b| b.1.flow.links.cmp(&a.1.flow.links).then(a.0.cmp(&b.0)));
    let between = edges.len();
    if !paginated {
        edges.truncate(NORMAL_LIMITS.drawn);
    }
    view["children"] = json!(total);
    view["matching"] = json!(matching);
    view["between"] = json!(between);
    view["nodes"] = Value::Array(
        rows.into_iter()
            .map(|(k, t)| {
                let is_folder = lod == Lod::Crate && k != ".";
                json!({
                    "path": k,
                    "name": if k == "." { ".".to_owned() } else { name(&k).to_owned() },
                    "folder": is_folder,
                    "files": t.files,
                    "symbols": t.symbols,
                    "within": t.within,
                    "out": t.out,
                    "in": t.into,
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
    let manifest_dirs = if lod == Lod::Crate {
        discover_manifest_dirs(&index.root, &index.files)
    } else {
        Vec::new()
    };

    #[derive(Default)]
    struct Bucket {
        files: u64,
        symbols: u64,
        within: u64,
        out: u64,
        into: u64,
        folder: bool,
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
        let file_part = raw_path.split('#').next().unwrap_or(raw_path);
        let b = node_bucket(&manifest_dirs, lod, file_part);
        path_to_bucket.insert(raw_path.to_owned(), b.clone());
        let entry = buckets.entry(b).or_default();
        entry.files += node.get("files").and_then(Value::as_u64).unwrap_or(1);
        entry.symbols += node.get("symbols").and_then(Value::as_u64).unwrap_or(0);
        entry.within += node.get("within").and_then(Value::as_u64).unwrap_or(0);
        entry.folder |= lod == Lod::Crate
            || node.get("folder").and_then(Value::as_bool).unwrap_or(false)
            || file_part.contains('/');
    }

    let mut aggregated: BTreeMap<(String, String), (u64, u64)> = BTreeMap::new();
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
                .unwrap_or_else(|| node_bucket(&manifest_dirs, lod, f));
            let tb = path_to_bucket
                .get(t)
                .cloned()
                .unwrap_or_else(|| node_bucket(&manifest_dirs, lod, t));
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
                entry.0 += links;
                entry.1 += guessed;
            }
        }
    }

    view["nodes"] = Value::Array(
        buckets
            .into_iter()
            .map(|(k, b)| {
                json!({
                    "path": k,
                    "name": if k == "." { ".".to_owned() } else { name(&k).to_owned() },
                    "folder": b.folder,
                    "files": b.files,
                    "symbols": b.symbols,
                    "within": b.within,
                    "out": b.out,
                    "in": b.into,
                })
            })
            .collect(),
    );
    let mut edge_rows: Vec<_> = aggregated.into_iter().collect();
    edge_rows.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(&b.0)));
    view["edges"] = Value::Array(
        edge_rows
            .into_iter()
            .map(|((from, to), (links, guessed))| {
                json!({
                    "from": from,
                    "to": to,
                    "links": links,
                    "guessed": guessed,
                })
            })
            .collect(),
    );
    if let Some(obj) = view.as_object_mut() {
        obj.remove("paging");
    }
    true
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
    paginated: bool,
) -> bool {
    let mut reshaped = false;
    if let Some(level) = lod {
        let mode = view.get("mode").and_then(Value::as_str).map(str::to_owned);
        reshaped = if mode.is_none() {
            rebuild_default_map_at_lod(
                index,
                view,
                level,
                &LodFilter {
                    scope,
                    focus,
                    filter,
                    certain,
                    paginated,
                },
            )
        } else if mode.as_deref() != Some("file") {
            rollup_view_to_lod(index, view, level)
        } else {
            false
        };
        if let Some(obj) = view.as_object_mut() {
            obj.insert("lod".into(), json!(level.as_str()));
        }
    }
    if let Some(threshold) = min_weight {
        if let Some(edges) = view.get_mut("edges").and_then(Value::as_array_mut) {
            edges.retain(|edge| {
                edge.get("links")
                    .or_else(|| edge.get("weight"))
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    >= threshold
            });
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
            cross.retain(|edge| {
                edge.get("links")
                    .or_else(|| edge.get("weight"))
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    >= threshold
            });
        }
        if let Some(obj) = view.as_object_mut() {
            obj.insert("min_weight".into(), json!(threshold));
        }
    }
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
