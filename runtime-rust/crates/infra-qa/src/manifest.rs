use crate::common::*;
use anyhow::{Result, bail};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
fn walk(root: &Path, node: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for e in fs::read_dir(node)? {
        let e = e?;
        let p = e.path();
        let m = fs::symlink_metadata(&p)?;
        if m.file_type().is_symlink() {
            bail!("evidence tree contains a symlink");
        }
        if m.is_dir() {
            walk(root, &p, out)?;
        } else if m.is_file() && !e.file_name().to_string_lossy().starts_with('.') {
            out.push(p);
        }
    }
    let _ = root;
    Ok(())
}
pub fn artifacts(root: &Path, output: &Path) -> Result<Vec<PathBuf>> {
    let mut a = vec![];
    walk(root, root, &mut a)?;
    a.retain(|p| p != output);
    a.sort();
    Ok(a)
}
pub fn render(root: &Path, output: &Path) -> Result<String> {
    let mut s = String::new();
    for p in artifacts(root, output)? {
        s.push_str(&format!(
            "{}  {}\n",
            hash_file(&p)?,
            p.strip_prefix(root)?.to_string_lossy().replace('\\', "/")
        ));
    }
    Ok(s)
}
pub fn parse(path: &Path) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for (i, line) in fs::read_to_string(path)?.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (d, p) = line
            .split_once(char::is_whitespace)
            .ok_or_else(|| anyhow::anyhow!("malformed checksum line {}", i + 1))?;
        let p = p.trim().trim_start_matches('*');
        if d.len() != 64
            || !d.bytes().all(|b| b.is_ascii_hexdigit())
            || p.is_empty()
            || out.insert(p.to_string(), d.to_lowercase()).is_some()
        {
            bail!("invalid or duplicate checksum line {}", i + 1);
        }
    }
    Ok(out)
}
pub fn verify(root: &Path, path: &Path, complete: bool) -> Result<Vec<String>> {
    if !regular(path) {
        bail!("checksum manifest must be a regular file");
    }
    let want = parse(path)?;
    let mut errors = vec![];
    for (name, d) in &want {
        if !complete && name.starts_with("WITHHELD/") {
            continue;
        }
        match confined(root, name) {
            Ok(p) if regular(&p) => {
                if hash_file(&p)? != *d {
                    errors.push(format!("checksum mismatch: {name}"));
                }
            }
            _ => errors.push(format!("missing or unsafe checksum target: {name}")),
        }
    }
    if complete {
        for p in artifacts(root, path)? {
            let n = p.strip_prefix(root)?.to_string_lossy().replace('\\', "/");
            if !want.contains_key(&n) {
                errors.push(format!("unmanifested artifact: {n}"));
            }
        }
    }
    Ok(errors)
}
pub fn run(args: &[String]) -> Result<i32> {
    let (o, p) = parse_args(args, &["--output"], &["--check"])?;
    if p.len() != 1 {
        bail!("manifest ROOT [--output FILE] [--check]");
    }
    let root = crate::spelling::canonicalize(&p[0])?;
    let output = o
        .get("--output")
        .and_then(|v| v.last())
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("SHA256SUMS.txt"));
    if o.contains_key("--check") {
        let errors = verify(&root, &output, true)?;
        if !errors.is_empty() {
            bail!("{}", errors.join("; "));
        }
        println!("Verified {}", output.display());
    } else {
        atomic(&output, render(&root, &output)?.as_bytes())?;
        println!("Wrote {}", output.display());
    }
    Ok(0)
}
