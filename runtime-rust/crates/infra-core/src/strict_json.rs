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
