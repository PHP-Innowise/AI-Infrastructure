//! Fixture executable for the SkillCache contracts. It is not an installed
//! artifact. It answers to the executor's two command names, serves the
//! guardian command native capture needs, and otherwise acts as the "runtime"
//! of fixture programs: `<fixture> programs/<id>/program.json [--check]`
//! behaves as that JSON file says.
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let code = match arguments.first().map(String::as_str) {
        Some("internal-guard") => infra_host::process::guard_entry(&arguments[1..]).unwrap_or(1),
        // What an engine serves for the sandboxes the gates and the compiler use.
        Some("internal-discovery-probe") => {
            infra_host::sandbox::probe_entry(&arguments[1..]).unwrap_or(2)
        }
        Some("internal-sandbox-child") => {
            infra_host::sandbox::child_entry(&arguments[1..]).unwrap_or(2)
        }
        Some("skillcache") => infra_skillcache::cli::execute(
            arguments.get(1).map_or("", String::as_str),
            arguments.get(2..).unwrap_or(&[]),
        ),
        Some("skillcache-run") => infra_skillcache::cli::execute_run(&arguments[1..]),
        Some("skillcache-tool") => infra_skillcache::cli::execute_tool(&arguments[1..]),
        Some("skillcache-gate1") => infra_skillcache::cli::execute_gate1(&arguments[1..]),
        Some(path) => program(Path::new(path), arguments.get(1).map(String::as_str)),
        None => 2,
    };
    std::process::exit(code);
}

/// How many times this program has started in this workspace, this start
/// included.
fn started(name: &str) -> u64 {
    let folder = PathBuf::from(".fixture");
    let _ = std::fs::create_dir_all(&folder);
    let path = folder.join(format!("{name}.count"));
    let count = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok())
        .unwrap_or(0)
        + 1;
    let _ = std::fs::write(path, count.to_string());
    count
}

fn program(path: &Path, flag: Option<&str>) -> i32 {
    let Ok(text) = std::fs::read_to_string(path) else {
        eprintln!("fixture: cannot read {}", path.display());
        return 9;
    };
    let Ok(spec) = serde_json::from_str::<Value>(&text) else {
        eprintln!("fixture: {} is not JSON", path.display());
        return 9;
    };
    if flag == Some("--check") {
        let code = spec["check_exit"].as_i64().unwrap_or(0) as i32;
        if code == 0 {
            println!("{}", json!({"status": "complete", "result": {}}));
        } else {
            eprintln!("fixture: the self-test fails as specified");
        }
        return code;
    }
    let name = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("program")
        .to_string();
    let counted = spec["fail_times"].is_u64() || spec["sleep_first_ms"].is_u64();
    let count = if counted { started(&name) } else { 0 };
    if let Some(pairs) = spec["copy"].as_array() {
        for pair in pairs {
            if let (Some(from), Some(to)) = (pair[0].as_str(), pair[1].as_str()) {
                let _ = std::fs::copy(from, to);
            }
        }
    }
    if let Some(path) = spec["env_names_to"].as_str() {
        let mut names = std::env::vars_os()
            .filter_map(|(name, _)| name.into_string().ok())
            .collect::<Vec<_>>();
        names.sort();
        let _ = std::fs::write(path, names.join("\n"));
    }
    if count == 1
        && let Some(milliseconds) = spec["sleep_first_ms"].as_u64()
    {
        std::thread::sleep(std::time::Duration::from_millis(milliseconds));
    }
    if let Some(milliseconds) = spec["sleep_ms"].as_u64() {
        std::thread::sleep(std::time::Duration::from_millis(milliseconds));
    }
    if let Some(text) = spec["stderr"].as_str() {
        eprintln!("{text}");
    }
    if let Some(required) = spec["require_file"].as_str()
        && !Path::new(required).is_file()
    {
        eprintln!("fixture: {required} is missing");
        return 4;
    }
    if spec["fail_times"]
        .as_u64()
        .is_some_and(|times| count <= times)
    {
        eprintln!("fixture: start {count} fails as specified");
        return 3;
    }
    if let Some(code) = spec["exit"].as_i64().filter(|code| *code != 0) {
        eprintln!("fixture: exits with {code} as specified");
        return code as i32;
    }
    if let Some(files) = spec["write"].as_object() {
        for (path, content) in files {
            if let Some(parent) = Path::new(path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, content.as_str().unwrap_or_default());
        }
    }
    let mut result = spec["result"].as_object().cloned().unwrap_or_else(Map::new);
    if spec["echo_input"] == true {
        let input = std::env::var("SKILLCACHE_INPUT").unwrap_or_default();
        result.insert(
            "input".into(),
            serde_json::from_str(&input).unwrap_or(Value::Null),
        );
    }
    if let Some(relative) = spec["touch_package"].as_str() {
        let package = PathBuf::from(std::env::var("SKILLCACHE_PACKAGE").unwrap_or_default());
        let written = std::fs::write(package.join(relative), b"changed").is_ok();
        result.insert("package_written".into(), written.into());
    }
    match spec["stdout"].as_str() {
        Some(raw) => print!("{raw}"),
        None => println!("{}", json!({"status": "complete", "result": result})),
    }
    0
}
