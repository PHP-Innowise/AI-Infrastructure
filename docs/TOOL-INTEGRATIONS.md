# Tool Integrations

Each ready-made edition supports Claude Code, Cursor, and OpenAI Codex in the
same project. The tools share root policy and project context, but each keeps
its native discovery model. Do not make one tool load another tool's adapters.

## Directory Ownership

| Path | Owner | Contents |
| --- | --- | --- |
| `AGENTS.md` | Shared | Enforceable project/stack policy used across tools |
| `.agents/skills/` | Canonical skill edition | Skill workflows declared canonical by `project-brain/config/runtime.json`; Codex discovers them directly |
| `.claude/` | Claude Code | Commands, agent wrappers, skill mirrors, `settings.json`, hooks, and reference documents |
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
| Task Capsule into the prompt | `UserPromptSubmit` | `stop`/`sessionStart` render an `alwaysApply` rule | `UserPromptSubmit` |
| Turn checkpoint at turn end | `Stop` | `stop` | `Stop` |

## Automatic Memory Support by Tool

Automatic memory has two halves, and one of them depends on a client
capability that is not universal.

| | Claude Code | Codex | Cursor |
| --- | --- | --- | --- |
| Capsule retrieved into each prompt | yes (fresh) | yes (fresh) | **yes, one turn stale** (rendered `alwaysApply` rule) |
| Turn change set buffered and flushed | yes | yes | yes |
| Explicit `context.py retrieve` / `memory` | yes | yes | yes |

Cursor cannot receive the capsule at prompt time. Its nearest event,
`beforeSubmitPrompt`, returns `{"continue": true|false, "user_message": "..."}`
- it can allow or block a submission but cannot add context to the prompt, so
a hook that printed a capsule would produce output the client discards. This
is a client capability limit, not an installation fault, and
`working-memory-read.sh` is therefore not shipped in `.cursor/hooks/`.

The read path on Cursor is served through a rule file instead: the Cursor
mirrors of `working-memory-write.sh` (after the turn checkpoint) and
`local-context.sh` (at session start, so a fresh session or a branch switch
never serves the previous session's capsule) render the freshest capsule into
`.cursor/rules/working-memory.mdc` - an `alwaysApply` rule Cursor attaches to
every prompt. The file states its own staleness ("as of end of previous
turn"), is replaced atomically and only when a fresh render succeeds, and is
ignored local state (each edition's `.gitignore` lists it). This is a
declared MIRROR_RULES transformation of the canonical hooks (the
`_WM_DELIVERY_*` constants in `memory-bank/scripts/context_retrieval.py`),
not drift: `scripts/build_mirrors.py --check` verifies it.

Continuity is unaffected: `stop` runs at turn end without prompt access, so
the record is written exactly as on the other two clients. Retrieval on
Cursor can also be explicit - run `context.py retrieve` or the `memory`
command when a task needs prior context sharper than the rendered rule.

Treat the editions as equivalent in policy, skills, and enforcement, and as
differing only in capsule freshness: Claude Code and Codex retrieve
per-prompt, Cursor reads the rule rendered at the previous turn boundary.

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
is the bare script path. Wrapping it as `echo '$TOOL_INPUT' | <script>` feeds
the hook the literal string `$TOOL_INPUT`, which every validator treats as an
empty payload and passes.

## Claude Code Activation

1. Open the consuming project root, not the parent accelerator repository.
2. Confirm `AGENTS.md`, `.claude/settings.json`, `.claude/commands/`,
   `.claude/agents/`, `.claude/skills/`, and executable hook scripts are
   present.
3. Start a new Claude Code session. The session-start output should identify
   project/tooling markers and context validation status without printing
   record contents.
4. Type `/` and confirm installed commands such as `/verify`, `/memory`, and
   `/project-brain` are visible.
5. Inspect any permission or hook error rather than weakening the safety
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
   [features]
   hooks = true
   ```

3. Confirm `.agents/skills/`, `.codex/hooks.json`, and executable hook scripts
   are present.
4. Open the skills menu or ask Codex to use a known skill such as `verify`,
   `memory`, or `project-brain`.
5. Start a new session and confirm the metadata-only session hook runs.

The shipped config does not require an MCP server. Do not add the commented
MCP example unless a real workflow needs that server and the team has reviewed
its command, credentials, and data boundary.

Codex hook tool identifiers and input payload keys can vary by client version.
The shipped scripts read the documented command/path keys and fail open when a
key is absent, so a schema mismatch may become a no-op rather than blocking the
turn. After a Codex upgrade, test each safety hook with a benign representative
operation and inspect hook diagnostics before relying on enforcement.

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
