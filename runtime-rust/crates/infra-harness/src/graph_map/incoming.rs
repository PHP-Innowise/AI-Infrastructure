use super::*;

const LISTED: usize = 20;

impl Index {
    pub(crate) fn mark_without_incoming(&self, view: &mut Value, certain: bool, filter: &str) {
        let eligible = |symbol: &Symbol| !matches!(symbol.kind, Kind::Note | Kind::Heading);
        let mut file_incoming = vec![false; self.files.len()];
        let mut symbol_incoming = vec![false; self.symbols.len()];
        for &(from, to, _, guessed) in &self.uses {
            let source = &self.symbols[from as usize];
            let target = &self.symbols[to as usize];
            if (certain && guessed) || !eligible(source) || !eligible(target) {
                continue;
            }
            symbol_incoming[to as usize] = true;
            if source.file != target.file {
                file_incoming[target.file as usize] = true;
            }
        }

        let scope = view["scope"].as_str().unwrap_or("");
        let query = filter.trim().to_lowercase();
        let matches = |text: &str| query.is_empty() || text.to_lowercase().contains(&query);
        let mut files: Vec<_> = self
            .files
            .iter()
            .enumerate()
            .filter(|(at, path)| {
                (scope.is_empty()
                    || *path == scope
                    || path
                        .strip_prefix(scope)
                        .is_some_and(|tail| tail.starts_with('/')))
                    && self
                        .range(*at as u32)
                        .any(|id| eligible(&self.symbols[id as usize]))
            })
            .collect();
        files.sort_unstable_by_key(|(_, path)| *path);
        let names = Names::new(self, &[], &|_| None);
        let mut file_rows = Vec::new();
        let mut symbol_rows = Vec::new();
        let (mut file_total, mut symbol_total) = (0, 0);
        for (file, path) in files {
            let matches_path = matches(path);
            if !file_incoming[file] && matches_path {
                file_total += 1;
                if file_rows.len() < LISTED {
                    file_rows.push(json!({"path": path}));
                }
            }
            for at in self.range(file as u32) {
                let symbol = &self.symbols[at as usize];
                if !eligible(symbol)
                    || symbol.kind == Kind::File
                    || symbol_incoming[at as usize]
                    || (!matches_path && !matches(&self.shown(at)))
                {
                    continue;
                }
                symbol_total += 1;
                if symbol_rows.len() < LISTED {
                    symbol_rows.push(names.told(at, &names.key(at)));
                }
            }
        }
        view["without_incoming"] = json!({
            "files": file_rows, "file_total": file_total,
            "symbols": symbol_rows, "symbol_total": symbol_total,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incoming_links_use_the_whole_graph_and_distinguish_files_from_symbols() {
        let node = |id: &str, file: &str| {
            json!({
                "id":id, "label":format!("{id}()"), "source_file":file, "source_location":"L2"
            })
        };
        let edge = |from: &str, to: &str, confidence: &str| json!({"source":from,"target":to,"relation":"calls","confidence":confidence});
        let graph = json!({
            "nodes": [
                node("entry", "src/entry.php"),
                node("helper", "src/entry.php"),
                node("external", "web/index.php"),
                node("received", "src/received.php"),
                node("inferred", "src/inferred.php"),
                node("tested", "src/tested.php"),
                node("test", "tests/ReachTest.php"),
                {"id":"heading","label":"Documentation","source_file":"src/docs.md","file_type":"document"}
            ],
            "links": [
                edge("entry", "helper", "EXTRACTED"),
                edge("external", "received", "EXTRACTED"),
                edge("external", "inferred", "INFERRED"),
                edge("test", "tested", "EXTRACTED"),
            ]
        });
        let index = Index::read(graph.to_string().as_bytes(), Path::new("/work/shop")).unwrap();
        let report = |index: &Index, certain: bool, query: &str| {
            let mut view = json!({"scope":"src"});
            index.mark_without_incoming(&mut view, certain, query);
            view["without_incoming"].clone()
        };
        let all = report(&index, false, "");
        assert_eq!(all["files"], json!([{"path":"src/entry.php"}]));
        assert_eq!(all["symbol_total"], 1);
        assert_eq!(all["symbols"][0]["path"], "src/entry.php#L2:entry()");
        let certain = report(&index, true, "");
        assert_eq!(certain["file_total"], 2);
        assert_eq!(certain["symbols"][1]["name"], "inferred()");
        let plain = report(&index.without_tests(), false, "tested");
        assert_eq!(plain["files"], json!([{"path":"src/tested.php"}]));
        assert_eq!(plain["symbol_total"], 1);
        let symbol = report(&index, false, "entry()");
        assert_eq!(symbol["file_total"], 0);
        assert_eq!(symbol["symbol_total"], 1);
    }
}
