use crate::{api, dom, model as m, session};
use serde_json::{json, Value as V};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
#[derive(Clone, Debug)]
struct Token {
    start: usize,
    end: usize,
    trigger: char,
    query: String,
}
fn delimiter(c: char) -> bool {
    c.is_whitespace() || "([{\"'`".contains(c)
}
fn allowed(c: char, trigger: char) -> bool {
    if trigger == '@' {
        !c.is_whitespace() && !"\"'`()[]{}<>".contains(c)
    } else {
        c.is_ascii_alphanumeric() || "._:-".contains(c)
    }
}
fn token(text: &str, caret: usize, skills: bool) -> Option<Token> {
    let before = &text[..caret.min(text.len())];
    if before.starts_with('/') && !before[1..].chars().any(char::is_whitespace) {
        let end = caret
            + text[caret..]
                .chars()
                .take_while(|c| !c.is_whitespace())
                .map(char::len_utf8)
                .sum::<usize>();
        return Some(Token {
            start: 0,
            end,
            trigger: '/',
            query: before[1..].into(),
        });
    }
    let mut start = None;
    for (index, c) in before.char_indices().rev() {
        if c == '@' || c == '$' && skills {
            let boundary = index == 0
                || before[..index]
                    .chars()
                    .last()
                    .map(delimiter)
                    .unwrap_or(true);
            if boundary && before[index + 1..].chars().all(|x| allowed(x, c)) {
                start = Some((index, c));
            }
            break;
        }
        if delimiter(c) {
            break;
        }
    }
    let (start, trigger) = start?;
    let query = before[start + trigger.len_utf8()..].to_owned();
    if query.chars().count() > if trigger == '@' { 300 } else { 200 } {
        return None;
    }
    let end = caret
        + text[caret..]
            .chars()
            .take_while(|c| allowed(*c, trigger))
            .map(char::len_utf8)
            .sum::<usize>();
    Some(Token {
        start,
        end,
        trigger,
        query,
    })
}
fn byte_caret(text: &str, utf16: u32) -> usize {
    let mut units = 0;
    for (i, c) in text.char_indices() {
        if units >= utf16 {
            return i;
        }
        units += c.len_utf16() as u32;
    }
    text.len()
}
fn target(files: bool) -> Option<String> {
    let sid = m::read(|s| s.sid.clone());
    let session = m::read(|s| s.selected.clone());
    if !sid.is_empty() {
        if session.is_null()
            || session["creator"].is_object()
            || session["system_run"].is_object()
            || session["system_discovery"].is_object()
        {
            return None;
        }
        if !files
            && (session["workflow"] != "native"
                || session["fleet"].is_object()
                || session["clash"].is_object())
        {
            return None;
        }
        return Some(format!(
            "/api/sessions/{}/{}",
            dom::enc(&sid),
            if files { "files" } else { "commands" }
        ));
    }
    let project = dom::value("project");
    if project.is_empty() {
        return None;
    }
    if !files && (dom::value("workflow") != "native" || dom::checked("clash-enabled")) {
        return None;
    }
    let mut query = vec![];
    if !files {
        query.push(("provider", dom::value("provider")));
    }
    if dom::value("workspace") == "existing-worktree" {
        query.push(("worktree", dom::value("existing-worktree")));
    }
    Some(format!(
        "/api/projects/{}/{}{}",
        dom::enc(&project),
        if files { "files" } else { "commands" },
        dom::query(&query)
    ))
}
fn rank(items: Vec<V>, needle: &str) -> Vec<V> {
    let needle = needle.to_lowercase();
    let mut out = vec![];
    for (index, item) in items.into_iter().enumerate() {
        let mut names = vec![m::text(&item, "name").to_lowercase()];
        names.extend(
            m::array(&item["aliases"])
                .iter()
                .map(|x| m::string(x).to_lowercase()),
        );
        let score = if names.iter().any(|n| n.starts_with(&needle)) {
            Some(0)
        } else if names
            .iter()
            .any(|n| n.split([':', '_', '-']).any(|w| w.starts_with(&needle)))
        {
            Some(1)
        } else if names.iter().any(|n| n.contains(&needle)) {
            Some(2)
        } else if m::text(&item, "description")
            .to_lowercase()
            .contains(&needle)
        {
            Some(3)
        } else {
            None
        };
        if let Some(score) = score {
            out.push((score, index, item));
        }
    }
    out.sort_by_key(|(score, index, _)| (*score, *index));
    out.into_iter().map(|(_, _, v)| v).collect()
}
fn close() {
    m::set("composer-open", json!(false));
    dom::hidden("skill-hints-panel", true);
    dom::attr("prompt", "aria-expanded", "false");
}
pub async fn update() {
    let text = dom::value("prompt");
    let caret = dom::get("prompt")
        .and_then(|e| e.dyn_into::<web_sys::HtmlTextAreaElement>().ok())
        .and_then(|e| e.selection_start().ok().flatten())
        .map(|n| byte_caret(&text, n))
        .unwrap_or(text.len());
    let provider = m::read(|s| s.selected["provider"].as_str().map(str::to_owned))
        .unwrap_or_else(|| dom::value("provider"));
    let Some(t) = token(&text, caret, provider == "codex") else {
        close();
        return;
    };
    let Some(url) = target(t.trigger == '@') else {
        close();
        return;
    };
    let signature = format!("{}:{}:{}:{}", url, t.start, t.trigger, t.query);
    if m::data("composer-dismissed") == signature {
        close();
        return;
    }
    m::set(
        "composer-token",
        json!({"start":t.start,"end":t.end,"trigger":t.trigger.to_string(),"query":t.query,"signature":signature}),
    );
    m::set("composer-open", json!(true));
    dom::hidden("skill-hints-panel", false);
    dom::attr("prompt", "aria-expanded", "true");
    if t.trigger == '@' {
        let sep = if url.contains('?') { '&' } else { '?' };
        let path = format!("{url}{sep}q={}", dom::enc(&t.query));
        dom::later("composer-files", 120, move || {
            spawn_local(async move {
                load(&path, true).await;
            })
        });
    } else {
        let now = js_sys::Date::now();
        let cached = m::data("composer-cache");
        if cached["url"] == url && now - cached["at"].as_f64().unwrap_or(0.) < 30000. {
            present(&cached["data"], false);
        } else {
            load(&url, false).await;
        }
    }
}
async fn load(path: &str, files: bool) {
    dom::text(
        "skill-hints-status",
        if files {
            "Searching files…"
        } else {
            "Loading commands…"
        },
    );
    match api::scoped("composer", path).await {
        Ok((e, v)) if m::current("composer", e) => {
            if !files {
                m::set(
                    "composer-cache",
                    json!({"url":path,"at":js_sys::Date::now(),"data":v}),
                );
            }
            present(&v, files);
        }
        Ok(_) => {}
        Err(e) if !e.aborted => {
            dom::text("skill-hints-status", &e.message);
            dom::clear("skill-hints");
        }
        Err(_) => {}
    }
}
fn present(data: &V, files: bool) {
    let token = m::data("composer-token");
    let trigger = m::text(&token, "trigger");
    let items = if files {
        m::array(&data["items"])
    } else {
        rank(
            m::array(&data[if trigger == "$" { "skills" } else { "commands" }]),
            &m::text(&token, "query"),
        )
    };
    m::set("composer-items", json!(items));
    m::set("composer-active", json!(0));
    render();
}
fn render() {
    let items = m::array(&m::data("composer-items"));
    let active = m::data("composer-active").as_u64().unwrap_or(0) as usize;
    dom::clear("skill-hints");
    for (index, item) in items.iter().take(200).enumerate() {
        let name = item["name"]
            .as_str()
            .or_else(|| item["path"].as_str())
            .unwrap_or("");
        let e = dom::button(
            &format!(
                "{}{}{}",
                name,
                if item["description"].is_string() {
                    " · "
                } else {
                    ""
                },
                m::text(item, "description")
            ),
            "composer-choose",
            json!(index),
        );
        let _ = e.set_attribute("role", "option");
        let _ = e.set_attribute("id", &format!("composer-option-{index}"));
        let _ = e.set_attribute(
            "aria-selected",
            if index == active { "true" } else { "false" },
        );
        dom::append("skill-hints", &e);
    }
    dom::text("skill-hints-status", &format!("{} matches", items.len()));
    dom::attr(
        "prompt",
        "aria-activedescendant",
        &format!("composer-option-{active}"),
    );
}
async fn choose(index: usize, run: bool) {
    let Some(item) = m::array(&m::data("composer-items")).get(index).cloned() else {
        return;
    };
    let t = m::data("composer-token");
    let text = dom::value("prompt");
    let start = t["start"].as_u64().unwrap_or(0) as usize;
    let end = t["end"].as_u64().unwrap_or(0) as usize;
    if start > end
        || end > text.len()
        || !text.is_char_boundary(start)
        || !text.is_char_boundary(end)
    {
        close();
        return;
    }
    let trigger = m::text(&t, "trigger");
    let folder = trigger == "@" && item["kind"] == "folder";
    let name = if trigger == "@" {
        m::text(&item, "path")
    } else {
        m::text(&item, "name")
    };
    let after = &text[end..];
    let replacement = format!(
        "{trigger}{name}{}",
        if folder || after.starts_with(char::is_whitespace) {
            ""
        } else {
            " "
        }
    );
    let result = format!("{}{}{}", &text[..start], replacement, after);
    let caret = (text[..start].encode_utf16().count() + replacement.encode_utf16().count()) as u32;
    dom::set_value("prompt", &result);
    if let Some(e) =
        dom::get("prompt").and_then(|e| e.dyn_into::<web_sys::HtmlTextAreaElement>().ok())
    {
        let _ = e.set_selection_range(caret, caret);
    }
    dom::focus("prompt");
    close();
    session::controls();
    session::save_preferences();
    if folder {
        update().await;
    } else if run && trigger == "/" {
        if item["kind"] == "page" {
            intercept(&result).await;
        } else {
            let hint = m::text(&item, "hint");
            if hint.is_empty() || hint.starts_with('[') || hint.to_lowercase().contains("optional")
            {
                session::send().await;
            }
        }
    }
}
pub fn key(e: &web_sys::KeyboardEvent) -> bool {
    if !m::yes(&m::data("composer-open"))
        || e.is_composing()
        || e.target()
            .and_then(|x| x.dyn_into::<web_sys::Element>().ok())
            .map(|e| e.id())
            .as_deref()
            != Some("prompt")
    {
        return false;
    }
    let count = m::array(&m::data("composer-items")).len();
    let index = m::data("composer-active").as_u64().unwrap_or(0) as usize;
    match e.key().as_str() {
        "Escape" => {
            let t = m::data("composer-token");
            m::set("composer-dismissed", t["signature"].clone());
            close();
            true
        }
        "ArrowDown" | "ArrowUp" if count > 0 => {
            let next = if e.key() == "ArrowDown" {
                (index + 1) % count
            } else {
                (index + count - 1) % count
            };
            m::set("composer-active", json!(next));
            render();
            true
        }
        "Tab" if count > 0 && !e.shift_key() => {
            spawn_local(async move {
                choose(index, false).await;
            });
            true
        }
        "Enter"
            if count > 0 && !e.shift_key() && !e.ctrl_key() && !e.meta_key() && !e.alt_key() =>
        {
            spawn_local(async move {
                choose(index, true).await;
            });
            true
        }
        _ => false,
    }
}
pub async fn intercept(text: &str) -> bool {
    let Some(_) = target(false) else { return false };
    let parts = text.split_whitespace().collect::<Vec<_>>();
    if parts.is_empty() || parts.len() > 2 || !parts[0].starts_with('/') {
        return false;
    }
    let name = &parts[0][1..];
    let argument = parts.get(1).copied().unwrap_or("");
    let provider = dom::value("provider");
    let listed = m::array(&m::data("composer-cache")["data"]["commands"])
        .into_iter()
        .find(|v| v["name"] == name || m::array(&v["aliases"]).iter().any(|a| a == name));
    let action = if let Some(v) = listed {
        if v["kind"] != "page" {
            return false;
        }
        m::text(&v, "action")
    } else {
        match name {
            "clear" | "reset" if provider == "claude" => "new",
            "new" if provider == "codex" || provider == "claude" => "new",
            "model" if provider != "cursor" => "model",
            "effort" if provider == "claude" => "effort",
            "memory" if provider == "codex" => "memory",
            "status" | "diff" | "resume" | "login" | "help" => name,
            "cost" if provider != "claude" => "status",
            _ => return false,
        }
        .into()
    };
    if !argument.is_empty() && !["model", "effort", "new"].contains(&action.as_str()) {
        return false;
    }
    dom::set_value("prompt", "");
    close();
    dom::error("composer-notice", "");
    match action.as_str() {
        "new" => {
            if !m::read(|s| s.sid.is_empty()) {
                session::new_session(None);
            }
        }
        "model" => {
            if !argument.is_empty() {
                session::models("", Some(argument.into()));
            }
            dom::hidden("session-settings", false);
            dom::focus("model-choice");
        }
        "effort" => {
            if !argument.is_empty() {
                dom::set_value("thinking-effort", argument);
            }
            dom::hidden("session-settings", false);
            dom::focus("thinking-effort");
        }
        "help" => {
            dom::set_value("prompt", "/");
            dom::focus("prompt");
            update().await;
        }
        "login" => {
            session::sign_in(true).await;
            let v = m::data("sign-in");
            let s = &v["states"][&provider];
            dom::error(
                "composer-notice",
                &format!(
                    "{} · {}. Sign in from a terminal with {}",
                    provider,
                    m::text(s, "state"),
                    s["login"].as_str().unwrap_or("the provider CLI")
                ),
            );
        }
        "memory" => session::open_view("memory-use", true).await,
        "resume" => {
            dom::class("sidebar", "open", true);
            dom::class("scrim", "open", true);
            dom::attr("menu-toggle", "aria-expanded", "true");
            if let Some(e) = dom::all("#history button").first() {
                if let Some(x) = e.dyn_ref::<web_sys::HtmlElement>() {
                    let _ = x.focus();
                }
            }
        }
        _ => {
            if m::read(|s| s.sid.is_empty()) {
                dom::error(
                    "composer-error",
                    "Start a session before opening its changes or usage.",
                );
            } else {
                session::open_view(if action == "diff" { "changes" } else { "usage" }, true).await;
            }
        }
    }
    session::controls();
    true
}
pub async fn event(kind: &str, id: &str, action: &str, payload: &V, _value: &str) -> bool {
    if (kind == "input" || kind == "click") && id == "prompt" {
        m::take("composer-dismissed");
        update().await;
        return true;
    }
    if kind == "click" && action == "composer-choose" {
        choose(payload.as_u64().unwrap_or(0) as usize, false).await;
        return true;
    }
    false
}
