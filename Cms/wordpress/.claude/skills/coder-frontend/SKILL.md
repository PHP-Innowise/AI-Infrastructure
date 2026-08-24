---
name: coder-frontend
description: Implement WordPress frontend and editor UI in PHP templates, block themes, Gutenberg blocks, admin screens, or approved JavaScript stacks with correct enqueueing, escaping, accessibility, and editor/frontend parity.
phase: execution
flow-next: browser-verify
flow-alternatives: [code-reviewer, test-generator, verify]
related: [frontend-design, theme-development, block-development]
---

# WordPress Frontend Coder

## Procedure

1. Identify classic template, block theme, block editor, admin, widget,
   shortcode, or decoupled frontend surface and its supported browsers/build.
2. Follow the existing component and styling system. Prefer WordPress block,
   template, pattern, Interactivity API, data, i18n and component packages when
   they match the supported version.
3. Enqueue only where needed with registered handles, dependencies, versions,
   module/strategy choices and generated asset metadata. Never bundle another
   WordPress React runtime or expose secrets through localized script data.
4. Validate server inputs and escape at output. For JavaScript, render untrusted
   content as text or sanitize through an approved policy; avoid raw HTML sinks.
5. Preserve semantic HTML, keyboard operation, focus behavior, labels, status
   announcements, contrast, zoom/reflow, reduced motion, RTL and translation.
6. Keep editor controls and saved/frontend output aligned. Treat block
   attributes and saved markup as persisted compatibility contracts.
7. Test empty/loading/error/permission states, representative long content,
   mobile/desktop, logged-in/out, editor/frontend and relevant admin screens.

## Output

Return UI/assets changed, enqueue and data contract, accessibility/i18n notes,
build/tests/browser evidence, Context Summary, and next step.
