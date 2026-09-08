# AI-Infrastructure Harness

A local browser workspace over **Claude Code, Codex and Cursor Agent**.
Kit 1, Kit 2 and Kit 3 are sections within the workspace. The optional
LangGraph batch runner remains available separately below.

In **Sessions**, use **Attach files**, select files and write your message. You
can remove files before sending. Each message accepts up to five files, 4 MiB
per file and 8 MiB total. Sent files appear as downloads in message history and
survive server restarts. Unsent selections remain in the current page only.
Uploads are stored under the Harness state directory, outside the selected
project/worktree. Native agents receive local paths and read them using their
available tools; document/image format support depends on the provider and tools.
Filenames and sizes are validated, and stored bytes are checked before use.
Attachments are reference data and are not automatically executed or promoted
into Memory Bank. A failed send retains the selected files for correction.

## Infrastructure Creator in the browser

Choose **Infrastructure Creator** (also available through Kit 1), a registered
PHP project, Claude/Codex/Cursor, model, thinking effort and additional-agent
limit. Choose the current project or a new Git worktree, select the integrations
to generate, and start a scan. A worktree starts from committed HEAD; uncommitted
files are not copied. Targets must be outside the Harness source/state trees.

The server copies the existing generator into a private task workspace and runs
its native workflow in the shared session queue. Filesystem isolation is
required: **bubblewrap** (`bwrap`) on Linux, or the built-in **sandbox-exec**
(Seatbelt) on macOS. Agent writes are confined to the workspace, a private
temporary directory and native provider account-state directories; the project
and Creator control files stay read-only (bind mounts on Linux, write denial on
macOS). Native authentication remains with each provider. Network
access and native integrations remain available; this is filesystem isolation,
not a general-purpose sandbox for hostile plugins. Apple marks `sandbox-exec`
as deprecated but still ships it; a provider CLI that must write outside its
account directory fails inside the profile and reports it. Creator model-phase budgets can
be changed in the browser; empty time means no time limit. Apply and rollback
do not inherit model budgets.

Read the scan profile, generation plan and quality report, answer any questions,
and choose **Approve profile & generate**. Canonical generator validators run
independently after the provider exits. Generation ends at staging. Inspect the
file diffs and approve each replacement of existing team edits before choosing
**Apply reviewed files**. A changed project, staging payload or reviewed report
invalidates the approval. To preserve or merge a file differently, request a
revision; the generator prepares a new candidate and ownership manifest.

**Update existing accelerator** requires `.infra-manifest.json`. Runtime memory
and Project Brain records cannot be overwritten by the publication plan. Apply
uses the existing manifest-last publisher, then validates the installed target
with a disposable SQLite cache. On Linux that cache is a temporary overlay and
verification does not change the project cache; on macOS the project's ignored
`memory-bank/local` cache is rebuilt in place, because Seatbelt cannot overlay
a directory.
A failed postcheck rolls back; an interrupted publication exposes **Recover
interrupted publication**. Recovery preserves external edits made since apply
and reports conflicts instead of silently replacing them. Runs, reports and
recovery journals survive restarts in the server's private `creator/` directory.
Apply/recovery cannot be cancelled from either the Creator UI or session API.

Automated coverage uses disposable native CLI fixtures, real bubblewrap mounts
on Linux, an offline check of the generated Seatbelt profile and the canonical
publication/rollback helpers; model-generated content still
requires the scan and diff reviews and independent verification gates.

## Browser server

### Spec-Driven Development

In **Sessions → Workflow → SDD**, enter a feature slug such as `user-login` and
choose **Initialize**, **Specify**, **Plan**, **Tasks**, **Implement / resume** or
**Review**. Describe the request and launch the selected phase with Claude,
Codex or Cursor. The feature stays fixed; the phase can change on follow-up turns
in the same native session and workspace, including a Git worktree.

New documents live in `specs/<feature>/sdd-{spec,plan,tasks,progress}.md`; shared principles
live in `specs/memory/sdd-constitution.md`. The `sdd-` prefix satisfies installed
accelerator naming hooks. Existing unprefixed documents remain readable in place.
**Read SDD documents** shows bounded,
read-only snapshots from the session's actual workspace. Refresh after edits.
Review documents before explicitly selecting and sending the next phase.
Initialize only requests missing guidance; existing project documents are preserved.

The server requires nonempty, readable prerequisite documents (maximum 256 KiB
each) and blocks later phases while the specification contains
`[NEEDS CLARIFICATION` markers. It checks again before a provider starts, including
inside a new worktree. File presence is not semantic approval: native-agent
instructions limit each turn to its selected phase, and humans review its output.
Writing phases use Edit permissions; Review uses Plan permissions. Implementation
instructions require test-first tasks and stopping at the next checkpoint.
Use Results for diffs and independent verification commands. Phase settings are
saved in launch history alongside model, effort and budgets. No separate plugin
installation or automatic phase chaining is required.

### Clash: Claude and Codex confront each other

In **Sessions**, tick **Clash with a challenger** on a Workspace or Review
session. The selected provider is the first participant and a *different*
provider is the **challenger**. The stage follows the session mode:

- **Edit mode (implementation clash):** the first participant implements the
  task with Edit permissions. The challenger then reads the workspace and the
  diff against the session baseline in Plan permissions and returns a verdict
  with numbered objections (severity, file, line, claim, evidence). The
  implementer must fix, rebut or defer every open objection; the challenger
  re-checks and either accepts or maintains its objections.
- **Plan mode (review clash):** the first participant reviews the scope and
  reports findings; the challenger reviews the same scope independently, agrees
  with, disputes or cannot verify each finding, and adds what was missed; the
  reviewer defends or withdraws disputed findings and assesses the challenger's
  additions. The Review workflow always runs this stage.

Each cycle runs the opening turn plus up to the selected number of **challenge
rounds** (1–3, default 2). The challenger's final verdict ends the cycle; the
first participant responds only between rounds. The outcome is **converged**
when the challenger accepts with no blocking items, **unresolved** when the
rounds run out, and **incomplete** when a turn fails or returns no structured
verdict. Structured verdicts are parsed from the fenced JSON block each
participant is asked to end with; the ledger records every objection or finding
with its status and the exchange behind it. The ledger, per-turn usage and a
Markdown report are saved with the session and shown under **Clash ledger**;
Results shows the resulting diff for independent checks and delivery.

Every turn is a separate native run of one participant in the session's
workspace, resuming that participant's own native session. A follow-up message
with the box still ticked starts the next cycle: both native sessions resume,
the session mode stays fixed, and the challenger, the rounds or the challenger's
model and thinking effort can change. Untick the box to send an ordinary
follow-up; tick it later to have the challenger inspect the accumulated work,
continuing the earlier ledger when the stage is the same. The option is not
available for Plan, SDD or Fleet sessions, requires additional agents to be
off, and cannot be combined with separate planning and editing models.
Budgets apply to the whole cycle: Claude turns receive the remaining
USD allowance as a native cap, the token threshold counts every turn's reported
usage, and the time limit terminates the cycle. Cancel stops the running
participant; the ledger recorded up to the previous turn stays visible. The
ledger reports what each participant claimed and which claims survived; it is
not an independent verification of the work.

### Optional models for planning and editing

In Sessions, expand **Separate models for planning and editing** and enable
**Use separate planning and editing models**. Choose a model and thinking effort
for each role from the selected provider, or enter a custom model ID. The common
model controls display the effective selection while this option is enabled.

In SDD, **Implement / resume** uses Editing; Initialize, Specify, Plan, Tasks
and Review use Planning. Writing specification documents does not select the
code-editing model. In Workspace, **Mode → Plan/Edit** selects the role and can
change between turns while the option is enabled. Plan and Review workflows
always use Planning. Fleet and Creator retain their existing model controls.

Both role configurations are validated before launch. Changes apply when sending
the next turn and persist with the session; each launch retains its effective
model, effort and routing settings. Disable the option to use the common model
controls again. The default remains one model, and new sessions start with the
option disabled. Switching stays within one native provider and preserves the
session/workspace; it does not transfer conversations between providers.

### Starting the server

Requires Linux or macOS, Python 3.9+ and at least one authenticated native
agent CLI. The web server uses only Python's standard library; no venv,
Node frontend build or API-key form is required.
Fleet review additionally uses the optional LangGraph runtime described below.

From the repository root:

```bash
./harness-server start --project /absolute/path/to/project
# Started: http://127.0.0.1:8766
./harness-server status
./harness-server stop
```

`start` detaches the server from the terminal. `serve` keeps it in the
foreground. Repeat `--project` to register more directories; without it,
the current directory is registered. **Projects & Setup** can register additional
existing directories while the server is running. UI registrations persist across
restarts alongside the command-line projects. Stop and start again to change
server options such as its port or provider executable:

```bash
./harness-server start --port 8766 --timeout 900 \
  --project /path/to/project-a --project /path/to/project-b \
  --cursor-bin /absolute/path/to/cursor-agent
```

`--claude-bin` and `--codex-bin` also accept explicit executables. Discovery
checks the CLI's identity and required Cursor flags; an unrelated command
named `agent` is not accepted as Cursor. Cursor auto-update is disabled on
all harness invocations so it cannot replace another tool's launcher.
Authenticate using each native CLI in a terminal before running browser
tasks. Existing native credentials remain managed by that CLI. A detected
executable does **not** prove that its account is logged in.

### Workspace options

- **Projects & Setup:** add an existing project, inspect its Git state and
  accelerator readiness, select an edition and target tools, then preview and
  install the reviewed files. The resulting project is available in the session,
  skills and knowledge selectors immediately.
- **Sessions:** registered project, Claude/Codex/Cursor, model and thinking effort,
  Workspace/Plan/Review workflow, Plan/Edit mode, optional project context,
  additional-agent switch and a concurrent helper limit (1–40, default 3).
  Shows the selected project's Git branch and changes, with a refresh control.
  Choose the project directory or a new Git worktree with an optional new branch name.
  Native messages and tool status appear as they arrive. Cancel stops the
  process group; follow-ups resume the same native session and workspace directory.
- **Clash with a challenger:** a checkbox on Workspace and Review sessions that
  pits the selected provider against a different challenger provider on the
  implementation (Edit mode) or the review (Plan mode); inspect the
  objection/finding ledger and each turn's usage, then continue with a follow-up
  cycle. See [Clash](#clash-claude-and-codex-confront-each-other).
- **Fleet review workflow:** select code, security and/or performance reviewers,
  inspect their stages and findings, then approve or reject the report. Native
  reviewers run directly through the selected Claude/Codex/Cursor provider in
  plan mode, without nested helpers. The additional-agent limit bounds concurrent
  reviewers. Offline dry-run uses labelled synthetic findings and no model calls.
- **Results & usage:** open a session's changed files and Git diff, run an explicit
  test/lint command in its workspace, and inspect persisted output, exit code and
  check status separately from the agent outcome. Shows every recorded launch
  and total reported tokens, USD and run time; missing provider usage stays unknown.
- **Project context:** availability and size of a fixed set of project
  instructions and references. Optional prompt context includes up to 3 KB
  each from `AGENTS.md`, `CLAUDE.md`, `README.md`,
  `project-brain/README.md`, and `specs/MANIFEST.md`. Symlinks are skipped.
  The native CLI can independently load its normal project instructions.
- **Memory bank:** browse a project's `memory-bank`, select a document and
  read its Markdown source, including chunk metadata and status. The repository's
  Laravel, Symfony, PHP Core and WordPress banks appear separately. The viewer
  lists Markdown in the bank root, `chunks/` and `local/` notes; derived SQLite
  indexes, scripts, templates and Project Brain records are excluded. Browsing
  does not execute scripts or change files. Separate controls run context search,
  index maintenance, validation, source-freshness audit, re-attestation, retirement
  and privacy-filtered export through the selected project's installed CLI.
  Listings are bounded to 500 documents and document previews to 256 KiB,
  with visible truncation notices.
- **Project Brain:** browse tasks, findings, bugs, incidents, decisions, events,
  handoffs, promotion records and archives. Start tasks, create operational
  records, update progress and complete tasks through the existing governed
  runtime. Numeric revisions protect updates and completion against stale edits.
- **Accelerators:** Kit 1 opens **Infrastructure Creator** for the full
  Scan → Review → Generate → Apply workflow and manifest-aware updates.
  Kit 2 opens the same **Projects & Setup** installer for Laravel, Symfony,
  PHP Core or WordPress.
- **Open Source Kit:** the existing Kit 3 catalog, filters, dossiers and
  copyable installation commands inside the same browser workspace.
- **Skills:** choose a registered project and a catalog source, click **Load
  skills**, select individual skills and Claude/Codex/Cursor, then **Preview
  installation** and **Install selected skills**. The preview lists every
  destination and any conflicts. Installation adds missing files, keeps
  identical files and refuses differing files or unsafe paths. The installed
  list shows the project's existing skill folders; start a new agent session
  to load newly installed skills. Tracked skills show their source and commit (or
  a content hash for local skills). Use **Check update** or **Preview removal**,
  review the file diffs, then apply the single-use preview. Local edits and extra
  files block updates/removal. **Refresh source** reloads the catalog snapshot
  without restarting the runner.
- **Create skill:** write a name, a description of when the skill should be
  used, and Markdown instructions. Select a registered project and tools,
  then preview the generated `SKILL.md` and its destinations before saving.
  Creation works offline without Node.js or a model call. It shares the
  Skills installer's collision checks and never overwrites an existing file.
  The form draft survives section changes until the page is reloaded.

Worktrees start from the selected project's current committed `HEAD`; uncommitted
changes and untracked files stay in the original checkout. Git and at least one
commit are required. A blank branch name creates `codex/harness-<session-id>`;
an existing branch name is refused. For a project inside a repository, the same
subdirectory is used in the worktree and must exist in `HEAD`. The worktree and
branch remain after a session finishes or the server restarts, so results can be
inspected and committed. They live under the server state directory's `worktrees/`
folder; the session displays its exact directory and branch when created.
Follow-ups refuse a missing worktree or one belonging to a different repository.
Use **Results & usage → Worktree delivery** to commit selected files and transfer
a reviewed commit to a local project branch. Use normal Git worktree commands for
other worktree management. Project context in a run comes from its selected workspace; the separate
Memory bank and Skills sections continue to use the registered project directory.

**Results & Verification** compares the workspace with the commit recorded before
the first launch, including committed, staged, unstaged and nonignored untracked
changes. Pre-existing and external edits are included, so this is not an authorship
report. Older sessions without a recorded baseline use current HEAD explicitly.
Diffs are capped at 512 KiB / 500 files; large, binary or linked untracked contents
are omitted. Incomplete previews cannot establish verification freshness.

Checks run the exact command without a shell, in the same queue as agent runs.
They may modify the workspace; choose commands you trust. Output is bounded to
512 KiB, timeout is 1–3600 seconds, and **Cancel check** stops its process group.
A passing command proves only exit code 0. Checks keep the reviewed diff identity
and show when it has changed; ignored files and external services are outside that
comparison. Queued checks refuse a changed diff before executing. Check status
never replaces the agent's outcome. Creator retains its independent canonical
checks; its Results view aggregates launch usage across the run's phases.

Launch history and checks survive server restarts. History starts with this server
version; previous turns cannot be reconstructed. Totals include known usage plus
an explicit count of launches with unknown values, and remain after budget edits.
Check time is shown separately from model-launch time. Interrupted checks retain
their output and interrupted status after restart.

**Worktree delivery:** select pending files, enter a commit message and choose
**Preview selected files**, then **Commit reviewed files**. Pending files compare
with current worktree HEAD, separately from the full Results baseline. Selection
uses whole current files, including their staged and unstaged content. The preview
freezes the resulting Git tree; changed files, index or source HEAD invalidate it.
The commit preserves unselected edits and index entries. Git identity comes from
the repository/user configuration. Hooks and signing are disabled in this local
delivery path; use the Git CLI when those are required. Independent checks remain
explicit. An interrupted commit uses a journal to finish its index update on
restart when the recorded branch and index still match; unexpected external changes
block recovery for inspection instead of overwriting them.

Choose one source commit and an existing local target branch, then **Preview
transfer**. The runner rehearses a cherry-pick in a disposable detached worktree
and shows the resulting diff, recorded source checks and any conflicts. Only that
commit is transferred; preceding source commits stay separate. **Apply reviewed
commit** rechecks the source snapshot, target HEAD and checkout, then fast-forwards
the target to the prepared commit. The target worktree must be clean. Conflicts or
changed previews require a new review; ignored target files are not overwritten.
Unoccupied target branches use a temporary worktree without switching the primary
checkout. No push, remote fetch, branch deletion or source worktree removal occurs.
Source checks do not certify the resulting target tree. **Require a target check
before Apply** is selected by default: enter a trusted command and timeout before
previewing, then click **Run target checks**. The existing queue executes the command
without a model in a disposable detached worktree of the exact resulting commit,
from the selected project's relative directory. Dependencies and ignored files are
not copied; commands needing them must prepare their own environment. Output, exit
code, target branch/base and candidate SHA are saved in Results history. Cancel uses
the existing session cancellation; timeout, failure, cancellation, or changed tracked/
untracked files block Apply. Ignored build artifacts are allowed. A successful
check authorizes only its own preview; source/target changes still require a new
preview and check. Run only trusted commands: worktrees are not process sandboxes.

You may deselect the requirement before preparing a preview to deliver without a
target check; that choice is shown explicitly. The API accepts an optional `check`
object (`command`, `timeout`) in `preview_transfer`, followed by action `check` with
its `preview_id`. Apply enforces any selected check on the server. Preview state
survives page refreshes, but expires after 15 minutes (including checking time) or a
server restart; saved check logs remain. Results retain delivery commit IDs in
session events.

Delivery is available for ordinary session worktrees with their original branch;
Creator and current-project sessions retain their existing workflows.
Legacy sessions without a recorded baseline require another agent turn first.
Sparse worktrees and pending linked/special files are unsupported. Previews last 15
minutes, are single-use, and expire on restart. Lists show up to 100 non-merge
commits after the recorded session baseline. Pending changes are bounded to 500
files, 4 MiB per file and 64 MiB total; diff output is bounded to 512 KiB. Active
or queued sessions and checks block Git delivery through the existing global lock.

Custom skill names use up to 64 lowercase letters, digits and single hyphens.
Descriptions have up to 1,024 characters without angle brackets; instructions
have up to 32,000 UTF-8 bytes. The creator writes quoted YAML `name` and
`description` fields plus the Markdown body. The preview is invalidated when
the form changes. Supporting scripts and resources can be added separately
to the resulting skill folder when needed.

Skills discovery requires Node.js with `npx` and network access. It reuses
the Kit 3 pinned Vercel Skills manager in private temporary storage after
checking a bounded GitHub archive. Repository policy symlinks outside skill
folders are omitted; links inside skills are rejected. The cached source
snapshot stays fixed until **Refresh source**, a skill update check, or a server
restart. Public GitHub sources resolve HEAD through the [commit API](https://docs.github.com/en/rest/commits/commits#get-a-commit)
and download that exact commit; failed source checks do not report an update as
successful. Previewing does not change
the project; installation rechecks paths and conflicts and is available after
all agent sessions finish. Previews expire after 15 minutes and are single-use.

Selected files are copied to `.claude/skills`, `.agents/skills` (Codex), or
`.cursor/skills`. Other compatible tools may also discover `.agents/skills`.
Supporting files and executable permissions are preserved. This installs
standalone skills, not each upstream repository's plugins, hooks, or MCP
servers. Harness records source, revision and per-file hashes/permissions in its
private runner state. These receipts survive restart and manage updates/removal
through the UI; they are separate from Vercel's lock files and `kit3 update`.
Updates show additions, replacements, permission changes and removed upstream
files. Removal deletes only tracked files, keeping directories and other skills.
Before applying, the runner rechecks the receipt, project/directory identities and
all skill files, with the same active-session gate as installation. A local edit,
missing/extra file, symlink or hard link blocks the change. Diffs are bounded and
truncation is labelled. An interrupted update/removal reports completed writes and
does not advance tracking; inspect/restore the skill locally before retrying.

Older installations remain **Untracked**. Load the matching source and preview
installation to adopt an exact file-and-permission match; extra files prevent
adoption. Locally created skills are tracked by content hash and can be removed
without network access; they have no upstream update. Losing runner state loses
tracking, not installed skill files.
An interrupted installation may leave newly added files; refresh and preview
again to complete it. Existing files are never overwritten.

Source archives are limited to 16 MiB compressed and 64 MiB of files; one
installation is limited to 5,000 files and 64 MiB across selected tools.
Listings show up to 500 installed folders per tool. These bounds keep this
personal local server responsive.

**Model and thinking effort** can be selected when creating a session and
changed before the next message in an existing conversation. Active runs
keep their original settings. The model picker offers the provider's known
models and a **Custom model** entry for other IDs. **Provider default** adds
no model or ordinary effort override. Unsupported known
model/effort combinations are rejected before a run is queued.

Codex model choices and supported effort levels come from its local cached
catalog when available; the catalog is not a guarantee of account access.
Claude offers explicit versions, including Fable 5.1 and Opus 5, alongside
aliases labelled **auto**. Versioned entries send the full model ID;
auto entries let Claude choose the version according to its provider and
configuration. See [Claude model configuration](https://code.claude.com/docs/en/model-config)
for alias resolution and supported effort levels. Cursor has no
separate effort flag: select or enter its model variant, including an
`[effort=...]` suffix when that variant is supported by your account.
Model availability and custom model compatibility are ultimately checked
by the native provider. The picker does not fetch credentials or start a
model request to discover choices.

**Ultracode (workflows)** is available in Claude's thinking effort picker
for models supporting `xhigh` (Claude Code 2.1.203+). It combines `xhigh`
with native dynamic workflows, and requires **Use additional agents**.
The helper count is a required total in the prompt. It is not a native concurrency cap:
Claude bypasses its ordinary helper cap in Ultracode, and its workflow
runtime applies separate limits. Native permissions and workflow restrictions
still apply; disabled native workflows can leave only `xhigh` active.
Selecting another effort, including Provider default, explicitly disables
Ultracode and workflows for that turn, including on resume. This keeps
ordinary runs subject to their selected agent controls.

**Additional agents** are disabled by default; the main agent always runs.
Enable the switch and set **Required helpers** to require exactly that many
distinct helper launches on every turn, excluding the main agent. Each helper
must do a useful bounded subtask or independent check, and the model must wait
for and assess every result. Smaller tasks do not reduce the required count.
If native concurrency is lower, helpers must run in successive batches. Previous
turns, resumed helpers and repeated messages do not count as new launches.
If tools, permissions, task restrictions or remaining budgets prevent the full
count, the model must report the actual/required counts and explain the blocker.
The additional-agent choice is saved per session and reused on follow-up turns.
Between ordinary session turns, change **Use additional agents** or **Required
helpers** and send the next message to apply and save them. Active launches and
prepared context keep their settings locked; Fleet and Creator use their own
controls. Each launch retains its original helper settings in history.
Existing history migrates with helpers disabled.

Codex receives explicit native agent enable/disable and concurrency
settings, including its V2 main-thread offset. Claude disables its native
Agent/Task/Workflow tools when off; when on, it uses the native concurrent
subagent cap and limits spawn depth to one, and explicitly allows Agent/Task
tools for non-interactive requests. Explicit provider deny rules still apply.
Ultracode explicitly enables
the native Workflow tool; the same required total applies within native limits.
The values are passed per invocation,
without editing user or project configuration. These switches govern
native delegation tools, not every possible process an agent could run.
Cursor has no verified native CLI switch or numeric cap: both preferences
are passed in the prompt, and the UI explicitly labels them as instructions
rather than enforced limits. Enabling helpers can increase token usage.

The same requirement applies to Infrastructure Creator model phases. Fleet
continues to dispatch its selected reviewers directly, with nested delegation
disabled; its count still caps concurrent reviewers. A per-turn **Required helper
count confirmed / not confirmed** notice shows confirmed/required launches based
on native receipts, not the model's prose: Codex spawn completion with child IDs
or V2 `sub_agent_activity` start/completion receipts,
or Claude Agent/Task start/completion. Repeated receipts for one helper are
counted once. Only an exact match confirms the count; shortfalls and excess
launches are reported separately. A Workflow invocation alone does not prove
individual helpers ran; Cursor helper telemetry is not verified by the adapter.
Missing evidence is displayed without inventing success or automatically launching
paid retries. Native process completion remains separate from delegation evidence
and independent task verification.

Some Codex versions omit V2 helper activity from `exec --json`. Harness also
reads the matching thread's local journal to supplement partial stream receipts
through the read-only Codex state index. Only lifecycle metadata for the same
session, workspace and current launch/turn is accepted; earlier turns cannot
confirm a later one. Reads are bounded to the journal's last 4 MiB, and missing,
malformed or mismatched records remain unconfirmed. Model messages, prompts and
reasoning from the journal are never forwarded to the UI.

Plan/Review always use plan permissions. Codex uses its read-only sandbox
for Plan and workspace-write for Edit; Claude uses plan/acceptEdits;
Cursor uses plan/default agent mode with its sandbox enabled. These native
permission systems are not identical. The web UI does not implement an
interactive permission-approval relay or bypass native checks: a command
requiring unavailable approval can fail and must be handled in the CLI.

One run is active at a time, with a bounded queue of 16. This serializes
writes across registered projects. A configured time budget stops a run at its
deadline; a blank time field leaves it uncapped. **Cancel** still stops the run,
and a watchdog stops the process group if the server dies. After a restart,
unfinished sessions are marked interrupted and can be resumed if the
provider already returned a native session ID. Process completion means
the provider returned success, not that its code was independently tested.

**Budgets — money, tokens and time:** expand the section in Sessions or Creator.
Enter a USD cap ($0.01–$1,000), total-token threshold (1–1,000,000,000), and/or
elapsed time in whole seconds (1–86,400). Each empty field omits that limit,
including time. Settings persist with the session or Creator run.
Use **Save budgets** between launches or at a Creator checkpoint, then continue.
Active/queued runs keep their launch settings; stale edits are rejected.
For API clients that omit the entire `budgets` object, `--timeout` supplies an
initial time budget (900 seconds by default), returned in session settings.
Explicit `budgets.seconds: null` removes it. Browser forms always send their
budget fields explicitly, so a blank field never inherits a hidden deadline.

**Budget per agent** in session and Creator budget forms converts a per-agent
amount into the existing shared limits. Enter USD/tokens/time and click **Use
per-agent budgets**, then **Save budgets** for an existing session or Creator run.
For a new session, submit the resulting totals with the normal launch form. The
calculator never starts agents or saves on its own. It includes the main agent
plus allowed helper slots; disabling helpers counts only the main agent. Fleet
counts all selected reviewers, not just concurrent reviewers. USD and tokens are
multiplied by that count. Native time remains a shared deadline; Fleet sets the
reviewer timeout and multiplies it by `ceil(reviewers / concurrency)` for the
shared deadline. Setup also consumes this deadline, so allow extra total time
when needed. Blank fields clear shared caps; Fleet retains its separately
configured per-reviewer timeout.

The current totals also show equal shares (USD rounded down to six decimals,
whole tokens with an unallocated remainder). Native shares are planning guidance,
not independently enforced per-helper limits; actual helper counts may differ.
Fleet USD reservations can be smaller after spending/retries. Existing provider
support and aggregate limits still apply. Changing the calculator or agent count
does not overwrite totals until **Use per-agent budgets** is clicked again.
Calculated totals persist normally; the calculator inputs are a temporary draft.
Launch history retains the allocation used at launch, even after budgets change.


USD is supported by Claude's native `--max-budget-usd` flag. This is a stop
threshold, not a guaranteed billing ceiling: the final provider request may
overshoot it before cost is reported. Leave headroom when setting an allowance.
Overspending remains recorded, blocks successful review completion, and counts
against later Fleet allocations; it is never rounded down or retried automatically.
Codex and Cursor
have no verified monetary cap here. Native-session caps apply per turn; Creator
caps apply per scan/generate phase. Fleet's USD cap remains shared across
reviewers and retries, with prior costs/reservations retained when edited.
If an earlier uncapped call has unknown cost, a new capped Fleet is required.
Fleet token/time thresholds apply per graph invocation; reviewer timeouts remain
separate. Creator apply/rollback do not inherit model budgets.

Token accounting sums reported input and output, including cached input once
(Claude reports cache reads/writes separately; Codex cached input is a subset).
The runner stops when reported tokens reach the threshold, but CLI usage may
arrive only on completion: this is **not a hard pre-request token cap**, and an
invocation may exceed it. Unknown tokens/cost remain unknown. The panel shows
last-launch usage and any reached limit, retained when settings change and reset
only when the next process starts. Time limits terminate the owned process group.

History is stored in a private SQLite database under
`$XDG_STATE_HOME/ai-infrastructure-harness` (default
`~/.local/state/ai-infrastructure-harness`); `--state-dir` selects a separate
instance. State and logs are local and are not installed into projects.
Keep that directory private: prompts and agent answers are retained there.

The server binds only to `127.0.0.1`. Browser mutations require a
per-process token and matching Origin/Host; static routes do not expose
repository files. This is a personal local tool, not a remotely hosted or
multi-user service. Fleet review runs the existing LangGraph graph in a separate
local Python process, with the same queue and workspace checks as native sessions.

### Connect and prepare a project

Open **Projects & Setup** and choose **Browse…** to navigate local folders, or
enter an absolute path directly. The folder picker provides Home, Parent folder,
hidden folders, and a case-insensitive name search through subfolders. Choose
**Use this folder**, then **Add project**. It browses the computer running the
server; no project files are uploaded or read. Recursive search skips dependency
directories (`node_modules`, `vendor`, virtual environments and Git metadata),
is limited to 8 nested levels, 20,000 entries, 200 results and 1.5 seconds,
and reports partial results so you can narrow the location. Symbolic links are
omitted; unreadable subfolders are skipped. Registration does not create or change project files. Paths containing
symbolic links are refused. An unavailable saved directory remains visible for
diagnosis after a restart. Readiness shows the Git branch and commit, the detected
provider CLIs and installed policy/context files; CLI presence does not establish
authentication. Setup does not inspect working-tree changes or run project Git
filters; the session workspace controls retain their full Git status check.

Choose an edition and one or more tools. Preview runs the existing accelerator
installer in private staging, using its versioned inventory and merge rules.
Review the file actions, diffs and any collisions before installing. Supported
root-file merges retain project policy and ignore entries; an existing project
README is kept and the accelerator README goes to `ACCELERATOR.md`. Other differing
files block installation. Truncated diffs are marked, and every selected file is
listed. The source checkout and server storage cannot be installation targets.

Installation consumes the reviewed preview and rechecks the source, target and
file contents. A changed file or expired preview requires a new preview. Active
agent sessions block installation. If filesystem failure interrupts publication,
the response reports completed writes; inspect the result and preview again before
retrying. The installer never reports that a partial operation succeeded. After
installation, inspect readiness and use the action to start a session in that
project. No model is called by registration, preview or installation.

### Project Brain and Memory Bank

Choose a registered project and its bank before using either view. Source editions
in this repository have separate banks; an installed accelerator uses the consuming
project's root. Browse operations only read bounded canonical files. The UI remains
a viewer when `memory-bank/scripts/context.py` is missing.

Explicit operation buttons invoke that selected runtime with fixed arguments:

- **Search** reads the existing local full-text index. Run **Index** after source
  changes or when no index has been built. Results can include other project
  context, not just durable memory chunks.
- **Status**, **Validate** and **Audit** report index counts, Brain validity and
  chunk source freshness. Even these CLI commands may create the local SQLite
  database; opening the viewer does not.
- **Reindex bank** regenerates `memory-bank/INDEX.md` from chunk metadata.
  **Reverify** records the operator's attestation that the chunk was checked
  against its sources. It does not independently verify the meaning of a claim.
  **Retire** archives a chunk or marks it superseded by its replacement.
- **Start task**, **Create record**, **Update record** and **Complete task** use
  the project's ownership, lifecycle, privacy and revision checks. Next-step
  entries are appended; progress is replaced. Completion requires a current
  numeric revision and an explicit outcome. Handoffs follow the runtime's task
  lifecycle rather than a separate editor.
- **Compact** moves terminal Brain records into its archive, preserving history.
  **Export** calls the CLI's privacy-filtered exporter and offers a ZIP download.
  It never accepts an arbitrary output path or overwrites a previous export.

The Harness adds no second knowledge database and does not directly edit records
to bypass runtime validation. Raw file editing and import are not exposed by these
controls. Export downloads are temporary server artifacts.

### Link a session to a task

Open **Brain task & retrieved context** when creating a session, enable linking, select a bank and an
existing task or enter a new task ID and goal. **Prepare session** creates the
workspace first, binds the task there and retrieves a bounded context capsule.
Inspect the capsule before choosing **Run with this context**. The provider receives
that saved capsule; the server checks the task revision and source contents again
before launching it. Changed context requires a fresh preview. Each chat follow-up
also prepares a new capsule. These explicit retrievals disable the runtime's
repeat-query heuristic while retaining its privacy and source eligibility rules.

Task actions stay in the session's original workspace and bank, including when a
worktree is used. A committed task can be rebound to a rebuilt local index without
creating a second task. The reviewed capsule survives server restarts in session
history; canonical task and knowledge records remain owned by the project runtime.
Offline Fleet demonstrations cannot link a real task.

After a successful run, explicitly enter the task outcome and verification to
complete it at its current revision. Process success never completes the task
automatically. To retain reusable knowledge, create and verify a separate finding
or decision, resolve or accept it, then propose its content for Memory Bank review.
Only eligible verified records with fresh sources and allowed privacy can be
proposed or applied. Task records cannot be promoted. Manual proposals require an
independent reviewer and retain the runtime's source revision checks. The project's
automatic promotion setting is displayed and remains unchanged. Prompts, transcripts
and assistant output are never automatically copied into durable memory.

### Fleet review setup and use

Install its declared dependencies once from the repository root (Python 3.10+):

```bash
python3 -m venv harness/.venv
harness/.venv/bin/python -m pip install -e harness
```

Restart the browser server after installation. A different prepared Python can
be selected with `HARNESS_FLEET_PYTHON=/absolute/path/to/python`. Ordinary sessions
remain available when this optional runtime is missing.

1. Choose **Workflow → Fleet review**, a project/worktree and a provider.
2. Select reviewers and enter the review scope in the task field. For real
   reviews enable **Use additional agents**; its count caps concurrent reviewers.
   Each reviewer gets the selected model/effort. Ultracode is unavailable because
   this graph manages its own reviewers. **Offline dry-run** needs no authenticated
   CLI and does not execute the project's memory runtime.
3. Set a per-reviewer timeout if needed. **Budgets → Time** overrides the server
   timeout for that graph invocation. An optional USD budget is available
   for Claude; Codex/Cursor monetary cost is shown as unknown when unavailable.
4. Review the findings at **Awaiting approval**, then **Approve report** to save
   and download Markdown, or **Reject report** to finish without publishing it.
   The preview is visible before approval. Reports stay in local server storage.

Checkpoints live under `state/fleet/<session-id>/`. A waiting review survives
server restarts. **Resume** on an interrupted/failed/cancelled run continues from
its checkpoint; successful reviewer nodes are retained. Provider failures stop
the graph and are not reported as code findings. Session settings stay fixed;
start another review to change its scope, reviewers, provider, or workspace.

Real runs use the selected workspace's `memory-bank/scripts/context.py`, when
installed, for task creation, capsule validation, dispatch events and progress.
This writes Project Brain audit records; it does not create a second project
knowledge store. Without that runtime the review runs without the project audit
trail. Graph status, findings and approved reports remain in private server state.

A finite budget is divided among reviewers and passed to Claude's native budget
flag. The browser reserves allocations before launching a reviewer and stores
them across retries. Known costs release unused allocation; interrupted calls
with unknown cost retain their allocation. A retry can therefore be refused
when no budget remains. Reported costs include metered failed attempts when a
reviewer later succeeds. Native provider accounting determines actual charges;
an over-allocation result fails the reviewer instead of claiming the limit held.

Verification from the repository root:

```bash
python3 -m unittest tests.test_harness_providers tests.test_harness_sessions tests.test_harness_web tests.test_harness_process_guard tests.test_harness_skills tests.test_harness_fleet tests.test_harness_knowledge tests.test_harness_task_context tests.test_harness_setup tests.test_harness_creator tests.test_harness_results tests.test_harness_delivery tests.test_harness_clash
harness/.venv/bin/python -m unittest discover -s harness/tests -p 'test_*.py'
```

The Fleet integration tests use the optional runtime and synthetic/fake providers;
they skip when that runtime is absent. CI has a separate job that installs it.

The browser workflow was informed by the official
[DeepSeek Harness web mode](https://www.deepseek.com/harness/en/).
This implementation uses locally installed native CLIs rather than
vendoring DeepSeek's runtime or plugins.

## Optional LangGraph batch runner

External LangGraph batch harness for the AI-Infrastructure accelerators:
unattended multi-stage pipelines (nightly fleet review, mass migrations,
scheduled research) over headless coding-agent hosts.

The batch runner is **Stage D** of the agent-orchestration design
([`../docs/AGENT-ORCHESTRATION-DESIGN.md`](../docs/AGENT-ORCHESTRATION-DESIGN.md))
and a deliberate **companion, not a component**: it lives at the monorepo
root, OUTSIDE the editions — the installer never ships it, the inventories
never list it, no edition requires or depends on it, and the accelerator's
own runtime stays stdlib-only. This directory (with its own venv) is where
the LangGraph dependency is allowed to live.

## What it does

```
harness run --project /path/to/project --scope "HEAD~5..HEAD"
# ... runs lenses in parallel, then:
# PAUSED at the approval gate: {"findings": 7, "high": 2, "cost_usd": 1.84}
harness resume --project /path/to/project --thread fleet-review-... --approve
```

The `fleet-review` graph: scope → parallel review lenses (`Send` fan-out)
→ collect → **human approval gate** (`interrupt`, waits indefinitely across
processes) → report + record. Durable execution via the SQLite checkpointer:
a crashed or paused run resumes from its checkpoint with `--thread`.

## Architecture rules (from the design doc)

- **The blackboard is the project's own project-brain.** Nodes write and
  read through the target project's `context.py`: task lifecycle, capsule
  validation (`capsule --validate` refuses under-specified delegations),
  the message channel (`msg-dispatch` spawn/complete with SHA-256 capsule
  digests). LangGraph's checkpointer holds only graph position — interactive
  sessions and unattended runs share one audit trail.
- **Workers are headless host sessions.** The default `claude-cli` worker
  runs `claude -p --output-format json` in the project directory — the
  officially documented subprocess pattern — deliberately **without**
  `--bare`, so the project's `.claude/` world applies inside every worker:
  permissions deny rules, the subagent gate (roster-only spawning, write
  serialization), skills, and the SubagentStop observer. The worker prompt
  delegates to the project's own roster agent. `codex-cli` (`codex exec
  --json`, read-only sandbox) is available; the batch runner has no Cursor adapter on
  purpose — its headless mode has documented hangs.
- **Cost is a first-class signal.** Known worker costs accumulate in graph state;
  unknown cost stays unknown. `--budget-usd` is optional (default `none`), supported
  by Claude and dry-run. A finite budget is split across reviewers rather than
  copied to every parallel branch. Claude receives a native per-call budget limit;
  Codex rejects finite USD limits. The legacy CLI limit applies to a graph attempt;
  the browser additionally reserves allocations durably across failed retries.
- **`dry-run` worker** rehearses any pipeline offline (deterministic canned
  findings, no host, no tokens) — used by the test suite and recommended
  before every new pipeline.

## Install

```bash
python3 -m venv .venv && .venv/bin/pip install -e ".[dev]"
.venv/bin/python -m pytest            # offline graph and worker tests
```

Requires Python ≥ 3.10. The target project needs the accelerator installed
(for the blackboard); without `memory-bank/scripts/context.py` the harness
still runs, minus the audit trail.

## Auth for unattended runs

Subscription OAuth does not survive CI: use `ANTHROPIC_API_KEY`, or mint a
long-lived token with `claude setup-token`. Note Anthropic's policy: products
must not offer claude.ai login or rate limits — internal pipelines on your
own subscription are fine. Parallel workers share the subscription's rate
bucket; throttle accordingly (`--worker-timeout`, fewer lenses).

## When to use the batch runner

Interactive accelerator orchestration belongs in the accelerator's own `/flow-*` commands (Stage
A–C) — the main conversation is the orchestrator there, with checkpoints the
user answers in place. Short single-host scripts are simpler with the Claude
Agent SDK alone. This harness earns its dependencies only for long,
resumable, multi-stage unattended runs with human gates.
