use crate::strict;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
pub const LIMIT: u64 = i64::MAX as u64;
pub const KEYS: [&str; 5] = [
    "input_tokens",
    "output_tokens",
    "total_tokens",
    "cached_input_tokens",
    "reasoning_output_tokens",
];
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub reasoning_output_tokens: Option<u64>,
}
impl Usage {
    pub fn fields(&self) -> [Option<u64>; 5] {
        [
            self.input_tokens,
            self.output_tokens,
            self.total_tokens,
            self.cached_input_tokens,
            self.reasoning_output_tokens,
        ]
    }
    pub fn valid(&self) -> bool {
        let [i, o, t, c, r] = self.fields();
        if self.fields().iter().flatten().any(|v| *v > LIMIT)
            || t.is_some_and(|t| i.is_some_and(|i| i > t) || o.is_some_and(|o| o > t))
        {
            return false;
        }
        if let (Some(i), Some(o)) = (i, o)
            && i.checked_add(o)
                .is_none_or(|sum| sum > LIMIT || t.is_some_and(|t| sum != t))
        {
            return false;
        }
        if c.is_some_and(|c| i.is_none_or(|i| c > i)) || r.is_some_and(|r| o.is_none_or(|o| r > o))
        {
            return false;
        }
        true
    }
}
fn empty(size: Option<usize>, state: &str) -> Value {
    json!({"profile":"supplied-chat-completion-usage-v1","response_bytes":size,"usage_state":state,"usage":Usage::default(),"usage_scope":"supplied_response_only","provider_coverage":"UNKNOWN","physical_attempts":null,"usd_micros":null,"billing":"unknown","usage_verified":false})
}
fn number(v: Option<&Value>) -> Result<Option<u64>> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .filter(|v| *v <= LIMIT)
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("usage")),
    }
}
fn detail(v: Option<&Value>, allowed: &[&str], selected: &str) -> Result<Option<u64>> {
    if v.is_none() || v == Some(&Value::Null) {
        return Ok(None);
    }
    let m = v
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("usage"))?;
    if m.keys().any(|k| !allowed.contains(&k.as_str())) {
        bail!("unsupported");
    }
    for n in m.values() {
        number(Some(n))?;
    }
    number(m.get(selected))
}
pub fn completion_usage(raw: &[u8]) -> Value {
    let size = raw.len();
    if size > 524288 {
        return empty(Some(size), "unmeasured");
    }
    let body = match strict::decode(raw, 524288, 16, 4096) {
        Ok(v) => v,
        Err(e) => {
            return empty(
                Some(size),
                if e.to_string() == "input_bound" {
                    "unmeasured"
                } else {
                    "invalid"
                },
            );
        }
    };
    let mut pending = vec![&body];
    while let Some(v) = pending.pop() {
        match v {
            Value::Object(m) => {
                if m.len() > 64 {
                    return empty(Some(size), "unmeasured");
                }
                pending.extend(m.values());
            }
            Value::Array(a) => {
                if a.len() > 1024 {
                    return empty(Some(size), "unmeasured");
                }
                pending.extend(a);
            }
            Value::String(s) if s.chars().count() > 131072 => {
                return empty(Some(size), "unmeasured");
            }
            _ => (),
        }
    }
    if !body.is_object() || body["object"] != "chat.completion" || body.get("error").is_some() {
        return empty(Some(size), "unsupported");
    }
    let Some(usage) = body.get("usage").filter(|v| !v.is_null()) else {
        return empty(Some(size), "missing");
    };
    let parsed = (|| -> Result<Usage> {
        let m = usage.as_object().ok_or_else(|| anyhow::anyhow!("usage"))?;
        if m.keys().any(|k| {
            ![
                "prompt_tokens",
                "completion_tokens",
                "total_tokens",
                "prompt_tokens_details",
                "completion_tokens_details",
            ]
            .contains(&k.as_str())
        }) {
            bail!("unsupported");
        }
        let u = Usage {
            input_tokens: number(m.get("prompt_tokens"))?,
            output_tokens: number(m.get("completion_tokens"))?,
            total_tokens: number(m.get("total_tokens"))?,
            cached_input_tokens: detail(
                m.get("prompt_tokens_details"),
                &[
                    "cached_tokens",
                    "cache_write_tokens",
                    "audio_tokens",
                    "image_tokens",
                    "text_tokens",
                ],
                "cached_tokens",
            )?,
            reasoning_output_tokens: detail(
                m.get("completion_tokens_details"),
                &[
                    "reasoning_tokens",
                    "audio_tokens",
                    "text_tokens",
                    "accepted_prediction_tokens",
                    "rejected_prediction_tokens",
                ],
                "reasoning_tokens",
            )?,
        };
        if !u.valid() {
            bail!("usage");
        }
        Ok(u)
    })();
    match parsed {
        Ok(u) => {
            let state = if u.fields()[..3].iter().all(Option::is_some) {
                "complete"
            } else if u.fields().iter().any(Option::is_some) {
                "partial"
            } else {
                "missing"
            };
            let mut result = empty(Some(size), state);
            result["usage"] = json!(u);
            result
        }
        Err(e) => empty(
            Some(size),
            if e.to_string() == "unsupported" {
                "unsupported"
            } else {
                "invalid"
            },
        ),
    }
}
pub struct CompletionCapture {
    consumed: bool,
    observation: Value,
    decoder_state: &'static str,
}
impl Default for CompletionCapture {
    fn default() -> Self {
        Self {
            consumed: false,
            observation: empty(None, "unobserved"),
            decoder_state: "not_entered",
        }
    }
}
impl CompletionCapture {
    pub fn deliver<T, F>(&mut self, raw: &[u8], decoder: F) -> Result<T>
    where
        F: FnOnce(&[u8]) -> Result<T>,
    {
        if self.consumed {
            bail!("capture_consumed");
        }
        self.consumed = true;
        self.observation = completion_usage(raw);
        self.decoder_state = "running";
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decoder(raw))) {
            Ok(Ok(v)) => {
                self.decoder_state = "succeeded";
                Ok(v)
            }
            Ok(Err(e)) => {
                self.decoder_state = "failed";
                Err(e)
            }
            Err(payload) => {
                self.decoder_state = "cancelled";
                std::panic::resume_unwind(payload)
            }
        }
    }
    pub fn snapshot(&self) -> Value {
        json!({"consumed":self.consumed,"decoder_state":self.decoder_state,"observation":self.observation})
    }
    pub fn usage(&self) -> Usage {
        serde_json::from_value(self.observation["usage"].clone()).unwrap_or_default()
    }
}
