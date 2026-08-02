---
name: codebase-mapper
description: Map an existing Symfony codebase into `codebase/` documents that an agent reads instead of the source. Use before planning work in unfamiliar code, when onboarding onto a brownfield project, or when the existing map has drifted from the code. Triggers on "map the codebase", "how is this project structured", "where does X live", "orient me in this project".
phase: understanding
flow-next: architect
flow-alternatives: [researcher, requirements-analyst, writing-plans]
related: [researcher, architect, architecture-implementer, memory]
---

# Codebase Mapper

`codebase-mapper` is an AI skill/command name, not a shell executable. It
writes documents *about* code. It never edits code, and the map it produces
never outranks the source it describes.

## Why the Map Exists

Indexing covers policy, specifications, documentation, and governed records —
never arbitrary application source. That boundary keeps retrieval bounded and
auditable, but it leaves one question unanswerable: where does behavior
actually live? The map is the single indexed document that answers it, so
retrieval can locate `src` code without indexing every file in it.

A map is a convenience, not evidence. Anything acted upon must be confirmed in
the source it cites.

## Authority Gate

1. Resolve the repository root with `git rev-parse --show-toplevel`; stop
   safely if it fails.
2. Record `git rev-parse HEAD`. Every document written in this run carries that
   exact commit.
3. Resolve the mapped scope. Default it to `src`; use a narrower path only
   when the caller names one, and record whichever was used.

## Outputs

Write these under `codebase/` at the repository root:

| File | Covers |
| --- | --- |
| `STACK.md` | PHP version, Symfony version, key dependencies, tooling |
| `INTEGRATIONS.md` | External APIs, databases, queues, mail, storage, auth providers |
| `ARCHITECTURE.md` | Layers, data flow, entry points (`public/index.php`, `config/routes/`, and `src/Kernel.php`) |
| `STRUCTURE.md` | Directory layout (`src/`, `config/`, `templates/`, `migrations/`) and where new code belongs |
| `CONVENTIONS.md` | Naming, typing, error handling, Doctrine entities, repositories, and migrations |
| `TESTING.md` | PHPUnit layout, fixtures, doubles, how to run the suite |
| `CONCERNS.md` | Technical debt, known defects, security and performance risks |

Write only the documents the run actually covered. A missing file is honest; a
file asserting coverage that did not happen is not.

## Required Frontmatter

Every document begins with exactly these keys:

```text
---
description: <one line naming what this document covers>
mapped_commit: <full SHA recorded in the authority gate>
mapped_scope: src
---
```

- `description` is indexed and weighted far above the body, so it decides
  whether the document is found at all. State the subject, not the genre.
- `mapped_commit` is what freshness is measured against. Retrieval counts the
  commits landed on `mapped_scope` since it, and excludes the document once
  that exceeds the configured limit.
- Omitting `mapped_commit` makes retrieval exclude the document as
  `map-unverifiable`. That is deliberate: an unverifiable map is the one not to
  trust.

## Content Rules

- Every claim cites a path in backticks, for example
  `` `src/Billing/InvoiceTotal.php` ``. A claim with no path is deleted,
  not softened.
- Record current state only. No "was", "used to", "is being migrated to".
- Never record a secret value. `.env`, key material, and credential files are
  noted by existence only and never opened.
- Never invent structure. Areas not explored are listed as not explored.
- Regeneration replaces a document whole. Do not append revisions to it; the
  commit in the frontmatter is what dates it.

## Workflow

1. Pass the authority gate and fix the commit and scope for the whole run.
2. Explore one focus area at a time with `Glob` and `Grep`. Never read the tree
   up front: the point of the map is to spend that cost once, not per request.
3. Write each document whole, with the frontmatter above.
4. Refresh the index and report each document, its scope, and its drift.

## Safety

- MUST NOT read `.env`, `.env.*`, `*.key`, `*.pem`, credential files, or any
  Git-ignored path.
- MUST NOT modify application code, specifications, Project Brain records,
  Memory Bank chunks, or changelogs.
- MUST NOT stage, commit, or discard changes.
- MUST NOT assert coverage of an area it did not explore.
- MUST NOT present the map as authority; canonical sources outrank it.
