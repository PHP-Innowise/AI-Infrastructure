//! Governed context retrieval, final source custody and publication.
use crate::{
    BrainRepository, ErrorKind, Result, RuntimeConfig, RuntimeContext, RuntimeError,
    capsule::{self, Capsule, DeliveryProfile},
    domain::*,
    fs,
    index::{self, DOCUMENT_SELECT, Document, Exclusion, document_from_row},
    query::{self, Candidate, Diagnostics},
    receipts::{self, BaselinePlan, SessionPlan},
    storage::Store,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const TARGET_BUDGET: usize = 8000;
pub const HARD_BUDGET: usize = 12000;
pub const BUDGETS: [(&str, usize); 5] = [
    ("policy", 1200),
    ("handoff", 1500),
    ("durable", 3500),
    ("dynamic", 1500),
    ("evidence", 2000),
];
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RetrievalRequest {
    pub query: String,
    pub task_id: Option<String>,
    pub limit: usize,
    pub host: String,
    pub entry_point: String,
    pub manifest_scope: String,
    pub ephemeral: bool,
    pub query_source: String,
    pub query_fallback: Option<String>,
    pub gate: Option<String>,
    pub paths: Vec<String>,
    pub source_hints: Option<Value>,
    pub session_id: Option<String>,
    pub transcript: Option<PathBuf>,
    pub delivery_profile: DeliveryProfile,
    pub capsule_profile: Option<String>,
    pub refresh_index: bool,
    pub allow_unprovisioned: bool,
    pub automatic_links: bool,
    pub history_text: Option<String>,
    pub phase_seconds: BTreeMap<String, f64>,
    pub expected_branch: Option<String>,
    pub scope: Option<Vec<String>>,
    pub trace_paths: Vec<String>,
    pub provenance_response: bool,
    pub sanitize: bool,
    pub index_warnings: Vec<String>,
    pub source_notices: BTreeMap<String, u64>,
}
impl Default for RetrievalRequest {
    fn default() -> Self {
        Self {
            query: String::new(),
            task_id: None,
            limit: 3,
            host: "cli".into(),
            entry_point: "retrieve".into(),
            manifest_scope: "governed".into(),
            ephemeral: false,
            query_source: "explicit".into(),
            query_fallback: None,
            gate: None,
            paths: vec![],
            source_hints: None,
            session_id: None,
            transcript: None,
            delivery_profile: DeliveryProfile::Ordinary,
            capsule_profile: None,
            refresh_index: true,
            allow_unprovisioned: false,
            automatic_links: false,
            history_text: None,
            phase_seconds: BTreeMap::new(),
            expected_branch: None,
            scope: None,
            trace_paths: vec![],
            provenance_response: false,
            sanitize: false,
            index_warnings: vec![],
            source_notices: BTreeMap::new(),
        }
    }
}

pub fn admit_document(
    context: &RuntimeContext,
    config: &RuntimeConfig,
    document: &Document,
) -> Result<Option<String>> {
    admit_for(context, config, document, false)
}
/// What `memory search` withholds: what retrieval withholds, except that an
/// open handoff is listed, as the retired search listed it.
pub fn admit_searchable(
    context: &RuntimeContext,
    config: &RuntimeConfig,
    document: &Document,
) -> Result<Option<String>> {
    admit_for(context, config, document, true)
}
fn admit_for(
    context: &RuntimeContext,
    config: &RuntimeConfig,
    document: &Document,
    open_handoffs: bool,
) -> Result<Option<String>> {
    let item = Candidate::linked(document.clone(), "");
    if let Some(reason) =
        crate::source_policy::unscoped_reason_for(context, config, &item, open_handoffs)?
    {
        return Ok(Some(reason));
    }
    let (_, missing) = source_changes(&context.roots, &document.source_fingerprints)?;
    Ok((!missing.is_empty()).then(|| "source-missing".into()))
}
pub(crate) fn map_drift(context: &RuntimeContext, text: &str) -> Result<Option<u64>> {
    let commit = regex::Regex::new(r"(?m)^mapped_commit:\s*([0-9a-fA-F]{7,40})\s*$")
        .map_err(|_| RuntimeError::invalid("Invalid map expression"))?;
    let Some(recorded) = commit.captures(text).and_then(|captures| captures.get(1)) else {
        return Ok(None);
    };
    let mut arguments = crate::git::strings(&[
        "rev-list",
        "--count",
        &format!("{}..HEAD", recorded.as_str()),
    ]);
    let scope = regex::Regex::new(r"(?m)^mapped_scope:\s*(\S+)\s*$")
        .map_err(|_| RuntimeError::invalid("Invalid map scope expression"))?;
    if let Some(scope) = scope.captures(text).and_then(|captures| captures.get(1)) {
        arguments.push("--".into());
        arguments.push(scope.as_str().into());
    }
    let output = crate::git::run(
        &context.roots.project,
        &arguments,
        None,
        Duration::from_secs(5),
        16384,
    )?;
    if output.code != 0 {
        return Ok(None);
    }
    Ok(std::str::from_utf8(&output.stdout)
        .ok()
        .and_then(|text| text.trim().parse().ok()))
}
pub(crate) fn filter_observed(
    context: &RuntimeContext,
    config: &RuntimeConfig,
    candidates: Vec<Candidate>,
    custody: bool,
    mut trace: Option<&mut crate::observability::Trace>,
) -> Result<(Vec<Candidate>, Vec<Exclusion>)> {
    let mut selected = vec![];
    let mut excluded = vec![];
    for mut item in candidates {
        if custody {
            item.source_changed.clear();
        }
        let mut reason = crate::source_policy::unscoped_reason(context, config, &item)?;
        if reason.is_none() {
            let (changed, missing) =
                source_changes(&context.roots, &item.document.source_fingerprints)?;
            if !missing.is_empty() {
                reason = Some("source-missing".into());
            } else if !changed.is_empty() {
                item.source_changed = changed;
                if !custody {
                    item.adjusted_score *= 0.5;
                }
            }
        }
        if let Some(trace) = trace.as_deref_mut() {
            trace.observe(
                &item,
                "runtime",
                reason
                    .as_deref()
                    .unwrap_or(if item.source_changed.is_empty() {
                        "admitted"
                    } else {
                        "source-changed-admitted"
                    }),
                &json!({"score":item.adjusted_score}),
            );
        }
        if let Some(reason) = reason {
            excluded.push(Exclusion {
                path: item.document.path,
                reason,
            });
        } else {
            selected.push(item);
        }
    }
    Ok((selected, excluded))
}
pub fn own_saved_documents(
    store: &Store,
    context: &RuntimeContext,
    task: &Value,
) -> Result<Vec<String>> {
    let external = task["external_id"].as_str().unwrap_or("");
    let task_id = task["id"].as_str().unwrap_or("");
    let mut names =
        crate::history::ticket_ids(external, &crate::history::known_prefixes(store, Some(task)))
            .into_iter()
            .collect::<Vec<_>>();
    names.push(external.into());
    let twins = store
        .state()?
        .get("task-twins")
        .and_then(|raw| crate::domain::json::parse(raw).ok())
        .unwrap_or(json!({}));
    let task_ids = unique_strings(
        std::iter::once(task_id.to_string()).chain(
            twins[external]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(String::from),
        ),
    );
    let saved = |record: &Value| -> bool {
        if !matches!(record["type"].as_str(), Some("finding" | "decision")) {
            return false;
        }
        let id = record["external_id"].as_str().unwrap_or("");
        if names
            .iter()
            .any(|name| id == name || id.starts_with(&format!("{name}-")))
        {
            return true;
        }
        if !id.starts_with("learning-") {
            return false;
        }
        let learning = crate::Learning {
            kind: record["type"].as_str().unwrap_or("").into(),
            title: record["title"].as_str().unwrap_or("").into(),
            consequence: String::new(),
            sources: record["sources"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect(),
            expected_revision: None,
            record_id: None,
            source_fingerprints: None,
        };
        task_ids.iter().any(|task_id| {
            crate::results::learning_id(task_id, &learning).is_ok_and(|external| external == id)
        })
    };
    let mut verdicts = BTreeMap::<String, bool>::new();
    let mut paths = BTreeSet::new();
    let mut statement = store.conn.prepare(
        "SELECT path FROM documents WHERE kind IN ('brain-finding','brain-decision') ORDER BY path",
    )?;
    for path in statement.query_map([], |row| row.get::<_, String>(0))? {
        let path = path?;
        let verdict = fs::read_text(&context.roots.resolve(&path))
            .and_then(|text| parse_markdown_record(&text))
            .map(|(record, _)| saved(&record))
            .unwrap_or(false);
        verdicts.insert(path.clone(), verdict);
        if verdict {
            paths.insert(path);
        }
    }
    let mut statement=store.conn.prepare("SELECT l.path,l.ref_path FROM document_links l JOIN documents d ON d.path=l.path WHERE d.kind='memory' AND l.ref_kind='source' ORDER BY l.path,l.ref_path")?;
    for row in statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (chunk, cited) = row?;
        if !cited.starts_with("project-brain/") {
            continue;
        }
        let verdict = if let Some(verdict) = verdicts.get(&cited) {
            *verdict
        } else if verdicts.len() < 512 {
            let verdict = fs::read_text(&context.roots.resolve(&cited))
                .and_then(|text| parse_markdown_record(&text))
                .map(|(record, _)| saved(&record))
                .unwrap_or(false);
            verdicts.insert(cited, verdict);
            verdict
        } else {
            false
        };
        if verdict {
            paths.insert(chunk);
        }
    }
    Ok(paths.into_iter().collect())
}
fn apply_budgets(candidates: Vec<Candidate>) -> (Vec<Candidate>, Vec<Exclusion>, Option<String>) {
    let mut usage = BTreeMap::<String, usize>::new();
    let mut total = 0;
    let mut selected = Vec::new();
    let mut excluded = Vec::new();
    let mut reasons = Vec::new();
    let mut required = BTreeSet::new();
    for item in &candidates {
        if !item.document.conflicts.is_empty() {
            required.extend(item.document.conflicts.clone());
            if let Some(id) = &item.document.record_id {
                required.insert(id.clone());
            }
        }
    }
    for item in candidates {
        let cost = item.estimated_tokens;
        let category = &item.document.category;
        let limit = BUDGETS
            .iter()
            .find(|(key, _)| *key == category)
            .map(|(_, limit)| *limit)
            .unwrap_or(2000);
        let over = usage.get(category).copied().unwrap_or(0) + cost > limit
            || total + cost > TARGET_BUDGET;
        let conflict = item
            .document
            .record_id
            .as_ref()
            .is_some_and(|id| required.contains(id));
        if over && conflict && total + cost <= HARD_BUDGET {
            reasons.push(format!(
                "conflict pair {} retained beyond normal budget",
                item.document.record_id.as_deref().unwrap_or("")
            ));
        } else if over {
            let reason = if conflict { "hard-budget" } else { "budget" };
            if conflict {
                reasons.push(format!(
                    "conflict pair {} could not fit hard ceiling",
                    item.document.record_id.as_deref().unwrap_or("")
                ));
            }
            excluded.push(Exclusion {
                path: item.document.path,
                reason: reason.into(),
            });
            continue;
        }
        *usage.entry(category.clone()).or_default() += cost;
        total += cost;
        selected.push(item);
    }
    let reason = if reasons.is_empty() {
        None
    } else {
        Some(reasons.join("; "))
    };
    (selected, excluded, reason)
}
fn token_usage(selected: &[Candidate], episodes: &[crate::storage::Episode]) -> Value {
    let mut usage = BTreeMap::<String, usize>::new();
    for (category, _) in BUDGETS {
        usage.insert(category.into(), 0);
    }
    for item in selected {
        *usage.entry(item.document.category.clone()).or_default() += item.estimated_tokens;
    }
    let local = episodes
        .iter()
        .map(|episode| {
            query::estimated_tokens(
                &serde_json::to_value(episode)
                    .ok()
                    .and_then(|value| crate::domain::json::canonical(&value).ok())
                    .unwrap_or_default(),
            )
        })
        .sum::<usize>();
    usage.insert("local_episodes".into(), local);
    let total = usage.values().sum::<usize>();
    usage.insert("total".into(), total);
    usage.insert("target".into(), TARGET_BUDGET);
    usage.insert("hard".into(), HARD_BUDGET);
    json!(usage)
}
#[expect(
    clippy::too_many_arguments,
    reason = "Keep the public gate decision contract's independent policy inputs explicit."
)]
pub fn gate_decision(
    mode: &str,
    signature: &receipts::Signature,
    previous: Option<&Value>,
    diagnostics: &Diagnostics,
    no_match: &[String],
    matched: usize,
    selected: usize,
    conversation: bool,
    persistent: bool,
) -> Value {
    let same_query = previous
        .is_some_and(|previous| previous["query"].as_str() == Some(signature.query.as_str()));
    let same_paths = previous
        .is_some_and(|previous| previous["paths"].as_str() == Some(signature.paths.as_str()));
    let same_task = previous.is_some_and(|previous| {
        previous["task_revision"].as_str() == Some(signature.task_revision.as_str())
    });
    let (decision, reason) = if mode == "off" {
        ("retrieve", "gate-off")
    } else if selected == 0 {
        (
            "skip",
            if matched == 0 {
                "no-relevant-match"
            } else {
                "empty-after-filter"
            },
        )
    } else if same_query && same_paths && same_task {
        if persistent {
            ("retrieve", "persistent-rule-repeat")
        } else if conversation {
            ("retrieve", "not-held-by-conversation")
        } else {
            ("skip", "repeat-retrieval")
        }
    } else if same_query && same_paths {
        ("retrieve", "task-changed")
    } else {
        ("retrieve", "new-selection")
    };
    json!({"decision":decision,"mode":mode,"reason":reason,"signals":{"informative_terms":diagnostics.informative_terms,"distinctive_matches":diagnostics.distinctive_matches,"top_score":diagnostics.top_score,"no_match":no_match,"query_unchanged_from_previous_turn":same_query,"selection_identical_to_previous_turn":same_paths}})
}
fn grouped_projection(
    selected: &[Candidate],
    episodes: &[crate::storage::Episode],
) -> Result<Value> {
    let records = selected
        .iter()
        .filter_map(|item| {
            item.document
                .record_id
                .as_ref()
                .map(|id| (id.clone(), item.document.path.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let visible = records.keys().cloned().collect::<BTreeSet<_>>();
    let mut groups = BTreeMap::<String, Vec<Value>>::new();
    for (category, _) in BUDGETS {
        groups.insert(category.into(), vec![]);
    }
    for item in selected {
        let mut public = item.public_visible(Some(&visible));
        let partners = item
            .document
            .conflicts
            .iter()
            .filter(|id| Some(id.as_str()) != item.document.record_id.as_deref())
            .filter_map(|id| records.get(id))
            .collect::<Vec<_>>();
        if !partners.is_empty() {
            public["conflict"] = json!(partners);
        }
        groups
            .entry(item.document.category.clone())
            .or_default()
            .push(public);
    }
    let procedural = groups
        .get("policy")
        .into_iter()
        .flatten()
        .take(1)
        .cloned()
        .collect::<Vec<_>>();
    let history = BUDGETS
        .iter()
        .flat_map(|(category, _)| groups.get(*category).into_iter().flatten())
        .filter(|item| item["layer"] == "episodic")
        .collect::<Vec<_>>();
    let mut recorded = history
        .iter()
        .copied()
        .filter(|item| item["kind"] == "brain-event")
        .take(2)
        .cloned()
        .collect::<Vec<_>>();
    for episode in episodes.iter().take(2usize.saturating_sub(recorded.len())) {
        recorded.push(serde_json::to_value(episode)?);
    }
    recorded.extend(
        history
            .into_iter()
            .filter(|item| item["kind"] != "brain-event")
            .take(1)
            .cloned(),
    );
    let mut semantic = ["handoff", "durable", "dynamic", "evidence"]
        .into_iter()
        .flat_map(|category| groups.get(category).into_iter().flatten())
        .filter(|item| item["layer"] != "episodic")
        .take(3)
        .cloned()
        .collect::<Vec<_>>();
    semantic.sort_by_key(|item| {
        (
            item["selection"] == "source-hint",
            matches!(
                item["selection"].as_str(),
                Some("source-link" | "prompt-link" | "touch-link")
            ),
        )
    });
    Ok(json!({"categories":groups,"procedural":procedural,"semantic":semantic,"episodic":recorded}))
}
fn reconcile_public_conflicts(capsule: &mut Capsule, selected: &[Candidate]) {
    let originals = selected
        .iter()
        .map(|item| (item.document.path.clone(), item))
        .collect::<BTreeMap<_, _>>();
    let records = selected
        .iter()
        .filter_map(|item| {
            item.document
                .record_id
                .as_ref()
                .map(|id| (id.clone(), item.document.path.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let reconcile = |view: &mut Value| {
        if let Some(items) = view.as_array_mut() {
            for public in items {
                let Some(original) = public["path"].as_str().and_then(|path| originals.get(path))
                else {
                    continue;
                };
                let visible = original
                    .document
                    .conflicts
                    .iter()
                    .filter(|id| {
                        records.contains_key(*id)
                            && Some(id.as_str()) != original.document.record_id.as_deref()
                    })
                    .collect::<BTreeSet<_>>();
                if let Some(conflicts) = public.get_mut("conflicts").and_then(Value::as_array_mut) {
                    conflicts.retain(|id| {
                        id.as_str()
                            .is_some_and(|id| visible.iter().any(|visible| visible.as_str() == id))
                    });
                }
                let partners = visible
                    .into_iter()
                    .filter_map(|id| records.get(id))
                    .collect::<BTreeSet<_>>();
                if let Some(conflicts) = public.get_mut("conflict").and_then(Value::as_array_mut) {
                    conflicts.retain(|path| {
                        path.as_str().is_some_and(|path| {
                            partners.iter().any(|partner| partner.as_str() == path)
                        })
                    });
                }
                if public["conflict"].as_array().is_some_and(Vec::is_empty)
                    && let Some(map) = public.as_object_mut()
                {
                    map.remove("conflict");
                }
                if public.get("conflict_recorded").is_some()
                    && (!original.document.conflicts.is_empty()
                        || original.match_strength.as_deref() == Some("conflict"))
                {
                    public["conflict_recorded"] = true.into();
                }
            }
        }
    };
    for key in ["procedural", "semantic", "episodic", "selected"] {
        reconcile(&mut capsule.0[key]);
    }
    if let Some(groups) = capsule.0["categories"].as_object_mut() {
        for view in groups.values_mut() {
            reconcile(view);
        }
    }
}
fn procedural_slot_eligible(item: &Candidate) -> bool {
    if item.document.kind != "skill" {
        return true;
    }
    let path = &item.document.path;
    let (directory, name) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
    name == "SKILL.md"
        && ![".agents", ".claude", ".cursor", ".codex"]
            .iter()
            .any(|tool| {
                directory == format!("{tool}/skills")
                    || directory.ends_with(&format!("/{tool}/skills"))
            })
}
struct LayerSelection {
    selected: Vec<Candidate>,
    episodes: Vec<crate::storage::Episode>,
    excluded: Vec<Exclusion>,
    budget_excluded: Vec<Exclusion>,
}
fn select_layers(
    budgeted: Vec<Candidate>,
    mut episodes: Vec<crate::storage::Episode>,
) -> LayerSelection {
    let mut vacated = BTreeSet::new();
    let mut listed = BTreeSet::new();
    for item in budgeted
        .iter()
        .filter(|item| {
            item.document.category == "policy"
                && item.match_strength.as_deref() != Some("distinctive")
        })
        .take(1)
    {
        if procedural_slot_eligible(item) {
            listed.insert(item.document.path.clone());
        } else {
            vacated.insert(item.document.path.clone());
        }
    }
    let semantic = budgeted
        .iter()
        .enumerate()
        .filter(|(_, item)| item.document.category != "policy" && item.document.layer != "episodic")
        .take(3)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let events = budgeted
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            item.document.layer == "episodic" && item.document.kind == "brain-event"
        })
        .take(2)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let changelog = budgeted
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            item.document.layer == "episodic" && item.document.kind != "brain-event"
        })
        .take(1)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    episodes.truncate(2usize.saturating_sub(events.len()));
    let mut order = semantic
        .into_iter()
        .chain(events)
        .chain(changelog)
        .collect::<Vec<_>>();
    let episode_tokens = episodes
        .iter()
        .map(|episode| {
            query::estimated_tokens(
                &crate::domain::json::canonical(
                    &serde_json::to_value(episode).unwrap_or(Value::Null),
                )
                .unwrap_or_default(),
            )
        })
        .sum::<usize>();
    let layer_indices = order.iter().copied().collect::<BTreeSet<_>>();
    let mut total = order
        .iter()
        .map(|index| budgeted[*index].estimated_tokens)
        .sum::<usize>();
    let mut budget_excluded = vec![];
    if !episodes.is_empty() && total + episode_tokens > TARGET_BUDGET {
        for index in order.clone().into_iter().rev() {
            let item = &budgeted[index];
            if item.selection.as_deref() != Some("source-hint") {
                continue;
            }
            order.retain(|entry| *entry != index);
            total = total.saturating_sub(item.estimated_tokens);
            budget_excluded.push(Exclusion {
                path: item.document.path.clone(),
                reason: "budget".into(),
            });
            if total + episode_tokens <= TARGET_BUDGET {
                break;
            }
        }
    }
    if !episodes.is_empty() && total + episode_tokens > TARGET_BUDGET {
        episodes.clear();
        budget_excluded.push(Exclusion {
            path: "local-episode".into(),
            reason: "budget".into(),
        });
    }
    let excluded = budgeted
        .iter()
        .enumerate()
        .filter(|(index, _)| !layer_indices.contains(index))
        .map(|(_, item)| Exclusion {
            path: item.document.path.clone(),
            reason: if item.document.layer == "episodic" {
                "episodic-layer"
            } else if vacated.contains(&item.document.path) {
                "skill-subfile"
            } else if listed.contains(&item.document.path) {
                "host-listed"
            } else {
                "layer-limit"
            }
            .into(),
        })
        .collect();
    let mut pool = budgeted.into_iter().map(Some).collect::<Vec<_>>();
    let selected = order
        .into_iter()
        .filter_map(|index| pool[index].take())
        .collect();
    LayerSelection {
        selected,
        episodes,
        excluded,
        budget_excluded,
    }
}
#[expect(
    clippy::too_many_arguments,
    reason = "Keep frozen request, task, selection and warning projection inputs explicit."
)]
fn capsule_projection(
    context: &RuntimeContext,
    request: &RetrievalRequest,
    task: Option<&Value>,
    task_path: Option<&Path>,
    query: &str,
    selected: &[Candidate],
    episodes: &[crate::storage::Episode],
    warnings: &[String],
) -> Result<Capsule> {
    let working = if context.mode == "lightweight" {
        None
    } else {
        task.map(|task|json!({"task_id":task["external_id"],"goal":task["goal"],"progress":task["progress"],"auto_checkpoint":task.get("auto_checkpoint").cloned().unwrap_or(json!("")),"next_steps":task["next_steps"],"files":task["files"],"sources":task["sources"],"phase":task.get("phase").cloned().unwrap_or(Value::Null),"created_at":task["created_at"],"updated_at":task["updated_at"]}))
    };
    let (mut working, omitted) = capsule::project_working(working, request.delivery_profile)?;
    // The task's declared phase and its times travel with the projected state,
    // as the retired capsule carried them: the render shows the phase.
    if let (Some(working), Some(task)) = (working.as_mut(), task) {
        working["phase"] = task.get("phase").cloned().unwrap_or(Value::Null);
        working["created_at"] = task["created_at"].clone();
        working["updated_at"] = task["updated_at"].clone();
    }
    let projection = grouped_projection(selected, episodes)?;
    let visible = selected
        .iter()
        .filter_map(|item| item.document.record_id.clone())
        .collect::<BTreeSet<_>>();
    let mut capsule = Capsule::new(
        json!({"mode":context.mode,"query":query,"query_source":request.query_source,
        "task_id":task.and_then(|task|task["external_id"].as_str()).or(request.task_id.as_deref()),"task_uuid":task.and_then(|task|task["id"].as_str()),"task_revision":task.and_then(|task|task["revision"].as_u64()),
        "task_record":task_path.map(|path|path.strip_prefix(&context.roots.state).unwrap_or(path).to_string_lossy().replace('\\',"/")),
        "working":working,"omitted":omitted,"procedural":projection["procedural"],"semantic":projection["semantic"],"episodic":projection["episodic"],
        "selected":selected.iter().map(|item|item.public_visible(Some(&visible))).collect::<Vec<_>>(),"categories":projection["categories"],
        "no_match":[],"warnings":warnings,"repeated":0,"manifest":null,"last_turn":null,"token_estimates":token_usage(selected,episodes)}),
    );
    if !request.source_notices.is_empty() {
        capsule.0["source_notices"] = json!(request.source_notices);
    }
    if task.is_none() && request.task_id.is_some() {
        capsule.0["kind"] = "warming".into();
    }
    Ok(capsule)
}
pub fn retrieve(
    store: &mut Store,
    context: &RuntimeContext,
    request: &RetrievalRequest,
) -> Result<Capsule> {
    retrieve_inner(store, context, request, None)
}
/// Another active record claims the task's ID: say so and how to keep one, in
/// two lines, as the retired capsule did. The records come earliest created
/// first; a clone without a binding settles on the earliest, so every clone
/// advises cancelling a later one.
fn twin_notices(store: &Store, external_id: &str, served: &str) -> Result<Vec<String>> {
    let twins = store
        .state()?
        .get("task-twins")
        .and_then(|raw| crate::domain::json::parse(raw).ok())
        .unwrap_or(json!({}));
    let claimed = twins[external_id]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    if claimed.len() < 2 || !claimed.contains(&served) {
        return Ok(vec![]);
    }
    let other = claimed
        .iter()
        .find(|id| **id != served)
        .copied()
        .unwrap_or_default();
    let cancel = if served != claimed[0] { served } else { other };
    Ok(vec![
        format!("Task record {other} also claims {external_id}; this turn uses {served}."),
        format!(
            // The installed CLI's name is longer than the retired one's: the
            // whole command still fits the 160 characters a warning keeps.
            "One task per branch: memory-bank/bin/infra-engine memory brain-update --record-id {cancel} --revision auto --transition cancelled"
        ),
    ])
}
/// Seconds to the microsecond, as the retired manifests and gates rounded them.
fn micro(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}
pub fn retrieve_with_preparer(
    store: &mut Store,
    context: &RuntimeContext,
    request: &RetrievalRequest,
    preparer: &mut dyn crate::excerpts::ExcerptPreparer,
) -> Result<Capsule> {
    retrieve_inner(store, context, request, Some(preparer))
}
fn retrieve_inner(
    store: &mut Store,
    context: &RuntimeContext,
    request: &RetrievalRequest,
    preparer: Option<&mut dyn crate::excerpts::ExcerptPreparer>,
) -> Result<Capsule> {
    validate_request(request)?;
    let mut request = request.clone();
    if request.sanitize {
        request.query = crate::domain::sanitize_automatic_query(&request.query)?;
    }
    guard_shared_text("Task Capsule request", &[request.query.clone()])?;
    // The request asks with its plain words, each once and at most 32, as the
    // retired runtime asked (`build_capsule_query`); recorded history is still
    // searched with the whole request.
    if request.history_text.is_none() {
        request.history_text = Some(request.query.clone());
    }
    request.query = query::build_capsule_query(&request.query, None)?;
    let request = &request;
    let mut phase_seconds = request.phase_seconds.clone();
    if request.refresh_index && !store.read_only {
        // The index report's own phases, the filesystem walk (`stat`) apart
        // from indexing, as the retired retrieval recorded them; the
        // retrieval report counted no `stat` sample for an explicit retrieve.
        let index_started = Instant::now();
        let report = index::refresh_index(store, context, true)?;
        for (name, seconds) in &report.phase_seconds {
            if seconds.is_finite() && *seconds >= 0.0 {
                phase_seconds.insert(name.clone(), *seconds);
            }
        }
        phase_seconds
            .entry("index".into())
            .or_insert_with(|| index_started.elapsed().as_secs_f64());
    }
    // The body of retrieval, as the retired manifest timed it: after the
    // index phases it reports beside it, and without packing the capsule.
    let started = Instant::now();
    let mut pack_seconds = 0.0_f64;
    let config = RuntimeConfig::load(&context.roots)?;
    let profile = config.capsule_profile(request.capsule_profile.as_deref())?;
    let mut trace = crate::observability::Trace::new(config.clone(), &request.trace_paths)?;
    let mut source_policy = crate::source_policy::CurrentSourcePolicy::default();
    let _ = trace.watch(&store.conn);
    trace.search_limit = Some(request.limit.saturating_mul(10).max(20));
    let gate_environment = std::env::var("CONTEXT_RETRIEVAL_GATE").ok();
    let gate_mode = request
        .gate
        .as_deref()
        .or(gate_environment.as_deref())
        .or(config.value["retrieval_gate"].as_str())
        .unwrap_or("shadow");
    if !["off", "shadow", "enforce"].contains(&gate_mode) {
        return Err(RuntimeError::invalid("Invalid retrieval gate"));
    }
    admit_branch(context, request)?;
    let brain = BrainRepository::new(context.clone())?;
    let mut warnings = request.index_warnings.clone();
    let mut task = None::<Value>;
    let mut task_path = None::<PathBuf>;
    let mut task_image = None::<String>;
    let mut task_refused = false;
    // What a turn says about the binding of its task, after its other
    // warnings, as the retired capsule said it.
    let mut binding_notices = Vec::new();
    if context.mode == "governed"
        && let Some(id) = &request.task_id
    {
        // The record of the bound task was removed with uncommitted files: the
        // turn is served as for a task not recorded yet, knowledge included,
        // and says so.
        if request.allow_unprovisioned
            && let Some(binding) = store.binding(id)?
            && brain
                .find(&binding.task_uuid, Some("task"), false)
                .is_err_and(|error| error.kind == ErrorKind::NotFound)
        {
            binding_notices.push(format!(
                "The working task's record {} is gone (git stash -u or git clean removes an uncommitted task); the next checkpoint records the task again.",
                binding.task_uuid
            ));
        }
        let served =
            crate::checkpoint::served_uuid(Some(store), context, id)?.unwrap_or_else(|| id.clone());
        match brain.find(&served, Some("task"), false) {
            Ok(image) => {
                task_refused =
                    index::record_eligibility(context, &config, &image.record)?.is_some();
                task_image = Some(working_record_image(&image.record, &image.body)?);
                task_path = Some(image.path);
                if request.allow_unprovisioned {
                    binding_notices.extend(twin_notices(
                        store,
                        image.record["external_id"].as_str().unwrap_or(id),
                        image.record["id"].as_str().unwrap_or(&served),
                    )?);
                }
                task = Some(image.record);
            }
            Err(error) if error.kind == ErrorKind::NotFound && request.allow_unprovisioned => {}
            Err(error) => return Err(error),
        }
    }
    let mut query = request.query.clone();
    if let Some(fallback) = &request.query_fallback {
        let mut tokens = query::query_tokens(&query)?;
        tokens.extend(query::query_tokens(fallback)?);
        query = unique_strings(tokens).join(" ");
    }
    let mut scope = request.scope.clone();
    if profile == "recall" {
        scope = Some(match &task {
            Some(task) if !task_refused => own_saved_documents(store, context, task)?,
            _ => vec![],
        });
    }
    if store.read_only || profile == "navigation" || scope.as_ref().is_some_and(Vec::is_empty) {
        if store.read_only {
            warnings.push("Memory refresh incomplete: SQLite writes unavailable; retrieval was not consulted.".into());
        }
        warnings.extend(binding_notices.iter().cloned());
        let mut capsule = capsule_projection(
            context,
            request,
            task.as_ref(),
            task_path.as_deref(),
            &query,
            &[],
            &[],
            &warnings,
        )?;
        attach_local_working(store, context, request, &mut capsule)?;
        if let Some(id) = &request.task_id {
            capsule.0["pending_turns"] = json!(store.pending_turns(id)?.len());
        }
        let (mut capsule, _) = cap_working_custody(
            capsule,
            context,
            request,
            task.as_ref(),
            task_path.as_deref(),
            task_image.as_deref(),
        )?;
        // A capsule that searched nothing has the retired one's shape: no
        // categories, and its token estimates by layer.
        capsule.0["categories"] = json!({});
        capsule.0["token_estimates"] =
            json!({"procedural":0,"semantic":0,"episodic":0,"local_episodes":0,"total":0});
        return capsule::pack(capsule, request.delivery_profile);
    }
    warnings.extend(binding_notices);
    let (mut candidates, diagnostics) = query::rank_candidates_observed(
        &store.conn,
        &query,
        request.limit.saturating_mul(10).max(20),
        scope.as_deref(),
        Some(&mut trace),
    )?;
    let mut graph_anchor_count = 0usize;
    let mut graph_candidate_count = 0usize;
    let mut hint_stats = None::<Value>;
    let mut hinted_candidates = Vec::new();
    let request_text = request.history_text.as_deref().unwrap_or(&query);
    let known =
        crate::history::known_prefixes(store, if task_refused { None } else { task.as_ref() });
    let prompt_tickets = crate::history::request_tickets(request_text, None, &known);
    let request_tickets = crate::history::request_tickets(
        request_text,
        if task_refused { None } else { task.as_ref() },
        &known,
    );
    let mut weighted_episodes = Vec::new();
    let mut history = if scope.is_none() {
        if let Some(mut history) = crate::history::history_query(&store.conn, request_text)? {
            history.tickets = request_tickets.clone();
            history.known = known.clone();
            let (events, episodes) = crate::history::candidates(&store.conn, &history)?;
            weighted_episodes = episodes;
            events
        } else {
            vec![]
        }
    } else {
        vec![]
    };
    for item in &history {
        trace.observe(
            item,
            "history",
            "admitted",
            &json!({"score":item.adjusted_score}),
        );
    }
    let mut matched = candidates.len() + history.len() + weighted_episodes.len();
    let mut matched_layers = candidates
        .iter()
        .map(|item| item.document.layer.clone())
        .collect::<BTreeSet<_>>();
    if !history.is_empty() || !weighted_episodes.is_empty() {
        matched_layers.insert("episodic".into());
    }
    let no_match = ["procedural", "semantic", "episodic"]
        .into_iter()
        .filter(|layer| !matched_layers.contains(*layer))
        .map(String::from)
        .collect::<Vec<_>>();
    let task_ticket_events = history
        .iter()
        .filter(|item| {
            let named = crate::history::request_tickets(&item.document.title, None, &known);
            !named.is_disjoint(&request_tickets) && named.is_disjoint(&prompt_tickets)
        })
        .map(|item| item.document.path.clone())
        .collect::<BTreeSet<_>>();
    let lexical_count = candidates.len();
    let mut primed = std::mem::take(&mut candidates);
    primed.extend(history);
    source_policy.prime(&store.conn, context, &mut primed)?;
    history = primed.split_off(lexical_count);
    candidates = primed;
    let (filtered, mut excluded) = crate::source_policy::filter_in_scope(
        context,
        &config,
        &store.conn,
        &mut source_policy,
        std::mem::take(&mut candidates),
        false,
        Some(&mut trace),
    )?;
    candidates = filtered;
    candidates.sort_by(|left, right| right.adjusted_score.total_cmp(&left.adjusted_score));
    let exact = history
        .iter()
        .filter(|item| {
            !crate::history::request_tickets(&item.document.title, None, &known)
                .is_disjoint(&request_tickets)
        })
        .cloned()
        .collect::<Vec<_>>();
    let capacity = if exact.is_empty() { 1 } else { 2 };
    let (events, event_excluded) = crate::history::best_events_in_scope(
        context,
        &config,
        &store.conn,
        &mut source_policy,
        if exact.is_empty() { history } else { exact },
        capacity,
        &known,
        Some(&mut trace),
    )?;
    let mut events = events;
    excluded.extend(event_excluded);
    let mut history_episode = Vec::new();
    if let Some(best) = weighted_episodes.first()
        && events
            .first()
            .is_none_or(|event| best.score > event.adjusted_score)
    {
        history_episode.push(best.episode.clone());
        events.clear();
    }
    if scope.is_none() {
        let history_paths = events
            .iter()
            .map(|item| item.document.path.clone())
            .collect::<BTreeSet<_>>();
        let graph_anchors = candidates
            .iter()
            .filter(|item| {
                item.document.layer == "semantic"
                    && item.match_strength.as_deref() == Some("covered")
                    && task
                        .as_ref()
                        .and_then(|task| task["id"].as_str())
                        .is_none_or(|id| item.document.record_id.as_deref() != Some(id))
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut seen = candidates
            .iter()
            .map(|item| item.document.path.clone())
            .collect::<BTreeSet<_>>();
        seen.extend(history_paths.iter().cloned());
        let (linked, refused) = crate::source_links::path_candidates(
            &store.conn,
            context,
            &config,
            &request.paths,
            &seen,
            &mut source_policy,
            Some(&mut trace),
        )?;
        matched += linked.len() + refused.len();
        excluded.extend(refused);
        let mut lead = linked;
        lead.extend(candidates);
        candidates = lead;
        if request.automatic_links {
            let mut seeds =
                crate::source_links::prompt_path_seeds(&store.conn, context, request_text)?
                    .into_iter()
                    .map(|path| (path, "prompt".to_string()))
                    .collect::<Vec<_>>();
            let mut touched = vec![];
            if !task_refused {
                if let Some(id) = &request.task_id {
                    touched
                        .extend(store.recent_touched(id, crate::source_links::TOUCH_SEED_LIMIT)?);
                }
                if let Some(task) = &task {
                    touched.extend(
                        task["files"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .rev()
                            .filter_map(Value::as_str)
                            .map(String::from),
                    );
                }
            }
            let mut unique = BTreeSet::new();
            seeds.extend(
                touched
                    .into_iter()
                    .filter(|path| {
                        crate::source_links::file_link_problem(path).is_none()
                            && unique.insert(path.clone())
                    })
                    .take(crate::source_links::TOUCH_SEED_LIMIT)
                    .map(|path| (path, "touch".to_string())),
            );
            let mut unique = BTreeSet::new();
            seeds.retain(|(path, _)| unique.insert(path.clone()));
            let mut own_paths = history_paths.clone();
            if let (Some(task), Some(path)) = (&task, &task_path) {
                if let Ok(relative) = path.strip_prefix(&context.roots.state) {
                    own_paths.insert(relative.to_string_lossy().replace('\\', "/"));
                }
                if let Some(id) = task["id"].as_str() {
                    own_paths.insert(format!("project-brain/control/handoffs/{id}.md"));
                }
            }
            let (linked, refused) = crate::source_links::automatic_link_candidates(
                &store.conn,
                context,
                &config,
                &seeds,
                &candidates,
                &own_paths,
                &diagnostics.coverage,
                &mut source_policy,
                Some(&mut trace),
            )?;
            excluded.extend(refused);
            candidates = crate::source_links::insert_automatic_links(candidates, linked);
        }
        let mut seen = candidates
            .iter()
            .map(|item| item.document.path.clone())
            .collect::<BTreeSet<_>>();
        seen.extend(history_paths);
        let (linked, stats) = crate::source_links::source_link_candidates(
            &store.conn,
            context,
            &config,
            &graph_anchors,
            &seen,
            &mut source_policy,
            Some(&mut trace),
        )?;
        graph_anchor_count = stats.anchors;
        graph_candidate_count = linked.len();
        candidates.extend(linked);
        if let Some(hints) = &request.source_hints {
            let (mut references, mut state) = crate::source_hints::normalize(hints);
            let supported = crate::source_hints::schema_supported(context);
            if !supported {
                references.clear();
                state = "unsupported";
            }
            let mut seen = candidates
                .iter()
                .chain(events.iter())
                .map(|item| item.document.path.clone())
                .collect::<BTreeSet<_>>();
            let hints = if references.is_empty() {
                vec![]
            } else {
                crate::source_hints::candidates(
                    &store.conn,
                    context,
                    &config,
                    &references,
                    &mut seen,
                    &mut source_policy,
                    Some(&mut trace),
                )?
            };
            if supported {
                hint_stats = Some(
                    json!({"profile":"source-hints-v1","state":state,"supplied":if state=="ready"{references.len()}else{0},"admitted":hints.len(),"delivered":0}),
                );
            }
            hinted_candidates = hints;
        }
    }
    events.extend(candidates);
    candidates = events;
    let (filtered, ticket_excluded) =
        crate::history::ticket_filter(context, candidates, &request_tickets, &known, request_text)?;
    candidates = filtered;
    excluded.extend(ticket_excluded);
    candidates.extend(hinted_candidates);
    let promoted = candidates
        .iter()
        .filter(|item| {
            item.document.kind == "memory"
                && !matches!(
                    item.selection.as_deref(),
                    Some("source-link" | "source-hint")
                )
        })
        .flat_map(|item| {
            item.document
                .source_fingerprints
                .as_array()
                .into_iter()
                .flatten()
        })
        .filter_map(|row| row["path"].as_str().map(String::from))
        .collect::<BTreeSet<_>>();
    candidates.retain(|item| {
        let retain =
            !(item.document.kind.starts_with("brain-") && promoted.contains(&item.document.path));
        if !retain {
            excluded.push(Exclusion {
                path: item.document.path.clone(),
                reason: "promoted-to-chunk".into(),
            });
        }
        retain
    });
    expand_conflicts(
        store,
        context,
        &config,
        &mut source_policy,
        &mut candidates,
        &mut excluded,
        Some(&mut trace),
    )?;
    let own_id = task
        .as_ref()
        .and_then(|task| task["id"].as_str())
        .map(String::from);
    let own_handoff = own_id
        .as_ref()
        .map(|id| format!("project-brain/control/handoffs/{id}.md"));
    let host_loaded = crate::continuity::host_loaded(context, &request.host);
    candidates.retain(|item| {
        let reason = if own_id.is_some()
            && (item.document.record_id.as_deref() == own_id.as_deref()
                || Some(&item.document.path) == own_handoff.as_ref())
        {
            Some("working-task")
        } else if host_loaded.contains(&item.document.path) {
            Some("host-loaded")
        } else {
            None
        };
        if let Some(reason) = reason {
            excluded.push(Exclusion {
                path: item.document.path.clone(),
                reason: reason.into(),
            });
            false
        } else {
            true
        }
    });
    let mut unique = BTreeSet::new();
    candidates.retain(|item| unique.insert(item.document.path.clone()));
    candidates.sort_by_key(|item| item.selection.as_deref() == Some("source-hint"));
    for (position, item) in candidates.iter().enumerate() {
        trace.observe(item,"queue",item.selection.as_deref().unwrap_or("lexical"),&json!({"rank":position+1,"estimated_tokens":item.estimated_tokens,"score":item.adjusted_score}));
    }
    trace.exclusions(&excluded, "runtime");
    trace.boundary("queue_after_runtime", &candidates);
    let (budgeted, budget_excluded, escalation) = apply_budgets(candidates);
    trace.boundary("budget_admitted", &budgeted);
    trace.exclusions(&budget_excluded, "budget");
    excluded.extend(budget_excluded);
    for item in &budgeted {
        trace.observe(
            item,
            "budget",
            "admitted",
            &json!({"estimated_tokens":item.estimated_tokens}),
        );
    }
    let layers = select_layers(budgeted, history_episode);
    let mut selected = layers.selected;
    let mut episodes = layers.episodes;
    let mut layer_excluded = layers.excluded;
    excluded.extend(layers.budget_excluded);
    let terms =
        (!selected.is_empty()).then(|| crate::excerpts::QueryTerms::read(&store.conn, &query));
    for item in &mut selected {
        item.delivery_identity = Some(crate::excerpts::delivery_identity_with(
            &store.conn,
            item,
            &query,
            terms.as_ref(),
        ));
    }
    trace.boundary("layer_pre_novelty", &selected);
    let mut proposals = capsule_projection(
        context,
        request,
        task.as_ref(),
        task_path.as_deref(),
        &query,
        &selected,
        &episodes,
        &warnings,
    )?;
    attach_local_working(store, context, request, &mut proposals)?;
    let selector_prepared = preparer.is_some();
    if let Some(preparer) = preparer {
        crate::excerpts::prepare_with(
            &store.conn,
            &mut proposals,
            &mut selected,
            request.delivery_profile,
            preparer,
        )?;
    } else {
        proposals.prepare(&store.conn, &selected)?;
    }
    let prepared = proposals.1.clone();
    let proposal_windows = capsule::describe_excerpt_windows(&proposals);
    episodes.retain(|episode| {
        sensitive_label(&serde_json::to_string(episode).unwrap_or_default())
            .ok()
            .flatten()
            .is_none()
    });
    let snapshot = receipts::load(&store.conn)?;
    let session = request
        .session_id
        .as_deref()
        .filter(|id| receipts::valid_session(id));
    let entry = session.and_then(|id| snapshot.sessions.get(id));
    let turn = entry.and_then(|entry| entry["turn"].as_u64()).unwrap_or(0) + 1;
    let (compacted, transcript) =
        receipts::transcript_compacted(entry, request.transcript.as_deref());
    let mut repeated = 0;
    let mut repeated_items = Vec::new();
    if session.is_some() && !compacted {
        selected.retain(|item| {
            let (key, revision) = receipts::identity(item);
            let seen = receipts::was_handed(entry, &key, &revision, turn);
            if seen {
                repeated += 1;
                repeated_items.push(item.clone());
                layer_excluded.push(Exclusion {
                    path: item.document.path.clone(),
                    reason: "delivered-this-session".into(),
                });
                if let Some(previous) = entry {
                    trace.repeat_window(
                        context,
                        item,
                        &key,
                        &revision,
                        &previous["items"][key.as_str()],
                        turn,
                        previous,
                        proposal_windows
                            .get(&item.document.path)
                            .unwrap_or(&Value::Null),
                    );
                }
            }
            !seen
        });
        episodes.retain(|episode| {
            let key = format!("episode:{}", episode.id);
            let revision = episode_revision(episode);
            let seen = receipts::was_handed(entry, &key, &revision, turn);
            if seen {
                repeated += 1;
                layer_excluded.push(Exclusion {
                    path: format!("episode:{}", episode.id),
                    reason: "delivered-this-session".into(),
                });
            }
            !seen
        });
    }
    // Only a session's conversation withholds what it already holds: without
    // one nothing was checked, and the boundary stays unobserved, as the
    // retired runtime left it, rather than an exact zero.
    if session.is_some() {
        trace.boundary("repeat_withheld", &repeated_items);
    }
    rebase_explicit_working(
        context,
        request,
        &mut task,
        task_path.as_deref(),
        &mut task_image,
    );
    let mut baseline = receipts::baseline_key(
        own_id.as_deref().unwrap_or("no-task"),
        &request.host,
        &request.entry_point,
        request.delivery_profile.persistent(),
    );
    let mut identities = selected.iter().map(receipts::identity).collect::<Vec<_>>();
    identities.extend(
        episodes
            .iter()
            .map(|episode| (format!("episode:{}", episode.id), episode_revision(episode))),
    );
    let mut signature = receipts::signature(
        &query,
        &identities,
        task.as_ref()
            .and_then(|task| task["revision"].as_u64())
            .unwrap_or(0),
    );
    let mut gate = gate_decision(
        gate_mode,
        &signature,
        snapshot.baselines.get(&baseline),
        &diagnostics,
        &no_match,
        matched,
        selected.len() + episodes.len(),
        session.is_some(),
        request.delivery_profile.persistent(),
    );
    trace.boundary("gate_input", &selected);
    let initial_withheld = gate_mode == "enforce" && gate["decision"] == "skip";
    trace.boundary(
        "gate_withheld",
        if initial_withheld { &selected } else { &[] },
    );
    trace.exclusions(&layer_excluded, "layer");
    excluded.extend(layer_excluded);
    if initial_withheld {
        for item in &selected {
            trace.observe(item, "gate", "withheld", &Value::Null);
        }
        selected.clear();
        episodes.clear();
    }
    let mut capsule = capsule_projection(
        context,
        request,
        task.as_ref(),
        task_path.as_deref(),
        &query,
        &selected,
        &episodes,
        &warnings,
    )?;
    attach_local_working(store, context, request, &mut capsule)?;
    capsule.0["no_match"] = json!(no_match);
    capsule.0["gate"] = gate.clone();
    capsule.0["repeated"] = json!(repeated);
    if let Some(id) = &request.task_id {
        capsule.0["pending_turns"] = json!(store.pending_turns(id)?.len());
    }
    trace.boundary("pack_input", &selected);
    capsule.1 = prepared;
    let pack_started = Instant::now();
    capsule = capsule::pack(capsule, request.delivery_profile)?;
    pack_seconds += pack_started.elapsed().as_secs_f64();
    let shown = capsule.shown_paths();
    let shown_episodes = capsule.shown_episodes();
    let mut pack_omitted = Vec::new();
    let mut pack_excluded = Vec::new();
    selected.retain(|item| {
        let keep = shown.contains(&item.document.path);
        if !keep {
            pack_omitted.push(item.clone());
            pack_excluded.push(Exclusion {
                path: item.document.path.clone(),
                reason: "capsule-limit".into(),
            });
        }
        keep
    });
    episodes.retain(|episode| shown_episodes.contains(&episode.id));
    trace.boundary("pack_omitted", &pack_omitted);
    trace.exclusions(&pack_excluded, "capsule");
    excluded.extend(pack_excluded);
    let final_config = RuntimeConfig::load(&context.roots)?;
    let original_drift = selected
        .iter()
        .map(|item| {
            (
                item.document.path.clone(),
                item.source_changed.iter().cloned().collect::<BTreeSet<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut final_policy = crate::source_policy::CurrentSourcePolicy::default();
    let (current, refused) = crate::source_policy::filter_in_scope(
        context,
        &final_config,
        &store.conn,
        &mut final_policy,
        selected,
        true,
        Some(&mut trace),
    )?;
    excluded.extend(refused);
    selected = Vec::new();
    for item in current {
        if final_policy.missing_source(context, &item)? {
            excluded.push(Exclusion {
                path: item.document.path.clone(),
                reason: "source-missing".into(),
            });
        } else if original_drift.get(&item.document.path)
            != Some(&item.source_changed.iter().cloned().collect())
        {
            excluded.push(Exclusion {
                path: item.document.path.clone(),
                reason: "source-changed-during-pack".into(),
            });
        } else {
            selected.push(item);
        }
    }
    let (updated, working_state) = cap_working_custody(
        capsule,
        context,
        request,
        task.as_ref(),
        task_path.as_deref(),
        task_image.as_deref(),
    )?;
    capsule = updated;
    let derived_withdrawn =
        working_state.withdrawn && matches!(request.query_source.as_str(), "task" | "continuation");
    let scoped_withdrawn = working_state.denied && scope.is_some();
    if derived_withdrawn || scoped_withdrawn {
        selected.clear();
        episodes.clear();
    } else if working_state.denied {
        selected.retain(|item| {
            let remove = item.selection.as_deref() == Some("touch-link")
                || task_ticket_events.contains(&item.document.path);
            if remove {
                excluded.push(Exclusion {
                    path: item.document.path.clone(),
                    reason: "working-unavailable".into(),
                });
            }
            !remove
        });
    }
    if working_state.denied {
        task = None;
        baseline = receipts::baseline_key(
            "no-task",
            &request.host,
            &request.entry_point,
            request.delivery_profile.persistent(),
        );
    }
    let allowed = selected
        .iter()
        .map(|item| item.document.path.clone())
        .collect::<BTreeSet<_>>();
    for layer in ["procedural", "semantic", "episodic"] {
        if let Some(items) = capsule.0[layer].as_array_mut() {
            items.retain(|item| {
                item["path"]
                    .as_str()
                    .is_some_and(|path| allowed.contains(path))
                    || item["id"]
                        .as_i64()
                        .is_some_and(|id| episodes.iter().any(|episode| episode.id == id))
            });
        }
    }
    capsule::synchronize(&mut capsule);
    let pack_started = Instant::now();
    capsule = capsule::pack(capsule, request.delivery_profile)?;
    pack_seconds += pack_started.elapsed().as_secs_f64();
    let shown = capsule.shown_paths();
    let shown_episodes = capsule.shown_episodes();
    selected.retain(|item| {
        let keep = shown.contains(&item.document.path);
        if !keep {
            pack_omitted.push(item.clone());
            let entry = Exclusion {
                path: item.document.path.clone(),
                reason: "capsule-limit".into(),
            };
            trace.exclusions(std::slice::from_ref(&entry), "capsule");
            excluded.push(entry);
        }
        keep
    });
    trace.boundary("pack_omitted", &pack_omitted);
    episodes.retain(|episode| shown_episodes.contains(&episode.id));
    // The retired renderer cut excerpts from the capsule it delivered, so the
    // room their growth shares counted only the documents still in it. One the
    // pack dropped (a changelog past its slot, a fourth result) spent that
    // room here and kept the best excerpts from growing; they are prepared
    // again for what remains.
    if !selector_prepared
        && capsule.1.as_ref().is_some_and(|prepared| {
            prepared
                .excerpts
                .keys()
                .any(|key| match key.strip_prefix("episode ") {
                    Some(id) => id
                        .parse::<i64>()
                        .map_or(true, |id| !shown_episodes.contains(&id)),
                    None => !shown.contains(key),
                })
        })
    {
        capsule.prepare(&store.conn, &selected)?;
    }
    identities = selected.iter().map(receipts::identity).collect();
    identities.extend(
        episodes
            .iter()
            .map(|episode| (format!("episode:{}", episode.id), episode_revision(episode))),
    );
    signature = receipts::signature(
        capsule.0["query"].as_str().unwrap_or(&query),
        &identities,
        task.as_ref()
            .and_then(|task| task["revision"].as_u64())
            .unwrap_or(0),
    );
    gate = gate_decision(
        gate_mode,
        &signature,
        snapshot.baselines.get(&baseline),
        &diagnostics,
        &no_match,
        matched,
        selected.len() + episodes.len(),
        session.is_some(),
        request.delivery_profile.persistent(),
    );
    let withheld = gate_mode == "enforce" && gate["decision"] == "skip";
    if withheld {
        selected.clear();
        episodes.clear();
        identities.clear();
        for layer in ["procedural", "semantic", "episodic", "selected"] {
            capsule.0[layer] = json!([]);
        }
        capsule::synchronize(&mut capsule);
    }
    capsule.0["gate"] = gate.clone();
    reconcile_public_conflicts(&mut capsule, &selected);
    capsule.0["token_estimates"] = token_usage(&selected, &episodes);
    let manifest_id = uuid4();
    let local = request.ephemeral || request.manifest_scope == "local" || task.is_none();
    let directory = if local {
        context
            .roots
            .state
            .join("memory-bank/local/retrieval-manifests")
    } else {
        context
            .roots
            .brain_root()
            .join("control/retrieval-manifests")
    };
    let path = directory.join(format!("{manifest_id}.json"));
    let mut manifest = json!({"schema_version":4,"id":manifest_id,"created_at":utc_now(),"query":capsule.0["query"],"task_id":task.as_ref().and_then(|task|task["id"].as_str()),"task_revision":task.as_ref().and_then(|task|task["revision"].as_u64()),"local_episode_count":episodes.len(),"filters":{"privacy":final_config.value["allowed_privacy"],"owners":final_config.value["owners"],"authority":final_config.value["allowed_authority"],"freshness":true,"active_only":true},"selected":selected.iter().map(|item| {
        let mut value = json!({
            "path": item.document.path,
            "category": item.document.category,
            "estimated_tokens": item.estimated_tokens,
            "source_hash": item.document.source_hash,
            "score": (item.adjusted_score * 1_000_000.0).round() / 1_000_000.0,
            "rank": item.rank
        });
        if let Some(strength) = &item.match_strength {
            value["match"] = strength.clone().into();
        }
        if let Some(selection) = &item.selection {
            value["selection"] = selection.clone().into();
        }
        if !item.source_changed.is_empty() {
            value["source_changed"] = json!(true);
        }
        value
    }).collect::<Vec<_>>(),"excluded":excluded,"token_estimates":capsule.0["token_estimates"],"provider":final_config.value["provider"],"source_links":{"anchors":graph_anchor_count,"candidates":graph_candidate_count,"delivered":selected.iter().filter(|item|item.selection.as_deref()==Some("source-link")).count()},"escalation_reason":escalation,"query_source":capsule.0["query_source"],"host":request.host,"entry_point":request.entry_point,"gate":gate,"phase_seconds":{"stat":phase_seconds.get("stat").copied().map(micro),"index":phase_seconds.get("index").copied().map(micro),"retrieval":micro((started.elapsed().as_secs_f64()-pack_seconds).max(0.0))}});
    if let Some(mut stats) = hint_stats {
        stats["delivered"] = json!(
            selected
                .iter()
                .filter(|item| item.selection.as_deref() == Some("source-hint"))
                .count()
        );
        manifest["source_hints"] = stats;
    }
    validate_schema_file(&context.roots, "retrieval-manifest.schema.json", &manifest)?;
    admit_branch(context, request)?;
    let published = if local {
        fs::atomic_json(&path, &manifest)
    } else {
        let _guard = brain.lock()?;
        fs::atomic_json(&path, &manifest)
    };
    if let Err(error) = published {
        if !local {
            return Err(error);
        }
        capsule.0["manifest"] = Value::Null;
        capsule.0["manifest_publication"] = json!("unconfirmed");
        if request.provenance_response {
            capsule.2 = crate::observability::source_provenance(&capsule, &manifest);
        }
        return Ok(capsule);
    }
    capsule.0["manifest"] = json!(
        path.strip_prefix(&context.roots.state)
            .map_err(|_| RuntimeError::policy("Manifest escapes state"))?
            .to_string_lossy()
            .replace('\\', "/")
    );
    capsule.0["manifest_scope"] = json!(if local { "local" } else { "governed" });
    let baseline_plan = if withheld {
        None
    } else {
        Some(BaselinePlan {
            key: baseline.clone(),
            signature,
            expected: snapshot.baselines.get(&baseline).cloned(),
        })
    };
    let windows=selected.iter().filter(|item|trace.eligible(item)).map(|item|{let(key,revision)=receipts::identity(item);(query::sha256(key.as_bytes()),json!({"revision":revision,"handed_turn":turn,"manifest_id":manifest_id,"doc_ref":query::sha256(item.document.path.as_bytes())}))}).collect::<serde_json::Map<String,Value>>();
    let session_plan = session.map(|id| SessionPlan {
        id: id.into(),
        turn,
        expected: snapshot.sessions.get(id).cloned(),
        delivered: identities
            .into_iter()
            .map(|(key, revision)| (key, (revision, turn)))
            .collect(),
        transcript,
        reset: compacted,
        exact_next_turn: true,
        window_receipts: Some(Value::Object(windows)),
    });
    let receipt_status = receipts::commit(store, baseline_plan.as_ref(), session_plan.as_ref());
    trace.ordinary_receipt = crate::observability::observe_ordinary_receipt(
        match receipt_status {
            receipts::ReceiptStatus::Skipped => "skipped",
            receipts::ReceiptStatus::Deferred => "deferred",
            receipts::ReceiptStatus::Committed => "committed",
            receipts::ReceiptStatus::Unknown => "unknown",
        },
        baseline_plan.is_some() || session_plan.is_some(),
    );
    trace.boundary("selected_post_pack", &selected);
    for item in &selected {
        trace.observe(
            item,
            "delivery",
            "selected",
            &json!({"estimated_tokens":item.estimated_tokens}),
        );
    }
    trace.save(context, &manifest_id);
    if local {
        let retention = final_config.value["local_manifest_retention"]
            .as_u64()
            .unwrap_or(200)
            .max(1) as usize;
        let _ = prune_manifests(&directory, retention);
    }
    Ok(capsule)
}

fn validate_request(request: &RetrievalRequest) -> Result<()> {
    if request.limit == 0 {
        return Err(RuntimeError::invalid("--limit must be a positive integer"));
    }
    if !["cli", "claude", "codex", "cursor"].contains(&request.host.as_str())
        || !["context", "retrieve", "refresh", "hook-context"]
            .contains(&request.entry_point.as_str())
        || !["governed", "local"].contains(&request.manifest_scope.as_str())
        || !["prompt", "task", "task-id", "explicit", "continuation"]
            .contains(&request.query_source.as_str())
    {
        return Err(RuntimeError::invalid("Unsupported retrieval transport"));
    }
    if request.delivery_profile.persistent()
        && (request.host != "cursor"
            || !["refresh", "hook-context"].contains(&request.entry_point.as_str())
            || request.session_id.is_some()
            || request.transcript.is_some())
    {
        return Err(RuntimeError::invalid(
            "Unsupported persistent delivery profile",
        ));
    }
    if let Some(id) = &request.task_id {
        crate::storage::validate_task_id(id)?;
    }
    Ok(())
}
fn attach_local_working(
    store: &Store,
    context: &RuntimeContext,
    request: &RetrievalRequest,
    capsule: &mut Capsule,
) -> Result<()> {
    if context.mode == "lightweight"
        && let Some(id) = &request.task_id
    {
        let (working, omitted) = capsule::project_working(
            store.working(id)?.map(serde_json::to_value).transpose()?,
            request.delivery_profile,
        )?;
        capsule.0["working"] = json!(working);
        capsule.0["omitted"] = json!(omitted);
    }
    crate::continuity::attach_recent(store, context, capsule)?;
    let recent = capsule.0["recent_files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect::<Vec<_>>();
    let files = crate::continuity::ticket_files(
        store,
        context,
        &request.query,
        request.task_id.as_deref(),
        &recent,
    );
    if !files.is_empty() {
        let known = crate::history::known_prefixes(store, None);
        capsule.0["ticket_files"] =
            json!({"tickets":crate::history::ticket_ids(&request.query,&known),"paths":files});
    }
    capsule.0["last_turn"] = json!(crate::advisory::last_turn(
        context,
        !request.delivery_profile.persistent()
    ));
    Ok(())
}
fn episode_revision(episode: &crate::Episode) -> String {
    serde_json::to_value(episode)
        .and_then(|value| {
            crate::domain::json::canonical(&value)
                .map_err(|error| serde_json::Error::io(std::io::Error::other(error.message)))
        })
        .map(|value| query::sha256(value.as_bytes()))
        .unwrap_or_default()
}
#[derive(Clone, Copy, Debug, Default)]
struct WorkingCustody {
    withdrawn: bool,
    changed: bool,
    denied: bool,
}
fn working_record_image(record: &Value, body: &str) -> Result<String> {
    Ok(query::sha256(
        crate::domain::json::canonical(&json!([record, body]))?.as_bytes(),
    ))
}
fn read_working_record(path: &Path) -> Result<(Value, String)> {
    // Foundation uses a regular nonblocking nofollow descriptor and compares
    // its native image before/after reading. Text uses universal newlines.
    let text = fs::read_text(path)?;
    let (record, body) = parse_markdown_record(&text)?;
    validate_record(&record, false)?;
    Ok((record, body))
}
fn cap_working_custody(
    mut capsule: Capsule,
    context: &RuntimeContext,
    request: &RetrievalRequest,
    task: Option<&Value>,
    path: Option<&Path>,
    image: Option<&str>,
) -> Result<(Capsule, WorkingCustody)> {
    let Some(task) = task else {
        return Ok((capsule, WorkingCustody::default()));
    };
    let state = (|| -> Result<WorkingCustody> {
        let path =
            path.ok_or_else(|| RuntimeError::unavailable("Working record image unavailable"))?;
        let expected =
            image.ok_or_else(|| RuntimeError::unavailable("Working record image unavailable"))?;
        let (record, body) = read_working_record(path)?;
        let config = RuntimeConfig::load(&context.roots)?;
        let eligible = index::record_eligibility(context, &config, &record)?.is_none();
        let same = record["id"] == task["id"];
        let current = working_record_image(&record, &body)?;
        let changed = eligible && same && current != expected;
        let valid = eligible && same && current == expected;
        Ok(WorkingCustody {
            withdrawn: !valid,
            changed,
            denied: !valid && !changed,
        })
    })()
    .unwrap_or(WorkingCustody {
        withdrawn: true,
        changed: false,
        denied: true,
    });
    if state.withdrawn {
        capsule.0["working"] = Value::Null;
        if let Some(map) = capsule.0.as_object_mut() {
            map.remove("capsule_text");
        }
        if state.changed {
            capsule.0["working_changed"] = true.into();
        }
        if matches!(request.query_source.as_str(), "task" | "continuation") {
            capsule.0["query"] = json!(request.task_id);
            capsule.0["query_source"] = "task-id".into();
        }
    }
    if state.denied {
        capsule.0["task_id"] = json!(request.task_id);
        capsule.0["task_uuid"] = Value::Null;
        capsule.0["task_revision"] = Value::Null;
        capsule.0["task_record"] = Value::Null;
        capsule.0["last_turn"] = Value::Null;
        capsule.0["omitted"] = json!({});
        capsule.0["omitted"] = json!({});
        if let Some(map) = capsule.0.as_object_mut() {
            map.remove("working_changed");
        }
        if let Some(warnings) = capsule.0["warnings"].as_array_mut() {
            warnings.push(json!("Working task eligibility unavailable."));
        }
    }
    Ok((capsule, state))
}
fn rebase_explicit_working(
    context: &RuntimeContext,
    request: &RetrievalRequest,
    task: &mut Option<Value>,
    path: Option<&Path>,
    image: &mut Option<String>,
) {
    if matches!(request.query_source.as_str(), "task" | "continuation") {
        return;
    }
    let operation = (|| -> Result<()> {
        let (Some(old), Some(path), Some(expected)) = (task.as_ref(), path, image.as_ref()) else {
            return Ok(());
        };
        let (record, body) = read_working_record(path)?;
        let current = working_record_image(&record, &body)?;
        if record["id"] == old["id"]
            && current != *expected
            && index::record_eligibility(context, &RuntimeConfig::load(&context.roots)?, &record)?
                .is_none()
        {
            *task = Some(record);
            *image = Some(current);
        }
        Ok(())
    })();
    let _ = operation;
}
/// The hook named its task by the branch the checkout works on; the capsule
/// is delivered only while that still holds. During a rebase or a bisect,
/// or in a Codex thread's detached worktree, HEAD names no branch, and the
/// working identity is what the hook read (`GitFrame::working_identity`).
fn admit_branch(context: &RuntimeContext, request: &RetrievalRequest) -> Result<()> {
    if let Some(expected) = &request.expected_branch {
        let frame = crate::git::GitFrame::discover(&context.roots.project)?
            .ok_or_else(|| RuntimeError::unavailable("Git source context unavailable"))?;
        if frame.working_identity()?.as_deref() != Some(expected) {
            return Err(RuntimeError::conflict(
                "Working branch changed before delivery",
            ));
        }
    }
    Ok(())
}
fn expand_conflicts(
    store: &Store,
    context: &RuntimeContext,
    config: &RuntimeConfig,
    policy: &mut crate::source_policy::CurrentSourcePolicy,
    candidates: &mut Vec<Candidate>,
    excluded: &mut Vec<Exclusion>,
    mut trace: Option<&mut crate::observability::Trace>,
) -> Result<()> {
    let mut visited = candidates
        .iter()
        .filter_map(|item| item.document.record_id.clone())
        .collect::<BTreeSet<_>>();
    let mut pending = candidates
        .iter()
        .flat_map(|item| item.document.conflicts.clone())
        .collect::<BTreeSet<_>>();
    while let Some(id) = pending.pop_first() {
        if !visited.insert(id.clone()) {
            continue;
        }
        let sql = format!(
            "SELECT {DOCUMENT_SELECT} FROM documents d JOIN document_metadata m ON m.path=d.path WHERE m.record_id=?1 ORDER BY d.path"
        );
        let mut statement = store.conn.prepare(&sql)?;
        let documents = statement
            .query_map([&id], document_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if documents.is_empty() {
            excluded.push(Exclusion {
                path: format!("record:{id}"),
                reason: "conflict-unavailable".into(),
            });
        }
        for document in documents {
            let (mut kept, refused) = crate::source_policy::filter_in_scope(
                context,
                config,
                &store.conn,
                policy,
                vec![Candidate::linked(document, "conflict")],
                false,
                trace.as_deref_mut(),
            )?;
            excluded.extend(refused);
            for item in &kept {
                pending.extend(
                    item.document
                        .conflicts
                        .iter()
                        .filter(|id| !visited.contains(*id))
                        .cloned(),
                );
            }
            candidates.append(&mut kept);
        }
    }
    Ok(())
}
fn prune_manifests(directory: &Path, retention: usize) -> Result<()> {
    let mut files = std::fs::read_dir(directory)?
        .filter_map(std::result::Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    files.sort_by_key(|path| {
        std::fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()
    });
    let remove = files.len().saturating_sub(retention);
    for path in files.into_iter().take(remove) {
        if let Ok(metadata) = std::fs::symlink_metadata(&path)
            && metadata.is_file()
            && !fs::linked_at(&path, &metadata)?
        {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}
