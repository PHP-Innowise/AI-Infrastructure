//! Native discovery, embedding and loopback feedback server for the Rust/Wasm viewer.
mod feedback;
use crate::{Result, RuntimeError, common};
use base64::{Engine, engine::general_purpose::STANDARD};
use feedback::FeedbackStore;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
const TEXT: [&str; 26] = [
    "txt", "md", "json", "csv", "py", "js", "ts", "tsx", "jsx", "yaml", "yml", "xml", "html",
    "css", "sh", "rb", "go", "rs", "java", "c", "cpp", "h", "hpp", "sql", "r", "toml",
];
pub fn get_mime_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "html" => "text/html",
        "txt" | "md" => "text/plain",
        "json" => "application/json",
        "csv" => "text/csv",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}
pub fn embed_file(path: &Path) -> Value {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    let mime = get_mime_type(path);
    let raw = match std::fs::read(path) {
        Ok(raw) => raw,
        Err(_) => {
            return json!({"name":name,"type":if TEXT.contains(&ext.as_str()){"text"}else{"error"},"content":"(Error reading file)"});
        }
    };
    if TEXT.contains(&ext.as_str()) {
        return json!({"name":name,"type":"text","content":common::normalize(&String::from_utf8_lossy(&raw))});
    }
    let b64 = STANDARD.encode(&raw);
    let data_uri = |encoded: &str| format!("data:{mime};base64,{encoded}");
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" => {
            json!({"name":name,"type":"image","mime":mime,"data_uri":data_uri(&b64)})
        }
        "pdf" => json!({"name":name,"type":"pdf","data_uri":data_uri(&b64)}),
        "xlsx" => {
            let sheets = crate::workbook_preview::read_bytes(&raw);
            match sheets {
                Ok(sheets) => json!({"name":name,"type":"xlsx","data_b64":b64,"worksheets":sheets}),
                Err(e) => {
                    json!({"name":name,"type":"xlsx","data_b64":b64,"render_error":e.to_string()})
                }
            }
        }
        _ => json!({"name":name,"type":"binary","mime":mime,"data_uri":data_uri(&b64)}),
    }
}
pub fn build_run(root: &Path, directory: &Path) -> Result<Value> {
    let mut prompt = String::new();
    let mut eval_id = Value::Null;
    for file in [
        directory.join("eval_metadata.json"),
        directory
            .parent()
            .unwrap_or(directory)
            .join("eval_metadata.json"),
    ] {
        if let Ok(meta) = common::json_file(&file) {
            prompt = meta["prompt"].as_str().unwrap_or("").into();
            eval_id = meta.get("eval_id").cloned().unwrap_or(Value::Null);
        }
        if !prompt.is_empty() {
            break;
        }
    }
    if prompt.is_empty() {
        let re = regex::Regex::new(r"(?s)## Eval Prompt\n\n(.*?)(?:\n##|$)").unwrap();
        for file in [
            directory.join("transcript.md"),
            directory.join("outputs/transcript.md"),
        ] {
            if let Ok(text) = common::read(&file)
                && let Some(c) = re.captures(&text)
            {
                prompt = c[1].trim().into();
            }
            if !prompt.is_empty() {
                break;
            }
        }
    }
    if prompt.is_empty() {
        prompt = "(No prompt found)".into();
    }
    let id = directory
        .strip_prefix(root)
        .map_err(|_| RuntimeError::invalid("Run is outside workspace"))?
        .to_string_lossy()
        .replace(['/', '\\'], "-");
    let mut outputs = vec![];
    for file in common::sorted(&directory.join("outputs"))? {
        if file.is_file()
            && !["transcript.md", "user_notes.md", "metrics.json"].contains(
                &file
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .as_ref(),
            )
        {
            outputs.push(embed_file(&file));
        }
    }
    let mut grading = Value::Null;
    for file in [
        directory.join("grading.json"),
        directory.parent().unwrap_or(directory).join("grading.json"),
    ] {
        if let Ok(value) = common::json_file(&file) {
            grading = value;
        }
        if !grading.is_null() && grading != false && grading != json!({}) {
            break;
        }
    }
    Ok(json!({"id":id,"prompt":prompt,"eval_id":eval_id,"outputs":outputs,"grading":grading}))
}
fn recurse(
    root: &Path,
    current: &Path,
    runs: &mut Vec<Value>,
    seen: &mut std::collections::BTreeSet<PathBuf>,
) -> Result<()> {
    if !current.is_dir() {
        return Ok(());
    }
    let canonical = current.canonicalize()?;
    if !canonical.starts_with(root.canonicalize()?) || !seen.insert(canonical) {
        return Ok(());
    }
    if current.join("outputs").is_dir() {
        runs.push(build_run(root, current)?);
        return Ok(());
    }
    for child in common::sorted(current)? {
        if child.is_dir()
            && !["node_modules", ".git", "__pycache__", "skill", "inputs"].contains(
                &child
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .as_ref(),
            )
        {
            recurse(root, &child, runs, seen)?;
        }
    }
    Ok(())
}
pub fn find_runs(workspace: &Path) -> Result<Vec<Value>> {
    let mut runs = vec![];
    recurse(workspace, workspace, &mut runs, &mut Default::default())?;
    runs.sort_by(|a, b| {
        let a_id = a["eval_id"].as_f64().unwrap_or(f64::INFINITY);
        let b_id = b["eval_id"].as_f64().unwrap_or(f64::INFINITY);
        a_id.total_cmp(&b_id)
            .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
    });
    Ok(runs)
}
pub fn load_previous_iteration(workspace: &Path) -> Result<Value> {
    if !workspace.is_dir() {
        return Ok(json!({}));
    }
    let mut feedback_store = FeedbackStore::open(workspace)?;
    let raw = feedback_store.read()?;
    let mut feedback = BTreeMap::new();
    if let Ok(value) = serde_json::from_slice::<Value>(&raw)
        && let Some(reviews) = value["reviews"].as_array()
    {
        for r in reviews {
            if let (Some(id), Some(text)) = (r["run_id"].as_str(), r["feedback"].as_str())
                && !text.trim().is_empty()
            {
                feedback.insert(id.to_owned(), text.to_owned());
            }
        }
    }
    let mut previous = serde_json::Map::new();
    for run in find_runs(workspace)? {
        let id = run["id"].as_str().unwrap();
        previous.insert(
            id.into(),
            json!({"feedback":feedback.remove(id).unwrap_or_default(),"outputs":run["outputs"]}),
        );
    }
    for (id, text) in feedback {
        previous.insert(id, json!({"feedback":text,"outputs":[]}));
    }
    Ok(Value::Object(previous))
}
pub struct Assets {
    image: infra_tooling::wasm_assets::VerifiedAssets,
}
impl Assets {
    pub fn load(v: &Value) -> Result<Self> {
        let dir = common::text(v, "ui_assets")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("INFRA_UI_ASSETS").map(PathBuf::from))
            .or_else(|| {
                common::text(v, "source_root")
                    .map(|r| Path::new(r).join("runtime-rust/target/wasm-ui"))
            })
            .ok_or_else(|| {
                RuntimeError::invalid(
                    "ui_assets must identify the generated Rust/Wasm viewer image",
                )
            })?;
        let mut files = infra_core::bank::publication::PublicationFiles::new(&dir)?;
        let image = infra_tooling::wasm_assets::read_verified_paths(
            &mut files,
            "infra_ui.js",
            "infra_ui_bg.wasm",
            "abi-manifest.json",
        )?;
        image.recheck(&mut files)?;
        Ok(Self { image })
    }
}

#[derive(serde::Serialize)]
struct ViewerData<'a> {
    skill_name: &'a str,
    runs: &'a [Value],
    previous_feedback: &'a serde_json::Map<String, Value>,
    previous_outputs: &'a serde_json::Map<String, Value>,
    standalone: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    benchmark: Option<Value>,
}

fn viewer_data_json(
    runs: &[Value],
    name: &str,
    previous: &Value,
    benchmark: Option<&Value>,
    standalone: bool,
) -> Result<String> {
    let (mut old_feedback, mut old_outputs) = (serde_json::Map::new(), serde_json::Map::new());
    if let Some(map) = previous.as_object() {
        for (id, p) in map {
            if p["feedback"].as_str().is_some_and(|s| !s.is_empty()) {
                old_feedback.insert(id.clone(), p["feedback"].clone());
            }
            if p["outputs"].as_array().is_some_and(|a| !a.is_empty()) {
                old_outputs.insert(id.clone(), p["outputs"].clone());
            }
        }
    }
    let benchmark = benchmark.map(|b| {
        let mut value = b.clone();
        value["configuration_order"] = json!(
            b["run_summary"]
                .as_object()
                .map(|m| m
                    .keys()
                    .filter(|k| *k != "delta")
                    .cloned()
                    .collect::<Vec<_>>())
                .unwrap_or_default()
        );
        value
    });
    Ok(serde_json::to_string(&ViewerData {
        skill_name: name,
        runs,
        previous_feedback: &old_feedback,
        previous_outputs: &old_outputs,
        standalone,
        benchmark,
    })?
    .replace('&', "\\u0026")
    .replace('<', "\\u003c")
    .replace('>', "\\u003e")
    .replace('\u{2028}', "\\u2028")
    .replace('\u{2029}', "\\u2029"))
}

pub fn generate_html(
    runs: &[Value],
    name: &str,
    previous: &Value,
    benchmark: Option<&Value>,
    assets: &Assets,
    standalone: bool,
) -> Result<String> {
    let safe = viewer_data_json(runs, name, previous, benchmark, standalone)?;
    let html = include_str!("../resources/viewer.html")
        .replace("__EVAL_DATA__", &safe)
        .replace("__BOOT_URL__", "/boot.js");
    if standalone {
        String::from_utf8(infra_tooling::wasm_assets::standalone_html(
            html.as_bytes(),
            &assets.image.javascript,
            &assets.image.wasm,
        )?)
        .map_err(|_| RuntimeError::invalid("Native viewer encoding failed"))
    } else if html.len() > infra_tooling::wasm_assets::HTML_LIMIT {
        Err(RuntimeError::invalid("Viewer page exceeds byte limit"))
    } else {
        Ok(html)
    }
}
pub fn eval_set_execute(v: &Value) -> Result<Value> {
    let skill = crate::utils::parse_skill_md(Path::new(common::required(v, "skill_path")?))?;
    let items = common::json_file(Path::new(common::required(v, "eval_set")?))?;
    if !items.as_array().is_some_and(|items| {
        items
            .iter()
            .all(|i| i["query"].is_string() && i["should_trigger"].is_boolean())
    }) {
        return Err(RuntimeError::invalid(
            "Eval set must contain query strings and boolean should_trigger values",
        ));
    }
    let assets = Assets::load(v)?;
    let data = json!({"mode":"eval-set","items":items,"skill_name":skill.name,"description":skill.description,"standalone":true});
    let safe = serde_json::to_string(&data)?
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    let html = include_str!("../resources/eval-set.html")
        .replace("__SKILL_NAME_PLACEHOLDER__", &common::html(&skill.name))
        .replace(
            "__SKILL_DESCRIPTION_PLACEHOLDER__",
            &common::html(&skill.description),
        )
        .replace("__EVAL_DATA__", &safe)
        .replace("__BOOT_URL__", "/boot.js");
    let html = infra_tooling::wasm_assets::standalone_html(
        html.as_bytes(),
        &assets.image.javascript,
        &assets.image.wasm,
    )?;
    let output = common::required(v, "output")?;
    std::fs::write(output, html)?;
    Ok(common::cli(
        0,
        format!("Eval set review written to {output}\n"),
        "",
        json!({"output":output}),
    ))
}

fn response(
    status: u16,
    mime: &str,
    body: Vec<u8>,
) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    tiny_http::Response::from_data(body)
        .with_status_code(status)
        .with_header(
            tiny_http::Header::from_bytes("Content-Type".as_bytes(), mime.as_bytes()).unwrap(),
        )
        .with_header(
            tiny_http::Header::from_bytes(
                "X-Content-Type-Options".as_bytes(),
                "nosniff".as_bytes(),
            )
            .unwrap(),
        )
}
pub fn kill_port(port: u16, log: &mut String) -> Result<()> {
    if port == 0 {
        return Ok(());
    }
    let output = match common::invoke(
        vec!["lsof".into(), "-ti".into(), format!(":{port}")],
        &std::env::current_dir()?,
        common::env(),
        vec![],
        Duration::from_secs(5),
        |_| None,
    ) {
        Ok(x) => x,
        Err(_) => {
            log.push_str("Note: lsof not found, cannot check if port is in use\n");
            return Ok(());
        }
    };
    for pid in output
        .stdout
        .lines()
        .filter_map(|s| s.trim().parse::<i32>().ok())
        .filter(|pid| *pid > 1 && *pid != std::process::id() as i32)
    {
        #[cfg(unix)]
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        #[cfg(windows)]
        {
            let _ = common::invoke(
                vec!["taskkill".into(), "/PID".into(), pid.to_string()],
                &std::env::current_dir()?,
                common::env(),
                vec![],
                Duration::from_secs(5),
                |_| None,
            );
        }
    }
    Ok(())
}
pub fn execute(v: &Value) -> Result<Value> {
    let workspace = common::absolute(common::positional(v, "workspace", 0)?)?;
    if !workspace.is_dir() {
        return Ok(common::cli(
            1,
            "",
            format!("Error: {} is not a directory\n", workspace.display()),
            Value::Null,
        ));
    }
    let runs = find_runs(&workspace)?;
    if runs.is_empty() {
        return Ok(common::cli(
            1,
            "",
            format!("No runs found in {}\n", workspace.display()),
            Value::Null,
        ));
    }
    let name = common::text(v, "skill_name")
        .map(str::to_owned)
        .unwrap_or_else(|| {
            workspace
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .replace("-workspace", "")
        });
    let previous = common::text(v, "previous_workspace")
        .map(|p| load_previous_iteration(Path::new(p)))
        .transpose()?
        .unwrap_or(json!({}));
    let benchmark = common::text(v, "benchmark").map(PathBuf::from);
    let benchmark_data = benchmark.as_deref().and_then(|p| common::json_file(p).ok());
    let assets = Assets::load(v)?;
    if let Some(path) = common::text(v, "static") {
        let path = Path::new(path);
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(
            path,
            generate_html(
                &runs,
                &name,
                &previous,
                benchmark_data.as_ref(),
                &assets,
                true,
            )?,
        )?;
        return Ok(common::cli(
            0,
            format!("\n  Static viewer written to: {}\n\n", path.display()),
            "",
            json!({"path":path,"runs":runs.len()}),
        ));
    }
    let n = common::number(v, "port", 3117.0)?;
    if n < 0.0 || n > 65535.0 || n.fract() != 0.0 {
        return Err(RuntimeError::invalid("port must be in 0..65535"));
    }
    let mut log = String::new();
    kill_port(n as u16, &mut log)?;
    let server = tiny_http::Server::http(("127.0.0.1", n as u16))
        .or_else(|_| tiny_http::Server::http(("127.0.0.1", 0)))
        .map_err(|e| RuntimeError::unavailable(e.to_string()))?;
    let url = format!("http://{}", server.server_addr());
    let feedback = workspace.join("feedback.json");
    let mut feedback_store = FeedbackStore::open(&workspace)?;
    let announcement = format!(
        "\n  Eval Viewer\n  ─────────────────────────────────\n  URL:       {url}\n  Workspace: {}\n  Feedback:  {}\n\n  Press Ctrl+C to stop.\n\n",
        workspace.display(),
        feedback.display()
    );
    print!("{announcement}");
    use std::io::Write;
    let _ = std::io::stdout().flush();
    if let Err(e) = common::browser(&url) {
        log.push_str(&format!("Browser: {e}\n"));
    }
    while !infra_eval::cancel::interrupted() {
        let Some(mut request) = server
            .recv_timeout(Duration::from_millis(200))
            .map_err(|e| RuntimeError::unavailable(e.to_string()))?
        else {
            continue;
        };
        let path = request.url().to_owned();
        let method = request.method().as_str().to_owned();
        let reply = match (method.as_str(), path.as_str()) {
            ("GET", "/") | ("GET", "/index.html") => {
                let runs = find_runs(&workspace)?;
                let b = benchmark.as_deref().and_then(|p| common::json_file(p).ok());
                response(
                    200,
                    "text/html; charset=utf-8",
                    generate_html(&runs, &name, &previous, b.as_ref(), &assets, false)?
                        .into_bytes(),
                )
            }
            ("GET", "/boot.js") => response(
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../../../../harness/web/boot.js").to_vec(),
            ),
            ("GET", "/assets/infra_ui.js") => response(
                200,
                "text/javascript; charset=utf-8",
                assets.image.javascript.clone(),
            ),
            ("GET", "/assets/infra_ui_bg.wasm") => {
                response(200, "application/wasm", assets.image.wasm.clone())
            }
            ("GET", "/api/feedback") => match feedback_store.read() {
                Ok(bytes) => response(200, "application/json", bytes),
                Err(error) => response(
                    500,
                    "application/json",
                    json!({"error":error.to_string()}).to_string().into_bytes(),
                ),
            },
            ("POST", "/api/feedback") => {
                let mut body = vec![];
                let outcome = (|| -> Result<()> {
                    if !request.headers().iter().any(|h| {
                        h.field.equiv("Content-Type")
                            && feedback::json_content_type(h.value.as_str())
                    }) {
                        return Err(RuntimeError::invalid("Feedback requires application/json"));
                    }
                    request
                        .as_reader()
                        .take(8 * 1024 * 1024 + 1)
                        .read_to_end(&mut body)?;
                    if body.len() > 8 * 1024 * 1024 {
                        return Err(RuntimeError::invalid("Feedback exceeds limit"));
                    }
                    feedback_store.save(&body)
                })();
                match outcome {
                    Ok(()) => response(200, "application/json", b"{\"ok\":true}".to_vec()),
                    Err(e) => response(
                        500,
                        "application/json",
                        json!({"error":e.to_string()}).to_string().into_bytes(),
                    ),
                }
            }
            _ => response(404, "text/plain", b"Not found".to_vec()),
        };
        let _ = request.respond(reply);
    }
    Ok(common::cli(
        0,
        "\nStopped.\n",
        log,
        json!({"url":url,"feedback":feedback}),
    ))
}

#[cfg(test)]
mod embedding_tests {
    use super::*;

    #[test]
    fn uri_consumers_embed_exact_original_bytes_and_mime_types() {
        let directory = tempfile::tempdir().unwrap();
        let bytes = [0, 255, 13, 10, 0, 127, 128];
        for (extension, kind, mime) in [
            ("png", "image", "image/png"),
            ("jpg", "image", "image/jpeg"),
            ("jpeg", "image", "image/jpeg"),
            ("gif", "image", "image/gif"),
            ("svg", "image", "image/svg+xml"),
            ("webp", "image", "image/webp"),
            ("PDF", "pdf", "application/pdf"),
            (
                "docx",
                "binary",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            ),
            (
                "pptx",
                "binary",
                "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            ),
            ("zip", "binary", "application/zip"),
            ("unknown", "binary", "application/octet-stream"),
        ] {
            let name = format!("original.{extension}");
            let path = directory.path().join(&name);
            std::fs::write(&path, bytes).unwrap();
            let embedded = embed_file(&path);
            let uri = embedded["data_uri"].as_str().unwrap();
            let encoded = uri.strip_prefix(&format!("data:{mime};base64,")).unwrap();
            assert_eq!(STANDARD.decode(encoded).unwrap(), bytes);
            let mut expected = json!({"name":name,"type":kind,"data_uri":uri});
            if kind != "pdf" {
                expected["mime"] = json!(mime);
            }
            assert_eq!(embedded, expected);
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
    }

    #[test]
    fn text_and_failed_workbook_embeddings_keep_their_existing_contracts() {
        let directory = tempfile::tempdir().unwrap();
        let text = b"first\r\nsecond\r\xff";
        for extension in TEXT {
            let name = format!("original.{extension}");
            let path = directory.path().join(&name);
            std::fs::write(&path, text).unwrap();
            assert_eq!(
                embed_file(&path),
                json!({"name":name,"type":"text","content":"first\nsecond\n\u{fffd}"})
            );
            assert_eq!(std::fs::read(path).unwrap(), text);
        }
        let path = directory.path().join("original.XLSX");
        let original = b"not a workbook\x00\xff";
        std::fs::write(&path, original).unwrap();
        let embedded = embed_file(&path);
        assert_eq!(embedded["name"], "original.XLSX");
        assert_eq!(embedded["type"], "xlsx");
        assert!(embedded["render_error"].is_string());
        assert!(embedded["worksheets"].is_null());
        assert!(embedded["data_uri"].is_null());
        assert_eq!(
            STANDARD
                .decode(embedded["data_b64"].as_str().unwrap())
                .unwrap(),
            original
        );
        assert_eq!(std::fs::read(path).unwrap(), original);
    }
}

#[cfg(test)]
mod viewer_data_tests {
    use super::*;

    fn legacy_viewer_data_json(
        runs: &[Value],
        name: &str,
        previous: &Value,
        benchmark: Option<&Value>,
        standalone: bool,
    ) -> Result<String> {
        let (mut old_feedback, mut old_outputs) = (serde_json::Map::new(), serde_json::Map::new());
        if let Some(map) = previous.as_object() {
            for (id, p) in map {
                if p["feedback"].as_str().is_some_and(|s| !s.is_empty()) {
                    old_feedback.insert(id.clone(), p["feedback"].clone());
                }
                if p["outputs"].as_array().is_some_and(|a| !a.is_empty()) {
                    old_outputs.insert(id.clone(), p["outputs"].clone());
                }
            }
        }
        let mut data = json!({"skill_name":name,"runs":runs,"previous_feedback":old_feedback,"previous_outputs":old_outputs,"standalone":standalone});
        if let Some(b) = benchmark {
            data["benchmark"] = b.clone();
            data["benchmark"]["configuration_order"] = json!(
                b["run_summary"]
                    .as_object()
                    .map(|m| m
                        .keys()
                        .filter(|k| *k != "delta")
                        .cloned()
                        .collect::<Vec<_>>())
                    .unwrap_or_default()
            );
        }
        Ok(serde_json::to_string(&data)?
            .replace('&', "\\u0026")
            .replace('<', "\\u003c")
            .replace('>', "\\u003e")
            .replace('\u{2028}', "\\u2028")
            .replace('\u{2029}', "\\u2029"))
    }

    #[test]
    fn viewer_data_preserves_field_order_source_order_and_script_escaping() {
        let runs = [json!({"z":"&<>\u{2028}\u{2029}","a":1})];
        let previous = json!({
            "z":{"feedback":"z","outputs":[{"id":"z"}]},
            "a":{"feedback":"a","outputs":[{"id":"a"}]},
            "empty":{"feedback":"","outputs":[]}
        });
        let rendered = viewer_data_json(&runs, "skill", &previous, None, false).unwrap();
        assert_eq!(
            rendered,
            r#"{"skill_name":"skill","runs":[{"z":"\u0026\u003c\u003e\u2028\u2029","a":1}],"previous_feedback":{"z":"z","a":"a"},"previous_outputs":{"z":[{"id":"z"}],"a":[{"id":"a"}]},"standalone":false}"#
        );
        assert_eq!(
            rendered,
            legacy_viewer_data_json(&runs, "skill", &previous, None, false).unwrap()
        );
    }

    #[test]
    fn viewer_benchmark_override_preserves_existing_key_position() {
        let benchmark = json!({
            "first":1,
            "configuration_order":["old"],
            "run_summary":{"z":{},"delta":{},"a":{}},
            "last":true
        });
        let rendered =
            viewer_data_json(&[], "skill", &Value::Null, Some(&benchmark), true).unwrap();
        assert_eq!(
            rendered,
            r#"{"skill_name":"skill","runs":[],"previous_feedback":{},"previous_outputs":{},"standalone":true,"benchmark":{"first":1,"configuration_order":["z","a"],"run_summary":{"z":{},"delta":{},"a":{}},"last":true}}"#
        );
        assert_eq!(benchmark["configuration_order"], json!(["old"]));
    }

    #[test]
    fn viewer_data_matches_previous_serialization_for_optional_inputs() {
        let runs = [json!({"outputs":[{"data_b64":"AAABBB==","text":"&<>\u{2028}\u{2029}"}]})];
        let previous_cases = [
            Value::Null,
            json!(false),
            json!({"z":{"feedback":"&<>","outputs":[{"n":2,"a":1}]},"a":{"feedback":"yes","outputs":["x"]},"skip":{"feedback":4,"outputs":[]}}),
        ];
        let benchmark_cases = [
            None,
            Some(Value::Null),
            Some(json!({})),
            Some(json!({"run_summary":false,"configuration_order":null})),
            Some(
                json!({"run_summary":{"second":1,"delta":2,"first":3},"tail":"&<>\u{2028}\u{2029}"}),
            ),
        ];
        for previous in &previous_cases {
            for benchmark in &benchmark_cases {
                for standalone in [false, true] {
                    assert_eq!(
                        viewer_data_json(
                            &runs,
                            "<&>\u{2028}\u{2029}",
                            previous,
                            benchmark.as_ref(),
                            standalone
                        )
                        .unwrap(),
                        legacy_viewer_data_json(
                            &runs,
                            "<&>\u{2028}\u{2029}",
                            previous,
                            benchmark.as_ref(),
                            standalone
                        )
                        .unwrap()
                    );
                }
            }
        }
        assert_eq!(
            viewer_data_json(&[], "skill", &Value::Null, Some(&Value::Null), false).unwrap(),
            r#"{"skill_name":"skill","runs":[],"previous_feedback":{},"previous_outputs":{},"standalone":false,"benchmark":{"configuration_order":[]}}"#
        );
    }

    #[test]
    fn invalid_scalar_and_array_benchmarks_keep_the_existing_failure() {
        for benchmark in [json!(true), json!(4), json!("text"), json!([])] {
            assert!(
                std::panic::catch_unwind(|| {
                    legacy_viewer_data_json(&[], "skill", &Value::Null, Some(&benchmark), false)
                })
                .is_err()
            );
            assert!(
                std::panic::catch_unwind(|| {
                    viewer_data_json(&[], "skill", &Value::Null, Some(&benchmark), false)
                })
                .is_err()
            );
        }
    }
}
