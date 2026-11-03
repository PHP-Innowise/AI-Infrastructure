//! Native helper-start receipts; assertions in assistant text never count as launches.
use crate::{HostError, Result};
use serde_json::{Value as V, json};
use std::collections::{BTreeMap, BTreeSet};
pub fn instructions(
    provider: &str,
    enabled: bool,
    count: u32,
    effort: Option<&str>,
) -> Result<String> {
    if !["claude", "codex", "cursor"].contains(&provider) || !(1..=40).contains(&count) {
        return Err(HostError::invalid("Invalid provider helper settings."));
    }
    if !enabled {
        return Ok("Additional agents are disabled for this session. Perform the task in the main agent. Do not spawn subagents, teammates, workflows, or other agent processes.".into());
    }
    let limit = format!(
        "Use at most {count} additional agents concurrently. The main agent does not count toward the required total. {}",
        if provider == "claude" && effort == Some("ultracode") {
            "Ultracode is enabled: use Workflow; native workflow concurrency limits apply. "
        } else {
            ""
        }
    );
    Ok(format!(
        "You MUST launch exactly {count} distinct additional agents during this turn. This is the user's required total, not an optional maximum. Previous turns' agents, resuming an existing helper, and messages to the same helper do not count as new launches. Assign each helper a useful, bounded subtask or an independent check/review. Do not reduce the count because the task seems easy or fewer helpers would suffice. Wait for every helper's result and incorporate or assess it before your final answer. {limit}If native concurrency is lower, launch the remaining helpers in successive batches. Do not delegate recursively. Keep file writes coordinated and all agents within the shared budgets. Use only the provider's native delegation tools, never shell-launched agent CLIs as a workaround. If tools are unavailable, permission is denied, or the remaining budget or task restrictions prevent the required launches, state the concrete reason and actual/required counts in your final answer; explicitly mark the delegation requirement unmet. Never claim that an agent ran without evidence."
    ))
}
pub fn codex_helper(event: &V) -> Option<&str> {
    let kind = event["type"].as_str()?;
    let item = &event["item"];
    let phase = item["kind"].as_str()?;
    let id = item["agent_thread_id"].as_str()?;
    if !["item.started", "item.updated", "item.completed"].contains(&kind)
        || item["type"] != "sub_agent_activity"
        || !["started", "completed"].contains(&phase)
        || !identity(id)
    {
        None
    } else {
        Some(phase)
    }
}
pub fn identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}
#[derive(Clone, Debug)]
pub struct DelegationTracker {
    pub provider: String,
    pub required_count: u32,
    pub attempted: bool,
    pub helpers: BTreeSet<String>,
    pending: BTreeMap<String, String>,
    pub journal_confirmed: bool,
}
impl DelegationTracker {
    pub fn new(provider: &str, count: u32) -> Result<Self> {
        if !["claude", "codex", "cursor"].contains(&provider) || !(1..=40).contains(&count) {
            return Err(HostError::invalid("Invalid helper count."));
        }
        Ok(Self {
            provider: provider.into(),
            required_count: count,
            attempted: false,
            helpers: BTreeSet::new(),
            pending: BTreeMap::new(),
            journal_confirmed: false,
        })
    }
    pub fn confirmed(&self) -> bool {
        !self.helpers.is_empty()
    }
    pub fn observe(&mut self, event: &V) {
        if self.provider == "codex" {
            if codex_helper(event).is_some() {
                self.attempted = true;
                if let Some(id) = event["item"]["agent_thread_id"].as_str() {
                    self.helpers.insert(id.into());
                }
            }
            let item = &event["item"];
            if ["item.started", "item.updated", "item.completed"]
                .contains(&event["type"].as_str().unwrap_or(""))
                && item["type"] == "collab_tool_call"
                && item["tool"] == "spawn_agent"
            {
                self.attempted = true;
                if event["type"] == "item.completed" && item["status"] == "completed" {
                    for id in item["receiver_thread_ids"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(V::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        self.helpers.insert(id.into());
                    }
                }
            }
            return;
        }
        if self.provider != "claude" {
            return;
        }
        if event["type"] == "system"
            && event["subtype"] == "task_started"
            && let Some(id) = event["tool_use_id"].as_str()
            && self
                .pending
                .get(id)
                .is_some_and(|n| ["Agent", "Task"].contains(&n.as_str()))
        {
            self.helpers.insert(id.into());
        }
        for block in event["message"]["content"].as_array().into_iter().flatten() {
            if event["type"] == "assistant" && block["type"] == "tool_use" {
                let name = block["name"].as_str().unwrap_or("");
                if ["Agent", "Task", "Workflow"].contains(&name)
                    && let Some(id) = block["id"].as_str()
                {
                    self.attempted = true;
                    if block["input"]["resume"].is_null()
                        || block["input"]["resume"] == false
                        || block["input"]["resume"] == ""
                    {
                        self.pending.insert(id.into(), name.into());
                    }
                }
            } else if event["type"] == "user"
                && block["type"] == "tool_result"
                && let Some(id) = block["tool_use_id"].as_str()
            {
                let name = self.pending.remove(id);
                if name
                    .as_deref()
                    .is_some_and(|n| ["Agent", "Task"].contains(&n))
                    && block["is_error"] != true
                {
                    self.helpers.insert(id.into());
                }
            }
        }
    }
    pub fn reconcile(
        &mut self,
        native_id: &str,
        project: &std::path::Path,
        since: f64,
        until: f64,
    ) -> Vec<V> {
        if self.provider != "codex" {
            return vec![];
        }
        let activities = crate::rollout::journal_activity(native_id, project, since, until);
        let ids = activities
            .iter()
            .filter_map(|a| a["agent_id"].as_str())
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let new = ids
            .difference(&self.helpers)
            .cloned()
            .collect::<BTreeSet<_>>();
        if !new.is_empty() {
            self.attempted = true;
            self.journal_confirmed = true;
            self.helpers.extend(new.iter().cloned());
        }
        activities.into_iter().filter(|a|new.contains(a["agent_id"].as_str().unwrap_or(""))).map(|a|json!({"kind":"tool","text":format!("Agent activity: {} (native session journal)",a["phase"].as_str().unwrap_or("")),"ok":true})).collect()
    }
    pub fn summary(&self) -> V {
        let count = self.helpers.len() as u32;
        let (status, tail) = if count == self.required_count {
            (
                "confirmed",
                "The launch count meets the requirement; this does not verify the helpers' work."
                    .into(),
            )
        } else if count > self.required_count {
            (
                "exceeded",
                "More helpers ran than requested; the exact-count requirement was not met.".into(),
            )
        } else {
            (
                if count > 0 { "partial" } else { "unconfirmed" },
                format!(
                    "{} required launches remain unconfirmed. The requirement is not confirmed as met; check the model's explanation and provider restrictions.",
                    self.required_count - count
                ),
            )
        };
        let mut text = format!(
            "Confirmed {count}/{} required helper launches for this turn. {tail}",
            self.required_count
        );
        if self.journal_confirmed {
            text.push_str(
                " Includes receipts from the local Codex journal omitted by the CLI stream.",
            );
        }
        if self.attempted && count == 0 {
            text.push_str(
                " Tool attempts or Workflow calls alone do not confirm individual helpers.",
            );
        }
        if self.provider == "cursor" {
            text.push_str(" Cursor helper telemetry is not verified by this adapter.");
        }
        json!({"kind":"delegation","status":status,"text":text,"required_count":self.required_count,"confirmed_count":count,"missing_count":self.required_count.saturating_sub(count)})
    }
}
