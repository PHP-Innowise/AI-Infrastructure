---
name: frontend-design
description: Design Symfony frontend experiences using Twig, Symfony Forms, Symfony UX, Stimulus/Turbo, semantic HTML, CSS, and accessibility rules.
phase: planning
flow-next: writing-plans
flow-alternatives: [coder-frontend, api-designer]
---

# Symfony Frontend Design

Design UI around the project's existing Symfony frontend stack.

Design the complete user workflow around the consuming project's existing frontend stack. Do not introduce a new framework or asset pipeline without an explicit architecture decision.

## Required Context

Inspect routes/controllers, Twig layouts/components/macros, Forms/themes, translations, Stimulus controllers, Turbo/Live Components, AssetMapper/Encore/Vite configuration, CSS/design tokens, CSP, API contracts, authorization, existing UI patterns, analytics, and browser/functional tests.

Identify the audience, primary task, frequency, device/context constraints, data sensitivity, failure consequences, and accessibility needs before selecting interaction patterns.

## Stack Decision

- Prefer Twig + Symfony Forms for server-rendered workflows.
- Add Stimulus for focused client behavior and Turbo for navigation, frames, or streams when progressive enhancement remains intact.
- Use Live Components only when installed and when server-driven interactivity reduces overall state/contract complexity.
- Prefer AssetMapper for modest modern asset needs; retain Encore/Vite or an existing SPA when compilation, TypeScript, framework integration, team ownership, or ecosystem constraints justify it.
- Treat a separate frontend as an API, authentication, deployment, observability, and versioning decision rather than a template refactor.

Record why the existing stack is sufficient or why a change is justified, including migration and rollback costs.

## Page And State Contract

Define:

- route, controller/component, service call, voter/authorization behavior, and typed view model;
- document title, page heading, landmarks, navigation/breadcrumbs, content hierarchy, and primary action;
- initial/loading/success/empty/no-results/validation/authorization/not-found/conflict/offline/system-error states;
- redirects, flash messages, duplicate submission, refresh/back behavior, and preservation of user input;
- responsive layout, long content/translations, zoom/reflow, print behavior where relevant, and supported browsers;
- analytics/telemetry events without sensitive payloads.

Do not hide error or permission states behind generic blank screens.

## Forms And Input

Specify fields, types, labels, help, defaults, constraints, transformations, conditional behavior, autocomplete, keyboard order, and server-side error mapping. Define CSRF, upload limits, destructive-action confirmation, focus after failure, error summary, and resubmission/idempotency behavior.

Privileged values such as roles, ownership, tenant, price, audit metadata, or workflow state must not be trusted from hidden controls.

## Interaction Model

- Start with semantic HTML and a complete server round trip.
- Add Stimulus targets/actions/values only for local interaction state.
- Define Turbo Frame boundaries, navigation/history, redirects, validation rendering, stream targets, and fallback behavior.
- Define Live Component writable properties/actions, validation, authorization, hydration exposure, latency, and request frequency.
- For async API calls, define cancellation, stale response handling, retry, loading/error recovery, authentication/CSRF, and response contract.

## Accessibility Contract

Specify semantic elements, accessible names/descriptions, heading order, landmarks, keyboard interaction, visible focus, focus movement, live announcements, error association, contrast, non-color cues, reduced motion, touch targets, captions/alternatives, and 400% zoom/reflow behavior. Use native elements before ARIA and hand detailed review to `wcag-accessibility` when needed.

## Security And Privacy

- Preserve Twig auto-escaping and define any sanitized rich-text boundary.
- Keep authorization server-side and CSRF on state-changing browser requests.
- Avoid exposing sensitive entity fields, secrets, internal state, or personal data through HTML, JavaScript state, logs, analytics, or Turbo streams.
- Define CSP implications, allowed remote media/origins, upload behavior, and cache visibility for personalized pages.

Use the frontend boundary examples in [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md) to specify escaped output, CSRF-protected mutations, view models, and progressive enhancement.

## Verification Plan

Plan functional tests for server behavior, component/Stimulus tests where configured, template/build linting, and browser verification. Browser scenarios must cover keyboard-only use, invalid forms, focus, Turbo reconnect/navigation, no-JavaScript fallback, narrow/wide viewports, zoom/reflow, loading/error states, and no overlapping/clipped content.

## Design Output

Provide a route/page inventory, state matrix, Twig/component structure, Form/view-model contract, Stimulus/Turbo/Live Component behavior, authorization/security boundaries, responsive/accessibility rules, testing/browser plan, assumptions, and rollout risks. Record approved user-facing workflow decisions in `specs/`.
