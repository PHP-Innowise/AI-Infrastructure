//! Permission requests a running session waits on, and the person's answers.
//!
//! A session's Permissions setting says what an editing launch may do without
//! asking (`infra_host::permission`). With Manual, each request the host asks
//! becomes a `permission` event the page shows with its answers, and the
//! launch waits here until the person answers, the host withdraws the request
//! or the run ends.
use crate::{HarnessError, Result, sessions::Event};
use infra_host::permission::{Approvals, Decision, PermissionAsk, PermissionMode};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard},
};

/// A session's Permissions value: a mode's name, or null for a session that
/// names none and keeps the behaviour from before the setting existed
/// (workspace edits without a person, everything else declined).
pub fn validate(value: &Value) -> Result<Value> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::String(name) => PermissionMode::parse(name)
            .map(|mode| json!(mode.as_str()))
            .ok_or_else(|| HarnessError::invalid("Choose Auto, Manual or Bypass permissions.")),
        _ => Err(HarnessError::invalid(
            "Choose Auto, Manual or Bypass permissions.",
        )),
    }
}
/// The mode a session's editing launch runs with, if it names one.
pub fn mode(session: &Value) -> Option<PermissionMode> {
    session["permissions"]
        .as_str()
        .and_then(PermissionMode::parse)
}

struct Pending {
    sender: tokio::sync::oneshot::Sender<Decision>,
    emit: Event,
    ask: PermissionAsk,
}

/// Every request waiting for a person, by session and request.
#[derive(Default)]
pub struct PermissionDesk {
    pending: Mutex<BTreeMap<(String, String), Pending>>,
}
fn lock<T>(value: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    value
        .lock()
        .map_err(|_| HarnessError::unavailable("Permission requests lock failed."))
}
impl PermissionDesk {
    /// Where one run of a session puts its requests; its events go to `emit`.
    pub fn approvals(self: &Arc<Self>, session: &str, emit: Event) -> Arc<dyn Approvals> {
        Arc::new(SessionApprovals {
            desk: self.clone(),
            session: session.into(),
            emit,
        })
    }
    /// The person's answer to one request. The answer is recorded before the
    /// host hears it.
    pub fn decide(&self, session: &str, id: &str, decision: Decision) -> Result<Value> {
        let key = (session.to_owned(), id.to_owned());
        let mut pending = lock(&self.pending)?;
        let session_option = pending
            .get(&key)
            .ok_or_else(|| {
                HarnessError::conflict("This request is no longer waiting for an answer.")
            })?
            .ask
            .session_option;
        if decision == Decision::AllowSession && !session_option {
            return Err(HarnessError::invalid(
                "This host cannot remember an approval for the session; allow it once.",
            ));
        }
        let Some(request) = pending.remove(&key) else {
            return Err(HarnessError::conflict(
                "This request is no longer waiting for an answer.",
            ));
        };
        drop(pending);
        (request.emit)(request.ask.resolved_event(decision.state(), "person"))?;
        // The host may have stopped while the person decided.
        let delivered = request.sender.send(decision).is_ok();
        Ok(json!({"id": id, "state": decision.state(), "delivered": delivered}))
    }
    /// Answers no to whatever a session's finished or stopped run left
    /// waiting, so its page stops offering answers.
    pub fn expire(&self, session: &str) {
        let left: Vec<Pending> = match lock(&self.pending) {
            Ok(mut pending) => {
                let keys: Vec<_> = pending
                    .keys()
                    .filter(|(owner, _)| owner == session)
                    .cloned()
                    .collect();
                keys.into_iter()
                    .filter_map(|key| pending.remove(&key))
                    .collect()
            }
            Err(_) => vec![],
        };
        for request in left {
            let _ = (request.emit)(request.ask.resolved_event("expired", "harness"));
        }
    }
    /// The requests a session's run waits on now.
    pub fn waiting(&self, session: &str) -> Vec<Value> {
        lock(&self.pending)
            .map(|pending| {
                pending
                    .iter()
                    .filter(|((owner, _), _)| owner == session)
                    .map(|(_, request)| request.ask.pending_event())
                    .collect()
            })
            .unwrap_or_default()
    }
}

struct SessionApprovals {
    desk: Arc<PermissionDesk>,
    session: String,
    emit: Event,
}
impl Approvals for SessionApprovals {
    fn ask(&self, ask: &PermissionAsk) -> Result<tokio::sync::oneshot::Receiver<Decision>> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let mut pending = lock(&self.desk.pending)?;
        let key = (self.session.clone(), ask.id.clone());
        if pending.contains_key(&key) {
            return Err(HarnessError::conflict(
                "The host asked the same request twice.",
            ));
        }
        // The page sees the request before anyone can answer it.
        (self.emit)(ask.pending_event())?;
        pending.insert(
            key,
            Pending {
                sender,
                emit: self.emit.clone(),
                ask: ask.clone(),
            },
        );
        Ok(receiver)
    }
    fn withdraw(&self, id: &str) -> Result<()> {
        let request = lock(&self.desk.pending)?.remove(&(self.session.clone(), id.to_owned()));
        if let Some(request) = request {
            (request.emit)(request.ask.resolved_event("withdrawn", "host"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recorder() -> (Event, Arc<Mutex<Vec<Value>>>) {
        let events = Arc::new(Mutex::new(vec![]));
        let sink = events.clone();
        (
            Arc::new(move |event| {
                sink.lock().unwrap().push(event);
                Ok(())
            }),
            events,
        )
    }
    fn ask(id: &str, session_option: bool) -> PermissionAsk {
        PermissionAsk {
            id: id.into(),
            provider: "codex".into(),
            tool: "Shell".into(),
            summary: "cargo test".into(),
            detail: None,
            session_option,
        }
    }

    #[test]
    fn values_name_a_mode_or_nothing_and_refuse_anything_else() {
        assert_eq!(validate(&Value::Null).unwrap(), Value::Null);
        for name in ["auto", "manual", "bypass"] {
            assert_eq!(validate(&json!(name)).unwrap(), name);
        }
        for bad in [
            json!("Manual"),
            json!("bypassPermissions"),
            json!(true),
            json!(1),
        ] {
            assert!(validate(&bad).is_err(), "{bad}");
        }
        assert_eq!(mode(&json!({"permissions": null})), None);
        assert_eq!(
            mode(&json!({"permissions": "manual"})),
            Some(PermissionMode::Manual)
        );
    }

    #[tokio::test]
    async fn an_answer_reaches_the_waiting_launch_once_and_is_recorded_first() {
        let desk = Arc::new(PermissionDesk::default());
        let (emit, events) = recorder();
        let approvals = desk.approvals("session-1", emit);
        let receiver = approvals.ask(&ask("codex-7", true)).unwrap();
        assert_eq!(events.lock().unwrap()[0]["state"], "pending");
        assert_eq!(desk.waiting("session-1").len(), 1);
        assert!(desk.waiting("session-2").is_empty());
        // Another session cannot answer it.
        assert_eq!(
            desk.decide("session-2", "codex-7", Decision::Allow)
                .unwrap_err()
                .status,
            409
        );
        let answer = desk
            .decide("session-1", "codex-7", Decision::AllowSession)
            .unwrap();
        assert_eq!(answer["state"], "allowed_session");
        assert_eq!(answer["delivered"], true);
        assert_eq!(receiver.await.unwrap(), Decision::AllowSession);
        let recorded = events.lock().unwrap().clone();
        assert_eq!(recorded[1]["state"], "allowed_session");
        assert_eq!(recorded[1]["by"], "person");
        // A second answer finds nothing waiting.
        assert_eq!(
            desk.decide("session-1", "codex-7", Decision::Deny)
                .unwrap_err()
                .status,
            409
        );
    }

    #[tokio::test]
    async fn a_session_approval_needs_a_host_that_remembers_it() {
        let desk = Arc::new(PermissionDesk::default());
        let (emit, _) = recorder();
        let approvals = desk.approvals("s", emit);
        let receiver = approvals.ask(&ask("cursor-1", false)).unwrap();
        assert_eq!(
            desk.decide("s", "cursor-1", Decision::AllowSession)
                .unwrap_err()
                .status,
            400
        );
        // The refused answer left the request waiting.
        desk.decide("s", "cursor-1", Decision::Deny).unwrap();
        assert_eq!(receiver.await.unwrap(), Decision::Deny);
    }

    #[tokio::test]
    async fn withdrawn_and_expired_requests_stop_waiting() {
        let desk = Arc::new(PermissionDesk::default());
        let (emit, events) = recorder();
        let approvals = desk.approvals("s", emit);
        let withdrawn = approvals.ask(&ask("claude-1", true)).unwrap();
        let expired = approvals.ask(&ask("claude-2", true)).unwrap();
        assert!(approvals.ask(&ask("claude-2", true)).is_err());
        approvals.withdraw("claude-1").unwrap();
        assert!(withdrawn.await.is_err());
        desk.expire("s");
        assert!(expired.await.is_err());
        assert!(desk.waiting("s").is_empty());
        let states: Vec<_> = events
            .lock()
            .unwrap()
            .iter()
            .map(|event| event["state"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(states, ["pending", "pending", "withdrawn", "expired"]);
    }
}
