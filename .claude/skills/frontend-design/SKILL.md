---
name: frontend-design
description: Design user interfaces for Laravel projects using Blade, Livewire, Inertia, or a separate frontend when appropriate.
phase: planning
flow-next: writing-plans
flow-alternatives: [coder-frontend, api-designer]
related: [api-designer, coder-frontend, wcag-accessibility, web-design-guidelines]
---

# Frontend Design

## Overview

Design Laravel-compatible frontend experiences. Do not assume one frontend stack until the project reveals it.

## Stack Decision

| Need | Good fit |
| --- | --- |
| Server-rendered pages, simple forms | Blade |
| Dynamic Laravel-native interfaces | Livewire |
| SPA-like UX with Laravel routing/backend | Inertia with the project's chosen adapter |
| Separate product frontend | API-only Laravel plus the team's chosen frontend stack |
| Admin CRUD/productivity | Filament, Nova, or project-standard admin tooling |

## Required Context

Read:

- Existing `resources/views`, `resources/js`, `routes/web.php`, `routes/api.php`.
- Existing design tokens and components.
- API design when frontend consumes API endpoints.
- Accessibility rules in `wcag-accessibility`.

## Design Output

Include:

- User flow.
- Page/component list.
- Form fields, validation feedback, loading states, empty states, and error states.
- Accessibility notes.
- Responsive behavior.
- Data dependencies and endpoints.
- Recommended Laravel frontend stack for this feature.

## Laravel UI Notes

- Blade forms should include CSRF protection.
- Livewire components should keep authorization server-side.
- Inertia pages should treat server props as a contract and avoid exposing secrets.
- API-only frontends should rely on documented API Resources and auth behavior.

## Final Output

Return the design, stack recommendation, implementation notes, Context Summary, and next step.
