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
  tool payloads, secrets, customer data, and raw logs are prohibited.

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

### Task Phases

A task may declare the delivery-loop phase it stopped on. Storage keeps
exactly four canonical values — `understanding`, `planning`, `execution`,
`finalization` — matching the `phase` enums in the dynamic-record and handoff
schemas. The CLI (`update --phase`, `brain-update --phase`) additionally
accepts the Skill Flow Phase Map names `implementation`, `quality`, and
`verification`; each is recorded as `execution`, the canonical phase the
skills in those Phase Map rows declare in their own frontmatter. `utility` is
not a task phase: utility skills are cross-cutting tools, not a step a task
stops on. Phases carry no ordering constraint — any phase may follow any
other, because reality (rework, re-planning) does.

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
evidence 2,000 estimated tokens. The target is 8,000 and the hard ceiling is
12,000. Every governed retrieval writes a committed manifest under
`control/retrieval-manifests/`.

If one conflict side matches lexically, eligible linked records are fetched by
UUID even when they do not match the query. Conflict pairs may exceed normal
category/target budgets up to the hard ceiling, with an explicit escalation
reason; privacy, owner, authority, lifecycle, and freshness filters still win.

No network service, MCP server, embedding store, or automatic prompt injection
is part of this runtime.
