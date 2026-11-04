# Native host launch contract

The installed runtime is the Rust executable at `memory-bank/bin/infra-engine` (`infra-engine.exe` on Windows). Attached sessions use the selected native engine explicitly and pass the accelerator root as an argument. The runtime does not require Python.

Installed and generated projects on POSIX commit a portable launch (`infra_tooling::launch`), so a clone on another machine or at another path works once its own engine is installed:

| Host | Hook command | Memory MCP |
| --- | --- | --- |
| Claude Code | `"$CLAUDE_PROJECT_DIR"/.claude/hooks/ROLE.sh` (the notification role runs the engine under `$CLAUDE_PROJECT_DIR` directly) | `sh -c LAUNCHER sh mcp --host claude`; Claude Code sets `CLAUDE_PROJECT_DIR` in the server's environment |
| Cursor | `.cursor/hooks/ROLE.sh`, run from the project root | `sh -c LAUNCHER sh ${workspaceFolder} mcp --host cursor` |
| Codex | `sh -c 'exec "$(git rev-parse --show-toplevel 2>/dev/null \|\| pwd)/PREFIX.codex/hooks/ROLE.sh"'`, independent of the login shell Codex uses and of the session's subdirectory; `PREFIX` is the install root below the Git top level | `sh -c LAUNCHER sh mcp --host codex` from Git's top level |

Each `ROLE.sh` launcher and the MCP `LAUNCHER` script run `memory-bank/bin/infra-engine` of their checkout. A linked worktree, which Git gives no copy of the ignored engine, uses the engine and root of the same directory in its main checkout (`git rev-parse --git-common-dir`). Without an engine, the shell guard's launcher exits 2 with an install hint, the prompt launcher answers `{"continue": true}`, the session launcher prints a notice and the others exit 0. The MCP launcher script carries no `${` (hosts expand those), single quote or backslash. Installer, generator output (`repo hook-handler`), the dispatcher and native semantic gates share this formatter.

`infra_tooling::attach::native_handler` formats bound launch fields from an absolute executable and literal argv, and `hook_handler` supplies `hook HOST ROLE --root ABSOLUTE_ROOT`. Attached sessions, user-level dispatchers and native Windows installations use them. On POSIX the host then receives an ordinary shell command containing `exec` and individually quoted literal arguments; the shell performs no policy, root discovery or memory work.

On native Windows:

- Claude Code receives its documented exec-form handler: an absolute `command` plus an `args` array. This requires a client supporting the [documented native command-and-args hook schema](https://code.claude.com/docs/en/hooks#exec-form-and-shell-form). That reference does not state a minimum release for this schema; compatibility is expressed as a required capability, not a guessed version.
- Cursor and Codex receive an explicit Windows PowerShell launch (version 5.1 or later), resolved under the actual system directory through `GetSystemDirectoryW` with no profile or interaction. The UTF-16LE encoded command only establishes UTF-8 streams, invokes the single selected Rust executable with literal arguments and propagates its exit code. The encoding is host-shell quoting, not an additional implementation of runtime behavior.
- Cursor documents [PowerShell hook execution on Windows](https://cursor.com/docs/hooks). Codex currently wraps hook commands with its [configured command shell, defaulting to COMSPEC/cmd.exe on Windows](https://github.com/openai/codex/blob/main/codex-rs/hooks/src/engine/command_runner.rs); naming the absolute system PowerShell executable avoids a project directory or PATH entry shadowing it. Cursor uses PowerShell's invocation operator; Codex receives the quoted absolute executable for its command shell.

No Python, Bash or WSL interpreter is required for the native Windows handler. Native Windows and actual client execution remain necessary release evidence; generating handler JSON alone does not establish that evidence.

Windows PowerShell handlers are materialized on the target Windows system. A non-Windows authoring process cannot invent its system directory; cross-system Windows materialization requires an explicit trusted target-shell contract and is otherwise refused. Claude's native exec-form does not use this shell.
