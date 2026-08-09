---
name: sdd
description: Spec-driven development: carry a feature through durable spec, design and task-list artifacts in specs/ and tasks/, resumable across sessions. Invoked as /sdd.
disable-model-invocation: true
phase: planning
flow-next: writing-plans
flow-alternatives: [coder, architecture-implementer]
related: [requirements-analyst, architect, writing-plans, project-brain]
---

# Spec-Driven Development

## Overview

Carry a feature through durable artifacts - spec, design, task list, execution
record - before and while it is built. Each phase leaves a file on disk, so the
work survives a closed session, a compacted context, or a week away.

`/sdd` runs the phases by spawning roster agents. This skill is the contract
those agents write against: where each artifact lives, what it must contain,
and how to tell which phase is already done.

You invoke it; the model does not. The frontmatter carries
`disable-model-invocation: true`, which keeps this description out of every
session's context and means Claude will not start a multi-agent, multi-session
flow on its own initiative. The trade is deliberate: a skill run perhaps once
in a project's life should not be advertised on every prompt of it.

## Choose the right entry point

| | `/coder` | `/flow-feature` | `/sdd` |
|---|---|---|---|
| Horizon | minutes | one sitting | many sessions |
| Durable output | the diff | the diff | the diff plus reviewed files in `specs/` and `tasks/` |
| Resumption | n/a | restart the flow | read the artifacts, continue at the first unchecked task |
| Fits | a small, understood change | a feature describable in a paragraph | work whose scope must be agreed before code exists |

Do not run `/sdd` for a change you could finish before the spec is reviewed.
The artifacts are the point; if nobody will read them, they are overhead.

## The artifacts

| Phase | Artifact | Written by |
|---|---|---|
| 1 Specify | `specs/<feature>-spec.md` | requirements-analyst |
| 2 Design | `specs/<feature>-architecture.md` | architect |
| 3 Tasks | `tasks/<TASK-ID>/writing-plans-plan.md` | writing-plans |
| 4 Execute | checkboxes in that plan, plus one `context.py update` per task | coder, test-generator |

`specs/MANIFEST.md` is the index. Every spec file appears in its table with a
purpose, its dependencies and the date it was last touched. **A spec that is
not in the manifest does not exist** - the next session will not find it, and
`/sdd` will re-specify work that is already done. Update the manifest in the
same step that writes the spec, never later.

`<feature>` is a short kebab-case slug (`invoice-export`, not `feature1`), used
identically in both spec filenames so the pair sorts together.

## Spec file

`specs/<feature>-spec.md` answers *what* and *why*. It must not contain a
design.

```markdown
# Spec: [Feature]

## Problem
[What is wrong or missing today, and for whom.]

## User scenarios
1. **[Actor]** [does what] so that [outcome].
   Path: [the route through the system in the user's words, not the code's.]

[One scenario per distinct path worth building. Two that differ only in data
are one scenario.]

## Acceptance criteria
- [ ] **AC-1** [Checkable statement, not a task. "An expired invoice cannot be
  exported."]

[Number them. The task list and the tests both cite these IDs - that citation
is what keeps implementation tied to the spec instead of to the code.]

## Edge cases

| Case | Expected |
|---|---|
| [Empty input, boundary value, concurrent write, missing permission, upstream timeout] | [What the system does. Not "handle it".] |

An edge case with no expected behaviour is an open question, not a spec line.

## Out of scope
- [What this deliberately does not do, so the design is not over-built.]

## Open questions
- [Anything that must be answered before the design is trustworthy.]
```

Unresolved open questions block phase 2. Ask the user; do not guess and
proceed - a design built on a guess costs more than the question.

## Design file

`specs/<feature>-architecture.md` answers *how*, and records what was rejected.

```markdown
# Design: [Feature]

## Approach
[The chosen shape in a few sentences.]

## Components
- Controllers, and the Services they delegate to.
- Doctrine entities, repositories, migrations.
- Forms and validation constraints, voters for authorization.
- Messenger messages/handlers, events, cache - only where needed.

## Stack

| Choice | Version | Over the alternative, because |
|---|---|---|

Only what this feature adds, pins or changes. Do not restate the stack the
project already runs.

## Decisions
| Decision | Chosen | Rejected | Because |
|---|---|---|---|

## Risks
- [What could go wrong, and the cheapest way to find out early.]
```

The Decisions table is the part that pays for itself: it stops the next
session from relitigating a settled choice.

## Task list

`tasks/<TASK-ID>/writing-plans-plan.md` carries the execution state, so its
checkbox format is load-bearing:

```markdown
## Tasks
- [ ] 1. [File-level step, one commit's worth] (AC-1, AC-3)
- [ ] 2. [...] (AC-2)
```

One task is one coherent change an agent can finish and verify. A task nobody
can verify is a task that is not finished. Mark `[x]` only after the change is
made and its check passes.

Every task cites the acceptance criteria it serves, and every criterion is
cited by at least one task. A criterion no task claims is either unbuilt or
out of scope - decide which and say so; do not let it fall off silently. The
citation is also what phase 4 checks against, so a task with no AC is a task
with no definition of done.

## Resuming

`/sdd` on an existing feature must not restart. Determine the phase from the
artifacts, in this order, and continue at the first gap:

1. No entry in `specs/MANIFEST.md` -> phase 1.
2. Spec exists, no `-architecture.md` -> phase 2.
3. Design exists, no plan under `tasks/<TASK-ID>/` -> phase 3.
4. Plan exists with unchecked boxes -> phase 4, at the first unchecked task.
5. All boxes checked -> verification.

Cross-check against the governed task before trusting the files:
`python3 memory-bank/scripts/context.py retrieve --task-id <ID>`. If the task
record and the artifacts disagree, say so and ask - a silent reconciliation
loses whichever one was right.

## Boundaries

- SDD does not integrate. When the tasks are done and `/verify` passes, hand
  off to `/finishing-branch` as a separate, explicit step.
- SDD does not replace requirements gathering from outside the repository;
  `requirements-analyst` still reads the sources it normally reads.
- Do not put client-owned specification material into `specs/`. `Task/` holds
  it, is excluded from indexing, and stays where it is.
- Never delete a spec to "clean up". Supersede it: mark it superseded in the
  manifest and say which file replaced it.
