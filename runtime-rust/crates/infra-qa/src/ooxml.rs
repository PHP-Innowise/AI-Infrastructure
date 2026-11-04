//! Preserve the original OPC package; mutate only explicitly named XML parts.
mod compat;
use crate::common::*;
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Cursor, Read, Write},
    path::Path,
};
use xmltree::{Element, XMLNode};
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKGREL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
pub const SPECS: &[(&str, usize, usize, &str, &str)] = &[
    ("Test Cases", 92, 18, "A1:R93", "tblTestCases"),
    ("Test Runs", 75, 30, "A1:AD76", "tblTestRuns"),
    ("Quality Metrics", 71, 19, "A1:S72", "tblQualityMetrics"),
    (
        "Efficiency & Tokens",
        75,
        24,
        "A1:X76",
        "tblEfficiencyTokens",
    ),
    ("Context & Brain", 44, 26, "A1:Z45", "tblContextBrain"),
    ("Defects", 13, 22, "A1:V14", "tblDefects"),
];
fn elem(n: &str) -> Element {
    Element::new(n)
}
fn local(n: &str) -> &str {
    n.rsplit(':').next().unwrap_or(n)
}
fn child<'a>(e: &'a Element, n: &str) -> Option<&'a Element> {
    e.children
        .iter()
        .filter_map(XMLNode::as_element)
        .find(|x| local(&x.name) == n)
}
fn children<'a>(e: &'a Element, n: &str) -> Vec<&'a Element> {
    e.children
        .iter()
        .filter_map(XMLNode::as_element)
        .filter(|x| local(&x.name) == n)
        .collect()
}
fn child_mut<'a>(e: &'a mut Element, n: &str) -> Option<&'a mut Element> {
    e.children
        .iter_mut()
        .filter_map(XMLNode::as_mut_element)
        .find(|x| local(&x.name) == n)
}
fn ensure<'a>(e: &'a mut Element, n: &str) -> &'a mut Element {
    if child(e, n).is_none() {
        compat::insert_ordered(e, elem(n));
    }
    child_mut(e, n).unwrap()
}
fn set_text(e: &mut Element, s: String) {
    e.children.clear();
    e.children.push(XMLNode::Text(s));
}
fn content(e: &Element) -> String {
    let mut s = String::new();
    for n in &e.children {
        match n {
            XMLNode::Text(t) | XMLNode::CData(t) => s.push_str(t),
            XMLNode::Element(x) => s.push_str(&content(x)),
            _ => (),
        }
    }
    s
}
fn attr(e: &Element, n: &str) -> String {
    if let Some(value) = e.attributes.get(n) {
        return value.clone();
    }
    e.attributes
        .iter()
        .find(|(k, _)| k.as_str() != "xmlns" && !k.starts_with("xmlns:") && local(k) == n)
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}
fn xml(raw: &[u8]) -> Result<Element> {
    use quick_xml::{Reader, events::Event};
    let mut r = Reader::from_reader(raw);
    r.config_mut().trim_text(false);
    let mut stack = Vec::<Element>::new();
    let mut root = None;
    loop {
        match r.read_event()? {
            Event::Start(x) | Event::Empty(x) => {
                let empty = raw.get(r.buffer_position().saturating_sub(2) as usize) == Some(&b'/');
                let mut e = elem(std::str::from_utf8(x.name().as_ref())?);
                for a in x.attributes() {
                    let a = a?;
                    e.attributes.insert(
                        std::str::from_utf8(a.key.as_ref())?.to_owned(),
                        a.decode_and_unescape_value(r.decoder())?.into_owned(),
                    );
                }
                if empty {
                    if let Some(p) = stack.last_mut() {
                        p.children.push(XMLNode::Element(e));
                    } else {
                        root = Some(e);
                    }
                } else {
                    stack.push(e);
                }
            }
            Event::End(_) => {
                let e = stack.pop().context("unbalanced workbook XML")?;
                if let Some(p) = stack.last_mut() {
                    p.children.push(XMLNode::Element(e));
                } else {
                    if root.is_some() {
                        bail!("multiple XML roots");
                    }
                    root = Some(e);
                }
            }
            Event::Text(t) => {
                if let Some(p) = stack.last_mut() {
                    p.children.push(XMLNode::Text(
                        quick_xml::escape::unescape(&t.decode()?)?.into_owned(),
                    ));
                }
            }
            Event::GeneralRef(t) => {
                let entity = format!("&{};", std::str::from_utf8(&t)?);
                let value = quick_xml::escape::unescape(&entity)?.into_owned();
                if let Some(p) = stack.last_mut() {
                    p.children.push(XMLNode::Text(value));
                }
            }
            Event::CData(t) => {
                if let Some(p) = stack.last_mut() {
                    p.children.push(XMLNode::CData(t.decode()?.into_owned()));
                }
            }
            Event::Comment(t) => {
                if let Some(p) = stack.last_mut() {
                    p.children.push(XMLNode::Comment(t.decode()?.into_owned()));
                }
            }
            Event::PI(t) => {
                if let Some(p) = stack.last_mut() {
                    p.children.push(XMLNode::ProcessingInstruction(
                        String::from_utf8(t.target().to_vec())?,
                        Some(String::from_utf8(t.content().to_vec())?),
                    ));
                }
            }
            Event::DocType(_) => bail!("DOCTYPE not admitted"),
            Event::Eof => break,
            _ => (),
        }
    }
    if !stack.is_empty() {
        bail!("unbalanced workbook XML");
    }
    root.context("empty workbook XML")
}
fn encode(e: &Element) -> Result<Vec<u8>> {
    use quick_xml::{
        Writer,
        events::{BytesCData, BytesEnd, BytesPI, BytesStart, BytesText, Event},
    };
    fn put(e: &Element, w: &mut Writer<Vec<u8>>) -> Result<()> {
        let mut start = BytesStart::new(e.name.as_str());
        let mut attrs = e
            .attributes
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<BTreeMap<_, _>>();
        if let Some(ns) = &e.namespace {
            attrs.entry("xmlns".into()).or_insert(ns.clone());
        }
        if let Some(ns) = &e.namespaces {
            for (k, v) in ns.iter() {
                attrs
                    .entry(if k.is_empty() {
                        "xmlns".into()
                    } else {
                        format!("xmlns:{k}")
                    })
                    .or_insert(v.to_string());
            }
        }
        for (k, v) in &attrs {
            start.push_attribute((k.as_str(), v.as_str()));
        }
        w.write_event(Event::Start(start))?;
        for n in &e.children {
            match n {
                XMLNode::Element(c) => put(c, w)?,
                XMLNode::Text(t) => w.write_event(Event::Text(BytesText::new(t)))?,
                XMLNode::CData(t) => w.write_event(Event::CData(BytesCData::new(t)))?,
                XMLNode::Comment(t) => w.write_event(Event::Comment(BytesText::new(t)))?,
                XMLNode::ProcessingInstruction(t, c) => {
                    w.write_event(Event::PI(BytesPI::new(format!(
                        "{t}{}",
                        c.as_ref().map(|s| format!(" {s}")).unwrap_or_default()
                    ))))?
                }
            }
        }
        w.write_event(Event::End(BytesEnd::new(e.name.as_str())))?;
        Ok(())
    }
    let mut w = Writer::new(Vec::new());
    put(e, &mut w)?;
    Ok(w.into_inner())
}
fn namespace_image(from: &Element, to: &mut Element) {
    for (k, v) in &from.attributes {
        if k == "xmlns" || k.starts_with("xmlns:") {
            to.attributes.entry(k.clone()).or_insert(v.clone());
        }
    }
}
fn path_join(base: &str, target: &str) -> Result<String> {
    let mut stack = vec![];
    let combined = if target.starts_with('/') {
        target.trim_start_matches('/').to_string()
    } else {
        format!(
            "{}/{}",
            base.rsplit_once('/').map(|x| x.0).unwrap_or(""),
            target
        )
    };
    for p in combined.split('/') {
        match p {
            "" | "." => (),
            ".." => {
                if stack.pop().is_none() {
                    bail!("OOXML relationship escapes package");
                }
            }
            _ => stack.push(p),
        }
    }
    Ok(stack.join("/"))
}
fn rel_path(part: &str) -> String {
    let (p, n) = part.rsplit_once('/').unwrap_or(("", part));
    format!("{p}/_rels/{n}.rels")
}
fn letters(mut c: usize) -> String {
    let mut b = vec![];
    while c > 0 {
        c -= 1;
        b.push((b'A' + (c % 26) as u8) as char);
        c /= 26;
    }
    b.into_iter().rev().collect()
}
fn coord(s: &str) -> Result<(usize, usize)> {
    let split = s
        .find(|c: char| c.is_ascii_digit())
        .context("invalid cell coordinate")?;
    let mut col = 0;
    for c in s[..split].bytes() {
        if !c.is_ascii_uppercase() {
            bail!("invalid cell column");
        }
        col = col * 26 + (c - b'A' + 1) as usize;
    }
    let row = s[split..].parse()?;
    if col == 0 || col > 16384 || row == 0 || row > 1048576 {
        bail!("cell coordinate out of range");
    }
    Ok((col, row))
}
fn cell_value(c: &Element, strings: &[Element]) -> Value {
    if child(c, "f").is_some() {
        return Value::String(format!(
            "={}",
            child(c, "f").map(content).unwrap_or_default()
        ));
    }
    cached_cell_value(c, strings)
}
fn cached_cell_value(c: &Element, strings: &[Element]) -> Value {
    match attr(c, "t").as_str() {
        "s" => child(c, "v")
            .and_then(|x| content(x).parse::<usize>().ok())
            .and_then(|i| strings.get(i))
            .map(|x| Value::String(content(x)))
            .unwrap_or(Value::Null),
        "inlineStr" => child(c, "is")
            .map(|x| Value::String(content(x)))
            .unwrap_or(Value::Null),
        "b" => Value::Bool(child(c, "v").is_some_and(|x| content(x) == "1")),
        "str" | "e" => child(c, "v")
            .map(|x| Value::String(content(x)))
            .unwrap_or(Value::Null),
        _ => {
            let s = child(c, "v").map(content).unwrap_or_default();
            if s.is_empty() {
                Value::Null
            } else {
                serde_json::from_str(&s).unwrap_or(Value::String(s))
            }
        }
    }
}
fn replace_value(c: &mut Element, value: &Value) {
    c.children.retain(|x| {
        !x.as_element()
            .is_some_and(|e| matches!(local(&e.name), "f" | "v" | "is"))
    });
    c.attributes.remove("t");
    match value {
        Value::String(s) if s.starts_with('=') => {
            let mut f = elem("f");
            set_text(&mut f, s[1..].to_owned());
            c.children.push(XMLNode::Element(f));
        }
        Value::String(s) => {
            c.attributes.insert("t".into(), "inlineStr".into());
            let mut is = elem("is");
            let mut t = elem("t");
            t.attributes.insert("xml:space".into(), "preserve".into());
            set_text(&mut t, s.clone());
            is.children.push(XMLNode::Element(t));
            c.children.push(XMLNode::Element(is));
        }
        Value::Bool(b) => {
            c.attributes.insert("t".into(), "b".into());
            let mut v = elem("v");
            set_text(&mut v, if *b { "1" } else { "0" }.into());
            c.children.push(XMLNode::Element(v));
        }
        Value::Null => (),
        _ => {
            let mut v = elem("v");
            set_text(&mut v, value.to_string());
            c.children.push(XMLNode::Element(v));
        }
    }
}
#[derive(Clone)]
pub struct Package {
    parts: BTreeMap<String, Vec<u8>>,
    pub sheets: BTreeMap<String, String>,
    strings: Vec<Element>,
}
impl Package {
    pub fn read(path: &Path) -> Result<Self> {
        Self::from_reader(fs::File::open(path)?)
    }
    pub fn from_reader<R: Read + std::io::Seek>(reader: R) -> Result<Self> {
        let mut zip = zip::ZipArchive::new(reader)?;
        let mut parts = BTreeMap::new();
        let mut total = 0u64;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i)?;
            if f.is_dir() {
                continue;
            }
            let name = f.name().to_string();
            if name.starts_with('/')
                || name.split('/').any(|p| matches!(p, ".." | "." | ""))
                || parts.contains_key(&name)
                || f.size() > 64 * 1024 * 1024
            {
                bail!("invalid or oversized workbook package part");
            }
            total += f.size();
            if total > 512 * 1024 * 1024 {
                bail!("workbook package bound");
            }
            let mut b = vec![];
            f.by_ref().take(64 * 1024 * 1024 + 1).read_to_end(&mut b)?;
            if b.len() > 64 * 1024 * 1024 {
                bail!("workbook part bound");
            }
            parts.insert(name, b);
        }
        let wb = xml(parts.get("xl/workbook.xml").context("missing workbook")?)?;
        let rels = xml(parts
            .get("xl/_rels/workbook.xml.rels")
            .context("missing workbook relationships")?)?;
        let mut sheets = BTreeMap::new();
        for s in children(child(&wb, "sheets").context("missing sheets")?, "sheet") {
            let id = attr(s, "id");
            let r = children(&rels, "Relationship")
                .into_iter()
                .find(|r| attr(r, "Id") == id)
                .context("missing sheet relationship")?;
            if attr(r, "TargetMode") == "External" {
                bail!("external worksheet relationship");
            }
            let name = attr(s, "name");
            if sheets
                .insert(name, path_join("xl/workbook.xml", &attr(r, "Target"))?)
                .is_some()
            {
                bail!("duplicate worksheet name");
            }
        }
        let strings = parts
            .get("xl/sharedStrings.xml")
            .map(|b| xml(b).map(|x| children(&x, "si").into_iter().cloned().collect()))
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            parts,
            sheets,
            strings,
        })
    }
    /// Decode only a supplied cached scalar; formula expressions are kept separately.
    pub fn cached_value(&self, cell: &Element) -> Value {
        cached_cell_value(cell, &self.strings)
    }
    /// Match the canonical viewer's first Content Types styles Override selection.
    pub fn preview_styles(&self) -> Result<Option<Element>> {
        let types = self.part("[Content_Types].xml")?;
        for entry in types
            .children
            .iter()
            .filter_map(XMLNode::as_element)
            .filter(|entry| local(&entry.name) == "Override")
        {
            if entry.attributes.get("ContentType").is_some_and(|value| {
                value == "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"
            }) {
                let name = entry
                    .attributes
                    .get("PartName")
                    .context("styles part lacks name")?;
                let name = name.strip_prefix('/').unwrap_or(name);
                // Lookup is exclusively in the already admitted ZIP part map.
                return self.part(name).map(Some);
            }
        }
        Ok(None)
    }
    pub fn part(&self, p: &str) -> Result<Element> {
        let mut element = xml(self.parts.get(p).context("missing workbook XML part")?)?;
        if self.sheets.values().any(|path| path == p) {
            compat::worksheet(&mut element)?;
        }
        Ok(element)
    }
    pub fn put(&mut self, p: &str, e: &Element) -> Result<()> {
        fn uses_r(e: &Element) -> bool {
            e.attributes.keys().any(|k| k.starts_with("r:"))
                || e.children
                    .iter()
                    .filter_map(XMLNode::as_element)
                    .any(uses_r)
        }
        let mut e = e.clone();
        if uses_r(&e) {
            e.attributes.entry("xmlns:r".into()).or_insert(REL.into());
        }
        self.parts.insert(p.to_string(), encode(&e)?);
        Ok(())
    }
    fn relationships(&self, p: &str) -> Result<Element> {
        self.parts
            .get(&rel_path(p))
            .map(|b| xml(b))
            .transpose()
            .map(|e| {
                e.unwrap_or_else(|| {
                    let mut e = elem("Relationships");
                    e.namespace = Some(PKGREL.into());
                    e
                })
            })
    }
    fn add_relationship(
        &mut self,
        part: &str,
        kind: &str,
        target: &str,
        external: bool,
    ) -> Result<String> {
        let mut rs = self.relationships(part)?;
        let ids = children(&rs, "Relationship")
            .into_iter()
            .map(|r| attr(r, "Id"))
            .collect::<BTreeSet<_>>();
        let id = (1..)
            .map(|i| format!("rId{i}"))
            .find(|s| !ids.contains(s))
            .unwrap();
        let mut e = elem("Relationship");
        e.attributes.insert("Id".into(), id.clone());
        e.attributes.insert("Type".into(), format!("{REL}/{kind}"));
        e.attributes.insert("Target".into(), target.into());
        if external {
            e.attributes.insert("TargetMode".into(), "External".into());
        }
        rs.children.push(XMLNode::Element(e));
        self.put(&rel_path(part), &rs)?;
        Ok(id)
    }
    fn content_type(&mut self, part: &str, kind: &str) -> Result<()> {
        let mut e = self.part("[Content_Types].xml")?;
        if !children(&e, "Override")
            .into_iter()
            .any(|x| attr(x, "PartName") == format!("/{part}"))
        {
            let mut x = elem("Override");
            x.namespace = Some(CT.into());
            x.attributes.insert("PartName".into(), format!("/{part}"));
            x.attributes.insert("ContentType".into(), kind.into());
            e.children.push(XMLNode::Element(x));
            self.put("[Content_Types].xml", &e)?;
        }
        Ok(())
    }
    pub fn rows(&self, name: &str) -> Result<Vec<Vec<Value>>> {
        self.rows_bounded(name, usize::MAX)
    }
    /// Admit outer row slots and inner value slots before either Vec grows.
    pub fn rows_bounded(&self, name: &str, maximum_slots: usize) -> Result<Vec<Vec<Value>>> {
        Ok(self.sheet_rows_bounded(name, maximum_slots)?.1)
    }
    /// Return one normalized worksheet image together with its bounded dense rows.
    pub fn sheet_rows_bounded(
        &self,
        name: &str,
        maximum_slots: usize,
    ) -> Result<(Element, Vec<Vec<Value>>)> {
        let e = self.part(self.sheets.get(name).context("missing worksheet")?)?;
        let mut allocated = 0usize;
        let mut out = vec![];
        if let Some(data) = child(&e, "sheetData") {
            for row in children(data, "row") {
                let n: usize = attr(row, "r").parse()?;
                if n > 100000 {
                    bail!("worksheet row bound");
                }
                let rows_added = n.saturating_sub(out.len());
                allocated = allocated
                    .checked_add(rows_added)
                    .context("worksheet dense slot overflow")?;
                if allocated > maximum_slots {
                    bail!("worksheet dense slot bound");
                }
                while out.len() < n {
                    out.push(vec![]);
                }
                for c in children(row, "c") {
                    let (col, _) = coord(&attr(c, "r"))?;
                    let cells_added = col.saturating_sub(out[n - 1].len());
                    allocated = allocated
                        .checked_add(cells_added)
                        .context("worksheet dense slot overflow")?;
                    if allocated > maximum_slots {
                        bail!("worksheet dense slot bound");
                    }
                    while out[n - 1].len() < col {
                        out[n - 1].push(Value::Null);
                    }
                    out[n - 1][col - 1] = cell_value(c, &self.strings);
                }
            }
        }
        Ok((e, out))
    }
    pub fn records(&self, name: &str) -> Result<Vec<Value>> {
        let rows = self.rows(name)?;
        let headers = rows.first().context("empty worksheet")?;
        Ok(rows
            .iter()
            .skip(1)
            .filter(|r| r.iter().any(|v| !v.is_null() && v != ""))
            .map(|r| {
                let mut m = Map::new();
                for (i, h) in headers.iter().enumerate() {
                    let h = text(h).trim().to_string();
                    if !h.is_empty() {
                        m.insert(h, r.get(i).cloned().unwrap_or(Value::Null));
                    }
                }
                Value::Object(m)
            })
            .collect())
    }
    pub fn set_cell(&mut self, name: &str, address: &str, v: &Value) -> Result<()> {
        let p = self.sheets.get(name).context("missing worksheet")?.clone();
        let mut e = self.part(&p)?;
        let (_, r) = coord(address)?;
        let data = ensure(&mut e, "sheetData");
        if !children(data, "row")
            .iter()
            .any(|x| attr(x, "r") == r.to_string())
        {
            let mut x = elem("row");
            x.attributes.insert("r".into(), r.to_string());
            data.children.push(XMLNode::Element(x));
        }
        let row = data
            .children
            .iter_mut()
            .filter_map(XMLNode::as_mut_element)
            .find(|x| local(&x.name) == "row" && attr(x, "r") == r.to_string())
            .unwrap();
        if !children(row, "c").iter().any(|x| attr(x, "r") == address) {
            let mut x = elem("c");
            x.attributes.insert("r".into(), address.into());
            row.children.push(XMLNode::Element(x));
        }
        let c = row
            .children
            .iter_mut()
            .filter_map(XMLNode::as_mut_element)
            .find(|x| local(&x.name) == "c" && attr(x, "r") == address)
            .unwrap();
        replace_value(c, v);
        self.put(&p, &e)
    }
    pub fn write(&self, path: &Path) -> Result<()> {
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, b) in &self.parts {
            out.start_file(
                name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )?;
            out.write_all(b)?;
        }
        atomic(path, &out.finish()?.into_inner())
    }
    fn inline_source(c: &mut Element, strings: &[Element]) -> Result<()> {
        if attr(c, "t") == "s" {
            let index = child(c, "v")
                .context("shared string index")?
                .get_text()
                .context("shared string index")?
                .parse::<usize>()?;
            let si = strings.get(index).context("shared string outside table")?;
            let mut is = si.clone();
            is.name = "is".into();
            c.children.retain(|x| {
                !x.as_element()
                    .is_some_and(|e| matches!(local(&e.name), "v" | "is"))
            });
            c.children.push(XMLNode::Element(is));
            c.attributes.insert("t".into(), "inlineStr".into());
        }
        Ok(())
    }
    fn merge_styles(&mut self, source: &Package) -> Result<Vec<usize>> {
        let mut target = self.part("xl/styles.xml")?;
        let input = source.part("xl/styles.xml")?;
        let mut number_ids = BTreeMap::new();
        let max = children(
            child(&target, "numFmts").unwrap_or(&elem("numFmts")),
            "numFmt",
        )
        .into_iter()
        .filter_map(|e| attr(e, "numFmtId").parse::<usize>().ok())
        .max()
        .unwrap_or(163)
        .max(163);
        let mut next = max + 1;
        if let Some(nums) = child(&input, "numFmts") {
            for n in children(nums, "numFmt") {
                let old = attr(n, "numFmtId").parse::<usize>()?;
                let mut copy = n.clone();
                namespace_image(&input, &mut copy);
                copy.attributes.insert("numFmtId".into(), next.to_string());
                number_ids.insert(old, next);
                next += 1;
                ensure(&mut target, "numFmts")
                    .children
                    .push(XMLNode::Element(copy));
            }
        }
        let mut offsets = BTreeMap::new();
        for (name, tag) in [
            ("fonts", "font"),
            ("fills", "fill"),
            ("borders", "border"),
            ("cellStyleXfs", "xf"),
        ] {
            let n = children(ensure(&mut target, name), tag).len();
            offsets.insert(name, n);
            if let Some(group) = child(&input, name) {
                for x in children(group, tag) {
                    let mut x = x.clone();
                    namespace_image(&input, &mut x);
                    if name == "cellStyleXfs" {
                        remap_xf(&mut x, &offsets, &number_ids)?;
                    }
                    ensure(&mut target, name).children.push(XMLNode::Element(x));
                }
            }
        }
        let offset = children(ensure(&mut target, "cellXfs"), "xf").len();
        let mut mapping = vec![];
        if let Some(group) = child(&input, "cellXfs") {
            for x in children(group, "xf") {
                let mut x = x.clone();
                namespace_image(&input, &mut x);
                remap_xf(&mut x, &offsets, &number_ids)?;
                if let Some(id) = x.attributes.get_mut("xfId") {
                    *id = (id.parse::<usize>()? + offsets["cellStyleXfs"]).to_string();
                }
                mapping.push(offset + mapping.len());
                ensure(&mut target, "cellXfs")
                    .children
                    .push(XMLNode::Element(x));
            }
        }
        for n in [
            "numFmts",
            "fonts",
            "fills",
            "borders",
            "cellStyleXfs",
            "cellXfs",
        ] {
            if let Some(x) = child_mut(&mut target, n) {
                x.attributes.insert(
                    "count".into(),
                    x.children
                        .iter()
                        .filter(|n| n.as_element().is_some())
                        .count()
                        .to_string(),
                );
            }
        }
        self.put("xl/styles.xml", &target)?;
        Ok(mapping)
    }
    pub fn append(
        &mut self,
        source: &Package,
        name: &str,
        ids: &BTreeSet<String>,
        width: usize,
        styles: &[usize],
        defects: bool,
    ) -> Result<()> {
        let p = self
            .sheets
            .get(name)
            .context("target sheet missing")?
            .clone();
        let q = source.sheets.get(name).context("source sheet missing")?;
        let mut target = self.part(&p)?;
        let input = source.part(q)?;
        let srcrows = source.rows(name)?;
        let dstrows = self.rows(name)?;
        let sh = srcrows.first().context("source header")?;
        let th = dstrows.first().context("target header")?;
        let mut cmap = BTreeMap::new();
        for (c, h) in th.iter().take(width).enumerate() {
            let s = sh
                .iter()
                .position(|v| v == h)
                .context("missing source header")?;
            cmap.insert(s + 1, c + 1);
        }
        let mut selected = BTreeMap::new();
        let data = child(&input, "sheetData").context("source sheet data")?;
        for row in children(data, "row") {
            let r: usize = attr(row, "r").parse()?;
            if r < 2 {
                continue;
            }
            let id = if defects {
                let row = srcrows.get(r - 1).context("source defect row")?;
                let field = |name: &str| -> String {
                    sh.iter()
                        .position(|v| v == name)
                        .and_then(|i| row.get(i))
                        .map(text)
                        .unwrap_or_default()
                };
                format!(
                    "{}|{}|{}",
                    field("Run ID"),
                    field("Test Case ID"),
                    field("Defect ID")
                )
            } else {
                srcrows
                    .get(r - 1)
                    .and_then(|r| r.first())
                    .map(text)
                    .unwrap_or_default()
            };
            if ids.contains(&id) {
                selected.insert(id, row.clone());
            }
        }
        if selected.keys().cloned().collect::<BTreeSet<_>>() != *ids {
            bail!("source lacks selected workbook identities");
        }
        let mut mapping = BTreeMap::new();
        let first_row = children(ensure(&mut target, "sheetData"), "row")
            .into_iter()
            .filter_map(|r| attr(r, "r").parse::<usize>().ok())
            .max()
            .unwrap_or(1)
            + 1;
        for (next, (_, row)) in (first_row..).zip(selected) {
            let mut out = row.clone();
            namespace_image(&input, &mut out);
            out.attributes.insert("r".into(), next.to_string());
            out.attributes.remove("spans");
            out.children
                .retain(|x| !x.as_element().is_some_and(|e| local(&e.name) == "c"));
            for c in children(&row, "c") {
                let (old_col, _) = coord(&attr(c, "r"))?;
                if let Some(new_col) = cmap.get(&old_col) {
                    let mut c = c.clone();
                    let old = attr(&c, "r");
                    let new = format!("{}{next}", letters(*new_col));
                    c.attributes.insert("r".into(), new.clone());
                    if let Some(s) = c.attributes.get_mut("s") {
                        let old: usize = s.parse()?;
                        *s = styles
                            .get(old)
                            .context("source style outside table")?
                            .to_string();
                    }
                    if child(&c, "f").is_some() {
                        // copy_cell(source.value) transfers the expression, not a stale result.
                        // Keep ORIGINAL package caches available to read-only preview consumers.
                        c.attributes.remove("t");
                        c.children.retain(|node| {
                            !node
                                .as_element()
                                .is_some_and(|value| local(&value.name) == "v")
                        });
                    }
                    Self::inline_source(&mut c, &source.strings)?;
                    mapping.insert(old, new);
                    out.children.push(XMLNode::Element(c));
                }
            }
            if defects {
                let mut values = BTreeMap::new();
                for c in children(&out, "c") {
                    let (col, _) = coord(&attr(c, "r"))?;
                    values.insert(
                        text(th.get(col - 1).unwrap_or(&Value::Null)),
                        text(&cell_value(c, &[])),
                    );
                }
                let replacement = defect(
                    values.get("Run ID").map(String::as_str).unwrap_or(""),
                    values.get("Test Case ID").map(String::as_str).unwrap_or(""),
                    values.get("Defect ID").map(String::as_str).unwrap_or(""),
                )
                .context("unknown Codex defect composite")?;
                for c in out.children.iter_mut().filter_map(XMLNode::as_mut_element) {
                    let (col, _) = coord(&attr(c, "r"))?;
                    if th.get(col - 1).is_some_and(|v| v == "Defect ID") {
                        replace_value(c, &Value::String(replacement.into()));
                    }
                }
            }
            ensure(&mut target, "sheetData")
                .children
                .push(XMLNode::Element(out));
        }
        self.copy_hyperlinks(source, q, &p, &input, &mut target, &mapping)?;
        self.copy_comments(source, q, &p, &mut target, &mapping)?;
        self.put(&p, &target)
    }
    fn copy_hyperlinks(
        &mut self,
        source: &Package,
        q: &str,
        p: &str,
        input: &Element,
        target: &mut Element,
        map: &BTreeMap<String, String>,
    ) -> Result<()> {
        let rels = source.relationships(q)?;
        if let Some(hs) = child(input, "hyperlinks") {
            for h in children(hs, "hyperlink") {
                if let Some(new) = map.get(&attr(h, "ref")) {
                    let mut h = h.clone();
                    h.attributes.insert("ref".into(), new.clone());
                    let id = attr(&h, "id");
                    if !id.is_empty() {
                        let r = children(&rels, "Relationship")
                            .into_iter()
                            .find(|r| attr(r, "Id") == id)
                            .context("hyperlink relationship missing")?;
                        if !attr(r, "Type").ends_with("/hyperlink") {
                            bail!("invalid hyperlink relationship");
                        }
                        let new_id = self.add_relationship(
                            p,
                            "hyperlink",
                            &attr(r, "Target"),
                            attr(r, "TargetMode") == "External",
                        )?;
                        h.attributes.retain(|k, _| local(k) != "id");
                        h.attributes.insert("r:id".into(), new_id);
                    }
                    ensure(target, "hyperlinks")
                        .children
                        .push(XMLNode::Element(h));
                }
            }
        }
        Ok(())
    }
    fn find_related(&self, p: &str, kind: &str) -> Result<Option<(String, String)>> {
        let r = self.relationships(p)?;
        for x in children(&r, "Relationship") {
            if attr(x, "Type").ends_with(&format!("/{kind}")) {
                if attr(x, "TargetMode") == "External" {
                    bail!("external workbook object");
                }
                return Ok(Some((attr(x, "Id"), path_join(p, &attr(x, "Target"))?)));
            }
        }
        Ok(None)
    }
    fn unique_part(&self, prefix: &str, suffix: &str) -> String {
        (1..)
            .map(|n| format!("{prefix}{n}{suffix}"))
            .find(|p| !self.parts.contains_key(p))
            .unwrap()
    }
    fn copy_comments(
        &mut self,
        source: &Package,
        q: &str,
        p: &str,
        target: &mut Element,
        map: &BTreeMap<String, String>,
    ) -> Result<()> {
        let Some((_, srcpath)) = source.find_related(q, "comments")? else {
            return Ok(());
        };
        let src = source.part(&srcpath)?;
        let Some(list) = child(&src, "commentList") else {
            return Ok(());
        };
        let picked = children(list, "comment")
            .into_iter()
            .filter(|x| map.contains_key(&attr(x, "ref")))
            .cloned()
            .collect::<Vec<_>>();
        if picked.is_empty() {
            return Ok(());
        }
        let destpath = if let Some((_, d)) = self.find_related(p, "comments")? {
            d
        } else {
            let d = self.unique_part("xl/comments", ".xml");
            let mut e = elem("comments");
            e.namespace = Some(MAIN.into());
            e.children.push(XMLNode::Element(elem("authors")));
            e.children.push(XMLNode::Element(elem("commentList")));
            self.put(&d, &e)?;
            self.add_relationship(
                p,
                "comments",
                &format!("../{}", d.trim_start_matches("xl/")),
                false,
            )?;
            self.content_type(
                &d,
                "application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml",
            )?;
            d
        };
        let mut dest = self.part(&destpath)?;
        let offset = children(ensure(&mut dest, "authors"), "author").len();
        if let Some(a) = child(&src, "authors") {
            for a in children(a, "author") {
                ensure(&mut dest, "authors")
                    .children
                    .push(XMLNode::Element(a.clone()));
            }
        }
        for mut c in picked {
            let old = attr(&c, "ref");
            c.attributes.insert("ref".into(), map[&old].clone());
            let author = attr(&c, "authorId").parse::<usize>()?;
            c.attributes
                .insert("authorId".into(), (offset + author).to_string());
            ensure(&mut dest, "commentList")
                .children
                .push(XMLNode::Element(c));
        }
        self.put(&destpath, &dest)?;
        if let Some((_, vsrc)) = source.find_related(q, "vmlDrawing")? {
            let vinput = source.part(&vsrc)?;
            let (vdest, id) = if let Some((id, p)) = self.find_related(p, "vmlDrawing")? {
                (p, id)
            } else {
                let d = self.unique_part("xl/drawings/vmlDrawing", ".vml");
                let mut e = vinput.clone();
                e.children
                    .retain(|x| !x.as_element().is_some_and(|e| local(&e.name) == "shape"));
                self.put(&d, &e)?;
                let id = self.add_relationship(
                    p,
                    "vmlDrawing",
                    &format!("../drawings/{}", d.rsplit('/').next().unwrap()),
                    false,
                )?;
                let ct = self.part("[Content_Types].xml")?;
                if !children(&ct, "Default")
                    .into_iter()
                    .any(|x| attr(x, "Extension") == "vml")
                {
                    let mut ct = ct;
                    let mut d = elem("Default");
                    d.attributes.insert("Extension".into(), "vml".into());
                    d.attributes.insert(
                        "ContentType".into(),
                        "application/vnd.openxmlformats-officedocument.vmlDrawing".into(),
                    );
                    ct.children.push(XMLNode::Element(d));
                    self.put("[Content_Types].xml", &ct)?;
                }
                (d, id)
            };
            let mut vd = self.part(&vdest)?;
            let mut serial = 10000 + vd.children.len();
            for shape in vinput
                .children
                .iter()
                .filter_map(XMLNode::as_element)
                .filter(|e| local(&e.name) == "shape")
            {
                let Some(client) = child(shape, "ClientData") else {
                    continue;
                };
                let Some(row) = child(client, "Row").and_then(|e| content(e).parse::<usize>().ok())
                else {
                    continue;
                };
                let Some(col) =
                    child(client, "Column").and_then(|e| content(e).parse::<usize>().ok())
                else {
                    continue;
                };
                let old = format!("{}{}", letters(col + 1), row + 1);
                if let Some(new) = map.get(&old) {
                    let (nc, nr) = coord(new)?;
                    let mut copied = shape.clone();
                    serial += 1;
                    copied
                        .attributes
                        .insert("id".into(), format!("_x0000_s{serial}"));
                    if let Some(c) = child_mut(&mut copied, "ClientData") {
                        if let Some(x) = child_mut(c, "Row") {
                            set_text(x, (nr - 1).to_string());
                        }
                        if let Some(x) = child_mut(c, "Column") {
                            set_text(x, (nc - 1).to_string());
                        }
                        if let Some(a) = child_mut(c, "Anchor") {
                            let mut nums = content(a)
                                .split(',')
                                .map(|s| s.trim().parse::<i64>())
                                .collect::<std::result::Result<Vec<_>, _>>()?;
                            if nums.len() == 8 {
                                for i in [0, 4] {
                                    nums[i] += nc as i64 - col as i64 - 1;
                                }
                                for i in [2, 6] {
                                    nums[i] += nr as i64 - row as i64 - 1;
                                }
                                set_text(
                                    a,
                                    nums.iter()
                                        .map(ToString::to_string)
                                        .collect::<Vec<_>>()
                                        .join(", "),
                                );
                            }
                        }
                    }
                    vd.children.push(XMLNode::Element(copied));
                }
            }
            self.put(&vdest, &vd)?;
            let legacy = ensure(target, "legacyDrawing");
            legacy.attributes.insert("r:id".into(), id);
        }
        Ok(())
    }
    pub fn table(
        &mut self,
        name: &str,
        width: usize,
        reference: &str,
        table_name: &str,
    ) -> Result<()> {
        let p = self.sheets.get(name).context("missing sheet")?.clone();
        let mut sheet = self.part(&p)?;
        if let Some(data) = child_mut(&mut sheet, "sheetData") {
            for r in data.children.iter_mut().filter_map(XMLNode::as_mut_element) {
                r.children.retain(|n| match n.as_element() {
                    Some(c) if local(&c.name) == "c" => coord(&attr(c, "r"))
                        .map(|(col, _)| col <= width)
                        .unwrap_or(false),
                    _ => true,
                });
            }
        }
        let headers = self
            .rows(name)?
            .first()
            .cloned()
            .context("missing headers")?;
        let related = self.find_related(&p, "table")?;
        let (tp, id) = if let Some((id, tp)) = related {
            (tp, id)
        } else {
            let tp = self.unique_part("xl/tables/table", ".xml");
            let id = self.add_relationship(
                &p,
                "table",
                &format!("../tables/{}", tp.rsplit('/').next().unwrap()),
                false,
            )?;
            let mut t = elem("table");
            t.namespace = Some(MAIN.into());
            let max = self
                .parts
                .keys()
                .filter(|k| k.starts_with("xl/tables/") && k.ends_with(".xml"))
                .filter_map(|k| self.part(k).ok())
                .filter_map(|t| attr(&t, "id").parse::<usize>().ok())
                .max()
                .unwrap_or(0);
            t.attributes.insert("id".into(), (max + 1).to_string());
            let mut style = elem("tableStyleInfo");
            for (k, v) in [
                ("name", "TableStyleMedium2"),
                ("showFirstColumn", "0"),
                ("showLastColumn", "0"),
                ("showRowStripes", "1"),
                ("showColumnStripes", "0"),
            ] {
                style.attributes.insert(k.into(), v.into());
            }
            t.children.push(XMLNode::Element(style));
            self.put(&tp, &t)?;
            self.content_type(
                &tp,
                "application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml",
            )?;
            (tp, id)
        };
        let mut t = self.part(&tp)?;
        for k in ["name", "displayName"] {
            t.attributes.insert(k.into(), table_name.into());
        }
        t.attributes.insert("ref".into(), reference.into());
        ensure(&mut t, "autoFilter")
            .attributes
            .insert("ref".into(), reference.into());
        let cols = ensure(&mut t, "tableColumns");
        cols.children.clear();
        cols.attributes.insert("count".into(), width.to_string());
        for (i, h) in headers.iter().take(width).enumerate() {
            let mut c = elem("tableColumn");
            c.attributes.insert("id".into(), (i + 1).to_string());
            c.attributes.insert("name".into(), text(h));
            cols.children.push(XMLNode::Element(c));
        }
        self.put(&tp, &t)?;
        let tables = ensure(&mut sheet, "tableParts");
        tables.children.clear();
        tables.attributes.insert("count".into(), "1".into());
        let mut tab = elem("tablePart");
        tab.attributes.insert("r:id".into(), id);
        tables.children.push(XMLNode::Element(tab));
        ensure(&mut sheet, "dimension")
            .attributes
            .insert("ref".into(), reference.into());
        self.put(&p, &sheet)
    }
    fn add_sheet(&mut self, name: &str) -> Result<()> {
        let mut wb = self.part("xl/workbook.xml")?;
        let group = ensure(&mut wb, "sheets");
        let max = children(group, "sheet")
            .into_iter()
            .filter_map(|s| attr(s, "sheetId").parse::<usize>().ok())
            .max()
            .unwrap_or(0);
        group.children.retain(|x| {
            !x.as_element()
                .is_some_and(|e| local(&e.name) == "sheet" && attr(e, "name") == name)
        });
        let p = self.unique_part("xl/worksheets/sheet", ".xml");
        let id = self.add_relationship(
            "xl/workbook.xml",
            "worksheet",
            p.trim_start_matches("xl/"),
            false,
        )?;
        let mut s = elem("sheet");
        s.attributes.insert("name".into(), name.into());
        s.attributes.insert("sheetId".into(), (max + 1).to_string());
        s.attributes.insert("r:id".into(), id);
        group.children.push(XMLNode::Element(s));
        self.put("xl/workbook.xml", &wb)?;
        let mut sheet = elem("worksheet");
        sheet.namespace = Some(MAIN.into());
        sheet.namespaces = Some(xmltree::Namespace::empty());
        sheet.namespaces.as_mut().unwrap().put("r", REL);
        let mut views = elem("sheetViews");
        let mut view = elem("sheetView");
        view.attributes.insert("workbookViewId".into(), "0".into());
        let mut pane = elem("pane");
        for (k, v) in [
            ("ySplit", "1"),
            ("topLeftCell", "A2"),
            ("activePane", "bottomLeft"),
            ("state", "frozen"),
        ] {
            pane.attributes.insert(k.into(), v.into());
        }
        view.children.push(XMLNode::Element(pane));
        views.children.push(XMLNode::Element(view));
        sheet.children.push(XMLNode::Element(views));
        sheet.children.push(XMLNode::Element(elem("sheetData")));
        self.put(&p, &sheet)?;
        self.content_type(
            &p,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml",
        )?;
        self.sheets.insert(name.into(), p);
        Ok(())
    }
    pub fn sanitize(&mut self) -> Result<()> {
        let patterns = [
            Regex::new(r"/home/[^/\s]+/Desktop/AI-Infrastructure")?,
            Regex::new(r"/Users/[^/\s]+/[^\s,;)]*accelerator-php")?,
        ];
        for p in self.sheets.values().cloned().collect::<Vec<_>>() {
            let mut e = self.part(&p)?;
            let mut changed = false;
            if let Some(d) = child_mut(&mut e, "sheetData") {
                for r in d.children.iter_mut().filter_map(XMLNode::as_mut_element) {
                    for c in r.children.iter_mut().filter_map(XMLNode::as_mut_element) {
                        if child(c, "f").is_some() {
                            continue;
                        }
                        let v = cell_value(c, &self.strings);
                        if let Some(s) = v.as_str() {
                            let mut next = s.to_owned();
                            for re in &patterns {
                                next = re.replace_all(&next, "{repository}").into_owned();
                            }
                            if next != s {
                                replace_value(c, &Value::String(next));
                                changed = true;
                            }
                        }
                    }
                }
            }
            if changed {
                self.put(&p, &e)?;
            }
        }
        Ok(())
    }
}
fn remap_xf(
    x: &mut Element,
    offsets: &BTreeMap<&str, usize>,
    numbers: &BTreeMap<usize, usize>,
) -> Result<()> {
    for (k, group) in [
        ("fontId", "fonts"),
        ("fillId", "fills"),
        ("borderId", "borders"),
    ] {
        if let Some(id) = x.attributes.get_mut(k) {
            *id = (id.parse::<usize>()? + offsets.get(group).copied().unwrap_or(0)).to_string();
        }
    }
    if let Some(id) = x.attributes.get_mut("numFmtId")
        && let Some(n) = numbers.get(&id.parse::<usize>()?)
    {
        *id = n.to_string();
    }
    Ok(())
}
pub fn validate(book: &Package) -> Result<Vec<String>> {
    let mut errors = vec![];
    for (name, count, width, reference, _) in SPECS {
        match book.rows(name) {
            Ok(rows) => {
                let nonempty = rows
                    .iter()
                    .skip(1)
                    .filter(|r| r.iter().any(|v| !v.is_null() && v != ""))
                    .count();
                if nonempty != *count {
                    errors.push(format!("{name}: expected {count} rows"));
                }
                if rows.iter().map(Vec::len).max().unwrap_or(0) != *width {
                    errors.push(format!("{name}: expected {width} columns"));
                }
                let headers = rows.first().cloned().unwrap_or_default();
                if headers
                    .iter()
                    .map(Value::to_string)
                    .collect::<BTreeSet<_>>()
                    .len()
                    != headers.len()
                {
                    errors.push(format!("{name}: duplicate headers"));
                }
                let p = &book.sheets[*name];
                let sheet = book.part(p)?;
                if children(
                    child(&sheet, "tableParts").unwrap_or(&elem("tableParts")),
                    "tablePart",
                )
                .len()
                    != 1
                {
                    errors.push(format!("{name}: exactly one table required"));
                }
                let t = book.find_related(p, "table")?;
                if !t
                    .map(|(_, p)| book.part(&p).map(|e| attr(&e, "ref") == *reference))
                    .transpose()?
                    .unwrap_or(false)
                {
                    errors.push(format!("{name}: incorrect table range"));
                }
            }
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    if !book.sheets.contains_key("Provenance") {
        errors.push("workbook lacks Provenance".into());
    }
    let ids = book
        .records("Test Runs")?
        .iter()
        .map(|r| text(&r["Run ID"]))
        .collect::<BTreeSet<_>>();
    if ids != all_runs() {
        errors.push("authoritative run identities differ".into());
    }
    let dash = book.rows("Dashboard")?;
    for r in 4..=18 {
        let v = dash
            .get(r - 1)
            .and_then(|r| r.get(1))
            .and_then(Value::as_str)
            .unwrap_or("");
        if !v.starts_with('=') || v.contains("#REF!") {
            errors.push(format!("Dashboard B{r}: invalid formula"));
        }
    }
    Ok(errors)
}
pub fn reconstruct(evidence: &Path, current: &Path, output: &Path, force: bool) -> Result<()> {
    let paths = primary(evidence)?;
    let errors = crate::manifest::verify(evidence, &paths["SHA256SUMS.txt"], false)?;
    if !errors.is_empty() {
        bail!("{}", errors.join("; "));
    }
    let runs = records(
        &read_json(&paths["manifests/historical-runs.json"])?,
        &["runs", "records", "rows", "data"],
    )?;
    if runs.len() != 57
        || runs
            .iter()
            .map(|r| pick(r, &["Run ID", "run_id", "RunID"], ""))
            .collect::<BTreeSet<_>>()
            != historical().into_iter().collect()
    {
        bail!("historical manifests differ");
    }
    if records(
        &read_json(&paths["manifests/historical-testcases.json"])?,
        &["test_cases", "cases", "records", "rows", "data"],
    )?
    .len()
        != 74
    {
        bail!("historical test case count");
    }
    read_json(&paths["manifests/historical-othersheets.json"])?;
    let snapshot = &paths["manifests/workbook-snapshot-20260803-2002.xlsx"];
    let output_abs = compat::output_path(output)?;
    if [snapshot, current]
        .iter()
        .any(|p| fs::canonicalize(p).ok().as_ref() == Some(&output_abs))
    {
        bail!("output cannot overwrite either input");
    }
    if output.exists() && !force {
        bail!("output exists; use --force");
    }
    let mut target = Package::read(snapshot)?;
    let source = Package::read(current)?;
    for ((name, _, _, _, _), count) in SPECS.iter().zip([74, 57, 53, 57, 26, 9]) {
        if target.records(name)?.len() != count {
            bail!("historical {name} count mismatch");
        }
    }
    if target
        .records("Test Runs")?
        .iter()
        .map(|r| text(&r["Run ID"]))
        .collect::<BTreeSet<_>>()
        != historical().into_iter().collect()
    {
        bail!("historical run identities mismatch");
    }
    let styles = target.merge_styles(&source)?;
    for (name, _, width, range, table) in SPECS {
        let ids = if *name == "Test Cases" {
            (1..=18).map(|n| format!("TC-AI-{n:03}")).collect()
        } else if *name == "Defects" {
            source
                .records(name)?
                .iter()
                .filter(|r| {
                    defect(
                        &text(&r["Run ID"]),
                        &text(&r["Test Case ID"]),
                        &text(&r["Defect ID"]),
                    )
                    .is_some()
                })
                .map(|r| {
                    format!(
                        "{}|{}|{}",
                        text(&r["Run ID"]),
                        text(&r["Test Case ID"]),
                        text(&r["Defect ID"])
                    )
                })
                .collect()
        } else {
            codex().into_iter().collect()
        };
        target.append(&source, name, &ids, *width, &styles, *name == "Defects")?;
        target.table(name, *width, range, table)?;
    }
    let run_rows = target.rows("Test Runs")?;
    let header = run_rows.first().context("run headers")?;
    let dc = header
        .iter()
        .position(|v| v == "Defect ID")
        .context("defect header")?;
    let cc = header
        .iter()
        .position(|v| v == "Test Case ID")
        .context("case header")?;
    for (i, row) in run_rows.iter().enumerate().skip(1) {
        let run = text(row.first().unwrap_or(&Value::Null));
        let case = text(row.get(cc).unwrap_or(&Value::Null));
        for (original, new) in [
            ("DEF-001", "DEF-AI-001"),
            ("DEF-002", "DEF-AI-002"),
            ("DEF-003", "DEF-AI-003"),
            ("DEF-004", "DEF-AI-004"),
        ] {
            if defect(&run, &case, original) == Some(new) {
                if run != "RUN-20260803-013"
                    && text(row.get(dc).unwrap_or(&Value::Null)) != original
                {
                    bail!("run defect mismatch");
                }
                target.set_cell(
                    "Test Runs",
                    &format!("{}{}", letters(dc + 1), i + 1),
                    &Value::String(new.into()),
                )?;
            }
        }
    }
    let formulas = [
        (4, "=ROWS(tblTestRuns[Run ID])"),
        (5, "=COUNTIF(tblTestRuns[Result],\"Pass\")"),
        (6, "=COUNTIF(tblTestRuns[Result],\"Partial\")"),
        (7, "=COUNTIF(tblTestRuns[Result],\"Fail\")"),
        (8, "=COUNTIF(tblTestRuns[Result],\"Blocked\")"),
        (9, "=IF(B4=0,0,B5/B4)"),
        (
            10,
            "=IFERROR(COUNTIF(tblEfficiencyTokens[Successful First Attempt],\"Yes\")/ROWS(tblEfficiencyTokens[Run ID]),0)",
        ),
        (
            11,
            "=IFERROR(AVERAGEIF(tblQualityMetrics[Run ID],\"RUN-???\",tblQualityMetrics[Overall Score]),0)",
        ),
        (
            12,
            "=IFERROR(AVERAGEIF(tblEfficiencyTokens[Run ID],\"RUN-???\",tblEfficiencyTokens[Total Duration (seconds)]),0)",
        ),
        (
            13,
            "=IF(COUNT(tblEfficiencyTokens[Total Tokens])=0,\"N/A\",AVERAGE(tblEfficiencyTokens[Total Tokens]))",
        ),
        (
            14,
            "=IF(COUNT(tblEfficiencyTokens[Estimated Cost])=0,\"N/A\",AVERAGE(tblEfficiencyTokens[Estimated Cost]))",
        ),
        (
            15,
            "=IFERROR(AVERAGE(tblTestRuns[Human Intervention Count]),0)",
        ),
        (16, "=SUM(tblTestRuns[Safety Violation Count])"),
        (
            17,
            "=IFERROR(COUNTIF(tblContextBrain[Retrieval Successful],\"Yes\")/(COUNTIF(tblContextBrain[Retrieval Successful],\"Yes\")+COUNTIF(tblContextBrain[Retrieval Successful],\"No\")),0)",
        ),
        (
            18,
            "=COUNTIFS(tblDefects[Severity],\"Critical\",tblDefects[Status],\"<>Closed\",tblDefects[Status],\"<>Rejected\")",
        ),
    ];
    for (row, f) in formulas {
        target.set_cell("Dashboard", &format!("B{row}"), &Value::String(f.into()))?;
    }
    {
        let mut wb = target.part("xl/workbook.xml")?;
        let calc = ensure(&mut wb, "calcPr");
        calc.attributes.insert("fullCalcOnLoad".into(), "1".into());
        calc.attributes.insert("forceFullCalc".into(), "1".into());
        target.put("xl/workbook.xml", &wb)?;
    }
    target.add_sheet("Provenance")?;
    let provenance = [
        vec![
            Value::String("Source".into()),
            Value::String("SHA-256".into()),
            Value::String("Role".into()),
            Value::String("Precedence".into()),
        ],
        vec![
            "manifests/workbook-snapshot-20260803-2002.xlsx".into(),
            hash_file(snapshot)?.into(),
            "Historical formatting, formulas, and 57-run values".into(),
            1.into(),
        ],
        vec![
            "manifests/historical-runs.json".into(),
            hash_file(&paths["manifests/historical-runs.json"])?.into(),
            "Exact historical run corroboration".into(),
            2.into(),
        ],
        vec![
            "Accelerator TestCases.xlsx".into(),
            hash_file(current)?.into(),
            "TC-AI-001..018 definitions, runs, analytics, and defects".into(),
            3.into(),
        ],
        vec![
            "Generated At".into(),
            now().into(),
            "UTC".into(),
            Value::Null,
        ],
    ];
    for (r, row) in provenance.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            target.set_cell("Provenance", &format!("{}{}", letters(c + 1), r + 1), v)?;
        }
    }
    target.sanitize()?;
    let errors = validate(&target)?;
    if !errors.is_empty() {
        bail!("{}", errors.join("; "));
    }
    let parent = output.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let tmp = tempfile::Builder::new()
        .suffix(".xlsx")
        .tempfile_in(parent)?;
    target.write(tmp.path())?;
    let errors = validate(&Package::read(tmp.path())?)?;
    if !errors.is_empty() {
        bail!("{}", errors.join("; "));
    }
    tmp.persist(output).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod regression_tests;
