# Phase 1 Scan Output Contract

One contract for all seven discovery scanners - `stack-scanner`,
`architecture-scanner`, `conventions-scanner`, `domain-behavior-scanner`,
`integration-scanner`, `security-compliance-scanner`, `infra-ops-scanner`.
Read it before writing anything. Every rule here is mandatory; a scanner's own
`SKILL.md` adds detection duties on top, never a different output shape.
Sections 1-8 apply to every scanner; appendix A holds the seven report
templates - read only the one for the scanner you are running.

## 1. The run's task directory

All scan output lives in `tasks/TASK-{NNN}/` inside Infrastructure-Creator's own
folder, never in the target.

- `{NNN}` is the `tasks/.task-counter` value **zero-padded to three digits**:
  `TASK-001`, `TASK-012`, `TASK-137`. `TASK-1` is malformed.
- `infra-scan` allocates the directory and hands it to each scanner. A scanner
  run standalone allocates it the same way (read the counter, create the padded
  directory, increment).
- One directory per run, shared by all seven scanners. Never create a second
  directory, and never re-pad an existing one. If both `TASK-1` and `TASK-001`
  exist for the same run, stop and report the collision instead of guessing.

## 2. Three artifacts per scanner, all required

A human report, a machine ledger, and a coverage record are different artifacts
with different readers. "Exactly one" applies to each kind separately:

| Artifact | Path | Reader |
| --- | --- | --- |
| Report | `tasks/TASK-{NNN}/<scanner-name>-findings.md` | a human reviewing the profile |
| Ledger | `tasks/TASK-{NNN}/<scanner-name>-evidence.json` | `profile-synthesizer` and the plan validators |
| Coverage | `tasks/TASK-{NNN}/<scanner-name>-coverage.json` | `validate_scan_coverage.py` |

Write exactly one of each, per scanner, per run. A report without its ledger is
an incomplete scan: nothing in it can pass the evidence gate. A ledger without
its report hides the scan from human review. A scan without its coverage record
cannot be told apart from a scan that missed a subsystem - both produce a list
of what was found, and only the coverage record says what was *not*. None may be
written into the target project.

The report is the human's way into the ledger, not a second copy of it: a
summary a reviewer reads in a minute, with the detail left in the JSON beside
it. Past **80 lines** it is restating what the ledger already carries, and is
reported as `SCAN_REPORT_OVERLONG`; measured on a real run, the honest range is
12 to 23 lines each.

The report follows this scanner's template in appendix A. Every factual line in
it carries a confidence tag and cites its evidence id, so the report and the
ledger join without re-derivation - for example:

```markdown
- Resolved PHP: 8.2.0 pinned by `composer.lock` `platform-overrides` (confirmed - EV-STK-0003; composer.lock:L11-L13)
```

## 2a. Coverage record shape

```json
{
  "scanner": "stack-scanner",
  "target_root": "/absolute/path/to/target",
  "surfaces": [
    {
      "surface": "composer.json",
      "kind": "file",
      "disposition": "covered",
      "reason": "Declares the runtime and the dependency graph",
      "evidence_ids": ["EV-STK-0001"]
    },
    {
      "surface": "src/**",
      "kind": "tree",
      "disposition": "covered",
      "reason": "The target's own application modules",
      "evidence_ids": ["EV-STK-0004"]
    },
    {
      "surface": "vendor/**",
      "kind": "tree",
      "disposition": "excluded",
      "reason": "Third-party code, not the target's own",
      "evidence_ids": []
    },
    {
      "surface": ".env",
      "kind": "file",
      "disposition": "not-permitted",
      "reason": "Holds secret values; existence recorded, contents never read",
      "evidence_ids": []
    }
  ]
}
```

Members are exact: `surface`, `kind` (`file` or `tree`), `disposition`,
`reason`, `evidence_ids`. The four dispositions and what each costs to claim:

- **`covered`** - fully read. At least one evidence id in this scanner's own
  ledger must cite a path inside the surface, or the claim is
  `SCAN_COVERED_WITHOUT_EVIDENCE`. Naming a secret-bearing surface here is
  `SCAN_SECRET_COVERED` - a claim to have done what section 6 forbids.
- **`excluded`** - deliberately left out. Citing evidence from it contradicts
  the exclusion.
- **`truncated`** - the scan stopped partway, at a budget or a limit. Evidence
  from the part that was read is expected. Always reported as a visible warning:
  a cap nobody sees reads as complete coverage.
- **`not-permitted`** - the contract forbids reading it.

A surface may be a path, a `tree`, or a `dir/**` glob, so a large target is
described in a handful of lines rather than a thousand. **One tree disposition
accounts for everything under it** - `src/**` covered, with one evidence entry
inside `src`, answers for every module in it, and nothing has to be enumerated
directory by directory. Three entries can describe a whole small target. A
coverage record past **30 surfaces** is enumerating a tree it could have named,
and is reported as `SCAN_COVERAGE_VERBOSE`; measured on a real seven-scanner
run, the honest range is 5 to 17 each.

**Read to a budget and say where it ran out.** A scanner is not required to open
every file: it is required to say what it did with every surface. Reading the
first files of a large tree and marking the rest `truncated`, with the reason,
is a complete scan. Reading all of it slowly is not more complete - it is the
same coverage record, later. Every top-level entry of
the target, and every immediate child of `src`, `app`, `tests`, `config` and
their siblings, must be reached by some entry from some scanner, or it is
`SCAN_SURFACE_UNACCOUNTED`. Evidence cited from a surface no scanner says it
read is `SCAN_EVIDENCE_OUTSIDE_COVERAGE`; a surface one scanner covers and
another calls forbidden is `SCAN_DISPOSITION_CONFLICT`.

Run the gate yourself before returning:

```bash
python3 bootstrap-verifier/scripts/validate_scan_coverage.py --target <target> --task-dir tasks/TASK-001
```

## 2b. The reconciled claim set

Seven scanners run in parallel and each states its findings in prose inside its
own ledger. Nothing merges them, so nothing notices that two contradict each
other, and nothing notices when an invariant one of them confirmed never reaches
the plan. Reconciliation - the step in `infra-scan` that runs once all seven have
returned - promotes those prose claims into one
`tasks/TASK-{NNN}/project-claims.json`:

```json
{
  "target_root": "/absolute/path/to/target",
  "claims": [
    {
      "id": "CLM-0007",
      "statement": "A sealed document bundle rejects further pages",
      "claim_class": "invariant",
      "priority": "high",
      "evidence_ids": ["EV-DOM-0012"],
      "scanners": ["domain-behavior-scanner"],
      "status": "confirmed"
    }
  ],
  "contradictions": [
    {
      "id": "CTR-0001",
      "claim_ids": ["CLM-0007", "CLM-0019"],
      "statement": "The importer appends to bundles the domain calls sealed",
      "resolution": "unresolved"
    }
  ]
}
```

Claim members are exact: `id`, `statement`, `claim_class`, `priority`,
`evidence_ids`, `scanners`, `status`. A contradiction carries exactly `id`,
`claim_ids`, `statement`, `resolution`.

`claim_class` is one of `invariant`, `capability`, `convention`, `risk`,
`integration`, `command`; `priority` is `high`, `medium`, or `low`; `status`
reuses the ledger's `confirmed` / `inferred` / `unknown`. Nothing here is a new
finding - every claim names the evidence ids it was promoted from, and a claim
resting on evidence no ledger carries is `CLAIM_EVIDENCE_UNKNOWN`.

`priority: high` is not a fresh judgement: it is the `priority: high` the
domain-behavior-scanner already assigned to a business invariant in its own
report. Promoting a capability or a convention to `high` inflates the population
this gate blocks on, which is how a rule meant to catch one dropped invariant
turns into a rule that stops generation. Measured on four real runs, the honest
population is seven to twelve high-priority invariants per target, and all of
them were already carried into the plan.

An unresolved contradiction is always reported. It blocks when it touches a
high-priority invariant: a skill cannot be told to honour something discovery is
still arguing about. Otherwise it is a warning that belongs in the confidence
summary.

## 2c. What the selection leaves unowned

Discovery says what was read; the plan says what is owned. Until the two are
joined, a subsystem every scanner covered can reach publication with no skill
responsible for it and no diagnostic anywhere - measured on a real run, the CI
pipeline the scan had just found belonged to nobody, and so did the stored
GraphQL documents. Both were invisible, because the only question ever asked was
whether a *selected* skill was justified, never whether the selection left a
hole.

After synthesis, every surface a scanner reports `covered` or `truncated` must be
reachable from some selected skill's `ownership`, `writes`, or `path_contracts`,
or be recorded in `tasks/TASK-{NNN}/plan-ownership.json`:

```json
{
  "target_root": "/absolute/path/to/target",
  "unowned": [
    {
      "surface": "graphql/**",
      "reason": "Stored query documents run by hand against the schema; no generated skill executes or maintains them"
    }
  ]
}
```

Members are exact: `surface`, `reason`. A covered surface that is neither owned
nor declared is `OWNERSHIP_SURFACE_UNOWNED`; a declaration naming a surface no
scanner read, or one a skill does declare, is `OWNERSHIP_DECLARATION_UNKNOWN`.
`excluded` and `not-permitted` surfaces owe no owner - they are outside the
project's own material by construction. A surface may honestly have no owner;
what it may not do is have none silently.

**A high-priority invariant must survive into the plan.** Dropping one may well
be right - it may intersect no selected skill - but that is a decision, and an
undocumented decision cannot be told apart from an oversight. After synthesis,
run the gate with the plan:

```bash
python3 bootstrap-verifier/scripts/validate_scan_coverage.py --target <target> --task-dir tasks/TASK-001 --plan tasks/TASK-001/skill-generation-plan.json
```

An invariant the plan states against at least one of the same evidence ids, in
words that share the statement, is carried; anything else is
`CLAIM_INVARIANT_LOST`.

## 3. Ledger shape

```json
{
  "scanner": "stack-scanner",
  "target_root": "/absolute/path/to/target",
  "evidence": [
    {
      "id": "EV-STK-0003",
      "path": "composer.lock",
      "source_type": "configuration",
      "authority": "Lock file; records the pinned platform PHP the dependency graph was resolved against",
      "confidence": "confirmed",
      "line_range": {"start": 11, "end": 13},
      "fingerprint": "sha256:0e5f...64 lowercase hex chars...",
      "supported_claims": [
        "composer.lock platform-overrides pins PHP 8.2.0, while platform only mirrors the >=8.2.0 constraint"
      ]
    }
  ]
}
```

Each `evidence[]` entry uses **exactly** the members the machine plan accepts -
`id`, exactly one of `path` or `url`, `source_type`, `authority`, `confidence`,
optional `line_range`, `fingerprint`, `supported_claims`. Any other member is
rejected downstream (`EVIDENCE_FIELD_UNKNOWN`), so do not add scanner-private
fields; put scanner commentary in the report.

- **`id`** - `EV-<TAG>-<NNNN>`, where `<TAG>` identifies the scanner:
  `STK` stack, `ARC` architecture, `CNV` conventions, `DOM` domain-behavior,
  `INT` integration, `SEC` security-compliance, `OPS` infra-ops. The tag keeps
  ids unique when the seven run in parallel. `profile-synthesizer` may renumber
  them when merging - never assume your id survives into the plan, and always
  refer to evidence by id inside your own two artifacts.
- **`path`** - canonical and **relative to the target root**
  (`config/packages/messenger.yaml`). Never absolute, never containing `..`,
  never a path under `tasks/TASK-*`, never a path inside Infrastructure-Creator.
- **`url`** - only for external authority (vendor docs). No fingerprint; record
  the document version or retrieval authority in `authority`.
- **`source_type`** - exactly one of `spec/ADR`, `test`,
  `database constraint`, `workflow configuration`, `authorization rule`,
  `domain code`, `application code`, `configuration`, `interview answer`.
- **`confidence`** - `confirmed` (the cited file states it), `inferred`
  (indirect signal), or `unknown`.
- **`line_range`** - `{"start": <int >= 1>, "end": <int >= start>}`, and `end`
  must not exceed the file's real line count. Omit it rather than guess; a
  whole-file citation legitimately has no range.
- **`fingerprint`** - required for every `path` entry (see section 4).
- **`supported_claims`** - non-empty. Each claim is one specific sentence that
  the cited lines actually prove, naming the concrete thing found ("`.ddev`
  post-start hooks run `doctrine:schema:update --force`"). Restating the file's
  purpose ("this file configures the application") is not a claim and is
  rejected as generic support. Word every claim out of the cited range itself:
  the plan validator (`EVIDENCE_CLAIM_UNSUPPORTED`) requires words that
  literally appear in those lines and that are not PHP boilerplate. Citing
  `config/packages/flysystem.yaml` - good: "the `default` storage uses the
  `aws` adapter"; bad: "remote file uploads are supported", which reuses
  nothing the lines say.

### Recording what the target does *not* do

Some of the most useful findings are negative: an analyser that is configured
and invoked from nowhere, a queue with no consumer, a migration path with no
rollback. These could not be recorded at all, because every entry needed a path
and a fingerprint, and an absence has neither. Use an `absence` entry - it
replaces `path`/`url`/`fingerprint`/`line_range`, and the remaining members are
unchanged:

```json
{
  "id": "EV-STK-0031",
  "absence": {
    "subject": "No CI workflow or composer script invokes PHPStan",
    "search": "grep -rn phpstan .github composer.json",
    "accounted_matches": ["composer.json"]
  },
  "source_type": "configuration",
  "authority": "The only match declares the dependency; nothing runs it",
  "confidence": "confirmed",
  "supported_claims": ["PHPStan is installed but never invoked"]
}
```

The `search` must be a literal search the plan validator can resolve by itself -
no pipes, substitutions, or regular expressions - and its resolution must equal
`accounted_matches` exactly. What the accounted matches *mean* is your
judgement; which files exist is not. When the target later grows a CI step, the
entry fails loudly as `EVIDENCE_ABSENCE_CONTRADICTED` instead of quietly
staying wrong. Word the claim out of the search, not out of the subject line.

## 4. Fingerprints

The fingerprint is `sha256:` + the lowercase hex digest of the **whole file's
bytes** - not of the cited line range, and not of a normalized text form:

```bash
python3 -c "import hashlib,sys;print('sha256:'+hashlib.sha256(open(sys.argv[1],'rb').read()).hexdigest())" "$TARGET/composer.json"
```

Fingerprint the file at the moment you read it. If a file changes during the
run, re-read it and re-fingerprint; a stale digest blocks publication later
(`EVIDENCE_FINGERPRINT`). Never fingerprint a file you were not allowed to open
(section 6), and never fingerprint a generated or task-directory file as target
evidence.

## 5. Anchors and sibling inputs

A bounded anchor in either artifact is `path:L<start>-L<end>` or
`path:symbol:<Name>`, where the symbol literally occurs in the cited file
(`config/packages/messenger.yaml:symbol:framework.messenger.transports`,
`src/Command/PublishContentCommand.php:symbol:PublishContentCommand`).

Sibling scanners publish to the same task directory, so a step that needs
another scanner's output has a fixed address: `tasks/TASK-{NNN}/<sibling>-findings.md`
and `tasks/TASK-{NNN}/<sibling>-evidence.json`. When those files are absent -
a standalone run, or a sibling that failed - **degrade explicitly**:

1. Derive first-hand only the minimum your own step needs, and mark it
   `inferred`, not `confirmed`.
2. Do not re-run the sibling's scan in depth, and do not invent its verdict.
3. Record a gap line in your report:
   `Missing sibling input: <sibling> - degraded: <what was skipped or downgraded>`,
   and carry the same limitation into the affected findings.
4. Never block waiting on a sibling, and never write into a sibling's artifact.

## 6. Secrets rule (identical for every scanner)

Never open, read, print, summarize, or fingerprint `.env`, `.env.*`, private
keys, certificates, credential stores, or any file whose purpose is to hold
secret values. Existence may be recorded; contents may not.

Environment-variable **names** may be cited when they come from a non-secret
committed source: `config/**`, container and orchestration config, CI workflow
env blocks, deploy scripts, committed `.env.example` / `*.template` files, and
`env()` / `getenv()` / `$_ENV` / `%env()%` call sites in code. Values are never
recorded, quoted, hashed, or paraphrased - not even when they look harmless. A
key name knowable only from a real `.env` stays `unknown`. If a committed
example file contains what looks like a real value, cite the key name only and
flag the file in the report.

## 7. Signal classes, not filenames

Every detection list in a scanner's Process is a list of **examples of a signal
class**, not the definition of the class. A capability exists if any member of
its class is present, and the report names which member proved it. Before
reporting a capability absent, search the whole class and state in the report
what was searched - "no containerization" after checking only `Dockerfile` is a
wrong answer, not a cautious one.

## 8. Breadth checklists

The classes the scanners are contractually required to search in full. Each
list is a floor, not a ceiling: an unlisted member of the same class counts.

**Command declaration sites** (`stack-scanner` step 7, `infra-ops-scanner`
steps 3 and 7) - a project's dangerous commands usually live outside its
Composer scripts:

- `composer.json` `scripts` (including `auto-scripts`) and `package.json` `scripts`;
- deploy/release scripts: `deploy.sh`, `deploy/**`, `bin/*.sh`, `Makefile`, `Taskfile*`;
- environment and container hooks: `.ddev/config.yaml` `hooks:` and `.ddev/commands/**`, `.lando.yml` events, compose `command`/`entrypoint`, `Dockerfile` `RUN`/`CMD`, tracked git-hook scripts and hook managers;
- CI/CD job steps;
- project-declared console commands: `#[AsCommand]` attributes, `Command` subclasses, `console.command` service tags - plus their cron/scheduler/queue registration;
- provider/PaaS lifecycle hooks (`.platform.app.yaml` hooks, `Procfile`, release commands).

**Containerization and local-environment signals** (`infra-ops-scanner` step 1):
`Dockerfile`/`Containerfile` and `.dockerignore`; `docker-compose.yml`/`compose.yaml`;
container-owning environment managers (`.ddev/config.yaml`, `.lando.yml`,
`.devcontainer/`, `Vagrantfile` with a container provider, Tilt/Skaffold);
Nix/`Procfile`-style runtime declarations; hosting manifests that pin an image.
Whichever member is present also declares the runtime version, the service
topology (web server, database, cache/queue), and enabled extensions - read them
from that file (`FROM php:8.2-fpm` + `docker-php-ext-install`, or
`php_version:`/`database:` keys), not from a framework default.

**Integration candidate sources** (`integration-scanner` step 1): `composer.json`
`require`; runtime registration and config (`config/bundles.php`,
`config/packages/**`, service/provider definitions, DI wiring); `composer.lock`
for the transitive package that actually implements a configured subsystem;
environment/container/CI config and deploy scripts for service DSNs and base
URLs (names and non-secret config only, per section 6); `package.json` when a
build or runtime service calls an external provider.

**PHP runtime sources** (`stack-scanner` step 3): the constraint is
`composer.json` `require.php`, mirrored by `composer.lock` `platform`; the pin is
`composer.lock` `platform-overrides`, `.php-version`, a container image tag or
`php_version:` key, or a CI matrix entry.

## Appendix A - report templates

One template per scanner; the scanner's own Process defines what fills them. Keep the headings verbatim so `profile-synthesizer` can merge the seven reports mechanically, and drop no heading - an inapplicable section is reported as `none` or `N/A - not configured`, never omitted.

### stack-scanner

```markdown
# Stack Scanner Findings: [target_name]

**Target:** [path]  **Scanned:** [date]

## PHP Runtime
- Version constraint: [e.g. ^8.2] (confirmed - composer.json:L# / composer.lock platform)
- Pinned runtime: [platform-overrides / .php-version / image tag] (confirmed/inferred - path:L#) | unknown - constraint only

## Framework
- [Name + version] (confirmed - evidence path:L#)

## Package Manager & Autoload
- Composer; PSR-4: [Namespace => path, ...] (confirmed - composer.json)

## Entry Points
- [public/index.php, artisan, bin/console, ...] (confirmed - path)

## Testing
- Suites: [name -> config/root/bootstrap -> level/scope -> focused invocation] (confirmed/inferred - bounded anchors)
- Fixtures/factories/fakes: [paths and applicable suites] | none

## Lint / Format / Static Analysis
- [tool: config path] for each detected; "N/A - not configured" where none

## Repository Commands
- [command -> declaration site -> exact body -> resolved alias chain -> mutation/network class] for scripts, deploy/release scripts, environment hooks, and declared console commands; name the sites searched when none is risky

## Confidence Summary
[X confirmed, Y inferred, Z unknown]
```

### architecture-scanner

```markdown
# Architecture Scanner Findings: [target_name]

**Target:** [path]  **Scanned:** [date]

## Architecture Style
- [monolith | modular-monolith | microservices | event-driven] (confirmed/inferred - evidence path:L#)

## Layering / DDD
- [layered | hexagonal/ports-and-adapters | DDD | none] (confirmed/inferred - path)

## Module / Service Boundaries
- [boundary name => namespace/path] for each (confirmed/inferred - composer.json:L# / dir)

## Path Authority & Material Adjacency
- [path/glob -> required-existing | generated-runtime | creatable -> authority anchor]
- [interaction/request -> primary owner -> defer/fallback owner -> contract/evidence -> routing cases]

## Monorepo Signals
- [count and paths of composer.json files, or "single root"] (confirmed - paths)

## Communication Style
- [HTTP controllers | message/event bus | RPC] (confirmed/inferred - package + wiring path:L#)

## Framework-Specialty Signals
- ORM/data-access: [...] (confidence - path) | none
- DB migration tooling: [...] (confidence - path) | none
- Async/queue mechanism: [...] (confidence - path) | none
- Event listener/subscriber/observer: [...] (confidence - path) | none
- Notification delivery: [...] (confidence - path) | none
- Caching strategy: [...] (confidence - path) | none
- File/object storage abstraction: [...] (confidence - path) | none
- Auth/authorization scaffolding: [...] (confidence - path) | none
- Form/validator design: [...] (confidence - path) | none
- Admin/back-office panel: [...] (confidence - path) | none
- Declarative API resource framework: [...] (confidence - path) | none
- Custom console commands: [...] (confidence - path) | none
- Repository/data-access layer: [...] (confidence - path) | none
- DI container configuration style: [...] (confidence - path)
- Test data factories/fixtures: [...] (confidence - path) | none
- Package vs. application: [...] (confidence - path)

## Frontend Presence
- Rendering/templating layer: [...] (confidence - path) | none
- Frontend asset build: [...] (confidence - path) | none
- Verdict: [frontend skill group applies | no UI surface - skip frontend skills]

## Confidence Summary
[X confirmed, Y inferred, Z unknown]
```

### conventions-scanner

```markdown
# Conventions Scanner Findings: [target_name]

**Target:** [path]  **Scanned:** [date]

## Code Style / Format
- [tool: config path:L# + preset] (confirmed); "N/A - not configured" where none

## Static Analysis (reference only)
- [governs conventions; see stack-scanner findings] (confirmed/inferred)

## Git Hooks
- [captainhook.json / .husky / .pre-commit-config.yaml] (confirmed - path)

## Editor Config
- [.editorconfig key settings] (confirmed - path:L#)

## Commit Conventions
- [Conventional Commits / template / documented policy] (confirmed/inferred - path)

## Docs / ADRs
- [docs/, adr/, decisions/ + count] (confirmed - path)

## Contribution Governance
- [CONTRIBUTING.md, CODEOWNERS, .github templates] (confirmed - path)

## Path Authority & Documented Commands
- [path/glob -> required-existing | generated-runtime | creatable | unknown -> bounded authority anchor]
- [documented command -> resolved definition/effect class -> conflict or none]

## Confidence Summary
[X confirmed, Y inferred, Z unknown]
```

### domain-behavior-scanner

```markdown
# Domain Behavior Scanner Findings: [target_name]

**Target:** [path]  **Scanned:** [date]

## Project Purpose & Domain Vocabulary
- Purpose: [short factual description] (confidence; source type - path:L#)
- Terms: [term = evidenced meaning] (confidence; source type - path:L#)

## Project Sources of Truth
- [area] -> [source/location] -> [authority scope] (confidence; source type - path:L#)
- Contradictions: [sources that disagree] | none

## Core Modules & Domain Model
- [module] - responsibility: [...] - paths: [...] (confidence; source type - path:L#)
- [entity] - identifiers/relationships/lifecycle fields: [...] (confidence; source type - path:L#)

## Business Invariants
- `[invariant-id]` [rule] - priority: [high|normal] - affected scope/path authority: [...] - consequence: [...] (confidence; source type - bounded path anchor)

## Lifecycles & Transitions
- Statuses discovered: [entity -> statuses] (confidence; source type - path:L#)
- Confirmed transition: [entity: from -> to] - allowed when: [...] - forbidden when: [...] - side effects: [...] (confidence; source type - path:L#)

## Roles & Permissions
- [role/principal] -> [action] on [subject/resource] - restrictions: [...] - enforcement points: [...] (confidence; source type - path:L#)
- Completeness: [complete for named surface | partial/unknown]

## Audit Obligations
- [event] - fields required: [...] - retention/immutability: [...] (confidence; source type - path:L#)

## High-Risk / Forbidden Workflows
- [workflow/action] - risk indicator: [...] - documented approval/checks: [...] (confidence; source type - path:L#)

## Critical QA / Regression Scenarios
- [scenario] - given/when/then: [...] - protected invariant IDs: [...] - suite/focused command: [...] - fixtures/fakes: [...] - expected/forbidden result: [...] (confidence; source type - bounded test/spec anchor)

## Known Risks & Incident Lessons
- [sanitized lesson] - prevention rule: [...] (confidence; source type - path:L#)

## Domain Skill Candidates
- `[skill-name]` - bounded context: [...] - invariant IDs covered: [...] - procedure/assertion ownership: [...] - material adjacencies: [...] - distinct review purpose: [...] (confidence; source paths)
- none

## Gaps & Contradictions
- [material unknown or disagreement that could change generated policy/skills/memory]

## Confidence Summary
[X confirmed, Y inferred, Z unknown]
```

### integration-scanner

```markdown
# Integration Scanner Findings: [target_name]

**Target:** [path]  **Scanned:** [date]

## Integrations by Category
### [Category, e.g. Payment]
- [package] - wiring/call sites: [bounded anchors] - supported claims: [...] - review questions/external requirements: [...] (confirmed/inferred)
- Test/safety: [fake/fixture/sandbox, focused command, network policy, authorization, rollback, sanitization]
- Adjacency/routing: [primary provider owner; material defer/fallback owners; positive/negative/ambiguous/cross-domain cases]

### [Next category ...]
- ...

## Integration Contracts (non-PHP neighbors)
- [service] - referenced at [config path:L#] (inferred/unknown)

## Uncategorized / Ambiguous
- [package or reference] (unknown - path:L#)

## Confidence Summary
[X confirmed, Y inferred, Z unknown]
```

### security-compliance-scanner

```markdown
# Security Compliance Scanner Findings: [target_name]

**Target:** [path]  **Scanned:** [date]

## Authentication Pattern
- [session/token/OAuth/SAML + package] (confirmed - evidence path:L#)

## Secrets Handling (approach only - no values read)
- [.env keys referenced / vault SDK / config/secrets] (confirmed - path:L#)

## Security Tooling
- [composer audit / roave/security-advisories / enlightn / psalm taint / phpstan rules or "N/A - not configured"] (confirmed - path:L#)

## Compliance Mentions (textual only - not an assertion of compliance)
- [GDPR/PCI/HIPAA/... : location] for each; "None detected" if absent

## Hardening Signals
- [CSRF / headers / encryption config] (confirmed/inferred - path)

## Security Test Topology & Invariant Mapping
- [suite/focused command -> fixture/fake -> allowed/denied assertion -> invariant IDs -> bounded anchor]
- Adjacency/routing: [request -> primary owner -> defer/fallback owner]

## Confidence Summary
[X confirmed, Y inferred, Z unknown]
```

### infra-ops-scanner

```markdown
# Infra Ops Scanner Findings: [target_name]

**Target:** [path]  **Scanned:** [date]

## Containers
- Mechanism: [Dockerfile | compose | DDEV/Lando/devcontainer | image-pinning manifest | none - class searched] (confirmed - path:L#)
- Runtime/extensions/services: [PHP version, extensions, web/db/cache topology] (confirmed - path:L#)

## Orchestration
- Kubernetes / Helm: [manifests/charts or "N/A - not configured"] (confirmed - path)

## CI/CD
- [provider]: [stage -> exact/resolved command -> working directory/prerequisites -> mutation/network class] (confirmed - bounded anchor)

## Infrastructure as Code
- Terraform / Ansible / Pulumi: [resources or "N/A - not configured"] (confirmed - path)

## Deployment Tooling
- [Deployer/Envoy/Capistrano/Forge/Ploi or "N/A"] (confirmed/inferred - path)

## Deployment Target Hints
- [Bref/Platform.sh/Heroku/PaaS or "N/A"] (confirmed/inferred - path)

## Destructive-Command Risks (for hook-forge)
- [command: file:L# + risk] for each; "None detected" if absent

## Operational Path Authority & Adjacency
- [path -> required-existing | generated-runtime | creatable -> authority anchor]
- [operation/request -> primary owner -> material defer/fallback owners -> routing cases]

## Confidence Summary
[X confirmed, Y inferred, Z unknown]
```
