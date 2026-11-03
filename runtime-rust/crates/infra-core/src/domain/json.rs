//! Python-compatible JSON values and deterministic persisted identities.
use crate::{Result, RuntimeError};
use serde_json::{Map, Number, Value};

pub fn parse(text: &str) -> Result<Value> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        at: 0,
    };
    let value = parser.value(0)?;
    parser.space();
    if parser.at != parser.bytes.len() {
        return Err(RuntimeError::invalid("Trailing JSON data"));
    }
    Ok(value)
}
struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl Parser<'_> {
    fn space(&mut self) {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|b| b" \r\n\t".contains(b))
        {
            self.at += 1;
        }
    }
    fn value(&mut self, depth: usize) -> Result<Value> {
        if depth > 512 {
            return Err(RuntimeError::invalid("JSON nesting limit exceeded"));
        }
        self.space();
        match self.bytes.get(self.at).copied() {
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b'{') => {
                self.at += 1;
                self.space();
                let mut object = Map::new();
                if self.take(b'}') {
                    return Ok(Value::Object(object));
                }
                loop {
                    self.space();
                    let key = self.string()?;
                    self.space();
                    if !self.take(b':') {
                        return Err(RuntimeError::invalid("Invalid JSON object"));
                    }
                    let value = self.value(depth + 1)?;
                    object.insert(key, value);
                    self.space();
                    if self.take(b'}') {
                        break;
                    }
                    if !self.take(b',') {
                        return Err(RuntimeError::invalid("Invalid JSON object"));
                    }
                }
                Ok(Value::Object(object))
            }
            Some(b'[') => {
                self.at += 1;
                self.space();
                let mut array = Vec::new();
                if self.take(b']') {
                    return Ok(Value::Array(array));
                }
                loop {
                    array.push(self.value(depth + 1)?);
                    self.space();
                    if self.take(b']') {
                        break;
                    }
                    if !self.take(b',') {
                        return Err(RuntimeError::invalid("Invalid JSON array"));
                    }
                }
                Ok(Value::Array(array))
            }
            Some(_) => {
                let start = self.at;
                while self
                    .bytes
                    .get(self.at)
                    .is_some_and(|b| !b" \r\n\t,]}:".contains(b))
                {
                    self.at += 1;
                }
                let token = std::str::from_utf8(&self.bytes[start..self.at])
                    .map_err(|_| RuntimeError::invalid("Invalid JSON encoding"))?;
                match token {
                    "null" => Ok(Value::Null),
                    "true" => Ok(Value::Bool(true)),
                    "false" => Ok(Value::Bool(false)),
                    "NaN" | "Infinity" | "-Infinity" => {
                        Ok(Value::Number(Number::from_string_unchecked(token.into())))
                    }
                    _ => {
                        let value: Value = serde_json::from_str(token)?;
                        if value.is_number() && token.contains(['.', 'e', 'E']) {
                            let float = token
                                .parse::<f64>()
                                .map_err(|_| RuntimeError::invalid("Invalid JSON number"))?;
                            return Ok(Value::Number(match Number::from_f64(float) {
                                Some(number) => number,
                                None => Number::from_string_unchecked(
                                    if float.is_sign_negative() {
                                        "-Infinity"
                                    } else {
                                        "Infinity"
                                    }
                                    .into(),
                                ),
                            }));
                        }
                        if token == "-0" {
                            Ok(Value::Number(0.into()))
                        } else {
                            Ok(value)
                        }
                    }
                }
            }
            None => Err(RuntimeError::invalid("Incomplete JSON")),
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
        let start = self.at;
        if !self.take(b'"') {
            return Err(RuntimeError::invalid("JSON key must be text"));
        }
        while let Some(byte) = self.bytes.get(self.at).copied() {
            self.at += 1;
            if byte == b'\\' {
                if self.at >= self.bytes.len() {
                    break;
                }
                self.at += 1;
            } else if byte == b'"' {
                return serde_json::from_slice(&self.bytes[start..self.at]).map_err(Into::into);
            }
        }
        Err(RuntimeError::invalid("Unterminated JSON string"))
    }
}

fn number(number: &Number, allow_nonfinite: bool) -> Result<String> {
    let raw = number.to_string();
    if ["NaN", "Infinity", "-Infinity"].contains(&raw.as_str()) {
        return if allow_nonfinite {
            Ok(raw)
        } else {
            Err(RuntimeError::invalid("Nonfinite JSON number"))
        };
    }
    if !raw.contains(['.', 'e', 'E']) {
        return Ok(raw);
    }
    let negative = raw.starts_with('-');
    let unsigned = raw.strip_prefix('-').unwrap_or(&raw);
    let (coefficient, exponent) = unsigned
        .split_once(['e', 'E'])
        .map(|(a, b)| (a, b.parse::<i32>().unwrap_or(0)))
        .unwrap_or((unsigned, 0));
    let decimal = coefficient.find('.').unwrap_or(coefficient.len()) as i32;
    let all = coefficient.replace('.', "");
    let leading = all.bytes().take_while(|b| *b == b'0').count();
    let significant = &all[leading..];
    if significant.is_empty() {
        return Ok(if negative { "-0.0" } else { "0.0" }.into());
    }
    let last = significant
        .bytes()
        .rposition(|b| b != b'0')
        .map(|i| i + 1)
        .unwrap_or(1);
    let digits = &significant[..last];
    let power = exponent + decimal - leading as i32 - 1;
    let sign = if negative { "-" } else { "" };
    if !(-4..16).contains(&power) {
        let tail = if digits.len() > 1 {
            format!(".{}", &digits[1..])
        } else {
            String::new()
        };
        return Ok(format!(
            "{sign}{}{tail}e{}{:02}",
            &digits[..1],
            if power < 0 { "-" } else { "+" },
            power.unsigned_abs()
        ));
    }
    let position = power + 1;
    if position <= 0 {
        Ok(format!(
            "{sign}0.{}{}",
            "0".repeat((-position) as usize),
            digits
        ))
    } else if position as usize >= digits.len() {
        Ok(format!(
            "{sign}{}{}.0",
            digits,
            "0".repeat(position as usize - digits.len())
        ))
    } else {
        Ok(format!(
            "{sign}{}.{}",
            &digits[..position as usize],
            &digits[position as usize..]
        ))
    }
}

pub fn serialize(
    value: &Value,
    sort: bool,
    ascii: bool,
    pretty: bool,
    allow_nonfinite: bool,
) -> Result<String> {
    fn encode(
        value: &Value,
        sort: bool,
        pretty: bool,
        nonfinite: bool,
        depth: usize,
    ) -> Result<String> {
        let pad = |level: usize| "  ".repeat(level);
        match value {
            Value::Null => Ok("null".into()),
            Value::Bool(value) => Ok(value.to_string()),
            Value::Number(value) => number(value, nonfinite),
            Value::String(value) => serde_json::to_string(value).map_err(Into::into),
            Value::Array(items) => {
                if items.is_empty() {
                    return Ok("[]".into());
                }
                let values = items
                    .iter()
                    .map(|item| {
                        encode(item, sort, pretty, nonfinite, depth + 1).map(|item| {
                            if pretty {
                                format!("{}{item}", pad(depth + 1))
                            } else {
                                item
                            }
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(if pretty {
                    format!("[\n{}\n{}]", values.join(",\n"), pad(depth))
                } else {
                    format!("[{}]", values.join(","))
                })
            }
            Value::Object(object) => {
                if object.is_empty() {
                    return Ok("{}".into());
                }
                let mut keys = object.keys().collect::<Vec<_>>();
                if sort {
                    keys.sort();
                }
                let values = keys
                    .into_iter()
                    .map(|key| {
                        let key_json = serde_json::to_string(key)?;
                        let item = encode(&object[key], sort, pretty, nonfinite, depth + 1)?;
                        Ok(if pretty {
                            format!("{}{key_json}: {item}", pad(depth + 1))
                        } else {
                            format!("{key_json}:{item}")
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(if pretty {
                    format!("{{\n{}\n{}}}", values.join(",\n"), pad(depth))
                } else {
                    format!("{{{}}}", values.join(","))
                })
            }
        }
    }
    let encoded = encode(value, sort, pretty, allow_nonfinite, 0)?;
    if !ascii {
        return Ok(encoded);
    }
    let mut output = String::new();
    for character in encoded.chars() {
        if character.is_ascii() {
            output.push(character);
        } else {
            let mut units = [0u16; 2];
            for unit in character.encode_utf16(&mut units) {
                output.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    Ok(output)
}

pub fn canonical(value: &Value) -> Result<String> {
    serialize(value, true, false, false, false)
}
pub fn sorted_pretty(value: &Value) -> Result<String> {
    serialize(value, true, false, true, true)
}

/// Python json.dumps default spacing, distinct from compact publication seals.
pub fn python_default(
    value: &Value,
    sort: bool,
    ascii: bool,
    allow_nonfinite: bool,
) -> Result<String> {
    let compact = serialize(value, sort, ascii, false, allow_nonfinite)?;
    let mut result = String::with_capacity(compact.len());
    let mut quoted = false;
    let mut escaped = false;
    for character in compact.chars() {
        result.push(character);
        if quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
        } else if character == '"' {
            quoted = true;
        } else if character == ',' || character == ':' {
            result.push(' ');
        }
    }
    Ok(result)
}

/// Recover readable top-level identity strings even when unrelated Python JSON
/// strings contain escaped lone surrogates. This projection never admits a
/// malformed governed record for mutation or serialization.
pub fn identity_strings(
    text: &str,
    keys: &[&str],
) -> Result<std::collections::BTreeMap<String, String>> {
    fn hex(bytes: &[u8]) -> Option<u16> {
        if bytes.len() != 4 {
            return None;
        }
        let mut n = 0u16;
        for b in bytes {
            n = n * 16
                + match b {
                    b'0'..=b'9' => (b - b'0') as u16,
                    b'a'..=b'f' => (b - b'a' + 10) as u16,
                    b'A'..=b'F' => (b - b'A' + 10) as u16,
                    _ => return None,
                };
        }
        Some(n)
    }
    let bytes = text.as_bytes();
    let mut clean = Vec::with_capacity(bytes.len());
    let mut tokens = Vec::<(usize, usize, bool)>::new();
    let mut at = 0usize;
    while at < bytes.len() {
        if bytes[at] != b'"' {
            clean.push(bytes[at]);
            at += 1;
            continue;
        }
        let start = at;
        let mut altered = false;
        clean.push(b'"');
        at += 1;
        while at < bytes.len() {
            let b = bytes[at];
            if b == b'"' {
                clean.push(b);
                at += 1;
                break;
            }
            if b == b'\\' {
                if bytes.get(at + 1) == Some(&b'u')
                    && let Some(unit) = bytes.get(at + 2..at + 6).and_then(hex)
                {
                    if (0xd800..=0xdbff).contains(&unit) {
                        if bytes.get(at + 6..at + 8) == Some(b"\\u")
                            && bytes
                                .get(at + 8..at + 12)
                                .and_then(hex)
                                .is_some_and(|v| (0xdc00..=0xdfff).contains(&v))
                        {
                            clean.extend_from_slice(&bytes[at..at + 12]);
                            at += 12;
                            continue;
                        }
                        clean.extend_from_slice(b"\\ufffd");
                        at += 6;
                        altered = true;
                        continue;
                    } else if (0xdc00..=0xdfff).contains(&unit) {
                        clean.extend_from_slice(b"\\ufffd");
                        at += 6;
                        altered = true;
                        continue;
                    }
                }
                clean.push(b);
                at += 1;
                if at < bytes.len() {
                    clean.push(bytes[at]);
                    at += 1;
                }
                continue;
            }
            clean.push(b);
            at += 1;
        }
        tokens.push((start, at, altered));
    }
    let clean =
        std::str::from_utf8(&clean).map_err(|_| RuntimeError::invalid("Invalid JSON encoding"))?;
    if !parse(clean)?.is_object() {
        return Err(RuntimeError::invalid("Identity metadata must be an object"));
    }
    let mut selected = std::collections::BTreeMap::new();
    let mut depth = 0usize;
    let mut expect_key = false;
    let mut pending = None::<String>;
    let mut token = 0usize;
    let mut at = 0;
    while at < bytes.len() {
        if token < tokens.len() && tokens[token].0 == at {
            let (start, end, altered) = tokens[token];
            if depth == 1 && expect_key {
                pending = if !altered {
                    serde_json::from_str::<String>(&text[start..end]).ok()
                } else {
                    None
                };
                if let Some(key) = &pending
                    && keys.contains(&key.as_str())
                {
                    selected.remove(key);
                }
                expect_key = false;
            } else if depth == 1
                && let Some(key) = pending.take()
                && keys.contains(&key.as_str())
                && !altered
                && let Ok(value) = serde_json::from_str::<String>(&text[start..end])
            {
                selected.insert(key, value);
            }
            at = end;
            token += 1;
            continue;
        }
        match bytes[at] {
            b'{' => {
                if depth == 1 {
                    pending = None
                }
                depth += 1;
                if depth == 1 {
                    expect_key = true
                }
            }
            b'[' => {
                if depth == 1 {
                    pending = None
                }
                depth += 1
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            b',' if depth == 1 => {
                expect_key = true;
                pending = None
            }
            b':' if depth == 1 => {}
            b' ' | b'\n' | b'\r' | b'\t' => {}
            _ if depth == 1 => pending = None,
            _ => {}
        }
        at += 1;
    }
    Ok(selected)
}
