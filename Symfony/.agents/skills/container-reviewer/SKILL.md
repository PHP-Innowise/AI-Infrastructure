---
name: container-reviewer
description: Review Symfony service container config, autowiring, aliases, tags, decorators, env vars, compiler passes, and visibility.
phase: quality
flow-next: verify
flow-alternatives: [coder, code-reviewer]
---

# Symfony Container Reviewer

## Review Workflow

1. Inspect `services.yaml`, environment-specific config, bundle extensions, attributes, compiler passes, and the constructors of affected services.
2. Use `debug:container`, `debug:autowiring`, and `debug:config` to inspect effective wiring rather than inferring it from one file.
3. Trace every alias, bind, tag, decorator, factory, and compiler-pass mutation to a real framework or application requirement.
4. Run `lint:container` and focused tests after changes; check production environment compilation when service configuration changed.

## Checks

- Services are autowireable and constructor dependencies are explicit.
- Aliases bind narrow interfaces intentionally at real substitution or package boundaries; no interface-per-class ceremony.
- Tags/decorators/compiler passes are justified and documented.
- Env vars are referenced through config without reading `.env`.
- Services are private unless framework integration requires public visibility.
- `_defaults`, `_instanceof`, resource exclusion, binds, priorities, and autoconfiguration do not create surprising global behavior.
- Decorators preserve the wrapped contract, failure behavior, idempotency, and observability.
- Scalar configuration is typed and validated by bundle/configuration boundaries where applicable.
- Test-container access is not used to justify public production services or service-locator application code.
- `php bin/console lint:container` and relevant debug commands pass.

Compare dependency direction and boundary abstractions with [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md).

## Output

Report findings by severity with the effective service ID, source config/class, runtime impact, smallest Symfony-native correction, and verification command. Mark optional component checks N/A when the consuming project does not install them.
