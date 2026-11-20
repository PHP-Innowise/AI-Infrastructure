use super::*;

/// Identifies which embedded System code-graph surface is targeted:
/// - `Member`: `#system-member-graph-*` in `#systems-view`
/// - `Change`: `#system-change-graph-*` in `#system-changes-view`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum SystemEmbeddedKind {
    Member,
    Change,
}
pub(super) const BOX_WIDTH: f64 = 164.0;
pub(super) const BOX_HEIGHT: f64 = 30.0;
/// A link that skips a layer passes it in a thin slot of its own.
pub(super) const BEND_HEIGHT: f64 = 2.0;
pub(super) const BEND_GAP: f64 = 5.0;
pub(super) const COLUMN_GAP: f64 = 72.0;
pub(super) const ROW_GAP: f64 = 10.0;
pub(super) const MARGIN: f64 = 12.0;
pub(super) const CAPTION: f64 = 20.0;
/// The room at the top of a frame for the name of its folder, and the room
/// between a frame and what it holds.
pub(super) const FRAME_HEAD: f64 = 22.0;
pub(super) const FRAME_PAD: f64 = 8.0;
/// In the window of one link: how wide a name is, how far its two ends
/// stand apart, and the room between two names and between two files.
pub(super) const LINK_BOX: f64 = 220.0;
pub(super) const LINK_GAP: f64 = 220.0;
pub(super) const LINK_ROW: f64 = 6.0;
pub(super) const LINK_BETWEEN: f64 = 10.0;
/// How small a picture is made to fit its room. Smaller than this its boxes
/// cannot be told apart, and the rest of it is scrolled to.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) const SMALLEST: f64 = 0.25;
pub(super) const CAMERA_MIN: f64 = graph_viewport::DEFAULT_MIN_SCALE;
pub(super) const CAMERA_MAX: f64 = 2.4;
pub(super) const FOCUS_CAMERA_MAX: f64 = 1.75;
pub(super) const ZOOM_STEP: f64 = 1.25;
pub(super) const ENTITY_WIDTH: f64 = 156.0;
pub(super) const ENTITY_HEIGHT: f64 = 24.0;
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub x: f64,
    pub y: f64,
    /// `None` for a node no link here touches; those sit in a grid below.
    pub layer: Option<usize>,
    /// Part of a group whose members all reach each other.
    pub cycle: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Wire {
    pub edge: usize,
    pub path: String,
    /// Runs right to left: the weaker direction of a cycle.
    pub back: bool,
    /// Passes so many layers that it is led through none of them: one
    /// curve from end to end, drawn only while it is lit.
    pub portal: bool,
}
/// How far apart two links leave, or reach, one side of a box, and how much
/// of that side all of them take together.
pub(super) const PORT_GAP: f64 = 4.0;
pub(super) const PORT_SPREAD: f64 = 18.0;
/// A link that passes this many layers or more is a portal.
pub const PORTAL_SPAN: usize = 4;
/// How the links of a picture are led between its boxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Routing {
    /// The links of a box leave it, and reach it, apart along its side, in
    /// the order of where they lead, so that they do not cross at once.
    pub ports: bool,
    /// Links that pass layers together, away from one box or towards one,
    /// share one slot in each of them and part at its end.
    pub trunks: bool,
    /// A link that passes at least this many layers takes no slot in them.
    pub portals: Option<usize>,
}
impl Routing {
    /// Every link by itself: from the middle of a box, through a slot of
    /// its own in each layer it passes.
    pub const PLAIN: Self = Self {
        ports: false,
        trunks: false,
        portals: None,
    };
    /// As the pictures lead them.
    pub const CLEAN: Self = Self {
        ports: true,
        trunks: true,
        portals: Some(PORTAL_SPAN),
    };
}
/// The folders spread out in a picture: each is drawn as a frame around the
/// boxes of what it holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frames {
    /// The frame each node lies in; none for what the open folder itself
    /// holds.
    pub of: Vec<Option<usize>>,
    /// The frame that holds each frame. A holder stands before what it holds.
    pub holder: Vec<Option<usize>>,
}
/// Where a frame lies. A frame that holds nothing drawn has no width.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Framed {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub nodes: Vec<Placed>,
    pub wires: Vec<Wire>,
    /// The frames, as they were numbered.
    pub frames: Vec<Framed>,
    pub width: f64,
    pub height: f64,
    /// Where the grid of unlinked nodes begins, when there is one under a map.
    pub apart: Option<f64>,
}
/// A symbol or file created during a live session turn, anchored inside the map.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct CreatedEntity {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub parent: String,
    pub path: String,
    pub folder: String,
    pub line: u64,
    pub change: Option<String>,
    #[serde(default)]
    pub visibility: Option<String>,
    #[serde(default)]
    pub modifiers: Vec<String>,
}
/// Live focus targets & newly created entities overlaid onto the Sessions map.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LiveFocus {
    pub phase: String,
    pub summary: String,
    pub target_folders: Vec<String>,
    pub target_paths: Vec<String>,
    pub target_nodes: Vec<String>,
    pub created_entities: Vec<CreatedEntity>,
}
/// Plane-space position computed for a live created entity overlay badge.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedEntity {
    pub x: f64,
    pub y: f64,
    pub entity: CreatedEntity,
}
/// Resolved graph elements and bounding box for a `LiveFocus` on a laid-out `Plan`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FocusOverlay {
    pub nodes: BTreeSet<usize>,
    pub frames: BTreeSet<usize>,
    pub entities: Vec<PlacedEntity>,
    pub bbox: Option<(f64, f64, f64, f64)>,
    /// How many cards hang under each box that has some.
    pub under: BTreeMap<usize, usize>,
}
/// How much room the cards under a box take: they stand three to a column,
/// so three rows at most.
pub fn entity_room(cards: usize) -> f64 {
    match cards.min(3) {
        0 => 0.0,
        rows => 6.0 + rows as f64 * (ENTITY_HEIGHT + 4.0),
    }
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum EntityAnchor {
    Node(usize),
    Frame(usize),
    Dock,
}
pub(super) fn normalize_rel_path(raw: &str) -> String {
    let mut s = raw.trim().replace('\\', "/");
    while let Some(rest) = s.strip_prefix("./") {
        s = rest.to_owned();
    }
    s.trim_matches('/').to_owned()
}
pub(super) fn strip_symbol_suffix(path: &str) -> &str {
    path.split('#').next().unwrap_or(path)
}
pub(super) fn normalize_entity_kind(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "file" => "file",
        "class" => "class",
        "interface" => "interface",
        "trait" => "trait",
        "enum" => "enum",
        "struct" => "struct",
        "property" | "prop" => "property",
        "field" => "field",
        "const" | "constant" | "class_const" => "const",
        "method" => "method",
        "function" | "fn" => "function",
        _ => "symbol",
    }
    .to_owned()
}
pub(super) fn entity_kind_badge(kind: &str) -> &'static str {
    match kind {
        "file" => "F",
        "class" => "C",
        "interface" => "I",
        "trait" => "T",
        "enum" => "E",
        "struct" => "S",
        "property" | "prop" | "field" => "P",
        "const" | "constant" | "class_const" => "K",
        "method" => "M",
        "function" | "fn" => "fn",
        _ => "+",
    }
}
pub(super) fn string_list(value: &V) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str())
        .map(normalize_rel_path)
        .filter(|s| !s.is_empty())
        .collect()
}
/// Parses a backend or local `GraphFocus` JSON value into `LiveFocus`, or `None` when idle/empty.
pub fn parse_live_focus(value: &V) -> Option<LiveFocus> {
    if !value.is_object() {
        return None;
    }
    let phase = m::text(value, "phase").trim().to_ascii_lowercase();
    let summary = m::text(value, "summary").trim().to_owned();
    let target_folders = string_list(&value["target_folders"]);
    let target_paths = string_list(&value["target_paths"]);
    let target_nodes = string_list(&value["target_nodes"]);
    let created_entities = value["created_entities"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry.is_object() && !m::text(entry, "name").trim().is_empty())
        .map(|entry| {
            let path = normalize_rel_path(&m::text(entry, "path"));
            let parent = normalize_rel_path(&m::text(entry, "parent"));
            let folder = {
                let explicit = normalize_rel_path(&m::text(entry, "folder"));
                if explicit.is_empty() {
                    let base = if !parent.is_empty() { &parent } else { &path };
                    self::parent(strip_symbol_suffix(base)).to_owned()
                } else {
                    explicit
                }
            };
            let visibility = entry["visibility"]
                .as_str()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned);
            let modifiers = entry["modifiers"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            CreatedEntity {
                id: m::text(entry, "id"),
                kind: normalize_entity_kind(&m::text(entry, "kind")),
                name: m::text(entry, "name").trim().to_owned(),
                parent,
                path,
                folder,
                line: entry["line"].as_u64().unwrap_or(0),
                change: entry["change"]
                    .as_str()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_owned),
                visibility,
                modifiers,
            }
        })
        .collect::<Vec<_>>();
    if (phase.is_empty() || phase == "idle")
        && summary.is_empty()
        && target_folders.is_empty()
        && target_paths.is_empty()
        && target_nodes.is_empty()
        && created_entities.is_empty()
    {
        return None;
    }
    Some(LiveFocus {
        phase: if phase.is_empty() {
            "active".to_owned()
        } else {
            phase
        },
        summary,
        target_folders,
        target_paths,
        target_nodes,
        created_entities,
    })
}
pub(super) fn best_ancestor_node(target: &str, nodes: &[V], limit: usize) -> Option<usize> {
    let target_file = normalize_rel_path(strip_symbol_suffix(target));
    if target_file.is_empty() {
        return None;
    }
    let mut best: Option<(usize, usize)> = None;
    for (index, node) in nodes.iter().take(limit).enumerate() {
        let node_path = normalize_rel_path(&m::text(node, "path"));
        if node_path.is_empty() {
            continue;
        }
        let matches = target_file == node_path
            || target_file
                .strip_prefix(&node_path)
                .is_some_and(|rest| rest.starts_with('/'));
        if matches {
            let score = node_path.len() + if m::yes(&node["folder"]) { 1 } else { 2 };
            if best.is_none_or(|(_, prev)| score > prev) {
                best = Some((index, score));
            }
        }
    }
    best.map(|(index, _)| index)
}
pub(super) fn find_entity_anchor(
    entity: &CreatedEntity,
    nodes: &[V],
    frames: &[V],
    plan: &Plan,
    fallback_node: Option<usize>,
) -> EntityAnchor {
    let limit = plan.nodes.len().min(nodes.len());
    let mut candidates = Vec::new();
    for raw in [&entity.parent, &entity.path, &entity.folder] {
        let norm = normalize_rel_path(raw);
        if !norm.is_empty() {
            candidates.push(norm.clone());
            let stripped = normalize_rel_path(strip_symbol_suffix(&norm));
            if stripped != norm && !stripped.is_empty() {
                candidates.push(stripped);
            }
        }
    }
    // First look for an exact node match (file or symbol container).
    for candidate in &candidates {
        if let Some(index) = nodes.iter().take(limit).position(|node| {
            let path = normalize_rel_path(&m::text(node, "path"));
            !path.is_empty() && (path == *candidate || strip_symbol_suffix(&path) == candidate)
        }) {
            return EntityAnchor::Node(index);
        }
    }
    // Fall back to the deepest enclosing folder/file node, then an open frame.
    for candidate in &candidates {
        if let Some(index) = best_ancestor_node(candidate, nodes, limit) {
            return EntityAnchor::Node(index);
        }
    }
    for candidate in &candidates {
        let cand_file = normalize_rel_path(strip_symbol_suffix(candidate));
        if let Some((fi, _)) =
            frames
                .iter()
                .enumerate()
                .take(plan.frames.len())
                .find(|(fi, frame)| {
                    if plan.frames[*fi].width <= 0.0 {
                        return false;
                    }
                    let fpath = normalize_rel_path(&m::text(frame, "path"));
                    !fpath.is_empty()
                        && (cand_file == fpath
                            || cand_file
                                .strip_prefix(&fpath)
                                .is_some_and(|rest| rest.starts_with('/')))
                })
        {
            return EntityAnchor::Frame(fi);
        }
    }
    match fallback_node {
        Some(idx) => EntityAnchor::Node(idx),
        None => EntityAnchor::Dock,
    }
}
pub(super) fn union_bbox(
    bbox: Option<(f64, f64, f64, f64)>,
    rect: (f64, f64, f64, f64),
) -> (f64, f64, f64, f64) {
    match bbox {
        Some((x0, y0, x1, y1)) => (
            x0.min(rect.0),
            y0.min(rect.1),
            x1.max(rect.2),
            y1.max(rect.3),
        ),
        None => rect,
    }
}
/// Maps `LiveFocus` targets & created entities onto drawn `Plan` coordinates.
pub fn resolve_focus(view: &V, plan: &Plan, focus: &LiveFocus) -> FocusOverlay {
    let nodes = m::array(&view["nodes"]);
    let frames = m::array(&view["frames"]);
    let node_limit = plan.nodes.len().min(nodes.len());
    let frame_limit = plan.frames.len().min(frames.len());
    let mut overlay = FocusOverlay::default();

    let mut exact_targets = BTreeSet::new();
    let mut exact_files = BTreeSet::new();
    let mut all_targets = Vec::new();
    for list in [
        &focus.target_nodes,
        &focus.target_paths,
        &focus.target_folders,
    ] {
        for raw in list {
            let norm = normalize_rel_path(raw);
            if !norm.is_empty() {
                exact_targets.insert(norm.clone());
                let file = normalize_rel_path(strip_symbol_suffix(&norm));
                if !file.is_empty() {
                    exact_files.insert(file);
                }
                all_targets.push(norm);
            }
        }
    }
    let mut directly_matched_targets = BTreeSet::new();
    for (index, node) in nodes.iter().take(node_limit).enumerate() {
        let path = normalize_rel_path(&m::text(node, "path"));
        let file = normalize_rel_path(&m::text(node, "file"));
        let path_file = normalize_rel_path(strip_symbol_suffix(&path));
        if (!path.is_empty()
            && (exact_targets.contains(&path)
                || exact_files.contains(&path)
                || exact_targets.contains(&path_file)))
            || (!file.is_empty() && exact_files.contains(&file))
        {
            overlay.nodes.insert(index);
            if !path.is_empty() {
                directly_matched_targets.insert(path);
            }
            if !file.is_empty() {
                directly_matched_targets.insert(file);
            }
        }
    }
    // When a target lives inside a collapsed folder at root overview, highlight that ancestor folder.
    for target in &all_targets {
        let target_file = normalize_rel_path(strip_symbol_suffix(target));
        if directly_matched_targets.contains(target)
            || directly_matched_targets.contains(&target_file)
        {
            continue;
        }
        if let Some(ancestor) = best_ancestor_node(target, &nodes, node_limit) {
            overlay.nodes.insert(ancestor);
        }
    }
    for (fi, frame) in frames.iter().take(frame_limit).enumerate() {
        if plan.frames[fi].width <= 0.0 {
            continue;
        }
        let fpath = normalize_rel_path(&m::text(frame, "path"));
        if fpath.is_empty() {
            continue;
        }
        let frame_matches = all_targets.iter().any(|target| {
            let t_file = normalize_rel_path(strip_symbol_suffix(target));
            t_file == fpath
                || t_file
                    .strip_prefix(&fpath)
                    .is_some_and(|rest| rest.starts_with('/'))
        });
        if frame_matches {
            overlay.frames.insert(fi);
        }
    }
    let fallback_node = overlay.nodes.iter().next().copied();
    let mut anchor_counts: BTreeMap<EntityAnchor, usize> = BTreeMap::new();
    for entity in &focus.created_entities {
        let anchor = find_entity_anchor(entity, &nodes, &frames, plan, fallback_node);
        let slot = *anchor_counts.get(&anchor).unwrap_or(&0);
        anchor_counts.insert(anchor, slot + 1);
        let col = slot / 3;
        let row = slot % 3;
        let (x, y) = match anchor {
            EntityAnchor::Node(idx) => {
                overlay.nodes.insert(idx);
                let placed = &plan.nodes[idx];
                (
                    (placed.x + 8.0 + col as f64 * (ENTITY_WIDTH + 8.0)).max(MARGIN),
                    placed.y + BOX_HEIGHT + 6.0 + row as f64 * (ENTITY_HEIGHT + 4.0),
                )
            }
            EntityAnchor::Frame(fi) => {
                overlay.frames.insert(fi);
                let fr = &plan.frames[fi];
                (
                    fr.x + 12.0 + col as f64 * (ENTITY_WIDTH + 8.0),
                    fr.y + FRAME_HEAD + 8.0 + row as f64 * (ENTITY_HEIGHT + 4.0),
                )
            }
            EntityAnchor::Dock => (
                MARGIN + col as f64 * (ENTITY_WIDTH + 8.0),
                MARGIN + row as f64 * (ENTITY_HEIGHT + 6.0),
            ),
        };
        overlay.entities.push(PlacedEntity {
            x,
            y,
            entity: entity.clone(),
        });
    }
    overlay.under = anchor_counts
        .iter()
        .filter_map(|(anchor, cards)| match anchor {
            EntityAnchor::Node(node) => Some((*node, *cards)),
            _ => None,
        })
        .collect();
    for &idx in &overlay.nodes {
        let p = &plan.nodes[idx];
        overlay.bbox = Some(union_bbox(
            overlay.bbox,
            (p.x, p.y, p.x + BOX_WIDTH, p.y + BOX_HEIGHT),
        ));
    }
    for placed_entity in &overlay.entities {
        overlay.bbox = Some(union_bbox(
            overlay.bbox,
            (
                placed_entity.x,
                placed_entity.y,
                placed_entity.x + ENTITY_WIDTH,
                placed_entity.y + ENTITY_HEIGHT,
            ),
        ));
    }
    // Include targeted enclosing frames (and header padding when zooming onto nodes inside a frame).
    for &fi in &overlay.frames {
        let fr = &plan.frames[fi];
        let fpath = normalize_rel_path(&m::text(&frames[fi], "path"));
        let explicit_folder_only = focus.target_folders.iter().any(|folder| folder == &fpath)
            && !focus
                .target_paths
                .iter()
                .chain(&focus.target_nodes)
                .any(|t| {
                    t.strip_prefix(&fpath)
                        .is_some_and(|rest| rest.starts_with('/'))
                });
        if overlay.bbox.is_none() || explicit_folder_only {
            overlay.bbox = Some(union_bbox(
                overlay.bbox,
                (fr.x, fr.y, fr.x + fr.width, fr.y + fr.height),
            ));
        } else if let Some((x0, y0, x1, y1)) = overlay.bbox {
            // Keep folder header readable above top matched nodes inside the frame.
            if (y0 - fr.y).abs() <= 48.0 {
                overlay.bbox = Some((x0.min(fr.x + 4.0), fr.y, x1, y1));
            }
        }
    }
    overlay
}
/// Computes camera `(scale, pan_x, pan_y)` to fit the entire plane inside `viewport`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn camera_for_fit(plane: (f64, f64), viewport: (f64, f64)) -> (f64, f64, f64) {
    let cam = graph_viewport::fit_proportional(plane, viewport, 0.94, CAMERA_MIN, 1.0, 12.0);
    (cam.scale, cam.tx, cam.ty)
}
/// Computes camera `(scale, pan_x, pan_y)` focusing and centering `bbox` inside `viewport`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn camera_for_bbox(
    bbox: (f64, f64, f64, f64),
    viewport: (f64, f64),
    focus_max: f64,
) -> (f64, f64, f64) {
    let cam = graph_viewport::fit_bbox(
        bbox,
        viewport,
        (112.0, 88.0),
        (BOX_WIDTH + 96.0, BOX_HEIGHT + 80.0),
        0.90,
        CAMERA_MIN,
        focus_max.clamp(CAMERA_MIN, CAMERA_MAX),
    );
    (cam.scale, cam.tx, cam.ty)
}
/// Zooms around a viewport-relative anchor `(ax, ay)` while keeping that logical point fixed.
#[cfg_attr(not(test), allow(dead_code))]
pub fn zoom_at(
    scale0: f64,
    pan_x0: f64,
    pan_y0: f64,
    anchor: (f64, f64),
    next_scale: f64,
) -> (f64, f64, f64) {
    let cam = graph_viewport::zoom_at(
        graph_viewport::Camera::new(pan_x0, pan_y0, scale0),
        anchor,
        next_scale,
        CAMERA_MIN,
        CAMERA_MAX,
    );
    (cam.scale, cam.tx, cam.ty)
}
pub(super) fn prompt_tokens(prompt: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "add", "and", "app", "can", "class", "code", "create", "delete", "edit", "file", "files",
        "find", "fix", "for", "from", "function", "get", "graph", "help", "into", "make", "map",
        "method", "please", "remove", "set", "show", "test", "tests", "that", "the", "this",
        "update", "using", "with", "work",
    ];
    let mut out = Vec::new();
    for raw in
        prompt.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != '/' && c != '.')
    {
        let cleaned = raw.trim_matches(|c: char| c == '.' || c == '/' || c == '-');
        if cleaned.len() < 2 {
            continue;
        }
        let lower = cleaned.to_ascii_lowercase();
        if !STOP.contains(&lower.as_str()) && !out.contains(&lower) {
            out.push(lower);
        }
        if cleaned.contains('/') {
            for seg in cleaned.split('/') {
                let seg_l = seg.to_ascii_lowercase();
                if seg_l.len() >= 3 && !STOP.contains(&seg_l.as_str()) && !out.contains(&seg_l) {
                    out.push(seg_l);
                }
            }
        }
    }
    out
}
/// Fast local prompt-to-graph matcher for immediate 0ms predictive focus when user sends a prompt.
pub fn match_prompt_targets(view: &V, prompt: &str) -> Option<LiveFocus> {
    let tokens = prompt_tokens(prompt);
    if tokens.is_empty() {
        return None;
    }
    let nodes = m::array(&view["nodes"]);
    let frames = m::array(&view["frames"]);
    if nodes.is_empty() && frames.is_empty() {
        return None;
    }
    let mut scored: Vec<(u64, bool, String, String)> = Vec::new();
    for node in &nodes {
        let path = normalize_rel_path(&m::text(node, "path"));
        if path.is_empty() {
            continue;
        }
        let name = m::text(node, "name");
        let name_l = name.to_ascii_lowercase();
        let path_l = path.to_ascii_lowercase();
        let file_l = normalize_rel_path(&m::text(node, "file")).to_ascii_lowercase();
        let path_segs: Vec<&str> = path_l.split(['/', '.', '#', ':']).collect();
        let mut score = 0u64;
        for tok in &tokens {
            if tok == &path_l || tok == &name_l || (!file_l.is_empty() && tok == &file_l) {
                score += 120;
            } else if path_segs.iter().any(|seg| !seg.is_empty() && *seg == tok) {
                score += 80;
            } else if tok.len() >= 3 && (name_l.contains(tok) || path_l.contains(tok)) {
                score += 25 + (tok.len() as u64).min(20);
            }
        }
        if score >= 25 {
            scored.push((score, m::yes(&node["folder"]), path, name));
        }
    }
    for frame in &frames {
        let path = normalize_rel_path(&m::text(frame, "path"));
        if path.is_empty() || path.starts_with("role:") {
            continue;
        }
        let name = m::text(frame, "name");
        let name_l = name.to_ascii_lowercase();
        let path_l = path.to_ascii_lowercase();
        let mut score = 0u64;
        for tok in &tokens {
            if tok == &path_l || tok == &name_l {
                score += 100;
            } else if tok.len() >= 3 && (name_l.contains(tok) || path_l.contains(tok)) {
                score += 25;
            }
        }
        if score >= 25 && !scored.iter().any(|(_, _, p, _)| p == &path) {
            scored.push((score, true, path, name));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.2.cmp(&b.2)));
    let top_score = scored.first()?.0;
    let cutoff = (top_score / 2).max(25);
    let mut target_folders = Vec::new();
    let mut target_paths = Vec::new();
    let mut target_nodes = Vec::new();
    let mut labels = Vec::new();
    for (score, folder, path, name) in scored.into_iter().take(5) {
        if score < cutoff {
            break;
        }
        target_nodes.push(path.clone());
        if folder {
            if !target_folders.contains(&path) {
                target_folders.push(path);
            }
        } else if !target_paths.contains(&path) {
            target_paths.push(path);
        }
        let label = if name.is_empty() {
            "target".to_owned()
        } else {
            name
        };
        if !labels.contains(&label) && labels.len() < 3 {
            labels.push(label);
        }
    }
    if target_nodes.is_empty() {
        return None;
    }
    Some(LiveFocus {
        phase: "predicting".to_owned(),
        summary: format!("Focusing near {}", labels.join(", ")),
        target_folders,
        target_paths,
        target_nodes,
        created_entities: Vec::new(),
    })
}
/// Groups of nodes that all reach each other, as a group number per node.
pub(super) fn groups(count: usize, out: &[Vec<usize>]) -> Vec<usize> {
    struct Walk<'a> {
        out: &'a [Vec<usize>],
        order: Vec<Option<usize>>,
        low: Vec<usize>,
        stack: Vec<usize>,
        stacked: Vec<bool>,
        group: Vec<usize>,
        next: usize,
        found: usize,
    }
    impl Walk<'_> {
        fn visit(&mut self, node: usize) {
            self.order[node] = Some(self.next);
            self.low[node] = self.next;
            self.next += 1;
            self.stack.push(node);
            self.stacked[node] = true;
            for index in 0..self.out[node].len() {
                let other = self.out[node][index];
                match self.order[other] {
                    None => {
                        self.visit(other);
                        self.low[node] = self.low[node].min(self.low[other]);
                    }
                    Some(seen) if self.stacked[other] => {
                        self.low[node] = self.low[node].min(seen);
                    }
                    Some(_) => {}
                }
            }
            if Some(self.low[node]) == self.order[node] {
                while let Some(member) = self.stack.pop() {
                    self.stacked[member] = false;
                    self.group[member] = self.found;
                    if member == node {
                        break;
                    }
                }
                self.found += 1;
            }
        }
    }
    let mut walk = Walk {
        out,
        order: vec![None; count],
        low: vec![0; count],
        stack: vec![],
        stacked: vec![false; count],
        group: vec![0; count],
        next: 0,
        found: 0,
    };
    for node in 0..count {
        if walk.order[node].is_none() {
            walk.visit(node);
        }
    }
    walk.group
}
/// An order of the linked nodes in which as much link weight as possible
/// points forward (the heuristic of Eades, Lin and Smyth). Without cycles
/// every link points forward; in a cycle the weaker direction points back.
pub(super) fn order(
    count: usize,
    linked: &[bool],
    links: &BTreeMap<(usize, usize), u64>,
) -> Vec<usize> {
    let mut open = linked.to_vec();
    let mut out_adj: Vec<Vec<(usize, u64)>> = vec![vec![]; count];
    let mut in_adj: Vec<Vec<(usize, u64)>> = vec![vec![]; count];
    let mut out_w = vec![0u64; count];
    let mut in_w = vec![0u64; count];
    for (&(from, to), &weight) in links {
        if from < count && to < count && open[from] && open[to] {
            out_adj[from].push((to, weight));
            in_adj[to].push((from, weight));
            out_w[from] += weight;
            in_w[to] += weight;
        }
    }
    let close_node = |node: usize, open: &mut [bool], out_w: &mut [u64], in_w: &mut [u64]| {
        if !open[node] {
            return;
        }
        open[node] = false;
        for &(to, w) in &out_adj[node] {
            if open[to] {
                in_w[to] = in_w[to].saturating_sub(w);
            }
        }
        for &(from, w) in &in_adj[node] {
            if open[from] {
                out_w[from] = out_w[from].saturating_sub(w);
            }
        }
    };
    let mut remaining: Vec<usize> = (0..count).filter(|&n| open[n]).collect();
    let (mut first, mut last) = (vec![], vec![]);
    while !remaining.is_empty() {
        let mut moved = false;
        for &node in &remaining {
            if open[node] && out_w[node] == 0 {
                close_node(node, &mut open, &mut out_w, &mut in_w);
                last.push(node);
                moved = true;
            }
        }
        for &node in &remaining {
            if open[node] && in_w[node] == 0 {
                close_node(node, &mut open, &mut out_w, &mut in_w);
                first.push(node);
                moved = true;
            }
        }
        if moved {
            remaining.retain(|&n| open[n]);
            continue;
        }
        let best = remaining.iter().copied().max_by_key(|&node| {
            (
                out_w[node] as i128 - in_w[node] as i128,
                std::cmp::Reverse(node),
            )
        });
        if let Some(best) = best {
            close_node(best, &mut open, &mut out_w, &mut in_w);
            first.push(best);
            remaining.retain(|&n| open[n]);
        } else {
            break;
        }
    }
    last.reverse();
    first.extend(last);
    first
}
/// Which links of a picture say nothing new of what depends on what: a link
/// from one box to another that a longer way through other boxes already
/// joins. Leaving them out keeps every box reaching what it reached.
///
/// Boxes that all reach each other are one place for this: a link inside
/// such a group is never left out, and a way may lead through the group.
/// Of two boxes joined by a link and by no longer way, the link stays.
pub fn transitive(count: usize, links: &[(usize, usize)]) -> Vec<bool> {
    let mut out = vec![vec![]; count];
    for (from, to) in links {
        if from != to && *from < count && *to < count && !out[*from].contains(to) {
            out[*from].push(*to);
        }
    }
    // The groups come out with what is used before what uses it, so every
    // link between two groups leads from a later one to an earlier one.
    let group = groups(count, &out);
    let many = group.iter().copied().max().map_or(0, |last| last + 1);
    let mut next: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); many];
    for (from, to) in links {
        if *from < count && *to < count && group[*from] != group[*to] {
            next[group[*from]].insert(group[*to]);
        }
    }
    // What each group reaches, itself left out, as a set of bits.
    let words = many.div_ceil(64).max(1);
    let mut reach = vec![vec![0u64; words]; many];
    for at in 0..many {
        let mut held = vec![0u64; words];
        for used in &next[at] {
            held[used / 64] |= 1 << (used % 64);
            if *used < at {
                for (word, more) in held.iter_mut().zip(&reach[*used]) {
                    *word |= more;
                }
            }
        }
        reach[at] = held;
    }
    links
        .iter()
        .map(|(from, to)| {
            if *from >= count || *to >= count {
                return false;
            }
            let (user, used) = (group[*from], group[*to]);
            user != used
                && next[user]
                    .iter()
                    .any(|by| *by != used && reach[*by][used / 64] & (1 << (used % 64)) != 0)
        })
        .collect()
}
/// How far each box of a picture lies from one of them along its links:
/// what the box uses within `down` links is so many links after it, what
/// uses it within `up` links so many before it, and the box itself at none.
/// A box that lies both ways, as in a cycle, is told by the nearer way, and
/// by what the box uses where both are as near.
pub fn cone(
    count: usize,
    links: &[(usize, usize)],
    from: usize,
    up: usize,
    down: usize,
) -> Vec<Option<i32>> {
    let mut depth = vec![None; count];
    if from >= count {
        return depth;
    }
    let mut out_adj = vec![vec![]; count];
    let mut in_adj = vec![vec![]; count];
    for &(a, b) in links {
        if a < count && b < count {
            out_adj[a].push(b);
            in_adj[b].push(a);
        }
    }
    let walk = |adj: &[Vec<usize>], limit: usize| {
        let mut found = vec![None; count];
        found[from] = Some(0usize);
        let mut edge = vec![from];
        for step in 1..=limit {
            let mut further = vec![];
            for &near in &edge {
                for &far in &adj[near] {
                    if found[far].is_none() {
                        found[far] = Some(step);
                        further.push(far);
                    }
                }
            }
            if further.is_empty() {
                break;
            }
            edge = further;
        }
        found
    };
    let (after, before) = (walk(&out_adj, down), walk(&in_adj, up));
    for node in 0..count {
        depth[node] = match (after[node], before[node]) {
            (Some(after), Some(before)) if before < after => Some(-(before as i32)),
            (Some(after), _) => Some(after as i32),
            (None, Some(before)) => Some(-(before as i32)),
            (None, None) => None,
        };
    }
    depth
}
/// Whether a link lies in the cone of a box: it leads one step further
/// from the box along the arrows, or one step nearer to it against them.
pub fn in_cone(depth: &[Option<i32>], link: (usize, usize)) -> bool {
    match (
        depth.get(link.0).copied().flatten(),
        depth.get(link.1).copied().flatten(),
    ) {
        (Some(from), Some(to)) => (from >= 0 && to == from + 1) || (to <= 0 && from == to - 1),
        _ => false,
    }
}
pub(super) fn curve(path: &mut String, from: (f64, f64), to: (f64, f64)) {
    let reach = (to.0 - from.0) / 2.0;
    path.push_str(&format!(
        " C{:.1} {:.1} {:.1} {:.1} {:.1} {:.1}",
        from.0 + reach,
        from.1,
        to.0 - reach,
        to.1,
        to.0,
        to.1
    ));
}
/// A picture without frames.
#[cfg(test)]
pub fn plan(count: usize, edges: &[(usize, usize, u64)]) -> Plan {
    plan_with(count, edges, &Frames::default())
}
/// A picture in which every node stands where its links put it.
#[cfg(test)]
pub fn plan_with(count: usize, edges: &[(usize, usize, u64)], frames: &Frames) -> Plan {
    plan_beside(count, edges, frames, None)
}
/// Places `count` nodes and routes `edges` (user, used, weight) between them.
/// Layers run left to right along the links; a link that skips a layer bends
/// through a slot of its own there, so it does not cross a box. A cycle cannot
/// be laid out that way, so its weakest links run back, right to left.
///
/// Some nodes may lie in frames. The layers run through the whole picture,
/// so what a frame holds stands in the columns of everything else and still
/// shows which way it depends. A frame is a band across those columns: the
/// frames lie one under another above what is in no frame, a frame inside
/// its holder, and no other box stands in a band. A link between two frames
/// passes outside both.
///
/// With `pair`, two nodes stand side by side whatever their links say: in
/// the first layer, or, with the flag, in the last. They are the two places
/// of a picture whose links all lead on from them, or all lead to them; one
/// that a link drawn here leads to, or leads on from, is left where its
/// links put it.
pub fn plan_beside(
    count: usize,
    edges: &[(usize, usize, u64)],
    frames: &Frames,
    pair: Option<([usize; 2], bool)>,
) -> Plan {
    plan_routed(count, edges, frames, pair, Routing::PLAIN)
}
/// The same picture with its links led as `routing` says. The boxes of a
/// layer stand as their links put them either way; what changes is how
/// much room the links that pass a layer take there, and so how tall the
/// picture is.
///
/// With `ports` the links of a box leave its side one under another, in the
/// order of where they lead next, and reach a box the same way. With
/// `trunks` the links that leave one box and pass layers share one slot in
/// each layer they pass together; so do the links that lead to one box. A
/// link joins the larger of the two bundles it could. With `portals` a link
/// that passes that many layers takes no slot at all.
pub fn plan_routed(
    count: usize,
    edges: &[(usize, usize, u64)],
    frames: &Frames,
    pair: Option<([usize; 2], bool)>,
    routing: Routing,
) -> Plan {
    plan_roomy(count, edges, frames, pair, routing, &[])
}
/// The same picture with room kept under some of its boxes: `room` says for
/// each box how much. What hangs under a box, such as the cards of what a
/// session made in a file, then lies on nothing else. A box no link touches
/// stands in a grid, whose rows all stand as far apart as the most room
/// asked for among such boxes.
pub fn plan_roomy(
    count: usize,
    edges: &[(usize, usize, u64)],
    frames: &Frames,
    pair: Option<([usize; 2], bool)>,
    routing: Routing,
    room: &[f64],
) -> Plan {
    let under = |node: usize| {
        room.get(node)
            .copied()
            .filter(|room| room.is_finite())
            .unwrap_or(0.0)
            .max(0.0)
    };
    let mut links: BTreeMap<(usize, usize), u64> = BTreeMap::new();
    let mut out = vec![vec![]; count];
    let mut linked = vec![false; count];
    for (from, to, weight) in edges {
        if from != to && *from < count && *to < count {
            let known = links.entry((*from, *to)).or_insert(0);
            if *known == 0 {
                out[*from].push(*to);
            }
            *known += (*weight).max(1);
            linked[*from] = true;
            linked[*to] = true;
        }
    }
    let group = groups(count, &out);
    let mut size = vec![0usize; count];
    for node in 0..count {
        size[group[node]] += 1;
    }
    let order = order(count, &linked, &links);
    let mut rank = vec![0usize; count];
    for (index, node) in order.iter().enumerate() {
        rank[*node] = index;
    }
    // For layout every link points forward in that order; one that pointed
    // back is drawn the other way round afterwards.
    let wired = links
        .keys()
        .map(|(from, to)| {
            let back = rank[*from] > rank[*to];
            let (near, far) = if back { (*to, *from) } else { (*from, *to) };
            (near, far, back, (*from, *to))
        })
        .collect::<Vec<_>>();
    let mut wired_out = vec![vec![]; count];
    let mut has_wired_in = vec![false; count];
    for &(near, far, ..) in &wired {
        if near < count && far < count {
            wired_out[near].push(far);
            has_wired_in[far] = true;
        }
    }
    let mut layer = vec![0usize; count];
    for &node in &order {
        let next_layer = layer[node] + 1;
        for &far in &wired_out[node] {
            layer[far] = layer[far].max(next_layer);
        }
    }
    // Longest paths put everything nobody uses in the first layer, far from
    // what it uses. Each node moves right, up to the layer before the nearest
    // thing it uses, so its links stay short and cross fewer layers.
    for &node in order.iter().rev() {
        let nearest = wired_out[node].iter().map(|&far| layer[far]).min();
        if let Some(nearest) = nearest {
            layer[node] = layer[node].max(nearest - 1);
        }
    }
    // Two places shown side by side stand in one layer: the first where
    // their links lead on from them, the last where they lead to them.
    if let Some((pair, last)) = pair {
        let end = if last {
            order.iter().map(|node| layer[*node]).max().unwrap_or(0)
        } else {
            0
        };
        for node in pair {
            let held = node < count
                && if last {
                    !wired_out[node].is_empty()
                } else {
                    has_wired_in[node]
                };
            if node < count && linked[node] && !held {
                layer[node] = end;
            }
        }
    }
    let layers = order
        .iter()
        .map(|node| layer[*node])
        .max()
        .map_or(0, |last| last + 1);
    // A band is what one frame holds itself; the last band is what lies in
    // no frame. The band over a band is that of the frame's holder.
    let framed = frames.holder.len();
    let free = framed;
    let over = |band: usize| {
        (band != free).then(|| {
            frames.holder[band]
                .filter(|holder| *holder < band)
                .unwrap_or(free)
        })
    };
    // Where two bands meet: the nearest one that holds both.
    let meet = |one: usize, other: usize| {
        let mut held = vec![one];
        while let Some(band) = over(held[held.len() - 1]) {
            held.push(band);
        }
        let mut at = other;
        while !held.contains(&at) {
            at = over(at).unwrap_or(free);
        }
        at
    };
    let mut band = (0..count)
        .map(|node| {
            frames
                .of
                .get(node)
                .copied()
                .flatten()
                .filter(|frame| *frame < framed)
                .unwrap_or(free)
        })
        .collect::<Vec<_>>();
    // The frames from top to bottom: a frame begins, holds its frames, then
    // what it holds itself, and ends.
    #[derive(Clone, Copy)]
    enum Step {
        Begin(usize),
        Own(usize),
        End(usize),
    }
    let mut inner = vec![vec![]; framed];
    let mut todo = vec![];
    for frame in (0..framed).rev() {
        match over(frame) {
            Some(holder) if holder != free => inner[holder].insert(0, frame),
            _ => todo.push(Step::Begin(frame)),
        }
    }
    let mut steps = vec![];
    while let Some(step) = todo.pop() {
        if let Step::Begin(frame) = step {
            todo.push(Step::End(frame));
            todo.push(Step::Own(frame));
            todo.extend(inner[frame].iter().rev().map(|held| Step::Begin(*held)));
        }
        steps.push(step);
    }
    // The bands in that order, what lies in no frame last.
    let mut rank = vec![framed; framed + 1];
    for (at, frame) in steps
        .iter()
        .filter_map(|step| match step {
            Step::Own(frame) => Some(*frame),
            _ => None,
        })
        .enumerate()
    {
        rank[frame] = at;
    }
    // Items of a layer: nodes first, then one bend per link passing through.
    let mut columns: Vec<Vec<usize>> = vec![vec![]; layers];
    for node in (0..count).filter(|node| linked[*node]) {
        columns[layer[node]].push(node);
    }
    let mut chains: Vec<Vec<usize>> = vec![];
    let mut heights = vec![BOX_HEIGHT; count];
    // The layer each item stands in: a box in its own, a slot in the layer
    // its link passes there.
    let mut column_of = layer.clone();
    let (mut above, mut below): (Vec<Vec<usize>>, Vec<Vec<usize>>) =
        (vec![vec![]; count], vec![vec![]; count]);
    // A link that passes many layers is a portal: it takes no slot in them,
    // and is drawn from end to end only while it is lit. A link that runs
    // back closes a cycle and is always drawn.
    let passes = |near: usize, far: usize| layer[far].saturating_sub(layer[near]);
    let portal = wired
        .iter()
        .map(|(near, far, back, _)| {
            !*back
                && routing
                    .portals
                    .is_some_and(|least| passes(*near, *far) >= least)
        })
        .collect::<Vec<_>>();
    // Links that pass layers together share one slot in each: those that
    // leave one box, or those that lead to one.
    let (mut leaving, mut reaching) = (vec![0usize; count], vec![0usize; count]);
    for ((near, far, back, _), portal) in wired.iter().zip(&portal) {
        if routing.trunks && !*back && !*portal && passes(*near, *far) >= 2 {
            leaving[*near] += 1;
            reaching[*far] += 1;
        }
    }
    // The slot of a bundle in a layer and a band: by whether it leaves a
    // box or leads to one, and by that box.
    let mut trunks: BTreeMap<(bool, usize, usize, usize), usize> = BTreeMap::new();
    let mut pair_weights: BTreeMap<(usize, usize), f64> = BTreeMap::new();
    for ((near, far, back, orig_edge), portal) in wired.iter().zip(&portal) {
        let mut chain = vec![*near];
        // A link passes through the band that holds both of its ends.
        let through = meet(band[*near], band[*far]);
        let edge_w = links.get(orig_edge).copied().unwrap_or(1).max(1) as f64;
        // A link joins the larger bundle of the two it could.
        let trunk = if !routing.trunks || *back || *portal {
            None
        } else if leaving[*near] >= 2 && leaving[*near] >= reaching[*far] {
            Some((true, *near))
        } else if reaching[*far] >= 2 {
            Some((false, *far))
        } else {
            None
        };
        if !*portal {
            // The number of a layer names its slots as well as its column.
            #[allow(clippy::needless_range_loop)]
            for at in layer[*near] + 1..layer[*far] {
                let shared =
                    trunk.and_then(|(out, of)| trunks.get(&(out, of, at, through)).copied());
                let bend = match shared {
                    Some(bend) => bend,
                    None => {
                        let bend = heights.len();
                        heights.push(BEND_HEIGHT);
                        band.push(through);
                        column_of.push(at);
                        above.push(vec![]);
                        below.push(vec![]);
                        columns[at].push(bend);
                        if let Some((out, of)) = trunk {
                            trunks.insert((out, of, at, through), bend);
                        }
                        bend
                    }
                };
                chain.push(bend);
            }
        }
        chain.push(*far);
        // A portal pulls no box towards the other: it is not there to be
        // kept short.
        if !*portal {
            for pair in chain.windows(2) {
                *pair_weights.entry((pair[0], pair[1])).or_insert(0.0) += edge_w;
                *pair_weights.entry((pair[1], pair[0])).or_insert(0.0) += edge_w;
                if !below[pair[0]].contains(&pair[1]) {
                    below[pair[0]].push(pair[1]);
                    above[pair[1]].push(pair[0]);
                }
            }
        }
        chains.push(chain);
    }
    // In every layer the bands stand in the same order.
    for column in &mut columns {
        column.sort_by_key(|item| rank[band[*item]]);
    }
    // Each item moves towards the weighted average place of its neighbours in the
    // layer before it, then after it; a few rounds remove most crossings while
    // keeping heavy-weight dependencies aligned. It stays in its band.
    let mut place = vec![0.0; heights.len()];
    let number = |columns: &[Vec<usize>], place: &mut [f64]| {
        for column in columns {
            for (index, item) in column.iter().enumerate() {
                place[*item] = index as f64;
            }
        }
    };
    number(&columns, &mut place);
    for round in 0..8 {
        let forward = round % 2 == 0;
        let turn: Vec<usize> = if forward {
            (1..layers).collect()
        } else {
            (0..layers.saturating_sub(1)).rev().collect()
        };
        for at in turn {
            let near = if forward { &above } else { &below };
            let mut keyed = columns[at]
                .iter()
                .map(|item| {
                    let links_to = &near[*item];
                    let middle = if links_to.is_empty() {
                        place[*item]
                    } else {
                        let mut w_sum = 0.0;
                        let mut pos_sum = 0.0;
                        for other in links_to {
                            let w = pair_weights.get(&(*item, *other)).copied().unwrap_or(1.0);
                            w_sum += w;
                            pos_sum += place[*other] * w;
                        }
                        if w_sum > 0.0 {
                            pos_sum / w_sum
                        } else {
                            place[*item]
                        }
                    };
                    (rank[band[*item]], middle, place[*item], *item)
                })
                .collect::<Vec<_>>();
            keyed.sort_by(|a, b| {
                a.0.cmp(&b.0)
                    .then(a.1.total_cmp(&b.1))
                    .then(a.2.total_cmp(&b.2))
            });
            columns[at] = keyed.into_iter().map(|(.., item)| item).collect();
            number(&columns[at..=at], &mut place);
        }
    }
    // Boxes keep a row gap; the thin slots of passing links pack closer.
    let gap = |upper: usize, lower: usize| {
        if upper < count && lower < count {
            ROW_GAP
        } else {
            BEND_GAP
        }
    };
    // What a band holds in one layer is a run; a band is as tall as its
    // tallest run.
    let mut offset = vec![0.0; heights.len()];
    let mut runs: BTreeMap<(usize, usize), f64> = BTreeMap::new();
    for (at, column) in columns.iter().enumerate() {
        let mut y = 0.0;
        for (index, item) in column.iter().enumerate() {
            if index > 0 {
                let upper = column[index - 1];
                if band[upper] == band[*item] {
                    y += gap(upper, *item);
                } else {
                    runs.insert((at, band[upper]), y);
                    y = 0.0;
                }
            }
            offset[*item] = y;
            y += heights[*item];
            if *item < count {
                y += under(*item);
            }
        }
        if let Some(last) = column.last() {
            runs.insert((at, band[*last]), y);
        }
    }
    let mut tall = vec![0.0; framed + 1];
    for ((_, band), run) in &runs {
        tall[*band] = f64::max(tall[*band], *run);
    }
    // The layers a frame spans: those of what it holds, its frames included.
    let mut span: Vec<Option<(usize, usize)>> = vec![None; framed];
    let widen = |span: &mut Option<(usize, usize)>, first: usize, last: usize| {
        *span = Some(span.map_or((first, last), |(from, to)| (from.min(first), to.max(last))));
    };
    let pass = |span: &mut Vec<Option<(usize, usize)>>| {
        for frame in (0..framed).rev() {
            if let (Some(holder), Some((first, last))) =
                (over(frame).filter(|holder| *holder != free), span[frame])
            {
                widen(&mut span[holder], first, last);
            }
        }
    };
    for node in (0..count).filter(|node| linked[*node] && band[*node] != free) {
        widen(&mut span[band[node]], layer[node], layer[node]);
    }
    pass(&mut span);
    // How far apart the rows of a grid stand.
    let pitch = BOX_HEIGHT
        + ROW_GAP
        + (0..count)
            .filter(|node| !linked[*node])
            .map(under)
            .fold(0.0, f64::max);
    // What a frame holds that no link here touches lies in a grid under the
    // rest of it, from the frame's first layer on. Precomputed in O(N) once.
    let mut apart_by_frame: Vec<Vec<usize>> = vec![Vec::new(); framed];
    let mut apart_pos = vec![0usize; count];
    let mut loose = Vec::new();
    let mut loose_pos = vec![0usize; count];
    for node in 0..count {
        if linked[node] {
            continue;
        }
        let b = band[node];
        if b < framed {
            apart_pos[node] = apart_by_frame[b].len();
            apart_by_frame[b].push(node);
        } else if b == free {
            loose_pos[node] = loose.len();
            loose.push(node);
        }
    }
    let mut across = vec![1usize; framed];
    for frame in 0..framed {
        let unlinked = apart_by_frame[frame].len();
        if unlinked > 0 {
            let (first, last) = span[frame].unwrap_or((0, 0));
            across[frame] = (last - first + 1).max(unlinked.min(3));
            widen(&mut span[frame], first, first + across[frame] - 1);
        }
    }
    pass(&mut span);
    // A frame stands off what it holds, a holder off its frames, so the
    // picture begins further in the deeper its frames go.
    let mut deep = vec![1usize; framed];
    for frame in (0..framed).rev() {
        if let Some(holder) = over(frame).filter(|holder| *holder != free) {
            deep[holder] = deep[holder].max(deep[frame] + 1);
        }
    }
    let inset = FRAME_PAD * deep.iter().copied().max().unwrap_or(0) as f64;
    let mut band_top = vec![0.0; framed + 1];
    let mut grid_in = vec![0.0; framed];
    let mut edge = vec![(0.0, 0.0); framed];
    let mut y = MARGIN;
    for step in &steps {
        match *step {
            Step::Begin(frame) => {
                edge[frame].0 = y;
                y += FRAME_HEAD;
            }
            Step::Own(frame) => {
                band_top[frame] = y;
                y += tall[frame];
                let unlinked = apart_by_frame[frame].len();
                if unlinked > 0 {
                    if tall[frame] > 0.0 {
                        y += ROW_GAP;
                    }
                    grid_in[frame] = y;
                    let rows = unlinked.div_ceil(across[frame]);
                    y += rows as f64 * pitch - ROW_GAP;
                }
            }
            Step::End(frame) => {
                y += FRAME_PAD;
                edge[frame].1 = y;
                y += if over(frame) == Some(free) {
                    MARGIN
                } else {
                    FRAME_PAD
                };
            }
        }
    }
    band_top[free] = y;
    let mut top = vec![0.0; heights.len()];
    for (at, column) in columns.iter().enumerate() {
        for item in column {
            let band = band[*item];
            top[*item] = band_top[band] + (tall[band] - runs[&(at, band)]) / 2.0 + offset[*item];
        }
    }
    let left = |at: usize| MARGIN + inset + at as f64 * (BOX_WIDTH + COLUMN_GAP);
    let layered_width = if layers == 0 {
        0.0
    } else {
        left(layers - 1) + BOX_WIDTH + MARGIN + inset
    };
    let below_all = band_top[free] + tall[free];
    let per_row = (((layered_width - MARGIN) / (BOX_WIDTH + ROW_GAP)).floor() as usize).max(4);
    let apart = (layers > 0 && !loose.is_empty()).then_some(below_all + 2.0 * MARGIN);
    let grid_top = apart.map_or(band_top[free], |apart| apart + CAPTION);
    let mut nodes = vec![];
    for node in 0..count {
        nodes.push(if linked[node] {
            Placed {
                x: left(layer[node]),
                y: top[node],
                layer: Some(layer[node]),
                cycle: size[group[node]] > 1,
            }
        } else if band[node] != free {
            let frame = band[node];
            let at = apart_pos[node];
            Placed {
                x: left(span[frame].map_or(0, |(first, _)| first) + at % across[frame]),
                y: grid_in[frame] + (at / across[frame]) as f64 * pitch,
                layer: None,
                cycle: false,
            }
        } else {
            let at = loose_pos[node];
            Placed {
                x: MARGIN + inset + (at % per_row) as f64 * (BOX_WIDTH + ROW_GAP),
                y: grid_top + (at / per_row) as f64 * pitch,
                layer: None,
                cycle: false,
            }
        });
    }
    let frames = (0..framed)
        .map(|frame| match span[frame] {
            Some((first, last)) => {
                let pad = FRAME_PAD * deep[frame] as f64;
                Framed {
                    x: left(first) - pad,
                    y: edge[frame].0,
                    width: left(last) + BOX_WIDTH + pad - (left(first) - pad),
                    height: edge[frame].1 - edge[frame].0,
                }
            }
            None => Framed::default(),
        })
        .collect::<Vec<_>>();
    // Where each link leaves the box at its near end and reaches the box
    // at its far end, as a distance from the middle of the side: the links
    // of one side stand in the order of where they lead next, so the one
    // that leads up leaves above the one that leads down.
    let centre = |item: usize| top[item] + heights[item] / 2.0;
    let mut port = vec![(0.0f64, 0.0f64); chains.len()];
    if routing.ports {
        // For each box the links at one of its sides: where each leads
        // next, and which link it is.
        type Side = Vec<Vec<(f64, usize)>>;
        let (mut right, mut left_side): (Side, Side) = (vec![vec![]; count], vec![vec![]; count]);
        for (index, chain) in chains.iter().enumerate() {
            if portal[index] {
                continue;
            }
            let last = chain.len() - 1;
            right[chain[0]].push((centre(chain[1]), index));
            left_side[chain[last]].push((centre(chain[last - 1]), index));
        }
        let apart = |side: &mut Vec<(f64, usize)>| {
            side.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            let many = side.len();
            let room = PORT_SPREAD.min(PORT_GAP * many.saturating_sub(1) as f64);
            side.iter()
                .enumerate()
                .map(|(place, (_, index))| {
                    let part = if many < 2 {
                        0.0
                    } else {
                        place as f64 / (many - 1) as f64 - 0.5
                    };
                    (*index, room * part)
                })
                .collect::<Vec<_>>()
        };
        for node in 0..count {
            for (index, offset) in apart(&mut right[node]) {
                port[index].0 = offset;
            }
            for (index, offset) in apart(&mut left_side[node]) {
                port[index].1 = offset;
            }
        }
    }
    let mut edge_by_pair: HashMap<(usize, usize), usize> = HashMap::with_capacity(edges.len());
    for (idx, (from, to, _)) in edges.iter().enumerate() {
        edge_by_pair.entry((*from, *to)).or_insert(idx);
    }
    let mut wires = vec![];
    for (index, (chain, (_, _, back, pair))) in chains.iter().zip(&wired).enumerate() {
        let edge = edge_by_pair.get(pair).copied().unwrap_or(0);
        // The stops of the link from left to right; a slot is crossed straight.
        let mut stops = vec![];
        let last = chain.len() - 1;
        for (step, item) in chain.iter().enumerate() {
            let x = left(column_of[*item]);
            let y = centre(*item)
                + if step == 0 {
                    port[index].0
                } else if step == last {
                    port[index].1
                } else {
                    0.0
                };
            if step > 0 {
                stops.push(((x, y), false));
            }
            if step < last {
                stops.push(((x + BOX_WIDTH, y), step > 0));
            }
        }
        if *back {
            // Walk it right to left; the straight runs keep their places.
            let straight = stops
                .iter()
                .map(|(_, straight)| *straight)
                .collect::<Vec<_>>();
            stops.reverse();
            for (index, stop) in stops.iter_mut().enumerate() {
                stop.1 = index > 0 && straight[straight.len() - index];
            }
        }
        let mut path = format!("M{:.1} {:.1}", stops[0].0 .0, stops[0].0 .1);
        for pair in stops.windows(2) {
            if pair[1].1 {
                path.push_str(&format!(" L{:.1} {:.1}", pair[1].0 .0, pair[1].0 .1));
            } else {
                curve(&mut path, pair[0].0, pair[1].0);
            }
        }
        wires.push(Wire {
            edge,
            path,
            back: *back,
            portal: portal[index],
        });
    }
    let rows = loose.len().div_ceil(per_row);
    let grid_width = MARGIN + inset + loose.len().min(per_row) as f64 * (BOX_WIDTH + ROW_GAP);
    let framed_width = frames
        .iter()
        .map(|frame| frame.x + frame.width + MARGIN)
        .fold(0.0, f64::max);
    Plan {
        nodes,
        wires,
        frames,
        width: layered_width
            .max(grid_width)
            .max(framed_width)
            .max(BOX_WIDTH + 2.0 * MARGIN),
        height: if loose.is_empty() {
            below_all + MARGIN
        } else {
            grid_top + rows as f64 * pitch
        }
        .max(BOX_HEIGHT + 2.0 * MARGIN),
        apart,
    }
}

/// Applies persisted `(x, y)` user Shift+drag pins onto `plan.nodes`, expands
/// enclosing frames cleanly, and re-routes wires touching pinned nodes.
pub(super) fn apply_pin_overrides_to_plan(
    plan: &mut Plan,
    node_keys: &[String],
    pairs: &[(usize, usize)],
    pins: &BTreeMap<String, (f64, f64)>,
) -> BTreeSet<usize> {
    let mut moved = BTreeSet::new();
    if pins.is_empty() {
        return moved;
    }
    let orig_pos: Vec<(f64, f64)> = plan.nodes.iter().map(|n| (n.x, n.y)).collect();
    for (idx, key) in node_keys.iter().enumerate() {
        if let (Some(node), Some(&(px, py))) = (plan.nodes.get_mut(idx), pins.get(key)) {
            node.x = px.max(MARGIN);
            node.y = py.max(MARGIN);
            plan.width = plan.width.max(node.x + BOX_WIDTH + MARGIN);
            plan.height = plan.height.max(node.y + BOX_HEIGHT + MARGIN);
            moved.insert(idx);
        }
    }
    if moved.is_empty() {
        return moved;
    }
    for frame in &mut plan.frames {
        if frame.width <= 0.0 || frame.height <= 0.0 {
            continue;
        }
        let mut x0 = frame.x;
        let mut y0 = frame.y;
        let mut x1 = frame.x + frame.width;
        let mut y1 = frame.y + frame.height;
        let mut changed = false;
        for &idx in &moved {
            let (ox, oy) = orig_pos[idx];
            if ox >= frame.x - 1.0
                && ox + BOX_WIDTH <= frame.x + frame.width + 1.0
                && oy >= frame.y - 1.0
                && oy + BOX_HEIGHT <= frame.y + frame.height + 1.0
            {
                let moved_node = &plan.nodes[idx];
                x0 = x0.min((moved_node.x - FRAME_PAD).max(MARGIN * 0.5));
                y0 = y0.min((moved_node.y - FRAME_HEAD).max(MARGIN * 0.5));
                x1 = x1.max(moved_node.x + BOX_WIDTH + FRAME_PAD);
                y1 = y1.max(moved_node.y + BOX_HEIGHT + FRAME_PAD);
                changed = true;
            }
        }
        if changed {
            frame.x = x0;
            frame.y = y0;
            frame.width = (x1 - x0).max(frame.width);
            frame.height = (y1 - y0).max(frame.height);
            plan.width = plan.width.max(frame.x + frame.width + MARGIN);
            plan.height = plan.height.max(frame.y + frame.height + MARGIN);
        }
    }
    // Distribute port offsets cleanly along touched nodes so multiple pinned wires do not overlap.
    let mut side_ports: HashMap<(usize, bool), Vec<(f64, usize)>> = HashMap::new();
    for (wi, wire) in plan.wires.iter().enumerate() {
        let Some(&(from, to)) = pairs.get(wire.edge) else {
            continue;
        };
        if !moved.contains(&from) && !moved.contains(&to) {
            continue;
        }
        let (Some(a), Some(b)) = (plan.nodes.get(from), plan.nodes.get(to)) else {
            continue;
        };
        let from_right = a.x + BOX_WIDTH <= b.x || b.x + BOX_WIDTH > a.x;
        side_ports
            .entry((from, from_right))
            .or_default()
            .push((b.y, wi));
        side_ports
            .entry((to, !from_right))
            .or_default()
            .push((a.y, wi));
    }
    let mut start_off = vec![0.0f64; plan.wires.len()];
    let mut end_off = vec![0.0f64; plan.wires.len()];
    for ((node_idx, is_from_side), mut list) in side_ports {
        list.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let n = list.len();
        let spread = PORT_SPREAD.min(PORT_GAP * n.saturating_sub(1) as f64);
        for (slot, &(_, wi)) in list.iter().enumerate() {
            let off = if n < 2 {
                0.0
            } else {
                spread * (slot as f64 / (n - 1) as f64 - 0.5)
            };
            if let Some(&(f, t)) = pairs.get(plan.wires[wi].edge) {
                let a = &plan.nodes[f];
                let b = &plan.nodes[t];
                let f_right = a.x + BOX_WIDTH <= b.x || b.x + BOX_WIDTH > a.x;
                if f == node_idx && is_from_side == f_right {
                    start_off[wi] = off;
                }
                if t == node_idx && is_from_side == !f_right {
                    end_off[wi] = off;
                }
            }
        }
    }
    for (wi, wire) in plan.wires.iter_mut().enumerate() {
        let Some(&(from, to)) = pairs.get(wire.edge) else {
            continue;
        };
        if !moved.contains(&from) && !moved.contains(&to) {
            continue;
        }
        let (Some(a), Some(b)) = (plan.nodes.get(from), plan.nodes.get(to)) else {
            continue;
        };
        let path =
            route_pinned_wire_path((a.x, a.y), (b.x, b.y), start_off[wi], end_off[wi], false);
        wire.path = path;
    }
    moved
}

/// Computes a smooth SVG cubic wire path between pinned/dragged cards `(ax, ay)` and
/// `(bx, by)` without cutting across card interiors when horizontally overlapped,
/// optionally bowing detoured wires outward so spline curves stay legible.
pub(super) fn route_pinned_wire_path(
    a: (f64, f64),
    b: (f64, f64),
    sy_off: f64,
    ey_off: f64,
    detour: bool,
) -> String {
    let sy_mid = a.1 + BOX_HEIGHT * 0.5 + sy_off;
    let ey_mid = b.1 + BOX_HEIGHT * 0.5 + ey_off;
    if a.0 + BOX_WIDTH <= b.0 {
        let sx = a.0 + BOX_WIDTH;
        let ex = b.0;
        if detour && (ex - sx).abs() > 24.0 {
            let bow = 22.0;
            let dip = sy_mid.max(ey_mid) + bow;
            return format!(
                "M{sx:.1} {sy_mid:.1} C{:.1} {dip:.1} {:.1} {dip:.1} {ex:.1} {ey_mid:.1}",
                sx + (ex - sx) * 0.35,
                sx + (ex - sx) * 0.65,
            );
        }
        let mut path = format!("M{sx:.1} {sy_mid:.1}");
        curve(&mut path, (sx, sy_mid), (ex, ey_mid));
        return path;
    }
    if b.0 + BOX_WIDTH <= a.0 {
        let sx = a.0;
        let ex = b.0 + BOX_WIDTH;
        if detour && (sx - ex).abs() > 24.0 {
            let dip = sy_mid.max(ey_mid) + 22.0;
            return format!(
                "M{sx:.1} {sy_mid:.1} C{:.1} {dip:.1} {:.1} {dip:.1} {ex:.1} {ey_mid:.1}",
                sx + (ex - sx) * 0.35,
                sx + (ex - sx) * 0.65,
            );
        }
        let mut path = format!("M{sx:.1} {sy_mid:.1}");
        curve(&mut path, (sx, sy_mid), (ex, ey_mid));
        return path;
    }
    // Horizontally overlapped cards: route top-to-bottom / bottom-to-top when vertically separated,
    // or loop through a clear right-side channel so the wire stays outside both card bodies.
    let sx_mid = (a.0 + BOX_WIDTH * 0.5 + sy_off * 0.5).clamp(a.0 + 12.0, a.0 + BOX_WIDTH - 12.0);
    let ex_mid = (b.0 + BOX_WIDTH * 0.5 + ey_off * 0.5).clamp(b.0 + 12.0, b.0 + BOX_WIDTH - 12.0);
    if a.1 + BOX_HEIGHT + 8.0 <= b.1 {
        let sy = a.1 + BOX_HEIGHT;
        let ey = b.1;
        let ctrl = ((ey - sy) * 0.45).max(16.0);
        return format!(
            "M{sx_mid:.1} {sy:.1} C{sx_mid:.1} {:.1} {ex_mid:.1} {:.1} {ex_mid:.1} {ey:.1}",
            sy + ctrl,
            ey - ctrl,
        );
    }
    if b.1 + BOX_HEIGHT + 8.0 <= a.1 {
        let sy = a.1;
        let ey = b.1 + BOX_HEIGHT;
        let ctrl = ((sy - ey) * 0.45).max(16.0);
        return format!(
            "M{sx_mid:.1} {sy:.1} C{sx_mid:.1} {:.1} {ex_mid:.1} {:.1} {ex_mid:.1} {ey:.1}",
            sy - ctrl,
            ey + ctrl,
        );
    }
    let sx = a.0 + BOX_WIDTH;
    let ex = b.0 + BOX_WIDTH;
    let channel_x = a.0.max(b.0) + BOX_WIDTH + if detour { 36.0 } else { 26.0 };
    format!("M{sx:.1} {sy_mid:.1} C{channel_x:.1} {sy_mid:.1} {channel_x:.1} {ey_mid:.1} {ex:.1} {ey_mid:.1}")
}
/// How far the first link that passes over or under a row of layers stands
/// off it, and how much further each next one.
pub(super) const ARC_FIRST: f64 = 18.0;
pub(super) const ARC_STEP: f64 = 12.0;
/// How far apart two links leave or reach one edge of a box.
pub(super) const ARC_APART: f64 = 10.0;
/// Places `count` nodes in one row, in the order they come in, and routes
/// `edges` (user, used, weight) between them. This is the picture of layers:
/// their order is their own, not what their links make of it. A link to the
/// next node runs straight; one that passes a node arcs over the row; and a
/// link back, from a later node to an earlier one, arcs under it. No two arcs
/// of a side lie at one height, the shorter nearer the row, and those that
/// share an edge of a box leave it and reach it apart, so they nest.
pub fn plan_row(count: usize, edges: &[(usize, usize, u64)]) -> Plan {
    let mut seen = BTreeSet::new();
    let drawn = edges
        .iter()
        .enumerate()
        .filter(|(_, (from, to, _))| {
            from != to && *from < count && *to < count && seen.insert((*from, *to))
        })
        .map(|(edge, (from, to, _))| (edge, *from, *to))
        .collect::<Vec<_>>();
    // The arcs of a side, the shorter first.
    let arcs = |back: bool| {
        let mut arcs = drawn
            .iter()
            .filter(
                |(_, from, to)| {
                    if back {
                        from > to
                    } else {
                        *to > from + 1
                    }
                },
            )
            .map(|(_, from, to)| (from.abs_diff(*to), *from.min(to), (*from, *to)))
            .collect::<Vec<_>>();
        arcs.sort_unstable();
        arcs.into_iter().map(|(.., pair)| pair).collect::<Vec<_>>()
    };
    let (over, under) = (arcs(false), arcs(true));
    let reach = |arcs: &[(usize, usize)]| match arcs.len() {
        0 => 0.0,
        many => ARC_FIRST + ARC_STEP * (many - 1) as f64,
    };
    let top = MARGIN + reach(&over);
    let left = |at: usize| MARGIN + at as f64 * (BOX_WIDTH + COLUMN_GAP);
    // An arc leaves a box on the half towards where it runs and reaches a box
    // on the half it comes from; each next one of a box stands further out.
    let route = |arcs: &[(usize, usize)], below: bool| {
        let (mut leaving, mut reaching) = (vec![0usize; count], vec![0usize; count]);
        arcs.iter()
            .enumerate()
            .map(|(level, (from, to))| {
                let out = ARC_APART * leaving[*from] as f64;
                let into = ARC_APART * reaching[*to] as f64;
                leaving[*from] += 1;
                reaching[*to] += 1;
                // Over the row an arc runs to the right, under it to the left.
                let (start, end) = if below {
                    (
                        left(*from) + BOX_WIDTH * 0.25 + out,
                        left(*to) + BOX_WIDTH * 0.75 - into,
                    )
                } else {
                    (
                        left(*from) + BOX_WIDTH * 0.75 - out,
                        left(*to) + BOX_WIDTH * 0.25 + into,
                    )
                };
                // A curve held at both ends by handles of one length bulges
                // three quarters of that length.
                let pull = (ARC_FIRST + ARC_STEP * level as f64) / 0.75;
                let (edge, handle) = if below {
                    (top + BOX_HEIGHT, top + BOX_HEIGHT + pull)
                } else {
                    (top, top - pull)
                };
                (
                    (*from, *to),
                    format!(
                        "M{start:.1} {edge:.1} C{start:.1} {handle:.1} {end:.1} {handle:.1} {end:.1} {edge:.1}"
                    ),
                )
            })
            .collect::<Vec<_>>()
    };
    let mut paths = route(&over, false);
    paths.extend(route(&under, true));
    let middle = top + BOX_HEIGHT / 2.0;
    let wires = drawn
        .iter()
        .map(|(edge, from, to)| Wire {
            edge: *edge,
            path: match paths.iter().find(|(pair, _)| *pair == (*from, *to)) {
                Some((_, path)) => path.clone(),
                None => format!(
                    "M{:.1} {middle:.1} L{:.1} {middle:.1}",
                    left(*from) + BOX_WIDTH,
                    left(*to)
                ),
            },
            back: from > to,
            portal: false,
        })
        .collect();
    Plan {
        nodes: (0..count)
            .map(|node| Placed {
                x: left(node),
                y: top,
                layer: Some(node),
                cycle: false,
            })
            .collect(),
        wires,
        frames: vec![],
        width: left(count.max(1) - 1) + BOX_WIDTH + MARGIN,
        height: top + BOX_HEIGHT + reach(&under) + MARGIN,
        apart: None,
    }
}
