# WordPress AI Accelerator

> Enforceable agent policy lives in [AGENTS.md](AGENTS.md).

A ready-to-use workflow layer for AI-assisted WordPress engineering. It is
designed for plugins, classic and block themes, custom sites, Bedrock-style
projects, multisite code, Gutenberg blocks, WP-CLI tooling, REST APIs, and
WooCommerce extensions. It does not install WordPress or replace project code,
configuration, tests, product requirements, or deployment procedures.

## What It Provides

- WordPress-specific architecture, implementation, review, testing, security,
  performance, data, REST, block-editor, plugin, theme, CLI, multisite, and
  WooCommerce workflows.
- Shared policy and safety hooks for Claude Code, Cursor, and Codex.
- Temporary task documents under `tasks/` and durable living specs under
  `specs/`.
- Governed active-work coordination in `project-brain/`.
- Durable, reviewed project knowledge in `memory-bank/`, with a disposable
  local retrieval index.

This is an accelerator, not a starter plugin or theme. It follows the
consuming repository's established layout and supported versions instead of
forcing a boilerplate architecture.

## Supported Project Shapes

Before changing code, the agent identifies the actual repository shape:

| Shape | Typical evidence | Important boundary |
| --- | --- | --- |
| Plugin | plugin header, bootstrap file, `composer.json` | activation, hooks, uninstall and backward compatibility |
| Classic theme | `style.css`, `functions.php`, PHP templates | template hierarchy, enqueueing and escaping |
| Block theme | `theme.json`, `templates/`, `parts/`, patterns | schema version, block markup and editor/frontend parity |
| Block package | `block.json`, `src/`, `build/`, `@wordpress/scripts` | metadata, attributes, serialization and migrations |
| Full site/project | `wp-config.php`, `wp-content/`, Bedrock files | environment ownership and third-party boundaries |
| Multisite component | network hooks/config or site switching | site versus network scope and cache/data isolation |
| WooCommerce extension | WooCommerce dependency and integration classes | CRUD APIs, HPOS compatibility and order lifecycle |

The project's declared WordPress, Gutenberg, WooCommerce, PHP, Node, and
database versions always outrank the edition's examples.

## Multi-Tool Layout

| Tool | Reads | Notes |
| --- | --- | --- |
| Claude Code | `.claude/` | Commands, thin agent wrappers, hooks, governance and generated skill mirrors |
| Cursor | `.cursor/` | Self-contained commands, agents, rules, hooks and skill mirrors |
| Codex | `.agents/skills/` and `.codex/` | Canonical skills plus Codex configuration and hooks; no command-wrapper layer |

The canonical shared skills live in `.agents/skills/`. Claude/Cursor skills,
Cursor wrappers, and cross-tool governance files are generated according to
the repository mirror contract. Do not hand-edit generated mirrors.

## Workflow Model

```text
User request
    -> command selects a focused agent (Claude/Cursor)
    -> agent executes exactly one skill
    -> result, verification evidence, Context Summary, Next Steps
```

Codex discovers and invokes the same skills directly. Full flows such as
`flow-feature`, `flow-review`, and `sdd` remain orchestrator-controlled and
pause at their documented user checkpoints.

## WordPress Capability Set

### Core engineering

- `architect` — choose lifecycle boundaries and project structure.
- `coder` — implement backend behavior with WordPress APIs.
- `plugin-development` — plugin bootstrap, lifecycle, services, packaging,
  uninstall and compatibility.
- `theme-development` — classic/block theme templates, assets, `theme.json`,
  patterns and customization boundaries.
- `block-development` — static/dynamic blocks, metadata, attributes,
  deprecations, editor/frontend behavior and builds.
- `hooks-events` — actions, filters, priorities, callback contracts and event
  side effects.

### Interfaces and data

- `rest-api` — `register_rest_route`, schemas, permissions, controllers and
  stable response contracts.
- `content-modeling` — post types, taxonomies, metadata, registration and
  rewrite behavior.
- `database-designer` — options/meta versus custom tables, `$wpdb`, schema
  versions and safe migrations.
- `wp-cli` — idempotent commands, progress, exit codes and destructive guards.

### Platform specialties

- `multisite` — network/site scope, switching, provisioning and cache safety.
- `woocommerce` — extension architecture, CRUD objects, hooks, HPOS,
  checkout/order compatibility and payment boundaries.

The edition also includes requirements, research, planning, codebase mapping,
frontend design, accessibility, testing, debugging, review, security,
performance, dependencies, documentation, release, verification, context, and
memory workflows shared with the other accelerators but expressed in
WordPress terms where behavior differs.

## Senior WordPress Engineering Defaults

- Use the WordPress lifecycle and public APIs before direct infrastructure
  access.
- Namespace or prefix all globally visible identifiers.
- Keep callbacks thin and move multi-step behavior into cohesive, testable
  services without hiding WordPress integration boundaries.
- Treat nonces and capabilities as separate controls.
- Sanitize and validate input; escape late for the exact output context.
- Every REST route has an intentional `permission_callback` and schema.
- Use `$wpdb->prepare()` for SQL values; allowlist identifiers.
- Choose posts/meta/options/custom tables from access patterns and lifecycle,
  not convenience. Avoid large autoloaded options and unbounded serialized
  collections.
- Make migrations versioned, retryable, multisite-aware, and safe for rolling
  deploys. Do not flush rewrite rules on ordinary requests.
- Make WP-Cron work idempotent and reserve it for delay-tolerant jobs.
- Preserve public hooks, REST fields, block attributes, stored option formats,
  shortcodes, and documented PHP APIs through explicit deprecations and data
  migrations.
- Keep editor, admin, and frontend assets scoped to the screens that need them.
- Meet WordPress accessibility and internationalization conventions.

## Expected Tooling

Use only tooling already configured by the consuming project. Common choices:

```bash
composer validate --strict
composer test
vendor/bin/phpunit
vendor/bin/phpcs
vendor/bin/phpstan analyse
npm test
npm run lint:js
npm run lint:css
npm run build
npx wp-env start
wp core version
wp plugin status
wp theme status
```

Projects may instead use wp-browser/Codeception, Pest, Playwright, VIP
tooling, DDEV, Lando, Docker Compose, or custom CI scripts. Prefer repository
Composer and npm scripts. Missing tools are reported as `N/A`; agents do not
install them without approval.

## Quick Start

For a non-trivial task:

1. Identify project shape and supported versions from canonical files.
2. Use `codebase-mapper` for an unfamiliar repository.
3. Use `requirements-analyst`, `brainstorming`, or `architect` when
   requirements or boundaries are unclear.
4. Use the narrowest implementation skill: `plugin-development`,
   `theme-development`, `block-development`, `rest-api`, `coder`,
   or another specialty.
5. Run review, security, tests, and `verify` in proportion to risk.

Example requests:

```text
Use plugin-development to add an activation-safe schema upgrade.
Use block-development to create a dynamic event-list block.
Use rest-api to implement a capability-protected endpoint.
Use woocommerce to review this extension for HPOS compatibility.
Use verify to run the WordPress Definition of Done.
```

## Documentation and Context

- `tasks/TASK-NNN/` contains temporary, skill-prefixed work artifacts.
- `specs/` contains durable specifications registered in `specs/MANIFEST.md`.
- `project-brain/` is authoritative for shared active tasks and handoffs.
- `memory-bank/` contains durable reusable knowledge, not active task state.
- `memory-bank/local/context.db` is ignored and disposable.

Canonical code, configuration, tests, migrations, block metadata, schemas,
and specifications outrank all retrieved context.

### Task Capsule and Refresh

Use `memory` in Codex or `/memory` in Claude/Cursor for the authority-aware
context refresh. Complex phase handoffs use a Task Capsule capped at 8,000
Unicode characters with at most two Procedural, three Semantic, and one
Episodic result plus bounded Working state. It carries cited discovery hints,
not the parent conversation; agents verify the cited canonical sources before
making decisions. `checkpoint` records sanitized progress according to the
configured governed/lightweight mode, while explicit `complete` remains the
only completion transition.

## Installation

Use the repository-root inventory-driven installer. Start with `--dry-run` and
resolve every remaining collision manually:

```bash
python3 scripts/install_accelerator.py \
  --edition WordPress \
  --target "/absolute/path/to/wordpress-project" \
  --tool codex \
  --merge-existing \
  --dry-run
```

The edition source lives at `Cms/wordpress`, while its public installer name
is `WordPress`. Source-only tests and examples are not installed.

## Verification

Use the active tool's `DOD.md`. Implementation work normally requires
applicable PHP and JavaScript tests, WordPress Coding Standards, static
analysis, build/block validation, integration or browser checks, and a review
of capability, nonce, sanitization, escaping, SQL, migration, caching and
compatibility risk.

## Directory Structure

```text
Cms/wordpress/
├── AGENTS.md
├── VERSION
├── README.md
├── CHANGELOG.md
├── .agents/skills/        # canonical skills
├── .claude/               # Claude wrappers, hooks and generated skills
├── .cursor/               # Cursor integration
├── .codex/                # Codex integration
├── memory-bank/           # durable knowledge + local context runtime
├── project-brain/         # governed active-work control plane
├── specs/                 # living specifications
├── tasks/                 # temporary work scaffold
└── examples/              # source-only worked examples
```
