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
checkpoints that same task while the rebase runs.
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

## Automatic results in native sessions

Governed Codex, Claude Code and Cursor sessions now use one bounded same-agent Stop continuation to record the result of the work already performed. The prompt hook arms ignored metadata only; the normal Git checkpoint runs first at Stop. The continuation calls `context.py native-result` with progress, optional next steps and at most three source-backed findings/decisions. `verified` is a top-level boolean and means agent attestation, never human review. Empty learnings and an explicit skip are valid. No prompt, final prose or transcript is parsed into authoritative knowledge.

A read-only research result can provision its task after validation without inventing a Git delta. The challenge pins project, actor, host/session, task UUID/revision, checkout and runtime/config. A new user prompt retires the prior generation; the native internal continuation preserves it. Cursor sends that continuation back through its prompt hook as a prompt. The hook arms (which journals it as `continuation`) and exits before the refresh: the turn's own prompt already brought the capsule, and Cursor, which reads its rule before the hook runs, keeps the rule the last real prompt wrote for the person's next request. Codex 0.163 adds the continuation to the same turn as a `<hook_prompt>` item and runs no prompt hook for it; the same guard in its prompt hook, and `refresh --host codex` (and `--host claude`) building no capsule for such a query, stay as defence for a host that does send it as a prompt. Duplicate Stops request no additional continuation; exact repeated saves reuse existing result fencing. The recovery Stop does not checkpoint the same user work again. Completion still requires the explicit verified workflow.

A session whose save command Claude Code refused is not asked again for a day. Claude Code records the refusal on the command's result in the transcript (`toolDenialKind` `user-rejected`, `permission-rule` or `automode-blocked`; `interrupted`, `cancelled` and auto mode's classifier failing are not refusals), and the recovery Stop reads it from the transcript's last 512 KiB. A person who answers No in Claude Code's dialog ends the turn before any Stop runs, and a refusal made just before the recovery Stop may not be on disk yet (Claude Code writes its transcript in batches), so the next prompt reads it there too. A refusal comes back after every answer: a person said no, a rule or auto mode denied the command, or `claude -p` ran in a project whose trust was never accepted, whose allow rules Claude Code then ignores. Each of those answers cost two model calls. The first prompt after the refusal prints the usual "not saved completely" warning; later ones arm nothing (`prompt` `skipped` `host-refused`). A new session is asked again.

A turn that ends in plan mode (`permission_mode: plan`, which Claude Code and Codex send) is not asked: the host tells the agent to change nothing, overriding any other instruction, so the agent would refuse after every answer or write against the person's choice. A save asked before the switch is still settled. Codex's `permission_mode` follows its approval policy only, so for Codex the Stop reads the current turn's `turn_context` from the end of the rollout: Plan collaboration mode is skipped the same way, and a read-only sandbox, in which no save can write the project, as `read-only-sandbox`. A workspace-write sandbox whose restricted file-system policy does not let a command write the project's Brain - Codex started in a subdirectory of the project writes only there - is skipped as `outside-sandbox`: the most specific entry over the Brain decides, as it decides Codex's own read-only carve-outs. A policy that, read this way, would not let the turn write its own working directory is in a form the runtime does not know, and the save is asked for as before. A turn whose context it cannot find is asked as before. Each host gets the step its own way. Codex gets a Stop `decision: "block"`, whose reason it adds to the turn as a `<hook_prompt>` item, XML-escaped (the command's `&&` reaches the model as `&amp;&amp;`, which models read as `&&`). Cursor gets a `followup_message`. Claude Code gets `hookSpecificOutput.additionalContext` (Claude Code 2.1.163 or later; an earlier one is not asked). A block would show in its transcript as a hook error, raise "Stop hook error occurred" after every answer and reach the model twice. Claude Code's command is `cd <root> && echo -E '<JSON>' | python3 memory-bank/scripts/context.py … --input - --json`, the agent writing the JSON in place of `<JSON>`:

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
| `save` | `saved` and `held` with `learnings` written and `promoted` chunks (code `repeat` for an exact repeat of a finished save, `late` for a begun save finished after the recovery Stop marked it failed, `in-flight` for a save that landed after a prompt moved its request aside); `omitted` (`in-flight` likewise); `refused` with the refusal's machine code; `error` with the exception type |
| `stop-hook` | `exited` with the save request's `status` (124: killed at the budget), or `skipped` with `capsule-delivered` |

A refused result keeps the text the agent corrects and carries a fixed code (`ResultRefused.code`). That text says what to correct: an input that is not one JSON object is refused as `native-result:invalid-input` with the JSON error, its line and column, and, for a raw newline in a string, the hint to write it as `\n` and to pipe with `echo -E` (a plain `echo` in zsh turns `\n` into a line break). A learning with missing or unexpected fields names them, and `verified` inside a learning is called a top-level boolean. A save overtaken since it was requested (`memory-result-v1:prepared_stale_no_write`, or `native-result:task-changed` when the branch task was created or replaced meanwhile) says that nothing was written and that the agent reports the held save instead of retrying it: two sessions on one branch keep the first save. The codes: `memory-result-v1:schema`, `learning_fields` (often `verified` put inside a learning), `learning_type`, `no_sources`, `source_path`, `source_missing`, `conflicting_learnings`, `unverified`, `nothing`, `text_limit`, `sensitive`, `raw_text`, `task_id`, `result_id`, `revision`, `stale_revision`, `result_id_reused`, `task_closed`, `task_privacy`, `not_authorized`, `record_not_found`; the fence codes it already had (`scope_changed`, `prepared_stale_no_write`, `receipt_unavailable`, `attempt_*`); a `native-result:*` code; or `unclassified` for a Project Brain refusal that has none. The Stop hook writes its own line because the case worth counting is the one the runtime never finishes. The journal follows `memory-bank/local` as the runtime's other local files do, is never written through a linked file, and keeps eight lines per `refresh_health_retention` record. `scripts/memory_eval.py realized` prints it per host (see `docs/MEMORY-EVAL.md`).

Hooks need Python and a trusted/enabled native hook configuration. The integration adds one internal model continuation when a native result has not yet been submitted; its cost is not assumed to be free or offset by savings. Live-host acceptance and task/cost evaluation are separate checks.

That continuation follows every answer. On 21 Codex runs with one native save each (small tasks, 16-19 October 2026), it took a median of 10 seconds, 383 output tokens and 59k input tokens, most of them cached: about half of the answer's own time and tokens. A team that does not want it on every turn sets `"native_result_request": "off"` in `project-brain/config/runtime.json` (or `CONTEXT_NATIVE_RESULT_REQUEST=off` for one session). No prompt is then armed and no Stop asks; results reach memory only through `context.py record-result`, the MCP `memory_record_result` tool or the Harness, and the journal records each skipped prompt and Stop as `switched-off`. The default, `every-turn`, keeps the request after every answer. Any other value refuses the prompt's arming (`native-result:invalid-request-setting` in the journal). `--sync` never changes `runtime.json`, so the setting survives updates.

How the continuation ends depends on who reads the session. A headless run (`codex exec`, `claude -p`) hands its caller only the final message, so there the agent returns its answer again after the save. In a session a person reads as it goes, the answer is already on screen above the continuation, and the repeat doubled it: on nine Codex runs the message after the save was 93 % of the answer's length, against 21 % before the repeat was asked for. Under `"native_result_answer": "auto"`, the default, a session its host marks as attended ends with no further text or one short line:

- Claude Code: `CLAUDE_CODE_SESSION_ATTENDED=1` and an entrypoint of `cli`, `claude-vscode` or `claude-desktop`. A `claude -p` started from such a session inherits the entrypoint but is a child session, which Claude Code never marks attended.
- Codex: the rollout's first line names the session's source as `vscode` (the app, the IDE extension and the terminal UI) or `cli`; `exec` and subagents repeat.

Every other session repeats, Cursor's included. `repeat` always repeats and `brief` never does; `CONTEXT_NATIVE_RESULT_ANSWER` sets it for one session. An unknown value repeats, and the journal's `requested` line carries `native-result:invalid-answer-setting`: the save itself is asked for either way.

The step carries its rules - the JSON fields, the schema example, what belongs in learnings - in full only while the conversation lacks them. Every answer used to carry about 2,600 characters, and Claude Code's terminal showed them whole each time. The Stop reads the transcript's last 2 MiB: when a full step is there after the last compaction record, the step carries its commands, its ending and a pointer back to those rules, about 900 characters less. After a compaction, or without a readable transcript, the full step comes back. Cursor gets the full step every time: how its transcript records a summarized conversation could not be checked. `"native_result_instruction": "full"` in `runtime.json` (or `CONTEXT_NATIVE_RESULT_INSTRUCTION=full`) sends it every time; an unknown value does too.

The save takes two more model calls, and Claude Code counts them against `--max-turns` (`maxTurns` in the Agent SDK). A headless run whose answer comes in its last two allowed calls ends with `error_max_turns` and no `result`; the Stop hook is told neither the limit nor the count. Give such a run two more turns, or set `CONTEXT_NATIVE_RESULT_REQUEST=off` for it (`docs/TROUBLESHOOTING.md`).

### Decisions and known limits of the native save

- **The revision fence stays at the Stop.** A request pins the task's revision when the Stop asks for the save, not when the prompt was read, so it refuses only a save another writer overtook in the seconds before it. Pinning at the prompt would refuse every save of a session that ran beside another one. Letting a stale save through would overwrite the newer progress. On the four client projects, two sessions worked in one directory in the same minute in 2 to 4 % of the minutes of work, so a refused save is rare. It says why (see above), and the next prompt warns.
- **The Stop request is the save; the policy asks for an explicit one only where none comes.** `AGENTS.md` used to require an explicit save (`context.py record-result`, `memory_record_result`, Harness `memory-draft`) after all authorized work, and the Memory MCP server asked for `memory_record_result` before finishing. An agent that followed it saved every answer twice, once itself and once when the Stop asked: on a synced copy of a client project, gpt-6.1-sol on Codex 0.163 ran `record-result` in 12 of 12 answers and a closing `context.py update` in 7, and the native save that followed carried `learnings: []` every time. The save rule now opens with the prohibition - no `record-result`, `memory_record_result` or closing `update` where Stop requests the save - and the Stop bullet names where that is: governed Claude Code, Codex and Cursor sessions running the hooks, outside the Harness, unless `native_result_request` is `off`. The stage-boundary `update` rule leaves out an answer's end that Stop's request saves. Everywhere else the explicit save is still required, because nothing else would save the answer: a Harness session saves its `memory-draft` block, and lightweight mode, `native_result_request: off` and hosts without the hooks use the MCP tool or `record-result`. Plan mode and a read-only sandbox get no request and stay read-only scopes, so nothing is saved there. The order matters: a first rewording that kept "MUST save" in front and made the prohibition conditional did not change a session's first answer, where the agent has not yet seen a request. With the rule as shipped, on the same prompts and model, `record-result` ran in 2 of 12 answers and `update` in none, all 12 native saves landed on the first try, and 5 of them carried learnings. An agent cannot see `native_result_request: off` unless it reads `runtime.json`: there it may skip a save the policy still asks for.
- **The Harness grants nothing on its own.** Its attached launch passes the edition's permissions to Claude Code through `--settings`, which Claude Code honours without the trust dialog, so the save runs there. A project with its own installation keeps its own trust: the Harness does not add `--allowedTools` to `claude -p` behind the person's back. Accept the project's trust dialog once instead, which `claude_code_bench.py --target` and the rollout rehearsal report.
- **A prompt sent while the save is written does not cancel it.** Claude Code and Codex take a prompt sent while the agent still writes the save into the running turn: they run the prompt hook at once and hand the prompt to the agent with the save command's result. Re-arming there replaced the request: the save in flight was refused as `request-unavailable`, and the agent read "not saved completely" in the middle of saving. A request asked less than ten minutes before now moves aside instead (journal `armed` `previous-in-flight`, no warning): its save still lands, the continuation's Stop settles it, and the prompt after reports it if it did not. Codex's continuation prompt is recognised by either request's token. The queued prompt's own answer, given in the same continuation, is saved with the next answer's save; asking again at the continuation's Stop would be wrong, because a prompt sent after the save's last command runs as a turn of its own once this one ends, without the prompt hook running again, and that turn's Stop asks. Offline in the terminals of Claude Code 2.1.278 and Codex 0.163, with a scripted model that took 8 seconds to write the save; `scripts/claude_code_bench.py --queued 8` and `scripts/codex_bench.py --queued 8` repeat it.
- **After a compaction mid-turn** the session-start banner carries no task state. Claude Code's own summary keeps the current work, and the next prompt's capsule hands the memory again: a compaction resets what counts as handed.
- **Known limits:**
  - `/rewind` in Claude Code leaves no record in the transcript, so what the rewound turns were handed counts as handed for up to three turns, and a save step after rewinding past the conversation's first full one still points back to it.
  - Cursor's refusals of the save command are not recognised. Its hooks see `transcript_path`, and `postToolUseFailure` reports `permission_denied`, but nothing here could be checked against Cursor.
  - Claude Code started in a folder below the project root runs none of the project's hooks: it reads `.claude/settings.json`, which holds the hooks and the permission rules, only from the folder it starts in, while it finds skills, commands and agents above it. No capsule, no save and no guard hook runs there, and `.claude/CLAUDE.md`'s `@../AGENTS.md` becomes an import from outside the working directory, which Claude Code asks about in a terminal and leaves out under `-p`. Start Claude Code in the project root. Codex started below the root finds and runs the project's hooks, and its save is skipped as `outside-sandbox` (above). Both offline, on Claude Code 2.1.278 and Codex 0.163. Claude Code's own Bash sandbox (bubblewrap) started in the root lets the save write.
