# Shared memory in Claude Code, Codex and Cursor

Harness and direct clients use the consuming project's existing `project-brain/`
and `memory-bank/`. Install the accelerator in the actual worktree. SQLite and
local receipts are disposable; shared records/chunks remain authoritative.
Memory MCP needs only Python 3.9+ and SQLite FTS5, with no external service or key.

## Harness automatic sessions

Each technical message receives a bounded capsule. Automatic hooks and Harness
cut recognized secrets, email/phone/customer-identifier spans and transcript
role prefixes (`user:`, `stderr:`) out of the message before constructing memory
queries or automatic task goals; the words around them stay, because pasted logs
and addresses are ordinary in real prompts. A private key goes with its body.
For labelled names/addresses the adapter keeps only preceding text, since later
delimiters may belong to the value. A message with nothing technical left skips
retrieval rather than falling back to the task goal or branch name. The original
user message still goes to the native agent. Direct CLI/MCP queries and shared
writes keep strict validation. Hooks and Harness deliver the same rendered text:
selected excerpts within 3,600 characters (`capsule_text` in `refresh --json`).

If the final reply has no parseable
`memory-draft`, Harness makes one internal draft-only follow-up in the same native
session, in plan/read-only mode, without helpers or a new user message. Its limit
is 30 seconds within the original time/token/USD budget. Missing native identity
or unmeasurable remaining usage produces an explicit skip. Usage, instruction
character counts and the context fill include recovery. `CONTEXT_MEMORY_RECOVERY=1` suppresses memory
hooks. Recovery failure preserves completed work; cancellation keeps cancelled
status and prevents subsequent writes. An already launched write may finish.
Reviewed sessions/native Claude commands keep their flow.

Optional `used_memory` lists up to ten delivered paths whose claims the agent
checked and used. The notice puts the delivered items beside those the run's own
tools were seen to open (a Read that returned the file or any tool's copy of a
skill, a Skill load; *at least* that many when shell commands also ran, *unknown*
when they were the only way the run read, as for Codex) and beside **agent-reported
use**; neither proves a claim was used. Telemetry stores counts, not source
bodies/path lists.

## Automatic project registration

The accelerator installer and generator register Memory MCP for each selected
client. No separate `claude mcp add` command or manual config entry is needed.
The installer checks Python 3.9+ and SQLite FTS5 on the client PATH, including
`python` / `py -3` on native Windows, and preserves other servers/settings.

- Claude Code: `.mcp.json` uses a Python launcher anchored by the server-process
  `CLAUDE_PROJECT_DIR`, with a nearest-project-marker fallback.
- Codex: `.codex/config.toml` carries one managed `mcp_servers.harness_memory`
  table. The Python launcher finds the nearest declaring config from the
  session directory, including nested directories and worktrees.
- Cursor: `.cursor/mcp.json` uses `${workspaceFolder}`.

Launchers stop at their first project marker. Missing runtime there fails rather
than using an ancestor project's memory. Paths are portable across clones and
worktrees; registration never writes global client configuration.

A foreign server using the memory name, malformed configuration, symlinks and
edited managed Codex blocks block registration rather than overwriting them.
On Python 3.9/3.10, Codex merges validate a conservative TOML subset and refuse
unsupported syntax. Run registration with Python 3.11+ for the full TOML parser.
Dry runs and collision preflight write no project files. Harness setup previews
these merges while omitting other servers' configuration values.

For an already installed project after updating the runtime, the same helper can
repair registration in one command (select only clients used by that project):

```bash
python3 memory-bank/scripts/mcp_config.py --root . --tool claude --tool codex --tool cursor --dry-run
python3 memory-bank/scripts/mcp_config.py --root . --tool claude --tool codex --tool cursor
```

Native clients still control workspace trust and MCP tool approval. Accept project
MCP in Claude's `/mcp`, trust the Codex project, and enable the Cursor project MCP.
Allow the four memory tools through the client's supported approval settings when
automatic calls are desired. Configuration alone is not a grant of trust, and
registration does not force the agent to use every retrieved claim.
See [Claude MCP](https://code.claude.com/docs/en/mcp),
[Codex MCP](https://learn.chatgpt.com/docs/extend/mcp?surface=cli), and
[Cursor MCP](https://cursor.com/docs/mcp).

## Four tools

Cursor's installed hooks use a persistent-rule delivery profile. Short replies
such as `yes do it`, `да`, and `fix it` may use the last safe substantive query
for the same current project, named branch, task generation and policy. The
ignored SQLite row stores only that query and expires after one day; every
delivery refreshes and verifies current sources. A new explicit goal wins over
the old query. Changed scope, redaction, incomplete refresh or unknown ownership
withholds the query rather than reusing old facts. The first automatic checkpoint
can transfer it only to the task that checkpoint actually created.

Routine checkpoint/promotion counts do not rewrite the Cursor rule. Current
progress, next steps, source changes and failures still change its substantive
content. The ordinary CLI, Harness, Claude and Codex delivery profiles retain
their existing history and repeat behavior. This is local delivery behavior;
it does not establish model quality or provider-token savings.

- `memory_status`: check installed memory health.
- `memory_retrieve`: pass the current `task_id` and a concise module/class/ticket
  `query` before complex work, after compaction/scope changes and before uncovered
  decisions. Optional `paths` link canonical sources. Verify cited claims. The
  usual privacy/freshness/budget limits apply, with ignored local manifests.
  A new task receives a warming capsule without an authoritative write.
- `memory_checkpoint`: reuse the same task ID; supply `goal` when it is new.
  It provisions/binds that task even before any edit, flushes Git metadata and
  returns the authoritative numeric revision.
- `memory_record_result`: pass the current `revision`, a stable `result_id`,
  sanitized `progress`, up to three `next_steps`, and up to three reusable
  findings/decisions. Each learning needs `type`, `title`, `consequence` and
  1–10 canonical project `sources`. Set `verified: true` after checking claims.
  Records explicitly say **agent-attested, not human-reviewed**: the verification's
  ledger entry carries `[attestation:agent]`, a chunk promoted from it is tagged
  `agent-attested`, and the capsule marks such knowledge where it delivers it.
  Empty learnings is valid; explicit `next_steps: []` clears finished steps.

An automatic promotion attempt schedules its own retry before doing work.
If it fails or is interrupted, the next governed turn after the 30-second
delay retries one record, even when Git is clean. Owner-scoped metadata lives
under ignored `project-brain/local/`; it contains a timestamp and a record UUID,
never learning text. Attempts rotate so one failing source cannot starve other
records. Disabled promotion and lightweight mode remain inactive. A successful
scoped save cannot discard another record's pending work. Scheduling must be
writable before a background retry scans sources; explicit promotion can still
save records when this optional local state is unavailable. These retries need
subsequent turns; they are not a background timer. Older failures without a
marker are recovered by the existing normal flush or exact result replay.

Automatic apply rechecks the configured near-duplicate rule for stalled proposals as well as fresh ones. A chunk left by an interrupted apply remains protected from a later duplicate under a different date. The failed promotion names the conflicting chunk; it does not adopt, rewrite or delete that chunk. A valid revision can still supersede its recorded predecessor. This guard does not repair an interrupted index/proposal transaction or promise general crash convergence.

Use `CONTEXT_TASK_ID`, otherwise the branch task. During a rebase that is the
branch being rebased: the prompt hook reads its memory and the Stop hook
checkpoints that same task while the rebase runs. In a worktree Codex made, on
a detached HEAD, it is `codex-thread/<thread ID>`, which the capsule names
([A worktree Codex made](#a-worktree-codex-made)).
On any other detached HEAD, use an explicit existing task ID. Do not create
another task for the same work. Saving runs promotion
under the existing setting. A learning's record is named by its content, so a retry,
or a later result restating it, finds that record instead of writing a second one.
An exact completed result replays passively and preserves later edits. A
metadata-only fence in `project-brain/control/result-attempts/<task UUID>/` is
saved before record changes and completed before the disposable local receipt;
its file is named by task, result ID and request content, so another clone that
reuses a result ID for other content makes its own save and the clones' files
merge without a conflict. Losing only the receipt still permits exact replay. A
started attempt continues at a positively observed, unchanged frontier: task
UUID/revision, record byte images, current cited source fingerprints,
policy/actor/runtime scope and bounded native record/index/handoff domain must
match. Any other interrupted attempt with the same result ID runs again from the
start, every effect idempotent against the current records: a learning is
created only when absent, finished only when it is exactly this save's open
record, revised only under the usual proofs, and the task takes only progress it
does not already hold. No arbitrary rollback or power-loss transaction is
promised. Preserve these operation proofs with the Brain state; the installer
never copies another project's proofs. A finished save keeps only a small proof;
the newest 64 per task are kept, and compaction removes an archived task's folder
and the `control/result-learning-images/` entries of archived learnings.

Existing learnings outside the public/team save privacy scope are held without
returning their current title, UUID, status or revision. Only normalized incoming
learning fields and a fixed held reason are returned, including archived and
completed replay paths. Promotion still runs normally; inaccessible, missing or
ambiguous record identities are omitted from its response diagnostics with
`details_withheld=true`. This is output filtering, not deletion or skipped work.

CLI `--revision auto` reads the current task revision under the mutation lock
for a fresh request, including tasks already updated by hooks. It does not rebase
an ambiguous existing attempt. A numeric stale task CAS still rejects the new
operation before effects. MCP requires the numeric revision from checkpoint.
Rewording an agent's own closed/verified learning can update automatically only
when its exact UUID/revision/raw bytes match the runtime's last saved image.
Manual changes invalidate that proof even if an older agent attestation remains.
Deliberate correction may supply the original positive learning
`expected_revision`; never invent or reload one to conceal intervening changes.
Person-attested, archived, wrong-kind, unauthorized and conflicting learning
records stay intact and are reported individually with current revision/reason.
Independent progress, next steps and other valid facts still save. Content/path
privacy, result-ID/content identity and task authorization remain hard guards.
Production retrieval remains enabled. CLI example: `python3
memory-bank/scripts/context.py record-result --task-id ID --result-id RID
--revision auto --input result.json`. Completion remains explicit.

The native CLI/MCP accepts exact receipted replay after task archival or citation
deletion. Current task ownership/privacy and static content/path checks still
apply; the replay leaves later corrections and removed records alone. Current
source checks still govern every new result and any promotion retry. Reused
ordinary aliases select the active task; ambiguous UUID-shaped aliases are
refused. Use the task UUID to name an archived task unambiguously. CLI fresh-task or proven replay `auto` resolution occurs under the mutation lock; MCP
requires a positive integer revision. For an original live automatic save with a canonical linked UUID,
Harness now calls `prepare-result`, which validates content/privacy and returns a
versioned safe wire request. An insert-only private table binds that request and
its original omissions to the session, full generation, originating event and
runtime/actor/policy scope before the first mutation. A contender or repeat uses
`replay-result`: exact receipt proof and current authorization are required before
any record or promotion writes. Source deletion and archival do not regenerate
the request. A started attempt with the same digest, owner and attestation is
finished by the replay, whatever stopped it: each effect is idempotent against
the current records. Missing/corrupt proof and changed input/scope remain
incomplete; they never fall back to an ordinary write or the legacy chain.

The binding body is separate from public session/launch rows and cost metadata;
the original provider draft remains in its existing conversation event. Rejected
candidates may pass through the existing short-lived private request file, which
is cleaned on success and failure, but are never retained in the immutable table.
Bindings remain for the lifetime of the private session database. Each binding
has a 128 KiB admission limit; there is no lifetime row-count cap or unresolved-row eviction.
No startup or post-finalization replay is authorized by a persisted binding.
Owned automatic alias-only links resolve and bind their current canonical task
UUID before saving; the original UUID and alias selector remain bound on every repeat. Generic stale
errors never authorize refresh/rebase after an attempt may have started. Manual
saves and unsupported old runtimes do not acquire historical recovery permission. This is persistence correctness, not evidence of provider-token savings.

Without MCP use the installed context CLI and Project Brain protocol. `/memory`
refreshes layers; `/checkpoint` saves working continuity; reusable knowledge needs
source-backed findings/decisions. This does not authorize unrelated skill chaining
or writes outside a read-only scope.

## Verify

Confirm all four tools appear in each configured client. Retrieve for a fresh task,
checkpoint with a goal, record a source-backed result, then replay the same ID/content
and confirm no duplicates. Inspect Harness **Knowledge → Memory use**, run
`context.py validate --json` and `memory-bank/scripts/validate.py`. Distinguish real
client connections from isolated protocol tests.

When Claude Code or Codex updates, run the host benches before the next rollout; they
drive the installed binary offline against a scripted model (`scripts/README.md`):

```bash
python3 scripts/claude_code_bench.py                     # each --mode too: acceptEdits, dontAsk, auto, bypassPermissions, plan
python3 scripts/claude_code_bench.py --queued 8          # a question typed while the save is written, in the terminal
python3 scripts/claude_code_bench.py --attached          # as the Harness launches it
python3 scripts/codex_bench.py                           # and --attached, --sandbox read-only
python3 scripts/codex_bench.py --queued 8
python3 scripts/codex_bench.py --worktree                # and beside, --subfolder src, --app-server, --queued 8
python3 scripts/codex_bench.py --worktree codex          # one Codex makes, detached; and --app-server, --subfolder src
```

Then `--target DIR --sync` on a copy of each client project, and the rollout rehearsal.

`scripts/memory_lifecycle_fuzz.py --seed N` drives random lifecycles through a fresh installation's hooks - sessions of every host in the root, below it and in worktrees, a session entering a worktree, bursts of sessions answering at once, branches, commits and worktrees changing under them - and checks that every save lands in its branch's task and nothing is written into a worktree; `--attached` launches every session as the Harness does and checks that neither the project nor the clone is written into. CI runs fixed seeds of both.

## Automatic results in native sessions

Governed Codex, Claude Code and Cursor sessions now use one bounded same-agent Stop continuation to record the result of the work already performed. The prompt hook arms ignored metadata only; the normal Git checkpoint runs first at Stop. The continuation calls `context.py native-result` with progress, optional next steps and at most three source-backed findings/decisions. `verified` is a top-level boolean and means agent attestation, never human review. Empty learnings and an explicit skip are valid. No prompt, final prose or transcript is parsed into authoritative knowledge.

A read-only research result can provision its task after validation without inventing a Git delta. The challenge pins project, actor, host/session, task UUID/revision, checkout and runtime/config. A new user prompt retires the prior generation; the native internal continuation preserves it. Cursor sends that continuation back through its prompt hook as a prompt. The hook arms (which journals it as `continuation`) and exits before the refresh: the turn's own prompt already brought the capsule, and Cursor, which reads its rule before the hook runs, keeps the rule the last real prompt wrote for the person's next request. Codex 0.163 adds the continuation to the same turn as a `<hook_prompt>` item and runs no prompt hook for it; the same guard in its prompt hook, and `refresh --host codex` (and `--host claude`) building no capsule for such a query, stay as defence for a host that does send it as a prompt. Duplicate Stops request no additional continuation; exact repeated saves reuse existing result fencing. The recovery Stop does not checkpoint the same user work again. Completion still requires the explicit verified workflow.

A session whose save command Claude Code refused is not asked again for a day. Claude Code records the refusal on the command's result in the transcript (`toolDenialKind` `user-rejected`, `permission-rule` or `automode-blocked`; `interrupted`, `cancelled` and auto mode's classifier failing are not refusals), and the recovery Stop reads it from the transcript's last 512 KiB. A person who answers No in Claude Code's dialog ends the turn before any Stop runs, and a refusal made just before the recovery Stop may not be on disk yet (Claude Code writes its transcript in batches), so the next prompt reads it there too. A refusal comes back after every answer: a person said no, a rule or auto mode denied the command, or `claude -p` ran in a project whose trust was never accepted, whose allow rules Claude Code then ignores. Each of those answers cost two model calls. The first prompt after the refusal prints the usual "not saved completely" warning; later ones arm nothing (`prompt` `skipped` `host-refused`). A new session is asked again.

A turn that ends in plan mode (`permission_mode: plan`, which Claude Code and Codex send) is not asked: the host tells the agent to change nothing, overriding any other instruction, so the agent would refuse after every answer or write against the person's choice. A save asked before the switch is still settled. Codex's `permission_mode` follows its approval policy only, so for Codex the Stop reads the current turn's `turn_context` from the end of the rollout: Plan collaboration mode is skipped the same way, and a read-only sandbox, in which no save can write the project, as `read-only-sandbox`. A workspace-write sandbox whose restricted file-system policy does not let a command write the project's Brain - Codex started in a subdirectory of the project writes only there - still gets the save step when it writes a temporary directory, as Codex's does (`/tmp` and `$TMPDIR`): the save command finds the project read-only, checks the request and the result as a save does, so a malformed one is refused while the agent can correct it, and leaves the result in a folder of this user's there (`ai-accelerator-native-spool-<uid>`). Such a sandbox cannot write the project's journal either, so its `save` `spooled` line is lost; the session's next hook - the continuation's Stop, or the next prompt - runs outside the sandbox, saves the result and journals it (`saved` with code `spooled`); the file goes first, so a result is tried once. A sandbox that writes neither is skipped as `outside-sandbox`. The most specific entry over a path decides, as it decides Codex's own read-only carve-outs. A policy that, read this way, would not let the turn write its own working directory is in a form the runtime does not know, and the save is asked for as before. A turn whose context it cannot find is asked as before. Each host gets the step its own way. Codex gets a Stop `decision: "block"`, whose reason it adds to the turn as a `<hook_prompt>` item, XML-escaped (the command's `&&` reaches the model as `&amp;&amp;`, which models read as `&&`). Cursor gets a `followup_message`. Claude Code gets `hookSpecificOutput.additionalContext` (Claude Code 2.1.163 or later; an earlier one is not asked). A block would show in its transcript as a hook error, raise "Stop hook error occurred" after every answer and reach the model twice. Claude Code's command is `cd <root> && echo -E '<JSON>' | python3 memory-bank/scripts/context.py … --input - --json`, the agent writing the JSON in place of `<JSON>`:

- No subshell or `{ }` group: before such a command Claude Code asks, whatever the allow rules say. It asked after every answer.
- No heredoc: Claude Code also asks before one whose body holds braces and quotes.
- No `printf`: Claude Code asks before a `printf` operand holding `[` with `$` or a backtick, which a JSON with code in it has.
- `echo -E`, not plain `echo`: Claude Code's Bash tool runs zsh unless `$SHELL` is bash, and zsh's plain `echo` turns the JSON's `\n` and `\\` into characters.

The `cd` into the project, `echo` and the allowed CLI each pass without a prompt. Claude Code 2.1.278, driven offline by a scripted model, showed all of this; `scripts/claude_code_bench.py` repeats it against the installed Claude Code, and `scripts/codex_bench.py` does the same for Codex.

Conflicting learnings with the same type, normalized title and source set are refused before task provisioning or result digest binding. The first payload can then be corrected within the same continuation. Exact duplicate learnings remain valid and create one record. A save that genuinely began still requires its original payload and canonical evidence; this preflight correction does not authorize rebasing or changing a begun result.

An exact save that already began can retry through the canonical result protocol even after the recovery Stop marked it failed. Its original UUID, revision, content digest and prepared scope remain bound. A completed replay needs no live citation or active alias, so archival or alias reuse cannot redirect it. Losing the disposable receipt cache leaves shared completion evidence usable. Without canonical receipt/attempt evidence, a retry is refused rather than becoming a first write. Incomplete saves still require current safe sources and ordinary ownership/CAS checks; fresh saves validate sources before provisioning a task. This recovery adds no automatic continuation.

Challenges live under ignored `project-brain/local/native-results/`, contain metadata only, allow save for 24 hours and retain at most 128 sessions. Missing/invalid state is a refusal, never permission to overwrite. A full save proof without its outcome metadata remains held because it cannot prove every learning was accepted. Failed/held capture is surfaced by the next capsule, including Cursor's JSON-safe rule path. Cursor sets `loop_limit: 1`; aborted/error stops do not capture. Harness-delivered, internal-recovery and lightweight sessions keep their existing behavior.

A challenge file is rewritten every turn, so it cannot say how many saves were requested, made or refused. Each step therefore appends one line to ignored `memory-bank/local/native-results.ndjson`: `at`, `host`, `stage` (`prompt`, `stop`, `save`, `stop-hook`), `event` and, where one applies, a fixed `code` - never the payload, the result, a path, a task or a session.

| Stage | Events and codes |
|---|---|
| `prompt` | `armed` (code `previous-requested`/`-failed`/`-held`/`-refused` when the last request was not saved completely, the warning the hook prints; `previous-unreadable` when this runtime could not read the session's last request - another version of it wrote it, or the file is damaged - and replaced it, without a warning, since whether it was saved is unknown; `previous-in-flight` when the last request, asked less than ten minutes before, was moved aside because the prompt came while its save was being written), `continuation`, `skipped` (`not-governed`, `capsule-delivered`, `recovery-session`, `no-session-id`, `host-refused`), `error` |
| `stop` | `requested` with `answer` `brief` or `repeat` (code `native-result:invalid-answer-setting` when that setting is unknown); `failed` on the recovery Stop (`unsaved`: no save began; `incomplete`: one began and did not finish; `host-refused`: Claude Code refused the save command); `held` (`unconfirmed`: a finished save the challenge never took); `skipped` (the same four, `not-completed`, `not-stop-event`, `not-armed`, `plan-mode`, `read-only-sandbox`, `outside-sandbox`, `task-completed`/`-cancelled`, `recovery-<phase>`, `phase-<phase>`); `error` |
| `save` | `saved` and `held` with `learnings` written and `promoted` chunks (code `repeat` for an exact repeat of a finished save, `late` for a begun save finished after the recovery Stop marked it failed, `in-flight` for a save that landed after a prompt moved its request aside, `spooled` for one a hook made from the spool, `rebased` for one recorded at a task another session created or moved since the request); `omitted` (`in-flight` and `spooled` likewise); `spooled` (the agent's sandbox could not write the project; the result waits for the session's next hook - a line written only where that sandbox can write the journal, which Codex's below the project root cannot); `refused` with the refusal's machine code; `error` with the exception type |
| `stop-hook` | `exited` with the save request's `status` (124: killed at the budget), or `skipped` with `capsule-delivered` |

A refused result keeps the text the agent corrects and carries a fixed code (`ResultRefused.code`). That text says what to correct: an input that is not one JSON object is refused as `native-result:invalid-input` with the JSON error, its line and column, and, for a raw newline in a string, the hint to write it as `\n` and to pipe with `echo -E` (a plain `echo` in zsh turns `\n` into a line break). A learning with missing or unexpected fields names them, and `verified` inside a learning is called a top-level boolean. A save another session of the branch overtook - its save or its turn checkpoint moved the task's revision, or created the branch task, after this one was requested - is recorded at the task as it now is when it comes within ten minutes of its request (see below). One whose task was replaced meanwhile (completed and started again) is refused as `native-result:task-changed`; one made later than that, spooled until a hook long after, whose task moved on or was created meanwhile, as `memory-result-v1:prepared_stale_no_write` or `native-result:task-changed`, and so is a begun save whose task moved on before it finished. Each says that nothing was written and that the agent reports the held save instead of retrying it. The codes: `memory-result-v1:schema`, `learning_fields` (often `verified` put inside a learning), `learning_type`, `no_sources`, `source_path`, `source_missing`, `conflicting_learnings`, `unverified`, `nothing`, `text_limit`, `sensitive`, `raw_text`, `task_id`, `result_id`, `revision`, `stale_revision`, `result_id_reused`, `task_closed`, `task_privacy`, `not_authorized`, `record_not_found`; the fence codes it already had (`scope_changed`, `prepared_stale_no_write`, `receipt_unavailable`, `attempt_*`); a `native-result:*` code; or `unclassified` for a Project Brain refusal that has none. The Stop hook writes its own line because the case worth counting is the one the runtime never finishes. The journal follows `memory-bank/local` as the runtime's other local files do, is never written through a linked file, and keeps eight lines per `refresh_health_retention` record. `scripts/memory_eval.py realized` prints it per host (see `docs/MEMORY-EVAL.md`).

Hooks need Python and a trusted/enabled native hook configuration. The integration adds one internal model continuation when a native result has not yet been submitted; its cost is not assumed to be free or offset by savings. Live-host acceptance and task/cost evaluation are separate checks.

That continuation follows every answer. On 21 Codex runs with one native save each (small tasks, 16-19 October 2026), it took a median of 10 seconds, 383 output tokens and 59k input tokens, most of them cached: about half of the answer's own time and tokens. A team that does not want it on every turn sets `"native_result_request": "off"` in `project-brain/config/runtime.json` (or `CONTEXT_NATIVE_RESULT_REQUEST=off` for one session). No prompt is then armed and no Stop asks; results reach memory only through `context.py record-result`, the MCP `memory_record_result` tool or the Harness, and the journal records each skipped prompt and Stop as `switched-off`. The default, `every-turn`, keeps the request after every answer. Any other value refuses the prompt's arming (`native-result:invalid-request-setting` in the journal). `--sync` never changes `runtime.json`, so the setting survives updates.

How the continuation ends depends on who reads the session. A headless run (`codex exec`, `claude -p`) hands its caller only the final message, so there the agent returns its answer again after the save. In a session a person reads as it goes, the answer is already on screen above the continuation, and the repeat doubled it: on nine Codex runs the message after the save was 93 % of the answer's length, against 21 % before the repeat was asked for. Under `"native_result_answer": "auto"`, the default, a session its host marks as attended ends with no further text or one short line:

- Claude Code: `CLAUDE_CODE_SESSION_ATTENDED=1` and an entrypoint of `cli`, `claude-vscode` or `claude-desktop`. A `claude -p` started from such a session inherits the entrypoint but is a child session, which Claude Code never marks attended.
- Codex: the rollout's first line names the session's source as `vscode` (the app, the IDE extension and the terminal UI) or `cli`; `exec` and subagents repeat.

Every other session repeats, Cursor's included. `repeat` always repeats and `brief` never does; `CONTEXT_NATIVE_RESULT_ANSWER` sets it for one session. An unknown value repeats, and the journal's `requested` line carries `native-result:invalid-answer-setting`: the save itself is asked for either way.

The step carries its rules - the JSON fields, the schema example, what belongs in learnings - in full only while the conversation lacks them. Every answer used to carry about 2,600 characters, and Claude Code's terminal showed them whole each time. The Stop reads the transcript's last 2 MiB: when a full step is there after the last compaction record, the step carries its commands, its ending and a pointer back to those rules, about 900 characters less. After a compaction, or without a readable transcript, the full step comes back. Cursor gets the full step every time: how its transcript records a summarized conversation could not be checked. `"native_result_instruction": "full"` in `runtime.json` (or `CONTEXT_NATIVE_RESULT_INSTRUCTION=full`) sends it every time; an unknown value does too.

The save takes two more model calls, and Claude Code counts them against `--max-turns` (`maxTurns` in the Agent SDK). A headless run whose answer comes in its last two allowed calls ends with `error_max_turns` and no `result`; the Stop hook is told neither the limit nor the count. Give such a run two more turns, or set `CONTEXT_NATIVE_RESULT_REQUEST=off` for it (`docs/TROUBLESHOOTING.md`).

### Decisions and known limits of the native save

- **A save another session overtook is recorded where the task now is.** A request pins the task's revision when the Stop asks for the save, not when the prompt was read, and the save was refused when another writer had moved it in the seconds between: two sessions of one branch kept the first save. Pinning at the prompt would have refused every save of a session that ran beside another one. The other writer was often not a save at all: every fifth turn of a checkout, a Stop hook flushes the turns into the task as a checkpoint, which moves the revision too. The refusal dropped the save's learnings with its progress and had the agent report a held save to a person who had already read the answer, while a save a moment later replaced the other's progress anyway. Nothing else writes under the save's lock, so a save made within ten minutes of its request - the window in which a prompt sent meanwhile treats it as still being written - now takes the task's current revision there, as the Harness retries its own: its progress and next steps replace the other session's, and its learnings are recorded as any are (the same learning from both sessions is one record; a different consequence for it is held, as always). A task created by the other session since the request takes the save the same way. A save made later, spooled until a hook long after, still yields to what was written since, and a task replaced in between - completed and started again - is another task's and refuses it. The request is also pinned to the branch it was asked on, and no longer to its commit: a commit made while the agent wrote the save - the person's, or another session's on the branch - refused it as `native-result:scope-changed`. A branch switched in between still refuses it, and so does any move of a detached HEAD, as in the middle of a rebase. On the four client projects, two sessions worked in one directory in the same minute in 2 to 4 % of the minutes of work. `scripts/memory_lifecycle_fuzz.py`, which drives sessions of one branch through prompts, Stops and saves at once, lost saves this way on the runtime before the change and none on 24 seeds after it.
- **The Stop request is the save; the policy asks for an explicit one only where none comes.** `AGENTS.md` used to require an explicit save (`context.py record-result`, `memory_record_result`, Harness `memory-draft`) after all authorized work, and the Memory MCP server asked for `memory_record_result` before finishing. An agent that followed it saved every answer twice, once itself and once when the Stop asked: on a synced copy of a client project, gpt-6.1-sol on Codex 0.163 ran `record-result` in 12 of 12 answers and a closing `context.py update` in 7, and the native save that followed carried `learnings: []` every time. The save rule now opens with the prohibition - no `record-result`, `memory_record_result` or closing `update` where Stop requests the save - and the Stop bullet names where that is: governed Claude Code, Codex and Cursor sessions running the hooks, outside the Harness, unless `native_result_request` is `off`. The stage-boundary `update` rule leaves out an answer's end that Stop's request saves. Everywhere else the explicit save is still required, because nothing else would save the answer: a Harness session saves its `memory-draft` block, and lightweight mode, `native_result_request: off` and hosts without the hooks use the MCP tool or `record-result`. Plan mode and a read-only sandbox get no request and stay read-only scopes, so nothing is saved there. The order matters: a first rewording that kept "MUST save" in front and made the prohibition conditional did not change a session's first answer, where the agent has not yet seen a request. With the rule as shipped, on the same prompts and model, `record-result` ran in 2 of 12 answers and `update` in none, all 12 native saves landed on the first try, and 5 of them carried learnings. An agent cannot see `native_result_request: off` unless it reads `runtime.json`: there it may skip a save the policy still asks for.
- **The Harness grants nothing on its own.** Its attached launch passes the edition's permissions to Claude Code through `--settings`, which Claude Code honours without the trust dialog, so the save runs there. A project with its own installation keeps its own trust: the Harness does not add `--allowedTools` to `claude -p` behind the person's back. Accept the project's trust dialog once instead, which `claude_code_bench.py --target` and the rollout rehearsal report.
- **A prompt sent while the save is written does not cancel it.** Claude Code and Codex take a prompt sent while the agent still writes the save into the running turn: they run the prompt hook at once and hand the prompt to the agent with the save command's result. Re-arming there replaced the request: the save in flight was refused as `request-unavailable`, and the agent read "not saved completely" in the middle of saving. A request asked less than ten minutes before now moves aside instead (journal `armed` `previous-in-flight`, no warning): its save still lands, the continuation's Stop settles it, and the prompt after reports it if it did not. Codex's continuation prompt is recognised by either request's token. The Claude desktop app mostly runs the prompt hook only when a queued prompt starts, after the turn, so the race is rare there; in 509 local desktop sessions 26 of 2,456 human prompts (1.1 %) were sent while the agent still worked. The queued prompt's own answer, given in the same continuation, is saved with the next answer's save; asking again at the continuation's Stop would be wrong, because a prompt sent after the save's last command runs as a turn of its own once this one ends, without the prompt hook running again, and that turn's Stop asks. Offline in the terminals of Claude Code 2.1.278 and Codex 0.163, with a scripted model that took 8 seconds to write the save; `scripts/claude_code_bench.py --queued 8` and `scripts/codex_bench.py --queued 8` repeat it.
- **Claude Code started below the project root gets the project's settings through a dispatcher in the person's own configuration**: hooks, permission rules, `env` and `AGENTS.md`, as a session in the root has them ([below](#claude-code-started-below-the-project-root)). So does one started in a worktree of the project that Git gives none of the installation, for the worktree: the root's memory, the worktree's own task ([below](#claude-code-started-in-a-worktree)). Codex started in such a worktree gets the root's hooks and `AGENTS.md` from a dispatcher in the person's own Codex hooks, once the person has trusted its entries in Codex ([below](#codex-started-in-a-worktree)).
- **After a compaction mid-turn** the session-start banner carries no task state. Claude Code's own summary keeps the current work, and the next prompt's capsule hands the memory again: a compaction resets what counts as handed.
- **Known limits:**
  - `/rewind` in Claude Code leaves no record in the transcript, so what the rewound turns were handed counts as handed for up to three turns, and a save step after rewinding past the conversation's first full one still points back to it.
  - Cursor's refusals of the save command are not recognised. Its hooks see `transcript_path`, and `postToolUseFailure` reports `permission_denied`, but nothing here could be checked against Cursor.
  - Claude Code started below the project root or in a worktree before the installer in a terminal or a first session in the root runs none of the project's hooks; Cursor below the root was not checked ([below](#claude-code-started-below-the-project-root)). A session that enters a worktree after starting in the root, through Claude Code's `EnterWorktree`, keeps the root's hooks, which leave the memory out there ([below](#claude-code-started-in-a-worktree)); Codex started in a worktree runs the root's hooks only once the person has trusted the dispatcher's entries, and Cursor started in a worktree without the installation runs no hooks ([below](#codex-started-in-a-worktree)). Codex started below the root finds and runs the project's hooks, and its save waits in the spool for the next hook (above). Claude Code's own Bash sandbox (bubblewrap) started in the root lets the save write. Started below the root or in a worktree, it lets a command write only there and in claude-<uid> under its own temporary directory, which it gives the command as TMPDIR: the save waits in the spool there, and the next hook, outside the sandbox, makes it (journal `saved` with code `spooled`), as Codex's does - and so does an attached session, whose state the Harness keeps outside the project; `scripts/claude_code_bench.py --sandbox` (with `--subfolder src`, `--worktree`, `--desktop`) repeats it on 2.1.278 and 2.1.299.

### Claude Code started below the project root

Claude Code reads a project's `.claude/settings.json` - its hooks, its permission rules and `env` - only from the folder a session starts in. It finds skills, commands, agents and `CLAUDE.md` above that folder, and it reads the root's `settings.local.json` too, but resolves that file's paths from the session's folder. Without the dispatcher below, a session started in `src/` of an installed project ran no capsule, no save and no guard hook; in `bypassPermissions` it read and edited `.env` and ran `rm -rf` and `sudo`, all of which the root denies; it was asked about reads, edits and commands the project allows; and `.claude/CLAUDE.md`'s `@../AGENTS.md` became an import from outside the working directory, which Claude Code asks about once per repository in a terminal (No is the default) and leaves out under `-p`.

**The dispatcher.** `memory-bank/scripts/claude_subfolders.py` keeps one in the person's Claude Code configuration folder (`~/.claude`, or `$CLAUDE_CONFIG_DIR`): `ai-accelerator/ai-accelerator-dispatch.sh` and `.py`, the roots it serves in `ai-accelerator/projects`, and in `settings.json` one entry per project hook (with its matcher and timeout), a `permissions` entry for every tool call, an `environment` entry and four `policy-N` entries at a session's start.

- **Who sets it up:** `local-context.sh`, in the background, in a session in the root; and the installer run by a person in a terminal (stdin and stderr a terminal - never a test, a bench, the Harness or a call into the module).
- **What it writes:** only for the Claude Code that started the hook (the session's transcript must be in that folder); nothing when all is in place; nothing into a `settings.json` that is not a JSON object, and one linked from elsewhere is written where it lives; every write under a lock; nothing once the person has put a file named `off` into `ai-accelerator/`, which also stops the dispatcher. One dispatcher serves every project: `DISPATCH_VERSION` keeps a newer copy in place when a project with an older release starts.
- **Where it acts:** for a session started in a folder below a listed root, in the same checkout, it runs the root's hook there, with `CLAUDE_PROJECT_DIR` set to the root. It does nothing in the root itself or below no listed root - a string match, 1.1 ms a hook - nor below a nearer installed project or in another Git checkout inside the project. Where it serves, its `permissions` entry starts a Python helper before every tool call, about 22 ms, and the guard runs as in the root, about 7.5 ms a Bash call, the helper joining it only for a command that names the memory CLI. On Windows it runs under the Git Bash Claude Code runs hooks with: it compares roots in Git Bash's form (`/c/Users/me/shop` for `C:\Users\me\shop`) without regard to case, finds Python as the hooks do (`python`, `py -3`, `python3`) and runs hooks with Git for Windows' bash - a bare `bash` started from Python is WSL's.

**What a session below the root gets.**

- **The project's hooks**, `bash-validator.sh` among them. A command that runs nothing but the memory CLI - the save step's - gets the allow the project's rules give it in the root.
- **The project's permission rules** (`permissions`): a deny or an ask wherever the root makes one; an allow only for a file inside the root or a single plain command, never against a deny or ask of the person's own or the managed settings. Read and Edit paths match as Claude Code 2.1.278 matches them in the root: gitignore patterns from the root, where a trailing `/` or `/**` names a folder at any depth (`./secrets/**` denies `src/secrets/key.txt` too) and a pattern with another `/` is anchored. Command rules reach through what Claude Code's own reach through - `env`, `timeout`, `nice`, `stdbuf`, `sudo`, `ionice` and other wrappers with their options, leading assignments, shell keywords (`if ...; then`, `do`, `{`, `!`), a bare `xargs` - but not through `command -v`, an `xargs` with flags, `busybox`, `coproc` or a wrapper named by its path; a rule may name the wrapper or what it runs.
- **The project's `env`** (`environment`), except the `CLAUDE_` and `ANTHROPIC_` variables, which set Claude Code itself, written into the file Claude Code names in `CLAUDE_ENV_FILE` for the session's commands. What `CLAUDE_CODE_DISABLE_EXPLORE_PLAN_AGENTS` does is done there by the `Agent(...)` denies and `subagent-gate.sh`.
- **The memory CLI's path:** the policy and the skills name the CLI from the root, `python3 memory-bank/scripts/context.py`, and the session's commands run in its own folder, so the banner names it by its path in the root - bare, or in single quotes, the form the allow for the memory CLI takes. `local-context.sh` reads the session's folder from its input's `cwd`: the dispatcher runs it in the root, Codex in the session's folder, and both say where the session is.
- **`AGENTS.md`** (`policy-N`), where the project's `CLAUDE.md` imports it. Claude Code 2.1 keeps a hook's output in the conversation only up to 10,000 characters and writes a longer one to a file, of which the model sees the first 2,000, so the policy comes cut between sections, the last entry taking whatever does not fit. Nothing comes when Claude Code imports the file itself: the dialog's Yes, which it files under the repository's root (a linked worktree's under its main checkout's, with forward slashes on Windows) in its `.claude.json`.

**How it was checked.** Offline, the real Claude Code 2.1.278 (and 2.1.258, 2.1.259, and 2.1.299, the desktop app's) started in `src/` got the capsule, every line of `AGENTS.md` in the conversation and saved, in the four editions and in the default, `acceptEdits`, `dontAsk`, `bypassPermissions` and `auto` modes; after `--resume` and `/clear` too, the policy there once; in its terminal with No and with Yes to the import dialog, and with a question typed during the save. Nineteen fixed tool calls (`scripts/claude_code_bench.py --permissions`) got the same decision in `src/` as in the root, and randomized runs of rules and commands in both - 480 pairs after the last change - differed only where Claude Code itself decides: its reading of the root's `settings.local.json` from the session's folder (stricter, never looser) and its own checks of commands against the folder a session starts in. In an unrelated project the dispatcher printed nothing and failed no hook. The Windows CI job runs the dispatcher, its helper, the setup and the installer's module path under Git Bash and Python 3.13; Claude Code itself was not run on Windows. `scripts/claude_code_bench.py --subfolder src` and `--permissions` repeat the host runs. `scripts/claude_code_bench.py --subfolder src --cli` has the stand-in first run the CLI as the banner names it: on 2.1.278, and on 2.1.299 in the desktop app's mode, it ran without a permission prompt and answered for the root's branch.

### Claude Code started in a worktree

Git gives a linked worktree only what it tracks, and an installation is often kept out of a client's history: all four installations this was checked on were untracked or ignored. Claude Code's own worktrees - `claude --worktree`, the desktop app's `.claude/worktrees/<name>` - copy `.claude/settings.local.json` and the ignored files `.worktreeinclude` names, not the installation. Started in such a worktree, the real Claude Code 2.1.278 ran none of the project's hooks: no project context, no `AGENTS.md`, no capsule, no save. Of the person's own Claude Code sessions in October, 12 of 90 started in one of Claude Code's worktrees.

**What the dispatcher does there.** A session's folder is in a linked worktree when the nearest `.git` above it is a file naming `<main checkout>/.git/worktrees/<name>`, a folder Git keeps with a `gitdir` file in it. When the main checkout is a listed root, or holds one - a project installed in a folder of a monorepo is that folder of the worktree - and the worktree has no installation of its own, the dispatcher runs the root's hooks there for the worktree, with `ACCELERATOR_HOME` and `ACCELERATOR_STATE_DIR` naming the root and `ACCELERATOR_PROJECT_DIR` the worktree: the shared layout of `memory-bank/scripts/workspace_roots.py`. A root the session's folder is below, in its own checkout or one around it, comes first: an installed worktree of an installed project is served as itself. It finds all of that without starting a process - a relative `gitdir`, which Git 2.48 writes on request, costs one subshell - so a session elsewhere still costs a string match, 1.2 ms a hook.

- **Shared:** the Project Brain and the Memory Bank are the root's, with the keys its own sessions give them; what a worktree session saves lands there, and the root's sessions read it.
- **The worktree's own:** the task, which its branch names, as everywhere; Git, the project's documents and citations; and the index, `memory-bank/local/worktrees/<hash>/context.db` in the root, because the two checkouts' documents differ. A file named `worktree` beside it names the worktree, and the first checkpoint after the worktree is removed removes its index. Nothing is written into the worktree.
- **From the root:** the accelerator's policy, skills, `specs/` and `tasks/`, which the worktree has none of; a record that cites one of them by the root's key is read there, and a learning saved in the worktree may cite them.
- **The memory CLI:** the worktree has no `memory-bank/scripts/`, so the session's banner says that the `memory-bank/` and `project-brain/` paths the policy and skills name are in the root, and names the CLI by its path there - `python3 <root>/memory-bank/scripts/context.py`, bare or in single quotes, the form the dispatcher's allow for the memory CLI takes - in place of the policy's `python3 memory-bank/scripts/context.py`. Run in the worktree without the dispatcher's variables - a Codex command, the memory MCP server, a terminal - the runtime finds the shared layout by itself (`workspace_roots.worktree_project`), from where the run is: `CLAUDE_PROJECT_DIR`, which Claude Code gives its hooks and MCP servers, else the working directory, so the hooks of a session in the main checkout stay there after its shell moved into a worktree inside it.
- **Under a sandbox** that lets a command write only the session's folder - Codex's workspace-write in a worktree or below the root, Claude Code's - a lookup (`status`, `retrieve`, `context`, `search`) that cannot write the index where the hooks keep it builds one of its own in the user's temporary folder (`ai-accelerator-index-<uid>`), and its manifest is the local one, written when it can be: the shared record a task's lookup publishes goes under the Project Brain's lock, which the sandbox does not let it take. A write - an `update`, a `turn --flush` - waits for the session's next hook instead ([below](#memory-commands-a-sandbox-cannot-write)); the save goes through the spool. Claude Code 2.1.278, and 2.1.299 in the desktop app's mode, with the sandbox on and started in `src/`, looked an existing task up that way; before, `[Errno 30] Read-only file system` on `project-brain/local/runtime.lock`.
- **Permissions:** the root's rules decide for the worktree's files as they do for the root's in the root, a deny or an ask also covers the root's own files from there - its `.env` among them - and an allow is given only for a file inside the worktree. `environment` also gives the session's commands the three variables, so the save step's command serves the worktree as the hooks do.
- **Policy:** `AGENTS.md` comes in parts whatever Claude Code was told about imports, since the worktree has nothing to import it from, unless the worktree has an `AGENTS.md` of its own.
- **The capsule** follows the project's profile: under `recall`, which installations get, a new branch's task starts with its working state and no documents, as a new branch in the root does; under `full` the root's knowledge comes with it.
- **Left alone:** a worktree with its own installation (Claude Code runs its hooks), a worktree of another repository, a `.git` file naming a worktree Git does not know, a submodule, and any session a launcher lends an accelerator to - the Harness, `accelerator_attach.py` - which runs that accelerator's hooks itself and names it in `ACCELERATOR_HOME`.

**A session that entered a worktree.** Claude Code's `EnterWorktree` moves a session that started in the root into `.claude/worktrees/<name>`, on a branch of its own; the root's own hooks keep running there, `CLAUDE_PROJECT_DIR` still naming the root and the input's `cwd` the worktree. They serve a worktree that has none of the installation as the dispatcher serves a session started in it - the shared layout, the worktree's branch, its banner after `/clear` or a compaction - and leave one with an installation of its own as before (`Memory not consulted`), as they do a Cursor session there, whose capsule would be a rule written into the worktree. The turn that enters the worktree was armed by its prompt in the root, so its Stop arms the request in the worktree (`native-stop --entered`) and that turn's work is saved under the worktree's branch too. The save step names the worktree (`--project <worktree>`), which the root it runs from does not show, and pipes the result without naming the descriptor: once a session has entered a worktree, Claude Code refuses a program fed a descriptor it names (`--input -`), whatever the permission mode. It asks before a command that leaves the entered worktree, as the save step's `cd <root>` does; the dispatcher's guard entry gives the memory CLI alone the allow it has in the root (`ai-accelerator-dispatch.py entered`). `scripts/claude_code_bench.py --enter-worktree` has the stand-in enter a worktree first and read, answer and save there: on 2.1.278, and on 2.1.299 in the desktop app's mode, the save landed under the worktree's branch with no permission prompt and the worktree left clean.

The same holds for a session launched with the edition lent from the clone (attached: the Harness's native launches, `accelerator_attach.py run claude`): the lent hooks serve the worktree it entered with the project's one state, which keeps the project and all its worktrees, and `--project` takes a checkout of that project's repository. Its save step's `cd` goes into the clone, which the launch adds as a working directory, so Claude Code passes it from an entered worktree too - and kept the session's shell there: every command the agent ran after a save, for the rest of a session a person drives, ran in the clone, and the Stop that settles the save no longer saw the worktree. A save that goes through now ends by going back to the folder the session was in (`&& cd <folder>`), and a refused one keeps its exit status, which a `;` would have hidden behind the `cd`'s; `cd -` is refused in an entered worktree as "a program assembled at runtime". `claude_code_bench.py --attached --enter-worktree` repeats it.

**How it was checked.** Offline, the real Claude Code 2.1.278 - and 2.1.299, which the desktop app bundles and starts with `--setting-sources=user,project,local` - started in `.claude/worktrees/<name>` of an installed project, and in a folder of a worktree beside the project, got the project context, `AGENTS.md`, a capsule with the worktree's own task and the save step; the save landed in the root's Project Brain under the worktree's branch, and under `full` a refresh there returned what the root's session had saved. Seven randomized sets of rules, 280 calls, got the same decision in a worktree for the worktree's files as in the root for the root's. On synced copies of the four client installations, `scripts/memory_rollout_rehearsal.py --worktree` ran the hooks through the dispatcher in a worktree of each under both profiles: every save landed under the worktree's branch, the next capsule recalled it, and no worktree was written. Thirty turns at once through the same hooks - ten each in the root, in a worktree inside it and in one beside it - all saved, no hook failed, and the Project Brain and the Memory Bank validated afterwards. A worktree Git has just checked out costs its first Stop up to a quarter second more there: Git rechecks every file of a fresh index, and the hooks, which take no Git lock, cannot record that; the first Git command that may - Claude Code's own status, or the agent's - ends it. `scripts/claude_code_bench.py --worktree` (with `--subfolder src` in a folder of it) repeats the host run, in all four editions, and with `--desktop` as the desktop app drives Claude Code - stream-json, permission prompts over stdio - where 2.1.278 and 2.1.299 asked none in the root, below it or in a worktree; `tests/test_memory_claude_subfolders.py` (`Worktrees`) and `tests/test_memory_worktree_sessions.py` the dispatcher's and the runtime's part; both run in the Windows CI job too, where Claude Code itself was not run. With `--cli` the bench's stand-in first runs the CLI as the banner names it: on 2.1.278, and on 2.1.299 in the desktop app's mode, in the worktree and in `src/` of it, the command ran without a permission prompt and answered for the worktree's task. Codex 0.164 started in a worktree inside an installed project, with none of its hooks, looked a question up with the root's CLI from its workspace-write sandbox: before, `[Errno 30] Read-only file system` on the worktree's index; now an index of its own in the temporary folder, and the worktree left clean.

**Not covered.** After a session entered a worktree, the root's permission rules decide by the root's paths: the dispatcher does not apply them to the worktree's files, as it does for a session started there. Claude Code 2.1.278 rereads its settings when `~/.claude/settings.json` changes during a session, and after `EnterWorktree` takes the project's from the worktree, which has none: a worktree entered in the very session that first sets the dispatcher up keeps no memory. Codex and Cursor started in a worktree without the installation run no hooks: Codex 0.164 started in a worktree inside an installed project, whose hooks it was told to trust, ran none of them. The memory MCP server finds its project by the nearest `.mcp.json` (Codex: `.codex/config.toml`) above the session's folder: a worktree inside the root gets the root's server, which serves the worktree; a worktree beside the root finds none.

### Codex started in a worktree

Codex reads a project's `.codex/` - its `config.toml` and `hooks.json` - and `AGENTS.md` only in the folders between the session's folder and the nearest `.git`, which in a linked worktree is the worktree's own `.git` file, inside the project too. A worktree of an installation Git does not track has none of them: Codex 0.164 started in a worktree of an installed project, the main checkout's hooks approved, ran none of them - no capsule, no save, no guard. Codex makes worktrees of its own too - `codex --worktree` and a new worktree in the Codex app put them under `$CODEX_HOME/worktrees/<id>/<repository>`, on a detached HEAD ([below](#a-worktree-codex-made)).

**The dispatcher.** `memory-bank/scripts/codex_worktrees.py` keeps one in the person's Codex folder (`~/.codex`, or `$CODEX_HOME`): `ai-accelerator/ai-accelerator-dispatch.sh` and `.py`, the roots it serves in `ai-accelerator/projects`, and in `hooks.json`, whose hooks Codex runs in every session, one entry per hook of the project's `.codex/hooks.json`, with its matcher, timeout and context limit, and one named `policy`.

- **Who sets it up:** the Codex copy of `local-context.sh`, in the background, in a session in the root; and the installer run by a person in a terminal (`CODEX worktrees <folder>` on stderr).
- **What it writes:** only for the Codex that started the hook (the session's transcript must be under `sessions/` in that folder); nothing when all is in place; the dispatcher's entries after the person's own, which keep their place, written the same way every time; a `hooks.json` linked from elsewhere where it lives. Nothing into a `hooks.json` that is not a JSON object, nothing for a person who keeps hooks in `config.toml` - Codex loads both forms but warns at every start that the layer has two - nothing once `ai-accelerator/off` exists, and nothing on Windows, where Codex does not run a hook's command with bash. Never `config.toml`.
- **The person's approval:** Codex runs a hook only once its definition was trusted: a hash it keeps in `config.toml` under `hooks.state`, keyed by the file and the entry's place. The setup leaves that to the person. At the next start Codex's terminal says "Hooks need review - N hooks are new or changed" and offers "Trust all and continue"; `/hooks` lists each entry with its source (`User config - ~/.codex/hooks.json`) and command; the app shows "N need review" with "Trust all" under Settings > Hooks. `codex exec` skips a hook nobody trusted without a word, so a person who runs only `exec` trusts the entries once in the terminal. Codex asks again for a changed definition, not for an edited script, so a release that changes the dispatcher's scripts asks nothing; a newer release's dispatcher and entries stay when a project with an older one starts.
- **Where it acts:** a session in a linked worktree - its `.git` a file naming `<main checkout>/.git/worktrees/<name>`, where Git keeps a file naming it back - whose main checkout is a listed root or holds one, in the root's folder of the worktree, with no installation of its own between the session's folder and the worktree's top. There it runs the root's `.codex/hooks/<script>` with `ACCELERATOR_HOME` and `ACCELERATOR_STATE_DIR` naming the root and `ACCELERATOR_PROJECT_DIR` the worktree: the shared layout of [Claude Code started in a worktree](#claude-code-started-in-a-worktree), with the same memory, task and index. In the root and below it Codex runs the project's own hooks; there, in every other project and in a session a launcher lends an accelerator to (`ACCELERATOR_HOME`), the dispatcher exits after a string match, without starting another process: 2.3 ms an entry, 1.2 ms more than an empty hook. Codex runs a hook's command with `bash -c` in the session's folder, the hooks of one event at once.
- **The policy:** `policy` gives the root's `AGENTS.md` whole as the context of the session's start, its `additionalContextLimit` 32,768 of Codex's tokens - Codex keeps 2,500 of a hook's context by default and puts the rest in a file. It comes at a new, cleared or compacted start; a resumed session has it in its history, which Codex replays. Nothing comes where the worktree has an `AGENTS.md` of its own, which Codex reads itself.
- **The save:** no hook can give Codex's commands variables, so in the shared layout the Codex save step names the three roots in its command (`cd <root> && ACCELERATOR_HOME=... ACCELERATOR_STATE_DIR=... ACCELERATOR_PROJECT_DIR=... python3 memory-bank/scripts/context.py ...`). Without them the command served the main checkout and refused a learning that cites a file only the worktree's branch has. Codex's workspace-write sandbox lets a command write the worktree, `/tmp` and `$TMPDIR`, not the root: the save waits in the spool, and the next hook, outside the sandbox, makes it (journal `saved` with code `spooled`).
- **Not from the root:** the project's `.codex/config.toml` - `project_doc_max_bytes`, the multi-agent switches, the memory MCP server - and its `.agents/skills`. `subagent-gate.sh` still refuses Codex's built-in subagents, and the Stop request saves without the MCP server.

### A worktree Codex made

`codex --worktree` and `codex exec --worktree` check the session's `HEAD` out - committed work only, not the working tree's changes - in `$CODEX_HOME/worktrees/<4 hex>/<repository>`, on a detached HEAD, and keep the session's subfolder. The Codex app makes its worktrees the same way (`git worktree add --detach`, from the default branch's remote ref unless the person picks a branch or the working tree), in `<name or 4 hex>/<repository>`. Both write the thread that owns the checkout into `codex-thread.json` of its Git directory: `{"version":1,"ownerThreadId":"<UUID>"}` from the CLI, the app's one key a line. A branch comes there only by an act: the app's "Create branch" (`codex/` and the title's first words) or branch switcher, the agent, which the app tells to name its branches `codex/...`, or "Move to worktree", which checks the person's branch out there. "Hand off to local" moves the work to a branch in the main checkout and detaches the worktree again. The app finds a thread's worktree by that record, and hands the record to another thread only when it archives the owner while another thread still works there.

A detached HEAD names no task, and the root's hooks, run there by the dispatcher, gave such a session neither capsule nor save. The thread that owns the checkout now names its task, `codex-thread/<UUID>`: after `CONTEXT_TASK_ID`, the branch and a rebase's branch, before nothing. Any other detached HEAD - the root's, a bisect, a tag, `git worktree add --detach` by hand - still names none.

- **Why the thread:** it is Codex's own link between the thread and the checkout, unique, and the same for the checkout's life - a resumed thread, `/new` in the terminal, a worktree restored from the archive. The folder's name (four random hex digits, or a name the person gives) and Git's name for the worktree (`project`, `project1`) come back for other work once a worktree is removed; a task named by them would hand the old work's state to the new.
- **When a branch is made there:** it names the task from the next hook on, as everywhere. The thread's task keeps the detached turns and is not carried over to the branch: the branch is what travels - "Hand off to local" checks it out in the main checkout, a pull request elsewhere - and no owner record goes with it, so a task kept under the thread there would be found from that worktree only. The thread's conversation holds its earlier turns, and the save after each answer puts the session's progress into the branch's task.
- **Checked like a branch:** the hooks pass it as the inferred identity (`--expected-branch`), and the runtime delivers its capsule only while `HEAD` stays detached and the record still names that thread. `context.py status` reports it (`automatic_memory.task_identity.source` `codex-thread`); the first capsule, before the task is recorded, names it; its goal reads "Work of the Codex thread that owns its worktree".
- **Same as Claude Code's:** Claude Code makes its worktrees on a branch of their own (`claude/<name>`), so each has its own task from the first prompt; so does a worktree Codex made now, and both follow a branch once there is one.

**How it was checked.** Offline, the real Codex 0.164 against a scripted model in a network namespace, with an isolated `CODEX_HOME`. A session in the root of a fresh install set up eleven entries and wrote no approval. A session in a worktree before the person approved them ran no hook: no capsule, no save step. After "Trust all" - the same `hooks.state` records, through the app-server request the terminal and the app use - a session in `.claude/worktrees/<name>`, in `$CODEX_HOME/worktrees/<id>/project` and in `src/` of one got the capsule and every line of `AGENTS.md`; its save of a learning citing the branch's own file waited in the spool and landed in the root's Project Brain under the worktree's branch, the worktree got an index of its own in the root, and nothing was written into the worktree. So it went through `codex exec` in all four editions, and in PHP Core also through `codex app-server` as the desktop app drives a turn (on-request approvals, none asked) and in the terminal with a question typed while the save was written. With the roots taken out of the save step the same run ended `stop:failed:unsaved`. On synced copies of the four client installations, `scripts/memory_rollout_rehearsal.py --hosts codex --worktree` ran the hooks through the dispatcher in a worktree of each under both profiles: every save landed under the worktree's branch, the next capsule recalled it, and no worktree was written; the real Codex passed the bench in a worktree of synced copies of two of them, one inside the project and one beside it. `scripts/codex_bench.py --worktree [inside|beside]` (with `--subfolder`, `--app-server`, `--queued`) repeats it; `tests/test_memory_codex_worktrees.py` covers the setup, the dispatcher, the policy and the save step. In a worktree Codex made itself (`--worktree codex`: `codex exec --worktree`, and with `--subfolder src` in `src/` of it), detached, the session got the capsule and every line of `AGENTS.md`, and its save, of a learning citing a file the main checkout's working tree had deleted, landed under `codex-thread/<UUID>`, its own thread's; so it did through `codex app-server` in a worktree the bench made the way the app does, the owner record written once the thread had started (no approval asked). Before the change the same session ran the hooks and got neither. `memory-bank/tests/test_hooks.py` (`CodexWorktreeOwnerTest`) and `project-brain/tests/test_runtime.py` (`CodexThreadTaskTest`) cover the hooks, the admission, `status` and the capsule.

**Not covered.** The Codex app itself was not driven: its worktrees were read from its code and made by the bench the same way. When the app hands the owner record to another thread - it archives the owner while another thread still works there - that thread's task starts empty. Cursor started in a worktree without the installation runs no hooks.

### Memory commands a sandbox cannot write

The policy has an agent record a stage boundary with `context.py update`, and the `checkpoint` skill flushes the buffered turns (`turn --flush`) and updates the task itself. A sandbox that lets a command write only its own folder refused both: Codex's workspace-write sandbox in a worktree of the project or below its root - it writes the session's folder, `/tmp` and `$TMPDIR` - and Claude Code's sandbox there. Under Codex 0.164's own sandbox (`codex sandbox -P :workspace`) in a worktree of an installed project, with the roots named, `update` failed at the Brain's lock (`Read-only file system: .../project-brain/local/runtime.lock`) and `turn --flush` at the index (`attempt to write a readonly database`), whatever path the command named the CLI by.

- **They wait:** an `update` or a `turn --flush` that cannot write the Brain's lock folder (governed) and the index's folder is checked as far as nothing has to be written - the task ID, a changed field, the secret and personal-data refusals, so the agent can still correct it - and then waits, owner-only, in `ai-accelerator-memory-queue-<uid>` in the first temporary folder the sandbox writes (`$TMPDIR`, `/tmp`, Claude Code's `claude-<uid>`), as the save does. It answers `queued` (`"queued": true` with `--json`) and exits 0; the `checkpoint` skill reports that it is made when the turn ends.
- **The next hook makes them:** a `turn` that can write - the Stop hook's, outside the sandbox - makes what waits for its project, the state it writes and the checkout it reads, oldest first, after the turn it records and before the Stop's save request, so a stage boundary lands before the answer's save. A file goes before its command runs; each is journaled (`native-results.ndjson`, stage `queue`, `made` or `refused`), and one refused is named in the next capsule's last-turn line ("queued memory command not made: ..."). With nothing waiting, a turn spends 0.07 ms looking.
- **Nothing else:** only these two commands, rebuilt from their own options, for this project, younger than a day. A file naming anything else - another command, an index, another root, a `turn` without `--flush` - or a link is refused unread. A command a drain runs fails rather than waiting again.
- **Not queued:** `refresh`, which the `memory` skill runs, reports its three layers failed there ("SQLite writes unavailable"); the hooks refresh the index on every prompt.

**How it was checked.** Offline, a scripted model in a network namespace (`--cli checkpoint` in `scripts/codex_bench.py` and `scripts/claude_code_bench.py`): the stand-in makes a checkpoint as the installed skill says, with the memory CLI the session's banner names. Codex 0.164 in a worktree inside and beside the root and in one it made itself, Claude Code 2.1.299 with its sandbox on in `.claude/worktrees/<name>`: both writes answered `queued`, the Stop made both (journal `queue made` twice), and the checkpoint landed in the root's Project Brain under the worktree's task; before, the flush failed with `can't open file '<worktree>/memory-bank/scripts/context.py'`. Read-only folders stand in for the sandbox in `project-brain/tests/test_runtime.py` (`SandboxQueueTest`).

### The checkpoint names the hooks' task

The `checkpoint` skill took the branch alone and stopped on a detached HEAD, where the hooks name the branch a rebase in progress rewrites, or `codex-thread/<UUID>` in [a worktree Codex made](#a-worktree-codex-made): a checkpoint there saved nothing. Its step 1 now takes the task the hooks name - `CONTEXT_TASK_ID`, otherwise `git symbolic-ref --quiet --short HEAD`, and on a detached HEAD the `automatic_memory.task_identity.task_id` that `context.py status --json` reports. `status` names a rebase's branch too now (source `git-rebase`), where it said HEAD could not supply a task and told the person to set `CONTEXT_TASK_ID`, so it names what the hooks name in every state; any other detached HEAD still names none, and the skill stops with `status`'s remediation. The lightweight checkpoint resolves its task the same way, and so does the one the generator writes (`skill-forge/references/php-process-skills.md`). The skill and the `memory` skill read the mode - governed or lightweight - from `project-brain/config/runtime.json` under `git rev-parse --show-toplevel`, which a worktree the installation serves, or a project an accelerator is lent to, does not hold; both take the `mode` `status --json` reports now.

On a task with nothing buffered yet - a new branch, a worktree's first answer - the skill's `turn --flush` made no task and its `update` was refused ("Working task not found"), in the root too. An explicit `turn --flush` now makes the task, as the memory MCP server's checkpoint already did; the Stop hook never flushes explicitly, so visiting a branch still mints nothing, and a completed task is not made again.

**How it was checked.** The same benches: in a worktree Codex made, directly and under the app-server, step 1 took `codex-thread/<thread>` from `status` and the checkpoint landed under it, where the skill had stopped; in the project root of a fresh install, under either host, the checkpoint now lands where its `update` was refused. `memory-bank/tests/test_checkpoint.py` runs the prompt hook and the skill's step 1 side by side for a branch, `CONTEXT_TASK_ID`, both rebase backends, both owner records and a worktree Codex made.
