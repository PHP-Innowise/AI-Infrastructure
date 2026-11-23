use super::*;
#[test]
fn what_is_followed_is_read_from_a_request() {
    assert_eq!(Followed::parse("", "[\"src/a.php\"]", "[]").unwrap(), None);
    // A place named twice is unfolded two steps.
    assert_eq!(
        Followed::parse(
            "src/a.php",
            "[\"src/a.php\",\"src/b, c.php\",\"src/a.php\"]",
            ""
        )
        .unwrap(),
        Some(Followed {
            root: "src/a.php".to_owned(),
            out: vec![
                "src/a.php".to_owned(),
                "src/b, c.php".to_owned(),
                "src/a.php".to_owned()
            ],
            into: vec![],
        })
    );
    let many = serde_json::to_string(&vec!["x"; UNFOLDED + 1]).unwrap();
    let long = serde_json::to_string(&["x".repeat(1025)]).unwrap();
    for wrong in [
        many.as_str(),
        long.as_str(),
        "src/a.php",
        "[1]",
        "[\"\"]",
        "{}",
    ] {
        assert!(Followed::parse("src/a.php", wrong, "").is_err(), "{wrong}");
        assert!(Followed::parse("src/a.php", "", wrong).is_err(), "{wrong}");
    }
    // The map's route lets through every part of a request its handler
    // reads; a part it does not name is refused before the handler.
    let route = crate::http::ROUTES
        .iter()
        .find(|route| route.operation == "graph.map")
        .unwrap();
    for part in [
        "scope",
        "focus",
        "certain",
        "filter",
        "target",
        "follow",
        "out",
        "into",
        "notests",
        "spread",
        "from",
        "to",
        "path",
        "shared",
        "side",
        "skip",
        "cycle",
        "base",
        "patterns",
        "pattern",
        "history",
        "modules",
        "no_incoming",
        "launch",
        "compare_launch",
        "scenario",
        "focus_session",
        "whole",
    ] {
        assert!(route.query.contains(&part), "{part}");
    }
    // So does the route of the search.
    let route = crate::http::ROUTES
        .iter()
        .find(|route| route.operation == "graph.find")
        .unwrap();
    assert_eq!(
        (route.method, route.pattern, route.query),
        (
            "GET",
            "/api/projects/{project_id}/graph/find",
            &["query", "target", "notests"][..]
        )
    );
    let changes_route = crate::http::ROUTES
        .iter()
        .find(|route| route.operation == "graph.changes")
        .unwrap();
    assert_eq!(
        (
            changes_route.method,
            changes_route.pattern,
            changes_route.status,
            changes_route.token,
            changes_route.query
        ),
        (
            "GET",
            "/api/projects/{project_id}/graph/changes",
            200,
            false,
            &["target", "from", "to", "focus_session", "notests"][..],
        )
    );
    let impact_route = crate::http::ROUTES
        .iter()
        .find(|route| route.operation == "graph.file_impact")
        .unwrap();
    assert_eq!(
        (
            impact_route.method,
            impact_route.pattern,
            impact_route.status,
            impact_route.token,
            impact_route.query
        ),
        (
            "GET",
            "/api/projects/{project_id}/graph/file-impact",
            200,
            false,
            &[
                "path",
                "target",
                "from",
                "to",
                "focus_session",
                "agent",
                "certain"
            ][..]
        )
    );
    // The folders spread out in a picture come as the same kind of list.
    assert_eq!(
        places("[\"src\",\"src/b, c\"]", "many", "wrong").unwrap(),
        ["src", "src/b, c"]
    );
    assert!(places("", "many", "wrong").unwrap().is_empty());
    assert!(places(&many, "many", "wrong").is_err());
    assert!(places("src", "many", "wrong").is_err());
}
#[test]
fn progress_lines_of_both_releases_are_read() {
    assert_eq!(
        signal("  AST extraction: 500/994 uncached files (50%) [14 workers]"),
        Some(Signal::Reading {
            done: 500,
            total: 994
        })
    );
    assert_eq!(
        signal("  AST extraction: 1000/1000 files (100%)"),
        Some(Signal::Reading {
            done: 1000,
            total: 1000
        })
    );
    assert_eq!(
        signal("  AST cache: 3514/3520 files served from cache (99%); 6 to extract"),
        Some(Signal::Pending(6))
    );
    assert_eq!(
        signal("  AST cache: 12/12 files served from cache (100%); 0 to extract"),
        Some(Signal::Pending(0))
    );
    assert_eq!(
        signal("[graphify watch] Rebuilt: 22546 nodes, 50403 edges, 1053 communities"),
        Some(Signal::Built {
            nodes: 22546,
            edges: 50403
        })
    );
    assert_eq!(
        signal("[graphify watch] Rebuilt (no clustering): 50900 nodes, 102253 edges"),
        Some(Signal::Built {
            nodes: 50900,
            edges: 102253
        })
    );
    for other in [
        "",
        "Updating code graph in /repo (no LLM needed)...",
        "AST extraction: many/files",
        "AST cache: warming",
        "Rebuilt: nothing",
        "[graphify watch] No code-graph topology changes detected; outputs left untouched.",
    ] {
        assert_eq!(signal(other), None, "{other}");
    }
}
fn reading(points: &[(f64, u64)], total: u64) -> Observed {
    let mut seen = Observed::default();
    for (at, done) in points {
        seen.note(*at, Signal::Reading { done: *done, total });
    }
    seen
}
#[test]
fn a_first_build_has_no_estimate_until_two_counts_were_seen() {
    assert_eq!(expected(2.0, &Observed::default(), None), None);
    assert_eq!(expected(3.2, &reading(&[(3.1, 100)], 3520), None), None);
    assert_eq!(
        progress(3.2, &reading(&[(3.1, 100)], 3520), None, 0),
        (None, None, false)
    );
}
#[test]
fn a_first_build_extends_the_reading_rate_and_adds_the_silent_part() {
    // 100 files in 0.2 s: the other 3,320 need 6.64 s more, ending at 9.94 s.
    let seen = reading(&[(3.1, 100), (3.3, 200)], 3520);
    let total = expected(3.3, &seen, None).unwrap();
    assert!((total - 9.94 * (1.0 + TAIL_RATIO)).abs() < 0.01, "{total}");
    let (percent, left, overdue) = progress(3.3, &seen, None, 0);
    assert_eq!((percent, overdue), (Some(6), false));
    assert_eq!(left, Some((total - 3.3).ceil() as u64));
}
#[test]
fn the_silent_part_comes_from_the_projects_previous_build() {
    let past = Some(History {
        seconds: 44.0,
        tail: Some(38.0),
    });
    let mut seen = reading(&[(3.1, 100), (9.3, 3520)], 3520);
    assert_eq!(expected(20.0, &seen, past), Some(9.3 + 38.0));
    assert_eq!(progress(20.0, &seen, past, 0), (Some(42), Some(28), false));
    // Nothing left to read: the cache line ends the measured part.
    seen = Observed::default();
    seen.note(6.4, Signal::Pending(0));
    assert_eq!(expected(7.0, &seen, past), Some(6.4 + 38.0));
    // A build that printed no count at all is as long as the previous one.
    assert_eq!(expected(5.0, &Observed::default(), past), Some(44.0));
    // A previous build without a measured tail bounds the silent part.
    let whole = Some(History {
        seconds: 14.0,
        tail: None,
    });
    assert_eq!(
        expected(1.0, &reading(&[(0.9, 3)], 3), whole),
        Some(0.9 + 14.0)
    );
}
#[test]
fn percent_never_falls_and_stops_short_of_done_when_overdue() {
    let past = Some(History {
        seconds: 10.0,
        tail: None,
    });
    let seen = Observed::default();
    assert_eq!(progress(5.0, &seen, past, 0), (Some(50), Some(5), false));
    assert_eq!(progress(5.0, &seen, past, 70), (Some(70), Some(5), false));
    assert_eq!(progress(12.0, &seen, past, 70), (Some(99), Some(0), true));
}
#[test]
fn a_graph_question_is_told_from_other_commands() {
    for command in [
        "graphify query \"where is the session started\" --graph /state/graphs/ab/live/graph.json",
        "cd src && /home/u/.local/bin/graphify explain \"Sessions\" --graph g.json",
        "'/opt/my tools/graphify' path A B --graph g.json | head -40",
    ] {
        assert!(asked(command), "{command}");
    }
    for command in [
        "",
        "graphify update .",
        "rg graphify query",
        "echo graphify",
        "cat graphify-out/GRAPH_REPORT.md",
    ] {
        assert!(!asked(command), "{command}");
    }
}
/// The prompt shows a path inside a command, and Claude Code compares its
/// permission with the command as written, so the spelling has to survive
/// the shell the agent uses.
#[test]
fn a_path_is_one_shell_word_on_either_system() {
    assert_eq!(
        spelled("/home/me/.state/graph.json", false),
        "/home/me/.state/graph.json"
    );
    assert_eq!(
        spelled("/home/me/my state/it's.json", false),
        "'/home/me/my state/it'\\''s.json'"
    );
    assert_eq!(spelled("/tmp/a\\b", false), "'/tmp/a\\b'");
    assert_eq!(
        spelled("C:\\Users\\me\\AppData\\graph.json", true),
        "C:/Users/me/AppData/graph.json"
    );
    assert_eq!(
        spelled("C:\\Users\\Ann Lee\\graph.json", true),
        "\"C:/Users/Ann Lee/graph.json\""
    );
    assert_eq!(
        spelled("\\\\server\\share\\graph.json", true),
        "\"\\\\server\\share\\graph.json\""
    );
}
/// The launch of a session whose project has a published graph: Claude
/// Code gets the three rules beside the settings it already had, every
/// host gets the switch that keeps a question from rewriting skills.
#[test]
fn a_launch_carries_the_permission_for_claude_and_the_switch_for_any_host() {
    let root = tempfile::tempdir().unwrap();
    let store = SessionStore::open(&root.path().join("state")).unwrap();
    let folder = root.path().join("project");
    std::fs::create_dir_all(&folder).unwrap();
    let project = store.register(&folder).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let command = |argv: &[&str]| infra_host::provider::PreparedCommand {
        argv: argv.iter().map(|word| (*word).to_owned()).collect(),
        environment: std::collections::BTreeMap::from([("KEPT".to_owned(), "1".to_owned())]),
        input: vec![],
        readable: vec![],
        remove: vec![],
        prompt_prefix: String::new(),
    };
    let before =
        r#"{"env":{"A":"1"},"permissions":{"allow":["Bash(php:*)"],"deny":["Read(.env)"]}}"#;
    // No graph: the launch is left as it was.
    let mut untouched = command(&["claude", "--settings", before, "--resume", "id"]);
    prepare(&store, &project, "claude", &mut untouched).unwrap();
    assert_eq!(untouched.argv[2], before);
    assert_eq!(untouched.environment.len(), 1);

    let tool = root.path().join("graphify-tool");
    publish_for_tests(&store, &project, &tool);
    let rules = grant(&store, &project).unwrap().unwrap().rules;
    assert_eq!(rules.len(), 3);
    let mut claude = command(&["claude", "--settings", before, "--resume", "id"]);
    prepare(&store, &project, "claude", &mut claude).unwrap();
    let settings: Value = serde_json::from_str(&claude.argv[2]).unwrap();
    let mut allow = vec![json!("Bash(php:*)")];
    allow.extend(rules.iter().map(|rule| json!(rule)));
    assert_eq!(settings["permissions"]["allow"], Value::Array(allow));
    assert_eq!(settings["permissions"]["deny"], json!(["Read(.env)"]));
    assert_eq!(settings["env"], json!({"A": "1"}));
    assert_eq!(claude.argv[3..], ["--resume", "id"]);
    assert_eq!(claude.environment["GRAPHIFY_NO_AUTO_REFRESH"], "1");
    assert_eq!(claude.environment["KEPT"], "1");
    // The same launch prepared again repeats no rule.
    prepare(&store, &project, "claude", &mut claude).unwrap();
    let again: Value = serde_json::from_str(&claude.argv[2]).unwrap();
    assert_eq!(again, settings);
    // A Claude command without settings gets its own.
    let mut bare = command(&["claude", "--print"]);
    prepare(&store, &project, "claude", &mut bare).unwrap();
    assert_eq!(bare.argv[2], "--settings");
    assert_eq!(
        serde_json::from_str::<Value>(&bare.argv[3]).unwrap()["permissions"]["allow"],
        json!(rules)
    );
    // Codex runs what its sandbox allows: no rule, the switch only.
    let mut codex = command(&["codex", "exec"]);
    prepare(&store, &project, "codex", &mut codex).unwrap();
    assert_eq!(codex.argv, ["codex", "exec"]);
    assert_eq!(codex.environment["GRAPHIFY_NO_AUTO_REFRESH"], "1");
    // Settings that are not an object are refused, not replaced.
    let mut broken = command(&["claude", "--settings", "[]"]);
    assert!(prepare(&store, &project, "claude", &mut broken).is_err());
}
#[test]
fn a_release_is_found_in_version_output() {
    assert_eq!(release("graphify 0.9.105\n").as_deref(), Some("0.9.105"));
    assert_eq!(release("0.7.13\n").as_deref(), Some("0.7.13"));
    assert_eq!(release("error: unknown command '--version'"), None);
}
#[cfg(unix)]
mod builds {
    use super::super::*;
    use std::os::unix::fs::PermissionsExt;
    const TOOL: &str = r#"#!/bin/sh
here=$(dirname "$0")
if [ "$GRAPHIFY_NO_AUTO_REFRESH" != "1" ]; then echo "installed skills would be refreshed" >&2; exit 6; fi
if [ "$1" = "--version" ]; then echo "graphify 9.9.9"; exit 0; fi
echo run >> "$here/runs"
if [ -e "$GRAPHIFY_OUT/graph.json" ]; then echo "previous graph still present" >&2; exit 4; fi
if [ -n "$OPENAI_API_KEY" ]; then echo "provider key leaked" >&2; exit 5; fi
echo "  AST extraction: 1/2 uncached files (50%) [2 workers]"
if [ -e "$2/old" ]; then mkdir -p "$2/graphify-out"; echo overwritten > "$2/graphify-out/manifest.json"; fi
if [ -e "$2/slow" ]; then sleep 30; fi
if [ -e "$2/fail" ]; then echo "boom" >&2; exit 3; fi
echo "  AST extraction: 2/2 uncached files (100%) [2 workers]"
mkdir -p "$GRAPHIFY_OUT/cache"
if [ -e "$2/graph.fixture" ]; then cp "$2/graph.fixture" "$GRAPHIFY_OUT/graph.json"; else printf '{"nodes":[],"links":[]}' > "$GRAPHIFY_OUT/graph.json"; fi
echo "[graphify watch] Rebuilt: 7 nodes, 9 edges, 2 communities"
"#;
    struct Fixture {
        _root: tempfile::TempDir,
        store: SessionStore,
        graphs: Arc<Graphs>,
        project: String,
        folder: PathBuf,
        tool: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let store = SessionStore::open(&root.path().join("state")).unwrap();
            let folder = root.path().join("project");
            std::fs::create_dir_all(&folder).unwrap();
            let project = store.register(&folder).unwrap()["id"]
                .as_str()
                .unwrap()
                .to_owned();
            let tool = root.path().join("tool").join("graphify");
            std::fs::create_dir_all(tool.parent().unwrap()).unwrap();
            // The tests run on threads of one process, and a child that
            // another test starts meanwhile inherits whatever is open
            // here until it execs. Were the tool written here, such a
            // child could hold it open for writing, and starting the
            // tool then fails with "Text file busy". So its text is
            // written beside it, and a process of its own makes the
            // tool of that.
            let text = tool.with_extension("text");
            std::fs::write(&text, TOOL).unwrap();
            let copied = Command::new("cp")
                .arg("--")
                .arg(&text)
                .arg(&tool)
                .status()
                .unwrap();
            assert!(copied.success());
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
            let graphs = Graphs::with(store.clone(), Some(tool.clone()), Duration::ZERO);
            Self {
                _root: root,
                store,
                graphs,
                project,
                folder,
                tool,
            }
        }
        fn runs(&self) -> usize {
            std::fs::read_to_string(self.tool.with_file_name("runs"))
                .map(|text| text.lines().count())
                .unwrap_or(0)
        }
        fn settled(&self) -> Value {
            let started = Instant::now();
            loop {
                let status = self.graphs.status(&self.project, false).unwrap();
                // A build that was switched off is still ending its tool.
                if status["build"].is_null() {
                    return status;
                }
                assert!(started.elapsed() < Duration::from_secs(20), "{status}");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        fn git(&self, arguments: &[&str]) {
            let done = Command::new("git")
                .args(arguments)
                .current_dir(&self.folder)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@example.test")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@example.test")
                .output()
                .unwrap();
            assert!(done.status.success(), "{arguments:?}: {done:?}");
        }
        fn said(&self, arguments: &[&str]) -> String {
            let done = Command::new("git")
                .args(arguments)
                .current_dir(&self.folder)
                .output()
                .unwrap();
            assert!(done.status.success(), "{arguments:?}: {done:?}");
            String::from_utf8_lossy(&done.stdout).trim().to_owned()
        }
        fn write(&self, name: &str, text: &str) {
            std::fs::write(self.folder.join(name), text).unwrap();
        }
        fn commit(&self, message: &str) -> String {
            self.git(&["add", "--all"]);
            self.git(&["-c", "commit.gpgsign=false", "commit", "-q", "-m", message]);
            self.said(&["rev-parse", "HEAD"])
        }
        /// `main` with a cart that uses a price, `origin/main` standing
        /// there, and a `feature` branch whose order uses the cart. Each
        /// commit carries the graph the tool gives for its files. Returns
        /// both commits; `main` is checked out.
        fn branched(&self) -> (String, String) {
            self.git(&["init", "-q", "--initial-branch=main"]);
            self.write("cart.php", "<?php // cart\n");
            self.write("price.php", "<?php // price\n");
            self.write(
                    "graph.fixture",
                    r#"{"nodes":[{"id":"cart","source_file":"cart.php"},{"id":"price","source_file":"price.php"}],"links":[{"source":"cart","target":"price","relation":"calls","source_file":"cart.php"}]}"#,
                );
            let main = self.commit("one");
            self.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
            self.git(&["switch", "-q", "-c", "feature"]);
            self.write("cart.php", "<?php // cart, changed\n");
            self.write("order.php", "<?php // order\n");
            self.write(
                    "graph.fixture",
                    r#"{"nodes":[{"id":"cart","source_file":"cart.php"},{"id":"price","source_file":"price.php"},{"id":"order","source_file":"order.php"}],"links":[{"source":"cart","target":"price","relation":"calls","source_file":"cart.php"},{"source":"order","target":"cart","relation":"calls","source_file":"order.php"}]}"#,
                );
            let feature = self.commit("two");
            self.git(&["switch", "-q", "main"]);
            (main, feature)
        }
        /// The map of a chosen change once its graph is built or refused.
        fn mapped(&self, target: &Target) -> Value {
            let started = Instant::now();
            loop {
                let view = self
                    .graphs
                    .map(&self.project, Some(target), &Shown::default())
                    .unwrap();
                if view["state"] != "building" {
                    return view;
                }
                assert!(started.elapsed() < Duration::from_secs(20), "{view}");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        fn revisions(&self) -> Vec<String> {
            let mut names =
                std::fs::read_dir(folder(&self.store.state, &self.project).join("revisions"))
                    .map(|entries| {
                        entries
                            .flatten()
                            .map(|entry| entry.file_name().to_string_lossy().into_owned())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
            names.sort();
            names
        }
        fn session(&self, status: &str) {
            self.store
                    .connection()
                    .unwrap()
                    .execute(
                        "INSERT INTO sessions(id,title,project_id,project_path,provider,mode,workflow,project_context,status,created_at,updated_at)VALUES(?1,'t',?2,?3,'claude','edit','native',0,?4,'2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')",
                        rusqlite::params![store::id(), self.project, self.folder.to_string_lossy(), status],
                    )
                    .unwrap();
        }
    }
    #[test]
    fn a_pr_base_cycle_and_its_links_use_the_pinned_revision() {
        let fixture = Fixture::new();
        let (base, head) = fixture.branched();
        let target = Target::Pull(1);
        let resolved = Resolved {
            target: target.clone(),
            kind: "pull",
            name: "#1".to_owned(),
            title: None,
            pull: Some(1),
            head: None,
            base: Some(graph_changes::Side {
                name: "main".to_owned(),
                commit: base.clone(),
            }),
            revision: Some(head.clone()),
            prefix: String::new(),
            files: Vec::new(),
            notes: Vec::new(),
        };
        lock(&fixture.graphs.chosen)
            .unwrap()
            .insert((fixture.project.clone(), target.text()), Arc::new(resolved));
        let put = |commit: &str, edges: &[(&str, &str, &str)]| {
            let graph = revision(&fixture.store.state, &fixture.project, commit);
            std::fs::create_dir_all(graph.parent().unwrap()).unwrap();
            let links = edges.iter().map(|(from, to, confidence)| {
                    json!({"source": from, "target": to, "relation": "calls", "confidence": confidence})
                }).collect::<Vec<_>>();
            std::fs::write(graph, json!({
                    "nodes": (["a", "b", "c"].map(|node| json!({"id": node, "source_file": format!("{node}.php")}))),
                    "links": links,
                }).to_string()).unwrap();
        };
        put(
            &base,
            &[
                ("a", "b", "EXTRACTED"),
                ("b", "c", "EXTRACTED"),
                ("c", "a", "INFERRED"),
            ],
        );
        put(&head, &[("a", "b", "EXTRACTED"), ("b", "a", "EXTRACTED")]);
        let map = |shown| fixture.graphs.map(&fixture.project, Some(&target), &shown);
        let current = map(Shown {
            cycle: Some("a.php"),
            ..Shown::default()
        })
        .unwrap();
        assert_eq!(current["cycle"]["steps"], 2);
        let comparison = &current["cycle"]["comparison"];
        assert_eq!(comparison["revision"], base);
        assert_eq!(comparison["place"], "a.php");
        let historical = Shown {
            cycle: Some("a.php"),
            base: Some(&base),
            notests: true,
            ..Shown::default()
        };
        let earlier = map(historical).unwrap();
        assert_eq!(earlier["base"]["commit"], &base[..12]);
        assert_eq!(earlier["cycle"]["steps"], 3);
        assert_eq!(earlier["notests"], true);
        assert_eq!(
            map(Shown {
                certain: true,
                ..historical
            })
            .unwrap()["cycle"]["steps"],
            0
        );
        let link = Shown {
            link: Some(("b.php", "c.php")),
            ..Shown::default()
        };
        assert_eq!(map(link).unwrap()["links"], 0);
        assert_eq!(
            map(Shown {
                base: Some(&base),
                ..link
            })
            .unwrap()["links"],
            1
        );
        assert!(
            map(Shown {
                base: Some(&head),
                ..historical
            })
            .is_err()
        );
        assert!(
            fixture
                .graphs
                .map(&fixture.project, None, &historical)
                .is_err()
        );
        std::fs::remove_file(revision(&fixture.store.state, &fixture.project, &base)).unwrap();
        assert!(map(historical).is_err());
    }
    /// A shop in three layers, one type in each file.
    const SHOP: [(&str, &str); 5] = [
        ("controller", "src/Controller/OrderController.php"),
        ("service", "src/Service/OrderService.php"),
        ("mailer", "src/Service/MailService.php"),
        ("order", "src/Entity/Order.php"),
        ("line", "src/Entity/Line.php"),
    ];
    /// What the shop needs in the order of its layers.
    const NEEDS: [(&str, &str); 3] = [
        ("controller", "service"),
        ("service", "order"),
        ("service", "line"),
    ];
    /// A graph of types, each named with its file, and the calls between
    /// them. A name may carry the line it stands at, after `:`, and a
    /// call that ends in `?` is one the tool only inferred.
    fn sketch(files: &[(&str, &str)], calls: &[(&str, &str)]) -> String {
        let nodes = files
            .iter()
            .map(|(id, path)| match id.split_once(':') {
                Some((id, line)) => json!({
                    "id": id, "source_file": path, "source_location": format!("L{line}"),
                }),
                None => json!({"id": id, "source_file": path}),
            })
            .collect::<Vec<_>>();
        let links = calls
            .iter()
            .map(|(from, to)| {
                let (to, confidence) = match to.strip_suffix('?') {
                    Some(to) => (to, "INFERRED"),
                    None => (*to, "EXTRACTED"),
                };
                json!({
                    "source": from, "target": to, "relation": "calls",
                    "confidence": confidence,
                })
            })
            .collect::<Vec<_>>();
        json!({"nodes": nodes, "links": links}).to_string()
    }
    /// Writes such a graph where the graph of a commit is kept.
    fn draw(graph: &Path, files: &[(&str, &str)], calls: &[(&str, &str)]) {
        std::fs::create_dir_all(graph.parent().unwrap()).unwrap();
        std::fs::write(graph, sketch(files, calls)).unwrap();
    }
    /// Pull request 1 as chosen, of `head` over `base`, with the files
    /// it is said to have changed; Git is not asked.
    fn pull(
        fixture: &Fixture,
        base: Option<&str>,
        head: &str,
        files: Vec<graph_changes::Change>,
    ) -> Target {
        let target = Target::Pull(1);
        let resolved = Resolved {
            target: target.clone(),
            kind: "pull",
            name: "#1".to_owned(),
            title: None,
            pull: Some(1),
            head: None,
            base: base.map(|commit| graph_changes::Side {
                name: "main".to_owned(),
                commit: commit.to_owned(),
            }),
            revision: Some(head.to_owned()),
            prefix: String::new(),
            files,
            notes: Vec::new(),
        };
        lock(&fixture.graphs.chosen)
            .unwrap()
            .insert((fixture.project.clone(), target.text()), Arc::new(resolved));
        target
    }
    /// Waits for the build of a commit's graph to end. It publishes the
    /// graph first, and only then prunes the graphs of other commits and
    /// gives its place to the next build.
    fn rested(fixture: &Fixture) {
        let started = Instant::now();
        while !lock(&fixture.graphs.revisions).unwrap().is_empty() {
            assert!(started.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    /// The picture of the layers of the whole project.
    fn layers() -> Shown<'static> {
        Shown {
            pattern: Some("layers@"),
            ..Shown::default()
        }
    }
    #[test]
    fn the_layers_of_a_change_say_what_it_did_to_their_order() {
        let fixture = Fixture::new();
        let (base, head) = fixture.branched();
        let at = |commit: &str| revision(&fixture.store.state, &fixture.project, commit);
        // At the base a line of the model asks the mailer; at the head
        // it no longer does, and the order asks the service.
        draw(
            &at(&base),
            &SHOP,
            &[&NEEDS[..], &[("line", "mailer")]].concat(),
        );
        draw(
            &at(&head),
            &SHOP,
            &[&NEEDS[..], &[("order", "service")]].concat(),
        );
        let target = pull(&fixture, Some(&base), &head, Vec::new());
        let map = |shown| fixture.graphs.map(&fixture.project, Some(&target), &shown);
        let view = map(layers()).unwrap();
        assert_eq!(view["mode"], "pattern");
        let change = &view["pattern"]["change"];
        assert_eq!(change["state"], "ready");
        assert_eq!(
            change["base"],
            json!({"name": "main", "commit": &base[..12]})
        );
        assert_eq!(change["added"]["pairs"], 1);
        assert_eq!(
            change["added"]["rows"][0]["from"]["path"],
            "src/Entity/Order.php"
        );
        assert_eq!(change["removed"]["pairs"], 1);
        assert_eq!(
            change["removed"]["rows"][0]["to"]["path"],
            "src/Service/MailService.php"
        );
        assert_eq!(view["pattern"]["against"]["rows"][0]["added"], true);
        // A selected link is compared as the picture is.
        let link = map(Shown {
            focus: Some("layer:model>layer:services"),
            ..layers()
        })
        .unwrap();
        assert_eq!(link["focus"]["rows"][0]["added"], true);
        assert_eq!(link["pattern"]["change"]["added"]["pairs"], 1);
        // Any other picture of the change is not compared.
        let plain = map(Shown::default()).unwrap();
        assert!(plain["pattern"].is_null());
        // Without a base to measure from the picture says that it
        // cannot tell, and stays what it was.
        pull(&fixture, None, &head, Vec::new());
        let view = map(layers()).unwrap();
        assert_eq!(
            view["pattern"]["change"],
            json!({"state": "unavailable", "reason": "no_base", "base": null})
        );
        assert_eq!(view["pattern"]["against"]["pairs"], 1);
        assert!(view["pattern"]["against"]["rows"][0]["added"].is_null());
        // A base whose graph is not there yet is being prepared: the
        // picture says so, and has what it was asked for once the graph
        // is built. The tool gives that commit a graph without layers,
        // so every pair against the order is then the change's own.
        pull(&fixture, Some(&base), &head, Vec::new());
        std::fs::remove_file(at(&base)).unwrap();
        // The graph of the change is the oldest of more graphs than
        // are kept: it stays, as the pair of the base built for it.
        std::fs::File::options()
            .write(true)
            .open(at(&head))
            .unwrap()
            .set_modified(std::time::SystemTime::now() - Duration::from_secs(3600))
            .unwrap();
        for other in ["a", "b", "c"] {
            draw(&at(&other.repeat(40)), &SHOP, &NEEDS);
        }
        let waiting = map(layers()).unwrap();
        assert_eq!(waiting["pattern"]["change"]["state"], "building");
        assert_eq!(waiting["pattern"]["change"]["base"]["commit"], &base[..12]);
        assert!(waiting["pattern"]["change"]["added"].is_null());
        let started = Instant::now();
        let ready = loop {
            let view = map(layers()).unwrap();
            if view["pattern"]["change"]["state"] != "building" {
                break view;
            }
            assert!(started.elapsed() < Duration::from_secs(20), "{view}");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(ready["pattern"]["change"]["state"], "ready");
        assert_eq!(ready["pattern"]["change"]["added"]["pairs"], 1);
        assert_eq!(ready["pattern"]["change"]["removed"]["pairs"], 0);
        rested(&fixture);
        let kept = fixture.revisions();
        assert_eq!(kept.len(), REVISIONS, "{kept:?}");
        assert!(kept.contains(&head) && kept.contains(&base), "{kept:?}");
    }
    #[test]
    fn the_layers_of_a_change_are_compared_as_far_as_its_two_ends_can_be_read() {
        let fixture = Fixture::new();
        let (base, head) = fixture.branched();
        let at = |commit: &str| revision(&fixture.store.state, &fixture.project, commit);
        let did = |target: &Target, shown: Shown| {
            // The graphs of the two commits are rewritten between the
            // cases, and may be of one size and one moment.
            fixture.graphs.maps.forget(&fixture.project);
            let view = fixture
                .graphs
                .map(&fixture.project, Some(target), &shown)
                .unwrap();
            let change = view["pattern"]["change"].clone();
            (
                change["state"].as_str().unwrap_or("").to_owned(),
                change["reason"].as_str().unwrap_or("").to_owned(),
                change["added"]["pairs"].as_u64().unwrap_or(0),
                change["removed"]["pairs"].as_u64().unwrap_or(0),
            )
        };
        let told = |state: &str, reason: &str, added: u64, removed: u64| {
            (state.to_owned(), reason.to_owned(), added, removed)
        };
        // The order asks the service at both ends, under the name it
        // had at the base and under the one the change gave it.
        let old = SHOP.map(|(id, path)| match id {
            "order" => (id, "src/Entity/Purchase.php"),
            _ => (id, path),
        });
        let against = [&NEEDS[..], &[("order", "service")]].concat();
        draw(&at(&base), &old, &against);
        draw(&at(&head), &SHOP, &against);
        let moved = |status: char| {
            vec![graph_changes::Change {
                path: "src/Entity/Order.php".to_owned(),
                status,
                from: Some("src/Entity/Purchase.php".to_owned()),
            }]
        };
        // Known to be one file, it is one pair, and the change did
        // nothing to the order. A copy is a file of its own, and so is
        // one of which nothing says where it came from.
        let renamed = pull(&fixture, Some(&base), &head, moved('R'));
        assert_eq!(did(&renamed, layers()), told("ready", "", 0, 0));
        let copied = pull(&fixture, Some(&base), &head, moved('C'));
        assert_eq!(did(&copied, layers()), told("ready", "", 1, 1));
        let unknown = pull(&fixture, Some(&base), &head, Vec::new());
        assert_eq!(did(&unknown, layers()), told("ready", "", 1, 1));
        // What the tool only inferred is left out of both ends where
        // the picture leaves it out.
        let sure = Shown {
            certain: true,
            ..layers()
        };
        draw(&at(&base), &SHOP, &NEEDS);
        draw(
            &at(&head),
            &SHOP,
            &[&NEEDS[..], &[("order", "service?")]].concat(),
        );
        assert_eq!(did(&unknown, layers()), told("ready", "", 1, 0));
        assert_eq!(did(&unknown, sure), told("ready", "", 0, 0));
        // Tests inside a Rust file are told by the spans kept beside
        // the graph of a commit. A graph without them cannot be read
        // without its tests, at either end, and with them it can.
        let plain = Shown {
            notests: true,
            ..layers()
        };
        let rust = |spans: bool, commit: &str| {
            let mut files = SHOP.to_vec();
            files.push(("check:30", "src/Entity/order.rs"));
            files.push(("total:4", "src/Entity/order.rs"));
            // A test inside the model asks the service.
            draw(
                &at(commit),
                &files,
                &[&NEEDS[..], &[("check", "service")]].concat(),
            );
            let beside = at(commit).with_file_name(crate::graph_map::TEST_SPANS);
            if spans {
                std::fs::write(beside, r#"{"src/Entity/order.rs":[[25,40]]}"#).unwrap();
            } else {
                let _ = std::fs::remove_file(beside);
            }
        };
        let target = pull(&fixture, Some(&base), &head, Vec::new());
        draw(&at(&head), &SHOP, &against);
        rust(false, &base);
        assert_eq!(did(&target, plain), told("unavailable", "tests", 0, 0));
        assert_eq!(did(&target, layers()), told("ready", "", 1, 1));
        rust(false, &head);
        rust(true, &base);
        assert_eq!(did(&target, plain), told("unavailable", "tests", 0, 0));
        assert_eq!(did(&target, layers()), told("ready", "", 0, 0));
        // With the spans at both ends the test is left out of both:
        // without it nothing runs against the order, and with it one
        // pair does that the change neither added nor took away.
        rust(true, &head);
        assert_eq!(did(&target, plain), told("ready", "", 0, 0));
        assert_eq!(did(&target, layers()), told("ready", "", 0, 0));
        let view = fixture
            .graphs
            .map(&fixture.project, Some(&target), &plain)
            .unwrap();
        assert_eq!(view["pattern"]["against"]["pairs"], 0);
        // A base whose graph could not be built is not built again: the
        // picture says that it cannot compare.
        std::fs::remove_file(at(&base)).unwrap();
        lock(&fixture.graphs.refused)
            .unwrap()
            .insert((fixture.project.clone(), base.clone()), "boom".to_owned());
        assert_eq!(did(&target, layers()), told("unavailable", "", 0, 0));
        assert!(lock(&fixture.graphs.revisions).unwrap().is_empty());
        let view = fixture
            .graphs
            .map(&fixture.project, Some(&target), &layers())
            .unwrap();
        assert_eq!(view["pattern"]["change"]["base"]["commit"], &base[..12]);
        // The project's own graph, built from a clean checkout of the
        // base, stands for the graph of that commit. It is read with
        // the files of the checkout and needs no spans beside it.
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        let mut files = SHOP.to_vec();
        files.push(("total:4", "src/Entity/order.rs"));
        draw(
            &live(&fixture.store.state, &fixture.project),
            &files,
            &[&NEEDS[..], &[("line", "mailer")]].concat(),
        );
        assert_eq!(did(&target, plain), told("ready", "", 0, 1));
        // Until the graph of the change itself is there the picture is
        // of the project's own graph, and waits for it before telling;
        // a change whose graph could not be built is never told.
        std::fs::remove_file(at(&head)).unwrap();
        draw(
            &live(&fixture.store.state, &fixture.project),
            &SHOP,
            &against,
        );
        let first = fixture
            .graphs
            .map(&fixture.project, Some(&target), &layers())
            .unwrap();
        assert_eq!(first["preliminary"], true);
        assert_eq!(first["pattern"]["against"]["pairs"], 1);
        assert_eq!(first["pattern"]["change"], json!({"state": "waiting_head"}));
        lock(&fixture.graphs.refused)
            .unwrap()
            .insert((fixture.project.clone(), head.clone()), "boom".to_owned());
        assert_eq!(did(&target, layers()), told("unavailable", "head", 0, 0));
    }
    #[test]
    fn the_layers_of_the_checkouts_own_work_are_compared_with_where_it_started() {
        let fixture = Fixture::new();
        let (main, _) = fixture.branched();
        // The tool gives each commit the graph its `graph.fixture` holds:
        // a shop in layers is committed, one of its files in Rust, and
        // then, not committed, its order asks the service.
        let mut files = SHOP.to_vec();
        files.push(("total:4", "src/Entity/order.rs"));
        fixture.write("graph.fixture", &sketch(&files, &NEEDS));
        let head = fixture.commit("a shop in layers");
        let back = [&NEEDS[..], &[("order", "service")]].concat();
        fixture.write("graph.fixture", &sketch(&files, &back));
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        // The picture, once the graph of the base is built.
        let asked = |target: &Target, shown: Shown| {
            let started = Instant::now();
            loop {
                let view = fixture
                    .graphs
                    .map(&fixture.project, Some(target), &shown)
                    .unwrap();
                if view["pattern"]["change"]["state"] != "building" {
                    return view;
                }
                assert!(started.elapsed() < Duration::from_secs(20), "{view}");
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        let told = |target: &Target| asked(target, layers());
        let did = |view: &Value| {
            let change = &view["pattern"]["change"];
            (
                change["state"].as_str().unwrap_or("").to_owned(),
                change["base"]["commit"].as_str().unwrap_or("").to_owned(),
                change["added"]["pairs"].as_u64().unwrap_or(0),
                change["removed"]["pairs"].as_u64().unwrap_or(0),
            )
        };
        // What is not committed is measured from the last commit, whose
        // graph is built for it: the first answer says that it is being
        // prepared.
        let first = fixture
            .graphs
            .map(&fixture.project, Some(&Target::Uncommitted), &layers())
            .unwrap();
        assert_eq!(first["mode"], "pattern");
        assert_eq!(
            first["pattern"]["change"],
            json!({
                "state": "building",
                "base": {"name": "the last commit", "commit": &head[..12]},
            })
        );
        assert_eq!(
            did(&told(&Target::Uncommitted)),
            ("ready".to_owned(), head[..12].to_owned(), 1, 0)
        );
        // The checkout's own graph is read with the files of the
        // checkout, so without its tests it needs no spans beside it.
        let plain = asked(
            &Target::Uncommitted,
            Shown {
                notests: true,
                ..layers()
            },
        );
        assert_eq!(
            did(&plain),
            ("ready".to_owned(), head[..12].to_owned(), 1, 0)
        );
        // The whole of the work is measured from the branch it left,
        // where there was no shop yet.
        assert_eq!(
            did(&told(&Target::Work)),
            ("ready".to_owned(), main[..12].to_owned(), 1, 0)
        );
        // A graph older than the files does not hold what they do to
        // the order: until it is rebuilt nothing is told of the change.
        // The rebuild waits for the build of a commit to end.
        rested(&fixture);
        let worse = [&back[..], &[("line", "mailer")]].concat();
        fixture.write("graph.fixture", &sketch(&files, &worse));
        lock(&fixture.graphs.prints).unwrap().clear();
        let stale = fixture
            .graphs
            .map(&fixture.project, Some(&Target::Uncommitted), &layers())
            .unwrap();
        assert_eq!(stale["stale"], true);
        assert_eq!(
            stale["pattern"]["change"],
            json!({
                "state": "unavailable", "reason": "stale",
                "base": {"name": "the last commit", "commit": &head[..12]},
            })
        );
        assert_eq!(fixture.settled()["state"], "ready");
        let rebuilt = told(&Target::Uncommitted);
        assert_eq!(rebuilt["stale"], false);
        assert_eq!(
            did(&rebuilt),
            ("ready".to_owned(), head[..12].to_owned(), 2, 0)
        );
    }
    #[test]
    fn map_git_history_finds_indirect_peers_and_ranks_hotspots() {
        let fixture = Fixture::new();
        fixture.git(&["init", "-q", "--initial-branch=main"]);
        let files = [
            ("a", "A.php"),
            ("b", "B.php"),
            ("c", "C.php"),
            ("d", "D.php"),
        ];
        let links = [("a", "b"), ("b", "a"), ("c", "a"), ("b", "d")];
        fixture.write("graph.fixture", &sketch(&files, &links));
        for (_, path) in files {
            fixture.write(path, "first");
        }
        fixture.commit("first");
        for (_, path) in files {
            fixture.write(path, "second");
        }
        fixture.commit("second");
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        let map = |shown| fixture.graphs.map(&fixture.project, None, &shown).unwrap();
        assert!(map(Shown::default())["git_history"].is_null());
        let shown = Shown {
            focus: Some("A.php"),
            history: true,
            ..Shown::default()
        };
        let view = map(shown);
        let history = &view["git_history"];
        assert_eq!(history["commits"], 2, "{view}");
        assert_eq!(
            history["cochanges"]["rows"],
            json!([{"path":"D.php","together":2,"commits":2}])
        );
        assert_eq!(
            history["hot"],
            json!([
                {"path":"A.php","commits":2,"neighbors":2},
                {"path":"B.php","commits":2,"neighbors":2}
            ])
        );
        fixture.write("A.php", "third");
        fixture.commit("third");
        assert_eq!(map(shown)["git_history"]["cochanges"]["commits"], 3);
    }

    #[test]
    fn layers_use_the_project_rules_and_report_invalid_updates() {
        let fixture = Fixture::new();
        fixture.git(&["init", "-q", "--initial-branch=main"]);
        let files = [("a", "X.php"), ("b", "Y.php"), ("c", "Z.php")];
        fixture.write(
            "graph.fixture",
            &sketch(&files, &[("a", "b"), ("b", "a"), ("b", "c")]),
        );
        for (_, file) in files {
            fixture.write(file, "<?php");
        }
        let rules = json!({"version":1,"layers":[
            {"id":"first","name":"First","paths":["X.php"],"allows":[]},
            {"id":"second","name":"Second","paths":["Y.php"],"allows":["first"]}
        ]});
        let file = crate::graph_map::layer_rules::FILE;
        fixture.write(file, &rules.to_string());
        let revision = fixture.commit("policy");
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        let map = |shown| fixture.graphs.map(&fixture.project, None, &shown).unwrap();
        let ordinary = map(Shown {
            patterns: true,
            ..Shown::default()
        });
        assert_eq!(
            ordinary["patterns"]["kinds"][0]["instances"][0]["id"],
            "layers@"
        );
        let nodes = ordinary["nodes"].as_array().unwrap();
        let first = nodes.iter().find(|node| node["path"] == "X.php").unwrap();
        assert_eq!(first["architecture_layer"]["name"], "First");
        assert_eq!(ordinary["architecture"]["reciprocal_total"], 1);
        assert_eq!(ordinary["architecture"]["violations"]["pairs"], 1);
        let edges = ordinary["edges"].as_array().unwrap();
        assert_eq!(
            edges.iter().find(|edge| edge["from"] == "X.php").unwrap()["architecture_violation"],
            1
        );
        assert!(
            edges.iter().find(|edge| edge["to"] == "X.php").unwrap()["architecture_violation"]
                .is_null()
        );
        let file_view = map(Shown {
            scope: "X.php",
            patterns: true,
            ..Shown::default()
        });
        assert!(
            file_view["nodes"].as_array().unwrap().iter().any(|node| {
                node["symbol"] == true && node["architecture_layer"]["name"] == "First"
            }),
            "{file_view}"
        );
        let layers = |focus| Shown {
            pattern: Some("layers@"),
            focus,
            ..Shown::default()
        };
        let view = map(layers(None));
        assert_eq!(view["pattern"]["declared"], true);
        assert_eq!(view["pattern"]["apart"], 1);
        assert_eq!(view["pattern"]["against"]["pairs"], 1);
        assert_eq!(
            view["pattern"]["against"]["rows"][0]["from"]["path"],
            "X.php"
        );
        assert_eq!(
            map(layers(Some("layer:second>layer:first")))["focus"]["against"],
            false
        );
        fixture.write(file, "{invalid");
        let invalid = map(layers(None));
        assert!(invalid["architecture_error"].is_string());
        assert_ne!(invalid["mode"], "pattern");
        assert!(
            graph_changes::architecture_at(&fixture.folder, &revision, "")
                .unwrap()
                .is_some()
        );
    }

    /// A session of the project with the steps a run recorded, as the
    /// store keeps them: each step started and completed under one call.
    fn recorded(fixture: &Fixture, folder: &str, steps: &[Value]) -> String {
        let session = store::id();
        fixture
                .store
                .connection()
                .unwrap()
                .execute(
                    "INSERT INTO sessions(id,title,project_id,project_path,provider,mode,workflow,project_context,status,created_at,updated_at)VALUES(?1,'Round the totals once',?2,?3,'claude','edit','native',0,'completed','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')",
                    rusqlite::params![session, fixture.project, folder],
                )
                .unwrap();
        for (call, step) in steps.iter().enumerate() {
            if step["kind"] != "tool" {
                fixture.store.event(&session, step).unwrap();
                continue;
            }
            let mut started = step.clone();
            started["state"] = json!("started");
            started["call"] = json!(format!("call-{call}"));
            let failed = started.as_object_mut().unwrap().remove("failed").is_some();
            // What a step changed is known when it ends.
            let changes = started.as_object_mut().unwrap().remove("changes");
            fixture.store.event(&session, &started).unwrap();
            let mut ended = json!({
                "kind": "tool", "tool": step["tool"], "state": "completed",
                "call": started["call"], "ok": !failed,
            });
            if let Some(changes) = changes {
                ended["changes"] = changes;
            }
            fixture.store.event(&session, &ended).unwrap();
        }
        session
    }
    fn step(tool: &str, path: &str) -> Value {
        json!({"kind": "tool", "tool": tool, "path": path, "ok": true})
    }
    #[test]
    fn a_run_is_mapped_as_what_it_changed_with_what_it_opened() {
        let fixture = Fixture::new();
        fixture.branched();
        // A shop in which the service, the mailer and a line use the
        // order.
        let uses = [
            &NEEDS[..],
            &[
                ("mailer", "order"),
                ("line", "order"),
                ("order", "line"),
                ("test", "controller"),
            ],
        ]
        .concat();
        let mut files = SHOP.to_vec();
        files.extend([
            ("coupon", "src/Entity/Coupon.php"),
            ("test", "tests/OrderTest.php"),
        ]);
        fixture.write("graph.fixture", &sketch(&files, &uses));
        fixture.commit("a shop");
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        let folder = fixture.folder.to_string_lossy().into_owned();
        // The run read the order and the service, and a file outside the
        // project; it edited the order, made a coupon, deleted a file,
        // and failed to edit the mailer. It searched once and ran one
        // command.
        let mut failed = step("Edit", "src/Service/MailService.php");
        failed["failed"] = json!(true);
        let mut outside = step("Read", "/etc/hosts");
        outside["outside"] = json!(true);
        let mut patched = json!({"kind": "tool", "tool": "apply_patch", "ok": true});
        patched["changes"] = json!([{"path": "src/Old.php", "kind": "delete"}]);
        let mut searched = step("Grep", "src");
        searched["detail"] = json!("Order");
        let command = json!({"kind": "tool", "tool": "Bash", "detail": "php -l", "ok": true});
        let session = recorded(
            &fixture,
            &folder,
            &[
                json!({"kind": "text", "text": "Reading the order."}),
                step("Read", "src/Entity/Order.php"),
                step("Read", "src\\Service\\OrderService.php"),
                outside,
                step("Edit", "./src/Entity/Order.php"),
                step("Write", "src/Entity/Coupon.php"),
                patched,
                failed,
                searched,
                command,
            ],
        );
        let target = Target::Run(session.clone());
        let map = |shown| fixture.graphs.map(&fixture.project, Some(&target), &shown);
        let view = map(Shown::default()).unwrap();
        // The change is what the run changed, each file with what was
        // done to it; the mailer it failed to edit is none of it.
        assert_eq!(view["mode"], "changes");
        assert_eq!(view["target"], format!("run:{session}"));
        assert_eq!(
            (
                &view["chosen"]["kind"],
                &view["chosen"]["name"],
                &view["chosen"]["files"]
            ),
            (&json!("run"), &json!("Round the totals once"), &json!(3))
        );
        assert_eq!(
            (
                &view["change"]["files"],
                &view["change"]["removed"],
                &view["change"]["unknown"]
            ),
            (&json!(3), &json!(1), &json!(0))
        );
        let test_reach = view["test_reach"].as_array().unwrap();
        let order_tests = test_reach
            .iter()
            .find(|row| row["path"] == "src/Entity/Order.php")
            .unwrap();
        assert_eq!(order_tests["tests"][0]["path"], "tests/OrderTest.php");
        assert_eq!(order_tests["tests"][0]["steps"], 3);
        assert!(
            map(Shown {
                notests: true,
                ..Shown::default()
            })
            .unwrap()["test_reach"]
                .is_null()
        );
        let status = |path: &str| {
            view["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|node| node["path"] == path)
                .map(|node| (node["status"].clone(), node["opened"].clone()))
        };
        assert_eq!(
            status("src/Entity/Order.php"),
            Some((json!("M"), json!(true)))
        );
        // A file the run wrote without having opened it is one it made.
        assert_eq!(
            status("src/Entity/Coupon.php"),
            Some((json!("A"), Value::Null))
        );
        // What the run opened is laid over the picture: the service it
        // read, and not the mailer or the line.
        assert_eq!(
            status("src/Service/OrderService.php"),
            Some((Value::Null, json!(true)))
        );
        assert_eq!(
            status("src/Service/MailService.php"),
            Some((Value::Null, Value::Null))
        );
        // Beside the picture: what the run did, and of the three files
        // that use the order the two it never opened.
        let run = &view["run"];
        assert_eq!(
            json!({
                "session": run["session"], "title": run["title"], "provider": run["provider"],
                "status": run["status"], "opened": run["opened"], "edited": run["edited"],
                "outside": run["outside"], "searched": run["searched"],
                "commands": run["commands"], "helpers": run["helpers"],
                "limited": run["limited"], "elsewhere": run["elsewhere"],
            }),
            json!({
                "session": session, "title": "Round the totals once", "provider": "claude",
                "status": "completed", "opened": 2, "edited": 3, "outside": 1, "searched": 1,
                "commands": 1, "helpers": 0, "limited": false, "elsewhere": false,
            })
        );
        let reach = &run["reach"];
        assert_eq!(
            (&reach["users"], &reach["opened"], &reach["unopened"]),
            (&json!(3), &json!(1), &json!(2))
        );
        assert_eq!(
            reach["rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["path"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["src/Entity/Line.php", "src/Service/MailService.php"]
        );
        // The graph is the project's own, as for the checkout's work.
        assert_eq!(
            (&view["state"], &view["stale"], &view["rebuilding"]),
            (&json!("ready"), &json!(false), &json!(false))
        );
        // A folder of the change and a selected file are overlaid alike.
        let order = map(Shown {
            focus: Some("src/Entity/Order.php"),
            ..Shown::default()
        })
        .unwrap();
        let used_by = order["focus"]["used_by"].as_array().unwrap();
        assert_eq!(used_by.len(), 3);
        assert_eq!(
            used_by
                .iter()
                .filter(|row| row["opened"] == true)
                .map(|row| row["path"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["src/Service/OrderService.php"]
        );
        // The run is searched in the project's graph, chosen or not,
        // and is chosen as it is mapped.
        let found = fixture
            .graphs
            .find(&fixture.project, Some(&target), "Order", false)
            .unwrap();
        assert!(found["found"].as_u64().unwrap() > 0, "{found}");
        let chosen = fixture.graphs.choose(&fixture.project, &target).unwrap();
        assert_eq!(chosen["chosen"]["kind"], "run");
        // A session that is not there, and one of another project, are
        // no runs of this project.
        let nowhere = Target::Run(store::id());
        assert!(
            fixture
                .graphs
                .map(&fixture.project, Some(&nowhere), &Shown::default())
                .is_err()
        );
        let other = tempfile::tempdir().unwrap();
        let elsewhere = fixture.store.register(other.path()).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            fixture
                .graphs
                .map(&elsewhere, Some(&target), &Shown::default())
                .unwrap_err()
                .message,
            "This project has no such run."
        );
    }
    #[test]
    fn comparing_launches_keeps_the_original_changes_and_attributes_followup_actions() {
        let fixture = Fixture::new();
        fixture.branched();
        let links = [
            ("service", "order"),
            ("controller", "order"),
            ("mailer", "order"),
            ("line", "order"),
        ];
        fixture.write("graph.fixture", &sketch(&SHOP, &links));
        fixture.commit("a shop");
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        let order = "src/Entity/Order.php";
        let service = "src/Service/OrderService.php";
        let controller = SHOP.iter().find(|(id, _)| *id == "controller").unwrap().1;
        let mailer = "src/Service/MailService.php";
        let line = SHOP.iter().find(|(id, _)| *id == "line").unwrap().1;
        let session = recorded(
            &fixture,
            fixture.folder.to_str().unwrap(),
            &[step("Read", line)],
        );
        let first = store::hexid();
        let second = store::hexid();
        for (id, tool, path, call) in [
            (&first, "Edit", order, "1"),
            (&first, "Read", controller, "2"),
            (&second, "Edit", service, "1"),
            (&second, "Read", mailer, "2"),
        ] {
            for (state, fields) in [
                ("started", json!({"path": path})),
                ("completed", json!({"ok": true})),
            ] {
                let mut event = fields;
                event["kind"] = json!("tool");
                event["tool"] = json!(tool);
                event["state"] = json!(state);
                event["call"] = json!(call);
                event["launch_id"] = json!(id);
                fixture.store.event(&session, &event).unwrap();
            }
        }
        std::fs::create_dir_all(fixture.folder.join(line).parent().unwrap()).unwrap();
        fixture.write(line, "<?php");
        let shell = fixture
            .store
            .event(
                &session,
                &json!({
                    "kind":"tool", "tool":"Shell", "state":"completed", "ok":true, "exit_code":0,
                    "launch_id":second, "detail":format!("sed -n '1p' {line}"),
                }),
            )
            .unwrap();
        let target = Target::Run(session);
        let map = |launch, compare_launch| {
            fixture.graphs.map(
                &fixture.project,
                Some(&target),
                &Shown {
                    launch,
                    compare_launch,
                    ..Shown::default()
                },
            )
        };
        let view = map(Some(&first), Some(&second)).unwrap();
        assert_eq!(view["chosen"]["files"], 1);
        assert_eq!(view["run"]["opened"], 1);
        assert_eq!(view["run"]["launches"][0]["id"], first);
        assert_eq!(view["run"]["launches"][1]["id"], second);
        let comparison = &view["run"]["comparison"];
        assert_eq!(comparison["newly_touched"], 3);
        assert_eq!(comparison["gaps"], 0);
        let row = |path| {
            comparison["rows"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["path"] == path)
                .unwrap()
        };
        assert_eq!(row(service)["followup_edited"], true);
        assert_eq!(row(service)["opened"], false);
        assert_eq!(row(mailer)["followup_opened"], true);
        assert_eq!(row(controller)["opened"], true);
        assert_eq!(row(line)["opened"], false);
        assert_eq!(row(line)["followup_opened"], true);
        assert_eq!(row(line)["followup_shell_read"], shell);
        assert!(row(line)["shell_read"].is_null());
        assert!(map(None, None).unwrap()["run"]["comparison"].is_null());
        assert!(map(Some(&first), Some(&first)).is_err());
        assert!(map(Some(&first), Some("foreign")).is_err());
    }

    #[test]
    fn a_long_run_and_one_in_a_folder_of_its_own_are_read_as_far_as_they_were_recorded() {
        let fixture = Fixture::new();
        fixture.branched();
        fixture.write("graph.fixture", &sketch(&SHOP, &NEEDS));
        fixture.commit("a shop");
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        // More steps than the store hands out at once, and then the
        // notice that later steps are no longer named: a step after it
        // carries no file.
        let mut steps = (0..300)
            .map(|at| step("Read", &format!("docs/page{at:03}.md")))
            .collect::<Vec<_>>();
        steps.push(step("Edit", "src/Service/OrderService.php"));
        steps.push(json!({
            "kind": "status", "targets": "limited",
            "text": "Step details are not recorded after this point; steps are still counted.",
        }));
        steps.push(json!({"kind": "tool", "text": "Edit: started"}));
        // The run worked in a worktree: its files are named as the
        // project's are.
        let worktree = tempfile::tempdir().unwrap();
        let session = recorded(&fixture, &worktree.path().to_string_lossy(), &steps);
        let view = fixture
            .graphs
            .map(
                &fixture.project,
                Some(&Target::Run(session)),
                &Shown::default(),
            )
            .unwrap();
        let run = &view["run"];
        assert_eq!(
            (
                &run["opened"],
                &run["edited"],
                &run["limited"],
                &run["elsewhere"]
            ),
            (&json!(300), &json!(1), &json!(true), &json!(true))
        );
        assert_eq!(view["chosen"]["files"], 1);
        // The controller uses the service the run edited, and the run
        // never opened it.
        assert_eq!(
            (&run["reach"]["users"], &run["reach"]["unopened"]),
            (&json!(1), &json!(1))
        );
        assert_eq!(
            run["reach"]["rows"][0]["path"],
            "src/Controller/OrderController.php"
        );
    }
    #[test]
    fn switching_the_graph_on_builds_and_publishes_it() {
        let fixture = Fixture::new();
        let off = fixture.graphs.status(&fixture.project, true).unwrap();
        assert_eq!(off["state"], "off");
        assert_eq!(off["tool"]["version"], "9.9.9");
        assert_eq!(fixture.runs(), 0);
        let started = fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(started["state"], "building");
        let ready = fixture.settled();
        assert_eq!(ready["state"], "ready", "{ready}");
        assert_eq!(ready["graph"]["nodes"], 7);
        assert_eq!(ready["graph"]["edges"], 9);
        assert_eq!(ready["build"], Value::Null);
        let graph = live(&fixture.store.state, &fixture.project);
        assert!(graph.is_file());
        // The working folder keeps the cache and gives up its graph.
        let work = graph.parent().unwrap().with_file_name("work");
        assert!(work.join("cache").is_dir());
        assert!(!work.join("graph.json").exists());
        // Outside Git only a request rebuilds; the build starts clean.
        fixture
            .graphs
            .refresh(&fixture.project, Trigger::Visit)
            .unwrap();
        fixture
            .graphs
            .refresh(&fixture.project, Trigger::Finish)
            .unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        assert_eq!(fixture.runs(), 1);
        assert_eq!(
            fixture.graphs.act(&fixture.project, "rebuild").unwrap()["state"],
            "building"
        );
        assert_eq!(fixture.settled()["state"], "ready");
        assert_eq!(fixture.runs(), 2);
    }
    #[test]
    fn a_failed_build_keeps_the_published_graph_and_its_reason() {
        let fixture = Fixture::new();
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        std::fs::write(fixture.folder.join("fail"), "").unwrap();
        fixture.graphs.act(&fixture.project, "rebuild").unwrap();
        let failed = fixture.settled();
        assert_eq!(failed["state"], "failed", "{failed}");
        assert_eq!(failed["failure"]["message"], "boom");
        assert_eq!(failed["graph"]["nodes"], 7);
        assert!(live(&fixture.store.state, &fixture.project).is_file());
        // The same files are not tried again on their own.
        fixture
            .graphs
            .refresh(&fixture.project, Trigger::Finish)
            .unwrap();
        assert_eq!(fixture.settled()["state"], "failed");
        assert_eq!(fixture.runs(), 2);
        std::fs::remove_file(fixture.folder.join("fail")).unwrap();
        fixture.graphs.act(&fixture.project, "rebuild").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
    }
    #[test]
    fn deleting_ends_a_running_build_and_removes_the_folder() {
        let fixture = Fixture::new();
        std::fs::write(fixture.folder.join("slow"), "").unwrap();
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        let started = Instant::now();
        while fixture.graphs.status(&fixture.project, false).unwrap()["build"]["phase"] != "read" {
            assert!(started.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(20));
        }
        fixture.graphs.act(&fixture.project, "delete").unwrap();
        let off = fixture.settled();
        assert_eq!(off["state"], "off");
        assert_eq!(off["graph"], Value::Null);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(!folder(&fixture.store.state, &fixture.project).exists());
    }
    #[test]
    fn closing_ends_a_running_build() {
        let fixture = Fixture::new();
        std::fs::write(fixture.folder.join("slow"), "").unwrap();
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        let started = Instant::now();
        fixture.graphs.close();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(!live(&fixture.store.state, &fixture.project).exists());
    }
    #[test]
    fn a_changed_checkout_is_rebuilt_but_not_under_a_run() {
        let fixture = Fixture::new();
        fixture.git(&["init", "-q"]);
        std::fs::write(fixture.folder.join("a.rs"), "fn a() {}\n").unwrap();
        fixture.git(&["add", "a.rs"]);
        fixture.git(&["commit", "-q", "-m", "one"]);
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        let ready = fixture.settled();
        assert_eq!(ready["state"], "ready", "{ready}");
        assert_eq!(ready["graph"]["commit"].as_str().unwrap().len(), 12);
        fixture
            .graphs
            .refresh(&fixture.project, Trigger::Launch)
            .unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        assert_eq!(fixture.runs(), 1);
        std::fs::write(fixture.folder.join("a.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        fixture.session("running");
        fixture
            .graphs
            .refresh(&fixture.project, Trigger::Visit)
            .unwrap();
        fixture
            .graphs
            .refresh(&fixture.project, Trigger::Finish)
            .unwrap();
        assert_eq!(fixture.settled()["state"], "stale");
        assert_eq!(fixture.runs(), 1);
        fixture
            .graphs
            .refresh(&fixture.project, Trigger::Launch)
            .unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        assert_eq!(fixture.runs(), 2);
    }
    fn drawn(view: &Value) -> Vec<String> {
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| {
                format!(
                    "{} {}{}",
                    node["path"].as_str().unwrap(),
                    node["role"].as_str().unwrap(),
                    node["status"]
                        .as_str()
                        .map(|status| format!(" {status}"))
                        .unwrap_or_default()
                )
            })
            .collect()
    }
    #[test]
    fn a_branch_gets_the_graph_of_its_own_commit_and_the_checkout_is_left_alone() {
        let fixture = Fixture::new();
        let (main, feature) = fixture.branched();
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        let target = Target::Named("feature".into());
        // A change is worked out before it is mapped.
        assert!(
            fixture
                .graphs
                .map(&fixture.project, Some(&target), &Shown::default())
                .is_err()
        );
        let before = fixture.said(&["status", "--porcelain=v1"]);
        let chosen = fixture.graphs.choose(&fixture.project, &target).unwrap();
        assert_eq!(chosen["chosen"]["kind"], "branch");
        assert_eq!(chosen["chosen"]["files"], 3);
        assert_eq!(
            chosen["chosen"]["head"],
            json!({"name": "feature", "commit": feature[..12]})
        );
        assert_eq!(
            chosen["chosen"]["base"],
            json!({"name": "origin/main", "commit": main[..12]})
        );
        let view = fixture.mapped(&target);
        assert_eq!(view["state"], "ready", "{view}");
        // The order exists only on the branch: the graph is of its files.
        assert_eq!(
            drawn(&view),
            [
                "cart.php changed M",
                "order.php changed A",
                "price.php needed"
            ]
        );
        assert_eq!(view["change"]["files"], 3);
        assert_eq!(view["change"]["unknown"], 1);
        assert_eq!(fixture.revisions(), std::slice::from_ref(&feature));
        let mut kept = std::fs::read_dir(
            folder(&fixture.store.state, &fixture.project)
                .join("revisions")
                .join(&feature),
        )
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
        kept.sort();
        assert_eq!(kept, ["graph.json", "test-spans.json", "tree"]);
        assert_eq!(
            held(
                &folder(&fixture.store.state, &fixture.project)
                    .join("revisions")
                    .join(&feature)
                    .join("tree")
            ),
            ["cart.php", "order.php", "price.php"]
        );
        assert_eq!(fixture.said(&["status", "--porcelain=v1"]), before);
        assert_eq!(fixture.said(&["rev-parse", "HEAD"]), main);
        assert_eq!(fixture.said(&["symbolic-ref", "--short", "HEAD"]), "main");
        assert_eq!(fixture.runs(), 2);
        // Asked again, the graph that is there is used.
        let again = fixture.graphs.choose(&fixture.project, &target).unwrap();
        assert_eq!(again["state"], "ready");
        // The project's own graph was built from a clean checkout of
        // this commit, so that commit needs no graph of its own.
        let own = fixture
            .graphs
            .choose(&fixture.project, &Target::Named(main))
            .unwrap();
        assert_eq!(own["state"], "ready");
        assert_eq!(fixture.runs(), 2);
        assert_eq!(fixture.revisions(), [feature]);
    }
    /// The files under a folder, by their paths from it.
    fn held(home: &Path) -> Vec<String> {
        let mut found = Vec::new();
        let mut folders = vec![home.to_path_buf()];
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(&folder).unwrap().flatten() {
                if entry.file_type().unwrap().is_dir() {
                    folders.push(entry.path());
                } else {
                    found.push(
                        entry
                            .path()
                            .strip_prefix(home)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                }
            }
        }
        found.sort();
        found
    }
    #[test]
    fn the_graph_of_a_commit_is_read_with_what_is_kept_of_its_files() {
        let fixture = Fixture::new();
        let write = |name: &str, text: &str| {
            let path = fixture.folder.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        fixture.git(&["init", "-q", "--initial-branch=main"]);
        write("Cargo.toml", "[workspace]\nmembers = [\"app\", \"core\"]\n");
        write(
            "app/Cargo.toml",
            "[package]\nname = \"shop-app\"\n[dependencies]\nshop-core = { path = \"../core\" }\n",
        );
        write("core/Cargo.toml", "[package]\nname = \"shop-core\"\n");
        write("app/src/main.rs", "fn main() {}\n");
        write("core/src/lib.rs", "pub fn run() {}\n");
        // The tool has the core use the app, which is built the other
        // way round.
        write(
            "graph.fixture",
            r#"{"nodes":[{"id":"main","source_file":"app/src/main.rs"},{"id":"run","source_file":"core/src/lib.rs"}],"links":[{"source":"main","target":"run","relation":"calls","source_file":"app/src/main.rs"},{"source":"run","target":"main","relation":"imports_from","source_file":"core/src/lib.rs"}]}"#,
        );
        fixture.commit("one");
        fixture.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        fixture.git(&["switch", "-q", "-c", "feature"]);
        // On the branch neither is built with the other, and the core
        // takes one source of the app by its path.
        write("app/Cargo.toml", "[package]\nname = \"shop-app\"\n");
        write("app/src/shared/util.rs", "pub fn helper() {}\n");
        write(
            "core/src/lib.rs",
            "#[path = \"../../app/src/shared/util.rs\"]\nmod util;\npub fn run() {}\n\
                 #[test]\nfn runs_with_fixture() {}\n",
        );
        write(
            "graph.fixture",
            r#"{"nodes":[{"id":"main","source_file":"app/src/main.rs"},{"id":"run","source_file":"core/src/lib.rs"},{"id":"helper","source_file":"app/src/shared/util.rs"},{"id":"runs_with_fixture","source_file":"core/src/lib.rs","source_location":"L5"}],"links":[{"source":"main","target":"run","relation":"calls","source_file":"app/src/main.rs"},{"source":"run","target":"main","relation":"imports_from","source_file":"core/src/lib.rs"},{"source":"runs_with_fixture","target":"helper","relation":"calls","source_file":"core/src/lib.rs"}]}"#,
        );
        let feature = fixture.commit("two");
        fixture.git(&["switch", "-q", "main"]);
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        let arrows = |view: &Value| {
            view["edges"]
                .as_array()
                .unwrap()
                .iter()
                .map(|edge| {
                    format!(
                        "{} -> {}",
                        edge["from"].as_str().unwrap(),
                        edge["to"].as_str().unwrap()
                    )
                })
                .collect::<Vec<_>>()
        };
        // The project's own graph is read with the files of the
        // checkout: the app is built with the core.
        let own = fixture
            .graphs
            .map(&fixture.project, None, &Shown::default())
            .unwrap();
        assert_eq!(
            (&own["totals"]["links"], &own["totals"]["ruled_out"]),
            (&json!(1), &json!(1))
        );
        assert_eq!(arrows(&own), ["app -> core"]);
        // A folder a request names is spread out in the picture.
        let spread = ["app".to_owned()];
        let opened = fixture
            .graphs
            .map(
                &fixture.project,
                None,
                &Shown {
                    spread: &spread,
                    ..Shown::default()
                },
            )
            .unwrap();
        assert_eq!(arrows(&opened), ["app/src -> core"]);
        assert_eq!(opened["frames"][0]["path"], "app");
        assert_eq!(opened["nodes"][0]["holder"], "app");
        // A link a request names is looked into instead of a picture.
        let looked = fixture
            .graphs
            .map(
                &fixture.project,
                None,
                &Shown {
                    link: Some(("app", "core")),
                    ..Shown::default()
                },
            )
            .unwrap();
        assert_eq!(
            (&looked["mode"], &looked["links"], &looked["project_id"]),
            (&json!("link"), &json!(1), &json!(fixture.project))
        );
        assert_eq!(looked["used"]["groups"][0]["path"], "core/src/lib.rs");
        // The graph of the commit is read with the manifests of the
        // commit, which rule out both links between the two, and with
        // what its sources say: the helper is compiled into the core.
        let target = Target::Named("feature".into());
        fixture.graphs.choose(&fixture.project, &target).unwrap();
        let view = fixture.mapped(&target);
        assert_eq!(view["state"], "ready", "{view}");
        assert_eq!(
            (&view["totals"]["links"], &view["totals"]["ruled_out"]),
            (&json!(1), &json!(2))
        );
        assert_eq!(arrows(&view), ["core/src/lib.rs -> app/src/shared/util.rs"]);
        let plain = fixture
            .graphs
            .map(
                &fixture.project,
                Some(&target),
                &Shown {
                    notests: true,
                    ..Shown::default()
                },
            )
            .unwrap();
        assert_eq!(plain["tests"]["inside"], 1);
        assert!(arrows(&plain).is_empty());
        // Of the files of the commit, Cargo manifests and preserved sources are kept,
        // while non-source fixtures are stripped.
        let home = folder(&fixture.store.state, &fixture.project)
            .join("revisions")
            .join(&feature);
        assert_eq!(
            held(&home),
            [
                "graph.json",
                "test-spans.json",
                "tree/Cargo.toml",
                "tree/app/Cargo.toml",
                "tree/app/src/main.rs",
                "tree/app/src/shared/util.rs",
                "tree/core/Cargo.toml",
                "tree/core/src/lib.rs",
            ]
        );
    }
    #[test]
    fn the_current_work_is_mapped_on_the_projects_own_graph() {
        let fixture = Fixture::new();
        fixture.branched();
        fixture.git(&["switch", "-q", "feature"]);
        fixture.write("price.php", "<?php // price, edited\n");
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        let map = |target: &Target| {
            fixture
                .graphs
                .map(&fixture.project, Some(target), &Shown::default())
                .unwrap()
        };
        let work = map(&Target::Work);
        assert_eq!(work["state"], "ready");
        assert_eq!(
            (&work["stale"], &work["rebuilding"]),
            (&json!(false), &json!(false))
        );
        assert_eq!(work["chosen"]["kind"], "work");
        assert_eq!(work["chosen"]["name"], "feature");
        assert_eq!(work["chosen"]["base"]["name"], "origin/main");
        assert_eq!(work["chosen"]["files"], 4);
        assert_eq!(
            drawn(&work),
            [
                "cart.php changed M",
                "order.php changed A",
                "price.php changed M"
            ]
        );
        // What is not committed: the price, and the cart that uses it.
        let uncommitted = map(&Target::Uncommitted);
        assert_eq!(uncommitted["chosen"]["files"], 1);
        assert_eq!(
            drawn(&uncommitted),
            ["price.php changed M", "cart.php affected"]
        );
        // A file of the change opens to what it holds and stays in the
        // change: the files around it carry what happened to them.
        let opened = fixture
            .graphs
            .map(
                &fixture.project,
                Some(&Target::Work),
                &Shown {
                    scope: "cart.php",
                    ..Shown::default()
                },
            )
            .unwrap();
        assert_eq!(
            (
                opened["mode"].as_str(),
                opened["file"]["status"].as_str(),
                opened["chosen"]["kind"].as_str(),
                opened["state"].as_str()
            ),
            (Some("file"), Some("M"), Some("work"), Some("ready"))
        );
        let around = opened["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node["symbol"] == false)
            .map(|node| {
                format!(
                    "{} {} {}",
                    node["path"].as_str().unwrap(),
                    node["role"].as_str().unwrap(),
                    node["status"].as_str().unwrap_or("-")
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(around, ["order.php uses A", "price.php used M"]);
        // The links of a file are followed on the same graph with the
        // change still marked. A place the graph does not know falls
        // back to the change.
        let follow = |root: &str| {
            let followed = Followed {
                root: root.to_owned(),
                out: vec![root.to_owned()],
                into: vec![root.to_owned()],
            };
            fixture
                .graphs
                .map(
                    &fixture.project,
                    Some(&Target::Work),
                    &Shown {
                        followed: Some(&followed),
                        ..Shown::default()
                    },
                )
                .unwrap()
        };
        let followed = follow("cart.php");
        assert_eq!(
            (
                followed["mode"].as_str(),
                followed["chosen"]["kind"].as_str(),
                followed["state"].as_str()
            ),
            (Some("follow"), Some("work"), Some("ready"))
        );
        let places = followed["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| {
                format!(
                    "{} {}",
                    node["path"].as_str().unwrap(),
                    node["status"].as_str().unwrap_or("-")
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(places, ["cart.php M", "price.php M", "order.php A"]);
        assert_eq!(follow("nowhere.php")["mode"], "changes");
        // The links of a symbol are followed among symbols, and what
        // the change did to each is read with them. This fixture gives
        // its symbols neither a name nor a line: one is told by its
        // file alone, that of the new file is new, and a changed line
        // marks none.
        let traced = follow("order.php#");
        assert_eq!(
            (
                traced["mode"].as_str(),
                traced["followed"]["symbol"].as_bool(),
                traced["scope"].as_str(),
                traced["chosen"]["kind"].as_str()
            ),
            (Some("follow"), Some(true), Some("order.php"), Some("work"))
        );
        let symbols = traced["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| {
                format!(
                    "{} {} {}",
                    node["path"].as_str().unwrap(),
                    node["changed"],
                    node["status"].as_str().unwrap_or("-")
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(symbols, ["order.php# 1 A", "cart.php# 0 -"]);
        // The shortest ways between two places are found on the same
        // graph, with the change marked: the order reaches the price
        // through the cart. An end the graph does not know leaves the
        // change as it is drawn.
        let ways = |from: &str, to: &str| {
            fixture
                .graphs
                .map(
                    &fixture.project,
                    Some(&Target::Work),
                    &Shown {
                        path: Some((from, to)),
                        ..Shown::default()
                    },
                )
                .unwrap()
        };
        let way = ways("order.php", "price.php");
        assert_eq!(
            (
                way["mode"].as_str(),
                way["path"]["steps"].as_u64(),
                way["path"]["reversed"].as_bool(),
                way["chosen"]["kind"].as_str()
            ),
            (Some("path"), Some(2), Some(false), Some("work"))
        );
        let places = way["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| {
                format!(
                    "{} {}",
                    node["path"].as_str().unwrap(),
                    node["status"].as_str().unwrap_or("-")
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(places, ["order.php A", "cart.php M", "price.php M"]);
        assert_eq!(ways("order.php", "nowhere.php")["mode"], "changes");
        // What two places share is asked of the same graph. The order
        // reaches the price through the cart alone, so the order and
        // the cart share nothing, and the answer says so; a place the
        // graph does not know leaves the change as it is drawn.
        let sharing = |from: &str, to: &str, users: Option<bool>| {
            fixture
                .graphs
                .map(
                    &fixture.project,
                    Some(&Target::Work),
                    &Shown {
                        shared: Some(Sharing {
                            from,
                            to,
                            users,
                            skip: 0,
                        }),
                        ..Shown::default()
                    },
                )
                .unwrap()
        };
        let both = sharing("order.php", "cart.php", None);
        assert_eq!(
            (
                both["mode"].as_str(),
                both["shared"]["from"]["name"].as_str(),
                both["shared"]["side"].as_str(),
                both["shared"]["both_use"].as_u64(),
                both["shared"]["uses_both"].as_u64(),
                both["chosen"]["kind"].as_str()
            ),
            (
                Some("shared"),
                Some("order.php"),
                Some("both_use"),
                Some(0),
                Some(0),
                Some("work")
            )
        );
        // The kind that is asked for is the one answered.
        assert_eq!(
            sharing("order.php", "cart.php", Some(true))["shared"]["side"],
            "uses_both"
        );
        assert_eq!(sharing("order.php", "nowhere.php", None)["mode"], "changes");
        // On the whole project the same links carry no mark.
        let whole = fixture
            .graphs
            .map(
                &fixture.project,
                None,
                &Shown {
                    followed: Some(&Followed {
                        root: "order.php#".to_owned(),
                        out: vec!["order.php#".to_owned()],
                        into: vec![],
                    }),
                    ..Shown::default()
                },
            )
            .unwrap();
        assert_eq!(
            (&whole["mode"], &whole["nodes"][0]["changed"]),
            (&json!("follow"), &json!(0))
        );
        assert_eq!(whole["nodes"].as_array().unwrap().len(), 2);
        // Every answer says whether the tests are left out, and where
        // they are, how many that is. This project has none.
        assert_eq!(
            (&work["notests"], &work["tests"]),
            (&json!(false), &Value::Null)
        );
        let plain = fixture
            .graphs
            .map(
                &fixture.project,
                Some(&Target::Work),
                &Shown {
                    notests: true,
                    ..Shown::default()
                },
            )
            .unwrap();
        assert_eq!(
            (&plain["notests"], &plain["tests"]),
            (&json!(true), &json!({"files": 0, "inside": 0}))
        );
        assert_eq!(drawn(&plain), drawn(&work));
        // What the change did to the file is read with it. The graph of
        // this fixture gives its symbols no line, so a changed line
        // marks none of them, and a new file is new throughout.
        assert_eq!(
            opened["file"]["touched"],
            json!({"symbols": 0, "added": 0, "modified": 0, "between": 0, "top": false})
        );
        let added = fixture
            .graphs
            .map(
                &fixture.project,
                Some(&Target::Work),
                &Shown {
                    scope: "order.php",
                    ..Shown::default()
                },
            )
            .unwrap();
        assert_eq!(
            (&added["file"]["status"], &added["file"]["touched"]["added"]),
            (&json!("A"), &json!(1))
        );
        // A file the change left alone has nothing to mark.
        let beside = fixture
            .graphs
            .map(
                &fixture.project,
                Some(&Target::Uncommitted),
                &Shown {
                    scope: "cart.php",
                    ..Shown::default()
                },
            )
            .unwrap();
        assert_eq!(
            (&beside["mode"], &beside["file"]["touched"]),
            (&json!("file"), &Value::Null)
        );
        assert_eq!(fixture.runs(), 1);
        assert!(fixture.revisions().is_empty());
    }
    #[test]
    fn a_commit_whose_graph_fails_says_why_and_keeps_nothing() {
        let fixture = Fixture::new();
        fixture.branched();
        fixture.git(&["switch", "-q", "-c", "broken"]);
        fixture.write("fail", "");
        fixture.commit("broken");
        fixture.git(&["switch", "-q", "main"]);
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        let target = Target::Named("broken".into());
        fixture.graphs.choose(&fixture.project, &target).unwrap();
        let view = fixture.mapped(&target);
        assert_eq!(view["state"], "failed", "{view}");
        assert_eq!(view["failure"], "boom");
        // What is drawn is the first picture, on the project's graph.
        assert_eq!(view["preliminary"], true);
        assert!(view["nodes"].is_array());
        assert!(fixture.revisions().is_empty());
        assert_eq!(fixture.settled()["state"], "ready");
        // Choosing it again tries again.
        fixture.graphs.choose(&fixture.project, &target).unwrap();
        assert_eq!(fixture.mapped(&target)["state"], "failed");
        assert_eq!(fixture.runs(), 3);
    }
    #[test]
    fn only_the_newest_graphs_of_other_commits_are_kept() {
        let fixture = Fixture::new();
        fixture.branched();
        fixture.git(&["switch", "-q", "-c", "stack"]);
        let mut commits = Vec::new();
        for number in 0..REVISIONS + 1 {
            fixture.write(&format!("step{number}.php"), "<?php\n");
            commits.push(fixture.commit("step"));
        }
        fixture.git(&["switch", "-q", "main"]);
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        for commit in &commits {
            let target = Target::Named(commit.clone());
            fixture.graphs.choose(&fixture.project, &target).unwrap();
            assert_eq!(fixture.mapped(&target)["state"], "ready");
            // File times on some systems move in coarse steps.
            std::thread::sleep(Duration::from_millis(20));
        }
        let mut newest = commits[1..].to_vec();
        newest.sort();
        assert_eq!(fixture.revisions(), newest);
    }
    #[test]
    fn a_search_reads_the_graph_the_map_is_drawn_from() {
        let fixture = Fixture::new();
        fixture.branched();
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        let paths = |found: &Value| {
            found["rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["path"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        let find = |target: Option<&Target>, query: &str| {
            fixture.graphs.find(&fixture.project, target, query, false)
        };
        // The whole project and the checkout's own work are searched in
        // the project's graph, where there is a cart and no order.
        let own = find(None, "CART").unwrap();
        assert_eq!(paths(&own), ["cart.php"]);
        assert_eq!(
            (&own["mode"], &own["project_id"], &own["notests"]),
            (&json!("find"), &json!(fixture.project), &json!(false))
        );
        for target in [None, Some(Target::Work), Some(Target::Uncommitted)] {
            let found = find(target.as_ref(), "order").unwrap();
            assert_eq!(found["found"], 0, "{target:?}");
        }
        // A change that was not chosen has no map, and nothing to search.
        let target = Target::Named("feature".into());
        assert!(find(Some(&target), "order").is_err());
        // Chosen, it is searched in the graph of its own commit.
        fixture.graphs.choose(&fixture.project, &target).unwrap();
        assert_eq!(fixture.mapped(&target)["state"], "ready");
        assert_eq!(paths(&find(Some(&target), "order").unwrap()), ["order.php"]);
        // Whether tests were left out is told back with what was found.
        let plain = fixture
            .graphs
            .find(&fixture.project, None, "cart", true)
            .unwrap();
        assert_eq!(
            (&plain["notests"], &plain["found"]),
            (&json!(true), &json!(1))
        );
        // Where a commit has no graph and the project none either, there
        // is nothing to search, and that is said.
        let bare = Fixture::new();
        bare.branched();
        bare.git(&["switch", "-q", "-c", "broken"]);
        bare.write("fail", "");
        bare.commit("broken");
        bare.git(&["switch", "-q", "main"]);
        let target = Target::Named("broken".into());
        bare.graphs.choose(&bare.project, &target).unwrap();
        assert_eq!(bare.mapped(&target)["state"], "failed");
        let none = bare
            .graphs
            .find(&bare.project, Some(&target), " cart ", false)
            .unwrap();
        assert_eq!(
            (&none["missing"], &none["found"], &none["query"]),
            (&json!(true), &json!(0), &json!("cart"))
        );
        assert!(
            bare.graphs
                .find(&bare.project, None, "cart", false)
                .is_err()
        );
    }
    #[test]
    fn a_picked_commit_is_first_drawn_on_the_projects_graph_while_its_own_is_built() {
        let fixture = Fixture::new();
        fixture.branched();
        fixture.git(&["switch", "-q", "-c", "long"]);
        fixture.write("slow", "");
        fixture.write("price.php", "<?php // price, changed\n");
        let long = fixture.commit("Change the price");
        fixture.git(&["switch", "-q", "main"]);
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        let target = Target::Commit(long.clone());
        let chosen = fixture.graphs.choose(&fixture.project, &target).unwrap();
        assert_eq!(chosen["state"], "building");
        assert_eq!(chosen["target"], format!("commit:{long}"));
        assert_eq!(chosen["chosen"]["kind"], "commit");
        assert_eq!(chosen["chosen"]["title"], "Change the price");
        assert_eq!(chosen["chosen"]["name"], long[..12]);
        let view = fixture
            .graphs
            .map(&fixture.project, Some(&target), &Shown::default())
            .unwrap();
        // The files are the commit's, the links the checkout's: the
        // cart uses the price in the project's own graph.
        assert_eq!(view["state"], "building");
        assert_eq!(view["preliminary"], true);
        assert_eq!(view["commit_info"]["commit"], long);
        assert_eq!(view["commit_info"]["subject"], "Change the price");
        assert!(
            view["commit_info"]["parents"]
                .as_array()
                .is_some_and(|parents| parents.len() == 1)
        );
        assert_eq!(drawn(&view), ["price.php changed M", "cart.php affected"]);
        assert_eq!(view["change"]["files"], 2);
        // A search reads the project's graph until then, as the map does.
        let found = fixture
            .graphs
            .find(&fixture.project, Some(&target), "cart", false)
            .unwrap();
        assert_eq!(found["rows"][0]["path"], "cart.php");
        // The history and the branches are read for the picker.
        let history = fixture
            .graphs
            .commits(&fixture.project, "long", "", 0)
            .unwrap();
        assert_eq!(history["commits"][0]["commit"], long);
        assert_eq!(history["commits"][0]["subject"], "Change the price");
        assert_eq!(history["more"], false);
        let branches = fixture.graphs.branches(&fixture.project).unwrap();
        let names = branches["branches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|branch| branch["name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert!(names.contains(&"long".to_owned()) && names.contains(&"feature".to_owned()));
        fixture.graphs.act(&fixture.project, "delete").unwrap();
        let started = Instant::now();
        while !fixture.graphs.revisions.lock().unwrap().is_empty() {
            assert!(started.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    #[test]
    fn commit_target_auto_resolves_and_enriches_commit_info() {
        let fixture = Fixture::new();
        fixture.branched();
        let head = fixture.said(&["rev-parse", "feature"]);
        let parent = fixture.said(&["rev-parse", "feature^"]);
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");

        // Calling `map` or `find` directly on a `Target::Commit` without a
        // prior `choose()` auto-resolves smoothly and enriches `commit_info`.
        let short_target = Target::Commit(head[..8].to_owned());
        let view = fixture
            .graphs
            .map(&fixture.project, Some(&short_target), &Shown::default())
            .unwrap();
        assert_eq!(view["commit_info"]["commit"], head);
        assert_eq!(view["commit_info"]["subject"], "two");
        assert_eq!(view["commit_info"]["parents"], json!([parent]));
        assert!(view["commit_info"]["short"].is_string());
        assert!(view["commit_info"]["author"].is_string());
        assert!(view["commit_info"]["at"].as_i64().is_some());
        assert!(
            view["commit_info"]["refs"]
                .as_array()
                .is_some_and(|refs| refs.iter().any(|r| r["name"] == "feature"))
        );

        let found = fixture
            .graphs
            .find(&fixture.project, Some(&short_target), "cart", false)
            .unwrap();
        assert_eq!(found["rows"][0]["path"], "cart.php");

        // Non-commit targets that were not chosen still conflict.
        assert!(
            fixture
                .graphs
                .map(
                    &fixture.project,
                    Some(&Target::Named("feature".into())),
                    &Shown::default()
                )
                .is_err()
        );
    }
    #[test]
    fn deleting_the_graph_ends_the_build_of_a_commit() {
        let fixture = Fixture::new();
        fixture.branched();
        fixture.git(&["switch", "-q", "-c", "long"]);
        fixture.write("slow", "");
        fixture.commit("long");
        fixture.git(&["switch", "-q", "main"]);
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        let target = Target::Named("long".into());
        let chosen = fixture.graphs.choose(&fixture.project, &target).unwrap();
        assert_eq!(chosen["state"], "building");
        let off = fixture.graphs.act(&fixture.project, "delete").unwrap();
        assert_eq!(off["state"], "off");
        let started = Instant::now();
        while !fixture.graphs.revisions.lock().unwrap().is_empty() {
            assert!(started.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!folder(&fixture.store.state, &fixture.project).exists());
        // What was chosen went with the graph.
        assert!(
            fixture
                .graphs
                .map(&fixture.project, Some(&target), &Shown::default())
                .is_err()
        );
    }
    #[test]
    fn an_old_release_leaves_the_projects_own_graph_folder_as_it_was() {
        let fixture = Fixture::new();
        std::fs::write(fixture.folder.join("old"), "").unwrap();
        let own = fixture.folder.join("graphify-out");
        // No folder of its own: the one the tool created is removed.
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        assert!(!own.exists());
        // The user's own graph: its manifest keeps its bytes and its time,
        // also when the build is cut short.
        std::fs::create_dir_all(&own).unwrap();
        std::fs::write(own.join("graph.json"), "mine").unwrap();
        std::fs::write(own.join("manifest.json"), "mine").unwrap();
        let before = std::fs::metadata(own.join("manifest.json"))
            .unwrap()
            .modified()
            .unwrap();
        for slow in [false, true] {
            if slow {
                std::fs::write(fixture.folder.join("slow"), "").unwrap();
            }
            fixture.graphs.act(&fixture.project, "rebuild").unwrap();
            if slow {
                let started = Instant::now();
                while std::fs::read_to_string(own.join("manifest.json")).unwrap() == "mine" {
                    assert!(started.elapsed() < Duration::from_secs(10));
                    std::thread::sleep(Duration::from_millis(10));
                }
                fixture.graphs.act(&fixture.project, "disable").unwrap();
            }
            fixture.settled();
            assert_eq!(
                std::fs::read_to_string(own.join("manifest.json")).unwrap(),
                "mine"
            );
            assert_eq!(
                std::fs::metadata(own.join("manifest.json"))
                    .unwrap()
                    .modified()
                    .unwrap(),
                before
            );
            assert_eq!(
                std::fs::read_to_string(own.join("graph.json")).unwrap(),
                "mine"
            );
        }
    }
    #[test]
    fn a_launch_is_told_about_a_published_graph_only_where_that_was_asked_for() {
        let fixture = Fixture::new();
        let session = json!({"project_id": fixture.project, "project_path": fixture.folder.to_string_lossy()});
        let silent = || {
            assert_eq!(prompt_line(&fixture.store, &session).unwrap(), None);
            assert!(grant(&fixture.store, &fixture.project).unwrap().is_none());
        };
        silent();
        // A graph that is off has nothing to tell agents about.
        assert!(fixture.graphs.act(&fixture.project, "offer").is_err());
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        let ready = fixture.settled();
        assert_eq!(
            (&ready["state"], &ready["agents"]),
            (&json!("ready"), &json!(false))
        );
        // The graph is built for the map; agents hear of it only when
        // the person asks for that, and no longer once that is undone.
        silent();
        assert_eq!(
            fixture.graphs.act(&fixture.project, "offer").unwrap()["agents"],
            true
        );
        assert!(prompt_line(&fixture.store, &session).unwrap().is_some());
        assert_eq!(
            fixture.graphs.act(&fixture.project, "withhold").unwrap()["agents"],
            false
        );
        silent();
        fixture.graphs.act(&fixture.project, "offer").unwrap();
        let line = prompt_line(&fixture.store, &session).unwrap().unwrap();
        let graph = live(&fixture.store.state, &fixture.project);
        assert!(
            line.contains(&format!("query \"<question>\" --graph {}", graph.display())),
            "{line}"
        );
        assert!(line.contains("into 7 nodes."), "{line}");
        assert!(line.contains("Keep --graph and its path last"), "{line}");
        assert!(
            line.contains(&fixture.tool.to_string_lossy().into_owned()),
            "{line}"
        );
        let elsewhere = json!({"project_id": fixture.project, "project_path": "/another/checkout"});
        assert!(
            prompt_line(&fixture.store, &elsewhere)
                .unwrap()
                .unwrap()
                .contains("another checkout")
        );
        // Claude Code is permitted what the prompt offers and no more: each
        // rule opens and closes as one of the offered lines does, with the
        // graph as its last word.
        let permitted = grant(&fixture.store, &fixture.project).unwrap().unwrap();
        let tool = fixture.tool.to_string_lossy();
        assert_eq!(
            permitted.rules,
            ["query", "explain", "path"]
                .map(|verb| format!("Bash({tool} {verb} * --graph {})", graph.display()))
        );
        for rule in &permitted.rules {
            let (opening, closing) = rule["Bash(".len()..rule.len() - 1]
                .split_once(" * ")
                .unwrap();
            assert!(
                line.lines()
                    .any(|offered| offered.starts_with(opening) && offered.ends_with(closing)),
                "{rule}\n{line}"
            );
        }
        assert_eq!(permitted.environment, [("GRAPHIFY_NO_AUTO_REFRESH", "1")]);
        // Switching the graph off silences the launch and keeps the
        // choice; deleting the graph forgets it.
        fixture.graphs.act(&fixture.project, "disable").unwrap();
        silent();
        assert!(graph.is_file());
        assert_eq!(
            fixture.graphs.act(&fixture.project, "enable").unwrap()["agents"],
            true
        );
        fixture.settled();
        assert!(prompt_line(&fixture.store, &session).unwrap().is_some());
        assert_eq!(
            fixture.graphs.act(&fixture.project, "delete").unwrap()["agents"],
            false
        );
        silent();
    }
    #[test]
    fn auto_start_enables_unset_project_and_respects_user_disable() {
        let fixture = Fixture::new();
        assert_eq!(
            fixture.graphs.status(&fixture.project, true).unwrap()["state"],
            "off"
        );
        assert_eq!(
            fixture.graphs.status(&fixture.project, false).unwrap()["message"],
            "Code graph is turned off."
        );

        fixture.graphs.auto_start(&fixture.project).unwrap();
        let ready = fixture.settled();
        assert_eq!(ready["state"], "ready", "{ready}");
        assert_eq!(ready["message"], "Code graph & dependency map ready.");
        assert!(live(&fixture.store.state, &fixture.project).is_file());
        assert!(fixture.graphs.live_index(&fixture.project).is_some());
        assert_eq!(fixture.runs(), 1);

        let disabled = fixture.graphs.act(&fixture.project, "disable").unwrap();
        assert_eq!(disabled["state"], "off");
        fixture.graphs.auto_start(&fixture.project).unwrap();
        assert_eq!(
            fixture.graphs.status(&fixture.project, false).unwrap()["state"],
            "off"
        );
        assert_eq!(fixture.runs(), 1);

        fixture.graphs.act(&fixture.project, "enable").unwrap();
        let ready = fixture.settled();
        assert_eq!(ready["state"], "ready");
        assert_eq!(fixture.runs(), 1);

        fixture.graphs.act(&fixture.project, "delete").unwrap();
        let deleted = fixture.settled();
        assert_eq!(deleted["state"], "off");
        fixture.graphs.auto_start(&fixture.project).unwrap();
        assert_eq!(
            fixture.graphs.status(&fixture.project, false).unwrap()["state"],
            "off"
        );
        assert_eq!(fixture.runs(), 1);

        // auto_start_projects asynchronously starts unset projects when enabled.
        let root2 = tempfile::tempdir().unwrap();
        let store2 = SessionStore::open(&root2.path().join("state")).unwrap();
        let folder2 = root2.path().join("project");
        std::fs::create_dir_all(&folder2).unwrap();
        let p2 = store2.register(&folder2).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let missing = Graphs::with(
            store2,
            Some(PathBuf::from("/nonexistent/graphify-missing")),
            Duration::ZERO,
        );
        missing.auto_start_projects();
        let deadline = Instant::now();
        let state2 = loop {
            let st = missing.status(&p2, false).unwrap();
            if st["state"] != "off" || deadline.elapsed() > Duration::from_secs(5) {
                break st;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(state2["state"], "missing", "{state2}");
        assert!(
            state2["message"]
                .as_str()
                .unwrap()
                .contains("Graphify is not installed"),
            "{state2}"
        );
        missing.close();
    }
    #[test]
    fn map_exposes_graph_focus_and_english_message_without_overwriting_focus() {
        let fixture = Fixture::new();
        fixture.graphs.auto_start(&fixture.project).unwrap();
        assert_eq!(fixture.settled()["state"], "ready");

        let view = fixture
            .graphs
            .map(&fixture.project, None, &Shown::default())
            .unwrap();
        assert_eq!(view["message"], "Code graph & dependency map ready.");
        assert_eq!(view["graph_focus"], Value::Null);
    }
    #[tokio::test]
    async fn domain_execute_auto_start_and_map_focus_session() {
        let fixture = Fixture::new();
        std::fs::write(
            fixture.folder.join("graph.fixture"),
            json!({
                "nodes": [
                    {"id": "n1", "label": "App", "source_file": "src/app.php"},
                    {"id": "n2", "label": "Auth", "source_file": "src/auth/guard.php"}
                ],
                "links": [
                    {"source": "n1", "target": "n2", "relation": "calls", "confidence": "EXTRACTED"}
                ]
            })
            .to_string(),
        )
        .unwrap();

        let Payload::Json(auto) = fixture
            .graphs
            .execute(
                "graph.auto_start",
                json!({
                    "params": {"project_id": fixture.project},
                    "body": {},
                    "query": {},
                }),
            )
            .await
            .unwrap()
        else {
            panic!("expected json auto_start payload");
        };
        assert_eq!(auto, json!({"ok": true}));
        assert_eq!(fixture.settled()["state"], "ready");

        let created = fixture
            .store
            .create(
                &json!({
                    "project_id": fixture.project,
                    "project_path": fixture.folder,
                    "provider": "codex",
                }),
                "Focus overlay test",
                &[],
            )
            .unwrap();
        let sid = created["id"].as_str().unwrap().to_owned();
        let focus = crate::graph_live::GraphFocus {
            phase: "active".into(),
            summary: "Live: editing src/auth/guard.php".into(),
            target_folders: vec!["src/auth".into()],
            target_paths: vec!["src/auth/guard.php".into()],
            target_nodes: vec!["src/auth".into()],
            created_entities: crate::graph_live::diff_new_entities(
                "src/auth/guard.php",
                Some(""),
                "<?php\nclass Guard {\n}\n",
                false,
            ),
        };
        crate::graph_live::save_session_focus(&fixture.store, &sid, &focus).unwrap();

        let Payload::Json(map) = fixture
            .graphs
            .execute(
                "graph.map",
                json!({
                    "params": {"project_id": fixture.project},
                    "body": {},
                    "query": {"scope": "src", "focus": "src/app.php", "focus_session": sid},
                }),
            )
            .await
            .unwrap()
        else {
            panic!("expected json map payload");
        };
        assert_eq!(map["message"], "Code graph & dependency map ready.");
        assert_eq!(map["focus"]["path"], "src/app.php", "{map}");
        assert_eq!(map["graph_focus"]["phase"], "active", "{map}");
        assert!(map.get("run").is_none(), "{map}");
        let auth_node = map["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["path"] == "src/auth")
            .expect("src/auth node present");
        assert_eq!(auth_node["focus_target"], true, "{auth_node}");
        assert_eq!(auth_node["created_entities_count"], 1, "{auth_node}");
    }
    #[test]
    fn auto_start_projects_prioritizes_most_recent_session_project_first() {
        let root = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&root.path().join("state")).unwrap();
        let folder1 = root.path().join("project_first");
        let folder2 = root.path().join("project_second");
        std::fs::create_dir_all(&folder1).unwrap();
        std::fs::create_dir_all(&folder2).unwrap();
        let p1 = store.register(&folder1).unwrap();
        let p1_id = p1["id"].as_str().unwrap().to_owned();
        let p2 = store.register(&folder2).unwrap();
        let p2_id = p2["id"].as_str().unwrap().to_owned();
        // Associate a session with p2 so summaries() ranks p2 ahead of p1.
        store
            .create(
                &json!({
                    "project_id": p2_id,
                    "project_path": folder2,
                    "provider": "codex",
                }),
                "Recent project",
                &[],
            )
            .unwrap();

        let tool_dir = root.path().join("tool");
        std::fs::create_dir_all(&tool_dir).unwrap();
        let tool = tool_dir.join("graphify");
        let text = tool.with_extension("text");
        std::fs::write(&text, TOOL).unwrap();
        let copied = Command::new("cp")
            .arg("--")
            .arg(&text)
            .arg(&tool)
            .status()
            .unwrap();
        assert!(copied.success());
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();

        let graphs = Graphs::with(store, Some(tool), Duration::ZERO);
        assert_eq!(
            graphs.auto_start_order(),
            vec![p2_id.clone(), p1_id.clone()],
            "most-recently-active session's project must run first"
        );

        graphs.auto_start_projects();
        let started = Instant::now();
        loop {
            let s1 = graphs.status(&p1_id, false).unwrap();
            let s2 = graphs.status(&p2_id, false).unwrap();
            if s1["state"] == "ready" && s2["state"] == "ready" {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "p1={s1} p2={s2}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        graphs.close();
    }
    #[test]
    fn auto_start_rebuilds_when_live_graph_is_missing() {
        let fixture = Fixture::new();
        fixture.graphs.auto_start(&fixture.project).unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        assert_eq!(fixture.runs(), 1);

        let graph = live(&fixture.store.state, &fixture.project);
        std::fs::remove_file(&graph).unwrap();
        assert!(!graph.is_file());

        fixture.graphs.auto_start(&fixture.project).unwrap();
        let rebuilt = fixture.settled();
        assert_eq!(rebuilt["state"], "ready", "{rebuilt}");
        assert!(graph.is_file());
        assert_eq!(fixture.runs(), 2);
    }
    #[test]
    fn non_git_workspace_edit_recomputes_on_finish_despite_automatic_gap() {
        let fixture = Fixture::new();
        std::fs::write(fixture.folder.join("service.php"), "<?php // v1\n").unwrap();
        let graphs = Graphs::with(
            fixture.store.clone(),
            Some(fixture.tool.clone()),
            Duration::from_secs(30),
        );
        graphs.auto_start(&fixture.project).unwrap();
        let settle = || {
            let started = Instant::now();
            loop {
                let current = graphs.status(&fixture.project, false).unwrap();
                if current["state"] != "building" {
                    return current;
                }
                assert!(started.elapsed() < Duration::from_secs(10));
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        assert_eq!(settle()["state"], "ready");
        assert_eq!(fixture.runs(), 1);

        // Unchanged non-Git workspace must not re-run.
        graphs.refresh(&fixture.project, Trigger::Finish).unwrap();
        assert_eq!(settle()["state"], "ready");
        assert_eq!(fixture.runs(), 1);

        // Ignored cache directories must not count as workspace source changes.
        std::fs::create_dir_all(fixture.folder.join("graphify-out")).unwrap();
        std::fs::write(fixture.folder.join("graphify-out/noise.txt"), "x").unwrap();
        graphs.refresh(&fixture.project, Trigger::Finish).unwrap();
        assert_eq!(settle()["state"], "ready");
        assert_eq!(fixture.runs(), 1);

        // An actual non-Git file edit triggers immediate Finish recompute even within the 30s gap.
        std::fs::write(
            fixture.folder.join("service.php"),
            "<?php // v2 with extra code\n",
        )
        .unwrap();
        graphs.refresh(&fixture.project, Trigger::Finish).unwrap();
        assert_eq!(settle()["state"], "ready");
        assert_eq!(fixture.runs(), 2);
        graphs.close();
    }
    #[test]
    fn job_view_always_reports_monotonic_percentage_and_english_message() {
        let job = Job::new();

        job.stage.store(WAITING, Ordering::Release);
        let wait = job.view(None);
        assert_eq!(wait["phase"], "wait");
        assert_eq!(wait["percent"], 5);
        assert!(wait["message"].as_str().unwrap().contains("5%"), "{wait}");

        job.stage.store(EXPORTING, Ordering::Release);
        let export = job.view(None);
        assert_eq!(export["phase"], "export");
        assert_eq!(export["percent"], 12);
        assert!(
            export["message"].as_str().unwrap().contains("12%"),
            "{export}"
        );

        job.stage.store(0, Ordering::Release);
        let scan = job.view(None);
        let scan_pct = scan["percent"].as_u64().expect("non-null scan percent");
        assert!((15..=28).contains(&scan_pct), "{scan}");
        assert!(
            scan["message"]
                .as_str()
                .unwrap()
                .contains(&format!("{scan_pct}%")),
            "{scan}"
        );

        job.observe(
            "  AST extraction: 1/4 uncached files (25%) [2 workers]",
            false,
        );
        let read = job.view(None);
        let read_pct = read["percent"].as_u64().expect("non-null read percent");
        assert!(
            read_pct >= scan_pct && (1..=99).contains(&read_pct),
            "{read}"
        );
        let read_msg = read["message"].as_str().unwrap();
        assert!(read_msg.contains("(1/4)"), "{read}");
        assert!(read_msg.contains(&format!("{read_pct}%")), "{read}");

        job.observe(
            "  AST extraction: 4/4 uncached files (100%) [2 workers]",
            false,
        );
        let assemble = job.view(None);
        let assemble_pct = assemble["percent"]
            .as_u64()
            .expect("non-null assemble percent");
        assert!(
            assemble_pct >= read_pct && (1..=99).contains(&assemble_pct),
            "{assemble}"
        );
        let assemble_msg = assemble["message"].as_str().unwrap();
        assert!(assemble_msg.contains("Computing"), "{assemble}");
        assert!(
            assemble_msg.contains(&format!("{assemble_pct}%")),
            "{assemble}"
        );
    }
    #[tokio::test]
    async fn domain_execute_graph_map_whole_validates_and_marks_unscoped_overview() {
        let fixture = Fixture::new();
        std::fs::write(
            fixture.folder.join("graph.fixture"),
            json!({
                "nodes": [
                    {"id": "n1", "label": "App", "source_file": "src/app.php"},
                    {"id": "n2", "label": "Guard", "source_file": "src/auth/guard.php"}
                ],
                "links": [
                    {"source": "n1", "target": "n2", "relation": "calls", "confidence": "EXTRACTED"}
                ]
            })
            .to_string(),
        )
        .unwrap();
        fixture.graphs.auto_start(&fixture.project).unwrap();
        assert_eq!(fixture.settled()["state"], "ready");

        let Payload::Json(overview) = fixture
            .graphs
            .execute(
                "graph.map",
                json!({
                    "params": {"project_id": fixture.project},
                    "body": {},
                    "query": {},
                }),
            )
            .await
            .unwrap()
        else {
            panic!("expected json map payload");
        };
        assert_eq!(overview["whole"], json!(false), "{overview}");

        let Payload::Json(whole) = fixture
            .graphs
            .execute(
                "graph.map",
                json!({
                    "params": {"project_id": fixture.project},
                    "body": {},
                    "query": {"whole": "1"},
                }),
            )
            .await
            .unwrap()
        else {
            panic!("expected json whole map payload");
        };
        assert_eq!(whole["whole"], json!(true), "{whole}");

        let invalid = fixture
            .graphs
            .execute(
                "graph.map",
                json!({
                    "params": {"project_id": fixture.project},
                    "body": {},
                    "query": {"whole": "2"},
                }),
            )
            .await;
        assert!(invalid.is_err(), "expected invalid whole flag error");
    }
    #[tokio::test]
    async fn domain_execute_changes_and_file_impact_cover_non_git_and_git_projects() {
        // Non-Git project with NO Graphify build must still answer without error.
        let non_git = Fixture::new();
        std::fs::create_dir_all(non_git.folder.join("src/auth")).unwrap();
        std::fs::write(
            non_git.folder.join("src/auth/guard.php"),
            "<?php\nclass Guard {\n}\n",
        )
        .unwrap();

        let Payload::Json(empty_catalog) = non_git
            .graphs
            .execute(
                "graph.changes",
                json!({
                    "params": {"project_id": non_git.project},
                    "body": {},
                    "query": {"notests": "1"},
                }),
            )
            .await
            .unwrap()
        else {
            panic!("expected json changes payload");
        };
        assert_eq!(empty_catalog["project_id"], non_git.project);
        assert_eq!(empty_catalog["target"], "workspace");
        assert_eq!(empty_catalog["notests"], true);
        assert_eq!(empty_catalog["total_files"], 0, "{empty_catalog}");

        let created = non_git
            .store
            .create(
                &json!({
                    "project_id": non_git.project,
                    "project_path": non_git.folder,
                    "provider": "codex",
                }),
                "Non-Git focus catalog test",
                &[],
            )
            .unwrap();
        let sid = created["id"].as_str().unwrap().to_owned();
        let focus = crate::graph_live::GraphFocus {
            phase: "active".into(),
            summary: "Live: editing src/auth/guard.php".into(),
            target_folders: vec!["src/auth".into()],
            target_paths: vec!["src/auth/guard.php".into()],
            target_nodes: vec!["src/auth".into()],
            created_entities: crate::graph_live::diff_new_entities(
                "src/auth/guard.php",
                Some(""),
                "<?php\nclass Guard {\n}\n",
                false,
            ),
        };
        crate::graph_live::save_session_focus(&non_git.store, &sid, &focus).unwrap();

        let Payload::Json(focused_catalog) = non_git
            .graphs
            .execute(
                "graph.changes",
                json!({
                    "params": {"project_id": non_git.project},
                    "body": {},
                    "query": {"focus_session": sid},
                }),
            )
            .await
            .unwrap()
        else {
            panic!("expected json focused changes payload");
        };
        assert_eq!(focused_catalog["total_files"], 1, "{focused_catalog}");
        assert_eq!(
            focused_catalog["files"][0]["path"], "src/auth/guard.php",
            "{focused_catalog}"
        );

        let Payload::Json(non_git_impact) = non_git
            .graphs
            .execute(
                "graph.file_impact",
                json!({
                    "params": {"project_id": non_git.project},
                    "body": {},
                    "query": {"path": "src/auth/guard.php", "focus_session": sid},
                }),
            )
            .await
            .unwrap()
        else {
            panic!("expected json non-git file impact payload");
        };
        assert_eq!(non_git_impact["project_id"], non_git.project);
        assert_eq!(non_git_impact["path"], "src/auth/guard.php");
        assert_eq!(non_git_impact["in_graph"], false);

        let bad_path = non_git
            .graphs
            .execute(
                "graph.file_impact",
                json!({
                    "params": {"project_id": non_git.project},
                    "body": {},
                    "query": {"path": "   "},
                }),
            )
            .await;
        assert!(bad_path.is_err(), "expected empty path rejection");

        // Git repo without Graphify build still computes diff, dependencies, and callers.
        let git_fix = Fixture::new();
        git_fix.git(&["init", "-b", "main"]);
        std::fs::create_dir_all(git_fix.folder.join("src")).unwrap();
        git_fix.write("src/helper.rs", "pub fn helper() -> i32 {\n    1\n}\n");
        git_fix.write(
            "src/service.rs",
            "use crate::helper::helper;\npub fn run() -> i32 {\n    helper()\n}\n",
        );
        git_fix.write(
            "src/caller.rs",
            "use crate::service::run;\npub fn entry() -> i32 {\n    run()\n}\n",
        );
        git_fix.commit("initial");

        git_fix.write("src/extra.rs", "pub fn extra() -> i32 {\n    2\n}\n");
        git_fix.write(
            "src/service.rs",
            "use crate::extra::extra;\npub fn run_v2() -> i32 {\n    extra()\n}\n",
        );

        let Payload::Json(git_catalog) = git_fix
            .graphs
            .execute(
                "graph.changes",
                json!({
                    "params": {"project_id": git_fix.project},
                    "body": {},
                    "query": {"target": "work"},
                }),
            )
            .await
            .unwrap()
        else {
            panic!("expected json git catalog payload");
        };
        assert_eq!(git_catalog["target"], "work");
        assert_eq!(git_catalog["total_files"], 2, "{git_catalog}");
        assert_eq!(git_catalog["created_files"], 1, "{git_catalog}");
        assert_eq!(git_catalog["modified_files"], 1, "{git_catalog}");

        let Payload::Json(service_impact) = git_fix
            .graphs
            .execute(
                "graph.file_impact",
                json!({
                    "params": {"project_id": git_fix.project},
                    "body": {},
                    "query": {"path": "src/service.rs", "target": "work", "certain": "1"},
                }),
            )
            .await
            .unwrap()
        else {
            panic!("expected json service file impact payload");
        };
        assert_eq!(service_impact["path"], "src/service.rs");
        assert_eq!(service_impact["status"], "modified", "{service_impact}");
        assert_eq!(
            service_impact["summary"]["uses_added"], 2,
            "{service_impact}"
        );
        assert_eq!(
            service_impact["summary"]["uses_removed"], 2,
            "{service_impact}"
        );
        assert!(
            service_impact["uses"]["added"]
                .as_array()
                .unwrap()
                .iter()
                .any(|dep| dep["target"] == "src/extra.rs"),
            "{service_impact}"
        );
        assert!(
            service_impact["uses"]["removed"]
                .as_array()
                .unwrap()
                .iter()
                .any(|dep| dep["target"] == "src/helper.rs"),
            "{service_impact}"
        );
        assert!(
            service_impact["used_by"]
                .as_array()
                .unwrap()
                .iter()
                .any(|caller| caller["path"] == "src/caller.rs"),
            "{service_impact}"
        );
    }

    #[test]
    fn live_summary_written_on_build_matches_graph_metadata_and_counts() {
        let fixture = Fixture::new();
        fixture.git(&["init", "-q", "--initial-branch=main"]);
        fixture.write("a.php", "<?php class A {}\n");
        fixture.write("b.php", "<?php class B {}\n");
        fixture.write(
                "graph.fixture",
                r#"{"nodes":[{"id":"a","source_file":"a.php"},{"id":"b","source_file":"b.php"}],"links":[{"source":"a","target":"b","relation":"calls","source_file":"a.php"}]}"#,
            );
        let head = fixture.commit("init");
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");

        let live_dir = folder(&fixture.store.state, &fixture.project).join("live");
        let graph_meta = std::fs::metadata(live_dir.join("graph.json")).unwrap();
        let modified = graph_meta
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        let summary: Value =
            serde_json::from_slice(&std::fs::read(live_dir.join("summary.json")).unwrap()).unwrap();
        assert_eq!(summary["nodes"], 2, "{summary}");
        assert_eq!(summary["edges"], 1, "{summary}");
        assert_eq!(summary["commit"], &head[..12], "{summary}");
        assert_eq!(summary["graph_len"], graph_meta.len(), "{summary}");
        assert_eq!(summary["graph_mtime_secs"], modified.as_secs(), "{summary}");
        assert_eq!(
            summary["graph_mtime_nanos"],
            modified.subsec_nanos(),
            "{summary}"
        );
    }

    #[test]
    fn thin_with_limits_preserves_sources_respects_bounds_and_cleans_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path();
        std::fs::create_dir_all(tree.join("node_modules/pkg")).unwrap();
        std::fs::create_dir_all(tree.join(".git/objects")).unwrap();
        std::fs::create_dir_all(tree.join("src")).unwrap();
        std::fs::write(tree.join("Cargo.toml"), "[package]\nname=\"demo\"\n").unwrap();
        std::fs::write(
            tree.join("node_modules/pkg/index.js"),
            "export const x = 1;",
        )
        .unwrap();
        std::fs::write(tree.join(".git/objects/abc"), "raw").unwrap();
        std::fs::write(tree.join("src/app.php"), "<?php\nclass App {}\n").unwrap();
        std::fs::write(tree.join("src/bundle.min.js"), "function a(){}").unwrap();
        std::fs::write(tree.join("src/image.png"), [0u8, 1, 2]).unwrap();
        std::fs::write(
            tree.join("src/big.rs"),
            "#[path = \"other.rs\"]\nmod other;\n#[test]\nfn oversized_test() {}\n",
        )
        .unwrap();

        // `src/big.rs` (69 bytes) exceeds max_file_bytes=40 so it falls back to `.rs.modules`
        // while still capturing its #[test] spans; `src/app.php` (17 bytes) is preserved.
        let spans = thin_with_limits(tree, 40, 200).unwrap();
        assert!(spans.contains_key("src/big.rs"), "{spans:?}");
        assert_eq!(
            held(tree),
            ["Cargo.toml", "src/app.php", "src/big.rs.modules",]
        );
        assert_eq!(
            std::fs::read_to_string(tree.join("src/big.rs.modules")).unwrap(),
            "#[path = \"other.rs\"\nmod other;"
        );
    }

    #[test]
    fn commit_graph_map_runs_access_analysis_and_emits_telemetry() {
        let fixture = Fixture::new();
        fixture.git(&["init", "-q", "--initial-branch=main"]);
        fixture.write(
            "user.php",
            "<?php\nclass User {\n    public int $count = 0;\n    public string $name = '';\n}\n",
        );
        fixture.write(
            "graph.fixture",
            r#"{"nodes":[{"id":"u","source_file":"user.php"}],"links":[]}"#,
        );
        let _main = fixture.commit("main");
        fixture.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);

        fixture.git(&["switch", "-q", "-c", "feat-access"]);
        fixture.write(
                "ctrl.php",
                "<?php\nrequire_once 'user.php';\nclass Ctrl {\n    public function run(): int {\n        $u = new User();\n        $u->name = 'ok';\n        return $u->count;\n    }\n}\n",
            );
        fixture.write(
                "graph.fixture",
                r#"{"nodes":[{"id":"u","label":"User","source_file":"user.php"},{"id":"u_cnt","label":"$count","source_file":"user.php","source_location":"L3"},{"id":"u_name","label":"$name","source_file":"user.php","source_location":"L4"},{"id":"c","label":"Ctrl","source_file":"ctrl.php"},{"id":"c_run","label":"run","source_file":"ctrl.php","source_location":"L4"}],"links":[{"source":"c_run","target":"u_name","relation":"uses","confidence":"EXTRACTED","source_file":"ctrl.php"},{"source":"c_run","target":"u_cnt","relation":"uses","confidence":"EXTRACTED","source_file":"ctrl.php"}]}"#,
            );
        let _feat = fixture.commit("feat");
        fixture.git(&["switch", "-q", "main"]);

        // Commit/PR graph map preserves sources in `revisions/<commit>/tree`, runs `Access`,
        // and attaches non-breaking `"telemetry"`.
        let target = Target::Named("feat-access".into());
        fixture.graphs.choose(&fixture.project, &target).unwrap();
        let first = fixture.mapped(&target);
        assert_eq!(first["state"], "ready", "{first}");
        assert!(first["access"].is_object(), "{first}");
        assert!(
            first["access"]["sources"].as_u64().unwrap_or(0) >= 1,
            "{first}"
        );
        let edge_kinds = first["edges"][0]["kinds"].as_array().unwrap();
        assert!(
            edge_kinds
                .iter()
                .any(|k| k["kind"] == "writes" && k["by"] == "source"),
            "{first}"
        );
        assert!(
            edge_kinds
                .iter()
                .any(|k| k["kind"] == "reads" && k["by"] == "source"),
            "{first}"
        );
        assert!(
            first["telemetry"]["elapsed_ms"].as_u64().is_some(),
            "{first}"
        );
        assert!(
            first["telemetry"]["shown"].as_u64().unwrap_or(0) >= 2,
            "{first}"
        );
        assert!(
            first["telemetry"]["drawn"].as_u64().unwrap_or(0) >= 1,
            "{first}"
        );
        assert_eq!(first["telemetry"]["truncated"], false, "{first}");

        let second = fixture
            .graphs
            .map(&fixture.project, Some(&target), &Shown::default())
            .unwrap();
        assert_eq!(second["telemetry"]["cache_hit"], true, "{second}");

        // Verify live graph map transitions cache_hit from false -> true.
        fixture.graphs.act(&fixture.project, "enable").unwrap();
        assert_eq!(fixture.settled()["state"], "ready");
        let live_first = fixture
            .graphs
            .map(&fixture.project, None, &Shown::default())
            .unwrap();
        assert_eq!(live_first["telemetry"]["cache_hit"], false, "{live_first}");
        let live_second = fixture
            .graphs
            .map(&fixture.project, None, &Shown::default())
            .unwrap();
        assert_eq!(live_second["telemetry"]["cache_hit"], true, "{live_second}");
    }

    #[test]
    fn emits_graph_status_events_and_supports_map_query_params() {
        assert!(MapQueryParams::parse(Some("0"), Some("50"), Some("2")).is_ok());
        assert!(MapQueryParams::parse(Some("0"), Some("501"), None).is_err());
        assert!(MapQueryParams::parse(Some("0"), Some("0"), None).is_err());
        assert!(MapQueryParams::parse(None, None, Some("7")).is_err());

        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let ws = dir.path().join("repo");
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::write(ws.join("src/a.rs"), "pub fn a_fn() -> i32 { 1 }\n").unwrap();
        std::fs::write(ws.join("src/b.rs"), "pub fn b_fn() -> i32 { 2 }\n").unwrap();

        let store = SessionStore::open(&state).unwrap();
        let proj = store.register(&ws).unwrap();
        let proj_id = proj["id"].as_str().unwrap().to_owned();
        let graphs = Graphs::with_native_backend(store.clone(), Duration::ZERO);
        let mut rx = graphs.subscribe_events();
        let mut global_rx = subscribe_graph_events();

        graphs.auto_start(&proj_id).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if graphs.status(&proj_id, false).unwrap()["state"] == "ready" {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(graphs.status(&proj_id, false).unwrap()["state"], "ready");

        let mut seen_reasons = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if ev.project_id == proj_id {
                seen_reasons.push(ev.reason);
            }
        }
        assert!(
            seen_reasons.contains(&"build_started") && seen_reasons.contains(&"build_finished"),
            "expected build lifecycle SSE events: {seen_reasons:?}"
        );

        // Architecture-only (and mixed non-code) edit bumps revision without reparsing or rebuilding Index.
        let arch_before_idx_id = graphs.live_index(&proj_id).unwrap().index_id();
        graphs.record_live_paths(&proj_id, &[".harness-architecture.json".to_owned()]);
        let mut saw_arch = false;
        while let Ok(ev) = rx.try_recv() {
            if ev.project_id == proj_id && ev.reason == "architecture_updated" {
                saw_arch = true;
            }
        }
        assert!(saw_arch, "expected architecture_updated SSE event");
        std::fs::write(ws.join("README.md"), "# Updated notes\n").unwrap();
        graphs.record_live_paths(
            &proj_id,
            &[
                ".harness-architecture.json".to_owned(),
                "README.md".to_owned(),
            ],
        );
        let mut saw_mixed_arch = false;
        while let Ok(ev) = rx.try_recv() {
            if ev.project_id == proj_id && ev.reason == "architecture_updated" {
                saw_mixed_arch = true;
            }
        }
        assert!(
            saw_mixed_arch,
            "expected architecture_updated for arch + doc batch"
        );
        assert_eq!(
            graphs.live_index(&proj_id).unwrap().index_id(),
            arch_before_idx_id,
            "arch/non-code batch must not rebuild Index"
        );

        let mut saw_global = false;
        while let Ok(ev) = global_rx.try_recv() {
            if ev.project_id == proj_id && ev.state_dir == store.state {
                saw_global = true;
            }
        }
        assert!(saw_global, "expected global subscriber event");

        // Paging and depth metadata returned cleanly on `graph.map`.
        let paged = graphs
            .map(
                &proj_id,
                None,
                &Shown {
                    scope: "src",
                    offset: 0,
                    limit: Some(1),
                    depth: Some(2),
                    ..Shown::default()
                },
            )
            .unwrap();
        assert_eq!(paged["paging"]["limit"], 1);
        assert_eq!(paged["paging"]["has_more"], true);
        assert_eq!(paged["depth"], 2);
        assert!(paged["ir_revision"].as_u64().unwrap_or(0) >= 1);

        graphs.close();
    }

    #[tokio::test]
    async fn p0_async_status_act_range_map_changes_and_single_pass_update() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let ws = dir.path().join("repo");
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::write(ws.join("src/a.rs"), "pub fn alpha() -> i32 { 1 }\n").unwrap();

        let git = |args: &[&str]| -> String {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&ws)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@example.test")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@example.test")
                .output()
                .unwrap();
            assert!(out.status.success(), "{args:?}: {out:?}");
            String::from_utf8_lossy(&out.stdout).trim().to_owned()
        };
        git(&["init", "--quiet"]);
        git(&["add", "--all"]);
        git(&["commit", "--quiet", "-m", "c1"]);
        let c1 = git(&["rev-parse", "HEAD"]);

        std::fs::write(
            ws.join("src/b.rs"),
            "pub fn beta() -> i32 { crate::a::alpha() }\n",
        )
        .unwrap();
        git(&["add", "--all"]);
        git(&["commit", "--quiet", "-m", "c2"]);
        let c2 = git(&["rev-parse", "HEAD"]);

        let store = SessionStore::open(&state).unwrap();
        let proj = store.register(&ws).unwrap();
        let pid = proj["id"].as_str().unwrap().to_owned();
        let graphs = Graphs::with_native_backend(store, Duration::ZERO);
        let domain = crate::services::Domain::execute(
            &*graphs,
            "graph.act",
            json!({"params":{"project_id":pid},"query":{},"body":{"action":"enable"}}),
        );
        let Payload::Json(enabled) = domain.await.unwrap() else {
            panic!("expected json");
        };
        assert!(enabled["enabled"] == true);

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let Payload::Json(st) = crate::services::Domain::execute(
                &*graphs,
                "graph.status",
                json!({"params":{"project_id":pid},"query":{"visit":"1"},"body":{}}),
            )
            .await
            .unwrap() else {
                panic!("expected json");
            };
            if st["state"] == "ready" {
                break;
            }
            assert!(Instant::now() < deadline, "timed out waiting for graph");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        // `target=range:<A>..<B>` and `from`/`to` on `graph.changes`.
        let Payload::Json(ch_target) = crate::services::Domain::execute(
                &*graphs,
                "graph.changes",
                json!({"params":{"project_id":pid},"query":{"target":format!("range:{c1}..{c2}")},"body":{}}),
            )
            .await
            .unwrap()
            else {
                panic!("expected json");
            };
        let Payload::Json(ch_params) = crate::services::Domain::execute(
            &*graphs,
            "graph.changes",
            json!({"params":{"project_id":pid},"query":{"from":c1,"to":c2},"body":{}}),
        )
        .await
        .unwrap() else {
            panic!("expected json");
        };
        assert!(
            ch_target["files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["path"] == "src/b.rs"),
            "expected src/b.rs in range target changes: {ch_target}"
        );
        assert_eq!(ch_target["files"], ch_params["files"]);
        assert!(
                crate::services::Domain::execute(
                    &*graphs,
                    "graph.changes",
                    json!({"params":{"project_id":pid},"query":{"target":format!("range:{c1}..{c2}"),"from":c1},"body":{}}),
                )
                .await
                .is_err()
            );

        // `graph.map` with `target=range:<A>..<B>` and relaxed `base=<short-sha>`.
        let Payload::Json(mapped) = crate::services::Domain::execute(
                &*graphs,
                "graph.map",
                json!({"params":{"project_id":pid},"query":{"target":format!("range:{c1}..{c2}"),"base":&c1[..7]},"body":{}}),
            )
            .await
            .unwrap()
            else {
                panic!("expected json");
            };
        assert!(mapped["nodes"].is_array(), "expected range map: {mapped}");

        // Single-pass `apply_ir_path_updates` updates live Index and live/graph.json.
        std::fs::write(ws.join("src/c.rs"), "pub fn gamma() -> i32 { 3 }\n").unwrap();
        graphs
            .apply_ir_path_updates(&pid, &ws, &["src/c.rs".to_owned()])
            .unwrap();
        let idx = graphs.live_index(&pid).expect("updated live index");
        assert!(idx.files().iter().any(|p| p == "src/c.rs"));

        let live_json = state.join("graphs").join(&pid).join("live/graph.json");
        let flush_deadline = Instant::now() + Duration::from_secs(5);
        let mut wrote_c = false;
        while Instant::now() < flush_deadline {
            if let Ok(text) = std::fs::read_to_string(&live_json)
                && text.contains("src/c.rs")
            {
                wrote_c = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(wrote_c, "expected live/graph.json to contain src/c.rs");

        // Range map propagates `view["base"]` and `view["confidence"]`.
        assert_eq!(
            mapped["base"]["name"]
                .as_str()
                .map(|s| s.starts_with(&c1[..7])),
            Some(true)
        );
        assert!(mapped["confidence"]["extracted_ratio"].is_number());

        // Manifest preservation & pending_paths queue drain checks.
        assert!(is_preserved_source_path(Path::new("go.mod")));
        assert!(is_preserved_source_path(Path::new("tsconfig.app.json")));
        assert!(is_preserved_source_path(Path::new("jsconfig.json")));

        // Active build queues watcher updates into Job::pending_paths and drains after settle.
        let busy_job = Arc::new(Job::new());
        busy_job
            .pending_paths
            .lock()
            .unwrap()
            .extend(["src/a.rs".to_owned(), "src/b.rs".to_owned()]);
        let mut lock_jobs = lock(&graphs.jobs).unwrap();
        lock_jobs.insert(pid.clone(), Arc::clone(&busy_job));
        drop(lock_jobs);
        graphs
            .apply_ir_path_updates(&pid, &ws, &["src/b.rs".to_owned(), "src/c.rs".to_owned()])
            .unwrap();
        let drained = std::mem::take(&mut *busy_job.pending_paths.lock().unwrap());
        assert_eq!(drained, vec!["src/a.rs", "src/b.rs", "src/c.rs"]);
        lock(&graphs.jobs).unwrap().remove(&pid);

        graphs.close();
    }

    #[test]
    fn lod_min_weight_declared_pr_layers_diff_overlay_and_chosen_lru() {
        // Query parser bounds for `lod` & `min_weight`.
        let parsed =
            MapQueryParams::parse_with(Some("1"), Some("25"), Some("2"), Some("crate"), Some("2"))
                .unwrap();
        assert_eq!(parsed.lod, Some(crate::graph_map::Lod::Crate));
        assert_eq!(parsed.min_weight, Some(2));
        assert!(MapQueryParams::parse_with(None, None, None, Some("bogus"), None).is_err());
        assert!(MapQueryParams::parse_with(None, None, None, None, Some("0")).is_err());

        let fixture = Fixture::new();
        fixture.git(&["init", "-q", "--initial-branch=main"]);
        std::fs::create_dir_all(fixture.folder.join("crates/api")).unwrap();
        std::fs::create_dir_all(fixture.folder.join("crates/repo")).unwrap();
        std::fs::create_dir_all(fixture.folder.join("src/api")).unwrap();
        std::fs::create_dir_all(fixture.folder.join("src/repo")).unwrap();
        fixture.write(
                ".harness-architecture.json",
                r#"{"version":1,"layers":[{"id":"api","name":"API","paths":["src/api"],"allows":["repo"]},{"id":"repo","name":"Repo","paths":["src/repo"],"allows":[]}]}"#,
            );
        fixture.write("crates/api/Cargo.toml", "[package]\nname=\"api\"\n");
        fixture.write("crates/repo/Cargo.toml", "[package]\nname=\"repo\"\n");
        fixture.write("src/api/Ctrl.php", "<?php class Ctrl {}\n");
        fixture.write("src/repo/Repo.php", "<?php class Repo {}\n");
        fixture.write(
                "graph.fixture",
                r#"{"nodes":[{"id":"c","label":"Ctrl.go","source_file":"src/api/Ctrl.php","source_location":"L1"},{"id":"r","label":"Repo.get","source_file":"src/repo/Repo.php","source_location":"L1"}],"links":[{"source":"c","target":"r","relation":"calls","confidence":"EXTRACTED"}]}"#,
            );
        let base = fixture.commit("base");
        fixture.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);

        fixture.git(&["switch", "-q", "-c", "feat-declared"]);
        fixture.write(
                "graph.fixture",
                r#"{"nodes":[{"id":"c","label":"Ctrl.go","source_file":"src/api/Ctrl.php","source_location":"L1"},{"id":"r","label":"Repo.get","source_file":"src/repo/Repo.php","source_location":"L1"}],"links":[{"source":"r","target":"c","relation":"calls","confidence":"EXTRACTED"},{"source":"r","target":"c","relation":"uses","confidence":"EXTRACTED"}]}"#,
            );
        let head = fixture.commit("head");
        let target = pull(&fixture, Some(&base), &head, Vec::new());
        fixture
            .graphs
            .revise(
                &fixture.project,
                &fixture.folder,
                &head,
                "",
                RevisionBuild::Normal,
            )
            .unwrap();
        rested(&fixture);

        // Declared layers comparison + `diff_overlay` works across PR head vs base.
        let _ = fixture
            .graphs
            .map(
                &fixture.project,
                Some(&target),
                &Shown {
                    pattern: Some("layers@"),
                    ..Default::default()
                },
            )
            .unwrap();
        rested(&fixture);
        let view = fixture
            .graphs
            .map(
                &fixture.project,
                Some(&target),
                &Shown {
                    pattern: Some("layers@"),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(view["pattern"]["declared"], true, "{view}");
        assert_eq!(view["pattern"]["change"]["state"], "ready", "{view}");
        assert_eq!(view["pattern"]["change"]["added"]["pairs"], 1, "{view}");
        assert_eq!(view["diff_overlay"]["base_available"], true, "{view}");
        assert!(
            view["diff_overlay"]["rule_violations"]["introduced"]["count"]
                .as_u64()
                .unwrap_or(0)
                >= 1,
            "{view}"
        );

        // `lod=crate` and `min_weight=2` wire cleanly through `Graphs::map`.
        let lod_view = fixture
            .graphs
            .map(
                &fixture.project,
                Some(&target),
                &Shown {
                    lod: Some(crate::graph_map::Lod::Crate),
                    min_weight: Some(2),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(lod_view["lod"], "crate");
        assert_eq!(lod_view["min_weight"], 2);

        // `chosen` bounded LRU evicts oldest entries beyond 64.
        for i in 0..70 {
            let r = Arc::new(crate::graph_changes::Resolved {
                target: Target::Commit(format!("{i:040x}")),
                kind: "commit",
                name: format!("c{i}"),
                title: None,
                pull: None,
                head: None,
                base: None,
                revision: Some(format!("{i:040x}")),
                prefix: String::new(),
                files: Vec::new(),
                notes: Vec::new(),
            });
            fixture
                .graphs
                .insert_chosen((fixture.project.clone(), format!("commit:{i:040x}")), r)
                .unwrap();
        }
        assert!(lock(&fixture.graphs.chosen).unwrap().len() <= Graphs::CHOSEN_LRU_CAP);
    }

    #[tokio::test]
    async fn ro_ir_store_cache_incremental_update_parity_and_cooldown() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let ws = dir.path().join("repo");
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::write(ws.join("src/a.rs"), "pub fn alpha() -> i32 { 1 }\n").unwrap();
        std::fs::write(
            ws.join("src/b.rs"),
            "pub fn beta() -> i32 { crate::a::alpha() }\n",
        )
        .unwrap();

        let store = SessionStore::open(&state).unwrap();
        let proj = store.register(&ws).unwrap();
        let pid = proj["id"].as_str().unwrap().to_owned();
        let graphs = Graphs::with_native_backend(store, Duration::ZERO);
        let Payload::Json(enabled) = crate::services::Domain::execute(
            &*graphs,
            "graph.act",
            json!({"params":{"project_id":pid},"query":{},"body":{"action":"enable"}}),
        )
        .await
        .unwrap() else {
            panic!("expected json");
        };
        assert_eq!(enabled["enabled"], true);

        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let Payload::Json(st) = crate::services::Domain::execute(
                &*graphs,
                "graph.status",
                json!({"params":{"project_id":pid},"query":{"visit":"1"},"body":{}}),
            )
            .await
            .unwrap() else {
                panic!("expected json");
            };
            if st["state"] == "ready" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(graphs.live_index(&pid).is_some());

        // Read-only `GraphIrStore` handle cache hits and `evict_ir_store` clears both per-manager and RO_CACHE handles.
        let ro1 = graphs.get_ir_store_readonly(&pid).expect("ro1");
        let ro2 = graphs.get_ir_store_readonly(&pid).expect("ro2");
        assert!(Arc::ptr_eq(&ro1, &ro2));
        let cached_ro1 = crate::graph_ir::GraphIrStore::open_readonly_cached(&state, &pid)
            .unwrap()
            .expect("cached_ro1");
        let cached_ro2 = crate::graph_ir::GraphIrStore::open_readonly_cached(&state, &pid)
            .unwrap()
            .expect("cached_ro2");
        assert!(Arc::ptr_eq(&cached_ro1, &cached_ro2));
        graphs.evict_ir_store(&pid);
        let ro3 = graphs.get_ir_store_readonly(&pid).expect("ro3");
        assert!(!Arc::ptr_eq(&ro1, &ro3));
        let cached_ro3 = crate::graph_ir::GraphIrStore::open_readonly_cached(&state, &pid)
            .unwrap()
            .expect("cached_ro3");
        assert!(!Arc::ptr_eq(&cached_ro1, &cached_ro3));
        drop(cached_ro1);
        drop(cached_ro2);
        drop(cached_ro3);
        graphs.evict_ir_store(&pid);

        // Rapid incremental updates coalesce via cooldown and produce exact full-rebuild parity.
        std::fs::write(
            ws.join("src/c.rs"),
            "pub fn gamma() -> i32 { crate::b::beta() }\n",
        )
        .unwrap();
        graphs
            .apply_ir_path_updates(&pid, &ws, &["src/c.rs".to_owned()])
            .unwrap();
        std::fs::write(
            ws.join("src/c.rs"),
            "pub fn gamma() -> i32 { crate::a::alpha() + crate::b::beta() }\n",
        )
        .unwrap();
        graphs
            .apply_ir_path_updates(&pid, &ws, &["src/c.rs".to_owned()])
            .unwrap();

        let inc_idx = graphs.live_index(&pid).expect("incremental index");
        let ir = graphs.get_ir_store_readonly(&pid).expect("ro4");
        let (full_nodes, full_links, _, _) = ir.export_ir_graph_elements().unwrap();
        let full_idx = crate::graph_map::Index::from_ir_parts(
            &ws,
            Some(ir.db_path().to_path_buf()),
            Some(crate::graph_ir::NATIVE_IR_TOOL.to_owned()),
            full_nodes,
            full_links,
        )
        .unwrap();
        assert_eq!(inc_idx.files(), full_idx.files());
        assert_eq!(inc_idx.totals(), full_idx.totals());
        assert_eq!(
            inc_idx.modules("demo", None, false, ""),
            full_idx.modules("demo", None, false, "")
        );
        assert_eq!(
            inc_idx.view("demo", "", None, false, ""),
            full_idx.view("demo", "", None, false, "")
        );

        let live_json = state.join("graphs").join(&pid).join("live/graph.json");
        let flush_deadline = Instant::now() + Duration::from_secs(5);
        let mut flushed = false;
        while Instant::now() < flush_deadline {
            if let Ok(text) = std::fs::read_to_string(&live_json)
                && text.contains("gamma()")
            {
                flushed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(flushed, "expected coalesced live/graph.json update");

        // `cached_diff_overlay` memoizes immutable (head, base, index, rules) and invalidates cleanly.
        let base_g = json!({
            "nodes": [{"id": "a", "label": "a()", "source_file": "src/a.rs", "source_location": "L1"}],
            "links": []
        });
        let base_idx = crate::graph_map::Index::read(base_g.to_string().as_bytes(), &ws).unwrap();
        let renamed = std::collections::HashMap::new();
        let changed = vec!["src/c.rs".to_owned()];
        let rules_a = crate::graph_map::layer_rules::Rules::parse(
            br#"{"version":1,"layers":[{"id":"app","name":"App","paths":["src/**"],"allows":["app"]}]}"#,
        )
        .unwrap();
        let rules_b = crate::graph_map::layer_rules::Rules::parse(
            br#"{"version":1,"layers":[{"id":"core","name":"Core","paths":["src/**"],"allows":[]}]}"#,
        )
        .unwrap();
        let ov1 = graphs.cached_diff_overlay(
            &pid,
            Some("rev1"),
            Some("rev0"),
            &inc_idx,
            Some(&base_idx),
            &changed,
            &renamed,
            false,
            false,
            Some(&rules_a),
        );
        let ov2 = graphs.cached_diff_overlay(
            &pid,
            Some("rev1"),
            Some("rev0"),
            &inc_idx,
            Some(&base_idx),
            &changed,
            &renamed,
            false,
            false,
            Some(&rules_a),
        );
        assert_eq!(ov1, ov2);
        assert_eq!(lock(&graphs.diff_overlays).unwrap().len(), 1);
        let _ov3 = graphs.cached_diff_overlay(
            &pid,
            Some("rev1"),
            Some("rev0"),
            &inc_idx,
            Some(&base_idx),
            &changed,
            &renamed,
            false,
            false,
            Some(&rules_b),
        );
        assert_eq!(lock(&graphs.diff_overlays).unwrap().len(), 2);

        graphs.close();
    }

    #[test]
    fn incremental_updates_emit_sse_delta_hints_and_handle_batches_above_512() {
        let dir = tempfile::TempDir::new().unwrap();
        let state = dir.path().join("state");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "pub fn alpha() -> i32 { 1 }\n").unwrap();
        let store = SessionStore::open(&state).unwrap();
        let project = store.register(&root).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let graphs = Graphs::with_native_backend(store, Duration::ZERO);
        let mut rx = graphs.events.subscribe();
        assert_eq!(graphs.act(&project, "enable").unwrap()["state"], "building");
        let dead = Instant::now() + Duration::from_secs(10);
        while Instant::now() < dead {
            if graphs.status(&project, false).unwrap()["state"] == "ready" {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        while rx.try_recv().is_ok() {}
        std::fs::write(
            root.join("src/b.rs"),
            "pub fn beta() -> i32 { crate::a::alpha() }\n",
        )
        .unwrap();
        graphs
            .apply_ir_path_updates(&project, &root, &["src/b.rs".to_owned()])
            .unwrap();

        let mut matched_event = None;
        let event_dead = Instant::now() + Duration::from_secs(3);
        while Instant::now() < event_dead {
            if let Ok(ev) = rx.try_recv()
                && ev.project_id == project
                && ev.reason == "incremental_update"
            {
                matched_event = Some(ev);
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let ev = matched_event.expect("incremental_update SSE status event");
        assert_eq!(ev.changed_files, vec!["src/b.rs".to_owned()]);
        assert_eq!(ev.ir_revision, ev.revision);
        assert!(!ev.full_reload);

        let serialized = serde_json::to_value(&ev).unwrap();
        assert_eq!(serialized["changed_files"], json!(["src/b.rs"]));
        assert_eq!(serialized["ir_revision"], json!(ev.ir_revision));

        // Ensure >512 rel_paths do not fall back to full sync_workspace_with_delta.
        let mut batch = Vec::new();
        for i in 0..600 {
            let rel = format!("src/f{i:03}.rs");
            if i < 4 {
                std::fs::write(root.join(&rel), format!("pub fn f{i}() -> i32 {{ {i} }}\n"))
                    .unwrap();
            }
            batch.push(rel);
        }
        let large_delta = graphs
            .finalize_ir_update_delta(&project, &root, &batch)
            .unwrap()
            .expect("large batch incremental delta");
        assert!(
            large_delta.stats.files_scanned >= 600,
            "all 600 paths should be processed without MAX_WATCHER_BATCH tail truncation: {:?}",
            large_delta.stats
        );
        assert!(
            large_delta.edited_files.contains(&"src/f003.rs".to_owned()),
            "expected newly added files processed"
        );

        graphs.close();
    }
}

#[test]
fn a_folder_that_cannot_go_yet_is_tried_again_only_while_it_may_settle() {
    use std::sync::atomic::AtomicUsize;
    let busy = || std::io::Error::new(std::io::ErrorKind::PermissionDenied, "still open");
    let home = Path::new("graph-home");
    // Two refusals, then the file is let go: the removal succeeds.
    let calls = AtomicUsize::new(0);
    remove_until(home, Duration::from_secs(5), |_| {
        if calls.fetch_add(1, Ordering::AcqRel) < 2 {
            Err(busy())
        } else {
            Ok(())
        }
    })
    .unwrap();
    assert_eq!(calls.load(Ordering::Acquire), 3);
    // No time to settle: the first refusal is the answer.
    let calls = AtomicUsize::new(0);
    let refused = remove_until(home, Duration::ZERO, |_| {
        calls.fetch_add(1, Ordering::AcqRel);
        Err(busy())
    });
    assert_eq!(
        refused.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(calls.load(Ordering::Acquire), 1);
    // A refusal that does not pass is reported once the time is up.
    let started = Instant::now();
    assert!(remove_until(home, Duration::from_millis(120), |_| Err(busy())).is_err());
    assert!(started.elapsed() >= Duration::from_millis(120));
    // A folder that is already gone was removed.
    remove_until(home, Duration::ZERO, |_| {
        Err(std::io::Error::from(std::io::ErrorKind::NotFound))
    })
    .unwrap();
}

#[test]
fn git_print_cache_respects_racy_window_and_invalidates_on_head_index_or_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let run_git = |args: &[&str]| {
        let owned: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        let out = projects::git(root, &owned, 20).expect("git invocation");
        assert_eq!(out.code, 0, "git {args:?} failed: {:?}", out);
    };
    run_git(&["init", "-b", "main"]);
    run_git(&["config", "user.name", "Test"]);
    run_git(&["config", "user.email", "test@example.com"]);
    let tracked = root.join("app.txt");
    std::fs::write(&tracked, "v1\n").unwrap();
    run_git(&["add", "app.txt"]);
    run_git(&["commit", "-m", "init"]);

    let max_mtime_ns = || -> i128 {
        let mut latest = 0_i128;
        for p in [
            root.to_path_buf(),
            root.join("app.txt"),
            root.join(".git/HEAD"),
            root.join(".git/index"),
            root.join(".git/refs/heads/main"),
        ] {
            if let Ok(m) = std::fs::metadata(&p)
                && let Ok(t) = m.modified()
                && let Ok(d) = t.duration_since(std::time::UNIX_EPOCH)
            {
                latest = latest.max(d.as_nanos() as i128);
            }
        }
        latest
    };

    // Inside racy window (< PRINT_RACY_NS): must still spawn git rather than trusting cache.
    let base_t = max_mtime_ns();
    let first = helpers::print_with_now(root, base_t + 1_000).expect("initial print");
    let spawns_before = helpers::LOCAL_PRINT_GIT_SPAWN_COUNT.with(|c| c.get());
    let second_racy = helpers::print_with_now(root, base_t + 2_000).expect("racy print");
    let spawns_after_racy = helpers::LOCAL_PRINT_GIT_SPAWN_COUNT.with(|c| c.get());
    assert_eq!(first, second_racy);
    assert!(
        spawns_after_racy > spawns_before,
        "prints within racy window must not serve from cache without verification"
    );

    // Populate settled snapshot well after racy window -> next read must hit cache with 0 git spawns.
    let settled_t = max_mtime_ns() + helpers::PRINT_RACY_NS + 5_000_000_000;
    let warmed = helpers::print_with_now(root, settled_t).expect("warm settled cache");
    let spawns_warmed = helpers::LOCAL_PRINT_GIT_SPAWN_COUNT.with(|c| c.get());
    let cached = helpers::print_with_now(root, settled_t + 1_000_000).expect("cached hit");
    let spawns_cached = helpers::LOCAL_PRINT_GIT_SPAWN_COUNT.with(|c| c.get());
    assert_eq!(warmed, cached);
    assert_eq!(
        spawns_cached, spawns_warmed,
        "settled cache hit must spawn zero git processes"
    );

    // Worktree modification invalidates cache even without git add/commit.
    std::fs::write(&tracked, "v2-worktree\n").unwrap();
    let dirty_t = max_mtime_ns() + helpers::PRINT_RACY_NS + 10_000_000_000;
    let dirty = helpers::print_with_now(root, dirty_t).expect("dirty print");
    assert_ne!(dirty.hash, cached.hash);

    // Index change (.git/index) invalidates cache.
    run_git(&["add", "app.txt"]);
    let staged_t = max_mtime_ns() + helpers::PRINT_RACY_NS + 15_000_000_000;
    let staged = helpers::print_with_now(root, staged_t).expect("staged print");
    let spawns_staged_warm = helpers::LOCAL_PRINT_GIT_SPAWN_COUNT.with(|c| c.get());
    let staged_hit = helpers::print_with_now(root, staged_t + 1_000_000).expect("staged cache hit");
    assert_eq!(staged, staged_hit);
    assert_eq!(
        helpers::LOCAL_PRINT_GIT_SPAWN_COUNT.with(|c| c.get()),
        spawns_staged_warm
    );

    // HEAD / branch ref change (git commit) invalidates cache.
    run_git(&["commit", "-m", "second"]);
    let committed_t = max_mtime_ns() + helpers::PRINT_RACY_NS + 20_000_000_000;
    let committed = helpers::print_with_now(root, committed_t).expect("committed print");
    assert_ne!(committed.commit, cached.commit);
    assert_ne!(committed.hash, cached.hash);
}

#[test]
fn git_print_cache_detects_untracked_files_in_empty_and_ignored_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let run_git = |args: &[&str]| {
        let owned: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        let out = projects::git(root, &owned, 20).expect("git invocation");
        assert_eq!(out.code, 0, "git {args:?} failed: {:?}", out);
    };
    run_git(&["init", "-b", "main"]);
    run_git(&["config", "user.name", "Test"]);
    run_git(&["config", "user.email", "test@example.com"]);
    std::fs::write(root.join(".gitignore"), "ignored_only/*.tmp\n").unwrap();
    std::fs::write(root.join("app.txt"), "v1\n").unwrap();
    let empty_dir = root.join("empty_sub");
    let ignored_dir = root.join("ignored_only");
    std::fs::create_dir_all(&empty_dir).unwrap();
    std::fs::create_dir_all(&ignored_dir).unwrap();
    std::fs::write(ignored_dir.join("scratch.tmp"), "ign\n").unwrap();
    run_git(&["add", ".gitignore", "app.txt"]);
    run_git(&["commit", "-m", "init"]);

    let max_mtime_ns = || -> i128 {
        let mut latest = 0_i128;
        for p in [
            root.to_path_buf(),
            root.join("app.txt"),
            root.join(".gitignore"),
            empty_dir.clone(),
            empty_dir.join("new.php"),
            ignored_dir.clone(),
            ignored_dir.join("scratch.tmp"),
            root.join(".git/HEAD"),
            root.join(".git/index"),
            root.join(".git/refs/heads/main"),
        ] {
            if let Ok(m) = std::fs::metadata(&p)
                && let Ok(t) = m.modified()
                && let Ok(d) = t.duration_since(std::time::UNIX_EPOCH)
            {
                latest = latest.max(d.as_nanos() as i128);
            }
        }
        latest
    };

    let t1 = max_mtime_ns() + helpers::PRINT_RACY_NS + 5_000_000_000;
    let p1 = helpers::print_with_now(root, t1).expect("warm settled cache");
    let spawns_after_warm = helpers::LOCAL_PRINT_GIT_SPAWN_COUNT.with(|c| c.get());
    let p1_hit = helpers::print_with_now(root, t1 + 1_000_000).expect("cache hit");
    assert_eq!(p1, p1_hit);
    assert_eq!(
        helpers::LOCAL_PRINT_GIT_SPAWN_COUNT.with(|c| c.get()),
        spawns_after_warm
    );

    // Creating a new untracked file inside a pre-existing empty directory must invalidate cache.
    std::fs::write(empty_dir.join("new.php"), "<?php\n").unwrap();
    let t2 = max_mtime_ns() + helpers::PRINT_RACY_NS + 10_000_000_000;
    let p2 = helpers::print_with_now(root, t2).expect("print after empty_sub untracked");
    assert_ne!(
        p2.hash, p1.hash,
        "creating untracked file in previously empty directory must bust git print cache"
    );

    // Stage/commit `empty_sub/new.php` so `git status` is clean again and ignored_only settles into cache.
    run_git(&["add", "empty_sub/new.php"]);
    run_git(&["commit", "-m", "track new.php"]);
    let t2_clean = max_mtime_ns() + helpers::PRINT_RACY_NS + 15_000_000_000;
    let p2_clean = helpers::print_with_now(root, t2_clean).expect("re-warm settled cache");
    let spawns_before_ign_hit = helpers::LOCAL_PRINT_GIT_SPAWN_COUNT.with(|c| c.get());
    let p2_hit = helpers::print_with_now(root, t2_clean + 1_000_000).expect("settled hit");
    assert_eq!(p2_clean, p2_hit);
    assert_eq!(
        helpers::LOCAL_PRINT_GIT_SPAWN_COUNT.with(|c| c.get()),
        spawns_before_ign_hit
    );

    // Creating a non-ignored untracked file inside an ignored-only dir must also invalidate cache.
    std::fs::write(ignored_dir.join("visible.php"), "<?php\n").unwrap();
    let t3 = max_mtime_ns() + helpers::PRINT_RACY_NS + 20_000_000_000;
    let p3 = helpers::print_with_now(root, t3).expect("print after ignored_only untracked");
    assert_ne!(
        p3.hash, p2_clean.hash,
        "creating visible file in directory with only ignored files must bust git print cache"
    );
}
