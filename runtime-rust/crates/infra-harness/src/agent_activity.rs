//! Bounded display-only agent timelines. Receipts and native memory never depend on these events.
use crate::{
    Result,
    run_activity::{redact_command, relative, strip_root},
};
use infra_host::activity::{activity_events, total_tokens};
use serde_json::{Value as V, json};
use std::collections::BTreeMap;
pub struct ActivityStream {
    emit: Box<dyn FnMut(V) -> Result<()> + Send>,
    provider: String,
    roots: Vec<(String, String)>,
    aliases: BTreeMap<String, (String, String)>,
    home: Option<String>,
    cwd: String,
    pub count: usize,
    pub bytes: usize,
    limited: bool,
    silenced: bool,
    agent: Option<(String, u64)>,
    agent_count: usize,
    agent_limited: bool,
    last_error: Option<String>,
    thinking: String,
    tokens: Option<u64>,
    cost: Option<f64>,
    reported: bool,
    pub reasons: BTreeMap<String, String>,
}
impl ActivityStream {
    pub fn new(
        emit: impl FnMut(V) -> Result<()> + Send + 'static,
        provider: &str,
        roots: BTreeMap<String, String>,
        aliases: BTreeMap<String, (String, String)>,
    ) -> Self {
        let mut roots = roots
            .into_iter()
            .map(|(service, path)| (path, service))
            .collect::<Vec<_>>();
        roots.sort_by_key(|(root, _)| std::cmp::Reverse(root.split(['/', '\\']).count()));
        let home = std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).ok();
        Self {
            emit: Box::new(emit),
            provider: provider.into(),
            roots,
            aliases,
            home,
            cwd: String::new(),
            count: 0,
            bytes: 0,
            limited: false,
            silenced: false,
            agent: None,
            agent_count: 0,
            agent_limited: false,
            last_error: None,
            thinking: String::new(),
            tokens: Some(0),
            cost: Some(0.),
            reported: false,
            reasons: BTreeMap::new(),
        }
    }
    pub fn start(&mut self, agent: &str, attempt: u64, cwd: &str, details: &V) {
        self.agent = Some((agent.into(), attempt));
        self.cwd = cwd.into();
        self.agent_count = 0;
        self.agent_limited = false;
        self.last_error = None;
        self.thinking.clear();
        self.tokens = Some(0);
        self.cost = Some(0.);
        self.reported = false;
        let mut event = bounded(details);
        event.as_object_mut().map(|o| o.remove("label"));
        event["kind"] = json!("agent");
        event["status"] = json!("running");
        let label = details["label"].as_str().unwrap_or(agent);
        event["text"] = json!(format!("{label} started · attempt {attempt}"));
        self.send(event);
    }
    pub fn finish(&mut self, status: &str, details: &V) {
        let Some((agent, _)) = self.agent.clone() else {
            return;
        };
        self.flush_thinking();
        let mut event = bounded(details);
        event.as_object_mut().map(|object| object.remove("summary"));
        event["kind"] = json!("agent");
        event["status"] = json!(status);
        event["text"] = json!(format!("{agent} {status}"));
        if let Some(summary) = details["summary"].as_str().filter(|s| !s.is_empty()) {
            event["summary"] = json!(redact_command(
                &summary.chars().take(300).collect::<String>()
            ));
        }
        if status != "completed"
            && let Some(error) = self.last_error.as_deref()
        {
            let reason = explain(&self.provider, error);
            self.reasons.insert(agent, reason.clone());
            event["reason"] = json!(reason);
        }
        if self.reported {
            event["tokens"] = json!(self.tokens);
            event["cost_usd"] = json!(self.cost.map(|n| (n * 1000000.).round() / 1000000.));
        }
        self.send(event);
        self.agent = None;
    }
    pub fn line(&mut self, raw: &[u8]) {
        if self.agent.is_none() {
            return;
        }
        let Ok(event) = crate::strict_json::parse(raw, 2 * 1024 * 1024) else {
            return;
        };
        for item in activity_events(&self.provider, &event) {
            self.activity(item);
        }
    }
    fn flush_thinking(&mut self) {
        let text = self.thinking.trim().to_owned();
        self.thinking.clear();
        if !text.is_empty() {
            self.activity(json!({"type":"thinking","text":text}));
        }
    }
    fn activity(&mut self, item: V) {
        let kind = item["type"].as_str().unwrap_or("");
        if kind == "thinking_delta" {
            self.thinking.push_str(item["text"].as_str().unwrap_or(""));
            self.thinking = self.thinking.chars().take(1200).collect();
            return;
        }
        if kind == "thinking_end" || !self.thinking.is_empty() {
            self.flush_thinking();
            if kind == "thinking_end" {
                return;
            }
        }
        if kind == "error"
            && let Some(error) = item["text"].as_str().filter(|s| !s.trim().is_empty())
        {
            self.last_error = Some(
                redact_command(&error.split_whitespace().collect::<Vec<_>>().join(" "))
                    .chars()
                    .take(300)
                    .collect(),
            );
        }
        if kind == "usage" {
            self.reported = true;
            self.tokens = self
                .tokens
                .zip(total_tokens(&item))
                .and_then(|(a, b)| a.checked_add(b));
            self.cost = self
                .cost
                .zip(
                    item["cost_usd"]
                        .as_f64()
                        .filter(|n| n.is_finite() && *n >= 0.),
                )
                .and_then(|(a, b)| {
                    let sum = a + b;
                    sum.is_finite().then_some(sum)
                });
            let mut event = item;
            event.as_object_mut().map(|o| o.remove("type"));
            event["kind"] = json!("usage");
            self.send(event);
            return;
        }
        if self.agent_limited || self.limited {
            return;
        }
        if self.count + 1 > 3000 || self.bytes > 2_500_000 {
            self.limited = true;
            self.send(json!({"kind":"agent_activity","type":"status","text":"Further agent activity in this launch is not shown. Receipts remain authoritative."}));
            return;
        }
        if self.agent_count + 1 > 400 {
            self.agent_limited = true;
            self.send(json!({"kind":"agent_activity","type":"status","text":"Further activity of this agent is not shown. Its receipt remains authoritative."}));
            return;
        }
        let mut event = json!({"kind":"agent_activity"});
        for (key, value) in item.as_object().into_iter().flatten() {
            if key == "changes" {
                continue;
            }
            let value = if key == "detail" && kind == "tool"
                || ["text", "items"].contains(&key.as_str()) && kind == "plan"
            {
                self.unroot(value)
            } else {
                value.clone()
            };
            event[key] = redact(&value);
        }
        if let Some(changes) = item["changes"].as_array().filter(|_| kind == "tool") {
            let detail = changes
                .iter()
                .filter_map(|change| {
                    let path = change["path"].as_str().filter(|s| !s.is_empty())?;
                    Some(format!(
                        "{} {}",
                        change["kind"].as_str().unwrap_or("change"),
                        self.shown(path)
                    ))
                })
                .collect::<Vec<_>>()
                .join(", ");
            let detail = redact_command(&detail)
                .chars()
                .take(300)
                .collect::<String>();
            if !detail.is_empty() {
                event["detail"] = json!(detail);
            }
        }
        if let Some(path) = event["path"].as_str().map(str::to_owned) {
            let (service, path, outside) = self.locate(&path);
            event["path"] = json!(path);
            if let Some(service) = service {
                event["service"] = json!(service);
            }
            if outside {
                event["outside"] = json!(true);
            }
        }
        event["text"] = json!(describe(&event));
        self.agent_count += 1;
        self.send(event);
    }
    pub fn locate(&self, value: &str) -> (Option<String>, String, bool) {
        let full = crate::run_activity::full(value, &self.cwd);
        for (alias, (service, path)) in &self.aliases {
            let (found, outside) = relative(&full, alias, None);
            if !outside && found.as_deref() == Some("") {
                return (Some(service.clone()), path.clone(), false);
            }
        }
        for (root, service) in &self.roots {
            let (path, outside) = relative(&full, root, None);
            if !outside {
                return (Some(service.clone()), path.unwrap_or_default(), false);
            }
        }
        (
            None,
            full.rsplit(['/', '\\']).next().unwrap_or("").into(),
            true,
        )
    }
    pub fn unroot(&self, value: &V) -> V {
        match value {
            V::String(s) => {
                let mut roots = vec![self.cwd.clone()];
                roots.extend(self.roots.iter().map(|(r, _)| r.clone()));
                json!(strip_root(s, &roots, self.home.as_deref()))
            }
            V::Array(v) => json!(v.iter().map(|v| self.unroot(v)).collect::<Vec<_>>()),
            V::Object(o) => V::Object(o.iter().map(|(k, v)| (k.clone(), self.unroot(v))).collect()),
            _ => value.clone(),
        }
    }
    pub fn shown(&self, value: &str) -> String {
        let (service, path, outside) = self.locate(value);
        if outside {
            format!("{path} (outside the project)")
        } else {
            let path = if path.is_empty() { "." } else { &path };
            service
                .map(|s| format!("{s}:{path}"))
                .unwrap_or_else(|| path.into())
        }
    }
    fn send(&mut self, mut event: V) {
        if self.silenced {
            return;
        }
        if let Some((agent, attempt)) = &self.agent {
            event["agent"] = json!(agent);
            event["attempt"] = json!(attempt);
        }
        event["at"] =
            json!(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
        let size = serde_json::to_vec(&event).map(|b| b.len()).unwrap_or(0);
        if self.count + 1 > 4500 || self.bytes + size > 3_600_000 {
            self.silenced = true;
            let mut limited = json!({"kind":"agent_activity","type":"status","at":event["at"],"text":"Agent display limit reached; follow the remaining dispatches in their receipts."});
            if let Some((agent, attempt)) = &self.agent {
                limited["agent"] = json!(agent);
                limited["attempt"] = json!(attempt);
            }
            event = limited;
        }
        self.count += 1;
        self.bytes = self.bytes.saturating_add(size);
        let _ = (self.emit)(event);
    }
}
pub fn redact(value: &V) -> V {
    match value {
        V::String(v) => json!(redact_command(v)),
        V::Array(v) => json!(v.iter().map(redact).collect::<Vec<_>>()),
        V::Object(v) => V::Object(v.iter().map(|(k, v)| (k.clone(), redact(v))).collect()),
        _ => value.clone(),
    }
}
pub fn bounded(details: &V) -> V {
    let mut out = json!({});
    for (key, value) in details.as_object().into_iter().flatten() {
        if let Some(list) = value.as_array() {
            out[key] = json!(
                list.iter()
                    .take(12)
                    .map(|v| redact_command(
                        &v.as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| v.to_string())
                            .chars()
                            .take(300)
                            .collect::<String>()
                    ))
                    .collect::<Vec<_>>()
            );
            if list.len() > 12 {
                out[format!("{key}_count")] = json!(list.len());
            }
        } else {
            out[key] = if let Some(text) = value.as_str() {
                json!(redact_command(&text.chars().take(1024).collect::<String>()))
            } else {
                value.clone()
            };
        }
    }
    out
}
pub fn describe(event: &V) -> String {
    if event["type"] != "tool" {
        return event["text"].as_str().unwrap_or("").into();
    }
    let mut target = [event["service"].as_str(), event["path"].as_str()]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    if event["outside"] == true {
        target.push_str(" (outside the project)");
    }
    let outcome = if event["state"] == "completed" {
        if event["ok"].as_bool().unwrap_or(false) {
            "done"
        } else {
            "failed"
        }
    } else {
        ""
    };
    [
        event["tool"].as_str().unwrap_or("Tool"),
        &target,
        event["detail"].as_str().unwrap_or(""),
        outcome,
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}
pub fn explain(provider: &str, reason: &str) -> String {
    let (name, login) = match provider {
        "claude" => ("Claude Code", Some("claude auth login")),
        "codex" => ("Codex", Some("codex login")),
        "cursor" => ("Cursor Agent", Some("cursor-agent login")),
        _ => (provider, None),
    };
    let auth=regex::Regex::new(r"(?i)authenticat|logged in|log in|login|oauth|api key|unauthori[sz]ed|forbidden|\b40[13]\b").map(|r|r.is_match(reason)).unwrap_or(false);
    let limit = regex::Regex::new(r"(?i)rate.?limit|usage limit|quota|\b429\b|overloaded")
        .map(|r| r.is_match(reason))
        .unwrap_or(false);
    if let Some(login) = login.filter(|_| auth) {
        format!(
            "{name} is not signed in or its login expired ({reason}). Sign in again in a terminal with \"{login}\", then retry."
        )
    } else if limit {
        format!(
            "{name} reported a usage or rate limit ({reason}). Retry later or choose another provider."
        )
    } else {
        format!("{name} reported: {reason}")
    }
}
