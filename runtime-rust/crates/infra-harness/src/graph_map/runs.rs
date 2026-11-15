//! A run of an agent laid over the map of what it changed: which of the
//! files that use the change the run opened, and which it left unopened.
//!
//! A session records the files a run opened and those it changed. A graph
//! knows which files use the changed ones. A file that uses the change and
//! was never opened is where a break the run did not see is looked for
//! first. Shell evidence is limited to attributable commands, and a read
//! does not establish that the run checked the behavior.
use super::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Bound,
};

type Reads<'a> = (&'a BTreeSet<String>, &'a BTreeMap<String, u64>);
type RunEvidence<'a> = (Reads<'a>, &'a BTreeSet<&'a str>);

/// Unopened files listed at once; the rest are counted.
pub const LISTED: usize = 30;

impl Index {
    /// Says in a picture of what a run changed what the run opened. A box
    /// of a file the run opened says so, and a box of a folder how many
    /// files in it the run opened; so do the two sides of a selected place.
    /// `view["run"]["reach"]` tells of the files that use what the run
    /// changed where the picture is open: how many there are, how many of
    /// them the run opened, and the unopened ones, the most linked first.
    /// A changed file is no user of the change, and a deleted file has no
    /// users in a graph built after it was gone.
    #[cfg(test)]
    pub fn overlay_run(
        &self,
        view: &mut Value,
        changed: &[(String, char)],
        opened: &BTreeSet<String>,
        certain: bool,
    ) {
        self.overlay_run_with_comparison(view, changed, (opened, &BTreeMap::new()), certain, None);
    }

    pub fn overlay_run_with_comparison(
        &self,
        view: &mut Value,
        changed: &[(String, char)],
        (opened, shell_reads): Reads<'_>,
        certain: bool,
        comparison: Option<RunEvidence<'_>>,
    ) {
        let scope = view["scope"].as_str().unwrap_or("").to_owned();
        let within = |path: &str| {
            scope.is_empty()
                || path == scope
                || path
                    .strip_prefix(scope.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
        };
        let ids = self
            .files
            .iter()
            .enumerate()
            .map(|(id, path)| (path.as_str(), id as u32))
            .collect::<HashMap<_, _>>();
        // Every changed file is left out of the users; those of the open
        // folder are the ones whose users are told.
        let all = changed
            .iter()
            .filter(|(_, done)| *done != 'D')
            .filter_map(|(path, _)| ids.get(path.as_str()).copied())
            .collect::<HashSet<_>>();
        let ours = all
            .iter()
            .copied()
            .filter(|id| within(&self.files[*id as usize]))
            .collect::<HashSet<_>>();
        // Each with the changed files it uses, and through how many links.
        let mut users: HashMap<u32, (Flow, BTreeMap<u32, u64>)> = HashMap::new();
        for (from, to, pair) in &self.pairs {
            let weight = pair.weight(certain);
            if weight == 0 || !ours.contains(to) || all.contains(from) {
                continue;
            }
            let (flow, used) = users.entry(*from).or_default();
            flow.links += weight;
            flow.guessed += if certain { 0 } else { pair.guessed };
            *used.entry(*to).or_default() += weight;
        }
        let mut unopened = users
            .iter()
            .map(|(id, told)| (self.files[*id as usize].as_str(), told))
            .filter(|(path, _)| !opened.contains(*path))
            .collect::<Vec<_>>();
        unopened.sort_by(|a, b| b.1.0.links.cmp(&a.1.0.links).then(a.0.cmp(b.0)));
        view["run"]["reach"] = json!({
            "users": users.len(),
            "opened": users.len() - unopened.len(),
            "unopened": unopened.len(),
            "rows": unopened
                .iter()
                .take(LISTED)
                .map(|(path, (flow, used))| {
                    // The changed file it uses most is where its link is
                    // looked into.
                    let most = used
                        .iter()
                        .map(|(id, links)| (self.files[*id as usize].as_str(), *links))
                        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
                        .map_or("", |(path, _)| path);
                    json!({
                        "path": path, "name": name(path),
                        "links": flow.links, "guessed": flow.guessed,
                        "uses": used.len(), "to": most, "test": is_test(path),
                    })
                })
                .collect::<Vec<_>>(),
            "more": unopened.len().saturating_sub(LISTED),
        });
        let mut in_scope = shell_reads.iter().filter(|(path, _)| within(path));
        view["run"]["shell_reads"] = json!({
            "rows": in_scope.by_ref().take(LISTED).map(|(path, id)| json!({"path": path,"event": id})).collect::<Vec<_>>(),
            "more": in_scope.count(),
        });
        if let Some(((followup_opened, followup_shell_reads), followup_edited)) = comparison {
            let mut rows = users
                .iter()
                .map(|(id, (flow, _))| {
                    let path = self.files[*id as usize].as_str();
                    (
                        path,
                        flow.links,
                        opened.contains(path),
                        followup_opened.contains(path),
                        followup_edited.contains(path),
                    )
                })
                .collect::<Vec<_>>();
            let newly_touched = rows
                .iter()
                .filter(|(_, _, first, read, edit)| !first && (*read || *edit))
                .count();
            let gaps = rows
                .iter()
                .filter(|(_, _, first, read, edit)| !first && !read && !edit)
                .count();
            rows.sort_by(|a, b| {
                let order = |first, read, edit| (first, read || edit);
                order(a.2, a.3, a.4)
                    .cmp(&order(b.2, b.3, b.4))
                    .then(b.1.cmp(&a.1))
                    .then(a.0.cmp(b.0))
            });
            view["run"]["comparison"] = json!({
                "newly_touched": newly_touched,
                "gaps": gaps,
                "more": rows.len().saturating_sub(LISTED),
                "rows": rows.into_iter().take(LISTED).map(|(path, _, first, read, edit)| json!({
                    "path": path, "opened": first, "followup_opened": read, "followup_edited": edit,
                    "shell_read": shell_reads.get(path),
                    "followup_shell_read": followup_shell_reads.get(path),
                })).collect::<Vec<_>>(),
            });
        }
        // The boxes: a file by itself, a folder by the files in it. A
        // symbol is no file, and its path is none that a run opened.
        let inside = |folder: &str| {
            let from = format!("{folder}/");
            opened
                .range::<str, _>((Bound::Included(from.as_str()), Bound::Unbounded))
                .take_while(|path| path.starts_with(&from))
                .count()
        };
        let mark = |place: &mut Value| {
            let Some(path) = place["path"].as_str() else {
                return;
            };
            if place["folder"] == true {
                let files = inside(path);
                if files > 0 {
                    place["opened"] = json!(files);
                }
            } else if opened.contains(path) {
                let shell = shell_reads.get(path).copied();
                place["opened"] = json!(true);
                if let Some(shell) = shell {
                    place["shell_read"] = json!(shell);
                }
            }
        };
        for places in ["/nodes", "/focus/uses", "/focus/used_by"] {
            for place in view
                .pointer_mut(places)
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                mark(place);
            }
        }
    }

    /// Attaches live `GraphFocus` metadata (`target_paths`, `target_folders`, `target_nodes`,
    /// and `created_entities`) under `view["graph_focus"]` and `view["run"]["graph_focus"]`,
    /// preserving `view["focus"]` for the user-selected inspection panel.
    pub fn overlay_graph_focus(&self, view: &mut Value, focus: &crate::graph_live::GraphFocus) {
        let focus_json = serde_json::to_value(focus).unwrap_or(Value::Null);
        view["graph_focus"] = focus_json.clone();
        view["run"]["graph_focus"] = focus_json;

        let exact_targets: BTreeSet<&str> = focus
            .target_paths
            .iter()
            .chain(focus.target_folders.iter())
            .chain(focus.target_nodes.iter())
            .map(String::as_str)
            .collect();

        let is_subtree_hit = |folder: &str, candidate: &str| {
            candidate == folder
                || candidate
                    .strip_prefix(folder)
                    .is_some_and(|rest| rest.starts_with('/'))
        };

        for place in view
            .pointer_mut("/nodes")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            let Some(path) = place["path"].as_str().map(str::to_owned) else {
                continue;
            };
            let is_folder = place["folder"] == true;
            let targeted = exact_targets.contains(path.as_str())
                || (is_folder
                    && focus
                        .target_paths
                        .iter()
                        .chain(focus.target_folders.iter())
                        .any(|cand| is_subtree_hit(&path, cand)))
                || focus
                    .created_entities
                    .iter()
                    .any(|e| e.path == path || (is_folder && is_subtree_hit(&path, &e.path)));
            if targeted {
                place["focus_target"] = json!(true);
            }
            let created_here = focus
                .created_entities
                .iter()
                .filter(|e| e.path == path || (is_folder && is_subtree_hit(&path, &e.path)))
                .count();
            if created_here > 0 {
                place["created_entities_count"] = json!(created_here);
            }
        }
    }
}

#[cfg(test)]
mod tests;
