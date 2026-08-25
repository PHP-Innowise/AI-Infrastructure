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

### Step 2: Root Cause Analysis

Classify into one of these causes:

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
**Root cause:** {Why it happened — from Step 2}
**Rule:** MUST/MUST NOT {enforceable statement}
**Example:**
- Incorrect: {concrete bad example}
- Correct: {concrete good example}
**Enforcement:** Hook / Skill instruction / Review checklist / Policy
**Added:** {today's date}
**Retired:** {omit while the rule is live}
**Superseded-by:** {omit unless another rule replaced this one}
```

### Step 4: Determine Placement

Choose where the rule belongs:

| Scope | File | When |
|-------|------|------|
| Global policy | `AGENTS.md` | Applies to all agents and skills |
| Code style | the active edition's `GOLDEN-PRINCIPLES.md` | Naming, Laravel conventions, error handling, tests |
| Specific skill | the active edition's `skills/{name}/SKILL.md` | Only relevant to one skill's workflow |
| Process | the active edition's `STABILIZATION.md` | Add as example cycle for future reference |

If enforcement is automatable, also identify which hook to create/update.

Write into the canonical tree — `.agents/skills/{name}/SKILL.md` — never into
the generated `.claude`/`.cursor` mirrors. Here, regenerate with
`python3 scripts/build_mirrors.py --write --edition <edition>`; in a consuming
project, where that tool is not installed, make the identical edit in every
tool tree present, or `context.py parity` will report a governance-document
drift you just created.

### Step 4b: Decide add, overwrite, or retire

A pillar that can only add runs out of budget: `AGENTS.md` is paid on every
session and gated in CI. Before writing, look for what this rule replaces.

```bash
python3 memory-bank/scripts/context.py index
python3 memory-bank/scripts/context.py search "<the rule's own words>" --layer procedural
```

Index first: a stale index returns nothing and the decision degrades to `add`
without saying so. The procedural layer covers `AGENTS.md`, `CLAUDE.md` and
skill bodies only, so candidates in `GOLDEN-PRINCIPLES.md` and
`STABILIZATION.md` must be found by reading them.

Present the candidates and decide explicitly — never default to `add`:

- **add** — nothing it replaces.
- **overwrite** — an existing rule says this less well; edit in place and
  leave its `Added:` date alone.
- **retire** — an existing rule no longer applies. Give it `Retired:` today
  and, if this replaces it, `Superseded-by:`; move the block to
  `## Retired rules` in `STABILIZATION.md` and delete the line it placed
  wherever it was enforced. That is what returns budget.

Report the remaining budget with `python3 scripts/context_budget.py --headroom`
where that maintainer tool exists; a consuming project quotes its own ceiling
instead.

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
