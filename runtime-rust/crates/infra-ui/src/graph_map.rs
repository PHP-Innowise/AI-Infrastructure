//! Dependency map of the project's code graph: one folder at a time, what
//! uses what. Whatever depends on something stands to its left, so entry
//! points gather on the left edge and foundations on the right. A file opens
//! in turn to its classes, methods and functions and the files around them.
use crate::{api, dom, model as m};
use serde_json::{json, Value as V};
use std::collections::BTreeMap;
use wasm_bindgen::{closure::Closure, JsCast};
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
/// The room at the top of a frame for the name of its folder, and the room
/// between a frame and what it holds.
const FRAME_HEAD: f64 = 22.0;
const FRAME_PAD: f64 = 8.0;
/// In the window of one link: how wide a name is, how far its two ends
/// stand apart, and the room between two names and between two files.
const LINK_BOX: f64 = 220.0;
const LINK_GAP: f64 = 220.0;
const LINK_ROW: f64 = 6.0;
const LINK_BETWEEN: f64 = 10.0;
/// How small a picture is made to fit its room. Smaller than this its boxes
/// cannot be told apart, and the rest of it is scrolled to.
const SMALLEST: f64 = 0.25;
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
/// A picture without frames.
#[cfg(test)]
pub fn plan(count: usize, edges: &[(usize, usize, u64)]) -> Plan {
    plan_with(count, edges, &Frames::default())
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
pub fn plan_with(count: usize, edges: &[(usize, usize, u64)], frames: &Frames) -> Plan {
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
    let (mut above, mut below): (Vec<Vec<usize>>, Vec<Vec<usize>>) =
        (vec![vec![]; count], vec![vec![]; count]);
    for (near, far, ..) in &wired {
        let mut chain = vec![*near];
        // A link passes through the band that holds both of its ends.
        let through = meet(band[*near], band[*far]);
        for column in columns.iter_mut().take(layer[*far]).skip(layer[*near] + 1) {
            let bend = heights.len();
            heights.push(BEND_HEIGHT);
            band.push(through);
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
    // In every layer the bands stand in the same order.
    for column in &mut columns {
        column.sort_by_key(|item| rank[band[*item]]);
    }
    // Each item moves towards the average place of its neighbours in the
    // layer before it, then after it; a few rounds remove most crossings.
    // It stays in its band.
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
    // What a frame holds that no link here touches lies in a grid under the
    // rest of it, from the frame's first layer on.
    let apart_in = |frame: usize| {
        (0..count)
            .filter(|node| !linked[*node] && band[*node] == frame)
            .collect::<Vec<_>>()
    };
    let mut across = vec![1usize; framed];
    for frame in 0..framed {
        let unlinked = apart_in(frame).len();
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
                let unlinked = apart_in(frame).len();
                if unlinked > 0 {
                    if tall[frame] > 0.0 {
                        y += ROW_GAP;
                    }
                    grid_in[frame] = y;
                    let rows = unlinked.div_ceil(across[frame]);
                    y += rows as f64 * (BOX_HEIGHT + ROW_GAP) - ROW_GAP;
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
    let loose = (0..count)
        .filter(|node| !linked[*node] && band[*node] == free)
        .collect::<Vec<_>>();
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
            let at = apart_in(frame)
                .iter()
                .position(|other| *other == node)
                .unwrap_or(0);
            Placed {
                x: left(span[frame].map_or(0, |(first, _)| first) + at % across[frame]),
                y: grid_in[frame] + (at / across[frame]) as f64 * (BOX_HEIGHT + ROW_GAP),
                layer: None,
                cycle: false,
            }
        } else {
            let at = loose.iter().position(|other| *other == node).unwrap_or(0);
            Placed {
                x: MARGIN + inset + (at % per_row) as f64 * (BOX_WIDTH + ROW_GAP),
                y: grid_top + (at / per_row) as f64 * (BOX_HEIGHT + ROW_GAP),
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
/// The links of one place followed step by step, wherever they lead.
/// Whether the picture is one where a folder can be spread out among the
/// others: a folder of the whole project.
fn spreads(view: &V) -> bool {
    view["frames"].is_array()
}
fn follows(view: &V) -> bool {
    view["mode"] == "follow"
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
/// What the map shows, in a line: how much the graph holds, or what the
/// opened file, the followed place or the change is, and, without tests,
/// how much that leaves out.
pub fn summary(view: &V) -> String {
    format!("{}{}", shown(view), left_out(view))
}
/// What a map without tests leaves out: the files that are tests, and the
/// tests inside the files it keeps. Nothing where tests are not left out.
fn left_out(view: &V) -> String {
    let tests = &view["tests"];
    if !tests.is_object() {
        return String::new();
    }
    let number = |key: &str| tests[key].as_u64().unwrap_or(0);
    match (number("files"), number("inside")) {
        (0, 0) => " · no tests found to leave out".to_owned(),
        (files, 0) => format!(" · without {}", plural(files, "test file", "test files")),
        (0, inside) => format!(
            " · without {} inside its files",
            plural(inside, "test", "tests")
        ),
        (files, inside) => format!(
            " · without {} and {} inside other files",
            plural(files, "test file", "test files"),
            plural(inside, "test", "tests")
        ),
    }
}
fn shown(view: &V) -> String {
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
    if follows(view) {
        let followed = &view["followed"];
        let number = |key: &str| followed[key].as_u64().unwrap_or(0);
        // A file is followed among files; a folder among folders and files.
        let (one, many) = if m::yes(&followed["folder"]) {
            ("other", "others")
        } else {
            ("other file", "other files")
        };
        let mut text = format!("Links of {}, step by step", m::text(followed, "name"));
        text.push_str(&match number("uses") {
            0 => " · uses nothing else".to_owned(),
            used => format!(" · uses {}", plural(used, one, many)),
        });
        text.push_str(&match number("users") {
            0 => " · nothing else uses it".to_owned(),
            1 => format!(" · 1 {one} uses it"),
            users => format!(" · {} {many} use it", count(users)),
        });
        text.push_str(&format!(" · {} drawn", count(number("shown"))));
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
    // Links from a Rust crate into one it is not built with: the graph
    // holds them, and no map draws or counts them.
    if number("ruled_out") > 0 {
        text.push_str(&format!(
            " · {} ruled out by Cargo manifests",
            count(number("ruled_out"))
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
/// How the links followed from a place are unfolded, and what the picture
/// leaves out.
fn follow_note(view: &V) -> String {
    let followed = &view["followed"];
    let number = |key: &str| followed[key].as_u64().unwrap_or(0);
    let mut parts = context(view);
    parts.push(format!(
        "+ at the right corner of a box shows more of what it uses, + at the left corner more of what uses it, {} at a time, strongest first; − folds them back.",
        count(number("step"))
    ));
    if m::yes(&followed["full"]) {
        parts.push(format!(
            "The picture holds {} boxes: fold something to unfold more.",
            count(number("limit"))
        ));
    }
    // A link shows where one of its sides is unfolded; the rest are counted.
    match number("aside") {
        0 => {}
        1 => parts.push(
            "1 more link runs between these boxes; it shows where a side that holds it is unfolded."
                .to_owned(),
        ),
        aside => parts.push(format!(
            "{} more links run between these boxes; they show where a side that holds them is unfolded.",
            count(aside)
        )),
    }
    let drawn = m::array(&view["edges"]).len() as u64;
    let between = view["between"].as_u64().unwrap_or(0);
    if drawn > 0 && drawn < between {
        parts.push(format!(
            "Drawing the {} strongest of {} links.",
            count(drawn),
            count(between)
        ));
    }
    parts.join(" ")
}
pub fn note(view: &V) -> String {
    if symbols(view) {
        return file_note(view);
    }
    if follows(view) {
        return follow_note(view);
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
    // A folder that is spread out is a frame around what it holds.
    let frames = m::array(&view["frames"]);
    let frame = |path: &V| frames.iter().position(|frame| &frame["path"] == path);
    let framed = Frames {
        of: nodes.iter().map(|node| frame(&node["holder"])).collect(),
        holder: frames.iter().map(|held| frame(&held["holder"])).collect(),
    };
    let plan = plan_with(nodes.len(), &weighted, &framed);
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
    let followed = follows(view);
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
    for (index, place) in plan.frames.iter().enumerate() {
        if place.width > 0.0 {
            html.push_str(&format!(
                "<rect class=\"graph-map-frame\" data-inner=\"{}\" x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"8\"/>",
                framed.holder[index].is_some(),
                place.x,
                place.y,
                place.width,
                place.height
            ));
        }
    }
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
            "<path class=\"graph-map-edge{state}{}{}\" data-ends=\"{from} {to}\" d=\"{}\" stroke=\"url(#graph-map-flow-{})\" stroke-width=\"{:.1}\" marker-end=\"url(#graph-map-arrow)\"><title>{} uses {} · {}{}</title></path>",
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
    // Over each link lies a wider one that cannot be seen and is easy to
    // hit: selecting a link looks into it. They lie over all the links, so
    // none of them is hidden under a link drawn later.
    for wire in &plan.wires {
        let (from, to) = pairs[wire.edge];
        html.push_str(&format!(
            "<path class=\"graph-map-hit\" data-ends=\"{from} {to}\" d=\"{}\" data-action=\"graph-map-link\" data-payload=\"{}\"><title>{} uses {} · select to see what makes up this link</title></path>",
            wire.path,
            payload(&json!({"from": nodes[from]["path"], "to": nodes[to]["path"]})),
            escape(&m::text(&nodes[from], "name")),
            escape(&m::text(&nodes[to], "name")),
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
        let start = m::yes(&node["start"]);
        if start {
            label.push_str(", the links are followed from here");
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
        // A folder opens to its files, a file to what it holds. Among
        // followed links a box is only selected: opening it would leave them.
        let opens = !followed && (folder || (!symbol && node["symbols"].as_u64().unwrap_or(0) > 0));
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
            "<button type=\"button\" class=\"graph-map-node\" data-n=\"{index}\" data-action=\"graph-map-node\" data-payload=\"{}\" data-folder=\"{folder}\" data-kind=\"{}\" data-open=\"{opens}\" data-outside=\"{}\" data-changed=\"{}\" data-selected=\"{}\" data-dim=\"{}\" data-cycle=\"{}\" data-flow=\"{}\" data-out=\"{}\" data-in=\"{}\" data-start=\"{start}\" style=\"left:{:.0}px;top:{:.0}px\" title=\"{}\" aria-label=\"{}\">{}<span class=\"graph-map-node-name\">{}</span>",
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
        if followed {
            html.push_str(&handles(
                node,
                index,
                placed.x,
                placed.y,
                chosen.is_some() && !near(index),
            ));
        }
        if folder && spreads(view) {
            // A folder opens here too: + at its corner spreads it out.
            let told = format!(
                "Open {} here, among the others",
                escape(&m::text(node, "name"))
            );
            html.push_str(&format!(
                "<button type=\"button\" class=\"graph-map-spread\" data-of=\"{index}\" data-dim=\"{}\" data-action=\"graph-map-spread\" data-payload=\"{}\" style=\"left:{:.0}px;top:{:.0}px\" title=\"{told}\" aria-label=\"{told}\">+</button>",
                chosen.is_some() && !near(index),
                payload(&node["path"]),
                placed.x,
                placed.y,
            ));
        }
    }
    // The head of a frame: − draws the folder as one box again, then its
    // name, and how much of it is drawn where that is not all.
    for (held, place) in frames.iter().zip(&plan.frames) {
        if place.width <= 0.0 {
            continue;
        }
        let name = escape(&m::text(held, "name"));
        let (places, drawn) = (
            held["places"].as_u64().unwrap_or(0),
            held["drawn"].as_u64().unwrap_or(0),
        );
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-map-spread\" data-action=\"graph-map-gather\" data-payload=\"{}\" style=\"left:{:.0}px;top:{:.0}px\" title=\"Draw {name} as one box again\" aria-label=\"Draw {name} as one box again\">−</button><span class=\"graph-map-frame-name\" style=\"left:{:.0}px;top:{:.0}px;max-width:{:.0}px\" title=\"{}\">{name}{}</span>",
            payload(&held["path"]),
            place.x + 13.0,
            place.y + 11.0,
            place.x + 26.0,
            place.y + 3.0,
            (place.width - 32.0).max(40.0),
            escape(&m::text(held, "path")),
            if drawn < places {
                format!(
                    "<span class=\"graph-map-frame-count\"> · {} of {} drawn</span>",
                    count(drawn),
                    count(places)
                )
            } else {
                String::new()
            },
        ));
    }
    html.push_str("</div>");
    html
}
/// How far past a corner the second handle of a side hangs.
const HANDLE_GAP: f64 = 11.0;
/// The handles at the top corners of a box whose links are followed. The
/// right corner is for what the box uses and the left one for what uses it,
/// as the links leave on the right and arrive on the left. A side that is
/// unfolded has − to fold it back; a side with places not drawn has + with
/// how many, which unfolds the next of them.
fn handles(node: &V, at: usize, x: f64, y: f64, dim: bool) -> String {
    let name = escape(&m::text(node, "name"));
    let mut html = String::new();
    for (side, corner, outward, hang) in [
        ("out", x + BOX_WIDTH, 1.0, "after"),
        ("in", x, -1.0, "before"),
    ] {
        let what = if side == "out" {
            format!("what {name} uses")
        } else {
            format!("what uses {name}")
        };
        let more = node[format!("more_{side}").as_str()].as_u64().unwrap_or(0);
        let unfolded = m::yes(&node[format!("unfolded_{side}").as_str()]);
        let asked = payload(&json!({"path": node["path"], "side": side}));
        if unfolded {
            html.push_str(&format!(
                "<button type=\"button\" class=\"graph-map-more\" data-of=\"{at}\" data-side=\"{side}\" data-dim=\"{dim}\" data-action=\"graph-map-fold\" data-payload=\"{asked}\" style=\"left:{corner:.0}px;top:{y:.0}px\" title=\"Hide {what}\" aria-label=\"Hide {what}\">−</button>"
            ));
        }
        if more > 0 {
            // Beside a − the + hangs outside the corner, clear of the box.
            let (at, hang) = if unfolded {
                (corner + outward * HANDLE_GAP, hang)
            } else {
                (corner, "")
            };
            let told = if unfolded {
                format!("Show more of {what}: {} not drawn", count(more))
            } else {
                format!("Show {what}: {} not drawn", count(more))
            };
            html.push_str(&format!(
                "<button type=\"button\" class=\"graph-map-more\" data-of=\"{at}\" data-side=\"{side}\" data-hang=\"{hang}\" data-dim=\"{dim}\" data-action=\"graph-map-unfold\" data-payload=\"{asked}\" style=\"left:{at:.0}px;top:{y:.0}px\" title=\"{told}\" aria-label=\"{told}\">+{}</button>",
                count(more)
            ));
        }
    }
    html
}
/// Where the names of a looked-into link stand: what uses in a column on
/// the left, what is used in a column on the right, each file a frame around
/// its names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ends {
    /// Where each name stands, in the order the names were given: those
    /// that use, file by file, then those that are used.
    pub names: Vec<(f64, f64)>,
    /// The frame of each file, in the same order.
    pub files: Vec<Framed>,
    /// The curve of each link.
    pub wires: Vec<String>,
    pub width: f64,
    pub height: f64,
}
/// Lays out the two ends of a link. `users` and `used` say how many names
/// each file of an end holds; a link names its two names by their numbers
/// among all of them, those that use first. Files and the names in them are
/// ordered so that the links cross as little as a few rounds can make them.
/// Both ends begin at the top, so a long end does not push the short one
/// out of sight.
pub fn ends(users: &[usize], used: &[usize], links: &[(usize, usize)]) -> Ends {
    let left = users.iter().sum::<usize>();
    let count = left + used.iter().sum::<usize>();
    let mut file = Vec::with_capacity(count);
    for (group, names) in users.iter().chain(used).enumerate() {
        file.extend(std::iter::repeat_n(group, *names));
    }
    let mut near = vec![vec![]; count];
    for (from, to) in links {
        if *from < left && (left..count).contains(to) {
            near[*from].push(*to);
            near[*to].push(*from);
        }
    }
    let mut order = [(0..left).collect::<Vec<_>>(), (left..count).collect()];
    let mut place = (0..count)
        .map(|name| (if name < left { name } else { name - left }) as f64)
        .collect::<Vec<_>>();
    // Each name moves towards the average place of what it is linked with
    // at the other end, and a file towards that of its names.
    for round in 0..6 {
        let side = 1 - round % 2;
        let mut keyed = order[side]
            .iter()
            .map(|name| {
                let linked = &near[*name];
                let middle = if linked.is_empty() {
                    place[*name]
                } else {
                    linked.iter().map(|other| place[*other]).sum::<f64>() / linked.len() as f64
                };
                (middle, place[*name], *name)
            })
            .collect::<Vec<_>>();
        let mut middles: BTreeMap<usize, (f64, f64)> = BTreeMap::new();
        for (middle, _, name) in &keyed {
            let of_file = middles.entry(file[*name]).or_insert((0.0, 0.0));
            of_file.0 += middle;
            of_file.1 += 1.0;
        }
        let middle = |name: usize| middles[&file[name]].0 / middles[&file[name]].1;
        keyed.sort_by(|a, b| {
            middle(a.2)
                .total_cmp(&middle(b.2))
                .then(file[a.2].cmp(&file[b.2]))
                .then(a.0.total_cmp(&b.0))
                .then(a.1.total_cmp(&b.1))
        });
        order[side] = keyed.into_iter().map(|(.., name)| name).collect();
        for (index, name) in order[side].iter().enumerate() {
            place[*name] = index as f64;
        }
    }
    let mut names = vec![(0.0, 0.0); count];
    let mut files = vec![Framed::default(); users.len() + used.len()];
    let mut tall = [0.0f64; 2];
    for (side, order) in order.iter().enumerate() {
        let x = MARGIN + FRAME_PAD + side as f64 * (LINK_BOX + LINK_GAP);
        let mut y = MARGIN;
        let mut open: Option<usize> = None;
        for name in order {
            let of = file[*name];
            if open == Some(of) {
                y += LINK_ROW;
            } else {
                // The frame before ends under its last name; this one
                // begins with the room for the name of its file.
                if open.is_some() {
                    y += FRAME_PAD + LINK_BETWEEN;
                }
                open = Some(of);
                files[of] = Framed {
                    x: x - FRAME_PAD,
                    y,
                    width: LINK_BOX + 2.0 * FRAME_PAD,
                    height: 0.0,
                };
                y += FRAME_HEAD;
            }
            names[*name] = (x, y);
            y += BOX_HEIGHT;
            files[of].height = y + FRAME_PAD - files[of].y;
        }
        if open.is_some() {
            y += FRAME_PAD;
        }
        tall[side] = y;
    }
    let tallest = tall[0].max(tall[1]);
    let wires = links
        .iter()
        .map(|(from, to)| {
            if *from >= left || !(left..count).contains(to) {
                return String::new();
            }
            let start = (names[*from].0 + LINK_BOX, names[*from].1 + BOX_HEIGHT / 2.0);
            let end = (names[*to].0, names[*to].1 + BOX_HEIGHT / 2.0);
            let mut path = format!("M{:.1} {:.1}", start.0, start.1);
            curve(&mut path, start, end);
            path
        })
        .collect();
    Ends {
        names,
        files,
        wires,
        width: 2.0 * (MARGIN + FRAME_PAD + LINK_BOX) + LINK_GAP,
        height: (tallest + MARGIN).max(BOX_HEIGHT + 2.0 * MARGIN),
    }
}
/// The names of a looked-into link in one list, those that use first, each
/// with the file it lies in; and how many names each file of each end holds.
fn link_names(view: &V) -> (Vec<(V, V)>, [Vec<usize>; 2]) {
    let mut names = vec![];
    let mut held = [vec![], vec![]];
    for (side, end) in ["users", "used"].iter().enumerate() {
        for mut group in m::array(&view[*end]["groups"]) {
            let of_group = m::array(&group["names"]);
            held[side].push(of_group.len());
            // A name carries its file, not the file's other names.
            group["names"] = V::Null;
            names.extend(of_group.into_iter().map(|name| (name, group.clone())));
        }
    }
    (names, held)
}
/// The picture of one link, looked into: the names that use on the left,
/// the names that are used on the right, each in the frame of its file, and
/// every link between two of them. `chosen` is the name whose links are lit.
pub fn link_canvas(view: &V, chosen: &str) -> String {
    let (names, held) = link_names(view);
    let at = |key: &V| names.iter().position(|(name, _)| &name["key"] == key);
    let edges = m::array(&view["edges"]);
    let pairs = edges
        .iter()
        .filter_map(|edge| Some((at(&edge["from"])?, at(&edge["to"])?, edge)))
        .collect::<Vec<_>>();
    let plan = ends(
        &held[0],
        &held[1],
        &pairs
            .iter()
            .map(|(from, to, _)| (*from, *to))
            .collect::<Vec<_>>(),
    );
    let users = held[0].iter().sum::<usize>();
    let picked = names.iter().position(|(name, _)| name["key"] == chosen);
    let near = |name: usize| {
        picked.is_some_and(|picked| {
            picked == name
                || pairs.iter().any(|(from, to, _)| {
                    (*from, *to) == (picked, name) || (*from, *to) == (name, picked)
                })
        })
    };
    let mut html = format!(
        "<div class=\"graph-map-plane\" style=\"width:{:.0}px;height:{:.0}px\"><svg width=\"{:.0}\" height=\"{:.0}\" aria-hidden=\"true\"><defs><marker id=\"graph-link-arrow\" class=\"graph-map-arrow\" viewBox=\"0 0 8 8\" refX=\"7\" refY=\"4\" markerWidth=\"7\" markerHeight=\"7\" markerUnits=\"userSpaceOnUse\" orient=\"auto\"><path d=\"M0 0 L8 4 L0 8 z\"/></marker><linearGradient id=\"graph-link-flow\" gradientUnits=\"userSpaceOnUse\" x1=\"{:.0}\" y1=\"0\" x2=\"{:.0}\" y2=\"0\"><stop offset=\"0\" class=\"graph-map-flow-uses\"/><stop offset=\"0.5\" class=\"graph-map-flow-between\"/><stop offset=\"1\" class=\"graph-map-flow-used\"/></linearGradient></defs>",
        plan.width,
        plan.height,
        plan.width,
        plan.height,
        MARGIN + FRAME_PAD + LINK_BOX,
        MARGIN + FRAME_PAD + LINK_BOX + LINK_GAP,
    );
    for frame in &plan.files {
        if frame.width > 0.0 {
            html.push_str(&format!(
                "<rect class=\"graph-map-frame\" data-inner=\"false\" x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"8\"/>",
                frame.x, frame.y, frame.width, frame.height
            ));
        }
    }
    for ((from, to, edge), wire) in pairs.iter().zip(&plan.wires) {
        let state = match picked {
            Some(picked) if picked == *from || picked == *to => " graph-map-edge-on",
            Some(_) => " graph-map-edge-off",
            None => "",
        };
        let guessed = m::yes(&edge["guessed"]);
        html.push_str(&format!(
            "<path class=\"graph-map-edge{state}{}\" data-ends=\"{from} {to}\" d=\"{wire}\" stroke=\"url(#graph-link-flow)\" stroke-width=\"1.4\" marker-end=\"url(#graph-link-arrow)\"><title>{} uses {} · {}{}</title></path>",
            if guessed { " graph-map-edge-guess" } else { "" },
            escape(&m::text(&names[*from].0, "name")),
            escape(&m::text(&names[*to].0, "name")),
            escape(&m::text(edge, "kind").replace('_', " ")),
            if guessed { ", guessed by name" } else { "" },
        ));
    }
    html.push_str("</svg>");
    // The head of a frame: the file its names lie in.
    let mut first = 0;
    for (frame, size) in plan.files.iter().zip(held.iter().flatten()) {
        if let Some((_, file)) = names.get(first).filter(|_| *size > 0) {
            html.push_str(&format!(
                "<span class=\"graph-map-frame-name\" style=\"left:{:.0}px;top:{:.0}px;max-width:{:.0}px\" title=\"{}\">{}</span>",
                frame.x + FRAME_PAD,
                frame.y + 3.0,
                frame.width - 2.0 * FRAME_PAD,
                escape(&m::text(file, "path")),
                escape(&m::text(file, "name")),
            ));
        }
        first += size;
    }
    for (index, (name, file)) in names.iter().enumerate() {
        let (x, y) = plan.names[index];
        let uses = index < users;
        let kind = name["kind"].as_str().unwrap_or("");
        let line = name["line"].as_u64().unwrap_or(0);
        let links = name["links"].as_u64().unwrap_or(0);
        let place = match line {
            0 => m::text(file, "path"),
            line => format!("{}:{line}", m::text(file, "path")),
        };
        let label = format!(
            "{}, {}, {}, {} here",
            m::text(name, "name"),
            what(name),
            place,
            if uses {
                format!("uses through {}", plural(links, "link", "links"))
            } else {
                format!("is used through {}", plural(links, "link", "links"))
            }
        );
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-map-node\" data-n=\"{index}\" data-action=\"graph-link-node\" data-payload=\"{}\" data-folder=\"false\" data-kind=\"{}\" data-selected=\"{}\" data-dim=\"{}\" data-flow=\"{}\" data-out=\"{uses}\" data-in=\"{}\" style=\"left:{x:.0}px;top:{y:.0}px\" title=\"{}\" aria-label=\"{}\">{}<span class=\"graph-map-node-name\">{}</span>{}</button>",
            payload(&name["key"]),
            escape(kind),
            picked == Some(index),
            picked.is_some() && !near(index),
            if uses { "uses" } else { "used" },
            !uses,
            escape(&place),
            escape(&label),
            match letter(kind) {
                "" => String::new(),
                letter => format!(
                    "<span class=\"graph-map-node-kind\" aria-hidden=\"true\">{letter}</span>"
                ),
            },
            escape(&m::text(name, "name")),
            if links > 1 {
                format!(
                    "<span class=\"graph-map-node-count\" aria-hidden=\"true\">{}</span>",
                    count(links)
                )
            } else {
                String::new()
            },
        ));
    }
    html.push_str("</div>");
    html
}
/// What the window of a link is called: what uses, and what is used.
pub fn link_title(view: &V) -> String {
    format!(
        "{} uses {}",
        m::text(&view["from"], "name"),
        m::text(&view["to"], "name")
    )
}
/// The link in a line: how many links it stands for and how they were
/// found, and how many names in how many files lie at each end.
pub fn link_summary(view: &V) -> String {
    if m::yes(&view["missing"]) {
        return "The graph does not know one end of this link any more.".to_owned();
    }
    let links = view["links"].as_u64().unwrap_or(0);
    let guessed = view["guessed"].as_u64().unwrap_or(0);
    if links == 0 {
        return if m::yes(&view["certain"]) {
            "No link read from the code runs this way.".to_owned()
        } else {
            "No link runs this way.".to_owned()
        };
    }
    let end = |side: &str| {
        let names = view[side]["names"].as_u64().unwrap_or(0);
        match view[side]["files"].as_u64().unwrap_or(0) {
            1 => plural(names, "name", "names"),
            files => format!(
                "{} in {} files",
                plural(names, "name", "names"),
                count(files)
            ),
        }
    };
    format!(
        "{}: {} read from the code, {} guessed by name · {} {} {}",
        plural(links, "link", "links"),
        count(links - guessed.min(links)),
        count(guessed),
        end("users"),
        if view["users"]["names"] == 1 {
            "uses"
        } else {
            "use"
        },
        end("used"),
    )
}
/// What of a link is not drawn, where it is more than a window holds.
pub fn link_note(view: &V) -> String {
    let (names, _) = link_names(view);
    let users = m::array(&view["users"]["groups"])
        .iter()
        .map(|group| m::array(&group["names"]).len() as u64)
        .sum::<u64>();
    let used = names.len() as u64 - users;
    let number = |side: &str| view[side]["names"].as_u64().unwrap_or(0);
    let drawn = m::array(&view["edges"]).len() as u64;
    let links = view["links"].as_u64().unwrap_or(0);
    let mut parts = vec![];
    if m::yes(&view["preliminary"]) {
        parts.push(
            "The links are those the files have in the checkout, not in that commit.".to_owned(),
        );
    }
    if users < number("users") || used < number("used") {
        parts.push(format!(
            "Showing the most linked names: {} of {} that use, {} of {} that are used, and {} of the {} between them. Look into a narrower link to see the rest: open the folder, or open it here.",
            count(users),
            count(number("users")),
            count(used),
            count(number("used")),
            count(drawn),
            plural(links, "link", "links"),
        ));
    } else if drawn < links {
        parts.push(format!(
            "Drawing {} of the {}.",
            count(drawn),
            plural(links, "link", "links")
        ));
    }
    parts.join(" ")
}
/// Beside the picture of a link: what the chosen name is and what it is
/// linked with here; with no name chosen, how to choose one and of which
/// kinds the links are.
pub fn link_side(view: &V, chosen: &str) -> String {
    let (names, _) = link_names(view);
    let Some((name, file)) = names.iter().find(|(name, _)| name["key"] == chosen) else {
        let kinds = m::array(&view["kinds"])
            .iter()
            .map(|kind| {
                format!(
                    "{} {}",
                    escape(&m::text(kind, "kind").replace('_', " ")),
                    count(kind["links"].as_u64().unwrap_or(0))
                )
            })
            .collect::<Vec<_>>();
        return format!(
            "<p class=\"knowledge-note\">Select a name to light its links. Select it again to open its file on it in the map.</p>{}",
            if kinds.is_empty() {
                String::new()
            } else {
                format!(
                    "<p class=\"knowledge-note\">Links by kind: {}</p>",
                    kinds.join(" · ")
                )
            }
        );
    };
    let line = name["line"].as_u64().unwrap_or(0);
    let mut html = format!(
        "<h3>{}</h3><p class=\"graph-map-path\">{}{}</p><p class=\"knowledge-note\">{}</p><button type=\"button\" class=\"button compact\" data-action=\"graph-link-open\" data-payload=\"{}\">Open in the map</button>",
        escape(&m::text(name, "name")),
        escape(&m::text(file, "path")),
        if line > 0 {
            format!(":{line}")
        } else {
            String::new()
        },
        escape(&capital(&what(name))),
        payload(&name["key"]),
    );
    // What it is linked with in this link, each with the way it is.
    let edges = m::array(&view["edges"]);
    for (title, own, other) in [("Uses here", "from", "to"), ("Used by here", "to", "from")] {
        let linked = edges
            .iter()
            .filter(|edge| edge[own] == name["key"])
            .filter_map(|edge| {
                let (other, _) = names.iter().find(|(name, _)| name["key"] == edge[other])?;
                Some((other, edge))
            })
            .collect::<Vec<_>>();
        if linked.is_empty() {
            continue;
        }
        html.push_str(&format!(
            "<h4>{title} <span class=\"graph-map-total\">{}</span></h4><ul class=\"graph-map-rows\">",
            count(linked.len() as u64)
        ));
        for (other, edge) in linked {
            html.push_str(&format!(
                "<li><button type=\"button\" class=\"graph-map-row\" data-action=\"graph-link-node\" data-payload=\"{}\"><span class=\"graph-map-row-name\">{}</span><span class=\"graph-map-row-links\">{}{}</span></button></li>",
                payload(&other["key"]),
                escape(&m::text(other, "name")),
                escape(&m::text(edge, "kind").replace('_', " ")),
                if m::yes(&edge["guessed"]) { ", guessed" } else { "" },
            ));
        }
        html.push_str("</ul>");
    }
    html
}
/// The rows of one side of a selected place. `look` names the selected
/// place and whether the rows are what it uses: each row then stands for a
/// link between two places, and a button beside it looks into that link.
fn rows(title: &str, rows: &V, more: &V, look: Option<(&V, bool)>) -> String {
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
            "<li><button type=\"button\" class=\"graph-map-row\" data-action=\"graph-map-go\" data-changed=\"{}\" data-payload=\"{}\" title=\"{}\"><span class=\"graph-map-row-name\">{}{}{}</span><span class=\"graph-map-row-links\">{}</span></button>{}</li>",
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
            match look {
                // A row of two symbols is one link, and says it all.
                Some((place, uses)) if row["relation"].is_null() => {
                    let (from, to) = if uses {
                        (place, &row["path"])
                    } else {
                        (&row["path"], place)
                    };
                    format!(
                        "<button type=\"button\" class=\"graph-map-row-look\" data-action=\"graph-map-link\" data-payload=\"{}\" title=\"See what makes up this link\" aria-label=\"See what makes up the link with {}\">↗</button>",
                        payload(&json!({"from": from, "to": to})),
                        escape(&m::text(row, "name")),
                    )
                }
                _ => String::new(),
            },
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
/// Follows the links of a folder or a file step by step, wherever they lead.
fn follow_links(path: &V) -> String {
    format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-follow\" data-payload=\"{}\">Follow links</button>",
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
            html.push_str(&rows(
                "Members",
                &focus["members"],
                &focus["members_more"],
                None,
            ));
        }
        html.push_str(&rows("Uses", &focus["uses"], &focus["uses_more"], None));
        html.push_str(&rows(
            "Used by",
            &focus["used_by"],
            &focus["used_by_more"],
            None,
        ));
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
    html.push_str(&follow_links(&focus["path"]));
    // Its own symbols at the other end of the links with this file.
    html.push_str(&rows(
        "Uses this file",
        &focus["uses"],
        &focus["uses_more"],
        None,
    ));
    html.push_str(&rows(
        "Used by this file",
        &focus["used_by"],
        &focus["used_by_more"],
        None,
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
    let followed = follows(view);
    if !focus.is_object() {
        return if followed {
            "<p class=\"knowledge-note\">Select a box to see what it uses and what uses it. + at the right corner of a box shows more of what it uses, + at the left corner more of what uses it, and − folds them back. Select a link to see what makes it up.</p>"
        } else if changes(view) {
            "<p class=\"knowledge-note\">Select a file to see what it uses and what uses it. The highlighted ones changed; the others use them or are used by them. Select a file again to open it to its classes, methods and functions. Select a link to see what makes it up.</p>"
        } else {
            "<p class=\"knowledge-note\">Select a folder or a file to see what it uses and what uses it. Select it again to open it: a folder to what it holds, a file to its classes, methods and functions. + at the corner of a folder opens it here, among the others. Select a link to see what makes it up.</p>"
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
        } else if followed {
            // Among followed links a place may be one of a chosen change.
            let mut told = if folder {
                format!(
                    "Folder · {}",
                    plural(focus["files"].as_u64().unwrap_or(0), "file", "files")
                )
            } else if held > 0 {
                format!("File · {}", plural(held, "symbol", "symbols"))
            } else {
                "File".to_owned()
            };
            match (changed, focus["status"].as_str()) {
                (0, _) => {}
                (_, Some(status)) => told.push_str(&format!(" · {}", happened(status))),
                _ => told.push_str(&format!(
                    " · {} inside",
                    plural(changed, "changed file", "changed files")
                )),
            }
            if m::yes(&focus["start"]) {
                told.push_str(" · the links are followed from here");
            }
            told
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
        if spreads(view) && m::yes(&focus["inside"]) {
            html.push_str(&format!(
                "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-spread\" data-payload=\"{}\">Open here</button>",
                payload(&focus["path"])
            ));
        }
    } else if held > 0 {
        html.push_str(&open_file(&focus["path"]));
    }
    // The place the links are followed from is followed already.
    if !m::yes(&focus["start"]) {
        html.push_str(&follow_links(&focus["path"]));
    }
    let (uses, used) = if beside {
        ("Uses in the change", "Used by the change")
    } else {
        ("Uses", "Used by")
    };
    // Each row is a link of the selected place, and can be looked into.
    let place = &focus["path"];
    html.push_str(&rows(
        uses,
        &focus["uses"],
        &focus["uses_more"],
        Some((place, true)),
    ));
    html.push_str(&rows(
        used,
        &focus["used_by"],
        &focus["used_by_more"],
        Some((place, false)),
    ));
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
                    "<span class=\"graph-map-crumb\" aria-current=\"location\">{}{}</span>",
                    escape(&m::text(crumb, "name")),
                    if follows(view) { ": links" } else { "" }
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
        _ if follows(view) => format!(
            "{LEGEND} + and − at the corners of a box unfold and fold what it uses, on the right, and what uses it, on the left.{}",
            if change { " Highlighted: changed." } else { "" }
        ),
        _ if !m::array(&view["frames"]).is_empty() => {
            format!("{LEGEND} A frame is a folder opened here: − at its corner closes it.")
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
/// The scale at which a picture lies wholly in the room it has: never
/// larger than it is drawn, and no smaller than `SMALLEST`. A picture or a
/// room without a size is left as it is.
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
pub fn scaled(size: (f64, f64), scale: f64) -> String {
    let plain = format!("width:{:.0}px;height:{:.0}px", size.0, size.1);
    if scale >= 1.0 {
        return plain;
    }
    format!(
        "{plain};transform:scale({scale:.4});transform-origin:0 0;margin:0 -{:.0}px -{:.0}px 0",
        size.0 * (1.0 - scale),
        size.1 * (1.0 - scale)
    )
}
/// Draws the picture of the map at the scale the viewer asked for: fitted
/// to the room it has, or as it is.
fn fit() {
    let Some(canvas) = dom::get("graph-map-canvas") else {
        return;
    };
    let Some(plane) = canvas
        .query_selector(".graph-map-plane")
        .ok()
        .flatten()
        .and_then(|plane| plane.dyn_into::<web_sys::HtmlElement>().ok())
    else {
        return;
    };
    // A box is as large as it is laid out, whatever scale it is drawn at.
    let size = (plane.offset_width() as f64, plane.offset_height() as f64);
    let scale = if m::yes(&state()["fit"]) {
        // A little less than the room, so that no rounding brings up a
        // scroll bar.
        scale(
            size,
            (
                canvas.client_width() as f64 - 2.0,
                canvas.client_height() as f64 - 2.0,
            ),
        )
    } else {
        1.0
    };
    let _ = plane.set_attribute("style", &scaled(size, scale));
}
/// What the pointer rests on in a picture: a box, by its number there, or a
/// link, by the numbers of its two ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pointed {
    Place(usize),
    Link(usize, usize),
}
/// The two ends a link is marked with.
fn ends_of(mark: &str) -> Option<(usize, usize)> {
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
pub fn lit_link(pointed: Pointed, link: (usize, usize)) -> bool {
    match pointed {
        Pointed::Place(place) => link.0 == place || link.1 == place,
        Pointed::Link(from, to) => link == (from, to),
    }
}
/// Whether a box is lit: the box itself and what its links lead to, or the
/// two ends of the link.
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
/// Lights what the pointer rests on in a picture, with what it is linked
/// to, and leaves the rest faint: a box with its links and the boxes at
/// their other ends, a link with its two ends. The picture is not drawn
/// again and the server is asked nothing; once the pointer has left, the
/// picture is as it was, with what was selected in it.
fn point(canvas: &str, target: Option<web_sys::Element>) {
    let Some(plane) = dom::all(&format!("#{canvas} .graph-map-plane"))
        .into_iter()
        .next()
    else {
        return;
    };
    let stands = |part: &web_sys::Element| {
        pointed(
            part.get_attribute("data-n")
                .or_else(|| part.get_attribute("data-of"))
                .as_deref(),
            part.get_attribute("data-ends").as_deref(),
        )
    };
    let at = target
        .and_then(|target| {
            target
                .closest("[data-n],[data-of],[data-ends]")
                .ok()
                .flatten()
        })
        .and_then(|part| stands(&part));
    // The pointer moves over the parts of one box many times.
    let told = at.map(|at| format!("{at:?}")).unwrap_or_default();
    if plane.get_attribute("data-pointed").unwrap_or_default() == told {
        return;
    }
    let _ = plane.set_attribute("data-pointed", &told);
    let parts = dom::all(&format!(
        "#{canvas} [data-n],#{canvas} [data-of],#{canvas} [data-ends]"
    ));
    let Some(at) = at else {
        let _ = plane.remove_attribute("data-hover");
        for part in &parts {
            let _ = part.remove_attribute("data-lit");
        }
        return;
    };
    let links = parts
        .iter()
        .filter_map(|part| ends_of(&part.get_attribute("data-ends")?))
        .collect::<Vec<_>>();
    for part in &parts {
        let lit = match stands(part) {
            Some(Pointed::Place(place)) => lit_place(at, &links, place),
            Some(Pointed::Link(from, to)) => lit_link(at, (from, to)),
            None => false,
        };
        let _ = if lit {
            part.set_attribute("data-lit", "true")
        } else {
            part.remove_attribute("data-lit")
        };
    }
    let _ = plane.set_attribute("data-hover", "true");
}
/// Fits the picture again when the window changes its size, and lights
/// what the pointer or the keyboard rests on in the map and in the window
/// of a link. Asked for each time the map opens, and set up once.
fn watch() {
    if m::yes(&m::data("graph-map-watched")) {
        return;
    }
    m::set("graph-map-watched", json!(true));
    let resized = Closure::<dyn FnMut()>::new(|| {
        if open() {
            fit();
        }
    });
    let _ =
        dom::window().add_event_listener_with_callback("resize", resized.as_ref().unchecked_ref());
    resized.forget();
    for canvas in ["graph-map-canvas", "graph-link-canvas"] {
        let Some(element) = dom::get(canvas) else {
            continue;
        };
        // Entering a part lights it; leaving the picture puts it out.
        for (kind, rests) in [
            ("mouseover", true),
            ("focusin", true),
            ("mouseleave", false),
            ("focusout", false),
        ] {
            let moved = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                let target = event
                    .target()
                    .and_then(|target| target.dyn_into::<web_sys::Element>().ok());
                point(canvas, target.filter(|_| rests));
            });
            let _ = element.add_event_listener_with_callback(kind, moved.as_ref().unchecked_ref());
            moved.forget();
        }
    }
}
fn fresh(target: &str) -> V {
    json!({"target": target, "scope": "", "focus": "", "certain": false, "notests": false, "fit": false, "filter": "", "follow": "", "out": [], "into": [], "spread": []})
}
/// How many folders a picture keeps spread out; the oldest gives way.
const SPREAD: usize = 32;
/// The state with a folder spread out in the picture, or, with `gather`,
/// drawn as one box again together with whatever was spread out inside it.
fn spread(mut state: V, folder: &str, gather: bool) -> V {
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
const NOTESTS: &str = "graph-map-notests";
const FIT: &str = "graph-map-fit";
/// A map begun anew, with tests left out and pictures fitted where they
/// were the last time.
fn anew(target: &str) -> V {
    let mut state = fresh(target);
    state["notests"] = json!(dom::storage(NOTESTS).as_deref() == Some("1"));
    state["fit"] = json!(dom::storage(FIT).as_deref() == Some("1"));
    state
}
/// The places a list of the state names: those whose side `key` (`out` or
/// `into`) is unfolded among followed links, each once for every step
/// unfolded, or the folders spread out in a picture (`spread`).
fn unfolded(state: &V, key: &str) -> Vec<String> {
    m::array(&state[key])
        .iter()
        .filter_map(|place| place.as_str().map(str::to_owned))
        .collect()
}
/// The state with no place followed.
fn unfollowed(mut state: V) -> V {
    state["follow"] = json!("");
    state["out"] = json!([]);
    state["into"] = json!([]);
    state
}
fn state() -> V {
    let state = m::data("graph-map");
    if state.is_object() {
        state
    } else {
        fresh("")
    }
}
/// The link the window shows: its two ends, the name chosen in it, and
/// what the server said of it.
fn looked() -> V {
    let link = m::data("graph-link");
    if link.is_object() {
        link
    } else {
        json!({"from": "", "to": "", "chosen": "", "view": null})
    }
}
/// Draws the window of the link as it stands.
fn draw_link() {
    let link = looked();
    let view = &link["view"];
    if !view.is_object() {
        return;
    }
    let chosen = m::text(&link, "chosen");
    dom::text("graph-link-title", &link_title(view));
    dom::text("graph-link-summary", &link_summary(view));
    dom::text("graph-link-note", &link_note(view));
    html("graph-link-canvas", &link_canvas(view, &chosen));
    html("graph-link-side", &link_side(view, &chosen));
}
/// Opens the window of one link of the map and reads what makes the link
/// up, on the graph the map is read from. The map stays as it is behind it.
async fn look(from: &str, to: &str) {
    let state = state();
    let project = dom::value("project");
    if project.is_empty() || from.is_empty() || to.is_empty() {
        return;
    }
    m::set(
        "graph-link",
        json!({"from": from, "to": to, "chosen": "", "view": null}),
    );
    dom::text("graph-link-title", "Link");
    dom::text("graph-link-summary", "");
    dom::text("graph-link-note", "Reading the link…");
    dom::error("graph-link-error", "");
    html("graph-link-canvas", "");
    html("graph-link-side", "");
    dom::dialog("graph-link", true);
    let mut query = vec![("from", from.to_owned()), ("to", to.to_owned())];
    if !m::text(&state, "target").is_empty() {
        query.push(("target", m::text(&state, "target")));
    }
    for key in ["certain", "notests"] {
        if m::yes(&state[key]) {
            query.push((key, "1".to_owned()));
        }
    }
    let path = format!(
        "/api/projects/{}/graph/map{}",
        dom::enc(&project),
        dom::query(&query)
    );
    match api::scoped("graph-link", &path).await {
        Ok((epoch, view)) => {
            if !m::current("graph-link", epoch) || dom::value("project") != project {
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
    dom::set_checked("graph-map-notests", m::yes(&view["notests"]));
    dom::set_checked("graph-map-fit", m::yes(&state()["fit"]));
    dom::disabled("graph-map-filter", follows(view));
    html("graph-map-canvas", &canvas(view));
    fit();
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
    for key in ["certain", "notests"] {
        if m::yes(&state[key]) {
            query.push((key, "1".to_owned()));
        }
    }
    // The unfolded places of each side, as a list a URL can carry whatever
    // the paths hold.
    for key in ["out", "into"] {
        let places = unfolded(&state, key);
        if !m::text(&state, "follow").is_empty() && !places.is_empty() {
            query.push((key, json!(places).to_string()));
        }
    }
    if !m::text(&state, "follow").is_empty() {
        query.push(("follow", m::text(&state, "follow")));
    } else if !unfolded(&state, "spread").is_empty() {
        // The folders spread out in the picture of the open one.
        query.push(("spread", json!(unfolded(&state, "spread")).to_string()));
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
            if follows(&view) {
                // What is no longer drawn is no longer unfolded.
                let drawn = m::array(&view["nodes"]);
                for key in ["out", "into"] {
                    let kept = unfolded(&state, key)
                        .into_iter()
                        .filter(|place| drawn.iter().any(|node| node["path"] == place.as_str()))
                        .collect::<Vec<_>>();
                    state[key] = json!(kept);
                }
            } else {
                // The graph does not know the place, or it was left.
                state = unfollowed(state);
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
            state = anew(if changes { "work" } else { "" });
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
            watch();
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
            state = anew(target);
            chosen = true;
        }
        ("change", "graph-map-target", _) => {
            shut();
            state = anew(value);
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
            state = unfollowed(state);
            state["scope"] = payload.clone();
            state["focus"] = json!("");
            state["filter"] = json!("");
            dom::set_value("graph-map-filter", "");
        }
        ("click", _, "graph-map-follow") => {
            // The first picture is one step each way: what it uses and
            // what uses it. Nothing is selected, so no box of it is dimmed.
            state["follow"] = payload.clone();
            state["out"] = json!([payload]);
            state["into"] = json!([payload]);
            state["focus"] = json!("");
            state["filter"] = json!("");
            dom::set_value("graph-map-filter", "");
        }
        ("click", _, "graph-map-unfold" | "graph-map-fold") => {
            let key = if payload["side"] == "in" {
                "into"
            } else {
                "out"
            };
            let mut places = unfolded(&state, key);
            let place = payload["path"].as_str().unwrap_or("");
            if action == "graph-map-fold" {
                places.retain(|other| other != place);
            } else {
                // Named once more, a place shows the next of that side.
                places.push(place.to_owned());
            }
            state[key] = json!(places);
        }
        ("click", _, "graph-map-link") => {
            look(
                payload["from"].as_str().unwrap_or(""),
                payload["to"].as_str().unwrap_or(""),
            )
            .await;
            return true;
        }
        ("click", "graph-link-close", _) => {
            dom::dialog("graph-link", false);
            return true;
        }
        ("click", _, "graph-link-node" | "graph-link-open") => {
            let key = payload.as_str().unwrap_or("");
            let mut link = looked();
            if action == "graph-link-node" && link["chosen"] != key {
                link["chosen"] = json!(key);
                m::set("graph-link", link);
                draw_link();
                return true;
            }
            // Selected again, or asked for: its file opens on it in the map.
            let (names, _) = link_names(&link["view"]);
            let Some((_, file)) = names.iter().find(|(name, _)| name["key"] == key) else {
                return true;
            };
            dom::dialog("graph-link", false);
            state = unfollowed(state);
            state["scope"] = file["path"].clone();
            state["focus"] = json!(key);
            state["filter"] = json!("");
            dom::set_value("graph-map-filter", "");
        }
        ("click", _, "graph-map-spread" | "graph-map-gather") => {
            state = spread(
                state,
                payload.as_str().unwrap_or(""),
                action == "graph-map-gather",
            );
        }
        ("click", _, "graph-map-go") => {
            let path = payload["path"].as_str().unwrap_or("");
            if let Some(file) = payload["open"].as_str() {
                // A symbol of another file: that file opens on it.
                state = unfollowed(state);
                state["scope"] = json!(file);
                state["filter"] = json!("");
            } else if !m::yes(&payload["inside"]) {
                // It is not drawn here: show it among its own neighbours,
                // in the whole project.
                state = unfollowed(state);
                state["target"] = json!("");
                state["scope"] = json!(parent(path));
                state["filter"] = json!("");
            }
            state["focus"] = json!(path);
        }
        ("change", "graph-map-certain", _) => state["certain"] = json!(checked),
        ("change", "graph-map-notests", _) => {
            // The choice is the viewer's and holds for the next map too.
            state["notests"] = json!(checked);
            dom::save(NOTESTS, checked.then_some("1"));
        }
        ("change", "graph-map-fit", _) => {
            // So does this one, and it asks the server for nothing: the
            // picture that is there is drawn at another scale.
            state["fit"] = json!(checked);
            dom::save(FIT, checked.then_some("1"));
            m::set("graph-map", state);
            fit();
            return true;
        }
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
        assert!(html.starts_with("<h3>Price.php</h3><p class=\"graph-map-path\">src/Cart/Price.php</p><p class=\"knowledge-note\">Changed file · added · used by this file · 3 symbols</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-open\" data-payload=\"&quot;src/Cart/Price.php&quot;\">Open file</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-follow\" data-payload=\"&quot;src/Cart/Price.php&quot;\">Follow links</button><h4>Uses this file <span"), "{html}");
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
        // A file the graph names nothing in does not open; it is followed.
        folder["focus"]["symbols"] = json!(0);
        assert!(side(&folder).contains("<p class=\"knowledge-note\">File</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-follow\""));
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
        // What the manifests of Rust crates rule out is said after it.
        let mut ruled = view();
        ruled["totals"]["ruled_out"] = json!(1157);
        assert!(summary(&ruled)
            .ends_with("270 skipped, their file is unclear · 1,157 ruled out by Cargo manifests"));
        ruled["totals"]["ruled_out"] = json!(0);
        assert_eq!(summary(&ruled), summary(&view()));
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
    /// The links of `lib/core.php` followed: what it uses is unfolded, and
    /// so is what uses the changed `lib/db.php`, of which three are not drawn.
    fn trail() -> V {
        json!({
            "mode": "follow", "scope": "lib", "certain": false,
            "crumbs": [{"name": "office", "path": ""}, {"name": "lib", "path": "lib"}, {"name": "core.php", "path": "lib/core.php"}],
            "totals": {"files": 21, "symbols": 22, "relations": 22, "links": 22, "read": 21, "guessed": 1, "unplaced": 0},
            "followed": {"path": "lib/core.php", "name": "core.php", "folder": false, "uses": 2, "users": 2, "shown": 4, "limit": 60, "step": 12, "full": false},
            "children": 4, "matching": 4,
            "nodes": [
                {"path": "lib/core.php", "name": "core.php", "folder": false, "files": 1, "symbols": 2, "within": 0, "out": 3, "in": 2, "start": true, "more_out": 0, "more_in": 2, "unfolded_out": true, "unfolded_in": false, "changed": 0, "status": null},
                {"path": "lib/db.php", "name": "db.php", "folder": false, "files": 1, "symbols": 1, "within": 0, "out": 0, "in": 17, "start": false, "more_out": 0, "more_in": 3, "unfolded_out": false, "unfolded_in": true, "changed": 1, "status": "M"},
                {"path": "lib/deep", "name": "deep", "folder": true, "files": 1, "symbols": 1, "within": 0, "out": 0, "in": 1, "start": false, "more_out": 0, "more_in": 0, "unfolded_out": false, "unfolded_in": false, "changed": 0, "status": null},
                {"path": "reports/r01.php", "name": "r<01>.php", "folder": false, "files": 1, "symbols": 1, "within": 0, "out": 1, "in": 0, "start": false, "more_out": 0, "more_in": 0, "unfolded_out": false, "unfolded_in": false, "changed": 0, "status": null}
            ],
            "between": 3, "backbone": false,
            "edges": [
                {"from": "lib/core.php", "to": "lib/db.php", "links": 2, "guessed": 0},
                {"from": "lib/core.php", "to": "lib/deep", "links": 1, "guessed": 0},
                {"from": "reports/r01.php", "to": "lib/db.php", "links": 1, "guessed": 0}
            ],
            "focus": null
        })
    }
    #[test]
    fn followed_links_say_where_they_start_and_how_they_are_unfolded() {
        let view = trail();
        assert_eq!(
            summary(&view),
            "Links of core.php, step by step · uses 2 other files · 2 other files use it · 4 drawn"
        );
        assert_eq!(
            note(&view),
            "+ at the right corner of a box shows more of what it uses, + at the left corner more of what uses it, 12 at a time, strongest first; − folds them back."
        );
        // The place they start from closes the path, marked as followed.
        assert!(crumbs(&view).ends_with(
            "<span class=\"graph-map-crumb\" aria-current=\"location\">core.php: links</span>"
        ));
        assert!(!crumbs(&super::tests::view()).contains(": links"));
        let told = legend(&view, false);
        assert!(
            told.contains("Dotted: against the main direction"),
            "{told}"
        );
        assert!(told.ends_with("+ and − at the corners of a box unfold and fold what it uses, on the right, and what uses it, on the left."));
        assert!(legend(&view, true).ends_with("on the left. Highlighted: changed."));
        // A folder is followed among folders and files, a single user is one.
        let mut folder = trail();
        folder["followed"] = json!({"path": "lib", "name": "lib", "folder": true, "uses": 0, "users": 1, "shown": 60, "limit": 60, "step": 12, "full": true});
        folder["between"] = json!(200);
        assert_eq!(
            summary(&folder),
            "Links of lib, step by step · uses nothing else · 1 other uses it · 60 drawn"
        );
        folder["followed"]["aside"] = json!(7);
        assert!(note(&folder).ends_with(
            "The picture holds 60 boxes: fold something to unfold more. 7 more links run between these boxes; they show where a side that holds them is unfolded. Drawing the 3 strongest of 200 links."
        ));
        folder["followed"]["aside"] = json!(1);
        assert!(note(&folder).contains("1 more link runs between these boxes; it shows where"));
        // Inside a chosen change the graph may be behind the files.
        let mut stale = trail();
        stale["stale"] = json!(true);
        assert!(note(&stale).starts_with("The graph is older than the files"));
    }
    #[test]
    fn a_box_of_followed_links_has_a_handle_at_each_corner_that_has_more() {
        let html = canvas(&trail());
        assert_eq!(
            html.matches("<button type=\"button\" class=\"graph-map-node\"")
                .count(),
            4
        );
        // A box is only selected here: opening it would leave the links.
        assert_eq!(html.matches("data-open=\"true\"").count(), 0);
        assert_eq!(html.matches("data-start=\"true\"").count(), 1);
        assert!(html.contains("aria-label=\"core.php, file, uses others through 3 links, is used through 2 links, the links are followed from here\""));
        // The changed file keeps Git's letter.
        assert!(html.contains("aria-label=\"db.php, file, uses others through 0 links, is used through 17 links, modified\""));
        // core.php stands in the first column and db.php in the second.
        let (core, db) = (MARGIN, MARGIN + BOX_WIDTH + COLUMN_GAP);
        // What core.php uses is all drawn and can be folded; of what uses it
        // two are not drawn.
        assert!(html.contains(&format!(
            "data-side=\"out\" data-dim=\"false\" data-action=\"graph-map-fold\" data-payload=\"{}\" style=\"left:{}px;top:",
            payload(&json!({"path": "lib/core.php", "side": "out"})),
            core + BOX_WIDTH
        )), "{html}");
        assert!(html.contains(
            "title=\"Hide what core.php uses\" aria-label=\"Hide what core.php uses\">−</button>"
        ));
        assert!(html.contains(&format!(
            "data-side=\"in\" data-hang=\"\" data-dim=\"false\" data-action=\"graph-map-unfold\" data-payload=\"{}\" style=\"left:{core}px;top:",
            payload(&json!({"path": "lib/core.php", "side": "in"}))
        )), "{html}");
        assert!(html.contains("title=\"Show what uses core.php: 2 not drawn\" aria-label=\"Show what uses core.php: 2 not drawn\">+2</button>"));
        // What uses db.php is unfolded and three more are not drawn: the +
        // hangs outside the corner, beside the −.
        assert!(html.contains(&format!("style=\"left:{db}px;top:")));
        assert!(html.contains("title=\"Hide what uses db.php\""));
        assert!(html.contains(&format!(
            "data-side=\"in\" data-hang=\"before\" data-dim=\"false\" data-action=\"graph-map-unfold\" data-payload=\"{}\" style=\"left:{}px;top:",
            payload(&json!({"path": "lib/db.php", "side": "in"})),
            db - HANDLE_GAP
        )));
        assert!(html.contains("title=\"Show more of what uses db.php: 3 not drawn\""));
        // Two handles to fold, two to unfold, and none where nothing is left.
        assert_eq!(html.matches("data-action=\"graph-map-fold\"").count(), 2);
        assert_eq!(html.matches("data-action=\"graph-map-unfold\"").count(), 2);
        // A name from the project never becomes markup, in a handle either.
        let mut odd = trail();
        odd["nodes"][3]["more_out"] = json!(1234);
        let html = canvas(&odd);
        assert!(html.contains("title=\"Show what r&lt;01&gt;.php uses: 1,234 not drawn\""));
        assert!(html.contains(">+1,234</button>"));
        assert!(!html.contains("r<01>"));
        // With a box selected, the handles of the boxes away from it dim.
        let mut selected = trail();
        selected["focus"] = json!({"path": "lib/deep", "name": "deep"});
        let html = canvas(&selected);
        assert_eq!(
            html.matches("data-dim=\"true\" data-action=\"graph-map-")
                .count(),
            2
        );
        assert_eq!(
            html.matches("data-dim=\"false\" data-action=\"graph-map-")
                .count(),
            2
        );
        // No other map has handles.
        assert!(!canvas(&super::tests::view()).contains("graph-map-more"));
    }
    #[test]
    fn a_map_without_tests_says_what_it_leaves_out() {
        let mut view = super::tests::view();
        let whole = summary(&view);
        assert_eq!(left_out(&view), "");
        view["tests"] = json!({"files": 397, "inside": 1208});
        assert_eq!(
            summary(&view),
            format!("{whole} · without 397 test files and 1,208 tests inside other files")
        );
        view["tests"] = json!({"files": 1, "inside": 0});
        assert_eq!(summary(&view), format!("{whole} · without 1 test file"));
        view["tests"] = json!({"files": 0, "inside": 1});
        assert_eq!(
            summary(&view),
            format!("{whole} · without 1 test inside its files")
        );
        view["tests"] = json!({"files": 0, "inside": 0});
        assert_eq!(
            summary(&view),
            format!("{whole} · no tests found to leave out")
        );
        // A change, an opened file and followed links say it as a folder does.
        for mut other in [change(), file(), trail()] {
            let whole = summary(&other);
            other["tests"] = json!({"files": 2, "inside": 0});
            assert_eq!(summary(&other), format!("{whole} · without 2 test files"));
        }
        // A map begins with its tests, until the viewer chose otherwise.
        assert_eq!(fresh("work")["notests"], false);
    }
    #[test]
    fn the_side_panel_offers_to_follow_links_and_tells_a_followed_place() {
        // A selected folder or file of any map can be followed.
        let folder = side(&super::tests::view());
        assert!(
            folder.contains(&format!(
                "data-action=\"graph-map-follow\" data-payload=\"{}\">Follow links</button>",
                payload(&json!("src/Order"))
            )),
            "{folder}"
        );
        let nothing = side(&trail());
        assert!(nothing.contains("Select a box to see what it uses and what uses it."));
        assert!(nothing.contains("+ at the right corner"));
        // The place the links are followed from is followed already.
        let mut start = trail();
        start["focus"] = json!({
            "path": "lib/core.php", "name": "core.php", "folder": false, "inside": true, "files": 1, "symbols": 2,
            "start": true, "changed": 0, "status": null,
            "uses": [{"path": "lib/db.php", "name": "lib/db.php", "folder": false, "inside": true, "links": 2, "guessed": 0, "changed": true}],
            "uses_more": 0, "used_by": [], "used_by_more": 0, "kinds": [{"kind": "calls", "links": 3}]
        });
        let html = side(&start);
        assert!(
            html.contains("File · 2 symbols · the links are followed from here"),
            "{html}"
        );
        assert!(html.contains("data-action=\"graph-map-open\""));
        assert!(!html.contains("graph-map-follow"));
        assert!(html.contains("<h4>Uses <span class=\"graph-map-total\">1</span></h4>"));
        assert!(html.contains("Uses by kind: calls 3"));
        // Another place of the picture, changed, can be followed in turn.
        let mut other = trail();
        other["focus"] = json!({
            "path": "lib/db.php", "name": "db.php", "folder": false, "inside": true, "files": 1, "symbols": 1,
            "start": false, "changed": 1, "status": "M",
            "uses": [], "uses_more": 0, "used_by": [], "used_by_more": 0, "kinds": []
        });
        let html = side(&other);
        assert!(html.contains("File · 1 symbol · modified"), "{html}");
        assert!(html.contains("graph-map-follow"));
        let mut folder = trail();
        folder["focus"] = json!({
            "path": "lib/deep", "name": "deep", "folder": true, "inside": true, "files": 4, "symbols": 9,
            "start": false, "changed": 2, "status": null,
            "uses": [], "uses_more": 0, "used_by": [], "used_by_more": 0, "kinds": []
        });
        let html = side(&folder);
        assert!(
            html.contains("Folder · 4 files · 2 changed files inside"),
            "{html}"
        );
        assert!(html.contains(">Open folder</button>"));
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
    /// A column of the layout, by number.
    fn column(at: usize, inset: f64) -> f64 {
        MARGIN + inset + at as f64 * (BOX_WIDTH + COLUMN_GAP)
    }
    #[test]
    fn a_frame_is_a_band_across_the_layers_above_what_lies_in_none() {
        // a uses x, x uses y, y uses b, and a uses b past them. x, y and z
        // are of one spread-out folder; z and c are linked to nothing.
        let frames = Frames {
            of: vec![None, Some(0), Some(0), None, Some(0), None],
            holder: vec![None],
        };
        let plan = plan_with(6, &[(0, 1, 1), (1, 2, 1), (2, 3, 1), (0, 3, 1)], &frames);
        // The layers are those of the whole picture.
        assert_eq!(
            (0..4).map(|node| layer(&plan, node)).collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
        let (frame, inset) = (&plan.frames[0], FRAME_PAD);
        assert_eq!(
            (frame.x, frame.y, frame.width, frame.height),
            (
                column(1, inset) - FRAME_PAD,
                MARGIN,
                BOX_WIDTH + COLUMN_GAP + BOX_WIDTH + 2.0 * FRAME_PAD,
                FRAME_HEAD + BOX_HEIGHT + ROW_GAP + BOX_HEIGHT + FRAME_PAD
            )
        );
        // What the frame holds stands in its layers, under the room for the
        // name; what no link touches lies under that, still in the frame.
        let inside = MARGIN + FRAME_HEAD;
        assert_eq!(
            (plan.nodes[1].x, plan.nodes[1].y),
            (column(1, inset), inside)
        );
        assert_eq!(
            (plan.nodes[2].x, plan.nodes[2].y),
            (column(2, inset), inside)
        );
        assert_eq!(
            (plan.nodes[4].x, plan.nodes[4].y, plan.nodes[4].layer),
            (column(1, inset), inside + BOX_HEIGHT + ROW_GAP, None)
        );
        // Everything else lies under the frame: the two ends of the picture
        // and, further down, what is linked to nothing.
        let under = frame.y + frame.height + MARGIN;
        assert_eq!(
            (plan.nodes[0].x, plan.nodes[0].y),
            (column(0, inset), under)
        );
        assert_eq!(
            (plan.nodes[3].x, plan.nodes[3].y),
            (column(3, inset), under)
        );
        assert_eq!(plan.apart, Some(under + BOX_HEIGHT + 2.0 * MARGIN));
        assert_eq!(
            (plan.nodes[5].x, plan.nodes[5].y),
            (MARGIN + inset, under + BOX_HEIGHT + 2.0 * MARGIN + CAPTION)
        );
        // The link that passes the frame's layers passes under the frame.
        let past = plan.wires.iter().find(|wire| wire.edge == 3).unwrap();
        let heights = past
            .path
            .split(|letter: char| !(letter.is_ascii_digit() || letter == '.'))
            .filter(|part| !part.is_empty())
            .map(|part| part.parse::<f64>().unwrap())
            .skip(1)
            .step_by(2)
            .collect::<Vec<_>>();
        assert!(heights.iter().all(|y| *y >= under), "{}", past.path);
        assert!(plan.width >= frame.x + frame.width + MARGIN);
        assert_eq!(plan.height, plan.nodes[5].y + BOX_HEIGHT + ROW_GAP);
        // Without frames the picture is as it was.
        assert!(plan_with(4, &[(0, 1, 1)], &Frames::default())
            .frames
            .is_empty());
        assert_eq!(
            plan_with(4, &[(0, 1, 1), (2, 1, 1)], &Frames::default()),
            super::plan(4, &[(0, 1, 1), (2, 1, 1)])
        );
    }
    #[test]
    fn a_frame_lies_inside_the_frame_of_its_holder() {
        // x of the inner folder uses y of the outer one, which uses b.
        let frames = Frames {
            of: vec![Some(1), Some(0), None],
            holder: vec![None, Some(0)],
        };
        let plan = plan_with(3, &[(0, 1, 1), (1, 2, 1)], &frames);
        let (outer, inner) = (&plan.frames[0], &plan.frames[1]);
        assert!(inner.x > outer.x && inner.x + inner.width < outer.x + outer.width);
        assert!(inner.y > outer.y && inner.y + inner.height < outer.y + outer.height);
        // The inner frame holds x alone, above what the outer one holds
        // itself; the picture begins further in for the two of them.
        let inset = 2.0 * FRAME_PAD;
        assert_eq!(
            (inner.x, inner.width),
            (column(0, inset) - FRAME_PAD, BOX_WIDTH + 2.0 * FRAME_PAD)
        );
        assert_eq!(
            (outer.x, outer.width),
            (
                column(0, inset) - inset,
                BOX_WIDTH + COLUMN_GAP + BOX_WIDTH + 2.0 * inset
            )
        );
        assert!(plan.nodes[0].y >= inner.y + FRAME_HEAD);
        assert!(plan.nodes[0].y + BOX_HEIGHT <= inner.y + inner.height);
        assert!(plan.nodes[1].y >= inner.y + inner.height);
        assert!(plan.nodes[1].y + BOX_HEIGHT <= outer.y + outer.height);
        assert!(plan.nodes[2].y >= outer.y + outer.height);
        // A frame that names a later one as its holder stands on its own.
        let odd = Frames {
            of: vec![Some(0), Some(1)],
            holder: vec![Some(1), Some(7)],
        };
        let plan = plan_with(2, &[(0, 1, 1)], &odd);
        assert!(plan.frames[0].y + plan.frames[0].height <= plan.frames[1].y);
    }
    #[test]
    fn a_frame_of_what_is_linked_to_nothing_is_a_grid_of_its_own() {
        // Five places of one folder and one beside them, no link anywhere.
        let frames = Frames {
            of: vec![Some(0), Some(0), Some(0), Some(0), Some(0), None],
            holder: vec![None],
        };
        let plan = plan_with(6, &[], &frames);
        let frame = &plan.frames[0];
        // Three to a row, two rows, in a frame at the corner of the picture.
        assert_eq!((frame.x, frame.y), (MARGIN, MARGIN));
        assert_eq!(
            frame.width,
            3.0 * BOX_WIDTH + 2.0 * COLUMN_GAP + 2.0 * FRAME_PAD
        );
        assert_eq!(
            frame.height,
            FRAME_HEAD + 2.0 * BOX_HEIGHT + ROW_GAP + FRAME_PAD
        );
        assert_eq!(
            (0..5)
                .map(|node| (plan.nodes[node].x, plan.nodes[node].y))
                .collect::<Vec<_>>(),
            [
                (column(0, FRAME_PAD), MARGIN + FRAME_HEAD),
                (column(1, FRAME_PAD), MARGIN + FRAME_HEAD),
                (column(2, FRAME_PAD), MARGIN + FRAME_HEAD),
                (
                    column(0, FRAME_PAD),
                    MARGIN + FRAME_HEAD + BOX_HEIGHT + ROW_GAP
                ),
                (
                    column(1, FRAME_PAD),
                    MARGIN + FRAME_HEAD + BOX_HEIGHT + ROW_GAP
                ),
            ]
        );
        // The one beside them lies under the frame, and no caption parts
        // them: nothing here is linked.
        assert_eq!(plan.apart, None);
        assert_eq!(plan.nodes[5].y, frame.y + frame.height + MARGIN);
        assert!(plan.width >= frame.x + frame.width + MARGIN);
    }
    /// A folder with one of its folders spread out: `src/Cart` gives way to
    /// two of the five places it holds.
    fn spread_view() -> V {
        json!({
            "scope": "src", "certain": false, "children": 7, "matching": 7, "between": 3,
            "crumbs": [{"name": "shop", "path": ""}, {"name": "src", "path": "src"}],
            "totals": {"files": 1115, "links": 4165, "read": 115, "guessed": 4050, "unplaced": 0},
            "nodes": [
                {"path": "src/Cart/Cart.php", "name": "Cart.php", "folder": false, "holder": "src/Cart", "files": 1, "symbols": 2, "out": 1, "in": 2, "within": 0},
                {"path": "src/Cart/Rules", "name": "Rules", "folder": true, "holder": "src/Cart", "files": 3, "out": 0, "in": 1, "within": 1},
                {"path": "src/Order", "name": "Order", "folder": true, "holder": null, "files": 1, "out": 2, "in": 0, "within": 0},
                {"path": "src/a<b>.php", "name": "a<b>.php", "folder": false, "holder": null, "files": 1, "out": 0, "in": 0, "within": 0}
            ],
            "frames": [{"path": "src/Cart", "name": "Cart", "holder": null, "places": 5, "drawn": 2}],
            "edges": [
                {"from": "src/Order", "to": "src/Cart/Cart.php", "links": 2, "guessed": 0},
                {"from": "src/Cart/Cart.php", "to": "src/Cart/Rules", "links": 1, "guessed": 1}
            ],
            "focus": null
        })
    }
    #[test]
    fn a_spread_out_folder_is_drawn_as_a_frame_with_its_name_and_a_way_back() {
        let html = canvas(&spread_view());
        // One frame, behind the links, of the open folder itself.
        assert_eq!(html.matches("<rect class=\"graph-map-frame\"").count(), 1);
        assert!(html.contains("<rect class=\"graph-map-frame\" data-inner=\"false\""));
        assert!(html.find("graph-map-frame\"").unwrap() < html.find("graph-map-edge").unwrap());
        // Its head: − gathers the folder, then its name and how much of it
        // is drawn.
        assert!(
            html.contains("data-action=\"graph-map-gather\" data-payload=\"&quot;src/Cart&quot;\"")
        );
        assert!(html.contains("title=\"Draw Cart as one box again\" aria-label=\"Draw Cart as one box again\">−</button>"));
        assert!(html.contains("title=\"src/Cart\">Cart<span class=\"graph-map-frame-count\"> · 2 of 5 drawn</span></span>"));
        // A folder has + at its corner, in the frame as outside it; a file
        // has none.
        assert_eq!(html.matches("data-action=\"graph-map-spread\"").count(), 2);
        assert!(html.contains(
            "data-action=\"graph-map-spread\" data-payload=\"&quot;src/Cart/Rules&quot;\""
        ));
        assert!(html.contains("title=\"Open Order here, among the others\""));
        // What the frame holds lies in it; the rest lies under it.
        let frames = Frames {
            of: vec![Some(0), Some(0), None, None],
            holder: vec![None],
        };
        let plan = plan_with(4, &[(2, 0, 2), (0, 1, 1)], &frames);
        let bottom = plan.frames[0].y + plan.frames[0].height;
        assert!(plan.nodes[0].y + BOX_HEIGHT < bottom && plan.nodes[1].y + BOX_HEIGHT < bottom);
        assert!(plan.nodes[2].y > bottom && plan.nodes[3].y > bottom);
        // A frame that holds all of its folder says no more than its name.
        let mut whole = spread_view();
        whole["frames"][0]["drawn"] = json!(5);
        assert!(canvas(&whole).contains("title=\"src/Cart\">Cart</span>"));
        // A frame inside another is told apart.
        let mut nested = spread_view();
        nested["frames"] = json!([
            {"path": "src/Cart", "name": "Cart", "holder": null, "places": 2, "drawn": 1},
            {"path": "src/Cart/Rules", "name": "Rules", "holder": "src/Cart", "places": 1, "drawn": 1}
        ]);
        nested["nodes"][1] = json!({"path": "src/Cart/Rules/Tax.php", "name": "Tax.php", "folder": false, "holder": "src/Cart/Rules", "files": 1, "out": 0, "in": 1, "within": 0});
        nested["edges"][1]["to"] = json!("src/Cart/Rules/Tax.php");
        let html = canvas(&nested);
        assert_eq!(html.matches("<rect class=\"graph-map-frame\"").count(), 2);
        assert!(html.contains("<rect class=\"graph-map-frame\" data-inner=\"true\""));
        // Only the picture of a folder of the whole project spreads folders
        // out: no other picture has the handles, and a folder view from an
        // older server draws as before.
        for other in [super::tests::view(), change(), file(), trail()] {
            assert!(!canvas(&other).contains("graph-map-spread"));
        }
        // The legend tells a frame once there is one.
        assert!(legend(&spread_view(), false).contains("A frame is a folder opened here"));
        assert!(!legend(&super::tests::view(), false).contains("frame"));
    }
    #[test]
    fn a_selected_folder_is_opened_here_and_a_spread_out_one_is_remembered() {
        let mut view = spread_view();
        view["focus"] = json!({
            "path": "src/Order", "name": "Order", "folder": true, "inside": true, "files": 1, "within": 0,
            "uses": [], "uses_more": 0, "used_by": [], "used_by_more": 0, "kinds": []
        });
        let html = side(&view);
        assert!(html.contains("data-action=\"graph-map-open\" data-payload=\"&quot;src/Order&quot;\">Open folder</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-spread\" data-payload=\"&quot;src/Order&quot;\">Open here</button>"));
        // A folder outside the open one is not in the picture to open here,
        // nor is a file; another picture does not offer it.
        view["focus"]["inside"] = json!(false);
        assert!(!side(&view).contains("Open here"));
        assert!(!side(&super::tests::view()).contains("Open here"));
        assert!(side(&json!({"focus": null})).contains("+ at the corner of a folder opens it here"));
        // The state: a folder is named once, the newest last.
        let state = spread(fresh(""), "src/Cart", false);
        let state = spread(state, "src", false);
        let state = spread(state, "src/Cart", false);
        assert_eq!(state["spread"], json!(["src", "src/Cart"]));
        // Gathered, it goes with what was spread out inside it, and nothing
        // else does.
        let state = spread(state, "src/Cart/Rules", false);
        let state = spread(state, "src/Cartoon", false);
        let state = spread(state, "src/Cart", true);
        assert_eq!(state["spread"], json!(["src", "src/Cartoon"]));
        // A selected folder that is spread out is selected no more.
        let mut chosen = fresh("");
        chosen["focus"] = json!("lib");
        assert_eq!(spread(chosen.clone(), "lib", false)["focus"], "");
        assert_eq!(spread(chosen, "src", false)["focus"], "lib");
        // Only so many are kept; the oldest gives way.
        let mut many = fresh("");
        for number in 0..SPREAD + 3 {
            many = spread(many, &format!("f{number}"), false);
        }
        let kept = unfolded(&many, "spread");
        assert_eq!(kept.len(), SPREAD);
        assert_eq!((kept[0].as_str(), kept[SPREAD - 1].as_str()), ("f3", "f34"));
        assert_eq!(fresh("work")["spread"], json!([]));
    }
    #[test]
    fn the_ends_of_a_link_stand_in_two_columns_file_by_file() {
        // Two files use one: the first file with two names, the second
        // with one; the file that is used holds two names.
        let plan = ends(&[2, 1], &[2], &[(0, 3), (0, 4), (1, 3), (2, 3)]);
        let (left, right) = (MARGIN + FRAME_PAD, MARGIN + FRAME_PAD + LINK_BOX + LINK_GAP);
        let frame = LINK_BOX + 2.0 * FRAME_PAD;
        let first = MARGIN + FRAME_HEAD;
        let second = first + BOX_HEIGHT + LINK_ROW;
        // Each file is a frame around its names; the next begins under it.
        let other = first + 2.0 * BOX_HEIGHT + LINK_ROW + FRAME_PAD + LINK_BETWEEN;
        assert_eq!(
            plan.names[..3],
            [(left, first), (left, second), (left, other + FRAME_HEAD)]
        );
        assert_eq!(
            plan.files[..2],
            [
                Framed {
                    x: MARGIN,
                    y: MARGIN,
                    width: frame,
                    height: FRAME_HEAD + 2.0 * BOX_HEIGHT + LINK_ROW + FRAME_PAD
                },
                Framed {
                    x: MARGIN,
                    y: other,
                    width: frame,
                    height: FRAME_HEAD + BOX_HEIGHT + FRAME_PAD
                },
            ]
        );
        // The other end begins at the top as well, and the name used only
        // by the topmost name stands on top there.
        assert_eq!(plan.names[3..], [(right, second), (right, first)]);
        assert_eq!(
            plan.files[2],
            Framed {
                x: right - FRAME_PAD,
                y: MARGIN,
                width: frame,
                height: FRAME_HEAD + 2.0 * BOX_HEIGHT + LINK_ROW + FRAME_PAD
            }
        );
        assert_eq!(
            (plan.width, plan.height),
            (
                2.0 * (MARGIN + FRAME_PAD + LINK_BOX) + LINK_GAP,
                other + FRAME_HEAD + BOX_HEIGHT + FRAME_PAD + MARGIN
            )
        );
        // A link runs from the right edge of what uses to the left edge of
        // what is used, at mid height.
        assert_eq!(plan.wires.len(), 4);
        assert!(plan.wires[0].starts_with(&format!(
            "M{:.1} {:.1} C",
            left + LINK_BOX,
            first + BOX_HEIGHT / 2.0
        )));
        assert!(plan.wires[0].ends_with(&format!(
            " {:.1} {:.1}",
            right,
            second + BOX_HEIGHT / 2.0
        )));
        // Two links that would cross are untangled: what the second name
        // uses is put under what the first uses.
        let crossed = ends(&[2], &[2], &[(0, 3), (1, 2)]);
        assert!(crossed.names[3].1 < crossed.names[2].1);
        // And files change places as their names do.
        let files = ends(&[1, 1], &[1, 1], &[(0, 3), (1, 2)]);
        assert!(files.files[3].y < files.files[2].y);
        // A link that names nothing here is no curve, and an empty link is
        // an empty picture of some size.
        assert_eq!(ends(&[1], &[1], &[(0, 9), (1, 0)]).wires, ["", ""]);
        let empty = ends(&[], &[], &[]);
        assert!(empty.names.is_empty() && empty.files.is_empty());
        assert!(empty.height > 0.0 && empty.width > 0.0);
    }
    /// The link between two folders, looked into: three names of two files
    /// use two names of one file.
    fn link_view() -> V {
        json!({
            "mode": "link", "certain": false,
            "from": {"path": "src/Order", "name": "Order", "what": "folder"},
            "to": {"path": "src/Cart", "name": "Cart", "what": "folder"},
            "links": 4, "guessed": 1,
            "users": {"names": 3, "files": 2, "groups": [
                {"path": "src/Order/Order.php", "name": "Order.php", "names": [
                    {"key": "src/Order/Order.php#L3:Order", "name": "Order", "kind": "type", "line": 3, "owner": null, "links": 2},
                    {"key": "src/Order/Order.php#L9:.total()", "name": "Order.total()", "kind": "method", "line": 9, "owner": "Order", "links": 1}
                ]},
                {"path": "src/Order/Notifier.php", "name": "Notifier.php", "names": [
                    {"key": "src/Order/Notifier.php#L5:notify()", "name": "notify()", "kind": "function", "line": 5, "owner": null, "links": 1}
                ]}
            ]},
            "used": {"names": 2, "files": 1, "groups": [
                {"path": "src/Cart/Cart.php", "name": "Cart.php", "names": [
                    {"key": "src/Cart/Cart.php#L4:Cart", "name": "Cart", "kind": "type", "line": 4, "owner": null, "links": 3},
                    {"key": "src/Cart/Cart.php#L20:.add<T>()", "name": "Cart.add<T>()", "kind": "method", "line": 20, "owner": "Cart", "links": 1}
                ]}
            ]},
            "edges": [
                {"from": "src/Order/Order.php#L3:Order", "to": "src/Cart/Cart.php#L4:Cart", "kind": "imports_from", "guessed": false},
                {"from": "src/Order/Order.php#L3:Order", "to": "src/Cart/Cart.php#L20:.add<T>()", "kind": "calls", "guessed": true},
                {"from": "src/Order/Order.php#L9:.total()", "to": "src/Cart/Cart.php#L4:Cart", "kind": "calls", "guessed": false},
                {"from": "src/Order/Notifier.php#L5:notify()", "to": "src/Cart/Cart.php#L4:Cart", "kind": "references", "guessed": false}
            ],
            "kinds": [{"kind": "calls", "links": 2}, {"kind": "imports_from", "links": 1}, {"kind": "references", "links": 1}]
        })
    }
    #[test]
    fn a_link_of_the_map_is_selected_and_its_window_shows_only_what_makes_it_up() {
        // Every link of a map lies under a wider one that is selected, and
        // says which two places it joins.
        let html = canvas(&super::tests::view());
        assert_eq!(html.matches("<path class=\"graph-map-hit\"").count(), 2);
        assert!(html.contains("data-action=\"graph-map-link\" data-payload=\"{&quot;from&quot;:&quot;src/Order&quot;,&quot;to&quot;:&quot;src/Cart&quot;}\"><title>Order uses Cart · select to see what makes up this link</title>"));
        // They lie over the links that are seen.
        assert!(
            html.rfind("class=\"graph-map-edge").unwrap()
                < html.find("class=\"graph-map-hit\"").unwrap()
        );
        for other in [change(), file(), trail(), spread_view()] {
            let html = canvas(&other);
            assert_eq!(
                html.matches("<path class=\"graph-map-hit\"").count(),
                html.matches("<path class=\"graph-map-edge").count()
            );
        }
        // So does each row of a selected place: the link is named in the
        // direction it runs.
        let side = side(&super::tests::view());
        assert!(side.contains("class=\"graph-map-row-look\" data-action=\"graph-map-link\" data-payload=\"{&quot;from&quot;:&quot;src/Order&quot;,&quot;to&quot;:&quot;src/Cart&quot;}\" title=\"See what makes up this link\" aria-label=\"See what makes up the link with Cart\">↗</button></li>"));
        assert!(side.contains("data-payload=\"{&quot;from&quot;:&quot;tests&quot;,&quot;to&quot;:&quot;src/Order&quot;}\""));
        // A row of two symbols is one link, and has no window to open.
        let mut opened = file();
        opened["focus"] = json!({
            "symbol": true, "name": "Cart.total()", "file": "src/Cart/Cart.php", "line": 14, "kind": "method",
            "uses": [{"path": "src/Cart/Cart.php#L20:.add()", "name": "Cart.add()", "symbol": true, "relation": "calls", "guessed": 0}],
            "uses_more": 0, "used_by": [], "used_by_more": 0, "kinds": []
        });
        assert!(!super::side(&opened).contains("graph-map-row-look"));
        // The window: its name, and the link in a line.
        let view = link_view();
        assert_eq!(link_title(&view), "Order uses Cart");
        assert_eq!(
            link_summary(&view),
            "4 links: 3 read from the code, 1 guessed by name · 3 names in 2 files use 2 names"
        );
        assert_eq!(link_note(&view), "");
        // The picture: a frame for each file, a box for each name, a curve
        // for each link, and no link to select further.
        let html = link_canvas(&view, "");
        assert_eq!(html.matches("<rect class=\"graph-map-frame\"").count(), 3);
        assert_eq!(html.matches("data-action=\"graph-link-node\"").count(), 5);
        assert_eq!(html.matches("<path class=\"graph-map-edge").count(), 4);
        assert_eq!(
            html.matches("class=\"graph-map-edge graph-map-edge-guess\"")
                .count(),
            1
        );
        assert!(!html.contains("graph-map-hit") && !html.contains("graph-map-link"));
        assert!(html.contains("title=\"src/Order/Notifier.php\">Notifier.php</span>"));
        assert!(html.contains("<title>Order uses Cart · imports from</title>"));
        assert!(
            html.contains("<title>Order uses Cart.add&lt;T&gt;() · calls, guessed by name</title>")
        );
        // A name says what it is, where it lies and how much it does here.
        assert!(html.contains("data-payload=\"&quot;src/Order/Order.php#L3:Order&quot;\" data-folder=\"false\" data-kind=\"type\" data-selected=\"false\" data-dim=\"false\" data-flow=\"uses\" data-out=\"true\" data-in=\"false\""));
        assert!(html.contains("title=\"src/Order/Order.php:3\" aria-label=\"Order, type, src/Order/Order.php:3, uses through 2 links here\"><span class=\"graph-map-node-kind\" aria-hidden=\"true\">T</span><span class=\"graph-map-node-name\">Order</span><span class=\"graph-map-node-count\" aria-hidden=\"true\">2</span></button>"));
        assert!(html.contains("data-flow=\"used\" data-out=\"false\" data-in=\"true\""));
        assert!(html.contains("aria-label=\"Cart.add&lt;T&gt;(), method of Cart, src/Cart/Cart.php:20, is used through 1 link here\""));
        // A chosen name lights its links and dims what it is not linked
        // with.
        let lit = link_canvas(&view, "src/Order/Order.php#L9:.total()");
        assert_eq!(lit.matches("graph-map-edge graph-map-edge-on").count(), 1);
        assert_eq!(lit.matches("graph-map-edge-off").count(), 3);
        assert_eq!(lit.matches("data-selected=\"true\"").count(), 1);
        assert_eq!(lit.matches("data-dim=\"true\"").count(), 3);
        // Beside the picture: how to choose, and the kinds of the links.
        let idle = link_side(&view, "");
        assert!(idle.contains("Select a name to light its links."));
        assert!(idle.contains("<p class=\"knowledge-note\">Links by kind: calls 2 · imports from 1 · references 1</p>"));
        // Or the chosen name, the way to its file, and what it is linked
        // with here.
        let told = link_side(&view, "src/Cart/Cart.php#L4:Cart");
        assert!(told.contains("<h3>Cart</h3><p class=\"graph-map-path\">src/Cart/Cart.php:4</p><p class=\"knowledge-note\">Type</p><button type=\"button\" class=\"button compact\" data-action=\"graph-link-open\" data-payload=\"&quot;src/Cart/Cart.php#L4:Cart&quot;\">Open in the map</button>"));
        assert!(told.contains("<h4>Used by here <span class=\"graph-map-total\">3</span></h4>"));
        assert!(!told.contains("Uses here"));
        assert!(told.contains("data-action=\"graph-link-node\" data-payload=\"&quot;src/Order/Notifier.php#L5:notify()&quot;\"><span class=\"graph-map-row-name\">notify()</span><span class=\"graph-map-row-links\">references</span>"));
        let user = link_side(&view, "src/Order/Order.php#L3:Order");
        assert!(user.contains("<h4>Uses here <span class=\"graph-map-total\">2</span></h4>"));
        assert!(user.contains("<span class=\"graph-map-row-links\">calls, guessed</span>"));
    }
    #[test]
    fn the_window_of_a_link_says_what_it_does_not_draw_and_what_is_not_there() {
        // More names than a window holds.
        let mut wide = link_view();
        wide["links"] = json!(1_240);
        wide["guessed"] = json!(40);
        wide["users"]["names"] = json!(310);
        wide["users"]["files"] = json!(41);
        wide["used"]["names"] = json!(95);
        wide["used"]["files"] = json!(12);
        assert_eq!(
            link_summary(&wide),
            "1,240 links: 1,200 read from the code, 40 guessed by name · 310 names in 41 files use 95 names in 12 files"
        );
        assert_eq!(
            link_note(&wide),
            "Showing the most linked names: 3 of 310 that use, 2 of 95 that are used, and 4 of the 1,240 links between them. Look into a narrower link to see the rest: open the folder, or open it here."
        );
        // All names are drawn, and more links than are.
        let mut dense = link_view();
        dense["links"] = json!(9);
        assert_eq!(link_note(&dense), "Drawing 4 of the 9 links.");
        // One name uses one.
        let mut single = link_view();
        single["links"] = json!(1);
        single["guessed"] = json!(0);
        single["users"]["names"] = json!(1);
        single["users"]["files"] = json!(1);
        single["used"]["names"] = json!(1);
        single["used"]["files"] = json!(1);
        assert_eq!(
            link_summary(&single),
            "1 link: 1 read from the code, 0 guessed by name · 1 name uses 1 name"
        );
        // A link the other way, a link that is gone, a first picture.
        let mut none = link_view();
        none["links"] = json!(0);
        assert_eq!(link_summary(&none), "No link runs this way.");
        none["certain"] = json!(true);
        assert_eq!(
            link_summary(&none),
            "No link read from the code runs this way."
        );
        let gone = json!({"mode": "link", "missing": true, "from": {"path": "a", "name": "a"}, "to": {"path": "b", "name": "b"}});
        assert_eq!(
            link_summary(&gone),
            "The graph does not know one end of this link any more."
        );
        assert_eq!(
            (link_title(&gone), link_note(&gone)),
            ("a uses b".to_owned(), String::new())
        );
        assert!(link_canvas(&gone, "").contains("graph-map-plane"));
        assert!(link_side(&gone, "").contains("Select a name"));
        let mut early = link_view();
        early["preliminary"] = json!(true);
        assert_eq!(
            link_note(&early),
            "The links are those the files have in the checkout, not in that commit."
        );
    }
    #[test]
    fn a_picture_is_fitted_to_the_room_it_has() {
        // Too wide by half: it is drawn at half its size, and takes the
        // room of that size.
        assert_eq!(scale((2000.0, 400.0), (1000.0, 600.0)), 0.5);
        assert_eq!(
            scaled((2000.0, 400.0), 0.5),
            "width:2000px;height:400px;transform:scale(0.5000);transform-origin:0 0;margin:0 -1000px -200px 0"
        );
        // The tighter of the two sides decides.
        assert_eq!(scale((1000.0, 1200.0), (1000.0, 300.0)), 0.25);
        assert_eq!(scale((1500.0, 900.0), (1000.0, 300.0)), 1.0 / 3.0);
        // A picture that has room is drawn as it is, never larger.
        assert_eq!(scale((700.0, 300.0), (1000.0, 600.0)), 1.0);
        assert_eq!(scaled((700.0, 300.0), 1.0), "width:700px;height:300px");
        // One far too large is made no smaller than can be read at all,
        // and the rest of it is scrolled to.
        assert_eq!(scale((9000.0, 400.0), (900.0, 600.0)), SMALLEST);
        // Nothing is asked of a picture or a room that has no size yet.
        assert_eq!(scale((0.0, 400.0), (900.0, 600.0)), 1.0);
        assert_eq!(scale((900.0, 400.0), (-2.0, 600.0)), 1.0);
        // A map begins as it is drawn, until the viewer chose otherwise.
        assert_eq!(fresh("work")["fit"], false);
    }
    #[test]
    fn what_the_pointer_rests_on_is_lit_with_what_it_is_linked_to() {
        // Three boxes in a row: the first uses the second, the second the
        // third.
        let links = [(0, 1), (1, 2)];
        // A box lights its links and the boxes at their other ends.
        let middle = Pointed::Place(1);
        assert!(links.iter().all(|link| lit_link(middle, *link)));
        assert!((0..3).all(|place| lit_place(middle, &links, place)));
        let first = Pointed::Place(0);
        assert_eq!(links.map(|link| lit_link(first, link)), [true, false]);
        assert_eq!(
            [0, 1, 2].map(|place| lit_place(first, &links, place)),
            [true, true, false]
        );
        // A link lights itself and its two ends, not the link back.
        let link = Pointed::Link(1, 2);
        assert_eq!(links.map(|other| lit_link(link, other)), [false, true]);
        assert!(!lit_link(link, (2, 1)));
        assert_eq!(
            [0, 1, 2].map(|place| lit_place(link, &links, place)),
            [false, true, true]
        );
        // What a part of a picture stands for is read from its marks: a
        // box or a handle of it by its number, a link by its two ends.
        assert_eq!(pointed(Some("7"), None), Some(Pointed::Place(7)));
        assert_eq!(pointed(None, Some("3 12")), Some(Pointed::Link(3, 12)));
        assert_eq!(pointed(Some("7"), Some("3 12")), Some(Pointed::Place(7)));
        for wrong in [
            (None, None),
            (Some("x"), None),
            (None, Some("3")),
            (None, Some("3 x")),
        ] {
            assert_eq!(pointed(wrong.0, wrong.1), None, "{wrong:?}");
        }
        // The pictures carry those marks: each box its number, each link
        // and the wider link over it the numbers of its ends, each handle
        // the number of its box.
        let html = canvas(&super::tests::view());
        assert!(html.contains("class=\"graph-map-node\" data-n=\"0\" data-action=\"graph-map-node\" data-payload=\"&quot;src/Cart&quot;\""));
        assert!(
            html.contains("class=\"graph-map-node\" data-n=\"2\" data-action=\"graph-map-node\"")
        );
        assert_eq!(html.matches(" data-ends=\"1 0\" ").count(), 2);
        assert_eq!(html.matches(" data-ends=\"1 2\" ").count(), 2);
        assert!(html.contains("<path class=\"graph-map-hit\" data-ends=\"1 0\" d=\"M"));
        let spread = canvas(&spread_view());
        assert!(spread.contains("class=\"graph-map-spread\" data-of=\"1\" data-dim=\"false\" data-action=\"graph-map-spread\" data-payload=\"&quot;src/Cart/Rules&quot;\""));
        // The corner of a frame is of no box, and is never made faint.
        assert!(spread.contains("class=\"graph-map-spread\" data-action=\"graph-map-gather\""));
        let followed = canvas(&trail());
        assert_eq!(
            followed
                .matches("class=\"graph-map-more\" data-of=\"")
                .count(),
            followed.matches("class=\"graph-map-more\"").count()
        );
        assert!(followed.matches("class=\"graph-map-more\"").count() > 0);
        // So does the window of a link: the names that use come first.
        let link = link_canvas(&link_view(), "");
        assert!(link.contains("class=\"graph-map-node\" data-n=\"0\" data-action=\"graph-link-node\" data-payload=\"&quot;src/Order/Order.php#L3:Order&quot;\""));
        assert!(link.contains("class=\"graph-map-node\" data-n=\"4\" data-action=\"graph-link-node\" data-payload=\"&quot;src/Cart/Cart.php#L20:.add&lt;T&gt;()&quot;\""));
        assert_eq!(
            ["0 3", "0 4", "1 3", "2 3"]
                .map(|ends| link.matches(&format!(" data-ends=\"{ends}\" ")).count()),
            [1; 4]
        );
    }
}
