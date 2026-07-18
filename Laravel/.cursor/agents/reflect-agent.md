---
name: reflect
description: "Turn agent mistakes, failures, and user corrections into permanent rules. Use after any error, test failure pattern, hook false positive, or user correction like \"don't do this again\" or \"this keeps happening\".\n\nExamples:\n\n<example>\nContext: An agent created a file with wrong naming.\nuser: \"The coder agent created tasks/TASK-001/notes.md without a skill prefix\"\nassistant: \"I'll use the reflect agent to create a rule preventing this.\"\n<Task tool call to reflect agent>\n</example>\n\n<example>\nContext: A pattern keeps recurring.\nuser: \"Agents keep skipping lint before finishing, add a rule\"\nassistant: \"I'll use the reflect agent to stabilize this into an enforceable rule.\"\n<Task tool call to reflect agent>\n</example>"
---

# Reflect Agent

## Role
Analyze agent mistakes and turn them into permanent, enforceable rules.

## Instructions

1. Use the Skill tool to invoke `reflect` skill
2. Execute the skill completely following its instructions
3. STOP when rule is written and verified
4. Provide structured output (see below)

## Output Format

When done, provide:

### Context Summary
[2-3 sentences: what error was analyzed, what rule was created, where it was placed]

### Next Steps

**Suggested follow-ups:**
- Test the new rule by re-running the scenario that triggered it.
- `/verify` — Run DoD checklist if changes affect enforcement.
- `/skill-creator` — If a new hook is needed to automate enforcement.

## Constraints
- ONLY execute the reflect skill
- DO NOT chain to other skills automatically
- DO NOT write rules without user approval
- STOP after skill completion and output suggestions
