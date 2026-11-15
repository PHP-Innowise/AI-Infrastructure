//! Tests of a run laid over the map of what it changed.
use super::*;

const CART: &str = "src/Cart/Cart.php";
const PRICE: &str = "src/Cart/Price.php";
const ORDER: &str = "src/Order/Order.php";
const INVOICE: &str = "src/Order/Invoice.php";
const REPORT: &str = "src/Report/Report.php";
const MAILER: &str = "src/Mail/Mailer.php";
const TAX: &str = "src/Tax/Tax.php";
const CART_TEST: &str = "tests/Cart/CartTest.php";
const OLD: &str = "src/Old/Legacy.php";
const KEEPER: &str = "src/Old/Keeper.php";
const GUESS: &str = "src/Guess/Guess.php";
const LOG: &str = "src/OrderLog/Log.php";

/// A shop of one type in each file. The cart and the price are what a run
/// changes; the order, the invoice, the report and a test use them.
fn shop() -> Index {
    let files = [
        CART, PRICE, ORDER, INVOICE, REPORT, MAILER, TAX, CART_TEST, OLD, KEEPER, GUESS, LOG,
    ];
    let nodes = files
        .iter()
        .map(|path| json!({"id": path, "label": name(path), "source_file": path}))
        .collect::<Vec<_>>();
    let link = |from: &str, to: &str, confidence: &str| json!({"source": from, "target": to, "relation": "calls", "confidence": confidence});
    let links = vec![
        // Users of the change: the order twice over the cart and once over
        // the price; the invoice over the price, once as the tool read it
        // and once as it guessed; the report once over each of the two; a
        // test; and a file the tool only guessed to use the cart.
        link(ORDER, CART, "EXTRACTED"),
        link(ORDER, CART, "EXTRACTED"),
        link(ORDER, PRICE, "EXTRACTED"),
        link(INVOICE, PRICE, "EXTRACTED"),
        link(INVOICE, PRICE, "INFERRED"),
        link(REPORT, CART, "EXTRACTED"),
        link(REPORT, PRICE, "EXTRACTED"),
        link(CART_TEST, CART, "EXTRACTED"),
        link(GUESS, CART, "INFERRED"),
        // A folder named like the start of another is not that other.
        link(TAX, LOG, "EXTRACTED"),
        // Within the change, what the change uses, and a user of a user.
        link(CART, PRICE, "EXTRACTED"),
        link(CART, TAX, "EXTRACTED"),
        link(MAILER, ORDER, "EXTRACTED"),
        // A user of a file the run deleted.
        link(KEEPER, OLD, "EXTRACTED"),
    ];
    Index::read(
        json!({"nodes": nodes, "links": links})
            .to_string()
            .as_bytes(),
        Path::new("/work/shop"),
    )
    .unwrap()
}
fn changed() -> Vec<(String, char)> {
    vec![
        (CART.to_owned(), 'M'),
        (PRICE.to_owned(), 'A'),
        (OLD.to_owned(), 'D'),
        ("src/New/Unknown.php".to_owned(), 'A'),
    ]
}
fn opened(paths: &[&str]) -> BTreeSet<String> {
    paths.iter().map(|path| (*path).to_owned()).collect()
}
/// The rows of the unopened: path, links, guessed, changed files used, test.
fn rows(view: &Value) -> Vec<(String, u64, u64, u64, bool)> {
    view["run"]["reach"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["path"].as_str().unwrap().to_owned(),
                row["links"].as_u64().unwrap(),
                row["guessed"].as_u64().unwrap(),
                row["uses"].as_u64().unwrap(),
                row["test"] == true,
            )
        })
        .collect()
}
fn reach(view: &Value) -> (u64, u64, u64, u64) {
    let reach = &view["run"]["reach"];
    (
        reach["users"].as_u64().unwrap(),
        reach["opened"].as_u64().unwrap(),
        reach["unopened"].as_u64().unwrap(),
        reach["more"].as_u64().unwrap(),
    )
}
#[test]
fn a_run_says_which_users_of_its_change_it_did_not_open() {
    let index = shop();
    let changed = changed();
    let mut view = index.changes("demo", &changed, "", None, false, "");
    // The run opened what it changed, the order, and a file that uses
    // nothing of the change.
    index.overlay_run(&mut view, &changed, &opened(&[CART, ORDER, TAX]), false);
    // Five files use the change: the order, the invoice, the report, the
    // test and the guessed one. A changed file is no user of the change,
    // what the change uses is none, nor is the user of a user, nor the user
    // of a file that is gone.
    assert_eq!(reach(&view), (5, 1, 4, 0));
    // The unopened, the most linked first and by their paths after that;
    // each with how many of its links the tool guessed and how many of the
    // changed files it uses.
    assert_eq!(
        rows(&view),
        [
            (INVOICE.to_owned(), 2, 1, 1, false),
            (REPORT.to_owned(), 2, 0, 2, false),
            (GUESS.to_owned(), 1, 1, 1, false),
            (CART_TEST.to_owned(), 1, 0, 1, true),
        ]
    );
    assert_eq!(view["run"]["reach"]["rows"][0]["name"], "Invoice.php");
    assert_eq!(view["run"]["reach"]["rows"][0]["to"], PRICE);
    // The report uses the cart and the price alike: its link is looked
    // into with the first of them.
    assert_eq!(view["run"]["reach"]["rows"][1]["to"], CART);
    // Opened, the order is counted and not listed, though it is the most
    // linked of them; unopened, it comes first with its three links over
    // two changed files.
    let mut none = index.changes("demo", &changed, "", None, false, "");
    index.overlay_run(&mut none, &changed, &opened(&[]), false);
    assert_eq!(reach(&none), (5, 0, 5, 0));
    assert_eq!(rows(&none)[0], (ORDER.to_owned(), 3, 0, 2, false));
    // Its link is looked into where it is strongest: with the cart.
    assert_eq!(none["run"]["reach"]["rows"][0]["to"], CART);
    // All opened, nothing is left to look at.
    let mut all = index.changes("demo", &changed, "", None, false, "");
    index.overlay_run(
        &mut all,
        &changed,
        &opened(&[ORDER, INVOICE, REPORT, CART_TEST, GUESS]),
        false,
    );
    assert_eq!(reach(&all), (5, 5, 0, 0));
    assert_eq!(all["run"]["reach"]["rows"], json!([]));
    // Without what the tool only inferred the guessed one uses nothing,
    // and the invoice has the one link that was read.
    let mut sure = index.changes("demo", &changed, "", None, true, "");
    index.overlay_run(&mut sure, &changed, &opened(&[]), true);
    assert_eq!(reach(&sure), (4, 0, 4, 0));
    assert!(rows(&sure).iter().all(|row| row.0 != GUESS && row.2 == 0));
    assert!(rows(&sure).contains(&(INVOICE.to_owned(), 1, 0, 1, false)));
    // A run that changed nothing the graph knows has no users to tell of.
    let nothing = vec![("src/New/Unknown.php".to_owned(), 'A')];
    let mut view = index.changes("demo", &nothing, "", None, false, "");
    index.overlay_run(&mut view, &nothing, &opened(&[ORDER]), false);
    assert_eq!(reach(&view), (0, 0, 0, 0));
}
#[test]
fn the_users_told_are_those_of_what_changed_where_the_picture_is_open() {
    let index = shop();
    let changed = vec![
        (CART.to_owned(), 'M'),
        (PRICE.to_owned(), 'M'),
        (ORDER.to_owned(), 'M'),
        (LOG.to_owned(), 'M'),
    ];
    let told = |scope: &str| {
        let mut view = index.changes("demo", &changed, scope, None, false, "");
        index.overlay_run(&mut view, &changed, &opened(&[]), false);
        (
            view["scope"].as_str().unwrap().to_owned(),
            reach(&view),
            rows(&view).into_iter().map(|row| row.0).collect::<Vec<_>>(),
        )
    };
    // In the whole project: what uses one of the four and is none of them.
    assert_eq!(
        told(""),
        (
            String::new(),
            (6, 0, 6, 0),
            vec![
                INVOICE.to_owned(),
                REPORT.to_owned(),
                GUESS.to_owned(),
                MAILER.to_owned(),
                TAX.to_owned(),
                CART_TEST.to_owned(),
            ]
        )
    );
    // In the folder of the order only the order changed: its one user.
    // What uses the log is of another folder, whose name only begins so.
    assert_eq!(
        told("src/Order"),
        (
            "src/Order".to_owned(),
            (1, 0, 1, 0),
            vec![MAILER.to_owned()]
        )
    );
    // In the folder of the cart: the order uses the cart and the price,
    // and is no user of them, being changed itself.
    assert_eq!(
        told("src/Cart"),
        (
            "src/Cart".to_owned(),
            (4, 0, 4, 0),
            vec![
                INVOICE.to_owned(),
                REPORT.to_owned(),
                GUESS.to_owned(),
                CART_TEST.to_owned(),
            ]
        )
    );
    // An opened file is a picture of that file alone.
    let mut file = index
        .symbols("demo", CART, &changed, None, None, false, "")
        .unwrap();
    index.overlay_run(&mut file, &changed, &opened(&[REPORT]), false);
    assert_eq!(file["scope"], CART);
    assert_eq!(reach(&file), (3, 1, 2, 0));
    assert_eq!(rows(&file)[0].0, GUESS);
    assert_eq!(rows(&file)[1].0, CART_TEST);
}
#[test]
fn the_boxes_and_the_sides_of_a_place_say_what_the_run_opened() {
    let index = shop();
    let changed = changed();
    let seen = opened(&[CART, ORDER, TAX, "src/Order/Gone.php"]);
    let marks = |view: &Value, places: &str| {
        view.pointer(places)
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter(|place| !place["opened"].is_null())
            .map(|place| {
                (
                    place["path"].as_str().unwrap().to_owned(),
                    place["opened"].clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    // In the picture of the change: the files the run opened, and no other.
    let mut view = index.changes("demo", &changed, "", Some(CART), false, "");
    index.overlay_run(&mut view, &changed, &seen, false);
    let mut boxes = marks(&view, "/nodes");
    boxes.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        boxes,
        [
            (CART.to_owned(), json!(true)),
            (ORDER.to_owned(), json!(true)),
            (TAX.to_owned(), json!(true)),
        ]
    );
    // Beside the selected cart: of what uses it the order was opened, of
    // what it uses the tax.
    assert_eq!(
        marks(&view, "/focus/used_by"),
        [(ORDER.to_owned(), json!(true))]
    );
    assert_eq!(marks(&view, "/focus/uses"), [(TAX.to_owned(), json!(true))]);
    assert!(view["focus"]["used_by"].as_array().unwrap().len() > 1);
    // A folder says how many files in it the run opened, counting those the
    // graph does not know; a folder with none says nothing, and a folder
    // named like the start of another is not that other.
    let mut folders = index.view("demo", "src", None, false, "");
    index.overlay_run(
        &mut folders,
        &changed,
        &opened(&[CART, ORDER, "src/Order/Gone.php", "src/Orders/Other.php"]),
        false,
    );
    let mut boxes = marks(&folders, "/nodes");
    boxes.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        boxes,
        [
            ("src/Cart".to_owned(), json!(1)),
            ("src/Order".to_owned(), json!(2)),
        ]
    );
    // The symbols of an opened file are no files.
    let mut file = index
        .symbols("demo", CART, &changed, None, None, false, "")
        .unwrap();
    index.overlay_run(&mut file, &changed, &seen, false);
    assert!(
        file["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node["path"].as_str().is_some_and(|path| path.contains('#')))
            .all(|node| node["opened"].is_null())
    );
}
#[test]
fn more_unopened_users_than_fit_are_counted() {
    let users = LISTED + 4;
    let mut nodes = vec![json!({"id": "core", "label": "core", "source_file": "lib/core.php"})];
    let mut links = Vec::new();
    for at in 0..users {
        let path = format!("lib/user{at:02}.php");
        nodes.push(json!({"id": path, "label": path, "source_file": path}));
        links.push(json!({"source": path, "target": "core", "relation": "calls", "confidence": "EXTRACTED"}));
        if at == 9 {
            links.push(json!({"source": path, "target": "core", "relation": "imports", "confidence": "EXTRACTED"}));
        }
    }
    let index = Index::read(
        json!({"nodes": nodes, "links": links})
            .to_string()
            .as_bytes(),
        Path::new("/work/lib"),
    )
    .unwrap();
    let changed = vec![("lib/core.php".to_owned(), 'M')];
    let mut view = index.changes("demo", &changed, "", None, false, "");
    index.overlay_run(&mut view, &changed, &opened(&["lib/user00.php"]), false);
    assert_eq!(
        reach(&view),
        (
            users as u64,
            1,
            users as u64 - 1,
            users as u64 - 1 - LISTED as u64
        )
    );
    let listed = rows(&view);
    assert_eq!(listed.len(), LISTED);
    // The most linked first, then by path; the opened one is not among them.
    assert_eq!(listed[0], ("lib/user09.php".to_owned(), 2, 0, 1, false));
    assert_eq!(listed[1].0, "lib/user01.php");
    assert!(listed.iter().all(|row| row.0 != "lib/user00.php"));
}

#[test]
fn graph_focus_overlay_preserves_selected_focus_and_marks_live_targets() {
    let index = Index::read(
        json!({
            "nodes": [
                {"id": "a", "label": "JwtGuard", "source_file": "src/Auth/JwtGuard.php"},
                {"id": "b", "label": "Invoice", "source_file": "src/Billing/Invoice.php"}
            ],
            "links": [
                {"source": "b", "target": "a", "relation": "calls", "confidence": "EXTRACTED"}
            ]
        })
        .to_string()
        .as_bytes(),
        Path::new("/work/app"),
    )
    .unwrap();
    let mut view = index.view("app", "src/Auth", Some("src/Auth/JwtGuard.php"), false, "");
    let selected_focus = view["focus"].clone();
    assert_eq!(selected_focus["path"], "src/Auth/JwtGuard.php");

    let live = crate::graph_live::GraphFocus {
        phase: "active".into(),
        summary: "Live: 2 new entities in src/Auth".into(),
        target_folders: vec!["src/Auth".into()],
        target_paths: vec!["src/Auth/JwtGuard.php".into()],
        target_nodes: vec!["src/Auth".into(), "src/Auth/JwtGuard.php".into()],
        created_entities: crate::graph_live::diff_new_entities(
            "src/Auth/JwtGuard.php",
            None,
            "<?php\nclass JwtGuard {}\n",
            true,
        ),
    };
    index.overlay_graph_focus(&mut view, &live);

    // Clicked-node `view["focus"]` remains intact while `graph_focus` carries targets + entities.
    assert_eq!(view["focus"], selected_focus);
    assert_eq!(view["graph_focus"]["phase"], "active");
    assert_eq!(view["run"]["graph_focus"], view["graph_focus"]);
    assert_eq!(
        view["graph_focus"]["created_entities"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    let file_node = view["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|n| n["path"] == "src/Auth/JwtGuard.php")
        .expect("file node");
    assert_eq!(file_node["focus_target"], true);
    assert_eq!(file_node["created_entities_count"], 2);

    // At root scope, parent folder `src` inherits subtree target & created count.
    let mut root_view = index.view("app", "", Some("src"), false, "");
    let root_selected_focus = root_view["focus"].clone();
    index.overlay_graph_focus(&mut root_view, &live);
    assert_eq!(root_view["focus"], root_selected_focus);
    let src_node = root_view["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|n| n["path"] == "src")
        .expect("src folder node");
    assert_eq!(src_node["focus_target"], true);
    assert_eq!(src_node["created_entities_count"], 2);
}
