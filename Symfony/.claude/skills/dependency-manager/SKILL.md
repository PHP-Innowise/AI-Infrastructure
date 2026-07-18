---
name: dependency-manager
description: "Manage Symfony Composer dependencies: audit vulnerabilities, review outdated packages, evaluate bundles/components, inspect Flex recipe impact, and update safely."
phase: execution
flow-next: verify
flow-alternatives: [security-reviewer, researcher, code-reviewer]
---

# Symfony Dependency Manager

## Rules

- Prefer Symfony components and maintained bundles with clear compatibility.
- Check Symfony version constraints before adding/updating packages.
- Inspect Symfony Flex recipes before accepting generated config changes.
- Run `composer audit` and triage advisories.
- Avoid adding packages for trivial code.
- Document new environment variables/configuration without reading `.env`.

## Workflow

1. Read `composer.json`, `composer.lock`, Symfony Flex state, configured repositories, PHP extensions, and CI/deployment constraints without reading secrets.
2. State the capability gap before selecting a package. Compare Symfony-native support, a small local implementation, and maintained packages.
3. Check PHP/Symfony constraints, transitive dependency impact, release cadence, open security advisories, license, abandonment status, and upgrade path.
4. Make the smallest Composer change that proves compatibility. Review lockfile changes for unrelated upgrades and inspect every Flex recipe diff before accepting it.
5. Audit generated configuration, routes, migrations, public assets, environment variables, service registration, worker requirements, and rollback/removal behavior.
6. Run the project test/lint/static-analysis suite, `composer audit`, container compilation, and relevant functional smoke tests.

Do not introduce a package to hide a poorly placed responsibility. Use [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md) to keep vendor adapters at infrastructure boundaries and application services vendor-independent.

## Common Commands

```bash
composer validate --strict
composer audit
composer outdated --direct
composer why-not symfony/framework-bundle <version>
composer recipes
```

## Output

Include packages and exact constraints changed, rejected alternatives, lockfile/recipe/config impact, security/license/maintenance assessment, operational requirements, rollback plan, and verification evidence.
