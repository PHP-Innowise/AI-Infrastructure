---
name: theme-development
description: Build and review WordPress classic or block themes, including template hierarchy, theme.json, templates, parts, patterns, assets, customization, accessibility, internationalization, and child-theme compatibility.
phase: execution
flow-next: browser-verify
flow-alternatives: [code-reviewer, test-generator, verify]
related: [frontend-design, coder-frontend, block-development, wcag-accessibility]
---

# Theme Development

## Procedure

1. Determine classic, block, hybrid, parent, or child theme from `style.css`,
   `functions.php`, `theme.json`, `templates/`, `parts/`, and build files.
2. Read existing template hierarchy, supports, menus, image sizes, patterns,
   asset registration, translation domain, and customization contracts before
   choosing files.
3. For block themes, target the project's supported `theme.json` schema and
   prefer global settings/styles, templates, parts, patterns, and block style
   variations over brittle selector overrides.
4. For classic themes, preserve template hierarchy and child-theme override
   behavior. Use template tags and query APIs; never mutate the main query
   globally when a scoped query or documented hook is appropriate.
5. Enqueue scripts and styles on the correct hook with explicit dependencies,
   versions, footer/strategy choices, and screen or template scope. Do not
   hard-code plugin-owned functionality into the theme.
6. Escape at output for HTML, attributes, URLs, JavaScript, and text areas.
   Preserve deliberately allowed markup through a narrow `wp_kses()` policy.
7. Make every control keyboard-usable, visible at focus, semantically correct,
   translatable, responsive, and robust at text zoom and reduced motion.
8. Test editor and frontend parity, representative content extremes, template
   fallback, logged-in/out states, RTL where supported, and child-theme impact.

## Boundaries

- Themes own presentation. Portable post types, business rules, scheduled
  work, durable integrations, and site-critical behavior belong in a plugin.
- Avoid copying WordPress core block markup that will drift when a supported
  template, pattern, variation, or style configuration can express the need.
- Preserve user customizations and content; do not overwrite them as a build
  or activation side effect.

## Output

Return theme shape, templates/assets changed, editor/frontend and child-theme
impact, accessibility/i18n checks, visual evidence, Context Summary, and next
step.
