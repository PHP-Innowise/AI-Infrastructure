//! Maintainer-only native working-memory rehearsal. No model/provider is contacted.
use anyhow::{Context, Result, bail};
use infra_pilots::run_working_memory::rehearsal;
use infra_system::common;
use std::{collections::BTreeMap, path::PathBuf};
fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args == ["--consumer"] {
        return rehearsal::consumer();
    }
    let mut options = BTreeMap::new();
    let mut recovery = false;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--recover" {
            if recovery {
                bail!("duplicate_recover")
            }
            recovery = true;
            i += 1;
            continue;
        }
        if !["--output", "--redis-endpoint", "--rounds"].contains(&args[i].as_str()) {
            bail!("unknown_option")
        }
        let key = args[i].clone();
        i += 1;
        let value = args.get(i).context("missing_option_value")?.clone();
        i += 1;
        if options.insert(key, value).is_some() {
            bail!("duplicate_option")
        }
    }
    let directory = PathBuf::from(options.get("--output").context("--output required")?);
    let endpoint = options
        .get("--redis-endpoint")
        .map(|value| value.parse())
        .transpose()?;
    let rounds = options
        .get("--rounds")
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(20);
    let scenario = if recovery {
        rehearsal::Scenario::reopen(&directory)?
    } else {
        rehearsal::Scenario::create(&directory)?
    };
    let report = rehearsal::trial(&scenario, endpoint, recovery, rounds)?;
    common::save(
        &scenario.directory,
        if recovery {
            "recovery-result.json"
        } else {
            "result.json"
        },
        &report,
        true,
    )?;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
