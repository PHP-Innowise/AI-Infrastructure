//! Cross-service topology, contract, architecture, and run-evidence engine.
//!
//! Consumes [`crate::catalog::System`] plus optional per-service [`ServiceCodeSummary`]
//! snapshots and [`DispatchEvidence`] artifacts without depending on `infra-harness`.

use crate::{
    catalog::{Service, System, git_head},
    common::*,
    execution::{self, access_scope},
};
use infra_core::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json as object};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    time::Duration,
};

/// Cross-service reference extracted or inferred from a member service's code graph or sources.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeReference {
    pub from_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_symbol: Option<String>,
    pub target_service: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<String>,
    #[serde(default = "default_code_kind")]
    pub kind: String,
    #[serde(default = "default_extracted")]
    pub certainty: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

fn default_code_kind() -> String {
    "code".into()
}

fn default_extracted() -> String {
    "EXTRACTED".into()
}

fn default_unindexed() -> String {
    "unindexed".into()
}

/// Changed file entry scoped to a single service root.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFileEntry {
    pub path: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additions: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletions: Option<usize>,
    #[serde(default)]
    pub is_test: bool,
}

/// Changed symbol/declaration entry scoped to a single service root.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedEntityEntry {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub change: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default)]
    pub visibility_narrowed: bool,
}

/// Intra-service architectural layer violation summary.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerViolationEntry {
    pub from_path: String,
    pub to_path: String,
    pub from_layer: String,
    pub to_layer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Optional per-service code-graph and diff summary supplied to [`SystemGraph`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ServiceCodeSummary {
    #[serde(default = "default_unindexed")]
    pub state: String,
    #[serde(default)]
    pub nodes: usize,
    #[serde(default)]
    pub edges: usize,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub stale: bool,
    #[serde(default)]
    pub dirty: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<String>,
    #[serde(default)]
    pub code_refs: Vec<CodeReference>,
    #[serde(default)]
    pub changed_files: Vec<ChangedFileEntry>,
    #[serde(default)]
    pub changed_entities: Vec<ChangedEntityEntry>,
    #[serde(default)]
    pub opened_files: BTreeSet<String>,
    #[serde(default)]
    pub shell_reads: BTreeMap<String, i64>,
    #[serde(default)]
    pub layer_violations: Vec<LayerViolationEntry>,
    /// Optional explicit consumer file map keyed by `"provider:contract"` or `"contract"`.
    #[serde(default)]
    pub consumer_files: BTreeMap<String, Vec<String>>,
}

/// Offered Graphify code-graph access for a worker dispatch.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceGraphOffer {
    pub service: String,
    pub command: String,
    pub graph_path: PathBuf,
    #[serde(default)]
    pub nodes: usize,
    #[serde(default)]
    pub edges: usize,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub readable_paths: Vec<PathBuf>,
    #[serde(default)]
    pub claude_allow_rules: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_instructions: Option<String>,
}

/// File change entry stored inside [`DispatchEvidence`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceFileChange {
    pub path: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

/// Durable per-dispatch run evidence persisted at
/// `<state>/ai-system/<rid>/execution/<phase>-a<attempt>.evidence.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DispatchEvidence {
    pub phase: String,
    pub attempt: u64,
    #[serde(default)]
    pub launch_id: Option<String>,
    pub service: String,
    pub root: String,
    #[serde(default)]
    pub base_head: Option<String>,
    #[serde(default)]
    pub head_after: Option<String>,
    #[serde(default)]
    pub git_changed: Vec<EvidenceFileChange>,
    #[serde(default)]
    pub ledger_edited: Vec<EvidenceFileChange>,
    #[serde(default)]
    pub opened: Vec<String>,
    #[serde(default)]
    pub shell_reads: BTreeMap<String, i64>,
    #[serde(default)]
    pub shell_events: Vec<Value>,
    #[serde(default)]
    pub graph_asked: bool,
    #[serde(default)]
    pub steps: usize,
    #[serde(default)]
    pub searched: usize,
    #[serde(default)]
    pub commands: usize,
    #[serde(default)]
    pub helpers: usize,
    #[serde(default)]
    pub limited: bool,
    #[serde(default)]
    pub created_entities: Vec<Value>,
    #[serde(default)]
    pub target_folders: Vec<String>,
    #[serde(default)]
    pub target_paths: Vec<String>,
    #[serde(default)]
    pub target_nodes: Vec<String>,
    #[serde(default)]
    pub file_touches: Vec<Value>,
    #[serde(default)]
    pub plan_items: Vec<Value>,
    #[serde(default)]
    pub command_runs: Vec<Value>,
}

/// Context passed to [`CodeGraphBridge`] hooks for each sequential system dispatch.
#[derive(Clone, Debug)]
pub struct DispatchGraphContext {
    /// Outer system run ID (`<rid>` under `<state>/ai-system/<rid>`) when launched via `SystemManager`,
    /// or `journal_run_id` during standalone CLI execution.
    pub run_id: String,
    /// Journal `run_id` inside `execution/run.json`.
    pub journal_run_id: String,
    pub launch_id: Option<String>,
    pub execution_dir: PathBuf,
    pub system_file: PathBuf,
    pub provider: String,
    pub mode: String,
    pub access: String,
    pub tell_agents_graph: Option<bool>,
    pub phase: String,
    pub dispatch_id: String,
    pub attempt: u64,
    pub service: String,
    pub root: PathBuf,
    pub roots: BTreeMap<String, PathBuf>,
    pub selected: Vec<String>,
    pub readable: Vec<String>,
    pub writable: Vec<String>,
    pub base_head: Option<String>,
    pub head_after: Option<String>,
    pub plan: Value,
    pub state: Value,
}

/// Bridge implemented by `infra-harness` to connect member-service code graphs,
/// live focus tracking, and tool-level `RunLedger` capture into `infra-system` runs.
pub trait CodeGraphBridge: Send + Sync {
    fn on_run_start(&self, _run_id: &str, _state: &Value, _system: &System) -> Result<()> {
        Ok(())
    }

    fn offers_for_dispatch(&self, _ctx: &DispatchGraphContext) -> Result<Vec<ServiceGraphOffer>> {
        Ok(vec![])
    }

    fn on_dispatch_start(&self, _ctx: &DispatchGraphContext) -> Result<()> {
        Ok(())
    }

    fn on_dispatch_signal(&self, _ctx: &DispatchGraphContext, _raw_line: &[u8]) -> Result<()> {
        Ok(())
    }

    fn on_dispatch_finish(
        &self,
        _ctx: &DispatchGraphContext,
        _receipt: &Value,
    ) -> Result<Option<DispatchEvidence>> {
        Ok(None)
    }

    fn on_run_finish(&self, _run_id: &str, _state: &Value, _system: &System) -> Result<()> {
        Ok(())
    }

    /// Adds to the view of a run what the bridge knows of it. The view
    /// comes without a summary of its evidence: a bridge that sums the
    /// evidence up puts it in `view["evidence"]`, and where it leaves none
    /// the manager sums the dispatches up itself.
    fn enrich_run_view(
        &self,
        _run_id: &str,
        _system: Option<&System>,
        _state: Option<&Value>,
        _dispatches: &[DispatchEvidence],
        _view: &mut Value,
    ) -> Result<()> {
        Ok(())
    }
}

/// Default no-op bridge used by standalone CLI commands and unit tests.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopCodeGraphBridge;

impl CodeGraphBridge for NoopCodeGraphBridge {}

/// Declared service-tier rule loaded from `system-architecture.json` or `.harness-architecture.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectureLayerRule {
    pub name: String,
    #[serde(default)]
    pub services: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub allows: Option<Vec<String>>,
}

/// Declared or inferred cross-service architectural layer configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemArchitectureRules {
    pub declared: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
    #[serde(default)]
    pub layers: Vec<ArchitectureLayerRule>,
}

/// Query parameters for [`SystemGraph::query`] and [`SystemGraph::view`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemMapQuery {
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub focus: Option<String>,
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub spread: Vec<String>,
    #[serde(default)]
    pub whole: bool,
    #[serde(default)]
    pub certain: bool,
    #[serde(default)]
    pub follow: Option<String>,
    #[serde(default)]
    pub out: usize,
    #[serde(default)]
    pub into: usize,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub path: bool,
    #[serde(default)]
    pub shared: bool,
    #[serde(default)]
    pub side: Option<String>,
    #[serde(default)]
    pub skip: usize,
    #[serde(default)]
    pub cycle: bool,
    #[serde(default)]
    pub layers: bool,
    #[serde(default)]
    pub seed_services: Vec<String>,
    #[serde(default)]
    pub contracts: Vec<String>,
    #[serde(default)]
    pub depth: Option<usize>,
}

/// Query parameters for [`SystemGraph::impact_across_services`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossServiceImpactQuery {
    #[serde(default)]
    pub service: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub contract: Option<String>,
    #[serde(default)]
    pub certain: bool,
    /// Optional map of service_id (plus `"verify"` / `"__system__"`) -> opened relative paths.
    #[serde(default)]
    pub opened_by_service: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContractLinkDetail {
    pub id: String,
    pub kind: String,
    pub consumer_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_version: Option<String>,
    pub provenance: String,
    pub certainty: String,
    pub broken_risk: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_reason: Option<String>,
    #[serde(default)]
    pub provider_sources: Vec<String>,
    #[serde(default)]
    pub consumer_sources: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub service: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(default)]
    pub dirty: bool,
    #[serde(default)]
    pub relationships_complete: bool,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub capability_count: usize,
    #[serde(default)]
    pub provides_count: usize,
    #[serde(default)]
    pub consumes_count: usize,
    #[serde(default)]
    pub source_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub code_graph: Value,
    #[serde(default)]
    pub in_count: usize,
    #[serde(default)]
    pub out_count: usize,
    #[serde(default)]
    pub changed_count: usize,
    #[serde(default)]
    pub opened_count: usize,
    #[serde(default)]
    pub shell_read: bool,
    #[serde(default)]
    pub broken_risk: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<usize>,
    #[serde(default)]
    pub expandable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemEdge {
    /// Source of dependency (`consumer` for service/contract edges).
    pub from: String,
    /// Target of dependency (`provider` for service/contract edges).
    pub to: String,
    pub consumer: String,
    pub provider: String,
    pub kinds: Vec<String>,
    pub contracts: Vec<ContractLinkDetail>,
    pub provenance: String,
    pub certainty: String,
    pub broken_risk: bool,
    #[serde(default)]
    pub risk_reasons: Vec<String>,
    pub violation: bool,
    pub against: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub violation_reason: Option<String>,
    pub count: usize,
    /// Whether this edge connects top-level services (`"service"`), shared system docs (`"shared"`),
    /// or expanded child items (`"detail"`).
    pub level: String,
}

/// Rich queryable cross-service topology, contract, and architecture graph.
#[derive(Clone, Debug)]
pub struct SystemGraph {
    pub system_name: String,
    pub config: PathBuf,
    pub root: PathBuf,
    pub nodes: BTreeMap<String, SystemNode>,
    pub service_order: Vec<String>,
    pub edges: Vec<SystemEdge>,
    pub warnings: Vec<Value>,
    pub architecture: SystemArchitectureRules,
    pub summaries: BTreeMap<String, ServiceCodeSummary>,
    manifests: BTreeMap<String, Value>,
    shared_sources: Vec<Value>,
}

impl SystemGraph {
    /// Builds a [`SystemGraph`] from a loaded [`System`] without external code-graph summaries.
    pub fn simple(system: &System) -> Self {
        Self::from_system(system, BTreeMap::new())
    }

    /// Builds a [`SystemGraph`] combining declared system/service passports with optional
    /// per-service [`ServiceCodeSummary`] snapshots.
    pub fn from_system(
        system: &System,
        mut summaries: BTreeMap<String, ServiceCodeSummary>,
    ) -> Self {
        let system_name = string(&system.config_data["name"]).to_string();
        let shared_sources = system.config_data["shared_sources"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let architecture = load_architecture_rules(&system.root, &system.services);

        let mut nodes = BTreeMap::new();
        let mut manifests = BTreeMap::new();
        let service_order = system.services.keys().cloned().collect::<Vec<_>>();

        // 1. Create `__system__` node and its shared source child nodes.
        let sys_summary = summaries.entry("__system__".into()).or_default().clone();
        let sys_head = git_head(&system.root);
        let sys_node = SystemNode {
            id: "__system__".into(),
            label: if system_name.is_empty() {
                "__system__".into()
            } else {
                format!("{system_name} (__system__)")
            },
            kind: "system".into(),
            service: "__system__".into(),
            parent: None,
            root: Some(system.root.to_string_lossy().into_owned()),
            access: Some("available".into()),
            stack: Some(sys_summary.stack.clone().unwrap_or_else(|| "system".into())),
            head: sys_head,
            dirty: sys_summary.dirty || !sys_summary.changed_files.is_empty(),
            relationships_complete: true,
            owner: "system".into(),
            description: "Shared system governance, architecture, and cross-service contracts"
                .into(),
            capability_count: 0,
            provides_count: 0,
            consumes_count: 0,
            source_count: shared_sources.len(),
            contract_kind: None,
            version: None,
            status: None,
            source_kind: None,
            path: None,
            sources: shared_sources
                .iter()
                .map(|entry| string(&entry["path"]).to_string())
                .collect(),
            keywords: vec![],
            code_graph: object!({
                "state": sys_summary.state,
                "nodes": sys_summary.nodes,
                "edges": sys_summary.edges,
                "commit": sys_summary.commit,
                "stale": sys_summary.stale,
                "layer_violations_count": sys_summary.layer_violations.len(),
            }),
            in_count: 0,
            out_count: 0,
            changed_count: sys_summary.changed_files.len(),
            opened_count: sys_summary.opened_files.len(),
            shell_read: !sys_summary.shell_reads.is_empty(),
            broken_risk: false,
            layer: None,
            tier: None,
            expandable: !shared_sources.is_empty(),
        };
        nodes.insert("__system__".into(), sys_node);

        for entry in &shared_sources {
            let rel_path = string(&entry["path"]).to_string();
            if rel_path.is_empty() {
                continue;
            }
            let src_id = format!("source:__system__:{rel_path}");
            let is_changed = sys_summary
                .changed_files
                .iter()
                .any(|item| item.path == rel_path);
            let is_opened = sys_summary.opened_files.contains(&rel_path);
            let is_shell = sys_summary.shell_reads.contains_key(&rel_path);
            nodes.insert(
                src_id.clone(),
                SystemNode {
                    id: src_id,
                    label: rel_path.clone(),
                    kind: "source".into(),
                    service: "__system__".into(),
                    parent: Some("__system__".into()),
                    root: None,
                    access: Some("available".into()),
                    stack: None,
                    head: None,
                    dirty: is_changed,
                    relationships_complete: true,
                    owner: String::new(),
                    description: format!("Shared {} source", string(&entry["kind"])),
                    capability_count: 0,
                    provides_count: 0,
                    consumes_count: 0,
                    source_count: 0,
                    contract_kind: None,
                    version: None,
                    status: None,
                    source_kind: Some(string(&entry["kind"]).to_string()),
                    path: Some(rel_path),
                    sources: vec![],
                    keywords: vec![],
                    code_graph: Value::Null,
                    in_count: 0,
                    out_count: 0,
                    changed_count: usize::from(is_changed),
                    opened_count: usize::from(is_opened),
                    shell_read: is_shell,
                    broken_risk: false,
                    layer: None,
                    tier: None,
                    expandable: false,
                },
            );
        }

        // 2. Create member `service` nodes and their expandable child nodes.
        for (sid, service) in &system.services {
            let summary = summaries.entry(sid.clone()).or_default().clone();
            let stack = summary
                .stack
                .clone()
                .unwrap_or_else(|| detect_service_stack(&service.root));
            let manifest = service.manifest.clone();
            if let Some(passport) = &manifest {
                manifests.insert(sid.clone(), passport.clone());
            }
            let capabilities = manifest
                .as_ref()
                .and_then(|m| m["capabilities"].as_array())
                .cloned()
                .unwrap_or_default();
            let provides = manifest
                .as_ref()
                .and_then(|m| m["provides"].as_array())
                .cloned()
                .unwrap_or_default();
            let consumes = manifest
                .as_ref()
                .and_then(|m| m["consumes"].as_array())
                .cloned()
                .unwrap_or_default();
            let declared_sources = manifest
                .as_ref()
                .and_then(|m| m["sources"].as_array())
                .cloned()
                .unwrap_or_default();

            let mut all_source_kinds = BTreeMap::<String, String>::new();
            for entry in &declared_sources {
                let p = string(&entry["path"]).to_string();
                if !p.is_empty() {
                    all_source_kinds.insert(p, string(&entry["kind"]).to_string());
                }
            }
            for cap in &capabilities {
                for src in texts(&cap["sources"]) {
                    all_source_kinds
                        .entry(src)
                        .or_insert_with(|| "code".to_string());
                }
            }
            for prov in &provides {
                for src in texts(&prov["sources"]) {
                    all_source_kinds
                        .entry(src)
                        .or_insert_with(|| "contract".to_string());
                }
            }

            let has_warning_risk = system.warnings.iter().any(|warning| {
                warning["service"] == sid.as_str()
                    && matches!(
                        string(&warning["reason"]),
                        "provider_contract_unavailable" | "contract_version_mismatch"
                    )
            });

            let expandable =
                !capabilities.is_empty() || !provides.is_empty() || !all_source_kinds.is_empty();

            nodes.insert(
                sid.clone(),
                SystemNode {
                    id: sid.clone(),
                    label: sid.clone(),
                    kind: "service".into(),
                    service: sid.clone(),
                    parent: None,
                    root: Some(service.root.to_string_lossy().into_owned()),
                    access: Some(service.access.clone()),
                    stack: Some(stack),
                    head: service.head.clone(),
                    dirty: summary.dirty || !summary.changed_files.is_empty(),
                    relationships_complete: manifest
                        .as_ref()
                        .and_then(|m| m["relationships_complete"].as_bool())
                        .unwrap_or(false),
                    owner: manifest
                        .as_ref()
                        .map(|m| string(&m["owner"]).to_string())
                        .unwrap_or_default(),
                    description: manifest
                        .as_ref()
                        .map(|m| string(&m["description"]).to_string())
                        .unwrap_or_default(),
                    capability_count: capabilities.len(),
                    provides_count: provides.len(),
                    consumes_count: consumes.len(),
                    source_count: all_source_kinds.len(),
                    contract_kind: None,
                    version: None,
                    status: None,
                    source_kind: None,
                    path: None,
                    sources: all_source_kinds.keys().cloned().collect(),
                    keywords: vec![],
                    code_graph: object!({
                        "state": summary.state,
                        "nodes": summary.nodes,
                        "edges": summary.edges,
                        "commit": summary.commit,
                        "stale": summary.stale,
                        "layer_violations_count": summary.layer_violations.len(),
                    }),
                    in_count: 0,
                    out_count: 0,
                    changed_count: summary.changed_files.len(),
                    opened_count: summary.opened_files.len(),
                    shell_read: !summary.shell_reads.is_empty(),
                    broken_risk: has_warning_risk,
                    layer: None,
                    tier: None,
                    expandable,
                },
            );

            for cap in &capabilities {
                let cid = string(&cap["id"]).to_string();
                let node_id = format!("cap:{sid}:{cid}");
                let cap_sources = texts(&cap["sources"]);
                let changed_cnt = cap_sources
                    .iter()
                    .filter(|p| summary.changed_files.iter().any(|cf| &cf.path == *p))
                    .count();
                let opened_cnt = cap_sources
                    .iter()
                    .filter(|p| summary.opened_files.contains(*p))
                    .count();
                nodes.insert(
                    node_id.clone(),
                    SystemNode {
                        id: node_id,
                        label: cid,
                        kind: "capability".into(),
                        service: sid.clone(),
                        parent: Some(sid.clone()),
                        root: None,
                        access: Some(service.access.clone()),
                        stack: None,
                        head: None,
                        dirty: changed_cnt > 0,
                        relationships_complete: true,
                        owner: String::new(),
                        description: string(&cap["description"]).to_string(),
                        capability_count: 0,
                        provides_count: 0,
                        consumes_count: 0,
                        source_count: cap_sources.len(),
                        contract_kind: None,
                        version: None,
                        status: Some(string(&cap["status"]).to_string()),
                        source_kind: None,
                        path: None,
                        sources: cap_sources,
                        keywords: texts(&cap["keywords"]),
                        code_graph: Value::Null,
                        in_count: 0,
                        out_count: 0,
                        changed_count: changed_cnt,
                        opened_count: opened_cnt,
                        shell_read: false,
                        broken_risk: false,
                        layer: None,
                        tier: None,
                        expandable: false,
                    },
                );
            }

            for prov in &provides {
                let cid = string(&prov["id"]).to_string();
                let ckind = string(&prov["kind"]).to_string();
                let cver = string(&prov["version"]).to_string();
                let prov_sources = texts(&prov["sources"]);
                let node_id = format!("contract:{sid}:{cid}");
                let changed_cnt = prov_sources
                    .iter()
                    .filter(|p| summary.changed_files.iter().any(|cf| &cf.path == *p))
                    .count();
                let opened_cnt = prov_sources
                    .iter()
                    .filter(|p| summary.opened_files.contains(*p))
                    .count();
                nodes.insert(
                    node_id.clone(),
                    SystemNode {
                        id: node_id,
                        label: format!("{cid} ({ckind} {cver})"),
                        kind: "contract".into(),
                        service: sid.clone(),
                        parent: Some(sid.clone()),
                        root: None,
                        access: Some(service.access.clone()),
                        stack: None,
                        head: None,
                        dirty: changed_cnt > 0,
                        relationships_complete: true,
                        owner: String::new(),
                        description: format!("{ckind} contract {cid}@{cver} provided by {sid}"),
                        capability_count: 0,
                        provides_count: 0,
                        consumes_count: 0,
                        source_count: prov_sources.len(),
                        contract_kind: Some(ckind),
                        version: Some(cver),
                        status: None,
                        source_kind: None,
                        path: None,
                        sources: prov_sources,
                        keywords: vec![],
                        code_graph: Value::Null,
                        in_count: 0,
                        out_count: 0,
                        changed_count: changed_cnt,
                        opened_count: opened_cnt,
                        shell_read: false,
                        broken_risk: false,
                        layer: None,
                        tier: None,
                        expandable: false,
                    },
                );
            }

            for (rel_path, skind) in &all_source_kinds {
                let node_id = format!("source:{sid}:{rel_path}");
                let is_changed = summary.changed_files.iter().any(|cf| &cf.path == rel_path);
                let is_opened = summary.opened_files.contains(rel_path);
                let is_shell = summary.shell_reads.contains_key(rel_path);
                nodes.insert(
                    node_id.clone(),
                    SystemNode {
                        id: node_id,
                        label: rel_path.clone(),
                        kind: "source".into(),
                        service: sid.clone(),
                        parent: Some(sid.clone()),
                        root: None,
                        access: Some(service.access.clone()),
                        stack: None,
                        head: None,
                        dirty: is_changed,
                        relationships_complete: true,
                        owner: String::new(),
                        description: format!("{skind} source in {sid}"),
                        capability_count: 0,
                        provides_count: 0,
                        consumes_count: 0,
                        source_count: 0,
                        contract_kind: None,
                        version: None,
                        status: None,
                        source_kind: Some(skind.clone()),
                        path: Some(rel_path.clone()),
                        sources: vec![],
                        keywords: vec![],
                        code_graph: Value::Null,
                        in_count: 0,
                        out_count: 0,
                        changed_count: usize::from(is_changed),
                        opened_count: usize::from(is_opened),
                        shell_read: is_shell,
                        broken_risk: false,
                        layer: None,
                        tier: None,
                        expandable: false,
                    },
                );
            }
        }

        // 3. Build cross-service edges (declared + verified + undeclared_in_code), shared edges, and detail edges.
        let mut pair_contracts: BTreeMap<(String, String), Vec<ContractLinkDetail>> =
            BTreeMap::new();
        let mut detail_edges = Vec::<SystemEdge>::new();

        for (consumer_id, consumer_service) in &system.services {
            let consumer_summary = summaries.get(consumer_id);
            if let Some(passport) = &consumer_service.manifest {
                for consume in passport["consumes"].as_array().into_iter().flatten() {
                    let provider_id = string(&consume["service"]).to_string();
                    let cid = string(&consume["contract"]).to_string();
                    let consumer_ver = string(&consume["version"]).to_string();
                    let provider_contract = system
                        .services
                        .get(&provider_id)
                        .and_then(|svc| svc.manifest.as_ref())
                        .and_then(|m| m["provides"].as_array())
                        .and_then(|arr| arr.iter().find(|entry| entry["id"] == cid));

                    let (ckind, provider_ver, provider_sources, risk_reason) =
                        match provider_contract {
                            Some(prov) => {
                                let pver = string(&prov["version"]).to_string();
                                let reason = (pver != consumer_ver)
                                    .then(|| "contract_version_mismatch".to_string());
                                (
                                    string(&prov["kind"]).to_string(),
                                    Some(pver),
                                    texts(&prov["sources"]),
                                    reason,
                                )
                            }
                            None => (
                                "other".into(),
                                None,
                                vec![],
                                Some("provider_contract_unavailable".into()),
                            ),
                        };

                    let matching_refs = consumer_summary
                        .map(|summary| {
                            summary
                                .code_refs
                                .iter()
                                .filter(|r| {
                                    r.target_service == provider_id
                                        && (r.contract.as_deref() == Some(cid.as_str())
                                            || r.contract.is_none())
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();

                    let provenance = if !matching_refs.is_empty() {
                        "verified".to_string()
                    } else {
                        "declared".to_string()
                    };

                    let consumer_sources = resolve_consumer_sources_for_contract(
                        passport,
                        consumer_summary,
                        &provider_id,
                        &cid,
                    );

                    let broken_risk = risk_reason.is_some();
                    let detail = ContractLinkDetail {
                        id: cid.clone(),
                        kind: ckind.clone(),
                        consumer_version: consumer_ver,
                        provider_version: provider_ver,
                        provenance: provenance.clone(),
                        certainty: "EXTRACTED".into(),
                        broken_risk,
                        risk_reason: risk_reason.clone(),
                        provider_sources: provider_sources.clone(),
                        consumer_sources: consumer_sources.clone(),
                    };
                    pair_contracts
                        .entry((consumer_id.clone(), provider_id.clone()))
                        .or_default()
                        .push(detail.clone());

                    let contract_node_id = format!("contract:{provider_id}:{cid}");
                    if nodes.contains_key(&contract_node_id) {
                        detail_edges.push(SystemEdge {
                            from: consumer_id.clone(),
                            to: contract_node_id.clone(),
                            consumer: consumer_id.clone(),
                            provider: provider_id.clone(),
                            kinds: vec![ckind.clone()],
                            contracts: vec![detail],
                            provenance,
                            certainty: "EXTRACTED".into(),
                            broken_risk,
                            risk_reasons: risk_reason.into_iter().collect(),
                            violation: false,
                            against: false,
                            violation_reason: None,
                            count: 1,
                            level: "detail".into(),
                        });
                    }
                }
            }

            // Undeclared cross-service references discovered in code (`code_refs`).
            if let Some(summary) = consumer_summary {
                let declared_set: BTreeSet<(String, String)> = consumer_service
                    .manifest
                    .as_ref()
                    .and_then(|m| m["consumes"].as_array())
                    .into_iter()
                    .flatten()
                    .map(|c| {
                        (
                            string(&c["service"]).to_string(),
                            string(&c["contract"]).to_string(),
                        )
                    })
                    .collect();
                let declared_providers: BTreeSet<String> =
                    declared_set.iter().map(|(p, _)| p.clone()).collect();

                for code_ref in &summary.code_refs {
                    let target = &code_ref.target_service;
                    if target == consumer_id || !system.services.contains_key(target) {
                        continue;
                    }
                    let is_declared = match &code_ref.contract {
                        Some(cid) => declared_set.contains(&(target.clone(), cid.clone())),
                        None => declared_providers.contains(target),
                    };
                    if is_declared {
                        continue;
                    }
                    let cid = code_ref
                        .contract
                        .clone()
                        .unwrap_or_else(|| format!("undeclared:{}", code_ref.from_path));
                    let kind = if ["http", "event", "rpc", "graphql", "other", "shared", "code"]
                        .contains(&code_ref.kind.as_str())
                    {
                        code_ref.kind.clone()
                    } else {
                        "code".into()
                    };
                    let entries = pair_contracts
                        .entry((consumer_id.clone(), target.clone()))
                        .or_default();
                    if let Some(existing) = entries
                        .iter_mut()
                        .find(|e| e.id == cid && e.provenance == "undeclared_in_code")
                    {
                        if !existing.consumer_sources.contains(&code_ref.from_path) {
                            existing.consumer_sources.push(code_ref.from_path.clone());
                        }
                    } else {
                        entries.push(ContractLinkDetail {
                            id: cid,
                            kind,
                            consumer_version: "undeclared".into(),
                            provider_version: None,
                            provenance: "undeclared_in_code".into(),
                            certainty: if code_ref.certainty.is_empty() {
                                "INFERRED".into()
                            } else {
                                code_ref.certainty.clone()
                            },
                            broken_risk: true,
                            risk_reason: Some("undeclared_in_code".into()),
                            provider_sources: vec![],
                            consumer_sources: vec![code_ref.from_path.clone()],
                        });
                    }
                }
            }
        }

        // Detail edges inside each service: contract -> provider, capability -> source, contract -> source.
        for (sid, passport) in &manifests {
            for prov in passport["provides"].as_array().into_iter().flatten() {
                let cid = string(&prov["id"]);
                let ckind = string(&prov["kind"]).to_string();
                let contract_node = format!("contract:{sid}:{cid}");
                if nodes.contains_key(&contract_node) {
                    detail_edges.push(SystemEdge {
                        from: contract_node.clone(),
                        to: sid.clone(),
                        consumer: contract_node.clone(),
                        provider: sid.clone(),
                        kinds: vec![ckind],
                        contracts: vec![],
                        provenance: "declared".into(),
                        certainty: "EXTRACTED".into(),
                        broken_risk: false,
                        risk_reasons: vec![],
                        violation: false,
                        against: false,
                        violation_reason: None,
                        count: 1,
                        level: "detail".into(),
                    });
                    for src in texts(&prov["sources"]) {
                        let src_node = format!("source:{sid}:{src}");
                        if nodes.contains_key(&src_node) {
                            detail_edges.push(SystemEdge {
                                from: contract_node.clone(),
                                to: src_node,
                                consumer: contract_node.clone(),
                                provider: sid.clone(),
                                kinds: vec!["code".into()],
                                contracts: vec![],
                                provenance: "declared".into(),
                                certainty: "EXTRACTED".into(),
                                broken_risk: false,
                                risk_reasons: vec![],
                                violation: false,
                                against: false,
                                violation_reason: None,
                                count: 1,
                                level: "detail".into(),
                            });
                        }
                    }
                }
            }
            for cap in passport["capabilities"].as_array().into_iter().flatten() {
                let cap_id = string(&cap["id"]);
                let cap_node = format!("cap:{sid}:{cap_id}");
                if nodes.contains_key(&cap_node) {
                    detail_edges.push(SystemEdge {
                        from: sid.clone(),
                        to: cap_node.clone(),
                        consumer: sid.clone(),
                        provider: sid.clone(),
                        kinds: vec!["other".into()],
                        contracts: vec![],
                        provenance: "declared".into(),
                        certainty: "EXTRACTED".into(),
                        broken_risk: false,
                        risk_reasons: vec![],
                        violation: false,
                        against: false,
                        violation_reason: None,
                        count: 1,
                        level: "detail".into(),
                    });
                    for src in texts(&cap["sources"]) {
                        let src_node = format!("source:{sid}:{src}");
                        if nodes.contains_key(&src_node) {
                            detail_edges.push(SystemEdge {
                                from: cap_node.clone(),
                                to: src_node,
                                consumer: sid.clone(),
                                provider: sid.clone(),
                                kinds: vec!["code".into()],
                                contracts: vec![],
                                provenance: "declared".into(),
                                certainty: "EXTRACTED".into(),
                                broken_risk: false,
                                risk_reasons: vec![],
                                violation: false,
                                against: false,
                                violation_reason: None,
                                count: 1,
                                level: "detail".into(),
                            });
                        }
                    }
                }
            }
        }

        let mut edges = Vec::new();
        for ((consumer, provider), contracts) in pair_contracts {
            let mut kinds_set = BTreeSet::new();
            let mut risk_reasons = BTreeSet::new();
            let mut has_verified = false;
            let mut has_declared = false;
            let mut has_undeclared = false;
            let mut has_extracted = false;
            for item in &contracts {
                kinds_set.insert(item.kind.clone());
                if item.provenance == "undeclared_in_code" {
                    kinds_set.insert("code".into());
                    has_undeclared = true;
                } else if item.provenance == "verified" {
                    has_verified = true;
                } else {
                    has_declared = true;
                }
                if item.certainty == "EXTRACTED" {
                    has_extracted = true;
                }
                if let Some(reason) = &item.risk_reason {
                    risk_reasons.insert(reason.clone());
                }
            }
            let provenance = if has_undeclared && !has_verified && !has_declared {
                "undeclared_in_code"
            } else if has_undeclared {
                // Surface drift prominently whenever undeclared code calls accompany or bypass passports.
                "undeclared_in_code"
            } else if has_verified {
                "verified"
            } else {
                "declared"
            }
            .to_string();
            let broken_risk = !risk_reasons.is_empty();
            if broken_risk && let Some(node) = nodes.get_mut(&consumer) {
                node.broken_risk = true;
            }
            edges.push(SystemEdge {
                from: consumer.clone(),
                to: provider.clone(),
                consumer,
                provider,
                kinds: kinds_set.into_iter().collect(),
                count: contracts.len(),
                contracts,
                provenance,
                certainty: if has_extracted {
                    "EXTRACTED".into()
                } else {
                    "INFERRED".into()
                },
                broken_risk,
                risk_reasons: risk_reasons.into_iter().collect(),
                violation: false,
                against: false,
                violation_reason: None,
                level: "service".into(),
            });
        }

        // Shared governance edges to `__system__` when `shared_sources` is non-empty.
        if !shared_sources.is_empty() {
            for sid in &service_order {
                edges.push(SystemEdge {
                    from: sid.clone(),
                    to: "__system__".into(),
                    consumer: sid.clone(),
                    provider: "__system__".into(),
                    kinds: vec!["shared".into()],
                    contracts: vec![],
                    provenance: "declared".into(),
                    certainty: "EXTRACTED".into(),
                    broken_risk: false,
                    risk_reasons: vec![],
                    violation: false,
                    against: false,
                    violation_reason: None,
                    count: shared_sources.len(),
                    level: "shared".into(),
                });
            }
        }

        edges.extend(detail_edges);

        // Compute in_count / out_count on service nodes from service-level edges.
        let mut in_counts = BTreeMap::<String, usize>::new();
        let mut out_counts = BTreeMap::<String, usize>::new();
        for edge in edges.iter().filter(|edge| edge.level == "service") {
            *out_counts.entry(edge.from.clone()).or_default() += 1;
            *in_counts.entry(edge.to.clone()).or_default() += 1;
        }
        for (id, node) in &mut nodes {
            if node.kind == "service" {
                node.in_count = in_counts.get(id).copied().unwrap_or(0);
                node.out_count = out_counts.get(id).copied().unwrap_or(0);
            }
        }

        let mut graph = Self {
            system_name,
            config: system.config.clone(),
            root: system.root.clone(),
            nodes,
            service_order,
            edges,
            warnings: system.warnings.clone(),
            architecture,
            summaries,
            manifests,
            shared_sources,
        };
        graph.apply_layers_and_violations();
        graph
    }

    fn apply_layers_and_violations(&mut self) {
        let mut tier_map = BTreeMap::<String, (String, usize)>::new();
        let mut allows_map = BTreeMap::<String, Option<BTreeSet<String>>>::new();

        if self.architecture.declared && !self.architecture.layers.is_empty() {
            for (idx, rule) in self.architecture.layers.iter().enumerate() {
                allows_map.insert(
                    rule.name.clone(),
                    rule.allows
                        .as_ref()
                        .map(|items| items.iter().cloned().collect()),
                );
                for sid in &self.service_order {
                    if tier_map.contains_key(sid) {
                        continue;
                    }
                    let svc_root = self
                        .nodes
                        .get(sid)
                        .and_then(|n| n.root.as_deref())
                        .unwrap_or("");
                    let matched = rule.services.iter().any(|s| s == sid)
                        || rule.paths.iter().any(|pattern| {
                            path_pattern_matches(pattern, sid, svc_root, &self.root)
                        });
                    if matched {
                        tier_map.insert(sid.clone(), (rule.name.clone(), idx));
                    }
                }
            }
        } else {
            let inferred = self.infer_service_tiers();
            for (sid, (name, idx)) in inferred {
                tier_map.insert(sid, (name, idx));
            }
        }

        for (sid, (layer_name, tier_idx)) in &tier_map {
            if let Some(node) = self.nodes.get_mut(sid) {
                node.layer = Some(layer_name.clone());
                node.tier = Some(*tier_idx);
            }
        }

        let cycle_pairs: BTreeSet<(String, String)> = self
            .cycles(false)
            .into_iter()
            .flat_map(|cycle| {
                cycle
                    .windows(2)
                    .map(|pair| (pair[0].clone(), pair[1].clone()))
                    .collect::<Vec<_>>()
            })
            .collect();

        for edge in &mut self.edges {
            if edge.level != "service" {
                continue;
            }
            if self.architecture.declared
                && let (Some((from_layer, from_tier)), Some((to_layer, to_tier))) =
                    (tier_map.get(&edge.from), tier_map.get(&edge.to))
                && from_layer != to_layer
            {
                if let Some(Some(allowed)) = allows_map.get(from_layer) {
                    if !allowed.contains(to_layer) {
                        edge.violation = true;
                        edge.against = from_tier > to_tier;
                        edge.violation_reason = Some(format!(
                            "layer {from_layer} is not allowed to depend on {to_layer}"
                        ));
                        continue;
                    }
                } else if from_tier > to_tier {
                    edge.violation = true;
                    edge.against = true;
                    edge.violation_reason = Some(format!(
                        "upward dependency from lower tier {from_layer} to higher tier {to_layer}"
                    ));
                    continue;
                }
            }
            if cycle_pairs.contains(&(edge.from.clone(), edge.to.clone())) {
                edge.violation = true;
                edge.against = true;
                if edge.violation_reason.is_none() {
                    edge.violation_reason = Some("cross-service dependency cycle".into());
                }
            } else if let (Some((_, from_tier)), Some((_, to_tier))) =
                (tier_map.get(&edge.from), tier_map.get(&edge.to))
                && from_tier > to_tier
            {
                edge.against = true;
                edge.violation = true;
                edge.violation_reason = Some("upward dependency against inferred tier".into());
            }
        }
    }

    fn infer_service_tiers(&self) -> BTreeMap<String, (String, usize)> {
        let mut adj: BTreeMap<String, Vec<String>> = self
            .service_order
            .iter()
            .map(|sid| (sid.clone(), vec![]))
            .collect();
        let mut indeg: BTreeMap<String, usize> = self
            .service_order
            .iter()
            .map(|sid| (sid.clone(), 0))
            .collect();
        for edge in self.edges.iter().filter(|e| e.level == "service") {
            if adj.contains_key(&edge.from) && adj.contains_key(&edge.to) {
                adj.entry(edge.from.clone())
                    .or_default()
                    .push(edge.to.clone());
                *indeg.entry(edge.to.clone()).or_default() += 1;
            }
        }
        let mut depth: BTreeMap<String, usize> = self
            .service_order
            .iter()
            .map(|sid| (sid.clone(), 0))
            .collect();
        let mut queue: VecDeque<String> = indeg
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(sid, _)| sid.clone())
            .collect();
        let mut visited = BTreeSet::new();
        while let Some(u) = queue.pop_front() {
            visited.insert(u.clone());
            let d = depth[&u];
            if let Some(targets) = adj.get(&u) {
                for v in targets {
                    let next_d = (d + 1).min(self.service_order.len());
                    if next_d > depth[v] {
                        depth.insert(v.clone(), next_d);
                    }
                    if let Some(entry) = indeg.get_mut(v) {
                        *entry = entry.saturating_sub(1);
                        if *entry == 0 {
                            queue.push_back(v.clone());
                        }
                    }
                }
            }
        }
        let max_visited = visited
            .iter()
            .filter_map(|sid| depth.get(sid).copied())
            .max()
            .unwrap_or(0);
        for sid in &self.service_order {
            if !visited.contains(sid) {
                depth.insert(sid.clone(), max_visited + 1);
            }
        }
        let max_tier = depth.values().copied().max().unwrap_or(0);
        depth
            .into_iter()
            .map(|(sid, t)| {
                let out_cnt = adj.get(&sid).map_or(0, Vec::len);
                let label = if !visited.contains(&sid) {
                    "cycle".into()
                } else if t == 0 && out_cnt > 0 {
                    "edge".into()
                } else if out_cnt == 0 && max_tier > 0 {
                    "foundation".into()
                } else {
                    format!("tier-{t}")
                };
                (sid, (label, t))
            })
            .collect()
    }

    fn service_edges(&self, certain: bool) -> Vec<&SystemEdge> {
        self.edges
            .iter()
            .filter(|edge| edge.level == "service" && (!certain || edge.certainty == "EXTRACTED"))
            .collect()
    }

    /// Exports the full cross-service graph JSON (used by `infra-engine system map --format json`
    /// and HTTP topology views).
    pub fn to_json(&self) -> Value {
        let mut view = self.view(&SystemMapQuery::default());
        view["cycles"] = object!(self.cycles(false));
        view["layers"] = self.layers();
        let contract_risks = self
            .edges
            .iter()
            .filter(|e| e.level == "service" && e.broken_risk)
            .flat_map(|e| {
                e.contracts.iter().filter(|c| c.broken_risk).map(move |c| {
                    object!({
                        "consumer": e.consumer,
                        "provider": e.provider,
                        "contract": c.id,
                        "kind": c.kind,
                        "consumer_version": c.consumer_version,
                        "provider_version": c.provider_version,
                        "provenance": c.provenance,
                        "reason": c.risk_reason,
                    })
                })
            })
            .collect::<Vec<_>>();
        view["contract_risks"] = object!(contract_risks);
        view["warnings"] = object!(self.warnings);
        view
    }

    /// Unified router matching `/map` query flags (`link`, `path`, `shared`, `cycle`, `view`).
    pub fn query(&self, params: &SystemMapQuery) -> Value {
        if let (Some(from), Some(to)) = (params.from.as_deref(), params.to.as_deref()) {
            if params.path {
                return self.shortest_path(from, to, params.focus.as_deref(), params.certain);
            }
            if params.shared {
                return self.shared(
                    from,
                    to,
                    params.side.as_deref(),
                    params.skip,
                    params.certain,
                    params.filter.as_deref().unwrap_or("all"),
                );
            }
            return self.link(from, to, params.certain);
        }
        if params.cycle {
            return self.cycle_view(
                params.scope.as_deref(),
                params.focus.as_deref(),
                params.certain,
            );
        }
        self.view(params)
    }

    /// Produces a filtered/expanded map view (`spread`, `whole`, `follow`, `layers`, `blast_radius`).
    pub fn view(&self, params: &SystemMapQuery) -> Value {
        let expanded_services: BTreeSet<String> = if params.whole {
            std::iter::once("__system__".to_string())
                .chain(self.service_order.iter().cloned())
                .collect()
        } else {
            let mut set: BTreeSet<String> = params
                .spread
                .iter()
                .flat_map(|entry| entry.split(','))
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_string)
                .collect();
            if let Some(scope) = &params.scope
                && (scope == "__system__" || self.nodes.contains_key(scope))
            {
                set.insert(scope.clone());
            }
            set
        };

        // Optional `follow` neighborhood filter (`out` = downstream consumers, `into` = upstream providers).
        let follow_allowed: Option<BTreeSet<String>> = params
            .follow
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|seed| {
                let out_hops = if params.out == 0 && params.into == 0 {
                    1
                } else {
                    params.out
                };
                let into_hops = params.into;
                let mut keep = BTreeSet::from([seed.to_string()]);
                // Downstream consumers (`to == current`, move to `from`).
                if out_hops > 0 {
                    let mut q = VecDeque::from([(seed.to_string(), 0usize)]);
                    while let Some((cur, d)) = q.pop_front() {
                        if d >= out_hops {
                            continue;
                        }
                        for edge in self.service_edges(params.certain) {
                            if edge.to == cur && keep.insert(edge.from.clone()) {
                                q.push_back((edge.from.clone(), d + 1));
                            }
                        }
                    }
                }
                // Upstream providers (`from == current`, move to `to`).
                if into_hops > 0 {
                    let mut q = VecDeque::from([(seed.to_string(), 0usize)]);
                    while let Some((cur, d)) = q.pop_front() {
                        if d >= into_hops {
                            continue;
                        }
                        for edge in self.service_edges(params.certain) {
                            if edge.from == cur && keep.insert(edge.to.clone()) {
                                q.push_back((edge.to.clone(), d + 1));
                            }
                        }
                    }
                }
                keep
            });

        let filter = params.filter.as_deref().unwrap_or("all");

        let visible_edges = self
            .edges
            .iter()
            .filter(|edge| {
                if params.certain && edge.certainty != "EXTRACTED" {
                    return false;
                }
                match edge.level.as_str() {
                    "service" => {
                        if let Some(allowed) = &follow_allowed
                            && (!allowed.contains(&edge.from) || !allowed.contains(&edge.to))
                        {
                            return false;
                        }
                        edge_matches_filter(edge, filter)
                    }
                    "shared" => {
                        if filter != "all" && filter != "shared" {
                            return false;
                        }
                        follow_allowed.is_none()
                    }
                    "detail" => {
                        let from_svc = self
                            .nodes
                            .get(&edge.from)
                            .map(|n| n.service.as_str())
                            .unwrap_or("");
                        let to_svc = self
                            .nodes
                            .get(&edge.to)
                            .map(|n| n.service.as_str())
                            .unwrap_or("");
                        expanded_services.contains(from_svc)
                            && (from_svc == to_svc
                                || expanded_services.contains(to_svc)
                                || self
                                    .nodes
                                    .get(&edge.from)
                                    .is_some_and(|n| n.kind == "service"))
                    }
                    _ => false,
                }
            })
            .cloned()
            .collect::<Vec<_>>();

        let endpoint_ids: BTreeSet<String> = visible_edges
            .iter()
            .flat_map(|e| [e.from.clone(), e.to.clone()])
            .collect();

        let mut visible_nodes = Vec::new();
        for node in self.nodes.values() {
            let show = match node.kind.as_str() {
                "service" => {
                    if let Some(allowed) = &follow_allowed {
                        allowed.contains(&node.id)
                    } else if matches!(filter, "risks" | "violations" | "changed") {
                        (filter == "changed" && node.changed_count > 0)
                            || endpoint_ids.contains(&node.id)
                    } else {
                        true
                    }
                }
                "system" => {
                    (params.whole
                        || expanded_services.contains("__system__")
                        || !self.shared_sources.is_empty()
                        || node.changed_count > 0)
                        && follow_allowed
                            .as_ref()
                            .is_none_or(|allowed| allowed.contains("__system__"))
                }
                _ => expanded_services.contains(&node.service),
            };
            if show {
                visible_nodes.push(node.clone());
            }
        }

        let frames = expanded_services
            .iter()
            .filter_map(|sid| {
                let parent = self.nodes.get(sid)?;
                let children = visible_nodes
                    .iter()
                    .filter(|n| n.parent.as_deref() == Some(sid.as_str()))
                    .map(|n| n.id.clone())
                    .collect::<Vec<_>>();
                (!children.is_empty()).then(|| {
                    object!({
                        "id": sid,
                        "label": parent.label,
                        "kind": parent.kind,
                        "stack": parent.stack,
                        "layer": parent.layer,
                        "children": children,
                    })
                })
            })
            .collect::<Vec<_>>();

        let focus_payload = params
            .focus
            .as_deref()
            .and_then(|focus_id| self.focus_detail(focus_id, params.certain));

        let mut out = object!({
            "kind": "system_graph",
            "system": self.system_name,
            "root": self.root,
            "whole": params.whole,
            "spread": expanded_services,
            "nodes": visible_nodes,
            "edges": visible_edges,
            "frames": frames,
            "focus": focus_payload,
            "summary": {
                "service_count": self.service_order.len(),
                "edge_count": self.edges.iter().filter(|e| e.level == "service").count(),
                "risk_count": self.edges.iter().filter(|e| e.level == "service" && e.broken_risk).count(),
                "violation_count": self.edges.iter().filter(|e| e.level == "service" && e.violation).count(),
                "undeclared_count": self.edges.iter().filter(|e| e.level == "service" && e.provenance == "undeclared_in_code").count(),
            }
        });
        if params.layers {
            out["layers"] = self.layers();
        }
        if !params.seed_services.is_empty() || !params.contracts.is_empty() {
            out["blast_radius"] = self.blast_radius(
                &params.seed_services,
                &params.contracts,
                params.depth.unwrap_or(32),
            );
        }
        out
    }

    fn focus_detail(&self, id: &str, certain: bool) -> Option<Value> {
        let node = self.nodes.get(id)?;
        let uses = self
            .edges
            .iter()
            .filter(|e| {
                e.from == id && e.level == "service" && (!certain || e.certainty == "EXTRACTED")
            })
            .map(|e| {
                object!({
                    "id": e.to,
                    "kinds": e.kinds,
                    "provenance": e.provenance,
                    "broken_risk": e.broken_risk,
                    "violation": e.violation,
                    "contracts": e.contracts,
                })
            })
            .collect::<Vec<_>>();
        let used_by = self
            .edges
            .iter()
            .filter(|e| {
                e.to == id && e.level == "service" && (!certain || e.certainty == "EXTRACTED")
            })
            .map(|e| {
                object!({
                    "id": e.from,
                    "kinds": e.kinds,
                    "provenance": e.provenance,
                    "broken_risk": e.broken_risk,
                    "violation": e.violation,
                    "contracts": e.contracts,
                })
            })
            .collect::<Vec<_>>();
        let passport = self.manifests.get(&node.service);
        let provides = if node.kind == "service" {
            passport
                .and_then(|m| m["provides"].as_array())
                .cloned()
                .unwrap_or_default()
        } else {
            vec![]
        };
        let consumes = if node.kind == "service" {
            passport
                .and_then(|m| m["consumes"].as_array())
                .cloned()
                .unwrap_or_default()
        } else {
            vec![]
        };
        let capabilities = if node.kind == "service" {
            passport
                .and_then(|m| m["capabilities"].as_array())
                .cloned()
                .unwrap_or_default()
        } else {
            vec![]
        };
        Some(object!({
            "node": node,
            "uses": uses,
            "used_by": used_by,
            "provides": provides,
            "consumes": consumes,
            "capabilities": capabilities,
        }))
    }

    /// Inspects the contract and code links between two services (`from` and `to`).
    pub fn link(&self, from: &str, to: &str, certain: bool) -> Value {
        let forward = self
            .service_edges(certain)
            .into_iter()
            .find(|e| e.from == from && e.to == to)
            .cloned();
        let reverse = self
            .service_edges(certain)
            .into_iter()
            .find(|e| e.from == to && e.to == from)
            .cloned();

        let direction = match (&forward, &reverse) {
            (Some(_), Some(_)) => "bidirectional",
            (Some(_), None) => "forward",
            (None, Some(_)) => "reverse",
            (None, None) => "none",
        };
        let primary = forward.as_ref().or(reverse.as_ref());
        let code_refs = self
            .summaries
            .get(from)
            .map(|s| {
                s.code_refs
                    .iter()
                    .filter(|r| r.target_service == to && (!certain || r.certainty == "EXTRACTED"))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let reverse_code_refs = self
            .summaries
            .get(to)
            .map(|s| {
                s.code_refs
                    .iter()
                    .filter(|r| {
                        r.target_service == from && (!certain || r.certainty == "EXTRACTED")
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        object!({
            "kind": "system_link",
            "from": from,
            "to": to,
            "direction": direction,
            "edge": primary,
            "forward": forward,
            "reverse": reverse,
            "contracts": primary.map(|e| e.contracts.clone()).unwrap_or_default(),
            "kinds": primary.map(|e| e.kinds.clone()).unwrap_or_default(),
            "provenance": primary.map(|e| e.provenance.clone()),
            "broken_risk": primary.is_some_and(|e| e.broken_risk),
            "violation": primary.is_some_and(|e| e.violation),
            "violation_reason": primary.and_then(|e| e.violation_reason.clone()),
            "code_refs": code_refs,
            "reverse_code_refs": reverse_code_refs,
        })
    }

    /// Finds the shortest cross-service dependency path between `from` and `to` (checking reverse
    /// direction if no forward path exists, matching `graph_map` semantics).
    pub fn shortest_path(&self, from: &str, to: &str, focus: Option<&str>, certain: bool) -> Value {
        let edges = self.service_edges(certain);
        let bfs = |start: &str, goal: &str| -> Option<Vec<String>> {
            if !self.nodes.contains_key(start) || !self.nodes.contains_key(goal) {
                return None;
            }
            let mut adj: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
            for edge in &edges {
                adj.entry(edge.from.as_str())
                    .or_default()
                    .push(edge.to.as_str());
            }
            for list in adj.values_mut() {
                list.sort_unstable();
                list.dedup();
            }
            let mut queue = VecDeque::new();
            let mut prev: BTreeMap<String, String> = BTreeMap::new();
            if start == goal {
                if let Some(neighbors) = adj.get(start) {
                    for &next in neighbors {
                        prev.insert(next.to_string(), start.to_string());
                        queue.push_back(next.to_string());
                    }
                }
            } else {
                queue.push_back(start.to_string());
            }
            while let Some(cur) = queue.pop_front() {
                if cur == goal && (!prev.is_empty() || start != goal) {
                    let mut chain = vec![goal.to_string()];
                    let mut walk = goal.to_string();
                    while let Some(parent) = prev.get(&walk) {
                        chain.push(parent.clone());
                        if parent == start {
                            break;
                        }
                        walk = parent.clone();
                    }
                    chain.reverse();
                    return Some(chain);
                }
                if let Some(neighbors) = adj.get(cur.as_str()) {
                    for &next in neighbors {
                        if !prev.contains_key(next) && (next != start || start == goal) {
                            prev.insert(next.to_string(), cur.clone());
                            queue.push_back(next.to_string());
                        }
                    }
                }
            }
            None
        };

        let (direction, steps) = if let Some(forward) = bfs(from, to) {
            ("forward", forward)
        } else if from != to
            && let Some(rev) = bfs(to, from)
        {
            ("reverse", rev)
        } else {
            ("none", vec![])
        };

        let step_set: BTreeSet<&str> = steps.iter().map(String::as_str).collect();
        let path_edges = steps
            .windows(2)
            .filter_map(|pair| {
                edges
                    .iter()
                    .find(|e| e.from == pair[0] && e.to == pair[1])
                    .copied()
                    .cloned()
            })
            .collect::<Vec<_>>();
        let path_nodes = self
            .service_order
            .iter()
            .filter(|sid| {
                step_set.contains(sid.as_str()) || sid.as_str() == from || sid.as_str() == to
            })
            .filter_map(|sid| self.nodes.get(sid).cloned())
            .collect::<Vec<_>>();

        object!({
            "kind": "system_path",
            "from": from,
            "to": to,
            "found": !steps.is_empty(),
            "direction": direction,
            "hops": steps.len().saturating_sub(1),
            "steps": steps,
            "nodes": path_nodes,
            "edges": path_edges,
            "focus": focus.and_then(|id| self.focus_detail(id, certain)),
        })
    }

    /// Computes shared upstream providers (`both_use`) and shared downstream consumers (`uses_both`)
    /// between services `a` and `b`.
    pub fn shared(
        &self,
        a: &str,
        b: &str,
        side: Option<&str>,
        skip: usize,
        certain: bool,
        filter: &str,
    ) -> Value {
        let edges = self
            .service_edges(certain)
            .into_iter()
            .filter(|e| edge_matches_filter(e, filter))
            .collect::<Vec<_>>();
        let a_uses: BTreeSet<String> = edges
            .iter()
            .filter(|e| e.from == a)
            .map(|e| e.to.clone())
            .collect();
        let b_uses: BTreeSet<String> = edges
            .iter()
            .filter(|e| e.from == b)
            .map(|e| e.to.clone())
            .collect();
        let a_used_by: BTreeSet<String> = edges
            .iter()
            .filter(|e| e.to == a)
            .map(|e| e.from.clone())
            .collect();
        let b_used_by: BTreeSet<String> = edges
            .iter()
            .filter(|e| e.to == b)
            .map(|e| e.from.clone())
            .collect();

        let all_both_use = a_uses
            .intersection(&b_uses)
            .filter(|id| id.as_str() != a && id.as_str() != b)
            .cloned()
            .collect::<Vec<_>>();
        let all_uses_both = a_used_by
            .intersection(&b_used_by)
            .filter(|id| id.as_str() != a && id.as_str() != b)
            .cloned()
            .collect::<Vec<_>>();

        let page_size = 24usize;
        let show_both_use = matches!(side, None | Some("") | Some("both") | Some("both_use"));
        let show_uses_both = matches!(side, None | Some("") | Some("both") | Some("uses_both"));

        let paged_both_use = if show_both_use {
            all_both_use
                .iter()
                .skip(skip)
                .take(page_size)
                .cloned()
                .collect::<Vec<_>>()
        } else {
            vec![]
        };
        let paged_uses_both = if show_uses_both {
            all_uses_both
                .iter()
                .skip(skip)
                .take(page_size)
                .cloned()
                .collect::<Vec<_>>()
        } else {
            vec![]
        };

        let mut keep = BTreeSet::from([a.to_string(), b.to_string()]);
        keep.extend(paged_both_use.iter().cloned());
        keep.extend(paged_uses_both.iter().cloned());

        let sub_nodes = keep
            .iter()
            .filter_map(|id| self.nodes.get(id).cloned())
            .collect::<Vec<_>>();
        let sub_edges = edges
            .into_iter()
            .filter(|e| {
                ((e.from == a || e.from == b) && paged_both_use.contains(&e.to))
                    || (paged_uses_both.contains(&e.from) && (e.to == a || e.to == b))
                    || ((e.from == a && e.to == b) || (e.from == b && e.to == a))
            })
            .cloned()
            .collect::<Vec<_>>();

        let has_more = (show_both_use && skip + paged_both_use.len() < all_both_use.len())
            || (show_uses_both && skip + paged_uses_both.len() < all_uses_both.len());

        object!({
            "kind": "system_shared",
            "a": a,
            "b": b,
            "both_use": paged_both_use,
            "uses_both": paged_uses_both,
            "total_both_use": all_both_use.len(),
            "total_uses_both": all_uses_both.len(),
            "skip": skip,
            "has_more": has_more,
            "nodes": sub_nodes,
            "edges": sub_edges,
        })
    }

    /// Returns canonical cross-service dependency cycles (`[A, B, ..., A]`).
    pub fn cycles(&self, certain: bool) -> Vec<Vec<String>> {
        let mut adj: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for edge in self.service_edges(certain) {
            adj.entry(edge.from.clone())
                .or_default()
                .push(edge.to.clone());
        }
        for targets in adj.values_mut() {
            targets.sort();
            targets.dedup();
        }
        let mut seen_keys = BTreeSet::<String>::new();
        let mut cycles = Vec::<Vec<String>>::new();
        const MAX_CYCLES: usize = 64;

        for start in &self.service_order {
            if cycles.len() >= MAX_CYCLES {
                break;
            }
            let mut stack = vec![(start.clone(), vec![start.clone()])];
            while let Some((cur, path)) = stack.pop() {
                if cycles.len() >= MAX_CYCLES {
                    break;
                }
                let Some(neighbors) = adj.get(&cur) else {
                    continue;
                };
                for next in neighbors.iter().rev() {
                    if next == start && path.len() >= 2 {
                        let canonical = canonicalize_cycle(&path);
                        let key = canonical.join("->");
                        if seen_keys.insert(key) {
                            let mut closed = canonical;
                            closed.push(closed[0].clone());
                            cycles.push(closed);
                        }
                    } else if next > start && !path.contains(next) && path.len() < 16 {
                        let mut next_path = path.clone();
                        next_path.push(next.clone());
                        stack.push((next.clone(), next_path));
                    }
                }
            }
        }
        cycles.sort();
        cycles
    }

    /// Map payload highlighting cross-service dependency cycles.
    pub fn cycle_view(&self, origin: Option<&str>, focus: Option<&str>, certain: bool) -> Value {
        let all_cycles = self.cycles(certain);
        let filtered = match origin.filter(|o| !o.is_empty()) {
            Some(target) => all_cycles
                .into_iter()
                .filter(|c| c.iter().any(|item| item == target))
                .collect::<Vec<_>>(),
            None => all_cycles,
        };
        let mut cycle_nodes_set = BTreeSet::new();
        let mut cycle_pairs = BTreeSet::new();
        for cycle in &filtered {
            for item in cycle {
                cycle_nodes_set.insert(item.clone());
            }
            for pair in cycle.windows(2) {
                cycle_pairs.insert((pair[0].clone(), pair[1].clone()));
            }
        }
        let nodes = cycle_nodes_set
            .iter()
            .filter_map(|id| self.nodes.get(id).cloned())
            .collect::<Vec<_>>();
        let edges = self
            .service_edges(certain)
            .into_iter()
            .filter(|e| cycle_pairs.contains(&(e.from.clone(), e.to.clone())))
            .cloned()
            .collect::<Vec<_>>();

        object!({
            "kind": "system_cycles",
            "count": filtered.len(),
            "cycles": filtered,
            "cycle_nodes": cycle_nodes_set,
            "nodes": nodes,
            "edges": edges,
            "focus": focus.and_then(|id| self.focus_detail(id, certain)),
        })
    }

    /// Computes downstream blast radius and implementation wave order for `Plan a change`.
    pub fn blast_radius(
        &self,
        seed_services: &[String],
        contracts: &[String],
        depth: usize,
    ) -> Value {
        let bounded_depth = depth.min(64);
        let mut waves: BTreeMap<String, Value> = BTreeMap::new();
        let mut queue = VecDeque::<String>::new();
        let mut contract_filters: BTreeSet<(String, String)> = BTreeSet::new();

        for sid in seed_services {
            if self.nodes.get(sid).is_some_and(|n| n.kind == "service") && !waves.contains_key(sid)
            {
                waves.insert(
                    sid.clone(),
                    object!({
                        "service": sid,
                        "wave": 0,
                        "reason": "seed",
                        "via_contracts": [],
                        "path": [sid],
                    }),
                );
                queue.push_back(sid.clone());
            }
        }

        for selector in contracts {
            let resolved = if let Some((sid, cid)) = selector.split_once(':') {
                Some((sid.to_string(), cid.to_string()))
            } else {
                let matches = self
                    .manifests
                    .iter()
                    .filter(|(_, m)| {
                        m["provides"]
                            .as_array()
                            .is_some_and(|arr| arr.iter().any(|p| p["id"] == selector.as_str()))
                    })
                    .map(|(sid, _)| sid.clone())
                    .collect::<Vec<_>>();
                (matches.len() == 1).then(|| (matches[0].clone(), selector.clone()))
            };
            if let Some((sid, cid)) = resolved {
                contract_filters.insert((sid.clone(), cid.clone()));
                if !waves.contains_key(&sid) {
                    waves.insert(
                        sid.clone(),
                        object!({
                            "service": sid,
                            "wave": 0,
                            "reason": "contract_provider",
                            "via_contracts": [cid],
                            "path": [sid],
                        }),
                    );
                    queue.push_back(sid);
                }
            }
        }

        let mut impacted_edges = Vec::new();
        while let Some(provider_id) = queue.pop_front() {
            let current_path = texts(&waves[&provider_id]["path"]);
            let current_wave = waves[&provider_id]["wave"].as_u64().unwrap_or(0) as usize;
            if current_wave >= bounded_depth {
                continue;
            }
            for edge in self.service_edges(false) {
                if edge.provider != provider_id {
                    continue;
                }
                let matched_contracts = edge
                    .contracts
                    .iter()
                    .filter(|c| {
                        contract_filters.is_empty()
                            || current_wave > 0
                            || contract_filters.contains(&(provider_id.clone(), c.id.clone()))
                    })
                    .map(|c| c.id.clone())
                    .collect::<Vec<_>>();
                if matched_contracts.is_empty() && !edge.contracts.is_empty() {
                    continue;
                }
                impacted_edges.push(edge.clone());
                let consumer = edge.consumer.clone();
                if !waves.contains_key(&consumer) {
                    let mut next_path = current_path.clone();
                    next_path.push(consumer.clone());
                    waves.insert(
                        consumer.clone(),
                        object!({
                            "service": consumer,
                            "wave": current_wave + 1,
                            "reason": "downstream_consumer",
                            "via_contracts": matched_contracts,
                            "broken_risk": edge.broken_risk,
                            "path": next_path,
                        }),
                    );
                    queue.push_back(consumer);
                }
            }
        }

        let mut ordered = waves.values().cloned().collect::<Vec<_>>();
        ordered.sort_by(|a, b| {
            a["wave"]
                .as_u64()
                .cmp(&b["wave"].as_u64())
                .then_with(|| string(&a["service"]).cmp(string(&b["service"])))
        });
        let recommended_order = ordered
            .iter()
            .map(|entry| string(&entry["service"]).to_string())
            .collect::<Vec<_>>();
        let mut by_wave: BTreeMap<u64, Vec<String>> = BTreeMap::new();
        for entry in &ordered {
            let w = entry["wave"].as_u64().unwrap_or(0);
            by_wave
                .entry(w)
                .or_default()
                .push(string(&entry["service"]).to_string());
        }
        let grouped_waves = by_wave
            .into_iter()
            .filter(|(w, _)| *w > 0)
            .map(|(w, svcs)| {
                object!({
                    "wave": w,
                    "depth": w,
                    "services": svcs,
                })
            })
            .collect::<Vec<_>>();

        object!({
            "seed_services": seed_services,
            "contracts": contracts,
            "depth": bounded_depth,
            "affected_count": ordered.len(),
            "services": ordered,
            "waves": grouped_waves,
            "recommended_order": recommended_order,
            "edges": impacted_edges,
        })
    }

    /// Returns inferred or declared architectural layers and cross-service tier violations.
    pub fn layers(&self) -> Value {
        let mut grouped: BTreeMap<(usize, String), Vec<String>> = BTreeMap::new();
        for sid in &self.service_order {
            if let Some(node) = self.nodes.get(sid) {
                let tier = node.tier.unwrap_or(0);
                let name = node.layer.clone().unwrap_or_else(|| format!("tier-{tier}"));
                grouped.entry((tier, name)).or_default().push(sid.clone());
            }
        }
        let tiers = grouped
            .into_iter()
            .map(|((tier, name), services)| {
                let allows = self
                    .architecture
                    .layers
                    .iter()
                    .find(|r| r.name == name)
                    .and_then(|r| r.allows.clone());
                object!({
                    "tier": tier,
                    "name": name,
                    "services": services,
                    "allows": allows,
                })
            })
            .collect::<Vec<_>>();

        let violations = self
            .edges
            .iter()
            .filter(|e| e.level == "service" && e.violation)
            .map(|e| {
                object!({
                    "from": e.from,
                    "to": e.to,
                    "consumer": e.consumer,
                    "provider": e.provider,
                    "from_layer": self.nodes.get(&e.from).and_then(|n| n.layer.clone()),
                    "to_layer": self.nodes.get(&e.to).and_then(|n| n.layer.clone()),
                    "against": e.against,
                    "reason": e.violation_reason,
                    "contracts": e.contracts.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>();

        let intra_service = self
            .summaries
            .iter()
            .filter(|(_, s)| !s.layer_violations.is_empty())
            .map(|(sid, s)| (sid.clone(), object!(s.layer_violations)))
            .collect::<serde_json::Map<_, _>>();

        object!({
            "declared": self.architecture.declared,
            "source_file": self.architecture.source_file,
            "tiers": tiers,
            "violations": violations,
            "violation_count": violations.len(),
            "intra_service_violations": intra_service,
        })
    }

    /// Searches across services, contracts, capabilities, declared sources, and code entities.
    pub fn find(&self, query: &str) -> Value {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return object!({ "query": query, "results": [] });
        }
        let mut hits: Vec<(i64, Value)> = Vec::new();
        for node in self.nodes.values() {
            let id_low = node.id.to_lowercase();
            let label_low = node.label.to_lowercase();
            let desc_low = node.description.to_lowercase();
            let kw_match = node
                .keywords
                .iter()
                .any(|kw| kw.to_lowercase().contains(&needle));
            let score = if label_low == needle || id_low == needle {
                100
            } else if label_low.starts_with(&needle) {
                85
            } else if label_low.contains(&needle) || id_low.contains(&needle) {
                70
            } else if kw_match {
                55
            } else if desc_low.contains(&needle)
                || node
                    .stack
                    .as_ref()
                    .is_some_and(|s| s.to_lowercase().contains(&needle))
            {
                40
            } else {
                0
            };
            if score > 0 {
                let kind_bonus = match node.kind.as_str() {
                    "service" => 5,
                    "contract" => 4,
                    "capability" => 3,
                    _ => 0,
                };
                hits.push((
                    score + kind_bonus,
                    object!({
                        "id": node.id,
                        "label": node.label,
                        "kind": node.kind,
                        "service": node.service,
                        "path": node.path,
                        "contract_kind": node.contract_kind,
                        "version": node.version,
                        "description": node.description,
                        "score": score + kind_bonus,
                    }),
                ));
            }
        }
        for (sid, summary) in &self.summaries {
            for entity in &summary.changed_entities {
                let name_low = entity.name.to_lowercase();
                if name_low.contains(&needle) {
                    let score = if name_low == needle { 80 } else { 50 };
                    hits.push((
                        score,
                        object!({
                            "id": format!("entity:{sid}:{}:{}", entity.path, entity.name),
                            "label": entity.name,
                            "kind": "entity",
                            "entity_kind": entity.kind,
                            "service": sid,
                            "path": entity.path,
                            "change": entity.change,
                            "score": score,
                        }),
                    ));
                }
            }
        }
        hits.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| string(&a.1["id"]).cmp(string(&b.1["id"])))
        });
        hits.truncate(100);
        object!({
            "query": query,
            "results": hits.into_iter().map(|(_, item)| item).collect::<Vec<_>>(),
        })
    }

    /// Aggregates changed files and symbols across `__system__` and member services,
    /// mapping changes to impacted provided contracts and downstream consumer services.
    pub fn changes_across_services(&self, service_filter: Option<&str>, notests: bool) -> Value {
        let mut services_out = Vec::new();
        let mut total_files = 0usize;
        let mut total_entities = 0usize;
        let mut impacted_contracts = Vec::new();
        let mut downstream_services = BTreeSet::new();

        let all_ids = std::iter::once("__system__".to_string())
            .chain(self.service_order.iter().cloned())
            .collect::<Vec<_>>();

        for sid in all_ids {
            if let Some(filter_sid) = service_filter.filter(|s| !s.is_empty())
                && sid != filter_sid
            {
                continue;
            }
            let Some(summary) = self.summaries.get(&sid) else {
                continue;
            };
            let files = summary
                .changed_files
                .iter()
                .filter(|f| !notests || (!f.is_test && !looks_like_test_path(&f.path)))
                .cloned()
                .collect::<Vec<_>>();
            let entities = summary
                .changed_entities
                .iter()
                .filter(|e| !notests || !looks_like_test_path(&e.path))
                .cloned()
                .collect::<Vec<_>>();
            if files.is_empty() && entities.is_empty() {
                continue;
            }
            total_files += files.len();
            total_entities += entities.len();

            let mut svc_contracts = Vec::new();
            if sid != "__system__" {
                let impact = self.impact_across_services(&CrossServiceImpactQuery {
                    service: Some(sid.clone()),
                    path: None,
                    contract: None,
                    certain: false,
                    opened_by_service: BTreeMap::new(),
                });
                if let Some(arr) = impact["impacted_contracts"].as_array() {
                    svc_contracts = arr.clone();
                    impacted_contracts.extend(arr.iter().cloned());
                }
                for ds in texts(&impact["downstream_services"]) {
                    downstream_services.insert(ds);
                }
            }

            services_out.push(object!({
                "service": sid,
                "root": self.nodes.get(&sid).and_then(|n| n.root.clone()),
                "changed_files": files,
                "changed_entities": entities,
                "impacted_contracts": svc_contracts,
            }));
        }

        object!({
            "system": self.system_name,
            "total_files": total_files,
            "total_entities": total_entities,
            "services": services_out,
            "impacted_contracts": impacted_contracts,
            "downstream_services": downstream_services,
        })
    }

    /// Maps changed files/symbols or explicit contract selectors in Service A to impacted
    /// `provides` contracts & `capabilities`, downstream consuming services B/C, their consumer
    /// source files, and `cross_service_unopened` files left unopened during a system run.
    pub fn impact_across_services(&self, query: &CrossServiceImpactQuery) -> Value {
        let resolved_service = query.service.clone().or_else(|| {
            query.contract.as_deref().and_then(|sel| {
                if let Some((sid, _)) = sel.split_once(':') {
                    Some(sid.to_string())
                } else {
                    self.manifests
                        .iter()
                        .find(|(_, m)| {
                            m["provides"]
                                .as_array()
                                .is_some_and(|arr| arr.iter().any(|p| p["id"] == sel))
                        })
                        .map(|(sid, _)| sid.clone())
                }
            })
        });

        let Some(sid) = resolved_service else {
            return object!({
                "service": Value::Null,
                "path": query.path,
                "impacted_capabilities": [],
                "impacted_contracts": [],
                "contract_consumers": [],
                "downstream_services": [],
                "cross_service_unopened": [],
                "broken_risks": [],
            });
        };

        let target_paths: Vec<String> = if let Some(path) = &query.path {
            vec![path.clone()]
        } else if query.contract.is_some() {
            // An explicit contract without a path asks about that contract
            // alone, not every uncommitted file of its provider.
            Vec::new()
        } else {
            self.summaries
                .get(&sid)
                .map(|s| s.changed_files.iter().map(|f| f.path.clone()).collect())
                .unwrap_or_default()
        };

        let explicit_contract = query.contract.as_deref().map(|sel| {
            sel.split_once(':')
                .map(|(_, cid)| cid.to_string())
                .unwrap_or_else(|| sel.to_string())
        });

        let mut impacted_capabilities = Vec::new();
        let mut impacted_contracts = Vec::new();
        let mut matched_contract_ids = BTreeSet::new();

        if let Some(passport) = self.manifests.get(&sid) {
            for cap in passport["capabilities"].as_array().into_iter().flatten() {
                let cap_sources = texts(&cap["sources"]);
                let matched_files = target_paths
                    .iter()
                    .filter(|tp| cap_sources.iter().any(|cs| source_path_matches(tp, cs)))
                    .cloned()
                    .collect::<Vec<_>>();
                if !matched_files.is_empty() {
                    impacted_capabilities.push(object!({
                        "service": sid,
                        "capability": cap["id"],
                        "status": cap["status"],
                        "matched_paths": matched_files,
                        "sources": cap_sources,
                    }));
                }
            }

            for prov in passport["provides"].as_array().into_iter().flatten() {
                let cid = string(&prov["id"]).to_string();
                let prov_sources = texts(&prov["sources"]);
                let mut matched_files = target_paths
                    .iter()
                    .filter(|tp| prov_sources.iter().any(|ps| source_path_matches(tp, ps)))
                    .cloned()
                    .collect::<Vec<_>>();
                if explicit_contract.as_deref() == Some(cid.as_str()) && matched_files.is_empty() {
                    matched_files = prov_sources.clone();
                }
                if !matched_files.is_empty() || explicit_contract.as_deref() == Some(cid.as_str()) {
                    matched_contract_ids.insert(cid.clone());
                    impacted_contracts.push(object!({
                        "service": sid,
                        "contract": cid,
                        "kind": prov["kind"],
                        "version": prov["version"],
                        "matched_paths": matched_files,
                        "sources": prov_sources,
                    }));
                }
            }
        }

        // Also check undeclared / code-referenced contracts targeting `sid` when no explicit match yet.
        let verify_opened: BTreeSet<String> = query
            .opened_by_service
            .get("verify")
            .into_iter()
            .chain(query.opened_by_service.get("__system__"))
            .flat_map(|set| set.iter().cloned())
            .collect();

        let mut contract_consumers = Vec::new();
        let mut downstream_services = BTreeSet::new();
        let mut cross_service_unopened = Vec::new();
        let mut broken_risks = Vec::new();

        for edge in self.service_edges(query.certain) {
            if edge.provider != sid {
                continue;
            }
            for c in &edge.contracts {
                if !matched_contract_ids.contains(&c.id) {
                    continue;
                }
                let consumer_id = edge.consumer.clone();
                downstream_services.insert(consumer_id.clone());

                let consumer_summary = self.summaries.get(&consumer_id);
                let consumer_passport = self.manifests.get(&consumer_id);
                let consumer_paths = consumer_passport
                    .map(|p| {
                        resolve_consumer_sources_for_contract(p, consumer_summary, &sid, &c.id)
                    })
                    .unwrap_or_else(|| c.consumer_sources.clone());

                let mut opened_in_consumer: BTreeSet<String> = consumer_summary
                    .map(|s| s.opened_files.clone())
                    .unwrap_or_default();
                if let Some(extra) = query.opened_by_service.get(&consumer_id) {
                    opened_in_consumer.extend(extra.iter().cloned());
                }

                let mut unopened_paths = Vec::new();
                for rel_path in &consumer_paths {
                    let prefixed = format!("{consumer_id}/{rel_path}");
                    let was_opened = opened_in_consumer.contains(rel_path)
                        || opened_in_consumer.contains(&prefixed)
                        || verify_opened.contains(rel_path)
                        || verify_opened.contains(&prefixed);
                    if !was_opened {
                        unopened_paths.push(rel_path.clone());
                        cross_service_unopened.push(object!({
                            "provider": sid,
                            "contract": c.id,
                            "kind": c.kind,
                            "consumer": consumer_id,
                            "path": rel_path,
                            "broken_risk": c.broken_risk,
                        }));
                    }
                }

                if c.broken_risk {
                    broken_risks.push(object!({
                        "provider": sid,
                        "consumer": consumer_id,
                        "contract": c.id,
                        "reason": c.risk_reason,
                    }));
                }

                contract_consumers.push(object!({
                    "provider": sid,
                    "consumer": consumer_id,
                    "contract": c.id,
                    "kind": c.kind,
                    "consumer_version": c.consumer_version,
                    "provider_version": c.provider_version,
                    "provenance": c.provenance,
                    "broken_risk": c.broken_risk,
                    "risk_reason": c.risk_reason,
                    "consumer_sources": consumer_paths,
                    "unopened_sources": unopened_paths,
                }));
            }
        }

        // Transitive downstream blast radius for impacted contracts.
        let contract_selectors = matched_contract_ids
            .iter()
            .map(|cid| format!("{sid}:{cid}"))
            .collect::<Vec<_>>();
        let blast = if !contract_selectors.is_empty() {
            self.blast_radius(&[], &contract_selectors, 16)
        } else if !target_paths.is_empty() {
            self.blast_radius(std::slice::from_ref(&sid), &[], 16)
        } else {
            object!({ "services": [], "recommended_order": [] })
        };
        if let Some(arr) = blast["services"].as_array() {
            for item in arr {
                let ds = string(&item["service"]);
                if !ds.is_empty() && ds != sid {
                    downstream_services.insert(ds.to_string());
                }
            }
        }

        object!({
            "service": sid,
            "path": query.path,
            "target_paths": target_paths,
            "impacted_capabilities": impacted_capabilities,
            "impacted_contracts": impacted_contracts,
            "contract_consumers": contract_consumers,
            "downstream_services": downstream_services,
            "cross_service_unopened": cross_service_unopened,
            "broken_risks": broken_risks,
            "blast_radius": blast,
        })
    }
}

fn edge_matches_filter(edge: &SystemEdge, filter: &str) -> bool {
    match filter {
        "" | "all" | "contracts" => true,
        "risks" | "broken" => edge.broken_risk,
        "violations" | "layers" => edge.violation || edge.against,
        "undeclared" | "drift" => edge.provenance == "undeclared_in_code",
        "verified" => edge.provenance == "verified",
        kind => edge.kinds.iter().any(|k| k == kind),
    }
}

fn canonicalize_cycle(cycle: &[String]) -> Vec<String> {
    if cycle.is_empty() {
        return vec![];
    }
    let min_idx = cycle
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.cmp(b.1))
        .map_or(0, |(idx, _)| idx);
    let mut rotated = Vec::with_capacity(cycle.len());
    rotated.extend_from_slice(&cycle[min_idx..]);
    rotated.extend_from_slice(&cycle[..min_idx]);
    rotated
}

fn source_path_matches(changed_path: &str, declared_source: &str) -> bool {
    if changed_path == declared_source {
        return true;
    }
    let trimmed = declared_source.trim_end_matches('/');
    changed_path
        .strip_prefix(trimmed)
        .is_some_and(|rest| rest.starts_with('/'))
}

fn resolve_consumer_sources_for_contract(
    consumer_passport: &Value,
    consumer_summary: Option<&ServiceCodeSummary>,
    provider_id: &str,
    contract_id: &str,
) -> Vec<String> {
    let mut out = BTreeSet::new();
    if let Some(summary) = consumer_summary {
        for key in [
            format!("{provider_id}:{contract_id}"),
            contract_id.to_string(),
        ] {
            if let Some(files) = summary.consumer_files.get(&key) {
                out.extend(files.iter().cloned());
            }
        }
        for r in &summary.code_refs {
            if r.target_service == provider_id
                && (r.contract.as_deref() == Some(contract_id) || r.contract.is_none())
                && !r.from_path.is_empty()
            {
                out.insert(r.from_path.clone());
            }
        }
    }
    let contract_tokens = contract_id
        .split(['.', '-', '_', ':', '/'])
        .filter(|tok| tok.len() >= 3)
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    let provider_low = provider_id.to_lowercase();

    for cap in consumer_passport["capabilities"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let hay = format!(
            "{} {} {}",
            string(&cap["id"]),
            string(&cap["description"]),
            texts(&cap["keywords"]).join(" ")
        )
        .to_lowercase();
        let mentions =
            hay.contains(&provider_low) || contract_tokens.iter().any(|tok| hay.contains(tok));
        if mentions {
            out.extend(texts(&cap["sources"]));
        }
    }

    for src in consumer_passport["sources"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let p = string(&src["path"]);
        let kind = string(&src["kind"]);
        if p.is_empty() {
            continue;
        }
        let p_low = p.to_lowercase();
        if kind == "contract"
            || p_low.contains(&provider_low)
            || contract_tokens.iter().any(|tok| p_low.contains(tok))
        {
            out.insert(p.to_string());
        }
    }

    // Fallback to declared code/contract sources on the consumer passport if no specific match yet.
    if out.is_empty() {
        for src in consumer_passport["sources"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if matches!(string(&src["kind"]), "contract" | "code") {
                let p = string(&src["path"]);
                if !p.is_empty() {
                    out.insert(p.to_string());
                }
            }
        }
    }
    out.into_iter().collect()
}

fn detect_service_stack(root: &Path) -> String {
    let probes = [
        ("composer.json", "php"),
        ("Cargo.toml", "rust"),
        ("go.mod", "go"),
        ("package.json", "node"),
        ("tsconfig.json", "node"),
        ("pyproject.toml", "python"),
        ("requirements.txt", "python"),
    ];
    for (file, stack) in probes {
        if std::fs::symlink_metadata(root.join(file)).is_ok_and(|m| m.is_file()) {
            return stack.into();
        }
    }
    for parent in root.ancestors().skip(1).take(3) {
        if std::fs::symlink_metadata(parent.join("composer.json")).is_ok_and(|m| m.is_file()) {
            return "php".into();
        }
        if std::fs::symlink_metadata(parent.join("Cargo.toml")).is_ok_and(|m| m.is_file()) {
            return "rust".into();
        }
        if std::fs::symlink_metadata(parent.join("go.mod")).is_ok_and(|m| m.is_file()) {
            return "go".into();
        }
        if std::fs::symlink_metadata(parent.join("package.json")).is_ok_and(|m| m.is_file()) {
            return "node".into();
        }
        if std::fs::symlink_metadata(parent.join(".git")).is_ok() {
            break;
        }
    }
    "unknown".into()
}

fn load_architecture_rules(
    system_root: &Path,
    _services: &BTreeMap<String, Service>,
) -> SystemArchitectureRules {
    for candidate in ["system-architecture.json", ".harness-architecture.json"] {
        let Ok(raw) = read_file(system_root, candidate, MAX_SOURCE_BYTES) else {
            continue;
        };
        let Ok(doc) = parse_json(&raw, MAX_SOURCE_BYTES) else {
            continue;
        };
        let Some(arr) = doc.get("layers").and_then(Value::as_array) else {
            continue;
        };
        let mut layers = Vec::new();
        for item in arr.iter().take(64) {
            let name = string(&item["name"]).trim().to_string();
            if name.is_empty() {
                continue;
            }
            let services = texts(&item["services"]);
            let paths = texts(&item["paths"]);
            let allows = item.get("allows").and_then(|v| v.as_array()).map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            });
            layers.push(ArchitectureLayerRule {
                name,
                services,
                paths,
                allows,
            });
        }
        if !layers.is_empty() {
            return SystemArchitectureRules {
                declared: true,
                source_file: Some(candidate.into()),
                layers,
            };
        }
    }
    SystemArchitectureRules::default()
}

fn path_pattern_matches(pattern: &str, sid: &str, svc_root: &str, system_root: &Path) -> bool {
    let clean = pattern
        .trim_start_matches("./")
        .trim_end_matches("/**")
        .trim_end_matches("/*")
        .trim_end_matches('/');
    if clean.is_empty() {
        return false;
    }
    if clean == sid {
        return true;
    }
    let rel_from_sys = Path::new(svc_root)
        .strip_prefix(system_root)
        .ok()
        .and_then(|p| p.to_str())
        .unwrap_or(svc_root)
        .replace('\\', "/");
    rel_from_sys == clean
        || rel_from_sys.ends_with(&format!("/{clean}"))
        || rel_from_sys.starts_with(&format!("{clean}/"))
}

fn looks_like_test_path(path: &str) -> bool {
    let low = path.to_lowercase();
    low.starts_with("tests/")
        || low.contains("/tests/")
        || low.starts_with("test/")
        || low.contains("/test/")
        || low.ends_with("_test.go")
        || low.ends_with("_test.rs")
        || low.ends_with(".test.ts")
        || low.ends_with(".test.js")
        || low.ends_with(".spec.ts")
        || low.ends_with(".spec.js")
        || low.ends_with("test.php")
}

/// Loads all saved `<phase>-a<attempt>.evidence.json` artifacts for a system run's `execution/` directory.
pub fn load_run_evidence(execution_dir: &Path, state: &Value) -> Result<Vec<DispatchEvidence>> {
    if !execution_dir.is_dir() {
        return Ok(vec![]);
    }
    let mut dispatches = Vec::new();
    for step in items(&state["steps"], MAX_SERVICES + 2)? {
        let phase = string(&step["id"]);
        let max_attempt = step["attempt"].as_u64().unwrap_or(0);
        for attempt in 1..=max_attempt {
            let probe = object!({ "id": phase, "attempt": attempt });
            let name = evidence_name(&probe);
            match read_json(execution_dir, &name, execution::MAX_STATE) {
                Ok(raw) => {
                    if let Ok(parsed) = serde_json::from_value::<DispatchEvidence>(raw) {
                        dispatches.push(parsed);
                    }
                }
                Err(err) if err.kind == infra_core::ErrorKind::NotFound => continue,
                Err(_) => continue,
            }
        }
    }
    Ok(dispatches)
}

/// A path as the page shows and opens it, with `/` between its parts on
/// every system.
fn slashed(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text.into_owned()
    }
}
/// Summarizes per-dispatch [`DispatchEvidence`] records for `systems.get` (`GET /api/system-runs/{id}`),
/// including `cross_service_unopened` reach across downstream contract consumers.
pub fn summarize_run_evidence(
    system: Option<&System>,
    state: &Value,
    dispatches: &[DispatchEvidence],
) -> Value {
    let mut by_step = serde_json::Map::new();
    let mut all_opened = BTreeSet::new();
    let mut all_edited = BTreeMap::<String, String>::new();
    let mut all_git_changed = BTreeMap::<String, EvidenceFileChange>::new();
    let mut all_shell_reads = serde_json::Map::new();
    let mut created_entities = Vec::new();
    let mut file_touches = Vec::new();
    let mut plan_items = Vec::new();
    let mut command_runs = Vec::new();
    let mut graph_asked = false;
    let mut total_steps = 0usize;
    let mut total_searched = 0usize;
    let mut total_commands = 0usize;
    let mut total_helpers = 0usize;
    let mut opened_by_service: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for ev in dispatches {
        if let Ok(val) = serde_json::to_value(ev) {
            by_step.insert(ev.phase.clone(), val);
        }
        let prefix = if ev.service == "__system__" {
            "__system__".to_string()
        } else {
            ev.service.clone()
        };
        let bucket_key = if ev.phase == "verify" {
            "verify".to_string()
        } else {
            ev.service.clone()
        };
        for p in &ev.opened {
            let qualified = qualify_service_path(&prefix, p);
            all_opened.insert(qualified);
            opened_by_service
                .entry(bucket_key.clone())
                .or_default()
                .insert(strip_service_prefix(&prefix, p));
        }
        for (p, event_id) in &ev.shell_reads {
            let qualified = qualify_service_path(&prefix, p);
            all_opened.insert(qualified.clone());
            all_shell_reads.insert(qualified, object!(*event_id));
            opened_by_service
                .entry(bucket_key.clone())
                .or_default()
                .insert(strip_service_prefix(&prefix, p));
        }
        for item in &ev.ledger_edited {
            let qualified = qualify_service_path(&prefix, &item.path);
            all_edited.insert(qualified, item.status.clone());
        }
        for item in &ev.git_changed {
            let qualified = qualify_service_path(&prefix, &item.path);
            all_git_changed.insert(
                qualified.clone(),
                EvidenceFileChange {
                    path: qualified,
                    status: item.status.clone(),
                    from: item.from.clone(),
                },
            );
        }
        graph_asked |= ev.graph_asked;
        total_steps += ev.steps;
        total_searched += ev.searched;
        total_commands += ev.commands;
        total_helpers += ev.helpers;
        created_entities.extend(ev.created_entities.iter().cloned());
        file_touches.extend(ev.file_touches.iter().cloned());
        plan_items.extend(ev.plan_items.iter().cloned());
        command_runs.extend(ev.command_runs.iter().cloned());
    }

    let overlaps = edited_by_several(state, dispatches);

    let mut cross_service_unopened = Vec::new();
    let mut unopened_contract_consumers = Vec::new();
    if let Some(sys) = system {
        let summaries = collect_run_service_summaries(sys, dispatches);
        let graph = SystemGraph::from_system(sys, summaries);
        let mut candidate_services: BTreeSet<String> =
            texts(&state["selected"]).into_iter().collect();
        for ev in dispatches {
            if ev.service != "__system__" && !ev.service.is_empty() {
                candidate_services.insert(ev.service.clone());
            }
        }
        for sid in candidate_services {
            let has_edits = dispatches.iter().any(|ev| {
                (ev.service == sid || access_scope(state) == "all")
                    && (!ev.git_changed.is_empty() || !ev.ledger_edited.is_empty())
            });
            if !has_edits {
                continue;
            }
            let impact = graph.impact_across_services(&CrossServiceImpactQuery {
                service: Some(sid),
                path: None,
                contract: None,
                certain: false,
                opened_by_service: opened_by_service.clone(),
            });
            if let Some(arr) = impact["cross_service_unopened"].as_array() {
                for item in arr {
                    if !cross_service_unopened.contains(item) {
                        cross_service_unopened.push(item.clone());
                    }
                }
            }
            if let Some(arr) = impact["contract_consumers"].as_array() {
                for item in arr {
                    if item["unopened_sources"]
                        .as_array()
                        .is_some_and(|u| !u.is_empty())
                        && !unopened_contract_consumers.contains(item)
                    {
                        unopened_contract_consumers.push(item.clone());
                    }
                }
            }
        }
    }

    object!({
        "dispatches": dispatches,
        "by_step": by_step,
        "opened": all_opened,
        "edited": all_edited.into_iter().map(|(path, status)| object!({ "path": path, "status": status })).collect::<Vec<_>>(),
        "git_changed": all_git_changed.into_values().collect::<Vec<_>>(),
        "shell_opened": all_shell_reads.len(),
        "shell_reads": all_shell_reads,
        "created_entities": created_entities,
        "graph_asked": graph_asked,
        "steps": total_steps,
        "searched": total_searched,
        "commands": total_commands,
        "helpers": total_helpers,
        "file_touches": file_touches,
        "plan_items": plan_items,
        "command_runs": command_runs,
        "unopened_callers": [],
        "unopened_contract_consumers": unopened_contract_consumers,
        "cross_service_unopened": cross_service_unopened,
        "overlaps": overlaps,
    })
}

/// How many files edited by several agents a run tells of.
const OVERLAPS: usize = 24;

/// The files that more than one step of a run edited, each with the steps in
/// the order they ran. A run dispatches its agents one after another, and
/// the Harness runs no two writers on one repository at once, so no two
/// agents write a file at the same moment; a later agent can still change
/// what an earlier one made, and a person reviewing the run should know
/// where. A file is told apart by where it lies, so a file of a monorepo
/// that two services edit is one file. Two attempts of one step are one
/// step.
fn edited_by_several(state: &Value, dispatches: &[DispatchEvidence]) -> Vec<Value> {
    // Where each file lies, and the service and the path in it of the
    // first step that edited it: where that path leaves the service, the
    // file is shown from the folder that holds every root of the run.
    type Inside = Option<(String, String)>;
    type Steps = Vec<(String, String)>;
    let mut files: BTreeMap<PathBuf, (Inside, Steps)> = BTreeMap::new();
    for ev in dispatches {
        let mut seen = BTreeSet::new();
        for item in ev.ledger_edited.iter().chain(&ev.git_changed) {
            // A path may name its service first where a step could write
            // into every service of the run.
            let (service, root, rest) = match item.path.split_once('/') {
                Some((service, rest))
                    if service != ev.service && state["roots"][service].is_string() =>
                {
                    (
                        service,
                        PathBuf::from(string(&state["roots"][service])),
                        rest,
                    )
                }
                _ => (
                    ev.service.as_str(),
                    PathBuf::from(&ev.root),
                    item.path.as_str(),
                ),
            };
            let place = lexical(&root.join(rest));
            if !seen.insert(place.clone()) {
                continue;
            }
            let inside = place
                .strip_prefix(lexical(&root))
                .ok()
                .map(|inside| (service.to_owned(), slashed(inside)));
            let entry = files.entry(place).or_insert_with(|| (inside, Vec::new()));
            if !entry.1.iter().any(|(phase, _)| *phase == ev.phase) {
                entry.1.push((ev.phase.clone(), ev.service.clone()));
            }
        }
    }
    // The folder that holds every root of the run.
    let common = dispatches
        .iter()
        .map(|ev| lexical(Path::new(&ev.root)))
        .reduce(|held, root| {
            held.components()
                .zip(root.components())
                .take_while(|(a, b)| a == b)
                .map(|(part, _)| part.as_os_str())
                .collect()
        })
        .unwrap_or_default();
    let mut several = files
        .into_iter()
        .filter(|(_, (_, steps))| steps.len() > 1)
        .map(|(place, (inside, steps))| {
            let shown = match &inside {
                Some((service, path)) => qualify_service_path(service, path),
                None => slashed(place.strip_prefix(&common).unwrap_or(&place)),
            };
            (shown, inside, steps)
        })
        .collect::<Vec<_>>();
    several.sort_by(|a, b| a.0.cmp(&b.0));
    several
        .into_iter()
        .take(OVERLAPS)
        .map(|(shown, inside, steps)| {
            object!({
                "path": shown,
                "service": inside.as_ref().map(|(service, _)| service.clone()),
                "file": inside.map(|(_, path)| path),
                "steps": steps.iter().map(|(phase, _)| phase.clone()).collect::<Vec<_>>(),
                "services": steps.iter().map(|(_, service)| service.clone()).collect::<Vec<_>>(),
            })
        })
        .collect()
}

/// A path with `.` and `..` worked out, without asking the file system.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn qualify_service_path(service: &str, path: &str) -> String {
    if path.starts_with(&format!("{service}/")) {
        path.to_string()
    } else {
        format!("{service}/{path}")
    }
}

fn strip_service_prefix(service: &str, path: &str) -> String {
    path.strip_prefix(&format!("{service}/"))
        .unwrap_or(path)
        .to_string()
}

/// Collects git worktree changes (scoped accurately to monorepo subfolders as well as standalone
/// repositories), optional run [`DispatchEvidence`], and lightweight declared-source contract
/// references across `__system__` and member services.
pub fn collect_git_service_summaries(
    system: &System,
    run_evidence: &[DispatchEvidence],
) -> BTreeMap<String, ServiceCodeSummary> {
    collect_service_summaries(system, run_evidence, true)
}

/// Like [`collect_git_service_summaries`], but changed files come only from the
/// run's own [`DispatchEvidence`] (`git_changed` + `ledger_edited`): uncommitted
/// files a run did not touch, in any service, are not attributed to it.
pub fn collect_run_service_summaries(
    system: &System,
    run_evidence: &[DispatchEvidence],
) -> BTreeMap<String, ServiceCodeSummary> {
    collect_service_summaries(system, run_evidence, false)
}

fn collect_service_summaries(
    system: &System,
    run_evidence: &[DispatchEvidence],
    include_worktree: bool,
) -> BTreeMap<String, ServiceCodeSummary> {
    let mut summaries = BTreeMap::new();
    let all_roots: Vec<(String, PathBuf, Option<&Value>)> =
        std::iter::once(("__system__".to_string(), system.root.clone(), None))
            .chain(
                system
                    .services
                    .iter()
                    .filter(|(_, svc)| svc.access == "available")
                    .map(|(sid, svc)| (sid.clone(), svc.root.clone(), svc.manifest.as_ref())),
            )
            .collect();

    // Build lookup of all provided contracts across services: `(provider_id, contract_id, kind)`.
    let provided_catalog: Vec<(String, String, String)> = system
        .services
        .iter()
        .flat_map(|(sid, svc)| {
            svc.manifest
                .as_ref()
                .and_then(|m| m["provides"].as_array())
                .into_iter()
                .flatten()
                .map(move |prov| {
                    (
                        sid.clone(),
                        string(&prov["id"]).to_string(),
                        string(&prov["kind"]).to_string(),
                    )
                })
        })
        .collect();

    for (sid, root, manifest) in all_roots {
        let mut summary = ServiceCodeSummary {
            state: "unindexed".into(),
            stack: Some(if sid == "__system__" {
                "system".into()
            } else {
                detect_service_stack(&root)
            }),
            commit: git_head(&root),
            ..Default::default()
        };

        // Git worktree status scoped to `root` (even if `root` is a subfolder of a monorepo!).
        let worktree = if include_worktree {
            git_worktree_changes(&root)
        } else {
            Vec::new()
        };
        for change in worktree {
            summary.dirty = true;
            summary.changed_files.push(ChangedFileEntry {
                is_test: looks_like_test_path(&change.path),
                path: change.path,
                status: change.status,
                from: change.from,
                additions: None,
                deletions: None,
            });
        }

        // Merge any `DispatchEvidence` entries targeting this service.
        for ev in run_evidence {
            if ev.service == sid || (ev.phase == "verify" && sid == "__system__") {
                for p in &ev.opened {
                    summary.opened_files.insert(strip_service_prefix(&sid, p));
                }
                for (p, eid) in &ev.shell_reads {
                    let clean = strip_service_prefix(&sid, p);
                    summary.opened_files.insert(clean.clone());
                    summary.shell_reads.insert(clean, *eid);
                }
                for ch in ev.git_changed.iter().chain(ev.ledger_edited.iter()) {
                    let clean = strip_service_prefix(&sid, &ch.path);
                    if !summary.changed_files.iter().any(|f| f.path == clean) {
                        summary.changed_files.push(ChangedFileEntry {
                            is_test: looks_like_test_path(&clean),
                            path: clean,
                            status: ch.status.clone(),
                            from: ch.from.clone(),
                            additions: None,
                            deletions: None,
                        });
                    }
                }
            } else if ev.opened.iter().any(|p| p.starts_with(&format!("{sid}/"))) {
                for p in &ev.opened {
                    if let Some(rel) = p.strip_prefix(&format!("{sid}/")) {
                        summary.opened_files.insert(rel.to_string());
                    }
                }
            }
        }

        // Lightweight contract token scan across declared code/contract sources for `verified` / `undeclared_in_code`.
        if let Some(passport) = manifest {
            let mut source_paths = BTreeSet::new();
            for entry in passport["sources"].as_array().into_iter().flatten() {
                if matches!(string(&entry["kind"]), "code" | "contract") {
                    source_paths.insert(string(&entry["path"]).to_string());
                }
            }
            for cap in passport["capabilities"].as_array().into_iter().flatten() {
                source_paths.extend(texts(&cap["sources"]));
            }
            for rel_path in source_paths.into_iter().take(40) {
                let Ok(raw) = read_file(&root, &rel_path, MAX_SOURCE_BYTES) else {
                    continue;
                };
                let Ok(content) = std::str::from_utf8(&raw) else {
                    continue;
                };
                if sensitive(content) {
                    continue;
                }
                for (provider_id, cid, ckind) in &provided_catalog {
                    if provider_id == &sid || cid.len() < 4 {
                        continue;
                    }
                    if content.contains(cid.as_str()) {
                        summary.code_refs.push(CodeReference {
                            from_path: rel_path.clone(),
                            from_symbol: None,
                            target_service: provider_id.clone(),
                            contract: Some(cid.clone()),
                            kind: ckind.clone(),
                            certainty: "EXTRACTED".into(),
                            detail: Some(format!("references contract {cid}")),
                        });
                    }
                }
            }
        }

        summaries.insert(sid, summary);
    }
    summaries
}

/// Read-only inspection of published Graphify artifacts in an optional Harness `state_dir`
/// (`sessions.sqlite3` + `graphs/<id>/live/graph.json`) or local `<root>/graphify-out/graph.json`.
/// Never creates or mutates database or filesystem state.
pub fn enrich_summaries_from_graph_store(
    system: &System,
    state_dir: Option<&Path>,
    summaries: &mut BTreeMap<String, ServiceCodeSummary>,
) {
    let mut project_by_root = BTreeMap::<PathBuf, (String, Option<Value>)>::new();
    if let Some(state) = state_dir
        && state.join("sessions.sqlite3").is_file()
        && let Ok(conn) = rusqlite::Connection::open_with_flags(
            state.join("sessions.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        && let Ok(mut stmt) = conn.prepare("SELECT id, path FROM registered_projects")
        && let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
    {
        for row in rows.flatten() {
            let (pid, raw_path) = row;
            let canon = infra_core::fs::canonicalize(Path::new(&raw_path))
                .unwrap_or_else(|_| PathBuf::from(&raw_path));
            let setting = conn
                .query_row(
                    "SELECT value FROM harness_settings WHERE name=?1",
                    [format!("graph.{pid}")],
                    |r| r.get::<_, String>(0),
                )
                .ok()
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
            project_by_root.insert(canon, (pid, setting));
        }
    }

    let roots: Vec<(String, PathBuf)> =
        std::iter::once(("__system__".to_string(), system.root.clone()))
            .chain(
                system
                    .services
                    .iter()
                    .filter(|(_, svc)| svc.access == "available")
                    .map(|(sid, svc)| (sid.clone(), svc.root.clone())),
            )
            .collect();

    for (sid, root) in roots {
        let Some(summary) = summaries.get_mut(&sid) else {
            continue;
        };
        let canon = infra_core::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
        let mut graph_file: Option<PathBuf> = None;
        let mut built_meta: Option<Value> = None;

        if let Some(state) = state_dir {
            if let Some((pid, setting)) = project_by_root.get(&canon) {
                let candidate = state.join("graphs").join(pid).join("live/graph.json");
                if candidate.is_file() {
                    graph_file = Some(candidate);
                    built_meta = setting.as_ref().and_then(|s| s.get("built").cloned());
                }
            }
            if graph_file.is_none() {
                // Members inside a registered parent workspace or monorepo
                // get a synthetic Harness graph id from their canonical root.
                let synth_id = format!(
                    "sys-{}",
                    &infra_core::query::sha256(canon.to_string_lossy().as_bytes())[..16]
                );
                let candidate = state.join("graphs").join(&synth_id).join("live/graph.json");
                if candidate.is_file() {
                    graph_file = Some(candidate);
                    built_meta = rusqlite::Connection::open_with_flags(
                        state.join("sessions.sqlite3"),
                        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
                    )
                    .ok()
                    .and_then(|conn| {
                        conn.query_row(
                            "SELECT value FROM harness_settings WHERE name=?1",
                            [format!("graph.{synth_id}")],
                            |r| r.get::<_, String>(0),
                        )
                        .ok()
                    })
                    .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                    .and_then(|s| s.get("built").cloned());
                }
            }
            if graph_file.is_none() {
                let direct = state.join("graphs").join(&sid).join("live/graph.json");
                if direct.is_file() {
                    graph_file = Some(direct);
                }
            }
        }
        if graph_file.is_none() {
            let local_graph = root.join("graphify-out/graph.json");
            if local_graph.is_file() {
                graph_file = Some(local_graph);
            }
        }

        let Some(path) = graph_file else {
            continue;
        };

        let head = summary.commit.clone();
        if let Some(built) = built_meta.filter(Value::is_object) {
            let built_nodes = built["nodes"].as_u64().unwrap_or(0) as usize;
            let built_edges = built["edges"].as_u64().unwrap_or(0) as usize;
            let built_commit = built["commit"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| head.clone());
            let stale =
                summary.dirty || (built_commit.is_some() && head.is_some() && built_commit != head);
            summary.nodes = built_nodes;
            summary.edges = built_edges;
            summary.commit = built_commit;
            summary.stale = stale;
            summary.state = if stale {
                "stale".into()
            } else {
                "ready".into()
            };
            continue;
        }

        if let Ok(meta) = std::fs::metadata(&path)
            && meta.is_file()
        {
            let parsed = read_matching_live_summary(&path, &meta).or_else(|| {
                (meta.len() <= 64 * 1024 * 1024)
                    .then(|| stream_graph_summary(&path))
                    .flatten()
            });
            if let Some((n_count, e_count, g_commit)) = parsed {
                let stale =
                    summary.dirty || (g_commit.is_some() && head.is_some() && g_commit != head);
                summary.nodes = n_count;
                summary.edges = e_count;
                if let Some(c) = g_commit {
                    summary.commit = Some(c);
                }
                summary.stale = stale;
                summary.state = if stale {
                    "stale".into()
                } else {
                    "ready".into()
                };
            }
        }
    }
}

#[derive(Deserialize)]
struct LiveGraphSummary {
    nodes: usize,
    edges: usize,
    #[serde(default)]
    commit: Option<String>,
    graph_len: u64,
    #[serde(default)]
    graph_mtime_secs: Option<u64>,
    #[serde(default)]
    graph_mtime_nanos: Option<u32>,
}

fn read_matching_live_summary(
    graph_path: &Path,
    graph_meta: &std::fs::Metadata,
) -> Option<(usize, usize, Option<String>)> {
    let summary_path = graph_path.with_file_name("summary.json");
    let sum_meta = std::fs::symlink_metadata(&summary_path).ok()?;
    if !sum_meta.is_file() || sum_meta.len() > 64 * 1024 {
        return None;
    }
    let file = std::fs::File::open(&summary_path).ok()?;
    let summary: LiveGraphSummary = serde_json::from_reader(std::io::BufReader::new(file)).ok()?;
    if summary.graph_len != graph_meta.len() {
        return None;
    }
    let modified = graph_meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    if summary.graph_mtime_secs != Some(modified.as_secs())
        || summary.graph_mtime_nanos != Some(modified.subsec_nanos())
    {
        return None;
    }
    Some((
        summary.nodes,
        summary.edges,
        summary.commit.filter(|s| !s.is_empty()),
    ))
}

fn stream_graph_summary(path: &Path) -> Option<(usize, usize, Option<String>)> {
    use serde::de::{Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
    use std::fmt;

    struct OptArrayLen(Option<usize>);
    impl<'de> Deserialize<'de> for OptArrayLen {
        fn deserialize<D: Deserializer<'de>>(
            deserializer: D,
        ) -> std::result::Result<Self, D::Error> {
            struct LenVisitor;
            impl<'de> Visitor<'de> for LenVisitor {
                type Value = OptArrayLen;
                fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    f.write_str("an optional JSON array")
                }
                fn visit_seq<A: SeqAccess<'de>>(
                    self,
                    mut seq: A,
                ) -> std::result::Result<OptArrayLen, A::Error> {
                    let mut count = 0usize;
                    while seq.next_element::<IgnoredAny>()?.is_some() {
                        count += 1;
                    }
                    Ok(OptArrayLen(Some(count)))
                }
                fn visit_map<A: MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> std::result::Result<OptArrayLen, A::Error> {
                    while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                    Ok(OptArrayLen(None))
                }
                fn visit_unit<E>(self) -> std::result::Result<OptArrayLen, E> {
                    Ok(OptArrayLen(None))
                }
                fn visit_bool<E>(self, _: bool) -> std::result::Result<OptArrayLen, E> {
                    Ok(OptArrayLen(None))
                }
                fn visit_i64<E>(self, _: i64) -> std::result::Result<OptArrayLen, E> {
                    Ok(OptArrayLen(None))
                }
                fn visit_u64<E>(self, _: u64) -> std::result::Result<OptArrayLen, E> {
                    Ok(OptArrayLen(None))
                }
                fn visit_f64<E>(self, _: f64) -> std::result::Result<OptArrayLen, E> {
                    Ok(OptArrayLen(None))
                }
                fn visit_str<E>(self, _: &str) -> std::result::Result<OptArrayLen, E> {
                    Ok(OptArrayLen(None))
                }
            }
            deserializer.deserialize_any(LenVisitor)
        }
    }

    struct OptStr(Option<String>);
    impl<'de> Deserialize<'de> for OptStr {
        fn deserialize<D: Deserializer<'de>>(
            deserializer: D,
        ) -> std::result::Result<Self, D::Error> {
            struct StrVisitor;
            impl<'de> Visitor<'de> for StrVisitor {
                type Value = OptStr;
                fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    f.write_str("an optional JSON string")
                }
                fn visit_str<E>(self, v: &str) -> std::result::Result<OptStr, E> {
                    Ok(OptStr(Some(v.to_owned())))
                }
                fn visit_string<E>(self, v: String) -> std::result::Result<OptStr, E> {
                    Ok(OptStr(Some(v)))
                }
                fn visit_unit<E>(self) -> std::result::Result<OptStr, E> {
                    Ok(OptStr(None))
                }
                fn visit_bool<E>(self, _: bool) -> std::result::Result<OptStr, E> {
                    Ok(OptStr(None))
                }
                fn visit_i64<E>(self, _: i64) -> std::result::Result<OptStr, E> {
                    Ok(OptStr(None))
                }
                fn visit_u64<E>(self, _: u64) -> std::result::Result<OptStr, E> {
                    Ok(OptStr(None))
                }
                fn visit_f64<E>(self, _: f64) -> std::result::Result<OptStr, E> {
                    Ok(OptStr(None))
                }
                fn visit_seq<A: SeqAccess<'de>>(
                    self,
                    mut seq: A,
                ) -> std::result::Result<OptStr, A::Error> {
                    while seq.next_element::<IgnoredAny>()?.is_some() {}
                    Ok(OptStr(None))
                }
                fn visit_map<A: MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> std::result::Result<OptStr, A::Error> {
                    while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                    Ok(OptStr(None))
                }
            }
            deserializer.deserialize_any(StrVisitor)
        }
    }

    struct MetaCommit(Option<String>);
    impl<'de> Deserialize<'de> for MetaCommit {
        fn deserialize<D: Deserializer<'de>>(
            deserializer: D,
        ) -> std::result::Result<Self, D::Error> {
            struct MetaVisitor;
            impl<'de> Visitor<'de> for MetaVisitor {
                type Value = MetaCommit;
                fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    f.write_str("an optional meta object")
                }
                fn visit_map<A: MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> std::result::Result<MetaCommit, A::Error> {
                    let mut commit = None;
                    while let Some(key) = map.next_key::<String>()? {
                        if key == "commit" {
                            commit = map.next_value::<OptStr>()?.0;
                        } else {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                    Ok(MetaCommit(commit))
                }
                fn visit_unit<E>(self) -> std::result::Result<MetaCommit, E> {
                    Ok(MetaCommit(None))
                }
                fn visit_bool<E>(self, _: bool) -> std::result::Result<MetaCommit, E> {
                    Ok(MetaCommit(None))
                }
                fn visit_i64<E>(self, _: i64) -> std::result::Result<MetaCommit, E> {
                    Ok(MetaCommit(None))
                }
                fn visit_u64<E>(self, _: u64) -> std::result::Result<MetaCommit, E> {
                    Ok(MetaCommit(None))
                }
                fn visit_f64<E>(self, _: f64) -> std::result::Result<MetaCommit, E> {
                    Ok(MetaCommit(None))
                }
                fn visit_str<E>(self, _: &str) -> std::result::Result<MetaCommit, E> {
                    Ok(MetaCommit(None))
                }
                fn visit_seq<A: SeqAccess<'de>>(
                    self,
                    mut seq: A,
                ) -> std::result::Result<MetaCommit, A::Error> {
                    while seq.next_element::<IgnoredAny>()?.is_some() {}
                    Ok(MetaCommit(None))
                }
            }
            deserializer.deserialize_any(MetaVisitor)
        }
    }

    struct GraphStreamSummary {
        nodes: usize,
        edges: usize,
        commit: Option<String>,
    }
    impl<'de> Deserialize<'de> for GraphStreamSummary {
        fn deserialize<D: Deserializer<'de>>(
            deserializer: D,
        ) -> std::result::Result<Self, D::Error> {
            struct GraphVisitor;
            impl<'de> Visitor<'de> for GraphVisitor {
                type Value = GraphStreamSummary;
                fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    f.write_str("a graph summary object")
                }
                fn visit_map<A: MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> std::result::Result<GraphStreamSummary, A::Error> {
                    let mut nodes = None;
                    let mut links = None;
                    let mut edges = None;
                    let mut commit = None;
                    let mut meta_commit = None;
                    while let Some(key) = map.next_key::<String>()? {
                        match key.as_str() {
                            "nodes" => nodes = map.next_value::<OptArrayLen>()?.0,
                            "links" => links = map.next_value::<OptArrayLen>()?.0,
                            "edges" => edges = map.next_value::<OptArrayLen>()?.0,
                            "commit" => commit = map.next_value::<OptStr>()?.0,
                            "meta" => meta_commit = map.next_value::<MetaCommit>()?.0,
                            _ => {
                                map.next_value::<IgnoredAny>()?;
                            }
                        }
                    }
                    let final_commit = commit
                        .filter(|s| !s.is_empty())
                        .or_else(|| meta_commit.filter(|s| !s.is_empty()));
                    Ok(GraphStreamSummary {
                        nodes: nodes.unwrap_or(0),
                        edges: links.or(edges).unwrap_or(0),
                        commit: final_commit,
                    })
                }
            }
            deserializer.deserialize_map(GraphVisitor)
        }
    }

    let file = std::fs::File::open(path).ok()?;
    let reader = std::io::BufReader::with_capacity(64 * 1024, file);
    let mut de = serde_json::Deserializer::from_reader(reader);
    let summary = GraphStreamSummary::deserialize(&mut de).ok()?;
    de.end().ok()?;
    Some((summary.nodes, summary.edges, summary.commit))
}

/// Builds a comprehensive cross-service graph status report for `infra-engine system graph status`.
pub fn system_graph_status(
    system: &System,
    summaries: &BTreeMap<String, ServiceCodeSummary>,
    graph: &SystemGraph,
) -> Value {
    let mut services_map = serde_json::Map::new();
    let mut ready_count = 0usize;
    let mut dirty_count = 0usize;
    let mut available_services = 0usize;

    if let Some(sys_summary) = summaries.get("__system__") {
        if matches!(sys_summary.state.as_str(), "ready" | "stale") {
            ready_count += 1;
        }
        if sys_summary.dirty {
            dirty_count += 1;
        }
        services_map.insert(
            "__system__".into(),
            object!({
                "id": "__system__",
                "kind": "system",
                "access": "available",
                "root": system.root.to_string_lossy(),
                "stack": sys_summary.stack.clone().unwrap_or_else(|| "system".into()),
                "head": sys_summary.commit,
                "dirty": sys_summary.dirty,
                "changed_files": sys_summary.changed_files.len(),
                "state": sys_summary.state,
                "nodes": sys_summary.nodes,
                "edges": sys_summary.edges,
                "commit": sys_summary.commit,
                "stale": sys_summary.stale,
                "shared_sources": system.config_data["shared_sources"].as_array().map_or(0, Vec::len),
            }),
        );
    }

    for (sid, svc) in &system.services {
        if svc.access == "available" {
            available_services += 1;
        }
        let summary = summaries.get(sid);
        let state = if svc.access != "available" {
            "unavailable"
        } else {
            summary.map_or("unindexed", |s| s.state.as_str())
        };
        if matches!(state, "ready" | "stale") {
            ready_count += 1;
        }
        let dirty = summary.is_some_and(|s| s.dirty);
        if dirty {
            dirty_count += 1;
        }
        let passport = svc.manifest.as_ref();
        services_map.insert(
            sid.clone(),
            object!({
                "id": sid,
                "kind": "service",
                "access": svc.access,
                "root": (svc.access == "available").then(|| svc.root.to_string_lossy().into_owned()),
                "stack": summary.and_then(|s| s.stack.clone()),
                "head": if svc.access == "available" { git_head(&svc.root) } else { None },
                "dirty": dirty,
                "changed_files": summary.map_or(0, |s| s.changed_files.len()),
                "state": state,
                "nodes": summary.map_or(0, |s| s.nodes),
                "edges": summary.map_or(0, |s| s.edges),
                "commit": summary.and_then(|s| s.commit.clone()),
                "stale": summary.is_some_and(|s| s.stale),
                "relationships_complete": passport.is_some_and(|m| m["relationships_complete"] == true),
                "capabilities": passport.and_then(|m| m["capabilities"].as_array()).map_or(0, Vec::len),
                "provides": passport.and_then(|m| m["provides"].as_array()).map_or(0, Vec::len),
                "consumes": passport.and_then(|m| m["consumes"].as_array()).map_or(0, Vec::len),
                "sources": passport.and_then(|m| m["sources"].as_array()).map_or(0, Vec::len),
            }),
        );
    }

    let full_json = graph.to_json();
    object!({
        "kind": "system_graph_status",
        "system": string(&system.config_data["name"]),
        "root": system.root.to_string_lossy(),
        "total_services": system.services.len(),
        "available_services": available_services,
        "ready_count": ready_count,
        "dirty_count": dirty_count,
        "contract_risk_count": full_json["contract_risks"].as_array().map_or(0, Vec::len),
        "cycle_count": full_json["cycles"].as_array().map_or(0, Vec::len),
        "services": services_map,
        "warnings": system.warnings,
    })
}

/// Returns git working-tree changes strictly inside `root` (handling monorepo subfolders cleanly).
pub fn git_worktree_changes(root: &Path) -> Vec<EvidenceFileChange> {
    let Some((toplevel, prefix)) = git_toplevel_and_prefix(root) else {
        return vec![];
    };
    let mut args =
        infra_core::git::strings(&["status", "--porcelain=v1", "-z", "--untracked-files=all"]);
    if !prefix.is_empty() {
        args.push("--".into());
        args.push(prefix.clone().into());
    }
    let Ok(output) =
        infra_core::git::run(&toplevel, &args, None, Duration::from_secs(5), 512 * 1024)
    else {
        return vec![];
    };
    if output.code != 0 {
        return vec![];
    }
    parse_porcelain_z(&output.stdout, &prefix)
}

/// Computes git changes inside `root` between an optional `base_head` and the current checkout state.
pub fn git_changes_since(root: &Path, base_head: Option<&str>) -> Vec<EvidenceFileChange> {
    let mut by_path = BTreeMap::<String, EvidenceFileChange>::new();
    if let (Some(base), Some((toplevel, prefix))) = (base_head, git_toplevel_and_prefix(root)) {
        let mut args = infra_core::git::strings(&["diff", "--name-status", "-z", base]);
        if !prefix.is_empty() {
            args.push("--".into());
            args.push(prefix.clone().into());
        }
        if let Ok(out) =
            infra_core::git::run(&toplevel, &args, None, Duration::from_secs(5), 512 * 1024)
            && out.code == 0
        {
            for item in parse_name_status_z(&out.stdout, &prefix) {
                by_path.insert(item.path.clone(), item);
            }
        }
    }
    for item in git_worktree_changes(root) {
        by_path.entry(item.path.clone()).or_insert(item);
    }
    by_path.into_values().collect()
}

fn git_toplevel_and_prefix(root: &Path) -> Option<(PathBuf, String)> {
    let canonical_root = infra_core::fs::canonicalize(root).ok()?;
    let out = infra_core::git::run(
        &canonical_root,
        &infra_core::git::strings(&["rev-parse", "--show-toplevel"]),
        None,
        Duration::from_secs(5),
        16384,
    )
    .ok()?;
    if out.code != 0 {
        return None;
    }
    let raw_top = String::from_utf8(out.stdout).ok()?;
    let toplevel = infra_core::fs::canonicalize(Path::new(raw_top.trim())).ok()?;
    let prefix = canonical_root
        .strip_prefix(&toplevel)
        .ok()?
        .to_str()?
        .replace('\\', "/")
        .trim_matches('/')
        .to_string();
    Some((toplevel, prefix))
}

fn strip_repo_prefix(repo_path: &str, prefix: &str) -> Option<String> {
    let normalized = repo_path.replace('\\', "/");
    if prefix.is_empty() {
        return (!normalized.is_empty()).then_some(normalized);
    }
    normalized
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|rest| !rest.is_empty())
        .map(str::to_string)
}

/// Task, event and memory records the runtime writes itself (Brain dispatch
/// records, Memory Bank, security scratch) are not a worker's source edits.
fn is_runtime_record_path(rel: &str) -> bool {
    ["project-brain/", "memory-bank/", ".agentic-security/"]
        .iter()
        .any(|prefix| rel.starts_with(prefix))
}

fn parse_porcelain_z(bytes: &[u8], prefix: &str) -> Vec<EvidenceFileChange> {
    let parts = bytes
        .split(|b| *b == 0)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let mut out = Vec::new();
    let mut idx = 0usize;
    while idx < parts.len() {
        let entry = String::from_utf8_lossy(parts[idx]);
        if entry.len() < 4 {
            idx += 1;
            continue;
        }
        let xy = &entry[..2];
        let raw_path = &entry[3..];
        let status = match xy {
            "??" => "untracked",
            s if s.contains('R') => "renamed",
            s if s.contains('A') => "added",
            s if s.contains('D') => "deleted",
            _ => "modified",
        };
        let from = if xy.starts_with('R') || xy.starts_with('C') {
            idx += 1;
            parts
                .get(idx)
                .map(|src| String::from_utf8_lossy(src).into_owned())
                .and_then(|src| strip_repo_prefix(&src, prefix))
        } else {
            None
        };
        if let Some(rel) = strip_repo_prefix(raw_path, prefix) {
            // Skip internal local runtime folders.
            if !is_runtime_record_path(&rel) {
                out.push(EvidenceFileChange {
                    path: rel,
                    status: status.into(),
                    from,
                });
            }
        }
        idx += 1;
    }
    out
}

fn parse_name_status_z(bytes: &[u8], prefix: &str) -> Vec<EvidenceFileChange> {
    let parts = bytes
        .split(|b| *b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect::<Vec<_>>();
    let mut out = Vec::new();
    let mut idx = 0usize;
    while idx < parts.len() {
        let code = &parts[idx];
        let Some(first_path) = parts.get(idx + 1) else {
            break;
        };
        if code.starts_with('R') || code.starts_with('C') {
            let Some(second_path) = parts.get(idx + 2) else {
                break;
            };
            if let Some(rel) = strip_repo_prefix(second_path, prefix)
                && !is_runtime_record_path(&rel)
            {
                out.push(EvidenceFileChange {
                    path: rel,
                    status: "renamed".into(),
                    from: strip_repo_prefix(first_path, prefix),
                });
            }
            idx += 3;
        } else {
            let status = match code.as_str() {
                "A" => "added",
                "D" => "deleted",
                _ => "modified",
            };
            if let Some(rel) = strip_repo_prefix(first_path, prefix)
                && !is_runtime_record_path(&rel)
            {
                out.push(EvidenceFileChange {
                    path: rel,
                    status: status.into(),
                    from: None,
                });
            }
            idx += 2;
        }
    }
    out
}
