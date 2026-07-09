---
name: frontend-design
description: Design user interfaces for native PHP projects using server-rendered templates, semantic HTML, CSS, and progressive enhancement. Framework-neutral (no assumed JS framework).
phase: planning
flow-next: writing-plans
flow-alternatives: [coder-frontend, api-designer]
related: [api-designer, coder-frontend, wcag-accessibility, web-design-guidelines]
---

# Frontend Design

## Overview

Design frontend experiences for native PHP applications. This base assumes server-rendered PHP templates with semantic HTML, CSS, and progressive enhancement. A specific JS framework (React, Vue, etc.) is out of scope on this branch; it belongs on a framework-specific branch.

Do not assume a rendering approach until the project reveals it.

## Rendering Decision

| Need | Good fit |
| --- | --- |
| Content pages, simple forms | Server-rendered PHP templates + plain HTML |
| Small dynamic touches (toggles, validation hints) | Server-rendered templates + progressive enhancement with vanilla JS |
| Highly interactive UI | API-only PHP backend + the team's chosen frontend (out of scope here) |
| Admin CRUD | Server-rendered templates or a project-standard admin approach |

Default to the least JavaScript that meets the requirement. Pages must work without JS first, then enhance.

## Required Context

Read:

- Existing templates/views directory and layout/partials.
- Existing CSS and any design tokens.
- API design when the UI consumes API endpoints.
- Accessibility rules in `wcag-accessibility`.

## Design Output

Include:

- User flow.
- Page/partial list.
- Form fields, validation feedback, loading states, empty states, and error states.
- Accessibility notes (semantic structure, labels, focus, keyboard).
- Responsive behavior.
- Data dependencies and endpoints.
- Recommended rendering approach for this feature.

## Native PHP UI Notes

- Server-rendered forms must include a CSRF token for state-changing requests.
- Authorization stays server-side; hiding a control is not authorization.
- Escape all dynamic output in templates (`htmlspecialchars`) to prevent XSS.
- Re-render forms with prior input and per-field error messages on validation failure.
- Progressive enhancement: the baseline HTML flow must work if JavaScript fails to load.

## Frontend Best Practices

- **Semantic HTML first:** use the right element (`button`, `a`, `nav`, `main`, `form`, `table`, `dialog`) before reaching for `div` + ARIA. Correct semantics give you accessibility and keyboard support for free.
- **Progressive enhancement:** the core flow works with HTML alone; CSS improves presentation; JS adds convenience. Never make required functionality JS-only.
- **Responsive and fluid:** mobile-first CSS, relative units (`rem`, `%`, `clamp()`), flexbox/grid, and `max-width` containers. Test at small and large widths.
- **Performance:** minimize render-blocking assets, defer non-critical JS, lazy-load below-the-fold images (`loading="lazy"`), set width/height to avoid layout shift, and compress/serve modern image formats.
- **Forms UX:** label every field, show inline per-field errors, preserve entered values on failure, use correct `type`/`inputmode`/`autocomplete`, and disable the submit button only after starting submission (never as the sole guard).
- **CSS architecture:** keep it consistent (utility classes, BEM, or design tokens); avoid deep specificity wars and `!important`. Define spacing/color/type scales as tokens.
- **State feedback:** always design loading, empty, error, and success states, not just the happy path.
- **Accessibility as a requirement:** color contrast, visible focus, keyboard operability, and reduced-motion support are acceptance criteria, not extras. See `wcag-accessibility`.

## Final Output

Return the design, rendering recommendation, implementation notes, Context Summary, and next step.
