use super::*;

const LISTED: usize = 16;

#[derive(Default)]
struct Module {
    symbols: usize,
    files: BTreeSet<u32>,
    inside: Flow,
    out: Flow,
    into: Flow,
}

impl Index {
    pub(crate) fn modules(
        &self,
        project: &str,
        focus: Option<&str>,
        certain: bool,
        filter: &str,
    ) -> Value {
        let mut modules: BTreeMap<u32, Module> = BTreeMap::new();
        let mut unassigned = 0;
        for symbol in &self.symbols {
            if symbol.kind == Kind::Note {
                continue;
            }
            if let Some(id) = symbol.community {
                let module = modules.entry(id).or_default();
                module.symbols += 1;
                module.files.insert(symbol.file);
            } else {
                unassigned += 1;
            }
        }
        let selected = focus
            .and_then(|path| path.strip_prefix("module:")?.parse::<u32>().ok())
            .filter(|id| modules.contains_key(id));
        let mut edges: BTreeMap<(u32, u32), EdgeFlow> = BTreeMap::new();
        let mut inside: BTreeMap<(u32, u32), Flow> = BTreeMap::new();
        let mut outside = 0;
        let add = |flow: &mut Flow, guessed: bool| {
            flow.links += 1;
            flow.guessed += u64::from(guessed);
        };
        for &(from, to, kind, guessed) in &self.uses {
            let (source, target) = (&self.symbols[from as usize], &self.symbols[to as usize]);
            if (certain && guessed) || source.kind == Kind::Note || target.kind == Kind::Note {
                continue;
            }
            match (source.community, target.community) {
                (Some(source_id), Some(target_id)) if source_id == target_id => {
                    add(&mut modules.get_mut(&source_id).unwrap().inside, guessed);
                    if selected == Some(source_id) {
                        add(inside.entry((from, to)).or_default(), guessed);
                    }
                }
                (Some(source_id), Some(target_id)) => {
                    edges
                        .entry((source_id, target_id))
                        .or_default()
                        .add_link(kind, guessed);
                    add(&mut modules.get_mut(&source_id).unwrap().out, guessed);
                    add(&mut modules.get_mut(&target_id).unwrap().into, guessed);
                }
                _ => outside += 1,
            }
        }
        let key = |id| format!("module:{id}");
        let label = |id| {
            self.community_names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| format!("Community {id}"))
        };
        let node = |id, module: &Module| {
            json!({
                "path": key(id), "name": label(id), "module": true,
                "files": module.files.len(), "symbols": module.symbols,
                "within": module.inside.links, "inside_guessed": module.inside.guessed,
                "out": module.out.links, "in": module.into.links,
            })
        };
        let query = filter.trim().to_lowercase();
        let mut matching: Vec<_> = modules
            .iter()
            .filter(|(id, module)| {
                query.is_empty()
                    || label(**id).to_lowercase().contains(&query)
                    || id.to_string().contains(&query)
                    || module
                        .files
                        .iter()
                        .any(|file| self.files[*file as usize].to_lowercase().contains(&query))
            })
            .collect();
        matching.sort_by_key(|(id, module)| {
            (
                selected != Some(**id),
                std::cmp::Reverse(module.symbols),
                **id,
            )
        });
        let shown: HashSet<_> = matching.iter().take(SHOWN).map(|(id, _)| **id).collect();
        let mut drawn: Vec<_> = edges
            .iter()
            .filter(|((from, to), _)| shown.contains(from) && shown.contains(to))
            .collect();
        drawn.sort_by_key(|(pair, edge)| (std::cmp::Reverse(edge.flow.links), **pair));
        let focus = selected.map(|id| {
            let module = &modules[&id];
            let mut focused = node(id, module);
            let mut paths: Vec<_> = module.files.iter().map(|file| &self.files[*file as usize]).collect();
            paths.sort_unstable();
            focused["members"] = json!(paths.into_iter().take(LISTED).collect::<Vec<_>>());
            let mut ranked: Vec<_> = inside.into_iter().collect();
            ranked.sort_by_key(|(pair, flow)| (std::cmp::Reverse(flow.links), *pair));
            focused["internal_total"] = json!(ranked.len());
            focused["internal"] = json!(ranked.into_iter().take(LISTED).map(|((from, to), flow)| {
                let endpoint = |at| {
                    let symbol: &Symbol = &self.symbols[at as usize];
                    let file = &self.files[symbol.file as usize];
                    json!({"name": self.shown(at), "file": file, "line": symbol.line})
                };
                json!({"from": endpoint(from), "to": endpoint(to), "links": flow.links, "guessed": flow.guessed})
            }).collect::<Vec<_>>());
            for (outgoing, field) in [(true, "uses"), (false, "used_by")] {
                let mut peers: Vec<_> = edges.iter().filter_map(|(&(from, to), edge)| {
                    if outgoing { (from == id).then_some((to, edge.flow)) }
                    else { (to == id).then_some((from, edge.flow)) }
                }).collect();
                peers.sort_by_key(|(peer, flow)| (std::cmp::Reverse(flow.links), *peer));
                focused[format!("{field}_total")] = json!(peers.len());
                focused[field] = json!(peers.into_iter().take(LISTED).map(|(peer, flow)| {
                    json!({"path": key(peer), "name": label(peer), "links": flow.links, "guessed": flow.guessed})
                }).collect::<Vec<_>>());
            }
            focused
        });
        json!({
            "mode":"modules", "scope":"", "crumbs":Self::crumbs(project, ""), "certain":certain,
            "children": modules.len(), "matching": matching.len(), "between": edges.len(),
            "module_info":{"unassigned":unassigned, "outside":outside},
            "nodes":matching.into_iter().take(SHOWN).map(|(id, module)| node(*id, module)).collect::<Vec<_>>(),
            "edges":drawn.into_iter().take(DRAWN).map(|(&(from, to), edge)| {
                self.edge_json(&key(from), &key(to), edge)
            }).collect::<Vec<_>>(),
            "focus":focus,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn community_edges_count_inside_and_crossing_links_with_filters() {
        let test = "tests/CartTest.php";
        let input = json!({
            "nodes": [
                {"id":"a", "label":"cart()", "source_file":"src/cart.php", "community":0, "community_name":"Cart"},
                {"id":"b", "label":"total()", "source_file":"src/cart.php", "community":0},
                {"id":"c", "label":"payment()", "source_file":"src/cart.php", "community":"5", "community_name":"Payment"},
                {"id":"d", "label":"test()", "source_file":test, "community":5},
                {"id":"e", "label":"utility()", "source_file":"src/util.php", "community":null},
            ],
            "links": [
                {"source":"a","target":"b","relation":"calls","confidence":"EXTRACTED"},
                {"source":"b","target":"c","relation":"calls","confidence":"EXTRACTED"},
                {"source":"c","target":"a","relation":"calls","confidence":"INFERRED"},
                {"source":"d","target":"c","relation":"calls","confidence":"EXTRACTED"},
                {"source":"c","target":"e","relation":"calls","confidence":"EXTRACTED"},
            ],
        });
        let index = Index::read(input.to_string().as_bytes(), Path::new("/work/shop")).unwrap();
        let view = index.modules("shop", Some("module:0"), false, "");
        assert_eq!(view["children"], 2);
        assert_eq!(view["edges"].as_array().unwrap().len(), 2);
        assert_eq!(view["module_info"], json!({"unassigned":1,"outside":1}));
        assert_eq!(view["focus"]["name"], "Cart");
        assert_eq!(view["focus"]["members"], json!(["src/cart.php"]));
        assert_eq!(view["focus"]["within"], 1);
        assert_eq!(view["focus"]["internal"][0]["from"]["name"], "cart()");
        assert_eq!(view["focus"]["uses"][0]["name"], "Payment");
        assert_eq!(view["focus"]["used_by"][0]["guessed"], 1);
        let plain = index
            .without_tests()
            .modules("shop", Some("module:5"), true, "src/cart");
        assert_eq!(plain["matching"], 2);
        assert_eq!(plain["edges"].as_array().unwrap().len(), 1);
        assert_eq!(plain["focus"]["members"], json!(["src/cart.php"]));
        assert_eq!(plain["focus"]["within"], 0);
        let empty = index.modules("shop", None, false, "missing");
        assert_eq!(empty["matching"], 0);
    }

    #[test]
    fn module_edges_include_aggregated_dependency_kinds() {
        let input = json!({
            "nodes": [
                {"id": "a", "label": "Ctrl", "source_file": "src/a.php", "community": 0, "community_name": "Ctrl"},
                {"id": "b", "label": "Svc", "source_file": "src/b.php", "community": 5, "community_name": "Svc"},
            ],
            "links": [
                {"source": "a", "target": "b", "relation": "calls", "confidence": "EXTRACTED"},
                {"source": "a", "target": "b", "relation": "calls", "confidence": "INFERRED"},
                {"source": "a", "target": "b", "relation": "imports", "confidence": "EXTRACTED"},
                {"source": "a", "target": "b", "relation": "extends", "confidence": "EXTRACTED"},
                {"source": "a", "target": "b", "relation": "implements", "confidence": "EXTRACTED"},
                {"source": "a", "target": "b", "relation": "reads", "confidence": "EXTRACTED"},
                {"source": "a", "target": "b", "relation": "writes", "confidence": "EXTRACTED"},
                {"source": "a", "target": "b", "relation": "uses_const", "confidence": "EXTRACTED"},
            ],
        });
        let index = Index::read(input.to_string().as_bytes(), Path::new("/work/shop")).unwrap();
        let all = index.modules("shop", None, false, "");
        assert_eq!(
            all["edges"],
            json!([{
                "from": "module:0",
                "to": "module:5",
                "links": 8,
                "guessed": 1,
                "kinds": [
                    {"kind": "calls", "links": 2},
                    {"kind": "extends", "links": 1},
                    {"kind": "implements", "links": 1},
                    {"kind": "imports", "links": 1},
                    {"kind": "reads", "links": 1},
                    {"kind": "uses_const", "links": 1},
                    {"kind": "writes", "links": 1},
                ],
            }])
        );
        let certain = index.modules("shop", None, true, "");
        assert_eq!(
            certain["edges"],
            json!([{
                "from": "module:0",
                "to": "module:5",
                "links": 7,
                "guessed": 0,
                "kinds": [
                    {"kind": "calls", "links": 1},
                    {"kind": "extends", "links": 1},
                    {"kind": "implements", "links": 1},
                    {"kind": "imports", "links": 1},
                    {"kind": "reads", "links": 1},
                    {"kind": "uses_const", "links": 1},
                    {"kind": "writes", "links": 1},
                ],
            }])
        );
    }

    #[test]
    fn native_ir_populates_communities_usable_in_modules_mode() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let root = dir.path().join("app");
        std::fs::create_dir_all(root.join("src/Billing")).unwrap();
        std::fs::create_dir_all(root.join("src/Orders")).unwrap();
        std::fs::write(
            root.join("composer.json"),
            r#"{"autoload":{"psr-4":{"App\\":"src/"}}}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("src/Billing/Tax.php"),
            "<?php\nnamespace App\\Billing;\nclass Tax { public function calc(): int { return 1; } }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/Orders/Checkout.php"),
            "<?php\nnamespace App\\Orders;\nuse App\\Billing\\Tax;\nclass Checkout { public function run(Tax $tax): int { return $tax->calc(); } }\n",
        )
        .unwrap();

        let ir = crate::graph_ir::GraphIrStore::open(&state, "mod").unwrap();
        ir.sync_workspace(&root).unwrap();
        let (nodes, links, ..) = ir.export_ir_graph_elements().unwrap();
        let idx = Index::from_ir_parts(
            &root,
            Some(ir.db_path().to_path_buf()),
            Some(crate::graph_ir::NATIVE_IR_TOOL.to_owned()),
            nodes,
            links,
        )
        .unwrap();
        let view = idx.modules("mod", None, false, "");
        assert_eq!(view["children"].as_u64(), Some(2));
        assert!(
            !view["edges"].as_array().unwrap().is_empty(),
            "expected cross-module edges from native IR: {view}"
        );
    }
}
