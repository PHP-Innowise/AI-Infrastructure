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
}
impl PermissionAsk {
    /// The session event that shows the request until it is answered. A
    /// stored event's `id` is its place in the session, so the request goes
    /// by `request_id`.
    pub fn pending_event(&self) -> Value {
        json!({
            "kind": "permission",
            "state": "pending",
            "request_id": self.id,
            "provider": self.provider,
            "tool": self.tool,
            "summary": self.summary,
            "detail": self.detail,
            "session_option": self.session_option,
            "text": format!("{} asks to run {}: {}", self.provider_name(), self.tool, self.summary),
        })
    }
    /// The session event for a request the launch answered on its own.
    pub fn resolved_event(&self, state: &str, by: &str) -> Value {
        json!({
            "kind": "permission",
            "state": state,
            "request_id": self.id,
            "provider": self.provider,
            "tool": self.tool,
            "summary": self.summary,
            "detail": self.detail,
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
        };
        let event = ask.pending_event();
        assert_eq!(event["kind"], "permission");
        assert_eq!(event["state"], "pending");
        assert_eq!(event["request_id"], "claude-7");
        assert_eq!(event["session_option"], true);
        assert_eq!(
            event["text"],
            "Claude Code asks to run Bash: git push origin main"
        );
        let declined = ask.resolved_event("declined", "auto");
        assert_eq!(declined["by"], "auto");
        assert_eq!(
            declined["text"],
            "Bash declined automatically: git push origin main"
        );
    }
}
