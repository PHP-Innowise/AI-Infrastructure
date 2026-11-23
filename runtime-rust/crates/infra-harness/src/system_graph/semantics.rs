use super::helpers::collect_service_code_files;
use super::resolver::{MAX_UNDECLARED_SCAN_BYTES, ResolvedServiceTarget};
use crate::{
    graph::Graphs,
    graph_map::{self, Index},
};
use infra_system::catalog::System;
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
    sync::{Arc, OnceLock},
};

/// What code that uses a contract would hold of it: its id, the path,
/// route, endpoint, topic, event or queue it is reached by, and the paths
/// and dotted names its summary gives.
pub(super) fn contract_probes(prov: &Value) -> Vec<String> {
    let mut probes = Vec::new();
    if let Some(cid) = prov["id"].as_str().filter(|cid| cid.len() >= 4) {
        probes.push(cid.to_owned());
    }
    for key in ["path", "route", "endpoint", "topic", "event", "queue"] {
        if let Some(v) = prov[key].as_str().filter(|s| s.len() >= 4) {
            probes.push(v.to_owned());
        }
    }
    if let Some(summary) = prov["summary"].as_str() {
        for token in summary.split_whitespace() {
            if (token.starts_with('/') && token.len() >= 5)
                || (token.contains('.') && token.len() >= 6)
            {
                probes.push(
                    token
                        .trim_matches(|c: char| c == ',' || c == ';')
                        .to_owned(),
                );
            }
        }
    }
    probes
}

/// How many files of a provider are read for what names a contract, how
/// many bytes in all, and how many files of each side are listed.
const BRIDGE_FILES: usize = 400;
const BRIDGE_BYTES: u64 = 8 * 1024 * 1024;
const BRIDGE_LISTED: usize = 24;

struct SemanticBridgeRegexes {
    php_route: Regex,
    php_api_resource: Regex,
    php_attr_route: Regex,
    js_route: Regex,
    decorator_route: Regex,
    py_route: Regex,
    rust_route: Regex,
    rust_attr_route: Regex,
    go_route: Regex,
    http_call_fn: Regex,
    url_literal: Regex,
    path_literal: Regex,
    php_event_new: Regex,
    php_event_dispatch_static: Regex,
    php_event_dispatch_str: Regex,
    generic_publish_str: Regex,
    generic_publish_new: Regex,
    php_event_listen: Regex,
    php_as_message_handler: Regex,
    php_handle_typed_event: Regex,
    decorator_event_listen: Regex,
    broker_listener: Regex,
    generic_subscribe_str: Regex,
    spring_event_listener: Regex,
    py_receiver: Regex,
    laravel_di_bind: Regex,
    symfony_as_alias: Regex,
    js_inject_provide: Regex,
    spring_di_qualifier: Regex,
    rust_di_type: Regex,
    sql_delete: Regex,
    sql_insert: Regex,
    sql_update: Regex,
    sql_create_alter: Regex,
    sql_select_from: Regex,
    sql_join: Regex,
    laravel_db_table: Regex,
    knex_table: Regex,
    prisma_model: Regex,
    orm_table_decl: Regex,
}

fn semantic_regexes() -> &'static SemanticBridgeRegexes {
    static RE: OnceLock<SemanticBridgeRegexes> = OnceLock::new();
    RE.get_or_init(|| SemanticBridgeRegexes {
        php_route: Regex::new(
            r#"(?i)\bRoute::(get|post|put|patch|delete|any|options)\s*\(\s*['"]([^'"]+)['"]"#,
        )
        .expect("php_route"),
        php_api_resource: Regex::new(r#"\bRoute::apiResource\s*\(\s*['"]([^'"]+)['"]"#)
            .expect("php_api_resource"),
        php_attr_route: Regex::new(
            r#"#\[\s*Route\s*\(\s*['"]([^'"]+)['"](?:[^)\]]*methods:\s*\[?\s*['"]([A-Za-z]+)['"])?"#,
        )
        .expect("php_attr_route"),
        js_route: Regex::new(
            r#"\b(?:app|router|fastify)\.(get|post|put|patch|delete|all)\s*\(\s*['"`]([^'"`]+)['"`]"#,
        )
        .expect("js_route"),
        decorator_route: Regex::new(
            r#"@(Get|Post|Put|Patch|Delete|RequestMapping|GetMapping|PostMapping|PutMapping|PatchMapping|DeleteMapping)\s*\(\s*(?:value\s*=\s*|path\s*=\s*)?['"]([^'"]+)['"]"#,
        )
        .expect("decorator_route"),
        py_route: Regex::new(
            r#"@(?:app|router|bp|blueprint)\.(get|post|put|patch|delete|route)\s*\(\s*['"]([^'"]+)['"]"#,
        )
        .expect("py_route"),
        rust_route: Regex::new(
            r#"\.route\s*\(\s*"([^"]+)"\s*,\s*(?:[a-z_]+::)?(get|post|put|patch|delete)?"#,
        )
        .expect("rust_route"),
        rust_attr_route: Regex::new(r#"#\[\s*(get|post|put|patch|delete)\s*\(\s*"([^"]+)""#)
            .expect("rust_attr_route"),
        go_route: Regex::new(
            r#"\.(HandleFunc|GET|POST|PUT|PATCH|DELETE|Get|Post|Put|Patch|Delete)\s*\(\s*"([^"]+)""#,
        )
        .expect("go_route"),
        http_call_fn: Regex::new(
            r#"(?i)(?:\bfetch|\baxios(?:\.(?:get|post|put|patch|delete|request))?|\bHttp::(?:get|post|put|patch|delete)|\.?(?:get|post|put|patch|delete|request))\s*\(\s*['"`]((?:https?://[^'"`?\s)]+)?/[^'"`?\s)]+)['"`]"#,
        )
        .expect("http_call_fn"),
        url_literal: Regex::new(r#"['"`](https?://[A-Za-z0-9_.:-]+(/[^'"`?\s)]+))['"`]"#)
            .expect("url_literal"),
        path_literal: Regex::new(
            r#"['"`](/[A-Za-z0-9_{}:.-]+(?:/[A-Za-z0-9_{}:.-]+)+)['"`]"#,
        )
        .expect("path_literal"),
        php_event_new: Regex::new(
            r#"\bevent\s*\(\s*new\s+(?:\\?[A-Za-z0-9_]+\\)*([A-Z][A-Za-z0-9_]+)"#,
        )
        .expect("php_event_new"),
        php_event_dispatch_static: Regex::new(r#"\b([A-Z][A-Za-z0-9_]+)::dispatch\s*\("#)
            .expect("php_event_dispatch_static"),
        php_event_dispatch_str: Regex::new(
            r#"\b(?:Event::dispatch|dispatch)\s*\(\s*['"]([A-Za-z0-9_.:/-]{3,120})['"]"#,
        )
        .expect("php_event_dispatch_str"),
        generic_publish_str: Regex::new(
            r#"(?:\b|\.)(?:publish|emit|dispatch|produce|publishEvent|sendEvent|enqueue|broadcast)\s*\(\s*['"`]([A-Za-z0-9_.:/-]{3,120})['"`]"#,
        )
        .expect("generic_publish_str"),
        generic_publish_new: Regex::new(
            r#"(?:\b|\.)(?:publish|emit|dispatch|publishEvent)\s*\(\s*new\s+(?:\\?[A-Za-z0-9_]+\\)*([A-Z][A-Za-z0-9_]+)"#,
        )
        .expect("generic_publish_new"),
        php_event_listen: Regex::new(
            r#"\bEvent::listen\s*\(\s*(?:['"]([A-Za-z0-9_\\.:/-]{3,120})['"]|(?:\\?[A-Za-z0-9_]+\\)*([A-Z][A-Za-z0-9_]+)::class)"#,
        )
        .expect("php_event_listen"),
        php_as_message_handler: Regex::new(
            r#"#\[\s*AsMessageHandler[^\]]*\][\s\S]{0,240}?function\s+\w+\s*\(\s*(?:\\?[A-Za-z0-9_]+\\)*([A-Z][A-Za-z0-9_]+)"#,
        )
        .expect("php_as_message_handler"),
        php_handle_typed_event: Regex::new(
            r#"function\s+(?:handle|__invoke)\s*\(\s*(?:\\?[A-Za-z0-9_]+\\)*([A-Z][A-Za-z0-9_]*(?:Event|Message|Job|Notification))\b"#,
        )
        .expect("php_handle_typed_event"),
        decorator_event_listen: Regex::new(
            r#"@(?:OnEvent|EventPattern|MessagePattern)\s*\(\s*['"`]([A-Za-z0-9_.:/-]{3,120})['"`]"#,
        )
        .expect("decorator_event_listen"),
        broker_listener: Regex::new(
            r#"@(?:KafkaListener|RabbitListener)\s*\([^)]*(?:topics|queues)\s*=\s*['"]([A-Za-z0-9_.:/-]{3,120})['"]"#,
        )
        .expect("broker_listener"),
        generic_subscribe_str: Regex::new(
            r#"(?:\b|\.)(?:subscribe|consume|listen|on)\s*\(\s*['"`]([A-Za-z0-9_.:/-]{3,120})['"`]"#,
        )
        .expect("generic_subscribe_str"),
        spring_event_listener: Regex::new(
            r#"@EventListener\b[\s\S]{0,160}?\(\s*([A-Z][A-Za-z0-9_]+)\b"#,
        )
        .expect("spring_event_listener"),
        py_receiver: Regex::new(r#"@receiver\s*\(\s*([A-Za-z0-9_.]+)"#).expect("py_receiver"),
        laravel_di_bind: Regex::new(
            r#"->\s*(?:bind|singleton|scoped)\s*\(\s*(?:\\?[A-Za-z0-9_]+\\)*([A-Z][A-Za-z0-9_]+)::class\s*,\s*(?:\\?[A-Za-z0-9_]+\\)*([A-Z][A-Za-z0-9_]+)::class"#,
        )
        .expect("laravel_di_bind"),
        symfony_as_alias: Regex::new(
            r#"#\[\s*AsAlias\s*\(\s*(?:id:\s*)?(?:\\?[A-Za-z0-9_]+\\)*([A-Z][A-Za-z0-9_]+)::class"#,
        )
        .expect("symfony_as_alias"),
        js_inject_provide: Regex::new(
            r#"(?:@Inject\s*\(\s*['"`]([A-Za-z0-9_.:/-]{4,120})['"`]|\bprovide\s*:\s*['"`]([A-Za-z0-9_.:/-]{4,120})['"`])"#,
        )
        .expect("js_inject_provide"),
        spring_di_qualifier: Regex::new(
            r#"@(?:Qualifier|Service)\s*\(\s*["']([A-Za-z0-9_.:/-]{4,120})["']"#,
        )
        .expect("spring_di_qualifier"),
        rust_di_type: Regex::new(
            r#"(?:(?:Extension|State)\s*<\s*(?:[A-Za-z0-9_]+::)*([A-Z][A-Za-z0-9_]+)\s*>|Arc\s*<\s*dyn\s+(?:[A-Za-z0-9_]+::)*([A-Z][A-Za-z0-9_]+))"#,
        )
        .expect("rust_di_type"),
        sql_delete: Regex::new(r#"(?i)\bDELETE\s+FROM\s+[`"\[]?([a-zA-Z_][a-zA-Z0-9_]{1,63})[`"\]]?"#)
            .expect("sql_delete"),
        sql_insert: Regex::new(
            r#"(?i)\bINSERT\s+(?:IGNORE\s+)?INTO\s+[`"\[]?([a-zA-Z_][a-zA-Z0-9_]{1,63})[`"\]]?"#,
        )
        .expect("sql_insert"),
        sql_update: Regex::new(
            r#"(?i)\bUPDATE\s+[`"\[]?([a-zA-Z_][a-zA-Z0-9_]{1,63})[`"\]]?\s+SET\b"#,
        )
        .expect("sql_update"),
        sql_create_alter: Regex::new(
            r#"(?i)\b(?:CREATE|ALTER)\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?[`"\[]?([a-zA-Z_][a-zA-Z0-9_]{1,63})[`"\]]?"#,
        )
        .expect("sql_create_alter"),
        sql_select_from: Regex::new(
            r#"(?is)\bSELECT\b.{1,240}?\bFROM\s+[`"\[]?([a-zA-Z_][a-zA-Z0-9_]{1,63})[`"\]]?"#,
        )
        .expect("sql_select_from"),
        sql_join: Regex::new(r#"(?i)\bJOIN\s+[`"\[]?([a-zA-Z_][a-zA-Z0-9_]{1,63})[`"\]]?"#)
            .expect("sql_join"),
        laravel_db_table: Regex::new(
            r#"\bDB::table\s*\(\s*['"]([a-zA-Z_][a-zA-Z0-9_]{1,63})['"]\s*\)([^;\n]{0,200})"#,
        )
        .expect("laravel_db_table"),
        knex_table: Regex::new(
            r#"\bknex\s*\(\s*['"]([a-zA-Z_][a-zA-Z0-9_]{1,63})['"]\s*\)([^;\n]{0,200})"#,
        )
        .expect("knex_table"),
        prisma_model: Regex::new(
            r#"\bprisma\.([a-zA-Z_][a-zA-Z0-9_]{1,63})\.(findMany|findUnique|findFirst|count|aggregate|create|createMany|update|updateMany|delete|deleteMany|upsert)\b"#,
        )
        .expect("prisma_model"),
        orm_table_decl: Regex::new(
            r#"(?:\$table\s*=\s*['"]([a-zA-Z_][a-zA-Z0-9_]{1,63})['"]|#\[\s*ORM\\Table\s*\(\s*(?:name:\s*)?['"]([a-zA-Z_][a-zA-Z0-9_]{1,63})['"]|@Table\s*\(\s*(?:name\s*=\s*)?["']([a-zA-Z_][a-zA-Z0-9_]{1,63})["']|__tablename__\s*=\s*['"]([a-zA-Z_][a-zA-Z0-9_]{1,63})['"]|table!\s*\{\s*([a-zA-Z_][a-zA-Z0-9_]{1,63}))"#,
        )
        .expect("orm_table_decl"),
    })
}

#[derive(Debug, Default)]
struct RouteSite {
    display: String,
    methods: BTreeSet<String>,
    files: BTreeSet<String>,
}

#[derive(Debug, Default)]
struct TopicSite {
    display: String,
    files: BTreeSet<String>,
}

#[derive(Debug, Default)]
struct DiSite {
    display: String,
    kinds: BTreeSet<String>,
    files: BTreeSet<String>,
}

#[derive(Default)]
struct ServiceSemanticSignals {
    exposed_routes: BTreeMap<String, RouteSite>,
    called_routes: BTreeMap<String, RouteSite>,
    published_events: BTreeMap<String, TopicSite>,
    handled_events: BTreeMap<String, TopicSite>,
    di_bindings: BTreeMap<String, DiSite>,
    table_reads: BTreeMap<String, BTreeSet<String>>,
    table_writes: BTreeMap<String, BTreeSet<String>>,
}

fn normalize_route_path(raw: &str) -> Option<(String, String)> {
    let contract = crate::graph_ast::normalize_route_contract(None, raw, false)?;
    let without_query = contract.strip_prefix("route:* ")?.trim_end_matches('/');
    if without_query.is_empty()
        || without_query == "/"
        || matches!(
            without_query,
            "/health" | "/healthz" | "/ready" | "/metrics" | "/favicon.ico"
        )
        || [
            "/etc/", "/usr/", "/var/", "/tmp/", "/dev/", "/proc/", "/bin/", "/opt/",
        ]
        .iter()
        .any(|prefix| without_query.starts_with(prefix))
    {
        return None;
    }
    let norm = without_query.replace("{param}", "{*}");
    Some((norm, without_query.to_owned()))
}

fn routes_match(exposed_norm: &str, called_norm: &str) -> bool {
    if exposed_norm == called_norm {
        return true;
    }
    let exp: Vec<&str> = exposed_norm.trim_start_matches('/').split('/').collect();
    let cal: Vec<&str> = called_norm.trim_start_matches('/').split('/').collect();
    if exp.len() < 2 || exp.len() != cal.len() {
        return false;
    }
    exp.iter()
        .zip(cal.iter())
        .all(|(e, c)| e == c || *e == "{*}" || *c == "{*}")
}

fn normalize_event_topic(raw: &str) -> Option<(String, String)> {
    let trimmed = raw
        .trim()
        .trim_matches(|c: char| matches!(c, '\'' | '"' | '`' | ' '));
    let short = trimmed.rsplit('\\').next().unwrap_or(trimmed).trim();
    if short.len() < 3 {
        return None;
    }
    let lower = short.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "error"
            | "data"
            | "end"
            | "close"
            | "open"
            | "exit"
            | "click"
            | "change"
            | "input"
            | "focus"
            | "blur"
            | "connect"
            | "disconnect"
            | "listening"
            | "message"
            | "ready"
            | "drain"
            | "finish"
            | "pipe"
            | "unpipe"
            | "warning"
            | "timeout"
    ) {
        return None;
    }
    Some((lower, short.to_owned()))
}

fn normalize_di_symbol(raw: &str) -> Option<(String, String)> {
    let trimmed = raw
        .trim()
        .rsplit(['\\', ':'])
        .next()
        .unwrap_or(raw.trim())
        .trim();
    if trimmed.len() < 4 {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "service"
            | "client"
            | "config"
            | "logger"
            | "context"
            | "state"
            | "appstate"
            | "request"
            | "response"
            | "string"
            | "array"
            | "object"
    ) {
        return None;
    }
    Some((lower, trimmed.to_owned()))
}

fn normalize_table_name(raw: &str) -> Option<String> {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.len() < 2 {
        return None;
    }
    if matches!(
        lower.as_str(),
        "select"
            | "from"
            | "where"
            | "set"
            | "values"
            | "null"
            | "true"
            | "false"
            | "dual"
            | "inner"
            | "left"
            | "right"
            | "outer"
            | "join"
            | "as"
            | "on"
            | "and"
            | "or"
            | "table"
            | "into"
            | "exists"
            | "case"
            | "when"
            | "then"
            | "else"
            | "end"
            | "order"
            | "group"
            | "by"
            | "limit"
            | "offset"
            | "having"
            | "distinct"
            | "count"
            | "sum"
            | "avg"
            | "min"
            | "max"
            | "not"
            | "is"
            | "in"
            | "like"
            | "between"
            | "union"
            | "all"
            | "any"
    ) {
        return None;
    }
    Some(lower)
}

impl ServiceSemanticSignals {
    fn add_exposed_route(&mut self, raw_path: &str, method: Option<&str>, file: &str) {
        if let Some((norm, display)) = normalize_route_path(raw_path) {
            let entry = self
                .exposed_routes
                .entry(norm)
                .or_insert_with(|| RouteSite {
                    display,
                    methods: BTreeSet::new(),
                    files: BTreeSet::new(),
                });
            if let Some(m) = method.map(str::trim).filter(|m| !m.is_empty()) {
                entry.methods.insert(m.to_ascii_uppercase());
            }
            entry.files.insert(file.to_owned());
        }
    }

    fn add_called_route(&mut self, raw_path: &str, method: Option<&str>, file: &str) {
        if let Some((norm, display)) = normalize_route_path(raw_path) {
            let entry = self.called_routes.entry(norm).or_insert_with(|| RouteSite {
                display,
                methods: BTreeSet::new(),
                files: BTreeSet::new(),
            });
            if let Some(m) = method.map(str::trim).filter(|m| !m.is_empty()) {
                entry.methods.insert(m.to_ascii_uppercase());
            }
            entry.files.insert(file.to_owned());
        }
    }

    fn add_published_event(&mut self, raw: &str, file: &str) {
        if let Some((key, display)) = normalize_event_topic(raw) {
            let entry = self
                .published_events
                .entry(key)
                .or_insert_with(|| TopicSite {
                    display,
                    files: BTreeSet::new(),
                });
            entry.files.insert(file.to_owned());
        }
    }

    fn add_handled_event(&mut self, raw: &str, file: &str) {
        if let Some((key, display)) = normalize_event_topic(raw) {
            let entry = self.handled_events.entry(key).or_insert_with(|| TopicSite {
                display,
                files: BTreeSet::new(),
            });
            entry.files.insert(file.to_owned());
        }
    }

    fn add_di_binding(&mut self, raw: &str, kind: &str, file: &str) {
        if let Some((key, display)) = normalize_di_symbol(raw) {
            let entry = self.di_bindings.entry(key).or_insert_with(|| DiSite {
                display,
                kinds: BTreeSet::new(),
                files: BTreeSet::new(),
            });
            entry.kinds.insert(kind.to_owned());
            entry.files.insert(file.to_owned());
        }
    }

    fn add_table_read(&mut self, raw: &str, file: &str) {
        if let Some(tbl) = normalize_table_name(raw) {
            self.table_reads
                .entry(tbl)
                .or_default()
                .insert(file.to_owned());
        }
    }

    fn add_table_write(&mut self, raw: &str, file: &str) {
        if let Some(tbl) = normalize_table_name(raw) {
            self.table_writes
                .entry(tbl)
                .or_default()
                .insert(file.to_owned());
        }
    }

    fn scan_source(&mut self, rel_path: &str, content: &str) {
        let re = semantic_regexes();
        // 1. Route definitions
        for cap in re.php_route.captures_iter(content) {
            self.add_exposed_route(&cap[2], Some(&cap[1]), rel_path);
        }
        for cap in re.php_api_resource.captures_iter(content) {
            self.add_exposed_route(&cap[1], Some("RESOURCE"), rel_path);
        }
        for cap in re.php_attr_route.captures_iter(content) {
            self.add_exposed_route(&cap[1], cap.get(2).map(|m| m.as_str()), rel_path);
        }
        for cap in re.js_route.captures_iter(content) {
            self.add_exposed_route(&cap[2], Some(&cap[1]), rel_path);
        }
        for cap in re.decorator_route.captures_iter(content) {
            let method = match &cap[1] {
                "Get" | "GetMapping" => Some("GET"),
                "Post" | "PostMapping" => Some("POST"),
                "Put" | "PutMapping" => Some("PUT"),
                "Patch" | "PatchMapping" => Some("PATCH"),
                "Delete" | "DeleteMapping" => Some("DELETE"),
                _ => None,
            };
            self.add_exposed_route(&cap[2], method, rel_path);
        }
        for cap in re.py_route.captures_iter(content) {
            let method = match &cap[1] {
                "route" => None,
                other => Some(other),
            };
            self.add_exposed_route(&cap[2], method, rel_path);
        }
        for cap in re.rust_route.captures_iter(content) {
            self.add_exposed_route(&cap[1], cap.get(2).map(|m| m.as_str()), rel_path);
        }
        for cap in re.rust_attr_route.captures_iter(content) {
            self.add_exposed_route(&cap[2], Some(&cap[1]), rel_path);
        }
        for cap in re.go_route.captures_iter(content) {
            let method = match &cap[1] {
                "HandleFunc" => None,
                other => Some(other),
            };
            self.add_exposed_route(&cap[2], method, rel_path);
        }

        // 2. HTTP / route calls
        for cap in re.http_call_fn.captures_iter(content) {
            self.add_called_route(&cap[1], None, rel_path);
        }
        for cap in re.url_literal.captures_iter(content) {
            self.add_called_route(&cap[1], None, rel_path);
        }
        for cap in re.path_literal.captures_iter(content) {
            if let Some((norm, _)) = normalize_route_path(&cap[1])
                && !self.exposed_routes.contains_key(&norm)
            {
                self.add_called_route(&cap[1], None, rel_path);
            }
        }

        // 3. Event publishers & handlers
        for cap in re.php_event_new.captures_iter(content) {
            self.add_published_event(&cap[1], rel_path);
        }
        for cap in re.php_event_dispatch_static.captures_iter(content) {
            self.add_published_event(&cap[1], rel_path);
        }
        for cap in re.php_event_dispatch_str.captures_iter(content) {
            self.add_published_event(&cap[1], rel_path);
        }
        for cap in re.generic_publish_str.captures_iter(content) {
            self.add_published_event(&cap[1], rel_path);
        }
        for cap in re.generic_publish_new.captures_iter(content) {
            self.add_published_event(&cap[1], rel_path);
        }
        for cap in re.php_event_listen.captures_iter(content) {
            if let Some(m) = cap.get(1).or_else(|| cap.get(2)) {
                self.add_handled_event(m.as_str(), rel_path);
            }
        }
        for cap in re.php_as_message_handler.captures_iter(content) {
            self.add_handled_event(&cap[1], rel_path);
        }
        for cap in re.php_handle_typed_event.captures_iter(content) {
            self.add_handled_event(&cap[1], rel_path);
        }
        for cap in re.decorator_event_listen.captures_iter(content) {
            self.add_handled_event(&cap[1], rel_path);
        }
        for cap in re.broker_listener.captures_iter(content) {
            self.add_handled_event(&cap[1], rel_path);
        }
        for cap in re.generic_subscribe_str.captures_iter(content) {
            self.add_handled_event(&cap[1], rel_path);
        }
        for cap in re.spring_event_listener.captures_iter(content) {
            self.add_handled_event(&cap[1], rel_path);
        }
        for cap in re.py_receiver.captures_iter(content) {
            self.add_handled_event(&cap[1], rel_path);
        }

        // 4. DI / service bindings
        for cap in re.laravel_di_bind.captures_iter(content) {
            self.add_di_binding(&cap[1], "binds", rel_path);
            self.add_di_binding(&cap[2], "binds", rel_path);
        }
        for cap in re.symfony_as_alias.captures_iter(content) {
            self.add_di_binding(&cap[1], "binds", rel_path);
        }
        for cap in re.js_inject_provide.captures_iter(content) {
            if let Some(m) = cap.get(1) {
                self.add_di_binding(m.as_str(), "injects", rel_path);
            }
            if let Some(m) = cap.get(2) {
                self.add_di_binding(m.as_str(), "binds", rel_path);
            }
        }
        for cap in re.spring_di_qualifier.captures_iter(content) {
            self.add_di_binding(&cap[1], "binds", rel_path);
        }
        for cap in re.rust_di_type.captures_iter(content) {
            if let Some(m) = cap.get(1).or_else(|| cap.get(2)) {
                self.add_di_binding(m.as_str(), "injects", rel_path);
            }
        }

        // 5. Database table reads & writes
        for cap in re.sql_delete.captures_iter(content) {
            self.add_table_write(&cap[1], rel_path);
        }
        for cap in re.sql_insert.captures_iter(content) {
            self.add_table_write(&cap[1], rel_path);
        }
        for cap in re.sql_update.captures_iter(content) {
            self.add_table_write(&cap[1], rel_path);
        }
        for cap in re.sql_create_alter.captures_iter(content) {
            self.add_table_write(&cap[1], rel_path);
        }
        for cap in re.sql_select_from.captures_iter(content) {
            self.add_table_read(&cap[1], rel_path);
        }
        for cap in re.sql_join.captures_iter(content) {
            self.add_table_read(&cap[1], rel_path);
        }
        for cap in re.laravel_db_table.captures_iter(content) {
            let chain = cap[2].to_ascii_lowercase();
            if [
                "insert",
                "update",
                "delete",
                "upsert",
                "increment",
                "decrement",
                "truncate",
            ]
            .iter()
            .any(|kw| chain.contains(kw))
            {
                self.add_table_write(&cap[1], rel_path);
            } else {
                self.add_table_read(&cap[1], rel_path);
            }
        }
        for cap in re.knex_table.captures_iter(content) {
            let chain = cap[2].to_ascii_lowercase();
            if [".insert", ".update", ".del", ".delete", ".truncate"]
                .iter()
                .any(|kw| chain.contains(kw))
            {
                self.add_table_write(&cap[1], rel_path);
            } else {
                self.add_table_read(&cap[1], rel_path);
            }
        }
        for cap in re.prisma_model.captures_iter(content) {
            match &cap[2] {
                "create" | "createMany" | "update" | "updateMany" | "delete" | "deleteMany"
                | "upsert" => self.add_table_write(&cap[1], rel_path),
                _ => self.add_table_read(&cap[1], rel_path),
            }
        }
        let lower_content = content.to_ascii_lowercase();
        let has_write_hint = ["insert", "update", "delete", "save(", "persist(", "create("]
            .iter()
            .any(|kw| lower_content.contains(kw));
        let has_read_hint = ["select", "find", "get(", "fetch", "query", "where("]
            .iter()
            .any(|kw| lower_content.contains(kw));
        for cap in re.orm_table_decl.captures_iter(content) {
            if let Some(m) = cap
                .get(1)
                .or_else(|| cap.get(2))
                .or_else(|| cap.get(3))
                .or_else(|| cap.get(4))
                .or_else(|| cap.get(5))
            {
                if has_write_hint {
                    self.add_table_write(m.as_str(), rel_path);
                }
                if has_read_hint || !has_write_hint {
                    self.add_table_read(m.as_str(), rel_path);
                }
            }
        }
    }

    fn ingest_semantic_relation(&mut self, relation: &str, target_label: &str, from_file: &str) {
        match relation {
            "exposes_route" | "route" => {
                let raw = target_label.strip_prefix("route:").unwrap_or(target_label);
                let (method, path) = match raw.trim().split_once(' ') {
                    Some((m, p)) if p.starts_with('/') => (Some(m), p),
                    _ => (None, raw),
                };
                self.add_exposed_route(path, method, from_file);
            }
            "calls_route" => {
                let raw = target_label.strip_prefix("route:").unwrap_or(target_label);
                let (method, path) = match raw.trim().split_once(' ') {
                    Some((m, p)) if p.starts_with('/') => (Some(m), p),
                    _ => (None, raw),
                };
                self.add_called_route(path, method, from_file);
            }
            "di_binds" | "di_binding" => {
                let raw = target_label
                    .strip_prefix("service:")
                    .unwrap_or(target_label);
                self.add_di_binding(raw, "binds", from_file);
            }
            "publishes_event" => {
                let raw = target_label.strip_prefix("event:").unwrap_or(target_label);
                self.add_published_event(raw, from_file);
            }
            "handles_event" => {
                let raw = target_label.strip_prefix("event:").unwrap_or(target_label);
                self.add_handled_event(raw, from_file);
            }
            "reads_table" | "table_read" => {
                let raw = target_label.strip_prefix("table:").unwrap_or(target_label);
                self.add_table_read(raw, from_file);
            }
            "writes_table" | "table_write" => {
                let raw = target_label.strip_prefix("table:").unwrap_or(target_label);
                self.add_table_write(raw, from_file);
            }
            _ => {}
        }
    }

    fn ingest_ir_store_ref(
        &mut self,
        store: &crate::graph_ir::GraphIrStore,
        certain: bool,
    ) -> Option<Vec<String>> {
        let (files, edges) = store.semantic_edges_for_system().ok()?;
        if files.is_empty() && edges.is_empty() {
            return None;
        }
        for (file_path, kind, target, _source_symbol, _line, evidence) in &edges {
            let guessed = evidence.eq_ignore_ascii_case("AMBIGUOUS")
                || evidence.eq_ignore_ascii_case("INFERRED");
            if certain && guessed {
                continue;
            }
            if kind == "di_binds" && evidence.to_ascii_lowercase().contains("inject") {
                let raw = target.strip_prefix("service:").unwrap_or(target);
                self.add_di_binding(raw, "injects", file_path);
            } else {
                self.ingest_semantic_relation(kind, target, file_path);
            }
        }
        Some(files)
    }

    #[allow(dead_code)]
    fn ingest_ir_store(
        &mut self,
        graphs: Option<&Graphs>,
        state_dir: &Path,
        graph_id: &str,
        certain: bool,
    ) -> Option<Vec<String>> {
        let store = graphs
            .and_then(|g| g.get_ir_store_readonly(graph_id))
            .or_else(|| {
                crate::graph_ir::GraphIrStore::open_readonly(state_dir, graph_id)
                    .ok()
                    .flatten()
                    .map(Arc::new)
            })?;
        self.ingest_ir_store_ref(&store, certain)
    }

    fn ingest_index(&mut self, index: &Index, certain: bool) {
        let files = index.files();
        let symbols = index.symbol_entries();
        for &(from, to, kind_id, guessed) in index.uses() {
            if certain && guessed {
                continue;
            }
            let relation = index.kind_name(kind_id);
            let from_file = files
                .get(symbols[from as usize].file as usize)
                .map(String::as_str)
                .unwrap_or("");
            let target_label = index.symbol_label(to);
            self.ingest_semantic_relation(relation, target_label, from_file);
        }
    }
}

fn read_bounded_service_file(
    root: &Path,
    canon_root: Option<&Path>,
    rel_path: &str,
) -> Option<(String, u64)> {
    if rel_path.is_empty()
        || rel_path.starts_with('/')
        || rel_path.contains('\\')
        || rel_path
            .split('/')
            .any(|part| part.is_empty() || part == "..")
    {
        return None;
    }
    let full_path = root.join(rel_path);
    if let Some(canon_root) = canon_root {
        let canon_file = infra_core::fs::canonicalize(&full_path).ok()?;
        if !canon_file.starts_with(canon_root) {
            return None;
        }
    }
    let meta = fs::symlink_metadata(&full_path).ok()?;
    if !meta.is_file() || meta.len() > MAX_UNDECLARED_SCAN_BYTES {
        return None;
    }
    let file = fs::File::open(&full_path).ok()?;
    let mut buf = Vec::with_capacity(meta.len().min(MAX_UNDECLARED_SCAN_BYTES) as usize);
    file.take(MAX_UNDECLARED_SCAN_BYTES + 1)
        .read_to_end(&mut buf)
        .ok()?;
    if buf.len() as u64 > MAX_UNDECLARED_SCAN_BYTES {
        return None;
    }
    let len = buf.len() as u64;
    let content = String::from_utf8(buf).ok()?;
    Some((content, len))
}

#[cfg(test)]
fn collect_service_semantics_with(
    state_dir: Option<&Path>,
    target: Option<&ResolvedServiceTarget>,
    index: Option<&Arc<Index>>,
    certain: bool,
    need_content_probes: bool,
    on_file: impl FnMut(&str, &str),
) -> (ServiceSemanticSignals, usize, bool) {
    collect_service_semantics_with_graphs(
        None,
        state_dir,
        target,
        index,
        certain,
        need_content_probes,
        on_file,
    )
}

fn extract_probe_subtokens(raw: &str) -> Vec<String> {
    raw.split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '$')
        .map(|s| s.trim().trim_start_matches('$').to_ascii_lowercase())
        .filter(|s| s.len() >= 2)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Queries SQLite `ir_symbol_refs` and `ir_semantic_edges` for files matching any probe string.
pub(super) fn find_ir_probe_matches(
    ir: &crate::graph_ir::GraphIrStore,
    probes: &[String],
) -> Vec<(String, String)> {
    if probes.is_empty() || !ir.has_symbol_refs_index().unwrap_or(false) {
        return Vec::new();
    }
    let probe_tokens: Vec<(String, Vec<String>)> = probes
        .iter()
        .filter(|p| !p.trim().is_empty())
        .map(|p| (p.clone(), extract_probe_subtokens(p)))
        .filter(|(_, toks)| !toks.is_empty())
        .collect();
    if probe_tokens.is_empty() {
        return Vec::new();
    }
    let all_tok_set: BTreeSet<String> = probe_tokens
        .iter()
        .flat_map(|(_, toks)| toks.iter().cloned())
        .collect();
    let tok_refs: Vec<&str> = all_tok_set.iter().map(String::as_str).collect();
    let Ok(ref_hits) = ir.find_probe_matching_files(&tok_refs) else {
        return Vec::new();
    };
    let mut matches = Vec::new();
    for (file_path, file_toks) in ref_hits {
        let tok_set: BTreeSet<&str> = file_toks.iter().map(String::as_str).collect();
        for (probe, required) in &probe_tokens {
            if required.iter().all(|req| tok_set.contains(req.as_str())) {
                matches.push((file_path.clone(), probe.clone()));
            }
        }
    }
    if let Ok((_, sem_edges)) = ir.semantic_edges_for_system() {
        for (file_path, _kind, target, _src_sym, _line, _ev) in sem_edges {
            let t_lower = target.to_ascii_lowercase();
            for (probe, required) in &probe_tokens {
                let p_lower = probe.to_ascii_lowercase();
                if t_lower == p_lower || required.iter().all(|req| t_lower.contains(req.as_str())) {
                    matches.push((file_path.clone(), probe.clone()));
                }
            }
        }
    }
    matches.sort();
    matches.dedup();
    matches
}

pub(super) fn ensure_ir_store_for_system(
    graphs: Option<&Graphs>,
    state_dir: Option<&Path>,
    target: Option<&ResolvedServiceTarget>,
) -> Option<Arc<crate::graph_ir::GraphIrStore>> {
    let (state, tgt) = state_dir.zip(target)?;
    if let Some(existing) = graphs
        .and_then(|g| g.get_ir_store_readonly(&tgt.graph_id))
        .or_else(|| {
            crate::graph_ir::GraphIrStore::open_readonly(state, &tgt.graph_id)
                .ok()
                .flatten()
                .map(Arc::new)
        })
        && existing
            .summary_counts()
            .ok()
            .flatten()
            .is_some_and(|s| s.files > 0)
    {
        return Some(existing);
    }
    if !tgt.root.is_dir() {
        return None;
    }
    if tgt.graph_id.is_empty()
        || tgt.graph_id == "."
        || tgt.graph_id == ".."
        || tgt.graph_id.contains(['/', '\\', '\0'])
    {
        return None;
    }
    // Keep system-level incremental IR cache isolated from `state/graphs/<id>` so services with
    // disabled code graphs do not falsely flip `provider["graph"]` / `GraphIrStore::exists`.
    let sys_ir_dir = state.join("system-ir");
    let db_path = sys_ir_dir.join(format!("{}.sqlite", tgt.graph_id));
    let store = Arc::new(crate::graph_ir::GraphIrStore::open_at(&db_path).ok()?);
    // Incremental workspace sync checks mtime/size/hash per file so new/edited/deleted files update fast.
    let _ = store.sync_workspace(&tgt.root);
    Some(store)
}

fn collect_service_semantics_with_graphs(
    graphs: Option<&Graphs>,
    state_dir: Option<&Path>,
    target: Option<&ResolvedServiceTarget>,
    index: Option<&Arc<Index>>,
    certain: bool,
    need_content_probes: bool,
    mut on_file: impl FnMut(&str, &str),
) -> (ServiceSemanticSignals, usize, bool) {
    let mut signals = ServiceSemanticSignals::default();
    let ir_store_opt = ensure_ir_store_for_system(graphs, state_dir, target);
    let ir_files = ir_store_opt
        .as_ref()
        .and_then(|store| signals.ingest_ir_store_ref(store, certain));
    let ir_loaded = ir_files.is_some();
    if let Some(idx) = index {
        signals.ingest_index(idx, certain);
    }
    let Some(target) = target else {
        return (signals, 0, false);
    };
    if ir_loaded && !need_content_probes {
        return (signals, 0, false);
    }
    let canon_root = infra_core::fs::canonicalize(&target.root).ok();
    let files = match (ir_files, index) {
        (Some(files), _) => files
            .into_iter()
            .filter(|path| !graph_map::is_test_path(path))
            .collect::<Vec<_>>(),
        (None, Some(idx)) => idx
            .files()
            .iter()
            .filter(|path| !graph_map::is_test_path(path))
            .cloned()
            .collect::<Vec<_>>(),
        (None, None) => collect_service_code_files(&target.root, BRIDGE_FILES),
    };
    let mut files_read = 0usize;
    let mut bytes_read = 0u64;
    let mut cut = false;
    for rel_path in files {
        if rel_path == "ai-service.json" || rel_path.ends_with(".lock") {
            continue;
        }
        if files_read >= BRIDGE_FILES || bytes_read > BRIDGE_BYTES {
            cut = true;
            break;
        }
        let Some((content, len)) =
            read_bounded_service_file(&target.root, canon_root.as_deref(), &rel_path)
        else {
            continue;
        };
        files_read += 1;
        bytes_read += len;
        if !ir_loaded {
            signals.scan_source(&rel_path, &content);
        }
        on_file(&rel_path, &content);
    }
    (signals, files_read, cut)
}

fn build_semantic_bridges(
    from: &str,
    to: &str,
    from_sig: &ServiceSemanticSignals,
    to_sig: &ServiceSemanticSignals,
) -> Value {
    let mut routes = Vec::new();
    let mut push_routes = |caller: &str,
                           provider: &str,
                           direction: &str,
                           calls: &BTreeMap<String, RouteSite>,
                           exposed: &BTreeMap<String, RouteSite>| {
        for (cal_norm, cal_site) in calls {
            for (exp_norm, exp_site) in exposed {
                if routes_match(exp_norm, cal_norm) {
                    routes.push(json!({
                        "route": exp_site.display,
                        "method": exp_site.methods.iter().next().map(String::as_str).unwrap_or("ANY"),
                        "methods": exp_site.methods.iter().cloned().collect::<Vec<_>>(),
                        "direction": direction,
                        "caller": caller,
                        "provider": provider,
                        "caller_files": cal_site.files.iter().cloned().collect::<Vec<_>>(),
                        "provider_files": exp_site.files.iter().cloned().collect::<Vec<_>>(),
                    }));
                }
            }
        }
    };
    push_routes(
        from,
        to,
        "from->to",
        &from_sig.called_routes,
        &to_sig.exposed_routes,
    );
    push_routes(
        to,
        from,
        "to->from",
        &to_sig.called_routes,
        &from_sig.exposed_routes,
    );
    routes.truncate(BRIDGE_LISTED);

    let mut events = Vec::new();
    let mut push_events = |producer: &str,
                           consumer: &str,
                           direction: &str,
                           pubs: &BTreeMap<String, TopicSite>,
                           handlers: &BTreeMap<String, TopicSite>| {
        for (topic_key, pub_site) in pubs {
            if let Some(sub_site) = handlers.get(topic_key) {
                events.push(json!({
                    "event": pub_site.display,
                    "direction": direction,
                    "producer": producer,
                    "consumer": consumer,
                    "producer_files": pub_site.files.iter().cloned().collect::<Vec<_>>(),
                    "consumer_files": sub_site.files.iter().cloned().collect::<Vec<_>>(),
                }));
            }
        }
    };
    push_events(
        from,
        to,
        "from->to",
        &from_sig.published_events,
        &to_sig.handled_events,
    );
    push_events(
        to,
        from,
        "to->from",
        &to_sig.published_events,
        &from_sig.handled_events,
    );
    events.truncate(BRIDGE_LISTED);

    let mut di_bindings = Vec::new();
    for (key, left) in &from_sig.di_bindings {
        if let Some(right) = to_sig.di_bindings.get(key) {
            di_bindings.push(json!({
                "service": left.display,
                "from_kinds": left.kinds.iter().cloned().collect::<Vec<_>>(),
                "to_kinds": right.kinds.iter().cloned().collect::<Vec<_>>(),
                "from_files": left.files.iter().cloned().collect::<Vec<_>>(),
                "to_files": right.files.iter().cloned().collect::<Vec<_>>(),
            }));
        }
    }
    di_bindings.truncate(BRIDGE_LISTED);

    let mut all_tables = BTreeSet::new();
    for tbl in from_sig
        .table_reads
        .keys()
        .chain(from_sig.table_writes.keys())
    {
        if to_sig.table_reads.contains_key(tbl) || to_sig.table_writes.contains_key(tbl) {
            all_tables.insert(tbl.clone());
        }
    }
    let mut tables = Vec::new();
    for tbl in all_tables.into_iter().take(BRIDGE_LISTED) {
        let from_r = from_sig.table_reads.get(&tbl);
        let from_w = from_sig.table_writes.get(&tbl);
        let to_r = to_sig.table_reads.get(&tbl);
        let to_w = to_sig.table_writes.get(&tbl);
        let mut from_access = Vec::new();
        if from_r.is_some() {
            from_access.push("read");
        }
        if from_w.is_some() {
            from_access.push("write");
        }
        let mut to_access = Vec::new();
        if to_r.is_some() {
            to_access.push("read");
        }
        if to_w.is_some() {
            to_access.push("write");
        }
        let coupling = if from_w.is_some() && to_w.is_some() {
            "shared_write"
        } else if from_w.is_some() || to_w.is_some() {
            "cross_read_write"
        } else {
            "shared_read"
        };
        let mut from_files = BTreeSet::new();
        for f in from_r
            .into_iter()
            .flatten()
            .chain(from_w.into_iter().flatten())
        {
            from_files.insert(f.clone());
        }
        let mut to_files = BTreeSet::new();
        for f in to_r.into_iter().flatten().chain(to_w.into_iter().flatten()) {
            to_files.insert(f.clone());
        }
        tables.push(json!({
            "table": tbl,
            "coupling": coupling,
            "from_access": from_access,
            "to_access": to_access,
            "from_files": from_files.into_iter().collect::<Vec<_>>(),
            "to_files": to_files.into_iter().collect::<Vec<_>>(),
        }));
    }

    let total = routes.len() + events.len() + di_bindings.len() + tables.len();
    json!({
        "routes": routes,
        "events": events,
        "di_bindings": di_bindings,
        "tables": tables,
        "summary": {
            "routes": routes.len(),
            "events": events.len(),
            "di_bindings": di_bindings.len(),
            "tables": tables.len(),
            "total": total,
        },
    })
}

pub(super) struct BridgeViewContext<'a> {
    pub(super) target_spec: Option<&'a str>,
    pub(super) run_id: Option<&'a str>,
    pub(super) opened_by_service: Option<&'a BTreeMap<String, BTreeSet<String>>>,
}

/// The code on the two sides of a link between two services. Of the
/// consumer, the files that name what it uses of the provider, as the code
/// references of the system graph found them. Of the provider, the files
/// its passport says a contract is made in, and the files that name the
/// contract. Each file is told how many files of its own service a change
/// of it reaches, where the service has a code graph: that is how far the
/// contract goes into each side.
pub(super) fn bridge_view(
    graphs: &Graphs,
    system: &System,
    targets: &[ResolvedServiceTarget],
    link: &Value,
    contract: Option<&str>,
    certain: bool,
    ctx: &BridgeViewContext<'_>,
) -> Value {
    let (from, to) = (
        link["from"].as_str().unwrap_or(""),
        link["to"].as_str().unwrap_or(""),
    );
    let ids = match contract {
        Some(contract) => vec![contract.to_owned()],
        None => link["contracts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c["id"].as_str().map(str::to_owned))
            .collect::<Vec<_>>(),
    };
    let index_of = |service: &str| {
        targets
            .iter()
            .find(|target| target.service_id == service)
            .and_then(|target| graphs.index_for_target_or_live(&target.graph_id, ctx.target_spec))
    };
    let reach = |index: &Option<Arc<Index>>, path: &str| match index {
        Some(index) => index
            .blast(path, certain)
            .map(|blast| json!({"files": blast["files"], "tests": blast["tests"]}))
            .unwrap_or(Value::Null),
        None => Value::Null,
    };
    // What the passport names first, then code, then the documents that
    // name the contract as well.
    let document = |path: &str| {
        let path = path.to_ascii_lowercase();
        [".md", ".markdown", ".rst", ".txt", ".adoc"]
            .iter()
            .any(|end| path.ends_with(end))
    };
    let service_opened = |service: &str, path: &str| -> bool {
        let clean = path.replace('\\', "/").trim_start_matches("./").to_owned();
        ctx.opened_by_service
            .and_then(|map| map.get(service))
            .is_some_and(|set| set.contains(&clean))
    };
    let listed = |service: &str,
                  sites: BTreeMap<String, (BTreeSet<String>, Value)>,
                  index: &Option<Arc<Index>>| {
        let total = sites.len();
        let more = total.saturating_sub(BRIDGE_LISTED);
        let opened_total = ctx.opened_by_service.map_or(0usize, |_| {
            sites
                .keys()
                .filter(|path| service_opened(service, path))
                .count()
        });
        let mut sites = sites.into_iter().collect::<Vec<_>>();
        sites.sort_by_key(|(path, (_, row))| {
            (row["declared"] != true, document(path), path.clone())
        });
        let rows = sites
            .into_iter()
            .take(BRIDGE_LISTED)
            .map(|(path, (contracts, mut row))| {
                row["document"] = json!(document(&path));
                row["path"] = json!(path);
                row["contracts"] = json!(contracts);
                row["in_graph"] = json!(
                    index
                        .as_ref()
                        .is_some_and(|index| index.file_id(&path).is_some())
                );
                row["reach"] = reach(index, &path);
                if ctx.run_id.is_some() {
                    row["opened_by_run"] = json!(service_opened(service, &path));
                }
                row
            })
            .collect::<Vec<_>>();
        (rows, more, opened_total, total)
    };
    // The consumer: what its code was found to name of the provider.
    let consumer_target = targets.iter().find(|target| target.service_id == from);
    let consumer_loaded = index_of(from);
    let consumer_is_snapshot = consumer_loaded.as_ref().is_some_and(|(_, snap)| *snap);
    let consumer_index = consumer_loaded.map(|(idx, _)| idx);
    let mut certain_refs = 0usize;
    let mut guessed_refs = 0usize;
    let mut consumer: BTreeMap<String, (BTreeSet<String>, Value)> = BTreeMap::new();
    for found in link["code_refs"].as_array().into_iter().flatten() {
        let named = found["contract"].as_str();
        if let (Some(asked), Some(named)) = (contract, named)
            && asked != named
        {
            continue;
        }
        let Some(path) = found["from_path"].as_str() else {
            continue;
        };
        if matches!(found["certainty"].as_str(), Some("INFERRED" | "guessed")) {
            guessed_refs += 1;
        } else {
            certain_refs += 1;
        }
        let entry = consumer.entry(path.to_owned()).or_insert_with(|| {
            (
                BTreeSet::new(),
                json!({"detail": found["detail"], "certainty": found["certainty"]}),
            )
        });
        if let Some(sym) = found["from_symbol"].as_str().filter(|s| !s.is_empty()) {
            let list = entry
                .1
                .as_object_mut()
                .expect("consumer row object")
                .entry("from_symbols")
                .or_insert_with(|| json!([]));
            if let Some(arr) = list.as_array_mut()
                && !arr.iter().any(|v| v.as_str() == Some(sym))
            {
                arr.push(json!(sym));
            }
        }
        if let Some(named) = named {
            entry.0.insert(named.to_owned());
        }
    }
    // The provider: where its passport says each contract is made, and
    // what of its code names the contract.
    let provider_target = targets.iter().find(|target| target.service_id == to);
    let provider_loaded = index_of(to);
    let provider_is_snapshot = provider_loaded.as_ref().is_some_and(|(_, snap)| *snap);
    let provider_index = provider_loaded.map(|(idx, _)| idx);
    let provides = system
        .services
        .get(to)
        .and_then(|service| service.manifest.as_ref())
        .and_then(|manifest| manifest["provides"].as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|prov| {
            prov["id"]
                .as_str()
                .is_some_and(|id| ids.iter().any(|asked| asked == id))
        })
        .collect::<Vec<_>>();
    let mut provider: BTreeMap<String, (BTreeSet<String>, Value)> = BTreeMap::new();
    for prov in &provides {
        let id = prov["id"].as_str().unwrap_or("");
        for source in prov["sources"].as_array().into_iter().flatten() {
            if let Some(path) = source.as_str().filter(|path| !path.is_empty()) {
                let entry = provider.entry(path.to_owned()).or_insert_with(|| {
                    (BTreeSet::new(), json!({"declared": true, "found": false}))
                });
                entry.0.insert(id.to_owned());
                entry.1["declared"] = json!(true);
            }
        }
    }
    let probes = provides
        .iter()
        .map(|prov| {
            (
                prov["id"].as_str().unwrap_or("").to_owned(),
                contract_probes(prov),
            )
        })
        .filter(|(_, probes)| !probes.is_empty())
        .collect::<Vec<_>>();
    let provider_state_dir = (!provider_is_snapshot).then(|| graphs.state_dir());
    let consumer_state_dir = (!consumer_is_snapshot).then(|| graphs.state_dir());
    let provider_ir = ensure_ir_store_for_system(Some(graphs), provider_state_dir, provider_target);
    let ir_handles_probes = provider_ir
        .as_ref()
        .is_some_and(|ir| ir.has_symbol_refs_index().unwrap_or(false));
    if !probes.is_empty()
        && ir_handles_probes
        && let Some(ref ir) = provider_ir
    {
        let flat_probes: Vec<String> = probes
            .iter()
            .flat_map(|(_, ps)| ps.iter().cloned())
            .collect();
        let hits = find_ir_probe_matches(ir, &flat_probes);
        for (rel_path, matched_probe) in hits {
            for (id, contract_ps) in &probes {
                if contract_ps.iter().any(|p| p == &matched_probe) {
                    let entry = provider.entry(rel_path.clone()).or_insert_with(|| {
                        (BTreeSet::new(), json!({"declared": false, "found": true}))
                    });
                    entry.0.insert(id.clone());
                    entry.1["found"] = json!(true);
                }
            }
        }
    }
    let need_disk_probes = !probes.is_empty() && !ir_handles_probes;
    let (provider_semantics, files_read, raw_cut) = collect_service_semantics_with_graphs(
        Some(graphs),
        provider_state_dir,
        provider_target,
        provider_index.as_ref(),
        certain,
        need_disk_probes,
        |rel_path, content| {
            if need_disk_probes {
                for (id, probes) in &probes {
                    if probes.iter().any(|probe| content.contains(probe.as_str())) {
                        let entry = provider.entry(rel_path.to_owned()).or_insert_with(|| {
                            (BTreeSet::new(), json!({"declared": false, "found": true}))
                        });
                        entry.0.insert(id.clone());
                        entry.1["found"] = json!(true);
                    }
                }
            }
        },
    );
    let scanned = if need_disk_probes { files_read } else { 0 };
    let cut = need_disk_probes && raw_cut;
    let (consumer_semantics, _, _) = collect_service_semantics_with_graphs(
        Some(graphs),
        consumer_state_dir,
        consumer_target,
        consumer_index.as_ref(),
        certain,
        false,
        |_, _| {},
    );
    let semantic_bridges =
        build_semantic_bridges(from, to, &consumer_semantics, &provider_semantics);
    let declared_sites = provider
        .values()
        .filter(|(_, row)| row["declared"] == true)
        .count();
    let discovered_sites = provider
        .values()
        .filter(|(_, row)| row["found"] == true && row["declared"] != true)
        .count();
    let certain_total = certain_refs + declared_sites;
    let total_confidence = certain_total + guessed_refs + discovered_sites;
    let confidence_ratio: Option<f64> = if total_confidence == 0 {
        None
    } else {
        Some(((certain_total as f64 / total_confidence as f64) * 1000.0).round() / 1000.0)
    };
    let confidence_status = if total_confidence == 0 {
        "unverified"
    } else if guessed_refs + discovered_sites == 0 {
        "verified"
    } else {
        "mixed"
    };
    let (consumer_sites, consumer_more, consumer_opened, consumer_total) =
        listed(from, consumer, &consumer_index);
    let (provider_sites, provider_more, provider_opened, provider_total) =
        listed(to, provider, &provider_index);
    let mut out = json!({
        "kind": "system_bridge",
        "from": from,
        "to": to,
        "direction": link["direction"],
        "contracts": ids,
        "contract": contract,
        "certain": certain,
        "confidence": {
            "certain_refs": certain_refs,
            "guessed_refs": guessed_refs,
            "declared_sites": declared_sites,
            "discovered_sites": discovered_sites,
            "total": total_confidence,
            "status": confidence_status,
            "ratio": confidence_ratio,
        },
        "consumer": {
            "service": from,
            "graph": consumer_index.is_some(),
            "sites": consumer_sites,
            "more": consumer_more,
        },
        "provider": {
            "service": to,
            "graph": provider_index.is_some(),
            "sites": provider_sites,
            "more": provider_more,
            "scanned": scanned,
            "cut": cut,
        },
        "semantic_bridges": semantic_bridges,
    });
    if let Some(ts) = ctx.target_spec {
        out["target"] = json!(ts);
    }
    if let Some(rid) = ctx.run_id {
        out["run_id"] = json!(rid);
        out["run_coverage"] = json!({
            "run_id": rid,
            "consumer_opened": consumer_opened,
            "consumer_total": consumer_total,
            "provider_opened": provider_opened,
            "provider_total": provider_total,
        });
    }
    out
}

#[cfg(test)]
mod semantic_bridge_tests {
    use super::super::{HarnessCodeGraphBridge, SystemGraphDomain, resolve_service_target};
    use super::*;
    use crate::{
        graph::publish_for_tests,
        services::{Domain, Payload},
        store::SessionStore,
    };
    use tempfile::TempDir;

    fn make_service(root: &Path, id: &str, provides: Value, consumes: Value, sources: Value) {
        fs::create_dir_all(root).unwrap();
        let passport = json!({
            "schema_version": 1,
            "id": id,
            "description": format!("{id} service"),
            "owner": "platform",
            "relationships_complete": true,
            "capabilities": [],
            "provides": provides,
            "consumes": consumes,
            "sources": sources,
        });
        fs::write(
            root.join("ai-service.json"),
            serde_json::to_vec_pretty(&passport).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn bridge_view_discovers_cross_service_routes_events_di_and_table_couplings() {
        let tmp = TempDir::new().unwrap();
        let state_dir = tmp.path().join("state");
        let store = SessionStore::open(&state_dir).unwrap();
        let mono_root = tmp.path().join("mono");
        let web_root = mono_root.join("services/web");
        let orders_root = mono_root.join("services/orders");
        fs::create_dir_all(web_root.join("src")).unwrap();
        fs::create_dir_all(orders_root.join("src")).unwrap();

        // Consumer (`web`):
        // - Calls GET/fetch on `/api/v1/orders/42` (matches provider's `/api/v1/orders/{id}`)
        // - Handles `OrderPaid` event & publishes `checkout.requested`
        // - Injects `BillingToken` DI binding
        // - Reads `orders` table (cross_read_write), writes `payments` (shared_write), reads `currencies` (shared_read)
        fs::write(
            web_root.join("src/OrderClient.ts"),
            r#"
import { Inject } from '@nestjs/common';

export class OrderClient {
  constructor(@Inject('BillingToken') private readonly token: string) {}
  async fetchOrder() {
    return fetch('http://orders/api/v1/orders/42');
  }
  async startCheckout() {
    emit('checkout.requested', { id: 1 });
  }
}
"#,
        )
        .unwrap();
        fs::write(
            web_root.join("src/OrderMirrorWorker.php"),
            r#"<?php
class OrderMirrorWorker {
    #[AsMessageHandler]
    public function onPaid(OrderPaid $event): void {
        $rows = DB::table('orders')->where('id', 1)->get();
        DB::table('payments')->insert(['order_id' => 1]);
        $cur = "SELECT code FROM currencies WHERE active = 1";
    }
}
"#,
        )
        .unwrap();

        // Provider (`orders`):
        // - Exposes Laravel/Symfony `/api/v1/orders/{id}` and Express `/api/v1/checkout`
        // - Publishes `OrderPaid` and subscribes to `checkout.requested`
        // - Binds `BillingToken` via `provide: 'BillingToken'`
        // - Writes `orders` and `payments`, reads `currencies`
        fs::write(
            orders_root.join("src/OrderController.php"),
            r#"<?php
Route::get('/api/v1/orders/{id}', [OrderController::class, 'show']);

class OrderController {
    public function store() {
        $sql = "INSERT INTO orders (total) VALUES (100)";
        $pay = "UPDATE payments SET status = 'paid' WHERE id = 1";
        $cur = "SELECT code FROM currencies";
        event(new \App\Events\OrderPaid());
    }
}
"#,
        )
        .unwrap();
        fs::write(
            orders_root.join("src/CheckoutConsumer.ts"),
            r#"
export const billingProvider = { provide: 'BillingToken', useValue: 'x' };

@OnEvent('checkout.requested')
export class CheckoutConsumer {
  handle() {}
}
"#,
        )
        .unwrap();

        make_service(
            &orders_root,
            "orders",
            json!([{
                "id": "orders-http-v1",
                "kind": "http",
                "version": "1.0.0",
                "sources": ["src/OrderController.php"]
            }]),
            json!([]),
            json!([
                {"path": "src/OrderController.php", "kind": "contract"},
                {"path": "src/CheckoutConsumer.ts", "kind": "code"}
            ]),
        );
        make_service(
            &web_root,
            "web",
            json!([]),
            json!([{
                "service": "orders",
                "contract": "orders-http-v1",
                "version": "1.0.0"
            }]),
            json!([
                {"path": "src/OrderClient.ts", "kind": "code"},
                {"path": "src/OrderMirrorWorker.php", "kind": "code"}
            ]),
        );

        let sys_cfg = json!({
            "schema_version": 1,
            "name": "semantic-shop",
            "shared_sources": [],
            "services": [
                {"id": "web", "root": "services/web", "manifest": "ai-service.json"},
                {"id": "orders", "root": "services/orders", "manifest": "ai-service.json"}
            ]
        });
        fs::write(
            mono_root.join("system.json"),
            serde_json::to_vec_pretty(&sys_cfg).unwrap(),
        )
        .unwrap();

        let pid = store.register(&mono_root).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let fake_graphify = tmp.path().join("fake-graphify");
        fs::write(&fake_graphify, "#!/bin/sh\nexit 0\n").unwrap();
        let graphs = Graphs::with(
            store.clone(),
            Some(fake_graphify.clone()),
            std::time::Duration::from_secs(30),
        );
        let bridge = HarnessCodeGraphBridge::new(store.clone(), graphs.clone());
        let domain = SystemGraphDomain::new(store.clone(), graphs.clone(), bridge, None);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let payload = rt
            .block_on(domain.execute(
                "system_graph.bridge",
                json!({"query": {"project_id": pid, "from": "web", "to": "orders"}, "body": {}}),
            ))
            .unwrap();
        let Payload::Json(view) = payload else {
            panic!("expected json payload");
        };

        // Legacy fields preserved.
        assert_eq!(view["kind"], "system_bridge");
        assert_eq!(view["from"], "web");
        assert_eq!(view["to"], "orders");
        assert_eq!(view["contracts"], json!(["orders-http-v1"]));
        assert_eq!(
            view["provider"]["sites"][0]["path"],
            "src/OrderController.php"
        );
        assert!(view["consumer"].is_object());

        let sem = &view["semantic_bridges"];
        assert_eq!(sem["summary"]["routes"], 1, "{sem}");
        assert_eq!(sem["routes"][0]["route"], "/api/v1/orders/{param}");
        assert_eq!(sem["routes"][0]["method"], "GET");
        assert_eq!(sem["routes"][0]["direction"], "from->to");
        assert_eq!(
            sem["routes"][0]["caller_files"],
            json!(["src/OrderClient.ts"])
        );
        assert_eq!(
            sem["routes"][0]["provider_files"],
            json!(["src/OrderController.php"])
        );

        let events = sem["events"].as_array().unwrap();
        assert_eq!(events.len(), 2, "{sem}");
        assert!(events.iter().any(|e| {
            e["event"] == "checkout.requested"
                && e["direction"] == "from->to"
                && e["producer"] == "web"
                && e["consumer"] == "orders"
        }));
        assert!(events.iter().any(|e| {
            e["event"] == "OrderPaid"
                && e["direction"] == "to->from"
                && e["producer"] == "orders"
                && e["consumer"] == "web"
        }));

        let di = sem["di_bindings"].as_array().unwrap();
        assert_eq!(di.len(), 1, "{sem}");
        assert_eq!(di[0]["service"], "BillingToken");
        assert_eq!(di[0]["from_kinds"], json!(["injects"]));
        assert_eq!(di[0]["to_kinds"], json!(["binds"]));

        let tables = sem["tables"].as_array().unwrap();
        assert_eq!(tables.len(), 3, "{sem}");
        let by_table = |name: &str| {
            tables
                .iter()
                .find(|t| t["table"] == name)
                .unwrap_or_else(|| panic!("missing table {name}: {sem}"))
        };
        assert_eq!(by_table("orders")["coupling"], "cross_read_write");
        assert_eq!(by_table("orders")["from_access"], json!(["read"]));
        assert_eq!(by_table("orders")["to_access"], json!(["write"]));
        assert_eq!(by_table("payments")["coupling"], "shared_write");
        assert_eq!(by_table("currencies")["coupling"], "shared_read");

        // Also verify graph-level semantic edges integrate alongside source scans.
        let (_, orders_t) =
            resolve_service_target(&store, Some(&graphs), None, &pid, None, "orders").unwrap();
        publish_for_tests(&store, &orders_t.graph_id, &fake_graphify);
        let graph_file = store
            .state
            .join("graphs")
            .join(&orders_t.graph_id)
            .join("live")
            .join("graph.json");
        fs::create_dir_all(graph_file.parent().unwrap()).unwrap();
        let doc = json!({
            "directed": true,
            "multigraph": false,
            "graph": {},
            "nodes": [
                {"id": "ctrl", "label": "OrderController", "source_file": "src/OrderController.php", "source_location": "L1"},
                {"id": "rt", "label": "route:POST /api/v1/checkout", "source_file": "src/OrderController.php", "source_location": "L2"}
            ],
            "links": [
                {"source": "ctrl", "target": "rt", "relation": "exposes_route", "confidence": "EXTRACTED"}
            ]
        });
        fs::write(&graph_file, serde_json::to_vec(&doc).unwrap()).unwrap();
        fs::write(
            web_root.join("src/CheckoutCaller.ts"),
            "export const go = () => axios.post('/api/v1/checkout');\n",
        )
        .unwrap();

        let payload2 = rt
            .block_on(domain.execute(
                "system_graph.bridge",
                json!({"query": {"project_id": pid, "from": "web", "to": "orders"}, "body": {}}),
            ))
            .unwrap();
        let Payload::Json(view2) = payload2 else {
            panic!("expected json payload");
        };
        let routes2 = view2["semantic_bridges"]["routes"].as_array().unwrap();
        assert!(
            routes2
                .iter()
                .any(|r| r["route"] == "/api/v1/checkout" && r["method"] == "POST"),
            "{routes2:?}"
        );
    }

    #[test]
    fn read_bounded_service_file_rejects_traversal_and_outside_symlinks() {
        let tmp = TempDir::new().unwrap();
        let svc = tmp.path().join("svc");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(svc.join("src")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(svc.join("src/ok.php"), "<?php echo 1;\n").unwrap();
        let secret = outside.join("secret.php");
        fs::write(&secret, "<?php // secret\n").unwrap();
        let canon = infra_core::fs::canonicalize(&svc).unwrap();

        assert!(read_bounded_service_file(&svc, Some(&canon), "../outside/secret.php").is_none());
        assert!(read_bounded_service_file(&svc, Some(&canon), "/etc/passwd").is_none());
        assert!(read_bounded_service_file(&svc, Some(&canon), "src/ok.php").is_some());

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&secret, svc.join("src/escape.php")).unwrap();
            assert!(read_bounded_service_file(&svc, Some(&canon), "src/escape.php").is_none());
        }
    }

    #[test]
    fn collect_service_semantics_reads_ir_store_first_without_disk_scan() {
        let tmp = TempDir::new().unwrap();
        let state_dir = tmp.path().join("state");
        let svc_root = tmp.path().join("svc");
        fs::create_dir_all(svc_root.join("src")).unwrap();
        fs::write(
            svc_root.join("src/Client.ts"),
            "export function run() {\n  emit('checkout.requested', { id: 1 });\n  return fetch('/api/v1/orders/42');\n}\n",
        )
        .unwrap();

        let ir = crate::graph_ir::GraphIrStore::open(&state_dir, "svc-graph").unwrap();
        ir.sync_workspace(&svc_root).unwrap();
        // Replace on-disk file with an empty stub without re-syncing IR so only GraphIrStore has the signals.
        fs::write(svc_root.join("src/Client.ts"), "// stub on disk\n").unwrap();

        let target = ResolvedServiceTarget {
            project_id: "mono".to_owned(),
            config_path: "system.json".to_owned(),
            service_id: "web".to_owned(),
            graph_id: "svc-graph".to_owned(),
            root: svc_root,
            name: "web".to_owned(),
            reused_project: false,
        };

        let (signals, files_read, cut) = collect_service_semantics_with(
            Some(&state_dir),
            Some(&target),
            None,
            true,
            false,
            |_, _| {},
        );
        assert_eq!(files_read, 0);
        assert!(!cut);
        assert!(
            signals
                .called_routes
                .values()
                .any(|site| site.methods.contains("GET")),
            "expected GET route from IR: {:?}",
            signals.called_routes
        );
        assert!(
            signals.published_events.contains_key("checkout.requested"),
            "expected event from IR: {:?}",
            signals.published_events
        );
    }
}
