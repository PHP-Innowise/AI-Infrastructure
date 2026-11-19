use crate::{dom, model};
use serde_json::{json, Value as V};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};
use wasm_bindgen::{closure::Closure, JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

#[derive(Clone, Debug)]
pub struct Error {
    pub status: u16,
    pub message: String,
    pub aborted: bool,
}
impl From<JsValue> for Error {
    fn from(v: JsValue) -> Self {
        let name = js_sys::Reflect::get(&v, &"name".into())
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default();
        Self {
            status: 0,
            message: if name == "AbortError" {
                "Request cancelled".into()
            } else {
                "Could not reach the local runner. Refresh before retrying an uncertain request."
                    .into()
            },
            aborted: name == "AbortError",
        }
    }
}
pub async fn get(path: &str) -> Result<V, Error> {
    call("GET", path, None, None).await
}
pub async fn post(path: &str, body: V) -> Result<V, Error> {
    call("POST", path, Some(body), None).await
}
pub async fn delete(path: &str) -> Result<V, Error> {
    call("DELETE", path, None, None).await
}
#[allow(dead_code)]
pub async fn delete_with_body(path: &str, body: V) -> Result<V, Error> {
    call("DELETE", path, Some(body), None).await
}
pub async fn scoped(key: &str, path: &str) -> Result<(u64, V), Error> {
    let epoch = model::next(key);
    let controller = web_sys::AbortController::new().map_err(Error::from)?;
    model::write(|s| {
        s.aborts.insert(key.into(), controller.clone());
    });
    let result = call("GET", path, None, Some(controller.signal())).await;
    if model::current(key, epoch) {
        model::write(|s| {
            s.aborts.remove(key);
        });
    }
    result.map(|v| (epoch, v))
}
pub async fn call(
    method: &str,
    path: &str,
    body: Option<V>,
    signal: Option<web_sys::AbortSignal>,
) -> Result<V, Error> {
    let options = web_sys::RequestInit::new();
    options.set_method(method);
    options.set_credentials(web_sys::RequestCredentials::SameOrigin);
    options.set_cache(web_sys::RequestCache::NoStore);
    options.set_mode(web_sys::RequestMode::SameOrigin);
    if let Some(signal) = signal.as_ref() {
        options.set_signal(Some(signal));
    }
    if let Some(v) = body.as_ref() {
        options.set_body(&JsValue::from_str(&v.to_string()));
    }
    let request = web_sys::Request::new_with_str_and_init(path, &options).map_err(Error::from)?;
    request
        .headers()
        .set("Accept", "application/json")
        .map_err(Error::from)?;
    request
        .headers()
        .set(
            "X-Harness-Token",
            &model::read(|s| model::text(&s.boot, "csrf")),
        )
        .map_err(Error::from)?;
    if body.is_some() {
        request
            .headers()
            .set("Content-Type", "application/json")
            .map_err(Error::from)?;
    }
    let response = JsFuture::from(dom::window().fetch_with_request(&request))
        .await
        .map_err(Error::from)?
        .dyn_into::<web_sys::Response>()
        .map_err(Error::from)?;
    let status = response.status();
    let raw = JsFuture::from(response.text().map_err(Error::from)?)
        .await
        .map_err(Error::from)?
        .as_string()
        .unwrap_or_default();
    let data: V = serde_json::from_str(&raw).unwrap_or(V::Null);
    if !response.ok() {
        let message = data["error"]
            .as_str()
            .or_else(|| data["message"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("The local runner returned HTTP {status}."));
        if status == 401 || status == 403 {
            model::write(|s| s.auth_failed = true);
            dom::error("connection-message","The local connection token is no longer accepted. Reconnect before sending another request.");
            dom::hidden("connection-banner", false);
            crate::session::controls();
        }
        return Err(Error {
            status,
            message,
            aborted: false,
        });
    }
    if method == "DELETE" && (status == 204 || raw.trim().is_empty()) {
        return Ok(json!({"ok": true}));
    }
    if !data.is_object() && !data.is_array() {
        return Err(Error {
            status,
            message: "The local runner returned invalid JSON.".into(),
            aborted: false,
        });
    }
    Ok(data)
}

type SseHandler = Rc<dyn Fn(&V)>;

const DEFAULT_SSE_EVENTS: &[&str] = &[
    "resync",
    "session_event",
    "session_status",
    "waiting_counts",
    "system_run",
    "creator_run",
    "results_check",
    "graph_status",
    "permission",
    "snapshot",
    "heartbeat",
];

#[derive(Default)]
struct SseClient {
    source: Option<web_sys::EventSource>,
    connected: bool,
    enabled: bool,
    attempt: u32,
    last_event_id: String,
    handlers: BTreeMap<String, Vec<SseHandler>>,
    attached_kinds: BTreeSet<String>,
    open_cb: Option<Closure<dyn FnMut(web_sys::Event)>>,
    error_cb: Option<Closure<dyn FnMut(web_sys::Event)>>,
    message_cbs: Vec<Closure<dyn FnMut(web_sys::MessageEvent)>>,
}

thread_local! {
    static SSE: RefCell<SseClient> = RefCell::new(SseClient::default());
}

/// Computes bounded exponential backoff in milliseconds (`1s`, `2s`, `4s`, `8s`, capped at `15s`).
pub fn sse_backoff_ms(attempt: u32) -> i32 {
    let shift = attempt.min(4);
    let ms = 1000i32.saturating_mul(1i32 << shift);
    ms.min(15_000)
}

/// Normalizes SSE event type + raw JSON data into `(canonical_kind, payload)`.
pub fn normalize_sse_payload(event_name: &str, raw_data: &str) -> (String, V) {
    let parsed: V = if raw_data.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(raw_data).unwrap_or_else(|_| json!({ "raw": raw_data }))
    };
    let explicit_kind = event_name.trim();
    let payload_kind = parsed["kind"]
        .as_str()
        .or_else(|| parsed["event"].as_str())
        .or_else(|| parsed["type"].as_str())
        .unwrap_or("")
        .trim();
    let kind = if !explicit_kind.is_empty() && explicit_kind != "message" {
        explicit_kind.to_owned()
    } else if !payload_kind.is_empty() {
        payload_kind.to_owned()
    } else {
        "message".to_owned()
    };
    let mut normalized = if parsed.is_object() {
        parsed
    } else {
        json!({ "value": parsed })
    };
    if normalized["sse_event"].is_null() {
        normalized["sse_event"] = json!(kind);
    }
    if normalized["kind"].is_null() {
        normalized["kind"] = json!(kind);
    }
    (kind, normalized)
}

/// Returns `true` when the `/api/events/stream` `EventSource` connection is currently open.
pub fn sse_connected() -> bool {
    SSE.with(|sse| sse.borrow().connected)
}

/// Registers an SSE listener for a named event kind (or `"*"` for all events).
pub fn on_sse(event_kind: &str, handler: impl Fn(&V) + 'static) {
    let kind = event_kind.trim().to_owned();
    if kind.is_empty() {
        return;
    }
    let handler: SseHandler = Rc::new(handler);
    let source_to_attach = SSE.with(|sse| {
        let mut sse = sse.borrow_mut();
        sse.handlers.entry(kind.clone()).or_default().push(handler);
        if kind != "*" && !sse.attached_kinds.contains(&kind) {
            sse.source.clone()
        } else {
            None
        }
    });
    if let Some(source) = source_to_attach {
        attach_named_listener(&source, &kind);
    }
}

/// Dispatches an incoming SSE frame to registered handlers; exposed for unit tests and EventSource callbacks.
pub fn dispatch_sse_payload(event_name: &str, raw_data: &str, last_id: Option<&str>) -> String {
    if let Some(id) = last_id.map(str::trim).filter(|s| !s.is_empty()) {
        SSE.with(|sse| sse.borrow_mut().last_event_id = id.to_owned());
    }
    let (kind, payload) = normalize_sse_payload(event_name, raw_data);
    let handlers: Vec<SseHandler> = SSE.with(|sse| {
        let sse = sse.borrow();
        let mut out = Vec::new();
        if let Some(exact) = sse.handlers.get(&kind) {
            out.extend(exact.iter().cloned());
        }
        if let Some(wild) = sse.handlers.get("*") {
            out.extend(wild.iter().cloned());
        }
        out
    });
    for handler in handlers {
        handler(&payload);
    }
    kind
}

fn close_sse_source() {
    SSE.with(|sse| {
        let mut sse = sse.borrow_mut();
        sse.connected = false;
        if let Some(source) = sse.source.take() {
            source.set_onopen(None);
            source.set_onerror(None);
            source.close();
        }
        sse.attached_kinds.clear();
        sse.open_cb = None;
        sse.error_cb = None;
        sse.message_cbs.clear();
    });
    model::write(|s| s.stream.sse_connected = false);
}

fn attach_named_listener(source: &web_sys::EventSource, event_name: &str) {
    let already = SSE.with(|sse| {
        !sse.borrow_mut()
            .attached_kinds
            .insert(event_name.to_owned())
    });
    if already {
        return;
    }
    let kind = event_name.to_owned();
    let cb = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |ev: web_sys::MessageEvent| {
        let raw = ev.data().as_string().unwrap_or_default();
        let last_id = ev.last_event_id();
        dispatch_sse_payload(
            &kind,
            &raw,
            (!last_id.is_empty()).then_some(last_id.as_str()),
        );
    });
    let target: &web_sys::EventTarget = source.as_ref();
    let _ = target.add_event_listener_with_callback(event_name, cb.as_ref().unchecked_ref());
    SSE.with(|sse| sse.borrow_mut().message_cbs.push(cb));
}

fn stream_url() -> String {
    let last_id = SSE.with(|sse| sse.borrow().last_event_id.clone());
    if last_id.is_empty() {
        "/api/events/stream".into()
    } else {
        format!("/api/events/stream?last_event_id={}", dom::enc(&last_id))
    }
}

/// Opens `/api/events/stream` `EventSource` with typed event listeners and automatic reconnect backoff.
pub fn start_sse() {
    if model::read(|s| s.auth_failed) {
        return;
    }
    dom::cancel_timer("sse-reconnect");
    close_sse_source();
    SSE.with(|sse| sse.borrow_mut().enabled = true);
    let init = web_sys::EventSourceInit::new();
    init.set_with_credentials(true);
    let Ok(source) = web_sys::EventSource::new_with_event_source_init_dict(&stream_url(), &init)
    else {
        schedule_sse_reconnect();
        return;
    };
    let open_cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        SSE.with(|sse| {
            let mut sse = sse.borrow_mut();
            sse.connected = true;
            sse.attempt = 0;
        });
        model::write(|s| s.stream.sse_connected = true);
        dispatch_sse_payload("sse_open", "{}", None);
    });
    let error_cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        close_sse_source();
        dispatch_sse_payload("sse_error", "{}", None);
        if SSE.with(|sse| sse.borrow().enabled) && !model::read(|s| s.auth_failed) {
            schedule_sse_reconnect();
        }
    });
    source.set_onopen(Some(open_cb.as_ref().unchecked_ref()));
    source.set_onerror(Some(error_cb.as_ref().unchecked_ref()));

    let mut kinds: BTreeSet<String> = DEFAULT_SSE_EVENTS.iter().map(|s| (*s).to_owned()).collect();
    kinds.insert("message".into());
    SSE.with(|sse| {
        let mut sse = sse.borrow_mut();
        for key in sse.handlers.keys() {
            if key != "*" && !key.starts_with("sse_") {
                kinds.insert(key.clone());
            }
        }
        sse.source = Some(source.clone());
        sse.open_cb = Some(open_cb);
        sse.error_cb = Some(error_cb);
    });
    for kind in kinds {
        attach_named_listener(&source, &kind);
    }
}

fn schedule_sse_reconnect() {
    let delay = SSE.with(|sse| {
        let mut sse = sse.borrow_mut();
        if !sse.enabled {
            return None;
        }
        let delay = sse_backoff_ms(sse.attempt);
        sse.attempt = sse.attempt.saturating_add(1);
        Some(delay)
    });
    if let Some(ms) = delay {
        dom::later("sse-reconnect", ms, || {
            start_sse();
        });
    }
}

/// Closes `/api/events/stream` cleanly and disables automatic SSE reconnection.
#[allow(dead_code)]
pub fn stop_sse() {
    dom::cancel_timer("sse-reconnect");
    SSE.with(|sse| {
        let mut sse = sse.borrow_mut();
        sse.enabled = false;
        sse.attempt = 0;
    });
    close_sse_source();
}

#[cfg(test)]
mod sse_contract_tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn backoff_scales_exponentially_and_caps_at_15s() {
        assert_eq!(sse_backoff_ms(0), 1000);
        assert_eq!(sse_backoff_ms(1), 2000);
        assert_eq!(sse_backoff_ms(2), 4000);
        assert_eq!(sse_backoff_ms(3), 8000);
        assert_eq!(sse_backoff_ms(4), 15000);
        assert_eq!(sse_backoff_ms(9), 15000);
    }

    #[test]
    fn named_and_unnamed_sse_payloads_route_to_exact_and_wildcard_handlers() {
        SSE.with(|sse| {
            let mut s = sse.borrow_mut();
            s.handlers.clear();
            s.last_event_id.clear();
        });
        let seen_system = Rc::new(Cell::new(0));
        let seen_wild = Rc::new(Cell::new(0));
        let seen_session = Rc::new(Cell::new(0));
        let sys_clone = seen_system.clone();
        let wild_clone = seen_wild.clone();
        let sess_clone = seen_session.clone();

        on_sse("system_run", move |v| {
            if v["run_id"] == "r-1" && v["sse_event"] == "system_run" {
                sys_clone.set(sys_clone.get() + 1);
            }
        });
        on_sse("session_event", move |v| {
            if v["session_id"] == "s-1" {
                sess_clone.set(sess_clone.get() + 1);
            }
        });
        on_sse("*", move |_| {
            wild_clone.set(wild_clone.get() + 1);
        });

        assert_eq!(
            dispatch_sse_payload(
                "system_run",
                r#"{"run_id":"r-1","status":"running"}"#,
                Some("42")
            ),
            "system_run"
        );
        assert_eq!(
            dispatch_sse_payload(
                "message",
                r#"{"kind":"session_event","session_id":"s-1"}"#,
                Some("43")
            ),
            "session_event"
        );
        assert_eq!(seen_system.get(), 1);
        assert_eq!(seen_session.get(), 1);
        assert_eq!(seen_wild.get(), 2);
        SSE.with(|sse| assert_eq!(sse.borrow().last_event_id, "43"));
    }
}
