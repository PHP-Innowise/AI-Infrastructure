# Example Epic: WordPress Content Platform

This source-only example shows the kind of client-owned requirements that may
be placed under uppercase `Task/`. It is not installed into consuming projects
and is not an instruction to generate these features automatically.

## Goal

Provide an editorial content type with structured metadata, block-editor
support, a public read API, capability-protected management, accessible theme
templates, and an idempotent data migration.

## Acceptance Boundaries

- The content model states post type, taxonomy, metadata and REST contracts.
- Editors receive only capabilities required for the workflow.
- Existing URLs and stored block content remain compatible.
- Public output is escaped, translatable and keyboard accessible.
- Queries remain bounded on representative production-scale data.
- Migration, rollback and multisite scope are documented and tested.
