//! Session memory options and bounded automatic retrieval queries.
use crate::{HarnessError, Result, store};
use serde_json::{Value, json};
pub fn query(prompt: &str) -> Result<String> {
    let plain = prompt
        .chars()
        .map(|c| {
            if c == '\n' || c == '\t' || !c.is_control() {
                c
            } else {
                ' '
            }
        })
        .collect::<String>();
    let value = infra_core::domain::sanitize_automatic_query(&plain)?;
    let mut end = value.len().min(4000);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    Ok(value[..end].trim().to_owned())
}
fn text(value: &Value, name: &str, max: usize) -> Result<String> {
    let text = value
        .as_str()
        .ok_or_else(|| HarnessError::invalid(format!("Enter a valid {name}.")))?;
    if text.trim().is_empty()
        || text.len() > max
        || text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(HarnessError::invalid(format!("Enter a bounded {name}.")));
    }
    Ok(text.into())
}
pub fn validate(data: &Value, prompt: &str) -> Result<Value> {
    if data.is_null() {
        return Ok(Value::Null);
    }
    if !store::exact(
        data,
        &[],
        &[
            "bank",
            "task_id",
            "query",
            "create",
            "goal",
            "record_id",
            "review",
            "auto",
        ],
    ) {
        return Err(HarnessError::invalid("Invalid task context options."));
    }
    let review = data["review"].as_bool().unwrap_or(false);
    let auto = data["auto"].as_bool().unwrap_or(false);
    for key in ["review", "auto", "create"] {
        if data.get(key).is_some_and(|v| !v.is_boolean()) {
            return Err(HarnessError::invalid("Memory options must be booleans."));
        }
    }
    let mut options = data.clone();
    if auto {
        if ["task_id", "goal", "record_id", "create"]
            .iter()
            .any(|key| data.get(*key).is_some())
        {
            return Err(HarnessError::invalid(
                "An automatic task takes its ID and goal from the first message.",
            ));
        }
        if prompt.trim().is_empty() {
            return Err(HarnessError::invalid(
                "An automatic task needs its first message.",
            ));
        }
        let q = query(prompt)?;
        let line = q
            .lines()
            .find(|s| !s.trim().is_empty())
            .unwrap_or("Work on the project.");
        let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        let goal = if collapsed.chars().count() > 200 {
            let mut s = collapsed.chars().take(199).collect::<String>();
            while s.ends_with(char::is_whitespace) {
                s.pop();
            }
            s.push('…');
            s
        } else {
            collapsed
        };
        let expression = regex::Regex::new("[a-z0-9]+")
            .map_err(|_| HarnessError::invalid("Automatic task slug expression is invalid."))?;
        let lower = goal.to_lowercase();
        let mut slug = expression
            .find_iter(&lower)
            .take(5)
            .map(|m| m.as_str())
            .collect::<Vec<_>>()
            .join("-");
        slug.truncate(slug.len().min(40));
        while slug.ends_with('-') {
            slug.pop();
        }
        if slug.is_empty() {
            slug = "task".into();
        }
        options["task_id"] = format!("harness/{slug}-{}", &store::hexid()[..6]).into();
        options["goal"] = goal.into();
        options["create"] = true.into();
    }
    let bank = options["bank"].as_str().unwrap_or("");
    if bank.is_empty()
        || bank.starts_with('/')
        || bank.contains('\\')
        || bank.contains('\0')
        || bank
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
        || !(bank == "memory-bank" || bank.ends_with("/memory-bank"))
    {
        return Err(HarnessError::invalid("Select a project Memory Bank."));
    }
    let task = text(&options["task_id"], "task ID", 200)?.trim().to_owned();
    let expression = regex::Regex::new(r"^[\p{L}\p{N}][\p{L}\p{N}\p{M}_./#+-]{0,199}$")
        .map_err(|_| HarnessError::invalid("Task ID expression is invalid."))?;
    if !expression.is_match(&task) || task.split('/').any(|p| p == "..") {
        return Err(HarnessError::invalid("Enter a valid Brain task ID."));
    }
    let create = options["create"].as_bool().unwrap_or(false);
    let query = if review {
        text(&options["query"], "context query", 4000)?
    } else {
        let value = options
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or(prompt);
        if options.get("query").is_some_and(|v| !v.is_string()) || value.len() > 32000 {
            return Err(HarnessError::invalid("Enter a bounded context query."));
        }
        query(value)?
    };
    let mut result =
        json!({"bank":bank,"task_id":task,"query":query,"create":create,"review":review});
    if create || options.get("goal").is_some() {
        result["goal"] = text(&options["goal"], "task goal", 16000)?.into();
    }
    if let Some(id) = options.get("record_id") {
        let id = text(id, "task record ID", 36)?.trim().to_owned();
        if create || !infra_core::domain::is_uuid4(&id) {
            return Err(HarnessError::invalid(
                "Choose an existing task UUID or create a new task.",
            ));
        }
        result["record_id"] = id.into();
    }
    Ok(result)
}
/// A follow-up's change to a session's task context, which holds only
/// `review` and `query`: a reviewed query is the person's own bounded text, an
/// automatic one is screened as the first message's was. The task, bank and
/// goal stay as the session started them, and the next turn's context needs
/// approval again.
pub fn followup(brain: &Value, update: &Value) -> Result<Value> {
    if !brain.is_object() {
        return Err(HarnessError::invalid(
            "This session has no task context to change.",
        ));
    }
    if !store::exact(update, &[], &["review", "query"]) {
        return Err(HarnessError::invalid("Invalid task context options."));
    }
    let mut brain = brain.clone();
    let review = match update.get("review") {
        Some(value) => value
            .as_bool()
            .ok_or_else(|| HarnessError::invalid("Memory options must be booleans."))?,
        None => brain["review"].as_bool().unwrap_or(false),
    };
    brain["review"] = review.into();
    if let Some(raw) = update.get("query") {
        let value = raw
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Enter a bounded context query."))?;
        brain["query"] = if review {
            text(raw, "context query", 4000)?
        } else {
            if value.len() > 32000 {
                return Err(HarnessError::invalid("Enter a bounded context query."));
            }
            query(value)?
        }
        .into();
    }
    brain["approved"] = false.into();
    brain["context_id"] = Value::Null;
    Ok(brain)
}

#[cfg(test)]
mod followup_tests {
    use super::*;

    fn started() -> Value {
        json!({"bank":"memory-bank","task_id":"TASK-7","query":"retry policy","create":false,"review":true,"approved":true,"context_id":"ctx-1","record_id":"11111111-2222-4333-8444-555555555555"})
    }

    #[test]
    fn a_followup_changes_review_and_query_only() {
        let reviewed =
            followup(&started(), &json!({"query": "Session expiry\nand renewal"})).unwrap();
        assert_eq!(reviewed["query"], "Session expiry\nand renewal");
        assert_eq!(reviewed["review"], true);
        assert_eq!(reviewed["approved"], false);
        assert!(reviewed["context_id"].is_null());
        for kept in ["bank", "task_id", "record_id", "create"] {
            assert_eq!(reviewed[kept], started()[kept], "{kept}");
        }
        // An automatic query is screened as the first message's is.
        let automatic = followup(
            &started(),
            &json!({"review": false, "query": "retry with API_KEY=Sup3rS3cret99 for jane.doe@example.com"}),
        )
        .unwrap();
        assert_eq!(automatic["review"], false);
        let query = automatic["query"].as_str().unwrap();
        assert!(query.starts_with("retry with"), "{query}");
        assert!(
            !query.contains("Sup3rS3cret99") && !query.contains("jane.doe"),
            "{query}"
        );
        // Only the approval resets when nothing else is given.
        let same = followup(&started(), &json!({})).unwrap();
        assert_eq!(same["query"], "retry policy");
        assert_eq!(same["approved"], false);
    }

    #[test]
    fn a_followup_refuses_other_fields_and_unbounded_queries() {
        for update in [
            json!({"task_id": "TASK-8"}),
            json!({"bank": "other/memory-bank"}),
            json!({"review": "yes"}),
            json!({"query": 7}),
            json!({"query": ""}),
            json!({"query": "x".repeat(4001)}),
            json!({"query": "bell \u{7}"}),
        ] {
            assert!(followup(&started(), &update).is_err(), "{update}");
        }
        assert!(followup(&Value::Null, &json!({"query": "q"})).is_err());
    }
}
