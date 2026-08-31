---
flow: feature
stages:
  - { phase: understanding, agents: [requirements-analyst] }
  - { phase: planning, agents: [architect] }
  - { phase: planning, agents: [writing-plans], checkpoint: plan-approval }
  - { phase: implementation, agents: [coder] }
  - { phase: implementation, agents: [test-generator] }
  - { phase: verification, agents: [code-reviewer, security-reviewer], parallel: true }
  - { phase: verification, agents: [verify] }
  - { phase: finalization, agents: [finishing-branch], checkpoint: integration }
---

# Flow: Feature

Orchestrate a full feature from the main conversation - requirements, architecture, plan, checkpoint, code, tests, parallel review, verification, finish - by spawning this accelerator's own agents stage by stage with bounded capsule handoffs; expect several agent runs' worth of tokens.

## Input
$ARGUMENTS

## Instructions

If $ARGUMENTS is not empty, treat it as the feature request. If it is empty,
ask the user what to build before starting.

You - the MAIN conversation - are the orchestrator. Execute the `stages`
list from the frontmatter in order, one stage at a time. Do not delegate
orchestration to a sub-agent, and do not run stages the user has not reached.

**Setup.** Derive the task ID from the current Git branch (or
`CONTEXT_TASK_ID`). Ensure the governed task exists: run
`python3 memory-bank/scripts/context.py start --task-id <ID> --goal "<sanitized goal>"`
if it does not (an existing task is never overwritten).

**Per stage:**

1. Build one delegation capsule per agent. Mandatory sections, in this order:
   - **Objective** - what this stage must produce, in one or two sentences.
   - **Output format** - the returning-handoff fields from SKILL FLOW.md
     (work completed, decisions, files, verification evidence, risks,
     next step, blockers).
   - **Tool and source guidance** - the explicit files or directories to
     read, and the task ID for `context.py retrieve`.
   - **Task boundaries** - what the agent must NOT do in this stage.
   - **Decisions and assumptions so far** - carried forward from previous
     stages' Context Summaries; never leave this empty after stage one.

   Keep the serialized capsule within the 8,000-character Task Capsule
   bound. Pass file paths, never file contents, raw diffs, or transcripts.
2. Save the capsule to a scratch file and record the dispatch:
   `python3 memory-bank/scripts/context.py msg-dispatch --task-id <ID> --agent <name> --event spawn --capsule-file <file>`
   - it refuses an under-specified capsule, so fix the capsule before
   spawning. Then spawn each agent with the Task tool: **subagent_type** =
   the agent name from the stage, **prompt** = the capsule. When the stage
   has `parallel: true` (read-only agents only), spawn all of its agents in
   one message; otherwise spawn strictly one agent at a time and wait for it.
3. When an agent returns, read its Context Summary, then record progress:
   `python3 memory-bank/scripts/context.py update --task-id <ID> --actor <name> --progress "<stage>: <sanitized one-line result>"`
   (the SubagentStop hook records the completion in the channel
   automatically). Check the channel for agent messages addressed to you:
   `python3 memory-bank/scripts/context.py msg-read --task-id <ID> --for main --since <last seen seq>`.
   Carry its decisions into the next capsule.
4. At a `checkpoint:` stage, STOP after the spawned agent returns: present
   the accumulated result (the plan, the diff summary) and wait for explicit
   user approval. The user may amend the plan, skip a stage, or abort the
   flow; do not continue without an answer.
5. If an agent reports a blocker or fails, stop the flow, report what
   happened, and suggest `/debugger`. Never retry a failed stage silently
   more than once.

**Rules.** Spawn only agents from this accelerator's roster - the subagent
gate denies everything else. Write-capable agents (`writes: true` in their
frontmatter: coder, architecture-implementer, refactorer, ...) run one at a
time - the gate enforces this with a lock that the completion hook clears;
only read-only agents may share a `parallel: true` stage. For a follow-up
with the same agent, prefer resuming it - it keeps its own context - over
spawning a fresh instance. Skip a stage that clearly does
not apply (say so and mention it at the next checkpoint). Simple tasks do
not need this flow - suggest `/coder` instead when the request is small.
