//! JSON façade for the single native engine, CLI and HTTP frontends.
use crate::{
    common,
    installer::{InstallOptions, Installer},
};
use infra_core::{Result, RuntimeError};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
fn path(request: &Value, name: &str, default: Option<&str>) -> Result<PathBuf> {
    request
        .get(name)
        .and_then(Value::as_str)
        .or(default)
        .map(PathBuf::from)
        .ok_or_else(|| RuntimeError::invalid(format!("Missing {name} path")))
}
fn string<'a>(request: &'a Value, name: &str) -> Result<&'a str> {
    request
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| RuntimeError::invalid(format!("Missing {name}")))
}
fn flag(request: &Value, name: &str) -> Result<bool> {
    match request.get(name) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        _ => Err(RuntimeError::invalid(format!("{name} must be boolean"))),
    }
}
fn tools(request: &Value) -> Result<Vec<String>> {
    let Some(input) = request.get("tools") else {
        return Ok(["claude", "cursor", "codex"]
            .into_iter()
            .map(String::from)
            .collect());
    };
    let values = input
        .as_array()
        .ok_or_else(|| RuntimeError::invalid("tools must be an array"))?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(String::from)
                .ok_or_else(|| RuntimeError::invalid("tool names must be strings"))
        })
        .collect()
}
fn editions(request: &Value) -> Result<Vec<String>> {
    let values = if let Some(values) = request.get("editions") {
        optional_strings(&json!({"editions":values}), "editions")?
    } else if let Some(value) = request.get("edition") {
        match value {
            Value::String(value) => vec![value.clone()],
            Value::Array(_) => optional_strings(&json!({"editions":value}), "editions")?,
            _ => {
                return Err(RuntimeError::invalid(
                    "edition must be text or a text array",
                ));
            }
        }
    } else {
        common::EDITIONS
            .iter()
            .map(|value| (*value).to_string())
            .collect()
    };
    if values.is_empty() {
        return Err(RuntimeError::invalid("Select at least one edition"));
    }
    let mut result = Vec::new();
    for value in values {
        let name = common::edition(&value)?.to_string();
        if !result.contains(&name) {
            result.push(name);
        }
    }
    Ok(result)
}
fn options(request: &Value, sync: bool) -> Result<InstallOptions> {
    let mut value = request.get("options").cloned().unwrap_or_else(|| json!({}));
    let object = value
        .as_object_mut()
        .ok_or_else(|| RuntimeError::invalid("Install options must be an object"))?;
    for key in [
        "dry_run",
        "overwrite",
        "merge_existing",
        "update_tracked",
        "rewire_codex",
        "allow_downgrade",
    ] {
        if !object.contains_key(key)
            && let Some(value) = request.get(key)
        {
            object.insert(key.into(), value.clone());
        }
    }
    let options: InstallOptions = serde_json::from_value(value)?;
    if !sync && (options.update_tracked || options.rewire_codex || options.allow_downgrade) {
        return Err(RuntimeError::invalid(
            "update_tracked, rewire_codex and allow_downgrade require sync",
        ));
    }
    Ok(options)
}
pub fn execute(operation: &str, request: &Value) -> Result<Value> {
    if !request.is_object() {
        return Err(RuntimeError::invalid(
            "Native tooling request must be an object",
        ));
    }
    let root = path(request, "root", Some("."))?;
    match operation {
        "core-sync" => {
            crate::sync::core_sync(&root, flag(request, "write")?, !flag(request, "no_chain")?)
        }
        "assets" | "asset-parity" => crate::sync::asset_sync(&root, flag(request, "write")?),
        "mirrors" | "policy-lock" => {
            let mut reports = Vec::new();
            for edition in editions(request)? {
                reports.push(if operation == "mirrors" {
                    crate::mirrors::process(&root, &edition, flag(request, "write")?)?
                } else {
                    crate::policy::lock(&root, &edition, flag(request, "write")?)?
                });
            }
            if reports.len() == 1 {
                Ok(reports.remove(0))
            } else {
                Ok(
                    json!({"passed":reports.iter().all(|report|report["passed"]!=false),"editions":reports}),
                )
            }
        }
        "budget" | "context-budget" => {
            crate::budget::report(&root, flag(request, "check")?, flag(request, "headroom")?)
        }
        "links" => crate::links::check(&root, flag(request, "standalone")?),
        "inventory" => {
            let installer = Installer::new(&root)?;
            Ok(serde_json::to_value(installer.inventory(
                string(request, "edition")?,
                &tools(request)?,
            )?)?)
        }
        "inventory-write" | "inventory-check" => {
            let installer = Installer::new(&root)?;
            let destination = request["inventory_out"].as_str().map(Path::new);
            let editions = if let Some(edition) = request["edition"].as_str() {
                vec![edition.to_string()]
            } else {
                common::PHP_EDITIONS
                    .iter()
                    .map(|edition| (*edition).to_string())
                    .collect()
            };
            let mut reports = Vec::new();
            for edition in editions {
                reports.push(installer.write_inventory_to(
                    &edition,
                    operation == "inventory-write",
                    destination,
                )?);
            }
            Ok(
                json!({"passed":reports.iter().all(|report|report["passed"]!=false),"inventories":reports}),
            )
        }
        "sync" => {
            let mut installer = Installer::new(&root)?;
            if let Some(engine) = request.get("engine").and_then(Value::as_str) {
                installer = installer.with_engine(Path::new(engine))?;
            }
            let options = options(request, true)?;
            installer.sync(&path(request, "target", None)?, &options)
        }
        "detect-edition" => {
            let installer = Installer::new(&root)?;
            Ok(json!({"edition":installer.detect_edition(&path(request,"target",None)?)?}))
        }
        "install" | "overlay" | "install-plan" => {
            let mut installer = Installer::new(&root)?;
            if let Some(engine) = request.get("engine").and_then(Value::as_str) {
                installer = installer.with_engine(Path::new(engine))?;
            }
            let target = path(request, "target", None)?;
            let edition = string(request, "edition")?;
            let tools = tools(request)?;
            if operation == "overlay" {
                installer.overlay(&target, edition, &tools)
            } else {
                let options = options(request, false)?;
                if operation == "install-plan" {
                    Ok(serde_json::to_value(
                        installer.plan(&target, edition, &tools, &options)?,
                    )?)
                } else {
                    installer.install(&target, edition, &tools, &options)
                }
            }
        }
        "install-apply" => {
            let mut installer = Installer::new(&root)?;
            if let Some(engine) = request["engine"].as_str() {
                installer = installer.with_engine(Path::new(engine))?;
            }
            let plan: crate::installer::InstallPlan = serde_json::from_value(
                request
                    .get("plan")
                    .cloned()
                    .ok_or_else(|| RuntimeError::invalid("Missing reviewed install plan"))?,
            )?;
            installer.apply_plan(&plan)
        }
        "install-preview-images" => {
            let mut installer = Installer::new(&root)?;
            if let Some(engine) = request["engine"].as_str() {
                installer = installer.with_engine(Path::new(engine))?;
            }
            let plan: crate::installer::InstallPlan = serde_json::from_value(
                request
                    .get("plan")
                    .cloned()
                    .ok_or_else(|| RuntimeError::invalid("Missing reviewed install plan"))?,
            )?;
            Ok(serde_json::to_value(installer.preview_images(
                &plan,
                request["max_text_bytes"].as_u64().unwrap_or(16384) as usize,
            )?)?)
        }
        "hook-handler" => {
            let home = infra_core::fs::absolute(&root)?;
            let engine = path(request, "engine", None)?;
            crate::attach::hook_handler(
                &engine,
                &home,
                string(request, "host")?,
                string(request, "role")?,
                cfg!(windows),
            )
        }
        "native-handler" => {
            let engine = path(request, "engine", None)?;
            crate::attach::native_handler(
                &engine,
                &optional_strings(
                    request,
                    if request.get("arguments").is_some() {
                        "arguments"
                    } else {
                        "_positionals"
                    },
                )?,
                string(request, "host")?,
                cfg!(windows),
            )
        }
        "mcp-config" | "mcp-register" => crate::mcp_config::execute(request),
        "php-snippets" => crate::php::check(&root, flag(request, "require_php")?),
        "stabilization" => {
            crate::stabilization::check(&root, &optional_strings(request, "editions")?)
        }
        "changelog" => crate::changelog::check(
            &root,
            request["base"].as_str().or_else(|| {
                request["_positionals"]
                    .as_array()
                    .and_then(|values| values.first())
                    .and_then(Value::as_str)
            }),
            flag(request, "strict")?,
        ),
        "routes" => crate::routes::check(
            &root,
            &optional_strings(request, "editions")?,
            request["allowlist"].as_str().map(Path::new),
        ),
        "registry" => {
            crate::registry::execute(request["action"].as_str().unwrap_or("check"), request)
        }
        "kit" | "open-source-kit" => kit_reply(crate::kit::execute(
            request["action"].as_str().unwrap_or("catalog"),
            request,
        )?),
        "catalog" => kit_reply(crate::kit::execute("catalog", request)?),
        "collect" => crate::collect::execute(request),
        "release" => {
            crate::release::execute(request["action"].as_str().unwrap_or("categorize"), request)
        }
        "release-categorize" => crate::release::execute("categorize", request),
        "json-syntax" => crate::checker::check_tracked_json(&root),
        "check" => crate::checker::execute(request),
        "build-ui" => crate::ui_build::build(&root, request),
        "write-ci" | "check-ci" => crate::checker::write_ci(&root, operation == "write-ci"),
        "attach" => crate::attach::execute(
            &format!("attach-{}", request["action"].as_str().unwrap_or("overlay")),
            request,
        ),
        "artifacts" | "assemble" => crate::artifacts::assemble(
            &root,
            &optional_strings(request, "editions")?,
            flag(request, "include_generator")?,
            flag(request, "write")?,
        ),
        operation if operation.starts_with("registry-") => crate::registry::execute(
            operation.strip_prefix("registry-").unwrap_or("check"),
            request,
        ),
        operation if operation.starts_with("kit-") => kit_reply(crate::kit::execute(
            operation.strip_prefix("kit-").unwrap_or("catalog"),
            request,
        )?),
        operation if operation.starts_with("attach-") => crate::attach::execute(operation, request),
        _ => Err(RuntimeError::invalid(format!(
            "Unknown native tooling operation: {operation}"
        ))),
    }
}

fn optional_strings(request: &Value, key: &str) -> Result<Vec<String>> {
    match request.get(key) {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(String::from)
                    .ok_or_else(|| RuntimeError::invalid(format!("{key} must contain strings")))
            })
            .collect(),
        _ => Err(RuntimeError::invalid(format!("{key} must be an array"))),
    }
}
fn kit_reply(value: Value) -> Result<Value> {
    if value["stdio_inherited"] == true {
        return Ok(
            json!({"_cli":{"code":value["exit_code"].as_i64().unwrap_or(0),"stdout":"","stderr":""},"result":value}),
        );
    }
    if let Some(code) = value["exit_code"].as_i64() {
        return Ok(
            json!({"_cli":{"code":code,"stdout":value["stdout"].as_str().unwrap_or(""),"stderr":value["stderr"].as_str().unwrap_or("")},"result":value}),
        );
    }
    Ok(value)
}

/// Interpret a native report consistently for direct CI and CLI callers.
pub fn report_exit_code(value: &Value) -> i32 {
    let number = |value: &Value| value.as_i64().map(|code| i32::try_from(code).unwrap_or(1));
    if let Some(code) = number(&value["_cli"]["code"]).or_else(|| number(&value["exit_code"]))
        && code != 0
    {
        return code;
    }
    let populated = |value: &Value| match value {
        Value::Null | Value::Bool(false) => false,
        Value::Number(number) => number.as_i64().is_none_or(|value| value != 0),
        Value::String(value) => !value.is_empty(),
        Value::Array(values) => !values.is_empty(),
        Value::Object(values) => !values.is_empty(),
        Value::Bool(true) => true,
    };
    if populated(&value["collisions"]) {
        return 2;
    }
    if value["passed"] == false
        || value["valid"] == false
        || populated(&value["errors"])
        || populated(&value["error"])
        || matches!(
            value["status"].as_str(),
            Some("fail" | "failed" | "error" | "refused")
        )
    {
        return 1;
    }
    0
}
