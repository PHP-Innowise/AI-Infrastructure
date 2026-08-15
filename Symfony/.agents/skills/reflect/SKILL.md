---
name: reflect
description: "Turn agent mistakes, failures, and user corrections into permanent rules. Use after any agent error, test failure pattern, hook false positive, workflow friction, or user correction. Triggers on \"reflect\", \"learn from\", \"remember this\", \"add rule\", \"stabilize\", \"this keeps happening\", \"agent did wrong\", \"don't do this again\". Automates the error-to-rule cycle from STABILIZATION.md."
---

# Reflect

Automate the stabilization cycle: Error -> Root Cause -> Rule -> Example -> Enforce.

## Generated File Naming Convention (MANDATORY)

**ANY file created by this skill MUST be prefixed with `reflect-`:**
- `reflect-report.md`, `reflect-new-rules.md`

## Workflow

### Step 1: Gather the Incident

Identify what happened. Sources:
- User describes the problem directly
- Agent output from a previous command (passed as context)
- Hook warning/block output
- Test failure pattern

Extract:
- **What happened** (the error or unwanted behavior)
- **What was expected** (the correct behavior)
- **Where it happened** (which skill, agent, hook, or file)

If unclear, ask the user — max 2 questions.

### Step 2: Localize, Then Name the Root Cause

**2a. Localize.** Name the interaction edge and the responsible side first: `COMPONENT - COMPONENT | blame: SIDE`. The component list and the two attribution rules — label the earliest failure after which the run never recovered, and blame follows behavior rather than opportunity — are defined in the active edition's `STABILIZATION.md`.

The blamed side decides which repair is legitimate at all:

| Blame | Repair |
|-------|--------|
| Model | rule text: skill instruction, `AGENTS.md`, review checklist |
| Context, Memory, or Tool | harness change: hook, `context.py`, capsule or record template. A rule alone cannot fix what the model cannot observe |
| Local or external environment | `project-brain/` incident; a rule only when the condition was recoverable and the run failed to recover |
| Owner or Grader | `DOD.md`, `AGENTS.md`, or the request itself; report the contradiction to the user instead of resolving it silently |

Stop here when the environment is to blame and the condition was unrecoverable: there is nothing to stabilize. Say so, and record the incident instead of drafting a rule.

**2b. Root cause.** Classify into one of these causes:

| Root Cause | Description | Example |
|-----------|-------------|---------|
| Ambiguous instruction | Soft wish instead of hard constraint | "Try to use prefixes" vs "MUST prefix with skill name" |
| Missing context | Agent didn't know about a convention | No mention of zero-padded task dirs |
| Wrong model level | Task needed more reasoning than model provides | haiku doing root cause analysis |
| Missing enforcement | Rule exists but nothing checks it | Naming convention with no hook |
| Missing workflow step | No step exists for this situation | No verify-fix loop on test failure |
| Stale/conflicting docs | Two sources say different things | README says TASK-1, skills say TASK-001 |

### Step 3: Draft the Rule

Write a rule following this template:

```
### Rule: {Short Name}

**Trigger:** {What error was observed}
**Edge:** {COMPONENT - COMPONENT — from Step 2a}
**Blame:** {Responsible component — from Step 2a}
**Root cause:** {Why it happened — from Step 2b}
**Rule:** MUST/MUST NOT {enforceable statement}
**Example:**
- Incorrect: {concrete bad example}
- Correct: {concrete good example}
**Enforcement:** Hook / Skill instruction / Review checklist / Policy
**Added:** {today's date}
```

### Step 4: Determine Placement

Choose where the rule belongs:

| Scope | File | When |
|-------|------|------|
| Global policy | `AGENTS.md` | Applies to all agents and skills |
| Code style | the active edition's `GOLDEN-PRINCIPLES.md` | Naming, Symfony/PSR conventions, error handling, tests |
| Specific skill | the active edition's `skills/{name}/SKILL.md` | Only relevant to one skill's workflow |
| Process | the active edition's `STABILIZATION.md` | Add as example cycle for future reference |

Placement must agree with the blame from Step 2a. A Context, Memory, or Tool failure filed as one more policy sentence is a mislocalization, not a rule: name the harness change instead.

If enforcement is automatable, also identify which hook to create/update.

### Step 5: Present and Apply

Present the drafted rule to the user with:
1. The rule text
2. The target file
3. The exact location (section) where it will be added

**Wait for user approval before writing.**

After approval:
- Write the rule to the target file
- If adding to STABILIZATION.md, append as a new example cycle
- If a hook needs updating, describe the change (don't modify hooks without explicit approval)

### Step 6: Verify

After writing:
- Read back the modified file to confirm correct placement
- Check no existing rules were damaged
- Confirm the new rule doesn't conflict with existing ones

---

## Next Steps

After reflection is complete, STOP and present these options:

**Suggested follow-ups:**
- Test the new rule by re-running the scenario that triggered it.
- `verify` — Run DoD checklist if changes affect enforcement.
- `skill-creator` — If a new hook or skill modification is needed.
