# Reconciliation — where the four Phase-2 designs disagreed

The architecture, schema, API and frontend specs were produced concurrently and
disagree at the seams. That is the point of running them together. This file is
the authoritative resolution: **where a design document contradicts a decision
recorded here, this file wins.** Each entry says what was decided, why, and what
was rejected.

Nothing here overrides `requirements-analyst-open-questions.md` Section A, which
is settled owner policy.

---

## 1. The tenant session variable — a real defect, not a difference of style

**Decided: `set_config()`, never `SET … = ?`.**

`architect-architecture.md:275` writes the tenant into the PostgreSQL session
with a parameterized function call; `database-designer-eloquent-schema.md:320`
wrote it as `DB::statement('SET app.current_trainer_id = ?', [...])`.

The second form does not work. PostgreSQL's `SET` is a utility command, not a
parameterizable statement — it cannot take a bind placeholder. Proven against
PostgreSQL 17 rather than argued from the manual:

```
PREPARE arch(text) AS SELECT set_config('app.current_trainer_id', $1, false);
  → PREPARE, EXECUTE arch('42') → set_config = 42

PREPARE schema_variant(text) AS SET app.current_trainer_id = $1;
  → ERROR: syntax error at or near "SET"
```

This is the mechanism every Row-Level Security policy reads. The bound form
would have failed on the first request of every session — every login, every
page — and the failure would have arrived as a SQL syntax error rather than as
anything resembling a tenancy problem.

**Applied** to `database-designer-eloquent-schema.md` already; the code sample
now uses `DB::selectOne('select set_config(?, ?, false)', [...])` and carries
the reasoning inline. The architecture's Decisions table already rejected the
`SET` form explicitly; the schema had simply not inherited that row.

*Rejected:* `SET app.x = '$id'` with the id interpolated into SQL — it works,
but it puts a request-controlled value into a statement string for no benefit.
`SET LOCAL` / `set_config(…, true)` — transaction-scoped, so the tenant is lost
on any unit of work that does not open a transaction, and not every one here
does.

---

## 2. Eloquent model namespace

**Decided: flat `App\Models\`.** Actions, Policies, Controllers, Form Requests
and Jobs stay module-scoped under `App\<Module>\`.

`architect-architecture.md` names `App\Identity\Models` once;
`database-designer-eloquent-schema.md` uses `App\Models\` in 28 places and is
the document that defines all 58 models concretely.

Flat wins on three counts. It is Laravel's own convention, so a Laravel
developer finds models where they expect them. It is where the weight of
concrete definition already sits. And it costs nothing architecturally: the
module boundary rule is *"a module may write another module's data only through
the owning module's service"*, and that is enforced on Actions and Policies —
the layers where a violation actually happens — not on model class names. A
`Crm` action reading an `Event` model is legitimate; a `Crm` action writing an
`Event` directly is not, and the arch test that forbids it inspects the Action,
not the model.

*Rejected:* module-scoped models (`App\Scheduling\Models\Event`). It reads
tidily and mirrors the Symfony edition, but it buys no enforcement the arch
tests do not already provide, and it surprises every Laravel reader.

---

## 3. Token ledger service name

**Decided: `TokenLedgerService`.**

`architect-architecture.md` calls it `TokenLedger`; the schema calls it
`TokenLedgerService`; the API spec uses both. No substantive difference — this
is a coin flip resolved toward the document that names it in context alongside
the tables it writes. What matters is that exactly one name exists, because it
is the **sole writer** of ledger entries and a second name invites a second
writer.

---

## 4. Application root

**Decided: `Task/app/`, a new directory beside `Epics/` and `designs/`.**

The architecture raised this as its Open question 1, reasoning that
`Task/Epics/` is read-only client material and a live application underneath it
is questionable. The concern is sound but the premise is not: `Task/app/` sits
*beside* `Epics/` and `designs/`, never inside either. This is settled by the
owner's brief, and the Symfony edition ran the same layout with `git status`
proving `Task/Epics/` and `Task/designs/` byte-for-byte unchanged throughout.

Not an open question. Do not re-raise it.

---

## 5. Scaffolding the schema assumed but that does not exist

`database-designer-eloquent-schema.md` reconciles parts of itself against
`compose.yaml`, `00_roles.sql`, a startup gate and `config/tenancy/*.txt` as
though they were present. **They are not.** No Laravel application exists in
this edition yet; `Task/` holds only `Epics/` and `designs/`.

Every such artifact is therefore a **requirement Phase 3 must create**, not an
observation either document inherited. Treat the schema's references to them as
a specification of what the walking skeleton owes.

---

## 6. Still genuinely open

Carried forward rather than resolved, because no input settles them:

- **Rate-limit thresholds** for login, public-code lookup and payment attempts.
  The API spec proposes values and flags them as proposals; no epic states one.
- **Tenant-resolution sources 1 and 4** are named in the carried-over council
  verdict but never defined in any available input for this edition.
- **Octane leakage.** The Symfony edition had no equivalent risk. If Octane is
  ever adopted, the tenant session variable and the resolved `TenantContext`
  both need explicit resets between requests, and the schema's own note is the
  starting point.
