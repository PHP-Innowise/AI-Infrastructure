---
name: skill-creator
description: Create, modify, validate, and evaluate Cursor skills. Use when adding specialized workflows, improving existing skills, tightening trigger descriptions, reorganizing resources, running automated positive/negative trigger evaluations, optimizing descriptions, or benchmarking with-skill versus baseline output quality.
phase: understanding
---

# Cursor Skill Creator

Create focused, discoverable skills under `.cursor/skills/<skill-name>/` without depending on Claude Code, `claude -p`, or `.claude/commands`.

## Workflow

1. Clarify the capability, trigger phrases, non-goals, expected output, dependencies, and success criteria.
2. Inspect neighboring skills, root `AGENTS.md`, Cursor rules, and applicable naming/hooks before editing.
3. Choose a short lowercase kebab-case name that describes the capability.
4. Write `.cursor/skills/<name>/SKILL.md` with valid YAML frontmatter and concise imperative instructions.
5. Add only resources the workflow actually uses:
   - `references/` for detailed material loaded on demand;
   - `scripts/` for deterministic helpers with documented prerequisites;
   - `assets/` for files copied or transformed into outputs.
6. Add a Cursor-native command and agent wrapper only when the workflow should be directly invokable.
7. Validate syntax, local links, referenced commands, safety constraints, and trigger quality.
8. Test with realistic positive and negative prompts in a fresh Cursor chat when available. Use automated evaluation only when the user requires measured discovery behavior or benchmarking.

## Improving Existing Skills

1. Inventory the current trigger contract, workflow, resources, native metadata, wrappers, outputs, safety rules, links, and known evaluation behavior.
2. State the concrete quality gap before editing. Preserve useful behavior and prefer the smallest coherent change over a wholesale rewrite.
3. Keep Cursor-native frontmatter, paths, commands, agents, and discovery behavior intact; do not overwrite them with another edition's adapters.
4. Remove a resource only after proving no skill, wrapper, script, or documentation references it.
5. Use progressive disclosure or a governed shared example for reusable depth. For Symfony code workflows, enforce `AGENTS.md` and consult [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md); do not force PHP examples into non-code skills.
6. Re-run baseline positive, negative, and edge prompts after the revision and report manual versus measured evidence accurately.

## Frontmatter

Required:

```yaml
---
name: example-skill
description: "What the skill does and the concrete situations that should trigger it."
---
```

- Quote descriptions containing YAML-significant punctuation.
- Keep trigger conditions in `description`; the body is loaded only after discovery.
- Cursor agent and command frontmatter must use only Cursor-supported keys.

## Quality Rules

- Keep one skill responsible for one coherent workflow.
- Prefer progressive disclosure over a very long `SKILL.md`.
- Use maintained libraries and existing project helpers instead of embedding fragile replacements.
- Use portable scripts, explicit dependencies, deterministic output, and safe failure behavior.
- Do not include credentials, hidden network actions, destructive defaults, or surprising behavior.
- Do not copy Claude-only model, invocation, phase, or command metadata into Cursor adapters.
- Follow the one-skill-then-stop contract and provide Context Summary and Next Steps where required by policy.

## Validation

At minimum:

- parse YAML frontmatter with an installed YAML parser;
- confirm the directory name equals the frontmatter `name`;
- verify referenced local files exist;
- run syntax checks for bundled scripts;
- exercise 2-3 prompts that should trigger and 2-3 nearby prompts that should not;
- review generated artifacts against root naming and security policy.

### Automated Evaluation And Benchmarking

Use the bundled harness only when measured trigger behavior or comparative benchmarking is required. Native CLI calls may consume model credits.

Prerequisites:

- authenticated `cursor-agent` CLI available on `PATH`, or a compatible executable configured through `SKILL_CREATOR_CURSOR_CLI`;
- Python 3.10 or newer;
- PyYAML for `scripts/quick_validate.py` and `scripts/package_skill.py`; trigger evaluation, optimization, benchmarking, and HTML review otherwise use the Python standard library.

Create an evaluation set with unique positive and negative queries:

```json
[
  {"query": "Create a Cursor skill for API contract reviews", "should_trigger": true},
  {"query": "Fix this Doctrine query", "should_trigger": false}
]
```

From `.cursor/skills/skill-creator/`, run:

```bash
python3 -m scripts.run_eval \
  --eval-set /path/to/evals.json \
  --skill-path ../target-skill

python3 -m scripts.run_loop \
  --eval-set /path/to/evals.json \
  --skill-path ../target-skill \
  --results-dir /path/to/results
```

The harness creates a unique probe skill in a temporary minimal workspace, invokes `cursor-agent --print --output-format json`, and counts an exact marker emitted only when the probe body loads. The temporary workspace prevents the CLI from writing to the repository, is removed automatically, and the harness fails closed on CLI errors. Use `--model` only to override the CLI's configured model.

For output-quality benchmarks, compare repeated `with_skill` and `without_skill` runs using the schemas in [references/schemas.md](references/schemas.md), then aggregate and review them:

```bash
python3 scripts/aggregate_benchmark.py /path/to/benchmark
python3 eval-viewer/generate_review.py /path/to/benchmark
```

Treat trigger rates as probabilistic evidence. Use multiple runs, balanced positive and negative queries, held-out cases, and a fixed model/configuration for comparisons. Never represent fake-CLI tests as model-quality evidence.

## Output

Report the skill path, command/agent adapters created, trigger contract, resources added, validation performed, evaluation prompts/results, and remaining limitations. Distinguish syntax tests, deterministic adapter tests, live native evaluations, and manual review.
