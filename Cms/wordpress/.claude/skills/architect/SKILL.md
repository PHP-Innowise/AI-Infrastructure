---
name: architect
description: Make WordPress architecture decisions for plugins, themes, blocks, custom sites, multisite, REST, data, integrations, background work, and WooCommerce while respecting lifecycle and backward compatibility.
phase: planning
flow-next: writing-plans
flow-alternatives: [database-designer, api-designer, architecture-implementer]
related: [content-modeling, plugin-development, theme-development]
---

# WordPress Architect

## Procedure

1. Establish repository shape, owned deployment unit, supported WordPress/PHP/
   Gutenberg/WooCommerce versions, hosting constraints, and current extension
   points from canonical files.
2. Map lifecycle entry points: plugin/theme load, `init`, admin, REST, AJAX,
   CLI, cron/background, activation/deactivation/uninstall, block editor and
   frontend rendering.
3. Identify public contracts and persisted schemas that constrain change:
   hooks, PHP APIs, REST routes, blocks/attributes, shortcodes, options/meta,
   database tables, scheduled actions and external messages.
4. Keep adapters at WordPress boundaries thin. Put cohesive business behavior
   in testable services, but do not hide lifecycle, capabilities, transactions,
   caching, or extension hooks behind unnecessary abstraction.
5. Choose WordPress-native storage and APIs first. Justify custom tables,
   external services, queues, containers, or framework components with access
   patterns and operational evidence.
6. Decide capability/object authorization, nonce/authentication, sanitization,
   escaping, privacy/export/erase, multisite scope, caching/invalidation,
   background reliability, and failure/rollback before implementation.
7. Design compatibility and migration for stored data and public contracts.
   Separate deploy, activation, backfill, read switch, and cleanup when a
   rolling migration cannot safely complete in one request.
8. Record alternatives, trade-offs, assumptions, evidence, risks, test seams,
   and the smallest implementation slices.

## Reject

- logic at file include time that depends on current request state;
- one giant plugin class or `functions.php` as an application layer;
- one interface per implementation without a substitution boundary;
- direct core-file edits, bundled WordPress copies, or monkey patches;
- role-name authorization, nonce-only permission checks, global asset loading;
- meta/options as an unbounded relational database;
- synchronous all-site or all-content loops on web requests.

## Output

Return context/evidence, boundaries and dependency direction, lifecycle map,
data and public contracts, security/operations/compatibility decisions,
alternatives, implementation slices, verification plan, Context Summary, and
next step.
