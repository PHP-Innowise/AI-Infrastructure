//! Read-only worksheet preview preparation. Cached values and merged regions are retained.
mod format;
use crate::{Result, RuntimeError};
use infra_qa::ooxml::Package;
use serde_json::{Value, json};
use std::path::Path;
use xmltree::Element;
fn children<'a>(e: &'a Element, name: &str) -> Vec<&'a Element> {
    e.children
        .iter()
        .filter_map(|n| n.as_element())
        .filter(|e| e.name.rsplit(':').next() == Some(name))
        .collect()
}
fn child<'a>(e: &'a Element, name: &str) -> Option<&'a Element> {
    children(e, name).into_iter().next()
}
fn text(e: &Element) -> String {
    e.children
        .iter()
        .map(|n| match n {
            xmltree::XMLNode::Text(s) | xmltree::XMLNode::CData(s) => s.clone(),
            xmltree::XMLNode::Element(e) => text(e),
            _ => String::new(),
        })
        .collect()
}
fn nonempty_text(e: &Element) -> bool {
    e.children.iter().any(|node| match node {
        xmltree::XMLNode::Text(value) | xmltree::XMLNode::CData(value) => !value.is_empty(),
        xmltree::XMLNode::Element(child) => nonempty_text(child),
        _ => false,
    })
}
// Duplicate physical coordinates have no single raw/cache/provenance image.
// Refuse this ambiguous preview before cache projection; original bytes remain downloadable.
fn admit_cell_coordinates(sheet: &Element) -> Result<()> {
    let Some(data) = child(sheet, "sheetData") else {
        return Ok(());
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut visits = 0usize;
    for row in data
        .children
        .iter()
        .filter_map(xmltree::XMLNode::as_element)
        .filter(|row| row.name.rsplit(':').next() == Some("row"))
    {
        for cell in row
            .children
            .iter()
            .filter_map(xmltree::XMLNode::as_element)
            .filter(|cell| cell.name.rsplit(':').next() == Some("c"))
        {
            visits += 1;
            if visits > 200_000 {
                return Err(RuntimeError::invalid("worksheet physical cell bound"));
            }
            let coordinate = cell
                .attributes
                .get("r")
                .and_then(|value| coord(value))
                .ok_or_else(|| RuntimeError::invalid("invalid worksheet cell coordinate"))?;
            if !seen.insert(coordinate) {
                return Err(RuntimeError::invalid("duplicate worksheet cell coordinate"));
            }
        }
    }
    Ok(())
}
fn coord(s: &str) -> Option<(usize, usize)> {
    let split = s.find(|c: char| c.is_ascii_digit())?;
    let mut col = 0;
    for c in s[..split].bytes() {
        if !c.is_ascii_uppercase() {
            return None;
        }
        col = col * 26 + (c - b'A' + 1) as usize;
    }
    let row: usize = s[split..].parse().ok()?;
    if col == 0 || row == 0 {
        None
    } else {
        Some((row - 1, col - 1))
    }
}
pub fn read(path: &Path) -> Result<Vec<Value>> {
    project(Package::read(path).map_err(|e| RuntimeError::invalid(e.to_string()))?)
}
pub fn read_bytes(bytes: &[u8]) -> Result<Vec<Value>> {
    project(
        Package::from_reader(std::io::Cursor::new(bytes))
            .map_err(|e| RuntimeError::invalid(e.to_string()))?,
    )
}
fn project(book: Package) -> Result<Vec<Value>> {
    let workbook = book
        .part("xl/workbook.xml")
        .map_err(|e| RuntimeError::invalid(e.to_string()))?;
    let names = child(&workbook, "sheets")
        .map(|s| {
            children(s, "sheet")
                .iter()
                .filter_map(|e| e.attributes.get("name").cloned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| book.sheets.keys().cloned().collect());
    let mut formatting = format::Workbook::new(&book, &workbook);
    let mut result = vec![];
    let mut dense_slots_left = 200_000usize;
    for name in names {
        let mut rows = book
            .rows_bounded(&name, dense_slots_left)
            .map_err(|e| RuntimeError::invalid(e.to_string()))?;
        let used_slots = rows
            .iter()
            .try_fold(rows.len(), |total, row| total.checked_add(row.len()))
            .ok_or_else(|| RuntimeError::invalid("worksheet dense slot overflow"))?;
        dense_slots_left = dense_slots_left
            .checked_sub(used_slots)
            .ok_or_else(|| RuntimeError::invalid("worksheet dense slot bound"))?;
        let sheet = book
            .part(&book.sheets[&name])
            .map_err(|e| RuntimeError::invalid(e.to_string()))?;
        admit_cell_coordinates(&sheet)?;
        let mut formulas = serde_json::Map::new();
        if let Some(data) = child(&sheet, "sheetData") {
            for row in children(data, "row") {
                for cell in children(row, "c") {
                    let address = cell.attributes.get("r").map(String::as_str).unwrap_or("");
                    if let (Some(formula), Some((r, c))) = (child(cell, "f"), coord(address)) {
                        formulas.insert(format!("{r}:{c}"), json!(text(formula)));
                        let kind = cell.attributes.get("t").map(String::as_str).unwrap_or("n");
                        let supplied = child(cell, "v")
                            .is_some_and(|value| kind == "str" || nonempty_text(value))
                            || kind == "inlineStr" && child(cell, "is").is_some();
                        if supplied {
                            let cached = book.cached_value(cell);
                            if let Some(row) = rows.get_mut(r)
                                && let Some(cell) = row.get_mut(c)
                            {
                                *cell = cached;
                            }
                        }
                    }
                }
            }
        }
        let mut merges = vec![];
        if let Some(group) = child(&sheet, "mergeCells") {
            for merge in children(group, "mergeCell") {
                if let Some((a, b)) = merge.attributes.get("ref").and_then(|s| s.split_once(':'))
                    && let (Some((r, c)), Some((rr, cc))) = (coord(a), coord(b))
                    && rr >= r
                    && cc >= c
                {
                    merges.push(json!({"row":r,"column":c,"rows":rr-r+1,"columns":cc-c+1}));
                }
            }
        }
        for row in &mut rows {
            for value in row {
                if let Some(b) = value.as_bool() {
                    *value = json!(if b { "TRUE" } else { "FALSE" });
                }
            }
        }
        let projection = formatting.sheet(&sheet, &rows);
        let mut value = serde_json::Map::new();
        value.insert("name".into(), Value::String(name));
        value.insert(
            "rows".into(),
            Value::Array(rows.into_iter().map(Value::Array).collect()),
        );
        value.insert("merges".into(), Value::Array(merges));
        value.insert("formulas".into(), Value::Object(formulas));
        projection.attach(&mut value);
        result.push(Value::Object(value));
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
