---
flow: sdd
stages:
  - { phase: understanding, agents: [requirements-analyst] }
  - { phase: planning, agents: [architect], checkpoint: spec-approval }
  - { phase: planning, agents: [writing-plans], checkpoint: task-breakdown }
  - { phase: implementation, agents: [coder] }
  - { phase: implementation, agents: [test-generator] }
  - { phase: verification, agents: [code-reviewer, security-reviewer], parallel: true }
  - { phase: verification, agents: [verify] }
---

# Flow: SDD

Drive spec-driven development from the main conversation - specify, design, break into tasks, then execute task by task - by spawning this accelerator's own agents stage by stage, leaving a durable artifact in `specs/` and `tasks/` at every phase so the work can be reviewed and resumed in a later session.

## Input
$ARGUMENTS

## Instructions

Read `.claude/skills/sdd/SKILL.md` first: it defines the artifact paths, the
file templates, and the resumption order this flow depends on.

If $ARGUMENTS is not empty, treat it as the feature request or the feature
slug of work already started. If it is empty, list the features currently in
`specs/MANIFEST.md` with their phase and ask which to start or continue.

You - the MAIN conversation - are the orchestrator. Do not delegate
orchestration to a sub-agent.

**Setup.** Derive the task ID from the current Git branch (or
`CONTEXT_TASK_ID`) and ensure the governed task exists:
`python3 memory-bank/scripts/context.py start --task-id <ID> --goal "<sanitized goal>"`
(an existing task is never overwritten). Derive the feature slug from the
request; reuse the existing slug when continuing.

**Resume before you start.** Work out the current phase from the artifacts
using the order in the skill, and enter the flow at the first gap. Say which
phase you resumed at and why. Re-running a completed phase overwrites a
reviewed artifact - treat it as a change the user must ask for.

**Per stage,** follow the delegation protocol: build one bounded capsule per
agent (objective; output format per SKILL FLOW.md; tool and source guidance
including the artifact paths and the task ID; task boundaries; decisions and
assumptions so far), keep it inside the 8,000-character Task Capsule bound,
pass file paths rather than contents, then record the dispatch -
`python3 memory-bank/scripts/context.py msg-dispatch --task-id <ID> --agent <name> --event spawn --capsule-file <file>` -
before spawning with the Task tool. Spawn write-capable agents strictly one at
a time; spawn the `parallel: true` stage's agents in one message. After each
return, read the Context Summary, check the channel
(`msg-read --task-id <ID> --for main`), and record progress with
`update --task-id <ID> --actor <name>`.

**What each stage must leave behind.** This is the part that makes the flow
spec-driven rather than another feature flow. A stage is not complete until
its artifact exists:

1. **requirements-analyst** writes `specs/<slug>-spec.md` - problem, user
   scenarios, numbered acceptance criteria, edge cases, out of scope, open
   questions - and adds its row to `specs/MANIFEST.md`. Boundary: no design,
   no file-level steps. Do not proceed with open questions unanswered.
2. **architect** writes `specs/<slug>-architecture.md`: approach, components,
   stack (only what this feature adds or pins), the Decisions table with
   rejected options, risks. Every acceptance criterion and every edge case
   must be answerable from the design - name any that is not, rather than
   designing past it. Updates the manifest row. Boundary: no implementation.
   **Checkpoint `spec-approval`** - present spec and design together, wait.
3. **writing-plans** writes `tasks/<TASK-ID>/writing-plans-plan.md`: a
   numbered checkbox task list derived from the design, each task citing the
   acceptance criteria it serves. Before presenting it, check the coverage
   both ways - every criterion cited by some task, every task citing some
   criterion - and report any gap instead of hiding it. Boundary: no code.
   **Checkpoint `task-breakdown`** - present the task list and wait.
4. **coder** implements the unchecked tasks. Give it one task, or a small
   contiguous group, per capsule - never the whole list. This is where the
   flow earns "spec-driven", so each capsule carries, verbatim, the
   acceptance criteria that task cites plus any edge-case rows touching it;
   the spec path alone is not enough, because an agent given only a path
   reads the code instead. The capsule's Objective is to satisfy those
   criteria, not to make the described change.
   After each task: confirm the cited criteria hold, mark the checkbox `[x]`,
   and run
   `update --task-id <ID> --actor coder --progress "task <n> (AC-x): <one line>"`.
   That pair is what makes an interrupted run resumable; skipping it strands
   the work.
   If the code cannot satisfy a criterion as written, **stop**. Amend the
   spec with the user and let the change flow down through design and tasks.
   Never redefine done by quietly adjusting the criterion to match the code -
   that is the one failure that makes the whole artifact chain worthless.
5. **test-generator** writes tests per acceptance criterion and per edge-case
   row, citing the AC id in each test name, working from the spec rather than
   from the code it just read. A criterion with no test is not covered,
   however green the suite is.
6. **code-reviewer + security-reviewer** review in parallel, report only.
   Give both the spec: a change that is clean but does not do what the spec
   says is still a finding.
7. **verify** runs the Definition of Done. Before declaring the feature done,
   re-read the spec and confirm every acceptance criterion is checked and
   every edge-case row has a behaviour in the code; list any that are not.

If an agent cannot satisfy its stage because the upstream artifact is wrong,
stop and fix the artifact - amending the spec is cheaper than building the
wrong thing and is why the checkpoints exist.

**Rules.** Spawn only agents from this accelerator's roster - the subagent
gate denies everything else. Write-capable agents (`writes: true`: coder,
test-generator, architecture-implementer, ...) run one at a time; only
read-only agents share the `parallel: true` stage. Stop the flow and report on
a blocker or a failed stage; never retry a failed stage silently more than
once. This flow does not integrate the work: when `/verify` passes, suggest
`/finishing-branch` as a separate step. For a feature that fits in one sitting
and needs no reviewable spec, suggest `/flow-feature` instead; for a small
change, `/coder`.
