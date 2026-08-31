# Open-Source Kit (Kit 3)

Third bootstrap path, alongside Infrastructure-Creator (Kit 1, bespoke
generated) and the ready-made editions (Kit 2, Laravel/Symfony/PHP Core/
WordPress). Kit 3 admits **other people's** tools into a client project under
gates we own.

What Kit 3 actually is — four things, in order:

1. **Admission** — what passed the gates, at which pinned ref, on whose decision.
2. **Isolation** — namespace, roster, hook precedence.
3. **Measurement** — cost on the same ruler as our own editions.
4. **Removal** — uninstall with no residue. We leave; the client keeps what they chose.

The tool list is a consequence of the gates, not a decision taken ahead of them.

Two files matter:

| Path | Role |
| --- | --- |
| `resources.json` | The browsable catalog. What exists. |
| `registry/*.json` | The admission decision. What review concluded. |

Being in the catalog is not permission to install — the registry carries that
judgement. **Enforcement is not wired yet:** the selector prints the verdict as
a warning but does not refuse, and 13 of 15 catalog entries have no registry
file at all. Admission is therefore a step you perform (§"Installing for real"
step 1), not something the tool guarantees. See "Not built yet" at the end.

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

### 2. The registry reports a verdict per candidate

```bash
python3 scripts/validate_registry.py
```

Expect, per entry:

```
REJECTED 	graphify      	Graphify-Labs/graphify
  tier 3, default disabled, pinned_ref NONE
  failing gates:  collisions, maintenance_ownership, measurability
  unknown gates:  data_egress, pinning, uninstall, auto_update
  scores:         automation_depth=5, token_efficiency=0, integration_coverage=5, trust_signals=2
  intersections:  3 recorded

VALID	2 registry entr(ies)
```

`VALID` means the files are well-formed and every stored verdict matches what
its gates compute. It does **not** mean the tools are approved — read the
verdict on each line.

Both committed entries are `REJECTED` today. That is the expected state, not a
failure: each failing gate carries a `resolves_by` naming exactly what would
change it.

### 3. The CI gate is silent on success

```bash
python3 scripts/validate_registry.py --check ; echo "exit=$?"
```

Expect `VALID	2 registry entr(ies)` and `exit=0`.

### 4. Prove the verdict cannot be faked

This is the property the whole registry rests on. Flip a verdict without
touching the gates that produced it:

```bash
cp install/open-source-kit/registry/graphify.json /tmp/graphify.backup.json

python3 - <<'PY'
import json
p = "install/open-source-kit/registry/graphify.json"
d = json.load(open(p))
d["verdict"] = "approved"          # gates left untouched
json.dump(d, open(p, "w"), indent=2)
PY

python3 scripts/validate_registry.py --check ; echo "exit=$?"
```

Expect a refusal:

```
INVALID	1 problem(s) across 2 entr(ies)
  graphify: stored verdict 'approved' disagrees with the gates, which compute 'rejected'
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
  !!    NOT REVIEWED - no registry entry; nothing has judged this against the twelve gates
  !     not pinned yet - once installed, record the exact ref with `--pin obra-superpowers=<ref>`

WOULD_WRITE	/tmp/kit3-demo/.kit3-manifest.json
```

The `!!` line is the admission warning. A pick with a registry entry shows its
verdict instead — try `--select graphify --dry-run` for
`admission verdict is REJECTED`. It warns; it does not refuse.

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
      "license": null,
      "name": "obra/superpowers (+ superpowers-skills)",
      "pinned_ref": "abc1234",
      "reviewed_date": "2026-08-26",
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
| `approved` | Every binary gate passes. Installable, subject to its conditions. |
| `blocked` | Nothing fails, but something is `unknown`. Absence of evidence is not a pass. |
| `rejected` | At least one binary gate fails. |

The verdict is stored in the file so it is greppable, but never trusted:
`validate_registry.py` recomputes it from the gates and fails when the two
disagree. A stored `approved` cannot outrank a failing gate — that is step 4
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

1. **Check admission.** `validate_registry.py --id <tool>`. Only `approved`
   proceeds. `blocked` means someone does the work in that gate's `resolves_by`
   first.
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

- **The selector does not consult the registry.** `install_open_source_kit.py`
  reads `resources.json` only; a `rejected` tool can still be selected today.
  Admission is therefore enforced by convention (step 1 above), not by the
  tool. Closing this is the highest-value next change.
- **No cost preview.** Step 2 should print the startup-cost delta before you
  commit. Needs an external-tree mode in `scripts/context_budget.py`.
- **No uninstall.** Pillar 4 — removing exactly what the manifest lists,
  leaving no residue — has no implementation.
