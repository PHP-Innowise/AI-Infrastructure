use super::*;

pub(super) fn emit_sql_table_edges(
    ctx: &mut ExtractCtx<'_>,
    sql: &str,
    source_symbol: Option<String>,
    line: u32,
) {
    let upper = sql.to_ascii_uppercase();
    let has_select = contains_sql_verb(&upper, "SELECT") && contains_sql_verb(&upper, "FROM");
    let has_insert = contains_sql_verb(&upper, "INSERT") && contains_sql_verb(&upper, "INTO");
    let has_replace = contains_sql_verb(&upper, "REPLACE") && contains_sql_verb(&upper, "INTO");
    let has_merge = contains_sql_verb(&upper, "MERGE") && contains_sql_verb(&upper, "INTO");
    let has_update = contains_sql_verb(&upper, "UPDATE") && contains_sql_verb(&upper, "SET");
    let has_delete = contains_sql_verb(&upper, "DELETE") && contains_sql_verb(&upper, "FROM");
    let has_truncate = contains_sql_verb(&upper, "TRUNCATE");
    if !(has_select
        || has_insert
        || has_replace
        || has_merge
        || has_update
        || has_delete
        || has_truncate)
    {
        return;
    }

    let tokens = tokenize_sql(sql);
    let mut cte_names: BTreeSet<String> = BTreeSet::new();
    for i in 0..tokens.len() {
        let up = tokens[i].to_ascii_uppercase();
        if up == "WITH" {
            let mut j = i + 1;
            if tokens
                .get(j)
                .is_some_and(|t| t.eq_ignore_ascii_case("RECURSIVE"))
            {
                j += 1;
            }
            if let Some(name) = tokens.get(j).and_then(|t| clean_sql_table_name(t))
                && tokens
                    .get(j + 1)
                    .is_some_and(|t| t.eq_ignore_ascii_case("AS"))
            {
                cte_names.insert(name.to_ascii_lowercase());
            }
        } else if up == ","
            && let Some(name) = tokens.get(i + 1).and_then(|t| clean_sql_table_name(t))
            && tokens
                .get(i + 2)
                .is_some_and(|t| t.eq_ignore_ascii_case("AS"))
            && tokens.get(i + 3).is_some_and(|t| *t == "(")
        {
            cte_names.insert(name.to_ascii_lowercase());
        }
    }

    let mut i = 0;
    while i < tokens.len() {
        let tok_up = tokens[i].to_ascii_uppercase();
        if tok_up == "FROM" && (has_select || has_delete) {
            let mut j = i + 1;
            let mut expect_table = true;
            while j < tokens.len() {
                let raw = tokens[j];
                let up = raw.to_ascii_uppercase();
                if matches!(
                    up.as_str(),
                    "WHERE"
                        | "JOIN"
                        | "INNER"
                        | "LEFT"
                        | "RIGHT"
                        | "FULL"
                        | "OUTER"
                        | "CROSS"
                        | "NATURAL"
                        | "ON"
                        | "USING"
                        | "GROUP"
                        | "ORDER"
                        | "HAVING"
                        | "LIMIT"
                        | "OFFSET"
                        | "UNION"
                        | "EXCEPT"
                        | "INTERSECT"
                        | "SET"
                        | "VALUES"
                        | "RETURNING"
                        | "WINDOW"
                        | "FOR"
                        | ")"
                        | ";"
                ) {
                    break;
                }
                if raw == "," {
                    expect_table = true;
                    j += 1;
                    continue;
                }
                if raw == "(" {
                    // Derived table subquery `(SELECT ...)`: let outer loop scan inside.
                    break;
                }
                if expect_table && let Some(tbl) = clean_sql_table_name(raw) {
                    if !cte_names.contains(&tbl.to_ascii_lowercase()) {
                        ctx.push_semantic(
                            "reads_table",
                            format!("table:{tbl}"),
                            source_symbol.clone(),
                            line,
                            sql,
                        );
                    }
                    expect_table = false;
                }
                j += 1;
            }
            i = j;
            continue;
        } else if tok_up == "JOIN" && has_select {
            if let Some(tbl) = tokens.get(i + 1).and_then(|t| clean_sql_table_name(t))
                && !cte_names.contains(&tbl.to_ascii_lowercase())
            {
                ctx.push_semantic(
                    "reads_table",
                    format!("table:{tbl}"),
                    source_symbol.clone(),
                    line,
                    sql,
                );
            }
        } else if ((tok_up == "INTO" && (has_insert || has_replace || has_merge))
            || (tok_up == "UPDATE" && has_update))
            && let Some(tbl) = tokens.get(i + 1).and_then(|t| clean_sql_table_name(t))
            && !cte_names.contains(&tbl.to_ascii_lowercase())
        {
            ctx.push_semantic(
                "writes_table",
                format!("table:{tbl}"),
                source_symbol.clone(),
                line,
                sql,
            );
        } else if tok_up == "TRUNCATE" && has_truncate {
            let next_idx = if tokens
                .get(i + 1)
                .is_some_and(|t| t.eq_ignore_ascii_case("TABLE"))
            {
                i + 2
            } else {
                i + 1
            };
            if let Some(tbl) = tokens.get(next_idx).and_then(|t| clean_sql_table_name(t)) {
                ctx.push_semantic(
                    "writes_table",
                    format!("table:{tbl}"),
                    source_symbol.clone(),
                    line,
                    sql,
                );
            }
        }
        i += 1;
    }
}

fn tokenize_sql(sql: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i];
        if ch.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if matches!(ch, b',' | b'(' | b')' | b';') {
            out.push(&sql[i..i + 1]);
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && !matches!(bytes[i], b',' | b'(' | b')' | b';')
        {
            i += 1;
        }
        out.push(&sql[start..i]);
    }
    out
}

fn contains_sql_verb(upper: &str, verb: &str) -> bool {
    upper
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|tok| tok == verb)
}

pub(super) fn clean_sql_table_name(raw: &str) -> Option<String> {
    let trimmed = raw
        .trim()
        .trim_matches(|c| matches!(c, '`' | '"' | '[' | ']' | '\'' | ';'));
    if trimmed.is_empty() {
        return None;
    }
    let up = trimmed.to_ascii_uppercase();
    if matches!(
        up.as_str(),
        "SELECT"
            | "WHERE"
            | "SET"
            | "VALUES"
            | "FROM"
            | "JOIN"
            | "INNER"
            | "LEFT"
            | "RIGHT"
            | "OUTER"
            | "CROSS"
            | "ON"
            | "AS"
            | "AND"
            | "OR"
            | "NULL"
            | "TRUE"
            | "FALSE"
            | "DUAL"
            | "LATERAL"
            | "ONLY"
    ) {
        return None;
    }
    if trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        && trimmed
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
    {
        Some(trimmed.to_owned())
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_semantic_from_call(
    ctx: &mut ExtractCtx<'_>,
    receiver: Option<&str>,
    callee: &str,
    args_text: &str,
    string_args: &[String],
    full_chain_text: &str,
    source_symbol: Option<String>,
    line: u32,
) {
    let recv_lower = receiver.unwrap_or("").to_ascii_lowercase();
    let callee_lower = callee.to_ascii_lowercase();
    let evidence = full_chain_text;

    // 1. Routes (`Route::get('/x')`, `app.get('/x')`, `.route('/x')`, `HandleFunc('/x')`, `r.GET('/x')`)
    let mut route_emitted = false;
    let http_verb = match callee_lower.as_str() {
        "get" | "mapget" => Some("GET"),
        "post" | "mappost" => Some("POST"),
        "put" | "mapput" => Some("PUT"),
        "patch" | "mappatch" => Some("PATCH"),
        "delete" | "mapdelete" => Some("DELETE"),
        "options" => Some("OPTIONS"),
        "head" => Some("HEAD"),
        "any" | "match" | "route" | "handle" | "handlefunc" | "resource" | "apiresource" => {
            Some("ANY")
        }
        _ => None,
    };
    if let Some(verb) = http_verb {
        let is_route_receiver = recv_lower.is_empty()
            || recv_lower.contains("route")
            || recv_lower.contains("app")
            || recv_lower.contains("mux")
            || recv_lower.contains("fastify")
            || recv_lower.contains("server")
            || recv_lower.contains("hono")
            || recv_lower.contains("chi")
            || recv_lower.contains("gin")
            || recv_lower.contains("echo")
            || matches!(recv_lower.as_str(), "r" | "e" | "g" | "web")
            || (recv_lower == "http" && matches!(callee_lower.as_str(), "handle" | "handlefunc"));
        if is_route_receiver && let Some(path_arg) = string_args.first() {
            let clean_path = path_arg.trim();
            if !clean_path.is_empty()
                && (clean_path.starts_with('/')
                    || recv_lower.contains("route")
                    || callee_lower == "route"
                    || callee_lower.starts_with("handle")
                    || callee_lower.starts_with("map"))
            {
                let composed_path = ctx.compose_call_route_path(clean_path);
                if let Some(target) = normalize_route_contract(Some(verb), &composed_path, false) {
                    ctx.push_semantic(
                        "exposes_route",
                        target,
                        source_symbol.clone(),
                        line,
                        evidence,
                    );
                    route_emitted = true;
                }
            }
        }
    }

    let args_inner = args_text.trim();
    let args_inner = args_inner.strip_prefix('(').unwrap_or(args_inner);
    let args_inner = args_inner.strip_suffix(')').unwrap_or(args_inner).trim();

    if !route_emitted {
        emit_http_client_route_call(
            ctx,
            &recv_lower,
            &callee_lower,
            args_inner,
            string_args,
            source_symbol.as_deref(),
            line,
            evidence,
        );
    }

    // 2. DI bindings & container resolutions (`bind`, `singleton`, `scoped`, `app()->make(...)`, `Container::get(...)`, `resolve(...)`)
    let is_di_binding = matches!(
        callee_lower.as_str(),
        "bind" | "singleton" | "scoped" | "addsingleton" | "addscoped" | "addtransient" | "provide"
    );
    let is_express_app_get = callee_lower == "get"
        && recv_lower == "app"
        && (!string_args.is_empty()
            || (matches!(ctx.grammar, AstGrammar::TypeScript | AstGrammar::JavaScript) && {
                let parts = split_top_level_commas(args_inner);
                parts.len() >= 2 && !parts[1].trim_start().starts_with('{')
            }));
    let is_container_resolve = !route_emitted
        && !is_express_app_get
        && !args_inner.is_empty()
        && ((matches!(
            callee_lower.as_str(),
            "make" | "makewith" | "get" | "resolve"
        ) && is_di_container_receiver(&recv_lower))
            || (recv_lower.is_empty()
                && ((callee_lower == "app")
                    || (callee_lower == "resolve" && ctx.grammar == AstGrammar::Php))));
    if is_di_binding || is_container_resolve {
        let targets = extract_di_targets(args_inner, args_text, string_args, full_chain_text);
        for svc in targets {
            ctx.push_semantic(
                "di_binds",
                format!("service:{svc}"),
                source_symbol.clone(),
                line,
                evidence,
            );
            if is_container_resolve && svc.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                ctx.push_ref(AstRef {
                    target_name: svc,
                    member: None,
                    context: "di_resolve".to_owned(),
                    access: "type".to_owned(),
                    member_kind: None,
                    owner_type: None,
                    enclosing_symbol: source_symbol.clone(),
                    is_static: false,
                    line,
                    evidence: clip_str(evidence, 160),
                });
            }
        }
    }

    // 3. Domain events / queues (`event(...)`, `dispatch(...)`, `emit(...)`, `publish(...)`, `enqueue(...)`, `produce(...)`, `broadcast(...)`, `sendEvent(...)`, `Event::listen(...)`)
    if matches!(
        callee_lower.as_str(),
        "event"
            | "dispatch"
            | "dispatchsync"
            | "dispatchafterresponse"
            | "emit"
            | "publish"
            | "publishevent"
            | "produce"
            | "sendevent"
            | "enqueue"
            | "broadcast"
    ) {
        let event_name =
            extract_event_name_from_args(receiver, &callee_lower, args_inner, string_args);
        if let Some(ev) = event_name {
            ctx.push_semantic(
                "publishes_event",
                format!("event:{ev}"),
                source_symbol.clone(),
                line,
                evidence,
            );
        }
    } else if matches!(
        callee_lower.as_str(),
        "listen" | "subscribe" | "on" | "consume"
    ) && (recv_lower.contains("event")
        || recv_lower.contains("bus")
        || recv_lower.contains("emitter")
        || recv_lower.contains("broker")
        || recv_lower.contains("pubsub")
        || recv_lower.contains("queue")
        || recv_lower.contains("kafka")
        || recv_lower.contains("rabbit")
        || matches!(callee_lower.as_str(), "listen" | "consume"))
        && let Some(ev) =
            extract_event_name_from_args(receiver, &callee_lower, args_inner, string_args)
    {
        ctx.push_semantic(
            "handles_event",
            format!("event:{ev}"),
            source_symbol.clone(),
            line,
            evidence,
        );
    }

    // 4. ORM `DB::table('tbl')`
    if callee_lower == "table"
        && (recv_lower == "db" || recv_lower == "schema" || recv_lower.ends_with("\\db"))
        && let Some(tbl) = string_args.first().and_then(|s| clean_sql_table_name(s))
    {
        let chain_lower = full_chain_text.to_ascii_lowercase();
        let kind = if chain_lower.contains("->insert")
            || chain_lower.contains("->update")
            || chain_lower.contains("->delete")
            || chain_lower.contains("->upsert")
        {
            "writes_table"
        } else {
            "reads_table"
        };
        ctx.push_semantic(kind, format!("table:{tbl}"), source_symbol, line, evidence);
    }
}

fn extract_event_name_from_args(
    receiver: Option<&str>,
    callee_lower: &str,
    args_text: &str,
    string_args: &[String],
) -> Option<String> {
    if callee_lower == "dispatch"
        && let Some(recv) = receiver
    {
        let short = recv.rsplit(['\\', ':', '.']).next().unwrap_or(recv).trim();
        if !short.is_empty()
            && !matches!(
                short.to_ascii_lowercase().as_str(),
                "event" | "bus" | "dispatcher" | "this"
            )
            && short.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        {
            return Some(short.to_owned());
        }
    }
    if let Some(s) = string_args.first()
        && !s.trim().is_empty()
    {
        return Some(s.trim().to_owned());
    }
    for part in args_text.split(',') {
        let trimmed = part.trim();
        if let Some(cls) = extract_class_ref_name(trimmed) {
            return Some(cls);
        }
        if let Some(after_new) = trimmed.strip_prefix("new ") {
            let cls = after_new
                .split(['(', '{', ' '])
                .next()
                .unwrap_or("")
                .rsplit(['\\', ':', '.'])
                .next()
                .unwrap_or("")
                .trim();
            if !cls.is_empty() {
                return Some(cls.to_owned());
            }
        }
    }
    None
}

pub(super) fn emit_semantic_from_attribute(
    ctx: &mut ExtractCtx<'_>,
    attr_name: &str,
    attr_text: &str,
    string_args: &[String],
    annotated_symbol: Option<&str>,
    param_types: &[String],
    line: u32,
) {
    let short = attr_name
        .rsplit(['\\', ':', '.'])
        .next()
        .unwrap_or(attr_name)
        .trim_start_matches(['@', '#']);
    let lower = short.to_ascii_lowercase();

    // Routes: `Route`, `GetMapping`, `PostMapping`, `PutMapping`, `DeleteMapping`, `PatchMapping`,
    // `RequestMapping`, `HttpGet`, `HttpPost`, `HttpPut`, `HttpDelete`, `HttpPatch`, `get`, `post`, etc.
    let route_verb = match lower.as_str() {
        "getmapping" | "httpget" | "get" => Some("GET"),
        "postmapping" | "httppost" | "post" => Some("POST"),
        "putmapping" | "httpput" | "put" => Some("PUT"),
        "deletemapping" | "httpdelete" | "delete" => Some("DELETE"),
        "patchmapping" | "httppatch" | "patch" => Some("PATCH"),
        "route" | "requestmapping" | "path" => Some("ANY"),
        _ => None,
    };
    if let Some(verb) = route_verb {
        if let Some(path) = string_args.first() {
            let composed = ctx.compose_attr_route_path(path);
            if let Some(target) = normalize_route_contract(Some(verb), &composed, false) {
                ctx.push_semantic(
                    "exposes_route",
                    target,
                    annotated_symbol.map(str::to_owned),
                    line,
                    attr_text,
                );
            }
        } else if verb != "ANY" {
            let composed = ctx.compose_attr_route_path("/");
            if let Some(target) = normalize_route_contract(Some(verb), &composed, false) {
                ctx.push_semantic(
                    "exposes_route",
                    target,
                    annotated_symbol.map(str::to_owned),
                    line,
                    attr_text,
                );
            }
        }
    }

    // DI bindings: `Injectable`, `Service`, `Repository`, `Component`, `Bean`, `AsAlias`
    if matches!(
        lower.as_str(),
        "injectable" | "service" | "repository" | "component" | "bean" | "asalias" | "inject"
    ) {
        let svc = extract_class_ref_name(attr_text)
            .or_else(|| string_args.first().cloned())
            .or_else(|| annotated_symbol.map(str::to_owned));
        if let Some(s) = svc {
            ctx.push_semantic(
                "di_binds",
                format!("service:{s}"),
                annotated_symbol.map(str::to_owned),
                line,
                attr_text,
            );
        }
    }

    // Event handlers: `AsMessageHandler`, `AsEventListener`, `EventListener`, `EventHandler`, `KafkaListener`, `RabbitListener`, `OnEvent`, `EventPattern`, `MessagePattern`, `receiver`
    if matches!(
        lower.as_str(),
        "asmessagehandler"
            | "aseventlistener"
            | "eventlistener"
            | "eventhandler"
            | "kafkalistener"
            | "rabbitlistener"
            | "onevent"
            | "eventpattern"
            | "messagepattern"
            | "receiver"
    ) {
        let ev = extract_class_ref_name(attr_text)
            .or_else(|| string_args.first().cloned())
            .or_else(|| param_types.first().cloned());
        if let Some(e) = ev {
            ctx.push_semantic(
                "handles_event",
                format!("event:{e}"),
                annotated_symbol.map(str::to_owned),
                line,
                attr_text,
            );
        }
    }

    // ORM table attributes: `Table` (`#[ORM\Table(name: 'tbl')]`, `@Table(name = "tbl")`)
    if lower == "table"
        && let Some(tbl) = string_args.first().and_then(|s| clean_sql_table_name(s))
    {
        ctx.push_semantic(
            "reads_table",
            format!("table:{tbl}"),
            annotated_symbol.map(str::to_owned),
            line,
            attr_text,
        );
    }

    // Test attributes across languages (`@Test`, `#[Test]`, `[Fact]`, `[Theory]`, `[TestMethod]`)
    if matches!(
        lower.as_str(),
        "test" | "fact" | "theory" | "testmethod" | "parameterizedtest"
    ) {
        ctx.test_lines.insert(line);
    }
}

fn split_top_level_commas(raw: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth_paren = 0usize;
    let mut depth_angle = 0usize;
    let mut depth_bracket = 0usize;
    let mut depth_brace = 0usize;
    let mut in_quote: Option<char> = None;
    let mut escape = false;
    let mut start = 0usize;

    for (idx, ch) in raw.char_indices() {
        if let Some(q) = in_quote {
            if escape {
                escape = false;
                continue;
            }
            if ch == '\\' {
                escape = true;
            } else if ch == q {
                in_quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' | '`' => in_quote = Some(ch),
            '(' => depth_paren += 1,
            ')' => depth_paren = depth_paren.saturating_sub(1),
            '<' => depth_angle += 1,
            '>' => {
                // Skip `=>` closure arrows so they do not decrement angle bracket depth.
                if !raw[..idx].ends_with('=') {
                    depth_angle = depth_angle.saturating_sub(1);
                }
            }
            '[' => depth_bracket += 1,
            ']' => depth_bracket = depth_bracket.saturating_sub(1),
            '{' => depth_brace += 1,
            '}' => depth_brace = depth_brace.saturating_sub(1),
            ',' if depth_paren == 0
                && depth_angle == 0
                && depth_bracket == 0
                && depth_brace == 0 =>
            {
                parts.push(raw[start..idx].trim());
                start = idx + 1;
            }
            _ => {}
        }
    }
    if start <= raw.len() {
        let tail = raw[start..].trim();
        if !tail.is_empty() {
            parts.push(tail);
        }
    }
    parts
}

fn clean_di_type_token(raw: &str) -> Option<String> {
    let cleaned = raw
        .trim()
        .trim_start_matches("typeof")
        .trim()
        .trim_matches(|c: char| {
            matches!(
                c,
                '(' | ')' | '[' | ']' | '{' | '}' | ' ' | '\t' | '\n' | '\r'
            )
        })
        .trim();
    if cleaned.is_empty() || cleaned.starts_with(['\'', '"', '`', '$', '|']) {
        return None;
    }
    if cleaned.starts_with("fn")
        || cleaned.starts_with("function")
        || cleaned.contains("=>")
        || cleaned.starts_with('{')
    {
        return None;
    }
    let candidate = cleaned.strip_prefix("dyn ").unwrap_or(cleaned).trim();
    let short = candidate
        .rsplit(['\\', ':', '.'])
        .next()
        .unwrap_or(candidate)
        .split('<')
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
    if short.is_empty()
        || !short.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        || matches!(short, "Closure" | "Object" | "Array" | "String")
    {
        return None;
    }
    Some(short.to_owned())
}

fn extract_new_type_in_closure(raw: &str) -> Option<String> {
    let idx = raw.find("new ")?;
    let after = raw[idx + 4..].trim_start();
    let tok = after
        .split(|c: char| {
            !c.is_ascii_alphanumeric() && c != '_' && c != '\\' && c != ':' && c != '.'
        })
        .next()
        .unwrap_or("");
    clean_di_type_token(tok)
}

fn is_di_container_receiver(recv_lower: &str) -> bool {
    let r = recv_lower.trim();
    matches!(
        r,
        "app"
            | "app()"
            | "$app"
            | "$this->app"
            | "$this->container"
            | "container"
            | "$container"
            | "injector"
            | "$injector"
            | "moduleref"
            | "servicelocator"
            | "di"
    ) || r.ends_with("\\app")
        || r.ends_with("\\container")
        || r.ends_with("->app")
        || r.ends_with("->container")
        || r.ends_with(".container")
        || r.ends_with(".injector")
}

pub(super) fn extract_di_targets(
    args_inner: &str,
    args_text: &str,
    string_args: &[String],
    full_chain_text: &str,
) -> Vec<String> {
    let mut targets = Vec::new();
    let mut push_unique = |val: String| {
        if !val.is_empty() && !targets.contains(&val) {
            targets.push(val);
        }
    };

    for part in split_top_level_commas(args_inner) {
        let p = part.trim();
        if let Some(ty) = extract_class_ref_name(p) {
            push_unique(ty);
            continue;
        }
        if let Some(inner) = p.strip_prefix("typeof(").and_then(|s| s.strip_suffix(')'))
            && let Some(ty) = clean_di_type_token(inner)
        {
            push_unique(ty);
            continue;
        }
        if (p.contains("provide:") || p.contains("useClass:") || p.contains("useExisting:"))
            && (p.starts_with('{') || p.contains("useClass:"))
        {
            let trimmed_obj = p.trim_matches(|c: char| matches!(c, '{' | '}' | ' '));
            for kv in split_top_level_commas(trimmed_obj) {
                if let Some((k, v)) = kv.split_once(':') {
                    let key = k.trim().trim_matches(|c: char| matches!(c, '{' | ' '));
                    if matches!(key, "provide" | "useClass" | "useExisting")
                        && let Some(ty) = clean_di_type_token(v)
                    {
                        push_unique(ty);
                    }
                }
            }
            continue;
        }
        if let Some(ty) = extract_new_type_in_closure(p) {
            push_unique(ty);
            continue;
        }
        if !p.contains(' ')
            && let Some(ty) = clean_di_type_token(p)
        {
            push_unique(ty);
        }
    }

    // Generic type arguments immediately preceding `args_text` (e.g. `AddSingleton<ITokenCache, RedisTokenCache>()`).
    let chain_prefix = full_chain_text
        .trim_end()
        .strip_suffix(args_text.trim())
        .unwrap_or(full_chain_text)
        .trim_end();
    if let Some(without_gt) = chain_prefix.strip_suffix('>')
        && let Some(lt_idx) = without_gt.rfind('<')
    {
        let generic_inner = &without_gt[lt_idx + 1..];
        for arg in split_top_level_commas(generic_inner) {
            if let Some(ty) = clean_di_type_token(arg) {
                push_unique(ty);
            }
        }
    }

    if targets.is_empty()
        && let Some(s) = string_args.first()
        && !s.trim().is_empty()
    {
        targets.push(s.trim().to_owned());
    }
    targets
}

/// Collapses brace-balanced JS/TS template interpolations (`${...}`) to `placeholder`.
/// Rejects empty (`${}`) or unclosed interpolations and any whitespace outside `${...}`.
/// Returns `(collapsed, leading_template_slash)` where `leading_template_slash` is `true`
/// when `raw` starts with `${...}` immediately followed by `/`.
struct TemplateExprState {
    brace_depth: usize,
    quote: Option<u8>,
    escaped: bool,
}

fn collapse_route_template_interpolations(raw: &str, placeholder: &str) -> Option<(String, bool)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let bytes = trimmed.as_bytes();
    let mut out = String::with_capacity(trimmed.len());
    let mut idx = 0usize;

    while idx < bytes.len() {
        if bytes[idx] == b'$' && idx + 1 < bytes.len() && bytes[idx + 1] == b'{' {
            let expr_start = idx + 2;
            let mut cursor = expr_start;
            let mut stack = vec![TemplateExprState {
                brace_depth: 1,
                quote: None,
                escaped: false,
            }];
            while cursor < bytes.len() && !stack.is_empty() {
                let b = bytes[cursor];
                let frame = stack.last_mut().expect("stack non-empty");
                if let Some(q) = frame.quote {
                    if frame.escaped {
                        frame.escaped = false;
                    } else if b == b'\\' {
                        frame.escaped = true;
                    } else if q == b'`'
                        && b == b'$'
                        && cursor + 1 < bytes.len()
                        && bytes[cursor + 1] == b'{'
                    {
                        stack.push(TemplateExprState {
                            brace_depth: 1,
                            quote: None,
                            escaped: false,
                        });
                        cursor += 2;
                        continue;
                    } else if b == q {
                        frame.quote = None;
                    }
                    cursor += 1;
                    continue;
                }
                match b {
                    b'\'' | b'"' | b'`' => frame.quote = Some(b),
                    b'{' => frame.brace_depth += 1,
                    b'}' => {
                        frame.brace_depth -= 1;
                        if frame.brace_depth == 0 {
                            stack.pop();
                        }
                    }
                    _ => {}
                }
                cursor += 1;
            }
            if !stack.is_empty() || trimmed[expr_start..cursor - 1].trim().is_empty() {
                return None;
            }
            out.push_str(placeholder);
            idx = cursor;
            continue;
        }
        let ch = trimmed[idx..].chars().next()?;
        if ch.is_whitespace() {
            return None;
        }
        out.push(ch);
        idx += ch.len_utf8();
    }

    let leading_template_slash =
        out.starts_with(placeholder) && out.trim_start_matches(placeholder).starts_with('/');
    Some((out, leading_template_slash))
}

fn find_closing_delim(chars: &[char], open_idx: usize, open: char, close: char) -> Option<usize> {
    let mut i = open_idx + 1;
    let mut depth = 1usize;
    while i < chars.len() {
        if chars[i] == open {
            depth += 1;
        } else if chars[i] == close {
            depth -= 1;
            if depth == 0 {
                return (i > open_idx + 1).then_some(i);
            }
        }
        i += 1;
    }
    None
}

/// Normalizes a single URL path segment (whole-segment or inline placeholders) to `placeholder`.
fn normalize_route_segment(seg: &str, placeholder: &str) -> String {
    if seg == placeholder || (seg.starts_with(':') && seg.len() > 1) {
        return placeholder.to_owned();
    }
    let chars: Vec<char> = seg.chars().collect();
    let mut out = String::with_capacity(seg.len());
    let mut i = 0usize;
    while i < chars.len() {
        let close = match chars[i] {
            '{' => Some('}'),
            '<' => Some('>'),
            '[' => Some(']'),
            _ => None,
        };
        if let Some(close_ch) = close
            && let Some(end) = find_closing_delim(&chars, i, chars[i], close_ch)
        {
            out.push_str(placeholder);
            i = end + 1;
            continue;
        }
        out.push(chars[i].to_ascii_lowercase());
        i += 1;
    }
    out
}

/// Normalizes a route specification and optional HTTP verb into canonical `route:<VERB|*> /norm/{param}`.
pub(crate) fn normalize_route_contract(
    verb: Option<&str>,
    raw_path: &str,
    require_leading_slash_or_http: bool,
) -> Option<String> {
    let (collapsed, leading_template_slash) =
        collapse_route_template_interpolations(raw_path, "{param}")?;
    let path_part = if let Some(rest) = collapsed
        .strip_prefix("http://")
        .or_else(|| collapsed.strip_prefix("https://"))
    {
        match rest.find('/') {
            Some(slash) => &rest[slash..],
            None => "/",
        }
    } else if leading_template_slash {
        collapsed.trim_start_matches("{param}")
    } else if collapsed.starts_with('/') {
        collapsed.as_str()
    } else if require_leading_slash_or_http {
        return None;
    } else {
        collapsed.as_str()
    };
    let without_query = path_part.split(['?', '#']).next().unwrap_or(path_part);
    let mut norm_segments = Vec::new();
    for seg in without_query.split('/') {
        if seg.is_empty() {
            continue;
        }
        norm_segments.push(normalize_route_segment(seg, "{param}"));
    }
    let norm_path = if norm_segments.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}", norm_segments.join("/"))
    };
    let norm_verb = match verb.map(str::trim) {
        Some(v) if !v.is_empty() && !v.eq_ignore_ascii_case("ANY") && v != "*" => {
            v.to_ascii_uppercase()
        }
        _ => "*".to_owned(),
    };
    Some(format!("route:{norm_verb} {norm_path}"))
}

pub(super) fn extract_http_verb_token(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_uppercase().as_str() {
        "GET" => Some("GET"),
        "POST" => Some("POST"),
        "PUT" => Some("PUT"),
        "PATCH" => Some("PATCH"),
        "DELETE" => Some("DELETE"),
        "HEAD" => Some("HEAD"),
        "OPTIONS" => Some("OPTIONS"),
        _ => None,
    }
}

fn extract_explicit_http_verb(args_inner: &str, string_args: &[String]) -> Option<&'static str> {
    for arg in string_args {
        if let Some(v) = extract_http_verb_token(arg) {
            return Some(v);
        }
    }
    let lower = args_inner.to_ascii_lowercase();
    for (needle, verb) in [
        ("method:'post'", "POST"),
        ("method:\"post\"", "POST"),
        ("method:'put'", "PUT"),
        ("method:\"put\"", "PUT"),
        ("method:'patch'", "PATCH"),
        ("method:\"patch\"", "PATCH"),
        ("method:'delete'", "DELETE"),
        ("method:\"delete\"", "DELETE"),
        ("method:'get'", "GET"),
        ("method:\"get\"", "GET"),
    ] {
        let compact: String = lower.chars().filter(|c| !c.is_whitespace()).collect();
        if compact.contains(needle) {
            return Some(verb);
        }
    }
    None
}

fn is_http_client_receiver(recv_lower: &str) -> bool {
    matches!(
        recv_lower,
        "axios"
            | "http"
            | "httpclient"
            | "requests"
            | "httpx"
            | "got"
            | "ky"
            | "superagent"
            | "guzzle"
            | "resttemplate"
            | "webclient"
            | "apiclient"
            | "restclient"
            | "client"
            | "api"
    ) || [
        ".axios",
        ".http",
        "\\http",
        "::http",
        "->http",
        ".httpclient",
        "->httpclient",
        "httpclient",
        ".apiclient",
        "apiclient",
        ".restclient",
        "restclient",
        ".client",
        "->client",
        ".api",
        "->api",
    ]
    .iter()
    .any(|suffix| recv_lower.ends_with(suffix))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_http_client_route_call(
    ctx: &mut ExtractCtx<'_>,
    recv_lower: &str,
    callee_lower: &str,
    args_inner: &str,
    string_args: &[String],
    source_symbol: Option<&str>,
    line: u32,
    evidence: &str,
) {
    let mut verb: Option<&'static str> = None;
    let is_fetch_or_axios_fn = (callee_lower == "fetch"
        && (recv_lower.is_empty()
            || matches!(
                recv_lower,
                "window" | "globalthis" | "self" | "this" | "$this"
            )))
        || (callee_lower == "axios" && recv_lower.is_empty());

    if is_fetch_or_axios_fn {
        verb = Some(extract_explicit_http_verb(args_inner, string_args).unwrap_or("GET"));
    } else if is_http_client_receiver(recv_lower) {
        verb = match callee_lower {
            "get" | "getasync" | "getjson" | "getforobject" | "getforentity" => Some("GET"),
            "post" | "postasync" | "postjson" | "postforobject" | "postforentity" => Some("POST"),
            "put" | "putasync" => Some("PUT"),
            "patch" | "patchasync" => Some("PATCH"),
            "delete" | "deleteasync" => Some("DELETE"),
            "head" => Some("HEAD"),
            "options" => Some("OPTIONS"),
            "request" | "send" | "newrequest" => {
                Some(extract_explicit_http_verb(args_inner, string_args).unwrap_or("ANY"))
            }
            _ => None,
        };
    }

    let Some(http_method) = verb else {
        return;
    };
    let Some(target) = string_args
        .iter()
        .find_map(|arg| normalize_route_contract(Some(http_method), arg, true))
    else {
        return;
    };
    ctx.push_semantic(
        "calls_route",
        target,
        source_symbol.map(str::to_owned),
        line,
        evidence,
    );
}
