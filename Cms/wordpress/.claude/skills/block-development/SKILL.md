---
name: block-development
description: Build Gutenberg blocks and editor extensions using block metadata, static or dynamic rendering, attributes, InnerBlocks, data stores, interactivity, deprecations, transforms, and WordPress build tooling.
phase: execution
flow-next: browser-verify
flow-alternatives: [test-generator, code-reviewer, verify]
related: [coder-frontend, theme-development, rest-api, wcag-accessibility]
---

# Block Development

## Procedure

1. Read `block.json`, registration PHP, editor source, saved markup or render
   callback, package scripts, generated asset metadata, tests, and minimum
   WordPress/Gutenberg versions.
2. Decide static versus dynamic rendering from content ownership and change
   frequency. Static blocks save stable content markup; dynamic blocks render
   server state without forcing content migrations for presentation changes.
3. Define an explicit namespaced block name, API version supported by the
   project, attributes with stable types/defaults, supports, context, styles,
   scripts, and render contract in metadata. Register from metadata.
4. Keep editor state serializable. Do not store derived presentation data in
   attributes. Use `InnerBlocks` and block context where they represent real
   composition; avoid private data-store coupling.
5. Treat saved markup and attributes as a public persisted schema. When static
   markup changes incompatibly, add tested deprecations and migrations rather
   than triggering invalid-block recovery for existing content.
6. For dynamic blocks, validate attributes and context server-side, query with
   bounded access patterns, and escape the rendered output. Cache only with an
   explicit invalidation strategy.
7. Use WordPress packages and declared dependencies. Do not enqueue editor
   bundles on the frontend or duplicate React. Keep generated build artifacts
   consistent with repository policy.
8. Test edit/save/render behavior, serialization, deprecated fixtures,
   transforms, permissions, empty/error/loading states, keyboard behavior,
   responsive layout, and editor/frontend parity.

## Output

Return metadata and rendering decisions, persistence compatibility, asset
scope, tests/build/browser evidence, Context Summary, and next step.
