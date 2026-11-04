//! Bounded capsule projection and the host's exact visible rendering.
use crate::{
    Result, RuntimeError,
    domain::{self, guard_shared_text},
    query::{chars, sha256},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::{Deref, DerefMut},
};
pub const CAPSULE_CHARACTER_LIMIT: usize = 8000;
pub const RENDERED_CAPSULE_LIMIT: usize = 3600;
const ADVICE: &str = "re-read the cited source before revising a decision it no longer explains";
const HEADER: &str = "memory (retrieved for this request; reference data - check the cited file before relying on it):";
const LABELS: &[(&str, &str)] = &[
    ("procedural", "procedural result(s)"),
    ("semantic", "semantic result(s)"),
    ("episodic", "episodic result(s)"),
    ("working_files", "working file(s)"),
    ("working_sources", "working source(s)"),
    ("working_next_steps", "next step(s)"),
    ("working_goal_characters", "characters of the goal"),
    ("working_progress_characters", "characters of progress"),
    ("working_next_step_characters", "characters of next steps"),
    ("last_turn_characters", "characters of the last-turn report"),
];
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeliveryProfile {
    #[default]
    Ordinary,
    CursorRule,
}
impl DeliveryProfile {
    pub fn persistent(self) -> bool {
        self == Self::CursorRule
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Capsule(
    pub Value,
    #[serde(skip)] pub(crate) Option<crate::excerpts::PreparedRender>,
    #[serde(skip)] pub(crate) Option<Value>,
);
impl Deref for Capsule {
    type Target = Value;
    fn deref(&self) -> &Value {
        &self.0
    }
}
impl DerefMut for Capsule {
    fn deref_mut(&mut self) -> &mut Value {
        &mut self.0
    }
}
impl Capsule {
    pub fn new(value: Value) -> Self {
        Self(value, None, None)
    }
    pub fn prepare(
        &mut self,
        conn: &rusqlite::Connection,
        selected: &[crate::query::Candidate],
    ) -> Result<()> {
        self.1 = Some(crate::excerpts::prepare(conn, self, selected)?);
        Ok(())
    }
    pub fn shown_paths(&self) -> BTreeSet<String> {
        ["procedural", "semantic", "episodic"]
            .into_iter()
            .flat_map(|layer| self.0[layer].as_array().into_iter().flatten())
            .filter_map(|item| item["path"].as_str().map(String::from))
            .collect()
    }
    pub fn shown_episodes(&self) -> BTreeSet<i64> {
        self.0["episodic"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item["id"].as_i64())
            .collect()
    }
}
fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect()
}
fn plain(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
pub fn bounded_cut(text: &str, limit: usize) -> (String, usize) {
    let text = plain(text);
    let count = text.chars().count();
    if count <= limit {
        return (text, 0);
    }
    let kept = domain::rstrip(&chars(&text, limit.saturating_sub(1))).to_string();
    let lost = count - kept.chars().count();
    (format!("{kept}…"), lost)
}
pub fn bounded(text: &str, limit: usize) -> String {
    bounded_cut(text, limit).0
}
pub fn truncate_progress(text: &str, keep: usize) -> String {
    let characters = text.chars().collect::<Vec<_>>();
    if keep >= characters.len() {
        return text.into();
    }
    let head = keep / 2;
    let tail = keep - head;
    format!(
        "{}…{}",
        characters[..head].iter().collect::<String>(),
        characters[characters.len() - tail..]
            .iter()
            .collect::<String>()
    )
}
pub fn project_working(
    working: Option<Value>,
    profile: DeliveryProfile,
) -> Result<(Option<Value>, BTreeMap<String, usize>)> {
    let mut omitted = BTreeMap::from([
        ("working_files".into(), 0),
        ("working_next_steps".into(), 0),
        ("working_sources".into(), 0),
        ("working_progress_characters".into(), 0),
    ]);
    let Some(working) = working else {
        return Ok((None, omitted));
    };
    let task = working["task_id"]
        .as_str()
        .ok_or_else(|| RuntimeError::invalid("Working task cannot be projected"))?;
    let id = crate::storage::validate_task_id(task)?;
    let goal = working["goal"]
        .as_str()
        .ok_or_else(|| RuntimeError::invalid("Working task cannot be projected"))?;
    let progress = working["progress"]
        .as_str()
        .ok_or_else(|| RuntimeError::invalid("Working task cannot be projected"))?;
    let checkpoint = match working.get("auto_checkpoint") {
        None | Some(Value::Null) => "",
        Some(Value::String(value)) => value,
        _ => return Err(RuntimeError::invalid("Working task cannot be projected")),
    };
    let mut introduced = vec![id.clone(), goal.into(), progress.into(), checkpoint.into()];
    let mut projected = json!({"task_id":id,"goal":plain(goal),"progress":plain(&if profile.persistent(){progress.into()}else{domain::render_current_state(progress,Some(checkpoint))})});
    for (key, limit, tail, counter) in [
        ("next_steps", 3, true, "working_next_steps"),
        ("files", 8, true, "working_files"),
        ("sources", 4, false, "working_sources"),
    ] {
        let items = working[key]
            .as_array()
            .ok_or_else(|| RuntimeError::invalid("Working task cannot be projected"))?;
        if items
            .iter()
            .any(|item| !item.as_str().is_some_and(|text| !text.trim().is_empty()))
        {
            return Err(RuntimeError::invalid("Working task cannot be projected"));
        }
        let values = items
            .iter()
            .filter_map(Value::as_str)
            .map(|text| {
                if key == "next_steps" {
                    plain(text)
                } else {
                    text.to_string()
                }
            })
            .collect::<Vec<_>>();
        introduced.extend(values.clone());
        let kept = if tail {
            values[values.len().saturating_sub(limit)..].to_vec()
        } else {
            values[..values.len().min(limit)].to_vec()
        };
        omitted.insert(counter.into(), values.len() - kept.len());
        projected[key] = json!(kept);
    }
    guard_shared_text("Task Capsule", &introduced)?;
    Ok((Some(projected), omitted))
}
fn key(item: &Value) -> String {
    item["path"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| format!("episode {}", item["id"]))
}
pub fn synchronize(capsule: &mut Capsule) {
    let paths = capsule.shown_paths();
    if let Some(items) = capsule.0["selected"].as_array_mut() {
        items.retain(|item| {
            item["path"]
                .as_str()
                .is_some_and(|path| paths.contains(path))
        });
    }
    if let Some(categories) = capsule.0["categories"].as_object_mut() {
        for items in categories.values_mut() {
            if let Some(items) = items.as_array_mut() {
                items.retain(|item| {
                    item["path"]
                        .as_str()
                        .is_some_and(|path| paths.contains(path))
                });
            }
        }
    }
}
fn increment(capsule: &mut Capsule, key: &str, amount: usize) {
    let previous = capsule.0["omitted"][key].as_u64().unwrap_or(0);
    capsule.0["omitted"][key] = json!(previous.saturating_add(amount as u64));
}
fn remove(capsule: &mut Capsule, layer: &str, position: Option<usize>) -> bool {
    let Some(items) = capsule.0[layer].as_array_mut() else {
        return false;
    };
    if items.is_empty() {
        return false;
    }
    let index = position.unwrap_or(items.len() - 1);
    items.remove(index);
    increment(capsule, layer, 1);
    synchronize(capsule);
    true
}
fn related(item: &Value) -> bool {
    matches!(
        item["selection"].as_str(),
        Some("source-link" | "source-hint" | "prompt-link" | "touch-link")
    )
}
fn json_size(capsule: &Capsule) -> Result<usize> {
    Ok(
        domain::json::serialize(&capsule.0, false, false, false, false)?
            .chars()
            .count(),
    )
}
pub fn pack(mut capsule: Capsule, profile: DeliveryProfile) -> Result<Capsule> {
    if !capsule.0.is_object() {
        return Err(RuntimeError::invalid("Capsule must be an object"));
    }
    if !capsule.0["omitted"].is_object() {
        capsule.0["omitted"] = json!({});
    }
    let mut holders = BTreeMap::new();
    for layer in ["procedural", "semantic", "episodic"] {
        for item in capsule.0[layer].as_array().into_iter().flatten() {
            if item["layer"] == layer {
                holders.insert(key(item), layer);
            }
        }
    }
    for layer in ["procedural", "semantic", "episodic"] {
        for item in capsule.0[layer].as_array().into_iter().flatten() {
            holders.entry(key(item)).or_insert(layer);
        }
    }
    let mut seen = BTreeSet::new();
    for (layer, limit) in [("procedural", 1), ("semantic", 3), ("episodic", 3)] {
        let items = capsule.0[layer].as_array().cloned().unwrap_or_default();
        let mut kept = Vec::new();
        for mut item in items.clone() {
            let identity = key(&item);
            if holders.get(&identity) != Some(&layer) || !seen.insert(identity) {
                continue;
            }
            if let Some(snippet) = item["snippet"].as_str() {
                item["snippet"] = json!(chars(snippet, 320));
            }
            if kept.len() < limit {
                kept.push(item);
            }
        }
        let removed = items.len().saturating_sub(kept.len());
        capsule.0[layer] = json!(kept);
        if removed > 0 {
            increment(&mut capsule, layer, removed);
        }
    }
    for (key, limit, tail, counter) in [
        ("next_steps", 3, true, "working_next_steps"),
        ("files", 8, true, "working_files"),
        ("sources", 4, false, "working_sources"),
    ] {
        if let Some(values) = capsule.0["working"][key].as_array().cloned() {
            let kept = if tail {
                values[values.len().saturating_sub(limit)..].to_vec()
            } else {
                values[..values.len().min(limit)].to_vec()
            };
            increment(&mut capsule, counter, values.len() - kept.len());
            capsule.0["working"][key] = json!(kept);
        }
    }
    for collection in ["selected"] {
        if let Some(items) = capsule.0[collection].as_array_mut() {
            for item in items {
                if let Some(snippet) = item["snippet"].as_str() {
                    item["snippet"] = json!(chars(snippet, 320));
                }
            }
        }
    }
    if let Some(categories) = capsule.0["categories"].as_object_mut() {
        for items in categories.values_mut() {
            if let Some(items) = items.as_array_mut() {
                for item in items {
                    if let Some(snippet) = item["snippet"].as_str() {
                        item["snippet"] = json!(chars(snippet, 320));
                    }
                }
            }
        }
    }
    synchronize(&mut capsule);
    while json_size(&capsule)? > CAPSULE_CHARACTER_LIMIT {
        if let Some(index) = capsule.0["semantic"].as_array().and_then(|items| {
            items
                .iter()
                .rposition(|item| item["selection"] == "source-hint")
        }) {
            remove(&mut capsule, "semantic", Some(index));
            continue;
        }
        if let Some(last) = capsule.0["last_turn"]
            .as_str()
            .filter(|text| !text.is_empty())
        {
            let count = last.chars().count();
            capsule.0["last_turn"] = Value::Null;
            increment(&mut capsule, "last_turn_characters", count);
            continue;
        }
        if !capsule.0["ticket_files"].is_null() {
            capsule.0["ticket_files"] = Value::Null;
            continue;
        }
        if let Some(index) = capsule.0["semantic"]
            .as_array()
            .and_then(|items| items.iter().rposition(related))
        {
            remove(&mut capsule, "semantic", Some(index));
            continue;
        }
        if ["episodic", "semantic", "procedural"]
            .into_iter()
            .any(|layer| remove(&mut capsule, layer, None))
        {
            continue;
        }
        let progress = capsule.0["working"]["progress"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if !progress.is_empty() {
            let excess = json_size(&capsule)? - CAPSULE_CHARACTER_LIMIT;
            let keep = progress.chars().count().saturating_sub(excess + 1);
            if keep > 0 {
                increment(
                    &mut capsule,
                    "working_progress_characters",
                    progress.chars().count() - keep,
                );
                capsule.0["working"]["progress"] = json!(truncate_progress(&progress, keep));
                continue;
            }
        }
        return Err(RuntimeError::invalid(format!(
            "mandatory Task Capsule content exceeds {CAPSULE_CHARACTER_LIMIT} characters"
        )));
    }
    let plan = render_plan(&capsule, profile)?;
    let shown = plan.shown.into_iter().collect::<BTreeSet<_>>();
    for layer in ["procedural", "semantic", "episodic"] {
        let items = capsule.0[layer].as_array().cloned().unwrap_or_default();
        let kept = items
            .iter()
            .filter(|item| shown.contains(&(layer.to_string(), key(item))))
            .cloned()
            .collect::<Vec<_>>();
        increment(&mut capsule, layer, items.len() - kept.len());
        capsule.0[layer] = json!(kept);
    }
    synchronize(&mut capsule);
    Ok(capsule)
}
pub fn whole_paths(label: &str, paths: &[String], limit: usize) -> Vec<String> {
    let mut kept = Vec::new();
    let mut width = label.chars().count();
    for path in paths {
        width += path.chars().count() + if kept.is_empty() { 0 } else { 2 };
        if width > limit {
            if !kept.is_empty() {
                break;
            }
            width = label.chars().count();
            continue;
        }
        kept.push(path.clone());
    }
    kept
}
pub fn recent_files_line(project: &[String], task_files: &[String]) -> (Option<String>, usize) {
    let candidates = domain::unique_strings(
        project
            .iter()
            .cloned()
            .chain(task_files.iter().rev().cloned()),
    )
    .into_iter()
    .take(10)
    .collect::<Vec<_>>();
    let kept = whole_paths("recent files: ", &candidates, 720);
    let omitted = domain::unique_strings(candidates.into_iter().chain(task_files.iter().cloned()))
        .iter()
        .filter(|path| !kept.contains(path))
        .count();
    (
        (!kept.is_empty()).then(|| format!("recent files: {}", kept.join(", "))),
        omitted,
    )
}
fn working_state(capsule: &Capsule) -> (Vec<String>, BTreeMap<String, usize>) {
    let mut lines = Vec::new();
    let mut cuts = BTreeMap::new();
    let working = &capsule.0["working"];
    if !working.is_object() {
        return (lines, cuts);
    }
    if let Some(phase) = working["phase"].as_str().filter(|value| !value.is_empty()) {
        lines.push(format!("phase: {phase}"));
    }
    let progress = plain(working["progress"].as_str().unwrap_or(""));
    let count = progress.chars().count();
    let displayed = if count > 400 {
        cuts.insert("working_progress_characters".into(), count - 399);
        truncate_progress(&progress, 399)
    } else {
        progress
    };
    if !displayed.is_empty() {
        lines.push(format!("progress: {displayed}"));
    }
    for step in strings(&working["next_steps"]) {
        let (text, lost) = bounded_cut(&step, 200);
        *cuts
            .entry("working_next_step_characters".into())
            .or_default() += lost;
        lines.push(format!("next: {text}"));
    }
    let files = strings(&working["files"]);
    let project = strings(&capsule.0["recent_files"]);
    if !project.is_empty() {
        let (line, omitted) = recent_files_line(&project, &files);
        cuts.insert("working_files".into(), omitted);
        if let Some(line) = line {
            lines.push(line);
        }
    } else if !files.is_empty() {
        let mut recent = whole_paths(
            "recent files: ",
            &files.iter().rev().take(5).cloned().collect::<Vec<_>>(),
            500,
        );
        recent.reverse();
        cuts.insert("working_files".into(), files.len() - recent.len());
        lines.push(format!("recent files: {}", recent.join(", ")));
    }
    let sources = strings(&working["sources"]);
    if !sources.is_empty() {
        let cited = whole_paths("sources: ", &sources, 500);
        cuts.insert("working_sources".into(), sources.len() - cited.len());
        lines.push(format!("sources: {}", cited.join(", ")));
    }
    if let Some(record) = capsule.0["task_record"]
        .as_str()
        .filter(|text| !text.is_empty())
    {
        lines.push(format!("task record: {record}"));
    }
    (lines, cuts)
}
fn compaction(capsule: &Capsule, cuts: &BTreeMap<String, usize>) -> Option<String> {
    let parts = LABELS
        .iter()
        .filter_map(|(key, label)| {
            let count = capsule.0["omitted"][*key].as_u64().unwrap_or(0) as usize
                + cuts.get(*key).copied().unwrap_or(0);
            (count > 0).then(|| format!("{count} {label}"))
        })
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| {
        format!(
            "compaction: omitted {} — this capsule is a lossy view; {ADVICE}",
            parts.join(", ")
        )
    })
}
fn item_line(item: &Value, kind: &str, section: &str) -> String {
    let mut label = if let Some(path) = item["path"].as_str() {
        bounded(path, 260)
    } else {
        format!("episode {}", item["id"])
    };
    if !section.is_empty() && item["path"].is_string() {
        label.push_str(&format!(
            " § {}",
            bounded(&section.replace(" — ", " - "), 80)
        ));
    }
    let title = item["title"]
        .as_str()
        .or_else(|| item["summary"].as_str())
        .unwrap_or("");
    let mut marks = Vec::new();
    if item["match"] == "distinctive" {
        marks.push("weak match".into());
    }
    if item["conflict_recorded"] == true
        || item["conflicts"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
        || item["match"] == "conflict"
    {
        marks.push("recorded conflict; verify sources".into());
    }
    if item["attestation"] == "agent" {
        marks.push("agent-attested, not reviewed by a person".into());
    }
    if matches!(
        item["selection"].as_str(),
        Some("prompt-link" | "touch-link")
    ) && let Some(via) = item["via"].as_str().filter(|value| !value.is_empty())
    {
        marks.push(format!(
            "linked through {}, {}",
            bounded(via, 160),
            if item["selection"] == "prompt-link" {
                "named in the request"
            } else {
                "changed in this task"
            }
        ));
    }
    if let Some(changed) = item["source_changed"]
        .as_array()
        .filter(|items| !items.is_empty())
    {
        let more = if changed.len() > 1 {
            format!(" +{} more", changed.len() - 1)
        } else {
            String::new()
        };
        marks.push(format!(
            "cited file changed: {}{more}; check it before relying on this",
            bounded(changed[0].as_str().unwrap_or(""), 120)
        ));
    }
    format!(
        "- {kind} {label} — {}{}",
        bounded(title, 160),
        if marks.is_empty() {
            String::new()
        } else {
            format!(" ({})", marks.join("; "))
        }
    )
}
#[derive(Clone, Debug)]
pub struct RenderPlan {
    pub text: String,
    pub shown: Vec<(String, String)>,
    pub records: Vec<Value>,
}
#[derive(Clone)]
struct Entry {
    layer: String,
    item: Value,
    kind: String,
    path: String,
    heading: String,
    body: String,
    initial: String,
    raw: bool,
}
pub fn render_plan(capsule: &Capsule, profile: DeliveryProfile) -> Result<RenderPlan> {
    let mut head = Vec::new();
    let mut cuts = BTreeMap::new();
    let working = &capsule.0["working"];
    if !working.is_object() {
        head.push(if capsule.0["task_record_unreadable"] == true {
            "working: unavailable (the task's record cannot be read)".into()
        } else if capsule.0["working_changed"] == true {
            format!(
                "working: unavailable (the task's record changed while this turn read it; read {})",
                bounded(capsule.0["task_record"].as_str().unwrap_or("the task record"), 200)
            )
        } else if capsule.0["task_closed"] == true {
            let grave = char::from(96);
            format!(
                "working: closed (this branch's task is finished; turns are not recorded until a new one starts: start a new task: {grave}memory-bank/bin/infra-engine memory start --task-id {} --goal \"...\"{grave})",
                capsule.0["task_id"].as_str().filter(|id| !id.is_empty()).unwrap_or("<task>")
            )
        } else if capsule.0["kind"] == "warming" {
            let id = capsule.0["task_id"].as_str().unwrap_or("");
            let named = if regex::Regex::new(r"^codex-thread-[0-9a-f]{32}$")
                .map(|pattern| pattern.is_match(id))
                .unwrap_or(false)
            {
                format!("task {id}; ")
            } else {
                String::new()
            };
            format!("working: not recorded yet ({named}the task starts at the 5-turn checkpoint)")
        } else {
            "working: unavailable".into()
        });
    } else {
        let (goal, lost) = bounded_cut(working["goal"].as_str().unwrap_or(""), 200);
        cuts.insert("working_goal_characters".into(), lost);
        head.push(format!(
            "working: {} — {goal}",
            bounded(working["task_id"].as_str().unwrap_or(""), 200)
        ));
        let (state, state_cuts) = working_state(capsule);
        cuts.extend(state_cuts);
        head.extend(state.into_iter().map(|line| {
            bounded(
                &line,
                if line.starts_with("recent files: ") {
                    720
                } else {
                    500
                },
            )
        }));
    }
    if !working.is_object()
        && let Some(line) = recent_files_line(&strings(&capsule.0["recent_files"]), &[]).0
    {
        head.push(line);
    }
    if capsule.0["ticket_files"].is_object() {
        let tickets = strings(&capsule.0["ticket_files"]["tickets"])
            .into_iter()
            .take(3)
            .collect::<Vec<_>>();
        let label = if tickets.is_empty() {
            "ticket files: ".into()
        } else {
            format!("ticket files ({}): ", tickets.join(", "))
        };
        let paths = whole_paths(
            &label,
            &strings(&capsule.0["ticket_files"]["paths"])
                .into_iter()
                .take(10)
                .collect::<Vec<_>>(),
            720,
        );
        if !paths.is_empty() {
            let index = head
                .iter()
                .position(|line| line.starts_with("recent files: "))
                .map(|index| index + 1)
                .unwrap_or(head.len());
            head.insert(index, format!("{label}{}", paths.join(", ")));
        }
    }
    // A lightweight task is shown above from local state: nothing is pending.
    if capsule.0["kind"] == "warming" && capsule.0["task_closed"] != true && !working.is_object() {
        head.push(if profile.persistent() {
            "warming: task checkpoint pending".into()
        } else {
            format!(
                "warming: {} turn(s) pending",
                capsule.0["pending_turns"].as_u64().unwrap_or(0)
            )
        });
    }
    let notices = [
        ("invalid-utf8", "invalid UTF-8 content"),
        ("invalid-utf8-path", "invalid UTF-8 paths"),
    ]
    .into_iter()
    .filter_map(|(key, label)| {
        capsule.0["source_notices"][key]
            .as_u64()
            .filter(|count| *count > 0)
            .map(|count| format!("{label} {count}"))
    })
    .collect::<Vec<_>>();
    if !notices.is_empty() {
        head.push(bounded(
            &format!("sources omitted: {}", notices.join("; ")),
            160,
        ));
    }
    for warning in strings(&capsule.0["warnings"]).into_iter().take(3) {
        head.push(format!("warning: {}", bounded(&warning, 160)));
    }
    let mut compact = compaction(capsule, &cuts);
    let marker = format!("compaction: lossy view — {ADVICE}");
    let mut tail = Vec::new();
    if let Some(last) = capsule.0["last_turn"]
        .as_str()
        .filter(|text| !text.is_empty())
    {
        tail.push(format!("Last turn: {}", bounded(last, 600)));
    }
    if let Some(source) = capsule.0["query_source"].as_str()
        && (source == "task-id" || source == "task" && !profile.persistent())
    {
        tail.push(if source == "task" {
            "query: from task goal".into()
        } else {
            "query: from branch name only".into()
        });
    }
    if capsule.0["gate"]["mode"] == "enforce" && capsule.0["gate"]["decision"] == "skip" {
        tail.push(format!(
            "gate: skipped — {}",
            capsule.0["gate"]["reason"].as_str().unwrap_or("unknown")
        ));
    }
    let repeated = capsule.0["repeated"].as_u64().unwrap_or(0);
    if repeated > 0 {
        tail.push(format!(
            "memory: {repeated} item(s) handed earlier in this conversation still apply"
        ));
    }
    if repeated == 0
        && !["procedural", "semantic", "episodic"].iter().any(|layer| {
            capsule.0[*layer]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        })
    {
        let mut no_match = strings(&capsule.0["no_match"]);
        no_match.sort();
        if !no_match.is_empty() {
            tail.push(format!("no-match: {}", no_match.join(", ")));
        }
    }
    let mut entries = Vec::new();
    for (layer, label) in [
        ("semantic", "memory"),
        ("episodic", "history"),
        ("procedural", "skill"),
    ] {
        for item in capsule.0[layer].as_array().into_iter().flatten() {
            if !item.is_object() {
                continue;
            }
            let path = key(item);
            let excerpt = capsule
                .1
                .as_ref()
                .and_then(|prepared| prepared.excerpts.get(&path));
            let raw = excerpt
                .map(|excerpt| excerpt.text.as_str())
                .or_else(|| {
                    if item["match"] == "distinctive" || layer == "procedural" {
                        None
                    } else {
                        item["snippet"]
                            .as_str()
                            .or_else(|| item["outcome"].as_str())
                    }
                })
                .unwrap_or("");
            let limit = excerpt
                .map(|excerpt| excerpt.limit)
                .unwrap_or(if layer == "semantic" { 800 } else { 600 });
            let body = bounded(
                &domain::sanitize_automatic_query(raw).unwrap_or_default(),
                limit,
            );
            let heading = excerpt
                .map(|excerpt| {
                    domain::sanitize_automatic_query(&excerpt.heading).unwrap_or_default()
                })
                .unwrap_or_default();
            entries.push(Entry {
                layer: layer.into(),
                item: item.clone(),
                kind: if layer == "procedural" && item["kind"] == "policy" {
                    "policy".into()
                } else {
                    label.into()
                },
                path,
                heading,
                initial: body.clone(),
                body,
                raw: !raw.is_empty(),
            });
        }
    }
    fn assemble(
        head: &[String],
        compact: &Option<String>,
        tail: &[String],
        entries: &[Entry],
    ) -> Vec<String> {
        let mut lines = head.to_vec();
        lines.extend(compact.iter().cloned());
        lines.extend_from_slice(tail);
        if !entries.is_empty() {
            lines.push(HEADER.into());
        }
        for entry in entries {
            lines.push(item_line(&entry.item, &entry.kind, &entry.heading));
            if !entry.body.is_empty() {
                lines.push(format!("  {}", entry.body));
            }
        }
        lines
    }
    let mut lines = assemble(&head, &compact, &tail, &entries);
    while lines
        .iter()
        .map(|line| line.chars().count() + 1)
        .sum::<usize>()
        > RENDERED_CAPSULE_LIMIT
    {
        if let Some(index) = entries
            .iter()
            .rposition(|entry| entry.item["selection"] == "source-hint")
        {
            entries.remove(index);
            compact = Some(marker.clone());
        } else if let Some(index) = entries.iter().rposition(|entry| !entry.body.is_empty()) {
            let size = entries[index].body.chars().count();
            entries[index].body = if size > 120 {
                bounded(&entries[index].body, size / 2)
            } else {
                String::new()
            };
        } else if compact.as_deref() != Some(marker.as_str()) {
            compact = Some(marker.clone());
        } else if let Some(index) = entries
            .iter()
            .rposition(|entry| entry.layer == "procedural")
        {
            entries.remove(index);
        } else if let Some(index) = entries.iter().rposition(|entry| related(&entry.item)) {
            entries.remove(index);
        } else if !entries.is_empty() {
            entries.pop();
        } else if !tail.is_empty() {
            tail.pop();
        } else if head.len() > 1 {
            head.pop();
        } else {
            break;
        }
        lines = assemble(&head, &compact, &tail, &entries);
    }
    let mut records = Vec::new();
    let mut shown = Vec::new();
    for entry in entries {
        shown.push((entry.layer.clone(), entry.path.clone()));
        let snippet =
            domain::sanitize_automatic_query(entry.item["snippet"].as_str().unwrap_or(""))
                .unwrap_or_default();
        let section = if entry.heading.is_empty() {
            "none"
        } else if ["Sources", "Goal", "Progress", "Files", "Next Steps"]
            .contains(&entry.heading.as_str())
        {
            &entry.heading
        } else {
            "other"
        };
        let header = item_line(&entry.item, &entry.kind, &entry.heading);
        records.push(json!({"path":entry.path,"header_present":true,"header_sha256":sha256(header.as_bytes()),"body_bytes":entry.body.len(),"body_sha256":sha256(entry.body.as_bytes()),"pointer_only":entry.body.is_empty(),"initial_excerpt_bytes":entry.initial.len(),"body_reason":if !entry.body.is_empty(){"excerpt"}else if !entry.initial.is_empty(){"render-limit"}else if entry.raw{"sanitized-empty"}else{"no-excerpt"},"section":section,"section_sha256":sha256(entry.heading.as_bytes()),"snippet_sha256":sha256(snippet.as_bytes()),"snippet_bytes":snippet.len(),"snippet_in_body":!snippet.is_empty()&&entry.body.contains(&snippet),"shrunk":entry.body!=entry.initial}));
    }
    Ok(RenderPlan {
        text: lines.join("\n"),
        shown,
        records,
    })
}
pub fn render(capsule: &Capsule, profile: DeliveryProfile) -> Result<String> {
    Ok(render_plan(capsule, profile)?.text)
}

pub fn describe_excerpt_windows(capsule: &Capsule) -> BTreeMap<String, Value> {
    let mut output = BTreeMap::new();
    for layer in ["procedural", "semantic", "episodic"] {
        for item in capsule.0[layer].as_array().into_iter().flatten() {
            let Some(path) = item["path"].as_str() else {
                continue;
            };
            let excerpt = capsule
                .1
                .as_ref()
                .and_then(|prepared| prepared.excerpts.get(path));
            let raw = excerpt
                .map(|excerpt| excerpt.text.as_str())
                .or_else(|| {
                    if item["match"] == "distinctive" || layer == "procedural" {
                        None
                    } else {
                        item["snippet"].as_str()
                    }
                })
                .unwrap_or("");
            let limit = excerpt
                .map(|excerpt| excerpt.limit)
                .unwrap_or(if layer == "semantic" { 800 } else { 600 });
            let body = bounded(
                &domain::sanitize_automatic_query(raw).unwrap_or_default(),
                limit,
            );
            output.insert(path.to_string(),json!({"sha256":sha256(body.as_bytes()),"bytes":body.len(),"state":if !body.is_empty(){"excerpt"}else if !raw.is_empty(){"sanitized-empty"}else{"no-excerpt"}}));
        }
    }
    output
}

#[cfg(test)]
mod native_guidance_tests {
    use super::*;

    #[test]
    fn closed_task_recovery_uses_the_installed_native_command() {
        let capsule = Capsule::new(json!({"working":null,"task_closed":true,"task_id":"TASK-X"}));
        for profile in [DeliveryProfile::Ordinary, DeliveryProfile::CursorRule] {
            let output = render(&capsule, profile).unwrap();
            assert!(output.contains("memory-bank/bin/infra-engine memory start --task-id TASK-X"));
            assert!(!output.contains("context.py"));
        }
    }
}
