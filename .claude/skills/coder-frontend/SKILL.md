---
name: coder-frontend
description: Implement frontend work in Laravel projects using Blade, Livewire, Inertia, or project-specific JavaScript tooling.
phase: execution
flow-next: code-reviewer
flow-alternatives: [browser-verify, test-generator]
related: [frontend-design, coder, browser-verify]
---

# Coder Frontend

## Overview

Implement frontend changes according to the project's existing Laravel frontend stack. Do not introduce Blade, Livewire, Inertia, a separate JavaScript app, or a component library without confirming it fits the existing project.

## Locate The Frontend

Check for:

- Blade: `resources/views/**/*.blade.php`
- Livewire: `app/Livewire`, `resources/views/livewire`
- Inertia: `resources/js/Pages`, `resources/js/Layouts`
- Vite: `vite.config.js` or `vite.config.ts`
- Separate frontend app: `frontend/`, `apps/frontend/`, or similar

## Implementation Rules

- Preserve server-side authorization. UI visibility is not authorization.
- Keep validation feedback aligned with Laravel Form Request errors.
- Use existing components and design tokens.
- Include loading, empty, and error states for async behavior.
- Maintain accessibility: labels, focus states, keyboard support, semantic landmarks.
- Keep frontend commands scoped to the project that owns the frontend tooling.

## Blade Pattern

```blade
<form method="POST" action="{{ route('invitations.store') }}">
    @csrf

    <label for="email">Email</label>
    <input id="email" name="email" value="{{ old('email') }}" required>

    @error('email')
        <p role="alert">{{ $message }}</p>
    @enderror

    <button type="submit">Send invitation</button>
</form>
```

## Inertia/SPA Notes

- Treat controller props and API Resources as public contracts.
- Avoid leaking hidden authorization state or secrets into props.
- Use the existing state/data-fetching library.
- Run frontend build/lint commands only if configured.

## Verification

Possible checks:

```bash
php artisan test
<frontend-lint-command>
<frontend-build-command>
```

Use only commands present in the project.

## Final Output

Return what changed, frontend stack used, tests/checks run, accessibility notes, Context Summary, and next step.
