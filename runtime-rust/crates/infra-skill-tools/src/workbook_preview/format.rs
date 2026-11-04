//! Bounded worksheet display projection. Raw values/source bytes remain separate.
mod ssf;
use super::{child, children, coord, nonempty_text};
use infra_qa::ooxml::Package;
use ooxml_numfmt::{
    DateSystem, FormatOptions,
    bounded::{Budget, Format},
};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};
use xmltree::Element;

const CELLS: usize = 200_000;
const PROJECTION: usize = 33_554_432;
const WARNINGS: usize = 256;
const WARNING_BYTES: usize = WARNINGS * 256;
const CACHE: usize = 256;

#[derive(Default)]
pub(super) struct Sheet {
    display: Map<String, Value>,
    provenance: Map<String, Value>,
    warnings: Vec<Value>,
    suppressed: u64,
}
impl Sheet {
    pub(super) fn attach(self, target: &mut Map<String, Value>) {
        target.insert("display".into(), Value::Object(self.display));
        target.insert("display_provenance".into(), Value::Object(self.provenance));
        target.insert("format_warnings".into(), Value::Array(self.warnings));
        target.insert("format_warnings_suppressed".into(), json!(self.suppressed));
    }
}
#[derive(Clone)]
enum Cached {
    Ready(Arc<Format>, bool),
    Failed(&'static str),
}

pub(super) struct Workbook {
    table: ssf::Table,
    table_problem: Option<&'static str>,
    options: FormatOptions,
    budget: Budget,
    cache: BTreeMap<u32, Cached>,
    order: VecDeque<u32>,
    cells: usize,
    projection_left: usize,
    warnings_left: usize,
    degraded: Option<&'static str>,
}
fn key_len(row: usize, col: usize) -> usize {
    fn digits(mut value: usize) -> usize {
        let mut count = 1;
        while value >= 10 {
            value /= 10;
            count += 1;
        }
        count
    }
    digits(row) + 1 + digits(col)
}
impl Workbook {
    pub(super) fn new(book: &Package, workbook: &Element) -> Self {
        let epoch = child(workbook, "workbookPr")
            .and_then(|value| value.attributes.get("date1904"))
            .is_some_and(|value| matches!(value.as_str(), "1" | "true"));
        let (table, problem) = match book.preview_styles() {
            Ok(styles) => match ssf::Table::read(styles.as_ref()) {
                Ok(table) => (table, None),
                Err(code) => (ssf::Table::empty(), Some(code)),
            },
            Err(_) => (ssf::Table::empty(), Some("invalid-style-sheet")),
        };
        Self {
            table,
            table_problem: problem,
            options: FormatOptions {
                date_system: if epoch {
                    DateSystem::Date1904
                } else {
                    DateSystem::Date1900
                },
                fill_count: 0,
                ..FormatOptions::default()
            },
            budget: Budget::default(),
            cache: BTreeMap::new(),
            order: VecDeque::new(),
            cells: 0,
            projection_left: PROJECTION - WARNING_BYTES,
            warnings_left: WARNINGS,
            degraded: None,
        }
    }
    fn warn(
        &mut self,
        sheet: &mut Sheet,
        at: Option<(usize, usize)>,
        code: &'static str,
        format: Option<u32>,
    ) {
        if self.warnings_left == 0 {
            sheet.suppressed = sheet.suppressed.saturating_add(1);
            return;
        }
        // Fixed record fields/code lengths and bounded coordinates fit256bytes.
        // The whole warning reserve is withheld before display insertion.
        self.warnings_left -= 1;
        let cell = at.map(|(row, col)| format!("{row}:{col}"));
        sheet
            .warnings
            .push(json!({"code":code,"cell":cell,"num_fmt_id":format}));
    }
    fn commit(
        &mut self,
        sheet: &mut Sheet,
        at: (usize, usize),
        value: String,
        origin: Option<&'static str>,
    ) {
        // Six bytes per string byte bounds JSON escaping; keys are decimal/colon.
        // Provenance is a small fixed enum, reserved before key/String copies.
        let cost = value
            .len()
            .checked_mul(6)
            .and_then(|bytes| bytes.checked_add(key_len(at.0, at.1) + 8))
            .and_then(|bytes| bytes.checked_add(if origin.is_some() { 128 } else { 0 }));
        let Some(cost) = cost.filter(|cost| *cost <= self.projection_left) else {
            self.warn(sheet, Some(at), "format-output-limit", None);
            return;
        };
        self.projection_left -= cost;
        let key = format!("{}:{}", at.0, at.1);
        if let Some(origin) = origin {
            sheet
                .provenance
                .insert(key.clone(), Value::String(origin.into()));
        }
        sheet.display.insert(key, Value::String(value));
    }
    fn remember(&mut self, id: u32, value: Cached) -> Cached {
        if self.cache.len() >= CACHE
            && let Some(old) = self.order.pop_front()
        {
            self.cache.remove(&old);
        }
        self.order.push_back(id);
        self.cache.insert(id, value.clone());
        value
    }
    fn compiled(&mut self, id: u32) -> Cached {
        if let Some(value) = self.cache.get(&id) {
            return value.clone();
        }
        let (code, unknown) = self.table.code(id);
        let code = match code {
            ssf::Code::Text(code) => code,
            ssf::Code::Rejected(reason) => return self.remember(id, Cached::Failed(reason)),
        };
        // Normalization/failure scans also spend work before they inspect code.
        if let Err(error) = self.budget.charge_work(code.len() as u64 * 4 + 32) {
            self.degraded = Some(error.code());
            return self.remember(id, Cached::Failed(error.code()));
        }
        let normalized = match ssf::normalize(&code) {
            Ok(code) => code,
            Err(reason) => return self.remember(id, Cached::Failed(reason)),
        };
        let result = match Format::parse(&normalized, &mut self.budget) {
            Ok(format) => Cached::Ready(Arc::new(format), unknown),
            Err(error) => {
                if error.code() == "format-work-limit" {
                    self.degraded = Some(error.code());
                }
                Cached::Failed(error.code())
            }
        };
        self.remember(id, result)
    }
    fn number(
        &mut self,
        sheet: &mut Sheet,
        at: (usize, usize),
        id: u32,
        value: f64,
    ) -> Option<String> {
        let result = match self.compiled(id) {
            Cached::Ready(format, unknown) => {
                if unknown {
                    self.warn(sheet, Some(at), "unknown-format-id", Some(id));
                }
                format
                    .number(value, &self.options, &mut self.budget)
                    .map_err(|error| error.code())
            }
            Cached::Failed(code) => Err(code),
        };
        match result {
            Ok(value) => Some(value),
            Err(code) => {
                self.warn(sheet, Some(at), code, Some(id));
                if matches!(code, "format-work-limit" | "format-output-limit") {
                    self.degraded = Some(code);
                    return None;
                }
                // Canonical HTML's format_cell fallback is General. A failed
                // fallback is explicit degradation, never an invented value.
                if id != 0
                    && let Cached::Ready(general, _) = self.compiled(0)
                {
                    match general.number(value, &self.options, &mut self.budget) {
                        Ok(value) => return Some(value),
                        Err(error) => {
                            self.warn(sheet, Some(at), error.code(), Some(0));
                            self.degraded = Some(error.code());
                        }
                    }
                }
                None
            }
        }
    }
    fn boolean(
        &mut self,
        sheet: &mut Sheet,
        at: (usize, usize),
        id: u32,
        value: bool,
    ) -> Option<String> {
        let text = if value { "TRUE" } else { "FALSE" };
        match self.compiled(id) {
            Cached::Ready(format, unknown) => {
                if unknown {
                    self.warn(sheet, Some(at), "unknown-format-id", Some(id));
                }
                match format.text(text, &self.options, &mut self.budget) {
                    Ok(value) => Some(value),
                    Err(error) => {
                        self.warn(sheet, Some(at), error.code(), Some(id));
                        if matches!(error.code(), "format-work-limit" | "format-output-limit") {
                            self.degraded = Some(error.code());
                            None
                        } else {
                            Some(text.into())
                        }
                    }
                }
            }
            Cached::Failed(code) => {
                self.warn(sheet, Some(at), code, Some(id));
                Some(text.into())
            }
        }
    }
    pub(super) fn sheet(&mut self, source: &Element, rows: &[Vec<Value>]) -> Sheet {
        let mut result = Sheet::default();
        if let Some(problem) = self.table_problem {
            self.warn(&mut result, None, problem, None);
        }
        let Some(data) = child(source, "sheetData") else {
            return result;
        };
        for row in children(data, "row") {
            for cell in children(row, "c") {
                let Some(at) = cell.attributes.get("r").and_then(|address| coord(address)) else {
                    self.warn(&mut result, None, "invalid-cache-value", None);
                    continue;
                };
                if self.cells >= CELLS {
                    self.warn(&mut result, Some(at), "format-cell-limit", None);
                    continue;
                }
                self.cells += 1;
                let raw = rows.get(at.0).and_then(|row| row.get(at.1));
                let formula = child(cell, "f").is_some();
                let kind = cell.attributes.get("t").map(String::as_str).unwrap_or("n");
                let supplied = child(cell, "v").is_some_and(nonempty_text);
                let mut origin = None;
                if formula && !supplied && kind == "b" {
                    origin = Some("missing-bool-formula-cache:false-default");
                    self.warn(&mut result, Some(at), "formula-cache-missing", None);
                }
                if formula && !supplied && !matches!(kind, "str" | "inlineStr" | "b") {
                    // The exact default0.20.3 read→HTML oracle omits missing
                    // numeric caches (including empty v), never displays0.
                    self.warn(&mut result, Some(at), "formula-cache-missing", None);
                    self.commit(
                        &mut result,
                        at,
                        String::new(),
                        Some("missing-formula-cache:blank"),
                    );
                    continue;
                }
                if formula && child(cell, "v").is_none() && kind == "str" {
                    self.warn(&mut result, Some(at), "formula-cache-missing", None);
                    self.commit(
                        &mut result,
                        at,
                        String::new(),
                        Some("missing-text-formula-cache:empty"),
                    );
                    continue;
                }
                if formula && kind == "inlineStr" && child(cell, "is").is_none() {
                    self.warn(&mut result, Some(at), "formula-cache-missing", None);
                    self.commit(
                        &mut result,
                        at,
                        String::new(),
                        Some("missing-inline-formula-cache:empty"),
                    );
                    continue;
                }
                // Default cellHTML=true gives string cells h priority over w.
                // Preserve original text even when custom text formatting changed w.
                if matches!(kind, "s" | "str" | "inlineStr") {
                    continue;
                }
                if kind == "e" {
                    if raw.and_then(Value::as_str).is_none_or(|value| {
                        !matches!(
                            value,
                            "#NULL!"
                                | "#DIV/0!"
                                | "#VALUE!"
                                | "#REF!"
                                | "#NAME?"
                                | "#NUM!"
                                | "#N/A"
                                | "#GETTING_DATA"
                                | "#SPILL!"
                                | "#CALC!"
                        )
                    }) {
                        self.warn(&mut result, Some(at), "invalid-cache-value", None);
                        self.commit(
                            &mut result,
                            at,
                            String::new(),
                            Some("invalid-error-cache:no-scalar"),
                        );
                    }
                    continue;
                }
                if let Some(reason) = self.degraded {
                    self.warn(&mut result, Some(at), reason, None);
                    continue;
                }
                if let Err(error) = self.budget.charge_work(32) {
                    self.degraded = Some(error.code());
                    self.warn(&mut result, Some(at), error.code(), None);
                    continue;
                }
                let (id, problem) = self.table.style(cell);
                if let Some(problem) = problem {
                    self.warn(&mut result, Some(at), problem, Some(id));
                }
                let display = if kind == "b" {
                    let value = origin.is_none()
                        && raw.is_some_and(|value| value == "TRUE" || value == true);
                    self.boolean(&mut result, at, id, value)
                } else if matches!(kind, "n" | "") {
                    let value = raw
                        .and_then(Value::as_f64)
                        .filter(|value| value.is_finite());
                    match value {
                        Some(value) => self.number(&mut result, at, id, value),
                        None if raw.is_none_or(Value::is_null) => None,
                        None => {
                            self.warn(&mut result, Some(at), "invalid-cache-value", Some(id));
                            None
                        }
                    }
                } else {
                    self.warn(&mut result, Some(at), "unsupported-format", Some(id));
                    None
                };
                if let Some(display) = display {
                    if raw.and_then(Value::as_str) == Some(display.as_str()) {
                        continue;
                    }
                    self.commit(&mut result, at, display, origin);
                }
            }
        }
        result
    }
}
#[cfg(test)]
#[path = "format_tests.rs"]
mod tests;
