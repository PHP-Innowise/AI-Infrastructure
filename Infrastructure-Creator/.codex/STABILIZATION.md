# Stabilization - Infrastructure-Creator

How to turn a mistake this generator made into a durable rule, so it does not recur. Stabilization is the loop that keeps the generator trustworthy over time.

## When To Stabilize

Stabilize whenever the generator:

- Produced a finding that turned out to be wrong or unsupported by evidence.
- Generated an artifact that failed `bootstrap-verifier` for a reason that could have been prevented.
- Wrote (or nearly wrote) into a target without an explicit collision decision.
- Emitted an edition that was not selected, or skipped one that was.
- Carried a placeholder, a secret, or a stale cross-reference into output.

## The Loop

1. **Capture the failure precisely.** What did the generator do, against which target, and what was the observable wrong result? Reference the run's `tasks/TASK-{N}/` artifacts.
2. **Find the smallest missing rule.** Was it a policy gap (`AGENTS.md`), a checklist gap (`DOD.md`), a hook gap (a guard that should have blocked it), or a skill-instruction gap (a `SKILL.md` that was ambiguous)?
3. **Write the rule where it will be enforced.** Prefer the strongest layer: a hook that blocks it > a `DOD.md` check that catches it > a `SKILL.md` instruction that prevents it > a note. Wishes in prose are the weakest and last resort.
4. **Add a regression check if possible.** For structural mistakes, extend `bootstrap-verifier` (or the target's `validate_generated.py`) so the same class of error is caught mechanically next time.
5. **Record it.** Note the change in `CHANGELOG.md` if it changes generator behavior.

## Examples

- *Mistake:* a scanner reported "uses Redis" because a Redis client was in `composer.json`, but it was a dev-only dependency never wired in. *Rule:* `integration-scanner` must distinguish `require` from `require-dev` and confirm runtime wiring (config/service registration) before marking an integration `confirmed`; downgrade to `inferred` otherwise.
- *Mistake:* generation created a `.codex/` folder for a Cursor-only team. *Rule:* `hook-forge`/`agent-forge`/`command-forge` must read the profile's AI Tool Selection field and hard-skip unselected editions; `bootstrap-verifier` must assert no unselected edition folder exists.
- *Mistake:* a generated skill's `related:` pointed at a skill that was never generated. *Rule:* `skill-flow-composer` must build cross-references only from the actually-generated skill set, and `bootstrap-verifier` must fail on any dangling reference.

Prefer enforcement over exhortation. A rule that a hook or validator enforces is worth ten paragraphs of guidance.
