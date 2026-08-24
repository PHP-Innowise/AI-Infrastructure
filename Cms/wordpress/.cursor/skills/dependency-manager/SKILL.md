---
name: dependency-manager
description: Manage WordPress PHP, JavaScript, WordPress.org, Composer, npm, and bundled dependencies with compatibility, licensing, security, build, prefixing, and release-package review.
phase: quality
flow-next: verify
flow-alternatives: [researcher, coder, systematic-debugger]
related: [plugin-development, block-development, security-reviewer]
---

# WordPress Dependency Manager

## Procedure

1. Inventory `composer.json/lock`, `package.json` and lockfile, WordPress plugin/
   theme dependencies, build tooling, bundled vendor code, supported runtime
   versions, CI and release packaging.
2. For a new dependency, verify maintained source, license, advisory history,
   install/runtime footprint, PHP/WordPress/Node compatibility, API stability,
   transitive graph and whether a WordPress/core API already solves the need.
3. Distinguish project dependencies from distributable plugin/theme
   dependencies. Public extensions must avoid global Composer class collisions
   through an established prefixing/isolation strategy when necessary; do not
   invent one without checking the release pipeline.
4. Preserve lockfile/package-manager convention and use the narrowest supported
   update. Never mix npm/yarn/pnpm lockfiles.
5. Review build artifacts and WordPress script dependencies so React and core
   packages are externalized correctly. Ensure dev dependencies, tests,
   sources, credentials and caches do not enter release packages.
6. Run configured audit, tests, standards, static analysis, JavaScript lint/
   tests/build and package smoke checks. Document unavoidable advisories and
   upgrade blockers; do not install or publish without authorization.

## Output

Return dependency decision/change, compatibility/license/security analysis,
lock/build/package impact, commands and results, risks, Context Summary, and
next step.
