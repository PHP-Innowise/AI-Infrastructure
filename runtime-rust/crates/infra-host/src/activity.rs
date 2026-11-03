//! Display-only provider targets. File bodies, diffs, tool output and MCP arguments never pass this boundary.
use serde_json::{Value as V, json};
pub const TEXT_LIMIT: usize = 2000;
pub const THINKING_LIMIT: usize = 1200;
pub const DETAIL_LIMIT: usize = 300;
fn text(v: &V) -> &str {
    v.as_str().unwrap_or("")
}
pub fn bounded(value: &V, limit: usize, single: bool) -> Option<String> {
    let value = value.as_str()?;
    let value = if single {
        value.split_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        value.trim().into()
    };
    if value.is_empty() {
        None
    } else if value.chars().count() <= limit {
        Some(value)
    } else {
        Some(format!(
            "{}…",
            value
                .chars()
                .take(limit.saturating_sub(1))
                .collect::<String>()
        ))
    }
}
pub fn call_token(value: &V) -> Option<String> {
    let value = value.as_str().filter(|s| !s.trim().is_empty())?;
    if value.len() <= 128
        && value
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
    {
        Some(value.into())
    } else {
        Some(format!("h{}", &crate::store::sha(value.as_bytes())[..24]))
    }
}
fn safe_call(value: &V) -> bool {
    value.as_str().is_some_and(|s| {
        !s.is_empty()
            && s.len() <= 128
            && s.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
    })
}
fn url_location(value: &str) -> Option<String> {
    let (scheme, tail) = value.split_once("://")?;
    if !["http", "https"].contains(&scheme) {
        return None;
    }
    let end = tail.find(['/', '?', '#']).unwrap_or(tail.len());
    let authority = tail[..end].rsplit('@').next()?;
    let host = if authority.starts_with('[') {
        authority.split_once(']')?.0.strip_prefix('[')?
    } else {
        authority.split(':').next()?
    };
    if host.is_empty() {
        return None;
    }
    let rest = &tail[end..];
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    Some(format!("{scheme}://{}{rest}", host.to_lowercase()))
}
pub fn command_head(value: &str) -> Option<String> {
    let mut text = value.trim().replace("\\\r\n", " ").replace("\\\n", " ");
    let mut cut = false;
    let bytes = text.as_bytes();
    let mut heredoc_end = None;
    let mut at = 0;
    while at + 1 < bytes.len() {
        if bytes[at] == b'<'
            && bytes[at + 1] == b'<'
            && (at == 0
                || !(bytes[at - 1].is_ascii_alphanumeric()
                    || bytes[at - 1] == b'_'
                    || bytes[at - 1] == b'<'))
            && bytes.get(at + 2) != Some(&b'<')
        {
            let mut pos = at + 2;
            if bytes.get(pos) == Some(&b'-') {
                pos += 1;
            }
            while bytes.get(pos).is_some_and(|b| *b == b' ' || *b == b'\t') {
                pos += 1;
            }
            let quote = bytes
                .get(pos)
                .copied()
                .filter(|b| *b == b'\'' || *b == b'"');
            if quote.is_some() {
                pos += 1;
            }
            if bytes
                .get(pos)
                .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
            {
                pos += 1;
                while bytes
                    .get(pos)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || b"_.-".contains(b))
                {
                    pos += 1;
                }
                if let Some(q) = quote {
                    if bytes.get(pos) != Some(&q) {
                        at += 1;
                        continue;
                    }
                    pos += 1;
                }
                heredoc_end = Some(pos);
                break;
            }
        }
        at += 1;
    }
    if let Some(end) = heredoc_end {
        text.truncate(end);
        cut = true;
    }
    if let Some(end) = text.find(['\n', '\r']) {
        text.truncate(end);
        cut = true;
    }
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(format!("{text}{}", if cut { " …" } else { "" }))
    }
}
fn mcp_name(tool: &str) -> Option<String> {
    if !tool.to_lowercase().starts_with("mcp__") {
        return None;
    }
    let name = tool
        .split("__")
        .skip(1)
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(".");
    (!name.is_empty()).then_some(name)
}
pub fn activity_target(arguments: &V, tool: &str) -> (Option<String>, Option<String>) {
    if !arguments.is_object() {
        return (None, None);
    }
    if let Some(name) = mcp_name(tool) {
        return (None, bounded(&json!(name), DETAIL_LIMIT, true));
    }
    let path = [
        "file_path",
        "notebook_path",
        "path",
        "filePath",
        "targetFile",
        "target_file",
        "targetDirectory",
        "directory",
    ]
    .iter()
    .find_map(|k| bounded(&arguments[*k], 1024, true));
    let lower = tool.to_lowercase();
    let keys: &[&str] = match lower.as_str() {
        "bash" | "shell" | "powershell" | "run_terminal_cmd" | "run_command" | "terminal" => {
            &["command", "cmd"]
        }
        "grep" | "glob" | "ls" | "search" | "grep_search" | "file_search" | "glob_file_search"
        | "codebase_search" | "semsearch" | "semantic_search" => {
            &["pattern", "globPattern", "glob_pattern", "glob", "query"]
        }
        "websearch" | "web_search" | "web search" => &["query", "searchTerm", "search_term"],
        "webfetch" | "web_fetch" | "fetch" => &["url"],
        "task" | "agent" => &["description"],
        "skill" => &["skill"],
        _ => &[],
    };
    let mut detail = None;
    for key in keys {
        if let Some(value) = arguments[*key].as_str().filter(|s| !s.trim().is_empty()) {
            detail = if *key == "url" {
                url_location(value)
            } else if ["command", "cmd"].contains(key) {
                command_head(value)
            } else if [
                "pattern",
                "globPattern",
                "glob_pattern",
                "glob",
                "query",
                "searchTerm",
                "search_term",
            ]
            .contains(key)
            {
                Some(format!("\"{value}\""))
            } else {
                Some(value.into())
            };
            break;
        }
    }
    (
        path,
        detail.and_then(|v| bounded(&json!(v), DETAIL_LIMIT, true)),
    )
}
fn todo_status(entry: &V) -> String {
    let status = text(&entry["status"])
        .to_lowercase()
        .replace("todo_status_", "");
    match status.as_str() {
        "completed" => "done",
        "in_progress" => "active",
        "cancelled" => "cancelled",
        _ => "pending",
    }
    .into()
}
pub fn plan_items(
    entries: &V,
    key: &str,
    codex: bool,
    form: Option<&str>,
    merge: bool,
) -> (Vec<V>, usize, Option<String>) {
    let valid = entries
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| {
            e.is_object()
                && (e[key].as_str().is_some_and(|s| !s.trim().is_empty())
                    || merge && safe_call(&e["id"]))
        })
        .collect::<Vec<_>>();
    let mut items = vec![];
    let mut active = None;
    for entry in valid.iter().take(30) {
        let mut item = json!({});
        if let Some(value) = bounded(&entry[key], 200, true) {
            item["text"] = json!(value);
        }
        if !merge || entry.get("status").is_some() {
            item["status"] = json!(if codex {
                if entry["completed"] == true {
                    "done".into()
                } else {
                    "pending".into()
                }
            } else {
                todo_status(entry)
            });
        }
        if safe_call(&entry["id"]) {
            item["id"] = entry["id"].clone();
        }
        if let Some(form) = form
            && active.is_none()
            && item["status"] == "active"
        {
            active = bounded(&entry[form], 200, true);
        }
        items.push(item);
    }
    (items, valid.len(), active)
}
pub fn plan_event(entries: &V, key: &str, codex: bool, form: Option<&str>, merge: bool) -> Vec<V> {
    let (items, total, active) = plan_items(entries, key, codex, form, merge);
    if items.is_empty() {
        return vec![];
    }
    let mut event = json!({"kind":"plan","items":items,"total":total});
    if let Some(active) = active {
        event["active_form"] = json!(active);
    }
    if merge {
        event["merge"] = json!(true);
    }
    vec![event]
}
fn activity_plan(entries: &V, key: &str, codex: bool) -> Vec<V> {
    let (items, _, _) = plan_items(entries, key, codex, None, false);
    if items.is_empty() {
        return vec![];
    }
    let lines = items
        .iter()
        .map(|i| {
            format!(
                "{} {}",
                match text(&i["status"]) {
                    "done" => "✓",
                    "active" => "→",
                    _ => "○",
                },
                text(&i["text"])
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    vec![json!({"type":"plan","text":lines,"items":items})]
}
pub fn tool(
    name: &str,
    call: &V,
    state: &str,
    ok: bool,
    arguments: Option<&V>,
    path: Option<String>,
    detail: Option<String>,
) -> V {
    let (path, detail) = arguments
        .map(|a| activity_target(a, name))
        .unwrap_or((path, detail));
    let mut item = json!({"type":"tool","tool":bounded(&json!(name),120,true).unwrap_or_else(||"Tool".into()),"state":state,"ok":ok});
    for (key, value) in [
        ("call", call_token(call)),
        ("path", path),
        ("detail", detail),
    ] {
        if let Some(value) = value.filter(|s| !s.is_empty()) {
            item[key] = json!(value);
        }
    }
    item
}
pub fn targets(
    name: &str,
    call: &V,
    state: &str,
    ok: bool,
    arguments: Option<&V>,
    path: Option<String>,
    detail: Option<String>,
) -> V {
    let mut event = tool(name, call, state, ok, arguments, path, detail);
    event.as_object_mut().expect("tool object").remove("type");
    event
}
fn codex_command(item: &V) -> Option<String> {
    let command = &item["command"];
    let mut command = if let Some(parts) = command.as_array() {
        let parts = parts.iter().filter_map(V::as_str).collect::<Vec<_>>();
        if parts.len() == 3 && ["-lc", "-c"].contains(&parts[1]) && shell_executable(parts[0]) {
            parts[2].into()
        } else {
            parts.join(" ")
        }
    } else {
        command.as_str()?.into()
    };
    let parts = command.trim().splitn(3, ' ').collect::<Vec<_>>();
    if parts.len() == 3 && parts[1] == "-lc" && shell_executable(parts[0]) {
        let body = parts[2];
        if body.len() >= 2
            && ((body.starts_with('\'') && body.ends_with('\''))
                || (body.starts_with('"') && body.ends_with('"')))
        {
            command = body[1..body.len() - 1].into();
        }
    }
    command_head(&command).and_then(|s| bounded(&json!(s), DETAIL_LIMIT, true))
}
fn shell_executable(name: &str) -> bool {
    ["sh", "bash", "zsh"].contains(&name.replace('\\', "/").rsplit('/').next().unwrap_or(""))
}
fn codex_changes(item: &V) -> Vec<V> {
    item["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c.is_object() && c["path"].is_string())
        .take(20)
        .cloned()
        .collect()
}
pub fn codex_targets(item: &V, state: &str) -> V {
    let kind = text(&item["type"]);
    let ok = !["failed", "declined"].contains(&text(&item["status"]));
    let mut out = match kind {
        "command_execution" => targets(
            "Shell",
            &item["id"],
            state,
            ok && (item["exit_code"].is_null() || item["exit_code"] == 0),
            None,
            None,
            codex_command(item),
        ),
        "file_change" => {
            let mut out = targets("Edit", &item["id"], state, ok, None, None, None);
            out["changes"] = json!(
                codex_changes(item)
                    .into_iter()
                    .map(|c| json!({"path":c["path"],"kind":c["kind"]}))
                    .collect::<Vec<_>>()
            );
            out
        }
        "mcp_tool_call" => targets(
            "MCP",
            &item["id"],
            state,
            ok,
            None,
            None,
            Some(
                [text(&item["server"]), text(&item["tool"])]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("."),
            ),
        ),
        _ => targets(
            "WebSearch",
            &item["id"],
            state,
            ok,
            None,
            None,
            bounded(&item["query"], DETAIL_LIMIT, true),
        ),
    };
    if item["status"] == "declined" {
        out["outcome"] = json!("not_run");
    }
    if let Some(code) = item["exit_code"].as_i64() {
        out["exit_code"] = json!(code);
    }
    out
}
pub fn cursor_call(name: &str, call: &V) -> (String, V) {
    let mut name = name.to_owned();
    let mut args = call["args"].clone();
    if name == "function" {
        name = call["name"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("function")
            .into();
        args = if let Some(s) = call["arguments"].as_str() {
            crate::strict_json::parse(s.as_bytes(), 2 * 1024 * 1024).unwrap_or(V::Null)
        } else {
            call["arguments"].clone()
        };
    } else if let Some(base) = name.strip_suffix("ToolCall") {
        name = base.into();
    }
    (name, args)
}
pub fn cursor_targets(name: &str, args: &V, call: &V, state: &str, outcome: &V) -> V {
    let ok = !outcome.is_object()
        || outcome
            .as_object()
            .is_some_and(|o| o.is_empty() || o.contains_key("success"));
    let mut out = if name == "mcp" && args.is_object() {
        targets(
            "MCP",
            call,
            state,
            ok,
            None,
            None,
            Some(
                [text(&args["providerIdentifier"]), text(&args["toolName"])]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("."),
            ),
        )
    } else {
        targets(&capitalized(name), call, state, ok, Some(args), None, None)
    };
    if outcome.get("rejected").is_some() {
        out["outcome"] = json!("not_run");
    }
    out
}
fn capitalized(name: &str) -> String {
    let mut chars = name.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}
fn error(event: &V, fallback: &str) -> String {
    let error = if event["error"].is_object() {
        event["error"]["message"].as_str()
    } else {
        event["error"].as_str()
    };
    if let Some(value) = error.filter(|s| !s.is_empty()) {
        return value.into();
    }
    let errors = event["errors"]
        .as_array()
        .map(|v| {
            v.iter()
                .filter_map(V::as_str)
                .collect::<Vec<_>>()
                .join("; ")
        })
        .unwrap_or_default();
    if !errors.is_empty() {
        return errors;
    }
    event["message"]
        .as_str()
        .or_else(|| event["result"].as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback)
        .into()
}
pub fn usage(event: &V) -> Vec<V> {
    let mut out = json!({"kind":"usage"});
    for key in [
        "input_tokens",
        "output_tokens",
        "cached_input_tokens",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
    ] {
        if let Some(n) = event["usage"][key].as_u64() {
            out[key] = json!(n);
        }
    }
    if let Some(n) = event["total_cost_usd"]
        .as_f64()
        .filter(|n| n.is_finite() && *n >= 0.)
    {
        out["cost_usd"] = json!(n);
    }
    if out.as_object().map(|o| o.len() > 1).unwrap_or(false) {
        vec![out]
    } else {
        vec![]
    }
}
pub fn total_tokens(usage: &V) -> Option<u64> {
    let mut total = usage["input_tokens"]
        .as_u64()?
        .checked_add(usage["output_tokens"].as_u64()?)?;
    for key in ["cache_read_input_tokens", "cache_creation_input_tokens"] {
        if let Some(n) = usage[key].as_u64() {
            total = total.checked_add(n)?;
        }
    }
    Some(total)
}
pub fn activity_events(provider: &str, event: &V) -> Vec<V> {
    let kind = text(&event["type"]);
    if kind.is_empty() {
        return vec![];
    }
    if provider == "codex" {
        return codex_activity(kind, event);
    }
    if kind == "system" && event["subtype"] == "init" {
        return vec![
            json!({"type":"status","text":format!("Session started{}",bounded(&event["model"],120,true).map(|m|format!(" · model {m}")).unwrap_or_default())}),
        ];
    }
    if kind == "error" {
        return vec![json!({"type":"error","text":error(event,"Provider error")})];
    }
    if kind == "result" {
        let mut out = usage(event);
        for item in &mut out {
            item["type"] = json!("usage");
            item.as_object_mut().expect("usage").remove("kind");
        }
        if event["subtype"] != "success" || event["is_error"] != false {
            out.push(json!({"type":"error","text":error(event,"Provider run failed")}));
        }
        return out;
    }
    if kind == "assistant" {
        if !event["error"].is_null() && event["error"] != false {
            return vec![json!({"type":"error","text":error(event,"Assistant request failed")})];
        }
        if provider == "cursor"
            && event.get("timestamp_ms").is_some()
            && event.get("model_call_id").is_none()
        {
            return vec![];
        }
        let mut out = vec![];
        for block in event["message"]["content"].as_array().into_iter().flatten() {
            match text(&block["type"]) {
                "text" | "thinking" => {
                    let thinking = block["type"] == "thinking";
                    if let Some(value) = bounded(
                        &block[if thinking { "thinking" } else { "text" }],
                        if thinking { THINKING_LIMIT } else { TEXT_LIMIT },
                        false,
                    ) {
                        out.push(json!({"type":if thinking{"thinking"}else{"text"},"text":value}));
                    }
                }
                "tool_use" => {
                    let name = text(&block["name"]);
                    if name == "TodoWrite" && block["input"].is_object() {
                        out.extend(activity_plan(&block["input"]["todos"], "content", false));
                    } else {
                        out.push(tool(
                            name,
                            &block["id"],
                            "started",
                            true,
                            if name == "StructuredOutput" {
                                None
                            } else {
                                Some(&block["input"])
                            },
                            None,
                            if name == "StructuredOutput" {
                                Some("Final structured report".into())
                            } else {
                                None
                            },
                        ));
                    }
                }
                _ => {}
            }
        }
        return out;
    }
    if kind == "user" && provider == "claude" {
        return event["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["type"] == "tool_result")
            .map(|b| {
                tool(
                    "Tool",
                    &b["tool_use_id"],
                    "completed",
                    b["is_error"] != true,
                    None,
                    None,
                    None,
                )
            })
            .collect();
    }
    if kind == "tool_call" && provider == "cursor" {
        let mut out = vec![];
        let state = if event["subtype"] == "completed" {
            "completed"
        } else {
            "started"
        };
        for (name, call) in event["tool_call"].as_object().into_iter().flatten() {
            if !call.is_object() {
                continue;
            }
            let (name, args) = cursor_call(name, call);
            if ["updateTodos", "todo"].contains(&name.as_str()) && args.is_object() {
                out.extend(activity_plan(&args["todos"], "content", false));
                continue;
            }
            let outcome = &call["result"];
            let ok = !outcome.is_object()
                || outcome
                    .as_object()
                    .is_some_and(|o| o.is_empty() || o.contains_key("success"));
            if name == "mcp" && args.is_object() {
                let label = [text(&args["providerIdentifier"]), text(&args["toolName"])]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(".");
                out.push(tool(
                    &format!("MCP {}", if label.is_empty() { "tool" } else { &label }),
                    &event["call_id"],
                    state,
                    ok,
                    None,
                    None,
                    None,
                ));
            } else {
                out.push(tool(
                    &capitalized(&name),
                    &event["call_id"],
                    state,
                    ok,
                    Some(&args),
                    None,
                    None,
                ));
            }
        }
        return out;
    }
    if kind == "thinking" && provider == "cursor" {
        if event["subtype"] == "delta" {
            if let Some(value) = event["text"].as_str().filter(|s| !s.is_empty()) {
                return vec![
                    json!({"type":"thinking_delta","text":value.chars().take(THINKING_LIMIT).collect::<String>()}),
                ];
            }
        } else if event["subtype"] == "completed" {
            return vec![json!({"type":"thinking_end"})];
        }
    }
    vec![]
}
fn codex_activity(kind: &str, event: &V) -> Vec<V> {
    if ["error", "turn.failed"].contains(&kind) {
        return vec![json!({"type":"error","text":error(event,"Provider run failed")})];
    }
    if kind == "turn.completed" {
        return usage(event)
            .into_iter()
            .map(|mut v| {
                v["type"] = json!("usage");
                v.as_object_mut().expect("usage").remove("kind");
                v
            })
            .collect();
    }
    let item = &event["item"];
    if !["item.started", "item.updated", "item.completed"].contains(&kind) || !item.is_object() {
        return vec![];
    }
    let t = text(&item["type"]);
    let finished = kind == "item.completed";
    if finished && ["agent_message", "reasoning", "error"].contains(&t) {
        let key = if t == "error" { "message" } else { "text" };
        return bounded(&item[key],if t=="reasoning"{THINKING_LIMIT}else{TEXT_LIMIT},false).map(|text|vec![json!({"type":if t=="reasoning"{"thinking"}else if t=="error"{"error"}else{"text"},"text":text})]).unwrap_or_default();
    }
    if t == "todo_list" {
        return activity_plan(&item["items"], "text", true);
    }
    if kind == "item.updated" {
        return vec![];
    }
    let state = if finished { "completed" } else { "started" };
    let ok = !["failed", "declined"].contains(&text(&item["status"]));
    let out = match t {
        "command_execution" => tool(
            "Shell",
            &item["id"],
            state,
            ok && (item["exit_code"].is_null() || item["exit_code"] == 0),
            None,
            None,
            codex_command(item),
        ),
        "file_change" => {
            let changes = codex_changes(item);
            let detail = changes
                .iter()
                .map(|c| {
                    format!(
                        "{} {}",
                        c["kind"].as_str().unwrap_or("change"),
                        text(&c["path"])
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let mut out = tool(
                "Edit",
                &item["id"],
                state,
                ok,
                None,
                if changes.len() == 1 {
                    changes[0]["path"].as_str().map(str::to_owned)
                } else {
                    None
                },
                bounded(&json!(detail), DETAIL_LIMIT, true),
            );
            out["changes"] = json!(
                changes
                    .into_iter()
                    .map(
                        |c| json!({"path":c["path"],"kind":c["kind"].as_str().unwrap_or("change")})
                    )
                    .collect::<Vec<_>>()
            );
            out
        }
        "mcp_tool_call" => tool(
            &format!(
                "MCP {}",
                [text(&item["server"]), text(&item["tool"])]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(".")
            ),
            &item["id"],
            state,
            ok,
            None,
            None,
            None,
        ),
        "web_search" => tool(
            "Web search",
            &item["id"],
            state,
            ok,
            None,
            None,
            bounded(&item["query"], DETAIL_LIMIT, true),
        ),
        "collab_tool_call" | "sub_agent_activity" => tool(
            &format!(
                "Agent {}",
                item["tool"]
                    .as_str()
                    .or_else(|| item["kind"].as_str())
                    .unwrap_or("activity")
            ),
            &item["id"],
            state,
            ok,
            None,
            None,
            None,
        ),
        _ => return vec![],
    };
    vec![out]
}

pub fn normalize(provider: &str, event: &V, include_targets: bool) -> Vec<V> {
    if !["claude", "codex", "cursor"].contains(&provider) || !event.is_object() {
        return vec![];
    }
    let kind = text(&event["type"]);
    if kind.is_empty() {
        return vec![];
    }
    let sid = &event[if provider == "codex" {
        "thread_id"
    } else {
        "session_id"
    }];
    let session = if sid.as_str().is_some_and(crate::delegation::identity) {
        json!({"native_session_id":sid})
    } else {
        json!({})
    };
    if provider == "codex" && kind == "thread.started"
        || provider != "codex" && kind == "system" && event["subtype"] == "init"
    {
        return if session["native_session_id"].is_string() {
            vec![with_fields(json!({"kind":"session"}), &session)]
        } else {
            vec![]
        };
    }
    if provider == "codex" && kind == "turn.started" {
        return vec![json!({"kind":"status","text":"Running"})];
    }
    if kind == "error" {
        return vec![json!({"kind":"error","text":error(event,"Provider error"),"ok":false})];
    }
    if provider == "codex" && ["turn.completed", "turn.failed"].contains(&kind)
        || provider != "codex" && kind == "result"
    {
        let ok = kind == "turn.completed"
            && (event["error"].is_null() || event["error"] == false)
            && event["is_error"] != true
            || kind == "result" && event["subtype"] == "success" && event["is_error"] == false;
        let mut out = usage(event);
        out.push(with_fields(json!({"kind":"result","ok":ok,"text":if ok{text(&event["result"]).to_owned()}else{error(event,"Provider run failed")}}),&session));
        return out;
    }
    if provider == "codex" {
        let item = &event["item"];
        if !item.is_object() {
            return vec![];
        }
        if let Some(phase) = crate::delegation::codex_helper(event) {
            return vec![
                json!({"kind":"tool","text":format!("Agent activity: {phase}"),"ok":true}),
            ];
        }
        let t = text(&item["type"]);
        if kind == "item.completed" && t == "agent_message" {
            return item["text"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| vec![json!({"kind":"text","text":s})])
                .unwrap_or_default();
        }
        if t == "todo_list" && include_targets {
            return plan_event(&item["items"], "text", true, None, false);
        }
        if ["item.started", "item.updated", "item.completed"].contains(&kind)
            && t == "collab_tool_call"
        {
            let tool = text(&item["tool"]);
            let status = text(&item["status"]);
            if !["spawn_agent", "send_input", "wait", "close_agent"].contains(&tool)
                || !["in_progress", "completed", "failed"].contains(&status)
            {
                return vec![];
            }
            let mut label = json!({"kind":"tool","text":format!("Agent {tool}: {status}"),"ok":status!="failed"});
            if include_targets && kind != "item.updated" {
                label["targets"] = targets(
                    "Agent",
                    &item["id"],
                    if kind == "item.completed" {
                        "completed"
                    } else {
                        "started"
                    },
                    status != "failed",
                    None,
                    None,
                    Some(tool.into()),
                );
            }
            return vec![label];
        }
        if ["item.started", "item.completed"].contains(&kind)
            && [
                "command_execution",
                "file_change",
                "mcp_tool_call",
                "web_search",
            ]
            .contains(&t)
        {
            let state = if kind == "item.started" {
                "started"
            } else {
                "completed"
            };
            let ok = item["status"] != "failed"
                && (item["exit_code"].is_null() || item["exit_code"] == 0);
            let mut label = json!({"kind":"tool","text":format!("{t}: {state}"),"ok":ok});
            if include_targets {
                label["targets"] = codex_targets(item, state);
            }
            return vec![label];
        }
        return vec![];
    }
    if kind == "assistant" {
        if !event["error"].is_null() && event["error"] != false {
            return vec![
                json!({"kind":"error","text":error(event,"Assistant request failed"),"ok":false}),
            ];
        }
        if provider == "cursor"
            && event.get("timestamp_ms").is_some()
            && event.get("model_call_id").is_none()
        {
            return vec![];
        }
        let message = &event["message"];
        if !message.is_object() || message["role"].as_str().unwrap_or("assistant") != "assistant" {
            return vec![];
        }
        let parent = &event["parent_tool_use_id"];
        let mut out = vec![];
        for block in message["content"].as_array().into_iter().flatten() {
            if block["type"] == "text" {
                if let Some(t) = block["text"].as_str() {
                    out.push(json!({"kind":"text","text":t}));
                }
            } else if block["type"] == "tool_use" {
                let name = block["name"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Tool");
                let mut label = json!({"kind":"tool","text":format!("{name}: started")});
                if include_targets {
                    let mut t = targets(
                        name,
                        &block["id"],
                        "started",
                        true,
                        if name == "StructuredOutput" {
                            None
                        } else {
                            Some(&block["input"])
                        },
                        None,
                        if name == "StructuredOutput" {
                            Some("Final structured report".into())
                        } else {
                            None
                        },
                    );
                    if let Some(parent) = call_token(parent) {
                        t["parent"] = json!(parent);
                    }
                    label["targets"] = t;
                }
                out.push(label);
                if include_targets && name == "TodoWrite" && parent.is_null() {
                    out.extend(plan_event(
                        &block["input"]["todos"],
                        "content",
                        false,
                        Some("activeForm"),
                        false,
                    ));
                }
            }
        }
        return out;
    }
    if kind == "tool_call" && provider == "cursor" {
        let mut out = vec![];
        for (name, call) in event["tool_call"].as_object().into_iter().flatten() {
            if !call.is_object() {
                continue;
            }
            let label = if name == "function" {
                text(&call["name"])
            } else {
                name
            };
            let label = if label.is_empty() { "Tool" } else { label };
            let outcome = &call["result"];
            let ok = outcome.get("error").is_none();
            let phase = if event["subtype"] == "completed" {
                "completed"
            } else {
                "started"
            };
            let mut event_label = json!({"kind":"tool","text":format!("{label}: {phase}"),"ok":ok});
            if include_targets {
                let (tool, args) = cursor_call(name, call);
                event_label["targets"] =
                    cursor_targets(&tool, &args, &event["call_id"], phase, outcome);
                out.push(event_label);
                if ["updateTodos", "todo"].contains(&tool.as_str()) {
                    let mut plan =
                        plan_event(&outcome["success"]["todos"], "content", false, None, false);
                    if plan.is_empty() && args.is_object() {
                        plan = plan_event(
                            &args["todos"],
                            "content",
                            false,
                            None,
                            args["merge"] == true,
                        );
                    }
                    out.extend(plan);
                }
            } else {
                out.push(event_label);
            }
        }
        return out;
    }
    if kind == "user" && provider == "claude" {
        return event["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["type"] == "tool_result")
            .map(|b| {
                let mut label =
                    json!({"kind":"tool","text":"Tool: completed","ok":b["is_error"]!=true});
                if include_targets {
                    let mut t = targets(
                        "",
                        &b["tool_use_id"],
                        "completed",
                        b["is_error"] != true,
                        None,
                        None,
                        None,
                    );
                    t.as_object_mut().expect("targets").remove("tool");
                    label["targets"] = t;
                }
                label
            })
            .collect();
    }
    vec![]
}
fn with_fields(mut base: V, extra: &V) -> V {
    for (k, v) in extra.as_object().into_iter().flatten() {
        base[k] = v.clone();
    }
    base
}
