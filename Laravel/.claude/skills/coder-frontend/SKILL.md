---
name: coder-frontend
description: Implement frontend work in Laravel projects using Blade templates/components, Livewire components, or Inertia.js pages (Vue/React/Svelte), with Vite for asset compilation.
phase: execution
flow-next: code-reviewer
flow-alternatives: [browser-verify, test-generator]
related: [frontend-design, coder, browser-verify]
---

# Coder Frontend

## Overview

Implement frontend changes for Laravel applications using whichever stack the design specified or the project already uses: Blade (optionally with Alpine.js), Livewire, or Inertia.js. Do not introduce a new stack (e.g. add Inertia to a Blade-only app, or a different JS framework) without confirming it fits the project.

## Locate The Frontend

Check for:

- Blade views/components: `resources/views/**/*.blade.php`, `resources/views/components/`.
- Livewire components: `app/Livewire/` (or `app/Http/Livewire/` on older versions) plus matching Blade views in `resources/views/livewire/`.
- Inertia pages: `resources/js/Pages/**/*.{vue,jsx,tsx,svelte}`, shared layout components, and the root Blade template (`resources/views/app.blade.php`) that boots Inertia.
- Alpine.js usage (`x-data`, `x-on`, `@click`) inline in Blade views, or `resources/js/app.js` where Alpine/Livewire/Inertia are bootstrapped.
- Vite config (`vite.config.js`) and entrypoints declared there.
- Static assets under `public/` and `resources/css`/`resources/js`.

## Implementation Rules

- Preserve server-side authorization (Policies/Gates in controllers or Livewire component methods). UI visibility is not authorization.
- Rely on Blade's `{{ }}` auto-escaping for dynamic output; never use `{!! !!}` on untrusted data.
- Include `@csrf` in every state-changing Blade form; verify Livewire/Inertia requests carry the CSRF/XSRF token via Laravel's default session/Axios setup.
- Re-render forms with prior input and per-field errors on validation failure (`old()` + `$errors` in Blade, `$this->validate()` + public properties in Livewire, `form.errors` from Inertia's `useForm`).
- Provide loading, empty, and error states for any async behavior (`wire:loading`, `wire:offline` in Livewire; `form.processing`, `router.on('start'/'finish')` in Inertia).
- Maintain accessibility: labels, focus states, keyboard support, semantic landmarks (see `wcag-accessibility`).
- For Blade + Alpine.js pages, keep the baseline HTML flow working without JS where feasible, then enhance with Alpine.
- Compile assets through Vite; never hand-edit built files in `public/build`.

## Blade Component And Layout Pattern

```blade
{{-- resources/views/components/invitation-form.blade.php --}}
@props(['errors' => null, 'old' => []])

<form method="POST" action="{{ route('invitations.store') }}">
    @csrf

    <label for="email">Email</label>
    <input
        id="email"
        name="email"
        type="email"
        value="{{ old('email', $old['email'] ?? '') }}"
        required
        @if ($errors?->has('email')) aria-invalid="true" aria-describedby="email-error" @endif
    >
    @if ($errors?->has('email'))
        <p id="email-error" role="alert">{{ $errors->first('email') }}</p>
    @endif

    <button type="submit">Send invitation</button>
</form>
```

```blade
{{-- resources/views/layouts/app.blade.php --}}
<!DOCTYPE html>
<html lang="{{ str_replace('_', '-', app()->getLocale()) }}">
<head>
    <meta charset="utf-8">
    <title>@yield('title', config('app.name'))</title>
    @vite(['resources/css/app.css', 'resources/js/app.js'])
</head>
<body>
    <main>@yield('content')</main>
</body>
</html>
```

Usage: `<x-invitation-form :errors="$errors" :old="old()" />`, or `@extends('layouts.app')` / `@section('content') ... @endsection` for page-level layouts.

## Livewire Component Pattern

```php
// app/Livewire/InvitationForm.php
namespace App\Livewire;

use Livewire\Component;

class InvitationForm extends Component
{
    public string $email = '';

    protected function rules(): array
    {
        return ['email' => ['required', 'email', 'unique:invitations,email']];
    }

    public function send(): void
    {
        $this->validate();

        Invitation::create(['email' => $this->email]);

        $this->reset('email');
        $this->dispatch('invitation-sent');
    }

    public function render()
    {
        return view('livewire.invitation-form');
    }
}
```

```blade
{{-- resources/views/livewire/invitation-form.blade.php --}}
<form wire:submit="send">
    <label for="email">Email</label>
    <input id="email" type="email" wire:model="email" required>
    @error('email') <p role="alert">{{ $message }}</p> @enderror

    <button type="submit" wire:loading.attr="disabled" wire:target="send">
        Send invitation
    </button>
</form>
```

Validate with `$this->validate()` (or `#[Validate]` attributes) inside the component class, never trust client-side state. Use `wire:model.live` only when real-time updates are actually needed; prefer deferred `wire:model` for plain forms to cut network chatter.

Check which Livewire major version the project is on (`composer.json`, `livewire/livewire` constraint) before reaching for Volt. Livewire v4 ships native single-file components — `new class extends Component { ... }` inline in one `.blade.php` file, as shown above — so on v4 projects prefer that built-in format over adding `livewire/volt` as a new dependency. Volt is still a reasonable, supported choice on projects still pinned to Livewire v3.

## Inertia Page Pattern

```php
// app/Http/Controllers/InvitationController.php
public function create(): \Inertia\Response
{
    return Inertia::render('Invitations/Create');
}

public function store(StoreInvitationRequest $request): \Illuminate\Http\RedirectResponse
{
    Invitation::create($request->validated());

    return to_route('invitations.index');
}
```

```vue
<!-- resources/js/Pages/Invitations/Create.vue -->
<script setup>
import { useForm } from '@inertiajs/vue3'

const form = useForm({ email: '' })

function submit() {
  form.post(route('invitations.store'))
}
</script>

<template>
  <form @submit.prevent="submit">
    <label for="email">Email</label>
    <input id="email" v-model="form.email" type="email" required>
    <p v-if="form.errors.email" role="alert">{{ form.errors.email }}</p>

    <button type="submit" :disabled="form.processing">Send invitation</button>
  </form>
</template>
```

The React/Svelte equivalents follow the same shape: controller returns `Inertia::render('Page', [...])` with props, the page component reads props and uses `useForm` for submission state (`processing`, `errors`, `progress`).

## Output Escaping By Context (XSS)

Escaping is context-sensitive:

- **Blade HTML body/attribute:** `{{ $v }}` auto-escapes via `htmlspecialchars($v, ENT_QUOTES, 'UTF-8')`; never use `{!! $v !!}` on untrusted data — it bypasses escaping entirely.
- **Vue/React/Svelte templates:** standard interpolation (`{{ }}` in Vue, `{}` in JSX) escapes by default; avoid `v-html`, `dangerouslySetInnerHTML`, or `{@html}` on untrusted data.
- **URL parameter:** validate the scheme (allow only `http`/`https`/`mailto`) before rendering a user-supplied URL as an `href`, in any of the three stacks.
- **Inside `<script>` / JS context:** when passing PHP data into inline JS, use `Js::from($v)` or `json_encode($v, JSON_HEX_TAG | JSON_HEX_APOS | JSON_HEX_QUOT | JSON_HEX_AMP)`; Inertia already serializes props safely, so avoid re-embedding raw untrusted strings into extra inline `<script>` tags.
- **CSS context:** avoid injecting user data into styles; if unavoidable, allowlist strictly.

Consider a Content-Security-Policy header as defense-in-depth, and avoid inline event handlers/`style` so a stricter CSP is possible (Alpine's `x-on` and Livewire's `wire:` directives are compiled by their runtimes, not inline `on*` attributes).

## Asset And Progressive-Enhancement Hygiene

- Compile assets with Vite: `npm run dev` for local development with HMR, `npm run build` for production; reference entrypoints in Blade with `@vite(['resources/css/app.css', 'resources/js/app.js'])`.
- Keep Alpine.js behavior declarative and scoped (`x-data`, `x-show`, `x-on:click`) rather than manual `document.querySelector` wiring; feature-detect and degrade gracefully for Blade-only pages.
- Let Vite handle cache-busting and versioning of built assets; don't hand-version files under `public/build`.
- Do not block first paint on non-critical scripts; Vite's module preloading and Inertia's code-split pages already help, but avoid adding large synchronous third-party scripts to the layout.
- Keep templates/components logic-light: format and escape in the view/component, compute in the controller, Livewire component method, or a dedicated Action/Service class.

## Verification

Possible checks:

```bash
composer test
php artisan test
npm run build
npm run lint
php artisan pint --test
```

Use only commands present in the project. Also verify markup against `wcag-accessibility` rules.

## Final Output

Return what changed, rendering approach used (Blade/Livewire/Inertia), tests/checks run, accessibility notes, Context Summary, and next step.
