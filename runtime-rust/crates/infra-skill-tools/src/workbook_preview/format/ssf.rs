//! Exact canonical0.20.3 table/normalization seam; no JS execution.
use super::child;
use std::{collections::BTreeMap, sync::Arc};
use xmltree::Element;

pub(super) const META_VISITS: usize = 4096;
pub(super) const CODE_BYTES: usize = 4096;
#[derive(Clone, PartialEq, Eq)]
pub(super) enum Code {
    Text(Arc<str>),
    Rejected(&'static str),
}
pub(super) struct Table {
    active: BTreeMap<u32, Code>,
    original: BTreeMap<u32, Code>,
    styles: Vec<u32>,
}
fn named<'a>(parent: &'a Element, name: &'a str) -> impl Iterator<Item = &'a Element> {
    parent
        .children
        .iter()
        .filter_map(xmltree::XMLNode::as_element)
        .filter(move |element| element.name.rsplit(':').next() == Some(name))
}
fn visit(left: &mut usize) -> Result<(), &'static str> {
    *left = left.checked_sub(1).ok_or("format-definition-limit")?;
    Ok(())
}
fn id(value: Option<&String>) -> Result<u32, &'static str> {
    let Some(value) = value else {
        return Ok(0);
    };
    if value.len() > 10 {
        return Err("invalid-style-sheet");
    }
    value.parse().map_err(|_| "invalid-style-sheet")
}
impl Table {
    pub(super) fn empty() -> Self {
        Self {
            active: BTreeMap::new(),
            original: BTreeMap::new(),
            styles: Vec::new(),
        }
    }
    pub(super) fn read(styles: Option<&Element>) -> Result<Self, &'static str> {
        let mut table = Self::empty();
        let Some(styles) = styles else {
            return Ok(table);
        };
        let mut visits = META_VISITS;
        if let Some(group) = child(styles, "numFmts") {
            for definition in named(group, "numFmt") {
                visit(&mut visits)?;
                let key = id(definition.attributes.get("numFmtId"))?;
                let code = match definition.attributes.get("formatCode") {
                    Some(code) if code.len() <= CODE_BYTES => Code::Text(Arc::from(code.as_str())),
                    Some(_) => Code::Rejected("format-length-limit"),
                    None => Code::Rejected("invalid-format-code"),
                };
                // Borrowed attribute length is checked BEFORE this bounded clone.
                table.original.insert(key, code.clone());
                if key == 0 {
                    continue;
                } // yv's General0 path ignores custom0.
                let mut loaded = key;
                if key > 392 {
                    loaded = 60;
                    for candidate in (61..=392).rev() {
                        visit(&mut visits)?;
                        if !table.original.contains_key(&candidate) && builtin(candidate).is_none()
                        {
                            loaded = candidate;
                            break;
                        }
                    }
                    table.original.insert(loaded, code.clone());
                }
                table.active.insert(loaded, code);
            }
        }
        if let Some(group) = child(styles, "cellXfs") {
            for style in named(group, "xf") {
                visit(&mut visits)?;
                let mut key = id(style.attributes.get("numFmtId"))?;
                if key > 392
                    && let Some(code) = table.original.get(&key)
                {
                    for candidate in (61..=392).rev() {
                        visit(&mut visits)?;
                        if table
                            .original
                            .get(&candidate)
                            .is_some_and(|other| other == code)
                        {
                            key = candidate;
                            break;
                        }
                    }
                }
                table.styles.push(key);
            }
        }
        Ok(table)
    }
    pub(super) fn style(&self, cell: &Element) -> (u32, Option<&'static str>) {
        let Some(raw) = cell.attributes.get("s") else {
            return (0, None);
        };
        let index = if raw.len() <= 10 {
            raw.parse::<usize>().ok()
        } else {
            None
        };
        match index.and_then(|index| self.styles.get(index)) {
            Some(id) => (*id, None),
            None => (0, Some("invalid-style-index")),
        }
    }
    pub(super) fn code(&self, key: u32) -> (Code, bool) {
        if key == 0 {
            return (Code::Text(Arc::from("General")), false);
        }
        if let Some(code) = self.active.get(&key) {
            return (code.clone(), false);
        }
        if let Some(Code::Rejected(reason)) = self.original.get(&key) {
            return (Code::Rejected(reason), false);
        }
        if let Some(code) = builtin(key).or_else(|| default_code(key)) {
            return (Code::Text(Arc::from(code)), false);
        }
        (Code::Text(Arc::from("General")), true)
    }
}
fn builtin(key: u32) -> Option<&'static str> {
    if key == 56 {
        return Some("\"上午/下午 \"hh\"時\"mm\"分\"ss\"秒 \"");
    }
    ooxml_numfmt::format_code_from_id(key)
}
fn default_code(key: u32) -> Option<&'static str> {
    Some(match key {
        5 | 63 => "\"$\"#,##0_);\\(\"$\"#,##0\\)",
        6 | 64 => "\"$\"#,##0_);[Red]\\(\"$\"#,##0\\)",
        7 | 65 => "\"$\"#,##0.00_);\\(\"$\"#,##0.00\\)",
        8 | 66 => "\"$\"#,##0.00_);[Red]\\(\"$\"#,##0.00\\)",
        23..=26 => "General",
        27..=31 | 36 | 50..=58 | 71..=72 => "m/d/yy",
        32..=35 | 77 => "h:mm:ss",
        41 => "_(* #,##0_);_(* (#,##0);_(* \"-\"_);_(@_)",
        42 => "_(\"$\"* #,##0_);_(\"$\"* (#,##0);_(\"$\"* \"-\"_);_(@_)",
        43 => "_(* #,##0.00_);_(* (#,##0.00);_(* \"-\"??_);_(@_)",
        44 => "_(\"$\"* #,##0.00_);_(\"$\"* (#,##0.00);_(\"$\"* \"-\"??_);_(@_)",
        59 => "0",
        60 => "0.00",
        61 => "#,##0",
        62 => "#,##0.00",
        67 => "0%",
        68 => "0.00%",
        69 => "# ?/?",
        70 => "# ??/??",
        73 => "d-mmm-yy",
        74 => "d-mmm",
        75 => "mmm-yy",
        76 => "h:mm",
        78 => "m/d/yy h:mm",
        79 => "mm:ss",
        80 => "[h]:mm:ss",
        81 => "mmss.0",
        _ => return None,
    })
}
fn push(output: &mut String, value: &str) -> Result<(), &'static str> {
    if output
        .len()
        .checked_add(value.len())
        .is_none_or(|length| length > CODE_BYTES)
    {
        return Err("format-length-limit");
    }
    output.push_str(value);
    Ok(())
}
/// Normalize the actual preview grammar BEFORE generic parsing can drop fills.
pub(super) fn normalize(code: &str) -> Result<String, &'static str> {
    if code.len() > CODE_BYTES {
        return Err("format-length-limit");
    }
    if code
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("General"))
    {
        return Ok("General".into());
    }
    let mut output = String::with_capacity(code.len());
    let mut input = code.char_indices().peekable();
    while let Some((at, ch)) = input.next() {
        match ch {
            'G' => {
                let Some(word) = code
                    .get(at..at + 7)
                    .filter(|word| word.eq_ignore_ascii_case("General"))
                else {
                    return Err("invalid-format-code");
                };
                push(&mut output, word)?;
                for _ in 0..6 {
                    input.next();
                }
            }
            'g' => return Err("unsupported-format"),
            '"' => {
                push(&mut output, "\"")?;
                let mut closed = false;
                for (_, next) in input.by_ref() {
                    let mut buf = [0; 4];
                    push(&mut output, next.encode_utf8(&mut buf))?;
                    if next == '"' {
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    return Err("invalid-format-code");
                }
            }
            '\\' => {
                let Some((_, next)) = input.next() else {
                    return Err("invalid-format-code");
                };
                if next.len_utf16() > 1 {
                    return Err("unsupported-format");
                }
                let mut buf = [0; 4];
                push(&mut output, "\\")?;
                push(&mut output, next.encode_utf8(&mut buf))?;
            }
            '_' => {
                if input.next().is_some_and(|(_, next)| next.len_utf16() > 1) {
                    return Err("unsupported-format");
                }
                push(&mut output, " ")?;
            }
            '*' => {
                if input
                    .peek()
                    .is_some_and(|(_, next)| matches!(*next, ' ' | '*'))
                {
                    input.next();
                }
                // Other following source characters stay ordinary syntax exactly once.
            }
            '[' => {
                push(&mut output, "[")?;
                let mut closed = false;
                for (_, next) in input.by_ref() {
                    let mut buf = [0; 4];
                    push(&mut output, next.encode_utf8(&mut buf))?;
                    if next == ']' {
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    return Err("invalid-format-code");
                }
            }
            ']' => return Err("invalid-format-code"),
            next if next.is_ascii_alphanumeric() || " ,.$-+/():!^&'~{}<>=€%#?;@".contains(next) =>
            {
                if next.is_ascii_alphabetic()
                    && !"MmDdYyHhSsEeBbAaPacfijklopqrtuvwxz".contains(next)
                {
                    return Err("invalid-format-code");
                }
                let mut buf = [0; 4];
                push(&mut output, next.encode_utf8(&mut buf))?;
            }
            _ => return Err("invalid-format-code"),
        }
    }
    if output.is_empty() {
        push(&mut output, "\"\"")?;
    }
    Ok(output)
}
