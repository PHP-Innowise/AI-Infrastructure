# WordPress Accelerator Changelog

All notable changes to the WordPress edition are recorded here. Shared
context-runtime changes are also recorded in the repository-root changelog.

## Unreleased

- Clarify the `writing-plans` feature-planning intent so it remains in the
  first two procedural results after adding context handoff skills; preserve
  the existing measured routing floor and retrieval boundaries.

- Merge multiple captured chats into a new Codex, Claude Code, or Cursor task: frozen source archives, source-attributed previews, explicit conflict handling, same-branch automatic selection, and optional `context-load merge` source selection. Reopening preserves the merge; native task creation remains client-capability dependent.

- Add trusted automatic same-branch context continuity for Codex, Claude Code and Cursor. Hooks store bounded documented visible text only in ignored `.context-handoff/`, restore historical excerpts without Brain/Bank/SQLite writes, and support an explicit process-local opt-out. Client lifecycle delivery remains capability-dependent; synthetic tests do not prove a live account.

- Add `context-save` and `context-load` for Codex, Claude Code and Cursor:
  save a summary or selected topic, preserve an explicitly exported conversation
  verbatim in full mode, and check source drift before the next task.
  Shared runtime changes are recorded in the root changelog.

### Changed

- Memory promotion policy now matches runtime configuration: eligible verified
  terminal records may be applied unattended only when automatic promotion is
  enabled and remain explicitly marked as unreviewed; otherwise an independent
  human review is required.

## 2.0.0 - 2026-08-24

### Added

- First ready-made WordPress edition under `Cms/wordpress` for Claude Code,
  Cursor and Codex.
- WordPress policy covering lifecycle, namespacing, hooks, capabilities,
  nonces, sanitization, contextual escaping, REST permissions, `$wpdb`, data
  migrations, caching, cron, multisite, accessibility, i18n and compatibility.
- Dedicated plugin, theme, Gutenberg block, hooks/events, REST implementation,
  content modeling, WP-CLI, multisite, WooCommerce, and cron/background skills.
- WordPress-specific architecture, coding, frontend, database, testing,
  security, performance, dependency, review and verification workflows.
- Inventory-driven installation, generated tool mirrors, shared Project Brain,
  local context runtime and durable Memory Bank support.
