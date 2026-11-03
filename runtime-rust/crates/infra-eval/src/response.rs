//! Public response shape and literal-source evidence. No factual oracle lives here.
use crate::{Result, artifact};
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub const MARKER: &str = "MEMORY_TASK_ANSWER";
pub const LIMITS: [(&str, u64, u64); 12] = [
    ("max_messages", 1, 64),
    ("max_message_bytes", 1, 262144),
    ("max_total_bytes", 1, 1048576),
    ("max_answer_string_chars", 0, 65536),
    ("integer_abs_max", 1, 9007199254740991),
    ("min_citations", 0, 16),
    ("max_citations", 0, 16),
    ("max_citation_span_lines", 1, 10000),
    ("max_citation_quote_chars", 1, 65536),
    ("max_citation_path_chars", 1, 1024),
    ("min_explanation_chars", 0, 65536),
    ("max_explanation_chars", 0, 65536),
];
pub const CONTEXT: [&str; 4] = [
    "protocol_sha256",
    "slot_id",
    "verifier_sha256",
    "source_image_sha256",
];
pub fn identity(v: &Value, max: usize) -> bool {
    v.as_str()
        .is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= max && !s.contains('\0'))
}
pub fn path(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.chars().count() <= max
        && !s.contains(['\\', ':'])
        && !s.chars().any(|c| c < ' ' || c == '\u{7f}')
        && !s.split('/').any(|p| p.is_empty() || p == "." || p == "..")
}
pub fn validate(public: &Value) -> Result<Value> {
    let v2 = public["schema"] == "memory-response-contract-v2";
    let mut keys = vec!["schema", "case_id", "marker", "answer_fields", "limits"];
    if v2 {
        keys.push("citation_corpus");
    }
    if !artifact::exact(public, &keys)
        || !["memory-response-contract-v1", "memory-response-contract-v2"]
            .contains(&public["schema"].as_str().unwrap_or(""))
        || public["marker"] != MARKER
        || !identity(&public["case_id"], 128)
    {
        return Err(artifact::invalid("invalid public contract"));
    }
    let fields = public["answer_fields"]
        .as_object()
        .filter(|m| (1..=32).contains(&m.len()))
        .ok_or_else(|| artifact::invalid("one to 32 flat answer fields required"))?;
    for (name, spec) in fields {
        if !identity(&json!(name), 64)
            || !artifact::exact(spec, &["type", "nullable"])
            || !["boolean", "integer", "string"].contains(&spec["type"].as_str().unwrap_or(""))
            || !spec["nullable"].is_boolean()
        {
            return Err(artifact::invalid("invalid public answer field"));
        }
    }
    if !public["limits"]
        .as_object()
        .is_some_and(|m| m.len() == LIMITS.len())
    {
        return Err(artifact::invalid("all public limits must be declared"));
    }
    for (name, low, high) in LIMITS {
        if !public["limits"][name]
            .as_u64()
            .is_some_and(|n| n >= low && n <= high)
        {
            return Err(artifact::invalid("invalid public limit"));
        }
    }
    let l = &public["limits"];
    if l["min_citations"].as_u64() > l["max_citations"].as_u64()
        || l["min_explanation_chars"].as_u64() > l["max_explanation_chars"].as_u64()
        || l["max_total_bytes"].as_u64() < l["max_message_bytes"].as_u64()
    {
        return Err(artifact::invalid("incoherent public limits"));
    }
    if v2 {
        let corpus = &public["citation_corpus"];
        if !artifact::exact(corpus, &["schema", "files"])
            || corpus["schema"] != "memory-citation-corpus-v1"
        {
            return Err(artifact::invalid("invalid citation corpus"));
        }
        let files = corpus["files"]
            .as_array()
            .filter(|v| v.len() <= 256)
            .ok_or_else(|| artifact::invalid("invalid citation corpus"))?;
        let (mut previous, mut total) = (None::<&str>, 0u64);
        for entry in files {
            let name = entry["path"]
                .as_str()
                .ok_or_else(|| artifact::invalid("invalid citation path"))?;
            if !artifact::exact(entry, &["path", "sha256", "bytes"])
                || !path(
                    name,
                    l["max_citation_path_chars"].as_u64().unwrap() as usize,
                )
                || !entry["sha256"]
                    .as_str()
                    .is_some_and(|s| artifact::hex(s, 64))
                || !entry["bytes"].as_u64().is_some_and(|n| n <= 1048576)
                || previous.is_some_and(|p| name <= p)
            {
                return Err(artifact::invalid("invalid citation corpus entry"));
            }
            previous = Some(name);
            total += entry["bytes"].as_u64().unwrap();
        }
        if total > 16777216
            || infra_core::domain::json::canonical(corpus)?.len() > 65536
            || files.is_empty() && (l["min_citations"] != 0 || l["max_citations"] != 0)
        {
            return Err(artifact::invalid("citation corpus exceeds bounds"));
        }
        if render_unchecked(public)?.len() > 131072 {
            return Err(artifact::invalid("rendered v2 contract exceeds byte bound"));
        }
    }
    Ok(public.clone())
}
pub fn contract_digest(public: &Value) -> Result<String> {
    artifact::digest(&validate(public)?)
}
pub fn render(public: &Value) -> Result<String> {
    render_unchecked(&validate(public)?)
}
fn render_unchecked(public: &Value) -> Result<String> {
    let mut rules = format!(
        "Every answer attempt must use the exact ASCII marker on its first line: {MARKER}\nLF and CRLF marker line endings are accepted. Follow it with one plain JSON object; no fences or surrounding prose. The exact keys are case_id, answer, citations, explanation.\ncase_id must equal the public case_id. answer has exactly the declared flat fields. Boolean is not integer; integers (including citation lines) are bounded by integer_abs_max. Only fields declared nullable accept null. String lengths count Unicode code points.\nEach citation has exactly path, start_line, end_line, quote. Path and quote are nonempty strings within their declared character limits; line numbers are positive integers, end_line >= start_line, and inclusive span is bounded. This checks shape, not source truth.\nExplanation length is bounded as declared; length does not prove substance. Message limits count exact UTF-8 bytes; transcript limits cover all supplied assistant messages. Duplicate JSON keys and nonfinite values are refused.\nThe last first-line-marked attempt is selected, even when its JSON or case_id is invalid. There is no fallback. Later unmarked acknowledgments are ignored; unmarked intent is not inferred.\n"
    );
    if public["schema"] == "memory-response-contract-v2" {
        rules.push_str("Only exact paths in citation_corpus.files are eligible citation sources, at their declared raw-byte SHA-256 and byte sizes. Readable, installed, indexed, generated or retrieved files absent from that complete list are not eligible. Paths are literal relative POSIX names: no empty, dot or dot-dot components, backslashes, colons, C0 controls or DEL. No percent decoding, case folding or Unicode normalization is performed.\nSource bytes decode as strict UTF-8 without BOM removal or newline conversion. Lines are LF-delimited, one-based and inclusive; a final LF does not create a phantom extra line, and an empty file has zero lines. CR, including CR in CRLF, is preserved; lone CR and Unicode line separators do not start new lines. The quote must be an exact contiguous substring within the requested original line window (excluding only its final LF). No trimming, whitespace folding, ellipsis expansion or newline normalization is applied.\nAn empty corpus permits only an empty citation list. Source eligibility establishes literal presence, not factual truth, current authority, an adopted decision or approval. Source bytes and complete prompt transport must be frozen and verified by the caller; shape checking alone does not verify source contents.\n");
    }
    Ok(format!(
        "{rules}\nPUBLIC_CONTRACT\n{}",
        infra_core::domain::json::serialize(public, true, false, true, false)?
    ))
}
pub fn marked(text: &str) -> bool {
    text.split('\n')
        .next()
        .unwrap_or("")
        .strip_suffix('\r')
        .unwrap_or(text.split('\n').next().unwrap_or(""))
        == MARKER
}
pub fn body(text: &str) -> Option<&str> {
    let (first, body) = text.split_once('\n')?;
    (first.strip_suffix('\r').unwrap_or(first) == MARKER).then_some(body)
}
fn checked(errors: Vec<String>) -> Value {
    json!({"valid":errors.is_empty(),"errors":errors})
}
pub fn check(public: &Value, text: &str) -> Result<Value> {
    let public = validate(public)?;
    let l = &public["limits"];
    let mut errors = vec![];
    if text.len() > l["max_message_bytes"].as_u64().unwrap() as usize {
        return Ok(checked(vec![
            "message exceeds declared UTF-8 byte limit or has wrong type".into(),
        ]));
    }
    let value = match body(text).and_then(|b| artifact::strict(b.as_bytes()).ok()) {
        Some(v) => v,
        None => {
            return Ok(checked(vec![
                "missing first-line marker or invalid strict JSON".into(),
            ]));
        }
    };
    if !artifact::exact(&value, &["case_id", "answer", "citations", "explanation"]) {
        return Ok(checked(vec![
            "response must have exactly the disclosed envelope keys".into(),
        ]));
    }
    if value["case_id"] != public["case_id"] {
        errors.push("case_id differs from the public case_id".into());
    }
    let fields = public["answer_fields"].as_object().unwrap();
    if !artifact::exact(
        &value["answer"],
        &fields.keys().map(String::as_str).collect::<Vec<_>>(),
    ) {
        errors.push("answer must have exactly the declared flat fields".into());
    } else {
        for (name, spec) in fields {
            let a = &value["answer"][name];
            if a.is_null() && spec["nullable"] == true {
                continue;
            }
            let valid = match spec["type"].as_str().unwrap_or("") {
                "boolean" => a.is_boolean(),
                "integer" => {
                    a.as_i64()
                        .is_some_and(|i| i.unsigned_abs() <= l["integer_abs_max"].as_u64().unwrap())
                        || a.as_u64()
                            .is_some_and(|i| i <= l["integer_abs_max"].as_u64().unwrap())
                }
                "string" => a.as_str().is_some_and(|s| {
                    s.chars().count() <= l["max_answer_string_chars"].as_u64().unwrap() as usize
                }),
                _ => false,
            };
            if !valid {
                errors.push(format!("invalid answer field: {name}"));
            }
        }
    }
    let citations = value["citations"].as_array();
    if !citations.is_some_and(|v| {
        v.len() >= l["min_citations"].as_u64().unwrap() as usize
            && v.len() <= l["max_citations"].as_u64().unwrap() as usize
    }) {
        errors.push("citation count is outside the declared bounds".into());
    } else {
        for c in citations.unwrap() {
            if !artifact::exact(c, &["path", "start_line", "end_line", "quote"]) {
                errors.push("citation must have exactly the disclosed keys".into());
                continue;
            }
            for (field, limit) in [
                ("path", "max_citation_path_chars"),
                ("quote", "max_citation_quote_chars"),
            ] {
                if !c[field].as_str().is_some_and(|s| {
                    !s.is_empty() && s.chars().count() <= l[limit].as_u64().unwrap() as usize
                }) {
                    errors.push(format!(
                        "citation {field} is outside the declared character bounds"
                    ));
                }
            }
            if public["schema"] == "memory-response-contract-v2" {
                if !c["path"].as_str().is_some_and(|s| {
                    path(s, l["max_citation_path_chars"].as_u64().unwrap() as usize)
                }) {
                    errors.push("citation path is not a canonical relative POSIX path".into());
                } else if !public["citation_corpus"]["files"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|e| e["path"] == c["path"])
                {
                    errors.push("citation path is not in the public citation corpus".into());
                }
            }
            if !c["start_line"]
                .as_u64()
                .zip(c["end_line"].as_u64())
                .is_some_and(|(s, e)| {
                    s >= 1
                        && e >= s
                        && e <= l["integer_abs_max"].as_u64().unwrap()
                        && e - s < l["max_citation_span_lines"].as_u64().unwrap()
                })
            {
                errors.push("citation line range is outside the declared bounds".into());
            }
        }
    }
    if !value["explanation"].as_str().is_some_and(|s| {
        let n = s.chars().count() as u64;
        n >= l["min_explanation_chars"].as_u64().unwrap()
            && n <= l["max_explanation_chars"].as_u64().unwrap()
    }) {
        errors.push("explanation is outside the declared character bounds".into());
    }
    Ok(checked(errors))
}
pub fn citations(public: &Value, text: &str, sources: &BTreeMap<String, Vec<u8>>) -> Result<Value> {
    let public = validate(public)?;
    if public["schema"] != "memory-response-contract-v2" {
        return Err(artifact::invalid("citation source checking requires v2"));
    }
    let files = public["citation_corpus"]["files"].as_array().unwrap();
    if sources.len() != files.len() {
        return Err(artifact::invalid("citation source context paths differ"));
    }
    let mut decoded = BTreeMap::new();
    for e in files {
        let name = e["path"].as_str().unwrap();
        let raw = sources
            .get(name)
            .ok_or_else(|| artifact::invalid("citation source context paths differ"))?;
        if raw.len() as u64 != e["bytes"].as_u64().unwrap()
            || artifact::sha(raw) != e["sha256"].as_str().unwrap()
        {
            return Err(artifact::invalid("citation source context bytes differ"));
        }
        decoded.insert(
            name,
            std::str::from_utf8(raw)
                .map_err(|_| artifact::invalid("citation source context is not strict UTF-8"))?,
        );
    }
    let contract_check = check(&public, text)?;
    if contract_check["valid"] != true {
        return Ok(contract_check);
    }
    let value = artifact::strict(body(text).unwrap().as_bytes())?;
    let mut errors = vec![];
    for c in value["citations"].as_array().unwrap() {
        let source = decoded[c["path"].as_str().unwrap()];
        let start = c["start_line"].as_u64().unwrap() as usize;
        let end = c["end_line"].as_u64().unwrap() as usize;
        let mut lines = source.split_inclusive('\n');
        let mut offset = 0;
        let (mut a, mut b) = (None, None);
        for n in 1..=end {
            let Some(line) = lines.next() else {
                break;
            };
            if n == start {
                a = Some(offset);
            }
            offset += line.len();
            if n == end {
                b = Some(offset - usize::from(line.ends_with('\n')));
            }
        }
        match (a, b) {
            (Some(a), Some(b)) if source[a..b].contains(c["quote"].as_str().unwrap()) => {}
            (Some(_), Some(_)) => {
                errors.push("citation quote is not literal within its declared line range".into())
            }
            _ => errors.push("citation line range does not exist in the declared source".into()),
        }
    }
    Ok(checked(errors))
}
pub fn select(public: &Value, messages: &Value) -> Result<Value> {
    let public = validate(public)?;
    let l = &public["limits"];
    let mut empty = json!({"status":"unavailable","index":null,"text":null,"message_sha256":null,"messages_sha256":null,"check":{"valid":null,"errors":[]}});
    let Some(messages) = messages
        .as_array()
        .filter(|v| v.len() <= l["max_messages"].as_u64().unwrap() as usize)
    else {
        empty["check"]["errors"] = json!(["assistant message count exceeds declared bound"]);
        return Ok(empty);
    };
    let mut total = 0usize;
    for text in messages {
        let Some(text) = text.as_str() else {
            empty["check"]["errors"] = json!(["message has wrong type"]);
            return Ok(empty);
        };
        total += text.len();
        if text.len() > l["max_message_bytes"].as_u64().unwrap() as usize
            || total > l["max_total_bytes"].as_u64().unwrap() as usize
        {
            empty["check"]["errors"] = json!(["assistant transcript exceeds declared byte bound"]);
            return Ok(empty);
        }
    }
    let stream = artifact::digest(&json!(messages))?;
    let Some(index) = messages.iter().rposition(|t| marked(t.as_str().unwrap())) else {
        empty["status"] = "unmarked".into();
        empty["messages_sha256"] = stream.into();
        empty["check"] = checked(vec!["no first-line-marked attempt".into()]);
        return Ok(empty);
    };
    let text = messages[index].as_str().unwrap();
    Ok(
        json!({"status":"selected","index":index,"text":text,"message_sha256":artifact::sha(text.as_bytes()),"messages_sha256":stream,"check":check(&public,text)?}),
    )
}
pub fn make_artifact(public: &Value, messages: &Value, context: &Value) -> Result<Value> {
    let public = validate(public)?;
    if !artifact::exact(context, &CONTEXT)
        || !identity(&context["slot_id"], 128)
        || CONTEXT
            .iter()
            .filter(|k| **k != "slot_id")
            .any(|k| !context[*k].as_str().is_some_and(|s| artifact::hex(s, 64)))
    {
        return Err(artifact::invalid("invalid review context"));
    }
    let s = select(&public, messages)?;
    let mut b = context.clone();
    b["case_id"] = public["case_id"].clone();
    b["public_contract_sha256"] = contract_digest(&public)?.into();
    b["messages_sha256"] = s["messages_sha256"].clone();
    b["selected_message_index"] = s["index"].clone();
    b["selected_message_sha256"] = s["message_sha256"].clone();
    Ok(json!({"schema":"memory-answer-artifact-v1","binding":b,"selection":s}))
}
pub fn artifact_valid(public: &Value, a: &Value) -> Result<bool> {
    let public = validate(public)?;
    let s = &a["selection"];
    let b = &a["binding"];
    let keys = [
        "protocol_sha256",
        "slot_id",
        "verifier_sha256",
        "source_image_sha256",
        "case_id",
        "public_contract_sha256",
        "messages_sha256",
        "selected_message_index",
        "selected_message_sha256",
    ];
    if !artifact::exact(a, &["schema", "binding", "selection"])
        || a["schema"] != "memory-answer-artifact-v1"
        || !artifact::exact(b, &keys)
        || !artifact::exact(
            s,
            &[
                "status",
                "index",
                "text",
                "message_sha256",
                "messages_sha256",
                "check",
            ],
        )
        || b["case_id"] != public["case_id"]
        || b["public_contract_sha256"] != contract_digest(&public)?
        || !identity(&b["slot_id"], 128)
        || keys
            .iter()
            .filter(|k| !["slot_id", "case_id", "selected_message_index"].contains(k))
            .any(|k| !b[*k].as_str().is_some_and(|v| artifact::hex(v, 64)))
    {
        return Ok(false);
    }
    let Some(text) = s["text"].as_str() else {
        return Ok(false);
    };
    Ok(s["status"] == "selected"
        && s["index"] == b["selected_message_index"]
        && s["index"]
            .as_u64()
            .is_some_and(|n| n < public["limits"]["max_messages"].as_u64().unwrap())
        && s["messages_sha256"] == b["messages_sha256"]
        && text.len() <= public["limits"]["max_message_bytes"].as_u64().unwrap() as usize
        && marked(text)
        && s["message_sha256"] == artifact::sha(text.as_bytes())
        && s["message_sha256"] == b["selected_message_sha256"]
        && s["check"] == check(&public, text)?)
}
pub fn review(
    public: &Value,
    a: &Value,
    machine: Option<bool>,
    receipt: Option<&str>,
) -> Result<Value> {
    let valid = artifact_valid(public, a)?;
    let (mut status, mut verdict, mut reviewer) = ("missing", None::<String>, Value::Null);
    if !valid {
        status = "artifact-invalid";
    } else if let Some(raw) = receipt {
        match if raw.len() <= 16384 {
            artifact::strict(raw.as_bytes()).ok()
        } else {
            None
        } {
            Some(r)
                if artifact::exact(
                    &r,
                    &[
                        "schema",
                        "binding",
                        "reviewer_id",
                        "reviewer_kind",
                        "verdict",
                    ],
                ) && r["schema"] == "independent-answer-review-v1"
                    && identity(&r["reviewer_id"], 128)
                    && ["agent", "human"].contains(&r["reviewer_kind"].as_str().unwrap_or(""))
                    && ["consistent", "contradiction", "uncertain"]
                        .contains(&r["verdict"].as_str().unwrap_or("")) =>
            {
                if r["binding"] != a["binding"] {
                    status = "mismatch";
                } else {
                    verdict = r["verdict"].as_str().map(String::from);
                    status = match verdict.as_deref() {
                        Some("consistent") => "consistent",
                        Some("contradiction") => "contradiction",
                        _ => "uncertain",
                    };
                    reviewer = json!({"id":r["reviewer_id"],"claimed_kind":r["reviewer_kind"]});
                }
            }
            _ => status = "invalid",
        }
    }
    let correct = match machine {
        Some(false) => Some(false),
        Some(true)
            if valid
                && (a["selection"]["check"]["valid"] != true
                    || verdict.as_deref() == Some("contradiction")) =>
        {
            Some(false)
        }
        Some(true) if valid && verdict.as_deref() == Some("consistent") => Some(true),
        _ => None,
    };
    Ok(
        json!({"machine_correct":machine,"task_correct":correct,"review_status":status,"reviewer":reviewer,"authenticated":false,"human_confirmation":false,"semantic_truth_proven":false}),
    )
}
