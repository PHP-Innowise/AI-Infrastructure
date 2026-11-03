//! Subject-only recorded history and project work-item discrimination.
use crate::{
    Result, RuntimeConfig, RuntimeContext,
    index::{DOCUMENT_SELECT, Exclusion, document_from_row},
    query::{self, Candidate},
    storage::{Episode, Store},
};
use rusqlite::{Connection, params_from_iter};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const STOP: &str = "AES AGPL ANSI ASCII BCP BSD CET CP CRC CSS CVE CVSS CWE DES DIN DNS DSA ECDSA ECMA EN EPL ES FIPS GDPR GMT GPL GPT HMAC HTML HTTP HTTPS IEC IEEE IP IPV ISO JSON KOI LATIN LGPL MD MPEG MPL NIST OAUTH OWASP PCI PER PHP PSR RFC RSA SHA SOC SSL TCP TLS UCS UDP UTC UTF UUID WCAG WIN XML YAML";
fn word(character: char) -> bool {
    let mut encoded = [0; 4];
    regex::Regex::new(r"^[\p{L}\p{N}_]$")
        .is_ok_and(|pattern| pattern.is_match(character.encode_utf8(&mut encoded)))
}
pub fn ticket_ids(text: &str, known: &BTreeSet<String>) -> Vec<String> {
    let Ok(pattern) = regex::Regex::new(r"([A-Z]{2,10})-([0-9]{1,6})") else {
        return vec![];
    };
    let mut output = Vec::new();
    for captures in pattern.captures_iter(text) {
        let Some(found) = captures.get(0) else {
            continue;
        };
        let before = text[..found.start()].chars().next_back();
        let after = text[found.end()..].chars().next();
        if before.is_some_and(word) || after.is_some_and(word) {
            continue;
        }
        if text[found.end()..]
            .strip_prefix('.')
            .and_then(|tail| tail.chars().next())
            .is_some_and(|c| c.is_ascii_digit())
        {
            continue;
        }
        let prefix = &captures[1];
        if STOP.split_whitespace().any(|stop| stop == prefix)
            || (!known.is_empty() && !known.contains(prefix))
        {
            continue;
        }
        let value = found.as_str().to_string();
        if !output.contains(&value) {
            output.push(value);
        }
    }
    output
}
pub fn work_prefixes(record: &Value) -> BTreeSet<String> {
    let text = if record["type"] == "task" {
        record["external_id"].as_str().unwrap_or("").to_string()
    } else if record["type"] == "event" {
        let title = crate::results::collapse(record["title"].as_str().unwrap_or(""));
        if query::folded(&title).starts_with("completed ") {
            title[10..]
                .split_once(": ")
                .map(|(subject, _)| subject.trim().to_string())
                .unwrap_or_default()
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    ticket_ids(&text, &BTreeSet::new())
        .into_iter()
        .filter_map(|ticket| ticket.split_once('-').map(|(prefix, _)| prefix.into()))
        .collect()
}
pub fn known_prefixes(store: &Store, task: Option<&Value>) -> BTreeSet<String> {
    let mut known = store
        .state()
        .ok()
        .and_then(|state| state.get("ticket-prefixes").cloned())
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default()
        .into_iter()
        .collect::<BTreeSet<_>>();
    if let Some(task) = task {
        known.extend(work_prefixes(task));
    }
    known
}
pub fn request_tickets(
    text: &str,
    task: Option<&Value>,
    known: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut found = ticket_ids(text, known)
        .into_iter()
        .map(|ticket| ticket.to_lowercase())
        .collect::<BTreeSet<_>>();
    if let Some(task) = task {
        let text = ["external_id", "title", "goal"]
            .map(|key| task[key].as_str().unwrap_or(""))
            .join(" ");
        found.extend(
            ticket_ids(&text, known)
                .into_iter()
                .map(|ticket| ticket.to_lowercase()),
        );
    }
    found
}
pub fn ticket_filter(
    context: &RuntimeContext,
    items: Vec<Candidate>,
    tickets: &BTreeSet<String>,
    known: &BTreeSet<String>,
    request: &str,
) -> Result<(Vec<Candidate>, Vec<Exclusion>)> {
    if tickets.is_empty() {
        return Ok((items, vec![]));
    }
    let mut kept = Vec::new();
    let mut excluded = Vec::new();
    for item in items {
        let document = &item.document;
        let selected = document.category == "durable" || document.kind == "brain-finding";
        let changelog = document.kind == "changelog";
        if !selected && !changelog {
            kept.push(item);
            continue;
        }
        let mut named = ticket_ids(&format!("{}\n{}", document.title, document.content), known)
            .into_iter()
            .map(|ticket| ticket.to_lowercase())
            .collect::<BTreeSet<_>>();
        if changelog && query::folded(request).contains("changelog") {
            kept.push(item);
            continue;
        }
        if named.is_empty()
            && document.kind == "brain-finding"
            && let Ok(text) = crate::fs::read_text(&context.roots.resolve(&document.path))
        {
            named.extend(
                ticket_ids(&text, known)
                    .into_iter()
                    .map(|ticket| ticket.to_lowercase()),
            );
        }
        if !named.is_empty() && named.is_disjoint(tickets) {
            excluded.push(Exclusion {
                path: document.path.clone(),
                reason: "ticket-mismatch".into(),
            });
        } else {
            kept.push(item);
        }
    }
    Ok((kept, excluded))
}

#[derive(Clone, Debug)]
pub struct HistoryQuery {
    pub terms: Vec<String>,
    pub weights: BTreeMap<String, f64>,
    pub minimum: usize,
    pub floor: f64,
    pub anchor_floor: f64,
    pub tickets: BTreeSet<String>,
    pub known: BTreeSet<String>,
    pub events: BTreeMap<String, BTreeSet<String>>,
    pub episodes: BTreeMap<String, BTreeSet<i64>>,
}
#[derive(Clone, Debug)]
pub struct WeightedEpisode {
    pub score: f64,
    pub mass: f64,
    pub episode: Episode,
}
pub fn history_query(conn: &Connection, text: &str) -> Result<Option<HistoryQuery>> {
    let tokens = match query::query_tokens(text) {
        Ok(tokens) => query::evidence_tokens(&tokens),
        Err(_) => return Ok(None),
    };
    if tokens.is_empty() {
        return Ok(None);
    }
    let documents = conn.query_row("SELECT COUNT(*) FROM documents", [], |row| {
        row.get::<_, usize>(0)
    })?;
    let events_indexed = conn.query_row(
        "SELECT COUNT(*) FROM documents WHERE kind='brain-event'",
        [],
        |row| row.get::<_, usize>(0),
    )?;
    let episodes_indexed = conn
        .query_row("SELECT COUNT(*) FROM episodes", [], |row| {
            row.get::<_, usize>(0)
        })
        .unwrap_or(0);
    let total = documents + episodes_indexed;
    if total == 0 || (events_indexed == 0 && episodes_indexed == 0) {
        return Ok(None);
    }
    let frequencies = query::frequencies(conn, &tokens)?;
    let mut events = BTreeMap::new();
    let mut episodes = BTreeMap::new();
    let mut counts = BTreeMap::new();
    for token in &tokens {
        let mut event_ids = BTreeSet::new();
        let mut episode_ids = BTreeSet::new();
        if events_indexed > 0 && frequencies.get(token).copied().unwrap_or(0) > 0 {
            let mut statement = conn.prepare(
                "SELECT path FROM documents WHERE documents MATCH ?1 AND kind='brain-event'",
            )?;
            for path in statement.query_map([format!("summary : \"{token}\"")], |row| {
                row.get::<_, String>(0)
            })? {
                event_ids.insert(path?);
            }
        }
        if episodes_indexed > 0 {
            let mut statement =
                conn.prepare("SELECT rowid FROM episodes WHERE episodes MATCH ?1")?;
            for id in statement
                .query_map([format!("{{summary outcome}} : \"{token}\"")], |row| {
                    row.get::<_, i64>(0)
                })?
            {
                episode_ids.insert(id?);
            }
        }
        let count = frequencies
            .get(token)
            .copied()
            .filter(|count| *count > 0)
            .unwrap_or(episode_ids.len());
        counts.insert(token.clone(), count);
        events.insert(token.clone(), event_ids);
        episodes.insert(token.clone(), episode_ids);
    }
    let pool = tokens
        .iter()
        .filter(|token| counts[*token] > 0 && (counts[*token] as f64) <= total as f64 * 0.5)
        .cloned()
        .collect::<Vec<_>>();
    let mut reach = pool
        .iter()
        .filter(|token| !events[*token].is_empty() || !episodes[*token].is_empty())
        .cloned()
        .collect::<Vec<_>>();
    if reach.is_empty() {
        return Ok(None);
    }
    let positions = tokens
        .iter()
        .enumerate()
        .map(|(index, token)| (token.clone(), index))
        .collect::<BTreeMap<_, _>>();
    reach.sort_by_key(|token| (counts[token], positions[token]));
    reach.truncate(24);
    reach.sort_by_key(|token| positions[token]);
    let mut minimum = if pool.len() >= 2 { 2 } else { 1 };
    let forms = query::term_forms(&reach);
    let mut families = reach
        .iter()
        .map(|term| {
            forms
                .get(term)
                .filter(|form| !form.is_empty())
                .cloned()
                .unwrap_or_else(|| query::folded(term))
        })
        .collect::<BTreeSet<_>>();
    if families.len() < minimum {
        let all = query::term_forms(&pool);
        for term in &pool {
            families.insert(
                all.get(term)
                    .filter(|form| !form.is_empty())
                    .cloned()
                    .unwrap_or_else(|| query::folded(term)),
            );
            if families.len() >= minimum {
                break;
            }
        }
        minimum = if families.len() >= 2 { 2 } else { 1 };
    }
    let weights = reach
        .iter()
        .map(|term| {
            (
                term.clone(),
                ((total as f64 + 1.0) / (counts[term] as f64 + 0.5))
                    .ln()
                    .max(0.05),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let floor = minimum as f64 * ((total as f64 + 1.0) / 1.5).ln().max(0.05);
    let mut values = weights.values().copied().collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    let anchor_floor = values.into_iter().take(minimum).sum();
    events.retain(|term, _| reach.contains(term));
    episodes.retain(|term, _| reach.contains(term));
    Ok(Some(HistoryQuery {
        terms: reach,
        weights,
        minimum,
        floor,
        anchor_floor,
        tickets: ticket_ids(text, &BTreeSet::new())
            .into_iter()
            .map(|ticket| ticket.to_lowercase())
            .collect(),
        known: BTreeSet::new(),
        events,
        episodes,
    }))
}
fn admitted<T: Ord + Clone>(
    history: &HistoryQuery,
    members: &BTreeMap<String, BTreeSet<T>>,
) -> BTreeMap<T, f64> {
    let forms = query::term_forms(&history.terms);
    let mut concepts = BTreeMap::<T, BTreeSet<String>>::new();
    let mut masses = BTreeMap::<T, f64>::new();
    let mut anchors = BTreeSet::new();
    for (term, found) in members {
        for member in found {
            concepts.entry(member.clone()).or_default().insert(
                forms
                    .get(term)
                    .filter(|form| !form.is_empty())
                    .cloned()
                    .unwrap_or_else(|| query::folded(term)),
            );
            *masses.entry(member.clone()).or_default() +=
                history.weights.get(term).copied().unwrap_or(0.0);
            if query::is_anchor(term, &[]) {
                anchors.insert(member.clone());
            }
        }
    }
    masses
        .into_iter()
        .filter(|(member, mass)| {
            concepts.get(member).map(BTreeSet::len).unwrap_or(0) >= history.minimum
                && *mass + 1e-9
                    >= if anchors.contains(member) {
                        history.anchor_floor
                    } else {
                        history.floor
                    }
        })
        .collect()
}
pub fn candidates(
    conn: &Connection,
    history: &HistoryQuery,
) -> Result<(Vec<Candidate>, Vec<WeightedEpisode>)> {
    let event_mass = admitted(history, &history.events);
    let episode_mass = admitted(history, &history.episodes);
    let mut events = Vec::new();
    let mut episodes = Vec::new();
    if !event_mass.is_empty() {
        let sql = format!(
            "SELECT {DOCUMENT_SELECT} FROM documents d JOIN document_metadata m ON m.path=d.path WHERE d.kind='brain-event' AND d.path IN ({})",
            vec!["?"; event_mass.len()].join(",")
        );
        let mut statement = conn.prepare(&sql)?;
        for document in
            statement.query_map(params_from_iter(event_mass.keys()), document_from_row)?
        {
            let document = document?;
            let mass = event_mass[&document.path];
            let mut item = Candidate::linked(document, "history");
            item.match_strength = Some("covered".into());
            item.selection = None;
            item.score = -mass;
            item.adjusted_score = mass
                * query::ranking_weight(
                    &item.document.authority,
                    item.document.confidence,
                    &item.document.updated_at,
                );
            item.snippet =
                crate::excerpts::quote_candidate(conn, &item, &history.terms.join(" "), 320)?;
            item.estimated_tokens =
                query::estimated_tokens(&format!("{}{}", item.document.title, item.snippet));
            events.push(item);
        }
    }
    events.sort_by(|left, right| {
        right
            .adjusted_score
            .total_cmp(&left.adjusted_score)
            .then_with(|| left.document.path.cmp(&right.document.path))
    });
    for (index, item) in events.iter_mut().enumerate() {
        item.rank = Some(index + 1);
    }
    if !episode_mass.is_empty() {
        let sql = format!(
            "SELECT rowid,summary,outcome,files,verification,sources,created_at FROM episodes WHERE rowid IN ({})",
            vec!["?"; episode_mass.len()].join(",")
        );
        let mut statement = conn.prepare(&sql)?;
        for row in statement.query_map(params_from_iter(episode_mass.keys()), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })? {
            let (id, summary, outcome, files, verification, sources, created_at) = row?;
            let mass = episode_mass[&id];
            let score = mass * query::ranking_weight("observed", 1.0, &created_at);
            episodes.push(WeightedEpisode {
                score,
                mass,
                episode: Episode {
                    id,
                    layer: "episodic".into(),
                    summary,
                    outcome,
                    files: serde_json::from_str(&files)?,
                    verification: serde_json::from_str(&verification)?,
                    sources: serde_json::from_str(&sources)?,
                    created_at,
                },
            });
        }
    }
    episodes.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| right.episode.id.cmp(&left.episode.id))
    });
    Ok((events, episodes))
}
pub fn best_events(
    context: &RuntimeContext,
    config: &RuntimeConfig,
    events: Vec<Candidate>,
    limit: usize,
    known: &BTreeSet<String>,
) -> Result<(Vec<Candidate>, Vec<Exclusion>)> {
    best_events_observed(context, config, events, limit, known, None)
}
pub fn best_events_observed(
    context: &RuntimeContext,
    config: &RuntimeConfig,
    events: Vec<Candidate>,
    limit: usize,
    known: &BTreeSet<String>,
    mut trace: Option<&mut crate::observability::Trace>,
) -> Result<(Vec<Candidate>, Vec<Exclusion>)> {
    best_events_filtered(events, limit, known, |items| {
        crate::retrieval::filter_observed(context, config, items, false, trace.as_deref_mut())
    })
}
#[expect(
    clippy::too_many_arguments,
    reason = "Scoped history admission shares connection, source policy, known IDs, ranking limit and trace from one retrieval"
)]
pub fn best_events_in_scope(
    context: &RuntimeContext,
    config: &RuntimeConfig,
    connection: &Connection,
    policy: &mut crate::source_policy::CurrentSourcePolicy,
    events: Vec<Candidate>,
    limit: usize,
    known: &BTreeSet<String>,
    mut trace: Option<&mut crate::observability::Trace>,
) -> Result<(Vec<Candidate>, Vec<Exclusion>)> {
    best_events_filtered(events, limit, known, |items| {
        crate::source_policy::filter_in_scope(
            context,
            config,
            connection,
            policy,
            items,
            false,
            trace.as_deref_mut(),
        )
    })
}
fn best_events_filtered<F>(
    events: Vec<Candidate>,
    limit: usize,
    known: &BTreeSet<String>,
    mut filter: F,
) -> Result<(Vec<Candidate>, Vec<Exclusion>)>
where
    F: FnMut(Vec<Candidate>) -> Result<(Vec<Candidate>, Vec<Exclusion>)>,
{
    let mut capacity = limit.max(1);
    let mut input = events;
    if limit == 1
        && std::env::var("CONTEXT_EVENT_SERIES")
            .ok()
            .is_some_and(|value| {
                ["1", "true", "yes"]
                    .contains(&query::folded(value.trim_matches(crate::results::py_space)).as_str())
            })
    {
        let mut groups = Vec::<(String, Vec<usize>)>::new();
        for (index, item) in input.iter().enumerate() {
            for ticket in ticket_ids(&item.document.title, known) {
                if let Some((_, rows)) = groups.iter_mut().find(|(name, _)| name == &ticket) {
                    rows.push(index);
                } else {
                    groups.push((ticket, vec![index]));
                }
            }
        }
        let mut best = None::<&Vec<usize>>;
        for (_, rows) in &groups {
            if best.is_none_or(|best| rows.len() > best.len()) {
                best = Some(rows);
            }
        }
        if let Some(series) = best.filter(|series| series.len() > 1) {
            capacity = series.len().min(2);
            let selected = series.iter().copied().collect::<BTreeSet<_>>();
            input = input
                .into_iter()
                .enumerate()
                .filter(|(index, _)| selected.contains(index))
                .map(|(_, item)| item)
                .collect();
        }
    }
    let mut kept = Vec::<Candidate>::new();
    let mut excluded = vec![];
    for item in input {
        if kept.len() >= capacity
            && item.adjusted_score <= kept.last().map(|item| item.adjusted_score).unwrap_or(0.0)
        {
            break;
        }
        let (items, refused) = filter(vec![item])?;
        excluded.extend(refused);
        kept.extend(items);
        kept.sort_by(|left, right| right.adjusted_score.total_cmp(&left.adjusted_score));
        kept.truncate(capacity);
    }
    Ok((kept, excluded))
}
