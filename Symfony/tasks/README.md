# Task Documentation

This directory contains temporary documentation for specific implementation tasks.

## Structure

Each task has its own directory: `TASK-001/`, `TASK-002/`, etc.

Task directories may contain:
- `requirements-analyst-requirements.md` - Requirements analysis (from /requirements-analyst)
- `brainstorming-design.md` - Design decisions (from /brainstorm)
- `writing-plans-plan.md` - Implementation plan (from /writing-plans)

## Task Numbering

Tasks are auto-numbered starting from 001 (zero-padded to 3 digits). The `.task-counter` file tracks the next number.

## When to Delete

Task directories are temporary. Delete them after:
- Implementation is complete
- Code is merged/deployed
- Team has reviewed the work

Keep task docs if you need to reference the original requirements or design decisions. Otherwise, the living specifications in `specs/` provide ongoing documentation.

## Living Specifications

For ongoing project documentation, see `specs/`:
- `specs/MANIFEST.md` - Project overview and spec index
- `specs/architect-architecture.md` - System architecture
- `specs/api-designer-spec.md` - API endpoints, requests, resources, auth, and error contracts
- `specs/frontend-design-spec.md` - UI components and design system
- `specs/docs-generator-implementation.md` - Build process, deployment, tooling
