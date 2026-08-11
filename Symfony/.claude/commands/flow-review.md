---
flow: review
stages:
  - { phase: verification, agents: [code-reviewer, security-reviewer, performance-optimization], parallel: true }
---

# Flow: Review

Run a parallel multi-lens review of the current change - code quality, security, performance - by spawning this accelerator's read-only review agents concurrently, then synthesizing one deduplicated report in the main conversation.

## Input
$ARGUMENTS

## Instructions

If $ARGUMENTS is not empty, treat it as the review scope (files, a diff
range, or a PR reference). If it is empty, review the current branch's
changes against the default branch.

You - the MAIN conversation - are the orchestrator and the synthesizer.

1. Determine the scope: the list of changed files (for example from
   `git diff --name-only`) and the task ID from the current Git branch.
2. Spawn all three agents of the single `parallel: true` stage in one
   message with the Task tool - **subagent_type** `code-reviewer`,
   `security-reviewer`, and `performance-optimization` - each with its own
   delegation capsule:
   - **Objective** - review the scope through that agent's lens only.
   - **Output format** - a findings list: file:line, severity, one-sentence
     claim, evidence; plus the standard Context Summary.
   - **Tool and source guidance** - the changed-file list and the task ID.
   - **Task boundaries** - report only; do NOT modify files, run fixes, or
     apply optimizations in this flow.
   - **Decisions and assumptions so far** - the review scope and anything
     the user said about intent.
   Before each spawn, save the capsule to a scratch file and record it -
   `python3 memory-bank/scripts/context.py msg-dispatch --task-id <ID> --agent <name> --event spawn --capsule-file <file>`
   (it refuses an under-specified capsule); completions are recorded in the
   channel by the SubagentStop hook automatically.
3. When all three return, synthesize in the main conversation: deduplicate
   overlapping findings, rank by severity, drop claims without evidence,
   and present one report grouped by file.
4. Record the outcome:
   `python3 memory-bank/scripts/context.py update --task-id <ID> --progress "flow-review: <N> findings, <top severity>"`.
5. Suggest next steps: `/coder` to fix confirmed findings, `/debugger` for
   unclear failures, `/verify` for the full Definition of Done.

**Rules.** All three agents are bounded to report-only by their capsules in
this flow; spawn only agents from this accelerator's roster. Note that
`performance-optimization` is declared `writes: true`, so the gate holds the
write lock for it during the stage - harmless here (the other two are
read-only), but do not add a second write-capable agent to this stage. If
the scope is empty (no changes), say so and stop instead of spawning agents.
