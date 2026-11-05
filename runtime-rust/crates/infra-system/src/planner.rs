use crate::{
    catalog::{ReadBudget, System},
    common::*,
};
use infra_core::Result;
use serde_json::{Value, json as object};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
impl System {
    pub fn plan(
        &mut self,
        query: &str,
        change_id: &str,
        services: &[String],
        contracts: &[String],
        budget: usize,
        depth: usize,
    ) -> Result<Value> {
        text(&object!(query), "task", 2000)?;
        identifier(change_id)?;
        if !(128..=64000).contains(&budget) {
            return Err(fail("Context budget must be 128..64000 characters"));
        }
        let routing = self.locate(query)?;
        let mut seeds = services.to_vec();
        let mut inferred = false;
        if seeds.is_empty()
            && contracts.is_empty()
            && routing["candidates"]
                .as_array()
                .is_some_and(|candidates| candidates.len() == 1)
        {
            seeds.push(string(&routing["candidates"][0]["service"]).into());
            inferred = true;
        }
        let selected = self.impact(&seeds, contracts, depth)?;
        let selected_ids = selected
            .iter()
            .map(|entry| string(&entry["service"]).to_string())
            .collect::<Vec<_>>();
        let mut capsule = object!({"task":query,"services":selected_ids,"sources":[]});
        let mut length = encoded(&capsule)?.chars().count();
        if length > budget {
            return Err(fail(
                "Context budget cannot hold task and service identities",
            ));
        }
        let mut groups = BTreeMap::<String, VecDeque<(std::path::PathBuf, Value)>>::new();
        // What each source is cited for (capability and contract text), so a
        // source the task is about outranks an unrelated one of the same kind.
        let mut cited = BTreeMap::<(String, String), String>::new();
        if !selected.is_empty() {
            for entry in items(&self.config_data["shared_sources"], MAX_ITEMS)? {
                groups
                    .entry("__system__".into())
                    .or_default()
                    .push_back((self.root.clone(), entry.clone()));
            }
        }
        for selected in &selected {
            let sid = string(&selected["service"]);
            let service = self
                .services
                .get(sid)
                .ok_or_else(|| fail("Unknown selected service"))?;
            let Some(manifest) = &service.manifest else {
                continue;
            };
            let mut declared = BTreeMap::new();
            for entry in items(&manifest["sources"], MAX_ITEMS)? {
                declared.insert(string(&entry["path"]).to_string(), entry.clone());
            }
            for capability in items(&manifest["capabilities"], MAX_ITEMS)? {
                let about = format!(
                    "{} {} {}",
                    string(&capability["id"]),
                    string(&capability["description"]),
                    texts(&capability["keywords"]).join(" ")
                );
                for path in texts(&capability["sources"]) {
                    let text = cited.entry((sid.to_string(), path.clone())).or_default();
                    text.push(' ');
                    text.push_str(&about);
                    declared.entry(path.clone()).or_insert(
                        object!({"path":path,"kind":if is_memory(&path){"memory"}else{"code"}}),
                    );
                }
            }
            for contract in items(&manifest["provides"], MAX_ITEMS)? {
                for path in texts(&contract["sources"]) {
                    let text = cited.entry((sid.to_string(), path.clone())).or_default();
                    text.push(' ');
                    text.push_str(string(&contract["id"]));
                    declared.entry(path.clone()).or_insert(
                        object!({"path":path,"kind":if is_memory(&path){"memory"}else{"contract"}}),
                    );
                }
            }
            for entry in declared.into_values() {
                groups
                    .entry(sid.into())
                    .or_default()
                    .push_back((service.root.clone(), entry));
            }
        }
        fn priority(value: &Value) -> usize {
            ["policy", "contract", "spec", "memory", "code", "test"]
                .iter()
                .position(|kind| *kind == string(&value["kind"]))
                .unwrap_or(6)
        }
        // Policies lead; then sources sharing the most task terms through their
        // path or what cites them; then kind and path as before.
        let task_terms = crate::catalog::terms(query);
        for (sid, group) in groups.iter_mut() {
            let mut entries = group
                .drain(..)
                .map(|(root, entry)| {
                    let path = string(&entry["path"]).to_string();
                    let about = format!(
                        "{} {}",
                        path.replace(['/', '.', '-', '_'], " "),
                        cited
                            .get(&(sid.clone(), path.clone()))
                            .map_or("", String::as_str)
                    );
                    let relevance = crate::catalog::terms(&about)
                        .intersection(&task_terms)
                        .count();
                    (root, entry, relevance)
                })
                .collect::<Vec<_>>();
            entries.sort_by_key(|(_, entry, relevance)| {
                (
                    usize::from(entry["kind"] != "policy"),
                    std::cmp::Reverse(*relevance),
                    priority(entry),
                    string(&entry["path"]).to_string(),
                )
            });
            group.extend(entries.into_iter().map(|(root, entry, _)| (root, entry)));
        }
        let mut queue = groups.keys().cloned().collect::<VecDeque<_>>();
        let mut omissions = vec![];
        let mut reads = ReadBudget::default();
        let mut exhausted = false;
        while let Some(sid) = queue.pop_front() {
            let Some((root, entry)) = groups.get_mut(&sid).and_then(VecDeque::pop_front) else {
                continue;
            };
            if groups.get(&sid).is_some_and(|group| !group.is_empty()) {
                queue.push_back(sid.clone());
            }
            let path = string(&entry["path"]);
            let separator = usize::from(
                capsule["sources"]
                    .as_array()
                    .is_some_and(|sources| !sources.is_empty()),
            );
            let floor=encoded(&object!({"service":sid,"path":path,"kind":entry["kind"],"sha256":"0".repeat(64),"trust":"source_evidence","excerpt":"","truncated":true}))?.chars().count();
            let room = budget.saturating_sub(length + separator + floor);
            if room < 1 {
                omissions.push(object!({"service":sid,"path":path,"reason":"budget"}));
                continue;
            }
            if exhausted {
                omissions.push(object!({"service":sid,"path":path,"reason":"read_limit"}));
                continue;
            }
            let content =
                match self.source_content(&sid, &root, &entry, Some(&mut reads), Some(room)) {
                    Ok(Some(content)) => content,
                    Ok(None) => {
                        omissions.push(object!({"service":sid,"path":path,"reason":"ineligible"}));
                        continue;
                    }
                    Err(error)
                        if matches!(error.code.as_deref(), Some("budget" | "read_limit")) =>
                    {
                        let reason = error.code.as_deref().unwrap_or("read_limit");
                        if reason == "read_limit" {
                            exhausted = true;
                            self.warn("__system__", "source_read_limit", None);
                        }
                        omissions.push(object!({"service":sid,"path":path,"reason":reason}));
                        continue;
                    }
                    Err(error) => return Err(error),
                };
            let added = separator + encoded(&content)?.chars().count();
            if length + added <= budget {
                capsule["sources"]
                    .as_array_mut()
                    .ok_or_else(|| fail("Capsule sources lost"))?
                    .push(content);
                length += added;
            } else {
                omissions.push(object!({"service":sid,"path":path,"reason":"budget"}));
            }
        }
        let mut steps = vec![
            object!({"id":"scope","depends_on":[],"goal":"Confirm affected services and missing relationships"}),
            object!({"id":"contracts","depends_on":["scope"],"goal":"Agree invariants, changed contracts, compatibility and delivery order"}),
        ];
        let mut previous = "contracts".to_string();
        let mut ordered = selected.clone();
        ordered.sort_by_key(|entry| {
            (
                texts(&entry["path"]).len(),
                string(&entry["service"]).to_string(),
            )
        });
        for entry in ordered {
            let sid = string(&entry["service"]);
            let step = format!("service-{sid}");
            steps.push(object!({"id":step,"service":sid,"depends_on":[previous],"task_reference":{"external_id":format!("{change_id}/{sid}"),"created":false},"goal":"Inspect local policy and sources, prepare scoped changes and local validation","acceptance":["Follow the service's policy and source contracts","Report local checks run and checks not run","Do not infer deployment order from this investigation order"]}));
            previous = step;
        }
        steps.extend([object!({"id":"verify","depends_on":[previous],"goal":"Check local results, producer/consumer compatibility and the end-to-end scenario"}),object!({"id":"knowledge","depends_on":["verify"],"goal":"Update source catalogs and reusable local/system knowledge through their owning runtime"})]);
        let mut routing = routing;
        routing["selection_provenance"] = if inferred { "inferred" } else { "explicit" }.into();
        let heads = selected_ids
            .iter()
            .map(|sid| {
                (
                    sid.clone(),
                    object!(
                        self.services
                            .get(sid)
                            .and_then(|service| service.head.clone())
                    ),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        Ok(
            object!({"schema_version":1,"kind":"ai-system-plan","change_id":change_id,"system":self.config_data["name"],"status":if selected.is_empty(){"needs_selection"}else{"needs_review"},"executed":false,"routing":routing,"impact":selected,
 "relationships":self.edges.iter().filter(|edge|selected_ids.iter().any(|sid|edge["provider"]==*sid)&&selected_ids.iter().any(|sid|edge["consumer"]==*sid)).cloned().collect::<Vec<_>>(),
 "context":capsule,"context_chars":length,"context_budget_chars":budget,"memory_layout":{"system":{"brain":"project-brain","bank":"memory-bank"},"services":selected_ids.iter().map(|sid|object!({"service":sid,"brain":"project-brain","bank":"memory-bank","tasks_created":false})).collect::<Vec<_>>()},"omitted_sources":omissions,"warnings":self.warnings,"steps":steps,
 "snapshot":{"files":self.snapshot.iter().map(|((sid,path),digest)|object!({"service":sid,"path":path,"sha256":digest})).collect::<Vec<_>>(),"missing":self.missing.iter().map(|(sid,path)|object!({"service":sid,"path":path})).collect::<Vec<_>>(),"heads":heads}}),
        )
    }
    pub fn verify(&mut self, plan: &Value) -> Result<Value> {
        if plan["kind"] != "ai-system-plan" || plan["schema_version"] != 1 {
            return Err(fail("Not an ai-system plan"));
        }
        let snapshot = &plan["snapshot"];
        fields(snapshot, &["files", "missing", "heads"], &[])?;
        let heads = snapshot["heads"]
            .as_object()
            .filter(|heads| heads.len() <= MAX_SERVICES)
            .ok_or_else(|| fail("Invalid snapshot heads"))?;
        let mut changed = vec![];
        let mut expected = BTreeMap::new();
        for entry in items(&snapshot["files"], MAX_SERVICES * MAX_ITEMS * 3 + MAX_ITEMS)? {
            fields(entry, &["service", "path", "sha256"], &[])?;
            let sid = string(&entry["service"]);
            let path = relative(string(&entry["path"]))?;
            let hash = string(&entry["sha256"]);
            if !hex(hash, 64)
                || expected
                    .insert((sid.to_string(), path.clone()), hash.to_string())
                    .is_some()
            {
                return Err(fail("Invalid or duplicate snapshot source"));
            }
            let root = if sid == "__system__" {
                Some(self.root.clone())
            } else {
                self.services
                    .get(sid)
                    .filter(|service| service.access == "available")
                    .map(|service| service.root.clone())
            };
            let Some(root) = root else {
                changed.push(object!({"service":sid,"path":path,"reason":"unavailable"}));
                continue;
            };
            match read_file(&root, &path, MAX_BYTES) {
                Ok(raw) if digest(&raw) == hash => {}
                Ok(_) => {
                    changed.push(object!({"service":sid,"path":path,"reason":"content_changed"}))
                }
                Err(_) => changed.push(object!({"service":sid,"path":path,"reason":"unavailable"})),
            }
        }
        for ((sid, path), hash) in &self.snapshot {
            if expected.get(&(sid.clone(), path.clone())) != Some(hash)
                && !changed
                    .iter()
                    .any(|change| change["service"] == *sid && change["path"] == *path)
            {
                changed.push(object!({"service":sid,"path":path,"reason":"catalog_changed"}));
            }
        }
        for entry in items(
            &snapshot["missing"],
            MAX_SERVICES * MAX_ITEMS * 3 + MAX_ITEMS,
        )? {
            fields(entry, &["service", "path"], &[])?;
            let sid = string(&entry["service"]);
            let path = relative(string(&entry["path"]))?;
            let root = if sid == "__system__" {
                Some(&self.root)
            } else {
                self.services
                    .get(sid)
                    .filter(|service| service.access == "available")
                    .map(|service| &service.root)
            };
            if root.is_some_and(|root| read_file(root, &path, MAX_BYTES).is_ok()) {
                changed.push(object!({"service":sid,"path":path,"reason":"source_appeared"}));
            }
        }
        for (sid, head) in heads {
            if !self.services.contains_key(sid)
                || (!head.is_null() && !(40..=64).contains(&string(head).len()))
                || (!head.is_null()
                    && !string(head)
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
            {
                return Err(fail("Invalid snapshot service or commit"));
            }
            if object!(self.services[sid].head) != *head {
                changed.push(object!({"service":sid,"reason":"head_changed"}));
            }
        }
        let context = &plan["context"];
        fields(context, &["task", "services", "sources"], &[])?;
        for entry in items(&context["sources"], MAX_SERVICES * MAX_ITEMS)? {
            if !entry.is_object() {
                return Err(fail("Invalid saved context source"));
            }
            let sid = string(&entry["service"]);
            let path = relative(string(&entry["path"]))?;
            if !expected.contains_key(&(sid.to_string(), path.clone())) {
                return Err(fail("Context source has no snapshot fingerprint"));
            }
            if entry["kind"] == "memory" || is_memory(&path) {
                let root = if sid == "__system__" {
                    Some(self.root.clone())
                } else {
                    self.services
                        .get(sid)
                        .filter(|service| service.access == "available")
                        .map(|service| service.root.clone())
                };
                if let Some(root) = root
                    && self
                        .source_content(
                            sid,
                            &root,
                            &object!({"path":path,"kind":"memory"}),
                            None,
                            None,
                        )?
                        .is_none()
                {
                    changed.push(object!({"service":sid,"path":path,"reason":"memory_ineligible"}));
                }
            }
        }
        Ok(
            object!({"fresh":changed.is_empty(),"changed":changed,"warnings":self.warnings,"meaning":"Source currency only; this does not approve a plan or verify its claims"}),
        )
    }
    pub fn selected_plan(&mut self, plan: &Value, current: bool) -> Result<Vec<String>> {
        self.verify(plan)?;
        identifier(string(&plan["change_id"]))?;
        if plan["status"] != "needs_review" || plan["executed"] != false {
            return Err(fail("Execution requires selected unexecuted plan"));
        }
        text(&plan["context"]["task"], "task", 2000)?;
        let selected = texts(&plan["context"]["services"]);
        if selected.is_empty() || selected.len() > MAX_SERVICES || !unique(&selected) {
            return Err(fail("Execution requires distinct selected services"));
        }
        for sid in &selected {
            identifier(sid)?;
            if !self.services.contains_key(sid)
                || (current && self.services[sid].access != "available")
            {
                return Err(fail("Selected service unavailable"));
            }
        }
        let required = std::iter::once((
            "__system__".to_string(),
            self.config
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_string(),
        ))
        .chain(
            self.services
                .iter()
                .filter(|(_, service)| service.access == "available")
                .map(|(sid, service)| (sid.clone(), service.manifest_path.clone())),
        )
        .collect::<BTreeSet<_>>();
        let fingerprinted = plan["snapshot"]["files"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|entry| {
                (
                    string(&entry["service"]).to_string(),
                    string(&entry["path"]).to_string(),
                )
            })
            .collect::<BTreeSet<_>>();
        if (current && !required.is_subset(&fingerprinted))
            || plan["snapshot"]["heads"]
                .as_object()
                .map(|heads| heads.keys().cloned().collect::<BTreeSet<_>>())
                != Some(selected.iter().cloned().collect())
        {
            return Err(fail("Incomplete plan provenance"));
        }
        let relationships = self
            .edges
            .iter()
            .filter(|edge| {
                selected.iter().any(|sid| edge["provider"] == *sid)
                    && selected.iter().any(|sid| edge["consumer"] == *sid)
            })
            .cloned()
            .collect::<Vec<_>>();
        if current && plan["relationships"] != object!(relationships) {
            return Err(fail(
                "The services' declared contracts changed after this plan was prepared; prepare a new plan",
            ));
        }
        let budget = plan["context_budget_chars"]
            .as_u64()
            .filter(|budget| (128..=64000).contains(budget))
            .ok_or_else(|| fail("Invalid saved context budget"))?;
        if encoded(&plan["context"])?.chars().count() > budget as usize {
            return Err(fail("Saved context exceeds original budget"));
        }
        Ok(selected)
    }
    pub fn refresh_checkpoint(
        &mut self,
        plan: &Value,
        accepted: &BTreeSet<(String, String)>,
    ) -> Result<Value> {
        let mut updated = plan.clone();
        let mut hashes = BTreeMap::new();
        for entry in updated["snapshot"]["files"]
            .as_array_mut()
            .ok_or_else(|| fail("Invalid snapshot files"))?
        {
            let sid = string(&entry["service"]).to_string();
            let path = relative(string(&entry["path"]))?;
            let root = if sid == "__system__" {
                self.root.clone()
            } else {
                self.services
                    .get(&sid)
                    .ok_or_else(|| fail("Snapshot unknown service"))?
                    .root
                    .clone()
            };
            let hash = digest(&read_file(&root, &path, MAX_BYTES)?);
            if entry["sha256"] != hash && !accepted.contains(&(sid.clone(), path.clone())) {
                return Err(fail("Source changed during canonical context refresh"));
            }
            entry["sha256"] = hash.clone().into();
            hashes.insert((sid, path), hash);
        }
        // A worker may grow a file the plan quotes. Its new excerpt may take
        // the room the approved context left free and is cut beyond that, so
        // an accepted edit never stops a run that fit when it was approved.
        let budget = updated["context_budget_chars"].as_u64().unwrap_or(0) as usize;
        let mut room = budget.saturating_sub(encoded(&plan["context"])?.chars().count());
        let mut sources = vec![];
        for entry in items(&updated["context"]["sources"], MAX_SERVICES * MAX_ITEMS)? {
            let before = encoded(entry)?.chars().count();
            let sid = string(&entry["service"]);
            let path = string(&entry["path"]);
            let root = if sid == "__system__" {
                self.root.clone()
            } else {
                self.services
                    .get(sid)
                    .ok_or_else(|| fail("Context unknown service"))?
                    .root
                    .clone()
            };
            let content = self
                .source_content(
                    sid,
                    &root,
                    &object!({"path":path,"kind":entry["kind"]}),
                    None,
                    None,
                )?
                .ok_or_else(|| fail("Delivered source no longer eligible; prepare new plan"))?;
            if hashes
                .get(&(sid.to_string(), path.to_string()))
                .map(String::as_str)
                != Some(string(&content["sha256"]))
            {
                return Err(fail("Source changed while building canonical excerpts"));
            }
            let content = fit_excerpt(content, before + room)?;
            room = (room + before).saturating_sub(encoded(&content)?.chars().count());
            sources.push(content);
        }
        updated["context"]["sources"] = object!(sources);
        let size = encoded(&updated["context"])?.chars().count();
        updated["context_chars"] = object!(size);
        if size > budget {
            return Err(fail("Updated context exceeds original budget"));
        }
        if System::new(&self.config, &self.allow_roots)?.verify(&updated)?["fresh"] != true {
            return Err(fail("Sources changed during checkpoint refresh"));
        }
        Ok(updated)
    }
}
/// Keeps the longest start of a source excerpt whose entry fits `limit`
/// characters in canonical form, marking it truncated. Escaped characters
/// encode longer, so the cut is searched rather than counted.
fn fit_excerpt(mut content: Value, limit: usize) -> Result<Value> {
    if encoded(&content)?.chars().count() <= limit {
        return Ok(content);
    }
    let excerpt = string(&content["excerpt"]).chars().collect::<Vec<_>>();
    content["truncated"] = true.into();
    let fits = |content: &mut Value, keep: usize| -> Result<bool> {
        content["excerpt"] = excerpt[..keep].iter().collect::<String>().into();
        Ok(encoded(content)?.chars().count() <= limit)
    };
    // The whole excerpt did not fit, so at least one character goes.
    let (mut low, mut high) = (0, excerpt.len().saturating_sub(1));
    while low < high {
        let middle = (low + high).div_ceil(2);
        if fits(&mut content, middle)? {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    if !fits(&mut content, low)? {
        return Err(fail("Updated context exceeds original budget"));
    }
    Ok(content)
}
