# Task Capsule Context Optimization Design

**Date:** 2026-07-29
**Status:** Approved in conversation
**Scope:** Laravel, Symfony, and PHP Core accelerator editions

## Problem

The repository-local Context Engine preserves useful state across sessions, but
token use can still grow in three places:

1. an agent reads broad project documentation at the start of a request;
2. a long task accumulates conversation history, logs, and intermediate
   reasoning;
3. a fresh agent repeats repository discovery instead of receiving a focused
   handoff.

The existing `context` command already returns bounded FTS5 snippets instead of
full documents. The larger opportunity is therefore progressive disclosure:
pass a small, source-linked task packet first and read full sources only when
the current step requires them.

## Decision

Adopt a lightweight, OpenGSD-inspired **Task Capsule** inside the current
accelerator. Borrow two ideas from OpenGSD:

- use a fresh context at meaningful phase boundaries;
- transfer durable, inspectable artifacts instead of the parent conversation.

Do not install `gsd-core` or `gsd-pi`. Do not add `.planning/`, another
database, a second task lifecycle, or another memory layer.

The Context Engine remains repository-local, with one ignored SQLite database
and the existing Working, Procedural, Semantic, and Episodic layers.

References:

- <https://github.com/open-gsd/gsd-core/blob/next/docs/explanation/context-engineering.md>
- <https://www.opengsd.net/docs/v2/auto-mode>

## Goals

- Reduce context transferred at request start, during long tasks, and between
  agents.
- Keep `memory` and `checkpoint` argument-free.
- Preserve the existing explicit `complete` lifecycle.
- Keep repository code, configuration, tests, specifications, and policy
  authoritative.
- Preserve the same behavior across Laravel, Symfony, and PHP Core.
- Avoid model-specific tokenizers and new runtime dependencies.

## Non-Goals

- Installing or wrapping OpenGSD.
- Automatically injecting context into every request.
- Replacing the current SQLite database or FTS5 retrieval.
- Adding embeddings, vector search, MCP, or a central memory service.
- Storing raw conversations, prompts, responses, diffs, or reasoning.
- Automatically spawning a fresh agent for every small change.
- Automatically completing Working tasks.

## Architecture

A Task Capsule is a bounded, ephemeral projection of existing state:

```text
current request
      +
Working Memory
      +
Procedural / Semantic / Episodic retrieval
      +
current safe file references
      |
      v
bounded Task Capsule
      |
      +--> current agent for a simple task
      |
      `--> fresh phase agent for a complex task
```

The capsule is generated on demand and is not stored in a new table. Existing
Working tasks and episodes remain the only local lifecycle records.

The existing `context.py context` result remains the public retrieval
primitive. Its current fields and explicit CLI form remain compatible. The
accelerator's phase workflow composes the capsule from that result, the current
request, and safe Working data. No new user-facing command sequence is
required.

When a valid Working task exists, the workflow uses `context`. Before Working
exists, it uses the existing layer-filtered `search` operation with the current
request and omits the Working section. It does not create a placeholder task or
add another retrieval command.

## Task Capsule Contract

The serialized capsule contains only:

- current goal and task ID when available;
- latest sanitized progress and next step;
- priority changed or related file paths;
- at most two Procedural results;
- at most three Semantic results;
- at most one Episodic result;
- verification criteria or evidence when available;
- source paths or episode identifiers for every retrieved claim;
- safe warnings, such as missing Working state or a failed index refresh.

Retrieved document entries contain a title, path, layer, kind, and short FTS5
snippet. They do not contain the full source document. A worker reads a cited
source only when its current step requires more information.

The capsule must not contain:

- parent conversation history;
- raw diffs, logs, command output, prompts, responses, or reasoning;
- contents of ignored, binary, credential, or sensitive files;
- duplicate source entries;
- copied policy or documentation unrelated to the current step.

## Budget

The complete serialized capsule has a hard limit of 8,000 Unicode characters,
including field names and source metadata.

Budget priority is:

1. goal, latest progress, next step, and safe file references;
2. applicable Procedural constraints;
3. Semantic knowledge;
4. Episodic precedent;
5. optional verification detail.

When the result exceeds the budget:

1. remove the lowest-ranked Episodic result;
2. remove lowest-ranked Semantic results;
3. shorten optional verification detail;
4. shorten long file lists to the highest-priority paths plus an omitted count;
5. shorten progress text while preserving the latest outcome and next step.

The goal and mandatory applicable Procedural constraints are never silently
removed. If they cannot fit, capsule generation fails with a safe actionable
error instead of emitting an incomplete packet.

The implementation uses deterministic character and item limits. It does not
add a model tokenizer. Runtime-reported token usage may be recorded as
observational benchmark data, but it is not required for correctness.

## Retrieval and Deduplication

The retrieval query is assembled from the current request plus available
Working goal, latest progress, next step, and safe file names. Generated values
are treated as data and are never interpolated into executable shell syntax.

FTS5 remains the retrieval engine. Results keep their existing BM25 relevance
ordering with a deterministic path or episode-ID tie-break. Documents are
deduplicated by canonical repository path; episodes are deduplicated by local
episode ID.

The capsule links to authoritative sources rather than copying them. An agent
may progressively open one cited source, verify the relevant claim, and stop.
It must not preload every cited file automatically.

## Hybrid Phase Routing

Simple tasks stay in the current context. No file-count, line-count, or
token-estimation heuristic decides otherwise.

A fresh context is used only at a natural boundary of an already complex
workflow:

- research to planning;
- planning to implementation;
- implementation to independent verification;
- recovery after runtime compaction or exhausted context.

The current orchestrating agent supplies the capsule and explicit current-step
files to the fresh phase agent. It does not pass the parent transcript.

The phase agent returns a compact handoff containing:

- work completed;
- decisions made;
- files changed or examined;
- verification evidence;
- next step;
- unresolved blockers or questions.

The handoff is sanitized before it updates Working Memory. It never stores raw
agent output. When Working exists, the orchestrator writes the compact handoff
through the existing `context.py update` operation. When it does not exist but
Git-visible work now exists, the ordinary `checkpoint` procedure creates it.
Otherwise the handoff remains in the current orchestration result until a
durable task artifact or Git-visible change exists.

## Existing Command Behavior

`checkpoint` remains Working-only:

- derives task ID from the current named Git branch;
- summarizes all current staged, unstaged, and untracked non-ignored changes;
- creates or updates Working Memory;
- does not index other layers, complete the task, or modify Git state.

`memory` remains the all-layer refresh:

- runs the Working checkpoint procedure when safe and applicable;
- indexes Procedural, Semantic, and changelog-backed Episodic sources;
- does not create an episode or complete the task.

`context` supplies bounded retrieval data for the Task Capsule.

`complete` remains an explicit operation that atomically creates the local
episode and removes the corresponding Working task.

## Missing or Degraded State

- **No Working task:** build an ephemeral capsule from the current request and
  branch when available. Do not create a Working task before Git-visible work
  exists. A later `checkpoint` creates it normally.
- **Clean tree:** capsule retrieval remains available; Working may be absent.
- **Detached HEAD:** do not invent a task ID. Build a request-only capsule and
  report the missing branch.
- **Index refresh failure:** return Working/request context with a safe warning
  and omit long-lived retrieval rather than presenting stale data as current.
- **Empty layer:** omit the layer without error.
- **Oversized Working state:** preserve goal, latest progress, next step, and
  priority files; report omitted counts.
- **Phase-agent failure:** keep Working Memory unchanged and return control to
  the orchestrating agent.

Existing secret rejection, Git-ignore filtering, invalid UTF-8 handling, path
validation, and transaction guarantees remain mandatory.

## Compatibility

- Existing `context.py` commands and explicit lifecycle arguments continue to
  work.
- Existing local `context.db` files remain readable.
- `memory`, `checkpoint`, and manual `complete` semantics do not change.
- Task Capsule output is derived and can be regenerated.
- Mirrored implementation and tests remain byte-identical across all three
  editions where the current Context Engine is byte-identical.

## Verification

### Automated Tests

Add the smallest tests that prove:

- the serialized capsule never exceeds 8,000 characters;
- layer item limits are enforced;
- over-budget removal follows the documented priority;
- mandatory goal and Procedural constraints are preserved or fail safely;
- source paths and episode IDs are deduplicated deterministically;
- full document contents and parent conversation text are absent;
- missing Working state, clean tree, detached HEAD, and empty layers are safe;
- index failure returns Working/request context without stale retrieval;
- secret and ignored-file protections remain intact;
- all three edition copies remain byte-identical.

Run every edition's full memory-bank test suite and validator.

### Bauherrenmappe Pressure Check

Verify at least three representative tasks on a disposable Bauherrenmappe
clone:

1. a focused single-area change;
2. a multi-file feature or refactor;
3. an independent verification task.

For each task, record:

- baseline context size: Working data plus full contents of sources selected by
  the existing default retrieval;
- Task Capsule serialized size;
- selected and expected authoritative source paths;
- task verification result;
- Git status before and after;
- optional runtime-reported token usage when available.

The first release succeeds when:

- capsule text is at least 60% smaller than the baseline for each scenario;
- every expected authoritative source remains cited;
- the same task verification still passes;
- the original Bauherrenmappe checkout remains unchanged;
- no sensitive file content is read or stored.

## Alternatives Rejected

### Retrieval Limits Only

The existing command already limits results and returns short snippets. Smaller
numeric limits alone do not address long parent conversations or repeated
repository discovery.

### Full OpenGSD Installation

OpenGSD would add its own commands, agents, planning artifacts, and lifecycle.
That overlaps with the accelerator's existing command/agent/skill model,
`tasks/`, `specs/`, and Context Engine state.

### Adaptive Token-Aware Orchestration

Automatic token measurement, compaction thresholds, and model-specific routing
would add dependencies and runtime coupling before deterministic Task Capsules
have been measured. Add them only if the Bauherrenmappe benchmark shows the
fixed capsule contract is insufficient.
