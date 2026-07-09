---
name: coder-frontend
description: Implement frontend work in native PHP projects using server-rendered templates, semantic HTML, CSS, and progressive enhancement with vanilla JS. Framework-neutral (no assumed JS framework).
phase: execution
flow-next: code-reviewer
flow-alternatives: [browser-verify, test-generator]
related: [frontend-design, coder, browser-verify]
---

# Coder Frontend

## Overview

Implement frontend changes for native PHP applications using server-rendered templates and progressive enhancement. Do not introduce a component library or a JavaScript framework without confirming it fits the project; framework-specific frontends belong on a framework-specific branch.

## Locate The Frontend

Check for:

- Templates: `templates/`, `views/`, `resources/views/`, or `*.php`/`*.phtml` view files.
- A templating engine if one is used (e.g. Twig `*.twig`).
- CSS/asset location and any build pipeline (`package.json`, a bundler config) if present.
- Static assets under `public/`.

## Implementation Rules

- Preserve server-side authorization. UI visibility is not authorization.
- Escape all dynamic output with `htmlspecialchars(...)` (or the engine's auto-escaping) to prevent XSS.
- Include a CSRF token in every state-changing form.
- Re-render forms with prior input and per-field errors on validation failure.
- Provide loading, empty, and error states for any async behavior.
- Maintain accessibility: labels, focus states, keyboard support, semantic landmarks (see `wcag-accessibility`).
- Progressive enhancement: baseline HTML must work without JavaScript, then enhance.
- Keep any frontend build/lint commands scoped to the project that owns them.

## Server-Rendered Form Pattern (plain PHP)

```php
<form method="post" action="/invitations">
    <input type="hidden" name="_csrf" value="<?= htmlspecialchars($csrfToken, ENT_QUOTES) ?>">

    <label for="email">Email</label>
    <input
        id="email"
        name="email"
        type="email"
        value="<?= htmlspecialchars($old['email'] ?? '', ENT_QUOTES) ?>"
        required
        <?= isset($errors['email']) ? 'aria-invalid="true" aria-describedby="email-error"' : '' ?>
    >
    <?php if (isset($errors['email'])): ?>
        <p id="email-error" role="alert"><?= htmlspecialchars($errors['email'], ENT_QUOTES) ?></p>
    <?php endif; ?>

    <button type="submit">Send invitation</button>
</form>
```

## Progressive Enhancement Notes

- Add small vanilla-JS behaviors (inline validation hints, disclosure toggles) on top of working HTML.
- Never move authorization or required validation into the client only.
- Avoid leaking secrets or hidden authorization state into rendered markup or data attributes.

## Output Escaping By Context (XSS)

Escaping is context-sensitive; `htmlspecialchars` alone is not always enough:

- **HTML body / attribute:** `htmlspecialchars($v, ENT_QUOTES, 'UTF-8')`; always quote attribute values.
- **URL parameter:** `urlencode()`/`rawurlencode()`; validate the scheme (allow only `http`/`https`/`mailto`) to block `javascript:` URLs.
- **Inside `<script>` / JS context:** prefer `json_encode($v, JSON_HEX_TAG | JSON_HEX_APOS | JSON_HEX_QUOT | JSON_HEX_AMP)`; do not hand-concatenate into scripts.
- **CSS context:** avoid injecting user data into styles; if unavoidable, allowlist strictly.

Consider a Content-Security-Policy header as defense-in-depth, and avoid inline event handlers/`style` so a stricter CSP is possible.

## Asset And Progressive-Enhancement Hygiene

- Keep JS unobtrusive: attach behavior via `addEventListener`, not inline `onclick`. Feature-detect; degrade gracefully.
- Version/cache-bust static assets; set sensible cache headers.
- Do not block first paint on non-critical scripts (`defer`/`async`); avoid layout shift by reserving image/media dimensions.
- Keep templates logic-light: format and escape in the view, compute in the handler/service.

## Verification

Possible checks:

```bash
composer test
php -l path/to/template.php
<frontend-lint-command>
<frontend-build-command>
```

Use only commands present in the project. Also verify markup against `wcag-accessibility` rules.

## Final Output

Return what changed, rendering approach used, tests/checks run, accessibility notes, Context Summary, and next step.
