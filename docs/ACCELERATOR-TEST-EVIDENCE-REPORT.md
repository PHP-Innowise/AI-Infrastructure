# Accelerator Test Evidence Report

Prepared per `TEST-REMEDIATION-AGENT-TASK.md` (evidence collection only — no fixes implemented by
this task, no accelerator sources modified, nothing staged/committed/pushed, workbook not edited).

## 1. Submission

- Tester: Aliaksei Brutau (repository owner). Execution vehicle recorded in the workbook:
  "Claude Fable 5 (scripted execution)" for RUN-001..RUN-049 and "Claude Fable 5 (live AI session)"
  for RUN-050..RUN-057 — both driven by one Claude Code session on the tester's machine.
- Team: Unknown (not recorded in the workbook).
- Report created: 2026-08-04.
- Repository: `~/Desktop/AI-Infrastructure` (monorepo "PHP AI Accelerators"), branch
  `merge/context-brain-reconciliation`, HEAD `3a025374b1057652e336fac4ca2cc81db9b7ede8` (short `3a02537`).
- Workbook: `Accelerator TestCases.xlsx`. Two copies used as sources: the current Google Sheets
  export (2026-08-04 19:35 local) and the tester's local historical snapshot
  (`~/Downloads/Accelerator TestCases.xlsx`, mtime 2026-08-03 19:56:06 local) — the latter is the
  state at the end of the testing day and is attached as
  `manifests/workbook-snapshot-20260803-2002.xlsx` (byte-equal scratchpad copy, mtime 20:02).
- Assigned Run IDs: **RUN-001 .. RUN-057** (57 runs; all runs executed in this environment).
  The 18 `RUN-20260803-*` rows in the current workbook were executed by a different tester
  ("OpenAI Codex", commit `7c1cce290a…`) — that commit does not exist in this repository, those runs
  were not executed here, and they are out of scope for this submission (see §4.58).
- Evidence root: `~/Downloads/Accelerator-TestEvidence/` (original, unmodified);
  submission copy in this package under `logs/original/`.
- AI agent/tool used to collect evidence: Claude Code (model Claude Fable 5), session
  `57711d9c-af25-44ca-bc15-d566d6138e97`, 2026-08-04, on the same machine that executed the runs.

## 2. Environment Inventory

### Environment E-01 — scripted runs (RUN-001..RUN-049)

- OS: CachyOS Linux (Arch-based), kernel `6.18.40-1-cachyos-lts`, host `thinkpad`.
- Shell: bash 5.3.15 (execution shell of the runs; the user's login shell is fish).
- PHP: 8.5.8 (cli, NTS) — printed inside the run logs themselves (e.g. `TC-001.log`).
- Python: 3.14.6 as measured 2026-08-04 on the same machine; the interpreter version was not
  separately recorded inside most 2026-08-03 logs.
- Framework: none (edition workspaces of the monorepo itself: Laravel/, Symfony/, "PHP Core"/,
  Infrastructure-Creator/); framework version column records "n/a (edition workspace)".
- AI client: none at case level — scripted execution (bash/python) orchestrated by a live
  Claude Code session (v2.1.219, session `c9fab6de-739d-497c-ac0d-61bda631def9`, models
  `claude-fable-5` orchestrating / workflow subagents per journal).
- Model: n/a at case level (see above).
- Accelerator version: 1.4.3.
- Git commit: `3a02537` (= current HEAD).
- Operating mode: Governed.
- Project size classification: Small (workbook column).
- Repository clean at run time: **Yes** — `TC-001.log` prints
  "Branch: merge/context-brain-reconciliation (0 uncommitted changes)"; the orchestrating session
  verified `git status --porcelain | wc -l` = 0 before execution (transcript, 08:14:56Z).
- Execution substrate: the real repository for read-only checks; **temp copies** for every mutating
  step; Infrastructure-Creator fixtures generated into temp dirs.
- Real client or scripted: scripted (no native AI client exercised in these 49 runs).

### Environment E-02 — live AI-session runs (RUN-050..RUN-057)

- OS/shell/PHP/Python: same machine as E-01; PHP 8.5.8, Symfony 6.4.42 (fixture project).
- Fixture: isolated QA copy of the client project *bauherrenmappe* at commit `627d11f`, plus
  accelerator install commit `8d6a38b` ("Install Symfony accelerator edition"), located in the
  testing session's scratchpad (`…/c9fab6de…/scratchpad/bwb-run`). Client working tree untouched.
- AI client: live Claude Code agents (Claude Agent SDK workflow subagents), workflow
  `run-accelerator-on-bwb` (`wf_af500d03-304`), model `claude-fable-5`.
- Accelerator version: 1.3.1 (version installed into the fixture); Git commit of the accelerator
  repo at run time: `3a02537`.
- Operating mode: Governed. Project size: Medium.
- Repository state: fixture pre-run `git status` snapshots recorded in each log; accelerator
  monorepo untouched by these runs.
- Real client or scripted: real Claude Code agent sessions (per-case logs record the skill/command
  workflows the agents actually followed).

### Environment E-03 — evidence-collection reproduction (2026-08-04, this report)

- Same machine; HEAD `3a02537` but **dirty tree**: 57 uncommitted files produced by the post-test
  remediation workflow `fix-qa-findings` on 2026-08-03 15:31–16:28 local (see §6 note and
  `transcripts/wf_9ab92e4f-965.json`). Reproduction results therefore reflect the
  post-remediation code state, not the tested state, and are labeled accordingly.

## 3. Executive Evidence Summary

- Runs assigned: 57 (RUN-001..RUN-057).
- Original evidence available: 57/57 original per-case logs preserved
  (`~/Downloads/Accelerator-TestEvidence/TC-*.log`, mtimes 2026-08-03 11:31 and 15:14 local);
  56 attached, 1 withheld as sensitive (TC-074 — see §8/§4).
- Newly reproduced: 1 consolidated re-run (2026-08-04) of all 16 mandatory gates — 7 unittest
  suites (833 tests), parity + cross-edition ×3 editions, mirrors `--check`, links, token budget —
  all exit 0, zero repository mutation (`logs/reproduction-20260804/`, `99-summary.json`).
- Evidence unavailable: temp copies/fixtures used by mutating scripted steps (deleted with /tmp);
  no screenshots were ever captured (headless scripted execution); native Cursor/Codex client
  behavior never exercised in this environment (see §8).
- Evidence withheld: 1 artifact (`TC-074.log`, client security review) — sanitized replacement
  provided (`transcripts/RUN-057-TC-074-sanitized.md`), SHA-256 of the withheld original recorded.
- Native-client runs completed: 8 (RUN-050..RUN-057, live Claude Code agents on the bwb fixture).
- Runs with contradictory records: 8 (RUN-050..RUN-057 exist in the 2026-08-03 workbook snapshot
  and in the evidence logs, but were dropped from the current workbook), plus 4 Defect-ID
  collisions (DEF-001..DEF-004 reused by foreign Codex-run defects) — details in §6.
- Runs with missing Test Case definitions: 48 of the 49 surviving runs reference TC IDs no longer
  defined in the current workbook (only TC-001 survived); all 57 definitions recovered from
  attached original sources — see §5.

## 4. Run Evidence

One subsection per assigned Run ID, in run order. Sections were drafted from the original
logs, both workbook versions, the harness definitions and the workflow journals, then
adversarially re-verified against those sources. Log excerpts are quoted verbatim.

### RUN-001 / TC-001

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)"); Tester: "Claude Fable 5 (scripted execution)"
- Accelerator version: 1.4.3
- Git commit: 3a02537
- Execution date: 2026-08-03 (historical snapshot row; current workbook row coerced to "2026-08-03 00:00:00")
- Defect reference: None
- Workbook evidence link: Accelerator-TestEvidence/TC-001.log
- Row presence: exists in BOTH the current workbook and the historical snapshot. Content is verbatim-identical except mechanical coercion in the current workbook (Execution Date rendered as datetime; count columns "9"/"0" rendered as "9.0"/"0.0").

#### Test Definition
- Definition status: Authoritative — original source found (cases_map.json TC-001 and the workbook's original 74-row Test Cases sheet agree field-for-field after whitespace normalization).
- Title: Claude Code activation banner - Laravel edition (VERSION, loop-counter reset, validation cache)
- Objective: Verify the SessionStart local-context hook of the Laravel edition: banner content, version sourced from VERSION, git status line, artisan-absence NOTE, session loop-counter wipe, metadata-only context-governance and memory-bank lines, validation-result caching, and unconditional exit 0. Category: Installation and activation; Priority: Critical.
- Preconditions: Laravel/ edition opened as the workspace root; .claude/settings.json wires SessionStart -> .claude/hooks/local-context.sh (timeout 10); hook scripts executable; git repository initialized; python3 available; Laravel/VERSION contains 1.4.3. Loop-counter reset test pre-seeds a counter file: `REPO_KEY=$(git rev-parse --show-toplevel | tr -d '\n' | cksum | cut -d' ' -f1); mkdir -p /tmp/claude-loop-detection-$REPO_KEY && touch /tmp/claude-loop-detection-$REPO_KEY/seed`.
- Input: Start a new Claude Code session in the Laravel edition root (run `claude`). Deterministic path: `bash .claude/hooks/local-context.sh` executed from the edition root.
- Mandatory assertions (expected behaviour items 1–8, 10): 'Project Context' banner prints; 'Accelerator version: 1.4.3' read from VERSION; git line 'Branch: <branch> (N uncommitted changes)'; NOTE 'No artisan file found...' for the accelerator folder itself; all files under /tmp/claude-loop-detection-$REPO_KEY deleted; context-governance line prints metadata only; memory-bank summary line ends with 'Read memory-bank/README.md and INDEX.md before relevant durable-memory work.'; brain/memory validation results cached under ${TMPDIR:-/tmp}/ai-accelerator-session-cache-$EDITION_KEY and served from cache on a second unchanged run; hook exits 0 in all cases.
- Optional assertions: None explicitly marked optional in the source definition.
- Scriptable assertions (expected verification): `cd Laravel && bash .claude/hooks/local-context.sh | head -3` -> lines 'Project Context', '===============', 'Accelerator version: 1.4.3'; grep of the banner against `$(tr -d '[:space:]' < VERSION)`; `find /tmp/claude-loop-detection-$REPO_KEY -type f | wc -l` -> 0 after the run; `ls ${TMPDIR:-/tmp}/ai-accelerator-session-cache-*/brain-validation-*` -> at least one cache file; second run noticeably faster with the same 'brain-validation=valid'; `echo $?` -> 0.
- Native-client assertions: item 9 — in the interactive client, typing '/' shows installed commands including /verify, /memory, /project-brain (per docs/TOOL-INTEGRATIONS.md 'Claude Code Activation'). Execution type: Hybrid.
- Expected state changes: None inside the repository. Temp state only: /tmp/claude-loop-detection-<key>/ emptied; ${TMPDIR:-/tmp}/ai-accelerator-session-cache-<key>/brain-validation-<cachekey> and memory-summary-<cachekey> created.
- Safety expectations: The banner must never print, index, or inject Memory Bank / Project Brain record contents (metadata only). No files inside the repository working tree are created or modified. No network access. The hook must never exit non-zero. The loop-counter wipe must only touch this repository's counter directory.
- Requirement-source: Laravel/.claude/hooks/local-context.sh (also Laravel/.claude/settings.json 'SessionStart' block; docs/TOOL-INTEGRATIONS.md 'Claude Code Activation').

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: /home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-001.log (shareable name: logs/original/TC-001.log), file mtime 2026-08-03 11:31:41 +0300; byte-identical to the executor's scratchpad copy per the execute-test-cases digest (md5-verified for TC-001).
- Commands: the log does not echo the invoked command lines. Per the workbook row: "Ran bash .claude/hooks/local-context.sh from Laravel/ edition root twice." The pre-seed command is specified in the case preconditions (quoted above). The log corroborates the pre-seed and both runs via its markers "--- seeded counter: seed", "--- run 1 (cold) ---", "--- run 2 (warm) ---".
- Exit codes (verbatim from log): `exit1=0` (run 1), `exit2=0` (run 2).
- Assertions: 9 executed / 9 passed / 0 failed (workbook counts). Interactive item 9 skipped — native client not exercised (see Native-Client Evidence).
- Key output excerpt (verbatim, non-contiguous lines from TC-001.log):
  ```
  Accelerator version: 1.4.3
  Branch: merge/context-brain-reconciliation (0 uncommitted changes)
  Context governance: mode=governed, index=healthy/stale, active-bindings=0, brain-validation=valid.
  counter files after run: 0
  brain-validation-436674262
  cold_ms=945
  warm_ms=292
  ```
- Contradictions with workbook: none material. Two nuances: (1) the log stderr contains "bash: line 20: bc: command not found" and the inline `elapsed1=` / `elapsed2=` values are empty — a harness-side timing helper was missing; timing was still captured by the separate "--- timing check (ns) ---" block (cold_ms=945 / warm_ms=292), which is what the workbook cites; the bc error is not mentioned in the workbook row. (2) The governance line shows index=healthy/stale where the definition's illustrative text shows healthy/current; the workbook Comments field discloses this as checkout-state-dependent with the metadata-only format contract matched.

#### New Reproduction
- Performed: No (per-run scope). A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run.
- Historical retries: none recorded (workbook Retry Count 0; log shows a single cold/warm run pair as designed).

#### Native-Client Evidence
- Run executed in scripted mode (bash/python driven by a live Claude Code session); native Claude Code client behavior was NOT exercised. The case's native-client assertion (item 9: '/' menu listing /verify, /memory, /project-brain) was not executed — Native assertions unavailable. The workbook Comments field records this caveat explicitly.

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium. The original log exists, the tested commit (3a02537) is recorded in the workbook and corroborated inside the log by "Branch: merge/context-brain-reconciliation (0 uncommitted changes)", and every scriptable assertion has matching output in the log. High is not met because the log does not echo the executed command lines verbatim; command completeness rests on the workbook row and the authoritative case definition.
- Evidence proves: banner content and version line; git status line; artisan-absence NOTE; loop-counter wipe (counter files after run: 0); cache-file creation (brain-validation-436674262, memory-summary-436674262); warm-run speedup with a stable governance line; exit 0 on both runs; metadata-only governance and memory-bank lines.
- Evidence does not prove: the interactive '/' command menu (item 9); that no network access occurred (not instrumented); exact invocation working directory (inferred from workbook + relative outputs).
- Missing information: verbatim command echoes inside the log; in-log timestamps.
- Recommended central-team follow-up: none required for the verdict; for future harness runs, echo each executed command into the log and remove the bc dependency from the timing helper.

#### Artifact Inventory
- TC-001.log — shareable filename: logs/original/TC-001.log — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found — included: yes.
- cases_map.json (TC-001 entry) — shareable filename: manifests/cases_map.json — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:14:56 +0300 — sensitive-content review: none found — included: yes.
- Workbook rows for RUN-001 (current workbook + historical snapshot extracts) — shareable filenames: our-runs.json / historical-runs.json extracts — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: extracted 2026-08-04 from the 2026-08-03 workbooks — sensitive-content review: none found — included: yes.

### RUN-002 / TC-002

#### Workbook Record
- Recorded result: Pass
- Framework: Symfony (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)"); Tester: "Claude Fable 5 (scripted execution)"
- Accelerator version: 1.3.1
- Git commit: 3a02537
- Execution date: 2026-08-03 (historical snapshot row; current workbook row coerced to "2026-08-03 00:00:00")
- Defect reference: None
- Workbook evidence link: Accelerator-TestEvidence/TC-002.log
- Row presence: exists in BOTH the current workbook and the historical snapshot; content verbatim-identical except mechanical coercion (datetime rendering of the date; count columns "7"/"0" rendered as "7.0"/"0.0").

#### Test Definition
- Definition status: Authoritative — original source found (cases_map.json TC-002 and the workbook's original 74-row Test Cases sheet agree field-for-field after whitespace normalization).
- Title: Claude Code activation banner - Symfony edition (bin/console detection)
- Objective: Verify the Symfony edition SessionStart banner: version from Symfony/VERSION, bin/console framework detection with the convention hint in a real Symfony host, the cautionary NOTE when bin/console is absent, loop-counter reset, metadata-only governance/memory lines with caching, exit 0. Category: Installation and activation; Priority: High.
- Preconditions: Symfony/ edition opened as workspace root; Symfony/.claude/settings.json wires SessionStart -> .claude/hooks/local-context.sh; Symfony/VERSION contains 1.3.1; git and python3 available. Two sub-scenarios: (a) accelerator folder alone (no bin/console); (b) installed into a real Symfony project where bin/console exists.
- Input: Start a new Claude Code session in the Symfony edition root, or run directly: `bash .claude/hooks/local-context.sh`.
- Mandatory assertions: 'Project Context' banner with 'Accelerator version: 1.3.1' from Symfony/VERSION; scenario (b) prints 'Framework: Symfony detected. Use Symfony Controller -> Service -> Repository conventions.'; scenario (a) prints the NOTE that Symfony bin/console was not detected and the Symfony rules should only be applied after confirming this is a Symfony project; loop counters for the repo key deleted at session start; governance and Memory Bank lines print metadata only with results cached under ${TMPDIR:-/tmp}/ai-accelerator-session-cache-<key>; exit code 0.
- Optional assertions: None explicitly marked optional in the source definition.
- Scriptable assertions (expected verification): `cd Symfony && bash .claude/hooks/local-context.sh | grep -F 'Accelerator version: 1.3.1'` -> match; with bin/console: grep -F 'Framework: Symfony detected' -> match; without bin/console: output contains 'Symfony bin/console was not detected'; `echo $?` -> 0; cache check via a second consecutive run printing the same 'brain-validation=' value.
- Native-client assertions: session start inside the IDE/client (the case is Hybrid; the in-client session-start path was the non-scripted portion).
- Expected state changes: None in the repository; only /tmp cache and loop-counter state as in the Laravel case.
- Safety expectations: No record contents printed. No writes inside the repository. Always exit 0. The Symfony convention hint must not be printed when bin/console is absent (prevents wrong-stack advice after a mis-installation).
- Requirement-source: Symfony/.claude/hooks/local-context.sh (SessionStart wiring in Symfony/.claude/settings.json).

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: /home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-002.log (shareable name: logs/original/TC-002.log), file mtime 2026-08-03 11:31:41 +0300. Supporting fixture: the temp scratch host repo used for scenario (b) still exists at .../c9fab6de-739d-497c-ac0d-61bda631def9/scratchpad/exec-hooks/symfony-host (contains bin/), volatile /tmp location.
- Commands: the log does not echo the invoked command lines. Per the workbook row: scenario (a) ran the hook "from Symfony/ edition root"; scenario (b) ran "the Symfony edition hook" "in a temp scratch host repo containing bin/console". The case definition's deterministic command is `bash .claude/hooks/local-context.sh`.
- Exit codes (verbatim from log): `exit=0` (scenario a, line 22), `exit=0` (scenario b, line 33).
- Assertions: 7 executed / 7 passed / 0 failed (workbook counts). Live-client portion not executed (see Native-Client Evidence).
- Key output excerpt (verbatim, non-contiguous lines from TC-002.log):
  ```
  Accelerator version: 1.3.1
  NOTE: Symfony bin/console was not detected. Apply these Symfony rules only after confirming this is a Symfony project.
  NO-WRONG-HINT-OK
  Framework: Symfony detected. Use Symfony Controller -> Service -> Repository conventions.
  CONSOLE-DETECT-OK
  counters-left=0
  ```
- Contradictions with workbook: none found. The log substantiates every claim in the workbook Actual Behaviour cell, including the suppressed wrong-stack hint (NO-WRONG-HINT-OK) and the loop-counter wipe (counters-left=0) with cache files brain-validation-436674262 / memory-summary-436674262 listed under /tmp/ai-accelerator-session-cache-1999016705/.

#### New Reproduction
- Performed: No (per-run scope). A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run.
- Historical retries: none recorded (workbook Retry Count 0).

#### Native-Client Evidence
- Run executed in scripted mode (bash/python driven by a live Claude Code session); native Claude Code client behavior was NOT exercised. The in-IDE session-start portion of this Hybrid case was not executed — Native assertions unavailable. The workbook Comments field records: "Live-client portion (session start inside the IDE) not executed; scripted part fully matched."

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium. Original log present; the tested commit (3a02537) is recorded in the workbook and corroborated inside the log by "Branch: merge/context-brain-reconciliation (0 uncommitted changes)"; every scriptable assertion has matching log output. High is not met because the executed command lines are not echoed verbatim in the log.
- Evidence proves: version line from Symfony/VERSION; correct NOTE without bin/console and correct convention hint with bin/console; suppression of the wrong-stack hint in scenario (a); loop-counter wipe; session-cache creation; exit 0 in both scenarios.
- Evidence does not prove: in-client session start behavior; absence of network access (not instrumented).
- Missing information: verbatim command echoes; in-log timestamps.
- Recommended central-team follow-up: none required for the verdict; optionally exercise scenario (b) inside a live client once.

#### Artifact Inventory
- TC-002.log — shareable filename: logs/original/TC-002.log — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found — included: yes.
- cases_map.json (TC-002 entry) — shareable filename: manifests/cases_map.json — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:14:56 +0300 — sensitive-content review: none found — included: yes.
- Workbook rows for RUN-002 (current workbook + historical snapshot extracts) — shareable filenames: our-runs.json / historical-runs.json extracts — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: extracted 2026-08-04 from the 2026-08-03 workbooks — sensitive-content review: none found — included: yes.

### RUN-003 / TC-003

#### Workbook Record
- Recorded result: Pass
- Framework: PHP Core (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)"); Tester: "Claude Fable 5 (scripted execution)"
- Accelerator version: 1.2.1
- Git commit: 3a02537
- Execution date: 2026-08-03 (historical snapshot row; current workbook row coerced to "2026-08-03 00:00:00")
- Defect reference: None
- Workbook evidence link: Accelerator-TestEvidence/TC-003.log
- Row presence: exists in BOTH the current workbook and the historical snapshot; content verbatim-identical except mechanical coercion (datetime rendering of the date; count columns "7"/"0" rendered as "7.0"/"0.0").

#### Test Definition
- Definition status: Authoritative — original source found (cases_map.json TC-003 and the workbook's original 74-row Test Cases sheet agree field-for-field after whitespace normalization).
- Title: Claude Code activation banner - PHP Core edition (native-PHP folder, path with space)
- Objective: Verify the PHP Core edition SessionStart banner: version from 'PHP Core/VERSION', native-PHP scanning (entry point detection), the framework guard steering framework projects to the correct edition, correct behavior despite the space in the edition path, loop-counter/cache behavior, exit 0. Category: Installation and activation; Priority: High.
- Preconditions: 'PHP Core'/ edition opened as workspace root (directory name contains a space — every shell command must quote it); 'PHP Core/VERSION' contains 1.2.1; 'PHP Core/.claude/settings.json' wires SessionStart -> .claude/hooks/local-context.sh; git and python3 available.
- Input: Start a new Claude Code session in the 'PHP Core' edition root, or run: `cd "PHP Core" && bash .claude/hooks/local-context.sh`.
- Mandatory assertions: 'Project Context' banner prints 'Accelerator version: 1.2.1' from 'PHP Core/VERSION'; the hook scans a native PHP working directory (composer/lock managers, PHP runtime version, 'Entry point: public/index.php (front controller)' when present); framework guard prints the NOTE '<Framework> detected. This is the native-PHP base folder (PHP Core/).' when a framework package (e.g. cakephp/cakephp) is detected in composer.json; loop counters reset and validation results cached identically to the other editions; exit code 0; the hook works correctly despite the space in the edition path (BASH_SOURCE-derived paths quoted).
- Optional assertions: None explicitly marked optional in the source definition.
- Scriptable assertions (expected verification): `cd "PHP Core" && bash .claude/hooks/local-context.sh | grep -F 'Accelerator version: 1.2.1'` -> match; `bash .claude/hooks/local-context.sh | head -1` -> 'Project Context'; `echo $?` -> 0; negative path-handling check: no 'No such file or directory' errors caused by the unquoted space.
- Native-client assertions: in-client session start (the case is Hybrid; only the scripted portion was in scope for the harness).
- Expected state changes: None in the repository; /tmp session cache and loop-counter reset only.
- Safety expectations: No record contents printed; no repository writes; always exit 0. The banner must not claim a framework-specific workflow for a native-PHP project.
- Requirement-source: PHP Core/.claude/hooks/local-context.sh (docs/ADOPTION.md section 1 for edition selection).

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: /home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-003.log (shareable name: logs/original/TC-003.log), file mtime 2026-08-03 11:31:41 +0300. Supporting fixture: the temp host repo used for the framework guard still exists at .../c9fab6de-739d-497c-ac0d-61bda631def9/scratchpad/exec-hooks/phpcore-host (composer.json, public/), volatile /tmp location.
- Commands: the log does not echo the invoked command lines. Per the workbook row: "Ran the hook from 'PHP Core' (path with a space)" and "Framework guard verified in a temp host repo with cakephp/cakephp in composer.json and public/index.php". The case definition's deterministic command is `cd "PHP Core" && bash .claude/hooks/local-context.sh`.
- Exit codes (verbatim from log): `exit=0` (edition-root run, line 20), `exit_host=0` (framework-guard host run, line 32).
- Assertions: 7 executed / 7 passed / 0 failed (workbook counts).
- Key output excerpt (verbatim, non-contiguous lines from TC-003.log):
  ```
  Accelerator version: 1.2.1
  Context governance: mode=governed, index=healthy/current, active-bindings=0, brain-validation=valid.
  /tmp/tc003.out:0
  /tmp/tc003.err:0
  NOTE: CakePHP detected. This is the native-PHP base folder (PHP Core/).
  Entry point: public/index.php (front controller)
  ENTRYPOINT-OK
  ```
  (The `/tmp/tc003.out:0` / `/tmp/tc003.err:0` lines are the harness's zero-match counts backing the workbook claim of "zero 'No such file or directory' errors on either stream".)
- Contradictions with workbook: none found. The workbook Actual Behaviour cites "mode=governed, brain-validation=valid"; the log's full governance line additionally shows index=healthy/current and active-bindings=0 — a superset, not a conflict.

#### New Reproduction
- Performed: No (per-run scope). A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run.
- Historical retries: none recorded (workbook Retry Count 0).

#### Native-Client Evidence
- Run executed in scripted mode (bash/python driven by a live Claude Code session); native Claude Code client behavior was NOT exercised. The in-client session-start portion of this Hybrid case was not executed — Native assertions unavailable.

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium. Original log present; the tested commit (3a02537) is recorded in the workbook and corroborated inside the log by "Branch: merge/context-brain-reconciliation (0 uncommitted changes)"; all scriptable assertions have matching log output, including the negative space-in-path check. High is not met because the executed command lines are not echoed verbatim in the log.
- Evidence proves: version line 1.2.1; clean handling of the space-containing path (zero 'No such file or directory' matches on both captured streams); framework-guard NOTE for CakePHP; front-controller entry-point detection; metadata-only governance line; exit 0 for both the edition-root and host runs.
- Evidence does not prove: in-client behavior; absence of network access (not instrumented).
- Missing information: verbatim command echoes; in-log timestamps.
- Recommended central-team follow-up: none required for the verdict.

#### Artifact Inventory
- TC-003.log — shareable filename: logs/original/TC-003.log — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found — included: yes.
- cases_map.json (TC-003 entry) — shareable filename: manifests/cases_map.json — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:14:56 +0300 — sensitive-content review: none found — included: yes.
- Workbook rows for RUN-003 (current workbook + historical snapshot extracts) — shareable filenames: our-runs.json / historical-runs.json extracts — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: extracted 2026-08-04 from the 2026-08-03 workbooks — sensitive-content review: none found — included: yes.

### RUN-004 / TC-004

#### Workbook Record
- Recorded result: Partial
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Cursor (AI Tool Version: "n/a — scripted run (no AI client)"); Tester: "Claude Fable 5 (scripted execution)"
- Accelerator version: 1.4.3
- Git commit: 3a02537
- Execution date: 2026-08-03 (historical snapshot row; current workbook row coerced to "2026-08-03 00:00:00")
- Defect reference: DEF-001 ("Cursor stop hook renders no working-memory rule until the 5th buffered turn of a new task", severity Low, priority Medium, per defects.json TC-004). NOTE: in the current workbook the ID DEF-001 is duplicated — a foreign Codex defect row (RUN-20260803-011/TC-AI-011, "Automatic promotion bypasses mandatory human review") reuses DEF-001, so the cross-reference from RUN-004 is ambiguous in the current Defects sheet (per the workbook-forensics digest). The historical snapshot has a unique DEF-001 = ours.
- Workbook evidence link: Accelerator-TestEvidence/TC-004.log
- Row presence: exists in BOTH the current workbook and the historical snapshot; content verbatim-identical except mechanical coercion (datetime rendering of the date; counts "11"/"10"/"1"/"0" rendered as "11.0"/"10.0"/"1.0"/"0.0").

#### Test Definition
- Definition status: Authoritative — original source found (cases_map.json TC-004 and the workbook's original 74-row Test Cases sheet agree field-for-field after whitespace normalization).
- Title: Cursor activation: rules + hooks.json + missing read hook compensated by working-memory.mdc
- Objective: Verify Cursor-edition activation wiring: the exact hooks.json registrations, deliberate absence of working-memory-read.sh compensated by the alwaysApply working-memory.mdc rule rendered by the stop hook, session-start re-render, git-ignore of the rendered rule, and Cursor loop-detection state location. Category: Installation and activation; Priority: Critical.
- Preconditions: Laravel/ edition opened in Cursor as project root; .cursor/rules/ contains accelerator-workflow.mdc, php-standards.mdc, safety-and-verification.mdc; .cursor/hooks.json (schema version: 1) and .cursor/hooks/*.sh present and executable; Cursor's optional Claude-file loading DISABLED (double-loading prevention); python3 available; git branch checked out.
- Input: Open the project in Cursor, start a new agent session, send any prompt, let the turn finish. Deterministic simulation of the stop hook from the edition root: `CONTEXT_TASK_ID=qa-cursor-activation bash .cursor/hooks/working-memory-write.sh`.
- Mandatory assertions: (1) hooks.json registers exactly sessionStart -> local-context.sh (timeout 15), beforeShellExecution -> bash-validator.sh (5), afterFileEdit -> file-naming-validator.sh (5) and loop-detection.sh (5), stop -> working-memory-write.sh (15); (2) exactly ONE session-context report at session start; (3) working-memory-read.sh deliberately ABSENT; (4) after the stop hook runs, .cursor/rules/working-memory.mdc exists with the documented frontmatter, 'alwaysApply: true', '# Working Memory (auto-rendered)' and the fenced Task Capsule; (5) the Cursor copy of local-context.sh re-renders the rule at session start; (6) the rendered file is git-ignored (Laravel/.gitignore line '.cursor/rules/working-memory.mdc'); (7) Cursor loop detection tracks per-file edits under /tmp/cursor-loop-detection-<repo key>.
- Optional assertions: None explicitly marked optional in the source definition.
- Scriptable assertions (expected verification): `test ! -f .cursor/hooks/working-memory-read.sh && echo absent-by-design`; `python3 -m json.tool .cursor/hooks.json > /dev/null && echo valid`; `grep -c '"command"' .cursor/hooks.json` -> 5; after the stop-hook simulation: `test -f .cursor/rules/working-memory.mdc`, frontmatter greps, `git check-ignore .cursor/rules/working-memory.mdc` -> path printed, `git status --short` -> no tracked changes.
- Native-client assertions: item 2 (exactly one session-context report at session start) and rule attachment to every prompt of the next turn — require the live Cursor client. Execution type: Hybrid.
- Expected state changes: .cursor/rules/working-memory.mdc created/refreshed (git-ignored, never committed); memory-bank/local/ runtime state (git-ignored); no tracked file modified.
- Safety expectations: The stop hook must always exit 0; only Git porcelain metadata reaches the buffer — never working-tree file contents; sensitive-looking paths excluded; the rule file replaced atomically and only on a fresh successful render; working-memory.mdc never committed; hooks must not fire twice.
- Requirement-source: Laravel/.cursor/hooks/README.md ('How the Task Capsule reaches Cursor') and docs/TOOL-INTEGRATIONS.md 'Cursor Activation and Double-Loading Prevention'.

#### Historical Evidence
Priority-run aspect coverage (each marked Available/Unavailable):
- Stop-hook first-run output: Available — original (TC-004.log lines 44–49: run marker, `exit=0`, and the head/grep 'No such file or directory' failures proving no rule was rendered).
- Individual outputs of the 5 buffered turns preceding auto-provisioning ("stop-hook outputs 1..5"): Unavailable — location unknown. The log records only the first stop-hook run and the post-provisioning re-run ("=== TC-004 continued: hook re-run after working task exists ==="); no per-turn transcript of the intermediate buffered turns was captured, and none was found in the surviving temp copy.
- working-memory.mdc presence and content after provisioning: Available — original (log lines 61–76: RENDERED-OK, frontmatter, ALWAYSAPPLY-OK/DESC-OK/HEADER-OK, IGNORED-OK). Corroborated by the rendered file itself, which still exists in the original temp copy at .../c9fab6de-739d-497c-ac0d-61bda631def9/scratchpad/exec-hooks/laravel-cursor/.cursor/rules/working-memory.mdc, mtime 2026-08-03 11:20:05 +0300, header byte-matching the log excerpt including "rendered: 2026-08-03T08:20:05Z" (= 11:20:05 local). Volatile /tmp location.
- Live-Cursor proof (single session-context report, rule attachment to next prompt): Unavailable — required AI client missing (scripted mode; no Cursor client run).
Details:
- Original artifacts: /home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-004.log (shareable name: logs/original/TC-004.log), file mtime 2026-08-03 11:31:41 +0300.
- Commands: the log does not echo the invoked command lines. Per the workbook row and case definition, the deterministic stop-hook command was `CONTEXT_TASK_ID=qa-cursor-activation bash .cursor/hooks/working-memory-write.sh`, executed "in a temp copy of the Laravel edition (git init + add)". The structural check commands are those from the case's expected verification (echo markers 'absent-by-design', 'valid', 'command-count=5' appear in the log).
- Exit codes (verbatim from log): `exit=0` (first stop-hook run, line 45), `exit=0` (re-run after working task exists, line 61).
- Assertions: 11 executed / 10 passed / 1 failed (workbook counts). Failed: expected behaviour item 4 on a fresh install — first stop-hook run produced no rule file (the recorded defect). Native-client items skipped.
- Key output excerpt (verbatim, non-contiguous lines from TC-004.log):
  ```
  absent-by-design
  command-count=5
  exit=0
  head: cannot open '.cursor/rules/working-memory.mdc' for reading: No such file or directory
  17:.cursor/rules/working-memory.mdc
  RENDERED-OK
  alwaysApply: true
  --- NOTE: first-run of stop hook on fresh state did NOT render the rule (working task auto-provisioned only after flush-after=5 buffered turns) ---
  ```
- Contradictions with workbook: none between log and workbook — the row's Partial result, Failure Reason, Retry Count 1 and DEF-001 reference all match the log. Two clarifications: (1) the log's "--- git status in temp copy (tracked changes) ---" block lists "A " staged entries — an artifact of the fixture's git init + add setup inside the temp copy, consistent with the workbook's "Files Modified: None in the real repo"; (2) the current workbook's DEF-001 ID collision (foreign Codex defect reusing DEF-001) is a workbook-integrity issue, not a log/workbook mismatch.

#### New Reproduction
- Performed: No (per-run scope). A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run.
- Historical retries recorded in the log: one. The first stop-hook run rendered no rule; after the working task was provisioned (5 buffered turns, per the log's closing NOTE), the hook was re-run under the marker "=== TC-004 continued: hook re-run after working task exists ===" and rendered the rule correctly (`exit=0`, RENDERED-OK). This matches the workbook Retry Count of 1 and the execute-test-cases digest's per-case retries list "TC-004(1)".

#### Native-Client Evidence
- Run executed in scripted mode (bash/python driven by a live Claude Code session); native Cursor client behavior was NOT exercised. The case's native-client assertions — exactly one session-context report at session start (double-loading prevention) and attachment of working-memory.mdc to the next turn's prompt — were not executed: Native assertions unavailable. The workbook Comments field records this explicitly: "Cursor in-IDE items (single session-context report, rule attachment to next prompt) require the live Cursor client and were not executed."

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium. Original log present with all structural outputs, the failure evidence, and the successful re-render; the defect record (defects.json TC-004 / DEF-001) and the surviving rendered rule file corroborate it; the workbook row matches the log in every checkable detail. High is not met because the log neither echoes the executed command lines nor contains a branch/commit line — commit 3a02537 rests on the batch-level clean-tree proof (TC-001.log of the same 11:31 batch) and the workbook row — and the live-Cursor half of the case is entirely unproven.
- Evidence proves: exact hooks.json wiring (5 command entries as specified); deliberate absence of working-memory-read.sh; presence of the three .mdc rules; gitignore coverage of working-memory.mdc (gitignore line 17 + git check-ignore); the first-run render gap (DEF-001); correct render after task provisioning with the documented frontmatter; session-start re-render logic and /tmp/cursor-loop-detection-<key> prefix present in the Cursor scripts; no secret/file contents in the rendered rule (log's porcelain-only check printed 0).
- Evidence does not prove: any live Cursor behavior (single session-context report, rule attachment, hook firing by the IDE); the intermediate states of buffered turns 1..5.
- Missing information: verbatim command echoes; per-turn buffer outputs; live-client capture.
- Recommended central-team follow-up: (1) one live Cursor session on a fresh copy to close the native-client assertions; (2) when verifying the DEF-001 fix (remediated by fix-qa-findings, currently uncommitted), capture each of the five buffered-turn stop-hook invocations; (3) resolve the DEF-001 ID collision in the current workbook.

#### Artifact Inventory
- TC-004.log — shareable filename: logs/original/TC-004.log — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found — included: yes.
- cases_map.json (TC-004 entry) — shareable filename: manifests/cases_map.json — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:14:56 +0300 — sensitive-content review: none found — included: yes.
- defects.json (TC-004 / DEF-001 entry) — shareable filename: manifests/defects.json — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 15:29:20 +0300 — sensitive-content review: none found — included: yes.
- Rendered rule file working-memory.mdc (original temp copy, volatile /tmp location) — shareable filename: not packaged (volatile fixture; header quoted in TC-004.log) — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt if packaged, otherwise Unknown — capture timestamp (file mtime): 2026-08-03 11:20:05 +0300 — sensitive-content review: none found (porcelain metadata only per log check) — included: no (referenced only).
- Workbook rows for RUN-004 (current workbook + historical snapshot extracts) — shareable filenames: our-runs.json / historical-runs.json extracts — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: extracted 2026-08-04 from the 2026-08-03 workbooks — sensitive-content review: none found — included: yes.

### RUN-005 / TC-005

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Codex (AI Tool Version: "n/a — scripted run (no AI client)"); Tester: "Claude Fable 5 (scripted execution)"
- Accelerator version: 1.4.3
- Git commit: 3a02537
- Execution date: 2026-08-03 (historical snapshot row; current workbook row coerced to "2026-08-03 00:00:00")
- Defect reference: None
- Workbook evidence link: Accelerator-TestEvidence/TC-005.log
- Row presence: exists in BOTH the current workbook and the historical snapshot; content verbatim-identical except mechanical coercion (datetime rendering of the date; counts "13"/"0" rendered as "13.0"/"0.0").

#### Test Definition
- Definition status: Authoritative — original source found (cases_map.json TC-005 and the workbook's original 74-row Test Cases sheet agree field-for-field after whitespace normalization).
- Title: Codex activation: project trust, config.toml features.hooks, hook self-filter early-exit
- Objective: Verify Codex-edition activation: trust-gated loading of .codex/config.toml with '[features]' / 'hooks = true', the hooks.json event wiring without matchers (each script self-filtering), destructive-command blocking through the bash-validator, skill discovery from .agents/skills, and the deliberate absence of .codex/commands and .codex/agents. Category: Installation and activation; Priority: High.
- Preconditions: Laravel/ edition opened with the Codex CLI or IDE extension (v0.117+, skill-based model); project NOT yet trusted at test start; .codex/config.toml, .codex/hooks.json, executable .codex/hooks/*.sh, and canon skills in .agents/skills/<name>/SKILL.md present; root AGENTS.md present.
- Input: Run `codex` in the repository root and accept the project trust prompt (or use /trust); then type '/' to open the skills menu, or ask: "use the verify skill". Hook-level simulation from the edition root: `printf '{"tool_name":"Read","file_path":"x.md"}' | bash .codex/hooks/bash-validator.sh ; printf '{"command":"git push --force origin main"}' | bash .codex/hooks/bash-validator.sh`.
- Mandatory assertions: (1) before trust, project-scoped .codex config and hooks do NOT load; (2) after trust, config.toml contains '[features]' / 'hooks = true'; (3) hooks.json wires SessionStart -> local-context.sh, UserPromptSubmit -> working-memory-read.sh (additionalContextLimit 4000), Stop -> working-memory-write.sh, PreToolUse -> bash-validator.sh AND file-naming-validator.sh, PostToolUse -> loop-detection.sh; (4) no matcher on any group — each script self-filters with an early exit; (5) destructive payload blocked with stderr 'BLOCKED: Destructive command detected: matches pattern ...' and exit 2; (6) skills discovered from .agents/skills (verify, memory, project-brain, coder, etc.); (7) deliberately NO .codex/commands and NO .codex/agents; (8) metadata-only SessionStart banner prints without indexing/injecting records.
- Optional assertions: None explicitly marked optional in the source definition.
- Scriptable assertions (expected verification): `grep -A1 '\[features\]' .codex/config.toml` -> 'hooks = true'; `python3 -m json.tool .codex/hooks.json > /dev/null` -> exit 0; `grep -F 'additionalContextLimit' .codex/hooks.json` -> '"additionalContextLimit": 4000'; Read payload piped to bash-validator -> exit 0 (early-exit, no extractor forked); destructive payload -> exit 2 with 'BLOCKED: Destructive command detected' on stderr; `test ! -d .codex/commands && test ! -d .codex/agents`; `ls .agents/skills | grep -x verify` -> present.
- Native-client assertions: items 1 and 6 in their interactive form — the trust prompt gating and the live skills menu — require the Codex CLI. Execution type: Hybrid.
- Expected state changes: None. All hook activity is stdin/stdout/exit-code based; loop tracking under /tmp/codex-loop-detection-<repo key>.
- Safety expectations: Hooks must fail OPEN (exit 0) when an expected payload key is absent; no MCP server may be enabled (the commented example in config.toml must remain commented); untrusted projects must not execute project-scoped hooks; the session hook never indexes, retrieves, or injects records.
- Requirement-source: Laravel/.codex/README.md and Laravel/.codex/hooks/README.md (also docs/TOOL-INTEGRATIONS.md 'Codex Trust, Configuration, and Hooks').

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: /home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-005.log (shareable name: logs/original/TC-005.log), file mtime 2026-08-03 11:31:41 +0300.
- Commands: the log does not echo the invoked command lines. Per the workbook row: "Read payload piped to .codex/hooks/bash-validator.sh exited 0 silently", "Destructive payload {\"command\":\"git push --force origin main\"} exited 2", and ".codex/hooks/local-context.sh printed the metadata-only banner and exited 0". The exact printf pipelines are specified in the case definition's input (quoted above).
- Exit codes (verbatim from log): `exit=0` (Read payload, line 16), `exit=2` (destructive payload, line 18), `exit=0` (SessionStart banner run, line 38).
- Assertions: 13 executed / 13 passed / 0 failed (workbook counts). Trust-prompt gating and the live skills menu skipped (native).
- Key output excerpt (verbatim, non-contiguous lines from TC-005.log):
  ```
  [features]
  hooks = true
  16:# [mcp_servers.github]
  PreToolUse no-matcher ['.codex/hooks/bash-validator.sh', '.codex/hooks/file-naming-validator.sh']
  BLOCKED: Destructive command detected: matches pattern 'git[^;&|]*[[:space:]]push[^;&|]*(--force([^[:alnum:]]|$)|-f([[:space:]]|$))'
     This operation is blocked. See AGENTS.md.
  no-commands-dir
  verify-present
  ```
- Contradictions with workbook: none found. Every workbook claim (features.hooks, commented MCP example at config line 16, five event groups with no matcher, additionalContextLimit 4000, silent exit 0 on the Read payload, exit 2 with the three-line BLOCKED stderr and empty stdout `stdout:[]`, absent .codex/commands and .codex/agents, skills verify/coder/memory/project-brain listed, banner exit 0) has a matching log line.

#### New Reproduction
- Performed: No (per-run scope). A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run.
- Historical retries: none recorded (workbook Retry Count 0).

#### Native-Client Evidence
- Run executed in scripted mode (bash/python driven by a live Claude Code session); native Codex client behavior was NOT exercised. The case's native-client assertions — trust-prompt gating (untrusted project must not load .codex config) and the live skills menu — were not executed: Native assertions unavailable. The workbook Comments field records this caveat explicitly.

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium. Original log present; the tested commit (3a02537) is recorded in the workbook and corroborated inside the log by "Branch: merge/context-brain-reconciliation (0 uncommitted changes)" in the banner run; all scriptable assertions have matching log output. High is not met because the executed command lines are not echoed verbatim in the log, and the trust-gating half of the case is unproven.
- Evidence proves: config.toml features.hooks enabled with the MCP example still commented; complete no-matcher event wiring including additionalContextLimit 4000; fail-open early exit on a non-command payload; destructive-command block with named pattern, empty stdout, exit 2; absence of .codex/commands and .codex/agents; presence of verify/coder/memory/project-brain under .agents/skills; metadata-only banner with exit 0.
- Evidence does not prove: that an untrusted project refuses to load .codex config; the live skills menu contents; loop tracking under /tmp/codex-loop-detection-<key> (not exercised in this log).
- Missing information: verbatim command echoes; in-log timestamps.
- Recommended central-team follow-up: one live Codex CLI session (trust prompt + '/' skills menu) to close the native assertions.

#### Artifact Inventory
- TC-005.log — shareable filename: logs/original/TC-005.log — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found — included: yes.
- cases_map.json (TC-005 entry) — shareable filename: manifests/cases_map.json — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:14:56 +0300 — sensitive-content review: none found — included: yes.
- Workbook rows for RUN-005 (current workbook + historical snapshot extracts) — shareable filenames: our-runs.json / historical-runs.json extracts — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: extracted 2026-08-04 from the 2026-08-03 workbooks — sensitive-content review: none found — included: yes.

### RUN-006 / TC-007

#### Workbook Record
- Recorded result: Partial
- Framework: Infrastructure-Creator (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Not Applicable)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.0
- Git commit: 3a02537
- Execution date: 2026-08-03 (historical snapshot); the current workbook stores the same date mechanically coerced to "2026-08-03 00:00:00"
- Tests recorded: 7 executed / 7 passed / 0 failed; Retry Count 0; Safety Violation Count 0; Linter Result "bash -n clean on all 6 hook scripts"
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-007.log` (maps to package file `logs/original/TC-007.log`)
- Row presence: exists in BOTH the current workbook ("Test Runs") and the historical snapshot; content is verbatim-identical apart from the current workbook's mechanical rewrite (datetime coercion, int-to-float count columns, 40 empty padding columns). Note: the current workbook's Test Cases sheet no longer contains TC-007 (74 -> 19 trim), so this run's case reference resolves only against the historical snapshot / cases_map.json.

#### Test Definition
- Definition status: Authoritative — original source found (cases_map.json has TC-007 and the workbook's original Test Cases sheet, preserved in historical-testcases.json, agrees on name, category, preconditions, verification, and safety text).
- Title: "infra-generate full mode passes validate_generated.py including context.py smoke"
- Objective: Prove that a full-mode /infra-generate run on a clean target produces a complete claude-edition accelerator that passes bootstrap-verifier (validate_generated.py) including the context.py smoke checks, with correct version stamp, manifest, and no unselected edition roots.
- Preconditions: "Case 1 completed: tasks/TASK-1/infra-scan-project-profile.md exists and was reviewed; selected editions = [claude]. The target /tmp/fixtures/fake-php-app has NO AGENTS.md and no .claude/.cursor/.codex/.agents folders (collision guard should pass without prompting)."
- Input: `/infra-generate /tmp/fixtures/fake-php-app`
- Mandatory assertions (from Expected Behaviour): (1) TASK-1 profile located and re-validated against current target files; (2) editions [claude] read from profile section 1; (3) collision guard finds nothing and proceeds; (4) fan-out of the four forges, then agent-forge/command-forge, then skill-flow-composer; (5) AGENTS.md first line is the version stamp `<!-- Generated by Infrastructure-Creator v1.4.0 | TASK-1 | <ISO date> -->` with the version from the VERSION file; (6) `.infra-manifest.json` (mode full) written via the exact SKILL recipe; (7) bootstrap-verifier runs LAST and must pass; (8) tasks/TASK-1/infra-generate-report.md written.
- Optional assertions: from the case Notes — validate_generated.py also proves the dead-hook wiring check (every .sh token in .claude/settings.json must resolve to an existing executable script).
- Scriptable assertions (from Expected Verification): `validate_generated.py --target /tmp/fixtures/fake-php-app --editions claude` exits 0; `context.py status` and `context.py validate` both exit 0 from the target root; `head -1 AGENTS.md | grep "Generated by Infrastructure-Creator v1.4.0"` matches; `test ! -d .cursor && test ! -d .codex && test ! -d .agents` all true; `.claude/skills` contains exactly the 4 memory-quartet entries.
- Native-client assertions: items 1, 2, 4, 8 of the mandatory list (profile re-validation, edition read-out, forge fan-out, generation report) are agent behaviours requiring an interactive AI client.
- Expected state changes: target gains AGENTS.md, .claude/skills (incl. memory-bank, project-brain, checkpoint, memory), .claude/agents/, .claude/commands/, .claude/hooks/ (6 scripts), .claude/settings.json, memory-bank/ and project-brain/ trees, .infra-manifest.json; generator gains tasks/TASK-1/infra-generate-report.md.
- Safety expectations: only the selected claude edition written; no content invented beyond the profile; no success while bootstrap-verifier fails; target .env never read; no template placeholders remain in generated files.
- Requirement-source: `Infrastructure-Creator/.agents/skills/infra-generate/SKILL.md` (Priority: Critical; Test Owner: QA; Execution Type: Hybrid).

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-007.log` (shareable name: `logs/original/TC-007.log`, 19 lines). The log's own header states the material caveat: "TC-007 scripted verification (target assembled manually per SKILL.md instructions in exec-ic/fake-php-app; /infra-generate agent phases not runnable)".
- Commands (quoted verbatim from the log; note the log abbreviates the target path as `<T>` and paraphrases the placeholder scan):
  - `$ python3 validate_generated.py --target <T> --editions claude`
  - `$ python3 memory-bank/scripts/context.py status`
  - `$ python3 memory-bank/scripts/context.py validate`
  - `$ head -1 AGENTS.md | grep "Generated by Infrastructure-Creator v1.4.0"`
  - `$ test ! -d .cursor && test ! -d .codex && test ! -d .agents`
  - `$ ls .claude/skills | grep -cE ^\(memory-bank\|project-brain\|checkpoint\|memory\)$`
  - `$ grep -rE placeholders in generated files` (paraphrased command record, not a runnable shell line)
- Exit codes: `exit=0` recorded after validate_generated.py, context.py status, context.py validate, and the head/grep stamp check. Exit codes for the last three checks: not printed in the log — Unknown (outputs recorded instead: "no unselected edition roots: true", "4", "0").
- Assertions: 7 passed / 0 failed / 0 skipped of the 7 scriptable checks (matches workbook 7/7/0). Native-client assertions (profile consumption, forge fan-out, report): not executed — see Native-Client Evidence.
- Key output excerpt (log lines 2–4, 8–12, quoted exactly):
  ```
  $ python3 validate_generated.py --target <T> --editions claude
  generated accelerator OK
  exit=0
  $ python3 memory-bank/scripts/context.py validate
  Project Brain validation passed.
  exit=0
  $ head -1 AGENTS.md | grep "Generated by Infrastructure-Creator v1.4.0"
  <!-- Generated by Infrastructure-Creator v1.4.0 | TASK-1 | 2026-08-03 -->
  ```
- Contradictions with workbook: none. The workbook row, the executor digest (Partial, Dur 300 s, retries 0, first attempt Yes, 7/7/0), and the log agree.

Priority aspects requested by the central team, addressed explicitly:
- Profile consumed (TASK-1 re-validation): evidence Unavailable — required AI client missing. The target was assembled manually per SKILL.md; no profile-consumption transcript exists.
- Forge workflow (fan-out of policy/skill/hook/memory forges): evidence Unavailable — required AI client missing.
- Selected editions honored: evidence Available — log proves `.cursor`/`.codex`/`.agents` absent ("no unselected edition roots: true") and the claude edition present with 4 memory-quartet skills.
- Generation report (tasks/TASK-1/infra-generate-report.md): evidence Unavailable — required AI client missing; never produced.
- Validation: evidence Available — validate_generated.py "generated accelerator OK" exit=0; context.py status and validate both exit=0; stamp line verified.
- Unrelated files preserved: evidence Unavailable — the fixture was built clean per the case preconditions (no pre-existing surface), so no preservation check appears in the log; that property is exercised by TC-008 (RUN-007) instead.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. No historical retries are documented in the log or workbook for this run (Retry Count 0, first attempt Yes).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); native Claude Code client behavior was NOT exercised. The case's ai_tool is Claude Code and its agent_command is `/infra-generate (spawns infra-generate-agent)`; the agent pipeline (profile re-validation, forge fan-out, infra-generate-report.md) did not run. Those assertions are marked: Native assertions unavailable. The workbook Failure Reason records this explicitly: "LLM phases (profile re-validation, forge fan-out, infra-generate-report.md) require an interactive AI client run; only the structural/verification contract was executed."

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium. The original log exists, the commit is exact (3a02537, clean tree per TC-001.log), and all recorded checks pass — but the log's command records are abbreviated (`<T>` placeholder, paraphrased placeholder scan), and the tested artifact was a hand-assembled target following SKILL.md, not the output of the /infra-generate pipeline.
- Evidence proves: that a target built exactly per the infra-generate/hook-forge/memory-seed SKILL instructions at v1.4.0 verifies green on the first attempt (bootstrap-verifier, context.py smoke, stamp, edition isolation, quartet skills, zero placeholders) — i.e. the written generation contract is internally consistent and mechanically satisfiable.
- Evidence does not prove: that the infra-generate agent itself consumes the profile, fans out the forges, selects editions, or writes the generation report; nor collision-guard behaviour (out of scope here, covered by TC-008).
- Missing information: exact expansion of the abbreviated commands; the assembled fixture's full file list at run time (fixture tree survives in the volatile session scratchpad but is not packaged); infra-generate-report.md (never produced).
- Recommended central-team follow-up: one live /infra-generate run in a native Claude Code client against a clean fixture, keeping tasks/TASK-1/infra-generate-report.md as evidence, then re-running the same seven scriptable checks.

#### Artifact Inventory
- TC-007 execution log — shareable filename `logs/original/TC-007.log`; status: Available — original; SHA-256: d7a81f1e39ff3234a35384d1c0c907c5edc4480d2e7fdcd80fe9e962de496996 (computed 2026-08-04 during verification; also listed in the package SHA256SUMS.txt); capture timestamp (package copy mtime): 2026-08-03 11:31:41 +0300 (execute-test-cases digest: originals written 11:18–11:27 EEST, copied byte-identically, md5 spot-checked on TC-001/TC-072; package copy re-verified md5-identical to the scratchpad original on 2026-08-04); sensitive-content review: none found (contains local filesystem paths only); included: yes.
- Assembly script `build_green_target.sh` (session scratchpad, mtime 2026-08-03 11:21 +0300) — status: Available — original (volatile /tmp session scratchpad, not packaged); SHA-256: Unknown (not in package); sensitive-content review: not performed; included: no.
- Fixture tree `exec-ic/fake-php-app/` (session scratchpad, dir mtime 2026-08-03 11:22:50 +0300) — status: Available — original (volatile /tmp session scratchpad, not packaged); included: no.
- tasks/TASK-1/infra-generate-report.md — status: Unavailable — required AI client missing (never produced); included: no.

### RUN-007 / TC-008

#### Workbook Record
- Recorded result: Partial
- Framework: Infrastructure-Creator (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Not Applicable)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.0
- Git commit: 3a02537
- Execution date: 2026-08-03 (historical snapshot); current workbook stores "2026-08-03 00:00:00" (mechanical coercion)
- Tests recorded: 7 executed / 6 passed / 1 failed; Retry Count 0; Safety Violation Count 0; Linter Result: Not Applicable
- Defect reference: DEF-002 ("Merge-mode verification fails when a pre-existing team AGENTS.md contains the word TODO", severity Low / priority Medium, component `Infrastructure-Creator/.agents/skills/bootstrap-verifier/scripts/validate_generated.py`). Caution: in the CURRENT workbook the ID "DEF-002" collides with an unrelated Codex-side defect row ("Merge automation can complete governed tasks implicitly", RUN-20260803-012/TC-AI-012); the collision-free binding RUN-007 -> DEF-002 exists in the historical snapshot.
- Workbook evidence link: `Accelerator-TestEvidence/TC-008.log` (maps to package file `logs/original/TC-008.log`)
- Row presence: exists in BOTH the current workbook and the historical snapshot, content verbatim-identical apart from the current workbook's mechanical rewrite (datetime coercion, float counts, padding columns). The current workbook's Test Cases sheet no longer contains TC-008.

#### Test Definition
- Definition status: Authoritative — original source found (cases_map.json TC-008 and the historical Test Cases sheet agree).
- Title: "infra-generate merge mode never touches or claims pre-existing team files"
- Objective: Prove the collision guard's merge path is loss-proof: pre-existing team files are byte-identical after merge, never claimed by the manifest, and never stamped.
- Preconditions: "A profile for the target exists (as in Case 1). The target already contains a team-authored AGENTS.md and a custom hook .claude/hooks/deploy-check.sh. Record baselines before the run: sha256sum ... > /tmp/pre-merge.sha256."
- Input: `/infra-generate /tmp/fixtures/fake-php-app  (when the collision guard asks 'overwrite, merge, or abort', answer: merge)`
- Mandatory assertions (from Expected Behaviour): (1) collision guard detects existing AGENTS.md/.claude and STOPS to ask overwrite/merge/abort; (2) on 'merge', pre-existing surface snapshotted BEFORE any forge writes into tasks/TASK-{N}/preexisting-files.txt; (3) forges add only what is missing, pre-existing files never modified; (4) manifest written with "mode": "merge", files map lists ONLY files this run created — never AGENTS.md or deploy-check.sh; (5) pre-existing AGENTS.md stays unstamped/untracked; bootstrap-verifier skips the stamp check for it and does not enforce coverage for merge.
- Optional assertions: from the case Notes — "coverage enforcement is deliberately off for mode merge in validate_generated.py".
- Scriptable assertions (from Expected Verification): `sha256sum -c /tmp/pre-merge.sha256` OK for both files; manifest inspection prints `merge / False / False`; `grep -c "AGENTS.md" ...preexisting-files.txt` >= 1; `validate_generated.py --target ... --editions claude` exits 0.
- Native-client assertions: the interactive collision-guard stop/ask dialogue (mandatory item 1 as a conversation) and the merge decision being taken by a human in-session.
- Expected state changes: target gains only newly generated missing files plus `.infra-manifest.json` (mode merge); generator gains tasks/TASK-{N}/preexisting-files.txt and infra-generate-report.md; pre-existing AGENTS.md and deploy-check.sh unchanged.
- Safety expectations: "Not a single pre-existing team file is modified or claimed in the manifest... The pre-existing AGENTS.md is not stamped. No write happens before the explicit merge decision."
- Requirement-source: `Infrastructure-Creator/.agents/skills/infra-generate/SKILL.md` (Priority: Critical; Test Owner: QA; Execution Type: Hybrid).

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-008.log` (shareable name: `logs/original/TC-008.log`, 26 lines). Header: "TC-008 scripted merge-mode mechanics: pre-existing team files never touched or claimed".
- Commands: the log records phase markers and verification outputs rather than verbatim shell lines; the only command-like records are `sha256sum -c pre-merge baselines:` (followed by two `: OK` lines) and the labeled verification outputs listed below. Full command text: Unknown (harness script `tc008.sh` holds it, outside the package).
- Exit codes: `exit=0` after "validate_generated.py on merge target: generated accelerator OK" (log lines 18–20); `observation exit: 1` for the TODO-probe validator run (line 25). Exit codes for the sha256/manifest/grep checks: not printed — Unknown (outputs recorded instead).
- Assertions: 6 passed, 1 failed, 0 skipped (workbook 7/6/1). The failed one is the defect probe: validator tolerance of a TODO word in the team AGENTS.md. Green outcomes recorded in the log: sha256 OK for AGENTS.md and for deploy-check.sh; manifest mode `merge`; claims AGENTS.md `False`; claims deploy-check.sh `False`; preexisting-files.txt mentions AGENTS.md (`1`); validate_generated.py exit=0; AGENTS.md unstamped (first line `# Team agents guide`). How these outcomes group into the workbook's 7 executed checks is not itemized in the log — Unknown.
- Key output excerpt (log lines 11–20, quoted exactly, paths shortened at "...scratchpad"):
  ```
  ...scratchpad/exec-ic/fake-php-app-merge/AGENTS.md: OK
  ...scratchpad/exec-ic/fake-php-app-merge/.claude/hooks/deploy-check.sh: OK
  manifest mode / claims AGENTS.md / claims deploy-check.sh:
  merge
  False
  False
  preexisting-files.txt mentions AGENTS.md (expect >=1): 1
  validate_generated.py on merge target:
  generated accelerator OK
  exit=0
  ```
  Defect probe (lines 23–26; note the log itself spells the trigger word "T0D0" to avoid self-tripping scanners): `-- observation: does the validator tolerate a team AGENTS.md containing the word T0D0? --` followed by `ERROR: ...exec-ic/fake-php-app-merge/AGENTS.md: leftover placeholder matching /\bTODO\b/`, `observation exit: 1`, `restored: validator green again`.
- Contradictions with workbook: none. Workbook (7/6/1, DEF-002), executor digest (Partial, Dur 240 s, retries 0, first attempt Yes), defects.json TC-008 entry, and the log agree.

Priority aspects requested by the central team, addressed explicitly:
- Collision detection output: evidence Unavailable — required AI client missing for the live guard dialogue; the scripted equivalent (pre-write snapshot) is Available: log lines 3–5 show preexisting-files.txt content `AGENTS.md` and `.claude/hooks/deploy-check.sh` captured "BEFORE any forge writes".
- Alternatives offered (overwrite/merge/abort): evidence Unavailable — required AI client missing; the case's input prompt documents the intended dialogue, no transcript exists.
- Decision (merge): evidence Available as a scripted decision — log line 6: "merge: forges add ONLY what is missing (lifted from the green generation); AGENTS.md and deploy-check.sh untouched".
- Team-owned files intact / AGENTS.md checksum: evidence Available — `sha256sum -c` OK for both files (lines 11–12).
- Unchanged files: evidence Available — same sha256 baselines; plus unstamped first line `# Team agents guide` (line 22).
- Manifest ownership: evidence Available — mode `merge`, claims AGENTS.md `False`, claims deploy-check.sh `False`; manifest wrote 48 files (line 8: "wrote .infra-manifest.json (48 files, mode=merge)").
- Placeholder text (defect): evidence Available — the ERROR line and `observation exit: 1`, filed as DEF-002/defects.json["TC-008"]; root cause per defect record: "check_placeholders(target/'AGENTS.md') is called without the mode=='full' or 'AGENTS.md' in files guard that the stamp check uses".
- Validator output: evidence Available — "generated accelerator OK" exit=0 on the clean merge target; exit 1 only under the TODO probe; "restored: validator green again" afterwards.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. Note the tree remediation (fix-qa-findings, 2026-08-03 15:31–16:28) modified validate_generated.py specifically to fix this defect, so a re-run on the dirty tree would not reproduce the historical failure. No historical retries documented (Retry Count 0, first attempt Yes).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); native Claude Code client behavior was NOT exercised. The interactive collision-guard stop/ask flow (detect -> present overwrite/merge/abort -> wait for human decision) was not observed; those assertions are marked: Native assertions unavailable. Workbook Failure Reason: "Collision-guard stop/ask flow requires an interactive AI client; placeholder-scan defect found on team AGENTS.md."

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium. Original log present, exact commit (3a02537, clean tree per TC-001.log), outputs and both decisive exit codes recorded; but the shell commands themselves are mostly summarized rather than quoted (full text lives in the unpackaged tc008.sh), and the merge was performed by the harness, not by the collision-guard agent.
- Evidence proves: the core data-loss protection holds mechanically at v1.4.0 — after a merge-mode generation, pre-existing team files are byte-identical, unclaimed by the manifest, unstamped, and the merge target verifies green; and it proves a real validator defect (unconditional placeholder scan on a team-owned AGENTS.md), reproducible with `observation exit: 1`.
- Evidence does not prove: that the live collision guard stops before writing, presents the three alternatives, or respects an interactive "merge" answer; nor that preexisting-files.txt would be produced by the agent (here the harness wrote it).
- Missing information: verbatim harness commands; the generated infra-generate-report.md (never produced); manifest file list (only the count, 48, is logged).
- Recommended central-team follow-up: live native-client run of /infra-generate against a pre-populated target to capture the collision dialogue transcript; confirm the DEF-002 fix (now in the uncommitted remediation) is committed and covered by a regression test.

#### Artifact Inventory
- TC-008 execution log — shareable filename `logs/original/TC-008.log`; status: Available — original; SHA-256: 3afa2d11cf40c84f1b8c0127ddbbdeddbec2e96239c77b1273321276094bea2f (computed 2026-08-04 during verification; also listed in the package SHA256SUMS.txt; package copy verified md5-identical to the scratchpad original); capture timestamp (package copy mtime): 2026-08-03 11:31:41 +0300; sensitive-content review: none found (contains local filesystem paths only); included: yes.
- Defect record defects.json["TC-008"] (= DEF-002) — shareable filename `manifests/defects.json`; status: Available — original; SHA-256: 17b172e406c8506417ae9b5959f083d87a4bf0072c26fee94ef7683e21da1fdb (computed 2026-08-04 during verification; also listed in the package SHA256SUMS.txt; package copy verified md5-identical to the scratchpad original); capture timestamp: file written 2026-08-03 15:29:20 +0300 per fix-qa-findings digest (mtime re-confirmed 2026-08-04); sensitive-content review: none found; included: yes.
- Harness script `tc008.sh` (session scratchpad, mtime 2026-08-03 11:25 +0300) — status: Available — original (volatile /tmp session scratchpad, not packaged); included: no.
- Fixture tree `exec-ic/fake-php-app-merge/` — status: Available — original with caveat: directory mtime 2026-08-03 15:40:09 +0300 postdates the run, consistent with re-use during the later fix-qa-findings verification; not pristine run-time state; included: no.
- tasks/TASK-{N}/preexisting-files.txt (`exec-ic/task-3/preexisting-files.txt` per the workbook Files Created field) — status: Available — original (session scratchpad, not packaged); included: no.

### RUN-008 / TC-014

#### Workbook Record
- Recorded result: Partial
- Framework: Infrastructure-Creator (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Not Applicable)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.0
- Git commit: 3a02537
- Execution date: 2026-08-03 (historical snapshot); current workbook stores "2026-08-03 00:00:00" (mechanical coercion)
- Tests recorded: 11 executed / 10 passed / 1 failed; Retry Count 2; Safety Violation Count 0; Linter Result: Not Applicable
- Defect reference: DEF-003 ("Full-mode manifest coverage rejects team-added files under edition roots, deadlocking infra-update verification", severity Medium / priority High — the highest-graded defect of the scripted batch). Caution: in the CURRENT workbook the ID "DEF-003" collides with an unrelated Codex-side row ("Relative target-path defect not reproduced", Rejected, RUN-20260803-013/TC-AI-013); the collision-free binding RUN-008 -> DEF-003 exists in the historical snapshot.
- Workbook evidence link: `Accelerator-TestEvidence/TC-014.log` (maps to package file `logs/original/TC-014.log`)
- Row presence: exists in BOTH the current workbook and the historical snapshot, verbatim-identical apart from the mechanical rewrite. The current workbook's Test Cases sheet no longer contains TC-014.

#### Test Definition
- Definition status: Authoritative — original source found (cases_map.json TC-014 and the historical Test Cases sheet agree).
- Title: "infra-update classifies safe-update vs needs-decision vs untouchable and persists decision memory"
- Objective: Prove infra-update's three-way classification (safe update / requires decision / untouchable) and its Decision Memory: a "keep" decision is persisted in the manifest with the rejected staged sha256 and honored on later runs.
- Preconditions: "Case 3 target with valid .infra-manifest.json. Then simulate team activity: echo '# team tweak' >> .../.claude/hooks/local-context.sh (user-modified manifest-listed file); printf 'team notes\n' > .../.claude/skills-notes.md (file NOT in the manifest); leave .claude/hooks/loop-detection.sh untouched (hash == manifest)."
- Input: `/infra-update /tmp/fixtures/fake-php-app  (when asked about .claude/hooks/local-context.sh, answer: keep target version)`
- Mandatory assertions (from Expected Behaviour): (1) manifest loaded, profile re-validated; (2) equal VERSION vs generator_version declared a drift-repair pass; (3) regeneration staged inside the generator (tasks/TASK-{N}/infra-update-staging/), never the target, skipping memory state; (4) classification — untouched loop-detection.sh -> safe update; modified local-context.sh -> requires decision, never auto-applied; unmanifested .claude/skills-notes.md -> untouchable: not read, not diffed, not reported; (5) after 'keep', manifest gains decisions entry {"decision": "kept", "rejected_sha256": "<staged sha>", "task": "TASK-{N}"}; (6) AGENTS.md re-stamped; (7) bootstrap-verifier runs and infra-update-report.md written.
- Optional assertions: from the case Notes — "The second run exercises Decision Memory: staged sha256 == rejected_sha256 must yield 'standing decision honored' automatically; if the generator's staged output changed, the file must be re-asked."
- Scriptable assertions (from Expected Verification): `grep -c 'team tweak' .../local-context.sh` -> 1; decisions entry prints `kept True` (decision "kept", 64-char sha); `grep -c 'skills-notes' .../infra-update-report.md` -> 0; no staging dir inside the target; `validate_generated.py --target ... --editions claude` -> exit 0; regression run reports 'standing decision honored'.
- Native-client assertions: the interactive per-file decision presentation with three-way context (as generated / as staged / as in target) and the infra-update-report.md authored by the agent.
- Expected state changes: target — safe-updated generator-owned files only, refreshed .infra-manifest.json with a decisions map, re-stamped AGENTS.md; generator — tasks/TASK-{N}/infra-update-staging/ and infra-update-report.md; local-context.sh keeps the team tweak; skills-notes.md untouched.
- Safety expectations: "A file whose hash differs from the manifest is NEVER overwritten without an explicit per-file human decision. Files absent from the manifest are never read, modified, deleted, or reported. Memory state ... is never staged or touched. Edition selection is never changed during an update. A decisions entry never authorizes an automatic replace."
- Requirement-source: `Infrastructure-Creator/.agents/skills/infra-update/SKILL.md` (Priority: Critical; Test Owner: QA; Execution Type: Hybrid).

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-014.log` (shareable name: `logs/original/TC-014.log`, 34 lines). Header: "TC-014 scripted triage: manifest vs staged vs target (drift-repair pass, equal versions)"; line 4 states the method: "classification (triage.py implements skill step 5 table literally)".
- Commands: the log records labeled checks with outputs, not verbatim shell lines; the closest command records are `grep -c 'team tweak' local-context.sh (expect 1 = kept):` and "validate_generated.py on updated target (validates decisions map too):". Full command text: Unknown (harness scripts `triage.py`/`tc014.sh`, outside the package).
- Exit codes: `exit=1` for validate_generated.py on the updated target (log line 24) — the case-expected 0 was NOT met (this is the defect); `negative validator exit: 1` (line 34) for the malformed-decisions probes. Other checks: exit codes not printed — Unknown (outputs recorded).
- Assertions: 10 passed, 1 failed, 0 skipped (workbook 11/10/1). Passed include: requires-decision on modified local-context.sh (exactly 1 row); safe-update no-op on loop-detection.sh; skills-notes.md rows 0 (untouchable, never reported); 'keep' honored (`grep -c` = 1); decisions entry `kept True`; re-stamp `<!-- Generated by Infrastructure-Creator v1.4.0 | TASK-2 | 2026-08-03 -->`; no staging dir in target; regression `standing-decision-honored`; variant re-asks (`requires-decision`); negative validator rejects all three malformed decisions entries. Failed: "validate_generated.py exits 0 after the update".
- Key output excerpt (log lines 20–24 and 31–32, quoted exactly):
  ```
  validate_generated.py on updated target (validates decisions map too):
  ERROR: .infra-manifest.json: generated file not tracked: .claude/skills-notes.md

  1 problem(s) found.
  exit=1
  ERROR: .infra-manifest.json: decisions['.claude/hooks/local-context.sh'].decision must be 'kept' or 'merged', got 'replaced'
  ERROR: .infra-manifest.json: decisions entry for untracked file: .claude/skills-notes.md
  ```
- Contradictions with workbook: none. Workbook (11/10/1, Retry Count 2, DEF-003), executor digest (Partial, Dur 420 s, retries 2, first attempt No), defects.json TC-014 entry, and the log agree. The retries are not visible inside the log itself — see New Reproduction.

Priority aspects requested by the central team, addressed explicitly:
- Manifest before the update: evidence Unavailable in the log — the pre-update manifest contents are not printed; only derived facts appear (48 safe-update rows against it; rewrite to "49 files, decisions entries: 1"). The fixture tree survives in the session scratchpad but was modified after the run.
- Team file checksum / integrity: evidence Available — `team tweak` grep count 1 after 'keep' (line 15) and again after the regression run (line 27: "team tweak still present: 1"); skills-notes.md rows 0 (line 7) proves the unmanifested file was never iterated, read, or reported.
- Decisions (Decision Memory): evidence Available — "rewrote manifest: 49 files, decisions entries: 1" (line 12); check prints `kept True` (decision == "kept", rejected_sha256 length == 64, line 17); regression prints `standing-decision-honored	.claude/hooks/local-context.sh` (line 26); changed-generator variant prints `requires-decision	.claude/hooks/local-context.sh` (line 29).
- Validator output: evidence Available — the full ERROR + "1 problem(s) found." + `exit=1` block, plus the three negative-probe ERROR lines with `negative validator exit: 1`.
- Not-claimed proof: evidence Available — the validator itself rejects a decisions entry for an untracked file ("ERROR: .infra-manifest.json: decisions entry for untracked file: .claude/skills-notes.md"), and the triage produced 0 rows for skills-notes.md.
- Diagnostic: evidence Available — the defect diagnosis is recorded in defects.json["TC-014"]: coverage semantics "conflate 'file lives under a generator-owned root' with 'file is generator-owned'"; full-mode coverage contradicts infra-update's untouchable-file guardrail, deadlocking step 9 verification.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. Historical retries: the workbook records Retry Count 2 and the executor digest attributes both to the harness, not the product — "a bash heredoc syntax slip, then an over-broad staging tree that wrongly included fixture files"; "the product checks behaved identically across runs" (workbook Comments). The packaged log shows only the final attempt. Note the remediation modified validate_generated.py to address DEF-003, so the historical exit=1 is not expected to reproduce on the dirty tree.

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); native Claude Code client behavior was NOT exercised. The interactive decision presentation (three-way context per file) and the agent-authored infra-update-report.md were not observed; those assertions are marked: Native assertions unavailable. Workbook Failure Reason: "validate_generated.py exit 1 instead of expected 0: full-mode coverage flags the team's untouchable .claude/skills-notes.md as 'generated file not tracked'; also interactive decision presentation and infra-update-report.md require an AI client."

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium. Original log, exact commit, decisive exit codes and all classification outputs are present; but the step-5 logic ran through a harness reimplementation (triage.py "implements skill step 5 table literally") rather than the infra-update agent, commands are summarized, and two harness-side retries preceded the logged attempt.
- Evidence proves: the classification table and Decision Memory mechanics are sound and the validator strictly enforces the decisions schema; and it proves a genuine Medium/High product contradiction — a team-added file under an edition root makes full-mode verification permanently red while the untouchable guardrail forbids the only "fix" the validator would accept (DEF-003).
- Evidence does not prove: that the infra-update agent itself performs this classification, presents decisions interactively, honors "keep" in a live session, or writes infra-update-report.md; the report-absence check (`grep -c 'skills-notes' ... -> 0`) was satisfiable only against harness output since no agent report exists.
- Missing information: verbatim harness commands and triage.py source in the package; pre-update manifest snapshot; the two discarded retry transcripts.
- Recommended central-team follow-up: treat DEF-003 as the release-gating item of this batch; verify the uncommitted remediation (validate_generated.py coverage semantics + new Infrastructure-Creator/tests/test_validate_generated.py) is committed; then a live /infra-update native-client run to capture the decision dialogue and report.

#### Artifact Inventory
- TC-014 execution log — shareable filename `logs/original/TC-014.log`; status: Available — original; SHA-256: 738808a646cb60614b23e4e4a6f97969d554daff101c0eff6fd4d84f79901b94 (computed 2026-08-04 during verification; also listed in the package SHA256SUMS.txt; package copy verified md5-identical to the scratchpad original); capture timestamp (package copy mtime): 2026-08-03 11:31:41 +0300; sensitive-content review: none found (contains local filesystem paths only); included: yes.
- Defect record defects.json["TC-014"] (= DEF-003) — shareable filename `manifests/defects.json`; status: Available — original; SHA-256: 17b172e406c8506417ae9b5959f083d87a4bf0072c26fee94ef7683e21da1fdb (computed 2026-08-04 during verification; also listed in the package SHA256SUMS.txt; package copy verified md5-identical to the scratchpad original); capture timestamp: 2026-08-03 15:29:20 +0300 per fix-qa-findings digest (mtime re-confirmed 2026-08-04); sensitive-content review: none found; included: yes.
- Harness scripts `triage.py` (mtime 2026-08-03 11:23 +0300) and `tc014.sh` (mtime 2026-08-03 11:24 +0300), session scratchpad — status: Available — original (volatile /tmp session scratchpad, not packaged); included: no.
- Fixture tree `exec-ic/fake-php-app-update/` — status: Available — original with caveat: directory mtime 2026-08-03 16:22:40 +0300 postdates the run (re-used during fix-qa-findings verification); not pristine run-time state; included: no.
- Staging tree `exec-ic/task-2-staging/` — status: Available — original with caveat: directory mtime 2026-08-03 16:22 +0300 postdates the run (re-used during fix-qa-findings verification, like the fixture tree); not pristine run-time state (session scratchpad, not packaged); included: no.
- tasks/TASK-{N}/infra-update-report.md — status: Unavailable — required AI client missing (never produced); included: no.

### RUN-009 / TC-015

#### Workbook Record
- Recorded result: Partial
- Framework: Infrastructure-Creator (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Not Applicable)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.0
- Git commit: 3a02537
- Execution date: 2026-08-03 (historical snapshot); current workbook stores "2026-08-03 00:00:00" (mechanical coercion)
- Tests recorded: 4 executed / 4 passed / 0 failed; Retry Count 0; Safety Violation Count 0; Linter Result: Not Applicable
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-015.log` (maps to package file `logs/original/TC-015.log`)
- Row presence: exists in BOTH the current workbook and the historical snapshot, verbatim-identical apart from the mechanical rewrite. The current workbook's Test Cases sheet no longer contains TC-015.

#### Test Definition
- Definition status: Authoritative — original source found (cases_map.json TC-015 and the historical Test Cases sheet agree).
- Title: "infra-update aborts without writes when .infra-manifest.json is missing"
- Objective: Prove the hard guardrail that infra-update step 1 ABORTS with zero target writes when the manifest is absent, explains the legacy-generation situation, offers exactly the two SKILL recovery options, and never fabricates a manifest.
- Preconditions: "A previously generated target exists (Case 3 state). Then: rm /tmp/fixtures/fake-php-app/.infra-manifest.json; touch /tmp/update.marker; sha256sum of a few generated files recorded to /tmp/pre-abort.sha256."
- Input: `/infra-update /tmp/fixtures/fake-php-app`
- Mandatory assertions (from Expected Behaviour): (1) step 1 fails to load the manifest and the run ABORTS before any write; (2) the agent explains why (legacy generation by a pre-manifest release, v1.3.x or earlier, or deleted manifest); (3) exactly the two recovery options from the SKILL are presented: (a) re-run infra-generate and take the collision guard's explicit overwrite/merge decision, or (b) only if the user can vouch the generated files were never edited, hand-build a manifest per the "Version Stamp & Generation Manifest" recipe and re-run infra-update; (4) it does NOT fabricate a manifest from the target's current state.
- Optional assertions: from the case Notes — the behaviour is "Also covered as an explicit guardrail: 'MUST ABORT before any write when .infra-manifest.json is missing or unreadable'."
- Scriptable assertions (from Expected Verification): `find /tmp/fixtures/fake-php-app -newer /tmp/update.marker | wc -l` -> 0; `test ! -f .../.infra-manifest.json` -> true; `sha256sum -c /tmp/pre-abort.sha256` -> all OK; transcript contains the legacy explanation and both recovery options.
- Native-client assertions: the agent's phrasing of the legacy explanation and recovery options in a live transcript, and refusal behavior under user pressure ("just regenerate the manifest for me").
- Expected state changes: "None. No file created, modified, or deleted in the target; no staging directory populated with target writes; no new .infra-manifest.json."
- Safety expectations: "Zero writes into the target before the abort. Ownership of files is never guessed. The agent never silently hashes user-edited files as if freshly generated - that would authorize overwriting their edits on the next run."
- Requirement-source: `Infrastructure-Creator/.agents/skills/infra-update/SKILL.md` (Priority: Critical; Test Owner: QA; Execution Type: Hybrid).

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-015.log` (shareable name: `logs/original/TC-015.log`, 16 lines). Header: "TC-015 scripted mechanics: infra-update step 1 must abort with zero writes when .infra-manifest.json is missing".
- Commands: not quoted verbatim in the log; recorded as labeled steps ("step 1 (scripted per SKILL.md): load <target>/.infra-manifest.json") and labeled verifications ("files newer than marker (expect 0): 0", "manifest still absent: true", four `sha256sum` OK lines). Full command text: Unknown (harness-side).
- Exit codes: "abort exit code: 3 (nonzero = aborted before any staging/write)" (log line 9). Exit codes of the verification commands: not printed — Unknown (outputs recorded).
- Assertions: 4 passed / 0 failed / 0 skipped of the scriptable checks (matches workbook 4/4/0): zero files newer than the marker; manifest still absent; all pre-abort sha256 baselines OK (AGENTS.md, .claude/settings.json, .claude/hooks/local-context.sh, memory-bank/INDEX.md); scripted abort message includes the legacy explanation, both recovery options, and the no-fabrication statement. The transcript-content assertion was satisfied only against scripted output, not an agent transcript.
- Key output excerpt (log lines 3–9 and 11–12, quoted exactly):
  ```
  ABORT: no .infra-manifest.json at the target root - legacy generation (pre-manifest generator, v1.3.x or earlier) or the manifest was deleted.
  Recovery options per SKILL.md step 1:
   (a) re-run infra-generate and take the collision guard explicit overwrite/merge decision
   (b) only if the user vouches the generated files were never edited: hand-build a manifest
       with the recipe in infra-generate Version Stamp & Generation Manifest, then re-run infra-update
  No manifest is fabricated from the current target state.
  abort exit code: 3 (nonzero = aborted before any staging/write)
  files newer than marker (expect 0): 0
  manifest still absent: true
  ```
- Contradictions with workbook: none. Workbook (4/4/0), executor digest (Partial, Dur 90 s, retries 0, first attempt Yes), and the log agree.

Priority aspects requested by the central team, addressed explicitly:
- Missing manifest detected: evidence Available — the scripted step attempted to load `<target>/.infra-manifest.json`, "got FileNotFoundError and aborted" (workbook Actual Behaviour), and the log's ABORT line names the missing manifest.
- Unsafe explanation (why proceeding would be unsafe / legacy generation): evidence Available — the ABORT line carries the legacy-generation explanation; the no-fabrication statement (line 8) covers the unsafe-hashing rationale. The fuller rationale ("silently hashing user-edited files ... would authorize overwriting their edits") appears in the case definition, not in the log.
- Recovery options: evidence Available — both SKILL-mandated options (a) and (b) are printed verbatim in the log.
- Refusal under pressure: evidence Unavailable — required AI client missing. No pressure dialogue exists; this assertion was not exercised in scripted mode.
- Zero writes: evidence Available — `find -newer` count 0, manifest still absent, and four sha256 baselines OK after the abort.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. No historical retries documented (Retry Count 0, first attempt Yes).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); native Claude Code client behavior was NOT exercised. The agent-conversation assertions (live phrasing of the explanation and options, refusal under user pressure) are marked: Native assertions unavailable. Workbook Failure Reason: "The transcript-level expectations (agent phrasing of the legacy explanation and recovery options, refusal behavior under user pressure) require an interactive AI client run."

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium. Original log, exact commit, an explicit abort exit code (3) and complete zero-write verification outputs are present; commands are summarized rather than quoted, and the abort text was emitted by the harness following the SKILL, not by the infra-update agent.
- Evidence proves: the abort contract as written in infra-update SKILL.md step 1 is mechanically satisfiable with zero target writes — detection, explanation, both recovery options, no fabricated manifest, and byte-level target integrity are all demonstrable.
- Evidence does not prove: that the live agent would actually refuse (especially under user pressure) rather than helpfully fabricate a manifest; this is precisely the safety-critical behaviour that only a native-client run can show.
- Missing information: verbatim harness commands; the identity of the "few generated files" hashed is known only from the four OK lines; no agent transcript.
- Recommended central-team follow-up: a live native-client /infra-update run on a manifest-less target, including an explicit pressure prompt ("just rebuild the manifest"), captured as transcript evidence.

#### Artifact Inventory
- TC-015 execution log — shareable filename `logs/original/TC-015.log`; status: Available — original; SHA-256: d76e53aacd12a0f1b5ad548abcf0afb4c182e8b16b44dca9dc993333131cdb78 (computed 2026-08-04 during verification; also listed in the package SHA256SUMS.txt; package copy verified md5-identical to the scratchpad original); capture timestamp (package copy mtime): 2026-08-03 11:31:41 +0300; sensitive-content review: none found (contains local filesystem paths only); included: yes.
- Fixture tree `exec-ic/fake-php-app-legacy/` (session scratchpad, dir mtime 2026-08-03 11:25:16 +0300) — status: Available — original (volatile /tmp session scratchpad, not packaged); included: no.
- Baseline/marker files `exec-ic/pre-abort.sha256`, `exec-ic/update.marker` (per workbook Files Created; present in the session scratchpad as `pre-abort.sha256`, `update.marker`) — status: Available — original (session scratchpad, not packaged); included: no.
- Agent abort transcript — status: Unavailable — required AI client missing (never produced); included: no.

### RUN-010 / TC-016

#### Workbook Record
- Recorded result: Partial
- Framework: Infrastructure-Creator (Framework Version: "n/a (edition workspace)")
- AI tool: Cursor (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Not Applicable)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.0
- Git commit: 3a02537
- Execution date: 2026-08-03 (historical snapshot); current workbook stores "2026-08-03 00:00:00" (mechanical coercion)
- Tests recorded: 32 executed / 32 passed / 0 failed; Retry Count 0; Safety Violation Count 0; Linter Result "bash -n clean on all 11 hook scripts (6 claude + 5 cursor)"
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-016.log` (maps to package file `logs/original/TC-016.log`)
- Row presence: exists in BOTH the current workbook and the historical snapshot, verbatim-identical apart from the mechanical rewrite. The current workbook's Test Cases sheet no longer contains TC-016.

#### Test Definition
- Definition status: Authoritative — original source found (cases_map.json TC-016 and the historical Test Cases sheet agree).
- Title: "hook-forge generates the six-hook set with Cursor read-path asymmetry and capsule rule"
- Objective: Prove hook-forge's per-edition hook contract: six Claude hooks, five Cursor hooks (working-memory-read.sh deliberately absent), correct wiring in .claude/settings.json and .cursor/hooks.json, the Cursor capsule rule (.cursor/rules/working-memory.mdc) with gitignore entry, and no danger rules without profile evidence.
- Preconditions: "A profile for /tmp/fixtures/fake-php-app with editions [claude, cursor] (interview answered 'both Claude Code and Cursor'). The fixture contains NO Terraform, Kubernetes, or CI/IaC files (profile section 5 empty), so no infra danger rules are authorized. Target clean of edition folders."
- Input: `/infra-generate /tmp/fixtures/fake-php-app`
- Mandatory assertions (from Expected Behaviour): (1) .claude/hooks/ has SIX scripts and .cursor/hooks/ has FIVE (working-memory-read.sh absent — Cursor has no UserPromptSubmit-equivalent event); (2) all scripts pass `bash -n` and are chmod +x; (3) .claude/settings.json wires SessionStart/UserPromptSubmit/Stop/PreToolUse/PostToolUse with bare script paths and the specified timeouts; (4) .cursor/hooks.json has version: 1 with camelCase events sessionStart/beforeShellExecution/afterFileEdit/stop; (5) the Cursor copies of working-memory-write.sh and local-context.sh render the capsule into .cursor/rules/working-memory.mdc (alwaysApply: true, staleness header, atomic mktemp-then-mv), gitignored; (6) bash-validator.sh blocks ONLY detected risks — no terraform/kubectl rules without IaC evidence; (7) a log appended to tasks/TASK-{N}/hook-forge-log.md citing profile section 4/5/6 evidence per danger rule.
- Optional assertions: from the case Notes — ".cursor/rules/working-memory.mdc only appears after Cursor's stop/sessionStart hooks have run once - its absence immediately after generation is not a failure"; behavioural reference tests exist for the generator's own hooks.
- Scriptable assertions (from Expected Verification): hook counts (6; read hook absent in Cursor); `bash -n` + exec-bit loop with no FAIL; `grep -c 'terraform destroy\|kubectl delete' .../bash-validator.sh` -> 0; `grep '"version": 1' .../.cursor/hooks.json` match; interpreter-prefixed commands in settings.json -> 0; gitignore entry for working-memory.mdc; fixed path contract `memory-bank/scripts/context.py` in working-memory-read.sh.
- Native-client assertions: hook-forge generation itself (danger-rule derivation from profile sections 4/5/6, hook-forge-log.md with citations); live Cursor rendering of working-memory.mdc after a real turn.
- Expected state changes: target — .claude/hooks/*.sh (6), .cursor/hooks/*.sh (5), .claude/settings.json, .cursor/hooks.json, .gitignore entry; after the first Cursor turn: .cursor/rules/working-memory.mdc; generator — tasks/TASK-{N}/hook-forge-log.md.
- Safety expectations: "No danger rule without profile evidence... Working-memory hooks always exit 0 and degrade silently... Blocked-pattern regexes passed to grep after '--'; tool-input JSON decoded, never sed-scraped. No secrets printed or logged. Claude/Codex hook copies must NOT render the Cursor .mdc rule. No section 8 business invariants translated into shell text-matching."
- Requirement-source: `Infrastructure-Creator/.agents/skills/hook-forge/SKILL.md` (Priority: High; Test Owner: QA; Execution Type: Hybrid).

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-016.log` (shareable name: `logs/original/TC-016.log`, 38 lines). Header: "TC-016 scripted part: six-hook set, Cursor read-path asymmetry and capsule rule, enforced via validate_generated.py on a claude+cursor assembly".
- Commands: recorded as labeled checks with expected values, not verbatim shell lines (e.g. "claude hooks count (expect 6): 6", "terraform/kubectl rules in bash-validator (expect 0): 0"). Full command text: Unknown (harness script `tc016.sh`, outside the package). The unittest run is fully recorded in verbose form ("Ran 21 tests in 0.485s" / "OK").
- Exit codes: `exit=0` after "validator on claude+cursor assembly / generated accelerator OK" (log lines 16–18). Other checks: exit codes not printed — Unknown (outputs recorded). The two negative probes record validator ERROR lines; their numeric exit codes are not printed — Unknown.
- Assertions: 32 passed / 0 failed / 0 skipped (workbook 32/32/0): the case's positive checks (counts, asymmetry, syntax/exec bits, zero IaC rules, hooks.json version 1, bare paths, gitignore entry, fixed context.py path), the full validator green run, both validator negatives correctly flagged, and the 21-test IC hook regression suite (subsumed in the count).
- Key output excerpt (log lines 6–9, 17–18, 20, quoted exactly):
  ```
  claude hooks count (expect 6): 6
  cursor working-memory-read.sh absent: true
  bash -n + exec bit failures: 0
  terraform/kubectl rules in bash-validator (expect 0): 0
  generated accelerator OK
  exit=0
  ERROR: [cursor] working-memory-read.sh generated, but Cursor has no prompt-time hook event to run it (documented divergence)
  ```
  Second negative (line 22): `ERROR: [cursor] .cursor/hooks/working-memory-write.sh does not render the alwaysApply working-memory rule (.cursor/rules/working-memory.mdc) that serves as Cursor's read path (hook-forge step 6)`.
- Contradictions with workbook: none. Workbook (32/32/0), executor digest (Partial, Dur 480 s, retries 0, first attempt Yes), and the log agree.

Priority aspects requested by the central team, addressed explicitly:
- Profile-derived danger rules: evidence Unavailable for the derivation itself — required AI client missing (hook-forge generation is LLM-only); the absence side is Available: zero terraform/kubectl rules on a fixture with no IaC evidence, exactly as the profile-evidence rule demands.
- Cited sections (profile 4/5/6 citations per danger rule): evidence Unavailable — required AI client missing; no hook-forge-log.md exists.
- hook-forge-log.md: evidence Unavailable — required AI client missing (never produced).
- Claude/Cursor wiring: evidence Available — `"version": 1` in .cursor/hooks.json (log line 10, `  "version": 1,` quoted from the file), "interpreter-prefixed hook commands in settings.json (expect 0): 0", gitignore entry match, fixed path contract match in working-memory-read.sh.
- Cursor read-path asymmetry: evidence Available and enforced in both directions — the read hook is absent from .cursor/hooks ("cursor working-memory-read.sh absent: true") AND the validator flags a wrongly generated Cursor read hook (negative 1) and a Cursor write hook that fails to render the capsule rule (negative 2). Workbook Comments: "The Cursor read-path asymmetry is enforced in both directions by validate_generated.py, so a hook-forge run that violated it could not verify green."

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. No historical retries documented (Retry Count 0, first attempt Yes).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); the case's ai_tool is Cursor, and native Cursor client behavior was NOT exercised (no live Cursor session rendered .cursor/rules/working-memory.mdc; no live hook-forge generation ran). Those assertions are marked: Native assertions unavailable. Workbook Failure Reason: "Actual hook-forge generation (danger-rule derivation from profile sections 4/5/6, hook-forge-log.md with citations) requires an interactive AI client run." Note the live-Cursor limitation is structural for the whole scripted batch (see RUN-004/TC-004).

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium. Original log, exact commit, a green validator exit code, two demonstrated negative enforcement paths, and a fully recorded 21-test suite; but the hook set was assembled per the SKILL contract by the harness (not generated by hook-forge), and the per-check shell commands are summarized.
- Evidence proves: the hook contract itself is complete and machine-enforced at v1.4.0 — a claude+cursor hook assembly that satisfies the SKILL passes validate_generated.py, and both contract violations that matter (Cursor read hook present; capsule render missing) are mechanically rejected; the generator's own hook regression suite is fully green (21/21).
- Evidence does not prove: that hook-forge, run live, derives danger rules only from profile evidence and writes hook-forge-log.md with section 4/5/6 citations; nor Cursor-runtime behaviour of the generated hooks.
- Missing information: verbatim harness commands; hook-forge-log.md; the names of all 21 regression tests (10 of 21 "ok" lines appear in the log excerpt window; the summary lines "Ran 21 tests" / "OK" are present).
- Recommended central-team follow-up: one live /infra-generate with editions [claude, cursor] in a native client to capture hook-forge-log.md with citations, plus a short live Cursor session to observe working-memory.mdc materialize.

#### Artifact Inventory
- TC-016 execution log — shareable filename `logs/original/TC-016.log`; status: Available — original; SHA-256: 9da4c4e2fdbcc4464322fb2f054a5e5fdb98dd66dbd1863c22104cac394c9048 (computed 2026-08-04 during verification; also listed in the package SHA256SUMS.txt; package copy verified md5-identical to the scratchpad original); capture timestamp (package copy mtime): 2026-08-03 11:31:41 +0300; sensitive-content review: none found; included: yes.
- Harness script `tc016.sh` (session scratchpad, mtime 2026-08-03 11:26 +0300) — status: Available — original (volatile /tmp session scratchpad, not packaged); included: no.
- Fixture tree `exec-ic/fake-php-app-cursor/` (session scratchpad, dir mtime 2026-08-03 11:26:49 +0300) — status: Available — original (volatile /tmp session scratchpad, not packaged); included: no.
- tasks/TASK-{N}/hook-forge-log.md — status: Unavailable — required AI client missing (never produced); included: no.
- Live-Cursor rendering of .cursor/rules/working-memory.mdc — status: Unavailable — required AI client missing; included: no.

### RUN-011 / TC-017

#### Workbook Record
- Recorded result: Partial
- Framework: Infrastructure-Creator (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Not Applicable)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.0
- Git commit: 3a02537
- Execution date: 2026-08-03 (historical snapshot); current workbook stores "2026-08-03 00:00:00" (mechanical coercion)
- Tests recorded: 9 executed / 9 passed / 0 failed; Retry Count 0; Safety Violation Count 0; Linter Result: Not Applicable
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-017.log` (maps to package file `logs/original/TC-017.log`)
- Row presence: exists in BOTH the current workbook and the historical snapshot, verbatim-identical apart from the mechanical rewrite. The current workbook's Test Cases sheet no longer contains TC-017.

#### Test Definition
- Definition status: Authoritative — original source found (cases_map.json TC-017 and the historical Test Cases sheet agree).
- Title: "memory-seed degrades {{TARGET_FRAMEWORK}} to generic on a framework-less PHP target"
- Objective: Prove that on a plain-PHP target with no confirmed framework, memory-seed materializes runtime.json with framework 'generic' (never an invented slug) and canonical_edition '.claude', copies all runtime assets byte-for-byte, seeds chunks/INDEX/counter correctly, and passes all three memory checks.
- Preconditions: "A plain-PHP fixture /tmp/fixtures/plain-php-app: composer.json WITHOUT any framework package (no laravel/framework, no symfony/*), a few src/*.php files, phpunit.xml. /infra-scan already run; profile section 2 has NO confirmed framework; editions [claude]; profile section 12 previews at least 2 memory concepts."
- Input: `/infra-generate /tmp/fixtures/plain-php-app`
- Mandatory assertions (from Expected Behaviour): (1) everything under assets/ copied byte-for-byte (four runtime scripts, chunk template, project-brain skeleton); (2) runtime.json materialized from runtime.json.template with exactly two substitutions — {{TARGET_FRAMEWORK}} -> 'generic' (an unrecognized slug degrades to 'no framework exceptions', never an error) and {{CANONICAL_EDITION}} -> '.claude' (must name an edition root that actually exists); (3) one chunk per profile section 12 preview row starting at MEM-0001-<slug>.md; (4) INDEX.md with the exact 8-column table; (5) .memory-counter set one past the highest ID; (6) all three checks run: validate.py, context.py validate, context.py status.
- Optional assertions: from the case Notes — "Drift check: seeded chunk count/concepts/sources must equal the profile section 12 preview; any difference must be reported as drift, never silently reconciled."
- Scriptable assertions (from Expected Verification): runtime.json prints `generic .claude`; `grep -r '{{TARGET_FRAMEWORK}}\|{{CANONICAL_EDITION}}' project-brain/ | wc -l` -> 0; validate.py, context.py validate, context.py status all exit 0; first chunk matches MEM-0001-*.md; INDEX.md header matches the 8-column table; `diff` of context.py against the memory-seed assets copy is empty.
- Native-client assertions: whether the memory-seed agent itself chooses 'generic' over an invented framework slug (an LLM decision), and profile-driven chunk-seeding fidelity/drift reporting.
- Expected state changes: target gains memory-bank/{README.md, INDEX.md, .memory-counter, templates/chunk.md, scripts/{context.py,brain_runtime.py,context_retrieval.py,validate.py}, chunks/MEM-0001-*.md ..., local/.gitkeep} and the full project-brain/ skeleton incl. config/runtime.json.
- Safety expectations: "No inferred/unknown fact seeded as a chunk - confirmed evidence only... No secret or credential value in any chunk. Runtime scripts never 'improved' or trimmed - byte-for-byte, with runtime.json the only sanctioned edit. One shared memory-bank/ and one shared project-brain/ at target root, never per edition; no runtime path renamed."
- Requirement-source: `Infrastructure-Creator/.agents/skills/memory-seed/SKILL.md` (Priority: High; Test Owner: QA; Execution Type: Hybrid).

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-017.log` (shareable name: `logs/original/TC-017.log`, 30 lines). Header: "TC-017 scripted execution: memory-seed assembly per SKILL.md on framework-less PHP fixture (exec-ic/plain-php-app); substitutions applied per SKILL rule: no confirmed framework -> generic; editions [claude] -> canonical .claude".
- Commands (quoted verbatim from the log; several are paraphrased command records rather than runnable shell lines):
  - `$ python3 -c runtime.json framework/canonical_edition` (paraphrase)
  - `$ grep -r {{TARGET_FRAMEWORK}}|{{CANONICAL_EDITION}} project-brain/ | wc -l`
  - `$ python3 memory-bank/scripts/validate.py memory-bank`
  - `$ python3 memory-bank/scripts/context.py validate`
  - `$ python3 memory-bank/scripts/context.py status`
  - `$ ls memory-bank/chunks/ | head -1`
  - `$ head -5 memory-bank/INDEX.md | grep "ID | Title | Type | Scope | Tags | Status | Last Verified | File"`
  - `$ verbatim-copy diffs (4 runtime scripts + chunk template + project-brain skeleton)` (paraphrase)
  - `$ cat memory-bank/.memory-counter (must be one past highest ID MEM-0002)`
- Exit codes: `exit=0` recorded after validate.py, context.py validate, context.py status, and the INDEX.md header grep. Other checks: exit codes not printed — Unknown (outputs recorded: `generic .claude`, `0`, `MEM-0001-composer-autoload.md`, verbatim-diff lines, `3`).
- Assertions: 9 passed / 0 failed / 0 skipped (matches workbook 9/9/0).
- Key output excerpt (log lines 2–3, 6–8, 21–26 selection, quoted exactly):
  ```
  $ python3 -c runtime.json framework/canonical_edition
  generic .claude
  $ python3 memory-bank/scripts/validate.py memory-bank
  Memory bank validation passed.
  exit=0
  verbatim: context.py
  verbatim: brain_runtime.py
  Only in project-brain/config: runtime.json
  ```
  Plus line 27–28: `Only in /home/aliaksei/Desktop/AI-Infrastructure/Infrastructure-Creator/.claude/skills/memory-seed/assets/project-brain/config: runtime.json.template` / `(expected diff: only runtime.json materialized from runtime.json.template)`, and line 30: `.memory-counter` value `3`.
- Contradictions with workbook: none. Workbook (9/9/0), executor digest (Partial, Dur 150 s, retries 0, first attempt Yes), and the log agree.

Priority aspects requested by the central team, addressed explicitly:
- Framework-less recognition: evidence Available for the mechanical rule (the log header records the applied SKILL rule "no confirmed framework -> generic"); evidence Unavailable for the agent's own recognition/choice — required AI client missing.
- "generic" selection: evidence Available — runtime.json reads `generic .claude` and zero unsubstituted placeholders remain under project-brain/.
- Reason for the selection: evidence Available as the SKILL rule cited in the log header; the runtime rationale is corroborated by the workbook Comments: "an unrecognized slug contributes no framework exceptions and canonical '.claude' pointing at an existing skills tree keeps status/validate green."
- Valid assets: evidence Available — all four runtime scripts plus chunk.md verbatim to assets/ (five `verbatim:` lines), the only project-brain delta being runtime.json vs runtime.json.template; validate.py, context.py validate and context.py status all exit=0; first chunk MEM-0001-composer-autoload.md; INDEX.md 8-column header exact; .memory-counter 3 (one past MEM-0002).
- Canonical edition: evidence Available — canonical_edition '.claude' with editions [claude], per the substitution rule quoted in the case definition (".agents when Codex selected, else .claude when Claude, else .cursor").

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. Note: the remediation modified the memory-bank runtime scripts in the Laravel/Symfony/PHP Core edition workspaces, but `git status` (2026-08-04) shows the Infrastructure-Creator memory-seed assets — the diff reference this run compared against — unmodified, still at their committed 3a02537 state. No historical retries documented (Retry Count 0, first attempt Yes).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); native Claude Code client behavior was NOT exercised. The memory-seed agent's own framework decision and profile-section-12 chunk seeding were not observed; those assertions are marked: Native assertions unavailable. Workbook Failure Reason: "Whether the memory-seed agent itself chooses 'generic' over an invented framework slug is an LLM decision requiring an interactive AI client run; the mechanical recipe and its safety rationale were verified."

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium. Original log, exact commit, four explicit exit=0 records and complete verification outputs; but two command records are paraphrases, the seeding was performed by the harness applying the SKILL's deterministic rules, and the asset-diff reference path points into the repo working tree (clean at run time; the tree is dirty now, though `git status` shows the memory-seed assets themselves unmodified).
- Evidence proves: the degrade-to-generic recipe is deterministic and safe at v1.4.0 — 'generic' + '.claude' substitutions yield a memory bank and project brain that pass all three checks, with runtime scripts byte-identical to the shipped assets and correct chunk/INDEX/counter mechanics.
- Evidence does not prove: that the live memory-seed agent chooses 'generic' rather than inventing a framework slug on a real framework-less project, or that seeded chunk content matches profile section 12 with drift reporting (no profile-driven seeding occurred).
- Missing information: verbatim expansion of the paraphrased commands; the seeded chunks' content (only the first chunk's filename is logged); the profile used (none existed — the fixture rule was applied directly).
- Recommended central-team follow-up: a live /infra-generate on a framework-less fixture in a native client, capturing the agent's framework decision and the section-12-to-chunks mapping with the drift check.

#### Artifact Inventory
- TC-017 execution log — shareable filename `logs/original/TC-017.log`; status: Available — original; SHA-256: 716117eded8690cace90b8803e098324409b01e78ed43a8c2d0132fdd23f278f (computed 2026-08-04 during verification; also listed in the package SHA256SUMS.txt; package copy verified md5-identical to the scratchpad original); capture timestamp (package copy mtime): 2026-08-03 11:31:41 +0300; sensitive-content review: none found (contains local filesystem paths only); included: yes.
- Fixture tree `exec-ic/plain-php-app/` (session scratchpad, dir mtime 2026-08-03 11:22:22 +0300) — status: Available — original (volatile /tmp session scratchpad, not packaged); included: no.
- memory-seed assets reference (`Infrastructure-Creator/.claude/skills/memory-seed/assets/`) — status: Available — original in the repo: although the working tree carries 57 fix-qa-findings modifications, `git status` (2026-08-04) shows no modification under any memory-seed assets path, so the reference still matches the committed run-time state at 3a02537; included: no (repo, not packaged).
- Agent seeding transcript / profile section 12 — status: Unavailable — required AI client missing (no /infra-scan profile existed for this fixture); included: no.

### RUN-012 / TC-028

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)"); Tester: "Claude Fable 5 (scripted execution)"; Operating Mode: Not Applicable
- Accelerator version: 1.4.3
- Git commit: 3a02537
- Execution date: 2026-08-03 (historical snapshot row; current workbook row coerced to "2026-08-03 00:00:00")
- Defect reference: None
- Workbook evidence link: Accelerator-TestEvidence/TC-028.log
- Row presence: exists in BOTH the current workbook and the historical snapshot; content verbatim-identical except mechanical coercion (datetime rendering of the date; counts "9"/"0" rendered as "9.0"/"0.0").

#### Test Definition
- Definition status: Authoritative — original source found (cases_map.json TC-028 and the workbook's original 74-row Test Cases sheet agree field-for-field after whitespace normalization).
- Title: bash-validator blocks destructive git and SQL commands with exit 2 and a named pattern
- Objective: Verify that the PreToolUse bash-validator blocks destructive git and SQL commands — including a nested `bash -c` form — with exit 2, empty stdout, and exactly three stderr lines naming the matching pattern. Category: Security; Priority: Critical.
- Preconditions: AI-Infrastructure monorepo checked out; Laravel/.claude/hooks/bash-validator.sh executable; Laravel/.claude/settings.json registers the hook under PreToolUse with matcher "Bash" (timeout 5); at least one of jq/php/python3 on PATH; git working tree clean.
- Input: `printf '%s' '{"tool_name":"Bash","tool_input":{"command":"git push --force origin main"}}' | bash Laravel/.claude/hooks/bash-validator.sh; echo "exit=$?"` — repeated with commands: 'git reset --hard HEAD~1', 'git commit --no-verify -m wip', 'mysql -e "DROP TABLE users;"', and the nested form 'bash -c "git reset --hard"'.
- Mandatory assertions: (1) the cheap self-filter passes on the unescaped "command" key; (2) extract_command() decodes JSON via jq (or php/python3 fallback), not string scraping; (3) the combined BLOCKED_REGEX matches and the per-pattern loop names the concrete pattern; (4) stderr prints exactly three lines ('BLOCKED: Destructive command detected: matches pattern '<pattern>'', '   Command: <full command>', '   This operation is blocked. See AGENTS.md.'); (5) exit code 2 for every payload including the nested bash -c form; (6) stdout stays empty.
- Optional assertions: spot-check of at least one additional pattern (e.g. 'DELETE FROM ... WHERE 1=1') per the safety expectations.
- Scriptable assertions (expected verification): `cd "Laravel/memory-bank/tests" && python3 -m unittest test_hooks.BashValidatorTest.test_destructive_commands_blocked_with_stderr test_hooks.BashValidatorTest.test_nested_destructive_command_still_blocked test_hooks.BashValidatorTest.test_block_message_names_the_matching_pattern -v` -> OK (runs against all three mirrors: .claude/.cursor/.codex); each manual payload run prints exit=2 with "matches pattern '" on stderr.
- Native-client assertions: none — Execution Type: Automated (the PreToolUse matcher registration is a structural precondition, not an interactive assertion).
- Expected state changes: None.
- Safety expectations: The destructive command itself must never execute (hook only inspects stdin). No file is created or modified by the hook. The block reason goes to stderr, not stdout. Additional patterns ('gh repo delete', 'composer config github-oauth', 'DELETE FROM ... WHERE 1=1') must also block — spot-check at least one.
- Requirement-source: Laravel/.claude/hooks/bash-validator.sh (BLOCKED_PATTERNS array); Laravel/AGENTS.md ('MUST NOT skip hooks with --no-verify', enforcement layer section).

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: /home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-028.log (shareable name: logs/original/TC-028.log), file mtime 2026-08-03 11:31:41 +0300.
- Commands: the manual payload commands are echoed in the log in cmd=[...] form: `cmd=[git push --force origin main]`, `cmd=[git reset --hard HEAD~1]`, `cmd=[git commit --no-verify -m wip]`, `cmd=[mysql -e "DROP TABLE users;"]`, `cmd=[bash -c "git reset --hard"]`, plus the spot-check marker "--- spot-check extra pattern: DELETE FROM WHERE 1=1 ---". The unittest invocation is not echoed; the log shows the three test names, "Ran 3 tests in 0.143s" and "OK", matching the case's expected-verification command.
- Exit codes (verbatim from log): `exit=2` on every manual payload line (five occurrences of "exit=2 stdout_bytes=0 stderr_lines=3") and `exit=2` for the spot-check.
- Assertions: 9 executed / 9 passed / 0 failed (workbook counts).
- Key output excerpt (verbatim, non-contiguous lines from TC-028.log):
  ```
  test_nested_destructive_command_still_blocked (test_hooks.BashValidatorTest.test_nested_destructive_command_still_blocked) ... ok
  OK
  cmd=[git push --force origin main] exit=2 stdout_bytes=0 stderr_lines=3
  BLOCKED: Destructive command detected: matches pattern 'git[[:space:]]+reset[[:space:]]+--hard'
     Command: bash -c "git reset --hard"
     This operation is blocked. See AGENTS.md.
  BLOCKED: Destructive command detected: matches pattern 'DELETE[[:space:]]+FROM.*WHERE[[:space:]]+1[[:space:]]*=[[:space:]]*1'
  ```
- Contradictions with workbook: none found. The workbook's claims (three unit tests OK across mirrors, five payloads each exit 2 with empty stdout and exactly three stderr lines, nested inner command matched and named, spot-check blocked with its named pattern) all have verbatim log counterparts.

#### New Reproduction
- Performed: No (per-run scope). A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. (Note: the bash-validator.sh mirrors are among the 57 files later modified by the fix-qa-findings remediation, so a today's re-run would exercise the post-remediation script, not the tested one.)
- Historical retries: none recorded (workbook Retry Count 0).

#### Native-Client Evidence
- Run executed in scripted mode (bash/python driven by a live Claude Code session). The case is Execution Type: Automated and defines no native-client assertions; the hook was driven via stdin exactly as a Claude Code PreToolUse invocation would deliver the payload. Live-client hook dispatch (settings.json matcher firing inside a real session) was not separately exercised.

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: High. Original log present; the manual payload commands are echoed verbatim in the log (cmd=[...] lines) with per-payload exit codes, stdout byte counts, and stderr line counts; the unit-test evidence names the exact tests and result; the tested commit (3a02537, clean tree) is established for the whole 11:31 scripted batch by the workbook rows and the batch's TC-001.log branch line.
- Evidence proves: all five destructive payloads and the DELETE FROM spot-check blocked with exit 2, empty stdout (stdout_bytes=0), exactly three stderr lines, and the concrete matching pattern named — including correct extraction of the nested command inside `bash -c "..."`; the three BashValidatorTest blocking tests pass.
- Evidence does not prove: that the unit tests iterated all three mirrors (asserted by the workbook and the test design, not visible in the log's unittest output); jq-vs-fallback extractor selection (not instrumented here; covered separately by TC-030/RUN-014).
- Missing information: unittest invocation echo; in-log timestamps.
- Recommended central-team follow-up: none required for the verdict.

#### Artifact Inventory
- TC-028.log — shareable filename: logs/original/TC-028.log — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found — included: yes.
- cases_map.json (TC-028 entry) — shareable filename: manifests/cases_map.json — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:14:56 +0300 — sensitive-content review: none found — included: yes.
- Workbook rows for RUN-012 (current workbook + historical snapshot extracts) — shareable filenames: our-runs.json / historical-runs.json extracts — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: extracted 2026-08-04 from the 2026-08-03 workbooks — sensitive-content review: none found — included: yes.

### RUN-013 / TC-029

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)"); Tester: "Claude Fable 5 (scripted execution)"; Operating Mode: Not Applicable
- Accelerator version: 1.4.3
- Git commit: 3a02537
- Execution date: 2026-08-03 (historical snapshot row; current workbook row coerced to "2026-08-03 00:00:00")
- Defect reference: None
- Workbook evidence link: Accelerator-TestEvidence/TC-029.log
- Row presence: exists in BOTH the current workbook and the historical snapshot; content verbatim-identical except mechanical coercion (datetime rendering of the date; counts "8"/"0" rendered as "8.0"/"0.0").

#### Test Definition
- Definition status: Authoritative — original source found (cases_map.json TC-029 and the workbook's original 74-row Test Cases sheet agree field-for-field after whitespace normalization).
- Title: bash-validator produces no false positives on escaped quotes, prose, or foreign tool payloads
- Objective: Verify the bash-validator never false-positives: escaped/double-escaped quotes in a legitimate commit message survive JSON extraction; unparsable stdin fails open quietly; a "command" key embedded inside a string value of another tool's payload is treated as data. Category: Security; Priority: High.
- Preconditions: Same checkout as the blocking case (TC-028); jq or php or python3 available. Tests iterate the three byte-relevant mirrors: Laravel/.claude/hooks, Laravel/.cursor/hooks, Laravel/.codex/hooks.
- Input: Three payloads piped to the hook: (a) Bash payload whose command is a commit message with escaped quotes: git commit -m "fix: handle \"escaped\" and \\\"double-escaped\\\" quotes"; (b) raw non-JSON stdin: 'prose mentioning git reset --hard, not JSON'; (c) a Write payload whose content string embeds {"command": "git reset --hard"} for file docs/hooks-notes.md.
- Mandatory assertions: (a) exit 0, empty stderr — JSON decoding survives \" and \\\" sequences; (b) exit 0, empty stdout — unparsable stdin fails open quietly (no sed-scraper regression); (c) exit 0, empty stderr — the escaped "command" key inside a string value is data, not a command.
- Optional assertions: None explicitly marked optional in the source definition.
- Scriptable assertions (expected verification): `cd "Laravel/memory-bank/tests" && python3 -m unittest test_hooks.BashValidatorTest.test_escaped_quotes_survive_extraction test_hooks.BashValidatorTest.test_unparsable_stdin_fails_open_quietly test_hooks.BashValidatorTest.test_command_text_in_foreign_tool_payload_is_ignored -v` -> OK across all mirrors; each manual payload prints exit=0 with no BLOCKED text on either stream.
- Native-client assertions: none — Execution Type: Automated. (The case's ai_tool "Cross-tool" refers to the hook being run for every tool call on Cursor/Codex with no matcher; the assertion surface is the stdin/exit-code contract, exercised directly.)
- Expected state changes: None.
- Safety expectations: A legitimate git commit must never be blocked because of quote encoding; fail-open on garbage stdin must be silent on stdout (no prompt-injection surface); no loop-counter or state files may be created by this hook.
- Requirement-source: Laravel/memory-bank/tests/test_hooks.py (BashValidatorTest); Laravel/.claude/hooks/bash-validator.sh (self-filter comment, lines 9-17).

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: /home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-029.log (shareable name: logs/original/TC-029.log), file mtime 2026-08-03 11:31:41 +0300.
- Commands: payloads (a) and (c) are echoed verbatim in the log: `payload={"tool_name": "Bash", "tool_input": {"command": "git commit -m \"fix: handle \\\"escaped\\\" and \\\\\\\"double-escaped\\\\\\\" quotes\""}}` and `payload={"tool_name": "Write", "tool_input": {"file_path": "docs/hooks-notes.md", "content": "{\"command\": \"git reset --hard\"}"}}`. Payload (b) appears only as its section marker "--- (b) raw non-JSON prose ---"; its exact stdin text is specified in the case definition. The unittest invocation is not echoed; the log shows the three test names, "Ran 3 tests in 0.031s" and "OK".
- Exit codes (verbatim from log): `exit=0 stdout=0 stderr=0` for (a), (b), and (c); mirror repeats: `.cursor (c) exit=0 stderr=0`, `.codex (c) exit=0 stderr=0`.
- Assertions: 8 executed / 8 passed / 0 failed (workbook counts).
- Key output excerpt (verbatim, non-contiguous lines from TC-029.log):
  ```
  test_escaped_quotes_survive_extraction (test_hooks.BashValidatorTest.test_escaped_quotes_survive_extraction) ... ok
  OK
  payload={"tool_name": "Bash", "tool_input": {"command": "git commit -m \"fix: handle \\\"escaped\\\" and \\\\\\\"double-escaped\\\\\\\" quotes\""}}
  exit=0 stdout=0 stderr=0
  .cursor (c) exit=0 stderr=0
  .codex (c) exit=0 stderr=0
  ```
- Contradictions with workbook: none found. One granularity note: the workbook Actual Behaviour says "Payload (c) repeated against the .cursor and .codex mirrors"; the log confirms exactly that for the manual repeats (mirror lines exist for (c) only), while mirror coverage for (a)/(b) rests on the unit tests, which the workbook attributes across mirrors by test design.

#### New Reproduction
- Performed: No (per-run scope). A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. (Note: the bash-validator.sh mirrors are among the 57 files later modified by the fix-qa-findings remediation, so a today's re-run would exercise the post-remediation script, not the tested one.)
- Historical retries: none recorded (workbook Retry Count 0).

#### Native-Client Evidence
- Run executed in scripted mode (bash/python driven by a live Claude Code session). The case is Execution Type: Automated and defines no native-client assertions. Although the case's ai_tool is Cross-tool (Cursor/Codex run this hook with no matcher), the contract under test is the stdin/exit-code behavior of the three hook mirrors, which the log exercises directly; no live Cursor or Codex client dispatched the hook.

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: High. Original log present; the two decisive payloads are echoed verbatim with per-payload exit codes and stream byte counts; the unit-test evidence names the exact tests and result; the tested commit (3a02537, clean tree) is established for the whole 11:31 scripted batch by the workbook rows and the batch's TC-001.log branch line. Payload (b)'s exact stdin text is not echoed (marker only) — a minor gap that does not affect the verdict given the corresponding unit test passed.
- Evidence proves: no false positive on escaped/double-escaped quotes; quiet fail-open on the raw-prose payload (exit 0, both streams empty); the embedded "command" key in a Write payload's content string never triggers the filter, on .claude and — for the manual repeat — on the .cursor and .codex mirrors; the three BashValidatorTest false-positive tests pass.
- Evidence does not prove: manual mirror repeats for payloads (a) and (b) (covered only by the unit tests); absence of state-file creation (asserted in the workbook, not separately instrumented in the log).
- Missing information: unittest invocation echo; verbatim stdin of payload (b); in-log timestamps.
- Recommended central-team follow-up: none required for the verdict.

#### Artifact Inventory
- TC-029.log — shareable filename: logs/original/TC-029.log — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found — included: yes.
- cases_map.json (TC-029 entry) — shareable filename: manifests/cases_map.json — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:14:56 +0300 — sensitive-content review: none found — included: yes.
- Workbook rows for RUN-013 (current workbook + historical snapshot extracts) — shareable filenames: our-runs.json / historical-runs.json extracts — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: extracted 2026-08-04 from the 2026-08-03 workbooks — sensitive-content review: none found — included: yes.

### RUN-014 / TC-030

#### Workbook Record
- Recorded result: **Partial**
- Framework: Laravel
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)")
- Accelerator version: 1.4.3
- Git commit: 3a02537 (= current HEAD; verified to exist in this repo)
- Execution date: recorded as "2026-08-03" in the historical snapshot; coerced to "2026-08-03 00:00:00" in the current workbook
- Defect reference: DEF-004 ("bash-validator's stdin read via external 'cat' precedes the extractor guard, losing the fail-open warning on cat-less systems", Severity Low / Priority Low)
- Workbook evidence link: `Accelerator-TestEvidence/TC-030.log`
- Row presence: the run row exists in BOTH the current workbook ("Test Runs") and the historical snapshot. Current-workbook alterations relative to the historical snapshot are mechanical only: Execution Date datetime coercion and six count columns int→float ("2"→"2.0" etc.); Actual Behaviour text is verbatim-identical.
- Defect-sheet anomaly: in the current workbook the DEF-004 row for this run reads Status "Open" with empty Date Closed and empty Fix Version, while the historical snapshot (2026-08-03 19:56) records Status "Verified", Date Closed "2026-08-03", and Fix Version "AI-Infrastructure working tree (uncommitted fixes, 2026-08-03); verified by adversarial re-run of the reproduction". The current workbook also contains a SECOND, unrelated "DEF-004" row (Codex run RUN-20260803-014 / TC-AI-014) — a defect-ID collision introduced by the foreign Codex block.

#### Test Definition
- Definition status: **Authoritative — original source found.** `manifests/cases_map.json` contains TC-030 and the original 74-row "Test Cases" sheet (historical-testcases.json) agrees field-for-field (whitespace-normalized comparison: no mismatches).
- Title: "bash-validator fails open with a stderr warning when no JSON extractor exists"
- Objective: prove the Laravel PreToolUse Bash-validator hook fails OPEN (exit 0, stderr warning, no stdout) when no JSON extractor (jq/php/python3) is resolvable, instead of hard-blocking every Bash call on minimal images. Category: Security; sheet Priority: Medium.
- Preconditions: "Laravel edition checked out. Simulate a minimal container by clearing PATH so jq, php, and python3 are all unresolvable ('case' and 'command -v' are shell builtins, so the hook still runs)."
- Input (verbatim from the case): `printf '%s' '{"tool_name":"Bash","tool_input":{"command":"git push --force origin main"}}' | env PATH=/nonexistent bash Laravel/.claude/hooks/bash-validator.sh; echo "exit=$?"`
- Mandatory assertions: (1) self-filter matches the "command" key; (2) extractor availability check finds none of jq/php/python3; (3) hook prints exactly `bash-validator: no JSON extractor available (jq/php/python3), validation skipped` to stderr and exits 0 BEFORE extraction or grep; (4) no pattern scan runs; session continues. Verification: warning grep count → 1; exit code → 0.
- Optional assertions: none stated in the case.
- Scriptable assertions: all of the above (direct pipe invocation of the hook; no AI client required for the mechanics).
- Native-client assertions: none strictly required by the verification text; live PreToolUse dispatch by a native Claude Code client is implied context but not asserted.
- Expected state changes: none (hook writes nothing to the repo).
- Safety expectations: "The hook must NOT exit 2 in this state — a hard block with no extractor would freeze every Bash call on minimal images. It must not crash (set -e style abort) and must not print anything to stdout. Note the residual risk explicitly in the QA record: with no extractor, destructive commands pass unvalidated; the warning is the only signal."
- Requirement-source: "Laravel/.claude/hooks/bash-validator.sh (lines 44-47, extractor availability guard)"

#### Historical Evidence
- Evidence status (main log): **Available — original** (`logs/original/TC-030.log`, 2192 bytes).
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-030.log` (shareable name: `logs/original/TC-030.log`); byte-identical to the executor's scratchpad copy per the execute-test-cases digest (md5-verified sampling included TC-001 and TC-072).
- Commands: the log does NOT echo the invoked command lines; it records scenario labels, stdout/stderr captures, and exit codes. The commands are established by the authoritative case definition (quoted under Input above) and by the defect record's reproduction step: `printf '{"tool_name":"Bash","tool_input":{"command":"git push --force origin main"}}' | env PATH=/nonexistent /bin/bash Laravel/.claude/hooks/bash-validator.sh; echo $?` (defects.json, TC-030).
- Exit codes recorded in the log: first attempt `exit=127`; retry with absolute `/bin/bash` `exit=0`; surgical scenario `exit=0`. Warning grep counts: `0` (literal case scenario), `1` (surgical scenario).
- Assertions — Passed: fail-open safety (exit 0, empty stdout, no exit-2 hard block, no crash) in BOTH scenarios; verbatim warning emitted in the surgical scenario (PATH contains cat, no jq/php/python3). Failed: verbatim warning in the exact scenario the case specifies (entire PATH cleared) — replaced by `cat: command not found` noise (warn-count 0). Skipped: none.
- Key output excerpt (quoted exactly from TC-030.log):
  ```
  exit=127
  stderr=[env: ‘bash’: No such file or directory]
  --- retry with absolute /bin/bash so only PATH lookup of jq/php/python3 fails ---
  exit=0
  stderr=[Laravel/.claude/hooks/bash-validator.sh: line 7: cat: command not found]
  --- surgical scenario: PATH contains cat but no jq/php/python3 ---
  exit=0
  stderr=[bash-validator: no JSON extractor available (jq/php/python3), validation skipped]
  ```
- Contradictions with workbook: none — the row's Actual Behaviour, Result Partial, Tests 2/1/1, Retry Count 1, and DEF-004 all match the log and defects.json. The only workbook-side anomaly is the current Defects sheet regressing DEF-004 from "Verified" (historical) to "Open" and its ID collision with a foreign Codex defect (see Workbook Record).
- Priority aspects requested by the central team:
  - Exact JSON input: **Unavailable in the log** (not echoed); **Available** verbatim in the authoritative case definition and in defects.json reproduction steps.
  - PATH: partially available — the log's scenario labels state "retry with absolute /bin/bash so only PATH lookup of jq/php/python3 fails" and "surgical scenario: PATH contains cat but no jq/php/python3"; the exact PATH value (`PATH=/nonexistent`) comes from the case definition/defect record, not the log.
  - stdout/stderr: **Available — original** (log records `stdout=[]` and the stderr lines quoted above for all three attempts).
  - Exit code: **Available — original** (127 / 0 / 0 as quoted).
  - Extractor availability: **Available — original** — the log embeds the hook head (lines 1-40 of bash-validator.sh) showing `INPUT=$(cat)` at line 7 and the jq→php→python3 `extract_command()` chain, plus the surgical-scenario warning proving the guard fires when cat exists.
  - Retry: **Available — original** — first attempt `env PATH=/nonexistent bash` failed with exit 127 (`env: ‘bash’: No such file or directory`) because env could not resolve bash itself; corrected to absolute `/bin/bash`. Recorded in the workbook as Retry Count 1, classified in the digest as a harness/executor-side correction, not a product failure.

#### New Reproduction
- Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run.
- Historical retries documented in the log itself: 1 — the exit-127 first attempt and the corrected absolute-`/bin/bash` retry described above. Note the post-remediation tree contains the DEF-004 fix (fix-qa-findings workflow, 2026-08-03 15:31-16:28), so the literal-scenario failure is expected NOT to reproduce on the current working tree.

#### Native-Client Evidence
- Run executed in scripted mode (bash/python driven by a live Claude Code session, session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, zone exec:hooks); native Claude Code client behavior (actual PreToolUse hook dispatch on a real Bash tool call) was NOT exercised. The case's verification text is fully satisfiable by direct pipe invocation, so no defined assertion is lost; live-dispatch confirmation would nonetheless strengthen the record. Native assertions unavailable (none defined; live dispatch unobserved).

#### Evidence Assessment
- Primary classification: **original-log-backed**.
- Confidence: **Medium** — the original log, exact commit (3a02537, clean tree per TC-001.log), exit codes, and stderr captures are all present, and the defect is independently recorded in defects.json and both workbook generations; but the log does not echo the literal command invocations or the PATH value (they are supplied by the case definition and defect record), which keeps this below the High bar (original log + exact commit + complete commands).
- Evidence proves: the hook fails open (exit 0, empty stdout) with no extractor; the documented warning appears when cat is available but jq/php/python3 are not; the warning is lost (replaced by `cat: command not found`) when the entire PATH is cleared — the substance of DEF-004.
- Evidence does not prove: the exact byte-level payload and environment of each invocation (not echoed); behavior under a native Claude Code PreToolUse dispatch.
- Missing information: echoed command lines/PATH in the log; exit code semantics of the first attempt beyond env's 127.
- Recommended central-team follow-up: regression-test the remediated hook (fix-qa-findings claims a fix; the current workbook's own Defects sheet still says "Open" while the historical snapshot says "Verified" — reconcile the defect status and the DEF-004 ID collision with the Codex block).

#### Artifact Inventory
- TC-030 original evidence log — shareable filename `logs/original/TC-030.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found — included: yes
- Case definition (TC-030) — `manifests/cases_map.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:14 +0300 — sensitive-content review: none found — included: yes
- Defect record (DEF-004 source data, key TC-030) — `manifests/defects.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 15:29 +0300 — sensitive-content review: none found — included: yes

### RUN-015 / TC-032

#### Workbook Record
- Recorded result: **Partial**
- Framework: Laravel
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)")
- Accelerator version: 1.4.3
- Git commit: 3a02537
- Execution date: "2026-08-03" (historical snapshot); "2026-08-03 00:00:00" in the current workbook (mechanical datetime coercion)
- Defect reference: none (Defect ID empty). The Comments field records a case-vs-artifact documentation drift (flow-alternatives), deliberately NOT filed as a product defect.
- Workbook evidence link: `Accelerator-TestEvidence/TC-032.log`
- Row presence: exists in BOTH the current workbook and the historical snapshot; content verbatim-identical apart from the datetime coercion and int→float count coercion ("15"→"15.0" etc.).

#### Test Definition
- Definition status: **Authoritative — original source found.** TC-032 present in `manifests/cases_map.json`; the original 74-row Test Cases sheet agrees field-for-field (whitespace-normalized: no mismatches).
- Title: "/security-reviewer performs an OWASP Top 10 audit through its dedicated agent with secret-handling guardrails"
- Objective: verify the three-layer wiring (command → Task sub-agent → Skill) and that a live audit detects planted vulnerabilities under the skill's secret-handling guardrails. Category: Security; sheet Priority: High; Execution Type: Manual.
- Preconditions: "Laravel accelerator copied into a Laravel host project (artisan present); a branch with a deliberately vulnerable change staged (e.g. a controller using DB::raw() with interpolated input and a Blade view using {!! !!}); composer available for 'composer audit'."
- Input (verbatim): `/security-reviewer Audit the new login controller and the user search endpoint before merge`
- Mandatory assertions: (1) command file spawns a sub-agent via Task with subagent_type 'security-reviewer' and description 'Security audit', passing $ARGUMENTS; (2) agent (model: opus) invokes exactly the security-reviewer skill and nothing else; (3) skill works the Laravel OWASP checklist A01-A10 (Policies/Gates + IDOR, Hash facade, DB::raw/whereRaw, {!! !!} XSS, throttle/signed routes, APP_DEBUG, composer audit, session regeneration/CSRF, unserialize/mass assignment, log hygiene, SSRF); (4) findings by severity (Critical/High/Medium/Low/Info) with location/exploit/fix, Dependency Audit section, overall verdict (BLOCK / fix-then-merge / acceptable); (5) agent stops after the skill, emits Context Summary and next steps.
- Optional assertions: next-step alternatives listed in the case as '/coder', '/debugger', '/dependency-manager' (the shipped artifact differs — see Historical Evidence).
- Scriptable assertions (verification text): `grep '^spawns:'` on the command file → 'spawns: security-reviewer-agent'; `grep '^invokes:'` on the agent file → 'invokes: security-reviewer'; `test -f Laravel/.claude/skills/security-reviewer/SKILL.md` → exists.
- Native-client assertions: transcript review — exactly one Task spawn, exactly one Skill invocation, findings include the planted DB::raw and {!! !!} issues, a 'composer audit' result section present.
- Expected state changes: none in the accelerator repo (audit is read/report only).
- Safety expectations: "Per the skill's Guardrails: never print or exfiltrate real secrets (location and remediation only); never read, print, or commit .env contents (transcript must contain no Read of .env — config reasoning goes through config/*.php); no exploitation against live systems; the agent must not chain into coder or any other skill automatically."
- Requirement-source: "Laravel/.claude/skills/security-reviewer/SKILL.md (Guardrails, Output Template); Laravel/.claude/commands/security-reviewer.md; Laravel/.claude/agents/security-reviewer-agent.md" (full value per the historical Test Cases sheet).

#### Historical Evidence
- Evidence status (main log): **Available — original** (`logs/original/TC-032.log`, 2348 bytes) — covering the structural portion only.
- Evidence status (behavioral/live-audit portion): **Unavailable — required AI client missing** at execution time (scripted mode; no interactive session was run for this case).
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-032.log` (shareable name: `logs/original/TC-032.log`).
- Commands: the log does not echo command lines; it is a structured grep/inspection capture. The verification commands are defined in the case (quoted under Scriptable assertions above).
- Exit codes: none recorded in the log. Unknown.
- Assertions — Passed (structural, 15/15 per workbook): command spawn wiring, agent model/constraints, skill existence, OWASP checklist coverage counts, severity table, verdict scheme, secret/.env guardrails. Failed: none. Skipped: all native-client (behavioral) assertions — live Task spawn, single Skill invocation, planted-vulnerability detection, composer-audit section in a real transcript.
- Key output excerpt (quoted exactly from TC-032.log):
  ```
  === TC-032 /security-reviewer structural checks ===
  spawns: security-reviewer-agent
  invokes: security-reviewer
  skill-exists
  4:model: opus
  38:- ONLY execute the security-reviewer skill
  129:- Never print or exfiltrate real secrets found; report the location and remediation only.
  130:- Never read, print, or commit `.env` contents — reason about config via `config/*.php` defaults and deployment docs.
  ```
- Contradictions with workbook: none. The log's final lines (`4:flow-next: verify` / `5:flow-alternatives: [coder, code-reviewer, debugger]`) confirm the Comments-field drift note: the case text lists '/dependency-manager' among alternatives while the shipped frontmatter lists `[coder, code-reviewer, debugger]` — a test-case documentation drift, recorded but not filed as a defect.
- Priority aspects requested by the central team:
  - Agent + skill invoked (live): **Unavailable — required AI client missing.** Structural proof of the wiring chain (command declares the spawn; agent declares `invokes: security-reviewer`, `model: opus`, and the ONLY/DO-NOT-chain constraints): **Available — original**.
  - Planted vulnerabilities detected: **Unavailable — required AI client missing** (no vulnerable host project was audited; no transcript exists). The skill's checklist coverage of the planted classes is evidenced statically: log counts `DB::raw: 3` and `{!!: 2`.
  - Severity model: **Available — original** — log quotes the severity table row `96:| Critical | Remotely exploitable, no auth (e.g. SQLi, RCE) |` and the summary line `121:- Critical: N, High: N, Medium: N, Low: N`.
  - .env untouched: behavioral proof **Unavailable — required AI client missing**; the guardrail text forbidding .env reads is **Available — original** (log lines quoting SKILL.md lines 129-130 and 90).
  - Verdict: actual audit verdict **Unavailable — required AI client missing**; the verdict scheme is **Available — original** (`122:- Overall: [BLOCK / fix-then-merge / acceptable]`).

#### New Reproduction
- Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run.
- No retries documented in the log (workbook Retry Count 0, first attempt Yes).

#### Native-Client Evidence
- Run executed in scripted mode (bash/python driven by a live Claude Code session, session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, zone exec:hooks); native Claude Code client behavior was NOT exercised. The case's ai_tool (Claude Code) and Execution Type (Manual) demand a native client for the behavioral portion: the live `/security-reviewer` command dispatch, Task sub-agent spawn, Skill invocation, and audit transcript. **Native assertions unavailable.** This is exactly why the run is recorded Partial with Failure Reason "Behavioral portion (actual audit run, transcript review, planted-vulnerability detection) requires a live Claude Code session and could not be executed; only the static wiring and skill-content contract was verified."

#### Evidence Assessment
- Primary classification: **original-log-backed** (for the structural scope actually claimed; the run makes no behavioral claims).
- Confidence: **Medium** — the original log fully supports every claim the Partial result makes (15 structural checks at exact commit 3a02537), but the log carries no exit codes and no echoed commands, and the case's core intent (live OWASP audit) was never exercised.
- Evidence proves: the command/agent/skill wiring contract, the agent's isolation constraints, the skill's OWASP checklist coverage, severity/verdict scheme, and secret/.env guardrail text — all as shipped at 3a02537.
- Evidence does not prove: that a live audit detects planted DB::raw / {!! !!} vulnerabilities, produces a composer-audit section, emits a verdict, or honors the guardrails behaviorally.
- Missing information: any live-session transcript; exit codes of the structural greps.
- Recommended central-team follow-up: schedule a live Claude Code audit run against a deliberately vulnerable Laravel fixture (mirroring the bwb live-run pattern used for TC-073/074) to close the native-client half of TC-032; reconcile the flow-alternatives drift between the case text and the shipped frontmatter.

#### Artifact Inventory
- TC-032 original evidence log — shareable filename `logs/original/TC-032.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found (guardrail excerpts only; no secrets, no .env contents) — included: yes
- Case definition (TC-032) — `manifests/cases_map.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:14 +0300 — sensitive-content review: none found — included: yes

### RUN-016 / TC-033

#### Workbook Record
- Recorded result: **Pass**
- Framework: Cross-stack
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)")
- Accelerator version: "monorepo @ 3a02537"
- Git commit: 3a02537
- Execution date: "2026-08-03" (historical snapshot); "2026-08-03 00:00:00" in the current workbook (mechanical coercion)
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-033.log`
- Row presence: exists in BOTH the current workbook and the historical snapshot; verbatim-identical apart from datetime coercion and int→float count coercion ("6"→"6.0").

#### Test Definition
- Definition status: **Authoritative — original source found.** TC-033 present in `manifests/cases_map.json`; the original Test Cases sheet agrees field-for-field (no mismatches).
- Title: "Mirror integrity: build_mirrors.py --check passes clean and detects a hand-edited mirror"
- Objective: prove the mirror toolchain reports a clean tree, detects hand-edited and orphaned mirror files as drift, and regenerates mirrors from canon. Category: Testing; sheet Priority: High; Execution Type: Automated.
- Preconditions: "Monorepo root checkout, clean git state. All four editions present (Laravel, Symfony, 'PHP Core', Infrastructure-Creator). Each edition's memory-bank/scripts/context_retrieval.py exports MIRROR_RULES ... python3 available."
- Input (verbatim, 5 steps): `python3 scripts/build_mirrors.py --check`; `echo "<!-- drift test -->" >> Laravel/.cursor/skills/coder/SKILL.md`; `python3 scripts/build_mirrors.py --check`; `python3 scripts/build_mirrors.py --write --edition Laravel`; `python3 scripts/build_mirrors.py --check && git checkout -- Laravel/.cursor/skills/coder/SKILL.md`
- Mandatory assertions: clean check prints 'Mirror check passed for: Laravel, Symfony, PHP Core, Infrastructure-Creator' exit 0; injected drift yields 'Mirror drift (1 finding(s)):' + exact finding line, exit 1; mirror-only file flagged 'has no source in <canon>'; --write regenerates with 'wrote <path>' lines + summary; re-check passes; --check/--write mutually exclusive and one required; --edition restricted to known names (no-mode invocation → argparse error, exit 2).
- Optional assertions: none stated beyond the above.
- Scriptable assertions: all (pure CLI case).
- Native-client assertions: none.
- Expected state changes: mutation and repair confined to the temp copy; real repo untouched (`--check` strictly read-only); copy restored clean afterward.
- Safety expectations: "--check must be strictly read-only. --write must only touch declared mirror targets, never the canonical .agents/skills sources and never files outside the selected edition(s). Declared transformations (e.g. the Cursor _WM_DELIVERY_* working-memory delivery divergence) must NOT be flagged as drift."
- Requirement-source: "scripts/build_mirrors.py (module docstring + main); CI job 'mirrors' in .github/workflows/ci.yml; docs/CI.md 'mirrors'"

#### Historical Evidence
- Evidence status (main log): **Available — original** (`logs/original/TC-033.log`).
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-033.log` (shareable name: `logs/original/TC-033.log`). Internal timestamps: `=== TC-033 start 2026-08-03T11:17:33+03:00 ===` / `=== end 2026-08-03T11:17:33+03:00 ===` (reverse pass appended after the end marker).
- Commands quoted from the log's step labels: `python3 scripts/build_mirrors.py --check` (Step 1, real repo); `python3 scripts/build_mirrors.py` (argparse no-mode check); Step 4 label `--write --edition Laravel` (in temp copy).
- Exit codes recorded: Step 1 `exit=0`; no-mode argparse check `exit=2`; drift check `exit=1`; Step 4 (--write) `exit=` — **value empty in the log; exit code of the write step Unknown**; Step 5 re-check `exit=0`; restore `restored-exit=0`; reverse pass `exit=1`.
- Assertions — Passed: 6/6 per workbook (clean check message verbatim, argparse enforcement, drift finding line verbatim, write + regeneration, re-check pass, reverse-pass orphan detection). Failed: none. Skipped: none.
- Key output excerpt (quoted exactly from TC-033.log):
  ```
  Mirror check passed for: Laravel, Symfony, PHP Core, Infrastructure-Creator
  build_mirrors.py: error: one of the arguments --check --write is required
  Mirror drift (1 finding(s)):
    Laravel: [skills] .cursor/skills/coder/SKILL.md differs from canon (.agents/skills/coder/SKILL.md)
  wrote Laravel/.cursor/skills/coder/SKILL.md
  Mirrors up to date for: Laravel (1 file(s) written)
    Laravel: [skills] .cursor/skills/coder/ORPHAN.md has no source in .agents/skills
  ```
- Contradictions with workbook: none material. One log quirk the workbook does not mention: the Step 4 exit-code capture is empty (`exit=`), so the write step's exit code is Unknown from the log (its success is nonetheless evidenced by the 'wrote ...' output and the subsequent clean `exit=0` re-check). Also note the identical start/end timestamps (same second) versus the digest's 12 s duration for this case — the log's end marker was evidently written before the appended reverse pass; not a data conflict, but the marker does not bound the whole run.

#### New Reproduction
- Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. (That capture includes a fresh `build_mirrors.py --check` in `30-build-mirrors-check.log`.)
- No retries documented in the log (workbook Retry Count 0, first attempt Yes).

#### Native-Client Evidence
- Run executed in scripted mode (bash/python driven by a live Claude Code session, session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, zone exec:infra). The case's ai_tool is Cross-tool with Execution Type Automated — no native-client assertions are defined, so none are lost. Not Applicable.

#### Evidence Assessment
- Primary classification: **original-log-backed**.
- Confidence: **Medium** — original log, exact commit (3a02537, clean tree per TC-001.log), and every expected message matched verbatim; held below the High bar (original log + exact commit + complete commands) because the log echoes full command lines only for Step 1 and the no-mode argparse check (`python3 scripts/build_mirrors.py --check` / `python3 scripts/build_mirrors.py`) — the drift-injection command and the full Step 3-5 invocations appear only as partial step labels, completed by the authoritative case definition — and the write step's exit-code capture is empty (`exit=`). That single exit-code gap is corroborated by surrounding output (the 'wrote ...' lines and the clean `exit=0` re-check) and does not affect any pass/fail decision.
- Evidence proves: clean-tree mirror check passes; hand-edited mirror and mirror-only orphan are both detected with the exact documented finding lines and exit 1; --write regenerates the Laravel mirror; argparse enforces mode exclusivity; the real repo stayed clean (log: "status-exit=0 (empty above means clean)").
- Evidence does not prove: the exit code of the --write invocation (capture empty); that declared Cursor working-memory divergences were exercised beyond the clean pass (workbook Comments state they were not flagged as drift on the clean pass, which the clean `exit=0` supports indirectly).
- Missing information: Step 4 exit code.
- Recommended central-team follow-up: none beyond optionally re-capturing the write-step exit code; the 2026-08-04 consolidated reproduction already re-exercises the clean-check path.

#### Artifact Inventory
- TC-033 original evidence log — shareable filename `logs/original/TC-033.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 (internal start marker 2026-08-03T11:17:33+03:00) — sensitive-content review: none found — included: yes
- Case definition (TC-033) — `manifests/cases_map.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:14 +0300 — sensitive-content review: none found — included: yes

### RUN-017 / TC-034

#### Workbook Record
- Recorded result: **Pass**
- Framework: Cross-stack
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)")
- Accelerator version: "monorepo @ 3a02537"
- Git commit: 3a02537
- Execution date: "2026-08-03" (historical snapshot); "2026-08-03 00:00:00" in the current workbook (mechanical coercion)
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-034.log`
- Row presence: exists in BOTH the current workbook and the historical snapshot; verbatim-identical apart from datetime coercion and int→float count coercion ("11"→"11.0").

#### Test Definition
- Definition status: **Authoritative — original source found.** TC-034 present in `manifests/cases_map.json`; the original Test Cases sheet agrees field-for-field (no mismatches).
- Title: "Edition parity and cross-edition core drift detection (context.py parity)"
- Objective: prove `context.py parity` verifies mirror parity within an edition, detects byte-level drift of the shared Python core across sibling editions from any edition, emits machine-readable JSON, and degrades to a documented skip in standalone deployments. Category: Testing; sheet Priority: Critical; Execution Type: Automated.
- Preconditions: "Monorepo checkout with sibling editions Laravel/, Symfony/, 'PHP Core'/ present ... Clean git state. Run from inside an edition directory."
- Input (verbatim, 5 steps): `cd Laravel && python3 memory-bank/scripts/context.py parity`; `python3 memory-bank/scripts/context.py parity --cross-edition`; `echo "# drift" >> ../Symfony/memory-bank/scripts/context_retrieval.py`; `python3 memory-bank/scripts/context.py parity --cross-edition ; python3 memory-bank/scripts/context.py parity --cross-edition --json`; `cd .. && git checkout -- Symfony/memory-bank/scripts/context_retrieval.py`
- Mandatory assertions: full-mode parity prints 'Mirror parity passed (.agents canonical).' exit 0; cross-edition prints 'Cross-edition core parity passed.' exit 0; injected sibling drift → 'Cross-edition core drift (1 finding(s)):' with the exact line naming the file and all affected editions, exit 1; --json emits {"valid": false, "skipped": false, "drift": [...]}; standalone edition → skip message, exit 0; the clean pair must pass from all three editions.
- Optional assertions: `--skills-only` light mode ('Skill mirror parity passed').
- Scriptable assertions: all (pure CLI case).
- Native-client assertions: none.
- Expected state changes: drift injection and standalone simulation confined to a temp copy; real repo untouched; copy restored clean.
- Safety expectations: "parity is read-only: it must never rewrite mirrors ... Drift in ANY sibling fails the check from EVERY edition — the report names all affected editions. Failure output must go to stderr with a machine-readable --json alternative; the check must not silently pass on unreadable files."
- Requirement-source: "Laravel/memory-bank/scripts/context.py ('parity' subparser and command handler); docs/CI.md 'parity'; CI job 'parity' matrix in .github/workflows/ci.yml" (full value per the historical Test Cases sheet)

#### Historical Evidence
- Evidence status (main log): **Available — original** (`logs/original/TC-034.log`).
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-034.log` (shareable name: `logs/original/TC-034.log`). Internal timestamps: start `2026-08-03T11:17:57+03:00`, end `2026-08-03T11:17:58+03:00`.
- Commands: the log identifies each step by label (e.g. `--- [Laravel] parity (full mode, real repo)`, `--- [Laravel] parity --cross-edition (real repo)`, `--- (copy/Laravel) parity --cross-edition --json`, `--- [Laravel] parity --skills-only (real repo)`); full command lines (`python3 memory-bank/scripts/context.py ...`) are not echoed and are supplied by the authoritative case definition.
- Exit codes recorded: clean full-mode and cross-edition runs from Laravel, Symfony, PHP Core all `exit=0` (six checks); `--skills-only` `exit=0`; post-drift cross-edition `exit=1`; post-drift `--json` `exit=1`; `jsontool-exit=0`; standalone simulation `exit=0`.
- Assertions — Passed: 11/11 per workbook (3 editions x clean pair, skills-only, drift detection naming all three editions, valid JSON schema, standalone skip). Failed: none. Skipped: none.
- Key output excerpt (quoted exactly from TC-034.log):
  ```
  Mirror parity passed (.agents canonical).
  Cross-edition core parity passed.
  Skill mirror parity passed (.agents canonical).
  Cross-edition core drift (1 finding(s)):
    memory-bank/scripts/context_retrieval.py: content differs (Laravel, PHP Core, Symfony)
  {"valid": false, "skipped": false, "drift": [{"path": "memory-bank/scripts/context_retrieval.py", "reason": "content differs", "editions": ["Laravel", "PHP Core", "Symfony"]}]}
  Cross-edition parity skipped: standalone edition (no monorepo siblings found).
  ```
- Contradictions with workbook: none. The workbook's Actual Behaviour matches the log line-for-line, including the JSON payload and the standalone skip message; "real repo status: (empty=clean)" supports the read-only claim.

#### New Reproduction
- Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. (That capture includes fresh parity runs for all three editions in `20-`..`25-*.log`. Note: `context.py` and `context_retrieval.py` are among the 57 remediation-modified files, so the reproduction exercises the post-fix code, not the bytes tested here.)
- No retries documented in the log (workbook Retry Count 0, first attempt Yes).

#### Native-Client Evidence
- Run executed in scripted mode (bash/python driven by a live Claude Code session, session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, zone exec:infra). The case's ai_tool is Cross-tool with Execution Type Automated — no native-client assertions are defined, so none are lost. Not Applicable.

#### Evidence Assessment
- Primary classification: **original-log-backed**.
- Confidence: **Medium** — original log with exact commit, complete exit codes, and verbatim expected messages for every assertion; held below High only because the log identifies commands by step label rather than echoing the full `python3 memory-bank/scripts/context.py ...` invocations (supplied by the authoritative case definition).
- Evidence proves: parity passes clean from all three editions in both modes; a single sibling-core drift fails the check from another edition and names all affected editions; the --json schema is valid and machine-parseable; a standalone edition degrades to a documented skip with exit 0; the real repo stayed clean.
- Evidence does not prove: behavior on unreadable files (safety expectation not probed); stderr-vs-stdout channel separation (the log captures merged output).
- Missing information: echoed command lines; stream attribution of failure output.
- Recommended central-team follow-up: none required; optionally add an unreadable-file probe to a future regression pass.

#### Artifact Inventory
- TC-034 original evidence log — shareable filename `logs/original/TC-034.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 (internal markers 11:17:57–11:17:58 +03:00) — sensitive-content review: none found — included: yes
- Case definition (TC-034) — `manifests/cases_map.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:14 +0300 — sensitive-content review: none found — included: yes

### RUN-018 / TC-035

#### Workbook Record
- Recorded result: **Pass**
- Framework: Cross-stack
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)")
- Accelerator version: "monorepo @ 3a02537"
- Git commit: 3a02537
- Execution date: "2026-08-03" (historical snapshot); "2026-08-03 00:00:00" in the current workbook (mechanical coercion)
- Defect reference: none (the run's single retry was a harness-side probe correction, documented in Comments, not a product issue)
- Workbook evidence link: `Accelerator-TestEvidence/TC-035.log`
- Row presence: exists in BOTH the current workbook and the historical snapshot. Besides the datetime and int→float coercions common to all 49 surviving rows, RUN-018 is one of exactly two rows (with RUN-024) whose Actual Behaviour text was whitespace-collapsed in the current workbook: the historical snapshot preserves the gate's aligned output (`'ok    PHP Core.agents_md_bytes: ...'`, four spaces) while the current row reads `'ok PHP Core...'` (single space). Content is otherwise identical (whitespace-normalized comparison: equal).

#### Test Definition
- Definition status: **Authoritative — original source found.** TC-035 present in `manifests/cases_map.json`; the original Test Cases sheet agrees field-for-field (no mismatches).
- Title: "Startup context budget ceiling enforcement (context_budget.py --check)"
- Objective: prove the startup-context budget gate reports per-edition measurements, enforces per-category ceilings from token_budget.json, fails on regressions with actionable remediation text, and cannot silently skip an edition. Category: Testing; sheet Priority: Medium; Execution Type: Automated.
- Preconditions: "Monorepo root checkout, clean git state. scripts/token_budget.json present with per-edition ceilings for the four categories agents_md_bytes, frontmatter_bytes, description_bytes, skills (policy: observed value + ~5% headroom). Current measured values comfortably under ceilings (e.g. PHP Core.agents_md_bytes 12408 <= 12796)."
- Input (verbatim, 5 steps): `python3 scripts/context_budget.py`; `python3 scripts/context_budget.py --check`; `python3 -c "open('PHP Core/AGENTS.md','a').write('x'*2000)"`; `python3 scripts/context_budget.py --check`; `git checkout -- "PHP Core/AGENTS.md"`
- Mandatory assertions: report mode prints per-edition measurements, exit 0; --check prints one 'ok' line per category per edition (16 = 4x4) then 'All startup context budgets within ceilings.', exit 0; injected 2000-byte growth → 'FAIL  PHP Core.agents_md_bytes: <measured> > ceiling 12796' plus the remediation text, exit 1; an unknown edition in token_budget.json, or an edition with no ceilings, is itself a FAIL — the gate cannot silently skip an edition.
- Optional assertions: none beyond the above.
- Scriptable assertions: all (pure CLI case).
- Native-client assertions: none.
- Expected state changes: injection and token_budget.json substitutions confined to a temp copy; real repo untouched; copy restored clean after each mutation.
- Safety expectations: "The script is strictly read-only (it must never edit AGENTS.md, skills, or token_budget.json). Raising a ceiling is a human decision that must land in the same change as the growth it justifies — the tool only reports. No network, stdlib only."
- Requirement-source: "scripts/context_budget.py and scripts/token_budget.json (_comment policy field); docs/CI.md 'lint' section (budget step explanation)"

#### Historical Evidence
- Evidence status (main log): **Available — original** (`logs/original/TC-035.log`).
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-035.log` (shareable name: `logs/original/TC-035.log`). Internal markers: start/end both `2026-08-03T11:18:12+03:00`, with a corrected-probe appendix after the end marker.
- Commands: identified by step labels ("--- Step 1 (real repo): report mode", "--- Step 2 (real repo): --check gate", "--- Step 3 (TEMP COPY): inject 2000-byte growth into PHP Core/AGENTS.md", "--- Extra: token_budget.json with unknown edition (substituted json in copy)"); full command lines are not echoed and are supplied by the authoritative case definition.
- Exit codes recorded: report mode `exit=0`; clean --check `exit=0` with ok-line count `16`; post-injection --check `exit=1`; first (mis-targeted) unknown-edition probe `exit=0`; first missing-edition probe `exit=0`; corrected unknown-edition probe `exit=1`; corrected missing-edition probe `exit=1`.
- Assertions — Passed: 7/7 per workbook (report table, 16 ok lines + summary, FAIL line + remediation text verbatim, both token_budget.json integrity FAILs after probe correction, copy restored clean). Failed: none. Skipped: none.
- Key output excerpt (quoted exactly from TC-035.log):
  ```
  ok    PHP Core.agents_md_bytes: 12408 <= 12796
  All startup context budgets within ceilings.
  FAIL  PHP Core.agents_md_bytes: 14408 > ceiling 12796
  1 budget violation(s). Either trim the startup context
  back under the ceiling, or — for a justified permanent increase —
  raise the ceiling in scripts/token_budget.json in the same change.
  FAIL  token_budget.json lists unknown edition(s): ['NoSuchEdition']
  FAIL  Symfony: no ceilings in token_budget.json
  ```
- Contradictions with workbook: none between the log and the row content. Presentation-level alteration in the current workbook only: the whitespace collapse of the `'ok    '` alignment inside Actual Behaviour (see Workbook Record) — the historical snapshot matches the log's actual output spacing; the current row does not.

#### New Reproduction
- Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. (That capture includes a fresh budget gate run in `32-context-budget.log`.)
- Historical retry documented in the log itself: 1 — the log's appendix is explicit: "--- CORRECTED extra checks (ceilings live under top-level 'editions' key; previous attempt mutated wrong level)". The first unknown-edition/missing-edition probes mutated the wrong JSON level, so the gate correctly ignored them (exit 0); the corrected mutations produced both FAIL paths (exit 1). Recorded in the workbook as Retry Count 1 and explained in Comments as a test-harness mistake, not a product issue.

#### Native-Client Evidence
- Run executed in scripted mode (bash/python driven by a live Claude Code session, session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, zone exec:infra). The case's ai_tool is Cross-tool with Execution Type Automated — no native-client assertions are defined, so none are lost. Not Applicable.

#### Evidence Assessment
- Primary classification: **original-log-backed**.
- Confidence: **Medium** — original log at exact commit with all exit codes, the full 16-line ok table (printed three times: clean gate plus both first-pass probes; a fourth pass shows 15 ok + 1 FAIL after injection), the exact FAIL and remediation text, and an honestly documented probe correction; below High only because command lines are identified by label rather than echoed verbatim (supplied by the authoritative case definition).
- Evidence proves: report and gate modes behave as documented on the clean tree; a 2000-byte AGENTS.md regression is caught with the exact ceiling (12796) and remediation text; token_budget.json integrity failures (unknown edition, missing edition) each fail the gate; mutations stayed in the temp copy ("real repo status: (empty=clean)"; "copy clean final(2)").
- Evidence does not prove: strict read-only behavior of the script under adversarial conditions beyond the observed clean statuses; no-network/stdlib-only claims (not probed).
- Missing information: echoed command lines; exit-code capture for one intermediate pipe (log line "exit-of-pipe-head-shown-above").
- Recommended central-team follow-up: none for the product. For the workbook: restore the historical (whitespace-faithful) Actual Behaviour text for RUN-018, since the collapsed variant no longer matches the tool's actual aligned output.

#### Artifact Inventory
- TC-035 original evidence log — shareable filename `logs/original/TC-035.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 (internal marker 2026-08-03T11:18:12+03:00) — sensitive-content review: none found — included: yes
- Case definition (TC-035) — `manifests/cases_map.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:14 +0300 — sensitive-content review: none found — included: yes

### RUN-019 / TC-036

#### Workbook Record
- Recorded result: **Pass**
- Framework: Cross-stack
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)")
- Accelerator version: "monorepo @ 3a02537"
- Git commit: 3a02537
- Execution date: "2026-08-03" (historical snapshot); "2026-08-03 00:00:00" in the current workbook (mechanical coercion)
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-036.log`
- Row presence: exists in BOTH the current workbook and the historical snapshot; verbatim-identical apart from datetime coercion and int→float count coercion ("17"→"17.0").

#### Test Definition
- Definition status: **Authoritative — original source found.** TC-036 present in `manifests/cases_map.json`; the original Test Cases sheet agrees field-for-field (no mismatches).
- Title: "Full local CI reproduction per docs/CI.md (all six jobs)"
- Objective: run every command docs/CI.md lists locally and prove each exits 0 on a healthy checkout, matching the .github/workflows/ci.yml jobs (tests, parity, mirrors, lint, core-changelog, links) one-to-one. Category: Testing; sheet Priority: Critical; Execution Type: Automated.
- Preconditions: "Monorepo root checkout, clean git state, on a branch with a resolvable merge base against main. Requirements exactly as documented: Python 3, git, bash, and shellcheck (for one lint step ...). No sudo and no dependencies beyond the Python standard library are needed."
- Input (verbatim from the case, abbreviated list): the 7 unittest suite invocations (`(cd "Laravel/memory-bank/tests" && python3 -m unittest discover)` etc.), the three per-edition parity pairs, `python3 scripts/build_mirrors.py --check`, `git ls-files -z -- '*.sh' | xargs -0 -r -n1 bash -n`, `git ls-files -z -- '*.sh' | xargs -0 -r shellcheck -S error`, the tracked-JSON `python3 -m json.tool` loop, `python3 scripts/context_budget.py --check`, `bash scripts/check_core_changelog.sh`, `python3 scripts/check_links.py`
- Mandatory assertions: every command exits 0; all 7 suites pass; each edition prints both parity pass lines; mirrors line verbatim; lint silent (bash -n, shellcheck -S error), all tracked JSON parses, budget summary line; changelog pass or documented degraded skip; links pass line; CI workflow cross-check (same run: lines, job names).
- Optional assertions: OK-marker chaining per step ("Chain each command with '&& echo OK-<step>' and require every OK marker").
- Scriptable assertions: all (pure CLI case).
- Native-client assertions: none.
- Expected state changes: none — "No step may modify tracked files — git status --short must be identical before and after the full run."
- Safety expectations: "No step may use sudo, install packages, or reach the network ... shellcheck absence must fail only that one step, matching the documented requirement, not be silently skipped."
- Requirement-source: "docs/CI.md ('Running the checks locally') and .github/workflows/ci.yml"

#### Historical Evidence
- Evidence status (main log): **Available — original** (`logs/original/TC-036.log`).
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-036.log` (shareable name: `logs/original/TC-036.log`). Internal timestamps: start `2026-08-03T11:23:07+03:00`, end `2026-08-03T11:25:20+03:00` (~2m13s; digest duration 133 s agrees).
- Commands: not echoed in the log; the run is evidenced by per-job OK markers (the case's own prescribed verification style) — `OK-tests-laravel-mb` ... `OK-tests-ic`, `OK-parity-laravel/symfony/phpcore`, `OK-mirrors`, `OK-bash-n (64 files)`, `OK-shellcheck`, `OK-json (82 files)`, `OK-budget`, `OK-changelog`, `OK-links`. Commands themselves are supplied by the authoritative case definition (which quotes docs/CI.md).
- Exit codes: only one explicit capture in the log: `exit=0 (explicit unresolvable ref degrades to skip)` for the core-changelog degraded path. All other steps are evidenced by their OK markers (the marker prints only if the chained command succeeded, per the case's verification scheme); individual exit codes otherwise not recorded.
- Assertions — Passed: 17/17 per workbook (7 suites, 3 parity pairs, mirrors, 4 lint sub-steps, changelog incl. degraded skip, links, workflow cross-check, status-identical check). Failed: none. Skipped: none.
- Key output excerpt (quoted exactly from TC-036.log):
  ```
  === TC-036 start 2026-08-03T11:23:07+03:00 === (shellcheck 0.11.0 provided via scratchpad venv on PATH; not preinstalled on this machine)
  Mirror check passed for: Laravel, Symfony, PHP Core, Infrastructure-Creator
  OK-bash-n (64 files)
  version: 0.11.0
  OK-json (82 files)
  All startup context budgets within ceilings.
  core-changelog: cannot resolve a merge base with some-branch (missing remote or unfetched base); skipping.
  STATUS-IDENTICAL (clean before and after)
  ```
- Contradictions with workbook: none. The workbook Comments transparently disclose the environment deviation the log header also records: shellcheck 0.11.0 came from a scratchpad virtualenv put on PATH (not a system install — "no package installation, no sudo, no network"), which the case's own safety text tolerates only insofar as shellcheck absence "must fail only that one step"; the run chose to supply the binary rather than exercise the absence path.

#### New Reproduction
- Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. (That capture covers the same gate families: 7 test suites, parity x3 + cross, mirrors, links, budget — see `10-`..`32-*.log` and `99-summary.json`.)
- No retries documented in the log (workbook Retry Count 0, first attempt Yes).

#### Native-Client Evidence
- Run executed in scripted mode (bash/python driven by a live Claude Code session, session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, zone exec:infra). The case's ai_tool is Cross-tool with Execution Type Automated — no native-client assertions are defined, so none are lost. Not Applicable.

#### Evidence Assessment
- Primary classification: **original-log-backed**.
- Confidence: **Medium** — the original log at the exact commit shows every job's OK marker plus the key verbatim pass lines and the timed window; below High because the log records markers rather than echoed commands and per-step exit codes (both prescribed by, and traceable to, the authoritative case definition), and because one lint tool (shellcheck) came from a non-default source disclosed in the log header.
- Evidence proves: at 3a02537 the full docs/CI.md local battery passed end-to-end — 7 green suites, parity for all three editions, mirror check, bash -n over 64 tracked .sh files, shellcheck 0.11.0 -S error clean, 82 tracked JSON files valid, budget gate green, changelog gate pass plus documented degraded skip (exit 0), link check clean — and the working tree was byte-identical before/after ("status-before lines: 0", "STATUS-IDENTICAL").
- Evidence does not prove: per-step exit codes other than the one explicit capture; the "10" printed by the workflow cross-check (a bare count in the log whose exact grep expression is not echoed); behavior when shellcheck is genuinely absent (the documented single-step failure path was reasoned, not executed).
- Missing information: echoed command lines; per-step exit codes; the exact cross-check command behind the "10".
- Recommended central-team follow-up: if strict environment fidelity matters, re-run the lint job on a machine with distribution-packaged shellcheck, and optionally capture the shellcheck-absent failure mode once.

#### Artifact Inventory
- TC-036 original evidence log — shareable filename `logs/original/TC-036.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 (internal window 11:23:07–11:25:20 +03:00) — sensitive-content review: none found — included: yes
- Case definition (TC-036) — `manifests/cases_map.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:14 +0300 — sensitive-content review: none found — included: yes

### RUN-020 / TC-039

#### Workbook Record
- Recorded result: **Pass**
- Framework: PHP Core
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)")
- Accelerator version: 1.2.1
- Git commit: 3a02537
- Execution date: "2026-08-03" (historical snapshot); "2026-08-03 00:00:00" in the current workbook (mechanical coercion)
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-039.log`
- Row presence: exists in BOTH the current workbook and the historical snapshot; verbatim-identical apart from datetime coercion and int→float count coercion ("789"→"789.0").

#### Test Definition
- Definition status: **Authoritative — original source found.** TC-039 present in `manifests/cases_map.json`; the original Test Cases sheet agrees field-for-field (no mismatches).
- Title: "Edition Python infrastructure suites are fully green (132 memory-bank + 124 project-brain tests)"
- Objective: prove the stdlib-only Python infrastructure test suites run fully green with exact expected counts in PHP Core and identically in Laravel and Symfony (byte-identical core). Category: Testing; sheet Priority: Critical; Execution Type: Automated.
- Preconditions: "Fresh checkout of the monorepo; Python 3.x available; no virtualenv needed (stdlib unittest only). Working directory is the edition root — note the space in \"PHP Core\" must be quoted."
- Input (verbatim): `cd "PHP Core" && python3 -m unittest discover -s memory-bank/tests && python3 -m unittest discover -s project-brain/tests`
- Mandatory assertions: memory-bank suite discovers and runs exactly 132 tests ending OK; project-brain suite runs exactly 124 tests ending OK; same counts repeat in Laravel/ and Symfony/; matches the CI matrix in .github/workflows/ci.yml.
- Optional assertions: parity cross-check (`context.py parity` + `--cross-edition` exit 0).
- Scriptable assertions: all (pure CLI case).
- Native-client assertions: none.
- Expected state changes: none — "Test runs must not modify the repository working tree (git status --porcelain unchanged afterwards) ... must not leave stray files outside temp dirs (only __pycache__ may appear)."
- Safety expectations: as quoted above, plus no real-git-config modification and no network access.
- Requirement-source: ".github/workflows/ci.yml (tests and parity job matrices)"

#### Historical Evidence
- Evidence status (main log): **Available — original** (`logs/original/TC-039.log`).
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-039.log` (shareable name: `logs/original/TC-039.log`). Internal timestamps: start `2026-08-03T11:20:20+03:00`, end `2026-08-03T11:22:25+03:00` (~2m05s; digest duration 125 s agrees).
- Commands: not echoed in the log; sections are labeled ("--- Primary (PHP Core, per input_prompt):", "--- Repeat in Laravel:", "--- Repeat in Symfony:", "--- 7th suite (Infrastructure-Creator):", "--- parity cross-check (PHP Core):"). The commands are supplied by the authoritative case definition. Note the log's primary section explicitly cites "per input_prompt".
- Exit codes recorded: one explicit capture, `exit=0`, for the parity cross-check. Suite results are evidenced by unittest's own terminal output ("Ran N tests in S" + "OK") rather than captured exit codes.
- Assertions — Passed: 789/789 tests total; per workbook 7 suites — PHP Core 132 OK + 124 OK, Laravel 132 OK + 124 OK, Symfony 132 OK + 124 OK, Infrastructure-Creator 21 OK — plus parity cross-check and both cleanliness checks. Failed: none. Skipped: none.
- Key output excerpt (quoted exactly from TC-039.log):
  ```
  --- Primary (PHP Core, per input_prompt):
  Ran 132 tests in 19.444s
  OK
  Ran 124 tests in 22.560s
  OK
  --- parity cross-check (PHP Core):
  Mirror parity passed (.agents canonical).
  Cross-edition core parity passed.
  --- new __pycache__ dirs vs before:
  no new pycache dirs (list identical)
  ```
- Contradictions with workbook: none. Counts, timings, parity result, clean git status ("(empty=clean)"), and the no-new-__pycache__ observation in the row all match the log exactly. The 7th suite (Infrastructure-Creator, 21 tests) is an executor addition beyond the case's three-edition scope, transparently labeled in both log and row; total 789 = 3x132 + 3x124 + 21.

#### New Reproduction
- Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. (That capture re-runs all 7 suites — `10-`..`16-*.log` — against the post-remediation code, whose test files were among the 57 modified files; its counts therefore need not equal 132/124.)
- No retries documented in the log (workbook Retry Count 0, first attempt Yes).

#### Native-Client Evidence
- Run executed in scripted mode (bash/python driven by a live Claude Code session, session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, zone exec:infra). The case's ai_tool is Cross-tool with Execution Type Automated — no native-client assertions are defined, so none are lost. Not Applicable.

#### Evidence Assessment
- Primary classification: **original-log-backed**.
- Confidence: **Medium** — original log at the exact commit with unittest's own count/OK lines for all seven suites and a bounded time window; below High because command invocations are referenced by label ("per input_prompt") rather than echoed, and per-suite exit codes are not captured (the "OK" terminal line is unittest's success signal).
- Evidence proves: at 3a02537 the memory-bank suite ran exactly 132 tests and the project-brain suite exactly 124 tests, all OK, identically in PHP Core, Laravel, and Symfony (consistent with the byte-identical core, independently confirmed by the parity cross-check exit 0); the Infrastructure-Creator suite (21 tests) was also green; the real repo tree stayed clean with no new __pycache__ directories.
- Evidence does not prove: absence of network access during the suites (not probed); per-suite process exit codes.
- Missing information: echoed command lines; per-suite exit codes.
- Recommended central-team follow-up: none — this run is also cross-corroborated by RUN-019/TC-036 (same suites green within its CI battery, logged 11:23-11:25 the same morning).

#### Artifact Inventory
- TC-039 original evidence log — shareable filename `logs/original/TC-039.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 (internal window 11:20:20–11:22:25 +03:00) — sensitive-content review: none found — included: yes
- Case definition (TC-039) — `manifests/cases_map.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:14 +0300 — sensitive-content review: none found — included: yes

### RUN-021 / TC-041

#### Workbook Record
- Recorded result: Pass
- Framework: Infrastructure-Creator (Framework Version: "n/a (edition workspace)")
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Not Applicable)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.0
- Git commit: 3a02537 (= current HEAD; Starting Repository State: "Clean checkout at 3a02537; mutating steps ran in a temp copy")
- Execution date: 2026-08-03 (historical snapshot); current workbook shows "2026-08-03 00:00:00" (mechanical datetime coercion applied to all 49 surviving rows)
- Defect reference: None (workbook Defect ID empty; no TC-041 entry in defects.json)
- Workbook evidence link: Accelerator-TestEvidence/TC-041.log
- Row presence: exists in BOTH the current workbook ("Test Runs") and the historical snapshot. Content is verbatim-identical apart from the mechanical rewrite (date-to-datetime, count columns int-to-float: 5/5/0 became 5.0/5.0/0.0).
- Counts recorded: Tests Executed 5 / Passed 5 / Failed 0; Human Intervention 0; Retry 0; Safety Violations 0.

#### Test Definition
- Definition status: Authoritative — original source found (cases_map.json TC-041; the workbook's original 74-row Test Cases sheet agrees field-by-field, whitespace-normalized).
- Title: Manifest integrity: sha256 map, version source, runtime-state exclusion, AGENTS.md stamp (Category: Testing; Priority: Critical; Owner: QA).
- Objective: independently verify the `.infra-manifest.json` contract of a full-mode Infrastructure-Creator generation, without relying on validate_generated.py.
- Preconditions (quoted): "Case 3 completed: /tmp/fixtures/fake-php-app carries a full-mode generation with .infra-manifest.json and a stamped AGENTS.md. Infrastructure-Creator/VERSION contains 1.4.0."
- Input: the python3 manifest verification one-liner from the case definition (recipe from infra-generate SKILL.md), which recomputes sha256 for every tracked file and scans for runtime-state entries; full text in cases_map.json `input_prompt`.
- Mandatory assertions: manifest parses; manifest_version 1; generator "Infrastructure-Creator"; generator_version equals the VERSION file (1.4.0); task references the run's TASK-{N}; profile points at tasks/TASK-{N}/infra-scan-project-profile.md; mode "full"; `mismatches: []`; `runtime-state leaks: []`; AGENTS.md first line is the stamp matching version and task.
- Optional assertions: none stated in the definition. (Sheet Notes: "The same contract is enforced by validate_generated.py's manifest checks; this case double-checks it independently of the validator implementation.")
- Scriptable assertions: all of the above — expected_verification is a pure shell/python check (script prints "mismatches: []", "runtime-state leaks: []", "version: 1.4.0 task: TASK-1 mode: full"; `head -1` stamp check; VERSION-equality test returning true).
- Native-client assertions: none in the verification itself; only the precondition ("Case 3 completed", i.e. an agent-driven infra-generate run) presumes an AI client.
- Expected state changes (quoted): "None (verification only - no file may change during this check)."
- Safety expectations (quoted): "No version hardcoded or recalled from CHANGELOG.md - VERSION is the single source of truth. A fresh infra-generate manifest carries no 'decisions' map (nothing has been decided yet). Listing runtime state would let infra-update overwrite the team's live memory - it must be absent."
- Requirement-source: Infrastructure-Creator/.agents/skills/infra-generate/SKILL.md.

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: /home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-041.log (shareable name: logs/original/TC-041.log; 14 lines).
- Commands: the log records labeled step markers, not full command text: "$ manifest verification one-liner from the case definition", "$ head -1 AGENTS.md", "$ VERSION == manifest generator_version", "$ file count in manifest". The complete one-liner appears only in the case definition (cases_map.json `input_prompt`).
- Exit codes: not recorded in the log — Unknown.
- Assertions: 5 executed / 5 passed / 0 failed (workbook counts; the log shows every expected value matched).
- Key output excerpt (log lines 3-8, 10, 12, 14, quoted exactly):
  ```
  mismatches: []
  runtime-state leaks: []
  version: 1.4.0 task: TASK-1 mode: full
  manifest_version: 1 generator: Infrastructure-Creator profile: tasks/TASK-1/infra-scan-project-profile.md
  decisions map present on fresh generation: False
  tracks itself: False
  <!-- Generated by Infrastructure-Creator v1.4.0 | TASK-1 | 2026-08-03 -->
  true
  49
  ```
- Contradictions with workbook: none. The workbook itself discloses the one precondition deviation (Comments: "Target was hand-assembled (Case-3 agent run not available), but the manifest under test was produced by the exact shipped recipe"), and the log header confirms it: "full-mode manifest written via the exact recipe from infra-generate SKILL.md, TASK-1, editions claude".

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. The log documents no retries (workbook Retry Count 0).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); the case's ai_tool is Cross-tool and its verification is tool-agnostic, so no native-client assertion exists in the verification itself. The precondition-side aspect that presumes an AI client (a genuine agent-driven infra-generate "Case 3" generation) was substituted by hand-assembly per the shipped SKILL.md recipe — that agent-pipeline aspect: Native assertions unavailable.

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium — the original log and the exact commit (3a02537, clean tree per TC-001.log zone evidence) are available, but the log carries labeled step markers rather than complete verbatim commands, and no per-step exit codes; High is therefore not reachable.
- Evidence proves: the manifest on the verified target satisfied the full contract — 49 files tracked with all sha256 recomputed clean, no runtime-state leaks, no self-tracking, no `decisions` map, version/task/mode 1.4.0/TASK-1/full, matching AGENTS.md stamp, VERSION-file equality (log prints `true`).
- Evidence does not prove: that the shipped infra-generate LLM pipeline itself produces such a manifest end-to-end (the target was hand-assembled per the recipe, disclosed in the workbook and the log header).
- Missing information: verbatim command transcript; per-command exit codes; in-log confirmation of the fixture path (the case says /tmp/fixtures/fake-php-app; the workbook Files Created cell — "None (ran against the existing exec-ic/fake-php-app temp copy)" — and sibling log TC-042 place the target at the session scratchpad exec-ic/fake-php-app, but the TC-041 log itself names no absolute path).
- Recommended central-team follow-up: re-run TC-041 immediately after a genuine agent-driven Case-3 generation (covered by TC-007/TC-058 scope) to close the pipeline-provenance gap; capture command lines and exit codes in the log.

#### Artifact Inventory
- TC-041.log — shareable filename logs/original/TC-041.log — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 11:31:41 +0300 (file mtime) — sensitive-content review: none found — included: yes
- Case definition (TC-041 entry) — shareable filename manifests/cases_map.json — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 11:14:56 +0300 (file mtime) — sensitive-content review: none found — included: yes
- Workbook row (historical snapshot) — shareable filename manifests/workbook-snapshot-20260803-2002.xlsx — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 20:02:35 +0300 (file mtime) — sensitive-content review: none found — included: yes
- Fixture target (exec-ic/fake-php-app with .infra-manifest.json, session scratchpad of c9fab6de) — shareable filename: none assigned — Available — original (still present under /tmp/claude-1000/.../c9fab6de-739d-497c-ac0d-61bda631def9/scratchpad/exec-ic/ as of 2026-08-04, but session-temp and at risk of loss on reboot) — SHA-256: not recorded — capture timestamp Unknown — sensitive-content review: none found — included: no

### RUN-022 / TC-042

#### Workbook Record
- Recorded result: Pass
- Framework: Infrastructure-Creator (Framework Version: "n/a (edition workspace)")
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Not Applicable)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.0
- Git commit: 3a02537 (Starting Repository State: "Clean checkout at 3a02537; mutating steps ran in a temp copy")
- Execution date: 2026-08-03 (historical snapshot); current workbook "2026-08-03 00:00:00" (mechanical coercion)
- Defect reference: None (workbook Defect ID empty; no TC-042 entry in defects.json)
- Workbook evidence link: Accelerator-TestEvidence/TC-042.log
- Row presence: exists in BOTH the current workbook and the historical snapshot; content identical apart from the mechanical rewrite (datetime, counts 6/6/0 to 6.0/6.0/0.0).
- Counts recorded: Tests Executed 6 / Passed 6 / Failed 0; Human Intervention 0; Retry 0; Safety Violations 0.

#### Test Definition
- Definition status: Authoritative — original source found (cases_map.json TC-042; the workbook's original 74-row Test Cases sheet agrees field-by-field, whitespace-normalized).
- Title: validate_generated.py catches broken frontmatter, dangling references, and dead hooks (Category: Testing; Priority: High; Owner: QA).
- Objective: prove the bootstrap-verifier validator detects three injected defect classes (broken frontmatter, dangling skill reference, dead/non-executable hook wiring) and returns to green after restore.
- Preconditions (quoted): "A green full-mode generation exists (Case 3, validator exit 0). Snapshot the target, then inject three defects: (1) delete the closing '---' of the frontmatter in /tmp/fixtures/fake-php-app/.claude/skills/coding/SKILL.md; (2) add 'ghost-skill' to the related: list in .claude/skills/testing/SKILL.md; (3) edit .claude/settings.json to wire .claude/hooks/ghost-hook.sh (nonexistent) and chmod -x .claude/hooks/bash-validator.sh."
- Input (quoted): `python3 Infrastructure-Creator/.agents/skills/bootstrap-verifier/scripts/validate_generated.py --target /tmp/fixtures/fake-php-app --editions claude; echo exit=$?`
- Mandatory assertions: validator exits non-zero and prints to stderr: an invalid-frontmatter finding for coding/SKILL.md; an unresolved cross-reference finding for 'ghost-skill'; a dead-hook wiring finding for ghost-hook.sh; a missing-executable-bit finding for bash-validator.sh. Grep thresholds: 'ghost' >=2, 'frontmatter' >=1 (case-insensitive), 'bash-validator.sh' >=1. After restoring the sabotages: exit 0.
- Optional assertions (from sheet Notes): "include that variant as an optional fourth sabotage" — an interpreter-prefixed wired command ('bash .claude/hooks/x.sh') hiding a missing script; not exercised in this run.
- Scriptable assertions: all of the mandatory set (raw-script invocation, grep counts, restore-and-rerun).
- Native-client assertions: the skill-driven classification flow — exec-bit failure classified auto-fixable (chmod +x, re-run, no manifest hash refresh for permissions-only fixes) while the dangling cross-reference is escalated to the user — is LLM-driven (bootstrap-verifier skill) and needs a native client.
- Expected state changes (quoted): "None from the validator itself (read-only script). Skill-driven run additionally writes tasks/TASK-{N}/bootstrap-verifier-report.md with Result: FAIL and the per-check table."
- Safety expectations (quoted): "The validator itself modifies nothing. The verifier skill only auto-fixes safe items (executable bit), refreshes manifest hashes after content-changing auto-fixes only, and reports FAIL rather than papering over escalation-class findings. infra-generate must not report success while any failure is unresolved."
- Requirement-source: Infrastructure-Creator/.agents/skills/bootstrap-verifier/SKILL.md.

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: /home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-042.log (shareable name: logs/original/TC-042.log; 28 lines).
- Commands: the log records section markers ("-- baseline --", "-- sabotage 1: remove closing frontmatter delimiter in .claude/skills/coding/SKILL.md --", "-- sabotage 2: add ghost-skill to related: in .claude/skills/testing/SKILL.md --", "-- sabotage 3: wire nonexistent ghost-hook.sh in settings.json and chmod -x bash-validator.sh --", "-- run validator on sabotaged target --", "-- grep assertions --", "-- restore snapshot, chmod +x, re-run --"), not literal command lines; the invocation form is specified in the case definition `input_prompt`.
- Exit codes (quoted from log): "baseline exit=0"; sabotaged run "exit=1"; "restored exit=0".
- Assertions: 6 executed / 6 passed / 0 failed (workbook counts). The sabotaged run produced 9 validator findings covering all injected defects.
- Key output excerpt (log lines 10, 12-15, 20-21, quoted exactly; long scratchpad prefixes elided with [...]):
  ```
  ERROR: [claude] [...]/exec-ic/fake-php-app/.claude/skills/coding/SKILL.md: missing 'name' in frontmatter
  ERROR: [claude] [...]/exec-ic/fake-php-app/.claude/skills/testing/SKILL.md: related -> 'ghost-skill' does not resolve to a generated skill
  ERROR: [...]/exec-ic/fake-php-app/.claude/hooks/bash-validator.sh: not executable (chmod +x needed)
  ERROR: [...]/exec-ic/fake-php-app/.claude/settings.json: wired hook does not exist: .claude/hooks/ghost-hook.sh
  ERROR: [...]/exec-ic/fake-php-app/.claude/settings.json: wired hook not executable: .claude/hooks/bash-validator.sh
  9 problem(s) found.
  exit=1
  ```
  Grep assertion results (log lines 23-25): "ghost mentions (expect >=2): 2", "frontmatter mentions (expect >=1): 2", "bash-validator.sh mentions (expect >=1): 2".
- Contradictions with workbook: none. The log fully substantiates the workbook's Actual Behaviour, including the 9-finding total and the restore to exit 0. The fixture path in the log is the session scratchpad exec-ic/fake-php-app rather than the case's /tmp/fixtures/fake-php-app — a disclosed harness relocation, consistent across the exec:ic zone.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. Note: the remediation workflow fix-qa-findings (2026-08-03 15:31-16:28) later modified validate_generated.py for the TC-008/TC-014 defects, so the current working-tree validator is no longer byte-identical to the one tested here. The log documents no retries (workbook Retry Count 0).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); the case's ai_tool is Cross-tool and the raw-script path is tool-agnostic. The bootstrap-verifier skill-driven escalation/auto-fix classification flow demands a native AI client and was NOT exercised (workbook Comments confirm: "The bootstrap-verifier skill-layer escalation flow is LLM-driven and was not exercised") — those assertions: Native assertions unavailable.

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium — original log, exact commit, and explicit exit codes for all three phases are present, but the log does not contain the literal command lines (they come from the case definition), and the optional fourth sabotage plus the skill-layer flow were not exercised; High is therefore not reachable.
- Evidence proves: the validator detected every injected defect class (frontmatter, dangling reference, dead hook, exec bit) with 9 findings and exit 1, and returned to exit 0 after restore; the sabotage-injection markers match the case's three prescribed defects.
- Evidence does not prove: the skill-driven classification (auto-fixable vs escalation) or the bootstrap-verifier-report.md writing; the interpreter-prefixed dead-hook variant.
- Missing information: literal command transcript; stderr/stdout stream attribution (the case expects findings on stderr; the log merges streams).
- Recommended central-team follow-up: exercise the bootstrap-verifier skill flow in a native Claude Code session (classification, report file, manifest-refresh policy) and add the optional interpreter-prefixed sabotage; note the validator has since been modified by remediation, so a fresh baseline is needed.

#### Artifact Inventory
- TC-042.log — shareable filename logs/original/TC-042.log — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 11:31:41 +0300 (file mtime) — sensitive-content review: none found — included: yes
- Case definition (TC-042 entry) — shareable filename manifests/cases_map.json — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 11:14:56 +0300 (file mtime) — sensitive-content review: none found — included: yes
- Workbook row (historical snapshot) — shareable filename manifests/workbook-snapshot-20260803-2002.xlsx — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 20:02:35 +0300 (file mtime) — sensitive-content review: none found — included: yes
- Fixture target and snapshot (exec-ic/fake-php-app, exec-ic/fake-php-app.snapshot, session scratchpad of c9fab6de) — shareable filename: none assigned — Available — original (still present under /tmp as of 2026-08-04; session-temp, at risk of loss on reboot) — SHA-256: not recorded — capture timestamp Unknown — sensitive-content review: none found — included: no

### RUN-023 / TC-043

#### Workbook Record
- Recorded result: Pass
- Framework: Cross-stack (Framework Version: "n/a (edition workspace)")
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Not Applicable)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: "monorepo @ 3a02537" (repository-level toolchain, not a semver edition)
- Git commit: 3a02537 (Starting Repository State: "Clean checkout at 3a02537; mutating steps ran in a temp copy")
- Execution date: 2026-08-03 (historical snapshot); current workbook "2026-08-03 00:00:00" (mechanical coercion)
- Defect reference: None (workbook Defect ID empty; no TC-043 entry in defects.json)
- Workbook evidence link: Accelerator-TestEvidence/TC-043.log
- Row presence: exists in BOTH the current workbook and the historical snapshot; content identical apart from the mechanical rewrite (datetime, counts 6/6/0 to 6.0/6.0/0.0).
- Counts recorded: Tests Executed 6 / Passed 6 / Failed 0; Human Intervention 0; Retry 0; Safety Violations 0.

#### Test Definition
- Definition status: Authoritative — original source found (cases_map.json TC-043; the workbook's original 74-row Test Cases sheet agrees field-by-field, whitespace-normalized).
- Title: Relative markdown link integrity across the monorepo (check_links.py) (Category: Documentation; Priority: Medium; Owner: QA).
- Objective: verify scripts/check_links.py passes clean on the real tree, catches an injected broken relative link, skips links inside fenced code blocks, and honors the fnmatch allowlist.
- Preconditions (quoted): "Monorepo root checkout, clean git state. git available (the checker walks .md files tracked by git via git ls-files, so untracked files are invisible until added to the index). Optional allowlist scripts/check_links_ignore.txt absent while empty (current state: file exists as fnmatch allowlist support)."
- Input (quoted, 5 steps): `python3 scripts/check_links.py`; inject `printf '\n[missing](does-not-exist.md)\n' >> docs/README.md`; re-run; negative control `printf '\n\`\`\`\n[fenced](also-missing.md)\n\`\`\`\n' >> docs/README.md && python3 scripts/check_links.py`; restore `git checkout -- docs/README.md`.
- Mandatory assertions: clean run prints "All relative markdown links resolve. No broken links found." and exits 0; injected link reported exactly once, exit 1; fenced-block link NOT reported; restore returns exit 0.
- Optional assertions: allowlist suppression (one fnmatch pattern per line, '#' comments) suppresses a matching finding; URL-decoding of %20 paths (load-bearing for 'PHP Core' links per sheet Notes).
- Scriptable assertions: all assertions are scriptable (pure python/git toolchain).
- Native-client assertions: none — the case has no AI-client component.
- Expected state changes (quoted): "None (docs/README.md modification is test scaffolding, restored)."
- Safety expectations (quoted): "Read-only: the checker must never rewrite markdown files. It must not fetch external URLs (http(s) links are skipped, no network I/O). A brand-new markdown file with broken links must be caught once staged — QA should verify the git-tracked-only scope is understood, not treated as a bug."
- Requirement-source: "scripts/check_links.py (module docstring); docs/CI.md 'links'; CI job 'links' in .github/workflows/ci.yml".

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: /home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-043.log (shareable name: logs/original/TC-043.log; 39 lines; internal timestamps "=== TC-043 start 2026-08-03T11:18:49+03:00 ===" / "=== end 2026-08-03T11:18:50+03:00 ===").
- Commands: the log records step markers ("--- Step 1 (real repo): clean run", "--- Step 2 (TEMP COPY): inject broken link into docs/README.md", "--- Step 4 (copy): broken link inside fenced code block must NOT be reported", "--- allowlist suppression check (copy): add pattern to check_links_ignore.txt", "--- Step 5: restore copy"), not literal command lines; the commands are specified in the case definition `input_prompt`.
- Exit codes (quoted from log): Step 1 "exit=0"; Step 3 "exit=1"; Step 4 "exit=1" (still only the Step-2 finding); allowlist check "exit=0"; restore "exit=0"; fenced-link grep "(grep exit=1 ; 0 matches expected)".
- Assertions: 6 executed / 6 passed / 0 failed (workbook counts).
- Key output excerpt (log lines 3, 20-22, 29-30, 33, quoted exactly):
  ```
  All relative markdown links resolve. No broken links found.
  Found 1 broken relative link(s):

  docs/README.md:78: broken link (does-not-exist.md) -> docs/does-not-exist.md
  --- occurrences of also-missing in output above should be zero; grep check:
  0
  All relative markdown links resolve. No broken links found.
  ```
- Contradictions with workbook: none between log and workbook. Two disclosed deviations from the case text, both recorded in the workbook Comments and visible in the log: (1) finding format is "<file>:<line>: broken link (<target>) -> <resolved path>" rather than the case's "<file>:<line>: [missing](target)" — cosmetic, the case's own grep-based verification still passes; (2) the allowlist was not empty as the precondition suggested — the log shows the allowlist file content with the single documented pattern `*/Task/*` and its owner-decision comment block. Additionally the mutating steps ran in a temp copy rather than directly on the repo (safety-conservative deviation; the log line "--- real repo status:" followed by "(empty=clean)" documents the untouched real tree).

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. The log documents no retries (workbook Retry Count 0).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); the case's ai_tool is Cross-tool and defines no native-client assertions, so nothing is lost to scripted execution here.

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium — original log with internal timestamps, exact commit, and explicit exit codes for every step are present, but the literal command lines are not in the log (they come from the case definition); High is therefore not reachable.
- Evidence proves: clean-tree pass, single-finding detection of the injected broken link with exit 1, fenced-code-block suppression (0 occurrences of the fenced target), fnmatch allowlist suppression, clean restore, and that the real repository tree stayed clean.
- Evidence does not prove: the URL-decoding (%20 / 'PHP Core') branch in isolation (only implicitly via the clean pass over the tree that contains such links); the git-tracked-only scope behavior for untracked files.
- Missing information: literal command transcript; the temp-copy path used.
- Recommended central-team follow-up: none critical; optionally align the case's expected finding-format string with the implemented output, and add an explicit untracked-file scope probe.

#### Artifact Inventory
- TC-043.log — shareable filename logs/original/TC-043.log — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 11:31:41 +0300 (file mtime; internal run timestamps 11:18:49-11:18:50 +0300) — sensitive-content review: none found — included: yes
- Case definition (TC-043 entry) — shareable filename manifests/cases_map.json — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 11:14:56 +0300 (file mtime) — sensitive-content review: none found — included: yes
- Workbook row (historical snapshot) — shareable filename manifests/workbook-snapshot-20260803-2002.xlsx — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 20:02:35 +0300 (file mtime) — sensitive-content review: none found — included: yes
- Temp mutation copy — shareable filename: none assigned — Unavailable — location unknown (the log does not name the copy path; session scratchpad exec-crossstack/ directories still exist but the specific copy used was not identified) — SHA-256: not recorded — capture timestamp Unknown — sensitive-content review: none found — included: no

### RUN-024 / TC-044

#### Workbook Record
- Recorded result: Pass
- Framework: Cross-stack (Framework Version: "n/a (edition workspace)")
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Not Applicable)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: "monorepo @ 3a02537" (repository-level toolchain, not a semver edition)
- Git commit: 3a02537 (Starting Repository State: "Clean checkout at 3a02537; mutating steps ran in a temp copy")
- Execution date: 2026-08-03 (historical snapshot); current workbook "2026-08-03 00:00:00" (mechanical coercion)
- Defect reference: None (workbook Defect ID empty; no TC-044 entry in defects.json)
- Workbook evidence link: Accelerator-TestEvidence/TC-044.log
- Row presence: exists in BOTH the current workbook and the historical snapshot. Beyond the mechanical rewrite (datetime, counts 9/9/0 to 9.0/9.0/0.0, Retry 1 to 1.0), RUN-024 is one of only two rows (with RUN-018) whose Actual Behaviour text had whitespace collapsed in the current workbook: the historical cell's two-space indent in "'  - Laravel/.claude/hooks/bash-validator.sh'" (which matches the log verbatim) became a single space. Content is otherwise identical.
- Counts recorded: Tests Executed 9 / Passed 9 / Failed 0; Human Intervention 0; Retry 1; Safety Violations 0. Linter Result: "bash -n clean (covered by TC-036 over all tracked .sh)".

#### Test Definition
- Definition status: Authoritative — original source found (cases_map.json TC-044; the workbook's original 74-row Test Cases sheet agrees field-by-field, whitespace-normalized).
- Title: Core-changelog gate: shared-core PR without a root CHANGELOG.md entry fails (Category: Documentation; Priority: High; Owner: QA).
- Objective: verify scripts/check_core_changelog.sh fails a shared-core diff lacking a root CHANGELOG.md entry, passes once the entry exists, ignores non-core diffs, and degrades (exit 0 with explanation) when no merge base is resolvable.
- Preconditions (quoted): "Monorepo checkout with a resolvable merge base: either an origin remote with origin/main fetched, or a local main branch (the script falls back to local main when origin/main is absent). Create a scratch feature branch off main with a clean tree. bash and git available."
- Input (quoted, condensed): Step 1 `echo '# gate test' >> Laravel/.claude/hooks/bash-validator.sh`; Step 2 `bash scripts/check_core_changelog.sh`; Step 3 add a root CHANGELOG.md entry; Step 4 re-run; Step 5 `git checkout -- Laravel/.claude/hooks/bash-validator.sh CHANGELOG.md`; also the explicit form `bash scripts/check_core_changelog.sh some-branch`.
- Mandatory assertions: core-touch-without-changelog fails with the exact stderr message listing the file and remediation text, exit 1; with a CHANGELOG entry passes ("shared-core files changed and the root CHANGELOG.md was updated."), exit 0; non-core diff passes ("no shared-core files touched; no root CHANGELOG.md entry required."), exit 0; unresolvable merge base degrades with "cannot resolve a merge base" on stderr, exit 0.
- Optional assertions: committed-diff variants behave the same as working-tree variants; CI wiring (pull_request-only job, fetch-depth 0, base ref origin/${{ github.base_ref }}).
- Scriptable assertions: all listed assertions are scriptable.
- Native-client assertions: none — the case has no AI-client component.
- Expected state changes (quoted): "None permanently (test edits to Laravel/.claude/hooks/bash-validator.sh and CHANGELOG.md are restored in Step 5)."
- Safety expectations (quoted): "The script is read-only over git metadata (git diff --name-only); it must never modify the CHANGELOG or the diffed files. It must never hard-fail on an unanswerable question (degraded skip is mandatory, exit 0 with an explanation). Editing a generated mirror hook to dodge the pattern is out of scope — mirrors are covered by the same .(claude|cursor|codex)/hooks/ pattern."
- Requirement-source: "scripts/check_core_changelog.sh (header comment + CORE_PATTERN); docs/CI.md 'changelog'; CI job 'core-changelog' in .github/workflows/ci.yml".

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: /home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-044.log (shareable name: logs/original/TC-044.log; 40 lines; header "=== TC-044 RETRY 2026-08-03T11:19:59+03:00 === (origin/main predates the script; using explicit BASE_REF form with a fictitious base branch qa-base at HEAD)"; footer "=== end 2026-08-03T11:19:59+03:00 ===").
- Commands: the log records scenario markers ("--- baseline: no changes vs qa-base", "--- Step 1: touch shared core only (uncommitted)", "--- Step 2: gate must FAIL", "--- fictitious COMMIT variant on scratch branch (clean tree, committed diff vs qa-base)", "--- negative scope: only Infrastructure-Creator/README.md (uncommitted)", "--- unresolvable explicit base ref: some-branch", "--- default-run on copy merge branch vs origin/main (real committed diff):"), not literal command lines; the commands are specified in the case definition `input_prompt`.
- Exit codes (quoted from log, in order): baseline "exit=0"; Step 2 "exit=1"; Step 4 "exit=0"; committed variant "exit=0"; committed core without changelog "exit=1"; negative scope "exit=0"; unresolvable ref "exit=0"; default-run "exit=0". (8 exits recorded — see Contradictions.)
- Assertions: 9 executed / 9 passed / 0 failed (workbook counts).
- Key output excerpt (log lines 8-12, 16, 30, 33, quoted exactly):
  ```
  core-changelog: this diff touches shared-core files but not the root CHANGELOG.md:
    - Laravel/.claude/hooks/bash-validator.sh

  Add an entry to the root CHANGELOG.md (its header states the scope),
  or move the change out of the shared core.
  core-changelog: shared-core files changed and the root CHANGELOG.md was updated.
  core-changelog: no shared-core files touched; no root CHANGELOG.md entry required.
  core-changelog: cannot resolve a merge base with some-branch (missing remote or unfetched base); skipping.
  ```
- Contradictions with workbook: one count-level gap. The workbook's Actual Behaviour describes a dedicated "Degradation clone (origin removed, no local main) -> same degraded skip, exit 0", but the log contains no separate degradation-clone section — only the unresolvable-explicit-ref degraded skip (log line 33). Correspondingly the log records 8 explicit exit codes while the workbook counts 9 tests executed; the ninth (degradation-clone) scenario is asserted by the workbook but not evidenced in the log. Also, all scenarios ran against a fictitious base branch "qa-base" at HEAD via the documented explicit-BASE_REF form (disclosed in the log header and workbook), not the case's literal scratch-branch-off-main procedure — an environmental adaptation, since origin/main predates the script under test.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run.
Historical retry (documented in the log itself): the surviving log is explicitly a RETRY — header quoted above; per the execute-test-cases digest, "TC-044: check_core_changelog all scenarios via explicit BASE_REF in a temp clone (retry environmental: script absent on older origin/main)". The workbook Comments state the first attempt failed because "checking out a scratch branch off origin/main removed the script itself (it does not exist on that older main), so the first attempt hit 'No such file or directory'". No log of the first attempt exists (Unavailable — deleted or never captured; only the retry log was kept). Workbook Retry Count: 1, First attempt: No (digest).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); the case's ai_tool is Cross-tool and defines no native-client assertions, so nothing is lost to scripted execution here.

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium — original retry log with internal timestamps, exact commit, and explicit exit codes are present, but literal command lines are absent from the log, the first-attempt log was not kept, and one of the nine counted scenarios (degradation clone) is not evidenced in the log.
- Evidence proves: the gate's fail/pass/negative-scope/degraded-skip behaviors with verbatim expected messages, for both uncommitted and committed diffs, against an explicit base ref; cleanup ("copy restored clean", "cleanup scratch branches").
- Evidence does not prove: the ninth scenario (origin-removed degradation clone); default-base-ref behavior on a true origin/main containing the script (structurally impossible at this commit — origin/main predated the script); the CI job wiring.
- Missing information: first-attempt log; literal command transcript; temp-clone path.
- Recommended central-team follow-up: after the script lands on origin/main, re-run TC-044 in default-BASE_REF form; either re-evidence or decrement the ninth (degradation-clone) scenario; keep first-attempt logs when a retry occurs.

#### Artifact Inventory
- TC-044.log — shareable filename logs/original/TC-044.log — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 11:31:41 +0300 (file mtime; internal run timestamp 11:19:59 +0300) — sensitive-content review: none found — included: yes
- First-attempt log — shareable filename: none — Unavailable — deleted (never captured into the evidence set; only the RETRY log survives) — SHA-256: n/a — capture timestamp Unknown — sensitive-content review: Not Applicable — included: no
- Case definition (TC-044 entry) — shareable filename manifests/cases_map.json — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 11:14:56 +0300 (file mtime) — sensitive-content review: none found — included: yes
- Workbook row (historical snapshot) — shareable filename manifests/workbook-snapshot-20260803-2002.xlsx — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 20:02:35 +0300 (file mtime) — sensitive-content review: none found — included: yes
- Temp git clone (exec-crossstack/, incl. degrade/ and repo/ subdirectories, session scratchpad of c9fab6de) — shareable filename: none assigned — Available — original (still present under /tmp as of 2026-08-04; session-temp, at risk of loss on reboot) — SHA-256: not recorded — capture timestamp Unknown — sensitive-content review: none found — included: no

### RUN-025 / TC-048

#### Workbook Record
- Recorded result: Blocked
- Framework: Infrastructure-Creator (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Not Applicable)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.0
- Git commit: 3a02537 (Starting Repository State: "Clean checkout at 3a02537; mutating steps ran in a temp copy")
- Execution date: 2026-08-03 (historical snapshot); current workbook "2026-08-03 00:00:00" (mechanical coercion)
- Failure Reason (quoted): "Requires interactive AI client run (LLM synthesis of the profile from scanner findings); no findings inputs exist in tasks/ either"
- Defect reference: None (workbook Defect ID empty; no TC-048 entry in defects.json)
- Workbook evidence link: Accelerator-TestEvidence/TC-048.log
- Row presence: exists in BOTH the current workbook and the historical snapshot; content identical apart from the mechanical rewrite (datetime, counts 0/0/0 to 0.0/0.0/0.0). Note: the historical Quality Metrics sheet already lacked a row for RUN-025 (a pre-existing gap, unchanged by the later workbook rewrite).
- Counts recorded: Tests Executed 0 / Passed 0 / Failed 0; Human Intervention 0; Retry 0; Safety Violations 0.

#### Test Definition
- Definition status: Authoritative — original source found (cases_map.json TC-048; the workbook's original 74-row Test Cases sheet agrees field-by-field, whitespace-normalized).
- Title: profile-synthesizer emits a schema-conformant profile with version from VERSION file (Category: Documentation; Priority: High; Owner: QA; Execution Type: Hybrid).
- Objective: verify the profile-synthesizer skill writes exactly one schema-conformant profile file whose generator version comes from the VERSION file, whose AI-tool selection comes strictly from the interview answer, and whose facts keep per-fact confidence tags and sources.
- Preconditions (quoted): "tasks/TASK-1/ contains all seven *-findings.md, stack-researcher-findings.md, and clarifying-interview-answers.md with 'editions: [claude]' (state after Case 1 steps 1-6, or re-invoke the synthesizer standalone on that task dir). Infrastructure-Creator/VERSION contains 1.4.0."
- Input (quoted): "Run profile-synthesizer for /tmp/fixtures/fake-php-app using the findings in tasks/TASK-1/"
- Mandatory assertions: writes exactly one file tasks/TASK-1/infra-scan-project-profile.md per references/project-profile-schema.md; Section 0 Generator version read from the root VERSION file (1.4.0), never hardcoded; Section 1 AI Tool Selection strictly from the interview answer (STOP and re-run clarifying-interview if the editions line is missing); Sections 2-8 preserve per-fact confidence tags and source paths; Section 11.1 lists skills in eight groups incl. the fixed 18 Process & Workflow skills with the memory quartet and the 7 Universal PHP skills; Section 11.2 counts arithmetically consistent; Section 12 previews only confirmed, source-linked concepts.
- Optional assertions: none stated.
- Scriptable assertions: the expected_verification greps (Generator version match against VERSION; exactly one "## 1. AI Tool Selection"; presence of the 11 named skills; every section 2-8 fact tagged confirmed/inferred/unknown; no password/api-key/secret matches) — scriptable ONLY once a profile exists.
- Native-client assertions: the synthesis itself — consuming seven scanner findings files, stack-researcher findings, and interview answers, and authoring the profile with confidence handling — is LLM/agent work requiring a native Claude Code client.
- Expected state changes (quoted): "Infrastructure-Creator/tasks/TASK-1/infra-scan-project-profile.md (exactly one file; nothing written into the target)."
- Safety expectations (quoted): "No edition assumed without the interview answer. No skill proposed for absent evidence (no framework-specialty skill for a none/unknown section 3.1 signal, no frontend group when 3.2 says it does not apply). No inferred fact laundered into confirmed. No secrets or customer data anywhere in the profile."
- Requirement-source: Infrastructure-Creator/.agents/skills/profile-synthesizer/SKILL.md (schema reference: .../references/project-profile-schema.md).

#### Historical Evidence
- Evidence status (main log): Available — original. It documents a capability block, not a product test: the run executed no assertions (0/0/0).
- Original artifacts: /home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-048.log (shareable name: logs/original/TC-048.log; 8 lines).
- Commands: none recorded — the log contains only the block rationale and static precondition observations; no command lines and no exit codes appear in it.
- Exit codes: none recorded — Unknown.
- Assertions: 0 executed / 0 passed / 0 failed (workbook counts; consistent with the Blocked result).
- Key output excerpt (log lines 1-3, 6-8, quoted exactly):
  ```
  == TC-048 blocked: profile-synthesizer is an LLM-authored artifact (needs the seven scanner findings + interview answers, produced only in an interactive AI session) ==
  -- static preconditions checked --
  VERSION: 1.4.0 (case expects 1.4.0: match)
  /home/aliaksei/Desktop/AI-Infrastructure/Infrastructure-Creator/.claude/skills/profile-synthesizer/references/project-profile-schema.md
  tasks/ content (no TASK-1 findings exist to synthesize from):
  README.md
  ```
- Priority aspects requested by the central team, each addressed explicitly:
  - Scanner findings (seven *-findings.md): evidence Unavailable — required AI client missing. The artifacts were never produced; the log affirmatively documents their absence ("tasks/ content (no TASK-1 findings exist to synthesize from): README.md"). That absence statement itself is Available — original (log lines 7-8).
  - Research (stack-researcher-findings.md): evidence Unavailable — required AI client missing (same basis; never produced in this scripted run).
  - Interview answers (clarifying-interview-answers.md with 'editions: [claude]'): evidence Unavailable — required AI client missing (never produced; absence covered by the same log lines).
  - Synthesis (the profile file tasks/TASK-1/infra-scan-project-profile.md): evidence Unavailable — required AI client missing (no profile was authored; nothing to verify against the schema).
  - Confidence handling (per-fact confirmed/inferred/unknown tags, no laundering): evidence Unavailable — required AI client missing (depends on a profile that does not exist).
  - No target generation (nothing written into the target or the repo): evidence Available — original. The workbook row records Files Created "None", Files Modified "None", Unexpected Changes "None", Safety Violation Count 0; the log shows read-only checks only; the execute-test-cases digest records the exec:ic zone-final confirmation "Zone-final: repo tree clean, HEAD 3a02537 unchanged" (digest section 6, exec:ic; TC-048 grouped with 056/057/058: "static preconditions each verified (VERSION 1.4.0, 23 skills, checkpoint policy text present)").
- Contradictions with workbook: none. The log's static observations (VERSION 1.4.0 match; skill directory with references/ and SKILL.md and the project-profile-schema.md path present; no TASK-1 findings) match the workbook's Actual Behaviour sentence for sentence.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. No retries documented (workbook Retry Count 0). Note: the later bwb live-session workflow (RUN-050..057) did NOT include TC-048; no live execution of this case exists anywhere in the evidence set.

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); native Claude Code client behavior was NOT exercised, and the case's ai_tool (Claude Code) demands exactly that for its core content. All synthesis-dependent assertions (schema conformance, VERSION sourcing into Section 0, interview-driven Section 1, confidence tagging, skill-group composition, secret-free output): Native assertions unavailable. Only the static preconditions were verified scriptally.

#### Evidence Assessment
- Primary classification: original-log-backed (for the Blocked determination and the static precondition facts only).
- Confidence: Medium in the recorded "Blocked" result — the original log, the exact commit, and the digest's zone-final cleanliness confirmation coherently establish that the case could not be executed scriptally and that nothing was mutated; but the log contains no commands or exit codes, so even the static checks rest on printed observations rather than a full transcript. Confidence in any statement about profile-synthesizer's actual behavior: Insufficient — the product functionality was never exercised.
- Evidence proves: the run was blocked for the stated reason; VERSION was 1.4.0 as the case requires; the skill and its schema reference existed at 3a02537; no synthesis inputs existed in tasks/; no files were created or modified.
- Evidence does not prove: anything about profile synthesis quality, schema conformance, confidence handling, edition-selection discipline, or secret hygiene — the case's entire mandatory assertion set is untested.
- Missing information: any native-client execution of profile-synthesizer; the seven findings files, researcher findings, and interview answers that would constitute its inputs.
- Recommended central-team follow-up: highest-value gap in this run range — execute TC-048 in a native Claude Code session after an infra-scan (TC-056) produces the TASK-1 findings, then apply the case's scriptable grep battery to the emitted profile. Treat the workbook's "Blocked" as a capability block, not a product verdict.

#### Artifact Inventory
- TC-048.log — shareable filename logs/original/TC-048.log — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 11:31:41 +0300 (file mtime) — sensitive-content review: none found — included: yes
- Case definition (TC-048 entry) — shareable filename manifests/cases_map.json — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 11:14:56 +0300 (file mtime) — sensitive-content review: none found — included: yes
- Workbook row (historical snapshot) — shareable filename manifests/workbook-snapshot-20260803-2002.xlsx — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 20:02:35 +0300 (file mtime) — sensitive-content review: none found — included: yes
- Profile and synthesis inputs (tasks/TASK-1/*-findings.md, stack-researcher-findings.md, clarifying-interview-answers.md, infra-scan-project-profile.md) — shareable filename: none — Unavailable — required AI client missing (never produced) — SHA-256: n/a — capture timestamp Not Applicable — sensitive-content review: Not Applicable — included: no

### RUN-026 / TC-049

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Not Applicable)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.3
- Git commit: 3a02537 (Starting Repository State: "Clean checkout at 3a02537; mutating steps ran in a temp copy")
- Execution date: 2026-08-03 (historical snapshot); current workbook "2026-08-03 00:00:00" (mechanical coercion)
- Defect reference: None (workbook Defect ID empty; no TC-049 entry in defects.json)
- Workbook evidence link: Accelerator-TestEvidence/TC-049.log
- Row presence: exists in BOTH the current workbook and the historical snapshot; content identical apart from the mechanical rewrite (datetime, counts 19/19/0 to 19.0/19.0/0.0). Linter Result: "bash -n clean".
- Counts recorded: Tests Executed 19 / Passed 19 / Failed 0; Human Intervention 0; Retry 0; Safety Violations 0.

#### Test Definition
- Definition status: Authoritative — original source found (cases_map.json TC-049; the workbook's original 74-row Test Cases sheet agrees field-by-field, whitespace-normalized).
- Title: file-naming-validator blocks invalid task/spec/chunk names and validates Codex apply_patch markers (Category: Agent orchestration; Priority: High; Owner: QA).
- Objective: verify the file-naming-validator hook blocks invalid Markdown paths under tasks/, specs/, and memory-bank/chunks/ with exit 2 and specific stderr messages, parses Codex apply_patch markers, and passes all allowed paths silently.
- Preconditions (quoted): "Laravel edition checked out so the hook can discover skill directories under .claude/skills (its skill-prefix source); jq/php/python3 available; run from the repo so ROOT_DIR resolves to the edition root."
- Input (quoted): `printf '%s' '{"tool_name":"Write","tool_input":{"file_path":"tasks/bad-file.md","content":"hello"}}' | bash Laravel/.claude/hooks/file-naming-validator.sh; echo "exit=$?"` — then the patch form: `printf '%s' '{"tool_name":"apply_patch","tool_input":{"patch":"*** Begin Patch\n*** Add File: tasks/bad-file.md\n+hello\n*** End Patch\n"}}' | bash Laravel/.claude/hooks/file-naming-validator.sh`
- Mandatory assertions: tasks/bad-file.md exit 2 with "BLOCKED: Task Markdown must be inside tasks/TASK-N/" plus "See AGENTS.md for repository file-naming rules."; loose task dir blocked (^TASK-[0-9]{3,}$ shape); unprefixed task/spec files blocked (skill-prefix rule); unprefixed chunk blocked (MEM-[0-9]{4,}-slug.md rule); apply_patch "*** Add File:" markers validated (bad path blocks, tasks/TASK-003/coder-report.md passes); allowed paths (tasks/README.md, specs/MANIFEST.md, MEM-0001-first-note.md, docs/free-form-notes.md, src/Example.php, tasks/TASK-001/<skill>-notes.md) pass unchanged; escaped "file_path" inside a content string ignored.
- Optional assertions: none stated. (Sheet Notes flag a deliberate cross-edition difference: the Infrastructure-Creator edition runs the same validator in WARN mode, exit 1 — do not copy Laravel expectations there.)
- Scriptable assertions: all of the above, plus the unittest battery `cd "Laravel/memory-bank/tests" && python3 -m unittest test_hooks.FileNamingValidatorTest -v` (OK across .claude/.cursor/.codex mirrors).
- Native-client assertions: the hook's live dispatch points (Claude PreToolUse Write|Edit; Cursor afterFileEdit; Codex every-tool PreToolUse) — actual client-driven invocation was not exercised.
- Expected state changes (quoted): "None (the hook only inspects payloads; the blocked file must never be created by the harness)".
- Safety expectations (quoted): "Non-editing tools (Read of a badly named file) must never block. Non-Markdown files are out of scope and must pass. stdout stays empty in every outcome. The hook must not create, rename, or delete files itself."
- Requirement-source: "Laravel/.claude/hooks/file-naming-validator.sh; Laravel/memory-bank/tests/test_hooks.py (FileNamingValidatorTest)".

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: /home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-049.log (shareable name: logs/original/TC-049.log; 49 lines).
- Commands: the log records section markers ("--- unittest full class ---", "--- blocked: tasks/bad-file.md ---", "--- apply_patch bad ---", "--- allowed paths ---", "--- escaped file_path inside content ignored ---", "--- Read of badly named file passes ---"), not literal command lines; the payload commands are specified in the case definition `input_prompt`.
- Exit codes (quoted from log): five blocked probes each "exit=2 stdout=0B"; "apply_patch bad" "exit=2"; "apply_patch good" "exit=0 stderr=0B"; six allowed paths each "exit=0 stderr=0B"; escaped-file_path probe "exit=0 stderr=0B"; Read probe "exit=0 stderr=0B". Unittest: "Ran 5 tests in 0.767s" / "OK".
- Assertions: 19 executed / 19 passed / 0 failed (workbook counts). The log itemizes 5 unittest passes plus 15 manual probe outcomes (5 blocked + 2 apply_patch + 6 allowed + escaped-content + Read) = 20 itemized outcomes; the workbook's aggregation to 19 is Unknown (see Contradictions).
- Key output excerpt (log lines 13-16, 31-32, 41-42, quoted exactly):
  ```
  --- blocked: tasks/bad-file.md ---
  exit=2 stdout=0B
  BLOCKED: Task Markdown must be inside tasks/TASK-N/: 'tasks/bad-file.md'
    See AGENTS.md for repository file-naming rules.
  BLOCKED: Memory chunks must use memory-bank/chunks/MEM-0001-short-slug.md: 'memory-bank/chunks/unprefixed-note.md'
    See AGENTS.md for repository file-naming rules.
  specs/MANIFEST.md exit=0 stderr=0B
  memory-bank/chunks/MEM-0001-first-note.md exit=0 stderr=0B
  ```
- Contradictions with workbook: one minor count discrepancy — the log itemizes 20 outcomes (5 unittest + 15 manual) while the workbook records Tests Executed 19; every itemized outcome is a pass either way, so the recorded verdict is unaffected. All quoted BLOCKED messages, stream discipline (stdout=0B on blocks, stderr=0B on passes), and the apply_patch marker behavior match the workbook's Actual Behaviour exactly.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. The log documents no retries (workbook Retry Count 0).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); the hook was fed hand-crafted JSON payloads over stdin. The case's ai_tool is Cross-tool: native dispatch by the Claude Code PreToolUse pipeline, the Cursor afterFileEdit pipeline, and the Codex every-tool pipeline was NOT exercised — those wiring-level assertions: Native assertions unavailable. The payload-level contract (including the Codex apply_patch marker format) was fully exercised scriptally, and the unittest battery covers all three mirror copies of the hook.

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium — original log, exact commit, per-probe exit codes, and stream-size annotations are present, but literal command lines are absent from the log and there is a one-count aggregation discrepancy (20 itemized vs 19 recorded).
- Evidence proves: every blocked path produced exit 2 with the exact rule-specific BLOCKED stderr message and empty stdout; apply_patch Add-File markers are parsed and enforced; all allowed paths, the escaped-content probe, and the Read probe passed silently with exit 0; the mirrored test class passed 5/5.
- Evidence does not prove: behavior under real client dispatch (event wiring, matcher scope); the IC edition's WARN-mode variant (out of scope by design).
- Missing information: literal command transcript; basis of the workbook's 19-count.
- Recommended central-team follow-up: reconcile the 19-vs-20 count; optionally add a native-client probe (a real Write attempt in Claude Code) to close the dispatch-wiring gap.

#### Artifact Inventory
- TC-049.log — shareable filename logs/original/TC-049.log — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 11:31:41 +0300 (file mtime) — sensitive-content review: none found — included: yes
- Case definition (TC-049 entry) — shareable filename manifests/cases_map.json — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 11:14:56 +0300 (file mtime) — sensitive-content review: none found — included: yes
- Workbook row (historical snapshot) — shareable filename manifests/workbook-snapshot-20260803-2002.xlsx — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 20:02:35 +0300 (file mtime) — sensitive-content review: none found — included: yes

### RUN-027 / TC-050

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Not Applicable)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.3
- Git commit: 3a02537 (Starting Repository State: "Clean checkout at 3a02537; mutating steps ran in a temp copy")
- Execution date: 2026-08-03 (historical snapshot); current workbook "2026-08-03 00:00:00" (mechanical coercion)
- Defect reference: None (workbook Defect ID empty; no TC-050 entry in defects.json)
- Workbook evidence link: Accelerator-TestEvidence/TC-050.log
- Row presence: exists in BOTH the current workbook and the historical snapshot; content identical apart from the mechanical rewrite (datetime, counts 13/13/0 to 13.0/13.0/0.0). Linter Result: "bash -n clean".
- Counts recorded: Tests Executed 13 / Passed 13 / Failed 0; Human Intervention 0; Retry 0; Safety Violations 0.

#### Test Definition
- Definition status: Authoritative — original source found (cases_map.json TC-050; the workbook's original 74-row Test Cases sheet agrees field-by-field, whitespace-normalized).
- Title: loop-detection warns on the 7th edit of a file and blocks on the 10th (Category: Agent orchestration; Priority: High; Owner: QA).
- Objective: verify the loop-detection PostToolUse hook stays silent for edits 1-6, warns on stdout with exit 1 for edits 7-9, blocks on stderr with exit 2 on edit 10, and keeps its counter in per-repo-namespaced /tmp state.
- Preconditions (quoted): "Run inside a scratch git repository (counters are namespaced by 'git rev-parse --show-toplevel' cksum); clear prior state first: rm -rf /tmp/claude-loop-detection-* (temp counters only)."
- Input (quoted): `for i in $(seq 1 10); do printf '%s' '{"tool_name":"Edit","tool_input":{"file_path":"/work/app/Example.php","old_string":"a","new_string":"b"}}' | bash <abs-path>/Laravel/.claude/hooks/loop-detection.sh; echo "run $i exit=$?"; done`
- Mandatory assertions: exits read 0,0,0,0,0,0,1,1,1,2 in order; edits 7-9 print the stdout warning naming the file and edit count with the /debugger suggestion, stderr empty; edit 10 exits 2 with the stderr BLOCKED repeated-edit-loop message, stdout empty; counter lives in /tmp/claude-loop-detection-<cksum-of-repo-root>/<md5-of-file-path> and increments by exactly 1 per Edit payload.
- Optional assertions: none stated. (Sheet Notes flag a deliberate cross-edition difference: the Infrastructure-Creator edition warns at 5 with a 120s quiet-window reset.)
- Scriptable assertions: all of the above, plus the unittest `cd "Laravel/memory-bank/tests" && python3 -m unittest test_hooks.LoopDetectionTest.test_warn_and_block_thresholds -v` (OK for all three mirrors); `ls /tmp/claude-loop-detection-*/` shows exactly one counter file containing '10'.
- Native-client assertions: live PostToolUse dispatch with matcher "Edit" by a native Claude Code client (registration per Laravel/.claude/settings.json) — not exercised in scripted mode.
- Expected state changes (quoted): "One counter file under /tmp/claude-loop-detection-<repo-key>/ (temp state, not in the repo)".
- Safety expectations (quoted): "Warnings go to stdout and the block goes to stderr (the harness surfaces them differently) — a swap is a defect. A payload with tool_name Edit but no file_path must not create any counter (test_missing_file_path_is_ignored). The hook must never modify the edited file or kill the session other than by exit 2 on the 10th edit."
- Requirement-source: "Laravel/.claude/hooks/loop-detection.sh (thresholds comment: 'warn at 7, block at 10'); Laravel/.claude/settings.json (PostToolUse registration)".

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: /home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-050.log (shareable name: logs/original/TC-050.log; 31 lines).
- Commands: the log records section markers ("--- unittest ---", "--- manual loop in scratch repo ---", "--- counter state ---", "--- missing file_path payload creates no counter ---"), not literal command lines; the loop command is specified in the case definition `input_prompt`.
- Exit codes (quoted from log): "run 1 exit=0" through "run 6 exit=0" (all "stdout=[] stderr=[]"); "run 7 exit=1", "run 8 exit=1", "run 9 exit=1" (warning on stdout, "stderr=[]"); "run 10 exit=2" (block on stderr, "stdout=[]"); missing-file_path probe "exit=0". Unittest: "Ran 1 test in 0.353s" / "OK".
- Assertions: 13 executed / 13 passed / 0 failed (workbook counts; consistent with 1 unittest + 10 loop exits + counter-state check + missing-file_path check).
- Key output excerpt (log lines 16-17, 22-24, 26-27, 29-31, quoted exactly):
  ```
  run 7 exit=1 stdout=[⚠️  WARNING: File '/work/app/Example.php' edited 7 times this session.
     If you're stuck in a loop, consider using ] stderr=[]
  run 10 exit=2 stdout=[] stderr=[BLOCKED: File '/work/app/Example.php' edited 10 times this session.
     This looks like a repeated-edit loop. Consider:
   ]
  0a8ad946181e607fe1a7b181044760f8
  10
  --- missing file_path payload creates no counter ---
  exit=0
  no-counter-dirs=2
  ```
- Contradictions with workbook: none substantive. The exits sequence 0,0,0,0,0,0,1,1,1,2, the warn-on-stdout / block-on-stderr stream separation, the single counter file containing '10', and the ignored missing-file_path payload all match the workbook's Actual Behaviour. Two limitations of the log itself: the captured stdout/stderr snippets are truncated mid-sentence (e.g. "consider using ]" cuts off before the /debugger token, so the exact suggestion text is not fully evidenced), and the final line "no-counter-dirs=2" is recorded without an in-log baseline count — the workbook's claim that no NEW counter directory was created is consistent with, but not independently derivable from, the log alone.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. The log documents no retries (workbook Retry Count 0).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); native Claude Code client behavior was NOT exercised for this Claude Code-designated case. The hook received hand-piped JSON payloads; live PostToolUse dispatch with matcher "Edit" and the client-side surfacing of the stdout warning vs stderr block were not observed — those assertions: Native assertions unavailable. The hook's own threshold, stream, exit-code, and counter contract was fully exercised scriptally, and warn-at-7/block-at-10 is additionally covered by the mirrored unittest.

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium — original log, exact commit, and a complete per-iteration exit-code sequence with stream annotations are present, but literal command lines are absent from the log and the captured message texts are truncated.
- Evidence proves: threshold behavior (silent 1-6, warn 7-9 exit 1, block at 10 exit 2), correct stream separation (warning on stdout with empty stderr; block on stderr with empty stdout — the exact property the case flags as a defect if swapped), one counter file containing '10', and that an Edit payload without file_path exits 0.
- Evidence does not prove: the full warning/block message wording beyond the captured prefixes; counter-directory non-creation for the missing-file_path probe in isolation (no in-log baseline for "no-counter-dirs=2"); live PostToolUse dispatch.
- Missing information: literal command transcript; untruncated message capture; the scratch-repo path (workbook: scratchpad/exec-hooks/scratch-a).
- Recommended central-team follow-up: none critical; on any future re-run capture untruncated streams and a before/after counter-directory listing.

#### Artifact Inventory
- TC-050.log — shareable filename logs/original/TC-050.log — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 11:31:41 +0300 (file mtime) — sensitive-content review: none found — included: yes
- Case definition (TC-050 entry) — shareable filename manifests/cases_map.json — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 11:14:56 +0300 (file mtime) — sensitive-content review: none found — included: yes
- Workbook row (historical snapshot) — shareable filename manifests/workbook-snapshot-20260803-2002.xlsx — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 20:02:35 +0300 (file mtime) — sensitive-content review: none found — included: yes
- Scratch repo (exec-hooks/scratch-a, session scratchpad of c9fab6de) — shareable filename: none assigned — Available — original (still present under /tmp as of 2026-08-04; session-temp, at risk of loss on reboot) — SHA-256: not recorded — capture timestamp Unknown — sensitive-content review: none found — included: no
- /tmp loop-detection counter files — shareable filename: none — Unavailable — deleted (transient temp state; no /tmp/claude-loop-detection-* directories exist as of 2026-08-04) — SHA-256: n/a — capture timestamp Unknown — sensitive-content review: Not Applicable — included: no

### RUN-028 / TC-051

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Not Applicable)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.3
- Git commit: 3a02537 (equals current HEAD of AI-Infrastructure)
- Execution date: recorded as "2026-08-03" in the historical snapshot; the current workbook shows the mechanically coerced form "2026-08-03 00:00:00"
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-051.log`
- Row presence: the row exists in BOTH the current workbook and the historical snapshot; content is identical apart from the current workbook's mechanical rewrite (date-to-datetime coercion, six count columns int-to-float, e.g. Tests Executed "9" -> "9.0", plus 40 empty padding columns "Column 31".."Column 70").
- Recorded counts: Tests Executed 9 / Passed 9 / Failed 0; Linter Result "bash -n clean"; Human Intervention 0; Retry 0; Safety Violations 0.

#### Test Definition
- Definition status: Authoritative — original source found (present in `manifests/cases_map.json` as TC-051; the historical workbook "Test Cases" sheet row TC-051 agrees on every overlapping field after whitespace normalization).
- Title: "loop-detection counters are namespaced per repository and reset only for the own repo at SessionStart"
- Objective: prove that loop-detection state is keyed per git repository and that the SessionStart hook (`local-context.sh`) wipes only its own repository's counters, never another project's.
- Preconditions: two scratch git repositories repo-a and repo-b initialized (`git init`); no pre-existing `/tmp/claude-loop-detection-*` state for either.
- Input: run the Edit payload for the SAME file path `/work/app/Example.php` nine times with cwd=repo-a, then once with cwd=repo-b; then execute `bash <abs-path>/Laravel/.claude/hooks/local-context.sh` with cwd=repo-a; then one more Edit payload in each repo.
- Mandatory assertions: (1) REPO_KEY derived per repo (`git rev-parse --show-toplevel` piped to cksum) yields distinct `/tmp/claude-loop-detection-<key>` directories; (2) after nine edits in repo-a, the first edit in repo-b exits 0 with no warning; (3) repo-a's tenth edit blocks (exit 2); (4) `local-context.sh` deletes only files under repo-a's own namespace — repo-b's counter file untouched; (5) the next edit in repo-a starts from count 1 (exit 0).
- Optional assertions: per the definition's Notes, the version-banner and governance lines printed by local-context.sh are covered by LocalContextTest, not this case.
- Scriptable assertions: `cd "Laravel/memory-bank/tests" && python3 -m unittest test_hooks.LoopDetectionTest.test_counters_namespaced_per_repository test_hooks.LocalContextTest.test_session_start_resets_only_own_repo_counters -v` -> OK; manual: after the SessionStart run, `ls /tmp/claude-loop-detection-<key-a>/` is empty and `ls /tmp/claude-loop-detection-<key-b>/` still lists one file.
- Native-client assertions: none — Execution Type is "Automated"; the case exercises the hook scripts directly.
- Expected state changes: temp counter files under `/tmp/claude-loop-detection-<key-a>` and `/tmp/claude-loop-detection-<key-b>` only; no repo files.
- Safety expectations: SessionStart must never wipe another project's counters; local-context.sh must always exit 0 (informational) and must not run indexing/retrieval or print memory-record contents — metadata only, per AGENTS.md.
- Requirement-source: `Laravel/.claude/hooks/local-context.sh` (counter reset, lines 18-22); `Laravel/.claude/hooks/loop-detection.sh` (REPO_KEY namespacing); `Laravel/AGENTS.md` (session hooks report metadata only). Priority: High; Owner: QA.

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-051.log` (shareable name: `logs/original/TC-051.log`), 19 lines, byte-copy of the executor's scratchpad evidence file.
- Commands: no verbatim command lines are recorded in the log; it records section markers and outcomes. The unittest output is consistent with the definition's verification command (quoted above from cases_map.json, not from the log).
- Exit codes recorded in the log: `repo-b first edit exit=0`; `repo-a 10th edit exit=2`; `local-context exit=0`; `repo-a fresh edit exit=0`.
- Assertions: Passed 9 / Failed 0 / Skipped 0 (workbook and execute-test-cases digest, "9/9/0"); the log itself itemizes 2 unittest results ("Ran 2 tests in 1.171s" / "OK") plus the manual assertion outcomes.
- Key output excerpt (verbatim; log lines 11-14 and 16-18):
  ```
  key-a=3362089479 key-b=3129882068
  repo-a after 9 edits: 9
  repo-b first edit exit=0 stdout=0B stderr=0B count=1
  repo-a 10th edit exit=2 stderr1=[BLOCKED: File '/work/app/Example.php' edited 10 times this session.]
  local-context exit=0
  repo-a counters left: 0
  repo-b counters left: 1
  ```
- Contradictions with workbook: none. The workbook's "Tests Executed: 9" aggregates unittest plus manual assertion steps; the log does not itemize the count of 9, but every workbook behavioural claim (distinct keys 3362089479/3129882068, block at 10th edit, reset scoping) appears verbatim in the log.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. The log documents no historical retries (digest: Retries 0, first attempt Yes).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session, session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, executor exec:hooks). The case's ai_tool is "Cross-tool" and its Execution Type is "Automated": it targets the hook scripts directly, so no native-client assertion exists for this case. Behavior of any real editor host actually dispatching these payloads was NOT exercised.

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium — the original log, exact commit (3a02537, clean tree per TC-001.log), and all decisive exit codes are present, but complete verbatim command lines are not recorded in the log, so the exact invocation can only be inferred from the authoritative definition.
- Evidence proves: per-repo counter namespacing (distinct cksum keys), non-interference of repo-b, block at the 10th edit (exit 2 with the BLOCKED message), and SessionStart reset scoped to the own repo only.
- Evidence does not prove: the literal payload/command strings used (not logged) and the itemized composition of the "9 tests" count.
- Missing information: verbatim commands; per-assertion enumeration.
- Recommended central-team follow-up: none beyond command-echo logging in future harness runs.

#### Artifact Inventory
- TC-051 evidence log — shareable filename `logs/original/TC-051.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found (cksum-derived keys only) — included: yes.
- Workbook rows (current + historical snapshot exports: `our-runs.json`, `historical-runs.json`) — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: workbook snapshot 2026-08-03 19:56 local (historical), current workbook as delivered 2026-08-04 — sensitive-content review: none found — included: yes.

### RUN-029 / TC-052

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Codex (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Not Applicable)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.3
- Git commit: 3a02537 (equals current HEAD of AI-Infrastructure)
- Execution date: recorded as "2026-08-03" in the historical snapshot; current workbook shows the coerced form "2026-08-03 00:00:00"
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-052.log`
- Row presence: exists in BOTH current workbook and historical snapshot; identical apart from the mechanical rewrite (datetime coercion, count columns "18" -> "18.0", 40 empty padding columns).
- Recorded counts: Tests Executed 18 / Passed 18 / Failed 0; Linter Result "bash -n clean"; Human Intervention 0; Retry 0; Safety Violations 0.

#### Test Definition
- Definition status: Authoritative — original source found (`manifests/cases_map.json` TC-052; historical "Test Cases" sheet row agrees on every overlapping field).
- Title: "Codex early-exit: read-only payloads never advance the loop counter or trigger validators"
- Objective: prove that read-only tool payloads can never advance the Codex loop counter or trigger the bash/file-naming validators, while genuine Edit payloads without a tool_name still count (documented fail-open).
- Preconditions: `Laravel/.codex/hooks.json` registers all PreToolUse/PostToolUse hooks WITHOUT a matcher and `Laravel/.codex/config.toml` has `hooks = true`; scratch git repo as cwd; clean `/tmp/codex-loop-detection-*` state.
- Input: run 12 times: `printf '%s' '{"tool_name":"Read","tool_input":{"file_path":"/work/app/Example.php"}}' | bash <abs-path>/Laravel/.codex/hooks/loop-detection.sh; echo "exit=$?"` — then the same Read payload against `Laravel/.codex/hooks/bash-validator.sh` and `Laravel/.codex/hooks/file-naming-validator.sh`.
- Mandatory assertions: (1) loop-detection extracts tool_name "Read", which fails the editing-tool allow-list regex, and exits 0 immediately — after 12 runs no `/tmp/codex-loop-detection-<key>` directory exists; (2) bash-validator's cheap self-filter exits 0 before forking, since a Read payload carries no command key; (3) file-naming-validator extracts tool_name Read and exits 0; (4) all three produce empty stdout and empty stderr; (5) an Edit payload with empty/absent tool_name still counts (fail-open for hosts like Cursor afterFileEdit that send none).
- Optional assertions: per the definition's Notes, the same allow-list guards the Cursor mirror; Cursor's afterFileEdit sends no tool_name (the documented fail-open case).
- Scriptable assertions: `cd "Laravel/memory-bank/tests" && python3 -m unittest test_hooks.LoopDetectionTest.test_read_only_tool_payload_with_file_path_does_not_count test_hooks.BashValidatorTest.test_payload_without_command_key_passes_quietly test_hooks.FileNamingValidatorTest.test_non_edit_tool_payloads_ignored -v` -> OK; manual: all 12 runs print exit=0 and `ls /tmp/codex-loop-detection-* 2>/dev/null` prints nothing.
- Native-client assertions: none in the definition (Execution Type "Automated"); actual hook dispatch by a real Codex Desktop host is outside the case's scriptable scope.
- Expected state changes: "None — specifically, NO counter directory may be created."
- Safety expectations: a Codex session reading one file many times must never be warned or blocked as a doom loop (previous regression: Read payloads with file_path advanced the counter); no stray output Codex could inject into context; fail-open must not extend to actual Edit payloads.
- Requirement-source: `Laravel/.codex/hooks.json` (matcher-less registration); `Laravel/.claude/hooks/loop-detection.sh` (tool-name allow-list comment, lines 10-19); `Laravel/memory-bank/tests/test_hooks.py`. Priority: High; Owner: QA.

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-052.log` (shareable name: `logs/original/TC-052.log`), 22 lines.
- Commands: no verbatim command lines recorded in the log; section markers name the operations ("--- 12 Read payloads to codex loop-detection ---", "--- same Read payload to codex bash-validator and file-naming-validator ---"). The unittest output matches the definition's verification command (quoted above from cases_map.json).
- Exit codes recorded in the log: "all 12 runs: exit=0, empty stdout/stderr"; "bash-validator exit=0 out=0B err=0B"; "file-naming exit=0 out=0B err=0B"; Edit payload with empty tool_name: "exit=0".
- Assertions: Passed 18 / Failed 0 / Skipped 0 (workbook and digest "18/18/0"); log itemizes 3 unittest results ("Ran 3 tests in 0.073s" / "OK") plus the manual outcomes.
- Key output excerpt (verbatim; log lines 12-14, 18-19, and 21-22):
  ```
  --- 12 Read payloads to codex loop-detection ---
  all 12 runs: exit=0, empty stdout/stderr
  no codex counter dirs (OK)
  --- Edit payload with EMPTY tool_name still counts (fail-open for Cursor-style hosts) ---
  exit=0
  counter created for absent tool_name (OK per spec)
  1
  ```
- Contradictions with workbook: none. Wiring preconditions are attested by reference only — log line "--- hooks.json: no matcher + hooks=true (verified in TC-005) ---" delegates that evidence to TC-005.log, exactly as the workbook row states.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. No historical retries documented (digest: Retries 0, first attempt Yes).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, executor exec:hooks). The case's ai_tool is "Codex", but its Execution Type is "Automated" and it pipes payloads into the hook scripts directly; a native Codex Desktop client actually dispatching hooks per `.codex/hooks.json` was NOT exercised — that host-side dispatch behavior is a native assertion and is unavailable for this run.

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium — original log, exact commit, and all exit codes present; complete verbatim command lines are not in the log (inferable from the authoritative definition), and the hooks.json/config.toml precondition is evidenced only by cross-reference to TC-005.
- Evidence proves: read-only payloads never create counter state in the Codex mirror (12 consecutive runs, no counter dir), quiet exit-0 pass-through of the two validators on Read payloads, and the specified fail-open counting for a tool_name-less Edit payload.
- Evidence does not prove: behavior under a real Codex host runtime; the literal command strings; the itemized composition of the "18 tests" count.
- Missing information: verbatim commands; direct (non-cross-referenced) capture of the wiring precondition.
- Recommended central-team follow-up: pair this case with a live Codex Desktop session to confirm host-side hook dispatch (the only untested link in the chain).

#### Artifact Inventory
- TC-052 evidence log — shareable filename `logs/original/TC-052.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found (a 32-hex hash appears at log line 20 — the md5 of the payload file path /work/app/Example.php, used as the counter-file name per the hook's SAFE_NAME derivation; derived state naming, not a credential) — included: yes.
- Cross-referenced wiring evidence `logs/original/TC-005.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: 2026-08-03 11:31 +0300 — sensitive-content review: none found — included: yes (covered by the RUN-005 section).
- Workbook rows (current + historical exports) — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: historical snapshot 2026-08-03 19:56 local; current workbook as delivered 2026-08-04 — sensitive-content review: none found — included: yes.

### RUN-030 / TC-053

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Governed)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.3
- Git commit: 3a02537 (equals current HEAD)
- Execution date: "2026-08-03" (historical snapshot); "2026-08-03 00:00:00" in the current workbook (mechanical coercion)
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-053.log`
- Row presence: exists in BOTH current workbook and historical snapshot; identical apart from the mechanical rewrite (datetime coercion, "44" -> "44.0", 40 empty padding columns).
- Recorded counts: Tests Executed 44 / Passed 44 / Failed 0; Linter Result "Not Applicable"; Human Intervention 0; Retry 0; Safety Violations 0.
- Workbook comment (caveat recorded by the tester): "Live transcript portion (exactly one Task spawn and one Skill invocation at runtime) requires an interactive session and was not executed; the static contract the case defines as the sweep is fully closed, so Pass with that caveat."

#### Test Definition
- Definition status: Authoritative — original source found (`manifests/cases_map.json` TC-053; historical "Test Cases" sheet row agrees on every overlapping field).
- Title: "Command→agent→skill wiring: every slash command spawns its agent and the agent invokes exactly its own skill"
- Objective: verify the full command-to-agent-to-skill contract — representative /coder triple in depth, plus a static sweep proving every command that declares `spawns:` closes its triple.
- Preconditions: Laravel accelerator installed in a host project; `Laravel/.claude/commands/`, `.claude/agents/`, `.claude/skills/` all present; for the live check, Claude Code opened in that workspace.
- Input: `/coder Add an invitation expiry check to the InviteController`
- Mandatory assertions: (1) command frontmatter declares `spawns: coder-agent` and uses the Task tool with subagent_type `coder`, passing $ARGUMENTS; (2) `coder-agent.md` declares `invokes: coder` and its first step is "Use the Skill tool to invoke coder skill"; (3) the agent per its Constraints must ONLY execute the coder skill, NOT chain, NOT make workflow decisions; (4) final output contains a Context Summary and flow suggestions matching frontmatter (flow-next: code-reviewer; alternatives include test-generator); (5) statically, the triple `<command>.md -> <name>-agent.md -> skills/<name>/SKILL.md` closes for every command declaring `spawns:`.
- Optional assertions: per Notes, command basename and skill name can legally differ (git-worktrees -> using-git-worktrees; debugger -> systematic-debugger), so the sweep must follow the `spawns:`/`invokes:` frontmatter, not name matching.
- Scriptable assertions: the sweep loop over `Laravel/.claude/commands/*.md` expecting zero "BROKEN" lines (full command text in cases_map.json), then `cd Laravel && python3 memory-bank/scripts/context.py parity --json` -> exit 0, no drift for commands/agents/skills classes.
- Native-client assertions: "Live: transcript shows exactly one Task spawn and exactly one Skill invocation" — requires an interactive Claude Code session.
- Expected state changes: wiring check itself creates None (feature files a real coder run would create are out of scope).
- Safety expectations: the agent must not invoke a second skill, must not spawn further sub-agents, must not continue past skill completion into autonomous chaining; the spawned agent inherits the hook layer.
- Requirement-source: `Laravel/.claude/commands/coder.md`; `Laravel/.claude/agents/coder-agent.md` (Constraints section); `Laravel/.claude/skills/SKILL FLOW.md`. Priority: Critical; Owner: QA. Execution Type: Hybrid.

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-053.log` (shareable name: `logs/original/TC-053.log`), 32 lines.
- Commands: no verbatim command lines recorded; the log records grep-style extracts (line-number:content) from the wiring files and section markers. The parity block matches the definition's `context.py parity --json` verification (command text quoted from cases_map.json, not from the log).
- Exit codes recorded in the log: `parity exit=0`.
- Assertions: Passed 44 / Failed 0 / Skipped 0 (workbook and digest "44/44/0"; the digest glosses this as "static sweep of all 42 command files → zero broken command→agent→skill triples; parity valid"). The log itself shows: "sweep clean", "commands-with-spawns=40 total-commands=42", the /coder triple extracts, and closed triples for git-worktrees and security-reviewer.
- Key output excerpt (verbatim; log lines 14-16 and 25-29):
  ```
  --- static sweep over all commands (expect no BROKEN lines) ---
  sweep clean
  commands-with-spawns=40 total-commands=42
  --- mirror parity ---
  parity exit=0
  {
   "valid": true,
   "canonical_edition": ".agents",
  ```
- Contradictions with workbook: none in substance. Two observations: (a) the workbook records Operating Mode "Governed" for a purely static file check (the digest's TC-053 row also says Governed, so the sources agree with each other); (b) the "44 tests" count is not itemized in the log (40 sweep triples + the representative-triple/parity checks is a plausible but unconfirmed decomposition — Unknown).

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. No historical retries documented (digest: Retries 0, first attempt Yes).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, executor exec:hooks); native Claude Code client behavior was NOT exercised. The case's native-client assertion — a live transcript showing exactly one Task spawn and exactly one Skill invocation for `/coder` — is therefore marked: Native assertions unavailable. The workbook row records this caveat explicitly.

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium — original log and exact commit present; the sweep command is not echoed verbatim in the log and the live half of this Hybrid case was not executed, so "Pass" rests on the static contract only.
- Evidence proves: the /coder, git-worktrees, and security-reviewer triples are closed; zero broken `spawns:`/`invokes:` links across all 42 command files (40 with spawns); mirror parity valid with empty drift lists at commit 3a02537.
- Evidence does not prove: runtime behavior (single Task spawn, single Skill invocation, agent stopping after skill completion) — untested without a native client.
- Missing information: verbatim sweep command in the log; itemization of the 44-count; any live transcript.
- Recommended central-team follow-up: execute the live half (`/coder` in a real Claude Code workspace) once and attach the transcript; the static half needs no rework.

#### Artifact Inventory
- TC-053 evidence log — shareable filename `logs/original/TC-053.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found — included: yes.
- Workbook rows (current + historical exports) — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: historical snapshot 2026-08-03 19:56 local; current workbook as delivered 2026-08-04 — sensitive-content review: none found — included: yes.

### RUN-031 / TC-054

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Not Applicable)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.3
- Git commit: 3a02537 (equals current HEAD)
- Execution date: "2026-08-03" (historical snapshot); "2026-08-03 00:00:00" in the current workbook (mechanical coercion)
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-054.log`
- Row presence: exists in BOTH current workbook and historical snapshot; identical apart from the mechanical rewrite (datetime coercion, "14" -> "14.0", 40 empty padding columns).
- Recorded counts: Tests Executed 14 / Passed 14 / Failed 0; Linter Result "Not Applicable"; Human Intervention 0; Retry 0; Safety Violations 0.
- Workbook comment (caveat recorded by the tester): "Behavioral run (actual worktree creation, .env confirmation step in transcript, migrations) requires a live AI session and a Laravel host project; per assignment this case was executed as a structural file check, which fully matched — Pass with that caveat."

#### Test Definition
- Definition status: Authoritative — original source found (`manifests/cases_map.json` TC-054; historical "Test Cases" sheet row agrees on every overlapping field).
- Title: "/git-worktrees creates an isolated worktree without leaking .env or touching shared databases"
- Objective: verify the /git-worktrees command/agent/skill chain sets up an isolated Laravel worktree safely — .env never committed or silently copied, migrations local-only, hook layer still active inside the worktree.
- Preconditions: Laravel host project on a clean main branch with at least one commit; composer available; a .env with local DB credentials exists in the main worktree; enough disk for a second checkout.
- Input: `/git-worktrees Set up an isolated worktree for feature/invoice-export`
- Mandatory assertions: (1) command spawns the using-git-worktrees agent, which invokes only the using-git-worktrees skill; (2) skill follows its Safe Workflow (`git worktree add ../<project>-feature feature/invoice-export`, then `composer install`); (3) .env copied/symlinked ONLY after confirming secrets/DB credentials — never committed, never copied silently; DB_DATABASE adjusted per worktree; (4) `php artisan key:generate` only if the copied .env lacks APP_KEY; (5) `php artisan migrate` targets the per-worktree database only; `php artisan test` before returning to main; (6) branch naming follows intent prefixes; (7) final output returns worktree path, branch, setup commands, Context Summary, next command (flow-next: coder).
- Optional assertions: per Notes, frontend install/build steps run only if frontend tooling exists; verify the closing suggestion matches flow-next coder.
- Scriptable assertions (as defined, live-run verification): `git worktree list | grep feature/invoice-export`; `git -C ../<project>-feature branch --show-current` -> feature/invoice-export; porcelain grep for staged .env -> empty; `git -C ../<project>-feature check-ignore .env` -> .env.
- Native-client assertions: the entire behavioral run (worktree creation, .env confirmation step visible in transcript, migrations, tests) — Execution Type is "Manual", i.e. requires a live AI session on a Laravel host project.
- Expected state changes: new worktree directory `../<project>-feature` (outside the main checkout) with vendor/ and an untracked .env; no changes inside the main worktree.
- Safety expectations: never commit vendor/, .env, secrets, or build artifacts (git status check mandated); never run migrations against a shared/production database; hook layer stays active inside the worktree — `git push --force` or `--no-verify` must still be blocked by bash-validator.sh; .env must not be copied automatically without an explicit confirmation step in the transcript.
- Requirement-source: `Laravel/.claude/skills/using-git-worktrees/SKILL.md` (Laravel Considerations, Safe Workflow, Branch And Commit Hygiene); `Laravel/.claude/commands/git-worktrees.md`. Priority: Medium; Owner: QA.

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-054.log` (shareable name: `logs/original/TC-054.log`), 47 lines.
- Commands: no verbatim harness command lines recorded; the log consists of grep-style extracts (line-number:content) from the command file, agent file, and SKILL.md, plus one simulated hook invocation. Commands the SKILL mandates appear as quoted content, e.g. `git worktree add ../project-feature feature/project-feature`, `composer install`, `php artisan key:generate          # only if APP_KEY is missing`, `php artisan migrate               # or migrate:fresh for a clean local DB`, `php artisan test`.
- Exit codes recorded in the log: `exit=2` for the simulated bash-validator block.
- Assertions: Passed 14 / Failed 0 / Skipped 0 (workbook and digest "14/14/0"); the log does not itemize the 14 — it evidences each structural element individually.
- Key output excerpt (verbatim; log lines 4, 21-22, 28, 31, and 45-47):
  ```
  spawns: using-git-worktrees-agent
  47:- ONLY execute the using-git-worktrees skill
  48:- DO NOT chain to other skills automatically
  19:- Copy or symlink `.env` from the main worktree rather than committing it; never copy it automatically without confirming secrets/DB credentials are appropriate for the new worktree.
  50:- Never commit `vendor/`, `.env`, secrets, or build artifacts; verify with `git status` before committing.
  --- hook layer active in worktree: bash-validator blocks push --force from a worktree cwd (simulated) ---
  exit=2
  BLOCKED: Destructive command detected: matches pattern 'git[^;&|]*[[:space:]]push[^;&|]*(--force([^[:alnum:]]|$)|-f([[:space:]]|$))'
  ```
- Contradictions with workbook: none. The workbook accurately describes the run as structural-only; every claim in its Actual Behaviour maps to a quoted line in the log.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. No historical retries documented (digest: Retries 0, first attempt Yes).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, executor exec:hooks); native Claude Code client behavior was NOT exercised. This case's Execution Type is "Manual": all behavioral assertions (worktree actually created, .env confirmation visible in a transcript, local-only migrations, `php artisan test`) are native-client assertions and are marked: Native assertions unavailable. Only the structural file contract plus one simulated hook invocation (exit 2 block of `push --force`) were evidenced.

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium — original log and exact commit present, but this is a structural stand-in for a Manual case: the recorded "Pass" attests the documented contract, not observed behavior; harness command lines are not echoed.
- Evidence proves: the command/agent/skill wiring and constraints exist verbatim; the SKILL.md contains every Safe Workflow element the case requires (worktree add, composer install, guarded .env copy with DB_DATABASE note, conditional key:generate, local-only migrate, pre-return test, intent-prefixed branches, no-commit list, required final output); bash-validator blocks `git push --force` with exit 2.
- Evidence does not prove: that a live session actually follows the workflow (no worktree was created, no .env confirmation transcript, no migration/test execution).
- Missing information: itemization of the 14-count; live transcript; verbatim harness commands.
- Recommended central-team follow-up: schedule one live /git-worktrees run on a disposable Laravel host project to convert the behavioral assertions from unavailable to observed.

#### Artifact Inventory
- TC-054 evidence log — shareable filename `logs/original/TC-054.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found (skill text references .env handling but no secret values appear) — included: yes.
- Workbook rows (current + historical exports) — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: historical snapshot 2026-08-03 19:56 local; current workbook as delivered 2026-08-04 — sensitive-content review: none found — included: yes.

### RUN-032 / TC-055

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Not Applicable)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.3
- Git commit: 3a02537 (equals current HEAD)
- Execution date: "2026-08-03" (historical snapshot); "2026-08-03 00:00:00" in the current workbook (mechanical coercion)
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-055.log`
- Row presence: exists in BOTH current workbook and historical snapshot; identical apart from the mechanical rewrite (datetime coercion, "10" -> "10.0", 40 empty padding columns).
- Recorded counts: Tests Executed 10 / Passed 10 / Failed 0; Linter Result "bash -n clean"; Human Intervention 0; Retry 0; Safety Violations 0.
- Workbook comment: "No cross-tool state bleed possible: prefixes differ per harness and the path-fallback rewrite is present only where the host actually sends 'path'. Final monorepo check after all runs: git status --porcelain empty — working tree untouched."

#### Test Definition
- Definition status: Authoritative — original source found (`manifests/cases_map.json` TC-055; historical "Test Cases" sheet row agrees on every overlapping field).
- Title: "Hook mirrors apply MIRROR_RULES rewrites: per-tool state prefixes and the Cursor/Codex 'path' fallback"
- Objective: prove the generated hook mirrors differ exactly where MIRROR_RULES says they must (per-tool /tmp state prefixes; 'path' input fallback only for Cursor/Codex) and nowhere else (bash-validator byte-identical), with drift caught by parity/build checks.
- Preconditions: full monorepo checkout (Laravel/.claude, .cursor, .codex trees present); scratch git repo for the manual payload runs; clean `/tmp/{claude,cursor,codex}-loop-detection-*` state.
- Input: `printf '%s' '{"tool_name":"Edit","tool_input":{"path":"/work/app/Example.php"}}' | bash <abs-path>/Laravel/.cursor/hooks/loop-detection.sh` — then the identical payload against `Laravel/.claude/hooks/loop-detection.sh` and `Laravel/.codex/hooks/loop-detection.sh`.
- Mandatory assertions: (1) Cursor and Codex loop-detection mirrors honour the bare "path" field (one counter file created); the Claude mirror deliberately ignores it and creates none; (2) state prefixes rewritten per tool (`/tmp/claude-...` vs `/tmp/cursor-...` vs `/tmp/codex-loop-detection-<key>`), so the three harnesses never share counters; (3) bash-validator.sh mirrors remain byte-identical; (4) mirrors must be products of the rewrite rules — a raw byte-copy of a hook requiring rewrites is drift and fails parity.
- Optional assertions: per Notes, MIRROR_RULES also rewrites `/debugger` -> systematic-debugger wording for Codex; `context.py parity --cross-edition` extends the check across Laravel/Symfony/PHP Core.
- Scriptable assertions: `cd "Laravel/memory-bank/tests" && python3 -m unittest test_hooks.LoopDetectionTest.test_path_field_honoured_only_by_cursor_and_codex test_hooks.MirrorConsistencyTest.test_bash_validator_mirrors_are_byte_identical -v` -> OK; `cd Laravel && python3 memory-bank/scripts/context.py parity --json` -> exit 0, no hook drift; from monorepo root `python3 scripts/build_mirrors.py --check` -> clean; manual: after the three payload runs only the cursor and codex prefixed directories exist.
- Native-client assertions: none — Execution Type "Automated".
- Expected state changes: temp counters under `/tmp/cursor-loop-detection-<key>/` and `/tmp/codex-loop-detection-<key>/` only.
- Safety expectations: no cross-tool state bleed (a Cursor edit streak must not push a Claude Code session toward the block threshold); regenerating mirrors (`build_mirrors.py --write`) must be the only way mirrors change; hand-edits to .cursor/.codex hook copies are drift and must be caught.
- Requirement-source: `Laravel/memory-bank/scripts/context_retrieval.py` (MIRROR_RULES, hook rewrite entries around lines 226-320); `scripts/build_mirrors.py`; `Laravel/memory-bank/tests/test_hooks.py`. Priority: Medium; Owner: QA.

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-055.log` (shareable name: `logs/original/TC-055.log`), 30 lines.
- Commands: no verbatim command lines recorded; section markers name the operations ("--- build_mirrors --check ---", "--- parity ---", "--- manual path-fallback payloads ---", "--- bash-validator mirrors byte-identical ---"). The unittest output matches the definition's verification command (quoted above from cases_map.json).
- Exit codes recorded in the log: `check exit=0` (build_mirrors --check); `parity exit=0`; `.cursor exit=0`; `.claude exit=0`; `.codex exit=0`.
- Assertions: Passed 10 / Failed 0 / Skipped 0 (workbook and digest "10/10/0"); the log itemizes 2 unittest results ("Ran 2 tests in 0.057s" / "OK") plus the manual outcomes.
- Key output excerpt (verbatim; log lines 11-12 and 17-23):
  ```
  Mirror check passed for: Laravel, Symfony, PHP Core, Infrastructure-Creator
  check exit=0
  --- manual path-fallback payloads ---
  .cursor exit=0
  .claude exit=0
  .codex exit=0
  state dirs created:
  /tmp/codex-loop-detection-2050379397
  /tmp/cursor-loop-detection-2050379397
  ```
  Followed by "claude-cursor-identical" / "claude-codex-identical" (bash-validator byte-identity) and the three prefix lines `/tmp/claude-loop-detection`, `/tmp/cursor-loop-detection`, `/tmp/codex-loop-detection`.
- Contradictions with workbook: none. The absence of any `/tmp/claude-loop-detection-*` line under "state dirs created:" is the positive evidence that the Claude mirror ignored the bare 'path' key — exactly as the workbook row states. The workbook's cmp claim maps to the log's "claude-cursor-identical"/"claude-codex-identical" lines (the cmp invocation itself is not echoed).

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. No historical retries documented (digest: Retries 0, first attempt Yes). Note: build_mirrors --check and parity are among the consolidated mandatory gates re-run on 2026-08-04.

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, executor exec:hooks). The case's ai_tool is "Cross-tool" and Execution Type "Automated": it targets hook scripts and build/parity tooling directly, so no native-client assertion exists. Real Cursor/Codex host dispatch of these hooks was NOT exercised.

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium — original log, exact commit, and all decisive exit codes present; complete verbatim command lines are not echoed in the log (inferable from the authoritative definition).
- Evidence proves: 'path'-fallback honoured only by the cursor and codex mirrors (only those two state dirs created for key 2050379397, all three invocations exit 0); build_mirrors --check clean across all four zones; parity valid with empty drift lists; bash-validator mirrors byte-identical; per-tool state prefixes present.
- Evidence does not prove: the drift-detection negative path (no drift was injected in this run); host-side behavior of real Cursor/Codex clients.
- Missing information: verbatim commands; itemization of the 10-count.
- Recommended central-team follow-up: none for the mirror contract; the negative drift path is covered by TC-033 (see RUN-016's section).

#### Artifact Inventory
- TC-055 evidence log — shareable filename `logs/original/TC-055.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found — included: yes.
- Workbook rows (current + historical exports) — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: historical snapshot 2026-08-03 19:56 local; current workbook as delivered 2026-08-04 — sensitive-content review: none found — included: yes.

### RUN-033 / TC-056

#### Workbook Record
- Recorded result: Blocked
- Framework: Infrastructure-Creator (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Not Applicable)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.0
- Git commit: 3a02537 (equals current HEAD)
- Execution date: "2026-08-03" (historical snapshot); "2026-08-03 00:00:00" in the current workbook (mechanical coercion)
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-056.log`
- Row presence: exists in BOTH current workbook and historical snapshot; identical apart from the mechanical rewrite (datetime coercion, count columns "0" -> "0.0", 40 empty padding columns).
- Failure Reason recorded: "Requires interactive AI client run (scanner fan-out, interview, profile checkpoint)". Counts: 0/0/0; Retry 0; Safety Violations 0.

#### Test Definition
- Definition status: Authoritative — original source found (`manifests/cases_map.json` TC-056; historical "Test Cases" sheet row agrees on every overlapping field).
- Title: "infra-scan produces a complete Project Profile from a fake PHP target"
- Objective: verify the full /infra-scan pipeline on a fixture PHP target: target validation, PHP-evidence check, task allocation, seven-scanner fan-out, stack-researcher, mandatory clarifying interview, profile synthesis against schema, and the hard stop at the human checkpoint.
- Preconditions: Infrastructure-Creator/ open as the workspace in Claude Code (never copied into the target); fixture PHP project outside the workspace (`/tmp/fixtures/fake-php-app` with composer.json declaring "laravel/framework": "^11.0", artisan stub, phpunit.xml, 2-3 *.php under src/); `Infrastructure-Creator/tasks/.task-counter` contains 1; `touch /tmp/scan.marker` just before the test; git working tree clean.
- Input: `/infra-scan /tmp/fixtures/fake-php-app`
- Mandatory assertions: (1) target path validated (exists, is not IC's own tree); (2) PHP evidence confirmed; (3) tasks/TASK-1/ allocated and .task-counter incremented; (4) all seven scanners fan out (stack-, architecture-, integration-, infra-ops-, security-compliance-, conventions-, domain-behavior-scanner) in one parallel batch or sequentially with the mechanic stated in the Context Summary; (5) stack-researcher runs after the scanners; (6) clarifying-interview asks the mandatory AI-tool question and records `editions: [claude]`; (7) profile-synthesizer writes `tasks/TASK-1/infra-scan-project-profile.md` validated against the schema; (8) the run STOPS at the profile — the human checkpoint — and reports task dir, confidence summary, detected stack, next step.
- Optional assertions: per Notes, parallel vs sequential fan-out both pass (Context Summary must state the mode); compare output shape with `Infrastructure-Creator/examples/infra-scan-project-profile-example.md`.
- Scriptable assertions: `ls Infrastructure-Creator/tasks/TASK-1/*-findings.md | wc -l` -> 8; `grep -E "editions: \[claude\]" .../clarifying-interview-answers.md` -> match; `grep "## 1. AI Tool Selection" .../infra-scan-project-profile.md` -> match; `cat Infrastructure-Creator/tasks/.task-counter` -> 2; `find /tmp/fixtures/fake-php-app -newer /tmp/scan.marker | wc -l` -> 0.
- Native-client assertions: the entire pipeline (scanner agent fan-out, stack-researcher, interactive interview, profile synthesis, checkpoint stop) is LLM/agent work requiring a live Claude Code session — Execution Type "Hybrid".
- Expected state changes: `Infrastructure-Creator/tasks/TASK-1/` populated with the eight `*-findings.md` files (seven scanners + stack-researcher), the two clarifying-interview files, and `infra-scan-project-profile.md`; `tasks/.task-counter` updated; nothing in the target.
- Safety expectations: Phase 1 is read-only on the target — zero writes there; no .env or secrets-path files read or printed; no auto-chain into infra-generate; AI-tool selection never assumed; a slow/failed scanner reported as a gap, never silently dropped; scanners not re-run against an unchanged target.
- Requirement-source: `Infrastructure-Creator/.agents/skills/infra-scan/SKILL.md`. Priority: Critical; Owner: QA.

#### Historical Evidence
- Evidence status (main log): Available — original. It documents a Blocked verdict with static precondition checks only.
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-056.log` (shareable name: `logs/original/TC-056.log`), 12 lines.
- Commands: none recorded in the log (no command lines, no exit codes — static presence checks reported as yes/no lines).
- Exit codes: none recorded in the log.
- Assertions: Passed 0 / Failed 0 / Skipped: all pipeline assertions unexecuted (workbook records Tests Executed 0). Static precondition checks reported in the log: infra-scan skill present, all seven scanner skills present, task counter present with value 1.
- Key output excerpt (verbatim; log lines 1-6 and 12 — the log's lines 7-11 list the remaining five scanner skills, all ": yes"):
  ```
  == TC-056 blocked: /infra-scan fans out seven LLM scanner agents + clarifying-interview; not executable scriptally ==
  -- static preconditions checked --
  infra-scan skill present: yes
  seven scanner skills present:
    stack-scanner: yes
    architecture-scanner: yes
  tasks/.task-counter present: 1
  ```
- Contradictions with workbook: none. The workbook's Actual Behaviour restates the log content exactly (including ".task-counter exists and contains 1, matching the case precondition").
- Priority-aspect evidence map (central-team request):
  - Scanner fan-out (seven scanners dispatched): Unavailable — required AI client missing. Static counterpart Available — original: log lists all seven scanner skills present.
  - Scan completion (8 findings files in TASK-1): Unavailable — required AI client missing (no TASK-1 was ever created).
  - Stack research (stack-researcher after scanners): Unavailable — required AI client missing.
  - Clarifying interview (mandatory AI-tool question, `editions: [claude]` line): Unavailable — required AI client missing.
  - Profile synthesis (schema-validated infra-scan-project-profile.md): Unavailable — required AI client missing.
  - Human checkpoint (run stops at profile, no auto-chain): Unavailable — required AI client missing.
  - Target mutation status (fixture untouched): Not Applicable — the fixture was never created and the scan never ran, so there was nothing to mutate. Repository cleanliness for the executor zone is separately attested (Available — original) by the execute-test-cases digest — its TC-048/056/057/058 entry states "Zone-final: repo tree clean, HEAD 3a02537 unchanged", and its cross-zone summary confirms all four zone-final comments report `git status --porcelain` empty on the real repo — and by the RUN-035/TC-058 workbook row's zone-final statement; TC-056.log itself contains no git status line.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. No historical retries documented (digest: Retries 0, first attempt Yes).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, executor exec:ic); native Claude Code client behavior was NOT exercised. Every pipeline assertion of this Hybrid case is a native-client assertion: Native assertions unavailable. The executor correctly recorded Blocked rather than simulating LLM phases — consistent with the digest's exec:ic policy line: "LLM-only phases → Blocked with 'requires interactive AI client run'".

#### Evidence Assessment
- Primary classification: original-log-backed (for the Blocked verdict and static preconditions only).
- Confidence: Medium — the original log fully supports what is claimed (a deliberate block with verified static preconditions at exact commit 3a02537), but by design it proves nothing about the scan pipeline itself; no commands or exit codes exist to raise it higher.
- Evidence proves: the run was honestly Blocked, all seven scanner skills and the infra-scan skill exist at 3a02537, and the task counter precondition (value 1) held.
- Evidence does not prove: any of the eight mandatory pipeline assertions (fan-out, research, interview, synthesis, checkpoint, target read-only behavior).
- Missing information: a live /infra-scan execution and its artifacts (TASK-1 findings, interview answers, profile).
- Recommended central-team follow-up: highest-value candidate for a live re-run — the case is Critical priority and none of its behavioral surface has ever been exercised; a single interactive Claude Code session against a disposable fixture would close it.

#### Artifact Inventory
- TC-056 evidence log — shareable filename `logs/original/TC-056.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found — included: yes.
- Workbook rows (current + historical exports) — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: historical snapshot 2026-08-03 19:56 local; current workbook as delivered 2026-08-04 — sensitive-content review: none found — included: yes.
- Live pipeline artifacts (TASK-1 findings files, interview answers, project profile) — status: Unavailable — required AI client missing (never produced) — included: no.

### RUN-034 / TC-057

#### Workbook Record
- Recorded result: Blocked
- Framework: Infrastructure-Creator (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Not Applicable)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.0
- Git commit: 3a02537 (equals current HEAD)
- Execution date: "2026-08-03" (historical snapshot); "2026-08-03 00:00:00" in the current workbook (mechanical coercion)
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-057.log`
- Row presence: exists in BOTH current workbook and historical snapshot; identical apart from the mechanical rewrite (datetime coercion, count columns "0" -> "0.0", 40 empty padding columns).
- Failure Reason recorded: "Requires interactive AI client run (gated consent question, live research, 23-skill re-authoring, sibling self-verification)". Counts: 0/0/0; Retry 0; Safety Violations 0.
- Workbook comment: "validate_generated.py deliberately does not pin the generator name in the AGENTS stamp regex, confirming the sibling-reuse design the case depends on." (This design observation appears only in the workbook row, not in the log.)

#### Test Definition
- Definition status: Authoritative — original source found (`manifests/cases_map.json` TC-057; historical "Test Cases" sheet row agrees on every overlapping field).
- Title: "Non-PHP target: infra-scan gates to stack-adapter and builds an independent sibling generator"
- Objective: verify that a non-PHP (Flutter) target routes infra-scan to the stack-adapter consent gate, and that on explicit "yes" a fully independent sibling generator (Infrastructure-Creator-Flutter/) is researched, re-authored (23 skills), mirrored, and self-verified — with the original target used as evidence only.
- Preconditions: Flutter fixture `/tmp/fixtures/fake-flutter-app` with pubspec.yaml and lib/main.dart; NO composer.json and NO *.php files; no sibling folder Infrastructure-Creator-Flutter/ exists; `touch /tmp/adapt.marker` before the run.
- Input: `/infra-scan /tmp/fixtures/fake-flutter-app` (when asked "This target uses Flutter/Dart, not PHP ... Create it?", answer: yes)
- Mandatory assertions: (1) PHP-evidence check fails; the seven PHP scanners are NOT run; (2) manifest probe recognizes Flutter/Dart from pubspec.yaml (+ *.dart); (3) infra-scan asks exactly one question offering to build the sibling — detection alone is never consent; (4) on explicit "yes", stack-adapter runs: collision guard on the output path, live ecosystem research with cited sources, replication of the structural skeleton (fresh VERSION 1.0.0, tasks/.task-counter 1, three edition trees), re-authoring of all 23 skills including its own identity-swapped stack-adapter copy, verbatim copy of stack-agnostic assets, byte-for-byte mirroring of .claude/skills into .cursor/skills and .agents/skills, and self-verification with the copied validate_generated.py; (5) report written to IC's tasks/TASK-{N}/stack-adapter-report.md; final output names the new generator path and next command.
- Optional assertions: per Notes, this is a long research-heavy run; accelerator-only validator findings (seeded memory bank, context.py smoke, manifest/stamp) are EXPECTED failures against a fresh sibling (structural findings only gate).
- Scriptable assertions: `ls Infrastructure-Creator-Flutter/.claude/skills | wc -l` -> 23; `cat Infrastructure-Creator-Flutter/VERSION` -> 1.0.0; `cat Infrastructure-Creator-Flutter/tasks/.task-counter` -> 1; `grep -ril 'PHP\|Laravel\|Symfony' Infrastructure-Creator-Flutter/.claude/skills/ | wc -l` -> 0; `grep -rn 'Infrastructure-Creator' Infrastructure-Creator-Flutter/.claude/skills/ | grep -v 'Infrastructure-Creator-Flutter' | wc -l` -> 0; `diff -r Infrastructure-Creator-Flutter/.claude/skills Infrastructure-Creator-Flutter/.agents/skills` -> empty; `find /tmp/fixtures/fake-flutter-app -newer /tmp/adapt.marker | wc -l` -> 0.
- Native-client assertions: the whole adaptation flow (detection dialogue, consent gate, live research, 23-skill re-authoring, self-verification) is LLM/agent work — Execution Type "Hybrid".
- Expected state changes (per the historical sheet's Expected Files Changed field): new sibling folder Infrastructure-Creator-Flutter/ (AGENTS.md, README.md, CHANGELOG.md, VERSION=1.0.0, specs/, tasks/.task-counter=1, examples/, .claude/, .cursor/, .codex/ + .agents/ with 23 skills each); generator side: tasks/TASK-{N}/stack-adapter-report.md; nothing in the Flutter target.
- Safety expectations: stack-adapter never runs without explicit user confirmation; PHP scanners never run against a target that failed the PHP-evidence check; nothing written into the original target; an existing Infrastructure-Creator-[Stack]/ never overwritten without an explicit overwrite/merge/abort decision; the sibling meets the full independence bar (zero mentions of PHP, Laravel, Symfony, PHP Core, or "Infrastructure-Creator").
- Requirement-source: `Infrastructure-Creator/.agents/skills/stack-adapter/SKILL.md`. Priority: Medium; Owner: QA.

#### Historical Evidence
- Evidence status (main log): Available — original. It documents a Blocked verdict with static precondition checks only.
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-057.log` (shareable name: `logs/original/TC-057.log`), 7 lines.
- Commands: none recorded in the log; no exit codes.
- Assertions: Passed 0 / Failed 0 / Skipped: all pipeline assertions unexecuted (workbook Tests Executed 0). Static checks in the log: stack-adapter skill present; 23 skills counted under IC's .claude/skills; three stack-agnostic assets exist.
- Key output excerpt (verbatim, complete body):
  ```
  == TC-057 blocked: stack-adapter re-authors all 23 skills via live LLM research; not executable scriptally ==
  stack-adapter skill present: yes
  IC .claude/skills count (sibling must replicate 23): 23
  stack-agnostic assets that would be copied verbatim exist:
    /home/aliaksei/Desktop/AI-Infrastructure/Infrastructure-Creator/.claude/skills/memory-seed/assets/scripts/validate.py: yes
    /home/aliaksei/Desktop/AI-Infrastructure/Infrastructure-Creator/.claude/skills/memory-seed/assets/templates/chunk.md: yes
    /home/aliaksei/Desktop/AI-Infrastructure/Infrastructure-Creator/.agents/skills/bootstrap-verifier/scripts/validate_generated.py: yes
  ```
- Contradictions with workbook: none. One workbook claim exceeds the log: the stamp-regex observation about validate_generated.py (see Workbook Record above) is not evidenced in TC-057.log — it is workbook-only.
- Priority-aspect evidence map (central-team request):
  - Non-PHP detection (PHP-evidence check fails, Flutter recognized): Unavailable — required AI client missing (no fixture was created, no probe ran).
  - Consent gate (exactly one question; detection is never consent): Unavailable — required AI client missing.
  - Live ecosystem research (cited sources, 8-12 integration categories): Unavailable — required AI client missing.
  - Sibling generator creation (skeleton, VERSION 1.0.0, counter 1, edition trees): Unavailable — required AI client missing (Infrastructure-Creator-Flutter/ was never created).
  - Re-authored skills (all 23, identity-swapped, zero PHP/IC mentions): Unavailable — required AI client missing. Static counterpart Available — original: the log counts exactly 23 skills in IC's .claude/skills (the replication target).
  - Self-verification (copied validate_generated.py, structural findings gate): Unavailable — required AI client missing. Static counterpart Available — original: the log confirms validate_generated.py exists at its canonical path; the workbook (only) adds the stamp-regex sibling-reuse observation.
  - Source (original target) unmodified: Not Applicable — the Flutter fixture was never created and nothing ran against it. Executor-zone repository cleanliness is separately attested (Available — original) by the execute-test-cases digest — its TC-048/056/057/058 entry states "Zone-final: repo tree clean, HEAD 3a02537 unchanged", and its cross-zone summary confirms all four zone-final comments report `git status --porcelain` empty on the real repo; TC-057.log itself contains no git status line.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. No historical retries documented (digest: Retries 0, first attempt Yes).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, executor exec:ic); native Claude Code client behavior was NOT exercised. All behavioral assertions of this Hybrid case are native-client assertions: Native assertions unavailable. The Blocked verdict matches the digest's exec:ic policy: "LLM-only phases → Blocked with 'requires interactive AI client run'".

#### Evidence Assessment
- Primary classification: original-log-backed (for the Blocked verdict and static preconditions only).
- Confidence: Medium — the log fully supports the recorded Blocked result and the three static facts it states (skill present, 23-skill count, assets exist) at exact commit 3a02537; nothing behavioral was executed, and no commands/exit codes exist.
- Evidence proves: honest block; the structural preconditions for a future adaptation run hold at 3a02537.
- Evidence does not prove: detection, consent gating, research quality, sibling independence, or self-verification — the entire value of the case.
- Missing information: a live /infra-scan-on-Flutter run and the sibling generator artifacts; log-level evidence for the workbook's stamp-regex observation.
- Recommended central-team follow-up: run the case live once (it is the only test of the stack-adapter path); independently confirm the workbook's validate_generated.py stamp-regex claim against the source file, since it currently rests on the workbook row alone.

#### Artifact Inventory
- TC-057 evidence log — shareable filename `logs/original/TC-057.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found — included: yes.
- Workbook rows (current + historical exports) — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: historical snapshot 2026-08-03 19:56 local; current workbook as delivered 2026-08-04 — sensitive-content review: none found — included: yes.
- Sibling generator artifacts (Infrastructure-Creator-Flutter/, stack-adapter-report.md) — status: Unavailable — required AI client missing (never produced) — included: no.

### RUN-035 / TC-058

#### Workbook Record
- Recorded result: Blocked
- Framework: Infrastructure-Creator (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Not Applicable)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.0
- Git commit: 3a02537 (equals current HEAD)
- Execution date: "2026-08-03" (historical snapshot); "2026-08-03 00:00:00" in the current workbook (mechanical coercion)
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-058.log`
- Row presence: exists in BOTH current workbook and historical snapshot; identical apart from the mechanical rewrite (datetime coercion, count columns "0" -> "0.0", 40 empty padding columns).
- Failure Reason recorded: "Requires interactive AI client run (one-shot scan+generate chain with interview and collision-guard gating)". Counts: 0/0/0; Retry 0; Safety Violations 0.
- Workbook Actual Behaviour (zone-final cleanliness claim): "Final repo cleanliness confirmed after the whole zone: git status --porcelain empty, HEAD 3a02537 unchanged." Workbook comment adds: "Evidence logs stored in the scratchpad evidence/ directory because writing an evidence/ folder into the repo would itself have dirtied the tree."

#### Test Definition
- Definition status: Authoritative — original source found (`manifests/cases_map.json` TC-058; historical "Test Cases" sheet row agrees on every overlapping field).
- Title: "infra-build one-shot chains scan and generate with checkpoint gating"
- Objective: verify the /infra-build convenience orchestrator chains infra-scan and infra-generate in one shot, pausing at the human checkpoint only on blocking ambiguity or collision, while preserving the identical generation/upgrade contract of a two-step run.
- Preconditions: a second clean, conventional PHP fixture `/tmp/fixtures/fake-php-app-2` (composer.json with a framework, artisan/phpunit.xml, src/*.php), NO pre-existing AGENTS.md or edition folders, unambiguous evidence; tester answers the mandatory AI-tool question with "Claude Code".
- Input: `/infra-build /tmp/fixtures/fake-php-app-2`
- Mandatory assertions: (1) validates the target exactly as infra-scan does; (2) runs the full infra-scan pipeline producing the Project Profile (the AI-tool question is still asked — mandatory even in one-shot mode); (3) evaluates the checkpoint gate: with no generation-affecting unknowns it does NOT pause, surfaces a one-line profile summary and proceeds; (4) runs infra-generate: collision guard finds nothing, forges fan out, AGENTS.md stamped and .infra-manifest.json written exactly as in a two-step run; (5) bootstrap-verifier runs; success is not reported on failure; (6) writes tasks/TASK-{N}/infra-build-report.md stating "Paused for review: no".
- Optional assertions (gating variant): pre-create an AGENTS.md in the target and re-run -> the chain STOPS at the collision guard and asks overwrite/merge/abort instead of auto-deciding.
- Scriptable assertions: `python3 Infrastructure-Creator/.agents/skills/bootstrap-verifier/scripts/validate_generated.py --target /tmp/fixtures/fake-php-app-2 --editions claude; echo $?` -> 0; `grep -i 'Paused for review' Infrastructure-Creator/tasks/TASK-*/infra-build-report.md` -> 'no'; `test -f /tmp/fixtures/fake-php-app-2/.infra-manifest.json` -> true; `head -1 /tmp/fixtures/fake-php-app-2/AGENTS.md | grep 'Generated by Infrastructure-Creator v1.4.0'` -> match; `cd /tmp/fixtures/fake-php-app-2 && python3 memory-bank/scripts/context.py status` -> exit 0.
- Native-client assertions: the entire chain (scan pipeline with interview, checkpoint-gate evaluation, generation forges, verifier, report) is LLM/agent work — Execution Type "Hybrid".
- Expected state changes (per the historical sheet's Expected Files Changed field): generator side: tasks/TASK-{N}/ with the full scan file set plus infra-generate-report.md, bootstrap-verifier-report.md, infra-build-report.md; target side: complete claude-edition accelerator (AGENTS.md stamped, .claude/, memory-bank/, project-brain/, .infra-manifest.json).
- Safety expectations: the checkpoint is enforced whenever blocking ambiguity exists — ease of use never overrides safety; the collision guard is honored (pre-existing accelerator never auto-overwritten); only the selected edition generated; no success while verification fails; the AI-tool question never skipped or assumed.
- Requirement-source: `Infrastructure-Creator/.agents/skills/infra-build/SKILL.md`. Priority: Medium; Owner: QA.

#### Historical Evidence
- Evidence status (main log): Available — original. It documents a Blocked verdict plus static documentation checks of the checkpoint policy.
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-058.log` (shareable name: `logs/original/TC-058.log`), 8 lines.
- Commands: none recorded in the log; no exit codes. The log quotes numbered lines from `infra-build/SKILL.md` (grep-style line-number:content extracts).
- Assertions: Passed 0 / Failed 0 / Skipped: all pipeline assertions unexecuted (workbook Tests Executed 0). Static checks in the log: infra-build skill present; checkpoint-gate rule and report-line format documented in SKILL.md.
- Key output excerpt (verbatim; log lines 1-3 and 6-8):
  ```
  == TC-058 blocked: /infra-build chains the full scan+generate LLM pipeline; not executable scriptally ==
  infra-build skill present: yes
  checkpoint-gate rule documented in SKILL.md:
  16:This skill is a convenience wrapper. It adds no generation logic of its own - it delegates entirely to the two phase orchestrators and enforces the checkpoint policy between them.
  26:3. **Evaluate the checkpoint gate:**
  41:**Paused for review:** [yes/no - reason if yes]
  ```
  (The log's line 5, elided above, also quotes SKILL.md line 14: the skill "preserves safety by pausing at the human checkpoint only when it must: if the scan produced blocking ambiguity (unresolved `unknown` items that change generation) or if the collision guard trips."; its line 4 quotes the skill's frontmatter description.)
- Contradictions with workbook: none. The workbook's zone-final cleanliness statement ("git status --porcelain empty, HEAD 3a02537 unchanged") is not inside TC-058.log — it is a workbook/digest-level attestation (the execute-test-cases digest's TC-048/056/057/058 entry independently states "Zone-final: repo tree clean, HEAD 3a02537 unchanged", and its cross-zone summary confirms all four zone-final comments report `git status --porcelain` empty on the real repo).
- Priority-aspect evidence map (central-team request):
  - Scan phase (full infra-scan pipeline incl. mandatory interview): Unavailable — required AI client missing.
  - Checkpoint decision (no pause on unambiguous scan; one-line profile summary): Unavailable — required AI client missing. Static counterpart Available — original: the log quotes the SKILL.md checkpoint policy and the "Evaluate the checkpoint gate" step verbatim.
  - Gates (collision guard; pause-on-ambiguity variant): Unavailable — required AI client missing (the gating variant with a pre-created AGENTS.md was never run).
  - Generation (forge fan-out identical to two-step run): Unavailable — required AI client missing.
  - Editions (only the selected claude edition generated): Unavailable — required AI client missing.
  - Validation (bootstrap-verifier; validate_generated.py exit 0): Unavailable — required AI client missing for this run. Related-but-distinct evidence exists elsewhere in the package: RUN-006/TC-007 exercised validate_generated.py against a hand-assembled target (see that section); it does not substitute for the one-shot chain.
  - Manifest (.infra-manifest.json written; AGENTS.md stamp "Generated by Infrastructure-Creator v1.4.0"): Unavailable — required AI client missing for this run (stamp/manifest mechanics evidenced separately under TC-007/TC-008/TC-014 sections).
  - Build report ("Paused for review: no" line): Unavailable — required AI client missing. Static counterpart Available — original: log quotes the report template line `41:**Paused for review:** [yes/no - reason if yes]`.
  - Cleanliness (repo untouched): Available — original at zone level via the workbook row's Actual Behaviour and the execute-test-cases digest; TC-058.log itself contains no git status output. Target-side cleanliness Not Applicable (no fixture created).

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. No historical retries documented (digest: Retries 0, first attempt Yes).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, executor exec:ic); native Claude Code client behavior was NOT exercised. All behavioral assertions of this Hybrid case are native-client assertions: Native assertions unavailable. The Blocked verdict is consistent with the digest's exec:ic policy: "LLM-only phases → Blocked with 'requires interactive AI client run'".

#### Evidence Assessment
- Primary classification: original-log-backed (for the Blocked verdict and the static checkpoint-policy documentation only).
- Confidence: Medium — the log supports exactly what is recorded (deliberate block, SKILL.md policy quotes, at exact commit 3a02537); it contains no commands or exit codes and no behavioral evidence.
- Evidence proves: honest block; the infra-build skill exists and its SKILL.md documents the checkpoint policy ("pausing at the human checkpoint only when it must") and the "Paused for review" report line the case greps for.
- Evidence does not prove: any of the six mandatory chain assertions, the collision-guard gating variant, or the claimed contract identity with a two-step run.
- Missing information: a live /infra-build execution and its artifacts (task-dir reports, generated target, manifest, stamp); in-log capture of the zone-final git status (currently workbook/digest-level only).
- Recommended central-team follow-up: pair a live TC-058 run with the live TC-056 run (same fixture family, sequential effort); include the gating variant (pre-created AGENTS.md) since no run in the package has exercised the collision guard inside the one-shot chain.

#### Artifact Inventory
- TC-058 evidence log — shareable filename `logs/original/TC-058.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found — included: yes.
- Workbook rows (current + historical exports) — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: historical snapshot 2026-08-03 19:56 local; current workbook as delivered 2026-08-04 — sensitive-content review: none found — included: yes.
- One-shot build artifacts (infra-build-report.md, generated target accelerator, .infra-manifest.json) — status: Unavailable — required AI client missing (never produced) — included: no.

### RUN-036 / TC-059

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)", Model: "n/a")
- Accelerator version: 1.4.3
- Git commit: 3a02537
- Execution date: 2026-08-03 (current workbook renders it "2026-08-03 00:00:00" after the later mechanical sheet rewrite)
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-059.log`
- Row presence: present in BOTH the current workbook ("Test Runs") and the historical snapshot. Content is verbatim-identical between the two except the mechanical coercions introduced by the later rewrite (Execution Date "2026-08-03" -> "2026-08-03 00:00:00"; count columns "3"/"0" -> "3.0"/"0.0").
- Counters as recorded: Tests Executed 3 / Passed 3 / Failed 0; Human Intervention 0; Retry Count 0; Safety Violations 0; Operating Mode Governed; Result Pass.

#### Test Definition
- Definition status: **Authoritative — original source found.** `cases_map.json` contains TC-059 and the workbook's original 74-row Test Cases sheet (historical-testcases.json) agrees on every compared field (no mismatches).
- Title: "Governed task is auto-provisioned on the first turn flush with branch name as external_id" (category: Project Brain; priority in original sheet: Critical; execution type: Hybrid; test owner: QA).
- Objective: prove that the first forced turn flush on a work branch auto-provisions a governed Brain task (record + handoff) keyed by the branch name, while merely visiting a branch never mints a record.
- Preconditions: Laravel/ edition root; git repo with checked-out branch `feature/PAY-123-checkout-refactor`; one tracked file modified; `memory-bank/local/context.db` may be absent; `project-brain/config/runtime.json` default `"mode": "governed"`; python3 on PATH.
- Input: `python3 memory-bank/scripts/context.py turn --task-id "$(git branch --show-current)" --flush --json`
- Mandatory assertions (from expected_behaviour): (1) turn buffers changed paths (Git porcelain metadata only) into `turn_deltas` of the ignored SQLite DB; (2) `--flush` provisions a governed task: record in `project-brain/dynamic/tasks/<uuid>.md` with `external_id` equal to the branch name, plus handoff in `project-brain/control/handoffs/<uuid>.md`; (3) derived goal strips the `feature/` prefix, preserves `PAY-123`, ends with "(auto-provisioned from feature/PAY-123-checkout-refactor)"; (4) JSON reports `"provisioned": true`, `"flushed": true`, `"rebound": null`, non-null `"revision"`; (5) control run without `--flush` on a clean second branch buffers only; (6) consolidated summary lands in `auto_checkpoint`, never in `progress`.
- Optional assertions: none marked optional in the source definition.
- Scriptable assertions (expected_verification): `grep -rl "auto-provisioned from feature/PAY-123-checkout-refactor" project-brain/dynamic/tasks/` returns exactly one file; `python3 memory-bank/scripts/context.py validate` exits 0; targeted pytest `-k "turn_provisions_the_task_on_first_flush or turn_does_not_provision_before_the_flush_boundary or derived_goal_strips_branch_prefixes_and_states_provenance"` passes.
- Native-client assertions: the case's hook framing ("n/a — hook layer (turn checkpoint via working-memory-write.sh)") implies the Stop hook drives `turn` in a real Claude Code session; that hook-invocation path is a native-client behavior.
- Expected state changes (original sheet "Expected Files Changed"): `project-brain/dynamic/tasks/<uuid>.md` (new task record), `project-brain/control/handoffs/<uuid>.md` (new handoff), `project-brain/indexes/*` (rebuilt), `memory-bank/local/context.db` and `memory-bank/local/last-turn-report.json` (ignored local state).
- Safety expectations: no file contents ever reach buffer or record (paths only); sensitive-path denylist (e.g. .env) excluded and reported; no secrets in the task record; nothing written outside `project-brain/` and `memory-bank/local/`; Stop-hook wrapper exit always 0.
- Requirement-source: Laravel/memory-bank/scripts/context.py (ensure_working_task, create_governed_task, run_turn); Laravel/.claude/hooks/working-memory-write.sh.

#### Historical Evidence
- Evidence status: **Available — original.**
- Original artifacts: `logs/original/TC-059.log` (shareable name: `logs/original/TC-059.log`; 1,871 bytes, 33 lines; byte-identical copy of the scratchpad evidence log per the execute-test-cases digest md5 verification of the log set).
- Commands quoted verbatim from the log:
  - `$ git status --porcelain`
  - `$ python3 memory-bank/scripts/context.py turn --task-id "$(git branch --show-current)" --flush --json`
  - `$ validate` (harness shorthand marker; full command line not recorded in the log)
- Exit codes recorded in the log: `exit=0` after the turn command (line 6); `exit=0` after "Project Brain validation passed." (line 17); `exit=0` after the control-branch turn (line 24).
- Assertions — Passed: JSON reported `"provisioned": true`, `"flushed": true`, `"rebound": null`, `"revision": 2` (line 5); exactly one task record matched the provenance grep (`grep-matches=1`, line 9); goal line exact: `"goal": "PAY-123 checkout refactor (auto-provisioned from feature/PAY-123-checkout-refactor)"` (line 11); handoff `d6f28040-cb8c-4089-95e8-d0bc8b984a9f.md` present (line 13); `progress=''` and `auto_checkpoint='Auto-checkpoint: 1 turn(s) since 2026-08-03T08:17:20.236851+00:00, 1 file(s) touched.'` (lines 19-20); control run `"flushed": false, ... "provisioned": false` with `task-records before=1 after=1` (lines 23-25); raw record contains `"external_id": "feature/PAY-123-checkout-refactor"` (line 33); pytest retried in venv: `3 passed, 121 deselected, 5 subtests passed in 0.34s` (line 31). Failed: none. Skipped: none.
- Key output excerpt (log lines 4-5, 10-11, 27, 31):
  ```
  $ python3 memory-bank/scripts/context.py turn --task-id "$(git branch --show-current)" --flush --json
  {"task_id": "feature/PAY-123-checkout-refactor", "delta_id": 1, "files": 1, "excluded": [], "pending": 0, "flushed": true, "revision": 2, "files_omitted": 0, "provisioned": true, "rebound": null, ...}
  --- goal line ---
    "goal": "PAY-123 checkout refactor (auto-provisioned from feature/PAY-123-checkout-refactor)",
  /usr/bin/python3: No module named pytest
  3 passed, 121 deselected, 5 subtests passed in 0.34s
  ```
- Contradictions with workbook: none. One internal log nuance: `get --json` printed `external_id=None` (line 21) while the raw record file contains `"external_id": "feature/PAY-123-checkout-refactor"` (line 33); the workbook's claim that external_id equals the branch name is backed by the raw-record line and the grep, not by the `get --json` surface.

#### New Reproduction
- Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run.
- Historical retry documented in the log itself: system `python3` lacked pytest (`/usr/bin/python3: No module named pytest`, line 27); the log's "--- pytest (retry with venv python) ---" section (line 29) re-ran the targeted tests via a scratchpad venv (pytest 9.1.1 per the workbook comment) with result `3 passed, 121 deselected, 5 subtests passed in 0.34s`. The workbook Retry Count is 0; this venv fallback was a harness environment correction, not a product retry.

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); native Claude Code client behavior was NOT exercised. The case's ai_tool is Claude Code and its hook framing (Stop hook `working-memory-write.sh` driving the turn checkpoint) demands a native client for full fidelity; the log exercises `context.py turn` directly. Native assertions (Stop-hook invocation by the client, stdout silencing by the hook wrapper) are marked "Native assertions unavailable".

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: **High.** The original log is present and complete, the case's central command appears verbatim with its full JSON output and exit code, all six mandatory behaviors have direct log lines, and the commit (3a02537, clean tree) is corroborated by the workbook row, the execute-test-cases workflow journal, and TC-001.log's "0 uncommitted changes" banner.
- Evidence proves: auto-provisioning on first flush with branch-name external_id, correct derived-goal formatting, checkpoint-vs-progress separation, no-provisioning-on-visit control, validator pass, and targeted unit-test pass at commit 3a02537.
- Evidence does not prove: behavior when the Stop hook itself (native client) drives the turn; sensitive-path exclusion (not probed in this log — asserted only by the safety_expectations and the workbook comment that only paths reached the buffer).
- Missing information: full command lines for the validate/get/control steps (harness shorthand); the commit hash is not stated inside this log file itself.
- Recommended central-team follow-up: none specific to this run beyond a one-time native Claude Code session exercising the Stop-hook path (shared with RUN-042/RUN-043 below).

#### Artifact Inventory
- TC-059 evidence log — shareable filename `logs/original/TC-059.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:31:41 +0300 (content window Mon Aug 3 08:17:20-08:17:33 UTC 2026) — sensitive-content review: none found — included: yes.
- Case definition TC-059 — shareable filename `manifests/cases_map.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:14 — sensitive-content review: none found — included: yes.
- Executor workflow journal — shareable filename `transcripts/wf_e8e050fd-8be.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:30 — sensitive-content review: none found — included: yes.

### RUN-037 / TC-060

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)", Model: "n/a")
- Accelerator version: 1.4.3
- Git commit: 3a02537
- Execution date: 2026-08-03 (current workbook renders "2026-08-03 00:00:00" after the later mechanical rewrite)
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-060.log`
- Row presence: present in BOTH the current workbook and the historical snapshot; identical except the mechanical datetime/float coercions (counts "3"/"1"/"0" -> "3.0"/"1.0"/"0.0").
- Counters as recorded: Tests Executed 3 / Passed 3 / Failed 0; Human Intervention 0; Retry Count 1; Safety Violations 0; Operating Mode Governed; Result Pass.

#### Test Definition
- Definition status: **Authoritative — original source found.** `cases_map.json` contains TC-060 and the workbook's original Test Cases sheet agrees on every compared field (no mismatches). Original-sheet priority: High; owner: QA; execution type: Hybrid.
- Title: "Manual operator progress survives repeated automatic flushes (auto_checkpoint is a separate field)" (category: Project Brain).
- Objective: prove the automated checkpoint writer can never replace, truncate, or impersonate the operator's manual `progress` narrative.
- Preconditions: Laravel/ edition root; governed mode; task started via `python3 memory-bank/scripts/context.py --owner tester start --task-id feature/manual-progress --goal "Refactor checkout"`; at least one tracked file modified per turn.
- Input: `python3 memory-bank/scripts/context.py update --task-id feature/manual-progress --revision auto --progress "Implemented CartService; next: wire controller"`; then 10x `python3 memory-bank/scripts/context.py turn --task-id feature/manual-progress --flush --json`; then `python3 memory-bank/scripts/context.py get --task-id feature/manual-progress --json`.
- Mandatory assertions: (1) manual update writes the operator narrative into `progress`; (2) each forced flush writes ONLY `auto_checkpoint` ("Auto-checkpoint: N turn(s) since <timestamp>, M file(s) touched.") and merges changed paths bounded by `--max-files` (omitted count reported); (3) after ten flushes `get` still shows the exact manual progress string; (4) handoff and Task Capsule lead with manual progress, not the checkpoint; (5) a task with only an auto_checkpoint never presents it as manual progress.
- Optional assertions: none marked optional in the source definition.
- Scriptable assertions: `get --json` piped assert `d['progress'].startswith('Implemented CartService')` and `d['auto_checkpoint'].startswith('Auto-checkpoint:')`; targeted pytest `-k "manual_progress_survives_ten_automatic_flushes or handoff_and_capsule_lead_with_manual_progress or a_checkpoint_alone_does_not_pose_as_manual_progress"` passes.
- Native-client assertions: none demanded beyond CLI; "Cross-tool" implies the same behavior should hold when any AI tool's hook layer drives the flushes (not exercised here).
- Expected state changes: `project-brain/dynamic/tasks/<uuid>.md` (revision incremented per flush, progress untouched by flushes), `project-brain/control/handoffs/<uuid>.md` (refreshed), `memory-bank/local/context.db`.
- Safety expectations: automated writer never replaces/truncates operator progress; `--progress` content with secret-looking strings rejected without echoing the secret; only the governed task named by --task-id is touched.
- Requirement-source: Laravel/memory-bank/scripts/context.py (flush_turn_deltas docstring: progress belongs to the operator; update_working_task auto_checkpoint handling).

#### Historical Evidence
- Evidence status: **Available — original.**
- Original artifacts: `logs/original/TC-060.log` (shareable name: `logs/original/TC-060.log`; 3,251 bytes, 93 lines).
- Commands quoted verbatim from the log (the harness records shorthand markers, not full command lines): `$ update --progress`, `$ 10 forced flushes`, `$ get --json`.
- Exit codes recorded in the log: `start-exit=0` (line 3); `update-exit=1` (first attempt without --owner, line 6); `update-exit=0` (retry, line 61); `secret-exit=1` (secret-probe rejection, line 57).
- Assertions — first attempt (as the case prompt is literally written, without `--owner`): every mutation refused with `context: Owner is not authorized to mutate record: local` (lines 5, 8-17); `get --json` assert FAILED (`AssertionError`, `progress=''`, `auto_checkpoint=None`, lines 19-25). Retry with consistent `--owner tester` — Passed: `progress='Implemented CartService; next: wire controller'` and `auto_checkpoint='Auto-checkpoint: 1 turn(s) since 2026-08-03T08:18:33.960645+00:00, 1 file(s) touched.'` with `ASSERTS_OK` (lines 73-75); ten flushes bumped revisions (`flush True rev 3` ... `flush True rev 12`, lines 62-71); handoff leads with manual progress followed by "Since checkpoint:" (lines 85-87); pytest `3 passed, 121 deselected in 1.85s` (line 54); secret-looking `--progress` rejected: `context: possible GitHub token detected; working task not stored` (line 56). Failed (final): none. Skipped: none.
- Key output excerpt (log lines 59-61, 72-75, 85-87):
  ```
  === RETRY with consistent --owner tester ===
  Working task updated: feature/manual-progress.
  update-exit=0
  $ get --json
  progress='Implemented CartService; next: wire controller'
  auto_checkpoint='Auto-checkpoint: 1 turn(s) since 2026-08-03T08:18:33.960645+00:00, 1 file(s) touched.'
  ASSERTS_OK
  ## Current State
  Implemented CartService; next: wire controller
  Since checkpoint: Auto-checkpoint: 1 turn(s) since 2026-08-03T08:18:33.960645+00:00, 1 file(s) touched.
  ```
- Contradictions with workbook: none. The workbook's Retry Count 1 and its comment (retry caused by the case prompt omitting `--owner` on follow-up commands; the refusal is correct owner-authorization behavior, not a defect) match the log exactly. Minor log nuance: the sections "--- capsule leads with manual progress? ---" (line 91) and "--- capsule recheck ---" (line 93) are empty in the log; capsule-lead ordering is evidenced only via the passing `handoff_and_capsule_lead_with_manual_progress` pytest, as the workbook comment itself states.

#### New Reproduction
- Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run.
- Historical retry documented in the log: "=== RETRY with consistent --owner tester ===" (line 59) after the initial owner-authorization refusals; this is the workbook's Retry Count 1.

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); native cross-tool client behavior (Claude Code / Cursor / Codex hook layers issuing the flushes) was NOT exercised. The case's ai_tool is Cross-tool; cross-client parity assertions are marked "Native assertions unavailable".

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: **Medium.** Original log present and outputs complete, workbook matches the log, commit corroborated via the workbook row + workflow journal + TC-001.log clean-tree banner; held below High because the log records harness shorthand markers instead of complete command lines, and the capsule-lead assertion rests on the pytest name rather than captured capsule output.
- Evidence proves: owner authorization refuses unauthorized mutation; manual progress survives ten forced flushes verbatim; auto_checkpoint is a separate field; handoff leads with manual progress; secret-looking progress is rejected without echoing the value.
- Evidence does not prove: capsule (as opposed to handoff) ordering from captured output; `--max-files` bounding / omitted-count reporting (not probed in this log); behavior under a native client's hook layer.
- Missing information: full command lines; capsule stdout.
- Recommended central-team follow-up: none blocking; optionally capture capsule stdout in a future re-run to close the ordering assertion directly.

#### Artifact Inventory
- TC-060 evidence log — shareable filename `logs/original/TC-060.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:31:41 +0300 (content window 08:18:11-08:18:34 UTC) — sensitive-content review: none found (the GitHub-token probe value is not echoed anywhere in the log) — included: yes.
- Case definition TC-060 — shareable filename `manifests/cases_map.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:14 — sensitive-content review: none found — included: yes.
- Executor workflow journal — shareable filename `transcripts/wf_e8e050fd-8be.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:30 — sensitive-content review: none found — included: yes.

### RUN-038 / TC-061

> Priority run — the central team requested extra depth. Each requested aspect is addressed explicitly below with its evidence availability.

#### Workbook Record
- Recorded result: Partial
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)", Model: "n/a")
- Accelerator version: 1.4.3
- Git commit: 3a02537
- Execution date: 2026-08-03 (current workbook renders "2026-08-03 00:00:00" after the later mechanical rewrite)
- Defect reference: DEF-005 (workbook Defects sheet; corresponds to `manifests/defects.json` key "TC-061": "TC-061 case definition uses a phase vocabulary the CLI rejects", severity Low / priority Low). Note the current workbook also contains an unrelated Codex-side "DEF-005"-adjacent numbering collision block (DEF-001..DEF-004 duplicated); DEF-005 itself is not collided per the workbook forensics digest.
- Workbook evidence link: `Accelerator-TestEvidence/TC-061.log`
- Failure Reason as recorded: two literal expectations of the case do not match the implementation — (1) `--phase implementing` is not a valid CLI choice (valid: understanding/planning/execution/finalization); (2) for the case's query the retrieve output surfaces the task record (same continuation content) rather than naming the handoff path and phase. Functional intent met; repo tests pass.
- Row presence: present in BOTH the current workbook and the historical snapshot; identical except the mechanical datetime/float coercions.
- Counters as recorded: Tests Executed 2 / Passed 2 / Failed 0; Human Intervention 0; Retry Count 1; Safety Violations 0; Operating Mode Governed; Result Partial.

#### Test Definition
- Definition status: **Authoritative — original source found**, with one documented divergence between the two original sources: the harness definition `cases_map.json` (the version actually executed) writes `--phase implementing`, while the workbook's original Test Cases sheet writes `--phase implementation` in both Preconditions and Expected Behaviour. All other compared fields agree. Both variants are outside the CLI's accepted vocabulary.
- Title: "Handoff record carries a governed task across sessions" (category: Project Brain; original-sheet priority: High; owner: QA; execution type: Hybrid).
- Objective: prove a Session-A governed task's handoff is a bounded continuation record that lets Session B continue without transcript replay.
- Preconditions (harness variant): Laravel/ edition root, governed mode; Session A: `--owner sessionA start --task-id feature/handoff-demo --goal "Add invoice export"`, then `update --task-id feature/handoff-demo --revision auto --progress "Export service done" --next-step "Add controller test" --phase implementing`; both stores committed; Session B is a fresh shell.
- Input: Session B: `python3 memory-bank/scripts/context.py retrieve "invoice export continuation" --task-id feature/handoff-demo --json`
- Mandatory assertions: (1) after Session A's update the handoff `project-brain/control/handoffs/<task_uuid>.md` is refreshed as a bounded continuation record (objective, sanitized progress, next steps, phase) — never a transcript; budget category handoff = 1,500 estimated tokens per PROTOCOL.md; (2) the `--phase` value reaches the handoff; (3) Session B's retrieve assembles governed context whose handoff/dynamic layers include the task's handoff content; (4) the handoff validates against its task; a stale task binding is rejected by the validator.
- Optional assertions: none marked optional in the source definition.
- Scriptable assertions: `ls project-brain/control/handoffs/*.md` shows the handoff; `validate` exits 0; retrieve output mentions the handoff path and the phase; targeted pytest `-k "a_task_phase_reaches_the_handoff or retrieve_writes_bounded_budgeted_manifest_and_context_alias_matches"` passes.
- Native-client assertions: the agent_command frames the flow as `/project-brain` (command file `.claude/commands/project-brain.md`) driving context.py — the slash-command layer is native-client behavior.
- Expected state changes: `project-brain/control/handoffs/<task_uuid>.md` refreshed by Session A; manifest in `project-brain/control/retrieval-manifests/` or `memory-bank/local` depending on `--ephemeral`; no new records created by retrieval.
- Safety expectations: handoff contains no raw prompts, responses, chain-of-thought, command output, secrets, or personal data ("A handoff is a bounded continuation record, not a transcript"); Session B's retrieve is read-only — no revision consumed.
- Requirement-source: Laravel/.claude/skills/project-brain/SKILL.md (Governed Task Lifecycle steps 3-6); Laravel/project-brain/PROTOCOL.md (handoff budget and authority).

#### Historical Evidence
- Evidence status: **Available — original.**
- Original artifacts: `logs/original/TC-061.log` (shareable name: `logs/original/TC-061.log`; 3,420 bytes, 91 lines).
- Commands quoted verbatim from the log: the harness records shorthand markers; the one full command form preserved is inside the argparse usage echo (below). Marker lines include `--- Session A ---`, `--- Session B (fresh shell via env -i bash -c) ---`, `--- validate ---`, `=== RETRY with valid phase vocabulary (execution) ===`, `--- text-mode retrieve (Session B) ---`, `--- is the handoff indexed / retrievable at all? ---`.
- Exit codes recorded in the log: `exit=0` (Session A start, line 4); `exit=2` (the rejected update, line 11); `retrieve-exit=0` (lines 29 and 59); `exit=0` after "Project Brain validation passed." (line 35); `exit=0` (retry update, line 42).
- Assertions — Passed: handoff file exists and is bounded (lines 15-27 initial, lines 45-57 after retry: Phase `execution`, Objective `Add invoice export`, Current State `Export service done`, Next Actions `- Add controller test`); validate passed; pytest `2 passed, 122 deselected in 0.64s` (line 38); transcript-safety scan `0 transcript lines` (line 67); retrieval read-only — `revision 2` before/after (line 69); handoff present in a retrieval layer (`handoff-in-any-layer: True`, line 64). Failed (literal case expectations, recorded as the Partial): `mentions handoff path: False` and `mentions implementing phase: False` (lines 30-31), and after retry `mentions handoff path: False` / `mentions execution phase: False` (lines 60-61). Skipped: none.
- Key output excerpt (log lines 5-11):
  ```
  usage: context.py update [-h] --task-id TASK_ID [--progress PROGRESS]
                           [--next-step NEXT_STEP] [--file FILE]
                           [--source SOURCE]
                           [--phase {understanding,planning,execution,finalization}]
                           [--revision REVISION] [--json]
  context.py update: error: argument --phase: invalid choice: 'implementing' (choose from 'understanding', 'planning', 'execution', 'finalization')
  exit=2
  ```
- Contradictions with workbook: none — the workbook's Partial, Failure Reason, Retry Count 1, and DEF-005 all match the log content exactly.

Priority aspects requested by the central team:
1. **Original definition — evidence Available.** Both original sources exist: `manifests/cases_map.json` TC-061 (harness definition, executed) and the historical Test Cases sheet row (historical-testcases.json). Divergence: sheet says `--phase implementation`, harness says `--phase implementing`; the log proves the harness variant (`implementing`) was the one executed (argparse names it verbatim).
2. **Source of the "implementing" phase value — evidence Available (provenance), root cause recorded.** The case was authored by the `accelerator-test-cases` workflow (wf_3c021cce-042, 2026-08-03 10:57-11:05, brain-memory drafting agent per that workflow's digest); `defects.json` records root_cause "Test case authored against an older/imagined phase vocabulary". No repo source for an "implementing" phase vocabulary was identified; the CLI vocabulary at 3a02537 is understanding/planning/execution/finalization.
3. **Failing command — evidence Available.** The exact rejection is in the log: `context.py update: error: argument --phase: invalid choice: 'implementing' (choose from 'understanding', 'planning', 'execution', 'finalization')` with `exit=2`. defects.json reproduction_steps preserves the full failing command line: `python3 memory-bank/scripts/context.py --owner sessionA update --task-id feature/handoff-demo --revision auto --phase implementing`.
4. **Supported phases — evidence Available.** The argparse usage echo in the log lists `[--phase {understanding,planning,execution,finalization}]` verbatim.
5. **Queries and rankings — evidence Available.** (a) Case query "invoice export continuation": JSON retrieve returned top-level keys `['categories', 'episodic', 'last_turn', 'manifest', 'manifest_scope', 'procedural', 'query', 'selected', 'semantic', 'task_id', 'task_revision', 'task_uuid', 'token_estimates', 'warnings', 'working']` (line 32); the working layer carried `{"task_id": "feature/handoff-demo", "goal": "Add invoice export", "progress": "Export service done", "next_steps": ["Add controller test"], ...}` (line 62). (b) Text-mode ranking for the same query (lines 72-80): procedural listed `.agents/skills/architecture-implementer/SKILL.md`, `.agents/skills/test-generator/SKILL.md`, `.agents/skills/systematic-debugger/SKILL.md`; semantic ranked `project-brain/dynamic/tasks/5f95c846-d090-4870-9e45-0e774dd3abb6.md — Add invoice export` first, then `README.md — Laravel AI Accelerator` — i.e. the task record outranked the handoff for the case's query. (c) A query matching the handoff ("handoff invoice export" per the workbook narrative) surfaced it: `semantic brain-handoff: project-brain/control/handoffs/5f95c846-d090-4870-9e45-0e774dd3abb6.md — Handoff feature/handoff-demo` with Phase `execution` visible (lines 82-89) — proving the handoff IS indexed (kind brain-handoff) and retrievable with its phase.

#### New Reproduction
- Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run.
- Historical retry documented in the log: "=== RETRY with valid phase vocabulary (execution) ===" (line 40) after the argparse rejection; this is the workbook's Retry Count 1. Note: the fix-qa-findings workflow (2026-08-03 15:31-16:28) later addressed the `--phase` vocabulary finding (DEF-007 grouping per its digest) in the uncommitted working tree; that remediation postdates and does not alter this run's evidence.

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); native cross-tool client behavior was NOT exercised. The `/project-brain` slash-command layer (`.claude/commands/project-brain.md`) named in the case's agent_command was not invoked by a native client; Session B was simulated with `env -i bash -c` (fresh environment), which is a faithful process-isolation proxy but not a real second client session. These assertions are marked "Native assertions unavailable".

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: **Medium.** The original log is complete and the decisive outputs (argparse usage + error, handoff content before/after, JSON/text retrieve layers, validator pass, pytest) are captured verbatim, and the workbook matches the log exactly; held below High because most executed command lines are recorded as shorthand markers (the full failing command survives only in defects.json) and the commit is corroborated externally rather than stated in this log.
- Evidence proves: the case-as-written fails at argparse (both the "implementing" and, by the same vocabulary, the sheet's "implementation" variant); the CLI phase vocabulary at 3a02537; with a valid phase the handoff is a bounded, transcript-free continuation record carrying phase/objective/state/next actions; cross-session continuation works without replay; retrieval is read-only; the handoff is indexed and retrievable; the Partial is test-case drift, not a product regression.
- Evidence does not prove: `/project-brain` slash-command behavior in a native client; the stale-binding validator rejection (delegated to pytest coverage per the log's final marker line, not directly probed here); handoff token-budget measurement against the 1,500-token PROTOCOL.md category.
- Missing information: full command lines for the non-failing steps; a captured ranking explanation (scores) for why the task record outranks the handoff.
- Recommended central-team follow-up: align the case definition with the CLI vocabulary (or vice versa) and restate the retrieve assertion to accept the task record carrying identical continuation content; both sub-findings are already tracked as DEF-005 / defects.json "TC-061".

#### Artifact Inventory
- TC-061 evidence log — shareable filename `logs/original/TC-061.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:31:41 +0300 (content window 08:19:09-08:19:26 UTC) — sensitive-content review: none found — included: yes.
- Case definition TC-061 (harness) — shareable filename `manifests/cases_map.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:14 — sensitive-content review: none found — included: yes.
- Defect record (workbook DEF-005) — shareable filename `manifests/defects.json` (key "TC-061") — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 15:29 — sensitive-content review: none found — included: yes.
- Executor workflow journal — shareable filename `transcripts/wf_e8e050fd-8be.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:30 — sensitive-content review: none found — included: yes.

### RUN-039 / TC-062

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)", Model: "n/a")
- Accelerator version: 1.4.3
- Git commit: 3a02537
- Execution date: 2026-08-03 (current workbook renders "2026-08-03 00:00:00" after the later mechanical rewrite)
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-062.log`
- Row presence: present in BOTH the current workbook and the historical snapshot; identical except the mechanical datetime/float coercions.
- Counters as recorded: Tests Executed 4 / Passed 4 / Failed 0; Human Intervention 0; Retry Count 0; Safety Violations 0; Operating Mode Governed; Result Pass.

#### Test Definition
- Definition status: **Authoritative — original source found.** `cases_map.json` contains TC-062 and the workbook's original Test Cases sheet agrees on every compared field (no mismatches). Original-sheet priority: Critical; owner: QA; execution type: Hybrid.
- Title: "CAS: brain-update with a stale revision is rejected and the record is left untouched" (category: Project Brain).
- Objective: prove compare-and-swap concurrency control on governed Brain records: stale writers are rejected without partial writes.
- Preconditions: Laravel/ edition root, governed mode; finding created via `python3 memory-bank/scripts/context.py --owner tester brain-create finding --external-id FND-CAS-1 --title "N+1 in OrderRepository"` — UUID and revision 1 noted.
- Input: `python3 memory-bank/scripts/context.py brain-update --record-id <UUID> --revision 1 --progress "first update" --reason "CAS demo"` (succeeds, revision becomes 2); then repeat with the now-stale revision 1 and `--progress "lost update attempt" --reason "stale write"`.
- Mandatory assertions: (1) first update with current revision succeeds, increments to revision 2, appends an append-only transition/ledger entry; (2) second update presenting revision 1 is rejected non-zero with a stale-revision error, record keeps revision 2 and the first update's progress; (3) documented recovery is reload (`brain-get`), reconcile, retry; (4) `--revision auto` performs a locked CAS; `--revision 0`, negative, or non-integer rejected by the `revision_argument` parser; (5) under two truly concurrent writers only one revision writer wins.
- Optional assertions: none marked optional in the source definition.
- Scriptable assertions: second command exits non-zero and stderr names the revision conflict; `brain-get --json` assert `d['revision']==2 and d['progress']=='first update'`; targeted pytest `-k "concurrent_compare_and_swap_allows_one_revision_writer or revision_cas_owner_authorization_and_transition_rules or revision_auto_performs_a_locked_compare_and_swap or revision_argument_rejects_non_positive_and_non_auto_values"` passes.
- Native-client assertions: none — pure CLI case; Cross-tool label implies vocabulary parity across clients only.
- Expected state changes: `project-brain/dynamic/findings/<UUID>.md` (revision 2 after step 1; unchanged by the rejected write); `project-brain/indexes/*` rebuilt on successful mutation only.
- Safety expectations: no partial write on rejection (snapshot/restore leaves both stores intact); error message must not echo secrets from the attempted payload; revision/lifecycle fields never hand-edited.
- Requirement-source: Laravel/.claude/skills/project-brain/SKILL.md (Governed Records rule 6: 'On a stale revision, reload, reconcile, and retry'); Laravel/memory-bank/scripts/brain_runtime.py (update_record CAS).

#### Historical Evidence
- Evidence status: **Available — original.**
- Original artifacts: `logs/original/TC-062.log` (shareable name: `logs/original/TC-062.log`; 1,739 bytes, 27 lines).
- Commands quoted verbatim from the log (shorthand markers): `$ brain-create finding`, `$ first update rev 1`, `$ stale update rev 1 again`, `$ brain-get: record untouched by stale write`, `$ invalid revision values`, `$ --revision auto works`.
- Exit codes recorded in the log: `exit=0` (create, line 3); `exit=0` (first update, line 8); `exit=1` (stale update, line 11); `exit=0` recorded three times after the three parser-rejection messages (lines 16, 18, 20 — see Contradictions); `exit=0` (`--revision auto`, line 23).
- Assertions — Passed: create returned the full record JSON at `"revision": 1` with initial transition `{"from": null, "to": "open", ... "reason": "Finding created"}` (line 4); stale write rejected with `context: Stale finding revision: expected 1, current 2` and `exit=1` (lines 10-11); record untouched: `revision=2 progress=first update ASSERT_OK` (line 13); invalid values each produced `context.py brain-update: error: argument --revision: Revision must be a positive integer or 'auto'` (lines 15, 17, 19); `--revision auto` succeeded (lines 22-23); pytest `4 passed, 120 deselected in 0.48s` (line 26). Failed: none. Skipped: none. The concurrent-writers assertion (5) is covered by the passing `concurrent_compare_and_swap_allows_one_revision_writer` test, not by a direct log probe.
- Key output excerpt (log lines 9-13):
  ```
  $ stale update rev 1 again
  context: Stale finding revision: expected 1, current 2
  exit=1
  $ brain-get: record untouched by stale write
  revision=2 progress=first update ASSERT_OK
  ```
- Contradictions with workbook: one recording artifact worth flagging. The log prints `exit=0` (lines 16, 18, 20) immediately after each `Revision must be a positive integer or 'auto'` parser error, although argparse rejections exit non-zero and the case expects rejection; the rejection messages themselves are present verbatim, and non-zero exit behavior is separately proven by the passing `revision_argument_rejects_non_positive_and_non_auto_values` pytest. This looks like a harness capture artifact (exit code read after an intervening pipeline step), not a product behavior claim; the workbook's statement ("--revision 0, -3, and 'abc' were all rejected by the revision_argument parser") is supported by the printed errors, but the literal `exit=0` lines contradict a strict "exits non-zero" reading of the scriptable assertion for these three probes.

#### New Reproduction
- Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run.
- No historical retries are documented in the log (workbook Retry Count 0).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); native cross-tool client behavior was NOT exercised. This case is CLI-only in substance; no native assertions are demanded beyond cross-client availability, which is marked "Native assertions unavailable".

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: **Medium.** Original log complete and decisive outputs verbatim (create JSON, stale-revision refusal, untouched-record assert, parser errors, auto-CAS success, pytest); held below High because command lines are shorthand markers, the three `exit=0` lines after parser errors are an unresolved capture artifact, and the commit is corroborated externally.
- Evidence proves: CAS stale-write rejection with no partial write; append-only transition ledger; `--revision auto` locked CAS; parser rejection messages for invalid revisions; targeted concurrency/authorization tests pass at 3a02537.
- Evidence does not prove: non-zero exit codes for the three invalid-revision probes from the log alone (see artifact above; proven only via pytest); no-secret-echo on rejection (no secret payload probe in this log).
- Missing information: full command lines; explanation of the `exit=0` capture artifact.
- Recommended central-team follow-up: if exit-code strictness matters for the record, a one-line re-probe of `--revision 0` capturing `$?` directly would close the artifact; otherwise none.

#### Artifact Inventory
- TC-062 evidence log — shareable filename `logs/original/TC-062.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:31:41 +0300 (content window 08:20:20-08:20:21 UTC) — sensitive-content review: none found — included: yes.
- Case definition TC-062 — shareable filename `manifests/cases_map.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:14 — sensitive-content review: none found — included: yes.
- Executor workflow journal — shareable filename `transcripts/wf_e8e050fd-8be.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:30 — sensitive-content review: none found — included: yes.

### RUN-040 / TC-063

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)", Model: "n/a")
- Accelerator version: 1.4.3
- Git commit: 3a02537
- Execution date: 2026-08-03 (current workbook renders "2026-08-03 00:00:00" after the later mechanical rewrite)
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-063.log`
- Row presence: present in BOTH the current workbook and the historical snapshot; identical except the mechanical datetime/float coercions.
- Counters as recorded: Tests Executed 5 / Passed 5 / Failed 0; Human Intervention 0; Retry Count 1; Safety Violations 0; Operating Mode Governed; Result Pass.

#### Test Definition
- Definition status: **Authoritative — original source found.** `cases_map.json` contains TC-063 and the workbook's original Test Cases sheet agrees on every compared field (no mismatches). Original-sheet priority: Critical; owner: QA; execution type: Hybrid.
- Title: "Authority: observed→verified promotion accepted, every other transition rejected" (category: Project Brain).
- Objective: prove the authority state machine has exactly one edge (observed→verified) and that refusals consume nothing.
- Preconditions: Laravel/ edition root, governed mode; finding created with default `--authority observed` (choices inferred|observed|verified); UUID and revision known.
- Input: `python3 memory-bank/scripts/context.py brain-update --record-id <UUID> --revision auto --authority verified --reason "Verified against migration and test run"`; then the illegal reverse: `python3 memory-bank/scripts/context.py brain-update --record-id <UUID> --revision auto --authority observed --reason "downgrade attempt"`.
- Mandatory assertions: (1) promotion observed→verified consumes one CAS revision and appends an authority ledger entry (actor, reason, from/to, timestamp), status chain untouched; (2) downgrade exits non-zero with exact shape `Illegal authority transition: verified -> observed; only observed -> verified is allowed`, record stays verified; (3) observed→inferred / inferred→verified refused the same way; (4) event records refuse authority changes entirely; (5) a hand-tampered authority ledger is rejected by `validate`.
- Optional assertions: none marked optional in the source definition.
- Scriptable assertions: `brain-get --json` assert `authority=='verified'`; downgrade exits non-zero printing 'Illegal authority transition'; targeted pytest `-k "cli_brain_update_promotes_authority_and_reports_refusals or reverse_and_arbitrary_authority_transitions_are_rejected or authority_promotion_runs_under_the_same_compare_and_swap or event_authority_is_immutable or tampered_authority_ledger_is_rejected_by_the_validator"` passes.
- Native-client assertions: none — pure CLI case.
- Expected state changes: `project-brain/dynamic/findings/<UUID>.md` (authority: verified, ledger appended); no files changed by the rejected attempts.
- Safety expectations: authority promotion is not a shortcut around review — never applies a Memory Bank promotion by itself; no revision consumed by a refused transition; refusal messages never echo secrets.
- Requirement-source: Laravel/memory-bank/scripts/brain_runtime.py (AUTHORITY_TRANSITIONS = {"observed": {"verified"}}; update_record); Laravel/memory-bank/scripts/context.py (brain-update --authority help text).

#### Historical Evidence
- Evidence status: **Available — original.**
- Original artifacts: `logs/original/TC-063.log` (shareable name: `logs/original/TC-063.log`; 3,354 bytes, 104 lines).
- Commands quoted verbatim from the log (shorthand markers): `$ promote observed->verified`, `$ illegal downgrade verified->observed`, `$ observed->inferred attempt on a second record`, `$ event authority immutability`, `$ attempt authority change on event`.
- Exit codes recorded in the log: `exit=0` (promotion, line 5); `exit=1` (downgrade, line 10); `exit=1` (observed→inferred, line 14); `event-create-exit=2` (first event create, missing --external-id, line 16) followed by `exit=1` (line 18); retry section `exit=2` (line 51) then event create with `--external-id` `exit=0` (line 59); authority change on event `exit=1` (line 63).
- Assertions — Passed: promotion succeeded (`authority=verified rev=2`, line 6); downgrade refused verbatim `context: Illegal authority transition: verified -> observed; only observed -> verified is allowed` with record unchanged (`still authority=verified revision=2`, lines 9-11); observed→inferred refused: `context: Illegal authority transition: observed -> inferred; only observed -> verified is allowed` (line 13); event authority change refused: `context: Events are immutable; only lifecycle supersession is allowed` (line 62); ledger entry appended into the record's `transitions` array with actor `tester`, reason "Verified against migration and test run", `"from": "observed"`, `"to": "verified"`, timestamps (lines 25-40 and raw-record dump lines 73-90); pytest `5 passed, 119 deselected, 5 subtests passed in 0.27s` (line 21). Failed: none. Skipped: the inferred→verified direction was not directly probed in-log (covered by the passing `reverse_and_arbitrary_authority_transitions_are_rejected` test); the tampered-ledger rejection is likewise pytest-covered, not probed directly.
- Key output excerpt (log lines 8-14):
  ```
  $ illegal downgrade verified->observed
  context: Illegal authority transition: verified -> observed; only observed -> verified is allowed
  exit=1
  still authority=verified revision=2
  $ observed->inferred attempt on a second record
  context: Illegal authority transition: observed -> inferred; only observed -> verified is allowed
  exit=1
  ```
- Contradictions with workbook: none material. One internal log nuance: line 7 records `authority_ledger: "MISSING"` — the harness probed for a dedicated `authority_ledger` field that does not exist; the authority entry is appended to the record's `transitions` ledger, which the log then dumps in full (lines 25-40). The workbook's "appended a ledger entry with actor/reason/from=observed/to=verified/timestamp" is supported by those transitions entries, not by a field named authority_ledger.

#### New Reproduction
- Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run.
- Historical retry documented in the log: the first `brain-create event` failed with `event-create-exit=2` because `--external-id` is required (`context.py brain-create: error: the following arguments are required: --external-id`, line 49); the log's "--- retry event create with required fields ---" / "--- event create with --external-id ---" sections re-created the event successfully (`exit=0`, `event UUID=90662416-333c-4b0c-919b-3f48001db4cb`, lines 59-60). This is the workbook's Retry Count 1; the workbook comment correctly attributes it to a precondition the case narrative does not mention, not to a product failure.

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); native cross-tool client behavior was NOT exercised. No native assertions are demanded by this CLI case beyond cross-client availability, which is marked "Native assertions unavailable".

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: **Medium.** Original log complete with verbatim refusal shapes, ledger dump, and pytest results; held below High because command lines are shorthand markers, two of the five mandatory assertions (inferred→verified refusal, tampered-ledger validator rejection) rest on pytest names rather than direct probes, and the commit is corroborated externally.
- Evidence proves: single-edge authority state machine with exact refusal wording; refused transitions consume no revision; event authority immutability; ledger append with full actor/reason/from/to/timestamp detail; targeted tests pass at 3a02537.
- Evidence does not prove: the tampered-ledger rejection via direct manipulation; that the promotion did not trigger a Memory Bank promotion (asserted in the workbook comment "No Memory Bank promotion was applied by the authority promotion itself" — no direct log probe).
- Missing information: full command lines; direct inferred→verified probe.
- Recommended central-team follow-up: none; the pytest coverage names map one-to-one onto the unprobed assertions.

#### Artifact Inventory
- TC-063 evidence log — shareable filename `logs/original/TC-063.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:31:41 +0300 (content window 08:20:38-08:20:39 UTC) — sensitive-content review: none found — included: yes.
- Case definition TC-063 — shareable filename `manifests/cases_map.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:14 — sensitive-content review: none found — included: yes.
- Executor workflow journal — shareable filename `transcripts/wf_e8e050fd-8be.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:30 — sensitive-content review: none found — included: yes.

### RUN-041 / TC-064

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)", Model: "n/a")
- Accelerator version: 1.4.3
- Git commit: 3a02537
- Execution date: 2026-08-03 (current workbook renders "2026-08-03 00:00:00" after the later mechanical rewrite)
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-064.log`
- Row presence: present in BOTH the current workbook and the historical snapshot; identical except the mechanical datetime/float coercions.
- Counters as recorded: Tests Executed 5 / Passed 5 / Failed 0; Human Intervention 0; Retry Count 0; Safety Violations 0; Operating Mode Governed; Result Pass.

#### Test Definition
- Definition status: **Authoritative — original source found.** `cases_map.json` contains TC-064 and the workbook's original Test Cases sheet agrees on every compared field (no mismatches). Original-sheet priority: Critical; owner: QA; execution type: Hybrid.
- Title: "Second machine: deleted local context.db is repaired by rebind without touching the governed record" (category: Project Brain).
- Objective: prove the Git-travelling Brain record is the authority and the SQLite binding is disposable machine-local state repairable by `rebind` (or implicitly by `turn --flush`) with zero governed-record mutation.
- Preconditions: Laravel/ edition root; machine A created governed task `feature/rebind-demo` (record + handoff committed); machine B simulated via `rm memory-bank/local/context.db` (memory-bank/local/ is gitignored); record UUID and revision noted from `project-brain/dynamic/tasks/`.
- Input: `python3 memory-bank/scripts/context.py rebind --task-id feature/rebind-demo --json`; same command a second time; then negative control: `python3 memory-bank/scripts/context.py rebind --task-id feature/rebind-demo --record <UUID-of-a-DIFFERENT-task>`.
- Mandatory assertions: (1) first rebind resolves by external_id, recreates only the local binding, reports `"already_bound": false`, revision NOT incremented (pointer repair, no owner check); (2) second rebind reports `"already_bound": true` without recreating; (3) negative control refused — rebind can only reconnect a task to its own identity; a binding pointing elsewhere is refused; (4) non-task record UUID and terminal task refused with explicit reasons; (5) alternative path: `turn --task-id feature/rebind-demo --flush` after deleting the DB restores the binding implicitly, reporting the record UUID in the JSON `"rebound"` field.
- Optional assertions: none marked optional in the source definition.
- Scriptable assertions: `git diff --exit-code -- project-brain/` exits 0 after both rebinds; first JSON already_bound false, second true; repoint attempt exits non-zero mentioning the mismatch; targeted pytest `-k "rebind_restores_the_binding_for_an_explicit_record or rebind_resolves_the_record_by_task_id_alone or rebind_reports_an_intact_binding_instead_of_recreating_it or rebind_refuses_to_repoint_one_task_at_another or flush_on_a_second_machine_lands_in_the_existing_task"` passes.
- Native-client assertions: none — pure CLI case.
- Expected state changes: `memory-bank/local/context.db` recreated with the binding; `project-brain/dynamic/tasks/<uuid>.md` unchanged (same revision, same content).
- Safety expectations: no governed record mutation, no revision consumption, no handoff rewrite; existing local bindings never silently overwritten; rebind requires governed mode and refuses lightweight.
- Requirement-source: Laravel/memory-bank/scripts/context.py (rebind_governed_task docstring); Laravel/.gitignore (memory-bank/local/).

#### Historical Evidence
- Evidence status: **Available — original.**
- Original artifacts: `logs/original/TC-064.log` (shareable name: `logs/original/TC-064.log`; 1,825 bytes, 28 lines).
- Commands quoted verbatim from the log (shorthand markers): `$ rebind #1`, `$ rebind #2`, `$ negative: repoint at a different task record`, `$ git diff project-brain unchanged`.
- Exit codes recorded in the log: `exit=0` (task start, line 4); `exit=0` (rebind #1, line 12); `exit=0` (rebind #2, line 15); `exit=1` (negative repoint, line 18); `git-diff-exit=0` (line 20).
- Assertions — Passed: rebind #1 JSON reports `"already_bound": false` at `"revision": 1` (line 11); rebind #2 identical JSON with `"already_bound": true` (line 14); negative control refused verbatim: `context: Rebind target mismatch: record 5f95c846-d090-4870-9e45-0e774dd3abb6 belongs to task id 'feature/handoff-demo', not 'feature/rebind-demo'` (line 17); `revision-after=1 (must equal 1)` (line 21); alternative path after re-deleting the DB: `rebound='52aad7a9-6d0e-4b95-b341-1cce5ad08d7b' flushed=True provisioned=False` (line 24); pytest `5 passed, 119 deselected in 1.15s` (line 27). Failed: none. Skipped: assertion (4) — non-task record UUID and terminal-task refusals — has no direct probe in the log; it is covered only insofar as the targeted pytest set passes.
- Key output excerpt (log lines 16-21):
  ```
  $ negative: repoint at a different task record
  context: Rebind target mismatch: record 5f95c846-d090-4870-9e45-0e774dd3abb6 belongs to task id 'feature/handoff-demo', not 'feature/rebind-demo'
  exit=1
  $ git diff project-brain unchanged
  git-diff-exit=0
  revision-after=1 (must equal 1)
  ```
- Contradictions with workbook: none. Note for cross-run traceability: the alternative-path `rebound` UUID `52aad7a9-...` is the `feature/manual-progress` task from TC-060 (the log line 23 shows the branch context `feature/manual-progress`), i.e. the implicit-restore probe ran against the executor's shared temp-copy state rather than `feature/rebind-demo`; the JSON fields (`flushed=True provisioned=False`, non-null `rebound`) still demonstrate the documented implicit-restore behavior, and the dedicated `flush_on_a_second_machine_lands_in_the_existing_task` test passed.

#### New Reproduction
- Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run.
- No historical retries are documented in the log (workbook Retry Count 0).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); native cross-tool client behavior was NOT exercised. No native assertions are demanded by this CLI case beyond cross-client availability, which is marked "Native assertions unavailable".

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: **Medium.** Original log complete with both rebind JSON payloads, the exact mismatch refusal, git-diff proof, and pytest; held below High because command lines are shorthand markers, assertion (4) lacks a direct probe, the implicit-restore probe ran on a neighboring task's binding, and the commit is corroborated externally.
- Evidence proves: rebind is a pure pointer repair (already_bound false/true sequence, revision constant at 1, `git diff -- project-brain/` clean); repoint refusal with explicit reason; implicit binding restore via `turn --flush` reporting `rebound`; targeted tests pass at 3a02537.
- Evidence does not prove: refusals for non-task UUIDs and terminal tasks (pytest-only); lightweight-mode refusal (not probed); silent-overwrite protection for a binding pointing elsewhere (pytest-only).
- Missing information: full command lines; a rebind-demo-scoped run of the implicit-restore path.
- Recommended central-team follow-up: none.

#### Artifact Inventory
- TC-064 evidence log — shareable filename `logs/original/TC-064.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:31:41 +0300 (content window 08:21:18-08:21:20 UTC) — sensitive-content review: none found — included: yes.
- Case definition TC-064 — shareable filename `manifests/cases_map.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:14 — sensitive-content review: none found — included: yes.
- Executor workflow journal — shareable filename `transcripts/wf_e8e050fd-8be.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:30 — sensitive-content review: none found — included: yes.

### RUN-042 / TC-065

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)", Model: "n/a")
- Accelerator version: 1.4.3
- Git commit: 3a02537
- Execution date: 2026-08-03 (current workbook renders "2026-08-03 00:00:00" after the later mechanical rewrite)
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-065.log`
- Row presence: present in BOTH the current workbook and the historical snapshot; identical except the mechanical datetime/float coercions.
- Counters as recorded: Tests Executed 4 / Passed 4 / Failed 0; Human Intervention 0; Retry Count 0; Safety Violations 0; Operating Mode Governed; Result Pass.

#### Test Definition
- Definition status: **Authoritative — original source found.** `cases_map.json` contains TC-065 and the workbook's original Test Cases sheet agrees on every compared field (no mismatches). Original-sheet priority: High; owner: QA; execution type: Hybrid.
- Title: "last-turn-report surfaces a blocked promotion in the next request's Task Capsule" (category: Project Brain).
- Objective: prove that the Stop-hook turn's machine summary survives stdout silencing via `memory-bank/local/last-turn-report.json` and that the next refresh folds a blocked-promotion reason into the capsule with problems leading.
- Preconditions: Laravel/ edition root, governed mode, automatic_promotion enabled; a finding transitioned to `resolved` but still `authority: observed` (promotion blocked, reason `authority`); a governed task for the current branch with a pending delta.
- Input: `python3 memory-bank/scripts/context.py turn --task-id "$(git branch --show-current)" --flush --json` (simulates the Stop hook, whose stdout is silenced); then: `python3 memory-bank/scripts/context.py refresh --query "continue the checkout work" --task-id "$(git branch --show-current)" --ephemeral`.
- Mandatory assertions: (1) turn writes `memory-bank/local/last-turn-report.json` (schema_version 1, timestamp, task_id, flushed, files, promotion_blocked, promotion_failed, compaction errors, excluded paths); (2) next refresh folds it into one `Last turn:` line where problems lead — `promotion blocked: authority` before any routine counters; (3) section capped at 600 characters, at most 3 excluded paths listed; (4) a report older than 24h / future-dated / malformed / missing is silently ignored; (5) a buffering-only turn replaces the report and produces no section.
- Optional assertions: none marked optional in the source definition.
- Scriptable assertions: python assert `any(b.get('reason')=='authority' for b in d['promotion_blocked'])`; refresh stdout contains a line starting 'Last turn:' including 'promotion blocked: authority'; targeted pytest `-k "blocked_promotion_reaches_the_next_refresh_capsule or a_stale_report_is_ignored or a_missing_or_malformed_report_never_breaks_refresh or a_quiet_buffering_turn_replaces_the_report_and_says_nothing"` passes.
- Native-client assertions: the case is framed as hook-layer behavior ("working-memory-write.sh Stop hook + working-memory-read.sh UserPromptSubmit hook"); real hook invocation and stdout silencing by the client are native behaviors.
- Expected state changes: `memory-bank/local/last-turn-report.json` (rewritten each turn, gitignored); no shared-store changes from the blocked promotion.
- Safety expectations: a write failure of the report degrades silently; the report never contains file contents or secrets (only counters, reasons, paths); hook exit code stays 0.
- Requirement-source: Laravel/memory-bank/scripts/context.py (write_last_turn_report / load_last_turn_report / summarize_last_turn); Laravel/.claude/hooks/working-memory-read.sh.

#### Historical Evidence
- Evidence status: **Available — original.**
- Original artifacts: `logs/original/TC-065.log` (shareable name: `logs/original/TC-065.log`; 2,521 bytes, 56 lines).
- Commands quoted verbatim from the log (shorthand markers): `--- create finding, resolve while authority=observed ---`, `--- pending delta + turn flush (simulated Stop hook) ---`, `--- last-turn-report.json ---`, `--- next request refresh ---`, `--- quiet buffering turn replaces report ---`.
- Exit codes recorded in the log: `exit=0` (finding update, line 5); `exit=0` (turn flush, line 9); `refresh-exit=0` (line 41).
- Assertions — Passed: precondition established (`status=resolved authority=observed`, line 6); turn JSON contains `"promotion_blocked": [{"record_id": "c46947c7-abf0-4c68-86e8-715f77023264", "reason": "authority is observed, not verified"}]` (line 8); `last-turn-report.json` dumped with `schema_version: 1`, `task_id: "feature/manual-progress"`, `timestamp`, `flushed: true`, `files: 1`, `promotion_blocked` with reason/record_id, `promotion_failed: []`, `compaction_errors: []`, `excluded_paths: []` (lines 16-39); refresh output contains `Last turn: promotion blocked: authority is observed, not verified; buffer flushed: 1 file(s) this turn` with `FOUND_BLOCKED_LINE` and `last-turn-line-len=102` (lines 42-45, within the 600-char cap); pytest `4 passed, 120 deselected in 1.95s` (line 48); quiet-turn follow-up: `flushed False`, `report flushed field now: False blocked: []`, `no Last turn section` (lines 53-56). Failed (then adapted): the case's literal assert `reason=='authority'` raised `AssertionError` (lines 11-15) because the implementation stores the fuller sentence; the log's "--- adapted assert ---" section then records `ADAPTED_ASSERT_OK: reason='authority is observed, not verified'` (line 51). Skipped: stale/future-dated/malformed-report tolerance — no direct probe; covered by the named passing pytest cases.
- Key output excerpt (log lines 40-45):
  ```
  --- next request refresh ---
  refresh-exit=0
  5:Last turn: promotion blocked: authority is observed, not verified; buffer flushed: 1 file(s) this turn
  5:Last turn: promotion blocked: authority is observed, not verified; buffer flushed: 1 file(s) this turn
  FOUND_BLOCKED_LINE
  last-turn-line-len=102
  ```
- Contradictions with workbook: none — the workbook comment explicitly discloses the literal-assert nuance (implementation stores 'authority is observed, not verified', which starts with 'authority'; the capsule substring requirement 'promotion blocked: authority' literally holds). The duplicated grep line in the excerpt is a harness output duplication, not two refresh sections.

#### New Reproduction
- Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run.
- No command retries are documented in the log (workbook Retry Count 0); the "adapted assert" re-check (line 50-51) is a historical assertion adaptation recorded in the log, not a re-execution of the product commands.

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); native Claude Code client behavior was NOT exercised. The case's ai_tool is Claude Code and its core premise (Stop hook stdout silencing, UserPromptSubmit hook assembling the next capsule) is native-client wiring; the log simulates the Stop hook by invoking `context.py turn` directly (the log's own marker says "simulated Stop hook"). Native assertions (actual hook invocation, hook exit code 0, stdout discard) are marked "Native assertions unavailable".

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: **Medium.** Original log complete with the full report JSON, exact capsule line, length measurement, and quiet-turn control; held below High because command lines are shorthand markers, the hook layer itself was simulated rather than natively exercised, and the commit is corroborated externally.
- Evidence proves: report write with schema_version 1 and blocked-promotion payload; next-refresh capsule line with problems leading and 102-char section length; quiet-turn replacement with no section; targeted tolerance tests pass at 3a02537.
- Evidence does not prove: native hook invocation/stdout silencing; stale/malformed-report tolerance by direct probe; the 3-excluded-paths listing cap (excluded_paths was empty in this run).
- Missing information: full command lines; native-client capture.
- Recommended central-team follow-up: fold this case into the same one-time native Claude Code hook session recommended for RUN-036/RUN-043; consider aligning the case's literal `reason=='authority'` assert with the implemented sentence (cosmetic case-definition drift, no defect filed).

#### Artifact Inventory
- TC-065 evidence log — shareable filename `logs/original/TC-065.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:31:41 +0300 (content window 08:21:35-08:21:38 UTC) — sensitive-content review: none found — included: yes.
- Case definition TC-065 — shareable filename `manifests/cases_map.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:14 — sensitive-content review: none found — included: yes.
- Executor workflow journal — shareable filename `transcripts/wf_e8e050fd-8be.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:30 — sensitive-content review: none found — included: yes.

### RUN-043 / TC-066

> Priority run — the central team requested extra depth. Each requested aspect is addressed explicitly below with its evidence availability.

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)", Model: "n/a")
- Accelerator version: 1.4.3
- Git commit: 3a02537
- Execution date: 2026-08-03 (current workbook renders "2026-08-03 00:00:00" after the later mechanical rewrite)
- Defect reference: DEF-006 (workbook Defects sheet; corresponds to `manifests/defects.json` key "TC-066": "refresh distills the query before the capsule privacy gate, bypassing raw-query refusal", severity Low / priority Medium). DEF-006 is not affected by the Codex-side DEF-001..DEF-004 numbering collisions per the workbook forensics digest.
- Workbook evidence link: `Accelerator-TestEvidence/TC-066.log`
- Row presence: present in BOTH the current workbook and the historical snapshot; identical except the mechanical datetime/float coercions.
- Counters as recorded: Tests Executed 6 / Passed 6 / Failed 0; Human Intervention 0; Retry Count 0; Safety Violations 0; Operating Mode Governed; Result Pass (core case passed; the privacy-gate asymmetry was filed as a defect rather than failing the run).

#### Test Definition
- Definition status: **Authoritative — original source found.** `cases_map.json` contains TC-066 and the workbook's original Test Cases sheet agrees on every compared field (no mismatches). Original-sheet priority: High; owner: QA; execution type: Hybrid.
- Title: "Task Capsule respects the 8000-character budget and distills a long prompt whose subject sits in the tail" (category: Project Brain).
- Objective: prove rarity-ranked query distillation (not first-N-words) and capsule budget discipline, with ephemeral manifests kept out of shared history.
- Preconditions: Laravel/ edition root, governed task bound for the current branch, index populated (skills, README, specs, memory-bank chunks, CHANGELOG across procedural/semantic/episodic layers); prompt of several hundred words of generic filler whose actual subject appears only in the final sentence.
- Input: `python3 memory-bank/scripts/context.py refresh --query "$(cat /tmp/long-prompt.txt)" --task-id "$(git branch --show-current)" --ephemeral`
- Mandatory assertions: (1) raw prompt distilled to at most 24 informative terms (CAPSULE_PROMPT_TERM_LIMIT) ranked by rarity against the index, so the tail subject survives; (2) capsule within CAPSULE_CHARACTER_LIMIT = 8000 characters; layer caps procedural 2, semantic 3, episodic 1 (as written in the case definition — but see Contradictions), working files capped at 8, sources at 4; (3) under budget pressure optional layers drop before working state, latest progress suffix preserved, Last-turn section drops before retrieved layers; (4) semantic/procedural sections include the document matching the tail subject; (5) `--ephemeral` writes the retrieval manifest (including the query) to ignored local state, not shared Git history; (6) the layer report prints even when a layer fails.
- Optional assertions: none marked optional in the source definition.
- Scriptable assertions: capsule section of stdout measured alone <= 8000 characters; tail-subject document path appears in output; `git status --porcelain` shows no change under project-brain/; targeted pytest across both suites `-k "refresh_distills_a_raw_prompt_so_the_tail_subject_survives or distill_capsule_query_prefers_rare_terms_from_the_prompt_tail or context_capsule_stays_within_character_budget or capsule_budget_drops_optional_layers_before_working or capsule_budget_preserves_latest_progress_suffix or ephemeral_manifest_stays_out_of_shared_history"` passes.
- Native-client assertions: the case is framed as hook-layer behavior ("working-memory-read.sh assembles the capsule at prompt time"; the hook passes the raw prompt JSON from stdin to refresh) — real prompt-time invocation is native-client wiring.
- Expected state changes: `memory-bank/local/*` only (ephemeral manifest and index state); no writes under `project-brain/control/retrieval-manifests/` because of `--ephemeral`.
- Safety expectations: queries or working state matching secret patterns or personal-data patterns (emails, phone numbers, customer/patient identifiers per CAPSULE_PRIVATE_PATTERNS) are rejected without being echoed or persisted; raw transcript-looking text (user:/assistant:/stdout: prefixes) refused; mandatory content larger than the budget rejected rather than truncated.
- Requirement-source: Laravel/memory-bank/scripts/context.py (CAPSULE_CHARACTER_LIMIT, CAPSULE_PROMPT_TERM_LIMIT, CAPSULE_LAYER_LIMITS, assemble_capsule); Laravel/.claude/hooks/working-memory-read.sh.

#### Historical Evidence
- Evidence status: **Available — original.**
- Original artifacts: `logs/original/TC-066.log` (shareable name: `logs/original/TC-066.log`; 1,813 bytes, 51 lines).
- Commands quoted verbatim from the log (shorthand markers): `$ refresh with long prompt`, `$ private-data query (email)`, `$ transcript-looking query`, `$ retrieve with raw email query (privacy gate check)`, `$ context with transcript-looking query`, `$ refresh path: distillation happens before privacy gate; distilled tokens persisted in manifest:`.
- Exit codes recorded in the log: `exit=0` (refresh with long prompt, line 4); `exit=0` (private-data refresh probe, line 38); `exit=0` (transcript-looking refresh probe, line 42); `exit=0` (retrieve refusal probe, line 45); `exit=0` (context refusal probe, line 48 — see Contradictions on these last two).
- Assertions — Passed: `prompt words: 654` (line 2); layer report printed (`procedural: updated (90 documents)`, `semantic: updated (16 documents)`, `episodic: updated (1 documents)`, lines 5-7); capsule listed `.agents/skills/filament/SKILL.md — Filament` in procedural with `TAIL_SUBJECT_FOUND` (lines 12, 20-21); capsule length `436` characters (line 19, <= 8000); `PROJECT_BRAIN_UNCHANGED` (line 22); ephemeral manifests present as three local JSON files with `0 tracked manifest changes` (lines 24-28); pytest `6 passed, 179 deselected in 0.77s` (line 31); retrieve/context refusals verbatim: `context: Task Capsule request contains private or raw data; replace it with a sanitized summary` (lines 44, 47); manifest persistence of distilled tokens: `"query": "contact customer john doe example com about",` with the log's note `(raw email string itself is never persisted — tokenization strips @ and dots)` (lines 50-51). Failed: none of the case's own assertions. Skipped: budget-pressure ordering and progress-suffix preservation — no direct probe; covered by the named passing pytest cases (as the workbook states).
- Key output excerpt (log lines 43-51):
  ```
  $ retrieve with raw email query (privacy gate check)
  context: Task Capsule request contains private or raw data; replace it with a sanitized summary
  exit=0
  $ context with transcript-looking query
  context: Task Capsule request contains private or raw data; replace it with a sanitized summary
  exit=0
  $ refresh path: distillation happens before privacy gate; distilled tokens persisted in manifest:
    "query": "contact customer john doe example com about",
  (raw email string itself is never persisted — tokenization strips @ and dots)
  ```
- Contradictions with workbook and sources:
  1. Layer caps: the case definition (and original sheet) state caps "procedural 2, semantic 3, episodic 1"; the observed capsule listed 3 procedural, 2 semantic, 1 episodic entries (log lines 9-17). The workbook's Actual Behaviour reports the observation faithfully ("procedural 3 shown, semantic 2, episodic 1 in this index") — so the workbook matches the log, but the case definition's cap numbers do not match the observed listing. No defect was filed on this point.
  2. Exit-code capture artifact: the two refusal probes print the refusal message but record `exit=0` (lines 45, 48), whereas a refusal would be expected non-zero. The refusal text is verbatim; the exit lines look like the same harness capture artifact seen in TC-062. The workbook says the probes "are refused", which the printed messages support; the literal `exit=0` lines remain unexplained.
  3. The workbook Actual Behaviour cites a second persisted fragment, 'please fix assistant sure thing' (from the transcript-looking query); the log shows the transcript-looking refresh probe ran (lines 39-42) but only the email-derived fragment appears in the log's manifest excerpt. Evidence for the second fragment: Unavailable — location unknown (asserted in the workbook and consistent with defects.json's mechanism description, but not printed in this log).

Priority aspects requested by the central team:
1. **Synthetic query — evidence Available.** `prompt words: 654` (line 2); subject (Filament) in the final sentence per the workbook narrative; the tail-subject survival is proven by `.agents/skills/filament/SKILL.md — Filament` in the capsule and `TAIL_SUBJECT_FOUND` (lines 20-21). The prompt file itself (`/tmp/long-prompt.txt`-style temp input) was not preserved: Unavailable — environment no longer exists.
2. **retrieve / context / refresh results — evidence Available.** refresh with the raw email and transcript-looking queries succeeded (`exit=0`, capsule rendered, lines 34-42); retrieve and context with the same class of input printed the refusal `Task Capsule request contains private or raw data; replace it with a sanitized summary` (lines 44, 47; recorded exit codes per the artifact note above). This asymmetry is exactly DEF-006 / defects.json "TC-066" (root_cause: "Privacy check runs on the distilled term list instead of the raw prompt in the refresh path").
3. **Manifest redaction — evidence Available.** The persisted manifest query is the tokenized form `"contact customer john doe example com about"` — '@' and dots stripped by `\w+` tokenization, raw email string never persisted verbatim (log lines 50-51; same wording in defects.json actual_result).
4. **Tokenized fragments — evidence partially Available.** Email-derived fragment: Available (log line 50). Transcript-derived fragment ('please fix assistant sure thing'): Unavailable in the log — recorded only in the workbook row (see Contradiction 3).
5. **Gitignore status — evidence Available.** Manifests live under `memory-bank/local/retrieval-manifests/` (three UUID-named .json files, log lines 24-26) with `0 tracked manifest changes` (line 28) and `PROJECT_BRAIN_UNCHANGED` (line 22); the `ephemeral_manifest_stays_out_of_shared_history` test is in the passing pytest set; `memory-bank/local/` is gitignored per the TC-064 requirement-source (Laravel/.gitignore) and the workbook's "gitignored machine-local state" characterization.

#### New Reproduction
- Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run.
- No historical retries are documented in the log (workbook Retry Count 0). Note: the fix-qa-findings workflow (2026-08-03 15:31-16:28) later addressed the privacy-gate ordering (TC-066) in the uncommitted working tree; that remediation postdates and does not alter this run's evidence.

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session); native Claude Code client behavior was NOT exercised. The case's ai_tool is Claude Code and its hook framing (working-memory-read.sh assembling the capsule at prompt time, raw prompt passed from hook stdin) is native wiring; the log invokes `context.py refresh` directly. Native assertions are marked "Native assertions unavailable".

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: **Medium.** Original log complete for the core case (654-word prompt, tail-subject survival, 436-char capsule, untracked manifests, pytest 6/6) and for the defect mechanism (refusal messages plus persisted tokenized query); held below High because command lines are shorthand markers, the long prompt itself was not preserved, two exit codes carry a capture artifact, one workbook-cited fragment is not in the log, and the commit is corroborated externally.
- Evidence proves: rarity-ranked distillation preserving a tail subject; capsule budget compliance (436 <= 8000); ephemeral manifests confined to untracked local state with project-brain/ unchanged; the refresh-vs-retrieve/context privacy-gate asymmetry with tokenized (not raw) persistence — i.e. DEF-006 as filed.
- Evidence does not prove: the case-definition layer-cap numbers (observed listing disagrees — see Contradiction 1); non-zero exit on refusals; persistence of the transcript-derived fragment; budget-pressure ordering by direct probe; native prompt-time hook behavior.
- Missing information: preserved prompt text; manifest file contents beyond the quoted query line; resolution of the exit-code artifact; source of the workbook's second fragment claim.
- Recommended central-team follow-up: (a) verify the remediated ordering (fix-qa-findings) closes DEF-006 once committed; (b) reconcile CAPSULE_LAYER_LIMITS numbers between the case definition and the implementation (which of "procedural 2, semantic 3" vs "procedural 3, semantic 2" is canonical); (c) optional 30-second probe to reproduce the transcript-fragment persistence the workbook cites.

#### Artifact Inventory
- TC-066 evidence log — shareable filename `logs/original/TC-066.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:31:41 +0300 (content window 08:22:27-08:22:28 UTC) — sensitive-content review: none found (the only personal-data-like content is the deliberately synthetic tokenized fragment "john doe example com", part of the defect evidence) — included: yes.
- Case definition TC-066 — shareable filename `manifests/cases_map.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:14 — sensitive-content review: none found — included: yes.
- Defect record (workbook DEF-006) — shareable filename `manifests/defects.json` (key "TC-066") — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 15:29 — sensitive-content review: none found (synthetic example email only) — included: yes.
- Executor workflow journal — shareable filename `transcripts/wf_e8e050fd-8be.json` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: file mtime 2026-08-03 11:30 — sensitive-content review: none found — included: yes.
- Long synthetic prompt file — shareable filename: n/a — status: Unavailable — environment no longer exists (temp file in the scripted executor's scratchpad; only its word count and tail subject survive in the log) — included: no.

### RUN-044 / TC-067

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Governed)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.3
- Git commit: 3a02537 (equals current HEAD of AI-Infrastructure)
- Execution date: recorded as "2026-08-03" in the historical snapshot; the current workbook shows the mechanically coerced form "2026-08-03 00:00:00"
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-067.log`
- Row presence: the row exists in BOTH the current workbook and the historical snapshot; content is identical apart from the current workbook's mechanical rewrite (date-to-datetime coercion, six count columns int-to-float, e.g. Retry Count "2" -> "2.0", plus 40 empty padding columns "Column 31".."Column 70").
- Recorded counts: Tests Executed 4 / Passed 4 / Failed 0; Linter Result "Not Applicable"; Human Intervention 0; Retry 2; Safety Violations 0. Files Created: "2 MEM-YYYYMMDD chunks, 2 promotion JSONs, archived finding records (temp copy); evidence/TC-067.log"; Files Modified: "memory-bank/INDEX.md regenerated (temp copy)".

#### Test Definition
- Definition status: Authoritative — original source found (present in `manifests/cases_map.json` as TC-067; the historical workbook "Test Cases" sheet row TC-067 agrees on every overlapping field after whitespace normalization). Note: the CURRENT workbook's Test Cases sheet no longer contains this definition (74 -> 19 trim); it survives only in the historical snapshot and cases_map.json.
- Title: "Auto-promotion: resolved verified finding lands in the Memory Bank with a MEM-YYYYMMDD-<hex8> chunk"
- Objective: prove that on the turn flush boundary `auto_promote` promotes a resolved+verified finding into a date-based Memory Bank chunk without human review while keeping the absence of human approval visible in both stores.
- Preconditions: Laravel edition root; `project-brain/config/runtime.json` defaults `"automatic_promotion": true`, `"allowed_authority": ["observed","verified"]`, `"allowed_privacy": ["public","team"]`; a finding created, promoted to `--authority verified`, transitioned to terminal state via `brain-update --record-id <UUID> --revision auto --transition resolved --reason "Fix confirmed"`; a work branch with a pending delta so the flush boundary fires.
- Input: `python3 memory-bank/scripts/context.py turn --task-id "$(git branch --show-current)" --flush --json`
- Mandatory assertions: (1) auto_promote proposes/auto-reviews/applies on the flush boundary only; (2) chunk `memory-bank/chunks/MEM-YYYYMMDD-xxxxxxxx-<slug>.md` minted from today's UTC date + first 8 hex of the source UUID, `.memory-counter` neither read nor written; (3) chunk tags include `project-brain`, `promoted`, `auto-promoted` and the promotion JSON names no reviewer; (4) INDEX.md regenerated from frontmatter; (5) turn JSON `promoted` array lists {record_id, type, memory_id}; (6) negative control: terminal observed finding blocked under `promotion_blocked` with reason `authority`; (7) tasks are never auto-promoted.
- Optional assertions: per the definition's Notes, auto_promote also retries a stalled promotion in place instead of creating an orphaned second proposal (not exercised here).
- Scriptable assertions: `ls memory-bank/chunks/ | grep -E '^MEM-[0-9]{8}-[0-9a-f]{8}-'` matches the new chunk; `grep -l 'auto-promoted' memory-bank/chunks/MEM-*-*.md`; `python3 memory-bank/scripts/validate.py memory-bank` exits 0; the four named `project-brain/tests/test_runtime.py -k` tests pass.
- Native-client assertions: none demanded beyond the hook-layer flush; Execution Type is "Hybrid" (CLI-driven flush).
- Expected state changes: `memory-bank/chunks/MEM-<date>-<hex8>-<slug>.md` (new), `memory-bank/INDEX.md` (regenerated), `project-brain/control/promotions/<promotion-id>.json` (status applied, destination_memory_id set).
- Safety expectations: promotion apply is transactional (snapshot restore on failure); `automatic_promotion: false` disables the path; an automatic promotion can never be dressed up as human review; secret-bearing content blocks the apply.
- Requirement-source: `Laravel/memory-bank/scripts/brain_runtime.py` (auto_promote, apply_promotion — MEM id minting); `Laravel/project-brain/config/runtime.json`. Priority: Critical; Owner: QA.

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-067.log` (shareable name: `logs/original/TC-067.log`), 194 lines, byte-copy of the executor's scratchpad evidence file; in-log timestamps "=== TC-067 start Mon Aug  3 08:23:28 AM UTC 2026 ===" through "=== TC-067 retry3 end Mon Aug  3 08:24:28 AM UTC 2026 ===".
- Commands: no verbatim command lines are recorded in the log; it records section markers (e.g. `--- flush boundary ---`), exit-code lines, and captured outputs. The prescribed invocation is quoted above from cases_map.json, not from the log. The log does capture verbatim CLI usage text from three failed harness probes: `context.py brain-update: error: argument --record-id: expected one argument`.
- Exit codes recorded in the log: `exit=0` (finding updates), `neg-exit=0`, `turn-exit=0` (all four flush-boundary invocations: initial attempt and each of the three labeled retries), `create-exit=0` (retries 2 and 3), `validate-exit=0` (retries 2 and 3), plus "Memory bank validation passed." with `exit=0` in the first attempt.
- Assertions: Passed 4 / Failed 0 / Skipped 0 for the pytest verification ("4 passed, 120 deselected in 0.55s", matching the workbook's 4/4/0). Behavioural assertions (1)-(6) are directly evidenced in the log; (7) tasks-never-promoted is evidenced via the passing `automatic_promotion_excludes_tasks_and_unverified_records` test (in the -k selection) rather than a dedicated log probe. The transactional-rollback and `automatic_promotion: false` safety expectations were not exercised in this run.
- Key output excerpt (verbatim):
  ```
  promoted: [{"record_id": "d2a526d6-155c-42eb-a0f0-ebf337f3f3b0", "type": "finding", "memory_id": "MEM-20260803-d2a526d6"}, {"record_id": "dd20ac66-910b-4e0d-966b-8fbba6ceb95f", "type": "finding", "memory_id": "MEM-20260803-dd20ac66"}]
  blocked: []
  new-chunk=MEM-20260803-d2a526d6-payment-retry-policy.md  uuid8=dd20ac66  date=20260803
  counter before: 26ab0db90d72e28ad0ba1e22ee510510  memory-bank/.memory-counter
  counter after:  26ab0db90d72e28ad0ba1e22ee510510  memory-bank/.memory-counter
  8:| MEM-20260803-d2a526d6 | Payment retry policy | decision | application | project-brain, promoted, auto-promoted | active | 2026-08-03 | chunks/MEM-20260803-d2a526d6-payment-retry-policy.md |
  ```
  Both captured promotion JSONs show `"review_mode": "automatic"`, `"reviewer": null`, `"outcome": "promoted"`, `"status": "applied"` with `destination_memory_id` set.
- Contradictions with workbook: none of substance. Minor count nuance: the workbook narrates "Three attempts" with Retry Count 2, while the log contains four labeled attempt sections (initial, "RETRY with substantive content", "RETRY 2: real source paths", "RETRY 3"); the first RETRY failed harness-side (a `json.decoder.JSONDecodeError` reading /tmp/tc067c.json left `UUID=` empty, producing three brain-update usage errors) and created no new probe record, which is consistent with not counting it as a product-level attempt. Both intermediate refusals quoted in the workbook appear verbatim in the log: "record carries no content beyond its own title" and "cited source changed since the record was written: README.md".

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. Historical retries documented in the log itself: attempt 1 blocked with "record carries no content beyond its own title"; a first retry failed harness-side (JSONDecodeError + three `--record-id: expected one argument` usage errors) and the flush stayed blocked; retry 2 was blocked with "cited source changed since the record was written: README.md"; retry 3 succeeded, auto-promoting both eligible findings. The execute-test-cases journal and the workbook characterize the two counted retries as correct guard behavior ("each refusal being correct behavior", "sensible promotion guards"), not product failures.

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session, session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, executor exec:brain); native Claude Code client behavior was NOT exercised. The case's ai_tool is "Claude Code", but its assertions target the CLI flush boundary (`context.py turn --flush`), which the scripted runner drove directly in a temp copy of the Laravel edition; no assertion in this case strictly requires an interactive client, so no assertions are marked "Native assertions unavailable".

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium — the original log captures the decisive artifacts verbatim (promotion JSONs with reviewer null/review_mode automatic, chunk frontmatter tags, INDEX row, counter md5 stability, promoted/blocked arrays, all exit codes), and the commit (3a02537, clean tree) is established batch-wide by TC-001.log and the workflow journal; but complete verbatim command lines are not recorded in the log, and the attempt-count bookkeeping (4 log sections vs "three attempts"/Retry Count 2) requires the digest's explanation.
- Evidence proves: flush-boundary auto-promotion of resolved+verified findings without a reviewer; conflict-free MEM-YYYYMMDD-<uuid8> id minting with the legacy counter untouched; visible "auto-promoted" labeling in chunk tags and promotion JSONs; INDEX regeneration; authority-based blocking of the observed negative control; two additional (undocumented-in-the-case but correct) promotion guards: title-only content and cited-source freshness.
- Evidence does not prove: transactional rollback on write failure, behavior with `automatic_promotion: false`, or the exact command strings used by the harness.
- Missing information: verbatim commands; explicit log probe for "tasks never promoted" (covered only via pytest).
- Recommended central-team follow-up: none for the product; consider documenting the two extra promotion guards (content-beyond-title, source-fingerprint freshness) in the case catalog, as the workbook comment suggests.

#### Artifact Inventory
- TC-067 evidence log — shareable filename `logs/original/TC-067.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found (synthetic retry-policy content only) — included: yes.
- Workbook rows (current + historical snapshot exports: `our-runs.json`, `historical-runs.json`) — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: workbook snapshot 2026-08-03 19:56 local (historical), current workbook as delivered 2026-08-04 — sensitive-content review: none found — included: yes.

### RUN-045 / TC-068

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Governed)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.3
- Git commit: 3a02537 (equals current HEAD of AI-Infrastructure)
- Execution date: recorded as "2026-08-03" in the historical snapshot; the current workbook shows the mechanically coerced form "2026-08-03 00:00:00"
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-068.log`
- Row presence: the row exists in BOTH the current workbook and the historical snapshot; content is identical apart from the current workbook's mechanical rewrite (date-to-datetime coercion, six count columns int-to-float, e.g. Tests Executed "1" -> "1.0", plus 40 empty padding columns "Column 31".."Column 70").
- Recorded counts: Tests Executed 1 / Passed 1 / Failed 0; Linter Result "Not Applicable"; Human Intervention 0; Retry 0; Safety Violations 0. Files Modified: "memory-bank/INDEX.md (deleted and deterministically reconstructed, net-zero diff; temp copy)".

#### Test Definition
- Definition status: Authoritative — original source found (present in `manifests/cases_map.json` as TC-068; the historical workbook "Test Cases" sheet row TC-068 agrees on every overlapping field after whitespace normalization). Note: the CURRENT workbook's Test Cases sheet no longer contains this definition (74 -> 19 trim); it survives only in the historical snapshot and cases_map.json.
- Title: "reindex-bank is idempotent and reconstructs a deleted INDEX.md"
- Objective: prove that `context.py reindex-bank` is idempotent, deterministically reconstructs a deleted INDEX.md from chunk frontmatter, preserves hand-written preambles, and never touches chunks or the legacy `.memory-counter`.
- Preconditions: Laravel edition root with at least the shipped chunk `memory-bank/chunks/MEM-0001-cross-edition-sync.md` and a current committed `memory-bank/INDEX.md`; clean git status so drift is observable.
- Input: `python3 memory-bank/scripts/context.py reindex-bank --json; rm memory-bank/INDEX.md; python3 memory-bank/scripts/context.py reindex-bank --json; python3 memory-bank/scripts/context.py reindex-bank --json`
- Mandatory assertions: (1) first run reports `"changed": false` and rewrites nothing; (2) after deleting INDEX.md, the second run reconstructs it fully and reports `"changed": true` with the chunk count; (3) the third run reports `"changed": false` again (deterministic); (4) a hand-written preamble above the table header is preserved verbatim; (5) `.memory-counter`, if present, is neither read nor written; (6) `git diff` shows no net change to INDEX.md afterwards.
- Optional assertions: per the definition's Notes, reindex-bank replaces every manual "update INDEX.md and increment .memory-counter" step from the legacy workflow (contextual, not separately asserted).
- Scriptable assertions: `git diff --exit-code -- memory-bank/INDEX.md` exits 0 after the sequence; second run JSON contains `"changed": true`, third `"changed": false`; `python3 -m pytest project-brain/tests/test_runtime.py -k reindex_bank_is_idempotent_and_restores_a_deleted_index` passes.
- Native-client assertions: none — Execution Type is "Automated"; ai_tool is "Cross-tool" (pure CLI case).
- Expected state changes: `memory-bank/INDEX.md` (deleted then reconstructed byte-identically); nothing else.
- Safety expectations: no chunk file modified — INDEX.md is the only write target and goes through the mutation lock + atomic write; rows never hand-appended; no `.memory-counter` mutation.
- Requirement-source: `Laravel/memory-bank/scripts/brain_runtime.py` (reindex_bank docstring); `Laravel/.claude/skills/memory-bank/SKILL.md` (step 6: regenerate the index with reindex-bank, do not hand-edit INDEX.md rows). Priority: High; Owner: QA.

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-068.log` (shareable name: `logs/original/TC-068.log`), 27 lines, byte-copy of the executor's scratchpad evidence file; in-log timestamps "=== TC-068 start Mon Aug  3 08:24:57 AM UTC 2026 ===" / "=== TC-068 end Mon Aug  3 08:24:57 AM UTC 2026 ===".
- Commands: the log records paraphrased command markers, not full command lines: `$ run 1 (up-to-date index)`, `$ rm INDEX.md; run 2`, `$ run 3`, `$ git diff INDEX.md`. The full invocation is quoted above from cases_map.json.
- Exit codes recorded in the log: `exit=0` after each of the three reindex runs and after the preamble regeneration; `git-diff-exit=0`.
- Assertions: Passed 1 / Failed 0 / Skipped 0 for the pytest verification ("1 passed, 123 deselected in 0.21s", matching the workbook's 1/1/0). All six behavioural assertions are directly evidenced in the log: idempotence, reconstruction (`"chunks": 3, "changed": true`), determinism, preamble survival ("Hand-written preamble line kept verbatim."), counter stability ("counter unchanged: yes"), and net-zero git diff. Chunk immutability is additionally evidenced by "chunks md5 unchanged: yes".
- Key output excerpt (verbatim):
  ```
  $ run 1 (up-to-date index)
  {"index": "memory-bank/INDEX.md", "chunks": 3, "changed": false}
  $ rm INDEX.md; run 2
  {"index": "memory-bank/INDEX.md", "chunks": 3, "changed": true}
  $ run 3
  {"index": "memory-bank/INDEX.md", "chunks": 3, "changed": false}
  git-diff-exit=0
  ```
- Contradictions with workbook: none. Note the bank at run time held 3 chunks (MEM-0001 plus the two chunks auto-promoted by the preceding TC-067 run in the same shared temp copy), which the workbook row reflects ("a 3-chunk bank committed clean"); the case precondition required only "at least" the shipped chunk, so this is compliant.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. The log documents no historical retries (digest: Retries 0, first attempt Yes).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session, session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, executor exec:brain). The case's ai_tool is "Cross-tool" and its Execution Type is "Automated": it targets the CLI directly, so no native-client assertion exists for this case.

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium — the original log carries every decisive JSON output and exit code and the commit (3a02537, clean tree) is established batch-wide by TC-001.log and the workflow journal, but the log records paraphrased command markers rather than complete verbatim command lines.
- Evidence proves: idempotence (changed=false on an up-to-date index), full deterministic reconstruction of a deleted INDEX.md from chunk frontmatter (changed=true, chunks=3, then changed=false), byte-level net-zero drift (`git-diff-exit=0`), preamble preservation, and non-mutation of chunks and `.memory-counter`.
- Evidence does not prove: the mutation-lock/atomic-write mechanics themselves (asserted structurally by the safety expectation, not probed here) or the literal command strings used.
- Missing information: verbatim commands.
- Recommended central-team follow-up: none.

#### Artifact Inventory
- TC-068 evidence log — shareable filename `logs/original/TC-068.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found — included: yes.
- Workbook rows (current + historical snapshot exports: `our-runs.json`, `historical-runs.json`) — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: workbook snapshot 2026-08-03 19:56 local (historical), current workbook as delivered 2026-08-04 — sensitive-content review: none found — included: yes.

### RUN-046 / TC-069

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Governed)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.3
- Git commit: 3a02537 (equals current HEAD of AI-Infrastructure)
- Execution date: recorded as "2026-08-03" in the historical snapshot; the current workbook shows the mechanically coerced form "2026-08-03 00:00:00"
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-069.log`
- Row presence: the row exists in BOTH the current workbook and the historical snapshot; content is identical apart from the current workbook's mechanical rewrite (date-to-datetime coercion, six count columns int-to-float, e.g. Tests Executed "19" -> "19.0", plus 40 empty padding columns "Column 31".."Column 70").
- Recorded counts: Tests Executed 19 / Passed 19 / Failed 0; Linter Result "Not Applicable"; Human Intervention 0; Retry 0; Safety Violations 0. Files Created: "1 valid chunk kept in temp copy; transient negative-control chunks (created and removed); evidence/TC-069.log"; Files Modified: "memory-bank/INDEX.md reindexed (temp copy)".

#### Test Definition
- Definition status: Authoritative — original source found (present in `manifests/cases_map.json` as TC-069; the historical workbook "Test Cases" sheet row TC-069 agrees on every overlapping field after whitespace normalization). Note: the CURRENT workbook's Test Cases sheet no longer contains this definition (74 -> 19 trim); it survives only in the historical snapshot and cases_map.json.
- Title: "validate.py accepts a mixed bank of legacy MEM-NNNN and date-based MEM-YYYYMMDD-hex chunks"
- Objective: prove the Memory Bank validator accepts both chunk ID formats, rejects malformed names/ids/keys, detects secrets without echoing them, and ignores the retired `.memory-counter`.
- Preconditions: Laravel edition root; bank contains legacy `MEM-0001-cross-edition-sync.md` plus a new `MEM-20260803-1a2b3c4d-payment-retry-policy.md` built from the shipped template with matching frontmatter id and all REQUIRED_KEYS; `reindex-bank` run so INDEX.md lists both.
- Input: `python3 memory-bank/scripts/validate.py memory-bank; python3 memory-bank/scripts/validate.py memory-bank --summary`
- Mandatory assertions: (1) both ID formats accepted via ID_PATTERN `^MEM-(?:\d{8}-[0-9a-f]{8}|\d{4,})$` and the lowercase-slug filename pattern, exit 0; (2) `--summary` prints status counts without chunk contents; (3) negative controls fail naming the offending file: `MEM-123-short.md`, `MEM-20260803-XYZ-bad.md`, frontmatter id/filename mismatch, SECRET_PATTERNS content; (4) an on-disk `.memory-counter` is deliberately ignored; (5) valid_from/valid_to accepted, unknown extra keys rejected.
- Optional assertions: per the definition's Notes, validate.py takes the bank directory as an optional positional argument (exercised implicitly by the `memory-bank` argument).
- Scriptable assertions: `echo $?` is 0 for the mixed bank; negative-control runs exit non-zero naming the bad filename without echoing secret values; `python3 -m pytest memory-bank/tests/test_validate.py` passes; `python3 -m pytest project-brain/tests/test_runtime.py -k knowledge_past_its_validity_may_not_call_itself_active` passes.
- Native-client assertions: none — Execution Type is "Automated"; ai_tool is "Cross-tool" (pure CLI case).
- Expected state changes: none (validator is read-only).
- Safety expectations: the validator must never modify, rename, or delete chunks; existing MEM-NNNN chunks keep their IDs forever; secret-pattern failures must not print the matched secret.
- Requirement-source: `Laravel/memory-bank/scripts/validate.py` (ID_PATTERN, FILENAME_PATTERN, REQUIRED_KEYS, SECRET_PATTERNS comments); `Laravel/.claude/skills/memory-bank/SKILL.md` (chunk ID minting rules). Priority: High; Owner: QA.

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-069.log` (shareable name: `logs/original/TC-069.log`), 48 lines, byte-copy of the executor's scratchpad evidence file; in-log timestamps "=== TC-069 start Mon Aug  3 08:25:24 AM UTC 2026 ===" / "=== TC-069 end Mon Aug  3 08:25:26 AM UTC 2026 ===", followed by additional labeled probe sections ("secret control redo", "secret control redo 2").
- Commands: the log records paraphrased command markers, not full command lines: `$ validate mixed bank`, `$ validate --summary`, `$ id/filename mismatch`, `$ secret content`, `$ unknown extra key`. The prescribed invocation is quoted above from cases_map.json.
- Exit codes recorded in the log: `exit=0` (mixed-bank validation, --summary, post-cleanup validation, final cleanup); `grep-exit=0`; `MEM-123-short exit=1`; `validate-exit=1` (both secret-control redos); `final-exit=0`. Note: an `exit=0` line also appears directly after the first MEM-123-short rejection message; the log does not record which command that exit code belongs to (attribution Unknown), while the later explicit `MEM-123-short exit=1` records the validator's own exit for that negative control.
- Assertions: Passed 19 / Failed 0 / Skipped 0 per workbook and digest (19/19/0 = "18 passed, 3 subtests passed in 0.82s" for memory-bank/tests/test_validate.py plus "1 passed, 123 deselected in 0.23s" for the -k runtime test). Behavioural assertions (1), (2), (3), (5, unknown-key half) directly evidenced. Assertion (4) (`.memory-counter` ignored) and the valid_from/valid_to half of (5) are stated in the workbook row but not itemized in the log (the created chunk's frontmatter is not reproduced there); log evidence for them: Unknown.
- Key output excerpt (verbatim):
  ```
  Memory bank: 4 chunks (4 active, 0 needs review).
  - /tmp/claude-1000/-home-aliaksei-Desktop-AI-Infrastructure/c9fab6de-739d-497c-ac0d-61bda631def9/scratchpad/exec-brain/memory-bank/chunks/MEM-20260803-4d4d4d4d-extrakey.md: unexpected metadata keys: made_up_key
  --- secret control redo 2 (indexed, canonical AWS example key) ---
  validate-exit=1
  Memory bank validation failed (2 error(s)):
  - /tmp/claude-1000/-home-aliaksei-Desktop-AI-Infrastructure/c9fab6de-739d-497c-ac0d-61bda631def9/scratchpad/exec-brain/memory-bank/chunks/MEM-20260803-3c3c3c3c-secretive.md: possible AWS access key detected; value intentionally not printed
  secret value echoed? 0
  ```
- Contradictions with workbook: none. The workbook comment itself discloses that "Two secret-control iterations were needed to isolate the secret check (the first collided with the id-mismatch and missing-from-INDEX checks, which fire first) — validator layering, not a defect", which matches the log's two redo sections; Retry Count 0 is consistent with these being probe refinements inside one attempt rather than case-level retries.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. Historical in-run probe iterations documented in the log: the first secret control tripped the earlier-firing id-mismatch check ("frontmatter id does not match filename id"); "secret control redo" then tripped "chunk is missing from INDEX.md"; "secret control redo 2 (indexed, canonical AWS example key)" finally isolated the secret detector ("possible AWS access key detected; value intentionally not printed", `validate-exit=1`). In every iteration the log records the secret value appearing 0 times in output.

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session, session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, executor exec:brain). The case's ai_tool is "Cross-tool" and its Execution Type is "Automated": it targets validate.py directly, so no native-client assertion exists for this case.

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium — the original log carries the decisive validator outputs, rejection messages, and exit codes, and the commit (3a02537, clean tree) is established batch-wide by TC-001.log and the workflow journal; but complete verbatim command lines are absent, one early exit-code line has unrecorded attribution, and two definition points (`.memory-counter` ignored; valid_from/valid_to accepted) rest on the workbook narrative rather than log lines.
- Evidence proves: mixed-bank acceptance of both ID formats (exit 0); contents-free `--summary`; rejection of malformed filenames, id/filename mismatches, and unknown frontmatter keys with the offending file named; AWS-key detection that deliberately withholds the value (echo count 0); restoration of a clean passing bank afterwards.
- Evidence does not prove (from the log alone): that `.memory-counter` was present and ignored; that valid_from/valid_to keys were present in the accepted chunk; the literal command strings used.
- Missing information: verbatim commands; frontmatter dump of the created chunk; attribution of the early `exit=0`/`grep-exit=0` lines.
- Recommended central-team follow-up: none for the product; if re-run, capture the accepted chunk's frontmatter and an explicit `.memory-counter` presence probe in the log.

#### Artifact Inventory
- TC-069 evidence log — shareable filename `logs/original/TC-069.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found (the planted AWS-format control key never appears; the log records "secret value echoed? 0") — included: yes.
- Workbook rows (current + historical snapshot exports: `our-runs.json`, `historical-runs.json`) — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: workbook snapshot 2026-08-03 19:56 local (historical), current workbook as delivered 2026-08-04 — sensitive-content review: none found — included: yes.

### RUN-047 / TC-070

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Cross-tool (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Governed)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.3
- Git commit: 3a02537 (equals current HEAD of AI-Infrastructure)
- Execution date: recorded as "2026-08-03" in the historical snapshot; the current workbook shows the mechanically coerced form "2026-08-03 00:00:00"
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-070.log`
- Row presence: the row exists in BOTH the current workbook and the historical snapshot; content is identical apart from the current workbook's mechanical rewrite (date-to-datetime coercion, six count columns int-to-float, e.g. Tests Executed "4" -> "4.0", plus 40 empty padding columns "Column 31".."Column 70").
- Recorded counts: Tests Executed 4 / Passed 4 / Failed 0; Linter Result "Not Applicable"; Human Intervention 0; Retry 0; Safety Violations 0. Files Created: "4 finding records (temp copy); evidence/TC-070.log"; Files Modified: "one record's updated_at aged for the stale simulation (temp copy only)".

#### Test Definition
- Definition status: Authoritative — original source found (present in `manifests/cases_map.json` as TC-070; the historical workbook "Test Cases" sheet row TC-070 agrees on every overlapping field after whitespace normalization). Note: the CURRENT workbook's Test Cases sheet no longer contains this definition (74 -> 19 trim); it survives only in the historical snapshot and cases_map.json.
- Title: "Retrieval ranking: a verified record outranks an observed one and a fresh record outranks a stale one at equal lexical fit"
- Objective: prove that governed retrieval scales BM25 lexical fit with provenance multipliers (authority, recency, confidence) so verified beats observed and fresh beats stale at equal lexical fit, without hiding floored records.
- Preconditions: Laravel edition root, governed task bound. Pair A: two findings with near-identical titles/content on one distinctive topic, one observed, one verified. Pair B: two same-authority records on a second topic, one with updated_at ~60+ days old, one current. Index refreshed.
- Input: `python3 memory-bank/scripts/context.py retrieve "<pair-A topic term>" --task-id "$(git branch --show-current)" --json  # then repeat with the pair-B topic term`
- Mandatory assertions: (1) AUTHORITY_WEIGHTS verified=1.0 vs observed=0.85 puts the verified record first at equal lexical fit; (2) RECENCY_HALF_LIFE_DAYS=30 with RECENCY_WEIGHT_FLOOR=0.5 puts the fresh record first while the stale one still appears (scaled, never hidden); (3) confidence floored at 0.7 the same way; (4) BM25 column weights (title 2.0, summary 8.0) favour self-declared subject matter, and Porter stemming lets 'review' match 'Reviewer'; (5) ordering visible in the JSON result's dynamic/durable sections.
- Optional assertions: per the definition's Notes, PROTOCOL.md (~line 92) — privacy, owner, authority, lifecycle, and freshness filters still win over any ranking multiplier (suite-level property).
- Scriptable assertions: in each JSON result the expected record's path appears before its twin; the four named `project-brain/tests/test_runtime.py -k` tests pass (verified-vs-observed, fresh-vs-stale, stemming, focused-document survival).
- Native-client assertions: none demanded — Execution Type is "Hybrid"; ai_tool is "Cross-tool" (CLI retrieval).
- Expected state changes: none (retrieval is read-only apart from the manifest and local index).
- Safety expectations: privacy filters win over ranking — private/restricted records never enter the FTS index or results regardless of score; ranking multipliers must not resurrect stale-source-filtered documents.
- Requirement-source: `Laravel/memory-bank/scripts/context_retrieval.py` (AUTHORITY_WEIGHTS, RECENCY_HALF_LIFE_DAYS, RECENCY_WEIGHT_FLOOR, CONFIDENCE_WEIGHT_FLOOR, BM25_WEIGHTS comments). Priority: Medium; Owner: QA.

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-070.log` (shareable name: `logs/original/TC-070.log`), 23 lines, byte-copy of the executor's scratchpad evidence file; in-log timestamps "=== TC-070 start Mon Aug  3 08:26:29 AM UTC 2026 ===" / "=== TC-070 end Mon Aug  3 08:26:31 AM UTC 2026 ===".
- Commands: no verbatim command lines are recorded in the log; it records section markers (e.g. `--- query pair A ---`) and captured outputs. The prescribed invocation is quoted above from cases_map.json.
- Exit codes recorded in the log: `index-exit=0`; `exit=0` (query pair A); `exit=0` (query pair B).
- Assertions: Passed 4 / Failed 0 / Skipped 0 for the pytest verification ("4 passed, 120 deselected in 0.77s", matching the workbook's 4/4/0). Behavioural assertions (1), (2), (4, stemming half), (5) directly evidenced in the log. Assertion (3) (confidence floor 0.7) and the title/summary BM25-weighting half of (4) are not probed in this log (spec/suite-level properties); the privacy-over-ranking safety expectation likewise rests on the test suite per the workbook comment, not on a log probe.
- Key output excerpt (verbatim):
  ```
  aged updated_at -> 2026-05-25T08:26:30.013460+00:00
  finding order: ['project-brain/dynamic/findings/dfae029f-9ce9-4868-9781-a7a4cb36fd64.md', 'project-brain/dynamic/findings/8f48474a-d67b-451c-9b8e-546fe53a9b75.md']
  observed idx 1 verified idx 0 -> verified first: True
  finding order: ['project-brain/dynamic/findings/a654ee8b-bcb8-42b9-bd85-ab1da7b1d7cd.md', 'project-brain/dynamic/findings/d297eaba-96af-42b4-a70b-7d51318f3867.md']
  stale idx 1 fresh idx 0 -> fresh first: True ; stale still present: True
  procedural skill: .agents/skills/review-pr/SKILL.md — PR Reviewer
  ```
- Contradictions with workbook: none. The workbook's "aged to updated_at -70 days" matches the log's "--- age B1 updated_at by 70 days (test simulation, temp copy only) ---" and the resulting 2026-05-25 timestamp; the workbook's stemming claim (query 'review' matched 'PR Reviewer') matches the log's stemming lines including `flow-alternatives: [coder, code-[reviewer], finishing-branch]`.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. The log documents no historical retries (digest: Retries 0, first attempt Yes).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session, session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, executor exec:brain). The case's ai_tool is "Cross-tool": it targets the retrieval CLI directly, so no native-client assertion exists for this case.

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium — the original log shows the exact orderings (record paths, indices), the aging simulation timestamp, exit codes, and the passing 4-test verification, and the commit (3a02537, clean tree) is established batch-wide by TC-001.log and the workflow journal; but complete verbatim command lines and the raw JSON result sections are not reproduced in the log.
- Evidence proves: authority multiplier ordering (verified before observed at equal lexical fit), recency ordering with floored-not-hidden stale records, Porter stemming ('review' matching 'Reviewer'), and that ordering is observable in the retrieval results.
- Evidence does not prove (from the log alone): the confidence floor (0.7), the specific BM25 column weights, or the privacy-over-ranking property — these rest on source constants and the wider test suite.
- Missing information: verbatim commands; raw JSON result excerpts.
- Recommended central-team follow-up: none.

#### Artifact Inventory
- TC-070 evidence log — shareable filename `logs/original/TC-070.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found (synthetic topic records only) — included: yes.
- Workbook rows (current + historical snapshot exports: `our-runs.json`, `historical-runs.json`) — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: workbook snapshot 2026-08-03 19:56 local (historical), current workbook as delivered 2026-08-04 — sensitive-content review: none found — included: yes.

### RUN-048 / TC-071

#### Workbook Record
- Recorded result: Pass
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Cursor (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Governed)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.3
- Git commit: 3a02537 (equals current HEAD of AI-Infrastructure)
- Execution date: recorded as "2026-08-03" in the historical snapshot; the current workbook shows the mechanically coerced form "2026-08-03 00:00:00"
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-071.log`
- Row presence: the row exists in BOTH the current workbook and the historical snapshot; content is identical apart from the current workbook's mechanical rewrite (date-to-datetime coercion, six count columns int-to-float, e.g. Tests Executed "26" -> "26.0", plus 40 empty padding columns "Column 31".."Column 70").
- Recorded counts: Tests Executed 26 / Passed 26 / Failed 0; Linter Result "Not Applicable"; Human Intervention 0; Retry 0; Safety Violations 0. Files Created: ".cursor/rules/working-memory.mdc (gitignored, temp copy); evidence/TC-071.log"; Files Modified: "spec-desc.md test delta (temp copy, reverted)".

#### Test Definition
- Definition status: Authoritative — original source found (present in `manifests/cases_map.json` as TC-071; the historical workbook "Test Cases" sheet row TC-071 agrees on every overlapping field after whitespace normalization). Note: the CURRENT workbook's Test Cases sheet no longer contains this definition (74 -> 19 trim); it survives only in the historical snapshot and cases_map.json.
- Title: "Cursor receives the Task Capsule as a gitignored alwaysApply rule with a self-declared staleness header"
- Objective: prove that Cursor's Stop hook (`.cursor/hooks/working-memory-write.sh`) checkpoints the turn, renders the Task Capsule into `.cursor/rules/working-memory.mdc` (alwaysApply, gitignored, self-declared one-turn staleness), never fails the turn, and leaves the previous rule intact on a failed render.
- Preconditions: Laravel edition root on a work branch with a governed task and a pending file delta; `.cursor/rules/working-memory.mdc` absent. Note: Cursor has no UserPromptSubmit-equivalent event, so `working-memory-read.sh` is deliberately NOT shipped in `.cursor/hooks/` — the read path is served by the write hook after the turn checkpoint.
- Input: `bash .cursor/hooks/working-memory-write.sh  # simulates Cursor's Stop hook firing at end of turn`
- Mandatory assertions: (1) the hook first runs the standard turn checkpoint (`context.py turn --task-id <branch> --flush-after 5`) under CONTEXT_HOOK_BUDGET (default 5s); (2) it renders the freshest capsule via `context.py context "$TASK_ID" --task-id "$TASK_ID" --ephemeral` and writes the .mdc atomically (mktemp + mv) with frontmatter `description: Working memory - Task Capsule as of end of previous turn` and `alwaysApply: true`; (3) the body self-declares staleness ("Task Capsule as of end of previous turn (task: <id>, rendered: <UTC timestamp>)" plus "Retrieved context is not authoritative - verify the source."); (4) the file is ignored local state per Laravel/.gitignore and a failed render leaves the previous rule intact; (5) the hook always exits 0.
- Optional assertions: per the definition's Notes, verification inside a real Cursor session that the rule is attached to the next prompt (alwaysApply) — a native-client item.
- Scriptable assertions: `grep -q 'alwaysApply: true' .cursor/rules/working-memory.mdc`; `grep -q 'as of end of previous turn' ...`; `git check-ignore .cursor/rules/working-memory.mdc` exits 0; hook `echo $?` is 0; `python3 -m pytest memory-bank/tests/test_hooks.py` passes; `python3 memory-bank/scripts/context.py parity` exits 0.
- Native-client assertions: real Cursor session attaching the rendered rule to the next prompt (alwaysApply behavior in the client). Execution Type is "Manual".
- Expected state changes: `.cursor/rules/working-memory.mdc` (rendered, gitignored); `memory-bank/local/context.db` and last-turn-report.json updated by the checkpoint.
- Safety expectations: the rendered rule contains no secrets, raw prompts, or personal data (capsule filters run upstream in context.py); the hook must never block or fail Cursor's turn (exit 0 even on timeout), must not commit the .mdc, and must not write outside `.cursor/rules/` and `memory-bank/local/`.
- Requirement-source: `Laravel/.cursor/hooks/working-memory-write.sh` (capsule-delivery block); `Laravel/.gitignore` (.cursor/rules/working-memory.mdc entry); `docs/TOOL-INTEGRATIONS.md` (capsule delivery matrix: Cursor = "yes, one turn stale"). Priority: High; Owner: QA.

#### Historical Evidence
- Evidence status (main log): Available — original.
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-071.log` (shareable name: `logs/original/TC-071.log`), 37 lines, byte-copy of the executor's scratchpad evidence file; in-log timestamps "=== TC-071 start Mon Aug  3 08:26:45 AM UTC 2026 ===" / "=== TC-071 end Mon Aug  3 08:26:50 AM UTC 2026 ===".
- Commands: one verbatim command line is recorded: `$ bash .cursor/hooks/working-memory-write.sh` (identical to the case's agent_command). Other steps are recorded as section markers and sentinel lines.
- Exit codes recorded in the log: `hook-exit=0`; `hook-exit-under-timeout=0`; `parity-exit=0`. Also recorded: `read hook shipped for cursor? 0` (a match count proving working-memory-read.sh is absent from `.cursor/hooks/`, consistent with the shipped file listing at the top of the log).
- Assertions: Passed 26 / Failed 0 / Skipped 0 for the pytest verification ("26 passed, 112 subtests passed in 4.59s", matching the workbook's 26/26/0), plus "Mirror parity passed (.agents canonical)." Behavioural assertions (2)-(5) directly evidenced via the rendered rule text and sentinels ALWAYSAPPLY_OK, STALENESS_OK, NOT_AUTHORITATIVE_OK, GITIGNORED_OK, PREVIOUS_RULE_INTACT. Assertion (1) (checkpoint runs first) is evidenced indirectly: the rendered capsule body shows the governed task line ("working: feature/manual-progress — Refactor checkout"); the checkpoint invocation itself is not separately echoed. The timeout-control budget value (CONTEXT_HOOK_BUDGET=0.001) appears in the workbook row, not in the log, which records only `hook-exit-under-timeout=0` and `PREVIOUS_RULE_INTACT`. The native-client assertion (rule attached to the next Cursor prompt) was not exercised: Native assertions unavailable.
- Key output excerpt (verbatim):
  ```
  $ bash .cursor/hooks/working-memory-write.sh
  hook-exit=0
  description: Working memory - Task Capsule as of end of previous turn
  alwaysApply: true
  Task Capsule as of end of previous turn (task: feature/manual-progress, rendered: 2026-08-03T08:26:45Z).
  Retrieved context is not authoritative - verify the source.
  GITIGNORED_OK
  PREVIOUS_RULE_INTACT
  ```
- Contradictions with workbook: none. The workbook comment "Executed as a real CLI invocation of the Cursor Stop hook in the temp copy (case is labeled Manual but is fully scriptable)" accurately reflects the log.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. The log documents no historical retries (digest: Retries 0, first attempt Yes).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session, session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, executor exec:brain); native Cursor client behavior was NOT exercised. The case's ai_tool is "Cursor" and its Execution Type is "Manual": the assertion that a real Cursor session attaches the rendered alwaysApply rule to the next prompt is marked "Native assertions unavailable". Everything below that layer (the Stop hook as a CLI contract, the rendered rule file, gitignore status, timeout resilience, hooks test suite, mirror parity) was fully exercised.

#### Evidence Assessment
- Primary classification: original-log-backed.
- Confidence: Medium — the original log includes the verbatim hook invocation, the rendered rule's frontmatter and staleness body, all exit codes, the timeout control, and the full hooks-suite and parity results, and the commit (3a02537, clean tree) is established batch-wide by TC-001.log and the workflow journal; it is not High because the native Cursor attachment behavior is unverifiable in scripted mode and the timeout-forcing budget value is recorded only in the workbook.
- Evidence proves: the read-path asymmetry (no working-memory-read.sh shipped, by design); successful render of `.cursor/rules/working-memory.mdc` with `alwaysApply: true` and the self-declared staleness/non-authority text; gitignored status; fail-safe behavior (exit 0 under a forced render timeout with the previous rule left byte-identical per the workbook's md5 claim and the log's PREVIOUS_RULE_INTACT sentinel); 26 hook tests + 112 subtests passing and Cursor/Claude mirror parity.
- Evidence does not prove: that a live Cursor client attaches the rule to the next prompt; the exact checkpoint command executed inside the hook; the numeric budget used for the timeout control (workbook-only).
- Missing information: native Cursor session evidence; in-log echo of the internal checkpoint invocation and budget value.
- Recommended central-team follow-up: if native proof is required, repeat the case once inside a licensed Cursor client and capture the next-prompt rule attachment.

#### Artifact Inventory
- TC-071 evidence log — shareable filename `logs/original/TC-071.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found (rendered capsule contains only the synthetic task line "feature/manual-progress — Refactor checkout") — included: yes.
- Workbook rows (current + historical snapshot exports: `our-runs.json`, `historical-runs.json`) — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: workbook snapshot 2026-08-03 19:56 local (historical), current workbook as delivered 2026-08-04 — sensitive-content review: none found — included: yes.

### RUN-049 / TC-072

#### Workbook Record
- Recorded result: Partial
- Framework: Laravel (Framework Version: "n/a (edition workspace)")
- AI tool: Claude Code (AI Tool Version: "n/a — scripted run (no AI client)"; Model: n/a; Operating Mode: Governed)
- Tester: Claude Fable 5 (scripted execution)
- Accelerator version: 1.4.3
- Git commit: 3a02537 (equals current HEAD of AI-Infrastructure)
- Execution date: recorded as "2026-08-03" in the historical snapshot; the current workbook shows the mechanically coerced form "2026-08-03 00:00:00"
- Defect reference: none (no defect filed for TC-072; defects.json has no TC-072 key)
- Workbook evidence link: `Accelerator-TestEvidence/TC-072.log`
- Row presence: the row exists in BOTH the current workbook and the historical snapshot; content is identical apart from the current workbook's mechanical rewrite (date-to-datetime coercion, six count columns int-to-float, e.g. Tests Executed "3" -> "3.0", plus 40 empty padding columns "Column 31".."Column 70").
- Recorded counts: Tests Executed 3 / Passed 3 / Failed 0; Linter Result "Not Applicable"; Human Intervention 0; Retry 0; Safety Violations 0. Files Created: "evidence/TC-072.log"; Files Modified: "None tracked". Recorded Failure Reason (verbatim): "In-agent execution of /checkpoint and /checkpoint some-argument and /memory (behaviors 1-3 as conversational flows) requires an interactive AI coding session that this runner cannot drive; only the command/skill prescriptions and the underlying CLI contract (validate/refresh/status, governed authority, no tracked-file mutation) were verified."

#### Test Definition
- Definition status: Authoritative — original source found (present in `manifests/cases_map.json` as TC-072; the historical workbook "Test Cases" sheet row TC-072 agrees on every overlapping field after whitespace normalization). Note: the CURRENT workbook's Test Cases sheet no longer contains this definition (74 -> 19 trim); it survives only in the historical snapshot and cases_map.json.
- Title: "Unified /memory refresh reports all context layers and /checkpoint defers to governed authority"
- Objective: prove that the in-agent commands /checkpoint and /memory defer to governed Project Brain authority: /checkpoint refuses arguments and never invents a branch task or mutates SQLite working state; /memory validates, refreshes, and reports all context layers exactly as the CLI returns them, without completing tasks, promoting memory, or editing tracked files.
- Preconditions: Laravel workspace with accelerator; `project-brain/config/runtime.json` has `"mode": "governed"`; at least one uncommitted change on a feature branch; SQLite index exists under `memory-bank/local/context.db` (or is created by refresh).
- Input: `/checkpoint    (then, as separate turns)    /checkpoint some-argument    and    /memory`
- Mandatory assertions: (1) /checkpoint in governed mode passes the authority gate (`git rev-parse --show-toplevel`), reads runtime.json, reports "working: skipped" with an actionable instruction, does NOT derive a task from the branch and does NOT mutate SQLite working state; (2) /checkpoint some-argument stops and asks the user to invoke /checkpoint without arguments; (3) /memory treats Project Brain as the only task authority, runs `python3 memory-bank/scripts/context.py validate`, sets working: governed, runs `python3 memory-bank/scripts/context.py refresh --json` (the same command the request hook runs) and reports procedural/semantic/episodic exactly as returned, then runs `python3 memory-bank/scripts/context.py status --json` and reports mode, index health/counts, Brain validation, and the four statuses; (4) neither command completes tasks, promotes memory, or edits Git-tracked files.
- Optional assertions: per the definition's Notes, in lightweight mode (explicitly configured only) /memory executes the checkpoint SKILL.md workflow (not applicable here — governed mode).
- Scriptable assertions: `python3 memory-bank/scripts/context.py status --json` contains `"mode": "governed"`, `"authority": "project-brain"`, a layers object with procedural/semantic/episodic counts, and `"working": 0` (no branch task invented); `git status --porcelain` unchanged before vs after; `python3 memory-bank/scripts/context.py validate` exits 0.
- Native-client assertions: behaviors (1)-(3) as conversational flows — the argument refusal dialogue, the "working: skipped" report, and the layer report — require an interactive AI coding session executing the slash commands.
- Expected state changes: none in the Git-tracked tree; only ignored local index state (`memory-bank/local/context.db`) may change.
- Safety expectations: MUST NOT run complete, record, or clear; MUST NOT pass `--query` to refresh (no retrieval manifest); MUST NOT create or edit Project Brain records, Memory Bank chunks, changelogs, or specs; MUST NOT stage, commit, or discard changes; MUST NOT create a second task authority in governed mode; a failed layer must be named, and a successful lightweight checkpoint must not be rolled back because a layer failed.
- Requirement-source: `Laravel/.agents/skills/memory/SKILL.md` and `Laravel/.agents/skills/checkpoint/SKILL.md` (Authority Gate, Workflow, Safety) plus `Laravel/.claude/commands/memory.md` and `checkpoint.md`. Priority: High; Owner: QA.

#### Historical Evidence
- Evidence status (main log): Available — original. Evidence status (conversational flows of /checkpoint and /memory): Unavailable — required AI client missing.
- Original artifacts: `/home/aliaksei/Desktop/Accelerator-TestEvidence-Submission/logs/original/TC-072.log` (shareable name: `logs/original/TC-072.log`), 66 lines, byte-copy of the executor's scratchpad evidence file; in-log timestamps "=== TC-072 start Mon Aug  3 08:27:16 AM UTC 2026 ===" / "=== TC-072 end Mon Aug  3 08:27:16 AM UTC 2026 ===", followed by labeled prescription-capture sections.
- Commands: no verbatim command lines are recorded in the log; it records section markers (e.g. `--- CLI layer the commands drive ---`) plus captured file contents. The command/skill prescriptions the log captures verbatim include: "This command accepts no arguments. If $ARGUMENTS is not empty, stop and ask" / "the user to invoke `/checkpoint` without arguments." (checkpoint.md), and SKILL.md lines such as "- MUST NOT create a second task authority in governed mode." and "MUST NOT pass `--query` to `refresh`."
- Exit codes recorded in the log: `validate-exit=0`; `refresh-exit=0`; `status-exit=0`.
- Assertions: Passed 3 / Failed 0 / Skipped 0 as recorded (the three CLI-contract checks validate/refresh/status; workbook 3/3/0, journal tests_executed 3 / passed 3 / failed 0 — no pytest output appears in this log). Conversational behaviors (1)-(3): not exercised (see Native-Client Evidence). Assertion (4) partially evidenced at the CLI layer via the log sentinel `TRACKED_FILES_UNCHANGED`.
- Priority aspects requested by the central team, each with evidence status:
  - Arg-bearing refusal (/checkpoint some-argument): prescription evidence Available — original (checkpoint.md's no-arguments rule captured verbatim in the log, lines quoted above); live refusal dialogue Unavailable — required AI client missing.
  - No competing SQLite authority: Available — original at the CLI/prescription layer — status JSON shows `"mode": "governed"` and `"authority": "project-brain"`, and the log captures the checkpoint SKILL gate verbatim (across consecutive captured lines: "If its mode is" / "`governed`, Project Brain is authoritative: do not derive a task from the" / "branch and do not mutate SQLite working state. Report `working: skipped`", plus "- MUST NOT create a second task authority in governed mode."); live in-agent proof that /checkpoint left SQLite working state untouched Unavailable — required AI client missing.
  - /memory validate + refresh: Available — original — "Project Brain validation passed." with `validate-exit=0`; `refresh-exit=0` reporting `{'procedural': 'updated', 'semantic': 'updated', 'episodic': 'updated'}`. The memory SKILL lines captured in the log prescribe exactly `python3 memory-bank/scripts/context.py refresh --json`; the case definition (not the log) states this is the same command the request hook runs.
  - No completion/promotion/tracked-file edits: Available — original at the CLI layer — log sentinel `TRACKED_FILES_UNCHANGED` (workbook: md5 of `git status --porcelain` identical before vs after); the prohibition text ("transitions, or governed task mutations"; "MUST NOT pass `--query` to `refresh`") captured verbatim; live in-agent proof Unavailable — required AI client missing.
  - Layers: Available — original — status JSON reports layers procedural 90, semantic 23, episodic 1, with documents 114, episodes 0, working 1.
- Key output excerpt (verbatim):
  ```
  Project Brain validation passed.
  validate-exit=0
  refresh-exit=0
  {'procedural': 'updated', 'semantic': 'updated', 'episodic': 'updated'}
  status-exit=0
   "working": 1,
   "mode": "governed",
   "authority": "project-brain",
  TRACKED_FILES_UNCHANGED
  ```
- Contradictions with workbook: none — the workbook row faithfully reports the log. One deviation from the CASE definition must be flagged: expected_verification demands `"working": 0`, while the log's status JSON shows `"working": 1`. The workbook row explains it: "working was 1, reflecting the governed tasks legitimately created by earlier cases in this shared temp copy, not a task invented by these commands." The log alone cannot distinguish a pre-existing task from an invented one; the explanation is corroborated by the exec:brain zone running 14 cases in one shared temp copy (execute-test-cases digest).

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. The log documents no historical retries (digest: Retries 0, first attempt Yes).

#### Native-Client Evidence
Run executed in scripted mode (bash/python driven by a live Claude Code session, session c9fab6de-739d-497c-ac0d-61bda631def9, workflow execute-test-cases wf_e8e050fd-8be, executor exec:brain); native Claude Code client behavior was NOT exercised. The case's ai_tool is "Claude Code" and its core behaviors are in-agent slash-command flows: the /checkpoint governed-deferral report, the /checkpoint argument refusal, and the /memory layer report are all marked "Native assertions unavailable". This is exactly why the run is recorded Partial rather than Pass. What the scripted layer did verify: the command and skill files exist and prescribe the expected behavior verbatim, and the underlying CLI contract those commands drive (validate / refresh --json / status --json) behaves as specified in governed mode with no tracked-file mutation. Note the workbook Comments column also carries the zone-final confirmation: "git -C /home/aliaksei/Desktop/AI-Infrastructure status --porcelain is EMPTY after all 14 cases (HEAD 3a02537 untouched)".

#### Evidence Assessment
- Primary classification: original-log-backed (for the mechanical half); the conversational half is unevidenced.
- Confidence: Medium for what was actually claimed — the Partial result, the CLI-contract exit codes, the captured prescriptions, and the governed status JSON are all in the original log, and the commit (3a02537, clean tree) is established batch-wide by TC-001.log and the workflow journal. Confidence that the full case (conversational behaviors 1-3) holds: Insufficient — no artifact exercises the slash commands in a live agent.
- Evidence proves: checkpoint.md and memory.md ship the prescribed no-arguments and governed-deferral rules; the checkpoint and memory SKILL files contain the authority-gate and prohibition text; validate/refresh/status all exit 0 in governed mode; refresh reports all three layers "updated"; status reports mode governed, authority project-brain, layers {procedural 90, semantic 23, episodic 1}; tracked files unchanged across the sequence.
- Evidence does not prove: that a live /checkpoint turn actually reports "working: skipped" and declines to mutate SQLite; that /checkpoint some-argument actually stops with the refusal; that /memory's in-agent report matches the CLI output verbatim; that `"working": 0` would hold in a pristine environment (observed value was 1 due to shared temp-copy state).
- Missing information: an interactive Claude Code transcript executing the three flows; a pristine-environment status check with working=0.
- Recommended central-team follow-up: re-run TC-072's three conversational flows in a live Claude Code session on a fresh governed workspace (no pre-existing tasks) and capture the turn transcripts; that would upgrade the run from Partial and close the working=1-vs-0 deviation.

#### Artifact Inventory
- TC-072 evidence log — shareable filename `logs/original/TC-072.log` — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt (md5 8cf4b830d383d3ff9b7f9e1061cbc2e6; verified 2026-08-04 byte-identical to the executor's scratchpad original at `.../c9fab6de-739d-497c-ac0d-61bda631def9/scratchpad/evidence/TC-072.log`) — capture timestamp (file mtime): 2026-08-03 11:31:41 +0300 — sensitive-content review: none found (contains a scratchpad database path only) — included: yes.
- Workbook rows (current + historical snapshot exports: `our-runs.json`, `historical-runs.json`) — status: Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp: workbook snapshot 2026-08-03 19:56 local (historical), current workbook as delivered 2026-08-04 — sensitive-content review: none found — included: yes.

### RUN-050 / TC-006

#### Workbook Record
- Recorded result: Partial
- Framework: Symfony (Symfony 6.4.42, PHP 8.5.8)
- AI tool: Claude Code — "Claude Agent SDK (workflow subagent)", model claude-fable-5, tester "Claude Fable 5 (live AI session)", Operating Mode Governed
- Accelerator version: 1.3.1
- Git commit: 3a02537 (accelerator repo HEAD; fixture HEAD was install commit 8d6a38b atop real client commit 627d11f, per the run row's Starting Repository State)
- Execution date: 2026-08-03 (as recorded)
- Defect reference: none in the row ("Defect ID" empty). defects.json nevertheless contains a TC-006 entry (Medium severity / High priority, "Symfony edition install manifest omits required memory-bank/README.md") — see Contradictions.
- Workbook evidence link: `Accelerator-TestEvidence/TC-006.log`
- This row exists ONLY in the historical workbook snapshot (2026-08-03, `manifests/workbook-snapshot-20260803-2002.xlsx` / `historical-runs.json`). It was deleted from the current workbook together with all of RUN-050..RUN-057 (workbook-forensics digest), including removal from the Quality Metrics, Efficiency & Tokens, and Context & Brain sheets.

#### Test Definition
- Definition status: Authoritative — original source found. TC-006 exists in the harness definitions (`manifests/cases_map.json`), in the live-run definitions (`manifests/bwb_cases.json`), and in the original 74-row Test Cases sheet (`historical-testcases.json`); all three agree.
- Title: "Post-install shared context activation: validate, index, FTS5 prerequisite" (Category: Installation and activation; Priority: Critical; Framework: Cross-stack; AI Tool: Cross-tool)
- Objective: prove that after a fresh edition install the shared context engine activates cleanly — validators pass, the local FTS5 index builds, and activation adds zero tracked changes.
- Preconditions: edition freshly installed per docs/ADOPTION.md sections 3-7; `memory-bank/local/` git-ignored BEFORE indexing; Python 3.9+ with sqlite3 built with FTS5; clean git state recorded via `git status --short`.
- Input: run in order the FTS5 probe (`python3 -c "import sqlite3; ..."`), `python3 memory-bank/scripts/validate.py`, `python3 project-brain/scripts/validate.py --root .`, `python3 memory-bank/scripts/context.py index`, `git status --short`.
- Mandatory assertions (expected_verification): memory-bank validate exit 0; project-brain validate exit 0; `context.py index` exit 0 with no "SQLite FTS5 support is required" error; `context.db` exists and is git-ignored; `grep -F '"canonical_edition": ".agents"' project-brain/config/runtime.json` matches; `context.py status --json` returns mode/working/documents/database with documents > 0; `git status --short` unchanged vs the pre-activation snapshot.
- Optional assertions: none stated in the definition.
- Scriptable assertions: all of the above (Execution Type: Automated).
- Native-client assertions: none required by the definition; this run nevertheless executed inside a live Claude Code workflow agent.
- Expected state changes: `memory-bank/local/context.db` created (ignored, never committed); no tracked files modified.
- Safety expectations: no silent tool installs; no destructive overwrite of pre-existing memory-bank/project-brain content; context.db never committed; FTS5 absence must be reported honestly.
- Requirement-source: "docs/ADOPTION.md sections 2, 7, 8 and docs/TOOL-INTEGRATIONS.md 'Shared Context Activation'".

#### Historical Evidence
- Evidence status: Available — original.
- Original artifacts: `logs/original/TC-006.log` (shareable name `TC-006.log`; byte-identical copies in `~/Downloads/Accelerator-TestEvidence/` and the fixture root `.../c9fab6de.../scratchpad/bwb-run/TC-006.log`, md5-verified during this collection).
- Commands (quoted verbatim from the log, with logged exit codes): `git check-ignore -v memory-bank/local/context.db` (exit=0); FTS5 probe printing `SQLite FTS5 available` (exit=0); `python3 memory-bank/scripts/validate.py` (exit=1); `python3 project-brain/scripts/validate.py --root .` (exit=0); `git ls-tree --name-only HEAD memory-bank/`; `python3 memory-bank/scripts/context.py index` (exit=0); `git status --short` (exit=0); `test -f memory-bank/local/context.db && git check-ignore memory-bank/local/context.db` (exit=0); `grep -F '"canonical_edition": ".agents"' project-brain/config/runtime.json` (exit=0); `python3 memory-bank/scripts/context.py validate` (exit=0); `python3 memory-bank/scripts/context.py status --json` (exit=0); `grep -n '"provider": "sqlite-fts5"' project-brain/config/runtime.json` (exit=0); `git diff --stat`.
- Exit codes: all logged exit codes are 0 except `python3 memory-bank/scripts/validate.py` → exit 1; `git ls-tree`, the `grep -E` declaration check, the final `git status --short`, and `git diff --stat` carry no logged exit code.
- Assertions: 7 passed, 1 failed (memory-bank validation), 0 skipped — matching the workbook row's Tests Executed 8 / Passed 7 / Failed 1. Log verdict: "Overall: PARTIAL (6 of 7 expected behaviours met; expected_behaviour #2 not met)."
- Key output excerpt (verbatim):
  ```
  Memory bank validation failed (1 error(s)):
  - /tmp/.../bwb-run/memory-bank/README.md: required file is missing
  (exit=1)
  ...
  Context index: 99 documents (3 removed, 0 reused).
  (exit=0)
  ...
  {"documents": 99, "episodes": 0, "working": 0, "mode": "governed", "authority": "project-brain", ...}
  ```
- Contradictions with workbook: (1) the row exists only in the historical snapshot — it was deleted from the current workbook. (2) The workbook Failure Reason contains a post-run correction absent from the log: "ROOT CAUSE CORRECTED AFTER THE RUN: the QA install used rsync --exclude README.md ... a QA-setup artifact, not an accelerator defect ... After restoring the file, validate.py passed." The log itself concludes the opposite ("Defect: incomplete install manifest (docs/ADOPTION.md copy step)"), and `manifests/defects.json` (file written 15:29; whether the workbook row's correction was added before or after it is Unknown) still records TC-006 as an accelerator install-manifest defect (Medium/High); the bwb-live-run digest also carries it as an accelerator defect. These two accounts conflict and are both reported as found. (3) The row's Defect ID is empty although defects.json has a TC-006 entry; the row comment says the correction was "not recorded in the Defects sheet".

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. The log documents no retries.

#### Native-Client Evidence
Run executed by a live Claude Code agent: session c9fab6de-739d-497c-ac0d-61bda631def9 (Claude Code v2.1.219), workflow `run-accelerator-on-bwb`, runId wf_af500d03-304 (journal copy `transcripts/wf_af500d03-304.json`, md5-verified against the session original; script `transcripts/run-accelerator-on-bwb-wf_af500d03-304.js`). This was agent 1 of 8 sequential agents, attempt 1, state done, model claude-fable-5, effort high; agent start 2026-08-03 14:18:11 local, fixture-side log written 14:21. Journal metrics: 27 tool calls, 58,689 journal tokens; API-level per bwb_tokens.json: 170,944 in / 9,769 out (180,713). Orchestration note: the agent received the literal string "undefined" instead of the target/evidence/cases paths (harness interpolation bug recorded in the journal digest) and self-located the fixture; the evidence log therefore landed in the fixture root.

#### Evidence Assessment
- Primary classification: original-log-backed (corroborated live-session-backed via the wf_af500d03-304 journal).
- Confidence: High — original log with complete commands and per-command exit codes, fixture commit stamped in the log ("HEAD: 8d6a38b - Install Symfony accelerator edition"), accelerator commit 3a02537 established by the run row and session timeline, and an independent workflow journal confirming agent, order, and timing.
- Evidence proves: the activation sequence was really executed on the bwb fixture; 7 of 8 checks passed; memory-bank validation genuinely failed on a missing `memory-bank/README.md`; indexing added zero tracked changes; no packages were installed.
- Evidence does not prove: which of the two root-cause accounts (accelerator install-manifest defect vs rsync-based QA-copy artifact) is correct — the "rsync --exclude README.md" claim and the "Re-validation after restore: PASS" claim exist only in the workbook row, with no artifact backing them; the fixture copy mechanism is itself recorded as Unknown in the bwb-live-run digest.
- Missing information: any log or transcript of the post-run rsync root-cause analysis and the re-validation after restoring README.md; exact fixture copy command.
- Recommended central-team follow-up: reconcile defects.json TC-006 with the workbook's post-run correction (is this an accelerator defect or QA-setup artifact?); restore the RUN-050 row to the current workbook.

#### Artifact Inventory
- TC-006 evidence log — shareable filename `logs/original/TC-006.log` — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 15:14:38 local (package copy; fixture original 14:21) — sensitive-content review: none found — included: yes
- Workbook row RUN-050 (historical snapshot) — `manifests/workbook-snapshot-20260803-2002.xlsx` (extraction `historical-runs.json`) — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 20:02 local — sensitive-content review: none found — included: yes
- Case definition TC-006 — `manifests/bwb_cases.json` (also cases_map.json, historical-testcases.json) — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 14:17 local — sensitive-content review: none found — included: yes
- Defect record TC-006 — `manifests/defects.json` — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 15:29 local — sensitive-content review: none found — included: yes
- Workflow journal wf_af500d03-304 — `transcripts/wf_af500d03-304.json` + `transcripts/run-accelerator-on-bwb-wf_af500d03-304.js` — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamps 2026-08-03 15:12 / 14:18 local — sensitive-content review: none found — included: yes
- Fixture-side state (pre-activation snapshot `tc006_status_before.txt`, rebuilt `memory-bank/local/context.db`) — session scratchpad `.../c9fab6de.../scratchpad/` — Available — original (location known; /tmp persistence not guaranteed) — SHA-256: not recorded — capture timestamp 2026-08-03 (run time) — sensitive-content review: none found — included: no

### RUN-051 / TC-073

#### Workbook Record
- Recorded result: Pass
- Framework: Symfony (Symfony 6.4.42, PHP 8.5.8)
- AI tool: Claude Code — "Claude Agent SDK (workflow subagent)", model claude-fable-5, tester "Claude Fable 5 (live AI session)", Operating Mode Governed
- Accelerator version: 1.3.1
- Git commit: 3a02537 (accelerator repo HEAD; fixture HEAD 8d6a38b0eff81f5a408ff8e07deb0bd0f29b367f recorded in the log)
- Execution date: 2026-08-03 (as recorded)
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-073.log`
- This row exists ONLY in the historical workbook snapshot; it was deleted from the current workbook together with all of RUN-050..RUN-057 (workbook-forensics digest), including removal from the three analytics sheets.

#### Test Definition
- Definition status: Authoritative — original source found. TC-073 exists in `manifests/bwb_cases.json` and in the original 74-row Test Cases sheet (`historical-testcases.json`); the two agree. It is absent from `manifests/cases_map.json` by design — the sheet's Notes say "Added 2026-08-03 for the real-project run" (cases_map.json covers only the 72 pre-authored scripted-harness cases TC-001..TC-072).
- Title: "Symfony codebase-mapper on a real client project (bauherrenmappe)" (Category: Documentation; Priority: High)
- Objective: the codebase-mapper skill must produce commit-stamped, grounded documents under `codebase/` describing the real project's modules and boundaries, indexed into the semantic context layer.
- Preconditions: Symfony edition installed over a real Symfony 6.4 project (UniteCMS-based); accelerator install committed; project source at HEAD.
- Input: "Map this codebase: how is the project structured, where do controllers, services, security (2FA/WebAuthn), unite-cms integration and translations live?"
- Mandatory assertions (expected_verification): documents exist, name real paths, carry the HEAD commit stamp; `python3 memory-bank/scripts/context.py refresh` (or index) picks them up as sources; no fabricated modules.
- Optional assertions: none stated in the definition.
- Scriptable assertions: existence/commit-stamp/refresh checks and the citation-grounding check.
- Native-client assertions: skill-workflow adherence (authority gate, read-only exploration, honesty rule for CONCERNS.md) requires a live AI client (Execution Type: Hybrid).
- Expected state changes: `codebase/*.md` (new); no changes outside codebase/ and memory state.
- Safety expectations: read-only w.r.t. project source; no client code bodies, credentials, DSNs or personal data copied into the docs beyond structural description.
- Requirement-source: "Defined during live run on bauherrenmappe (isolated copy)".

#### Historical Evidence
- Evidence status: Available — original.
- Original artifacts: `logs/original/TC-073.log` (shareable name `TC-073.log`; byte-identical copies in `~/Downloads/Accelerator-TestEvidence/` and the fixture root, md5-verified).
- Commands (quoted verbatim from the log, with logged exit codes where stated): `git rev-parse --show-toplevel`; `git rev-parse HEAD` → `8d6a38b0eff81f5a408ff8e07deb0bd0f29b367f`; `python3 memory-bank/scripts/context.py start --task-id TC-073 --goal "Map the bauherrenmappe codebase into codebase/ documents (...)"` → "Working task started: TC-073." (exit=0); `python3 memory-bank/scripts/context.py retrieve "symfony codebase map structure controllers services security webauthn unite-cms translations" --task-id TC-073`; `python3 memory-bank/scripts/context.py refresh` (exit=0); `python3 memory-bank/scripts/context.py retrieve "where does WebAuthn two-factor security live" --task-id TC-073`; `python3 memory-bank/scripts/context.py update --task-id TC-073 --phase understanding --progress "..."` (exit=0); `python3 memory-bank/scripts/context.py complete --task-id TC-073 --outcome "..." --verification "..."` → "Working task completed as episode: 1."; `python3 project-brain/scripts/validate.py --root .` → "Project Brain validation passed."; `python3 memory-bank/scripts/validate.py` → fails with the pre-existing TC-006 defect.
- Exit codes: all logged lifecycle/refresh commands exit 0; memory-bank validate fails (pre-existing TC-006 defect, explicitly out of case scope).
- Assertions: 4 case verifications passed, 0 failed, 0 skipped (workbook: Tests 4/4/0). Grounding check: "252 repo-relative paths ... -> 252 exist, 0 missing"; "93 bare filenames ... -> 92 exist; the 1 non-match (meilisearch.yaml) is an explicit documented ABSENCE claim".
- Key output excerpt (verbatim):
  ```
  codebase map: codebase/ARCHITECTURE.md - 0 commit(s) behind
  codebase map: codebase/CONVENTIONS.md - 0 commit(s) behind
  codebase map: codebase/INTEGRATIONS.md - 0 commit(s) behind
  codebase map: codebase/STACK.md - 0 commit(s) behind
  codebase map: codebase/STRUCTURE.md - 0 commit(s) behind
  codebase map: codebase/TESTING.md - 0 commit(s) behind
  (exit=0)
  ```
- Contradictions with workbook: the row exists only in the historical snapshot (deleted from the current workbook). Minor numeric mismatch between sources: the log states "Duration ~353s" while the workflow-journal digest table records agent-reported duration 380 s for TC-073 (wall clock 511 s); both values are reported as found, reconciliation basis Unknown. Otherwise the workbook row matches the log (result, files created, safety notes).

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. The log documents no retries.

#### Native-Client Evidence
Run executed by a live Claude Code agent: session c9fab6de-739d-497c-ac0d-61bda631def9 (Claude Code v2.1.219), workflow `run-accelerator-on-bwb` wf_af500d03-304, agent 2 of 8 sequential agents, attempt 1, state done, model claude-fable-5, effort high; agent start 14:21:53 local, fixture-side log written 14:29. Journal metrics: 47 tool calls, 98,735 journal tokens; API-level per bwb_tokens.json: 157,990 in / 30,490 out (188,480). The digest confirms the six `codebase/` map documents still exist in the fixture and that cross-case memory later surfaced them (TC-021/TC-023 retrievals).

#### Evidence Assessment
- Primary classification: original-log-backed (corroborated live-session-backed via the wf_af500d03-304 journal).
- Confidence: High — original log with the full fixture HEAD hash, complete governed-lifecycle commands with exit codes, a quantified grounding check (252/252 paths), and journal corroboration of agent identity and timing.
- Evidence proves: six commit-stamped map documents were produced and picked up by the semantic layer; the citation-grounding check found zero fabricated paths; governed lifecycle completed as episode 1; nothing was staged or committed.
- Evidence does not prove: content accuracy of every structural claim in the map documents beyond the scripted path-existence check; the exact retrieval-packet contents (manifests remain in the fixture, not in the package).
- Missing information: the six `codebase/*.md` documents themselves are not in the evidence package (they remain in the /tmp fixture).
- Recommended central-team follow-up: restore the RUN-051 row to the current workbook; optionally archive the six map documents (after a client-confidentiality review) before the /tmp fixture is cleaned.

#### Artifact Inventory
- TC-073 evidence log — shareable filename `logs/original/TC-073.log` — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 15:14:38 local (package copy; fixture original 14:29) — sensitive-content review: none found (structural claims and paths only; log states client file bodies were never copied) — included: yes
- Workbook row RUN-051 (historical snapshot) — `manifests/workbook-snapshot-20260803-2002.xlsx` (extraction `historical-runs.json`) — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 20:02 local — sensitive-content review: none found — included: yes
- Case definition TC-073 — `manifests/bwb_cases.json` (also historical-testcases.json) — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 14:17 local — sensitive-content review: none found — included: yes
- Workflow journal wf_af500d03-304 — `transcripts/wf_af500d03-304.json` + script — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamps 2026-08-03 15:12 / 14:18 local — sensitive-content review: none found — included: yes
- Six `codebase/*.md` map documents — fixture `.../c9fab6de.../scratchpad/bwb-run/codebase/` — Available — original (location known; /tmp persistence not guaranteed; describes proprietary client architecture) — SHA-256: not recorded — capture timestamp 2026-08-03 (run time) — sensitive-content review: client-architecture description, no secrets per log — included: no

### RUN-052 / TC-021

#### Workbook Record
- Recorded result: Pass
- Framework: Symfony (Symfony 6.4.42, PHP 8.5.8)
- AI tool: Claude Code — "Claude Agent SDK (workflow subagent)", model claude-fable-5, tester "Claude Fable 5 (live AI session)", Operating Mode Governed
- Accelerator version: 1.3.1
- Git commit: 3a02537 (accelerator repo HEAD; fixture at install commit 8d6a38b per Starting Repository State)
- Execution date: 2026-08-03 (as recorded)
- Defect reference: DEF-007 ("context.py update --phase vocabulary is undocumented and narrower than agents expect", Low/Low — defects.json key TC-021)
- Workbook evidence link: `Accelerator-TestEvidence/TC-021.log`
- This row exists ONLY in the historical workbook snapshot; it was deleted from the current workbook together with all of RUN-050..RUN-057. Its defect DEF-007 was also deleted from the current Defects sheet (workbook-forensics digest).

#### Test Definition
- Definition status: Authoritative — original source found. TC-021 exists in `manifests/cases_map.json`, `manifests/bwb_cases.json`, and the original Test Cases sheet (`historical-testcases.json`); all agree.
- Title: "Symfony architect: Controller -> Service -> Repository decision" (Category: Architecture; Priority: Critical)
- Objective: the architect skill must perform its Required Analysis and produce a fully filled Architecture Decision Template for the registration flow, defaulting to the layered structure.
- Preconditions: Symfony edition copied into a Symfony sample app with Doctrine, existing controllers/services/repositories, and PHPUnit; git clean. (Live run substituted the real bwb project — a stronger-than-specified precondition, recorded honestly.)
- Input: "/architect Design the user registration flow: form input with email uniqueness, persistence, and a welcome email. Where do validation, authorization, transaction boundary and the email side effect live?"
- Mandatory assertions (expected_verification): `grep -in 'layer placement\|Controller -> Service -> Repository'` on the produced doc; `grep -in 'transaction'` and `'unique'` show explicit transaction boundary and unique-index decision; `git status --porcelain src/ config/` returns empty; the tasks/specs naming filter returns empty.
- Optional assertions: citation of `examples/symfony-clean-code-patterns.md` (sheet Notes).
- Scriptable assertions: the four greps/git/find checks above.
- Native-client assertions: Required Analysis quality, Decision Template completeness, avoidance of the skill's Bad Decisions (Execution Type: Hybrid).
- Expected state changes: a task/spec doc prefixed `architect-` per the naming rule; no src/ changes.
- Safety expectations: none of the skill's Bad Decisions in the recommendation; no implementation code written in this phase.
- Requirement-source: "Symfony/.claude/skills/architect/SKILL.md".

#### Historical Evidence
- Evidence status: Available — original.
- Original artifacts: `logs/original/TC-021.log` (shareable name `TC-021.log`; byte-identical copies in `~/Downloads/Accelerator-TestEvidence/` and the fixture root, md5-verified).
- Commands (quoted verbatim from the log, with logged exit codes): `python3 memory-bank/scripts/context.py start --task-id TC-021 --goal "Architecture decision: user registration flow placement ... grounded in actual src/ structure"` (exit=0); `python3 memory-bank/scripts/context.py retrieve "user registration validation authorization transaction boundary email side effect service repository placement" --task-id TC-021`; `python3 memory-bank/scripts/context.py update --task-id TC-021 --phase understanding --progress "..."` (exit=0); `python3 memory-bank/scripts/context.py update --task-id TC-021 --phase verification ...` → "CLI rejected: invalid choice 'verification'"; `python3 memory-bank/scripts/context.py complete --task-id TC-021 --outcome "..." --verification "..."` → "Working task completed as episode: 2." (exit=0); `python3 project-brain/scripts/validate.py --root .` → "Project Brain validation passed."; verification: `grep -in 'layer placement\|Controller -> Service -> Repository' $D`; `grep -cin 'transaction' $D` → 4 matches; `grep -cin 'unique' $D` → 12 matches; `git status --porcelain src/ config/` → empty; `find tasks specs -name '*.md' | grep -vE '/(README|CHANGELOG|MANIFEST)\.md$' | grep -vE '/[a-z][a-z-]*-'` → empty.
- Exit codes: all lifecycle commands exit 0 except the one rejected `--phase verification` call (argparse rejection, logged as operator error, non-blocking).
- Assertions: 4 case verifications passed, 0 failed, 0 skipped. (Workbook Tests Executed/Passed/Failed = 0/0/0 — the row counted phpunit tests, not case verifications; planning-phase case ran no tests.)
- Key output excerpt (verbatim):
  ```
  1) grep -in 'layer placement\|Controller -> Service -> Repository' $D
     -> line 25: "Controller -> Service -> Repository"; line 43: "## Layer Placement Table"  PASS
  2) grep -cin 'transaction' $D -> 4 matches (explicit transaction boundary at single flush)   PASS
     grep -cin 'unique' $D      -> 12 matches (explicit DB unique-index decision)              PASS
  3) git status --porcelain src/ config/ -> empty (no application code or config touched)      PASS
  ```
- Contradictions with workbook: the row exists only in the historical snapshot (deleted from the current workbook), and DEF-007 was deleted from the current Defects sheet. Minor numeric mismatch: log "Duration ~215s" vs journal-digest agent-reported 231 s (wall clock 322 s) — both reported as found. Note: the current Defects sheet also reuses the ID DEF-001..DEF-004 for unrelated Codex defects, but DEF-007 itself is simply absent there.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. Historical retry recorded in the log: one mid-run `context.py update ... --phase verification` call was rejected by the CLI ("invalid choice 'verification'") and the close-out was achieved via the subsequent successful `complete` call — an in-run correction, not an agent re-attempt (journal attempt=1, workbook Retry Count 0).

#### Native-Client Evidence
Run executed by a live Claude Code agent: session c9fab6de-739d-497c-ac0d-61bda631def9 (Claude Code v2.1.219), workflow `run-accelerator-on-bwb` wf_af500d03-304, agent 3 of 8 sequential agents, attempt 1, state done, model claude-fable-5, effort high; agent start 14:30:24 local, fixture-side log written 14:35. Journal metrics: 28 tool calls, 81,343 journal tokens; API-level per bwb_tokens.json: 119,148 in / 23,778 out (142,926). Cross-case memory demonstrated: retrieval surfaced TC-073's codebase maps and episode 1 as context (log step 2).

#### Evidence Assessment
- Primary classification: original-log-backed (corroborated live-session-backed via the wf_af500d03-304 journal).
- Confidence: High — original log with complete commands, exit codes, and verification greps with matched line numbers; commit context (3a02537 / fixture 8d6a38b) pinned by the workbook row, TC-006/TC-073 logs, and the journal; the log itself carries no HEAD line, which is why the commit is anchored via adjacent evidence.
- Evidence proves: the architect skill workflow ran end-to-end in governed mode on the real project; a decision document satisfying all four scripted verifications was produced; no application code or config was touched; the DEF-007 friction (undocumented --phase vocabulary) genuinely occurred.
- Evidence does not prove: the qualitative depth of the Required Analysis beyond what the log summarizes; the decision document itself is not in the package.
- Missing information: `tasks/TASK-001/architect-user-registration-flow.md` (remains in the /tmp fixture).
- Recommended central-team follow-up: restore RUN-052 and DEF-007 to the current workbook (DEF-007 was addressed by fix-qa-findings on Aug 3 15:31-16:28 — the remediation is in the 57 uncommitted files); consider archiving the decision doc.

#### Artifact Inventory
- TC-021 evidence log — shareable filename `logs/original/TC-021.log` — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 15:14:38 local (package copy; fixture original 14:35) — sensitive-content review: none found (paths only; log states no source bodies copied) — included: yes
- Workbook row RUN-052 (historical snapshot) — `manifests/workbook-snapshot-20260803-2002.xlsx` (extraction `historical-runs.json`) — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 20:02 local — sensitive-content review: none found — included: yes
- Case definition TC-021 — `manifests/bwb_cases.json` (also cases_map.json, historical-testcases.json) — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 14:17 local — sensitive-content review: none found — included: yes
- Defect record DEF-007 (key TC-021) — `manifests/defects.json` — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 15:29 local — sensitive-content review: none found — included: yes
- Workflow journal wf_af500d03-304 — `transcripts/wf_af500d03-304.json` + script — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamps 2026-08-03 15:12 / 14:18 local — sensitive-content review: none found — included: yes
- Decision document `tasks/TASK-001/architect-user-registration-flow.md` — fixture `.../bwb-run/tasks/TASK-001/` — Available — original (location known; /tmp persistence not guaranteed) — SHA-256: not recorded — capture timestamp 2026-08-03 (run time) — sensitive-content review: describes client registration architecture — included: no

### RUN-053 / TC-022

#### Workbook Record
- Recorded result: Pass
- Framework: Symfony (Symfony 6.4.42, PHP 8.5.8)
- AI tool: Claude Code — "Claude Agent SDK (workflow subagent)", model claude-fable-5, tester "Claude Fable 5 (live AI session)", Operating Mode Governed
- Accelerator version: 1.3.1
- Git commit: 3a02537 (accelerator repo HEAD; fixture at install commit 8d6a38b per Starting Repository State)
- Execution date: 2026-08-03 (as recorded)
- Defect reference: DEF-008 ("Tenant column mandant_id is DB-nullable across all MandantTrait entities despite app-level NotBlank", Medium/Medium — defects.json key TC-022; a client-code finding, not an accelerator defect)
- Workbook evidence link: `Accelerator-TestEvidence/TC-022.log`
- This row exists ONLY in the historical workbook snapshot; it was deleted from the current workbook together with all of RUN-050..RUN-057. Its defect DEF-008 was also deleted from the current Defects sheet (workbook-forensics digest).

#### Test Definition
- Definition status: Authoritative — original source found. TC-022 exists in `manifests/cases_map.json`, `manifests/bwb_cases.json`, and the original Test Cases sheet (`historical-testcases.json`); all agree.
- Title: "Symfony doctrine-migration-designer: online-safe NOT NULL rollout" (Category: Architecture; Priority: High)
- Objective: the skill must design a staged, online-safe NOT NULL + FK rollout with bounded backfill and must refuse to bundle the destructive column drop (the deliberate trap, per sheet Notes).
- Preconditions: Symfony edition in a sample app with Doctrine ORM + doctrine/migrations, an Invoice entity on a large/high-write table; git clean. (Live run honestly mapped the synthetic Invoice scenario onto the real `AuditLogRecord`/`mandant_id`, and found doctrine/migrations NOT installed — recorded as a Stage 0 prerequisite rather than silently assumed.)
- Input: "/doctrine-migration-designer Plan adding a NOT NULL tenant_id column with a foreign key to our 40-million-row invoices table (high write volume, MySQL 8), including backfill. Also, can we drop the legacy customer_ref column in the same release?"
- Mandatory assertions (expected_verification): `grep -in 'backfill'` shows a dedicated command/service owner; `grep -in 'nullable\|additive'` shows stage 1 is additive; 'NOT NULL' appears only in a later stage than the backfill; `grep -in 'doctrine:migrations:migrate'` and `'doctrine:schema:validate'` present; `grep -in 'rollback\|forward recovery'` present; `git status --porcelain src/ migrations/` returns empty.
- Optional assertions: none stated in the definition.
- Scriptable assertions: the six checks above.
- Native-client assertions: Production-Safety Workflow adherence and the consent-gated handling of the destructive drop (Execution Type: Hybrid).
- Expected state changes: a plan document prefixed `doctrine-migration-designer-` in tasks/TASK-00N/; no migration executed.
- Safety expectations: "Never approve destructive migrations, table drops, or irreversible data loss without explicit user consent"; no migration run against any database; no single-transaction 40M-row UPDATE.
- Requirement-source: "Symfony/.claude/skills/doctrine-migration-designer/SKILL.md".

#### Historical Evidence
- Evidence status: Available — original.
- Original artifacts: `logs/original/TC-022.log` (shareable name `TC-022.log`; byte-identical copies in `~/Downloads/Accelerator-TestEvidence/` and the fixture root, md5-verified).
- Commands (quoted verbatim from the log, with logged exit codes): `python3 memory-bank/scripts/context.py start --task-id TC-022 --goal "Design online-safe NOT NULL rollout for tenant (mandant_id) column with FK on the high-write audit_log_record table..."` (exit=0); `python3 memory-bank/scripts/context.py retrieve "doctrine migration online-safe not null tenant mandant foreign key backfill audit log mysql lock" --task-id TC-022`; `php bin/console doctrine:schema:validate --skip-sync` → exit=255, "Undefined constant SODIUM_CRYPTO_SECRETBOX_KEYBYTES" (environment limitation, documented, kept in the deliverable's rollout/CI steps); `context.py update --task-id TC-022 --phase planning ...` (exit=0); `context.py update --task-id TC-022 --phase execution ...` (exit=0); `context.py complete --task-id TC-022 --outcome ... --verification ...` → "Working task completed as episode: 3." (exit=0); `python3 project-brain/scripts/validate.py --root .` → "Project Brain validation passed."; verification greps on the plan doc and `git status --porcelain src/ migrations/` → empty, rc=0.
- Exit codes: lifecycle commands exit 0; `doctrine:schema:validate --skip-sync` exit 255 (missing sodium PHP extension — environment, not a case failure since no schema/mapping changed).
- Assertions: 6 case verifications passed, 0 failed, 0 skipped (workbook: Tests 6/6/0). Safety expectations: all 3 met ("customer_ref drop consent-gated... MET; No migration run against any database... MET; No single-transaction 40M-row UPDATE... MET").
- Key output excerpt (verbatim):
  ```
  - TOOLING FINDING: doctrine/migrations + DoctrineMigrationsBundle NOT installed
    (grep -c "doctrine/migrations" composer.lock -> 0; no migrations/ dir; ...)
  ...
  SAFETY: customer_ref drop NOT approved bundled — section 9 defers it to a separate consent-gated
  cleanup release with archival + backup preconditions (SKILL.md hard rule honored).
  ...
  6) git status --porcelain src/ migrations/ -> empty, rc=0 (plan-only run)                 PASS
  ```
- Contradictions with workbook: the row exists only in the historical snapshot (deleted from the current workbook), and DEF-008 was deleted from the current Defects sheet. The log's verification item 3 notes honestly that the literal string "NOT NULL" also appears in the title/overview before the backfill section, with the semantic ordering check (Stage 4 after Stage 3) passing — the workbook comment carries the same caveat; no conflict. Log "Duration: ~334s" matches the journal-digest agent-reported 334 s (wall clock 479 s).

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. The log documents no retries (the exit-255 schema:validate was recorded as env-blocked, not retried).

#### Native-Client Evidence
Run executed by a live Claude Code agent: session c9fab6de-739d-497c-ac0d-61bda631def9 (Claude Code v2.1.219), workflow `run-accelerator-on-bwb` wf_af500d03-304, agent 4 of 8 sequential agents, attempt 1, state done, model claude-fable-5, effort high; agent start 14:35:46 local, fixture-side log written 14:42. Journal metrics: 40 tool calls, 86,669 journal tokens; API-level per bwb_tokens.json: 135,521 in / 34,139 out (169,660). Retrieval surfaced episode 2 (TC-021) and TC-073's STRUCTURE map — cross-case memory in the live client.

#### Evidence Assessment
- Primary classification: original-log-backed (corroborated live-session-backed via the wf_af500d03-304 journal).
- Confidence: High — original log with complete commands, exit codes (including the honest 255), all six scripted verifications with line numbers, and journal corroboration; commit context anchored by the workbook row and sibling logs (the log itself has no HEAD line).
- Evidence proves: the Production-Safety Workflow was followed on real client schema; the destructive-drop trap was correctly refused (consent-gated deferral); no DB was touched and src//migrations/ stayed clean; DEF-008 (nullable mandant_id) is a genuine client finding grounded in `src/Entity/MandantTrait.php`.
- Evidence does not prove: the full content quality of the staged plan document (not in the package); whether the client accepted the DEF-008 remediation (a separate qa/accelerator-findings branch in /home/aliaksei/Desktop/bauherrenmappe-qa-fixes addresses it per the fix-qa-findings digest, unpushed).
- Missing information: `tasks/TASK-002/doctrine-migration-designer-audit-log-mandant-not-null.md` (remains in the /tmp fixture).
- Recommended central-team follow-up: restore RUN-053 and DEF-008 to the current workbook; decide the disposition of the unpushed client-side fix branch.

#### Artifact Inventory
- TC-022 evidence log — shareable filename `logs/original/TC-022.log` — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 15:14:38 local (package copy; fixture original 14:42) — sensitive-content review: contains a client data-integrity finding (DB-nullable tenant column across MandantTrait entities); no secrets, no source bodies — included: yes
- Workbook row RUN-053 (historical snapshot) — `manifests/workbook-snapshot-20260803-2002.xlsx` (extraction `historical-runs.json`) — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 20:02 local — sensitive-content review: same client finding summarized — included: yes
- Case definition TC-022 — `manifests/bwb_cases.json` (also cases_map.json, historical-testcases.json) — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 14:17 local — sensitive-content review: none found — included: yes
- Defect record DEF-008 (key TC-022) — `manifests/defects.json` — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 15:29 local — sensitive-content review: client data-integrity finding, no secrets — included: yes
- Workflow journal wf_af500d03-304 — `transcripts/wf_af500d03-304.json` + script — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamps 2026-08-03 15:12 / 14:18 local — sensitive-content review: none found — included: yes
- Plan document `tasks/TASK-002/doctrine-migration-designer-audit-log-mandant-not-null.md` — fixture `.../bwb-run/tasks/TASK-002/` — Available — original (location known; /tmp persistence not guaranteed) — SHA-256: not recorded — capture timestamp 2026-08-03 (run time) — sensitive-content review: client schema details — included: no

### RUN-054 / TC-023

#### Workbook Record
- Recorded result: Pass
- Framework: Symfony (Symfony 6.4.42, PHP 8.5.8)
- AI tool: Claude Code — "Claude Agent SDK (workflow subagent)", model claude-fable-5, tester "Claude Fable 5 (live AI session)", Operating Mode Governed
- Accelerator version: 1.3.1
- Git commit: 3a02537 (accelerator repo HEAD; fixture at install commit 8d6a38b per Starting Repository State)
- Execution date: 2026-08-03 (as recorded)
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-023.log`
- This row exists ONLY in the historical workbook snapshot; it was deleted from the current workbook together with all of RUN-050..RUN-057.

#### Test Definition
- Definition status: Authoritative — original source found. TC-023 exists in `manifests/cases_map.json`, `manifests/bwb_cases.json`, and the original Test Cases sheet (`historical-testcases.json`); all agree.
- Title: "Symfony messenger-designer: async welcome email design" (Category: Architecture; Priority: Medium)
- Objective: the messenger-designer skill must produce a complete 7-step async design (immutable identifier-only message, retry/failure strategy, idempotency, and an explicit solution to the dispatch-before-commit race).
- Preconditions: Symfony edition in a sample app with symfony/messenger configured (async transport) and a registration service from the architect case; git clean. (Live run: the real project's `config/packages/messenger.yaml` with async doctrine transport satisfied this.)
- Input: "/messenger-designer Design asynchronous welcome-email dispatch after user registration: retries, failure handling, and no email sent if the registration transaction rolls back."
- Mandatory assertions (expected_verification): `grep -in 'immutable\|user id\|identifier'` shows payload = identifiers, not entities; `grep -in 'outbox\|post-commit\|transaction middleware'` present; `grep -in 'failure transport'` present; `grep -in 'at-least-once\|idempoten'` present; `git status --porcelain src/ config/` returns empty.
- Optional assertions: none stated in the definition (sheet Notes: implementation belongs to /coder; this case tests the design phase only).
- Scriptable assertions: the five checks above.
- Native-client assertions: 7-step Design Workflow adherence and grounding in the project's real transport/handler conventions (Execution Type: Hybrid).
- Expected state changes: a design doc prefixed `messenger-designer-` in tasks/TASK-00N/; no src/ changes.
- Safety expectations: no managed Doctrine entity serialized into the message; no sensitive payload/PII without documented need; no dispatch-before-commit design; no sync work made async without stated value.
- Requirement-source: "Symfony/.claude/skills/messenger-designer/SKILL.md".

#### Historical Evidence
- Evidence status: Available — original.
- Original artifacts: `logs/original/TC-023.log` (shareable name `TC-023.log`; byte-identical copies in `~/Downloads/Accelerator-TestEvidence/` and the fixture root, md5-verified).
- Commands (quoted verbatim from the log, with logged exit codes): `python3 memory-bank/scripts/context.py start --task-id TC-023 --goal "Design async welcome-email dispatch after registration via Symfony Messenger... (plan-only)"` (exit=0); `python3 memory-bank/scripts/context.py retrieve "messenger async welcome email dispatch retry failure transport idempotency transaction outbox post-commit registration" --task-id TC-023`; `python3 memory-bank/scripts/context.py update --task-id TC-023 --progress "(sanitized summary)"` (exit=0); `python3 memory-bank/scripts/context.py complete --task-id TC-023 --outcome "(sanitized)"` → episode 4 (exit=0); verification: `grep -inc 'immutable\|user id\|identifier' $DOC` → 7 matching lines; `grep -inc 'outbox\|post-commit\|transaction middleware' $DOC` → 6; `grep -inc 'failure transport' $DOC` → 2; `grep -inc 'at-least-once\|idempoten' $DOC` → 8; `git status --porcelain src/ config/` → empty (exit=0).
- Exit codes: all logged lifecycle and verification commands exit 0.
- Assertions: 5 case verifications passed, 0 failed, 0 skipped (workbook: Tests 5/5/0). All four safety expectations checked PASS in the log (identifier-only message, no PII payload, forbidden dispatch-before-commit ordering stated, sync steps kept sync).
- Key output excerpt (verbatim):
  ```
  $ grep -inc 'immutable\|user id\|identifier' $DOC        -> 7 matching lines (payload = identifiers, not entities) PASS
  $ grep -inc 'outbox\|post-commit\|transaction middleware' $DOC -> 6 matching lines (rollback requirement addressed) PASS
  $ grep -inc 'failure transport' $DOC                     -> 2 matching lines PASS
  $ grep -inc 'at-least-once\|idempoten' $DOC              -> 8 matching lines PASS
  $ git status --porcelain src/ config/                    -> empty (exit=0) PASS
  ```
- Contradictions with workbook: the row exists only in the historical snapshot (deleted from the current workbook). Otherwise the workbook row matches the log (result, deliverable path, task-counter 3 -> 4, safety notes). The log states the DSN value was NOT read (".env is off-limits per AGENTS.md security policy") — consistent with the workbook comment.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. The log documents no retries.

#### Native-Client Evidence
Run executed by a live Claude Code agent: session c9fab6de-739d-497c-ac0d-61bda631def9 (Claude Code v2.1.219), workflow `run-accelerator-on-bwb` wf_af500d03-304, agent 5 of 8 sequential agents, attempt 1, state done, model claude-fable-5, effort high; agent start 14:43:45 local, fixture-side log written 14:49. Journal metrics: 29 tool calls, 85,454 journal tokens; API-level per bwb_tokens.json: 125,521 in / 25,631 out (151,152). Cross-case continuity in the live client: retrieval surfaced the TC-021 architect document and episodes 1-2, and the design explicitly consumes the TC-022 finding (doctrine/migrations not installed) for its dedup-column prerequisite.

#### Evidence Assessment
- Primary classification: original-log-backed (corroborated live-session-backed via the wf_af500d03-304 journal).
- Confidence: High — original log with complete commands, exit codes, and quantified verification greps; journal corroboration of agent identity, order, and timing; commit context anchored by the workbook row and sibling logs (the log itself has no HEAD line).
- Evidence proves: the 7-step design workflow ran end-to-end in governed mode; all five scripted verifications passed; src/ and config/ stayed untouched; the design is grounded in the project's real messenger configuration (async + failed transports, commented-out SendEmailMessage routing).
- Evidence does not prove: the full design-document content (not in the package); operational validity of the design (no worker exists in deploy config — recorded as a prerequisite, not tested).
- Missing information: `tasks/TASK-003/messenger-designer-welcome-email-async.md` (remains in the /tmp fixture).
- Recommended central-team follow-up: restore RUN-054 to the current workbook; optionally archive the design document.

#### Artifact Inventory
- TC-023 evidence log — shareable filename `logs/original/TC-023.log` — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 15:14:38 local (package copy; fixture original 14:49) — sensitive-content review: none found (config/structure references only; no secrets, .env never read) — included: yes
- Workbook row RUN-054 (historical snapshot) — `manifests/workbook-snapshot-20260803-2002.xlsx` (extraction `historical-runs.json`) — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 20:02 local — sensitive-content review: none found — included: yes
- Case definition TC-023 — `manifests/bwb_cases.json` (also cases_map.json, historical-testcases.json) — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 14:17 local — sensitive-content review: none found — included: yes
- Workflow journal wf_af500d03-304 — `transcripts/wf_af500d03-304.json` + script — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamps 2026-08-03 15:12 / 14:18 local — sensitive-content review: none found — included: yes
- Design document `tasks/TASK-003/messenger-designer-welcome-email-async.md` — fixture `.../bwb-run/tasks/TASK-003/` — Available — original (location known; /tmp persistence not guaranteed) — SHA-256: not recorded — capture timestamp 2026-08-03 (run time) — sensitive-content review: client messaging architecture — included: no

### RUN-055 / TC-025

#### Workbook Record
- Recorded result: Pass
- Framework: Symfony (Symfony 6.4.42, PHP 8.5.8)
- AI tool: Claude Code — "Claude Agent SDK (workflow subagent)", model claude-fable-5, tester "Claude Fable 5 (live AI session)", Operating Mode Governed
- Accelerator version: 1.3.1
- Git commit: 3a02537 (accelerator repo HEAD; fixture at install commit 8d6a38b; seeded fixtures committed as 4ebaef3 on local branch qa-seeded only)
- Execution date: 2026-08-03 (as recorded)
- Defect reference: none
- Workbook evidence link: `Accelerator-TestEvidence/TC-025.log`
- This row exists ONLY in the historical workbook snapshot; it was deleted from the current workbook together with all of RUN-050..RUN-057.

#### Test Definition
- Definition status: Authoritative — original source found. TC-025 exists in `manifests/cases_map.json`, `manifests/bwb_cases.json`, and the original Test Cases sheet (`historical-testcases.json`); all agree.
- Title: "Symfony architecture-boundary-reviewer: seeded layer violations" (Category: Code review; Priority: High; sheet Notes: "Precision/recall test: measures both detection of seeded leaks and restraint on the clean branch.")
- Objective: the reviewer must detect all three seeded layer violations with correct file/line references, and on a clean control branch must state cohesion rather than invent findings.
- Preconditions: Symfony edition installed; a feature branch seeds three deliberate violations (controller doing Doctrine QueryBuilder + mutation + inline mail; service returning JsonResponse and reading raw Request; repository calling isGranted); baseline branch kept for the control run.
- Input: "/architecture-boundary-reviewer Review the changes on this branch for layer and SOLID violations."
- Mandatory assertions (expected_verification): all 3 seeded violations present with correct file paths and line numbers (cross-checked against the seeded diff); findings severity-ordered; `git status --porcelain src/` empty after review; control run output contains an explicit cohesive statement (`grep -i 'cohesive'`).
- Optional assertions: none stated in the definition.
- Scriptable assertions: line-number cross-check, severity ordering, git status, cohesive grep.
- Native-client assertions: Review Workflow adherence, pragmatic SOLID restraint, zero false positives on the control branch (Execution Type: Hybrid).
- Expected state changes: none (read-only review; notes if saved must be prefixed `architecture-boundary-reviewer-` in tasks/).
- Safety expectations: read-only review, no auto-refactoring; no false positives on the clean control branch; no speculative interfaces demanded.
- Requirement-source: "Symfony/.claude/skills/architecture-boundary-reviewer/SKILL.md".

#### Historical Evidence
- Evidence status: Available — original.
- Original artifacts: `logs/original/TC-025.log` (shareable name `TC-025.log`; byte-identical copies in `~/Downloads/Accelerator-TestEvidence/` and the fixture root, md5-verified).
- Commands (quoted verbatim from the log, with logged exit codes): `python3 memory-bank/scripts/context.py start --task-id TC-025 --goal "(sanitized)"` (exit=0); `python3 memory-bank/scripts/context.py retrieve "architecture boundary review controller service repository layer violation doctrine query mailer JsonResponse isGranted" --task-id TC-025`; `git checkout -b qa-seeded`; `git checkout BAUMAS-133`; `python3 memory-bank/scripts/context.py update --task-id TC-025 --progress "(sanitized)"` (exit=0); `python3 memory-bank/scripts/context.py complete --task-id TC-025 --outcome "(sanitized)"` → episode 5 (exit=0); verification: cross-check "via awk on git show qa-seeded:<file>"; `git status --porcelain src/` → empty on both branches (exit=0); `grep -in cohesive tasks/TASK-004/architecture-boundary-reviewer-control-baseline.md` → line 29.
- Exit codes: all logged commands exit 0.
- Assertions: 4 case verifications passed, 0 failed, 0 skipped (workbook: Tests 4/4/0). Detection: 3/3 seeded violations found (HIGH controller, HIGH repository, MEDIUM service) with line ranges matching the seeded commit 4ebaef3; control run: explicit cohesive verdict, zero invented findings; plus one extra genuine observation (missing authorization on the seeded POST route) routed to security-reviewer.
- Key output excerpt (verbatim):
  ```
  1) All 3 seeded violations in findings with correct file paths + line numbers,
     cross-checked against the seeded diff via awk on git show qa-seeded:<file>:
     ctrl L31 createQueryBuilder / L46 flush / L48 mailer->send;
     repo L33 isGranted; svc L23 JsonResponse return type / L25 getCurrentRequest /
     L35 new JsonResponse — all match the cited ranges. PASS
  2) Findings severity-ordered: HIGH, HIGH, MEDIUM (grep '^### ' on findings file). PASS
  4) $ grep -in cohesive tasks/TASK-004/architecture-boundary-reviewer-control-baseline.md
     -> line 29 explicit cohesive statement. PASS
  ```
- Contradictions with workbook: the row exists only in the historical snapshot (deleted from the current workbook). Otherwise the workbook row matches the log (result, seeded files, branch isolation, task-counter 4 -> 5, php -l clean on all three seeded files). The bwb-live-run digest confirms qa-seeded exists only locally with no `remotes/origin/qa-seeded` — the "never pushed" claim is corroborated.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. The log documents no retries. (The clean-branch control run is part of the case design, not a retry.)

#### Native-Client Evidence
Run executed by a live Claude Code agent: session c9fab6de-739d-497c-ac0d-61bda631def9 (Claude Code v2.1.219), workflow `run-accelerator-on-bwb` wf_af500d03-304, agent 6 of 8 sequential agents, attempt 1, state done, model claude-fable-5, effort high; agent start 14:49:44 local, fixture-side log written 14:55. Journal metrics: 36 tool calls, 83,239 journal tokens; API-level per bwb_tokens.json: 146,735 in / 25,082 out (171,817). The precision/recall dimension (detection AND restraint) is exactly the part that required a live AI client; both halves are documented with scriptable cross-checks.

#### Evidence Assessment
- Primary classification: original-log-backed (corroborated live-session-backed via the wf_af500d03-304 journal).
- Confidence: High — original log with complete commands, exact seeded-line cross-checks against a named commit (4ebaef3), an explicit control-run outcome, and journal corroboration; the seeded branch and both reviewer documents remain verifiable in the fixture.
- Evidence proves: 3/3 seeded violations detected with correct paths and line numbers; findings severity-ordered; review strictly read-only on both branches; zero false positives on the control branch; seeded code isolated on the local qa-seeded branch and never pushed.
- Evidence does not prove: the full text quality of the two reviewer documents (not in the package).
- Missing information: `tasks/TASK-004/architecture-boundary-reviewer-qa-seeded-findings.md` and `...-control-baseline.md` (remain in the /tmp fixture).
- Recommended central-team follow-up: restore RUN-055 to the current workbook; if the fixture is to be discarded, archive the two reviewer documents and the qa-seeded diff first.

#### Artifact Inventory
- TC-025 evidence log — shareable filename `logs/original/TC-025.log` — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 15:14:38 local (package copy; fixture original 14:55) — sensitive-content review: none found (only agent-authored QA fixture code is described; no client source bodies) — included: yes
- Workbook row RUN-055 (historical snapshot) — `manifests/workbook-snapshot-20260803-2002.xlsx` (extraction `historical-runs.json`) — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 20:02 local — sensitive-content review: none found — included: yes
- Case definition TC-025 — `manifests/bwb_cases.json` (also cases_map.json, historical-testcases.json) — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 14:17 local — sensitive-content review: none found — included: yes
- Workflow journal wf_af500d03-304 — `transcripts/wf_af500d03-304.json` + script — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamps 2026-08-03 15:12 / 14:18 local — sensitive-content review: none found — included: yes
- Reviewer documents (findings + control baseline) and seeded commit 4ebaef3 on branch qa-seeded — fixture `.../bwb-run/` — Available — original (location known; /tmp persistence not guaranteed) — SHA-256: not recorded — capture timestamp 2026-08-03 (run time) — sensitive-content review: QA fixture code only in the seeded commit; reviewer docs reference client patterns — included: no

### RUN-056 / TC-040

#### Workbook Record
- Recorded result: Partial
- Framework: Symfony (Symfony 6.4.42, PHP 8.5.8)
- AI tool: Claude Code — "Claude Agent SDK (workflow subagent)", model claude-fable-5, tester "Claude Fable 5 (live AI session)", Operating Mode Governed
- Accelerator version: 1.3.1
- Git commit: 3a02537 (accelerator repo HEAD; fixture at install commit 8d6a38b per Starting Repository State)
- Execution date: 2026-08-03 (as recorded)
- Defect reference: DEF-009 ("phpunit.xml.dist declares the PHPUnit 9.3 schema and fails XML validation under the installed PHPUnit 10.5", Low/Low — defects.json key TC-040; a client-configuration finding, not an accelerator defect)
- Workbook evidence link: `Accelerator-TestEvidence/TC-040.log`
- This row exists ONLY in the historical workbook snapshot; it was deleted from the current workbook together with all of RUN-050..RUN-057. Its defect DEF-009 was also deleted from the current Defects sheet (workbook-forensics digest).

#### Test Definition
- Definition status: Authoritative — original source found. TC-040 exists in `manifests/cases_map.json`, `manifests/bwb_cases.json`, and the original Test Cases sheet (`historical-testcases.json`); all agree.
- Title: "Symfony test generator picks the lowest useful test layer and honors the coverage contract" (Category: Testing; Priority: High)
- Objective: the test-generator skill must inspect Required Context, select the lowest useful test layer (voter unit test + protected-route functional test), honor the Coverage Contract, and write a validation map including uncovered rows.
- Preconditions: Symfony accelerator in an app workspace; a security Voter guards an /invitations route; PHPUnit configured; fixtures/factories exist; governed task TASK-002 with a spec listing the access rules; test DB isolated. (Live run recorded precondition drift honestly: no InvitationVoter class and no REST /invitations route exist — the real invitation module is GraphQL; the TASK-002 access-rule spec does not exist in the copy — gap recorded inside the validation map.)
- Input: "/test-generator Add tests for the invitation voter and the protected /invitations route for TASK-002"
- Mandatory assertions (expected_verification): `vendor/bin/phpunit --filter Invitation` → new tests pass; `grep -n '| Requirement | Source | Test | State |' tasks/TASK-002/test-generator-validation.md` → present; no `doctrine:fixtures:load --purge` in the run transcript; the voter unit test does not extend KernelTestCase/WebTestCase.
- Optional assertions: none stated in the definition (sheet Notes flag the deliberate Symfony/Laravel frontmatter divergence as not-a-defect).
- Scriptable assertions: the phpunit filter run, the validation-map grep, the purge-command grep, the extends check.
- Native-client assertions: Required Context inspection, layer-selection judgment, honest mapping of the synthetic scenario onto real code (Execution Type: Hybrid).
- Expected state changes: unit + functional test files under tests/, `tasks/TASK-002/test-generator-validation.md`; no src/ changes.
- Safety expectations: no fixture purge without consent; no production data/credentials/personal data in fixtures; no mocking of platform-dependent Doctrine query behavior; destructive fixture loading only in the test DB.
- Requirement-source: "Symfony/.agents/skills/test-generator/SKILL.md (Required Context, Select The Lowest Useful Layer, Coverage Contract, Test Data, Validation Map)".

#### Historical Evidence
- Evidence status: Available — original.
- Original artifacts: `logs/original/TC-040.log` (shareable name `TC-040.log`; byte-identical copies in `~/Downloads/Accelerator-TestEvidence/` and the fixture root, md5-verified).
- Commands (quoted verbatim from the log, with logged outcomes): `python3 memory-bank/scripts/context.py start --task-id TC-040 --goal "(sanitized)"` (exit=0); `python3 memory-bank/scripts/context.py retrieve "voter unit test functional test invitation access rule phpunit conventions" --task-id TC-040` (exit=0); `context.py update --task-id TC-040 --phase planning ...` (exit=0); `context.py complete --task-id TC-040 ...` → "Working task completed as episode: 6." (exit=0); `php -l tests/Security/AdminVoterInvitationTest.php` → no syntax errors; `php -l tests/Integration/GraphQL/InvitationAccessTest.php` → no syntax errors; `vendor/bin/phpunit --filter AdminVoterInvitation` → "OK: 5 tests, 6 assertions (plus 1 PRE-EXISTING runner warning...)"; `vendor/bin/phpunit --filter Invitation` → "10 tests: 5 unit pass; 5 kernel-booting tests ERROR with Doctrine PDO \"could not find driver\""; control: `vendor/bin/phpunit --filter MandantDeletion` → "same PDOException \"could not find driver\""; `grep -n '| Requirement | Source | Test | State |' tasks/TASK-002/test-generator-validation.md` → line 19; `grep -rn "fixtures:load" <new tests + validation map>` → no matches; `grep -n "extends" tests/Security/AdminVoterInvitationTest.php` → "extends TestCase" only.
- Exit codes: governed lifecycle commands exit 0; numeric exit codes for the phpunit runs are not stated in the log (outcomes quoted above are as logged).
- Assertions: workbook Tests Executed 10 / Passed 5 / Failed 5. Case verifications: validation-map grep present, no purge command, plain-TestCase check — passed; the "new tests pass" verification is explicitly "PARTIAL vs \"new tests pass\"" (unit layer 5/5 green; functional layer blocked by the environment, proven environmental via the identical failure of the pre-existing MandantDeletion test).
- Key output excerpt (verbatim):
  ```
  $ vendor/bin/phpunit --filter AdminVoterInvitation
    -> OK: 5 tests, 6 assertions (plus 1 PRE-EXISTING runner warning: phpunit.xml.dist declares the 9.3 XSD and does not validate against PHPUnit 10.5).
  $ vendor/bin/phpunit --filter Invitation
    -> 10 tests: 5 unit pass; 5 kernel-booting tests ERROR with Doctrine PDO "could not find driver".
  ...
  - The actual blocker is the missing pdo_mysql extension (php -m shows PDO + mysqlnd but no pdo_mysql) — no test database is reachable in this QA copy.
  ```
- Contradictions with workbook: the row exists only in the historical snapshot (deleted from the current workbook), and DEF-009 was deleted from the current Defects sheet. No content conflict between the log and the row: both record the Partial result, the pdo_mysql root cause, the control run, the unexpected non-appearance of the anticipated sodium blocker for PHPUnit, and the TASK-002 spec gap.

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. The log documents no retries; the MandantDeletion control run was a diagnostic, not a retry.

#### Native-Client Evidence
Run executed by a live Claude Code agent: session c9fab6de-739d-497c-ac0d-61bda631def9 (Claude Code v2.1.219), workflow `run-accelerator-on-bwb` wf_af500d03-304, agent 7 of 8 sequential agents, attempt 1, state done, model claude-fable-5, effort high; agent start 14:55:52 local, fixture-side log written 15:06. Journal metrics: 74 tool calls (the largest of the 8), 125,240 journal tokens; API-level per bwb_tokens.json: 208,468 in / 39,750 out (248,218 — the largest of the 8). The honest re-mapping of the synthetic scenario (nonexistent InvitationVoter//invitations) onto the real AdminVoter/GraphQL invite path is native-client judgment documented step-by-step in the log.

#### Evidence Assessment
- Primary classification: original-log-backed (corroborated live-session-backed via the wf_af500d03-304 journal).
- Confidence: High — original log with complete commands and quoted outcomes, an explicit environmental-control experiment, defect record, and journal corroboration; two caveats: phpunit exit codes are recorded as outcomes rather than numeric codes, and the log carries no HEAD line (commit anchored via the workbook row, sibling logs, and the journal).
- Evidence proves: the skill workflow was followed; 5-test voter unit suite is green; the 5 functional-test failures are environmental (missing pdo_mysql/test DB), proven by the identical failure of a pre-existing DB-backed test; no fixture purge was invoked; DEF-009 (stale PHPUnit 9.3 schema) is real and pre-existing.
- Evidence does not prove: that the functional tests would pass in a complete environment (never executed to green anywhere); full content of the validation map.
- Missing information: the two generated test files and `tasks/TASK-002/test-generator-validation.md` (remain in the /tmp fixture); numeric phpunit exit codes.
- Recommended central-team follow-up: restore RUN-056 and DEF-009 to the current workbook; re-run the functional layer in an environment with pdo_mysql + test DB (the bauherrenmappe-qa-fixes branch also contains the DEF-009 schema migration, unpushed).

#### Artifact Inventory
- TC-040 evidence log — shareable filename `logs/original/TC-040.log` — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 15:14:38 local (package copy; fixture original 15:06) — sensitive-content review: describes client test/security conventions and the access-decision chain; no secrets, synthetic test data only — included: yes
- Workbook row RUN-056 (historical snapshot) — `manifests/workbook-snapshot-20260803-2002.xlsx` (extraction `historical-runs.json`) — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 20:02 local — sensitive-content review: none found — included: yes
- Case definition TC-040 — `manifests/bwb_cases.json` (also cases_map.json, historical-testcases.json) — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 14:17 local — sensitive-content review: none found — included: yes
- Defect record DEF-009 (key TC-040) — `manifests/defects.json` — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 15:29 local — sensitive-content review: none found — included: yes
- Workflow journal wf_af500d03-304 — `transcripts/wf_af500d03-304.json` + script — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamps 2026-08-03 15:12 / 14:18 local — sensitive-content review: none found — included: yes
- Generated tests + validation map (`tests/Security/AdminVoterInvitationTest.php`, `tests/Integration/GraphQL/InvitationAccessTest.php`, `tasks/TASK-002/test-generator-validation.md`) — fixture `.../bwb-run/` — Available — original (location known; /tmp persistence not guaranteed) — SHA-256: not recorded — capture timestamp 2026-08-03 (run time) — sensitive-content review: synthetic test data (example.at users), tests exercise client access rules — included: no

### RUN-057 / TC-074

#### Workbook Record
- Recorded result: Pass
- Framework: Symfony (Symfony 6.4.42, PHP 8.5.8)
- AI tool: Claude Code — "Claude Agent SDK (workflow subagent)", model claude-fable-5, tester "Claude Fable 5 (live AI session)", Operating Mode Governed
- Accelerator version: 1.3.1
- Git commit: 3a02537 (accelerator repo HEAD; fixture HEAD=8d6a38b with the reviewed real client commit 627d11f at HEAD~1)
- Execution date: 2026-08-03 (as recorded)
- Defect reference: none ("the reviewed commit is client code, so its findings are the review deliverable, not accelerator defects" — row Comments)
- Workbook evidence link: `Accelerator-TestEvidence/TC-074.log`
- This row exists ONLY in the historical workbook snapshot; it was deleted from the current workbook together with all of RUN-050..RUN-057.

#### Test Definition
- Definition status: Authoritative — original source found. TC-074 exists in `manifests/bwb_cases.json` and in the original 74-row Test Cases sheet (`historical-testcases.json`); the two agree. It is absent from `manifests/cases_map.json` by design — the sheet's Notes say "Added 2026-08-03 for the real-project run".
- Title: "Symfony code-reviewer on a real security commit (BAUMAS-133)" (Category: Code review; Priority: High)
- Objective: the code-reviewer skill must review the actual diff of real commit 627d11f ("changePassword: invalidate the user's other active sessions") with severity-ranked findings referencing real files/lines and no fabrications.
- Preconditions: same installed copy; real commit 627d11f at project HEAD~1.
- Input: "Review the changePassword session-invalidation commit (627d11f) for correctness, security, Controller->Service->Repository boundaries and operational risk."
- Mandatory assertions (expected_verification): every referenced file/line exists in `git show 627d11f`; at least boundary + security dimensions addressed; verdicts justified.
- Optional assertions: a task-prefixed report under tasks/ (optional per Expected Files Changed: "None (review report...)").
- Scriptable assertions: the file/line existence re-verification.
- Native-client assertions: review judgment — severity ranking, mechanism tracing, session-invalidation semantics coverage (Execution Type: Hybrid).
- Expected state changes: none.
- Safety expectations: strictly read-only for src/; "review text must not include secrets or large verbatim client code excerpts (short line references acceptable in local evidence only, not in the workbook)".
- Requirement-source: "Defined during live run on bauherrenmappe (isolated copy)".

#### Historical Evidence
- Evidence status, main log: Withheld — sensitive. The original `TC-074.log` exists (7,344 bytes, 2026-08-03; byte-identical copies verified by md5 during this collection in `~/Downloads/Accelerator-TestEvidence/TC-074.log` and the fixture root `.../bwb-run/TC-074.log`, mtime 15:11:13), but it was deliberately excluded from this package by a prior evidence decision because it contains an unresolved High-severity security finding against a production client system with exact file/line references and remediation guidance. It is the only one of the 8 live-run logs so withheld.
- Evidence status, sanitized replacement: Available — sanitized — `transcripts/RUN-057-TC-074-sanitized.md` (this package), which reproduces the non-sensitive mechanics of the withheld log.
- Original artifacts: withheld original at the locations above (shareable name: none — access via approved channel only; its SHA-256 is recorded in SHA256SUMS.txt under `WITHHELD/TC-074.log` per the sanitized replacement); sanitized replacement `transcripts/RUN-057-TC-074-sanitized.md`.
- Commands (quoted verbatim from the sanitized replacement; governed lifecycle commands all exit 0 as stated there): `python3 memory-bank/scripts/context.py start --task-id TC-074 --goal "Review commit 627d11f ..."` → "Working task started: TC-074."; `python3 memory-bank/scripts/context.py retrieve "... security review" --task-id TC-074`; `python3 memory-bank/scripts/context.py update --task-id TC-074 --phase understanding --progress "..."` → updated; `python3 memory-bank/scripts/context.py complete --task-id TC-074 --outcome "..." --verification "..."` → "Working task completed as episode: 7."; `python3 project-brain/scripts/validate.py --root .` → "Project Brain validation passed."; `git show --stat 627d11f` → 5 files, +90/−3; `php -l` clean on both changed source files; `phpunit` not executable (Doctrine "could not find driver" — no pdo_mysql).
- Exit codes: governed lifecycle commands exit 0 per the sanitized replacement; phpunit blocked by the environment (numeric code not stated).
- Assertions: case verifications passed — every cited file/line re-verified against the tree (sanitized replacement item 4: "no fabricated files"); boundary + security dimensions both addressed. Workbook test counters 8 executed / 0 passed / 8 failed reflect the environment-blocked phpunit attempt (all errored on the missing DB driver; the review is static). 0 safety violations.
- Key output excerpt (verbatim, from the sanitized replacement):
  ```
  6. **Findings profile (counts only)**: 1 High, 2 Medium, 3 Low, 2 open questions;
     verdict aligned with discovered severity.
  7. **Safety**: read-only for `src/`; nothing staged or committed; post-run `git status` deltas
     were governed-CLI bookkeeping plus the evidence log itself; the log states no secrets,
     no `.env` content, and no client source bodies were copied into it.
  8. **Recorded result**: Pass (review executed per skill; findings reference verified real
     files/lines; tests not runnable in the sandbox — environment limitation recorded).
  ```
- Nature of the High finding (at the level already recorded in the retained workbook row, no file/line detail here): the commit's claimed "global logout" does not cover the stateless API token surface — API bearer/refresh tokens survive a password change.
- Contradictions with workbook: the row exists only in the historical snapshot (deleted from the current workbook). The workbook row's Actual Behaviour describes the findings in more detail than this section repeats (it predates the sanitization decision); no factual conflict between row, withheld log, and sanitized replacement was found. Note: during this collection the withheld log was briefly restored into `logs/original/` in error and removed again once the standing withhold decision was found; the package state matches the decision (56 original logs + sanitized replacement).

#### New Reproduction
Performed: No for per-run scope. A consolidated re-run of all mandatory gates was captured 2026-08-04 into logs/reproduction-20260804/ on a dirty tree (post-remediation state, HEAD 3a02537 + 57 modified files); it does not re-prove this historical run. Neither the sanitized replacement nor the workbook row documents any retries.

#### Native-Client Evidence
Run executed by a live Claude Code agent: session c9fab6de-739d-497c-ac0d-61bda631def9 (Claude Code v2.1.219), workflow `run-accelerator-on-bwb` wf_af500d03-304, agent 8 of 8 sequential agents, attempt 1, state done, model claude-fable-5, effort high; agent start 15:06:54 local, fixture-side log written 15:11, last journal progress 15:12:03. Journal metrics: 26 tool calls, 77,945 journal tokens; API-level per bwb_tokens.json: 139,407 in / 20,532 out (159,939). The review's central native-client contribution — tracing that the commit's claimed behavior rests on framework-implicit mechanisms rather than production code — is documented in the sanitized replacement (item 4) and summarized in the workbook row.

#### Evidence Assessment
- Primary classification: original-log-backed (original exists and was verified by hash and inspection during collection, but is Withheld — sensitive from the package; the shareable chain is the sanitized replacement + workbook row + journal, i.e. effectively live-session-backed for package consumers).
- Confidence: High — the original log exists at two verified locations, states the review target commit (627d11f) and fixture HEAD (8d6a38b), records the governed lifecycle with outcomes, and is corroborated by the workflow journal; the package-shareable subset alone (sanitized + row + journal) would rate Medium because the per-finding evidence is withheld.
- Evidence proves: the code-reviewer skill ran end-to-end in governed mode against real commit 627d11f; a severity-ranked review (1 High / 2 Medium / 3 Low, 2 open questions) was produced with every cited file/line re-verified; the run was read-only with nothing staged or committed; test execution was honestly recorded as environment-blocked.
- Evidence does not prove: the validity of each individual finding (client-side confirmation is outside this evidence set); whether the client has been informed of or remediated the High finding — Unknown.
- Missing information: package-shareable per-finding detail (withheld by design); numeric phpunit exit code; client remediation status.
- Recommended central-team follow-up: restore RUN-057 to the current workbook; retrieve the withheld log through an approved confidential channel and verify it against the SHA-256 recorded under `WITHHELD/TC-074.log`; confirm the High finding has been routed to the client (ticket BAUMAS-133 context).

#### Artifact Inventory
- TC-074 evidence log (original) — shareable filename: none (withheld; originals at `~/Downloads/Accelerator-TestEvidence/TC-074.log` and fixture `.../bwb-run/TC-074.log`, md5-identical) — Withheld — sensitive — SHA-256: recorded in package SHA256SUMS.txt (under `WITHHELD/TC-074.log`) — capture timestamp 2026-08-03 15:14 local (Downloads copy; fixture original 15:11:13) — sensitive-content review: contains an unresolved High-severity security finding against a production client system with file/line references and remediation guidance — included: no
- Sanitized replacement — shareable filename `transcripts/RUN-057-TC-074-sanitized.md` — Available — sanitized — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-04 19:47 local — sensitive-content review: sanitized by design, counts and mechanics only — included: yes
- Workbook row RUN-057 (historical snapshot) — `manifests/workbook-snapshot-20260803-2002.xlsx` (extraction `historical-runs.json`) — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 20:02 local — sensitive-content review: summarizes the security findings at workbook level (no secrets; predates the sanitization decision — central team should treat the row's Actual Behaviour as client-confidential) — included: yes
- Case definition TC-074 — `manifests/bwb_cases.json` (also historical-testcases.json) — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamp 2026-08-03 14:17 local — sensitive-content review: names the client project and commit subject only — included: yes
- Workflow journal wf_af500d03-304 — `transcripts/wf_af500d03-304.json` + `transcripts/run-accelerator-on-bwb-wf_af500d03-304.js` — Available — original — SHA-256: recorded in package SHA256SUMS.txt — capture timestamps 2026-08-03 15:12 / 14:18 local — sensitive-content review: none found — included: yes
- Governed-CLI records (task 46b7b501-fca5-4ec3-a891-07e16090f784, handoff, retrieval manifests, episode 7) — fixture `.../bwb-run/project-brain/` — Available — original (location known; /tmp persistence not guaranteed) — SHA-256: not recorded — capture timestamp 2026-08-03 (run time) — sensitive-content review: sanitized outcome strings per log convention — included: no

### Foreign runs out of scope (RUN-20260803-001 .. RUN-20260803-018)

The 18 `RUN-20260803-*` rows present in the current workbook record tester "OpenAI Codex"
(Codex Desktop 0.145.0 / GPT-5 / CI / local macOS host) at Git commit
`7c1cce290a4c87cf4029ce5c09b624d19c661c9a`. That commit does not exist in this repository
(`git cat-file -t 7c1cce290a` fails), no artifact of those runs exists on this machine, and this
tester did not execute them. Evidence status: `Not Applicable` (different tester and environment).
They are inventoried here only because the priority list in TEST-REMEDIATION-AGENT-TASK.md
references five of them (RUN-20260803-011/-012/-014/-016/-018); evidence for those must come from
the Codex tester's environment.

## 5. Missing or Ambiguous Test Definitions

The current workbook's `Test Cases` sheet defines only 19 Test Case IDs, none of which cover the 50 distinct
Test Case IDs referenced by this tester's runs (RUN-001..RUN-057). All of those definitions exist in the
attached original sources, so every entry below is recoverable verbatim:

- `manifests/cases_map.json` — the harness definition map (TC-001..TC-072) actually used to drive the scripted runs;
- `manifests/bwb_cases.json` — live-run case definitions for TC-006/021/022/023/025/040 (TC-073, TC-074 are defined in the workbook snapshot sheet);
- `manifests/workbook-snapshot-20260803-2002.xlsx`, sheet `Test Cases` — the original 74-row definition sheet
  as it existed on 2026-08-03 19:56 local, before the sheet was reduced to 19 rows.

| Test Case ID | Title (from original source) | Status | Source |
|---|---|---|---|
| TC-001 | Claude Code activation banner - Laravel edition (VERSION, loop-counter reset, validation cache) | Authoritative — original source found (still present in current workbook) | cases_map.json; workbook snapshot Test Cases sheet |
| TC-002 | Claude Code activation banner - Symfony edition (bin/console detection) | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-003 | Claude Code activation banner - PHP Core edition (native-PHP folder, path with space) | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-004 | Cursor activation: rules + hooks.json + missing read hook compensated by working-memory.mdc | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-005 | Codex activation: project trust, config.toml features.hooks, hook self-filter early-exit | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-007 | infra-generate full mode passes validate_generated.py including context.py smoke | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-008 | infra-generate merge mode never touches or claims pre-existing team files | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-014 | infra-update classifies safe-update vs needs-decision vs untouchable and persists decision memo | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-015 | infra-update aborts without writes when .infra-manifest.json is missing | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-016 | hook-forge generates the six-hook set with Cursor read-path asymmetry and capsule rule | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-017 | memory-seed degrades {{TARGET_FRAMEWORK}} to generic on a framework-less PHP target | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-028 | bash-validator blocks destructive git and SQL commands with exit 2 and a named pattern | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-029 | bash-validator produces no false positives on escaped quotes, prose, or foreign tool payloads | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-030 | bash-validator fails open with a stderr warning when no JSON extractor exists | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-032 | /security-reviewer performs an OWASP Top 10 audit through its dedicated agent with secret-handl | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-033 | Mirror integrity: build_mirrors.py --check passes clean and detects a hand-edited mirror | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-034 | Edition parity and cross-edition core drift detection (context.py parity) | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-035 | Startup context budget ceiling enforcement (context_budget.py --check) | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-036 | Full local CI reproduction per docs/CI.md (all six jobs) | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-039 | Edition Python infrastructure suites are fully green (132 memory-bank + 124 project-brain tests | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-041 | Manifest integrity: sha256 map, version source, runtime-state exclusion, AGENTS.md stamp | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-042 | validate_generated.py catches broken frontmatter, dangling references, and dead hooks | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-043 | Relative markdown link integrity across the monorepo (check_links.py) | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-044 | Core-changelog gate: shared-core PR without a root CHANGELOG.md entry fails | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-048 | profile-synthesizer emits a schema-conformant profile with version from VERSION file | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-049 | file-naming-validator blocks invalid task/spec/chunk names and validates Codex apply_patch mark | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-050 | loop-detection warns on the 7th edit of a file and blocks on the 10th | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-051 | loop-detection counters are namespaced per repository and reset only for the own repo at Sessio | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-052 | Codex early-exit: read-only payloads never advance the loop counter or trigger validators | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-053 | Command→agent→skill wiring: every slash command spawns its agent and the agent invokes exactly  | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-054 | /git-worktrees creates an isolated worktree without leaking .env or touching shared databases | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-055 | Hook mirrors apply MIRROR_RULES rewrites: per-tool state prefixes and the Cursor/Codex 'path' f | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-056 | infra-scan produces a complete Project Profile from a fake PHP target | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-057 | Non-PHP target: infra-scan gates to stack-adapter and builds an independent sibling generator | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-058 | infra-build one-shot chains scan and generate with checkpoint gating | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-059 | Governed task is auto-provisioned on the first turn flush with branch name as external_id | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-060 | Manual operator progress survives repeated automatic flushes (auto_checkpoint is a separate fie | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-061 | Handoff record carries a governed task across sessions | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-062 | CAS: brain-update with a stale revision is rejected and the record is left untouched | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-063 | Authority: observed→verified promotion accepted, every other transition rejected | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-064 | Second machine: deleted local context.db is repaired by rebind without touching the governed re | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-065 | last-turn-report surfaces a blocked promotion in the next request's Task Capsule | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-066 | Task Capsule respects the 8000-character budget and distills a long prompt whose subject sits i | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-067 | Auto-promotion: resolved verified finding lands in the Memory Bank with a MEM-YYYYMMDD-<hex8> c | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-068 | reindex-bank is idempotent and reconstructs a deleted INDEX.md | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-069 | validate.py accepts a mixed bank of legacy MEM-NNNN and date-based MEM-YYYYMMDD-hex chunks | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-070 | Retrieval ranking: a verified record outranks an observed one and a fresh record outranks a sta | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-071 | Cursor receives the Task Capsule as a gitignored alwaysApply rule with a self-declared stalenes | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-072 | Unified /memory refresh reports all context layers and /checkpoint defers to governed authority | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-006 | Post-install shared context activation: validate, index, FTS5 prerequisite | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-073 | Symfony codebase-mapper on a real client project (bauherrenmappe) | Authoritative — original source found | workbook snapshot Test Cases sheet |
| TC-021 | Symfony architect: Controller -> Service -> Repository decision | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-022 | Symfony doctrine-migration-designer: online-safe NOT NULL rollout | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-023 | Symfony messenger-designer: async welcome email design | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-025 | Symfony architecture-boundary-reviewer: seeded layer violations | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-040 | Symfony test generator picks the lowest useful test layer and honors the coverage contract | Authoritative — original source found | cases_map.json; workbook snapshot Test Cases sheet |
| TC-074 | Symfony code-reviewer on a real security commit (BAUMAS-133) | Authoritative — original source found | workbook snapshot Test Cases sheet |

## 6. Contradictions and Corrections Proposed

A full programmatic diff of the 2026-08-03 19:56 workbook snapshot against the current workbook is
attached as `transcripts/analysis/workbook-forensics.md`. The contradictions that affect the
integrity of this tester's records:

1. **Eight runs deleted from the current workbook.**
   - Run IDs: RUN-050, RUN-051, RUN-052, RUN-053, RUN-054, RUN-055, RUN-056, RUN-057.
   - Workbook statement (current): the runs are absent from "Test Runs", "Quality Metrics",
     "Efficiency & Tokens" and "Context & Brain".
   - Evidence statement: all 8 runs exist in the 2026-08-03 snapshot (attached), have original
     evidence logs (TC-006/021/022/023/025/040/073/074, mtime 2026-08-03 15:14 local), a workflow
     journal (`transcripts/wf_af500d03-304.json`) and a transcript record of their execution
     window (11:18–12:12 UTC).
   - Proposed correction: restore the 8 rows (and their analytics rows) from the snapshot.
   - Historical record must remain preserved: yes — the snapshot is attached unmodified.

2. **Three defects deleted with their parent runs.** DEF-007 (RUN-052/TC-021), DEF-008
   (RUN-053/TC-022), DEF-009 (RUN-056/TC-040) are gone from the current "Defects" sheet.
   Evidence: present in the snapshot's Defects sheet and in `manifests/defects.json`.
   Proposed correction: restore the three rows.

3. **Defect-ID collisions.** The current workbook re-issues DEF-001..DEF-004 to four foreign
   Codex-run defects while this tester's DEF-001..DEF-004 (RUN-004, RUN-007, RUN-008, RUN-014)
   are still present — the same ID now resolves to two unrelated defects. Per the task document,
   defects must be identified as Run ID + Test Case ID + Defect ID; every bare "Defect ID"
   cross-reference in our surviving rows is now ambiguous.
   Proposed correction: renumber the Codex defects (or namespace them, e.g. DEF-AI-00x).

4. **73 of 74 test-case definitions removed.** The current "Test Cases" sheet keeps only TC-001 of
   this tester's definitions (plus 18 new TC-AI-*). 48 of the 49 surviving runs reference
   undefined TC IDs. All 74 original definitions are attached (see §5).
   Proposed correction: restore the "Test Cases" sheet rows for every TC id referenced by a run.

5. **Mechanical rewrite artifacts in surviving rows** (no content loss detected): header padded
   30→70 columns with empty "Column 31..70"; `Execution Date` re-serialized `2026-08-03` →
   `2026-08-03 00:00:00`; six numeric columns re-typed int→float; whitespace collapsed in two
   "Actual Behaviour" cells (RUN-018, RUN-024); a stray duplicated header row sits mid-sheet in
   "Quality Metrics". No correction required beyond awareness; the snapshot preserves the
   original formatting.

6. **Foreign rows under a non-existent commit.** The 18 added `RUN-20260803-*` rows are pinned to
   commit `7c1cce290a4c87cf4029ce5c09b624d19c661c9a`, which does not exist in this repository
   (`git cat-file -t` fails), and their "Accelerator Version" column holds a branch name instead
   of a version. No correction proposed by this tester (foreign records); flagged for the central
   team's provenance tracking.

## 7. Consolidated Artifact Inventory

Full SHA-256 for every file is in `SHA256SUMS.txt` (sha256sum format, verifiable with
`sha256sum -c SHA256SUMS.txt`). Capture timestamp = file mtime (local, UTC+3).

| Artifact | Run IDs | Type | Status | SHA-256 (first 16) | Captured | Included |
|---|---|---|---|---|---|---|
| `logs/original/TC-001.log` | RUN-001 | Original run log | Available — original | `223bf7d013013c30` | 2026-08-03 11:31 | yes |
| `logs/original/TC-002.log` | RUN-002 | Original run log | Available — original | `adcec68a082b39f0` | 2026-08-03 11:31 | yes |
| `logs/original/TC-003.log` | RUN-003 | Original run log | Available — original | `6bc395d78e68d457` | 2026-08-03 11:31 | yes |
| `logs/original/TC-004.log` | RUN-004 | Original run log | Available — original | `7f27bffef458c5a5` | 2026-08-03 11:31 | yes |
| `logs/original/TC-005.log` | RUN-005 | Original run log | Available — original | `2096be5f9b34ed14` | 2026-08-03 11:31 | yes |
| `logs/original/TC-006.log` | RUN-050 | Original run log | Available — original | `56e74f5098bf4791` | 2026-08-03 15:14 | yes |
| `logs/original/TC-007.log` | RUN-006 | Original run log | Available — original | `d7a81f1e39ff3234` | 2026-08-03 11:31 | yes |
| `logs/original/TC-008.log` | RUN-007 | Original run log | Available — original | `3afa2d11cf40c84f` | 2026-08-03 11:31 | yes |
| `logs/original/TC-014.log` | RUN-008 | Original run log | Available — original | `738808a646cb6061` | 2026-08-03 11:31 | yes |
| `logs/original/TC-015.log` | RUN-009 | Original run log | Available — original | `d76e53aacd12a0f1` | 2026-08-03 11:31 | yes |
| `logs/original/TC-016.log` | RUN-010 | Original run log | Available — original | `9da4c4e2fdbcc446` | 2026-08-03 11:31 | yes |
| `logs/original/TC-017.log` | RUN-011 | Original run log | Available — original | `716117eded8690ca` | 2026-08-03 11:31 | yes |
| `logs/original/TC-021.log` | RUN-052 | Original run log | Available — original | `1b598d6a12592de5` | 2026-08-03 15:14 | yes |
| `logs/original/TC-022.log` | RUN-053 | Original run log | Available — original | `8fae8827d29da957` | 2026-08-03 15:14 | yes |
| `logs/original/TC-023.log` | RUN-054 | Original run log | Available — original | `61206f3a99cb7154` | 2026-08-03 15:14 | yes |
| `logs/original/TC-025.log` | RUN-055 | Original run log | Available — original | `3bb783b650492a3f` | 2026-08-03 15:14 | yes |
| `logs/original/TC-028.log` | RUN-012 | Original run log | Available — original | `9fd0e01d6413cf7a` | 2026-08-03 11:31 | yes |
| `logs/original/TC-029.log` | RUN-013 | Original run log | Available — original | `8fa86cf6e861e085` | 2026-08-03 11:31 | yes |
| `logs/original/TC-030.log` | RUN-014 | Original run log | Available — original | `597abb087daba847` | 2026-08-03 11:31 | yes |
| `logs/original/TC-032.log` | RUN-015 | Original run log | Available — original | `d1df3b7a742f32c9` | 2026-08-03 11:31 | yes |
| `logs/original/TC-033.log` | RUN-016 | Original run log | Available — original | `8ef2e7f5372167e8` | 2026-08-03 11:31 | yes |
| `logs/original/TC-034.log` | RUN-017 | Original run log | Available — original | `247e30728900a7d1` | 2026-08-03 11:31 | yes |
| `logs/original/TC-035.log` | RUN-018 | Original run log | Available — original | `6eb4d302a3b3fcfd` | 2026-08-03 11:31 | yes |
| `logs/original/TC-036.log` | RUN-019 | Original run log | Available — original | `eb16fe44b0b7de99` | 2026-08-03 11:31 | yes |
| `logs/original/TC-039.log` | RUN-020 | Original run log | Available — original | `6e4b2efc729f7ea2` | 2026-08-03 11:31 | yes |
| `logs/original/TC-040.log` | RUN-056 | Original run log | Available — original | `0571a18c255c4aa3` | 2026-08-03 15:14 | yes |
| `logs/original/TC-041.log` | RUN-021 | Original run log | Available — original | `89ea1cebca35def5` | 2026-08-03 11:31 | yes |
| `logs/original/TC-042.log` | RUN-022 | Original run log | Available — original | `8319520f21506508` | 2026-08-03 11:31 | yes |
| `logs/original/TC-043.log` | RUN-023 | Original run log | Available — original | `00adc9a1bad990f0` | 2026-08-03 11:31 | yes |
| `logs/original/TC-044.log` | RUN-024 | Original run log | Available — original | `a8d47bc9ea2f2074` | 2026-08-03 11:31 | yes |
| `logs/original/TC-048.log` | RUN-025 | Original run log | Available — original | `3425ddd26a9e1e06` | 2026-08-03 11:31 | yes |
| `logs/original/TC-049.log` | RUN-026 | Original run log | Available — original | `53c4c8e121d94e35` | 2026-08-03 11:31 | yes |
| `logs/original/TC-050.log` | RUN-027 | Original run log | Available — original | `9417c14377777cc0` | 2026-08-03 11:31 | yes |
| `logs/original/TC-051.log` | RUN-028 | Original run log | Available — original | `7a588966f20b539a` | 2026-08-03 11:31 | yes |
| `logs/original/TC-052.log` | RUN-029 | Original run log | Available — original | `9ea657148d0569b4` | 2026-08-03 11:31 | yes |
| `logs/original/TC-053.log` | RUN-030 | Original run log | Available — original | `89fc7eeac195f90f` | 2026-08-03 11:31 | yes |
| `logs/original/TC-054.log` | RUN-031 | Original run log | Available — original | `1904dbb99e8b383d` | 2026-08-03 11:31 | yes |
| `logs/original/TC-055.log` | RUN-032 | Original run log | Available — original | `25b3db3ef264b447` | 2026-08-03 11:31 | yes |
| `logs/original/TC-056.log` | RUN-033 | Original run log | Available — original | `abedaf573405abe0` | 2026-08-03 11:31 | yes |
| `logs/original/TC-057.log` | RUN-034 | Original run log | Available — original | `62b7e5e42efedc2f` | 2026-08-03 11:31 | yes |
| `logs/original/TC-058.log` | RUN-035 | Original run log | Available — original | `a4b42427d0eb93b5` | 2026-08-03 11:31 | yes |
| `logs/original/TC-059.log` | RUN-036 | Original run log | Available — original | `e58e15ef5a80d4ba` | 2026-08-03 11:31 | yes |
| `logs/original/TC-060.log` | RUN-037 | Original run log | Available — original | `dda0fd87fbcf2363` | 2026-08-03 11:31 | yes |
| `logs/original/TC-061.log` | RUN-038 | Original run log | Available — original | `fcd79129eb12200a` | 2026-08-03 11:31 | yes |
| `logs/original/TC-062.log` | RUN-039 | Original run log | Available — original | `3896f85d33c6c551` | 2026-08-03 11:31 | yes |
| `logs/original/TC-063.log` | RUN-040 | Original run log | Available — original | `a603408732e5ddf2` | 2026-08-03 11:31 | yes |
| `logs/original/TC-064.log` | RUN-041 | Original run log | Available — original | `13bba671102cd46f` | 2026-08-03 11:31 | yes |
| `logs/original/TC-065.log` | RUN-042 | Original run log | Available — original | `e4c8d1e07804c1ad` | 2026-08-03 11:31 | yes |
| `logs/original/TC-066.log` | RUN-043 | Original run log | Available — original | `42d4bdad31aa82e3` | 2026-08-03 11:31 | yes |
| `logs/original/TC-067.log` | RUN-044 | Original run log | Available — original | `86a85cf8191a6704` | 2026-08-03 11:31 | yes |
| `logs/original/TC-068.log` | RUN-045 | Original run log | Available — original | `5b0cd9edd191df27` | 2026-08-03 11:31 | yes |
| `logs/original/TC-069.log` | RUN-046 | Original run log | Available — original | `53efe271944882b0` | 2026-08-03 11:31 | yes |
| `logs/original/TC-070.log` | RUN-047 | Original run log | Available — original | `da7b507568d5569b` | 2026-08-03 11:31 | yes |
| `logs/original/TC-071.log` | RUN-048 | Original run log | Available — original | `1b4358c7768f62ad` | 2026-08-03 11:31 | yes |
| `logs/original/TC-072.log` | RUN-049 | Original run log | Available — original | `38122c3a7b9d6224` | 2026-08-03 11:31 | yes |
| `logs/original/TC-073.log` | RUN-051 | Original run log | Available — original | `8d47b9e435dbc202` | 2026-08-03 15:14 | yes |
| `logs/reproduction-20260804/00-environment.log` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `7cb4d91ab25961c9` | 2026-08-04 19:42 | yes |
| `logs/reproduction-20260804/00-git-status-after.txt` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `02a107c7e6ff6a64` | 2026-08-04 19:44 | yes |
| `logs/reproduction-20260804/00-git-status-before.txt` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `02a107c7e6ff6a64` | 2026-08-04 19:42 | yes |
| `logs/reproduction-20260804/10-laravel-memory-bank-tests.log` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `5b2b9d423e85c108` | 2026-08-04 19:42 | yes |
| `logs/reproduction-20260804/11-laravel-project-brain-tests.log` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `a5876241ba59e559` | 2026-08-04 19:42 | yes |
| `logs/reproduction-20260804/12-symfony-memory-bank-tests.log` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `8643c7b6d83bff65` | 2026-08-04 19:43 | yes |
| `logs/reproduction-20260804/13-symfony-project-brain-tests.log` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `ffd254dce289859e` | 2026-08-04 19:43 | yes |
| `logs/reproduction-20260804/14-phpcore-memory-bank-tests.log` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `f76c6b21937c3ee8` | 2026-08-04 19:43 | yes |
| `logs/reproduction-20260804/15-phpcore-project-brain-tests.log` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `55ca47777d4d5a4e` | 2026-08-04 19:44 | yes |
| `logs/reproduction-20260804/16-infrastructure-creator-tests.log` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `f7ca58ba546f77de` | 2026-08-04 19:44 | yes |
| `logs/reproduction-20260804/20-laravel-parity.log` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `34c3822418326140` | 2026-08-04 19:44 | yes |
| `logs/reproduction-20260804/21-laravel-parity-cross.log` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `f782acd87aa5e5da` | 2026-08-04 19:44 | yes |
| `logs/reproduction-20260804/22-symfony-parity.log` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `34c3822418326140` | 2026-08-04 19:44 | yes |
| `logs/reproduction-20260804/23-symfony-parity-cross.log` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `f782acd87aa5e5da` | 2026-08-04 19:44 | yes |
| `logs/reproduction-20260804/24-phpcore-parity.log` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `34c3822418326140` | 2026-08-04 19:44 | yes |
| `logs/reproduction-20260804/25-phpcore-parity-cross.log` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `f782acd87aa5e5da` | 2026-08-04 19:44 | yes |
| `logs/reproduction-20260804/30-build-mirrors-check.log` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `835e86f8e4b0c4b0` | 2026-08-04 19:44 | yes |
| `logs/reproduction-20260804/31-check-links.log` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `e38052ebb42dfecc` | 2026-08-04 19:44 | yes |
| `logs/reproduction-20260804/32-context-budget.log` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `f491454b313c7a01` | 2026-08-04 19:44 | yes |
| `logs/reproduction-20260804/99-summary.json` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `32dcca166ed5c145` | 2026-08-04 19:44 | yes |
| `logs/reproduction-20260804/reproduce.sh` | All (context) | New reproduction (2026-08-04, dirty tree) | Available — newly reproduced | `d288a60a5507c439` | 2026-08-04 19:41 | yes |
| `manifests/bwb_cases.json` | RUN-050..057 | Live-run case definitions | Available — original | `545349e1bf527198` | 2026-08-03 14:17 | yes |
| `manifests/bwb_tokens.json` | RUN-050..057 | Live-run token accounting | Available — original | `6290166148f232e5` | 2026-08-03 15:13 | yes |
| `manifests/cases_map.json` | RUN-001..049 | Harness test-case definitions | Available — original | `56e6332c7c2efa45` | 2026-08-03 11:14 | yes |
| `manifests/defects.json` | Defect-bearing runs | Defect records (2026-08-03 15:29) | Available — original | `17b172e406c85064` | 2026-08-03 15:29 | yes |
| `manifests/historical-othersheets.json` | All | Package file | Available — original | `35f4742526dbf15b` | 2026-08-04 19:40 | yes |
| `manifests/historical-runs.json` | All | Package file | Available — original | `a2b31d35cfaf73ea` | 2026-08-04 19:40 | yes |
| `manifests/historical-testcases.json` | All | Package file | Available — original | `e38a4d0e9bc5bac0` | 2026-08-04 19:40 | yes |
| `manifests/our-runs.json` | All | Package file | Available — original | `4e91e2076654671e` | 2026-08-04 19:37 | yes |
| `manifests/workbook-backup-20260803-1105.xlsx` | RUN-001..057 | Workbook snapshot (2026-08-03) | Available — original | `5156b5b7b85907c7` | 2026-08-03 11:05 | yes |
| `manifests/workbook-snapshot-20260803-2002.xlsx` | RUN-001..057 | Workbook snapshot (2026-08-03) | Available — original | `e6846459f794eca8` | 2026-08-03 20:02 | yes |
| `transcripts/RUN-057-TC-074-sanitized.md` | RUN-057 | Sanitized replacement for withheld log | Available — sanitized | `cfaa9151cb2d0115` | 2026-08-04 19:47 | yes |
| `transcripts/accelerator-test-cases-wf_3c021cce-042.js` | RUN-001..057 | Workflow script (original) | Available — original | `207b36c7bdb3c9ac` | 2026-08-03 10:57 | yes |
| `transcripts/analysis/accelerator-test-cases.md` | All | Evidence-collection analysis digest | Available — newly reproduced | `3cd44dcf755ba81d` | 2026-08-04 19:47 | yes |
| `transcripts/analysis/bwb-live-run.md` | All | Evidence-collection analysis digest | Available — newly reproduced | `f90fcec27dff9410` | 2026-08-04 19:47 | yes |
| `transcripts/analysis/execute-test-cases.md` | All | Evidence-collection analysis digest | Available — newly reproduced | `288bb096ddcf81b3` | 2026-08-04 19:48 | yes |
| `transcripts/analysis/fix-qa-findings.md` | All | Evidence-collection analysis digest | Available — newly reproduced | `a5736886604c775f` | 2026-08-04 19:47 | yes |
| `transcripts/analysis/session-timeline.md` | All | Evidence-collection analysis digest | Available — newly reproduced | `70223d23aa9f16e1` | 2026-08-04 19:47 | yes |
| `transcripts/analysis/workbook-forensics.md` | All | Evidence-collection analysis digest | Available — newly reproduced | `208cf1b31559d35e` | 2026-08-04 19:47 | yes |
| `transcripts/execute-test-cases-wf_e8e050fd-8be.js` | RUN-001..057 | Workflow script (original) | Available — original | `66c18d1a604bcc85` | 2026-08-03 11:15 | yes |
| `transcripts/fix-qa-findings-wf_9ab92e4f-965.js` | RUN-001..057 | Workflow script (original) | Available — original | `20fcae7b32c0f63d` | 2026-08-03 15:31 | yes |
| `transcripts/run-accelerator-on-bwb-wf_af500d03-304.js` | RUN-001..057 | Workflow script (original) | Available — original | `2aebae0e19de9c6d` | 2026-08-03 14:18 | yes |
| `transcripts/wf_3c021cce-042.json` | RUN-001..057 | Workflow journal (original) | Available — original | `002c72ce73f69c55` | 2026-08-03 11:05 | yes |
| `transcripts/wf_9ab92e4f-965.json` | RUN-001..057 | Workflow journal (original) | Available — original | `005838b7dc891bf7` | 2026-08-03 16:28 | yes |
| `transcripts/wf_af500d03-304.json` | RUN-001..057 | Workflow journal (original) | Available — original | `218b0bcaf5e86deb` | 2026-08-03 15:12 | yes |
| `transcripts/wf_e8e050fd-8be.json` | RUN-001..057 | Workflow journal (original) | Available — original | `ef1f4451cb2f8964` | 2026-08-03 11:30 | yes |
| `WITHHELD/TC-074.log` (not in package; original at `~/Downloads/Accelerator-TestEvidence/TC-074.log`) | RUN-057 | Original run log | Withheld — sensitive | `52d93fa5be2e86ca` | 2026-08-03 15:14 | no |

## 8. Unavailable Evidence

| Run ID(s) | Missing artifact | Last known location | Reason unavailable | Can it be reproduced |
|---|---|---|---|---|
| RUN-001..049 (mutating cases) | Temp working copies / IC-generated fixtures used by mutation steps | `/tmp/…` per-case temp dirs, 2026-08-03 | Deleted with /tmp lifecycle | Yes — the harness (`manifests/cases_map.json`) records the exact commands; fixtures are regenerable |
| RUN-001..057 | Screenshots | Never existed | Headless scripted/agent execution; no GUI captures were made during the runs | Partially — future runs could capture terminal output; historical state cannot be re-screenshotted |
| RUN-004, RUN-010, RUN-048 (Cursor-flavored cases) | Native Cursor client behavior | Never captured | Runs executed in scripted mode; the Cursor client itself was not driven | Yes, by a tester with Cursor installed |
| RUN-005, RUN-029 (Codex-flavored cases) | Native Codex client behavior | Never captured | Same — scripted mode only | Yes, by a tester with Codex installed |
| RUN-057 | Raw TC-074.log in this package | `~/Downloads/Accelerator-TestEvidence/TC-074.log` (exists, intact) | **Withheld — sensitive** (unresolved High-severity security finding about a real client product) | N/A — original preserved locally; SHA-256 recorded; sanitized replacement attached |
| RUN-050..057 | bwb fixture directory (`bwb-run/`) | Testing-session scratchpad (still exists locally) | Not attached — contains a full copy of client source code | N/A — fixture can be rebuilt from client repo @ 627d11f + install commit recipe in the logs |
| All | Google Sheets edit history between 2026-08-03 19:56 and 2026-08-04 | Google Sheets version history | Not accessible to this evidence agent | The central team can inspect Sheets revision history directly |

## 9. Submission Checklist

- [x] Every assigned Run ID has a report section (57 sections in §4)
- [x] Exact commits are recorded (3a02537 monorepo; 627d11f / 8d6a38b fixture; reproduction on 3a02537 + dirty tree)
- [x] Original and new evidence are distinguished (Historical Evidence vs New Reproduction subsections; reproduction logs in a separate directory)
- [x] Static and native-client evidence are distinguished (per-run Native-Client Evidence subsection; E-01 scripted vs E-02 live)
- [x] Missing artifacts are explicit (§8; per-run statuses use the required vocabulary)
- [x] No secrets or private data are included (TC-074.log withheld; bwb fixture not attached; logs reviewed — they record that `.env` was never opened)
- [x] Checksums are recorded (`SHA256SUMS.txt`, including the withheld artifact)
- [x] All attachments are referenced (§7, §10)
- [x] Accelerator source files were not modified (evidence collection ran read-only; reproduction verified `git status` unchanged before/after — `logs/reproduction-20260804/00-git-status-{before,after}.txt` identical)
- [x] Workbook was not edited (both xlsx files copied, never written)
- [x] Nothing was staged, committed, or pushed

## 10. Files Sent to the Central Team

- `ACCELERATOR-TEST-EVIDENCE-REPORT.md` (this file)
- `logs/original/TC-001.log`
- `logs/original/TC-002.log`
- `logs/original/TC-003.log`
- `logs/original/TC-004.log`
- `logs/original/TC-005.log`
- `logs/original/TC-006.log`
- `logs/original/TC-007.log`
- `logs/original/TC-008.log`
- `logs/original/TC-014.log`
- `logs/original/TC-015.log`
- `logs/original/TC-016.log`
- `logs/original/TC-017.log`
- `logs/original/TC-021.log`
- `logs/original/TC-022.log`
- `logs/original/TC-023.log`
- `logs/original/TC-025.log`
- `logs/original/TC-028.log`
- `logs/original/TC-029.log`
- `logs/original/TC-030.log`
- `logs/original/TC-032.log`
- `logs/original/TC-033.log`
- `logs/original/TC-034.log`
- `logs/original/TC-035.log`
- `logs/original/TC-036.log`
- `logs/original/TC-039.log`
- `logs/original/TC-040.log`
- `logs/original/TC-041.log`
- `logs/original/TC-042.log`
- `logs/original/TC-043.log`
- `logs/original/TC-044.log`
- `logs/original/TC-048.log`
- `logs/original/TC-049.log`
- `logs/original/TC-050.log`
- `logs/original/TC-051.log`
- `logs/original/TC-052.log`
- `logs/original/TC-053.log`
- `logs/original/TC-054.log`
- `logs/original/TC-055.log`
- `logs/original/TC-056.log`
- `logs/original/TC-057.log`
- `logs/original/TC-058.log`
- `logs/original/TC-059.log`
- `logs/original/TC-060.log`
- `logs/original/TC-061.log`
- `logs/original/TC-062.log`
- `logs/original/TC-063.log`
- `logs/original/TC-064.log`
- `logs/original/TC-065.log`
- `logs/original/TC-066.log`
- `logs/original/TC-067.log`
- `logs/original/TC-068.log`
- `logs/original/TC-069.log`
- `logs/original/TC-070.log`
- `logs/original/TC-071.log`
- `logs/original/TC-072.log`
- `logs/original/TC-073.log`
- `logs/reproduction-20260804/00-environment.log`
- `logs/reproduction-20260804/00-git-status-after.txt`
- `logs/reproduction-20260804/00-git-status-before.txt`
- `logs/reproduction-20260804/10-laravel-memory-bank-tests.log`
- `logs/reproduction-20260804/11-laravel-project-brain-tests.log`
- `logs/reproduction-20260804/12-symfony-memory-bank-tests.log`
- `logs/reproduction-20260804/13-symfony-project-brain-tests.log`
- `logs/reproduction-20260804/14-phpcore-memory-bank-tests.log`
- `logs/reproduction-20260804/15-phpcore-project-brain-tests.log`
- `logs/reproduction-20260804/16-infrastructure-creator-tests.log`
- `logs/reproduction-20260804/20-laravel-parity.log`
- `logs/reproduction-20260804/21-laravel-parity-cross.log`
- `logs/reproduction-20260804/22-symfony-parity.log`
- `logs/reproduction-20260804/23-symfony-parity-cross.log`
- `logs/reproduction-20260804/24-phpcore-parity.log`
- `logs/reproduction-20260804/25-phpcore-parity-cross.log`
- `logs/reproduction-20260804/30-build-mirrors-check.log`
- `logs/reproduction-20260804/31-check-links.log`
- `logs/reproduction-20260804/32-context-budget.log`
- `logs/reproduction-20260804/99-summary.json`
- `logs/reproduction-20260804/reproduce.sh`
- `manifests/bwb_cases.json`
- `manifests/bwb_tokens.json`
- `manifests/cases_map.json`
- `manifests/defects.json`
- `manifests/historical-othersheets.json`
- `manifests/historical-runs.json`
- `manifests/historical-testcases.json`
- `manifests/our-runs.json`
- `manifests/workbook-backup-20260803-1105.xlsx`
- `manifests/workbook-snapshot-20260803-2002.xlsx`
- `transcripts/RUN-057-TC-074-sanitized.md`
- `transcripts/accelerator-test-cases-wf_3c021cce-042.js`
- `transcripts/analysis/accelerator-test-cases.md`
- `transcripts/analysis/bwb-live-run.md`
- `transcripts/analysis/execute-test-cases.md`
- `transcripts/analysis/fix-qa-findings.md`
- `transcripts/analysis/session-timeline.md`
- `transcripts/analysis/workbook-forensics.md`
- `transcripts/execute-test-cases-wf_e8e050fd-8be.js`
- `transcripts/fix-qa-findings-wf_9ab92e4f-965.js`
- `transcripts/run-accelerator-on-bwb-wf_af500d03-304.js`
- `transcripts/wf_3c021cce-042.json`
- `transcripts/wf_9ab92e4f-965.json`
- `transcripts/wf_af500d03-304.json`
- `transcripts/wf_e8e050fd-8be.json`
- `SHA256SUMS.txt`

Not sent: raw `TC-074.log` (withheld — sensitive, §8), the `bwb-run/` fixture (client source code), `.env` files (never read), the full Claude Code session transcript (contains unrelated private work; targeted excerpts appear in `transcripts/analysis/session-timeline.md`).
