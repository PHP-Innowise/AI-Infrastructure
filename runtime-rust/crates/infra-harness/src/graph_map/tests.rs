use super::*;
fn index(graph: Value) -> Index {
    Index::read(graph.to_string().as_bytes(), Path::new("/work/shop")).unwrap()
}
#[test]
fn cycle_returns_to_the_origin_by_the_shortest_available_directed_route() {
    let node = |id: &str, file: &str| json!({"id": id, "label": id, "source_file": file});
    let edge = |from: &str, to: &str, confidence: &str| json!({"source": from, "target": to, "relation": "calls", "confidence": confidence});
    let index = index(json!({
        "nodes": [
            node("a", "src/a.php"), node("b", "src/b.php"),
            node("c", "src/c.php"), node("d", "src/d.php"),
            node("entry", "entry.php"), node("first", "folder/first.php"),
            node("last", "folder/last.php"), node("bridge", "bridge.php"),
            node("x", "symbols.php"), node("y", "symbols.php"),
            node("p", "left/p.php"), node("q", "right/q.php"), node("r", "right/r.php")
        ],
        "links": [
            edge("a", "b", "EXTRACTED"), edge("b", "a", "INFERRED"),
            edge("a", "c", "EXTRACTED"), edge("c", "d", "EXTRACTED"),
            edge("d", "a", "EXTRACTED"), edge("entry", "a", "EXTRACTED"),
            edge("first", "bridge", "EXTRACTED"), edge("bridge", "last", "EXTRACTED"),
            edge("x", "y", "EXTRACTED"), edge("y", "x", "EXTRACTED"),
            edge("p", "q", "EXTRACTED"), edge("r", "p", "EXTRACTED")
        ]
    }));
    let cycle =
        |path: &str, certain| index.cycle("demo", path, &[], &|_| None, Some(path), certain);
    let guessed = cycle("src/a.php", false);
    assert_eq!(guessed["cycle"]["steps"], 2);
    assert_eq!(
        guessed["edges"],
        json!([
            {
                "from": "src/a.php",
                "to": "src/b.php",
                "links": 1,
                "guessed": 0,
                "kinds": [{"kind": "calls", "links": 1}]
            },
            {
                "from": "src/b.php",
                "to": "src/a.php",
                "links": 1,
                "guessed": 1,
                "kinds": [{"kind": "calls", "links": 1}]
            }
        ])
    );
    assert_eq!(guessed["focus"]["uses"][0]["path"], "src/b.php");
    assert_eq!(guessed["focus"]["used_by"][0]["guessed"], 1);
    let certain = cycle("src/a.php", true);
    assert_eq!(certain["cycle"]["steps"], 3);
    assert_eq!(certain["edges"][2]["from"], "src/d.php");
    assert_eq!(certain["edges"][2]["to"], "src/a.php");
    assert_eq!(cycle("src/b.php", true)["cycle"]["steps"], 0);
    assert_eq!(cycle("entry.php", false)["cycle"]["steps"], 0);
    assert_eq!(cycle("gone.php", true)["cycle"]["origin"]["missing"], true);
    let folder = cycle("folder", true);
    assert_eq!(folder["cycle"]["origin"]["what"], "folder");
    assert_eq!(folder["cycle"]["steps"], 2);
    assert_eq!(folder["edges"][0]["from"], "folder");
    assert_eq!(folder["edges"][1]["to"], "folder");
    let grouped = index.view("demo", "", None, true, "");
    assert!(
        grouped["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge["from"] == "left" && edge["to"] == "right")
    );
    assert!(
        grouped["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge["from"] == "right" && edge["to"] == "left")
    );
    assert_eq!(cycle("left", true)["cycle"]["steps"], 0);
    let symbols = index
        .symbols("demo", "symbols.php", &[], None, None, false, "")
        .unwrap();
    let first = symbols["nodes"][0]["path"].as_str().unwrap();
    let named = cycle(first, true);
    assert_eq!(named["cycle"]["names"], true);
    assert_eq!(named["cycle"]["steps"], 2);
    assert_eq!(named["edges"][1]["to"], first);
}

#[test]
fn a_pr_cycle_compares_renamed_and_moved_places_with_the_full_base_graph() {
    let graph = |head| {
        let path = if head { "src/new.php" } else { "src/old.php" };
        let line = if head { "L31" } else { "L3" };
        let node = |id, file, line| json!({"id": id, "label": format!("{id}()"), "source_file": file, "source_location": line});
        let edge = |from, to, confidence| json!({"source": from, "target": to, "relation": "calls", "confidence": confidence});
        let mut links = vec![
            edge("a", "b", "EXTRACTED"),
            edge("b", "c", "EXTRACTED"),
            edge("c", "a", "EXTRACTED"),
        ];
        if head {
            links.push(edge("b", "a", "INFERRED"));
        }
        index(json!({
            "nodes": [node("a", path, line), node("b", "src/b.php", "L5"), node("c", "src/c.php", "L7")],
            "links": links,
        }))
    };
    let head = graph(true);
    let base = graph(false);
    let renames = HashMap::from([("src/new.php", "src/old.php")]);
    for (place, earlier) in [
        ("src/new.php", "src/old.php"),
        ("src/new.php#L31:a()", "src/old.php#L3:a()"),
    ] {
        let mut view = head.cycle("project", place, &[], &|_| None, None, false);
        assert_eq!(view["cycle"]["steps"], 2, "{view}");
        assert_eq!(
            head.compare_cycle(&head, &base, &mut view, &renames, false),
            ("present", Some(earlier.to_owned()))
        );
        assert_eq!(view["edges"][0]["added"], false);
        assert_eq!(view["edges"][1]["added"], true);
        assert_eq!(view["edges"][1]["guessed"], 1);
        let mut confirmed = head.cycle("project", place, &[], &|_| None, None, true);
        assert_eq!(confirmed["cycle"]["steps"], 3);
        assert_eq!(
            head.compare_cycle(&head, &base, &mut confirmed, &renames, true),
            ("present", Some(earlier.to_owned()))
        );
        let historical = base.cycle("project", earlier, &[], &|_| None, Some(earlier), false);
        assert_eq!(historical["cycle"]["origin"]["path"], earlier);
        assert_eq!(historical["cycle"]["steps"], 3);
        assert!(
            confirmed["edges"]
                .as_array()
                .unwrap()
                .iter()
                .all(|edge| edge["added"] == false)
        );
    }
}

#[test]
fn a_missing_head_symbol_does_not_match_a_file_cycle_in_the_base() {
    let base = index(json!({
        "nodes": [
            {"id": "f", "label": "f()", "source_file": "a.php"},
            {"id": "a", "source_file": "a.php"}, {"id": "b", "source_file": "b.php"},
        ],
        "links": [
            {"source": "a", "target": "b", "relation": "calls"},
            {"source": "b", "target": "a", "relation": "calls"},
        ],
    }));
    let head = index(json!({"nodes": [{"id": "a", "source_file": "a.php"}], "links": []}));
    assert_eq!(
        base.cycle("project", "a.php", &[], &|_| None, None, false)["cycle"]["steps"],
        2
    );
    let mut view = head.cycle("project", "a.php#f()", &[], &|_| None, None, false);
    assert_eq!(
        head.compare_cycle(&head, &base, &mut view, &HashMap::new(), false),
        ("ambiguous", None),
    );
}

#[test]
fn a_cycle_too_long_to_draw_keeps_the_whole_ordered_chain() {
    let steps = FOLLOWED + 1;
    let nodes = (0..steps)
        .map(|n| json!({"id": n.to_string(), "source_file": format!("v{n}.php")}))
        .collect::<Vec<_>>();
    let links = (0..steps)
        .map(|n| {
            json!({
                "source": n.to_string(),
                "target": ((n + 1) % steps).to_string(),
                "relation": "calls",
                "confidence": if n == steps - 1 { "INFERRED" } else { "EXTRACTED" }
            })
        })
        .collect::<Vec<_>>();
    let index = index(json!({"nodes": nodes, "links": links}));
    let view = index.cycle("demo", "v0.php", &[], &|_| None, None, false);
    assert_eq!(view["cycle"]["steps"], steps);
    let places = view["cycle"]["places"].as_array().unwrap();
    assert_eq!(places.len(), steps + 1);
    for (at, place) in places.iter().enumerate() {
        assert_eq!(place["path"], format!("v{}.php", at % steps));
    }
    assert_eq!(places[steps]["guessed"], 1);
    assert_eq!(places[steps]["links"], 1);
    assert_eq!(view["nodes"], json!([]));
}
fn shop() -> Index {
    index(json!({
        "directed": false,
        "nodes": [
            {"id": "cart", "label": "Cart", "source_file": "src/Cart/Cart.php"},
            {"id": "cart_total", "label": ".total()", "source_file": "src/Cart/Cart.php"},
            {"id": "price", "label": "Price", "source_file": "src/Cart/Price.php"},
            {"id": "order", "label": "Order", "source_file": "src/Order/Order.php"},
            {"id": "mail", "label": "Mailer", "source_file": "/work/shop/src/Mail/Mailer.php"},
            {"id": "test", "label": "CartTest", "source_file": "tests\\CartTest.php"},
            {"id": "readme", "label": "README", "source_file": "README.md"},
            {"id": "idea", "label": "a concept without a file"},
            {"id": 7, "label": "a numeric id", "source_file": "src/Order/Order.php"}
        ],
        "links": [
            {"source": "cart", "target": "cart_total", "relation": "method", "confidence": "EXTRACTED", "source_file": "src/Cart/Cart.php"},
            {"source": "cart", "target": "price", "relation": "imports", "confidence": "EXTRACTED", "source_file": "src/Cart/Cart.php"},
            {"source": "cart_total", "target": "price", "relation": "calls", "confidence": "INFERRED", "source_file": "src/Cart/Cart.php"},
            {"source": "order", "target": "cart", "relation": "imports", "confidence": "EXTRACTED", "source_file": "src/Order/Order.php"},
            {"source": "mail", "target": "order", "relation": "calls", "confidence": "INFERRED", "source_file": "src/Order/Order.php"},
            {"source": "test", "target": "cart", "relation": "calls", "confidence": "INFERRED", "source_file": "tests/CartTest.php"},
            {"source": "test", "target": "price", "relation": "calls", "confidence": "EXTRACTED", "source_file": "tests/CartTest.php"},
            {"source": "readme", "target": "cart", "relation": "contains", "confidence": "EXTRACTED", "source_file": "README.md"},
            {"source": "order", "target": "price", "relation": "calls", "confidence": "INFERRED", "source_file": "src/Elsewhere.php"},
            {"source": "idea", "target": "cart", "relation": "references", "confidence": "EXTRACTED"},
            {"source": "order", "target": "missing", "relation": "calls", "confidence": "EXTRACTED"},
            {"source": 7, "target": "mail", "relation": "_python_import_marker"}
        ]
    }))
}
fn rows(view: &Value, key: &str) -> Vec<String> {
    view[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| match key {
            "edges" => format!(
                "{} -> {} {}/{}",
                row["from"].as_str().unwrap(),
                row["to"].as_str().unwrap(),
                row["links"],
                row["guessed"]
            ),
            _ => format!(
                "{} {} files={} out={} in={} within={}",
                row["path"].as_str().unwrap(),
                if row["folder"] == true {
                    "folder"
                } else {
                    "file"
                },
                row["files"],
                row["out"],
                row["in"],
                row["within"]
            ),
        })
        .collect()
}
#[test]
fn links_become_file_dependencies_with_a_direction_and_a_confidence() {
    let index = shop();
    // Six files hold symbols; the concept without a file is no file.
    assert_eq!(index.files.len(), 6);
    assert_eq!((index.nodes, index.links, index.unplaced), (9, 12, 1));
    let view = index.view("shop", "", None, false, "");
    assert_eq!(
        view["totals"],
        json!({"files": 6, "symbols": 9, "relations": 12, "links": 6, "read": 3, "guessed": 3, "unplaced": 1, "ruled_out": 0})
    );
    assert_eq!(
        rows(&view, "nodes"),
        [
            "src folder files=4 out=0 in=2 within=4",
            "tests folder files=1 out=2 in=0 within=0",
            "README.md file files=1 out=0 in=0 within=0",
        ]
    );
    assert_eq!(rows(&view, "edges"), ["tests -> src 2/1"]);
    assert_eq!(view["crumbs"], json!([{"name": "shop", "path": ""}]));
}
fn change(index: &Index, changed: &[(&str, char)], scope: &str, focus: Option<&str>) -> Value {
    let changed = changed
        .iter()
        .map(|(path, kind)| ((*path).to_owned(), *kind))
        .collect::<Vec<_>>();
    index.changes("shop", &changed, scope, focus, false, "")
}
fn places(view: &Value) -> Vec<String> {
    view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| {
            format!(
                "{} {} {}{} out={} in={}",
                node["path"].as_str().unwrap(),
                node["role"].as_str().unwrap(),
                node["name"].as_str().unwrap(),
                match node["status"].as_str() {
                    Some(status) => format!(" [{status}]"),
                    None if node["changed"].as_u64().unwrap() > 0 => {
                        format!(" [{} changed]", node["changed"])
                    }
                    None => String::new(),
                },
                node["out"],
                node["in"]
            )
        })
        .collect()
}
#[test]
fn a_change_is_drawn_with_what_it_uses_and_what_uses_it() {
    let index = shop();
    let changed = [
        ("src/Cart/Cart.php", 'M'),
        ("src/New.php", 'A'),
        ("src/Old.php", 'D'),
    ];
    let view = change(&index, &changed, "", None);
    assert_eq!(view["mode"], "changes");
    assert_eq!(
        places(&view),
        [
            "src/Cart/Cart.php changed Cart.php [M] out=2 in=2",
            "src/Cart/Price.php needed Price.php out=0 in=2",
            "src/Order/Order.php affected Order.php out=1 in=0",
            "tests/CartTest.php affected CartTest.php out=1 in=0",
        ]
    );
    assert_eq!(
        rows(&view, "edges"),
        [
            "src/Cart/Cart.php -> src/Cart/Price.php 2/1",
            "src/Order/Order.php -> src/Cart/Cart.php 1/0",
            "tests/CartTest.php -> src/Cart/Cart.php 1/1",
        ]
    );
    // One file is in the graph; the new one has no symbols there and
    // the deleted one is gone.
    assert_eq!(
        view["change"],
        json!({"files": 3, "graphed": 1, "here": 1, "removed": 1, "unknown": 1, "single": true, "hidden": 0, "affected": 2, "needed": 1})
    );
    assert_eq!(
        (view["children"].as_u64(), view["matching"].as_u64()),
        (Some(3), Some(3))
    );
    // Links the tool only guessed can be left out.
    let certain = index.changes(
        "shop",
        &[("src/Cart/Cart.php".to_owned(), 'M')],
        "",
        None,
        true,
        "",
    );
    assert_eq!(
        rows(&certain, "edges"),
        [
            "src/Cart/Cart.php -> src/Cart/Price.php 1/0",
            "src/Order/Order.php -> src/Cart/Cart.php 1/0",
        ]
    );
    // The filter chooses among the linked files; the change stays.
    let filtered = index.changes(
        "shop",
        &[("src/Cart/Cart.php".to_owned(), 'M')],
        "",
        None,
        false,
        "TEST",
    );
    assert_eq!(
        places(&filtered),
        [
            "src/Cart/Cart.php changed Cart.php [M] out=2 in=2",
            "tests/CartTest.php affected CartTest.php out=1 in=0",
        ]
    );
    assert_eq!(
        (filtered["children"].as_u64(), filtered["matching"].as_u64()),
        (Some(3), Some(1))
    );
    // A change the graph knows nothing of draws nothing.
    let unknown = change(&index, &[("docs/guide.md", 'A')], "", None);
    assert_eq!(unknown["nodes"], json!([]));
    assert_eq!(unknown["change"]["unknown"], 1);
}
#[test]
fn a_selected_place_of_a_change_lists_both_sides() {
    let index = shop();
    let changed = [("src/Cart/Cart.php", 'M'), ("src/Order/Order.php", 'M')];
    let view = change(&index, &changed, "", Some("src/Cart/Cart.php"));
    let focus = &view["focus"];
    let listed = |key: &str| {
        focus[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                format!(
                    "{} {}/{}{}{}",
                    row["path"].as_str().unwrap(),
                    row["links"],
                    row["guessed"],
                    if row["changed"] == true {
                        " changed"
                    } else {
                        ""
                    },
                    if row["inside"] == true {
                        ""
                    } else {
                        " not drawn"
                    }
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(listed("uses"), ["src/Cart/Price.php 2/1"]);
    assert_eq!(
        listed("used_by"),
        ["src/Order/Order.php 1/0 changed", "tests/CartTest.php 1/1"]
    );
    assert_eq!(
        (focus["role"].as_str(), focus["status"].as_str()),
        (Some("changed"), Some("M"))
    );
    assert_eq!(
        focus["kinds"],
        json!([{"kind": "calls", "links": 1}, {"kind": "imports", "links": 1}])
    );
    // A file beside the change lists its links with the change only:
    // the mailer is used by Order.php and by nothing else that changed.
    let beside = change(&index, &changed, "", Some("src/Mail/Mailer.php"));
    assert_eq!(beside["focus"]["role"], "needed");
    assert_eq!(beside["focus"]["uses"], json!([]));
    assert_eq!(beside["focus"]["used_by"][0]["path"], "src/Order/Order.php");
    // A place that is not drawn is not selected.
    assert!(change(&index, &changed, "", Some("README.md"))["focus"].is_null());
}
#[test]
fn a_wide_change_is_drawn_by_folder_and_a_folder_opens_to_its_files() {
    let mut nodes = vec![
        json!({"id": "core", "source_file": "lib/core.php"}),
        json!({"id": "main", "source_file": "app/main.php"}),
    ];
    let mut links = vec![
        json!({"source": "main", "target": "a0", "relation": "calls", "source_file": "app/main.php"}),
    ];
    let mut changed = Vec::new();
    for folder in ["a", "b"] {
        for number in 0..15 {
            let (id, file) = (
                format!("{folder}{number}"),
                format!("{folder}/f{number}.php"),
            );
            nodes.push(json!({"id": id, "source_file": file}));
            links.push(
                json!({"source": id, "target": "core", "relation": "calls", "source_file": file}),
            );
            changed.push((file, 'M'));
        }
    }
    let index = index(json!({"nodes": nodes, "links": links}));
    let wide = index.changes("shop", &changed, "", None, false, "");
    assert_eq!(
        places(&wide),
        [
            "a changed a [15 changed] out=15 in=1",
            "b changed b [15 changed] out=15 in=0",
            "lib/core.php needed core.php out=0 in=30",
            "app/main.php affected main.php out=1 in=0",
        ]
    );
    assert_eq!(
        rows(&wide, "edges"),
        [
            "a -> lib/core.php 15/0",
            "b -> lib/core.php 15/0",
            "app/main.php -> a 1/0"
        ]
    );
    assert_eq!(wide["change"]["single"], false);
    assert_eq!(wide["nodes"][0]["folder"], true);
    // The opened folder holds few enough files to draw each; what
    // changed outside it no longer counts as the change here.
    let opened = index.changes("shop", &changed, "a", None, false, "");
    assert_eq!(opened["scope"], "a");
    assert_eq!(opened["change"]["here"], 15);
    assert_eq!(opened["change"]["graphed"], 30);
    assert_eq!(opened["nodes"].as_array().unwrap().len(), 17);
    assert_eq!(opened["nodes"][0]["folder"], false);
    assert_eq!(
        opened["crumbs"],
        json!([{"name": "shop", "path": ""}, {"name": "a", "path": "a"}])
    );
    // A folder without changes opens the whole change instead.
    assert_eq!(
        index.changes("shop", &changed, "lib", None, false, "")["scope"],
        ""
    );
}
#[test]
fn a_folder_whose_changes_lie_in_one_folder_below_is_passed_through() {
    let mut nodes = vec![json!({"id": "core", "source_file": "lib/core.php"})];
    let mut links = Vec::new();
    let mut changed = Vec::new();
    for number in 0..30 {
        let (id, file) = (
            format!("f{number}"),
            format!("app/src/Billing/f{number}.php"),
        );
        nodes.push(json!({"id": id, "source_file": file}));
        links.push(
            json!({"source": id, "target": "core", "relation": "calls", "source_file": file}),
        );
        changed.push((file, 'M'));
    }
    let index = index(json!({"nodes": nodes, "links": links}));
    // Thirty files are too many to draw one by one from the top, and
    // `app` and `app/src` each hold nothing but the next folder.
    let view = index.changes("shop", &changed, "", None, false, "");
    assert_eq!(view["scope"], "app/src/Billing");
    assert_eq!(view["change"]["here"], 30);
    assert_eq!(view["nodes"].as_array().unwrap().len(), 31);
    assert_eq!(view["nodes"][0]["folder"], false);
    assert_eq!(view["crumbs"].as_array().unwrap().len(), 4);
    // One changed file in a folder is still a folder with a count.
    changed.push(("lib/core.php".to_owned(), 'M'));
    let top = index.changes("shop", &changed, "", None, false, "");
    assert_eq!(top["scope"], "");
    assert_eq!(
        places(&top),
        [
            "app changed app [30 changed] out=30 in=0",
            "lib changed lib [1 changed] out=0 in=30"
        ]
    );
}
#[test]
fn files_that_share_a_name_are_told_apart_by_their_folder() {
    let all = [
        "src/Cart/Cart.php",
        "src/Order/Cart.php",
        "src/Order/Order.php",
        "README.md",
        "docs/README.md",
    ];
    assert_eq!(label(all[0], &all), "Cart/Cart.php");
    assert_eq!(label(all[1], &all), "Order/Cart.php");
    assert_eq!(label(all[2], &all), "Order.php");
    // A file at the top has nothing to add to its name.
    assert_eq!(label(all[3], &all), "README.md");
    assert_eq!(label(all[4], &all), "docs/README.md");
}
#[test]
fn a_link_points_from_the_file_it_was_found_in() {
    // The undirected graph stored Mailer -> Order; the call sits in Order.php.
    let view = shop().view("shop", "src", None, false, "");
    assert_eq!(
        rows(&view, "edges"),
        ["src/Order -> src/Cart 1/0", "src/Order -> src/Mail 1/1"]
    );
    assert_eq!(
        rows(&view, "nodes"),
        [
            "src/Cart folder files=2 out=0 in=3 within=2",
            "src/Order folder files=1 out=2 in=0 within=0",
            "src/Mail folder files=1 out=0 in=1 within=0",
        ]
    );
    assert_eq!(
        view["crumbs"],
        json!([{"name": "shop", "path": ""}, {"name": "src", "path": "src"}])
    );
}
#[test]
fn a_focus_lists_what_it_uses_and_what_uses_it_inside_and_outside_the_folder() {
    let view = shop().view("shop", "src", Some("src/Cart"), false, "");
    let focus = &view["focus"];
    assert_eq!(focus["name"], "Cart");
    assert_eq!(
        (focus["files"].as_u64(), focus["within"].as_u64()),
        (Some(2), Some(2))
    );
    assert_eq!(focus["uses"], json!([]));
    assert_eq!(
        focus["used_by"],
        json!([
            {"path": "tests", "name": "tests", "folder": true, "inside": false, "links": 2, "guessed": 1},
            {"path": "src/Order", "name": "Order", "folder": true, "inside": true, "links": 1, "guessed": 0},
        ])
    );
    let order = shop().view("shop", "src", Some("src/Order"), false, "");
    assert_eq!(
        order["focus"]["kinds"],
        json!([{"kind": "calls", "links": 1}, {"kind": "imports", "links": 1}])
    );
    // A place outside the folder can be focused too; an unknown one cannot.
    let tests = shop().view("shop", "src", Some("tests"), false, "");
    assert_eq!(tests["focus"]["inside"], false);
    assert_eq!(tests["focus"]["uses"][0]["path"], "src/Cart");
    assert_eq!(
        shop().view("shop", "src", Some("nowhere"), false, "")["focus"],
        Value::Null
    );
}
#[test]
fn certain_leaves_out_what_the_tool_inferred() {
    let view = shop().view("shop", "src", Some("src/Order"), true, "");
    assert_eq!(rows(&view, "edges"), ["src/Order -> src/Cart 1/0"]);
    assert_eq!(view["focus"]["uses"].as_array().unwrap().len(), 1);
    assert_eq!(view["totals"]["guessed"], 3);
    assert_eq!(view["certain"], true);
}
#[test]
fn a_filter_and_the_limit_choose_which_children_are_drawn() {
    let view = shop().view("shop", "src", None, false, "ma");
    assert_eq!(
        rows(&view, "nodes"),
        ["src/Mail folder files=1 out=0 in=1 within=0"]
    );
    assert_eq!(
        (view["children"].as_u64(), view["matching"].as_u64()),
        (Some(3), Some(1))
    );
    let files = (0..SHOWN + 5)
        .map(|at| json!({"id": format!("n{at}"), "source_file": format!("lib/f{at:02}.php")}))
        .collect::<Vec<_>>();
    let links = (1..SHOWN + 5)
            .map(|at| json!({"source": format!("n{at}"), "target": "n0", "relation": "imports", "source_file": format!("lib/f{at:02}.php")}))
            .collect::<Vec<_>>();
    let wide = index(json!({"nodes": files, "edges": links}));
    let view = wide.view("wide", "lib", None, false, "");
    assert_eq!(view["nodes"].as_array().unwrap().len(), SHOWN);
    assert_eq!(view["children"].as_u64(), Some(SHOWN as u64 + 5));
    // The most used file comes first, and a link without a confidence was read.
    assert_eq!(view["nodes"][0]["path"], "lib/f00.php");
    assert_eq!(view["totals"]["read"].as_u64(), Some(SHOWN as u64 + 4));
    assert_eq!(view["between"].as_u64(), Some(SHOWN as u64 - 1));
}
#[test]
fn a_dense_folder_keeps_the_strongest_uses_of_each_child() {
    // Six files that all use each other; file N uses file M through N * 10 + M links.
    let nodes = (0..6)
        .map(|at| json!({"id": format!("n{at}"), "source_file": format!("f{at}.py")}))
        .collect::<Vec<_>>();
    let mut links = vec![];
    for from in 0..6 {
        for to in (0..6).filter(|to| *to != from) {
            for _ in 0..from * 10 + to + 1 {
                links.push(json!({"source": format!("n{from}"), "target": format!("n{to}"), "relation": "calls", "source_file": format!("f{from}.py")}));
            }
        }
    }
    let dense = index(json!({"nodes": nodes, "links": links}));
    let view = dense.view("dense", "", Some("f0.py"), false, "");
    assert_eq!(
        (view["between"].as_u64(), view["backbone"].as_bool()),
        (Some(30), Some(true))
    );
    let drawn = rows(&view, "edges");
    assert_eq!(drawn.len(), 6 * STRONGEST);
    // File 0 keeps its links to 5, 4 and 3, the three it uses most.
    assert_eq!(
        drawn
            .iter()
            .filter(|row| row.starts_with("f0.py ->"))
            .collect::<Vec<_>>(),
        [
            "f0.py -> f5.py 6/0",
            "f0.py -> f4.py 5/0",
            "f0.py -> f3.py 4/0"
        ]
    );
    // The focus still lists everything.
    assert_eq!(view["focus"]["uses"].as_array().unwrap().len(), 5);
    // A sparse folder is drawn in full.
    assert_eq!(
        shop().view("shop", "src", None, false, "")["backbone"],
        false
    );
}
#[test]
fn an_unknown_folder_opens_the_project_and_a_wrong_file_is_refused() {
    let view = shop().view("shop", "src/Nowhere", None, false, "");
    assert_eq!(view["scope"], "");
    assert_eq!(
        shop().view("shop", "/src/", None, false, "")["scope"],
        "src"
    );
    assert!(Index::read("[1, 2]".as_bytes(), Path::new("/")).is_err());
    assert!(Index::read("{\"nodes\": 5}".as_bytes(), Path::new("/")).is_err());
    // Links before nodes are still counted.
    let early = index(json!({
        "links": [{"source": "a", "target": "b", "relation": "imports", "source_file": "a.py"}],
        "nodes": [{"id": "a", "source_file": "a.py"}, {"id": "b", "source_file": "b.py"}],
    }));
    assert_eq!(
        rows(&early.view("p", "", None, false, ""), "edges"),
        ["a.py -> b.py 1/0"]
    );
}
#[test]
fn a_place_stands_for_a_file_inside_and_outside_the_open_folder() {
    assert_eq!(group("", "src/Cart/Cart.php"), ("src", true));
    assert_eq!(group("", "README.md"), ("README.md", true));
    assert_eq!(group("src", "src/Cart/Cart.php"), ("src/Cart", true));
    assert_eq!(
        group("src/Cart", "src/Cart/Cart.php"),
        ("src/Cart/Cart.php", true)
    );
    assert_eq!(
        group("src/Cart", "src/Order/Order.php"),
        ("src/Order", false)
    );
    assert_eq!(group("src/Cart", "tests/CartTest.php"), ("tests", false));
    assert_eq!(
        group("src/Cart", "src/Cartoon.php"),
        ("src/Cartoon.php", false)
    );
    assert_eq!(group("src/Cart", "README.md"), ("README.md", false));
}
/// A small shop as the tool writes it: a file node that holds classes
/// and functions, classes that hold methods, an enum with its cases.
/// An application over a library. `main` uses two handlers and both use
/// `core`; `core` uses `db` through two links and a cache through one;
/// one handler is only guessed to use `db`; `count` reports use `db`.
/// The places of a view, each with the frame that holds it.
fn held(view: &Value) -> Vec<String> {
    let mut places = view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| {
            format!(
                "{}{}",
                node["path"].as_str().unwrap(),
                node["holder"]
                    .as_str()
                    .map(|holder| format!(" in {holder}"))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>();
    places.sort();
    places
}
/// The frames of a view: the folder, the frame that holds it, and how
/// many of its places are drawn.
fn framed(view: &Value) -> Vec<String> {
    view["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|frame| {
            format!(
                "{}{} {}/{}",
                frame["path"].as_str().unwrap(),
                frame["holder"]
                    .as_str()
                    .map(|holder| format!(" in {holder}"))
                    .unwrap_or_default(),
                frame["drawn"],
                frame["places"]
            )
        })
        .collect()
}
fn sorted(mut rows: Vec<String>) -> Vec<String> {
    rows.sort();
    rows
}
#[test]
fn a_folder_is_spread_out_among_what_lies_beside_it() {
    let index = shop();
    let spread = |scope: &str, folders: &[&str], focus: Option<&str>, filter: &str| {
        let folders = folders
            .iter()
            .map(|folder| (*folder).to_owned())
            .collect::<Vec<_>>();
        index.view_with("shop", scope, &folders, focus, false, filter)
    };
    // As it is drawn without: the folder is one place.
    let plain = index.view("shop", "", None, false, "");
    assert_eq!(held(&plain), ["README.md", "src", "tests"]);
    assert_eq!(framed(&plain), [""; 0]);
    // Spread out, it gives way to what it holds, and the links are those
    // of what it holds.
    let view = spread("", &["src"], None, "");
    assert_eq!(
        held(&view),
        [
            "README.md",
            "src/Cart in src",
            "src/Mail in src",
            "src/Order in src",
            "tests"
        ]
    );
    assert_eq!(framed(&view), ["src 3/3"]);
    assert_eq!(
        sorted(rows(&view, "edges")),
        [
            "src/Order -> src/Cart 1/0",
            "src/Order -> src/Mail 1/1",
            "tests -> src/Cart 2/1"
        ]
    );
    assert_eq!((&view["scope"], &view["children"]), (&json!(""), &json!(5)));
    // A folder inside it is spread out in turn, down to its files.
    let deep = spread("", &["src/Cart", "src"], None, "");
    assert_eq!(
        held(&deep),
        [
            "README.md",
            "src/Cart/Cart.php in src/Cart",
            "src/Cart/Price.php in src/Cart",
            "src/Mail in src",
            "src/Order in src",
            "tests"
        ]
    );
    assert_eq!(framed(&deep), ["src 2/2", "src/Cart in src 2/2"]);
    assert_eq!(
        sorted(rows(&deep, "edges")),
        [
            "src/Cart/Cart.php -> src/Cart/Price.php 2/1",
            "src/Order -> src/Cart/Cart.php 1/0",
            "src/Order -> src/Mail 1/1",
            "tests -> src/Cart/Cart.php 1/1",
            "tests -> src/Cart/Price.php 1/0"
        ]
    );
    // What is named and not reached is passed over: a folder inside one
    // that is not spread out, a file, a name the graph does not know.
    let unreached = spread("", &["src/Cart", "README.md", "nowhere"], None, "");
    assert_eq!(unreached["nodes"], plain["nodes"]);
    assert_eq!(framed(&unreached), [""; 0]);
    // In an open folder the frame of its own folder has no holder.
    let inside = spread("src", &["src/Cart/"], None, "");
    assert_eq!(
        held(&inside),
        [
            "src/Cart/Cart.php in src/Cart",
            "src/Cart/Price.php in src/Cart",
            "src/Mail",
            "src/Order"
        ]
    );
    assert_eq!(framed(&inside), ["src/Cart 2/2"]);
    // A filter keeps the places of that name, wherever they lie, with
    // the frames that hold them.
    let filtered = spread("", &["src", "src/Cart"], None, "price");
    assert_eq!(held(&filtered), ["src/Cart/Price.php in src/Cart"]);
    assert_eq!(framed(&filtered), ["src 0/2", "src/Cart in src 1/2"]);
    assert_eq!(filtered["matching"], 1);
    // A place in a frame is selected as any other.
    let chosen = spread("", &["src", "src/Cart"], Some("src/Cart/Cart.php"), "");
    assert_eq!(chosen["focus"]["name"], "Cart.php");
    let side = |key: &str| {
        chosen["focus"][key]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["path"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(side("uses"), ["src/Cart/Price.php"]);
    assert_eq!(sorted(side("used_by")), ["src/Order", "tests"]);
    // A folder that is spread out is no place to select.
    assert_eq!(spread("", &["src"], Some("src"), "")["focus"], Value::Null);
}
/// A project of `folders` folders of `files` files each, every file of
/// which uses one hub, as does one more file beside them.
fn wheel(folders: usize, files: usize) -> Index {
    let mut nodes = vec![
        json!({"id": "hub", "label": "hub()", "source_file": "hub.rs"}),
        json!({"id": "app", "label": "app()", "source_file": "app.rs"}),
    ];
    let mut links = vec![
        json!({"source": "app", "target": "hub", "relation": "calls", "confidence": "EXTRACTED", "source_file": "app.rs"}),
    ];
    for folder in 0..folders {
        for file in 0..files {
            let (id, path) = (
                format!("f{folder}_{file}"),
                format!("p{folder}/f{file:02}.rs"),
            );
            nodes.push(json!({"id": id, "label": "spoke()", "source_file": path}));
            links.push(json!({"source": id, "target": "hub", "relation": "calls", "confidence": "EXTRACTED", "source_file": path}));
        }
    }
    index(json!({"directed": false, "nodes": nodes, "links": links}))
}
#[test]
fn a_spread_out_folder_shows_its_most_linked_and_a_full_picture_keeps_the_open_folder() {
    let index = wheel(1, HELD + 5);
    let view = index.view_with("wheel", "", &["p0".to_owned()], None, false, "");
    // The frame holds what it can; the open folder keeps its own.
    assert_eq!(framed(&view), [format!("p0 {HELD}/{}", HELD + 5)]);
    let places = held(&view);
    assert_eq!(places.len(), HELD + 2);
    assert!(places.contains(&"app.rs".to_owned()) && places.contains(&"hub.rs".to_owned()));
    assert!(places.contains(&"p0/f00.rs in p0".to_owned()));
    assert!(!places.contains(&format!("p0/f{:02}.rs in p0", HELD)));
    assert_eq!(
        (&view["children"], &view["matching"]),
        (&json!(HELD + 7), &json!(HELD + 7))
    );
    // More is spread out than a picture holds: the frames fill their
    // share, and what the open folder itself holds is still there.
    let folders = SPREAD / HELD;
    let index = wheel(folders, HELD);
    let all = (0..folders)
        .map(|folder| format!("p{folder}"))
        .collect::<Vec<_>>();
    let view = index.view_with("wheel", "", &all, None, false, "");
    let places = held(&view);
    assert_eq!(places.len(), SPREAD - OWN + 2);
    assert!(places.contains(&"app.rs".to_owned()) && places.contains(&"hub.rs".to_owned()));
    assert_eq!(framed(&view).len(), (SPREAD - OWN) / HELD);
}
/// One end of a looked-into link: each file with the names drawn in it.
fn end(view: &Value, side: &str) -> Vec<String> {
    view[side]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|group| {
            format!(
                "{}: {}",
                group["name"].as_str().unwrap(),
                group["names"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|name| format!(
                        "{} {}x{}",
                        name["kind"].as_str().unwrap(),
                        name["name"].as_str().unwrap(),
                        name["links"]
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect()
}
/// The links of a looked-into link, by the names of their two ends.
fn wires(view: &Value) -> Vec<String> {
    let named = |key: &Value| {
        ["users", "used"]
            .iter()
            .flat_map(|side| view[*side]["groups"].as_array().unwrap())
            .flat_map(|group| group["names"].as_array().unwrap())
            .find(|name| name["key"] == *key)
            .map(|name| name["name"].as_str().unwrap().to_owned())
            .unwrap()
    };
    view["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| {
            format!(
                "{} -> {} {}{}",
                named(&edge["from"]),
                named(&edge["to"]),
                edge["kind"].as_str().unwrap(),
                if edge["guessed"] == true {
                    " guessed"
                } else {
                    ""
                }
            )
        })
        .collect()
}
#[test]
fn a_link_is_looked_into_down_to_the_names_at_its_two_ends() {
    let index = shop();
    // The arrow of the project's picture: the tests use the source.
    let view = index.link("tests", "src", false);
    assert_eq!(
        (&view["mode"], &view["links"], &view["guessed"]),
        (&json!("link"), &json!(2), &json!(1))
    );
    assert_eq!(
        view["from"],
        json!({"path": "tests", "name": "tests", "what": "folder"})
    );
    assert_eq!(
        view["to"],
        json!({"path": "src", "name": "src", "what": "folder"})
    );
    // What has a method is a type; the graph says no more of the rest.
    assert_eq!(end(&view, "users"), ["CartTest.php: name CartTestx2"]);
    assert_eq!(
        end(&view, "used"),
        ["Cart.php: type Cartx1", "Price.php: name Pricex1"]
    );
    assert_eq!(
        wires(&view),
        ["CartTest -> Cart calls guessed", "CartTest -> Price calls"]
    );
    assert_eq!(
        (&view["users"]["names"], &view["users"]["files"]),
        (&json!(1), &json!(1))
    );
    assert_eq!(
        (&view["used"]["names"], &view["used"]["files"]),
        (&json!(2), &json!(2))
    );
    assert_eq!(view["kinds"], json!([{"kind": "calls", "links": 2}]));
    // Only what the tool read from the code.
    let certain = index.link("tests", "src", true);
    assert_eq!(
        (&certain["links"], &certain["guessed"]),
        (&json!(1), &json!(0))
    );
    assert_eq!(wires(&certain), ["CartTest -> Price calls"]);
    // Between two files the names are their symbols: a method under the
    // name of its type.
    let files = index.link("src/Cart/Cart.php", "src/Cart/Price.php", false);
    assert_eq!(files["from"]["what"], "file");
    assert_eq!(
        end(&files, "users"),
        ["Cart.php: type Cartx1, method Cart.total()x1"]
    );
    assert_eq!(end(&files, "used"), ["Price.php: name Pricex2"]);
    assert_eq!(
        wires(&files),
        [
            "Cart -> Price imports",
            "Cart.total() -> Price calls guessed"
        ]
    );
    // A symbol of an opened file is an end by the name that file's
    // picture gives it.
    let key = files["users"]["groups"][0]["names"][1]["key"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(key, "src/Cart/Cart.php#.total()");
    let symbol = index.link(&key, "src/Cart/Price.php", false);
    assert_eq!(
        symbol["from"],
        json!({"path": key, "name": "Cart.total()", "what": "symbol", "file": "src/Cart/Cart.php"})
    );
    assert_eq!(wires(&symbol), ["Cart.total() -> Price calls guessed"]);
    // The other way round nothing links the two, and that is an answer.
    let back = index.link("src", "tests", false);
    assert_eq!(
        (&back["links"], &back["missing"]),
        (&json!(0), &Value::Null)
    );
    assert_eq!((end(&back, "users"), wires(&back)), (vec![], vec![]));
    // An end the graph does not know makes the link missing: the
    // project itself, a path that is not there, a symbol that is not.
    for (from, to) in [
        ("", "src"),
        ("src", "nowhere"),
        ("src/Cart/Cart.php#.gone()", "src"),
    ] {
        let missing = index.link(from, to, false);
        assert_eq!(missing["missing"], true, "{from} -> {to}");
        assert_eq!(missing["mode"], "link");
    }
}
#[test]
fn a_wide_link_shows_the_most_linked_names_of_each_end_and_counts_them_all() {
    // Every file of one folder uses the hub; the hub is one name.
    let index = wheel(1, LINKED + 7);
    let view = index.link("p0", "hub.rs", false);
    assert_eq!(view["links"], json!(LINKED + 7));
    assert_eq!(
        (&view["users"]["names"], &view["users"]["files"]),
        (&json!(LINKED + 7), &json!(LINKED + 7))
    );
    let drawn = view["users"]["groups"].as_array().unwrap().len();
    assert_eq!(drawn, LINKED);
    // A link is drawn where both of its ends are.
    assert_eq!(view["edges"].as_array().unwrap().len(), LINKED);
    assert_eq!(
        end(&view, "used"),
        [format!("hub.rs: function hub()x{}", LINKED + 7)]
    );
    // Files that share a name are told apart by their folders.
    let index = wheel(2, 1);
    let view = index.link("p0", "hub.rs", false);
    assert_eq!(end(&view, "users"), ["f00.rs: function spoke()x1"]);
    let both = index.link("p1/f00.rs", "hub.rs", false);
    assert_eq!(end(&both, "users"), ["f00.rs: function spoke()x1"]);
}
fn office(count: usize) -> Index {
    let call = |from: &str, to: &str, file: &str, read: bool| {
        json!({
            "source": from, "target": to, "relation": "calls",
            "confidence": if read { "EXTRACTED" } else { "INFERRED" },
            "source_file": file
        })
    };
    let mut nodes = vec![
        json!({"id": "main", "label": "main", "source_file": "app/main.php"}),
        json!({"id": "users", "label": "users", "source_file": "app/api/users.php"}),
        json!({"id": "orders", "label": "orders", "source_file": "app/api/orders.php"}),
        json!({"id": "core", "label": "core", "source_file": "lib/core.php"}),
        json!({"id": "boot", "label": "boot", "source_file": "lib/core.php"}),
        json!({"id": "db", "label": "db", "source_file": "lib/db.php"}),
        json!({"id": "store", "label": "store", "source_file": "lib/deep/cache/store.php"}),
    ];
    let mut links = vec![
        call("main", "users", "app/main.php", true),
        call("main", "orders", "app/main.php", true),
        call("users", "core", "app/api/users.php", true),
        call("orders", "core", "app/api/orders.php", true),
        call("orders", "db", "app/api/orders.php", false),
        call("core", "db", "lib/core.php", true),
        call("boot", "db", "lib/core.php", true),
        call("core", "store", "lib/core.php", true),
    ];
    for at in 1..=count {
        let (id, file) = (format!("r{at:02}"), format!("reports/r{at:02}.php"));
        nodes.push(json!({"id": id, "label": id, "source_file": file}));
        links.push(call(&id, "db", &file, true));
    }
    index(json!({"directed": false, "nodes": nodes, "links": links}))
}
/// Each box of followed links: its name, how many of what it uses and of
/// what uses it are not drawn, and which of its sides are unfolded.
fn steps(view: &Value) -> Vec<String> {
    view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| {
            format!(
                "{}{} +{}/{}{}{}",
                node["name"].as_str().unwrap(),
                if node["folder"] == true { "/" } else { "" },
                node["more_out"],
                node["more_in"],
                if node["unfolded_out"] == true {
                    " out"
                } else {
                    ""
                },
                if node["unfolded_in"] == true {
                    " in"
                } else {
                    ""
                },
            )
        })
        .collect()
}
fn named(places: &[&str]) -> Vec<String> {
    places.iter().map(|place| (*place).to_owned()).collect()
}
#[test]
fn the_links_of_a_file_are_followed_step_by_step() {
    let index = office(14);
    let follow = |out: &[&str], into: &[&str]| {
        index
            .follow(
                "office",
                "lib/core.php",
                &named(out),
                &named(into),
                &[],
                None,
                false,
            )
            .unwrap()
    };
    // The file alone at first, with how many it uses and how many use it.
    let alone = follow(&[], &[]);
    assert_eq!(
        (&alone["mode"], &alone["scope"]),
        (&json!("follow"), &json!("lib"))
    );
    assert_eq!(steps(&alone), ["core.php +2/2"]);
    assert_eq!(alone["nodes"][0]["start"], true);
    assert_eq!(rows(&alone, "edges"), Vec::<String>::new());
    assert_eq!(
        alone["followed"],
        json!({
            "path": "lib/core.php", "name": "core.php", "folder": false, "uses": 2, "users": 2,
            "shown": 1, "limit": FOLLOWED, "step": STEP, "full": false, "aside": 0
        })
    );
    assert_eq!(
        alone["crumbs"],
        json!([
            {"name": "office", "path": ""},
            {"name": "lib", "path": "lib"},
            {"name": "core.php", "path": "lib/core.php"}
        ])
    );
    // What it uses, strongest first: db through two links, then the cache.
    let uses = follow(&["lib/core.php"], &[]);
    assert_eq!(
        steps(&uses),
        ["core.php +0/2 out", "db.php +0/15", "store.php +0/0"]
    );
    assert_eq!(
        rows(&uses, "edges"),
        [
            "lib/core.php -> lib/db.php 2/0",
            "lib/core.php -> lib/deep/cache/store.php 1/0"
        ]
    );
    // Both ways. orders is guessed to use db and both are drawn, but
    // neither that side of orders nor that side of db is unfolded: the
    // link waits, and each of the two counts the other as not shown.
    let both = follow(&["lib/core.php"], &["lib/core.php"]);
    assert_eq!(
        steps(&both),
        [
            "core.php +0/0 out in",
            "db.php +0/15",
            "store.php +0/0",
            "orders.php +1/1",
            "users.php +0/1"
        ]
    );
    assert_eq!(
        rows(&both, "edges"),
        [
            "lib/core.php -> lib/db.php 2/0",
            "app/api/orders.php -> lib/core.php 1/0",
            "app/api/users.php -> lib/core.php 1/0",
            "lib/core.php -> lib/deep/cache/store.php 1/0"
        ]
    );
    assert_eq!(both["followed"]["aside"], 1);
    // Unfolding what orders uses draws it, and adds no place.
    let drawn = follow(&["lib/core.php", "app/api/orders.php"], &["lib/core.php"]);
    assert_eq!(drawn["nodes"].as_array().unwrap().len(), 5);
    assert!(rows(&drawn, "edges").contains(&"app/api/orders.php -> lib/db.php 1/1".to_owned()));
    assert_eq!(steps(&drawn)[3], "orders.php +0/1 out");
    assert_eq!(drawn["followed"]["aside"], 0);
    // A step is twelve places that are not drawn yet: of the fourteen
    // reports that use db, twelve come first and the rest with the next.
    let step = follow(&["lib/core.php"], &["lib/core.php", "lib/db.php"]);
    assert_eq!(step["nodes"].as_array().unwrap().len(), 5 + STEP);
    assert_eq!(steps(&step)[1], "db.php +0/2 in");
    assert_eq!(step["followed"]["shown"], 17);
    let whole = follow(
        &["lib/core.php"],
        &["lib/core.php", "lib/db.php", "lib/db.php"],
    );
    assert_eq!(whole["nodes"].as_array().unwrap().len(), 19);
    assert_eq!(steps(&whole)[1], "db.php +0/0 in");
    // Folding a side takes away what only it brought: the handlers go,
    // and orders comes back as one of those that use db, its link to
    // core waiting again.
    let folded = follow(&["lib/core.php"], &["lib/db.php", "lib/db.php"]);
    assert_eq!(steps(&folded)[0], "core.php +0/2 out");
    assert_eq!(steps(&folded)[3], "orders.php +1/1");
    assert_eq!(folded["nodes"].as_array().unwrap().len(), 3 + 15);
    // What is unfolded behind a folded side is not reached.
    let unreached = follow(&[], &["lib/db.php"]);
    assert_eq!(steps(&unreached), ["core.php +2/2"]);
}
#[test]
fn followed_links_leave_out_guesses_and_stop_at_a_full_picture() {
    let index = office(14);
    let certain = index
        .follow(
            "office",
            "lib/core.php",
            &named(&["lib/core.php"]),
            &named(&["lib/core.php"]),
            &[],
            None,
            true,
        )
        .unwrap();
    assert!(
        !rows(&certain, "edges")
            .iter()
            .any(|edge| edge.starts_with("app/api/orders.php -> lib/db.php"))
    );
    assert_eq!(rows(&certain, "edges").len(), 4);
    // Seventy reports use db; however often its side is unfolded, the
    // picture stops at its limit and says so.
    let crowd = office(70);
    let mut into = named(&["lib/core.php"]);
    into.extend(named(&["lib/db.php"; 8]));
    let full = crowd
        .follow(
            "office",
            "lib/core.php",
            &named(&["lib/core.php"]),
            &into,
            &[],
            None,
            false,
        )
        .unwrap();
    assert_eq!(full["nodes"].as_array().unwrap().len(), FOLLOWED);
    assert_eq!(full["followed"]["full"], true);
    assert_eq!(steps(&full)[1], "db.php +0/15 in");
    // Nothing is followed from what the graph does not know.
    for unknown in ["", "nowhere.php", "lib/core"] {
        assert!(
            index
                .follow("office", unknown, &[], &[], &[], None, false)
                .is_none(),
            "{unknown}"
        );
    }
}
#[test]
fn a_folder_is_followed_among_what_lies_as_deep_as_itself() {
    let index = office(14);
    assert_eq!(level("lib/deep/cache/store.php", 1), "lib");
    assert_eq!(level("lib/deep/cache/store.php", 2), "lib/deep");
    assert_eq!(level("lib/db.php", 2), "lib/db.php");
    assert_eq!(level("lib/db.php", usize::MAX), "lib/db.php");
    // A top folder among the top folders: the application and the
    // reports use the library, the reports through more links.
    let top = index
        .follow("office", "lib", &[], &named(&["lib"]), &[], None, false)
        .unwrap();
    assert_eq!(steps(&top), ["lib/ +0/0 in", "reports/ +0/0", "app/ +0/0"]);
    assert_eq!(
        rows(&top, "edges"),
        ["reports -> lib 14/0", "app -> lib 3/1"]
    );
    assert_eq!(
        (
            &top["scope"],
            &top["followed"]["folder"],
            &top["nodes"][0]["files"]
        ),
        (&json!(""), &json!(true), &json!(3))
    );
    // A folder two deep: the files that lie no deeper stand for
    // themselves, and what lies deeper is the folder at that depth.
    let api = index
        .follow(
            "office",
            "app/api",
            &named(&["app/api", "lib/core.php"]),
            &named(&["app/api"]),
            &[],
            None,
            false,
        )
        .unwrap();
    assert_eq!(
        steps(&api),
        [
            "api/ +0/0 out in",
            "core.php +0/0 out",
            "db.php +0/14",
            "main.php +0/0",
            "deep/ +0/0"
        ]
    );
    assert_eq!(api["nodes"][4]["path"], "lib/deep");
}
#[test]
fn followed_links_mark_a_chosen_change_and_list_both_sides_of_a_place() {
    let index = office(14);
    let changed = [
        ("lib/db.php".to_owned(), 'M'),
        ("reports/r01.php".to_owned(), 'A'),
        ("gone.php".to_owned(), 'D'),
    ];
    let view = index
        .follow(
            "office",
            "lib/core.php",
            &named(&["lib/core.php"]),
            &[],
            &changed,
            Some("lib/db.php"),
            false,
        )
        .unwrap();
    let marks = view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| (node["changed"].as_u64().unwrap(), node["status"].as_str()))
        .collect::<Vec<_>>();
    assert_eq!(marks, [(0, None), (1, Some("M")), (0, None)]);
    // The selected place lists every place on each side, drawn or not.
    let focus = &view["focus"];
    assert_eq!(
        (
            &focus["path"],
            &focus["name"],
            &focus["status"],
            &focus["start"]
        ),
        (
            &json!("lib/db.php"),
            &json!("db.php"),
            &json!("M"),
            &json!(false)
        )
    );
    assert_eq!(focus["uses"], json!([]));
    assert_eq!(
        (&focus["uses_more"], &focus["used_by_more"]),
        (&json!(0), &json!(4))
    );
    let users = focus["used_by"].as_array().unwrap();
    assert_eq!(users.len(), LISTED);
    assert_eq!(
        users[0],
        json!({"path": "lib/core.php", "name": "core.php", "folder": false, "inside": true, "links": 2, "guessed": 0, "changed": false})
    );
    assert_eq!(
        users[1],
        json!({"path": "app/api/orders.php", "name": "orders.php", "folder": false, "inside": false, "links": 1, "guessed": 1, "changed": false})
    );
    assert_eq!(users[2]["changed"], true);
    // What the followed place uses, by kind.
    let core = index
        .follow(
            "office",
            "lib/core.php",
            &[],
            &[],
            &[],
            Some("lib/core.php"),
            false,
        )
        .unwrap();
    assert_eq!(
        core["focus"]["kinds"],
        json!([{"kind": "calls", "links": 3}])
    );
    assert_eq!(core["focus"]["start"], true);
    // A folder that holds changed files says how many, not what happened.
    let top = index
        .follow(
            "office",
            "lib",
            &[],
            &named(&["lib"]),
            &changed,
            None,
            false,
        )
        .unwrap();
    let marks = top["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| (node["changed"].as_u64().unwrap(), node["status"].as_str()))
        .collect::<Vec<_>>();
    assert_eq!(marks, [(1, None), (1, None), (0, None)]);
    // A place that is not drawn is not selected.
    let absent = index
        .follow(
            "office",
            "lib/core.php",
            &[],
            &[],
            &[],
            Some("lib/db.php"),
            false,
        )
        .unwrap();
    assert_eq!(absent["focus"], Value::Null);
}
#[test]
fn a_test_is_told_by_where_a_file_lies_and_how_it_is_named() {
    for path in [
        "tests/CartTest.php",
        "tests/Feature/checkout.php",
        "src/Cart/Tests/CartHelper.php",
        "packages/cart/__tests__/total.js",
        "spec/models/cart_spec.rb",
        "internal/cart/testdata/order.go",
        "src/DataFixtures/UserFixtures.php",
        "database/fixtures/users.php",
        "web/__mocks__/api.ts",
        "e2e/login.ts",
        "cypress/support/commands.js",
        "App.Tests/CartTests.cs",
        "services/integration-tests/run.py",
        "crates/core/unit_tests/mod.rs",
        "src/UnitTests/Cart.cs",
        "src/Cart/CartTest.php",
        "src/Cart/CartTests.cs",
        "src/Cart/CartTestCase.php",
        "web/cart.test.tsx",
        "web/cart.spec.js",
        "api/app.e2e-spec.ts",
        "web/cart.cy.ts",
        "shop/test_cart.py",
        "shop/cart_test.py",
        "shop/conftest.py",
        "cart/cart_test.go",
        "src/cart_tests.rs",
        "src/cart/tests.rs",
        "src/cart_fixture.rs",
        "lib/cart_spec.rb",
    ] {
        assert!(is_test(path), "{path}");
    }
    for path in [
        "src/Cart/Cart.php",
        "src/Latest/News.php",
        "src/Contest/Entry.php",
        "src/Contest.php",
        "src/Attest.php",
        "src/LatestNews.php",
        "src/Protest/Vote.php",
        "specs/MANIFEST.md",
        "Task/Epics/Epic-01_Users_SPEC.md",
        "docs/testing.md",
        "web/jquery.min.js",
        "web/vite.config.ts",
        "src/Inspector.php",
        "src/TestimonialController.php",
        "src/fixture_loader.rs",
        "README.md",
        "",
    ] {
        assert!(!is_test(path), "{path}");
    }
}
/// A Rust source with what a build keeps and what only tests use.
const SOURCE: &str = "\
pub fn run() -> u32 {
    helper() + tool()
}

fn helper() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checks the sum.
    #[test]
    fn checks() {
        assert_eq!(run(), probe());
    }

    struct Fake;
    impl Fake {
        fn new() -> Self {
            Fake
        }
    }
}

#[cfg(not(test))]
fn live() {}

#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn standalone(
    seed: u32,
) {
    run();
}

#[cfg(test)] mod extra;
";
#[test]
fn the_tests_inside_a_rust_source_are_its_marked_items() {
    // The module from its attribute to its closing brace, the function
    // whose signature runs over lines, and the module on one line.
    assert_eq!(rust_tests(SOURCE), [(9, 25), (30, 36), (38, 38)]);
    assert_eq!(rust_tests("fn plain() {}\n"), Vec::<(u32, u32)>::new());
    assert_eq!(rust_tests(""), Vec::<(u32, u32)>::new());
    // An item that is never closed runs to the end of the text.
    assert_eq!(rust_tests("#[test]\nfn open() {\n    run();\n"), [(1, 3)]);
    // A test often holds a source of its own: a brace in a string, in a
    // raw string, in a character or in a comment is not the module's.
    let holding = "#[cfg(test)]\nmod tests {\n    const A: &str = \"\n}\n\";\n    const B: &str = r#\"\n}\"#;\n    const C: char = '}';\n    const D: char = '\\'';\n    // }\n    /* } /* } */ } */\n    fn f<'a>(x: &'a [u8; 2]) -> &'a [u8; 2] { x }\n}\nfn after() {}\n";
    assert_eq!(rust_tests(holding), [(1, 13)]);
    assert_eq!(item_end("fn a() { b(\"}\"); }", 0), 17);
    assert_eq!(item_end("static X: [u8; 2] = [1, 2]; fn b() {}", 0), 26);
    assert_eq!(item_end("mod r#type;", 0), 10);
    for attribute in [
        "test",
        "tokio::test",
        "tokio::test(flavor = \"multi_thread\")",
        "rstest",
        "test_case(1, 2)",
        "bench",
        "wasm_bindgen_test",
        "cfg(test)",
        "cfg(all(test, feature = \"slow\"))",
        "cfg(any(test, debug_assertions))",
    ] {
        assert!(marks_test(attribute), "{attribute}");
    }
    for attribute in [
        "cfg(not(test))",
        "cfg(all(not(test), unix))",
        "cfg(feature = \"test\")",
        "cfg(feature = \"test-utils\")",
        "cfg_attr(test, derive(Debug))",
        "derive(Debug)",
        "testing",
        "allow(dead_code)",
        "",
    ] {
        assert!(!marks_test(attribute), "{attribute}");
    }
    assert_eq!(attributes("#[cfg(test)] mod extra;"), (true, "mod extra;"));
    assert_eq!(
        attributes("#[doc = \"a ] b\"] #[test] fn x() {}"),
        (true, "fn x() {}")
    );
    assert_eq!(attributes("    fn plain() {}"), (false, "fn plain() {}"));
    assert_eq!(attributes("#[allow("), (false, ""));
}
#[test]
fn a_graph_without_tests_drops_test_files_and_counts_its_links_anew() {
    let index = shop();
    let plain = index.plain_with(&|_| None);
    // The one test file goes, with the two links it made.
    assert_eq!(plain.left_out(), (1, 0));
    assert_eq!(index.left_out(), (0, 0));
    assert_eq!(plain.files.len(), 5);
    assert!(!plain.files.iter().any(|file| file.starts_with("tests/")));
    let view = plain.view("shop", "", None, false, "");
    assert_eq!(
        view["totals"],
        json!({"files": 5, "symbols": 8, "relations": 10, "links": 4, "read": 2, "guessed": 2, "unplaced": 1, "ruled_out": 0})
    );
    assert_eq!(
        rows(&view, "nodes"),
        [
            "src folder files=4 out=0 in=0 within=4",
            "README.md file files=1 out=0 in=0 within=0"
        ]
    );
    // What is left reads as it did: the folder of the cart, its two links.
    let folder = plain.view("shop", "src/Cart", None, false, "");
    assert_eq!(
        rows(&folder, "edges"),
        ["src/Cart/Cart.php -> src/Cart/Price.php 2/1"]
    );
    // A folder of tests is not there to open: the project opens.
    assert_eq!(plain.view("shop", "tests", None, false, "")["scope"], "");
    assert_eq!(
        index.view("shop", "tests", None, false, "")["scope"],
        "tests"
    );
    assert!(
        plain
            .follow("shop", "tests/CartTest.php", &[], &[], &[], None, false)
            .is_none()
    );
    // The graph without tests is made once and kept.
    assert!(Arc::ptr_eq(&index.without_tests(), &index.without_tests()));
    assert_eq!(index.without_tests().left_out(), (1, 0));
}
#[test]
fn a_graph_without_tests_drops_the_tests_inside_a_rust_source() {
    let index = index(json!({
        "directed": false,
        "nodes": [
            {"id": "run", "label": "run()", "source_file": "src/lib.rs", "source_location": "L1"},
            {"id": "helper", "label": "helper()", "source_file": "src/lib.rs", "source_location": "L5"},
            {"id": "checks", "label": "checks()", "source_file": "src/lib.rs", "source_location": "L15"},
            {"id": "fake", "label": "Fake", "source_file": "src/lib.rs", "source_location": "L19"},
            {"id": "fake_new", "label": ".new()", "source_file": "src/lib.rs"},
            {"id": "live", "label": "live()", "source_file": "src/lib.rs", "source_location": "L28"},
            {"id": "standalone", "label": "standalone()", "source_file": "src/lib.rs", "source_location": "L32"},
            {"id": "tool", "label": "tool()", "source_file": "src/util.rs", "source_location": "L1"},
            {"id": "probe", "label": "probe()", "source_file": "src/util.rs", "source_location": "L5"}
        ],
        "links": [
            {"source": "run", "target": "helper", "relation": "calls", "confidence": "EXTRACTED", "source_file": "src/lib.rs"},
            {"source": "run", "target": "tool", "relation": "calls", "confidence": "EXTRACTED", "source_file": "src/lib.rs"},
            {"source": "live", "target": "tool", "relation": "calls", "confidence": "EXTRACTED", "source_file": "src/lib.rs"},
            {"source": "checks", "target": "run", "relation": "calls", "confidence": "EXTRACTED", "source_file": "src/lib.rs"},
            {"source": "checks", "target": "probe", "relation": "calls", "confidence": "EXTRACTED", "source_file": "src/lib.rs"},
            {"source": "standalone", "target": "run", "relation": "calls", "confidence": "INFERRED", "source_file": "src/lib.rs"},
            {"source": "fake", "target": "fake_new", "relation": "method", "confidence": "EXTRACTED", "source_file": "src/lib.rs"}
        ]
    }));
    let edges = |index: &Index| rows(&index.view("lib", "src", None, false, ""), "edges");
    assert_eq!(edges(&index), ["src/lib.rs -> src/util.rs 3/0"]);
    // A source that cannot be read keeps everything it holds.
    let unread = index.plain_with(&|_| None);
    assert_eq!(unread.left_out(), (0, 0));
    assert_eq!(edges(&unread), ["src/lib.rs -> src/util.rs 3/0"]);
    // Read, it loses the test, the test's type with its method, which
    // has no line of its own, and the test outside the module; the link
    // only the test made is gone from the file.
    let plain = index.plain_with(&|path| (path == "src/lib.rs").then(|| SOURCE.to_owned()));
    assert_eq!(plain.left_out(), (0, 4));
    assert_eq!(edges(&plain), ["src/lib.rs -> src/util.rs 2/0"]);
    let opened = plain.view("lib", "src/lib.rs", None, false, "");
    assert_eq!(opened["mode"], "file");
    let names = opened["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|node| node["symbol"] == true)
        .map(|node| node["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, ["run()", "helper()", "live()"]);
    assert_eq!(
        (&opened["file"]["functions"], &opened["file"]["types"]),
        (&json!(3), &json!(0))
    );
    // probe() is used by nothing that is left.
    let util = plain.view("lib", "src/util.rs", None, false, "");
    assert_eq!(util["file"]["used_by"], 1);
}
#[test]
fn a_link_between_crates_that_are_not_built_together_is_left_out() {
    let graph = json!({
            "directed": false,
            "nodes": [
                {"id": "main", "label": "main()", "source_file": "crates/app/src/main.rs"},
                {"id": "core", "label": "Core", "source_file": "crates/core/src/lib.rs"},
                {"id": "core_run", "label": ".run()", "source_file": "crates/core/src/lib.rs"},
                {"id": "tool", "label": "tool()", "source_file": "crates/tool/src/lib.rs"},
                {"id": "check", "label": "check()", "source_file": "crates/core/tests/it.rs"},
                {"id": "cart", "label": "Cart", "source_file": "web/cart.php"},
                {"id": "gen", "label": "gen()", "source_file": "scripts/gen.rs"}
            ],
            "links": [
                {"source": "core", "target": "core_run", "relation": "method", "confidence": "EXTRACTED", "source_file": "crates/core/src/lib.rs"},
                {"source": "main", "target": "core", "relation": "imports_from", "confidence": "EXTRACTED", "source_file": "crates/app/src/main.rs"},
                {"source": "main", "target": "tool", "relation": "calls", "confidence": "INFERRED", "source_file": "crates/app/src/main.rs"},
                {"source": "core", "target": "main", "relation": "imports_from", "confidence": "EXTRACTED", "source_file": "crates/core/src/lib.rs"},
                {"source": "core_run", "target": "tool", "relation": "calls", "confidence": "INFERRED", "source_file": "crates/core/src/lib.rs"},
                {"source": "tool", "target": "main", "relation": "calls", "confidence": "EXTRACTED", "source_file": "crates/tool/src/lib.rs"},
                {"source": "tool", "target": "core", "relation": "references", "confidence": "EXTRACTED", "source_file": "crates/tool/src/lib.rs"},
                {"source": "check", "target": "main", "relation": "calls", "confidence": "INFERRED", "source_file": "crates/core/tests/it.rs"},
                {"source": "check", "target": "core", "relation": "calls", "confidence": "EXTRACTED", "source_file": "crates/core/tests/it.rs"},
                {"source": "cart", "target": "core", "relation": "calls", "confidence": "INFERRED", "source_file": "web/cart.php"},
                {"source": "gen", "target": "tool", "relation": "calls", "confidence": "INFERRED", "source_file": "scripts/gen.rs"}
            ]
        })
        .to_string();
    // The app is built with the core; the tool's tests with the app.
    let manifests = [
        ("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n"),
        (
            "crates/app/Cargo.toml",
            "[package]\nname = \"shop-app\"\n[dependencies]\nshop-core = { path = \"../core\" }\n",
        ),
        (
            "crates/core/Cargo.toml",
            "[package]\nname = \"shop-core\"\n",
        ),
        (
            "crates/tool/Cargo.toml",
            "[package]\nname = \"shop-tool\"\n[dev-dependencies]\nshop-app = { path = \"../app\" }\n",
        ),
    ];
    let read = |manifests: &[(&str, &str)]| {
        Index::read_with(graph.as_bytes(), Path::new("/work/shop"), &|path| {
            manifests
                .iter()
                .find(|(at, _)| *at == path)
                .map(|(_, text)| (*text).to_owned())
        })
        .unwrap()
    };
    // The arrows of a folder, in the order of their names.
    let edges = |index: &Index, scope: &str| {
        let mut edges = rows(&index.view("shop", scope, None, false, ""), "edges");
        edges.sort();
        edges
    };
    // As the tool gave it: the core uses the app and the tool, which no
    // build of it could.
    let whole = read(&[]);
    assert_eq!(
        whole.view("shop", "", None, false, "")["totals"],
        json!({"files": 6, "symbols": 7, "relations": 11, "links": 10, "read": 5, "guessed": 5, "unplaced": 0, "ruled_out": 0})
    );
    assert_eq!(
        edges(&whole, "crates"),
        [
            "crates/app -> crates/core 1/0",
            "crates/app -> crates/tool 1/1",
            "crates/core -> crates/app 2/1",
            "crates/core -> crates/tool 1/1",
            "crates/tool -> crates/app 1/0",
            "crates/tool -> crates/core 1/0",
        ]
    );
    // With the manifests, four links cannot be: the app's into the
    // tool, and the three out of the core, one of them from its test.
    // What is not Rust, and the source of no crate, keep theirs.
    let index = read(&manifests);
    assert_eq!(
        index.view("shop", "", None, false, "")["totals"],
        json!({"files": 6, "symbols": 7, "relations": 11, "links": 6, "read": 4, "guessed": 2, "unplaced": 0, "ruled_out": 4})
    );
    assert_eq!(
        edges(&index, "crates"),
        [
            "crates/app -> crates/core 1/0",
            "crates/tool -> crates/app 1/0",
            "crates/tool -> crates/core 1/0",
        ]
    );
    assert_eq!(
        edges(&index, ""),
        ["scripts -> crates 1/1", "web -> crates 1/1"]
    );
    // The file that was said to use two others uses none.
    let core = index.view("shop", "crates/core/src/lib.rs", None, false, "");
    assert_eq!(
        (&core["file"]["uses"], &core["file"]["used_by"]),
        (&json!(0), &json!(4))
    );
    // Without tests, the link ruled out of a test is not counted twice.
    let plain = index.plain_with(&|_| None);
    assert_eq!(plain.left_out(), (1, 0));
    assert_eq!(
        plain.view("shop", "", None, false, "")["totals"],
        json!({"files": 5, "symbols": 6, "relations": 9, "links": 5, "read": 3, "guessed": 2, "unplaced": 0, "ruled_out": 3})
    );
}
fn store() -> Index {
    let cart = "src/Cart/Cart.php";
    let price = "src/Cart/Price.php";
    let notifier = "src/Order/Notifier.php";
    let test = "tests/CartTest.php";
    let node = |id: &str, label: &str, file: &str, line: u32| json!({"id": id, "label": label, "file_type": "code", "source_file": file, "source_location": format!("L{line}")});
    let class = |id: &str, label: &str, file: &str, line: u32| {
        let mut node = node(id, label, file, line);
        node["_callable_class"] = json!(true);
        node
    };
    let link = |source: &str, relation: &str, target: &str, file: &str| json!({"source": source, "target": target, "relation": relation, "confidence": "EXTRACTED", "source_file": file});
    let guess = |source: &str, relation: &str, target: &str, file: &str| json!({"source": source, "target": target, "relation": relation, "confidence": "INFERRED", "source_file": file});
    index(json!({
        "nodes": [
            node("cart_php", "Cart.php", cart, 1),
            class("cart", "Cart", cart, 12),
            node("cart_add", ".add()", cart, 18),
            node("cart_total", ".total()", cart, 19),
            node("cart_checkout", ".checkout()", cart, 26),
            // The tool gives a Rust struct no class flag: what has
            // cases or methods is a type all the same.
            node("status", "Status", cart, 10),
            node("status_open", "Open", cart, 10),
            node("status_paid", "Paid", cart, 10),
            node("format", "format_price()", cart, 29),
            node("limit", "LIMIT", cart, 5),
            {"id": "why", "label": "Totals are kept in cents so that sums never round.", "file_type": "rationale", "source_file": cart, "source_location": "L2"},
            // The node of a file can be named by the end of its path.
            node("price_php", "Cart/Price.php", price, 1),
            class("price", "Price", price, 4),
            node("price_of", ".of()", price, 9),
            node("price_plus", ".plus()", price, 10),
            node("notifier_php", "Notifier.php", notifier, 1),
            class("notifier", "Notifier", notifier, 8),
            node("notifier_paid", ".paid()", notifier, 11),
            node("test_php", "CartTest.php", test, 1),
            class("test", "CartTest", test, 5),
            node("test_total", ".testTotal()", test, 7),
            node("test_empty", ".testEmpty()", test, 15),
            {"id": "external", "label": "Countable", "file_type": "code", "source_file": "", "source_location": ""}
        ],
        "links": [
            link("cart_php", "contains", "cart", cart),
            link("cart_php", "contains", "status", cart),
            link("cart_php", "contains", "format", cart),
            link("cart_php", "contains", "limit", cart),
            link("cart", "method", "cart_add", cart),
            link("cart", "method", "cart_total", cart),
            link("cart", "method", "cart_checkout", cart),
            link("status", "case_of", "status_open", cart),
            link("status", "case_of", "status_paid", cart),
            link("why", "rationale_for", "cart", cart),
            link("price_php", "contains", "price", price),
            link("price", "method", "price_of", price),
            link("price", "method", "price_plus", price),
            link("notifier_php", "contains", "notifier", notifier),
            link("notifier", "method", "notifier_paid", notifier),
            link("test_php", "contains", "test", test),
            link("test", "method", "test_total", test),
            link("test", "method", "test_empty", test),
            // Inside the file.
            link("cart_checkout", "calls", "cart_total", cart),
            link("cart_checkout", "references", "status", cart),
            link("cart_add", "references", "limit", cart),
            link("cart_total", "calls", "cart_total", cart),
            // Out of it.
            link("cart_php", "imports", "notifier", cart),
            link("cart", "implements", "external", cart),
            link("cart_total", "calls", "price_of", cart),
            link("cart_total", "calls", "price_plus", cart),
            link("cart_add", "references", "price", cart),
            guess("cart_checkout", "calls", "notifier_paid", cart),
            guess("format", "calls", "price_plus", cart),
            // Into it.
            link("test_total", "calls", "cart_total", test),
            link("test_total", "calls", "cart_add", test),
            guess("test_empty", "calls", "cart_total", test),
            link("notifier_paid", "calls", "format", notifier)
        ]
    }))
}
const CART: &str = "src/Cart/Cart.php";
/// The symbols of an opened file, or the files around it, as drawn.
fn drawn(view: &Value, symbols: bool) -> Vec<String> {
    view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|node| (node["symbol"] == true) == symbols)
        .map(|node| {
            let path = node["path"].as_str().unwrap();
            if symbols {
                format!(
                    "{} {} {}{} out={} in={}",
                    path.strip_prefix(CART).unwrap_or(path),
                    node["kind"].as_str().unwrap(),
                    node["name"].as_str().unwrap(),
                    match node["members"].as_u64().unwrap() {
                        0 => String::new(),
                        members => format!(" holds {members}"),
                    },
                    node["out"],
                    node["in"]
                )
            } else {
                format!(
                    "{path} {} {}{} holds {} out={} in={}",
                    node["role"].as_str().unwrap(),
                    node["name"].as_str().unwrap(),
                    match node["status"].as_str() {
                        Some(status) => format!(" [{status}]"),
                        None => String::new(),
                    },
                    node["symbols"],
                    node["out"],
                    node["in"]
                )
            }
        })
        .collect()
}
/// The rows of one side of the selection: the symbol, how it is linked,
/// and where it is when that is another file.
fn told(focus: &Value, side: &str) -> Vec<String> {
    focus[side]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            format!(
                "{} {} {}{}{}",
                row["name"].as_str().unwrap(),
                row["kind"].as_str().unwrap(),
                match row["relation"].as_str() {
                    Some(relation) => relation.to_owned(),
                    None => format!("{} links", row["links"]),
                },
                if row["guessed"].as_u64().unwrap() > 0 {
                    format!(" guessed={}", row["guessed"])
                } else {
                    String::new()
                },
                match row["file"].as_str() {
                    Some(file) => format!(" in {file}"),
                    None => String::new(),
                }
            )
        })
        .collect()
}
fn short(edges: Vec<String>) -> Vec<String> {
    edges
        .into_iter()
        .map(|edge| edge.replace(CART, ""))
        .collect()
}
#[test]
fn a_file_opens_to_its_classes_methods_and_functions() {
    let view = store().view("shop", CART, None, false, "");
    assert_eq!(
        (view["mode"].as_str(), view["scope"].as_str()),
        (Some("file"), Some(CART))
    );
    // Every symbol is a box, the most linked first: a method under the
    // name of its class, the file's own node under the file's name. The
    // comment the tool kept is not a symbol.
    assert_eq!(
        drawn(&view, true),
        [
            "#L19:.total() method Cart.total() out=2 in=3",
            "#L18:.add() method Cart.add() out=2 in=1",
            "#L26:.checkout() method Cart.checkout() out=3 in=0",
            "#L29:format_price() function format_price() out=1 in=1",
            "#L1:Cart.php file Cart.php out=1 in=0",
            "#L5:LIMIT name LIMIT out=0 in=1",
            "#L10:Status type Status holds 2 out=0 in=1",
            "#L10:Open case Open out=0 in=0",
            "#L10:Paid case Paid out=0 in=0",
            "#L12:Cart type Cart holds 3 out=0 in=0",
        ]
    );
    // The files around are the ones those symbols use or are used by.
    assert_eq!(
        drawn(&view, false),
        [
            "src/Cart/Price.php used Price.php holds 3 out=0 in=4",
            "src/Order/Notifier.php both Notifier.php holds 2 out=1 in=2",
            "tests/CartTest.php uses CartTest.php holds 3 out=3 in=0",
        ]
    );
    assert_eq!(
        short(rows(&view, "edges")),
        [
            "#L19:.total() -> src/Cart/Price.php 2/0",
            "tests/CartTest.php -> #L19:.total() 2/1",
            "#L18:.add() -> #L5:LIMIT 1/0",
            "#L18:.add() -> src/Cart/Price.php 1/0",
            "#L1:Cart.php -> src/Order/Notifier.php 1/0",
            "#L26:.checkout() -> #L10:Status 1/0",
            "#L26:.checkout() -> #L19:.total() 1/0",
            "#L26:.checkout() -> src/Order/Notifier.php 1/1",
            "#L29:format_price() -> src/Cart/Price.php 1/1",
            "src/Order/Notifier.php -> #L29:format_price() 1/0",
            "tests/CartTest.php -> #L18:.add() 1/0",
        ]
    );
    // A link to something outside the project, as an interface of the
    // language, has no place and is not counted.
    assert_eq!(
        view["file"],
        json!({
            "path": CART, "name": "Cart.php", "status": null, "touched": null,
            "symbols": 9, "types": 2, "methods": 3, "functions": 1, "headings": 0, "others": 3,
            "inside": 3, "out": 6, "in": 4, "uses": 2, "used_by": 2,
        })
    );
    assert_eq!(
        (
            view["children"].as_u64(),
            view["matching"].as_u64(),
            &view["around"]
        ),
        (Some(10), Some(10), &json!({"files": 3, "shown": 3}))
    );
    assert_eq!(
        view["crumbs"],
        json!([
            {"name": "shop", "path": ""},
            {"name": "src", "path": "src"},
            {"name": "Cart", "path": "src/Cart"},
            {"name": "Cart.php", "path": CART},
        ])
    );
    // Between files nothing changes: the links inside a file are not
    // links between files.
    let project = store().view("shop", "", None, false, "");
    assert_eq!(project["totals"]["links"], 10);
    assert_eq!(rows(&project, "edges"), ["tests -> src 3/1"]);
    // A folder still opens as a folder, and a path the graph does not
    // know opens the project.
    assert!(store().view("shop", "src/Cart", None, false, "")["mode"].is_null());
    assert_eq!(
        store().view("shop", "src/Cart/Gone.php", None, false, "")["scope"],
        ""
    );
}
#[test]
fn a_selected_symbol_lists_what_it_uses_and_what_uses_it() {
    let index = store();
    let key = format!("{CART}#L19:.total()");
    let view = index.view("shop", CART, Some(&key), false, "");
    let focus = &view["focus"];
    assert_eq!(
        (
            focus["name"].as_str(),
            focus["kind"].as_str(),
            focus["owner"].as_str(),
            focus["line"].as_u64()
        ),
        (Some("Cart.total()"), Some("method"), Some("Cart"), Some(19))
    );
    // A call of itself is no link.
    assert_eq!(
        told(focus, "uses"),
        [
            "Price.of() method calls in src/Cart/Price.php",
            "Price.plus() method calls in src/Cart/Price.php",
        ]
    );
    // What was read comes before what was guessed, this file first.
    assert_eq!(
        told(focus, "used_by"),
        [
            "Cart.checkout() method calls",
            "CartTest.testTotal() method calls in tests/CartTest.php",
            "CartTest.testEmpty() method calls guessed=1 in tests/CartTest.php",
        ]
    );
    assert_eq!(focus["kinds"], json!([{"kind": "calls", "links": 2}]));
    // A symbol of another file is named the way that file selects it.
    assert_eq!(focus["uses"][0]["path"], "src/Cart/Price.php#L9:.of()");
    assert_eq!(
        (&focus["uses"][0]["inside"], &focus["used_by"][0]["inside"]),
        (&json!(false), &json!(true))
    );
    // A type lists what it holds, in the order of the file.
    let key = format!("{CART}#L12:Cart");
    let view = index.view("shop", CART, Some(&key), false, "");
    let held = view["focus"]["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            format!(
                "{} {} {}",
                row["name"].as_str().unwrap(),
                row["kind"].as_str().unwrap(),
                row["line"]
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        held,
        [
            ".add() method 18",
            ".total() method 19",
            ".checkout() method 26"
        ]
    );
    assert_eq!(view["focus"]["members_more"], 0);
    // Only what the tool read.
    let key = format!("{CART}#L26:.checkout()");
    let certain = index.view("shop", CART, Some(&key), true, "");
    assert_eq!(
        told(&certain["focus"], "uses"),
        ["Status type references", "Cart.total() method calls"]
    );
    assert_eq!(certain["file"]["out"], 4);
    // A name that is not a symbol of the file selects nothing.
    let view = index.view(
        "shop",
        CART,
        Some("src/Cart/Cart.php#L99:.gone()"),
        false,
        "",
    );
    assert!(view["focus"].is_null());
}
#[test]
fn a_selected_file_around_lists_its_symbols_that_are_linked_with_the_open_one() {
    let index = store();
    let view = index.view("shop", CART, Some("src/Order/Notifier.php"), false, "");
    let focus = &view["focus"];
    assert_eq!(
        (
            focus["name"].as_str(),
            focus["kind"].as_str(),
            focus["role"].as_str(),
            focus["symbols"].as_u64()
        ),
        (Some("Notifier.php"), Some("file"), Some("both"), Some(2))
    );
    // Its symbols that use the open file, and its symbols the open file
    // uses, each with the number of links.
    assert_eq!(
        told(focus, "uses"),
        ["Notifier.paid() method 1 links in src/Order/Notifier.php"]
    );
    assert_eq!(
        told(focus, "used_by"),
        [
            "Notifier type 1 links in src/Order/Notifier.php",
            "Notifier.paid() method 1 links guessed=1 in src/Order/Notifier.php",
        ]
    );
    assert_eq!(focus["kinds"], json!([{"kind": "calls", "links": 1}]));
    let view = index.view("shop", CART, Some("src/Cart/Price.php"), false, "");
    assert_eq!(
        told(&view["focus"], "used_by"),
        [
            "Price.plus() method 2 links guessed=1 in src/Cart/Price.php",
            "Price type 1 links in src/Cart/Price.php",
            "Price.of() method 1 links in src/Cart/Price.php",
        ]
    );
    // A file that is not around the open one is not selected.
    assert!(index.view("shop", CART, Some("README.md"), false, "")["focus"].is_null());
}
#[test]
fn a_filter_and_the_limit_choose_which_symbols_of_a_file_are_drawn() {
    let shop = store();
    let view = shop.view("shop", CART, None, false, "cart.");
    // The name a method is shown under includes its class. The file's
    // own node is named after the class too, and stays out.
    assert_eq!(
        drawn(&view, true),
        [
            "#L19:.total() method Cart.total() out=2 in=3",
            "#L18:.add() method Cart.add() out=2 in=1",
            "#L26:.checkout() method Cart.checkout() out=3 in=0",
        ]
    );
    assert_eq!(
        (view["children"].as_u64(), view["matching"].as_u64()),
        (Some(10), Some(3))
    );
    // The selected symbol is drawn whatever the filter says.
    let key = format!("{CART}#L29:format_price()");
    let view = shop.view("shop", CART, Some(&key), false, "status");
    assert_eq!(
        drawn(&view, true),
        [
            "#L10:Status type Status holds 2 out=0 in=1",
            "#L29:format_price() function format_price() out=1 in=1"
        ]
    );
    assert_eq!(view["focus"]["name"], "format_price()");
    // The view says that the filter would have left it out, and says
    // nothing of the kind where the selected symbol is drawn anyway.
    assert_eq!(view["beyond"], true);
    let key = format!("{CART}#L10:Status");
    assert_eq!(
        shop.view("shop", CART, Some(&key), false, "status")["beyond"],
        false
    );
    // A long file keeps its most linked symbols, and at least a few
    // places for the files around.
    let mut nodes = vec![
        json!({"id": "wide", "label": "wide.py", "source_file": "wide.py", "source_location": "L1"}),
    ];
    let mut links = vec![];
    for at in 0..NAMED + 10 {
        nodes.push(json!({"id": format!("f{at}"), "label": format!("f{at}()"), "source_file": "wide.py", "source_location": format!("L{}", at + 2)}));
        links.push(json!({"source": "wide", "target": format!("f{at}"), "relation": "contains", "source_file": "wide.py"}));
        nodes.push(json!({"id": format!("user{at}"), "label": format!("user{at}()"), "source_file": format!("users/u{at:02}.py"), "source_location": "L1"}));
        // The later a function stands, the more it is used.
        for user in 0..=at {
            links.push(json!({"source": format!("user{user}"), "target": format!("f{at}"), "relation": "calls", "source_file": format!("users/u{user:02}.py")}));
        }
    }
    let wide = index(json!({"nodes": nodes, "links": links}));
    let view = wide.view("wide", "wide.py", None, false, "");
    let symbols = drawn(&view, true);
    assert_eq!(symbols.len(), NAMED);
    assert_eq!(
        symbols[0],
        format!(
            "wide.py#L{}:f{}() function f{}() out=0 in={}",
            NAMED + 11,
            NAMED + 9,
            NAMED + 9,
            NAMED + 10
        )
    );
    assert_eq!(
        (view["children"].as_u64(), view["matching"].as_u64()),
        (Some(NAMED as u64 + 10), Some(NAMED as u64 + 10))
    );
    assert_eq!(
        view["around"],
        json!({"files": NAMED + 10, "shown": AROUND})
    );
    assert_eq!(view["nodes"].as_array().unwrap().len(), SHOWN);
    // The least used function is not drawn until it is selected.
    assert!(!symbols.iter().any(|symbol| symbol.contains("f0()")));
    let view = wide.view("wide", "wide.py", Some("wide.py#L2:f0()"), false, "");
    assert_eq!(drawn(&view, true).len(), NAMED + 1);
    assert_eq!(view["focus"]["used_by"][0]["name"], "user0()");
}
#[test]
fn a_change_opens_a_file_too_and_marks_the_changed_files_around_it() {
    let index = store();
    let changed = [
        (CART, 'M'),
        ("src/Cart/Price.php", 'A'),
        ("src/Old.php", 'D'),
    ];
    let view = change(&index, &changed, CART, None);
    assert_eq!(
        (view["mode"].as_str(), view["file"]["status"].as_str()),
        (Some("file"), Some("M"))
    );
    assert_eq!(
        drawn(&view, false),
        [
            "src/Cart/Price.php used Price.php [A] holds 3 out=0 in=4",
            "src/Order/Notifier.php both Notifier.php holds 2 out=1 in=2",
            "tests/CartTest.php uses CartTest.php holds 3 out=3 in=0",
        ]
    );
    assert_eq!(
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node["changed"] == 1)
            .count(),
        1
    );
    // A file that did not change opens as well.
    let beside = change(&index, &changed, "tests/CartTest.php", None);
    assert_eq!(
        (beside["mode"].as_str(), &beside["file"]["status"]),
        (Some("file"), &Value::Null)
    );
    // The map of the change says what each of its files holds.
    let whole = change(&index, &changed, "", None);
    assert_eq!(whole["mode"], "changes");
    assert_eq!(
        whole["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| format!("{} {}", node["path"].as_str().unwrap(), node["symbols"]))
            .collect::<Vec<_>>(),
        [
            "src/Cart/Cart.php 9",
            "src/Cart/Price.php 3",
            "src/Order/Notifier.php 2",
            "tests/CartTest.php 3"
        ]
    );
    // And so does a folder of the project's map.
    let folder = index.view("shop", "src/Cart", Some("src/Cart/Cart.php"), false, "");
    assert_eq!(folder["nodes"][0]["symbols"], 9);
    assert_eq!(folder["focus"]["symbols"], 9);
}
/// A class with three methods, a function below it and a statement at
/// the bottom, as a change leaves the file.
const BASKET: &str = "<?php
namespace Shop;

use Shop\\Notifier;
use Shop\\Price;

/** A cart. */
class Cart
{
    private array $items = [];

    /** Adds a price. */
    public function add(Price $price): void
    {
        $this->items[] = $price;
    }

    public function total(): int
    {
        $sum = 0;
        foreach ($this->items as $item) {
            $sum += $item->cents() * 2;
        }
        return $sum;
    }

    /** Counts the items. */
    #[Pure]
    public function count(): int
    {
        return count($this->items);
    }
}

function format_price(Price $price): string
{
    return number_format($price->cents() / 100, 2);
}

echo format_price(new Price(1));
";
fn basket() -> Index {
    index(basket_graph())
}
fn basket_graph() -> Value {
    let file = "src/Cart.php";
    let node = |id: &str, label: &str, line: u32| json!({"id": id, "label": label, "file_type": "code", "source_file": file, "source_location": format!("L{line}")});
    let holds = |holder: &str, relation: &str, held: &str| json!({"source": holder, "target": held, "relation": relation, "source_file": file});
    json!({
        "nodes": [
            node("file", "Cart.php", 1),
            node("cart", "Cart", 8),
            node("add", ".add()", 13),
            node("total", ".total()", 18),
            node("count", ".count()", 29),
            node("format", "format_price()", 35),
            {"id": "other", "label": "other.php", "source_file": "src/other.php", "source_location": "L1"},
            {"id": "user", "label": "user()", "source_file": "src/other.php", "source_location": "L3"}
        ],
        "links": [
            holds("file", "contains", "cart"),
            holds("file", "contains", "format"),
            holds("cart", "method", "add"),
            holds("cart", "method", "total"),
            holds("cart", "method", "count"),
            {"source": "other", "target": "user", "relation": "contains", "source_file": "src/other.php"},
            {"source": "user", "target": "format", "relation": "calls", "source_file": "src/other.php"},
            {"source": "file", "target": "user", "relation": "imports", "source_file": file}
        ]
    })
}
fn hunk(at: u32, added: u32, removed: u32) -> Hunk {
    Hunk {
        at,
        added,
        removed,
        blank: false,
    }
}
/// The symbols of an opened file with what a change did to each: Git's
/// letter, and how many symbols changed inside.
fn marked(index: &Index, scope: &str, text: &str, hunks: &[Hunk]) -> (Vec<String>, Value) {
    let edited = Edited {
        new: false,
        text: text.to_owned(),
        hunks: hunks.to_vec(),
    };
    let changed = [(scope.to_owned(), 'M')];
    let view = index
        .symbols("shop", scope, &changed, Some(&edited), None, false, "")
        .unwrap();
    let symbols = view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|node| node["symbol"] == true)
        .map(|node| {
            format!(
                "{}{}{}",
                node["name"].as_str().unwrap(),
                match node["status"].as_str() {
                    Some(status) => format!(" {status}"),
                    None => String::new(),
                },
                match node["touched"].as_u64().unwrap() {
                    0 => String::new(),
                    inside => format!(" holds {inside}"),
                }
            )
        })
        .collect();
    (symbols, view["file"]["touched"].clone())
}
#[test]
fn a_change_marks_the_symbols_whose_lines_it_touched() {
    let index = basket();
    // An import is added, a line of `total` is rewritten, `count` is
    // inserted with the comment and the attribute above it, and the
    // statement at the bottom of the file is rewritten.
    let (symbols, reach) = marked(
        &index,
        "src/Cart.php",
        BASKET,
        &[
            hunk(5, 1, 0),
            hunk(22, 1, 1),
            hunk(26, 7, 0),
            hunk(40, 1, 1),
        ],
    );
    // What the change touched comes first. The class is touched through
    // its methods, the lines outside every symbol are the file's own,
    // and `add`, right above the inserted method, is not touched.
    assert_eq!(
        symbols,
        [
            "Cart.php M",
            "Cart holds 2",
            "Cart.total() M",
            "Cart.count() A",
            "format_price()",
            "Cart.add()",
        ]
    );
    assert_eq!(
        reach,
        json!({"symbols": 2, "added": 1, "modified": 1, "between": 0, "top": true})
    );
    // A property of the class is the class's own line.
    let (symbols, _) = marked(&index, "src/Cart.php", BASKET, &[hunk(10, 1, 1)]);
    assert_eq!(symbols[0], "Cart M");
    assert_eq!(
        symbols
            .iter()
            .filter(|symbol| symbol.contains(" M"))
            .count(),
        1
    );
    // The comment above a method is the method's, and a blank line is
    // nobody's.
    let (symbols, reach) = marked(
        &index,
        "src/Cart.php",
        BASKET,
        &[hunk(12, 1, 1), hunk(17, 1, 0)],
    );
    assert_eq!(symbols[..2], ["Cart holds 1", "Cart.add() M"]);
    assert_eq!(reach["symbols"], 1);
    // Lines removed inside a method touch it; lines removed between two
    // symbols are counted, and removed blank lines are not.
    let gone = |at: u32, blank: bool| Hunk {
        at,
        added: 0,
        removed: 2,
        blank,
    };
    let (symbols, reach) = marked(
        &index,
        "src/Cart.php",
        BASKET,
        &[gone(15, false), gone(33, false), gone(20, true)],
    );
    assert_eq!(symbols[..2], ["Cart holds 1", "Cart.add() M"]);
    assert_eq!(
        (reach["symbols"].as_u64(), reach["between"].as_u64()),
        (Some(1), Some(1))
    );
    // A function that is new down to its last line is added, and one
    // whose lines replaced others is modified.
    let (symbols, _) = marked(&index, "src/Cart.php", BASKET, &[hunk(34, 5, 0)]);
    assert_eq!(symbols[0], "format_price() A");
    let (symbols, _) = marked(&index, "src/Cart.php", BASKET, &[hunk(34, 5, 2)]);
    assert_eq!(symbols[0], "format_price() M");
}
#[test]
fn a_line_past_the_end_of_a_symbol_belongs_to_what_holds_it() {
    let node = |id: &str, label: &str, line: u32| json!({"id": id, "label": label, "file_type": "code", "source_file": "app.py", "source_location": format!("L{line}")});
    let index = index(json!({
        "nodes": [
            node("app", "app.py", 1),
            node("outer", "outer()", 3),
            node("inner", "inner()", 5),
            node("other", "other()", 11),
            node("where", "bounded()", 15)
        ],
        "links": [
            {"source": "app", "target": "outer", "relation": "contains", "source_file": "app.py"},
            {"source": "outer", "target": "inner", "relation": "contains", "source_file": "app.py"},
            {"source": "app", "target": "other", "relation": "contains", "source_file": "app.py"},
            {"source": "app", "target": "where", "relation": "contains", "source_file": "app.py"}
        ]
    }));
    let text = "import os

def outer():
    x = 1
    def inner():
        return 2
    return inner() + x

# What follows.
@cached
def other(): return answer()
print(other())

# A signature that goes on at its own depth.
fn bounded<T>(x: T) -> u32
where
    T: Debug,
{
    1
}
";
    // The line after the inner function is the outer one's.
    let (symbols, _) = marked(&index, "app.py", text, &[hunk(7, 1, 1)]);
    assert_eq!(symbols[0], "outer() M");
    assert!(!symbols.iter().any(|symbol| symbol.starts_with("inner() ")));
    // A decorator is the function's below it; a statement after a
    // function that is whole on its line is the file's.
    let (symbols, reach) = marked(&index, "app.py", text, &[hunk(10, 1, 1), hunk(12, 1, 1)]);
    assert_eq!(symbols[0], "other() M");
    assert_eq!(
        reach,
        json!({"symbols": 1, "added": 0, "modified": 1, "between": 0, "top": true})
    );
    // A body that opens after a clause at the signature's depth is
    // still the function's.
    let (symbols, reach) = marked(&index, "app.py", text, &[hunk(19, 1, 1)]);
    assert_eq!(symbols[0], "bounded() M");
    assert_eq!(reach["top"], false);
    // A symbol the graph places beyond the text is not marked: the
    // line that changed is the file's own.
    let (symbols, reach) = marked(&index, "app.py", "import os\n", &[hunk(1, 1, 1)]);
    assert!(
        symbols.iter().all(|symbol| !symbol.contains(' ')),
        "{symbols:?}"
    );
    assert_eq!(
        (&reach["symbols"], &reach["top"]),
        (&json!(0), &json!(true))
    );
}
#[test]
fn a_new_file_is_new_throughout_and_an_unread_change_marks_nothing() {
    let index = basket();
    let changed = [("src/Cart.php".to_owned(), 'A')];
    let new = Edited {
        new: true,
        ..Edited::default()
    };
    let view = index
        .symbols(
            "shop",
            "src/Cart.php",
            &changed,
            Some(&new),
            None,
            false,
            "",
        )
        .unwrap();
    assert!(
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node["symbol"] == true)
            .all(|node| node["status"] == "A" && node["changed"] == 1)
    );
    assert_eq!(
        view["file"]["touched"],
        json!({"symbols": 5, "added": 5, "modified": 0, "between": 0, "top": true})
    );
    // The selected symbol and the rows of its side say it too.
    let view = index
        .symbols(
            "shop",
            "src/Cart.php",
            &changed,
            Some(&new),
            Some("src/Cart.php#L8:Cart"),
            false,
            "",
        )
        .unwrap();
    assert_eq!(
        (
            &view["focus"]["status"],
            &view["focus"]["touched"],
            &view["focus"]["members"][0]["changed"]
        ),
        (&json!("A"), &json!(3), &json!(true))
    );
    // Without what the change did, nothing is marked and the view says
    // that it does not know.
    let plain = index
        .symbols("shop", "src/Cart.php", &changed, None, None, false, "")
        .unwrap();
    assert!(plain["file"]["touched"].is_null());
    assert!(
        plain["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|node| node["status"].is_null())
    );
    assert!(
        index
            .symbols("shop", "src", &changed, None, None, false, "")
            .is_none()
    );
    assert_eq!(index.opens("/src/Cart.php/"), Some("src/Cart.php"));
    assert_eq!(index.opens("src"), None);
}
#[test]
fn a_line_is_read_for_its_depth_and_for_what_it_is() {
    let read = lines(
        "fn a() {\n\tlet x = 1;\n  \n    // note\n    #[test]\n}\nwhere\nend_of(x)\n  * a note\n*p = 1;\n",
    );
    let told = read
        .iter()
        .map(|line| (line.indent, line.kind, line.whole))
        .collect::<Vec<_>>();
    assert_eq!(
        told,
        [
            (0, Row::Code, false),
            (4, Row::Code, true),
            (2, Row::Blank, false),
            (4, Row::Comment, false),
            (4, Row::Comment, false),
            (0, Row::Closer, true),
            (0, Row::Closer, false),
            (0, Row::Code, true),
            (2, Row::Comment, false),
            (0, Row::Code, true),
        ]
    );
}
#[test]
fn what_a_symbol_is_comes_from_the_graph_and_odd_graphs_are_read() {
    let graph = index(json!({
        "nodes": [
            // A document: the page is the file, the headings its symbols.
            {"id": "page", "label": "docs/guide.md", "file_type": "document", "node_kind": "page", "source_file": "docs/guide.md", "source_location": "L1"},
            {"id": "h1", "label": "Guide", "file_type": "document", "node_kind": "heading", "source_file": "docs/guide.md", "source_location": "L1"},
            {"id": "h2", "label": "Install", "file_type": "document", "node_kind": "heading", "source_file": "docs/guide.md", "source_location": "L5"},
            // No node for the file, a label that is a number, a place
            // that is a number, fields of an unexpected shape.
            {"id": "a", "label": 42, "source_file": "odd.py", "source_location": 7, "file_type": ["code"], "_callable_class": "yes", "node_kind": {"a": 1}},
            {"id": "b", "label": "run()", "source_file": "odd.py", "source_location": "L9-L12", "_callable_class": null},
            {"id": "c", "label": "inner()", "source_file": "odd.py", "source_location": "L10"},
            // Two symbols of one name on one line, and two that hold
            // each other.
            {"id": "d", "label": "same", "source_file": "odd.py", "source_location": "L20"},
            {"id": "e", "label": "same", "source_file": "odd.py", "source_location": "L20"},
            {"id": "f", "label": "loop", "source_file": "odd.py"},
            {"id": "g", "label": "pool", "source_file": "odd.py"},
            {"id": "long", "label": "ж".repeat(200), "source_file": "odd.py", "source_location": "L30"}
        ],
        "links": [
            {"source": "page", "target": "h1", "relation": "contains", "source_file": "docs/guide.md"},
            {"source": "h1", "target": "h2", "relation": "contains", "source_file": "docs/guide.md"},
            {"source": "h2", "target": "b", "relation": "references", "source_file": "docs/guide.md"},
            {"source": "b", "target": "c", "relation": "contains", "source_file": "odd.py"},
            {"source": "b", "target": "b", "relation": "contains", "source_file": "odd.py"},
            {"source": "f", "target": "g", "relation": "contains", "source_file": "odd.py"},
            {"source": "g", "target": "f", "relation": "contains", "source_file": "odd.py"},
            // A link found in a third file says nothing of these two.
            {"source": "d", "target": "e", "relation": "calls", "source_file": "docs/guide.md"}
        ]
    }));
    let guide = graph.view("p", "docs/guide.md", None, false, "");
    let listed = |view: &Value| {
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node["symbol"] == true)
            .map(|node| {
                format!(
                    "{} {} {}",
                    node["path"].as_str().unwrap(),
                    node["kind"].as_str().unwrap(),
                    node["owner"].as_str().unwrap_or("-")
                )
            })
            .collect::<Vec<_>>()
    };
    // The page has no link of its own, so it is no box.
    assert_eq!(
        listed(&guide),
        [
            "docs/guide.md#L5:Install heading Guide",
            "docs/guide.md#L1:Guide heading -"
        ]
    );
    assert_eq!(
        (&guide["file"]["headings"], &guide["file"]["others"]),
        (&json!(2), &json!(0))
    );
    let odd = graph.view("p", "odd.py", None, false, "");
    let long = format!("odd.py#L30:{} name -", "ж".repeat(NAME_LIMIT / 2));
    assert_eq!(
        listed(&odd),
        [
            "odd.py#L9:run() function -",
            "odd.py#loop name pool",
            "odd.py#pool name loop",
            "odd.py#L7:42 name -",
            "odd.py#L10:inner() function run()",
            "odd.py#L20:same name -",
            "odd.py#L20:same~2 name -",
            long.as_str(),
        ]
    );
    assert_eq!(graph.unplaced, 0);
    assert_eq!(odd["file"]["inside"], 0);
    // The second of two that share a name is selected by its own.
    let second = graph.view("p", "odd.py", Some("odd.py#L20:same~2"), false, "");
    assert_eq!(second["focus"]["path"], "odd.py#L20:same~2");
    assert_eq!(line(Some("L42")), 42);
    assert_eq!(
        (line(Some("12-20")), line(Some("top")), line(None)),
        (12, 0, 0)
    );
    assert_eq!(clip("жук", 3), "ж");
}
#[test]
fn a_graph_is_read_once_until_its_file_changes() {
    let folder = tempfile::tempdir().unwrap();
    let graph = folder.path().join("graph.json");
    let maps = Maps::default();
    assert!(maps.index("p", &graph, folder.path()).is_err());
    std::fs::write(
        &graph,
        json!({"nodes": [{"id": "a", "source_file": "a.py"}], "links": []}).to_string(),
    )
    .unwrap();
    let first = maps.index("p", &graph, folder.path()).unwrap();
    let again = maps.index("p", &graph, folder.path()).unwrap();
    assert!(Arc::ptr_eq(&first, &again));
    std::fs::write(&graph, json!({"nodes": [{"id": "a", "source_file": "a.py"}, {"id": "b", "source_file": "b.py"}], "links": []}).to_string()).unwrap();
    let changed = maps.index("p", &graph, folder.path()).unwrap();
    assert!(!Arc::ptr_eq(&first, &changed));
    assert_eq!(changed.files.len(), 2);
    maps.forget("p");
    assert!(!Arc::ptr_eq(
        &changed,
        &maps.index("p", &graph, folder.path()).unwrap()
    ));
}

#[test]
fn maps_index_releases_lock_during_read_deduplicates_same_key_and_evicts_lru() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    let folder = tempfile::tempdir().unwrap();
    let make_graph = |name: &str, file: &str| {
        let path = folder.path().join(name);
        std::fs::write(
            &path,
            json!({"nodes": [{"id": file, "source_file": file}], "links": []}).to_string(),
        )
        .unwrap();
        path
    };

    let graph_a = make_graph("a.json", "a.py");
    let graph_b = make_graph("b.json", "b.py");
    let maps = Arc::new(Maps::default());

    // 1. Parsing project A does not block project B, while a second caller for A
    // joins the in-flight read without double-parsing.
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let a_reads = Arc::new(AtomicUsize::new(0));

    let maps_a1 = maps.clone();
    let path_a1 = graph_a.clone();
    let root_a1 = folder.path().to_path_buf();
    let reads_a1 = a_reads.clone();
    let t_a1 = std::thread::spawn(move || {
        maps_a1.load_with("proj-a", &path_a1, |path| {
            reads_a1.fetch_add(1, Ordering::SeqCst);
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Index::read(std::fs::File::open(path)?, &root_a1)
        })
    });

    entered_rx.recv().unwrap();

    // While A is parked inside Index::read, B parses immediately without blocking.
    let idx_b = maps.index("proj-b", &graph_b, folder.path()).unwrap();
    assert_eq!(idx_b.files, vec!["b.py"]);

    // Second concurrent caller for A waits on the single-flight slot and never runs its reader.
    let maps_a2 = maps.clone();
    let path_a2 = graph_a.clone();
    let reads_a2 = a_reads.clone();
    let t_a2 = std::thread::spawn(move || {
        maps_a2.load_with("proj-a", &path_a2, |_| {
            reads_a2.fetch_add(1, Ordering::SeqCst);
            panic!("concurrent caller for same project must not double-parse");
        })
    });

    // Give t_a2 a moment to attach to the in-flight slot before releasing A.
    std::thread::sleep(std::time::Duration::from_millis(25));
    release_tx.send(()).unwrap();

    let idx_a1 = t_a1.join().unwrap().unwrap();
    let idx_a2 = t_a2.join().unwrap().unwrap();
    assert!(Arc::ptr_eq(&idx_a1, &idx_a2));
    assert_eq!(a_reads.load(Ordering::SeqCst), 1);

    // 2. LRU eviction: fill KEPT (= 4) slots, touch the oldest ("p1") so "p2" becomes LRU,
    // then insert a 5th entry and confirm "p2" is evicted while "p1", "p3", "p4", "p5" stay cached.
    let lru_maps = Maps::default();
    let mut paths = Vec::new();
    for i in 1..=5 {
        paths.push((
            format!("p{i}"),
            make_graph(&format!("p{i}.json"), &format!("p{i}.py")),
        ));
    }
    let mut refs = Vec::new();
    for (name, path) in &paths[0..4] {
        refs.push(lru_maps.index(name, path, folder.path()).unwrap());
    }
    // Cache hit on p1 promotes it to MRU; p2 is now the LRU entry.
    let p1_hit = lru_maps
        .load_with(&paths[0].0, &paths[0].1, |_| {
            panic!("p1 should still be cached")
        })
        .unwrap();
    assert!(Arc::ptr_eq(&refs[0], &p1_hit));

    // Adding p5 must evict p2 (LRU), not p1 (FIFO oldest).
    let p5 = lru_maps
        .index(&paths[4].0, &paths[4].1, folder.path())
        .unwrap();
    assert_eq!(p5.files, vec!["p5.py"]);

    let p1_after = lru_maps
        .load_with(&paths[0].0, &paths[0].1, |_| {
            panic!("p1 was promoted and must remain cached")
        })
        .unwrap();
    assert!(Arc::ptr_eq(&refs[0], &p1_after));

    for idx in [2usize, 3] {
        let cached = lru_maps
            .load_with(&paths[idx].0, &paths[idx].1, |_| {
                panic!("p{} must remain cached", idx + 1)
            })
            .unwrap();
        assert!(Arc::ptr_eq(&refs[idx], &cached));
    }

    let p2_reloaded = Arc::new(AtomicUsize::new(0));
    let p2_flag = p2_reloaded.clone();
    let p2_after = lru_maps
        .load_with(&paths[1].0, &paths[1].1, |path| {
            p2_flag.fetch_add(1, Ordering::SeqCst);
            Index::read(std::fs::File::open(path)?, folder.path())
        })
        .unwrap();
    assert_eq!(p2_reloaded.load(Ordering::SeqCst), 1);
    assert!(!Arc::ptr_eq(&refs[1], &p2_after));

    // 3. forget() while an in-flight read is running prevents stale caching.
    let (f_enter_tx, f_enter_rx) = mpsc::channel();
    let (f_rel_tx, f_rel_rx) = mpsc::channel();
    let maps_f = Arc::new(Maps::default());
    let maps_f_bg = maps_f.clone();
    let g_forget = paths[0].1.clone();
    let root_f = folder.path().to_path_buf();
    let t_forget = std::thread::spawn(move || {
        maps_f_bg.load_with("p-forget", &g_forget, |path| {
            f_enter_tx.send(()).unwrap();
            f_rel_rx.recv().unwrap();
            Index::read(std::fs::File::open(path)?, &root_f)
        })
    });
    f_enter_rx.recv().unwrap();
    maps_f.forget("p-forget");
    f_rel_tx.send(()).unwrap();
    let stale = t_forget.join().unwrap().unwrap();
    let fresh_reads = Arc::new(AtomicUsize::new(0));
    let fresh_flag = fresh_reads.clone();
    let fresh = maps_f
        .load_with("p-forget", &paths[0].1, |path| {
            fresh_flag.fetch_add(1, Ordering::SeqCst);
            Index::read(std::fs::File::open(path)?, folder.path())
        })
        .unwrap();
    assert_eq!(fresh_reads.load(Ordering::SeqCst), 1);
    assert!(!Arc::ptr_eq(&stale, &fresh));
}
/// The links drawn among followed symbols, by the names of their ends.
fn calls(view: &Value) -> Vec<String> {
    let name = |key: &Value| {
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| &node["path"] == key)
            .map(|node| node["name"].as_str().unwrap().to_owned())
            .unwrap()
    };
    view["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| {
            format!(
                "{} -> {} {}/{}",
                name(&edge["from"]),
                name(&edge["to"]),
                edge["links"],
                edge["guessed"]
            )
        })
        .collect()
}
const TOTAL_KEY: &str = "src/Cart/Cart.php#L19:.total()";
const CHECKOUT_KEY: &str = "src/Cart/Cart.php#L26:.checkout()";
#[test]
fn the_links_of_a_method_are_followed_among_symbols() {
    let index = store();
    let follow = |out: &[&str], into: &[&str], focus: Option<&str>, certain: bool| {
        index
            .follow(
                "shop",
                TOTAL_KEY,
                &named(out),
                &named(into),
                &[],
                focus,
                certain,
            )
            .unwrap()
    };
    // The method alone at first, with how many it uses and how many use
    // it. The way back leads to its file, where it lies.
    let alone = follow(&[], &[], None, false);
    assert_eq!(
        (&alone["mode"], &alone["scope"]),
        (&json!("follow"), &json!(CART))
    );
    assert_eq!(steps(&alone), ["Cart.total() +2/3"]);
    assert_eq!(
        alone["followed"],
        json!({
            "path": TOTAL_KEY, "name": "Cart.total()", "folder": false, "symbol": true,
            "kind": "method", "file": CART, "line": 19, "uses": 2, "users": 3,
            "shown": 1, "limit": FOLLOWED, "step": STEP, "full": false, "aside": 0
        })
    );
    assert_eq!(
        alone["crumbs"],
        json!([
            {"name": "shop", "path": ""},
            {"name": "src", "path": "src"},
            {"name": "Cart", "path": "src/Cart"},
            {"name": "Cart.php", "path": CART},
            {"name": "Cart.total()", "path": TOTAL_KEY}
        ])
    );
    let start = &alone["nodes"][0];
    assert_eq!(
        (
            &start["path"],
            &start["symbol"],
            &start["kind"],
            &start["line"]
        ),
        (
            &json!(TOTAL_KEY),
            &json!(true),
            &json!("method"),
            &json!(19)
        )
    );
    assert_eq!(
        (
            &start["file"],
            &start["owner"],
            &start["start"],
            &start["changed"]
        ),
        (&json!(CART), &json!("Cart"), &json!(true), &json!(0))
    );
    assert_eq!((&start["out"], &start["in"]), (&json!(2), &json!(3)));
    assert_eq!(calls(&alone), Vec::<String>::new());
    // One step each way: what it calls, in the order of their lines, and
    // what calls it. What was read from the code comes before what was
    // guessed, and its own file before the others.
    let both = follow(&[TOTAL_KEY], &[TOTAL_KEY], None, false);
    assert_eq!(
        steps(&both),
        [
            "Cart.total() +0/0 out in",
            "Price.of() +0/0",
            "Price.plus() +0/1",
            "Cart.checkout() +2/0",
            "CartTest.testTotal() +1/0",
            "CartTest.testEmpty() +0/0",
        ]
    );
    assert_eq!(
        sorted(calls(&both)),
        [
            "Cart.checkout() -> Cart.total() 1/0",
            "Cart.total() -> Price.of() 1/0",
            "Cart.total() -> Price.plus() 1/0",
            "CartTest.testEmpty() -> Cart.total() 1/1",
            "CartTest.testTotal() -> Cart.total() 1/0",
        ]
    );
    assert_eq!(both["followed"]["shown"], 6);
    // Each symbol is named as the picture of its own file selects it.
    assert_eq!(both["nodes"][1]["path"], "src/Cart/Price.php#L9:.of()");
    assert_eq!(both["nodes"][1]["file"], "src/Cart/Price.php");
    // A step further from one of them: what checkout uses besides joins
    // after what was there, its own file first and the guess last.
    let further = follow(&[TOTAL_KEY, CHECKOUT_KEY], &[TOTAL_KEY], None, false);
    assert_eq!(
        steps(&further)[3..],
        [
            "Cart.checkout() +0/0 out",
            "CartTest.testTotal() +1/0",
            "CartTest.testEmpty() +0/0",
            "Status +0/0",
            "Notifier.paid() +1/0",
        ]
    );
    assert!(calls(&further).contains(&"Cart.checkout() -> Notifier.paid() 1/1".to_owned()));
    assert!(calls(&further).contains(&"Cart.checkout() -> Status 1/0".to_owned()));
    // Without what the tool inferred the guessed caller is not there.
    let read = follow(&[TOTAL_KEY], &[TOTAL_KEY], None, true);
    assert_eq!(read["followed"]["users"], 2);
    assert!(!steps(&read).iter().any(|step| step.contains("testEmpty")));
    // A selected symbol lists both sides, each with how it is linked,
    // where it lies and whether the picture draws it.
    let chosen = follow(&[TOTAL_KEY], &[], Some(TOTAL_KEY), false);
    let focus = &chosen["focus"];
    assert_eq!(
        (
            &focus["name"],
            &focus["kind"],
            &focus["file"],
            &focus["start"]
        ),
        (
            &json!("Cart.total()"),
            &json!("method"),
            &json!(CART),
            &json!(true)
        )
    );
    let told = |side: &str| {
        focus[side]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                format!(
                    "{} {} in {}{}{}",
                    row["relation"].as_str().unwrap(),
                    row["name"].as_str().unwrap(),
                    row["file"].as_str().unwrap(),
                    if row["inside"] == true { " drawn" } else { "" },
                    if row["guessed"] == 1 { " guessed" } else { "" },
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        told("uses"),
        [
            "calls Price.of() in src/Cart/Price.php drawn",
            "calls Price.plus() in src/Cart/Price.php drawn",
        ]
    );
    assert_eq!(
        told("used_by"),
        [
            "calls Cart.checkout() in src/Cart/Cart.php",
            "calls CartTest.testTotal() in tests/CartTest.php",
            "calls CartTest.testEmpty() in tests/CartTest.php guessed",
        ]
    );
    assert_eq!(focus["used_by"][0]["path"], CHECKOUT_KEY);
    assert_eq!(focus["kinds"], json!([{"kind": "calls", "links": 2}]));
    // A symbol the picture does not draw is not selected.
    assert_eq!(
        follow(&[], &[], Some(CHECKOUT_KEY), false)["focus"],
        Value::Null
    );
    // A class is followed by its own links, not by those of its methods,
    // and the node of a file stands for what the file imports.
    let class = index
        .follow(
            "shop",
            "src/Order/Notifier.php#L8:Notifier",
            &[],
            &named(&["src/Order/Notifier.php#L8:Notifier"]),
            &[],
            None,
            false,
        )
        .unwrap();
    assert_eq!(steps(&class), ["Notifier +0/0 in", "Cart.php +0/0"]);
    assert_eq!(calls(&class), ["Cart.php -> Notifier 1/0"]);
    assert_eq!(class["nodes"][1]["kind"], "file");
    // What names no symbol is followed as a file or a folder, or not at
    // all.
    let none = |root: &str| index.follow("shop", root, &[], &[], &[], None, false);
    assert_eq!(none(CART).unwrap()["followed"]["folder"], false);
    assert!(none(CART).unwrap()["followed"].get("symbol").is_none());
    assert!(none("src/Cart/Cart.php#L19:.gone()").is_none());
    assert!(none("src/Cart/Nothing.php#L1:x").is_none());
}
#[test]
fn followed_symbols_stop_at_a_full_picture_and_count_the_links_left_aside() {
    // One function that a hundred others call, each from a file of its
    // own, and that calls two.
    let mut nodes = vec![
        json!({"id": "hub", "label": "hub()", "source_file": "lib/hub.py", "source_location": "L1"}),
        json!({"id": "left", "label": "left()", "source_file": "lib/hub.py", "source_location": "L5"}),
        json!({"id": "right", "label": "right()", "source_file": "lib/hub.py", "source_location": "L9"}),
    ];
    let call = |from: &str, to: &str, file: &str| json!({"source": from, "target": to, "relation": "calls", "confidence": "EXTRACTED", "source_file": file});
    let mut links = vec![
        call("hub", "left", "lib/hub.py"),
        call("hub", "right", "lib/hub.py"),
        call("left", "right", "lib/hub.py"),
    ];
    for at in 0..100 {
        let (id, file) = (format!("u{at:03}"), format!("users/u{at:03}.py"));
        nodes.push(json!({"id": id, "label": format!("user{at:03}()"), "source_file": file, "source_location": "L1"}));
        links.push(call(&id, "hub", &file));
    }
    let index = index(json!({"nodes": nodes, "links": links}));
    let hub = "lib/hub.py#L1:hub()";
    let follow = |out: &[&str], into: &[&str]| {
        index
            .follow("hub", hub, &named(out), &named(into), &[], None, false)
            .unwrap()
    };
    // A step shows the next twelve of a side, and says how many wait.
    let once = follow(&[], &[hub]);
    assert_eq!(steps(&once)[0], format!("hub() +2/{} in", 100 - STEP));
    assert_eq!(once["nodes"].as_array().unwrap().len(), 1 + STEP);
    assert_eq!(once["nodes"][1]["name"], "user000()");
    // Named twice, the side shows twelve more.
    let twice = follow(&[], &[hub, hub]);
    assert_eq!(twice["nodes"].as_array().unwrap().len(), 1 + 2 * STEP);
    // The picture holds so many boxes and says that it is full.
    let all = vec![hub; 10];
    let full = follow(&[hub], &all);
    assert_eq!(full["nodes"].as_array().unwrap().len(), FOLLOWED);
    assert_eq!(full["followed"]["full"], true);
    assert_eq!(full["followed"]["users"], 100);
    // What it calls joined first; the link between the two it calls is
    // not one of its own, and waits for a side that holds it.
    assert_eq!(steps(&full)[1], "left() +1/0");
    assert_eq!(steps(&full)[2], "right() +0/1");
    assert_eq!(full["followed"]["aside"], 1);
    let opened = follow(&[hub, "lib/hub.py#L5:left()"], &[]);
    assert!(calls(&opened).contains(&"left() -> right() 1/0".to_owned()));
    assert_eq!(opened["followed"]["aside"], 0);
}
#[test]
fn followed_symbols_carry_what_a_chosen_change_did_to_them() {
    let index = basket();
    let format = "src/Cart.php#L35:format_price()";
    let changed = [
        ("src/Cart.php".to_owned(), 'M'),
        ("src/other.php".to_owned(), 'A'),
    ];
    let asked = RefCell::new(Vec::new());
    let edited = |path: &str| {
        asked.borrow_mut().push(path.to_owned());
        Some(match path {
            // The lines of the function replaced others.
            "src/Cart.php" => Edited {
                new: false,
                text: BASKET.to_owned(),
                hunks: vec![hunk(34, 5, 2)],
            },
            _ => Edited {
                new: true,
                ..Edited::default()
            },
        })
    };
    let view = index
        .follow_with(
            "shop",
            format,
            &[],
            &named(&[format]),
            &changed,
            &edited,
            Some(format),
            false,
        )
        .unwrap();
    let marks = view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| {
            format!(
                "{} {} {}",
                node["name"].as_str().unwrap(),
                node["changed"],
                node["status"].as_str().unwrap_or("-")
            )
        })
        .collect::<Vec<_>>();
    // The function is modified, and its caller lies in a new file.
    assert_eq!(marks, ["format_price() 1 M", "user() 1 A"]);
    assert_eq!(view["focus"]["status"], "M");
    // A row beside it says the same of the symbol it names.
    assert_eq!(view["focus"]["used_by"][0]["changed"], true);
    // Each file of the change is asked for once, however many of its
    // symbols are drawn and listed.
    assert_eq!(
        sorted(asked.borrow().clone()),
        ["src/Cart.php", "src/other.php"]
    );
    // A function the change left as it was carries no mark, though its
    // file changed; and a file outside the change is not asked for.
    let asked = RefCell::new(Vec::new());
    let elsewhere = |path: &str| {
        asked.borrow_mut().push(path.to_owned());
        Some(Edited {
            new: false,
            text: BASKET.to_owned(),
            hunks: vec![hunk(22, 1, 1)],
        })
    };
    let view = index
        .follow_with(
            "shop",
            format,
            &[],
            &named(&[format]),
            &changed[..1],
            &elsewhere,
            None,
            false,
        )
        .unwrap();
    assert_eq!(
        (&view["nodes"][0]["changed"], &view["nodes"][0]["status"]),
        (&json!(0), &Value::Null)
    );
    assert_eq!(view["nodes"][1]["changed"], 0);
    assert_eq!(*asked.borrow(), ["src/Cart.php"]);
    // Nor does a row that names it, from the other end of the link.
    let from_user = index
        .follow_with(
            "shop",
            "src/other.php#L3:user()",
            &[],
            &[],
            &changed[..1],
            &elsewhere,
            Some("src/other.php#L3:user()"),
            false,
        )
        .unwrap();
    let row = &from_user["focus"]["uses"][0];
    assert_eq!(
        (&row["name"], &row["changed"], &row["inside"]),
        (&json!("format_price()"), &json!(false), &json!(false))
    );
    // Where the change cannot be told, as with a graph older than the
    // files, links are followed without marks.
    let plain = index
        .follow(
            "shop",
            format,
            &[],
            &named(&[format]),
            &changed,
            None,
            false,
        )
        .unwrap();
    assert_eq!(plain["nodes"][0]["changed"], 0);
    assert_eq!(steps(&plain), ["format_price() +0/0 in", "user() +0/1"]);
}
/// The places of a path as they are drawn: the step each stands at, its
/// name, and whether the path starts or ends there.
fn way(view: &Value) -> Vec<String> {
    view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| {
            format!(
                "{} {}{}{}",
                node["step"],
                node["name"].as_str().unwrap(),
                if node["start"] == true { " start" } else { "" },
                if node["end"] == true { " end" } else { "" },
            )
        })
        .collect()
}
/// What a path says of itself: whether one was found and which way it
/// runs, its steps, the places and links on such ways and how many are
/// drawn.
fn told_of(view: &Value) -> String {
    let path = &view["path"];
    format!(
        "{}{}{} steps={} places={} links={} shown={}",
        if path["found"] == true {
            "found"
        } else {
            "none"
        },
        if path["reversed"] == true {
            " reversed"
        } else {
            ""
        },
        if path["within"] == true {
            " within"
        } else {
            ""
        },
        path["steps"],
        path["places"],
        path["links"],
        path["shown"],
    )
}
#[test]
fn the_shortest_ways_between_two_files_are_chains_of_files() {
    let index = office(3);
    let path = |from: &str, to: &str, focus: Option<&str>, certain: bool| {
        index
            .path("office", from, to, &[], &|_| None, focus, certain)
            .unwrap()
    };
    // The entry point reaches the database in two links, through the
    // orders, whose link was only guessed.
    let short = path("app/main.php", "lib/db.php", None, false);
    assert_eq!(short["mode"], "path");
    assert_eq!(told_of(&short), "found steps=2 places=3 links=2 shown=3");
    assert_eq!(
        way(&short),
        ["0 main.php start", "1 orders.php", "2 db.php end"]
    );
    assert_eq!(
        sorted(calls(&short)),
        ["main.php -> orders.php 1/0", "orders.php -> db.php 1/1"]
    );
    // The ends are told as they were asked for, and the way back leads
    // to where the first of them lies.
    assert_eq!(
        short["path"]["from"],
        json!({"path": "app/main.php", "name": "main.php", "what": "file"})
    );
    assert_eq!(short["path"]["to"]["name"], "db.php");
    assert_eq!(short["path"]["names"], false);
    assert_eq!(short["scope"], "app");
    assert_eq!(
        short["crumbs"],
        json!([
            {"name": "office", "path": ""},
            {"name": "app", "path": "app"},
            {"name": "main.php → db.php", "path": ""}
        ])
    );
    // Without what the tool guessed the way takes three links, and both
    // files of the API lie on a shortest one.
    let read = path("app/main.php", "lib/db.php", None, true);
    assert_eq!(told_of(&read), "found steps=3 places=5 links=5 shown=5");
    assert_eq!(
        way(&read),
        [
            "0 main.php start",
            "1 orders.php",
            "1 users.php",
            "2 core.php",
            "3 db.php end"
        ]
    );
    assert_eq!(
        sorted(calls(&read)),
        [
            "core.php -> db.php 2/0",
            "main.php -> orders.php 1/0",
            "main.php -> users.php 1/0",
            "orders.php -> core.php 1/0",
            "users.php -> core.php 1/0",
        ]
    );
    // A place drawn lists what it leads to on the way and what leads to
    // it, and how it uses the next.
    let chosen = path("app/main.php", "lib/db.php", Some("lib/core.php"), true);
    let focus = &chosen["focus"];
    assert_eq!(
        (&focus["path"], &focus["step"]),
        (&json!("lib/core.php"), &json!(2))
    );
    let rows = |side: &str| {
        focus[side]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                format!(
                    "{} {}{}",
                    row["name"].as_str().unwrap(),
                    row["links"],
                    if row["inside"] == true { " drawn" } else { "" }
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(rows("uses"), ["db.php 2 drawn"]);
    assert_eq!(rows("used_by"), ["users.php 1 drawn", "orders.php 1 drawn"]);
    assert_eq!(
        (&focus["uses_more"], &focus["used_by_more"]),
        (&json!(0), &json!(0))
    );
    assert_eq!(focus["kinds"], json!([{"kind": "calls", "links": 2}]));
    // Nothing leads from the database to the entry point: the way that
    // is there runs the other way round, and is said to.
    let back = path("lib/db.php", "app/main.php", None, false);
    assert_eq!(
        told_of(&back),
        "found reversed steps=2 places=3 links=2 shown=3"
    );
    assert_eq!(way(&back), way(&short));
    assert_eq!(back["path"]["from"]["name"], "db.php");
    // Two files that neither way leads between have no path.
    let none = path("lib/db.php", "lib/deep/cache/store.php", None, false);
    assert_eq!(told_of(&none), "none steps=0 places=0 links=0 shown=0");
    assert_eq!(none["nodes"], json!([]));
    assert_eq!(none["mode"], "path");
    // An end the graph does not know is no path at all.
    assert!(
        index
            .path(
                "office",
                "app/main.php",
                "lib/gone.php",
                &[],
                &|_| None,
                None,
                false
            )
            .is_none()
    );
}
#[test]
fn a_folder_at_an_end_of_a_path_stands_for_every_file_in_it() {
    let index = office(3);
    let path = |from: &str, to: &str| {
        index
            .path("office", from, to, &[], &|_| None, None, false)
            .unwrap()
    };
    // The application uses the library directly, through two of its
    // files; the entry point does not, and is on no shortest way.
    let direct = path("app", "lib");
    assert_eq!(told_of(&direct), "found steps=1 places=4 links=3 shown=4");
    assert_eq!(
        way(&direct),
        [
            "0 orders.php start",
            "0 users.php start",
            "1 core.php end",
            "1 db.php end"
        ]
    );
    assert_eq!(
        direct["path"]["from"],
        json!({"path": "app", "name": "app", "what": "folder"})
    );
    assert_eq!(direct["scope"], "");
    // A folder and a file further on: both files of the API reach the
    // cache through the core.
    let cache = path("app", "lib/deep/cache/store.php");
    assert_eq!(told_of(&cache), "found steps=2 places=4 links=3 shown=4");
    assert_eq!(
        way(&cache),
        [
            "0 orders.php start",
            "0 users.php start",
            "1 core.php",
            "2 store.php end"
        ]
    );
    // What lies in the other end is no way to it.
    let inside = path("lib", "lib/db.php");
    assert_eq!(
        told_of(&inside),
        "none within steps=0 places=0 links=0 shown=0"
    );
    assert_eq!(told_of(&path("lib/db.php", "lib/db.php")), told_of(&inside));
    // More files start a shortest way than a step shows: the most
    // linked are drawn, by their paths where they are linked alike, and
    // all are counted.
    let wide = office(30);
    let many = wide
        .path(
            "office",
            "reports",
            "lib/db.php",
            &[],
            &|_| None,
            None,
            false,
        )
        .unwrap();
    assert_eq!(
        told_of(&many),
        format!("found steps=1 places=31 links=30 shown={}", WAY + 1)
    );
    assert_eq!(many["path"]["wide"], json!(WAY));
    assert_eq!(way(&many)[0], "0 r01.php start");
    assert_eq!(way(&many)[WAY], "1 db.php end");
    assert_eq!(calls(&many).len(), WAY);
    // A place of the picture says how many more lie on such ways.
    let end = wide
        .path(
            "office",
            "reports",
            "lib/db.php",
            &[],
            &|_| None,
            Some("lib/db.php"),
            false,
        )
        .unwrap();
    assert_eq!(
        (
            end["focus"]["used_by"].as_array().unwrap().len(),
            &end["focus"]["used_by_more"]
        ),
        (WAY, &json!(30 - WAY))
    );
    // A long way shows fewer at a step, and at least one.
    let mut nodes = Vec::new();
    let mut links = Vec::new();
    for at in 0..40 {
        nodes.push(json!({"id": format!("n{at}"), "label": format!("n{at}"), "source_file": format!("chain/f{at:02}.py")}));
        if at > 0 {
            links.push(json!({"source": format!("n{}", at - 1), "target": format!("n{at}"), "relation": "calls", "source_file": format!("chain/f{:02}.py", at - 1)}));
        }
    }
    // A second way of the same length for the first step.
    nodes.push(json!({"id": "side", "label": "side", "source_file": "chain/side.py"}));
    links.push(json!({"source": "n0", "target": "side", "relation": "calls", "source_file": "chain/f00.py"}));
    links.push(json!({"source": "side", "target": "n2", "relation": "calls", "source_file": "chain/side.py"}));
    let chain = self::index(json!({"nodes": nodes, "links": links}));
    let long = chain
        .path(
            "chain",
            "chain/f00.py",
            "chain/f39.py",
            &[],
            &|_| None,
            None,
            false,
        )
        .unwrap();
    assert_eq!(told_of(&long), "found steps=39 places=41 links=41 shown=40");
    assert_eq!(long["path"]["wide"], 1);
}
#[test]
fn a_path_from_or_to_a_symbol_is_a_chain_of_symbols() {
    let index = store();
    let path = |from: &str, to: &str, focus: Option<&str>, certain: bool| {
        index
            .path("shop", from, to, &[], &|_| None, focus, certain)
            .unwrap()
    };
    let test_total = "tests/CartTest.php#L7:.testTotal()";
    let price_of = "src/Cart/Price.php#L9:.of()";
    // The test reaches the price through the total of the cart.
    let chain = path(test_total, price_of, Some(TOTAL_KEY), false);
    assert_eq!(told_of(&chain), "found steps=2 places=3 links=2 shown=3");
    assert_eq!(chain["path"]["names"], true);
    assert_eq!(
        way(&chain),
        [
            "0 CartTest.testTotal() start",
            "1 Cart.total()",
            "2 Price.of() end"
        ]
    );
    assert_eq!(
        sorted(calls(&chain)),
        [
            "Cart.total() -> Price.of() 1/0",
            "CartTest.testTotal() -> Cart.total() 1/0"
        ]
    );
    // Each is a symbol of its own file, told as that file's picture
    // selects it, and the ends say what they are.
    let middle = &chain["nodes"][1];
    assert_eq!(
        (
            &middle["path"],
            &middle["symbol"],
            &middle["kind"],
            &middle["file"]
        ),
        (
            &json!(TOTAL_KEY),
            &json!(true),
            &json!("method"),
            &json!(CART)
        )
    );
    assert_eq!(
        chain["path"]["from"],
        json!({
            "path": test_total, "name": "CartTest.testTotal()", "what": "method",
            "symbol": true, "file": "tests/CartTest.php", "line": 7
        })
    );
    assert_eq!(chain["scope"], "tests/CartTest.php");
    assert_eq!(
        chain["crumbs"].as_array().unwrap().last().unwrap()["name"],
        "CartTest.testTotal() → Price.of()"
    );
    // The selected one lists the next and the one before, each with how
    // the two are linked.
    let focus = &chain["focus"];
    assert_eq!(
        (
            &focus["uses"][0]["name"],
            &focus["uses"][0]["relation"],
            &focus["uses"][0]["inside"]
        ),
        (&json!("Price.of()"), &json!("calls"), &json!(true))
    );
    assert_eq!(focus["used_by"][0]["path"], test_total);
    assert_eq!(focus["kinds"], json!([{"kind": "calls", "links": 1}]));
    // A file at the other end stands for every symbol in it: checkout
    // reaches two methods of the price, both through the total.
    let file = path(CHECKOUT_KEY, "src/Cart/Price.php", None, false);
    assert_eq!(told_of(&file), "found steps=2 places=4 links=3 shown=4");
    assert_eq!(
        way(&file),
        [
            "0 Cart.checkout() start",
            "1 Cart.total()",
            "2 Price.of() end",
            "2 Price.plus() end"
        ]
    );
    assert_eq!(file["path"]["to"]["what"], "file");
    // So does a folder, and a file at the start: something in the tests
    // reaches the limit through the adding.
    let folder = path("tests", "src/Cart/Cart.php#L5:LIMIT", None, false);
    assert_eq!(
        way(&folder),
        [
            "0 CartTest.testTotal() start",
            "1 Cart.add()",
            "2 LIMIT end"
        ]
    );
    // The way that is there runs from the second end to the first.
    let back = path("src/Cart/Price.php#L10:.plus()", CHECKOUT_KEY, None, true);
    assert_eq!(
        told_of(&back),
        "found reversed steps=2 places=3 links=2 shown=3"
    );
    assert_eq!(
        way(&back),
        [
            "0 Cart.checkout() start",
            "1 Cart.total()",
            "2 Price.plus() end"
        ]
    );
    // A guess is a way too, until only what was read is asked for.
    let guessed = path(
        CHECKOUT_KEY,
        "src/Order/Notifier.php#L11:.paid()",
        None,
        false,
    );
    assert_eq!(calls(&guessed), ["Cart.checkout() -> Notifier.paid() 1/1"]);
    let read = path(
        CHECKOUT_KEY,
        "src/Order/Notifier.php#L11:.paid()",
        None,
        true,
    );
    assert_eq!(told_of(&read), "none steps=0 places=0 links=0 shown=0");
    // A symbol of the file at the other end lies in it.
    let own = path(TOTAL_KEY, CART, None, false);
    assert_eq!(own["path"]["within"], true);
}
#[test]
fn a_path_carries_what_a_chosen_change_did_to_its_places() {
    // Among symbols: the caller lies in a new file, and the lines of
    // the function replaced others.
    let index = basket();
    let changed = [
        ("src/Cart.php".to_owned(), 'M'),
        ("src/other.php".to_owned(), 'A'),
    ];
    let edited = |path: &str| {
        Some(match path {
            "src/Cart.php" => Edited {
                new: false,
                text: BASKET.to_owned(),
                hunks: vec![hunk(34, 5, 2)],
            },
            _ => Edited {
                new: true,
                ..Edited::default()
            },
        })
    };
    let view = index
        .path(
            "shop",
            "src/other.php#L3:user()",
            "src/Cart.php#L35:format_price()",
            &changed,
            &edited,
            None,
            false,
        )
        .unwrap();
    let marks = |view: &Value| {
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| {
                format!(
                    "{} {} {}",
                    node["name"].as_str().unwrap(),
                    node["changed"],
                    node["status"].as_str().unwrap_or("-")
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(marks(&view), ["user() 1 A", "format_price() 1 M"]);
    // Among files: a file of the change carries Git's letter.
    let files = index
        .path(
            "shop",
            "src/other.php",
            "src/Cart.php",
            &changed[..1],
            &edited,
            Some("src/other.php"),
            false,
        )
        .unwrap();
    assert_eq!(marks(&files), ["other.php 0 -", "Cart.php 1 M"]);
    assert_eq!(files["focus"]["uses"][0]["changed"], true);
}
/// The boxes of a path that no drawn link leads to, and those that none
/// leads on from, the two ends apart.
fn loose(view: &Value) -> Vec<String> {
    let steps = view["path"]["steps"].as_u64().unwrap();
    let ends = |key: &str| {
        view["edges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|edge| edge[key].as_str().unwrap())
            .collect::<HashSet<_>>()
    };
    let (leads, led) = (ends("from"), ends("to"));
    view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|node| {
            let (path, step) = (
                node["path"].as_str().unwrap(),
                node["step"].as_u64().unwrap(),
            );
            (step > 0 && !led.contains(path)) || (step < steps && !leads.contains(path))
        })
        .map(|node| node["path"].as_str().unwrap().to_owned())
        .collect()
}
#[test]
fn a_path_with_more_links_than_are_drawn_leaves_no_box_without_its_way() {
    // Four layers of ten files between two files, each file of a layer
    // using every file of the next: more links than a picture draws.
    let layer = |at: usize| -> Vec<String> {
        match at {
            0 => vec!["a.py".to_owned()],
            5 => vec!["z.py".to_owned()],
            _ => (0..10).map(|file| format!("l{at}/f{file}.py")).collect(),
        }
    };
    let id = |path: &str| path.replace(['/', '.'], "_");
    let (mut nodes, mut links) = (Vec::new(), Vec::new());
    for at in 0..6 {
        for path in layer(at) {
            nodes.push(json!({"id": id(&path), "label": id(&path), "source_file": path}));
            if at == 5 {
                continue;
            }
            for next in layer(at + 1) {
                links.push(json!({"source": id(&path), "target": id(&next), "relation": "calls", "source_file": path}));
            }
        }
    }
    let index = self::index(json!({"nodes": nodes, "links": links}));
    let view = index
        .path("layers", "a.py", "z.py", &[], &|_| None, None, false)
        .unwrap();
    // Eight of each layer are drawn, with 8 + 3 × 64 + 8 links between
    // them, of which the picture draws its limit.
    assert_eq!(told_of(&view), "found steps=5 places=42 links=320 shown=34");
    assert_eq!(view["between"], 208);
    assert_eq!(view["edges"].as_array().unwrap().len(), DRAWN);
    // The links are alike, so their order is that of their files: the
    // last of it alone would leave the far end with nothing leading to
    // it. Each box keeps a link into it and a link out of it.
    assert_eq!(loose(&view), Vec::<String>::new());
    assert_eq!(
        calls(&view)
            .iter()
            .filter(|link| link.ends_with("-> z.py 1/0"))
            .count(),
        8
    );
}
#[test]
fn ways_longer_than_the_shortest_are_drawn_whole_when_they_are_asked_for() {
    let files = |uses: &[(&str, &str)]| {
        let mut names = uses
            .iter()
            .flat_map(|(from, to)| [*from, *to])
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        self::index(json!({
            "nodes": names.iter().map(|name| json!({
                "id": name, "label": name, "source_file": format!("{name}.py"),
            })).collect::<Vec<_>>(),
            "links": uses.iter().map(|(from, to)| json!({
                "source": from, "target": to, "relation": "calls",
                "source_file": format!("{from}.py"),
            })).collect::<Vec<_>>(),
        }))
    };
    // a reaches d in two links, through b and through c. Through e and
    // f it takes three, and so it does through b and then c. Through
    // g, h and i it takes four. j uses d and nothing leads to j.
    let index = files(&[
        ("a", "b"),
        ("b", "d"),
        ("a", "c"),
        ("c", "d"),
        ("a", "e"),
        ("e", "f"),
        ("f", "d"),
        ("b", "c"),
        ("a", "g"),
        ("g", "h"),
        ("h", "i"),
        ("i", "d"),
        ("j", "d"),
    ]);
    let ways = |from: &str, to: &str, focus: Option<&str>, longer: usize| {
        index
            .path_with("ways", from, to, &[], &|_| None, focus, false, longer)
            .unwrap()
    };
    let round = |view: &Value, key: &str| {
        view[key]
            .as_array()
            .unwrap()
            .iter()
            .filter(|part| part["detour"] == true)
            .count()
    };
    // The shortest ways alone: what was drawn before longer ways could
    // be asked for, and what `path` still answers.
    let shortest = ways("a.py", "d.py", None, 0);
    assert_eq!(told_of(&shortest), "found steps=2 places=4 links=4 shown=4");
    assert_eq!(
        calls(&shortest),
        [
            "a.py -> b.py 1/0",
            "a.py -> c.py 1/0",
            "b.py -> d.py 1/0",
            "c.py -> d.py 1/0"
        ]
    );
    assert_eq!(shortest["path"]["longer"], 0);
    assert_eq!(shortest["path"]["detours"], 0);
    assert_eq!(
        (round(&shortest, "nodes"), round(&shortest, "edges")),
        (0, 0)
    );
    assert_eq!(
        shortest,
        index
            .path("ways", "a.py", "d.py", &[], &|_| None, None, false)
            .unwrap()
    );
    // One link longer: the way through e and f, with its two places,
    // and the link from b to c, which joins two places already drawn.
    // The shortest ways are told and counted as before.
    let one = ways("a.py", "d.py", None, 1);
    assert_eq!(told_of(&one), "found steps=2 places=4 links=4 shown=4");
    assert_eq!(one["path"]["longer"], 1);
    assert_eq!(one["path"]["detours"], 2);
    assert_eq!(one["path"]["aside"], 2);
    assert_eq!(one["children"], 6);
    assert_eq!(one["path"]["detours_more"], false);
    assert_eq!(
        calls(&one),
        [
            "a.py -> b.py 1/0",
            "a.py -> c.py 1/0",
            "b.py -> d.py 1/0",
            "c.py -> d.py 1/0",
            "b.py -> c.py 1/0",
            "a.py -> e.py 1/0",
            "e.py -> f.py 1/0",
            "f.py -> d.py 1/0",
        ]
    );
    assert_eq!((round(&one, "nodes"), round(&one, "edges")), (2, 4));
    // A place of a way round is neither end of the path, though it
    // stands as many steps in as the far end.
    assert_eq!(
        boxes(&one),
        ["a.py first", "b.py", "c.py", "d.py second", "e.py", "f.py"]
    );
    // Every box lies on a way that is drawn whole.
    assert_eq!(loose(&one), Vec::<String>::new());
    // Two links longer adds the way of four links; no way is five long.
    let two = ways("a.py", "d.py", None, 2);
    assert_eq!(two["path"]["detours"], 3);
    assert_eq!(told_of(&two), "found steps=2 places=4 links=4 shown=4");
    assert_eq!(two["path"]["aside"], 5);
    assert!(calls(&two).contains(&"h.py -> i.py 1/0".to_owned()));
    let three = ways("a.py", "d.py", None, 3);
    assert_eq!(three["path"]["detours"], 3);
    assert_eq!(three["edges"], two["edges"]);
    // More than a picture may ask for is as much as it may.
    assert_eq!(ways("a.py", "d.py", None, 9)["path"]["longer"], LONGER);
    // Nothing that leads nowhere is drawn: j uses d, and no way from a
    // passes it.
    assert!(!boxes(&three).iter().any(|name| name.starts_with("j.py")));
    // Beside a selected box stand all the links drawn at it: c is led
    // to from a on a shortest way and from b on a way round.
    let paths = |rows: &Value| {
        rows.as_array()
            .unwrap()
            .iter()
            .map(|row| row["path"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let beside = ways("a.py", "d.py", Some("c.py"), 1);
    assert_eq!(paths(&beside["focus"]["uses"]), ["d.py"]);
    assert_eq!(paths(&beside["focus"]["used_by"]), ["a.py", "b.py"]);
    let beside = ways("a.py", "d.py", Some("c.py"), 0);
    assert_eq!(paths(&beside["focus"]["used_by"]), ["a.py"]);
    // Asked the other way round, the ways are those that exist, and
    // their longer ones with them.
    let turned = ways("d.py", "a.py", None, 1);
    assert_eq!(
        told_of(&turned),
        "found reversed steps=2 places=4 links=4 shown=4"
    );
    assert_eq!(turned["path"]["detours"], 2);
    // Where nothing leads either way there is nothing longer either.
    let none = ways("j.py", "a.py", None, 2);
    assert_eq!(told_of(&none), "none steps=0 places=0 links=0 shown=0");
    assert_eq!(none["path"]["detours"], 0);
    // Twenty ways of three links beside one of two: as many are drawn
    // as a picture holds, each whole, and the answer says there are
    // more.
    let mut uses = vec![
        ("a".to_owned(), "z".to_owned()),
        ("z".to_owned(), "d".to_owned()),
    ];
    for at in 0..20 {
        uses.push(("a".to_owned(), format!("m{at:02}")));
        uses.push((format!("m{at:02}"), format!("n{at:02}")));
        uses.push((format!("n{at:02}"), "d".to_owned()));
    }
    let borrowed = uses
        .iter()
        .map(|(from, to)| (from.as_str(), to.as_str()))
        .collect::<Vec<_>>();
    let index = files(&borrowed);
    let many = index
        .path_with("ways", "a.py", "d.py", &[], &|_| None, None, false, 1)
        .unwrap();
    assert_eq!(many["path"]["detours"], DETOURS);
    assert_eq!(many["path"]["detours_more"], true);
    assert_eq!(round(&many, "nodes"), 2 * DETOURS);
    assert_eq!(round(&many, "edges"), 3 * DETOURS);
    assert_eq!(loose(&many), Vec::<String>::new());
    // The first of them in the order of their files.
    assert!(calls(&many).contains(&"m00.py -> n00.py 1/0".to_owned()));
    assert!(!calls(&many).contains(&"m19.py -> n19.py 1/0".to_owned()));
}
/// Graphs made at random, and every path asked between their places
/// checked against distances worked out apart: the fewest links, the way
/// that exists, every place and link on such ways counted, and a picture
/// in which each box stands at its step and lies on a way drawn whole.
#[test]
fn a_path_on_a_graph_made_at_random_is_as_short_as_its_links_allow() {
    const FAR: usize = usize::MAX;
    let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
    let mut below = move |limit: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % limit as u64) as usize
    };
    // How far every place lies from every other along the links.
    let apart = |uses: &[Vec<bool>]| {
        let count = uses.len();
        let mut far = vec![vec![FAR; count]; count];
        for (user, row) in uses.iter().enumerate() {
            far[user][user] = 0;
            for (used, linked) in row.iter().enumerate() {
                if *linked && user != used {
                    far[user][used] = 1;
                }
            }
        }
        for through in 0..count {
            let onward = far[through].clone();
            for row in &mut far {
                let first = row[through];
                if first == FAR {
                    continue;
                }
                for (to, second) in onward.iter().enumerate() {
                    if *second != FAR && first + second < row[to] {
                        row[to] = first + second;
                    }
                }
            }
        }
        far
    };
    // One path against the distances: `starts` and `ends` are what its
    // two ends stand for, and `ids` tells a drawn place by its name.
    let check = |view: &Value,
                 starts: &[usize],
                 ends: &[usize],
                 uses: &[Vec<bool>],
                 far: &[Vec<usize>],
                 ids: &HashMap<String, usize>,
                 what: &str| {
        let path = &view["path"];
        let within = starts.iter().any(|place| ends.contains(place));
        assert_eq!(path["within"], within, "{what}");
        let nearest = |from: &[usize], to: &[usize]| {
            from.iter()
                .flat_map(|a| to.iter().map(move |b| far[*a][*b]))
                .min()
                .unwrap()
        };
        let (forward, back) = (nearest(starts, ends), nearest(ends, starts));
        if within || (forward == FAR && back == FAR) {
            assert_eq!(path["found"], false, "{what}");
            assert_eq!(view["nodes"], json!([]), "{what}");
            assert_eq!(view["edges"], json!([]), "{what}");
            return;
        }
        let reversed = forward == FAR;
        let (first, last, steps) = if reversed {
            (ends, starts, back)
        } else {
            (starts, ends, forward)
        };
        assert_eq!(
            (&path["found"], &path["reversed"], &path["steps"]),
            (&json!(true), &json!(reversed), &json!(steps)),
            "{what}"
        );
        let from = |place: usize| first.iter().map(|a| far[*a][place]).min().unwrap();
        let to = |place: usize| last.iter().map(|b| far[place][*b]).min().unwrap();
        let on = |place: usize| {
            from(place) != FAR && to(place) != FAR && from(place) + to(place) == steps
        };
        let count = uses.len();
        let places = (0..count).filter(|place| on(*place)).count();
        let links = (0..count)
            .flat_map(|user| (0..count).map(move |used| (user, used)))
            .filter(|(user, used)| {
                uses[*user][*used]
                    && from(*user) != FAR
                    && to(*used) != FAR
                    && from(*user) + 1 + to(*used) == steps
            })
            .count();
        assert_eq!(
            (&path["places"], &path["links"]),
            (&json!(places), &json!(links)),
            "{what}"
        );
        // Each box stands at its step of a shortest way, and the ends
        // are the first step and the last.
        let mut drawn = HashMap::new();
        for node in view["nodes"].as_array().unwrap() {
            let place = ids[node["path"].as_str().unwrap()];
            let step = node["step"].as_u64().unwrap() as usize;
            assert!(on(place) && from(place) == step, "{what}: {node}");
            assert_eq!(
                (&node["start"], &node["end"]),
                (&json!(step == 0), &json!(step == steps)),
                "{what}"
            );
            assert!(drawn.insert(place, step).is_none(), "{what}: {node}");
        }
        assert_eq!(path["shown"], json!(drawn.len()), "{what}");
        let wide = WAY.min(FOLLOWED / (steps + 1)).max(1);
        assert_eq!(path["wide"], json!(wide), "{what}");
        let mut whole = true;
        for step in 0..=steps {
            let here = drawn.values().filter(|at| **at == step).count();
            let all = (0..count)
                .filter(|place| on(*place) && from(*place) == step)
                .count();
            assert!((1..=wide.min(all)).contains(&here), "{what}: step {step}");
            whole &= all <= wide;
        }
        // Where every step has room for all of its places, all are drawn.
        if whole {
            assert_eq!(drawn.len(), places, "{what}");
        }
        // Each link drawn is a link, from a box to one a step further.
        for edge in view["edges"].as_array().unwrap() {
            let (user, used) = (
                ids[edge["from"].as_str().unwrap()],
                ids[edge["to"].as_str().unwrap()],
            );
            assert!(uses[user][used], "{what}: {edge}");
            assert_eq!(drawn[&user] + 1, drawn[&used], "{what}: {edge}");
        }
        assert_eq!(loose(view), Vec::<String>::new(), "{what}");
    };
    let (mut found, mut turned, mut none, mut capped) = (0, 0, 0, 0);
    for round in 0..60 {
        // A few folders of files, some names in each file, and links
        // between the names wherever they fall.
        let count = 6 + below(40);
        let files = 2 + below(count.min(16) - 1);
        let folders = 1 + below(4);
        let file_of = |name: usize| name % files;
        let path_of = |file: usize| format!("d{}/f{file:02}.py", file % folders);
        let nodes = (0..count)
                .map(|name| json!({"id": format!("n{name}"), "label": format!("n{name}"), "source_file": path_of(file_of(name))}))
                .collect::<Vec<_>>();
        let mut pairs = (0..below(count * 3 + 1))
            .map(|_| (below(count), below(count)))
            .collect::<Vec<_>>();
        // Every third graph has a name that many use and that uses
        // many, and every third one two names with many ways of two
        // links between them: more places on the ways than a step shows.
        let (hub, beyond) = (below(count), below(count));
        match round % 3 {
            0 => {
                for _ in 0..30 {
                    pairs.push((below(count), hub));
                    pairs.push((hub, below(count)));
                }
            }
            1 => {
                for _ in 0..16 {
                    let between = below(count);
                    pairs.push((hub, between));
                    pairs.push((between, beyond));
                }
            }
            _ => {}
        }
        let mut links = Vec::new();
        let mut uses = vec![vec![false; count]; count];
        let mut file_uses = vec![vec![false; files]; files];
        for (user, used) in pairs {
            if user == used {
                continue;
            }
            uses[user][used] = true;
            if file_of(user) != file_of(used) {
                file_uses[file_of(user)][file_of(used)] = true;
            }
            links.push(json!({"source": format!("n{user}"), "target": format!("n{used}"), "relation": "calls", "source_file": path_of(file_of(user))}));
        }
        let index = self::index(json!({"nodes": nodes, "links": links}));
        // Each name as a picture selects it, and each file by its path.
        let mut keys = vec![String::new(); count];
        for file in 0..index.files.len() as u32 {
            for (key, at) in index.keys(file).into_iter().zip(index.range(file)) {
                let name: usize = index.label(at)[1..].parse().unwrap();
                keys[name] = key;
            }
        }
        let names = keys
            .iter()
            .enumerate()
            .map(|(name, key)| (key.clone(), name))
            .collect::<HashMap<_, _>>();
        let paths = (0..files)
            .map(|file| (path_of(file), file))
            .collect::<HashMap<_, _>>();
        let (far, file_far) = (apart(&uses), apart(&file_uses));
        // What an end stands for among files, and among names.
        let in_folder = |folder: usize| {
            (0..files)
                .filter(|file| file % folders == folder)
                .collect::<Vec<_>>()
        };
        let names_of = |held: &[usize]| {
            (0..count)
                .filter(|name| held.contains(&file_of(*name)))
                .collect::<Vec<_>>()
        };
        for turn in 0..40 {
            // The two names that were given many ways are asked first.
            let (a, b) = if turn == 0 {
                (hub, beyond)
            } else {
                (below(count), below(count))
            };
            let (file_a, file_b) = (file_of(a), file_of(b));
            let (folder_a, folder_b) = (file_a % folders, file_b % folders);
            let asked = [
                // Among files: two files, a folder and a file, two folders.
                (
                    path_of(file_a),
                    path_of(file_b),
                    vec![file_a],
                    vec![file_b],
                    false,
                ),
                (
                    format!("d{folder_a}"),
                    path_of(file_b),
                    in_folder(folder_a),
                    vec![file_b],
                    false,
                ),
                (
                    format!("d{folder_a}"),
                    format!("d{folder_b}"),
                    in_folder(folder_a),
                    in_folder(folder_b),
                    false,
                ),
                // Among names: two names, a name and a file, a folder and a name.
                (keys[a].clone(), keys[b].clone(), vec![a], vec![b], true),
                (
                    keys[a].clone(),
                    path_of(file_b),
                    vec![a],
                    names_of(&[file_b]),
                    true,
                ),
                (
                    format!("d{folder_a}"),
                    keys[b].clone(),
                    names_of(&in_folder(folder_a)),
                    vec![b],
                    true,
                ),
            ];
            for (from, to, starts, ends, named) in &asked {
                let what = format!("round {round}: {from} → {to}");
                let view = index
                    .path("random", from, to, &[], &|_| None, None, false)
                    .unwrap_or_else(|| panic!("{what}"));
                assert_eq!(view["path"]["names"], *named, "{what}");
                if *named {
                    check(&view, starts, ends, &uses, &far, &names, &what);
                } else {
                    check(&view, starts, ends, &file_uses, &file_far, &paths, &what);
                }
                let path = &view["path"];
                match (path["found"] == true, path["reversed"] == true) {
                    (true, false) => found += 1,
                    (true, true) => turned += 1,
                    _ => none += 1,
                }
                if path["shown"].as_u64() < path["places"].as_u64() {
                    capped += 1;
                }
            }
        }
    }
    // The graphs are of every sort: ways forward, ways back, none at
    // all, and more places on the ways than a step shows.
    assert!(
        found > 5000 && turned > 500 && none > 4000 && capped > 30,
        "{found} {turned} {none} {capped}"
    );
}
/// Two places asked what they share, from the nearest on.
fn asking<'a>(from: &'a str, to: &'a str, users: Option<bool>) -> Sharing<'a> {
    Sharing {
        from,
        to,
        users,
        skip: 0,
    }
}
/// What two places are said to share: which kind is drawn and how many
/// places of it, then how many places both use and how many use both;
/// and, where it is so, after how many the picture begins and how many
/// of the kind a filter left.
fn shared_of(view: &Value) -> String {
    let shared = &view["shared"];
    let users = shared["side"] == "uses_both";
    let mut told = format!(
        "{}{} drawn of what {} · both use {} · {} use both",
        if shared["within"] == true {
            "within · "
        } else {
            ""
        },
        shared["shown"],
        if users { "uses both" } else { "both use" },
        shared["both_use"],
        shared["uses_both"],
    );
    if shared["skip"] != 0 {
        told.push_str(&format!(" · after {}", shared["skip"]));
    }
    if shared["matching"] != shared[if users { "uses_both" } else { "both_use" }] {
        told.push_str(&format!(" · {} match", shared["matching"]));
    }
    told
}
/// The boxes of what two places share: which of the two a box is, and
/// for what they share how many links each of the two is away.
fn boxes(view: &Value) -> Vec<String> {
    view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| {
            format!(
                "{}{}{}{}{}",
                node["name"].as_str().unwrap(),
                if node["folder"] == true { "/" } else { "" },
                if node["start"] == true { " first" } else { "" },
                if node["end"] == true { " second" } else { "" },
                match node["shares"].as_array() {
                    Some(far) => format!(" shared {}+{}", far[0], far[1]),
                    None => String::new(),
                },
            )
        })
        .collect()
}
#[test]
fn two_files_share_what_both_use_and_what_uses_both() {
    let index = office(3);
    let shared = |from: &str, to: &str, users: Option<bool>, focus: Option<&str>, certain: bool| {
        index
            .shared(
                "office",
                asking(from, to, users),
                &[],
                &|_| None,
                focus,
                certain,
                "",
            )
            .unwrap()
    };
    let (users, orders) = ("app/api/users.php", "app/api/orders.php");
    // The two files of the API both use the core, the database and the
    // cache, and the entry point uses both. What both use is drawn where
    // nothing else is asked: the database is one link from the orders,
    // which was guessed, and two from the users.
    let api = shared(users, orders, None, None, false);
    assert_eq!(api["mode"], "shared");
    assert_eq!(
        shared_of(&api),
        "3 drawn of what both use · both use 3 · 1 use both"
    );
    assert_eq!(
        boxes(&api),
        [
            "users.php first",
            "orders.php second",
            "core.php shared 1+1",
            "db.php shared 2+1",
            "store.php shared 2+2",
        ]
    );
    // Each is drawn with one shortest way from either file: the users
    // reach the database through the core, and both reach the cache so.
    assert_eq!(
        sorted(calls(&api)),
        [
            "core.php -> db.php 2/0",
            "core.php -> store.php 1/0",
            "orders.php -> core.php 1/0",
            "orders.php -> db.php 1/1",
            "users.php -> core.php 1/0",
        ]
    );
    assert_eq!(api["between"], 5);
    assert_eq!(shared(users, orders, Some(false), None, false), api);
    // The two are told as they were asked for, and the way back leads to
    // where the first of them lies.
    assert_eq!(
        api["shared"]["from"],
        json!({"path": users, "name": "users.php", "what": "file"})
    );
    assert_eq!(api["shared"]["to"]["name"], "orders.php");
    assert_eq!(
        (&api["shared"]["names"], &api["shared"]["wide"]),
        (&json!(false), &json!(SHARED))
    );
    assert_eq!(api["scope"], "app/api");
    assert_eq!(
        api["crumbs"],
        json!([
            {"name": "office", "path": ""},
            {"name": "app", "path": "app"},
            {"name": "api", "path": "app/api"},
            {"name": "users.php and orders.php", "path": ""}
        ])
    );
    // What uses both is drawn where it is asked for: the links run from
    // it to the two.
    let above = shared(users, orders, Some(true), None, false);
    assert_eq!(
        shared_of(&above),
        "1 drawn of what uses both · both use 3 · 1 use both"
    );
    assert_eq!(
        boxes(&above),
        [
            "users.php first",
            "orders.php second",
            "main.php shared 1+1"
        ]
    );
    assert_eq!(
        sorted(calls(&above)),
        ["main.php -> orders.php 1/0", "main.php -> users.php 1/0"]
    );
    // Without what the tool guessed the orders reach the database
    // through the core as well, and it stands before the cache, being
    // the more linked of two that are as near.
    let read = shared(users, orders, None, None, true);
    assert_eq!(
        boxes(&read)[2..],
        [
            "core.php shared 1+1",
            "db.php shared 2+2",
            "store.php shared 2+2",
        ]
    );
    assert_eq!(calls(&read).len(), 4);
    // A box lists what it leads to on the ways drawn and what leads to
    // it, and how it uses the next.
    let chosen = shared(users, orders, None, Some("lib/core.php"), false);
    let focus = &chosen["focus"];
    assert_eq!(
        (&focus["path"], &focus["shares"]),
        (&json!("lib/core.php"), &json!([1, 1]))
    );
    assert_eq!(
        focus["shared_edges"],
        json!([[users, "lib/core.php"], [orders, "lib/core.php"]])
    );
    assert_eq!(
        shared(users, orders, None, Some("lib/db.php"), false)["focus"]["shared_edges"],
        json!([
            [users, "lib/core.php"],
            [orders, "lib/db.php"],
            ["lib/core.php", "lib/db.php"]
        ])
    );
    let cache = "lib/deep/cache/store.php";
    assert_eq!(
        shared(users, orders, None, Some(cache), false)["focus"]["shared_edges"],
        json!([
            [users, "lib/core.php"],
            [orders, "lib/core.php"],
            ["lib/core.php", cache]
        ])
    );
    assert_eq!(
        shared(users, orders, Some(true), Some("app/main.php"), false)["focus"]["shared_edges"],
        json!([["app/main.php", users], ["app/main.php", orders]])
    );
    assert!(shared(users, orders, None, Some(users), false)["focus"]["shared_edges"].is_null());
    let rows = |focus: &Value, side: &str| {
        focus[side]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                format!(
                    "{}{} {}{}",
                    row["name"].as_str().unwrap(),
                    if row["folder"] == true { "/" } else { "" },
                    row["links"],
                    if row["inside"] == true { " drawn" } else { "" }
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(rows(focus, "uses"), ["db.php 2 drawn", "store.php 1 drawn"]);
    assert_eq!(
        rows(focus, "used_by"),
        ["orders.php 1 drawn", "users.php 1 drawn"]
    );
    assert_eq!(
        (&focus["uses_more"], &focus["used_by_more"]),
        (&json!(0), &json!(0))
    );
    assert_eq!(focus["kinds"], json!([{"kind": "calls", "links": 3}]));
    // So does one of the two: nothing leads to it where what both use
    // is drawn, and nothing on from it where what uses both is.
    let first = shared(users, orders, None, Some(users), false);
    assert_eq!(
        (&first["focus"]["start"], &first["focus"]["shares"]),
        (&json!(true), &Value::Null)
    );
    assert_eq!(rows(&first["focus"], "uses"), ["core.php 1 drawn"]);
    assert_eq!(rows(&first["focus"], "used_by"), Vec::<String>::new());
    let second = shared(users, orders, Some(true), Some(orders), false);
    assert_eq!(rows(&second["focus"], "uses"), Vec::<String>::new());
    assert_eq!(rows(&second["focus"], "used_by"), ["main.php 1 drawn"]);
    // What one of the two reaches only through the other is not shared:
    // the entry point reaches the cache through the core alone, and the
    // database through the orders too.
    let through = shared("app/main.php", "lib/core.php", None, None, false);
    assert_eq!(
        shared_of(&through),
        "1 drawn of what both use · both use 1 · 0 use both"
    );
    assert_eq!(
        boxes(&through),
        [
            "main.php first",
            "core.php second",
            "orders.php",
            "db.php shared 2+1",
        ]
    );
    assert_eq!(
        sorted(calls(&through)),
        [
            "core.php -> db.php 2/0",
            "main.php -> orders.php 1/0",
            "orders.php -> db.php 1/1",
        ]
    );
    // Without that guess the two share nothing, and nothing is drawn.
    let nothing = shared("app/main.php", "lib/core.php", None, None, true);
    assert_eq!(
        shared_of(&nothing),
        "0 drawn of what both use · both use 0 · 0 use both"
    );
    assert_eq!(
        (&nothing["nodes"], &nothing["edges"], &nothing["mode"]),
        (&json!([]), &json!([]), &json!("shared"))
    );
    // Where the two use nothing in common and something uses both, that
    // is what is drawn; what both use is still answered where it is
    // asked for, with nothing to draw.
    let (db, cache) = ("lib/db.php", "lib/deep/cache/store.php");
    let leaves = shared(db, cache, None, None, false);
    assert_eq!(
        shared_of(&leaves),
        "4 drawn of what uses both · both use 0 · 4 use both"
    );
    assert_eq!(
        boxes(&leaves),
        [
            "db.php first",
            "store.php second",
            "orders.php shared 1+2",
            "users.php shared 2+2",
            "main.php shared 2+3",
            "core.php shared 1+1",
        ]
    );
    assert_eq!(
        sorted(calls(&leaves)),
        [
            "core.php -> db.php 2/0",
            "core.php -> store.php 1/0",
            "main.php -> orders.php 1/0",
            "orders.php -> core.php 1/0",
            "orders.php -> db.php 1/1",
            "users.php -> core.php 1/0",
        ]
    );
    let asked = shared(db, cache, Some(false), None, false);
    assert_eq!(
        shared_of(&asked),
        "0 drawn of what both use · both use 0 · 4 use both"
    );
    assert_eq!(asked["nodes"], json!([]));
    // A place the graph does not know shares nothing at all.
    assert!(
        index
            .shared(
                "office",
                asking(users, "lib/gone.php", None),
                &[],
                &|_| None,
                None,
                false,
                ""
            )
            .is_none()
    );
}
#[test]
fn a_folder_that_shares_is_one_box_with_all_its_links() {
    let index = office(3);
    let shared = |from: &str, to: &str, focus: Option<&str>| {
        index
            .shared(
                "office",
                asking(from, to, None),
                &[],
                &|_| None,
                focus,
                false,
                "",
            )
            .unwrap()
    };
    // The API and the reports both use the database: the API through
    // one link that was guessed, the reports through one from each of
    // their three files. Only the API has something that uses it.
    let folders = shared("app/api", "reports/", Some("reports"));
    assert_eq!(
        shared_of(&folders),
        "1 drawn of what both use · both use 1 · 0 use both"
    );
    assert_eq!(
        boxes(&folders),
        ["api/ first", "reports/ second", "db.php shared 1+1"]
    );
    assert_eq!(
        sorted(calls(&folders)),
        ["api -> db.php 1/1", "reports -> db.php 3/0"]
    );
    let api = &folders["nodes"][0];
    assert_eq!(
        (
            &api["path"],
            &api["files"],
            &api["symbols"],
            &api["out"],
            &api["in"]
        ),
        (
            &json!("app/api"),
            &json!(2),
            &json!(2),
            &json!(3),
            &json!(2)
        )
    );
    assert_eq!(
        folders["shared"]["to"],
        json!({"path": "reports", "name": "reports", "what": "folder"})
    );
    assert_eq!(folders["scope"], "app");
    // A folder that is selected lists its links as one, and says how
    // its files use what it leads to.
    let focus = &folders["focus"];
    assert_eq!(
        (&focus["path"], &focus["folder"], &focus["end"]),
        (&json!("reports"), &json!(true), &json!(true))
    );
    assert_eq!(
        (&focus["uses"][0]["path"], &focus["uses"][0]["links"]),
        (&json!("lib/db.php"), &json!(3))
    );
    assert_eq!(focus["kinds"], json!([{"kind": "calls", "links": 3}]));
    // And it is a row beside what it leads to.
    let used = shared("app/api", "reports", Some("lib/db.php"));
    let rows = used["focus"]["used_by"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            format!(
                "{} {} {}/{}",
                row["path"].as_str().unwrap(),
                row["folder"],
                row["links"],
                row["guessed"]
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(rows, ["reports true 3/0", "app/api true 1/1"]);
    // A folder that holds the entry point as well: what the API uses is
    // reached from inside it, and the database alone is shared with the
    // reports.
    let whole = shared("app", "reports", None);
    assert_eq!(
        boxes(&whole),
        ["app/ first", "reports/ second", "db.php shared 1+1"]
    );
    // What lies in the other is not apart from it, nor is a place from
    // itself.
    let inside = shared("lib", "lib/db.php", None);
    assert_eq!(
        shared_of(&inside),
        "within · 0 drawn of what both use · both use 0 · 0 use both"
    );
    assert_eq!(inside["nodes"], json!([]));
    assert_eq!(
        shared_of(&shared("lib/db.php", "lib/db.php", None)),
        shared_of(&inside)
    );
}
#[test]
fn what_two_symbols_share_is_told_among_symbols() {
    let index = store();
    let shared = |from: &str, to: &str, users: Option<bool>, focus: Option<&str>, certain: bool| {
        index
            .shared(
                "shop",
                asking(from, to, users),
                &[],
                &|_| None,
                focus,
                certain,
                "",
            )
            .unwrap()
    };
    let format = "src/Cart/Cart.php#L29:format_price()";
    let paid = "src/Order/Notifier.php#L11:.paid()";
    // The total and the formatting both call the adding of a price, the
    // formatting by a guess.
    let both = shared(TOTAL_KEY, format, None, None, false);
    assert_eq!(
        shared_of(&both),
        "1 drawn of what both use · both use 1 · 1 use both"
    );
    assert_eq!(both["shared"]["names"], true);
    assert_eq!(
        boxes(&both),
        [
            "Cart.total() first",
            "format_price() second",
            "Price.plus() shared 1+1",
        ]
    );
    assert_eq!(
        sorted(calls(&both)),
        [
            "Cart.total() -> Price.plus() 1/0",
            "format_price() -> Price.plus() 1/1",
        ]
    );
    // Each is a symbol of its own file, and the two say what they are.
    let plus = &both["nodes"][2];
    assert_eq!(
        (&plus["symbol"], &plus["kind"], &plus["file"], &plus["line"]),
        (
            &json!(true),
            &json!("method"),
            &json!("src/Cart/Price.php"),
            &json!(10)
        )
    );
    assert_eq!(both["shared"]["from"]["what"], "method");
    assert_eq!(both["scope"], CART);
    assert_eq!(
        both["crumbs"].as_array().unwrap().last().unwrap()["name"],
        "Cart.total() and format_price()"
    );
    // Checkout uses both: the total itself, and the formatting through
    // the notifier, which it is only guessed to call.
    let above = shared(TOTAL_KEY, format, Some(true), Some(paid), false);
    assert_eq!(
        boxes(&above),
        [
            "Cart.total() first",
            "format_price() second",
            "Cart.checkout() shared 1+2",
            "Notifier.paid()",
        ]
    );
    assert_eq!(
        sorted(calls(&above)),
        [
            "Cart.checkout() -> Cart.total() 1/0",
            "Cart.checkout() -> Notifier.paid() 1/1",
            "Notifier.paid() -> format_price() 1/0",
        ]
    );
    // A box on the way lists the next and the one before, each with how
    // the two are linked.
    let focus = &above["focus"];
    assert_eq!(focus["shares"], Value::Null);
    assert_eq!(
        (
            &focus["uses"][0]["name"],
            &focus["uses"][0]["relation"],
            &focus["uses"][0]["guessed"]
        ),
        (&json!("format_price()"), &json!("calls"), &json!(0))
    );
    assert_eq!(
        (
            &focus["used_by"][0]["path"],
            &focus["used_by"][0]["guessed"]
        ),
        (&json!(CHECKOUT_KEY), &json!(1))
    );
    assert_eq!(focus["kinds"], json!([{"kind": "calls", "links": 1}]));
    // Without what the tool guessed nothing is left of either kind.
    let read = shared(TOTAL_KEY, format, None, None, true);
    assert_eq!(
        shared_of(&read),
        "0 drawn of what both use · both use 0 · 0 use both"
    );
    // A file stands for every symbol in it, and is one box among the
    // symbols: the notifier reaches the adding through the formatting.
    let file = shared(TOTAL_KEY, "src/Order/Notifier.php", None, None, false);
    assert_eq!(
        boxes(&file),
        [
            "Cart.total() first",
            "Notifier.php second",
            "format_price()",
            "Price.plus() shared 1+2",
        ]
    );
    assert_eq!(
        sorted(calls(&file)),
        [
            "Cart.total() -> Price.plus() 1/0",
            "Notifier.php -> format_price() 1/0",
            "format_price() -> Price.plus() 1/1",
        ]
    );
    let notifier = &file["nodes"][1];
    assert_eq!(
        (
            &notifier["path"],
            &notifier["symbol"],
            &notifier["folder"],
            &notifier["symbols"]
        ),
        (
            &json!("src/Order/Notifier.php"),
            &Value::Null,
            &json!(false),
            &json!(2)
        )
    );
    assert_eq!(file["shared"]["to"]["what"], "file");
    // Checkout uses the total and something of the notifier.
    let users = shared(TOTAL_KEY, "src/Order/Notifier.php", Some(true), None, false);
    assert_eq!(
        sorted(calls(&users)),
        [
            "Cart.checkout() -> Cart.total() 1/0",
            "Cart.checkout() -> Notifier.php 1/1",
        ]
    );
    // A symbol of the file at the other end lies in it.
    let own = shared(TOTAL_KEY, CART, None, None, false);
    assert_eq!(own["shared"]["within"], true);
    // A filter is held against a symbol's name as it is shown, with
    // its class, and not against the path of its file.
    let named = |filter: &str| {
        index
            .shared(
                "shop",
                asking(TOTAL_KEY, format, None),
                &[],
                &|_| None,
                None,
                false,
                filter,
            )
            .unwrap()
    };
    assert_eq!(
        shared_of(&named("PRICE.PL")),
        "1 drawn of what both use · both use 1 · 1 use both"
    );
    assert_eq!(
        shared_of(&named("src/cart")),
        "0 drawn of what both use · both use 1 · 1 use both · 0 match"
    );
}
#[test]
fn what_two_places_share_is_shown_nearest_first_as_far_as_a_picture_has_room() {
    // Two files that both use fifteen others, the later ones through
    // more links, and that five others use.
    let node = |id: &str, file: &str| json!({"id": id, "label": id, "source_file": file});
    let call = |from: &str, to: &str, file: &str| json!({"source": from, "target": to, "relation": "calls", "source_file": file});
    let (mut nodes, mut links) = (
        vec![node("a", "src/a.py"), node("b", "src/b.py")],
        Vec::new(),
    );
    for at in 0..15 {
        let (id, file) = (format!("u{at:02}"), format!("lib/u{at:02}.py"));
        nodes.push(node(&id, &file));
        for copy in 0..=at / 5 {
            // More names of a file that use the same one are more links.
            let (a, b) = (format!("a{at}_{copy}"), format!("b{at}_{copy}"));
            nodes.push(node(&a, "src/a.py"));
            nodes.push(node(&b, "src/b.py"));
            links.push(call(&a, &id, "src/a.py"));
            links.push(call(&b, &id, "src/b.py"));
        }
    }
    for at in 0..5 {
        let (id, file) = (format!("t{at}"), format!("top/t{at}.py"));
        nodes.push(node(&id, &file));
        links.push(call(&id, "a", &file));
        links.push(call(&id, "b", &file));
    }
    let wide = self::index(json!({"nodes": nodes, "links": links}));
    let part = |users: Option<bool>, skip: usize, filter: &str| {
        let asked = Sharing {
            skip,
            ..asking("src/a.py", "src/b.py", users)
        };
        wide.shared("wide", asked, &[], &|_| None, None, false, filter)
            .unwrap()
    };
    let shared = |users: Option<bool>| part(users, 0, "");
    // The names of what a picture shows as shared, as they are drawn:
    // by their paths.
    let drawn = |view: &Value| {
        boxes(view)[2..]
            .iter()
            .map(|name| name[..3].to_owned())
            .collect::<Vec<_>>()
    };
    // All fifteen are one link from either file: the most linked are
    // drawn, by their paths where they are linked alike.
    let view = shared(None);
    assert_eq!(
        shared_of(&view),
        format!("{SHARED} drawn of what both use · both use 15 · 5 use both")
    );
    let used = boxes(&view)[2..].to_vec();
    // Three links from each file lead to the last five and two to the
    // five before them; of the first five, with one, two have room.
    assert_eq!(
        used.iter().map(|name| &name[..3]).collect::<Vec<_>>(),
        [
            "u00", "u01", "u05", "u06", "u07", "u08", "u09", "u10", "u11", "u12", "u13", "u14"
        ]
    );
    assert!(used.iter().all(|name| name.ends_with(".py shared 1+1")));
    assert_eq!(calls(&view).len(), 2 * SHARED);
    assert!(calls(&view).contains(&"a.py -> u14.py 3/0".to_owned()));
    // What uses both is another picture, with all five of it.
    let above = shared(Some(true));
    assert_eq!(
        shared_of(&above),
        "5 drawn of what uses both · both use 15 · 5 use both"
    );
    assert_eq!(calls(&above).len(), 10);
    // The picture after the first begins where that one ended, and
    // shows the three it had no room for.
    let next = part(None, SHARED, "");
    assert_eq!(
        shared_of(&next),
        "3 drawn of what both use · both use 15 · 5 use both · after 12"
    );
    assert_eq!(drawn(&next), ["u02", "u03", "u04"]);
    assert_eq!(calls(&next).len(), 6);
    // A picture may begin anywhere in the order of nearness: after the
    // five most linked come the five with two links, then five with one.
    let later = part(None, 5, "");
    assert_eq!(
        shared_of(&later),
        "10 drawn of what both use · both use 15 · 5 use both · after 5"
    );
    assert_eq!(
        drawn(&later),
        [
            "u00", "u01", "u02", "u03", "u04", "u05", "u06", "u07", "u08", "u09"
        ]
    );
    // A beginning past the last place is the last place.
    let past = part(None, 100, "");
    assert_eq!(
        shared_of(&past),
        "1 drawn of what both use · both use 15 · 5 use both · after 14"
    );
    assert_eq!(drawn(&past), ["u04"]);
    // A filter keeps the places whose path holds it, whatever the case
    // and the blanks around it; both kinds are counted as they were.
    let some = part(None, 0, " U1 ");
    assert_eq!(
        shared_of(&some),
        "5 drawn of what both use · both use 15 · 5 use both · 5 match"
    );
    assert_eq!(drawn(&some), ["u10", "u11", "u12", "u13", "u14"]);
    let folder = part(None, 0, "lib/u");
    assert_eq!(folder["shared"]["matching"], 15);
    assert_eq!(drawn(&folder), drawn(&view));
    // What a filter left is passed over like the rest: the ten with a
    // nought in their names, in their order of nearness.
    let tail = part(None, 8, "u0");
    assert_eq!(
        shared_of(&tail),
        "2 drawn of what both use · both use 15 · 5 use both · after 8 · 10 match"
    );
    assert_eq!(drawn(&tail), ["u03", "u04"]);
    // A filter that nothing matches draws nothing, and does not change
    // which kind is drawn where none was asked for.
    let none = part(None, 0, "top/");
    assert_eq!(
        shared_of(&none),
        "0 drawn of what both use · both use 15 · 5 use both · 0 match"
    );
    assert_eq!(
        (&none["nodes"], &none["edges"], &none["shared"]["skip"]),
        (&json!([]), &json!([]), &json!(0))
    );
    let users = part(Some(true), 3, "top/");
    assert_eq!(
        shared_of(&users),
        "2 drawn of what uses both · both use 15 · 5 use both · after 3"
    );
    // Where several ways to a place are as short, the strongest is the
    // one drawn, and the first by its path where they are as strong.
    let ways = |twice: bool| {
        let mut nodes = vec![
            node("a", "src/a.py"),
            node("b", "src/b.py"),
            node("m1", "mid/m1.py"),
            node("m2", "mid/m2.py"),
            node("x", "lib/x.py"),
        ];
        let mut links = vec![
            call("a", "m1", "src/a.py"),
            call("a", "m2", "src/a.py"),
            call("m1", "x", "mid/m1.py"),
            call("m2", "x", "mid/m2.py"),
            call("b", "x", "src/b.py"),
        ];
        if twice {
            nodes.push(node("m2b", "mid/m2.py"));
            links.push(call("m2b", "x", "mid/m2.py"));
        }
        let view = self::index(json!({"nodes": nodes, "links": links}))
            .shared(
                "ways",
                asking("src/a.py", "src/b.py", None),
                &[],
                &|_| None,
                None,
                false,
                "",
            )
            .unwrap();
        assert_eq!(
            shared_of(&view),
            "1 drawn of what both use · both use 1 · 0 use both"
        );
        (boxes(&view), sorted(calls(&view)))
    };
    assert_eq!(
        ways(false),
        (
            vec![
                "a.py first".to_owned(),
                "b.py second".to_owned(),
                "x.py shared 2+1".to_owned(),
                "m1.py".to_owned(),
            ],
            vec![
                "a.py -> m1.py 1/0".to_owned(),
                "b.py -> x.py 1/0".to_owned(),
                "m1.py -> x.py 1/0".to_owned(),
            ]
        )
    );
    assert_eq!(
        ways(true),
        (
            vec![
                "a.py first".to_owned(),
                "b.py second".to_owned(),
                "x.py shared 2+1".to_owned(),
                "m2.py".to_owned(),
            ],
            vec![
                "a.py -> m2.py 1/0".to_owned(),
                "b.py -> x.py 1/0".to_owned(),
                "m2.py -> x.py 2/0".to_owned(),
            ]
        )
    );
    // Another way of the same length that happens to pass a box drawn
    // is not drawn with it: a place keeps one way from each of the two.
    let nodes = vec![
        node("a", "src/a.py"),
        node("b", "src/b.py"),
        node("p", "lib/p.py"),
        node("q", "lib/q.py"),
        node("q2", "lib/q.py"),
        node("x", "lib/x.py"),
    ];
    let links = vec![
        call("a", "p", "src/a.py"),
        call("b", "p", "src/b.py"),
        call("a", "q", "src/a.py"),
        call("b", "q", "src/b.py"),
        call("p", "x", "lib/p.py"),
        call("q", "x", "lib/q.py"),
        call("q2", "x", "lib/q.py"),
    ];
    let view = self::index(json!({"nodes": nodes, "links": links}))
        .shared(
            "one",
            asking("src/a.py", "src/b.py", None),
            &[],
            &|_| None,
            None,
            false,
            "",
        )
        .unwrap();
    assert_eq!(
        boxes(&view)[2..],
        ["p.py shared 1+1", "q.py shared 1+1", "x.py shared 2+2"]
    );
    assert_eq!(
        sorted(calls(&view)),
        [
            "a.py -> p.py 1/0",
            "a.py -> q.py 1/0",
            "b.py -> p.py 1/0",
            "b.py -> q.py 1/0",
            "q.py -> x.py 2/0",
        ]
    );
    // Places that the two reach only by long ways fill a picture before
    // it shows as many: each needs the boxes of its ways.
    let (mut nodes, mut links) = (
        vec![node("a", "src/a.py"), node("b", "src/b.py")],
        Vec::new(),
    );
    for at in 0..12 {
        // Each of twelve is five links from a.py and one from b.py.
        let last = format!("far/f{at:02}.py");
        nodes.push(node(&format!("f{at}"), &last));
        links.push(call("b", &format!("f{at}"), "src/b.py"));
        let (mut before, mut from) = ("a".to_owned(), "src/a.py".to_owned());
        for step in 0..4 {
            let (id, file) = (format!("w{at}_{step}"), format!("way/w{at:02}_{step}.py"));
            nodes.push(node(&id, &file));
            links.push(call(&before, &id, &from));
            (before, from) = (id, file);
        }
        links.push(call(&before, &format!("f{at}"), &from));
    }
    let long = self::index(json!({"nodes": nodes, "links": links}));
    let view = long
        .shared(
            "long",
            asking("src/a.py", "src/b.py", None),
            &[],
            &|_| None,
            None,
            false,
            "",
        )
        .unwrap();
    // Five boxes for each: eleven of the twelve fit a picture.
    assert_eq!((SIDE, FOLLOWED), (58, 60));
    assert_eq!(
        shared_of(&view),
        "11 drawn of what both use · both use 12 · 0 use both"
    );
    assert_eq!(view["nodes"].as_array().unwrap().len(), 2 + 55);
    assert_eq!(boxes(&view)[2], "f00.py shared 5+1");
    assert_eq!(view["edges"].as_array().unwrap().len(), 11 * 6);
    assert!(!boxes(&view).iter().any(|name| name.starts_with("f11")));
    // The next picture begins after those eleven, with the twelfth.
    let asked = Sharing {
        skip: 11,
        ..asking("src/a.py", "src/b.py", None)
    };
    let next = long
        .shared("long", asked, &[], &|_| None, None, false, "")
        .unwrap();
    assert_eq!(
        shared_of(&next),
        "1 drawn of what both use · both use 12 · 0 use both · after 11"
    );
    assert_eq!(next["nodes"].as_array().unwrap().len(), 2 + 5);
    assert_eq!(boxes(&next)[2], "f11.py shared 5+1");
}
#[test]
fn what_two_places_share_carries_what_a_chosen_change_did() {
    let marks = |view: &Value| {
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| {
                format!(
                    "{} {} {}",
                    node["name"].as_str().unwrap(),
                    node["changed"],
                    node["status"].as_str().unwrap_or("-")
                )
            })
            .collect::<Vec<_>>()
    };
    // Among files: a folder says how many of its files changed, a
    // deleted one apart, and a file of the change carries Git's letter.
    let office = office(2);
    let altered = [
        ("reports/r01.php".to_owned(), 'M'),
        ("reports/r02.php".to_owned(), 'D'),
        ("lib/db.php".to_owned(), 'A'),
    ];
    let folders = office
        .shared(
            "office",
            asking("app/api", "reports", None),
            &altered,
            &|_| None,
            Some("reports"),
            false,
            "",
        )
        .unwrap();
    assert_eq!(marks(&folders), ["api 0 -", "reports 1 -", "db.php 1 A"]);
    assert_eq!(folders["focus"]["uses"][0]["changed"], true);
    // Among symbols: one caller lies in a new file, the other in a file
    // the change left alone, and the lines of the function they both
    // call replaced others.
    let mut graph = basket_graph();
    graph["nodes"].as_array_mut().unwrap().push(json!({
        "id": "second", "label": "second()",
        "source_file": "src/second.php", "source_location": "L3"
    }));
    graph["links"].as_array_mut().unwrap().push(json!({
        "source": "second", "target": "format", "relation": "calls",
        "source_file": "src/second.php"
    }));
    let index = self::index(graph);
    let changed = [
        ("src/Cart.php".to_owned(), 'M'),
        ("src/other.php".to_owned(), 'A'),
    ];
    let edited = |path: &str| {
        Some(match path {
            "src/Cart.php" => Edited {
                new: false,
                text: BASKET.to_owned(),
                hunks: vec![hunk(34, 5, 2)],
            },
            _ => Edited {
                new: true,
                ..Edited::default()
            },
        })
    };
    let format = "src/Cart.php#L35:format_price()";
    let view = index
        .shared(
            "shop",
            asking(
                "src/other.php#L3:user()",
                "src/second.php#L3:second()",
                None,
            ),
            &changed,
            &edited,
            Some(format),
            false,
            "",
        )
        .unwrap();
    assert_eq!(
        marks(&view),
        ["user() 1 A", "second() 0 -", "format_price() 1 M"]
    );
    assert_eq!(
        (&view["focus"]["status"], &view["focus"]["shares"]),
        (&json!("M"), &json!([1, 1]))
    );
    // A row beside it says the same of the symbol it names.
    let rows = view["focus"]["used_by"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| format!("{} {}", row["name"].as_str().unwrap(), row["changed"]))
        .collect::<Vec<_>>();
    assert_eq!(rows, ["user() true", "second() false"]);
}
/// Graphs made at random, and what two of their places share checked
/// against distances worked out apart, on a graph without the two: how
/// many places both use and how many use both, which kind is drawn,
/// that the nearest are the ones drawn, how far each is said to be, and
/// a picture in which every box lies on a shortest way drawn whole,
/// one from each of the two. One picture after another shows every
/// place once, the nearest first, and a filter keeps the places whose
/// name holds it.
#[test]
fn what_two_places_share_on_a_graph_made_at_random_is_what_its_links_allow() {
    const FAR: usize = usize::MAX;
    let mut seed = 0xD1B5_4A32_D192_ED03_u64;
    let mut below = move |limit: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % limit as u64) as usize
    };
    // What the answers for two places were: whether the two lie apart,
    // how many places they share, whether fewer are drawn than there
    // are, whether leaving out the ways through the other left a place
    // out, and whether what uses both was drawn unasked.
    struct Told {
        apart: bool,
        shared: usize,
        capped: bool,
        through: bool,
        turned: bool,
        /// Pictures after the first that were needed to show a kind.
        further: usize,
        /// A filter kept some places of a kind and left some out.
        narrowed: bool,
    }
    // How far every place lies from every other along the links, by
    // ways that pass none of `closed`.
    let apart = |uses: &[Vec<bool>], closed: &dyn Fn(usize) -> bool| {
        let count = uses.len();
        let mut far = vec![vec![FAR; count]; count];
        for (user, row) in uses.iter().enumerate() {
            if closed(user) {
                continue;
            }
            far[user][user] = 0;
            for (used, linked) in row.iter().enumerate() {
                if *linked && user != used && !closed(used) {
                    far[user][used] = 1;
                }
            }
        }
        for through in 0..count {
            let onward = far[through].clone();
            for row in &mut far {
                let before = row[through];
                if before == FAR {
                    continue;
                }
                for (to, after) in onward.iter().enumerate() {
                    if *after != FAR && before + after < row[to] {
                        row[to] = before + after;
                    }
                }
            }
        }
        far
    };
    // The answers for two places against the distances: `asked` gives
    // the answer where nothing is asked for (`None`), for what both use
    // and for what uses both, after as many places as it is told and
    // with the filter it is given; `first` and `second` are what the
    // two places stand for, `whole` is how far the places lie from each
    // other by any way, `ids` tells a drawn place by its name, `labels`
    // are the names a filter is held against, and `needle` is a filter,
    // which is tried, with the pictures after the first, where there is
    // one.
    #[allow(clippy::too_many_arguments)]
    let check = |asked: &dyn Fn(Option<bool>, usize, &str) -> Value,
                 first: &[usize],
                 second: &[usize],
                 uses: &[Vec<bool>],
                 whole: &[Vec<usize>],
                 ids: &HashMap<String, usize>,
                 labels: &[String],
                 needle: Option<&str>,
                 what: &str| {
        let count = uses.len();
        if first.iter().any(|place| second.contains(place)) {
            for users in [None, Some(false), Some(true)] {
                let view = asked(users, 0, "");
                assert_eq!(view["shared"]["within"], true, "{what}");
                assert_eq!(view["nodes"], json!([]), "{what}");
            }
            return Told {
                apart: false,
                shared: 0,
                capped: false,
                through: false,
                turned: false,
                further: 0,
                narrowed: false,
            };
        }
        // No way to what is shared passes what the two hold.
        let own = |place: usize| first.contains(&place) || second.contains(&place);
        let far = apart(uses, &own);
        // How many links a place lies from one of the two along the
        // links, or, with `back`, against them: one link out of what
        // the one holds, and then a way among the rest.
        let near = |end: &[usize], place: usize, back: bool| {
            (0..count)
                .filter(|next| {
                    !own(*next)
                        && end.iter().any(|held| {
                            if back {
                                uses[*next][*held]
                            } else {
                                uses[*held][*next]
                            }
                        })
                })
                .map(|next| {
                    if back {
                        far[place][next]
                    } else {
                        far[next][place]
                    }
                })
                .filter(|far| *far != FAR)
                .min()
                .map(|far| far + 1)
        };
        // What one of the two reaches at all, by whatever way.
        let loose = |end: &[usize], place: usize, back: bool| {
            end.iter().any(|held| {
                if back {
                    whole[place][*held] != FAR
                } else {
                    whole[*held][place] != FAR
                }
            })
        };
        // Each kind: every place of it with how far the two are.
        let kinds = [false, true].map(|back| {
            (0..count)
                .filter(|at| !own(*at))
                .filter_map(|at| {
                    near(first, at, back)
                        .zip(near(second, at, back))
                        .map(|(a, b)| (at, a, b))
                })
                .collect::<Vec<_>>()
        });
        let all = kinds[0].len() + kinds[1].len();
        // A place that both reach, but one of them only through the
        // other, is not shared.
        let through = [false, true].iter().zip(&kinds).any(|(back, shared)| {
            (0..count).any(|at| {
                !own(at)
                    && !shared.iter().any(|(known, ..)| *known == at)
                    && loose(first, at, *back)
                    && loose(second, at, *back)
            })
        });
        // Unasked, what both use is drawn, unless only the other kind
        // has anything.
        let turned = kinds[0].is_empty() && !kinds[1].is_empty();
        let views = [asked(Some(false), 0, ""), asked(Some(true), 0, "")];
        assert_eq!(asked(None, 0, ""), views[usize::from(turned)], "{what}");
        // What a picture shows as shared: each place with the links to
        // the two taken together, as its box says them.
        let shown_in = |view: &Value| {
            view["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|node| {
                    let far = node["shares"].as_array()?;
                    let links = far.iter().map(|far| far.as_u64().unwrap()).sum::<u64>();
                    Some((ids[node["path"].as_str().unwrap()], links as usize))
                })
                .collect::<Vec<_>>()
        };
        let (mut capped, mut further, mut narrowed) = (false, 0, false);
        for (back, shared) in [false, true].iter().zip(&kinds) {
            let back = *back;
            let view = &views[usize::from(back)];
            let told = &view["shared"];
            let role = if back { "uses_both" } else { "both_use" };
            assert_eq!(
                (
                    &told["within"],
                    &told["side"],
                    &told["both_use"],
                    &told["uses_both"]
                ),
                (
                    &json!(false),
                    &json!(role),
                    &json!(kinds[0].len()),
                    &json!(kinds[1].len())
                ),
                "{what}: {role}"
            );
            let nodes = view["nodes"].as_array().unwrap();
            let edges = view["edges"].as_array().unwrap();
            if shared.is_empty() {
                assert_eq!(
                    (nodes.len(), edges.len(), &told["shown"]),
                    (0, 0, &json!(0)),
                    "{what}: {role}"
                );
                continue;
            }
            // The links drawn, by the numbers of the boxes at their ends.
            let place = |path: &Value| nodes.iter().position(|node| &node["path"] == path).unwrap();
            let drawn = edges
                .iter()
                .map(|edge| (place(&edge["from"]), place(&edge["to"])))
                .collect::<Vec<_>>();
            assert_eq!(view["between"], json!(drawn.len()), "{what}: {role}");
            // How many links drawn lead from one box to another.
            let steps = |from: usize, to: usize| {
                let mut far = vec![FAR; nodes.len()];
                far[from] = 0;
                let mut front = vec![from];
                while let Some(at) = front.pop() {
                    for (user, used) in &drawn {
                        if *user == at && far[at] + 1 < far[*used] {
                            far[*used] = far[at] + 1;
                            front.push(*used);
                        }
                    }
                }
                far[to]
            };
            let shown = nodes
                .iter()
                .enumerate()
                .filter(|(_, node)| node["shares"].is_array())
                .collect::<Vec<_>>();
            assert_eq!(told["shown"], json!(shown.len()), "{what}: {role}");
            assert!(
                (1..=SHARED.min(shared.len())).contains(&shown.len()),
                "{what}: {role}"
            );
            let mut furthest = 0;
            for (index, node) in &shown {
                let at = ids[node["path"].as_str().unwrap()];
                let (_, a, b) = shared
                    .iter()
                    .find(|(known, ..)| *known == at)
                    .unwrap_or_else(|| panic!("{what}: {node}"));
                // It says how far each of the two is, and a way of
                // that many links is drawn between it and each.
                assert_eq!(node["shares"], json!([a, b]), "{what}: {node}");
                let ways = if back {
                    (steps(*index, 0), steps(*index, 1))
                } else {
                    (steps(0, *index), steps(1, *index))
                };
                assert_eq!(ways, (*a, *b), "{what}: {node}");
                furthest = furthest.max(a + b);
            }
            // The nearest are the ones drawn, and where fewer are
            // drawn than have room the picture is full.
            let rest = shared
                .iter()
                .filter(|(at, ..)| {
                    !shown
                        .iter()
                        .any(|(_, node)| ids[node["path"].as_str().unwrap()] == *at)
                })
                .map(|(_, a, b)| a + b)
                .min();
            if let Some(next) = rest {
                assert!(furthest <= next, "{what}: {role}");
                capped = true;
                if shown.len() < SHARED {
                    assert!(nodes.len() - 2 + next - 1 > SIDE, "{what}: {role}");
                }
            }
            // The two come first, each once, and no place is drawn
            // twice.
            assert!(nodes.len() <= FOLLOWED, "{what}: {role}");
            let mut seen = HashSet::new();
            for (index, node) in nodes.iter().enumerate() {
                assert!(
                    seen.insert(node["path"].as_str().unwrap()),
                    "{what}: {node}"
                );
                assert_eq!(
                    (&node["start"], &node["end"]),
                    (&json!(index == 0), &json!(index == 1)),
                    "{what}: {role}"
                );
                // The links run from the two to what both use, and to
                // the two from what uses both: nothing drawn leads
                // back to them. A box has one way from each of the two
                // at most, and one that is neither of the two nor
                // shared lies on a way.
                let into = drawn.iter().filter(|(_, used)| *used == index).count();
                let out = drawn.iter().filter(|(user, _)| *user == index).count();
                let (towards, onward) = if back { (out, into) } else { (into, out) };
                if index < 2 {
                    assert!(towards == 0 && onward > 0, "{what}: {node}");
                } else {
                    assert!((1..=2).contains(&towards), "{what}: {node}");
                    assert!(node["shares"].is_array() || onward > 0, "{what}: {node}");
                }
            }
            // Each link drawn is a link of the graph, or, at one of
            // the two, a link of something it holds.
            for (user, used) in &drawn {
                let held = |index: usize| match index {
                    0 => first.to_vec(),
                    1 => second.to_vec(),
                    _ => vec![ids[nodes[index]["path"].as_str().unwrap()]],
                };
                assert!(
                    held(*user)
                        .iter()
                        .any(|from| held(*used).iter().any(|to| uses[*from][*to])),
                    "{what}: {} -> {}",
                    nodes[*user]["path"],
                    nodes[*used]["path"]
                );
            }
            let Some(needle) = needle else {
                continue;
            };
            // One picture after another shows every place of the kind
            // once, the nearest first: each begins where the one before
            // it ended.
            let mut order = shared.iter().map(|(_, a, b)| a + b).collect::<Vec<_>>();
            order.sort_unstable();
            let (mut met, mut links, mut skip) = (HashSet::new(), Vec::new(), 0);
            while skip < shared.len() {
                // The first of them is the picture that was checked.
                let page = match skip {
                    0 => view.clone(),
                    skip => asked(Some(back), skip, ""),
                };
                let mut part = shown_in(&page);
                assert_eq!(
                    (
                        &page["shared"]["skip"],
                        &page["shared"]["matching"],
                        &page["shared"]["shown"]
                    ),
                    (&json!(skip), &json!(shared.len()), &json!(part.len())),
                    "{what}: {role} after {skip}"
                );
                assert_eq!(
                    part.len(),
                    SHARED.min(shared.len() - skip),
                    "{what}: {role} after {skip}"
                );
                part.sort_by_key(|(_, links)| *links);
                for (at, far) in part {
                    assert!(met.insert(at), "{what}: {role} after {skip}");
                    links.push(far);
                    skip += 1;
                }
                further += usize::from(skip < shared.len());
            }
            assert_eq!(links, order, "{what}: {role}");
            // A beginning past the last place is the last place.
            let past = asked(Some(back), shared.len() + 2, "");
            assert_eq!(
                (
                    &past["shared"]["skip"],
                    shown_in(&past).iter().map(|(_, links)| *links).max()
                ),
                (&json!(shared.len() - 1), order.last().copied()),
                "{what}: {role}"
            );
            // A filter keeps the places whose name holds it, and leaves
            // what both kinds count as it was.
            let kept = shared
                .iter()
                .filter(|(at, ..)| labels[*at].contains(needle))
                .map(|(at, ..)| *at)
                .collect::<HashSet<_>>();
            let some = asked(Some(back), 0, needle);
            assert_eq!(
                (
                    &some["shared"]["matching"],
                    &some["shared"]["both_use"],
                    &some["shared"]["uses_both"]
                ),
                (
                    &json!(kept.len()),
                    &json!(kinds[0].len()),
                    &json!(kinds[1].len())
                ),
                "{what}: {role} with {needle}"
            );
            let part = shown_in(&some);
            assert_eq!(
                part.len(),
                SHARED.min(kept.len()),
                "{what}: {role} with {needle}"
            );
            assert!(
                part.iter().all(|(at, _)| kept.contains(at)),
                "{what}: {role} with {needle}"
            );
            assert_eq!(
                some["nodes"].as_array().unwrap().is_empty(),
                kept.is_empty(),
                "{what}: {role} with {needle}"
            );
            narrowed |= !kept.is_empty() && kept.len() < shared.len();
        }
        Told {
            apart: true,
            shared: all,
            capped,
            through,
            turned,
            further,
            narrowed,
        }
    };
    let (mut some, mut none, mut within) = (0, 0, 0);
    let (mut capped, mut through, mut turned) = (0, 0, 0);
    let (mut further, mut narrowed) = (0, 0);
    for round in 0..60 {
        // A few folders of files, some names in each file, and links
        // between the names wherever they fall.
        let count = 6 + below(26);
        let files = 2 + below(count.min(16) - 1);
        let folders = 1 + below(4);
        let file_of = |name: usize| name % files;
        let path_of = |file: usize| format!("d{}/f{file:02}.py", file % folders);
        let nodes = (0..count)
                .map(|name| json!({"id": format!("n{name}"), "label": format!("n{name}"), "source_file": path_of(file_of(name))}))
                .collect::<Vec<_>>();
        let mut pairs = (0..below(count * 3 + 1))
            .map(|_| (below(count), below(count)))
            .collect::<Vec<_>>();
        // Every third graph has two names that many others use, and
        // that use many: more is shared than a picture shows. Every
        // third one has two names with long ways to what they share.
        let (one, other) = (below(count), below(count));
        match round % 3 {
            0 => {
                for _ in 0..30 {
                    let (above, under) = (below(count), below(count));
                    pairs.extend([(above, one), (above, other), (one, under), (other, under)]);
                }
            }
            1 => {
                for _ in 0..6 {
                    let way = [below(count), below(count), below(count), below(count)];
                    pairs.extend([(one, way[0]), (way[0], way[1]), (way[1], way[2])]);
                    pairs.extend([(other, way[3]), (way[3], way[2])]);
                }
            }
            _ => {}
        }
        let mut links = Vec::new();
        let mut uses = vec![vec![false; count]; count];
        let mut file_uses = vec![vec![false; files]; files];
        for (user, used) in pairs {
            if user == used {
                continue;
            }
            uses[user][used] = true;
            if file_of(user) != file_of(used) {
                file_uses[file_of(user)][file_of(used)] = true;
            }
            links.push(json!({"source": format!("n{user}"), "target": format!("n{used}"), "relation": "calls", "source_file": path_of(file_of(user))}));
        }
        let index = self::index(json!({"nodes": nodes, "links": links}));
        // Each name as a picture selects it, and each file by its path.
        let mut keys = vec![String::new(); count];
        for file in 0..index.files.len() as u32 {
            for (key, at) in index.keys(file).into_iter().zip(index.range(file)) {
                let name: usize = index.label(at)[1..].parse().unwrap();
                keys[name] = key;
            }
        }
        let names = keys
            .iter()
            .enumerate()
            .map(|(name, key)| (key.clone(), name))
            .collect::<HashMap<_, _>>();
        let paths = (0..files)
            .map(|file| (path_of(file), file))
            .collect::<HashMap<_, _>>();
        let (far, file_far) = (apart(&uses, &|_| false), apart(&file_uses, &|_| false));
        let file_labels = (0..files).map(path_of).collect::<Vec<_>>();
        let name_labels = (0..count)
            .map(|name| format!("n{name}"))
            .collect::<Vec<_>>();
        // What a place stands for among files, and among names.
        let in_folder = |folder: usize| {
            (0..files)
                .filter(|file| file % folders == folder)
                .collect::<Vec<_>>()
        };
        let names_of = |held: &[usize]| {
            (0..count)
                .filter(|name| held.contains(&file_of(*name)))
                .collect::<Vec<_>>()
        };
        for turn in 0..10 {
            // The two names that were given much to share come first.
            let (a, b) = if turn == 0 {
                (one, other)
            } else {
                (below(count), below(count))
            };
            let (file_a, file_b) = (file_of(a), file_of(b));
            let (folder_a, folder_b) = (file_a % folders, file_b % folders);
            let asked = [
                // Among files: two files, a folder and a file, two folders.
                (
                    path_of(file_a),
                    path_of(file_b),
                    vec![file_a],
                    vec![file_b],
                    false,
                ),
                (
                    format!("d{folder_a}"),
                    path_of(file_b),
                    in_folder(folder_a),
                    vec![file_b],
                    false,
                ),
                (
                    format!("d{folder_a}"),
                    format!("d{folder_b}"),
                    in_folder(folder_a),
                    in_folder(folder_b),
                    false,
                ),
                // Among names: two names, a name and a file, a folder and a name.
                (keys[a].clone(), keys[b].clone(), vec![a], vec![b], true),
                (
                    keys[a].clone(),
                    path_of(file_b),
                    vec![a],
                    names_of(&[file_b]),
                    true,
                ),
                (
                    format!("d{folder_a}"),
                    keys[b].clone(),
                    names_of(&in_folder(folder_a)),
                    vec![b],
                    true,
                ),
            ];
            // What a filter is held against: the path of a file, and a
            // name as it is shown. The filters are a part of some. They
            // are tried for the first pairs of a graph, with the
            // pictures after the first.
            let (file_needle, name_needle) =
                (format!("f{}", below(2)), format!("{}", 1 + below(3)));
            let tried = turn < 2;
            for (from, to, first, second, named) in &asked {
                let what = format!("round {round}: {from} and {to}");
                let answer = |users: Option<bool>, skip: usize, filter: &str| {
                    let asked = Sharing {
                        skip,
                        ..asking(from, to, users)
                    };
                    let view = index
                        .shared("random", asked, &[], &|_| None, None, false, filter)
                        .unwrap_or_else(|| panic!("{what}"));
                    assert_eq!(view["shared"]["names"], *named, "{what}");
                    view
                };
                let told = if *named {
                    check(
                        &answer,
                        first,
                        second,
                        &uses,
                        &far,
                        &names,
                        &name_labels,
                        tried.then_some(name_needle.as_str()),
                        &what,
                    )
                } else {
                    check(
                        &answer,
                        first,
                        second,
                        &file_uses,
                        &file_far,
                        &paths,
                        &file_labels,
                        tried.then_some(file_needle.as_str()),
                        &what,
                    )
                };
                match (told.apart, told.shared) {
                    (false, _) => within += 1,
                    (true, 0) => none += 1,
                    _ => some += 1,
                }
                capped += usize::from(told.capped);
                through += usize::from(told.through);
                turned += usize::from(told.turned);
                further += told.further;
                narrowed += usize::from(told.narrowed);
            }
        }
    }
    // The graphs are of every sort: places that share something, that
    // share nothing and that lie in each other, more shared than a
    // picture shows, places one reaches only through the other, and
    // places that nothing but their users ties together; pictures that
    // go on after the first, and filters that keep a part.
    assert!(
        some > 1500
            && none > 300
            && within > 900
            && capped > 300
            && through > 600
            && turned > 150
            && further > 120
            && narrowed > 150,
        "{some} {none} {within} {capped} {through} {turned} {further} {narrowed}"
    );
}
/// What a search found, in its order: what each place is, its name and
/// where it lies.
fn sought(index: &Index, query: &str) -> Vec<String> {
    index.find(query)["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let text = |key: &str| row[key].as_str().unwrap_or("").to_owned();
            match row["file"].as_str() {
                Some(file) => format!(
                    "{} {} in {file}:{}",
                    text("what"),
                    text("name"),
                    row["line"]
                ),
                None => format!("{} {} in {}", text("what"), text("name"), text("in")),
            }
        })
        .collect()
}
const TOTAL: &str = "method Cart.total() in src/Cart/Cart.php:19";
const TEST_TOTAL: &str = "method CartTest.testTotal() in tests/CartTest.php:7";
#[test]
fn a_search_finds_folders_files_and_names_wherever_they_lie() {
    let index = store();
    // The folder, the file without its ending and the class are all
    // called Cart; what only starts with it comes after them. A method
    // of Cart is not called Cart, and letters match whatever their case.
    let cart = [
        "folder Cart in src",
        "file Cart.php in src/Cart",
        "type Cart in src/Cart/Cart.php:12",
        "file CartTest.php in tests",
        "type CartTest in tests/CartTest.php:5",
    ];
    assert_eq!(sought(&index, "cart"), cart);
    assert_eq!(sought(&index, "  CART "), cart);
    let found = index.find("  CART ");
    assert_eq!(
        (&found["mode"], &found["query"]),
        (&json!("find"), &json!("CART"))
    );
    assert_eq!(
        (
            &found["found"],
            &found["folders"],
            &found["files"],
            &found["names"]
        ),
        (&json!(5), &json!(1), &json!(2), &json!(2))
    );
    // A method is named without its class and its brackets, and a word
    // inside a name written likeThis or like_this is found as well.
    assert_eq!(sought(&index, "total"), [TOTAL, TEST_TOTAL]);
    assert_eq!(
        sought(&index, "price"),
        [
            "file Price.php in src/Cart",
            "type Price in src/Cart/Price.php:4",
            "function format_price() in src/Cart/Cart.php:29",
        ]
    );
    // With its class before it, it is named as it is shown.
    assert_eq!(sought(&index, "cart.to"), [TOTAL]);
    assert_eq!(sought(&index, "t.total"), [TOTAL]);
    // A constant and a case are names; of two called alike the more
    // linked comes first.
    assert_eq!(
        sought(&index, "limit"),
        ["name LIMIT in src/Cart/Cart.php:5"]
    );
    assert_eq!(
        sought(&index, "paid"),
        [
            "method Notifier.paid() in src/Order/Notifier.php:11",
            "case Paid in src/Cart/Cart.php:10",
        ]
    );
    // The node that stands for a file is the file, found once, and a
    // comment the tool kept is no name.
    assert_eq!(sought(&index, "cart.php"), ["file Cart.php in src/Cart"]);
    assert!(sought(&index, "cents").is_empty());
    // Nothing is called that, and nothing was asked.
    assert_eq!(index.find("zzz")["found"], 0);
    let nothing = index.find("   ");
    assert_eq!(
        (&nothing["found"], &nothing["query"], &nothing["rows"]),
        (&json!(0), &json!(""), &json!([]))
    );
}
#[test]
fn further_words_of_a_search_say_where_a_place_lies() {
    let index = store();
    // Each word stands in the name or on the way to it: in the class of
    // a method, in the name of its file, in a folder above.
    assert_eq!(sought(&index, "total cart"), [TOTAL, TEST_TOTAL]);
    assert_eq!(sought(&index, "total test"), [TEST_TOTAL]);
    assert_eq!(sought(&index, "total tests"), [TEST_TOTAL]);
    assert_eq!(sought(&index, "total price.php"), Vec::<String>::new());
    assert_eq!(
        sought(&index, "php order"),
        ["file Notifier.php in src/Order"]
    );
    // A word that only says where names nothing by itself.
    assert_eq!(sought(&index, "order"), ["folder Order in src"]);
    // A word with a slash is the end of a path: the folder a place lies
    // in, and the start of its name.
    assert_eq!(sought(&index, "cart/price"), ["file Price.php in src/Cart"]);
    assert_eq!(
        sought(&index, "CART/PRICE.PHP"),
        ["file Price.php in src/Cart"]
    );
    assert_eq!(
        sought(&index, "src/"),
        ["folder Cart in src", "folder Order in src"]
    );
    assert_eq!(sought(&index, "order/"), ["file Notifier.php in src/Order"]);
    assert!(
        sought(&index, "rder/notifier").len() == 1 && sought(&index, "/rder/notifier").is_empty()
    );
    // At the front it is the start of a folder's name, or the project.
    assert_eq!(
        sought(&index, "/cart"),
        [
            "folder Cart in src",
            "file Cart.php in src/Cart",
            "file CartTest.php in tests",
        ]
    );
    assert_eq!(sought(&index, "/tests"), ["folder tests in "]);
    // For a name such a word says where its file lies.
    assert_eq!(sought(&index, "src/cart/ total"), [TOTAL]);
    assert!(sought(&index, "src/cart/total").is_empty());
}
#[test]
fn a_search_lists_the_best_named_first_and_counts_what_it_does_not_list() {
    let node = |id: &str, label: &str, file: &str| json!({"id": id, "label": label, "file_type": "code", "source_file": file, "source_location": "L1"});
    let call = |from: &str, to: &str| json!({"source": from, "target": to, "relation": "calls", "confidence": "EXTRACTED", "source_file": "c.py"});
    let index = index(json!({
        "nodes": [
            node("part", "Reprice", "b.py"),
            node("snake", "net_price()", "a.py"),
            node("camel", "netPrice()", "b.py"),
            node("list", "PriceList", "a.py"),
            node("plan", "PricePlan", "b.py"),
            node("whole", "Price", "a.py"),
            node("one", "one()", "c.py"),
            node("two", "two()", "c.py"),
            node("three", "three()", "c.py"),
            node("small", "index()", "a.py"),
            node("capital", "Index", "b.py"),
        ],
        "links": [
            call("one", "part"), call("two", "part"), call("three", "part"),
            call("one", "plan"), call("two", "plan"),
            call("one", "list"),
            call("one", "small"), call("two", "small"),
        ]
    }));
    // The whole name, then what starts with it, then a word inside a
    // name, then any part of one. Reprice is the most linked of all and
    // comes last; among those that start alike the more linked is first,
    // and of two equally linked the shorter name.
    assert_eq!(
        sought(&index, "price"),
        [
            "name Price in a.py:1",
            "name PricePlan in b.py:1",
            "name PriceList in a.py:1",
            "function netPrice() in b.py:1",
            "function net_price() in a.py:1",
            "name Reprice in b.py:1",
        ]
    );
    // Both are called index, and the more linked comes first; typed
    // with a capital, the word puts first what is written that way.
    assert_eq!(
        sought(&index, "index"),
        ["function index() in a.py:1", "name Index in b.py:1"]
    );
    assert_eq!(
        sought(&index, "Index"),
        ["name Index in b.py:1", "function index() in a.py:1"]
    );
    // What a method is called counts, not what its class is called.
    let classes = self::index(json!({
        "nodes": [
            node("tests", "IndexTests", "t.py"),
            node("method", ".index()", "t.py"),
            node("other", "user()", "t.py"),
            node("type", "Index", "i.py"),
        ],
        "links": [
            {"source": "tests", "target": "method", "relation": "method", "source_file": "t.py"},
            {"source": "other", "target": "method", "relation": "calls", "source_file": "t.py"},
        ]
    }));
    assert_eq!(
        sought(&classes, "index"),
        [
            "method IndexTests.index() in t.py:1",
            "name Index in i.py:1",
            "type IndexTests in t.py:1",
        ]
    );
    assert_eq!(
        sought(&classes, "Index"),
        [
            "name Index in i.py:1",
            "method IndexTests.index() in t.py:1",
            "type IndexTests in t.py:1",
        ]
    );
    // A word that says where is looked for, as typed, on the way.
    assert_eq!(
        sought(&classes, "index Tests")[0],
        "method IndexTests.index() in t.py:1"
    );
    // A capital orders what is found and leaves nothing out.
    assert_eq!(sought(&index, "Index a.py"), ["function index() in a.py:1"]);
    // A capital after capitals begins a word where small letters follow.
    assert_eq!(
        super::named("XMLParser", "xmlparser", "parser"),
        Some(Named::Word)
    );
    assert_eq!(
        super::named("XMLParser", "xmlparser", "lparser"),
        Some(Named::Part)
    );
    assert_eq!(super::named("v2Api", "v2api", "api"), Some(Named::Word));
    // Letters of more than one byte are walked whole.
    assert_eq!(super::named("Ééé", "ééé", "é"), Some(Named::Start));
    assert_eq!(super::named("жук", "жук", "ук"), Some(Named::Part));
    // Where the lower case is longer than the name, capitals are not
    // looked for: they do not lie where the lower case says.
    assert_eq!(super::named("aStan", "astan", "stan"), Some(Named::Word));
    assert_eq!(
        super::named("İaStan", "i\u{307}astan", "stan"),
        Some(Named::Part)
    );
    assert_eq!(super::named("any", "any", ""), None);
    // More is found than a list holds: the best are listed, all counted.
    let wide = office(FOUND + 20);
    let found = wide.find("r");
    let rows = found["rows"].as_array().unwrap();
    assert_eq!(rows.len(), FOUND);
    // The reports, each a file and a name in it, and their folder; and
    // `core`, `orders`, `users` and `store` with the files of the four.
    assert_eq!(found["found"], json!(1 + 2 * (FOUND + 20) + 8));
    assert_eq!(
        (&found["folders"], &found["files"], &found["names"]),
        (&json!(1), &json!(FOUND + 24), &json!(FOUND + 24))
    );
    // The folder holds the links of all its files.
    assert_eq!(sought(&wide, "r")[0], "folder reports in ");
    assert_eq!(rows[0]["files"], json!(FOUND + 20));
}
#[test]
fn what_a_search_found_is_selected_in_the_picture_it_lies_in() {
    let index = store();
    // A name is told by what the picture of its file selects it by.
    let found = index.find("total");
    let row = &found["rows"][0];
    assert_eq!(
        (&row["path"], &row["file"], &row["what"], &row["line"]),
        (
            &json!("src/Cart/Cart.php#L19:.total()"),
            &json!(CART),
            &json!("method"),
            &json!(19)
        )
    );
    // It is shown with its class, and says where its own name begins.
    assert_eq!(
        (&row["name"], &row["own"]),
        (&json!("Cart.total()"), &json!(5))
    );
    assert_eq!(index.find("price")["rows"][1]["own"], 0);
    let view = index.view("shop", CART, row["path"].as_str(), false, "");
    assert_eq!(view["focus"]["path"], row["path"]);
    // A file and a folder are told by their paths, with where they lie.
    let found = index.find("cart");
    assert_eq!(
        found["rows"][0],
        json!({"path": "src/Cart", "name": "Cart", "what": "folder", "in": "src", "files": 2})
    );
    assert_eq!(
        found["rows"][1],
        json!({"path": CART, "name": "Cart.php", "what": "file", "in": "src/Cart", "symbols": 9})
    );
    let view = index.view("shop", "src/Cart", Some(CART), false, "");
    assert_eq!(view["focus"]["path"], CART);
    // What is left out with the tests is not found either.
    let plain = index.plain_with(&|_| None);
    assert_eq!(sought(&plain, "total"), [TOTAL]);
    assert_eq!(sought(&plain, "cart").len(), 3);
}
#[test]
fn a_selected_place_is_drawn_whatever_the_limit_and_the_filter_say() {
    let files = (0..SHOWN + 5)
        .map(|at| json!({"id": format!("n{at}"), "source_file": format!("lib/f{at:02}.php")}))
        .collect::<Vec<_>>();
    let links = (1..SHOWN + 5)
            .map(|at| json!({"source": format!("n{at}"), "target": "n0", "relation": "imports", "source_file": format!("lib/f{at:02}.php")}))
            .collect::<Vec<_>>();
    let wide = index(json!({"nodes": files, "edges": links}));
    let last = format!("lib/f{:02}.php", SHOWN + 4);
    let paths = |view: &Value| {
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["path"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    // The last of the files is no one of the most linked.
    let view = wide.view("wide", "lib", None, false, "");
    assert!(!paths(&view).contains(&last));
    assert_eq!(view["beyond"], false);
    // Selected, it is drawn beside them, with its link, and the view
    // says that one more is drawn than the limit lets.
    let view = wide.view("wide", "lib", Some(&last), false, "");
    assert_eq!(view["beyond"], true);
    assert_eq!(paths(&view).len(), SHOWN + 1);
    assert_eq!(paths(&view).last(), Some(&last));
    assert_eq!(view["focus"]["path"], json!(last));
    assert!(rows(&view, "edges").contains(&format!("{last} -> lib/f00.php 1/0")));
    // A filter that leaves it out does not hide it.
    let view = wide.view("wide", "lib", Some(&last), false, "f0");
    assert_eq!(paths(&view).len(), 11);
    assert_eq!(paths(&view).last(), Some(&last));
    assert_eq!(
        (&view["matching"], &view["beyond"]),
        (&json!(10), &json!(true))
    );
    // One that is drawn anyway is drawn once.
    let view = wide.view("wide", "lib", Some("lib/f01.php"), false, "");
    assert_eq!(paths(&view).len(), SHOWN);
    assert_eq!(view["beyond"], false);
    // So it is in a folder that is spread out and shows a part of what
    // it holds.
    let index = wheel(1, HELD + 5);
    let spoke = format!("p0/f{:02}.rs", HELD + 4);
    let spread = ["p0".to_owned()];
    let view = index.view_with("wheel", "", &spread, Some(&spoke), false, "");
    assert!(held(&view).contains(&format!("{spoke} in p0")));
    assert_eq!(framed(&view), [format!("p0 {}/{}", HELD + 1, HELD + 5)]);
    // A place outside the open folder is listed, not drawn.
    let shop = shop();
    let outside = shop.view("shop", "src", Some("tests"), false, "");
    assert_eq!(outside["focus"]["path"], "tests");
    assert_eq!(outside["beyond"], false);
    assert_eq!(
        rows(&outside, "nodes"),
        rows(&shop.view("shop", "src", None, false, ""), "nodes")
    );
}

#[test]
fn view_with_whole_unfolds_breadth_first_with_raised_caps_and_flags() {
    let mut nodes = Vec::new();
    let mut links = Vec::new();
    for folder in 0..45 {
        let dir = if folder % 2 == 0 {
            format!("m{folder:02}")
        } else {
            format!("m{folder:02}/sub")
        };
        for file in 0..2 {
            let id = format!("sym_{folder}_{file}");
            let path = format!("{dir}/f{file}.rs");
            nodes.push(
                json!({"id": id, "label": format!("fn_{folder}_{file}()"), "source_file": path}),
            );
        }
        if folder > 0 {
            links.push(json!({
                "source": format!("sym_{folder}_0"),
                "target": format!("sym_{}_0", folder - 1),
                "relation": "calls",
                "confidence": "EXTRACTED",
                "source_file": format!("{dir}/f0.rs"),
            }));
        }
    }
    let wide = index(json!({"nodes": nodes, "links": links}));
    assert_eq!(wide.files().len(), 90);
    assert!(wide.file_id("m00/f0.rs").is_some());
    assert_eq!(wide.file_id("missing.rs"), None);
    assert!(!wide.pairs().is_empty());
    assert!(!wide.symbol_entries().is_empty());
    assert!(!wide.uses().is_empty());

    let collapsed = wide.view_with_whole("wide", "", &[], None, false, "", false);
    assert_eq!(collapsed["whole"], false);
    assert_eq!(collapsed["nodes"].as_array().unwrap().len(), SHOWN);
    assert!(collapsed["frames"].as_array().unwrap().is_empty());

    let unfolded = wide.view_with_whole("wide", "", &[], None, false, "", true);
    assert_eq!(unfolded["whole"], true);
    let drawn_nodes = unfolded["nodes"].as_array().unwrap();
    assert_eq!(drawn_nodes.len(), 90);
    assert!(drawn_nodes.iter().all(|node| node["folder"] == false));
    let frames = unfolded["frames"].as_array().unwrap();
    assert!(frames.len() >= 45);
    assert!(
        unfolded["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge["from"] == "m01/sub/f0.rs" && edge["to"] == "m00/f0.rs")
    );

    let scoped = wide.view_with_whole("wide", "m00", &[], None, false, "", true);
    assert_eq!(scoped["whole"], true);
    assert_eq!(scoped["scope"], "m00");
    assert_eq!(scoped["nodes"].as_array().unwrap().len(), 2);
}

#[test]
fn file_and_link_views_include_declared_referenced_members_and_visibility_warnings() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
            dir.path().join("src/Invoice.php"),
            "<?php\nclass Invoice {\n    public const PRECISION = 2;\n    private int $total = 100;\n    public function subtotal(): int { return $this->total; }\n}\n",
        )
        .unwrap();
    std::fs::write(
            dir.path().join("src/Checkout.php"),
            "<?php\nrequire_once __DIR__ . '/Invoice.php';\nclass Checkout {\n    public function apply(Invoice $inv): int {\n        return $inv->subtotal() + $inv->total + Invoice::PRECISION;\n    }\n}\n",
        )
        .unwrap();

    let graph = json!({
        "nodes": [
            {"id": "inv_cls", "label": "Invoice", "file_type": "code", "_callable_class": "true", "source_file": "src/Invoice.php", "source_location": "L2"},
            {"id": "inv_prec", "label": "PRECISION", "file_type": "code", "source_file": "src/Invoice.php", "source_location": "L3"},
            {"id": "inv_total", "label": "total", "file_type": "code", "source_file": "src/Invoice.php", "source_location": "L4"},
            {"id": "inv_sub", "label": "subtotal()", "file_type": "code", "source_file": "src/Invoice.php", "source_location": "L5"},
            {"id": "chk_cls", "label": "Checkout", "file_type": "code", "_callable_class": "true", "source_file": "src/Checkout.php", "source_location": "L3"},
            {"id": "chk_apply", "label": "apply()", "file_type": "code", "source_file": "src/Checkout.php", "source_location": "L4"}
        ],
        "links": [
            {"source": "inv_cls", "target": "inv_prec", "relation": "contains", "confidence": "EXTRACTED", "source_file": "src/Invoice.php"},
            {"source": "inv_cls", "target": "inv_total", "relation": "contains", "confidence": "EXTRACTED", "source_file": "src/Invoice.php"},
            {"source": "inv_cls", "target": "inv_sub", "relation": "method", "confidence": "EXTRACTED", "source_file": "src/Invoice.php"},
            {"source": "chk_cls", "target": "chk_apply", "relation": "method", "confidence": "EXTRACTED", "source_file": "src/Checkout.php"},
            {"source": "chk_apply", "target": "inv_sub", "relation": "calls", "confidence": "EXTRACTED", "source_file": "src/Checkout.php"},
            {"source": "chk_apply", "target": "inv_total", "relation": "calls", "confidence": "EXTRACTED", "source_file": "src/Checkout.php"},
            {"source": "chk_apply", "target": "inv_prec", "relation": "calls", "confidence": "EXTRACTED", "source_file": "src/Checkout.php"}
        ]
    });
    let index = Index::read(graph.to_string().as_bytes(), dir.path()).unwrap();

    // File view without focus keeps existing invariants intact and emits declared_members & neighbor refs
    let overview = index.view("shop", "src/Invoice.php", None, false, "");
    assert_eq!(overview["file"]["path"], "src/Invoice.php");
    assert_eq!(overview["file"]["symbols"], 4);
    assert_eq!(overview["file"]["types"], 1);
    assert_eq!(overview["file"]["methods"], 1);
    assert_eq!(overview["file"]["used_by"], 1);

    let declared = overview["declared_members"].as_array().unwrap();
    assert!(declared.iter().any(|m| {
        m["name"] == "$total" && m["kind"] == "property" && m["visibility"] == "private"
    }));
    assert!(declared.iter().any(|m| {
        m["name"] == "PRECISION" && m["kind"] == "const" && m["visibility"] == "public"
    }));

    let nodes = overview["nodes"].as_array().unwrap();
    let inv_node = nodes
        .iter()
        .find(|n| n["name"] == "Invoice")
        .expect("Invoice class node");
    assert_eq!(inv_node["kind"], "type");
    assert!(inv_node["members"].as_u64().is_some());

    let total_node = nodes
        .iter()
        .find(|n| n["name"] == "total")
        .expect("total symbol node");
    assert_eq!(total_node["kind"], "name");
    assert_eq!(total_node["member_kind"], "property");
    assert_eq!(total_node["visibility"], "private");

    let inv_key = inv_node["path"].as_str().unwrap().to_owned();
    let class_focus = index.view("shop", "src/Invoice.php", Some(&inv_key), false, "");
    assert_eq!(class_focus["focus"]["kind"], "type");
    assert_eq!(class_focus["focus"]["member_kind"], "class");
    assert!(
        class_focus["focus"]["members"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| {
                m["name"] == "total"
                    && m["kind"] == "name"
                    && m["member_kind"] == "property"
                    && m["visibility"] == "private"
            })
    );

    let checkout_node = nodes
        .iter()
        .find(|n| n["path"] == "src/Checkout.php" && n["kind"] == "file")
        .expect("Checkout neighbor file node");
    assert!(
        checkout_node.get("declared_members").is_none(),
        "around nodes must not duplicate declared_members arrays"
    );
    assert!(
        checkout_node.get("referenced_members").is_none(),
        "around nodes must not duplicate referenced_members arrays"
    );

    // Neighbor file focus adds referenced_members and visibility_warnings cleanly
    let neighbor_focus = index.view(
        "shop",
        "src/Invoice.php",
        Some("src/Checkout.php"),
        false,
        "",
    );
    assert!(
        neighbor_focus["focus"]["referenced_members"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["name"] == "$total" && r["access_risk"] == "inaccessible")
    );
    assert!(
        !neighbor_focus["focus"]["visibility_warnings"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // Symbol focus on Invoice class overlays visibility/members without breaking symbol fields
    let total_key = total_node["path"].as_str().unwrap().to_owned();
    let sym_focus = index.view("shop", "src/Invoice.php", Some(&total_key), false, "");
    assert_eq!(sym_focus["focus"]["kind"], "name");
    assert_eq!(sym_focus["focus"]["member_kind"], "property");
    assert_eq!(sym_focus["focus"]["visibility"], "private");
    assert!(
        !sym_focus["focus"]["used_by"][0]["visibility_warnings"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // Link view preserves from/to intact, enriches groups/names/edges, and reports inaccessible member warning
    let link = index.link("src/Checkout.php", "src/Invoice.php", false);
    assert_eq!(
        link["from"],
        json!({"path": "src/Checkout.php", "name": "Checkout.php", "what": "file"})
    );
    assert_eq!(
        link["to"],
        json!({"path": "src/Invoice.php", "name": "Invoice.php", "what": "file"})
    );
    assert!(link["member_refs"].as_array().unwrap().iter().any(|r| {
        r["name"] == "$total" && r["visibility"] == "private" && r["access_risk"] == "inaccessible"
    }));
    assert!(
        link["visibility_warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["member"] == "$total" && w["visibility"] == "private")
    );
    assert!(
        !link["users"]["groups"][0]["referenced_members"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let used_group = &link["used"]["groups"][0];
    assert!(
        !used_group["declared_members"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(used_group["names"].as_array().unwrap().iter().any(|n| {
        n["name"] == "total"
            && n["kind"] == "name"
            && n["member_kind"] == "property"
            && n["visibility"] == "private"
            && n["access_risk"] == "inaccessible"
    }));
    assert!(link["edges"].as_array().unwrap().iter().any(|e| {
        e["to"].as_str().is_some_and(|s| s.contains("total")) && e["access_risk"] == "inaccessible"
    }));
}

#[test]
fn edges_expose_aggregated_dependency_kinds_across_map_modes() {
    let sym = |id: &str, label: &str, file: &str, line: &str| json!({"id": id, "label": label, "source_file": file, "source_location": line});
    let link = |from: &str, to: &str, relation: &str, confidence: &str| json!({"source": from, "target": to, "relation": relation, "confidence": confidence});
    let ctrl_file = "src/ctrl/OrderController.php";
    let svc_file = "src/svc/OrderService.php";
    let audit_file = "src/shared/AuditLog.php";
    let helper_file = "src/extra/Helper.php";
    let index = index(json!({
        "nodes": [
            sym("ctrl", "OrderController", ctrl_file, "L3"),
            sym("ctrl_handle", "handle()", ctrl_file, "L7"),
            sym("svc", "OrderService", svc_file, "L3"),
            sym("svc_run", "run()", svc_file, "L8"),
            sym("audit", "AuditLog", audit_file, "L3"),
            sym("audit_rec", "record()", audit_file, "L6"),
            sym("helper", "Helper", helper_file, "L3"),
        ],
        "links": [
            link("ctrl_handle", "svc_run", "calls", "EXTRACTED"),
            link("ctrl_handle", "svc_run", "calls", "EXTRACTED"),
            link("ctrl_handle", "svc_run", "calls", "INFERRED"),
            link("ctrl_handle", "svc_run", "reads", "EXTRACTED"),
            link("ctrl_handle", "svc_run", "reads", "EXTRACTED"),
            link("ctrl_handle", "svc_run", "writes", "EXTRACTED"),
            link("ctrl_handle", "svc_run", "uses_const", "EXTRACTED"),
            link("ctrl", "svc", "imports", "EXTRACTED"),
            link("ctrl", "svc", "extends", "EXTRACTED"),
            link("ctrl", "svc", "implements", "EXTRACTED"),
            link("svc_run", "audit_rec", "calls", "EXTRACTED"),
            link("svc_run", "audit_rec", "imports", "EXTRACTED"),
            link("audit_rec", "ctrl_handle", "writes", "EXTRACTED"),
            link("helper", "svc_run", "calls", "EXTRACTED"),
            link("helper", "svc_run", "reads", "EXTRACTED"),
        ],
    }));

    let edge = |view: &Value, from: &str, to: &str| -> Value {
        view["edges"]
            .as_array()
            .unwrap()
            .iter()
            .find(|edge| edge["from"] == from && edge["to"] == to)
            .unwrap_or_else(|| panic!("missing edge {from} -> {to} in {view}"))
            .clone()
    };

    let expected_file_all = json!([
        {"kind": "calls", "links": 3},
        {"kind": "reads", "links": 2},
        {"kind": "extends", "links": 1},
        {"kind": "implements", "links": 1},
        {"kind": "imports", "links": 1},
        {"kind": "uses_const", "links": 1},
        {"kind": "writes", "links": 1},
    ]);
    let expected_file_certain = json!([
        {"kind": "calls", "links": 2},
        {"kind": "reads", "links": 2},
        {"kind": "extends", "links": 1},
        {"kind": "implements", "links": 1},
        {"kind": "imports", "links": 1},
        {"kind": "uses_const", "links": 1},
        {"kind": "writes", "links": 1},
    ]);
    let expected_sym_all = json!([
        {"kind": "calls", "links": 3},
        {"kind": "reads", "links": 2},
        {"kind": "uses_const", "links": 1},
        {"kind": "writes", "links": 1},
    ]);

    // Scope ("src" folder view & spread view with focus)
    let root_view = index.view("shop", "src", Some("src/ctrl"), false, "");
    assert_eq!(
        edge(&root_view, "src/ctrl", "src/svc")["kinds"],
        expected_file_all
    );
    assert_eq!(
        edge(
            &index.view("shop", "src", None, true, ""),
            "src/ctrl",
            "src/svc"
        )["kinds"],
        expected_file_certain
    );

    let nested = index.view_with(
        "shop",
        "src",
        &[String::from("src/ctrl"), String::from("src/svc")],
        Some(ctrl_file),
        false,
        "",
    );
    assert_eq!(
        edge(&nested, ctrl_file, svc_file)["kinds"],
        expected_file_all
    );
    assert_eq!(
        nested["focus"]["kinds"],
        json!([
            {"kind": "calls", "links": 3},
            {"kind": "reads", "links": 2},
            {"kind": "extends", "links": 1},
            {"kind": "implements", "links": 1},
            {"kind": "imports", "links": 1},
            {"kind": "uses_const", "links": 1},
            {"kind": "writes", "links": 1},
        ])
    );

    // Changes
    let changed_view = index.changes("shop", &[(ctrl_file.to_owned(), 'M')], "", None, false, "");
    assert_eq!(
        edge(&changed_view, ctrl_file, svc_file)["kinds"],
        expected_file_all
    );

    // Follow (file & symbol)
    let follow_files = index
        .follow(
            "shop",
            ctrl_file,
            &[ctrl_file.to_owned()],
            &[],
            &[],
            None,
            false,
        )
        .unwrap();
    assert_eq!(
        edge(&follow_files, ctrl_file, svc_file)["kinds"],
        expected_file_all
    );

    let handle_key = format!("{ctrl_file}#L7:handle()");
    let run_key = format!("{svc_file}#L8:run()");
    let follow_syms = index
        .follow(
            "shop",
            &handle_key,
            std::slice::from_ref(&handle_key),
            &[],
            &[],
            None,
            false,
        )
        .unwrap();
    assert_eq!(
        edge(&follow_syms, &handle_key, &run_key)["kinds"],
        expected_sym_all
    );

    // File symbol view
    let file_view = index
        .symbols("shop", ctrl_file, &[], None, None, false, "")
        .unwrap();
    assert_eq!(
        edge(&file_view, &handle_key, svc_file)["kinds"],
        expected_sym_all
    );

    // Cycle (folder and file)
    let folder_cycle = index.cycle("shop", "src/ctrl", &[], &|_| None, None, false);
    assert_eq!(
        edge(&folder_cycle, "src/ctrl", svc_file)["kinds"],
        expected_file_all
    );
    let file_cycle = index.cycle("shop", ctrl_file, &[], &|_| None, None, false);
    assert_eq!(
        edge(&file_cycle, ctrl_file, svc_file)["kinds"],
        expected_file_all
    );

    // Path
    let path_view = index
        .path("shop", ctrl_file, audit_file, &[], &|_| None, None, false)
        .unwrap();
    assert_eq!(
        edge(&path_view, ctrl_file, svc_file)["kinds"],
        expected_file_all
    );
    assert_eq!(
        edge(&path_view, svc_file, audit_file)["kinds"],
        json!([{"kind": "calls", "links": 1}, {"kind": "imports", "links": 1}])
    );

    // Shared
    let shared_view = index
        .shared(
            "shop",
            Sharing {
                from: ctrl_file,
                to: helper_file,
                users: Some(false),
                skip: 0,
            },
            &[],
            &|_| None,
            None,
            false,
            "",
        )
        .unwrap();
    assert_eq!(
        edge(&shared_view, ctrl_file, svc_file)["kinds"],
        expected_file_all
    );
    assert_eq!(
        edge(&shared_view, helper_file, svc_file)["kinds"],
        json!([{"kind": "calls", "links": 1}, {"kind": "reads", "links": 1}])
    );

    // Scenario
    let scenario_view = index.scenario("shop", ctrl_file, false, "", 0);
    assert_eq!(
        edge(&scenario_view, ctrl_file, svc_file)["kinds"],
        expected_file_all
    );
}

#[test]
fn maps_index_reports_cache_hit_and_telemetry_tracks_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let graph_path = dir.path().join("graph.json");
    std::fs::write(
        &graph_path,
        json!({
            "nodes": [
                {"id": "a", "label": "A", "source_file": "src/A.php"},
                {"id": "b", "label": "B", "source_file": "src/B.php"}
            ],
            "links": [
                {"source": "a", "target": "b", "relation": "calls", "confidence": "EXTRACTED"}
            ]
        })
        .to_string(),
    )
    .unwrap();

    let maps = Maps::default();
    let (_idx1, hit1) = maps
        .index_with_cache_hit("proj", &graph_path, dir.path())
        .unwrap();
    assert!(!hit1);
    let (_idx2, hit2) = maps
        .index_with_cache_hit("proj", &graph_path, dir.path())
        .unwrap();
    assert!(hit2);

    let mut view = json!({
        "matching": 10,
        "between": 5,
        "nodes": [{"name": "a"}, {"name": "b"}],
        "edges": [{"from": "a", "to": "b"}],
        "access": {"left": 2},
        "backbone": false
    });
    finalize_telemetry(&mut view, 17, true);
    assert_eq!(
        view["telemetry"],
        json!({
            "elapsed_ms": 17,
            "cache_hit": true,
            "shown": 2,
            "drawn": 1,
            "limited": true,
            "truncated": true
        })
    );

    let mut untruncated = json!({
        "mode": "changes",
        "matching": 0,
        "between": 1,
        "nodes": [{"name": "a", "changed": 1}],
        "edges": [{"from": "a", "to": "b"}],
        "change": {"hidden": 0},
        "backbone": true
    });
    finalize_telemetry(&mut untruncated, 5, false);
    assert_eq!(
        untruncated["telemetry"],
        json!({
            "elapsed_ms": 5,
            "cache_hit": false,
            "shown": 1,
            "drawn": 1,
            "limited": true,
            "truncated": false
        })
    );
}

#[test]
fn maps_and_index_expose_native_ir_backend_and_summary_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let project_dir = dir.path().join("project");
    std::fs::create_dir_all(project_dir.join("src")).unwrap();
    std::fs::write(
        project_dir.join("src/lib.rs"),
        "pub fn greet() -> i32 {\n    42\n}\n",
    )
    .unwrap();

    let maps = Maps::default();
    assert_eq!(maps.ir_summary(&state, "p1"), None);

    let ir = crate::graph_ir::GraphIrStore::open(&state, "p1").unwrap();
    ir.sync_workspace(&project_dir).unwrap();
    let live_graph = state.join("graphs/p1/live/graph.json");
    ir.write_live_graph_atomic(&live_graph).unwrap();

    let idx = maps.index("p1", &live_graph, &project_dir).unwrap();
    assert_eq!(idx.backend(), Some(crate::graph_ir::NATIVE_IR_TOOL));

    let mut view = idx.view("p1", "", None, false, "");
    assert_eq!(view["backend"], crate::graph_ir::NATIVE_IR_TOOL);

    let summary = maps.ir_summary(&state, "p1").expect("ir summary");
    assert_eq!(summary.files, 1);
    attach_ir_metadata(&mut view, idx.backend(), Some(&summary));
    assert_eq!(view["ir_summary"]["files"], 1);
    assert!(view["ir_summary"]["symbols"].as_u64().unwrap_or(0) >= 1);
}

#[test]
fn maps_cache_preserves_commit_entries_on_forget_live_and_pages_views() {
    let dir = tempfile::tempdir().unwrap();
    let make_graph = |name: &str, nodes: Vec<Value>| {
        let p = dir.path().join(name);
        std::fs::write(&p, json!({"nodes": nodes, "links": []}).to_string()).unwrap();
        p
    };
    let live_path = make_graph(
        "live.json",
        vec![json!({"id": "a", "label": "a", "source_file": "src/a.rs"})],
    );
    let rev_path = make_graph(
        "rev.json",
        vec![json!({"id": "b", "label": "b", "source_file": "src/b.rs"})],
    );

    let maps = Maps::default();
    let _ = maps.index("proj", &live_path, dir.path()).unwrap();
    let rev_idx = maps.index("proj@abc123", &rev_path, dir.path()).unwrap();
    maps.forget_live("proj");

    // Revision pool survives `forget_live("proj")`.
    let rev_hit = maps
        .load_with("proj@abc123", &rev_path, |_| {
            panic!("commit entry should survive forget_live")
        })
        .unwrap();
    assert!(Arc::ptr_eq(&rev_idx, &rev_hit));

    // Direct SQLite IR construction builds Index with persisted `is_test` without JSON.
    let ir = crate::graph_ir::GraphIrStore::open(&dir.path().join("state"), "direct").unwrap();
    let proj_dir = dir.path().join("repo");
    std::fs::create_dir_all(proj_dir.join("src")).unwrap();
    std::fs::write(
        proj_dir.join("src/a.rs"),
        "pub fn f() {}\n#[cfg(test)]\nmod t {\n    #[test]\n    fn t1() {}\n}\n",
    )
    .unwrap();
    std::fs::write(proj_dir.join("src/b.rs"), "pub fn g() {}\n").unwrap();
    std::fs::write(proj_dir.join("src/c.rs"), "pub fn h() {}\n").unwrap();
    ir.sync_workspace(&proj_dir).unwrap();
    let (ir_nodes, ir_links, ..) = ir.export_ir_graph_elements().unwrap();
    let idx = Index::from_ir_parts(
        &proj_dir,
        Some(ir.db_path().to_path_buf()),
        Some(crate::graph_ir::NATIVE_IR_TOOL.to_owned()),
        ir_nodes,
        ir_links,
    )
    .unwrap();
    assert_eq!(idx.without_tests().left_out().1, 1);

    let page1 = idx.view_with_whole_options(
        "direct",
        "src",
        &[],
        None,
        false,
        "",
        false,
        0,
        Some(2),
        Some(2),
    );
    assert_eq!(page1["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(page1["paging"]["has_more"], json!(true));
    assert_eq!(page1["paging"]["next_offset"], json!(2));
    assert_eq!(page1["depth"], json!(2));

    let page2 = idx.view_with_whole_options(
        "direct",
        "src",
        &[],
        None,
        false,
        "",
        false,
        2,
        Some(2),
        Some(2),
    );
    assert_eq!(page2["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(page2["paging"]["has_more"], json!(false));
}

#[test]
fn maps_lru_single_flight_telemetry_confidence_and_paging_fixes() {
    let dir = tempfile::tempdir().unwrap();
    let idx = |path: &str| -> Arc<Index> {
        let graph = json!({
            "nodes": [{"id": "n", "label": "n()", "source_file": path, "source_location": "L1"}],
            "links": []
        });
        Arc::new(Index::read(graph.to_string().as_bytes(), dir.path()).unwrap())
    };
    let maps = Maps::default();
    for i in 0..(KEPT_LIVE + 4) {
        let pid = format!("p{i}");
        maps.put_live_ir(&pid, 1, 0, idx("src/a.rs"));
    }
    assert!(
        maps.live_ir("p0").is_none(),
        "expected LRU eviction of oldest entry"
    );
    assert!(maps.live_ir(&format!("p{}", KEPT_LIVE + 3)).is_some());

    let loads = std::sync::atomic::AtomicUsize::new(0);
    let first = maps
        .get_or_load_live_ir("sf", 5, 1, || {
            loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Some(idx("src/sf.rs")))
        })
        .unwrap()
        .unwrap();
    assert!(!first.1);
    let second = maps
        .get_or_load_live_ir("sf", 5, 1, || {
            loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Some(idx("src/sf2.rs")))
        })
        .unwrap()
        .unwrap();
    assert!(second.1);
    assert_eq!(loads.load(std::sync::atomic::Ordering::SeqCst), 1);

    // Changing write_epoch invalidates cache even when ir_revision stays equal.
    let bumped = maps
        .get_or_load_live_ir("sf", 5, 2, || {
            loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Some(idx("src/sf_v2.rs")))
        })
        .unwrap()
        .unwrap();
    assert!(!bumped.1);
    assert_eq!(loads.load(std::sync::atomic::Ordering::SeqCst), 2);

    // `finalize_telemetry` must compare frame.places against frame.drawn, not graph-wide drawn.
    let mut frame_trunc = json!({
        "mode": null,
        "matching": 1,
        "between": 2,
        "nodes": [{"path": "src/a.rs"}],
        "edges": [{"from": "a", "to": "b"}, {"from": "b", "to": "c"}],
        "frames": [{"places": 5, "drawn": 1}],
    });
    finalize_telemetry(&mut frame_trunc, 3, true);
    assert_eq!(frame_trunc["telemetry"]["truncated"], true);

    // Focus hoisting preserves returned <= limit and populates cross_page_edges & confidence.
    let cross_graph = json!({
        "nodes": [
            {"id": "a", "label": "a()", "source_file": "src/a.rs", "source_location": "L1"},
            {"id": "b", "label": "b()", "source_file": "src/b.rs", "source_location": "L1"},
            {"id": "c", "label": "c()", "source_file": "src/c.rs", "source_location": "L1"}
        ],
        "links": [
            {"source": "a", "target": "b", "relation": "calls", "confidence": "EXTRACTED"},
            {"source": "b", "target": "c", "relation": "calls", "confidence": "INFERRED"}
        ]
    });
    let cidx = Index::read(cross_graph.to_string().as_bytes(), dir.path()).unwrap();
    let mut page = cidx.view_with_limits_opts(
        "cross",
        "src",
        &[],
        None,
        false,
        "",
        NORMAL_LIMITS,
        0,
        Some(1),
        None,
    );
    assert_eq!(page["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(page["paging"]["returned"], 1);
    assert_eq!(page["paging"]["next_offset"], 1);
    assert!(
        !page["paging"]["cross_page_edges"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    attach_confidence_metadata(&cidx, &mut page);
    assert_eq!(page["confidence"]["extracted"], 1);
    assert_eq!(page["confidence"]["inferred"], 1);
    assert_eq!(page["confidence"]["ratio"], 0.5);

    // Cycle cut_hints emitted on Index::cycle.
    let cyc_graph = json!({
        "nodes": [
            {"id": "a", "label": "a()", "source_file": "src/a.rs", "source_location": "L1"},
            {"id": "b", "label": "b()", "source_file": "src/b.rs", "source_location": "L1"}
        ],
        "links": [
            {"source": "a", "target": "b", "relation": "calls", "confidence": "EXTRACTED"},
            {"source": "b", "target": "a", "relation": "calls", "confidence": "INFERRED"}
        ]
    });
    let cyc_idx = Index::read(cyc_graph.to_string().as_bytes(), dir.path()).unwrap();
    let cyc = cyc_idx.cycle("cyc", "src/a.rs", &[], &|_| None, None, false);
    let hints = cyc["cycle"]["cut_hints"]
        .as_array()
        .expect("cycle cut_hints");
    assert!(!hints.is_empty());
    assert_eq!(hints[0]["breaks_cycles"], true);
}

#[test]
fn lens_pagination_follow_all_seen_lod_min_weight_diff_overlay_and_sem_hubs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("crates/alpha/src")).unwrap();
    std::fs::create_dir_all(root.join("crates/beta/src")).unwrap();
    std::fs::write(
        root.join("crates/alpha/Cargo.toml"),
        "[package]\nname=\"alpha\"\n[dependencies]\nbeta = { path = \"../beta\" }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("crates/beta/Cargo.toml"),
        "[package]\nname=\"beta\"\n[dependencies]\nalpha = { path = \"../alpha\" }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("crates/alpha/src/a.rs"),
        "pub fn a1() {}\npub fn a2() {}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("crates/beta/src/b.rs"),
        "pub fn b1() {}\npub fn b2() {}\n",
    )
    .unwrap();
    std::fs::write(root.join("crates/beta/src/c.rs"), "pub fn c1() {}\n").unwrap();

    // Semantic route hub anchored in `a.rs` linked from `b.rs` must not inflate code `pairs`.
    let g = json!({
        "nodes": [
            {"id": "a1", "label": "a1()", "source_file": "crates/alpha/src/a.rs", "source_location": "L1"},
            {"id": "a2", "label": "a2()", "source_file": "crates/alpha/src/a.rs", "source_location": "L2"},
            {"id": "b1", "label": "b1()", "source_file": "crates/beta/src/b.rs", "source_location": "L1"},
            {"id": "c1", "label": "c1()", "source_file": "crates/beta/src/c.rs", "source_location": "L1"},
            {"id": "sem:route:GET /api/orders/{param}", "label": "route:GET /api/orders/{param}", "node_kind": "semantic", "source_file": "crates/alpha/src/a.rs", "source_location": "L1"}
        ],
        "links": [
            {"source": "a1", "target": "b1", "relation": "calls", "confidence": "EXTRACTED"},
            {"source": "a2", "target": "b1", "relation": "calls", "confidence": "EXTRACTED"},
            {"source": "b1", "target": "c1", "relation": "calls", "confidence": "EXTRACTED"},
            {"source": "c1", "target": "a1", "relation": "calls", "confidence": "EXTRACTED"},
            {"source": "b1", "target": "sem:route:GET /api/orders/{param}", "relation": "calls_route", "confidence": "EXTRACTED"}
        ]
    });
    let idx = Index::read(g.to_string().as_bytes(), root).unwrap();
    // `b.rs -> a.rs` did NOT happen directly (only via `sem:route:*`), so no `b.rs -> a.rs` code pair exists.
    let b_id = idx
        .files
        .iter()
        .position(|f| f == "crates/beta/src/b.rs")
        .unwrap() as u32;
    let a_id = idx
        .files
        .iter()
        .position(|f| f == "crates/alpha/src/a.rs")
        .unwrap() as u32;
    assert!(
        !idx.pairs.iter().any(|(f, t, _)| *f == b_id && *t == a_id),
        "sem:* contract hub must not create spurious code pair onto anchor_file"
    );

    // `paginate_lens_view`: offset > 0 pins focus anchor, offset past end sets has_more=false, cross_page_edges sorted desc.
    let mut obj = serde_json::Map::new();
    obj.insert(
        "nodes".into(),
        json!([
            {"path": "n1", "name": "n1"},
            {"path": "anchor", "name": "anchor"},
            {"path": "n2", "name": "n2"}
        ]),
    );
    obj.insert(
        "edges".into(),
        json!([
            {"from": "anchor", "to": "n1", "links": 1},
            {"from": "anchor", "to": "n2", "links": 9}
        ]),
    );
    obj.insert("focus".into(), json!({"path": "anchor"}));
    paginate_lens_view(&mut obj, 1, Some(1), &[]);
    assert_eq!(obj["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(obj["paging"]["returned"], 1);
    assert_eq!(obj["paging"]["has_more"], true);
    let mut past_end = serde_json::Map::new();
    past_end.insert("nodes".into(), json!([{"path": "a"}, {"path": "b"}]));
    paginate_lens_view(&mut past_end, 10, Some(5), &[]);
    assert_eq!(past_end["paging"]["returned"], 0);
    assert_eq!(past_end["paging"]["has_more"], false);

    // Descending cross_page_edges check when anchor kept & edge weights vary.
    let mut weighted = serde_json::Map::new();
    weighted.insert(
        "nodes".into(),
        json!([{"path": "a"}, {"path": "b"}, {"path": "c"}]),
    );
    weighted.insert(
        "edges".into(),
        json!([
            {"from": "a", "to": "b", "links": 2},
            {"from": "a", "to": "c", "links": 10}
        ]),
    );
    paginate_lens_view(&mut weighted, 0, Some(1), &["a"]);
    assert_eq!(weighted["paging"]["cross_page_edges"][0]["links"], 10);

    // `follow_with_depth`: out-of-page focus remains in `view["focus"]` and `more_out`/`more_in` accurate.
    let followed = idx
        .follow_with_depth(
            "demo",
            "crates/alpha/src/a.rs",
            &[],
            &[],
            &[],
            &|_| None,
            Some("crates/beta/src/c.rs"),
            false,
            Some(2),
            0,
            Some(1),
        )
        .expect("follow view");
    assert_eq!(followed["focus"]["path"], "crates/beta/src/c.rs");

    // `Lod::Crate` + `min_weight`: aggregates crate buckets and drops links < 2.
    let mut crate_view = idx.view("demo", "", None, false, "");
    let reshaped = apply_lod_and_min_weight(
        &idx,
        &mut crate_view,
        Some(Lod::Crate),
        Some(2),
        "",
        None,
        "",
        false,
        0,
        None,
    );
    assert!(reshaped);
    assert_eq!(crate_view["lod"], "crate");
    assert_eq!(crate_view["min_weight"], 2);
    let crate_nodes: Vec<&str> = crate_view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["path"].as_str())
        .collect();
    assert!(crate_nodes.contains(&"crates/alpha"), "{crate_nodes:?}");
    assert!(crate_nodes.contains(&"crates/beta"), "{crate_nodes:?}");
    assert!(
        crate_view["edges"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["links"].as_u64().unwrap_or(0) >= 2)
    );

    // `view_at_lod` respects `(offset, limit)` pagination and emits `paging` metadata.
    let paged_lod = view_at_lod(
        &idx,
        "demo",
        Lod::Crate,
        None,
        &LodFilter {
            scope: "",
            focus: None,
            filter: "",
            certain: false,
            offset: 1,
            limit: Some(1),
        },
    );
    assert_eq!(paged_lod["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(paged_lod["paging"]["offset"], 1);
    assert_eq!(paged_lod["paging"]["limit"], 1);
    assert_eq!(paged_lod["paging"]["returned"], 1);
    assert!(paged_lod["paging"]["total"].as_u64().unwrap_or(0) >= 2);

    // `Lod::File` & `Lod::Symbol` views build non-empty nodes/edges cleanly, and focusing a
    // symbol at `Lod::Symbol` retains its 1-hop incoming/outgoing neighbors.
    let mut sym_view = idx.view("demo", "", None, false, "");
    assert!(apply_lod_and_min_weight(
        &idx,
        &mut sym_view,
        Some(Lod::Symbol),
        None,
        "",
        None,
        "",
        false,
        0,
        None,
    ));
    assert_eq!(sym_view["lod"], "symbol");
    let sym_paths: Vec<&str> = sym_view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["path"].as_str())
        .collect();
    assert!(!sym_paths.is_empty());
    let target_key = sym_paths
        .iter()
        .find(|p| p.contains("crates/beta/src/c.rs#"))
        .copied()
        .expect("beta symbol key");
    let sym_focus_view = view_at_lod(
        &idx,
        "demo",
        Lod::Symbol,
        None,
        &LodFilter {
            scope: "",
            focus: Some(target_key),
            filter: "",
            certain: false,
            offset: 0,
            limit: None,
        },
    );
    assert!(
        sym_focus_view["nodes"].as_array().unwrap().len() >= 2,
        "focused symbol should include 1-hop neighbors: {sym_focus_view}"
    );
    assert!(
        !sym_focus_view["edges"].as_array().unwrap().is_empty(),
        "focused symbol should include incident edges: {sym_focus_view}"
    );

    // `structural_diff_overlay`: detects added edge and SCC introduction vs base graph.
    let base_g = json!({
        "nodes": [
            {"id": "a1", "label": "a1()", "source_file": "crates/alpha/src/a.rs", "source_location": "L1"},
            {"id": "b1", "label": "b1()", "source_file": "crates/beta/src/b.rs", "source_location": "L1"}
        ],
        "links": [
            {"source": "a1", "target": "b1", "relation": "calls", "confidence": "EXTRACTED"}
        ]
    });
    let base_idx = Index::read(base_g.to_string().as_bytes(), root).unwrap();
    let diff = structural_diff_overlay(
        &idx,
        Some(&base_idx),
        &["crates/beta/src/c.rs".to_owned()],
        &HashMap::new(),
        false,
        None,
    );
    assert_eq!(diff["base_available"], true);
    assert!(diff["added_edges"]["count"].as_u64().unwrap() >= 2);
    assert!(!diff["sccs_introduced"].as_array().unwrap().is_empty());

    // `PersistentMemberStore` stamp invalidation on disk edit.
    let mut mc1 = FileMemberCache::new(&idx);
    let m1_len = mc1.declared("crates/alpha/src/a.rs").len();
    assert_eq!(m1_len, 2);
    std::fs::write(
        root.join("crates/alpha/src/a.rs"),
        "pub fn a1() {}\npub fn a2() {}\npub fn a3() {}\n",
    )
    .unwrap();
    let mut mc2 = FileMemberCache::new(&idx);
    let m2_len = mc2.declared("crates/alpha/src/a.rs").len();
    assert_eq!(m2_len, 3);
}

#[test]
fn lod_folder_root_manifest_rollup_marks_end_line_and_racy_stat() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("packages/api/src")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"root\"\n").unwrap();
    std::fs::write(
        root.join("packages/api/composer.json"),
        "{\"name\":\"acme/api\"}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/main.rs"),
        "pub fn entry() {}\npub fn stay() {}\n",
    )
    .unwrap();
    // Dedented interior line at L6 inside helper (L2..=L8) tests `end_line` vs indent fallback.
    std::fs::write(
        root.join("packages/api/src/Handler.php"),
        "<?php\nfunction helper() {\n    $s = <<<TXT\nline4\nline5\ndedented_interior\nTXT;\n}\n\n\n\nfunction trailing() {}\n",
    )
    .unwrap();

    let ir_nodes = vec![
        crate::graph_ir::IrNodeSpec {
            id: "file:src/main.rs".into(),
            label: "main.rs".into(),
            source_file: "src/main.rs".into(),
            source_location: None,
            end_line: 0,
            file_type: Some("code".into()),
            node_kind: Some("page".into()),
            community: None,
            community_name: None,
            callable_class: false,
            is_abstract: false,
            is_test: false,
        },
        crate::graph_ir::IrNodeSpec {
            id: "fn:entry".into(),
            label: "entry()".into(),
            source_file: "src/main.rs".into(),
            source_location: Some("L1".into()),
            end_line: 1,
            file_type: None,
            node_kind: Some("function".into()),
            community: None,
            community_name: None,
            callable_class: false,
            is_abstract: false,
            is_test: false,
        },
        crate::graph_ir::IrNodeSpec {
            id: "file:packages/api/src/Handler.php".into(),
            label: "Handler.php".into(),
            source_file: "packages/api/src/Handler.php".into(),
            source_location: None,
            end_line: 0,
            file_type: Some("code".into()),
            node_kind: Some("page".into()),
            community: None,
            community_name: None,
            callable_class: false,
            is_abstract: false,
            is_test: false,
        },
        crate::graph_ir::IrNodeSpec {
            id: "fn:helper".into(),
            label: "helper()".into(),
            source_file: "packages/api/src/Handler.php".into(),
            source_location: Some("L2".into()),
            end_line: 8,
            file_type: None,
            node_kind: Some("function".into()),
            community: None,
            community_name: None,
            callable_class: false,
            is_abstract: false,
            is_test: false,
        },
        crate::graph_ir::IrNodeSpec {
            id: "fn:trailing".into(),
            label: "trailing()".into(),
            source_file: "packages/api/src/Handler.php".into(),
            source_location: Some("L12".into()),
            end_line: 12,
            file_type: None,
            node_kind: Some("function".into()),
            community: None,
            community_name: None,
            callable_class: false,
            is_abstract: false,
            is_test: false,
        },
    ];
    let ir_links = vec![crate::graph_ir::IrLinkSpec {
        source: "fn:entry".into(),
        target: "fn:helper".into(),
        relation: Some("calls".into()),
        confidence: Some("EXTRACTED".into()),
        source_file: Some("src/main.rs".into()),
        context: None,
        evidence: None,
    }];
    let idx = Index::from_ir_parts(root, None, None, ir_nodes, ir_links).unwrap();

    // 1. Root manifest `""` discovery + Folder LOD + mark preservation across rollup_view_to_lod.
    let manifests = idx.manifest_dirs();
    assert!(manifests.iter().any(|m| m.is_empty()), "{manifests:?}");
    assert!(
        manifests.iter().any(|m| m == "packages/api"),
        "{manifests:?}"
    );

    let mut folder_view = idx.view("demo", "", None, false, "");
    assert!(lod::apply_lod_and_min_weight(
        &idx,
        &mut folder_view,
        Some(Lod::Folder),
        None,
        "",
        None,
        "",
        false,
        0,
        None,
    ));
    assert_eq!(folder_view["lod"], "folder");
    let folder_paths: Vec<&str> = folder_view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["path"].as_str())
        .collect();
    assert!(folder_paths.contains(&"src"), "{folder_paths:?}");
    assert!(
        folder_paths.contains(&"packages/api/src"),
        "{folder_paths:?}"
    );

    // Rollup preserves blast/circle/patterns/changed marks on nodes and violation/detour on edges.
    let mut marked_view = json!({
        "mode": "map",
        "scope": "",
        "nodes": [
            {"path": "src/main.rs", "name": "main.rs", "folder": false, "blast": {"hop": 2, "left": 1.0}, "circle": 1, "changed": 1, "patterns": 1},
            {"path": "packages/api/src/Handler.php", "name": "Handler.php", "folder": false, "suspect": 1}
        ],
        "edges": [
            {"from": "src/main.rs", "to": "packages/api/src/Handler.php", "links": 2, "architecture_violation": 2, "detour": true}
        ]
    });
    assert!(lod::rollup_view_to_lod(&idx, &mut marked_view, Lod::Crate));
    let rolled_nodes = marked_view["nodes"].as_array().unwrap();
    let root_bucket = rolled_nodes.iter().find(|n| n["path"] == ".").unwrap();
    assert_eq!(root_bucket["blast"]["hop"], 2);
    assert_eq!(root_bucket["circle"], 1);
    assert_eq!(root_bucket["changed"], 1);
    assert_eq!(root_bucket["patterns"], 1);
    let rolled_edge = &marked_view["edges"].as_array().unwrap()[0];
    assert_eq!(rolled_edge["architecture_violation"], 2);
    assert_eq!(rolled_edge["detour"], true);

    // 2. `end_line` accurate span in `access::lines_of` and `file_inspect::marks`.
    let handler_file = idx.opened("packages/api/src/Handler.php").unwrap();
    let helper_sym = idx
        .range(handler_file)
        .find(|&s| idx.label(s) == "helper()")
        .unwrap();
    assert_eq!(idx.lines_of(helper_sym), 2..9);

    let text = std::fs::read_to_string(root.join("packages/api/src/Handler.php")).unwrap();
    let edited = Edited {
        new: false,
        hunks: vec![Hunk {
            at: 7,
            added: 1,
            removed: 1,
            blank: false,
        }],
        text,
    };
    let marks = idx.marks(handler_file, &edited);
    let helper_slot = (helper_sym - idx.first[handler_file as usize]) as usize;
    let file_sym = idx
        .range(handler_file)
        .find(|&s| idx.symbols[s as usize].kind == Kind::File)
        .unwrap();
    let file_slot = (file_sym - idx.first[handler_file as usize]) as usize;
    assert_eq!(
        marks.status[helper_slot],
        Some('M'),
        "expected L7 edit inside L2..=L8 helper"
    );
    assert_eq!(
        marks.status[file_slot], None,
        "L7 edit should not leak to top/file node"
    );

    // 3. `STAT_RACY_NS` guard refuses caching same-second/same-length rewrite of `src/main.rs`.
    let mut mc1 = FileMemberCache::new(&idx);
    let names1: Vec<String> = mc1
        .declared("src/main.rs")
        .iter()
        .map(|m| m.name.clone())
        .collect();
    assert_eq!(names1, vec!["entry", "stay"]);
    std::fs::write(
        root.join("src/main.rs"),
        "pub fn entry() {}\npub fn swap() {}\n",
    )
    .unwrap();
    let mut mc2 = FileMemberCache::new(&idx);
    let names2: Vec<String> = mc2
        .declared("src/main.rs")
        .iter()
        .map(|m| m.name.clone())
        .collect();
    assert_eq!(names2, vec!["entry", "swap"]);
}

#[test]
fn update_files_preserves_communities_and_exact_link_unplaced_totals() {
    use crate::graph_ir::{IrDeltaElements, IrLinkSpec, IrNodeSpec};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("auth")).unwrap();
    std::fs::create_dir_all(root.join("billing")).unwrap();

    let node = |id: &str, label: &str, file: &str, comm: u32, cname: &str| IrNodeSpec {
        id: id.to_owned(),
        label: label.to_owned(),
        source_file: file.to_owned(),
        source_location: Some("L1".to_owned()),
        end_line: 10,
        file_type: Some("code".to_owned()),
        node_kind: Some("function".to_owned()),
        community: Some(comm),
        community_name: Some(cname.to_owned()),
        callable_class: false,
        is_abstract: false,
        is_test: false,
    };
    let mut page_auth = node("file:auth/a.rs", "a.rs", "auth/a.rs", 0, "auth");
    page_auth.node_kind = Some("page".to_owned());
    let mut page_bill = node("file:billing/b.rs", "b.rs", "billing/b.rs", 1, "billing");
    page_bill.node_kind = Some("page".to_owned());
    let mut class_auth = node("cls:A", "Svc", "auth/a.rs", 0, "auth");
    class_auth.node_kind = Some("class".to_owned());
    class_auth.callable_class = true;
    let sym_a1 = node("fn:a1", ".run()", "auth/a.rs", 0, "auth");
    let sym_b1 = node("fn:b1", "charge()", "billing/b.rs", 1, "billing");

    let initial_nodes = vec![
        page_auth.clone(),
        class_auth.clone(),
        sym_a1.clone(),
        page_bill.clone(),
        sym_b1.clone(),
    ];
    let initial_links = vec![
        // Nested method link (stored in parent).
        IrLinkSpec {
            source: "cls:A".to_owned(),
            target: "fn:a1".to_owned(),
            relation: Some("method".to_owned()),
            confidence: None,
            source_file: Some("auth/a.rs".to_owned()),
            context: None,
            evidence: None,
        },
        // Structural link whose parent is a File node (parent reset to NONE in finish_builder!).
        IrLinkSpec {
            source: "file:auth/a.rs".to_owned(),
            target: "cls:A".to_owned(),
            relation: Some("contains".to_owned()),
            confidence: None,
            source_file: Some("auth/a.rs".to_owned()),
            context: None,
            evidence: None,
        },
        // Self-loop and underscore-prefixed link in auth/a.rs.
        IrLinkSpec {
            source: "fn:a1".to_owned(),
            target: "fn:a1".to_owned(),
            relation: Some("calls".to_owned()),
            confidence: Some("EXTRACTED".to_owned()),
            source_file: Some("auth/a.rs".to_owned()),
            context: None,
            evidence: None,
        },
        IrLinkSpec {
            source: "fn:a1".to_owned(),
            target: "fn:b1".to_owned(),
            relation: Some("_debug".to_owned()),
            confidence: None,
            source_file: Some("auth/a.rs".to_owned()),
            context: None,
            evidence: None,
        },
        // Normal cross-file link.
        IrLinkSpec {
            source: "fn:b1".to_owned(),
            target: "fn:a1".to_owned(),
            relation: Some("calls".to_owned()),
            confidence: Some("EXTRACTED".to_owned()),
            source_file: Some("billing/b.rs".to_owned()),
            context: None,
            evidence: None,
        },
        // Unplaced cross-file link (source_file matches neither auth/a.rs nor billing/b.rs).
        IrLinkSpec {
            source: "fn:a1".to_owned(),
            target: "fn:b1".to_owned(),
            relation: Some("calls".to_owned()),
            confidence: Some("EXTRACTED".to_owned()),
            source_file: Some("other/ghost.rs".to_owned()),
            context: None,
            evidence: None,
        },
    ];

    let full1 = Index::from_ir_parts(
        root,
        None,
        None,
        initial_nodes.clone(),
        initial_links.clone(),
    )
    .unwrap();
    assert_eq!(full1.links, 6);
    assert_eq!(full1.unplaced, 1);

    // Re-apply an incremental edit on auth/a.rs where billing/b.rs is a dirty neighbor;
    // omit community fields on the delta nodes to verify old-file community fallback too.
    let mut edited_page_auth = page_auth;
    edited_page_auth.community = None;
    edited_page_auth.community_name = None;
    let mut edited_class_auth = class_auth;
    edited_class_auth.community = None;
    edited_class_auth.community_name = None;
    let mut edited_a1 = sym_a1;
    edited_a1.community = None;
    edited_a1.community_name = None;

    let delta = IrDeltaElements {
        files: vec!["auth/a.rs".to_owned(), "billing/b.rs".to_owned()],
        edited_files: BTreeSet::from(["auth/a.rs".to_owned()]),
        dirty_files: BTreeSet::from(["auth/a.rs".to_owned(), "billing/b.rs".to_owned()]),
        removed_files: BTreeSet::new(),
        neighbor_files: vec!["billing/b.rs".to_owned()],
        rs_touched: false,
        node_specs: vec![edited_page_auth, edited_class_auth, edited_a1],
        link_specs: initial_links.clone(),
    };

    let inc = full1.update_files(root, delta).unwrap();
    assert_eq!(inc.totals(), full1.totals());
    assert_eq!(
        inc.modules("proj", None, false, ""),
        full1.modules("proj", None, false, "")
    );
}

#[test]
fn lod_symbol_preserves_focus_blast_marks_architecture_and_discovers_extended_manifests() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let state = dir.path().join("state");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("packages/swift-mod/Sources")).unwrap();
    std::fs::create_dir_all(root.join("services/py-svc")).unwrap();
    std::fs::write(root.join("tsconfig.json"), "{}\n").unwrap();
    std::fs::write(root.join("packages/swift-mod/Package.swift"), "// swift\n").unwrap();
    std::fs::write(root.join("services/py-svc/setup.py"), "setup()\n").unwrap();
    std::fs::write(root.join("src/api.ts"), "export function apiCall() {}\n").unwrap();
    std::fs::write(root.join("src/core.ts"), "export function coreLeaf() {}\n").unwrap();
    std::fs::write(
        root.join("packages/swift-mod/Sources/Mod.swift"),
        "func swiftHelper() {}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("services/py-svc/app.py"),
        "def py_entry(): pass\n",
    )
    .unwrap();

    let ir_nodes = vec![
        crate::graph_ir::IrNodeSpec {
            id: "file:src/api.ts".into(),
            label: "api.ts".into(),
            source_file: "src/api.ts".into(),
            source_location: None,
            end_line: 0,
            file_type: Some("code".into()),
            node_kind: Some("page".into()),
            community: None,
            community_name: None,
            callable_class: false,
            is_abstract: false,
            is_test: false,
        },
        crate::graph_ir::IrNodeSpec {
            id: "fn:api".into(),
            label: "apiCall()".into(),
            source_file: "src/api.ts".into(),
            source_location: Some("L1".into()),
            end_line: 1,
            file_type: None,
            node_kind: Some("function".into()),
            community: None,
            community_name: None,
            callable_class: false,
            is_abstract: false,
            is_test: false,
        },
        crate::graph_ir::IrNodeSpec {
            id: "file:src/core.ts".into(),
            label: "core.ts".into(),
            source_file: "src/core.ts".into(),
            source_location: None,
            end_line: 0,
            file_type: Some("code".into()),
            node_kind: Some("page".into()),
            community: None,
            community_name: None,
            callable_class: false,
            is_abstract: false,
            is_test: false,
        },
        crate::graph_ir::IrNodeSpec {
            id: "fn:core".into(),
            label: "coreLeaf()".into(),
            source_file: "src/core.ts".into(),
            source_location: Some("L1".into()),
            end_line: 1,
            file_type: None,
            node_kind: Some("function".into()),
            community: None,
            community_name: None,
            callable_class: false,
            is_abstract: false,
            is_test: false,
        },
        crate::graph_ir::IrNodeSpec {
            id: "file:packages/swift-mod/Sources/Mod.swift".into(),
            label: "Mod.swift".into(),
            source_file: "packages/swift-mod/Sources/Mod.swift".into(),
            source_location: None,
            end_line: 0,
            file_type: Some("code".into()),
            node_kind: Some("page".into()),
            community: None,
            community_name: None,
            callable_class: false,
            is_abstract: false,
            is_test: false,
        },
        crate::graph_ir::IrNodeSpec {
            id: "file:services/py-svc/app.py".into(),
            label: "app.py".into(),
            source_file: "services/py-svc/app.py".into(),
            source_location: None,
            end_line: 0,
            file_type: Some("code".into()),
            node_kind: Some("page".into()),
            community: None,
            community_name: None,
            callable_class: false,
            is_abstract: false,
            is_test: false,
        },
    ];
    let ir_links = vec![crate::graph_ir::IrLinkSpec {
        source: "fn:api".into(),
        target: "fn:core".into(),
        relation: Some("calls".into()),
        confidence: Some("EXTRACTED".into()),
        source_file: Some("src/api.ts".into()),
        context: None,
        evidence: None,
    }];
    let idx = Index::from_ir_parts(&root, None, None, ir_nodes, ir_links).unwrap();

    // Extended manifest discovery finds root tsconfig.json, Package.swift, and setup.py,
    // and root "." bucket keeps `folder: true` at Crate LOD.
    let manifests = idx.manifest_dirs();
    assert!(manifests.iter().any(|m| m.is_empty()), "{manifests:?}");
    assert!(
        manifests.iter().any(|m| m == "packages/swift-mod"),
        "{manifests:?}"
    );
    assert!(
        manifests.iter().any(|m| m == "services/py-svc"),
        "{manifests:?}"
    );

    let crate_view = idx.view_at_lod_unfiltered(
        "demo",
        Lod::Crate,
        &LodFilter {
            scope: "",
            focus: None,
            filter: "",
            certain: false,
            offset: 0,
            limit: None,
        },
    );
    let root_node = crate_view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["path"] == ".")
        .expect("root '.' bucket node");
    assert_eq!(root_node["folder"], true, "{root_node}");

    // `Lod::Symbol` preserves `view["focus"]` for `mark_blast`, and `mark_architecture` marks symbol edges.
    let rules = crate::graph_map::layer_rules::Rules::parse(
        serde_json::json!({
            "version": 1,
            "layers": [
                {"id": "api", "name": "API", "paths": ["src/api.ts"], "allows": []},
                {"id": "core", "name": "Core", "paths": ["src/core.ts"], "allows": []}
            ]
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let core_focus = "src/core.ts#L1:coreLeaf()";
    let mut sym_view = idx.view_at_lod_unfiltered(
        "demo",
        Lod::Symbol,
        &LodFilter {
            scope: "",
            focus: Some(core_focus),
            filter: "",
            certain: false,
            offset: 0,
            limit: None,
        },
    );
    assert_eq!(sym_view["lod"], "symbol");
    assert_eq!(sym_view["focus"]["path"], core_focus);
    idx.mark_architecture(&mut sym_view, false, Some(&rules));
    idx.mark_blast(&mut sym_view);
    assert!(
        sym_view["focus"]["blast"]["files"].as_u64().unwrap_or(0) >= 1,
        "expected mark_blast at symbol LOD: {}",
        sym_view["focus"]
    );
    assert!(
        sym_view["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["architecture_violation"].as_u64().unwrap_or(0) >= 1),
        "expected architecture_violation on symbol LOD edge: {}",
        sym_view["edges"]
    );

    // `Maps::ir_summary` reads via `open_readonly_cached` cleanly.
    let ir_store = crate::graph_ir::GraphIrStore::open(&state, "cache-ro").unwrap();
    ir_store.sync_workspace(&root).unwrap();
    let maps = Maps::default();
    let summary = maps
        .ir_summary(&state, "cache-ro")
        .expect("cached ir_summary");
    assert!(summary.files >= 2);
}

#[test]
fn architecture_rules_support_forbids_package_boundaries_cycle_budgets_and_semantic_rules() {
    use crate::graph_ir::{IrLinkSpec, IrNodeSpec};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let bad = crate::graph_map::layer_rules::Rules::parse(
        br#"{"version":1,"layers":[{"id":"api","name":"API","paths":["src/api/**"],"forbids":["ghost"]}]}"#,
    );
    assert!(
        bad.is_err(),
        "unknown layer ID in `forbids` must be rejected"
    );

    let rules_json = serde_json::json!({
        "version": 1,
        "enforce_crate_boundaries": true,
        "max_cycles": 0,
        "layers": [
            {
                "id": "controller",
                "name": "Controllers",
                "paths": ["app/Controllers/**"],
                "forbids": ["ui"],
                "forbids_semantic": ["writes_table"],
                "max_cycles": 0
            },
            {
                "id": "service",
                "name": "Services",
                "paths": ["app/Services/**"]
            },
            {
                "id": "ui",
                "name": "UI",
                "paths": ["app/Ui/**"],
                "allows": []
            }
        ],
        "package_rules": [
            {
                "id": "ui-pkg",
                "paths": ["packages/ui/**"],
                "forbids": ["db-pkg"]
            },
            {
                "id": "db-pkg",
                "paths": ["packages/db/**"]
            }
        ],
        "semantic_rules": [
            {
                "id": "no-ui-routes",
                "from": "app/Ui/**",
                "relation": "exposes_route",
                "target": "*",
                "effect": "forbid"
            }
        ]
    });
    let rules = crate::graph_map::layer_rules::Rules::parse(rules_json.to_string().as_bytes())
        .expect("parse extended rules");

    let mk_file = |path: &str| IrNodeSpec {
        id: format!("file:{path}"),
        label: path.rsplit('/').next().unwrap_or(path).to_owned(),
        source_file: path.to_owned(),
        source_location: None,
        end_line: 0,
        file_type: Some("code".into()),
        node_kind: Some("page".into()),
        community: None,
        community_name: None,
        callable_class: false,
        is_abstract: false,
        is_test: false,
    };
    let mk_fn = |id: &str, path: &str, name: &str| IrNodeSpec {
        id: id.to_owned(),
        label: format!("{name}()"),
        source_file: path.to_owned(),
        source_location: Some("L1".into()),
        end_line: 1,
        file_type: None,
        node_kind: Some("function".into()),
        community: None,
        community_name: None,
        callable_class: false,
        is_abstract: false,
        is_test: false,
    };

    let nodes = vec![
        mk_file("app/Controllers/OrderController.php"),
        mk_fn("fn:ctrl", "app/Controllers/OrderController.php", "store"),
        mk_file("app/Services/OrderService.php"),
        mk_fn("fn:svc", "app/Services/OrderService.php", "handle"),
        mk_file("app/Ui/Button.php"),
        mk_fn("fn:ui", "app/Ui/Button.php", "render"),
        mk_file("packages/ui/view.ts"),
        mk_fn("fn:pkg_ui", "packages/ui/view.ts", "renderView"),
        mk_file("packages/db/client.ts"),
        mk_fn("fn:pkg_db", "packages/db/client.ts", "queryDb"),
        IrNodeSpec {
            id: "sem:table:orders".into(),
            label: "table:orders".into(),
            source_file: "app/Services/OrderService.php".into(),
            source_location: Some("L2".into()),
            end_line: 0,
            file_type: None,
            node_kind: Some("semantic".into()),
            community: None,
            community_name: None,
            callable_class: false,
            is_abstract: false,
            is_test: false,
        },
        IrNodeSpec {
            id: "sem:route:GET /bad".into(),
            label: "route:GET /bad".into(),
            source_file: "app/Ui/Button.php".into(),
            source_location: Some("L2".into()),
            end_line: 0,
            file_type: None,
            node_kind: Some("semantic".into()),
            community: None,
            community_name: None,
            callable_class: false,
            is_abstract: false,
            is_test: false,
        },
    ];
    let mk_call = |src: &str, dst: &str, file: &str| IrLinkSpec {
        source: src.into(),
        target: dst.into(),
        relation: Some("calls".into()),
        confidence: Some("EXTRACTED".into()),
        source_file: Some(file.into()),
        context: None,
        evidence: None,
    };
    let links = vec![
        // Allowed: controller -> service (since controller only `forbids: ["ui"]`).
        mk_call("fn:ctrl", "fn:svc", "app/Controllers/OrderController.php"),
        // Layer `forbids` violation + cycle: controller -> ui and ui -> controller.
        mk_call("fn:ctrl", "fn:ui", "app/Controllers/OrderController.php"),
        mk_call("fn:ui", "fn:ctrl", "app/Ui/Button.php"),
        // Package boundary violation: ui-pkg -> db-pkg.
        mk_call("fn:pkg_ui", "fn:pkg_db", "packages/ui/view.ts"),
        // Semantic violation: controller -> writes_table.
        IrLinkSpec {
            source: "fn:ctrl".into(),
            target: "sem:table:orders".into(),
            relation: Some("writes_table".into()),
            confidence: Some("EXTRACTED".into()),
            source_file: Some("app/Controllers/OrderController.php".into()),
            context: Some("writes_table".into()),
            evidence: Some("DB::table('orders')->insert".into()),
        },
        // Top-level semantic_rules violation: app/Ui/** -> exposes_route.
        IrLinkSpec {
            source: "fn:ui".into(),
            target: "sem:route:GET /bad".into(),
            relation: Some("exposes_route".into()),
            confidence: Some("EXTRACTED".into()),
            source_file: Some("app/Ui/Button.php".into()),
            context: Some("exposes_route".into()),
            evidence: Some("Route::get('/bad')".into()),
        },
    ];

    let idx = Index::from_ir_parts(root, None, None, nodes, links).unwrap();
    let violations = idx.architecture_violations("", false, &rules);
    assert!(
        violations.iter().any(|v| v.kind == "layer"
            && v.from == "app/Controllers/OrderController.php"
            && v.to == "app/Ui/Button.php"),
        "expected layer forbids violation: {violations:?}"
    );
    assert!(
        !violations.iter().any(|v| v.kind == "layer"
            && v.from == "app/Controllers/OrderController.php"
            && v.to == "app/Services/OrderService.php"),
        "omitting `allows` with `forbids` must allow non-forbidden layer `service`: {violations:?}"
    );
    assert!(
        violations.iter().any(|v| v.kind == "package"
            && v.from == "packages/ui/view.ts"
            && v.to == "packages/db/client.ts"),
        "expected package boundary violation: {violations:?}"
    );
    assert!(
        violations.iter().any(|v| v.kind == "cycle_budget"),
        "expected max_cycles budget violation: {violations:?}"
    );
    assert!(
        violations.iter().any(|v| v.kind == "semantic"
            && v.from == "app/Controllers/OrderController.php"
            && v.relation.as_deref() == Some("writes_table")),
        "expected semantic writes_table violation on controller: {violations:?}"
    );
    assert!(
        violations.iter().any(|v| v.kind == "semantic"
            && v.from == "app/Ui/Button.php"
            && v.relation.as_deref() == Some("exposes_route")),
        "expected top-level semantic_rules violation: {violations:?}"
    );

    let mut file_view = idx.view_at_lod_unfiltered(
        "demo",
        Lod::File,
        &LodFilter {
            scope: "",
            focus: None,
            filter: "",
            certain: false,
            offset: 0,
            limit: None,
        },
    );
    idx.mark_architecture(&mut file_view, false, Some(&rules));
    assert!(
        file_view["pattern"]["cycle_budget"]["exceeded"] == true,
        "expected cycle_budget telemetry in mark_architecture: {}",
        file_view["pattern"]
    );
    assert!(
        !file_view["pattern"]["package_violations"]
            .as_array()
            .unwrap()
            .is_empty(),
        "expected package_violations in mark_architecture: {}",
        file_view["pattern"]
    );
    assert!(
        !file_view["pattern"]["semantic_violations"]
            .as_array()
            .unwrap()
            .is_empty(),
        "expected semantic_violations in mark_architecture: {}",
        file_view["pattern"]
    );
}

#[test]
fn semantic_virtual_hubs_partition_by_namespace_not_anchor_file_at_all_lods() {
    use crate::graph_ir::{IrLinkSpec, IrNodeSpec};
    let dir = tempfile::tempdir().unwrap();
    let mk_file = |path: &str| IrNodeSpec {
        id: format!("file:{path}"),
        label: path.rsplit('/').next().unwrap_or(path).to_owned(),
        source_file: path.to_owned(),
        source_location: None,
        end_line: 0,
        file_type: Some("code".into()),
        node_kind: Some("page".into()),
        community: None,
        community_name: None,
        callable_class: false,
        is_abstract: false,
        is_test: false,
    };
    let mk_fn = |id: &str, path: &str, name: &str| IrNodeSpec {
        id: id.to_owned(),
        label: format!("{name}()"),
        source_file: path.to_owned(),
        source_location: Some("L1".into()),
        end_line: 1,
        file_type: None,
        node_kind: Some("function".into()),
        community: None,
        community_name: None,
        callable_class: false,
        is_abstract: false,
        is_test: false,
    };
    let mk_sem = |id: &str, label: &str, ns_bucket: &str| IrNodeSpec {
        id: id.to_owned(),
        label: label.to_owned(),
        source_file: ns_bucket.to_owned(),
        source_location: Some("L2".into()),
        end_line: 0,
        file_type: None,
        node_kind: Some("semantic".into()),
        community: None,
        community_name: None,
        callable_class: false,
        is_abstract: false,
        is_test: false,
    };
    let nodes = vec![
        mk_file("app/Controllers/OrderController.php"),
        mk_fn("fn:ctrl", "app/Controllers/OrderController.php", "store"),
        mk_file("app/Services/LedgerService.php"),
        mk_fn("fn:svc", "app/Services/LedgerService.php", "post"),
        mk_sem(
            "sem:table:orders",
            "table:orders",
            crate::graph_ir::semantic_namespace_bucket("sem:table:orders"),
        ),
        mk_sem(
            "sem:route:GET /orders",
            "route:GET /orders",
            crate::graph_ir::semantic_namespace_bucket("sem:route:GET /orders"),
        ),
    ];
    let links = vec![
        IrLinkSpec {
            source: "fn:ctrl".into(),
            target: "sem:table:orders".into(),
            relation: Some("writes_table".into()),
            confidence: Some("EXTRACTED".into()),
            source_file: Some("app/Controllers/OrderController.php".into()),
            context: Some("writes_table".into()),
            evidence: None,
        },
        IrLinkSpec {
            source: "fn:svc".into(),
            target: "sem:table:orders".into(),
            relation: Some("reads_table".into()),
            confidence: Some("EXTRACTED".into()),
            source_file: Some("app/Services/LedgerService.php".into()),
            context: Some("reads_table".into()),
            evidence: None,
        },
        IrLinkSpec {
            source: "fn:ctrl".into(),
            target: "sem:route:GET /orders".into(),
            relation: Some("exposes_route".into()),
            confidence: Some("EXTRACTED".into()),
            source_file: Some("app/Controllers/OrderController.php".into()),
            context: Some("exposes_route".into()),
            evidence: None,
        },
    ];
    let idx = Index::from_ir_parts(dir.path(), None, None, nodes, links).unwrap();
    let filter = LodFilter {
        scope: "",
        focus: None,
        filter: "",
        certain: false,
        offset: 0,
        limit: None,
    };

    for lod in [Lod::File, Lod::Folder, Lod::Crate] {
        let view = idx.view_at_lod_unfiltered("demo", lod, &filter);
        let sem_buckets: Vec<&Value> = view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| {
                n["path"]
                    .as_str()
                    .unwrap_or("")
                    .starts_with("__semantic__/")
            })
            .collect();
        assert!(
            sem_buckets
                .iter()
                .any(|n| n["path"] == "__semantic__/tables"),
            "expected __semantic__/tables bucket at {lod:?}: {:?}",
            view["nodes"]
        );
        assert!(
            sem_buckets
                .iter()
                .any(|n| n["path"] == "__semantic__/routes"),
            "expected __semantic__/routes bucket at {lod:?}: {:?}",
            view["nodes"]
        );
        for b in sem_buckets {
            assert_eq!(b["node_kind"], "semantic");
            assert_eq!(b["semantic"], true);
        }
    }

    // Legacy file view where a sem:* node has anchor_file set must STILL fold under __semantic__/*,
    // never into app/Services or root `.`.
    let mut legacy_view = json!({
        "nodes": [
            {"path": "app/Services/LedgerService.php", "name": "LedgerService.php", "out": 1, "in": 0},
            {"path": "sem:table:orders", "name": "table:orders", "anchor_file": "app/Services/LedgerService.php", "node_kind": "semantic", "semantic": true, "out": 0, "in": 1}
        ],
        "edges": [
            {"from": "app/Services/LedgerService.php", "to": "sem:table:orders", "links": 1}
        ]
    });
    assert!(lod::rollup_view_to_lod(&idx, &mut legacy_view, Lod::Crate));
    let sem_node = legacy_view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["path"] == "__semantic__/tables")
        .expect("legacy anchored sem node rolls up to __semantic__/tables");
    assert_eq!(sem_node["node_kind"], "semantic");
    assert_eq!(sem_node["semantic"], true);
}

#[test]
fn apply_min_weight_preserves_one_link_violations_in_edges_and_cross_page() {
    let mut view = json!({
        "nodes": [
            {"path": "crates/a"},
            {"path": "crates/b"},
            {"path": "crates/c"}
        ],
        "edges": [
            {"from": "crates/a", "to": "crates/b", "links": 1},
            {"from": "crates/a", "to": "crates/c", "links": 1, "against": true},
            {"from": "crates/b", "to": "crates/c", "links": 1, "kept": "violation"},
            {"from": "crates/c", "to": "crates/a", "links": 4}
        ],
        "between": 4,
        "paging": {
            "cross_page_edges": [
                {"from": "crates/a", "to": "crates/b", "links": 1},
                {"from": "crates/a", "to": "crates/c", "links": 1, "against": true}
            ]
        }
    });
    Index::apply_min_weight_and_pagination(&mut view, Some(2), None, 0, None, false);
    let edges = view["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 3, "only clean 1-link edge dropped: {edges:?}");
    assert!(
        edges
            .iter()
            .any(|e| e["to"] == "crates/c" && e["against"] == true)
    );
    assert!(edges.iter().any(|e| e["kept"] == "violation"));
    assert!(
        !edges
            .iter()
            .any(|e| e["from"] == "crates/a" && e["to"] == "crates/b")
    );
    assert_eq!(view["between"], 3);
    let cross = view["paging"]["cross_page_edges"].as_array().unwrap();
    assert_eq!(cross.len(), 1);
    assert_eq!(cross[0]["against"], true);
}
