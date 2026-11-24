//! Embedded Omnigraph v0.15.0 schema (`.pg`), stored queries (`.gq`),
//! `cluster.yaml`, and Cedar `policy.yaml` bundles for Harness context graphs.

use crate::{HarnessError, Result};
use std::path::Path;

/// Default graph id used by Harness. Must match `^[a-zA-Z0-9-]{1,64}$` (no underscores).
pub const HARNESS_GRAPH_ID: &str = "harness-context";

/// Dedicated Omnigraph branch holding synced canonical `Project Brain` and `Memory Bank` knowledge.
pub const CANONICAL_KNOWLEDGE_BRANCH: &str = "knowledge/canonical";

/// Pinned Omnigraph release tag supported by the local/Docker sidecar installer.
pub const OMNIGRAPH_VERSION: &str = "v0.15.0";

/// Official Linux x86_64 release tarball containing `omnigraph` and `omnigraph-server`.
pub const OMNIGRAPH_RELEASE_URL_LINUX_X86_64: &str = "https://github.com/ModernRelay/omnigraph/releases/download/v0.15.0/omnigraph-linux-x86_64.tar.gz";

/// Official Docker image for `omnigraph-server`.
pub const OMNIGRAPH_DOCKER_IMAGE: &str = "ghcr.io/modernrelay/omnigraph-server:v0.15.0";

/// Row limit baked into `QUERY_RECENT_ATOMS` (matches Studio max page size).
pub(crate) const RECENT_ATOMS_LIMIT: usize = 1000;

/// Validated `.pg` schema for Harness sessions, context atoms, files, symbols, and turn snapshots.
pub const HARNESS_CONTEXT_PG: &str = r#"// Harness shared session context graph schema (Omnigraph v0.15.0)
node Session {
    session_id: String @key
    project_id: String @index
    title: String @index
    branch: String
    provider: String
    workflow: String
    status: String @index
    goal: String?
    created_at: DateTime
    updated_at: DateTime
}

node ContextAtom {
    atom_id: String @key
    project_id: String @index
    kind: enum(finding, decision, warning, contract, insight, artifact, question)
    title: String @index
    summary: String @index
    body: String @index
    tags: [String]?
    source_branch: String @index
    source_session_id: String? @index
    confidence: F64
    pin_priority: I32
    verified: Bool
    origin_layer: String?
    authority: String?
    privacy: String?
    brain_record_id: String?
    memory_id: String?
    record_revision: I64?
    created_at: DateTime
    updated_at: DateTime
}

node FileRef {
    path: String @key
    project_id: String @index
    language: String?
    last_touched_at: DateTime
}

node SymbolRef {
    fqn: String @key
    project_id: String @index
    file_path: String @index
    kind: String
    name: String @index
}

node TurnSnapshot {
    snapshot_id: String @key
    session_id: String @index
    turn_index: I32
    summary: String @index
    created_at: DateTime
}

edge PublishedBy: ContextAtom -> Session {
    role: String?
    published_at: DateTime
    @key(@src, @dst)
}

edge ConsumedBy: ContextAtom -> Session {
    consumed_at: DateTime
    injection_mode: String?
    relevance: F64?
    @key(@src, @dst)
}

edge TouchesFile: ContextAtom -> FileRef {
    line_start: I32?
    line_end: I32?
    access: String?
    @key(@src, @dst)
}

edge MentionsSymbol: ContextAtom -> SymbolRef {
    role: String?
    @key(@src, @dst)
}

edge RelatesTo: ContextAtom -> ContextAtom {
    relation: enum(supports, contradicts, supersedes, depends_on, refines)
    note: String?
    confidence: F64?
    created_at: DateTime
    @key(@src, @dst, relation)
}

edge DerivedFrom: ContextAtom -> TurnSnapshot {
    recorded_at: DateTime
    @key(@src, @dst)
}

edge SessionFollows: Session -> Session {
    relation: String
    created_at: DateTime
    @key(@src, @dst)
}
"#;

/// Stored query: hybrid BM25 search across `ContextAtom.title` and `ContextAtom.body`.
pub const QUERY_CONTEXT_SEARCH: &str = r#"query context_search($q: String, $project_id: String) {
    match {
        $a: ContextAtom { project_id: $project_id }
    }
    return {
        $a.atom_id as atom_id,
        $a.kind as kind,
        $a.title as title,
        $a.summary as summary,
        $a.body as body,
        $a.tags as tags,
        $a.source_branch as source_branch,
        $a.source_session_id as source_session_id,
        $a.confidence as confidence,
        $a.pin_priority as pin_priority,
        $a.verified as verified,
        $a.origin_layer as origin_layer,
        $a.authority as authority,
        $a.privacy as privacy,
        $a.brain_record_id as brain_record_id,
        $a.memory_id as memory_id,
        $a.record_revision as record_revision,
        $a.created_at as created_at,
        $a.updated_at as updated_at,
        rrf(bm25($a.title, $q), bm25($a.body, $q)) as score
    }
    order { rrf(bm25($a.title, $q), bm25($a.body, $q)) }
    limit 100
}"#;

/// Inline query pushing `$kind` predicate directly into BM25 search (`ContextAtom.kind = $kind`).
pub(crate) const GQ_CONTEXT_SEARCH_BY_KIND: &str = r#"query context_search_by_kind($q: String, $project_id: String, $kind: String) {
    match {
        $a: ContextAtom { project_id: $project_id }
        $a.kind = $kind
    }
    return {
        $a.atom_id as atom_id,
        $a.kind as kind,
        $a.title as title,
        $a.summary as summary,
        $a.body as body,
        $a.tags as tags,
        $a.source_branch as source_branch,
        $a.source_session_id as source_session_id,
        $a.confidence as confidence,
        $a.pin_priority as pin_priority,
        $a.verified as verified,
        $a.origin_layer as origin_layer,
        $a.authority as authority,
        $a.privacy as privacy,
        $a.brain_record_id as brain_record_id,
        $a.memory_id as memory_id,
        $a.record_revision as record_revision,
        $a.created_at as created_at,
        $a.updated_at as updated_at,
        rrf(bm25($a.title, $q), bm25($a.body, $q)) as score
    }
    order { rrf(bm25($a.title, $q), bm25($a.body, $q)) }
    limit 100
}"#;

/// Stored query: atoms published or consumed by a specific session.
pub const QUERY_SESSION_NEIGHBORS: &str = r#"query session_neighbors($session_id: String) {
    match {
        $s: Session { session_id: $session_id }
        $a (publishedBy | consumedBy) $s
    }
    return {
        $a.atom_id as atom_id,
        $a.kind as kind,
        $a.title as title,
        $a.summary as summary,
        $a.body as body,
        $a.source_branch as source_branch,
        $a.confidence as confidence,
        $a.pin_priority as pin_priority,
        $a.verified as verified,
        $a.origin_layer as origin_layer,
        $a.authority as authority,
        $a.privacy as privacy,
        $a.brain_record_id as brain_record_id,
        $a.memory_id as memory_id,
        $a.record_revision as record_revision,
        $a.updated_at as updated_at
    }
    order { $a.pin_priority desc, $a.updated_at desc }
    limit 50
}"#;

/// Stored query: context atoms linked to a file path.
pub const QUERY_FILE_CONTEXT: &str = r#"query file_context($path: String) {
    match {
        $f: FileRef { path: $path }
        $a touchesFile $f
    }
    return {
        $a.atom_id as atom_id,
        $a.kind as kind,
        $a.title as title,
        $a.summary as summary,
        $a.body as body,
        $a.source_branch as source_branch,
        $a.confidence as confidence,
        $a.pin_priority as pin_priority,
        $a.verified as verified,
        $a.origin_layer as origin_layer,
        $a.authority as authority,
        $a.privacy as privacy,
        $a.brain_record_id as brain_record_id,
        $a.memory_id as memory_id,
        $a.record_revision as record_revision,
        $a.updated_at as updated_at
    }
    order { $a.pin_priority desc, $a.updated_at desc }
    limit 50
}"#;

/// Stored query: highest-priority and most recent context atoms for a project.
pub const QUERY_RECENT_ATOMS: &str = r#"query recent_atoms($project_id: String) {
    match {
        $a: ContextAtom { project_id: $project_id }
    }
    return {
        $a.atom_id as atom_id,
        $a.kind as kind,
        $a.title as title,
        $a.summary as summary,
        $a.body as body,
        $a.tags as tags,
        $a.source_branch as source_branch,
        $a.source_session_id as source_session_id,
        $a.confidence as confidence,
        $a.pin_priority as pin_priority,
        $a.verified as verified,
        $a.origin_layer as origin_layer,
        $a.authority as authority,
        $a.privacy as privacy,
        $a.brain_record_id as brain_record_id,
        $a.memory_id as memory_id,
        $a.record_revision as record_revision,
        $a.created_at as created_at,
        $a.updated_at as updated_at
    }
    order { $a.pin_priority desc, $a.updated_at desc }
    limit 1000
}"#;

/// Stored query: `RelatesTo` edges marked as `contradicts` within a project.
pub const QUERY_CONTRADICTIONS: &str = r#"query contradictions($project_id: String) {
    match {
        $a: ContextAtom { project_id: $project_id }
        $a $r:relatesTo $b
        $r.relation = "contradicts"
    }
    return {
        $a.atom_id as source_id,
        $a.title as source_title,
        $b.atom_id as target_id,
        $b.title as target_title,
        $r.note as note,
        $r.confidence as confidence,
        $r.created_at as created_at
    }
    order { $r.created_at desc }
    limit 50
}"#;

/// Combined `.gq` file written to `queries/harness_context.gq` in cluster deployments.
pub const HARNESS_STORED_QUERIES_GQ: &str = concat!(
    "// Stored queries for Harness Context Graph\n\n",
    "query context_search($q: String, $project_id: String) {\n",
    "    match {\n",
    "        $a: ContextAtom { project_id: $project_id }\n",
    "    }\n",
    "    return {\n",
    "        $a.atom_id as atom_id,\n",
    "        $a.kind as kind,\n",
    "        $a.title as title,\n",
    "        $a.summary as summary,\n",
    "        $a.body as body,\n",
    "        $a.tags as tags,\n",
    "        $a.source_branch as source_branch,\n",
    "        $a.source_session_id as source_session_id,\n",
    "        $a.confidence as confidence,\n",
    "        $a.pin_priority as pin_priority,\n",
    "        $a.verified as verified,\n",
    "        $a.origin_layer as origin_layer,\n",
    "        $a.authority as authority,\n",
    "        $a.privacy as privacy,\n",
    "        $a.brain_record_id as brain_record_id,\n",
    "        $a.memory_id as memory_id,\n",
    "        $a.record_revision as record_revision,\n",
    "        $a.created_at as created_at,\n",
    "        $a.updated_at as updated_at,\n",
    "        rrf(bm25($a.title, $q), bm25($a.body, $q)) as score\n",
    "    }\n",
    "    order { rrf(bm25($a.title, $q), bm25($a.body, $q)) }\n",
    "    limit 100\n",
    "}\n\n",
    "query session_neighbors($session_id: String) {\n",
    "    match {\n",
    "        $s: Session { session_id: $session_id }\n",
    "        $a (publishedBy | consumedBy) $s\n",
    "    }\n",
    "    return {\n",
    "        $a.atom_id as atom_id,\n",
    "        $a.kind as kind,\n",
    "        $a.title as title,\n",
    "        $a.summary as summary,\n",
    "        $a.body as body,\n",
    "        $a.source_branch as source_branch,\n",
    "        $a.confidence as confidence,\n",
    "        $a.pin_priority as pin_priority,\n",
    "        $a.verified as verified,\n",
    "        $a.origin_layer as origin_layer,\n",
    "        $a.authority as authority,\n",
    "        $a.privacy as privacy,\n",
    "        $a.brain_record_id as brain_record_id,\n",
    "        $a.memory_id as memory_id,\n",
    "        $a.record_revision as record_revision,\n",
    "        $a.updated_at as updated_at\n",
    "    }\n",
    "    order { $a.pin_priority desc, $a.updated_at desc }\n",
    "    limit 50\n",
    "}\n\n",
    "query file_context($path: String) {\n",
    "    match {\n",
    "        $f: FileRef { path: $path }\n",
    "        $a touchesFile $f\n",
    "    }\n",
    "    return {\n",
    "        $a.atom_id as atom_id,\n",
    "        $a.kind as kind,\n",
    "        $a.title as title,\n",
    "        $a.summary as summary,\n",
    "        $a.body as body,\n",
    "        $a.source_branch as source_branch,\n",
    "        $a.confidence as confidence,\n",
    "        $a.pin_priority as pin_priority,\n",
    "        $a.verified as verified,\n",
    "        $a.origin_layer as origin_layer,\n",
    "        $a.authority as authority,\n",
    "        $a.privacy as privacy,\n",
    "        $a.brain_record_id as brain_record_id,\n",
    "        $a.memory_id as memory_id,\n",
    "        $a.record_revision as record_revision,\n",
    "        $a.updated_at as updated_at\n",
    "    }\n",
    "    order { $a.pin_priority desc, $a.updated_at desc }\n",
    "    limit 50\n",
    "}\n\n",
    "query recent_atoms($project_id: String) {\n",
    "    match {\n",
    "        $a: ContextAtom { project_id: $project_id }\n",
    "    }\n",
    "    return {\n",
    "        $a.atom_id as atom_id,\n",
    "        $a.kind as kind,\n",
    "        $a.title as title,\n",
    "        $a.summary as summary,\n",
    "        $a.body as body,\n",
    "        $a.tags as tags,\n",
    "        $a.source_branch as source_branch,\n",
    "        $a.source_session_id as source_session_id,\n",
    "        $a.confidence as confidence,\n",
    "        $a.pin_priority as pin_priority,\n",
    "        $a.verified as verified,\n",
    "        $a.origin_layer as origin_layer,\n",
    "        $a.authority as authority,\n",
    "        $a.privacy as privacy,\n",
    "        $a.brain_record_id as brain_record_id,\n",
    "        $a.memory_id as memory_id,\n",
    "        $a.record_revision as record_revision,\n",
    "        $a.created_at as created_at,\n",
    "        $a.updated_at as updated_at\n",
    "    }\n",
    "    order { $a.pin_priority desc, $a.updated_at desc }\n",
    "    limit 1000\n",
    "}\n\n",
    "query contradictions($project_id: String) {\n",
    "    match {\n",
    "        $a: ContextAtom { project_id: $project_id }\n",
    "        $a $r:relatesTo $b\n",
    "        $r.relation = \"contradicts\"\n",
    "    }\n",
    "    return {\n",
    "        $a.atom_id as source_id,\n",
    "        $a.title as source_title,\n",
    "        $b.atom_id as target_id,\n",
    "        $b.title as target_title,\n",
    "        $r.note as note,\n",
    "        $r.confidence as confidence,\n",
    "        $r.created_at as created_at\n",
    "    }\n",
    "    order { $r.created_at desc }\n",
    "    limit 50\n",
    "}\n"
);

/// Metadata for each embedded stored `.gq` query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoredQueryDef {
    pub name: &'static str,
    pub source: &'static str,
    pub description: &'static str,
}

/// All five stored queries registered by Harness in `harness_context.gq`.
pub const STORED_QUERIES: &[StoredQueryDef] = &[
    StoredQueryDef {
        name: "context_search",
        source: QUERY_CONTEXT_SEARCH,
        description: "Hybrid BM25 search across ContextAtom title and body within a project.",
    },
    StoredQueryDef {
        name: "session_neighbors",
        source: QUERY_SESSION_NEIGHBORS,
        description: "Atoms published by or consumed by a specific Harness session.",
    },
    StoredQueryDef {
        name: "file_context",
        source: QUERY_FILE_CONTEXT,
        description: "Context atoms referencing a given file path.",
    },
    StoredQueryDef {
        name: "recent_atoms",
        source: QUERY_RECENT_ATOMS,
        description: "Highest-priority and most recently updated context atoms for a project.",
    },
    StoredQueryDef {
        name: "contradictions",
        source: QUERY_CONTRADICTIONS,
        description: "Context atom pairs connected by a `contradicts` edge.",
    },
];

// Parameterized inline GQ read/write queries executed over `POST /query` and `POST /mutate`.
pub(crate) const GQ_LIST_SESSIONS: &str = r#"query list_sessions($project_id: String) {
    match { $s: Session { project_id: $project_id } }
    return {
        $s.session_id as session_id,
        $s.project_id as project_id,
        $s.title as title,
        $s.branch as branch,
        $s.provider as provider,
        $s.workflow as workflow,
        $s.status as status,
        $s.goal as goal,
        $s.created_at as created_at,
        $s.updated_at as updated_at
    }
    order { $s.updated_at desc }
    limit 500
}"#;

pub(crate) const GQ_LIST_FILES: &str = r#"query list_files($project_id: String) {
    match { $f: FileRef { project_id: $project_id } }
    return {
        $f.path as path,
        $f.project_id as project_id,
        $f.language as language,
        $f.last_touched_at as last_touched_at
    }
    order { $f.last_touched_at desc }
    limit 1000
}"#;

pub(crate) const GQ_LIST_SYMBOLS: &str = r#"query list_symbols($project_id: String) {
    match { $s: SymbolRef { project_id: $project_id } }
    return {
        $s.fqn as fqn,
        $s.project_id as project_id,
        $s.file_path as file_path,
        $s.kind as kind,
        $s.name as name
    }
    order { $s.file_path asc, $s.name asc }
    limit 1000
}"#;

pub(crate) const GQ_LIST_PUBLISHED_EDGES: &str = r#"query list_published_edges($project_id: String) {
    match {
        $a: ContextAtom { project_id: $project_id }
        $a $e:publishedBy $s
    }
    return {
        $a.atom_id as source_id,
        $s.session_id as target_id,
        $e.role as role,
        $e.published_at as published_at
    }
    limit 2000
}"#;

pub(crate) const GQ_LIST_CONSUMED_EDGES: &str = r#"query list_consumed_edges($project_id: String) {
    match {
        $a: ContextAtom { project_id: $project_id }
        $a $e:consumedBy $s
    }
    return {
        $a.atom_id as source_id,
        $s.session_id as target_id,
        $e.injection_mode as injection_mode,
        $e.relevance as relevance,
        $e.consumed_at as consumed_at
    }
    limit 2000
}"#;

pub(crate) const GQ_LIST_FILE_EDGES: &str = r#"query list_file_edges($project_id: String) {
    match {
        $a: ContextAtom { project_id: $project_id }
        $a $e:touchesFile $f
    }
    return {
        $a.atom_id as source_id,
        $f.path as target_id,
        $e.access as access,
        $e.line_start as line_start,
        $e.line_end as line_end
    }
    limit 10000
}"#;

pub(crate) const GQ_LIST_ATOM_RELATIONS: &str = r#"query list_atom_relations($project_id: String) {
    match {
        $a: ContextAtom { project_id: $project_id }
        $a $e:relatesTo $b
    }
    return {
        $a.atom_id as source_id,
        $b.atom_id as target_id,
        $e.relation as relation,
        $e.note as note,
        $e.confidence as confidence,
        $e.created_at as created_at
    }
    limit 2000
}"#;

pub(crate) const GQ_LIST_SESSION_FOLLOWS: &str = r#"query list_session_follows($project_id: String) {
    match {
        $s1: Session { project_id: $project_id }
        $s1 $e:sessionFollows $s2
    }
    return {
        $s1.session_id as source_id,
        $s2.session_id as target_id,
        $e.relation as relation,
        $e.created_at as created_at
    }
    limit 2000
}"#;

// Slim ID-only projection queries used by `compute_stats` to avoid fetching full atom/edge bodies.
pub(crate) const GQ_COUNT_ATOM_IDS: &str = r#"query recent_atoms($project_id: String) {
    match { $a: ContextAtom { project_id: $project_id } }
    return { $a.atom_id as atom_id }
    limit 1000
}"#;

pub(crate) const GQ_COUNT_SESSION_IDS: &str = r#"query list_sessions($project_id: String) {
    match { $s: Session { project_id: $project_id } }
    return { $s.session_id as session_id }
    limit 500
}"#;

pub(crate) const GQ_COUNT_SYMBOL_IDS: &str = r#"query list_symbols($project_id: String) {
    match { $s: SymbolRef { project_id: $project_id } }
    return { $s.fqn as fqn }
    limit 1000
}"#;

pub(crate) const GQ_COUNT_PUBLISHED_EDGE_IDS: &str = r#"query list_published_edges($project_id: String) {
    match {
        $a: ContextAtom { project_id: $project_id }
        $a $e:publishedBy $s
    }
    return { $a.atom_id as source_id }
    limit 2000
}"#;

pub(crate) const GQ_COUNT_CONSUMED_EDGE_IDS: &str = r#"query list_consumed_edges($project_id: String) {
    match {
        $a: ContextAtom { project_id: $project_id }
        $a $e:consumedBy $s
    }
    return { $a.atom_id as source_id }
    limit 2000
}"#;

pub(crate) const GQ_COUNT_FILE_EDGE_IDS: &str = r#"query list_file_edges($project_id: String) {
    match {
        $a: ContextAtom { project_id: $project_id }
        $a $e:touchesFile $f
    }
    return { $a.atom_id as source_id }
    limit 2000
}"#;

pub(crate) const GQ_COUNT_ATOM_RELATION_IDS: &str = r#"query list_atom_relations($project_id: String) {
    match {
        $a: ContextAtom { project_id: $project_id }
        $a $e:relatesTo $b
    }
    return { $a.atom_id as source_id }
    limit 2000
}"#;

pub(crate) const GQ_COUNT_SESSION_FOLLOW_IDS: &str = r#"query list_session_follows($project_id: String) {
    match {
        $s1: Session { project_id: $project_id }
        $s1 $e:sessionFollows $s2
    }
    return { $s1.session_id as source_id }
    limit 2000
}"#;

pub(crate) const GQ_COUNT_SYMBOL_EDGE_IDS: &str = r#"query list_symbol_edges($project_id: String) {
    match {
        $a: ContextAtom { project_id: $project_id }
        $a $e:mentionsSymbol $s
    }
    return { $a.atom_id as source_id }
    limit 2000
}"#;

pub(crate) const GQ_UPSERT_SESSION: &str = r#"query upsert_session(
    $session_id: String,
    $project_id: String,
    $title: String,
    $branch: String,
    $provider: String,
    $workflow: String,
    $status: String,
    $goal: String?,
    $created_at: DateTime,
    $updated_at: DateTime
) {
    insert Session {
        session_id: $session_id,
        project_id: $project_id,
        title: $title,
        branch: $branch,
        provider: $provider,
        workflow: $workflow,
        status: $status,
        goal: $goal,
        created_at: $created_at,
        updated_at: $updated_at
    }
}"#;

pub(crate) const GQ_UPSERT_ATOM: &str = r#"query upsert_atom(
    $atom_id: String,
    $project_id: String,
    $kind: String,
    $title: String,
    $summary: String,
    $body: String,
    $tags: [String]?,
    $source_branch: String,
    $source_session_id: String?,
    $confidence: F64,
    $pin_priority: I32,
    $verified: Bool,
    $origin_layer: String?,
    $authority: String?,
    $privacy: String?,
    $brain_record_id: String?,
    $memory_id: String?,
    $record_revision: I64?,
    $created_at: DateTime,
    $updated_at: DateTime
) {
    insert ContextAtom {
        atom_id: $atom_id,
        project_id: $project_id,
        kind: $kind,
        title: $title,
        summary: $summary,
        body: $body,
        tags: $tags,
        source_branch: $source_branch,
        source_session_id: $source_session_id,
        confidence: $confidence,
        pin_priority: $pin_priority,
        verified: $verified,
        origin_layer: $origin_layer,
        authority: $authority,
        privacy: $privacy,
        brain_record_id: $brain_record_id,
        memory_id: $memory_id,
        record_revision: $record_revision,
        created_at: $created_at,
        updated_at: $updated_at
    }
}"#;

pub(crate) const GQ_UPSERT_FILE: &str = r#"query upsert_file(
    $path: String,
    $project_id: String,
    $language: String?,
    $last_touched_at: DateTime
) {
    insert FileRef {
        path: $path,
        project_id: $project_id,
        language: $language,
        last_touched_at: $last_touched_at
    }
}"#;

pub(crate) const GQ_UPSERT_TURN_SNAPSHOT: &str = r#"query upsert_turn_snapshot(
    $snapshot_id: String,
    $session_id: String,
    $turn_index: I32,
    $summary: String,
    $created_at: DateTime
) {
    insert TurnSnapshot {
        snapshot_id: $snapshot_id,
        session_id: $session_id,
        turn_index: $turn_index,
        summary: $summary,
        created_at: $created_at
    }
}"#;

pub(crate) const GQ_LINK_PUBLISHED_BY: &str = r#"query link_published_by(
    $atom_id: String,
    $session_id: String,
    $role: String?,
    $published_at: DateTime
) {
    insert PublishedBy {
        from: $atom_id,
        to: $session_id,
        role: $role,
        published_at: $published_at
    }
}"#;

pub(crate) const GQ_LINK_CONSUMED_BY: &str = r#"query link_consumed_by(
    $atom_id: String,
    $session_id: String,
    $consumed_at: DateTime,
    $injection_mode: String?,
    $relevance: F64?
) {
    insert ConsumedBy {
        from: $atom_id,
        to: $session_id,
        consumed_at: $consumed_at,
        injection_mode: $injection_mode,
        relevance: $relevance
    }
}"#;

pub(crate) const GQ_LINK_TOUCHES_FILE: &str = r#"query link_touches_file(
    $atom_id: String,
    $path: String,
    $access: String?,
    $line_start: I32?,
    $line_end: I32?
) {
    insert TouchesFile {
        from: $atom_id,
        to: $path,
        access: $access,
        line_start: $line_start,
        line_end: $line_end
    }
}"#;

/// A declaration a context atom names in a file it touches.
pub(crate) const GQ_UPSERT_SYMBOL: &str = r#"query upsert_symbol(
    $fqn: String,
    $project_id: String,
    $file_path: String,
    $kind: String,
    $name: String
) {
    insert SymbolRef {
        fqn: $fqn,
        project_id: $project_id,
        file_path: $file_path,
        kind: $kind,
        name: $name
    }
}"#;

pub(crate) const GQ_LINK_MENTIONS_SYMBOL: &str = r#"query link_mentions_symbol(
    $atom_id: String,
    $fqn: String,
    $role: String?
) {
    insert MentionsSymbol {
        from: $atom_id,
        to: $fqn,
        role: $role
    }
}"#;

/// An atom's touched files shrank or emptied: clear prior `TouchesFile` edges
/// before writing the updated set.
pub(crate) const GQ_CLEAR_ATOM_FILES: &str = r#"query clear_atom_files($atom_id: String) {
    delete TouchesFile where @src = $atom_id
}"#;

/// An atom whose text changed names other declarations: its old mentions go
/// before the new ones are linked.
pub(crate) const GQ_CLEAR_ATOM_SYMBOLS: &str = r#"query clear_atom_symbols($atom_id: String) {
    delete MentionsSymbol where @src = $atom_id
}"#;

/// The declarations the project's atoms name, with the file each is in.
pub(crate) const GQ_LIST_SYMBOL_EDGES: &str = r#"query list_symbol_edges($project_id: String) {
    match {
        $a: ContextAtom { project_id: $project_id }
        $a $e:mentionsSymbol $s
    }
    return {
        $a.atom_id as source_id,
        $s.fqn as target_id,
        $s.name as name,
        $s.kind as kind,
        $s.file_path as file_path,
        $e.role as role
    }
    limit 2000
}"#;

pub(crate) const GQ_LINK_RELATES_TO: &str = r#"query link_relates_to(
    $source_id: String,
    $target_id: String,
    $relation: String,
    $note: String?,
    $confidence: F64?,
    $created_at: DateTime
) {
    insert RelatesTo {
        from: $source_id,
        to: $target_id,
        relation: $relation,
        note: $note,
        confidence: $confidence,
        created_at: $created_at
    }
}"#;

pub(crate) const GQ_LINK_DERIVED_FROM: &str = r#"query link_derived_from(
    $atom_id: String,
    $snapshot_id: String,
    $recorded_at: DateTime
) {
    insert DerivedFrom {
        from: $atom_id,
        to: $snapshot_id,
        recorded_at: $recorded_at
    }
}"#;

pub(crate) const GQ_LINK_SESSION_FOLLOWS: &str = r#"query link_session_follows(
    $from_session_id: String,
    $to_session_id: String,
    $relation: String,
    $created_at: DateTime
) {
    insert SessionFollows {
        from: $from_session_id,
        to: $to_session_id,
        relation: $relation,
        created_at: $created_at
    }
}"#;

pub(crate) const GQ_CLEAR_BRANCH_CONTEXT: &str = r#"query clear_branch_context() {
    delete PublishedBy where @src != ""
    delete ConsumedBy where @src != ""
    delete TouchesFile where @src != ""
    delete MentionsSymbol where @src != ""
    delete RelatesTo where @src != ""
    delete DerivedFrom where @src != ""
    delete SessionFollows where @src != ""
    delete ContextAtom where atom_id != ""
    delete FileRef where path != ""
    delete SymbolRef where fqn != ""
    delete TurnSnapshot where snapshot_id != ""
    delete Session where session_id != ""
}"#;

/// The latest turn snapshot of a session on a branch: the next one takes the
/// following `turn_index`.
pub(crate) const GQ_LIST_SESSION_TURN_SNAPSHOTS: &str = r#"query list_session_turn_snapshots($session_id: String) {
    match {
        $t: TurnSnapshot { session_id: $session_id }
    }
    return {
        $t.snapshot_id as snapshot_id,
        $t.turn_index as turn_index
    }
    order { $t.turn_index desc }
    limit 1
}"#;

/// Remove one synced atom and every edge that names it from a branch (the
/// canonical sync prunes Project Brain and Memory Bank atoms gone from their
/// source).
pub(crate) const GQ_DELETE_CANONICAL_ATOM: &str = r#"query delete_canonical_atom($atom_id: String) {
    delete PublishedBy where @src = $atom_id
    delete ConsumedBy where @src = $atom_id
    delete TouchesFile where @src = $atom_id
    delete MentionsSymbol where @src = $atom_id
    delete RelatesTo where @src = $atom_id
    delete RelatesTo where @dst = $atom_id
    delete DerivedFrom where @src = $atom_id
    delete ContextAtom where atom_id = $atom_id
}"#;

/// Default `cluster.yaml` template using `{{GRAPH_ID}}` placeholder.
pub const CLUSTER_YAML_TEMPLATE: &str = r#"version: 1
state:
  backend: cluster
  lock: true
graphs:
  {{GRAPH_ID}}:
    schema: harness_context.pg
    queries: queries/
policies:
  graph-access:
    file: policy.yaml
    applies_to: [{{GRAPH_ID}}]
  server-access:
    file: server.policy.yaml
    applies_to: [cluster]
"#;

/// Graph-scoped Cedar policy bundle allowing Harness actors full graph + branch + query access.
pub const GRAPH_POLICY_YAML: &str = r#"version: 1
protected_branches: [main]
groups:
  harness_actors: [harness, harness-ui, harness-session]
rules:
  - id: harness-read-write
    allow:
      actors: { group: harness_actors }
      actions: [read, export, change]
      branch_scope: any
  - id: harness-branches
    allow:
      actors: { group: harness_actors }
      actions: [branch_create, branch_delete, branch_merge, schema_apply, commit_publish, commit_drop]
      target_branch_scope: any
  - id: harness-invoke
    allow:
      actors: { group: harness_actors }
      actions: [invoke_query]
"#;

/// Alias for `GRAPH_POLICY_YAML` (`policy.yaml`).
pub const POLICY_YAML: &str = GRAPH_POLICY_YAML;

/// Cluster-scoped Cedar policy bundle allowing Harness admins `graph_list` and `config_manage`.
pub const SERVER_POLICY_YAML: &str = r#"version: 1
groups:
  harness_admins: [harness, harness-ui]
rules:
  - id: harness-list-graphs
    allow:
      actors: { group: harness_admins }
      actions: [graph_list]
  - id: harness-config-manage
    allow:
      actors: { group: harness_admins }
      actions: [config_manage]
"#;

/// Validate an Omnigraph graph ID against `^[a-zA-Z0-9-]{1,64}$`.
pub fn validate_graph_id(graph_id: &str) -> Result<()> {
    if graph_id.is_empty()
        || graph_id.len() > 64
        || !graph_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(HarnessError::invalid(
            "Omnigraph graph_id must match ^[a-zA-Z0-9-]{1,64}$ (underscores are forbidden).",
        ));
    }
    Ok(())
}

/// Render `cluster.yaml` for a validated `graph_id`.
pub fn render_cluster_yaml(graph_id: &str) -> Result<String> {
    validate_graph_id(graph_id)?;
    Ok(CLUSTER_YAML_TEMPLATE.replace("{{GRAPH_ID}}", graph_id))
}

/// Materialize `cluster.yaml`, `harness_context.pg`, `queries/harness_context.gq`,
/// `policy.yaml`, and `server.policy.yaml` inside `config_dir`.
pub fn write_cluster_bundle(config_dir: &Path, graph_id: &str) -> Result<()> {
    let cluster_yaml = render_cluster_yaml(graph_id)?;
    let queries_dir = config_dir.join("queries");
    std::fs::create_dir_all(&queries_dir)?;
    std::fs::write(config_dir.join("cluster.yaml"), cluster_yaml)?;
    std::fs::write(config_dir.join("harness_context.pg"), HARNESS_CONTEXT_PG)?;
    std::fs::write(
        queries_dir.join("harness_context.gq"),
        HARNESS_STORED_QUERIES_GQ,
    )?;
    std::fs::write(config_dir.join("policy.yaml"), POLICY_YAML)?;
    std::fs::write(config_dir.join("server.policy.yaml"), SERVER_POLICY_YAML)?;
    Ok(())
}
