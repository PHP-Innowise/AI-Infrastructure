---
name: plugin-development
description: Design and implement production WordPress plugins. Use for plugin bootstrap, lifecycle hooks, service wiring, settings, packaging, upgrades, uninstall, compatibility, and public extension contracts.
phase: execution
flow-next: code-reviewer
flow-alternatives: [test-generator, security-reviewer, verify]
related: [architect, coder, hooks-events, database-designer]
---

# Plugin Development

## Purpose

Build or change a plugin without leaking work into file-load time, colliding
with other extensions, losing stored data, or breaking public contracts.

## Procedure

1. Read the main plugin file, headers, bootstrap/composition root, Composer
   metadata, existing hooks, activation/deactivation/uninstall behavior, tests,
   packaging configuration, and minimum WordPress/PHP requirements.
2. Inventory public compatibility surfaces: hooks, filters, PHP functions and
   classes, REST routes, shortcodes, blocks, settings, option/meta formats,
   cron hook names, script handles, and database schema versions.
3. Choose the smallest integration boundary. The bootstrap registers services
   and hooks; hook callbacks adapt WordPress values; cohesive services own
   business behavior. Do not introduce a container or interface per class
   without a real substitution boundary.
4. Separate normal boot, admin boot, REST registration, CLI registration,
   activation, deactivation, migration, and uninstall. Activation must be
   repeatable; deactivation must not delete user data; uninstall deletes data
   only when the documented product policy explicitly requires it.
5. Prefix or namespace every global identifier. Load translations on the
   supported lifecycle for the project's minimum WordPress version.
6. Gate admin behavior by screen and capability. Register settings through the
   Settings API with sanitize callbacks. Keep frontend/editor assets scoped.
7. Version data changes independently from the plugin version. Make upgrades
   idempotent, resumable where data is large, and safe under concurrent requests.
8. Preserve backward compatibility or add a deprecation/migration path. Never
   silently rename option keys, hook names, REST fields, or block attributes.
9. Add focused unit/integration tests and run packaging checks to prove that
   source-only files, secrets, development dependencies, and build caches do
   not enter the release artifact.

## Review Risks

- request work at plugin include time;
- activation on multisite without explicit network/site semantics;
- `flush_rewrite_rules()` on ordinary requests;
- autoloaded options containing large or growing data;
- uninstall that deletes shared or user-owned content unexpectedly;
- bundled dependency/class collisions;
- update routines that assume one request completes the migration;
- plugin header or readme version drift.

## Output

Return changed lifecycle boundaries, public compatibility impact, stored-data
impact, security decisions, tests/build evidence, Context Summary, and the
next appropriate review or verification skill.
