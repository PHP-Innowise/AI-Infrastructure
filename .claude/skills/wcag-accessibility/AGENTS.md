# WCAG Accessibility Guidelines

Practical accessibility guidance for Laravel, Blade, Livewire, Inertia, and general HTML interfaces. These rules are based on WCAG 2.2 Level AA and should be used during frontend design, implementation, and review.

## How To Use

- Start with semantic HTML before adding ARIA.
- Prefer native controls: `button`, `a`, `label`, `input`, `select`, `textarea`, `table`, `dialog`, and landmark elements.
- In Blade and Livewire, ensure the rendered HTML is accessible, not just the template code.
- In Inertia or separate JavaScript frontends, apply the same rendered-HTML rules.
- Validate forms with Laravel on the server and expose errors accessibly in the UI.

## Rule Map

### Semantic
- `rules/semantic-button-link.md`
- `rules/semantic-heading-hierarchy.md`
- `rules/semantic-landmark-regions.md`
- `rules/semantic-list-structure.md`
- `rules/semantic-table-markup.md`

### Keyboard
- `rules/keyboard-focus-trap.md`
- `rules/keyboard-focus-visible.md`
- `rules/keyboard-interactive-elements.md`
- `rules/keyboard-skip-link.md`
- `rules/keyboard-tab-order.md`

### Forms
- `rules/forms-autocomplete.md`
- `rules/forms-error-messages.md`
- `rules/forms-fieldset-legend.md`
- `rules/forms-labels-required.md`

### ARIA
- `rules/aria-expanded-states.md`
- `rules/aria-hidden-misuse.md`
- `rules/aria-labels-required.md`
- `rules/aria-live-regions.md`
- `rules/aria-prefer-semantic.md`

### Color, Media, Motion, Responsive
- `rules/color-contrast-text.md`
- `rules/color-contrast-ui.md`
- `rules/color-not-only-indicator.md`
- `rules/media-alt-text.md`
- `rules/media-decorative-images.md`
- `rules/media-video-captions.md`
- `rules/motion-no-autoplay.md`
- `rules/motion-reduced-motion.md`
- `rules/motion-safe-transitions.md`
- `rules/responsive-reflow.md`
- `rules/responsive-text-resize.md`
- `rules/responsive-touch-target.md`

## Laravel Review Checklist

- Labels are associated with inputs using `for` and `id`, or wrapping labels.
- Laravel validation errors are visible and connected with `aria-describedby`.
- Buttons and links use semantic elements.
- Livewire/Inertia dynamic updates announce important changes with live regions.
- Modals trap focus and restore focus when closed.
- Color is not the only way information is communicated.
- Motion respects `prefers-reduced-motion`.
- Touch targets are large enough on mobile.
