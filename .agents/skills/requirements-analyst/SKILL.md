---
name: requirements-analyst
description: Analyze requirements from Confluence pages, decompose into actionable tasks, and validate completeness. Use when user asks to analyze requirements, break down features, create tasks from specs, parse Confluence docs, or validate requirements. Triggers on "analyze requirements", "break down feature", "create tasks from", "parse confluence", "requirements from", "decompose into tasks".
phase: understanding
flow-next: brainstorming
flow-alternatives: [architect, writing-plans]
related: [brainstorming, architect]
---

# Requirements Analyst

## Overview

Parse requirements from various sources (Confluence, specs, user requests), decompose into actionable tasks, and validate completeness.

**Announce at start:** "I'm using the requirements-analyst skill to analyze and decompose these requirements."

## Generated File Naming Convention (MANDATORY)

**ANY file created by this skill MUST be prefixed with `requirements-analyst-`.**
Predefined output (`requirements-analyst-requirements.md`) already follows this convention.
Any additional ad-hoc files (summaries, notes, reports) MUST also follow this rule:
- ✅ `requirements-analyst-gap-analysis.md`
- ❌ `GAP_ANALYSIS.md`

## The Process

### Step 1: Gather Requirements

**From Confluence/Documents:**
- Extract functional and non-functional requirements
- Note acceptance criteria

**From User Input:**
- Ask clarifying questions (one at a time). **Use AskUserQuestion tool.**
- Prefer multiple choice when possible
- Extract implicit requirements

### Step 2: Analyze and Categorize

**Requirement Categories:**

| Category | Description | Examples |
|----------|-------------|----------|
| Functional | What the system should do | "Users can upload files" |
| Non-Functional | Quality attributes | "Response time < 200ms" |
| Business Rules | Domain constraints | "Orders over $100 get free shipping" |
| Integration | External systems | "Sync with Stripe API" |
| Security | Auth/Authorization | "Only admins can delete users" |

### Step 3: Decompose to Tasks

**For each requirement, identify:**

```markdown
## Requirement: [Name]

### Backend Tasks
- [ ] Migration: [schema change]
- [ ] Eloquent Model: [domain entity, relationships, casts]
- [ ] Form Request: [input validation and authorization gate]
- [ ] Policy/Gate: [authorization rule]
- [ ] Controller/Action: [route handler]
- [ ] API Resource: [response contract shape]
- [ ] Job/Event/Listener/Notification: [only when workflow complexity needs it]
- [ ] Factory/Seeder: [test and local data]

### Frontend Tasks (Blade / Livewire / Inertia)
- [ ] Blade view / Livewire component / Inertia page: [view]
- [ ] Vite asset: [CSS/JS bundling, component styling]
- [ ] Progressive enhancement: [minimal JS behavior if not using Livewire/Inertia]
- [ ] Accessibility: [semantic markup, labels, focus]

### Testing Tasks
- [ ] Feature Tests: HTTP behavior, validation, authorization, persistence (PHPUnit/Pest)
- [ ] Unit Tests: pure logic, Actions/Services, value objects
- [ ] Browser/E2E Tests: user flows via Dusk or Playwright if tooling exists
```

### Step 4: Validate Completeness

**Validation Checklist:**

- [ ] All functional requirements mapped to tasks
- [ ] Happy path covered
- [ ] Error cases identified
- [ ] Edge cases considered
- [ ] Security requirements addressed
- [ ] Performance requirements noted
- [ ] Testing strategy defined

### Step 5: Output Requirements Document

```markdown
# [Feature Name] Requirements

## Overview
[Brief description]

## Source
[Confluence link / User request / etc.]

## Functional Requirements
1. FR-001: [Requirement]
   - Acceptance: [Criteria]
   - Priority: [High/Medium/Low]

## Non-Functional Requirements
1. NFR-001: [Requirement]
   - Metric: [Measurable criteria]

## Business Rules
1. BR-001: [Rule]

## Task Breakdown

### Eloquent Models
| Model | Attributes | Relations |
|-------|------------|-----------|
| X | name, status | belongsTo Y |

### Actions/Services
| Class | Purpose | Methods |
|-------|---------|---------|
| XService | Handle X operations | create, update, delete |

### Controllers/Routes
| Controller | Route | Purpose |
|------------|-------|---------|
| XController | `/api/x` (routes/api.php) | CRUD for X |

## Gap Analysis
- [ ] [Any unclear requirements]
- [ ] [Missing information]

## Next Steps (Suggested)
[Do not auto-execute, present to user]
```

---

## Final Output (MANDATORY)

**Before presenting next steps, you MUST write the requirements document to a file:**

### Task Numbering Logic

1. **Read task counter:**
   - If `tasks/.task-counter` exists: read the number, use it, increment and write back
   - If missing: scan `tasks/` for existing `TASK-*` directories, use max(N) + 1, create counter file

2. **Create task directory:** `tasks/TASK-{N}/` (e.g., `tasks/TASK-001/`)

3. **Write requirements:** `tasks/TASK-{N}/requirements-analyst-requirements.md`
   - Use the Requirements Document Template from Step 5 as the structure
   - Include all gathered requirements, task breakdown, and gap analysis

**Example:** First task creates `tasks/TASK-001/requirements-analyst-requirements.md`, increments counter to 2

**Atomic counter update to prevent race conditions:**
```bash
# Read current counter
current=$(cat tasks/.task-counter)
# Create task directory
mkdir -p tasks/TASK-$(printf "%03d" $current)
# Write requirements file
# Increment counter
echo $((current + 1)) > tasks/.task-counter
```

This file preserves the analysis context so the conversation can be cleared before implementation.

---

## Next Steps

After requirements document is written to file, STOP and present these options:

**Next by flow:** [[/brainstorm]] `[TASK-{N} context]` - Refine requirements into a concrete design through collaborative dialogue. See [[moc-understanding]] for phase context.

**Pass to next skill:** Include the task number in your context summary (e.g., "TASK-001: User authentication requirements analyzed")

**Alternatives:**
- [[/architect]] `[TASK-{N} context]` - Skip brainstorming if requirements are clear and jump to architecture decisions.
- [[/writing-plans]] `[TASK-{N} context]` - Create implementation plan directly if design is already established.
