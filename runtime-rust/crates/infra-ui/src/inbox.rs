//! Next-turn input and durable browser-side ownership of an uncertain POST.
use crate::{api, commands, dom, model as m, session};
use serde_json::{json, Value as V};

/// Largest saved state `stored` reads back; `persist` never writes more.
const STORED_LIMIT: usize = 150000;
fn key(sid: &str) -> String {
    format!("infra-harness-inbox-v1:{sid}")
}
fn cache(sid: &str) -> String {
    format!("inbox-receipts:{sid}")
}
fn owner(sid: &str, epoch: u64) -> bool {
    m::read(|s| m::selection_matches(&s.sid, s.epoch, sid, epoch))
}
fn stored(sid: &str) -> V {
    match dom::storage(&key(sid)) {
        None => V::Null,
        Some(raw) if raw.len() <= STORED_LIMIT => {
            serde_json::from_str(&raw).unwrap_or(json!({"invalid":true}))
        }
        Some(_) => json!({"invalid":true}),
    }
}
/// JSON escapes can grow a 32000-byte message several times over; state that
/// could not be read back would lock the composer, so it is never written.
fn storable(value: &V) -> Option<String> {
    let raw = value.to_string();
    (raw.len() <= STORED_LIMIT).then_some(raw)
}
fn persist(sid: &str, value: &V) -> bool {
    let Ok(Some(storage)) = dom::window().local_storage() else {
        return false;
    };
    if value.is_null() {
        storage.remove_item(&key(sid)).is_ok()
    } else if let Some(raw) = storable(value) {
        storage.set_item(&key(sid), &raw).is_ok()
    } else {
        false
    }
}
fn unresolved(value: &V) -> bool {
    !value["request"].is_null() || value["invalid"] == true
}
pub fn needs_confirmation() -> bool {
    let sid = m::read(|s| s.sid.clone());
    !sid.is_empty() && unresolved(&stored(&sid))
}
/// Saved state that cannot be read names no request to confirm.
pub fn unreadable() -> bool {
    let sid = m::read(|s| s.sid.clone());
    !sid.is_empty() && stored(&sid)["invalid"] == true
}
pub fn eligible(v: &V) -> bool {
    v["status"] == "running"
        && v["workflow"] == "native"
        && v["agents_enabled"] == false
        && ["claude", "codex", "cursor"].contains(&v["provider"].as_str().unwrap_or(""))
        && ["native_session_id", "git_common_dir"]
            .iter()
            .all(|k| v[*k].as_str().is_some_and(|s| !s.trim().is_empty()))
        && [
            "brain",
            "sdd",
            "fleet",
            "clash",
            "creator",
            "system_run",
            "system_discovery",
        ]
        .iter()
        .all(|k| v[*k].is_null())
}
pub fn has_pending(sid: &str) -> bool {
    m::array(&m::data(&cache(sid))["items"])
        .iter()
        .any(|v| v["status"] == "pending")
}
pub fn watching(sid: &str) -> bool {
    track(&m::data(&cache(sid)), unresolved(&stored(sid)))
}
fn track(receipts: &V, unconfirmed: bool) -> bool {
    unconfirmed
        || receipts["retry"] == true
        || m::array(&receipts["items"])
            .iter()
            .any(|v| ["pending", "assigned"].contains(&v["status"].as_str().unwrap_or("")))
}
pub fn stop() {
    m::next("session-inbox");
    m::next("inbox-refresh");
}
fn refresh_failed(sid: &str, message: &str, retry: bool) {
    let mut data = m::data(&cache(sid));
    if !data.is_object() {
        data = json!({});
    }
    data["error"] = json!(true);
    data["retry"] = json!(retry);
    m::set(&cache(sid), data);
    dom::error("inbox-error", message);
}
fn prompt_valid(prompt: &str) -> bool {
    !prompt.trim().is_empty()
        && prompt.len() <= 32000
        && !prompt.contains('\0')
        && !prompt.trim().starts_with('/')
}
pub fn can_send(v: &V, prompt: &str) -> bool {
    let receipts = m::data(&cache(&m::text(v, "id")));
    eligible(v)
        && receipts["items"].is_array()
        && receipts["error"] != true
        && !has_pending(&m::text(v, "id"))
        && prompt_valid(prompt)
}
pub fn save_draft() {
    let sid = m::read(|s| s.sid.clone());
    if sid.is_empty() || m::read(|s| s.restoring) {
        return;
    }
    if unresolved(&stored(&sid)) {
        return;
    }
    let draft = dom::value("prompt");
    if draft.len() > 32000 {
        return;
    }
    // A draft too large to read back is dropped rather than restoring an older one.
    if draft.is_empty() || !persist(&sid, &json!({"draft":draft,"request":null})) {
        let _ = persist(&sid, &V::Null);
    }
}
pub fn restore(sid: &str) {
    let saved = stored(sid);
    if let Some(draft) = saved["draft"].as_str().filter(|v| v.len() <= 32000) {
        dom::set_value("prompt", draft);
    }
}
pub fn clear_draft(sid: &str, prompt: &str) {
    let saved = stored(sid);
    if !unresolved(&saved)
        && saved["draft"]
            .as_str()
            .is_some_and(|v| v.trim() == prompt.trim())
    {
        let _ = persist(sid, &V::Null);
    }
}
fn request(saved: &V, sid: &str, prompt: &str, id: &str) -> Result<V, String> {
    if saved["invalid"] == true {
        return Err(
            "Saved message state is invalid. Review this conversation before sending again.".into(),
        );
    }
    if !saved["request"].is_null() {
        let attempt = &saved["request"];
        if attempt["session_id"] == sid
            && attempt["request_id"]
                .as_str()
                .is_some_and(|v| !v.is_empty())
            && attempt["prompt"].as_str().is_some_and(prompt_valid)
        {
            return Ok(attempt.clone());
        }
        return Err("The saved message has an invalid owner. Review this conversation before sending again.".into());
    }
    if !prompt_valid(prompt) {
        return Err("Enter 1–32000 UTF-8 bytes without a leading slash command. Queueing supports plain text only.".into());
    }
    Ok(json!({"session_id":sid,"request_id":id,"prompt":prompt.trim()}))
}
fn receipt_valid(v: &V, sid: &str) -> bool {
    v["session_id"] == sid
        && v["id"].as_u64().is_some_and(|id| id > 0)
        && v["request_id"].as_str().is_some_and(|id| !id.is_empty())
        && v["prompt"].as_str().is_some_and(prompt_valid)
        && [
            "pending",
            "assigned",
            "completed",
            "blocked",
            "cancelled",
            "uncertain",
        ]
        .contains(&v["status"].as_str().unwrap_or(""))
}
fn matches(v: &V, attempt: &V) -> bool {
    receipt_valid(v, attempt["session_id"].as_str().unwrap_or(""))
        && v["request_id"] == attempt["request_id"]
        && v["prompt"] == attempt["prompt"]
}
/// Whether a refused POST proves that its request identity was never recorded.
fn not_recorded(saved: &V, status: u16) -> bool {
    if !unresolved(saved) {
        // An explicitly refused first attempt cannot have committed.
        return [400, 401, 403, 404, 409, 413, 422].contains(&status);
    }
    // A retry repeats the original identity and body. The runner returns any
    // receipt recorded for that identity before it checks session state, and
    // receipts are never deleted, so a later conflict or missing session
    // proves nothing was recorded; a refused body refused the original alike.
    // 401/403 come from the connection gate before that lookup: keep waiting.
    [400, 404, 409, 413, 422].contains(&status)
}
fn remember(sid: &str, receipt: V) {
    let mut items = m::array(&m::data(&cache(sid))["items"]);
    items.retain(|item| item["request_id"] != receipt["request_id"]);
    items.push(receipt);
    items.sort_by_key(|v| std::cmp::Reverse(v["id"].as_u64().unwrap_or(0)));
    items.truncate(100);
    m::set(&cache(sid), json!({"items":items}));
}
fn acknowledge(sid: &str, epoch: u64, attempt: &V) {
    if stored(sid)["request"] == *attempt {
        let _ = persist(sid, &V::Null);
    }
    if owner(sid, epoch) && dom::value("prompt").trim() == attempt["prompt"].as_str().unwrap_or("")
    {
        dom::set_value("prompt", "");
    }
}
pub async fn submit() {
    let (sid, epoch, selected, busy) = m::read(|s| {
        (
            s.sid.clone(),
            s.epoch,
            s.selected.clone(),
            s.pending || s.auth_failed,
        )
    });
    if sid.is_empty() || busy {
        return;
    }
    let saved = stored(&sid);
    if saved["invalid"] == true {
        // Nothing readable names a request to confirm. Receipts the runner
        // recorded stay listed under Next messages.
        dom::error(
            "composer-error",
            if persist(&sid, &V::Null) {
                "The saved message state could not be read and was cleared. Check Next messages before sending it again."
            } else {
                "Could not clear the unreadable saved message state. Enable local storage and try again."
            },
        );
        session::controls();
        return;
    }
    if !unresolved(&saved) && (!eligible(&selected) || has_pending(&sid)) {
        return;
    }
    let receipts = m::data(&cache(&sid));
    if !unresolved(&saved) && (!receipts["items"].is_array() || receipts["error"] == true) {
        dom::error(
            "composer-error",
            "Wait for verified next-message receipts before queueing another message.",
        );
        return;
    }
    let raw = dom::value("prompt");
    let identity = if unresolved(&saved) {
        m::text(&saved["request"], "request_id")
    } else if let Some(identity) = m::secure_request_identity() {
        identity
    } else {
        dom::error("composer-error", "Secure message identities are unavailable in this browser. Reconnect before queueing a message.");
        return;
    };
    let attempt = match request(&saved, &sid, &raw, &identity) {
        Ok(attempt) => attempt,
        Err(message) => {
            dom::error("composer-error", &message);
            return;
        }
    };
    let durable = json!({"draft":attempt["prompt"],"request":attempt});
    if !persist(&sid, &durable) {
        dom::error("composer-error", "Could not save the retry identity in this browser. Enable local storage, or shorten the message, before queueing it.");
        return;
    }
    stop();
    commands::close();
    m::write(|s| s.pending = true);
    session::controls();
    dom::error("composer-error", "");
    let result = api::post(
        &format!("/api/sessions/{}/inbox", dom::enc(&sid)),
        json!({"request_id":attempt["request_id"],"prompt":attempt["prompt"]}),
    )
    .await;
    stop();
    match result {
        Ok(receipt) if matches(&receipt, &attempt) => {
            remember(&sid, receipt);
            // Clear only the exact attempt we have now observed, never another draft.
            acknowledge(&sid, epoch, &attempt);
        }
        Ok(_) => {
            if owner(&sid, epoch) {
                dom::error("composer-error", "The runner returned an invalid receipt. Retry saving with the same message identity.");
            }
        }
        Err(error) => {
            // A refusal that proves nothing was recorded releases the identity
            // and keeps the text as a draft; network/server/invalid-response
            // failures and connection-gate refusals of a retry retain it.
            let released = not_recorded(&saved, error.status)
                && stored(&sid)["request"] == attempt
                && persist(&sid, &json!({"draft":attempt["prompt"],"request":null}));
            if owner(&sid, epoch) {
                dom::error(
                    "composer-error",
                    &if released && unresolved(&saved) {
                        format!(
                            "{} The runner never saved this message; its text is back in the composer.",
                            error.message
                        )
                    } else {
                        error.message.clone()
                    },
                );
            }
        }
    }
    m::write(|s| s.pending = false);
    if owner(&sid, epoch) {
        session::controls();
        session::poll(epoch).await;
    }
}
pub async fn refresh(sid: &str, epoch: u64) {
    let attempt = m::next("inbox-refresh");
    match api::scoped(
        "session-inbox",
        &format!("/api/sessions/{}/inbox", dom::enc(sid)),
    )
    .await
    {
        Ok((generation, data))
            if owner(sid, epoch)
                && m::current("session-inbox", generation)
                && m::current("inbox-refresh", attempt) =>
        {
            if let Some(items) = data["items"].as_array().filter(|items| {
                items.len() <= 100 && items.iter().all(|item| receipt_valid(item, sid))
            }) {
                m::set(&cache(sid), json!({"items":items}));
                dom::error("inbox-error", "");
                let saved = stored(sid);
                if !m::read(|s| s.pending)
                    && saved["request"].is_object()
                    && items
                        .iter()
                        .any(|receipt| matches(receipt, &saved["request"]))
                {
                    acknowledge(sid, epoch, &saved["request"]);
                    dom::error("composer-error", "");
                }
            } else {
                refresh_failed(sid, "Could not verify the next-message receipts. Refresh before queueing another message.", true);
            }
        }
        Err(error)
            if !error.aborted && owner(sid, epoch) && m::current("inbox-refresh", attempt) =>
        {
            refresh_failed(
                sid,
                &error.message,
                ![400, 401, 403, 404].contains(&error.status),
            )
        }
        _ => {}
    }
}
pub async fn cancel(id: &str) {
    let (sid, epoch, busy) = m::read(|s| (s.sid.clone(), s.epoch, s.pending || s.auth_failed));
    if sid.is_empty() || busy {
        return;
    }
    stop();
    m::write(|s| s.pending = true);
    session::controls();
    let result = api::post(
        &format!(
            "/api/sessions/{}/inbox/{}/cancel",
            dom::enc(&sid),
            dom::enc(id)
        ),
        json!({}),
    )
    .await;
    stop();
    if owner(&sid, epoch) {
        match result {
            Ok(receipt) if receipt_valid(&receipt, &sid) && receipt["request_id"] == id => {
                remember(&sid, receipt);
                dom::error("inbox-error", "");
            }
            Ok(_) => dom::error(
                "inbox-error",
                "The runner returned an invalid cancellation receipt. Refresh its status.",
            ),
            Err(error) => dom::error("inbox-error", &error.message),
        }
    }
    m::write(|s| s.pending = false);
    if owner(&sid, epoch) {
        session::controls();
        session::poll(epoch).await;
    }
}
fn status(value: &str) -> &'static str {
    match value {
        "pending" => "Queued for the next turn",
        "assigned" => "Next turn assigned",
        "completed" => "Turn completed",
        "blocked" => "Needs review",
        "cancelled" => "Cancelled",
        _ => "Delivery uncertain — review before continuing",
    }
}
pub fn render() {
    let (sid, selected, pending) = m::read(|s| {
        (
            s.sid.clone(),
            s.selected.clone(),
            s.pending || s.auth_failed,
        )
    });
    let items = m::array(&m::data(&cache(&sid))["items"]);
    let saved = if sid.is_empty() {
        V::Null
    } else {
        stored(&sid)
    };
    let unknown = unresolved(&saved);
    dom::hidden("session-inbox", items.is_empty() && !unknown);
    let projection = json!({"sid":sid,"items":items.iter().take(5).collect::<Vec<_>>(),"request":saved["request"],"invalid":saved["invalid"],"disabled":pending});
    if m::data("inbox-render") != projection {
        m::set("inbox-render", projection);
        dom::clear("inbox-receipts");
        if unknown {
            let card = dom::el("li", "inbox-receipt", "");
            if saved["invalid"] == true {
                dom::add(
                    &card,
                    &dom::el("p", "inbox-status", "Saved state unreadable"),
                );
                dom::add(
                    &card,
                    &dom::el(
                        "p",
                        "knowledge-note",
                        "Clear it to continue. Messages the runner recorded stay listed here.",
                    ),
                );
            } else {
                dom::add(&card, &dom::el("p", "inbox-status", "Save not confirmed"));
                dom::add(
                    &card,
                    &dom::el(
                        "p",
                        "inbox-prompt",
                        saved["request"]["prompt"].as_str().unwrap_or(""),
                    ),
                );
                dom::add(&card, &dom::el("p", "knowledge-note", "This message may already be saved. Retry saving before sending another message; its identity will stay the same. If the runner never saved it, its text returns to the composer."));
            }
            dom::append("inbox-receipts", &card);
        }
        for item in items.iter().take(5) {
            let card = dom::el("li", "inbox-receipt", "");
            dom::add(
                &card,
                &dom::el(
                    "p",
                    "inbox-status",
                    status(item["status"].as_str().unwrap_or("")),
                ),
            );
            dom::add(
                &card,
                &dom::el("p", "inbox-prompt", item["prompt"].as_str().unwrap_or("")),
            );
            if let Some(reason) = item["reason"].as_str() {
                dom::add(&card, &dom::el("p", "knowledge-note", reason));
            }
            if ["pending", "blocked"].contains(&item["status"].as_str().unwrap_or("")) {
                let cancel = dom::button(
                    "Cancel queued message",
                    "inbox-cancel",
                    item["request_id"].clone(),
                );
                if pending {
                    let _ = cancel.set_attribute("disabled", "");
                }
                dom::add(&card, &cancel);
            } else if item["status"] == "assigned" {
                dom::add(
                    &card,
                    &dom::el(
                        "p",
                        "knowledge-note",
                        "Use Cancel session to stop an assigned turn.",
                    ),
                );
            }
            dom::append("inbox-receipts", &card);
        }
    }
    let hint = if saved["invalid"] == true {
        "Clear the unreadable saved state before sending another message."
    } else if unknown {
        "Retry confirms the original saved message, including after the current turn finishes, or returns its text to the composer if the runner never saved it."
    } else if has_pending(&sid) {
        "A next message is already queued. Cancel it before queueing another."
    } else if eligible(&selected) {
        "Plain text only. This message runs after the current turn finishes successfully, with the same settings."
    } else {
        ""
    };
    if dom::get("inbox-hint")
        .and_then(|e| e.text_content())
        .as_deref()
        != Some(hint)
    {
        dom::text("inbox-hint", hint);
    }
    dom::hidden("inbox-hint", hint.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> V {
        json!({"id":"a","status":"running","workflow":"native","provider":"claude","agents_enabled":false,"native_session_id":"n","git_common_dir":"/repo/.git"})
    }
    #[test]
    fn eligibility_excludes_queued_unknown_and_special_authority() {
        let native = session();
        assert!(eligible(&native));
        for (field, value) in [
            ("status", json!("queued")),
            ("native_session_id", V::Null),
            ("git_common_dir", V::Null),
            ("agents_enabled", json!(true)),
            ("brain", json!({})),
            ("sdd", json!({})),
            ("clash", json!({})),
            ("creator", json!({})),
            ("system_run", json!({})),
            ("fleet", json!({})),
        ] {
            let mut changed = native.clone();
            changed[field] = value;
            assert!(!eligible(&changed), "{field}");
        }
    }
    #[test]
    fn uncertain_retry_keeps_original_body_even_after_source_finishes_or_draft_changes() {
        let first = request(&V::Null, "a", " Continue. ", "original").unwrap();
        let saved = json!({"request":first});
        let mut completed = session();
        completed["status"] = json!("completed");
        assert!(!eligible(&completed));
        assert!(unresolved(&saved));
        assert_eq!(
            request(&saved, "a", "Different text.", "new-id").unwrap(),
            first
        );
        assert!(request(&saved, "b", "Continue.", "new-id").is_err());
        assert!(request(&json!({"invalid":true}), "a", "Continue.", "new-id").is_err());
    }
    #[test]
    fn receipt_requires_same_session_request_prompt_and_known_status() {
        let attempt = request(&V::Null, "a", "Continue.", "original").unwrap();
        let receipt = json!({"id":1,"session_id":"a","request_id":"original","prompt":"Continue.","status":"pending"});
        assert!(matches(&receipt, &attempt));
        for (field, value) in [
            ("session_id", json!("b")),
            ("request_id", json!("other")),
            ("prompt", json!("Changed.")),
            ("status", json!("processed")),
            ("id", json!(0)),
        ] {
            let mut changed = receipt.clone();
            changed[field] = value;
            assert!(!matches(&changed, &attempt), "{field}");
        }
    }
    #[test]
    fn queue_input_enforces_bytes_and_plain_text_before_post() {
        for prompt in [
            " ".into(),
            " /model different".into(),
            "x\0y".into(),
            "é".repeat(16001),
        ] {
            assert!(request(&V::Null, "a", &prompt, "id").is_err());
        }
        assert!(request(&V::Null, "a", &"é".repeat(16000), "id").is_ok());
    }

    #[test]
    fn retry_keeps_an_uncertain_identity_until_the_runner_proves_it_unsaved() {
        let attempt = request(&V::Null, "a", "Continue.", "original").unwrap();
        let saved = json!({"request":attempt});
        // Lost replies, server faults and connection-gate refusals say nothing
        // about whether the original POST committed.
        for status in [0, 200, 401, 403, 500, 503] {
            assert!(!not_recorded(&saved, status), "{status}");
            assert_eq!(
                request(&saved, "a", "Continue.", "new-id").unwrap()["request_id"],
                "original"
            );
        }
        // The runner returns a recorded identity's receipt before any state
        // check, so its refusal of the retry proves the original was not kept:
        // e.g. 409 once the source turn finished. The text returns as a draft.
        for status in [400, 404, 409, 413, 422] {
            assert!(not_recorded(&saved, status), "{status}");
        }
        assert!(not_recorded(&V::Null, 409));
        assert!(not_recorded(&V::Null, 403));
        assert!(!not_recorded(&V::Null, 0));
        assert!(!not_recorded(&V::Null, 500));
    }

    #[test]
    fn saved_state_that_could_not_be_read_back_is_never_written() {
        let plain = "x".repeat(32000);
        let attempt = request(&V::Null, "a", &plain, "id").unwrap();
        assert!(storable(&json!({"draft":plain,"request":attempt})).is_some());
        // Each control byte is escaped as six JSON bytes: a valid 32000-byte
        // message would exceed what `stored` reads and lock the composer.
        let escaped = "\u{1}".repeat(32000);
        let attempt = request(&V::Null, "a", &escaped, "id").unwrap();
        assert!(storable(&json!({"draft":escaped,"request":attempt})).is_none());
        assert!(storable(&json!({"draft":escaped,"request":null})).is_none());
    }

    #[test]
    fn missing_receipts_during_source_completion_keep_polling_after_transient_failure() {
        assert!(track(&json!({"error":true,"retry":true}), false));
        assert!(track(&json!({"items":[{"status":"pending"}]}), false));
        assert!(track(&json!({"items":[{"status":"assigned"}]}), false));
        assert!(track(&json!({"items":[]}), true));
        assert!(!track(&json!({"items":[{"status":"completed"}]}), false));
        assert!(!track(&json!({"error":true,"retry":false}), false));
    }
}
