//! Typed async HTTP `0.14` client for Omnigraph v0.15.0 servers.

use crate::HarnessError;
use reqwest::{Method, Response, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// Required contract header on all `/graphs*` and `/cluster*` endpoints.
pub const HTTP_API_CONTRACT_HEADER: &str = "Omnigraph-Http-Api";
/// HTTP API contract version spoken by Omnigraph v0.15.0.
pub const HTTP_API_CONTRACT_VERSION: &str = "0.14";
/// Conditional write commit CAS header.
pub const IF_GRAPH_COMMIT_HEADER: &str = "Omnigraph-If-Graph-Commit";
/// Conditional actor verification header.
pub const IF_ACTOR_HEADER: &str = "Omnigraph-If-Actor";

/// Typed error returned by the Omnigraph HTTP client.
#[derive(Clone, Debug)]
pub struct OmnigraphError {
    pub status: u16,
    pub code: Option<String>,
    pub message: String,
    pub merge_conflicts: Vec<MergeConflict>,
    pub precondition_failure: Option<Box<PreconditionFailure>>,
    pub change_feed_gap: Option<Box<ChangeFeedGap>>,
    pub raw_body: Option<Box<Value>>,
}

impl OmnigraphError {
    /// Construct a transport or client-side error.
    pub fn transport(message: impl Into<String>) -> Self {
        Self {
            status: 503,
            code: Some("unavailable".into()),
            message: message.into(),
            merge_conflicts: Vec::new(),
            precondition_failure: None,
            change_feed_gap: None,
            raw_body: None,
        }
    }

    /// Construct a client-side validation error.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            status: 400,
            code: Some("bad_request".into()),
            message: message.into(),
            merge_conflicts: Vec::new(),
            precondition_failure: None,
            change_feed_gap: None,
            raw_body: None,
        }
    }

    /// Whether this error carries HTTP 409 merge conflicts.
    pub fn is_merge_conflict(&self) -> bool {
        self.status == 409 && !self.merge_conflicts.is_empty()
    }

    /// Whether this error is a graph-commit CAS precondition failure (`412`).
    pub fn is_precondition_failed(&self) -> bool {
        self.status == 412
            && (self.code.as_deref() == Some("precondition_failed")
                || self.precondition_failure.is_some())
    }

    /// Whether this error is an `Omnigraph-If-Actor` mismatch (`412`).
    pub fn is_actor_mismatch(&self) -> bool {
        self.status == 412 && self.code.as_deref() == Some("actor_mismatch")
    }

    /// Whether the target resource was not found (`404`).
    pub fn is_not_found(&self) -> bool {
        self.status == 404 || self.code.as_deref() == Some("not_found")
    }

    /// Whether a change-feed cursor fell behind retained history (`410`).
    pub fn is_change_feed_gap(&self) -> bool {
        self.status == 410 || self.code.as_deref() == Some("change_feed_gap")
    }

    /// Convert into a `HarnessError` with matching HTTP status semantics.
    pub fn into_harness_error(self) -> HarnessError {
        match self.status {
            400 => HarnessError::invalid(self.message),
            401 | 403 => HarnessError::forbidden(self.message),
            404 => HarnessError::missing(self.message),
            409 | 412 => HarnessError::conflict(self.message),
            _ => HarnessError::unavailable(self.message),
        }
    }
}

impl std::fmt::Display for OmnigraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(code) = &self.code {
            write!(
                f,
                "omnigraph HTTP {} ({code}): {}",
                self.status, self.message
            )
        } else {
            write!(f, "omnigraph HTTP {}: {}", self.status, self.message)
        }
    }
}

impl std::error::Error for OmnigraphError {}

impl From<OmnigraphError> for HarnessError {
    fn from(value: OmnigraphError) -> Self {
        value.into_harness_error()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreconditionFailure {
    pub expected: String,
    #[serde(default)]
    pub actual: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangeFeedGap {
    #[serde(default)]
    pub cursor: Option<String>,
    pub first_unreadable_commit_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Node,
    Edge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeConflictKind {
    DivergentInsert,
    DivergentUpdate,
    DeleteVsUpdate,
    OrphanEdge,
    UniqueViolation,
    CardinalityViolation,
    ValueConstraintViolation,
    #[serde(other)]
    Other,
}

impl MergeConflictKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DivergentInsert => "divergent_insert",
            Self::DivergentUpdate => "divergent_update",
            Self::DeleteVsUpdate => "delete_vs_update",
            Self::OrphanEdge => "orphan_edge",
            Self::UniqueViolation => "unique_violation",
            Self::CardinalityViolation => "cardinality_violation",
            Self::ValueConstraintViolation => "value_constraint_violation",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeConflict {
    pub entity_kind: EntityKind,
    pub type_name: String,
    #[serde(default)]
    pub entity_id: Option<String>,
    pub kind: MergeConflictKind,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    #[serde(default)]
    pub internal_schema_version: u32,
    #[serde(default)]
    pub source_version: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReadinessResponse {
    pub ready: bool,
    pub status: String,
    #[serde(default)]
    pub booted_serving_digest: Option<String>,
    #[serde(default)]
    pub state_revision: u64,
    #[serde(default)]
    pub state_cas: Option<String>,
    #[serde(default)]
    pub served_graph_count: usize,
    #[serde(default)]
    pub ready_graph_count: usize,
    #[serde(default)]
    pub loading_graph_count: usize,
    #[serde(default)]
    pub blocked_graph_count: usize,
    #[serde(default)]
    pub shutdown_grace_seconds: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GraphSummary {
    pub graph_id: String,
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub read_available: bool,
    #[serde(default)]
    pub write_available: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GraphsListResponse {
    #[serde(default)]
    pub graphs: Vec<GraphSummary>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SystemColumns {
    pub id: String,
    pub src: String,
    pub dst: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SchemaResponse {
    pub schema_source: String,
    #[serde(default)]
    pub system_columns: Option<SystemColumns>,
    #[serde(default)]
    pub catalogue_source: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QueryRequest {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StoredQueryRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expect_mutation: Option<bool>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReadTarget {
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub snapshot: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RankingOutput {
    pub by: String,
    pub order: String,
    #[serde(default)]
    pub exhaustive: bool,
    #[serde(default)]
    pub score_columns: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QueryResponse {
    #[serde(default)]
    pub query_name: String,
    #[serde(default)]
    pub target: ReadTarget,
    #[serde(default)]
    pub row_count: usize,
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub rows: Vec<Value>,
    #[serde(default)]
    pub graph_commit_id: Option<String>,
    #[serde(default)]
    pub page: Option<Value>,
    #[serde(default)]
    pub ranking: Option<RankingOutput>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChangeRequest {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitOutput {
    pub graph_commit_id: String,
    #[serde(default)]
    pub graph_branch: Option<String>,
    #[serde(default)]
    pub graph_manifest_version: u64,
    #[serde(default)]
    pub parent_commit_id: Option<String>,
    #[serde(default)]
    pub merged_parent_commit_id: Option<String>,
    #[serde(default)]
    pub actor_id: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default = "default_true")]
    pub published: bool,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub root_commit_id: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChangeResponse {
    pub branch: String,
    #[serde(default)]
    pub query_name: String,
    #[serde(default)]
    pub affected_nodes: usize,
    #[serde(default)]
    pub affected_edges: usize,
    #[serde(default)]
    pub actor_id: Option<String>,
    #[serde(default)]
    pub commit: Option<CommitOutput>,
    #[serde(default)]
    pub outcome: Option<Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BranchCreateRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_commit: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BranchCreateResponse {
    #[serde(default)]
    pub uri: String,
    pub from: String,
    pub name: String,
    #[serde(default)]
    pub actor_id: Option<String>,
    pub from_commit_id: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BranchDetail {
    pub name: String,
    pub head: CommitOutput,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub from_commit_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BranchListResponse {
    #[serde(default)]
    pub branches: Vec<String>,
    #[serde(default)]
    pub details: Option<Vec<BranchDetail>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BranchDeleteResponse {
    #[serde(default)]
    pub uri: String,
    pub name: String,
    #[serde(default)]
    pub actor_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BranchMergeRequest {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default)]
    pub delete_branch: bool,
    #[serde(default)]
    pub as_commit: bool,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchMergeOutcome {
    AlreadyUpToDate,
    FastForward,
    Merged,
}

impl BranchMergeOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AlreadyUpToDate => "already_up_to_date",
            Self::FastForward => "fast_forward",
            Self::Merged => "merged",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BranchMergeResponse {
    pub source: String,
    pub target: String,
    pub outcome: BranchMergeOutcome,
    #[serde(default)]
    pub commit: Option<CommitOutput>,
    #[serde(default)]
    pub actor_id: Option<String>,
    #[serde(default)]
    pub branch_deleted: Option<bool>,
    #[serde(default)]
    pub branch_delete_error_details: Option<Value>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeOp {
    Insert,
    Update,
    Delete,
}

impl ChangeOp {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Insert => "insert",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeTypeInfo {
    #[serde(default)]
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeEndpoints {
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChangeImage {
    pub properties: Value,
    #[serde(default)]
    pub endpoints: Option<ChangeEndpoints>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityChange {
    pub kind: EntityKind,
    #[serde(rename = "type")]
    pub entity_type: ChangeTypeInfo,
    pub id: String,
    pub op: ChangeOp,
    #[serde(default)]
    pub before: Option<ChangeImage>,
    #[serde(default)]
    pub after: Option<ChangeImage>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeCause {
    pub graph_commit_id: String,
    #[serde(default)]
    pub parent_commit_id: Option<String>,
    #[serde(default)]
    pub merged_parent_commit_id: Option<String>,
    pub authored_branch: String,
    #[serde(default)]
    pub actor_id: Option<String>,
    #[serde(default)]
    pub authored_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CommitListQuery {
    pub branch: Option<String>,
    pub limit: Option<usize>,
    pub page_token: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CommitListResponse {
    #[serde(default)]
    pub commits: Vec<CommitOutput>,
    #[serde(default)]
    pub head_commit_id: Option<String>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CommitChangesResponse {
    pub cause: ChangeCause,
    #[serde(default)]
    pub changes: Vec<EntityChange>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DiffQuery {
    pub from: String,
    pub to: String,
    pub limit: Option<usize>,
    pub page_token: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BranchDiffQuery {
    pub source: String,
    pub target: Option<String>,
    pub limit: Option<usize>,
    pub page_token: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DiffOutput {
    pub from_commit_id: String,
    pub to_commit_id: String,
    #[serde(default)]
    pub base_commit_id: Option<String>,
    #[serde(default)]
    pub changes: Vec<EntityChange>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChangeFeedQuery {
    pub branch: Option<String>,
    pub cursor: Option<String>,
    pub start: Option<String>,
    pub page_token: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChangeBlock {
    pub cause: ChangeCause,
    #[serde(default)]
    pub changes: Vec<EntityChange>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChangeFeedResponse {
    #[serde(default)]
    pub blocks: Vec<ChangeBlock>,
    #[serde(default)]
    pub next_page_token: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub caught_up: Option<bool>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChangeBaselineRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kind: Vec<EntityKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub r#type: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub op: Vec<ChangeOp>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChangeBaselineResponse {
    pub snapshot_commit_id: String,
    pub resume_cursor: String,
    #[serde(default)]
    pub records: Vec<Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExportRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default)]
    pub type_names: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IngestRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    pub data: String,
}

/// Async client bound to a single Omnigraph endpoint + graph id.
#[derive(Clone, Debug)]
pub struct OmnigraphClient {
    http: reqwest::Client,
    base_url: String,
    token: Option<String>,
    graph_id: String,
    expected_actor: Option<String>,
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

impl OmnigraphClient {
    /// Build a shared pooled `reqwest::Client` configured for Omnigraph HTTP requests.
    pub fn build_http_client(timeout: Duration) -> Result<reqwest::Client, OmnigraphError> {
        reqwest::Client::builder()
            .timeout(timeout)
            .user_agent("AI-Infrastructure-Harness")
            .build()
            .map_err(|err| OmnigraphError::transport(format!("HTTP client init failed: {err}")))
    }

    /// Create a new client targeting `base_url` and `graph_id` with its own pooled HTTP client.
    pub fn new(
        base_url: impl Into<String>,
        token: Option<String>,
        graph_id: impl Into<String>,
    ) -> Result<Self, OmnigraphError> {
        let http = Self::build_http_client(Duration::from_secs(30))?;
        Self::with_http_client(base_url, token, graph_id, http)
    }

    /// Create a new client targeting `base_url` and `graph_id` reusing an existing `reqwest::Client` pool.
    pub fn with_http_client(
        base_url: impl Into<String>,
        token: Option<String>,
        graph_id: impl Into<String>,
        http: reqwest::Client,
    ) -> Result<Self, OmnigraphError> {
        let raw = base_url.into();
        let trimmed = raw.trim().trim_end_matches('/').to_string();
        let parsed = Url::parse(&trimmed)
            .map_err(|err| OmnigraphError::invalid(format!("Invalid Omnigraph URL: {err}")))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(OmnigraphError::invalid(
                "Omnigraph endpoint must use http or https.",
            ));
        }
        let graph_id = graph_id.into();
        if graph_id.trim().is_empty() {
            return Err(OmnigraphError::invalid(
                "Omnigraph graph_id cannot be empty.",
            ));
        }
        Ok(Self {
            http,
            base_url: trimmed,
            token: token
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty()),
            graph_id,
            expected_actor: None,
        })
    }

    /// Require that the server authenticates the bearer token as `actor_id` via `Omnigraph-If-Actor`.
    pub fn with_actor(mut self, actor_id: Option<String>) -> Self {
        self.expected_actor = actor_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        self
    }

    /// Configured endpoint base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Configured target graph id.
    pub fn graph_id(&self) -> &str {
        &self.graph_id
    }

    /// Configured Streamable HTTP MCP URL (active when server runs with auth).
    pub fn mcp_url(&self) -> String {
        format!("{}/mcp", self.base_url)
    }

    fn graph_url(&self, suffix: &str) -> Result<Url, OmnigraphError> {
        let encoded_graph = encode_segment(&self.graph_id);
        let full = format!("{}/graphs/{encoded_graph}{suffix}", self.base_url);
        Url::parse(&full)
            .map_err(|err| OmnigraphError::invalid(format!("Invalid graph URL '{full}': {err}")))
    }

    fn root_url(&self, path: &str) -> Result<Url, OmnigraphError> {
        let full = format!("{}{path}", self.base_url);
        Url::parse(&full)
            .map_err(|err| OmnigraphError::invalid(format!("Invalid URL '{full}': {err}")))
    }

    async fn send_raw(
        &self,
        method: Method,
        url: Url,
        contract_headers: bool,
        if_commit: Option<&str>,
        json_body: Option<Vec<u8>>,
    ) -> Result<Response, OmnigraphError> {
        self.send_raw_with_timeout(
            method,
            url,
            contract_headers,
            if_commit,
            None,
            json_body.as_ref().map(|_| "application/json"),
            json_body,
            None,
        )
        .await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Shared low-level transport helper for JSON and NDJSON endpoints."
    )]
    async fn send_raw_with(
        &self,
        method: Method,
        url: Url,
        contract_headers: bool,
        if_commit: Option<&str>,
        actor_override: Option<&str>,
        content_type: Option<&str>,
        body: Option<Vec<u8>>,
    ) -> Result<Response, OmnigraphError> {
        self.send_raw_with_timeout(
            method,
            url,
            contract_headers,
            if_commit,
            actor_override,
            content_type,
            body,
            None,
        )
        .await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Shared low-level transport helper for JSON, NDJSON, and probe endpoints."
    )]
    async fn send_raw_with_timeout(
        &self,
        method: Method,
        url: Url,
        contract_headers: bool,
        if_commit: Option<&str>,
        actor_override: Option<&str>,
        content_type: Option<&str>,
        body: Option<Vec<u8>>,
        timeout: Option<Duration>,
    ) -> Result<Response, OmnigraphError> {
        let mut req = self.http.request(method, url);
        if let Some(t) = timeout {
            req = req.timeout(t);
        }
        if contract_headers {
            req = req.header(HTTP_API_CONTRACT_HEADER, HTTP_API_CONTRACT_VERSION);
            if let Some(token) = &self.token {
                req = req.bearer_auth(token);
            }
            let actor = actor_override
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or(self.expected_actor.as_deref());
            if let Some(actor) = actor {
                req = req.header(IF_ACTOR_HEADER, actor);
            }
        }
        if let Some(commit) = if_commit {
            req = req.header(IF_GRAPH_COMMIT_HEADER, commit);
        }
        if let Some(bytes) = body {
            if let Some(ct) = content_type {
                req = req.header(reqwest::header::CONTENT_TYPE, ct);
            }
            req = req.body(bytes);
        }
        req.send()
            .await
            .map_err(|err| OmnigraphError::transport(format!("Request failed: {err}")))
    }

    async fn decode_json<T: for<'de> Deserialize<'de>>(
        resp: Response,
    ) -> Result<T, OmnigraphError> {
        let status = resp.status();
        let bytes = resp
            .bytes()
            .await
            .map_err(|err| OmnigraphError::transport(format!("Failed reading body: {err}")))?;
        if !status.is_success() {
            return Err(parse_error(status, &bytes));
        }
        serde_json::from_slice(&bytes).map_err(|err| {
            OmnigraphError::transport(format!(
                "Failed decoding Omnigraph response ({status}): {err}"
            ))
        })
    }

    async fn get_contract<T: for<'de> Deserialize<'de>>(
        &self,
        url: Url,
    ) -> Result<T, OmnigraphError> {
        let resp = self.send_raw(Method::GET, url, true, None, None).await?;
        Self::decode_json(resp).await
    }

    async fn post_contract<B: Serialize, T: for<'de> Deserialize<'de>>(
        &self,
        url: Url,
        body: &B,
        if_commit: Option<&str>,
    ) -> Result<T, OmnigraphError> {
        let payload = serde_json::to_vec(body)
            .map_err(|err| OmnigraphError::invalid(format!("Failed serializing body: {err}")))?;
        let resp = self
            .send_raw(Method::POST, url, true, if_commit, Some(payload))
            .await?;
        Self::decode_json(resp).await
    }

    /// Probe `GET /healthz` with a short probe timeout.
    pub async fn health(&self) -> Result<HealthResponse, OmnigraphError> {
        self.health_with_timeout(PROBE_TIMEOUT).await
    }

    /// Probe `GET /healthz` with an explicit request timeout.
    pub async fn health_with_timeout(
        &self,
        timeout: Duration,
    ) -> Result<HealthResponse, OmnigraphError> {
        let url = self.root_url("/healthz")?;
        let resp = self
            .send_raw_with_timeout(
                Method::GET,
                url,
                false,
                None,
                None,
                None,
                None,
                Some(timeout),
            )
            .await?;
        Self::decode_json(resp).await
    }

    /// Probe `GET /readyz` with a short probe timeout.
    pub async fn ready(&self) -> Result<ReadinessResponse, OmnigraphError> {
        let url = self.root_url("/readyz")?;
        let resp = self
            .send_raw_with_timeout(
                Method::GET,
                url,
                false,
                None,
                None,
                None,
                None,
                Some(PROBE_TIMEOUT),
            )
            .await?;
        Self::decode_json(resp).await
    }

    /// List served graphs (`GET /graphs`).
    pub async fn list_graphs(&self) -> Result<GraphsListResponse, OmnigraphError> {
        let url = self.root_url("/graphs")?;
        self.get_contract(url).await
    }

    /// Inspect graph schema (`GET /graphs/{graph_id}/schema`).
    pub async fn schema(&self) -> Result<SchemaResponse, OmnigraphError> {
        let url = self.graph_url("/schema")?;
        self.get_contract(url).await
    }

    /// Execute an inline GQ read query (`POST /graphs/{graph_id}/query`).
    pub async fn query(&self, req: &QueryRequest) -> Result<QueryResponse, OmnigraphError> {
        let url = self.graph_url("/query")?;
        self.post_contract(url, req, None).await
    }

    /// Execute a stored read query (`POST /graphs/{graph_id}/queries/{name}`).
    pub async fn stored_query(
        &self,
        name: &str,
        req: &StoredQueryRequest,
    ) -> Result<QueryResponse, OmnigraphError> {
        let url = self.graph_url(&format!("/queries/{}", encode_segment(name)))?;
        self.post_contract(url, req, None).await
    }

    /// Execute a stored mutation (`POST /graphs/{graph_id}/queries/{name}[/if-graph-commit]`).
    pub async fn stored_mutate(
        &self,
        name: &str,
        req: &StoredQueryRequest,
        expected_commit: Option<&str>,
    ) -> Result<ChangeResponse, OmnigraphError> {
        let suffix = if expected_commit.is_some() {
            format!("/queries/{}/if-graph-commit", encode_segment(name))
        } else {
            format!("/queries/{}", encode_segment(name))
        };
        let url = self.graph_url(&suffix)?;
        self.post_contract(url, req, expected_commit).await
    }

    /// Execute an inline GQ mutation (`POST /graphs/{graph_id}/mutate`).
    pub async fn mutate(&self, req: &ChangeRequest) -> Result<ChangeResponse, OmnigraphError> {
        let url = self.graph_url("/mutate")?;
        self.post_contract(url, req, None).await
    }

    /// Execute a CAS-guarded inline GQ mutation (`POST /graphs/{graph_id}/mutate/if-graph-commit`).
    pub async fn mutate_if_commit(
        &self,
        req: &ChangeRequest,
        expected_commit: &str,
    ) -> Result<ChangeResponse, OmnigraphError> {
        if expected_commit.trim().is_empty() {
            return Err(OmnigraphError::invalid(
                "Omnigraph-If-Graph-Commit cannot be empty.",
            ));
        }
        let url = self.graph_url("/mutate/if-graph-commit")?;
        self.post_contract(url, req, Some(expected_commit)).await
    }

    /// List branches (`GET /graphs/{graph_id}/branches?detailed=...`).
    pub async fn list_branches(
        &self,
        detailed: bool,
    ) -> Result<BranchListResponse, OmnigraphError> {
        let mut url = self.graph_url("/branches")?;
        if detailed {
            url.query_pairs_mut().append_pair("detailed", "true");
        }
        self.get_contract(url).await
    }

    /// Create a branch (`POST /graphs/{graph_id}/branches`).
    pub async fn create_branch(
        &self,
        req: &BranchCreateRequest,
    ) -> Result<BranchCreateResponse, OmnigraphError> {
        let url = self.graph_url("/branches")?;
        self.post_contract(url, req, None).await
    }

    /// Delete a branch (`DELETE /graphs/{graph_id}/branches/{branch_name}`).
    pub async fn delete_branch(&self, name: &str) -> Result<BranchDeleteResponse, OmnigraphError> {
        let url = self.graph_url(&format!("/branches/{}", encode_segment(name)))?;
        let resp = self.send_raw(Method::DELETE, url, true, None, None).await?;
        Self::decode_json(resp).await
    }

    /// Compare a branch against its merge base with target (`GET /graphs/{graph_id}/branches/diff`).
    pub async fn branch_diff(&self, query: &BranchDiffQuery) -> Result<DiffOutput, OmnigraphError> {
        let mut url = self.graph_url("/branches/diff")?;
        {
            let mut qp = url.query_pairs_mut();
            qp.append_pair("source", &query.source);
            if let Some(target) = &query.target {
                qp.append_pair("target", target);
            }
            if let Some(limit) = query.limit {
                qp.append_pair("limit", &limit.to_string());
            }
            if let Some(token) = &query.page_token {
                qp.append_pair("page_token", token);
            }
        }
        self.get_contract(url).await
    }

    /// Three-way merge `source` into `target` (`POST /graphs/{graph_id}/branches/merge`).
    pub async fn merge_branches(
        &self,
        req: &BranchMergeRequest,
    ) -> Result<BranchMergeResponse, OmnigraphError> {
        let url = self.graph_url("/branches/merge")?;
        self.post_contract(url, req, None).await
    }

    /// List commits (`GET /graphs/{graph_id}/commits`).
    pub async fn list_commits(
        &self,
        query: &CommitListQuery,
    ) -> Result<CommitListResponse, OmnigraphError> {
        let mut url = self.graph_url("/commits")?;
        {
            let mut qp = url.query_pairs_mut();
            if let Some(branch) = &query.branch {
                qp.append_pair("branch", branch);
            }
            if let Some(limit) = query.limit {
                qp.append_pair("limit", &limit.to_string());
            }
            if let Some(token) = &query.page_token {
                qp.append_pair("page_token", token);
            }
        }
        self.get_contract(url).await
    }

    /// Inspect entity changes of a specific commit (`GET /graphs/{graph_id}/commits/{id}/changes`).
    pub async fn commit_changes(
        &self,
        commit_id: &str,
        limit: Option<usize>,
        page_token: Option<&str>,
    ) -> Result<CommitChangesResponse, OmnigraphError> {
        let mut url = self.graph_url(&format!("/commits/{}/changes", encode_segment(commit_id)))?;
        {
            let mut qp = url.query_pairs_mut();
            if let Some(limit) = limit {
                qp.append_pair("limit", &limit.to_string());
            }
            if let Some(token) = page_token {
                qp.append_pair("page_token", token);
            }
        }
        self.get_contract(url).await
    }

    /// Two-point diff (`GET /graphs/{graph_id}/diff?from=&to=`).
    pub async fn diff(&self, query: &DiffQuery) -> Result<DiffOutput, OmnigraphError> {
        let mut url = self.graph_url("/diff")?;
        {
            let mut qp = url.query_pairs_mut();
            qp.append_pair("from", &query.from);
            qp.append_pair("to", &query.to);
            if let Some(limit) = query.limit {
                qp.append_pair("limit", &limit.to_string());
            }
            if let Some(token) = &query.page_token {
                qp.append_pair("page_token", token);
            }
        }
        self.get_contract(url).await
    }

    /// Poll durable change feed (`GET /graphs/{graph_id}/changes`).
    pub async fn changes(
        &self,
        query: &ChangeFeedQuery,
    ) -> Result<ChangeFeedResponse, OmnigraphError> {
        let mut url = self.graph_url("/changes")?;
        {
            let mut qp = url.query_pairs_mut();
            if let Some(branch) = &query.branch {
                qp.append_pair("branch", branch);
            }
            if let Some(cursor) = &query.cursor {
                qp.append_pair("cursor", cursor);
            }
            if let Some(start) = &query.start {
                qp.append_pair("start", start);
            }
            if let Some(token) = &query.page_token {
                qp.append_pair("page_token", token);
            }
            if let Some(limit) = query.limit {
                qp.append_pair("limit", &limit.to_string());
            }
        }
        self.get_contract(url).await
    }

    /// Capture a change-feed baseline snapshot + resume cursor (`POST /graphs/{graph_id}/changes/baseline`).
    pub async fn changes_baseline(
        &self,
        req: &ChangeBaselineRequest,
    ) -> Result<ChangeBaselineResponse, OmnigraphError> {
        let url = self.graph_url("/changes/baseline")?;
        let payload = serde_json::to_vec(req)
            .map_err(|err| OmnigraphError::invalid(format!("Failed serializing body: {err}")))?;
        let resp = self
            .send_raw(Method::POST, url, true, None, Some(payload))
            .await?;
        let status = resp.status();
        let bytes = resp
            .bytes()
            .await
            .map_err(|err| OmnigraphError::transport(format!("Failed reading body: {err}")))?;
        if !status.is_success() {
            return Err(parse_error(status, &bytes));
        }
        // Omnigraph streams NDJSON lines ending with `{"baseline": {"snapshot_commit_id": ..., "resume_cursor": ...}}`,
        // or returns a single JSON object in simpler servers/mocks. Support both cleanly.
        if let Ok(single) = serde_json::from_slice::<ChangeBaselineResponse>(&bytes)
            && !single.snapshot_commit_id.is_empty()
        {
            return Ok(single);
        }
        let text = String::from_utf8_lossy(&bytes);
        let mut records = Vec::new();
        let mut terminal: Option<ChangeBaselineResponse> = None;
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let val: Value = serde_json::from_str(line).map_err(|err| {
                OmnigraphError::transport(format!("Invalid NDJSON baseline line: {err}"))
            })?;
            if let Some(baseline) = val.get("baseline") {
                let mut resp: ChangeBaselineResponse = serde_json::from_value(baseline.clone())
                    .map_err(|err| {
                        OmnigraphError::transport(format!(
                            "Invalid baseline terminal record: {err}"
                        ))
                    })?;
                if !records.is_empty() {
                    resp.records = records;
                }
                terminal = Some(resp);
                break;
            }
            records.push(val);
        }
        terminal.ok_or_else(|| {
            OmnigraphError::transport("Baseline stream ended without terminal baseline record.")
        })
    }

    /// Stream the contents of a branch as NDJSON (`POST /graphs/{graph_id}/export`).
    pub async fn export_branch(
        &self,
        branch: Option<&str>,
        type_names: &[&str],
        actor: Option<&str>,
    ) -> Result<String, OmnigraphError> {
        let url = self.graph_url("/export")?;
        let req = ExportRequest {
            branch: branch
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            type_names: type_names
                .iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
        };
        let payload = serde_json::to_vec(&req).map_err(|err| {
            OmnigraphError::invalid(format!("Failed serializing export body: {err}"))
        })?;
        let resp = self
            .send_raw_with(
                Method::POST,
                url,
                true,
                None,
                actor,
                Some("application/json"),
                Some(payload),
            )
            .await?;
        let status = resp.status();
        let bytes = resp.bytes().await.map_err(|err| {
            OmnigraphError::transport(format!("Failed reading export stream: {err}"))
        })?;
        if !status.is_success() {
            return Err(parse_error(status, &bytes));
        }
        String::from_utf8(bytes.to_vec()).map_err(|err| {
            OmnigraphError::transport(format!("Omnigraph export was not valid UTF-8: {err}"))
        })
    }

    /// Bulk-load an NDJSON graph batch onto `branch` (`POST /graphs/{graph_id}/load/ndjson`).
    pub async fn load_ndjson(
        &self,
        branch: &str,
        from: Option<&str>,
        mode: &str,
        ndjson: &str,
        actor: Option<&str>,
    ) -> Result<Value, OmnigraphError> {
        let normalized_mode = normalize_load_mode(mode)?;
        let mut url = self.graph_url("/load/ndjson")?;
        {
            let mut qp = url.query_pairs_mut();
            let trimmed_branch = branch.trim();
            if !trimmed_branch.is_empty() {
                qp.append_pair("branch", trimmed_branch);
            }
            if let Some(base) = from.map(str::trim).filter(|s| !s.is_empty()) {
                qp.append_pair("from", base);
            }
            qp.append_pair("mode", normalized_mode);
        }
        let resp = self
            .send_raw_with(
                Method::POST,
                url,
                true,
                None,
                actor,
                Some("application/x-ndjson"),
                Some(ndjson.as_bytes().to_vec()),
            )
            .await?;
        Self::decode_json(resp).await
    }

    /// Compatibility-load NDJSON data via JSON envelope (`POST /graphs/{graph_id}/load`).
    pub async fn load_json(
        &self,
        branch: &str,
        from: Option<&str>,
        mode: &str,
        ndjson: &str,
        actor: Option<&str>,
    ) -> Result<Value, OmnigraphError> {
        let normalized_mode = normalize_load_mode(mode)?;
        let url = self.graph_url("/load")?;
        let req = IngestRequest {
            branch: (!branch.trim().is_empty()).then(|| branch.trim().to_string()),
            from: from
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            mode: Some(normalized_mode.to_string()),
            data: ndjson.to_string(),
        };
        let payload = serde_json::to_vec(&req).map_err(|err| {
            OmnigraphError::invalid(format!("Failed serializing load body: {err}"))
        })?;
        let resp = self
            .send_raw_with(
                Method::POST,
                url,
                true,
                None,
                actor,
                Some("application/json"),
                Some(payload),
            )
            .await?;
        Self::decode_json(resp).await
    }
}

fn normalize_load_mode(mode: &str) -> Result<&'static str, OmnigraphError> {
    match mode.trim().to_ascii_lowercase().as_str() {
        "" | "merge" => Ok("merge"),
        "append" => Ok("append"),
        "overwrite" => Ok("overwrite"),
        other => Err(OmnigraphError::invalid(format!(
            "Unsupported Omnigraph load mode '{other}'; expected merge, append, or overwrite."
        ))),
    }
}

fn parse_error(status: StatusCode, bytes: &[u8]) -> OmnigraphError {
    if let Ok(val) = serde_json::from_slice::<Value>(bytes) {
        let code = val.get("code").and_then(Value::as_str).map(str::to_string);
        let message = val
            .get("error")
            .and_then(Value::as_str)
            .or_else(|| val.get("message").and_then(Value::as_str))
            .unwrap_or_else(|| status.canonical_reason().unwrap_or("Omnigraph error"))
            .to_string();
        let merge_conflicts = val
            .get("merge_conflicts")
            .and_then(|v| serde_json::from_value::<Vec<MergeConflict>>(v.clone()).ok())
            .unwrap_or_default();
        let precondition_failure = val
            .get("precondition_failure")
            .and_then(|v| serde_json::from_value::<PreconditionFailure>(v.clone()).ok())
            .map(Box::new);
        let change_feed_gap = val
            .get("change_feed_gap")
            .and_then(|v| serde_json::from_value::<ChangeFeedGap>(v.clone()).ok())
            .map(Box::new);
        return OmnigraphError {
            status: status.as_u16(),
            code,
            message,
            merge_conflicts,
            precondition_failure,
            change_feed_gap,
            raw_body: Some(Box::new(val)),
        };
    }
    let text = String::from_utf8_lossy(bytes).trim().to_string();
    OmnigraphError {
        status: status.as_u16(),
        code: None,
        message: if text.is_empty() {
            status
                .canonical_reason()
                .unwrap_or("Omnigraph error")
                .into()
        } else {
            text
        },
        merge_conflicts: Vec::new(),
        precondition_failure: None,
        change_feed_gap: None,
        raw_body: None,
    }
}

fn encode_segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
