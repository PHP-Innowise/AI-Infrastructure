//! Multi-service code graph target resolution, HTTP domain handlers, live run bridge, and
//! multi-root read-only `system_explain` session adaptation.

mod domain;
mod helpers;
mod live_bridge;
mod resolver;
mod semantics;

pub use domain::SystemGraphDomain;
pub use live_bridge::HarnessCodeGraphBridge;
pub(crate) use resolver::sysrun_launches;
pub use resolver::{
    ResolvedServiceTarget, adapt_system_explain_launch, compute_graph_id_for_root,
    load_system_focus_value, load_system_run_record, registered_project_roots, resolve_all_targets,
    resolve_service_target, resolve_system, system_explain_prompt_prefix, validate_system_explain,
};

#[cfg(test)]
use crate::{HarnessError, Result, store};
#[cfg(test)]
use infra_system::{CodeGraphBridge, DispatchEvidence, DispatchGraphContext, EvidenceFileChange};
#[cfg(test)]
#[allow(unused_imports)]
use resolver::{
    agent_matches, resolve_launch_selector, resolve_targets_from_system_file,
    store_synthetic_target_setting, validate_config_rel,
};

#[cfg(test)]
#[path = "system_graph_tests.rs"]
mod tests;
