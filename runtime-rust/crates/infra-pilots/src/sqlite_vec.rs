use anyhow::{Context, Result, bail};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
fn expanded(p: &str) -> PathBuf {
    if (p == "~" || p.starts_with("~/"))
        && let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
    {
        return PathBuf::from(home).join(p.trim_start_matches("~/"));
    }
    PathBuf::from(p)
}
pub fn probe(vec: Option<&Path>, lembed: Option<&Path>, model: Option<&Path>) -> Value {
    let mut out = json!({"available":false,"checks":{},"reason":null});
    for (k, p) in [
        ("vec-extension", vec),
        ("lembed-extension", lembed),
        ("model", model),
    ] {
        let Some(p) = p else {
            out["reason"] = format!("{k}-missing").into();
            return out;
        };
        let m = fs::symlink_metadata(p);
        if !m
            .as_ref()
            .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
        {
            out["reason"] = format!("{k}-missing").into();
            return out;
        }
        out["checks"][k] = json!({"path":p,"bytes":m.unwrap().len()});
    }
    let connection = match Connection::open_in_memory() {
        Ok(c) => c,
        Err(_) => {
            out["reason"] = "extension-loading-unavailable".into();
            return out;
        }
    };
    if unsafe { connection.load_extension_enable() }.is_err() {
        out["reason"] = "extension-loading-unavailable".into();
        return out;
    }
    for (k, p, func) in [
        ("vec", vec.unwrap(), "vec_version"),
        ("lembed", lembed.unwrap(), "lembed_version"),
    ] {
        if unsafe { connection.load_extension(p, None::<&str>) }.is_err() {
            out["reason"] = format!("{k}-extension-load-failed").into();
            let _ = connection.load_extension_disable();
            return out;
        }
        let version = connection
            .query_row(&format!("SELECT {func}()"), [], |r| r.get::<_, String>(0))
            .ok();
        out["checks"][format!("{k}-version")] = json!(version);
    }
    let _ = connection.load_extension_disable();
    out["available"] = true.into();
    out
}
pub fn fuse(lexical: &[String], dense: &[String], k: u64) -> Result<Value> {
    if k < 1 {
        bail!("k must be positive");
    }
    let mut scores = BTreeMap::<String, f64>::new();
    let mut ranks = BTreeMap::<String, BTreeMap<String, usize>>::new();
    for (source, rows) in [("lexical", lexical), ("dense", dense)] {
        let mut seen = BTreeSet::new();
        let mut rank = 0;
        for p in rows {
            if p.is_empty() || !seen.insert(p) {
                continue;
            }
            rank += 1;
            *scores.entry(p.clone()).or_default() += 1.0 / (k as f64 + rank as f64);
            ranks
                .entry(p.clone())
                .or_default()
                .insert(source.into(), rank);
        }
    }
    let mut paths = scores.keys().cloned().collect::<Vec<_>>();
    paths.sort_by(|a, b| scores[b].total_cmp(&scores[a]).then_with(|| a.cmp(b)));
    Ok(json!(
        paths
            .iter()
            .map(|p| json!({"path":p,"score":(scores[p]*1e9).round()/1e9,"ranks":ranks[p]}))
            .collect::<Vec<_>>()
    ))
}
pub fn run(command: &str, args: &[String]) -> Result<i32> {
    let mut values = BTreeMap::new();
    let mut json_output = false;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--json" {
            json_output = true;
        } else {
            let k = args[i].clone();
            if ![
                "--vec-extension",
                "--lembed-extension",
                "--model",
                "--lexical",
                "--dense",
                "--k",
            ]
            .contains(&k.as_str())
            {
                bail!("unknown vector option");
            }
            i += 1;
            values.insert(
                k,
                args.get(i).context("missing vector option value")?.clone(),
            );
        }
        i += 1;
    }
    let result = match command {
        "probe" => {
            let paths = ["--vec-extension", "--lembed-extension", "--model"]
                .map(|k| values.get(k).map(|p| expanded(p)));
            probe(
                paths[0].as_deref(),
                paths[1].as_deref(),
                paths[2].as_deref(),
            )
        }
        "fuse" => {
            let read = |k: &str| -> Result<Vec<String>> {
                Ok(serde_json::from_slice(&fs::read(
                    values.get(k).context("missing ranked path input")?,
                )?)?)
            };
            fuse(
                &read("--lexical")?,
                &read("--dense")?,
                values
                    .get("--k")
                    .map(|s| s.parse())
                    .transpose()?
                    .unwrap_or(60),
            )?
        }
        _ => bail!("unknown vector operation"),
    };
    if json_output {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else if command == "probe" {
        println!(
            "available: {}{}",
            result["available"],
            result["reason"]
                .as_str()
                .map(|s| format!(" ({s})"))
                .unwrap_or_default()
        );
    } else {
        for r in result.as_array().unwrap() {
            println!("{} {}", r["ranks"], r["path"].as_str().unwrap());
        }
    }
    Ok(if command == "probe" && result["available"] != true {
        1
    } else {
        0
    })
}
