---
name: test-generator
description: Generate WordPress PHP and JavaScript tests for plugins, themes, hooks, REST routes, blocks, WP-CLI, multisite, cron, data migrations, and WooCommerce using the project's configured test stack.
phase: quality
flow-next: verify
flow-alternatives: [coder, systematic-debugger, code-reviewer]
related: [security-reviewer, browser-verify, block-development]
---

# WordPress Test Generator

## Procedure

1. Read project test bootstrap, factories/fixtures, Composer/npm scripts, CI,
   base classes, WordPress test-library version and existing conventions.
2. Test public behavior through its real boundary: execute hooks, dispatch the
   REST server, render/serialize blocks, run commands, trigger scheduled
   callbacks, switch sites, or use WooCommerce factories/CRUD objects.
3. Create isolated data with factories and clean global hooks, current user,
   locale, site, options, cache and uploads. Tests must not rely on order or
   mutate shared fixtures.
4. Cover success plus the highest-risk failure: unauthorized/invalid input,
   nonce and capability separation, escaped output, SQL injection boundary,
   duplicate retry, migration rerun, concurrency/locking, cache invalidation,
   multisite isolation or persisted block compatibility as applicable.
5. Mock only external systems or expensive narrow boundaries. Do not mock the
   WordPress API whose integration the test is meant to prove.
6. For blocks/JavaScript, use configured Jest/testing-library/e2e tools and
   include saved-markup/deprecation fixtures when serialization can drift.
7. Run the focused test while developing, then the full relevant suite; report
   filtered runs as filtered, not complete evidence.

## Output

Return tests added, boundary and risks covered, fixtures/cleanup, commands and
results, remaining gaps, Context Summary, and next step.
