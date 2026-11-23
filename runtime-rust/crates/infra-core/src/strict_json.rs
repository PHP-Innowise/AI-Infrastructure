//! Strict bounded JSON for untrusted native protocol and custody inputs.
//! Objects are built here, rather than through serde's generic Value visitor:
//! serde_json's arbitrary_precision private number marker remains a literal
//! object member, independent of feature unification in the executable.
//! Legacy document/metadata parsing lives in domain::json and stays separate.
use crate::{ErrorKind, Result, RuntimeError};
use serde_json::{Map, Value};

fn refused(code: &str, message: &str) -> RuntimeError {
    RuntimeError::coded(
        ErrorKind::InvalidInput,
        format!("strict-json:{code}"),
        message,
    )
}

/// Decode one JSON value. The root has depth zero; keys do not consume value
/// nodes. An object, array or scalar consumes one node. Limits are checked
/// before allocating descendants. A fixed recursion ceiling also protects a
/// trusted caller accidentally supplying an impractical depth allowance.
pub fn parse(bytes: &[u8], max_bytes: usize, max_depth: usize, max_nodes: usize) -> Result<Value> {
    if bytes.len() > max_bytes {
        return Err(refused("byte-limit", "JSON body exceeds its byte limit."));
    }
    std::str::from_utf8(bytes)
        .map_err(|_| refused("encoding", "JSON input is not valid UTF-8."))?;
    let mut parser = Parser {
        bytes,
        at: 0,
        max_depth: max_depth.min(512),
        max_nodes,
        nodes: 0,
    };
    let value = parser.value(0)?;
    parser.space();
    if parser.at != bytes.len() {
        return Err(refused("syntax", "Trailing JSON data."));
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
    max_depth: usize,
    max_nodes: usize,
    nodes: usize,
}
impl Parser<'_> {
    fn space(&mut self) {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|b| b" \t\r\n".contains(b))
        {
            self.at += 1;
        }
    }
    fn take(&mut self, byte: u8) -> bool {
        if self.bytes.get(self.at) == Some(&byte) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn string(&mut self) -> Result<String> {
        let begin = self.at;
        if !self.take(b'"') {
            return Err(refused("syntax", "JSON object keys must be strings."));
        }
        while let Some(byte) = self.bytes.get(self.at).copied() {
            self.at += 1;
            if byte == b'\\' {
                if self.at == self.bytes.len() {
                    return Err(refused("encoding", "Incomplete JSON escape."));
                }
                self.at += 1;
            } else if byte == b'"' {
                return serde_json::from_slice::<String>(&self.bytes[begin..self.at])
                    .map_err(|_| refused("encoding", "Invalid JSON string or escape."));
            } else if byte < 0x20 {
                return Err(refused(
                    "encoding",
                    "Unescaped control character in JSON string.",
                ));
            }
        }
        Err(refused("syntax", "Unterminated JSON string."))
    }
    fn value(&mut self, depth: usize) -> Result<Value> {
        if depth > self.max_depth {
            return Err(refused("depth-limit", "JSON nesting limit exceeded."));
        }
        if self.nodes >= self.max_nodes {
            return Err(refused("node-limit", "JSON value count exceeded."));
        }
        self.nodes += 1;
        self.space();
        match self.bytes.get(self.at).copied() {
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b'{') => {
                self.at += 1;
                self.space();
                let mut members = Map::new();
                if self.take(b'}') {
                    return Ok(Value::Object(members));
                }
                loop {
                    self.space();
                    let key = self.string()?;
                    self.space();
                    if !self.take(b':') {
                        return Err(refused("syntax", "JSON object member has no colon."));
                    }
                    if members.contains_key(&key) {
                        return Err(refused("duplicate-key", "Duplicate JSON field."));
                    }
                    let value = self.value(depth + 1)?;
                    members.insert(key, value);
                    self.space();
                    if self.take(b'}') {
                        break;
                    }
                    if !self.take(b',') {
                        return Err(refused("syntax", "Invalid JSON object separator."));
                    }
                }
                Ok(Value::Object(members))
            }
            Some(b'[') => {
                self.at += 1;
                self.space();
                let mut values = Vec::new();
                if self.take(b']') {
                    return Ok(Value::Array(values));
                }
                loop {
                    values.push(self.value(depth + 1)?);
                    self.space();
                    if self.take(b']') {
                        break;
                    }
                    if !self.take(b',') {
                        return Err(refused("syntax", "Invalid JSON array separator."));
                    }
                }
                Ok(Value::Array(values))
            }
            Some(_) => {
                let begin = self.at;
                while self
                    .bytes
                    .get(self.at)
                    .is_some_and(|b| !b" \t\r\n,]}".contains(b))
                {
                    self.at += 1;
                }
                if begin == self.at {
                    return Err(refused("syntax", "Expected a JSON value."));
                }
                let token = std::str::from_utf8(&self.bytes[begin..self.at])
                    .map_err(|_| refused("encoding", "JSON input is not valid UTF-8."))?;
                if ["NaN", "Infinity", "-Infinity"].contains(&token) {
                    return Err(refused("nonfinite", "Nonfinite JSON number."));
                }
                // Only a scalar token is passed to serde; no object can reach
                // its private-number map representation through this route.
                let _: Value = serde_json::from_str(token)
                    .map_err(|_| refused("syntax", "Invalid JSON scalar."))?;
                let value = crate::domain::json::parse(token)
                    .map_err(|_| refused("syntax", "Invalid JSON scalar."))?;
                if value.is_number()
                    && token.contains(['.', 'e', 'E'])
                    && value.as_f64().is_none_or(|value| !value.is_finite())
                {
                    return Err(refused("nonfinite", "Nonfinite JSON number."));
                }
                Ok(value)
            }
            None => Err(refused("syntax", "Incomplete JSON input.")),
        }
    }
}

/// Computes the lowercase hex SHA-256 digest of a JSON value serialized in canonical form.
pub fn canonical_sha256(value: &Value) -> Result<String> {
    let encoded = crate::domain::json::canonical(value)?;
    Ok(crate::query::sha256(encoded.as_bytes()))
}

/// Computes the canonical SHA-256 body digest of a sealed JSON object excluding `"seal_sha256"`.
pub fn seal_digest(protocol: &Value) -> Result<String> {
    let mut body = protocol.clone();
    if let Some(members) = body.as_object_mut() {
        members.remove("seal_sha256");
    }
    canonical_sha256(&body)
}

/// Seals a JSON report object in place at its top-level keys (`schema`, `input_digest`,
/// `sealed_at`, `seal_sha256`), matching the `treatments` / `sealed_ab` envelope convention.
pub fn seal_envelope(schema: &str, mut report: Value, input_digest: &str) -> Result<Value> {
    let obj = report.as_object_mut().ok_or_else(|| {
        refused(
            "envelope-type",
            "Sealed envelope report must be a JSON object.",
        )
    })?;
    obj.insert("schema".into(), Value::String(schema.into()));
    obj.insert("input_digest".into(), Value::String(input_digest.into()));
    obj.insert(
        "sealed_at".into(),
        Value::String(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
    );
    let digest = seal_digest(&Value::Object(obj.clone()))?;
    obj.insert("seal_sha256".into(), Value::String(digest));
    Ok(report)
}

/// Verifies that a sealed JSON report object carries a matching `"seal_sha256"` over all
/// remaining top-level and nested keys.
pub fn verify_sealed_digest(sealed: &Value, mismatch_message: &str) -> Result<()> {
    if !sealed.is_object() {
        return Err(RuntimeError::invalid(mismatch_message));
    }
    let expected = seal_digest(sealed)?;
    if sealed["seal_sha256"].as_str() != Some(expected.as_str()) {
        return Err(RuntimeError::invalid(mismatch_message));
    }
    Ok(())
}

#[cfg(test)]
mod sealed_envelope_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_sha256_and_sealed_envelope_detect_top_level_and_nested_tampering() {
        let a = json!({"b": 2, "a": {"y": true, "x": 1}});
        let b = json!({"a": {"x": 1, "y": true}, "b": 2});
        assert_eq!(canonical_sha256(&a).unwrap(), canonical_sha256(&b).unwrap());

        let input_digest = canonical_sha256(&json!({"case": 1})).unwrap();
        let mut sealed = seal_envelope(
            "test-report-v1",
            json!({"horizon": 2, "metrics": {"tokens": 42}}),
            &input_digest,
        )
        .unwrap();
        assert_eq!(sealed["schema"], "test-report-v1");
        assert_eq!(sealed["input_digest"], input_digest);
        verify_sealed_digest(&sealed, "tampered").unwrap();

        let mut top_tampered = sealed.clone();
        top_tampered["horizon"] = json!(3);
        assert!(verify_sealed_digest(&top_tampered, "tampered").is_err());

        sealed["metrics"]["tokens"] = json!(0);
        assert!(verify_sealed_digest(&sealed, "tampered").is_err());
        assert!(verify_sealed_digest(&json!(["not-an-object"]), "tampered").is_err());
        assert!(seal_envelope("test-report-v1", json!([]), &input_digest).is_err());
    }
}
