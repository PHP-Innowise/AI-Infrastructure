# Open-Source Kit (Kit 3)

Third bootstrap path, alongside Infrastructure-Creator (Kit 1, bespoke
generated) and the ready-made editions (Kit 2, Laravel/Symfony/PHP Core/
WordPress). Kit 3 admits **other people's** tools into a client project under
gates we own.

What Kit 3 actually is — four things, in order:

1. **Record** — what was checked, what was found, at which pinned ref, and how it was installed.
2. **Isolation** — namespace, roster, hook precedence.
3. **Measurement** — cost on the same ruler as our own editions.
4. **Removal** — uninstall with no residue. We leave; the client keeps what they chose.

The tool list is a consequence of the gates, not a decision taken ahead of them.

Two files matter:

| Path | Role |
| --- | --- |
| `resources.json` | The browsable catalog. What exists. |
| `registry/*.json` | The risk dossier. What review found. |

**The registry describes; it does not forbid.** Nothing here refuses a tool.
This script installs nothing either way — it records a choice and prints a
command a human runs — so a refusal would only block writing the choice down,
and an install that happened anyway would then be missing from the audit trail
entirely. What the registry produces is a dossier: what was checked, what was
found, and what would resolve each open item. The team decides.

What that buys you: the selector names what review found before you commit, and
`.kit3-manifest.json` carries it into the client project, where it is visible in
a diff long after the terminal output is gone. 13 of 15 catalog entries have no
registry file yet and are reported as `NOT REVIEWED`.

---

## Test it end to end

Every command below runs from the repository root and is safe — nothing here
downloads or executes third-party code, and nothing writes outside a temp
directory you name.

### 1. The catalog lists

```bash
python3 scripts/install_open_source_kit.py --list
```

Expect entries grouped by category (`marketplace`, `mcp`, `skills`,
`subagents`, `discovery-index`), each with id, target tools, and description.

### 2. The registry reports what it found per candidate

```bash
python3 scripts/validate_registry.py
```

Expect, per entry:

```
KNOWN_RISKS   	graphify      	Graphify-Labs/graphify
  tier 3, default disabled, pinned_ref NONE
  risks found:    collisions, maintenance_ownership, measurability
  open questions: data_egress, pinning, uninstall, auto_update
  scores:         automation_depth=5, token_efficiency=0, integration_coverage=5, trust_signals=2
  intersections:  3 recorded

VALID	2 registry entr(ies)
```

`VALID` means the files are well-formed and every stored status matches what
its gates compute. It says nothing about whether a tool is safe — read the
status on each line.

Both committed entries are `KNOWN_RISKS` today. That is a finding, not a
failure: each risk carries a `resolves_by` naming exactly what would close it.

### 3. The CI gate is silent on success

```bash
python3 scripts/validate_registry.py --check ; echo "exit=$?"
```

Expect `VALID	2 registry entr(ies)` and `exit=0`.

### 4. Prove the status cannot be faked

This is the property the whole registry rests on. Flip a status without
touching the gates that produced it:

```bash
cp install/open-source-kit/registry/graphify.json /tmp/graphify.backup.json

python3 - <<'PY'
import json
p = "install/open-source-kit/registry/graphify.json"
d = json.load(open(p))
d["status"] = "clear"              # gates left untouched
json.dump(d, open(p, "w"), indent=2)
PY

python3 scripts/validate_registry.py --check ; echo "exit=$?"
```

Expect a refusal:

```
INVALID	1 problem(s) across 2 entr(ies)
  graphify: stored status 'clear' disagrees with the gates, which compute 'known_risks'
exit=1
```

Restore:

```bash
cp /tmp/graphify.backup.json install/open-source-kit/registry/graphify.json
python3 scripts/validate_registry.py --check
```

Other tampering worth trying, each producing its own distinct error rather than
a silent pass: delete a `resolves_by` from a failing gate, empty an
`intersection_map`, set a score to `9`, or change a `relation` to something
outside `duplicates`/`replaces`/`conflicts`/`complements`.

### 5. Selection is a dry run first

```bash
mkdir -p /tmp/kit3-demo

python3 scripts/install_open_source_kit.py \
  --select obra-superpowers --target /tmp/kit3-demo --dry-run
```

Expect:

```
WOULD_SELECT	obra-superpowers	obra/superpowers (+ superpowers-skills)
  url:  https://github.com/obra/superpowers
  how:  Inside a Claude Code session: `/plugin install superpowers@claude-plugins-official` ...
  risk: Passed Anthropic's own marketplace listing bar - the strongest trust signal in this catalog.
  !!    NOT REVIEWED - no registry entry; nothing checked this against the twelve gates
  !     not pinned yet - once installed, record the exact ref with `--pin obra-superpowers=<ref>`

WOULD_WRITE	/tmp/kit3-demo/.kit3-manifest.json
```

The `!!` line names what review found. A pick with a registry entry shows its
status instead — try `--select graphify --dry-run` for `KNOWN RISKS recorded`.
It informs; it never refuses. The same fact is written into the manifest under
`review`, so it survives the terminal.

Confirm nothing was written:

```bash
ls -la /tmp/kit3-demo        # no .kit3-manifest.json
```

### 6. Interactive selection (the checkbox path)

```bash
python3 scripts/install_open_source_kit.py --target /tmp/kit3-demo --dry-run
```

Prints a numbered list and waits at `>`. Enter comma-separated numbers
(`2,5,7`), `all`, or empty to cancel. Non-interactively:

```bash
echo "2,5,7" | python3 scripts/install_open_source_kit.py \
  --target /tmp/kit3-demo --dry-run
```

An LLM driving the flow uses `--select id1,id2` instead — same mechanism, no
terminal prompt.

### 7. Recording a selection writes a manifest

```bash
python3 scripts/install_open_source_kit.py \
  --select obra-superpowers --target /tmp/kit3-demo \
  --pin obra-superpowers=abc1234

cat /tmp/kit3-demo/.kit3-manifest.json
```

Expect:

```json
{
  "entries": {
    "obra-superpowers": {
      "category": "skills",
      "install_guidance": "Inside a Claude Code session: `/plugin install superpowers@claude-plugins-official` - already listed in the official marketplace, no separate marketplace add needed.",
      "install_method": "claude-marketplace",
      "license": null,
      "name": "obra/superpowers (+ superpowers-skills)",
      "pinned_ref": "abc1234",
      "review": {
        "reviewed": false,
        "status": null
      },
      "reviewed_date": "2026-08-26",
      "risk_notes": "Passed Anthropic's own marketplace listing bar - the strongest trust signal in this catalog.",
      "selected_date": "2026-08-31",
      "url": "https://github.com/obra/superpowers"
    }
  },
  "kit": "open-source-kit",
  "schema_version": 1
}
```

Selecting again merges rather than overwrites — run it with a second id and
confirm both entries survive.

This file is committed with the client project and is the whole audit trail, so
it answers *how* as well as *what*:

| Field | Answers |
| --- | --- |
| `install_method` / `install_guidance` | How it gets installed. `curl \| bash` is not `git clone`, and the method is part of the risk. Named `guidance`, not `command`: this is what the tool proposed, not proof of what a human ran. |
| `pinned_ref` | Which exact ref is in the project. `null` means a floating version that can change under the client without review. |
| `review.reviewed` | Whether anything checked it against the twelve gates at all. `false` is not "safe" — it is "unexamined". |
| `review.status` | What that check found, when it happened. |
| `risk_notes` | The catalog's standing warning, carried along so it is not left behind in a terminal. |
| `license` | `null` means confirm at install time; do not assume permissive. |
| `selected_date` vs `reviewed_date` | When it was picked, versus how current the review was at that moment. |

`review` is a snapshot taken at selection time and is not refreshed — if a
registry entry is written later, an older manifest keeps saying `reviewed:
false`. Re-running the selector for that id updates it.

### 8. Failure modes behave

```bash
python3 scripts/install_open_source_kit.py --select does-not-exist --target /tmp/kit3-demo
# -> exit 1, "unknown resource id", no manifest written

python3 scripts/install_open_source_kit.py --select obra-superpowers --target /tmp/kit3-demo --pin not-a-pair
# -> exit 1, "--pin must be ID=REF"

python3 scripts/install_open_source_kit.py --select obra-superpowers
# -> exit 2, --target is required

python3 scripts/install_open_source_kit.py --select obra-superpowers --target /nope-12345
# -> exit 1, "--target is not an existing directory", checked before any output

python3 scripts/install_open_source_kit.py --select obra-superpowers --target /tmp/kit3-demo --pin typo=abc123
# -> exit 1, "--pin names id(s) not in this selection" - a dropped pin would
#    otherwise leave you believing a ref was recorded
```

A pin already in the manifest survives a later selection that carries none —
`--pin X=ref`, then `--select all`, and `X` keeps `ref`.

### 9. The test suites

```bash
python3 -m unittest tests.test_registry tests.test_open_source_kit
```

Expect `Ran 52 tests ... OK` — 38 registry, 14 catalog/selector.

### 10. Clean up

```bash
rm -rf /tmp/kit3-demo /tmp/graphify.backup.json
```

---

## The gates

Twelve per candidate. **Eight binary** — one failure rejects the tool however
good it is:

`license` · `data_egress` · `pinning` · `uninstall` · `collisions` ·
`auto_update` · `maintenance_ownership` · `measurability`

**Four scored** 0–5, informing without blocking:

`automation_depth` · `token_efficiency` · `integration_coverage` · `trust_signals`

| Verdict | Meaning |
| --- | --- |
| `clear` | Every binary gate passes. Nothing outstanding was found. |
| `open_questions` | Nothing failed, but something is `unknown` — absence of evidence is not evidence of safety. |
| `known_risks` | At least one binary gate failed. The risk is named, with what would close it. |

None of the three is a permission or a prohibition. `known_risks` means we
looked and wrote down what we saw; installing anyway is a decision the team may
take with its eyes open, and the manifest will record that it was taken.

The status is stored in the file so it is greppable, but never trusted:
`validate_registry.py` recomputes it from the gates and fails when the two
disagree. A stored `clear` cannot outrank a failing gate — that is step 4
above.

### What each binary gate means

| # | Gate | Fails when |
| --- | --- | --- |
| 1 | `license` | Incompatible with client work-for-hire terms, non-commercial, or unknown-and-unconfirmable. Mixed licensing fails until each component in use is confirmed separately. |
| 2 | `data_egress` | Sends client code, prompts, or telemetry outbound without disclosure and consent. An NDA question before a technical one. |
| 3 | `pinning` | Cannot be installed at an exact immutable ref. A floating `main`, a `latest` tag, or `curl \| bash` of an unpinned URL all fail. |
| 4 | `uninstall` | No complete removal path, or removal leaves residue. We will leave; the tool must be removable by whoever stays. |
| 5 | `collisions` | Collides with a Kit 1/Kit 2 name or architecture with no recorded resolution. An architectural collision — a design `subagent-gate.sh` refuses by construction — fails here just as a name collision does. |
| 6 | `auto_update` | Updates itself inside the client repository. Third-party code changing under a client without review is unacceptable at any quality level. |
| 7 | `maintenance_ownership` | Nobody owns the ticket when it breaks in the client's production. If the answer is "nobody", it is not installed. Forking makes the answer "us" — a deliberate, priced choice. |
| 8 | `measurability` | Its cost cannot be measured. What we cannot measure, we cannot put in an offer. |

### What each scored gate measures

| # | Gate | Measures |
| --- | --- | --- |
| 9 | `automation_depth` | Does it orchestrate itself, or wait to be invoked? Deeper automation is more useful *and* more consequential — it earns more scrutiny, not less. |
| 10 | `token_efficiency` | Startup and per-invocation cost via `scripts/context_budget.py`, not the vendor's claim. Unmeasured scores 0 and fails gate 8. |
| 11 | `integration_coverage` | How many of Claude Code / Cursor / Codex it actually targets. A single-tool resource leaves a gap the moment a team mixes tools. |
| 12 | `trust_signals` | Commit freshness, bus factor, issue response, release discipline, tests, network calls in the install script, presence of auto-update. |

### Required blocks in an entry

Beyond the gates:

- `install` — method, `pinned_ref`, `namespace_prefix`, and the exact command a
  human will run. The registry never executes it.
- `intersection_map` — **mandatory, may not be empty.** One row per overlap with
  something we already ship, typed `duplicates`/`replaces`/`conflicts`/
  `complements`. The hidden cost of Kit 3 is not context; it is two systems
  doing one job. An entry with no intersection analysis is unreviewed, whatever
  its gates say.
- `lifecycle` — uninstall command, residue, whether removal is manifest-tracked.
- `default_state` — `enabled` or `disabled`. Disabled is the default default.
- `tier` — 1 (considered for every engagement), 2 (situational), 3 (specialist,
  needs a specific reason).

A gate that is `fail` or `unknown` must carry `resolves_by`, so a rejection is a
work item rather than a dead end.

### Star count is not a trust signal

Excluded from scoring by construction. Several catalog entries carry counts
that are statistically implausible for their age — verified via the GitHub API
(not the profile page) on 2026-08-31:

| Repo | Stars | Forks | Age |
| --- | --- | --- | --- |
| `mattpocock/skills` (hosts `grillme`) | 242,415 | 20,609 | ~7 months |
| `Graphify-Labs/graphify` | 112,842 | 10,980 | ~5 months |
| `JuliusBrussee/caveman` | 101,968 | 5,933 | ~5 months |
| `Yeachan-Heo/oh-my-claudecode` | 38,913 | 3,491 | ~7.5 months |

For scale, the first has more stars in seven months than `facebook/react`
gathered in twelve years. Independent research
([arXiv 2412.13459](https://arxiv.org/html/2412.13459v2), "Six Million
Suspected Fake Stars on GitHub") found AI/LLM tooling is now the largest
category of fake-star recipients.

An inflated count is evidence the *signal* is fake, not that the *code* is —
so it is recorded under `trust_signals.disqualified_signals` as a finding and
never scored. These four are catalogued as `HIGH RISK` by explicit decision;
`multica-ai/andrej-karpathy-skills` and `affaan-m/ecc` remain excluded
entirely, `ecc` additionally for the same architectural conflict that rejects
`ohmyclaude`.

Replaceable signals, used instead: commit freshness, bus factor, issue
response time, release/semver discipline, presence of tests, network calls in
the install script, and whether the thing auto-updates.

### What to check before anything reaches a client repo

- License compatible with client work-for-hire terms.
- Every hook and install script read in full — grep `curl`/`fetch`,
  `exec`/`eval`, credential env vars.
- Installed at a pinned ref, never a floating `main`; recorded with `--pin`.
- No duplicate agent/skill/hook names against the installed edition's roster.
- Every subagent declares a minimal `tools:` allowlist; reviewers read-only.
- Hooks from different sources don't double-block or contradict each other.
- Startup context cost measured, not taken from the vendor's claim.
- Tested on a throwaway branch before rollout.
- Re-run on every update, not just first install — a clean dependency can turn
  in a later release.
- `.kit3-manifest.json` committed with the client project.

---

## Installing for real

Testing is above; this is the actual engagement flow.

1. **Read what review found.** `validate_registry.py --id <tool>`. `clear` means
   nothing outstanding. `known_risks` and `open_questions` name what is
   unresolved and what would close it — decide whether to close it first or to
   proceed knowingly. Nothing stops you; the choice is recorded either way.
2. **Dry run** against the client project, read every risk note.
3. **Record** the selection — writes `.kit3-manifest.json`.
4. **Install by hand.** Run the printed command yourself, after reading the
   tool's install script and hooks in full: grep for `curl`/`fetch`, `exec`/
   `eval`, and credential env vars. The selector never does this for you, and
   that is deliberate — it is the step that forces someone to read third-party
   code before it enters a client repository.
5. **Pin** the exact ref actually installed with `--pin ID=REF`.
6. **Verify isolation** — no name collisions against the installed edition's
   roster; hook interaction tested on a throwaway branch, including the case
   where two enforcement systems both block.
7. **Commit `.kit3-manifest.json`** with the client project. It is the audit
   trail of what we added and why.

### Not built yet

Three gaps, stated plainly:

- **Most of the catalog is unreviewed.** 13 of 15 entries have no registry file,
  so they report `NOT REVIEWED` — nothing has checked them against the twelve
  gates. This is the largest gap: an unreviewed tool is not a safe one, it is an
  unexamined one. Writing those entries is the highest-value next work.
- **No cost preview.** Step 2 should print the startup-cost delta before you
  commit. Needs an external-tree mode in `scripts/context_budget.py`, which is
  also what closes the `measurability` gate on both current entries.
- **No uninstall.** Pillar 4 — removing exactly what the manifest lists,
  leaving no residue — has no implementation.
