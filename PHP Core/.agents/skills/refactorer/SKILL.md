---
name: refactorer
description: Perform behavior-preserving refactors and safe PHP upgrades in native PHP. Use to reduce duplication, extract seams, improve types, or apply Rector rules without changing observable behavior. Triggers on "refactor", "clean up", "extract", "reduce duplication", "modernize", "upgrade PHP".
phase: execution
flow-next: verify
flow-alternatives: [code-reviewer, test-generator, performance-optimization]
related: [coder, test-generator, code-reviewer, architect]
---

# Refactorer

## Overview

Improve code structure without changing what it does. The safety contract of refactoring is: observable behavior stays identical, proven by tests before and after.

## Scope Boundary

If a change alters observable behavior — new feature, bug fix, different output — that is `/coder`, not this skill. The moment you need to change what the code *does*, stop refactoring, ship the behavior change via `/coder`, then refactor separately. Keep the two in separate commits so review and rollback stay clean.

## The Safety Rule

```
NO REFACTOR WITHOUT A CHARACTERIZATION SAFETY NET
```

Before changing structure, ensure tests cover the behavior you are about to move. If coverage is missing, add characterization tests first (or hand off to `/test-generator`), then refactor.

## Workflow

1. **Pin behavior.** Run the existing tests; if the area is untested, write characterization tests that capture current behavior (including quirks). Record a green baseline.
2. **Refactor in small steps.** One transformation at a time; keep the suite green after each step. Commit-sized increments, not a big-bang rewrite.
3. **Modernize types.** Add `declare(strict_types=1)`, parameter/return/property types, and readonly where safe.
4. **Automate where possible.** Use Rector for mechanical, rule-based changes (dead code, type declarations, PHP version migrations); review each diff.
5. **Re-verify.** Run tests, static analysis, and formatting. Behavior and public signatures must be unchanged unless the task explicitly allows API changes.

## Common Refactorings

- Extract Method / Extract Class to break up long functions and God classes.
- Replace conditional sprawl with polymorphism or a lookup map.
- Introduce a value object to replace primitive obsession.
- Introduce an interface at a boundary to enable testing and swapping.
- Replace static/global calls with injected dependencies.
- Replace magic arrays with typed DTOs.
- Guard clauses to flatten nested conditionals.

## Rector Usage

```bash
vendor/bin/rector process --dry-run   # preview changes
vendor/bin/rector process             # apply, then review the diff
```

Prefer curated rule sets (dead code, code quality, a specific PHP version upgrade set). Never blindly accept a large Rector diff; review it like any change.

## Code Smell Catalog

Recognize the smell, then apply the matching refactoring:

| Smell | Signal | Refactoring |
| --- | --- | --- |
| Long method | Does many things; hard to name | Extract Method; guard clauses |
| Large/God class | Too many responsibilities | Extract Class; split by responsibility |
| Long parameter list | 4+ params, booleans | Introduce Parameter Object / DTO |
| Primitive obsession | Raw strings/ints for domain concepts | Introduce Value Object / enum |
| Feature envy | Method uses another object's data more than its own | Move Method |
| Shotgun surgery | One change edits many files | Consolidate behavior behind one seam |
| Duplicated logic | Copy-pasted blocks | Extract shared method/class |
| Conditional sprawl | Nested `if`/`switch` on type | Polymorphism or lookup map |
| Temporal coupling | Methods must be called in a hidden order | Encapsulate the sequence |
| Static/global calls | `new`/globals inside logic | Inject dependency behind interface |

## Large Refactors: Strangler Fig

For risky, large-scale change, do not rewrite in place. Build the new implementation beside the old, route a slice of behavior to it behind a seam (interface/feature flag), verify parity, migrate slice by slice, then remove the old path. Each step keeps the suite green and is independently revertible.

## Boundaries

- Do not change behavior; if you discover a bug, note it and hand off to `/coder` or `/debugger` rather than silently "fixing" it inside a refactor.
- Do not change public signatures/contracts unless the task explicitly scopes an API change.
- Keep refactor commits separate from behavior-change commits so review and rollback stay clean.

## Verification

```bash
composer test        # must match the pre-refactor result
composer analyse     # equal or fewer errors than baseline
composer lint
```

## Final Output

Return what was refactored, the safety net used, the before/after test result, any bugs found (not fixed here), Context Summary, and next step (`/verify`, `/code-reviewer`, or `/test-generator`).
