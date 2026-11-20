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
fn normalize_command_name(raw: &str) -> String {
    raw.trim_start_matches('/').to_lowercase()
}

fn rank(items: Vec<V>, needle: &str) -> Vec<V> {
    let needle = normalize_command_name(needle);
    let mut out = vec![];
    for (index, item) in items.into_iter().enumerate() {
        let mut names = vec![normalize_command_name(&m::text(&item, "name"))];
        names.extend(
            m::array(&item["aliases"])
                .iter()
                .map(|x| normalize_command_name(&m::string(x))),
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
fn clear_active_descendant() {
    if let Some(prompt) = dom::get("prompt") {
        let _ = prompt.remove_attribute("aria-activedescendant");
    }
}
fn active_option_id(count: usize, active: usize) -> Option<String> {
    if count == 0 {
        None
    } else {
        Some(format!("composer-option-{}", active.min(count - 1)))
    }
}
fn next_option_index(count: usize, current: usize, down: bool) -> usize {
    if count == 0 {
        return 0;
    }
    let current = current.min(count - 1);
    if down {
        (current + 1) % count
    } else {
        (current + count - 1) % count
    }
}
fn scroll_option_into_view(option: &web_sys::Element) {
    let Some(panel) = dom::get("skill-hints-panel") else {
        return;
    };
    let Some(el) = option.dyn_ref::<web_sys::HtmlElement>() else {
        return;
    };
    let top = el.offset_top();
    let bottom = top + el.offset_height();
    let view_top = panel.scroll_top();
    let view_bottom = view_top + panel.client_height();
    if top < view_top {
        panel.set_scroll_top(top);
    } else if bottom > view_bottom {
        panel.set_scroll_top((bottom - panel.client_height()).max(0));
    }
}
fn select_active(previous: usize, next: usize, count: usize) {
    let next = if count == 0 { 0 } else { next.min(count - 1) };
    m::set("composer-active", json!(next));
    if previous != next {
        if let Some(prev_el) = dom::get(&format!("composer-option-{previous}")) {
            let _ = prev_el.set_attribute("aria-selected", "false");
        }
    }
    if let Some(next_id) = active_option_id(count, next) {
        if let Some(next_el) = dom::get(&next_id) {
            let _ = next_el.set_attribute("aria-selected", "true");
            scroll_option_into_view(&next_el);
        }
        dom::attr("prompt", "aria-activedescendant", &next_id);
    } else {
        clear_active_descendant();
    }
}
fn within_composer_or_hints(el: &web_sys::Element) -> bool {
    el.id() == "prompt" || el.closest("#skill-hints-panel").ok().flatten().is_some()
}

pub(crate) fn handle_pointer_down(target: &web_sys::Element) {
    if m::yes(&m::data("composer-open")) && !within_composer_or_hints(target) {
        close();
    }
}

pub(crate) fn handle_focus_in(target: &web_sys::Element) {
    dom::cancel_timer("composer-blur-check");
    if m::yes(&m::data("composer-open")) && !within_composer_or_hints(target) {
        close();
    }
}

pub(crate) fn handle_focus_out(target: &web_sys::Element, event: &web_sys::FocusEvent) {
    if !m::yes(&m::data("composer-open")) || !within_composer_or_hints(target) {
        return;
    }
    if let Some(next) = event
        .related_target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
    {
        if !within_composer_or_hints(&next) {
            close();
        }
        return;
    }
    dom::later("composer-blur-check", 0, || {
        if m::yes(&m::data("composer-open")) {
            let active_inside = dom::document()
                .active_element()
                .is_some_and(|el| within_composer_or_hints(&el));
            if !active_inside {
                close();
            }
        }
    });
}

pub(crate) fn close() {
    dom::cancel_timer("composer-blur-check");
    m::next("composer");
    m::set("composer-open", json!(false));
    m::take("composer-rendered-signature");
    dom::hidden("skill-hints-panel", true);
    dom::attr("prompt", "aria-expanded", "false");
    clear_active_descendant();
}
pub async fn update() {
    dom::cancel_timer("composer-blur-check");
    if m::read(|s| m::active(&s.selected)) || crate::inbox::needs_confirmation() {
        close();
        return;
    }
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
    if m::yes(&m::data("composer-open"))
        && m::data("composer-rendered-signature") == signature
        && (t.trigger == '@'
            || (m::data("composer-cache")["url"] == url
                && js_sys::Date::now() - m::data("composer-cache")["at"].as_f64().unwrap_or(0.)
                    < 30000.))
    {
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
            clear_active_descendant();
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
pub(crate) fn composer_hint_parts(item: &V) -> (String, String, String) {
    let name = item["name"]
        .as_str()
        .or_else(|| item["path"].as_str())
        .unwrap_or("")
        .to_owned();
    let hint = item["hint"]
        .as_str()
        .or_else(|| item["argumentHint"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("")
        .to_owned();
    let description = m::text(item, "description");
    (name, hint, description)
}

fn render() {
    let items = m::array(&m::data("composer-items"));
    let count = items.len().min(200);
    let active = if count == 0 {
        0
    } else {
        (m::data("composer-active").as_u64().unwrap_or(0) as usize).min(count - 1)
    };
    dom::clear("skill-hints");
    for (index, item) in items.iter().take(200).enumerate() {
        let (name, hint, description) = composer_hint_parts(item);
        let li = dom::el("li", "", "");
        let _ = li.set_attribute("role", "none");
        let btn = dom::button("", "composer-choose", json!(index));
        btn.set_class_name("skill-hint");
        let _ = btn.set_attribute("role", "option");
        let _ = btn.set_attribute("id", &format!("composer-option-{index}"));
        let _ = btn.set_attribute(
            "aria-selected",
            if index == active { "true" } else { "false" },
        );
        let title = dom::el("span", "skill-hint-name", &name);
        if !hint.is_empty() {
            dom::add(
                &title,
                &dom::el("span", "skill-hint-argument", &format!(" {hint}")),
            );
        }
        dom::add(&btn, &title);
        if !description.is_empty() {
            dom::add(
                &btn,
                &dom::el("span", "skill-hint-description", &description),
            );
        }
        dom::add(&li, &btn);
        dom::append("skill-hints", &li);
    }
    dom::text("skill-hints-status", &format!("{} matches", items.len()));
    m::set(
        "composer-rendered-signature",
        m::data("composer-token")["signature"].clone(),
    );
    if let Some(id) = active_option_id(count, active) {
        dom::attr("prompt", "aria-activedescendant", &id);
    } else {
        clear_active_descendant();
    }
}
pub(crate) fn format_composer_replacement(trigger: &str, item: &V, after: &str) -> (String, bool) {
    let folder = trigger == "@" && item["kind"] == "folder";
    let raw_name = if trigger == "@" {
        m::text(item, "path")
    } else {
        m::text(item, "name")
    };
    let clean_name = if trigger == "/" {
        raw_name.trim_start_matches('/')
    } else {
        raw_name.as_str()
    };
    let replacement = format!(
        "{trigger}{clean_name}{}",
        if folder || after.starts_with(char::is_whitespace) {
            ""
        } else {
            " "
        }
    );
    (replacement, folder)
}

pub(crate) fn should_autosend_command(item: &V) -> bool {
    let (_, hint, _) = composer_hint_parts(item);
    hint.is_empty() || hint.starts_with('[') || hint.to_ascii_lowercase().contains("optional")
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
    let after = &text[end..];
    let (replacement, folder) = format_composer_replacement(&trigger, &item, after);
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
        } else if should_autosend_command(&item) {
            session::send().await;
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
    let count = m::array(&m::data("composer-items")).len().min(200);
    let index = m::data("composer-active").as_u64().unwrap_or(0) as usize;
    match e.key().as_str() {
        "Escape" => {
            let t = m::data("composer-token");
            m::set("composer-dismissed", t["signature"].clone());
            close();
            true
        }
        "ArrowDown" | "ArrowUp" if count > 0 => {
            let next = next_option_index(count, index, e.key() == "ArrowDown");
            select_active(index, next, count);
            true
        }
        "Tab" if count > 0 && !e.shift_key() => {
            let chosen = index.min(count - 1);
            spawn_local(async move {
                choose(chosen, false).await;
            });
            true
        }
        "Enter"
            if count > 0 && !e.shift_key() && !e.ctrl_key() && !e.meta_key() && !e.alt_key() =>
        {
            let chosen = index.min(count - 1);
            spawn_local(async move {
                choose(chosen, true).await;
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
        .find(|v| {
            v["name"]
                .as_str()
                .is_some_and(|s| s.trim_start_matches('/') == name)
                || m::array(&v["aliases"]).iter().any(|a| {
                    a.as_str()
                        .is_some_and(|s| s.trim_start_matches('/') == name)
                })
        });
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
                session::mark_dirty("model");
            }
            dom::hidden("session-settings", false);
            dom::attr("session-settings-toggle", "aria-expanded", "true");
            dom::focus("model-choice");
        }
        "effort" => {
            if !argument.is_empty() {
                dom::set_value("thinking-effort", argument);
                session::mark_dirty("model");
            }
            dom::hidden("session-settings", false);
            dom::attr("session-settings-toggle", "aria-expanded", "true");
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
            session::set_sidebar_open(true, true);
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
pub async fn event(kind: &str, id: &str, action: &str, payload: &V, value: &str) -> bool {
    if (kind == "input" || kind == "click") && id == "prompt" {
        m::take("composer-dismissed");
        update().await;
        return true;
    }
    if kind == "input" && id == "command-palette-input" {
        m::write(|s| {
            s.workspace.command_palette_query = value.to_owned();
            s.workspace.command_palette_selected = 0;
        });
        render_palette();
        return true;
    }
    if kind == "click" && action == "composer-choose" {
        choose(payload.as_u64().unwrap_or(0) as usize, false).await;
        return true;
    }
    if kind == "click" && action == "command-palette-open" {
        open_palette();
        return true;
    }
    if kind == "click" && (id == "command-palette" || action == "command-palette-close") {
        close_palette();
        return true;
    }
    if kind == "click" && action == "palette-run" {
        let idx = payload.as_u64().unwrap_or(0) as usize;
        let entry = PALETTE_ENTRIES
            .with(|entries| entries.borrow().get(idx).cloned())
            .or_else(|| {
                let query = m::read(|s| s.workspace.command_palette_query.clone());
                collect_palette_entries(&query).into_iter().nth(idx)
            });
        if let Some(entry) = entry {
            execute_palette_entry(entry).await;
        }
        return true;
    }
    false
}

/// Global Cmd/Ctrl+K command palette item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteEntry {
    pub id: String,
    pub category: String,
    pub title: String,
    pub subtitle: String,
    pub action: String,
    pub payload: V,
}

const PALETTE_VIEWS: &[(&str, &str, &str)] = &[
    (
        "sessions",
        "Sessions chat",
        "Open active session conversation",
    ),
    (
        "changes",
        "Changes & delivery",
        "Review diffs, checks, and git delivery",
    ),
    (
        "checks",
        "Session checks",
        "Run and inspect verification checks",
    ),
    (
        "usage",
        "Usage & context",
        "Inspect token and cost breakdown",
    ),
    (
        "systems",
        "System Orchestration",
        "Manage multi-service systems",
    ),
    (
        "system-changes",
        "System changes",
        "Inspect orchestrated service changes",
    ),
    (
        "omnigraph",
        "Context Graph Studio",
        "Explore cross-session knowledge graph",
    ),
    (
        "creator",
        "Infrastructure Creator",
        "Generate target project infrastructure",
    ),
    (
        "setup",
        "Setup & install",
        "Install or update target accelerators",
    ),
    (
        "skills",
        "Skills catalog",
        "Browse and install agent skills",
    ),
    (
        "skill-capsules",
        "Skill capsules",
        "Inspect compiled plans of repeated skill runs",
    ),
    (
        "create-skill",
        "Create skill",
        "Author a custom repository skill",
    ),
    (
        "memory-use",
        "Memory usage",
        "Inspect memory bank utilization",
    ),
    ("memory", "Memory Bank", "Browse durable knowledge chunks"),
    (
        "brain",
        "Project Brain",
        "Manage active findings and decisions",
    ),
    (
        "context",
        "Local Context Engine",
        "Search indexed repository context",
    ),
    (
        "accelerators",
        "Accelerators overview",
        "Inspect installed accelerator packs",
    ),
];

pub fn palette_open() -> bool {
    m::read(|s| s.workspace.command_palette_open)
}

/// Builds and ranks filtered entries across quick actions, views, workspace projects, and recent sessions.
pub fn collect_palette_entries(query: &str) -> Vec<PaletteEntry> {
    let q = query.trim().to_ascii_lowercase();
    let mut candidates = vec![
        PaletteEntry {
            id: "act-new-session".into(),
            category: "Action".into(),
            title: "New session".into(),
            subtitle: "Start a fresh chat in the current workspace project".into(),
            action: "palette-new-session".into(),
            payload: V::Null,
        },
        PaletteEntry {
            id: "act-approvals".into(),
            category: "Action".into(),
            title: "Approvals queue".into(),
            subtitle: "Review pending permissions and remembered session rules".into(),
            action: "palette-approvals".into(),
            payload: V::Null,
        },
        PaletteEntry {
            id: "act-refresh".into(),
            category: "Action".into(),
            title: "Refresh sessions".into(),
            subtitle: "Sync active session list and waiting counts".into(),
            action: "palette-refresh".into(),
            payload: V::Null,
        },
        PaletteEntry {
            id: "act-tab-graph".into(),
            category: "Action".into(),
            title: "Workspace pane: Code graph".into(),
            subtitle: "Switch right pane to Code graph".into(),
            action: "palette-right-tab".into(),
            payload: json!("graph"),
        },
        PaletteEntry {
            id: "act-tab-diff".into(),
            category: "Action".into(),
            title: "Workspace pane: Changes & Diff".into(),
            subtitle: "Switch right pane to grouped diff viewer".into(),
            action: "palette-right-tab".into(),
            payload: json!("diff"),
        },
        PaletteEntry {
            id: "act-tab-checks".into(),
            category: "Action".into(),
            title: "Workspace pane: Checks".into(),
            subtitle: "Switch right pane to verification checks".into(),
            action: "palette-right-tab".into(),
            payload: json!("checks"),
        },
        PaletteEntry {
            id: "act-theme".into(),
            category: "Action".into(),
            title: "Toggle theme (System / Dark / Light)".into(),
            subtitle: "Cycle workspace color scheme".into(),
            action: "palette-theme".into(),
            payload: V::Null,
        },
    ];

    for &(view_id, title, subtitle) in PALETTE_VIEWS {
        candidates.push(PaletteEntry {
            id: format!("view-{view_id}"),
            category: "View".into(),
            title: title.to_owned(),
            subtitle: subtitle.to_owned(),
            action: "palette-view".into(),
            payload: json!(view_id),
        });
    }

    let projects = m::read(|s| m::array(&s.boot["projects"]));
    for proj in projects {
        let pid = m::text(&proj, "id");
        if pid.is_empty() {
            continue;
        }
        let name = m::summary(&proj);
        candidates.push(PaletteEntry {
            id: format!("proj-{pid}"),
            category: "Project".into(),
            title: format!("Switch project: {name}"),
            subtitle: m::text(&proj, "path"),
            action: "palette-project".into(),
            payload: json!(pid),
        });
    }

    let sessions = m::read(|s| s.sessions.clone());
    for sess in sessions.into_iter().take(30) {
        let sid = m::text(&sess, "id");
        if sid.is_empty() {
            continue;
        }
        let title = m::summary(&sess);
        let status = m::label(&m::text(&sess, "status"));
        let proj = m::text(&sess, "project_id");
        candidates.push(PaletteEntry {
            id: format!("sess-{sid}"),
            category: "Session".into(),
            title: if title.is_empty() { sid.clone() } else { title },
            subtitle: if proj.is_empty() {
                status
            } else {
                format!("{status} · {proj}")
            },
            action: "palette-session".into(),
            payload: json!(sid),
        });
    }

    if q.is_empty() {
        candidates.truncate(40);
        return candidates;
    }

    let mut ranked = Vec::<(u8, PaletteEntry)>::new();
    for item in candidates {
        let t_low = item.title.to_ascii_lowercase();
        let s_low = item.subtitle.to_ascii_lowercase();
        let c_low = item.category.to_ascii_lowercase();
        if t_low.starts_with(&q) {
            ranked.push((0, item));
        } else if t_low.contains(&q) || c_low.contains(&q) {
            ranked.push((1, item));
        } else if s_low.contains(&q) {
            ranked.push((2, item));
        }
    }
    ranked.sort_by_key(|(score, _)| *score);
    ranked
        .into_iter()
        .take(40)
        .map(|(_, entry)| entry)
        .collect()
}

thread_local! {
    static PALETTE_ENTRIES: std::cell::RefCell<Vec<PaletteEntry>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

pub fn open_palette() {
    close();
    dom::save_focus_for_overlay("command-palette");
    m::write(|s| {
        s.workspace.command_palette_open = true;
        s.workspace.command_palette_query.clear();
        s.workspace.command_palette_selected = 0;
    });
    dom::set_value("command-palette-input", "");
    dom::attr("command-palette-input", "aria-expanded", "true");
    dom::hidden("command-palette", false);
    render_palette();
    dom::focus("command-palette-input");
}

pub fn close_palette() {
    let was_open = palette_open()
        || dom::get("command-palette")
            .is_some_and(|e| !e.has_attribute("hidden") || e.has_attribute("open"));
    m::write(|s| {
        s.workspace.command_palette_open = false;
        s.workspace.command_palette_query.clear();
        s.workspace.command_palette_selected = 0;
    });
    PALETTE_ENTRIES.with(|entries| entries.borrow_mut().clear());
    dom::attr("command-palette-input", "aria-expanded", "false");
    if let Some(input) = dom::get("command-palette-input") {
        let _ = input.remove_attribute("aria-activedescendant");
    }
    if !was_open {
        return;
    }
    dom::hidden("command-palette", true);
}

pub fn render_palette() {
    let Some(list) = dom::get("command-palette-list") else {
        return;
    };
    let (query, selected) = m::read(|s| {
        (
            s.workspace.command_palette_query.clone(),
            s.workspace.command_palette_selected,
        )
    });
    let entries = collect_palette_entries(&query);
    let active_idx = if entries.is_empty() {
        0
    } else {
        selected.min(entries.len() - 1)
    };
    m::write(|s| s.workspace.command_palette_selected = active_idx);
    PALETTE_ENTRIES.with(|slot| *slot.borrow_mut() = entries.clone());
    list.set_text_content(None);
    if entries.is_empty() {
        dom::add(
            &list,
            &dom::el(
                "p",
                "muted command-palette-empty",
                "No matching commands, views, projects, or sessions.",
            ),
        );
        if let Some(input) = dom::get("command-palette-input") {
            let _ = input.remove_attribute("aria-activedescendant");
        }
        return;
    }
    for (idx, item) in entries.iter().enumerate() {
        let opt_id = format!("command-palette-option-{idx}");
        let is_selected = idx == active_idx;
        let btn = dom::button("", "palette-run", json!(idx));
        btn.set_class_name("command-palette-item");
        let _ = btn.set_attribute("id", &opt_id);
        let _ = btn.set_attribute("role", "option");
        let _ = btn.set_attribute("aria-selected", if is_selected { "true" } else { "false" });
        let head = dom::el("span", "command-palette-item-head", "");
        dom::add(
            &head,
            &dom::el("span", "command-palette-cat", &item.category),
        );
        dom::add(
            &head,
            &dom::el("strong", "command-palette-title", &item.title),
        );
        dom::add(&btn, &head);
        if !item.subtitle.is_empty() {
            dom::add(
                &btn,
                &dom::el("span", "command-palette-sub", &item.subtitle),
            );
        }
        dom::add(&list, &btn);
    }
    let active_opt_id = format!("command-palette-option-{active_idx}");
    dom::attr(
        "command-palette-input",
        "aria-activedescendant",
        &active_opt_id,
    );
    if let Some(active_el) = dom::get(&active_opt_id) {
        active_el.scroll_into_view();
    }
}

pub async fn execute_palette_entry(entry: PaletteEntry) {
    close_palette();
    match entry.action.as_str() {
        "palette-new-session" => {
            session::new_session(None);
            session::open_view("sessions", true).await;
        }
        "palette-approvals" => {
            crate::run::open_approvals_drawer().await;
        }
        "palette-refresh" => {
            session::refresh().await;
        }
        "palette-right-tab" => {
            if m::read(|s| s.view != "sessions") {
                session::open_view("sessions", true).await;
            }
            session::set_workspace_right_tab(entry.payload.as_str().unwrap_or("graph"));
        }
        "palette-theme" => {
            let current = dom::storage("harness.theme.v1").unwrap_or_else(|| "system".into());
            let next = match current.as_str() {
                "dark" => "light",
                "light" => "system",
                _ => "dark",
            };
            dom::save(
                "harness.theme.v1",
                if next == "system" { None } else { Some(next) },
            );
            if let Some(radio) = dom::document()
                .query_selector(&format!("input[name=\"theme\"][value=\"{next}\"]"))
                .ok()
                .flatten()
                .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
            {
                radio.set_checked(true);
            }
            session::apply_theme(next);
        }
        "palette-view" => {
            session::open_view(entry.payload.as_str().unwrap_or("sessions"), true).await;
        }
        "palette-project" => {
            if let Some(pid) = entry.payload.as_str() {
                session::switch_project_from_shell(pid).await;
            }
        }
        "palette-session" => {
            if let Some(sid) = entry.payload.as_str() {
                session::select(sid).await;
            }
        }
        _ => {}
    }
}

pub(crate) fn is_palette_button_defer_target(id: &str, has_palette_item_ancestor: bool) -> bool {
    id == "command-palette-close" || has_palette_item_ancestor
}

fn should_defer_palette_enter(target: &web_sys::Element) -> bool {
    is_palette_button_defer_target(
        &target.id(),
        target
            .closest(".command-palette-item")
            .ok()
            .flatten()
            .is_some(),
    )
}

pub(crate) fn palette_boundary_index(count: usize, end: bool) -> usize {
    if end {
        count.saturating_sub(1)
    } else {
        0
    }
}

pub(crate) fn palette_selection_step(count: usize, current: usize, key: &str) -> Option<usize> {
    if count == 0 {
        return None;
    }
    match key {
        "ArrowDown" => Some(next_option_index(count, current, true)),
        "ArrowUp" => Some(next_option_index(count, current, false)),
        "Home" => Some(palette_boundary_index(count, false)),
        "End" => Some(palette_boundary_index(count, true)),
        _ => None,
    }
}

fn move_palette_selection(key: &str) -> bool {
    let count = PALETTE_ENTRIES.with(|entries| entries.borrow().len());
    let prev = m::read(|s| s.workspace.command_palette_selected);
    let Some(next) = palette_selection_step(count, prev, key) else {
        return matches!(key, "ArrowDown" | "ArrowUp" | "Home" | "End");
    };
    m::write(|s| s.workspace.command_palette_selected = next);
    if prev != next {
        if let Some(prev_el) = dom::get(&format!("command-palette-option-{prev}")) {
            let _ = prev_el.set_attribute("aria-selected", "false");
        }
    }
    let next_id = format!("command-palette-option-{next}");
    if let Some(next_el) = dom::get(&next_id) {
        let _ = next_el.set_attribute("aria-selected", "true");
        next_el.scroll_into_view();
    }
    dom::attr("command-palette-input", "aria-activedescendant", &next_id);
    true
}

/// Global keyboard handler for `Cmd/Ctrl+K` Command Palette and palette navigation.
pub fn palette_key(event: &web_sys::KeyboardEvent) -> bool {
    if (event.ctrl_key() || event.meta_key())
        && !event.alt_key()
        && event.key().eq_ignore_ascii_case("k")
        && !event.is_composing()
    {
        if palette_open() {
            close_palette();
        } else {
            open_palette();
        }
        return true;
    }
    if !palette_open() {
        return false;
    }
    match event.key().as_str() {
        "Escape" => {
            close_palette();
            true
        }
        key @ ("ArrowDown" | "ArrowUp" | "Home" | "End") => move_palette_selection(key),
        "Enter" => {
            if event
                .target()
                .and_then(|v| v.dyn_into::<web_sys::Element>().ok())
                .is_some_and(|target| should_defer_palette_enter(&target))
            {
                return false;
            }
            let (query, selected) = m::read(|s| {
                (
                    s.workspace.command_palette_query.clone(),
                    s.workspace.command_palette_selected,
                )
            });
            let entry = PALETTE_ENTRIES
                .with(|entries| entries.borrow().get(selected).cloned())
                .or_else(|| collect_palette_entries(&query).into_iter().nth(selected));
            if let Some(entry) = entry {
                spawn_local(async move {
                    execute_palette_entry(entry).await;
                });
            }
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_option_id_omits_when_empty_and_clamps_otherwise() {
        assert_eq!(active_option_id(0, 0), None);
        assert_eq!(active_option_id(0, 5), None);
        assert_eq!(
            active_option_id(3, 1),
            Some("composer-option-1".to_string())
        );
        assert_eq!(
            active_option_id(3, 9),
            Some("composer-option-2".to_string())
        );
    }

    #[test]
    fn next_option_index_wraps_within_visible_items() {
        assert_eq!(next_option_index(0, 0, true), 0);
        assert_eq!(next_option_index(3, 0, true), 1);
        assert_eq!(next_option_index(3, 2, true), 0);
        assert_eq!(next_option_index(3, 0, false), 2);
        assert_eq!(next_option_index(3, 2, false), 1);
        assert_eq!(next_option_index(3, 99, false), 1);
    }

    #[test]
    fn command_palette_collects_and_ranks_views_projects_and_sessions() {
        m::write(|s| {
            s.boot = json!({
                "projects": [{"id": "laravel-app", "name": "Laravel API", "path": "/srv/api"}]
            });
            s.sessions = vec![
                json!({"id": "sess-9", "title": "Fix Billing Invoice", "project_id": "laravel-app", "status": "running"}),
            ];
        });
        let all = collect_palette_entries("");
        assert!(all.iter().any(|e| e.action == "palette-new-session"));
        assert!(all.iter().any(|e| e.action == "palette-project"));
        assert!(all.iter().any(|e| e.action == "palette-session"));

        let billing = collect_palette_entries("billing");
        assert_eq!(billing.len(), 1);
        assert_eq!(billing[0].payload, json!("sess-9"));

        let laravel = collect_palette_entries("laravel");
        assert!(laravel.iter().any(|e| e.payload == json!("laravel-app")));
    }

    #[test]
    fn enter_key_defers_to_close_and_option_buttons_only() {
        assert!(is_palette_button_defer_target(
            "command-palette-close",
            false
        ));
        assert!(is_palette_button_defer_target(
            "command-palette-option-0",
            true
        ));
        assert!(!is_palette_button_defer_target(
            "command-palette-input",
            false
        ));
    }

    #[test]
    fn palette_home_end_boundaries_stay_in_range() {
        assert_eq!(palette_boundary_index(5, false), 0);
        assert_eq!(palette_boundary_index(5, true), 4);
        assert_eq!(palette_boundary_index(0, false), 0);
        assert_eq!(palette_boundary_index(0, true), 0);

        assert_eq!(
            composer_hint_parts(&json!({
                "name": "/model",
                "argumentHint": "[model]",
                "description": "Choose the model"
            })),
            ("/model".into(), "[model]".into(), "Choose the model".into())
        );
        assert_eq!(
            composer_hint_parts(&json!({
                "path": "src/main.rs",
                "hint": " line "
            })),
            ("src/main.rs".into(), "line".into(), String::new())
        );
    }

    #[test]
    fn choose_replacement_and_autosend_handle_leading_slash_and_argument_hint() {
        // Leading '/' in item["name"] must not produce "//model " and rank() matches prefix.
        let model_cmd = json!({
            "name": "/model",
            "argumentHint": "[model]",
            "description": "Switch model"
        });
        assert_eq!(
            format_composer_replacement("/", &model_cmd, ""),
            ("/model ".to_owned(), false)
        );
        assert!(should_autosend_command(&model_cmd));

        // Required argumentHint must NOT auto-send even when "hint" field is absent.
        let required_arg_cmd = json!({
            "name": "/review",
            "argumentHint": "<pr-number>"
        });
        assert!(!should_autosend_command(&required_arg_cmd));
        assert!(should_autosend_command(&json!({
            "name": "help",
            "argumentHint": "optional topic"
        })));

        let ranked = rank(
            vec![
                json!({"name": "clear", "description": "Reset chat"}),
                json!({"name": "/model", "description": "Choose model"}),
            ],
            "mod",
        );
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0]["name"], "/model");
    }

    #[test]
    fn palette_selection_step_navigates_in_place_and_ignores_empty_lists() {
        assert_eq!(palette_selection_step(0, 0, "ArrowDown"), None);
        assert_eq!(palette_selection_step(4, 0, "ArrowDown"), Some(1));
        assert_eq!(palette_selection_step(4, 3, "ArrowDown"), Some(0));
        assert_eq!(palette_selection_step(4, 0, "ArrowUp"), Some(3));
        assert_eq!(palette_selection_step(4, 2, "Home"), Some(0));
        assert_eq!(palette_selection_step(4, 1, "End"), Some(3));
        assert_eq!(palette_selection_step(4, 1, "Escape"), None);
    }
}
