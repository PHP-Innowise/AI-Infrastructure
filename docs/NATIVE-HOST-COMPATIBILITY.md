# Native host launch contract

The installed runtime is the Rust executable at `memory-bank/bin/infra-engine` (`infra-engine.exe` on Windows). Attached sessions use the selected native engine explicitly and pass the accelerator root as an argument. The runtime does not require Python.

`infra_tooling::attach::native_handler` formats host launch fields from an absolute executable and literal argv. `hook_handler` supplies `hook HOST ROLE --root ABSOLUTE_ROOT`. Installer, attached sessions, generator output and native semantic gates share this formatter.

On POSIX, the host receives an ordinary shell command containing `exec` and individually quoted literal arguments. The shell performs no policy, root discovery or memory work.

On native Windows:

- Claude Code receives its documented exec-form handler: an absolute `command` plus an `args` array. This requires a client supporting the [documented native command-and-args hook schema](https://code.claude.com/docs/en/hooks#exec-form-and-shell-form). That reference does not state a minimum release for this schema; compatibility is expressed as a required capability, not a guessed version.
- Cursor and Codex receive an explicit Windows PowerShell launch (version 5.1 or later), resolved under the actual system directory through `GetSystemDirectoryW` with no profile or interaction. The UTF-16LE encoded command only establishes UTF-8 streams, invokes the single selected Rust executable with literal arguments and propagates its exit code. The encoding is host-shell quoting, not an additional implementation of runtime behavior.
- Cursor documents [PowerShell hook execution on Windows](https://cursor.com/docs/hooks). Codex currently wraps hook commands with its [configured command shell, defaulting to COMSPEC/cmd.exe on Windows](https://github.com/openai/codex/blob/main/codex-rs/hooks/src/engine/command_runner.rs); naming the absolute system PowerShell executable avoids a project directory or PATH entry shadowing it. Cursor uses PowerShell's invocation operator; Codex receives the quoted absolute executable for its command shell.

No Python, Bash or WSL interpreter is required for the native Windows handler. Native Windows and actual client execution remain necessary release evidence; generating handler JSON alone does not establish that evidence.

Windows PowerShell handlers are materialized on the target Windows system. A non-Windows authoring process cannot invent its system directory; cross-system Windows materialization requires an explicit trusted target-shell contract and is otherwise refused. Claude's native exec-form does not use this shell.
