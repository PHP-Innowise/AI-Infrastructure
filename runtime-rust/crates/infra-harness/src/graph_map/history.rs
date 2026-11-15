use super::*;

const LISTED: usize = 12;

impl Index {
    pub(crate) fn mark_history(&self, view: &mut Value, history: &crate::graph_changes::History) {
        let scope = view["scope"].as_str().unwrap_or("");
        let focus = &view["focus"];
        let selected = if focus["symbol"] == true {
            focus["file"].as_str().or(view["file"]["path"].as_str())
        } else {
            focus["path"].as_str().or(view["file"]["path"].as_str())
        };
        let files: HashMap<_, _> = self
            .files
            .iter()
            .enumerate()
            .map(|(id, path)| (path.as_str(), id))
            .collect();
        let mut neighbors = vec![HashSet::new(); files.len()];
        for (from, to, _) in &self.pairs {
            neighbors[*from as usize].insert(*to as usize);
            neighbors[*to as usize].insert(*from as usize);
        }

        let mut edits = vec![0; files.len()];
        let mut together = vec![0; files.len()];
        let selected_id = selected.and_then(|path| files.get(path)).copied();
        for commit in &history.files {
            let touched: Vec<_> = commit
                .iter()
                .filter_map(|path| files.get(path.as_str()).copied())
                .collect();
            let shared = selected_id.is_some_and(|id| touched.contains(&id));
            for id in touched {
                edits[id] += 1;
                together[id] += usize::from(shared);
            }
        }
        let folder = selected.filter(|_| selected_id.is_none()).unwrap_or(scope);
        let in_folder = |path: &str| {
            folder.is_empty()
                || path == folder
                || path
                    .strip_prefix(folder)
                    .is_some_and(|tail| tail.starts_with('/'))
        };
        let mut hot: Vec<_> = self
            .files
            .iter()
            .enumerate()
            .filter(|(id, path)| in_folder(path) && edits[*id] >= 2 && neighbors[*id].len() >= 2)
            .collect();
        hot.sort_by_key(|(id, path)| (std::cmp::Reverse(edits[*id] * neighbors[*id].len()), *path));
        let cochanges = selected_id.map(|selected| {
            let mut peers: Vec<_> = self
                .files
                .iter()
                .enumerate()
                .filter(|(id, _)| {
                    *id != selected && together[*id] >= 2 && !neighbors[selected].contains(id)
                })
                .collect();
            peers.sort_by_key(|(id, path)| (std::cmp::Reverse(together[*id]), *path));
            json!({
                "path": self.files[selected],
                "commits": edits[selected],
                "total": peers.len(),
                "rows": peers.into_iter().take(LISTED).map(|(id, path)| {
                    json!({"path": path, "together": together[id], "commits": edits[id]})
                }).collect::<Vec<_>>(),
            })
        });
        view["git_history"] = json!({
            "state": "ready",
            "commits": history.files.len(),
            "skipped": history.skipped,
            "revision": history.revision,
            "hot_total": hot.len(),
            "hot": hot.into_iter().take(LISTED).map(|(id, path)| {
                json!({"path": path, "commits": edits[id], "neighbors": neighbors[id].len()})
            }).collect::<Vec<_>>(),
            "cochanges": cochanges,
        });
    }
}
