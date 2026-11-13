//! Dataset-free oracle executor. This binary must never import the pilot
//! corpus, reference patches, prompts or host expected outputs.
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};
async fn execute(args: &[String]) -> Value {
    let (command, required) = match args {
        [mode, compiler, linker]
            if mode == "compile"
                && Path::new(compiler).is_absolute()
                && Path::new(linker).is_absolute() =>
        {
            (
                vec![
                    compiler.clone(),
                    "--edition=2024".into(),
                    format!("-Clinker={linker}"),
                    "/input/main.rs".into(),
                    "-o".into(),
                    "/work/candidate".into(),
                ],
                vec![compiler.clone(), linker.clone()],
            )
        }
        [mode] if mode == "execute" => (
            vec!["/input/candidate".into()],
            vec!["/input/candidate".into()],
        ),
        _ => {
            return json!({"schema":"ab-oracle-child-v1","completed":false,"error_code":"invalid-command"});
        }
    };
    // A toolchain that cannot start is an unavailable oracle, not a failed artifact.
    if !required
        .iter()
        .all(|path| std::fs::metadata(path).is_ok_and(|meta| meta.is_file()))
    {
        return json!({"schema":"ab-oracle-child-v1","completed":false,"error_code":"child-unavailable"});
    }
    match infra_host::process::run(
        &command,
        Path::new("/work"),
        &BTreeMap::new(),
        &[],
        infra_host::process::Limits {
            seconds: Some(8),
            output: 64 * 1024,
            line: 64 * 1024,
        },
        Arc::new(AtomicBool::new(false)),
        |_| Ok(()),
    )
    .await
    {
        Ok(output) => {
            json!({"schema":"ab-oracle-child-v1","completed":true,"code":output.code,"output":String::from_utf8_lossy(&output.bytes)})
        }
        Err(error) => {
            json!({"schema":"ab-oracle-child-v1","completed":false,"error_code":error.code,"error":error.message})
        }
    }
}
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|arg| arg == "internal-guard") {
        std::process::exit(infra_host::process::guard_entry(&args[1..]).unwrap_or(125));
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()
        .unwrap();
    println!("{}", runtime.block_on(execute(&args)));
}
