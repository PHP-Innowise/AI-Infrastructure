use super::*;
use std::io::{Cursor, Write};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
fn attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn standard_styles() -> String {
    let codes = [
        (164, "$#,##0.00;($#,##0.00);\"-\""),
        (165, "yyyy-mm-dd hh:mm"),
        (166, "]General"),
        (167, "0;0;0;\"prefix:\"@"),
        (168, "0;;"),
        (169, "0*x"),
        (170, "0* "),
        (171, "0**"),
        (172, "0.000000000000"),
        (173, "é]General"),
    ];
    let ids = [
        2, 10, 165, 164, 0, 2, 5, 6, 7, 8, 32, 56, 166, 500, 167, 168, 169, 170, 171, 14, 172, 173,
    ];
    let mut styles = format!("<styleSheet xmlns=\"{MAIN}\"><numFmts>");
    for (id, code) in codes {
        styles.push_str(&format!(
            "<numFmt numFmtId=\"{id}\" formatCode=\"{}\"/>",
            attr(code)
        ));
    }
    styles.push_str("</numFmts><cellStyleXfs><xf numFmtId=\"10\"/></cellStyleXfs><cellXfs>");
    for (index, id) in ids.into_iter().enumerate() {
        if index == 4 {
            styles.push_str("<xf xfId=\"0\"/>");
        } else {
            styles.push_str(&format!(
                "<xf numFmtId=\"{id}\"{} />",
                if index == 5 {
                    " xfId=\"0\" applyNumberFormat=\"0\""
                } else {
                    ""
                }
            ));
        }
    }
    styles.push_str("</cellXfs></styleSheet>");
    styles
}
fn book(rows: &str, date1904: bool, styles: &str, conflicting: bool) -> TestResult<Vec<u8>> {
    book_range(rows, date1904, styles, conflicting, "A1:Z1")
}
fn book_range(
    rows: &str,
    date1904: bool,
    styles: &str,
    conflicting: bool,
    range: &str,
) -> TestResult<Vec<u8>> {
    let selected = if conflicting {
        "xl/selected-styles.xml"
    } else {
        "xl/styles.xml"
    };
    let extra = if conflicting {
        "<Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>"
    } else {
        ""
    };
    let mut parts = vec![
        (
            "[Content_Types].xml".to_owned(),
            format!(
                r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/><Override PartName="/{selected}" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>{extra}</Types>"#
            ),
        ),
        (
            "_rels/.rels".into(),
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/workbook.xml".into(),
            format!(
                r#"<workbook xmlns="{MAIN}" xmlns:r="{REL}"><workbookPr date1904="{}"/><sheets><sheet name="Fixture" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
                if date1904 { 1 } else { 0 }
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels".into(),
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="{REL}/styles" Target="styles.xml"/><Relationship Id="rId3" Type="{REL}/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/worksheets/sheet1.xml".into(),
            format!(
                r#"<worksheet xmlns="{MAIN}"><dimension ref="{range}"/><sheetData>{rows}</sheetData></worksheet>"#
            ),
        ),
        (
            "xl/sharedStrings.xml".into(),
            format!(r#"<sst xmlns="{MAIN}"><si><t>00α</t></si></sst>"#),
        ),
        (selected.into(), styles.to_owned()),
    ];
    if conflicting {
        parts.push((
            "xl/styles.xml".into(),
            format!(
                r#"<styleSheet xmlns="{MAIN}"><cellXfs><xf numFmtId="10"/></cellXfs></styleSheet>"#
            ),
        ));
    }
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in parts {
        writer.start_file(
            name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )?;
        writer.write_all(data.as_bytes())?;
    }
    Ok(writer.finish()?.into_inner())
}
fn projected(bytes: &[u8]) -> TestResult<Value> {
    Ok(crate::workbook_preview::read_bytes(bytes)?.remove(0))
}
fn cell(address: &str, style: Option<usize>, value: &str) -> String {
    format!(
        "<c r=\"{address}\"{}><v>{}</v></c>",
        style
            .map(|value| format!(" s=\"{value}\""))
            .unwrap_or_default(),
        attr(value)
    )
}
fn warning(sheet: &Value, code: &str) -> bool {
    sheet["format_warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["code"] == code)
}

#[test]
fn exact_reference_numeric_style_selection_and_builtin_fallbacks() -> TestResult {
    // Captured by Root from the SRI-verified0.20.3 read→sheet_to_html oracle.
    let specs = [
        ("A1", None, "1", "1"),
        ("B1", Some(0), "1", "1.00"),
        ("C1", Some(4), "0.25", "0.25"),
        ("D1", Some(5), "1", "1.00"),
        ("E1", Some(99), "1", "1"),
        ("F1", Some(6), "1234.5", "$1,235 "),
        ("G1", Some(7), "-1234.5", "($1,235)"),
        ("H1", Some(8), "1234.5", "$1,234.50 "),
        ("I1", Some(9), "-1234.5", "($1,234.50)"),
        ("J1", Some(10), "0.5", "12:00:00"),
        ("K1", Some(11), "0.5", "上午/下午 12時00分00秒 "),
        ("L1", Some(13), "1.25", "1.25"),
    ];
    let cells = specs
        .iter()
        .map(|(address, style, value, _)| cell(address, *style, value))
        .collect::<String>();
    let bytes = book(
        &format!("<row r=\"1\">{cells}</row>"),
        false,
        &standard_styles(),
        false,
    )?;
    let sheet = projected(&bytes)?;
    for (index, (_, _, _, expected)) in specs.iter().enumerate() {
        assert_eq!(sheet["display"][format!("0:{index}")], *expected);
    }
    assert_eq!(sheet["rows"][0][5], 1234.5);
    Ok(())
}

#[test]
fn exact_reference_dates_include_both_epochs_zero_fraction_and_phantom_day() -> TestResult {
    for (epoch, expected) in [
        (
            false,
            [
                "1900-01-00 00:00",
                "1900-01-00 12:00",
                "1900-02-29 00:00",
                "1/0/00",
                "2/29/00",
            ],
        ),
        (
            true,
            [
                "1904-01-01 00:00",
                "1904-01-01 12:00",
                "1904-03-01 00:00",
                "1/1/04",
                "3/1/04",
            ],
        ),
    ] {
        let cells = [
            ("A1", 2, "0"),
            ("B1", 2, "0.5"),
            ("C1", 2, "60"),
            ("D1", 19, "0"),
            ("E1", 19, "60"),
        ]
        .into_iter()
        .map(|(address, style, value)| cell(address, Some(style), value))
        .collect::<String>();
        let bytes = book(
            &format!("<row r=\"1\">{cells}</row>"),
            epoch,
            &standard_styles(),
            false,
        )?;
        let sheet = projected(&bytes)?;
        for (index, expected) in expected.into_iter().enumerate() {
            assert_eq!(sheet["display"][format!("0:{index}")], expected);
        }
        assert_eq!(sheet["rows"][0][0], 0);
        assert_eq!(sheet["rows"][0][1], 0.5);
    }
    Ok(())
}

#[test]
fn actual_html_contract_preserves_stale_typed_cache_and_raw_text_priority() -> TestResult {
    let rows = r#"<row r="1">
<c r="A1" s="1"><f>1/8</f><v>0.25</v></c>
<c r="B1" t="s"><f>&quot;00α&quot;</f><v>0</v></c>
<c r="C1" t="b"><f>1=1</f><v>1</v></c>
<c r="D1" t="e"><f>1/0</f><v>#DIV/0!</v></c>
<c r="E1" t="str" s="14"><f>&quot;abc&quot;</f><v>abc</v></c>
<c r="F1" t="str" s="0"><f>&quot;00123&quot;</f><v>00123</v></c>
</row>"#;
    let bytes = book(rows, false, &standard_styles(), false)?;
    let sheet = projected(&bytes)?;
    assert_eq!(sheet["rows"][0][0], 0.25); // Deliberately stale1/8 cache, no evaluation.
    assert_eq!(sheet["display"]["0:0"], "25.00%");
    assert_eq!(sheet["formulas"]["0:0"], "1/8");
    assert_eq!(sheet["rows"][0][1], "00α");
    assert_eq!(sheet["rows"][0][2], "TRUE");
    assert_eq!(sheet["rows"][0][3], "#DIV/0!");
    assert_eq!(sheet["rows"][0][4], "abc");
    assert!(sheet["display"].get("0:4").is_none()); // Canonical h=abc wins w=prefix:abc.
    assert_eq!(sheet["rows"][0][5], "00123");
    Ok(())
}

#[test]
fn missing_formula_caches_never_become_invented_numeric_results() -> TestResult {
    let rows = r#"<row r="1">
<c r="A1"><f>1/0</f></c><c r="B1" s="1"><f>1/0</f><v/></c>
<c r="C1" t="n"><f>1/0</f></c><c r="D1" t="b"><f>1/0</f></c>
<c r="E1" t="str"><f>1/0</f></c><c r="F1" t="e"><f>1/0</f></c>
<c r="G1" t="s"><f>1/0</f></c><c r="H1" t="inlineStr"><f>1/0</f></c>
<c r="I1"><f>1/0</f><v>0</v></c>
</row>"#;
    let sheet = projected(&book(rows, false, &standard_styles(), false)?)?;
    for col in [0, 1, 2, 4, 5, 6, 7] {
        assert_eq!(sheet["rows"][0][col], "=1/0");
        assert_eq!(sheet["display"][format!("0:{col}")], "");
        assert!(sheet["display_provenance"][format!("0:{col}")].is_string());
    }
    assert_eq!(sheet["rows"][0][3], "=1/0");
    assert_eq!(sheet["display"]["0:3"], "FALSE");
    assert_eq!(
        sheet["display_provenance"]["0:3"],
        "missing-bool-formula-cache:false-default"
    );
    assert_eq!(sheet["rows"][0][8], 0);
    assert_eq!(sheet["display"]["0:8"], "0");
    assert!(sheet["display_provenance"].get("0:8").is_none());
    assert!(warning(&sheet, "formula-cache-missing"));
    Ok(())
}

#[test]
fn invalid_fill_empty_and_precision_cases_use_exact_reference_text() -> TestResult {
    let specs = [
        ("A1", 15, "-1", ""),
        ("B1", 15, "0", ""),
        ("C1", 12, "1.25", "1.25"),
        ("D1", 16, "1", "1x"),
        ("E1", 17, "1", "1"),
        ("F1", 18, "1", "1"),
        ("G1", 20, "0.125", "0.125000000000"),
        ("H1", 0, "2.675", "2.68"),
        ("I1", 0, "1.005", "1.00"),
        ("J1", 21, "1.25", "1.25"),
    ];
    let cells = specs
        .iter()
        .map(|(address, style, value, _)| cell(address, Some(*style), value))
        .collect::<String>();
    let sheet = projected(&book(
        &format!("<row r=\"1\">{cells}</row>"),
        false,
        &standard_styles(),
        false,
    )?)?;
    for (col, (_, _, _, expected)) in specs.iter().enumerate() {
        assert_eq!(sheet["display"][format!("0:{col}")], *expected);
    }
    assert!(warning(&sheet, "invalid-format-code"));
    Ok(())
}

#[test]
fn first_content_type_styles_and_same_source_image_drive_public_embedding() -> TestResult {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let bytes = book(
        "<row r=\"1\"><c r=\"A1\" s=\"0\"><v>1</v></c></row>",
        false,
        &standard_styles(),
        true,
    )?;
    let sheet = projected(&bytes)?;
    assert_eq!(sheet["display"]["0:0"], "1.00");
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("source.xlsx");
    std::fs::write(&path, &bytes)?;
    let embedded = crate::review::embed_file(&path);
    assert_eq!(embedded["worksheets"][0], sheet);
    assert_eq!(
        STANDARD.decode(embedded["data_b64"].as_str().unwrap())?,
        bytes
    );
    assert_eq!(std::fs::read(&path)?, bytes);
    Ok(())
}

#[test]
fn metadata_lengths_counts_projection_and_work_refusals_are_bounded() -> TestResult {
    let long = format!(
        "<styleSheet xmlns=\"{MAIN}\"><numFmts><numFmt numFmtId=\"164\" formatCode=\"{}\"/></numFmts><cellXfs><xf numFmtId=\"164\"/></cellXfs></styleSheet>",
        "x".repeat(4097)
    );
    let rows = "<row r=\"1\"><c r=\"A1\" s=\"0\"><v>1.25</v></c></row>";
    let bytes = book(rows, false, &long, false)?;
    let sheet = projected(&bytes)?;
    assert_eq!(sheet["display"]["0:0"], "1.25");
    assert!(warning(&sheet, "format-length-limit"));
    let many = format!(
        "<styleSheet xmlns=\"{MAIN}\"><cellXfs>{}</cellXfs></styleSheet>",
        "<xf numFmtId=\"0\"/>".repeat(4097)
    );
    let sheet = projected(&book(rows, false, &many, false)?)?;
    assert!(warning(&sheet, "format-definition-limit"));
    let package = Package::from_reader(Cursor::new(book(rows, false, &standard_styles(), false)?))?;
    let wb = package.part("xl/workbook.xml")?;
    let source = package.part(&package.sheets["Fixture"])?;
    let raw = package.rows("Fixture")?;
    let mut context = Workbook::new(&package, &wb);
    context.projection_left = 0;
    let projected = context.sheet(&source, &raw);
    assert!(projected.display.is_empty());
    assert!(
        projected
            .warnings
            .iter()
            .any(|item| item["code"] == "format-output-limit")
    );
    let mut context = Workbook::new(&package, &wb);
    context.budget = Budget::with_limits(0, PROJECTION);
    assert!(context.sheet(&source, &raw).display.is_empty());
    assert_eq!(context.degraded, Some("format-work-limit"));
    Ok(())
}

#[test]
fn compiled_cache_limit_is_exercised_by_actual_distinct_styles() -> TestResult {
    let mut styles = format!("<styleSheet xmlns=\"{MAIN}\"><numFmts>");
    let mut xfs = String::new();
    let mut rows = String::new();
    for index in 0..270 {
        let id = 61 + index;
        styles.push_str(&format!("<numFmt numFmtId=\"{id}\" formatCode=\"0.00\"/>"));
        xfs.push_str(&format!("<xf numFmtId=\"{id}\"/>"));
        rows.push_str(&format!(
            "<row r=\"{}\"><c r=\"A{}\" s=\"{index}\"><v>1</v></c></row>",
            index + 1,
            index + 1
        ));
    }
    styles.push_str(&format!("</numFmts><cellXfs>{xfs}</cellXfs></styleSheet>"));
    let package = Package::from_reader(Cursor::new(book_range(
        &rows, false, &styles, false, "A1:A270",
    )?))?;
    let wb = package.part("xl/workbook.xml")?;
    let source = package.part(&package.sheets["Fixture"])?;
    let raw = package.rows("Fixture")?;
    let mut context = Workbook::new(&package, &wb);
    let result = context.sheet(&source, &raw);
    assert_eq!(result.display.len(), 270);
    assert!(result.warnings.is_empty());
    assert!(context.cache.len() <= CACHE);
    assert_eq!(result.display["269:0"], "1.00");
    Ok(())
}

#[test]
fn sparse_far_columns_are_admitted_before_dense_rows_expand() -> TestResult {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let rows = (1..=13)
        .map(|row| format!("<row r=\"{row}\"><c r=\"XFD{row}\"><v>1</v></c></row>"))
        .collect::<String>();
    let bytes = book_range(&rows, false, &standard_styles(), false, "A1:XFD13")?;
    let error = crate::workbook_preview::read_bytes(&bytes).unwrap_err();
    assert!(
        error.to_string().contains("worksheet dense slot bound"),
        "{error}"
    );
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("sparse.xlsx");
    std::fs::write(&path, &bytes)?;
    let embedded = crate::review::embed_file(&path);
    assert!(
        embedded["render_error"]
            .as_str()
            .unwrap()
            .contains("worksheet dense slot bound")
    );
    assert_eq!(
        STANDARD.decode(embedded["data_b64"].as_str().unwrap())?,
        bytes
    );
    assert_eq!(std::fs::read(&path)?, bytes);
    Ok(())
}

#[test]
fn duplicate_coordinate_preview_refuses_ambiguous_cache_provenance() -> TestResult {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let rows = r#"<row r="1"><c r="A1"><f>1/0</f></c><c r="A1"><v>1</v></c></row>"#;
    let bytes = book_range(rows, false, &standard_styles(), false, "A1")?;
    let error = crate::workbook_preview::read_bytes(&bytes).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("duplicate worksheet cell coordinate")
    );
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("ambiguous.xlsx");
    std::fs::write(&path, &bytes)?;
    let embedded = crate::review::embed_file(&path);
    assert!(embedded["worksheets"].is_null());
    assert!(
        embedded["render_error"]
            .as_str()
            .unwrap()
            .contains("duplicate worksheet cell coordinate")
    );
    assert_eq!(
        STANDARD.decode(embedded["data_b64"].as_str().unwrap())?,
        bytes
    );
    assert_eq!(std::fs::read(&path)?, bytes);
    Ok(())
}

#[test]
#[ignore = "explicit disposable fixture materialization for the visible Wasm review"]
fn materialize_excel_preview_browser_fixture() -> TestResult {
    let root = std::path::PathBuf::from(std::env::var("INFRA_WORKBOOK_BROWSER_FIXTURE")?);
    // The caller selects a new disposable directory; never reuse an existing workspace.
    std::fs::create_dir(&root)?;
    let labeled = |number: usize, label: &str, value: &str| {
        format!(
            "<row r=\"{number}\"><c r=\"A{number}\" t=\"inlineStr\"><is><t>{}</t></is></c>{value}</row>",
            attr(label)
        )
    };
    let mut formats = String::new();
    for (number, label, value) in [
        (1, "Percent", cell("B1", Some(1), "0.125")),
        (2, "Currency", cell("B2", Some(8), "1234.5")),
        (3, "Explicit style zero", cell("B3", Some(0), "1")),
        (4, "Absent style", cell("B4", None, "1")),
        (5, "1900 phantom day", cell("B5", Some(2), "60")),
        (6, "Fill literal once", cell("B6", Some(16), "12")),
        (
            7,
            "Raw text wins formatted text",
            "<c r=\"B7\" t=\"str\" s=\"14\"><v>abc</v></c>".into(),
        ),
        (8, "Invalid format fallback", cell("B8", Some(12), "1.25")),
    ] {
        formats.push_str(&labeled(number, label, &value));
    }
    let mut caches = String::new();
    for (number, label, value) in [
        (
            1,
            "Supplied stale cache",
            r#"<c r="B1" s="1"><f>1/8</f><v>0.25</v></c>"#,
        ),
        (2, "Missing numeric cache", r#"<c r="B2"><f>1/0</f></c>"#),
        (
            3,
            "Missing boolean cache",
            r#"<c r="B3" t="b"><f>1/0</f></c>"#,
        ),
        (
            4,
            "Supplied numeric zero",
            r#"<c r="B4" s="1"><f>1/0</f><v>0</v></c>"#,
        ),
        (
            5,
            "Supplied text cache",
            r#"<c r="B5" t="str" s="0"><f>&quot;00123&quot;</f><v>00123</v></c>"#,
        ),
    ] {
        caches.push_str(&labeled(number, label, value));
    }
    let mut dates = String::new();
    for (number, label, value) in [
        (1, "1904 zero", "0"),
        (2, "1904 half day", "0.5"),
        (3, "1904 serial sixty", "60"),
    ] {
        dates.push_str(&labeled(
            number,
            label,
            &cell(&format!("B{number}"), Some(2), value),
        ));
    }
    let cases = [
        (
            "01-formats",
            "formats.xlsx",
            formats,
            false,
            vec![
                "12.50%",
                "$1,234.50 ",
                "1.00",
                "1",
                "1900-02-29 00:00",
                "12x",
                "abc",
                "1.25",
            ],
        ),
        (
            "02-caches",
            "caches.xlsx",
            caches,
            false,
            vec!["25.00%", "", "FALSE", "0.00%", "00123"],
        ),
        (
            "03-date1904",
            "date1904.xlsx",
            dates,
            true,
            vec!["1904-01-01 00:00", "1904-01-01 12:00", "1904-03-01 00:00"],
        ),
    ];
    let mut manifest = Vec::new();
    for (index, (run, filename, rows, epoch, expected)) in cases.into_iter().enumerate() {
        let bytes = book_range(
            &rows,
            epoch,
            &standard_styles(),
            false,
            &format!("A1:B{}", expected.len()),
        )?;
        let sheet = projected(&bytes)?;
        for (row, value) in expected.iter().enumerate() {
            let key = format!("{row}:1");
            let actual = sheet["display"].get(&key).unwrap_or(&sheet["rows"][row][1]);
            assert_eq!(actual, *value, "{run} B{}", row + 1);
        }
        let directory = root.join(run);
        std::fs::create_dir(&directory)?;
        std::fs::create_dir(directory.join("outputs"))?;
        std::fs::write(directory.join("outputs").join(filename), &bytes)?;
        std::fs::write(
            directory.join("eval_metadata.json"),
            serde_json::to_vec_pretty(&json!({
                "eval_id":index + 1,
                "prompt":format!("Native Excel preview fixture: {run}. Inspect the labeled value column; Download must preserve the original workbook.")
            }))?,
        )?;
        manifest.push(json!({
            "run_id":run,
            "source":format!("{run}/outputs/{filename}"),
            "source_bytes":bytes.len(),
            "visible_value_column":expected,
            "worksheet":sheet
        }));
    }
    std::fs::write(
        root.join("expected.json"),
        serde_json::to_vec_pretty(&json!({
            "scope":"Selected frozen0.20.3 reference cases; no full SSF or Excel parity claim.",
            "runs":manifest
        }))?,
    )?;
    Ok(())
}
