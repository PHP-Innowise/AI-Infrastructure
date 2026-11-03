//! Production composition. Every route group has a concrete native manager.
use crate::{
    Result,
    accelerators::Accelerators,
    bindings::{ResultsBridge, SessionLinks, SystemBridge, SystemWorkflow, SystemsDomain},
    blackboard::CoreBlackboards,
    creator::CreatorManager,
    creator_runner::CreatorRunner,
    desktop::{FactoryFuture, ServerConfig, ServerControl, ServicesFactory},
    knowledge::KnowledgeDomain,
    provider::ProviderDomain,
    services::{Domain, NativeServices, Services},
    sessions::{Sessions, SpecialWorkflow},
    store::{self, SessionStore},
    updates::Updates,
};
use serde_json::{Value, json};
#[cfg(target_os = "macos")]
use std::path::Path;
use std::{collections::BTreeMap, sync::Arc};
pub struct NativeFactory;
struct SkillChanges {
    skills: Arc<crate::skills::SkillsDomain>,
}
impl Domain for SkillChanges {
    fn execute<'a>(&'a self, operation: &'a str, request: Value) -> crate::services::Reply<'a> {
        Box::pin(async move {
            let changed = ["skills.install", "skills.apply_change"].contains(&operation);
            let result = self.skills.execute(operation, request).await?;
            if changed {
                crate::commands::invalidate();
            }
            Ok(result)
        })
    }
}
fn sandbox_problem(_providers: &ProviderDomain) -> Result<Value> {
    #[cfg(target_os = "linux")]
    {
        Ok(if crate::provider::find_executable("bwrap").is_none() {
            json!("Discovery requires bubblewrap on Linux.")
        } else {
            Value::Null
        })
    }
    #[cfg(target_os = "macos")]
    {
        return Ok(if !Path::new("/usr/bin/sandbox-exec").is_file() {
            json!("Discovery requires sandbox-exec on macOS.")
        } else {
            Value::Null
        });
    }
    #[cfg(windows)]
    {
        Ok(if !_providers.get("codex")?.available {
            json!(
                "Discovery requires Codex with elevated sandbox and unique permission profiles on Windows."
            )
        } else {
            Value::Null
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = _providers;
        Ok(json!(
            "Discovery isolation is unavailable on this platform."
        ))
    }
}
fn projects(store: &SessionStore, accelerators: &Accelerators) -> Result<Vec<Value>> {
    let mut result = vec![];
    for mut project in store.projects()? {
        project["accelerator"] = accelerators.summary(project["id"].as_str().unwrap_or(""))?;
        result.push(project);
    }
    Ok(result)
}
fn bootstrap(
    store: &SessionStore,
    accelerators: &Accelerators,
    providers: &ProviderDomain,
    updates: &Updates,
    timeout: u64,
    engine: &Value,
) -> Result<Value> {
    Ok(
        json!({"projects":projects(store,accelerators)?,"providers":providers.listing()?,"sessions":store.summaries()?,"update":updates.status()?,
 "workflows":[{"id":"native","name":"Workspace","description":"Use the native agent and the project's own instructions."},{"id":"sdd","name":"SDD · Spec-Driven Development","description":"Specify, plan, break down tasks, implement and review one phase at a time."},{"id":"plan","name":"Plan","description":"Inspect the project and prepare a concrete implementation plan."},{"id":"review","name":"Review","description":"Review the requested scope and report actionable findings."},{"id":"fleet-review","name":"Fleet review","description":"Run reviewers, inspect findings and approve a saved report."}],
 "sdd_phases":[{"id":"initialize","name":"Initialize","description":"Create missing project principles and SDD guidance."},{"id":"specify","name":"Specify","description":"Write the feature specification: what and why."},{"id":"plan","name":"Plan","description":"Translate the reviewed specification into a technical plan."},{"id":"tasks","name":"Tasks","description":"Break the reviewed plan into test-first tasks and checkpoints."},{"id":"implement","name":"Implement / resume","description":"Execute the next task group and stop at its checkpoint."},{"id":"review","name":"Review","description":"Check implementation against acceptance criteria without editing files."}],
 "clash":{"workflows":["native","review"],"stages":{"implement":"The first participant implements, the challenger attacks the change, and the implementer fixes or rebuts objections.","review":"The first participant reviews, the challenger reviews independently and disputes findings, and the reviewer defends or withdraws each one."},"max_rounds":3,"default_rounds":2},
 "accelerators":[{"id":"kit1","name":"Kit 1 · Infrastructure Creator","description":"Scan, review, generate and apply a bespoke accelerator; update manifest-owned files."},{"id":"kit2","name":"Kit 2 · Ready-made editions","description":"Lend an edition to sessions from this clone or install its reviewed files into a project.","editions":["Laravel","Symfony","PHP Core","Cms/wordpress"]},{"id":"kit3","name":"Kit 3 · Open Source Kit","description":"Discover community tools and their installation commands."}],
 "runtime":{"engine":engine,"source_version":accelerators.source_version()?,"timeout_seconds":timeout,"max_active":1,"discovery_sandbox":sandbox_problem(providers)?,"max_agents":40,"default_agent_count":3,"context_excerpt_bytes":3000,"fleet":{"available":true,"backend":"native","max_worker_timeout":3600,"lenses":[{"id":"code-reviewer","name":"Code reviewer"},{"id":"security-reviewer","name":"Security reviewer"},{"id":"performance-optimization","name":"Performance optimization"}],"detail":"Durable Rust worker state; existing bounded checkpoint data is imported without executing Python."}}}),
    )
}
impl ServicesFactory for NativeFactory {
    fn create<'a>(
        &'a self,
        config: &'a ServerConfig,
        control: Arc<ServerControl>,
    ) -> FactoryFuture<'a> {
        Box::pin(async move {
            let store = SessionStore::open(&config.state)?;
            for project in &config.projects {
                store.register(project)?;
            }
            let providers = Arc::new(ProviderDomain::new(&config.providers).await?);
            let knowledge = Arc::new(KnowledgeDomain::new(store.clone(), config.source.clone()));
            let links = Arc::new(SessionLinks::new());
            let blackboards = Arc::new(CoreBlackboards {
                knowledge: knowledge.clone(),
            });
            let runtime_engine =
                infra_tooling::installer::Installer::new(&config.source)?.engine_capabilities()?;
            let gates = Arc::new(infra_adapters::generator_gate::Gate::current()?);
            let mut special: BTreeMap<String, Arc<dyn SpecialWorkflow>> = BTreeMap::new();
            special.insert(
                "creator".into(),
                Arc::new(CreatorRunner {
                    store: store.clone(),
                    source: config.source.clone(),
                    gates,
                }),
            );
            special.insert(
                "system_run".into(),
                Arc::new(SystemWorkflow {
                    store: store.clone(),
                    links: links.clone(),
                    discovery: false,
                }),
            );
            special.insert(
                "system_discovery".into(),
                Arc::new(SystemWorkflow {
                    store: store.clone(),
                    links: links.clone(),
                    discovery: true,
                }),
            );
            let sessions = Sessions::new(
                store.clone(),
                config.source.clone(),
                providers.clone(),
                knowledge.clone(),
                blackboards,
                special,
                config.timeout,
            )?;
            links.bind(Arc::downgrade(&sessions))?;
            let accelerators = Arc::new(Accelerators::new(
                store.clone(),
                config.source.clone(),
                providers.clone(),
                Arc::downgrade(&sessions),
            )?);
            sessions.install_accelerators(accelerators.clone())?;
            let commands = Arc::new(crate::commands::CommandCatalog::new(
                store.clone(),
                config.source.clone(),
                providers.providers.clone(),
            ));
            sessions.install_commands(commands.clone())?;
            let backend = Arc::new(ResultsBridge {
                links: links.clone(),
                knowledge: knowledge.clone(),
                operation: sessions.operation_lock(),
                cancel: sessions.shutdown_cancel(),
            });
            let results = Arc::new(crate::results::Results::new(store.clone(), backend)?);
            sessions.install_results(results.clone())?;
            let delivery =
                Arc::new(crate::delivery::Delivery::open(store.clone(), results.clone()).await?);
            let target: Arc<dyn crate::results::TargetCheckWorkflow> = delivery.clone();
            results.bind_target(Arc::downgrade(&target))?;
            let system_bridge = Arc::new(SystemBridge {
                links: links.clone(),
                store: store.clone(),
            });
            let systems = Arc::new(SystemsDomain {
                manager: Arc::new(infra_system::SystemManager::new(
                    config.source.clone(),
                    store.state.clone(),
                    system_bridge,
                )?),
            });
            let creator = Arc::new(CreatorManager::new(
                store.clone(),
                config.source.clone(),
                Arc::downgrade(&sessions),
            )?);
            let setup = Arc::new(crate::setup::Setup::new(
                store.clone(),
                config.source.clone(),
                Arc::downgrade(&sessions),
                accelerators.clone(),
            ));
            let skills = Arc::new(SkillChanges {
                skills: Arc::new(crate::skills::SkillsDomain::new(
                    store.clone(),
                    config.source.clone(),
                )?),
            });
            let updates = Updates::new(config.source.clone(), Arc::downgrade(&sessions), control);
            let project = Arc::new(crate::projects::ProjectDomain {
                store: store.clone(),
            });
            crate::chat_merge::ChatMerge::new(store.clone()).recover()?;
            let mut domains: BTreeMap<String, Arc<dyn Domain>> = BTreeMap::new();
            for group in ["sessions", "brain", "context"] {
                domains.insert(group.into(), sessions.clone());
            }
            domains.insert("setup".into(), setup);
            domains.insert("accelerators".into(), accelerators.clone());
            domains.insert("systems".into(), systems.clone());
            domains.insert("discovery".into(), systems);
            domains.insert("creator".into(), creator);
            domains.insert("skills".into(), skills);
            domains.insert(
                "results".into(),
                Arc::new(crate::results::ResultService { results }),
            );
            domains.insert("delivery".into(), delivery);
            domains.insert("projects".into(), project.clone());
            domains.insert("files".into(), project);
            domains.insert("commands".into(), commands);
            for group in ["memory", "knowledge"] {
                domains.insert(group.into(), knowledge.clone());
            }
            domains.insert("updates".into(), updates.clone());
            domains.insert("providers".into(), providers.clone());
            let mut overrides: BTreeMap<String, Arc<dyn Domain>> = BTreeMap::new();
            overrides.insert("context.project".into(), knowledge.clone());
            overrides.insert("memory.save".into(), sessions.clone());
            let bs = store.clone();
            let ba = accelerators.clone();
            let bp = providers.clone();
            let bu = updates.clone();
            let timeout = config.timeout;
            let bootstrap =
                Arc::new(move || bootstrap(&bs, &ba, &bp, &bu, timeout, &runtime_engine));
            let closing = sessions.clone();
            let update_close = updates.clone();
            let closer = Arc::new(move || {
                let closing = closing.clone();
                let updates = update_close.clone();
                Box::pin(async move {
                    updates.close();
                    closing.close()?;
                    closing.wait_closed().await
                }) as crate::services::CloseFuture<'static>
            });
            let services = Arc::new(NativeServices::new(
                store.clone(),
                domains,
                bootstrap,
                overrides,
                closer,
            )?);
            sessions.activate()?;
            for project in store.projects()? {
                let id = project["id"].as_str().unwrap_or("");
                if let Err(error) = accelerators.auto_attach(id) {
                    store.set_setting(
                        &format!("accelerator-diagnostic.{id}"),
                        &json!({"error":error.message,"at":store::now()}),
                    )?;
                }
            }
            let reports = accelerators.keep_all_current()?;
            store.set_setting("accelerator-startup-sync", &json!(reports))?;
            updates.start();
            Ok(services as Arc<dyn Services>)
        })
    }
}
