//! Tests of the reads and writes the sources add to the links of a picture.
use super::*;

const ACCOUNT: &str = "src/Ledger/Account.php";
const TELLER: &str = "src/Desk/Teller.php";
const REPORT: &str = "src/Desk/Report.php";
const VAULT: &str = "lib/Bank/Vault.php";

/// A ledger whose teller writes the balance of an account and reads it, a
/// report that only calls, and a vault that reads a constant of the account.
fn ledger() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().unwrap();
    let write = |path: &str, text: &str| {
        let file = dir.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    };
    write(
        ACCOUNT,
        "<?php\nnamespace App\\Ledger;\nclass Account {\n    public const LIMIT = 10;\n    public int $balance = 0;\n    public function total(): int { return $this->balance; }\n}\n",
    );
    write(
        TELLER,
        "<?php\nnamespace App\\Desk;\nuse App\\Ledger\\Account;\nclass Teller {\n    public function deposit(Account $acc): void {\n        $acc->balance = $acc->balance + 5;\n    }\n    public function audit(Account $acc): int {\n        return $acc->balance + Account::LIMIT + $acc->total();\n    }\n}\n",
    );
    write(
        REPORT,
        "<?php\nnamespace App\\Desk;\nuse App\\Ledger\\Account;\nclass Report {\n    public function show(Account $acc): int { return $acc->total(); }\n}\n",
    );
    write(
        VAULT,
        "<?php\nnamespace Lib\\Bank;\nuse App\\Ledger\\Account;\nclass Vault {\n    public function cap(): int { return Account::LIMIT; }\n}\n",
    );
    let node = |id: &str, label: &str, file: &str, line: u32| json!({"id": id, "label": label, "source_file": file, "source_location": format!("L{line}")});
    let link = |from: &str, to: &str, relation: &str| json!({"source": from, "target": to, "relation": relation, "confidence": "EXTRACTED"});
    let graph = json!({
        "nodes": [
            {"id": "account", "label": "Account", "source_file": ACCOUNT, "source_location": "L3", "_callable_class": true},
            node("total", ".total()", ACCOUNT, 6),
            {"id": "teller", "label": "Teller", "source_file": TELLER, "source_location": "L4", "_callable_class": true},
            node("deposit", ".deposit()", TELLER, 5),
            node("audit", ".audit()", TELLER, 8),
            {"id": "report", "label": "Report", "source_file": REPORT, "source_location": "L4", "_callable_class": true},
            node("show", ".show()", REPORT, 5),
            {"id": "vault", "label": "Vault", "source_file": VAULT, "source_location": "L4", "_callable_class": true},
            node("cap", ".cap()", VAULT, 5),
        ],
        "links": [
            link("account", "total", "method"),
            link("teller", "deposit", "method"),
            link("teller", "audit", "method"),
            link("report", "show", "method"),
            link("vault", "cap", "method"),
            link("deposit", "account", "references"),
            link("audit", "account", "references"),
            link("audit", "total", "calls"),
            link("show", "total", "calls"),
            link("cap", "account", "references"),
        ],
    });
    let index = Index::read(graph.to_string().as_bytes(), dir.path()).unwrap();
    (dir, index)
}
/// The kinds of a link, each with its count and, where the source told it,
/// a mark.
fn kinds(view: &Value, from: &str, to: &str) -> Vec<String> {
    view["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|edge| edge["from"] == from && edge["to"] == to)
        .unwrap_or_else(|| panic!("no link {from} -> {to} in {view}"))["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|kind| {
            format!(
                "{} {}{}",
                kind["kind"].as_str().unwrap(),
                kind["links"],
                if kind["by"] == "source" {
                    " source"
                } else {
                    ""
                }
            )
        })
        .collect()
}
#[test]
fn a_link_between_files_tells_what_the_source_reads_and_writes() {
    let (_dir, index) = ledger();
    let mut view = index.view("bank", "src", Some(TELLER), false, "");
    // Folders of the open folder: the desk uses the ledger.
    assert_eq!(
        kinds(&view, "src/Desk", "src/Ledger"),
        ["calls 2", "references 2"].map(String::from)
    );
    index.mark_access(&mut view);
    // The teller writes the balance, and reads the balance and the limit.
    // The report only calls, which the graph told already.
    assert_eq!(
        kinds(&view, "src/Desk", "src/Ledger"),
        [
            "calls 2",
            "references 2",
            "writes 1 source",
            "reads 2 source"
        ]
        .map(String::from)
    );
    assert_eq!(
        view["access"],
        json!({"links": 1, "left": 0, "sources": 2}),
        "{view}"
    );
    // A source is read once for a graph.
    let mut again = index.view("bank", "src", None, false, "");
    index.mark_access(&mut again);
    assert_eq!(
        again["access"],
        json!({"links": 1, "left": 0, "sources": 0})
    );
    // File by file, the report adds nothing of its own.
    let mut desk = index.view("bank", "src/Desk", Some(TELLER), false, "");
    index.mark_access(&mut desk);
    assert_eq!(desk["access"]["links"], 0, "{desk}");
    let mut project = index.view("bank", "", None, false, "");
    index.mark_access(&mut project);
    assert_eq!(
        kinds(&project, "lib", "src"),
        ["references 1", "reads 1 source"].map(String::from)
    );
}
#[test]
fn an_opened_file_tells_each_symbol_its_own_reads_and_writes() {
    let (_dir, index) = ledger();
    let mut view = index.view("bank", TELLER, None, false, "");
    assert_eq!(view["mode"], "file");
    index.mark_access(&mut view);
    let key = |name: &str| {
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["name"] == name)
            .unwrap_or_else(|| panic!("no {name} in {view}"))["path"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    // Depositing writes the balance and reads it; the audit reads the
    // balance and the limit, in its own lines.
    assert_eq!(
        kinds(&view, &key("Teller.deposit()"), ACCOUNT),
        ["references 1", "writes 1 source", "reads 1 source"].map(String::from)
    );
    assert_eq!(
        kinds(&view, &key("Teller.audit()"), ACCOUNT),
        ["calls 1", "references 1", "reads 2 source"].map(String::from)
    );
    // Seen from the account, the teller reaches its type.
    let mut account = index.view("bank", ACCOUNT, None, false, "");
    index.mark_access(&mut account);
    let class = account["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["name"] == "Account")
        .unwrap()["path"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        kinds(&account, TELLER, &class),
        ["references 2", "writes 1 source", "reads 2 source"].map(String::from)
    );
    assert_eq!(
        kinds(&account, VAULT, &class),
        ["references 1", "reads 1 source"].map(String::from)
    );
}
#[test]
fn a_picture_reads_no_more_sources_than_its_room_and_tells_a_link_whole() {
    // More files use one file than a picture reads anew: each is a link of
    // its own, so the first are told and the rest are left for the next
    // picture, which finds the first in its keeping.
    let dir = tempfile::tempdir().unwrap();
    let write = |path: &str, text: &str| {
        let file = dir.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    };
    write(
        "src/Config.php",
        "<?php\nnamespace App;\nclass Config {\n    public const TTL = 5;\n}\n",
    );
    let users = ACCESS_SOURCES + 7;
    let mut nodes = vec![
        json!({"id": "config", "label": "Config", "source_file": "src/Config.php", "source_location": "L3"}),
    ];
    let mut links = vec![];
    for at in 0..users {
        let path = format!("src/User{at:03}.php");
        write(
            &path,
            &format!(
                "<?php\nnamespace App;\nclass User{at:03} {{\n    public function ttl(): int {{ return Config::TTL; }}\n}}\n"
            ),
        );
        nodes.push(json!({"id": format!("user{at}"), "label": format!("User{at:03}"), "source_file": path, "source_location": "L3"}));
        links.push(json!({"source": format!("user{at}"), "target": "config", "relation": "references", "confidence": "EXTRACTED"}));
    }
    let index = Index::read(
        json!({"nodes": nodes, "links": links})
            .to_string()
            .as_bytes(),
        dir.path(),
    )
    .unwrap();
    // A picture given every link, whatever a folder would draw.
    let picture = || {
        json!({
            "scope": "src",
            "nodes": (0..users)
                .map(|at| json!({"path": format!("src/User{at:03}.php")}))
                .chain([json!({"path": "src/Config.php"})])
                .collect::<Vec<_>>(),
            "edges": (0..users)
                .map(|at| json!({"from": format!("src/User{at:03}.php"), "to": "src/Config.php", "links": 1, "guessed": 0}))
                .collect::<Vec<_>>(),
        })
    };
    let told = |view: &Value| {
        view["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|edge| edge["kinds"] == json!([{"kind": "reads", "links": 1, "by": "source"}]))
            .count()
    };
    let mut first = picture();
    index.mark_access(&mut first);
    assert_eq!(
        first["access"],
        json!({"links": ACCESS_SOURCES, "left": 7, "sources": ACCESS_SOURCES})
    );
    assert_eq!(told(&first), ACCESS_SOURCES);
    let mut second = picture();
    index.mark_access(&mut second);
    assert_eq!(
        second["access"],
        json!({"links": users, "left": 0, "sources": 7})
    );
    assert_eq!(told(&second), users);
}
#[test]
fn other_pictures_and_unreadable_sources_are_left_as_they_are() {
    let (dir, index) = ledger();
    // A picture of a path, of a cycle or of a pattern is not of this kind.
    for mode in ["path", "cycle", "pattern", "modules", "scenario", "link"] {
        let mut view = json!({"mode": mode, "nodes": [], "edges": []});
        index.mark_access(&mut view);
        assert!(view["access"].is_null(), "{mode}");
    }
    // A source that is gone reaches nothing, and the link stays as the
    // graph gave it.
    std::fs::remove_file(dir.path().join(TELLER)).unwrap();
    let mut view = index.view("bank", "src/Desk", Some(TELLER), false, "");
    index.mark_access(&mut view);
    let mut project = index.view("bank", "src", None, false, "");
    index.mark_access(&mut project);
    assert_eq!(
        kinds(&project, "src/Desk", "src/Ledger"),
        ["calls 2", "references 2"].map(String::from)
    );
}
