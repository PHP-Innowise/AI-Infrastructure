//! A data-only decoder for the small LangGraph shape used by Fleet.
use crate::{HarnessError, Result};
use serde_json::{Map, Number, Value, json};
const LIMIT: usize = 16 * 1024 * 1024;
const TAG: &str = "$harness.legacy";
fn invalid() -> HarnessError {
    HarnessError::invalid(
        "Unsupported or malformed legacy checkpoint; its original bytes were preserved.",
    )
}
pub fn typed(kind: &str, bytes: &[u8]) -> Result<Value> {
    if bytes.len() > LIMIT {
        return Err(invalid());
    }
    match kind {
        "null" if bytes.is_empty() => Ok(Value::Null),
        "json" => legacy_json(crate::strict_json::parse(bytes, LIMIT)?, 0),
        "msgpack" => {
            let mut nodes = 0;
            decode(bytes, 0, &mut nodes)
        }
        _ => Err(invalid()),
    }
}
fn legacy_json(value: Value, depth: usize) -> Result<Value> {
    if depth > 128 {
        return Err(invalid());
    }
    match value {
        Value::Array(values) => Ok(Value::Array(
            values
                .into_iter()
                .map(|v| legacy_json(v, depth + 1))
                .collect::<Result<_>>()?,
        )),
        Value::Object(mut values) => {
            if values.contains_key(TAG) {
                return Err(invalid());
            }
            if values.get("lc") == Some(&json!(2))
                && values.get("type").and_then(Value::as_str) == Some("constructor")
            {
                if values.keys().any(|k| {
                    !["lc", "type", "id", "args", "kwargs", "method"].contains(&k.as_str())
                }) || values.get("method").is_some_and(|v| !v.is_null())
                {
                    return Err(invalid());
                }
                let id = values.remove("id").ok_or_else(invalid)?;
                let id = id.as_array().ok_or_else(invalid)?;
                if id.len() != 3 || id[0] != "langgraph" || id[1] != "types" {
                    return Err(invalid());
                }
                let class = id[2].as_str().ok_or_else(invalid)?.to_string();
                let args = legacy_json(values.remove("args").unwrap_or(json!([])), depth + 1)?;
                let kwargs = legacy_json(values.remove("kwargs").unwrap_or(json!({})), depth + 1)?;
                if args.as_array().is_none() || kwargs.as_object().is_none() {
                    return Err(invalid());
                }
                if args.as_array().is_some_and(|a| !a.is_empty()) {
                    if kwargs.as_object().is_some_and(|a| !a.is_empty()) {
                        return Err(invalid());
                    }
                    return constructor(1, "langgraph.types", &class, args);
                }
                return constructor(2, "langgraph.types", &class, kwargs);
            }
            let mut out = Map::new();
            for (k, v) in values {
                out.insert(k, legacy_json(v, depth + 1)?);
            }
            Ok(Value::Object(out))
        }
        value => Ok(value),
    }
}
fn decode(data: &[u8], depth: usize, nodes: &mut usize) -> Result<Value> {
    let mut d = Decoder {
        data,
        pos: 0,
        nodes,
    };
    let v = d.value(depth)?;
    if d.pos != data.len() {
        return Err(invalid());
    }
    Ok(v)
}
struct Decoder<'a, 'n> {
    data: &'a [u8],
    pos: usize,
    nodes: &'n mut usize,
}
impl Decoder<'_, '_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let end = self.pos.checked_add(n).ok_or_else(invalid)?;
        if end > self.data.len() {
            return Err(invalid());
        }
        let start = self.pos;
        self.pos = end;
        Ok(&self.data[start..end])
    }
    fn uint(&mut self, n: usize) -> Result<u64> {
        let mut out = 0;
        for b in self.take(n)? {
            out = (out << 8) | u64::from(*b);
        }
        Ok(out)
    }
    fn len(&mut self, n: usize) -> Result<usize> {
        let value = self.uint(n)?;
        let len = usize::try_from(value).map_err(|_| invalid())?;
        if len > LIMIT {
            return Err(invalid());
        }
        Ok(len)
    }
    fn string(&mut self, n: usize) -> Result<Value> {
        let text = std::str::from_utf8(self.take(n)?).map_err(|_| invalid())?;
        Ok(Value::String(text.into()))
    }
    fn array(&mut self, n: usize, depth: usize) -> Result<Value> {
        if n > 1_000_000 {
            return Err(invalid());
        }
        let mut out = Vec::new();
        for _ in 0..n {
            out.push(self.value(depth + 1)?);
        }
        Ok(Value::Array(out))
    }
    fn map(&mut self, n: usize, depth: usize) -> Result<Value> {
        if n > 1_000_000 {
            return Err(invalid());
        }
        let mut out = Map::new();
        for _ in 0..n {
            let key = self
                .value(depth + 1)?
                .as_str()
                .ok_or_else(invalid)?
                .to_string();
            if key == TAG || out.contains_key(&key) {
                return Err(invalid());
            }
            out.insert(key, self.value(depth + 1)?);
        }
        Ok(Value::Object(out))
    }
    fn ext(&mut self, n: usize, depth: usize) -> Result<Value> {
        let code = self.uint(1)? as i8;
        let bytes = self.take(n)?.to_vec();
        let inner = decode(&bytes, depth + 1, self.nodes)?;
        let fields = inner.as_array().ok_or_else(invalid)?;
        if fields.len() != 3 {
            return Err(invalid());
        }
        let module = fields[0].as_str().ok_or_else(invalid)?;
        let class = fields[1].as_str().ok_or_else(invalid)?;
        constructor(code, module, class, fields[2].clone())
    }
    fn value(&mut self, depth: usize) -> Result<Value> {
        if depth > 128 || *self.nodes >= 1_000_000 {
            return Err(invalid());
        }
        *self.nodes += 1;
        let code = self.uint(1)? as u8;
        match code {
            0x00..=0x7f => Ok(u64::from(code).into()),
            0xe0..=0xff => Ok(i64::from(code as i8).into()),
            0x80..=0x8f => self.map(usize::from(code & 15), depth),
            0x90..=0x9f => self.array(usize::from(code & 15), depth),
            0xa0..=0xbf => self.string(usize::from(code & 31)),
            0xc0 => Ok(Value::Null),
            0xc2 => Ok(false.into()),
            0xc3 => Ok(true.into()),
            0xca => {
                let bits = self.uint(4)? as u32;
                Number::from_f64(f32::from_bits(bits) as f64)
                    .map(Value::Number)
                    .ok_or_else(invalid)
            }
            0xcb => {
                let bits = self.uint(8)?;
                Number::from_f64(f64::from_bits(bits))
                    .map(Value::Number)
                    .ok_or_else(invalid)
            }
            0xcc => Ok(self.uint(1)?.into()),
            0xcd => Ok(self.uint(2)?.into()),
            0xce => Ok(self.uint(4)?.into()),
            0xcf => Ok(self.uint(8)?.into()),
            0xd0 => Ok((self.uint(1)? as i8 as i64).into()),
            0xd1 => Ok((self.uint(2)? as i16 as i64).into()),
            0xd2 => Ok((self.uint(4)? as i32 as i64).into()),
            0xd3 => Ok((self.uint(8)? as i64).into()),
            0xd9 => {
                let n = self.len(1)?;
                self.string(n)
            }
            0xda => {
                let n = self.len(2)?;
                self.string(n)
            }
            0xdb => {
                let n = self.len(4)?;
                self.string(n)
            }
            0xdc => {
                let n = self.len(2)?;
                self.array(n, depth)
            }
            0xdd => {
                let n = self.len(4)?;
                self.array(n, depth)
            }
            0xde => {
                let n = self.len(2)?;
                self.map(n, depth)
            }
            0xdf => {
                let n = self.len(4)?;
                self.map(n, depth)
            }
            0xd4 => self.ext(1, depth),
            0xd5 => self.ext(2, depth),
            0xd6 => self.ext(4, depth),
            0xd7 => self.ext(8, depth),
            0xd8 => self.ext(16, depth),
            0xc7 => {
                let n = self.len(1)?;
                self.ext(n, depth)
            }
            0xc8 => {
                let n = self.len(2)?;
                self.ext(n, depth)
            }
            0xc9 => {
                let n = self.len(4)?;
                self.ext(n, depth)
            }
            _ => Err(invalid()),
        }
    }
}
fn constructor(code: i8, module: &str, class: &str, value: Value) -> Result<Value> {
    if code == 0 && module == "builtins" && matches!(class, "set" | "frozenset") {
        let array = value.as_array().ok_or_else(invalid)?;
        if array
            .iter()
            .any(|v| !v.is_string() && !v.is_number() && !v.is_boolean() && !v.is_null())
        {
            return Err(invalid());
        }
        return Ok(json!({(TAG):"set","items":array}));
    }
    if module != "langgraph.types" {
        return Err(invalid());
    }
    match (code, class) {
        (1, "Send") => {
            let args = value.as_array().ok_or_else(invalid)?;
            if !(2..=3).contains(&args.len())
                || args.len() == 3 && !args[2].is_null()
                || args[0] != "review"
                || !args[1].is_object()
            {
                return Err(invalid());
            }
            let arg = args[1].as_object().ok_or_else(invalid)?;
            if arg.keys().any(|k| {
                !["scope", "task_id", "thread_id", "lens", "budget_usd"].contains(&k.as_str())
            }) || ["scope", "task_id", "thread_id", "lens"]
                .iter()
                .any(|k| !arg.get(*k).is_some_and(Value::is_string))
            {
                return Err(invalid());
            }
            if !arg.get("budget_usd").is_some_and(|v| {
                v.is_null() || v.as_f64().is_some_and(|n| n.is_finite() && n >= 0.)
            }) {
                return Err(invalid());
            }
            Ok(json!({(TAG):"send","node":"review","arg":args[1]}))
        }
        (2, "Interrupt") => {
            let map = value.as_object().ok_or_else(invalid)?;
            if !map.contains_key("value")
                || map
                    .keys()
                    .any(|k| !["value", "id", "ns", "when", "resumable"].contains(&k.as_str()))
            {
                return Err(invalid());
            }
            let id = if let Some(id) = map.get("id") {
                id.as_str().ok_or_else(invalid)?.to_owned()
            } else if let Some(ns) = map.get("ns") {
                let ns = ns
                    .as_array()
                    .ok_or_else(invalid)?
                    .iter()
                    .map(|v| v.as_str().ok_or_else(invalid))
                    .collect::<Result<Vec<_>>>()?;
                format!(
                    "{:032x}",
                    xxhash_rust::xxh3::xxh3_128(ns.join("|").as_bytes())
                )
            } else {
                "placeholder-id".into()
            };
            Ok(json!({(TAG):"interrupt","id":id,"value":map["value"]}))
        }
        (2, "PregelTask") => {
            let map = value.as_object().ok_or_else(invalid)?;
            if map.keys().any(|k| {
                ![
                    "id",
                    "name",
                    "path",
                    "error",
                    "interrupts",
                    "state",
                    "result",
                ]
                .contains(&k.as_str())
            }) || !map.get("id").is_some_and(Value::is_string)
                || !map.get("name").and_then(Value::as_str).is_some_and(node)
                || !map.get("path").is_some_and(Value::is_array)
                || map.get("state").is_some_and(|v| !v.is_null())
                || map
                    .get("error")
                    .is_some_and(|v| !v.is_null() && !v.is_string())
            {
                return Err(invalid());
            }
            Ok(json!({(TAG):"task","data":map}))
        }
        (2, "Command") => {
            let map = value.as_object().ok_or_else(invalid)?;
            if map
                .keys()
                .any(|k| !["graph", "update", "resume", "goto"].contains(&k.as_str()))
                || map.get("graph").is_some_and(|v| !v.is_null())
                || map.get("update").is_some_and(|v| !v.is_null())
                || map
                    .get("goto")
                    .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
                || !map.get("resume").is_some_and(Value::is_boolean)
            {
                return Err(invalid());
            }
            Ok(json!({(TAG):"command","resume":map["resume"]}))
        }
        _ => Err(invalid()),
    }
}
pub fn node(value: &str) -> bool {
    matches!(
        value,
        "__start__" | "scope" | "review" | "collect" | "gate" | "record"
    )
}
pub fn tag(value: &Value, name: &str) -> bool {
    value.get(TAG).and_then(Value::as_str) == Some(name)
}
