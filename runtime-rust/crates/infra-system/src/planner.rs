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
            let mut declared = BTreeSet::new();
            for entry in items(&self.config_data["shared_sources"], MAX_ITEMS)? {
                declared.insert(string(&entry["path"]).to_string());
                groups
                    .entry("__system__".into())
                    .or_default()
                    .push_back((self.root.clone(), entry.clone()));
            }
            // The system's memory and each participant's below are offered to
            // the context every agent shares; the reader admits current chunks
            // and marks those no person reviewed.
            for path in memory_chunks(&self.root).unwrap_or_default() {
                if !declared.contains(&path) {
                    groups.entry("__system__".into()).or_default().push_back((
                        self.root.clone(),
                        object!({"path":path,"kind":"memory","discovered":true}),
                    ));
                }
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
            for path in memory_chunks(&service.root).unwrap_or_default() {
                declared
                    .entry(path.clone())
                    .or_insert(object!({"path":path,"kind":"memory","discovered":true}));
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
        // Memory found in a bank is counted per service when it is not shared,
        // so a large bank does not fill the plan with one line per chunk.
        let mut unshared = BTreeMap::<String, BTreeMap<String, usize>>::new();
        let mut omit = |sid: &str, entry: &Value, reason: &str| {
            if entry["discovered"] == true {
                *unshared
                    .entry(sid.to_string())
                    .or_default()
                    .entry(reason.to_string())
                    .or_default() += 1;
            } else {
                omissions.push(object!({"service":sid,"path":entry["path"],"reason":reason}));
            }
        };
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
                omit(&sid, &entry, "budget");
                continue;
            }
            if exhausted {
                omit(&sid, &entry, "read_limit");
                continue;
            }
            let content =
                match self.source_content(&sid, &root, &entry, Some(&mut reads), Some(room)) {
                    Ok(Some(content)) => content,
                    Ok(None) => {
                        omit(&sid, &entry, "ineligible");
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
                        omit(&sid, &entry, reason);
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
                omit(&sid, &entry, "budget");
            }
        }
        for (sid, causes) in unshared {
            let count = causes.values().sum::<usize>();
            self.warnings.push(
                object!({"service":sid,"reason":"memory_not_shared","count":count,"causes":causes}),
            );
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
        let program_checks = if selected_ids.is_empty() {
            Vec::new()
        } else {
            self.detect_verify_program_checks(&selected_ids)
        };
        let mut verify_step = object!({"id":"verify","depends_on":[previous],"goal":"Check local results, producer/consumer compatibility and the end-to-end scenario"});
        if !program_checks.is_empty() {
            verify_step["program_checks"] = Value::Array(program_checks);
            verify_step["require_llm_review"] = object!(true);
        }
        steps.extend([
            verify_step,
            object!({"id":"knowledge","depends_on":["verify"],"goal":"Update source catalogs and reusable local/system knowledge through their owning runtime"}),
        ]);
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
        let taken_ns = now_unix_ns();
        let file_stamps = self
            .snapshot
            .keys()
            .filter_map(|(sid, path)| {
                let root = if sid == "__system__" {
                    &self.root
                } else {
                    &self.services.get(sid)?.root
                };
                let (size, mtime_ns) = file_stamp(root, path)?;
                Some(object!({
                    "service": sid,
                    "path": path,
                    "size": size,
                    "mtime_ns": mtime_ns.to_string(),
                    "taken_ns": taken_ns.to_string(),
                }))
            })
            .collect::<Vec<_>>();
        Ok(
            object!({"schema_version":1,"kind":"ai-system-plan","change_id":change_id,"system":self.config_data["name"],"status":if selected.is_empty(){"needs_selection"}else{"needs_review"},"executed":false,"routing":routing,"impact":selected,
 "relationships":self.edges.iter().filter(|edge|selected_ids.iter().any(|sid|edge["provider"]==*sid)&&selected_ids.iter().any(|sid|edge["consumer"]==*sid)).cloned().collect::<Vec<_>>(),
 "context":capsule,"context_chars":length,"context_budget_chars":budget,"memory_layout":{"system":{"brain":"project-brain","bank":"memory-bank"},"services":selected_ids.iter().map(|sid|object!({"service":sid,"brain":"project-brain","bank":"memory-bank","tasks_created":false})).collect::<Vec<_>>()},"omitted_sources":omissions,"warnings":self.warnings,"steps":steps,
 "snapshot":{"files":self.snapshot.iter().map(|((sid,path),digest)|object!({"service":sid,"path":path,"sha256":digest})).collect::<Vec<_>>(),"missing":self.missing.iter().map(|(sid,path)|object!({"service":sid,"path":path})).collect::<Vec<_>>(),"heads":heads,"file_stamps":file_stamps}}),
        )
    }
    pub fn verify(&mut self, plan: &Value) -> Result<Value> {
        if plan["kind"] != "ai-system-plan" || plan["schema_version"] != 1 {
            return Err(fail("Not an ai-system plan"));
        }
        let snapshot = &plan["snapshot"];
        fields(snapshot, &["files", "missing", "heads"], &["file_stamps"])?;
        let stamps = parse_file_stamps(&snapshot["file_stamps"]);
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
            let stamp_match = stamps.get(&(sid.to_string(), path.clone())).is_some_and(
                |&(size, mtime_ns, taken_ns)| {
                    mtime_ns <= taken_ns.saturating_sub(RACY_NS)
                        && file_stamp(&root, &path) == Some((size, mtime_ns))
                },
            );
            if stamp_match {
                continue;
            }
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
        let mut dropped = vec![];
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
            let Some(content) = self.source_content(
                sid,
                &root,
                &object!({"path":path,"kind":entry["kind"]}),
                None,
                None,
            )?
            else {
                // Memory that stopped holding (a worker changed what it cites,
                // or its review fell due) leaves the shared context; the
                // sources it described are still there.
                if entry["kind"] == "memory" {
                    dropped.push(
                        object!({"service":sid,"path":path,"reason":"memory_no_longer_holds"}),
                    );
                    room += before + 1;
                    continue;
                }
                return Err(fail(
                    "Delivered source no longer eligible; prepare new plan",
                ));
            };
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
        if !dropped.is_empty() {
            let omitted = updated["omitted_sources"]
                .as_array_mut()
                .ok_or_else(|| fail("Invalid omitted sources"))?;
            for entry in dropped {
                if !omitted.contains(&entry) {
                    omitted.push(entry);
                }
            }
        }
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

    fn detect_verify_program_checks(&self, selected_ids: &[String]) -> Vec<Value> {
        const MAX_AUTO_CHECKS: usize = 16;
        let mut checks = Vec::new();
        let mut seen_roots = BTreeSet::new();

        let candidates = selected_ids
            .iter()
            .filter_map(|sid| {
                let svc = self.services.get(sid)?;
                (svc.access == "available" && svc.manifest.is_some())
                    .then_some((sid.as_str(), svc.root.as_path()))
            })
            .chain(std::iter::once(("__system__", self.root.as_path())));

        for (sid, root) in candidates {
            if checks.len() >= MAX_AUTO_CHECKS {
                break;
            }
            let canon = infra_core::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
            if !seen_roots.insert(canon) {
                continue;
            }
            collect_root_program_checks(sid, root, MAX_AUTO_CHECKS - checks.len(), &mut checks);
        }
        checks
    }
}

fn is_regular_file(root: &std::path::Path, rel: &str) -> bool {
    std::fs::symlink_metadata(root.join(rel))
        .is_ok_and(|meta| !meta.file_type().is_symlink() && meta.is_file())
}

fn is_real_dir(root: &std::path::Path, rel: &str) -> bool {
    std::fs::symlink_metadata(root.join(rel))
        .is_ok_and(|meta| !meta.file_type().is_symlink() && meta.is_dir())
}

fn is_safe_package_script(cmd: &str) -> bool {
    let trimmed = cmd.trim();
    if trimmed.is_empty() || trimmed.len() > 512 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("no test specified") {
        return false;
    }
    if [
        "--fix", "--write", " -w", "rm ", "curl ", "wget ", "chmod ", "chown ",
    ]
    .iter()
    .any(|bad| lower.contains(bad))
    {
        return false;
    }
    true
}

fn push_check(
    out: &mut Vec<Value>,
    limit: usize,
    name: String,
    argv: &[&str],
    cwd_service: &str,
    timeout_seconds: Option<u64>,
) {
    if out.len() >= limit {
        return;
    }
    let mut entry = object!({
        "name": name,
        "argv": argv,
        "cwd_service": cwd_service,
    });
    if let Some(secs) = timeout_seconds {
        entry["timeout_seconds"] = object!(secs);
    }
    out.push(entry);
}

fn collect_root_program_checks(
    sid: &str,
    root: &std::path::Path,
    remaining: usize,
    out: &mut Vec<Value>,
) {
    if remaining == 0 {
        return;
    }
    let start_len = out.len();
    let max_len = start_len + remaining;

    if is_regular_file(root, "composer.json") {
        if is_regular_file(root, "vendor/bin/phpunit")
            && (is_regular_file(root, "phpunit.xml") || is_regular_file(root, "phpunit.xml.dist"))
        {
            push_check(
                out,
                max_len,
                format!("{sid}:phpunit"),
                &["vendor/bin/phpunit"],
                sid,
                Some(120),
            );
        } else if is_regular_file(root, "vendor/bin/pest") {
            push_check(
                out,
                max_len,
                format!("{sid}:pest"),
                &["vendor/bin/pest"],
                sid,
                Some(120),
            );
        }
        if is_regular_file(root, "vendor/bin/phpstan")
            && (is_regular_file(root, "phpstan.neon") || is_regular_file(root, "phpstan.neon.dist"))
        {
            push_check(
                out,
                max_len,
                format!("{sid}:phpstan"),
                &["vendor/bin/phpstan", "analyse", "--no-progress"],
                sid,
                Some(120),
            );
        }
        if read_file(root, "composer.json", MAX_SOURCE_BYTES)
            .ok()
            .and_then(|raw| parse_json(&raw, MAX_SOURCE_BYTES).ok())
            .is_some_and(|doc| doc.is_object())
        {
            push_check(
                out,
                max_len,
                format!("{sid}:composer-validate"),
                &[
                    "composer",
                    "validate",
                    "--no-check-all",
                    "--no-check-publish",
                    "--no-interaction",
                ],
                sid,
                Some(60),
            );
        }
    }

    if is_regular_file(root, "Cargo.toml") {
        push_check(
            out,
            max_len,
            format!("{sid}:cargo-test"),
            &["cargo", "test", "--offline"],
            sid,
            Some(120),
        );
    }

    if is_regular_file(root, "package.json")
        && let Ok(raw) = read_file(root, "package.json", MAX_SOURCE_BYTES)
        && let Ok(doc) = parse_json(&raw, MAX_SOURCE_BYTES)
        && let Some(scripts) = doc.get("scripts").and_then(Value::as_object)
    {
        let pkg_mgr = if is_regular_file(root, "pnpm-lock.yaml") {
            "pnpm"
        } else if is_regular_file(root, "yarn.lock") {
            "yarn"
        } else {
            "npm"
        };
        for script in ["typecheck", "lint", "test", "check"] {
            if let Some(cmd) = scripts.get(script).and_then(Value::as_str)
                && is_safe_package_script(cmd)
            {
                let argv: [&str; 4] = if pkg_mgr == "yarn" {
                    ["yarn", "run", "--silent", script]
                } else {
                    [pkg_mgr, "run", script, "--silent"]
                };
                push_check(
                    out,
                    max_len,
                    format!("{sid}:{pkg_mgr}-{script}"),
                    &argv,
                    sid,
                    Some(120),
                );
            }
        }
    }

    if (is_regular_file(root, "pyproject.toml") || is_regular_file(root, "pytest.ini"))
        && is_real_dir(root, "tests")
    {
        push_check(
            out,
            max_len,
            format!("{sid}:pytest"),
            &["pytest", "-q"],
            sid,
            Some(120),
        );
    }

    if is_regular_file(root, "go.mod") {
        push_check(
            out,
            max_len,
            format!("{sid}:go-test"),
            &["go", "test", "./..."],
            sid,
            Some(120),
        );
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

/// Nanosecond racy-stat threshold matching `infra_skillcache::fingerprint::RACY_NS`:
/// files modified within 2s before the snapshot was taken are always re-read and hashed.
const RACY_NS: i128 = 2_000_000_000;

fn time_to_unix_ns(time: std::time::SystemTime) -> Option<i128> {
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(delta) => i128::try_from(delta.as_nanos()).ok(),
        Err(err) => i128::try_from(err.duration().as_nanos())
            .ok()
            .and_then(|ns| ns.checked_neg()),
    }
}

fn now_unix_ns() -> i128 {
    time_to_unix_ns(std::time::SystemTime::now()).unwrap_or_default()
}

fn file_stamp(root: &std::path::Path, path: &str) -> Option<(u64, i128)> {
    let meta = std::fs::symlink_metadata(root.join(path)).ok()?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return None;
    }
    let mtime = time_to_unix_ns(meta.modified().ok()?)?;
    Some((meta.len(), mtime))
}

fn parse_file_stamps(value: &Value) -> BTreeMap<(String, String), (u64, i128, i128)> {
    let mut stamps = BTreeMap::new();
    for entry in value.as_array().into_iter().flatten() {
        let Some(sid) = entry["service"].as_str() else {
            continue;
        };
        let Some(path) = entry["path"].as_str() else {
            continue;
        };
        let Some(size) = entry["size"].as_u64() else {
            continue;
        };
        let Some(mtime_ns) = entry["mtime_ns"].as_str().and_then(|s| s.parse().ok()) else {
            continue;
        };
        let Some(taken_ns) = entry["taken_ns"].as_str().and_then(|s| s.parse().ok()) else {
            continue;
        };
        stamps.insert(
            (sid.to_string(), path.to_string()),
            (size, mtime_ns, taken_ns),
        );
    }
    stamps
}

#[cfg(test)]
mod racy_verify_tests {
    use super::*;

    #[test]
    fn racy_ns_threshold_guards_recent_edits_and_skips_settled_identical_stats() {
        let dir = std::env::temp_dir().join(format!("infra-sys-racy-{}", uuid::Uuid::new_v4()));
        let path = "src/example.txt";
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join(path), b"hello world\n").unwrap();
        let (size, mtime_ns) = file_stamp(&dir, path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        // Settled file (> RACY_NS older than snapshot) is trusted via stat without hashing.
        let settled_taken = mtime_ns + RACY_NS + 1;
        assert!(mtime_ns <= settled_taken.saturating_sub(RACY_NS));

        // File modified inside RACY_NS window is never skipped.
        let racy_taken = mtime_ns + RACY_NS - 1;
        assert!(!(mtime_ns <= racy_taken.saturating_sub(RACY_NS)));

        let raw_stamps = object!([{
            "service": "__system__",
            "path": path,
            "size": size,
            "mtime_ns": mtime_ns.to_string(),
            "taken_ns": settled_taken.to_string(),
        }]);
        let parsed = parse_file_stamps(&raw_stamps);
        assert_eq!(
            parsed.get(&("__system__".into(), path.into())),
            Some(&(size, mtime_ns, settled_taken))
        );
    }

    #[test]
    fn system_plan_emits_deterministic_program_checks_for_known_stacks_with_llm_review() {
        let dir = std::env::temp_dir().join(format!("infra-sys-checks-{}", uuid::Uuid::new_v4()));
        let sys_dir = dir.join("sys");
        let php_dir = dir.join("php-svc");
        let node_dir = dir.join("node-svc");
        let plain_dir = dir.join("plain-svc");
        std::fs::create_dir_all(&sys_dir).unwrap();
        std::fs::create_dir_all(php_dir.join("vendor/bin")).unwrap();
        std::fs::create_dir_all(&node_dir).unwrap();
        std::fs::create_dir_all(&plain_dir).unwrap();

        std::fs::write(sys_dir.join("AGENTS.md"), "# Policy\n").unwrap();
        std::fs::write(
            sys_dir.join("system.json"),
            serde_json::to_string_pretty(&object!({
                "schema_version": 1,
                "name": "checks-sys",
                "services": [
                    {"id": "orders", "root": "../php-svc", "manifest": "ai-service.json"},
                    {"id": "web", "root": "../node-svc", "manifest": "ai-service.json"},
                    {"id": "plain", "root": "../plain-svc", "manifest": "ai-service.json"}
                ],
                "shared_sources": [{"kind": "policy", "path": "AGENTS.md"}]
            }))
            .unwrap(),
        )
        .unwrap();

        let manifest = |id: &str| {
            serde_json::to_string_pretty(&object!({
                "schema_version": 1,
                "id": id,
                "description": id,
                "owner": "@team",
                "capabilities": [{"id": id, "description": id, "status": "implemented", "sources": []}],
                "provides": [],
                "consumes": [],
                "sources": []
            }))
            .unwrap()
        };
        std::fs::write(php_dir.join("ai-service.json"), manifest("orders")).unwrap();
        std::fs::write(php_dir.join("composer.json"), r#"{"name":"acme/orders"}"#).unwrap();
        std::fs::write(php_dir.join("phpunit.xml.dist"), "<phpunit/>").unwrap();
        std::fs::write(php_dir.join("vendor/bin/phpunit"), "#!/bin/sh\n").unwrap();

        std::fs::write(node_dir.join("ai-service.json"), manifest("web")).unwrap();
        std::fs::write(
            node_dir.join("package.json"),
            r#"{"name":"web","scripts":{"test":"vitest run","lint":"eslint --fix .","typecheck":"tsc --noEmit"}}"#,
        )
        .unwrap();

        std::fs::write(plain_dir.join("ai-service.json"), manifest("plain")).unwrap();

        let mut system =
            System::new(&sys_dir.join("system.json"), std::slice::from_ref(&dir)).unwrap();

        // Plain fixture without project manifests keeps legacy verify step shape.
        let plain_plan = system
            .plan("Plain task", "chg-plain", &["plain".into()], &[], 8000, 4)
            .unwrap();
        let plain_verify = plain_plan["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "verify")
            .unwrap();
        assert!(plain_verify.get("program_checks").is_none());

        // PHP + Node services emit safe deterministic checks and keep require_llm_review: true.
        let stack_plan = system
            .plan(
                "Cross-service change",
                "chg-stacks",
                &["orders".into(), "web".into()],
                &[],
                8000,
                4,
            )
            .unwrap();
        let verify = stack_plan["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "verify")
            .unwrap();
        assert_eq!(verify["require_llm_review"], true);
        let checks = verify["program_checks"].as_array().unwrap();
        let names: Vec<&str> = checks.iter().filter_map(|c| c["name"].as_str()).collect();
        assert!(names.contains(&"orders:phpunit"), "{names:?}");
        assert!(names.contains(&"orders:composer-validate"), "{names:?}");
        assert!(names.contains(&"web:npm-typecheck"), "{names:?}");
        assert!(names.contains(&"web:npm-test"), "{names:?}");
        // Mutating `--fix` script is filtered out.
        assert!(!names.contains(&"web:npm-lint"), "{names:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
