//! Dependency map of the project's code graph: one folder at a time, what
//! uses what. Whatever depends on something stands to its left, so entry
//! points gather on the left edge and foundations on the right. A file opens
//! in turn to its classes, methods and functions and the files around them.
use crate::{api, dom, model as m};
use serde_json::{json, Value as V};
use std::collections::BTreeMap;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
const BOX_WIDTH: f64 = 164.0;
const BOX_HEIGHT: f64 = 30.0;
/// A link that skips a layer passes it in a thin slot of its own.
const BEND_HEIGHT: f64 = 2.0;
const BEND_GAP: f64 = 5.0;
const COLUMN_GAP: f64 = 72.0;
const ROW_GAP: f64 = 10.0;
const MARGIN: f64 = 12.0;
const CAPTION: f64 = 20.0;
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
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub nodes: Vec<Placed>,
    pub wires: Vec<Wire>,
    pub width: f64,
    pub height: f64,
    /// Where the grid of unlinked nodes begins, when there is one under a map.
    pub apart: Option<f64>,
}
/// Groups of nodes that all reach each other, as a group number per node.
fn groups(count: usize, out: &[Vec<usize>]) -> Vec<usize> {
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
fn order(count: usize, linked: &[bool], links: &BTreeMap<(usize, usize), u64>) -> Vec<usize> {
    let mut open = linked.to_vec();
    let flow = |open: &[bool], node: usize| {
        let (mut out, mut into) = (0u64, 0u64);
        for ((from, to), weight) in links {
            if open[*from] && open[*to] {
                if *from == node {
                    out += weight;
                }
                if *to == node {
                    into += weight;
                }
            }
        }
        (out, into)
    };
    let (mut first, mut last) = (vec![], vec![]);
    while open.iter().any(|open| *open) {
        let mut moved = false;
        for node in 0..count {
            if open[node] && flow(&open, node).0 == 0 {
                open[node] = false;
                last.push(node);
                moved = true;
            }
        }
        for node in 0..count {
            if open[node] && flow(&open, node).1 == 0 {
                open[node] = false;
                first.push(node);
                moved = true;
            }
        }
        if moved {
            continue;
        }
        let best = (0..count).filter(|node| open[*node]).max_by_key(|node| {
            let (out, into) = flow(&open, *node);
            (out as i128 - into as i128, std::cmp::Reverse(*node))
        });
        if let Some(best) = best {
            open[best] = false;
            first.push(best);
        }
    }
    last.reverse();
    first.extend(last);
    first
}
fn curve(path: &mut String, from: (f64, f64), to: (f64, f64)) {
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
/// Places `count` nodes and routes `edges` (user, used, weight) between them.
/// Layers run left to right along the links; a link that skips a layer bends
/// through a slot of its own there, so it does not cross a box. A cycle cannot
/// be laid out that way, so its weakest links run back, right to left.
pub fn plan(count: usize, edges: &[(usize, usize, u64)]) -> Plan {
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
    let mut layer = vec![0usize; count];
    for node in &order {
        for (near, far, ..) in &wired {
            if near == node {
                layer[*far] = layer[*far].max(layer[*near] + 1);
            }
        }
    }
    // Longest paths put everything nobody uses in the first layer, far from
    // what it uses. Each node moves right, up to the layer before the nearest
    // thing it uses, so its links stay short and cross fewer layers.
    for node in order.iter().rev() {
        let nearest = wired
            .iter()
            .filter(|(near, ..)| near == node)
            .map(|(_, far, ..)| layer[*far])
            .min();
        if let Some(nearest) = nearest {
            layer[*node] = layer[*node].max(nearest - 1);
        }
    }
    let layers = order
        .iter()
        .map(|node| layer[*node])
        .max()
        .map_or(0, |last| last + 1);
    // Items of a layer: nodes first, then one bend per link passing through.
    let mut columns: Vec<Vec<usize>> = vec![vec![]; layers];
    for node in (0..count).filter(|node| linked[*node]) {
        columns[layer[node]].push(node);
    }
    let mut chains: Vec<Vec<usize>> = vec![];
    let mut heights = vec![BOX_HEIGHT; count];
    let (mut above, mut below): (Vec<Vec<usize>>, Vec<Vec<usize>>) =
        (vec![vec![]; count], vec![vec![]; count]);
    for (near, far, ..) in &wired {
        let mut chain = vec![*near];
        for column in columns.iter_mut().take(layer[*far]).skip(layer[*near] + 1) {
            let bend = heights.len();
            heights.push(BEND_HEIGHT);
            above.push(vec![]);
            below.push(vec![]);
            column.push(bend);
            chain.push(bend);
        }
        chain.push(*far);
        for pair in chain.windows(2) {
            below[pair[0]].push(pair[1]);
            above[pair[1]].push(pair[0]);
        }
        chains.push(chain);
    }
    // Each item moves towards the average place of its neighbours in the
    // layer before it, then after it; a few rounds remove most crossings.
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
                    let links = &near[*item];
                    let middle = if links.is_empty() {
                        place[*item]
                    } else {
                        links.iter().map(|other| place[*other]).sum::<f64>() / links.len() as f64
                    };
                    (middle, place[*item], *item)
                })
                .collect::<Vec<_>>();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
            columns[at] = keyed.into_iter().map(|(_, _, item)| item).collect();
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
    let mut top = vec![0.0; heights.len()];
    let mut tall = vec![0.0; layers];
    for (at, column) in columns.iter().enumerate() {
        let mut y = 0.0;
        for (index, item) in column.iter().enumerate() {
            if index > 0 {
                y += gap(column[index - 1], *item);
            }
            top[*item] = y;
            y += heights[*item];
        }
        tall[at] = y;
    }
    let tallest = tall.iter().copied().fold(0.0, f64::max);
    for (at, column) in columns.iter().enumerate() {
        for item in column {
            top[*item] += MARGIN + (tallest - tall[at]) / 2.0;
        }
    }
    let left = |at: usize| MARGIN + at as f64 * (BOX_WIDTH + COLUMN_GAP);
    let layered_width = if layers == 0 {
        0.0
    } else {
        left(layers - 1) + BOX_WIDTH + MARGIN
    };
    let loose = (0..count).filter(|node| !linked[*node]).collect::<Vec<_>>();
    let per_row = (((layered_width - MARGIN) / (BOX_WIDTH + ROW_GAP)).floor() as usize).max(4);
    let apart = (layers > 0 && !loose.is_empty()).then_some(MARGIN + tallest + 2.0 * MARGIN);
    let grid_top = apart.map_or(MARGIN, |apart| apart + CAPTION);
    let mut nodes = vec![];
    for node in 0..count {
        nodes.push(if linked[node] {
            Placed {
                x: left(layer[node]),
                y: top[node],
                layer: Some(layer[node]),
                cycle: size[group[node]] > 1,
            }
        } else {
            let at = loose.iter().position(|other| *other == node).unwrap_or(0);
            Placed {
                x: MARGIN + (at % per_row) as f64 * (BOX_WIDTH + ROW_GAP),
                y: grid_top + (at / per_row) as f64 * (BOX_HEIGHT + ROW_GAP),
                layer: None,
                cycle: false,
            }
        });
    }
    let mut wires = vec![];
    for (chain, (near, _, back, pair)) in chains.iter().zip(&wired) {
        let edge = edges
            .iter()
            .position(|(from, to, _)| (*from, *to) == *pair)
            .unwrap_or(0);
        // The stops of the link from left to right; a slot is crossed straight.
        let mut stops = vec![];
        for (step, item) in chain.iter().enumerate() {
            let (x, y) = (left(layer[*near] + step), top[*item] + heights[*item] / 2.0);
            if step > 0 {
                stops.push(((x, y), false));
            }
            if step + 1 < chain.len() {
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
        });
    }
    let rows = loose.len().div_ceil(per_row);
    let grid_width = MARGIN + loose.len().min(per_row) as f64 * (BOX_WIDTH + ROW_GAP);
    Plan {
        nodes,
        wires,
        width: layered_width.max(grid_width).max(BOX_WIDTH + 2.0 * MARGIN),
        height: if loose.is_empty() {
            tallest + 2.0 * MARGIN
        } else {
            grid_top + rows as f64 * (BOX_HEIGHT + ROW_GAP)
        }
        .max(BOX_HEIGHT + 2.0 * MARGIN),
        apart,
    }
}
fn count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}
fn plural(value: u64, one: &str, many: &str) -> String {
    format!("{} {}", count(value), if value == 1 { one } else { many })
}
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for letter in text.chars() {
        match letter {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}
fn payload(value: &V) -> String {
    escape(&value.to_string())
}
/// How a link reads: how many, and how many of them the tool only inferred.
fn weight(links: u64, guessed: u64) -> String {
    match guessed {
        0 => plural(links, "link", "links"),
        guessed if guessed == links => format!("{} guessed", plural(links, "link", "links")),
        guessed => format!(
            "{}, {} guessed",
            plural(links, "link", "links"),
            count(guessed)
        ),
    }
}
fn changes(view: &V) -> bool {
    view["mode"] == "changes"
}
/// A file opened to its classes, methods and functions.
fn symbols(view: &V) -> bool {
    view["mode"] == "file"
}
/// The letter a symbol's box carries for what it is. The file's own node
/// and a name the graph says nothing more about carry none.
fn letter(kind: &str) -> &'static str {
    match kind {
        "type" => "T",
        "method" => "m",
        "function" => "f",
        "case" => "c",
        "heading" => "#",
        _ => "",
    }
}
/// What a symbol is, in words: its kind and what holds it.
fn what(symbol: &V) -> String {
    let (word, joined) = match symbol["kind"].as_str().unwrap_or("") {
        "file" => {
            return "the file itself, for what belongs to no symbol in it, such as its imports"
                .to_owned()
        }
        "type" => ("type", "in"),
        "method" => ("method", "of"),
        "function" => ("function", "in"),
        "case" => ("case", "of"),
        "heading" => ("heading", "under"),
        _ => ("name (a constant, a field or a plain type)", "in"),
    };
    match symbol["owner"].as_str() {
        Some(owner) => format!("{word} {joined} {owner}"),
        None => word.to_owned(),
    }
}
fn capital(text: &str) -> String {
    let mut letters = text.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}
fn base(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
/// Git's letter for what happened to a file, in words.
fn happened(status: &str) -> &'static str {
    match status {
        "A" => "added",
        "M" => "modified",
        "R" => "renamed",
        "C" => "copied",
        "T" => "changed in kind",
        "U" => "in conflict",
        _ => "changed",
    }
}
/// What the Show list offers: the whole project and the checkout's own
/// work. A commit, a pull request or a branch chosen in the picker stays in
/// the list while it is the one shown.
pub fn targets(place: &V, chosen: &str) -> String {
    let option = |value: &str, label: &str| {
        format!(
            "<option value=\"{}\"{}>{}</option>",
            escape(value),
            if value == chosen { " selected" } else { "" },
            escape(label)
        )
    };
    let mut html = option("", "The whole project");
    if !m::yes(&place["git"]) {
        return html;
    }
    let branch = place["branch"].as_str().unwrap_or("this checkout");
    let against = place["against"].as_str().or(place["base"].as_str());
    html.push_str(&option(
        "work",
        &match (against, place["pull"].as_u64()) {
            (Some(base), Some(pull)) => {
                format!("Current work: {branch} against {base} (pull request #{pull})")
            }
            (Some(base), None) => format!("Current work: {branch} against {base}"),
            (None, _) => format!("Current work: {branch}"),
        },
    ));
    html.push_str(&option("uncommitted", "Uncommitted changes only"));
    if !["", "work", "uncommitted"].contains(&chosen) {
        let label = match chosen.split_once(':') {
            Some(("pr", number)) => m::array(&place["pulls"])
                .iter()
                .find(|pull| pull["number"].as_u64() == number.parse().ok())
                .map_or_else(
                    || format!("Pull request #{number}"),
                    |pull| format!("#{number} {}", m::text(pull, "title")),
                ),
            Some(("commit", hash)) => {
                format!("Commit {}", hash.chars().take(8).collect::<String>())
            }
            Some((_, name)) => name.to_owned(),
            None => chosen.to_owned(),
        };
        html.push_str(&option(chosen, &label));
    }
    html
}
/// What a typed text names: a short number, with or without `#`, is a pull
/// request; anything else is a branch, a tag or a commit.
pub fn typed(text: &str) -> Option<String> {
    let text = text.trim();
    let digits = text.strip_prefix('#').unwrap_or(text);
    let number = !digits.is_empty()
        && !digits.starts_with('0')
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && (digits.len() <= 6 || text.starts_with('#'))
        && digits.len() <= 9;
    if number {
        return Some(format!("pr:{digits}"));
    }
    (!text.is_empty()
        && text.len() <= 200
        && !text.starts_with(['-', '#'])
        && !text
            .chars()
            .any(|letter| letter.is_whitespace() || letter.is_control()))
    .then(|| format!("ref:{text}"))
}
/// The two sides of the chosen change, in one line.
pub fn chosen(view: &V) -> String {
    let chosen = &view["chosen"];
    if !chosen.is_object() {
        return String::new();
    }
    let side = |side: &V| {
        side.is_object().then(|| {
            format!(
                "{} at {}",
                m::text(side, "name"),
                m::text(side, "commit").chars().take(8).collect::<String>()
            )
        })
    };
    let (head, base) = (side(&chosen["head"]), side(&chosen["base"]));
    let name = m::text(chosen, "name");
    let text = match (chosen["kind"].as_str(), head, base) {
        (Some("uncommitted"), ..) => format!("What is not committed on {name}"),
        (Some("work"), Some(head), Some(base)) => {
            format!("{head} and what is not committed, against {base}")
        }
        (Some("work"), ..) => format!("{name} and what is not committed"),
        // One commit of the history: its own change, against its parent.
        (Some("commit"), ..) => format!(
            "commit {}",
            m::text(&chosen["head"], "commit")
                .chars()
                .take(8)
                .collect::<String>()
        ),
        (_, Some(head), Some(base)) => format!("{head} against {base}"),
        (_, Some(head), None) => head,
        _ => name,
    };
    match chosen["title"].as_str() {
        Some(title) => format!("{title} · {text}"),
        None => text,
    }
}
/// The sentence above the map: what the graph holds and how sure it is, or,
/// for a change, how much changed and how far it reaches.
pub fn summary(view: &V) -> String {
    if symbols(view) {
        let file = &view["file"];
        let number = |key: &str| file[key].as_u64().unwrap_or(0);
        let held = [
            ("types", "type", "types"),
            ("methods", "method", "methods"),
            ("functions", "function", "functions"),
            ("headings", "heading", "headings"),
            ("others", "other", "others"),
        ]
        .iter()
        .filter(|(key, ..)| number(key) > 0)
        .map(|(key, one, many)| plural(number(key), one, many))
        .collect::<Vec<_>>();
        let mut text = m::text(file, "name");
        if let Some(status) = file["status"].as_str() {
            text.push_str(&format!(" ({})", happened(status)));
        }
        text.push_str(&if held.is_empty() {
            " · the graph names nothing inside it".to_owned()
        } else {
            format!(
                " · {} · {} between them",
                held.join(", "),
                plural(number("inside"), "link", "links")
            )
        });
        text.push_str(&match number("uses") {
            0 => " · uses no other file".to_owned(),
            used => format!(" · uses {}", plural(used, "other file", "other files")),
        });
        text.push_str(&match number("used_by") {
            0 => " · no other file uses it".to_owned(),
            1 => " · 1 other file uses it".to_owned(),
            users => format!(" · {} other files use it", count(users)),
        });
        return text;
    }
    if changes(view) {
        let number = |key: &str| view["change"][key].as_u64().unwrap_or(0);
        let mut text = plural(number("files"), "changed file", "changed files");
        if number("graphed") < number("files") {
            text.push_str(&format!(", {} in the graph", count(number("graphed"))));
        }
        if number("here") < number("graphed") {
            text.push_str(&format!(", {} in this folder", count(number("here"))));
        }
        text.push_str(&match number("affected") {
            0 => " · no other file uses them".to_owned(),
            1 => " · 1 other file uses them".to_owned(),
            users => format!(" · {} other files use them", count(users)),
        });
        text.push_str(&match number("needed") {
            0 => " · they use no other file".to_owned(),
            used => format!(" · they use {}", plural(used, "other file", "other files")),
        });
        if number("removed") > 0 {
            text.push_str(&format!(" · {} deleted", count(number("removed"))));
        }
        return text;
    }
    let totals = &view["totals"];
    let number = |key: &str| totals[key].as_u64().unwrap_or(0);
    let mut text = format!(
        "{} · {} between files: {} read from the code, {} guessed by name",
        plural(number("files"), "file", "files"),
        plural(number("links"), "link", "links"),
        count(number("read")),
        count(number("guessed")),
    );
    if number("unplaced") > 0 {
        text.push_str(&format!(
            " · {} skipped, their file is unclear",
            count(number("unplaced"))
        ));
    }
    text
}
/// What the map of a change says first: the notes of the change, and
/// whether the graph is behind the files.
fn context(view: &V) -> Vec<String> {
    let mut parts = m::array(&view["chosen"]["notes"])
        .iter()
        .filter_map(|note| note.as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    if m::yes(&view["stale"]) {
        parts.push(
            if m::yes(&view["rebuilding"]) {
                "The graph is older than the files: it is being rebuilt and the map will follow."
            } else {
                "The graph is older than the files, so the links of edited files may be out of date. Rebuild the graph to refresh them."
            }
            .to_owned(),
        );
    }
    parts
}
/// What an opened file leaves out or has nothing to show for.
/// How far a change reached into an opened file: which of its symbols it
/// touched, or why that is not told.
fn reach(view: &V) -> Option<String> {
    let file = &view["file"];
    file["status"].as_str()?;
    // A file that holds no symbol has none to mark.
    if file["symbols"].as_u64().unwrap_or(0) == 0 {
        return None;
    }
    let touched = &file["touched"];
    if !touched.is_object() {
        return Some(
            if m::yes(&view["stale"]) || m::yes(&view["preliminary"]) {
                "The symbols the change touched are marked once the graph is of these files."
            } else {
                "Which symbols the change touched could not be told from the file."
            }
            .to_owned(),
        );
    }
    if matches!(file["status"].as_str(), Some("A" | "C")) {
        return Some(
            "The whole file is new in this change, so every symbol in it is added.".to_owned(),
        );
    }
    let number = |key: &str| touched[key].as_u64().unwrap_or(0);
    let outside = m::yes(&touched["top"]);
    let mut text = match number("symbols") {
        0 if outside => "The change touched no symbol here, only lines outside them.".to_owned(),
        0 => "The change touched no line of a symbol here.".to_owned(),
        symbols => {
            let kinds = [("added", "added"), ("modified", "modified")]
                .iter()
                .filter(|(key, _)| number(key) > 0)
                .map(|(key, word)| format!("{} {word}", count(number(key))))
                .collect::<Vec<_>>();
            format!(
                "The change touched {} of {}: {}{}.",
                count(symbols),
                plural(file["symbols"].as_u64().unwrap_or(0), "symbol", "symbols"),
                kinds.join(", "),
                if outside {
                    ", and lines outside them"
                } else {
                    ""
                }
            )
        }
    };
    match number("between") {
        0 => {}
        1 => text.push_str(" Lines were also removed between symbols in 1 place."),
        places => text.push_str(&format!(
            " Lines were also removed between symbols in {} places.",
            count(places)
        )),
    }
    Some(text)
}
fn file_note(view: &V) -> String {
    let number = |value: &V| value.as_u64().unwrap_or(0);
    let shown = m::array(&view["nodes"])
        .iter()
        .filter(|node| m::yes(&node["symbol"]))
        .count() as u64;
    let drawn = m::array(&view["edges"]).len() as u64;
    let mut parts = context(view);
    parts.extend(reach(view));
    if number(&view["children"]) == 0 {
        parts.push("The graph names nothing inside this file.".to_owned());
    } else if number(&view["matching"]) == 0 {
        parts.push("No name in this file matches the filter.".to_owned());
    } else if shown < number(&view["matching"]) {
        parts.push(format!(
            "Showing the {} most linked of {} symbols in this file; filter by name to reach the rest.",
            count(shown),
            count(number(&view["matching"]))
        ));
    }
    let around = &view["around"];
    if number(&around["shown"]) < number(&around["files"]) {
        parts.push(format!(
            "Showing the {} most linked of {} files around them.",
            count(number(&around["shown"])),
            count(number(&around["files"]))
        ));
    }
    if shown > 0 && drawn == 0 {
        parts.push(
            if m::yes(&view["certain"]) {
                "No link read from the code touches these."
            } else {
                "Nothing is linked to these."
            }
            .to_owned(),
        );
    } else if drawn > 0 && drawn < number(&view["between"]) {
        parts.push(format!(
            "Drawing the {} strongest of {} links.",
            count(drawn),
            count(number(&view["between"]))
        ));
    }
    parts.join(" ")
}
fn change_note(view: &V) -> String {
    let change = &view["change"];
    let number = |value: &V| value.as_u64().unwrap_or(0);
    let nodes = m::array(&view["nodes"]);
    let around = nodes
        .iter()
        .filter(|node| number(&node["changed"]) == 0)
        .count() as u64;
    let mut parts = context(view);
    if number(&change["files"]) == 0 {
        parts.push("Nothing is changed here.".to_owned());
    } else if number(&change["graphed"]) == 0 {
        parts.push(
            "None of the changed files is in the graph: the tool found no code in them.".to_owned(),
        );
    } else {
        if !m::yes(&change["single"]) {
            parts.push(
                "The change is too wide to draw file by file, so it is drawn by folder. Select a folder twice to open it."
                    .to_owned(),
            );
        }
        match number(&change["hidden"]) {
            0 => {}
            1 => parts.push("1 more place with changes is not drawn.".to_owned()),
            hidden => parts.push(format!(
                "{} more places with changes are not drawn.",
                count(hidden)
            )),
        }
        if number(&view["children"]) > 0 && number(&view["matching"]) == 0 {
            parts.push("No linked file matches the filter.".to_owned());
        } else if around < number(&view["matching"]) {
            parts.push(format!(
                "Showing the {} most linked of {} files around the change; filter by name to reach the rest.",
                count(around),
                count(number(&view["matching"]))
            ));
        }
        let drawn = m::array(&view["edges"]).len() as u64;
        if !nodes.is_empty() && drawn == 0 {
            parts.push(if m::yes(&view["certain"]) {
                "No link read from the code touches the change.".to_owned()
            } else {
                "Nothing links the change to other files.".to_owned()
            });
        } else if drawn < number(&view["between"]) {
            parts.push(format!(
                "Drawing the {} strongest of {} links.",
                count(drawn),
                count(number(&view["between"]))
            ));
        }
    }
    parts.join(" ")
}
/// What the map leaves out or has nothing to show for, if anything.
pub fn note(view: &V) -> String {
    if symbols(view) {
        return file_note(view);
    }
    if changes(view) {
        return change_note(view);
    }
    let number = |key: &str| view[key].as_u64().unwrap_or(0);
    let shown = m::array(&view["nodes"]).len() as u64;
    let drawn = m::array(&view["edges"]).len() as u64;
    let mut parts = vec![];
    if number("children") == 0 {
        parts.push("The graph names no files here.".to_owned());
    } else if number("matching") == 0 {
        parts.push("No name here matches the filter.".to_owned());
    } else if shown < number("matching") {
        parts.push(format!(
            "Showing the {} most connected of {} here; filter by name to reach the rest.",
            count(shown),
            count(number("matching"))
        ));
    }
    if shown > 0 && drawn == 0 {
        parts.push(if m::yes(&view["certain"]) {
            "No link read from the code connects these.".to_owned()
        } else {
            "Nothing here is linked to anything else here.".to_owned()
        });
    } else if drawn > 0 && drawn < number("between") {
        parts.push(if m::yes(&view["backbone"]) {
            format!(
                "Too many links to draw at once: showing each one's 3 strongest, {} of {}. Select one to list all of its links.",
                count(drawn),
                count(number("between"))
            )
        } else {
            format!(
                "Drawing the {} strongest of {} links.",
                count(drawn),
                count(number("between"))
            )
        });
    }
    parts.join(" ")
}
/// Which a box does more through the links drawn here, in five even steps
/// from only using to only being used. A box no link here touches leans
/// neither way.
fn leaning(uses: u64, used: u64) -> &'static str {
    // Wide enough that the largest counts still compare as they are.
    let (fifths, whole) = (u128::from(used) * 5, u128::from(uses) + u128::from(used));
    if whole == 0 {
        ""
    } else if fifths <= whole {
        "uses"
    } else if fifths < whole * 2 {
        "uses-more"
    } else if fifths <= whole * 3 {
        "even"
    } else if fifths < whole * 4 {
        "used-more"
    } else {
        "used"
    }
}
/// The map itself: links as curves underneath, one button per folder or
/// file, or, in an opened file, per symbol and per file around. Colour tells
/// the direction: a link runs from the colour of using to the colour of being
/// used, and a box is tinted by which of the two it does more here.
pub fn canvas(view: &V) -> String {
    let nodes = m::array(&view["nodes"]);
    let edges = m::array(&view["edges"]);
    let at = |path: &V| nodes.iter().position(|node| &node["path"] == path);
    // A link whose end is not drawn is not drawn either.
    let edges = edges
        .iter()
        .filter(|edge| at(&edge["from"]).is_some() && at(&edge["to"]).is_some())
        .collect::<Vec<_>>();
    let pairs = edges
        .iter()
        .filter_map(|edge| Some((at(&edge["from"])?, at(&edge["to"])?)))
        .collect::<Vec<_>>();
    let weighted = pairs
        .iter()
        .zip(&edges)
        .map(|((from, to), edge)| (*from, *to, edge["links"].as_u64().unwrap_or(1)))
        .collect::<Vec<_>>();
    let plan = plan(nodes.len(), &weighted);
    // How much each box uses and is used through the links drawn here.
    let mut flow = vec![(0u64, 0u64); nodes.len()];
    for (from, to, links) in &weighted {
        if from != to {
            flow[*from].0 += (*links).max(1);
            flow[*to].1 += (*links).max(1);
        }
    }
    // A link starts in the colour of using and ends in the colour of being
    // used. It shades along the way between its two columns, so the links
    // between the same two columns share one shading.
    let span = |wire: &Wire| {
        let (from, to) = pairs[wire.edge];
        let (start, end) = (plan.nodes[from].x, plan.nodes[to].x);
        if wire.back {
            (start.round() as i64, (end + BOX_WIDTH).round() as i64)
        } else {
            ((start + BOX_WIDTH).round() as i64, end.round() as i64)
        }
    };
    let mut spans = plan.wires.iter().map(span).collect::<Vec<_>>();
    spans.sort_unstable();
    spans.dedup();
    let focus = view["focus"]["path"].as_str();
    let chosen = focus.and_then(|path| at(&json!(path)));
    // A folder of changed files both uses and is used by the files around
    // it, so in a change map a link running back is no finding; nor is it
    // around an opened file.
    let opened = symbols(view);
    let cycles = !changes(view) && !opened;
    let near = |node: usize| {
        chosen.is_some_and(|chosen| {
            chosen == node
                || pairs
                    .iter()
                    .any(|pair| *pair == (chosen, node) || *pair == (node, chosen))
        })
    };
    let mut html = format!(
        "<div class=\"graph-map-plane\" style=\"width:{:.0}px;height:{:.0}px\"><svg width=\"{:.0}\" height=\"{:.0}\" aria-hidden=\"true\"><defs>",
        plan.width, plan.height, plan.width, plan.height
    );
    // The head of an arrow lies at what is used.
    html.push_str("<marker id=\"graph-map-arrow\" class=\"graph-map-arrow\" viewBox=\"0 0 8 8\" refX=\"7\" refY=\"4\" markerWidth=\"7\" markerHeight=\"7\" markerUnits=\"userSpaceOnUse\" orient=\"auto\"><path d=\"M0 0 L8 4 L0 8 z\"/></marker>");
    for (index, (start, end)) in spans.iter().enumerate() {
        html.push_str(&format!(
            "<linearGradient id=\"graph-map-flow-{index}\" gradientUnits=\"userSpaceOnUse\" x1=\"{start}\" y1=\"0\" x2=\"{end}\" y2=\"0\"><stop offset=\"0\" class=\"graph-map-flow-uses\"/><stop offset=\"0.5\" class=\"graph-map-flow-between\"/><stop offset=\"1\" class=\"graph-map-flow-used\"/></linearGradient>"
        ));
    }
    html.push_str("</defs>");
    for wire in &plan.wires {
        let edge = edges[wire.edge];
        let (links, guessed) = (
            edge["links"].as_u64().unwrap_or(0),
            edge["guessed"].as_u64().unwrap_or(0),
        );
        let (from, to) = pairs[wire.edge];
        let back = wire.back && cycles;
        let state = match chosen {
            Some(chosen) if chosen == from || chosen == to => " graph-map-edge-on",
            Some(_) => " graph-map-edge-off",
            None => "",
        };
        html.push_str(&format!(
            "<path class=\"graph-map-edge{state}{}{}\" d=\"{}\" stroke=\"url(#graph-map-flow-{})\" stroke-width=\"{:.1}\" marker-end=\"url(#graph-map-arrow)\"><title>{} uses {} · {}{}</title></path>",
            if guessed == links { " graph-map-edge-guess" } else { "" },
            if back { " graph-map-edge-back" } else { "" },
            wire.path,
            spans.binary_search(&span(wire)).unwrap_or(0),
            // A link that closes a cycle is a row of dots, kept thin.
            (1.0 + ((links.max(1) as f64).log2() * 0.5).min(2.5)).min(if back { 1.8 } else { 3.5 }),
            escape(&m::text(&nodes[from], "name")),
            escape(&m::text(&nodes[to], "name")),
            weight(links, guessed),
            if back { " · against the main direction, closes a cycle" } else { "" },
        ));
    }
    html.push_str("</svg>");
    if let Some(apart) = plan.apart {
        html.push_str(&format!(
            "<span class=\"graph-map-caption\" style=\"top:{apart:.0}px\">Not linked to anything here</span>"
        ));
    }
    for (index, node) in nodes.iter().enumerate() {
        let placed = &plan.nodes[index];
        let folder = m::yes(&node["folder"]);
        let symbol = m::yes(&node["symbol"]);
        let line = node["line"].as_u64().unwrap_or(0);
        let (uses, used) = (
            node["out"].as_u64().unwrap_or(0),
            node["in"].as_u64().unwrap_or(0),
        );
        let mut label = format!(
            "{}, {}",
            m::text(node, "name"),
            if symbol {
                match line {
                    0 => what(node),
                    line => format!("{}, line {line}", what(node)),
                }
            } else if folder {
                format!(
                    "folder, {}",
                    plural(node["files"].as_u64().unwrap_or(0), "file", "files")
                )
            } else {
                "file".to_owned()
            }
        );
        label.push_str(&format!(
            ", uses others through {}, is used through {}",
            plural(uses, "link", "links"),
            plural(used, "link", "links")
        ));
        let cycle = placed.cycle && cycles;
        if cycle {
            label.push_str(", part of a cycle");
        }
        let changed = node["changed"].as_u64().unwrap_or(0);
        // What a file around an opened one is to it, or what a place of a
        // change map is to the change.
        let mark = if opened {
            let mut mark = match node["role"].as_str() {
                Some("uses") => ", uses this file",
                Some("used") => ", used by this file",
                Some("both") => ", uses this file and is used by it",
                _ => "",
            }
            .to_owned();
            if let Some(status) = node["status"].as_str() {
                mark.push_str(&format!(", {}", happened(status)));
            }
            // A type whose own lines stand as they were can still hold
            // methods the change touched.
            match node["touched"].as_u64() {
                Some(inside) if symbol && inside > 0 => {
                    mark.push_str(&format!(", {} changed inside", count(inside)));
                }
                _ => {}
            }
            mark
        } else {
            match (changed, node["status"].as_str(), node["role"].as_str()) {
                (0, _, Some("affected")) => ", uses the change".to_owned(),
                (0, _, Some("needed")) => ", used by the change".to_owned(),
                (0, _, Some("both")) => ", uses the change and is used by it".to_owned(),
                (0, ..) => String::new(),
                (_, Some(status), _) => format!(", {}", happened(status)),
                _ => format!(
                    ", {} inside",
                    plural(changed, "changed file", "changed files")
                ),
            }
        };
        label.push_str(&mark);
        let kind = node["kind"].as_str().unwrap_or("");
        // A folder opens to its files, a file to what it holds.
        let opens = folder || (!symbol && node["symbols"].as_u64().unwrap_or(0) > 0);
        let title = match (symbol, line) {
            (true, 0) => format!("{} · {}", m::text(node, "name"), m::text(view, "scope")),
            (true, line) => format!(
                "{} · {}:{line}",
                m::text(node, "name"),
                m::text(view, "scope")
            ),
            _ => m::text(node, "path"),
        };
        let (out, into) = flow[index];
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-map-node\" data-action=\"graph-map-node\" data-payload=\"{}\" data-folder=\"{folder}\" data-kind=\"{}\" data-open=\"{opens}\" data-outside=\"{}\" data-changed=\"{}\" data-selected=\"{}\" data-dim=\"{}\" data-cycle=\"{}\" data-flow=\"{}\" data-out=\"{}\" data-in=\"{}\" style=\"left:{:.0}px;top:{:.0}px\" title=\"{}\" aria-label=\"{}\">{}<span class=\"graph-map-node-name\">{}</span>",
            payload(&node["path"]),
            escape(kind),
            opened && !symbol,
            changed > 0,
            chosen == Some(index),
            chosen.is_some() && !near(index),
            cycle,
            leaning(out, into),
            out > 0,
            into > 0,
            placed.x,
            placed.y,
            escape(&title),
            escape(&label),
            match letter(kind) {
                "" => String::new(),
                letter => format!(
                    "<span class=\"graph-map-node-kind\" aria-hidden=\"true\">{letter}</span>"
                ),
            },
            escape(&m::text(node, "name")),
        ));
        if cycle {
            html.push_str("<span class=\"graph-map-node-cycle\" aria-hidden=\"true\">↻</span>");
        }
        if changed > 0 {
            // A changed file carries Git's letter and a folder how many
            // changed. So does a symbol: the letter where its own lines
            // changed, else how many symbols changed inside it.
            html.push_str(&format!(
                "<span class=\"graph-map-node-status\" aria-hidden=\"true\">{}</span>",
                match node["status"].as_str() {
                    Some(status) => escape(status),
                    None if symbol => count(node["touched"].as_u64().unwrap_or(0)),
                    None => count(changed),
                }
            ));
        } else if folder {
            html.push_str(&format!(
                "<span class=\"graph-map-node-count\" aria-hidden=\"true\">{}</span>",
                count(node["files"].as_u64().unwrap_or(0))
            ));
        }
        html.push_str("</button>");
    }
    html.push_str("</div>");
    html
}
fn rows(title: &str, rows: &V, more: &V) -> String {
    let rows = m::array(rows);
    let more = more.as_u64().unwrap_or(0);
    let mut html = format!(
        "<h4>{title} <span class=\"graph-map-total\">{}</span></h4>",
        count(rows.len() as u64 + more)
    );
    if rows.is_empty() {
        html.push_str("<p class=\"knowledge-note\">Nothing.</p>");
        return html;
    }
    html.push_str("<ul class=\"graph-map-rows\">");
    for row in &rows {
        // A symbol of another file is reached by opening that file.
        let elsewhere = row["file"].as_str().filter(|_| m::yes(&row["symbol"]));
        let guessed = row["guessed"].as_u64().unwrap_or(0);
        // Two symbols are linked once, in one way; a place stands for many
        // links; what a type holds has a line.
        let told = match (row["relation"].as_str(), row["links"].as_u64()) {
            (Some(relation), _) => format!(
                "{}{}",
                escape(&relation.replace('_', " ")),
                if guessed > 0 { ", guessed" } else { "" }
            ),
            (None, Some(links)) => weight(links, guessed),
            _ => match row["line"].as_u64() {
                Some(line) if line > 0 => format!("line {line}"),
                _ => String::new(),
            },
        };
        html.push_str(&format!(
            "<li><button type=\"button\" class=\"graph-map-row\" data-action=\"graph-map-go\" data-changed=\"{}\" data-payload=\"{}\" title=\"{}\"><span class=\"graph-map-row-name\">{}{}{}</span><span class=\"graph-map-row-links\">{}</span></button></li>",
            m::yes(&row["changed"]),
            payload(&json!({"path": row["path"], "inside": row["inside"], "open": elsewhere})),
            escape(&m::text(row, "path")),
            escape(&m::text(row, "name")),
            if m::yes(&row["folder"]) { "/" } else { "" },
            match elsewhere {
                Some(file) => format!(
                    "<span class=\"graph-map-row-file\">{}</span>",
                    escape(base(file))
                ),
                None => String::new(),
            },
            told,
        ));
    }
    html.push_str("</ul>");
    if more > 0 {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">and {} more</p>",
            count(more)
        ));
    }
    html
}
fn kinds(focus: &V) -> String {
    let kinds = m::array(&focus["kinds"])
        .iter()
        .map(|kind| {
            format!(
                "{} {}",
                escape(&m::text(kind, "kind").replace('_', " ")),
                count(kind["links"].as_u64().unwrap_or(0))
            )
        })
        .collect::<Vec<_>>();
    if kinds.is_empty() {
        String::new()
    } else {
        format!(
            "<p class=\"knowledge-note\">Uses by kind: {}</p>",
            kinds.join(" · ")
        )
    }
}
fn open_file(path: &V) -> String {
    format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-open\" data-payload=\"{}\">Open file</button>",
        payload(path)
    )
}
/// What a symbol is, with what the change did to it: to its own lines, and
/// to how many symbols inside it.
fn reached(symbol: &V, mut text: String) -> String {
    if let Some(status) = symbol["status"].as_str() {
        text.push_str(&format!(" · {} by the change", happened(status)));
    }
    match symbol["touched"].as_u64() {
        Some(inside) if inside > 0 => {
            text.push_str(&format!(" · {} changed inside", count(inside)));
        }
        _ => {}
    }
    text
}
/// The side panel of an opened file: what the selected symbol uses and what
/// uses it, or which symbols of a file around are linked with this one.
fn file_side(view: &V) -> String {
    let focus = &view["focus"];
    if !focus.is_object() {
        return "<p class=\"knowledge-note\">Select a class, a method or a function to see what it uses and what uses it. A box without a letter is another file that this one uses or is used by: select it again to open it.</p>".to_owned();
    }
    if m::yes(&focus["symbol"]) {
        let line = focus["line"].as_u64().unwrap_or(0);
        let mut html = format!(
            "<h3>{}</h3><p class=\"graph-map-path\">{}{}</p><p class=\"knowledge-note\">{}</p>",
            escape(&m::text(focus, "name")),
            escape(&m::text(focus, "file")),
            if line > 0 {
                format!(":{line}")
            } else {
                String::new()
            },
            escape(&reached(focus, capital(&what(focus)))),
        );
        if !m::array(&focus["members"]).is_empty() {
            html.push_str(&rows("Members", &focus["members"], &focus["members_more"]));
        }
        html.push_str(&rows("Uses", &focus["uses"], &focus["uses_more"]));
        html.push_str(&rows("Used by", &focus["used_by"], &focus["used_by_more"]));
        html.push_str(&kinds(focus));
        return html;
    }
    let held = focus["symbols"].as_u64().unwrap_or(0);
    let mut html = format!(
        "<h3>{}</h3><p class=\"graph-map-path\">{}</p><p class=\"knowledge-note\">{}{} · {}</p>",
        escape(&m::text(focus, "name")),
        escape(&m::text(focus, "path")),
        match focus["status"].as_str() {
            Some(status) => format!("Changed file · {}", happened(status)),
            None => "Another file".to_owned(),
        },
        match focus["role"].as_str() {
            Some("uses") => " · uses this file",
            Some("used") => " · used by this file",
            _ => " · uses this file and is used by it",
        },
        plural(held, "symbol", "symbols"),
    );
    if held > 0 {
        html.push_str(&open_file(&focus["path"]));
    }
    // Its own symbols at the other end of the links with this file.
    html.push_str(&rows("Uses this file", &focus["uses"], &focus["uses_more"]));
    html.push_str(&rows(
        "Used by this file",
        &focus["used_by"],
        &focus["used_by_more"],
    ));
    html.push_str(&kinds(focus));
    html
}
/// The side panel: what the selected place uses and what uses it.
pub fn side(view: &V) -> String {
    if symbols(view) {
        return file_side(view);
    }
    let focus = &view["focus"];
    if !focus.is_object() {
        return if changes(view) {
            "<p class=\"knowledge-note\">Select a file to see what it uses and what uses it. The highlighted ones changed; the others use them or are used by them. Select a file again to open it to its classes, methods and functions.</p>"
        } else {
            "<p class=\"knowledge-note\">Select a folder or a file to see what it uses and what uses it. Select it again to open it: a folder to what it holds, a file to its classes, methods and functions.</p>"
        }
        .to_owned();
    }
    let folder = m::yes(&focus["folder"]);
    let changed = focus["changed"].as_u64().unwrap_or(0);
    let held = focus["symbols"].as_u64().unwrap_or(0);
    // Beside a change, a file's links are the ones it has with the change.
    let beside = changes(view) && changed == 0;
    let mut html = format!(
        "<h3>{}</h3><p class=\"graph-map-path\">{}</p><p class=\"knowledge-note\">{}</p>",
        escape(&m::text(focus, "name")),
        escape(&m::text(focus, "path")),
        if changes(view) {
            match (changed, focus["status"].as_str(), focus["role"].as_str()) {
                (0, _, role) => format!(
                    "{} · {}",
                    if m::yes(&focus["altered"]) {
                        "Changed too, outside the open folder"
                    } else {
                        "Not changed"
                    },
                    match role {
                        Some("affected") => "uses the change",
                        Some("needed") => "used by the change",
                        _ => "uses the change and is used by it",
                    }
                ),
                (_, Some(status), _) => format!("Changed file · {}", happened(status)),
                _ => format!(
                    "Folder · {}",
                    plural(changed, "changed file", "changed files")
                ),
            }
        } else if folder {
            format!(
                "Folder · {} · {} inside it",
                plural(focus["files"].as_u64().unwrap_or(0), "file", "files"),
                plural(focus["within"].as_u64().unwrap_or(0), "link", "links")
            )
        } else if held > 0 {
            format!("File · {}", plural(held, "symbol", "symbols"))
        } else {
            "File".to_owned()
        }
    );
    if folder {
        html.push_str(&format!(
            "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-open\" data-payload=\"{}\">Open folder</button>",
            payload(&focus["path"])
        ));
    } else if held > 0 {
        html.push_str(&open_file(&focus["path"]));
    }
    let (uses, used) = if beside {
        ("Uses in the change", "Used by the change")
    } else {
        ("Uses", "Used by")
    };
    html.push_str(&rows(uses, &focus["uses"], &focus["uses_more"]));
    html.push_str(&rows(used, &focus["used_by"], &focus["used_by_more"]));
    html.push_str(&kinds(focus));
    html
}
fn crumbs(view: &V) -> String {
    let crumbs = m::array(&view["crumbs"]);
    let last = crumbs.len().saturating_sub(1);
    crumbs
        .iter()
        .enumerate()
        .map(|(index, crumb)| {
            if index == last {
                format!(
                    "<span class=\"graph-map-crumb\" aria-current=\"location\">{}</span>",
                    escape(&m::text(crumb, "name"))
                )
            } else {
                format!(
                    "<button type=\"button\" class=\"graph-map-crumb\" data-action=\"graph-map-open\" data-payload=\"{}\">{}</button><span aria-hidden=\"true\">/</span>",
                    payload(&crumb["path"]),
                    escape(&m::text(crumb, "name"))
                )
            }
        })
        .collect()
}
fn html(id: &str, markup: &str) {
    if let Some(element) = dom::get(id) {
        element.set_inner_html(markup);
    }
}
const LEGEND: &str = "Dotted: against the main direction, closing a cycle (↻).";
const FILE_LEGEND: &str = "T type · m method · f function · c case · # heading. A box without a letter is another file, a plain name, or the open file itself for its imports.";
/// What the colours and the lines of the map mean, each colour beside a
/// patch of it, then what is particular to the picture: a folder, an opened
/// file, or a change (`change` says a change is chosen).
fn legend(view: &V, change: bool) -> String {
    let patch = |side: &str| {
        format!("<span class=\"graph-map-key graph-map-key-{side}\" aria-hidden=\"true\"></span>")
    };
    let particular = match (symbols(view), change) {
        (true, true) => format!("{FILE_LEGEND} Highlighted: touched by the change."),
        (true, false) => FILE_LEGEND.to_owned(),
        _ if changes(view) => {
            "Highlighted: changed. The rest use the change or are used by it.".to_owned()
        }
        _ => LEGEND.to_owned(),
    };
    format!(
        "An arrow runs from what uses {} to what is used {}, and a box is tinted by which it does more here. Solid: read from the code. Dashed: guessed by name. Thicker: more links. {particular}",
        patch("uses"),
        patch("used"),
    )
}
const TABS: [&str; 3] = ["commits", "pulls", "branches"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
/// A day as a history shows it, in universal time.
fn day(at: i64) -> String {
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
fn fresh(target: &str) -> V {
    json!({"target": target, "scope": "", "focus": "", "certain": false, "filter": ""})
}
fn state() -> V {
    let state = m::data("graph-map");
    if state.is_object() {
        state
    } else {
        fresh("")
    }
}
fn picker() -> V {
    let pick = m::data("graph-pick");
    if pick.is_object() {
        pick
    } else {
        json!({"open": false, "tab": "commits", "query": "", "scope": "", "state": "open", "active": 0, "rows": [], "more": false, "loading": false, "reason": null, "failed": null})
    }
}
fn open() -> bool {
    dom::get("graph-map").is_some_and(|dialog| dialog.has_attribute("open"))
}
fn blank() {
    html("graph-map-crumbs", "");
    html("graph-map-canvas", "");
    html("graph-map-side", "");
    dom::text("graph-map-summary", "");
}
/// The Show list as it stands.
fn list() {
    html(
        "graph-map-target",
        &targets(&m::data("graph-map-place"), &m::text(&state(), "target")),
    );
}
/// Draws the picker as it stands.
fn show() {
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
fn mark(next: usize) {
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
    let (Some(list), Some(row)) = (
        dom::get("graph-pick-list"),
        dom::get(&format!("graph-pick-row-{next}"))
            .and_then(|row| row.dyn_into::<web_sys::HtmlElement>().ok()),
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
/// Reads the rows of the picker's open tab, or, with `further`, the next
/// page of a history after the ones it has.
async fn fetch(further: bool) {
    let pick = picker();
    let project = dom::value("project");
    if project.is_empty() || !m::yes(&pick["open"]) {
        return;
    }
    let tab = m::text(&pick, "tab");
    let had = if further {
        m::array(&pick["rows"])
    } else {
        vec![]
    };
    let query = m::text(&pick, "query").trim().to_owned();
    let base = format!("/api/projects/{}/graph", dom::enc(&project));
    let path = match tab.as_str() {
        "pulls" => format!(
            "{base}/pulls{}",
            dom::query(&[("state", m::text(&pick, "state")), ("query", query)])
        ),
        "branches" => format!("{base}/branches"),
        _ => format!(
            "{base}/commits{}",
            dom::query(&[
                ("scope", m::text(&pick, "scope")),
                ("query", query),
                (
                    "skip",
                    if had.is_empty() {
                        String::new()
                    } else {
                        had.len().to_string()
                    }
                ),
            ])
        ),
    };
    let mut reading = pick;
    reading["loading"] = json!(true);
    dom::text("graph-pick-note", &remark(&reading));
    m::set("graph-pick", reading);
    let found = api::scoped("graph-pick", &path).await;
    let mut pick = picker();
    match found {
        Ok((epoch, found)) => {
            if !m::current("graph-pick", epoch) || m::text(&pick, "tab") != tab {
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
async fn histories() {
    let project = dom::value("project");
    let path = format!("/api/projects/{}/graph/branches", dom::enc(&project));
    let Ok(found) = api::get(&path).await else {
        return;
    };
    if dom::value("project") == project {
        m::set("graph-pick-branches", found["branches"].clone());
        html(
            "graph-pick-scope",
            &scopes(
                &m::data("graph-map-place"),
                &found["branches"],
                &m::text(&picker(), "scope"),
            ),
        );
    }
}
fn shut() {
    let mut pick = picker();
    if !m::yes(&pick["open"]) {
        return;
    }
    pick["open"] = json!(false);
    m::set("graph-pick", pick);
    dom::cancel_timer("graph-pick-search");
    show();
}
/// Keys of the open picker: the arrows walk the list, Enter chooses the
/// marked row and Escape closes the picker, not the map behind it.
pub fn key(e: &web_sys::KeyboardEvent) -> bool {
    let pick = picker();
    if !m::yes(&pick["open"]) || e.is_composing() {
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
fn render(view: &V) {
    // A chosen change stays chosen while one of its files is open.
    let change = view["chosen"].is_object();
    dom::text("graph-map-chosen", &chosen(view));
    let building = change && view["state"] == "building";
    dom::hidden("graph-map-progress", !building);
    let mut phase = String::new();
    if building {
        let shown = crate::graph::graph_view(
            &json!({"enabled": true, "state": "building", "build": view["build"]}),
        );
        if let Some(bar) = dom::get("graph-map-progress-bar") {
            let _ = match shown.percent {
                Some(percent) => bar.set_attribute("value", &percent.to_string()),
                None => bar.remove_attribute("value"),
            };
        }
        dom::text("graph-map-progress-amount", &shown.amount);
        dom::text("graph-map-progress-left", &shown.left);
        phase = shown.note;
    }
    if change && view["state"] == "failed" {
        dom::error(
            "graph-map-error",
            &format!(
                "The graph of that commit could not be built: {}",
                m::text(view, "failure")
            ),
        );
    }
    let before = interim(view, &phase);
    if change && !view["nodes"].is_array() {
        // Neither the commit's own graph nor the project's to draw on.
        blank();
        dom::text(
            "graph-map-note",
            if before.is_empty() && view["state"] != "failed" {
                "Reading the change…"
            } else {
                &before
            },
        );
        return;
    }
    html("graph-map-crumbs", &crumbs(view));
    dom::text("graph-map-summary", &summary(view));
    dom::text(
        "graph-map-note",
        [before, note(view)]
            .iter()
            .filter(|part| !part.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" ")
            .as_str(),
    );
    dom::set_checked("graph-map-certain", m::yes(&view["certain"]));
    html("graph-map-canvas", &canvas(view));
    html("graph-map-side", &side(view));
    html("graph-map-legend", &legend(view, change));
}
/// Asks the server to work out a pull request, a branch or a commit, which
/// starts the graph of its commit when there is none. `false` when it could
/// not, or when something else was chosen meanwhile.
async fn choose(target: &str) -> bool {
    let project = dom::value("project");
    dom::error("graph-map-error", "");
    dom::hidden("graph-map-progress", true);
    blank();
    dom::text("graph-map-chosen", "");
    dom::text("graph-map-note", "Reading the change…");
    let path = format!("/api/projects/{}/graph", dom::enc(&project));
    let asked = api::post(&path, json!({"action": "changes", "target": target})).await;
    if dom::value("project") != project || m::text(&state(), "target") != target {
        return false;
    }
    match asked {
        Ok(_) => true,
        Err(error) => {
            dom::text("graph-map-note", "");
            dom::error("graph-map-error", &error.message);
            false
        }
    }
}
/// Reads the map as the Show list and the folder stand. `quiet` is a repeat
/// while a graph is being built, which leaves the picture as it is.
async fn load(quiet: bool) {
    let state = state();
    let project = dom::value("project");
    if project.is_empty() {
        return;
    }
    let target = m::text(&state, "target");
    let mut query = vec![("scope", m::text(&state, "scope"))];
    for key in ["focus", "filter", "target"] {
        if !m::text(&state, key).is_empty() {
            query.push((key, m::text(&state, key)));
        }
    }
    if m::yes(&state["certain"]) {
        query.push(("certain", "1".to_owned()));
    }
    let path = format!(
        "/api/projects/{}/graph/map{}",
        dom::enc(&project),
        dom::query(&query)
    );
    if !quiet {
        dom::attr("graph-map-canvas", "aria-busy", "true");
    }
    let mut again = false;
    match api::scoped("graph-map", &path).await {
        Ok((epoch, view)) => {
            if !m::current("graph-map", epoch) || dom::value("project") != project {
                return;
            }
            // The server opens the project when the asked folder is gone.
            let mut state = state;
            if view["scope"].is_string() {
                state["scope"] = view["scope"].clone();
            }
            // A graph that was never started, as after a restart of the
            // Harness, is asked for once more.
            again = view["state"] == "missing" && state["asked"] != json!(target);
            if again {
                state["asked"] = json!(target);
            }
            m::set("graph-map", state);
            dom::error("graph-map-error", "");
            render(&view);
            if open() && (view["state"] == "building" || m::yes(&view["rebuilding"])) {
                dom::later("graph-map-poll", 1000, || {
                    spawn_local(async {
                        load(true).await;
                    })
                });
            } else {
                dom::cancel_timer("graph-map-poll");
            }
        }
        Err(error) if error.aborted => return,
        Err(error) => {
            dom::cancel_timer("graph-map-poll");
            if error.status == 409 && !target.is_empty() && state["asked"] != json!(target) {
                // The server no longer knows what was chosen.
                let mut state = state;
                state["asked"] = json!(target);
                m::set("graph-map", state);
                again = true;
            } else {
                dom::hidden("graph-map-progress", true);
                dom::error("graph-map-error", &error.message);
            }
        }
    }
    dom::attr("graph-map-canvas", "aria-busy", "false");
    if again && choose(&target).await {
        Box::pin(load(false)).await;
    }
}
/// What the checkout is on and its open pull requests, for the Show list.
async fn place() {
    let project = dom::value("project");
    if project.is_empty() {
        return;
    }
    let path = format!("/api/projects/{}/graph/pulls", dom::enc(&project));
    let Ok(found) = api::get(&path).await else {
        return;
    };
    if dom::value("project") == project {
        m::set("graph-map-place", found);
        list();
    }
}
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}
pub async fn event(
    kind: &str,
    id: &str,
    action: &str,
    payload: &V,
    value: &str,
    checked: bool,
) -> bool {
    let mut state = state();
    // A pull request, a branch or a commit is worked out before it is mapped.
    let mut chosen = false;
    match (kind, id, action) {
        ("click", "graph-view" | "graph-changes", _) => {
            let changes = id == "graph-changes";
            state = fresh(if changes { "work" } else { "" });
            m::set("graph-map", state.clone());
            dom::set_value("graph-map-filter", "");
            dom::error("graph-map-error", "");
            dom::hidden("graph-map-progress", true);
            blank();
            dom::text("graph-map-chosen", "");
            dom::text(
                "graph-map-note",
                if changes {
                    "Reading the change…"
                } else {
                    "Reading the graph…"
                },
            );
            shut();
            list();
            dom::dialog("graph-map", true);
            if changes {
                // The open pull requests say what the branch is stacked on.
                place().await;
            } else {
                spawn_local(place());
            }
        }
        ("click", "graph-map-close", _) => {
            dom::cancel_timer("graph-map-poll");
            shut();
            dom::dialog("graph-map", false);
            return true;
        }
        ("click", "graph-pick-open", _) => {
            let mut pick = picker();
            if m::yes(&pick["open"]) {
                shut();
                return true;
            }
            for (key, value) in [
                ("open", json!(true)),
                ("query", json!("")),
                ("active", json!(0)),
                ("rows", json!([])),
                ("more", json!(false)),
                ("failed", V::Null),
            ] {
                pick[key] = value;
            }
            dom::set_value("graph-pick-search", "");
            dom::set_value("graph-pick-state", &m::text(&pick, "state"));
            html(
                "graph-pick-scope",
                &scopes(
                    &m::data("graph-map-place"),
                    &m::data("graph-pick-branches"),
                    &m::text(&pick, "scope"),
                ),
            );
            m::set("graph-pick", pick);
            show();
            dom::focus("graph-pick-search");
            spawn_local(histories());
            fetch(false).await;
            return true;
        }
        ("click", "graph-pick-close", _) => {
            shut();
            dom::focus("graph-pick-open");
            return true;
        }
        (
            "click",
            "graph-pick-tab-commits" | "graph-pick-tab-pulls" | "graph-pick-tab-branches",
            _,
        ) => {
            let mut pick = picker();
            pick["tab"] = json!(id.rsplit('-').next().unwrap_or("commits"));
            // Each list is searched in its own words.
            pick["query"] = json!("");
            pick["rows"] = json!([]);
            pick["active"] = json!(0);
            pick["more"] = json!(false);
            pick["failed"] = V::Null;
            m::set("graph-pick", pick);
            dom::set_value("graph-pick-search", "");
            show();
            dom::focus("graph-pick-search");
            fetch(false).await;
            return true;
        }
        ("input", "graph-pick-search", _) => {
            let mut pick = picker();
            pick["query"] = json!(value);
            pick["active"] = json!(0);
            let local = pick["tab"] == "branches";
            m::set("graph-pick", pick);
            if local {
                show();
            } else {
                // The server searches once the typing pauses.
                dom::later("graph-pick-search", 220, || {
                    spawn_local(fetch(false));
                });
            }
            return true;
        }
        ("change", "graph-pick-scope" | "graph-pick-state", _) => {
            let mut pick = picker();
            pick[if id == "graph-pick-scope" {
                "scope"
            } else {
                "state"
            }] = json!(value);
            m::set("graph-pick", pick);
            fetch(false).await;
            return true;
        }
        ("click", _, "graph-pick-more") => {
            fetch(true).await;
            return true;
        }
        ("click", _, "graph-pick-choose") => {
            let Some(target) = payload.as_str() else {
                return true;
            };
            shut();
            state = fresh(target);
            chosen = true;
        }
        ("change", "graph-map-target", _) => {
            shut();
            state = fresh(value);
            chosen = true;
        }
        ("click", _, "graph-map-node") => {
            let path = payload.as_str().unwrap_or("");
            let wanted = payload.to_string();
            let again = dom::all(".graph-map-node[data-selected=\"true\"][data-open=\"true\"]")
                .iter()
                .any(|node| node.get_attribute("data-payload").as_deref() == Some(wanted.as_str()));
            if again {
                // A selected folder or file that is selected again opens.
                state["scope"] = json!(path);
                state["focus"] = json!("");
                state["filter"] = json!("");
                dom::set_value("graph-map-filter", "");
            } else {
                state["focus"] = json!(path);
            }
        }
        ("click", _, "graph-map-open") => {
            state["scope"] = payload.clone();
            state["focus"] = json!("");
            state["filter"] = json!("");
            dom::set_value("graph-map-filter", "");
        }
        ("click", _, "graph-map-go") => {
            let path = payload["path"].as_str().unwrap_or("");
            if let Some(file) = payload["open"].as_str() {
                // A symbol of another file: that file opens on it.
                state["scope"] = json!(file);
                state["filter"] = json!("");
            } else if !m::yes(&payload["inside"]) {
                // It is not drawn here: show it among its own neighbours,
                // in the whole project.
                state["target"] = json!("");
                state["scope"] = json!(parent(path));
                state["filter"] = json!("");
            }
            state["focus"] = json!(path);
        }
        ("change", "graph-map-certain", _) => state["certain"] = json!(checked),
        ("input", "graph-map-filter", _) => state["filter"] = json!(value),
        _ => return false,
    }
    m::set("graph-map", state.clone());
    let target = m::text(&state, "target");
    if chosen || (kind, action) == ("click", "graph-map-go") {
        // The filter field follows: it is emptied where the place changed.
        dom::set_value("graph-map-filter", &m::text(&state, "filter"));
        list();
    }
    if chosen && target.contains(':') && !choose(&target).await {
        return true;
    }
    load(false).await;
    true
}
#[cfg(test)]
mod tests {
    use super::*;
    fn layer(plan: &Plan, node: usize) -> usize {
        plan.nodes[node].layer.unwrap()
    }
    #[test]
    fn users_stand_left_of_what_they_use() {
        // 0 uses 1, 1 uses 2, and 0 also uses 2 directly.
        let plan = plan(3, &[(0, 1, 1), (1, 2, 1), (0, 2, 1)]);
        assert_eq!(
            (layer(&plan, 0), layer(&plan, 1), layer(&plan, 2)),
            (0, 1, 2)
        );
        assert!(plan.nodes[0].x < plan.nodes[1].x && plan.nodes[1].x < plan.nodes[2].x);
        assert!(plan.nodes.iter().all(|node| !node.cycle));
        assert!(plan.wires.iter().all(|wire| !wire.back));
        assert_eq!(plan.wires.len(), 3);
        // The long link bends through its own slot in the middle layer: it is
        // the only path with a straight run, and that run clears the box.
        let long = plan.wires.iter().find(|wire| wire.edge == 2).unwrap();
        assert!(long.path.contains(" L"), "{}", long.path);
        assert!(
            plan.wires
                .iter()
                .filter(|wire| wire.path.contains(" L"))
                .count()
                == 1
        );
        let run = long.path.split(" L").nth(1).unwrap();
        let height = run.split(' ').nth(1).unwrap().parse::<f64>().unwrap();
        let (top, bottom) = (plan.nodes[1].y, plan.nodes[1].y + BOX_HEIGHT);
        assert!(
            height < top || height > bottom,
            "{height} in {top}..{bottom}"
        );
    }
    #[test]
    fn the_weaker_direction_of_a_cycle_runs_back() {
        // 0 uses 1; 1 uses 2 nine times and 2 uses 1 once; 2 uses 3.
        let plan = plan(4, &[(0, 1, 5), (1, 2, 9), (2, 1, 1), (2, 3, 5)]);
        assert_eq!(
            (0..4).map(|node| layer(&plan, node)).collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
        assert_eq!(
            plan.nodes.iter().map(|node| node.cycle).collect::<Vec<_>>(),
            [false, true, true, false]
        );
        assert_eq!(
            plan.wires
                .iter()
                .map(|wire| (wire.edge, wire.back))
                .collect::<Vec<_>>(),
            [(0, false), (1, false), (2, true), (3, false)]
        );
        // The link back starts at the left side of 2 and ends at the right side of 1.
        let start = |wire: &Wire| {
            wire.path[1..]
                .split(' ')
                .next()
                .unwrap()
                .parse::<f64>()
                .unwrap()
        };
        let end = |wire: &Wire| {
            let parts = wire.path.split(' ').collect::<Vec<_>>();
            parts[parts.len() - 2].parse::<f64>().unwrap()
        };
        let back = &plan.wires[2];
        assert_eq!(start(back), plan.nodes[2].x);
        assert_eq!(end(back), plan.nodes[1].x + BOX_WIDTH);
        assert_eq!(start(&plan.wires[1]), plan.nodes[1].x + BOX_WIDTH);
        assert_eq!(end(&plan.wires[1]), plan.nodes[2].x);
        // A cycle of equal links still gets one layer per member.
        let ring = super::plan(3, &[(0, 1, 1), (1, 2, 1), (2, 0, 1)]);
        let mut layers = (0..3).map(|node| layer(&ring, node)).collect::<Vec<_>>();
        layers.sort();
        assert_eq!(layers, [0, 1, 2]);
        assert_eq!(ring.wires.iter().filter(|wire| wire.back).count(), 1);
        assert!(ring.nodes.iter().all(|node| node.cycle));
    }
    #[test]
    fn a_link_back_over_a_layer_keeps_its_straight_run() {
        // 0 uses 1, 1 uses 2, and 2 uses 0 once: that link crosses the middle layer.
        let plan = plan(3, &[(0, 1, 4), (1, 2, 4), (2, 0, 1)]);
        let back = plan.wires.iter().find(|wire| wire.back).unwrap();
        assert_eq!(back.edge, 2);
        let parts = back.path.split(' ').collect::<Vec<_>>();
        // Start, a curve to the slot, the straight run across it, a curve to the end.
        assert_eq!(parts.iter().filter(|part| part.starts_with('L')).count(), 1);
        assert_eq!(parts.iter().filter(|part| part.starts_with('C')).count(), 2);
        let run = parts.iter().position(|part| part.starts_with('L')).unwrap();
        // The run is horizontal and as wide as a box.
        assert_eq!(parts[run - 1], parts[run + 1]);
        let from = parts[run - 2].parse::<f64>().unwrap();
        let to = parts[run][1..].parse::<f64>().unwrap();
        assert_eq!(from - to, BOX_WIDTH);
    }
    #[test]
    fn a_node_stands_next_to_what_it_uses() {
        // 0 uses 1 and 1 uses 2; 3 uses only 2 and so stands beside 1, not 0.
        let plan = plan(4, &[(0, 1, 1), (1, 2, 1), (3, 2, 1)]);
        assert_eq!(
            (0..4).map(|node| layer(&plan, node)).collect::<Vec<_>>(),
            [0, 1, 2, 1]
        );
        assert!(plan.wires.iter().all(|wire| !wire.path.contains(" L")));
        // A node used on the way is not pulled past its own user.
        let chain = super::plan(4, &[(0, 1, 1), (1, 3, 1), (0, 2, 1), (2, 3, 1)]);
        assert_eq!(
            (0..4).map(|node| layer(&chain, node)).collect::<Vec<_>>(),
            [0, 1, 1, 2]
        );
    }
    #[test]
    fn neighbours_line_up_so_links_do_not_cross() {
        // 0 uses 3 and 1 uses 2: the right layer follows the left one.
        let plan = plan(4, &[(0, 3, 1), (1, 2, 1)]);
        assert!(plan.nodes[0].y < plan.nodes[1].y);
        assert!(plan.nodes[3].y < plan.nodes[2].y);
    }
    #[test]
    fn unlinked_nodes_sit_below_and_nothing_overlaps() {
        let plan = plan(7, &[(0, 1, 1), (0, 0, 1), (1, 9, 1)]);
        assert_eq!(plan.wires.len(), 1);
        let apart = plan.apart.unwrap();
        let loose = plan.nodes[2..].iter().collect::<Vec<_>>();
        assert!(loose
            .iter()
            .all(|node| node.layer.is_none() && node.y >= apart + CAPTION));
        assert!(plan.nodes[..2]
            .iter()
            .all(|node| node.y + BOX_HEIGHT < apart));
        for (index, a) in plan.nodes.iter().enumerate() {
            assert!(a.x + BOX_WIDTH <= plan.width && a.y + BOX_HEIGHT <= plan.height);
            for b in &plan.nodes[index + 1..] {
                let apart = (a.x - b.x).abs() >= BOX_WIDTH || (a.y - b.y).abs() >= BOX_HEIGHT;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
        let empty = super::plan(0, &[]);
        assert!(empty.nodes.is_empty() && empty.wires.is_empty());
        let alone = super::plan(1, &[]);
        assert_eq!((alone.nodes[0].layer, alone.apart), (None, None));
    }
    fn view() -> V {
        json!({
            "scope": "src", "certain": false, "children": 3, "matching": 3, "between": 2,
            "crumbs": [{"name": "shop", "path": ""}, {"name": "src", "path": "src"}],
            "totals": {"files": 1115, "links": 4165, "read": 115, "guessed": 4050, "unplaced": 270},
            "nodes": [
                {"path": "src/Cart", "name": "Cart", "folder": true, "files": 2, "out": 0, "in": 3, "within": 2},
                {"path": "src/Order", "name": "Order", "folder": true, "files": 1, "out": 2, "in": 0, "within": 0},
                {"path": "src/a<b>.php", "name": "a<b>.php", "folder": false, "files": 1, "out": 0, "in": 1, "within": 0}
            ],
            "edges": [
                {"from": "src/Order", "to": "src/Cart", "links": 3, "guessed": 1},
                {"from": "src/Order", "to": "src/a<b>.php", "links": 2, "guessed": 2}
            ],
            "focus": {
                "path": "src/Order", "name": "Order", "folder": true, "inside": true, "files": 1, "within": 0,
                "uses": [{"path": "src/Cart", "name": "Cart", "folder": true, "inside": true, "links": 3, "guessed": 1}],
                "uses_more": 0,
                "used_by": [{"path": "tests", "name": "tests", "folder": true, "inside": false, "links": 1, "guessed": 1}],
                "used_by_more": 4,
                "kinds": [{"kind": "calls", "links": 4}, {"kind": "imports_from", "links": 1}]
            }
        })
    }
    fn change() -> V {
        json!({
            "mode": "changes",
            "scope": "",
            "crumbs": [{"name": "shop", "path": ""}],
            "certain": false,
            "totals": {"files": 6, "symbols": 9, "relations": 12, "links": 6, "read": 3, "guessed": 3, "unplaced": 0},
            "change": {"files": 3, "graphed": 1, "here": 1, "removed": 1, "unknown": 1, "single": true, "hidden": 0, "affected": 2, "needed": 1},
            "children": 2,
            "matching": 2,
            "nodes": [
                {"path": "src/Cart/Cart.php", "name": "Cart.php", "folder": false, "files": 1, "within": 0, "out": 2, "in": 2, "changed": 1, "status": "M", "role": "changed", "altered": false},
                {"path": "src/Cart/Price.php", "name": "Price.php", "folder": false, "files": 1, "within": 0, "out": 0, "in": 2, "changed": 0, "status": null, "role": "needed", "altered": false},
                {"path": "src/Order/Order.php", "name": "Order.php", "folder": false, "files": 1, "within": 0, "out": 1, "in": 0, "changed": 0, "status": null, "role": "affected", "altered": false}
            ],
            "between": 2,
            "backbone": false,
            "edges": [
                {"from": "src/Cart/Cart.php", "to": "src/Cart/Price.php", "links": 2, "guessed": 1},
                {"from": "src/Order/Order.php", "to": "src/Cart/Cart.php", "links": 1, "guessed": 0}
            ],
            "focus": null,
            "state": "ready",
            "chosen": {"target": "pr:12", "kind": "pull", "name": "#12", "title": "Round totals once", "pull": 12,
                "head": {"name": "fix/rounding", "commit": "1a2b3c4d5e6f"}, "base": {"name": "main", "commit": "9f8e7d6c5b4a"}, "notes": [], "files": 3}
        })
    }
    #[test]
    fn the_show_list_offers_the_checkout_and_keeps_what_was_picked() {
        let place = json!({
            "git": true, "branch": "fix/rounding", "base": "origin/main", "pull": 12, "against": "release",
            "pulls": [
                {"number": 12, "title": "Round <totals> once", "head": "fix/rounding", "base": "release", "draft": false},
                {"number": 9, "title": "Wip", "head": "wip", "base": "main", "draft": true}
            ]
        });
        // A branch with an open request is measured against that request's base.
        assert_eq!(
            targets(&place, "work"),
            "<option value=\"\">The whole project</option><option value=\"work\" selected>Current work: fix/rounding against release (pull request #12)</option><option value=\"uncommitted\">Uncommitted changes only</option>"
        );
        // What the picker chose stays in the list while it is shown: a
        // request by its title when it is known, a commit by its hash.
        assert!(
            targets(&place, "pr:9").ends_with("<option value=\"pr:9\" selected>#9 Wip</option>")
        );
        assert!(targets(&place, "pr:77")
            .ends_with("<option value=\"pr:77\" selected>Pull request #77</option>"));
        assert!(targets(&place, "commit:1a2b3c4d5e6f7a8b9c0d").ends_with(
            "<option value=\"commit:1a2b3c4d5e6f7a8b9c0d\" selected>Commit 1a2b3c4d</option>"
        ));
        assert!(targets(&place, "ref:feature/x")
            .ends_with("<option value=\"ref:feature/x\" selected>feature/x</option>"));
        // Without an open request the default branch is the base.
        let alone = json!({"git": true, "branch": "main", "base": "origin/main", "pull": null, "against": null, "pulls": []});
        assert!(targets(&alone, "")
            .contains("<option value=\"work\">Current work: main against origin/main</option>"));
        // Outside Git there is only the project.
        assert_eq!(
            targets(&json!({"git": false}), ""),
            "<option value=\"\" selected>The whole project</option>"
        );
    }
    #[test]
    fn a_time_is_told_as_a_span_and_after_a_month_as_a_day() {
        let now = 1_794_400_000;
        for (passed, text) in [
            (-5, "just now"),
            (59, "just now"),
            (60, "1 minute ago"),
            (3_599, "59 minutes ago"),
            (3_600, "1 hour ago"),
            (86_399, "23 hours ago"),
            (86_400, "yesterday"),
            (172_800, "2 days ago"),
            (2_591_999, "29 days ago"),
        ] {
            assert_eq!(ago(now, now - passed), text, "{passed}");
        }
        assert_eq!(ago(now, now - 2_592_000), day(now - 2_592_000));
        for (at, text) in [
            (0, "1 Jan 1970"),
            (951_782_400, "29 Feb 2000"),
            (1_709_251_199, "29 Feb 2024"),
            (1_709_251_200, "1 Mar 2024"),
            (1_794_400_000, "11 Nov 2026"),
            (1_798_761_599, "31 Dec 2026"),
        ] {
            assert_eq!(day(at), text, "{at}");
        }
    }
    fn commits() -> V {
        json!({
            "open": true, "tab": "commits", "query": "", "scope": "", "state": "open", "active": 1, "more": true,
            "rows": [
                {"commit": "1a2b3c4d5e6f", "short": "1a2b3c4", "subject": "Round <totals> once", "author": "Ann", "at": 1_794_396_400,
                    "refs": [{"name": "main", "kind": "head"}, {"name": "origin/main", "kind": "remote"}, {"name": "v2", "kind": "tag"}], "merge": false},
                {"commit": "9f8e7d6c5b4a", "short": "9f8e7d6", "subject": "Merge the cart", "author": "Bo", "at": 1_794_313_600, "refs": [], "merge": true}
            ]
        })
    }
    #[test]
    fn the_history_is_a_row_per_commit_with_its_names_author_time_and_hash() {
        let pick = commits();
        assert_eq!(
            offered(&pick)
                .iter()
                .map(|row| m::text(row, "target"))
                .collect::<Vec<_>>(),
            ["commit:1a2b3c4d5e6f", "commit:9f8e7d6c5b4a"]
        );
        let html = options(&pick, 1_794_400_000);
        assert!(html.starts_with(
            "<li class=\"graph-pick-row\" id=\"graph-pick-row-0\" role=\"option\" aria-selected=\"false\" data-action=\"graph-pick-choose\" data-payload=\"&quot;commit:1a2b3c4d5e6f&quot;\"><span class=\"graph-pick-main\"><span class=\"graph-pick-title\">Round &lt;totals&gt; once</span><span class=\"graph-pick-badges\"><span class=\"graph-pick-badge\" data-kind=\"head\">main</span><span class=\"graph-pick-badge\" data-kind=\"remote\">origin/main</span><span class=\"graph-pick-badge\" data-kind=\"tag\">v2</span></span></span><span class=\"graph-pick-who\">Ann</span><span class=\"graph-pick-when\">1 hour ago</span><code class=\"graph-pick-code\">1a2b3c4</code></li>"
        ), "{html}");
        // The marked row, a merge, and a history that goes on.
        assert!(html.contains("id=\"graph-pick-row-1\" role=\"option\" aria-selected=\"true\""));
        assert!(html.contains("<span class=\"graph-pick-badge\" data-kind=\"merge\">merge</span>"));
        assert!(html.contains("<span class=\"graph-pick-when\">yesterday</span>"));
        assert!(html.ends_with("data-action=\"graph-pick-more\">Show older commits</button></li>"));
        assert_eq!(
            remark(&pick),
            "↑ ↓ to move · Enter to choose · Esc to close"
        );
        let mut none = commits();
        none["rows"] = json!([]);
        assert_eq!(options(&none, 0), "<li class=\"graph-pick-more\" role=\"presentation\"><button type=\"button\" class=\"button compact\" data-action=\"graph-pick-more\">Show older commits</button></li>");
        none["more"] = json!(false);
        assert_eq!(options(&none, 0), "");
        assert_eq!(remark(&none), "No commit matches.");
        none["loading"] = json!(true);
        assert_eq!(remark(&none), "Reading…");
        none["failed"] = json!("This project folder is not a Git checkout.");
        assert_eq!(remark(&none), "This project folder is not a Git checkout.");
    }
    #[test]
    fn pull_requests_are_listed_with_their_state_and_a_typed_number_is_offered() {
        let mut pick = json!({
            "open": true, "tab": "pulls", "query": "", "state": "all", "active": 0, "more": false, "reason": null,
            "rows": [
                {"number": 89, "title": "Run working memory", "head": "codex/pilot", "base": "main", "draft": true, "state": "open", "author": "ann", "at": 1_794_396_400},
                {"number": 85, "title": "Rust runtime", "head": "codex/rust", "base": "main", "draft": false, "state": "merged", "author": "bo", "at": 1_794_313_600}
            ]
        });
        let html = options(&pick, 1_794_400_000);
        assert!(html.contains("data-payload=\"&quot;pr:89&quot;\"><span class=\"graph-pick-main\"><span class=\"graph-pick-number\">#89</span><span class=\"graph-pick-title\">Run working memory</span><span class=\"graph-pick-badges\"><span class=\"graph-pick-badge\" data-kind=\"draft\">draft</span></span></span><span class=\"graph-pick-detail\">codex/pilot → main</span><span class=\"graph-pick-who\">ann</span><span class=\"graph-pick-when\">1 hour ago</span></li>"), "{html}");
        assert!(
            html.contains("<span class=\"graph-pick-badge\" data-kind=\"merged\">merged</span>")
        );
        assert!(!html.contains("graph-pick-more"));
        // A number that the list does not hold is offered first; one that
        // it holds is not offered twice.
        pick["query"] = json!("#77");
        let rows = offered(&pick);
        assert_eq!(rows.len(), 3);
        assert_eq!(
            (m::text(&rows[0], "target"), &rows[0]["typed"]),
            ("pr:77".to_owned(), &json!(true))
        );
        assert!(options(&pick, 0).starts_with("<li class=\"graph-pick-row\" id=\"graph-pick-row-0\" role=\"option\" aria-selected=\"true\" data-action=\"graph-pick-choose\" data-payload=\"&quot;pr:77&quot;\"><span class=\"graph-pick-main\"><span class=\"graph-pick-number\">#77</span><span class=\"graph-pick-title\">Look up this pull request</span></span></li>"));
        pick["query"] = json!("89");
        assert_eq!(offered(&pick).len(), 2);
        pick["query"] = json!("memory");
        assert_eq!(offered(&pick).len(), 2);
        // Without the forge's tool the reason is said, and a number still works.
        pick["rows"] = json!([]);
        pick["reason"] = json!("the GitHub CLI (gh) is not installed");
        pick["query"] = json!("");
        assert_eq!(
            remark(&pick),
            "Pull requests cannot be listed: the GitHub CLI (gh) is not installed. A number can still be typed."
        );
        pick["reason"] = V::Null;
        assert_eq!(remark(&pick), "No pull request matches.");
    }
    #[test]
    fn branches_are_filtered_as_they_are_typed_and_offered_as_histories() {
        let mut pick = json!({
            "open": true, "tab": "branches", "query": "", "active": 0, "more": false,
            "rows": [
                {"name": "feature/cart", "remote": false, "commit": "1a2b", "at": 1_794_396_400, "subject": "Cart work", "current": true, "pull": 12},
                {"name": "main", "remote": false, "commit": "9f8e", "at": 1_794_313_600, "subject": "Release", "current": false, "pull": null},
                {"name": "origin/review", "remote": true, "commit": "7c7c", "at": null, "subject": "", "current": false, "pull": null}
            ]
        });
        let html = options(&pick, 1_794_400_000);
        assert!(html.contains("data-payload=\"&quot;ref:feature/cart&quot;\"><span class=\"graph-pick-main\"><span class=\"graph-pick-title\">feature/cart</span><span class=\"graph-pick-badges\"><span class=\"graph-pick-badge\" data-kind=\"current\">checked out</span><span class=\"graph-pick-badge\" data-kind=\"open\">#12</span></span></span><span class=\"graph-pick-detail\">Cart work</span><span class=\"graph-pick-when\">1 hour ago</span></li>"), "{html}");
        assert!(html.contains("data-payload=\"&quot;ref:origin/review&quot;\"><span class=\"graph-pick-main\"><span class=\"graph-pick-title\">origin/review</span></span></li>"));
        pick["query"] = json!(" MAIN ");
        assert_eq!(
            offered(&pick)
                .iter()
                .map(|row| m::text(row, "target"))
                .collect::<Vec<_>>(),
            ["ref:main"]
        );
        pick["query"] = json!("nothing");
        assert_eq!(remark(&pick), "No branch matches.");
        // The Commits tab can list the history of any local branch.
        assert_eq!(
            scopes(&json!({"branch": "feature/cart"}), &pick["rows"], "main"),
            "<option value=\"\">This branch: feature/cart</option><option value=\"*\">All branches</option><option value=\"main\" selected>main</option>"
        );
        assert_eq!(
            scopes(&json!({"branch": null}), &V::Null, ""),
            "<option value=\"\" selected>This checkout</option><option value=\"*\">All branches</option>"
        );
    }
    #[test]
    fn a_picked_commit_is_named_by_its_message_and_drawn_before_its_graph_is_built() {
        let mut view = change();
        view["chosen"] = json!({"target": "commit:1a2b3c4d5e6f", "kind": "commit", "name": "1a2b3c4d5e6f", "title": "Round totals once",
            "head": {"name": "1a2b3c4d5e6f", "commit": "1a2b3c4d5e6f"}, "base": {"name": "its parent", "commit": "9f8e7d6c5b4a"}, "notes": [], "files": 3});
        assert_eq!(chosen(&view), "Round totals once · commit 1a2b3c4d");
        assert_eq!(interim(&view, "Reading files"), "");
        view["state"] = json!("building");
        assert_eq!(
            interim(&view, "Reading files · 100 of 3,500"),
            "Building the graph of that commit. Reading files · 100 of 3,500."
        );
        view["preliminary"] = json!(true);
        assert_eq!(
            interim(&view, "Assembling and clustering the graph"),
            "Building the graph of that commit. Assembling and clustering the graph. Until it is ready, the links are those the files have in the checkout."
        );
        view["state"] = json!("failed");
        assert_eq!(
            interim(&view, ""),
            "The links are those the files have in the checkout, not in that commit."
        );
    }
    #[test]
    fn a_typed_target_is_a_pull_request_number_or_a_name() {
        for (text, target) in [
            ("12", Some("pr:12")),
            (" #12 ", Some("pr:12")),
            ("#1234567", Some("pr:1234567")),
            // Seven digits without the sign read as the start of a commit.
            ("1234567", Some("ref:1234567")),
            ("feature/cart", Some("ref:feature/cart")),
            ("1a2b3c4", Some("ref:1a2b3c4")),
            ("HEAD~2", Some("ref:HEAD~2")),
            ("012", Some("ref:012")),
            ("", None),
            ("#", None),
            ("#x", None),
            ("-x", None),
            ("two words", None),
        ] {
            assert_eq!(typed(text).as_deref(), target, "{text:?}");
        }
    }
    #[test]
    fn a_change_says_what_is_compared_and_how_far_it_reaches() {
        let view = change();
        assert_eq!(
            chosen(&view),
            "Round totals once · fix/rounding at 1a2b3c4d against main at 9f8e7d6c"
        );
        assert_eq!(
            summary(&view),
            "3 changed files, 1 in the graph · 2 other files use them · they use 1 other file · 1 deleted"
        );
        assert_eq!(note(&view), "");
        let mut work = change();
        work["chosen"] = json!({"kind": "work", "name": "fix/rounding", "title": null,
            "head": {"name": "fix/rounding", "commit": "1a2b3c4d5e6f"}, "base": {"name": "origin/main", "commit": "9f8e7d6c5b4a"},
            "notes": ["No base branch was found, so only uncommitted files are shown."]});
        work["stale"] = json!(true);
        work["rebuilding"] = json!(true);
        work["change"]["single"] = json!(false);
        work["change"]["hidden"] = json!(2);
        work["matching"] = json!(9);
        assert_eq!(
            chosen(&work),
            "fix/rounding at 1a2b3c4d and what is not committed, against origin/main at 9f8e7d6c"
        );
        assert_eq!(
            note(&work),
            "No base branch was found, so only uncommitted files are shown. The graph is older than the files: it is being rebuilt and the map will follow. The change is too wide to draw file by file, so it is drawn by folder. Select a folder twice to open it. 2 more places with changes are not drawn. Showing the 2 most linked of 9 files around the change; filter by name to reach the rest."
        );
        let mut uncommitted = change();
        uncommitted["chosen"] =
            json!({"kind": "uncommitted", "name": "main", "head": null, "base": null, "notes": []});
        uncommitted["change"] = json!({"files": 1, "graphed": 0, "here": 0, "removed": 0, "unknown": 1, "single": true, "hidden": 0, "affected": 0, "needed": 0});
        uncommitted["nodes"] = json!([]);
        uncommitted["edges"] = json!([]);
        assert_eq!(chosen(&uncommitted), "What is not committed on main");
        assert_eq!(
            summary(&uncommitted),
            "1 changed file, 0 in the graph · no other file uses them · they use no other file"
        );
        assert_eq!(
            note(&uncommitted),
            "None of the changed files is in the graph: the tool found no code in them."
        );
        uncommitted["change"]["files"] = json!(0);
        assert_eq!(note(&uncommitted), "Nothing is changed here.");
    }
    #[test]
    fn a_change_map_marks_what_changed_and_what_each_other_file_is_to_it() {
        let view = change();
        let html = canvas(&view);
        assert!(
            html.contains("data-changed=\"true\" data-selected=\"false\""),
            "{html}"
        );
        assert_eq!(html.matches("data-changed=\"true\"").count(), 1);
        assert!(
            html.contains("<span class=\"graph-map-node-status\" aria-hidden=\"true\">M</span>")
        );
        assert!(html.contains("aria-label=\"Cart.php, file, uses others through 2 links, is used through 2 links, modified\""));
        assert!(html.contains("aria-label=\"Price.php, file, uses others through 0 links, is used through 2 links, used by the change\""));
        assert!(html.contains("aria-label=\"Order.php, file, uses others through 1 link, is used through 0 links, uses the change\""));
        // A folder of changed files carries how many changed, not how many it holds.
        let mut wide = change();
        wide["nodes"][0] = json!({"path": "src/Cart", "name": "Cart", "folder": true, "files": 15, "within": 0, "out": 2, "in": 2, "changed": 15, "status": null, "role": "changed", "altered": false});
        wide["edges"] = json!([]);
        let html = canvas(&wide);
        assert!(
            html.contains("<span class=\"graph-map-node-status\" aria-hidden=\"true\">15</span>")
        );
        assert!(!html.contains("graph-map-node-count"));
        assert!(html.contains("15 changed files inside"));
        assert!(side(&view).contains("The highlighted ones changed"));
    }
    #[test]
    fn the_side_panel_of_a_change_says_what_the_selected_file_is_to_it() {
        let mut view = change();
        view["focus"] = json!({
            "path": "src/Order/Order.php", "name": "Order.php", "folder": false, "inside": true, "files": 1, "within": 0, "out": 1, "in": 0,
            "changed": 0, "status": null, "role": "affected", "altered": true,
            "uses": [{"path": "src/Cart/Cart.php", "name": "src/Cart/Cart.php", "folder": false, "inside": true, "links": 1, "guessed": 0, "changed": true}],
            "uses_more": 0, "used_by": [], "used_by_more": 0, "kinds": [{"kind": "calls", "links": 1}]
        });
        let html = side(&view);
        assert!(
            html.contains("Changed too, outside the open folder · uses the change"),
            "{html}"
        );
        assert!(
            html.contains("<h4>Uses in the change <span class=\"graph-map-total\">1</span></h4>")
        );
        assert!(html.contains("<h4>Used by the change"));
        assert!(html.contains("data-action=\"graph-map-go\" data-changed=\"true\""));
        view["focus"] = json!({
            "path": "src/Cart/Cart.php", "name": "Cart.php", "folder": false, "inside": true, "files": 1, "within": 0, "out": 2, "in": 2,
            "changed": 1, "status": "A", "role": "changed", "altered": false,
            "uses": [], "uses_more": 0, "used_by": [], "used_by_more": 0, "kinds": []
        });
        let html = side(&view);
        assert!(html.contains("Changed file · added"));
        assert!(html.contains("<h4>Uses <span"));
        view["focus"]["folder"] = json!(true);
        view["focus"]["status"] = V::Null;
        view["focus"]["changed"] = json!(15);
        let html = side(&view);
        assert!(html.contains("Folder · 15 changed files"));
        assert!(html.contains("data-action=\"graph-map-open\""));
    }
    const CART: &str = "src/Cart/Cart.php";
    /// A file opened to its symbols, with a file it uses and one that uses it.
    fn file() -> V {
        let symbol = |key: &str,
                      name: &str,
                      kind: &str,
                      line: u64,
                      owner: V,
                      held: u64,
                      out: u64,
                      into: u64| {
            json!({"path": format!("{CART}#{key}"), "name": name, "kind": kind, "line": line, "owner": owner, "members": held,
                "symbol": true, "folder": false, "files": 1, "within": 0, "out": out, "in": into})
        };
        json!({
            "mode": "file",
            "scope": CART,
            "crumbs": [{"name": "shop", "path": ""}, {"name": "src", "path": "src"}, {"name": "Cart", "path": "src/Cart"}, {"name": "Cart.php", "path": CART}],
            "certain": false,
            "totals": {"files": 6, "symbols": 23, "relations": 33, "links": 10, "read": 7, "guessed": 3, "unplaced": 0},
            "file": {"path": CART, "name": "Cart.php", "status": null, "symbols": 9, "types": 2, "methods": 3, "functions": 1, "others": 3,
                "inside": 3, "out": 6, "in": 4, "uses": 2, "used_by": 1},
            "children": 5,
            "matching": 5,
            "around": {"files": 2, "shown": 2},
            "nodes": [
                symbol("L19:.total()", "Cart.total()", "method", 19, json!("Cart"), 0, 2, 3),
                symbol("L12:Cart", "Cart", "type", 12, V::Null, 3, 0, 0),
                symbol("L29:format_<price>()", "format_<price>()", "function", 29, V::Null, 0, 1, 1),
                symbol("L1:Cart.php", "Cart.php", "file", 1, V::Null, 0, 1, 0),
                symbol("LIMIT", "LIMIT", "name", 0, V::Null, 0, 0, 1),
                {"path": "src/Cart/Price.php", "name": "Price.php", "kind": "file", "symbol": false, "folder": false, "files": 1, "symbols": 3,
                    "within": 0, "out": 0, "in": 3, "role": "used", "changed": 1, "status": "A"},
                {"path": "tests/CartTest.php", "name": "CartTest.php", "kind": "file", "symbol": false, "folder": false, "files": 1, "symbols": 0,
                    "within": 0, "out": 2, "in": 0, "role": "uses", "changed": 0, "status": null}
            ],
            "between": 3,
            "backbone": false,
            "edges": [
                {"from": format!("{CART}#L19:.total()"), "to": "src/Cart/Price.php", "links": 2, "guessed": 0},
                {"from": "tests/CartTest.php", "to": format!("{CART}#L19:.total()"), "links": 2, "guessed": 1},
                {"from": format!("{CART}#L29:format_<price>()"), "to": "src/Cart/Price.php", "links": 1, "guessed": 1}
            ],
            "focus": null
        })
    }
    #[test]
    fn an_opened_file_says_what_it_holds_and_what_is_left_out() {
        let view = file();
        assert_eq!(
            summary(&view),
            "Cart.php · 2 types, 3 methods, 1 function, 3 others · 3 links between them · uses 2 other files · 1 other file uses it"
        );
        assert_eq!(note(&view), "");
        let mut document = file();
        document["file"] = json!({"path": "docs/guide.md", "name": "guide.md", "status": null, "symbols": 28, "types": 0, "methods": 0, "functions": 0, "headings": 28, "others": 0,
            "inside": 0, "out": 6, "in": 7, "uses": 6, "used_by": 7});
        assert_eq!(
            summary(&document),
            "guide.md · 28 headings · 0 links between them · uses 6 other files · 7 other files use it"
        );
        // A long file keeps its most linked symbols and files.
        let mut long = file();
        long["matching"] = json!(67);
        long["around"] = json!({"files": 15, "shown": 2});
        long["between"] = json!(400);
        assert_eq!(
            note(&long),
            "Showing the 5 most linked of 67 symbols in this file; filter by name to reach the rest. Showing the 2 most linked of 15 files around them. Drawing the 3 strongest of 400 links."
        );
        let mut unmatched = file();
        unmatched["matching"] = json!(0);
        unmatched["nodes"] = json!([]);
        unmatched["edges"] = json!([]);
        assert_eq!(note(&unmatched), "No name in this file matches the filter.");
        let mut unlinked = file();
        unlinked["edges"] = json!([]);
        unlinked["certain"] = json!(true);
        assert_eq!(note(&unlinked), "No link read from the code touches these.");
        // A file of a change says what happened to it, and the notes of
        // the change stay above it.
        let mut changed = file();
        changed["file"] = json!({"path": "docs/guide.txt", "name": "guide.txt", "status": "M", "symbols": 0, "types": 0, "methods": 0, "functions": 0, "others": 0,
            "inside": 0, "out": 0, "in": 0, "uses": 0, "used_by": 4});
        changed["children"] = json!(0);
        changed["nodes"] = json!([]);
        changed["edges"] = json!([]);
        changed["chosen"] = json!({"kind": "work", "name": "fix/rounding", "notes": []});
        changed["stale"] = json!(true);
        assert_eq!(
            summary(&changed),
            "guide.txt (modified) · the graph names nothing inside it · uses no other file · 4 other files use it"
        );
        assert_eq!(
            note(&changed),
            "The graph is older than the files, so the links of edited files may be out of date. Rebuild the graph to refresh them. The graph names nothing inside this file."
        );
    }
    #[test]
    fn an_opened_file_draws_its_symbols_with_a_letter_and_the_files_around_without() {
        let html = canvas(&file());
        assert_eq!(html.matches("<button").count(), 7);
        assert_eq!(html.matches("<path class=\"graph-map-edge").count(), 3);
        // A type, a method and a function carry a letter; the file's own
        // node and a plain name carry none.
        for letter in ["T", "m", "f"] {
            assert!(
                html.contains(&format!(
                    "<span class=\"graph-map-node-kind\" aria-hidden=\"true\">{letter}</span>"
                )),
                "{letter}: {html}"
            );
        }
        assert_eq!(html.matches("graph-map-node-kind").count(), 3);
        assert!(html.contains("data-kind=\"method\" data-open=\"false\" data-outside=\"false\""));
        // The files around stand apart, and one that holds symbols opens.
        assert_eq!(html.matches("data-outside=\"true\"").count(), 2);
        assert_eq!(html.matches("data-open=\"true\"").count(), 1);
        assert!(html.contains(
            "data-kind=\"file\" data-open=\"true\" data-outside=\"true\" data-changed=\"true\""
        ));
        assert!(
            html.contains("<span class=\"graph-map-node-status\" aria-hidden=\"true\">A</span>")
        );
        assert!(html.contains("title=\"Cart.total() · src/Cart/Cart.php:19\" aria-label=\"Cart.total(), method of Cart, line 19, uses others through 2 links, is used through 3 links\""), "{html}");
        assert!(html.contains("aria-label=\"Cart, type, line 12, uses others through 0 links, is used through 0 links\""));
        assert!(html.contains("title=\"LIMIT · src/Cart/Cart.php\" aria-label=\"LIMIT, name (a constant, a field or a plain type), uses others"));
        assert!(html.contains("aria-label=\"Cart.php, the file itself, for what belongs to no symbol in it, such as its imports, line 1, uses others through 1 link, is used through 0 links\""));
        assert!(html.contains("aria-label=\"Price.php, file, uses others through 0 links, is used through 3 links, used by this file, added\""));
        assert!(html.contains("aria-label=\"CartTest.php, file, uses others through 2 links, is used through 0 links, uses this file\""));
        // A file that both uses a symbol and is used by one closes no cycle.
        assert_eq!(html.matches("data-cycle=\"true\"").count(), 0);
        assert!(!html.contains("closes a cycle"));
        // Names from the code never become markup.
        assert!(!html.contains("format_<price>"));
        assert!(html.contains("format_&lt;price&gt;()</span>"));
        // In a folder a file that holds symbols opens as a folder does.
        let mut folder = view();
        folder["nodes"][2]["symbols"] = json!(4);
        let html = canvas(&folder);
        assert_eq!(html.matches("data-open=\"true\"").count(), 3);
        assert_eq!(html.matches("data-outside=\"true\"").count(), 0);
        assert_eq!(canvas(&view()).matches("data-open=\"true\"").count(), 2);
    }
    #[test]
    fn an_opened_file_of_a_change_marks_the_symbols_the_change_touched() {
        let mut view = file();
        view["file"]["status"] = json!("M");
        view["file"]["touched"] =
            json!({"symbols": 2, "added": 1, "modified": 1, "between": 1, "top": true});
        view["chosen"] = json!({"kind": "work", "name": "fix/rounding", "notes": []});
        // The method was rewritten, the function is new, and the class
        // holds one of them: its own lines stand as they were.
        for (node, status, inside) in [(0, json!("M"), 0), (1, V::Null, 1), (2, json!("A"), 0)] {
            view["nodes"][node]["changed"] = json!(1);
            view["nodes"][node]["status"] = status;
            view["nodes"][node]["touched"] = json!(inside);
        }
        let html = canvas(&view);
        assert_eq!(html.matches("data-changed=\"true\"").count(), 4);
        assert!(html.contains("line 19, uses others through 2 links, is used through 3 links, modified\"><span class=\"graph-map-node-kind\" aria-hidden=\"true\">m</span><span class=\"graph-map-node-name\">Cart.total()</span><span class=\"graph-map-node-status\" aria-hidden=\"true\">M</span>"), "{html}");
        // A class shows how many of its symbols changed, a new function
        // Git's letter for added.
        assert!(html.contains("is used through 0 links, 1 changed inside\"><span class=\"graph-map-node-kind\" aria-hidden=\"true\">T</span><span class=\"graph-map-node-name\">Cart</span><span class=\"graph-map-node-status\" aria-hidden=\"true\">1</span>"), "{html}");
        assert!(html.contains("is used through 1 link, added\">"));
        assert_eq!(
            note(&view),
            "The change touched 2 of 9 symbols: 1 added, 1 modified, and lines outside them. Lines were also removed between symbols in 1 place."
        );
        // The selected symbol says it too, and so do the members of a type.
        view["focus"] = json!({
            "path": format!("{CART}#L12:Cart"), "name": "Cart", "kind": "type", "line": 12, "owner": null, "file": CART,
            "symbol": true, "folder": false, "inside": true, "out": 0, "in": 0, "changed": 1, "status": "M", "touched": 2,
            "members": [
                {"path": format!("{CART}#L19:.total()"), "name": ".total()", "kind": "method", "line": 19, "symbol": true, "folder": false, "inside": true, "changed": true},
                {"path": format!("{CART}#L18:.add()"), "name": ".add()", "kind": "method", "line": 18, "symbol": true, "folder": false, "inside": true, "changed": false}
            ],
            "members_more": 0, "uses": [], "uses_more": 0, "used_by": [], "used_by_more": 0, "kinds": []
        });
        let html = side(&view);
        assert!(
            html.contains(
                "<p class=\"knowledge-note\">Type · modified by the change · 2 changed inside</p>"
            ),
            "{html}"
        );
        assert_eq!(html.matches("data-changed=\"true\"").count(), 1);
        // What the change did is told in fewer words where it did less.
        let reach = |touched: V| {
            let mut view = view.clone();
            view["file"]["touched"] = touched;
            super::reach(&view)
        };
        assert_eq!(
            reach(json!({"symbols": 3, "added": 0, "modified": 3, "between": 0, "top": false}))
                .as_deref(),
            Some("The change touched 3 of 9 symbols: 3 modified.")
        );
        assert_eq!(
            reach(json!({"symbols": 0, "added": 0, "modified": 0, "between": 2, "top": true})).as_deref(),
            Some("The change touched no symbol here, only lines outside them. Lines were also removed between symbols in 2 places.")
        );
        assert_eq!(
            reach(json!({"symbols": 0, "added": 0, "modified": 0, "between": 0, "top": false}))
                .as_deref(),
            Some("The change touched no line of a symbol here.")
        );
        // Not told: the graph is behind the files, or the file is no text.
        assert_eq!(
            reach(V::Null).as_deref(),
            Some("Which symbols the change touched could not be told from the file.")
        );
        view["stale"] = json!(true);
        view["file"]["touched"] = V::Null;
        assert!(note(&view).ends_with(
            "Rebuild the graph to refresh them. The symbols the change touched are marked once the graph is of these files."
        ));
        // A new file is new throughout, which takes one sentence.
        view["stale"] = json!(false);
        view["file"]["status"] = json!("A");
        view["file"]["touched"] =
            json!({"symbols": 9, "added": 9, "modified": 0, "between": 0, "top": true});
        assert_eq!(
            super::reach(&view).as_deref(),
            Some("The whole file is new in this change, so every symbol in it is added.")
        );
        // A file the change left alone says nothing of it.
        assert_eq!(super::reach(&file()), None);
    }
    #[test]
    fn the_side_panel_of_an_opened_file_names_symbols_and_how_they_are_linked() {
        let mut view = file();
        assert!(side(&view).contains("Select a class, a method or a function"));
        view["focus"] = json!({
            "path": format!("{CART}#L19:.total()"), "name": "Cart.total()", "kind": "method", "line": 19, "owner": "Cart", "file": CART,
            "symbol": true, "folder": false, "inside": true, "out": 2, "in": 3, "members": [], "members_more": 0,
            "uses": [
                {"path": "src/Cart/Price.php#L9:.of()", "name": "Price.of()", "kind": "method", "line": 9, "owner": "Price", "file": "src/Cart/Price.php",
                    "symbol": true, "folder": false, "inside": false, "relation": "calls", "links": 1, "guessed": 0, "changed": true}
            ],
            "uses_more": 0,
            "used_by": [
                {"path": format!("{CART}#L26:.checkout()"), "name": "Cart.checkout()", "kind": "method", "line": 26, "owner": "Cart", "file": null,
                    "symbol": true, "folder": false, "inside": true, "relation": "indirect_call", "links": 1, "guessed": 1, "changed": false}
            ],
            "used_by_more": 2,
            "kinds": [{"kind": "calls", "links": 2}]
        });
        let html = side(&view);
        assert!(html.starts_with("<h3>Cart.total()</h3><p class=\"graph-map-path\">src/Cart/Cart.php:19</p><p class=\"knowledge-note\">Method of Cart</p><h4>Uses <span class=\"graph-map-total\">1</span></h4>"), "{html}");
        // A symbol of another file carries the file and opens it; one of
        // this file is only selected. Each is linked once, in one way. The
        // keys of a payload stand in the order the build's JSON keeps.
        for part in [
            "&quot;path&quot;:&quot;src/Cart/Price.php#L9:.of()&quot;",
            "&quot;inside&quot;:false",
            "&quot;open&quot;:&quot;src/Cart/Price.php&quot;",
            "&quot;path&quot;:&quot;src/Cart/Cart.php#L26:.checkout()&quot;",
            "&quot;inside&quot;:true",
            "&quot;open&quot;:null",
        ] {
            assert!(html.contains(part), "{part}: {html}");
        }
        assert!(
            html.contains("data-action=\"graph-map-go\" data-changed=\"true\" data-payload=\"{")
        );
        assert!(html.contains("title=\"src/Cart/Price.php#L9:.of()\"><span class=\"graph-map-row-name\">Price.of()<span class=\"graph-map-row-file\">Price.php</span></span><span class=\"graph-map-row-links\">calls</span>"), "{html}");
        assert!(html.contains("Cart.checkout()</span><span class=\"graph-map-row-links\">indirect call, guessed</span>"));
        assert!(html.contains("<h4>Used by <span class=\"graph-map-total\">3</span></h4>"));
        assert!(html.contains("Uses by kind: calls 2"));
        assert!(!html.contains("<h4>Members"));
        // A type lists what it holds, each with its line.
        view["focus"] = json!({
            "path": format!("{CART}#L12:Cart"), "name": "Cart", "kind": "type", "line": 12, "owner": null, "file": CART,
            "symbol": true, "folder": false, "inside": true, "out": 0, "in": 0,
            "members": [{"path": format!("{CART}#L19:.total()"), "name": ".total()", "kind": "method", "line": 19, "symbol": true, "folder": false, "inside": true}],
            "members_more": 2, "uses": [], "uses_more": 0, "used_by": [], "used_by_more": 0, "kinds": []
        });
        let html = side(&view);
        assert!(html.contains("<p class=\"knowledge-note\">Type</p><h4>Members <span class=\"graph-map-total\">3</span></h4>"), "{html}");
        assert!(html.contains(".total()</span><span class=\"graph-map-row-links\">line 19</span>"));
        assert!(!html.contains("Uses by kind"));
        // A file around lists its own symbols at the other end, and opens.
        view["focus"] = json!({
            "path": "src/Cart/Price.php", "name": "Price.php", "kind": "file", "symbol": false, "folder": false, "inside": true, "files": 1, "symbols": 3,
            "out": 0, "in": 3, "role": "used", "changed": 1, "status": "A",
            "uses": [], "uses_more": 0,
            "used_by": [{"path": "src/Cart/Price.php#L10:.plus()", "name": "Price.plus()", "kind": "method", "line": 10, "owner": "Price", "file": "src/Cart/Price.php",
                "symbol": true, "folder": false, "inside": false, "relation": null, "links": 2, "guessed": 1, "changed": true}],
            "used_by_more": 0, "kinds": []
        });
        let html = side(&view);
        assert!(html.starts_with("<h3>Price.php</h3><p class=\"graph-map-path\">src/Cart/Price.php</p><p class=\"knowledge-note\">Changed file · added · used by this file · 3 symbols</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-open\" data-payload=\"&quot;src/Cart/Price.php&quot;\">Open file</button><h4>Uses this file <span"), "{html}");
        assert!(
            html.contains("<h4>Used by this file <span class=\"graph-map-total\">1</span></h4>")
        );
        assert!(html.contains("Price.plus()<span class=\"graph-map-row-file\">Price.php</span></span><span class=\"graph-map-row-links\">2 links, 1 guessed</span>"));
        view["focus"]["status"] = V::Null;
        view["focus"]["role"] = json!("both");
        view["focus"]["symbols"] = json!(0);
        let html = side(&view);
        assert!(html.contains("Another file · uses this file and is used by it · 0 symbols"));
        assert!(!html.contains("Open file"));
        // In a folder a selected file says what it holds and opens.
        let mut folder = super::tests::view();
        folder["focus"]["folder"] = json!(false);
        folder["focus"]["symbols"] = json!(9);
        let html = side(&folder);
        assert!(html.contains("<p class=\"knowledge-note\">File · 9 symbols</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-open\" data-payload=\"&quot;src/Order&quot;\">Open file</button>"), "{html}");
        folder["focus"]["symbols"] = json!(0);
        assert!(side(&folder).contains("<p class=\"knowledge-note\">File</p><h4>"));
        assert_eq!(capital("method of Cart"), "Method of Cart");
        assert_eq!(
            (base("src/Cart/Cart.php"), base("README.md")),
            ("Cart.php", "README.md")
        );
    }
    #[test]
    fn the_summary_says_how_much_is_read_and_how_much_is_guessed() {
        assert_eq!(
            summary(&view()),
            "1,115 files · 4,165 links between files: 115 read from the code, 4,050 guessed by name · 270 skipped, their file is unclear"
        );
        assert_eq!(note(&view()), "");
        let mut capped = view();
        capped["matching"] = json!(178);
        capped["between"] = json!(500);
        assert_eq!(
            note(&capped),
            "Showing the 3 most connected of 178 here; filter by name to reach the rest. Drawing the 2 strongest of 500 links."
        );
        capped["backbone"] = json!(true);
        assert!(note(&capped).ends_with(
            "Too many links to draw at once: showing each one's 3 strongest, 2 of 500. Select one to list all of its links."
        ));
        let mut unlinked = view();
        unlinked["edges"] = json!([]);
        unlinked["certain"] = json!(true);
        assert_eq!(
            note(&unlinked),
            "No link read from the code connects these."
        );
        let mut unmatched = view();
        unmatched["matching"] = json!(0);
        unmatched["nodes"] = json!([]);
        unmatched["edges"] = json!([]);
        assert_eq!(note(&unmatched), "No name here matches the filter.");
    }
    #[test]
    fn the_map_marks_the_selection_its_links_and_guesses() {
        let html = canvas(&view());
        assert_eq!(html.matches("<button").count(), 3);
        assert_eq!(html.matches("<path class=\"graph-map-edge").count(), 2);
        // The selected folder, its two neighbours undimmed, both links lit.
        assert_eq!(html.matches("data-selected=\"true\"").count(), 1);
        assert_eq!(html.matches("data-dim=\"true\"").count(), 0);
        assert_eq!(html.matches("graph-map-edge-on").count(), 2);
        // Only the link that is all guesses is dashed.
        assert_eq!(html.matches("graph-map-edge-guess").count(), 1);
        assert!(html.contains("<title>Order uses Cart · 3 links, 1 guessed</title>"));
        assert!(html.contains("<title>Order uses a&lt;b&gt;.php · 2 links guessed</title>"));
        // Names from the project never become markup.
        assert!(!html.contains("a<b>"));
        assert!(html.contains("data-payload=\"&quot;src/a&lt;b&gt;.php&quot;\""));
        let mut unselected = view();
        unselected["focus"] = V::Null;
        let html = canvas(&unselected);
        assert_eq!(html.matches("data-selected=\"true\"").count(), 0);
        assert_eq!(html.matches("graph-map-edge-on").count(), 0);
    }
    #[test]
    fn a_link_shades_from_the_colour_of_using_to_the_colour_of_being_used() {
        let html = canvas(&view());
        // Order stands in the first column and what it uses in the second,
        // so both links take the one shading between those columns.
        let (start, end) = (MARGIN + BOX_WIDTH, MARGIN + BOX_WIDTH + COLUMN_GAP);
        assert_eq!(html.matches("<linearGradient ").count(), 1);
        assert!(html.contains(&format!(
            "<linearGradient id=\"graph-map-flow-0\" gradientUnits=\"userSpaceOnUse\" x1=\"{start}\" y1=\"0\" x2=\"{end}\" y2=\"0\"><stop offset=\"0\" class=\"graph-map-flow-uses\"/><stop offset=\"0.5\" class=\"graph-map-flow-between\"/><stop offset=\"1\" class=\"graph-map-flow-used\"/></linearGradient>"
        )), "{html}");
        assert_eq!(html.matches("stroke=\"url(#graph-map-flow-0)\"").count(), 2);
        // Every arrow has the one head, in the colour of being used.
        assert_eq!(html.matches("<marker ").count(), 1);
        assert_eq!(
            html.matches("marker-end=\"url(#graph-map-arrow)\"").count(),
            2
        );
        // The weaker direction of a cycle runs right to left, so it shades
        // right to left too; it is a thin row of dots however many links it is.
        let ring = json!({
            "scope": "src",
            "nodes": [
                {"path": "src/Cart", "name": "Cart", "folder": true, "files": 2},
                {"path": "src/Order", "name": "Order", "folder": true, "files": 1}
            ],
            "edges": [
                {"from": "src/Cart", "to": "src/Order", "links": 5000, "guessed": 0},
                {"from": "src/Order", "to": "src/Cart", "links": 300, "guessed": 0}
            ],
            "focus": null
        });
        let html = canvas(&ring);
        assert!(html.contains(&format!(
            "<linearGradient id=\"graph-map-flow-1\" gradientUnits=\"userSpaceOnUse\" x1=\"{end}\" y1=\"0\" x2=\"{start}\""
        )), "{html}");
        assert!(html.contains("stroke=\"url(#graph-map-flow-0)\" stroke-width=\"3.5\""));
        assert!(html.contains(" graph-map-edge-back\""));
        assert!(html.contains("stroke=\"url(#graph-map-flow-1)\" stroke-width=\"1.8\""));
        // In a change map a link running back is no finding and is drawn whole.
        let mut around = ring.clone();
        around["mode"] = json!("changes");
        let html = canvas(&around);
        assert!(!html.contains("graph-map-edge-back"));
        assert!(html.contains("stroke=\"url(#graph-map-flow-1)\" stroke-width=\"3.5\""));
    }
    #[test]
    fn a_box_is_tinted_by_which_it_does_more_here() {
        assert_eq!(leaning(0, 0), "");
        assert_eq!(leaning(3, 0), "uses");
        assert_eq!(leaning(4, 1), "uses");
        assert_eq!(leaning(3, 1), "uses-more");
        assert_eq!(leaning(3, 2), "even");
        assert_eq!(leaning(1, 1), "even");
        assert_eq!(leaning(2, 3), "even");
        assert_eq!(leaning(1, 3), "used-more");
        assert_eq!(leaning(1, 4), "used");
        assert_eq!(leaning(0, 7), "used");
        assert_eq!(leaning(u64::MAX, u64::MAX), "even");
        // Order only uses here; the folder and the file it uses are only used.
        // The side links leave from and the side they arrive at say the same.
        let html = canvas(&view());
        assert_eq!(
            html.matches("data-flow=\"uses\" data-out=\"true\" data-in=\"false\"")
                .count(),
            1
        );
        assert_eq!(
            html.matches("data-flow=\"used\" data-out=\"false\" data-in=\"true\"")
                .count(),
            2
        );
        // What a box does elsewhere does not tint it: no link here, no colour.
        let mut loose = view();
        loose["edges"] = json!([]);
        assert_eq!(
            canvas(&loose)
                .matches("data-flow=\"\" data-out=\"false\" data-in=\"false\"")
                .count(),
            3
        );
    }
    #[test]
    fn the_legend_shows_both_colours_and_what_is_particular_to_the_picture() {
        let folder = legend(&view(), false);
        assert!(folder.starts_with("An arrow runs from what uses <span class=\"graph-map-key graph-map-key-uses\" aria-hidden=\"true\"></span> to what is used <span class=\"graph-map-key graph-map-key-used\" aria-hidden=\"true\"></span>, and a box is tinted by which it does more here. Solid: read from the code."), "{folder}");
        assert!(folder.ends_with("Dotted: against the main direction, closing a cycle (↻)."));
        let around = legend(&change(), true);
        assert!(
            around.ends_with("Highlighted: changed. The rest use the change or are used by it.")
        );
        assert!(!around.contains("Dotted"));
        let opened = legend(&file(), false);
        assert!(opened.contains("T type · m method · f function"));
        assert!(opened.ends_with("for its imports."));
        assert!(legend(&file(), true).ends_with("Highlighted: touched by the change."));
    }
    #[test]
    fn the_side_panel_lists_both_directions() {
        let html = side(&view());
        assert!(html.contains("<h3>Order</h3>"));
        assert!(html.contains("Folder · 1 file · 0 links inside it"));
        assert!(html.contains("data-action=\"graph-map-open\""));
        assert!(html.contains("<h4>Uses <span class=\"graph-map-total\">1</span></h4>"));
        assert!(html.contains("<h4>Used by <span class=\"graph-map-total\">5</span></h4>"));
        assert!(html.contains("and 4 more"));
        assert!(html
            .contains("Cart/</span><span class=\"graph-map-row-links\">3 links, 1 guessed</span>"));
        assert!(html.contains("Uses by kind: calls 4 · imports from 1"));
        assert!(html.contains("&quot;inside&quot;:false"));
        assert!(side(&json!({"focus": null})).contains("Select a folder or a file"));
        assert_eq!(parent("src/Cart/Cart.php"), "src/Cart");
        assert_eq!(parent("README.md"), "");
    }
}
