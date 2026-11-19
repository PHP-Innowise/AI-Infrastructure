//! What a launch may do without asking, and who answers when it asks.
//!
//! Three modes apply to a launch that edits; a planning launch stays read-only
//! whatever the mode:
//! - Auto: the host's own automatic review decides, and nothing waits for a
//!   person. A request the review would put to a person is declined.
//! - Manual: every request the host asks permission for waits for a person.
//! - Bypass: nothing is asked and the host's sandbox is off.
use crate::Result;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionMode {
    Auto,
    Manual,
    Bypass,
}
impl PermissionMode {
    pub const ALL: [&'static str; 3] = ["auto", "manual", "bypass"];
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "manual" => Some(Self::Manual),
            "bypass" => Some(Self::Bypass),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "manual",
            Self::Bypass => "bypass",
        }
    }
}

/// Maximum character length for [`PermissionAsk::diff_preview`].
pub const DIFF_PREVIEW_LIMIT: usize = 6144;

/// Sanitizes and bounds a multiline diff preview to at most [`DIFF_PREVIEW_LIMIT`] chars.
pub fn bound_diff_preview(raw: &str) -> Option<String> {
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let clean: String = normalized
        .chars()
        .map(|c| {
            if c == '\n' || c == '\t' || !c.is_control() {
                c
            } else {
                ' '
            }
        })
        .collect();
    let clean = clean.trim_matches('\n').trim_end();
    if clean.trim().is_empty() {
        return None;
    }
    if clean.chars().count() <= DIFF_PREVIEW_LIMIT {
        return Some(clean.to_owned());
    }
    let mut out: String = clean.chars().take(DIFF_PREVIEW_LIMIT - 1).collect();
    out.push('…');
    Some(out)
}

/// A person's answer to one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// Allow this and requests like it for the rest of the host's session.
    AllowSession,
    Deny,
}
impl Decision {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "allow" => Some(Self::Allow),
            "allow_session" => Some(Self::AllowSession),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::AllowSession => "allow_session",
            Self::Deny => "deny",
        }
    }
    /// The state a decided request's event carries.
    pub fn state(self) -> &'static str {
        match self {
            Self::Allow => "allowed",
            Self::AllowSession => "allowed_session",
            Self::Deny => "denied",
        }
    }
}

/// A person's answer to a permission request or native agent question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub decision: Decision,
    pub answers: Option<Value>,
}
impl Answer {
    pub fn from_decision(decision: Decision) -> Self {
        Self {
            decision,
            answers: None,
        }
    }
}
impl From<Decision> for Answer {
    fn from(decision: Decision) -> Self {
        Self::from_decision(decision)
    }
}

/// One request a host makes while it runs, as a person sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionAsk {
    /// Unique within the launch.
    pub id: String,
    pub provider: String,
    /// What asks: "Bash", "Edit", "Shell", "File change", "Network"…
    pub tool: String,
    /// One line naming what would happen: a command, a path, a host.
    pub summary: String,
    /// Why the host asks, when it says.
    pub detail: Option<String>,
    /// Whether the host can remember an approval for the rest of its session.
    pub session_option: bool,
    /// Semantic category: "command", "file_change", "permissions", "question", "tool".
    pub kind: String,
    /// Bounded unified patch or file-write preview (<= 6144 chars).
    pub diff_preview: Option<String>,
    /// Human-readable description of what "Allow for this session" remembers.
    pub rule_summary: Option<String>,
    /// Canonical Harness-local session rule key when Harness caches the rule itself.
    pub session_rule_key: Option<String>,
    /// Structured questions when `kind == "question"`.
    pub questions: Option<Vec<Value>>,
}
impl Default for PermissionAsk {
    fn default() -> Self {
        Self {
            id: String::new(),
            provider: String::new(),
            tool: String::new(),
            summary: String::new(),
            detail: None,
            session_option: false,
            kind: "tool".to_owned(),
            diff_preview: None,
            rule_summary: None,
            session_rule_key: None,
            questions: None,
        }
    }
}
impl PermissionAsk {
    fn ask_kind(&self) -> &str {
        if self.kind.trim().is_empty() {
            "tool"
        } else {
            self.kind.as_str()
        }
    }
    /// The session event that shows the request until it is answered. A
    /// stored event's `id` is its place in the session, so the request goes
    /// by `request_id`.
    pub fn pending_event(&self) -> Value {
        self.pending_event_with_timeout(None, None)
    }
    /// Variant of [`PermissionAsk::pending_event`] that attaches optional
    /// server-configured deadline metadata (`timeout_seconds` and RFC3339 UTC
    /// `expires_at`).
    pub fn pending_event_with_timeout(
        &self,
        timeout_seconds: Option<u64>,
        expires_at: Option<&str>,
    ) -> Value {
        let ask_kind = self.ask_kind();
        let diff_preview = self.diff_preview.as_deref().and_then(bound_diff_preview);
        let text = if ask_kind == "question" {
            format!("{} asks: {}", self.provider_name(), self.summary)
        } else {
            format!(
                "{} asks to run {}: {}",
                self.provider_name(),
                self.tool,
                self.summary
            )
        };
        json!({
            "kind": "permission",
            "state": "pending",
            "request_id": self.id,
            "provider": self.provider,
            "tool": self.tool,
            "summary": self.summary,
            "detail": self.detail,
            "session_option": self.session_option,
            "ask_kind": ask_kind,
            "diff_preview": diff_preview,
            "rule_summary": self.rule_summary,
            "has_session_rule": self.session_rule_key.is_some(),
            "questions": self.questions,
            "timeout_seconds": timeout_seconds,
            "expires_at": expires_at,
            "text": text,
        })
    }
    /// The session event for a request the launch answered on its own.
    pub fn resolved_event(&self, state: &str, by: &str) -> Value {
        let ask_kind = self.ask_kind();
        let diff_preview = self.diff_preview.as_deref().and_then(bound_diff_preview);
        json!({
            "kind": "permission",
            "state": state,
            "request_id": self.id,
            "provider": self.provider,
            "tool": self.tool,
            "summary": self.summary,
            "detail": self.detail,
            "ask_kind": ask_kind,
            "diff_preview": diff_preview,
            "rule_summary": self.rule_summary,
            "has_session_rule": self.session_rule_key.is_some(),
            "questions": self.questions,
            "by": by,
            "text": format!("{} {}: {}", self.tool, state_label(state), self.summary),
        })
    }
    fn provider_name(&self) -> &str {
        match self.provider.as_str() {
            "claude" => "Claude Code",
            "codex" => "Codex",
            "cursor" => "Cursor",
            other => other,
        }
    }
}
fn state_label(state: &str) -> &str {
    match state {
        "allowed" => "allowed",
        "allowed_session" => "allowed for the session",
        "denied" => "denied",
        "declined" => "declined automatically",
        "withdrawn" => "withdrawn by the host",
        "expired" => "no longer waiting",
        other => other,
    }
}

/// Where a launch puts the requests a person answers.
pub trait Approvals: Send + Sync {
    /// Records the request where the person sees it. The receiver yields
    /// their decision; it closes without one when the request is withdrawn.
    fn ask(&self, ask: &PermissionAsk) -> Result<tokio::sync::oneshot::Receiver<Decision>>;
    /// Records the request where the person sees it and yields both their
    /// decision and any structured question answers.
    fn ask_answer(&self, ask: &PermissionAsk) -> Result<tokio::sync::oneshot::Receiver<Answer>> {
        let mut rx = self.ask(ask)?;
        let (mut tx, answer_rx) = tokio::sync::oneshot::channel();
        match rx.try_recv() {
            Ok(decision) => {
                let _ = tx.send(Answer::from_decision(decision));
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {}
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    handle.spawn(async move {
                        tokio::select! {
                            res = &mut rx => {
                                if let Ok(decision) = res {
                                    let _ = tx.send(Answer::from_decision(decision));
                                }
                            }
                            _ = tx.closed() => {}
                        }
                    });
                } else {
                    std::thread::spawn(move || {
                        if let Ok(decision) = rx.blocking_recv() {
                            let _ = tx.send(Answer::from_decision(decision));
                        }
                    });
                }
            }
        }
        Ok(answer_rx)
    }
    /// The host stopped waiting for this request.
    fn withdraw(&self, id: &str) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_and_decisions_read_their_own_names_only() {
        for name in PermissionMode::ALL {
            assert_eq!(PermissionMode::parse(name).unwrap().as_str(), name);
        }
        assert_eq!(PermissionMode::parse("Auto"), None);
        assert_eq!(PermissionMode::parse("bypassPermissions"), None);
        for name in ["allow", "allow_session", "deny"] {
            assert_eq!(Decision::parse(name).unwrap().as_str(), name);
        }
        assert_eq!(Decision::parse("accept"), None);
        assert_eq!(Decision::AllowSession.state(), "allowed_session");
    }

    #[test]
    fn a_pending_event_names_what_would_run() {
        let ask = PermissionAsk {
            id: "claude-7".into(),
            provider: "claude".into(),
            tool: "Bash".into(),
            summary: "git push origin main".into(),
            detail: Some("Push the branch".into()),
            session_option: true,
            kind: "command".into(),
            diff_preview: None,
            rule_summary: Some("Session rule: Bash(git push *)".into()),
            session_rule_key: None,
            questions: None,
        };
        let event = ask.pending_event();
        assert_eq!(event["kind"], "permission");
        assert_eq!(event["state"], "pending");
        assert_eq!(event["request_id"], "claude-7");
        assert_eq!(event["session_option"], true);
        assert_eq!(event["ask_kind"], "command");
        assert_eq!(event["diff_preview"], Value::Null);
        assert_eq!(event["rule_summary"], "Session rule: Bash(git push *)");
        assert_eq!(event["has_session_rule"], false);
        assert_eq!(event["questions"], Value::Null);
        assert_eq!(
            event["text"],
            "Claude Code asks to run Bash: git push origin main"
        );
        let declined = ask.resolved_event("declined", "auto");
        assert_eq!(declined["by"], "auto");
        assert_eq!(declined["ask_kind"], "command");
        assert_eq!(declined["rule_summary"], "Session rule: Bash(git push *)");
        assert_eq!(declined["has_session_rule"], false);
        assert_eq!(
            declined["text"],
            "Bash declined automatically: git push origin main"
        );
    }

    #[test]
    fn enriched_ask_fields_emit_cleanly_and_bound_diff_preview() {
        let oversized = format!("--- a/src/lib.rs\n+++ b/src/lib.rs\n{}", "+x".repeat(4000));
        let questions = vec![json!({
            "id": "q1",
            "header": "Mode",
            "question": "Which mode?",
            "multi_select": false,
            "options": [{"label": "Fast", "description": "Quick pass"}],
        })];
        let ask = PermissionAsk {
            id: "cursor-1".into(),
            provider: "cursor".into(),
            tool: "Edit".into(),
            summary: "src/lib.rs".into(),
            detail: None,
            session_option: true,
            kind: "question".into(),
            diff_preview: bound_diff_preview(&oversized),
            rule_summary: Some("Remember exact command/path in Harness for this session".into()),
            session_rule_key: Some("cursor:edit:src/lib.rs".into()),
            questions: Some(questions.clone()),
        };
        let preview = ask.diff_preview.as_ref().unwrap();
        assert_eq!(preview.chars().count(), DIFF_PREVIEW_LIMIT);
        assert!(preview.contains('\n'));
        assert!(preview.ends_with('…'));

        let pending = ask.pending_event();
        assert_eq!(pending["ask_kind"], "question");
        assert_eq!(pending["has_session_rule"], true);
        assert_eq!(pending["questions"], Value::Array(questions.clone()));
        assert_eq!(pending["text"], "Cursor asks: src/lib.rs");
        assert_eq!(
            pending["diff_preview"].as_str().unwrap().chars().count(),
            DIFF_PREVIEW_LIMIT
        );

        let resolved = ask.resolved_event("allowed", "person");
        assert_eq!(resolved["ask_kind"], "question");
        assert_eq!(resolved["has_session_rule"], true);
        assert_eq!(resolved["questions"], Value::Array(questions));
        assert_eq!(
            resolved["diff_preview"].as_str().unwrap().chars().count(),
            DIFF_PREVIEW_LIMIT
        );
    }

    #[test]
    fn pending_event_with_timeout_emits_deadline_fields() {
        let ask = PermissionAsk {
            id: "codex-9".into(),
            provider: "codex".into(),
            tool: "command".into(),
            summary: "cargo test".into(),
            ..Default::default()
        };
        let without = ask.pending_event();
        assert_eq!(without["timeout_seconds"], Value::Null);
        assert_eq!(without["expires_at"], Value::Null);

        let with = ask.pending_event_with_timeout(Some(45), Some("2026-11-19T12:00:45.000Z"));
        assert_eq!(with["timeout_seconds"], 45);
        assert_eq!(with["expires_at"], "2026-11-19T12:00:45.000Z");
    }

    #[tokio::test]
    async fn approvals_ask_answer_defaults_to_mapping_ask_decision() {
        struct LegacyApprovals {
            decision: Decision,
        }
        impl Approvals for LegacyApprovals {
            fn ask(
                &self,
                _ask: &PermissionAsk,
            ) -> Result<tokio::sync::oneshot::Receiver<Decision>> {
                let (tx, rx) = tokio::sync::oneshot::channel();
                let d = self.decision;
                tokio::spawn(async move {
                    tokio::task::yield_now().await;
                    let _ = tx.send(d);
                });
                Ok(rx)
            }
            fn withdraw(&self, _id: &str) -> Result<()> {
                Ok(())
            }
        }

        let approvals = LegacyApprovals {
            decision: Decision::AllowSession,
        };
        let rx = approvals.ask_answer(&PermissionAsk::default()).unwrap();
        assert_eq!(
            rx.await.unwrap(),
            Answer::from_decision(Decision::AllowSession)
        );
        let answer_from: Answer = Decision::Deny.into();
        assert_eq!(answer_from, Answer::from_decision(Decision::Deny));

        // Withdrawn / closed sender closes the mapped Answer receiver cleanly.
        struct WithdrawnApprovals;
        impl Approvals for WithdrawnApprovals {
            fn ask(
                &self,
                _ask: &PermissionAsk,
            ) -> Result<tokio::sync::oneshot::Receiver<Decision>> {
                let (tx, rx) = tokio::sync::oneshot::channel();
                drop(tx);
                Ok(rx)
            }
            fn withdraw(&self, _id: &str) -> Result<()> {
                Ok(())
            }
        }
        let closed_rx = WithdrawnApprovals
            .ask_answer(&PermissionAsk::default())
            .unwrap();
        assert!(closed_rx.await.is_err());
    }
}
