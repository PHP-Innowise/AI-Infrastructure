pub mod catalog;
pub mod common;
pub mod evidence;
pub mod ledger;
pub mod manifest;
pub mod ooxml;
pub mod validate;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
pub fn run(command: &str, args: &[String]) -> Result<i32> {
    if command == "manifest" {
        return manifest::run(args);
    }
    let root = common::root()?;
    run_at(&root, command, args)
}
pub fn run_at(root: &std::path::Path, command: &str, args: &[String]) -> Result<i32> {
    let root = root.to_path_buf();
    match command {
        "manifest" => manifest::run(args),
        "run" => evidence::run(&root, args),
        "validate" => validate::run(&root, args),
        "reconstruct-workbook" => {
            let (o, p) = common::parse_args(
                args,
                &["--evidence-root", "--current", "--output"],
                &["--force"],
            )?;
            if !p.is_empty() {
                bail!("unexpected argument");
            }
            let e = PathBuf::from(common::option(
                &o,
                "--evidence-root",
                &root
                    .join("docs/Accelerator-TestEvidence-Submission")
                    .to_string_lossy(),
            ));
            let c = PathBuf::from(common::option(
                &o,
                "--current",
                &root.join("Accelerator TestCases.xlsx").to_string_lossy(),
            ));
            let out = PathBuf::from(common::option(
                &o,
                "--output",
                &root
                    .join("Accelerator TestCases.reconstructed.xlsx")
                    .to_string_lossy(),
            ));
            ooxml::reconstruct(&e, &c, &out, o.contains_key("--force"))?;
            println!("Wrote reconstructed workbook: {}", out.display());
            Ok(0)
        }
        "build-ledger" => {
            let (o, p) = common::parse_args(
                args,
                &["--evidence-root", "--workbook", "--output"],
                &["--release"],
            )?;
            if !p.is_empty() {
                bail!("unexpected argument");
            }
            let e = PathBuf::from(common::option(
                &o,
                "--evidence-root",
                &root
                    .join("docs/Accelerator-TestEvidence-Submission")
                    .to_string_lossy(),
            ));
            let w = PathBuf::from(common::option(
                &o,
                "--workbook",
                &root.join("Accelerator TestCases.xlsx").to_string_lossy(),
            ));
            let out = PathBuf::from(common::option(
                &o,
                "--output",
                &root
                    .join("docs/qa/75-run-disposition.json")
                    .to_string_lossy(),
            ));
            common::write_json(
                &out,
                &ledger::build(&root, &e, &w, o.contains_key("--release"))?,
            )?;
            println!("Wrote75 ledger records to {}", out.display());
            Ok(0)
        }
        "apply-decisions" => {
            let (o, p) = common::parse_args(args, &["--ledger", "--decisions", "--output"], &[])?;
            if !p.is_empty() {
                bail!("unexpected argument");
            }
            let input = PathBuf::from(common::option(
                &o,
                "--ledger",
                &root
                    .join("docs/qa/75-run-disposition.json")
                    .to_string_lossy(),
            ));
            let decisions = PathBuf::from(common::option(
                &o,
                "--decisions",
                &root
                    .join("docs/qa/disposition-decisions.json")
                    .to_string_lossy(),
            ));
            let out = PathBuf::from(common::option(&o, "--output", &input.to_string_lossy()));
            common::write_json(
                &out,
                &ledger::apply(
                    &root,
                    &common::read_json(&input)?,
                    &common::read_json(&decisions)?,
                )?,
            )?;
            Ok(0)
        }
        "migrate-catalog" => {
            let (o, p) = common::parse_args(
                args,
                &["--input", "--mappings", "--output", "--legacy-output"],
                &[],
            )?;
            if !p.is_empty() {
                bail!("unexpected argument");
            }
            let required = |key: &str| -> Result<&str> {
                o.get(key)
                    .and_then(|v| v.last())
                    .map(String::as_str)
                    .with_context(|| format!("missing {key}"))
            };
            let input = common::read_json(&PathBuf::from(required("--input")?))?;
            let map = common::read_json(&PathBuf::from(required("--mappings")?))?;
            let migrated = evidence::migrate_catalog(&input, &map)?;
            let legacy = PathBuf::from(common::option(
                &o,
                "--legacy-output",
                &root.join("qa/tc-ai/cases.v1.json").to_string_lossy(),
            ));
            if legacy.exists() {
                if common::read_json(&legacy)? != input {
                    bail!("legacy catalog output exists with different bytes");
                }
            } else {
                common::write_json(&legacy, &input)?;
            }
            common::write_json(&PathBuf::from(required("--output")?), &migrated)?;
            Ok(0)
        }
        _ => bail!("unknown QA operation"),
    }
}
