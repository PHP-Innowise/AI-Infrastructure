# Tool Integrations

Each ready-made edition supports Claude Code, Cursor, and OpenAI Codex in the
same project. The tools share root policy and project context, but each keeps
its native discovery model. Do not make one tool load another tool's adapters.

## Directory Ownership

| Path | Owner | Contents |
| --- | --- | --- |
| `AGENTS.md` | Shared | Enforceable project/stack policy used across tools |
| `.agents/skills/` | Canonical skill edition | Skill workflows declared canonical by `project-brain/config/runtime.json`; Codex discovers them directly |
| `.claude/` | Claude Code | `CLAUDE.md` (the `@../AGENTS.md` import), commands, agent wrappers, skill mirrors, `settings.json`, hooks, and reference documents |
| `.cursor/` | Cursor | Commands, agents, skill mirrors, `.mdc` rules, `hooks.json`, hooks, and reference documents |
| `.codex/` | Codex | Trusted project config, hook wiring/scripts, and reference documents; not skills, commands, or agent wrappers |
| `memory-bank/` | Shared | Reviewed durable knowledge plus the ignored local context database |
| `project-brain/` | Shared | Governed tasks, handoffs, records, manifests, schemas, configuration, and promotion state |

All three ready-made runtime configurations set:

```json
{
  "canonical_edition": ".agents"
}
```

This makes `.agents/skills/` the declared behavioral reference for skill
parity and context indexing. It does not make Claude Code or Cursor execute
Codex wrappers. Their commands, agent definitions, frontmatter, rules,
settings, and hook schemas remain native and must be updated consistently
when canonical skill behavior changes.

## Commands, Agents, and Skills

### Claude Code

- `.claude/commands/*.md` provides user-facing slash commands.
- `.claude/agents/*.md` provides focused wrappers that execute one skill and
  stop.
- `.claude/skills/<name>/SKILL.md` is Claude Code's native mirror of the
  corresponding canonical workflow.
- `.claude/settings.json` owns project permissions and Claude hook wiring.

The normal route is command → agent → skill. A user may also request a skill
by name when the client supports skill discovery.

### Cursor

- `.cursor/commands/*.md` provides slash-command entry points.
- `.cursor/agents/*.md` provides Cursor-native focused agents.
- `.cursor/skills/<name>/SKILL.md` is Cursor's native skill mirror.
- `.cursor/rules/*.mdc` provides always-on and scoped Cursor policy.
- `.cursor/hooks.json` owns Cursor hook wiring.

Cursor is self-contained and does not need Claude files to supply these
components.

### Codex

- Codex discovers skills from `.agents/skills/<name>/SKILL.md`.
- Root `AGENTS.md` supplies policy; Codex walks from repository root toward
  the current working directory and combines applicable policy.
- `.codex/config.toml` supplies trusted project configuration and enables
  hooks.
- `.codex/hooks.json` and `.codex/hooks/*.sh` supply lifecycle hooks.

There is intentionally no Codex command layer and no copied one-skill agent
wrapper layer. Invoke a discovered skill by name or let Codex select it
implicitly. Do not create `.codex/commands/`, `.codex/agents/`, or textual
slash-command shims to imitate Claude Code or Cursor.

## Hook Ownership and Behavior

The same safety goals are adapted to each client's event schema:

| Purpose | Claude Code | Cursor | Codex |
| --- | --- | --- | --- |
| Session metadata | `SessionStart` | `sessionStart` | `SessionStart` |
| Shell safety | `PreToolUse` for Bash | `beforeShellExecution` | `PreToolUse` |
| File naming | `PreToolUse` for Write/Edit | `afterFileEdit` | `PreToolUse` |
| Edit-loop detection | `PostToolUse` for Edit | `afterFileEdit` | `PostToolUse` |
| Task Capsule into the prompt | `UserPromptSubmit` | `beforeSubmitPrompt` renders the prompt's capsule into an `alwaysApply` rule (`sessionStart` renders the branch's, and so does `stop` unless the rule holds the prompt's) | `UserPromptSubmit` |
| Turn checkpoint at turn end | `Stop` | `stop` | `Stop` |

## Automatic Memory Support by Tool

Automatic memory has two halves, and one of them depends on a client
capability that is not universal.

| | Claude Code | Codex | Cursor |
| --- | --- | --- | --- |
| Capsule retrieved into each prompt | yes (fresh) | yes (fresh) | **yes, through a rule** rendered from the prompt (same turn when Cursor reads its rules after the hook, else the next; not verified on a live client) |
| Turn change set buffered and flushed | yes | yes | yes |
| Explicit `context.py retrieve` / `memory` | yes | yes | yes |

Cursor cannot add context to a prompt from a hook. Its prompt-time event,
`beforeSubmitPrompt`, receives the prompt but answers only
`{"continue": true|false, "user_message": "..."}`; `sessionStart` can return
`additional_context`, but Cursor applies it racily and often not to the first
message, and `postToolUse` context is discarded for built-in tools. The one
channel Cursor sends with every request is an `alwaysApply` rule, so the
capsule is delivered through `.cursor/rules/working-memory.mdc`:

- The Cursor mirror of `working-memory-read.sh` runs on `beforeSubmitPrompt`.
  It retrieves for the prompt (`refresh --host cursor --sanitize --json`, with
  no `--session-id`: the rule is re-sent whole, so nothing in it may be left
  out as already handed), writes the rule atomically, and answers
  `{"continue": true}` on every exit path - memory never holds a prompt back.
  Cursor puts rules at the start of the model's context, so a rule rewritten
  for every prompt would cost the conversation its cached prefix each time;
  the hook replaces it only when the prompt retrieved an item the rule does
  not hold yet - a document, or another section of one it holds (a follow-up
  on the same sections, or small talk, leaves it).
  Whether the request that triggered it already carries the new rule depends
  on when Cursor reads its rules, which has not been verified on a live client;
  at worst the rule is one prompt behind.
- The Cursor mirror of `working-memory-write.sh` renders the branch's capsule
  into the same rule after the turn checkpoint, unless the rule holds the
  prompt hook's capsule for the same task: rebuilt from the task alone it would
  lose what was retrieved for the latest prompt, which is what the next request
  carries when Cursor reads its rules before the hook runs. The Cursor mirror of
  `local-context.sh` always renders it at session start, so a fresh session or
  a branch switch does not keep the previous session's render.
- Every render stands down when `CONTEXT_CAPSULE_DELIVERED=1`: the Harness has
  put the capsule into the prompt already.

The rule is replaced atomically only when a render succeeds and is ignored
local state (each edition's `.gitignore` lists it). These are declared
MIRROR_RULES transformations of the canonical hooks (the `_WM_DELIVERY_*` and
`_WM_PROMPT_*` constants in `memory-bank/scripts/context_retrieval.py`), not
drift: `scripts/build_mirrors.py --check` verifies them.

Cursor also runs the Claude Code hooks it finds (third-party hooks are on by
default and every matching hook from every source runs). The Claude copies of
`working-memory-read.sh` and `working-memory-write.sh` therefore stand down when
their payload carries `cursor_version` and the project has `.cursor/hooks.json`;
running both doubled every checkpoint and wrote retrievals nobody received.

Continuity is unaffected: `stop` runs at turn end without prompt access, so
the record is written exactly as on the other two clients. Retrieval on
Cursor can also be explicit - run `context.py retrieve` or the `memory`
command when a task needs prior context sharper than the rendered rule.

What the capsule is retrieved *for*: Claude Code, Codex and Cursor's prompt
hook pass the user's prompt, so the query is the request. Cursor's `stop` and
`sessionStart` renders have no prompt and supply the task identifier, which is
usually a branch name - and a branch name tokenized into a query asks memory
about the word `main`. The governed path therefore builds that query from the
task behind the identifier: its goal, manual progress, next steps and file stems,
the same enrichment the lightweight path already performed. The automatic
checkpoint is deliberately excluded (turn counts and a timestamp carry no
topic and would change the query on every flush), and so is an
auto-provisioned goal, which is the branch slug re-cased. When nothing
substantive remains, the bare identifier is used and the capsule says
`query: from branch name only` rather than `query: from task goal`; the
manifest records the same distinction as `query_source`.

Each automatic path also identifies its actual client to the runtime: Claude
Code passes `claude`, Codex passes `codex`, and Cursor passes `cursor`; a direct
CLI call defaults to `cli`. Retrieval manifest version 3 records that `host`
beside the `entry_point`. The repeat gate scopes its baseline to task, host, and
entry point, so one client's prompt hook cannot suppress another client's turn.

Treat the editions as equivalent in policy, skills, and enforcement; they
differ in how the capsule arrives - on stdout of the prompt hook for Claude
Code and Codex, through the rule the prompt hook rewrites for Cursor.

The installed scripts:

- report framework/tooling markers and context health at session start;
- block recognized destructive shell operations;
- warn about invalid task/spec Markdown names;
- detect repeated-edit loops.

Session hooks are metadata-only on Claude Code and Codex: they do not index
sources, retrieve context, print record bodies, or inject records into
prompts. The one documented exception is Cursor's `sessionStart`, which
additionally re-renders `.cursor/rules/working-memory.mdc` (see above) -
silently, to the file only, never into the session banner.

Hook return conventions are `0` to continue, `1` for a non-blocking warning
where supported, and `2` to block. Timeouts are seconds in both Cursor's
`hooks.json` and Claude Code's `settings.json`; Codex hooks carry no timeout
field. Preserve the native values and schemas when synchronizing hooks.

Claude Code delivers the tool-input JSON to a hook on stdin, so a hook command
is the script path itself. Wrapping it as `echo '$TOOL_INPUT' | <script>` feeds
the hook the literal string `$TOOL_INPUT`, which every validator treats as an
empty payload and passes.

Each host runs a hook command from a different directory, so each wiring file
anchors the script path its own way:

| Host | Command form | Why |
| --- | --- | --- |
| Claude Code | `"${CLAUDE_PROJECT_DIR}"/.claude/hooks/<script>.sh` | Hooks run in the session's current directory, which follows every `cd`; Claude Code exports the project root as `CLAUDE_PROJECT_DIR` and asks for the placeholder double-quoted in shell form. |
| Codex | `sh -c '...' sh <script>.sh` - a fixed launcher (spelled out in `.codex/hooks/README.md`) that walks up from the session's directory to the nearest directory holding `.codex/hooks.json` and execs that project's `.codex/hooks/<script>.sh` | Codex runs hooks in the session cwd through `$SHELL -lc` and exports no project-root variable. The walk works from a subdirectory, without Git, and for a project nested in a larger repository, and `sh` keeps it independent of the login shell. It stops at the project that declared the hook, so a missing script exits 127 instead of running a same-named script from an ancestor such as `~/.codex/hooks/`. |
| Cursor | `.cursor/hooks/<script>.sh` | Cursor runs project hooks from the project root. |

A bare relative path on Claude Code or Codex exits 127 as soon as the
session's directory is not the project root, and both hosts treat every exit
other than 2 as non-blocking, so a safety hook would stop guarding without an
error. `tests/test_hook_wiring.py` runs every wired command from a nested
directory to keep it that way.

## Claude Code Activation

1. Open the consuming project root, not the parent accelerator repository.
2. Confirm `AGENTS.md`, `.claude/CLAUDE.md`, `.claude/settings.json`,
   `.claude/commands/`, `.claude/agents/`, `.claude/skills/`, and executable
   hook scripts are present.
3. Run `/context` and confirm `AGENTS.md` is listed under **Memory files**.
   Claude Code reads `AGENTS.md` by itself only while the project has no
   `CLAUDE.md`, `.claude/CLAUDE.md` or `CLAUDE.local.md`; the shipped
   `.claude/CLAUDE.md` imports `@../AGENTS.md`, so the policy loads beside a
   project's own `CLAUDE.md` (Laravel Boost writes one) and on Claude Code
   versions that do not read `AGENTS.md` directly. An existing
   `.claude/CLAUDE.md` gets the import appended by `--merge-existing`.
4. Start a new Claude Code session. The session-start output should identify
   project/tooling markers and context validation status without printing
   record contents.
5. Type `/` and confirm installed commands such as `/verify`, `/memory`, and
   `/project-brain` are visible.
6. Inspect any permission or hook error rather than weakening the safety
   configuration globally.

Use `.claude/settings.local.json` for personal hooks or overrides that should
not be shared. Keep it uncommitted under the team's local-ignore policy.

## Cursor Activation and Double-Loading Prevention

1. Open the consuming project root in Cursor.
2. Confirm `AGENTS.md`, `.cursor/rules/`, `.cursor/skills/`,
   `.cursor/agents/`, `.cursor/commands/`, `.cursor/hooks.json`, and hook
   scripts are present.
3. Keep Cursor's optional Claude-file loading disabled when the self-contained
   `.cursor/` edition is installed. The control is in Cursor's Rules &
   Memories settings; labels can change between Cursor versions.
4. Start a new agent session and confirm one session-context report appears.
5. Type `/` and confirm each installed command appears once.

Duplicate commands/agents or hooks firing twice indicate that both native
`.cursor/` content and optional Claude-file loading are active. Disable the
optional Claude source and restart the session. Do not delete either edition
when the team genuinely uses both Cursor and Claude Code; prevent cross-loading
instead.

Cursor's file-naming validation runs after an edit and currently warns rather
than preventing the write. Treat the warning as a required correction even
though the file was already created.

## Codex Trust, Configuration, and Hooks

Codex loads project-scoped `.codex/config.toml` and hooks only for a trusted
project.

1. Open the repository with Codex and accept the project trust prompt. The
   shipped config also documents `/trust` for clients that expose that action.
2. Confirm `.codex/config.toml` contains:

   ```toml
   project_doc_max_bytes = 131072

   [features]
   hooks = true
   ```

   Codex reads the AGENTS.md chain only up to `project_doc_max_bytes` (32 KiB
   by default). A project's own AGENTS.md with the accelerator's policy block
   after it passes that, and the policy, coming last, is what gets cut. The
   Harness also passes a larger budget to a Codex launch whose `AGENTS.md`
   exceeds the default: its size plus 32 KiB, at least 128 KiB and at most
   256 KiB, and only for a regular file, never one reached through a link.

3. Confirm `.agents/skills/`, `.codex/hooks.json`, and executable hook scripts
   are present.
4. Open the skills menu or ask Codex to use a known skill such as `verify`,
   `memory`, or `project-brain`.
5. Review and trust the hooks in `/hooks`. Codex records trust against each
   hook definition's hash, so a hook whose command changed in an update is
   skipped until it is trusted again. The Harness does this for every project
   it serves: once per version of its clone it approves, in the user's Codex
   config, each hook definition that is exactly one the edition ships and runs
   a script byte-identical to the clone's - the accelerator's own code. A
   team's own hook or an edited script is left for review in `/hooks`; it
   rewrites an installed project's `.codex/hooks.json` to the current wiring
   only when it can approve it in the same step, and puts the old wiring back
   otherwise. `HARNESS_CODEX_HOOK_TRUST=0` turns the approval off.
6. Start a new session and confirm the metadata-only session hook runs.

The installer registers the bundled [local Memory MCP](MEMORY-INTEGRATION.md) for each selected client. Portable project launchers resolve the actual worktree; other servers/settings remain intact. Client trust and tool approval still follow the client's own settings. External MCP integrations remain opt-in.

Codex hook tool identifiers and input payload keys can vary by client version.
The shipped scripts read the documented command/path keys and fail open when a
key is absent, so a schema mismatch may become a no-op rather than blocking the
turn. After a Codex upgrade, test each safety hook with a benign representative
operation and inspect hook diagnostics before relying on enforcement.

## Optional External Companion: Batch Harness

Unattended multi-stage pipelines (nightly fleet review, mass migrations)
live outside the editions, in the repo-root `harness/` directory — a
LangGraph-based runner that drives headless host sessions
(`claude -p --output-format json`, `codex exec --json`) as workers. It has
its own venv and dependencies and is never shipped by the installer or
listed in the inventories. It
follows the same rule as MCP servers: **optional, external, opt-in per
team; the accelerator does not require, ship, or depend on it**, and the
shipped runtime stays standard-library-only.

Two properties make the companion composable rather than parallel
infrastructure: workers run in the project directory *without* `--bare`, so
the project's `.claude/` world — permission deny rules, the subagent gate,
skills, the SubagentStop observer — applies inside every worker; and all
durable state flows through the project's own `context.py` (task lifecycle,
capsule validation, the `msg-dispatch` journal), so interactive sessions
and unattended runs share one audit trail. LangGraph's checkpointer holds
only graph position. See `docs/AGENT-ORCHESTRATION-DESIGN.md` (Stage D)
for the decision record, including when a plain Agent SDK script is the
better tool.

## Version Caveats

- **Codex:** the edition follows the skill-based model used after custom
  prompts/slash commands were removed in Codex v0.117+. Older clients may not
  discover or hook the same way; upgrade the client rather than adding fake
  wrappers.
- **Cursor:** hook event names and timeout units differ from Claude Code, and
  settings labels can change. Keep `.cursor/hooks.json` in Cursor's schema and
  verify that optional Claude-file loading remains off.
- **Claude Code:** permissions and lifecycle hooks are owned by
  `.claude/settings.json`. Check the installed client against current Claude
  Code documentation before adopting new event types or permission syntax.
- **All tools:** a file being present does not prove that the client loaded it.
  Re-run the activation checks after client upgrades or integration changes.

## Optional Developer Tooling: Context Collection

`scripts/collect_context.py` packages a chosen slice of this repository into
one bundle for pasting into an external model — a code review in a chat
window, a second opinion on the memory core, a diff explained to a model that
cannot see the checkout. It wraps the
[`code2prompt`](https://code2prompt.dev/docs/how_to/cli/) CLI. Repository
contract tests pin the expected 4.3.0 argv and exclusion behavior; upstream
behavior must be manually re-verified before changing that pin. It follows the
same rule as MCP servers and the batch harness:
**optional, external, opt-in per developer; the accelerator does not require,
ship, or depend on it.** It saves zero runtime tokens — it spends the
maintainer's, to produce a bundle scoped on purpose.

```bash
./collect                                   # list the scopes
./collect skills --edition Laravel --dry-run
./collect core --edition Symfony            # -> .c2p/
./collect diff --base origin/main --stdout
```

`./collect` is a one-line `exec` wrapper at the repository root, so arguments
and the exit status pass through unchanged; `scripts/collect_context.py` is
executable and behaves identically. The wrapper carries no `.sh` extension, so
the `lint` job lists it by name alongside `*.sh` — see [CI](CI.md) — keeping
every tracked shell file inside `bash -n` and `shellcheck`.

Inside Claude Code the same thing is `/collect <scope> [options]`, from
`.claude/commands/collect.md` at the **repository root** — outside every
edition, so an installed accelerator never carries a command for a tool it
does not ship, and `build_mirrors.py`, which walks only the four edition
directories, never sees it. The command runs the script through bash
injection, caps the injected output, and instructs the model to report the
summary without opening the bundle: the bundle exists for a model that cannot
see the checkout, and reading it in the session that produced it would spend
exactly the context it was built to move elsewhere.

Scopes: `edition`, `skills`, `core`, `hooks` (all take `--edition`), plus
`tooling`, `docs`, `harness`, `diff` and `custom`. Bundles land in the
ignored `/.c2p/` alongside a manifest recording the exact patterns, counts
and binary version. Nothing is written without a scope, and a run whose
patterns match nothing fails loudly.

The wrapper exists because a bare CLI run was unsafe in *this* repository in
the measured 2026-08-08 snapshot. The counts below describe that snapshot and
code2prompt 4.3.0; they are regression context, not current repository totals:

| Guard | Unguarded behaviour on 4.3.0 |
|---|---|
| `.git` always excluded | The root with `--hidden` and no `-i` reads 2,213 files, `.git/config` (remote URLs) and `.git/COMMIT_EDITMSG` among them. |
| `Task/` excluded by default | `Laravel/Task/Epics/Epic-0*_*_SPEC.md` is named-client product specification. Collecting `Laravel/**/*.md` without the exclude adds 10 files and ~87k tokens of it. `--with-task` opts in and prints a warning. |
| Mirrors excluded by default | `.claude/`, `.cursor/` and `.codex/` are generated from canon by `build_mirrors.py`; including them triples a bundle for no added information. `--with-mirrors`, or the `hooks` scope, opts in. |
| Every run is config-isolated | A `.c2pconfig` in the working directory is auto-loaded and silently injects settings such as `line_numbers`, with no flag to disable it. Runs execute in an empty temporary directory with `XDG_CONFIG_HOME` pointed at it. |

Two upstream behaviours are worth knowing before writing custom patterns.
Patterns that match nothing exit 0 with an empty bundle and no warning — the
wrapper turns that into an error. And `code2prompt`'s `*` **crosses directory
separators**: `-i "*.md"` at the repository root reads 419 files, not the four
at the top level, and `-i "CHANGELOG.md"` matches five, one per edition. Only
a `./`-prefixed literal anchors. Scopes declare top-level wants separately and
the wrapper expands them with Python's own non-recursive glob.

The snapshot token counts came from cl100k (OpenAI BPE, the tokenizer
code2prompt 4.3.0 carried). **They are not counts of Claude tokens** — they are
relative comparison units, not bill predictions. Current upstream tokenizer
and API behavior are manual-verification claims. An exact Anthropic count may
require a current vendor-supported counting API, network access, and an API key,
which is a separate decision under [Security](SECURITY.md).

Not adopted: the `code2prompt-mcp` server and the `code2prompt-rs` Python SDK.
The MCP server is recorded as a decision rather than a silence — the shipped
Codex configuration deliberately requires no MCP server (see above and
[Security](SECURITY.md)). The 2026-08-08 evaluation found the server surface
poorer than this wrapper and the Python SDK unsuitable for the repository;
re-check those upstream surfaces manually before relying on that comparison.
The CLI is never a blocking CI gate: CI has no Rust toolchain, so
`tests/test_collect_context.py` skips its live checks and runs its contract
checks — including `test_containment`, which fails if `code2prompt` is ever
referenced from an edition or the installer.

## Shared Context Activation

Tool activation and context activation are separate. From the project root,
after confirming `memory-bank/local/` is ignored:

```bash
python3 memory-bank/scripts/validate.py
python3 project-brain/scripts/validate.py --root .
python3 memory-bank/scripts/context.py index
```

The default is governed mode. Read [Context Modes](CONTEXT-MODES.md) before
selecting lightweight mode, and use the
[User Task Workflow Example](examples/USER-TASK-WORKFLOW-EXAMPLE.md) for the
end-to-end lifecycle.
