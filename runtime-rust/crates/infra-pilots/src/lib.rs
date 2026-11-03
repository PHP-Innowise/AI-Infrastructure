pub mod excerpt;
pub mod hindsight;
pub mod longmemeval;
pub mod onnx;
pub mod sqlite_vec;
pub mod strict;
use anyhow::{Context, Result, bail};
use excerpt::ExcerptScorer;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::PathBuf,
    time::Duration,
};
fn request(max: usize) -> Result<Vec<u8>> {
    let mut raw = vec![];
    std::io::stdin()
        .take((max + 1) as u64)
        .read_to_end(&mut raw)?;
    if raw.len() > max {
        bail!("input_bound");
    }
    Ok(raw)
}
fn emit(v: &Value) -> Result<()> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, v)?;
    out.write_all(b"\n")?;
    Ok(())
}
/// Pure/stateless typed service facade. Stateful registries, captures, controllers
/// and journals remain explicit native Rust objects with caller-owned lifetime.
pub fn execute(operation: &str, input: &Value) -> Result<Value> {
    match operation {
        "sqlite-vec-fuse" => {
            let l: Vec<String> = serde_json::from_value(input["lexical"].clone())?;
            let d: Vec<String> = serde_json::from_value(input["dense"].clone())?;
            sqlite_vec::fuse(
                &l,
                &d,
                input
                    .get("k")
                    .map(|v| v.as_u64().context("k"))
                    .transpose()?
                    .unwrap_or(60),
            )
        }
        "sqlite-vec-probe" => {
            let paths = ["vec_extension", "lembed_extension", "model"]
                .map(|k| input[k].as_str().map(PathBuf::from));
            Ok(sqlite_vec::probe(
                paths[0].as_deref(),
                paths[1].as_deref(),
                paths[2].as_deref(),
            ))
        }
        "longmemeval-project" => {
            let p = longmemeval::projection::project_trajectory(input)?;
            Ok(
                json!({"trajectory_id":p.trajectory_id,"relative":p.relative,"body":String::from_utf8(p.body)?,"sha256":p.sha256,"input_sha256":p.input_sha256,"states":p.states}),
            )
        }
        "completion-usage" => {
            let raw = input["raw"].as_str().context("raw supplied JSON string")?;
            Ok(longmemeval::usage::completion_usage(raw.as_bytes()))
        }
        "compare-incremental" => longmemeval::comparison::compare_incremental(
            &input["baseline"],
            &input["treatment"],
            &input["acquisition_baseline"],
            &input["acquisition_treatment"],
        ),
        _ => {
            bail!("unknown stateless pilot operation; use typed native objects for stateful pilots")
        }
    }
}
/// CLI refusal contract: supplied-input and local-asset errors exit 2.
/// Operational unavailability already returned as report data keeps its explicit code.
pub fn run_cli(command: &str, args: &[String]) -> infra_core::Result<i32> {
    run(command, args).map_err(|error| {
        infra_core::RuntimeError::invalid(format!("Pilot operation refused: {error:#}"))
    })
}
pub fn run(command: &str, args: &[String]) -> Result<i32> {
    match command {
        "onnx-worker" => onnx::worker(args),
        "sqlite-vec" => {
            let (op, rest) = args.split_first().context("sqlite-vec probe|fuse")?;
            sqlite_vec::run(op, rest)
        }
        "onnx-score" => {
            let mut opts = BTreeMap::new();
            let mut i = 0;
            while i < args.len() {
                if !["--assets", "--worker", "--ort-library", "--timeout"]
                    .contains(&args[i].as_str())
                {
                    bail!("unknown ONNX option");
                }
                let k = args[i].clone();
                i += 1;
                opts.insert(k, args.get(i).context("missing ONNX option value")?.clone());
                i += 1;
            }
            let v = strict::decode(&request(49152)?, 49152, 8, 256)?;
            strict::keys(&v, &["query", "passages"], &["query", "passages"])?;
            let query = v["query"].as_str().context("query string")?;
            let passages: Vec<String> = serde_json::from_value(v["passages"].clone())?;
            let timeout = opts
                .get("--timeout")
                .map(|s| s.parse::<f64>())
                .transpose()?
                .unwrap_or(5.0);
            if !timeout.is_finite() || timeout <= 0.0 {
                bail!("invalid-timeout");
            }
            let worker = opts
                .get("--worker")
                .map(PathBuf::from)
                .unwrap_or(std::env::current_exe()?);
            let mut scorer = onnx::Scorer {
                assets: PathBuf::from(opts.get("--assets").context("--assets required")?),
                worker,
                ort_library: PathBuf::from(
                    opts.get("--ort-library")
                        .context("--ort-library required")?,
                ),
                timeout: Duration::from_secs_f64(timeout.min(5.0)),
            };
            match scorer.score(query, &passages, Duration::from_secs_f64(timeout.min(5.0))) {
                Ok(scores) => {
                    emit(&json!({"scores":scores}))?;
                    Ok(0)
                }
                Err(e) => {
                    emit(&json!({"error":e.reason}))?;
                    Ok(1)
                }
            }
        }
        "longmemeval-project" => {
            if !args.is_empty() {
                bail!("unexpected projection arguments");
            }
            let raw = request(262144)?;
            let v = longmemeval::projection::decode_trajectory(&raw)?;
            emit(&execute(command, &v)?)?;
            Ok(0)
        }
        "completion-usage" => {
            if !args.is_empty() {
                bail!("unexpected usage arguments");
            }
            emit(&longmemeval::usage::completion_usage(&request(524288)?))?;
            Ok(0)
        }
        "compare-incremental" => {
            if !args.is_empty() {
                bail!("unexpected comparison arguments");
            }
            let v = strict::decode(&request(2097152)?, 2097152, 32, 65536)?;
            emit(&execute(command, &v)?)?;
            Ok(0)
        }
        _ => bail!("unknown pilot operation"),
    }
}
