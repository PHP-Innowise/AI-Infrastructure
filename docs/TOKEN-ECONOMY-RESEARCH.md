> Исследование: какой инструмент интегрировать для экономии токенов.
> База: main@fdbc537 + 2031 транскрипт Claude Code (~798 МБ), 47 643 реальных вызова API.
> Метод и все скрипты воспроизводимы; [M] = измерено, остальное помечено как оценка.
> Сводка по-русски — в ответе сессии; ниже полный отчёт.

# Which tool to integrate for token savings — final report

**Basis:** repo `main@fdbc537` (verified: `git log` HEAD, 2,143 tracked files) and `/home/aliaksei/.claude/projects` (2,126 `.jsonl`, of which 95 are `journal.jsonl` → 2,031 transcript files: 106 main sessions + 1,925 subagent files). Every `[M]` below was recomputed by me this session; scripts in `/tmp/claude-1000/-home-aliaksei-Desktop-AI-Infrastructure/d2d48060-545f-4138-a7a9-b0ff44d1eff6/scratchpad/` (`final_spend.py`, `final_stats.py`, `ctx400.py`, `tools2.py`, `genset.py`, `attrib2.py`). The corpus is live (this session appends), so totals drift <0.1 % between runs. Two economies kept separate throughout: **(1) monorepo** maintainers, **(2) consuming project** with an edition installed.

---

## ANSWER IN ONE PARAGRAPH

**Integrate no third-party tool for token savings.** Measured on 47,643 real API calls, uncached input — the *only* quantity every code-packing tool (code2prompt, repomix, gitingest, yek, ai-digest, files-to-prompt) attacks — is **0.66 % of cost-weighted spend** [M]; 55.4 % is re-reading context that is already resident and a further 25.0 % is writing it. Nothing you install touches that. The money is in **what enters context and how long sessions run**, and the three highest-value actions are all in-house edits worth more than any candidate tool: (a) stop feeding 966 machine-generated mirror files into every diff an agent reads — measured **−63,850 cl100k tokens on one real PR**, ≈16–24 % of an AI-Infrastructure main session [M]; (b) stop stamping a UTC timestamp and a per-call manifest path into Cursor's `alwaysApply` rule, which is the single re-injected-every-turn surface in the product and is therefore billed on a **T²/2 curve** (807× at this corpus's median main-session length, T=109 for AI-Infrastructure) — but the fix must be made in the canon `_WM_DELIVERY_*_CURSOR` constants, not in the `.cursor/hooks` mirrors, and the JSON-validation guard must be removed in the same change or Cursor's working memory silently stops rendering; and (c) put MCP context-scoping rules where a consuming project can actually see them — the new "Optional MCP Integrations" section lives only in `README_EN.md` / `README_RU.md`, **neither of which the installer ships** (`grep -c MCP` = 0 in all four per-edition `README.md` and all four `AGENTS.md` [M]). If you want one *new* capability, it is a ~120-line stdlib Python cost-attribution script over `~/.claude/projects` — **not** OpenTelemetry: contrary to the earlier draft, `attributionSkill` / `attributionMcpServer` / `attributionMcpTool` / `attributionPlugin` / `attributionAgent` are all already in the transcripts on the same records as `message.usage` (10,975 / 10,146 / 10,146 / 10,182 / 79,715 occurrences [M]), and I ran the attribution: 7.17 % of all spend is skill-attributed, `engineering:code-review` alone is 5.97 % [M].

---

## MEASURED SPEND MAP

### Correction that changes every headline

Claude Code writes **one transcript record per content block**, each repeating `message.usage` verbatim; streaming intermediates carry a placeholder `output_tokens` (5–8) while the terminal record (`stop_reason` set) carries the true output. Deduping on `(message.id, usage-tuple)` — the method behind the earlier draft — splits one API call into two and double-bills its `cache_read`/`cache_creation`.

I ran **both** methods on the same snapshot [M]:

| | pair-dedup (earlier draft's method) | **terminal-per-`message.id` (correct)** |
|---|---:|---:|
| calls | 68,198 | **47,643** |
| grand total BTE | 1,475,905,712 | **1,206,911,712** (−22.3 %) |
| `uncached input` share | 0.92 % | **0.66 %** |
| subagent share of spend | 58.14 % | **48.81 %** |
| split groups (>1 usage tuple / message.id) | 20,555 — **all in subagents, zero in main** | — |

Main-session totals are **byte-identical under both methods** (617,770,979 BTE), which is the proof: main sessions have zero split groups. The earlier draft's main-session figures were right; everything it said about subagents and about the corpus total was inflated.

### The map (weights: uncached 1.0 · cache_read 0.1 · 5m-write 1.25 · 1h-write 2.0 · output 5.0; unit = base-input-token equivalent, **BTE**)

| component | raw tokens | weighted | **share** |
|---|---:|---:|---:|
| cache read | 6,688,782,173 | 668,878,217 | **55.42 %** |
| output | 45,632,063 | 228,160,315 | 18.90 % |
| cache write 5m | 134,738,928 | 168,423,660 | 13.95 % |
| cache write 1h | 66,753,591 | 133,507,182 | 11.06 % |
| **uncached input** | **7,942,338** | **7,942,338** | **0.66 %** |
| **total** | | **1,206,911,712** | |

**TTL stratification is absolute** [M, reproduced]: main sessions wrote `ephemeral_1h` on 13,355 calls and `ephemeral_5m` on **0**; subagents wrote `5m` on 34,228 and `1h` on **0**. Zero overlap in either direction. A flat 1.25× weight understates main-session writes by 60 %.

| stratum | n | calls | BTE | share | T median | T mean | T p90 | session BTE mean / median | peak ctx median |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| main | 106 | 13,371 | 617,770,979 | **51.19 %** | **78** | 126.1 | 370 | 5,828,028 / **1,885,721** | 196,240 |
| subagent | 1,923 | 34,272 | 589,140,733 | **48.81 %** | 12 | 17.8 | 38 | 306,365 / 195,996 | 68,832 |
| — AI-Infrastructure main only | 9 | 1,412 | 60,917,090 | — | **109** | 156.9 | — | 6,768,566 / **3,318,397** | 265,006 |

Derived [M]: mean main turn **46,202** BTE · mean subagent turn **17,190** BTE · ratio **2.69×** · **subagent breakeven = 6.63 displaced main turns** (not 9.6) · AI-Infrastructure main turn 43,142 BTE.

Context-size tail [M]: turns carrying ≥400 k of context are **7.78 % of turns / 25.34 % of spend** across all strata — but **27.56 % of turns / 49.31 % of spend among main sessions**, where a compaction policy would apply. (Subagents: 0.07 % / 0.23 %.)

### Multipliers — and the rule for using them

For a session of T turns:

| where a token lives | formula | T=78 (all-main median) | T=109 (AI-Infra median) | T=126 (all-main mean) |
|---|---|---:|---:|---:|
| fixed prefix (MCP schemas, AGENTS.md, skill descriptions, SessionStart) | `2.0 + 0.1(T−1)` | 9.7× | **12.8×** | 14.5× |
| tool result arriving at turn 5 | `2.0 + 0.1(T−5)` | 9.3× | **12.4×** | 14.1× |
| re-injected every prompt (`UserPromptSubmit`, Cursor `alwaysApply`) | `2.0T + 0.1·T(T−1)/2` | 456× | **807×** | 1,040× |

**Rule:** a median-derived multiplier must be divided by a *median* session, a mean-derived one by a *mean* session. Mixing them (as the earlier draft did on every "% of a mean main session") inflates or deflates by up to 3.1× — the main-session BTE distribution is skewed mean 5,828,028 / median 1,885,721.

**Both formulas are floors, not identities** [M]: over 91 main sessions with T ≥ 10, `Σ cache_write ÷ peak context` = median 0.92, mean 1.60, p90 2.92, max 12.48. The tail (>1) is 1 h TTL expiry plus compaction — the prefix is genuinely re-written 3×+ in long sessions, and compaction truncates long-carried payloads that the carry multiplier assumes stay resident.

### Tool-result payloads [M, mine]

16,136 results, 63,932,354 B. The ≥20 KB tail is **503 results / 34,293,071 B (53.6 % of all payload bytes from 3.1 % of results)**.

| tool | tail bytes | tail count | mean | median |
|---|---:|---:|---:|---:|
| `mcp__claude-in-chrome__browser_batch` | 46.02 % | 34.0 % | 46,308 B | **9,687 B** |
| `mcp__claude-in-chrome__computer` (screenshots) | 20.05 % | 20.7 % | 26,472 B | 544 B |
| **`Read` (built-in)** | 12.73 % | 15.5 % | 5,757 B | 1,965 B |
| `Bash` | 4.34 % | 10.3 % | 1,517 B | 723 B |

Browser MCP is ~1 % of calls and **~76 % of the heavy tail by bytes**. But note the medians: `browser_batch` is mean-46 k / median-9.7 k — a tail-driven distribution. The built-in `Read` is the largest non-MCP tail contributor, so "the heavy tail is an MCP problem" is only 3/4 true.

---

## RANKED RECOMMENDATIONS

### #1 — Generated `.gitattributes` (per edition) + rewrite `review-pr` Step 2
**What.** Extend `scripts/build_mirrors.py --write` to also emit `<edition>/.gitattributes`, one exact edition-relative path per generated file, `-diff linguist-generated=true`; `--check` verifies it like every other mirror. **Per-edition, not repo-root**: gitattributes patterns cannot contain spaces, and one edition is literally `PHP Core/`. I validated the mechanism in a scratch repo including the space-in-name case [M]. Then change `.agents/skills/review-pr/SKILL.md` Step 2 from bare `gh pr diff <number>` ("# The full diff", line 48) to a `--name-only` triage followed by a mirror-excluded local diff.

**Why a generated file and not three globs:** no glob is correct. Of 1,444 tracked files under `.claude|.cursor|.codex`, only **966 are generated (45.08 % of the 2,143 tracked)**; **478 are canon living inside those directories** [M, computed by executing `build_mirrors.expected_mirror_files` over all four editions]. Canon includes `.claude/agents` (141), `.claude/commands` (125), `.claude/hooks` (26), `.claude/{DOD,GOLDEN-PRINCIPLES,STABILIZATION}.md`, `.claude/settings.json`, `.codex/config.toml`, `.codex/hooks.json`, all 12 `.cursor/rules/*.mdc`, and even 60 files under `.claude/skills` + 60 under `.cursor/skills` (the vendored `skill-creator` tree, which the mirror rules skip). **The earlier draft's `*/.claude/** -diff` would have marked 431 canonical files — including the shared-core hooks that `check_core_changelog.sh` polices — as generated binaries.** 5 of the 966 have a space in their relative path (`SKILL FLOW.md`); emit `SKILL?FLOW.md` for those or accept 5 textual diffs.

**Measured impact on PR #11** (merge `90fa3a7`, parents `767dad8`/`215fe9f`), `code2prompt 4.3.0 --encoding cl100k`:

| variant | files | bytes | cl100k tokens |
|---|---:|---:|---:|
| full `git diff` | 206 | 941,788 | **225,441** |
| excluding the 66 truly-generated files (32.04 %) | 140 | 671,484 | **161,591** |
| **saving** | | −28.70 % | **−63,850 t (−28.3 %)** |

(The earlier draft excluded all 106 files under tool directories — 40 of them canon — and reported −34.2 % / 76 K. That over-claims by 19 %.)

**Carried cost, economy 1, statistics matched:** `63,850 × 12.4 = 791,740 BTE = 23.9 % of a median AI-Infrastructure main session`; or `63,850 × 17.19 = 1,097,582 BTE = 16.2 % of the mean`. **Range: 16–24 %.** Realizability checked: AI-Infrastructure main peak contexts are `[122k, 144k, 194k, 260k, 265k, 334k, 392k, 526k, 735k]` — 6 of 9 sessions can hold the post-exclusion 161 k payload [M].

**Economy** 1. **Burden** ~40 lines in `build_mirrors.py` + one canon skill edit ×3 editions + `.claude/commands/review-pr.md` ×3 + regeneration. Zero new prerequisites. **Tool-or-in-house:** in-house; the tool is `git`, already a hard prerequisite. **CI:** `check_core_changelog.sh` uses `git diff --name-only`, which still lists `-diff` files unchanged — verified in a scratch repo [M]. `build_mirrors.py` is under `scripts/`, which matches `CORE_PATTERN`, so this PR **does** need a root `CHANGELOG.md` entry.

### #2 — Cursor working-memory rule: kill the two per-turn invalidators (at canon)
**What.** In `memory-bank/scripts/context_retrieval.py`, edit **both** `_WM_DELIVERY_STOP_CURSOR` (line 149) and `_WM_DELIVERY_SESSION_CURSOR` (line 196): (a) delete `rendered: %s` / `$(date -u …)` from the header `printf`; (b) drop `--json` from the `hook-context` calls **and delete the JSON-validation guard in the same block**; (c) update the corresponding assertions in `memory-bank/tests/test_hooks.py`; (d) `build_mirrors.py --write`; (e) root `CHANGELOG.md` entry.

**Three feasibility corrections to the earlier draft, all verified:**
1. `.cursor/hooks` is a **mirror** of canonical `.claude/hooks` (`context_retrieval.py:283-286`; `docs/TOOL-INTEGRATIONS.md:111-113`). `Laravel/.cursor/hooks/working-memory-write.sh` and `local-context.sh` are both in the generated set [M]. Editing them fails `mirrors` + `parity` CI and is silently discarded by `--write`.
2. Removing `--json` **without** removing the guard at `working-memory-write.sh:55-58` (`… ! printf '%s' "$CAPSULE" | python3 -c 'json.load(sys.stdin)' … then CAPSULE_STATUS=1`) makes `CAPSULE_STATUS=1`, which falls through both branches: `working-memory.mdc` is **never written**. The rule would shrink not to 692 B but to 0 B.
3. There are **6 render sites, not 3** [M]: `{Laravel,Symfony,PHP Core}/.cursor/hooks/working-memory-write.sh:71` **and** `.../local-context.sh:{224,204,206}`. `test_hooks.py:553` already names both (`RENDER_HOOKS`). Fixing only the Stop hook leaves session-start re-invalidation and two renderers writing the same file in two formats.
4. `memory-bank/scripts/*.py` and `memory-bank/tests/*.py` are in `CROSS_EDITION_CORE_MANIFEST` (`context_retrieval.py:447-454`) and are **not** in `CROSS_EDITION_ALLOWED_DRIFT` — so the edit must be byte-identical across 3 editions, ×2 files = 6 edited files, verified by `parity --cross-edition` in CI.

**Why it matters.** The generated `.cursor/rules/working-memory.mdc` is `alwaysApply: true`, re-rendered every turn. It has two independent per-turn invalidators: the UTC timestamp, and `manifest` — a fresh per-call path `f"{manifest_id}.json"` (`context_retrieval.py:1861`) embedded in the JSON. It also carries a **4-view capsule**: `categories` / `procedural` / `semantic` / `selected` (+ empty `episodic`) at `context_retrieval.py:1903-1915`, all slices of the same dicts. Claude Code's `UserPromptSubmit` hook pipes rendered prose and pays none of this [M].

**Impact — stated honestly.** The rule file is **gitignored** (`Laravel/.gitignore:17`) and does not exist anywhere in the tree (`find` → 0 [M]), so the 5,708 B → 692 B / −87.9 % figures are **prior-eval only, not reproducible here**. What *is* verified is the mechanism and the multiplier: a per-prompt-reinjected token is billed **807× at T=109**, and a monotonically-increasing timestamp guarantees zero prefix reuse for whatever follows it. Even with no caching assumption at all, the floor is "bytes re-sent every turn ×T". **I do not have a defensible token number for this and will not invent one.**

**Economy** 2 (the product). **Burden** 6 canon files + 3 test files + regeneration + CHANGELOG. **In-house.** *A timestamp inside an always-applied rule is indefensible independent of any caching inference.*

### #3 — Put the MCP context rules where a consuming project can read them
**What.** The new "Optional MCP Integrations" section exists only at `README_EN.md:304` and its RU mirror. Measured: `grep -c MCP` = **0** in `Laravel/README.md`, `Symfony/README.md`, `PHP Core/README.md`, `Infrastructure-Creator/README.md`, root `README.md`, and all four `AGENTS.md`; `README_EN.md` / `README_RU.md` appear in **no** install inventory [M]. So today the guidance reaches economy 1 only. Port the section (plus new rules 7–9, §MCP below) into the per-edition `README.md` — which *is* inventory-listed — and into `Infrastructure-Creator/README.md`, the edition that generates accelerators for other people's projects.

**Anchors, corrected** [M]: `README_EN.md` "Security rules:" at **347**, rules 1–6 span **349–359**, append after 359 ✓. `README_RU.md` "Правила безопасности:" at **352**, rules span **354–364**, append after **364** — the earlier draft's 362 would insert between rules 4 and 5.

**Economy** 2. **Burden** three bullets ×5 files + one section port. **In-house.** No contradiction is introduced: `docs/TOOL-INTEGRATIONS.md:206` ("The shipped config does not require an MCP server") and `Laravel/.codex/config.toml:13,16` (MCP commented out) are compatible with *optional* ≠ *shipped* [M].

### #4 — A stdlib cost-attribution script over `~/.claude/projects` (replaces the OTel recommendation)
**What.** ~120 lines, Python 3 stdlib, developer-local, never shipped: walk `*.jsonl`, one call per `message.id` from the terminal record, weight by TTL, group by `attributionSkill` / `attributionMcpServer` / `attributionMcpTool` / `attributionPlugin` / `attributionAgent` / `isSidechain`.

**Why this and not OpenTelemetry.** The earlier draft's entire justification for OTel — *"attributes that do not exist anywhere in the transcript JSON… no local script can produce it"* — is **false**. Measured [M]: 317 files contain `attributionSkill`; corpus-wide `attributionSkill` 10,975, `attributionMcpServer` 10,146, `attributionMcpTool` 10,146, `attributionPlugin` 10,182, `attributionAgent` 79,715 — **every single one on an `assistant` record that also carries `message.usage`** (0 exceptions among assistant records; 3 non-assistant records total). I ran the attribution end-to-end: **7.17 % of all spend is skill-attributed; `engineering:code-review` = 72,118,039 BTE = 5.97 % of everything, over 3,999 calls** [M]. Three of the five items the earlier draft parked behind "wait 2–4 weeks for OTel" are answerable today.

OTel keeps a genuine, narrower advantage — per-request granularity, a cost counter, `effort`, and coverage of output tokens that never reach transcripts — but it is an optional extra, not the gate. Two further blockers on the earlier form: the CI `lint` job runs `python3 -m json.tool` on every tracked `*.json` (`.github/workflows/ci.yml`, "Validate JSON"), and JSON has no comments — I confirmed `json.tool` exits 1 on `{ // c \n "a":1 }` [M] — so **"ship commented-out in `.claude/settings.json`" is impossible**. Document the env vars in `docs/OPERATIONS.md` instead; `.codex/config.toml` is the only file in this repo that can carry commented config.

**Economy** both. **Burden** one scratch script, zero repo files, zero prerequisites. **In-house.** **Impact on tokens: zero** — it is the instrument that makes everything else falsifiable.

### #5 — Two policy lines in `docs/OPERATIONS.md`
1. **Subagent breakeven = 6.63 mean main turns** [M] (mean run 306,365 BTE ÷ mean main turn 46,202). Median subagent run is 12 turns of its own. Fan-out below that threshold loses money. Nothing in the repo states any threshold.
2. **Compact deliberately at ~400 k of context.** Main-session turns above that line are 27.56 % of turns and **49.31 % of main spend** [M].
3. **Correct canon/mirror wording** — the earlier draft's "never Read or Edit under `.claude/`, `.cursor/`, `.codex/`" is wrong and would forbid #2 and #4: canon is `.agents/skills` for skills, `.claude/{hooks,commands,agents}` + `.claude/{DOD,GOLDEN-PRINCIPLES,STABILIZATION}.md` + `.claude/settings.json` + `.codex/{config.toml,hooks.json}` + `.cursor/rules/*.mdc` for the rest; generated are the 966 files `build_mirrors.py` emits.

**Economy** both. **Burden** three sentences. **In-house.** **Impact** large but unattributable until #4 runs.

---

## THE ONE TOOL

**None.** No third-party tool clears the bar, and it is not close: the entire packing class attacks **0.66 %** of spend [M]; the structural-code class (ctags, tree-sitter, ast-grep, SCIP) optimises a map that the shipped retrieval engine barely reads — `snippet(documents, 5, …, 32)` is a **32-FTS-token** window (`context_retrieval.py:1548`) and `BM25_WEIGHTS = (1.0,1.0,1.0,2.0,8.0,1.0)` weights `summary` 8× against `content` 1× (`:494-496`) [M]; and the measurement class is dominated by data already sitting on disk (#4).

**If "integrate" must produce a repo artifact: the generated `.gitattributes` (#1).** It is the only intervention that is (a) measured end-to-end by me rather than estimated (225,441 → 161,591 cl100k tokens on a real PR), (b) worth 16–24 % of an AI-Infrastructure main session, (c) specific to this repo's defining property — 45.1 % of tracked files are machine-generated and pass `--check`, therefore carry zero independent information — and (d) adds no prerequisite. Its tool is `git`.

**One thing genuinely missing from the toolchain:** `gh` is **not installed** (`which gh` → not found [M]) while `Laravel/.agents/skills/review-pr/SKILL.md:20` declares it a prerequisite and Step 2 is three `gh` commands. `apt install gh` locally; it does **not** enter `docs/ADOPTION.md`.

---

## MCP SERVER GUIDANCE (ranked by context cost)

**Comparison unit — measured in cl100k, not bytes/4** [M]: Laravel `AGENTS.md` = **2,706 t** (12,887 B; bytes/4 says 3,222 → the repo's own estimator runs **+19.1 %** high here), 44 skill `name + description` entries = **2,288 t** (11,412 B; bytes/4 +24.7 %), SessionStart banner = **171 t** (622 B). **Accelerator fixed prefix ≈ 5,165 cl100k t.** Carried at 12.8× (AI-Infra median) = 66,112 BTE/session; at 9.7× (all-main median) = 50,101 BTE.

Tool counts below are **[M] counted by me from the current upstream READMEs**. Schema *token* sizes are **ESTIMATE** at ~500 t/tool (corroborated by public measurements of GitHub MCP at 42 k–55 k over 87–93 tools ⇒ 452–591 t/tool). **Zero README-listed servers appear anywhere in this corpus** [M] — the only `attributionMcpServer` values are `claude-in-chrome` (3,725), `ccd_session` (3,290), `Claude Browser` (1,378), `plugin:unite-cms:unite-cms-knowledge` (1,327), `Claude in Chrome` (394), `nimbalyst` (28), `browseros` (4). I have no first-party schema measurement for any of them.

| # | server / configuration | tools | est. schema t | ≈ accelerator prefixes | verdict |
|---|---|---:|---:|---:|---|
| 1 | **Context7** (`resolve-library-id`, `query-docs`) | **2** [M] | 400–1,000 | **0.08–0.19×** | **ENABLE** |
| 2 | GitHub MCP `--read-only` + `--toolsets context,repos,pull_requests` | 32 before read-only filtering [M] | ~16,000 → less after filtering | 3.1× → lower | **ENABLE, scoped** |
| 3 | Read-only database MCP | no published count | — | — | conditional — **payload risk** |
| 4 | Sentry **or** Datadog (never both) | no published count | — | — | conditional |
| 5 | Figma | no published count | — | — | design-led teams only |
| 6 | Linear **or** Atlassian Rovo | no published count | — | — | low value per token |
| 7 | AWS MCP | no published count | — | — | AWS-backed projects only |
| 8 | Playwright MCP, no `--caps` | **24** (23 core + 1 tabs) [M] | ~12,000 | 2.3× | **payload risk** |
| 9 | GitHub MCP, default 5 toolsets (`context,repos,issues,pull_requests,users`) | **42** [M] | ~21,000 | 4.1× | avoid |
| 10 | Playwright MCP, all caps | **69** [M] | ~34,500 | 6.7× | reject |
| 11 | **GitHub MCP `--toolsets all`** | **87** local (90 incl. 3 remote-only) [M] | 42,000–55,000 (measured, third-party) | **8.1–10.6×** | **reject** |

Rows 3–7 previously carried invented tool-count ranges. Sentry's README names skills and points at MCP Inspector "List Tools"; Atlassian documents product × intent groupings. **No published count exists — do not print one.** All nine README links resolve.

### The headline a PHP team needs
**One unscoped GitHub MCP server costs 8–11× the entire accelerator prefix** — `AGENTS.md`, all 44 skill descriptions and the session banner combined. Documented, verifiable saving from scoping: **87 tools → 32** (`--toolsets context,repos,pull_requests`) = **2.7× fewer tool definitions** [M], before `--read-only` removes more. GitHub's README does **not** annotate tools read-only vs write, so the post-filter count is **not derivable from documentation** — the earlier draft's "~12 tools / 7.75× cheaper" was unsourced. `--read-only` / `GITHUB_READ_ONLY=1` and `--toolsets` / `GITHUB_TOOLSETS` (env wins; `default` = the 5 above; special value `all`) are all verbatim-confirmed from the current README [M].

### Context7 — enable, but claim no saving
2 tools, ~0.1× the accelerator prefix, and it targets the one problem that lies **outside** this repository: the delta between the model's cutoff and the framework version actually installed in a client project. That is in no index here (`codebase/` ships empty; the index ingests markdown only). Against it, and the README must say so: **it is a fetching tool** — a 5,000-token pull at turn 5 of a T=109 session costs `5,000 × 12.4 = 62,000 BTE`, **~1.4 mean main turns and ~60× its own schema**. Its API is `query-docs(libraryId, query)` with **no length parameter** [M] — the caller cannot bound the response, so the only lever is query targeting; do not promise a knob that does not exist. Every edition already ships a curated `WebFetch` allowlist over this same surface at zero marginal prefix cost. **Adopt because the fixed cost is near-noise, not because there is evidence of saving. Whether it nets positive is unmeasurable from any data available here.**

### Playwright and the database MCP: ranked on payload, not schema
Browser MCP has the worst tool-result profile in the corpus: `browser_batch` mean **46,308 B**, and browser tools are **~76 % of the ≥20 KB tail by bytes** [M]. At `12.4×` carry, a single mean-sized `browser_batch` result costs ~11,600 t × 12.4 ≈ **144,000 BTE ≈ 3.3 mean main turns**. Playwright's 24-tool schema is paid back in roughly one call. Note the honest caveat: `browser_batch`'s **median is 9,687 B** — this is a tail-driven mean. The shipped `browser-verify` skill (present in all 3 PHP editions) is entirely tool-agnostic: it never names Playwright, never bounds a snapshot, never caps a screenshot. **If Playwright MCP stays in the recommended list, `browser-verify` must gain a bounding rule in the same change.** Same argument for a read-only database MCP.

### Draft rules 7–9 (append to "Security rules", retitled "Security and context rules")
> **7.** Scope every server to the smallest tool set it needs. Tool definitions sit at the front of the model's context, are re-read on every turn, and are rewritten in full whenever the cache expires. `github-mcp-server` defaults to `context, repos, issues, pull_requests, users` (42 tools); `--toolsets all` is 87. Do not use `all`. Run with `--read-only` (`GITHUB_READ_ONLY=1`) unless a workflow needs writes — that removes every write tool from the schema and satisfies rule 4 at the same time. Run `playwright-mcp` without `--caps` (24 tools) unless a capability is genuinely required; all capabilities is 69. A server left at its widest setting can occupy roughly ten times the context of this entire accelerator.
> **8.** Decide the server set before starting a session. Tool definitions are tier-1 in the prompt-cache hierarchy (`tools → system → messages`): adding or removing a server mid-session invalidates the whole cache, and the session's accumulated context is re-established at full price.
> **9.** What a server *returns* usually costs more than what it *declares*, because a large result is carried and re-read for the rest of the session. Prefer bounded, structured output; for anything that can return a page, a log stream, or a query result set, constrain the request at the call site — noting that Context7's `query-docs` has no size parameter, so there the only lever is a narrower query. Verify rather than estimate: `/context` reports the MCP tools row, and `attributionMcpServer` in the session transcript attributes cost per server.

---

## REJECTED (one line each)

| candidate | reason |
|---|---|
| **code2prompt 4.3.0** (installed) | Pure concatenator against a 0.66 % surface. Keep strictly as an offline measuring instrument — every cl100k figure in this report came from it — never shipped, never invoked by a skill or hook. |
| **repomix** | Only real reducer in its class, and it compresses none of what the index ingests: 16 tree-sitter grammars, **no markdown**, while this monorepo tracks **0 PHP files**; the "~70 %" is an unmethodical README line; ships an MCP server (README rule 6 forbids); `engines.node >= 22` vs Node 20.11.1 here — it would not run as installed. |
| **gitingest / files-to-prompt / yek / ai-digest / repo2txt / uithub** | Concatenators against 0.66 %. Also: gitingest `main` last moved 2025-08-16; files-to-prompt ~18 months stale; repo2txt declares no licence; ai-digest counts Anthropic tokens with the **Claude 2** tokenizer; uithub and hosted gitingest would upload `Task/Epics/*_SPEC.md` — named-client specs — to a third party. |
| **universal-ctags** | 8.8× the token cost of a stdlib map for **+0.41 pp** method recall; native binary + C libs absent from a Python-only CI. (Licence is **GPL-2.0-or-later**, not "-only" — the earlier draft overstated this; it is a weak objection either way for an executed tool.) |
| **tree-sitter / py-tree-sitter** | Same +0.41 pp; grammar ABI churn is a recurring tax; not in distro repos → `docs/ADOPTION.md` grows. |
| **ast-grep / Semgrep** | Emit findings, not a map; overlap `rg`, which agents already have; large native artifact (npm linux-x64 unpacked ~51 MiB at 0.45.1; the GitHub release zip is ~7.9 MiB — quote one and say which). |
| **SCIP/scip-php / nikic PHP-Parser / composer classmap** | Require `composer install` in the consuming project — direct violation of `docs/ADOPTION.md:46-49`. |
| **PHP `token_get_all()`** | Rejected on house style, not merit: a second language in shipped scripts, 6 more mirrors, unrunnable in the PHP-free CI, for +0.41 pp. |
| **Any "codebase map" dependency** | The shipped engine returns a **32-FTS-token** snippet window with `summary` weighted 8× vs `content` 1× [M] — a richer map delivers zero extra tokens through the shipped path. |
| **tokei / scc / cloc** | Whole output reproduced by `git ls-files \| wc \| awk` with more per-directory detail. |
| **sqlite-vec / txtai / chromadb** | Per-platform `.so` or a multi-hundred-MB model at install; turns `git clone` into a network install; fails hard rather than degrading. |
| **ccusage** | Reads exactly the `.jsonl` files #4 reads, adds a Node prerequisite to a stdlib house style, and does not implement per-skill/per-MCP attribution. (Its rejection stands, but *not* on "the data isn't in transcripts" — it is.) |
| **delta / difftastic** | Both *add* bytes to diff output; a repo whose top line item is diff volume should not adopt a diff pretty-printer. |
| **Generic filesystem / memory MCP** | Already forbidden by existing README rule 6. |
| **OpenTelemetry as the *primary* measurement recommendation** | Demoted, not rejected: the attribution it was justified by is already in transcripts [M], and "ship commented-out in `.claude/settings.json`" is impossible under the `json.tool` lint gate [M]. Document the env vars in `docs/OPERATIONS.md` as an optional extra. |

---

## SEQUENCE

**First — one PR, individually revertible. Must include a root `CHANGELOG.md` entry** (`CORE_PATTERN` at `check_core_changelog.sh:35` matches both `scripts/` and `(Laravel\|Symfony\|PHP Core)/memory-bank/(scripts\|tests)/`):
1. `build_mirrors.py`: emit + `--check` per-edition `.gitattributes`; regenerate. Re-run `check_core_changelog.sh` against the merge base (verified unaffected: `--name-only` still lists `-diff` files [M]).
2. `review-pr` Step 2 rewritten in `.agents/skills/` ×3 + `.claude/commands/review-pr.md` ×3, then `--write`. (`.agents/skills/` is in `CROSS_EDITION_ALLOWED_DRIFT`, so per-edition wording may differ — but the edit must still be made in all three.)
3. Cursor fix at canon: `_WM_DELIVERY_STOP_CURSOR` + `_WM_DELIVERY_SESSION_CURSOR` in 3 byte-identical `context_retrieval.py`, remove the JSON guard, update 3 `test_hooks.py`, regenerate.
4. Port the MCP section + rules 7–9 into the 4 per-edition `README.md` (inventory-listed) and update `README_EN.md`/`README_RU.md` at the corrected anchors (after EN 359 / RU 364).
5. `docs/OPERATIONS.md`: subagent breakeven 6.63 turns; compact at ~400 k; the corrected canon/mirror map; the optional OTel env vars.
6. `apt install gh` developer-locally.
7. **Only after step 3** add the `lint` regression `grep -rlnE 'date -u' */.cursor/hooks/*.sh` → must be empty (today it matches 6 files [M]; if step 3 fixed only the Stop hook it would still match 3 and red-light its own PR). Update `docs/CI.md:75-88`, which enumerates lint steps verbatim.
8. Add a `browser-verify` bounding rule if Playwright stays in the recommended list.

**Gated on measurement — but the gate is #4, which runs today, not a 2–4-week OTel soak:**
- Per-skill cost across 44–47 skills (`attributionSkill`; 7.17 % of spend is already attributable [M]). Only then is the CI gap — frontmatter gated at 4,015 cl100k t while 44 canon `SKILL.md` bodies are **81,719 cl100k t (20.4×, not 25.6×)** [M] — worth acting on.
- Whether the 48.81 % subagent bucket clears the 6.63-turn breakeven (`isSidechain` + `attributionAgent`).
- Whether Context7 nets positive, once any consuming project runs it (`attributionMcpServer`).
- Collapsing the 4-view capsule in `context_retrieval.py:1903-1915` — subsumed by #2 for Cursor, ~0 for Claude Code/Codex (already on the rendered path). Needs test churn for an unmeasured gain; do not touch it first.
- `context_budget.py`: gate `name + description` (2,288 cl100k t for Laravel) rather than full frontmatter (18,446 B / 4,015 cl100k t, **1.5–1.8× over-strict**), switch off bytes/4 (+19–25 % measured error on these files [M]), and relabel the fictional `TOTAL (monorepo root) 149 skills` row.

**Free, this week, needs an interactive session (I could not run either):** `/context` and `/doctor` in each edition. They report the MCP tools row and the skill-listing budget, and resolve whether the documented "1 % of the context window" budget is characters or tokens — which decides whether skill descriptions are being silently truncated today.

**Optional, only with a named reason:** Context7 (economy 2, PHP teams). Scoped read-only GitHub MCP (economy 2 only — for the monorepo, `gh` costs 0 prefix tokens and does the same job). repomix for a one-shot review of an *external* client codebase, never in-repo, never its MCP mode, and only on a Node ≥22 machine.

---

## UNVERIFIED CLAIMS THE READER MUST CHECK BEFORE ACTING

1. **Every MCP schema token size in the §MCP table is an ESTIMATE** at ~500 t/tool. Tool *counts* are [M] (I counted them from the current upstream READMEs); token sizes are not. Rows 3–7 have **no published tool count at all** and no number should be quoted for them. Zero of these servers appear in this corpus.
2. **The Cursor rule's byte figures (5,708 B → 692 B, −87.9 %) are prior-eval only.** `.cursor/rules/working-memory.mdc` is gitignored and absent from the tree [M] — I could not regenerate it. The *mechanism* (timestamp + per-call manifest path + 4-view capsule) is verified at the cited lines; the magnitude is not.
3. **Cursor's prompt-cache behaviour is undocumented and unobservable from here.** Any multiplier applied to #2 is inference. The floor — bytes re-sent every turn — does not depend on it.
4. **Cursor hook I/O contract.** `Laravel/.cursor/hooks.json` registers `sessionStart → local-context.sh`, and that script emits **bare stdout**, not `{"additional_context": …}` [M]. Whether Cursor ingests bare stdout at `sessionStart` — and whether `sessionStart`/`postToolUse` can inject context at all, which would be a structurally better fix than #2 — I could **not** verify: `cursor.com/docs/agent/hooks` returned 15 bytes to `curl` (JS-rendered). **Check this before pricing #2**; if `sessionStart` injection works, moving the capsule there beats shrinking the always-applied rule.
5. **`.gitattributes` and GitHub's server-side diff.** `-diff` is honoured by *local* git — verified [M]. Whether `gh pr diff` / the `.diff` endpoint honours it is **UNVERIFIED** (`gh` is not installed here). The 941,788 B / 225,441 t measurement is a local `git diff`. Also, `gh pr diff` takes no pathspec, so the rewritten Step 2 must fetch the PR ref and run local `git diff` — spell that out in the skill.
6. **Economy 2 is entirely unmeasured.** No transcript in this corpus comes from a PHP project with an edition installed. T, prefix size and tool mix there are unknown; T=78/109, the 5,165-token prefix and the 46,202 BTE main turn are properties of *this* harness. Repo-side quantities (capsule design, hook wiring, skill-listing bytes, README text, tool counts) transfer by construction; session-shape quantities do not.
7. **Carry multipliers are floors.** `Σ cache_write ÷ peak context` has mean 1.60 and p90 2.92 [M] — the prefix is re-written more than once in long sessions, and compaction truncates long-carried payloads that the tool-result multiplier assumes stay resident. Both errors point the same way (prefix cost understated, tool-result carry overstated in long sessions).
8. **Output tokens are undercounted in transcripts.** Streaming intermediates carry placeholders; roughly three-quarters of billed output never reaches the file. The 18.90 % output share is a floor.
9. **`hookAdditionalContext` is populated in 0 records** — hook-injected bytes as *billed* are not observable; the 622 B / 171 t SessionStart figure is what the hook *emits*.
10. **The skill-listing budget's unit** (documented as "1 % of the model's context window", with `SLASH_COMMAND_TOOL_CHAR_BUDGET` as a character override and a 1,536-character per-entry description cap) is characters-vs-tokens ambiguous. `/context` in an interactive session resolves it, and it changes what `context_budget.py` should gate.