//! Preview consumes original supplied caches; QA copy semantics must not erase them.
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{fs, io::Write};
type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn original_formula_caches_and_shared_expressions_survive_preview() -> TestResult {
    const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    const PKGREL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("original-cached-values.xlsx");
    let sheet = format!(
        r#"<worksheet xmlns="{MAIN}"><sheetData>
<row><c><f>1/8</f><v>0.25</v></c><c t="str"><f>&quot;supplied&quot;</f><v>supplied</v></c><c t="b"><f>1=1</f><v>1</v></c><c><f t="shared" si="4" ref="D1:D2">SUM(A1,1)</f><v>1.25</v></c></row>
<row><c><v>0.5</v></c><c r="D2"><f t="shared" si="4"/><v>1.5</v></c></row>
</sheetData><mergeCells count="1"><mergeCell ref="F1:G1"/></mergeCells></worksheet>"#
    );
    let parts = [
        ("[Content_Types].xml", r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>"#.to_owned()),
        ("_rels/.rels", format!(r#"<Relationships xmlns="{PKGREL}"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#)),
        ("xl/workbook.xml", format!(r#"<workbook xmlns="{MAIN}" xmlns:r="{REL}"><sheets><sheet name="Original" sheetId="1" r:id="rId1"/></sheets></workbook>"#)),
        ("xl/_rels/workbook.xml.rels", format!(r#"<Relationships xmlns="{PKGREL}"><Relationship Id="rId1" Type="{REL}/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#)),
        ("xl/worksheets/sheet1.xml", sheet),
        ("xl/styles.xml", format!(r#"<styleSheet xmlns="{MAIN}"><cellXfs count="1"><xf numFmtId="0"/></cellXfs></styleSheet>"#)),
    ];
    let mut zip = zip::ZipWriter::new(fs::File::create(&path)?);
    for (name, xml) in parts {
        zip.start_file(
            name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )?;
        zip.write_all(xml.as_bytes())?;
    }
    zip.finish()?;
    let original = fs::read(&path)?;
    let sheets = read(&path)?;
    assert_eq!(sheets.len(), 1);
    let sheet = &sheets[0];
    assert_eq!(sheet["rows"][0][0], 0.25);
    assert_eq!(sheet["rows"][0][1], "supplied");
    assert_eq!(sheet["rows"][0][2], "TRUE");
    assert_eq!(sheet["rows"][0][3], 1.25);
    assert_eq!(sheet["rows"][1][3], 1.5);
    // Deliberately stale supplied cache proves preview does not recalculate.
    assert_eq!(sheet["formulas"]["0:0"], "1/8");
    assert_eq!(sheet["formulas"]["1:3"], "SUM(A2,1)");
    assert_eq!(
        sheet["merges"][0],
        json!({"row":0,"column":5,"rows":1,"columns":2})
    );
    assert_eq!(
        fs::read(&path)?,
        original,
        "preview changed original ZIP bytes"
    );
    let embedded = crate::review::embed_file(&path);
    assert_eq!(embedded["type"], "xlsx");
    assert_eq!(embedded["worksheets"][0]["rows"], sheet["rows"]);
    assert_eq!(
        STANDARD.decode(embedded["data_b64"].as_str().expect("download bytes"))?,
        original
    );
    assert_eq!(
        fs::read(&path)?,
        original,
        "embedding changed original ZIP bytes"
    );
    Ok(())
}
