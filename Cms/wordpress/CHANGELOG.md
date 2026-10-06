# WordPress Accelerator Changelog

All notable changes to the WordPress edition are recorded here. Shared
context-runtime changes are also recorded in the repository-root changelog.

## Unreleased

### Fixed

- Hooks are wired through the project root: `"${CLAUDE_PROJECT_DIR}"/.claude/hooks/<script>.sh` in `.claude/settings.json`, and a root-finding `sh` launcher in `.codex/hooks.json`. The safety hooks now keep running when the session's directory is a subdirectory; before, they exited 127 and failed open. Policy lock regenerated. Codex users must re-trust the changed hooks once in `/hooks`.
- `.cursor/hooks/subagent-dispatch.sh` and `.codex/hooks/subagent-dispatch.sh` are executable again (100644 -> 100755). Cursor runs the hook directly on `subagentStop`; without the bit it exited 126, and the write-agent lock taken by `subagent-gate` stayed held until its 30-minute TTL. To repair an existing install, run `chmod +x .cursor/hooks/*.sh .codex/hooks/*.sh`. Reinstalling also repairs it. A hook whose content already matches gets only the bit added and is reported as `FIX_MODE` (`WOULD_FIX_MODE` with `--dry-run`). An install from an earlier release first collides on the files this release changed (`.claude/settings.json`, `.codex/hooks.json`, `bash-validator.sh`, the hook READMEs, the policy lock). `--merge-existing` then refuses and changes nothing, modes included, so use the `chmod` above or rerun with `--overwrite` after reviewing `--overwrite --dry-run`.
- bash-validator: added `wp db clean` and `wp site empty`; WP-CLI rules accept global flags before the subcommand, the `wp-cli.phar` form and launchers (`ddev wp`, `docker compose run ... wp`). New secret rules block `wp config get` of `DB_PASSWORD` or the keys and salts, and an unfiltered `wp config list`. `wp-config-sample.php` is now readable. The generic rules are documented in the root CHANGELOG.
- The Codex `SKILL FLOW.md` (`.agents/skills/`) named the Claude/Cursor commands `/brainstorm`, `/git-worktrees`, `/debugger` and `/docs-generator`, which do not exist for Codex (Codex has no command layer). They now name the skills `brainstorming`, `using-git-worktrees`, `systematic-debugger` and `documentation-generator`. Found by the new `scripts/check_routes.py` gate.
- bash-validator: reading `wp-config.php` is also blocked through `rg`, `ag`, `ack`, `egrep`, `fgrep` and `cut`. A search whose pattern is the file name (`grep -rn wp-config.php .`) passes. The leftover `schema:drop` console rule was removed, because its abbreviation matching blocked `schema:d` (`schema:dump`). Sail `art`/`a` count as artisan for the schema-reset rules.
- The Codex hook launcher in `.codex/hooks.json` now stops at the project that declared the hook (the nearest directory holding `.codex/hooks.json`). A missing hook script exits 127 and names the path it expected. Before, the launcher could run a same-named script from an ancestor directory such as `~/.codex/hooks/`. Codex users must re-trust the changed hooks once in `/hooks`. Policy lock regenerated.
- `.codex/hooks/README.md` and `.cursor/hooks/README.md` no longer send readers to `.claude/hooks/README.md`, which Codex-only and Cursor-only installs do not get. They now point at `BV_FRAMEWORK_RULES` in the `bash-validator.sh` next to them.

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
