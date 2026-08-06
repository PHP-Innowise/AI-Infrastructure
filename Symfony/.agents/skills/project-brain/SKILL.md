---
name: project-brain
description: Govern shared Symfony task context with Project Brain. Use for task lifecycle, handoffs, unified retrieval, findings, bugs, incidents, decisions, events, compaction, or automatic/independently reviewed promotions. Use memory-bank only for durable retrieval/capture and governed promotion application.
phase: utility
flow-next: null
flow-alternatives: [memory-bank, documentation-generator, reflect]
---

# Symfony Project Brain

Operate the shared, repository-backed control plane in `project-brain/` through the dependency-free facade at `memory-bank/scripts/context.py`. Governed mode is the default. Use `--mode lightweight` only when shared task state, formal handoffs, and governed records are explicitly unnecessary.

## Authority And Boundaries

1. Canonical policy, specs, current code, configuration, migrations, and tests establish truth and always outrank retrieved context.
2. `project-brain/` owns shared active tasks, handoffs, findings, bugs, incidents, decisions, events, retrieval manifests, conflicts, and promotion proposals.
3. `memory-bank/` owns governed reusable consequences, not active progress. SQLite is a disposable index plus local binding/cache in governed mode.
4. Never claim automatic prompt injection. Indexing and retrieval happen only through explicit CLI calls.
5. Never write raw prompts, responses, chain-of-thought, command output, secrets, customer data, personal data, or unredacted incident payloads.

## Select One Operation

- **Start/bind task:** create or bind the shared task and initial handoff.
- **Retrieve:** assemble governed, task-aware context through the one public retrieval command.
- **Update/handoff:** revision-check progress and refresh a concise handoff.
- **Record:** create or revise a finding, bug, incident, decision, or immutable event.
- **Complete:** verify outcomes and advance the shared task lifecycle.
- **Compact:** validate and atomically archive terminal, superseded, or stale records without deletion.
- **Promote durable knowledge:** use the configured automatic or independently reviewed promotion mode without falsifying review provenance.
- **Lightweight:** explicitly use local-only task/episode behavior with its durability and collaboration limits.

Execute only the selected operation, then stop. Inspect `python3 memory-bank/scripts/context.py --help` and the relevant `project-brain/templates/` file before mutation; do not invent unsupported flags or hand-edit revision/lifecycle fields when a command exists.

## Governed Task Lifecycle

1. Use a stable caller-supplied task ID and an explicit owner. Run the facade's `start` operation in governed mode before non-trivial work.
2. Confirm the created or bound task UUID/revision and initial handoff. SQLite may retain a local binding but must not become a second authority for progress.
3. Before material decisions, retrieve context with exactly:

   ```bash
   python3 memory-bank/scripts/context.py retrieve QUERY --task-id ID
   ```

4. Open cited canonical sources and verify material claims. Preserve surfaced conflicts and stale-source warnings; do not collapse disagreement into a false consensus.
5. Update only sanitized progress, evidence references, affected paths, next action, blockers, verification, and the canonical phase: `understanding`, `planning`, `implementation`, `verification`, or `finalization`. Input aliases normalize before persistence.
6. Refresh the handoff after meaningful progress and before changing agent/session. A handoff is a bounded continuation record, not a transcript.
7. Complete only after verification, using the current numeric revision. A merge completion candidate is advisory and never authorizes automatic closure. Record the outcome and evidence, advance only through legal transitions, and validate the resulting task and handoff.

Use `--mode lightweight` only by explicit choice. In that mode, local SQLite task state and episodes are machine-local, non-authoritative outside that session, and lost when the local database is deleted. Lightweight episodes are never auto-promoted.

## Supported Record Commands

- Shared task records use governed `start`, `update`, and `get`; these commands also maintain task binding and handoff state.
- The other five governed record types—`finding`, `bug`, `incident`, `decision`, and immutable `event`—use `brain-create`, `brain-update`, and `brain-get`.
- Inspect each subcommand's `--help` before use. Supply only supported flags; do not invent generic CRUD or lifecycle options.

## Findings, Bugs, Incidents, Decisions, And Events

1. Choose the narrowest record type and start from its template.
2. Use UUIDv4 identity, current revision, owner, lifecycle, authority, confidence, privacy, source fingerprints, evidence links, conflicts, and supersession fields required by `project-brain/PROTOCOL.md`.
3. Distinguish observed evidence from inference. A finding records a notable observation; a bug records reproducible incorrect behavior; an incident records operational impact and response; a decision records an accepted choice, rationale, alternatives, and consequences; an event is immutable historical evidence.
4. Keep restricted content out unless the caller is authorized and the configured privacy policy permits storage and retrieval.
5. Use append-only legal transitions and optimistic revision checks. On a stale revision, reload, reconcile, and retry; never overwrite concurrent work.
6. Validate active and archived records equally and rebuild deterministic indexes after supported mutations.

## Unified Retrieval

- The public agent interface is only `python3 memory-bank/scripts/context.py retrieve QUERY --task-id ID`; `context` may exist solely as a compatibility alias.
- Retrieval applies privacy, owner, authority, lifecycle, supersession, and source-freshness filters before selection.
- Treat SQLite/BM25 output and the context packet as discovery aids. Verify cited sources before implementation or decisions.
- Keep bounded snippets and category budgets. Delivered capsules allow at most 2 procedural, 3 semantic, and 1 episodic item and 8,000 serialized characters. Do not bypass the internal conflict ceiling or omit an escalation reason.
- Preserve relevant conflicting records together and report excluded/stale/private candidates only as safe metadata.
- Require a committed retrieval manifest for governed retrieval. The manifest records selection/exclusion metadata, not hidden reasoning or source bodies.

## Compaction

1. Validate the complete active and archive sets first.
2. Compact only terminal, superseded, or policy-stale records eligible under `project-brain/PROTOCOL.md`.
3. Use the supported compact operation and repository-wide mutation lock. Compaction moves records atomically; it never deletes history.
4. Rebuild deterministic active/archive indexes and validate again.
5. Report moved IDs, unchanged records, validation results, and any blocker. Do not compact records with unresolved lifecycle, privacy, link, or fingerprint errors.

## Promotion Proposals

1. Select the configured mode. In automatic mode, the turn boundary may promote eligible verified knowledge with truthful `automatic`/`approved-without-review` labels and no reviewer. In reviewed mode (`automatic_promotion=false`), use propose → independent human review → apply.
2. Propose only a durable, reusable, project-specific consequence supported by verified evidence. Exclude transient progress, raw evidence, generic Symfony advice, secrets, personal/customer data, and unresolved conflicts.
3. Search existing active memory first and prefer updating or superseding a matching chunk over creating a duplicate.
4. In reviewed mode, create a proposal under `project-brain/control/promotions/` with source record IDs/revisions, evidence, proposed destination/consequence, privacy review, conflicts, and review status.
5. In reviewed mode, stop at proposal. An agent must not supply the independent reviewer, approve its own proposal, or apply it.
6. After independent approval, use the `memory-bank` skill's reviewed-promotion mode to apply atomically and record the destination memory ID/revision and outcome.

## Validation

- Run the supported Project Brain validator and `python3 memory-bank/scripts/validate.py`.
- Confirm task/handoff and record links resolve, source fingerprints are current, revisions are monotonic, transitions are legal, indexes are deterministic, and archive records pass the same checks as active records.
- Confirm governed retrieval produced a manifest and no unauthorized/private/stale content leaked.
- Run the active edition's DOD and report unavailable tooling as `N/A - tooling not configured`.

## Output

Report mode, selected operation, task/record IDs and revisions, handoff path, retrieval manifest and cited canonical sources, conflicts/staleness/privacy exclusions, lifecycle or compaction changes, promotion proposal status, validation evidence, Context Summary, and Next Steps.
