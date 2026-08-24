# WordPress Golden Principles

These principles guide implementation, review and debugging. Canonical project
sources and supported versions take precedence.

## 1. WordPress Lifecycle Is Architecture

Make plugin/theme load, hooks, activation, admin, REST, CLI, cron, rendering
and uninstall explicit. File inclusion is not a request handler.

## 2. Native APIs Before Reinvention

Use WordPress APIs for hooks, capabilities, data, HTTP, filesystem, mail,
scheduling, cache, settings, blocks and REST. Add abstractions only where they
improve cohesion, testing, substitution or dependency direction.

## 3. Stable Names and Contracts

Prefix or namespace global identifiers. Preserve public hooks, filters, REST
schemas, blocks/attributes, shortcodes, stored formats and documented PHP APIs.
Deprecate and migrate deliberately.

## 4. Security Controls Stay Separate

Sanitize and validate input; escape late for context. Authentication/nonces do
not replace capability and object authorization. REST routes declare a
permission callback. SQL values are prepared and identifiers allowlisted.

## 5. Choose Storage From Access Patterns

Content, taxonomy, metadata, options, transients and custom tables have
different ownership and scaling properties. Avoid large autoloaded options,
unbounded serialization and relational workloads forced through post meta.

## 6. Retriable Operations Are Idempotent

Activation, migrations, webhooks, cron, scheduled actions, imports and CLI
batches may repeat or overlap. Track versions/identity, bound work, define
locking/retry/recovery, and never depend on one request completing everything.

## 7. Site, Network, User and Locale Scope Are Explicit

Cache keys, options, tables, capabilities and background work must match their
owner. Restore every multisite switch and never leak site-scoped objects.

## 8. Editor, Admin and Frontend Are Distinct Surfaces

Scope assets and data. Keep block editor and frontend behavior compatible,
preserve serialized content, use WordPress packages/dependencies, and avoid
loading admin/editor bundles everywhere.

## 9. Accessibility, Internationalization and Privacy Are Functional

Keyboard operation, semantics, focus, contrast, translation, RTL, personal
data export/erase and safe logging are acceptance criteria, not polish.

## 10. Measure and Verify

Profile before optimizing. Test through real WordPress boundaries. Run the
project's standards, static analysis, PHP/JavaScript tests, builds and
integration checks. Report failures and unavailable tooling honestly.
