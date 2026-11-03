//! Native regressions for retained QA workbook contracts and OPC preservation.
use super::*;
use serde_json::json;
use std::path::PathBuf;

#[test]
fn namespace_declarations_do_not_alias_cell_or_row_attributes() -> Result<()> {
    let namespace_only = xml(format!(r#"<row xmlns:r="{REL}"/>"#).as_bytes())?;
    assert_eq!(attr(&namespace_only, "r"), "");
    let row = xml(format!(
        r#"<row xmlns:r="{REL}" xmlns:other="urn:other" r="76" other:r="999"/>"#
    )
    .as_bytes())?;
    assert_eq!(attr(&row, "r"), "76");
    let cell = xml(format!(r#"<c xmlns:r="{REL}" r="AD76"/>"#).as_bytes())?;
    assert_eq!(attr(&cell, "r"), "AD76");
    let relation =
        xml(format!(r#"<hyperlink xmlns:r="{REL}" ref="C76" r:id="rId9"/>"#).as_bytes())?;
    assert_eq!(attr(&relation, "id"), "rId9");
    Ok(())
}

fn escaped(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn sheet_xml(headers: &[String], rows: &[Vec<Value>]) -> String {
    let mut data = String::new();
    for (row, values) in std::iter::once(
        headers
            .iter()
            .cloned()
            .map(Value::String)
            .collect::<Vec<_>>(),
    )
    .chain(rows.iter().cloned())
    .enumerate()
    {
        data.push_str(&format!("<row r=\"{}\">", row + 1));
        for (col, value) in values.iter().enumerate() {
            let address = format!("{}{}", letters(col + 1), row + 1);
            match value {
                Value::Null => data.push_str(&format!("<c r=\"{address}\"/>")),
                Value::String(value) if value.starts_with('=') => data.push_str(&format!(
                    "<c r=\"{address}\" s=\"0\"><f>{}</f></c>", escaped(&value[1..])
                )),
                Value::String(value) => data.push_str(&format!(
                    "<c r=\"{address}\" s=\"0\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",
                    escaped(value)
                )),
                Value::Bool(value) => data.push_str(&format!(
                    "<c r=\"{address}\" s=\"0\" t=\"b\"><v>{}</v></c>", usize::from(*value)
                )),
                value => data.push_str(&format!("<c r=\"{address}\" s=\"0\"><v>{value}</v></c>")),
            }
        }
        data.push_str("</row>");
    }
    format!(
        "<worksheet xmlns=\"{MAIN}\" xmlns:r=\"{REL}\"><sheetData>{data}</sheetData></worksheet>"
    )
}

fn target_styles() -> &'static str {
    r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts count="1"><numFmt numFmtId="164" formatCode="yyyy-mm-dd"/></numFmts><fonts count="1"><font><name val="Historical Font"/></font></fonts><fills count="1"><fill><patternFill patternType="none"/></fill></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="1"><xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs></styleSheet>"#
}

fn source_styles() -> &'static str {
    r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:k="urn:qa-style"><numFmts count="1"><numFmt numFmtId="164" formatCode="0.000"/></numFmts><fonts count="2"><font/><font><b/><name val="Source Font"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FF00FF00"/></patternFill></fill></fills><borders count="2"><border/><border><bottom style="thin"><color rgb="FFFF0000"/></bottom></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="164" fontId="1" fillId="1" borderId="1" xfId="0" applyAlignment="1" applyProtection="1"><alignment horizontal="center"/><protection locked="0"/><extLst><ext uri="urn:qa-style"><k:marker k:keep="yes"/></ext></extLst></xf></cellXfs></styleSheet>"#
}

fn fixture(sheets: Vec<(&str, String)>) -> Package {
    let mut parts = BTreeMap::<String, Vec<u8>>::new();
    let mut entries = String::new();
    let mut relationships = String::new();
    let mut types = format!(
        r#"<Types xmlns="{CT}"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#
    );
    let mut names = BTreeMap::new();
    for (index, (name, sheet)) in sheets.into_iter().enumerate() {
        let number = index + 1;
        let path = format!("xl/worksheets/sheet{number}.xml");
        entries.push_str(&format!(
            r#"<sheet name="{}" sheetId="{number}" r:id="rId{number}"/>"#,
            escaped(name)
        ));
        relationships.push_str(&format!(r#"<Relationship Id="rId{number}" Type="{REL}/worksheet" Target="worksheets/sheet{number}.xml"/>"#));
        types.push_str(&format!(r#"<Override PartName="/{path}" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#));
        parts.insert(path.clone(), sheet.into_bytes());
        names.insert(name.to_owned(), path);
    }
    types.push_str("</Types>");
    parts.insert("[Content_Types].xml".into(), types.into_bytes());
    parts.insert("_rels/.rels".into(), format!(r#"<Relationships xmlns="{PKGREL}"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#).into_bytes());
    parts.insert(
        "xl/workbook.xml".into(),
        format!(
            r#"<workbook xmlns="{MAIN}" xmlns:r="{REL}"><sheets>{entries}</sheets></workbook>"#
        )
        .into_bytes(),
    );
    parts.insert(
        "xl/_rels/workbook.xml.rels".into(),
        format!(r#"<Relationships xmlns="{PKGREL}">{relationships}</Relationships>"#).into_bytes(),
    );
    parts.insert("xl/styles.xml".into(), target_styles().as_bytes().to_vec());
    // These parts deliberately have no worksheet interpretation.
    parts.insert("customXml/item1.xml".into(), b"<?xml version=\"1.0\"?>\n<opaque xmlns=\"urn:owner\">  preserve &amp; spacing  </opaque>\n".to_vec());
    parts.insert(
        "xl/media/opaque.bin".into(),
        vec![0, 255, 127, 13, 10, 1, 2],
    );
    Package {
        parts,
        sheets: names,
        strings: vec![],
    }
}

fn reopened(book: &Package, directory: &Path, name: &str) -> Result<Package> {
    let path = directory.join(name);
    book.write(&path)?;
    Package::read(&path)
}

fn cell<'a>(sheet: &'a Element, address: &str) -> &'a Element {
    children(child(sheet, "sheetData").expect("sheetData"), "row")
        .into_iter()
        .flat_map(|row| children(row, "c"))
        .find(|value| attr(value, "r") == address)
        .expect("cell")
}

fn related_part(book: &Package, kind: &str) -> Result<Element> {
    let (_, path) = book
        .find_related("xl/worksheets/sheet1.xml", kind)?
        .context("relationship")?;
    book.part(&path)
}

#[test]
fn retained_missing_package_refuses_without_partial_workbook() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let current = temp.path().join("current.xlsx");
    fs::write(&current, b"input must remain unchanged")?;
    let output = temp.path().join("Accelerator TestCases.reconstructed.xlsx");
    let error = reconstruct(
        &temp.path().join("missing-package"),
        &current,
        &output,
        false,
    )
    .expect_err("missing authoritative package must refuse");
    assert!(error.to_string().contains("incomplete"), "{error:#}");
    assert!(!output.exists());
    assert_eq!(fs::read(&current)?, b"input must remain unchanged");
    let workbooks = fs::read_dir(temp.path())?
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|value| value == "xlsx")
        })
        .count();
    assert_eq!(
        workbooks, 1,
        "refusal created a temporary or final workbook"
    );
    Ok(())
}

#[test]
fn retained_machine_path_redaction_keeps_formulas_and_untouched_parts() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let headers = vec!["Value".into()];
    let input = fixture(vec![(
        "Test Runs",
        sheet_xml(
            &headers,
            &[
                vec![json!("/home/tester/Desktop/AI-Infrastructure/status")],
                vec![json!("/Users/tester/work/accelerator-php/docs/source.json")],
                vec![json!("=SUM(B1:B2)")],
                vec![json!("https://example.test/stable")],
            ],
        ),
    )]);
    let mut book = reopened(&input, temp.path(), "input.xlsx")?;
    let opaque = book.parts["customXml/item1.xml"].clone();
    let binary = book.parts["xl/media/opaque.bin"].clone();
    book.sanitize()?;
    let output = reopened(&book, temp.path(), "output.xlsx")?;
    let rows = output.rows("Test Runs")?;
    assert_eq!(rows[1][0], "{repository}/status");
    assert_eq!(rows[2][0], "{repository}/docs/source.json");
    assert_eq!(rows[3][0], "=SUM(B1:B2)");
    assert_eq!(rows[4][0], "https://example.test/stable");
    assert_eq!(output.parts["customXml/item1.xml"], opaque);
    assert_eq!(output.parts["xl/media/opaque.bin"], binary);
    let mut again = output.clone();
    again.sanitize()?;
    assert_eq!(
        again.parts, output.parts,
        "sanitizing twice changes serialized XML"
    );
    Ok(())
}

#[test]
fn selected_rows_keep_styles_comments_hyperlinks_formulas_and_package_extensions() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut target = fixture(vec![(
        "Test Runs",
        format!(
            r#"<worksheet xmlns="{MAIN}" xmlns:r="{REL}" xmlns:k="urn:owner"><dimension ref="A1:G2"/><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Run ID</t></is></c><c r="B1" t="inlineStr"><is><t>Formula</t></is></c><c r="C1" t="inlineStr"><is><t>Link</t></is></c><c r="D1" t="inlineStr"><is><t>Text</t></is></c><c r="E1" t="inlineStr"><is><t>Number</t></is></c><c r="F1" t="inlineStr"><is><t>Flag</t></is></c></row><row r="2" ht="20" customHeight="1"><c r="A2" t="inlineStr"><is><t>RUN-001</t></is></c><c r="B2" s="0"><f>1+1</f><v>2</v></c><c r="C2" t="inlineStr"><is><t>Historical link</t></is></c><c r="D2" t="inlineStr"><is><t>Historical text</t></is></c><c r="E2"><v>7</v></c><c r="F2" t="b"><v>0</v></c><c r="G2" s="0"/></row></sheetData><mergeCells count="1"><mergeCell ref="A8:C8"/></mergeCells><hyperlinks><hyperlink ref="C2" r:id="rId1"/></hyperlinks><legacyDrawing r:id="rId3"/><tableParts count="1"><tablePart r:id="rId5"/></tableParts><extLst><ext uri="urn:owner"><k:payload k:flag="keep">kept</k:payload></ext></extLst></worksheet>"#
        ),
    )]);
    target.parts.insert("xl/worksheets/_rels/sheet1.xml.rels".into(), format!(r#"<Relationships xmlns="{PKGREL}"><Relationship Id="rId1" Type="{REL}/hyperlink" Target="https://historical.example.test/" TargetMode="External"/><Relationship Id="rId2" Type="{REL}/comments" Target="../comments1.xml"/><Relationship Id="rId3" Type="{REL}/vmlDrawing" Target="../drawings/vmlDrawing1.vml"/><Relationship Id="rId4" Type="urn:owner/custom" Target="../custom.xml"/><Relationship Id="rId5" Type="{REL}/table" Target="../tables/table1.xml"/></Relationships>"#).into_bytes());
    target.parts.insert("xl/comments1.xml".into(), format!(r#"<comments xmlns="{MAIN}"><authors><author>Historical author</author></authors><commentList><comment ref="C2" authorId="0"><text><t>Historical comment</t></text></comment></commentList></comments>"#).into_bytes());
    target.parts.insert("xl/drawings/vmlDrawing1.vml".into(), br##"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel"><v:shapetype id="_x0000_t202"/><v:shape id="_x0000_s1025" type="#_x0000_t202"><x:ClientData ObjectType="Note"><x:Row>1</x:Row><x:Column>2</x:Column><x:Anchor>2, 0, 1, 0, 4, 0, 3, 0</x:Anchor></x:ClientData></v:shape></xml>"##.to_vec());
    target.parts.insert("xl/tables/table1.xml".into(), format!(r#"<table xmlns="{MAIN}" id="1" name="oldName" displayName="oldName" ref="A1:G2"><autoFilter ref="A1:G2"/><tableColumns count="0"/><tableStyleInfo name="TableStyleMedium9" showRowStripes="0"/></table>"#).into_bytes());
    target
        .parts
        .insert("xl/custom.xml".into(), b"<owner><keep/></owner>".to_vec());

    let mut source = fixture(vec![(
        "Test Runs",
        format!(
            r#"<worksheet xmlns="{MAIN}" xmlns:r="{REL}"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Run ID</t></is></c><c r="B1" t="inlineStr"><is><t>Text</t></is></c><c r="C1" t="inlineStr"><is><t>Flag</t></is></c><c r="D1" t="inlineStr"><is><t>Link</t></is></c><c r="E1" t="inlineStr"><is><t>Formula</t></is></c><c r="F1" t="inlineStr"><is><t>Number</t></is></c></row><row r="2" ht="31.5" customHeight="1"><c r="A2" t="inlineStr"><is><t>RUN-20260803-001</t></is></c><c r="B2" t="s"><v>0</v></c><c r="C2" t="b"><v>1</v></c><c r="D2" t="inlineStr"><is><t>Source link</t></is></c><c r="E2" s="1"><f>SUM(F2,1)</f><v>13.5</v></c><c r="F2" s="1"><v>12.5</v></c></row><row r="3"><c r="A3" t="inlineStr"><is><t>RUN-NOT-SELECTED</t></is></c><c r="D3" t="inlineStr"><is><t>Unselected</t></is></c></row></sheetData><hyperlinks><hyperlink ref="D2" r:id="rId1" tooltip="Source tooltip"/><hyperlink ref="D3" r:id="rId4"/></hyperlinks><legacyDrawing r:id="rId3"/></worksheet>"#
        ),
    )]);
    source
        .parts
        .insert("xl/styles.xml".into(), source_styles().as_bytes().to_vec());
    source.parts.insert("xl/sharedStrings.xml".into(), format!(r#"<sst xmlns="{MAIN}" count="1" uniqueCount="1"><si><r><rPr><b/></rPr><t>Rich </t></r><r><t>text</t></r></si></sst>"#).into_bytes());
    source.parts.insert("xl/worksheets/_rels/sheet1.xml.rels".into(), format!(r#"<Relationships xmlns="{PKGREL}"><Relationship Id="rId1" Type="{REL}/hyperlink" Target="https://source.example.test/item?q=1&amp;x=2" TargetMode="External"/><Relationship Id="rId2" Type="{REL}/comments" Target="../comments1.xml"/><Relationship Id="rId3" Type="{REL}/vmlDrawing" Target="../drawings/vmlDrawing1.vml"/><Relationship Id="rId4" Type="{REL}/hyperlink" Target="https://unselected.example.test/" TargetMode="External"/></Relationships>"#).into_bytes());
    source.parts.insert("xl/comments1.xml".into(), format!(r#"<comments xmlns="{MAIN}"><authors><author>Source author</author></authors><commentList><comment ref="D2" authorId="0"><text><r><rPr><b/></rPr><t>Selected comment</t></r></text></comment><comment ref="D3" authorId="0"><text><t>Unselected comment</t></text></comment></commentList></comments>"#).into_bytes());
    source.parts.insert("xl/drawings/vmlDrawing1.vml".into(), br##"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel"><v:shapetype id="_x0000_t202"/><v:shape id="_x0000_s1025" type="#_x0000_t202"><x:ClientData ObjectType="Note"><x:Row>1</x:Row><x:Column>3</x:Column><x:Anchor>3, 0, 1, 0, 5, 0, 3, 0</x:Anchor></x:ClientData></v:shape><v:shape id="_x0000_s1026" type="#_x0000_t202"><x:ClientData ObjectType="Note"><x:Row>2</x:Row><x:Column>3</x:Column></x:ClientData></v:shape></xml>"##.to_vec());

    let mut target = reopened(&target, temp.path(), "target.xlsx")?;
    let source = reopened(&source, temp.path(), "source.xlsx")?;
    let untouched = [
        "customXml/item1.xml",
        "xl/media/opaque.bin",
        "xl/custom.xml",
        "xl/workbook.xml",
        "xl/_rels/workbook.xml.rels",
    ]
    .into_iter()
    .map(|part| (part, target.parts[part].clone()))
    .collect::<Vec<_>>();
    let original_row = target.rows("Test Runs")?[1].clone();
    let styles = target.merge_styles(&source)?;
    target.append(
        &source,
        "Test Runs",
        &BTreeSet::from(["RUN-20260803-001".into()]),
        6,
        &styles,
        false,
    )?;
    target.table("Test Runs", 6, "A1:F3", "tblTestRuns")?;
    let output = reopened(&target, temp.path(), "merged.xlsx")?;
    let rows = output.rows("Test Runs")?;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1], original_row[..6]);
    assert_eq!(
        rows[2],
        vec![
            json!("RUN-20260803-001"),
            json!("=SUM(F2,1)"),
            json!("Source link"),
            json!("Rich text"),
            json!(12.5),
            json!(true)
        ]
    );
    for (part, raw) in untouched {
        assert_eq!(
            output.parts[part], raw,
            "changed untouched package part {part}"
        );
    }
    let sheet = output.part("xl/worksheets/sheet1.xml")?;
    assert_eq!(
        attr(
            children(child(&sheet, "sheetData").unwrap(), "row")[2],
            "ht"
        ),
        "31.5"
    );
    assert_eq!(attr(cell(&sheet, "E3"), "s"), styles[1].to_string());
    assert_eq!(attr(cell(&sheet, "B3"), "s"), styles[1].to_string());
    assert!(
        child(child(cell(&sheet, "D3"), "is").unwrap(), "r").is_some(),
        "shared rich text flattened"
    );
    assert_eq!(
        attr(
            children(child(&sheet, "mergeCells").unwrap(), "mergeCell")[0],
            "ref"
        ),
        "A8:C8"
    );
    assert_eq!(content(child(&sheet, "extLst").unwrap()), "kept");
    let style = output.part("xl/styles.xml")?;
    let xf = children(child(&style, "cellXfs").unwrap(), "xf")[styles[1]];
    assert_eq!(attr(xf, "fontId"), "2");
    assert_eq!(attr(xf, "fillId"), "2");
    assert_eq!(attr(xf, "borderId"), "2");
    assert_eq!(
        attr(xf, "numFmtId"),
        "165",
        "colliding custom format ID was not remapped"
    );
    assert_eq!(
        attr(child(xf, "alignment").unwrap(), "horizontal"),
        "center"
    );
    assert_eq!(attr(child(xf, "protection").unwrap(), "locked"), "0");
    assert!(child(xf, "extLst").is_some());
    let nums = children(child(&style, "numFmts").unwrap(), "numFmt");
    assert_eq!(
        nums.iter()
            .map(|value| (attr(value, "numFmtId"), attr(value, "formatCode")))
            .collect::<Vec<_>>(),
        vec![
            ("164".into(), "yyyy-mm-dd".into()),
            ("165".into(), "0.000".into())
        ]
    );
    let hyperlinks = children(child(&sheet, "hyperlinks").unwrap(), "hyperlink");
    assert_eq!(hyperlinks.len(), 2, "copied an unselected hyperlink");
    let copied = hyperlinks
        .iter()
        .find(|link| attr(link, "ref") == "C3")
        .unwrap();
    assert_eq!(attr(copied, "tooltip"), "Source tooltip");
    let relationships = output.relationships("xl/worksheets/sheet1.xml")?;
    let relationship = children(&relationships, "Relationship")
        .into_iter()
        .find(|link| attr(link, "Id") == attr(copied, "id"))
        .unwrap();
    assert_eq!(
        attr(relationship, "Target"),
        "https://source.example.test/item?q=1&x=2"
    );
    assert_eq!(attr(relationship, "TargetMode"), "External");
    assert!(
        children(&relationships, "Relationship")
            .iter()
            .any(|link| attr(link, "Type") == "urn:owner/custom")
    );
    let comments = related_part(&output, "comments")?;
    assert_eq!(
        children(child(&comments, "authors").unwrap(), "author")
            .iter()
            .map(|value| content(value))
            .collect::<Vec<_>>(),
        vec!["Historical author", "Source author"]
    );
    let comment_list = children(child(&comments, "commentList").unwrap(), "comment");
    assert_eq!(comment_list.len(), 2, "copied an unselected comment");
    assert_eq!(attr(comment_list[0], "ref"), "C2");
    assert_eq!(content(comment_list[0]), "Historical comment");
    assert_eq!(attr(comment_list[1], "ref"), "C3");
    assert_eq!(attr(comment_list[1], "authorId"), "1");
    assert_eq!(content(comment_list[1]), "Selected comment");
    assert!(child(child(comment_list[1], "text").unwrap(), "r").is_some());
    let vml = related_part(&output, "vmlDrawing")?;
    let shapes = children(&vml, "shape");
    assert_eq!(shapes.len(), 2, "copied an unselected VML note");
    assert_ne!(attr(shapes[0], "id"), attr(shapes[1], "id"));
    let client = child(shapes[1], "ClientData").unwrap();
    assert_eq!(content(child(client, "Row").unwrap()), "2");
    assert_eq!(content(child(client, "Column").unwrap()), "2");
    assert_eq!(
        content(child(client, "Anchor").unwrap()),
        "2, 0, 2, 0, 4, 0, 4, 0"
    );
    let table = related_part(&output, "table")?;
    assert_eq!(attr(&table, "ref"), "A1:F3");
    assert_eq!(attr(&table, "name"), "tblTestRuns");
    assert_eq!(attr(&table, "displayName"), "tblTestRuns");
    assert_eq!(attr(child(&table, "autoFilter").unwrap(), "ref"), "A1:F3");
    assert_eq!(
        attr(child(&table, "tableStyleInfo").unwrap(), "name"),
        "TableStyleMedium9"
    );
    assert_eq!(
        children(child(&table, "tableColumns").unwrap(), "tableColumn").len(),
        6
    );
    assert_eq!(
        children(child(&sheet, "tableParts").unwrap(), "tablePart").len(),
        1
    );
    assert_eq!(source.rows("Test Runs")?.len(), 3, "source package changed");
    Ok(())
}

fn reconstruction_book(historical: bool) -> Package {
    let mut sheets = vec![];
    for ((name, _, width, _, _), old_count) in SPECS.iter().zip([74, 57, 53, 57, 26, 9]) {
        let count = if historical {
            old_count
        } else if *name == "Defects" {
            4
        } else {
            18
        };
        let mut headers = if *name == "Defects" {
            vec![
                "Defect ID".into(),
                "Run ID".into(),
                "Test Case ID".into(),
                "Result".into(),
            ]
        } else if *name == "Test Cases" {
            vec![
                "Test Case ID".into(),
                "Title".into(),
                "Defect ID".into(),
                "Result".into(),
            ]
        } else {
            vec![
                "Run ID".into(),
                "Test Case ID".into(),
                "Defect ID".into(),
                "Result".into(),
            ]
        };
        while headers.len() < *width {
            headers.push(format!("Column {}", headers.len() + 1));
        }
        let mut rows = vec![];
        for index in 1..=count {
            let mut row = vec![Value::Null; *width];
            if *name == "Defects" {
                let run = if historical {
                    format!("RUN-{index:03}")
                } else {
                    format!("RUN-20260803-{:03}", index + 10)
                };
                row[0] = json!(if historical {
                    format!("DEF-H-{index:03}")
                } else {
                    format!("DEF-{index:03}")
                });
                row[1] = json!(run);
                row[2] = json!(if historical {
                    format!("TC-H-{index:03}")
                } else {
                    format!("TC-AI-{:03}", index + 10)
                });
            } else if *name == "Test Cases" {
                row[0] = json!(if historical {
                    format!("TC-H-{index:03}")
                } else {
                    format!("TC-AI-{index:03}")
                });
                row[1] = json!(format!("Supplied case {index}"));
            } else {
                row[0] = json!(if historical {
                    format!("RUN-{index:03}")
                } else {
                    format!("RUN-20260803-{index:03}")
                });
                row[1] = json!(if historical {
                    format!("TC-H-{index:03}")
                } else {
                    format!("TC-AI-{index:03}")
                });
                if !historical && (11..=14).contains(&index) {
                    row[2] = json!(format!("DEF-{:03}", index - 10));
                }
            }
            row[3] = json!(["Pass", "Partial", "Fail", "Blocked"][(index - 1) % 4]);
            rows.push(row);
        }
        sheets.push((*name, sheet_xml(&headers, &rows)));
    }
    sheets.push(("Dashboard", format!(r#"<worksheet xmlns="{MAIN}"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Supplied historical dashboard</t></is></c></row></sheetData></worksheet>"#)));
    fixture(sheets)
}

fn primary_fixture(directory: &Path) -> Result<(PathBuf, PathBuf)> {
    let evidence = directory.join("evidence");
    fs::create_dir_all(evidence.join("manifests"))?;
    let snapshot = evidence.join("manifests/workbook-snapshot-20260803-2002.xlsx");
    reconstruction_book(true).write(&snapshot)?;
    let current = directory.join("current.xlsx");
    reconstruction_book(false).write(&current)?;
    fs::write(
        evidence.join("ACCELERATOR-TEST-EVIDENCE-REPORT.md"),
        b"# Synthetic authoritative fixture\n",
    )?;
    let runs = (1..=57)
        .map(|index| json!({"Run ID": format!("RUN-{index:03}")}))
        .collect::<Vec<_>>();
    fs::write(
        evidence.join("manifests/historical-runs.json"),
        serde_json::to_vec(&json!({"runs": runs}))?,
    )?;
    let cases = (1..=74)
        .map(|index| json!({"test_case_id": format!("TC-H-{index:03}")}))
        .collect::<Vec<_>>();
    fs::write(
        evidence.join("manifests/historical-testcases.json"),
        serde_json::to_vec(&json!({"test_cases": cases}))?,
    )?;
    for name in [
        "historical-othersheets",
        "defects",
        "cases_map",
        "bwb_cases",
    ] {
        fs::write(evidence.join(format!("manifests/{name}.json")), b"{}\n")?;
    }
    let sums = evidence.join("SHA256SUMS.txt");
    fs::write(&sums, crate::manifest::render(&evidence, &sums)?)?;
    Ok((evidence, current))
}

#[test]
fn reconstruct_preserves_frozen_observations_and_publishes_complete_75_run_package() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (evidence, current) = primary_fixture(temp.path())?;
    let snapshot = evidence.join("manifests/workbook-snapshot-20260803-2002.xlsx");
    let snapshot_raw = fs::read(&snapshot)?;
    let current_raw = fs::read(&current)?;
    let old = Package::read(&snapshot)?;
    let supplied = Package::read(&current)?;
    let output = temp.path().join("new/nested/reconstructed.xlsx");
    assert!(!output.parent().unwrap().exists());
    reconstruct(&evidence, &current, &output, false)?;
    assert!(output.parent().unwrap().is_dir());
    let result = Package::read(&output)?;
    assert_eq!(validate(&result)?, Vec::<String>::new());
    assert_eq!(
        fs::read(&snapshot)?,
        snapshot_raw,
        "rewrote historical source"
    );
    assert_eq!(fs::read(&current)?, current_raw, "rewrote current source");
    let runs = result.records("Test Runs")?;
    let historical = old.records("Test Runs")?;
    assert_eq!(
        &runs[..57],
        historical.as_slice(),
        "rewrote frozen historical grades or values"
    );
    let originals = supplied.records("Test Runs")?;
    for (index, record) in runs[57..].iter().enumerate() {
        assert_eq!(
            record["Result"], originals[index]["Result"],
            "invented a newer outcome"
        );
    }
    assert_eq!(
        runs.iter()
            .map(|row| text(&row["Run ID"]))
            .collect::<BTreeSet<_>>(),
        all_runs()
    );
    for (run, case, old_defect, namespace) in [
        ("RUN-20260803-011", "TC-AI-011", "DEF-001", "DEF-AI-001"),
        ("RUN-20260803-012", "TC-AI-012", "DEF-002", "DEF-AI-002"),
        ("RUN-20260803-013", "TC-AI-013", "DEF-003", "DEF-AI-003"),
        ("RUN-20260803-014", "TC-AI-014", "DEF-004", "DEF-AI-004"),
    ] {
        let record = runs.iter().find(|row| row["Run ID"] == run).unwrap();
        assert_eq!(record["Test Case ID"], case);
        assert_eq!(record["Defect ID"], namespace);
        assert!(
            result
                .records("Defects")?
                .iter()
                .any(|row| row["Run ID"] == run
                    && row["Test Case ID"] == case
                    && row["Defect ID"] == namespace)
        );
        assert!(
            originals
                .iter()
                .any(|row| row["Run ID"] == run && row["Defect ID"] == old_defect)
        );
    }
    assert_eq!(
        result.parts["customXml/item1.xml"],
        old.parts["customXml/item1.xml"]
    );
    assert_eq!(
        result.parts["xl/media/opaque.bin"],
        old.parts["xl/media/opaque.bin"]
    );
    let dashboard = result.rows("Dashboard")?;
    assert_eq!(dashboard[3][1], "=ROWS(tblTestRuns[Run ID])");
    assert_eq!(
        dashboard[17][1],
        "=COUNTIFS(tblDefects[Severity],\"Critical\",tblDefects[Status],\"<>Closed\",tblDefects[Status],\"<>Rejected\")"
    );
    let workbook = result.part("xl/workbook.xml")?;
    assert_eq!(
        attr(child(&workbook, "calcPr").unwrap(), "fullCalcOnLoad"),
        "1"
    );
    assert_eq!(
        attr(child(&workbook, "calcPr").unwrap(), "forceFullCalc"),
        "1"
    );
    let provenance = result.rows("Provenance")?;
    assert_eq!(
        provenance[1][0],
        "manifests/workbook-snapshot-20260803-2002.xlsx"
    );
    assert_eq!(provenance[1][1], hash_file(&snapshot)?);
    assert_eq!(provenance[3][1], hash_file(&current)?);
    let output_raw = fs::read(&output)?;
    assert!(
        reconstruct(&evidence, &current, &output, false).is_err(),
        "overwrote existing output without --force"
    );
    assert_eq!(fs::read(&output)?, output_raw);
    assert!(
        reconstruct(&evidence, &current, &current, true).is_err(),
        "--force overwrote a source workbook"
    );
    assert_eq!(fs::read(&current)?, current_raw);
    let missing_alias = temp.path().join("missing/../current.xlsx");
    assert!(reconstruct(&evidence, &current, &missing_alias, true).is_err());
    assert!(!temp.path().join("missing").exists());
    assert_eq!(fs::read(&current)?, current_raw);
    #[cfg(unix)]
    {
        let alias = temp.path().join("current-alias.xlsx");
        std::os::unix::fs::symlink(&current, &alias)?;
        assert!(reconstruct(&evidence, &current, &alias, true).is_err());
        assert!(fs::symlink_metadata(&alias)?.file_type().is_symlink());
        assert_eq!(fs::read(&current)?, current_raw);
    }
    Ok(())
}

#[test]
fn checksum_failure_does_not_publish_a_partial_workbook() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (evidence, current) = primary_fixture(temp.path())?;
    fs::write(
        evidence.join("manifests/historical-othersheets.json"),
        b"{\"tampered\":true}\n",
    )?;
    let output = temp.path().join("not-created/nested/refused.xlsx");
    let before = fs::read_dir(temp.path())?
        .map(|entry| entry.map(|value| value.file_name()))
        .collect::<std::io::Result<BTreeSet<_>>>()?;
    let error =
        reconstruct(&evidence, &current, &output, false).expect_err("tampered evidence admitted");
    assert!(error.to_string().contains("checksum mismatch"), "{error:#}");
    assert!(!output.exists());
    let after = fs::read_dir(temp.path())?
        .map(|entry| entry.map(|value| value.file_name()))
        .collect::<std::io::Result<BTreeSet<_>>>()?;
    assert_eq!(
        before, after,
        "refused reconstruction leaked a temporary file"
    );
    Ok(())
}

#[test]
fn package_reader_refuses_paths_outside_opc_and_external_worksheets() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let headers = vec!["Run ID".into()];
    let input = fixture(vec![("Test Runs", sheet_xml(&headers, &[]))]);
    let mut escaped = input.clone();
    escaped
        .parts
        .insert("../escaped.xml".into(), b"<escaped/>".to_vec());
    let path = temp.path().join("escaped.xlsx");
    escaped.write(&path)?;
    assert!(
        Package::read(&path).is_err(),
        "admitted an OPC traversal name"
    );
    let mut external = input.clone();
    external.parts.insert("xl/_rels/workbook.xml.rels".into(), format!(
        r#"<Relationships xmlns="{PKGREL}"><Relationship Id="rId1" Type="{REL}/worksheet" Target="https://outside.example.test/sheet.xml" TargetMode="External"/></Relationships>"#
    ).into_bytes());
    let path = temp.path().join("external.xlsx");
    external.write(&path)?;
    assert!(
        Package::read(&path).is_err(),
        "admitted an external worksheet relationship"
    );
    assert!(!temp.path().join("escaped.xml").exists());
    Ok(())
}

#[test]
fn xml_character_references_preserve_headers_text_and_formula_operators() -> Result<()> {
    let raw = br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Context &amp; Brain</t></is></c><c r="B1"><f>IF(A2&lt;&gt;0,&quot;A&amp;B&quot;,&quot;&#x3B1;&quot;)</f><v>0</v></c><c r="C1" t="inlineStr"><is><t>&#65;&#x42;&apos;&quot;&lt;&gt;&amp;</t></is></c></row></sheetData></worksheet>"#;
    let book = fixture(vec![("Test Runs", std::str::from_utf8(raw)?.to_owned())]);
    let rows = book.rows("Test Runs")?;
    assert_eq!(rows[0][0], "Context & Brain");
    assert_eq!(rows[0][1], "=IF(A2<>0,\"A&B\",\"α\")");
    assert_eq!(rows[0][2], "AB'\"<>&");
    let temp = tempfile::tempdir()?;
    let roundtrip = reopened(&book, temp.path(), "entities.xlsx")?;
    assert_eq!(roundtrip.rows("Test Runs")?, rows);
    assert!(xml(br#"<x>&not_a_defined_entity;</x>"#).is_err());
    Ok(())
}

#[test]
fn selected_shared_followers_are_materialized_at_source_coordinates() -> Result<()> {
    let target_xml = format!(
        r#"<worksheet xmlns="{MAIN}"><sheetData>
<row r="1"><c r="A1" t="inlineStr"><is><t>Run ID</t></is></c><c r="B1" t="inlineStr"><is><t>Formula</t></is></c></row>
<row r="2"><c r="A2" t="inlineStr"><is><t>OLD-1</t></is></c><c r="B2"><f t="shared" si="7" ref="B2:B3">F2+$G$1</f><v>1</v></c></row>
<row r="3"><c r="A3" t="inlineStr"><is><t>OLD-2</t></is></c><c r="B3"><f t="shared" si="7"/><v>2</v></c></row>
</sheetData></worksheet>"#
    );
    let expression =
        r#"IF(A2>0,LOG10($A2)+'Sheet 2'!B2+SUM(C2:D2)+SUM(E:E)+SUM(2:2),"A2 ""quoted""")"#;
    let source_xml = format!(
        r#"<worksheet xmlns="{MAIN}"><sheetData>
<row r="1"><c r="A1" t="inlineStr"><is><t>Run ID</t></is></c><c r="B1" t="inlineStr"><is><t>Formula</t></is></c></row>
<row r="2"><c r="A2" t="inlineStr"><is><t>NOT-SELECTED</t></is></c><c r="B2"><f t="shared" si="7" ref="B2:B3">{}</f><v>11</v></c></row>
<row r="3"><c r="A3" t="inlineStr"><is><t>RUN-20260803-001</t></is></c><c r="B3"><f t="shared" si="7"/><v>12</v></c></row>
</sheetData></worksheet>"#,
        escaped(expression)
    );
    let mut target = fixture(vec![("Test Runs", target_xml)]);
    let source = fixture(vec![("Test Runs", source_xml)]);
    let source_image = source.parts.clone();
    let opaque = target.parts["xl/media/opaque.bin"].clone();
    target.append(
        &source,
        "Test Runs",
        &BTreeSet::from(["RUN-20260803-001".into()]),
        2,
        &[0],
        false,
    )?;
    let expected =
        r#"=IF(A3>0,LOG10($A3)+'Sheet 2'!B3+SUM(C3:D3)+SUM(E:E)+SUM(3:3),"A2 ""quoted""")"#;
    assert_eq!(target.rows("Test Runs")?[3][1], expected);
    assert_eq!(target.rows("Test Runs")?[2][1], "=F3+$G$1");
    assert_eq!(
        source.parts, source_image,
        "normalization rewrote source XML"
    );
    let temp = tempfile::tempdir()?;
    let result = reopened(&target, temp.path(), "shared-formulas.xlsx")?;
    let sheet = xml(&result.parts[&result.sheets["Test Runs"]])?;
    for address in ["B2", "B3", "B4"] {
        let formula = child(cell(&sheet, address), "f").context("copied formula")?;
        for attribute in ["t", "si", "ref"] {
            assert!(
                !formula.attributes.contains_key(attribute),
                "{address}: {attribute}"
            );
        }
    }
    assert_eq!(
        content(child(cell(&sheet, "B4"), "f").unwrap()),
        &expected[1..]
    );
    assert_eq!(result.parts["xl/media/opaque.bin"], opaque);
    Ok(())
}

#[test]
fn shared_reference_translation_keeps_literals_names_and_absolute_axes() -> Result<()> {
    let expression = r#"SUM($A1,A$1,$A$1,A1:A2,A:A,1:2,'O''Brien: Sheet'!A1,[Book.xlsx]Sheet1!A1,Table1[A1],Named_A1)+LOG10(A1)+"A1 ""literal""""#;
    let expected = r#"SUM($A3,C$1,$A$1,C3:C4,C:C,3:4,'O''Brien: Sheet'!C3,[Book.xlsx]Sheet1!C3,Table1[A1],Named_A1)+LOG10(C3)+"A1 ""literal""""#;
    assert_eq!(compat::translate_formula(expression, "B2", "D4")?, expected);
    assert_eq!(
        compat::translate_formula(expression, "B2", "B2")?,
        expression
    );
    assert_eq!(
        compat::translate_formula("SUM(A1 (B2))", "B2", "D4")?,
        "SUM(C3 (D4))"
    );
    assert!(
        compat::translate_formula("A1", "B2", "A1").is_err(),
        "wrapped a negative reference"
    );
    Ok(())
}

#[test]
fn implicit_coordinates_are_shared_by_rows_records_and_append() -> Result<()> {
    let headers = ["Run ID", "Field 2", "Field 3", "Field 4"]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let source_xml = format!(
        r#"<worksheet xmlns="{MAIN}"><sheetData>
<row><c t="inlineStr"><is><t>Run ID</t></is></c><c t="inlineStr"><is><t>Field 2</t></is></c><c t="inlineStr"><is><t>Field 3</t></is></c><c t="inlineStr"><is><t>Field 4</t></is></c></row>
<row r="4"><c r="A4" t="inlineStr"><is><t>RUN-20260803-001</t></is></c><c r="C4" t="inlineStr"><is><t>explicit C4</t></is></c><c t="inlineStr"><is><t>implicit D4</t></is></c></row>
<row><c t="inlineStr"><is><t>NOT-SELECTED</t></is></c><c t="inlineStr"><is><t>implicit B5</t></is></c></row>
</sheetData></worksheet>"#
    );
    let source = fixture(vec![("Test Runs", source_xml)]);
    let source_image = source.parts.clone();
    let rows = source.rows("Test Runs")?;
    assert_eq!(rows.len(), 5);
    assert_eq!(
        rows[0],
        headers.iter().map(|value| json!(value)).collect::<Vec<_>>()
    );
    assert_eq!(
        rows[3],
        vec![
            json!("RUN-20260803-001"),
            Value::Null,
            json!("explicit C4"),
            json!("implicit D4")
        ]
    );
    assert_eq!(rows[4][1], "implicit B5");
    let record = source
        .records("Test Runs")?
        .into_iter()
        .find(|row| row["Run ID"] == "RUN-20260803-001")
        .context("implicit record")?;
    assert_eq!(record["Field 4"], "implicit D4");
    let mut target = fixture(vec![("Test Runs", sheet_xml(&headers, &[]))]);
    target.append(
        &source,
        "Test Runs",
        &BTreeSet::from(["RUN-20260803-001".into()]),
        4,
        &[0],
        false,
    )?;
    let temp = tempfile::tempdir()?;
    let result = reopened(&target, temp.path(), "implicit-coordinates.xlsx")?;
    assert_eq!(result.rows("Test Runs")?[1], rows[3]);
    assert_eq!(
        source.parts, source_image,
        "filled coordinates in source bytes"
    );
    let normalized = source.part(&source.sheets["Test Runs"])?;
    assert_eq!(attr(cell(&normalized, "D4"), "r"), "D4");
    assert_eq!(attr(cell(&normalized, "B5"), "r"), "B5");
    for invalid in [
        format!(r#"<worksheet xmlns="{MAIN}"><sheetData><row r="0"/></sheetData></worksheet>"#),
        format!(
            r#"<worksheet xmlns="{MAIN}"><sheetData><row r="1"><c r="XFD1"/><c/></row></sheetData></worksheet>"#
        ),
    ] {
        assert!(
            fixture(vec![("Test Runs", invalid)])
                .rows("Test Runs")
                .is_err()
        );
    }
    Ok(())
}

#[test]
fn new_table_dimension_and_hyperlinks_obey_spreadsheetml_child_order() -> Result<()> {
    let headers = vec!["Run ID".into(), "Link".into()];
    let target_xml = sheet_xml(&headers, &[]).replace(
        "</worksheet>",
        r#"<extLst><ext uri="urn:owner"><owner:payload xmlns:owner="urn:owner" keep="yes">retained</owner:payload></ext></extLst></worksheet>"#,
    );
    let mut target = fixture(vec![("Test Runs", target_xml)]);
    let opaque = target.parts["customXml/item1.xml"].clone();
    target.table("Test Runs", 2, "A1:B1", "tblRuns")?;
    let mut source = fixture(vec![(
        "Test Runs",
        sheet_xml(
            &headers,
            &[vec![json!("RUN-20260803-001"), json!("Source link")]],
        )
        .replace(
            "</worksheet>",
            r#"<hyperlinks><hyperlink ref="B2" r:id="rId1"/></hyperlinks></worksheet>"#,
        ),
    )]);
    source.parts.insert(rel_path(&source.sheets["Test Runs"]), format!(
        r#"<Relationships xmlns="{PKGREL}"><Relationship Id="rId1" Type="{REL}/hyperlink" Target="https://example.test/source" TargetMode="External"/></Relationships>"#
    ).into_bytes());
    let source_image = source.parts.clone();
    target.append(
        &source,
        "Test Runs",
        &BTreeSet::from(["RUN-20260803-001".into()]),
        2,
        &[0],
        false,
    )?;
    target.table("Test Runs", 2, "A1:B2", "tblRuns")?;
    let temp = tempfile::tempdir()?;
    let result = reopened(&target, temp.path(), "ordered-elements.xlsx")?;
    let (_, table_path) = result
        .find_related(&result.sheets["Test Runs"], "table")?
        .context("table relationship")?;
    let table = xml(&result.parts[&table_path])?;
    let names = |element: &Element| {
        element
            .children
            .iter()
            .filter_map(XMLNode::as_element)
            .map(|element| local(&element.name).to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(&table),
        ["autoFilter", "tableColumns", "tableStyleInfo"]
    );
    let sheet = xml(&result.parts[&result.sheets["Test Runs"]])?;
    assert_eq!(
        names(&sheet),
        [
            "dimension",
            "sheetData",
            "hyperlinks",
            "tableParts",
            "extLst"
        ]
    );
    assert_eq!(content(child(&sheet, "extLst").unwrap()), "retained");
    assert_eq!(attr(child(&sheet, "dimension").unwrap(), "ref"), "A1:B2");
    assert_eq!(result.parts["customXml/item1.xml"], opaque);
    assert_eq!(source.parts, source_image);
    Ok(())
}

#[test]
fn copied_formula_caches_are_cleared_without_rewriting_supplied_results() -> Result<()> {
    let headers = [
        "Run ID",
        "Numeric formula",
        "Text formula",
        "Boolean formula",
        "Shared formula",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let source_xml = format!(
        r#"<worksheet xmlns="{MAIN}"><sheetData>
<row r="1"><c r="A1" t="inlineStr"><is><t>Run ID</t></is></c><c r="B1" t="inlineStr"><is><t>Numeric formula</t></is></c><c r="C1" t="inlineStr"><is><t>Text formula</t></is></c><c r="D1" t="inlineStr"><is><t>Boolean formula</t></is></c><c r="E1" t="inlineStr"><is><t>Shared formula</t></is></c></row>
<row r="2"><c r="A2" t="inlineStr"><is><t>NOT-SELECTED</t></is></c><c r="E2"><f t="shared" si="9" ref="E2:E3">B2+1</f><v>11</v></c></row>
<row r="3"><c r="A3" t="inlineStr"><is><t>RUN-20260803-001</t></is></c><c r="B3"><f>SUM(1,2)</f><v>3</v></c><c r="C3" t="str"><f>&quot;supplied result&quot;</f><v>supplied result</v></c><c r="D3" t="b"><f>1=1</f><v>1</v></c><c r="E3"><f t="shared" si="9"/><v>4</v></c></row>
</sheetData></worksheet>"#
    );
    let source = fixture(vec![("Test Runs", source_xml)]);
    let original = source.parts.clone();
    let mut target = fixture(vec![("Test Runs", sheet_xml(&headers, &[]))]);
    target.append(
        &source,
        "Test Runs",
        &BTreeSet::from(["RUN-20260803-001".into()]),
        5,
        &[0],
        false,
    )?;
    let temp = tempfile::tempdir()?;
    let result = reopened(&target, temp.path(), "no-copied-caches.xlsx")?;
    let target_xml = xml(&result.parts[&result.sheets["Test Runs"]])?;
    for (address, expression) in [
        ("B2", "SUM(1,2)"),
        ("C2", "\"supplied result\""),
        ("D2", "1=1"),
        ("E2", "B3+1"),
    ] {
        let copied = cell(&target_xml, address);
        assert_eq!(
            content(child(copied, "f").context("destination formula")?),
            expression
        );
        assert!(
            child(copied, "v").is_none(),
            "{address}: copied stale cache"
        );
        assert!(
            !copied.attributes.contains_key("t"),
            "{address}: kept cached result type"
        );
    }
    assert_eq!(source.parts, original, "discarded a supplied source cache");
    let supplied_xml = xml(&source.parts[&source.sheets["Test Runs"]])?;
    for (address, cached) in [
        ("B3", "3"),
        ("C3", "supplied result"),
        ("D3", "1"),
        ("E3", "4"),
    ] {
        assert_eq!(
            content(child(cell(&supplied_xml, address), "v").unwrap()),
            cached
        );
    }
    assert_eq!(attr(cell(&supplied_xml, "C3"), "t"), "str");
    assert_eq!(attr(cell(&supplied_xml, "D3"), "t"), "b");
    Ok(())
}

#[test]
fn shared_formula_expansion_is_admitted_before_any_expression_is_rewritten() -> Result<()> {
    let source = fixture(vec![(
        "Shared Budget",
        format!(
            r#"<worksheet xmlns="{MAIN}"><sheetData>
<row r="1"><c r="A1"><f t="shared" si="7" ref="A1:A3">&quot;xx&quot;</f><v>10</v></c></row>
<row r="2"><c r="A2"><f t="shared" si="7"/><v>20</v></c></row>
<row r="3"><c r="A3"><f t="shared" si="7"/><v>30</v></c></row>
</sheetData></worksheet>"#
        ),
    )]);
    let temp = tempfile::tempdir()?;
    let image = reopened(&source, temp.path(), "bounded-shared.xlsx")?;
    let part = &image.parts[&image.sheets["Shared Budget"]];
    let original = fs::read(temp.path().join("bounded-shared.xlsx"))?;
    // Four-byte quoted literal * three cells needs twelve retained bytes.
    // Work admission also covers the master scan and each attempted translation.
    for (bytes, work, reason) in [
        (11, 1024, "shared formula expansion byte bound"),
        (3, 1024, "shared formula expansion byte bound"),
        (1024, 79, "shared formula normalization work bound"),
    ] {
        let mut parsed = xml(part)?;
        let before = encode(&parsed)?;
        let error = compat::worksheet_test_limits(&mut parsed, bytes, work).unwrap_err();
        assert!(error.to_string().contains(reason), "{error:#}");
        assert_eq!(
            encode(&parsed)?,
            before,
            "partial shared rewrite before denial"
        );
        for address in ["A1", "A2", "A3"] {
            let formula = child(cell(&parsed, address), "f").unwrap();
            assert_eq!(attr(formula, "t"), "shared");
            assert_eq!(attr(formula, "si"), "7");
        }
    }
    assert_eq!(fs::read(temp.path().join("bounded-shared.xlsx"))?, original);
    Ok(())
}

#[test]
fn shared_literal_formula_budget_preserves_normal_boundary_and_caches() -> Result<()> {
    let source = fixture(vec![(
        "Shared Budget",
        format!(
            r#"<worksheet xmlns="{MAIN}"><sheetData>
<row r="1"><c r="A1"><f t="shared" si="7" ref="A1:A3">&quot;xx&quot;</f><v>10</v></c></row>
<row r="2"><c r="A2"><f t="shared" si="7"/><v>20</v></c></row>
<row r="3"><c r="A3"><f t="shared" si="7"/><v>30</v></c></row>
</sheetData></worksheet>"#
        ),
    )]);
    let temp = tempfile::tempdir()?;
    let image = reopened(&source, temp.path(), "admitted-shared.xlsx")?;
    let mut parsed = xml(&image.parts[&image.sheets["Shared Budget"]])?;
    compat::worksheet_test_limits(&mut parsed, 12, 1024)?;
    for (address, cached) in [("A1", "10"), ("A2", "20"), ("A3", "30")] {
        let c = cell(&parsed, address);
        let f = child(c, "f").unwrap();
        assert_eq!(content(f), "\"xx\"");
        assert!(!f.attributes.contains_key("t"));
        assert_eq!(content(child(c, "v").unwrap()), cached);
    }
    Ok(())
}
