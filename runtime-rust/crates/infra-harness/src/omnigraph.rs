//! Omnigraph v0.15.0 integration for shared Harness session context graphs.
//!
//! Provides the `.pg`/`.gq` schema bundle, typed HTTP `0.14` client, out-of-process
//! sidecar/Docker/external lifecycle manager (`OmnigraphManager`), and the
//! `OmnigraphDomain` service router.

pub mod client;
pub mod schema;
pub mod sidecar;
mod transfer;

#[cfg(test)]
pub(crate) mod tests;

pub use client::{
    BranchCreateRequest, BranchCreateResponse, BranchDeleteResponse, BranchDetail, BranchDiffQuery,
    BranchListResponse, BranchMergeOutcome, BranchMergeRequest, BranchMergeResponse,
    ChangeBaselineRequest, ChangeBaselineResponse, ChangeBlock, ChangeCause, ChangeFeedQuery,
    ChangeFeedResponse, ChangeImage, ChangeOp, ChangeRequest, ChangeResponse,
    CommitChangesResponse, CommitListQuery, CommitListResponse, CommitOutput, DiffOutput,
    DiffQuery, EntityChange, EntityKind, ExportRequest, GraphSummary, GraphsListResponse,
    HealthResponse, IngestRequest, MergeConflict, MergeConflictKind, OmnigraphClient,
    OmnigraphError, PreconditionFailure, QueryRequest, QueryResponse, RankingOutput,
    ReadinessResponse, SchemaResponse, StoredQueryRequest,
};
pub use schema::{
    CLUSTER_YAML_TEMPLATE, GRAPH_POLICY_YAML, HARNESS_CONTEXT_PG, HARNESS_GRAPH_ID,
    HARNESS_STORED_QUERIES_GQ, OMNIGRAPH_DOCKER_IMAGE, OMNIGRAPH_RELEASE_URL_LINUX_X86_64,
    OMNIGRAPH_VERSION, POLICY_YAML, SERVER_POLICY_YAML, STORED_QUERIES, StoredQueryDef,
    render_cluster_yaml, write_cluster_bundle,
};
pub use sidecar::{OmnigraphConfig, OmnigraphDomain, OmnigraphManager, OmnigraphMode};
pub use transfer::normalize_shared_space_branch;

/// Maps a Harness session ID to its deterministic Omnigraph branch name (`session/<sid12>`).
pub fn session_branch_name(session_id: &str) -> String {
    let cleaned: String = session_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(12)
        .collect();
    let suffix = if cleaned.is_empty() {
        "session"
    } else {
        cleaned.as_str()
    };
    format!("session/{suffix}")
}
