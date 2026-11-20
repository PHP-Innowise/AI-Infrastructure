use super::*;

pub(super) const TABS: [&str; 3] = ["commits", "pulls", "branches"];
pub(super) const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
/// A day as a history shows it, in universal time.
pub(super) fn day(at: i64) -> String {
    // Days since 1970 to a civil date (Hinnant's algorithm).
    let days = at.div_euclid(86_400) + 719_468;
    let (era, of_era) = (days.div_euclid(146_097), days.rem_euclid(146_097));
    let year = (of_era - of_era / 1_460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year + year / 4 - year / 100);
    let shifted = (5 * of_year + 2) / 153;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    format!(
        "{} {} {}",
        of_year - (153 * shifted + 2) / 5 + 1,
        MONTHS[(month - 1) as usize],
        year + era * 400 + i64::from(month <= 2)
    )
}
/// How long ago something happened, as a history shows it: a span for a
/// month, a day after that.
pub fn ago(now: i64, at: i64) -> String {
    let passed = now - at;
    let span =
        |count: i64, unit: &str| format!("{count} {unit}{} ago", if count == 1 { "" } else { "s" });
    match passed {
        ..60 => "just now".to_owned(),
        60..3_600 => span(passed / 60, "minute"),
        3_600..86_400 => span(passed / 3_600, "hour"),
        86_400..172_800 => "yesterday".to_owned(),
        172_800..2_592_000 => span(passed / 86_400, "day"),
        _ => day(at),
    }
}
/// The rows the open tab of the picker offers as they stand, each with the
/// target it chooses. Commits and pull requests are searched by the server;
/// branches are few enough to filter here.
pub fn offered(pick: &V) -> Vec<V> {
    let rows = m::array(&pick["rows"]);
    let query = m::text(pick, "query");
    let with = |row: &V, target: String| {
        let mut row = row.clone();
        row["target"] = json!(target);
        row
    };
    match pick["tab"].as_str() {
        Some("pulls") => {
            let mut offered = rows
                .iter()
                .map(|pull| with(pull, format!("pr:{}", pull["number"].as_u64().unwrap_or(0))))
                .collect::<Vec<_>>();
            // A number names a request whatever its state, also one the list
            // does not hold.
            if let Some(target) = typed(&query).filter(|target| target.starts_with("pr:")) {
                if !offered.iter().any(|row| row["target"] == json!(target)) {
                    let number = target[3..].parse::<u64>().unwrap_or(0);
                    offered.insert(
                        0,
                        json!({"target": target, "number": number, "title": "Look up this pull request", "typed": true}),
                    );
                }
            }
            offered
        }
        Some("branches") => {
            let needle = query.trim().to_lowercase();
            rows.iter()
                .filter(|branch| {
                    needle.is_empty() || m::text(branch, "name").to_lowercase().contains(&needle)
                })
                .map(|branch| with(branch, format!("ref:{}", m::text(branch, "name"))))
                .collect()
        }
        _ => rows
            .iter()
            .map(|commit| with(commit, format!("commit:{}", m::text(commit, "commit"))))
            .collect(),
    }
}
/// The list of the picker: one row per commit, pull request or branch, the
/// active one marked, and under a history that goes on, a way to read more.
pub fn options(pick: &V, now: i64) -> String {
    let active = pick["active"].as_u64().unwrap_or(0) as usize;
    let tab = pick["tab"].as_str().unwrap_or("commits");
    if tab == "commits" {
        let rows = offered(pick);
        let active_idx = (!rows.is_empty()).then(|| active.min(rows.len() - 1));
        let selected_target = pick["selected"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| m::text(&state(), "target"));
        let selected_sha = selected_target.strip_prefix("commit:");
        return crate::git_commit_graph::render_commit_graph_rows(
            &rows,
            active_idx,
            selected_sha,
            m::yes(&pick["more"]),
            now,
            crate::git_commit_graph::CommitGraphSurface::Picker,
        );
    }
    let badge = |text: &str, kind: &str| {
        format!(
            "<span class=\"graph-pick-badge\" data-kind=\"{kind}\">{}</span>",
            escape(text)
        )
    };
    let cell = |class: &str, text: &str| {
        if text.is_empty() {
            String::new()
        } else {
            format!("<span class=\"graph-pick-{class}\">{}</span>", escape(text))
        }
    };
    let mut html = String::new();
    for (index, row) in offered(pick).iter().enumerate() {
        let when = row["at"]
            .as_i64()
            .map(|at| cell("when", &ago(now, at)))
            .unwrap_or_default();
        // The row's own words, its badges, and what stands to their right.
        let (title, badges, rest) = match tab {
            "pulls" => {
                let mut badges = String::new();
                if m::yes(&row["draft"]) {
                    badges.push_str(&badge("draft", "draft"));
                }
                if let Some(state @ ("merged" | "closed")) = row["state"].as_str() {
                    badges.push_str(&badge(state, state));
                }
                let branches = if row["head"].is_string() {
                    format!("{} → {}", m::text(row, "head"), m::text(row, "base"))
                } else {
                    String::new()
                };
                (
                    format!(
                        "<span class=\"graph-pick-number\">#{}</span>{}",
                        row["number"].as_u64().unwrap_or(0),
                        cell("title", &m::text(row, "title"))
                    ),
                    badges,
                    format!(
                        "{}{}{when}",
                        cell("detail", &branches),
                        cell("who", &m::text(row, "author"))
                    ),
                )
            }
            "branches" => {
                let mut badges = String::new();
                if m::yes(&row["current"]) {
                    badges.push_str(&badge("checked out", "current"));
                }
                if let Some(pull) = row["pull"].as_u64() {
                    badges.push_str(&badge(&format!("#{pull}"), "open"));
                }
                (
                    cell("title", &m::text(row, "name")),
                    badges,
                    format!("{}{when}", cell("detail", &m::text(row, "subject"))),
                )
            }
            _ => {
                let mut badges = m::array(&row["refs"])
                    .iter()
                    .map(|name| badge(&m::text(name, "name"), &m::text(name, "kind")))
                    .collect::<String>();
                if m::yes(&row["merge"]) {
                    badges.push_str(&badge("merge", "merge"));
                }
                (
                    cell("title", &m::text(row, "subject")),
                    badges,
                    format!(
                        "{}{when}<code class=\"graph-pick-code\">{}</code>",
                        cell("who", &m::text(row, "author")),
                        escape(&m::text(row, "short"))
                    ),
                )
            }
        };
        html.push_str(&format!(
            "<li class=\"graph-pick-row\" id=\"graph-pick-row-{index}\" role=\"option\" aria-selected=\"{}\" data-action=\"graph-pick-choose\" data-payload=\"{}\"><span class=\"graph-pick-main\">{title}{}</span>{rest}</li>",
            index == active,
            payload(&row["target"]),
            if badges.is_empty() {
                String::new()
            } else {
                format!("<span class=\"graph-pick-badges\">{badges}</span>")
            },
        ));
    }
    if tab == "commits" && m::yes(&pick["more"]) {
        html.push_str("<li class=\"graph-pick-more\" role=\"presentation\"><button type=\"button\" class=\"button compact\" data-action=\"graph-pick-more\">Show older commits</button></li>");
    }
    html
}
/// The line under the picker's list: why it is empty, or how to walk it.
pub fn remark(pick: &V) -> String {
    let tab = pick["tab"].as_str().unwrap_or("commits");
    let count = offered(pick).len();
    if let Some(failed) = pick["failed"].as_str() {
        return failed.to_owned();
    }
    let mut parts = Vec::new();
    if let Some(reason) = pick["reason"].as_str().filter(|_| tab == "pulls") {
        parts.push(format!(
            "Pull requests cannot be listed: {reason}. A number can still be typed."
        ));
    }
    if m::yes(&pick["loading"]) && count == 0 {
        parts.push("Reading…".to_owned());
    } else if count == 0 && pick["reason"].is_null() {
        parts.push(
            match tab {
                "pulls" => "No pull request matches.",
                "branches" => "No branch matches.",
                _ => "No commit matches.",
            }
            .to_owned(),
        );
    } else if count > 0 {
        parts.push("↑ ↓ to move · Enter to choose · Esc to close".to_owned());
    }
    parts.join(" ")
}
/// Whose history the Commits tab lists: the checked-out branch, every
/// branch, or one of the local ones.
pub fn scopes(place: &V, branches: &V, chosen: &str) -> String {
    let option = |value: &str, label: &str| {
        format!(
            "<option value=\"{}\"{}>{}</option>",
            escape(value),
            if value == chosen { " selected" } else { "" },
            escape(label)
        )
    };
    let mut html = option(
        "",
        &match place["branch"].as_str() {
            Some(branch) => format!("This branch: {branch}"),
            None => "This checkout".to_owned(),
        },
    );
    html.push_str(&option("*", "All branches"));
    for branch in m::array(branches) {
        let name = m::text(&branch, "name");
        if !m::yes(&branch["remote"]) && !m::yes(&branch["current"]) {
            html.push_str(&option(&name, &name));
        }
    }
    html
}
/// What stands before the note of a change whose own graph is not there:
/// how its build is going, and whose links are drawn meanwhile.
pub fn interim(view: &V, phase: &str) -> String {
    let phase = format_graph_status_text(phase);
    let first = m::yes(&view["preliminary"]);
    match (view["state"] == "building", first) {
        (true, true) => format!(
            "Building the graph of that commit. {phase}. Until it is ready, the links are those the files have in the checkout."
        ),
        (true, false) => format!("Building the graph of that commit. {phase}."),
        (false, true) => {
            "The links are those the files have in the checkout, not in that commit.".to_owned()
        }
        _ => String::new(),
    }
}
/// A name with what a search typed marked in it: each word once, where the
/// name first holds it. A folder or a file is named by a word with a slash
/// too, by what follows the last slash, at the start of its name. For the
/// name of a symbol `own` says where it begins when it is shown with what
/// holds it, `total()` in `Cart.total()`: a word is looked for there first.
/// Letters match whatever their case.
pub fn marked(name: &str, query: &str, own: Option<usize>) -> String {
    let lower = name.to_lowercase();
    // A mark lies where the lower case holds a word, which is where the
    // name holds it only if the two are equally long.
    if lower.len() != name.len() {
        return escape(name);
    }
    let start = own.filter(|at| lower.is_char_boundary(*at)).unwrap_or(0);
    let mut spans = vec![];
    for word in query.split_whitespace() {
        let word = word.to_lowercase();
        let found = match word.rsplit_once('/') {
            Some((_, tail)) => (own.is_none() && !tail.is_empty() && lower.starts_with(tail))
                .then_some((0, tail.len())),
            None => lower[start..]
                .find(&word)
                .map(|at| start + at)
                .or_else(|| lower.find(&word))
                .map(|at| (at, at + word.len())),
        };
        if let Some((from, to)) = found {
            if name.is_char_boundary(from) && name.is_char_boundary(to) {
                spans.push((from, to));
            }
        }
    }
    spans.sort_unstable();
    let mut html = String::new();
    let mut at = 0;
    for (from, to) in spans {
        if to <= at {
            continue;
        }
        let from = from.max(at);
        html.push_str(&escape(&name[at..from]));
        html.push_str(&format!("<mark>{}</mark>", escape(&name[from..to])));
        at = to;
    }
    html.push_str(&escape(&name[at..]));
    html
}
/// What a search of the project found, as rows to choose from: what each
/// place is, its name with what was typed marked in it, and where it lies,
/// the folder of a folder or a file and the file and the line of a name.
pub fn found(finder: &V) -> String {
    let active = finder["active"].as_u64().unwrap_or(0) as usize;
    let asked = m::text(finder, "asked");
    let mut html = String::new();
    for (index, row) in m::array(&finder["rows"]).iter().enumerate() {
        let what = m::text(row, "what");
        let place = match (row["file"].as_str(), row["line"].as_u64()) {
            (Some(file), Some(line)) if line > 0 => format!("{file}:{line}"),
            (Some(file), _) => file.to_owned(),
            _ => m::text(row, "in"),
        };
        html.push_str(&format!(
            "<li class=\"graph-pick-row graph-find-row\" id=\"graph-find-row-{index}\" role=\"option\" aria-selected=\"{}\" data-action=\"graph-find-choose\" data-payload=\"{index}\" title=\"{}\"><span class=\"graph-find-what\">{}</span><span class=\"graph-pick-main\"><span class=\"graph-pick-title\">{}{}</span></span><span class=\"graph-find-where\"><bdi>{}</bdi></span></li>",
            index == active,
            escape(&m::text(row, "path")),
            escape(&what),
            marked(
                &m::text(row, "name"),
                &asked,
                row["file"]
                    .is_string()
                    .then(|| row["own"].as_u64().unwrap_or(0) as usize),
            ),
            if what == "folder" { "/" } else { "" },
            escape(&place),
        ));
    }
    html
}
/// The line under what a search found: why there is nothing, or how many
/// there are and how to walk them.
pub fn found_note(finder: &V) -> String {
    if let Some(failed) = finder["failed"].as_str() {
        return failed.to_owned();
    }
    let rows = m::array(&finder["rows"]).len() as u64;
    if m::yes(&finder["loading"]) && rows == 0 {
        return "Searching…".to_owned();
    }
    if m::yes(&finder["missing"]) {
        return "There is no graph to search yet.".to_owned();
    }
    let tests = if m::yes(&finder["notests"]) {
        " Tests are left out."
    } else {
        ""
    };
    let found = finder["found"].as_u64().unwrap_or(0);
    if found == 0 {
        return format!("Nothing in the graph is called that.{tests}");
    }
    let told = if rows < found {
        format!(
            "The {} best of {} found; type more to narrow them.",
            count(rows),
            count(found)
        )
    } else {
        format!("{} found.", count(found))
    };
    // While the other end of a path is chosen, a row is that end.
    let enter = if m::yes(&finder["choosing"]) {
        "Enter to lead the path there"
    } else {
        "Enter to go there"
    };
    format!("{told}{tests} ↑ ↓ to move · {enter} · Esc to close")
}
/// The map as it shows a place a search found. A name opens its file on
/// itself, in the map that is open. A folder or a file that the picture
/// draws is selected where it is; any other is shown among its own
/// neighbours, in the folder it lies in, in the whole project, and a folder
/// that was spread out there is one box again, to be selected.
pub fn sought(mut state: V, row: &V, drawn: bool) -> V {
    let path = row["path"].as_str().unwrap_or("");
    if let Some(file) = row["file"].as_str() {
        state = unfollowed(state);
        state["scope"] = json!(file);
        state["filter"] = json!("");
    } else if !drawn {
        state = unfollowed(state);
        state["target"] = json!("");
        state["scope"] = json!(parent(path));
        state["filter"] = json!("");
        if row["what"] == "folder" {
            state = spread(state, path, true);
        }
    }
    state["focus"] = json!(path);
    state
}
/// The scale at which a picture lies wholly in the room it has: never
/// larger than it is drawn, and no smaller than `SMALLEST`. A picture or a
/// room without a size is left as it is.
#[cfg_attr(not(test), allow(dead_code))]
pub fn scale(picture: (f64, f64), room: (f64, f64)) -> f64 {
    if picture.0 <= 0.0 || picture.1 <= 0.0 || room.0 <= 0.0 || room.1 <= 0.0 {
        return 1.0;
    }
    (room.0 / picture.0)
        .min(room.1 / picture.1)
        .clamp(SMALLEST, 1.0)
}
/// The style of a picture of one size drawn at a scale. A smaller picture
/// takes the room of its smaller size, so nothing is scrolled to but it.
#[cfg_attr(not(test), allow(dead_code))]
pub fn scaled(size: (f64, f64), scale: f64) -> String {
    let plain = format!("width:{:.0}px;height:{:.0}px", size.0, size.1);
    if (scale - 1.0).abs() < 1e-4 {
        return plain;
    }
    let dx = size.0 * (scale - 1.0);
    let dy = size.1 * (scale - 1.0);
    let dx = if dx.abs() < 0.5 { 0.0 } else { dx };
    let dy = if dy.abs() < 0.5 { 0.0 } else { dy };
    format!(
        "{plain};transform:scale({scale:.4});transform-origin:0 0;margin:0 {dx:.0}px {dy:.0}px 0"
    )
}
pub(super) const GRAPH_MAP_LOD: graph_viewport::LodThresholds = graph_viewport::DEFAULT_LOD;
/// How much a picture tells at a scale: `far` is an overview of names,
/// `near` adds to each box where it lies and how many links it has, and
/// `mid` is the picture as it is laid out.
#[cfg_attr(not(test), allow(dead_code))]
pub fn lod(scale: f64) -> &'static str {
    graph_viewport::lod_with(scale, GRAPH_MAP_LOD)
}
/// The room the small picture of a map may take, and its least side.
pub(super) const RADAR_ROOM: (f64, f64) = (176.0, 176.0);
pub(super) const RADAR_LEAST: f64 = 18.0;
/// How large the small picture of a map is drawn: as large as its room
/// lets a picture of that shape be.
pub fn radar_size(picture: (f64, f64)) -> (f64, f64) {
    if !(picture.0 > 0.0 && picture.1 > 0.0) {
        return (0.0, 0.0);
    }
    let by = (RADAR_ROOM.0 / picture.0).min(RADAR_ROOM.1 / picture.1);
    (
        (picture.0 * by).round().max(RADAR_LEAST),
        (picture.1 * by).round().max(RADAR_LEAST),
    )
}
/// What of a picture a window onto it shows, in the measures of the
/// picture: left, top, width and height. The window is `room` large, the
/// picture is drawn at `scale`, and its corner lies `corner` from the corner
/// of the window. `None` once the window shows all of the picture: there is
/// nowhere else to look.
pub fn in_window(
    corner: (f64, f64),
    scale: f64,
    room: (f64, f64),
    picture: (f64, f64),
) -> Option<(f64, f64, f64, f64)> {
    graph_viewport::radar_window_rect(
        graph_viewport::Camera::new(corner.0, corner.1, scale),
        room,
        picture,
    )
}
/// Where the corner of a picture lies from the corner of its window once
/// `point` of the picture is in the middle of the window.
#[cfg_attr(not(test), allow(dead_code))]
pub fn centred(point: (f64, f64), scale: f64, room: (f64, f64)) -> (f64, f64) {
    let cam = graph_viewport::center_camera_on(point, room, scale);
    (cam.tx, cam.ty)
}
/// The small picture of a map: its frames and its boxes, each box told by
/// what it is to the picture, and a frame around what the window shows,
/// which is set once the picture is measured.
pub(super) fn radar(
    picture: (f64, f64),
    frames: &[Framed],
    boxes: &[(f64, f64, &'static str)],
) -> String {
    let (width, height) = radar_size(picture);
    if width <= 0.0 || boxes.is_empty() {
        return String::new();
    }
    let mut svg = format!(
        "<svg class=\"graph-radar-picture\" width=\"{width:.0}\" height=\"{height:.0}\" viewBox=\"0 0 {:.0} {:.0}\" preserveAspectRatio=\"none\" data-w=\"{:.0}\" data-h=\"{:.0}\" focusable=\"false\">",
        picture.0, picture.1, picture.0, picture.1
    );
    for frame in frames.iter().filter(|frame| frame.width > 0.0) {
        svg.push_str(&format!(
            "<rect class=\"graph-radar-frame\" x=\"{:.0}\" y=\"{:.0}\" width=\"{:.0}\" height=\"{:.0}\"/>",
            frame.x, frame.y, frame.width, frame.height
        ));
    }
    for (x, y, state) in boxes {
        svg.push_str(&format!(
            "<rect class=\"graph-radar-box\" x=\"{x:.0}\" y=\"{y:.0}\" width=\"{BOX_WIDTH:.0}\" height=\"{BOX_HEIGHT:.0}\"{}/>",
            if state.is_empty() {
                String::new()
            } else {
                format!(" data-state=\"{state}\"")
            }
        ));
    }
    svg.push_str(
        "<rect class=\"graph-radar-view\" x=\"0\" y=\"0\" width=\"0\" height=\"0\"/></svg>",
    );
    svg
}
/// Computes the multiplicative zoom factor for a single DOM `wheel` event.
/// Returns `None` for zero or non-finite vertical deltas so pure horizontal
/// trackpad swipes pass through unintercepted.
#[cfg_attr(not(test), allow(dead_code))]
pub fn wheel_zoom_factor(delta_y: f64, delta_mode: u32) -> Option<f64> {
    graph_viewport::wheel_zoom_factor(delta_y, delta_mode)
}
/// Computes the new `(scroll_left, scroll_top)` for an overflow-scroll canvas
/// so the logical map point under `anchor` stays pinned after scaling from
/// `scale0` to `scale1`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn scroll_after_zoom(
    scroll_left0: f64,
    scroll_top0: f64,
    scale0: f64,
    scale1: f64,
    anchor: (f64, f64),
) -> (f64, f64) {
    let s0 = if scale0.is_finite() && scale0 > 0.0 {
        scale0
    } else {
        1.0
    };
    let s1 = if scale1.is_finite() && scale1 > 0.0 {
        scale1
    } else {
        1.0
    };
    let ratio = s1 / s0;
    (
        ((scroll_left0 + anchor.0) * ratio - anchor.0).max(0.0),
        ((scroll_top0 + anchor.1) * ratio - anchor.1).max(0.0),
    )
}
/// Draws, in the small picture of a map, around what the window of the map
/// shows. Positioning lives in CSS (`.graph-map-radar`), avoiding layout reflows
/// on every pan or zoom tick. A window that shows all of its picture has none:
/// there is nowhere else to look.
pub(super) fn sync_radar(canvas_id: &str, seen: Option<(f64, f64, f64, f64)>) {
    let Some(radar) = dom::get(&format!("{canvas_id}-radar")) else {
        return;
    };
    let frame = radar.query_selector(".graph-radar-view").ok().flatten();
    let (Some(seen), Some(frame)) = (seen, frame) else {
        dom::hide(&radar, true);
        return;
    };
    for (key, value) in [
        ("x", seen.0),
        ("y", seen.1),
        ("width", seen.2),
        ("height", seen.3),
    ] {
        let _ = frame.set_attribute(key, &format!("{value:.1}"));
    }
    dom::hide(&radar, false);
}
/// The small picture of a camera surface, drawn again for where its camera
/// looks.
pub(super) fn sync_surface_radar(surface: MapSurface, cam: &Camera) {
    let (width, height, measured) = viewport_size(surface.canvas);
    let seen = (measured && cam.plane_w > 0.0 && cam.plane_h > 0.0)
        .then(|| {
            in_window(
                (cam.pan_x, cam.pan_y),
                cam.scale,
                (width, height),
                (cam.plane_w, cam.plane_h),
            )
        })
        .flatten();
    sync_radar(surface.canvas, seen);
}
pub(super) fn radar_target_element(radar: &web_sys::Element) -> web_sys::Element {
    radar
        .query_selector(".graph-radar-picture")
        .ok()
        .flatten()
        .unwrap_or_else(|| radar.clone())
}
pub(super) fn radar_plane_size(canvas_id: &str, picture: &web_sys::Element) -> (f64, f64) {
    if let Some(surface) = pan_surface_for_canvas(canvas_id) {
        let cam = load_surface_camera(surface);
        if cam.plane_w > 0.0 && cam.plane_h > 0.0 {
            return (cam.plane_w, cam.plane_h);
        }
    }
    let size = |key: &str| {
        picture
            .get_attribute(key)
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| *value > 0.0)
            .unwrap_or(0.0)
    };
    (size("data-w"), size("data-h"))
}
/// Brings a point of a picture to the middle of its window.
pub(super) fn look_at(canvas_id: &str, point: (f64, f64)) {
    let Some(surface) = pan_surface_for_canvas(canvas_id) else {
        return;
    };
    let mut cam = load_surface_camera(surface);
    let (width, height, measured) = viewport_size(surface.canvas);
    if !measured {
        return;
    }
    let next = graph_viewport::center_camera_on(point, (width, height), cam.scale);
    cam.apply_viewport_cam(next);
    cam.fit_mode = false;
    cam.needs_measure = false;
    save_surface_camera(surface, &cam);
    apply_surface_camera(surface, &cam);
}
/// The small picture the pointer is held down on, by the window it is of.
pub(super) const RADAR_DRAG: &str = "graph-radar-drag";
pub(super) fn save_radar_drag(canvas_id: &str, drag: graph_viewport::RadarDrag) {
    let mut raw = drag.to_json();
    raw["canvas"] = json!(canvas_id);
    m::set(RADAR_DRAG, raw);
}
/// A press on the small picture of a map looks at what is pressed, and the
/// window follows the pointer while it is held down.
pub(super) fn radar_pointer(
    kind: &str,
    event: &web_sys::Event,
    target: Option<&web_sys::Element>,
) -> bool {
    let held = m::data(RADAR_DRAG);
    let mut drag = graph_viewport::RadarDrag::from_json(&held);
    match kind {
        "pointerdown" => {
            let Some(radar) =
                target.and_then(|target| target.closest(".graph-map-radar").ok().flatten())
            else {
                return false;
            };
            let id = radar.id();
            let Some(canvas_id) = id.strip_suffix("-radar") else {
                return true;
            };
            let picture = radar_target_element(&radar);
            let plane = radar_plane_size(canvas_id, &picture);
            match graph_viewport::handle_radar_pointer(
                kind,
                event,
                &picture,
                Some(canvas_id),
                plane,
                &mut drag,
            ) {
                graph_viewport::RadarAction::MoveTo { world_point } => {
                    save_radar_drag(canvas_id, drag);
                    look_at(canvas_id, world_point);
                }
                graph_viewport::RadarAction::Ignored => {
                    if drag.active {
                        save_radar_drag(canvas_id, drag);
                    }
                }
                graph_viewport::RadarAction::Released => {
                    m::take(RADAR_DRAG);
                }
            }
            true
        }
        "pointermove" | "pointerup" | "pointercancel" => {
            if !drag.active {
                return false;
            }
            let Some(canvas_id) = held["canvas"].as_str() else {
                m::take(RADAR_DRAG);
                return false;
            };
            let Some(radar) = dom::get(&format!("{canvas_id}-radar")) else {
                dom::attr(canvas_id, "data-panning", "false");
                m::take(RADAR_DRAG);
                return true;
            };
            let picture = radar_target_element(&radar);
            let plane = radar_plane_size(canvas_id, &picture);
            match graph_viewport::handle_radar_pointer(
                kind,
                event,
                &picture,
                Some(canvas_id),
                plane,
                &mut drag,
            ) {
                graph_viewport::RadarAction::MoveTo { world_point } => {
                    save_radar_drag(canvas_id, drag);
                    look_at(canvas_id, world_point);
                }
                graph_viewport::RadarAction::Released => {
                    m::take(RADAR_DRAG);
                }
                graph_viewport::RadarAction::Ignored => {
                    save_radar_drag(canvas_id, drag);
                }
            }
            true
        }
        _ => false,
    }
}
pub(super) fn link_plane_size() -> (f64, f64) {
    if let Some(plane) = dom::get("graph-link-canvas")
        .and_then(|c| c.query_selector(".graph-map-plane").ok().flatten())
        .and_then(|p| p.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let w = plane.offset_width() as f64;
        let h = plane.offset_height() as f64;
        if w > 0.0 && h > 0.0 {
            return (w, h);
        }
    }
    let cam = load_link_camera();
    (cam.plane_w.max(320.0), cam.plane_h.max(220.0))
}

pub(super) fn load_link_camera() -> Camera {
    let stored = m::data("graph-link-cam");
    if stored.is_object() {
        Camera {
            scale: stored["scale"].as_f64().unwrap_or(1.0),
            pan_x: stored["pan_x"].as_f64().unwrap_or(0.0),
            pan_y: stored["pan_y"].as_f64().unwrap_or(0.0),
            plane_w: stored["plane_w"].as_f64().unwrap_or(0.0),
            plane_h: stored["plane_h"].as_f64().unwrap_or(0.0),
            fit_mode: stored["fit_mode"].as_bool().unwrap_or(true),
            follow: false,
            paused: false,
            expanded: false,
            needs_measure: stored["needs_measure"].as_bool().unwrap_or(false),
            bbox: None,
        }
    } else {
        Camera::default()
    }
}

pub(super) fn save_link_camera(cam: &Camera) {
    m::set(
        "graph-link-cam",
        json!({
            "scale": cam.scale,
            "pan_x": cam.pan_x,
            "pan_y": cam.pan_y,
            "plane_w": cam.plane_w,
            "plane_h": cam.plane_h,
            "fit_mode": cam.fit_mode,
            "needs_measure": cam.needs_measure,
        }),
    );
}

pub(super) fn apply_link_camera(cam: &Camera) {
    let (pw, ph) = link_plane_size();
    let _ = graph_viewport::apply_transform_3d_by_id(
        "graph-link-canvas",
        Some("graph-link-zoom-level"),
        cam.viewport_cam(),
        (pw, ph),
        GRAPH_MAP_LOD,
    );
}

pub(super) fn fit_link_camera() {
    let mut cam = load_link_camera();
    let (pw, ph) = link_plane_size();
    let (vw, vh, measured) = viewport_size("graph-link-canvas");
    cam.plane_w = pw;
    cam.plane_h = ph;
    cam.needs_measure = !measured;
    cam.fit_mode = true;
    cam.apply_viewport_cam(graph_viewport::fit(
        (pw, ph),
        (vw, vh),
        graph_viewport::FitOptions::default(),
    ));
    save_link_camera(&cam);
    apply_link_camera(&cam);
}

pub(super) fn reset_link_camera_100() {
    let mut cam = load_link_camera();
    let (pw, ph) = link_plane_size();
    let (vw, vh, measured) = viewport_size("graph-link-canvas");
    cam.plane_w = pw;
    cam.plane_h = ph;
    cam.needs_measure = !measured;
    cam.fit_mode = false;
    cam.apply_viewport_cam(graph_viewport::reset_100((pw, ph), (vw, vh), 12.0));
    save_link_camera(&cam);
    apply_link_camera(&cam);
}

pub(super) fn zoom_link_camera_at(factor: f64, client_anchor: Option<(f64, f64)>) -> bool {
    let Some(canvas) = dom::get("graph-link-canvas") else {
        return false;
    };
    let mut cam = load_link_camera();
    let (pw, ph) = link_plane_size();
    if pw <= 0.0 || ph <= 0.0 {
        return false;
    }
    cam.plane_w = pw;
    cam.plane_h = ph;
    let (vw, vh, measured) = viewport_size("graph-link-canvas");
    cam.needs_measure = !measured;
    let anchor = match client_anchor {
        Some((cx, cy)) => {
            let rect = canvas.get_bounding_client_rect();
            (
                (cx - rect.left()).clamp(0.0, vw),
                (cy - rect.top()).clamp(0.0, vh),
            )
        }
        None => (vw / 2.0, vh / 2.0),
    };
    let next = graph_viewport::zoom_at(
        cam.viewport_cam(),
        anchor,
        cam.scale * factor,
        CAMERA_MIN,
        CAMERA_MAX,
    );
    cam.apply_viewport_cam(next);
    cam.fit_mode = false;
    save_link_camera(&cam);
    apply_link_camera(&cam);
    true
}

pub(super) fn zoom_scroll_canvas(
    canvas_id: &str,
    factor: f64,
    client_anchor: Option<(f64, f64)>,
) -> bool {
    if canvas_id == "graph-link-canvas" {
        return zoom_link_camera_at(factor, client_anchor);
    }
    false
}
pub(super) fn reset_scroll_canvas_zoom(canvas_id: &str) {
    if canvas_id == "graph-link-canvas" {
        reset_link_camera_100();
    }
}
/// Draws the modal map at the scale the viewer asked for: explicit
/// manual/wheel zoom when set, fitted to the viewport when Fit is enabled, or 100%.
pub(super) fn fit() {
    let mut cam = load_surface_camera(MODAL_SURFACE);
    if cam.plane_w <= 0.0 || cam.plane_h <= 0.0 {
        if let Some(plane) = dom::get(MODAL_SURFACE.canvas)
            .and_then(|canvas| canvas.query_selector(".graph-map-plane").ok().flatten())
            .and_then(|plane| plane.dyn_into::<web_sys::HtmlElement>().ok())
        {
            cam.plane_w = plane.offset_width() as f64;
            cam.plane_h = plane.offset_height() as f64;
        }
    }
    if cam.plane_w <= 0.0 || cam.plane_h <= 0.0 {
        return;
    }
    let st = state();
    let (vw, vh, measured) = viewport_size(MODAL_SURFACE.canvas);
    cam.needs_measure = !measured;
    if let Some(manual) = st["zoom"].as_f64().filter(|v| v.is_finite() && *v > 0.0) {
        cam.scale = manual.clamp(CAMERA_MIN, CAMERA_MAX);
        cam.fit_mode = false;
        cam.apply_viewport_cam(graph_viewport::center_camera_on(
            (cam.plane_w / 2.0, cam.plane_h / 2.0),
            (vw, vh),
            cam.scale,
        ));
        save_surface_camera(MODAL_SURFACE, &cam);
        apply_surface_camera(MODAL_SURFACE, &cam);
    } else if m::yes(&st["fit"]) {
        fit_surface_camera(MODAL_SURFACE, &mut cam);
    } else {
        cam.fit_mode = false;
        cam.apply_viewport_cam(graph_viewport::reset_100(
            (cam.plane_w, cam.plane_h),
            (vw, vh),
            12.0,
        ));
        save_surface_camera(MODAL_SURFACE, &cam);
        apply_surface_camera(MODAL_SURFACE, &cam);
    }
}
/// What the pointer rests on in a picture: a box, by its number there, or a
/// link, by the numbers of its two ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pointed {
    Place(usize),
    Link(usize, usize),
}
/// The two ends a link is marked with.
pub(super) fn ends_of(mark: &str) -> Option<(usize, usize)> {
    let (from, to) = mark.split_once(' ')?;
    Some((from.parse().ok()?, to.parse().ok()?))
}
/// What a part of a picture stands for, from its marks: the number of a
/// box, which a handle of the box carries too, or the ends of a link.
pub fn pointed(place: Option<&str>, ends: Option<&str>) -> Option<Pointed> {
    match (place, ends) {
        (Some(place), _) => place.parse().ok().map(Pointed::Place),
        (None, Some(ends)) => ends_of(ends).map(|(from, to)| Pointed::Link(from, to)),
        _ => None,
    }
}
/// Whether a link is lit while the pointer rests on something: the link
/// itself, or every link of the box.
#[cfg_attr(not(test), allow(dead_code))]
pub fn lit_link(pointed: Pointed, link: (usize, usize)) -> bool {
    match pointed {
        Pointed::Place(place) => link.0 == place || link.1 == place,
        Pointed::Link(from, to) => link == (from, to),
    }
}
/// Whether a box is lit: the box itself and what its links lead to, or the
/// two ends of the link.
#[cfg_attr(not(test), allow(dead_code))]
pub fn lit_place(pointed: Pointed, links: &[(usize, usize)], place: usize) -> bool {
    match pointed {
        Pointed::Place(at) => {
            at == place
                || links
                    .iter()
                    .any(|link| *link == (at, place) || *link == (place, at))
        }
        Pointed::Link(from, to) => place == from || place == to,
    }
}
/// Adjacency cache per canvas ID eliminating repeated full-DOM selector scans on pointermove.
struct HoverIndex {
    node_els: HashMap<usize, Vec<web_sys::Element>>,
    edge_els: HashMap<(usize, usize), Vec<web_sys::Element>>,
    adj: HashMap<usize, BTreeSet<usize>>,
    lit_els: Vec<web_sys::Element>,
}

thread_local! {
    static MAP_HOVER_CACHE: RefCell<HashMap<String, HoverIndex>> = RefCell::new(HashMap::new());
}

pub(super) fn invalidate_hover_cache(canvas_id: &str) {
    MAP_HOVER_CACHE.with(|cache| {
        cache.borrow_mut().remove(canvas_id);
    });
}

fn build_hover_index(canvas: &str) -> HoverIndex {
    let parts = dom::all(&format!(
        "#{canvas} [data-n],#{canvas} [data-of],#{canvas} [data-ends]"
    ));
    let mut node_els: HashMap<usize, Vec<web_sys::Element>> = HashMap::new();
    let mut edge_els: HashMap<(usize, usize), Vec<web_sys::Element>> = HashMap::new();
    let mut adj: HashMap<usize, BTreeSet<usize>> = HashMap::new();
    for part in parts {
        match pointed(
            part.get_attribute("data-n")
                .or_else(|| part.get_attribute("data-of"))
                .as_deref(),
            part.get_attribute("data-ends").as_deref(),
        ) {
            Some(Pointed::Place(idx)) => {
                node_els.entry(idx).or_default().push(part);
            }
            Some(Pointed::Link(from, to)) => {
                edge_els.entry((from, to)).or_default().push(part);
                adj.entry(from).or_default().insert(to);
                adj.entry(to).or_default().insert(from);
            }
            None => {}
        }
    }
    // Include cached pairs so dense Canvas2D canvases without SVG edge DOM still light neighbor nodes.
    SURFACE_PLAN_CACHE.with(|cache| {
        if let Some(entry) = cache.borrow().get(canvas) {
            for &(from, to) in &entry.pairs {
                adj.entry(from).or_default().insert(to);
                adj.entry(to).or_default().insert(from);
            }
        }
    });
    HoverIndex {
        node_els,
        edge_els,
        adj,
        lit_els: Vec::new(),
    }
}

fn repaint_dense_hover(canvas: &str, at: Option<Pointed>) {
    let (is_dense, hover_arg) = SURFACE_PLAN_CACHE.with(|cache| {
        let guard = cache.borrow();
        let Some(entry) = guard.get(canvas).filter(|e| e.is_dense) else {
            return (false, None);
        };
        let mapped = match at {
            Some(Pointed::Place(idx)) => Some((0u8, idx)),
            Some(Pointed::Link(from, to)) => entry
                .pairs
                .iter()
                .position(|&(f, t)| (f == from && t == to) || (f == to && t == from))
                .map(|ei| (1u8, ei)),
            None => None,
        };
        (true, mapped)
    });
    if is_dense {
        if let Some(surface) = pan_surface_for_canvas(canvas) {
            set_dense_hover(surface.canvas, hover_arg);
            let cam = load_surface_camera(surface);
            graph_viewport::canvas2d::paint_scene(canvas, cam.viewport_cam(), hover_arg, true);
        }
    }
}

/// Lights what the pointer rests on in a picture, with what it is linked
/// to, and leaves the rest faint: a box with its links and the boxes at
/// their other ends, a link with its two ends. Uses `MAP_HOVER_CACHE` for O(deg) updates.
pub(super) fn point(canvas: &str, target: Option<web_sys::Element>) {
    let Some(plane) =
        dom::get(canvas).and_then(|el| el.query_selector(".graph-map-plane").ok().flatten())
    else {
        return;
    };
    let hovered_part = target.and_then(|target| {
        target
            .closest("[data-n],[data-of],[data-ends]")
            .ok()
            .flatten()
    });
    let at = hovered_part.as_ref().and_then(|part| {
        pointed(
            part.get_attribute("data-n")
                .or_else(|| part.get_attribute("data-of"))
                .as_deref(),
            part.get_attribute("data-ends").as_deref(),
        )
    });
    let hovered_node_idx = match at {
        Some(Pointed::Place(idx)) => Some(idx),
        _ => None,
    };
    let node_el_opt = hovered_part.filter(|el| el.class_list().contains("graph-map-node"));
    graph_viewport::dense_dom::sync_dom_hover_attr(
        &plane,
        hovered_node_idx,
        node_el_opt.as_ref(),
        |el| {
            el.get_attribute("data-n")
                .and_then(|s| s.parse::<usize>().ok())
        },
    );
    let told = at.map(|at| format!("{at:?}")).unwrap_or_default();
    if plane.get_attribute("data-pointed").unwrap_or_default() == told {
        return;
    }
    let _ = plane.set_attribute("data-pointed", &told);

    MAP_HOVER_CACHE.with(|cache| {
        let mut map = cache.borrow_mut();
        let index = map
            .entry(canvas.to_owned())
            .or_insert_with(|| build_hover_index(canvas));
        for el in index.lit_els.drain(..) {
            if el.get_attribute("data-select-lit").as_deref() != Some("true") {
                let _ = el.remove_attribute("data-lit");
            }
        }
        let Some(at) = at else {
            let _ = plane.remove_attribute("data-hover");
            return;
        };
        let mut next_lit = Vec::new();
        match at {
            Pointed::Place(place) => {
                if let Some(els) = index.node_els.get(&place) {
                    next_lit.extend(els.iter().cloned());
                }
                if let Some(neighbors) = index.adj.get(&place) {
                    for &nbr in neighbors {
                        if let Some(els) = index.node_els.get(&nbr) {
                            next_lit.extend(els.iter().cloned());
                        }
                        if let Some(els) = index.edge_els.get(&(place, nbr)) {
                            next_lit.extend(els.iter().cloned());
                        }
                        if let Some(els) = index.edge_els.get(&(nbr, place)) {
                            next_lit.extend(els.iter().cloned());
                        }
                    }
                }
            }
            Pointed::Link(from, to) => {
                if let Some(els) = index.node_els.get(&from) {
                    next_lit.extend(els.iter().cloned());
                }
                if let Some(els) = index.node_els.get(&to) {
                    next_lit.extend(els.iter().cloned());
                }
                if let Some(els) = index.edge_els.get(&(from, to)) {
                    next_lit.extend(els.iter().cloned());
                }
            }
        }
        for el in &next_lit {
            let _ = el.set_attribute("data-lit", "true");
        }
        index.lit_els = next_lit;
        let _ = plane.set_attribute("data-hover", "true");
    });
    repaint_dense_hover(canvas, at);
}
/// Highlights all nodes belonging to one architecture layer on `canvas_id`
/// while the pointer or keyboard rests on that layer's pill in the side panel.
pub(super) fn highlight_layer(canvas_id: &str, layer: Option<&str>) {
    let Some(plane) =
        dom::get(canvas_id).and_then(|el| el.query_selector(".graph-map-plane").ok().flatten())
    else {
        return;
    };
    let layer = layer.filter(|id| !id.is_empty());
    let next_attr = layer.unwrap_or("");
    let changed = plane.get_attribute("data-layer-hover").unwrap_or_default() != next_attr;
    if graph_viewport::canvas2d::set_active_layer(canvas_id, layer) {
        if let Some(surface) = pan_surface_for_canvas(canvas_id) {
            let cam = load_surface_camera(surface);
            graph_viewport::canvas2d::paint_scene(
                canvas_id,
                cam.viewport_cam(),
                super::surfaces::current_dense_hover(canvas_id),
                true,
            );
        }
    }
    if !changed {
        return;
    }
    let nodes = dom::all(&format!("#{canvas_id} .graph-map-node[data-arch-layer-id]"));
    for node in &nodes {
        let _ = node.remove_attribute("data-layer-lit");
    }
    let Some(layer) = layer else {
        let _ = plane.remove_attribute("data-layer-hover");
        return;
    };
    let _ = plane.set_attribute("data-layer-hover", layer);
    for node in nodes {
        if node.get_attribute("data-arch-layer-id").as_deref() == Some(layer) {
            let _ = node.set_attribute("data-layer-lit", "true");
        }
    }
}
/// Spotlights the canvas node whose payload matches a hovered side-panel chip.
pub(super) fn point_part(canvas_id: &str, path: Option<&str>) {
    let target = path.and_then(|key| {
        let quoted = json!(key).to_string();
        dom::all(&format!("#{canvas_id} .graph-map-node"))
            .into_iter()
            .find(|node| {
                let raw = node.get_attribute("data-payload").unwrap_or_default();
                raw == key || raw == quoted
            })
    });
    if target.is_some() {
        point(canvas_id, target);
        return;
    }
    let dense_idx = path.and_then(|key| {
        SURFACE_PLAN_CACHE.with(|cache| {
            let guard = cache.borrow();
            let entry = guard.get(canvas_id).filter(|e| e.is_dense)?;
            entry.node_keys.iter().position(|k| k == key)
        })
    });
    if let Some(idx) = dense_idx {
        repaint_dense_hover(canvas_id, Some(Pointed::Place(idx)));
    } else {
        point(canvas_id, None);
    }
}
pub(super) fn attach_canvas_hover(canvas_id: &'static str) {
    let Some(element) = dom::get(canvas_id) else {
        return;
    };
    if element.get_attribute("data-hover-bound").as_deref() == Some("true") {
        return;
    }
    let _ = element.set_attribute("data-hover-bound", "true");
    // Entering a part lights it; leaving the picture puts it out.
    for (kind, rests) in [
        ("mouseover", true),
        ("focusin", true),
        ("mouseleave", false),
        ("pointerleave", false),
        ("pointercancel", false),
        ("focusout", false),
    ] {
        let moved = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            let target = event
                .target()
                .and_then(|target| target.dyn_into::<web_sys::Element>().ok());
            point(canvas_id, target.filter(|_| rests));
            if !rests {
                if let Some(canvas_el) =
                    dom::get(canvas_id).and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
                {
                    let _ = canvas_el.style().set_property("cursor", "");
                    let _ = canvas_el.remove_attribute("title");
                }
                if super::surfaces::set_dense_hover(canvas_id, None) {
                    if let Some(surface) = pan_surface_for_canvas(canvas_id) {
                        let cam = load_surface_camera(surface);
                        graph_viewport::canvas2d::paint_scene(
                            canvas_id,
                            cam.viewport_cam(),
                            None,
                            true,
                        );
                    }
                }
            }
        });
        let _ = element.add_event_listener_with_callback(kind, moved.as_ref().unchecked_ref());
        moved.forget();
    }
}
pub(super) fn attach_side_hover(side_id: &'static str, canvas_id: &'static str) {
    let Some(side_panel) = dom::get(side_id) else {
        return;
    };
    if side_panel.get_attribute("data-hover-bound").as_deref() == Some("true") {
        return;
    }
    let _ = side_panel.set_attribute("data-hover-bound", "true");
    for (kind, rests) in [
        ("mouseover", true),
        ("focusin", true),
        ("mouseleave", false),
        ("focusout", false),
    ] {
        let moved = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            let target = rests
                .then(|| {
                    event
                        .target()
                        .and_then(|el| el.dyn_into::<web_sys::Element>().ok())
                })
                .flatten();
            let layer = target
                .as_ref()
                .and_then(|el| el.closest("[data-arch-layer]").ok().flatten())
                .and_then(|el| el.get_attribute("data-arch-layer"));
            highlight_layer(canvas_id, layer.as_deref());
            let chip = target
                .as_ref()
                .and_then(|el| {
                    el.closest(".graph-map-part-chip[data-action='graph-map-node']")
                        .ok()
                        .flatten()
                })
                .and_then(|el| el.get_attribute("data-payload"));
            point_part(canvas_id, chip.as_deref());
        });
        let _ = side_panel.add_event_listener_with_callback(kind, moved.as_ref().unchecked_ref());
        moved.forget();
    }
}
pub(super) fn ensure_resize_watch() {
    graph_viewport::ensure_resize_observer(
        "graph-map-resize-watched",
        Some("graph-map-resize"),
        60,
        || {
            if open() {
                let cam = load_surface_camera(MODAL_SURFACE);
                let (vw, vh, measured) = viewport_size(MODAL_SURFACE.canvas);
                match graph_viewport::resize_refit_decision(
                    cam.viewport_cam(),
                    cam.fit_mode || cam.needs_measure,
                    (cam.plane_w, cam.plane_h),
                    (vw, vh),
                    measured,
                    graph_viewport::FitOptions::default(),
                ) {
                    graph_viewport::ResizeAction::Skip => {}
                    graph_viewport::ResizeAction::Refit(_) => fit(),
                    graph_viewport::ResizeAction::SyncRadar(_) => {
                        sync_surface_radar(MODAL_SURFACE, &cam);
                    }
                }
            }
            for surface in [
                SESSION_SURFACE,
                SYSTEM_EMBEDDED_SURFACE,
                SYSTEM_CHANGE_SURFACE,
            ] {
                refit_surface_if_visible(surface);
                // A window of another size shows another part of its picture,
                // also where the picture is left as it was.
                let (_, _, measured) = viewport_size(surface.canvas);
                if measured {
                    sync_surface_radar(surface, &load_surface_camera(surface));
                }
            }
        },
    );
}
pub(super) fn pan_surface_for_canvas(canvas_id: &str) -> Option<MapSurface> {
    match canvas_id {
        "graph-map-canvas" => Some(MODAL_SURFACE),
        "session-graph-canvas" => Some(SESSION_SURFACE),
        "system-member-graph-canvas" => Some(SYSTEM_EMBEDDED_SURFACE),
        "system-change-graph-canvas" => Some(SYSTEM_CHANGE_SURFACE),
        _ => None,
    }
}
pub(super) fn attach_canvas_wheel(canvas_id: &'static str) {
    let timer_key = pan_surface_for_canvas(canvas_id)
        .map(|s| s.wheel_timer_key.to_owned())
        .unwrap_or_else(|| format!("{canvas_id}-wheel"));
    graph_viewport::attach_wheel_zoom(canvas_id, &timer_key, move |factor, client| {
        if let Some(surface) = pan_surface_for_canvas(canvas_id) {
            let cam = load_surface_camera(surface);
            if cam.plane_w <= 0.0 || cam.plane_h <= 0.0 {
                return;
            }
            let anchor = dom::get(surface.canvas).map(|canvas| {
                let rect = canvas.get_bounding_client_rect();
                (client.0 - rect.left(), client.1 - rect.top())
            });
            zoom_surface_camera_at(surface, factor, anchor);
        } else {
            let _ = zoom_scroll_canvas(canvas_id, factor, Some(client));
        }
    });
}
pub fn attach_wheel_listeners() {
    thread_local! {
        static GRAPH_HOOKS_WIRED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    attach_canvas_wheel("session-graph-canvas");
    attach_canvas_wheel("graph-map-canvas");
    attach_canvas_wheel("graph-link-canvas");
    attach_system_wheel_listeners();
    if !GRAPH_HOOKS_WIRED.with(|wired| wired.replace(true)) {
        crate::api::on_sse("graph_status", crate::graph::on_graph_status_sse);
        crate::api::on_sse("resync", crate::graph::on_graph_resync_sse);
        crate::session::register_failure_localizer(crate::graph_map::localize_failure_request);
    }
}
/// Attaches non-passive cursor-anchored wheel zoom + node/side hover listeners to the embedded
/// System code-graph surfaces (`#system-member-graph-canvas`, `#system-change-graph-canvas`).
#[allow(dead_code)]
pub(crate) fn attach_system_wheel_listeners() {
    ensure_resize_watch();
    attach_canvas_wheel("system-member-graph-canvas");
    attach_canvas_hover("system-member-graph-canvas");
    attach_side_hover("system-member-graph-side", "system-member-graph-canvas");
    attach_legend_hover(
        "system-member-graph-arrow-legend",
        "system-member-graph-canvas",
    );
    attach_canvas_wheel("system-change-graph-canvas");
    attach_canvas_hover("system-change-graph-canvas");
    attach_side_hover("system-change-graph-side", "system-change-graph-canvas");
    attach_legend_hover(
        "system-change-graph-arrow-legend",
        "system-change-graph-canvas",
    );
}
pub(super) fn ensure_surface_watch(surface: MapSurface) {
    attach_wheel_listeners();
    ensure_resize_watch();
    attach_canvas_wheel(surface.canvas);
    attach_canvas_hover(surface.canvas);
    attach_side_hover(surface.side, surface.canvas);
    attach_legend_hover(
        &format!("{}-arrow-legend", surface.id_prefix),
        surface.canvas,
    );
}
/// Fits the picture again when the window changes its size, lights what
/// the pointer or the keyboard rests on in the map and in the window of a
/// link, and closes what a search found when the keyboard leaves the
/// search. Asked for each time the map opens, and set up once.
pub(super) fn watch() {
    attach_wheel_listeners();
    if m::yes(&m::data("graph-map-watched")) {
        return;
    }
    m::set("graph-map-watched", json!(true));
    ensure_resize_watch();
    attach_canvas_hover("graph-map-canvas");
    attach_canvas_hover("graph-link-canvas");
    attach_side_hover("graph-map-side", "graph-map-canvas");
    attach_legend_hover("graph-map-arrow-legend", "graph-map-canvas");
    attach_legend_hover("graph-link-arrow-legend", "graph-link-canvas");
    // What a search found is closed once the keyboard leaves the search;
    // moving on into the list keeps it open.
    if let Some(search) = dom::get("graph-find") {
        let within = search.clone();
        let left = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            let stays = js_sys::Reflect::get(&event, &"relatedTarget".into())
                .ok()
                .and_then(|next| next.dyn_into::<web_sys::Node>().ok())
                .is_some_and(|next| within.contains(Some(&next)));
            if !stays {
                close_found();
            }
        });
        let _ = search.add_event_listener_with_callback("focusout", left.as_ref().unchecked_ref());
        left.forget();
    }
}
pub(super) fn fresh(target: &str) -> V {
    json!({"target": target, "scope": "", "focus": "", "certain": false, "notests": false, "fit": false, "whole": false, "side_tab": "details", "changes_filter": "all", "member_filter": "all", "side_pinned": false, "filter": "", "follow": "", "out": [], "into": [], "spread": [], "path": [], "shared": false, "side": "", "skip": [], "pick": null, "patterns": false, "pattern": null, "pattern_return": null, "kinds": []})
}
/// How many folders a picture keeps spread out; the oldest gives way.
pub(super) const SPREAD: usize = 32;
/// The state with a folder spread out in the picture, or, with `gather`,
/// drawn as one box again together with whatever was spread out inside it.
pub(super) fn spread(mut state: V, folder: &str, gather: bool) -> V {
    let inside = format!("{folder}/");
    let mut folders = unfolded(&state, "spread");
    folders.retain(|other| other != folder && !(gather && other.starts_with(&inside)));
    if !gather {
        folders.push(folder.to_owned());
        let extra = folders.len().saturating_sub(SPREAD);
        folders.drain(..extra);
        // The folder is no place to select any more.
        if state["focus"] == folder {
            state["focus"] = json!("");
        }
    }
    state["spread"] = json!(folders);
    state
}
/// Where this browser keeps whether the map leaves tests out, and whether
/// it fits its pictures to the window.
pub(super) const NOTESTS: &str = "graph-map-notests";
pub(super) const FIT: &str = "graph-map-fit";
/// And whether it marks its pictures with the patterns found in them.
pub(super) const PATTERNS: &str = "graph-map-patterns";
/// And the kinds of links its key has switched off, their names joined by
/// commas. One choice holds for every picture of links.
pub(super) const LINKS_OFF: &str = "graph-map-links-off";
/// And whether the links that longer ways already join are left out.
pub(super) const HIDE_TRANSITIVE: &str = "graph-map-hide-transitive";
/// What the keys of the pictures hold in common, as they left it, or, the
/// first time it is asked for, as this browser kept it.
pub(super) fn key_store() -> V {
    let held = m::data(LINKS_OFF);
    if held.is_object() {
        return held;
    }
    let kept = dom::storage(LINKS_OFF).unwrap_or_default();
    let held = json!({
        "links_off": kept.split(',').filter(|kind| !kind.is_empty()).collect::<Vec<_>>(),
        "hide_transitive": dom::storage(HIDE_TRANSITIVE).as_deref() == Some("1"),
    });
    m::set(LINKS_OFF, held.clone());
    held
}
/// The kinds of links switched off now.
pub(super) fn links_off_now() -> KindsOff {
    kinds_off(&key_store())
}
/// What the key of a picture is set to: what all pictures share, and the
/// reach its own state holds.
pub(super) fn key_now(state: &V) -> KeyState {
    let held = key_store();
    KeyState {
        off: kinds_off(&held),
        hide_transitive: m::yes(&held["hide_transitive"]),
        cone: cone_of(state),
    }
}
/// A kind of link, or a preset of them, was selected in the key of a
/// picture: the choice is kept, and the picture that key belongs to is
/// drawn again from what it holds. The server is asked nothing.
pub(super) fn switch_kinds(action: &str, asked: &V) {
    let held = key_store();
    let surface = m::text(asked, "surface");
    match action {
        // How far the links of the selected box are lit is a choice of
        // the one picture.
        "graph-cone" => {
            if surface == "link" {
                return;
            }
            let surface = surface_by_name(&surface);
            let next = reaching(
                surface_state(surface),
                &m::text(asked, "side"),
                asked["hops"].as_u64().unwrap_or(1),
            );
            save_surface_state(surface, &next);
        }
        "graph-transitive-toggle" => {
            let mut next = held;
            let hide = !m::yes(&next["hide_transitive"]);
            next["hide_transitive"] = json!(hide);
            dom::save(HIDE_TRANSITIVE, hide.then_some("1"));
            m::set(LINKS_OFF, next);
        }
        _ => {
            let mut next = if action == "graph-kind-preset" {
                preset_kinds(held.clone(), &m::text(asked, "preset"))
            } else {
                switched_kind(
                    held.clone(),
                    &m::text(asked, "kind"),
                    m::yes(&m::data("pointer-mods")["alt"]),
                )
            };
            next["hide_transitive"] = held["hide_transitive"].clone();
            let kept = names_off(&kinds_off(&next)).join(",");
            dom::save(LINKS_OFF, (!kept.is_empty()).then_some(kept.as_str()));
            m::set(LINKS_OFF, next);
        }
    }
    let legend = if surface == "link" {
        draw_link();
        "graph-link-arrow-legend".to_owned()
    } else {
        let surface = surface_by_name(&surface);
        rerender_surface_preserving_camera(surface);
        format!("{}-arrow-legend", surface.id_prefix)
    };
    // The key was drawn again: the keyboard goes back to what was selected.
    let wanted = asked.to_string();
    if let Some(button) = dom::all(&format!("#{legend} [data-action='{action}']"))
        .into_iter()
        .find(|button| button.get_attribute("data-payload").as_deref() == Some(wanted.as_str()))
        .and_then(|button| button.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = button.focus();
    }
}
/// Resting on a kind in the key of a picture lights the links that hold
/// it, and leaving the key puts them back: a mark on the canvas the
/// stylesheet reads, so nothing is drawn again.
pub(super) fn attach_legend_hover(legend_id: &str, canvas_id: &str) {
    let Some(legend) = dom::get(legend_id) else {
        return;
    };
    if legend.get_attribute("data-hover-bound").as_deref() == Some("true") {
        return;
    }
    let _ = legend.set_attribute("data-hover-bound", "true");
    for (kind, rests) in [
        ("mouseover", true),
        ("focusin", true),
        ("mouseleave", false),
        ("focusout", false),
    ] {
        let canvas_id = canvas_id.to_owned();
        let moved = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            let held = rests
                .then(|| {
                    event
                        .target()
                        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                })
                .flatten()
                .and_then(|target| {
                    target
                        .closest(".graph-arrow-legend-item[aria-pressed='true']")
                        .ok()
                        .flatten()
                })
                .and_then(|item| item.get_attribute("data-edge-kind"));
            let Some(canvas) = dom::get(&canvas_id) else {
                return;
            };
            let _ = match &held {
                Some(kind) => canvas.set_attribute("data-kind-hover", kind),
                None => canvas.remove_attribute("data-kind-hover"),
            };
            graph_viewport::canvas2d::set_kind_hover(&canvas_id, held.as_deref());
            let is_dense = SURFACE_PLAN_CACHE.with(|cache| {
                cache
                    .borrow()
                    .get(canvas_id.as_str())
                    .is_some_and(|e| e.is_dense)
            });
            if is_dense {
                if let Some(surface) = pan_surface_for_canvas(&canvas_id) {
                    let cam = load_surface_camera(surface);
                    graph_viewport::canvas2d::paint_scene(
                        &canvas_id,
                        cam.viewport_cam(),
                        None,
                        true,
                    );
                }
            }
        });
        let _ = legend.add_event_listener_with_callback(kind, moved.as_ref().unchecked_ref());
        moved.forget();
    }
}
/// A map begun anew, with tests left out and pictures fitted where they
/// were the last time.
pub(super) fn anew(target: &str) -> V {
    let mut state = fresh(target);
    state["notests"] = json!(dom::storage(NOTESTS).as_deref() == Some("1"));
    state["fit"] = json!(dom::storage(FIT).as_deref() == Some("1"));
    state["patterns"] = json!(dom::storage(PATTERNS).as_deref() == Some("1"));
    state["history"] = json!(m::yes(&self::state()["history"]));
    state["modules"] = json!(m::yes(&self::state()["modules"]));
    state["no_incoming"] = json!(m::yes(&self::state()["no_incoming"]));
    state
}
/// The places a list of the state names: those whose side `key` (`out` or
/// `into`) is unfolded among followed links, each once for every step
/// unfolded, or the folders spread out in a picture (`spread`).
pub(super) fn unfolded(state: &V, key: &str) -> Vec<String> {
    m::array(&state[key])
        .iter()
        .filter_map(|place| place.as_str().map(str::to_owned))
        .collect()
}
/// Leave any followed links, path, shared places, cycle or found pattern. A
/// path that is still being asked for stays asked for.
pub(super) fn unfollowed(mut state: V) -> V {
    state["pattern"] = V::Null;
    state["pattern_return"] = V::Null;
    state["follow"] = json!("");
    state["out"] = json!([]);
    state["into"] = json!([]);
    state["path"] = json!([]);
    state["test_path_return"] = V::Null;
    state["scenario"] = V::Null;
    state["scenario_return"] = V::Null;
    state["scenario_skip"] = json!(0);
    state["cycle"] = V::Null;
    state["cycle_return"] = V::Null;
    state["cycle_base"] = V::Null;
    state["shared"] = json!(false);
    state["side"] = json!("");
    state["skip"] = json!([]);
    state
}
/// Open a cycle with one return point, keeping the original map as requested.
pub(super) fn showing_cycle(state: V, place: &str) -> Option<V> {
    if place.is_empty() || state["cycle"].is_string() || state["pick"].is_object() {
        return None;
    }
    let mut cycle = unfollowed(state.clone());
    cycle["cycle"] = json!(place);
    cycle["focus"] = json!(place);
    cycle["filter"] = json!("");
    cycle["cycle_return"] = state;
    Some(cycle)
}
pub(super) fn switch_cycle(mut state: V, comparison: &V) -> Option<V> {
    state["cycle"].as_str()?;
    if comparison.is_null() {
        if state["cycle_base"].is_object() {
            let base = state["cycle_base"].take();
            state["focus"] = base["head_focus"].clone();
        }
        return Some(state);
    }
    let place = comparison["place"].as_str()?;
    let revision = comparison["revision"].as_str()?;
    if state["cycle_base"].is_object() {
        return None;
    }
    state["cycle_base"] = json!({
        "place": place,
        "revision": revision,
        "head_focus": state["focus"],
    });
    state["focus"] = json!(place);
    Some(state)
}
/// The map as it shows one found pattern, with the map it was found in to
/// return to. A pattern opened from the picture of another returns where the
/// first was opened. `None` while the other end of a path is being chosen.
pub fn showing_pattern(state: V, id: &str) -> Option<V> {
    if id.is_empty() || state["pick"].is_object() {
        return None;
    }
    let back = if state["pattern"].is_string() {
        state["pattern_return"].clone()
    } else {
        state.clone()
    };
    let mut shown = unfollowed(state);
    shown["pattern"] = json!(id);
    shown["pattern_return"] = back;
    shown["focus"] = json!("");
    shown["filter"] = json!("");
    Some(shown)
}
/// The map with a link between two layers selected, where the layers of a
/// folder are drawn and the link is one between two of them; selected again,
/// it is let go of. `None` for any other link: that one is looked into.
pub fn between(mut state: V, from: &str, to: &str) -> Option<V> {
    let layers = state["pattern"]
        .as_str()
        .is_some_and(|pattern| pattern.starts_with("layers@"));
    if !layers || !from.starts_with("layer:") || !to.starts_with("layer:") {
        return None;
    }
    let link = format!("{from}>{to}");
    state["focus"] = json!(if state["focus"] == link.as_str() {
        ""
    } else {
        link.as_str()
    });
    Some(state)
}
/// The map a found pattern was opened from, with what was switched since:
/// those choices are the viewer's and hold wherever the map goes.
pub fn left_pattern(mut state: V) -> Option<V> {
    let mut previous = state["pattern_return"].take();
    if !previous.is_object() {
        return None;
    }
    for key in ["certain", "notests", "fit", "patterns", "kinds"] {
        previous[key] = state[key].take();
    }
    Some(previous)
}
pub(super) const PATTERN_BACK: &str = "<button type=\"button\" class=\"button compact\" id=\"graph-map-pattern-back\" data-action=\"graph-map-pattern-back\">Back to map</button>";
pub(super) const CYCLE_BACK: &str = "<button type=\"button\" class=\"button compact\" id=\"graph-map-cycle-back\" data-action=\"graph-map-cycle-back\">Back to map</button>";

pub(super) fn base_cycle_button(view: &V, showing_base: bool) -> String {
    let comparison = &view["cycle"]["comparison"];
    let (label, comparison) = if showing_base {
        ("Back to PR cycle", &V::Null)
    } else if comparison["state"] == "ready"
        && comparison["origin"] == "present"
        && comparison["place"].is_string()
        && comparison["revision"].is_string()
    {
        ("Show base cycle", comparison)
    } else {
        return String::new();
    };
    format!(
        "<button type=\"button\" class=\"button compact\" id=\"graph-map-cycle-switch\" data-action=\"graph-map-cycle-switch\" data-payload=\"{}\">{label}</button>",
        payload(comparison),
    )
}
/// The map as it answers another question of the two places it shows:
/// `true` asks what they share, a kind of that by its name (`both_use`,
/// `uses_both`) asks for that kind, and anything else for the path between
/// them again. Either kind begins with its nearest places; a filter holds
/// for both kinds, and is dropped with them. `None` where no two places are
/// shown.
pub fn sharing(mut state: V, asked: &V) -> Option<V> {
    if unfolded(&state, "path").len() != 2 {
        return None;
    }
    let side = asked
        .as_str()
        .filter(|side| ["both_use", "uses_both"].contains(side))
        .unwrap_or("");
    let shared = asked == &json!(true) || !side.is_empty();
    state["shared"] = json!(shared);
    state["side"] = json!(side);
    state["skip"] = json!([]);
    state["focus"] = json!("");
    if !shared {
        state["filter"] = json!("");
    }
    Some(state)
}
/// After how many of the nearest places each picture of what two places
/// share began, on the way to the one that is shown: none for the first.
pub(super) fn begun(state: &V) -> Vec<u64> {
    m::array(&state["skip"])
        .iter()
        .filter_map(V::as_u64)
        .collect()
}
/// The map as it shows other places that two places share: those after
/// `next` of the nearest, or, with none, those of the picture before.
/// Nothing stays selected.
pub fn paged(mut state: V, next: Option<u64>) -> V {
    let mut starts = begun(&state);
    match next {
        Some(next) if next > starts.last().copied().unwrap_or(0) => starts.push(next),
        Some(_) => {}
        None => {
            starts.pop();
        }
    }
    state["skip"] = json!(starts);
    state["focus"] = json!("");
    state
}
/// The state once the server said where the picture begins: it begins
/// where it can, which is earlier where places went since.
pub fn begins(mut state: V, at: u64) -> V {
    let mut starts = begun(&state);
    starts.retain(|start| *start < at);
    if at > 0 {
        starts.push(at);
    }
    state["skip"] = json!(starts);
    state
}
/// The map as it shows the shortest ways from the place a path is being
/// asked from to the place `to`. `None` where no path is being asked for,
/// or `to` is that place itself.
pub fn led(state: V, to: &str) -> Option<V> {
    let from = state["pick"]["path"].as_str()?.to_owned();
    if from.is_empty() || to.is_empty() || from == to {
        return None;
    }
    let mut state = unfollowed(state);
    state["path"] = json!([from, to]);
    state["pick"] = V::Null;
    state["focus"] = json!("");
    state["filter"] = json!("");
    Some(state)
}
/// What stands above the side panel where two places share more than one
/// picture shows: which of the places, in the order of nearness, this one
/// shows, and the ways to the ones before and to the ones after. Nothing
/// where one picture shows them all.
pub fn paging(view: &V) -> String {
    if !shares(view) {
        return String::new();
    }
    let shared = &view["shared"];
    let number = |key: &str| shared[key].as_u64().unwrap_or(0);
    let (matching, skip, shown) = (number("matching"), number("skip"), number("shown"));
    let more = skip + shown < matching;
    if shown == 0 || (skip == 0 && !more) {
        return String::new();
    }
    let all = number(if above(view) { "uses_both" } else { "both_use" });
    // A way that leads nowhere stays where it is, and cannot be taken.
    let turn = |label: &str, to: V, open: bool| {
        format!(
            "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-shared-page\" data-payload=\"{}\"{}>{label}</button>",
            payload(&to),
            if open { "" } else { " disabled" }
        )
    };
    format!(
        "<div class=\"graph-map-paging\"><p>{} of {}{}, the nearest first.</p>{}{}</div>",
        if shown == 1 {
            format!("Place {}", count(skip + 1))
        } else {
            format!("Places {} to {}", count(skip + 1), count(skip + shown))
        },
        count(matching),
        if matching < all {
            " that match the filter"
        } else {
            ""
        },
        turn("Nearer", V::Null, skip > 0),
        turn("Further", json!(skip + shown), more),
    )
}
/// What stands above the side panel while the other end of a path is being
/// chosen: where the path leads from, what to do, and the way out.
pub fn choosing(state: &V) -> String {
    let pick = &state["pick"];
    if !pick.is_object() {
        return String::new();
    }
    format!(
        "<div class=\"graph-map-picking\" role=\"status\"><p>Path from <strong>{}</strong>: select the place it should lead to, a box of any picture or a place the search finds.</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-path-cancel\">Cancel</button></div>",
        escape(&m::text(pick, "name"))
    )
}
pub(super) fn normalize_map_state(mut state: V) -> V {
    if !state.is_object() {
        return fresh("");
    }
    if !state["whole"].is_boolean() {
        state["whole"] = json!(false);
    }
    if !matches!(
        state["side_tab"].as_str(),
        Some("details" | "changes" | "analysis")
    ) {
        state["side_tab"] = json!("details");
    }
    if !matches!(
        state["changes_filter"].as_str(),
        Some("all" | "created" | "modified" | "deleted")
    ) {
        state["changes_filter"] = json!("all");
    }
    if !matches!(
        state["member_filter"].as_str(),
        Some("all" | "used" | "exposed" | "risk")
    ) {
        state["member_filter"] = json!("all");
    }
    if !state["side_pinned"].is_boolean() {
        state["side_pinned"] = json!(false);
    }
    state
}
pub(super) fn state() -> V {
    let state = m::data("graph-map");
    if state.is_object() {
        normalize_map_state(state)
    } else {
        fresh("")
    }
}
/// The link the window shows: its two ends, the name chosen in it, and
/// what the server said of it.
pub(super) fn looked() -> V {
    let link = m::data("graph-link");
    if link.is_object() {
        link
    } else {
        json!({"from": "", "to": "", "chosen": "", "view": null, "surface": "modal"})
    }
}
/// Draws the window of the link as it stands.
pub(super) fn draw_link() {
    let link = looked();
    let view = &link["view"];
    if !view.is_object() {
        return;
    }
    let chosen = m::text(&link, "chosen");
    let surface = surface_by_name(link["surface"].as_str().unwrap_or("modal"));
    let st = surface_state(surface);
    let scope = surface_scope(surface);
    let view_cached = m::data(surface.view_key);
    let impact = cached_impact_for_selection(
        surface,
        &st,
        view_cached.is_object().then_some(&view_cached),
    );
    crate::graph_explanation::scoped_link_context(view, &st["cycle_base"]["revision"], &scope);
    dom::text("graph-link-title", &link_title(view));
    dom::text("graph-link-summary", &link_summary(view));
    dom::text("graph-link-note", &link_note(view));
    html(
        "graph-link-canvas",
        &link_canvas_with(view, &chosen, impact.as_ref(), &links_off_now()),
    );
    invalidate_hover_cache("graph-link-canvas");
    fit_link_camera();
    html(
        "graph-link-side",
        &link_side_with(view, &chosen, impact.as_ref()),
    );
    html(
        "graph-link-arrow-legend",
        &kinds_legend_html(view, &links_off_now(), "link"),
    );
}
#[allow(dead_code)]
pub(crate) fn link_request(project: &str, from: &str, to: &str, selection: &V) -> String {
    build_scoped_link_request(&GraphTargetScope::single(project), from, to, selection)
}

#[allow(dead_code)]
pub(crate) fn graph_request(project: &str, query: Vec<(&str, String)>, selection: &V) -> String {
    build_scoped_graph_request(&GraphTargetScope::single(project), query, selection)
}

pub(crate) fn build_scoped_link_request(
    scope: &GraphTargetScope,
    from: &str,
    to: &str,
    selection: &V,
) -> String {
    build_scoped_graph_request(
        scope,
        vec![("from", from.to_owned()), ("to", to.to_owned())],
        selection,
    )
}

pub(crate) fn build_scoped_graph_request(
    scope: &GraphTargetScope,
    mut query: Vec<(&str, String)>,
    selection: &V,
) -> String {
    let target = effective_target(selection);
    if !target.is_empty() {
        query.push(("target", target.clone()));
    }
    for key in ["launch", "base"] {
        if let Some(value) = selection[key].as_str().filter(|value| !value.is_empty()) {
            query.push((key, value.to_owned()));
        }
    }
    if scope.is_system() {
        if let Some(rid) = effective_sysrun_id(scope, &target, selection["run_id"].as_str()) {
            query.push(("run_id", rid));
        }
        if let Some(agent) = selection["agent"].as_str().filter(|a| !a.is_empty()) {
            query.push(("agent", agent.to_owned()));
        }
    }
    for key in ["certain", "notests", "whole"] {
        if m::yes(&selection[key]) {
            query.push((key, "1".to_owned()));
        }
    }
    format_scoped_endpoint(scope, "map", query)
}
/// Opens the window of one link of the map and reads what makes the link
/// up, on the graph the map is read from. The map stays as it is behind it.
pub(super) async fn look_from(surface: MapSurface, from: &str, to: &str) {
    let state = surface_state(surface);
    let scope = surface_scope(surface);
    if scope.is_empty() || from.is_empty() || to.is_empty() {
        return;
    }
    let scope_key = scope.scope_key();
    m::set(
        "graph-link",
        json!({"from": from, "to": to, "chosen": "", "view": null, "surface": surface.name, "zoom": null}),
    );
    dom::text("graph-link-zoom-level", "100%");
    dom::text("graph-link-title", "Link");
    dom::text("graph-link-summary", "");
    dom::text("graph-link-note", "Reading the link…");
    dom::error("graph-link-error", "");
    html("graph-link-canvas", "");
    html("graph-link-side", "");
    html("graph-link-arrow-legend", "");
    dom::dialog("graph-link", true);
    attach_wheel_listeners();
    let path = build_scoped_link_request(
        &scope,
        from,
        to,
        &json!({
            "target": effective_target(&state), "launch": state["launch"],
            "agent": state["agent"],
            "base": state["cycle_base"]["revision"],
            "certain": state["certain"], "notests": state["notests"],
            "whole": state["whole"],
        }),
    );
    match api::scoped("graph-link", &path).await {
        Ok((epoch, view)) => {
            if !m::current("graph-link", epoch) || surface_scope(surface).scope_key() != scope_key {
                return;
            }
            let mut link = looked();
            link["view"] = view;
            m::set("graph-link", link);
            draw_link();
        }
        Err(error) if error.aborted => {}
        Err(error) => {
            dom::text("graph-link-note", "");
            dom::error("graph-link-error", &error.message);
        }
    }
}
#[allow(dead_code)]
pub(super) async fn look(from: &str, to: &str) {
    look_from(MODAL_SURFACE, from, to).await;
}
/// The search of the project as it stands: what is typed, what the server
/// found, for which words and which request, which row is marked, and
/// whether the list is open.
pub(super) fn finder() -> V {
    let finder = m::data("graph-find");
    if finder.is_object() {
        finder
    } else {
        json!({"open": false, "query": "", "asked": "", "for": "", "active": 0, "rows": [], "found": 0, "loading": false, "failed": null, "missing": false, "notests": false})
    }
}
pub fn build_scoped_find_path(
    scope: &GraphTargetScope,
    state: &V,
    raw_query: &str,
) -> Option<String> {
    let query = raw_query.trim().to_owned();
    if scope.is_empty() || query.is_empty() || state["cycle_base"].is_object() {
        return None;
    }
    let mut asked = vec![("query", query)];
    let target = effective_target(state);
    if !target.is_empty() {
        asked.push(("target", target.clone()));
    }
    if scope.is_system() {
        if let Some(rid) = effective_sysrun_id(scope, &target, None) {
            asked.push(("run_id", rid));
        }
    }
    if m::yes(&state["notests"]) {
        asked.push(("notests", "1".to_owned()));
    }
    Some(format_scoped_endpoint(scope, "find", asked))
}
/// The request that searches for what is typed in the graph the map is
/// drawn from; none where nothing is typed.
pub(super) fn find_path(finder: &V) -> Option<String> {
    let state = state();
    let scope = surface_scope(MODAL_SURFACE);
    build_scoped_find_path(&scope, &state, &m::text(finder, "query"))
}
/// Whether what is listed answers what is typed now, in the graph the map
/// is drawn from now.
pub(super) fn answered(finder: &V) -> bool {
    find_path(finder).is_some_and(|path| finder["for"] == path.as_str())
}
/// Draws the search of the project as it stands.
pub(super) fn show_found() {
    let finder = finder();
    let opened = m::yes(&finder["open"]);
    dom::hidden("graph-find-found", !opened);
    dom::attr(
        "graph-find-search",
        "aria-expanded",
        if opened { "true" } else { "false" },
    );
    if let Some(field) = dom::get("graph-find-search") {
        // The field keeps the keyboard and says which row is marked.
        let _ = if opened && !m::array(&finder["rows"]).is_empty() {
            field.set_attribute(
                "aria-activedescendant",
                &format!("graph-find-row-{}", finder["active"].as_u64().unwrap_or(0)),
            )
        } else {
            field.remove_attribute("aria-activedescendant")
        };
    }
    if opened {
        let mut finder = finder;
        finder["choosing"] = json!(state()["pick"].is_object());
        html("graph-find-list", &found(&finder));
        dom::text("graph-find-note", &found_note(&finder));
    }
}
/// Moves the mark of the search to a row and brings the row into view.
pub(super) fn mark_found(next: usize) {
    let mut finder = finder();
    let before = finder["active"].as_u64().unwrap_or(0);
    finder["active"] = json!(next);
    m::set("graph-find", finder);
    dom::attr(
        &format!("graph-find-row-{before}"),
        "aria-selected",
        "false",
    );
    dom::attr(&format!("graph-find-row-{next}"), "aria-selected", "true");
    dom::attr(
        "graph-find-search",
        "aria-activedescendant",
        &format!("graph-find-row-{next}"),
    );
    into_view("graph-find-list", &format!("graph-find-row-{next}"));
}
/// Closes the list of the search; what was typed and found stays.
pub(super) fn close_found() {
    let mut finder = finder();
    if !m::yes(&finder["open"]) {
        return;
    }
    finder["open"] = json!(false);
    m::set("graph-find", finder);
    show_found();
}
/// Asks the server for what is typed, in the graph the map is drawn from.
pub(super) async fn seek() {
    let finder = finder();
    let Some(path) = find_path(&finder) else {
        return;
    };
    let scope_key = surface_scope(MODAL_SURFACE).scope_key();
    let asked = m::text(&finder, "query").trim().to_owned();
    let mut reading = finder;
    reading["loading"] = json!(true);
    if m::yes(&reading["open"]) {
        dom::text("graph-find-note", &found_note(&reading));
    }
    m::set("graph-find", reading);
    let found = api::scoped("graph-find", &path).await;
    let mut finder = self::finder();
    match found {
        Ok((epoch, found)) => {
            if !m::current("graph-find", epoch)
                || surface_scope(MODAL_SURFACE).scope_key() != scope_key
            {
                return;
            }
            finder["rows"] = json!(m::array(&found["rows"]));
            finder["found"] = json!(found["found"].as_u64().unwrap_or(0));
            finder["missing"] = json!(m::yes(&found["missing"]));
            finder["notests"] = json!(m::yes(&found["notests"]));
            finder["asked"] = json!(asked);
            finder["for"] = json!(path);
            finder["active"] = json!(0);
            finder["failed"] = V::Null;
        }
        Err(error) if error.aborted => return,
        Err(error) => {
            finder["rows"] = json!([]);
            finder["found"] = json!(0);
            finder["for"] = json!("");
            finder["failed"] = json!(error.message);
        }
    }
    finder["loading"] = json!(false);
    m::set("graph-find", finder);
    show_found();
}
pub(super) fn parse_style_px(style: &str, prop: &str) -> Option<f64> {
    let prefix = format!("{prop}:");
    style.split(';').find_map(|part| {
        let rest = part.trim().strip_prefix(&prefix)?.trim();
        rest.strip_suffix("px")
            .unwrap_or(rest)
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
    })
}
/// Brings the selected box of the map into the middle of what is seen of
/// the picture, where it is not wholly seen, and puts the keyboard on it.
pub(super) fn reveal() {
    let Some(canvas) = dom::get("graph-map-canvas") else {
        return;
    };
    let Some(node) = canvas
        .query_selector(".graph-map-node[data-selected=\"true\"]")
        .ok()
        .flatten()
    else {
        return;
    };
    let (room, at) = (
        canvas.get_bounding_client_rect(),
        node.get_bounding_client_rect(),
    );
    let edge = 8.0;
    if at.left() < room.left() + edge
        || at.right() > room.right() - edge
        || at.top() < room.top() + edge
        || at.bottom() > room.bottom() - edge
    {
        let mut cam = load_surface_camera(MODAL_SURFACE);
        let style = node.get_attribute("style").unwrap_or_default();
        let scale = cam.scale.max(0.01);
        let wx = parse_style_px(&style, "left")
            .map(|x| x + BOX_WIDTH / 2.0)
            .unwrap_or_else(|| (at.left() + at.width() / 2.0 - room.left() - cam.pan_x) / scale);
        let wy = parse_style_px(&style, "top")
            .map(|y| y + BOX_HEIGHT / 2.0)
            .unwrap_or_else(|| (at.top() + at.height() / 2.0 - room.top() - cam.pan_y) / scale);
        let (vw, vh, _) = viewport_size(MODAL_SURFACE.canvas);
        cam.apply_viewport_cam(graph_viewport::center_camera_on(
            (wx, wy),
            (vw, vh),
            cam.scale,
        ));
        cam.fit_mode = false;
        cam.needs_measure = false;
        save_surface_camera(MODAL_SURFACE, &cam);
        apply_surface_camera(MODAL_SURFACE, &cam);
    }
    if let Ok(node) = node.dyn_into::<web_sys::HtmlElement>() {
        let _ = node.focus();
    }
}
/// Keys of the search of the project. A slash goes to its field from
/// anywhere in the map but another field. In the field and in the list the
/// arrows walk what was found, Enter goes to the marked place, and Escape
/// closes the list, not the map behind it.
pub(super) fn find_key(e: &web_sys::KeyboardEvent) -> bool {
    if !open() || state()["cycle_base"].is_object() {
        return false;
    }
    let target = e
        .target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok());
    let bare = !e.ctrl_key() && !e.meta_key() && !e.alt_key();
    if e.key() == "/" && bare {
        // In a field a slash is a letter; a box to tick takes none.
        let typing = target
            .as_ref()
            .is_some_and(|element| match element.tag_name().as_str() {
                "TEXTAREA" | "SELECT" => true,
                "INPUT" => !matches!(
                    element.get_attribute("type").as_deref(),
                    Some("checkbox" | "radio")
                ),
                _ => false,
            });
        // The window of a link lies over the map.
        let covered = dom::get("graph-link").is_some_and(|window| window.has_attribute("open"));
        if typing || covered {
            return false;
        }
        dom::focus("graph-find-search");
        return true;
    }
    let within = target.map(|element| element.id()).unwrap_or_default();
    if !["graph-find-search", "graph-find-list", "graph-find-found"].contains(&within.as_str()) {
        return false;
    }
    let finder = finder();
    let rows = m::array(&finder["rows"]).len();
    let active = finder["active"].as_u64().unwrap_or(0) as usize;
    if !m::yes(&finder["open"]) {
        // Closed with Escape, the list comes back with an arrow.
        if e.key() == "ArrowDown" && find_path(&finder).is_some() {
            spawn_local(async {
                event("focusin", "graph-find-search", "", &V::Null, "", false).await;
            });
            return true;
        }
        return false;
    }
    match e.key().as_str() {
        "Escape" => {
            close_found();
            dom::focus("graph-find-search");
            true
        }
        "ArrowDown" if rows > 0 => {
            mark_found((active + 1).min(rows - 1));
            true
        }
        "ArrowUp" if rows > 0 => {
            mark_found(active.saturating_sub(1));
            true
        }
        "Enter" if bare && !e.shift_key() => {
            spawn_local(async move {
                // Typed fast, the list still answers less than was typed:
                // the first of what the whole of it finds is meant.
                let mut row = active;
                if !answered(&self::finder()) {
                    dom::cancel_timer("graph-find-search");
                    seek().await;
                    row = 0;
                }
                if answered(&self::finder()) {
                    event("click", "", "graph-find-choose", &json!(row), "", false).await;
                }
            });
            true
        }
        _ => false,
    }
}
pub(super) fn fresh_commit_dock(scope_key: &str) -> V {
    json!({
        "open": false,
        "query": "",
        "scope": "",
        "active": null,
        "rows": [],
        "more": false,
        "loading": false,
        "failed": null,
        "scope_key": scope_key,
    })
}
pub(super) fn commit_dock() -> V {
    let dock = m::data("graph-commit-graph");
    if dock.is_object() {
        dock
    } else {
        fresh_commit_dock("")
    }
}
pub(super) fn commit_dock_note(dock: &V) -> String {
    if let Some(failed) = dock["failed"].as_str().filter(|s| !s.is_empty()) {
        return failed.to_owned();
    }
    let rows = m::array(&dock["rows"]);
    if m::yes(&dock["loading"]) {
        return if rows.is_empty() {
            "Reading commit history…".to_owned()
        } else {
            "Loading older commits…".to_owned()
        };
    }
    if rows.is_empty() {
        return "No commit matches.".to_owned();
    }
    format!(
        "{} · click a commit to inspect its changes",
        plural(rows.len() as u64, "commit", "commits")
    )
}
#[cfg(test)]
pub(super) fn render_commit_dock_list(dock: &V, selected_target: &str, now: i64) -> String {
    #[cfg(target_arch = "wasm32")]
    let window = dom::get("graph-commit-graph-list").map(|list| {
        (
            list.scroll_top() as f64,
            (list.client_height() as f64).max(320.0),
        )
    });
    #[cfg(not(target_arch = "wasm32"))]
    let window: Option<(f64, f64)> = None;
    render_commit_dock_list_with_window(dock, selected_target, now, window)
}

pub(super) fn render_commit_dock_list_with_window(
    dock: &V,
    selected_target: &str,
    now: i64,
    window: Option<(f64, f64)>,
) -> String {
    // A row shows how much memory to review only once its commit's banner
    // has the answers; rows are never asked for a count each.
    let counts = m::data(COMMIT_MEMORY_COUNTS);
    let errors = m::data(COMMIT_MEMORY_ERRORS);
    let rows = m::array(&dock["rows"])
        .into_iter()
        .map(|mut row| {
            if let Some(sha) = row["commit"].as_str() {
                if let Some(count) = counts[sha].as_u64() {
                    row["memory_review"] = json!(count);
                } else if errors[sha] == true {
                    row["memory_review_error"] = json!(true);
                }
            }
            row
        })
        .collect::<Vec<_>>();
    let selected_sha = selected_target.strip_prefix("commit:");
    let active_index = dock["active"]
        .as_u64()
        .map(|idx| idx as usize)
        .filter(|&idx| idx < rows.len());
    let range_base = m::text(dock, "range_base");
    let range_head = m::text(dock, "range_head");
    let range = (!range_base.is_empty() && !range_head.is_empty())
        .then_some((range_base.as_str(), range_head.as_str()));
    crate::git_commit_graph::render_commit_graph_rows_with_range(
        &rows,
        active_index,
        selected_sha,
        range,
        window,
        m::yes(&dock["more"]),
        now,
        crate::git_commit_graph::CommitGraphSurface::Dock,
    )
}
pub(super) fn is_commit_view(view: &V) -> bool {
    view["chosen"]["kind"] == "commit" || view.get("commit_info").is_some_and(V::is_object)
}
pub(super) fn render_commit_banner_for_view(view: &V, changes: Option<&V>) -> String {
    if !is_commit_view(view) {
        return String::new();
    }
    let dock = commit_dock();
    let range_bar = crate::git_commit_graph::render_commit_range_banner(
        &m::text(&dock, "range_base"),
        &m::text(&dock, "range_head"),
    );
    let banner = format!(
        "{range_bar}{}",
        crate::git_commit_graph::render_commit_banner(
            view.get("commit_info"),
            &view["chosen"],
            changes,
        )
    );
    if crate::git_commit_graph::renamed_files(changes, 1).is_empty() {
        return banner;
    }
    let project = view["project_id"]
        .as_str()
        .filter(|project| !project.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| surface_scope(MODAL_SURFACE).project_id().to_owned());
    let commit = view["commit_info"]["commit"]
        .as_str()
        .or_else(|| view["chosen"]["head"]["commit"].as_str())
        .or_else(|| {
            view["chosen"]["target"]
                .as_str()
                .and_then(|target| target.strip_prefix("commit:"))
        })
        .unwrap_or("");
    format!(
        "{banner}{}",
        commit_memory_section(&project, commit, changes)
    )
}
/// The commit-dock counts of memory to review, by commit, from the answers
/// its banner received.
pub(super) const COMMIT_MEMORY_COUNTS: &str = "graph-commit-memory-counts";
/// Commits whose batched renamed-file memory check failed, so the dock can
/// surface the failure instead of falsely caching `0`.
pub(super) const COMMIT_MEMORY_ERRORS: &str = "graph-commit-memory-errors";
/// At most this many renamed files of one commit are asked about.
pub(super) const COMMIT_MEMORY_FILES: usize = 12;
pub(super) fn rename_memory_key(project: &str, path: &str, previous: &str) -> String {
    format!("{project}\u{1f}{path}\u{1f}{previous}")
}
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
pub(super) fn build_memory_links_batch_payload(files: &[(String, String)]) -> V {
    json!({
        "files": files
            .iter()
            .map(|(path, previous)| json!({"path": path, "previous_path": previous}))
            .collect::<Vec<_>>()
    })
}
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
pub(super) fn apply_memory_links_batch_response(
    cache: &mut V,
    project: &str,
    requested: &[(String, String)],
    result: Result<&V, &str>,
    now: f64,
) {
    if !cache.is_object() {
        *cache = json!({});
    }
    match result {
        Ok(resp) => {
            let items = resp["items"].as_array();
            for (idx, (path, previous)) in requested.iter().enumerate() {
                let key = rename_memory_key(project, path, previous);
                let mut links = items
                    .and_then(|arr| {
                        arr.iter()
                            .find(|item| {
                                item["path"].as_str() == Some(path.as_str())
                                    && item["previous_path"].as_str().unwrap_or("")
                                        == previous.as_str()
                            })
                            .or_else(|| {
                                (arr.len() == requested.len())
                                    .then(|| arr.get(idx))
                                    .flatten()
                            })
                    })
                    .cloned()
                    .unwrap_or_else(|| json!({"error": "Missing memory-links batch item"}));
                links["at"] = json!(now);
                cache[&key] = links;
            }
        }
        Err(message) => {
            for (path, previous) in requested {
                let key = rename_memory_key(project, path, previous);
                cache[&key] = json!({"error": message, "at": now});
            }
        }
    }
}
/// What the project's memory says about the files a commit renamed: the
/// warnings to review, filled in as answers arrive. Nothing is retired or
/// changed; once every answer is in, the commit's dock row shows the count.
pub(super) fn commit_memory_section(project: &str, commit: &str, changes: Option<&V>) -> String {
    let renamed = crate::git_commit_graph::renamed_files(changes, COMMIT_MEMORY_FILES);
    if project.is_empty() || renamed.is_empty() {
        return String::new();
    }
    let cache = m::data(MEMORY_LINKS);
    let mut answers = Vec::with_capacity(renamed.len());
    let mut missing: Vec<(String, String)> = Vec::new();
    for (path, previous) in &renamed {
        let key = rename_memory_key(project, path, previous);
        let cached = cache[&key].clone();
        if cached.is_object() && memory_links_fresh(&cached) {
            answers.push(cached);
        } else {
            missing.push((path.clone(), previous.clone()));
            answers.push(json!({"pending": true}));
        }
    }
    #[cfg(target_arch = "wasm32")]
    if !missing.is_empty() {
        let now = js_sys::Date::now();
        let mut cache = m::data(MEMORY_LINKS);
        if !cache.is_object() {
            cache = json!({});
        }
        for (path, previous) in &missing {
            let key = rename_memory_key(project, path, previous);
            cache[&key] = json!({"pending": true, "at": now});
        }
        m::set(MEMORY_LINKS, cache);
        let project_owned = project.to_owned();
        let url = format!(
            "/api/projects/{}/graph/memory-links/batch",
            dom::enc(&project_owned)
        );
        let body = build_memory_links_batch_payload(&missing);
        spawn_local(async move {
            let res = crate::api::post(&url, body).await;
            let now = js_sys::Date::now();
            let mut cache = m::data(MEMORY_LINKS);
            match &res {
                Ok(resp) => {
                    apply_memory_links_batch_response(
                        &mut cache,
                        &project_owned,
                        &missing,
                        Ok(resp),
                        now,
                    );
                }
                Err(err) => {
                    apply_memory_links_batch_response(
                        &mut cache,
                        &project_owned,
                        &missing,
                        Err(err.message.as_str()),
                        now,
                    );
                }
            }
            m::set(MEMORY_LINKS, cache);
            refresh_commit_memory();
        });
    }
    if !commit.is_empty()
        && answers
            .iter()
            .all(|answer| answer.is_object() && answer["pending"] != true)
    {
        let any_error = answers
            .iter()
            .any(|answer| answer["error"].as_str().is_some());
        let mut counts = m::data(COMMIT_MEMORY_COUNTS);
        let mut errors = m::data(COMMIT_MEMORY_ERRORS);
        if any_error {
            if let Some(obj) = counts.as_object_mut() {
                if obj.remove(commit).is_some() {
                    m::set(COMMIT_MEMORY_COUNTS, counts);
                }
            }
            if !errors.is_object() {
                errors = json!({});
            }
            if errors[commit] != true {
                errors[commit] = json!(true);
                m::set(COMMIT_MEMORY_ERRORS, errors);
            }
        } else {
            if let Some(obj) = errors.as_object_mut() {
                if obj.remove(commit).is_some() {
                    m::set(COMMIT_MEMORY_ERRORS, errors);
                }
            }
            let count = crate::git_commit_graph::rename_review_count(&answers);
            if !counts.is_object() {
                counts = json!({});
            }
            if counts[commit] != count {
                counts[commit] = json!(count);
                m::set(COMMIT_MEMORY_COUNTS, counts);
            }
        }
    }
    let total = changes
        .and_then(|catalog| catalog["renamed_files"].as_u64())
        .map_or(renamed.len(), |count| count as usize);
    let body = crate::git_commit_graph::render_rename_memory_warnings(
        &renamed,
        &answers,
        total.saturating_sub(renamed.len()),
    );
    if body.is_empty() {
        return String::new();
    }
    format!(
        "<div class=\"graph-commit-memory\" data-commit=\"{}\" aria-live=\"polite\">{body}</div>",
        escape(commit)
    )
}
/// Redraws the commit banner, and the dock's counts, once the batched answer
/// about renamed files arrives.
#[cfg(target_arch = "wasm32")]
pub(super) fn refresh_commit_memory() {
    let view = m::data(MODAL_SURFACE.view_key);
    if !view.is_object() {
        return;
    }
    let changes = surface_changes_doc(MODAL_SURFACE, &state());
    let banner_html = render_commit_banner_for_view(&view, changes.as_ref());
    html("graph-commit-banner", &banner_html);
    dom::hidden("graph-commit-banner", banner_html.trim().is_empty());
    show_commit_dock();
}
/// Opens the dependency map of `project_id` at one Git commit, as its
/// history lists it.
pub(crate) fn open_project_commit(project_id: &str, sha: &str) {
    open_system_service_map(
        GraphTargetScope::single(project_id),
        Some(&format!("commit:{sha}")),
    );
}
pub(super) fn commit_action_target(action: &str, payload: &V) -> Option<String> {
    match action {
        "graph-commit-select" => payload
            .as_str()
            .or_else(|| payload["target"].as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        "graph-commit-reset" => Some(String::new()),
        _ => None,
    }
}
thread_local! {
    static COMMIT_DOCK_LAST_WINDOW: RefCell<Option<(usize, usize, usize, u64)>> =
        const { RefCell::new(None) };
    static COMMIT_DOCK_RAF_PENDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static COMMIT_DOCK_RAF_CB: RefCell<Option<Closure<dyn FnMut()>>> =
        const { RefCell::new(None) };
}

fn commit_dock_data_sig(dock: &V, selected_target: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let rows = m::array(&dock["rows"]);
    rows.len().hash(&mut h);
    rows.first().and_then(|r| r["commit"].as_str()).hash(&mut h);
    rows.last().and_then(|r| r["commit"].as_str()).hash(&mut h);
    selected_target.hash(&mut h);
    m::text(dock, "range_base").hash(&mut h);
    m::text(dock, "range_head").hash(&mut h);
    m::yes(&dock["more"]).hash(&mut h);
    h.finish()
}

fn sync_commit_dock_virtual_window(force: bool) {
    let dock = commit_dock();
    if !m::yes(&dock["open"]) {
        COMMIT_DOCK_LAST_WINDOW.with(|w| *w.borrow_mut() = None);
        return;
    }
    let rows = m::array(&dock["rows"]);
    let target = m::text(&state(), "target");
    let list_opt = dom::get("graph-commit-graph-list");
    let (scroll_top, viewport_h) = list_opt.as_ref().map_or((0.0, 320.0), |l| {
        (l.scroll_top() as f64, (l.client_height() as f64).max(320.0))
    });
    let (start_idx, end_idx) =
        crate::git_commit_graph::commit_dock_window_range(rows.len(), scroll_top, viewport_h);
    let sig = commit_dock_data_sig(&dock, &target);
    if !force
        && COMMIT_DOCK_LAST_WINDOW
            .with(|w| *w.borrow() == Some((start_idx, end_idx, rows.len(), sig)))
    {
        return;
    }
    COMMIT_DOCK_LAST_WINDOW.with(|w| *w.borrow_mut() = Some((start_idx, end_idx, rows.len(), sig)));
    let now = (js_sys::Date::now() / 1000.0) as i64;
    let list_html = render_commit_dock_list_with_window(
        &dock,
        &target,
        now,
        list_opt.as_ref().map(|_| (scroll_top, viewport_h)),
    );
    html("graph-commit-graph-list", &list_html);
    if let Some(list) = list_opt {
        if rows.len() > 80 && scroll_top > 0.0 {
            list.set_scroll_top(scroll_top.round() as i32);
        }
    }
}

pub(super) fn ensure_commit_dock_scroll_listener() {
    let Some(list) = dom::get("graph-commit-graph-list") else {
        return;
    };
    if list.get_attribute("data-scroll-watched").as_deref() == Some("true") {
        return;
    }
    let _ = list.set_attribute("data-scroll-watched", "true");
    COMMIT_DOCK_RAF_CB.with(|cell| {
        if cell.borrow().is_none() {
            *cell.borrow_mut() = Some(Closure::wrap(Box::new(move || {
                COMMIT_DOCK_RAF_PENDING.with(|p| p.set(false));
                let dock = commit_dock();
                if m::yes(&dock["open"]) && m::array(&dock["rows"]).len() > 80 {
                    sync_commit_dock_virtual_window(false);
                }
            }) as Box<dyn FnMut()>));
        }
    });
    let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        let dock = commit_dock();
        if !m::yes(&dock["open"]) || m::array(&dock["rows"]).len() <= 80 {
            return;
        }
        let already = COMMIT_DOCK_RAF_PENDING.with(|p| p.replace(true));
        if already {
            return;
        }
        COMMIT_DOCK_RAF_CB.with(|cell| {
            if let Some(raf_cb) = cell.borrow().as_ref() {
                let _ = dom::window().request_animation_frame(raf_cb.as_ref().unchecked_ref());
            } else {
                COMMIT_DOCK_RAF_PENDING.with(|p| p.set(false));
            }
        });
    });
    if list
        .add_event_listener_with_callback("scroll", cb.as_ref().unchecked_ref())
        .is_ok()
    {
        cb.forget();
    }
}

pub(super) fn show_commit_dock() {
    let dock = commit_dock();
    let opened = m::yes(&dock["open"]);
    dom::hidden("graph-commit-graph-panel", !opened);
    dom::attr(
        "graph-commit-graph-toggle",
        "aria-expanded",
        if opened { "true" } else { "false" },
    );
    if !opened {
        COMMIT_DOCK_LAST_WINDOW.with(|w| *w.borrow_mut() = None);
        return;
    }
    ensure_commit_dock_scroll_listener();
    let target = m::text(&state(), "target");
    let rows = m::array(&dock["rows"]);
    let active = dock["active"]
        .as_u64()
        .map(|idx| idx as usize)
        .filter(|&idx| idx < rows.len())
        .or_else(|| {
            crate::git_commit_graph::selected_commit_row_index(
                &rows,
                target.strip_prefix("commit:"),
            )
        });
    if rows.len() > 80 {
        if let (Some(idx), Some(list)) = (active, dom::get("graph-commit-graph-list")) {
            let viewport_h = (list.client_height() as f64).max(320.0);
            if let Some(next_top) = crate::git_commit_graph::scroll_top_for_commit_row(
                idx,
                list.scroll_top() as f64,
                viewport_h,
            ) {
                list.set_scroll_top(next_top.round() as i32);
            }
        }
    }
    sync_commit_dock_virtual_window(true);
    if let Some(idx) = active {
        dom::attr(
            "graph-commit-graph-list",
            "aria-activedescendant",
            &format!("graph-commit-row-{idx}"),
        );
    } else if let Some(list) = dom::get("graph-commit-graph-list") {
        let _ = list.remove_attribute("aria-activedescendant");
    }
    dom::text("graph-commit-graph-note", &commit_dock_note(&dock));
}
pub(super) fn shut_commit_dock() {
    let mut dock = commit_dock();
    if !m::yes(&dock["open"]) {
        return;
    }
    dock["open"] = json!(false);
    dock["active"] = V::Null;
    m::set("graph-commit-graph", dock);
    dom::cancel_timer("graph-commit-graph-search");
    show_commit_dock();
}
pub(super) fn mark_commit_dock(next: usize) {
    let mut dock = commit_dock();
    let rows = m::array(&dock["rows"]);
    let Some(last) = rows.len().checked_sub(1) else {
        return;
    };
    let next = next.min(last);
    let target = m::text(&state(), "target");
    let prev = dock["active"]
        .as_u64()
        .map(|idx| idx as usize)
        .filter(|&idx| idx < rows.len())
        .or_else(|| {
            crate::git_commit_graph::selected_commit_row_index(
                &rows,
                target.strip_prefix("commit:"),
            )
        });
    dock["active"] = json!(next);
    m::set("graph-commit-graph", dock);
    let active_id = format!("graph-commit-row-{next}");
    if rows.len() > 80 {
        if let Some(list) = dom::get("graph-commit-graph-list") {
            let viewport_h = (list.client_height() as f64).max(320.0);
            if let Some(next_top) = crate::git_commit_graph::scroll_top_for_commit_row(
                next,
                list.scroll_top() as f64,
                viewport_h,
            ) {
                list.set_scroll_top(next_top.round() as i32);
            }
        }
        if dom::get(&active_id).is_none() {
            sync_commit_dock_virtual_window(true);
        }
    }
    if let Some(before) = prev.filter(|&before| before != next) {
        dom::attr(
            &format!("graph-commit-row-{before}"),
            "aria-selected",
            "false",
        );
    }
    dom::attr(&active_id, "aria-selected", "true");
    dom::attr(
        "graph-commit-graph-list",
        "aria-activedescendant",
        &active_id,
    );
    into_view("graph-commit-graph-list", &active_id);
}
pub(super) fn reset_commit_dock_for_scope() {
    dom::cancel_timer("graph-commit-graph-search");
    m::next("graph-commit-graph");
    let scope_key = surface_scope(MODAL_SURFACE).scope_key();
    let mut dock = commit_dock();
    if m::text(&dock, "scope_key") != scope_key {
        dock = fresh_commit_dock(&scope_key);
    } else {
        dock["open"] = json!(false);
    }
    m::set("graph-commit-graph", dock);
    show_commit_dock();
}
pub(super) async fn fetch_commit_dock(further: bool) {
    let dock = commit_dock();
    let scope = surface_scope(MODAL_SURFACE);
    if scope.is_empty() || !m::yes(&dock["open"]) {
        return;
    }
    let scope_key = scope.scope_key();
    let had = if further {
        m::array(&dock["rows"])
    } else {
        vec![]
    };
    let path = build_scoped_picker_path(&scope, "commits", &dock, had.len());
    let mut reading = dock;
    reading["loading"] = json!(true);
    reading["scope_key"] = json!(scope_key.clone());
    dom::text("graph-commit-graph-note", &commit_dock_note(&reading));
    m::set("graph-commit-graph", reading);
    let found = api::scoped("graph-commit-graph", &path).await;
    let mut dock = commit_dock();
    match found {
        Ok((epoch, found)) => {
            if !m::current("graph-commit-graph", epoch)
                || surface_scope(MODAL_SURFACE).scope_key() != scope_key
            {
                return;
            }
            let mut rows = had;
            rows.extend(m::array(&found["commits"]));
            dock["rows"] = json!(rows);
            dock["more"] = json!(m::yes(&found["more"]));
            dock["failed"] = V::Null;
            dock["scope_key"] = json!(scope_key);
            if !further {
                dock["active"] = V::Null;
            }
        }
        Err(error) if error.aborted => return,
        Err(error) => {
            dock["rows"] = json!([]);
            dock["more"] = json!(false);
            dock["failed"] = json!(error.message);
            dock["active"] = V::Null;
        }
    }
    dock["loading"] = json!(false);
    m::set("graph-commit-graph", dock);
    show_commit_dock();
}
pub(super) async fn dock_histories() {
    let scope = surface_scope(MODAL_SURFACE);
    if scope.is_empty() {
        return;
    }
    let scope_key = scope.scope_key();
    let path = format_scoped_endpoint(&scope, "branches", Vec::new());
    let Ok(found) = api::get(&path).await else {
        return;
    };
    if surface_scope(MODAL_SURFACE).scope_key() == scope_key {
        m::set("graph-pick-branches", found["branches"].clone());
        html(
            "graph-commit-graph-scope",
            &scopes(
                &m::data("graph-map-place"),
                &found["branches"],
                &m::text(&commit_dock(), "scope"),
            ),
        );
    }
}
pub(super) fn picker() -> V {
    let pick = m::data("graph-pick");
    if pick.is_object() {
        pick
    } else {
        json!({"open": false, "tab": "commits", "query": "", "scope": "", "state": "open", "active": 0, "rows": [], "more": false, "loading": false, "reason": null, "failed": null})
    }
}
pub(super) fn open() -> bool {
    dom::get("graph-map").is_some_and(|dialog| dialog.has_attribute("open"))
}
pub(super) fn blank() {
    invalidate_surface_plan_cache("graph-map-canvas");
    graph_viewport::canvas2d::clear_scene("graph-map-canvas");
    invalidate_hover_cache("graph-map-canvas");
    set_dense_hover("graph-map-canvas", None);
    html("graph-map-crumbs", "");
    html("graph-map-canvas", "");
    html("graph-map-canvas-radar", "");
    sync_radar("graph-map-canvas", None);
    let cam = Camera {
        fit_mode: m::yes(&state()["fit"]),
        follow: false,
        needs_measure: true,
        ..Camera::default()
    };
    save_surface_camera(MODAL_SURFACE, &cam);
    m::set(
        MODAL_SURFACE.drag_key,
        graph_viewport::DragPan::default().to_json(),
    );
    html("graph-map-side", "");
    html("graph-map-arrow-legend", "");
    html("graph-commit-banner", "");
    dom::hidden("graph-commit-banner", true);
    dom::hidden("graph-map-launches", true);
    dom::hidden("graph-map-agent", true);
    dom::hidden("graph-map-agent-field", true);
    m::take("graph-map-attachment");
    dom::text("graph-map-summary", "");
}
pub(super) fn sync_modal_service_switcher(scope: &GraphTargetScope) {
    let is_sys = scope.is_system();
    dom::hidden("graph-map-service", !is_sys);
    dom::hidden("graph-map-service-field", !is_sys);
    if is_sys {
        html("graph-map-service", &service_switcher_options_html(scope));
        if let Some(svc) = scope.service_id() {
            dom::set_value("graph-map-service", svc);
        }
    }
}
/// The Show list as it stands, with the run of the session or active System Change on the screen.
pub(super) fn list() {
    let scope = surface_scope(MODAL_SURFACE);
    sync_modal_service_switcher(&scope);
    let chosen_target = effective_target(&state());
    let base_options = targets(&m::data("graph-map-place"), &chosen_target);
    let rendered = match &scope {
        GraphTargetScope::SingleProject { .. } => {
            with_run(base_options, &chosen_target, &m::read(|s| s.sid.clone()))
        }
        GraphTargetScope::SystemService {
            service_id,
            run_id,
            run_title,
            ..
        } => {
            let whole_label = if service_id == "__system__" {
                "The whole system root (__system__)".to_owned()
            } else {
                format!("The whole service ({service_id})")
            };
            let relabeled = base_options.replacen(
                ">The whole project</option>",
                &format!(">{}</option>", escape(&whole_label)),
                1,
            );
            with_system_run(
                relabeled,
                &chosen_target,
                run_id.as_deref(),
                run_title.as_deref(),
            )
        }
    };
    html("graph-map-target", &rendered);
}
/// Draws the picker as it stands.
pub(super) fn show() {
    let pick = picker();
    let opened = m::yes(&pick["open"]);
    dom::hidden("graph-pick", !opened);
    dom::attr(
        "graph-pick-open",
        "aria-expanded",
        if opened { "true" } else { "false" },
    );
    if !opened {
        return;
    }
    let tab = m::text(&pick, "tab");
    for name in TABS {
        dom::attr(
            &format!("graph-pick-tab-{name}"),
            "aria-selected",
            if name == tab { "true" } else { "false" },
        );
    }
    dom::hidden("graph-pick-scope", tab != "commits");
    dom::hidden("graph-pick-state", tab != "pulls");
    dom::attr(
        "graph-pick-search",
        "placeholder",
        match tab.as_str() {
            "pulls" => "Search pull requests, or type a number",
            "branches" => "Filter branches by name",
            _ => "Search commit messages, or paste a hash",
        },
    );
    html(
        "graph-pick-list",
        &options(&pick, (js_sys::Date::now() / 1000.0) as i64),
    );
    dom::text("graph-pick-note", &remark(&pick));
}
/// Moves the picker's mark to a row and brings the row into view; the list
/// is not drawn again, so it stays where it was scrolled to.
pub(super) fn mark(next: usize) {
    let mut pick = picker();
    let before = pick["active"].as_u64().unwrap_or(0);
    pick["active"] = json!(next);
    m::set("graph-pick", pick);
    dom::attr(
        &format!("graph-pick-row-{before}"),
        "aria-selected",
        "false",
    );
    dom::attr(&format!("graph-pick-row-{next}"), "aria-selected", "true");
    into_view("graph-pick-list", &format!("graph-pick-row-{next}"));
}
/// Scrolls a list as far as a row of it needs to be wholly seen.
pub(super) fn into_view(list: &str, row: &str) {
    let (Some(list), Some(row)) = (
        dom::get(list),
        dom::get(row).and_then(|row| row.dyn_into::<web_sys::HtmlElement>().ok()),
    ) else {
        return;
    };
    let (top, height) = (row.offset_top(), row.offset_height());
    if top < list.scroll_top() {
        list.set_scroll_top(top);
    } else if top + height > list.scroll_top() + list.client_height() {
        list.set_scroll_top(top + height - list.client_height());
    }
}
pub fn build_scoped_picker_path(
    scope: &GraphTargetScope,
    tab: &str,
    pick: &V,
    skip: usize,
) -> String {
    let query = m::text(pick, "query").trim().to_owned();
    match tab {
        "pulls" => {
            let mut params = Vec::new();
            let st = m::text(pick, "state");
            if !st.is_empty() {
                params.push(("state", st));
            }
            if !query.is_empty() {
                params.push(("query", query));
            }
            format_scoped_endpoint(scope, "pulls", params)
        }
        "branches" => format_scoped_endpoint(scope, "branches", Vec::new()),
        _ => {
            let mut params = Vec::new();
            let sc = m::text(pick, "scope");
            if !sc.is_empty() {
                params.push(("scope", sc));
            }
            if !query.is_empty() {
                params.push(("query", query));
            }
            if skip > 0 {
                params.push(("skip", skip.to_string()));
            }
            format_scoped_endpoint(scope, "commits", params)
        }
    }
}
/// Reads the rows of the picker's open tab, or, with `further`, the next
/// page of a history after the ones it has.
pub(super) async fn fetch(further: bool) {
    let pick = picker();
    let scope = surface_scope(MODAL_SURFACE);
    if scope.is_empty() || !m::yes(&pick["open"]) {
        return;
    }
    let scope_key = scope.scope_key();
    let tab = m::text(&pick, "tab");
    let had = if further {
        m::array(&pick["rows"])
    } else {
        vec![]
    };
    let path = build_scoped_picker_path(&scope, &tab, &pick, had.len());
    let mut reading = pick;
    reading["loading"] = json!(true);
    dom::text("graph-pick-note", &remark(&reading));
    m::set("graph-pick", reading);
    let found = api::scoped("graph-pick", &path).await;
    let mut pick = picker();
    match found {
        Ok((epoch, found)) => {
            if !m::current("graph-pick", epoch)
                || m::text(&pick, "tab") != tab
                || surface_scope(MODAL_SURFACE).scope_key() != scope_key
            {
                return;
            }
            let mut rows = had;
            rows.extend(m::array(&found[tab.as_str()]));
            if !further {
                pick["active"] = json!(0);
            }
            pick["rows"] = json!(rows);
            pick["more"] = json!(m::yes(&found["more"]));
            pick["reason"] = found["reason"].clone();
            pick["failed"] = V::Null;
            if tab == "branches" {
                m::set("graph-pick-branches", found["branches"].clone());
            }
        }
        Err(error) if error.aborted => return,
        Err(error) => {
            pick["rows"] = json!([]);
            pick["more"] = json!(false);
            pick["failed"] = json!(error.message);
        }
    }
    pick["loading"] = json!(false);
    let active = pick["active"].as_u64().unwrap_or(0) as usize;
    m::set("graph-pick", pick);
    show();
    if further {
        mark(active);
    }
}
/// The local branches, for the list of whose history to show.
pub(super) async fn histories() {
    let scope = surface_scope(MODAL_SURFACE);
    if scope.is_empty() {
        return;
    }
    let scope_key = scope.scope_key();
    let path = format_scoped_endpoint(&scope, "branches", Vec::new());
    let Ok(found) = api::get(&path).await else {
        return;
    };
    if surface_scope(MODAL_SURFACE).scope_key() == scope_key {
        m::set("graph-pick-branches", found["branches"].clone());
        let place = m::data("graph-map-place");
        html(
            "graph-pick-scope",
            &scopes(&place, &found["branches"], &m::text(&picker(), "scope")),
        );
        if m::yes(&commit_dock()["open"]) {
            html(
                "graph-commit-graph-scope",
                &scopes(
                    &place,
                    &found["branches"],
                    &m::text(&commit_dock(), "scope"),
                ),
            );
        }
    }
}
pub(super) fn shut() {
    let mut pick = picker();
    if !m::yes(&pick["open"]) {
        return;
    }
    pick["open"] = json!(false);
    m::set("graph-pick", pick);
    dom::cancel_timer("graph-pick-search");
    show();
}
pub(super) fn is_editable_key_target(target: Option<&web_sys::Element>) -> bool {
    target.is_some_and(|el| {
        matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT")
            || el
                .closest("[contenteditable=''], [contenteditable='true']")
                .ok()
                .flatten()
                .is_some()
    })
}
pub(super) fn commit_dock_key(e: &web_sys::KeyboardEvent) -> bool {
    if !open()
        || m::yes(&picker()["open"])
        || dom::get("graph-link").is_some_and(|window| window.has_attribute("open"))
    {
        return false;
    }
    let dock = commit_dock();
    if !m::yes(&dock["open"]) {
        return false;
    }
    let target_el = e
        .target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok());
    let within_panel = target_el.as_ref().is_some_and(|el| {
        el.closest("#graph-commit-graph-panel")
            .ok()
            .flatten()
            .is_some()
    });
    if e.key() == "Escape" && within_panel {
        shut_commit_dock();
        dom::focus("graph-commit-graph-toggle");
        return true;
    }
    let within_search = target_el
        .as_ref()
        .is_some_and(|el| el.id() == "graph-commit-graph-search");
    let within_list = target_el.as_ref().is_some_and(|el| {
        el.closest("#graph-commit-graph-list")
            .ok()
            .flatten()
            .is_some()
    });
    if !within_search && !within_list {
        return false;
    }
    let on_button = target_el
        .as_ref()
        .is_some_and(|el| el.closest("button").ok().flatten().is_some());
    let rows = m::array(&dock["rows"]);
    let Some(last) = rows.len().checked_sub(1) else {
        return false;
    };
    let current = dock["active"]
        .as_u64()
        .map(|idx| idx as usize)
        .filter(|&idx| idx < rows.len());
    let target = m::text(&state(), "target");
    let selected =
        crate::git_commit_graph::selected_commit_row_index(&rows, target.strip_prefix("commit:"));
    let bare = !e.ctrl_key() && !e.meta_key() && !e.alt_key();
    match e.key().as_str() {
        "ArrowDown" => {
            let anchor = current.or(selected);
            if anchor == Some(last) {
                if m::yes(&dock["more"]) && !m::yes(&dock["loading"]) {
                    spawn_local(async move {
                        fetch_commit_dock(true).await;
                        if m::array(&commit_dock()["rows"]).len() > last + 1 {
                            mark_commit_dock(last + 1);
                        }
                    });
                }
                return true;
            }
            if let Some(next) = crate::git_commit_graph::next_commit_dock_index(
                current,
                selected,
                rows.len(),
                "ArrowDown",
            ) {
                mark_commit_dock(next);
                return true;
            }
            false
        }
        "ArrowUp" => {
            if let Some(next) = crate::git_commit_graph::next_commit_dock_index(
                current,
                selected,
                rows.len(),
                "ArrowUp",
            ) {
                mark_commit_dock(next);
                return true;
            }
            false
        }
        "Home" | "End" if within_list => {
            if let Some(next) = crate::git_commit_graph::next_commit_dock_index(
                current,
                selected,
                rows.len(),
                e.key().as_str(),
            ) {
                mark_commit_dock(next);
                return true;
            }
            false
        }
        "Enter" | " " if bare && !on_button => {
            let Some(active) = current.or(selected) else {
                return false;
            };
            let row_target = rows[active]["target"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .or_else(|| {
                    rows[active]["commit"]
                        .as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(|sha| format!("commit:{sha}"))
                });
            let Some(row_target) = row_target else {
                return false;
            };
            if e.key() == " " {
                e.prevent_default();
            }
            m::set("graph-commit-shift", json!(e.shift_key()));
            spawn_local(async move {
                event(
                    "click",
                    "",
                    "graph-commit-select",
                    &json!(row_target),
                    "",
                    false,
                )
                .await;
            });
            true
        }
        _ => false,
    }
}
pub(super) fn active_embedded_surface_for_key() -> Option<MapSurface> {
    let view_mode = m::read(|s| s.view.clone());
    let candidate = match view_mode.as_str() {
        "systems" => SYSTEM_EMBEDDED_SURFACE,
        "system-changes" => SYSTEM_CHANGE_SURFACE,
        "omnigraph" => return None,
        _ => SESSION_SURFACE,
    };
    let visible = dom::get(candidate.canvas)
        .is_some_and(|el| el.closest("[hidden]").ok().flatten().is_none());
    visible.then_some(candidate)
}
fn embedded_find_key(e: &web_sys::KeyboardEvent) -> bool {
    let Some(target) = e
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
    else {
        return false;
    };
    let surface = [
        SESSION_SURFACE,
        SYSTEM_EMBEDDED_SURFACE,
        SYSTEM_CHANGE_SURFACE,
    ]
    .into_iter()
    .find(|s| {
        target
            .closest(&format!("#{}-find", s.id_prefix))
            .ok()
            .flatten()
            .is_some()
    });
    let Some(surface) = surface else {
        return false;
    };
    let prefix = surface.id_prefix;
    let found_id = format!("{prefix}-find-found");
    let search_id = format!("{prefix}-find-search");
    let list_id = format!("{prefix}-find-list");
    let open = dom::get(&found_id).is_some_and(|el| !el.has_attribute("hidden"));
    if !open {
        return false;
    }
    let options = dom::all(&format!("#{list_id} li[role=\"option\"]"));
    let current = options
        .iter()
        .position(|opt| opt.get_attribute("aria-selected").as_deref() == Some("true"))
        .unwrap_or(0);
    match e.key().as_str() {
        "Escape" => {
            e.prevent_default();
            dom::hidden(&found_id, true);
            dom::attr(&search_id, "aria-expanded", "false");
            dom::attr(&search_id, "aria-activedescendant", "");
            dom::focus(&search_id);
            true
        }
        "ArrowDown" | "ArrowUp" | "Home" | "End" if !options.is_empty() => {
            e.prevent_default();
            let last = options.len() - 1;
            let next = match e.key().as_str() {
                "ArrowDown" => (current + 1).min(last),
                "ArrowUp" => current.saturating_sub(1),
                "Home" => 0,
                _ => last,
            };
            for (idx, opt) in options.iter().enumerate() {
                let _ =
                    opt.set_attribute("aria-selected", if idx == next { "true" } else { "false" });
            }
            if let Some(opt) = options.get(next) {
                if let Some(opt_id) = opt.get_attribute("id") {
                    dom::attr(&search_id, "aria-activedescendant", &opt_id);
                }
                opt.scroll_into_view();
            }
            true
        }
        "Enter" if !options.is_empty() => {
            let chosen_opt = options.get(current).or_else(|| options.first());
            if let Some(opt) = chosen_opt {
                e.prevent_default();
                dom::hidden(&found_id, true);
                dom::attr(&search_id, "aria-expanded", "false");
                dom::attr(&search_id, "aria-activedescendant", "");
                if let Some(raw) = opt.get_attribute("data-payload") {
                    if let Ok(parsed) = serde_json::from_str::<V>(&raw) {
                        let path = m::text(&parsed, "path");
                        if !path.is_empty() {
                            focus_file_on_surface(surface, &path);
                        }
                    }
                }
                return true;
            }
            false
        }
        _ => false,
    }
}

fn handle_surface_canvas_keyboard(surface: MapSurface, e: &web_sys::KeyboardEvent) -> bool {
    let target_el = e
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
    if is_editable_key_target(target_el.as_ref()) {
        return false;
    }
    let key = e.key();
    let key_str = key.as_str();
    let radar_id = format!("{}-radar", surface.canvas);
    let on_radar = target_el
        .as_ref()
        .is_some_and(|el| el.closest(&format!("#{radar_id}")).ok().flatten().is_some());
    if on_radar
        && matches!(
            key_str,
            "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown"
        )
    {
        let step = if e.shift_key() { 120.0 } else { 40.0 };
        let (dx, dy) = match key_str {
            "ArrowLeft" => (step, 0.0),
            "ArrowRight" => (-step, 0.0),
            "ArrowUp" => (0.0, step),
            "ArrowDown" => (0.0, -step),
            _ => (0.0, 0.0),
        };
        let mut cam = load_surface_camera(surface);
        cam.pan_x += dx;
        cam.pan_y += dy;
        cam.fit_mode = false;
        save_surface_camera(surface, &cam);
        apply_surface_camera(surface, &cam);
        e.prevent_default();
        return true;
    }
    if e.alt_key() && !e.ctrl_key() && !e.meta_key() {
        if let Some(node_el) = target_el
            .as_ref()
            .and_then(|el| el.closest(".graph-map-node[data-n]").ok().flatten())
        {
            if let Some(idx) = node_el
                .get_attribute("data-n")
                .and_then(|s| s.parse::<usize>().ok())
            {
                let step = if e.shift_key() { 60.0 } else { 20.0 };
                let (dx, dy) = match key_str {
                    "ArrowLeft" => (-step, 0.0),
                    "ArrowRight" => (step, 0.0),
                    "ArrowUp" => (0.0, -step),
                    "ArrowDown" => (0.0, step),
                    "Backspace" | "Delete" => {
                        let key_str_val = node_el
                            .get_attribute("data-payload")
                            .and_then(|raw| serde_json::from_str::<V>(&raw).ok())
                            .and_then(|v| v.as_str().map(str::to_owned))
                            .unwrap_or_default();
                        if !key_str_val.is_empty()
                            && super::surfaces::unpin_surface_node(surface, &key_str_val)
                        {
                            e.prevent_default();
                            return true;
                        }
                        return false;
                    }
                    _ => return false,
                };
                if super::surfaces::nudge_surface_node_pin(surface, idx, dx, dy) {
                    e.prevent_default();
                    return true;
                }
            }
        }
    }
    let bare = !e.ctrl_key() && !e.meta_key() && !e.alt_key();
    if bare && matches!(key_str, "Enter" | " ") {
        if let Some(edge_hit) = target_el
            .as_ref()
            .and_then(|el| el.closest(".graph-map-hit[role='button']").ok().flatten())
        {
            if let Some(raw) = edge_hit.get_attribute("data-payload") {
                if let Ok(payload) = serde_json::from_str::<V>(&raw) {
                    e.prevent_default();
                    if surface.embedded {
                        let _ = handle_surface_click(surface, "", "graph-map-link", &payload);
                    } else {
                        spawn_local(async move {
                            event("click", "", "graph-map-link", &payload, "", false).await;
                        });
                    }
                    return true;
                }
            }
        }
    }
    false
}

/// Keys of the map: those of the search of the project, Commit Dock arrows,
/// the open picker (arrows/Enter/Escape), and zoom/Escape shortcuts across
/// the modal, link dialog, and embedded graph surfaces.
pub fn key(e: &web_sys::KeyboardEvent) -> bool {
    if dom::get("graph-shell-event").is_some_and(|dialog| dialog.has_attribute("open")) {
        return false;
    }
    if e.is_composing() {
        return false;
    }
    if find_key(e) || embedded_find_key(e) || commit_dock_key(e) {
        return true;
    }
    let pick = picker();
    if !m::yes(&pick["open"]) {
        let target_el = e
            .target()
            .and_then(|target| target.dyn_into::<web_sys::Element>().ok());
        let in_input = is_editable_key_target(target_el.as_ref());
        let bare = !e.ctrl_key() && !e.meta_key() && !e.alt_key();
        let link_open = dom::get("graph-link").is_some_and(|window| window.has_attribute("open"));
        if link_open {
            if !in_input && bare {
                match e.key().as_str() {
                    "+" | "=" => return zoom_link_camera_at(ZOOM_STEP, None),
                    "-" => return zoom_link_camera_at(1.0 / ZOOM_STEP, None),
                    "0" => {
                        fit_link_camera();
                        return true;
                    }
                    "1" => {
                        reset_link_camera_100();
                        return true;
                    }
                    _ => {}
                }
            }
            return false;
        }
        if open() {
            if handle_surface_canvas_keyboard(MODAL_SURFACE, e) {
                return true;
            }
            if e.key() == "Escape" {
                if dom::get("graph-map-display-popover")
                    .is_some_and(|el| !el.has_attribute("hidden"))
                {
                    dom::hidden("graph-map-display-popover", true);
                    dom::attr("graph-map-display-toggle", "aria-expanded", "false");
                    return true;
                }
                if state()["pick"].is_object() {
                    spawn_local(async {
                        event("click", "", "graph-map-path-cancel", &V::Null, "", false).await;
                    });
                    return true;
                }
                if state()["traced_path"].is_object() || m::yes(&state()["isolate_active"]) {
                    let mut st = state();
                    st["traced_path"] = V::Null;
                    st["isolate_active"] = json!(false);
                    save_surface_state(MODAL_SURFACE, &st);
                    rerender_surface_preserving_camera(MODAL_SURFACE);
                    graph_viewport::announce_live(
                        "graph-map-live-announce",
                        "Cleared path and isolation",
                    );
                    return true;
                }
            }
            if !in_input && bare {
                if matches!(
                    e.key().as_str(),
                    "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" | "Home" | "End"
                ) && target_el
                    .as_ref()
                    .is_some_and(|el| el.closest("#graph-map-canvas").ok().flatten().is_some())
                    && move_roving_node_focus(MODAL_SURFACE, &e.key(), target_el.as_ref())
                {
                    e.prevent_default();
                    return true;
                }
                let mut cam = load_surface_camera(MODAL_SURFACE);
                if cam.plane_w > 0.0 && cam.plane_h > 0.0 {
                    match e.key().as_str() {
                        "+" | "=" => {
                            zoom_surface_camera_at(MODAL_SURFACE, ZOOM_STEP, None);
                            return true;
                        }
                        "-" => {
                            zoom_surface_camera_at(MODAL_SURFACE, 1.0 / ZOOM_STEP, None);
                            return true;
                        }
                        "0" => {
                            let mut st = state();
                            st["fit"] = json!(true);
                            m::set("graph-map", st);
                            dom::save(FIT, Some("1"));
                            dom::set_checked("graph-map-fit", true);
                            fit_surface_camera(MODAL_SURFACE, &mut cam);
                            return true;
                        }
                        "1" => {
                            reset_surface_camera_100(MODAL_SURFACE);
                            return true;
                        }
                        _ => {}
                    }
                }
            }
            return false;
        }
        if let Some(surface) = active_embedded_surface_for_key() {
            if handle_surface_canvas_keyboard(surface, e) {
                return true;
            }
            let mut cam = load_surface_camera(surface);
            if e.key() == "Escape" {
                if surface_state(surface)["pick"].is_object() {
                    return handle_surface_click(surface, "", "graph-map-path-cancel", &V::Null);
                }
                let mut st = surface_state(surface);
                if st["traced_path"].is_object() || m::yes(&st["isolate_active"]) {
                    st["traced_path"] = V::Null;
                    st["isolate_active"] = json!(false);
                    save_surface_state(surface, &st);
                    rerender_surface_preserving_camera(surface);
                    graph_viewport::announce_live(
                        &format!("{}-live-announce", surface.id_prefix),
                        "Cleared path and isolation",
                    );
                    return true;
                }
                if cam.expanded {
                    cam.expanded = false;
                    cam.needs_measure = true;
                    save_surface_camera(surface, &cam);
                    apply_surface_camera(surface, &cam);
                    dom::later(surface.refit_timer_key, 60, move || {
                        refit_surface_if_visible(surface);
                    });
                    return true;
                }
                return false;
            }
            if !in_input && bare {
                if matches!(
                    e.key().as_str(),
                    "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" | "Home" | "End"
                ) && target_el.as_ref().is_some_and(|el| {
                    el.closest(&format!("#{}", surface.canvas))
                        .ok()
                        .flatten()
                        .is_some()
                }) && move_roving_node_focus(surface, &e.key(), target_el.as_ref())
                {
                    e.prevent_default();
                    return true;
                }
                if cam.plane_w > 0.0 && cam.plane_h > 0.0 {
                    match e.key().as_str() {
                        "+" | "=" => {
                            zoom_surface_camera_at(surface, ZOOM_STEP, None);
                            return true;
                        }
                        "-" => {
                            zoom_surface_camera_at(surface, 1.0 / ZOOM_STEP, None);
                            return true;
                        }
                        "0" => {
                            let mut st = surface_state(surface);
                            st["fit"] = json!(true);
                            save_surface_state(surface, &st);
                            fit_surface_camera(surface, &mut cam);
                            return true;
                        }
                        "1" => {
                            reset_surface_camera_100(surface);
                            return true;
                        }
                        _ => {}
                    }
                }
            }
        }
        return false;
    }
    if e.key() == "Escape" {
        shut();
        dom::focus("graph-pick-open");
        return true;
    }
    // In the search field and the list; a tab, a button or a select keeps
    // its own keys.
    let within = e
        .target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        .map(|element| element.id())
        .unwrap_or_default();
    if !["graph-pick-search", "graph-pick-list"].contains(&within.as_str()) {
        return false;
    }
    let rows = offered(&pick);
    let Some(last) = rows.len().checked_sub(1) else {
        return false;
    };
    let active = (pick["active"].as_u64().unwrap_or(0) as usize).min(last);
    match e.key().as_str() {
        "ArrowDown" if active == last => {
            // The end of a history that goes on reads the next page.
            if m::yes(&pick["more"]) && !m::yes(&pick["loading"]) {
                spawn_local(async move {
                    fetch(true).await;
                    if offered(&picker()).len() > active + 1 {
                        mark(active + 1);
                    }
                });
            }
            true
        }
        "ArrowDown" => {
            mark(active + 1);
            true
        }
        "ArrowUp" => {
            mark(active.saturating_sub(1));
            true
        }
        "Enter" if !e.shift_key() && !e.ctrl_key() && !e.meta_key() && !e.alt_key() => {
            let target = rows[active]["target"].clone();
            spawn_local(async move {
                event("click", "", "graph-pick-choose", &target, "", false).await;
            });
            true
        }
        _ => false,
    }
}
/// How many failing tests a picture of a change may be told of, and how
/// long a name of one may be.
pub(super) const FAILING: usize = 64;
pub(super) const FAILING_NAME: usize = 300;
/// The failing tests a person pasted: a test, a path or a line of the
/// output of a test run on each line, each once, as many as are taken.
pub fn failing_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || lines.iter().any(|known| known == line) {
            continue;
        }
        lines.push(line.chars().take(FAILING_NAME).collect());
        if lines.len() == FAILING {
            break;
        }
    }
    lines
}
pub(super) fn format_coverage_summary(cov: &V) -> Option<(String, String, &'static str)> {
    if !cov.is_object() {
        return None;
    }
    let fmt_raw = m::text(cov, "format").trim().to_ascii_lowercase();
    let fmt_label = match fmt_raw.as_str() {
        "lcov" => "LCOV",
        "clover" => "Clover",
        _ => "Coverage",
    };
    let covered = cov["covered_lines"].as_u64()?;
    let total = cov["total_lines"].as_u64()?;
    let rate = cov["rate"].as_f64().unwrap_or_else(|| {
        if total == 0 {
            0.0
        } else {
            covered as f64 / total as f64
        }
    });
    let pct = (rate * 100.0).round().clamp(0.0, 100.0) as u64;
    let tone = if total == 0 {
        "neutral"
    } else if pct >= 80 {
        "high"
    } else if pct >= 50 {
        "mid"
    } else {
        "low"
    };
    let label = format!("{fmt_label} {pct}% ({covered}/{total})");
    let title =
        format!("Observed {fmt_label} line coverage: {covered} of {total} lines covered ({pct}%)");
    Some((label, title, tone))
}

pub(super) fn coverage_pill_html(cov: &V) -> String {
    let Some((label, title, tone)) = format_coverage_summary(cov) else {
        return String::new();
    };
    let esc_label = escape(&label);
    let esc_title = escape(&title);
    format!(
        "<span class=\"graph-coverage-pill\" data-tone=\"{tone}\" title=\"{esc_title}\" aria-label=\"Coverage {esc_label}\">{esc_label}</span>"
    )
}

pub(super) fn coverage_provenance_pills_html(item: &V) -> String {
    let mut html = String::new();
    if item["source"].as_str() == Some("coverage") {
        html.push_str(
            "<span class=\"graph-coverage-pill\" data-kind=\"source\" title=\"Discovered directly from test coverage report\" aria-label=\"Discovered from coverage\">from coverage</span>",
        );
    }
    if m::yes(&item["coverage_verified"]) {
        html.push_str(
            "<span class=\"graph-coverage-pill\" data-kind=\"verified\" title=\"Verified hitting this file in test coverage report\" aria-label=\"Coverage verified\">coverage verified</span>",
        );
    }
    html
}

/// The changed files ranked by the tests that fail: a field to paste the
/// failing tests into, and what the server made of them.
pub(super) fn suspects_side(view: &V, state: &V, surface: MapSurface) -> String {
    if !changes(view) {
        return String::new();
    }
    let named = m::array(&state["failing"])
        .into_iter()
        .filter_map(|told| told.as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let suspects = &view["suspects"];
    let mut html = String::from("<h4>Failing tests</h4>");
    if suspects.is_object() {
        let failing = m::array(&suspects["failing"]);
        let rows = m::array(&suspects["rows"]);
        if failing.is_empty() {
            html.push_str("<p class=\"knowledge-note\">None of the names is a test file of this graph. A test is found by its path, or by the name of its file standing as a word of its own.</p>");
        } else if rows.is_empty() {
            html.push_str(&format!(
                "<p class=\"knowledge-note\">No changed file is reached by {}.</p>",
                match failing.len() {
                    1 => "the failing test".to_owned(),
                    many => format!("the {} failing tests", count(many as u64)),
                }
            ));
        } else {
            html.push_str(&format!(
                "<p class=\"knowledge-note\">The changed files that {} reach{}, first those that the failing tests reach and the passing ones do not. The graph stands in for what a test runs: this says where to look first, not what broke.</p><ol class=\"graph-map-rows graph-suspects\">",
                plural(failing.len() as u64, "failing test file", "failing test files"),
                if failing.len() == 1 { "es" } else { "" },
            ));
            for row in &rows {
                let path = m::text(row, "path");
                let folder = parent(&path);
                let (fail, pass, nearest) = (
                    row["failing"].as_u64().unwrap_or(0),
                    row["passing"].as_u64().unwrap_or(0),
                    row["nearest"].as_u64().unwrap_or(0),
                );
                let rank = row["rank"].as_u64().unwrap_or(0);
                let cov_pill = coverage_pill_html(&row["coverage"]);
                let provenance = coverage_provenance_pills_html(row);
                html.push_str(&format!(
                    "<li><button type=\"button\" class=\"graph-map-row\" data-suspect-rank=\"{rank}\" data-action=\"graph-map-node\" data-payload=\"{}\" title=\"{} · select it in the picture\"><span class=\"graph-map-node-suspect\" aria-hidden=\"true\">#{rank}</span><span class=\"graph-map-row-name\">{}</span>{cov_pill}{provenance}<span class=\"graph-map-row-links\">{}{} of {} failing, {} passing · {} from a failing test</span></button></li>",
                    payload(&row["path"]),
                    escape(&path),
                    escape(base(&path)),
                    if folder.is_empty() {
                        String::new()
                    } else {
                        format!("{} · ", escape(folder))
                    },
                    count(fail),
                    count(failing.len() as u64),
                    count(pass),
                    plural(nearest, "link", "links"),
                ));
            }
            html.push_str("</ol>");
            let more = suspects["more"].as_u64().unwrap_or(0);
            if more > 0 {
                html.push_str(&format!(
                    "<p class=\"knowledge-note\">and {} more</p>",
                    count(more)
                ));
            }
        }
        let clear = suspects["clear"].as_u64().unwrap_or(0);
        if clear > 0 && !failing.is_empty() {
            html.push_str(&format!(
                "<p class=\"knowledge-note\">No failing test reaches {}.</p>",
                plural(clear, "other changed file", "other changed files")
            ));
        }
        let unmatched = m::array(&suspects["unmatched"])
            .iter()
            .filter_map(|told| told.as_str().map(escape))
            .collect::<Vec<_>>();
        if !unmatched.is_empty() {
            html.push_str(&format!(
                "<p class=\"knowledge-note\">No test file found for: {}.</p>",
                unmatched.join(", ")
            ));
        }
        if !m::yes(&suspects["complete"]) {
            html.push_str("<p class=\"knowledge-note\">The search stopped before the end: some tests may be missing.</p>");
        }
    } else if named.is_empty() {
        html.push_str("<p class=\"knowledge-note\">Paste the tests that fail to rank the changed files by how likely each is behind them.</p>");
    }
    let asked = |clear: bool| {
        // Keys in the order of their names: a payload reads the same whether
        // or not the map keeps the order keys are put in.
        payload(&if clear {
            json!({"clear": true, "surface": surface.name})
        } else {
            json!({"surface": surface.name})
        })
    };
    html.push_str(&format!(
        "<details class=\"graph-suspects-ask\"{}><summary>{}</summary><textarea id=\"{}-failing\" class=\"graph-suspects-field\" rows=\"4\" spellcheck=\"false\" aria-label=\"Failing tests, one on each line\" placeholder=\"One on each line: a test, its file, or a line of the output of a test run\">{}</textarea><div class=\"graph-suspects-actions\"><button type=\"button\" class=\"button compact\" data-action=\"graph-map-failing\" data-payload=\"{}\">Rank the changes</button>{}</div></details>",
        if named.is_empty() { " open" } else { "" },
        if named.is_empty() {
            "Name the failing tests"
        } else {
            "Change the failing tests"
        },
        surface.id_prefix,
        escape(&named.join("\n")),
        asked(false),
        if named.is_empty() {
            String::new()
        } else {
            format!(
                "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-failing\" data-payload=\"{}\">Clear</button>",
                asked(true)
            )
        },
    ));
    html
}
pub(super) fn test_paths(view: &V) -> String {
    let rows = m::array(&view["test_reach"]);
    if rows.is_empty() {
        if (changes(view) || view["mode"] == "file")
            && view["chosen"].is_object()
            && (m::yes(&view["stale"]) || m::yes(&view["preliminary"]))
            && !m::yes(&view["notests"])
        {
            return "<p class=\"knowledge-note\">Test paths need the graph for this change.</p>"
                .to_owned();
        }
        return String::new();
    }
    let mut html = String::from(
        "<h4>Tests reaching the change</h4><p class=\"knowledge-note\">Candidates from graph links. Review the path before running a test.</p>",
    );
    for row in &rows {
        let tests = m::array(&row["tests"]);
        let total = row["total"].as_u64().unwrap_or(0);
        let path = m::text(row, "path");
        let status = match row["state"].as_str() {
            Some("deleted") => "Deleted file".to_owned(),
            Some("missing") => "File missing from the graph".to_owned(),
            Some("test_file") => "Changed test file".to_owned(),
            Some("partial") => format!(
                "{} · search limit reached",
                plural(total, "candidate", "candidates")
            ),
            _ if tests.is_empty() => "No test path found in the graph".to_owned(),
            _ => plural(total, "candidate", "candidates"),
        };
        let cov_pill = coverage_pill_html(&row["coverage"]);
        let cov_sep = if cov_pill.is_empty() { "" } else { " " };
        html.push_str(&format!(
            "<details><summary>{} · {}{cov_sep}{cov_pill}</summary><p class=\"graph-map-path\">{}</p>",
            escape(base(&path)),
            escape(&status),
            escape(&path),
        ));
        for test in &tests {
            let steps = test["steps"].as_u64().unwrap_or(0);
            let name = m::text(test, "name");
            let file = m::text(test, "file");
            let provenance = coverage_provenance_pills_html(test);
            let prov_sep = if provenance.is_empty() { "" } else { " " };
            html.push_str(&format!(
                "<p class=\"graph-map-path\"><button type=\"button\" class=\"button compact\" data-action=\"graph-map-test-path\" data-payload=\"{}\">{} · {}</button>{}{prov_sep}{provenance}</p>",
                payload(&json!({"from": test["path"], "to": test["to"]})),
                escape(&name),
                plural(steps, "link", "links"),
                if name == file { String::new() } else { format!(" · {}", escape(&file)) },
            ));
        }
        if total > tests.len() as u64 {
            html.push_str(&format!(
                "<p class=\"knowledge-note\">Showing the {} nearest candidates.</p>",
                tests.len()
            ));
        }
        html.push_str("</details>");
    }
    html
}

pub(super) fn layer_color(layer: &V) -> Option<String> {
    let id = layer["color"].as_u64().filter(|id| *id < 32)?;
    Some(layer_accent(id))
}

pub(super) fn architecture_side(view: &V) -> String {
    let architecture = &view["architecture"];
    if !architecture.is_object() {
        return String::new();
    }
    let declared = m::yes(&architecture["declared"]);
    let mode_label = if declared {
        "Declared rules"
    } else {
        "Inferred layers"
    };
    let mut html = format!(
        "<h4>Architecture <span class=\"graph-map-arch-mode\" data-declared=\"{declared}\">{mode_label}</span>{}</h4>",
        confidence_pill_html(view),
    );
    let layers = m::array(&architecture["layers"]);
    if !layers.is_empty() {
        html.push_str("<ul class=\"graph-map-layer-key\">");
        for layer in &layers {
            if let Some(color) = layer_color(layer) {
                let id = escape(&m::text(layer, "id"));
                html.push_str(&format!(
                    "<li data-arch-layer=\"{id}\" style=\"--architecture-color:{color}\"><span aria-hidden=\"true\" style=\"--architecture-color:{color}\"></span>{}</li>",
                    escape(&m::text(layer, "name"))
                ));
            }
        }
        html.push_str("</ul>");
    }
    if view["scope"].is_string() && !layered(view) {
        if let Some(layers_row) = m::array(&view["patterns"]["kinds"])
            .into_iter()
            .find(|kind| kind["id"] == "layers")
            .and_then(|kind| m::array(&kind["instances"]).into_iter().next())
        {
            html.push_str(&format!(
                "<button type=\"button\" class=\"button compact graph-map-arch-open\" data-action=\"graph-map-pattern\" data-payload=\"{}\">Inspect layers diagram</button>",
                payload(&layers_row["id"]),
            ));
        }
    }
    html.push_str(&pair_rows(
        if declared {
            "Rule violations"
        } else {
            "Against inferred layer order"
        },
        &architecture["violations"],
        "No forbidden dependencies found here.",
        true,
    ));
    let total = architecture["reciprocal_total"].as_u64().unwrap_or(0);
    if total > 0 {
        html.push_str(&format!(
            "<h4>Mutual layer dependencies <span class=\"graph-map-total\">{}</span></h4><p class=\"knowledge-note\">The layers depend on each other through the links shown here.</p><div class=\"graph-map-arch-pairs\">",
            count(total)
        ));
        for pair in m::array(&architecture["reciprocal"]) {
            let from_name = escape(&m::text(&pair["from"], "name"));
            let to_name = escape(&m::text(&pair["to"], "name"));
            let from_color = layer_color(&pair["from"])
                .map(|c| format!(" style=\"--architecture-color:{c}\""))
                .unwrap_or_default();
            let to_color = layer_color(&pair["to"])
                .map(|c| format!(" style=\"--architecture-color:{c}\""))
                .unwrap_or_default();
            html.push_str(&format!(
                "<div class=\"graph-map-arch-pair\"><div class=\"graph-map-arch-pair-head\"><span class=\"graph-map-arch-tag\"{from_color}>{from_name}</span><span aria-hidden=\"true\">⇄</span><span class=\"graph-map-arch-tag\"{to_color}>{to_name}</span></div><div class=\"graph-map-arch-pair-flows\"><span>→ {}</span><span>← {}</span></div></div>",
                weight(
                    pair["forward"].as_u64().unwrap_or(0),
                    pair["forward_guessed"].as_u64().unwrap_or(0)
                ),
                weight(
                    pair["reverse"].as_u64().unwrap_or(0),
                    pair["reverse_guessed"].as_u64().unwrap_or(0)
                ),
            ));
        }
        html.push_str("</div>");
    }
    html.push_str(&architecture_cut_hints_html(view));
    html
}

pub(super) fn scenario_button(view: &V) -> String {
    let focus = &view["focus"];
    let place = if focus["path"].is_string() {
        focus
    } else {
        &view["file"]
    };
    let Some(path) = place["path"].as_str().filter(|path| !path.is_empty()) else {
        return String::new();
    };
    if m::yes(&place["folder"]) || patterned(view) || cycles(view) || at_base(view) {
        return String::new();
    }
    format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-scenario\" data-payload=\"{}\">Scenario slice</button>",
        payload(&json!(path))
    )
}

pub(super) fn scenario_side(view: &V) -> String {
    let slice = &view["scenario"];
    let mut html = String::from(
        "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-scenario-back\">Back to map</button>",
    );
    if m::yes(&slice["missing"]) {
        html.push_str(
            "<p class=\"knowledge-note\">The starting point is absent from the current graph.</p>",
        );
        return html;
    }
    let drawn = m::array(&view["nodes"]).len().saturating_sub(1);
    let total = slice["reachable"].as_u64().unwrap_or(0);
    html.push_str(&format!("<p class=\"knowledge-note\">{} on the map. The list uses the shortest distance from the starting point. Filter by name or file path.</p>", plural(drawn as u64, "dependency", "dependencies")));
    if total == 0 {
        html.push_str("<p class=\"knowledge-note\">The graph found no outgoing dependencies.</p>");
        return html;
    }
    let matching = slice["matching"].as_u64().unwrap_or(0);
    let skip = slice["skip"].as_u64().unwrap_or(0);
    let rows = m::array(&slice["rows"]);
    if rows.is_empty() {
        html.push_str("<p class=\"knowledge-note\">No dependencies match the filter.</p>");
        return html;
    }
    html.push_str(&format!(
        "<h4>Reachable places · {matching}</h4><ul class=\"graph-map-rows\">"
    ));
    for row in &rows {
        let file = row["file"]
            .as_str()
            .unwrap_or_else(|| row["path"].as_str().unwrap_or(""));
        html.push_str(&format!(
            "<li><button type=\"button\" class=\"graph-map-row\" data-action=\"graph-map-scenario-open\" data-payload=\"{}\"><span class=\"graph-map-row-name\">{}</span><span class=\"graph-map-row-links\">{} · {}</span></button></li>",
            payload(&json!({"file":file,"path":row["path"],"symbol":row["symbol"]})),
            escape(&m::text(row, "name")), escape(file),
            plural(row["steps"].as_u64().unwrap_or(0), "step", "steps")
        ));
    }
    html.push_str("</ul>");
    let end = skip + rows.len() as u64;
    if skip > 0 || end < matching {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">{}–{end} of {matching}</p>",
            skip + 1
        ));
        for (show, label, offset) in [
            (skip > 0, "Previous", skip.saturating_sub(40)),
            (end < matching, "Next", end),
        ] {
            if show {
                html.push_str(&format!("<button type=\"button\" class=\"button compact\" data-action=\"graph-map-scenario-page\" data-payload=\"{offset}\">{label}</button>"));
            }
        }
    }
    html
}

pub(super) fn without_incoming_side(view: &V) -> String {
    let report = &view["without_incoming"];
    if !report.is_object() {
        return String::new();
    }
    let mut html = String::from(
        "<h4>No incoming links</h4><p class=\"knowledge-note\">Review before removing. Entry points, configuration, dynamic calls and external callers can be missing from the graph. The active link and test filters affect this list.</p>",
    );
    for (key, total, title) in [
        (
            "files",
            "file_total",
            "Files without links from other files",
        ),
        ("symbols", "symbol_total", "Symbols without incoming links"),
    ] {
        let total = report[total].as_u64().unwrap_or(0);
        let rows = m::array(&report[key]);
        html.push_str(&format!("<h4>{title} · {}</h4>", count(total)));
        if rows.is_empty() {
            continue;
        }
        html.push_str("<ul class=\"graph-map-rows\">");
        for row in &rows {
            let path = m::text(row, "path");
            let symbol = key == "symbols";
            let (action, data, label) = if symbol {
                (
                    "graph-map-go",
                    json!({"open":row["file"],"path":path}),
                    m::text(row, "name"),
                )
            } else {
                ("graph-map-open", json!(path), path)
            };
            let where_found = if symbol {
                let mut file = m::text(row, "file");
                if let Some(line) = row["line"].as_u64().filter(|line| *line > 0) {
                    file.push_str(&format!(":{line}"));
                }
                format!(
                    "<span class=\"graph-map-row-links\">{}</span>",
                    escape(&file)
                )
            } else {
                String::new()
            };
            html.push_str(&format!(
                "<li><button type=\"button\" class=\"graph-map-row\" data-action=\"{action}\" data-payload=\"{}\"><span class=\"graph-map-row-name\">{}</span>{where_found}</button></li>",
                payload(&data), escape(&label)
            ));
        }
        html.push_str("</ul>");
        if total > rows.len() as u64 {
            html.push_str(&format!(
                "<p class=\"knowledge-note\">Showing {} of {}. Filter by name or path to find more.</p>",
                rows.len(), count(total)
            ));
        }
    }
    html
}

pub(super) fn modules_side(view: &V) -> String {
    if view["children"] == 0 {
        return "<p class=\"knowledge-note\">This graph has no Graphify communities.</p>"
            .to_owned();
    }
    let info = &view["module_info"];
    let mut html = format!(
        "<p class=\"knowledge-note\">{} without a community · {} involving unassigned nodes</p>",
        plural(info["unassigned"].as_u64().unwrap_or(0), "node", "nodes"),
        plural(info["outside"].as_u64().unwrap_or(0), "link", "links")
    );
    let focus = &view["focus"];
    if !focus.is_object() {
        html.push_str(
            "<p class=\"knowledge-note\">Select a module to inspect its files and links.</p>",
        );
        return html;
    }
    let number = |key: &str| focus[key].as_u64().unwrap_or(0);
    html.push_str(&format!(
        "<h3>{}</h3><p class=\"knowledge-note\">{} · {} · {}</p>",
        escape(&m::text(focus, "name")),
        plural(number("files"), "file", "files"),
        plural(number("symbols"), "node", "nodes"),
        weight(number("within"), number("inside_guessed")),
    ));
    for (key, title) in [("uses", "Uses modules"), ("used_by", "Used by modules")] {
        let rows = m::array(&focus[key]);
        let total = number(&format!("{key}_total"));
        html.push_str(&format!("<h4>{title} · {total}</h4>"));
        for row in rows {
            html.push_str(&format!(
                "<p><button class=\"button compact\" type=\"button\" data-action=\"graph-map-module-select\" data-payload=\"{}\">{}</button> · {}</p>",
                payload(&row["path"]), escape(&m::text(&row, "name")),
                weight(row["links"].as_u64().unwrap_or(0), row["guessed"].as_u64().unwrap_or(0))
            ));
        }
    }
    html.push_str(&format!(
        "<h4>Internal links · {}</h4>",
        plural(number("internal_total"), "pair", "pairs")
    ));
    let endpoint = |end: &V| {
        let line = end["line"]
            .as_u64()
            .filter(|line| *line > 0)
            .map_or_else(String::new, |line| format!(":{line}"));
        format!(
            "{} ({line_file}{line})",
            escape(&m::text(end, "name")),
            line_file = escape(&m::text(end, "file"))
        )
    };
    for row in m::array(&focus["internal"]) {
        html.push_str(&format!(
            "<p class=\"graph-map-path\">{} → {} · {}</p>",
            endpoint(&row["from"]),
            endpoint(&row["to"]),
            weight(
                row["links"].as_u64().unwrap_or(0),
                row["guessed"].as_u64().unwrap_or(0)
            )
        ));
    }
    html.push_str(&format!("<h4>Files · {}</h4>", number("files")));
    for file in m::array(&focus["members"]) {
        html.push_str(&format!(
            "<p class=\"graph-map-path\"><button class=\"button compact\" type=\"button\" data-action=\"graph-map-module-file\" data-payload=\"{}\">{}</button></p>",
            payload(&file), escape(file.as_str().unwrap_or(""))
        ));
    }
    html
}

pub(super) fn history_side(view: &V) -> String {
    let history = &view["git_history"];
    match history["state"].as_str() {
        Some("unavailable") => {
            return "<p class=\"knowledge-note\">Git history could not be read.</p>".to_owned();
        }
        Some("empty") => {
            return "<p class=\"knowledge-note\">No local Git commits found.</p>".to_owned();
        }
        Some("ready") => {}
        _ => return String::new(),
    }
    let commits = history["commits"].as_u64().unwrap_or(0);
    let skipped = history["skipped"].as_u64().unwrap_or(0);
    let mut html = format!(
        "<h4>Git history</h4><p class=\"knowledge-note\">{} analysed through {}. {} large commits skipped.</p>",
        plural(commits, "commit", "commits"),
        escape(&m::text(history, "revision")),
        count(skipped)
    );
    let rows = |values: &V, field: &str, suffix: &str, total: u64| {
        let values = values.as_array().map(Vec::as_slice).unwrap_or_default();
        let number = |row: &V, key: &str| row[key].as_u64().unwrap_or(0);
        let mut list = String::from("<ul class=\"graph-map-rows\">");
        for row in values {
            let path = m::text(row, "path");
            list.push_str(&format!(
                "<li><button type=\"button\" class=\"graph-map-row\" data-action=\"graph-map-open\" data-payload=\"{}\"><span class=\"graph-map-row-name\">{}</span><span class=\"graph-map-row-links\">{} · {} {suffix}</span></button></li>",
                payload(&row["path"]), escape(&path), plural(number(row, "commits"), "commit", "commits"),
                count(number(row, field)),
            ));
        }
        list.push_str("</ul>");
        if total > values.len() as u64 {
            list.push_str(&format!(
                "<p class=\"knowledge-note\">Showing {} of {}.</p>",
                values.len(),
                count(total)
            ));
        }
        list
    };
    let cochanges = &history["cochanges"];
    if cochanges.is_object() {
        html.push_str("<h4>Frequently changed together</h4>");
        let total = cochanges["total"].as_u64().unwrap_or(0);
        if total == 0 {
            html.push_str("<p class=\"knowledge-note\">No repeated co-changes outside direct graph links.</p>");
        } else {
            html.push_str(&rows(&cochanges["rows"], "together", "together", total));
        }
    } else {
        html.push_str("<p class=\"knowledge-note\">Select a file to see which other files changed with it.</p>");
    }
    let total = history["hot_total"].as_u64().unwrap_or(0);
    html.push_str("<h4>Hotspots</h4>");
    if total == 0 {
        html.push_str("<p class=\"knowledge-note\">No files here with at least two commits and two graph neighbors.</p>");
    } else {
        html.push_str(&rows(&history["hot"], "neighbors", "neighbors", total));
    }
    html
}

pub(super) fn map_attachment(view: &V) -> Option<String> {
    let focus = &view["focus"];
    let place = if focus["path"].is_string() {
        focus
    } else {
        &view["file"]
    };
    let path = place["path"].as_str().filter(|path| !path.is_empty())?;
    let mut lines = vec![
        "Dependency map context".to_owned(),
        format!("Selected place: {}", json!(path)),
    ];
    if let Some(target) = view["target"].as_str().filter(|s| !s.is_empty()) {
        lines.push(format!("Change selection: {}", json!(target)));
    }
    if let Some(commit) = view["built"]["commit"]
        .as_str()
        .or(view["chosen"]["head"]["commit"].as_str())
    {
        lines.push(format!("Graph revision: {}", json!(commit)));
    }
    if let Some(backend) = graph_backend_label(view) {
        lines.push(format!("Graph backend: {backend}"));
    }
    if m::yes(&view["stale"]) || m::yes(&view["preliminary"]) {
        lines.push("The displayed graph was stale or preliminary.".to_owned());
    }
    lines.push(if m::yes(&view["certain"]) {
        "Link selection: links read from code.".to_owned()
    } else {
        "Link selection: includes links inferred by name.".to_owned()
    });
    for (key, label) in [("uses", "Uses"), ("used_by", "Used by")] {
        let rows = m::array(&place[key]);
        if rows.is_empty() {
            continue;
        }
        lines.push(format!("\n{label} (up to 8 shown):"));
        for row in rows.iter().take(8) {
            if let Some(neighbor) = row["path"].as_str() {
                lines.push(format!("- {}", json!(neighbor)));
            }
        }
    }
    let file = place["file"]
        .as_str()
        .or_else(|| {
            m::yes(&place["symbol"])
                .then(|| view["file"]["path"].as_str())
                .flatten()
        })
        .unwrap_or(path);
    if let Some(row) = m::array(&view["test_reach"])
        .into_iter()
        .find(|row| row["path"] == file)
    {
        if row["state"] == "partial" {
            lines.push("The search for test paths reached its limit.".to_owned());
        }
        if let Some((cov_label, _, _)) = format_coverage_summary(&row["coverage"]) {
            lines.push(format!("Observed coverage: {cov_label}"));
        }
        let tests = m::array(&row["tests"]);
        if !tests.is_empty() {
            lines.push("\nTest candidates from graph links:".to_owned());
            for test in tests {
                let mut tags = Vec::new();
                if test["source"].as_str() == Some("coverage") {
                    tags.push("from coverage");
                }
                if m::yes(&test["coverage_verified"]) {
                    tags.push("coverage verified");
                }
                let suffix = if tags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", tags.join(", "))
                };
                lines.push(format!(
                    "- {} ({} links){suffix}",
                    test["path"], test["steps"]
                ));
            }
        } else if row["state"] == "ready" {
            lines.push("No test path found in the graph.".to_owned());
        }
    }
    let text = lines.join("\n");
    Some(text.chars().take(6000).collect())
}

pub(super) fn launch_options(run: &V, comparison: bool) -> String {
    let mut options = if comparison {
        "<option value=\"\">No comparison</option>"
    } else {
        "<option value=\"\">All launches</option>"
    }
    .to_owned();
    let active_launch = m::text(run, "launch");
    for launch in m::array(&run["launches"]) {
        let id = {
            let direct = m::text(&launch, "id");
            if !direct.is_empty() {
                direct
            } else {
                let sid = m::text(&launch, "session_id");
                if !sid.is_empty() {
                    sid
                } else {
                    m::text(&launch, "launch_id")
                }
            }
        };
        if comparison && !id.is_empty() && id == active_launch {
            continue;
        }
        let number = launch["number"]
            .as_u64()
            .or_else(|| launch["launch_number"].as_u64())
            .unwrap_or(0);
        let at = {
            let direct = m::text(&launch, "at");
            if !direct.is_empty() {
                direct
            } else {
                m::text(&launch, "created_at")
            }
        };
        let at = at.get(..16).unwrap_or(&at).replace('T', " ");
        let label = if at.is_empty() {
            format!("Launch {number}")
        } else {
            format!("Launch {number} · {at} UTC")
        };
        options.push_str(&format!(
            "<option value=\"{}\">{}</option>",
            escape(&id),
            escape(&label)
        ));
    }
    options
}

pub fn agent_options_html(service_id: &str, run: &V, selected_agent: &str) -> String {
    let mut items: Vec<(String, String)> = vec![
        (String::new(), "All agents in launch".to_owned()),
        ("contracts".to_owned(), "contracts".to_owned()),
    ];
    let trimmed_service = service_id.trim();
    if !trimmed_service.is_empty() && trimmed_service != "__system__" {
        let svc_agent = format!("service-{trimmed_service}");
        items.push((svc_agent.clone(), svc_agent));
    }
    for extra in m::array(&run["agents"]) {
        let Some(id) = extra.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        if id != "verify" && !items.iter().any(|(k, _)| k == id) {
            items.push((id.to_owned(), id.to_owned()));
        }
    }
    if !items.iter().any(|(k, _)| k == "verify") {
        items.push(("verify".to_owned(), "verify".to_owned()));
    }
    let selected_trimmed = selected_agent.trim();
    if !selected_trimmed.is_empty() && !items.iter().any(|(k, _)| k == selected_trimmed) {
        items.push((selected_trimmed.to_owned(), selected_trimmed.to_owned()));
    }
    let mut options = String::new();
    for (val, label) in items {
        options.push_str(&format!(
            "<option value=\"{}\"{}>{}</option>",
            escape(&val),
            if val == selected_trimmed {
                " selected"
            } else {
                ""
            },
            escape(&label),
        ));
    }
    options
}

pub(super) fn show_launches(view: &V) {
    let scope = surface_scope(MODAL_SURFACE);
    let modal_st = state();
    let target = m::text(&modal_st, "target");
    let is_sysrun = scope.is_system()
        && (target.starts_with("sysrun:")
            || view["run"]["kind"] == "sysrun"
            || view["chosen"]["kind"] == "sysrun");
    let view_launches = m::array(&view["run"]["launches"]);
    let effective_launches = if !view_launches.is_empty() {
        view_launches
    } else if is_sysrun {
        match &scope {
            GraphTargetScope::SystemService { launches, .. } => launches.clone(),
            GraphTargetScope::SingleProject { .. } => Vec::new(),
        }
    } else {
        Vec::new()
    };
    let selected_launch = {
        let from_view = m::text(&view["run"], "launch");
        if !from_view.is_empty() {
            from_view
        } else {
            m::text(&modal_st, "launch")
        }
    };
    let selected_compare = {
        let from_view = m::text(&view["run"], "compare_launch");
        if !from_view.is_empty() {
            from_view
        } else {
            m::text(&modal_st, "compare_launch")
        }
    };
    let effective_run = json!({
        "launches": effective_launches,
        "launch": if selected_launch.is_empty() { V::Null } else { json!(selected_launch) },
        "compare_launch": if selected_compare.is_empty() { V::Null } else { json!(selected_compare) },
        "agents": view["run"]["agents"],
    });
    let launches_len = m::array(&effective_run["launches"]).len();
    let hidden = !is_sysrun && launches_len < 2;
    dom::hidden("graph-map-launches", hidden);
    dom::hidden("graph-map-agent", !is_sysrun);
    dom::hidden("graph-map-agent-field", !is_sysrun);
    if hidden {
        return;
    }
    for (id, key, comparing) in [
        ("graph-map-launch", "launch", false),
        ("graph-map-compare-launch", "compare_launch", true),
    ] {
        html(id, &launch_options(&effective_run, comparing));
        dom::set_value(id, &m::text(&effective_run, key));
    }
    dom::disabled(
        "graph-map-compare-launch",
        launches_len < 2 || !effective_run["launch"].is_string(),
    );
    if is_sysrun {
        let svc = scope.service_id().unwrap_or("");
        let selected_agent = {
            let from_view = m::text(&view["run"], "agent");
            if !from_view.is_empty() {
                from_view
            } else {
                m::text(&modal_st, "agent")
            }
        };
        html(
            "graph-map-agent",
            &agent_options_html(svc, &effective_run, &selected_agent),
        );
        dom::set_value("graph-map-agent", &selected_agent);
    }
}

pub(super) fn shell_event_button(session: &V, event: &V, launch: &str) -> String {
    if !session.is_string() || event.as_u64().is_none() {
        return String::new();
    }
    format!(
        "<button class=\"button compact\" type=\"button\" data-action=\"graph-map-shell-event\" data-payload=\"{}\" aria-label=\"Read via shell{launch}\">Read via shell</button>",
        payload(&json!({"session":session,"event":event})),
    )
}

pub(super) fn shell_reads(view: &V) -> String {
    if view["run"]["comparison"].is_object() {
        return String::new();
    }
    let reads = &view["run"]["shell_reads"];
    let rows = m::array(&reads["rows"]);
    if rows.is_empty() {
        return String::new();
    }
    let mut html = "<section><h4>Shell reads</h4><ul class=\"graph-map-rows\">".to_owned();
    for row in rows {
        html.push_str(&format!(
            "<li><span class=\"graph-map-row-name\">{}</span>{}</li>",
            escape(&m::text(&row, "path")),
            shell_event_button(&view["run"]["session"], &row["event"], "")
        ));
    }
    html.push_str("</ul>");
    if let Some(more) = reads["more"].as_u64().filter(|more| *more > 0) {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">{} more files.</p>",
            count(more)
        ));
    }
    html.push_str("</section>");
    html
}

pub(super) async fn show_shell_event(selection: &V) {
    let Some(session) = selection["session"].as_str() else {
        return;
    };
    let Some(event) = selection["event"].as_u64().filter(|id| *id > 0) else {
        return;
    };
    dom::text("graph-shell-event-command", "");
    dom::text("graph-shell-event-status", "Reading the event…");
    dom::error("graph-shell-event-error", "");
    dom::dialog("graph-shell-event", true);
    let path = format!("/api/sessions/{}?after={}", dom::enc(session), event - 1);
    match api::scoped("graph-shell-event", &path).await {
        Ok((epoch, page)) if m::current("graph-shell-event", epoch) => {
            dom::text("graph-shell-event-status", "");
            if let Some(command) = page["events"][0]["detail"].as_str().filter(|_| {
                page["session"]["id"] == session
                    && page["events"][0]["id"] == event
                    && page["events"][0]["tool"] == "Shell"
                    && page["events"][0]["state"] == "completed"
            }) {
                dom::text(
                    "graph-shell-event-status",
                    &format!("Event {event} · completed shell command"),
                );
                dom::text("graph-shell-event-command", command);
            } else {
                dom::error(
                    "graph-shell-event-error",
                    "The recorded shell event is unavailable.",
                );
            }
        }
        Err(error) if !error.aborted => {
            dom::text("graph-shell-event-status", "");
            dom::error("graph-shell-event-error", &error.message);
        }
        _ => {}
    }
}

pub(super) fn run_comparison(view: &V) -> String {
    let comparison = &view["run"]["comparison"];
    if !comparison.is_object() {
        return String::new();
    }
    let number = |key: &str| count(comparison[key].as_u64().unwrap_or(0));
    let mut html = format!(
        "<section><h4>Dependent files across launches</h4><p class=\"knowledge-note\">{} newly opened or edited · {} with no recorded action in either launch. Shell reads are attributed where the command identifies a file.</p>",
        number("newly_touched"),
        number("gaps"),
    );
    let rows = m::array(&comparison["rows"]);
    if rows.is_empty() {
        html.push_str(
            "<p class=\"knowledge-note\">The graph found no files that use these changes here.</p>",
        );
    } else {
        html.push_str("<ul class=\"graph-map-rows\">");
        for row in rows {
            let evidence = |opened, edited, shell| match (opened, edited, shell) {
                (_, true, true) => "Read via shell and edited",
                (_, false, true) => "Read via shell",
                (true, true, false) => "Opened and edited",
                (true, false, false) => "Opened",
                (false, true, false) => "Edited",
                (false, false, false) => "No recorded action",
            };
            let path = m::text(&row, "path");
            html.push_str(&format!(
                "<li><button type=\"button\" class=\"graph-map-row\" data-action=\"graph-map-open\" data-payload=\"{}\" title=\"{}\"><span class=\"graph-map-row-name\">{}</span><span class=\"graph-map-row-links\">Map launch · {}<br>Comparison · {}</span></button><div>{}{}</div></li>",
                payload(&row["path"]), escape(&path), escape(&path),
                evidence(m::yes(&row["opened"]), false, row["shell_read"].is_number()),
                evidence(m::yes(&row["followup_opened"]), m::yes(&row["followup_edited"]), row["followup_shell_read"].is_number()),
                shell_event_button(&view["run"]["session"], &row["shell_read"], " in map launch"),
                shell_event_button(&view["run"]["session"], &row["followup_shell_read"], " in comparison launch"),
            ));
        }
        html.push_str("</ul>");
    }
    if comparison["more"].as_u64().unwrap_or(0) > 0 {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">{} more dependent files.</p>",
            number("more")
        ));
    }
    html.push_str("</section>");
    html
}
