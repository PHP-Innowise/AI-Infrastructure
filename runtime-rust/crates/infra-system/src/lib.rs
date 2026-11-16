//! Native system passports, bounded discovery, reviewed plans and sequential execution.
pub mod backend;
pub mod catalog;
pub mod common;
pub mod discovery;
pub mod editor;
pub mod execution;
pub mod manager;
pub mod native;
pub mod planner;
pub mod routes;
pub mod runner;
pub mod system_graph;
pub mod workers;
pub use backend::{Project, SessionBridge};
pub use manager::SystemManager;
pub use routes::{execute, execute_async, initialize};
pub use system_graph::{
    ArchitectureLayerRule, ChangedEntityEntry, ChangedFileEntry, CodeGraphBridge, CodeReference,
    ContractLinkDetail, CrossServiceImpactQuery, DispatchEvidence, DispatchGraphContext,
    EvidenceFileChange, LayerViolationEntry, NoopCodeGraphBridge, ServiceCodeSummary,
    ServiceGraphOffer, SystemArchitectureRules, SystemEdge, SystemGraph, SystemMapQuery,
    SystemNode, collect_git_service_summaries, collect_run_service_summaries,
    enrich_summaries_from_graph_store, git_changes_since, git_worktree_changes, load_run_evidence,
    summarize_run_evidence, system_graph_status,
};
pub use workers::{
    AgentSignal, bwrap_ro_bind_args, macos_seatbelt_read_rules, offers_for_step, prepare_worker,
    run_discovery_request, run_discovery_request_with_adapter, run_system_request,
    run_system_request_with_adapter, run_system_request_with_bridge,
};
