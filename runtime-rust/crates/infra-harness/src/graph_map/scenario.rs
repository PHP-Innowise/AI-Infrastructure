use super::*;

const PAGE: usize = 40;

impl Index {
    pub(crate) fn scenario(
        &self,
        project: &str,
        origin: &str,
        certain: bool,
        filter: &str,
        skip: usize,
    ) -> Value {
        let mut view = json!({
            "mode": "scenario", "certain": certain, "totals": self.totals(),
            "crumbs": Self::crumbs(project, ""), "nodes": [], "edges": [],
            "scenario": {"origin": origin, "missing": true},
        });
        let Some(reach @ (Reach::File(_) | Reach::Symbol(_))) = self.reach(origin) else {
            return view;
        };
        let start = match reach {
            Reach::File(file) | Reach::Symbol(file) => file,
            Reach::Folder(_) => unreachable!(),
        };
        let net = Net::new(self, matches!(reach, Reach::Symbol(_)), certain);
        let mut blocked = vec![0; net.count()];
        blocked[start as usize] = 1;
        let mut distance = net.far(&[start], &blocked, true);
        distance[start as usize] = 0;
        let names = Names::new(self, &[], &|_| None);
        let mut reached: Vec<_> = distance
            .iter()
            .enumerate()
            .filter(|(_, steps)| **steps != u32::MAX)
            .map(|(at, _)| at as u32)
            .collect();
        let file = |at: u32| {
            &self.files[if net.symbols {
                self.symbols[at as usize].file as usize
            } else {
                at as usize
            }]
        };
        reached.sort_unstable_by_key(|&at| (distance[at as usize], file(at), at));
        let shown = &reached[..reached.len().min(SHOWN)];
        let keys: Vec<_> = shown.iter().map(|&at| net.key(&names, at)).collect();
        let paths: Vec<_> = keys.iter().map(String::as_str).collect();
        let ids: HashMap<_, _> = shown.iter().enumerate().map(|(i, &at)| (at, i)).collect();
        let nodes: Vec<_> = shown
            .iter()
            .enumerate()
            .map(|(i, &at)| {
                let mut node = net.node(&names, at, &keys[i], &paths);
                node["start"] = json!(at == start);
                node["steps"] = json!(distance[at as usize]);
                node
            })
            .collect();
        let mut edges = Vec::new();
        for (i, &at) in shown.iter().enumerate() {
            for (other, flow) in net.beside(at, true) {
                if let Some(&j) = ids.get(&other) {
                    edges.push((i, j, flow));
                }
            }
        }
        // Keep a shortest-path parent for each box before filling the remaining
        // edge budget. Even on a dense graph, every displayed box stays reachable.
        edges.sort_unstable_by_key(|(from, to, flow)| {
            (
                distance[shown[*from] as usize] + 1 != distance[shown[*to] as usize],
                *to,
                std::cmp::Reverse(flow.links),
                *from,
            )
        });
        let mut parent = HashSet::new();
        let mut rest = Vec::new();
        let mut kept = Vec::new();
        for (from, to, flow) in edges {
            let edge = json!({"from":keys[from], "to":keys[to], "links":flow.links, "guessed":flow.guessed});
            if distance[shown[from] as usize] + 1 == distance[shown[to] as usize]
                && parent.insert(to)
            {
                kept.push(edge);
            } else {
                rest.push((flow.links, edge));
            }
        }
        rest.sort_by_key(|(links, _)| std::cmp::Reverse(*links));
        kept.extend(
            rest.into_iter()
                .take(DRAWN.saturating_sub(kept.len()))
                .map(|(_, edge)| edge),
        );
        let query = filter.trim().to_lowercase();
        let matching: Vec<_> = reached
            .iter()
            .copied()
            .skip(1)
            .filter(|&at| {
                query.is_empty()
                    || net.named(at, &query)
                    || (net.symbols && file(at).to_lowercase().contains(&query))
            })
            .collect();
        let skip = skip.min(matching.len().saturating_sub(1) / PAGE * PAGE);
        let rows: Vec<_> = matching
            .iter()
            .skip(skip)
            .take(PAGE)
            .map(|&at| {
                let key = net.key(&names, at);
                let mut row = if net.symbols {
                    names.told(at, &key)
                } else {
                    json!({"path": key, "name": file(at)})
                };
                row["steps"] = json!(distance[at as usize]);
                row
            })
            .collect();
        view["scenario"] = json!({
            "origin": self.end(origin, reach), "reachable": reached.len() - 1,
            "matching": matching.len(), "skip": skip, "rows": rows,
        });
        view["nodes"] = json!(nodes);
        view["edges"] = json!(kept);
        view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_is_directed_transitive_and_terminates_on_cycles() {
        let node = |id: &str, file: &str| {
            json!({
                "id": id, "source_file": file, "label": format!("{id}()"), "source_location": "L2"
            })
        };
        let edge = |from: &str, to: &str, confidence: &str| json!({"source":from,"target":to,"relation":"calls","confidence":confidence});
        let graph = json!({"nodes":[
            node("entry", "src/Entry.php"), node("service", "src/Service.php"),
            node("db", "src/Db.php"), node("incoming", "src/Caller.php"),
            node("guessed", "src/Guessed.php"), node("other", "src/Entry.php"),
        ], "links":[
            edge("entry", "service", "EXTRACTED"), edge("service", "db", "EXTRACTED"),
            edge("db", "entry", "EXTRACTED"), edge("incoming", "entry", "EXTRACTED"),
            edge("other", "guessed", "EXTRACTED"), edge("entry", "guessed", "INFERRED"),
        ]});
        let index = Index::read(graph.to_string().as_bytes(), Path::new("/work/shop")).unwrap();
        let view = index.scenario("shop", "src/Entry.php#L2:entry()", true, "db", 0);
        assert_eq!(view["scenario"]["reachable"], 2);
        assert_eq!(view["scenario"]["matching"], 1);
        assert_eq!(view["scenario"]["rows"][0]["path"], "src/Db.php#L2:db()");
        assert_eq!(view["scenario"]["rows"][0]["steps"], 2);
        assert_eq!(view["nodes"].as_array().unwrap().len(), 3);
        assert_eq!(view["nodes"][0]["steps"], 0);
        assert_eq!(view["nodes"][0]["start"], true);
        assert_eq!(view["edges"].as_array().unwrap().len(), 3);
        let file = index.scenario("shop", "src/Entry.php", true, "", 0);
        assert_eq!(file["scenario"]["reachable"], 3);
        assert!(index.scenario("shop", "missing", false, "", 0)["scenario"]["missing"] == true);
    }

    #[test]
    fn scenario_lists_dependencies_beyond_the_drawing() {
        let mut nodes = Vec::new();
        let mut links = Vec::new();
        for at in 0..45 {
            nodes.push(
                json!({"id":at, "source_file":format!("f{at:02}.php"), "label":format!("f{at}")}),
            );
            if at > 0 {
                links.push(json!({"source":at - 1, "target":at, "relation":"calls"}));
            }
        }
        let graph = json!({"nodes":nodes,"links":links});
        let index = Index::read(graph.to_string().as_bytes(), Path::new("/work/shop")).unwrap();
        let view = index.scenario("shop", "f00.php", false, "", 40);
        assert_eq!(view["scenario"]["reachable"], 44);
        assert_eq!(view["nodes"].as_array().unwrap().len(), 40);
        assert_eq!(view["edges"].as_array().unwrap().len(), 39);
        assert_eq!(view["scenario"]["rows"][0]["steps"], 41);
        assert_eq!(view["scenario"]["rows"][3]["path"], "f44.php");
    }
}
