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
}
