use crate::{dom, model};
use serde_json::Value as V;
use wasm_bindgen::{JsCast, JsValue};
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
    if !data.is_object() && !data.is_array() {
        return Err(Error {
            status,
            message: "The local runner returned invalid JSON.".into(),
            aborted: false,
        });
    }
    Ok(data)
}
