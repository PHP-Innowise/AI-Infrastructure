# Hooks (Infrastructure-Creator)

These five POSIX-bash hooks guard the generator's own operation. Four are identical across the `.claude/`, `.cursor/`, and `.codex/` editions; only the registration schema differs (`settings.json` for Claude, `hooks.json` for Cursor and Codex). The fifth, `subagent-gate.sh`, is tool-owned: each host exposes a different subagent-gate contract (Claude PreToolUse exit codes, Cursor subagentStart permission JSON, Codex spawn_agent deny), so its three copies differ by design.

| Script | Trigger | Purpose |
| --- | --- | --- |
| `local-context.sh` | session start | Prints an orientation banner (this is a generator; target path required; read-only Phase 1). |
| `bash-validator.sh` | before a shell command | Blocks destructive/unsafe commands (`rm -rf` on broad paths, force-push, `--no-verify`, `.env` reads/writes, `DROP TABLE/DATABASE`). |
| `file-naming-validator.sh` | before/after a file write or edit | Warns when `.md` files under `tasks/`/`specs/` are not skill-prefixed. |
| `loop-detection.sh` | after an edit | Warns when the same file is edited 5+ times within 2 minutes. |
| `subagent-gate.sh` | before a subagent spawns | Restricts delegation to this project's own agents; host built-in subagents are denied (paired with the `Agent(...)` deny rules in `settings.json` and the `[features]`/`[agents]` switches in `.codex/config.toml`). |

All scripts read the tool-input JSON on stdin and communicate via exit codes: `0` pass, `1` warn (continue), `2` block. The four shared scripts avoid external dependencies (no `jq`) so they run anywhere Python-free bash runs; `subagent-gate.sh` prefers `jq` and falls back to `php`/`python3`, failing open when none is available.

In the Claude and Cursor editions `subagent-gate.sh` also serializes agents marked `writes: true`: one runs at a time, behind a `/tmp` lock keyed by the repository path (TTL 30 min, override with `SUBAGENT_WRITE_LOCK_TTL_MINUTES`), released when the holder finishes or by expiry. The Codex edition holds no lock - it denies multi-agent spawning outright, so there is nothing to serialize. The lock is advisory and machine-local, so two containers or two machines working the same branch are not serialized against each other. It covers subagent spawns only - the main conversation's own edits are not gated, and nothing running outside the tool is. It fails open with no JSON extractor, with an unreadable agent roster, and - for the check-and-take race only - without `flock`. Past the TTL the holder stops blocking.

To tighten `file-naming-validator.sh` from warn to block, change its final `exit 1` to `exit 2`.
