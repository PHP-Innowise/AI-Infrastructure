---
name: clarifying-interview
description: Turn the genuinely ambiguous or unverifiable items left by the six scanners and stack-researcher into a short, concrete question set for the user, always including the mandatory AI-tool-selection question, then record the answers. Takes a required target-project-path argument. Use after stack-researcher (or directly after the scanners if research flagged nothing) and before profile-synthesizer. Triggers on "clarifying interview", "ask the user", "what's still unknown", "which AI tool", "resolve open questions".
phase: synthesis
flow-next: profile-synthesizer
flow-alternatives: []
related: [infra-scan, stack-researcher, profile-synthesizer]
---

# Clarifying Interview

## Overview

`clarifying-interview` keeps friction low: it asks the user only what evidence could not settle, plus the one mandatory question about which AI tool(s) the target team uses. For a clean, conventional PHP project this may be a single question (the tool selection). It never fabricates answers and never assumes the tool selection from context.

The target project path is a **required** argument. This skill reads the current run's findings files to know what is still open; it never writes into the target.

## Generated File Naming Convention (MANDATORY)

Write two files into the current run's task directory:
- `tasks/TASK-{N}/clarifying-interview-questions.md`
- `tasks/TASK-{N}/clarifying-interview-answers.md`

## Process

1. **Gather open items.** Collect every finding marked `inferred` or `unknown` across the six `*-findings.md` and `stack-researcher-findings.md`. Discard items that do not change what gets generated (keep the question set short).
2. **Always include the AI-tool question** (mandatory, even if an edition folder exists elsewhere): "Which AI tool(s) does this project's team use - Claude Code, Cursor, Codex, or more than one?" Only the selected edition(s) will be generated.
3. **Phrase remaining questions concretely**, each tied to the finding it resolves and offering the most likely options. Examples: "A Redis client is in composer.json but no queue connection is configured - is Redis used in production (cache? queue? both?) or dev-only?"; "Two bounded contexts share a namespace - are these separate modules or one?"
4. **Ask.** Use the AI tool's structured question mechanism when available; otherwise write the questions to `clarifying-interview-questions.md` and ask the user to answer inline.
5. **Record answers** verbatim in `clarifying-interview-answers.md`, including the AI-tool selection as a discrete, machine-readable line (e.g. `editions: [cursor]`).
6. **Do not over-ask.** If nothing is genuinely ambiguous beyond the tool question, ask only that one.

## Output Template

```markdown
# Clarifying Interview: [target_name]

**Questions asked:** [count]
**AI tool(s) selected:** [claude|cursor|codex, ...]

## Resolved
- [finding -> answer]

## Still Unknown
- [anything the user could not answer -> stays `unknown` in the profile]
```

## Guardrails

- MUST always ask the AI-tool-selection question and record a concrete answer; MUST NOT assume it.
- MUST keep the question set minimal - only items that change generation.
- MUST NOT ask for secrets or credentials.
- MUST record answers verbatim; MUST NOT infer an answer the user did not give.

## Final Output

Return the two file paths, the AI-tool selection, a summary of what was resolved, and what remains `unknown`. Suggest `profile-synthesizer` as the next step.
