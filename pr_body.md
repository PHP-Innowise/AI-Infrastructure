## Summary

Seven commits modernizing the whole accelerator platform, produced by a multi-agent research pass (5 subsystem readers → 24 proposals → adversarial verification) followed by seven implementation phases, each with its own adversarial review and repair loop.

### Memory core (`570df77`)
- Automatic promotion unblocked: `observed → verified` authority transition (CAS-guarded, ledgered), surfaced through `brain-update --authority` and the verify skill
- Heavy maintenance (close-merged / promote / compact) moved to the flush boundary; batched merged-branch detection; cached default branch
- Manual `progress` no longer clobbered by auto-checkpoints (`auto_checkpoint` field); turn outcomes persisted to `last-turn-report.json` and shown in the next capsule
- Retrieval: query distilled from the whole prompt by index rarity, BM25 augmented with recency decay and authority weight, single stat pass, client `Task/Epics` specs removed from the index
- Race-free chunk IDs `MEM-YYYYMMDD-<hex>` + deterministic `context.py reindex-bank`; multi-machine continuity via `context.py rebind` + auto-rebind

### Enforcement hooks (`d4c2ce3`)
- Hardened generation everywhere (jq/php/python3 JSON extraction, stderr before exit 2, loud fail-open), including Infrastructure-Creator
- Loop counters namespaced by repository hash and reset on SessionStart — no more cross-project sharing or permanent file blocks
- Single-grep bash-validator, Codex tool_name early-exits, lighter SessionStart (validation cache, composer.lock version probe)
- Cursor finally receives the task capsule via an `alwaysApply` rule (`.cursor/rules/working-memory.mdc`, gitignored)
- First regression suites for the hook layer (4 editions)

### Single source of truth (`8433b9a`)
- `scripts/build_mirrors.py` regenerates `.claude`/`.cursor`/`.codex` mirrors from canon via declarative `MIRROR_RULES`; `--check` in CI
- Parity extended to hooks, commands, agents, DOD family, and skill `*.py`; new `parity --cross-edition` keeps the shared core byte-identical across editions

### Infrastructure-Creator (`6160482`)
- Generated accelerators now ship the full context-brain runtime and working-memory hooks; `validate_generated.py` smoke-runs the target and verifies wiring (dead-hooks bug class closed)
- Upgrade path: `VERSION` (1.4.0), generation stamp, `.infra-manifest.json` with per-file sha256, and the new `infra-update` skill

### Governance & tooling (`efd14d0`, `fe810e2`, `10329e6`)
- Relative-link checker with all broken links fixed; consolidated root `CHANGELOG.md`; per-edition `VERSION`
- Startup token budget made measurable (`context_budget.py` + pinned ceilings); Symfony policy deduplicated; 17 wordy skill descriptions tightened with no trigger term lost
- CI: seven unittest suites, full + cross-edition parity, mirror check, `bash -n` + shellcheck (error severity), JSON validation, link checker, budget check, core-changelog gate

## Verification

- 789 tests green across the seven suites (132 memory-bank + 124 project-brain per edition, 21 Infrastructure-Creator)
- `build_mirrors --check`, `parity` + `--cross-edition`, `check_links`, `context_budget --check`, changelog gate — all green at HEAD
- Every CI step was executed locally with the exact YAML commands (shellcheck 0.11.0: 0 findings)
- End-to-end scenarios exercised on throwaway git repos: promotion lifecycle, second-machine rebind, capsule budget, synthetic accelerator generation (full and merge modes)

## Notes for reviewers

- Client product specs under `Task/` were removed from the retrieval index but intentionally left in the repository — deleting or relocating them is an owner decision
- Files touched by several phases are committed once, in their dominant topical commit
- This PR's first CI run happens right here — the workflow triggers on pull requests

🤖 Generated with [Claude Code](https://claude.com/claude-code)
