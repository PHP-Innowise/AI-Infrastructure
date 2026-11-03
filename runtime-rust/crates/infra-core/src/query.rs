//! FTS5 query construction, identifier expansion and deterministic ranking.
use crate::{
    Result, RuntimeConfig, RuntimeContext, RuntimeError,
    index::{DOCUMENT_SELECT, Document, document_from_row},
    storage::{Episode, Store},
};
use rusqlite::{Connection, params, params_from_iter};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const BM25_WEIGHTS: [f64; 6] = [1.0, 1.0, 1.0, 2.0, 8.0, 1.0];
pub const QUERY_EXPANSION_LIMIT: usize = 48;
pub const IDENTIFIER_ROW_LIMIT: usize = 4096;
pub const RELATIVE_SCORE_FLOOR: f64 = 0.3;

pub fn sha256(bytes: &[u8]) -> String {
    crate::domain::hex_lower(Sha256::digest(bytes))
}
pub fn chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}
pub fn estimated_tokens(value: &str) -> usize {
    value.chars().count().div_ceil(4).max(1)
}
pub fn folded(value: &str) -> String {
    crate::unicode_fold::fold(value)
}
fn expression(pattern: &str) -> Result<regex::Regex> {
    regex::Regex::new(pattern).map_err(|_| RuntimeError::invalid("Invalid query expression"))
}
pub fn query_tokens(query: &str) -> Result<Vec<String>> {
    let words = expression(r"[\p{L}\p{N}_]+")?;
    let mut tokens = Vec::new();
    let mut seen = BTreeSet::new();
    for word in words.find_iter(query) {
        let word = word.as_str();
        if seen.insert(folded(word)) {
            tokens.push(word.into());
        }
        let folded_word = folded(word);
        for (pattern, alias) in [
            (r"предотвращ\w*", "prevent"),
            (r"подделк\w*", "spoof"),
            (r"вложенн\w*", "nested"),
            (r"неизвестн\w*", "unknown"),
            (r"записыва\w*", "written"),
            (r"отбрасыва\w*", "drop"),
            (r"просмотр\w*", "preview"),
            (r"временн\w*", "temporary"),
            (r"действительн\w*", "validity"),
            (r"измен\w*", "change"),
            (r"админк\w*", "admin"),
            (r"стар(?:ый|ая|ое|ые|ого|ой|ому|ым|ых|ыми|ую|ом)", "legacy"),
            (r"файл\w*", "files"),
            (r"свойств\w*", "properties"),
            (r"сущност\w*", "entity"),
            (r"строк(?:а|и|у|ой|ою|е|ам|ами|ах)?", "lines"),
            (r"спис(?:ок|ка|ку|ком|ке|ки|ков|кам|ками|ках)", "list"),
            (r"пут(?:ь|и|ем|ём|ям|ями|ях)", "path"),
            (r"лог(?:и|а|ов|е|у|ом|ам|ами|ах)?|логир\w*|логгер\w*", "log"),
            (r"пол(?:е|я|ей|ю|ям|ем|ями|ях)", "field"),
            (r"им(?:я|ени|енем|ена|ён|ен|енам|енами|енах)", "name"),
            (r"срок\w*", "ttl"),
        ] {
            if alias == "validity" && folded_word == "действительно" {
                continue;
            }
            if expression(&format!("^(?:{pattern})$"))?.is_match(&folded_word)
                && seen.insert(alias.into())
            {
                tokens.push(alias.into());
            }
        }
    }
    if tokens.is_empty() {
        return Err(RuntimeError::invalid("Search query must contain a word"));
    }
    Ok(tokens)
}
pub fn fts_expression(tokens: &[String]) -> String {
    tokens
        .iter()
        .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ")
}
pub fn identifier_terms(path: &str, content: &str) -> Vec<(String, String)> {
    let _ = path;
    let Ok(words) = regex::Regex::new(r"[A-Za-z_\\][A-Za-z0-9_\\]*(?:\.[A-Za-z0-9_]+)*") else {
        return Vec::new();
    };
    let text = identifier_text(content);
    let mut output = Vec::new();
    let mut seen = BTreeSet::new();
    for found in words.find_iter(&text) {
        let stem = identifier_stem(found.as_str());
        if let Some(expansion) = identifier_form(&stem) {
            let row = (stem, expansion);
            if seen.insert(row.clone()) {
                output.push(row);
            }
            if output.len() >= IDENTIFIER_ROW_LIMIT {
                break;
            }
        }
    }
    output
}
pub fn identifier_stem(value: &str) -> String {
    let lower = folded(value);
    for suffix in [
        ".php", ".py", ".md", ".json", ".js", ".mjs", ".cjs", ".ts", ".tsx", ".jsx", ".java",
        ".kt", ".rb", ".go", ".rs", ".c", ".h", ".cpp", ".hpp", ".cs", ".yaml", ".yml", ".xml",
        ".sql", ".sh", ".vue", ".svelte", ".css", ".scss",
    ] {
        if lower.ends_with(suffix) && value.len() > suffix.len() {
            return value[..value.len() - suffix.len()].into();
        }
    }
    value.into()
}
pub fn identifier_parts(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    for segment in value.split(['_', '\\', '.']) {
        let bytes = segment.as_bytes();
        let mut at = 0;
        while at < bytes.len() {
            let begin = at;
            if bytes[at].is_ascii_digit() {
                while at < bytes.len() && bytes[at].is_ascii_digit() {
                    at += 1;
                }
            } else if bytes[at].is_ascii_uppercase() {
                while at < bytes.len() && bytes[at].is_ascii_uppercase() {
                    at += 1;
                }
                if at < bytes.len() && bytes[at].is_ascii_lowercase() {
                    if at > begin + 1 {
                        at -= 1;
                    } else {
                        while at < bytes.len() && bytes[at].is_ascii_lowercase() {
                            at += 1;
                        }
                    }
                }
            } else if bytes[at].is_ascii_lowercase() {
                while at < bytes.len() && bytes[at].is_ascii_lowercase() {
                    at += 1;
                }
            } else {
                at += 1;
                continue;
            }
            parts.push(folded(&segment[begin..at]));
        }
    }
    parts
}
pub fn identifier_form(value: &str) -> Option<String> {
    if !(4..=200).contains(&value.chars().count()) {
        return None;
    }
    let boundary = value.contains(['_', '\\', '.'])
        || value
            .as_bytes()
            .windows(2)
            .any(|pair| pair[0].is_ascii_lowercase() && pair[1].is_ascii_uppercase())
        || value.as_bytes().windows(3).any(|pair| {
            pair[0].is_ascii_uppercase()
                && pair[1].is_ascii_uppercase()
                && pair[2].is_ascii_lowercase()
        });
    if !boundary {
        return None;
    }
    let parts = identifier_parts(value);
    if parts.len() < 2 {
        return None;
    }
    let expansion = parts.join(" ");
    if expansion == folded(value) {
        None
    } else {
        Some(expansion)
    }
}
pub fn identifier_text(content: &str) -> String {
    let body = content
        .strip_prefix("---\n")
        .and_then(|tail| tail.split_once("\n---\n").map(|(_, body)| body))
        .unwrap_or(content);
    let mut output = Vec::new();
    let mut omitted = false;
    for line in body.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            omitted = ["sources", "references", "files"].contains(&folded(heading.trim()).as_str());
        }
        if !omitted {
            output.push(line);
        }
    }
    output.join("\n")
}
pub fn expand_query_tokens(conn: &Connection, tokens: Vec<String>) -> Result<Vec<String>> {
    let mut expanded = tokens.clone();
    let limit = tokens.len() + QUERY_EXPANSION_LIMIT;
    let mut seen = expanded
        .iter()
        .map(|value| folded(value))
        .collect::<BTreeSet<_>>();
    let mut grams = Vec::new();
    'outer: for size in (2..=tokens.len().min(6)).rev() {
        for start in 0..=tokens.len() - size {
            grams.push(
                tokens[start..start + size]
                    .iter()
                    .map(|token| folded(token))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            if grams.len() >= 64 {
                break 'outer;
            }
        }
    }
    if grams.is_empty() {
        return Ok(expanded);
    }
    let sql = format!(
        "SELECT DISTINCT identifier FROM identifier_terms WHERE expansion IN ({})",
        vec!["?"; grams.len()].join(",")
    );
    if let Ok(mut statement) = conn.prepare(&sql)
        && let Ok(rows) = statement.query_map(params_from_iter(grams.iter()), |row| {
            row.get::<_, String>(0)
        })
    {
        for identifier in rows {
            let identifier = identifier?;
            if !identifier.is_empty() && seen.insert(folded(&identifier)) && expanded.len() < limit
            {
                expanded.push(identifier);
            }
        }
    }
    Ok(expanded)
}
pub fn frequencies(conn: &Connection, tokens: &[String]) -> Result<BTreeMap<String, usize>> {
    let mut output = BTreeMap::new();
    for token in tokens {
        let count = conn.query_row(
            "SELECT COUNT(*) FROM documents WHERE documents MATCH ?1",
            [fts_expression(std::slice::from_ref(token))],
            |row| row.get::<_, usize>(0),
        )?;
        output.insert(token.clone(), count);
    }
    Ok(output)
}
pub fn informative_tokens(conn: &Connection, tokens: Vec<String>) -> Result<Vec<String>> {
    let total = conn.query_row("SELECT COUNT(*) FROM documents", [], |row| {
        row.get::<_, usize>(0)
    })?;
    if total == 0 {
        return Ok(tokens);
    }
    let counts = frequencies(conn, &tokens)?;
    let matched = tokens
        .iter()
        .filter(|token| counts.get(*token).copied().unwrap_or(0) > 0)
        .cloned()
        .collect::<Vec<_>>();
    let informative = matched
        .iter()
        .filter(|token| counts.get(*token).copied().unwrap_or(0) as f64 <= total as f64 * 0.5)
        .cloned()
        .collect::<Vec<_>>();
    Ok(if !informative.is_empty() {
        informative
    } else if !matched.is_empty() {
        matched
    } else {
        tokens
    })
}
pub fn evidence_tokens(tokens: &[String]) -> Vec<String> {
    const STOP: &str = "a about above after again against all am an and any are aren as at be because been before being below between both but by can cannot could couldn did didn do does doesn doing don down during each few for from further had hadn has hasn have haven having he her here hers herself him himself his how i if in into is isn it its itself just let me more most mustn my myself no nor not now of off on once only or other ought our ours ourselves out over own same shan she should shouldn so some such than that the their theirs them themselves then there these they this those through to too under until up very was wasn we were weren what when where which while who whom why will with won would wouldn you your yours yourself yourselves s t ll re ve d m o y also anything everything something someone anyone please thanks thank ok okay yes yeah sure get got make made want need like one two way thing things still again really и в во не что он на я с со как а то все она так его но да ты к у же вы за бы по только ее мне было вот от меня еще нет о из ему теперь когда даже ну вдруг ли если уже или ни быть был него до вас нибудь опять уж вам ведь там потом себя ничего ей может они тут где есть надо ней для мы тебя их чем была сам чтоб без будто чего раз тоже себе под будет ж тогда кто этот того потому этого какой совсем ним здесь этом один почти мой тем чтобы нее сейчас были куда зачем всех никогда можно при наконец два об другой хоть после над больше тот через эти нас про всего них какая много разве три эту моя впрочем хорошо свою этой перед иногда лучше чуть том нельзя такой им более всегда конечно всю между это эта мои давай пожалуйста спасибо ок сделай сделать нужно какие good great nice fine cool perfect awesome excellent done look looks looking seems seem works working continue proceed go ahead lets right correct thx cheers hi hello hey bye agreed approve approved lgtm отлично супер класс готово продолжай продолжи продолжить дальше норм нормально верно понятно ясно ага угу привет пока согласен";
    tokens
        .iter()
        .filter(|token| !STOP.split_whitespace().any(|stop| stop == folded(token)))
        .cloned()
        .collect()
}
pub fn build_capsule_query(query: &str, working: Option<&Value>) -> Result<String> {
    let mut text = query.to_string();
    if let Some(working) = working {
        for field in ["goal", "progress"] {
            if let Some(value) = working[field].as_str() {
                text.push(' ');
                text.push_str(value);
            }
        }
        for field in ["next_steps", "files"] {
            for value in working[field]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                text.push(' ');
                text.push_str(value);
            }
        }
    }
    Ok(query_tokens(&text)?
        .into_iter()
        .take(32)
        .collect::<Vec<_>>()
        .join(" "))
}
pub fn distill_query(conn: &Connection, prompt: &str) -> Result<String> {
    let tokens = informative_tokens(conn, expand_query_tokens(conn, query_tokens(prompt)?)?)?;
    let counts = frequencies(conn, &tokens)?;
    let mut ranked = tokens.into_iter().enumerate().collect::<Vec<_>>();
    ranked.sort_by_key(|(index, token)| (counts.get(token).copied().unwrap_or(0), *index));
    ranked.truncate(24);
    ranked.sort_by_key(|(index, _)| *index);
    Ok(ranked
        .into_iter()
        .map(|(_, token)| token)
        .collect::<Vec<_>>()
        .join(" "))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchRequest {
    pub query: String,
    pub limit: usize,
    pub layer: Option<String>,
}
impl Default for SearchRequest {
    fn default() -> Self {
        Self {
            query: String::new(),
            limit: 8,
            layer: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchItem {
    pub path: String,
    pub layer: String,
    pub kind: String,
    pub title: String,
    pub snippet: String,
}
#[derive(Clone, Debug)]
pub struct Candidate {
    pub document: Document,
    pub score: f64,
    pub adjusted_score: f64,
    pub rank: Option<usize>,
    pub match_strength: Option<String>,
    pub selection: Option<String>,
    pub via: Option<String>,
    pub hint_raw_sha256: Option<String>,
    pub held_image: Option<crate::excerpts::IndexedImage>,
    pub held_source_hash: Option<String>,
    pub source_changed: Vec<String>,
    pub snippet: String,
    pub estimated_tokens: usize,
    pub delivery_identity: Option<(String, String)>,
}
impl Candidate {
    pub fn linked(document: Document, selection: &str) -> Self {
        let snippet = body_excerpt(&document.content, 320, &[], document.kind == "changelog");
        let cost = estimated_tokens(&format!("{}{snippet}", document.title));
        Self {
            document,
            score: 0.0,
            adjusted_score: 0.0,
            rank: None,
            match_strength: (selection == "conflict").then(|| "conflict".into()),
            selection: (!["", "history", "conflict"].contains(&selection))
                .then(|| selection.into()),
            via: None,
            hint_raw_sha256: None,
            held_image: None,
            held_source_hash: None,
            source_changed: vec![],
            snippet,
            estimated_tokens: cost,
            delivery_identity: None,
        }
    }
    pub fn image(&self) -> crate::excerpts::IndexedImage {
        self.held_image
            .clone()
            .unwrap_or_else(|| crate::excerpts::IndexedImage {
                title: self.document.title.clone(),
                summary: self.document.summary.clone(),
                content: self.document.content.clone(),
            })
    }
    pub fn public(&self) -> Value {
        self.public_visible(None)
    }
    pub fn public_visible(&self, visible: Option<&BTreeSet<String>>) -> Value {
        let conflicts = self
            .document
            .conflicts
            .iter()
            .filter(|id| visible.is_some_and(|visible| visible.contains(*id)))
            .collect::<Vec<_>>();
        let mut value = json!({"path":self.document.path,"layer":self.document.layer,"kind":self.document.kind,"title":self.document.title,
            "snippet":self.snippet,"category":self.document.category,"estimated_tokens":self.estimated_tokens,"record_id":self.document.record_id,"conflicts":conflicts});
        if !self.document.conflicts.is_empty() || self.match_strength.as_deref() == Some("conflict")
        {
            value["conflict_recorded"] = true.into();
        }
        if let Some(strength) = self
            .match_strength
            .as_deref()
            .filter(|strength| *strength != "covered")
        {
            value["match"] = strength.into();
        }
        if let Some(selection) = self
            .selection
            .as_deref()
            .filter(|selection| !selection.is_empty())
        {
            value["selection"] = selection.into();
        }
        if !self.source_changed.is_empty() {
            value["source_changed"] = json!(self.source_changed);
        }
        if self.document.attestation == "agent" {
            value["attestation"] = "agent".into();
        }
        if let Some(via) = self.via.as_deref().filter(|via| !via.is_empty()) {
            value["via"] = via.into();
        }
        value
    }
}
#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    pub informative_terms: usize,
    pub distinctive_matches: usize,
    pub top_score: Option<f64>,
    pub coverage: BTreeMap<String, usize>,
}

pub fn search(
    store: &Store,
    context: &RuntimeContext,
    request: &SearchRequest,
) -> Result<Vec<SearchItem>> {
    if request.limit == 0 {
        return Err(RuntimeError::invalid("--limit must be a positive integer"));
    }
    if request
        .layer
        .as_deref()
        .is_some_and(|layer| !["procedural", "semantic", "episodic"].contains(&layer))
    {
        return Err(RuntimeError::invalid("Invalid document layer"));
    }
    let tokens = expand_query_tokens(&store.conn, query_tokens(&request.query)?)?;
    let match_expression = fts_expression(&tokens);
    let sql = format!(
        "SELECT {DOCUMENT_SELECT},snippet(documents,5,'[',']',' … ',18) AS quoted FROM documents d JOIN document_metadata m ON m.path=d.path WHERE documents MATCH ?1 AND (?2 IS NULL OR d.layer=?2) ORDER BY bm25(documents),d.path LIMIT ?3"
    );
    let mut statement = store.conn.prepare(&sql)?;
    let rows = statement.query_map(
        params![
            match_expression,
            request.layer,
            request.limit.saturating_mul(10)
        ],
        |row| Ok((document_from_row(row)?, row.get::<_, String>("quoted")?)),
    )?;
    let config = RuntimeConfig::load(&context.roots)?;
    let mut output = Vec::new();
    for row in rows {
        let (document, snippet) = row?;
        if crate::retrieval::admit_document(context, &config, &document)?.is_none() {
            output.push(SearchItem {
                path: document.path,
                layer: document.layer,
                kind: document.kind,
                title: document.title,
                snippet,
            });
            if output.len() >= request.limit {
                break;
            }
        }
    }
    Ok(output)
}
pub fn ranking_weight(authority: &str, confidence: f64, updated_at: &str) -> f64 {
    let mut weight = if authority == "verified" { 1.0 } else { 0.85 };
    let confidence = if confidence.is_finite() {
        confidence.clamp(0.0, 1.0)
    } else {
        1.0
    };
    weight *= 0.7 + 0.3 * confidence;
    if !updated_at.is_empty()
        && let Ok(at) = chrono::DateTime::parse_from_rfc3339(updated_at)
    {
        let age = (crate::domain::clock::now() - at.with_timezone(&chrono::Utc))
            .num_milliseconds()
            .max(0) as f64
            / 86_400_000.0;
        weight *= 0.5 + 0.5 * 0.5_f64.powf(age / 30.0);
    }
    weight
}
pub fn rank_candidates(
    conn: &Connection,
    query: &str,
    limit: usize,
    scope: Option<&[String]>,
) -> Result<(Vec<Candidate>, Diagnostics)> {
    rank_candidates_observed(conn, query, limit, scope, None)
}
pub fn rank_candidates_observed(
    conn: &Connection,
    query: &str,
    limit: usize,
    scope: Option<&[String]>,
    mut trace: Option<&mut crate::observability::Trace>,
) -> Result<(Vec<Candidate>, Diagnostics)> {
    let tokens = informative_tokens(conn, expand_query_tokens(conn, query_tokens(query)?)?)?;
    let evidence = evidence_tokens(&tokens);
    if evidence.is_empty() {
        return Ok((vec![], Diagnostics::default()));
    }
    let total = conn
        .query_row("SELECT COUNT(*) FROM documents", [], |row| {
            row.get::<_, usize>(0)
        })?
        .max(1);
    let mut coverage = BTreeMap::<String, usize>::new();
    let mut rare = BTreeMap::<String, Vec<String>>::new();
    for token in &evidence {
        let mut statement = conn.prepare("SELECT path FROM documents WHERE documents MATCH ?1")?;
        let paths = statement
            .query_map([fts_expression(std::slice::from_ref(token))], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let distinctive = paths.len() as f64 <= (total as f64 * 0.1).max(1.0);
        for path in paths {
            *coverage.entry(path.clone()).or_default() += 1;
            if distinctive {
                rare.entry(path).or_default().push(token.clone());
            }
        }
    }
    let minimum = if evidence.len() >= 2 { 2 } else { 1 };
    if let Some(trace) = trace.as_deref_mut() {
        trace.coverage(evidence.len(), minimum, &coverage);
    }
    let mut arguments = vec![rusqlite::types::Value::Text(fts_expression(&tokens))];
    let restriction = if let Some(scope) = scope {
        let mut seen = BTreeSet::new();
        let scope = scope
            .iter()
            .filter(|path| seen.insert(path.as_str()))
            .take(500)
            .collect::<Vec<_>>();
        if scope.is_empty() {
            " AND 0".into()
        } else {
            arguments.extend(
                scope
                    .iter()
                    .map(|path| rusqlite::types::Value::Text((*path).clone())),
            );
            format!(" AND d.path IN ({})", vec!["?"; scope.len()].join(","))
        }
    } else {
        String::new()
    };
    arguments.push(rusqlite::types::Value::Integer(
        i64::try_from(limit).unwrap_or(i64::MAX),
    ));
    let sql = format!(
        "SELECT {DOCUMENT_SELECT},bm25(documents,1.0,1.0,1.0,2.0,8.0,1.0) AS score FROM documents d JOIN document_metadata m ON m.path=d.path WHERE documents MATCH ? AND d.kind!='brain-event'{restriction} ORDER BY score,d.path LIMIT ?"
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(arguments.iter()), |row| {
        Ok((document_from_row(row)?, row.get::<_, f64>("score")?))
    })?;
    let mut output = Vec::new();
    let mut observed_rank = 0usize;
    for row in rows {
        let (document, score) = row?;
        if let Some(trace) = trace.as_deref_mut() {
            observed_rank += usize::from(trace.eligible_document(&document));
            trace.observe_document(&document,"search","fetched",&json!({"rank":observed_rank,"matched_terms":coverage.get(&document.path).copied().unwrap_or(0),"required_terms":minimum}));
        }
        let covered = coverage.get(&document.path).copied().unwrap_or(0) >= minimum;
        let where_words = word_tokens(&format!("{} {}", document.path, document.title));
        let anchored = rare.get(&document.path).is_some_and(|terms| {
            terms.iter().any(|term| {
                document.layer == "procedural"
                    || ((frequencies(conn, std::slice::from_ref(term))
                        .ok()
                        .and_then(|values| values.get(term).copied())
                        .is_none_or(|count| count <= 3)
                        || contains_decimal_digit(term))
                        && is_anchor(term, &where_words))
            })
        });
        if !covered && !anchored {
            if let Some(trace) = trace.as_deref_mut() {
                trace.observe_document(&document, "admission", "term-coverage", &Value::Null);
            }
            continue;
        }
        let snippet = body_excerpt(
            &document.content,
            320,
            &evidence,
            document.kind == "changelog",
        );
        let cost = estimated_tokens(&format!("{}{snippet}", document.title));
        let adjusted_score = (-score).max(0.0)
            * ranking_weight(
                &document.authority,
                document.confidence,
                &document.updated_at,
            );
        let item = Candidate {
            document,
            score,
            adjusted_score,
            rank: None,
            match_strength: Some(if covered { "covered" } else { "distinctive" }.into()),
            selection: None,
            source_changed: vec![],
            snippet,
            estimated_tokens: cost,
            delivery_identity: None,
            via: None,
            hint_raw_sha256: None,
            held_image: None,
            held_source_hash: None,
        };
        if let Some(trace) = trace.as_deref_mut() {
            trace.observe(
                &item,
                "admission",
                "admitted",
                &json!({"score":adjusted_score}),
            );
        }
        output.push(item);
    }
    output.sort_by(|left, right| {
        right
            .adjusted_score
            .total_cmp(&left.adjusted_score)
            .then_with(|| left.score.total_cmp(&right.score))
            .then_with(|| left.document.path.cmp(&right.document.path))
    });
    let mut best = BTreeMap::new();
    for item in &output {
        best.entry(item.document.layer.clone())
            .or_insert(item.adjusted_score);
    }
    output.retain(|item| {
        let admitted = item.adjusted_score
            >= best.get(&item.document.layer).copied().unwrap_or(0.0) * RELATIVE_SCORE_FLOOR;
        if let Some(trace) = trace.as_deref_mut() {
            trace.observe(
                item,
                "ranking",
                if admitted {
                    "admitted"
                } else {
                    "relative-score-floor"
                },
                &json!({"score":item.adjusted_score}),
            );
        }
        admitted
    });
    let mut observed_rank = 0usize;
    for (index, item) in output.iter_mut().enumerate() {
        item.rank = Some(index + 1);
        if let Some(trace) = trace.as_deref_mut() {
            observed_rank += usize::from(trace.eligible(item));
            trace.observe(
                item,
                "ranking",
                "authorized-window-rank",
                &json!({"rank":observed_rank}),
            );
        }
        item.snippet = crate::excerpts::quote_candidate(conn, item, query, 320)?;
        item.estimated_tokens =
            estimated_tokens(&format!("{}{}", item.document.title, item.snippet));
    }
    let diagnostics = Diagnostics {
        informative_terms: evidence.len(),
        distinctive_matches: output
            .iter()
            .filter(|item| item.match_strength.as_deref() == Some("distinctive"))
            .count(),
        top_score: output.first().map(|item| item.adjusted_score),
        coverage,
    };
    Ok((output, diagnostics))
}
pub fn word_tokens(text: &str) -> Vec<String> {
    regex::Regex::new(r"[\p{L}\p{N}_]+")
        .map(|pattern| {
            pattern
                .find_iter(text)
                .map(|found| folded(found.as_str()))
                .collect()
        })
        .unwrap_or_default()
}
pub fn contains_decimal_digit(text: &str) -> bool {
    static DECIMAL: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    DECIMAL
        .get_or_init(|| regex::Regex::new(r"\p{Nd}").expect("constant Unicode decimal grammar"))
        .is_match(text)
}
pub fn is_anchor(token: &str, where_words: &[String]) -> bool {
    contains_decimal_digit(token)
        || token.contains('_')
        || token
            .as_bytes()
            .windows(2)
            .any(|pair| pair[0].is_ascii_lowercase() && pair[1].is_ascii_uppercase())
        || where_words.contains(&folded(token))
}
thread_local! {static TERM_FORMS:std::cell::RefCell<BTreeMap<String,String>>=const{std::cell::RefCell::new(BTreeMap::new())};}
pub fn term_forms(words: &[String]) -> BTreeMap<String, String> {
    let missing = TERM_FORMS.with(|cache| {
        words
            .iter()
            .filter(|word| !cache.borrow().contains_key(*word))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    });
    let fresh = (|| -> rusqlite::Result<BTreeMap<String, String>> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("CREATE VIRTUAL TABLE words USING fts5(word,tokenize='porter unicode61'); CREATE VIRTUAL TABLE forms USING fts5vocab(words,'instance');")?;
        for (index, word) in missing.iter().enumerate() {
            conn.execute(
                "INSERT INTO words(rowid,word) VALUES(?1,?2)",
                params![index + 1, word],
            )?;
        }
        let mut statement = conn.prepare("SELECT doc,term FROM forms ORDER BY doc,offset")?;
        let mut stems = BTreeMap::<usize, Vec<String>>::new();
        for row in statement.query_map([], |row| {
            Ok((row.get::<_, usize>(0)?, row.get::<_, String>(1)?))
        })? {
            let (id, term) = row?;
            stems.entry(id).or_default().push(term);
        }
        Ok(missing
            .iter()
            .enumerate()
            .map(|(index, word)| {
                (
                    word.clone(),
                    stems
                        .get(&(index + 1))
                        .cloned()
                        .unwrap_or_default()
                        .join(" "),
                )
            })
            .collect())
    })();
    let fresh = fresh.unwrap_or_else(|_| {
        missing
            .into_iter()
            .map(|word| (word.clone(), folded(&word)))
            .collect()
    });
    TERM_FORMS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() + fresh.len() > 4096 {
            cache.clear();
        }
        cache.extend(fresh.clone());
        words
            .iter()
            .map(|word| {
                (
                    word.clone(),
                    cache
                        .get(word)
                        .cloned()
                        .or_else(|| fresh.get(word).cloned())
                        .unwrap_or_else(|| folded(word)),
                )
            })
            .collect()
    })
}

pub fn body_excerpt(content: &str, limit: usize, tokens: &[String], prefer_head: bool) -> String {
    let body = content
        .strip_prefix("---\n")
        .and_then(|tail| tail.split_once("\n---\n").map(|(_, body)| body))
        .unwrap_or(content);
    let lines = body
        .lines()
        .filter(|line| !line.trim_start().starts_with('#') && !line.trim().is_empty())
        .map(str::trim)
        .collect::<Vec<_>>();
    let text = lines.join(" ");
    if text.chars().count() <= limit {
        return text;
    }
    let units = text
        .split_inclusive(['.', '!', '?', ';'])
        .map(str::trim)
        .filter(|unit| !unit.is_empty())
        .collect::<Vec<_>>();
    let weights = tokens.iter().map(|token| folded(token)).collect::<Vec<_>>();
    let mut best = (0usize, 0usize, 0usize);
    for start in 0..units.len() {
        if prefer_head && start > 0 {
            break;
        }
        let mut length = 0;
        let mut score = 0;
        let mut end = start;
        while end < units.len() {
            let cost = units[end].chars().count() + usize::from(end > start);
            if length + cost > limit.saturating_sub(4) {
                break;
            }
            length += cost;
            let folded_unit = folded(units[end]);
            score += weights
                .iter()
                .filter(|token| folded_unit.contains(token.as_str()))
                .count();
            end += 1;
        }
        if end == start {
            end = start + 1;
            score = weights
                .iter()
                .filter(|token| folded(units[start]).contains(token.as_str()))
                .count();
        }
        if score > best.0 || (start == 0 && best.2 == 0) {
            best = (score, start, end);
        }
    }
    let (_, start, end) = best;
    let piece = units.get(start..end).unwrap_or(&[]).join(" ");
    let piece = chars(&piece, limit.saturating_sub(4));
    format!(
        "{}{}{}",
        if start > 0 { "… " } else { "" },
        piece,
        if end < units.len()
            || piece.chars().count()
                < units
                    .get(start..end)
                    .unwrap_or(&[])
                    .join(" ")
                    .chars()
                    .count()
        {
            " …"
        } else {
            ""
        }
    )
}
pub fn search_episodes(conn: &Connection, query: &str, limit: usize) -> Result<Vec<Episode>> {
    let tokens = expand_query_tokens(conn, query_tokens(query)?)?;
    if evidence_tokens(&tokens).is_empty() {
        return Ok(vec![]);
    }
    let mut statement=conn.prepare("SELECT rowid,summary,outcome,files,verification,sources,created_at FROM episodes WHERE episodes MATCH ?1 ORDER BY bm25(episodes),rowid DESC LIMIT ?2")?;
    let rows = statement.query_map(params![fts_expression(&tokens), limit], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;
    rows.map(|row| {
        let (id, summary, outcome, files, verification, sources, created_at) = row?;
        Ok(Episode {
            id,
            layer: "episodic".into(),
            summary,
            outcome,
            files: serde_json::from_str(&files)?,
            verification: serde_json::from_str(&verification)?,
            sources: serde_json::from_str(&sources)?,
            created_at,
        })
    })
    .collect()
}
