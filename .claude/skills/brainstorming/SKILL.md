---
name: brainstorming
description: "You MUST use this before any creative work - creating features, building components, adding functionality, or modifying behavior. Explores user intent, requirements and design before implementation."
phase: understanding
flow-next: architect
flow-alternatives: [writing-plans, api-designer]
related: [requirements-analyst, writing-plans]
---

# Brainstorming Ideas Into Designs

## Overview

Help turn ideas into fully formed designs and specs through natural collaborative dialogue.

Start by understanding the current project context, then ask questions one at a time to refine the idea. Once you understand what you're building, present the design in small sections (200-300 words), checking after each section whether it looks right so far.

## Generated File Naming Convention (MANDATORY)

**ANY file created by this skill MUST be prefixed with `brainstorming-`.**
Predefined output (`brainstorming-design.md`) already follows this convention.
Any additional ad-hoc files (summaries, notes, explorations) MUST also follow this rule:
- ✅ `brainstorming-alternatives.md`
- ❌ `ALTERNATIVES.md`

## The Process

**Understanding the idea:**
- Check out the current project state first (files, docs, recent commits)
- Ask questions one at a time to refine the idea. **Use AskUserQuestion tool.**
- Prefer multiple choice questions when possible, but open-ended is fine too
- Only one question per message - if a topic needs more exploration, break it into multiple questions
- Focus on understanding: purpose, constraints, success criteria

**For new projects - gather tech stack info:**
If this is a new project (no existing codebase), ask about tech stack. Keep it simple - these are small projects with straightforward architecture. This branch assumes Symfony and Controller -> Service -> Repository layering.

| Question | Options | Default |
|----------|---------|---------|
| Project type? | Symfony API / Symfony full-stack (server-rendered) / Frontend / Full-stack | - |
| Database? (backend) | PostgreSQL / MySQL / SQLite / Existing database / None | PostgreSQL |
| Rendering? (frontend) | Twig/Symfony UX / Separate frontend / API-only / Existing stack | Twig/Symfony UX |

**Architecture defaults (don't over-question):**
- Backend: Symfony routes/controllers, request DTOs/Forms/Validator, application services, Doctrine repositories, voters, Messenger, Symfony DI
- Business logic: keep controllers thin and put workflows in services
- Frontend: Twig, Symfony Forms, Symfony UX/Stimulus/Turbo, or an API-only backend

**Library vs Custom decisions (MANDATORY):**
- When the design involves functionality that established libraries solve (logging, validation, HTTP clients, date handling, state management, auth, caching, etc.), you MUST ask the user whether they want to use an existing library or build a custom solution.
- **Default to recommending well-known libraries** - custom implementations should require justification.
- Use AskUserQuestion with options like: `["Use <library-name> (Recommended)", "Use <alternative-library>", "Custom implementation"]`. The built-in "Other" option lets the user suggest their own library — mention this explicitly in the question text (e.g., "Pick a library, or choose Other to suggest your own").
- If the user chooses a library, research it (via WebSearch or Context7) to confirm it fits their stack and is actively maintained.
- If the user wants custom, ask WHY - ensure there's a real reason (licensing, bundle size, specific requirements) and document the rationale in the design doc.
- **Never silently decide to write custom code when a proven, maintained package exists.** This prevents reinventing the wheel (e.g., using Symfony Security, Validator, Forms, Messenger, Cache, HttpClient, Monolog, Doctrine, or maintained bundles instead of hand-rolling fragile equivalents).

**Exploring approaches:**
- Propose 2-3 different approaches with trade-offs
- Present options conversationally with your recommendation and reasoning
- Lead with your recommended option and explain why

**Presenting the design:**
- Once you believe you understand what you're building, present the design
- Break it into sections of 200-300 words
- Ask after each section whether it looks right so far. **Use AskUserQuestion tool.**
- Cover: architecture, components, data flow, error handling, testing
- Be ready to go back and clarify if something doesn't make sense

## Design Document Template

```markdown
# [Feature Name] Design

## Problem Statement
[What problem are we solving?]

## Proposed Solution
[High-level approach]

## Architecture
[Components, layers, interactions]

## Data Model
[Doctrine entities, relationships, migrations, indexes, repositories, request/response contracts]

## API Design (if applicable)
[Endpoints, request/response formats]

## Error Handling
[Error cases, recovery strategies]

## Testing Strategy
[Unit, integration, e2e approach]

## Security Considerations
[Authentication, authorization, data protection]

## Open Questions
[Any unresolved decisions]
```

## Final Output (MANDATORY)

**Before presenting next steps, you MUST write the design document to a file:**

### Task Numbering Logic

1. **Check if task number provided:**
   - If coming from `/requirements-analyst`, use the task number from context (e.g., "TASK-001")
   - If no task number: run task counter logic (same as requirements-analyst)

2. **Task counter logic (if no task number provided):**
   - If `tasks/.task-counter` exists: read the number, use it, increment and write back
   - If missing: scan `tasks/` for existing `TASK-*` directories, use max(N) + 1, create counter file

3. **Create task directory:** `tasks/TASK-{N}/` (if not already created)

4. **Write design:** `tasks/TASK-{N}/brainstorming-design.md`
   - Use the Design Document Template as the structure
   - Include all design decisions, architecture, data model, API design, and open questions

**Example:** For TASK-001, writes `tasks/TASK-001/brainstorming-design.md`

This file preserves the design context so the conversation can be cleared before implementation.

## Key Principles

- **One question at a time** - Don't overwhelm with multiple questions
- **Multiple choice preferred** - Easier to answer than open-ended when possible
- **YAGNI ruthlessly** - Remove unnecessary features from all designs
- **Explore alternatives** - Always propose 2-3 approaches before settling
- **Incremental validation** - Present design in sections, validate each
- **Be flexible** - Go back and clarify when something doesn't make sense

---

## Next Steps

After design document is written to file, STOP and present these options:

**Next by flow:** /writing-plans `[TASK-{N} context]` - Create detailed implementation tasks from the design.

**Pass to next skill:** Include the task number in your context summary (e.g., "TASK-001: User notifications design completed")

**Alternatives:**
- /architect `[TASK-{N} context]` - Review architecture implications before creating the plan.
- /api-designer `[TASK-{N} context]` - Design REST APIs if the feature involves API work.
