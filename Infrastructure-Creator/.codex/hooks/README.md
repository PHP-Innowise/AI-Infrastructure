# Hooks (Infrastructure-Creator)

These four POSIX-bash hooks guard the generator's own operation. They are identical across the `.claude/`, `.cursor/`, and `.codex/` editions; only the registration schema differs (`settings.json` for Claude, `hooks.json` for Cursor and Codex).

| Script | Trigger | Purpose |
| --- | --- | --- |
| `local-context.sh` | session start | Prints an orientation banner (this is a generator; target path required; read-only Phase 1). |
| `bash-validator.sh` | before a shell command | Blocks destructive/unsafe commands (`rm -rf` on broad paths, force-push, `--no-verify`, `.env` reads/writes, `DROP TABLE/DATABASE`). |
| `file-naming-validator.sh` | before/after a file write or edit | Warns when `.md` files under `tasks/`/`specs/` are not skill-prefixed. |
| `loop-detection.sh` | after an edit | Warns when the same file is edited 5+ times within 2 minutes. |

All scripts read the tool-input JSON on stdin and communicate via exit codes: `0` pass, `1` warn (continue), `2` block. They avoid external dependencies (no `jq`) so they run anywhere Python-free bash runs.

To tighten `file-naming-validator.sh` from warn to block, change its final `exit 1` to `exit 2`.
