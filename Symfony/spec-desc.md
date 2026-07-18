## Hybrid Approach: Index + Modular Files

**Neither extreme is ideal.** Use a **manifest file + focused spec files**.

```
specs/
├── MANIFEST.md          # Index + project overview (always read first)
├── architect-architecture.md      # For architect agent
├── api-designer-spec.md          # For api-designer agent
├── frontend-design-spec.md     # For frontend-designer agent
└── docs-generator-implementation.md    # Generated, for docs-generator
```

---

## MANIFEST.md Structure

```markdown
# Project: {Name}

{2-3 sentence core purpose}

## Specs Index

| File             | Purpose                              | Depends On             |
| ---------------- | ------------------------------------ | ---------------------- |
| architect-architecture.md  | System design, components, data flow | -                      |
| api-designer-spec.md      | Endpoints, schemas, auth             | architecture           |
| frontend-design-spec.md | Pages, components, state             | architecture, api-spec |

## Key Decisions

- Backend: native PHP (Composer + PSR)
- Auth: session auth or token/JWT depending on client needs
- DB: PostgreSQL or MySQL via PDO
- Frontend: server-rendered PHP templates with progressive enhancement, or an API-only backend
```

---

## Why This Works Better for AI

| Factor               | One Big File            | Separate + Manifest       |
| -------------------- | ----------------------- | ------------------------- |
| Token cost per agent | High (reads everything) | Low (reads only relevant) |
| Context relevance    | Diluted                 | Focused                   |
| Update complexity    | Error-prone             | Isolated changes          |
| Cross-referencing    | Implicit                | Explicit in manifest      |

---

## Format Tips for AI Consumption

1. **Use structured data where possible** — tables, YAML blocks, JSON schemas are more parseable than prose
2. **Front-load critical info** — put the most important context in first 20% of each file
3. **Explicit over implicit** — state relationships directly, don't assume inference
4. **Consistent headers** — same structure across files helps pattern matching

```markdown
## Component: Invitation Registration

**Purpose:** Register invited users
**Depends on:** Invitation model, User model, Registration policy
**Exposes:** accept invitation, validate token, create user
```

---

## Agent-Specific Loading Pattern

```
Architect agent:      reads MANIFEST.md only
API-designer agent:   reads MANIFEST.md → architect-architecture.md
Frontend agent:       reads MANIFEST.md → architect-architecture.md → api-designer-spec.md
Docs-generator:       reads MANIFEST.md → all specs + source code
```
