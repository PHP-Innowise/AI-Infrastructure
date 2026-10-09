# Attached mode: use the accelerator without copying it into the project

Clone this repository once. Point the Harness (or a terminal launcher) at any
PHP project folder. Every session in that project then gets the matching
accelerator edition - policy, skills, agents, commands, hooks and memory - from
the clone, and **nothing is copied into the project**. `git pull` in the clone
updates every attached project at once.

This is the default for every project chosen in the Harness. Installing the
edition into the project (**Accelerators › Install into project**, or
[install/README.md](../install/README.md)) remains available for a team that
wants the accelerator's files in its own Git history.

## Use it from the browser (Harness)

```bash
git clone <this repository> ~/ai-accelerator
cd ~/ai-accelerator
./harness-server start
```

Open the printed address (`http://127.0.0.1:8766`). Started from the clone with
no `--project`, the Harness opens on an empty session that asks for a project,
as DeepSeek Harness asks for a workspace: choose **Choose a project folder** (or
the **Project** chip, or **＋ Choose a project folder…** in the sidebar
**Project** selector), browse to the folder and choose **Use this folder**. The
folder is registered and opened in a new session draft, and the edition its own
files point to is attached:

| Evidence in the project | Edition |
| --- | --- |
| `laravel/framework` in `composer.json`/`composer.lock`, or an `artisan` file | Laravel |
| `symfony/framework-bundle`, or `bin/console` with `config/bundles.php` | Symfony |
| WordPress packages or package type, `wp-config.php`, `wp-content/`, a theme or plugin header | WordPress |
| any other `composer.json` or PHP file at the root | PHP Core |

The sidebar shows `shop · Laravel attached` and the **Project** chip
`shop · Laravel`. The chip's panel names the clone the edition is attached from
and the directory that holds its memory, and switches or detaches the edition.
A project without PHP evidence gets no edition; choose one in that panel. A
project that already has an installed accelerator keeps using its own files.

Start a session as usual. The `/` menu (Claude Code, Cursor) and the `$` menu
(Codex) include the edition's commands and skills, and **Memory** works on the
attached state.

To skip the terminal next time, run `./accelerator-app install` once in the
clone. **AI Accelerator**, with the hare icon, then appears among the installed
applications, and a click starts the server and opens this page; see
[Desktop application](../harness/README.md#desktop-application).

## Use it from a terminal

From the project folder:

```bash
python3 ~/ai-accelerator/scripts/accelerator_attach.py run claude
python3 ~/ai-accelerator/scripts/accelerator_attach.py run codex
python3 ~/ai-accelerator/scripts/accelerator_attach.py run cursor
```

Arguments after `--` go to the CLI unchanged
(`... run codex -- exec "Review the routes"`). `detect` prints the edition the
project points to; `--edition` overrides it. `env` prints the three variables
for another launcher. The terminal and the Harness use the same state directory
for a project, so they share its memory; `--state-base DIR` keeps it elsewhere,
and a relative `DIR` is taken from the directory the launcher runs in.

## What each tool receives

Verified against Claude Code 2.1.278, Codex CLI 0.160.0 and Cursor Agent
2026.09.02 on 2026-10-06; see [Evidence](#evidence).

| | Claude Code | Codex | Cursor Agent |
| --- | --- | --- | --- |
| Skills, commands, subagents | `--add-dir <edition>`: loads `.claude/skills`, `.claude/commands`, `.claude/agents` under their own names (documented) | Codex has no setting for an extra skills folder, so the skill catalogue (name, description, `SKILL.md` path) goes into `developer_instructions`, the way Codex lists skills itself | `--plugin-dir <edition>` loads `.cursor-plugin/plugin.json`, which points at the edition's own `.cursor` rules, skills, agents and commands |
| Policy (`AGENTS.md`) | `--append-system-prompt-file` (a preamble plus `AGENTS.md`) | `-c developer_instructions=...` (preamble, skill catalogue, `AGENTS.md`) | prompt preamble naming `AGENTS.md`, plus the edition's always-applied rules |
| Hooks | `--settings` with the edition's hooks rewritten to absolute paths into the clone | `-c hooks.<Event>=...` with absolute commands; Codex runs them only after a one-time approval (below) | the plugin's `hooks` field points at the edition's `.cursor/hooks.json` |
| Permissions and environment | `--settings` merges the edition's `allow`/`deny` and `env`, and denies edits inside the clone | sandbox and agent flags as before; `--add-dir <state>` makes the state writable | as before |

Every launch also exports `ACCELERATOR_HOME` (the edition in the clone),
`ACCELERATOR_STATE_DIR`, `ACCELERATOR_PROJECT_DIR` and `ACCELERATOR_EDITION`.
The preamble tells the agent that paths such as `.claude/skills/...`,
`project-brain/templates/...` and `memory-bank/scripts/...` are relative to
`ACCELERATOR_HOME`, and to run the context runtime as
`python3 "$ACCELERATOR_HOME/memory-bank/scripts/context.py" ...` from the
project.

### Codex hook approval

Codex runs a hook only after its definition was approved once; the approval is
a hash under `hooks.state` in `~/.codex/config.toml`, the same record Codex's own
`/hooks` review writes. A session-flag hook has no `/hooks` entry to review, so
the Harness records the edition's seven hooks by itself, once per version of the
clone, when Codex is available (`HARNESS_CODEX_HOOK_TRUST=0` turns that off);
**Sessions › Project › Trust accelerator hooks in Codex** and
`accelerator_attach.py trust-codex-hooks` record them on request.
Moving the clone changes the commands and needs a new approval; a `git pull`
that edits only hook scripts does not. Without the approval Codex still gets the
policy and skills; only the hooks (session banner, automatic memory, command
and file-name guards) do not run.

## Where the memory lives

The accelerator's per-project state - Project Brain records, Memory Bank
chunks, the local index, retrieval manifests - is kept in
`<Harness state>/attached/<project id>/`, by default
`~/.local/state/ai-infrastructure-harness/attached/<id>` on Linux and macOS and
`%LOCALAPPDATA%\ai-infrastructure-harness\attached\<id>` on Windows. The
project id is the one the Harness lists for the project. The runtime lays the
directory out on first use from the edition's defaults; detaching keeps it, so
attaching again picks the memory up.

On Linux and macOS the state is its owner's alone - directories 0700, files
0600 - whatever the umask: the launcher and the runtime create it that way, and
tighten a state made before, where it is the accelerator's (never through a
link, never above the state directory).

This state belongs to one machine. A team that wants Project Brain and the
Memory Bank shared through Git installs the edition instead.

Work products that a workflow asks for - `specs/`, `tasks/TASK-NNN/`,
`codebase/` - are still written into the project, like any other change the
agent makes there.

## Limits

- **Live coverage.** On the verification machine only Codex was signed in. The
  Codex path ran end to end from the browser: the model named the attached
  edition, its `AGENTS.md`, the state directory and the right skill, followed
  the policy's Context Summary rule, and project memory was saved to and
  retrieved from the attached state. For Claude Code, the CLI's own
  `initialize` listing (113 commands, 44 agents from the edition) and the
  hooks were verified, but no model turn ran. Cursor Agent was not signed in;
  its plugin loading follows Cursor's documentation and is untested.
- **Other tools still write.** Plugins installed in a user's own CLI keep
  writing wherever they write: on the verification machine two Codex plugins
  (`agentic-security`, `agentops`) created `.agentic-security/` and `.agentops/`
  in every project they ran in. The accelerator itself writes nothing there.
- **One edition per project.** A project attaches one edition at a time. Every
  Harness launch carries it - Workspace, Plan, Review and SDD sessions, Clash
  participants and Fleet reviewers; Infrastructure Creator and System
  Orchestration runs have their own workspaces and do not use it.
- **Windows.** Hook commands run through `bash`, as installed hooks already
  require (Git Bash).

## How it works (for maintainers)

The runtime and hooks used to assume one root that held the tooling, the
project and the state. [`workspace_roots.py`](../Laravel/memory-bank/scripts/workspace_roots.py)
separates them:

| Root | Installed | Attached |
| --- | --- | --- |
| tooling | the project root | `ACCELERATOR_HOME` - the edition in the clone |
| state | the project root | `ACCELERATOR_STATE_DIR` (`--root`) |
| project | the project root | `ACCELERATOR_PROJECT_DIR` |

Attached mode needs all three variables, with `ACCELERATOR_HOME` naming the
runtime's own edition and `ACCELERATOR_STATE_DIR` naming the state root in use;
anything else is the installed layout, so a stray export cannot move an
installed project's state. Index keys show their root by shape: an absolute
path is tooling (so the accelerator's `AGENTS.md` never collides with a
project's own), `project-brain/...` and `memory-bank/...` are state, and every
other key is a project path. Git always runs in the project. Hooks keep
`ROOT_DIR` as the tooling root and add `PROJECT_DIR` and `STATE_DIR`.

[`scripts/accelerator_attach.py`](../scripts/accelerator_attach.py) is the
shared contract: edition detection, the state directory, and the per-tool
overlay. The Harness applies it in
[`harness/src/harness/accelerators.py`](../harness/src/harness/accelerators.py).

Tests: `memory-bank/tests/test_attached.py` in every edition (the three roots,
hooks run from the clone), `tests/test_harness_accelerators.py` (registration,
launch overlays, Knowledge on the attached state).

## Evidence

- Claude Code: `--add-dir` loads skills, commands and subagents from
  `.claude/` of the added directory; hooks and permissions come only from
  settings, not from the added directory
  ([permissions](https://code.claude.com/docs/en/permissions.md#additional-directories-grant-file-access-not-configuration),
  [skills](https://code.claude.com/docs/en/skills.md)). Probed with the CLI's
  `initialize` request: from a project with no accelerator files, 113 commands
  and 44 agents with the edition attached, against 63 and 6 without; hooks from
  `--settings` ran in `--print` mode.
- Codex: no configuration key adds a skills folder (`--strict-config` rejects
  `skills.extra_roots` and similar); `developer_instructions` reaches the model
  in `codex exec`; `-c hooks.*` hooks are listed as `sessionFlags`, are
  untrusted until approved, and do not run untrusted in `codex exec`;
  `config/batchWrite` of `hooks.state` makes them trusted.
- Cursor: plugin manifest fields `rules`, `skills`, `agents`, `commands`
  (string or array) and `hooks`
  ([plugins reference](https://cursor.com/docs/reference/plugins)); the CLI's
  `--plugin-dir` loads a local plugin directory.
