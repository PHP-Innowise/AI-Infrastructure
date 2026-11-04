//! Bounded authorized observation occurrences. These never decide retrieval.
use crate::{
    Result, RuntimeConfig, RuntimeContext, RuntimeError,
    bank::publication::trace::TraceFiles,
    capsule::{self, Capsule, DeliveryProfile},
    domain::{self, json},
    index::Exclusion,
    query::{self, Candidate},
};
use serde_json::{Value, json as object};
use std::collections::{BTreeMap, BTreeSet};
pub const RECORD_LIMIT: usize = 128;
pub const EVENT_LIMIT: usize = 16;
pub const WATCH_LIMIT: usize = 16;
pub const FILE_LIMIT: usize = 512 * 1024;
pub const COUNT_LIMIT: u64 = 65535;
pub const BODY_LIMIT: u64 = 4096;
pub const STAGES: &[(&str, &[&str])] = &[
    ("search", &["fetched"]),
    ("admission", &["term-coverage", "admitted"]),
    (
        "ranking",
        &["admitted", "relative-score-floor", "authorized-window-rank"],
    ),
    ("history", &["admitted"]),
    (
        "runtime",
        &[
            "admitted",
            "source-changed-admitted",
            "authority",
            "lifecycle",
            "stale",
            "map-unverifiable",
            "map-drift",
            "source-missing",
            "promoted-to-chunk",
            "working-task",
            "host-loaded",
            "path-link-limit",
            "file-link-limit",
        ],
    ),
    (
        "source-link",
        &[
            "hydrated",
            "anchor-conflict",
            "already-seen",
            "category",
            "conflict",
            "candidate-limit",
        ],
    ),
    (
        "file-link",
        &[
            "hydrated",
            "category",
            "conflict",
            "touch-without-query-term",
            "hub-limit",
            "already-selected-or-own",
            "pool-limit",
        ],
    ),
    ("shared-source", &["hydrated", "no-current-shared-revision"]),
    (
        "queue",
        &[
            "lexical",
            "path-link",
            "source-link",
            "prompt-link",
            "touch-link",
            "source-hint",
        ],
    ),
    ("budget", &["admitted", "budget", "hard-budget"]),
    (
        "layer",
        &[
            "episodic-layer",
            "skill-subfile",
            "host-listed",
            "layer-limit",
            "delivered-this-session",
        ],
    ),
    ("gate", &["withheld"]),
    ("capsule", &["capsule-limit"]),
    ("delivery", &["selected"]),
];
pub const BOUNDARIES: &[&str] = &[
    "queue_after_runtime",
    "budget_admitted",
    "layer_pre_novelty",
    "repeat_withheld",
    "gate_input",
    "gate_withheld",
    "pack_input",
    "pack_omitted",
    "selected_post_pack",
];
pub const RENDER_REASONS: &[&str] = &["excerpt", "render-limit", "sanitized-empty", "no-excerpt"];
pub const LOSSES: &[(&str, &str, &str)] = &[
    ("term_coverage", "admission", "term-coverage"),
    ("relative_score_floor", "ranking", "relative-score-floor"),
    ("budget", "budget", "budget"),
    ("hard_budget", "budget", "hard-budget"),
    ("layer", "layer", "layer-limit"),
    ("repeat", "layer", "delivered-this-session"),
    ("gate", "gate", "withheld"),
    ("pack", "capsule", "capsule-limit"),
    ("authority", "runtime", "authority"),
    ("lifecycle", "runtime", "lifecycle"),
    ("stale", "runtime", "stale"),
    ("source_missing", "runtime", "source-missing"),
    ("map_unverifiable", "runtime", "map-unverifiable"),
    ("map_drift", "runtime", "map-drift"),
];
pub fn counter(count: Option<u64>) -> Value {
    object!({"count":count,"state":if count.is_some(){"exact"}else{"unknown"}})
}
fn exact_keys(value: &Value, keys: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|map| map.len() == keys.len() && keys.iter().all(|key| map.contains_key(*key)))
}
fn same_keys(value: &Value, template: &Value) -> bool {
    match (value.as_object(), template.as_object()) {
        (Some(a), Some(b)) => a.len() == b.len() && a.keys().all(|key| b.contains_key(key)),
        _ => false,
    }
}
pub fn valid_counter(value: &Value) -> bool {
    exact_keys(value, &["count", "state"])
        && (value["state"] == "unknown" && value["count"].is_null()
            || value["state"]
                .as_str()
                .is_some_and(|state| ["exact", "lower-bound"].contains(&state))
                && value["count"].as_u64().is_some_and(|count| {
                    count <= COUNT_LIMIT
                        && (value["state"] != "lower-bound" || count == COUNT_LIMIT)
                }))
}
fn increment(value: &mut Value) {
    if let Some(count) = value["count"].as_u64() {
        if count < COUNT_LIMIT {
            value["count"] = object!(count + 1);
        } else {
            value["state"] = "lower-bound".into();
        }
    }
}
fn counters<'a>(names: impl IntoIterator<Item = &'a str>, count: Option<u64>) -> Value {
    Value::Object(
        names
            .into_iter()
            .map(|name| (name.into(), counter(count)))
            .collect(),
    )
}
pub fn funnel_template(active: bool) -> Value {
    let events = Value::Object(
        STAGES
            .iter()
            .map(|(stage, reasons)| {
                (
                    (*stage).into(),
                    counters(reasons.iter().copied(), if active { Some(0) } else { None }),
                )
            })
            .collect(),
    );
    object!({"schema_version":1,"observer_state":if active{"active"}else{"unavailable"},
        "scope":"authorized-metadata-observation-occurrences","absence":"unknown",
        "coverage":{"bounded_channels":"authorized-observations-only","history_pre_admission":"not-instrumented","episodes":"not-instrumented","working_sources":"not-instrumented","path_only_events":"authorized-retained-records-only"},
        "events":events,"boundaries":counters(BOUNDARIES.iter().copied(),None),
        "render":{"projection_state":"unknown","scope":"capsule_projection","authorization_scope":"authorized-retained-records","emission":"unknown","consumer":"unknown","headers":counter(None),"reasons":counters(RENDER_REASONS.iter().copied(),None)}})
}
pub fn valid_funnel(value: &Value) -> bool {
    let template = funnel_template(true);
    if !same_keys(value, &template)
        || value["schema_version"].as_u64() != Some(1)
        || !value["observer_state"]
            .as_str()
            .is_some_and(|state| ["active", "unavailable"].contains(&state))
        || ["scope", "absence", "coverage"]
            .iter()
            .any(|key| value[*key] != template[*key])
    {
        return false;
    }
    if !same_keys(&value["events"], &template["events"])
        || !same_keys(&value["boundaries"], &template["boundaries"])
    {
        return false;
    }
    for (stage, _) in STAGES {
        let group = &value["events"][stage];
        if !same_keys(group, &template["events"][stage])
            || !group
                .as_object()
                .is_some_and(|group| group.values().all(valid_counter))
        {
            return false;
        }
    }
    let render = &value["render"];
    value["boundaries"]
        .as_object()
        .is_some_and(|group| group.values().all(valid_counter))
        && same_keys(render, &template["render"])
        && render["projection_state"]
            .as_str()
            .is_some_and(|state| ["unknown", "observed"].contains(&state))
        && ["scope", "authorization_scope", "emission", "consumer"]
            .iter()
            .all(|key| render[*key] == template["render"][*key])
        && valid_counter(&render["headers"])
        && same_keys(&render["reasons"], &template["render"]["reasons"])
        && render["reasons"]
            .as_object()
            .is_some_and(|group| group.values().all(valid_counter))
}
pub fn with_render(funnel: &Value, records: &[Value]) -> Value {
    if !valid_funnel(funnel) {
        return funnel_template(false);
    }
    let mut result = funnel.clone();
    result["render"] = funnel_template(true)["render"].clone();
    let mut counts = counters(RENDER_REASONS.iter().copied(), Some(0));
    let mut headers = counter(Some(0));
    for record in records {
        let Some(reason) = record["body_reason"]
            .as_str()
            .filter(|reason| RENDER_REASONS.contains(reason))
        else {
            return result;
        };
        let Some(length) = record["body_bytes"].as_u64() else {
            return result;
        };
        if record["header_present"] != true
            || record["pointer_only"].as_bool() != Some(length == 0)
            || (reason == "excerpt") != (length > 0)
        {
            return result;
        }
        increment(&mut headers);
        increment(&mut counts[reason]);
    }
    result["render"]["projection_state"] = "observed".into();
    result["render"]["headers"] = headers;
    result["render"]["reasons"] = counts;
    result
}
pub fn unknown_ordinary_receipt() -> Value {
    object!({"schema_version":1,"observation_state":"unavailable","plans_present":null,"returned_status":"unknown"})
}
pub fn normalize_ordinary_receipt(value: &Value) -> Option<Value> {
    if !exact_keys(
        value,
        &[
            "schema_version",
            "observation_state",
            "plans_present",
            "returned_status",
        ],
    ) || value["schema_version"].as_u64() != Some(1)
    {
        return None;
    }
    let state = value["observation_state"].as_str()?;
    let status = value["returned_status"].as_str()?;
    match state {
        "unavailable" if value["plans_present"].is_null() && status == "unknown" => {}
        "observed"
            if value["plans_present"].as_bool().is_some_and(|plans| {
                ["skipped", "deferred", "committed"].contains(&status)
                    && (plans || status == "skipped")
            }) => {}
        _ => return None,
    }
    Some(
        object!({"schema_version":1,"observation_state":state,"plans_present":value["plans_present"],"returned_status":status}),
    )
}
pub fn observe_ordinary_receipt(status: &str, plans_present: bool) -> Value {
    normalize_ordinary_receipt(&object!({"schema_version":1,"observation_state":"observed","plans_present":plans_present,"returned_status":status})).unwrap_or_else(unknown_ordinary_receipt)
}
pub fn unknown_funnel_diagnosis() -> Value {
    object!({"schema_version":1,"observation_state":"unavailable","source_coverage":"unknown","quality":"unmeasured",
        "count_scope":"authorized-observation-occurrences","coverage":{"history_pre_admission":"not-instrumented","episodes":"not-instrumented","working_sources":"not-instrumented","path_only_events":"authorized-retained-records-only"},
        "loss_observations":counters(LOSSES.iter().map(|(key,_,_)|*key),None),"boundaries":counters(BOUNDARIES.iter().copied(),None),
        "render_projection":{"scope":"authorized-retained-records","headers":counter(None),"reasons":counters(RENDER_REASONS.iter().copied(),None)},
        "emission":"unknown","consumer":"unknown","next_action":"unknown"})
}
pub fn valid_funnel_diagnosis(value: &Value) -> bool {
    let template = unknown_funnel_diagnosis();
    if !same_keys(value, &template)
        || value["schema_version"].as_u64() != Some(1)
        || !value["observation_state"]
            .as_str()
            .is_some_and(|state| ["observed", "unavailable"].contains(&state))
        || !value["next_action"]
            .as_str()
            .is_some_and(|action| ["unknown", "inspect-observed-loss"].contains(&action))
        || [
            "source_coverage",
            "quality",
            "count_scope",
            "coverage",
            "emission",
            "consumer",
        ]
        .iter()
        .any(|key| value[*key] != template[*key])
    {
        return false;
    }
    for key in ["loss_observations", "boundaries"] {
        if !same_keys(&value[key], &template[key])
            || !value[key]
                .as_object()
                .is_some_and(|group| group.values().all(valid_counter))
        {
            return false;
        }
    }
    let render = &value["render_projection"];
    if !same_keys(render, &template["render_projection"])
        || render["scope"] != template["render_projection"]["scope"]
        || !valid_counter(&render["headers"])
        || !same_keys(
            &render["reasons"],
            &template["render_projection"]["reasons"],
        )
        || !render["reasons"]
            .as_object()
            .is_some_and(|group| group.values().all(valid_counter))
    {
        return false;
    }
    if value["observation_state"] == "unavailable" {
        return value == &template;
    }
    let loss = value["loss_observations"].as_object().is_some_and(|group| {
        group
            .values()
            .any(|item| item["count"].as_u64().is_some_and(|count| count > 0))
    });
    value["next_action"]
        == if loss {
            "inspect-observed-loss"
        } else {
            "unknown"
        }
}
pub fn safe_path(path: &str) -> bool {
    if path.is_empty()
        || path.chars().count() > 512
        || path.chars().any(|c| c < ' ' || c == '\u{7f}')
        || path.contains('#')
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return false;
    }
    match domain::source_path_problem(path) {
        None => true,
        Some("Cite canonical project sources, not private runtime state or derived memory") => {
            (path.starts_with("memory-bank/chunks/") || path.starts_with("project-brain/dynamic/"))
                && domain::source_path_problem(&format!("trace/{path}")).is_none()
        }
        _ => false,
    }
}
pub fn hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
pub fn trace_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                *byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)
            }
        })
}
fn manifest_id(capsule: &Capsule, strict: bool) -> Option<String> {
    let path = capsule.0["manifest"].as_str()?;
    let name = path.rsplit('/').next()?.strip_suffix(".json")?;
    if !trace_id(name) {
        return None;
    }
    if strict
        && ![
            format!("project-brain/control/retrieval-manifests/{name}.json"),
            format!("memory-bank/local/retrieval-manifests/{name}.json"),
        ]
        .contains(&path.to_string())
    {
        return None;
    }
    Some(name.into())
}

#[derive(Clone, Debug)]
pub struct Trace {
    pub config: RuntimeConfig,
    pub records: BTreeMap<String, Value>,
    pub order: Vec<String>,
    pub truncated: bool,
    pub search_limit: Option<usize>,
    pub evidence_terms: usize,
    pub required_terms: Option<usize>,
    pub funnel: Value,
    pub ordinary_receipt: Value,
}
impl Trace {
    pub fn new(config: RuntimeConfig, paths: &[String]) -> Result<Self> {
        if paths.len() > WATCH_LIMIT {
            return Err(RuntimeError::invalid(
                "--trace-path accepts at most 16 paths",
            ));
        }
        if paths.iter().any(|path| !safe_path(path)) {
            return Err(RuntimeError::invalid(
                "--trace-path requires a safe project-relative document path (max 512 characters)",
            ));
        }
        let mut records = BTreeMap::new();
        let mut order = vec![];
        for path in paths {
            if !records.contains_key(path) {
                order.push(path.clone());
                records.insert(
                    path.clone(),
                    object!({"path":path,"state":"unavailable","events":[]}),
                );
            }
        }
        Ok(Self {
            config,
            records,
            order,
            truncated: false,
            search_limit: None,
            evidence_terms: 0,
            required_terms: None,
            funnel: funnel_template(true),
            ordinary_receipt: Value::Null,
        })
    }
    fn eligible_fields(&self, path: &str, privacy: &str, owner: &str) -> bool {
        safe_path(path)
            && self.config.value["allowed_privacy"]
                .as_array()
                .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(privacy)))
            && (privacy != "restricted"
                || self
                    .config
                    .value
                    .get("owners")
                    .map(|owners| {
                        owners.as_array().is_some_and(|owners| {
                            owners
                                .iter()
                                .any(|value| value == "*" || value.as_str() == Some(owner))
                        })
                    })
                    .unwrap_or(true))
    }
    pub fn eligible_document(&self, item: &crate::index::Document) -> bool {
        self.eligible_fields(&item.path, &item.privacy, &item.owner)
    }
    pub fn eligible(&self, item: &Candidate) -> bool {
        self.eligible_document(&item.document)
    }
    fn record_fields(
        &mut self,
        path: &str,
        privacy: &str,
        owner: &str,
        digest: &str,
    ) -> Option<&mut Value> {
        if !self.eligible_fields(path, privacy, owner) {
            return None;
        }
        if !self.records.contains_key(path) {
            if self.records.len() >= RECORD_LIMIT {
                self.truncated = true;
                return None;
            }
            self.order.push(path.into());
            self.records.insert(
                path.into(),
                object!({"path":path,"state":"not_observed_in_bounded_search","events":[]}),
            );
        }
        let record = self.records.get_mut(path)?;
        if record["state"] == "unavailable" {
            record["state"] = "not_observed_in_bounded_search".into();
        }
        if hex(digest) {
            record["source_sha256"] = digest.into();
        }
        Some(record)
    }
    pub fn watch(&mut self, connection: &rusqlite::Connection) -> Result<()> {
        for path in self.order.clone() {
            let mut statement = connection.prepare(
                "SELECT path,privacy,owner,source_hash FROM document_metadata WHERE path=?1",
            )?;
            let rows = statement.query_map([&path], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            for row in rows {
                let (path, privacy, owner, digest) = row?;
                self.record_fields(&path, &privacy, &owner, &digest);
            }
        }
        Ok(())
    }
    pub fn coverage(
        &mut self,
        evidence_terms: usize,
        required_terms: usize,
        coverage: &BTreeMap<String, usize>,
    ) {
        self.evidence_terms = evidence_terms;
        self.required_terms = Some(required_terms);
        for (path, record) in &mut self.records {
            if record["state"] != "unavailable" {
                record["evidence_matched_terms"] =
                    object!(coverage.get(path).copied().unwrap_or(0));
            }
        }
    }
    pub fn observe_document(
        &mut self,
        item: &crate::index::Document,
        stage: &str,
        reason: &str,
        numbers: &Value,
    ) {
        if self.eligible_document(item)
            && STAGES
                .iter()
                .any(|(key, reasons)| *key == stage && reasons.contains(&reason))
        {
            increment(&mut self.funnel["events"][stage][reason]);
        }
        let Some(record) =
            self.record_fields(&item.path, &item.privacy, &item.owner, &item.source_hash)
        else {
            return;
        };
        record["state"] = "observed".into();
        let mut event = object!({"stage":stage,"reason":reason});
        for key in [
            "rank",
            "matched_terms",
            "required_terms",
            "score",
            "threshold",
            "estimated_tokens",
        ] {
            if let Some(number) = numbers.get(key) {
                if number.is_i64() || number.is_u64() {
                    event[key] = number.clone();
                } else if let Some(value) = number.as_f64().filter(|value| value.is_finite()) {
                    let rounded = format!("{value:.6}").parse::<f64>().unwrap_or(value);
                    event[key] = object!(rounded);
                }
            }
        }
        if let Some(events) = record["events"].as_array_mut() {
            if events.len() < EVENT_LIMIT {
                events.push(event);
            } else {
                if stage == "delivery" {
                    events[EVENT_LIMIT - 1] = event;
                }
                record["events_truncated"] = true.into();
            }
        }
    }
    pub fn observe(&mut self, item: &Candidate, stage: &str, reason: &str, numbers: &Value) {
        self.observe_document(&item.document, stage, reason, numbers)
    }
    pub fn exclusions(&mut self, entries: &[Exclusion], stage: &str) {
        for entry in entries {
            let Some(record) = self
                .records
                .get_mut(&entry.path)
                .filter(|record| record["state"] != "unavailable")
            else {
                continue;
            };
            let duplicate = record["events"]
                .as_array()
                .and_then(|events| events.last())
                .is_some_and(|last| last["stage"] == stage && last["reason"] == entry.reason);
            if duplicate {
                continue;
            }
            if STAGES
                .iter()
                .any(|(key, reasons)| *key == stage && reasons.contains(&entry.reason.as_str()))
            {
                increment(&mut self.funnel["events"][stage][entry.reason.as_str()]);
            }
            let event = object!({"stage":stage,"reason":entry.reason});
            if let Some(events) = record["events"].as_array_mut() {
                if events.len() < EVENT_LIMIT {
                    events.push(event);
                } else {
                    record["events_truncated"] = true.into();
                }
            }
        }
    }
    pub fn known_event(&mut self, path: &str, stage: &str, reason: &str) {
        if let Some(record) = self
            .records
            .get_mut(path)
            .filter(|record| record["state"] != "unavailable")
        {
            record["state"] = "observed".into();
            self.exclusions(
                &[Exclusion {
                    path: path.into(),
                    reason: reason.into(),
                }],
                stage,
            );
        }
    }
    pub fn boundary(&mut self, name: &str, items: &[Candidate]) {
        if !BOUNDARIES.contains(&name) {
            return;
        }
        let mut value = counter(Some(0));
        for item in items {
            if self.eligible(item) {
                increment(&mut value);
            }
        }
        self.funnel["boundaries"][name] = value;
    }
    pub fn snapshot(&self) -> Value {
        let mut result = object!({"schema_version":1,"funnel":if valid_funnel(&self.funnel){self.funnel.clone()}else{funnel_template(false)},
            "records":self.order.iter().filter_map(|path|self.records.get(path)).collect::<Vec<_>>(),"truncated":self.truncated,
            "search_limit":self.search_limit,"evidence_terms":self.evidence_terms,"required_terms":self.required_terms,
            "rank_scope":"authorized records in observed window; protected positions are omitted",
            "bounds":{"records":RECORD_LIMIT,"events_per_record":EVENT_LIMIT,"watched_paths":WATCH_LIMIT},
            "coverage":"existing bounded candidate windows only; unobserved causes are unknown"});
        if let Some(receipt) = normalize_ordinary_receipt(&self.ordinary_receipt) {
            result["ordinary_receipt"] = receipt;
        }
        result
    }
    pub fn save(&self, context: &RuntimeContext, id: &str) {
        let operation = (|| -> Result<()> {
            if !trace_id(id) {
                return Ok(());
            }
            let mut files = TraceFiles::new(&context.roots.state)?;
            let mut value = self.snapshot();
            value["id"] = id.into();
            value["created_at"] = domain::utc_now().into();
            let bytes = trace_payload_json(&value)?;
            if bytes.len() > FILE_LIMIT {
                return Ok(());
            }
            files.save(id, &bytes, FILE_LIMIT)?;
            files.prune(
                id,
                self.config.value["local_manifest_retention"]
                    .as_u64()
                    .unwrap_or(200)
                    .max(1) as usize,
            )?;
            files.close()?;
            Ok(())
        })();
        let _ = operation;
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "Repeat-window observation records the exact key/revision, turn, retained receipt and prepared proposal without rerendering sources"
    )]
    pub fn repeat_window(
        &mut self,
        context: &RuntimeContext,
        item: &Candidate,
        key: &str,
        revision: &str,
        seen: &Value,
        turn: u64,
        previous: &Value,
        proposal: &Value,
    ) {
        if self
            .record_fields(
                &item.document.path,
                &item.document.privacy,
                &item.document.owner,
                &item.document.source_hash,
            )
            .is_none()
        {
            return;
        }
        let mut observation =
            object!({"classification":"unknown-proposal","scope":"document-pre-novelty-pre-pack"});
        let valid = exact_keys(proposal, &["sha256", "bytes", "state"])
            && proposal["sha256"].as_str().is_some_and(hex)
            && proposal["bytes"]
                .as_u64()
                .is_some_and(|length| length <= BODY_LIMIT)
            && proposal["state"]
                .as_str()
                .is_some_and(|state| ["excerpt", "sanitized-empty", "no-excerpt"].contains(&state))
            && (proposal["state"] == "excerpt")
                == proposal["bytes"].as_u64().is_some_and(|length| length > 0)
            && (proposal["bytes"] != 0 || proposal["sha256"] == query::sha256(&[]));
        if valid {
            let length = proposal["bytes"].as_u64().unwrap_or(0);
            observation = object!({"classification":if length==0{"empty-proposal"}else{"unknown-prior-render"},"scope":"document-pre-novelty-pre-pack","proposal_sha256":proposal["sha256"],"proposal_bytes":length,"proposal_state":proposal["state"],"receipt_age":turn.saturating_sub(seen[1].as_u64().unwrap_or(turn))});
            if length > 0 {
                let refs = crate::receipts::window_references(previous, &previous["items"]);
                if let Some(reference) =
                    refs.get(&query::sha256(key.as_bytes()))
                        .filter(|reference| {
                            reference["revision"] == revision && reference["handed_turn"] == seen[1]
                        })
                    && let Some(mut prior) = read_window(context, reference, &item.document.path)
                {
                    let source = prior
                        .as_object_mut()
                        .and_then(|object| object.remove("source_sha256"));
                    observation["source_relation"] = object!(match source
                        .as_ref()
                        .and_then(Value::as_str)
                        .filter(|source| hex(source))
                    {
                        Some(source) if hex(&item.document.source_hash) =>
                            if source == item.document.source_hash {
                                "same-source"
                            } else {
                                "changed-source"
                            },
                        _ => "unknown-source",
                    });
                    observation["classification"] = object!(if prior["body_sha256"]
                        == proposal["sha256"]
                        && prior["body_bytes"] == proposal["bytes"]
                    {
                        "same-window"
                    } else {
                        "different-exposure-window"
                    });
                    observation["prior"] = prior;
                }
            }
        }
        if let Some(record) = self.records.get_mut(&item.document.path) {
            record["window_observation"] = observation;
        }
    }
}
pub fn trace_payload_json(payload: &Value) -> Result<Vec<u8>> {
    let mut value = payload.clone();
    if let Some(map) = value.as_object_mut()
        && let Some(receipt) = map
            .remove("ordinary_receipt")
            .and_then(|receipt| normalize_ordinary_receipt(&receipt))
    {
        map.insert("ordinary_receipt".into(), receipt);
    }
    let encode = |value: &Value| -> Result<Vec<u8>> {
        let mut text = json::sorted_pretty(value)?.into_bytes();
        text.push(b'\n');
        Ok(text)
    };
    let mut bytes = encode(&value)?;
    if bytes.len() > FILE_LIMIT {
        if let Some(map) = value.as_object_mut() {
            map.remove("ordinary_receipt");
        }
        bytes = encode(&value)?;
    }
    if bytes.len() > FILE_LIMIT {
        if let Some(map) = value.as_object_mut() {
            map.remove("funnel");
        }
        bytes = encode(&value)?;
    }
    Ok(bytes)
}

fn render_trace(capsule: &Capsule, rendered: &str, form: &str) -> Option<Value> {
    if !["capsule_text", "printed_capsule"].contains(&form) {
        return None;
    }
    for profile in [DeliveryProfile::Ordinary, DeliveryProfile::CursorRule] {
        let plan = match capsule::render_plan(capsule, profile) {
            Ok(plan) => plan,
            Err(_) => continue,
        };
        let actual = if form == "printed_capsule" {
            format!("{}\n", plan.text)
        } else {
            plan.text
        };
        if actual == rendered {
            return Some(
                object!({"sha256":query::sha256(rendered.as_bytes()),"bytes":rendered.len(),"form":form,"records":plan.records}),
            );
        }
    }
    None
}
pub fn render_records(capsule: &Capsule, rendered: &str) -> Vec<Value> {
    render_trace(capsule, rendered, "capsule_text")
        .and_then(|render| render["records"].as_array().cloned())
        .unwrap_or_default()
}
pub fn record_render(context: &RuntimeContext, capsule: &Capsule, rendered: &str, form: &str) {
    if let Some(render) = render_trace(capsule, rendered, form) {
        record_render_metadata(context, &capsule.0, &render);
    }
}
/// The actual emitter passes metadata computed before its typed held images were
/// released. No row, source body or public JSON capsule is rehydrated here.
pub fn record_render_metadata(context: &RuntimeContext, capsule: &Value, render: &Value) {
    let operation = (|| -> Result<()> {
        let id = manifest_id(&Capsule::new(capsule.clone()), false)
            .ok_or_else(|| RuntimeError::unavailable("Render trace reference unavailable"))?;
        if !render["sha256"].as_str().is_some_and(hex)
            || !render["bytes"]
                .as_u64()
                .is_some_and(|length| length <= FILE_LIMIT as u64)
            || !render["form"]
                .as_str()
                .is_some_and(|form| ["capsule_text", "printed_capsule"].contains(&form))
            || render["records"]
                .as_array()
                .is_none_or(|records| records.len() > RECORD_LIMIT)
        {
            return Ok(());
        }
        let mut files = TraceFiles::new(&context.roots.state)?;
        files.update_existing(&id,FILE_LIMIT,|previous|{
            let mut data=json::parse(std::str::from_utf8(previous).map_err(|_|RuntimeError::invalid("Invalid trace encoding"))?)?;
            let allowed=data["records"].as_array().ok_or_else(||RuntimeError::invalid("Invalid trace records"))?.iter().filter(|item|item["state"]!="unavailable").filter_map(|item|item["path"].as_str()).collect::<BTreeSet<_>>();
            let records=render["records"].as_array().ok_or_else(||RuntimeError::invalid("Invalid render records"))?.iter().filter(|item|item["path"].as_str().is_some_and(|path|allowed.contains(path))).cloned().collect::<Vec<_>>();
            data["render"]=object!({"sha256":render["sha256"],"bytes":render["bytes"],"form":render["form"],"records":records});
            if data.get("funnel").is_some(){data["funnel"]=with_render(&data["funnel"],&records);}
            let bytes=trace_payload_json(&data)?;Ok((bytes.len()<=FILE_LIMIT).then_some(bytes))
        })?;
        files.close()?;
        Ok(())
    })();
    let _ = operation;
}
pub fn diagnose(context: &RuntimeContext, capsule: &Capsule, expected: &Value) -> Value {
    diagnose_with_receipt(context, capsule, expected).0
}
pub fn diagnose_with_receipt(
    context: &RuntimeContext,
    capsule: &Capsule,
    expected: &Value,
) -> (Value, Value) {
    let mut receipt = unknown_ordinary_receipt();
    let operation = (|| -> Result<Value> {
        let id = manifest_id(capsule, true)
            .ok_or_else(|| RuntimeError::unavailable("Trace unavailable"))?;
        let form = expected["form"]
            .as_str()
            .filter(|form| ["capsule_text", "printed_capsule"].contains(form))
            .ok_or_else(|| RuntimeError::unavailable("Trace unavailable"))?;
        let length = expected["bytes"]
            .as_u64()
            .filter(|length| *length <= FILE_LIMIT as u64)
            .ok_or_else(|| RuntimeError::unavailable("Trace unavailable"))?;
        let digest = expected["sha256"]
            .as_str()
            .filter(|digest| hex(digest))
            .ok_or_else(|| RuntimeError::unavailable("Trace unavailable"))?;
        let mut files = TraceFiles::new(&context.roots.state)?;
        let bytes = files
            .read(&id, FILE_LIMIT, true)?
            .ok_or_else(|| RuntimeError::unavailable("Trace unavailable"))?;
        files.close()?;
        let value = crate::results::strict_json(&bytes)?;
        if value["schema_version"].as_u64() != Some(1)
            || value["id"] != id
            || value["render"]["form"] != form
            || value["render"]["bytes"].as_u64() != Some(length)
            || value["render"]["sha256"] != digest
        {
            return Err(RuntimeError::unavailable("Trace unavailable"));
        }
        if let Some(observed) = normalize_ordinary_receipt(&value["ordinary_receipt"]) {
            receipt = observed;
        }
        let funnel = &value["funnel"];
        if !valid_funnel(funnel)
            || funnel["observer_state"] != "active"
            || funnel["render"]["projection_state"] != "observed"
        {
            return Err(RuntimeError::unavailable("Trace unavailable"));
        }
        let mut result = unknown_funnel_diagnosis();
        result["observation_state"] = "observed".into();
        for (key, stage, reason) in LOSSES {
            result["loss_observations"][*key] = funnel["events"][*stage][*reason].clone();
        }
        result["boundaries"] = funnel["boundaries"].clone();
        result["render_projection"]["headers"] = funnel["render"]["headers"].clone();
        result["render_projection"]["reasons"] = funnel["render"]["reasons"].clone();
        if result["loss_observations"]
            .as_object()
            .is_some_and(|losses| {
                losses
                    .values()
                    .any(|counter| counter["count"].as_u64().is_some_and(|count| count > 0))
            })
        {
            result["next_action"] = "inspect-observed-loss".into();
        }
        if valid_funnel_diagnosis(&result) {
            Ok(result)
        } else {
            Err(RuntimeError::unavailable("Trace unavailable"))
        }
    })();
    (
        operation.unwrap_or_else(|_| unknown_funnel_diagnosis()),
        receipt,
    )
}
fn read_window(context: &RuntimeContext, reference: &Value, path: &str) -> Option<Value> {
    let operation = (|| -> Result<Value> {
        let id = reference["manifest_id"]
            .as_str()
            .filter(|id| trace_id(id))
            .ok_or_else(|| RuntimeError::unavailable("Window unavailable"))?;
        if reference["doc_ref"] != query::sha256(path.as_bytes()) {
            return Err(RuntimeError::unavailable("Window unavailable"));
        }
        let mut files = TraceFiles::new(&context.roots.state)?;
        let raw = files
            .read(id, FILE_LIMIT, false)?
            .ok_or_else(|| RuntimeError::unavailable("Window unavailable"))?;
        files.close()?;
        let data = crate::results::strict_json(&raw)?;
        if data["id"] != id
            || !data["render"]["form"]
                .as_str()
                .is_some_and(|form| ["capsule_text", "printed_capsule"].contains(&form))
        {
            return Err(RuntimeError::unavailable("Window unavailable"));
        }
        let records = data["records"]
            .as_array()
            .filter(|records| records.len() <= RECORD_LIMIT)
            .ok_or_else(|| RuntimeError::unavailable("Window unavailable"))?;
        let sources = records
            .iter()
            .filter(|item| item["path"] == path)
            .collect::<Vec<_>>();
        if sources.len() != 1 || sources[0]["state"] != "observed" {
            return Err(RuntimeError::unavailable("Window unavailable"));
        }
        let events = sources[0]["events"]
            .as_array()
            .filter(|events| events.len() <= EVENT_LIMIT)
            .ok_or_else(|| RuntimeError::unavailable("Window unavailable"))?;
        if !events
            .iter()
            .any(|event| event["stage"] == "delivery" && event["reason"] == "selected")
        {
            return Err(RuntimeError::unavailable("Window unavailable"));
        }
        let bodies = data["render"]["records"]
            .as_array()
            .filter(|records| records.len() <= RECORD_LIMIT)
            .ok_or_else(|| RuntimeError::unavailable("Window unavailable"))?;
        let bodies = bodies
            .iter()
            .filter(|item| item["path"] == path)
            .collect::<Vec<_>>();
        if bodies.len() != 1 {
            return Err(RuntimeError::unavailable("Window unavailable"));
        }
        let body = bodies[0];
        let length = body["body_bytes"]
            .as_u64()
            .filter(|length| *length <= BODY_LIMIT)
            .ok_or_else(|| RuntimeError::unavailable("Window unavailable"))?;
        let digest = body["body_sha256"]
            .as_str()
            .filter(|digest| hex(digest))
            .ok_or_else(|| RuntimeError::unavailable("Window unavailable"))?;
        let reason = body["body_reason"]
            .as_str()
            .filter(|reason| RENDER_REASONS.contains(reason))
            .ok_or_else(|| RuntimeError::unavailable("Window unavailable"))?;
        if body["header_present"] != true
            || body["pointer_only"].as_bool() != Some(length == 0)
            || !body["shrunk"].is_boolean()
            || (length > 0) != (reason == "excerpt")
            || length == 0 && digest != query::sha256(&[])
        {
            return Err(RuntimeError::unavailable("Window unavailable"));
        }
        let mut result = object!({"body_bytes":length,"body_sha256":digest,"body_reason":reason,"pointer_only":length==0,"shrunk":body["shrunk"]});
        if body["initial_excerpt_bytes"]
            .as_u64()
            .is_some_and(|initial| initial >= length && initial <= BODY_LIMIT)
        {
            result["initial_excerpt_bytes"] = body["initial_excerpt_bytes"].clone();
        }
        if sources[0]["source_sha256"].as_str().is_some_and(hex) {
            result["source_sha256"] = sources[0]["source_sha256"].clone();
        }
        Ok(result)
    })();
    operation.ok()
}
/// Local response provenance remains metadata only and is never publication proof.
pub fn source_provenance(capsule: &Capsule, manifest: &Value) -> Option<Value> {
    let operation = (|| -> Result<Value> {
        let visible = capsule.shown_paths();
        let mut observed = BTreeSet::new();
        for row in manifest["selected"]
            .as_array()
            .ok_or_else(|| RuntimeError::invalid("Incomplete provenance"))?
        {
            let path = row["path"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("Incomplete provenance"))?;
            if !observed.insert(path) {
                return Err(RuntimeError::invalid("Duplicate provenance path"));
            }
        }
        let rows = manifest["selected"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|row| {
                row["path"]
                    .as_str()
                    .is_some_and(|path| visible.contains(path))
            })
            .map(|row| object!({"path":row["path"],"source_hash":row["source_hash"]}))
            .collect::<Vec<_>>();
        if rows.len() > 32
            || rows
                .iter()
                .filter_map(|row| row["path"].as_str())
                .collect::<BTreeSet<_>>()
                != visible.iter().map(String::as_str).collect()
        {
            return Err(RuntimeError::invalid("Incomplete provenance"));
        }
        let frame = json::canonical(&capsule.0)?;
        let value = object!({"schema_version":1,"profile":"local-source-provenance-v1","scope":"local","publication":"unconfirmed","task_uuid":capsule.0["task_uuid"],"task_revision":capsule.0["task_revision"],"capsule_sha256":query::sha256(frame.as_bytes()),"selected":rows});
        if json::canonical(&value)?.len() > 65536 {
            return Err(RuntimeError::invalid("Provenance limit exceeded"));
        }
        Ok(value)
    })();
    operation.ok()
}

/// A command owns this metadata-only capability until its actual stdout write
/// succeeds. It contains no source body, query, process-local lookup or TLS.
#[derive(Clone, Debug)]
pub struct RenderFinalizer {
    context: RuntimeContext,
    capsule: Value,
    render: Value,
}
impl RenderFinalizer {
    pub fn from_projection(
        context: &RuntimeContext,
        capsule: &Capsule,
        text: &str,
        form: &str,
    ) -> Option<Self> {
        let render = render_trace(capsule, text, form)?;
        let reference = capsule.0["manifest"].as_str()?;
        Some(Self {
            context: context.clone(),
            capsule: object!({"manifest":reference}),
            render,
        })
    }
    pub fn finish(self) {
        record_render_metadata(&self.context, &self.capsule, &self.render);
    }
}
pub fn from_projection(
    context: &RuntimeContext,
    capsule: &Capsule,
    text: &str,
    form: &str,
) -> Option<RenderFinalizer> {
    RenderFinalizer::from_projection(context, capsule, text, form)
}
