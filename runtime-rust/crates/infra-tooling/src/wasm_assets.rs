//! Portable UI transport for the frozen Rust/Wasm artifacts. The generated
//! wasm-bindgen ABI glue is preserved byte-for-byte; all application behavior
//! remains in the compiled Rust module.
use infra_core::{Result, RuntimeError, bank::publication::PublicationFiles};
use std::collections::BTreeMap;
pub const ARTIFACT_LIMIT: usize = 32 * 1024 * 1024;
pub const HTML_LIMIT: usize = 128 * 1024 * 1024;
fn base64(bytes: &[u8]) -> String {
    const DIGITS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0] as usize;
        let b = chunk.get(1).copied().unwrap_or(0) as usize;
        let c = chunk.get(2).copied().unwrap_or(0) as usize;
        encoded.push(DIGITS[a >> 2] as char);
        encoded.push(DIGITS[((a & 3) << 4) | (b >> 4)] as char);
        encoded.push(if chunk.len() > 1 {
            DIGITS[((b & 15) << 2) | (c >> 6)] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            DIGITS[c & 63] as char
        } else {
            '='
        });
    }
    encoded
}
pub fn standalone_module_url(javascript: &[u8], wasm: &[u8]) -> Result<String> {
    if javascript.is_empty()
        || javascript.len() > ARTIFACT_LIMIT
        || wasm.len() > ARTIFACT_LIMIT
        || !wasm.starts_with(b"\0asm\x01\0\0\0")
    {
        return Err(RuntimeError::unavailable(
            "Build the UI artifacts before exporting this page",
        ));
    }
    std::str::from_utf8(javascript)
        .map_err(|_| RuntimeError::invalid("Compiled UI module is not UTF-8"))?;
    let javascript = base64(javascript);
    let wasm = base64(wasm);
    let loader = format!(
        "try{{const{{default:init}}=await import('data:text/javascript;base64,{javascript}');await init({{module_or_path:Uint8Array.from(atob('{wasm}'),c=>c.charCodeAt(0))}});}}catch{{const message=document.createElement('div');message.setAttribute('role','alert');message.textContent='Unable to load this page. Please try again.';document.body.prepend(message);}}"
    );
    Ok(format!(
        "data:text/javascript;base64,{}",
        base64(loader.as_bytes())
    ))
}
pub fn standalone_html(html: &[u8], javascript: &[u8], wasm: &[u8]) -> Result<Vec<u8>> {
    if html.len() > HTML_LIMIT {
        return Err(RuntimeError::invalid(
            "Exported page exceeds its byte limit",
        ));
    }
    let html = std::str::from_utf8(html)
        .map_err(|_| RuntimeError::invalid("Exported page template is not UTF-8"))?;
    let variants = [
        "<script type=\"module\" src=\"/boot.js\"></script>",
        "<script type=\"module\" src=\"boot.js\"></script>",
        "<script type=\"module\" src=\"./boot.js\"></script>",
    ];
    let matches = variants
        .iter()
        .map(|marker| html.matches(marker).count())
        .sum::<usize>();
    if matches != 1 {
        return Err(RuntimeError::invalid(
            "Exported page must contain exactly one UI loader",
        ));
    }
    let marker = variants
        .iter()
        .find(|marker| html.contains(**marker))
        .ok_or_else(|| RuntimeError::invalid("UI loader unavailable"))?;
    let url = standalone_module_url(javascript, wasm)?;
    let output = html
        .replacen(
            marker,
            &format!("<script type=\"module\" src=\"{url}\"></script>"),
            1,
        )
        .into_bytes();
    if output.len() > HTML_LIMIT {
        return Err(RuntimeError::invalid(
            "Exported page exceeds its byte limit",
        ));
    }
    Ok(output)
}

pub const JAVASCRIPT_PATH: &str = "runtime-rust/target/wasm-ui/infra_ui.js";
pub const WASM_PATH: &str = "runtime-rust/target/wasm-ui/infra_ui_bg.wasm";
pub const MANIFEST_PATH: &str = "runtime-rust/target/wasm-ui/abi-manifest.json";
type Image = (u64, u64, u32, u64, u64, i128, i128);
#[derive(Clone, Debug)]
pub struct VerifiedAssets {
    pub javascript: Vec<u8>,
    pub wasm: Vec<u8>,
    /// Digest of the UI sources the pair was compiled from, when the build
    /// recorded it (see `ui_build::source_sha256`).
    pub source_sha256: Option<String>,
    root: (u64, u64, u32),
    directories: BTreeMap<String, (u64, u64, u32)>,
    images: BTreeMap<String, Image>,
}
impl VerifiedAssets {
    pub fn recheck(&self, files: &mut PublicationFiles) -> Result<()> {
        if files.root_identity() != self.root {
            return Err(RuntimeError::conflict("UI artifact root custody changed"));
        }
        files.check_directory_identities(&self.directories)?;
        for (path, image) in &self.images {
            if files.image_identity(path)?.as_ref() != Some(image) {
                return Err(RuntimeError::conflict("UI artifact snapshot changed"));
            }
        }
        files.check()
    }
}
pub fn read_verified(files: &mut PublicationFiles) -> Result<VerifiedAssets> {
    read_verified_paths(files, JAVASCRIPT_PATH, WASM_PATH, MANIFEST_PATH)
}
pub fn read_verified_paths(
    files: &mut PublicationFiles,
    javascript_path: &str,
    wasm_path: &str,
    manifest_path: &str,
) -> Result<VerifiedAssets> {
    fn read(
        files: &mut PublicationFiles,
        path: &str,
        limit: usize,
        images: &mut BTreeMap<String, Image>,
    ) -> Result<Vec<u8>> {
        let before = files.image_identity(path)?.ok_or_else(|| {
            RuntimeError::unavailable("Build the UI artifacts before opening this page")
        })?;
        let bytes = files
            .read(path, limit)?
            .ok_or_else(|| RuntimeError::unavailable("UI artifact unavailable"))?;
        if files.image_identity(path)?.as_ref() != Some(&before) {
            return Err(RuntimeError::conflict("UI artifact changed during read"));
        }
        images.insert(path.into(), before);
        Ok(bytes)
    }
    let mut images = BTreeMap::new();
    let raw = read(files, manifest_path, 65536, &mut images)?;
    let manifest = infra_core::strict_json::parse(&raw, 65536, 8, 16)?;
    let object = manifest
        .as_object()
        .filter(|object| {
            ["schema_version", "js_sha256", "wasm_sha256"]
                .iter()
                .all(|key| object.contains_key(*key))
                && object.keys().all(|key| {
                    [
                        "schema_version",
                        "js_sha256",
                        "wasm_sha256",
                        "source_sha256",
                    ]
                    .contains(&key.as_str())
                })
        })
        .ok_or_else(|| RuntimeError::invalid("UI artifact manifest fields differ"))?;
    let digest = |name| {
        object
            .get(name)
            .and_then(serde_json::Value::as_str)
            .filter(|value| {
                value.len() == 64
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
            .map(String::from)
            .ok_or_else(|| RuntimeError::invalid("UI artifact manifest digest invalid"))
    };
    if object
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
    {
        return Err(RuntimeError::invalid(
            "UI artifact manifest version differs",
        ));
    }
    let javascript = read(files, javascript_path, ARTIFACT_LIMIT, &mut images)?;
    let wasm = read(files, wasm_path, ARTIFACT_LIMIT, &mut images)?;
    if infra_core::bank::publication::sha256(&javascript) != digest("js_sha256")?
        || infra_core::bank::publication::sha256(&wasm) != digest("wasm_sha256")?
    {
        return Err(RuntimeError::conflict(
            "UI artifact ABI pair differs from its manifest",
        ));
    }
    if javascript.is_empty()
        || std::str::from_utf8(&javascript).is_err()
        || !wasm.starts_with(b"\0asm\x01\0\0\0")
    {
        return Err(RuntimeError::invalid("UI artifact ABI pair invalid"));
    }
    let source_sha256 = if object.contains_key("source_sha256") {
        Some(digest("source_sha256")?)
    } else {
        None
    };
    let result = VerifiedAssets {
        javascript,
        wasm,
        source_sha256,
        root: files.root_identity(),
        directories: files.directory_identities()?,
        images,
    };
    result.recheck(files)?;
    Ok(result)
}
