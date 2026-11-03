use crate::{
    HarnessError, Result,
    process::{self, Limits},
};
pub use infra_host::provider::*;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};
#[derive(Clone)]
pub struct ProviderDomain {
    pub providers: std::sync::Arc<std::sync::RwLock<BTreeMap<String, Provider>>>,
}
impl ProviderDomain {
    pub async fn new(overrides: &BTreeMap<String, String>) -> Result<Self> {
        let providers = discover(overrides).await?;
        for p in &providers {
            infra_host::model_catalog::model_options(&p.id, p.executable.as_deref(), None).await?;
        }
        Ok(Self {
            providers: std::sync::Arc::new(std::sync::RwLock::new(
                providers.into_iter().map(|p| (p.id.clone(), p)).collect(),
            )),
        })
    }
    pub fn catalog(&self, id: &str) -> Result<Value> {
        infra_host::model_catalog::cached_options(id).ok_or_else(|| {
            HarnessError::unavailable("The native model catalog has not been loaded.")
        })
    }
    pub fn get(&self, id: &str) -> Result<Provider> {
        self.providers
            .read()
            .map_err(|_| HarnessError::unavailable("Provider registry lock failed."))?
            .get(id)
            .cloned()
            .ok_or_else(|| HarnessError::invalid("Choose a provider."))
    }
    pub fn listing(&self) -> Result<Vec<Value>> {
        Ok(self.providers.read().map_err(|_|HarnessError::unavailable("Provider registry lock failed."))?.values().map(|p|json!({"id":p.id,"name":p.name,"available":p.available,"detail":p.detail,"model_options":infra_host::model_catalog::cached_options(&p.id),"agent_control_detail":match p.id.as_str(){"claude"=>"Native concurrency limits apply; Ultracode uses its own workflow limits.","codex"=>"Uses Codex native helper concurrency; the main agent runs separately.",_=>"Instruction only: Cursor has no verified CLI control to disable or cap helpers."}})).collect())
    }
    pub async fn sign_in(&self) -> Result<Value> {
        let mut out = Vec::new();
        for id in ["claude", "codex", "cursor"] {
            let p = self.get(id)?;
            let login = match id {
                "claude" => "claude auth login",
                "codex" => "codex login",
                _ => "cursor-agent login",
            };
            let mut status = json!({"id":id,"state":"unknown","detail":"","login":login});
            if let Some(executable) = p.executable {
                let extra = match id {
                    "claude" => vec!["auth", "status", "--json"],
                    "codex" => vec!["login", "status"],
                    _ => vec!["--disable-auto-update", "status", "--format", "json"],
                };
                let mut argv = command_argv(&[executable.to_string_lossy().into()], Some(id))?;
                argv.extend(extra.into_iter().map(String::from));
                if let Ok(result) = process::run_raw_with_lock(
                    &argv,
                    &std::env::current_dir()?,
                    &BTreeMap::new(),
                    &[],
                    Limits {
                        seconds: Some(10),
                        output: 65536,
                        line: 65536,
                    },
                    Arc::new(AtomicBool::new(false)),
                    None,
                    |_| Ok(()),
                )
                .await
                {
                    let keyed = match id {
                        "claude" => [
                            "ANTHROPIC_API_KEY",
                            "ANTHROPIC_AUTH_TOKEN",
                            "CLAUDE_CODE_OAUTH_TOKEN",
                        ]
                        .iter()
                        .any(|k| std::env::var_os(k).is_some()),
                        "codex" => ["OPENAI_API_KEY", "CODEX_API_KEY"]
                            .iter()
                            .any(|k| std::env::var_os(k).is_some()),
                        _ => std::env::var_os("CURSOR_API_KEY").is_some(),
                    };
                    if id == "codex" {
                        let output = String::from_utf8_lossy(&result.bytes).to_lowercase();
                        if result.code == 0 && output.contains("logged in") {
                            status["state"] = "signed_in".into();
                        } else if result.code != 0 && output.contains("not logged in") && !keyed {
                            status["state"] = "signed_out".into();
                        }
                    } else if let Ok(value) = crate::strict_json::parse(&result.bytes, 65536) {
                        let signed = &value[if id == "claude" {
                            "loggedIn"
                        } else {
                            "isAuthenticated"
                        }];
                        let own = id == "cursor"
                            || value["apiProvider"].is_null()
                            || value["apiProvider"] == "firstParty";
                        if signed == true {
                            status["state"] = "signed_in".into();
                        } else if signed == false && own && !keyed {
                            status["state"] = "signed_out".into();
                        }
                    }
                }
            }
            status["detail"] = match status["state"].as_str() {
                Some("signed_in") => format!("{} is signed in.", p.name),
                Some("signed_out") => format!("{} reports that it is not signed in.", p.name),
                _ => String::new(),
            }
            .into();
            out.push(status);
        }
        Ok(json!({"providers":out}))
    }
}
impl crate::services::Domain for ProviderDomain {
    fn execute<'a>(&'a self, op: &'a str, request: Value) -> crate::services::Reply<'a> {
        Box::pin(async move {
            if op != "providers.sign_in"
                || request["query"].get("refresh").is_some_and(|v| v != "1")
            {
                return Err(HarnessError::invalid("Invalid sign-in request."));
            }
            Ok(crate::services::Payload::Json(self.sign_in().await?))
        })
    }
}
