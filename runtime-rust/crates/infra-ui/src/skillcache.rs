//! SkillCache in the Harness page: the composer's option, the preview of the
//! next turn, the run view's tab and the stream's cards. The capsules' own
//! view is `management::capsules`.
//!
//! Everything a person sees here is markup built from server data, so every
//! value is escaped, and colors come from `app.css` tokens through `data-tone`.
use crate::{api, dom, model as m, run, session};
use dom::escape_html as esc;
use serde_json::{json, Value as V};
use wasm_bindgen_futures::spawn_local;

/// The session option's values and what the chip calls them.
pub(crate) const MODES: [(&str, &str); 4] = [
    ("off", "Off"),
    ("observe", "Observe"),
    ("auto", "Auto"),
    ("warm_only", "Warm only"),
];

pub(crate) fn mode_label(mode: &str) -> &'static str {
    MODES
        .iter()
        .find(|(value, _)| *value == mode)
        .map_or("Off", |(_, label)| label)
}

/// SkillCache applies to an ordinary session: the native workflow, without a
/// Clash challenger. The specialized workflows run their own protocols.
pub(crate) fn eligible(workflow: &str, clash: bool) -> bool {
    workflow == "native" && !clash
}

/// What a new session or a follow-up sends as its `skillcache` option, or
/// nothing when the server's current value already says it.
pub(crate) fn request(
    mode: &str,
    model: &str,
    eligible: bool,
    stored: &V,
    follow: bool,
) -> Option<V> {
    let stored_mode = stored["mode"].as_str().unwrap_or("off");
    if !MODES.iter().any(|(value, _)| *value == mode) {
        return None;
    }
    if mode == "off" || !eligible {
        // A new session is off by saying nothing. A session that opted in
        // has to be told, also when the turn stops being an ordinary one:
        // the server refuses a Clash or SDD turn of a session that still has
        // SkillCache on, and the chip already shows it as off.
        return (follow && stored_mode != "off").then(|| json!({"mode": "off"}));
    }
    let model = model.trim();
    if follow && stored_mode == mode && stored["compiler"]["model"].as_str().unwrap_or("") == model
    {
        return None;
    }
    Some(json!({
        "mode": mode,
        "compiler": {
            "provider": "claude",
            "model": if model.is_empty() { V::Null } else { json!(model) },
        },
    }))
}

/// What the panel says about the chosen mode, in the terms of what is kept
/// and what is run.
pub(crate) fn note(mode: &str, provider: &str, eligible: bool) -> String {
    if !eligible {
        return "SkillCache applies to ordinary sessions: not to Fleet, Clash, SDD, plan or review workflows.".into();
    }
    let base = match mode {
        "observe" => "A turn that uses a skill keeps a redacted trace of its tool calls in the Harness's private state for 30 days, so that it can be compiled into a capsule. Nothing is added to the project, and nothing changes how the turn runs.",
        "auto" => "A message that names one skill with a qualified capsule runs the capsule's plan instead of reading the skill again. Every other turn runs as usual and keeps a redacted trace of its tool calls for 30 days.",
        "warm_only" => "A message that names one skill with a qualified capsule runs the capsule's plan instead of reading the skill again. Nothing is recorded.",
        _ => return "Off. Nothing is recorded and no capsule is used.".into(),
    };
    let provider_note = match (mode, provider) {
        ("observe", _) => "",
        (_, "codex") => " In Codex, name the skill as `$skill`.",
        (_, "claude") => " In Claude, name the skill as `$skill`. A Claude `/command` runs natively, so that turn is only observed.",
        (_, "cursor") => " Cursor turns are observed and never run a capsule.",
        _ => "",
    };
    format!("{base}{provider_note}")
}

/// What the composer asks the preflight to preview: only a mode that can run
/// a capsule has anything to preview.
pub(crate) fn preview_request(
    mode: &str,
    eligible: bool,
    provider: &str,
    session_mode: &str,
    permissions: &str,
) -> Option<V> {
    (eligible && matches!(mode, "auto" | "warm_only")).then(|| {
        json!({
            "mode": mode,
            "provider": provider,
            "session_mode": session_mode,
            "permissions": permissions,
        })
    })
}

/// Whether a prompt is worth a preview: it names a skill or a command.
pub(crate) fn names_a_skill(prompt: &str) -> bool {
    prompt.split_whitespace().any(|word| {
        let mut chars = word.chars();
        matches!(chars.next(), Some('$' | '/'))
            && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
    })
}

/// The pill beside the risk check: what the next turn would do, and its tone.
pub(crate) fn preview_pill(preview: &V) -> Option<(String, &'static str)> {
    match preview["decision"].as_str()? {
        "hit" => {
            let capsule = &preview["capsule"];
            let name = capsule["mu"].as_str().unwrap_or("capsule");
            let form = if capsule["form"] == "textual" {
                " (written procedure)"
            } else {
                ""
            };
            Some((format!("Warm: {name}{form}"), "ok"))
        }
        "miss" => match preview["reason"].as_str().unwrap_or("") {
            "stale" => Some(("Stale: cold".into(), "warn")),
            "no_single_skill" => None,
            // A `/command` goes to the CLI as typed; a `$skill` mention of
            // the same skill is what can run its capsule.
            "native_route" => Some((
                "Miss: cold (native command; use $skill to run the capsule)".into(),
                "muted",
            )),
            reason => Some((format!("Miss: cold ({})", miss_reason(reason)), "muted")),
        },
        _ => None,
    }
}

/// A miss reason in a person's words.
pub(crate) fn miss_reason(reason: &str) -> &str {
    match reason {
        "native_route" => "native command",
        "plan_mode" => "plan mode",
        "provider" => "this provider is observed only",
        "codex_manual" => "Codex Manual",
        "no_single_skill" => "no single skill named",
        "no_capsule" => "no qualified capsule",
        "ambiguous" => "several capsules match",
        "stale" => "the capsule is stale",
        "network" => "the capsule needs network",
        "unavailable" => "SkillCache is unavailable",
        other => other,
    }
}

pub(crate) fn phase_label(phase: &str) -> &'static str {
    match phase {
        "hit" => "Capsule used",
        "miss" => "Ran cold",
        "observed" => "Trace kept",
        "not_observed" => "No trace",
        "started" => "Plan started",
        "needs_bindings" => "Values needed",
        "your_steps" => "Agent's steps",
        "program_completed" => "Program ran",
        "program_failed" => "Step handed back",
        "interrupted" => "Step resumed",
        "complete" => "Plan complete",
        "needs_recompile" => "Needs recompile",
        _ => "SkillCache",
    }
}

pub(crate) fn phase_tone(event: &V) -> &'static str {
    match event["phase"].as_str().unwrap_or("") {
        "hit" | "complete" | "program_completed" | "observed" => "ok",
        "program_failed" | "needs_recompile" => "warn",
        "miss" if event["reason"] == "stale" => "warn",
        _ => "muted",
    }
}

/// The stream shows what a person may act on; the run view's tab lists all.
pub(crate) fn in_stream(event: &V) -> bool {
    match event["phase"].as_str().unwrap_or("") {
        "hit" | "observed" | "needs_bindings" | "your_steps" | "program_failed" | "complete"
        | "needs_recompile" => true,
        "miss" => matches!(
            event["reason"].as_str().unwrap_or(""),
            "stale" | "ambiguous" | "network" | "codex_manual" | "plan_mode" | "unavailable"
        ),
        _ => false,
    }
}

/// An event's sentence without the product's name in front: the card and the
/// tab are already titled.
pub(crate) fn event_text(event: &V) -> String {
    let text = event["text"].as_str().unwrap_or("");
    text.strip_prefix("SkillCache: ")
        .or_else(|| text.strip_prefix("SkillCache "))
        .map(|rest| {
            let mut chars = rest.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .unwrap_or_else(|| text.to_string())
}

/// A fingerprint of rendered markup. An element whose markup did not change
/// is left alone: replacing it would drop a click that is in progress on one
/// of its buttons, and the focus of someone using the keyboard.
pub(crate) fn signature(markup: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in markup.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{}:{hash:016x}", markup.len())
}

/// Puts `markup` into `element` unless it is what the element already shows.
pub(crate) fn set_markup(element: &web_sys::Element, markup: &str) {
    let fingerprint = signature(markup);
    if element.get_attribute("data-rendered").as_deref() == Some(fingerprint.as_str()) {
        return;
    }
    element.set_inner_html(markup);
    let _ = element.set_attribute("data-rendered", &fingerprint);
}

fn action_button(label: &str, action: &str, payload: &V) -> String {
    format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"{action}\" data-payload=\"{}\">{}</button>",
        esc(&payload.to_string()),
        esc(label)
    )
}

/// What can be done about one event, given the session's SkillCache state.
/// An observed turn can be compiled once; a capsule can be opened.
pub(crate) fn event_actions_html(event: &V, state: &V) -> String {
    let mut out = String::new();
    // The capsule is offered once per turn, where it enters and where it
    // asks for attention, not on every step of its plan.
    if matches!(event["phase"].as_str(), Some("hit" | "needs_recompile")) {
        if let Some(capsule) = event["capsule_id"].as_str().filter(|id| !id.is_empty()) {
            out.push_str(&action_button(
                "Open capsule",
                "skillcache-open-capsule",
                &json!(capsule),
            ));
        }
    }
    if event["phase"] != "observed" {
        return out;
    }
    let Some(generation) = event["generation"].as_str().filter(|g| !g.is_empty()) else {
        return out;
    };
    if !state.is_object() {
        return out;
    }
    let compile = m::array(&state["compiles"])
        .into_iter()
        .find(|compile| compile["source_generation"] == generation);
    if let Some(compile) = compile {
        let status = compile["status"].as_str().unwrap_or("queued");
        let outcome = compile["outcome"].as_str().unwrap_or("");
        out.push_str(&format!(
            "<span class=\"skillcache-compile\" data-status=\"{}\">Compilation {}{}</span>",
            esc(status),
            esc(status),
            if outcome.is_empty() {
                String::new()
            } else {
                format!(": {}", esc(outcome))
            }
        ));
        if let Some(capsule) = compile["capsule_id"].as_str().filter(|id| !id.is_empty()) {
            out.push_str(&action_button(
                "Open capsule",
                "skillcache-open-capsule",
                &json!(capsule),
            ));
        }
        return out;
    }
    match m::array(&state["traces"])
        .into_iter()
        .find(|trace| trace["generation"] == generation)
    {
        Some(trace) if trace["replayable"] == false => {
            // The starting state is a Git snapshot or a bounded copy of the
            // folder; a workspace that fits neither cannot be replayed.
            let why = trace["replay_reason"]
                .as_str()
                .filter(|reason| !reason.is_empty())
                .map(|reason| format!(" ({})", esc(reason)))
                .unwrap_or_default();
            out.push_str(&format!(
                "<span class=\"skillcache-compile\" data-status=\"unavailable\">This turn cannot be compiled: the workspace's starting state was not kept{why}, so the turn cannot be replayed.</span>"
            ));
        }
        Some(_) => out.push_str(&action_button(
            "Compile into a capsule",
            "skillcache-compile",
            &json!({"generation": generation}),
        )),
        None => out.push_str(
            "<span class=\"skillcache-compile\" data-status=\"unavailable\">The trace of this turn is no longer kept.</span>",
        ),
    }
    out
}

/// The run view's SkillCache tab: every SkillCache event of the turn in
/// order, with what can be done about it.
pub(crate) fn render_run_pane_html(events: &[V], state: &V, error: &str) -> String {
    if events.is_empty() {
        return String::new();
    }
    let mut out = String::from("<ol class=\"skillcache-phases\">");
    for event in events {
        let phase = event["phase"].as_str().unwrap_or("");
        let mut facts = Vec::new();
        if let Some(reason) = event["reason"].as_str().filter(|r| !r.is_empty()) {
            if phase == "miss" {
                facts.push(miss_reason(reason).to_string());
            } else if phase == "program_failed" {
                facts.push(format!("reason: {reason}"));
            }
        }
        if let Some(step) = event["step_index"].as_u64() {
            facts.push(format!("step {}", step + 1));
        }
        if let Some(program) = event["program"].as_str().filter(|p| !p.is_empty()) {
            facts.push(format!("program {program}"));
        }
        let actions = event_actions_html(event, state);
        out.push_str(&format!(
            "<li class=\"skillcache-phase\" data-phase=\"{}\" data-tone=\"{}\"><span class=\"skillcache-phase-name\">{}</span><span class=\"skillcache-phase-text\">{}</span>{}{}</li>",
            esc(phase),
            phase_tone(event),
            phase_label(phase),
            esc(&event_text(event)),
            if facts.is_empty() {
                String::new()
            } else {
                format!(
                    "<span class=\"skillcache-phase-facts\">{}</span>",
                    esc(&facts.join(" · "))
                )
            },
            if actions.is_empty() {
                String::new()
            } else {
                format!("<span class=\"skillcache-phase-actions\">{actions}</span>")
            },
        ));
    }
    out.push_str("</ol>");
    if !error.is_empty() {
        out.push_str(&format!(
            "<p class=\"error-text\" role=\"alert\">{}</p>",
            esc(error)
        ));
    }
    out
}

/// The body of a stream card: its label, its sentence and its actions.
pub(crate) fn stream_card_html(event: &V, state: &V) -> String {
    let actions = event_actions_html(event, state);
    format!(
        "<p><span class=\"skillcache-phase-name\">{}</span> {}</p>{}",
        phase_label(event["phase"].as_str().unwrap_or("")),
        esc(&event_text(event)),
        if actions.is_empty() {
            String::new()
        } else {
            format!("<div class=\"skillcache-phase-actions\">{actions}</div>")
        }
    )
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// What the Usage tab says about SkillCache in a session: how its turns were
/// decided, how many compiled steps went back to the agent, and what
/// compiling cost. That cost is the compiler's and is not part of any
/// launch above it, which is said in so many words. Empty when the session
/// never used SkillCache.
pub(crate) fn usage_summary(launches: &[V], state: &V) -> String {
    let mut hits = 0;
    let mut misses: Vec<(String, usize)> = Vec::new();
    for launch in launches {
        let decision = &launch["settings"]["skillcache"];
        match decision["decision"].as_str() {
            Some("hit") => hits += 1,
            Some("miss") => {
                let reason = miss_reason(decision["reason"].as_str().unwrap_or("")).to_string();
                match misses.iter_mut().find(|(known, _)| *known == reason) {
                    Some((_, count)) => *count += 1,
                    None => misses.push((reason, 1)),
                }
            }
            _ => {}
        }
    }
    let runs = m::array(&state["runs"]);
    let traces = m::array(&state["traces"]).len();
    let compiles = m::array(&state["compiles"]);
    let mut parts = Vec::new();
    if hits > 0 {
        parts.push(plural(hits, "warm turn", "warm turns"));
    }
    let cold = misses.iter().map(|(_, count)| count).sum::<usize>();
    if cold > 0 {
        parts.push(format!(
            "{} ({})",
            plural(cold, "cold turn", "cold turns"),
            misses
                .iter()
                .map(|(reason, count)| format!("{reason}: {count}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let handed_back = runs
        .iter()
        .filter_map(|run| run["deopts"].as_u64())
        .sum::<u64>() as usize;
    if !runs.is_empty() {
        parts.push(format!(
            "{} handed back to the agent",
            plural(handed_back, "compiled step", "compiled steps")
        ));
    }
    if traces > 0 {
        parts.push(format!("{} kept", plural(traces, "trace", "traces")));
    }
    if !compiles.is_empty() {
        let known = compiles
            .iter()
            .filter_map(|compile| compile["usage"]["cost_usd"].as_f64())
            .collect::<Vec<_>>();
        // A compilation without a reported price is counted as unpriced,
        // never as free.
        let cost = match (known.len(), compiles.len()) {
            (0, _) => "cost not reported".to_string(),
            (priced, all) if priced == all => format!("≈ ${:.4}", known.iter().sum::<f64>()),
            (priced, all) => format!(
                "≈ ${:.4} for {priced} of {all}; the rest not reported",
                known.iter().sum::<f64>()
            ),
        };
        parts.push(format!(
            "{}, {cost}, not part of the launches below",
            plural(compiles.len(), "compilation", "compilations")
        ));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("SkillCache: {}.", parts.join(" · "))
    }
}

/// Shows the Usage tab's SkillCache line for the selected session and reads
/// that session's SkillCache state when it has not been read.
pub fn render_usage(launches: &[V]) {
    m::set("skillcache-usage-launches", json!(launches));
    render_usage_line();
    let sid = m::read(|s| s.sid.clone());
    if !sid.is_empty() && !session_state().is_object() {
        spawn_local(async move {
            load_session(&sid, "").await;
        });
    }
}

fn render_usage_line() {
    let line = usage_summary(
        &m::array(&m::data("skillcache-usage-launches")),
        &session_state(),
    );
    dom::text("results-skillcache", &line);
    dom::hidden("results-skillcache", line.is_empty());
}

fn workflow_eligible() -> bool {
    eligible(&dom::value("workflow"), dom::checked("clash-enabled"))
}

/// The `skillcache` field of the turn the composer is about to send.
pub fn turn_request(follow: bool) -> Option<V> {
    let stored = m::read(|s| s.selected["skillcache"].clone());
    request(
        &dom::value("skillcache-mode"),
        &dom::value("skillcache-compiler-model"),
        workflow_eligible(),
        &stored,
        follow,
    )
}

/// The `skillcache` field of a preflight, when the next turn may run warm.
pub fn preflight_request() -> Option<V> {
    preview_request(
        &dom::value("skillcache-mode"),
        workflow_eligible(),
        &dom::value("provider"),
        &dom::value("mode"),
        &dom::value("permissions"),
    )
}

/// Whether the composer should ask for a preview of this prompt by itself.
pub fn wants_preview(prompt: &str) -> bool {
    preflight_request().is_some() && names_a_skill(prompt)
}

/// Shows or hides the pill for a preflight's answer.
pub fn render_preview(preview: &V) {
    match preview_pill(preview) {
        Some((text, tone)) => {
            dom::text("skillcache-preview", &text);
            dom::attr("skillcache-preview", "data-tone", tone);
            dom::hidden("skillcache-preview", false);
        }
        None => dom::hidden("skillcache-preview", true),
    }
}

/// The chip, the panel's note and whether the option can be changed now.
pub fn controls() {
    let allowed = workflow_eligible();
    let mode = dom::value("skillcache-mode");
    let shown = if allowed { mode.as_str() } else { "off" };
    dom::text("option-skillcache-value", mode_label(shown));
    dom::disabled("skillcache-mode", !allowed);
    dom::disabled("skillcache-compiler-model", !allowed || mode == "off");
    dom::text(
        "skillcache-note",
        &note(&mode, &dom::value("provider"), allowed),
    );
    if preflight_request().is_none() {
        dom::hidden("skillcache-preview", true);
    }
}

/// Puts a selected session's stored option into the composer.
pub fn apply_session(session: &V) {
    let stored = &session["skillcache"];
    dom::set_value("skillcache-mode", stored["mode"].as_str().unwrap_or("off"));
    dom::set_value(
        "skillcache-compiler-model",
        stored["compiler"]["model"].as_str().unwrap_or(""),
    );
}

/// What the settings snapshot compares to know whether the server's value
/// changed under an untouched control.
pub(crate) fn snapshot(session: &V) -> V {
    json!({
        "mode": session["skillcache"]["mode"].as_str().unwrap_or("off"),
        "model": session["skillcache"]["compiler"]["model"].as_str().unwrap_or(""),
    })
}

/// Renders the run view's tab for the current run.
pub fn render_run_pane(run: &V) {
    if let Some(pane) = dom::get("run-pane-skillcache") {
        set_markup(
            &pane,
            &render_run_pane_html(
                &m::array(&run["skillcache"]),
                &session_state(),
                m::data("skillcache-error").as_str().unwrap_or(""),
            ),
        );
    }
}

/// The selected session's SkillCache state, when it is that session's.
pub(crate) fn session_state() -> V {
    let state = m::data("skillcache-session");
    let sid = m::read(|s| s.sid.clone());
    if !sid.is_empty() && state["session_id"] == sid.as_str() {
        state
    } else {
        V::Null
    }
}

/// Whether the state named by `key` has to be asked for: not while it is
/// being asked for, and not while it is held. One state is kept, and the
/// Usage tab or a capsule's change may have put another session's there, so
/// a key that was read counts only while `held`, the state of the selected
/// session, is still there.
pub(crate) fn needs_state(key: &str, read_key: &V, loading: &V, held: &V) -> bool {
    let read = read_key == key && held.is_object();
    !(read || loading == key)
}

/// Loads the session's traces and compilations when the run shows SkillCache
/// events the loaded state has not seen. The key holds the session and the
/// number of events. It is recorded as read only once the state has arrived:
/// a request that was cut short by a change of session is asked again.
pub fn sync_session(run: &V) {
    let sid = m::read(|s| s.sid.clone());
    let events = m::array(&run["skillcache"]).len();
    if sid.is_empty() || events == 0 {
        return;
    }
    let key = format!("{sid}:{events}");
    if !needs_state(
        &key,
        &m::data("skillcache-session-key"),
        &m::data("skillcache-session-loading"),
        &session_state(),
    ) {
        return;
    }
    m::set("skillcache-session-loading", json!(key));
    spawn_local(async move {
        load_session(&sid, &key).await;
    });
}

/// Forgets what was loaded, so that the next render asks again.
pub fn invalidate_session() {
    m::take("skillcache-session-key");
}

async fn load_session(sid: &str, key: &str) {
    let path = format!("/api/sessions/{}/skillcache", dom::enc(sid));
    let answer = api::scoped("skillcache-session", &path).await;
    if m::data("skillcache-session-loading") == key {
        m::take("skillcache-session-loading");
    }
    match answer {
        Ok((epoch, state))
            if m::current("skillcache-session", epoch) && m::read(|s| s.sid == sid) =>
        {
            m::set("skillcache-session", state);
            if !key.is_empty() {
                m::set("skillcache-session-key", json!(key));
            }
            run::render();
            run::refresh_skillcache_cards();
            render_usage_line();
        }
        _ => {}
    }
}

/// A capsule changed somewhere in this project: what is shown may be stale.
pub fn on_sse_capsule(event: &V) {
    if m::text(event, "project_id") != m::current_project() {
        return;
    }
    invalidate_session();
    let sid = m::read(|s| s.sid.clone());
    let view = m::read(|s| s.view.clone());
    let capsule = m::text(event, "capsule_id");
    spawn_local(async move {
        if !sid.is_empty() && !m::array(&m::data("run")["skillcache"]).is_empty() {
            load_session(&sid, "").await;
        }
        if view == "skill-capsules" {
            crate::management::capsule_changed(&capsule).await;
        }
    });
}

pub async fn event(kind: &str, id: &str, action: &str, payload: &V) -> bool {
    if kind != "click" {
        return false;
    }
    if id == "skillcache-open-capsules" {
        session::open_view("skill-capsules", true).await;
        return true;
    }
    match action {
        "skillcache-open-capsule" => {
            m::set("skill-capsule-open", payload.clone());
            session::open_view("skill-capsules", true).await;
        }
        "skillcache-compile" => {
            let sid = m::read(|s| s.sid.clone());
            let generation = m::text(payload, "generation");
            if sid.is_empty() || generation.is_empty() {
                return true;
            }
            let model = dom::value("skillcache-compiler-model");
            let mut body = json!({"generation": generation});
            if !model.trim().is_empty() {
                body["compiler"] = json!({"provider": "claude", "model": model.trim()});
            }
            m::take("skillcache-error");
            let path = format!("/api/sessions/{}/skillcache/compile", dom::enc(&sid));
            if let Err(error) = api::post(&path, body).await {
                m::set("skillcache-error", json!(error.message));
            }
            invalidate_session();
            load_session(&sid, "").await;
            run::render();
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{assert_no_inline_hex_colors, payloads};

    #[test]
    fn the_option_is_sent_only_when_it_says_something_new() {
        let none = V::Null;
        // A new session is off by saying nothing.
        assert_eq!(request("off", "", true, &none, false), None);
        assert_eq!(
            request("observe", "", true, &none, false),
            Some(json!({"mode": "observe", "compiler": {"provider": "claude", "model": null}}))
        );
        assert_eq!(
            request("auto", " opus ", true, &none, false).unwrap()["compiler"]["model"],
            "opus"
        );
        // A specialized workflow never carries it, and a session that had it
        // on gives it up when its next turn is not an ordinary one.
        assert_eq!(request("auto", "", false, &none, false), None);
        assert_eq!(request("auto", "", false, &none, true), None);
        assert_eq!(
            request("auto", "", false, &json!({"mode": "auto"}), true),
            Some(json!({"mode": "off"}))
        );
        // A follow-up repeats nothing, and turning it off is said in words.
        let stored = json!({"mode": "auto", "compiler": {"provider": "claude", "model": null}});
        assert_eq!(request("auto", "", true, &stored, true), None);
        assert_eq!(
            request("warm_only", "", true, &stored, true).unwrap()["mode"],
            "warm_only"
        );
        assert_eq!(
            request("auto", "opus", true, &stored, true).unwrap()["compiler"]["model"],
            "opus"
        );
        assert_eq!(
            request("off", "", true, &stored, true),
            Some(json!({"mode": "off"}))
        );
        assert_eq!(request("off", "", true, &none, true), None);
        assert_eq!(request("nonsense", "", true, &none, false), None);
    }

    #[test]
    fn the_chip_and_the_note_name_what_is_kept_and_what_runs() {
        assert_eq!(mode_label("warm_only"), "Warm only");
        assert_eq!(mode_label("nonsense"), "Off");
        assert!(eligible("native", false));
        assert!(!eligible("native", true) && !eligible("fleet-review", false));
        assert!(note("off", "claude", true).starts_with("Off."));
        assert!(note("observe", "claude", true).contains("redacted trace"));
        assert!(note("observe", "claude", true).contains("30 days"));
        assert!(note("warm_only", "codex", true).contains("Nothing is recorded"));
        assert!(note("auto", "codex", true).contains("$skill"));
        // Claude runs a capsule for a `$skill` mention; its `/command` stays
        // native and is only observed. Cursor never runs a capsule.
        let claude = note("auto", "claude", true);
        assert!(claude.contains("$skill") && claude.contains("`/command` runs natively"));
        assert!(!claude.contains("never run a capsule"));
        assert!(note("warm_only", "cursor", true).contains("never run a capsule"));
        assert_eq!(miss_reason("native_route"), "native command");
        assert!(note("auto", "codex", false).contains("ordinary sessions"));
    }

    #[test]
    fn a_preview_is_asked_only_for_a_mode_that_can_run_a_capsule() {
        assert!(preview_request("observe", true, "codex", "edit", "auto").is_none());
        assert!(preview_request("auto", false, "codex", "edit", "auto").is_none());
        assert_eq!(
            preview_request("warm_only", true, "codex", "edit", "auto"),
            Some(
                json!({"mode": "warm_only", "provider": "codex", "session_mode": "edit", "permissions": "auto"})
            )
        );
        assert!(names_a_skill("please $pdf the invoices"));
        assert!(names_a_skill("/review this"));
        assert!(!names_a_skill("costs $5 and a/b"));
        assert!(!names_a_skill("just words"));
    }

    #[test]
    fn the_pill_says_warm_miss_or_stale() {
        let hit = json!({"decision": "hit", "capsule": {"mu": "pdf-totals", "form": "hybrid"}});
        assert_eq!(preview_pill(&hit), Some(("Warm: pdf-totals".into(), "ok")));
        let textual =
            json!({"decision": "hit", "capsule": {"mu": "pdf-totals", "form": "textual"}});
        assert_eq!(
            preview_pill(&textual).unwrap().0,
            "Warm: pdf-totals (written procedure)"
        );
        assert_eq!(
            preview_pill(&json!({"decision": "miss", "reason": "stale"})),
            Some(("Stale: cold".into(), "warn"))
        );
        assert_eq!(
            preview_pill(&json!({"decision": "miss", "reason": "no_capsule"})),
            Some(("Miss: cold (no qualified capsule)".into(), "muted"))
        );
        // A native command says what would have run the capsule.
        assert!(
            preview_pill(&json!({"decision": "miss", "reason": "native_route"}))
                .unwrap()
                .0
                .contains("use $skill")
        );
        // An ordinary message is not a miss worth a pill.
        assert_eq!(
            preview_pill(&json!({"decision": "miss", "reason": "no_single_skill"})),
            None
        );
        assert_eq!(preview_pill(&json!({"decision": "off"})), None);
        assert_eq!(preview_pill(&V::Null), None);
    }

    fn observed() -> V {
        json!({"kind": "skillcache", "phase": "observed", "generation": "g1",
               "text": "SkillCache kept this turn's trace (2 tool calls; skill: pdf). It can be compiled into a capsule."})
    }

    #[test]
    fn an_observed_turn_is_compiled_once() {
        // Before the session's state is known nothing is offered.
        assert_eq!(event_actions_html(&observed(), &V::Null), "");
        let state = json!({"traces": [{"generation": "g1", "replayable": true}], "compiles": []});
        let html = event_actions_html(&observed(), &state);
        assert_eq!(
            payloads(&html, "skillcache-compile"),
            [json!({"generation": "g1"})]
        );
        // A compilation takes the button's place.
        let compiling = json!({
            "traces": [{"generation": "g1", "replayable": true}],
            "compiles": [{"source_generation": "g1", "status": "completed", "capsule_id": "c1",
                          "outcome": "Qualified as a hybrid capsule in round 1."}]
        });
        let html = event_actions_html(&observed(), &compiling);
        assert!(payloads(&html, "skillcache-compile").is_empty());
        assert!(html.contains("Compilation completed: Qualified as a hybrid capsule"));
        assert_eq!(payloads(&html, "skillcache-open-capsule"), [json!("c1")]);
        // No snapshot of the turn's start, no replay, no compilation.
        let unreplayable =
            json!({"traces": [{"generation": "g1", "replayable": false}], "compiles": []});
        let html = event_actions_html(&observed(), &unreplayable);
        assert!(payloads(&html, "skillcache-compile").is_empty());
        assert!(html.contains("starting state was not kept"));
        assert!(
            !html.contains("Git repository"),
            "a folder copy replays too"
        );
        // The server's reason is shown, escaped.
        let refused = json!({"traces": [{"generation": "g1", "replayable": false,
                                         "replay_reason": "larger than <limit>"}], "compiles": []});
        let html = event_actions_html(&observed(), &refused);
        assert!(html.contains("(larger than &lt;limit&gt;)"), "{html}");
        assert_no_inline_hex_colors(&html);
        let gone = json!({"traces": [], "compiles": []});
        assert!(event_actions_html(&observed(), &gone).contains("no longer kept"));
    }

    #[test]
    fn the_run_tab_lists_every_phase_and_escapes_what_it_shows() {
        let events = [
            json!({"kind": "skillcache", "phase": "hit", "capsule_id": "c1", "mu": "pdf-totals",
                   "text": "SkillCache: capsule `pdf-totals` runs this request instead of the skill being read again."}),
            json!({"kind": "skillcache", "phase": "program_failed", "capsule_id": "c1", "step_index": 0, "program": "<extract>",
                   "reason": "nonzero_exit", "text": "SkillCache: program `<extract>` failed (nonzero_exit); step 1 is the agent's now, the rest of the plan stays compiled."}),
            json!({"kind": "skillcache", "phase": "complete", "capsule_id": "c1", "text": "SkillCache: the plan is complete (1 failed)."}),
        ];
        let html = render_run_pane_html(&events, &V::Null, "");
        assert_no_inline_hex_colors(&html);
        assert_eq!(html.matches("<li class=\"skillcache-phase\"").count(), 3);
        assert!(html.contains("data-phase=\"hit\" data-tone=\"ok\""));
        assert!(html.contains("data-phase=\"program_failed\" data-tone=\"warn\""));
        assert!(html.contains("Step handed back"));
        assert!(html.contains("reason: nonzero_exit · step 1 · program &lt;extract&gt;"));
        assert!(!html.contains("<extract>"), "{html}");
        // The sentence loses the product's name, not its meaning.
        assert!(html.contains(">Capsule `pdf-totals` runs this request"));
        assert!(html.contains(">The plan is complete (1 failed).<"));
        // The capsule is offered where it enters the turn, once.
        assert_eq!(payloads(&html, "skillcache-open-capsule"), [json!("c1")]);
        // A cold run has no tab content, and an error is shown with it.
        assert_eq!(render_run_pane_html(&[], &V::Null, "boom"), "");
        let failed = render_run_pane_html(&events[..1], &V::Null, "The queue is <full>");
        assert!(failed.contains("role=\"alert\">The queue is &lt;full&gt;</p>"));
    }

    #[test]
    fn the_usage_line_counts_decisions_and_keeps_the_compile_cost_apart() {
        let launch = |decision: V| json!({"settings": {"skillcache": decision}});
        let launches = [
            launch(json!({"decision": "hit", "capsule_id": "c1"})),
            launch(json!({"decision": "miss", "reason": "no_capsule"})),
            launch(json!({"decision": "miss", "reason": "no_capsule"})),
            launch(json!({"decision": "miss", "reason": "stale"})),
            json!({"settings": {"permissions": "auto"}}),
        ];
        let state = json!({
            "runs": [{"deopts": 1}, {"deopts": 0}],
            "traces": [{}, {}, {}],
            "compiles": [{"usage": {"cost_usd": 0.05}}, {"usage": {"input_tokens": 10}}],
        });
        assert_eq!(
            usage_summary(&launches, &state),
            "SkillCache: 1 warm turn · 3 cold turns (no qualified capsule: 2, the capsule is stale: 1) · 1 compiled step handed back to the agent · 3 traces kept · 2 compilations, ≈ $0.0500 for 1 of 2; the rest not reported, not part of the launches below."
        );
        // Before the session's state is read, only what the launches say.
        assert_eq!(
            usage_summary(&launches[..1], &V::Null),
            "SkillCache: 1 warm turn."
        );
        // A compilation with no reported price is not a free one.
        assert_eq!(
            usage_summary(&[], &json!({"compiles": [{"usage": null}]})),
            "SkillCache: 1 compilation, cost not reported, not part of the launches below."
        );
        assert_eq!(
            usage_summary(
                &[],
                &json!({"compiles": [{"usage": {"cost_usd": 0.05}}, {"usage": {"cost_usd": 0.01}}]})
            ),
            "SkillCache: 2 compilations, ≈ $0.0600, not part of the launches below."
        );
        // A session that never used SkillCache has no line.
        assert_eq!(
            usage_summary(
                &launches[4..],
                &json!({"runs": [], "traces": [], "compiles": []})
            ),
            ""
        );
    }

    #[test]
    fn the_session_state_is_asked_again_when_another_session_s_took_its_place() {
        let key = "a:5";
        let held = json!({"session_id": "a"});
        // Never read; being read; read and still held.
        assert!(needs_state(key, &V::Null, &V::Null, &V::Null));
        assert!(!needs_state(key, &V::Null, &json!(key), &V::Null));
        assert!(!needs_state(key, &json!(key), &V::Null, &held));
        // Read once, but the Usage tab of another session replaced the one
        // state that is kept: without asking again, this session's cards
        // would never learn what was compiled.
        assert!(needs_state(key, &json!(key), &V::Null, &V::Null));
        // More events than were read.
        assert!(needs_state("a:6", &json!(key), &V::Null, &held));
    }

    #[test]
    fn unchanged_markup_has_the_same_fingerprint() {
        // What decides whether an element is replaced: the same markup must
        // never look new, and different markup must never look the same.
        let pane = render_run_pane_html(&[observed()], &V::Null, "");
        assert_eq!(signature(&pane), signature(&pane.clone()));
        let with_state = render_run_pane_html(
            &[observed()],
            &json!({"traces": [{"generation": "g1", "replayable": true}], "compiles": []}),
            "",
        );
        assert_ne!(signature(&pane), signature(&with_state));
        assert_ne!(signature("ab"), signature("ba"));
        assert_ne!(signature(""), signature(" "));
    }

    #[test]
    fn the_stream_shows_what_a_person_may_act_on() {
        let phase = |phase: &str| json!({"kind": "skillcache", "phase": phase});
        for shown in [
            "hit",
            "observed",
            "needs_bindings",
            "your_steps",
            "program_failed",
            "complete",
            "needs_recompile",
        ] {
            assert!(in_stream(&phase(shown)), "{shown}");
        }
        for quiet in [
            "started",
            "program_completed",
            "interrupted",
            "not_observed",
        ] {
            assert!(!in_stream(&phase(quiet)), "{quiet}");
        }
        let miss = |reason: &str| json!({"kind": "skillcache", "phase": "miss", "reason": reason});
        // Every ordinary turn of an `auto` session is a miss; that is not news.
        assert!(!in_stream(&miss("no_single_skill")) && !in_stream(&miss("no_capsule")));
        assert!(in_stream(&miss("stale")) && in_stream(&miss("network")));
        assert_eq!(phase_tone(&miss("stale")), "warn");
        assert_eq!(phase_tone(&miss("no_capsule")), "muted");
        let card = stream_card_html(&observed(), &V::Null);
        assert_no_inline_hex_colors(&card);
        assert!(card.contains("Trace kept"));
        assert!(card.contains("Kept this turn&#39;s trace"));
    }
}
