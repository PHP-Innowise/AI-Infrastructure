use super::*;
use std::collections::VecDeque;

const LISTED: usize = 8;
const VISITS: usize = 250_000;

struct Candidate {
    steps: u32,
    file: u32,
    symbol: Option<(u32, u32)>,
}

/// Breadth-first along incoming edges, stopping if this response has used
/// its traversal budget. A place is visited only once per changed file.
fn walk(
    net: &Net<'_>,
    starts: Vec<u32>,
    budget: &mut usize,
    mut found: impl FnMut(u32, u32, u32),
) -> bool {
    let mut seen: HashSet<_> = starts.iter().copied().collect();
    let mut pending: VecDeque<_> = starts.into_iter().map(|at| (at, 0, at)).collect();
    while let Some((place, steps, goal)) = pending.pop_front() {
        let Some(left) = budget.checked_sub(1) else {
            return false;
        };
        *budget = left;
        if steps > 0 {
            found(place, steps, goal);
        }
        let mut complete = true;
        net.each(place, false, &mut |other| {
            if let Some(left) = budget.checked_sub(1) {
                *budget = left;
                if seen.insert(other) {
                    pending.push_back((other, steps + 1, goal));
                }
            } else {
                complete = false;
            }
        });
        if !complete {
            return false;
        }
    }
    true
}

impl Index {
    pub(crate) fn tests_reaching_changes(
        &self,
        view: &mut Value,
        changed: &[(String, char)],
        scope: &str,
        certain: bool,
    ) {
        if !matches!(view["mode"].as_str(), Some("changes" | "file")) || changed.is_empty() {
            return;
        }
        let ids: HashMap<_, _> = self
            .files
            .iter()
            .enumerate()
            .map(|(id, path)| (path.as_str(), id as u32))
            .collect();
        let mut paths: Vec<_> = changed
            .iter()
            .filter(|(path, _)| {
                scope.is_empty()
                    || path == scope
                    || path
                        .strip_prefix(scope)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            })
            .collect();
        if paths.is_empty() {
            return;
        }
        // A map sometimes opens a common folder automatically. Keep the
        // list sorted and prefer the selected file if work has to be capped.
        let focused = view["focus"]["path"].as_str();
        paths.sort_by_key(|(path, _)| (focused != Some(path.as_str()), path));
        let test_files: Vec<_> = self.files.iter().map(|path| is_test(path)).collect();
        let candidate_files: Vec<_> = self
            .files
            .iter()
            .zip(&test_files)
            .map(|(path, test)| {
                *test
                    && !path.split('/').any(|part| {
                        matches!(
                            part,
                            "fixtures" | "__fixtures__" | "__mocks__" | "testdata" | "conftest.py"
                        ) || part.ends_with("_fixture.rs")
                            || part.ends_with("_fixtures.rs")
                    })
            })
            .collect();
        let test_symbols = self
            .test_symbols
            .get_or_init(|| self.test_symbols(&test_files, &|path| text_at(&self.root, path)));
        let inline = |at: u32| {
            let symbol = &self.symbols[at as usize];
            !test_files[symbol.file as usize]
                && test_symbols[at as usize]
                && matches!(symbol.kind, Kind::Function | Kind::Method)
        };
        let has_inline = (0..self.symbols.len() as u32).any(inline);
        let files = Net::new(self, false, certain);
        let symbols = Net::new(self, true, certain);
        let names = Names::new(self, &[], &|_| None);
        let mut budget = VISITS;
        let rows: Vec<_> = paths.into_iter().map(|(path, status)| {
            let Some(file) = ids.get(path.as_str()).copied().filter(|_| *status != 'D') else {
                return json!({"path": path, "state": if *status == 'D' { "deleted" } else { "missing" }});
            };
            if test_files[file as usize] {
                return json!({"path": path, "state": "test_file"});
            }
            let mut found = Vec::new();
            let files_done = walk(&files, vec![file], &mut budget, |at, steps, _| {
                if candidate_files[at as usize] {
                    found.push(Candidate { steps, file: at, symbol: None });
                }
            });
            let symbols_done = !has_inline || (files_done && walk(
                &symbols,
                self.range(file).filter(|at| {
                    !test_symbols[*at as usize] && self.symbols[*at as usize].kind != Kind::Note
                }).collect(),
                &mut budget,
                |at, steps, goal| {
                    if inline(at) {
                        found.push(Candidate {
                            steps,
                            file: self.symbols[at as usize].file,
                            symbol: Some((at, goal)),
                        });
                    }
                },
            ));
            found.sort_by(|a, b| a.steps.cmp(&b.steps)
                .then(self.files[a.file as usize].cmp(&self.files[b.file as usize]))
                .then(a.symbol.cmp(&b.symbol)));
            let tests = found.iter().take(LISTED).map(|test| {
                let file = &self.files[test.file as usize];
                let (from, to, label) = test.symbol.map_or_else(
                    || (file.clone(), path.clone(), file.clone()),
                    |(at, goal)| (names.key(at), names.key(goal), self.shown(at)),
                );
                json!({"path": from, "to": to, "file": file, "name": label, "steps": test.steps})
            }).collect::<Vec<_>>();
            json!({"path": path, "state": if files_done && symbols_done { "ready" } else { "partial" },
                "tests": tests, "total": found.len()})
        }).collect();
        view["test_reach"] = Value::Array(rows);
    }
}

/// How many of the changed files that failing tests reach are listed.
const RANKED: usize = 12;

/// Whether what a person named, or pasted from the output of a test run,
/// names a test file: by its path, or by its name without the extension
/// standing as a word of its own, so that `Cart` does not name `CartTest`.
fn names_test(told: &str, path: &str) -> bool {
    if told.contains(path) {
        return true;
    }
    let stem = name(path);
    let stem = stem.split('.').next().unwrap_or(stem);
    if stem.len() < 4 {
        return false;
    }
    let word = |letter: char| letter.is_alphanumeric() || letter == '_';
    told.match_indices(stem).any(|(at, _)| {
        !told[..at].chars().next_back().is_some_and(word)
            && !told[at + stem.len()..].chars().next().is_some_and(word)
    })
}

impl Index {
    /// Ranks the changed files of a picture by how likely each is behind
    /// the tests that fail. `failing` names those tests as a person would:
    /// paths, names of test files, or lines of the output of a test run.
    ///
    /// The graph stands in for what a test runs: a test reaches a file
    /// where a chain of links leads from the test to it. A changed file is
    /// the more suspect the more of the failing tests reach it and the
    /// fewer of the passing ones do: the share of the failing tests that
    /// reach it, times the share of failing among the tests that reach it,
    /// under a root, which is 1 where exactly the failing tests reach it
    /// and 0 where none does. Of files as suspect, the one nearer to the
    /// failing tests comes first.
    ///
    /// This reads no test run: a link is not a call that happened, so the
    /// order says where to look first, not what broke.
    pub(crate) fn suspects(
        &self,
        view: &mut Value,
        changed: &[(String, char)],
        failing: &[String],
        certain: bool,
    ) {
        if view["mode"] != "changes" || changed.is_empty() || failing.is_empty() {
            return;
        }
        let candidates = self
            .files
            .iter()
            .map(|path| {
                is_test(path)
                    && !path.split('/').any(|part| {
                        matches!(
                            part,
                            "fixtures" | "__fixtures__" | "__mocks__" | "testdata" | "conftest.py"
                        ) || part.ends_with("_fixture.rs")
                            || part.ends_with("_fixtures.rs")
                    })
            })
            .collect::<Vec<_>>();
        // The tests that are named, and what was named that is no test.
        let mut failed = vec![false; self.files.len()];
        let mut unmatched = Vec::new();
        for told in failing {
            let mut any = false;
            for (file, path) in self.files.iter().enumerate() {
                if candidates[file] && names_test(told, path) {
                    failed[file] = true;
                    any = true;
                }
            }
            if !any {
                unmatched.push(clip(told.trim(), 120).to_owned());
            }
        }
        let failing_files = (0..self.files.len())
            .filter(|file| failed[*file])
            .collect::<Vec<_>>();
        let total = failing_files.len();
        let net = Net::new(self, false, certain);
        let mut budget = VISITS;
        let mut complete = true;
        let mut rows = Vec::new();
        let mut clear = 0usize;
        for (path, status) in changed {
            let Some(file) = self.file_id(path).filter(|_| *status != 'D') else {
                continue;
            };
            if candidates[file as usize] {
                continue;
            }
            // Which tests reach it, and from how near the failing ones do.
            let (mut fail, mut pass, mut nearest, mut far) = (0usize, 0usize, u32::MAX, 0u64);
            complete &= walk(&net, vec![file], &mut budget, |at, steps, _| {
                if !candidates[at as usize] {
                    return;
                }
                if failed[at as usize] {
                    fail += 1;
                    nearest = nearest.min(steps);
                    far += u64::from(steps);
                } else {
                    pass += 1;
                }
            });
            if fail == 0 {
                clear += 1;
                continue;
            }
            let score = fail as f64 / ((total * (fail + pass)) as f64).sqrt();
            rows.push((path.as_str(), *status, score, fail, pass, nearest, far));
        }
        rows.sort_by(|a, b| {
            b.2.total_cmp(&a.2)
                .then((a.6 * b.3 as u64).cmp(&(b.6 * a.3 as u64)))
                .then(a.0.cmp(b.0))
        });
        let ranked = rows.len();
        let listed = rows
            .iter()
            .take(RANKED)
            .enumerate()
            .map(|(at, (path, status, score, fail, pass, nearest, _))| {
                json!({
                    "path": path,
                    "status": status.to_string(),
                    "rank": at + 1,
                    "score": (score * 100.0 + 1e-6).round() / 100.0,
                    "failing": fail,
                    "passing": pass,
                    "nearest": nearest,
                })
            })
            .collect::<Vec<_>>();
        // The boxes of the picture carry their rank, and a folder the best
        // rank among the ranked files it holds: a change too wide to draw
        // file by file is drawn by folder.
        if let Some(nodes) = view["nodes"].as_array_mut() {
            for node in nodes {
                let Some(path) = node["path"].as_str().map(str::to_owned) else {
                    continue;
                };
                if let Some(row) = listed.iter().find(|row| row["path"] == path.as_str()) {
                    node["suspect"] = json!({"rank": row["rank"], "score": row["score"]});
                } else if node["folder"] == true {
                    let held = listed
                        .iter()
                        .filter(|row| {
                            row["path"].as_str().is_some_and(|file| {
                                file.strip_prefix(path.as_str())
                                    .is_some_and(|rest| rest.starts_with('/'))
                            })
                        })
                        .collect::<Vec<_>>();
                    if let Some(first) = held.first() {
                        node["suspect"] = json!({
                            "rank": first["rank"], "score": first["score"], "files": held.len(),
                        });
                    }
                }
            }
        }
        view["suspects"] = json!({
            "failing": failing_files
                .iter()
                .map(|file| self.files[*file].as_str())
                .collect::<Vec<_>>(),
            "unmatched": unmatched,
            "tests": candidates.iter().filter(|test| **test).count(),
            "rows": listed,
            "more": ranked.saturating_sub(RANKED),
            "clear": clear,
            "complete": complete,
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_only_incoming_paths_and_distinguishes_unavailable_files() {
        let node = |id: &str, file: &str| json!({"id":id, "label":id, "source_file":file});
        let edge = |from: &str, to: &str, confidence: &str| json!({"source":from, "target":to, "relation":"calls", "confidence":confidence});
        let graph = json!({
            "nodes": [
                node("changed", "src/changed.php"),
                node("alone", "src/alone.php"),
                node("bridge", "src/bridge.php"),
                node("test", "tests/CartTest.php"),
                node("reverse", "tests/ReverseTest.php"),
                node("direct", "tests/DirectTest.php"),
                node("fixture", "tests/fixtures/data.php"),
            ],
            "links": [
                edge("test", "bridge", "EXTRACTED"),
                edge("bridge", "changed", "EXTRACTED"),
                edge("changed", "bridge", "EXTRACTED"),
                edge("changed", "reverse", "EXTRACTED"),
                edge("direct", "changed", "INFERRED"),
                edge("fixture", "changed", "EXTRACTED"),
            ]
        });
        let index = Index::read(graph.to_string().as_bytes(), Path::new("/work/shop")).unwrap();
        let changed = [
            ("src/alone.php", 'M'),
            ("src/changed.php", 'M'),
            ("src/deleted.php", 'D'),
            ("src/missing.php", 'A'),
            ("tests/CartTest.php", 'M'),
        ]
        .map(|(path, status)| (path.to_owned(), status));
        let run = |certain| {
            let mut view = json!({"mode":"changes", "scope":"src"});
            index.tests_reaching_changes(&mut view, &changed, "", certain);
            view["test_reach"].clone()
        };
        let rows = run(false);
        assert_eq!(rows[0]["total"], 0);
        assert_eq!(rows[1]["total"], 2);
        assert_eq!(rows[1]["tests"][0]["path"], "tests/DirectTest.php");
        assert_eq!(rows[1]["tests"][0]["steps"], 1);
        assert_eq!(rows[1]["tests"][1]["path"], "tests/CartTest.php");
        assert_eq!(rows[1]["tests"][1]["steps"], 2);
        assert_eq!(rows[2]["state"], "deleted");
        assert_eq!(rows[3]["state"], "missing");
        assert_eq!(rows[4]["state"], "test_file");
        assert_eq!(run(true)[1]["total"], 1);

        let net = Net::new(&index, false, false);
        let file = index.opened("src/changed.php").unwrap();
        assert!(!walk(&net, vec![file], &mut 1, |_, _, _| {}));
    }

    #[test]
    fn opens_an_inline_test_path_inside_the_changed_file() {
        let graph = json!({
            "nodes": [
                {"id":"work", "label":"work()", "source_file":"src/lib.rs", "source_location":"L1"},
                {"id":"test", "label":"exercise()", "source_file":"src/lib.rs", "source_location":"L4"}
            ],
            "links": [{"source":"test", "target":"work", "relation":"calls", "confidence":"EXTRACTED"}]
        });
        let mut index = Index::read(graph.to_string().as_bytes(), Path::new("/work/shop")).unwrap();
        index.revision_tests = Some(BTreeMap::from([("src/lib.rs".into(), vec![(3, 4)])]));
        let mut view = json!({"mode":"file","scope":"src/lib.rs"});
        index.tests_reaching_changes(&mut view, &[("src/lib.rs".into(), 'M')], "src/lib.rs", true);
        let test = &view["test_reach"][0]["tests"][0];
        assert_eq!(test["steps"], 1, "{view}");
        let route = index
            .path(
                "shop",
                test["path"].as_str().unwrap(),
                test["to"].as_str().unwrap(),
                &[],
                &|_| None,
                None,
                true,
            )
            .unwrap();
        assert_eq!(route["path"]["found"], true, "{route}");
        assert_eq!(route["path"]["reversed"], false);
    }

    #[test]
    fn the_changed_files_that_failing_tests_reach_come_first() {
        // cart uses price; CartTest uses cart, PriceTest uses price,
        // MailTest uses mail. All three of cart, price and mail changed,
        // and a fourth file was deleted.
        let node = |id: &str, file: &str| json!({"id": id, "label": id, "source_file": file});
        let edge = |from: &str, to: &str| json!({"source": from, "target": to, "relation": "calls", "confidence": "EXTRACTED"});
        let graph = json!({
            "nodes": [
                node("price", "src/price.php"), node("cart", "src/cart.php"),
                node("mail", "src/mail.php"), node("carttest", "tests/CartTest.php"),
                node("pricetest", "tests/PriceTest.php"), node("mailtest", "tests/MailTest.php"),
                node("helper", "tests/fixtures/CartFixture.php"),
            ],
            "links": [
                edge("cart", "price"), edge("carttest", "cart"), edge("pricetest", "price"),
                edge("mailtest", "mail"), edge("helper", "cart"),
            ]
        });
        let index = Index::read(graph.to_string().as_bytes(), Path::new("/work/shop")).unwrap();
        let changed = [
            ("src/price.php", 'M'),
            ("src/mail.php", 'M'),
            ("src/cart.php", 'M'),
            ("src/gone.php", 'D'),
        ]
        .map(|(path, status)| (path.to_owned(), status));
        let rank = |failing: &[&str]| {
            let mut view = json!({
                "mode": "changes",
                "nodes": [{"path": "src/price.php"}, {"path": "src/cart.php"}, {"path": "src/mail.php"}]
            });
            let failing = failing
                .iter()
                .map(|told| (*told).to_owned())
                .collect::<Vec<_>>();
            index.suspects(&mut view, &changed, &failing, false);
            view
        };
        // CartTest fails, named as a test runner names it. It reaches cart
        // in one link and price in two; PriceTest, which passes, reaches
        // price as well, so price comes second. MailTest reaches nothing
        // that fails.
        let view = rank(&["1) Tests\\Unit\\CartTest::testTotal"]);
        let suspects = &view["suspects"];
        assert_eq!(suspects["failing"], json!(["tests/CartTest.php"]));
        assert_eq!(suspects["tests"], 3);
        assert_eq!(
            suspects["rows"],
            json!([
                {"path": "src/cart.php", "status": "M", "rank": 1, "score": 1.0, "failing": 1, "passing": 0, "nearest": 1},
                {"path": "src/price.php", "status": "M", "rank": 2, "score": 0.71, "failing": 1, "passing": 1, "nearest": 2}
            ])
        );
        assert_eq!(suspects["clear"], 1);
        assert_eq!(suspects["more"], 0);
        assert_eq!(suspects["unmatched"], json!([]));
        assert_eq!(suspects["complete"], true);
        // The boxes of the picture carry their rank.
        assert_eq!(
            view["nodes"][1]["suspect"],
            json!({"rank": 1, "score": 1.0})
        );
        assert_eq!(view["nodes"][0]["suspect"]["rank"], 2);
        assert!(view["nodes"][2]["suspect"].is_null());
        // A change drawn by folder: the folder carries the best rank of the
        // ranked files it holds, and how many they are.
        let mut folders = json!({
            "mode": "changes",
            "nodes": [{"path": "src", "folder": true}, {"path": "tests", "folder": true}]
        });
        index.suspects(&mut folders, &changed, &["CartTest".to_owned()], false);
        assert_eq!(
            folders["nodes"][0]["suspect"],
            json!({"rank": 1, "score": 1.0, "files": 2})
        );
        assert!(folders["nodes"][1]["suspect"].is_null());
        // A fixture is no test, a path names its test, and what names no
        // test file is said to.
        let view = rank(&["tests/MailTest.php", "CartFixture", "Cart"]);
        assert_eq!(view["suspects"]["failing"], json!(["tests/MailTest.php"]));
        assert_eq!(
            view["suspects"]["unmatched"],
            json!(["CartFixture", "Cart"])
        );
        assert_eq!(view["suspects"]["rows"][0]["path"], "src/mail.php");
        assert_eq!(view["suspects"]["rows"][0]["score"], 1.0);
        assert_eq!(view["suspects"]["clear"], 2);
        // Both fail: cart is reached by one of the two, price by one of
        // the two and by a passing test, mail by the other alone.
        let view = rank(&["CartTest", "MailTest"]);
        let rows = view["suspects"]["rows"].as_array().unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| (
                    row["path"].as_str().unwrap(),
                    row["score"].as_f64().unwrap()
                ))
                .collect::<Vec<_>>(),
            [
                ("src/cart.php", 0.71),
                ("src/mail.php", 0.71),
                ("src/price.php", 0.5)
            ]
        );
        // Nothing failing, or no picture of a change: nothing is told.
        assert!(rank(&[])["suspects"].is_null());
        let mut folder = json!({"mode": "folder", "nodes": []});
        index.suspects(&mut folder, &changed, &["CartTest".to_owned()], false);
        assert!(folder["suspects"].is_null());
    }

    #[test]
    fn a_test_is_named_by_its_path_or_by_its_name_as_a_word_of_its_own() {
        let path = "tests/Unit/CartTest.php";
        for told in [
            "tests/Unit/CartTest.php",
            "FAILED tests/Unit/CartTest.php::test_total - AssertionError",
            "1) Tests\\Unit\\CartTest::testTotal",
            "CartTest",
            "  CartTest.testTotal",
        ] {
            assert!(names_test(told, path), "{told}");
        }
        for told in ["Cart", "CartTestHelper", "MyCartTest", "", "tests/Unit"] {
            assert!(!names_test(told, path), "{told}");
        }
        // A name too short to tell is matched by its path only.
        assert!(!names_test("ab", "tests/ab.py"));
        assert!(names_test("see tests/ab.py", "tests/ab.py"));
        // The name of a Rust or Python test file is its stem.
        assert!(names_test(
            "test_cart::total ... FAILED",
            "tests/test_cart.py"
        ));
        assert!(names_test("cart_test", "src/cart_test.go"));
    }
}
