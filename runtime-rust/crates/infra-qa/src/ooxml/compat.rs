//! Normalize only admitted worksheet coordinates/shared references in a parsed copy.
//! Formula translation changes A1 references, never evaluates expressions.
use super::{attr, child, child_mut, children, content, coord, letters, local, set_text};
use anyhow::{Context, Result, bail};
use regex::Regex;
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
    sync::LazyLock,
};
use xmltree::{Element, XMLNode};

/// Keep insertion local: existing nodes and foreign extension payloads are not sorted.
pub(super) fn insert_ordered(parent: &mut Element, value: Element) {
    let order: &[&str] = match local(&parent.name) {
        "worksheet" => &[
            "sheetPr",
            "dimension",
            "sheetViews",
            "sheetFormatPr",
            "cols",
            "sheetData",
            "sheetCalcPr",
            "sheetProtection",
            "protectedRanges",
            "scenarios",
            "autoFilter",
            "sortState",
            "dataConsolidate",
            "customSheetViews",
            "mergeCells",
            "phoneticPr",
            "conditionalFormatting",
            "dataValidations",
            "hyperlinks",
            "printOptions",
            "pageMargins",
            "pageSetup",
            "headerFooter",
            "rowBreaks",
            "colBreaks",
            "customProperties",
            "cellWatches",
            "ignoredErrors",
            "smartTags",
            "drawing",
            "legacyDrawing",
            "legacyDrawingHF",
            "drawingHF",
            "picture",
            "oleObjects",
            "controls",
            "webPublishItems",
            "tableParts",
            "extLst",
        ],
        "table" => &[
            "autoFilter",
            "sortState",
            "tableColumns",
            "tableStyleInfo",
            "extLst",
        ],
        "styleSheet" => &[
            "numFmts",
            "fonts",
            "fills",
            "borders",
            "cellStyleXfs",
            "cellXfs",
            "cellStyles",
            "dxfs",
            "tableStyles",
            "colors",
            "extLst",
        ],
        "workbook" => &[
            "fileVersion",
            "fileSharing",
            "workbookPr",
            "workbookProtection",
            "bookViews",
            "sheets",
            "functionGroups",
            "externalReferences",
            "definedNames",
            "calcPr",
            "oleSize",
            "customWorkbookViews",
            "pivotCaches",
            "smartTagPr",
            "smartTagTypes",
            "webPublishing",
            "fileRecoveryPr",
            "webPublishObjects",
            "extLst",
        ],
        "comments" => &["authors", "commentList", "extLst"],
        _ => &[],
    };
    let position = order
        .iter()
        .position(|name| *name == local(&value.name))
        .and_then(|rank| {
            parent.children.iter().position(|node| {
                node.as_element()
                    .and_then(|node| order.iter().position(|name| *name == local(&node.name)))
                    .is_some_and(|existing| existing > rank)
            })
        });
    parent.children.insert(
        position.unwrap_or(parent.children.len()),
        XMLNode::Element(value),
    );
}

/// Fill implicit coordinates before any row, identity, append, table or preview consumer.
/// Shared formulas are expanded at SOURCE coordinates before selected cells are moved.
// Match Package::read's existing source part/whole-package limits. These
// normalization limits apply to each parsed worksheet, not cumulative callers.
const NORMALIZED_FORMULA_BYTES: usize = 64 * 1024 * 1024;
const NORMALIZATION_WORK_BYTES: usize = 512 * 1024 * 1024;

pub(super) fn worksheet(sheet: &mut Element) -> Result<()> {
    worksheet_bounded(sheet, NORMALIZED_FORMULA_BYTES, NORMALIZATION_WORK_BYTES)
}

#[cfg(test)]
pub(super) fn worksheet_test_limits(sheet: &mut Element, output: usize, work: usize) -> Result<()> {
    worksheet_bounded(
        sheet,
        output.min(NORMALIZED_FORMULA_BYTES),
        work.min(NORMALIZATION_WORK_BYTES),
    )
}

fn worksheet_bounded(sheet: &mut Element, output_limit: usize, work_limit: usize) -> Result<()> {
    if local(&sheet.name) != "worksheet" {
        return Ok(());
    }
    let Some(data) = child_mut(sheet, "sheetData") else {
        return Ok(());
    };
    let mut row_number = 0usize;
    for row in data
        .children
        .iter_mut()
        .filter_map(XMLNode::as_mut_element)
        .filter(|row| local(&row.name) == "row")
    {
        row_number = match row.attributes.get("r") {
            Some(value) => value.parse().context("invalid worksheet row coordinate")?,
            None => row_number
                .checked_add(1)
                .context("worksheet row overflow")?,
        };
        if row_number == 0 || row_number > 1_048_576 {
            bail!("worksheet row coordinate out of range");
        }
        row.attributes
            .entry("r".into())
            .or_insert_with(|| row_number.to_string());
        let mut column = 0usize;
        for cell in row
            .children
            .iter_mut()
            .filter_map(XMLNode::as_mut_element)
            .filter(|cell| local(&cell.name) == "c")
        {
            column = if let Some(address) = cell.attributes.get("r") {
                coord(address)?.0
            } else {
                let next = column.checked_add(1).context("worksheet column overflow")?;
                if next > 16_384 {
                    bail!("worksheet cell coordinate out of range");
                }
                cell.attributes
                    .insert("r".into(), format!("{}{row_number}", letters(next)));
                next
            };
        }
    }
    let mut masters = BTreeMap::<String, (String, String, usize)>::new();
    let mut has_shared = false;
    let (mut master_bytes, mut work_bytes) = (0usize, 0usize);
    for row in children(data, "row") {
        for cell in children(row, "c") {
            if let Some(formula) = child(cell, "f")
                && formula
                    .attributes
                    .get("t")
                    .is_some_and(|value| value == "shared")
            {
                has_shared = true;
                // Count borrowed text before content() can clone a large master.
                let length = formula_text_bytes(formula)?;
                if length == 0 {
                    continue;
                }
                let index = shared_index(formula)?;
                if masters.contains_key(&index) {
                    continue;
                }
                master_bytes = master_bytes
                    .checked_add(length)
                    .context("shared formula byte overflow")?;
                if master_bytes > output_limit {
                    bail!("shared formula expansion byte bound");
                }
                let scan = length
                    .checked_mul(2)
                    .and_then(|value| value.checked_add(1))
                    .context("shared formula work overflow")?;
                charge_work(&mut work_bytes, scan, work_limit)?;
                let expression = content(formula);
                let expanded_bound = translated_formula_bound(&expression)?;
                masters.insert(index, (attr(cell, "r"), expression, expanded_bound));
            }
        }
    }
    // An empty master table can still have invalid followers to reject.
    if !has_shared {
        return Ok(());
    }
    // Admit ALL copies and translation work before changing any shared formula.
    // Quoted literal text is charged once per copy, not an arbitrary reference
    // growth multiplier; other bytes conservatively allow the longest A1 axes.
    let mut expanded_bytes = 0usize;
    for row in children(data, "row") {
        for cell in children(row, "c") {
            if let Some(formula) = child(cell, "f")
                && formula
                    .attributes
                    .get("t")
                    .is_some_and(|value| value == "shared")
            {
                let index = shared_index(formula)?;
                let (origin, expression, bound) =
                    masters.get(&index).context("shared formula lacks master")?;
                let same = cell.attributes.get("r") == Some(origin);
                let copy = if same { expression.len() } else { *bound };
                expanded_bytes = expanded_bytes
                    .checked_add(copy)
                    .context("shared formula byte overflow")?;
                if expanded_bytes > output_limit {
                    bail!("shared formula expansion byte bound");
                }
                let work = expression
                    .len()
                    .checked_mul(if same { 1 } else { 8 })
                    .and_then(|value| value.checked_add(1))
                    .context("shared formula work overflow")?;
                charge_work(&mut work_bytes, work, work_limit)?;
            }
        }
    }
    for row in data
        .children
        .iter_mut()
        .filter_map(XMLNode::as_mut_element)
        .filter(|row| local(&row.name) == "row")
    {
        for cell in row
            .children
            .iter_mut()
            .filter_map(XMLNode::as_mut_element)
            .filter(|cell| local(&cell.name) == "c")
        {
            let address = attr(cell, "r");
            if let Some(formula) = child_mut(cell, "f")
                && attr(formula, "t") == "shared"
            {
                let index = shared_index(formula)?;
                let (origin, expression, _) =
                    masters.get(&index).context("shared formula lacks master")?;
                let expression = translate_formula(expression, origin, &address)?;
                for key in ["t", "si", "ref"] {
                    formula.attributes.remove(key);
                }
                set_text(formula, expression);
            }
        }
    }
    Ok(())
}

fn charge_work(current: &mut usize, additional: usize, maximum: usize) -> Result<()> {
    let next = current
        .checked_add(additional)
        .context("shared formula work overflow")?;
    if next > maximum {
        bail!("shared formula normalization work bound");
    }
    *current = next;
    Ok(())
}

fn formula_text_bytes(element: &Element) -> Result<usize> {
    element.children.iter().try_fold(0usize, |total, node| {
        let bytes = match node {
            XMLNode::Text(text) | XMLNode::CData(text) => text.len(),
            XMLNode::Element(element) => formula_text_bytes(element)?,
            _ => 0,
        };
        total
            .checked_add(bytes)
            .context("shared formula byte overflow")
    })
}

fn translated_formula_bound(expression: &str) -> Result<usize> {
    let bytes = expression.as_bytes();
    let (mut at, mut bound) = (0usize, 0usize);
    while at < bytes.len() {
        let (next, additional) = if bytes[at] == b'"' {
            let end = quoted_end(bytes, at, b'"')?;
            (end, end - at)
        } else {
            // A1 is two bytes; the largest valid translated cell is at most
            // twelve including absolute markers. Eight per source byte also
            // covers whole-row/column ranges and retains all original text.
            (at + 1, 8)
        };
        bound = bound
            .checked_add(additional)
            .context("shared formula byte overflow")?;
        at = next;
    }
    Ok(bound)
}

fn shared_index(formula: &Element) -> Result<String> {
    let index = formula
        .attributes
        .get("si")
        .context("shared formula lacks index")?;
    index
        .parse::<u32>()
        .context("invalid shared formula index")?;
    Ok(index.clone())
}

static CELL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\$?[A-Za-z]{1,3})(\$?[1-9][0-9]*)$").expect("constant A1 reference")
});
static COLUMN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\$?[A-Za-z]{1,3}$").expect("constant column reference"));
static ROW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\$?[1-9][0-9]*$").expect("constant row reference"));

fn shift_axis(value: &str, delta: i64, column: bool) -> Result<String> {
    if value.starts_with('$') || delta == 0 {
        return Ok(value.into());
    }
    let old = if column {
        value.bytes().fold(0i64, |total, byte| {
            total * 26 + i64::from(byte.to_ascii_uppercase() - b'A' + 1)
        })
    } else {
        value
            .parse::<i64>()
            .context("formula row reference overflow")?
    };
    let shifted = old
        .checked_add(delta)
        .context("formula reference overflow")?;
    let maximum = if column { 16_384 } else { 1_048_576 };
    if shifted < 1 || shifted > maximum {
        bail!("translated shared formula reference out of range");
    }
    Ok(if column {
        letters(shifted as usize)
    } else {
        shifted.to_string()
    })
}

/// Delimit ranges and worksheet qualifiers outside quotes/structured-reference brackets.
fn separators(token: &str, separator: u8) -> Vec<usize> {
    let bytes = token.as_bytes();
    let (mut quote, mut depth, mut index) = (false, 0usize, 0usize);
    let mut result = vec![];
    while index < bytes.len() {
        match bytes[index] {
            b'\'' if depth == 0 => {
                if quote && bytes.get(index + 1) == Some(&b'\'') {
                    index += 2;
                    continue;
                }
                quote = !quote;
            }
            b'[' if !quote => depth += 1,
            b']' if !quote => depth = depth.saturating_sub(1),
            byte if byte == separator && !quote && depth == 0 => result.push(index),
            _ => {}
        }
        index += 1;
    }
    result
}

fn translate_operand(token: &str, row_delta: i64, column_delta: i64) -> Result<String> {
    let colons = separators(token, b':');
    let range = !colons.is_empty();
    let mut result = String::new();
    let mut start = 0;
    for end in colons.iter().copied().chain(std::iter::once(token.len())) {
        let part = &token[start..end];
        let split = separators(part, b'!')
            .last()
            .map_or(0, |position| position + 1);
        let (qualifier, address) = part.split_at(split);
        result.push_str(qualifier);
        if let Some(capture) = CELL.captures(address) {
            result.push_str(&shift_axis(&capture[1], column_delta, true)?);
            result.push_str(&shift_axis(&capture[2], row_delta, false)?);
        } else if range && COLUMN.is_match(address) {
            result.push_str(&shift_axis(address, column_delta, true)?);
        } else if range && ROW.is_match(address) {
            result.push_str(&shift_axis(address, row_delta, false)?);
        } else {
            // Defined names, errors and structured references are not A1 references.
            result.push_str(address);
        }
        if end < token.len() {
            result.push(':');
        }
        start = end + 1;
    }
    Ok(result)
}

fn quoted_end(bytes: &[u8], start: usize, quote: u8) -> Result<usize> {
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == quote {
            if bytes.get(index + 1) == Some(&quote) {
                index += 2;
                continue;
            }
            return Ok(index + 1);
        }
        index += 1;
    }
    bail!("unterminated shared formula quote")
}

/// The openpyxl compatibility seam is token/reference translation, not calculation.
/// Strings, functions, names and structured references keep their original spelling.
pub(super) fn translate_formula(
    expression: &str,
    origin: &str,
    destination: &str,
) -> Result<String> {
    let (old_column, old_row) = coord(origin)?;
    let (column, row) = coord(destination)?;
    if origin == destination {
        return Ok(expression.into());
    }
    let row_delta = row as i64 - old_row as i64;
    let column_delta = column as i64 - old_column as i64;
    let bytes = expression.as_bytes();
    let mut result = String::with_capacity(expression.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            let end = quoted_end(bytes, index, b'"')?;
            result.push_str(&expression[index..end]);
            index = end;
        } else if bytes[index].is_ascii_whitespace() || b"+-*/^&=><%;{},()".contains(&bytes[index])
        {
            result.push(char::from(bytes[index]));
            index += 1;
        } else {
            let start = index;
            while index < bytes.len() {
                match bytes[index] {
                    b'\'' => index = quoted_end(bytes, index, b'\'')?,
                    b'[' => {
                        let mut depth = 1usize;
                        index += 1;
                        while index < bytes.len() && depth > 0 {
                            if bytes[index] == b'[' {
                                depth += 1;
                            }
                            if bytes[index] == b']' {
                                depth -= 1;
                            }
                            index += 1;
                        }
                        if depth != 0 {
                            bail!("unterminated shared formula bracket");
                        }
                    }
                    byte if byte.is_ascii_whitespace() || b"+-*/^&=><%;{},()\"".contains(&byte) => {
                        break;
                    }
                    _ => index += 1,
                }
            }
            let token = &expression[start..index];
            // LOG10 and similar function names also resemble valid A1 references.
            if bytes.get(index) == Some(&b'(') {
                result.push_str(token);
            } else {
                result.push_str(&translate_operand(token, row_delta, column_delta)?);
            }
        }
    }
    Ok(result)
}

/// Resolve an output spelling without creating its missing directories. Existing
/// components are resolved before '..', so an input alias cannot bypass comparison.
pub(super) fn output_path(path: &Path) -> Result<PathBuf> {
    fn resolve(path: &Path, links: &mut usize) -> Result<PathBuf> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let mut result = PathBuf::new();
        for component in absolute.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    result.pop();
                }
                Component::Normal(name) => {
                    let next = result.join(name);
                    match fs::symlink_metadata(&next) {
                        Ok(image) if image.file_type().is_symlink() => {
                            *links += 1;
                            if *links > 40 {
                                bail!("output symlink resolution bound");
                            }
                            let target = fs::read_link(&next)?;
                            result = resolve(
                                &if target.is_absolute() {
                                    target
                                } else {
                                    result.join(target)
                                },
                                links,
                            )?;
                        }
                        Ok(_) => result = crate::spelling::canonicalize(next)?,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => result = next,
                        Err(error) => return Err(error.into()),
                    }
                }
                _ => result.push(component.as_os_str()),
            }
        }
        Ok(result)
    }
    resolve(path, &mut 0)
}
