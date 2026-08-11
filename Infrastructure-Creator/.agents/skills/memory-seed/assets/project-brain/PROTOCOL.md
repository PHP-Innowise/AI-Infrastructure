# Project Brain Protocol

Project Brain is the shared, repository-backed authority for active work. The
Memory Bank contains only reviewed reusable knowledge. The SQLite Local Context
Engine is a disposable lexical index and local binding/cache.

## Invariants

- Record and control IDs are UUIDv4 values. External ticket IDs are aliases.
- Every mutation holds `project-brain/local/runtime.lock` and uses atomic file
  replacement.
- Records use strict JSON frontmatter, an optimistic integer `revision`, and an
  append-only transition history.
- Only an authorized owner may mutate a record. Stale expected revisions fail.
- Privacy, owner, authority, lifecycle, supersession, archive, and source
  freshness checks happen before indexing and again before retrieval.
- Canonical project sources outrank Project Brain, Memory Bank, and local
  indexes. Conflicts are retained explicitly.
- Terminal records are moved, never deleted. Active and archived records are
  held to the same validation contract.
- Task phases are stored only as `understanding`, `planning`,
  `implementation`, `verification`, or `finalization`. Input compatibility
  aliases normalize at the mutation boundary: `implementing` and `execution`
  become `implementation`; `review` becomes `verification`.
- Completion is always explicit and requires the caller's current numeric
  revision. Turn maintenance may report sanitized branch-merge evidence as a
  completion candidate, but it never closes the task or creates an episode.
- Durable-memory promotion has two modes, and the record always states which
  one produced it. Automatic promotion is the default: the turn-end hook
  promotes eligible knowledge unattended, and the runtime never dresses it up
  as reviewed — `reviewer` stays null, `review_mode` is `automatic`, the
  outcome is `approved-without-review`, the chunk is tagged `auto-promoted`,
  and `promote-review` refuses to sign such a promotion after the fact.
  Reviewed promotion is propose → independent human review → apply, and is
  reached by setting `automatic_promotion` to `false`. Eligibility is identical
  in both modes: only resolved findings and bugs, closed incidents, and
  accepted decisions — never tasks, whose checkpoint progress is not reusable
  knowledge. The source type, path, ID, and revision are rechecked at apply
  time in both modes; partial writes, including promotion status, roll back.
- Telemetry is disabled and metadata-only. Prompts, responses, source bodies,
  tool payloads, secrets, customer data, and raw logs are prohibited. The
  optional local event adapter writes nothing unless both telemetry switches
  are enabled and records unavailable host metrics explicitly as `N/A`.

## Modes

`governed` is the default. A task record and its handoff are authoritative;
SQLite stores only a binding/cache and an optional non-authoritative replay
episode. `lightweight` is an explicit local-only fallback preserving the
historical SQLite working-task and episode lifecycle.

## Dynamic Record Lifecycles

- task: `active → blocked | verifying | completed | cancelled`;
  `blocked → active | cancelled`; `verifying → active | completed | cancelled`.
- finding: `open → investigating | resolved | superseded`;
  `investigating → open | resolved | superseded`.
- bug: `reported → triaged | cancelled`; `triaged → fixing | cancelled`;
  `fixing → verifying | triaged | cancelled`;
  `verifying → fixing | resolved | cancelled`.
- incident: `open → contained | cancelled`; `contained → recovering | resolved`;
  `recovering → contained | resolved`; `resolved → closed`.
- decision: `proposed → accepted | rejected`; `accepted → superseded`.
- event: `recorded → superseded`; event content is otherwise immutable.

States with no outgoing edge are terminal. Incident `resolved` and decision
`accepted` deliberately remain active until closure/supersession. Every record
type uses owner authorization, UUIDv4 relationships, source fingerprints,
privacy/authority controls, CAS revisions, and append-only transitions.

Tasks retain the compatibility commands `start`, `update`, `get`, `complete`,
and `clear`. Other dynamic records use:

```text
brain-create TYPE --external-id ID --title TITLE
brain-update --record-id ID --revision N [--transition STATE]
brain-get --record-id ID
```

Cross-store task commands snapshot Brain records, handoffs, and deterministic
indexes while mutating the SQLite binding/episode transaction. Any failure
restores both stores. Compaction uses the same snapshot-and-move-journal rule.

## Retrieval

The public command is:

```text
python3 memory-bank/scripts/context.py retrieve QUERY --task-id ID
```

`context` is an alias. Retrieval uses SQLite FTS5/BM25 and bounded snippets.
Category budgets are policy 1,200; handoff 1,500; durable 3,500; dynamic 1,500;
evidence 2,000 estimated tokens. Internal candidate selection may escalate to
its 12,000-token conflict ceiling, but the delivered capsule is always capped
after ranking/filtering at 2 procedural, 3 semantic, 1 episodic item and 8,000
serialized characters. Every governed retrieval writes a manifest under
`control/retrieval-manifests/`; automated `--ephemeral` retrieval writes the
same metadata contract to ignored local state.

If one conflict side matches lexically, eligible linked records are fetched by
UUID even when they do not match the query. Conflict pairs may exceed normal
category/target budgets up to the hard ceiling, with an explicit escalation
reason; privacy, owner, authority, lifecycle, and freshness filters still win.

No network service, MCP server, embedding store, or automatic prompt injection
is part of this runtime.

## Messages

Orchestrated agents communicate through an append-only channel: one JSONL
journal per task under `control/messages/{task_uuid}.jsonl`, written under the
same mutation lock as every other Brain write and never rewritten — the git
history of the journal is the audit trail. An entry carries `seq`,
`from_actor`, `to_actor` (an agent slug, `main` for the orchestrating
conversation, or `*` to broadcast), a type from `finding`, `question`,
`handoff`, `dispatch`, `completion`, a body capped at 8,000 characters that
passes the same secret screening as task fields, optional path refs, and — for
dispatch events — the SHA-256 digest of the delegation capsule. `msg-dispatch`
refuses to record a spawn whose capsule fails the mandatory-section check
(objective, output format, tool and source guidance, boundaries, decisions and
assumptions; `capsule --validate` runs the same check standalone). The journal
keeps no read cursor: consumers filter with `msg-read --for ACTOR --since SEQ`
and track their own position. A terminal task refuses new messages but stays
readable; `validate` checks every journal line against the schema, its task,
and strict `seq` ordering. Task phases move only forward along the stored
vocabulary; `--allow-phase-regression` states a backward move is deliberate.
