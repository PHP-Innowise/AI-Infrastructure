---
name: coder-frontend
description: Implement Symfony frontend work with Twig, Forms, Symfony UX, Stimulus/Turbo, semantic HTML, CSS, and accessibility.
phase: execution
flow-next: code-reviewer
flow-alternatives: [browser-verify, verify]
---

# Symfony Frontend Coder

Implement frontend changes using existing project conventions.

## Required Context

Inspect controllers/Twig components, template inheritance, macros, Form types/themes, translations, asset configuration, Stimulus controllers, Turbo/Live Component usage, CSS tokens/layout conventions, CSP, and existing functional/browser tests. Preserve the installed stack; do not introduce a frontend framework or build pipeline without an approved architecture decision.

## Boundary Rules

- Controllers/components prepare typed view data, authorize, and delegate workflows to services.
- Twig renders presentation. It does not query Doctrine, make external calls, mutate entities, or decide business policy.
- Form types and request DTOs define input shape; Validator constraints protect the boundary; services own stateful decisions.
- Stimulus, Turbo, and Live Components enhance a complete server-rendered workflow rather than becoming a hidden business layer.
- Public JSON calls follow the API contract and CSRF/authentication rules; never duplicate server authorization in JavaScript only.

## Twig Implementation

- Extend the established base layout and reuse existing components/macros before adding another abstraction.
- Preserve auto-escaping. Use context-correct escaping for HTML, attributes, URLs, JavaScript, and JSON.
- Render `|raw` only for content sanitized by a trusted server-side policy; document the trust boundary in code.
- Avoid broad entity exposure that can trigger lazy loading or leak sensitive fields. Prefer explicit view DTOs for complex pages.
- Use semantic landmarks, one logical `h1`, ordered headings, real buttons/links, labelled tables, and meaningful empty states.
- Keep translation keys stable and pass variables explicitly; do not concatenate translated fragments that break grammar.

Example structure:

```twig
{% extends 'base.html.twig' %}

{% block body %}
  <main id="main-content">
    <h1>{{ page.title }}</h1>
    {% include 'invitation/_form.html.twig' with { form: form } only %}
  </main>
{% endblock %}
```

## Symfony Forms

- Set method/action deliberately and keep CSRF enabled for state-changing browser forms.
- Render visible labels, required state, help, field errors, and an error summary where multiple failures are likely.
- Ensure errors/help are associated through Form rendering or explicit `aria-describedby`; focus the summary/first invalid field after failure where appropriate.
- Use correct `autocomplete`, `inputmode`, input type, constraints, empty-data behavior, transformers, and collection prototypes.
- Do not bind privileged entity fields, ownership, roles, prices, tenant IDs, or workflow state directly from submitted data.
- Preserve submitted values on validation failure and prevent double submission when the workflow is not naturally idempotent.

## Stimulus And Turbo

- Keep controllers small, target/value/class driven, and safe across repeated `connect()`/`disconnect()` under Turbo.
- Remove listeners, observers, timers, and pending requests during disconnect; cancel or ignore stale responses.
- Provide a working non-JavaScript submit/navigation path and expose loading, success, empty, and recoverable error states.
- For Turbo Frames, return correct frame content, redirects, and validation status codes; handle missing-frame responses deliberately.
- For Turbo Streams, authorize every mutation server-side and keep stream targets stable and unique.
- Mark persistent elements only when their lifecycle and state transfer are understood.
- Live Component writable properties and actions require validation, authorization, bounded payloads, and hydration-exposure review.

Example controller lifecycle:

```js
import { Controller } from '@hotwired/stimulus';

export default class extends Controller {
  connect() {
    this.abortController = new AbortController();
  }

  disconnect() {
    this.abortController.abort();
  }
}
```

## CSS And Responsive Behavior

- Reuse design tokens and existing layout primitives. Avoid page-local near-duplicates.
- Define stable dimensions for toolbars, grids, media, and asynchronous regions so loading and error states do not shift the layout.
- Support narrow mobile, zoom/reflow at 400%, long translations/content, keyboard focus, reduced motion, high contrast, and touch targets.
- Never communicate state by color alone. Keep focus visible and verify text/background and UI-component contrast.
- Avoid fixed heights for text containers and hidden overflow that clips validation or translated content.

## Security

- Preserve Twig auto-escaping and sanitize trusted rich text before storage/rendering.
- Keep CSRF on state-changing browser actions and use same-origin/CORS policy intentionally for API calls.
- Do not place secrets, authorization decisions, internal identifiers, or sensitive serialized entities in HTML data attributes.
- Validate URLs and allowed origins before rendering user-controlled links, embeds, or remote media.
- Follow the project's Content Security Policy; avoid new inline script/style exceptions.

Use the Twig/Form/progressive-enhancement examples in [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md) as a security and responsibility check.

## Testing And Verification

Add the lowest useful evidence:

- functional tests for route, authorization, form submission, validation, redirect, flash, and rendered contract;
- component/Live Component tests for server-driven state;
- JavaScript tests for stateful Stimulus logic when configured;
- browser verification for focus, keyboard, Turbo lifecycle, responsive layout, loading/error states, and no-JavaScript fallback.

Run configured equivalents:

```bash
php bin/console lint:twig
vendor/bin/phpunit
npm test
npm run lint
npm run build
```

Report unavailable tooling as N/A. Use `browser-verify` when a runnable application exists.

## Output

Report templates/components/controllers/assets changed, server/client boundary decisions, accessibility and security behavior, tests/build/browser checks, unavailable tooling, and remaining risks. Include Context Summary and Next Steps.
