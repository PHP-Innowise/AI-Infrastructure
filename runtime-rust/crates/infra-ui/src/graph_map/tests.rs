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
fn two_places_stand_side_by_side_before_or_after_the_rest() {
    let layers = |plan: &Plan| {
        (0..plan.nodes.len())
            .map(|node| layer(plan, node))
            .collect::<Vec<_>>()
    };
    let none = Frames::default();
    // 0 reaches 3 through 2, and 1 uses 3 itself. Left to its links, 1
    // stands a layer in, next to what it uses.
    let below = [(0, 2, 1), (2, 3, 1), (1, 3, 1)];
    assert_eq!(layers(&plan(4, &below)), [0, 1, 1, 2]);
    // Side by side the two stand in the first layer, and the longer
    // link of 1 passes the layer between in a slot of its own.
    let beside = plan_beside(4, &below, &none, Some(([0, 1], false)));
    assert_eq!(layers(&beside), [0, 0, 1, 2]);
    assert_eq!(beside.nodes[0].x, beside.nodes[1].x);
    assert!(beside.wires.iter().all(|wire| !wire.back));
    let long = beside.wires.iter().find(|wire| wire.edge == 2).unwrap();
    assert!(long.path.contains(" L"), "{}", long.path);
    // 3 reaches 0 through 2 and uses 1 itself: the two stand in the
    // last layer, where their links lead to them.
    let above = [(3, 2, 1), (2, 0, 1), (3, 1, 1)];
    assert_eq!(layers(&plan(4, &above)), [2, 1, 1, 0]);
    let beside = plan_beside(4, &above, &none, Some(([0, 1], true)));
    assert_eq!(layers(&beside), [2, 2, 1, 0]);
    assert_eq!(beside.nodes[0].x, beside.nodes[1].x);
    assert!(beside.nodes[0].x > beside.nodes[2].x);
    // One of the two that a link leads to is left where its links put
    // it, and so is one that no link touches or that is not there.
    let held = [(0, 2, 1), (2, 1, 1), (1, 3, 1)];
    assert_eq!(layers(&plan(4, &held)), [0, 2, 1, 3]);
    assert_eq!(
        layers(&plan_beside(4, &held, &none, Some(([0, 1], false)))),
        [0, 2, 1, 3]
    );
    let apart = plan_beside(3, &[(0, 2, 1)], &none, Some(([1, 9], false)));
    assert_eq!(
        (apart.nodes[1].layer, layer(&apart, 0), layer(&apart, 2)),
        (None, 0, 1)
    );
    // Nothing asked for is the picture as it was.
    assert_eq!(plan_beside(4, &below, &none, None), plan(4, &below));
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
/// A picture led as `routing` says, without frames.
fn laid_out(count: usize, edges: &[(usize, usize, u64)], routing: Routing) -> Plan {
    plan_routed(count, edges, &Frames::default(), None, routing)
}
/// Where a drawn link starts and where it ends.
fn ends_at(wire: &Wire) -> ((f64, f64), (f64, f64)) {
    let numbers = wire
        .path
        .split(|c: char| c == ' ' || c.is_ascii_alphabetic())
        .filter_map(|part| part.parse::<f64>().ok())
        .collect::<Vec<_>>();
    (
        (numbers[0], numbers[1]),
        (numbers[numbers.len() - 2], numbers[numbers.len() - 1]),
    )
}
fn wire_of(plan: &Plan, edge: usize) -> &Wire {
    plan.wires.iter().find(|wire| wire.edge == edge).unwrap()
}
#[test]
fn the_links_of_a_box_leave_it_apart_in_the_order_of_where_they_lead() {
    // 0 uses 1, 2 and 3, which stand one under another.
    let edges = [(0, 1, 1), (0, 2, 1), (0, 3, 1)];
    let plain = laid_out(4, &edges, Routing::PLAIN);
    let middle = plain.nodes[0].y + BOX_HEIGHT / 2.0;
    assert!(plain.wires.iter().all(|wire| ends_at(wire).0 .1 == middle));
    let plan = laid_out(4, &edges, Routing::CLEAN);
    // The boxes stand where they stood.
    assert_eq!(plan.nodes, plain.nodes);
    let mut used = [1, 2, 3];
    used.sort_by(|a, b| plan.nodes[*a].y.total_cmp(&plan.nodes[*b].y));
    let leaves = used.map(|node| {
        let edge = edges.iter().position(|(_, to, _)| *to == node).unwrap();
        ends_at(wire_of(&plan, edge)).0 .1
    });
    // The link to the box above leaves above, four apart, about the
    // middle of the side.
    assert_eq!(
        leaves,
        [middle - PORT_GAP, middle, middle + PORT_GAP],
        "{plan:?}"
    );
    // Each of them reaches the middle of its own box: one link there.
    for (edge, (_, to, _)) in edges.iter().enumerate() {
        assert_eq!(
            ends_at(wire_of(&plan, edge)).1 .1,
            plan.nodes[*to].y + BOX_HEIGHT / 2.0
        );
    }
    // Many links share the side: they never take more of it than
    // `PORT_SPREAD`, and none leaves outside the box.
    let fan = (1..12).map(|to| (0, to, 1)).collect::<Vec<_>>();
    let plan = laid_out(12, &fan, Routing::CLEAN);
    let top = plan.nodes[0].y;
    let ys = plan
        .wires
        .iter()
        .map(|wire| ends_at(wire).0 .1)
        .collect::<Vec<_>>();
    let (least, most) = (
        ys.iter().copied().fold(f64::INFINITY, f64::min),
        ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    );
    assert_eq!(most - least, PORT_SPREAD);
    assert!(
        least > top && most < top + BOX_HEIGHT,
        "{least} {most} {top}"
    );
    // The links that arrive at one box stand apart the same way.
    let into = [(0, 2, 1), (1, 2, 1)];
    let plan = laid_out(3, &into, Routing::CLEAN);
    let arrive = [0, 1].map(|edge| ends_at(wire_of(&plan, edge)).1 .1);
    assert_eq!((arrive[0] - arrive[1]).abs(), PORT_GAP);
    assert_eq!(
        arrive[0] < arrive[1],
        plan.nodes[0].y < plan.nodes[1].y,
        "{plan:?}"
    );
}
#[test]
fn links_that_pass_layers_together_share_one_slot_in_each() {
    // A chain of four layers, and from its first box three links that
    // pass two layers to three boxes of the last.
    let edges = [
        (0, 1, 1),
        (1, 2, 1),
        (2, 3, 1),
        (2, 4, 1),
        (2, 5, 1),
        (0, 3, 1),
        (0, 4, 1),
        (0, 5, 1),
    ];
    let plain = laid_out(6, &edges, Routing::PLAIN);
    let bundled = laid_out(
        6,
        &edges,
        Routing {
            portals: None,
            ..Routing::CLEAN
        },
    );
    // Each of the three had a slot of its own in the two layers it
    // passes; together they take one in each.
    let slots = |plan: &Plan| {
        plan.wires
            .iter()
            .filter(|wire| wire.edge >= 5)
            .flat_map(|wire| {
                wire.path
                    .split(" L")
                    .skip(1)
                    .map(|run| run.split(' ').take(2).collect::<Vec<_>>().join(" "))
                    .collect::<Vec<_>>()
            })
            .collect::<BTreeSet<_>>()
            .len()
    };
    assert_eq!((slots(&plain), slots(&bundled)), (6, 2));
    // They run as one through the layers they pass and part at the end.
    let run = |edge: usize| {
        let path = &wire_of(&bundled, edge).path;
        path[path.find(" L").unwrap()..path.rfind(" C").unwrap()].to_owned()
    };
    assert_eq!(run(5), run(6));
    assert_eq!(run(6), run(7));
    let ends = [5, 6, 7].map(|edge| ends_at(wire_of(&bundled, edge)).1);
    assert!(ends[0] != ends[1] && ends[1] != ends[2]);
    // Links that lead to one box from boxes of several layers join the
    // same way: 3 is used by 0 past two layers and by 1 past one.
    let into = [(0, 1, 1), (1, 2, 1), (2, 3, 1), (0, 3, 1), (1, 3, 1)];
    let (plain, bundled) = (
        laid_out(4, &into, Routing::PLAIN),
        laid_out(
            4,
            &into,
            Routing {
                portals: None,
                ..Routing::CLEAN
            },
        ),
    );
    let bends = |plan: &Plan| {
        plan.wires
            .iter()
            .map(|w| w.path.matches(" L").count())
            .sum::<usize>()
    };
    assert_eq!((bends(&plain), bends(&bundled)), (3, 3));
    let last_run = |plan: &Plan, edge: usize| {
        let path = &wire_of(plan, edge).path;
        path[path.rfind(" L").unwrap()..path.rfind(" C").unwrap()].to_owned()
    };
    assert_ne!(last_run(&plain, 3), last_run(&plain, 4));
    assert_eq!(last_run(&bundled, 3), last_run(&bundled, 4));
    // A single link that passes a layer keeps a slot of its own: with
    // nothing to share, the links are led as they were.
    let alone = [(0, 1, 1), (1, 2, 1), (0, 2, 1)];
    let shared = Routing {
        trunks: true,
        ..Routing::PLAIN
    };
    assert_eq!(
        laid_out(3, &alone, shared).wires,
        laid_out(3, &alone, Routing::PLAIN).wires
    );
}
#[test]
fn a_layer_that_many_links_pass_is_lower_once_they_share_a_slot() {
    // Four boxes use one far box past a layer of ten boxes: four slots
    // among those ten where each link had its own, one where they
    // share, and the picture is lower. A slot is thin, so it is lower
    // by little: what a shared slot saves is the room of the links,
    // which adds up where many pass.
    let (sources, middle, target) = (0..4, 4..14, 14);
    let mut edges = vec![];
    for box_ in middle.clone() {
        edges.push((0, box_, 1));
        edges.push((box_, target, 1));
    }
    for source in sources.clone().skip(1) {
        edges.push((source, 3 + source, 1));
    }
    for source in sources {
        edges.push((source, target, 1));
    }
    let plain = laid_out(15, &edges, Routing::PLAIN);
    let bundled = laid_out(
        15,
        &edges,
        Routing {
            portals: None,
            ..Routing::CLEAN
        },
    );
    assert!(plain.nodes[..4].iter().all(|node| node.layer == Some(0)));
    assert_eq!(plain.nodes[14].layer, Some(2));
    let slots = |plan: &Plan| {
        plan.wires
            .iter()
            .flat_map(|wire| {
                wire.path
                    .split(" L")
                    .skip(1)
                    .map(|run| run.split(' ').take(2).collect::<Vec<_>>().join(" "))
                    .collect::<Vec<_>>()
            })
            .collect::<BTreeSet<_>>()
            .len()
    };
    assert_eq!((slots(&plain), slots(&bundled)), (4, 1));
    let saved = plain.height - bundled.height;
    assert!(
        (BEND_HEIGHT + BEND_GAP..=3.0 * (BEND_HEIGHT + BEND_GAP)).contains(&saved),
        "{} {}",
        plain.height,
        bundled.height
    );
}
#[test]
fn a_link_that_passes_many_layers_is_a_portal_and_takes_no_slot() {
    // A chain of five layers and a link from its first box to its last.
    let edges = [(0, 1, 1), (1, 2, 1), (2, 3, 1), (3, 4, 1), (0, 4, 1)];
    let plain = laid_out(5, &edges, Routing::PLAIN);
    let plan = laid_out(5, &edges, Routing::CLEAN);
    assert_eq!(PORTAL_SPAN, 4);
    let far = wire_of(&plan, 4);
    assert!(far.portal);
    assert!(plan.wires.iter().filter(|wire| wire.portal).count() == 1);
    // One curve from the right side of the first box to the left side
    // of the last, where it passed three slots before.
    assert_eq!(wire_of(&plain, 4).path.matches(" L").count(), 3);
    assert_eq!(far.path.matches(" L").count(), 0);
    assert_eq!(
        ends_at(far),
        (
            (
                plan.nodes[0].x + BOX_WIDTH,
                plan.nodes[0].y + BOX_HEIGHT / 2.0
            ),
            (plan.nodes[4].x, plan.nodes[4].y + BOX_HEIGHT / 2.0)
        )
    );
    // The picture is as tall as the chain alone.
    let chain = laid_out(5, &edges[..4], Routing::CLEAN);
    assert_eq!(plan.height, chain.height);
    assert!(plan.height < plain.height);
    // One layer fewer and it is led through the layers as any other.
    let near = [(0, 1, 1), (1, 2, 1), (2, 3, 1), (0, 3, 1)];
    assert!(laid_out(4, &near, Routing::CLEAN)
        .wires
        .iter()
        .all(|wire| !wire.portal));
    // A link that closes a cycle is drawn however far it runs.
    let ring = [(0, 1, 9), (1, 2, 9), (2, 3, 9), (3, 4, 9), (4, 0, 1)];
    let plan = laid_out(5, &ring, Routing::CLEAN);
    let back = wire_of(&plan, 4);
    assert!(back.back && !back.portal, "{plan:?}");
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
fn modules_render_as_graph_nodes_with_links_and_file_actions() {
    let view = json!({
        "mode":"modules", "children":2, "matching":2,
        "nodes":[
            {"path":"module:0","name":"Cart <shop>","module":true,"within":1,"out":1},
            {"path":"module:1","name":"Payment","module":true,"in":1}
        ],
        "edges":[{"from":"module:0","to":"module:1","links":1,"guessed":0}],
        "focus":{
            "name":"Cart <shop>","files":1,"symbols":2,"within":1,
            "uses_total":1,"uses":[{"path":"module:1","name":"Payment","links":1}],
            "members":["src/cart.php"],"internal_total":0
        }
    });
    let picture = canvas(&view);
    assert!(picture.contains("Cart &lt;shop&gt;, module, 1 internal link"));
    assert!(picture.contains("Cart &lt;shop&gt; uses Payment"));
    assert!(!picture.contains("graph-map-hit"));
    let side = modules_side(&view);
    assert!(side.contains("Uses modules · 1"));
    assert!(side.contains("graph-map-module-select"));
    assert!(side.contains("graph-map-module-file"));
    assert!(side.contains("src/cart.php"));
}

#[test]
fn test_paths_and_task_context_use_the_selected_changed_file() {
    let view = json!({
        "mode":"changes", "target":"run:one", "certain":true,
        "focus":{
            "path":"src/changed.php",
            "uses":[{"path":"src/dependency.php"}],
            "used_by":[{"path":"src/caller.php"}]
        },
        "test_reach":[
            {"path":"src/changed.php","state":"ready","total":1,"tests":[
                {"path":"tests/Cart<Test>.php","to":"src/changed.php",
                 "file":"tests/Cart<Test>.php","name":"tests/Cart<Test>.php","steps":2}
            ]},
            {"path":"src/alone.php","state":"ready","total":0,"tests":[]},
            {"path":"src/partial.php","state":"partial","total":0,"tests":[]},
            {"path":"src/deleted.php","state":"deleted"}
        ]
    });
    let html = test_paths(&view);
    assert!(html.contains(r#"data-action="graph-map-test-path""#));
    assert!(html.contains("tests/Cart&lt;Test&gt;.php · 2 links"));
    assert!(html.contains("No test path found in the graph"));
    assert!(html.contains("search limit reached"));
    assert!(html.contains("Deleted file"));
    let attachment = map_attachment(&view).unwrap();
    for text in [
        "src/changed.php",
        "run:one",
        "src/dependency.php",
        "src/caller.php",
        "tests/Cart<Test>.php",
        "links read from code",
    ] {
        assert!(attachment.contains(text), "{attachment}");
    }
    assert!(!attachment.contains("src/alone.php"), "{attachment}");
    assert!(map_attachment(&json!({"mode":"changes"})).is_none());
}

/// The change of the fixture as what a run did: the run opened the cart
/// it changed and the order, and of four files that use its change left
/// three unopened.
fn run_view() -> V {
    let mut view = change();
    view["chosen"] = json!({
        "target": "run:0b0e7f3a-5c1d-4e2f-9a6b-7c8d9e0f1a2b", "kind": "run",
        "name": "Round <totals> once", "title": null, "pull": null, "head": null,
        "base": null, "notes": [], "files": 3
    });
    view["run"] = json!({
        "session": "0b0e7f3a-5c1d-4e2f-9a6b-7c8d9e0f1a2b", "title": "Round <totals> once",
        "provider": "claude", "status": "completed", "steps": 9, "opened": 12, "edited": 3,
        "outside": 0, "searched": 0, "commands": 0, "helpers": 0, "limited": false,
        "elsewhere": false,
        "reach": {
            "users": 4, "opened": 1, "unopened": 3, "more": 0,
            "rows": [
                {"path": "src/Order/Invoice<b>.php", "name": "Invoice<b>.php", "links": 3,
                 "guessed": 1, "uses": 2, "to": "src/Cart/Cart<c>.php", "test": false},
                {"path": "src/Report/Report.php", "name": "Report.php", "links": 1,
                 "guessed": 1, "uses": 1, "to": "src/Cart/Cart.php", "test": false},
                {"path": "tests/Cart/CartTest.php", "name": "CartTest.php", "links": 1,
                 "guessed": 0, "uses": 1, "to": "src/Cart/Cart.php", "test": true}
            ]
        }
    });
    view["nodes"][0]["opened"] = json!(true);
    view["nodes"][2]["opened"] = json!(true);
    view
}
#[test]
fn a_run_is_laid_over_the_map_of_what_it_changed() {
    let (plain, view) = (change(), run_view());
    assert_eq!(chosen(&view), "What the run changed: Round <totals> once");
    let mut unnamed = view.clone();
    unnamed["chosen"]["name"] = json!("");
    assert_eq!(chosen(&unnamed), "What a run changed");
    // Above the map: how far the run looked at what uses its change.
    assert_eq!(
        summary(&view),
        format!(
            "{} · 3 of those that use them not opened by the run",
            summary(&plain)
        )
    );
    // The note says first what the run did; the rest is the change's.
    assert_eq!(note(&plain), "");
    assert_eq!(note(&view), "The run opened 12 files and changed 3 files.");
    let mut wide = view.clone();
    wide["change"]["single"] = json!(false);
    let mut wide_plain = plain.clone();
    wide_plain["change"]["single"] = json!(false);
    assert_eq!(
        note(&wide),
        format!(
            "The run opened 12 files and changed 3 files. {}",
            note(&wide_plain)
        )
    );
    assert!(note(&wide_plain).starts_with("The change is too wide"));
    // In the picture a box the run opened carries its mark and says so;
    // any other box does not.
    let html = canvas(&view);
    assert_eq!(html.matches(" data-run-opened=\"true\"").count(), 2);
    assert_eq!(
        html.matches("<span class=\"graph-map-node-opened\" aria-hidden=\"true\">◉</span>")
            .count(),
        2
    );
    for (name, opened) in [
        ("Cart.php", true),
        ("Price.php", false),
        ("Order.php", true),
    ] {
        let button = html
            .split("<button ")
            .find(|button| button.contains(&format!("aria-label=\"{name}, file")))
            .unwrap();
        assert_eq!(
            button.contains("data-run-opened=\"true\""),
            opened,
            "{name}"
        );
        assert_eq!(
            button.contains(", opened by the run"),
            opened,
            "{name}: {button}"
        );
    }
    assert!(
        html.contains("is used through 2 links, opened by the run, modified\">"),
        "{html}"
    );
    assert!(!canvas(&plain).contains("run-opened") && !canvas(&plain).contains('◉'));
    // A folder says how many of its files the run opened.
    let mut folders = view.clone();
    folders["nodes"][1]["folder"] = json!(true);
    folders["nodes"][1]["opened"] = json!(1200);
    let html = canvas(&folders);
    assert!(
        html.contains("<span class=\"graph-map-node-opened\" aria-hidden=\"true\">◉1,200</span>")
    );
    assert!(
        html.contains(", the run opened 1,200 files in it"),
        "{html}"
    );
    folders["nodes"][1]["opened"] = json!(1);
    assert!(canvas(&folders).contains(", the run opened 1 file in it"));
    // The key says what the mark is, where a run is shown.
    assert!(legend(&view, true)
        .ends_with("The rest use the change or are used by it. ◉: opened by the run; on a folder, how many of its files."));
    assert!(legend(&plain, true).ends_with("The rest use the change or are used by it."));
    // Beside the map, with nothing selected: the files that use the
    // change and were never opened. Each opens, and the mark beside it
    // looks into its link with the changed file it uses most.
    let html = side(&view);
    assert!(html.starts_with(&side(&plain)), "{html}");
    let list = &html[side(&plain).len()..];
    assert!(list.starts_with("<h4>Use the change, not opened by the run <span class=\"graph-map-total\">3</span></h4><ul class=\"graph-map-rows\"><li><button type=\"button\" class=\"graph-map-row\" data-unopened=\"true\" data-action=\"graph-map-open\" data-payload=\"&quot;src/Order/Invoice&lt;b&gt;.php&quot;\" title=\"src/Order/Invoice&lt;b&gt;.php · select to open it\"><span class=\"graph-map-row-name\">Invoice&lt;b&gt;.php</span><span class=\"graph-map-row-links\">src/Order · 3 links, 1 guessed · over 2 changed files</span></button><button type=\"button\" class=\"graph-map-row-look\" data-action=\"graph-map-link\" data-payload=\"{&quot;from&quot;:&quot;src/Order/Invoice&lt;b&gt;.php&quot;,&quot;to&quot;:&quot;src/Cart/Cart&lt;c&gt;.php&quot;}\" title=\"See what makes up this link\" aria-label=\"See what makes up the link with Cart&lt;c&gt;.php\">↗</button></li>"), "{list}");
    assert!(list.contains("<span class=\"graph-map-row-name\">Report.php</span><span class=\"graph-map-row-links\">src/Report · 1 link guessed</span>"), "{list}");
    assert!(list.ends_with("<span class=\"graph-map-row-name\">CartTest.php</span><span class=\"graph-map-row-links\">tests/Cart · 1 link · test</span></button><button type=\"button\" class=\"graph-map-row-look\" data-action=\"graph-map-link\" data-payload=\"{&quot;from&quot;:&quot;tests/Cart/CartTest.php&quot;,&quot;to&quot;:&quot;src/Cart/Cart.php&quot;}\" title=\"See what makes up this link\" aria-label=\"See what makes up the link with Cart.php\">↗</button></li></ul>"), "{list}");
    assert_eq!(list.matches("data-action=\"graph-map-open\"").count(), 3);
    // A file at the top of the project has no folder to name.
    let mut top = view.clone();
    top["run"]["reach"]["rows"][2]["path"] = json!("bootstrap<x>.php");
    assert!(side(&top).contains("title=\"bootstrap&lt;x&gt;.php · select to open it\"><span class=\"graph-map-row-name\">bootstrap&lt;x&gt;.php</span><span class=\"graph-map-row-links\">1 link · test</span>"));
    // More than are listed are counted: the number is of all of them.
    let mut many = view.clone();
    many["run"]["reach"]["unopened"] = json!(1203);
    many["run"]["reach"]["more"] = json!(1200);
    let html = side(&many);
    assert!(
        html.contains("not opened by the run <span class=\"graph-map-total\">1,203</span></h4>")
    );
    assert!(html.ends_with("</ul><p class=\"knowledge-note\">and 1,200 more</p>"));
    assert!(summary(&many).ends_with("· 1,203 of those that use them not opened by the run"));
    // A run that opened all that uses its change says so; where nothing
    // uses it, nothing is said.
    let mut all = view.clone();
    all["run"]["reach"] = json!({"users": 4, "opened": 4, "unopened": 0, "more": 0, "rows": []});
    assert_eq!(
        summary(&all),
        format!("{} · the run opened all that use them", summary(&plain))
    );
    assert_eq!(
        side(&all),
        format!(
            "{}<h4>Use the change, not opened by the run <span class=\"graph-map-total\">0</span></h4><p class=\"knowledge-note\">The run opened every file that uses what it changed here.</p>",
            side(&plain)
        )
    );
    let mut none = view.clone();
    none["run"]["reach"] = json!({"users": 0, "opened": 0, "unopened": 0, "more": 0, "rows": []});
    assert_eq!(summary(&none), summary(&plain));
    assert_eq!(side(&none), side(&plain));
    // With a place selected the panel is of that place, and what uses
    // it or is used by it says whether the run opened it.
    let mut selected = view.clone();
    selected["focus"] = json!({
        "path": "src/Cart/Cart.php", "name": "Cart.php", "folder": false, "inside": true,
        "files": 1, "symbols": 3, "within": 0, "out": 2, "in": 2, "changed": 1, "status": "M",
        "role": "changed", "altered": false,
        "uses": [{"path": "src/Cart/Price.php", "name": "src/Cart/Price.php", "folder": false, "inside": true, "links": 2, "guessed": 1, "changed": false}],
        "uses_more": 0,
        "used_by": [
            {"path": "src/Order/Order.php", "name": "src/Order/Order.php", "folder": false, "inside": true, "links": 1, "guessed": 0, "changed": false, "opened": true},
            {"path": "src/Order/Invoice.php", "name": "src/Order/Invoice.php", "folder": false, "inside": false, "links": 1, "guessed": 0, "changed": false}
        ],
        "used_by_more": 0, "kinds": []
    });
    let html = side(&selected);
    assert!(!html.contains("not opened by the run"));
    assert!(html.contains("src/Order/Order.php</span><span class=\"graph-map-row-links\">1 link · opened by the run</span>"), "{html}");
    assert!(html
        .contains("src/Order/Invoice.php</span><span class=\"graph-map-row-links\">1 link</span>"));
    assert_eq!(html.matches(" · opened by the run").count(), 1);
    // An opened file of the run lists what uses it alike.
    let mut opened = file();
    opened["run"] = view["run"].clone();
    let html = side(&opened);
    assert!(html.contains("<h4>Use the change, not opened by the run <span class=\"graph-map-total\">3</span></h4>"), "{html}");
    assert!(!side(&file()).contains("the run"));
}
#[test]
fn a_picture_of_a_run_says_what_of_its_looking_is_not_recorded() {
    let (plain, view) = (change(), run_view());
    let said = |change: &[(&str, V)]| {
        let mut view = view.clone();
        for (key, value) in change {
            view["run"][*key] = value.clone();
        }
        note(&view)
            .strip_suffix(&note(&plain))
            .unwrap()
            .trim_end()
            .to_owned()
    };
    assert_eq!(said(&[]), "The run opened 12 files and changed 3 files.");
    assert_eq!(
        said(&[("opened", json!(1)), ("edited", json!(1))]),
        "The run opened 1 file and changed 1 file."
    );
    // A command, a search and a helper show files that no step names.
    assert_eq!(
        said(&[("commands", json!(2))]),
        "The run opened 12 files and changed 3 files. It also ran 2 commands: some file reads may be missing."
    );
    assert_eq!(
        said(&[("commands", json!(1)), ("searched", json!(1200))]),
        "The run opened 12 files and changed 3 files. It also ran 1 command and searched 1,200 times: some file reads may be missing."
    );
    assert_eq!(
        said(&[
            ("commands", json!(2)),
            ("searched", json!(1)),
            ("helpers", json!(3))
        ]),
        "The run opened 12 files and changed 3 files. It also ran 2 commands, searched 1 time and started 3 helpers: some file reads may be missing."
    );
    assert_eq!(
        said(&[("helpers", json!(1))]),
        "The run opened 12 files and changed 3 files. It also started 1 helper: some file reads may be missing."
    );
    // What else is not known of it.
    assert_eq!(
        said(&[("limited", json!(true))]),
        "The run opened 12 files and changed 3 files. Its record stops before its end: what it did later is not known."
    );
    assert_eq!(
        said(&[("outside", json!(1))]),
        "The run opened 12 files and changed 3 files. 1 file it touched lies outside the project."
    );
    assert_eq!(
        said(&[("outside", json!(1200)), ("elsewhere", json!(true))]),
        "The run opened 12 files and changed 3 files. 1,200 files it touched lie outside the project. It worked in a folder of its own; the map is of the project's graph."
    );
    // A change that is no run says nothing of one.
    assert!(!note(&plain).contains("run"));
    // The Show list names a run that is shown, and the map is opened
    // on what its button asks for.
    let place = json!({"git": true, "branch": "main", "base": "origin/main"});
    assert!(targets(&place, "run:0b0e7f3a")
        .ends_with("<option value=\"run:0b0e7f3a\" selected>A run of an agent</option>"));
    // With a session on the screen the list offers its run; where
    // that run is the one shown it is called by what it is here, and
    // another session's run stays a run of an agent.
    let offered = with_run(targets(&place, "work"), "work", "0b0e7f3a");
    assert_eq!(
        offered,
        format!(
            "{}<option value=\"run:0b0e7f3a\">The run of this session</option>",
            targets(&place, "work")
        )
    );
    assert_eq!(
        with_run(targets(&place, "work"), "work", ""),
        targets(&place, "work")
    );
    let shown = with_run(targets(&place, "run:0b0e7f3a"), "run:0b0e7f3a", "0b0e7f3a");
    assert!(
        shown.ends_with("<option value=\"run:0b0e7f3a\" selected>The run of this session</option>")
    );
    assert_eq!(shown.matches("run:0b0e7f3a").count(), 1);
    let other = with_run(targets(&place, "run:ffff0000"), "run:ffff0000", "0b0e7f3a");
    assert!(other.ends_with(
        "<option value=\"run:ffff0000\" selected>A run of an agent</option><option value=\"run:0b0e7f3a\">The run of this session</option>"
    ), "{other}");
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
    assert!(targets(&place, "pr:9").ends_with("<option value=\"pr:9\" selected>#9 Wip</option>"));
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
        "<li class=\"graph-pick-row has-git-graph\" id=\"graph-pick-row-0\" role=\"option\" aria-selected=\"false\" data-action=\"graph-pick-choose\" data-payload=\"&quot;commit:1a2b3c4d5e6f&quot;\"><span class=\"git-graph-rail\"><svg class=\"git-graph-svg\""
    ), "{html}");
    assert!(html.contains("<span class=\"graph-pick-main\"><span class=\"graph-pick-title\">Round &lt;totals&gt; once</span><span class=\"graph-pick-badges\"><span class=\"graph-pick-badge\" data-kind=\"head\">main</span><span class=\"graph-pick-badge\" data-kind=\"remote\">origin/main</span><span class=\"graph-pick-badge\" data-kind=\"tag\">v2</span></span></span><span class=\"graph-pick-who\">Ann</span><span class=\"graph-pick-when\">1 hour ago</span><code class=\"graph-pick-code\">1a2b3c4</code></li>"));
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
    assert_eq!(
        options(&none, 0),
        "<li class=\"graph-pick-more\" role=\"presentation\"><button type=\"button\" class=\"button compact\" data-action=\"graph-pick-more\">Show older commits</button></li>"
    );
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
    assert!(html.contains("<span class=\"graph-pick-badge\" data-kind=\"merged\">merged</span>"));
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
        html.contains("<span class=\"graph-map-node-status\" data-status=\"modified\" aria-hidden=\"true\">M</span>")
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
        html.contains("<span class=\"graph-map-node-status\" data-status=\"modified\" aria-hidden=\"true\">15</span>")
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
    assert!(html.contains("<h4>Uses in the change <span class=\"graph-map-total\">1</span></h4>"));
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
    // The selected symbol is drawn whatever the limit says, and is no
    // one of the most linked.
    long["beyond"] = json!(true);
    assert!(note(&long).starts_with(
        "Showing the 4 most linked of 67 symbols in this file; filter by name to reach the rest. The selected one is drawn as well. Showing the 2 most linked"
    ));
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
    assert_eq!(html.matches(" marker-end=").count(), 3);
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
        html.contains("<span class=\"graph-map-node-status\" data-status=\"created\" aria-hidden=\"true\">A</span>")
    );
    assert!(html.contains("title=\"Cart.total() · src/Cart/Cart.php:19\" aria-label=\"Cart.total(), method of Cart, line 19, uses others through 2 links, is used through 3 links\""), "{html}");
    assert!(html.contains(
        "aria-label=\"Cart, type, line 12, uses others through 0 links, is used through 0 links\""
    ));
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
    assert!(html.contains("line 19, uses others through 2 links, is used through 3 links, modified\"><span class=\"graph-map-node-kind\" aria-hidden=\"true\">m</span><span class=\"graph-map-node-name\">Cart.total()</span><span class=\"graph-map-node-status\" data-status=\"modified\" aria-hidden=\"true\">M</span>"), "{html}");
    // A class shows how many of its symbols changed, a new function
    // Git's letter for added.
    assert!(html.contains("is used through 0 links, 1 changed inside\"><span class=\"graph-map-node-kind\" aria-hidden=\"true\">T</span><span class=\"graph-map-node-name\">Cart</span><span class=\"graph-map-node-status\" data-status=\"modified\" aria-hidden=\"true\">1</span>"), "{html}");
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
        reach(json!({"symbols": 0, "added": 0, "modified": 0, "between": 2, "top": true}))
            .as_deref(),
        Some(
            "The change touched no symbol here, only lines outside them. Lines were also removed between symbols in 2 places."
        )
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
    // What is linked with something offers to follow its links, among
    // symbols and into other files.
    assert!(html.starts_with("<h3>Cart.total()</h3><p class=\"graph-map-path\">src/Cart/Cart.php:19</p><p class=\"knowledge-note\">Method of Cart</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-follow\" data-payload=\"&quot;src/Cart/Cart.php#L19:.total()&quot;\">Follow links</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-path\" data-payload=\"{&quot;name&quot;:&quot;Cart.total()&quot;,&quot;path&quot;:&quot;src/Cart/Cart.php#L19:.total()&quot;}\">Path to…</button><h4>Uses <span class=\"graph-map-total\">1</span></h4>"), "{html}");
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
    assert!(html.contains("data-action=\"graph-map-go\" data-changed=\"true\" data-payload=\"{"));
    assert!(html.contains("title=\"src/Cart/Price.php#L9:.of()\"><span class=\"graph-map-row-name\">Price.of()<span class=\"graph-map-row-file\">Price.php</span></span><span class=\"graph-map-row-links\">calls</span>"), "{html}");
    assert!(html.contains(
        "Cart.checkout()</span><span class=\"graph-map-row-links\">indirect call, guessed</span>"
    ));
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
    // It is linked with nothing, so there are no links to follow.
    assert!(html.contains("<p class=\"knowledge-note\">Type</p><h4>Members <span class=\"graph-map-total\">3</span></h4>"), "{html}");
    assert!(!html.contains("graph-map-follow"));
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
    assert!(html.starts_with("<h3>Price.php</h3><p class=\"graph-map-path\">src/Cart/Price.php</p><p class=\"knowledge-note\">Changed file · added · used by this file · 3 symbols</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-open\" data-payload=\"&quot;src/Cart/Price.php&quot;\">Open file</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-follow\" data-payload=\"&quot;src/Cart/Price.php&quot;\">Follow links</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-path\" data-payload=\"{&quot;name&quot;:&quot;Price.php&quot;,&quot;path&quot;:&quot;src/Cart/Price.php&quot;}\">Path to…</button><h4>Uses this file <span"), "{html}");
    assert!(html.contains("<h4>Used by this file <span class=\"graph-map-total\">1</span></h4>"));
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
    // The selected place is drawn whatever the limit says: it is no one
    // of the most connected, and that is said.
    capped["beyond"] = json!(true);
    assert_eq!(
        note(&capped),
        "Showing the 2 most connected of 178 here; filter by name to reach the rest. The selected one is drawn as well. Drawing the 2 strongest of 500 links."
    );
    capped["beyond"] = json!(false);
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
    // So it is where the filter leaves the selected place out.
    let mut kept = view();
    kept["matching"] = json!(0);
    kept["beyond"] = json!(true);
    assert!(note(&kept)
        .starts_with("No name here matches the filter. The selected one is drawn as well."));
    kept["matching"] = json!(2);
    assert!(note(&kept).starts_with("The selected one is drawn as well."));
}
#[test]
fn the_map_marks_the_selection_its_links_and_guesses() {
    let html = canvas(&view());
    assert_eq!(html.matches("<button").count(), 3);
    assert_eq!(html.matches(" marker-end=").count(), 2);
    // The selected folder, its two neighbours undimmed, both links lit.
    assert_eq!(html.matches("data-selected=\"true\"").count(), 1);
    assert_eq!(html.matches("data-dim=\"true\"").count(), 0);
    assert_eq!(html.matches("graph-map-edge-on").count(), 2);
    // Only the link that is all guesses is hollow: drawn wider, with
    // its own middle over it in the colour of the ground.
    assert_eq!(html.matches("graph-map-edge-guess").count(), 1);
    assert_eq!(
        html.matches("<path class=\"graph-map-edge-core\"").count(),
        1
    );
    assert!(
        html.contains("graph-map-edge-guess\" data-ends=\"1 2\""),
        "{html}"
    );
    assert!(html.contains("<path class=\"graph-map-edge-core\" data-ends=\"1 2\""));
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
        "<linearGradient id=\"graph-map-flow-0\" class=\"graph-map-flow\" gradientUnits=\"userSpaceOnUse\" x1=\"{start}\" y1=\"0\" x2=\"{end}\" y2=\"0\"><stop offset=\"0\" class=\"graph-map-flow-uses\"/><stop offset=\"0.5\" class=\"graph-map-flow-between\"/><stop offset=\"1\" class=\"graph-map-flow-used\"/></linearGradient>"
    )), "{html}");
    assert_eq!(html.matches("stroke=\"url(#graph-map-flow-0)\"").count(), 2);
    // All 7 dependency arrow markers are registered, and the two a link
    // leaves by; default edges use the calls arrowhead.
    assert_eq!(html.matches("<marker ").count(), 9);
    assert_eq!(
        html.matches("marker-start=\"url(#graph-map-port)\"")
            .count(),
        2
    );
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
        "<linearGradient id=\"graph-map-flow-1\" class=\"graph-map-flow\" gradientUnits=\"userSpaceOnUse\" x1=\"{end}\" y1=\"0\" x2=\"{start}\""
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
    assert!(folder.starts_with("An arrow runs from what uses <span class=\"graph-map-key graph-map-key-uses\" aria-hidden=\"true\"></span> to what is used <span class=\"graph-map-key graph-map-key-used\" aria-hidden=\"true\"></span>, and a box is tinted by which it does more here. Its head and its dashes tell its kind, as the key of the links shows. Hollow: only guessed by name."), "{folder}");
    assert!(folder.ends_with("Dotted: against the main direction, closing a cycle (↻)."));
    let around = legend(&change(), true);
    assert!(around.ends_with("Highlighted: changed. The rest use the change or are used by it."));
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
    assert!(
        html.contains("Cart/</span><span class=\"graph-map-row-links\">3 links, 1 guessed</span>")
    );
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
    assert!(html.contains("data-action=\"graph-map-gather\" data-payload=\"&quot;src/Cart&quot;\""));
    assert!(html.contains(
        "title=\"Draw Cart as one box again\" aria-label=\"Draw Cart as one box again\">−</button>"
    ));
    assert!(html.contains(
        "title=\"src/Cart\">Cart<span class=\"graph-map-frame-count\"> · 2 of 5 drawn</span></span>"
    ));
    // A folder has + at its corner, in the frame as outside it; a file
    // has none.
    assert_eq!(html.matches("data-action=\"graph-map-spread\"").count(), 2);
    assert!(html
        .contains("data-action=\"graph-map-spread\" data-payload=\"&quot;src/Cart/Rules&quot;\""));
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
    assert!(plan.wires[0].ends_with(&format!(" {:.1} {:.1}", right, second + BOX_HEIGHT / 2.0)));
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
    assert!(html.contains("data-action=\"graph-map-link\" data-payload=\"{&quot;from&quot;:&quot;src/Order&quot;,&quot;to&quot;:&quot;src/Cart&quot;}\" role=\"button\" tabindex=\"0\" focusable=\"true\"><title>Order uses Cart · select to see what makes up this link</title>"));
    // They lie over the links that are seen.
    assert!(
        html.rfind("class=\"graph-map-edge").unwrap()
            < html.find("class=\"graph-map-hit\"").unwrap()
    );
    for other in [change(), file(), trail(), spread_view()] {
        let html = canvas(&other);
        assert_eq!(
            html.matches("<path class=\"graph-map-hit\"").count(),
            html.matches(" marker-end=").count()
        );
    }
    // So does each row of a selected place: the link is named in the
    // direction it runs.
    let side = side(&super::tests::view());
    assert!(side.contains("class=\"graph-map-row-look\" data-action=\"graph-map-link\" data-payload=\"{&quot;from&quot;:&quot;src/Order&quot;,&quot;to&quot;:&quot;src/Cart&quot;}\" title=\"See what makes up this link\" aria-label=\"See what makes up the link with Cart\">↗</button></li>"));
    assert!(side.contains(
        "data-payload=\"{&quot;from&quot;:&quot;tests&quot;,&quot;to&quot;:&quot;src/Order&quot;}\""
    ));
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
    assert_eq!(html.matches(" marker-end=").count(), 4);
    // The one link that was guessed by name is hollow.
    assert_eq!(
        html.matches("<path class=\"graph-map-edge-core\"").count(),
        1
    );
    assert_eq!(
        html.matches("class=\"graph-map-edge graph-map-edge-guess\"")
            .count(),
        1
    );
    assert!(!html.contains("graph-map-hit") && !html.contains("graph-map-link"));
    assert!(html.contains("title=\"src/Order/Notifier.php\">Notifier.php</span>"));
    assert!(html.contains("<title>Order uses Cart · imports from</title>"));
    assert!(html.contains("<title>Order uses Cart.add&lt;T&gt;() · calls, guessed by name</title>"));
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
    assert!(idle.contains(
        "<p class=\"knowledge-note\">Links by kind: calls 2 · imports from 1 · references 1</p>"
    ));
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
    assert!(html.contains("class=\"graph-map-node\" data-n=\"2\" data-action=\"graph-map-node\""));
    assert_eq!(html.matches(" data-ends=\"1 0\" ").count(), 2);
    // A guessed link is hollow, and the middle drawn over it is a part
    // of it: it is lit and steps back with it.
    assert_eq!(html.matches(" data-ends=\"1 2\" ").count(), 3);
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
    // The guessed one twice: the link, and its middle drawn over it.
    assert_eq!(
        ["0 3", "0 4", "1 3", "2 3"]
            .map(|ends| link.matches(&format!(" data-ends=\"{ends}\" ")).count()),
        [1, 2, 1, 1]
    );
}
const TOTAL: &str = "src/Cart/Cart.php#L19:.total()";
const PRICE_OF: &str = "src/Cart/Price.php#L9:.of()";
const TEST_TOTAL: &str = "tests/CartTest.php#L7:.testTotal()";
/// The links of a method followed one step each way: what it calls and
/// what calls it, in other files, and a file that imports its class.
fn traced() -> V {
    let node = |path: &str, name: &str, kind: &str, line: u64, owner: Option<&str>, file: &str| {
        json!({
            "path": path, "name": name, "kind": kind, "line": line, "owner": owner, "file": file,
            "symbol": true, "folder": false, "files": 1, "within": 0, "out": 0, "in": 0, "start": false,
            "more_out": 0, "more_in": 0, "unfolded_out": false, "unfolded_in": false,
            "changed": 0, "status": null, "touched": 0
        })
    };
    let mut start = node(TOTAL, "Cart.total()", "method", 19, Some("Cart"), CART);
    for (key, value) in [
        ("out", json!(2)),
        ("in", json!(3)),
        ("start", json!(true)),
        ("more_in", json!(1)),
        ("unfolded_out", json!(true)),
        ("unfolded_in", json!(true)),
        ("changed", json!(1)),
        ("status", json!("M")),
    ] {
        start[key] = value;
    }
    let mut caller = node(
        TEST_TOTAL,
        "CartTest.testTotal()",
        "method",
        7,
        Some("CartTest"),
        "tests/CartTest.php",
    );
    caller["out"] = json!(2);
    caller["more_out"] = json!(1);
    json!({
        "mode": "follow", "scope": CART, "certain": false,
        "crumbs": [
            {"name": "shop", "path": ""}, {"name": "src", "path": "src"}, {"name": "Cart", "path": "src/Cart"},
            {"name": "Cart.php", "path": CART}, {"name": "Cart.total()", "path": TOTAL}
        ],
        "totals": {"files": 21, "symbols": 22, "relations": 22, "links": 22, "read": 21, "guessed": 1, "unplaced": 0},
        "followed": {"path": TOTAL, "name": "Cart.total()", "folder": false, "symbol": true, "kind": "method", "file": CART, "line": 19,
            "uses": 2, "users": 3, "shown": 4, "limit": 60, "step": 12, "full": false, "aside": 0},
        "children": 4, "matching": 4,
        "nodes": [
            start,
            node(PRICE_OF, "Price.of()", "method", 9, Some("Price"), "src/Cart/Price.php"),
            caller,
            node("src/Order/Notifier.php#L1:Notifier.php", "Notifier<T>.php", "file", 0, None, "src/Order/Notifier.php")
        ],
        "between": 3, "backbone": false,
        "edges": [
            {"from": TOTAL, "to": PRICE_OF, "links": 1, "guessed": 0},
            {"from": TEST_TOTAL, "to": TOTAL, "links": 1, "guessed": 1},
            {"from": "src/Order/Notifier.php#L1:Notifier.php", "to": TOTAL, "links": 1, "guessed": 0}
        ],
        "focus": null
    })
}
#[test]
fn followed_symbols_say_what_they_are_and_which_file_each_lies_in() {
    let view = traced();
    assert_eq!(
        summary(&view),
        "Links of Cart.total(), step by step · uses 2 other names · 3 other names use it · 4 drawn"
    );
    // Two symbols are linked once, so no link is stronger than another.
    assert_eq!(
        note(&view),
        "+ at the right corner of a box shows more of what it uses, + at the left corner more of what uses it, 12 at a time, what was read from the code first; − folds them back."
    );
    // The way back leads to the file the symbol lies in.
    let path = crumbs(&view);
    assert!(path.ends_with("<button type=\"button\" class=\"graph-map-crumb\" data-action=\"graph-map-open\" data-payload=\"&quot;src/Cart/Cart.php&quot;\">Cart.php</button><span aria-hidden=\"true\">/</span><span class=\"graph-map-crumb\" aria-current=\"location\">Cart.total(): links</span>"), "{path}");
    let told = legend(&view, false);
    assert!(told.ends_with("on the left. T type · m method · f function · P property · K const · c case · # heading. A box without a letter is a plain name, or a file itself for its imports."), "{told}");
    assert!(legend(&view, true).ends_with("for its imports. Highlighted: touched by the change."));
    // A box carries the letter of what it is and its name, which keeps
    // its end where it is too long; its title tells its own file.
    let html = canvas(&view);
    assert!(html.starts_with("<div class=\"graph-map-plane\" data-names=\"true\" style=\""));
    assert_eq!(html.matches("<bdi>").count(), 4);
    assert!(html.contains("title=\"Price.of() · src/Cart/Price.php:9\" aria-label=\"Price.of(), method of Price, line 9, uses others through 0 links, is used through 0 links\"><span class=\"graph-map-node-kind\" aria-hidden=\"true\">m</span><span class=\"graph-map-node-name\"><bdi>Price.of()</bdi></span><span class=\"graph-map-node-detail\" aria-hidden=\"true\">Price.php:9</span></button>"), "{html}");
    // The node of a file has no letter and no line, and its name is text.
    assert!(html.contains("title=\"Notifier&lt;T&gt;.php · src/Order/Notifier.php\" aria-label=\"Notifier&lt;T&gt;.php, the file itself, for what belongs to no symbol in it, such as its imports, uses others"), "{html}");
    assert!(html
        .contains("<span class=\"graph-map-node-name\"><bdi>Notifier&lt;T&gt;.php</bdi></span>"));
    // The symbol they start from, with what the change did to it. No
    // box opens: selecting one again would leave the followed links.
    assert!(html.contains("data-payload=\"&quot;src/Cart/Cart.php#L19:.total()&quot;\" data-folder=\"false\" data-kind=\"method\" data-open=\"false\" data-outside=\"false\" data-changed=\"true\""), "{html}");
    assert!(html.contains("aria-label=\"Cart.total(), method of Cart, line 19, uses others through 2 links, is used through 3 links, the links are followed from here, modified\""), "{html}");
    assert!(
        html.contains("<span class=\"graph-map-node-status\" data-status=\"modified\" aria-hidden=\"true\">M</span>")
    );
    assert_eq!(html.matches("data-open=\"true\"").count(), 0);
    // The handles unfold and fold by the name of the symbol.
    assert!(html.contains("data-action=\"graph-map-unfold\" data-payload=\"{&quot;path&quot;:&quot;tests/CartTest.php#L7:.testTotal()&quot;,&quot;side&quot;:&quot;out&quot;}\""), "{html}");
    assert_eq!(html.matches("data-action=\"graph-map-fold\"").count(), 2);
    // A type that holds what the change touched says how many.
    let mut holder = traced();
    holder["nodes"][1]["kind"] = json!("type");
    holder["nodes"][1]["changed"] = json!(1);
    holder["nodes"][1]["touched"] = json!(2);
    let html = canvas(&holder);
    assert!(
        html.contains("is used through 0 links, 2 changed inside\""),
        "{html}"
    );
    assert!(
        html.contains("<span class=\"graph-map-node-status\" data-status=\"modified\" aria-hidden=\"true\">2</span>")
    );
    // Files that are followed are drawn as before.
    let files = canvas(&trail());
    assert!(!files.contains("data-names") && !files.contains("<bdi>"));
}
#[test]
fn the_side_panel_of_a_followed_symbol_leads_to_its_file_and_to_its_own_links() {
    let row = |path: &str, name: &str, file: &str, inside: bool, guessed: u64| {
        json!({
            "path": path, "name": name, "kind": "method", "line": 9, "owner": null, "file": file,
            "symbol": true, "folder": false, "inside": inside, "relation": "calls", "links": 1, "guessed": guessed, "changed": false
        })
    };
    let mut view = traced();
    let mut focus = view["nodes"][0].clone();
    for (key, value) in [
        ("inside", json!(true)),
        (
            "uses",
            json!([row(PRICE_OF, "Price.of()", "src/Cart/Price.php", true, 0)]),
        ),
        ("uses_more", json!(1)),
        (
            "used_by",
            json!([row(
                TEST_TOTAL,
                "CartTest.testTotal()",
                "tests/CartTest.php",
                false,
                1
            )]),
        ),
        ("used_by_more", json!(0)),
        ("kinds", json!([{"kind": "calls", "links": 2}])),
    ] {
        focus[key] = value;
    }
    view["focus"] = focus;
    let html = side(&view);
    // What it is, where it lies and what the change did to it; the way
    // to its file. The links are followed from it already.
    assert!(html.starts_with("<h3>Cart.total()</h3><p class=\"graph-map-path\">src/Cart/Cart.php:19</p><p class=\"knowledge-note\">Method of Cart · modified by the change · the links are followed from here</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-go\" data-payload=\"{&quot;open&quot;:&quot;src/Cart/Cart.php&quot;,&quot;path&quot;:&quot;src/Cart/Cart.php#L19:.total()&quot;}\">Open in its file</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-path\" data-payload=\"{&quot;name&quot;:&quot;Cart.total()&quot;,&quot;path&quot;:&quot;src/Cart/Cart.php#L19:.total()&quot;}\">Path to…</button><h4>Uses <span class=\"graph-map-total\">2</span></h4>"), "{html}");
    assert!(!html.contains("graph-map-follow"));
    // A symbol the picture draws is selected in it and says its file;
    // one it does not draw opens its file. A payload is read as the
    // page reads it: the order of its keys is the build's.
    let asked = html
        .split("class=\"graph-map-row\" data-action=\"graph-map-go\"")
        .skip(1)
        .map(|row| {
            let text = row.split("data-payload=\"").nth(1).unwrap();
            let text = text.split('"').next().unwrap().replace("&quot;", "\"");
            serde_json::from_str::<V>(&text).unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        asked,
        [
            json!({"path": PRICE_OF, "inside": true, "open": null}),
            json!({"path": TEST_TOTAL, "inside": false, "open": "tests/CartTest.php"}),
        ]
    );
    assert!(html.contains("title=\"src/Cart/Price.php#L9:.of()\"><span class=\"graph-map-row-name\">Price.of()<span class=\"graph-map-row-file\">Price.php</span></span><span class=\"graph-map-row-links\">calls</span>"), "{html}");
    assert!(html.contains("CartTest.testTotal()<span class=\"graph-map-row-file\">CartTest.php</span></span><span class=\"graph-map-row-links\">calls, guessed</span>"));
    assert!(html.contains("<p class=\"knowledge-note\">and 1 more</p>"));
    assert!(html.contains("Uses by kind: calls 2"));
    // A link of two symbols says it all: no row looks into it.
    assert!(!html.contains("graph-map-row-look"));
    // Another symbol of the picture offers to follow its own links.
    let mut other = view["nodes"][1].clone();
    for key in ["uses", "used_by"] {
        other[key] = json!([]);
    }
    other["inside"] = json!(true);
    view["focus"] = other;
    let html = side(&view);
    assert!(html.starts_with("<h3>Price.of()</h3><p class=\"graph-map-path\">src/Cart/Price.php:9</p><p class=\"knowledge-note\">Method of Price</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-go\" data-payload=\"{&quot;open&quot;:&quot;src/Cart/Price.php&quot;,&quot;path&quot;:&quot;src/Cart/Price.php#L9:.of()&quot;}\">Open in its file</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-follow\" data-payload=\"&quot;src/Cart/Price.php#L9:.of()&quot;\">Follow links</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-path\" data-payload=\"{&quot;name&quot;:&quot;Price.of()&quot;,&quot;path&quot;:&quot;src/Cart/Price.php#L9:.of()&quot;}\">Path to…</button>"), "{html}");
    // Nothing selected: the hint of followed links.
    view["focus"] = V::Null;
    assert!(side(&view).starts_with(
        "<p class=\"knowledge-note\">Select a box to see what it uses and what uses it."
    ));
}
/// The shortest ways from the entry point to the database: through
/// either file of the API, then the core.
fn routed() -> V {
    let node = |path: &str, name: &str, step: u64| {
        json!({
            "path": path, "name": name, "folder": false, "files": 1, "symbols": 2, "within": 0,
            "out": 2, "in": 1, "start": step == 0, "end": step == 3, "step": step,
            "changed": 0, "status": null
        })
    };
    json!({
        "mode": "path", "scope": "app", "certain": false,
        "crumbs": [{"name": "office", "path": ""}, {"name": "app", "path": "app"}, {"name": "main.php → db<1>.php", "path": ""}],
        "totals": {"files": 21, "symbols": 22, "relations": 22, "links": 22, "read": 21, "guessed": 1, "unplaced": 0},
        "path": {
            "from": {"path": "app/main.php", "name": "main.php", "what": "file"},
            "to": {"path": "lib/db.php", "name": "db<1>.php", "what": "file"},
            "names": false, "found": true, "reversed": false, "within": false,
            "steps": 3, "places": 5, "shown": 5, "links": 5, "wide": 8
        },
        "children": 5, "matching": 5,
        "nodes": [
            node("app/main.php", "main.php", 0),
            node("app/api/orders.php", "orders.php", 1),
            node("app/api/users.php", "users.php", 1),
            node("lib/core.php", "core.php", 2),
            node("lib/db.php", "db<1>.php", 3)
        ],
        "between": 5, "backbone": false,
        "edges": [
            {"from": "lib/core.php", "to": "lib/db.php", "links": 2, "guessed": 0},
            {"from": "app/api/orders.php", "to": "lib/core.php", "links": 1, "guessed": 0},
            {"from": "app/api/users.php", "to": "lib/core.php", "links": 1, "guessed": 0},
            {"from": "app/main.php", "to": "app/api/orders.php", "links": 1, "guessed": 0},
            {"from": "app/main.php", "to": "app/api/users.php", "links": 1, "guessed": 1}
        ],
        "focus": null
    })
}
#[test]
fn a_path_says_which_way_it_runs_how_long_it_is_and_what_it_leaves_out() {
    let view = routed();
    assert_eq!(
        summary(&view),
        "Path from main.php to db<1>.php · 3 links · 5 places on the shortest ways"
    );
    assert_eq!(
        note(&view),
        "Each box uses the next: these are the ways that take the fewest links. Select a link to see what makes it up."
    );
    // The two ends close the path above the picture, and beside them
    // stand the way to ask for the other direction and the way to ask
    // what the two share.
    assert!(crumbs(&view).ends_with(
        "<span class=\"graph-map-crumb\" aria-current=\"location\">main.php → db&lt;1&gt;.php: path</span><span aria-hidden=\"true\">·</span><button type=\"button\" class=\"graph-map-crumb\" data-action=\"graph-map-path-swap\">The other way</button><span aria-hidden=\"true\">·</span><button type=\"button\" class=\"graph-map-crumb\" data-action=\"graph-map-shared\" data-payload=\"true\">What they share</button>"
    ));
    let told = legend(&view, false);
    assert!(
        told.ends_with("Thicker: more links. The ringed boxes are the two ends of the path."),
        "{told}"
    );
    assert!(legend(&view, true).ends_with("the two ends of the path. Highlighted: changed."));
    // More places lie on such ways than a step shows.
    let mut wide = routed();
    wide["path"]["places"] = json!(1031);
    wide["between"] = json!(400);
    assert_eq!(
        summary(&wide),
        "Path from main.php to db<1>.php · 3 links · 1,031 places on the shortest ways, 5 drawn"
    );
    assert!(note(&wide).ends_with(
        "1,031 places lie on such ways; each step shows its 8 most linked. Drawing 5 of 400 links: the strongest into each box and out of it, then the strongest of the rest."
    ));
    // Nothing leads the way that was asked for: the picture runs the
    // other way round and says so, first of all.
    let mut turned = routed();
    turned["path"]["reversed"] = json!(true);
    turned["path"]["from"]["name"] = json!("db.php");
    turned["path"]["to"]["name"] = json!("main.php");
    turned["path"]["steps"] = json!(1);
    assert_eq!(
        summary(&turned),
        "Path from main.php to db.php · 1 link · 5 places on the shortest ways"
    );
    assert!(note(&turned).starts_with(
        "Nothing leads from db.php to main.php: this is the way from main.php to db.php. Each box uses the next"
    ));
    // No way either way, with a hint where guesses are left out.
    let mut none = routed();
    none["path"]["found"] = json!(false);
    none["nodes"] = json!([]);
    none["edges"] = json!([]);
    assert_eq!(summary(&none), "No path between main.php and db<1>.php");
    assert_eq!(
        note(&none),
        "No chain of links leads from main.php to db<1>.php, nor back. The two may still share something that both use, or that uses both: What they share, beside the path above the picture, shows it."
    );
    none["certain"] = json!(true);
    assert!(
        note(&none).ends_with("Links guessed by name are left out; with them there may be a way.")
    );
    // Where there is no path there is no other way to ask for, but
    // what the two share can be asked all the same.
    assert!(crumbs(&none).ends_with(
        ": path</span><span aria-hidden=\"true\">·</span><button type=\"button\" class=\"graph-map-crumb\" data-action=\"graph-map-shared\" data-payload=\"true\">What they share</button>"
    ));
    // One end in the other: the two are not apart, and share nothing
    // as two.
    none["path"]["within"] = json!(true);
    assert!(crumbs(&none).ends_with(": path</span>"));
    assert_eq!(summary(&none), "No path between main.php and db<1>.php");
    assert_eq!(
        note(&none),
        "One of main.php and db<1>.php lies in the other, so no path runs between them. Choose two places that lie apart."
    );
    // Inside a chosen change the graph may be behind the files.
    let mut stale = routed();
    stale["stale"] = json!(true);
    assert!(note(&stale).starts_with("The graph is older than the files"));
}
#[test]
fn the_boxes_of_a_path_are_ringed_at_its_two_ends_and_do_not_open() {
    let html = canvas(&routed());
    assert_eq!(
        html.matches("<button type=\"button\" class=\"graph-map-node\"")
            .count(),
        5
    );
    // The start and the end, each said in words too.
    assert!(html.contains("data-payload=\"&quot;app/main.php&quot;\" data-folder=\"false\" data-kind=\"\" data-open=\"false\""), "{html}");
    assert!(html.contains("data-start=\"true\" style="));
    assert!(
        html.contains("is used through 1 link, the path starts here\""),
        "{html}"
    );
    assert_eq!(
        html.matches("data-start=\"false\" data-end=\"true\" style=")
            .count(),
        1
    );
    assert!(html.contains("aria-label=\"db&lt;1&gt;.php, file, uses others through 2 links, is used through 1 link, the path ends here\""), "{html}");
    // A box is only selected, and has no handles: nothing is unfolded.
    assert_eq!(html.matches("data-open=\"true\"").count(), 0);
    assert!(!html.contains("graph-map-more"));
    assert!(!html.contains("data-names"));
    // Every link can be looked into.
    assert_eq!(html.matches("data-action=\"graph-map-link\"").count(), 5);
    // Among symbols the boxes carry their letters and keep the ends of
    // their names, and the legend tells the letters.
    let mut named = traced();
    named["mode"] = json!("path");
    named["path"] = json!({
        "from": {"path": TEST_TOTAL, "name": "CartTest.testTotal()", "what": "method", "symbol": true},
        "to": {"path": PRICE_OF, "name": "Price.of()", "what": "method", "symbol": true},
        "names": true, "found": true, "reversed": false, "within": false,
        "steps": 2, "places": 3, "shown": 3, "links": 2, "wide": 8
    });
    named["nodes"] = json!([named["nodes"][2], named["nodes"][0], named["nodes"][1]]);
    named["nodes"][0]["start"] = json!(true);
    named["nodes"][1]["start"] = json!(false);
    named["nodes"][2]["end"] = json!(true);
    named["edges"] = json!([
        {"from": TEST_TOTAL, "to": TOTAL, "links": 1, "guessed": 0},
        {"from": TOTAL, "to": PRICE_OF, "links": 1, "guessed": 0}
    ]);
    let html = canvas(&named);
    assert!(html.starts_with("<div class=\"graph-map-plane\" data-names=\"true\" style=\""));
    assert!(html.contains("title=\"Price.of() · src/Cart/Price.php:9\" aria-label=\"Price.of(), method of Price, line 9, uses others through 0 links, is used through 0 links, the path ends here\"><span class=\"graph-map-node-kind\" aria-hidden=\"true\">m</span><span class=\"graph-map-node-name\"><bdi>Price.of()</bdi></span><span class=\"graph-map-node-detail\" aria-hidden=\"true\">Price.php:9</span></button>"), "{html}");
    assert!(html.contains("the path starts here\""));
    assert!(!html.contains("graph-map-more"));
    assert!(legend(&named, true).ends_with("the two ends of the path. T type · m method · f function · P property · K const · c case · # heading. A box without a letter is a plain name, or a file itself for its imports. Highlighted: touched by the change."));
    assert_eq!(
        summary(&named),
        "Path from CartTest.testTotal() to Price.of() · 2 links · 3 places on the shortest ways"
    );
}
#[test]
fn the_side_panel_of_a_path_lists_the_next_and_the_one_before() {
    // Nothing selected: how to read the picture.
    let mut view = routed();
    let idle = side(&view);
    assert!(idle.starts_with("<p class=\"knowledge-note\">Select a box to see what it leads to on the way and what leads to it."));
    assert!(idle.ends_with("The other way asks for the ways from the second place to the first, and What they share for what both use and what uses both.</p>"));
    // A file of the way: what it is, the ways on from it, and the two
    // lists, each row a link that can be looked into.
    let row = |path: &str, name: &str, links: u64| json!({"path": path, "name": name, "folder": false, "inside": true, "links": links, "guessed": 0, "changed": false});
    let mut focus = view["nodes"][3].clone();
    for (key, value) in [
        ("inside", json!(true)),
        ("uses", json!([row("lib/db.php", "db<1>.php", 2)])),
        ("uses_more", json!(0)),
        (
            "used_by",
            json!([
                row("app/api/users.php", "users.php", 1),
                row("app/api/orders.php", "orders.php", 1)
            ]),
        ),
        ("used_by_more", json!(3)),
        ("kinds", json!([{"kind": "calls", "links": 2}])),
    ] {
        focus[key] = value;
    }
    view["focus"] = focus;
    let html = side(&view);
    assert!(html.starts_with("<h3>core.php</h3><p class=\"graph-map-path\">lib/core.php</p><p class=\"knowledge-note\">File · 2 symbols</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-open\" data-payload=\"&quot;lib/core.php&quot;\">Open file</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-follow\" data-payload=\"&quot;lib/core.php&quot;\">Follow links</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-path\" data-payload=\"{&quot;name&quot;:&quot;core.php&quot;,&quot;path&quot;:&quot;lib/core.php&quot;}\">Path to…</button><h4>Next on the way <span class=\"graph-map-total\">1</span></h4>"), "{html}");
    assert!(html.contains("<h4>Before it on the way <span class=\"graph-map-total\">5</span></h4>"));
    assert!(html.contains("<p class=\"knowledge-note\">and 3 more</p>"));
    assert_eq!(html.matches("class=\"graph-map-row-look\"").count(), 3);
    assert!(html.contains("Uses by kind: calls 2"));
    // The ends say what they are to the path, and the first one can
    // be followed like any other.
    view["focus"]["start"] = json!(true);
    let html = side(&view);
    assert!(
        html.contains("<p class=\"knowledge-note\">File · 2 symbols · the path starts here</p>")
    );
    assert!(html.contains("data-action=\"graph-map-follow\""));
    view["focus"]["start"] = json!(false);
    view["focus"]["end"] = json!(true);
    assert!(side(&view).contains("File · 2 symbols · the path ends here</p>"));
    // A symbol of the way leads to its file, to its links and to a
    // path from it.
    let mut named = traced();
    named["mode"] = json!("path");
    named["path"] = json!({"names": true, "found": true});
    let mut focus = named["nodes"][0].clone();
    for (key, value) in [
        ("inside", json!(true)),
        ("uses", json!([])),
        ("used_by", json!([])),
        ("end", json!(false)),
    ] {
        focus[key] = value;
    }
    named["focus"] = focus;
    let html = side(&named);
    assert!(html.starts_with("<h3>Cart.total()</h3><p class=\"graph-map-path\">src/Cart/Cart.php:19</p><p class=\"knowledge-note\">Method of Cart · modified by the change · the path starts here</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-go\" data-payload=\"{&quot;open&quot;:&quot;src/Cart/Cart.php&quot;,&quot;path&quot;:&quot;src/Cart/Cart.php#L19:.total()&quot;}\">Open in its file</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-follow\" data-payload=\"&quot;src/Cart/Cart.php#L19:.total()&quot;\">Follow links</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-path\" data-payload=\"{&quot;name&quot;:&quot;Cart.total()&quot;,&quot;path&quot;:&quot;src/Cart/Cart.php#L19:.total()&quot;}\">Path to…</button><h4>Next on the way <span"), "{html}");
    assert!(html.contains("<h4>Before it on the way <span"));
    // A selected folder of the whole project offers a path from it.
    let mut folder = super::tests::view();
    folder["focus"]["name"] = json!("Order");
    assert!(side(&folder).contains("data-action=\"graph-map-path\" data-payload=\"{&quot;name&quot;:&quot;Order&quot;,&quot;path&quot;:&quot;src/Order&quot;}\">Path to…</button>"));
}
#[test]
fn a_path_that_draws_nothing_leaves_its_two_ends_to_go_on_from() {
    // No chain of links joins the two files: each is named with where it
    // lies, and its links and a path to another place start beside it.
    let mut none = routed();
    none["path"]["found"] = json!(false);
    none["nodes"] = json!([]);
    none["edges"] = json!([]);
    assert_eq!(
        side(&none),
        "<p class=\"knowledge-note\">Nothing is drawn. What they share shows what both use and what uses both. Follow links shows what each of the two reaches, and Path to… asks for the way from it to another place.</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-shared\" data-payload=\"true\">What they share</button><h4>main.php</h4><p class=\"graph-map-path\">app/main.php</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-follow\" data-payload=\"&quot;app/main.php&quot;\">Follow links</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-path\" data-payload=\"{&quot;name&quot;:&quot;main.php&quot;,&quot;path&quot;:&quot;app/main.php&quot;}\">Path to…</button><h4>db&lt;1&gt;.php</h4><p class=\"graph-map-path\">lib/db.php</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-follow\" data-payload=\"&quot;lib/db.php&quot;\">Follow links</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-path\" data-payload=\"{&quot;name&quot;:&quot;db&lt;1&gt;.php&quot;,&quot;path&quot;:&quot;lib/db.php&quot;}\">Path to…</button>"
    );
    // One end in the other draws nothing either. A symbol lies at a line
    // of its file, and a folder is its path.
    none["path"]["within"] = json!(true);
    none["path"]["from"] = json!({"path": "src/Cart", "name": "Cart", "what": "folder"});
    none["path"]["to"] = json!({
        "path": TOTAL, "name": "Cart.total()", "what": "method", "symbol": true,
        "file": "src/Cart/Cart.php", "line": 19
    });
    let html = side(&none);
    assert!(
        html.starts_with("<p class=\"knowledge-note\">Nothing is drawn. Follow links shows what each of the two reaches, and Path to… asks for the way from it to another place.</p><h4>Cart</h4><p class=\"graph-map-path\">src/Cart</p>"),
        "{html}"
    );
    assert!(html.contains("<h4>Cart.total()</h4><p class=\"graph-map-path\">src/Cart/Cart.php:19</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-follow\" data-payload=\"&quot;src/Cart/Cart.php#L19:.total()&quot;\">Follow links</button>"), "{html}");
    // A path that is drawn says how to read it instead.
    assert!(side(&routed()).starts_with("<p class=\"knowledge-note\">Select a box"));
}
#[test]
fn the_other_end_of_a_path_is_chosen_after_the_first() {
    // Nothing is asked for: nothing stands above the side panel, and a
    // box that is selected is no end of a path.
    let state = fresh("pr:7");
    assert_eq!(choosing(&state), "");
    assert_eq!(led(state.clone(), "lib/db.php"), None);
    // A path is asked from a place: the note says from where, what to
    // do, and how to leave it.
    let mut asked = state;
    for (key, value) in [
        ("scope", json!("lib")),
        ("focus", json!("lib/a<b>.php")),
        ("filter", json!("a")),
        ("follow", json!("lib/core.php")),
        ("out", json!(["lib/core.php"])),
        ("into", json!(["lib/core.php"])),
        ("spread", json!(["src"])),
        ("pick", json!({"path": "lib/a<b>.php", "name": "a<b>.php"})),
    ] {
        asked[key] = value;
    }
    assert_eq!(
        choosing(&asked),
        "<div class=\"graph-map-picking\" role=\"status\"><p>Path from <strong>a&lt;b&gt;.php</strong>: select the place it should lead to, a box of any picture or a place the search finds.</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-path-cancel\">Cancel</button></div>"
    );
    // The place itself is no other end, nor is nothing.
    assert_eq!(led(asked.clone(), "lib/a<b>.php"), None);
    assert_eq!(led(asked.clone(), ""), None);
    // Another place is: the path takes the place of what was followed,
    // in the map that is open, and nothing is selected in it.
    let shown = led(asked.clone(), "lib/db.php").unwrap();
    let kept = |state: &V, keys: &[&str]| {
        keys.iter()
            .map(|key| state[*key].clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        kept(
            &shown,
            &["path", "pick", "follow", "out", "into", "focus", "filter"]
        ),
        [
            json!(["lib/a<b>.php", "lib/db.php"]),
            V::Null,
            json!(""),
            json!([]),
            json!([]),
            json!(""),
            json!("")
        ]
    );
    assert_eq!(
        kept(&shown, &["target", "scope", "spread"]),
        kept(&asked, &["target", "scope", "spread"])
    );
    assert_eq!(choosing(&shown), "");
    // Leaving a path for a folder or a file drops the path, and keeps
    // one that is still being asked for.
    let mut both = shown.clone();
    both["pick"] = json!({"path": "lib/db.php", "name": "db.php"});
    let left = unfollowed(both);
    assert_eq!(left["path"], json!([]));
    assert_eq!(left["pick"]["path"], "lib/db.php");
    // A place the search found leaves a shown path as well.
    let rows = m::array(&searched()["rows"]);
    assert_eq!(sought(shown, &rows[0], false)["path"], json!([]));
    // While an end is chosen the search says what a row is for.
    let mut finder = searched();
    finder["found"] = json!(4);
    finder["choosing"] = json!(true);
    assert_eq!(
        found_note(&finder),
        "4 found. ↑ ↓ to move · Enter to lead the path there · Esc to close"
    );
    assert_eq!(fresh("")["path"], json!([]));
    assert_eq!(fresh("")["pick"], V::Null);
}
/// What the two files of the API share, with what both use drawn: the
/// core, and the database, which the second of them uses by a link of
/// its own. The entry point uses both, and is another picture.
fn common() -> V {
    let node = |path: &str, name: &str, shares: V| {
        json!({
            "path": path, "name": name, "folder": false, "files": 1, "symbols": 2, "within": 0,
            "out": 2, "in": 1, "start": false, "end": false, "shares": shares,
            "changed": 0, "status": null
        })
    };
    let mut view = json!({
        "mode": "shared", "scope": "app/api", "certain": false,
        "crumbs": [
            {"name": "office", "path": ""}, {"name": "app", "path": "app"},
            {"name": "api", "path": "app/api"}, {"name": "users.php and orders<1>.php", "path": ""}
        ],
        "totals": {"files": 21, "symbols": 22, "relations": 22, "links": 22, "read": 21, "guessed": 1, "unplaced": 0},
        "shared": {
            "from": {"path": "app/api/users.php", "name": "users.php", "what": "file"},
            "to": {"path": "app/api/orders.php", "name": "orders<1>.php", "what": "file"},
            "names": false, "within": false, "side": "both_use",
            "both_use": 2, "uses_both": 1, "matching": 2, "skip": 0, "shown": 2, "wide": 12
        },
        "children": 4, "matching": 4,
        "nodes": [
            node("app/api/users.php", "users.php", V::Null),
            node("app/api/orders.php", "orders<1>.php", V::Null),
            node("lib/core.php", "core.php", json!([1, 1])),
            node("lib/db.php", "db.php", json!([2, 1]))
        ],
        "between": 4, "backbone": false,
        "edges": [
            {"from": "lib/core.php", "to": "lib/db.php", "links": 2, "guessed": 0},
            {"from": "app/api/orders.php", "to": "lib/core.php", "links": 1, "guessed": 0},
            {"from": "app/api/orders.php", "to": "lib/db.php", "links": 1, "guessed": 1},
            {"from": "app/api/users.php", "to": "lib/core.php", "links": 1, "guessed": 0}
        ],
        "focus": null
    });
    view["nodes"][0]["start"] = json!(true);
    view["nodes"][1]["end"] = json!(true);
    view
}
/// The other picture of the same two files: the entry point, which
/// uses both.
fn common_users() -> V {
    let mut view = common();
    let mut main = view["nodes"][2].clone();
    main["path"] = json!("app/main.php");
    main["name"] = json!("main.php");
    view["shared"]["side"] = json!("uses_both");
    view["shared"]["matching"] = json!(1);
    view["shared"]["shown"] = json!(1);
    view["nodes"] = json!([view["nodes"][0], view["nodes"][1], main]);
    view["edges"] = json!([
        {"from": "app/main.php", "to": "app/api/orders.php", "links": 1, "guessed": 0},
        {"from": "app/main.php", "to": "app/api/users.php", "links": 1, "guessed": 0}
    ]);
    view["between"] = json!(2);
    view
}
const ASK_USERS: &str = "<span aria-hidden=\"true\">·</span><button type=\"button\" class=\"graph-map-crumb\" data-action=\"graph-map-shared\" data-payload=\"&quot;uses_both&quot;\">What uses both</button>";
const ASK_USED: &str = "<span aria-hidden=\"true\">·</span><button type=\"button\" class=\"graph-map-crumb\" data-action=\"graph-map-shared\" data-payload=\"&quot;both_use&quot;\">What both use</button>";
const ASK_PATH: &str = "<span aria-hidden=\"true\">·</span><button type=\"button\" class=\"graph-map-crumb\" data-action=\"graph-map-shared\" data-payload=\"false\">The path between them</button>";
const THROUGH: &str =
    "A way that passes the other of the two does not count: the path between them shows those.";
#[test]
fn what_two_places_share_says_how_much_that_is_and_what_it_leaves_out() {
    // What both use is drawn: it comes first, and the other kind is
    // said to be there.
    let view = common();
    assert_eq!(
        summary(&view),
        "users.php and orders<1>.php both use 2 other files · 1 other file uses both"
    );
    assert_eq!(
        note(&view),
        format!(
            "What both use stands to the right of the two, each with one shortest way from either of them, the strongest of those as short. {THROUGH}"
        )
    );
    // The two close the path above the picture, with the kind that is
    // drawn; beside them stand the other kind and the way back to the
    // path between them.
    assert!(
        crumbs(&view).ends_with(&format!(
            "<span class=\"graph-map-crumb\" aria-current=\"location\">users.php and orders&lt;1&gt;.php: what both use</span>{ASK_USERS}{ASK_PATH}"
        )),
        "{}",
        crumbs(&view)
    );
    assert!(!crumbs(&view).contains("The other way"));
    let told = legend(&view, false);
    assert!(
        told.ends_with("Thicker: more links. Dotted: against the main direction, closing a cycle (↻). The ringed boxes are the two places. A name in bold is what both use; a box with a plain name lies on the way to it."),
        "{told}"
    );
    assert!(legend(&view, true).ends_with("lies on the way to it. Highlighted: changed."));
    // What uses both is drawn: the same the other way round.
    let users = common_users();
    assert_eq!(
        summary(&users),
        "1 other file uses both users.php and orders<1>.php · both use 2 other files"
    );
    assert_eq!(
        note(&users),
        format!(
            "What uses both stands to the left of the two, each with one shortest way to either of them, the strongest of those as short. {THROUGH}"
        )
    );
    assert!(
        crumbs(&users).ends_with(&format!(": what uses both</span>{ASK_USED}{ASK_PATH}")),
        "{}",
        crumbs(&users)
    );
    assert!(legend(&users, false).ends_with("closing a cycle (↻). The ringed boxes are the two places. A name in bold is what uses both; a box with a plain name lies on its way to them."));
    // More is shared than a picture shows: the nearest are drawn, and
    // the note says how the rest is reached.
    let mut wide = common();
    for (key, value) in [("both_use", 1037), ("matching", 1037), ("shown", 12)] {
        wide["shared"][key] = json!(value);
    }
    assert_eq!(
        summary(&wide),
        "users.php and orders<1>.php both use 1,037 other files, 12 drawn · 1 other file uses both"
    );
    let order = " The nearest come first: the fewest links from the two taken together, then the most linked. Filter names reaches a place by its name.";
    assert_eq!(note(&wide), format!("{}{order}", note(&common())));
    // A picture further on says which places it shows.
    wide["shared"]["skip"] = json!(12);
    assert_eq!(
        summary(&wide),
        "users.php and orders<1>.php both use 1,037 other files, places 13 to 24 drawn · 1 other file uses both"
    );
    assert_eq!(note(&wide), format!("{}{order}", note(&common())));
    wide["shared"]["skip"] = json!(1036);
    wide["shared"]["shown"] = json!(1);
    assert_eq!(
        summary(&wide),
        "users.php and orders<1>.php both use 1,037 other files, place 1,037 drawn · 1 other file uses both"
    );
    assert_eq!(note(&wide), format!("{}{order}", note(&common())));
    // A filter leaves some of them: the nearest of those are drawn.
    for (key, value) in [("matching", 31), ("skip", 0), ("shown", 12)] {
        wide["shared"][key] = json!(value);
    }
    assert_eq!(
        summary(&wide),
        "users.php and orders<1>.php both use 1,037 other files, 31 match the filter, 12 drawn · 1 other file uses both"
    );
    assert_eq!(note(&wide), format!("{}{order}", note(&common())));
    wide["shared"]["skip"] = json!(24);
    wide["shared"]["shown"] = json!(7);
    assert_eq!(
        summary(&wide),
        "users.php and orders<1>.php both use 1,037 other files, 31 match the filter, places 25 to 31 drawn · 1 other file uses both"
    );
    assert_eq!(note(&wide), format!("{}{order}", note(&common())));
    // It leaves few enough for one picture, one, or none.
    for (key, value) in [("matching", 5), ("skip", 0), ("shown", 5)] {
        wide["shared"][key] = json!(value);
    }
    assert_eq!(
        summary(&wide),
        "users.php and orders<1>.php both use 1,037 other files, 5 match the filter · 1 other file uses both"
    );
    // One picture shows them all: the line above says how many.
    assert_eq!(note(&wide), note(&common()));
    wide["shared"]["matching"] = json!(1);
    wide["shared"]["shown"] = json!(1);
    assert!(summary(&wide).contains(" other files, 1 matches the filter · 1 other"));
    wide["shared"]["matching"] = json!(0);
    wide["shared"]["shown"] = json!(0);
    wide["nodes"] = json!([]);
    wide["edges"] = json!([]);
    assert_eq!(
        summary(&wide),
        "users.php and orders<1>.php both use 1,037 other files, none matches the filter · 1 other file uses both"
    );
    assert_eq!(
        note(&wide),
        "None of the 1,037 that both use matches the filter."
    );
    // What uses both says the same the other way round.
    let mut wide = common_users();
    for (key, value) in [("uses_both", 20), ("matching", 20), ("shown", 9)] {
        wide["shared"][key] = json!(value);
    }
    assert_eq!(
        summary(&wide),
        "20 other files use both users.php and orders<1>.php, 9 drawn · both use 2 other files"
    );
    assert_eq!(
        note(&wide),
        format!(
            "{} The nearest come first: the fewest links to the two taken together, then the most linked. Filter names reaches a place by its name.",
            note(&common_users())
        )
    );
    wide["shared"]["matching"] = json!(0);
    wide["shared"]["shown"] = json!(0);
    assert_eq!(
        summary(&wide),
        "20 other files use both users.php and orders<1>.php, none matches the filter · both use 2 other files"
    );
    assert_eq!(
        note(&wide),
        "None of the 20 that use both matches the filter."
    );
    // One alone that uses both and that a filter leaves out.
    wide["shared"]["uses_both"] = json!(1);
    assert_eq!(
        summary(&wide),
        "1 other file uses both users.php and orders<1>.php, none matches the filter · both use 2 other files"
    );
    // One kind alone: the other has no picture to go to.
    let mut alone = common();
    alone["shared"]["uses_both"] = json!(0);
    assert_eq!(
        summary(&alone),
        "users.php and orders<1>.php both use 2 other files · nothing uses both"
    );
    assert_eq!(note(&alone), note(&common()));
    assert!(crumbs(&alone).ends_with(&format!(": what both use</span>{ASK_PATH}")));
    // A kind that was asked for and has nothing: nothing is drawn, and
    // the other kind is said to be there.
    let mut asked = common();
    asked["shared"]["both_use"] = json!(0);
    asked["shared"]["uses_both"] = json!(3);
    asked["shared"]["matching"] = json!(0);
    asked["shared"]["shown"] = json!(0);
    asked["nodes"] = json!([]);
    asked["edges"] = json!([]);
    assert_eq!(
        summary(&asked),
        "users.php and orders<1>.php use nothing in common · 3 other files use both"
    );
    assert_eq!(
        note(&asked),
        format!(
            "users.php and orders<1>.php use nothing in common. {THROUGH} What uses both is another picture: What uses both, beside the path above this one."
        )
    );
    asked["shared"]["side"] = json!("uses_both");
    asked["shared"]["both_use"] = json!(1);
    asked["shared"]["uses_both"] = json!(0);
    assert_eq!(
        summary(&asked),
        "Nothing uses both users.php and orders<1>.php · both use 1 other file"
    );
    assert_eq!(
        note(&asked),
        format!(
            "Nothing uses both users.php and orders<1>.php. {THROUGH} What they use in common is another picture: What both use, beside the path above this one."
        )
    );
    // Nothing of either kind, with a hint where guesses are left out.
    // The path between the two can still be asked for.
    let mut none = asked;
    none["shared"]["both_use"] = json!(0);
    assert_eq!(summary(&none), "users.php and orders<1>.php share nothing");
    assert_eq!(
        note(&none),
        format!(
            "Nothing is used by both users.php and orders<1>.php, and nothing uses both. {THROUGH}"
        )
    );
    none["certain"] = json!(true);
    assert!(note(&none)
        .ends_with("Links guessed by name are left out; with them there may be something."));
    assert!(crumbs(&none).ends_with(&format!(": what uses both</span>{ASK_PATH}")));
    // One of the two in the other: they are not apart, and there is
    // nothing more to ask of them.
    none["shared"]["within"] = json!(true);
    assert_eq!(summary(&none), "users.php and orders<1>.php share nothing");
    assert_eq!(
        note(&none),
        "One of users.php and orders<1>.php lies in the other, so the two are not apart. Choose two places that lie apart."
    );
    assert!(crumbs(&none).ends_with(": what uses both</span>"));
    // Among symbols what is shared is names.
    let mut named = common();
    named["shared"]["names"] = json!(true);
    assert_eq!(
        summary(&named),
        "users.php and orders<1>.php both use 2 other names · 1 other name uses both"
    );
    assert!(legend(&named, true).ends_with("lies on the way to it. T type · m method · f function · P property · K const · c case · # heading. A box without a letter is a plain name, or a file itself for its imports. Highlighted: touched by the change."));
    // Inside a chosen change the graph may be behind the files.
    let mut stale = common();
    stale["stale"] = json!(true);
    assert!(note(&stale).starts_with("The graph is older than the files"));
}
#[test]
fn the_boxes_of_what_two_places_share_are_ringed_or_bold_and_do_not_open() {
    let html = canvas(&common());
    assert_eq!(
        html.matches("<button type=\"button\" class=\"graph-map-node\"")
            .count(),
        4
    );
    // The two are ringed, each said in words too.
    assert_eq!(html.matches("data-start=\"true\" style=").count(), 1);
    assert_eq!(
        html.matches("data-start=\"false\" data-end=\"true\" style=")
            .count(),
        1
    );
    assert!(html.contains("aria-label=\"users.php, file, uses others through 2 links, is used through 1 link, the first of the two places\""), "{html}");
    assert!(html.contains("aria-label=\"orders&lt;1&gt;.php, file, uses others through 2 links, is used through 1 link, the second of the two places\""), "{html}");
    // What both use is marked, and says so.
    assert_eq!(
        html.matches("data-start=\"false\" data-shared=\"true\" style=")
            .count(),
        2
    );
    assert!(html.contains("aria-label=\"core.php, file, uses others through 2 links, is used through 1 link, both use it\""), "{html}");
    // A box is only selected, and has no handles: nothing is unfolded.
    assert_eq!(html.matches("data-open=\"true\"").count(), 0);
    assert!(!html.contains("graph-map-more"));
    assert!(!html.contains("graph-map-spread"));
    // Every link can be looked into, the guessed one drawn as a guess.
    assert_eq!(html.matches("data-action=\"graph-map-link\"").count(), 4);
    assert_eq!(html.matches("graph-map-edge-guess").count(), 1);
    // What uses both says that instead.
    let html = canvas(&common_users());
    assert_eq!(html.matches("data-shared=\"true\"").count(), 1);
    assert!(html.contains("aria-label=\"main.php, file, uses others through 2 links, is used through 1 link, uses both\""), "{html}");
    // A place on the way to what is shared is a plain box.
    let mut view = common();
    view["nodes"][2]["shares"] = V::Null;
    let html = canvas(&view);
    assert_eq!(html.matches("data-shared=\"true\"").count(), 1);
    assert!(
        html.contains(
            "aria-label=\"core.php, file, uses others through 2 links, is used through 1 link\""
        ),
        "{html}"
    );
    // One of the two can be a folder among files, drawn as one.
    view["nodes"][0]["folder"] = json!(true);
    view["nodes"][0]["files"] = json!(7);
    let html = canvas(&view);
    assert!(html.contains("data-payload=\"&quot;app/api/users.php&quot;\" data-folder=\"true\" data-kind=\"\" data-open=\"false\""), "{html}");
    assert!(html.contains("aria-label=\"users.php, folder, 7 files, uses others through 2 links, is used through 1 link, the first of the two places\""), "{html}");
    // The two stand side by side whatever their links say: here the
    // second leads to the database alone, a layer further on, and in a
    // picture that is none of two places it would stand a layer in.
    let left = |html: &str, path: &str| {
        let at = html
            .find(&format!("data-payload=\"&quot;{path}&quot;\" data-folder"))
            .unwrap();
        let from = at + html[at..].find("style=\"left:").unwrap() + 12;
        html[from..].split("px").next().unwrap().to_owned()
    };
    let mut far = common();
    far["edges"] = json!([
        {"from": "lib/core.php", "to": "lib/db.php", "links": 2, "guessed": 0},
        {"from": "app/api/orders.php", "to": "lib/db.php", "links": 1, "guessed": 1},
        {"from": "app/api/users.php", "to": "lib/core.php", "links": 1, "guessed": 0}
    ]);
    let html = canvas(&far);
    assert_eq!(
        (
            left(&html, "app/api/users.php"),
            left(&html, "app/api/orders.php")
        ),
        ("12".to_owned(), "12".to_owned())
    );
    far["mode"] = json!("path");
    let html = canvas(&far);
    assert_ne!(
        left(&html, "app/api/users.php"),
        left(&html, "app/api/orders.php")
    );
    // A picture that is none of two places marks nothing, whatever a
    // box says of itself.
    let mut other = routed();
    other["nodes"][1]["shares"] = json!([1, 1]);
    assert!(!canvas(&other).contains("data-shared"));
    // Among symbols the boxes carry their letters and keep the ends of
    // their names.
    let mut named = traced();
    named["mode"] = json!("shared");
    named["shared"] = json!({
        "from": {"path": TEST_TOTAL, "name": "CartTest.testTotal()", "what": "method", "symbol": true},
        "to": {"path": PRICE_OF, "name": "Price.of()", "what": "method", "symbol": true},
        "names": true, "within": false, "side": "both_use",
        "both_use": 1, "uses_both": 0, "matching": 1, "skip": 0, "shown": 1, "wide": 12
    });
    named["nodes"] = json!([named["nodes"][2], named["nodes"][1], named["nodes"][0]]);
    named["nodes"][0]["start"] = json!(true);
    named["nodes"][1]["end"] = json!(true);
    named["nodes"][2]["start"] = json!(false);
    named["nodes"][2]["shares"] = json!([1, 1]);
    named["edges"] = json!([
        {"from": TEST_TOTAL, "to": TOTAL, "links": 1, "guessed": 0},
        {"from": PRICE_OF, "to": TOTAL, "links": 1, "guessed": 0}
    ]);
    let html = canvas(&named);
    assert!(html.starts_with("<div class=\"graph-map-plane\" data-names=\"true\" style=\""));
    assert!(html.contains("is used through 3 links, both use it, modified\"><span class=\"graph-map-node-kind\" aria-hidden=\"true\">m</span><span class=\"graph-map-node-name\"><bdi>Cart.total()</bdi></span>"), "{html}");
    assert!(html.contains("aria-label=\"Price.of(), method of Price, line 9, uses others through 0 links, is used through 0 links, the second of the two places\""), "{html}");
    assert!(!html.contains("graph-map-more"));
    assert_eq!(
        summary(&named),
        "CartTest.testTotal() and Price.of() both use 1 other name · nothing uses both"
    );
}
#[test]
fn switching_cycle_revisions_keeps_the_pr_origin_and_current_filters() {
    let mut original = fresh("pr:1");
    original["focus"] = json!("src/renamed.php");
    let mut current = showing_cycle(original.clone(), "src/renamed.php#L8:a()").unwrap();
    current["focus"] = json!("src/b.php#L5:b()");
    let comparison = json!({
        "state": "ready", "origin": "present", "place": "src/original.php#L3:a()",
        "revision": "abc123",
    });
    let mut earlier = switch_cycle(current.clone(), &comparison).unwrap();
    assert_eq!(earlier["cycle"], "src/renamed.php#L8:a()");
    assert_eq!(earlier["focus"], "src/original.php#L3:a()");
    assert_eq!(earlier["cycle_return"], original);
    earlier["focus"] = json!("src/c.php#L4:c()");
    earlier["certain"] = json!(true);
    earlier["notests"] = json!(true);
    let resumed = switch_cycle(earlier, &V::Null).unwrap();
    assert_eq!(resumed["focus"], current["focus"]);
    assert!(m::yes(&resumed["certain"]) && m::yes(&resumed["notests"]));
    assert_eq!(resumed["cycle_return"], original);
    assert!(resumed["cycle_base"].is_null());

    let head = json!({"cycle": {"comparison": comparison}});
    assert!(base_cycle_button(&head, false).contains("Show base cycle"));
    assert!(base_cycle_button(&V::Null, true).contains("Back to PR cycle"));
    assert!(base_cycle_button(&V::Null, false).is_empty());
    assert_eq!(switch_cycle(resumed.clone(), &V::Null), Some(resumed));
}
#[test]
fn a_cycle_keeps_the_previous_filtered_page_as_its_return_point() {
    let mut map = fresh("work");
    map["scope"] = json!("src/api");
    map["path"] = json!(["src/api/a.php", "src/api/b.php"]);
    let mut map = sharing(map, &json!("uses_both")).unwrap();
    map["filter"] = json!("src/");
    let mut map = paged(paged(map, Some(9)), Some(21));
    map["focus"] = json!("src/shared.php");
    map["fit"] = json!(true);

    let mut cycle = showing_cycle(map.clone(), "src/shared.php").unwrap();
    assert_eq!(cycle["cycle"], "src/shared.php");
    assert_eq!(cycle["target"], "work");
    assert_eq!(cycle["filter"], "");
    assert!(begun(&cycle).is_empty());
    assert!(!m::yes(&cycle["shared"]));
    assert!(showing_cycle(cycle.clone(), "src/other.php").is_none());

    cycle["certain"] = json!(true);
    cycle["notests"] = json!(true);
    cycle["fit"] = json!(false);
    cycle["focus"] = json!("src/other.php");
    assert!(unfollowed(cycle.clone())["cycle_return"].is_null());
    let restored = cycle["cycle_return"].take();
    assert_eq!(restored, map);
    assert_eq!(begun(&paged(restored, None)), [9]);
}
#[test]
fn a_cycle_marks_its_return_and_explains_when_no_cycle_remains() {
    let mut view = json!({
        "mode": "cycle",
        "cycle": {"origin": {"name": "a.php"}, "steps": 2},
        "crumbs": [{"name": "a.php"}],
        "nodes": [
            {"path": "a.php", "name": "a.php", "start": true},
            {"path": "b.php", "name": "b.php"}
        ],
        "edges": [
            {"from": "a.php", "to": "b.php", "links": 2, "guessed": 0},
            {"from": "b.php", "to": "a.php", "links": 1, "guessed": 1}
        ],
        "focus": {"path": "a.php", "name": "a.php", "start": true}
    });
    assert_eq!(shown(&view), "Shortest cycle through a.php · 2 links");
    assert!(crumbs(&view).contains("a.php: cycle"));
    assert!(!crumbs(&view).contains("What they share"));
    let html = canvas(&view);
    assert!(html.contains("data-cycle-view=\"true\""));
    assert!(html.contains("graph-map-edge-on graph-map-edge-guess graph-map-edge-back"));
    assert_eq!(html.matches("data-start=\"true\"").count(), 1);
    assert!(!html.contains("graph-map-edge-off"));

    view["cycle"]["comparison"] = json!({
        "state": "building", "base": {"commit": "123456789abc"}
    });
    assert_eq!(
        cycle_comparison(&view),
        "Preparing the graph at base 123456789abc…"
    );
    view["cycle"]["comparison"]["state"] = json!("ready");
    view["cycle"]["comparison"]["origin"] = json!("present");
    assert_eq!(
        cycle_comparison(&view),
        "A cycle through this place already existed at base 123456789abc."
    );
    view["edges"][1]["added"] = json!(true);
    assert_eq!(
        canvas(&view).matches("data-cycle-added=\"true\"").count(),
        1
    );
    assert!(legend(&view, true).contains("Green links were added in this PR."));

    view["certain"] = json!(true);
    view["cycle"]["steps"] = json!(0);
    assert_eq!(
        shown(&view),
        "No cycle found through a.php using links read from the code"
    );
    view["cycle"]["origin"]["missing"] = json!(true);
    assert_eq!(shown(&view), "a.php is absent from this graph");

    view["focus"] = V::Null;
    view["cycle"]["places"] = json!([
        {"path": "a.php"},
        {"path": "b<&>.php", "links": 1, "guessed": 0},
        {"path": "a.php", "links": 1, "guessed": 1, "added": true}
    ]);
    assert!(
        side(&view)
            .contains("<li>b&lt;&amp;&gt;.php</li><li>a.php · incoming link inferred · incoming link added in PR</li>")
    );
}
#[test]
fn selecting_shared_places_lights_their_drawn_paths() {
    let mut view = common();
    view["focus"] = json!({
        "path": "lib/db.php",
        "shared_edges": [
            ["app/api/users.php", "lib/core.php"],
            ["app/api/orders.php", "lib/db.php"],
            ["lib/core.php", "lib/db.php"]
        ]
    });
    let html = canvas(&view);
    assert!(html.contains(
        "graph-map-edge-on graph-map-edge-guess\" data-ends=\"1 3\" data-shared-path=\"true\""
    ));
    assert!(html.contains("graph-map-edge-on\" data-ends=\"0 2\" data-shared-path=\"true\""));
    assert!(html.contains("graph-map-edge-on\" data-ends=\"2 3\" data-shared-path=\"true\""));
    assert!(html.contains("graph-map-edge-off\" data-ends=\"1 2\" d="));
    assert_eq!(html.matches("data-shared-path=\"true\"").count(), 7);
    assert_eq!(html.matches("data-action=\"graph-map-link\"").count(), 4);
    assert_eq!(html.matches("aria-pressed=\"true\"").count(), 1);
    assert!(!html.contains("data-dim=\"true\""));

    let mut view = common_users();
    view["focus"] = json!({
        "path": "app/main.php",
        "shared_edges": [
            ["app/main.php", "app/api/users.php"],
            ["app/main.php", "app/api/orders.php"]
        ]
    });
    let html = canvas(&view);
    assert!(html.contains("graph-map-edge-on\" data-ends=\"2 0\" data-shared-path=\"true\""));
    assert!(html.contains("graph-map-edge-on\" data-ends=\"2 1\" data-shared-path=\"true\""));
    assert_eq!(html.matches("data-shared-path=\"true\"").count(), 5);

    view["focus"] = json!({"path": "app/api/users.php"});
    let html = canvas(&view);
    assert!(!html.contains("data-shared-path"));
    assert!(html.contains("graph-map-edge-on\" data-ends=\"2 0\" d="));
    assert!(html.contains("graph-map-edge-off\" data-ends=\"2 1\" d="));
    view["focus"] = V::Null;
    let html = canvas(&view);
    assert!(!html.contains("graph-map-edge-on"));
    assert!(!html.contains("graph-map-edge-off"));
}
#[test]
fn the_side_panel_of_what_two_places_share_says_what_a_box_is_to_them() {
    // Nothing selected: how to highlight the paths or inspect a link.
    let mut view = common();
    let idle = side(&view);
    assert!(idle.contains("Select a shared place to highlight both paths."));
    // What both use: how far each of the two is, the ways on from it,
    // and the two lists, each row a link that can be looked into.
    let row = |path: &str, name: &str, links: u64| json!({"path": path, "name": name, "folder": false, "inside": true, "links": links, "guessed": 0, "changed": false});
    let mut focus = view["nodes"][3].clone();
    for (key, value) in [
        ("inside", json!(true)),
        ("uses", json!([])),
        ("uses_more", json!(0)),
        (
            "used_by",
            json!([
                row("lib/core.php", "core.php", 2),
                row("app/api/orders.php", "orders<1>.php", 1)
            ]),
        ),
        ("used_by_more", json!(0)),
        ("kinds", json!([])),
    ] {
        focus[key] = value;
    }
    view["focus"] = focus;
    let html = side(&view);
    assert!(html.starts_with("<h3>db.php</h3><p class=\"graph-map-path\">lib/db.php</p><p class=\"knowledge-note\">File · 2 symbols · both use it: 2 links from users.php, 1 link from orders&lt;1&gt;.php</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-open\" data-payload=\"&quot;lib/db.php&quot;\">Open file</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-follow\" data-payload=\"&quot;lib/db.php&quot;\">Follow links</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-path\" data-payload=\"{&quot;name&quot;:&quot;db.php&quot;,&quot;path&quot;:&quot;lib/db.php&quot;}\">Path to…</button><h4>Next on the way <span class=\"graph-map-total\">0</span></h4>"), "{html}");
    assert!(html.contains("<h4>Before it on the way <span class=\"graph-map-total\">2</span></h4>"));
    assert_eq!(html.matches("class=\"graph-map-row-look\"").count(), 2);
    // What uses both says how far the two are from it.
    view["shared"]["side"] = json!("uses_both");
    view["focus"]["shares"] = json!([1, 3]);
    assert!(side(&view).contains(
        "File · 2 symbols · uses both: 1 link to users.php, 3 links to orders&lt;1&gt;.php</p>"
    ));
    view["shared"]["side"] = json!("both_use");
    // A box on the way says nothing more, and each of the two says
    // which it is and can be followed like any other.
    view["focus"]["shares"] = V::Null;
    assert!(side(&view).contains("<p class=\"knowledge-note\">File · 2 symbols</p>"));
    view["focus"]["start"] = json!(true);
    let html = side(&view);
    assert!(html.contains("File · 2 symbols · the first of the two places</p>"));
    assert!(html.contains("data-action=\"graph-map-follow\""));
    view["focus"]["start"] = json!(false);
    view["focus"]["end"] = json!(true);
    assert!(side(&view).contains("File · 2 symbols · the second of the two places</p>"));
    // One of the two that is a folder is one, with the way into it.
    view["focus"]["folder"] = json!(true);
    view["focus"]["files"] = json!(7);
    let html = side(&view);
    assert!(html.contains("<p class=\"knowledge-note\">Folder · 7 files · the second of the two places</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-open\" data-payload=\"&quot;lib/db.php&quot;\">Open folder</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-follow\""), "{html}");
    // A symbol that is shared leads to its file, to its links and to a
    // path from it.
    let mut named = traced();
    named["mode"] = json!("shared");
    named["shared"] = json!({
        "from": {"path": TEST_TOTAL, "name": "CartTest.testTotal()"},
        "to": {"path": PRICE_OF, "name": "Price.of()"},
        "names": true, "within": false, "side": "both_use"
    });
    let mut focus = named["nodes"][0].clone();
    for (key, value) in [
        ("inside", json!(true)),
        ("uses", json!([])),
        ("used_by", json!([])),
        ("start", json!(false)),
        ("shares", json!([1, 1])),
    ] {
        focus[key] = value;
    }
    named["focus"] = focus;
    let html = side(&named);
    assert!(html.starts_with("<h3>Cart.total()</h3><p class=\"graph-map-path\">src/Cart/Cart.php:19</p><p class=\"knowledge-note\">Method of Cart · modified by the change · both use it: 1 link from CartTest.testTotal(), 1 link from Price.of()</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-go\" data-payload=\"{&quot;open&quot;:&quot;src/Cart/Cart.php&quot;,&quot;path&quot;:&quot;src/Cart/Cart.php#L19:.total()&quot;}\">Open in its file</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-follow\" data-payload=\"&quot;src/Cart/Cart.php#L19:.total()&quot;\">Follow links</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-path\" data-payload=\"{&quot;name&quot;:&quot;Cart.total()&quot;,&quot;path&quot;:&quot;src/Cart/Cart.php#L19:.total()&quot;}\">Path to…</button><h4>Next on the way <span"), "{html}");
    // A kind that has nothing while the other has: the other kind and
    // the path between the two are asked for from here, and each of
    // the two is left to go on from.
    let ends = "<h4>users.php</h4><p class=\"graph-map-path\">app/api/users.php</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-follow\" data-payload=\"&quot;app/api/users.php&quot;\">Follow links</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-path\" data-payload=\"{&quot;name&quot;:&quot;users.php&quot;,&quot;path&quot;:&quot;app/api/users.php&quot;}\">Path to…</button><h4>orders&lt;1&gt;.php</h4><p class=\"graph-map-path\">app/api/orders.php</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-follow\" data-payload=\"&quot;app/api/orders.php&quot;\">Follow links</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-path\" data-payload=\"{&quot;name&quot;:&quot;orders&lt;1&gt;.php&quot;,&quot;path&quot;:&quot;app/api/orders.php&quot;}\">Path to…</button>";
    let mut none = common();
    none["shared"]["both_use"] = json!(0);
    none["shared"]["matching"] = json!(0);
    none["shared"]["shown"] = json!(0);
    none["nodes"] = json!([]);
    none["edges"] = json!([]);
    assert_eq!(
        side(&none),
        format!(
            "<p class=\"knowledge-note\">Nothing is drawn. What uses both is another picture. The path between them asks how one of the two reaches the other. Follow links shows what each of the two reaches, and Path to… asks for the way from it to another place.</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-shared\" data-payload=\"&quot;uses_both&quot;\">What uses both</button><button type=\"button\" class=\"button compact\" data-action=\"graph-map-shared\" data-payload=\"false\">The path between them</button>{ends}"
        )
    );
    none["shared"]["side"] = json!("uses_both");
    none["shared"]["both_use"] = json!(4);
    none["shared"]["uses_both"] = json!(0);
    assert!(side(&none).starts_with("<p class=\"knowledge-note\">Nothing is drawn. What both use is another picture. The path between them asks how one of the two reaches the other. Follow links shows"));
    // Two places that share nothing are left with the path alone.
    none["shared"]["both_use"] = json!(0);
    assert_eq!(
        side(&none),
        format!(
            "<p class=\"knowledge-note\">Nothing is drawn. The path between them asks how one of the two reaches the other. Follow links shows what each of the two reaches, and Path to… asks for the way from it to another place.</p><button type=\"button\" class=\"button compact\" data-action=\"graph-map-shared\" data-payload=\"false\">The path between them</button>{ends}"
        )
    );
    // One of the two in the other leaves no path to ask for.
    none["shared"]["within"] = json!(true);
    assert_eq!(
        side(&none),
        format!(
            "<p class=\"knowledge-note\">Nothing is drawn. Follow links shows what each of the two reaches, and Path to… asks for the way from it to another place.</p>{ends}"
        )
    );
}
#[test]
fn what_two_places_share_is_gone_through_a_picture_at_a_time() {
    let turn = |label: &str, to: &str, open: bool| {
        format!(
            "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-shared-page\" data-payload=\"{to}\"{}>{label}</button>",
            if open { "" } else { " disabled" }
        )
    };
    let strip = |told: &str, near: bool, next: &str, far: bool| {
        format!(
            "<div class=\"graph-map-paging\"><p>{told}, the nearest first.</p>{}{}</div>",
            turn("Nearer", "null", near),
            turn("Further", next, far)
        )
    };
    // One picture shows all there is: nothing to go through. Nor is
    // there in a picture that is none of what two places share.
    let mut view = common();
    assert_eq!(paging(&view), "");
    assert_eq!(paging(&routed()), "");
    // More than a picture shows: the first one leads further only.
    for (key, value) in [("both_use", 1037), ("matching", 1037), ("shown", 12)] {
        view["shared"][key] = json!(value);
    }
    assert_eq!(
        paging(&view),
        strip("Places 1 to 12 of 1,037", false, "12", true)
    );
    // One in the middle leads both ways, and the next begins after
    // what this one shows, however many that is.
    view["shared"]["skip"] = json!(12);
    view["shared"]["shown"] = json!(9);
    assert_eq!(
        paging(&view),
        strip("Places 13 to 21 of 1,037", true, "21", true)
    );
    // The last leads back only, and may show one place.
    view["shared"]["skip"] = json!(1036);
    view["shared"]["shown"] = json!(1);
    assert_eq!(
        paging(&view),
        strip("Place 1,037 of 1,037", true, "1037", false)
    );
    // What a filter leaves is gone through the same way.
    for (key, value) in [("matching", 31), ("skip", 24), ("shown", 7)] {
        view["shared"][key] = json!(value);
    }
    assert_eq!(
        paging(&view),
        strip(
            "Places 25 to 31 of 31 that match the filter",
            true,
            "31",
            false
        )
    );
    // Few enough for one picture, or nothing at all: no strip.
    for (key, value) in [("matching", 5), ("skip", 0), ("shown", 5)] {
        view["shared"][key] = json!(value);
    }
    assert_eq!(paging(&view), "");
    view["shared"]["matching"] = json!(0);
    view["shared"]["shown"] = json!(0);
    assert_eq!(paging(&view), "");
    // What uses both counts its own kind.
    let mut users = common_users();
    for (key, value) in [("uses_both", 20), ("matching", 20), ("shown", 12)] {
        users["shared"][key] = json!(value);
    }
    assert_eq!(
        paging(&users),
        strip("Places 1 to 12 of 20", false, "12", true)
    );
    // The state behind it: each step further remembers where it began,
    // so that the way back shows the pictures as they were, and
    // nothing stays selected.
    let mut state = fresh("");
    state["path"] = json!(["lib/a.php", "lib/db.php"]);
    state["shared"] = json!(true);
    state["focus"] = json!("lib/core.php");
    assert_eq!(state["skip"], json!([]));
    let second = paged(state.clone(), Some(12));
    assert_eq!(
        (&second["skip"], &second["focus"]),
        (&json!([12]), &json!(""))
    );
    let third = paged(second.clone(), Some(21));
    assert_eq!(third["skip"], json!([12, 21]));
    // A step that leads no further is none.
    assert_eq!(paged(third.clone(), Some(21))["skip"], json!([12, 21]));
    assert_eq!(paged(third.clone(), None)["skip"], json!([12]));
    assert_eq!(paged(paged(third.clone(), None), None)["skip"], json!([]));
    assert_eq!(paged(state.clone(), None)["skip"], json!([]));
    // The server begins where it can: the way back keeps the pictures
    // that still lie before that.
    assert_eq!(begins(third.clone(), 21)["skip"], json!([12, 21]));
    assert_eq!(begins(third.clone(), 17)["skip"], json!([12, 17]));
    assert_eq!(begins(third.clone(), 12)["skip"], json!([12]));
    assert_eq!(begins(third.clone(), 5)["skip"], json!([5]));
    assert_eq!(begins(third.clone(), 0)["skip"], json!([]));
    assert_eq!(begins(state.clone(), 7)["skip"], json!([7]));
    assert_eq!(begins(third.clone(), 21)["focus"], "");
    // The other kind, and the path between the two, begin anew; the
    // filter holds for both kinds and goes with them.
    let mut far = third;
    far["filter"] = json!("core");
    let other = sharing(far.clone(), &json!("uses_both")).unwrap();
    assert_eq!(
        (&other["skip"], &other["filter"], &other["shared"]),
        (&json!([]), &json!("core"), &json!(true))
    );
    let path = sharing(far.clone(), &json!(false)).unwrap();
    assert_eq!(
        (&path["skip"], &path["filter"], &path["shared"]),
        (&json!([]), &json!(""), &json!(false))
    );
    assert_eq!(unfollowed(far)["skip"], json!([]));
}
#[test]
fn two_places_are_asked_what_they_share_and_for_the_path_again() {
    // No two places are shown: there is nothing to ask.
    let state = fresh("pr:7");
    assert_eq!(
        (&state["shared"], &state["side"]),
        (&json!(false), &json!(""))
    );
    assert_eq!(sharing(state.clone(), &json!(true)), None);
    // The two ends of a path, with a box of it selected: the same two
    // are asked what they share, in the map that is open, and nothing
    // is selected in the new picture. Which kind is drawn is left to
    // what there is.
    let mut pathed = state;
    for (key, value) in [
        ("scope", json!("lib")),
        ("path", json!(["lib/a<b>.php", "lib/db.php"])),
        ("focus", json!("lib/core.php")),
        ("spread", json!(["src"])),
    ] {
        pathed[key] = value;
    }
    let kept = |state: &V, keys: &[&str]| {
        keys.iter()
            .map(|key| state[*key].clone())
            .collect::<Vec<_>>()
    };
    let ends = json!(["lib/a<b>.php", "lib/db.php"]);
    let shown = sharing(pathed.clone(), &json!(true)).unwrap();
    assert_eq!(
        kept(&shown, &["shared", "side", "path", "focus"]),
        [json!(true), json!(""), ends.clone(), json!("")]
    );
    assert_eq!(
        kept(&shown, &["target", "scope", "spread", "pick"]),
        kept(&pathed, &["target", "scope", "spread", "pick"])
    );
    // One kind is asked for by its name, from the path or from the
    // other kind, and nothing stays selected.
    let mut chosen = shown.clone();
    chosen["focus"] = json!("lib/core.php");
    for side in ["uses_both", "both_use"] {
        for from in [&pathed, &chosen] {
            assert_eq!(
                kept(
                    &sharing(from.clone(), &json!(side)).unwrap(),
                    &["shared", "side", "path", "focus"]
                ),
                [json!(true), json!(side), ends.clone(), json!("")],
                "{side}"
            );
        }
    }
    // Anything else asks for the path between them again.
    let users = sharing(chosen, &json!("uses_both")).unwrap();
    for asked in [json!(false), V::Null, json!("elsewhere")] {
        assert_eq!(
            kept(
                &sharing(users.clone(), &asked).unwrap(),
                &["shared", "side", "path", "focus"]
            ),
            [json!(false), json!(""), ends.clone(), json!("")],
            "{asked}"
        );
    }
    // Leaving the two for a folder or a file leaves what they share.
    let left = unfollowed(users.clone());
    assert_eq!(
        kept(&left, &["shared", "side", "path"]),
        [json!(false), json!(""), json!([])]
    );
    // A path asked from a box of what they share is a path.
    let mut asked = users;
    asked["pick"] = json!({"path": "lib/core.php", "name": "core.php"});
    let other = led(asked, "lib/db.php").unwrap();
    assert_eq!(
        kept(&other, &["shared", "side", "path"]),
        [
            json!(false),
            json!(""),
            json!(["lib/core.php", "lib/db.php"])
        ]
    );
}
#[test]
fn what_a_search_typed_is_marked_in_the_names_it_found() {
    // Each word once, whatever its case, and nothing else.
    assert_eq!(marked("Cart.php", "cart", None), "<mark>Cart</mark>.php");
    assert_eq!(marked("Cart.php", "zzz", None), "Cart.php");
    assert_eq!(marked("Cart.php", "", None), "Cart.php");
    assert_eq!(
        marked("format_price()", "PRICE for", None),
        "<mark>for</mark>mat_<mark>price</mark>()"
    );
    // Words that share letters are marked together.
    assert_eq!(
        marked("format_price()", "format mat", None),
        "<mark>format</mark>_price()"
    );
    assert_eq!(
        marked("abcdef", "abc cde", None),
        "<mark>abc</mark><mark>de</mark>f"
    );
    // A method is shown with its class and found by its own name: the
    // word is marked there, though the class holds it too.
    assert_eq!(
        marked("CartTest.testTotal()", "test total", Some(9)),
        "CartTest.<mark>test</mark><mark>Total</mark>()"
    );
    assert_eq!(
        marked("Cart.total()", "cart.to", Some(5)),
        "<mark>Cart.to</mark>tal()"
    );
    assert_eq!(
        marked("Cart.total()", "cart total", Some(5)),
        "<mark>Cart</mark>.<mark>total</mark>()"
    );
    // A word with a slash names a folder or a file by the start of its
    // name, and says of a symbol only where its file lies.
    assert_eq!(
        marked("Price.php", "cart/pri", None),
        "<mark>Pri</mark>ce.php"
    );
    assert_eq!(marked("Price.php", "cart/", None), "Price.php");
    assert_eq!(marked("price()", "cart/pri", Some(0)), "price()");
    // What a name holds is written as text.
    assert_eq!(marked("a<b>&c", "b", None), "a&lt;<mark>b</mark>&gt;&amp;c");
    // Letters of more than one byte are marked whole, and a name whose
    // lower case is longer than itself is left unmarked.
    assert_eq!(marked("Жук.php", "жу", None), "<mark>Жу</mark>к.php");
    assert_eq!(marked("İx", "x", None), "İx");
    assert_eq!(marked("éa", "a", Some(1)), "é<mark>a</mark>");
}
fn searched() -> V {
    json!({
        "open": true, "query": "cart", "asked": "cart", "for": "/x", "active": 1, "found": 30,
        "loading": false, "failed": null, "missing": false, "notests": false,
        "rows": [
            {"path": "src/Cart", "name": "Cart", "what": "folder", "in": "src", "files": 2},
            {"path": "Cart<T>.php", "name": "Cart<T>.php", "what": "file", "in": "", "symbols": 9},
            {"path": "src/Cart/Cart.php#L12:Cart", "name": "Cart", "own": 0, "what": "type", "file": "src/Cart/Cart.php", "line": 12},
            {"path": "src/a.md#Cart", "name": "Cart", "own": 0, "what": "heading", "file": "src/a.md", "line": 0}
        ]
    })
}
#[test]
fn what_a_search_found_is_listed_with_what_it_is_and_where_it_lies() {
    let html = found(&searched());
    assert_eq!(
        html.matches("<li class=\"graph-pick-row graph-find-row\"")
            .count(),
        4
    );
    // A folder: its name with a slash, and the folder it lies in.
    assert!(html.contains("<li class=\"graph-pick-row graph-find-row\" id=\"graph-find-row-0\" role=\"option\" aria-selected=\"false\" data-action=\"graph-find-choose\" data-payload=\"0\" title=\"src/Cart\"><span class=\"graph-find-what\">folder</span><span class=\"graph-pick-main\"><span class=\"graph-pick-title\"><mark>Cart</mark>/</span></span><span class=\"graph-find-where\"><bdi>src</bdi></span></li>"));
    // The marked row, a file in the project itself; names are text.
    assert!(html.contains("id=\"graph-find-row-1\" role=\"option\" aria-selected=\"true\" data-action=\"graph-find-choose\" data-payload=\"1\" title=\"Cart&lt;T&gt;.php\"><span class=\"graph-find-what\">file</span><span class=\"graph-pick-main\"><span class=\"graph-pick-title\"><mark>Cart</mark>&lt;T&gt;.php</span></span><span class=\"graph-find-where\"><bdi></bdi></span>"));
    // A name: its file and its line, where the graph tells one.
    assert!(html.contains("<span class=\"graph-find-what\">type</span><span class=\"graph-pick-main\"><span class=\"graph-pick-title\"><mark>Cart</mark></span></span><span class=\"graph-find-where\"><bdi>src/Cart/Cart.php:12</bdi></span>"));
    assert!(html.contains("<span class=\"graph-find-what\">heading</span>"));
    assert!(html.contains("<bdi>src/a.md</bdi>"));
    assert_eq!(found(&json!({"rows": []})), "");
    // The line under them.
    let mut finder = searched();
    assert_eq!(
        found_note(&finder),
        "The 4 best of 30 found; type more to narrow them. ↑ ↓ to move · Enter to go there · Esc to close"
    );
    finder["found"] = json!(1234);
    assert_eq!(
        found_note(&finder),
        "The 4 best of 1,234 found; type more to narrow them. ↑ ↓ to move · Enter to go there · Esc to close"
    );
    finder["found"] = json!(4);
    finder["notests"] = json!(true);
    assert_eq!(
        found_note(&finder),
        "4 found. Tests are left out. ↑ ↓ to move · Enter to go there · Esc to close"
    );
    // While the server is asked what was found before stays listed.
    finder["loading"] = json!(true);
    assert!(found_note(&finder).starts_with("4 found."));
    finder["rows"] = json!([]);
    assert_eq!(found_note(&finder), "Searching…");
    finder["loading"] = json!(false);
    finder["found"] = json!(0);
    assert_eq!(
        found_note(&finder),
        "Nothing in the graph is called that. Tests are left out."
    );
    finder["notests"] = json!(false);
    assert_eq!(found_note(&finder), "Nothing in the graph is called that.");
    finder["missing"] = json!(true);
    assert_eq!(found_note(&finder), "There is no graph to search yet.");
    finder["failed"] = json!("This project has no published code graph.");
    assert_eq!(
        found_note(&finder),
        "This project has no published code graph."
    );
}
#[test]
fn the_map_goes_to_the_place_a_search_found() {
    let rows = m::array(&searched()["rows"]);
    // A map of a change with links followed and a folder spread out.
    let mut state = fresh("pr:7");
    for (key, value) in [
        ("scope", json!("lib")),
        ("focus", json!("lib/a.php")),
        ("filter", json!("a")),
        ("follow", json!("lib/a.php")),
        ("out", json!(["lib/a.php"])),
        ("into", json!(["lib/a.php"])),
        (
            "spread",
            json!(["src", "src/Cart", "src/Cart/Rules", "lib"]),
        ),
        ("certain", json!(true)),
    ] {
        state[key] = value;
    }
    let kept = |state: &V, keys: &[&str]| {
        keys.iter()
            .map(|key| state[*key].clone())
            .collect::<Vec<_>>()
    };
    // A name opens its file on itself, in the map that is open.
    let name = sought(state.clone(), &rows[2], false);
    assert_eq!(
        kept(
            &name,
            &["target", "scope", "focus", "filter", "follow", "out", "into", "certain"]
        ),
        [
            json!("pr:7"),
            json!("src/Cart/Cart.php"),
            json!("src/Cart/Cart.php#L12:Cart"),
            json!(""),
            json!(""),
            json!([]),
            json!([]),
            json!(true)
        ]
    );
    assert_eq!(name["spread"], state["spread"]);
    // So it does where a picture happens to draw a box of that name.
    assert_eq!(sought(state.clone(), &rows[2], true), name);
    // A folder the picture draws is selected where it is, and nothing
    // else changes.
    let here = sought(state.clone(), &rows[0], true);
    let mut selected = state.clone();
    selected["focus"] = json!("src/Cart");
    assert_eq!(here, selected);
    // One it does not draw is shown in the folder it lies in, in the
    // whole project, as one box: what was spread out of it and in it is
    // gathered.
    let there = sought(state.clone(), &rows[0], false);
    assert_eq!(
        kept(
            &there,
            &["target", "scope", "focus", "filter", "follow", "out", "into", "certain"]
        ),
        [
            json!(""),
            json!("src"),
            json!("src/Cart"),
            json!(""),
            json!(""),
            json!([]),
            json!([]),
            json!(true)
        ]
    );
    assert_eq!(there["spread"], json!(["src", "lib"]));
    // A file in the project itself is shown in the project, and no
    // folder is gathered for a file.
    let top = sought(state.clone(), &rows[1], false);
    assert_eq!(
        kept(&top, &["target", "scope", "focus"]),
        [json!(""), json!(""), json!("Cart<T>.php")]
    );
    assert_eq!(top["spread"], state["spread"]);
}
/// One found pattern as the server draws it: a strategy with a share of
/// its implementations and its one user.
fn strategy() -> V {
    let part = |name: &str, role: &str, frame: u64, start: bool| {
        json!({
            "path": format!("app/Pay/{name}.php#L3:{name}"), "name": name, "kind": "type",
            "line": 3, "file": format!("app/Pay/{name}.php"), "symbol": true, "folder": false,
            "files": 1, "out": u64::from(!start), "in": if start { 3 } else { 0 },
            "role": role, "start": start, "holder": format!("role:{frame}"),
        })
    };
    let key = |name: &str| format!("app/Pay/{name}.php#L3:{name}");
    json!({
        "mode": "pattern", "scope": "app/Pay/Gateway.php", "certain": false,
        "crumbs": [
            {"name": "shop", "path": ""}, {"name": "app", "path": "app"},
            {"name": "Strategy: Gateway", "path": ""}
        ],
        "pattern": {
            "id": "strategy@app/Pay/Gateway.php#L3:Gateway", "sort": "strategy",
            "name": "Strategy", "family": "design",
            "tells": "An abstraction with several implementations.",
            "named": false, "guessed": true,
            "of": {"path": key("Gateway"), "name": "Gateway", "what": "type", "symbol": true},
            "role": "abstraction",
            "told": ["12 implementations stand under Gateway."],
            "parts": [
                {"one": "implementation", "many": "implementations", "found": 12, "drawn": 2},
                {"one": "user", "many": "users", "found": 1, "drawn": 1}
            ],
        },
        "frames": [
            {"path": "role:0", "name": "Abstraction", "holder": null, "places": 1, "drawn": 1, "fixed": true},
            {"path": "role:1", "name": "Implementations", "holder": null, "places": 12, "drawn": 2, "fixed": true},
            {"path": "role:2", "name": "User", "holder": null, "places": 1, "drawn": 1, "fixed": true}
        ],
        "nodes": [
            part("Gateway", "abstraction", 0, true),
            part("Stripe", "implementation", 1, false),
            part("Pay<b>", "implementation", 1, false),
            part("Checkout", "user", 2, false)
        ],
        "edges": [
            {"from": key("Stripe"), "to": key("Gateway"), "links": 1, "guessed": 0, "tie": "implements"},
            {"from": key("Pay<b>"), "to": key("Gateway"), "links": 1, "guessed": 1, "tie": "implements"},
            {"from": key("Checkout"), "to": key("Gateway"), "links": 1, "guessed": 0, "tie": "holds and takes"}
        ],
        "focus": null,
    })
}
#[test]
fn a_found_pattern_is_drawn_part_by_part_in_the_frame_of_what_each_is() {
    let view = strategy();
    assert_eq!(
        summary(&view),
        "Strategy: Gateway · 12 implementations, 2 drawn · 1 user"
    );
    assert_eq!(
        note(&view),
        "An abstraction with several implementations. 12 implementations stand under Gateway. A hollow link is one the tool only guessed."
    );
    let html = canvas(&view);
    // A frame is named for what its parts are, says how many of them
    // are drawn where that is not all, and is no folder to close.
    assert!(
        html.contains("title=\"Abstraction\">Abstraction</span>"),
        "{html}"
    );
    assert!(html.contains(
        ">Implementations<span class=\"graph-map-frame-count\"> · 2 of 12 drawn</span></span>"
    ));
    assert!(html.contains("title=\"User\">User</span>"));
    assert!(!html.contains("graph-map-gather") && !html.contains("graph-map-spread"));
    assert_eq!(html.matches("class=\"graph-map-frame\"").count(), 3);
    // A part is a symbol of its own file: it is selected, never opened.
    assert_eq!(html.matches("data-open=\"false\"").count(), 4);
    assert!(html.contains("data-names=\"true\""));
    assert!(html.contains("Gateway, type, line 3, uses others through 0 links, is used through 3 links, the pattern is named after it, abstraction in the pattern"));
    assert!(html.contains("Checkout, type, line 3, uses others through 1 link, is used through 0 links, user in the pattern"));
    assert_eq!(html.matches("data-start=\"true\"").count(), 1);
    // A link says how the one part stands to the other.
    assert!(html.contains("<title>Stripe implements Gateway</title>"));
    assert!(html.contains("<title>Pay&lt;b&gt; implements Gateway · guessed</title>"));
    assert!(html.contains("<title>Checkout holds and takes Gateway</title>"));
    assert_eq!(html.matches("graph-map-edge-guess").count(), 1);
    // The way there ends in the finding, which is no place to open.
    let way = crumbs(&view);
    assert!(
        way.ends_with(
            "<span class=\"graph-map-crumb\" aria-current=\"location\">Strategy: Gateway</span>"
        ),
        "{way}"
    );
    assert_eq!(way.matches("data-action=\"graph-map-open\"").count(), 2);
    assert!(legend(&view, false).contains("A frame holds the parts that are one thing in the pattern; the ringed box is the type it is named after. T type"));
    // Beside it: what to do, or the selected part with what it is in
    // the pattern and what joins it to the others.
    assert!(side(&view).starts_with("<p class=\"knowledge-note\">A frame holds the parts"));
    let mut selected = view.clone();
    selected["focus"] = selected["nodes"][3].clone();
    selected["focus"]["inside"] = json!(true);
    selected["focus"]["uses"] = json!([{
        "path": "app/Pay/Gateway.php#L3:Gateway", "name": "Gateway", "kind": "type", "line": 3,
        "file": "app/Pay/Gateway.php", "symbol": true, "inside": true,
        "relation": "holds and takes", "role": "abstraction", "links": 1, "guessed": 0
    }]);
    selected["focus"]["used_by"] = json!([]);
    let html = side(&selected);
    assert!(html.starts_with("<h3>Checkout</h3><p class=\"graph-map-path\">app/Pay/Checkout.php:3</p><p class=\"knowledge-note\">Type · user in this pattern</p>"), "{html}");
    assert!(html.contains(">Open in its file</button>"));
    assert!(html.contains("<span class=\"graph-map-row-links\">holds and takes</span>"));
    assert!(html.contains("<h4>Used by <span class=\"graph-map-total\">0</span></h4>"));
    let html = canvas(&selected);
    assert_eq!(html.matches("data-selected=\"true\"").count(), 1);
    assert_eq!(html.matches("data-dim=\"true\"").count(), 2);
    assert_eq!(html.matches("aria-pressed=\"true\"").count(), 1);
    assert_eq!(html.matches("aria-pressed=\"false\"").count(), 3);
}
/// A folder of the map as the server marks it where patterns are asked
/// for.
fn marked_folder() -> V {
    let finding = |sort: &str, name: &str, of: &str, named: bool, guessed: bool| {
        json!({
            "id": format!("{sort}@app/Pay/{of}.php#L3:{of}"), "sort": sort, "name": name, "of": of,
            "file": format!("app/Pay/{of}.php"), "line": 3, "named": named, "guessed": guessed,
            "parts": "2 implementations · 1 user",
        })
    };
    json!({
        "mode": "map", "scope": "app", "certain": false,
        "crumbs": [{"name": "shop", "path": ""}, {"name": "app", "path": "app"}],
        "nodes": [
            {"path": "app/Pay", "name": "Pay", "folder": true, "files": 9, "inside": true, "out": 0, "in": 0, "patterns": 1200},
            {"path": "app/Kernel.php", "name": "Kernel.php", "folder": false, "files": 1, "symbols": 2, "inside": true, "out": 0, "in": 0, "patterns": 1},
            {"path": "app/Zero", "name": "Zero", "folder": true, "files": 3, "inside": true, "out": 0, "in": 0}
        ],
        "edges": [],
        "focus": null,
        "patterns": {
            "found": 1203,
            "kinds": [
                {
                    "id": "repository", "name": "Repository", "family": "architecture",
                    "tells": "A type named a <repository>.", "found": 1, "more": 0,
                    "instances": [finding("repository", "Repository", "Orders<T>", true, false)],
                },
                {
                    "id": "strategy", "name": "Strategy", "family": "design",
                    "tells": "An abstraction with several implementations.", "found": 1202, "more": 1197,
                    "instances": [
                        finding("strategy", "Strategy", "Gateway", false, true),
                        finding("strategy", "Strategy", "Clock", false, false),
                        finding("strategy", "Strategy", "Mailer", false, false),
                        finding("strategy", "Strategy", "Store", false, false),
                        finding("strategy", "Strategy", "Rule", false, false)
                    ],
                }
            ],
        },
    })
}
#[test]
fn a_picture_marks_what_holds_a_part_of_a_found_pattern_and_lists_the_findings() {
    let view = marked_folder();
    let html = canvas(&view);
    // One finding is a mark, more of them a mark with how many.
    assert!(
        html.contains("<span class=\"graph-map-node-pattern\" aria-hidden=\"true\">◆1,200</span>"),
        "{html}"
    );
    assert!(html.contains("<span class=\"graph-map-node-pattern\" aria-hidden=\"true\">◆</span>"));
    assert_eq!(html.matches("graph-map-node-pattern").count(), 2);
    assert!(html.contains("Pay, folder, 9 files, uses others through 0 links, is used through 0 links, holds parts of 1,200 found patterns"));
    assert!(html.contains("Kernel.php, file, uses others through 0 links, is used through 0 links, holds parts of 1 found pattern"));
    assert!(
        legend(&view, false).ends_with("◆: a part of a found pattern, or a place that holds one.")
    );
    // A picture that was not asked for patterns says nothing of them.
    let mut plain = view.clone();
    plain.as_object_mut().unwrap().remove("patterns");
    assert_eq!(found_patterns(&plain, &[]), "");
    assert!(!legend(&plain, false).contains('◆'));
    // With nothing selected: every pattern found here, architecture
    // first, each with its first findings.
    let list = found_patterns(&view, &[]);
    assert!(
        list.starts_with(
            "<h4>Patterns found here <span class=\"graph-map-total\">1,203</span></h4>"
        ),
        "{list}"
    );
    let (architecture, design) = (
        list.find("<p class=\"graph-map-family\">Architecture</p>")
            .unwrap(),
        list.find("<p class=\"graph-map-family\">Design</p>")
            .unwrap(),
    );
    assert!(architecture < design);
    assert!(list.contains("<p class=\"graph-map-pattern-name\" title=\"A type named a &lt;repository&gt;.\">Repository <span class=\"graph-map-total\">1</span></p>"));
    assert!(list.contains("Strategy <span class=\"graph-map-total\">1,202</span></p>"));
    assert_eq!(
        list.matches("data-action=\"graph-map-pattern\"").count(),
        1 + FIRST
    );
    // A finding says where it lies, what it has, and how far it is
    // believed.
    assert!(list.contains("data-payload=\"&quot;repository@app/Pay/Orders&lt;T&gt;.php#L3:Orders&lt;T&gt;&quot;\" title=\"app/Pay/Orders&lt;T&gt;.php:3\"><span class=\"graph-map-row-name\">Orders&lt;T&gt;<span class=\"graph-map-row-file\">Orders&lt;T&gt;.php</span></span><span class=\"graph-map-row-links\">2 implementations · 1 user · by name</span>"), "{list}");
    assert!(list.contains(">Gateway<span class=\"graph-map-row-file\">Gateway.php</span></span><span class=\"graph-map-row-links\">2 implementations · 1 user · guessed</span>"));
    assert!(!list.contains(">Store<") && !list.contains(">Rule<"));
    // More than the first few are asked for, pattern by pattern.
    assert!(list.contains("data-action=\"graph-map-pattern-kind\" data-payload=\"&quot;strategy&quot;\" aria-expanded=\"false\">All 5</button>"));
    assert_eq!(list.matches("graph-map-pattern-kind").count(), 1);
    let all = found_patterns(&view, &["strategy".to_owned()]);
    assert_eq!(all.matches("data-action=\"graph-map-pattern\"").count(), 6);
    assert!(all.contains("aria-expanded=\"true\">First 3</button><p class=\"knowledge-note\">and 1,197 more: open a folder to see those of it.</p>"));
    // Beside a selected place: the findings it has a part in.
    let mut selected = view.clone();
    selected["focus"] = json!({"path": "app/Pay", "patterns": [
        view["patterns"]["kinds"][1]["instances"][0], view["patterns"]["kinds"][0]["instances"][0]
    ]});
    let own = found_patterns(&selected, &[]);
    assert!(own.starts_with("<h4>Patterns <span class=\"graph-map-total\">2</span></h4><ul class=\"graph-map-rows\">"), "{own}");
    assert!(own.contains("<span class=\"graph-map-row-name\">Strategy: Gateway<span"));
    assert!(own.contains("<span class=\"graph-map-row-name\">Repository: Orders&lt;T&gt;<span"));
    assert!(!own.contains("graph-map-family"));
    selected["focus"]["patterns"] = json!([]);
    assert_eq!(
        found_patterns(&selected, &[]),
        "<h4>Patterns <span class=\"graph-map-total\">0</span></h4><p class=\"knowledge-note\">No found pattern has a part here.</p>"
    );
    // Beside followed links, a path, what two places share or a cycle
    // only a selected place says what it is part of.
    for mode in ["follow", "path", "shared", "cycle"] {
        let mut other = view.clone();
        other["mode"] = json!(mode);
        assert_eq!(found_patterns(&other, &[]), "", "{mode}");
        other["focus"] = selected["focus"].clone();
        other["focus"]["patterns"] = json!([view["patterns"]["kinds"][0]["instances"][0]]);
        assert!(
            found_patterns(&other, &[])
                .starts_with("<h4>Patterns <span class=\"graph-map-total\">1</span></h4>"),
            "{mode}"
        );
    }
    // Where none was found, what a pattern is told by.
    let mut none = view.clone();
    none["patterns"] = json!({"found": 0, "kinds": []});
    assert!(found_patterns(&none, &[]).ends_with("and a few by a name.</p>"));
}
#[test]
fn the_map_opens_a_found_pattern_and_returns_to_where_it_was() {
    let mut map = fresh("");
    for (key, value) in [
        ("scope", json!("app")),
        ("focus", json!("app/Pay")),
        ("filter", json!("pay")),
        ("patterns", json!(true)),
        ("spread", json!(["app/Pay"])),
    ] {
        map[key] = value;
    }
    let shown = showing_pattern(map.clone(), "strategy@app/Pay/Gateway.php#L3:Gateway").unwrap();
    assert_eq!(shown["pattern"], "strategy@app/Pay/Gateway.php#L3:Gateway");
    assert_eq!(shown["pattern_return"], map);
    assert_eq!(
        (shown["focus"].as_str(), shown["filter"].as_str()),
        (Some(""), Some(""))
    );
    assert_eq!(shown["patterns"], true);
    // Another finding opened from this picture returns to the same map.
    let mut inside = shown.clone();
    inside["focus"] = json!("app/Pay/Stripe.php#L3:Stripe");
    let next = showing_pattern(inside, "decorator@app/Pay/Log.php#L3:Log").unwrap();
    assert_eq!(next["pattern"], "decorator@app/Pay/Log.php#L3:Log");
    assert_eq!(next["pattern_return"], map);
    assert_eq!(next["focus"], "");
    // Nothing is opened while the other end of a path is being chosen.
    let mut picking = map.clone();
    picking["pick"] = json!({"path": "app/Pay", "name": "Pay"});
    assert!(showing_pattern(picking, "strategy@x").is_none());
    assert!(showing_pattern(map.clone(), "").is_none());
    // The way back keeps what was switched in the picture.
    let mut switched = next.clone();
    switched["notests"] = json!(true);
    switched["certain"] = json!(true);
    switched["kinds"] = json!(["strategy"]);
    let back = left_pattern(switched).unwrap();
    let mut expected = map.clone();
    expected["notests"] = json!(true);
    expected["certain"] = json!(true);
    expected["kinds"] = json!(["strategy"]);
    assert_eq!(back, expected);
    assert!(left_pattern(map.clone()).is_none());
    // Whatever leaves a picture leaves the pattern: a path from one of
    // its parts, a followed place, an opened folder.
    let mut from = shown.clone();
    from["pick"] = json!({"path": "app/Pay/Stripe.php#L3:Stripe", "name": "Stripe"});
    let path = led(from, "app/Kernel.php").unwrap();
    assert!(path["pattern"].is_null() && path["pattern_return"].is_null());
    let left = unfollowed(shown);
    assert!(left["pattern"].is_null() && left["pattern_return"].is_null());
    assert_eq!(left["patterns"], true);
}
#[test]
fn layers_stand_in_a_row_in_their_order_and_a_link_back_runs_under_it() {
    // Entry points, services, data access and the model, with a link
    // that passes a layer and two that run back.
    let edges = [
        (0, 1, 3),
        (1, 2, 2),
        (1, 3, 3),
        (2, 0, 1),
        (2, 3, 2),
        (3, 1, 2),
    ];
    let plan = plan_row(4, &edges);
    assert_eq!(
        plan.nodes
            .iter()
            .map(|node| (node.x, node.y, node.layer, node.cycle))
            .collect::<Vec<_>>(),
        [
            (12.0, 30.0, Some(0), false),
            (248.0, 30.0, Some(1), false),
            (484.0, 30.0, Some(2), false),
            (720.0, 30.0, Some(3), false),
        ]
    );
    assert_eq!(
        plan.wires
            .iter()
            .map(|wire| (wire.edge, wire.path.as_str(), wire.back))
            .collect::<Vec<_>>(),
        [
            // To the next layer: straight, from side to side.
            (0, "M176.0 45.0 L248.0 45.0", false),
            (1, "M412.0 45.0 L484.0 45.0", false),
            // Past a layer: over the row.
            (2, "M371.0 30.0 C371.0 6.0 761.0 6.0 761.0 30.0", false),
            // Back: under the row, the second arc deeper than the first.
            (3, "M525.0 60.0 C525.0 84.0 135.0 84.0 135.0 60.0", true),
            (4, "M648.0 45.0 L720.0 45.0", false),
            (5, "M761.0 60.0 C761.0 100.0 371.0 100.0 371.0 60.0", true),
        ]
    );
    assert_eq!((plan.width, plan.height), (896.0, 102.0));
    assert!(plan.frames.is_empty() && plan.apart.is_none());
    // Arcs that share an edge of a box leave it and reach it apart, the
    // longer one outside the shorter.
    let nested = plan_row(4, &[(0, 2, 1), (0, 3, 1), (1, 3, 1), (3, 0, 1), (2, 0, 1)]);
    let paths = nested
        .wires
        .iter()
        .map(|wire| wire.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        [
            "M135.0 54.0 C135.0 30.0 525.0 30.0 525.0 54.0",
            "M125.0 54.0 C125.0 -2.0 771.0 -2.0 771.0 54.0",
            "M371.0 54.0 C371.0 14.0 761.0 14.0 761.0 54.0",
            "M761.0 84.0 C761.0 124.0 125.0 124.0 125.0 84.0",
            "M525.0 84.0 C525.0 108.0 135.0 108.0 135.0 84.0",
        ]
    );
    assert_eq!(nested.height, 84.0 + 30.0 + 12.0);
    // A row without links, a link named twice, one to itself and one to
    // what is not there.
    let plain = plan_row(2, &[(0, 0, 1), (0, 1, 1), (0, 1, 4), (1, 5, 1)]);
    assert_eq!(plain.wires.len(), 1);
    assert_eq!(plain.wires[0].edge, 1);
    assert_eq!((plain.nodes[0].y, plain.height), (12.0, 54.0));
    let none = plan_row(0, &[]);
    assert!(none.nodes.is_empty() && none.wires.is_empty());
    assert_eq!((none.width, none.height), (188.0, 54.0));
}
/// The layers of a folder as the server draws them: four layers, two
/// links against their order, one of them guessed.
fn layers() -> V {
    let layer = |id: &str, name: &str, files: u64, out: u64, into: u64| {
        json!({
            "path": format!("layer:{id}"), "name": name, "folder": true, "layer": true,
            "files": files, "symbols": 0, "within": 0, "changed": 0, "out": out, "in": into,
        })
    };
    let end = |path: &str, layer: &str| json!({"path": path, "name": base(path), "layer": layer});
    let link = |from: &str, to: &str, links: u64, guessed: u64, against: bool| {
        json!({
            "from": format!("layer:{from}"), "to": format!("layer:{to}"),
            "links": links, "guessed": guessed, "against": against,
        })
    };
    json!({
        "mode": "pattern", "scope": "app", "certain": false,
        "crumbs": [
            {"name": "shop", "path": ""}, {"name": "app", "path": "app"},
            {"name": "Layers", "path": ""}
        ],
        "pattern": {
            "id": "layers@app", "sort": "layers", "name": "Layers", "family": "architecture",
            "tells": "Files filed in layers by their names.",
            "named": true, "guessed": true,
            "of": {"path": "app", "name": "app", "what": "folder"},
            "told": ["1 file of code here is filed in no layer."],
            "layers": [
                {"id": "entry", "name": "Entry points", "files": 2},
                {"id": "services", "name": "Services", "files": 2},
                {"id": "data", "name": "Data access", "files": 2},
                {"id": "model", "name": "Model", "files": 1200}
            ],
            "apart": 1,
            "against": {
                "links": 3, "guessed": 1, "pairs": 2, "more": 0,
                "rows": [
                    {
                        "from": end("app/Models/Order.php", "Model"),
                        "to": end("app/Services/Pay<b>.php", "Services"),
                        "links": 2, "guessed": 0
                    },
                    {
                        "from": end("app/Repositories/OrderRepository.php", "Data access"),
                        "to": end("app/Http/Controllers/CheckoutController.php", "Entry points"),
                        "links": 1, "guessed": 1
                    }
                ],
            },
        },
        "nodes": [
            layer("entry", "Entry points", 2, 3, 1),
            layer("services", "Services", 2, 5, 5),
            layer("data", "Data access", 2, 3, 2),
            layer("model", "Model", 1200, 2, 5)
        ],
        "edges": [
            link("entry", "services", 3, 0, false),
            link("services", "data", 2, 0, false),
            link("services", "model", 3, 0, false),
            link("data", "entry", 1, 1, true),
            link("data", "model", 2, 0, false),
            link("model", "services", 2, 0, true)
        ],
        "focus": null,
    })
}
#[test]
fn the_layers_of_a_folder_are_drawn_in_their_order_with_what_runs_against_it() {
    let view = layers();
    assert_eq!(
        summary(&view),
        "Layers: app · 4 layers · 1,206 files · 3 links against their order"
    );
    assert_eq!(
        note(&view),
        "Files filed in layers by their names. 1 file of code here is filed in no layer. A hollow link is one the tool only guessed."
    );
    let html = canvas(&view);
    // One row, in the order of the layers, whatever the links say.
    assert!(
        html.contains("style=\"width:896px;height:102px\""),
        "{html}"
    );
    for at in [
        "left:12px;top:30px",
        "left:248px;top:30px",
        "left:484px;top:30px",
        "left:720px;top:30px",
    ] {
        assert!(html.contains(at), "{at}");
    }
    // A layer is no folder and no symbol: it is selected, never opened,
    // and is called by its name.
    assert!(html.contains("title=\"Entry points\" aria-pressed=\"false\" aria-label=\"Entry points, layer, 2 files, uses others through 3 links, is used through 1 link\""), "{html}");
    assert!(html.contains("aria-label=\"Model, layer, 1,200 files, uses others through 2 links, is used through 5 links\""));
    assert_eq!(html.matches("data-open=\"false\"").count(), 4);
    assert!(!html.contains("data-names=\"true\""));
    assert!(html.contains("<span class=\"graph-map-node-count\" aria-hidden=\"true\">1,200</span>"));
    // A link back runs under the row, is marked, and says so; it is no
    // weaker direction of a cycle, and no layer is part of one.
    assert_eq!(html.matches(" graph-map-edge-against\"").count(), 2);
    assert_eq!(
        html.matches("marker-end=\"url(#graph-map-arrow-against)\"")
            .count(),
        2
    );
    assert_eq!(
        html.matches("marker-end=\"url(#graph-map-arrow)\"").count(),
        4
    );
    assert_eq!(
        html.matches("<marker id=\"graph-map-arrow-against\"")
            .count(),
        1
    );
    assert!(html.contains("d=\"M761.0 60.0 C761.0 100.0 371.0 100.0 371.0 60.0\""));
    assert!(html.contains(
        "<title>Model uses Services · 2 links · against the order of the layers</title>"
    ));
    assert!(html.contains("<title>Data access uses Entry points · 1 link guessed · against the order of the layers</title>"));
    assert!(html.contains("<title>Entry points uses Services · 3 links</title>"));
    assert!(html.contains("<title>Services uses Model · 3 links</title>"));
    assert!(!html.contains("graph-map-edge-back") && !html.contains("closes a cycle"));
    assert!(!html.contains("graph-map-node-cycle") && !html.contains("part of a cycle"));
    // One guessed link is dashed, against the order or not.
    assert_eq!(html.matches(" graph-map-edge-guess").count(), 1);
    // A link between two layers is many links of many files: it is
    // selected like any link, and says what selecting it shows.
    assert_eq!(html.matches("<path class=\"graph-map-hit\"").count(), 6);
    assert!(html.contains("<path class=\"graph-map-hit\" data-ends=\"3 1\" d=\"M761.0 60.0 C761.0 100.0 371.0 100.0 371.0 60.0\" data-action=\"graph-map-link\" data-payload=\"{&quot;from&quot;:&quot;layer:model&quot;,&quot;to&quot;:&quot;layer:services&quot;}\" role=\"button\" tabindex=\"0\" focusable=\"true\"><title>Model uses Services · select to see the pairs of files it is made of</title></path>"), "{html}");
    assert!(!html.contains("select to see what makes up this link"));
    // With nothing selected no link and no layer steps back.
    assert!(!html.contains("graph-map-edge-on") && !html.contains("graph-map-edge-off"));
    assert!(!html.contains("data-dim=\"true\""));
    // The way there ends in the layers, which are no place to open.
    let way = crumbs(&view);
    assert!(
        way.ends_with("<span class=\"graph-map-crumb\" aria-current=\"location\">Layers</span>"),
        "{way}"
    );
    assert_eq!(way.matches("data-action=\"graph-map-open\"").count(), 2);
    let key = legend(&view, false);
    assert!(
        key.ends_with("A link under the row, in red, runs back: against the order."),
        "{key}"
    );
    assert!(!key.contains("A frame holds") && !key.contains("T type"));
    // Beside it: what runs against the order, pair by pair, each a way
    // to what makes up the link of its two files.
    let html = side(&view);
    assert!(html.starts_with("<p class=\"knowledge-note\">A box is a layer: the files a name or a folder places in it."), "{html}");
    assert!(html.contains("<h4>Against the order <span class=\"graph-map-total\">2</span></h4>"));
    assert!(html.contains("data-pair=\"true\" data-action=\"graph-map-link\" data-payload=\"{&quot;from&quot;:&quot;app/Models/Order.php&quot;,&quot;to&quot;:&quot;app/Services/Pay&lt;b&gt;.php&quot;}\" title=\"app/Models/Order.php uses app/Services/Pay&lt;b&gt;.php · select to see what makes up this link\"><span class=\"graph-map-row-name\">Order.php → Pay&lt;b&gt;.php</span><span class=\"graph-map-row-links\">Model → Services · 2 links</span>"), "{html}");
    assert!(html.contains(
        "<span class=\"graph-map-row-links\">Data access → Entry points · 1 link guessed</span>"
    ));
    assert_eq!(html.matches("data-action=\"graph-map-link\"").count(), 2);
    let mut more = view.clone();
    more["pattern"]["against"]["more"] = json!(1200);
    let html = side(&more);
    assert!(
        html.contains("<h4>Against the order <span class=\"graph-map-total\">1,202</span></h4>")
    );
    assert!(html.ends_with("<p class=\"knowledge-note\">and 1,200 more</p>"));
    // A folder in order says so.
    let mut clean = view.clone();
    clean["pattern"]["against"] =
        json!({"links": 0, "guessed": 0, "pairs": 0, "rows": [], "more": 0});
    assert_eq!(
        summary(&clean),
        "Layers: app · 4 layers · 1,206 files · nothing against their order"
    );
    assert!(side(&clean).ends_with("<h4>Against the order <span class=\"graph-map-total\">0</span></h4><p class=\"knowledge-note\">No layer here needs one before it.</p>"));
    clean["pattern"]["against"]["links"] = json!(1);
    assert!(summary(&clean).ends_with("· 1 link against their order"));
}
#[test]
fn the_layers_of_a_change_say_what_it_set_against_their_order_and_what_it_took_away() {
    let end = |path: &str, layer: &str| json!({"path": path, "name": base(path), "layer": layer});
    let listed = |rows: V, links: u64, more: u64| {
        json!({
            "pairs": rows.as_array().unwrap().len() as u64 + more, "links": links,
            "guessed": 0, "rows": rows, "more": more,
        })
    };
    let plain = layers();
    let mut view = plain.clone();
    view["pattern"]["change"] = json!({
        "state": "ready", "base": {"name": "main", "commit": "0123456789ab"},
        "added": listed(json!([plain["pattern"]["against"]["rows"][0].clone()]), 2, 0),
        "removed": listed(
            json!([
                {
                    "from": end("app/Models/Cart<i>.php", "Model"),
                    "to": end("app/Http/Controllers/CartController.php", "Entry points"),
                    "links": 2, "guessed": 0
                },
                {
                    "from": end("app/Repositories/CartRepository.php", "Data access"),
                    "to": end("app/Services/CartService.php", "Services"),
                    "links": 1, "guessed": 1
                }
            ]),
            3,
            0,
        ),
    });
    view["pattern"]["against"]["rows"][0]["added"] = json!(true);
    view["edges"][5]["added"] = json!(2);
    // Above the picture: through how many links, as what stands
    // against the order is counted there.
    assert_eq!(
        summary(&view),
        "Layers: app · 4 layers · 1,206 files · 3 links against their order · this change adds 2 links and takes 3 away"
    );
    // Told, the comparison adds nothing to the note.
    assert_eq!(note(&view), note(&plain));
    // In the picture the link the change added to says so and carries
    // the number of those links, at the middle of its arc, over all the
    // links and under what is hit; no other link does.
    let html = canvas(&view);
    assert!(html.contains("<title>Model uses Services · 2 links · against the order of the layers · 2 links added by this change</title>"), "{html}");
    assert!(html.contains("<title>Data access uses Entry points · 1 link guessed · against the order of the layers</title>"));
    assert_eq!(html.matches("added by this change").count(), 1);
    // It bears the ends of its link, by which the link is lit.
    let mark =
        "<text class=\"graph-map-edge-mark\" data-ends=\"3 1\" x=\"566.0\" y=\"90.0\">+2</text>";
    assert_eq!(html.matches("<text ").count(), 1);
    let at = html.find(mark).unwrap();
    assert!(html.rfind("<path class=\"graph-map-edge").unwrap() < at);
    assert!(at < html.find("<path class=\"graph-map-hit\"").unwrap());
    assert_eq!(canvas(&plain).matches("graph-map-edge-mark").count(), 0);
    // The number is of a link against the order: on any other link the
    // word is not this picture's.
    let mut forward = plain.clone();
    forward["edges"][0]["added"] = json!(2);
    assert_eq!(canvas(&forward), canvas(&plain));
    // It steps back with its link when another is selected.
    let mut other = view.clone();
    other["focus"] = view["nodes"][0].clone();
    assert!(canvas(&other).contains(
        "<text class=\"graph-map-edge-mark graph-map-edge-off\" data-ends=\"3 1\" x=\"566.0\" y=\"90.0\">+2</text>"
    ));
    let mut one = view.clone();
    one["edges"][5]["added"] = json!(1200);
    let html = canvas(&one);
    assert!(html
        .contains("· against the order of the layers · 1,200 links added by this change</title>"));
    assert!(html.contains(">+1,200</text>"));
    one["edges"][5]["added"] = json!(1);
    assert!(canvas(&one).contains("· 1 link added by this change</title>"));
    // The key says what the number is, where there is one.
    assert!(legend(&view, true).ends_with(
        "A link under the row, in red, runs back: against the order. The number beside one counts the links this change added."
    ));
    assert!(legend(&plain, true).ends_with("runs back: against the order."));
    // Beside the picture what the change did comes first: the pairs it
    // set against the order, each a way to what makes up its link, and
    // those it took away, which are no links to look into any more.
    let html = side(&view);
    let (added, gone, all) = (
        html.find("<h4>Added by this change <span class=\"graph-map-total\">1</span></h4>")
            .unwrap(),
        html.find("<h4>Gone with this change <span class=\"graph-map-total\">2</span></h4>")
            .unwrap(),
        html.find("<h4>Against the order <span class=\"graph-map-total\">2</span></h4>")
            .unwrap(),
    );
    assert!(html.starts_with("<p class=\"knowledge-note\">A box is a layer:"));
    assert!(added < gone && gone < all, "{html}");
    let row = "title=\"app/Models/Order.php uses app/Services/Pay&lt;b&gt;.php · select to see what makes up this link\"><span class=\"graph-map-row-name\">Order.php → Pay&lt;b&gt;.php</span><span class=\"graph-map-row-links\">Model → Services · 2 links</span></button>";
    assert_eq!(html[added..gone].matches(row).count(), 1, "{html}");
    assert_eq!(
        html[added..gone]
            .matches("data-action=\"graph-map-link\"")
            .count(),
        1
    );
    assert!(html[gone..all].contains("<li><span class=\"graph-map-row\" data-pair=\"true\" data-gone=\"true\" title=\"app/Models/Cart&lt;i&gt;.php used app/Http/Controllers/CartController.php\"><span class=\"graph-map-row-name\">Cart&lt;i&gt;.php → CartController.php</span><span class=\"graph-map-row-links\">Model → Entry points · 2 links</span></span></li>"), "{html}");
    assert!(html[gone..all].contains("<span class=\"graph-map-row-links\">Data access → Services · 1 link guessed</span></span></li>"));
    assert_eq!(html[gone..all].matches("data-gone=\"true\"").count(), 2);
    assert!(!html[gone..all].contains("<button") && !html[gone..all].contains("data-action"));
    // Among all that runs against the order, what the change added
    // says so.
    assert!(html[all..].contains("<span class=\"graph-map-row-links\">Model → Services · 2 links · added by this change</span></button>"), "{html}");
    assert!(html[all..].contains(
        "<span class=\"graph-map-row-links\">Data access → Entry points · 1 link guessed</span></button>"
    ));
    assert_eq!(html.matches(" · added by this change").count(), 1);
    assert_eq!(html.matches("data-action=\"graph-map-link\"").count(), 3);
    assert!(!html[all..].contains("data-gone"));
    // More than are listed are counted.
    let mut many = view.clone();
    many["pattern"]["change"]["added"]["more"] = json!(40);
    many["pattern"]["change"]["removed"]["more"] = json!(1200);
    let html = side(&many);
    assert!(
        html.contains("<h4>Added by this change <span class=\"graph-map-total\">41</span></h4>")
    );
    assert!(html.contains("</ul><p class=\"knowledge-note\">and 40 more</p><h4>Gone with this change <span class=\"graph-map-total\">1,202</span></h4>"), "{html}");
    assert!(
        html.contains("</ul><p class=\"knowledge-note\">and 1,200 more</p><h4>Against the order ")
    );
    // A change that did nothing to the order says so, and one that
    // only took away or only added says that.
    let mut none = plain.clone();
    none["pattern"]["change"] = json!({
        "state": "ready", "base": {"name": "main", "commit": "0123456789ab"},
        "added": listed(json!([]), 0, 0), "removed": listed(json!([]), 0, 0),
    });
    assert!(summary(&none).ends_with("· 3 links against their order · this change adds none"));
    let html = side(&none);
    assert!(html.contains("Select it again to let go of it.</p><h4>Added by this change <span class=\"graph-map-total\">0</span></h4><p class=\"knowledge-note\">This change sets nothing against the order.</p><h4>Against the order <span class=\"graph-map-total\">2</span></h4>"), "{html}");
    assert!(!html.contains("Gone with this change"));
    assert_eq!(canvas(&none), canvas(&plain));
    assert_eq!(legend(&none, true), legend(&plain, true));
    let mut took = none.clone();
    took["pattern"]["change"]["removed"] = view["pattern"]["change"]["removed"].clone();
    assert!(summary(&took).ends_with("· this change takes 3 links away"));
    let html = side(&took);
    assert!(html.contains("<p class=\"knowledge-note\">This change sets nothing against the order.</p><h4>Gone with this change <span class=\"graph-map-total\">2</span></h4>"), "{html}");
    took["pattern"]["change"]["removed"]["links"] = json!(1);
    assert!(summary(&took).ends_with("· this change takes 1 link away"));
    let mut gave = view.clone();
    gave["pattern"]["change"]["removed"] = listed(json!([]), 0, 0);
    assert!(summary(&gave).ends_with("· this change adds 2 links"));
    assert!(!side(&gave).contains("Gone with this change"));
    gave["pattern"]["change"]["added"]["links"] = json!(1);
    assert!(summary(&gave).ends_with("· this change adds 1 link"));
    gave["pattern"]["change"]["removed"]["links"] = json!(1200);
    assert!(summary(&gave).ends_with("· this change adds 1 link and takes 1,200 away"));
    // With a layer or a link selected the panel is of what is
    // selected, and its pairs that the change added say so.
    let mut layer = view.clone();
    let mut focus = view["nodes"][3].clone();
    focus["signs"] = json!("a folder named models");
    focus["against"] = json!({"rows": [view["pattern"]["against"]["rows"][0].clone()], "more": 0});
    layer["focus"] = focus;
    let html = side(&layer);
    assert!(!html.contains("Added by this change") && !html.contains("Gone with this change"));
    assert!(html.contains(
        "<h4>Against the order, from here <span class=\"graph-map-total\">1</span></h4>"
    ));
    assert!(html.contains("<span class=\"graph-map-row-links\">Model → Services · 2 links · added by this change</span>"), "{html}");
    let mut link = view.clone();
    link["focus"] = json!({
        "path": "layer:model>layer:services", "link": true,
        "from": {"path": "layer:model", "name": "Model"},
        "to": {"path": "layer:services", "name": "Services"},
        "links": 3, "guessed": 1, "against": true, "pairs": 2, "more": 0,
        "rows": [
            {
                "from": end("app/Models/Order.php", "Model"),
                "to": end("app/Services/Pay<b>.php", "Services"),
                "links": 2, "guessed": 1, "added": true
            },
            {
                "from": end("app/Events/OrderPaid.php", "Model"),
                "to": end("app/Listeners/SendReceipt.php", "Services"),
                "links": 1, "guessed": 0
            }
        ],
    });
    let html = side(&link);
    assert!(!html.contains("Added by this change") && !html.contains("Gone with this change"));
    assert!(
        html.contains(
            "<span class=\"graph-map-row-links\">2 links, 1 guessed · added by this change</span>"
        ),
        "{html}"
    );
    assert!(html.contains("<span class=\"graph-map-row-links\">1 link</span>"));
}
#[test]
fn what_a_change_did_to_the_order_is_waited_for_or_said_to_be_untold() {
    let plain = layers();
    let told = |change: V| {
        let mut view = plain.clone();
        view["pattern"]["change"] = change;
        // Until the two ends are compared nothing of the comparison is
        // counted or listed.
        assert_eq!(summary(&view), summary(&plain));
        assert_eq!(side(&view), side(&plain));
        assert!(!side(&view).contains("this change"));
        assert_eq!(canvas(&view), canvas(&plain));
        let said = note(&view);
        (
            said.strip_prefix(&note(&plain))
                .unwrap()
                .trim_start()
                .to_owned(),
            comparing_order(&view),
        )
    };
    let base = json!({"name": "main", "commit": "0123456789ab"});
    for (change, said, asked_again) in [
        (
            json!({"state": "waiting_head", "base": base}),
            "Waiting for the graph of the change before telling what it did to the order.",
            true,
        ),
        (
            json!({"state": "building", "base": base}),
            "Preparing the graph at base 0123456789ab to tell what the change did to the order…",
            true,
        ),
        (
            json!({"state": "building", "base": null}),
            "Preparing the graph at the base of the change to tell what the change did to the order…",
            true,
        ),
        (
            json!({"state": "unavailable", "reason": "tests", "base": base}),
            "What the change did to the order cannot be told with Without tests for this cached revision.",
            false,
        ),
        (
            json!({"state": "unavailable", "reason": "no_base", "base": null}),
            "Where this change started could not be determined, so what it did to the order is not told.",
            false,
        ),
        (
            json!({"state": "unavailable", "reason": "head", "base": base}),
            "What the change did to the order needs the graph of the change itself.",
            false,
        ),
        // The graph is rebuilt and the map follows it: that is asked
        // for again by what says so, not by the comparison.
        (
            json!({"state": "unavailable", "reason": "stale", "base": base}),
            "What the change did to the order is told once the graph is of these files.",
            false,
        ),
        (
            json!({"state": "unavailable", "base": base}),
            "Could not compare with base 0123456789ab.",
            false,
        ),
        (
            json!({"state": "unavailable", "base": null}),
            "Could not compare with the base of the change.",
            false,
        ),
    ] {
        assert_eq!(
            told(change.clone()),
            (said.to_owned(), asked_again),
            "{change}"
        );
    }
    // Told, or with no change chosen, the picture is not asked for
    // again and the note says nothing more.
    let mut ready = plain.clone();
    ready["pattern"]["change"] = json!({
        "state": "ready", "base": base,
        "added": {"pairs": 0, "links": 0, "guessed": 0, "rows": [], "more": 0},
        "removed": {"pairs": 0, "links": 0, "guessed": 0, "rows": [], "more": 0},
    });
    assert_eq!(note(&ready), note(&plain));
    assert!(!comparing_order(&ready));
    assert!(!comparing_order(&plain));
    assert!(!comparing_order(
        &json!({"pattern": {"change": {"state": "unavailable"}}})
    ));
}
#[test]
fn the_middle_of_a_curve_is_where_a_word_about_it_stands() {
    assert_eq!(
        middle("M761.0 60.0 C761.0 100.0 371.0 100.0 371.0 60.0"),
        Some((566.0, 90.0))
    );
    // Not the middle of its ends: the handles pull it.
    assert_eq!(middle("M0 0 C0 8 8 16 16 16"), Some((5.0, 11.0)));
    // A straight run has a middle too, and the angle it runs at.
    assert_eq!(halfway("M10.0 10.0 L20.0 20.0"), Some((15.0, 15.0, 45.0)));
    assert_eq!(halfway("M30 4 L10 4"), Some((20.0, 4.0, 180.0)));
    for other in [
        "M1 2 C3 4 5 6 7",
        "M1 2 C3 4 5 6 7 8 9",
        "M1 2 C3 4 5 6 7 x",
        "M1 2 L3",
        "M1 2",
        "M1 2 3 L4 5",
        "M1 2 Q3 4 5 6",
        "L1 2 C3 4 5 6 7 8",
        "",
    ] {
        assert_eq!(middle(other), None, "{other}");
    }
}
/// A link that passes layers runs in pieces: a curve into the slot of
/// each layer it passes, straight through it, and a curve on. A word
/// about it stands halfway along the whole of it, so the links that
/// reach furthest keep their marks.
#[test]
fn a_link_of_several_pieces_has_its_middle_halfway_along_them() {
    // Curve, straight slot, curve: the middle lies in the slot.
    let (x, y, angle) =
        halfway("M164.0 15.0 C200.0 15.0 200.0 40.0 236.0 40.0 L400.0 40.0 C436.0 40.0 436.0 15.0 472.0 15.0")
            .unwrap();
    assert_eq!((x, y, angle), (318.0, 40.0, 0.0));
    // Drawn right to left, as the weaker direction of a cycle is, it
    // runs the other way at the same place.
    let (x, y, angle) =
        halfway("M472.0 15.0 C436.0 15.0 436.0 40.0 400.0 40.0 L236.0 40.0 C200.0 40.0 200.0 15.0 164.0 15.0")
            .unwrap();
    assert_eq!((x, y, angle), (318.0, 40.0, 180.0));
    // Two slots of unlike length: halfway by length, not by pieces.
    let (x, y, _) = halfway("M0 0 L10 0 L10 30").unwrap();
    assert_eq!((x, y), (10.0, 10.0));
    // Every long link of a picture has a middle that lies on it.
    let plan = plan(4, &[(0, 1, 1), (1, 2, 1), (2, 3, 1), (0, 3, 1), (0, 2, 1)]);
    let long = plan
        .wires
        .iter()
        .filter(|wire| wire.path.contains(" L"))
        .collect::<Vec<_>>();
    assert_eq!(long.len(), 2, "{plan:?}");
    for wire in long {
        let (x, _) = middle(&wire.path).expect("a middle of a long link");
        let (from, to) = (plan.nodes[0].x + BOX_WIDTH, plan.nodes[3].x);
        assert!(x > from && x < to, "{x} of {}", wire.path);
    }
}
#[test]
fn a_selected_layer_says_what_places_a_file_in_it_and_lists_its_files() {
    let mut view = layers();
    let mut focus = view["nodes"][3].clone();
    let beside = |layer: &str, name: &str, links: u64, against: bool| {
        json!({
            "path": format!("layer:{layer}"), "name": name, "folder": false, "inside": true,
            "links": links, "guessed": 0, "changed": false, "against": against,
        })
    };
    for (key, value) in [
        ("inside", json!(true)),
        (
            "signs",
            json!("a name that ends in <model>, or a folder named models"),
        ),
        ("uses", json!([beside("services", "Services", 2, true)])),
        (
            "used_by",
            json!([
                beside("services", "Services", 3, false),
                beside("data", "Data access", 2, false)
            ]),
        ),
        (
            "members",
            json!([{
                "path": "app/Models/Order.php", "name": "Order.php", "folder": false,
                "inside": false, "links": 5, "guessed": 0, "changed": false
            }]),
        ),
        ("members_more", json!(1199)),
        (
            "against",
            json!({"rows": [view["pattern"]["against"]["rows"][0].clone()], "more": 0}),
        ),
    ] {
        focus[key] = value;
    }
    view["focus"] = focus;
    let html = side(&view);
    assert!(html.starts_with("<h3>Model</h3><p class=\"knowledge-note\">Layer · 1,200 files · placed by a name that ends in &lt;model&gt;, or a folder named models</p><h4>Uses <span class=\"graph-map-total\">1</span></h4>"), "{html}");
    // Another layer is a row that selects it; what runs back says so.
    assert!(html.contains("&quot;path&quot;:&quot;layer:services&quot;"));
    assert!(
        html.contains(
            "Services</span><span class=\"graph-map-row-links\">2 links · against the order</span>"
        ),
        "{html}"
    );
    assert!(html.contains("<h4>Used by <span class=\"graph-map-total\">2</span></h4>"));
    assert!(html.contains("Services</span><span class=\"graph-map-row-links\">3 links</span>"));
    assert!(html.contains("Data access</span><span class=\"graph-map-row-links\">2 links</span>"));
    // Each of them is a link between two layers, which the mark beside
    // it selects: from the layer to what it uses, to it from what uses
    // it.
    assert_eq!(html.matches("class=\"graph-map-row-look\"").count(), 3);
    assert!(html.contains("class=\"graph-map-row-look\" data-action=\"graph-map-link\" data-payload=\"{&quot;from&quot;:&quot;layer:model&quot;,&quot;to&quot;:&quot;layer:services&quot;}\" title=\"See what makes up this link\" aria-label=\"See what makes up the link with Services\">↗</button>"), "{html}");
    assert!(html.contains("data-payload=\"{&quot;from&quot;:&quot;layer:data&quot;,&quot;to&quot;:&quot;layer:model&quot;}\" title=\"See what makes up this link\" aria-label=\"See what makes up the link with Data access\">"));
    assert!(html.contains(
        "<h4>Against the order, from here <span class=\"graph-map-total\">1</span></h4>"
    ));
    assert_eq!(html.matches("data-pair=\"true\"").count(), 1);
    // The files of the layer are no links to look into.
    assert_eq!(html.matches("data-action=\"graph-map-link\"").count(), 4);
    // Its files, the most linked first, each a way to it in its folder.
    assert!(html.contains("<h4>Files <span class=\"graph-map-total\">1,200</span></h4>"));
    assert!(html.contains("title=\"app/Models/Order.php\"><span class=\"graph-map-row-name\">Order.php</span><span class=\"graph-map-row-links\">5 links</span>"), "{html}");
    assert!(html.ends_with("<p class=\"knowledge-note\">and 1,199 more</p>"));
    let mut none = view.clone();
    none["focus"]["against"] = json!({"rows": [], "more": 0});
    assert!(side(&none).contains("<h4>Against the order, from here <span class=\"graph-map-total\">0</span></h4><p class=\"knowledge-note\">Nothing here needs a layer before this one.</p>"));
    // In the picture the layer is selected, and what it has no link
    // with steps back.
    let html = canvas(&view);
    assert_eq!(html.matches("data-selected=\"true\"").count(), 1);
    assert_eq!(html.matches("aria-pressed=\"true\"").count(), 1);
    assert_eq!(html.matches(" graph-map-edge-on").count(), 3);
    assert_eq!(html.matches(" graph-map-edge-off").count(), 3);
    assert_eq!(html.matches("data-dim=\"true\"").count(), 1);
}
#[test]
fn a_selected_link_between_two_layers_lists_the_pairs_of_files_it_is_made_of() {
    let mut view = layers();
    let end = |path: &str, layer: &str| json!({"path": path, "name": base(path), "layer": layer});
    view["focus"] = json!({
        "path": "layer:services>layer:model", "link": true,
        "from": {"path": "layer:services", "name": "Services"},
        "to": {"path": "layer:model", "name": "Model"},
        "links": 3, "guessed": 1, "against": false, "pairs": 2, "more": 0,
        "rows": [
            {
                "from": end("app/Services/Pay<b>.php", "Services"),
                "to": end("app/Models/Order.php", "Model"),
                "links": 2, "guessed": 1
            },
            {
                "from": end("app/Listeners/SendReceipt.php", "Services"),
                "to": end("app/Events/OrderPaid.php", "Model"),
                "links": 1, "guessed": 0
            }
        ],
    });
    // In the picture the link stands out with its two layers, and no
    // layer is the selected one.
    let html = canvas(&view);
    assert_eq!(html.matches(" graph-map-edge-on").count(), 1);
    assert_eq!(html.matches(" graph-map-edge-off").count(), 5);
    assert!(
        html.contains("<path class=\"graph-map-edge graph-map-edge-on\" data-ends=\"1 3\""),
        "{html}"
    );
    assert!(html.contains(
        "<path class=\"graph-map-edge graph-map-edge-off graph-map-edge-against\" data-ends=\"3 1\""
    ));
    assert_eq!(html.matches("data-selected=\"true\"").count(), 0);
    assert_eq!(html.matches("aria-pressed=\"true\"").count(), 0);
    assert_eq!(html.matches("data-dim=\"true\"").count(), 2);
    for (layer, dim) in [
        ("Entry points", true),
        ("Services", false),
        ("Data access", true),
        ("Model", false),
    ] {
        let button = html
            .split("<button ")
            .find(|button| button.contains(&format!("aria-label=\"{layer}, layer")))
            .unwrap();
        assert_eq!(button.contains("data-dim=\"true\""), dim, "{layer}");
    }
    // Beside it: the two layers once, then each pair of files as a way
    // to what makes up its link.
    let html = side(&view);
    assert!(html.starts_with("<h3>Services → Model</h3><p class=\"knowledge-note\">Link between two layers · 3 links, 1 guessed · 2 pairs of files</p><h4>Pairs of files <span class=\"graph-map-total\">2</span></h4>"), "{html}");
    assert!(html.contains("data-pair=\"true\" data-action=\"graph-map-link\" data-payload=\"{&quot;from&quot;:&quot;app/Services/Pay&lt;b&gt;.php&quot;,&quot;to&quot;:&quot;app/Models/Order.php&quot;}\" title=\"app/Services/Pay&lt;b&gt;.php uses app/Models/Order.php · select to see what makes up this link\"><span class=\"graph-map-row-name\">Pay&lt;b&gt;.php → Order.php</span><span class=\"graph-map-row-links\">2 links, 1 guessed</span>"), "{html}");
    assert!(html.contains("<span class=\"graph-map-row-name\">SendReceipt.php → OrderPaid.php</span><span class=\"graph-map-row-links\">1 link</span>"));
    assert_eq!(html.matches("data-action=\"graph-map-link\"").count(), 2);
    // A link against the order says so, one pair is one, and what did
    // not fit is counted.
    let mut back = view.clone();
    for (key, value) in [
        ("against", json!(true)),
        ("pairs", json!(1)),
        ("more", json!(1200)),
    ] {
        back["focus"][key] = value;
    }
    let html = side(&back);
    assert!(html.contains("<p class=\"knowledge-note\">Link between two layers · 3 links, 1 guessed · 1 pair of files · against the order</p><h4>Pairs of files <span class=\"graph-map-total\">1,202</span></h4>"), "{html}");
    assert!(html.ends_with("<p class=\"knowledge-note\">and 1,200 more</p>"));
}
#[test]
fn a_link_between_two_layers_is_selected_and_any_other_is_looked_into() {
    let mut map = fresh("");
    map["scope"] = json!("app");
    map["patterns"] = json!(true);
    let shown = showing_pattern(map.clone(), "layers@app").unwrap();
    // Selected, the link is the focus of the picture; selected again,
    // it is let go of; another link takes its place.
    let selected = between(shown.clone(), "layer:services", "layer:model").unwrap();
    assert_eq!(selected["focus"], "layer:services>layer:model");
    let mut expected = shown.clone();
    expected["focus"] = json!("layer:services>layer:model");
    assert_eq!(selected, expected);
    let other = between(selected.clone(), "layer:model", "layer:services").unwrap();
    assert_eq!(other["focus"], "layer:model>layer:services");
    assert_eq!(
        between(selected.clone(), "layer:services", "layer:model").unwrap(),
        shown
    );
    // A selected layer gives way to a link.
    let mut layer = shown.clone();
    layer["focus"] = json!("layer:model");
    assert_eq!(
        between(layer, "layer:services", "layer:model").unwrap()["focus"],
        "layer:services>layer:model"
    );
    // A pair of files of the layers is looked into, and so is every
    // link of any other picture.
    assert!(between(
        shown.clone(),
        "app/Models/Order.php",
        "app/Services/Pay.php"
    )
    .is_none());
    assert!(between(shown.clone(), "layer:model", "app/Services/Pay.php").is_none());
    assert!(between(shown.clone(), "app/Models/Order.php", "layer:services").is_none());
    assert!(between(map.clone(), "layer:services", "layer:model").is_none());
    let strategy = showing_pattern(map, "strategy@app/Pay/Gateway.php#L3:Gateway").unwrap();
    assert!(between(strategy, "layer:services", "layer:model").is_none());
}
#[test]
fn overview_groups_pattern_families_marks_nodes_and_enriches_architecture_side() {
    let mut view = marked_folder();
    view["nodes"] = json!([
        {"path": "app/Pay", "name": "Pay", "folder": true, "files": 9, "inside": true, "out": 2, "in": 0, "patterns": 1200, "architecture_layer": {"id": "services", "name": "Services", "color": 1}},
        {"path": "app/Repo.php", "name": "Repo.php", "folder": false, "files": 1, "symbols": 2, "inside": true, "out": 0, "in": 1, "patterns": 1},
        {"path": "app/Kernel.php", "name": "Kernel.php", "folder": false, "files": 1, "symbols": 2, "inside": true, "out": 0, "in": 0, "patterns": 1}
    ]);
    view["edges"] = json!([
        {"from": "app/Pay", "to": "app/Repo.php", "links": 2, "guessed": 0, "architecture_violation": 2}
    ]);
    view["patterns"]["kinds"][0]["instances"] = json!([
        {"id": "repository@app/Pay/Orders<T>.php#L3:Orders<T>", "sort": "repository", "name": "Repository", "of": "Orders<T>", "file": "app/Pay/Orders<T>.php", "line": 3, "named": true, "guessed": false, "parts": "2 implementations"},
        {"id": "repository@app/Repo.php#L1:Repo", "sort": "repository", "name": "Repository", "of": "Repo", "file": "app/Repo.php", "line": 1, "named": true, "guessed": false, "parts": "1 user"}
    ]);
    view["patterns"]["kinds"][1]["instances"] = json!([
        {"id": "strategy@app/Pay/Gateway.php#L3:Gateway", "sort": "strategy", "name": "Strategy", "of": "Gateway", "file": "app/Pay/Gateway.php", "line": 3, "named": false, "guessed": true, "parts": "2 implementations · 1 user"},
        {"id": "strategy@app/Kernel.php#L1:Kernel", "sort": "strategy", "name": "Strategy", "of": "Kernel", "file": "app/Kernel.php", "line": 1, "named": false, "guessed": false, "parts": "1 implementation"}
    ]);
    let mut kinds = view["patterns"]["kinds"].as_array().unwrap().clone();
    kinds.push(json!({
        "id": "layers", "name": "Layers", "family": "architecture",
        "tells": "Files filed in layers by their names.", "found": 1, "more": 0,
        "instances": [{"id": "layers@app", "sort": "layers", "name": "Layers", "of": "app", "file": "app", "line": 0, "named": true, "guessed": false, "parts": "2 layers"}]
    }));
    view["patterns"]["kinds"] = json!(kinds);
    view["architecture"] = json!({
        "declared": true,
        "layers": [
            {"id": "entry", "name": "Entry points", "color": 0},
            {"id": "services", "name": "Services", "color": 1}
        ],
        "violations": {"links": 2, "guessed": 0, "pairs": 1, "more": 0, "rows": [
            {"from": {"path": "app/Pay/A.php", "name": "A.php", "layer": "Services"}, "to": {"path": "app/Repo.php", "name": "Repo.php", "layer": "Entry points"}, "links": 2, "guessed": 0}
        ]},
        "reciprocal_total": 1,
        "reciprocal": [{
            "from": {"id": "entry", "name": "Entry points", "color": 0},
            "to": {"id": "services", "name": "Services", "color": 1},
            "forward": 3, "forward_guessed": 0,
            "reverse": 1, "reverse_guessed": 1
        }]
    });

    assert_eq!(pattern_family_of(&view, "app/Pay"), Some("both"));
    assert_eq!(pattern_family_of(&view, "app/Repo.php"), Some("arch"));
    assert_eq!(pattern_family_of(&view, "app/Kernel.php"), Some("design"));
    assert_eq!(pattern_family_of(&view, ""), None);

    let html = canvas(&view);
    assert!(html.contains("data-patterns-on=\"true\""), "{html}");
    assert!(html.contains("data-architecture=\"true\""), "{html}");
    assert!(html.contains("data-has-patterns=\"true\""), "{html}");
    assert!(html.contains("data-pattern-family=\"both\""), "{html}");
    assert!(html.contains("data-pattern-family=\"arch\""), "{html}");
    assert!(html.contains("data-pattern-family=\"design\""), "{html}");
    assert!(html.contains("data-arch-layer-id=\"services\""), "{html}");
    assert!(html.contains("class=\"graph-map-node-layer\""), "{html}");
    assert!(
        html.contains("class=\"graph-map-violation-halo\""),
        "{html}"
    );

    let patterns_html = found_patterns(&view, &[]);
    assert!(
        patterns_html
            .contains("<section class=\"graph-map-family-group\" data-family=\"architecture\">"),
        "{patterns_html}"
    );
    assert!(
        patterns_html.contains("<section class=\"graph-map-family-group\" data-family=\"design\">"),
        "{patterns_html}"
    );
    assert!(
        patterns_html.contains("class=\"graph-map-kind-card\""),
        "{patterns_html}"
    );
    assert!(
        patterns_html.contains(
            "<p class=\"graph-map-pattern-tells\">A type named a &lt;repository&gt;.</p>"
        ),
        "{patterns_html}"
    );
    assert!(
        patterns_html.contains("data-family=\"architecture\" data-sort=\"repository\""),
        "{patterns_html}"
    );
    assert!(
        patterns_html.contains("data-family=\"design\" data-sort=\"strategy\""),
        "{patterns_html}"
    );

    let arch_html = architecture_side(&view);
    assert!(
        arch_html
            .contains("class=\"graph-map-arch-mode\" data-declared=\"true\">Declared rules</span>"),
        "{arch_html}"
    );
    assert!(
        arch_html.contains(
            "<li data-arch-layer=\"entry\" style=\"--architecture-color:hsl(220 68% 46%)\">"
        ),
        "{arch_html}"
    );
    assert!(
        arch_html.contains(
            "class=\"button compact graph-map-arch-open\" data-action=\"graph-map-pattern\""
        ),
        "{arch_html}"
    );
    assert!(
        arch_html.contains("Inspect layers diagram</button>"),
        "{arch_html}"
    );
    assert!(
        arch_html.contains("class=\"graph-map-arch-pairs\""),
        "{arch_html}"
    );
    assert!(arch_html.contains("3 links"), "{arch_html}");
    assert!(arch_html.contains("1 link guessed"), "{arch_html}");
}
#[test]
fn focused_pattern_canvas_and_side_highlight_family_roles_and_ties() {
    let design = strategy();
    let html = canvas(&design);
    assert!(html.contains("data-pattern-family=\"design\""), "{html}");
    assert!(html.contains("data-pattern-sort=\"strategy\""), "{html}");
    assert!(html.contains("data-role-frame=\"anchor\""), "{html}");
    assert!(html.contains("data-role-frame=\"part\""), "{html}");
    assert!(html.contains("data-pattern-role=\"abstraction\""), "{html}");
    assert!(
        html.contains("<span class=\"graph-map-node-anchor\" aria-hidden=\"true\">★</span>"),
        "{html}"
    );
    assert!(html.contains("class=\"graph-map-edge-tie\""), "{html}");
    assert!(html.contains(">implements</text>"), "{html}");
    assert!(html.contains(">holds &amp; takes</text>"), "{html}");

    let design_side = side(&design);
    assert!(
        design_side.contains("class=\"graph-map-pattern-brief\" data-family=\"design\""),
        "{design_side}"
    );
    assert!(design_side.contains("Design pattern"), "{design_side}");
    assert!(
        design_side.contains("Includes guessed links"),
        "{design_side}"
    );
    assert!(
        design_side.contains("12 implementations stand under Gateway."),
        "{design_side}"
    );
    assert!(
        design_side.contains("class=\"graph-map-part-chip\" data-action=\"graph-map-node\""),
        "{design_side}"
    );
    assert!(design_side.contains("Gateway"), "{design_side}");
    assert!(design_side.contains("Pay&lt;b&gt;"), "{design_side}");

    let mut arch = design.clone();
    arch["pattern"]["sort"] = json!("repository");
    arch["pattern"]["name"] = json!("Repository");
    arch["pattern"]["family"] = json!("architecture");
    arch["pattern"]["named"] = json!(true);
    arch["pattern"]["guessed"] = json!(false);
    let arch_html = canvas(&arch);
    assert!(
        arch_html.contains("data-pattern-family=\"architecture\""),
        "{arch_html}"
    );
    let arch_side = side(&arch);
    assert!(
        arch_side.contains("class=\"graph-map-pattern-brief\" data-family=\"architecture\""),
        "{arch_side}"
    );
    assert!(arch_side.contains("Architecture pattern"), "{arch_side}");
    assert!(arch_side.contains("By name"), "{arch_side}");
    assert!(arch_side.contains("Verified from code"), "{arch_side}");

    let mut placed = Vec::new();
    assert_eq!(place_tie(&mut placed, (100.0, 100.0)), Some((100.0, 100.0)));
    assert_eq!(place_tie(&mut placed, (105.0, 95.0)), Some((105.0, 85.0)));
    assert_eq!(place_tie(&mut placed, (105.0, 105.0)), Some((105.0, 115.0)));
    assert_eq!(place_tie(&mut placed, (105.0, 95.0)), None);
    assert_eq!(place_tie(&mut placed, (180.0, 95.0)), Some((180.0, 95.0)));
}
#[test]
fn layers_diagram_renders_tier_bands_order_badges_and_violation_halos() {
    let view = layers();
    let html = canvas(&view);
    assert!(html.contains("data-layered=\"true\""), "{html}");
    assert!(
        html.contains("data-pattern-family=\"architecture\""),
        "{html}"
    );
    assert!(
        html.contains("<rect class=\"graph-map-arch-band\""),
        "{html}"
    );
    assert_eq!(html.matches("class=\"graph-map-arch-band\"").count(), 1);
    assert!(html.contains("data-layer=\"true\""), "{html}");
    for order in 1..=4 {
        assert!(
            html.contains(&format!("data-layer-order=\"{order}\"")),
            "{html}"
        );
        assert!(
            html.contains(&format!(
                "<span class=\"graph-map-layer-order\" aria-hidden=\"true\">{order}</span>"
            )),
            "{html}"
        );
    }
    assert_eq!(
        html.matches("class=\"graph-map-violation-halo\"").count(),
        2
    );

    let mut focused_violation = view;
    focused_violation["focus"] = json!({
        "path": "layer:model>layer:services", "link": true,
        "from": {"path": "layer:model", "name": "Model"},
        "to": {"path": "layer:services", "name": "Services"},
        "links": 2, "guessed": 0, "against": true, "pairs": 1, "more": 0,
        "rows": []
    });
    let focused_html = canvas(&focused_violation);
    assert!(
        focused_html.contains(
            "<path class=\"graph-map-violation-halo\" data-edge-state=\"on\" data-ends=\"3 1\""
        ),
        "{focused_html}"
    );
    assert!(
        focused_html.contains(
            "<path class=\"graph-map-violation-halo\" data-edge-state=\"off\" data-ends=\"2 0\""
        ),
        "{focused_html}"
    );
}
#[test]
fn camera_fit_bbox_zoom_and_pan_stay_within_bounds() {
    let (fit_s, _, _) = camera_for_fit((2000.0, 1200.0), (800.0, 500.0));
    assert!((0.25..=1.0).contains(&fit_s), "fit scale={fit_s}");

    let (small_s, small_x, small_y) = camera_for_fit((300.0, 200.0), (800.0, 500.0));
    assert_eq!(small_s, 1.0);
    assert_eq!((small_x, small_y), (250.0, 150.0));

    let bbox = (200.0, 100.0, 364.0, 130.0);
    let (focus_s, focus_x, focus_y) = camera_for_bbox(bbox, (800.0, 500.0), FOCUS_CAMERA_MAX);
    assert!(
        (1.0..=FOCUS_CAMERA_MAX).contains(&focus_s),
        "focus scale={focus_s}"
    );
    // Logical midpoint of bbox maps cleanly onto viewport center (400, 250).
    let mid_x = (bbox.0 + bbox.2) / 2.0;
    let mid_y = (bbox.1 + bbox.3) / 2.0;
    assert!(((mid_x * focus_s + focus_x) - 400.0).abs() <= 1.0);
    assert!(((mid_y * focus_s + focus_y) - 250.0).abs() <= 1.0);

    // Zoom at anchor keeps the logical point fixed and clamps to [0.25, 2.4].
    let (z_max, zx, zy) = zoom_at(1.0, 40.0, 20.0, (400.0, 250.0), 9.0);
    assert_eq!(z_max, 2.4);
    let lx = (400.0 - 40.0) / 1.0;
    let ly = (250.0 - 20.0) / 1.0;
    assert!(((lx * z_max + zx) - 400.0).abs() <= 1.0);
    assert!(((ly * z_max + zy) - 250.0).abs() <= 1.0);

    let (z_min, _, _) = zoom_at(1.0, 0.0, 0.0, (400.0, 250.0), 0.05);
    assert_eq!(z_min, 0.25);
}
#[test]
fn prompt_matcher_finds_matching_nodes_and_ignores_unrelated_text() {
    let view = json!({
        "nodes": [
            {"path": "src/Auth/JwtGuard.php", "name": "JwtGuard.php", "folder": false, "out": 2, "in": 1},
            {"path": "src/Auth", "name": "Auth", "folder": true, "files": 4, "out": 3, "in": 2},
            {"path": "src/Billing/Invoice.php", "name": "Invoice.php", "folder": false, "out": 1, "in": 0}
        ],
        "frames": []
    });
    let matched = match_prompt_targets(&view, "Please update JwtGuard token validation in Auth")
        .expect("should match Auth & JwtGuard");
    assert_eq!(matched.phase, "predicting");
    assert!(matched.summary.starts_with("Focusing near "));
    assert!(matched
        .target_paths
        .iter()
        .any(|p| p == "src/Auth/JwtGuard.php"));
    assert!(matched.target_folders.iter().any(|p| p == "src/Auth"));
    assert!(match_prompt_targets(&view, "please explain this code").is_none());
}
#[test]
fn focus_overlay_renders_target_zone_entity_badges_and_unique_svg_ids() {
    let view = json!({
        "scope": "",
        "nodes": [
            {"path": "src/Auth", "name": "Auth", "folder": true, "files": 3, "out": 1, "in": 0},
            {"path": "src/Billing", "name": "Billing", "folder": true, "files": 2, "out": 0, "in": 1}
        ],
        "edges": [
            {"from": "src/Auth", "to": "src/Billing", "links": 2, "guessed": 0}
        ],
        "frames": []
    });
    // Without live focus, default modal canvas stays completely unchanged.
    let modal_html = canvas(&view);
    assert!(!modal_html.contains("graph-map-target-zone"));
    assert!(!modal_html.contains("graph-map-entity-node"));
    assert!(!modal_html.contains("data-focus-target=\"true\""));

    let focus = parse_live_focus(&json!({
        "phase": "active",
        "summary": "Building Auth token classes",
        "target_folders": ["src/Auth"],
        "target_paths": ["src/Auth/JwtToken.php"],
        "target_nodes": ["src/Auth/JwtToken.php"],
        "created_entities": [
            {"id": "e1", "kind": "file", "name": "JwtToken.php", "parent": "src/Auth", "path": "src/Auth/JwtToken.php"},
            {"id": "e2", "kind": "class", "name": "JwtToken", "parent": "src/Auth/JwtToken.php", "path": "src/Auth/JwtToken.php#L3:JwtToken"},
            {"id": "e3", "kind": "interface", "name": "TokenSigner", "parent": "src/Auth", "path": "src/Auth/TokenSigner.php"},
            {"id": "e4", "kind": "trait", "name": "TokenClaims", "parent": "src/Auth", "path": "src/Auth/TokenClaims.php"},
            {"id": "e5", "kind": "enum", "name": "TokenKind", "parent": "src/Auth", "path": "src/Auth/TokenKind.php"},
            {"id": "e6", "kind": "struct", "name": "Header", "parent": "src/Auth", "path": "src/Auth/Header.rs"},
            {"id": "e7", "kind": "method", "name": "issue", "parent": "src/Auth", "path": "src/Auth/JwtToken.php#L10:issue"},
            {"id": "e8", "kind": "function", "name": "parse_jwt", "parent": "src/Auth", "path": "src/Auth/helpers.php#L4:parse_jwt"}
        ]
    }))
    .expect("live focus parses cleanly");

    let rendered = canvas_with(&view, "session-graph", Some(&focus));
    assert!(rendered.html.contains("id=\"session-graph-arrow\""));
    assert!(rendered.html.contains("url(#session-graph-flow-0)"));
    assert!(!rendered.html.contains("id=\"graph-map-arrow\""));
    assert!(rendered
        .html
        .contains("<rect class=\"graph-map-target-zone\""));
    assert!(rendered.html.contains("data-focus-target=\"true\""));
    assert_eq!(rendered.focus.entities.len(), 8);
    assert!(rendered.focus.bbox.is_some());
    for kind in [
        "file",
        "class",
        "interface",
        "trait",
        "enum",
        "struct",
        "method",
        "function",
    ] {
        assert!(
            rendered
                .html
                .contains(&format!("data-entity-kind=\"{kind}\"")),
            "missing kind {kind} in {}",
            rendered.html
        );
    }
    assert!(rendered
        .html
        .contains("class=\"graph-map-node graph-map-entity-node is-blinking\""));
    assert!(rendered
        .html
        .contains("<span class=\"graph-map-entity-badge\">NEW</span>"));

    let chips = focus_strip_chips_html(&focus);
    assert!(chips.contains("session-graph-focus-chip"));
    assert!(chips.contains("data-action=\"session-graph-focus-jump\""));
    assert!(chips.contains("class: JwtToken"));
}

#[test]
fn unbuilt_or_building_map_errors_show_calm_note_not_alert() {
    let building = json!({ "state": "building" });
    let off = json!({ "state": "off" });
    let ready = json!({ "state": "ready", "graph": { "commit": "abc" } });

    assert!(embedded_error_is_unbuilt(
        &building,
        "This project has no published code graph."
    ));
    assert_eq!(
        embedded_unbuilt_note(&building),
        "Building code graph and dependency map…"
    );
    assert!(embedded_error_is_unbuilt(&off, "Forbidden"));
    assert_eq!(
        embedded_unbuilt_note(&off),
        "Code graph is turned off for this project."
    );
    assert!(embedded_error_is_unbuilt(
        &V::Null,
        "This project has no published code graph."
    ));
    assert!(!embedded_error_is_unbuilt(&ready, "Internal server error"));
}

#[test]
fn created_entities_with_modified_or_deleted_change_render_accurate_badges() {
    let view = json!({
        "project_id": "proj",
        "scope": "src",
        "nodes": [
            {
                "path": "src/Auth.php",
                "name": "Auth.php",
                "folder": false,
                "kind": "file",
                "line": 1,
                "changed": 1,
                "links": 1
            }
        ],
        "edges": [],
        "frames": []
    });
    let focus = parse_live_focus(&json!({
        "phase": "active",
        "target_paths": ["src/Auth.php"],
        "created_entities": [
            {
                "id": "e1",
                "kind": "method",
                "change": "modified",
                "name": "refreshToken",
                "parent": "src/Auth.php",
                "path": "src/Auth.php",
                "folder": "src",
                "line": 42
            },
            {
                "id": "e2",
                "kind": "class",
                "change": "created",
                "name": "JwtClaim",
                "parent": "src/Auth.php",
                "path": "src/Auth.php",
                "folder": "src",
                "line": 10
            },
            {
                "id": "e3",
                "kind": "function",
                "change": "deleted",
                "name": "legacyHash",
                "parent": "src/Auth.php",
                "path": "src/Auth.php",
                "folder": "src",
                "line": 88
            }
        ]
    }))
    .expect("focus should parse");

    let rendered = canvas_with(&view, "session-graph", Some(&focus));
    assert!(rendered.html.contains("data-entity-change=\"modified\""));
    assert!(rendered
        .html
        .contains("<span class=\"graph-map-entity-badge\">MOD</span>"));
    assert!(rendered.html.contains("data-entity-change=\"created\""));
    assert!(rendered
        .html
        .contains("<span class=\"graph-map-entity-badge\">NEW</span>"));
    assert!(rendered.html.contains("data-entity-change=\"deleted\""));
    assert!(rendered
        .html
        .contains("<span class=\"graph-map-entity-badge\">DEL</span>"));

    let chips = focus_strip_chips_html(&focus);
    assert!(chips.contains("data-entity-change=\"modified\""));
    assert!(chips.contains("<span class=\"graph-map-entity-badge\">MOD</span>"));
    assert!(chips.contains("method: refreshToken"));
    assert!(chips.contains("<span class=\"graph-map-entity-badge\">NEW</span>"));
}

#[test]
fn normalize_and_request_paths_handle_whole_changes_and_impact_params() {
    let state = fresh("");
    assert_eq!(state["whole"], json!(false));
    assert_eq!(state["side_tab"], json!("details"));
    assert_eq!(state["changes_filter"], json!("all"));

    let whole_state = normalize_map_state(json!({
        "scope": "",
        "target": "work",
        "whole": true
    }));
    let map_url = build_map_path("my/proj", &whole_state, true);
    assert!(map_url.contains("/api/projects/my%2Fproj/graph/map?"));
    assert!(map_url.contains("whole=1"));
    assert!(map_url.contains("target=work"));

    let changes_url = build_changes_path("my/proj", "work", "sess-1", true);
    assert_eq!(
        changes_url,
        "/api/projects/my%2Fproj/graph/changes?target=work&focus_session=sess-1&notests=1"
    );

    let impact_url = build_file_impact_path("my/proj", "src/Auth.php#L10:Auth", "", "sess-1", true);
    assert_eq!(
        impact_url,
        "/api/projects/my%2Fproj/graph/file-impact?path=src%2FAuth.php&target=work&focus_session=sess-1&certain=1"
    );
}

#[test]
fn state_focus_file_unfolds_collapsed_ancestors_and_selects_details() {
    let hidden_view = json!({
        "nodes": [
            { "path": "src", "name": "src", "folder": true }
        ],
        "edges": []
    });
    let initial = normalize_map_state(json!({
        "scope": "docs",
        "spread": [],
        "filter": "old",
        "whole": false,
        "side_tab": "changes",
        "modules": true
    }));
    let (next, needs_reload) = state_focus_file(initial, "src/auth/jwt/Token.php", &hidden_view);
    assert!(needs_reload);
    assert_eq!(next["scope"], json!(""));
    assert_eq!(next["filter"], json!(""));
    assert_eq!(next["focus"], json!("src/auth/jwt/Token.php"));
    assert_eq!(next["side_tab"], json!("details"));
    assert_eq!(next["side_pinned"], json!(true));
    assert_eq!(next["modules"], json!(false));
    let spread = unfolded(&next, "spread");
    assert_eq!(spread, vec!["src", "src/auth", "src/auth/jwt"]);

    let visible_view = json!({
        "scope": "",
        "focus": { "path": "src/auth/jwt/Token.php", "folder": false },
        "nodes": [
            { "path": "src/auth/jwt/Token.php", "name": "Token.php", "folder": false }
        ],
        "edges": []
    });
    let (same, already_there) =
        state_focus_file(next.clone(), "src/auth/jwt/Token.php", &visible_view);
    assert!(!already_there);
    assert_eq!(same["focus"], json!("src/auth/jwt/Token.php"));
    assert_eq!(same["side_tab"], json!("details"));
}

#[test]
fn changes_side_html_renders_filters_badges_missing_disk_and_entities() {
    let changes = json!({
        "target": "work",
        "total_files": 3,
        "created_entities": 2,
        "modified_entities": 1,
        "deleted_entities": 1,
        "files": [
            {
                "path": "src/NewService.php",
                "status": "created",
                "exists": true,
                "in_graph": false,
                "summary": { "uses_added": 2, "uses_removed": 0, "callers_at_risk": 0, "impacted_callers": 0 },
                "entities": [
                    { "id": "c1", "kind": "class", "change": "created", "name": "NewService", "line": 5 }
                ]
            },
            {
                "path": "src/Auth.php",
                "status": "modified",
                "exists": true,
                "in_graph": true,
                "summary": { "uses_added": 1, "uses_removed": 1, "callers_at_risk": 2, "impacted_callers": 1 },
                "entities": [
                    { "id": "m1", "kind": "method", "change": "modified", "name": "login", "line": 22 }
                ]
            },
            {
                "path": "src/Legacy.php",
                "status": "deleted",
                "exists": false,
                "in_graph": true,
                "summary": { "uses_added": 0, "uses_removed": 3, "callers_at_risk": 1, "impacted_callers": 0 },
                "entities": []
            }
        ]
    });

    let all_html = changes_side_html(&changes, "all", "src/Auth.php", "session");
    assert!(all_html.contains("data-action=\"graph-changes-filter\""));
    assert!(all_html.contains("src/NewService.php"));
    assert!(all_html.contains("Not indexed"));
    assert!(all_html.contains("data-selected=\"true\""));
    assert!(all_html.contains("Missing on disk"));
    assert!(all_html.contains("data-action=\"graph-map-impact-focus\""));
    assert!(all_html.contains("NewService"));
    assert!(all_html.contains("login"));

    let deleted_html = changes_side_html(&changes, "deleted", "", "modal");
    assert!(deleted_html.contains("src/Legacy.php"));
    assert!(!deleted_html.contains("src/Auth.php"));
    assert!(!deleted_html.contains("src/NewService.php"));
}

#[test]
fn impact_side_html_sorts_callers_and_renders_unindexed_and_dependencies() {
    let impact = json!({
        "path": "src/NewAuth.php",
        "status": "created",
        "exists": true,
        "in_graph": false,
        "baseline": { "available": true, "label": "HEAD" },
        "summary": {
            "uses_added": 1,
            "uses_removed": 1,
            "uses_unchanged": 1,
            "callers_at_risk": 1,
            "callers_impacted": 1,
            "added_callers": 1,
            "new_cycles": 1,
            "resolved_cycles": 0,
            "new_violations": 0,
            "resolved_violations": 0
        },
        "entities": [
            { "id": "e1", "kind": "method", "change": "modified", "name": "verify", "line": 14, "signature_changed": true }
        ],
        "uses": {
            "added": [{ "target": "src/Token.php", "resolved": true, "exists": true, "kind": "imports", "symbols": ["Token"], "line": 3, "evidence": "use App\\Token;" }],
            "removed": [{ "target": "src/OldToken.php", "resolved": false, "exists": false, "kind": "imports", "symbols": ["OldToken"], "line": 4 }],
            "unchanged": [{ "target": "src/Config.php", "resolved": true, "exists": true, "kind": "imports", "symbols": ["Config"], "line": 2 }]
        },
        "used_by": [
            { "path": "src/SafeCaller.php", "exists": true, "impact": "unchanged", "reason": "signature unaffected", "symbols": ["info"], "changed_symbols": [], "links": 1, "guessed": false },
            { "path": "src/WarmCaller.php", "exists": true, "impact": "impacted", "reason": "body changed", "symbols": ["verify"], "changed_symbols": ["verify"], "links": 1, "guessed": true },
            { "path": "src/BrokenCaller.php", "exists": true, "impact": "broken_risk", "reason": "signature changed", "symbols": ["verify"], "changed_symbols": ["verify"], "links": 1, "guessed": false }
        ],
        "architecture_impact": { "new_cycles": 1, "resolved_cycles": 0, "new_violations": 0, "resolved_violations": 0 }
    });

    let html = impact_side_html(&impact, &V::Null, "session");
    assert!(html.contains("class=\"graph-impact-card\""));
    assert!(html.contains("Exists in workspace"));
    assert!(html.contains("Not yet in Graphify index"));
    assert!(html.contains("Baseline: HEAD"));
    assert!(html.contains("src/Token.php"));
    assert!(html.contains("src/OldToken.php"));
    assert!(html.contains("src/Config.php"));
    assert!(html.contains("class=\"graph-impact-entity-name\">verify:14</span>"));
    assert!(html.contains("title=\"signature changed\""));
    assert!(html.contains(">sig</span>"));
    assert!(!html.contains("verify, verify"));
    assert!(html.contains("guessed"));
    assert!(html.contains("data-action=\"graph-map-link\""));

    let pos_broken = html
        .find("src/BrokenCaller.php")
        .expect("broken caller present");
    let pos_warm = html
        .find("src/WarmCaller.php")
        .expect("warm caller present");
    let pos_safe = html
        .find("src/SafeCaller.php")
        .expect("safe caller present");
    assert!(pos_broken < pos_warm && pos_warm < pos_safe);
}

#[test]
fn canvas_with_overlays_tags_changes_diff_edges_and_synthetic_folder_edges() {
    let view = json!({
        "project_id": "proj",
        "scope": "",
        "focus": { "path": "src/Auth.php", "folder": false },
        "nodes": [
            { "path": "src/Auth.php", "name": "Auth.php", "folder": false, "selected": true, "changed": 0, "links": 2 },
            { "path": "src/Token.php", "name": "Token.php", "folder": false, "changed": 0, "links": 1 },
            { "path": "src/billing", "name": "billing", "folder": true, "changed": 0, "links": 1 }
        ],
        "edges": [
            { "from": "src/Auth.php", "to": "src/Token.php", "links": 1, "kinds": [{ "kind": "calls", "links": 1 }] }
        ],
        "frames": []
    });
    let changes = json!({
        "files": [
            { "path": "src/Auth.php", "status": "modified", "exists": true },
            { "path": "src/billing/Invoice.php", "status": "created", "exists": true }
        ]
    });
    let impact = json!({
        "path": "src/Auth.php",
        "uses": {
            "added": [{ "target": "src/Token.php" }],
            "removed": [{ "target": "src/billing/Invoice.php" }],
            "unchanged": []
        },
        "used_by": []
    });

    let rendered = canvas_with_overlays(
        &view,
        "session-graph",
        CanvasOverlayContext {
            changes: Some(&changes),
            impact: Some(&impact),
            ..Default::default()
        },
    );
    let bbox = rendered.selected_bbox.expect("selected_bbox present");
    assert!(bbox.2 - bbox.0 > BOX_WIDTH);
    assert!(rendered.html.contains("data-change-status=\"modified\""));
    assert!(rendered.html.contains("data-diff=\"added\""));
    assert!(rendered
        .html
        .contains("class=\"graph-map-edge graph-map-edge-synthetic\""));
    assert!(rendered.html.contains("data-diff=\"removed\" d=\"M"));
    assert!(rendered.html.contains("data-impact=\"added-dep\""));
    assert!(rendered.html.contains("data-impact=\"removed-dep\""));
}

#[test]
fn link_canvas_and_side_with_render_diff_marks_from_impact() {
    let view = json!({
        "mode": "link",
        "from": { "path": "src/Auth.php", "name": "Auth.php", "what": "file" },
        "to": { "path": "src/Token.php", "name": "Token.php", "what": "file" },
        "users": {
            "names": 1,
            "files": 1,
            "groups": [
                {
                    "path": "src/Auth.php",
                    "name": "Auth.php",
                    "names": [
                        { "key": "src/Auth.php#L10:login", "name": "login", "kind": "method", "line": 10, "links": 1 }
                    ]
                }
            ]
        },
        "used": {
            "names": 1,
            "files": 1,
            "groups": [
                {
                    "path": "src/Token.php",
                    "name": "Token.php",
                    "names": [
                        { "key": "src/Token.php#L5:issue", "name": "issue", "kind": "method", "line": 5, "links": 1 }
                    ]
                }
            ]
        },
        "edges": [
            { "from": "src/Auth.php#L10:login", "to": "src/Token.php#L5:issue", "kind": "calls" }
        ],
        "kinds": [{ "kind": "calls", "links": 1 }]
    });
    let impact = json!({
        "path": "src/Auth.php",
        "uses": {
            "added": [{ "target": "src/Token.php", "symbols": ["issue"] }],
            "removed": [],
            "unchanged": []
        },
        "used_by": []
    });
    let canvas_html = link_canvas_with(&view, "", Some(&impact), &[false; 7]);
    assert!(canvas_html.contains("data-diff=\"added\""));

    let side_html = link_side_with(&view, "", Some(&impact));
    assert!(side_html.contains("Added dependency in this change"));
    assert!(side_html.contains("data-status=\"added\""));
}

#[test]
fn visibility_taxonomy_and_kind_badges_handle_properties_consts_and_languages() {
    assert_eq!(normalize_entity_kind("property"), "property");
    assert_eq!(normalize_entity_kind("field"), "field");
    assert_eq!(normalize_entity_kind("constant"), "const");
    assert_eq!(normalize_entity_kind("class_const"), "const");
    assert_eq!(normalize_entity_kind("fn"), "function");
    assert_eq!(entity_kind_badge("property"), "P");
    assert_eq!(entity_kind_badge("field"), "P");
    assert_eq!(entity_kind_badge("const"), "K");
    assert_eq!(entity_kind_badge("method"), "M");
    assert_eq!(entity_kind_badge("function"), "fn");
    assert_eq!(letter("property"), "P");
    assert_eq!(letter("field"), "P");
    assert_eq!(letter("const"), "K");
    assert_eq!(letter("method"), "m");
    assert_eq!(letter("function"), "f");

    assert_eq!(
        normalize_visibility(Some("public")),
        Some(("public", "◎ pub", "pub"))
    );
    assert_eq!(
        normalize_visibility(Some("pub")),
        Some(("public", "◎ pub", "pub"))
    );
    assert_eq!(
        normalize_visibility(Some("export")),
        Some(("public", "◎ pub", "pub"))
    );
    assert_eq!(
        normalize_visibility(Some("protected")),
        Some(("protected", "◈ prot", "prot"))
    );
    assert_eq!(
        normalize_visibility(Some("protected(set)")),
        Some(("protected", "◈ prot", "prot"))
    );
    assert_eq!(
        normalize_visibility(Some("private")),
        Some(("private", "⊗ priv", "priv"))
    );
    assert_eq!(
        normalize_visibility(Some("#private")),
        Some(("private", "⊗ priv", "priv"))
    );
    assert_eq!(
        normalize_visibility(Some("private(set)")),
        Some(("private", "⊗ priv", "priv"))
    );
    assert_eq!(
        normalize_visibility(Some("pub(crate)")),
        Some(("internal", "◉ pkg", "pkg"))
    );
    assert_eq!(
        normalize_visibility(Some("pub(super)")),
        Some(("internal", "◉ pkg", "pkg"))
    );
    assert_eq!(
        normalize_visibility(Some("pub(in crate::api)")),
        Some(("internal", "◉ pkg", "pkg"))
    );
    assert_eq!(
        normalize_visibility(Some("package")),
        Some(("internal", "◉ pkg", "pkg"))
    );

    let full_pill = visibility_pill_html(Some("pub(crate)"), false);
    assert!(full_pill.contains("data-vis=\"internal\""));
    assert!(full_pill.contains("data-compact=\"false\""));
    assert!(full_pill.contains("title=\"Visibility: pub(crate)\""));
    assert!(full_pill.contains("aria-label=\"Visibility: pub(crate)\""));
    assert!(full_pill.contains(">◉ pkg</span>"));

    let compact_pill = visibility_pill_html(Some("#private"), true);
    assert!(compact_pill.contains("data-vis=\"private\""));
    assert!(compact_pill.contains("data-compact=\"true\""));
    assert!(compact_pill.contains("aria-label=\"Visibility: #private\""));
    assert!(compact_pill.contains(">priv</span>"));

    let narrowed = vis_delta_badge_html(Some("public"), Some("protected"), true);
    assert!(narrowed.contains("data-status=\"broken_risk\""));
    assert!(narrowed.contains("title=\"public → protected\""));
    assert!(narrowed.contains(">vis!</span>"));

    let widened = vis_delta_badge_html(Some("private"), Some("public"), false);
    assert!(widened.contains("data-status=\"modified\""));
    assert!(widened.contains("title=\"private → public\""));

    let entity: CreatedEntity = serde_json::from_value(json!({
        "id": "ce1",
        "kind": "property",
        "name": "$rate",
        "visibility": "protected",
        "modifiers": ["readonly"]
    }))
    .expect("deserialize CreatedEntity with visibility and modifiers");
    assert_eq!(entity.visibility.as_deref(), Some("protected"));
    assert_eq!(entity.modifiers, vec!["readonly"]);
}

#[test]
fn impact_card_renders_member_filters_visibility_warnings_and_accessible_groups() {
    let impact = json!({
        "path": "src/Billing/TaxService.php",
        "status": "modified",
        "exists": true,
        "in_graph": true,
        "summary": {
            "uses_added": 1,
            "uses_removed": 0,
            "uses_unchanged": 1,
            "callers_at_risk": 1,
            "callers_impacted": 0,
            "added_callers": 0,
            "visibility_warnings": 2
        },
        "entities": [
            {
                "id": "e-prop",
                "kind": "property",
                "change": "visibility_changed",
                "name": "$defaultRate",
                "line": 12,
                "visibility": "private",
                "previous_visibility": "public",
                "visibility_changed": true,
                "visibility_narrowed": true
            },
            {
                "id": "e-const",
                "kind": "const",
                "change": "modified",
                "name": "MAX_RATE",
                "line": 8,
                "visibility": "public",
                "signature_changed": true
            }
        ],
        "uses": {
            "added": [
                {
                    "target": "src/Config/TaxRules.php",
                    "resolved": true,
                    "exists": true,
                    "kind": "uses",
                    "line": 22,
                    "member_refs": [
                        {
                            "name": "rateFor",
                            "kind": "method",
                            "access": "call",
                            "line": 22,
                            "visibility": "public",
                            "signature": "rateFor(string $country): float",
                            "change": "added"
                        }
                    ],
                    "target_members": [
                        {
                            "name": "currency",
                            "kind": "property",
                            "line": 5,
                            "visibility": "protected",
                            "change": "unchanged"
                        }
                    ]
                }
            ],
            "removed": [],
            "unchanged": [
                {
                    "target": "src/Shared/Clock.php",
                    "resolved": true,
                    "exists": true,
                    "kind": "calls",
                    "member_refs": [
                        {
                            "name": "now",
                            "kind": "method",
                            "access": "call",
                            "visibility": "public",
                            "change": "unchanged"
                        }
                    ],
                    "target_members": []
                }
            ]
        },
        "used_by": [
            {
                "path": "src/Billing/Checkout.php",
                "exists": true,
                "impact": "broken_risk",
                "reason": "Direct property read becomes inaccessible",
                "links": 1,
                "used_members": [
                    {
                        "name": "$defaultRate",
                        "kind": "property",
                        "access": "read",
                        "line": 48,
                        "visibility": "private",
                        "previous_visibility": "public",
                        "visibility_narrowed": true,
                        "access_risk": "inaccessible",
                        "change": "visibility_changed"
                    }
                ],
                "visibility_warnings": [
                    {
                        "member": "$defaultRate",
                        "kind": "property",
                        "visibility": "private",
                        "previous_visibility": "public",
                        "line": 48,
                        "reason": "Narrowed from public to private; Checkout reads property directly"
                    }
                ]
            }
        ]
    });

    let html = impact_side_html_with_filter(&impact, &V::Null, "modal", "all");
    assert!(!html.contains("<summary><h4>"));
    assert!(html.contains("class=\"graph-impact-group-title\""));
    assert!(html.contains("!2 visibility warnings"));
    assert!(html.contains("data-action=\"graph-map-member-filter\""));
    assert!(html.contains("class=\"graph-impact-entity-name\">$defaultRate:12</span>"));
    assert!(html.contains("class=\"graph-impact-entity-name\">MAX_RATE:8</span>"));
    assert!(html.contains("data-member-kind=\"prop\""));
    assert!(html.contains("data-member-kind=\"const\""));
    assert!(html.contains("title=\"public → private\""));
    assert!(html.contains("1 used · 1 exposed"));
    assert!(html.contains("vis risk"));
    assert!(html.contains("<details class=\"graph-impact-members\" open>"));
    assert!(html.contains("class=\"graph-vis-warning-row\""));
    assert!(html.contains("<strong class=\"graph-vis-warning-member\">$defaultRate:48</strong>"));
    assert!(html.contains("<span class=\"graph-vis-warning-detail\">Narrowed from public to private; Checkout reads property directly</span>"));
    assert!(html.contains("Checkout reads property directly"));
    assert!(html.contains("data-narrowed=\"true\""));
    assert!(html.contains("data-access=\"read\""));
    assert!(html.contains("inaccessible"));

    let used_only = impact_side_html_with_filter(&impact, &V::Null, "modal", "used");
    assert!(used_only.contains("Called / Referenced"));
    assert!(!used_only.contains("Exposed by target"));

    let exposed_only = impact_side_html_with_filter(&impact, &V::Null, "modal", "exposed");
    assert!(exposed_only.contains("Exposed by target"));
    assert!(!exposed_only.contains("Called / Referenced"));

    let risk_only = impact_side_html_with_filter(&impact, &V::Null, "modal", "risk");
    assert!(risk_only.contains("rateFor"));
    assert!(!risk_only.contains("now</span><span class=\"graph-member-access\""));

    let mut click_state = fresh("");
    let outcome = apply_click_action(
        &mut click_state,
        "graph-map-member-filter",
        &json!({ "filter": "risk" }),
        MODAL_SURFACE,
        false,
        false,
    );
    assert!(outcome.handled);
    assert!(outcome.rerender);
    assert!(!outcome.reload);
    assert_eq!(click_state["member_filter"], "risk");
}

#[test]
fn normal_graph_map_surfaces_render_visibility_and_member_disclosures() {
    let view_overview = json!({
        "project_id": "proj",
        "scope": "src/Billing/Invoice.php",
        "nodes": [
            {
                "path": "src/Billing/Invoice.php#L10:total",
                "name": "total",
                "kind": "method",
                "symbol": true,
                "inside": true,
                "visibility": "public",
                "line": 10,
                "links": 1
            },
            {
                "path": "src/Billing/Invoice.php#L4:$subtotal",
                "name": "$subtotal",
                "kind": "property",
                "symbol": true,
                "inside": true,
                "visibility": "protected",
                "line": 4,
                "links": 1
            },
            {
                "path": "src/Billing/Invoice.php#L3:TAX_RATE",
                "name": "TAX_RATE",
                "kind": "const",
                "symbol": true,
                "inside": true,
                "visibility": "pub(crate)",
                "line": 3,
                "links": 0
            }
        ],
        "edges": [],
        "frames": [],
        "declared_members": [
            {
                "name": "$subtotal",
                "kind": "property",
                "visibility": "protected",
                "line": 4,
                "signature": "protected float $subtotal"
            },
            {
                "name": "VERSION",
                "kind": "const",
                "visibility": "public",
                "line": 2
            }
        ]
    });

    let focus = LiveFocus {
        phase: "editing".to_owned(),
        summary: "Added tax property".to_owned(),
        target_folders: vec![],
        target_paths: vec!["src/Billing/Invoice.php".to_owned()],
        target_nodes: vec![],
        created_entities: vec![CreatedEntity {
            id: "ce-prop".to_owned(),
            kind: "property".to_owned(),
            name: "$taxAmount".to_owned(),
            parent: "Invoice".to_owned(),
            path: "src/Billing/Invoice.php".to_owned(),
            folder: "src/Billing".to_owned(),
            line: 6,
            change: Some("created".to_owned()),
            visibility: Some("private".to_owned()),
            modifiers: vec![],
        }],
    };
    let rendered = canvas_with_overlays(
        &view_overview,
        "session-graph",
        CanvasOverlayContext {
            live_focus: Some(&focus),
            ..Default::default()
        },
    );
    assert!(rendered.html.contains("data-visibility=\"protected\""));
    assert!(rendered.html.contains("data-visibility=\"internal\""));
    assert!(rendered.html.contains("visibility pub(crate)"));
    assert!(rendered.html.contains(">P</span>"));
    assert!(rendered.html.contains(">K</span>"));
    assert!(rendered.html.contains("data-compact=\"true\""));
    assert!(rendered.html.contains(">priv</span>"));

    let strip = focus_strip_chips_html(&focus);
    assert!(strip.contains("data-vis=\"private\""));
    assert!(strip.contains("private property $taxAmount"));

    let fs_overview = file_side(&view_overview);
    assert!(fs_overview.contains("<details class=\"graph-impact-members\" open>"));
    assert!(fs_overview.contains("File properties &amp; methods (2)"));
    assert!(fs_overview.contains("class=\"graph-map-row graph-member-row graph-member-row-btn\""));
    assert!(fs_overview.contains("data-action=\"graph-map-go\""));
    assert!(fs_overview.contains("VERSION"));

    let view_focused = json!({
        "project_id": "proj",
        "scope": "src/Billing/Invoice.php",
        "focus": {
            "path": "src/Billing/Invoice.php#L4:$subtotal",
            "file": "src/Billing/Invoice.php",
            "name": "$subtotal",
            "kind": "property",
            "owner": "Invoice",
            "symbol": true,
            "visibility": "protected",
            "line": 4,
            "out": 1,
            "in": 1,
            "uses": [
                {
                    "path": "src/Money/Amount.php",
                    "name": "Amount.php",
                    "links": 1,
                    "visibility": "public",
                    "referenced_members": [
                        { "name": "cents", "kind": "field", "access": "read", "visibility": "public" }
                    ],
                    "declared_members": [
                        { "name": "currency", "kind": "const", "visibility": "public" }
                    ]
                }
            ],
            "used_by": []
        }
    });
    let fs_focused = file_side(&view_focused);
    assert!(fs_focused.contains("Property of Invoice"));
    assert!(fs_focused.contains("class=\"graph-member-meta\""));
    assert!(fs_focused.contains("Referenced members (1)"));
    assert!(fs_focused.contains("Exposed members (1)"));
    assert!(fs_focused.contains("cents"));

    let changes = json!({
        "files": [{
            "path": "src/Billing/Invoice.php",
            "status": "modified",
            "exists": true,
            "entities": [{
                "id": "ent1",
                "kind": "const",
                "change": "modified",
                "name": "MAX_ITEMS",
                "line": 5,
                "visibility": "private",
                "previous_visibility": "public",
                "visibility_narrowed": true
            }]
        }]
    });
    let ch = changes_side_html(&changes, "all", "", "session");
    assert!(ch.contains("class=\"graph-changes-entity-kind\">K</span>"));
    assert!(ch.contains("data-vis=\"private\""));
    assert!(ch.contains(">vis!</span>"));
}

#[test]
fn link_modal_renders_property_const_letters_visibility_and_warnings() {
    let view = json!({
        "mode": "link",
        "from": { "path": "src/Billing/Checkout.php", "name": "Checkout.php", "what": "file" },
        "to": { "path": "src/Billing/Invoice.php", "name": "Invoice.php", "what": "file" },
        "users": {
            "names": 1,
            "files": 1,
            "groups": [
                {
                    "path": "src/Billing/Checkout.php",
                    "name": "Checkout.php",
                    "referenced_members": [
                        { "name": "$total", "kind": "property", "access": "read", "visibility": "private", "access_risk": "inaccessible" }
                    ],
                    "names": [
                        { "key": "src/Billing/Checkout.php#L18:apply", "name": "apply", "kind": "method", "visibility": "public", "line": 18, "links": 2 }
                    ]
                }
            ]
        },
        "used": {
            "names": 2,
            "files": 1,
            "groups": [
                {
                    "path": "src/Billing/Invoice.php",
                    "name": "Invoice.php",
                    "declared_members": [
                        { "name": "$total", "kind": "property", "visibility": "private", "line": 7 },
                        { "name": "PRECISION", "kind": "const", "visibility": "public", "line": 3 }
                    ],
                    "names": [
                        { "key": "src/Billing/Invoice.php#L7:$total", "name": "$total", "kind": "property", "visibility": "private", "line": 7, "links": 1 },
                        { "key": "src/Billing/Invoice.php#L3:PRECISION", "name": "PRECISION", "kind": "const", "visibility": "public", "line": 3, "links": 1 }
                    ]
                }
            ]
        },
        "edges": [
            {
                "from": "src/Billing/Checkout.php#L18:apply",
                "to": "src/Billing/Invoice.php#L7:$total",
                "kind": "reads",
                "access_risk": "inaccessible"
            },
            {
                "from": "src/Billing/Checkout.php#L18:apply",
                "to": "src/Billing/Invoice.php#L3:PRECISION",
                "kind": "uses_const"
            }
        ],
        "member_refs": [
            { "name": "$total", "kind": "property", "access": "read", "visibility": "private", "access_risk": "inaccessible" }
        ],
        "visibility_warnings": [
            {
                "member": "$total",
                "kind": "property",
                "visibility": "private",
                "previous_visibility": "public",
                "line": 7,
                "reason": "Property became private"
            }
        ],
        "kinds": [{ "kind": "reads", "links": 1 }, { "kind": "uses_const", "links": 1 }]
    });

    let canvas = link_canvas_with(
        &view,
        "src/Billing/Checkout.php#L18:apply",
        None,
        &[false; 7],
    );
    assert!(canvas.contains(">P</span>"));
    assert!(canvas.contains(">K</span>"));
    assert!(canvas.contains("data-visibility=\"private\""));
    assert!(canvas.contains("data-compact=\"true\""));
    assert!(canvas.contains(">priv</span>"));
    assert!(canvas.contains("public → private"));
    assert!(
        canvas.contains("risk: Property became private") || canvas.contains("risk: inaccessible")
    );
    assert!(canvas.contains("visibility private"));

    let side = link_side_with(&view, "src/Billing/Checkout.php#L18:apply", None);
    assert!(side.contains("class=\"graph-member-meta\""));
    assert!(side.contains("Uses here"));
    assert!(side.contains("data-member-kind=\"prop\""));
    assert!(side.contains("data-member-kind=\"const\""));
    assert!(side.contains(">vis!</span>"));
    assert!(side.contains("Property became private"));
    assert!(side.contains("Checkout.php (Caller)"));
    assert!(side.contains("Invoice.php (Target)"));
}

#[test]
fn member_kind_fallback_neighbor_file_focus_and_changes_warnings() {
    let prop_sym =
        json!({ "name": "rate", "kind": "name", "member_kind": "property", "owner": "TaxRules" });
    let field_sym =
        json!({ "name": "cents", "kind": "name", "member_kind": "field", "owner": "Money" });
    let const_sym =
        json!({ "name": "MAX", "kind": "name", "member_kind": "const", "owner": "TaxRules" });
    assert_eq!(what(&prop_sym), "property of TaxRules");
    assert_eq!(what(&field_sym), "field of Money");
    assert_eq!(what(&const_sym), "constant of TaxRules");

    let canvas_view = json!({
        "project_id": "proj",
        "scope": "src/Billing/TaxRules.php",
        "nodes": [
            {
                "path": "src/Billing/TaxRules.php#L4:rate",
                "name": "rate",
                "kind": "name",
                "member_kind": "property",
                "owner": "TaxRules",
                "symbol": true,
                "inside": true,
                "visibility": "protected",
                "line": 4,
                "links": 1
            },
            {
                "path": "src/Billing/TaxRules.php#L2:MAX",
                "name": "MAX",
                "kind": "name",
                "member_kind": "const",
                "owner": "TaxRules",
                "symbol": true,
                "inside": true,
                "visibility": "public",
                "line": 2,
                "links": 1
            }
        ],
        "edges": [
            { "from": "src/Billing/TaxRules.php#L4:rate", "to": "src/Billing/TaxRules.php#L2:MAX", "links": 1 }
        ],
        "frames": []
    });
    let rendered = canvas_with_overlays(&canvas_view, "graph-map", CanvasOverlayContext::default());
    assert!(rendered.html.contains(">P</span>"));
    assert!(rendered.html.contains(">K</span>"));
    assert!(rendered.html.contains("property of TaxRules"));
    assert!(rendered.html.contains("constant of TaxRules"));

    let link_view = json!({
        "mode": "link",
        "from": { "path": "src/Billing/Checkout.php", "name": "Checkout.php", "what": "file" },
        "to": { "path": "src/Billing/TaxRules.php", "name": "TaxRules.php", "what": "file" },
        "users": {
            "names": 1,
            "files": 1,
            "groups": [{
                "path": "src/Billing/Checkout.php",
                "name": "Checkout.php",
                "names": [
                    { "key": "src/Billing/Checkout.php#L10:calc", "name": "calc", "kind": "method", "visibility": "public", "line": 10, "links": 2 }
                ]
            }]
        },
        "used": {
            "names": 2,
            "files": 1,
            "groups": [{
                "path": "src/Billing/TaxRules.php",
                "name": "TaxRules.php",
                "names": [
                    { "key": "src/Billing/TaxRules.php#L4:rate", "name": "rate", "kind": "name", "member_kind": "field", "owner": "TaxRules", "visibility": "private", "line": 4, "links": 1 },
                    { "key": "src/Billing/TaxRules.php#L2:MAX", "name": "MAX", "kind": "name", "member_kind": "const", "owner": "TaxRules", "visibility": "public", "line": 2, "links": 1 }
                ]
            }]
        },
        "edges": [
            { "from": "src/Billing/Checkout.php#L10:calc", "to": "src/Billing/TaxRules.php#L4:rate", "kind": "reads" },
            { "from": "src/Billing/Checkout.php#L10:calc", "to": "src/Billing/TaxRules.php#L2:MAX", "kind": "uses_const" }
        ],
        "kinds": []
    });
    let link_canvas_html = link_canvas_with(
        &link_view,
        "src/Billing/Checkout.php#L10:calc",
        None,
        &[false; 7],
    );
    assert!(link_canvas_html.contains(">P</span>"));
    assert!(link_canvas_html.contains(">K</span>"));
    assert!(link_canvas_html.contains("field of TaxRules"));
    assert!(link_canvas_html.contains("constant of TaxRules"));

    let link_side_html = link_side_with(&link_view, "src/Billing/Checkout.php#L10:calc", None);
    assert!(link_side_html.contains("data-member-kind=\"field\""));
    assert!(link_side_html.contains("data-member-kind=\"const\""));

    let symbol_focus_view = json!({
        "project_id": "proj",
        "scope": "src/Billing/TaxRules.php",
        "focus": {
            "path": "src/Billing/TaxRules.php#L4:rate",
            "file": "src/Billing/TaxRules.php",
            "name": "rate",
            "kind": "name",
            "member_kind": "property",
            "owner": "TaxRules",
            "symbol": true,
            "visibility": "protected",
            "line": 4,
            "out": 1,
            "in": 0,
            "members": [
                { "path": "src/Billing/TaxRules.php#L2:MAX", "name": "MAX", "kind": "name", "member_kind": "const", "visibility": "public", "line": 2 }
            ],
            "uses": [
                { "path": "src/Money/Amount.php#L3:cents", "name": "cents", "file": "src/Money/Amount.php", "kind": "name", "member_kind": "field", "symbol": true, "visibility": "public", "relation": "reads" }
            ],
            "used_by": []
        }
    });
    let fs_sym = file_side(&symbol_focus_view);
    assert!(fs_sym.contains("Property of TaxRules"));
    assert!(fs_sym.contains("class=\"graph-member-meta\""));
    assert!(fs_sym.contains("data-member-kind=\"prop\""));
    assert!(fs_sym.contains("data-member-kind=\"const\""));
    assert!(fs_sym.contains("data-member-kind=\"field\""));

    let neighbor_file_view = json!({
        "project_id": "proj",
        "scope": "src/Billing/Checkout.php",
        "focus": {
            "path": "src/Billing/TaxRules.php",
            "name": "TaxRules.php",
            "symbol": false,
            "role": "used",
            "symbols": 2,
            "declared_members": [
                { "name": "rate", "kind": "property", "visibility": "private", "line": 4 }
            ],
            "referenced_members": [
                { "name": "rate", "kind": "property", "access": "read", "visibility": "private", "access_risk": "inaccessible" }
            ],
            "visibility_warnings": [
                { "member": "rate", "kind": "property", "visibility": "private", "previous_visibility": "public", "line": 4, "reason": "Narrowed visibility breaks Checkout" }
            ],
            "uses": [],
            "used_by": []
        }
    });
    let fs_neighbor = file_side(&neighbor_file_view);
    assert!(fs_neighbor.contains("class=\"graph-vis-warning-box\""));
    assert!(fs_neighbor.contains("Narrowed visibility breaks Checkout"));
    assert!(fs_neighbor.contains("<details class=\"graph-impact-members\" open>"));
    assert!(fs_neighbor.contains("Referenced members (1)"));
    assert!(fs_neighbor.contains("Declared members (1)"));

    let changes = json!({
        "total_files": 1,
        "modified_entities": 1,
        "files": [{
            "path": "src/Billing/TaxRules.php",
            "status": "modified",
            "exists": true,
            "summary": {
                "uses_added": 0,
                "uses_removed": 0,
                "callers_at_risk": 1,
                "visibility_warnings": 2
            },
            "entities": []
        }]
    });
    let ch = changes_side_html(&changes, "all", "", "session");
    assert!(ch.contains("!2 visibility warnings"));
}

#[test]
fn scaled_supports_zoom_in_and_out_without_negative_zero() {
    assert_eq!(
        scaled((700.0, 300.0), 1.5),
        "width:700px;height:300px;transform:scale(1.5000);transform-origin:0 0;margin:0 350px 150px 0"
    );
    assert_eq!(
        scaled((0.0, 400.0), 0.5),
        "width:0px;height:400px;transform:scale(0.5000);transform-origin:0 0;margin:0 0px -200px 0"
    );
}

#[test]
fn wheel_zoom_factor_handles_directions_modes_and_horizontal_swipes() {
    assert_eq!(wheel_zoom_factor(0.0, 0), None);
    assert_eq!(wheel_zoom_factor(f64::NAN, 0), None);
    assert_eq!(wheel_zoom_factor(f64::INFINITY, 0), None);

    let up = wheel_zoom_factor(-100.0, 0).expect("wheel up factor");
    let down = wheel_zoom_factor(100.0, 0).expect("wheel down factor");
    assert!(up > 1.15 && up < 1.35);
    assert!((up * down - 1.0).abs() < 1e-9);

    let line_up = wheel_zoom_factor(-3.0, 1).expect("line wheel up");
    let page_up = wheel_zoom_factor(-1.0, 2).expect("page wheel up");
    assert!(line_up > 1.0);
    assert!(page_up > 1.0);

    let clamped = wheel_zoom_factor(-5000.0, 0).expect("clamped zoom");
    assert!(clamped <= 2.0);
}

#[test]
fn scroll_after_zoom_keeps_cursor_anchor_stationary() {
    let (left, top) = scroll_after_zoom(50.0, 40.0, 1.0, 2.0, (100.0, 80.0));
    assert!((left - 200.0).abs() < 1e-9);
    assert!((top - 160.0).abs() < 1e-9);
    assert!(((left + 100.0) / 2.0 - (50.0 + 100.0)).abs() < 1e-9);

    let (clamped_l, clamped_t) = scroll_after_zoom(10.0, 10.0, 2.0, 0.25, (200.0, 200.0));
    assert_eq!(clamped_l, 0.0);
    assert_eq!(clamped_t, 0.0);
}

#[test]
fn normalize_edge_arrow_kind_maps_all_canonical_aliases() {
    for (raw, expected) in [
        ("calls", "calls"),
        ("call", "calls"),
        ("instantiates", "calls"),
        ("indirect_call", "calls"),
        ("CALL", "calls"),
        ("unknown_rel", "calls"),
        ("", "calls"),
        ("imports", "imports"),
        ("imports_from", "imports"),
        ("includes", "imports"),
        ("requires", "imports"),
        ("re_exports", "imports"),
        ("crate_depends_on", "imports"),
        ("depends_on", "imports"),
        ("Imports-From", "imports"),
        ("extends", "extends"),
        ("inherits", "extends"),
        ("implements", "implements"),
        ("uses_trait", "implements"),
        ("trait", "implements"),
        ("writes", "writes"),
        ("mutates", "writes"),
        ("sets", "writes"),
        ("reads", "reads"),
        ("static_read", "reads"),
        ("uses_const", "reads"),
        ("const", "reads"),
        ("references_constant", "reads"),
        ("uses_static_prop", "reads"),
        ("uses", "uses"),
        ("references", "uses"),
        ("type_hint", "uses"),
        ("type hint", "uses"),
        ("uses_component", "uses"),
        ("binds_method", "uses"),
        ("bound_to", "uses"),
    ] {
        assert_eq!(normalize_edge_arrow_kind(raw), expected, "raw={raw:?}");
    }
}

#[test]
fn arrow_kind_for_edge_precedence_and_fallbacks() {
    let extends_wins = json!({
        "kinds": [
            {"kind": "calls", "links": 10},
            {"kind": "implements", "links": 5},
            {"kind": "extends", "links": 1}
        ]
    });
    assert_eq!(arrow_kind_for_edge(&extends_wins), "extends");

    let implements_wins = json!({
        "kinds": [
            {"kind": "writes", "links": 8},
            {"kind": "implements", "links": 1}
        ]
    });
    assert_eq!(arrow_kind_for_edge(&implements_wins), "implements");

    let highest_count = json!({
        "kinds": [
            {"kind": "imports", "links": 9},
            {"kind": "uses", "links": 1},
            {"kind": "calls", "links": 3},
            {"kind": "reads", "links": 4}
        ]
    });
    assert_eq!(arrow_kind_for_edge(&highest_count), "reads");

    let tie_writes = json!({
        "kinds": [
            {"kind": "calls", "links": 2},
            {"kind": "writes", "links": 2},
            {"kind": "reads", "links": 2}
        ]
    });
    assert_eq!(arrow_kind_for_edge(&tie_writes), "writes");

    let tie_calls = json!({
        "kinds": [
            {"kind": "reads", "links": 2},
            {"kind": "calls", "links": 2},
            {"kind": "uses", "links": 2}
        ]
    });
    assert_eq!(arrow_kind_for_edge(&tie_calls), "calls");

    let tie_reads = json!({
        "kinds": [
            {"kind": "uses", "links": 2},
            {"kind": "reads", "links": 2}
        ]
    });
    assert_eq!(arrow_kind_for_edge(&tie_reads), "reads");

    let import_plus_call = json!({
        "kinds": [
            {"kind": "imports", "links": 1},
            {"kind": "calls", "links": 1}
        ]
    });
    assert_eq!(arrow_kind_for_edge(&import_plus_call), "calls");

    let imports_alone = json!({
        "kinds": [{"kind": "imports_from", "links": 2}]
    });
    assert_eq!(arrow_kind_for_edge(&imports_alone), "imports");

    let empty_kinds_uses_kind = json!({"kinds": [], "kind": "uses_const"});
    assert_eq!(arrow_kind_for_edge(&empty_kinds_uses_kind), "reads");

    let empty_kinds_uses_tie = json!({"kinds": [], "tie": "implements and holds"});
    assert_eq!(arrow_kind_for_edge(&empty_kinds_uses_tie), "implements");

    assert_eq!(arrow_kind_for_edge(&json!({})), "calls");
}

#[test]
fn canvas_with_overlays_renders_distinct_markers_attributes_and_breakdowns() {
    let view = json!({
        "scope": "src",
        "architecture": {"rules": []},
        "nodes": [
            {"path": "src/A.php", "name": "A", "folder": false},
            {"path": "src/B.php", "name": "B", "folder": false},
            {"path": "src/C.php", "name": "C", "folder": false},
            {"path": "src/D.php", "name": "D", "folder": false},
            {"path": "src/E.php", "name": "E", "folder": false},
            {"path": "src/F.php", "name": "F", "folder": false},
            {"path": "src/G.php", "name": "G", "folder": false},
            {"path": "src/H.php", "name": "H", "folder": false}
        ],
        "edges": [
            {
                "from": "src/A.php",
                "to": "src/B.php",
                "links": 3,
                "guessed": 0,
                "kinds": [{"kind": "calls", "links": 2}, {"kind": "imports", "links": 1}]
            },
            {
                "from": "src/B.php",
                "to": "src/C.php",
                "links": 2,
                "guessed": 0,
                "kinds": [{"kind": "calls", "links": 1}, {"kind": "instantiates", "links": 1}]
            },
            {
                "from": "src/C.php",
                "to": "src/D.php",
                "links": 1,
                "guessed": 0,
                "kinds": [{"kind": "imports_from", "links": 1}]
            },
            {
                "from": "src/D.php",
                "to": "src/E.php",
                "links": 1,
                "guessed": 0,
                "kinds": [{"kind": "extends", "links": 1}]
            },
            {
                "from": "src/E.php",
                "to": "src/F.php",
                "links": 1,
                "guessed": 0,
                "kinds": [{"kind": "implements", "links": 1}]
            },
            {
                "from": "src/F.php",
                "to": "src/G.php",
                "links": 1,
                "guessed": 0,
                "kinds": [{"kind": "writes", "links": 1}]
            },
            {
                "from": "src/G.php",
                "to": "src/H.php",
                "links": 1,
                "guessed": 0,
                "kinds": [{"kind": "reads", "links": 1}]
            },
            {
                "from": "src/H.php",
                "to": "src/A.php",
                "links": 1,
                "guessed": 0,
                "kinds": [{"kind": "uses", "links": 1}]
            },
            {
                "from": "src/H.php",
                "to": "src/B.php",
                "links": 1,
                "guessed": 0,
                "architecture_violation": 1,
                "kinds": [{"kind": "extends", "links": 1}]
            }
        ]
    });

    let html = canvas_with(&view, "session-graph", None).html;
    assert_eq!(html.matches("<marker ").count(), 10);
    for suffix in [
        "",
        "-imports",
        "-extends",
        "-implements",
        "-writes",
        "-reads",
        "-uses",
        "-against",
    ] {
        assert!(
            html.contains(&format!("<marker id=\"session-graph-arrow{suffix}\"")),
            "missing marker suffix {suffix} in {html}"
        );
    }
    assert!(html.contains("<path d=\"M1 1 L7.5 4 L1 7\"/>"));
    assert!(html.contains("<path d=\"M0.8 0.8 L7.8 4 L0.8 7.2 Z\"/>"));
    assert!(html.contains("<path d=\"M0.5 4 L4 1 L7.5 4 L4 7 Z\"/>"));
    assert!(html.contains("<path d=\"M0.8 4 L4 1.2 L7.2 4 L4 6.8 Z\"/>"));
    assert!(html.contains("<path d=\"M0.5 1 L7.8 4 L0.5 7 L2.4 4 z\"/>"));

    // Multi-kind vs alias-only edges
    assert!(html.contains(
        "data-ends=\"0 1\" d=\"M"
    ) && html.contains("data-edge-kind=\"calls\" data-kinds=\"calls imports\" data-edge-multi=\"true\" stroke=\"url(#session-graph-flow-"));
    // A link that holds several kinds leaves by a ring, one of a single
    // kind by a dot.
    assert_eq!(
        html.matches("marker-start=\"url(#session-graph-port-multi)\"")
            .count(),
        1
    );
    assert_eq!(
        html.matches("marker-start=\"url(#session-graph-port)\"")
            .count(),
        8
    );
    assert!(html.contains(
        "marker-end=\"url(#session-graph-arrow)\"><title>A uses B · 3 links · calls 2, imports 1</title>"
    ));
    assert!(html.contains(
        "data-action=\"graph-map-link\" data-payload=\"{&quot;from&quot;:&quot;src/A.php&quot;,&quot;to&quot;:&quot;src/B.php&quot;}\" role=\"button\" tabindex=\"0\" focusable=\"true\"><title>A uses B · calls 2, imports 1 · select to see what makes up this link</title>"
    ));
    // calls + instantiates collapse into 1 canonical kind -> not multi
    assert!(html.contains("<title>B uses C · 2 links · calls 1, instantiates 1</title>"));
    assert_eq!(html.matches("data-edge-multi=\"true\"").count(), 1);

    for kind in [
        "imports",
        "extends",
        "implements",
        "writes",
        "reads",
        "uses",
    ] {
        assert!(html.contains(&format!("data-edge-kind=\"{kind}\"")));
        assert!(html.contains(&format!("marker-end=\"url(#session-graph-arrow-{kind})\"")));
    }
    // Violation keeps against arrowhead while reporting normalized semantic kind.
    assert!(html.contains("graph-map-edge-against\" data-ends=\"7 1\""));
    assert!(html.contains("marker-end=\"url(#session-graph-arrow-against)\""));
}

#[test]
fn link_canvas_with_renders_kind_markers_and_attributes() {
    let view = json!({
        "from": {"path": "src/A.php", "name": "A.php"},
        "to": {"path": "src/B.php", "name": "B.php"},
        "users": {"groups": [{
            "path": "src/A.php",
            "name": "A.php",
            "names": [
                {"key": "src/A.php#L10:.fnA()", "name": "fnA", "kind": "method", "line": 10, "links": 3},
                {"key": "src/A.php#L2:A", "name": "A", "kind": "type", "line": 2, "links": 4}
            ]
        }]},
        "used": {"groups": [{
            "path": "src/B.php",
            "name": "B.php",
            "names": [
                {"key": "src/B.php#L1:helper()", "name": "helper", "kind": "function", "line": 1, "links": 2},
                {"key": "src/B.php#L5:Base", "name": "Base", "kind": "type", "line": 5, "links": 1},
                {"key": "src/B.php#L9:T", "name": "T", "kind": "type", "line": 9, "links": 1},
                {"key": "src/B.php#L12:$prop", "name": "prop", "kind": "property", "line": 12, "links": 1},
                {"key": "src/B.php#L14:CONST_X", "name": "CONST_X", "kind": "const", "line": 14, "links": 1},
                {"key": "src/B.php#L18:TypeRef", "name": "TypeRef", "kind": "type", "line": 18, "links": 1}
            ]
        }]},
        "edges": [
            {"from": "src/A.php#L10:.fnA()", "to": "src/B.php#L1:helper()", "kind": "calls", "guessed": false},
            {"from": "src/A.php#L2:A", "to": "src/B.php#L1:helper()", "kind": "imports_from", "guessed": false},
            {"from": "src/A.php#L2:A", "to": "src/B.php#L5:Base", "kind": "extends", "guessed": false},
            {"from": "src/A.php#L2:A", "to": "src/B.php#L9:T", "kind": "uses_trait", "guessed": false},
            {"from": "src/A.php#L10:.fnA()", "to": "src/B.php#L12:$prop", "kind": "writes", "guessed": false},
            {"from": "src/A.php#L10:.fnA()", "to": "src/B.php#L14:CONST_X", "kind": "uses_const", "guessed": false},
            {"from": "src/A.php#L2:A", "to": "src/B.php#L18:TypeRef", "kind": "references", "guessed": false}
        ]
    });

    let html = link_canvas_with(&view, "", None, &[false; 7]);
    assert_eq!(html.matches("<marker ").count(), 9);
    assert!(!html.contains("graph-link-arrow-against"));
    assert!(html.contains("data-edge-kind=\"calls\""));
    assert!(html.contains("marker-end=\"url(#graph-link-arrow)\""));
    for kind in [
        "imports",
        "extends",
        "implements",
        "writes",
        "reads",
        "uses",
    ] {
        assert!(html.contains(&format!("<marker id=\"graph-link-arrow-{kind}\"")));
        assert!(html.contains(&format!("data-edge-kind=\"{kind}\"")));
        assert!(html.contains(&format!("marker-end=\"url(#graph-link-arrow-{kind})\"")));
    }
    assert!(html.contains("<title>A uses helper · imports from</title>"));
    assert!(html.contains("<title>fnA uses CONST_X · uses const</title>"));
}

#[test]
fn arrow_kinds_legend_html_and_existing_legend_endings_preserved() {
    let html = arrow_kinds_legend_html();
    assert_eq!(html.matches("class=\"graph-arrow-swatch\"").count(), 7);
    // The seven arrows, and the hollow line of what was only inferred.
    assert_eq!(html.matches("aria-hidden=\"true\"").count(), 8);
    assert!(!html.contains(" id=\""));
    for (kind, label) in [
        ("calls", "Calls"),
        ("imports", "Imports"),
        ("extends", "Extends"),
        ("implements", "Implements"),
        ("writes", "Writes"),
        ("reads", "Reads"),
        ("uses", "Uses"),
    ] {
        assert!(html.contains(&format!("data-edge-kind=\"{kind}\"")));
        assert!(html.contains(&format!(
            "<span class=\"graph-arrow-legend-name\">{label}</span>"
        )));
    }
    assert!(html.contains("stroke-dasharray=\"8 4\""));
    assert!(html.contains("stroke-dasharray=\"6 3 2 3\""));
    assert!(html.contains("stroke-dasharray=\"2 3\""));

    let view = json!({"scope": "src", "nodes": [], "edges": []});
    assert!(
        legend(&view, false).ends_with("Dotted: against the main direction, closing a cycle (↻).")
    );
}

#[test]
fn scoped_url_builders_cover_single_project_and_system_service_endpoints() {
    let single = GraphTargetScope::single("billing-repo");
    let sys = GraphTargetScope::SystemService {
        project_id: "monorepo-root".to_owned(),
        config_path: ".harness/system.yaml".to_owned(),
        service_id: "packages/billing-api".to_owned(),
        services: vec!["packages/billing-api".to_owned(), "web-gateway".to_owned()],
        run_id: Some("sysrun-42".to_owned()),
        run_title: Some("Split invoice schema".to_owned()),
        launches: vec![json!({"launch": 1}), json!({"launch": 2})],
    };

    let mut st = fresh("");
    st["scope"] = json!("src/Domain");
    st["target"] = json!("sysrun:sysrun-42");
    st["launch"] = json!("2");
    st["compare_launch"] = json!("1");
    st["agent"] = json!("service-packages/billing-api");
    st["focus_session"] = json!("sess-live-9");
    st["whole"] = json!(true);
    st["notests"] = json!(true);
    st["certain"] = json!(true);

    let sys_map = build_scoped_map_path(&sys, &st, true);
    assert!(sys_map.starts_with(
        "/api/systems/service-graph/map?project_id=monorepo-root&config_path=.harness%2Fsystem.yaml&service=packages%2Fbilling-api&"
    ));
    assert!(sys_map.contains("scope=src%2FDomain"));
    assert!(sys_map.contains("focus_session=sess-live-9"));
    assert!(sys_map.contains("target=sysrun%3Asysrun-42"));
    assert!(sys_map.contains("run_id=sysrun-42"));
    assert!(sys_map.contains("launch=2"));
    assert!(sys_map.contains("compare_launch=1"));
    assert!(sys_map.contains("agent=service-packages%2Fbilling-api"));
    assert!(sys_map.contains("whole=1"));
    assert!(sys_map.contains("notests=1"));
    assert!(sys_map.contains("certain=1"));

    let single_map = build_scoped_map_path(&single, &fresh("work"), false);
    assert!(single_map.starts_with("/api/projects/billing-repo/graph/map?"));
    assert!(!single_map.contains("agent="));
    assert!(!single_map.contains("run_id="));

    let sys_changes = build_scoped_changes_path(&sys, "sysrun:sysrun-42", "sess-live-9", true);
    assert!(sys_changes.starts_with(
        "/api/systems/service-graph/changes?project_id=monorepo-root&config_path=.harness%2Fsystem.yaml&service=packages%2Fbilling-api&"
    ));
    assert!(sys_changes.contains("target=sysrun%3Asysrun-42"));
    assert!(sys_changes.contains("run_id=sysrun-42"));
    assert!(sys_changes.contains("focus_session=sess-live-9"));
    assert!(sys_changes.contains("notests=1"));

    let sys_impact = build_scoped_file_impact_path(
        &sys,
        "./src/Domain/Invoice.php#L12:Invoice",
        "sysrun:sysrun-42",
        "sess-live-9",
        true,
    );
    assert!(sys_impact.starts_with(
        "/api/systems/service-graph/file-impact?project_id=monorepo-root&config_path=.harness%2Fsystem.yaml&service=packages%2Fbilling-api&"
    ));
    assert!(sys_impact.contains("path=src%2FDomain%2FInvoice.php"));
    assert!(sys_impact.contains("run_id=sysrun-42"));
    assert!(sys_impact.contains("certain=1"));

    let sys_find = build_scoped_find_path(&sys, &st, " Invoice ").expect("find path");
    assert!(sys_find.starts_with(
        "/api/systems/service-graph/find?project_id=monorepo-root&config_path=.harness%2Fsystem.yaml&service=packages%2Fbilling-api&"
    ));
    assert!(sys_find.contains("query=Invoice"));
    assert!(sys_find.contains("run_id=sysrun-42"));
    let mut base_st = st.clone();
    base_st["cycle_base"] = json!({"place": "src/A.php", "revision": "abc1234"});
    assert_eq!(build_scoped_find_path(&sys, &base_st, "Invoice"), None);

    let sys_link = build_scoped_link_request(
        &sys,
        "src/A.php",
        "src/B.php",
        &json!({
            "target": "sysrun:sysrun-42",
            "launch": "2",
            "agent": "contracts",
            "certain": true,
            "whole": true,
        }),
    );
    assert!(sys_link.starts_with(
        "/api/systems/service-graph/map?project_id=monorepo-root&config_path=.harness%2Fsystem.yaml&service=packages%2Fbilling-api&"
    ));
    assert!(sys_link.contains("from=src%2FA.php&to=src%2FB.php"));
    assert!(sys_link.contains("run_id=sysrun-42"));
    assert!(sys_link.contains("agent=contracts"));

    assert_eq!(
        build_scoped_picker_path(
            &sys.with_service("__system__"),
            "commits",
            &json!({"scope": "*", "query": "feat"}),
            20
        ),
        "/api/systems/service-graph/commits?project_id=monorepo-root&config_path=.harness%2Fsystem.yaml&service=__system__&scope=*&query=feat&skip=20"
    );
    assert_eq!(
        build_scoped_picker_path(
            &sys,
            "pulls",
            &json!({"state": "open", "query": "invoice"}),
            0
        ),
        "/api/systems/service-graph/pulls?project_id=monorepo-root&config_path=.harness%2Fsystem.yaml&service=packages%2Fbilling-api&state=open&query=invoice"
    );
    assert_eq!(
        build_scoped_picker_path(&sys, "branches", &json!({}), 0),
        "/api/systems/service-graph/branches?project_id=monorepo-root&config_path=.harness%2Fsystem.yaml&service=packages%2Fbilling-api"
    );
}

#[test]
fn system_header_switcher_sysrun_target_and_agent_options_render_cleanly() {
    assert_eq!(
        service_switcher_options_html(&GraphTargetScope::single("proj")),
        ""
    );
    let scope = GraphTargetScope::SystemService {
        project_id: "root".to_owned(),
        config_path: "system.yaml".to_owned(),
        service_id: "billing".to_owned(),
        services: vec![
            "__system__".to_owned(),
            "auth".to_owned(),
            "billing".to_owned(),
            "auth".to_owned(),
        ],
        run_id: Some("run-77".to_owned()),
        run_title: Some("Contract v2 rollout".to_owned()),
        launches: Vec::new(),
    };
    let switcher = service_switcher_options_html(&scope);
    assert!(switcher.contains("<option value=\"__topology__\">System topology</option>"));
    assert!(switcher.contains("<option value=\"__system__\">__system__ (System root)</option>"));
    assert!(switcher.contains("<option value=\"auth\">Service: auth</option>"));
    assert!(switcher.contains("<option value=\"billing\" selected>Service: billing</option>"));
    assert_eq!(switcher.matches("value=\"auth\"").count(), 1);

    let base_targets = targets(&json!({"git": true, "branch": "main"}), "sysrun:run-77");
    let enriched = with_system_run(
        base_targets,
        "sysrun:run-77",
        Some("run-77"),
        Some("Contract v2 rollout"),
    );
    assert!(enriched.contains(
        "<option value=\"sysrun:run-77\" selected>System change: Contract v2 rollout</option>"
    ));
    assert_eq!(enriched.matches("value=\"sysrun:run-77\"").count(), 1);

    let whole_targets = with_system_run(
        targets(&json!({"git": true, "branch": "main"}), ""),
        "",
        Some("run-77"),
        Some("Contract v2 rollout"),
    );
    assert!(whole_targets
        .contains("<option value=\"sysrun:run-77\">System change: Contract v2 rollout</option>"));

    let chosen_line = chosen(&json!({
        "chosen": {
            "kind": "sysrun",
            "title": "Contract v2 rollout",
            "head": {"name": "worktree", "commit": "abcdef123456"},
            "base": {"name": "main", "commit": "1234567890ab"}
        }
    }));
    assert!(chosen_line.contains("Contract v2 rollout"));

    let svc_agents = agent_options_html(
        "billing",
        &json!({"agents": ["contracts", "service-billing", "migration-audit", "verify"]}),
        "service-billing",
    );
    assert!(svc_agents.contains("<option value=\"\">All agents in launch</option>"));
    assert!(svc_agents.contains("<option value=\"contracts\">contracts</option>"));
    assert!(
        svc_agents.contains("<option value=\"service-billing\" selected>service-billing</option>")
    );
    assert!(svc_agents.contains("<option value=\"migration-audit\">migration-audit</option>"));
    assert!(svc_agents.ends_with("<option value=\"verify\">verify</option>"));

    let root_agents = agent_options_html("__system__", &json!({}), "verify");
    assert!(!root_agents.contains("service-__system__"));
    assert!(root_agents.contains("<option value=\"verify\" selected>verify</option>"));
}

#[test]
fn cross_service_unopened_and_contract_consumers_render_service_switch_rows() {
    let run_view = json!({
        "run": {
            "reach": {
                "users": 1,
                "unopened": 1,
                "rows": [
                    {"path": "src/LocalCaller.php", "name": "LocalCaller.php", "uses": 2}
                ],
                "cross_service_unopened": [
                    {
                        "service": "checkout-web",
                        "path": "src/Http/BillingClient.php",
                        "contract": "POST /v1/invoices",
                        "kind": "openapi"
                    }
                ],
                "cross_service_more": 2
            }
        }
    });
    let unopened_markup = unopened(&run_view);
    assert!(unopened_markup.contains("Use the change, not opened by the run"));
    assert!(unopened_markup.contains("Cross-service consumers, not opened by the run"));
    assert!(unopened_markup.contains("data-cross-service=\"true\""));
    assert!(unopened_markup.contains("data-action=\"graph-map-switch-service-file\""));
    assert!(unopened_markup.contains("checkout-web · BillingClient.php"));
    assert!(unopened_markup.contains("POST /v1/invoices"));
    assert!(unopened_markup.contains("and 2 more cross-service consumers"));

    let comparing_view = json!({
        "run": {
            "comparison": {"base_launch": 1},
            "reach": run_view["run"]["reach"].clone()
        }
    });
    assert_eq!(cross_service_unopened_html(&comparing_view), "");

    let impact = json!({
        "path": "src/Http/InvoiceController.php",
        "status": "modified",
        "summary": {"callers_total": 0, "broken_risk_count": 0},
        "contract_consumers": [
            {
                "service": "checkout-web",
                "path": "src/Http/BillingClient.php",
                "contract": "POST /v1/invoices",
                "kind": "http",
                "impact": "broken_risk",
                "opened": false,
                "reason": "Response field `total_cents` renamed"
            }
        ]
    });
    let side_html = impact_side_html_with_filter(&impact, &V::Null, "modal", "all");
    assert!(side_html.contains("data-contract-consumers=\"true\""));
    assert!(side_html.contains("Cross-service contract consumers"));
    assert!(side_html.contains("data-action=\"graph-map-switch-service-file\""));
    assert!(side_html.contains("checkout-web · src/Http/BillingClient.php"));
    assert!(side_html.contains(">Broken risk</span>"));
    assert!(side_html.contains("not opened in run"));
    assert!(side_html.contains("Response field `total_cents` renamed"));
}

#[test]
fn append_system_task_draft_and_system_surface_live_focus_overlays() {
    let first = append_system_task_draft("", "billing", "- Focus: src/Invoice.php");
    assert_eq!(
        first,
        "### Code Map Context (service `billing`)\n- Focus: src/Invoice.php"
    );
    // Duplicate attachment is idempotent.
    assert_eq!(
        append_system_task_draft(&first, "billing", "- Focus: src/Invoice.php"),
        first
    );
    let second = append_system_task_draft(
        "Coordinate schema rollout.",
        "__system__",
        "- Root config: docker-compose.yml",
    );
    assert_eq!(
        second,
        "Coordinate schema rollout.\n\n### Code Map Context (__system__ (root workspace))\n- Root config: docker-compose.yml"
    );

    let view = json!({
        "scope": "",
        "nodes": [
            {"path": "src/Invoice.php", "name": "Invoice.php", "what": "file"}
        ],
        "edges": []
    });
    let focus = LiveFocus {
        phase: "editing".to_owned(),
        summary: "Updating Invoice".to_owned(),
        target_folders: Vec::new(),
        target_paths: vec!["src/Invoice.php".to_owned()],
        target_nodes: Vec::new(),
        created_entities: vec![CreatedEntity {
            id: "src/Invoice.php::issue".to_owned(),
            kind: "method".to_owned(),
            name: "issue".to_owned(),
            parent: "Invoice".to_owned(),
            path: "src/Invoice.php".to_owned(),
            folder: "src".to_owned(),
            line: 44,
            change: Some("created".to_owned()),
            visibility: Some("public".to_owned()),
            modifiers: Vec::new(),
        }],
    };

    let member_canvas = canvas_with(&view, "system-member-graph", Some(&focus));
    assert!(member_canvas
        .html
        .contains("data-action=\"system-member-graph-entity\""));
    assert!(member_canvas.html.contains("is-blinking"));
    let member_chips = focus_strip_chips_html_for_surface(&focus, "system-member-graph");
    assert!(member_chips.contains("data-action=\"system-member-graph-focus-jump\""));

    let change_canvas = canvas_with(&view, "system-change-graph", Some(&focus));
    assert!(change_canvas
        .html
        .contains("data-action=\"system-change-graph-entity\""));
    let change_chips = focus_strip_chips_html_for_surface(&focus, "system-change-graph");
    assert!(change_chips.contains("data-action=\"system-change-graph-focus-jump\""));

    let session_canvas = canvas_with(&view, "session-graph", Some(&focus));
    assert!(session_canvas
        .html
        .contains("data-action=\"session-graph-entity\""));
    let session_chips = focus_strip_chips_html(&focus);
    assert!(session_chips.contains("data-action=\"session-graph-focus-jump\""));
}

#[test]
fn backend_system_graph_impact_and_system_run_launch_schemas_render_cleanly() {
    let backend_impact = json!({
        "path": "src/Orders/CancelOrderHandler.php",
        "status": "modified",
        "summary": {"callers_total": 0, "broken_risk_count": 1},
        "contract_consumers": [
            {
                "provider": "orders",
                "consumer": "payments",
                "contract": "order.cancelled",
                "kind": "event",
                "consumer_version": "1.0",
                "provider_version": "2.0",
                "provenance": "verified",
                "broken_risk": true,
                "risk_reason": "Consumer expects v1.0, provider emits v2.0",
                "consumer_sources": [
                    "src/Listeners/RefundOnCancel.php",
                    "src/Subscribers/OrderSubscriber.php"
                ],
                "unopened_sources": ["src/Listeners/RefundOnCancel.php"]
            }
        ]
    });
    let html = contract_consumers_html(&backend_impact, "modal");
    assert!(html.contains("data-service=\"payments\""));
    assert!(html.contains("data-path=\"src/Listeners/RefundOnCancel.php\""));
    assert!(html.contains("payments · src/Listeners/RefundOnCancel.php"));
    assert!(html.contains(">Broken risk</span>"));
    assert!(html.contains("contract: order.cancelled"));
    assert!(html.contains("v1.0"));
    assert!(html.contains("not opened in run"));
    assert!(html.contains("src/Listeners/RefundOnCancel.php, src/Subscribers/OrderSubscriber.php"));
    assert!(html.contains("Consumer expects v1.0, provider emits v2.0"));

    let backend_unopened = json!({
        "run": {
            "cross_service_unopened": [
                {
                    "consumer": "inventory",
                    "contract": "order.cancelled",
                    "kind": "event",
                    "unopened_sources": ["src/Handlers/ReleaseStock.php"]
                }
            ]
        }
    });
    let unopened_html = cross_service_unopened_html(&backend_unopened);
    assert!(unopened_html.contains("inventory · ReleaseStock.php"));
    assert!(unopened_html.contains("Open src/Handlers/ReleaseStock.php in service inventory"));

    let sys_run = json!({
        "launch": "sess-launch-2",
        "launches": [
            {
                "session_id": "sess-launch-1",
                "launch_number": 1,
                "status": "failed",
                "created_at": "2026-11-15T19:10:00Z"
            },
            {
                "session_id": "sess-launch-2",
                "launch_number": 2,
                "status": "completed",
                "created_at": "2026-11-15T20:45:30Z"
            }
        ]
    });
    let primary_opts = launch_options(&sys_run, false);
    assert!(primary_opts
        .contains("<option value=\"sess-launch-1\">Launch 1 · 2026-11-15 19:10 UTC</option>"));
    assert!(primary_opts
        .contains("<option value=\"sess-launch-2\">Launch 2 · 2026-11-15 20:45 UTC</option>"));

    let compare_opts = launch_options(&sys_run, true);
    assert!(compare_opts
        .contains("<option value=\"sess-launch-1\">Launch 1 · 2026-11-15 19:10 UTC</option>"));
    assert!(!compare_opts.contains("value=\"sess-launch-2\""));
}

/// Four files in a row and one link that passes two of them.
fn kinds_view() -> V {
    json!({
        "scope": "src",
        "nodes": [
            {"path": "src/A.php", "name": "A", "folder": false},
            {"path": "src/B.php", "name": "B", "folder": false},
            {"path": "src/C.php", "name": "C", "folder": false},
            {"path": "src/D.php", "name": "D", "folder": false}
        ],
        "edges": [
            {"from": "src/A.php", "to": "src/B.php", "links": 4, "guessed": 0,
             "kinds": [{"kind": "calls", "links": 3}, {"kind": "imports", "links": 1}]},
            {"from": "src/B.php", "to": "src/C.php", "links": 1, "guessed": 1,
             "kinds": [{"kind": "imports_from", "links": 1}]},
            {"from": "src/C.php", "to": "src/D.php", "links": 2, "guessed": 0,
             "kinds": [{"kind": "references", "links": 2}, {"kind": "writes", "links": 3, "by": "source"}]},
            {"from": "src/A.php", "to": "src/D.php", "links": 1, "guessed": 0,
             "kinds": [{"kind": "inherits", "links": 1}]}
        ],
        "focus": null
    })
}
fn off(kinds: &[&str]) -> KindsOff {
    kinds_off(&json!({"links_off": kinds}))
}
fn drawn_with(view: &V, links_off: KindsOff) -> String {
    canvas_with_overlays(
        view,
        "graph-map",
        CanvasOverlayContext {
            links_off,
            ..Default::default()
        },
    )
    .html
}

#[test]
fn a_link_holds_its_kinds_and_is_drawn_as_the_strongest_that_is_shown() {
    let view = kinds_view();
    let kinds = |at: usize| EdgeKinds::of(&view["edges"][at]);
    assert_eq!(kinds(0).held(), "calls imports");
    assert_eq!(kinds(2).held(), "writes uses");
    let all = [false; 7];
    assert_eq!(kinds(0).primary(&all), Some("calls"));
    assert!(kinds(0).multi(&all));
    // With its calls switched off the link is an import, of one kind.
    let no_calls = off(&["calls"]);
    assert_eq!(kinds(0).primary(&no_calls), Some("imports"));
    assert!(!kinds(0).multi(&no_calls));
    // With every kind it holds switched off it is not drawn.
    assert_eq!(kinds(0).primary(&off(&["calls", "imports"])), None);
    assert_eq!(kinds(1).primary(&off(&["imports"])), None);
    // What the source told counts like what the graph told.
    assert_eq!(kinds(2).primary(&all), Some("writes"));
    assert_eq!(kinds(2).primary(&off(&["writes"])), Some("uses"));
    // A link that says nothing of its kind is a call, as it is drawn.
    let plain = EdgeKinds::of(&json!({"from": "a", "to": "b", "links": 2}));
    assert_eq!(plain.held(), "calls");
    assert_eq!(plain.primary(&off(&["calls"])), None);
    // Names that are no kind switch nothing off.
    assert_eq!(off(&["nothing", "calls"]), off(&["calls"]));
}

#[test]
fn a_hidden_link_keeps_its_place_and_is_neither_drawn_nor_hit() {
    let view = kinds_view();
    let shown = drawn_with(&view, [false; 7]);
    assert_eq!(shown.matches("data-kind-hidden").count(), 0);
    let html = drawn_with(&view, off(&["imports"]));
    // The link of imports alone is hidden: itself, the middle of its
    // hollow line, and the wide link over it that is hit.
    assert_eq!(html.matches("data-kind-hidden=\"true\"").count(), 3);
    for part in [
        "<path class=\"graph-map-edge graph-map-edge-guess\" data-ends=\"1 2\" data-kind-hidden=\"true\"",
        "<path class=\"graph-map-edge-core\" data-ends=\"1 2\" data-kind-hidden=\"true\"",
        "<path class=\"graph-map-hit\" data-ends=\"1 2\" data-kind-hidden=\"true\"",
    ] {
        assert!(html.contains(part), "{part} in {html}");
    }
    // Every box stands where it stood: the key hides, it does not lay
    // the picture out anew.
    let places = |html: &str| {
        html.match_indices("style=\"left:")
            .map(|(at, _)| html[at..].split('"').nth(1).unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(places(&shown), places(&html));
    // A link of calls and imports stays, as calls, of one kind now.
    assert!(html.contains("data-ends=\"0 1\" d=\"M"));
    assert!(html.contains("data-edge-kind=\"calls\" data-kinds=\"calls imports\" stroke="));
    // With the calls off as well it is drawn as nothing at all.
    let html = drawn_with(&view, off(&["imports", "calls"]));
    assert!(html.contains("data-ends=\"0 1\" data-kind-hidden=\"true\""));
    // A link against the rules is drawn whatever the key says.
    let mut ruled = view.clone();
    ruled["edges"][3]["architecture_violation"] = json!(1);
    let html = drawn_with(&ruled, off(&["extends"]));
    assert!(
        html.contains("graph-map-edge-against\" data-ends=\"0 3\" d=\"M"),
        "{html}"
    );
}

#[test]
fn a_link_shades_into_the_colour_of_its_kind_and_keeps_its_dashes_when_guessed() {
    let html = drawn_with(&kinds_view(), [false; 7]);
    // One shading for each kind between each two columns.
    for (id, kind) in [
        ("graph-map-flow-0", None),
        ("graph-map-flow-1-extends", Some("extends")),
        ("graph-map-flow-2-imports", Some("imports")),
        ("graph-map-flow-3-writes", Some("writes")),
    ] {
        let kind_attr = kind
            .map(|kind| format!(" data-edge-kind=\"{kind}\""))
            .unwrap_or_default();
        assert!(
            html.contains(&format!(
                "<linearGradient id=\"{id}\" class=\"graph-map-flow\"{kind_attr} gradientUnits"
            )),
            "{id} in {html}"
        );
    }
    assert!(html.contains("stroke=\"url(#graph-map-flow-2-imports)\""));
    // No style of a link is set on the link itself but its shading: the
    // stylesheet gives the dashes of the kind, and nothing of a guess
    // takes them away.
    assert!(!html.contains("stroke-dasharray"));
    // The guessed import is hollow: two wider, its middle as wide as it
    // would be.
    assert!(html.contains(
        "data-edge-kind=\"imports\" data-kinds=\"imports\" stroke=\"url(#graph-map-flow-2-imports)\" stroke-width=\"3.0\""
    ), "{html}");
    assert!(html.contains("<path class=\"graph-map-edge-core\" data-ends=\"1 2\" d=\"M"));
    assert!(html.contains("data-edge-kind=\"imports\" stroke-width=\"1.0\"/>"));
}

#[test]
fn a_link_that_passes_layers_carries_a_chevron_halfway() {
    let html = drawn_with(&kinds_view(), [false; 7]);
    // One link passes two layers in slots of its own; the others reach
    // the next layer by one curve and need none.
    assert_eq!(
        html.matches("<path class=\"graph-map-edge-chevron\"")
            .count(),
        1
    );
    assert!(html.contains(
        "<path class=\"graph-map-edge-chevron\" data-ends=\"0 3\" data-edge-kind=\"extends\" data-kinds=\"extends\" transform=\"translate("
    ), "{html}");
    assert!(html.contains("rotate(0)\" d=\"M-3 -3.4 L1.4 0 L-3 3.4\"/>"));
    // It is hidden with its link.
    let html = drawn_with(&kinds_view(), off(&["extends"]));
    assert!(
        html.contains("data-kinds=\"extends\" data-kind-hidden=\"true\" transform=\"translate(")
    );
    // A word about the link stands where the chevron would: then the
    // word is drawn and the chevron is not.
    let mut ruled = kinds_view();
    ruled["pattern"] = json!({"sort": "layers", "declared": true});
    ruled["edges"][3]["against"] = json!(true);
    ruled["edges"][3]["added"] = json!(2);
    let html = drawn_with(&ruled, [false; 7]);
    if html.contains("graph-map-edge-mark") {
        assert_eq!(html.matches("graph-map-edge-chevron").count(), 0, "{html}");
    }
}

#[test]
fn the_key_of_the_links_counts_them_and_switches_them() {
    let view = kinds_view();
    let all = [false; 7];
    assert_eq!(kind_counts(&view, &all), ([1, 2, 1, 0, 1, 0, 1], 0));
    assert_eq!(kind_counts(&view, &off(&["imports"])).1, 1);
    assert_eq!(kind_counts(&view, &off(&["imports", "calls"])).1, 2);
    // A link whose end is not drawn is not counted, as it is not drawn.
    let mut cut = view.clone();
    cut["nodes"].as_array_mut().unwrap().pop();
    assert_eq!(kind_counts(&cut, &all).0, [1, 2, 0, 0, 0, 0, 0]);
    // The window of one link has no boxes of that kind: all are counted.
    let window = json!({"edges": [{"from": "a", "to": "b", "kind": "calls"}]});
    assert_eq!(kind_counts(&window, &all).0[0], 1);

    let html = kinds_legend_html(&view, &off(&["imports"]), "session");
    assert_eq!(html.matches("data-action=\"graph-kind-toggle\"").count(), 7);
    assert_eq!(html.matches("data-action=\"graph-kind-preset\"").count(), 4);
    assert!(html.contains(
        "data-payload=\"{&quot;kind&quot;:&quot;imports&quot;,&quot;surface&quot;:&quot;session&quot;}\" data-edge-kind=\"imports\" data-empty=\"false\" aria-pressed=\"false\""
    ), "{html}");
    assert!(html.contains(
        "<span class=\"graph-arrow-legend-name\">Imports</span><span class=\"graph-arrow-legend-count\">2</span>"
    ));
    assert!(
        html.contains("data-edge-kind=\"implements\" data-empty=\"true\" aria-pressed=\"true\"")
    );
    assert!(html.contains("1 link hidden by this key"));
    // The preset that fits what is switched off is the pressed one.
    let pressed = |html: &str| {
        html.split("data-action=\"graph-kind-preset\"")
            .skip(1)
            .filter(|part| part.contains("aria-pressed=\"true\""))
            .map(|part| {
                part.split("&quot;preset&quot;:&quot;")
                    .nth(1)
                    .unwrap()
                    .split('&')
                    .next()
                    .unwrap()
                    .to_owned()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(pressed(&html), ["no-imports"]);
    assert_eq!(pressed(&kinds_legend_html(&view, &all, "modal")), ["all"]);
    assert_eq!(
        pressed(&kinds_legend_html(
            &view,
            &off(&["imports", "extends", "implements", "uses"]),
            "modal"
        )),
        ["runtime"]
    );
    assert!(!kinds_legend_html(&view, &all, "modal").contains("hidden by this key"));
}

#[test]
fn a_kind_is_switched_off_and_on_shown_alone_and_never_leaves_nothing() {
    let held = |state: &V| {
        m::array(&state["links_off"])
            .iter()
            .map(|kind| kind.as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let state = json!({});
    let state = switched_kind(state, "imports", false);
    assert_eq!(held(&state), "imports");
    let state = switched_kind(state, "uses", false);
    assert_eq!(held(&state), "imports uses");
    let state = switched_kind(state, "imports", false);
    assert_eq!(held(&state), "uses");
    // Alone: every other kind goes off, also where the kind was off.
    let alone = switched_kind(state.clone(), "uses", true);
    assert_eq!(
        held(&alone),
        "calls imports extends implements writes reads"
    );
    // Asked again for the kind that is alone, every kind is shown.
    assert_eq!(held(&switched_kind(alone.clone(), "uses", true)), "");
    // Switching the last shown kind off shows every kind.
    assert_eq!(held(&switched_kind(alone.clone(), "uses", false)), "");
    // Another kind alone takes its place.
    assert_eq!(
        held(&switched_kind(alone, "calls", true)),
        "imports extends implements writes reads uses"
    );
    // A name that is no kind changes nothing.
    assert_eq!(switched_kind(state.clone(), "nothing", false), state);
    // The presets.
    assert_eq!(held(&preset_kinds(state.clone(), "all")), "");
    assert_eq!(
        held(&preset_kinds(state.clone(), "runtime")),
        "imports extends implements uses"
    );
    assert_eq!(
        held(&preset_kinds(state.clone(), "hierarchy")),
        "calls imports writes reads uses"
    );
    assert_eq!(held(&preset_kinds(state.clone(), "no-imports")), "imports");
    assert_eq!(preset_kinds(state.clone(), "nothing"), state);
}

#[test]
fn a_dependency_a_change_added_is_drawn_as_the_kind_it_is() {
    let view = json!({
        "scope": "src",
        "nodes": [
            {"path": "src/A.php", "name": "A", "folder": false},
            {"path": "src/B.php", "name": "B", "folder": false},
            {"path": "src/C.php", "name": "C", "folder": false}
        ],
        "edges": [],
        "focus": {"path": "src/A.php"}
    });
    let impact = json!({
        "path": "src/A.php",
        "uses": {
            "added": [
                {"target": "src/B.php", "kind": "writes"},
                {"target": "src/B.php", "kind": "reads"},
                {"target": "src/B.php", "kind": "writes"}
            ],
            "removed": [{"target": "src/C.php", "kind": "imports"}],
            "unchanged": []
        },
        "used_by": []
    });
    let html = canvas_with_overlays(
        &view,
        "graph-map",
        CanvasOverlayContext {
            impact: Some(&impact),
            ..Default::default()
        },
    )
    .html;
    assert!(html.contains(
        "data-diff=\"added\" d=\"M"
    ) && html.contains("data-edge-kind=\"writes\" data-kinds=\"writes\" stroke-width=\"2\" marker-end=\"url(#graph-map-arrow-writes)\"><title>A -> B (added dependency · writes)</title>"), "{html}");
    assert!(html.contains(
        "data-edge-kind=\"imports\" data-kinds=\"imports\" stroke-width=\"2\" marker-end=\"url(#graph-map-arrow-imports)\"><title>A -> C (removed dependency · imports)</title>"
    ));
}
#[test]
fn a_link_that_a_longer_way_joins_says_nothing_new() {
    // 0 uses 1 uses 2 uses 3, and 0 uses 2 and 3 as well: both of
    // those are said by the chain. 1 uses 3 past 2, which is said too.
    let links = [(0, 1), (1, 2), (2, 3), (0, 2), (0, 3), (1, 3)];
    assert_eq!(
        transitive(4, &links),
        [false, false, false, true, true, true]
    );
    // Without a longer way a link stays, however many links there are.
    assert_eq!(transitive(3, &[(0, 1), (0, 2)]), [false, false]);
    assert_eq!(transitive(3, &[(0, 1), (1, 2)]), [false, false]);
    // Boxes that reach each other are one place: no link among them is
    // left out, a way may lead through them, and a link into the group
    // that another of its boxes already brings is said twice.
    let ring = [(0, 1), (1, 2), (2, 1), (2, 3), (0, 3), (0, 2), (1, 3)];
    assert_eq!(
        transitive(4, &ring),
        [false, false, false, false, true, false, false]
    );
    // Every box still reaches what it reached once they are left out.
    let reaches = |links: &[(usize, usize)], count: usize| {
        (0..count)
            .map(|from| {
                let mut seen = vec![false; count];
                let mut todo = vec![from];
                while let Some(at) = todo.pop() {
                    for (a, b) in links {
                        if *a == at && !seen[*b] {
                            seen[*b] = true;
                            todo.push(*b);
                        }
                    }
                }
                seen
            })
            .collect::<Vec<_>>()
    };
    for (count, links) in [
        (4, links.to_vec()),
        (4, ring.to_vec()),
        (
            7,
            vec![
                (0, 1),
                (0, 2),
                (1, 3),
                (2, 3),
                (3, 4),
                (0, 4),
                (4, 5),
                (5, 6),
                (6, 4),
                (0, 6),
                (2, 5),
            ],
        ),
    ] {
        let said = transitive(count, &links);
        let kept = links
            .iter()
            .zip(&said)
            .filter(|(_, said)| !**said)
            .map(|(link, _)| *link)
            .collect::<Vec<_>>();
        assert!(kept.len() < links.len(), "{links:?}");
        assert_eq!(reaches(&kept, count), reaches(&links, count), "{links:?}");
    }
    // Links that name no box, and a box that uses itself, are no way.
    assert_eq!(
        transitive(2, &[(0, 1), (0, 9), (1, 1)]),
        [false, false, false]
    );
    assert_eq!(transitive(0, &[]), Vec::<bool>::new());
}

#[test]
fn the_reach_of_a_box_is_told_in_links_along_and_against_the_arrows() {
    // 0 uses 1 uses 2 uses 3; 4 uses 2; 5 uses 4.
    let links = [(0, 1), (1, 2), (2, 3), (4, 2), (5, 4)];
    let from_two = |up, down| cone(6, &links, 2, up, down);
    // One link each way: what it uses, and what uses it.
    assert_eq!(
        from_two(1, 1),
        [None, Some(-1), Some(0), Some(1), Some(-1), None]
    );
    // Two against the arrows reach what uses its users.
    assert_eq!(
        from_two(2, 1),
        [Some(-2), Some(-1), Some(0), Some(1), Some(-1), Some(-2)]
    );
    // Further than the picture goes changes nothing.
    assert_eq!(from_two(3, 3), from_two(2, 1));
    assert_eq!(
        cone(6, &links, 0, 1, 3),
        [Some(0), Some(1), Some(2), Some(3), None, None]
    );
    // The links of a cone lead one step on from the box, or one step
    // nearer to it: a link between two boxes as far as each other is
    // none of them, nor is one that leads back towards the box.
    let depth = from_two(2, 1);
    let lit = links
        .iter()
        .map(|link| in_cone(&depth, *link))
        .collect::<Vec<_>>();
    assert_eq!(lit, [true, true, true, true, true]);
    let narrow = from_two(1, 1);
    assert_eq!(
        links
            .iter()
            .map(|link| in_cone(&narrow, *link))
            .collect::<Vec<_>>(),
        [false, true, true, true, false]
    );
    let beside = cone(4, &[(0, 1), (0, 2), (1, 2), (2, 3)], 0, 1, 1);
    assert_eq!(beside, [Some(0), Some(1), Some(1), None]);
    assert!(!in_cone(&beside, (1, 2)));
    // In a cycle a box lies both ways: it is told by the nearer, and by
    // what the box uses where both are as near.
    let ring = cone(3, &[(0, 1), (1, 2), (2, 0)], 0, 2, 2);
    assert_eq!(ring, [Some(0), Some(1), Some(-1)]);
    let pair = cone(2, &[(0, 1), (1, 0)], 0, 1, 1);
    assert_eq!(pair, [Some(0), Some(1)]);
    // A box the picture does not hold reaches nothing.
    assert_eq!(cone(2, &[(0, 1)], 7, 1, 1), [None, None]);
}

#[test]
fn a_picture_leaves_out_what_longer_ways_say_and_lights_a_wider_reach() {
    // A uses B uses C uses D, and A uses C and D past them.
    let view = json!({
        "scope": "src",
        "nodes": [
            {"path": "src/A.php", "name": "A", "folder": false},
            {"path": "src/B.php", "name": "B", "folder": false},
            {"path": "src/C.php", "name": "C", "folder": false},
            {"path": "src/D.php", "name": "D", "folder": false}
        ],
        "edges": [
            {"from": "src/A.php", "to": "src/B.php", "links": 1, "guessed": 0},
            {"from": "src/B.php", "to": "src/C.php", "links": 1, "guessed": 0},
            {"from": "src/C.php", "to": "src/D.php", "links": 1, "guessed": 0},
            {"from": "src/A.php", "to": "src/C.php", "links": 1, "guessed": 0},
            {"from": "src/A.php", "to": "src/D.php", "links": 1, "guessed": 0, "architecture_violation": 1},
            {"from": "src/A.php", "to": "src/E.php", "links": 1, "guessed": 0}
        ],
        "focus": {"path": "src/A.php"}
    });
    // The link against the rules is said by a longer way and is kept
    // all the same; the link to what is not drawn is none to count.
    assert_eq!(transitive_links(&view), 1);
    let drawn = |context: CanvasOverlayContext| {
        let html = canvas_with_overlays(&view, "graph-map", context).html;
        html.match_indices(" data-ends=\"")
            .filter(|(at, _)| {
                html[..*at].ends_with('"')
                    && html[..*at]
                        .rfind("<path class=\"graph-map-edge")
                        .is_some_and(|start| !html[start..*at].contains('>'))
                    && !html[html[..*at].rfind("<path class=\"").unwrap()..*at]
                        .contains("graph-map-edge-core")
            })
            .map(|(at, _)| html[at + 12..].split('"').next().unwrap().to_owned())
            .collect::<BTreeSet<_>>()
    };
    let all = drawn(CanvasOverlayContext::default());
    assert_eq!(all.len(), 5, "{all:?}");
    let fewer = drawn(CanvasOverlayContext {
        hide_transitive: true,
        ..Default::default()
    });
    assert_eq!(
        fewer.into_iter().collect::<Vec<_>>(),
        ["0 1", "0 3", "1 2", "2 3"]
    );
    // A way, a cycle or a pattern leaves nothing out.
    let mut way = view.clone();
    way["mode"] = json!("path");
    way["path"] = json!({"found": true});
    assert_eq!(transitive_links(&way), 0);
    // With A selected its links are lit; with a reach of two, those of
    // what it uses as well.
    let lit = |cone| {
        let html = canvas_with_overlays(
            &view,
            "graph-map",
            CanvasOverlayContext {
                hide_transitive: true,
                cone,
                ..Default::default()
            },
        )
        .html;
        html.matches("class=\"graph-map-edge graph-map-edge-on")
            .count()
    };
    // D is one link from A by the link against the rules, so the link
    // from C to D leads no further from A and is not one of its reach.
    assert_eq!(
        (lit((0, 0)), lit((1, 1)), lit((1, 2)), lit((1, 3))),
        (2, 2, 3, 3)
    );
}

#[test]
fn the_key_offers_to_leave_out_transitive_links_and_to_widen_the_reach() {
    let mut view = kinds_view();
    view["edges"]
        .as_array_mut()
        .unwrap()
        .push(json!({"from": "src/A.php", "to": "src/C.php", "links": 1, "guessed": 0}));
    // A uses B uses C, so A uses C is said twice; A extends D past C.
    assert_eq!(transitive_links(&view), 2);
    let key = KeyState::default();
    let html = key_html(&view, &key, "modal");
    assert!(html.contains(
        "data-action=\"graph-transitive-toggle\" data-payload=\"{&quot;surface&quot;:&quot;modal&quot;}\" aria-pressed=\"false\""
    ), "{html}");
    assert!(html.contains("Hide transitive <span class=\"graph-arrow-legend-count\">2</span>"));
    // Nothing is selected: no reach to set.
    assert!(!html.contains("graph-cone"));
    // The note that ends the key stays its last part.
    assert!(
        html.rfind("graph-transitive-toggle").unwrap()
            < html.rfind("graph-arrow-legend-note").unwrap()
    );
    let on = key_html(
        &view,
        &KeyState {
            hide_transitive: true,
            ..key
        },
        "modal",
    );
    assert!(
        on.contains("data-action=\"graph-transitive-toggle\"")
            && on.contains("aria-pressed=\"true\" title=\"2 links are left out")
    );
    // A selected box: one to three links on each side, the reach that
    // is set pressed.
    view["focus"] = json!({"path": "src/B.php"});
    let html = key_html(
        &view,
        &KeyState {
            cone: (2, 1),
            ..key
        },
        "session",
    );
    assert_eq!(html.matches("data-action=\"graph-cone\"").count(), 6);
    for (side, hops, pressed) in [
        ("up", 2, true),
        ("up", 1, false),
        ("down", 1, true),
        ("down", 3, false),
    ] {
        assert!(html.contains(&format!(
            "data-payload=\"{{&quot;hops&quot;:{hops},&quot;side&quot;:&quot;{side}&quot;,&quot;surface&quot;:&quot;session&quot;}}\" aria-pressed=\"{pressed}\""
        )), "{side} {hops} in {html}");
    }
    // What is selected outside the picture has no links there to light.
    view["focus"] = json!({"path": "lib/Other.php"});
    assert!(!key_html(&view, &key, "modal").contains("graph-cone"));
    // A picture of a way has neither.
    let mut way = view.clone();
    way["mode"] = json!("path");
    way["path"] = json!({"found": true});
    let html = key_html(&way, &key, "modal");
    assert!(!html.contains("graph-transitive-toggle") && !html.contains("graph-cone"));
    // The reach of a state.
    assert_eq!(cone_of(&json!({})), (1, 1));
    let state = reaching(json!({}), "up", 3);
    assert_eq!(cone_of(&state), (3, 1));
    let state = reaching(state, "down", 2);
    assert_eq!(cone_of(&state), (3, 2));
    assert_eq!(cone_of(&reaching(state.clone(), "down", 99)), (3, 3));
    assert_eq!(cone_of(&reaching(state.clone(), "up", 0)), (1, 2));
    assert_eq!(reaching(state.clone(), "sideways", 2), state);
}

#[test]
fn a_picture_tells_more_or_less_by_the_scale_it_is_drawn_at() {
    assert_eq!(lod(1.0), "mid");
    assert_eq!(lod(0.5), "mid");
    assert_eq!(lod(0.49), "far");
    assert_eq!(lod(CAMERA_MIN), "far");
    assert_eq!(lod(1.4), "mid");
    assert_eq!(lod(1.41), "near");
    assert_eq!(lod(CAMERA_MAX), "near");
    // A scale that is none is the picture as it is laid out.
    assert_eq!(lod(0.0), "mid");
    assert_eq!(lod(f64::NAN), "mid");
}

#[test]
fn a_box_holds_a_second_line_for_a_picture_drawn_large() {
    let view = json!({
        "scope": "src",
        "nodes": [
            {"path": "src/Billing/Invoice.php", "name": "Invoice.php", "folder": false, "in": 3, "out": 12},
            {"path": "src/Billing", "name": "Billing", "folder": true, "files": 7, "in": 0, "out": 2},
            {"path": "src/Billing/Invoice.php::total", "name": "total", "symbol": true, "kind": "method", "line": 42, "file": "src/Billing/Invoice.php", "in": 1, "out": 0},
            {"path": "Top.php", "name": "Top.php", "folder": false}
        ],
        "edges": []
    });
    let html = canvas(&view);
    let line = |text: &str| {
        format!("<span class=\"graph-map-node-detail\" aria-hidden=\"true\">{text}</span>")
    };
    // Through how many links it is used and uses others, then where it
    // lies: the folder of a file, the file and line of a name, and for
    // a folder how many files it holds.
    assert!(html.contains(&line("3→ →12 · src/Billing")), "{html}");
    assert!(html.contains(&line("0→ →2 · 7 files")), "{html}");
    assert!(html.contains(&line("1→ →0 · Invoice.php:42")), "{html}");
    // A file at the top of the project that no link touches adds nothing.
    assert_eq!(html.matches("graph-map-node-detail").count(), 3);
}

#[test]
fn the_small_picture_frames_what_the_window_shows() {
    let (room, picture) = ((400.0, 300.0), (1000.0, 600.0));
    // The corner of the picture at the corner of its window.
    assert_eq!(
        in_window((0.0, 0.0), 1.0, room, picture),
        Some((0.0, 0.0, 400.0, 300.0))
    );
    // Moved on, and drawn at half its size: twice as much is seen.
    assert_eq!(
        in_window((-100.0, -50.0), 0.5, room, picture),
        Some((200.0, 100.0, 800.0, 500.0))
    );
    // What lies past the picture is no part of it.
    assert_eq!(
        in_window((-450.0, 100.0), 0.5, room, picture),
        Some((900.0, 0.0, 100.0, 400.0))
    );
    // A window that shows all of the picture has nothing to frame,
    // wherever in the window the picture lies.
    assert_eq!(in_window((0.0, 0.0), 0.4, room, picture), None);
    assert_eq!(in_window((50.0, 40.0), 0.25, room, picture), None);
    // Wide enough and not tall enough is not all of it.
    assert_eq!(
        in_window((0.0, -30.0), 0.4, (400.0, 200.0), picture),
        Some((0.0, 75.0, 1000.0, 500.0))
    );
    // A window moved off the picture frames nothing at the side the
    // picture lies on.
    assert_eq!(
        in_window((500.0, 0.0), 1.0, room, picture),
        Some((0.0, 0.0, 0.0, 300.0))
    );
    // No scale and no picture are nothing to show.
    assert_eq!(in_window((0.0, 0.0), 0.0, room, picture), None);
    assert_eq!(in_window((0.0, 0.0), 1.0, room, (0.0, 600.0)), None);
    // Looking at a point brings it to the middle of the window.
    let corner = centred((500.0, 300.0), 2.0, room);
    assert_eq!(corner, (-800.0, -450.0));
    let seen = in_window(corner, 2.0, room, picture).unwrap();
    assert_eq!(
        (seen.0 + seen.2 / 2.0, seen.1 + seen.3 / 2.0),
        (500.0, 300.0)
    );
    // The small picture is as large as its room lets its shape be, and
    // a long thin one keeps a side wide enough to press.
    assert_eq!(radar_size((1000.0, 500.0)), (176.0, 88.0));
    assert_eq!(radar_size((500.0, 1000.0)), (88.0, 176.0));
    assert_eq!(radar_size((4000.0, 100.0)), (176.0, 18.0));
    assert_eq!(radar_size((0.0, 100.0)), (0.0, 0.0));
}

#[test]
fn a_picture_is_drawn_in_small_with_what_its_boxes_are() {
    // A changed file uses B; B and C use each other; C is selected.
    let view = json!({
        "scope": "src",
        "nodes": [
            {"path": "src/A.php", "name": "A", "folder": false, "changed": 1, "status": "M"},
            {"path": "src/B.php", "name": "B", "folder": false},
            {"path": "src/C.php", "name": "C", "folder": false}
        ],
        "edges": [
            {"from": "src/A.php", "to": "src/B.php", "links": 1, "guessed": 0},
            {"from": "src/B.php", "to": "src/C.php", "links": 2, "guessed": 0},
            {"from": "src/C.php", "to": "src/B.php", "links": 1, "guessed": 0}
        ],
        "focus": {"path": "src/C.php"}
    });
    let rendered = canvas_with_overlays(&view, "graph-map", CanvasOverlayContext::default());
    let radar = &rendered.radar;
    let (width, height) = radar_size((rendered.width, rendered.height));
    assert!(
        radar.starts_with(&format!(
            "<svg class=\"graph-radar-picture\" width=\"{width:.0}\" height=\"{height:.0}\" viewBox=\"0 0 {:.0} {:.0}\" preserveAspectRatio=\"none\" data-w=\"{:.0}\" data-h=\"{:.0}\"",
            rendered.width, rendered.height, rendered.width, rendered.height
        )),
        "{radar}"
    );
    // A box for each box of the picture, where it stands there.
    assert_eq!(radar.matches("class=\"graph-radar-box\"").count(), 3);
    let plan = plan(3, &[(0, 1, 1), (1, 2, 2), (2, 1, 1)]);
    for placed in &plan.nodes {
        assert!(
            radar.contains(&format!(
                "<rect class=\"graph-radar-box\" x=\"{:.0}\" y=\"{:.0}\" width=\"{BOX_WIDTH:.0}\" height=\"{BOX_HEIGHT:.0}\"",
                placed.x, placed.y
            )),
            "{radar}"
        );
    }
    // One thing is told of a box: selected before changed before part
    // of a cycle.
    assert_eq!(radar.matches("data-state=\"changed\"").count(), 1);
    assert_eq!(radar.matches("data-state=\"selected\"").count(), 1);
    assert_eq!(radar.matches("data-state=\"cycle\"").count(), 1);
    // What the window shows is framed once the picture is measured.
    assert!(radar.ends_with(
        "<rect class=\"graph-radar-view\" x=\"0\" y=\"0\" width=\"0\" height=\"0\"/></svg>"
    ));
    // A picture of nothing has no small picture.
    let none = json!({"scope": "src", "nodes": [], "edges": []});
    assert_eq!(
        canvas_with_overlays(&none, "graph-map", CanvasOverlayContext::default()).radar,
        ""
    );
}

#[test]
fn a_box_says_it_is_part_of_a_circle_of_files_wherever_the_rest_lies() {
    // Order.php is part of a circle of four files, and this picture
    // draws no other of them as a file. The folder cart holds two of
    // them, so that circle leads out of it and back. The folder mail
    // holds a circle that stays inside it.
    let view = json!({
        "scope": "src",
        "nodes": [
            {"path": "src/Order.php", "name": "Order.php", "folder": false,
             "circle": {"files": 1, "groups": 1, "largest": 4, "leaves": true}},
            {"path": "src/cart", "name": "cart", "folder": true, "files": 5,
             "circle": {"files": 2, "groups": 1, "largest": 4, "leaves": true}},
            {"path": "src/mail", "name": "mail", "folder": true, "files": 3,
             "circle": {"files": 2, "groups": 1, "largest": 2, "leaves": false}},
            {"path": "src/Plain.php", "name": "Plain.php", "folder": false}
        ],
        "edges": [{"from": "src/Order.php", "to": "src/cart", "links": 1, "guessed": 0}],
        "circles": {"groups": 3, "files": 9, "largest": 4, "here": 2, "listed": [
            {"files": 4, "start": "src/cart/Cart.php",
             "names": ["Cart.php", "Price.php", "Order.php", "Stock.php"], "more": 0},
            {"files": 2, "start": "src/mail/Mailer.php", "names": ["Mailer.php", "Queue.php"], "more": 0}
        ]}
    });
    let html = canvas(&view);
    // No link of the picture closes a cycle: the picture alone would
    // mark nothing.
    assert_eq!(html.matches("data-cycle=\"true\"").count(), 0);
    assert!(
        html.contains("is used through 0 links, part of a cycle of 4 files\""),
        "{html}"
    );
    assert!(html.contains(", holds 2 files of cycles that lead out of it and back\""));
    assert!(html.contains(", holds 2 files that use each other in cycles\""));
    assert_eq!(html.matches(" data-circle=\"through\"").count(), 2);
    assert_eq!(html.matches(" data-circle=\"inside\"").count(), 1);
    // The mark of a file, and that of a folder with how many of its
    // files are part of circles.
    let mark = |held: &str| {
        format!("<span class=\"graph-map-node-cycle\" aria-hidden=\"true\">↻{held}</span>")
    };
    assert_eq!(html.matches(&mark("")).count(), 1);
    assert_eq!(html.matches(&mark("2")).count(), 2);
    // Beside the map: the circles that touch what is open, each shown
    // through the file that has most to do with the others.
    let side = circles_side(&view);
    assert!(
        side.starts_with("<h4>Cycles of files <span class=\"graph-map-total\">2</span></h4><p class=\"knowledge-note\">2 of the 3 cycles of the project touch what is open."),
        "{side}"
    );
    assert!(
        side.contains("data-action=\"graph-map-cycle\" data-payload=\"&quot;src/cart/Cart.php&quot;\" title=\"Show the shortest cycle through src/cart/Cart.php\"><span class=\"graph-map-row-name\">Cart.php, Price.php, Order.php, Stock.php</span><span class=\"graph-map-row-links\">4 files</span></button>"),
        "{side}"
    );
    assert_eq!(side.matches("data-action=\"graph-map-cycle\"").count(), 2);
    assert!(side.ends_with("</ul>"));
    // Where every circle of the project touches what is open, the
    // project is told as a whole.
    let mut all = view.clone();
    all["circles"]["groups"] = json!(2);
    assert!(circles_side(&all)
        .contains("9 files of the project use each other in 2 cycles; the largest holds 4."));
    // More files than are named, and more circles than are listed.
    let mut many = view.clone();
    many["circles"]["here"] = json!(9);
    many["circles"]["groups"] = json!(9);
    many["circles"]["listed"][0]["more"] = json!(12);
    let side = circles_side(&many);
    assert!(side.contains("Stock.php, +12</span>"), "{side}");
    assert!(side.ends_with("<p class=\"knowledge-note\">and 7 more</p>"));
    // A project without circles lists none, and neither does the
    // picture of one cycle.
    assert_eq!(circles_side(&json!({"scope": "src"})), "");
    let mut cycle = view.clone();
    cycle["mode"] = json!("cycle");
    assert_eq!(circles_side(&cycle), "");
}

#[test]
fn the_key_of_a_path_offers_its_longer_ways_and_the_picture_marks_them() {
    // a reaches c through b in two links, and through d and e in three.
    let view = json!({
        "mode": "path",
        "scope": "src",
        "path": {
            "from": {"path": "src/a.php", "name": "a.php"},
            "to": {"path": "src/c.php", "name": "c.php"},
            "names": false, "found": true, "reversed": false, "within": false,
            "steps": 2, "places": 3, "shown": 5, "links": 2, "wide": 8,
            "longer": 1, "detours": 1, "detours_more": false
        },
        "nodes": [
            {"path": "src/a.php", "name": "a.php", "folder": false, "start": true, "step": 0},
            {"path": "src/b.php", "name": "b.php", "folder": false, "step": 1},
            {"path": "src/c.php", "name": "c.php", "folder": false, "end": true, "step": 2},
            {"path": "src/d.php", "name": "d.php", "folder": false, "step": 1, "detour": true},
            {"path": "src/e.php", "name": "e.php", "folder": false, "step": 2, "detour": true}
        ],
        "edges": [
            {"from": "src/a.php", "to": "src/b.php", "links": 1, "guessed": 0},
            {"from": "src/b.php", "to": "src/c.php", "links": 1, "guessed": 0},
            {"from": "src/a.php", "to": "src/d.php", "links": 1, "guessed": 0, "detour": true},
            {"from": "src/d.php", "to": "src/e.php", "links": 1, "guessed": 0, "detour": true},
            {"from": "src/e.php", "to": "src/c.php", "links": 1, "guessed": 0, "detour": true}
        ]
    });
    let html = canvas(&view);
    // The three links and the two boxes that only the longer way has
    // say so, in their marks and in what is read out for them.
    assert_eq!(html.matches(" data-detour=\"true\" d=\"").count(), 3);
    assert_eq!(html.matches(" data-detour=\"true\"").count(), 5);
    assert_eq!(html.matches(", on a longer way only\"").count(), 2);
    assert_eq!(html.matches(" · on a longer way only").count(), 3);
    // The key: the shortest ways alone, or with them those longer by
    // one, two or three links; what is asked for is pressed, and the
    // key says how many ways round the picture draws.
    let key = key_html(&view, &KeyState::default(), "modal");
    assert_eq!(key.matches("data-action=\"graph-map-longer\"").count(), 4);
    assert!(
        key.contains("data-payload=\"{&quot;longer&quot;:1,&quot;surface&quot;:&quot;modal&quot;}\" aria-pressed=\"true\" title=\"Also the ways longer than the shortest by 1 link\">+1</button>"),
        "{key}"
    );
    assert!(key.contains("data-payload=\"{&quot;longer&quot;:0,&quot;surface&quot;:&quot;modal&quot;}\" aria-pressed=\"false\" title=\"The shortest ways alone\">none</button>"));
    assert!(key.contains("title=\"Also the ways longer than the shortest by 3 links\">+3</button>"));
    assert!(key.contains("<span class=\"graph-arrow-legend-side\">1 drawn, lighter</span>"));
    // It stands before the note that ends the key, and a path has
    // neither links to leave out nor a reach to widen.
    assert!(key.rfind("graph-map-longer").unwrap() < key.rfind("graph-arrow-legend-note").unwrap());
    assert!(!key.contains("graph-transitive-toggle") && !key.contains("graph-cone"));
    let told = |change: &dyn Fn(&mut V)| {
        let mut other = view.clone();
        change(&mut other);
        key_html(&other, &KeyState::default(), "session")
    };
    // More ways than are drawn, none at all, and the shortest alone.
    let more = told(&|view| {
        view["path"]["detours"] = json!(12);
        view["path"]["detours_more"] = json!(true);
    });
    assert!(more.contains(">12+ drawn, lighter</span>"), "{more}");
    assert!(more.contains("&quot;surface&quot;:&quot;session&quot;}"));
    assert!(told(&|view| view["path"]["detours"] = json!(0)).contains(">none found</span>"));
    let shortest = told(&|view| view["path"]["longer"] = json!(0));
    assert!(
        shortest.contains("aria-pressed=\"true\" title=\"The shortest ways alone\">none</button>")
    );
    assert!(!shortest.contains("graph-arrow-legend-side"));
    // Above the picture the path says how many ways round it draws,
    // and the note under it what they are.
    assert_eq!(
        summary(&view),
        "Path from a.php to c.php · 2 links · 3 places on the shortest ways · 1 longer way"
    );
    assert!(
        note(&view)
            .ends_with("Drawn lighter with them: 1 way longer by 1 link at most, each whole."),
        "{}",
        note(&view)
    );
    let mut many = view.clone();
    many["path"]["longer"] = json!(3);
    many["path"]["detours"] = json!(12);
    many["path"]["detours_more"] = json!(true);
    assert!(summary(&many).ends_with(" · at least 12 longer ways"));
    assert!(note(&many).ends_with(
        "Drawn lighter with them: 12 ways longer by 3 links at most, each whole; there are more."
    ));
    let mut none = view.clone();
    none["path"]["detours"] = json!(0);
    assert!(summary(&none).ends_with("3 places on the shortest ways"));
    assert!(note(&none).ends_with("No way that is longer by 1 link at most leads there."));
    // A path that leads nowhere has no ways round to offer.
    assert!(!told(&|view| view["path"]["found"] = json!(false)).contains("graph-map-longer"));
    // The request names how much longer only for a path, and no more
    // than may be asked for.
    let mut state = fresh("work");
    state["path"] = json!(["src/a.php", "src/c.php"]);
    assert!(!build_map_path("p", &state, false).contains("longer="));
    state["longer"] = json!(2);
    assert!(build_map_path("p", &state, false).contains("longer=2"));
    state["longer"] = json!(9);
    assert!(build_map_path("p", &state, false).contains("longer=3"));
    state["shared"] = json!(true);
    assert!(!build_map_path("p", &state, false).contains("longer="));
}

#[test]
fn the_cards_under_a_box_lie_on_no_other_box() {
    // a, b and c use x, so the three stand under each other in one
    // layer. Five files that no link touches stand in a grid of two
    // rows under them. A session made two names in b and one in the
    // first file of the grid.
    let file = |name: &str| json!({"path": format!("src/{name}.php"), "name": format!("{name}.php"), "folder": false});
    let view = json!({
        "scope": "src",
        "nodes": [
            file("a"), file("b"), file("c"), file("x"),
            file("lone"), file("u1"), file("u2"), file("u3"), file("u4")
        ],
        "edges": [
            {"from": "src/a.php", "to": "src/x.php", "links": 1, "guessed": 0},
            {"from": "src/b.php", "to": "src/x.php", "links": 1, "guessed": 0},
            {"from": "src/c.php", "to": "src/x.php", "links": 1, "guessed": 0}
        ]
    });
    let focus = parse_live_focus(&json!({
        "phase": "active",
        "summary": "Adding two methods",
        "target_paths": ["src/b.php"],
        "created_entities": [
            {"id": "e1", "kind": "method", "name": "one", "parent": "src/b.php", "path": "src/b.php#L3:one"},
            {"id": "e2", "kind": "method", "name": "two", "parent": "src/b.php", "path": "src/b.php#L9:two"},
            {"id": "e3", "kind": "function", "name": "alone", "parent": "src/lone.php", "path": "src/lone.php#L2:alone"}
        ]
    }))
    .unwrap();
    // Where each box stands in a picture, by its number.
    let boxes = |html: &str| {
        html.split("<button type=\"button\" class=\"graph-map-node\" data-n=\"")
            .skip(1)
            .map(|part| {
                let side = |key: &str| {
                    part.split(key)
                        .nth(1)
                        .and_then(|rest| rest.split("px").next())
                        .and_then(|number| number.parse::<f64>().ok())
                        .unwrap()
                };
                (side("style=\"left:"), side(";top:"))
            })
            .collect::<Vec<_>>()
    };
    let plain = boxes(&canvas(&view));
    let rendered = canvas_with(&view, "session-graph", Some(&focus));
    let roomy = boxes(&rendered.html);
    assert_eq!((plain.len(), roomy.len()), (9, 9));
    // The cards hang under b and under the first file of the grid.
    assert_eq!(rendered.focus.under, BTreeMap::from([(1, 2), (4, 1)]));
    assert_eq!(rendered.focus.entities.len(), 3);
    // Without them the boxes of a layer stand a row gap apart, and
    // the rows of the grid too: a card would lie on the next box.
    assert_eq!(plain[2].1 - plain[1].1, BOX_HEIGHT + ROW_GAP);
    assert_eq!(plain[8].1 - plain[4].1, BOX_HEIGHT + ROW_GAP);
    // With them the box under b stands as much lower as two cards
    // take, and the rows of the grid as much further apart as one.
    assert_eq!(
        (entity_room(0), entity_room(1), entity_room(2)),
        (0.0, 34.0, 62.0)
    );
    assert_eq!(entity_room(9), entity_room(3));
    assert_eq!(
        roomy[2].1 - roomy[1].1,
        BOX_HEIGHT + ROW_GAP + entity_room(2)
    );
    assert_eq!(roomy[1].1 - roomy[0].1, BOX_HEIGHT + ROW_GAP);
    assert_eq!(
        roomy[8].1 - roomy[4].1,
        BOX_HEIGHT + ROW_GAP + entity_room(1)
    );
    // No card lies on a box, and none on another card.
    let meet = |a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)| {
        a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
    };
    let cards = rendered
        .focus
        .entities
        .iter()
        .map(|card| (card.x, card.y, ENTITY_WIDTH, ENTITY_HEIGHT))
        .collect::<Vec<_>>();
    for (at, card) in cards.iter().enumerate() {
        for (node, (x, y)) in roomy.iter().enumerate() {
            assert!(
                !meet(*card, (*x, *y, BOX_WIDTH, BOX_HEIGHT)),
                "card {at} {card:?} lies on box {node} at {x},{y}"
            );
        }
        for other in &cards[at + 1..] {
            assert!(!meet(*card, *other), "{card:?} lies on {other:?}");
        }
    }
    // Laid out as it was, the same cards would lie on what stands under.
    let old = plan_routed(
        9,
        &[(0, 3, 1), (1, 3, 1), (2, 3, 1)],
        &Frames::default(),
        None,
        Routing::CLEAN,
    );
    let under_b = (
        old.nodes[1].x + 8.0,
        old.nodes[1].y + BOX_HEIGHT + 6.0,
        ENTITY_WIDTH,
        ENTITY_HEIGHT,
    );
    assert!(meet(
        under_b,
        (old.nodes[2].x, old.nodes[2].y, BOX_WIDTH, BOX_HEIGHT)
    ));
    // Room that is none, and room for boxes that are not there, leave
    // the picture as it was.
    assert_eq!(
        plan_roomy(
            9,
            &[(0, 3, 1), (1, 3, 1), (2, 3, 1)],
            &Frames::default(),
            None,
            Routing::CLEAN,
            &[0.0, -4.0, f64::NAN]
        ),
        old
    );
}

#[test]
fn a_selected_place_tells_what_a_change_to_it_reaches_and_marks_the_boxes() {
    let view = json!({
        "scope": "src",
        "nodes": [
            {"path": "src/price.php", "name": "price.php", "folder": false},
            {"path": "src/cart.php", "name": "cart.php", "folder": false},
            {"path": "src/web", "name": "web", "folder": true, "files": 3},
            {"path": "src/alone.php", "name": "alone.php", "folder": false}
        ],
        "edges": [
            {"from": "src/cart.php", "to": "src/price.php", "links": 2, "guessed": 0},
            {"from": "src/web", "to": "src/cart.php", "links": 1, "guessed": 0}
        ],
        "focus": {"path": "src/price.php", "blast": {
            "place": "src/price.php", "hops": 4, "files": 4, "names": 6, "tests": 1, "cut": false,
            "by_hop": [
                {"hop": 1, "files": 2, "names": 3}, {"hop": 2, "files": 2, "names": 3},
                {"hop": 3, "files": 0, "names": 0}, {"hop": 4, "files": 0, "names": 0}
            ],
            "listed": [
                {"path": "src/cart.php", "hop": 1, "left": 0.85, "name": "Cart.total()", "via": "calls", "through": false, "names": 2, "test": false},
                {"path": "tests/PriceTest.php", "hop": 1, "left": 0.85, "name": "PriceTest.testTotal()", "via": "calls", "through": false, "names": 1, "test": true},
                {"path": "src/web/Checkout.php", "hop": 2, "left": 0.72, "name": "Checkout.pay()", "via": "calls", "through": false, "names": 2, "test": false},
                {"path": "src/web/Gate.php", "hop": 2, "left": 0.34, "name": "Gate", "via": "calls", "through": true, "names": 1, "test": false}
            ],
            "more": 0,
            "marked": [
                ["src/cart.php", 1, 0.85], ["tests/PriceTest.php", 1, 0.85],
                ["src/web/Checkout.php", 2, 0.72], ["src/web/Gate.php", 2, 0.34]
            ]
        }}
    });
    let side = blast_side(&view);
    assert!(
        side.starts_with("<h4>A change here reaches <span class=\"graph-map-total\">4</span></h4><p class=\"knowledge-note\">Files that use it, and what uses those, up to 4 links away: 2 at 1 link, 2 at 2 links. 1 of them is a test. Each link passes on less"),
        "{side}"
    );
    assert!(side.contains(
        "<li><button type=\"button\" class=\"graph-map-row\" data-blast-hop=\"1\" data-action=\"graph-map-open\" data-payload=\"&quot;src/cart.php&quot;\" title=\"src/cart.php · reached at Cart.total() by calls · select to open it\"><span class=\"graph-map-row-name\">cart.php</span><span class=\"graph-map-row-links\">src · 1 link · 85 %</span></button></li>"
    ), "{side}");
    assert!(
        side.contains("<span class=\"graph-map-row-links\">tests · 1 link · 85 % · test</span>")
    );
    assert!(side.contains("title=\"src/web/Gate.php · reached at Gate by calls through an interface · select to open it\""));
    assert!(side.contains(
        "<span class=\"graph-map-row-links\">src/web · 2 links · 34 % · past an interface</span>"
    ));
    assert!(side.ends_with("</ul>"));
    // The boxes it reaches carry a bar as long as what is left there,
    // a folder that of the most reached file in it; the selected box
    // and what is not reached carry none.
    let html = canvas(&view);
    assert_eq!(
        html.matches("<span class=\"graph-map-node-blast\" aria-hidden=\"true\"></span>")
            .count(),
        2
    );
    assert!(html.contains(" data-blast=\"1\""));
    assert!(html.contains(" data-blast=\"2\""));
    assert!(html.contains(" style=\"--blast:0.85;left:"), "{html}");
    assert!(html.contains(" style=\"--blast:0.72;left:"));
    assert!(html.contains("a change to the selected place reaches it in 1 link, 85 % of it\""));
    assert!(html.contains("a change to the selected place reaches it in 2 links, 72 % of it\""));
    // Nothing reached, more than is listed, a walk cut short.
    let mut none = view.clone();
    none["focus"]["blast"]["files"] = json!(0);
    assert_eq!(
        blast_side(&none),
        "<h4>A change here reaches <span class=\"graph-map-total\">0</span></h4><p class=\"knowledge-note\">Nothing else uses it within 4 links.</p>"
    );
    let mut more = view.clone();
    more["focus"]["blast"]["more"] = json!(17);
    more["focus"]["blast"]["cut"] = json!(true);
    more["focus"]["blast"]["tests"] = json!(3);
    let side = blast_side(&more);
    assert!(side.contains(" 3 of them are tests. "));
    assert!(side.contains("there is more.</p>"));
    assert!(side.ends_with("<p class=\"knowledge-note\">and 17 more</p>"));
    // Without a selected place there is nothing to tell or to mark.
    let mut plain = view.clone();
    plain["focus"] = V::Null;
    assert_eq!(blast_side(&plain), "");
    assert!(!canvas(&plain).contains("graph-map-node-blast"));
}

#[test]
fn a_change_is_ranked_by_the_tests_that_fail() {
    // What a person pastes: each test once, blanks and repeats left out.
    assert_eq!(
        failing_lines("  CartTest \n\nFAILED tests/a.py::t\nCartTest\r\n"),
        ["CartTest", "FAILED tests/a.py::t"]
    );
    assert_eq!(failing_lines(&"x\n".repeat(3)).len(), 1);
    let many = (0..100)
        .map(|at| format!("T{at}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(failing_lines(&many).len(), FAILING);
    assert_eq!(failing_lines(&"y".repeat(500))[0].len(), FAILING_NAME);
    let view = json!({
        "mode": "changes",
        "scope": "",
        "nodes": [
            {"path": "src/cart.php", "name": "cart.php", "folder": false, "changed": 1, "status": "M",
             "suspect": {"rank": 1, "score": 1.0}},
            {"path": "src/price.php", "name": "price.php", "folder": false, "changed": 1, "status": "M",
             "suspect": {"rank": 2, "score": 0.71}},
            {"path": "src/mail.php", "name": "mail.php", "folder": false, "changed": 1, "status": "M"}
        ],
        "edges": [{"from": "src/cart.php", "to": "src/price.php", "links": 1, "guessed": 0}],
        "suspects": {
            "failing": ["tests/CartTest.php"], "unmatched": ["Cart"], "tests": 3,
            "rows": [
                {"path": "src/cart.php", "status": "M", "rank": 1, "score": 1.0, "failing": 1, "passing": 0, "nearest": 1},
                {"path": "src/price.php", "status": "M", "rank": 2, "score": 0.71, "failing": 1, "passing": 1, "nearest": 2}
            ],
            "more": 0, "clear": 1, "complete": true
        }
    });
    let state = json!({"failing": ["1) Tests\\Unit\\CartTest::testTotal", "Cart"]});
    let side = suspects_side(&view, &state, SESSION_SURFACE);
    assert!(side.starts_with("<h4>Failing tests</h4><p class=\"knowledge-note\">The changed files that 1 failing test file reaches, first those"), "{side}");
    assert!(side.contains(
        "<li><button type=\"button\" class=\"graph-map-row\" data-suspect-rank=\"1\" data-action=\"graph-map-node\" data-payload=\"&quot;src/cart.php&quot;\" title=\"src/cart.php · select it in the picture\"><span class=\"graph-map-node-suspect\" aria-hidden=\"true\">#1</span><span class=\"graph-map-row-name\">cart.php</span><span class=\"graph-map-row-links\">src · 1 of 1 failing, 0 passing · 1 link from a failing test</span></button></li>"
    ), "{side}");
    assert!(side.contains("src · 1 of 1 failing, 1 passing · 2 links from a failing test"));
    assert!(side
        .contains("<p class=\"knowledge-note\">No failing test reaches 1 other changed file.</p>"));
    assert!(side.contains("<p class=\"knowledge-note\">No test file found for: Cart.</p>"));
    // The field keeps what was pasted, and the tests can be changed or
    // dropped; the buttons say which picture they rank.
    assert!(side.contains("<details class=\"graph-suspects-ask\"><summary>Change the failing tests</summary><textarea id=\"session-graph-failing\""), "{side}");
    assert!(side.contains(">1) Tests\\Unit\\CartTest::testTotal\nCart</textarea>"));
    assert!(side.contains("data-action=\"graph-map-failing\" data-payload=\"{&quot;surface&quot;:&quot;session&quot;}\">Rank the changes</button>"));
    assert!(side.contains("data-payload=\"{&quot;clear&quot;:true,&quot;surface&quot;:&quot;session&quot;}\">Clear</button>"));
    // The boxes the failing tests reach carry their rank.
    let html = canvas(&view);
    assert!(html.contains("<span class=\"graph-map-node-suspect\" aria-hidden=\"true\">#1</span>"));
    assert!(html.contains("<span class=\"graph-map-node-suspect\" aria-hidden=\"true\">#2</span>"));
    assert_eq!(html.matches("graph-map-node-suspect").count(), 2);
    assert!(html.contains(", ranked 1 among the changes by the failing tests,"));
    // A folder that holds ranked files carries the best rank of them.
    let mut folders = view.clone();
    folders["nodes"] = json!([
        {"path": "src", "name": "src", "folder": true, "files": 3, "changed": 3,
         "suspect": {"rank": 1, "score": 1.0, "files": 2}}
    ]);
    folders["edges"] = json!([]);
    let html = canvas(&folders);
    assert!(html.contains("<span class=\"graph-map-node-suspect\" aria-hidden=\"true\">#1</span>"));
    assert!(html.contains(", holds 2 changed files the failing tests reach, the first ranked 1"));
    // Nothing asked yet: an open field and a word on what it does.
    let mut plain = view.clone();
    plain["suspects"] = V::Null;
    let side = suspects_side(&plain, &json!({}), MODAL_SURFACE);
    assert!(side.contains("Paste the tests that fail to rank the changed files"));
    assert!(side.contains("<details class=\"graph-suspects-ask\" open><summary>Name the failing tests</summary><textarea id=\"graph-map-failing\""));
    assert!(!side.contains(">Clear</button>"));
    // No test named matched, or nothing changed is reached.
    let mut none = view.clone();
    none["suspects"]["failing"] = json!([]);
    none["suspects"]["rows"] = json!([]);
    assert!(suspects_side(&none, &state, MODAL_SURFACE)
        .contains("None of the names is a test file of this graph."));
    let mut apart = view.clone();
    apart["suspects"]["rows"] = json!([]);
    assert!(suspects_side(&apart, &state, MODAL_SURFACE)
        .contains("No changed file is reached by the failing test."));
    // Only a picture of a change is ranked.
    let mut folder = view.clone();
    folder["mode"] = json!("folder");
    assert_eq!(suspects_side(&folder, &state, MODAL_SURFACE), "");
    // The request names the failing tests only for a change.
    let mut asked = fresh("work");
    asked["failing"] = json!(["CartTest"]);
    assert!(build_map_path("p", &asked, false).contains("failing="));
    asked["target"] = json!("");
    assert!(!build_map_path("p", &asked, false).contains("failing="));
}

#[test]
fn changes_lookup_handles_renamed_folder_deleted_false_positive_and_untouched_symbols() {
    let changes = json!({
        "files": [
            { "path": "src/NewName.php", "previous_path": "src/OldName.php", "status": "renamed" },
            { "path": "src/surviving_dir/Gone.php", "status": "deleted" },
            { "path": "src/dead_dir/Legacy.php", "status": "deleted" },
            { "path": "src/mixed_dir/Added.php", "status": "created" },
            { "path": "src/mixed_dir/Removed.php", "status": "deleted" },
            {
                "path": "src/Cart.php",
                "status": "modified",
                "entities": [
                    { "id": "m1", "kind": "method", "name": "total", "line": 19, "change": "modified" }
                ]
            }
        ]
    });
    let lookup = ChangesLookup::from_value(&changes).expect("lookup");
    assert_eq!(
        lookup.status_for("src/NewName.php", false).as_deref(),
        Some("renamed")
    );

    let surviving_folder =
        json!({ "path": "src/surviving_dir", "folder": true, "files": 3, "changed": 0 });
    let dead_folder = json!({ "path": "src/dead_dir", "folder": true, "files": 0, "changed": 0 });
    let mixed_folder = json!({ "path": "src/mixed_dir", "folder": true, "files": 2, "changed": 1 });
    let touched_sym = json!({ "path": "src/Cart.php#L19:.total()", "name": "Cart.total()", "kind": "symbol", "symbol": true, "line": 19, "changed": 0 });
    let untouched_sym = json!({ "path": "src/Cart.php#L12:Cart", "name": "Cart", "kind": "symbol", "symbol": true, "line": 12, "changed": 0 });
    let all_nodes = vec![
        surviving_folder.clone(),
        dead_folder.clone(),
        mixed_folder.clone(),
        touched_sym.clone(),
        untouched_sym.clone(),
    ];

    assert_eq!(
        lookup
            .status_for_node(&surviving_folder, &all_nodes)
            .as_deref(),
        Some("modified")
    );
    assert_eq!(
        lookup.status_for_node(&dead_folder, &all_nodes).as_deref(),
        Some("deleted")
    );
    assert_eq!(
        lookup.status_for_node(&mixed_folder, &all_nodes).as_deref(),
        Some("modified")
    );
    assert_eq!(
        lookup.status_for_node(&touched_sym, &all_nodes).as_deref(),
        Some("modified")
    );
    assert_eq!(lookup.status_for_node(&untouched_sym, &all_nodes), None);
}

#[test]
fn canvas_renders_renamed_pill_untouched_symbols_synthetic_deleted_and_commit_edge_diffs() {
    // 1. Renamed file + surviving folder vs deleted-only + synthetic deleted file + commit edge_diffs
    let view = json!({
        "mode": "changes",
        "scope": "",
        "chosen": { "kind": "commit", "head": { "commit": "abc123456789" } },
        "nodes": [
            { "path": "src/NewName.php", "name": "NewName.php", "folder": false, "changed": 1, "status": "R", "links": 1 },
            { "path": "src/Caller.php", "name": "Caller.php", "folder": false, "changed": 0, "status": null, "links": 2 },
            { "path": "src/Dep.php", "name": "Dep.php", "folder": false, "changed": 0, "status": null, "links": 1 },
            { "path": "src/surviving", "name": "surviving", "folder": true, "files": 4, "changed": 0, "status": null, "links": 0 }
        ],
        "edges": [
            { "from": "src/NewName.php", "to": "src/Dep.php", "links": 1, "kinds": [{ "kind": "calls", "links": 1 }] }
        ],
        "frames": []
    });
    let changes = json!({
        "target": "commit:abc123456789",
        "files": [
            { "path": "src/NewName.php", "previous_path": "src/OldName.php", "status": "renamed" },
            { "path": "src/Removed.php", "status": "deleted", "callers": ["src/Caller.php"] },
            { "path": "src/surviving/Gone.php", "status": "deleted" }
        ],
        "edge_diffs": {
            "added": [{ "from": "src/NewName.php", "to": "src/Dep.php", "kinds": ["calls"] }],
            "removed": [{ "from": "src/Caller.php", "to": "src/Dep.php", "kinds": ["imports"] }],
            "risk_callers": [{ "from": "src/Caller.php", "to": "src/Removed.php", "impact": "broken_risk" }]
        }
    });
    let rendered = canvas_with_overlays(
        &view,
        "graph-map",
        CanvasOverlayContext {
            changes: Some(&changes),
            ..Default::default()
        },
    );
    assert!(
        rendered.html.contains("data-commit-changes=\"true\""),
        "{}",
        rendered.html
    );
    assert!(rendered.html.contains("data-change-status=\"renamed\""));
    assert!(rendered.html.contains("<span class=\"graph-map-node-status\" data-status=\"renamed\" aria-hidden=\"true\">R</span>"));
    assert!(
        rendered
            .html
            .contains("data-payload=\"&quot;src/Removed.php&quot;\""),
        "{}",
        rendered.html
    );
    assert!(rendered.html.contains("data-change-status=\"deleted\""));
    assert!(rendered.html.contains("<span class=\"graph-map-node-status\" data-status=\"deleted\" aria-hidden=\"true\">D</span>"));
    assert!(rendered.html.contains("data-impact=\"caller-risk\""));
    assert!(rendered.html.contains("data-impact=\"broken_risk\""));
    assert!(rendered.html.contains("data-commit-touched=\"true\""));
    assert!(rendered.html.contains("data-diff=\"added\""));
    assert!(rendered.html.contains("data-diff=\"removed\""));
    // Surviving folder with a deleted file inside must be modified, not deleted!
    assert!(!rendered.html.contains("data-payload=\"&quot;src/surviving&quot;\" data-folder=\"true\" data-kind=\"\" data-open=\"true\" data-outside=\"false\" data-changed=\"false\" data-selected=\"false\" data-dim=\"false\" data-cycle=\"false\" data-flow=\"\" data-out=\"false\" data-in=\"false\" data-start=\"false\" data-change-status=\"deleted\""));

    // 2. Symbol over-highlighting fix in mode == "file"
    let file_view = json!({
        "mode": "file",
        "scope": "src/Cart.php",
        "chosen": { "kind": "commit", "head": { "commit": "abc123456789" } },
        "file": { "path": "src/Cart.php", "name": "Cart.php", "status": "M", "symbols": 2, "types": 1, "methods": 1, "functions": 0, "others": 0, "inside": 0, "out": 0, "in": 0, "uses": 0, "used_by": 0 },
        "nodes": [
            { "path": "src/Cart.php#L19:.total()", "name": "Cart.total()", "kind": "symbol", "line": 19, "symbol": true, "folder": false, "changed": 1, "status": "M" },
            { "path": "src/Cart.php#L12:Cart", "name": "Cart", "kind": "symbol", "line": 12, "symbol": true, "folder": false, "changed": 0, "status": null }
        ],
        "edges": [],
        "frames": []
    });
    let file_changes = json!({
        "target": "commit:abc123456789",
        "files": [
            {
                "path": "src/Cart.php",
                "status": "modified",
                "entities": [{ "id": "m1", "name": "total", "line": 19, "change": "modified" }]
            }
        ]
    });
    let file_rendered = canvas_with_overlays(
        &file_view,
        "graph-map",
        CanvasOverlayContext {
            changes: Some(&file_changes),
            ..Default::default()
        },
    );
    assert_eq!(
        file_rendered
            .html
            .matches("data-change-status=\"modified\"")
            .count(),
        1,
        "{}",
        file_rendered.html
    );
    assert_eq!(
        file_rendered.html.matches("graph-map-node-status").count(),
        1,
        "{}",
        file_rendered.html
    );

    // 3. Fallback safety when edge_diffs is absent
    let no_edge_diffs = json!({
        "target": "commit:abc123456789",
        "files": [{ "path": "src/NewName.php", "status": "renamed" }]
    });
    assert!(ImpactLookup::from_sources(
        None,
        Some(&no_edge_diffs),
        &view["nodes"].as_array().cloned().unwrap()
    )
    .is_none());
}

#[test]
fn legend_shows_all_diff_swatches_when_change_highlights_are_active() {
    let html = legend(&change(), true);
    for class in [
        "graph-map-key-uses",
        "graph-map-key-used",
        "graph-map-key-diff-added",
        "graph-map-key-diff-modified",
        "graph-map-key-diff-removed",
        "graph-map-key-diff-renamed",
        "graph-map-key-diff-risk",
        "graph-map-key-diff-touched",
    ] {
        assert!(html.contains(class), "missing {class} in {html}");
    }
    assert!(html.contains("Added/Created"));
    assert!(html.contains("Modified"));
    assert!(html.contains("Deleted"));
    assert!(html.contains("Renamed"));
    assert!(html.contains("Caller risk"));

    let plain = legend(&view(), false);
    assert!(!plain.contains("graph-map-key-diff-"));
}

#[test]
fn commit_graph_dock_and_banner_helpers_render_and_handle_actions() {
    let mut dock = fresh_commit_dock("project:p1");
    dock["rows"] = json!([
        {
            "commit": "aaaa1111bbbb2222",
            "short": "aaaa111",
            "subject": "Add commit graph",
            "author": "Alice",
            "at": 1_794_396_400,
            "refs": [{ "name": "main", "kind": "head" }],
            "merge": false,
            "parents": ["cccc3333dddd4444"]
        },
        {
            "commit": "cccc3333dddd4444",
            "short": "cccc333",
            "subject": "Base commit",
            "author": "Bob",
            "at": 1_794_313_600,
            "refs": [],
            "merge": false,
            "parents": []
        }
    ]);
    dock["more"] = json!(true);

    let list_html = render_commit_dock_list(&dock, "commit:aaaa1111bbbb2222", 1_794_400_000);
    assert!(list_html.contains("class=\"git-graph-row\""), "{list_html}");
    assert!(
        list_html.contains("data-action=\"graph-commit-select\""),
        "{list_html}"
    );
    assert!(list_html.contains("aria-selected=\"true\""), "{list_html}");
    assert!(
        list_html.contains("data-action=\"graph-commit-graph-more\""),
        "{list_html}"
    );
    assert_eq!(
        commit_dock_note(&dock),
        "2 commits · click a commit to inspect its changes"
    );

    let commit_view = json!({
        "chosen": { "kind": "commit", "head": { "commit": "aaaa1111bbbb2222" } },
        "commit_info": {
            "commit": "aaaa1111bbbb2222",
            "short": "aaaa111",
            "subject": "Add commit graph",
            "author": "Alice",
            "parents": ["cccc3333dddd4444"],
            "refs": [{ "name": "main", "kind": "head" }]
        }
    });
    let changes_catalog = json!({
        "created_files": 1,
        "modified_files": 2,
        "deleted_files": 1,
        "renamed_files": 1
    });
    let banner = render_commit_banner_for_view(&commit_view, Some(&changes_catalog));
    assert!(banner.contains("aaaa111"), "{banner}");
    assert!(banner.contains("Add commit graph"), "{banner}");
    assert!(
        banner.contains("data-action=\"graph-commit-select\""),
        "{banner}"
    );
    assert!(
        banner.contains("data-action=\"graph-commit-reset\""),
        "{banner}"
    );
    assert_eq!(render_commit_banner_for_view(&view(), None), "");

    assert_eq!(
        commit_action_target("graph-commit-select", &json!("commit:cccc3333dddd4444")),
        Some("commit:cccc3333dddd4444".to_owned())
    );
    assert_eq!(
        commit_action_target("graph-commit-reset", &V::Null),
        Some(String::new())
    );
    assert_eq!(commit_action_target("graph-commit-select", &V::Null), None);
}

#[test]
fn weighted_barycenter_and_pin_overrides_preserve_user_positions() {
    let edges = vec![(0, 1, 10), (0, 2, 1)];
    let mut plan = plan_roomy(
        3,
        &edges,
        &Frames::default(),
        None,
        Routing::PLAIN,
        &[0.0, 0.0, 0.0],
    );
    let keys = vec![
        "src/A.php".to_owned(),
        "src/Heavy.php".to_owned(),
        "src/Light.php".to_owned(),
    ];
    let mut pins = BTreeMap::new();
    pins.insert("src/Heavy.php".to_owned(), (520.0, 310.0));
    let moved = apply_pin_overrides_to_plan(&mut plan, &keys, &[(0, 1), (0, 2)], &pins);
    assert!(moved.contains(&1));
    assert!(!moved.contains(&0));
    assert!((plan.nodes[1].x - 520.0).abs() < 0.01);
    assert!((plan.nodes[1].y - 310.0).abs() < 0.01);
    assert!(plan.width >= 520.0 + BOX_WIDTH + MARGIN);
}

#[test]
fn synthetic_edge_path_routes_around_blocking_intermediate_cards() {
    let placed = vec![
        Placed {
            x: 10.0,
            y: 120.0,
            layer: Some(0),
            cycle: false,
        },
        Placed {
            x: 240.0,
            y: 120.0,
            layer: Some(1),
            cycle: false,
        },
        Placed {
            x: 470.0,
            y: 120.0,
            layer: Some(2),
            cycle: false,
        },
    ];
    let routed = synthetic_edge_path(0, 2, &placed, 1);
    assert!(routed.starts_with('M'));
    assert!(
        routed.contains('C') && routed.contains("102.0"),
        "channel-routed edge across an intermediate node should detour via horizontal corridor: {routed}"
    );
}

#[test]
fn modal_lenses_side_tabs_paged_paths_and_memory_bridge_follow_contract() {
    use crate::test_support::assert_no_inline_hex_colors;

    assert_eq!(active_modal_lens(&json!({}), &json!({})), "dependencies");
    assert_eq!(
        active_modal_lens(&json!({"target": "HEAD~1"}), &json!({})),
        "changes"
    );
    assert_eq!(
        active_modal_lens(&json!({"patterns": true}), &json!({})),
        "architecture"
    );
    assert_eq!(
        active_modal_lens(&json!({"failing": ["tests/OrderTest.php"]}), &json!({})),
        "tests"
    );
    assert_eq!(
        active_modal_lens(&json!({"history": true}), &json!({})),
        "runs"
    );

    let tabs = side_tabs_html("analysis", 3, "modal");
    assert!(tabs.contains("Details"));
    assert!(tabs.contains("Changes") && tabs.contains("(3)"));
    assert!(tabs.contains("Analysis"));
    assert!(tabs.contains("aria-selected=\"true\""));
    assert_no_inline_hex_colors(&tabs);

    let scoped = build_scoped_map_path(
        &GraphTargetScope::single("proj-1"),
        &json!({
            "scope": "src",
            "offset": 50,
            "limit": 120,
            "isolate_active": true,
            "isolate_depth": 3
        }),
        false,
    );
    assert!(scoped.contains("offset=50"), "{scoped}");
    assert!(scoped.contains("limit=120"), "{scoped}");
    assert!(scoped.contains("depth=3"), "{scoped}");

    let bridge = render_side_memory_bridge_html(
        "proj-1",
        &json!({
            "path": "src/Order.php",
            "brain_records": [{"id": "rec-1", "title": "Order flow"}],
            "memory_chunks": [],
            "omnigraph_atoms": []
        }),
    );
    assert!(bridge.contains("Open in Context Graph"), "{bridge}");
    assert!(
        bridge.contains("data-action=\"graph-map-open-omnigraph\""),
        "{bridge}"
    );
    assert_no_inline_hex_colors(&bridge);
}

#[test]
fn dense_canvas_captures_node_htmls_preserves_wire_edge_indices_and_refreshes_scene() {
    let mut nodes = Vec::with_capacity(185);
    for i in 0..185 {
        nodes.push(json!({
            "path": format!("src/n{i}.php"),
            "name": format!("N{i}.php"),
            "layer": i % 3,
            "in": 1,
            "out": 1,
        }));
    }
    let edges = json!([
        {"from": "src/n0.php", "to": "src/n1.php", "links": 2},
        {"from": "src/n1.php", "to": "src/n2.php", "links": 2},
        // Corridor/skip edge 0 -> 2: filtered when hide_transitive = true, or kept when hide_transitive = false
        {"from": "src/n0.php", "to": "src/n2.php", "links": 1}
    ]);
    let view = json!({
        "nodes": nodes,
        "edges": edges,
        "frames": [],
        "pattern": {"declared": false},
    });
    // Tight viewport near origin; node 184 lies well offscreen while roving_focus = Some(184).
    let tight_bounds = ViewportBounds {
        min_x: 0.0,
        min_y: 0.0,
        max_x: 260.0,
        max_y: 180.0,
    };
    let overlay = CanvasOverlayContext {
        live_focus: None,
        changes: None,
        impact: None,
        links_off: [false; 7],
        hide_transitive: false,
        cone: (0, 0),
        pinned_positions: None,
        path_nodes: None,
        path_edges: None,
        isolate_cone: None,
        viewport_bounds: Some(tight_bounds),
        roving_focus: Some(184),
    };
    let rendered = canvas_with_overlays(&view, "graph-map", overlay);
    assert_eq!(rendered.dense_node_htmls.len(), 185);
    let scene = rendered.dense_scene.as_ref().expect("dense_scene expected");
    assert_eq!(scene.nodes.len(), 185);
    // Roving focus node (184) must stay mounted even when offscreen, whereas nearby offscreen node 183 is skipped from initial HTML.
    assert!(
        rendered.html.contains("data-n=\"184\"") && rendered.html.contains("tabindex=\"0\""),
        "roving focus node must stay mounted in dense html"
    );
    assert!(
        !rendered.html.contains("data-n=\"183\""),
        "offscreen non-important node should be omitted from initial html"
    );
    assert!(
        rendered.dense_node_htmls[183].is_empty(),
        "dense_node_htmls must defer offscreen HTML allocation until roving focus or mount"
    );
    assert!(
        rendered
            .materialize_dense_node_html(&view, 183)
            .is_some_and(|h| h.contains("data-n=\"183\"")),
        "lazy materialize_dense_node_html must render omitted node HTML on demand"
    );
    assert!(!rendered.html.contains("graph-map-edge-mark"));
    assert!(!rendered.html.contains("graph-map-edge-chevron"));
    assert!(!rendered.html.contains("graph-map-violation-halo"));

    // Every DenseEdgeSpec index points back into pairs (valid even for multi-segment wires).
    let pairs = pairs_for_view(
        &view,
        &CanvasOverlayContext {
            live_focus: None,
            changes: None,
            impact: None,
            links_off: [false; 7],
            hide_transitive: false,
            cone: (0, 0),
            pinned_positions: None,
            path_nodes: None,
            path_edges: None,
            isolate_cone: None,
            viewport_bounds: None,
            roving_focus: None,
        },
    );
    assert!(scene.edges.iter().all(|e| e.index < pairs.len()));
    assert!(
        scene.edges.iter().any(|e| {
            e.cmds
                .iter()
                .any(|c| matches!(c, graph_viewport::canvas2d::DensePathCmd::Line(_, _)))
                && e.cmds
                    .iter()
                    .any(|c| matches!(c, graph_viewport::canvas2d::DensePathCmd::Curve { .. }))
        }),
        "skip-layer corridor wire should retain MoveTo+CurveTo+LineTo segments"
    );

    // Patch selection + traced path onto cached DenseScene in place.
    let mut path_view = view.clone();
    path_view["focus"] = json!({"path": "src/n0.php"});
    let mut path_nodes = BTreeSet::new();
    path_nodes.insert(0);
    path_nodes.insert(1);
    let mut path_edges = BTreeSet::new();
    path_edges.insert((0, 1));
    let mut pins = BTreeMap::new();
    pins.insert("src/n0.php".to_owned(), (90.0, 90.0));
    let patch_overlay = CanvasOverlayContext {
        live_focus: None,
        changes: None,
        impact: None,
        links_off: [false; 7],
        hide_transitive: false,
        cone: (0, 0),
        pinned_positions: Some(&pins),
        path_nodes: Some(&path_nodes),
        path_edges: Some(&path_edges),
        isolate_cone: None,
        viewport_bounds: None,
        roving_focus: None,
    };
    let patch = compute_canvas_selection_patch(
        &path_view,
        &rendered.plan_nodes,
        &rendered.plan_frames,
        &pairs,
        &patch_overlay,
    );
    let mut updated_scene = scene.clone();
    refresh_dense_scene_for_patch(
        &mut updated_scene,
        &pairs,
        &rendered.node_keys,
        &patch,
        &patch_overlay,
    );
    assert!(updated_scene.nodes[0].selected);
    assert!(updated_scene.nodes[0].lit);
    assert!(updated_scene.nodes[0].pinned);
    assert!(updated_scene.nodes[1].lit);
    assert!(updated_scene.nodes[2].dim);
    assert!(updated_scene
        .edges
        .iter()
        .any(|e| pairs[e.index] == (0, 1) && e.on && e.lit));
    assert!(updated_scene
        .edges
        .iter()
        .any(|e| pairs[e.index] == (1, 2) && e.dim && !e.on));
}

#[test]
fn compare_base_range_targets_and_open_full_state_transfer_work() {
    let range_state = json!({
        "target": "head_sha",
        "compare_base": "base_sha",
        "traced_path": {"from": "a.php", "to": "b.php"},
        "isolate_active": true,
        "isolate_depth": 3,
        "failing": ["tests/AuthTest.php"],
        "roving_focus": 4,
        "side_tab": "analysis",
        "patterns": true,
        "history": false,
        "modules": true,
        "no_incoming": true,
        "lens_tests": true,
        "fit": false,
    });
    assert_eq!(effective_target(&range_state), "range:base_sha..head_sha");
    assert_eq!(effective_target_from("range:x..y", "ignored"), "range:x..y");
    assert_eq!(effective_target_from("head_sha", ""), "head_sha");

    let map_url = build_scoped_map_path(&GraphTargetScope::single("proj-1"), &range_state, false);
    assert!(
        map_url.contains("target=range%3Abase_sha..head_sha"),
        "scoped map path must use range:<base>..<head>: {map_url}"
    );
    let changes_url =
        build_scoped_changes_path_for_state(&GraphTargetScope::single("proj-1"), &range_state, "");
    assert!(
        changes_url.contains("target=range%3Abase_sha..head_sha"),
        "scoped changes path must use range:<base>..<head>: {changes_url}"
    );
    assert_eq!(
        active_modal_lens(&json!({"compare_base": "b", "target": "h"}), &json!({})),
        "changes"
    );

    let transferred = transfer_surface_state_to_modal(&range_state, fresh("head_sha"));
    assert_eq!(transferred["traced_path"]["from"], json!("a.php"));
    assert_eq!(transferred["traced_path"]["to"], json!("b.php"));
    assert_eq!(transferred["isolate_active"], json!(true));
    assert_eq!(transferred["isolate_depth"], json!(3));
    assert_eq!(transferred["failing"], json!(["tests/AuthTest.php"]));
    assert_eq!(transferred["roving_focus"], json!(4));
    assert_eq!(transferred["compare_base"], json!("base_sha"));
    assert_eq!(transferred["side_tab"], json!("analysis"));
    assert_eq!(transferred["patterns"], json!(true));
    assert_eq!(transferred["modules"], json!(true));
    assert_eq!(transferred["no_incoming"], json!(true));
    assert_eq!(transferred["lens_tests"], json!(true));
    assert_eq!(transferred["fit"], json!(false));

    // Default isolate_depth is 2 and secondary navigation modes survive modal promotion.
    let sparse_state = json!({
        "target": "head_sha",
        "compare_base": "base_sha",
        "out": ["src/a.php"],
        "into": ["src/b.php"],
        "follow": "src/a.php",
        "shared": true,
    });
    let transferred_sparse = transfer_surface_state_to_modal(&sparse_state, fresh("head_sha"));
    assert_eq!(transferred_sparse["isolate_depth"], json!(2));
    assert_eq!(transferred_sparse["out"], json!(["src/a.php"]));
    assert_eq!(transferred_sparse["into"], json!(["src/b.php"]));
    assert_eq!(transferred_sparse["follow"], json!("src/a.php"));
    assert_eq!(transferred_sparse["shared"], json!(true));

    let scoped_map_req =
        build_scoped_graph_request(&GraphTargetScope::single("proj-1"), vec![], &range_state);
    assert!(
        scoped_map_req.contains("target=range%3Abase_sha..head_sha"),
        "{scoped_map_req}"
    );
    let scoped_find_req =
        build_scoped_find_path(&GraphTargetScope::single("proj-1"), &range_state, "Auth")
            .expect("find URL");
    assert!(
        scoped_find_req.contains("target=range%3Abase_sha..head_sha"),
        "{scoped_find_req}"
    );
}

#[test]
fn handles_preserve_node_index_hit_paths_skip_tabindex_and_roving_respects_isolate_cone() {
    let handle_html = handles(
        &json!({
            "name": "Service.php",
            "path": "src/Service.php",
            "more_out": 3,
            "unfolded_out": true,
            "more_in": 2,
            "unfolded_in": false,
        }),
        1,
        180.0,
        40.0,
        false,
    );
    assert!(handle_html.contains("data-of=\"1\""), "{handle_html}");
    assert!(!handle_html.contains("data-of=\"344\""), "{handle_html}");
    assert!(!handle_html.contains("data-of=\"355\""), "{handle_html}");

    let view = json!({
        "nodes": [
            {"path": "src/a.php", "name": "a.php"},
            {"path": "src/b.php", "name": "b.php"},
            {"path": "src/c.php", "name": "c.php"},
            {"path": "src/d.php", "name": "d.php"}
        ],
        "edges": [
            {"from": "src/a.php", "to": "src/b.php", "links": 1},
            {"from": "src/c.php", "to": "src/d.php", "links": 1}
        ],
        "frames": [],
    });
    let mut cone = BTreeSet::new();
    cone.insert(2);
    cone.insert(3);
    let rendered = canvas_with_overlays(
        &view,
        "graph-map",
        CanvasOverlayContext {
            isolate_cone: Some(&cone),
            roving_focus: Some(0),
            ..Default::default()
        },
    );
    assert!(
        rendered.html.contains("class=\"graph-map-hit\"")
            && rendered.html.contains("data-action=\"graph-map-link\""),
        "{}",
        rendered.html
    );
    assert!(
        !rendered
            .html
            .contains("tabindex=\"0\" role=\"button\"><title>"),
        "SVG hit paths must not hold tabindex=\"0\": {}",
        rendered.html
    );
    // Node 0 is outside cone, so node 2 (first node in cone) receives roving tabindex="0".
    assert!(
        rendered.html.contains("data-n=\"2\"") && rendered.html.contains("tabindex=\"0\""),
        "{}",
        rendered.html
    );
}

#[test]
fn selection_patch_highlights_traced_paths_and_edge_impact_ignores_empty_target_wildcards() {
    let nodes = vec![
        json!({"path": "src/a.php", "name": "a.php"}),
        json!({"path": "src/b.php", "name": "b.php"}),
        json!({"path": "src/c.php", "name": "c.php"}),
    ];
    let pairs = vec![(0, 1), (0, 2)];
    let mut path_nodes = BTreeSet::new();
    path_nodes.insert(0);
    path_nodes.insert(2);
    let mut path_edges = BTreeSet::new();
    path_edges.insert((0, 2));
    let plan_nodes = vec![
        Placed {
            x: 10.0,
            y: 10.0,
            layer: Some(0),
            cycle: false,
        };
        3
    ];
    let patch = compute_canvas_selection_patch(
        &json!({"nodes": nodes, "edges": [], "frames": []}),
        &plan_nodes,
        &[],
        &pairs,
        &CanvasOverlayContext {
            path_nodes: Some(&path_nodes),
            path_edges: Some(&path_edges),
            ..Default::default()
        },
    );
    assert_eq!(patch.edges_by_pair[&(0, 2)].edge_state, "on");
    assert!(patch.edges_by_pair[&(0, 2)].shared_path);
    assert_eq!(patch.edges_by_pair[&(0, 1)].edge_state, "off");
    assert!(patch.nodes[0].lit && !patch.nodes[0].dim);
    assert!(!patch.nodes[1].lit && patch.nodes[1].dim);
    assert!(patch.nodes[2].lit && !patch.nodes[2].dim);

    // An empty `to` in `risk_callers` must not falsely mark unrelated outgoing edges as `broken_risk`.
    let changes_doc = json!({
        "target": "work",
        "edge_diffs": {
            "risk_callers": [
                {"from": "src/a.php", "to": "", "impact": "broken_risk"},
                {"from": "src/a.php", "to": "src/c.php", "impact": "broken_risk"}
            ]
        }
    });
    let lookup = ImpactLookup::from_sources(None, Some(&changes_doc), &nodes).expect("lookup");
    assert_eq!(lookup.edge_impact(&nodes, 0, 1), None);
    assert_eq!(lookup.edge_impact(&nodes, 0, 2), Some("broken_risk"));
}

#[test]
fn pin_overrides_expand_frames_and_confidence_cycle_hints_and_commit_banner_gate_work() {
    let mut plan = plan_beside(
        2,
        &[(0, 1, 1)],
        &Frames {
            of: vec![Some(0), None],
            holder: vec![None],
        },
        None,
    );
    plan.frames = vec![Framed {
        x: plan.nodes[0].x - 8.0,
        y: plan.nodes[0].y - 22.0,
        width: BOX_WIDTH + 16.0,
        height: BOX_HEIGHT + 30.0,
    }];
    let keys = vec!["src/a.php".to_owned(), "src/b.php".to_owned()];
    let mut pins = BTreeMap::new();
    pins.insert("src/a.php".to_owned(), (900.0, 500.0));
    let moved = apply_pin_overrides_to_plan(&mut plan, &keys, &[(0, 1)], &pins);
    assert!(moved.contains(&0));
    assert!(plan.frames[0].x + plan.frames[0].width >= 900.0 + BOX_WIDTH);
    assert!(plan.frames[0].y + plan.frames[0].height >= 500.0 + BOX_HEIGHT);

    // Non-commit views must not render stale commit range banners.
    assert_eq!(
        render_commit_banner_for_view(&json!({"mode": "whole"}), None),
        ""
    );

    let view = json!({
        "confidence": {"extracted": 8, "inferred": 2, "ratio": 0.8},
        "circles": {
            "groups": 1,
            "here": 1,
            "files": 3,
            "largest": 3,
            "listed": [{"start": "src/a.php", "files": 3, "names": ["a.php", "b.php"], "more": 1}]
        },
        "circle_remediation": [{
            "files": 3,
            "cuts": [{"from": "src/b.php", "to": "src/a.php", "links": 2, "breaks_cycles": true, "hint": "Invert via interface"}]
        }],
        "architecture": {
            "declared": true,
            "layers": [{"id": "ui", "name": "UI"}],
            "violations": [],
            "remediation": [{"from": "src/ui/View.php", "to": "src/infra/Db.php", "from_layer": "ui", "to_layer": "infra", "links": 3, "guessed": true, "hint": "Route through domain port"}]
        }
    });
    let pill = confidence_pill_html(&view);
    assert!(pill.contains("80% extracted"), "{pill}");
    let changes_pill =
        confidence_pill_html(&json!({"confidence": {"extracted_links": 3, "inferred_links": 1}}));
    assert!(changes_pill.contains("75% extracted"), "{changes_pill}");
    let circles = circles_side(&view);
    assert!(circles.contains("Suggested cycle breaks"), "{circles}");
    assert!(
        circles.contains("breaks cycle · 2 links · Invert via interface"),
        "{circles}"
    );
    let arch = architecture_side(&view);
    assert!(arch.contains("80% extracted"), "{arch}");
    assert!(arch.contains("Suggested layer breaks"), "{arch}");
    assert!(
        arch.contains("3 links · inferred · Route through domain port"),
        "{arch}"
    );
    let cycle_view = side(&json!({
        "mode": "cycle",
        "cycle": {
            "places": [{"path": "src/a.php"}, {"path": "src/b.php"}],
            "cut_hints": [{"from": "src/b.php", "to": "src/a.php", "links": 1, "breaks_cycles": 2, "hint": "Break cycle edge"}]
        }
    }));
    assert!(
        cycle_view.contains("Suggested cycle breaks"),
        "{cycle_view}"
    );
    assert!(
        cycle_view.contains("breaks 2 · 1 link · Break cycle edge"),
        "{cycle_view}"
    );
    let layers_view = layers_side(&json!({
        "pattern": {
            "declared": true,
            "against": {"pairs": 0, "listed": []},
            "remediation": [{"from": "src/x.php", "to": "src/y.php", "from_layer": "ui", "to_layer": "db", "links": 2, "hint": "Decouple db call"}]
        }
    }));
    assert!(
        layers_view.contains("Suggested layer breaks"),
        "{layers_view}"
    );
    assert!(layers_view.contains("Decouple db call"), "{layers_view}");
    crate::test_support::assert_no_inline_hex_colors(&format!(
        "{pill}{changes_pill}{circles}{arch}{cycle_view}{layers_view}"
    ));
}

#[test]
fn dense_graph_skips_offscreen_extras_initially_and_populates_dense_meta() {
    let nodes: Vec<V> = (0..185)
        .map(|i| {
            json!({
                "path": format!("src/mod_{i}.php"),
                "name": format!("mod_{i}.php"),
                "folder": i % 15 == 0,
                "out": 1,
                "in": 1,
            })
        })
        .collect();
    let edges: Vec<V> = (0..184)
        .map(|i| json!({"from": format!("src/mod_{i}.php"), "to": format!("src/mod_{}.php", i + 1), "links": 2, "kinds": {"calls": 2}}))
        .collect();
    let view = json!({
        "nodes": nodes,
        "edges": edges,
        "frames": [],
        "spread": [],
    });
    let rendered = canvas_with_overlays(
        &view,
        "graph-map",
        CanvasOverlayContext {
            viewport_bounds: Some(ViewportBounds {
                min_x: 0.0,
                min_y: 0.0,
                max_x: 240.0,
                max_y: 120.0,
            }),
            ..Default::default()
        },
    );
    assert!(rendered.dense_scene.is_some());
    let meta = rendered.dense_meta.as_ref().expect("dense meta");
    assert_eq!(meta.edges.len(), 184);
    assert_eq!(meta.nodes.len(), 185);
    assert!(meta
        .edges
        .get(&0)
        .is_some_and(|em| em.explain_kind == "link"
            && em.from_index == Some(0)
            && em.to_index == Some(1)));
    assert_eq!(rendered.dense_extra_specs.len(), 185);
    // Lazily mount extras for an offscreen folder node when revealed by pan.
    if let Some(Some(folder_spec)) = rendered
        .dense_extra_specs
        .iter()
        .find(|s| s.as_ref().is_some_and(|x| x.folder_spread))
    {
        let extra = render_dense_node_extras_for_mounted(&view, folder_spec, None);
        assert!(extra.contains("data-of="), "{extra}");
        assert!(extra.contains("graph-map-spread"), "{extra}");
    }
}

#[test]
fn dense_portals_route_valid_actions_and_patch_preserves_semantics_and_isolate() {
    let nodes: Vec<V> = (0..185)
        .map(|i| {
            json!({
                "path": if i == 0 { "sem:auth.guard".to_owned() } else { format!("src/f_{i}.php") },
                "name": if i == 0 { "AuthGuard".to_owned() } else { format!("f_{i}") },
                "node_kind": if i == 0 { "semantic" } else { "file" },
                "visibility": if i == 1 { "public" } else { "" },
                "in_cycle": i == 1,
                "folder": false,
                "out": 1,
                "in": 1,
            })
        })
        .collect();
    let edges: Vec<V> = (0..184)
        .map(|i| {
            json!({
                "from": if i == 0 { "sem:auth.guard".to_owned() } else { format!("src/f_{i}.php") },
                "to": format!("src/f_{}.php", i + 1),
                "links": 1,
                "kinds": {"calls": 1}
            })
        })
        .collect();
    let view = json!({
        "mode": "map",
        "nodes": nodes,
        "edges": edges,
        "frames": [],
        "spread": [],
        "focus": {"path": "src/f_1.php"},
        "lod": {"tier": "overview", "shown": 185, "total": 900},
        "cycle": {"witness": ["src/f_1.php", "src/f_2.php", "src/f_1.php"], "cut_hints": [{"from": "src/f_2.php", "to": "src/f_1.php", "links": 1}]},
    });
    let impact = json!({
        "path": "src/f_1.php",
        "uses": {
            "added": [{"target": "src/f_2.php", "kind": "calls"}],
            "removed": [{"target": "src/f_99.php", "kind": "imports"}],
            "unchanged": []
        },
        "used_by": []
    });
    let mut iso = BTreeSet::new();
    iso.insert(0);
    iso.insert(1);
    iso.insert(2);
    let overlay_ctx = CanvasOverlayContext {
        impact: Some(&impact),
        isolate_cone: Some(&iso),
        ..Default::default()
    };
    let mut rendered = canvas_with_overlays(&view, "graph-map", overlay_ctx);
    let mut scene = rendered.dense_scene.take().expect("dense scene");
    let meta = rendered.dense_meta.as_ref().expect("dense meta");
    // P0 #3: synthetic removed edge outside isolate_cone is excluded, added edge (1->2) is included.
    let syn_added = scene
        .edges
        .iter()
        .find(|e| e.diff == "added")
        .expect("synthetic added edge in dense scene");
    assert!(meta
        .edges
        .get(&syn_added.index)
        .is_some_and(|m| m.summary.contains("-> f_2")));
    assert!(!scene.edges.iter().any(|e| e.diff == "removed"));

    // Refresh patch retains diff/impact on dense edge specs.
    let overlay_ctx = CanvasOverlayContext {
        impact: Some(&impact),
        isolate_cone: Some(&iso),
        ..Default::default()
    };
    let patch = compute_canvas_selection_patch(
        &view,
        &rendered.plan_nodes,
        &rendered.plan_frames,
        &rendered.pairs,
        &overlay_ctx,
    );
    refresh_dense_scene_for_patch(
        &mut scene,
        &rendered.pairs,
        &rendered.node_keys,
        &patch,
        &overlay_ctx,
    );
    assert!(scene
        .edges
        .iter()
        .any(|e| e.index == 1 && e.diff == "added"));

    // Semantic hub node metadata and progressive LOD + witness display.
    let sem_meta = meta.nodes.get(&0).expect("semantic node meta");
    assert_eq!(sem_meta.symbol_kind, "contract");
    assert!(rendered.dense_node_htmls[0].is_empty());
    assert!(rendered
        .materialize_dense_node_html(&view, 0)
        .is_some_and(|h| h.contains("data-semantic=\"true\"")));
    assert!(!rendered.html.contains("graph-map-edge-synthetic"));
    assert!(!rendered.html.contains("graph-map-edge-mark"));
    assert!(note(&view).contains("Progressive LOD (overview)"));
    let cut_html = cycle_cut_hints_html(&view);
    assert!(cut_html.contains("<strong>Cycle witness</strong>:"));
    assert!(cut_html.contains("src/f_1.php"));

    // P0 #2: portal extras route via valid actions and never emit unhandled "graph-map-symbol".
    let spec = DenseNodeExtraSpec {
        index: 1,
        x: 100.0,
        y: 100.0,
        portal_out: vec![2],
        ..Default::default()
    };
    let portal = render_dense_node_extras_for_mounted(&view, &spec, None);
    assert!(
        portal.contains("data-action=\"graph-map-node\""),
        "{portal}"
    );
    assert!(!portal.contains("graph-map-symbol"), "{portal}");
    let mut scen_view = view.clone();
    scen_view["mode"] = json!("scenario");
    let scen_portal = render_dense_node_extras_for_mounted(&scen_view, &spec, None);
    assert!(
        scen_portal.contains("data-action=\"graph-map-scenario-open\""),
        "{scen_portal}"
    );
    assert!(!scen_portal.contains("graph-map-symbol"), "{scen_portal}");
}

#[test]
fn pinned_wires_detours_synthetic_gutters_and_clean_modal_lens_selection() {
    // Horizontally overlapping cards route around box interiors instead of crossing straight through.
    let overlap_path = route_pinned_wire_path((10.0, 120.0), (50.0, 120.0), 0.0, 0.0, false);
    assert!(overlap_path.contains(" C"), "{overlap_path}");
    assert_ne!(
        route_pinned_wire_path((10.0, 120.0), (260.0, 120.0), 0.0, 0.0, true),
        route_pinned_wire_path((10.0, 120.0), (260.0, 120.0), 0.0, 0.0, false)
    );

    // synthetic_edge_path expands beyond 5 lane slots and stays below top folder header (>= 44.0).
    let placed = vec![
        Placed {
            x: 10.0,
            y: 120.0,
            layer: Some(0),
            cycle: false,
        },
        Placed {
            x: 200.0,
            y: 120.0,
            layer: Some(1),
            cycle: false,
        },
        Placed {
            x: 390.0,
            y: 120.0,
            layer: Some(2),
            cycle: false,
        },
    ];
    let slot1 = synthetic_edge_path(0, 2, &placed, 1);
    let slot7 = synthetic_edge_path(0, 2, &placed, 7);
    assert_ne!(slot1, slot7);
    let top_placed = vec![
        Placed {
            x: 10.0,
            y: 30.0,
            layer: Some(0),
            cycle: false,
        },
        Placed {
            x: 200.0,
            y: 30.0,
            layer: Some(1),
            cycle: false,
        },
        Placed {
            x: 390.0,
            y: 30.0,
            layer: Some(2),
            cycle: false,
        },
    ];
    let top_slot = synthetic_edge_path(0, 2, &top_placed, 1);
    let ctrl_y: f64 = top_slot
        .split(',')
        .next()
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        ctrl_y >= 44.0,
        "expected control y >= 44.0, got {ctrl_y} in {top_slot}"
    );

    assert_eq!(
        active_modal_lens(&json!({"lens_tests": true}), &json!({})),
        "tests"
    );
    assert_eq!(
        active_modal_lens(&json!({"lens_runs": true}), &json!({})),
        "runs"
    );
    assert_eq!(
        active_modal_lens(&json!({}), &json!({"mode": "patterns"})),
        "architecture"
    );
    assert_eq!(
        active_modal_lens(&json!({"target": "commit:abc1234"}), &json!({})),
        "changes"
    );
    assert_eq!(
        active_modal_lens(&json!({}), &json!({"mode": "map"})),
        "dependencies"
    );

    // Backend object witness (`cycle_witness` / `cycle.witness`) + `feedback_arc_cut` fallback + `telemetry.truncated`.
    let backend_cycle_view = json!({
        "mode": "cycle",
        "cycle_witness": {
            "length": 2,
            "files": ["src/a.rs", "src/b.rs", "src/a.rs"],
            "edges": [{"from": "src/a.rs", "to": "src/b.rs", "weight": 1, "provenance": []}]
        },
        "circle_remediation": [{
            "files": 2,
            "cuts": [],
            "feedback_arc_cut": [{"from": "src/b.rs", "to": "src/a.rs", "links": 1, "breaks_cycles": 1, "hint": "Cut feedback arc"}]
        }],
        "telemetry": {"truncated": true}
    });
    let backend_cut_html = cycle_cut_hints_html(&backend_cycle_view);
    assert!(
        backend_cut_html.contains("<strong>Cycle witness</strong>:"),
        "{backend_cut_html}"
    );
    assert!(
        backend_cut_html.contains("src/a.rs") && backend_cut_html.contains("src/b.rs"),
        "{backend_cut_html}"
    );
    assert!(
        backend_cut_html.contains("Cut feedback arc"),
        "{backend_cut_html}"
    );
    assert!(
        note(&json!({"mode": "map", "telemetry": {"truncated": true}}))
            .contains("Progressive LOD (overview)")
    );
    assert!(note(&json!({"mode": "map", "lod": "crate"})).contains("Progressive LOD (crate)"));
}

#[test]
fn modal_lens_switch_clears_submode_precedence_and_history_snapshot_restores_nulls() {
    for lens in ["dependencies", "changes", "architecture", "tests", "runs"] {
        let mut st = json!({
            "target": "commit:deadbeef",
            "compare_base": "main",
            "run_id": "sysrun-9",
            "cycle": "src/CycleA.php",
            "cycle_return": {"target": "work"},
            "cycle_base": {"place": "src/CycleA.php", "revision": "abc123"},
            "pattern": "Factory",
            "pattern_return": {"target": ""},
            "scenario": "src/Entry.php",
            "scenario_return": {"target": ""},
            "scenario_skip": 4,
            "shared": true,
            "path": ["src/A.php", "src/B.php"],
            "test_path_return": {"target": "work"},
            "traced_path": {"from": "src/A.php", "to": "src/B.php"},
            "isolate_active": true,
            "isolate_depth": 2,
            "offset": 50,
            "follow": "src/A.php",
            "out": ["src/B.php"],
            "into": ["src/C.php"],
            "side": "into",
            "skip": [1, 2],
            "history": true,
            "patterns": true,
            "modules": true,
            "lens_tests": true,
            "lens_runs": true,
            "failing": ["tests/OrderTest.php"]
        });
        assert!(apply_modal_lens_switch(&mut st, lens));
        assert!(st["cycle"].is_null(), "{lens}: cycle should clear");
        assert!(
            st["cycle_base"].is_null(),
            "{lens}: cycle_base should clear"
        );
        assert!(st["pattern"].is_null(), "{lens}: pattern should clear");
        assert!(st["scenario"].is_null(), "{lens}: scenario should clear");
        assert_eq!(st["scenario_skip"], json!(0), "{lens}");
        assert_eq!(st["shared"], json!(false), "{lens}");
        assert_eq!(st["path"], json!([]), "{lens}");
        assert!(
            st["traced_path"].is_null(),
            "{lens}: traced_path should clear"
        );
        assert_eq!(st["isolate_active"], json!(false), "{lens}");
        assert_eq!(st["offset"], json!(0), "{lens}");
        assert_eq!(st["follow"], json!(""), "{lens}");

        let url = build_scoped_map_path(&GraphTargetScope::single("proj-1"), &st, false);
        assert!(
            !url.contains("cycle=")
                && !url.contains("pattern=")
                && !url.contains("scenario=")
                && !url.contains("shared=")
                && !url.contains("path=")
                && !url.contains("depth=")
                && !url.contains("offset=")
                && !url.contains("follow="),
            "lens={lens} URL retained stale submode precedence: {url}"
        );
        assert_eq!(active_modal_lens(&st, &json!({})), lens);
    }

    assert!(!apply_modal_lens_switch(&mut json!({}), "unknown"));

    // Snapshot restore must overwrite non-null/active state when snap holds null or omits optional keys.
    let initial_snap = json!({
        "focus": "",
        "scope": "",
        "target": "",
        "compare_base": null,
        "run_id": null,
        "launch": null,
        "traced_path": null,
        "cycle": null,
        "cycle_base": null,
        "pattern": null,
        "side_tab": "details",
        "patterns": false,
        "history": false,
    });
    let active_state = json!({
        "focus": "src/CycleA.php",
        "scope": "src",
        "target": "work",
        "compare_base": "main",
        "run_id": "run-7",
        "launch": "2",
        "traced_path": {"from": "a", "to": "b"},
        "cycle": "src/CycleA.php",
        "cycle_base": {"place": "src/CycleA.php", "revision": "abc123"},
        "pattern": "Factory",
        "scenario": "src/Entry.php",
        "scenario_skip": 3,
        "shared": true,
        "path": ["src/A.php", "src/B.php"],
        "isolate_active": true,
        "isolate_depth": 3,
        "offset": 25,
        "follow": "src/CycleA.php",
        "out": ["src/B.php"],
        "into": ["src/C.php"],
        "spread": ["src"],
        "failing": ["tests/OrderTest.php"],
        "lens_tests": true,
        "lens_runs": true,
        "patterns": true,
        "modules": true,
        "history": true,
        "whole": true,
        "notests": true,
        "certain": true,
        "no_incoming": true,
        "side_tab": "analysis",
    });
    let mut restored = active_state.clone();
    apply_surface_history_snapshot(&mut restored, &initial_snap);
    assert_eq!(restored["focus"], json!(""));
    assert_eq!(restored["scope"], json!(""));
    assert_eq!(restored["target"], json!(""));
    assert!(restored["compare_base"].is_null());
    assert!(restored["run_id"].is_null());
    assert!(restored["launch"].is_null());
    assert!(restored["traced_path"].is_null());
    assert!(restored["cycle"].is_null());
    assert!(restored["cycle_base"].is_null());
    assert!(restored["pattern"].is_null());
    assert!(restored["scenario"].is_null());
    assert_eq!(restored["scenario_skip"], json!(0));
    assert_eq!(restored["shared"], json!(false));
    assert_eq!(restored["path"], json!([]));
    assert_eq!(restored["isolate_active"], json!(false));
    assert_eq!(restored["isolate_depth"], json!(2));
    assert_eq!(restored["offset"], json!(0));
    assert_eq!(restored["follow"], json!(""));
    assert_eq!(restored["out"], json!([]));
    assert_eq!(restored["into"], json!([]));
    assert_eq!(restored["spread"], json!([]));
    assert_eq!(restored["failing"], json!([]));
    assert_eq!(restored["lens_tests"], json!(false));
    assert_eq!(restored["lens_runs"], json!(false));
    assert_eq!(restored["modules"], json!(false));
    assert_eq!(restored["whole"], json!(false));
    assert_eq!(restored["notests"], json!(false));
    assert_eq!(restored["certain"], json!(false));
    assert_eq!(restored["no_incoming"], json!(false));
    assert_eq!(restored["side_tab"], json!("details"));

    // Leaving a cycle/pattern/isolated state back to a clean snapshot must trigger map reload.
    let mut cycle_only = initial_snap.clone();
    cycle_only["cycle"] = json!("src/CycleA.php");
    assert!(snapshot_opt_changed_scope(&restored, &cycle_only));
    assert!(!snapshot_opt_changed_scope(&restored, &initial_snap));
}

#[test]
fn progressive_lod_urls_and_cross_graph_code_map_context_are_preserved() {
    let coarse = json!({"auto_lod": true, "scale": 0.25, "scope": "", "focus": ""});
    assert_eq!(
        surfaces::progressive_lod_for_state(&coarse, None),
        (Some("crate".to_owned()), Some(2))
    );
    let url_coarse = build_scoped_map_path(&GraphTargetScope::single("proj-1"), &coarse, false);
    assert!(url_coarse.contains("lod=crate"), "{url_coarse}");
    assert!(url_coarse.contains("min_weight=2"), "{url_coarse}");

    let mid = json!({"auto_lod": true, "scale": 0.50, "scope": "", "focus": ""});
    assert_eq!(
        surfaces::progressive_lod_for_state(&mid, None),
        (Some("folder".to_owned()), None)
    );
    let url_mid = build_scoped_map_path(&GraphTargetScope::single("proj-1"), &mid, false);
    assert!(url_mid.contains("lod=folder"), "{url_mid}");
    assert!(!url_mid.contains("min_weight="), "{url_mid}");

    let explicit = json!({"lod": "folder", "min_weight": 3, "scope": "src"});
    let url_explicit = build_scoped_map_path(&GraphTargetScope::single("proj-1"), &explicit, false);
    assert!(url_explicit.contains("lod=folder"), "{url_explicit}");
    assert!(url_explicit.contains("min_weight=3"), "{url_explicit}");

    let mut st = json!({});
    surfaces::apply_code_map_jump_context(
        &mut st,
        Some("billing"),
        Some("deadbeef1234"),
        Some("publishes_event"),
        Some("charge"),
    );
    assert_eq!(st["target"], json!("commit:deadbeef1234"));
    assert_eq!(st["semantic_kind"], json!("publishes_event"));
    assert_eq!(st["omnigraph_filter"], json!("billing"));
    assert!(st["filter"].is_null());

    let mut file_st = json!({});
    surfaces::apply_code_map_jump_context(
        &mut file_st,
        Some("invoice"),
        Some("commit:abc1234"),
        Some("di_binds"),
        None,
    );
    assert_eq!(file_st["target"], json!("commit:abc1234"));
    assert_eq!(file_st["semantic_kind"], json!("di_binds"));
    assert_eq!(file_st["filter"], json!("invoice"));

    let min_only = json!({"auto_lod": false, "min_weight": 5, "scope": ""});
    assert_eq!(
        surfaces::progressive_lod_for_state(&min_only, None),
        (None, Some(5))
    );
    let url_min = build_scoped_map_path(&GraphTargetScope::single("proj-1"), &min_only, false);
    assert!(!url_min.contains("lod="), "{url_min}");
    assert!(url_min.contains("min_weight=5"), "{url_min}");
}

#[test]
fn graph_status_sse_invalidates_analysis_cache_keys_on_watcher_and_ir_updated() {
    m::set(surfaces::MODAL_SURFACE.changes_key, json!("prev-changes"));
    m::set(surfaces::MODAL_SURFACE.impact_key, json!("prev-impact"));
    let epoch_before = m::next(surfaces::MODAL_SURFACE.changes_epoch);

    let act = crate::graph::apply_graph_status_sse_event(
        &json!({
            "project_id": "proj-1",
            "state": "ready",
            "reason": "ir_updated",
            "revision": 42
        }),
        "proj-1",
    );
    assert_eq!(
        act,
        crate::graph::GraphSseAction::RefreshCandidate {
            event_project: "proj-1".to_owned(),
            banner_text: "Graph updated — Refresh".to_owned(),
            schedule_idle: true,
        }
    );
    assert!(m::data(surfaces::MODAL_SURFACE.changes_key).is_null());
    assert!(m::data(surfaces::MODAL_SURFACE.impact_key).is_null());
    assert!(!m::current(
        surfaces::MODAL_SURFACE.changes_epoch,
        epoch_before
    ));
    assert_eq!(m::data("graph")["ir_revision"], json!(42));

    m::set(surfaces::MODAL_SURFACE.changes_key, json!("prev-changes-2"));
    m::set(surfaces::MODAL_SURFACE.impact_key, json!("prev-impact-2"));
    let act_stale = crate::graph::apply_graph_status_sse_event(
        &json!({
            "project_id": "proj-1",
            "state": "stale",
            "reason": "watcher"
        }),
        "proj-1",
    );
    assert!(matches!(
        act_stale,
        crate::graph::GraphSseAction::RefreshCandidate {
            schedule_idle: false,
            ..
        }
    ));
    assert!(m::data(surfaces::MODAL_SURFACE.changes_key).is_null());
    assert!(m::data(surfaces::MODAL_SURFACE.impact_key).is_null());
}

#[test]
fn progressive_lod_reads_zoom_and_live_modal_camera_scale_and_sse_updates_graph_counts() {
    // 1. State `zoom` key (set by camera_dom interactive zoom) feeds progressive_lod_for_state.
    let zoomed_out = json!({"auto_lod": true, "zoom": 0.28, "scope": "", "focus": ""});
    assert_eq!(
        surfaces::progressive_lod_for_state(&zoomed_out, None),
        (Some("crate".to_owned()), Some(2))
    );
    let url_zoom = build_scoped_map_path(&GraphTargetScope::single("proj-1"), &zoomed_out, false);
    assert!(url_zoom.contains("lod=crate"), "{url_zoom}");
    assert!(url_zoom.contains("min_weight=2"), "{url_zoom}");

    // 2. Saved non-fit modal camera scale overrides default 1.0 when building modal map URLs.
    m::set(
        surfaces::MODAL_SURFACE.camera_key,
        json!({ "scale": 0.48, "fit_mode": false }),
    );
    let plain_auto = json!({"auto_lod": true, "fit": false, "scope": "", "focus": ""});
    let url_cam = build_scoped_map_path(&GraphTargetScope::single("proj-1"), &plain_auto, false);
    m::take(surfaces::MODAL_SURFACE.camera_key);
    assert!(url_cam.contains("lod=folder"), "{url_cam}");

    // 3. Fit-mode camera should NOT lock a modal request into coarse LOD.
    m::set(
        surfaces::MODAL_SURFACE.camera_key,
        json!({ "scale": 0.22, "fit_mode": true }),
    );
    let fit_auto = json!({"auto_lod": true, "fit": true, "scope": "", "focus": ""});
    let url_fit = build_scoped_map_path(&GraphTargetScope::single("proj-1"), &fit_auto, false);
    m::take(surfaces::MODAL_SURFACE.camera_key);
    assert!(!url_fit.contains("lod=crate"), "{url_fit}");

    // 4. Incremental SSE graph_status updates cached graph node/edge counts and previous count metadata.
    m::set(
        "graph",
        json!({ "state": "ready", "graph": { "nodes": 84, "edges": 190 } }),
    );
    let inc_act = crate::graph::apply_graph_status_sse_event(
        &json!({
            "project_id": "proj-1",
            "state": "updated",
            "reason": "incremental_update",
            "revision": 99,
            "nodes": 84,
            "edges": 190,
            "changed_files": ["src/main.rs"]
        }),
        "proj-1",
    );
    assert_eq!(
        inc_act,
        crate::graph::GraphSseAction::RefreshCandidate {
            event_project: "proj-1".to_owned(),
            banner_text: "Graph updated — Refresh".to_owned(),
            schedule_idle: true,
        }
    );
    assert_eq!(m::data("graph")["ir_revision"], json!(99));
    assert_eq!(m::data("graph")["graph"]["nodes"], json!(84));
    assert_eq!(m::data("graph-sse-delta-meta")["prev_nodes"], json!(84));

    // 5. Modal toolbar Explain button tracks focus & dense hit state cleanly.
    assert_eq!(
        surfaces::modal_explain_button_state(None, &V::Null, false),
        (
            false,
            "Explain",
            "Select a node first (or right-click any node or link)",
        )
    );
    assert_eq!(
        surfaces::modal_explain_button_state(
            Some(&json!({"focus": {"path": "src/auth", "folder": true}})),
            &V::Null,
            false,
        )
        .1,
        "Explain folder"
    );
    assert_eq!(
        surfaces::modal_explain_button_state(
            Some(&json!({"focus": {"path": "src/auth.rs", "symbol": false}})),
            &V::Null,
            false,
        )
        .1,
        "Explain file"
    );
    assert_eq!(
        surfaces::modal_explain_button_state(
            Some(&json!({"focus": {"path": "src/auth.rs#L10:verify", "symbol": true}})),
            &V::Null,
            false,
        )
        .1,
        "Explain symbol"
    );
    assert!(surfaces::modal_explain_button_state(None, &V::Null, true).0);
}

#[test]
fn auto_lod_defaults_true_on_fresh_and_normalized_embedded_states() {
    let f = fresh("");
    assert_eq!(f["auto_lod"], json!(true));

    // Legacy state without `auto_lod` or explicit `lod`/`min_weight` enables `auto_lod`.
    let norm_default = normalize_map_state(json!({"scope": ""}));
    assert_eq!(norm_default["auto_lod"], json!(true));

    // Explicit `lod` or `min_weight` disables implicit `auto_lod`, while explicit `false` stays `false`.
    let norm_explicit = normalize_map_state(json!({"lod": "folder"}));
    assert_eq!(norm_explicit["auto_lod"], json!(false));
    let norm_off = normalize_map_state(json!({"auto_lod": false}));
    assert_eq!(norm_off["auto_lod"], json!(false));

    // Embedded surface_state uses fresh/normalize defaults, so zooming out activates auto_lod URL.
    m::take(surfaces::SESSION_SURFACE.state_model_key);
    m::set(
        surfaces::SESSION_SURFACE.camera_key,
        json!({ "scale": 0.25, "fit_mode": false }),
    );
    let emb_st = surfaces::surface_state(surfaces::SESSION_SURFACE);
    assert_eq!(emb_st["auto_lod"], json!(true));
    let emb_url = build_scoped_map_path(&GraphTargetScope::single("proj-emb"), &emb_st, true);
    m::take(surfaces::SESSION_SURFACE.camera_key);
    assert!(
        emb_url.contains("lod=crate") && emb_url.contains("min_weight=2"),
        "expected fresh embedded surface at 0.25 scale to enable auto_lod: {emb_url}"
    );
}
