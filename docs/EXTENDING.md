# Extending the Accelerator

This guide is for contributors who add or change accelerator behavior. It
covers the three maintained accelerators and the generator:

- [`Laravel/`](../Laravel/README.md) for Laravel-specific workflows;
- [`Symfony/`](../Symfony/README.md) for Symfony-specific workflows;
- [`PHP Core/`](../PHP%20Core/README.md) for framework-neutral Composer, PSR,
  and native-PHP workflows;
- [`Infrastructure-Creator/`](../Infrastructure-Creator/README.md) for
  evidence-driven generation into an external PHP project.

Current policy, code, configuration, schemas, tests, hooks, and the active
edition's Definition of Done take precedence over this guide.

## Choose the Correct Home

Put a change in the narrowest place that owns the behavior.

### Laravel

Use `Laravel/` when the workflow depends on Laravel concepts such as Artisan,
Eloquent, Form Requests, Policies/Gates, Blade, Livewire, Inertia, Jobs,
Notifications, or Laravel package conventions. Do not add Laravel assumptions
to `PHP Core/`.

### Symfony

Use `Symfony/` when the workflow depends on Symfony concepts such as
Controller-Service-Repository placement, Doctrine, Forms, Validator, Voters,
Messenger, Twig, Symfony UX, the service container, or `bin/console`. Preserve
Symfony's own layering and version policy rather than mechanically translating
Laravel guidance.

### PHP Core

Use `PHP Core/` for framework-neutral PHP behavior: Composer and PSR
conventions, explicit HTTP boundaries, PDO or documented data layers,
framework-neutral validation and authorization, and portable PHP tooling.
Framework-specific examples belong in their framework sibling.

### Infrastructure-Creator

Use `Infrastructure-Creator/` only when changing how a bespoke accelerator is
scanned, profiled, generated, or verified. It is a generator, not a fourth
runtime accelerator. Its findings and generated artifacts must be grounded in
the target project's files; it must not invent frameworks, integrations,
permissions, owners, or tooling.

Infrastructure-Creator writes only the editions selected during the
clarifying interview, always creates the shared Memory Bank and Project Brain
planes, and uses its collision guard before writing into an existing target.
Do not claim that an authored accelerator change will be propagated or
generated automatically unless a current generator skill and validator
actually implement that behavior.

## Source and Edition Model

Each maintained accelerator has shared policy plus tool-native editions:

| Concern | Claude Code | Cursor | Codex |
| --- | --- | --- | --- |
| Skills | `.claude/skills/` | `.cursor/skills/` | `.agents/skills/` |
| Agent wrappers | `.claude/agents/` | `.cursor/agents/` | none by default |
| Command wrappers | `.claude/commands/` | `.cursor/commands/` | none |
| Rules/policy | root `AGENTS.md` and edition docs | root `AGENTS.md`, `.cursor/rules/*.mdc` | root `AGENTS.md` and edition docs |
| Hooks | `.claude/settings.json` + `.claude/hooks/` | `.cursor/hooks.json` + `.cursor/hooks/` | `.codex/config.toml`, `.codex/hooks.json`, `.codex/hooks/` |

For shared skill parity, `.agents/skills/` is canonical. This is declared by
`project-brain/config/runtime.json` as `"canonical_edition": ".agents"` and is
what `context.py parity` uses when detecting mirror drift. A complete change
must leave supported copies semantically aligned and byte-identical where the
repository's DOD requires it.

Tool wrappers are adapters, not alternate implementations:

- Claude commands use Claude frontmatter and spawn one matching agent.
- Cursor commands use Cursor's `name`/`description` schema; Cursor agents use
  the reduced supported frontmatter.
- Codex discovers skills directly from `.agents/skills/`. Do not add fake
  `.codex/commands/` or `.codex/agents/` wrappers.
- Hook event names, payloads, timeout units, and failure behavior are
  tool-specific. Preserve the native wiring documented in each
  `<edition>/hooks/README.md`.

## Adding or Updating a Skill

1. Select the owning accelerator or the generator using the placement rules
   above.
2. Read its root `AGENTS.md`, the active edition's `DOD.md` and
   `STABILIZATION.md`, related skills, examples, and tests.
3. Create or update `.agents/skills/<skill-name>/SKILL.md` and any cohesive
   `references/`, `assets/`, `scripts/`, or test resources. Use a lowercase
   kebab-case name and keep frontmatter within the accepted schema.
4. Mirror the skill to `.claude/skills/` and `.cursor/skills/`. Apply only
   necessary tool-path transformations; do not let workflow semantics drift.
5. Validate every changed skill directory:

   ```bash
   python3 .agents/skills/skill-creator/scripts/quick_validate.py \
     .agents/skills/<skill-name>
   python3 .claude/skills/skill-creator/scripts/quick_validate.py \
     .claude/skills/<skill-name>
   python3 .cursor/skills/skill-creator/scripts/quick_validate.py \
     .cursor/skills/<skill-name>
   ```

6. Add or update focused tests when the skill includes executable behavior,
   schemas, retrieval contracts, lifecycle rules, or a regression-prone
   structural requirement.
7. Run parity and the active DOD checks before reporting completion.

`quick_validate.py` checks basic skill structure and frontmatter. It does not
prove workflow quality, wrapper correctness, reference resolution, hook
safety, or cross-edition parity; those require the relevant tests and DOD.

## Commands and Agents

For Claude Code and Cursor, a normal command-agent-skill chain contains:

1. one command entry point;
2. one agent wrapper that invokes exactly one skill;
3. one skill that owns the workflow and stops with a Context Summary and Next
   Steps.

Keep wrappers thin. Workflow rules belong in the skill, not duplicated across
commands and agents. Update flow maps and every `flow-next`,
`flow-alternatives`, `related`, `invokes`, and `spawns` reference. Confirm all
references resolve.

Codex is deliberately different: add or update the skill under
`.agents/skills/`, then invoke it by discovered name. Codex custom slash-command
and one-skill agent mirrors are unsupported in this repository. Explicit Codex
subagent configuration is a separate feature and must not be inferred from the
Claude/Cursor wrapper model.

## Rules, Policy, and Stabilization

Use the strongest appropriate layer:

- root `AGENTS.md` for shared mandatory policy;
- `.cursor/rules/*.mdc` for Cursor-native always-on routing or standards;
- `SKILL.md` for operation-specific procedure;
- `DOD.md` for verifiable completion gates;
- hooks or tests for narrow behavior that can be checked reliably;
- `STABILIZATION.md` for the incident-to-rule process.

Do not describe prose policy as mechanically enforced. If a repeated failure
should be blocked, add the smallest safe hook or validator check and a
regression test where practical. Hooks must remain narrowly scoped and must
not become an undocumented sandbox.

Keep `DOD.md`, `GOLDEN-PRINCIPLES.md`, and `STABILIZATION.md` aligned across
the three editions. Infrastructure-Creator's DOD requires these companion
documents and its skill trees to be byte-identical where specified.

## Hooks

When changing hooks:

1. edit the canonical hook/runtime source identified by
   `scripts/build_mirrors.py` rather than a generated `.cursor/` or other
   mirror; for shared runtime templates this may require the byte-identical
   canonical file in each maintained edition;
2. update native wiring (`settings.json`, `hooks.json`, or `config.toml`);
3. preserve executable bits and run `bash -n` on shell scripts;
4. test allowed, warned, blocked, missing-key, crash, and timeout paths;
5. update the edition's hook README;
6. regenerate mirrors with `python3 scripts/build_mirrors.py --write`, review
   the generated diff, and verify it with `--check`;
7. verify that session banners remain metadata-only.

Current session hooks may report mode, index health/staleness, active binding
count, and validation status. They must not automatically index, retrieve,
print, or inject Project Brain or Memory Bank record content into the session
banner. Cursor's `sessionStart` hook may silently refresh the ignored
`.cursor/rules/working-memory.mdc` file from the most recently available
capsule; that file-only refresh is permitted, but its contents must not be
printed as banner output.

## Protocols, Schemas, Runtime, and Context Parity

Changes to Project Brain or the Local Context Engine are contract changes.
Review together:

- `project-brain/PROTOCOL.md`;
- `project-brain/config/runtime.json`, `providers.json`, and `telemetry.json`;
- strict schemas and templates;
- `memory-bank/scripts/context.py`, `brain_runtime.py`, and
  `context_retrieval.py`;
- Project Brain and Memory Bank tests;
- [`CONTEXT-MODES.md`](CONTEXT-MODES.md).

Keep common runtime assets byte-identical across Laravel, Symfony, and PHP
Core unless a file is explicitly framework-specific. Runtime configuration
contains the framework label and allowed policy differences. Validate:

```bash
python3 memory-bank/scripts/context.py validate
python3 memory-bank/scripts/context.py parity
python3 memory-bank/scripts/validate.py
python3 -m unittest discover project-brain/tests
python3 -m unittest discover memory-bank/tests
```

Run these from each affected accelerator. `parity` must fail on drift from
canonical `.agents/skills`; do not hide or normalize a real mismatch.

`parity` reports every drifted path in one run, and `--json` returns the same
list on the failure path. A file only one mirror carries, or one missing from a
mirror, counts as drift too.

Two things are exempt, both declared explicitly in
`memory-bank/scripts/context_retrieval.py`:

- `SKILL FLOW.md`, the per-edition orchestration catalog.
- Skills listed in `EDITION_OWNED_SKILLS`, whose body documents the host tool
  rather than a workflow this repository owns. `skill-creator` is there because
  it drives each product's own CLI (`codex exec`, `cursor-agent --print`,
  `claude -p`) with different environment variables and a different extension
  model per tool - Cursor builds command and agent wrappers, Codex is forbidden
  from creating them. Byte-parity would require telling a Codex user to run
  `cursor-agent`.

Everything else must be byte-identical across mirrors. When a shared skill needs
to name an edition directory, phrase it neutrally - "the active edition's
`DOD.md`" - rather than substituting `.claude`/`.cursor`/`.codex` per mirror.
Per-mirror substitution is what turned this gate permanently red, after which it
stopped being run at all. Adding an entry to `EDITION_OWNED_SKILLS` is a
documented exemption and needs the same justification in review; it is not a way
to silence drift you do not want to fix.

## Tests, Examples, and Documentation

- Add tests for behavior, not just file presence. Cover the highest-risk
  failure path: stale revisions, unauthorized owners, source freshness,
  privacy exclusion, rollback, corruption, or unsupported edition shape as
  applicable.
- Keep examples illustrative. They never override policy, schemas, tests, or
  current code.
- Update living specs when architecture or behavioral contracts change.
- Update public READMEs and operator docs when discovery, invocation, setup,
  safety, or recovery changes.
- Validate headings, links, code fences, and Markdown syntax for changed docs.
- Update the owning accelerator's `CHANGELOG.md` when release notes or
  generator behavior changes; changes to the shared core (memory/context
  core, Project Brain, hooks, mirror machinery, root `scripts/`) belong in
  the root `CHANGELOG.md`, which CI enforces on pull requests.

For generated targets, run Infrastructure-Creator's bundled structural gate:

```bash
python3 \
  Infrastructure-Creator/.agents/skills/bootstrap-verifier/scripts/validate_generated.py \
  --target /path/to/target \
  --editions claude,cursor,codex
```

Pass only the editions selected for that target. The validator checks
frontmatter, references, edition scope, hook syntax/executable bits, Memory
Bank validation, and placeholders. It does not authorize overwrites or prove
that generated domain claims are correct.

## Definition of Done and Changelog

Choose the DOD tier that matches the change:

- documentation-only work uses the minimum tier;
- implementation uses standard checks;
- merge/release work uses full checks;
- Infrastructure-Creator mirroring, release, and stack adaptation use its
  dedicated higher tiers.

Report every command as pass, fail, or `N/A - tooling not configured`. Do not
install missing tooling without approval, and do not claim success while a
relevant validator fails. Record user-facing or release-relevant changes in
the affected `CHANGELOG.md` (the root one when the change is to the shared
core).

## Changing a Description

An agent's or skill's `description:` is the selector: it is what decides which
of roughly forty-five skills answers a request. Changing one, or adding or
removing an entry from the roster, changes routing for every user of that
edition — and until there was eval data, no change of that kind carried a
single number about whether the choice got better or worse.

A pull request that edits any `description:`, or that adds or removes an agent
or a skill, must attach the delta to that edition's routing baseline:

```bash
python3 scripts/routing_eval.py --edition Symfony --runs 5 --write-baseline
```

This invokes a model, costs money and takes minutes, so it is deliberately
outside CI — the same treatment `docs/CI.md` gives the external harness. Run
it before and after, and put both pass rates in the pull request.

A baseline records the `policy_digest` of the surface that produced it, so a
stale one is mechanically visible: if the digest in
`install/policy-lock/<edition>-routing-baseline.json` does not match the one in
`<edition>/.accelerator-policy-lock.json`, the numbers describe a different
roster.

Read `miss` and `wrong` differently. A `miss` — nothing triggered — means the
description is too narrow. A `wrong` — a neighbour triggered — means it
overlaps that neighbour. They are repaired in opposite directions, which is
why the runner never collapses them into one failure count.

The cheap check that needs no model is
`python3 -m unittest tests.test_check_stabilization tests.test_policy_lock`
plus the per-edition skill-routing floor in
`project-brain/tests/fixtures/skill-routing-golden.json`, which asserts an
acceptable skill still reaches the capsule's two procedural slots at least as
often as it does today.

## Downstream Customization and Upgrades

Treat a copied accelerator or Infrastructure-Creator output as downstream
project infrastructure:

1. keep project-specific policy, skills, and hooks in the downstream
   repository;
2. record local deviations and their reasons;
3. upgrade in a review branch or worktree;
4. compare upstream and downstream files by logical component;
5. preserve target-specific evidence, permissions, commands, owners, and
   framework versions;
6. reapply only compatible upstream changes;
7. run downstream DOD, parity, context, hook, and application tests;
8. review the final diff before promotion.

Do not overwrite a downstream accelerator wholesale by default. Generated
projects may contain intentional project-specific policy and evidence. Use the
Infrastructure-Creator collision decision (`overwrite`, `merge`, or `abort`)
when generation is involved, and prefer `merge` plus human review for an
existing installation.

For security boundaries and safe extension constraints, see
[`SECURITY.md`](SECURITY.md). For diagnosis and recovery, see
[`TROUBLESHOOTING.md`](TROUBLESHOOTING.md).
