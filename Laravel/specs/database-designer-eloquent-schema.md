# Design: PracticePerfect Eloquent schema (Laravel + PostgreSQL)

Complete PostgreSQL 17 schema for the PracticePerfect platform on **Laravel**,
covering all eight epics in one pass (BR-01 through BR-08, plus every
acceptance criterion that names a data field). This document is a **port**,
not a fresh design: `specs/database-designer-schema.md` is the finished
58-table design produced during this project's Symfony edition, and
`specs/MANIFEST.md` marks it `translate` — the table set, keys, constraints,
indexes, RLS policies, ledger invariants, and the integer platform-fee
formula all carry over unchanged; only the *mapping mechanism* — the prior
edition's persistence-framework plumbing (its models, data-access classes,
query filters, migration tool) — is re-expressed, as **Laravel migrations,
Eloquent models, factories and seeders**. No terminology from that prior
framework's mapping layer appears anywhere below — where the source used a
framework-specific mechanism, this document names the Laravel mechanism
that does the same job and says so explicitly.

**What ports directly** (unchanged from the source, verified against it
below): the table set and relationships; column types, nullability,
defaults; primary/foreign keys and `ON DELETE` behavior; unique/CHECK
constraints; indexes and the queries that justify them; which tables are
global vs. trainer-scoped; the Row-Level Security policies; the token-ledger
invariants I1–I7; the integer round-half-up platform-fee formula (re-verified
against BR-05-7 and AC-05-11 below, independently of the source's own
verification); the migration build order 01 → 02 → {03, 04} → 05 → {06, 07,
08}.

**What is re-expressed, not copied**: every source mapping note becomes an
Eloquent model definition (`$fillable`/`$guarded`, casts including enum
casts, relationships, global scopes for tenancy); the source's query-level
tenant filter (Layer 2 of tenancy enforcement) becomes an Eloquent global
scope; `TenantResolver` /
`kernel.request` become Laravel middleware and a container binding, per
`specs/council-sharelink-tenant-resolution.md`'s own instruction that this
translation is required; the source's named data-access methods become
Eloquent query scopes, model methods, or Action/Service classes (Laravel
has no separate data-access layer to mirror — Eloquent models and the
query builder fill that role directly, per this repo's own
`.claude/skills/database-designer/SKILL.md`);
read-heavy screens use `toBase()`/`DB::table()` query-builder projections
instead of hydrating Eloquent models.

**No Laravel application scaffold exists yet in this repo** (`Task/app/`
does not exist — confirmed by directory listing before writing this
document), unlike the Symfony edition, which reconciled its schema against
real files (`compose.yaml`, `00-roles.sh`, its persistence-layer
package configuration, `config/tenancy/*.txt`). This document therefore *originates* the
Laravel-side infrastructure those files provided (database roles, RLS
bootstrap, tenancy config) rather than reconciling against it. Every place
that is true is called out explicitly rather than silently assumed —
see "Decisions" and "Open questions".

Traceability: every table cites the epic spec (file, heading, business rule)
it derives from, plus the exact section of `specs/database-designer-schema.md`
it ports. Business rules and data requirements were independently re-read
from `specs/requirements-analyst-epic-0{1..8}-*-spec.md` (`## Business rules`,
`## Data requirements`) to verify the source schema's citations before
porting them — not merely copied from its notes.

---

## Conventions

Stated once, referenced throughout, so table sections stay short.

### Table naming: Laravel's plural convention, applied mechanically

The source used singular table names (`trainer`, `event`, `rsvp`) because
its prior framework's naming strategy allows it. Laravel's convention is plural
snake_case table names with a singular StudlyCase model class
(`Trainer` → `trainers`). This document renames every table on that
convention and **every model declares `protected $table` explicitly**
rather than relying on Eloquent's automatic pluralization — this sidesteps
any ambiguity for naturally-uncountable or irregular words (`content_progress`,
`content_usage`, `trainer_billing_settings`) by simply stating the answer,
rather than asserting how Eloquent's inflector would resolve them. This is
the one mechanical, project-wide renaming; every other identifier
(columns, constraint logic, index shape) is unchanged from the source.

| Source table (Symfony) | Laravel table | Model |
|---|---|---|
| `account` | `accounts` | `Account` |
| `account_profile` | `account_profiles` | `AccountProfile` |
| `parent_child_link` | `parent_child_links` | `ParentChildLink` |
| `player_profile` | `player_profiles` | `PlayerProfile` |
| `email_verification_token` | `email_verification_tokens` | `EmailVerificationToken` |
| `password_reset_token` | `password_reset_tokens` | `PasswordResetToken` |
| `user_deletion_record` | `user_deletion_records` | `UserDeletionRecord` |
| `trainer` | `trainers` | `Trainer` |
| `trainer_branding_settings` | `trainer_branding_settings` | `TrainerBrandingSetting` |
| `account_trainer_link` | `account_trainer_links` | `AccountTrainerLink` |
| `public_tenant_code` | `public_tenant_codes` | `PublicTenantCode` |
| `audit_log_entry` | `audit_log_entries` | `AuditLogEntry` |
| `impersonation_session` | `impersonation_sessions` | `ImpersonationSession` |
| `platform_configuration` | `platform_configurations` | `PlatformConfiguration` |
| `feature_toggle` | `feature_toggles` | `FeatureToggle` |
| `platform_subscription` | `platform_subscriptions` | `PlatformSubscription` |
| `stripe_customer_link` | `stripe_customer_links` | `StripeCustomerLink` |
| `stripe_event_receipt` | `stripe_event_receipts` | `StripeEventReceipt` |
| `trainer_billing_settings` | `trainer_billing_settings` | `TrainerBillingSetting` |
| `coach_membership` | `coach_memberships` | `CoachMembership` |
| `player_trainer_membership` | `player_trainer_memberships` | `PlayerTrainerMembership` |
| `share_link` | `share_links` | `ShareLink` |
| `share_link_open` | `share_link_opens` | `ShareLinkOpen` |
| `availability_window` | `availability_windows` | `AvailabilityWindow` |
| `child_approval_request` | `child_approval_requests` | `ChildApprovalRequest` |
| `event` | `events` | `Event` |
| `rsvp` | `rsvps` | `Rsvp` |
| `attendance_record` | `attendance_records` | `AttendanceRecord` |
| `attendance_edit` | `attendance_edits` | `AttendanceEdit` |
| `coach_assignment` | `coach_assignments` | `CoachAssignment` |
| `coach_availability_override` | `coach_availability_overrides` | `CoachAvailabilityOverride` |
| `event_invitation` | `event_invitations` | `EventInvitation` |
| `event_duplication_record` | `event_duplication_records` | `EventDuplicationRecord` |
| `label` | `labels` | `Label` |
| `player_label` | `player_labels` | `PlayerLabel` |
| `player_flag` | `player_flags` | `PlayerFlag` |
| `player_note` | `player_notes` | `PlayerNote` |
| `playlist` | `playlists` | `Playlist` |
| `content_item` | `content_items` | `ContentItem` |
| `drill_detail` | `drill_details` | `DrillDetail` |
| `playlist_item` | `playlist_items` | `PlaylistItem` |
| `playlist_assignment` | `playlist_assignments` | `PlaylistAssignment` |
| `content_progress` | `content_progress` | `ContentProgress` |
| `playlist_access_grant` | `playlist_access_grants` | `PlaylistAccessGrant` |
| `content_usage` | `content_usage` | `ContentUsage` |
| `token_entry` | `token_entries` | `TokenEntry` |
| `token_balance` | `token_balances` | `TokenBalance` |
| `payment_record` | `payment_records` | `PaymentRecord` |
| `token_package` | `token_packages` | `TokenPackage` |
| `subscription_entitlement` | `subscription_entitlements` | `SubscriptionEntitlement` |
| `entitlement_coverage` | `entitlement_coverages` | `EntitlementCoverage` |
| `referral_link` | `referral_links` | `ReferralLink` |
| `referral` | `referrals` | `Referral` |
| `referral_assist_count` | `referral_assist_counts` | `ReferralAssistCount` |
| `coupon` | `coupons` | `Coupon` |
| `coupon_redemption` | `coupon_redemptions` | `CouponRedemption` |
| `form` | `forms` | `Form` |
| `form_submission` | `form_submissions` | `FormSubmission` |

`password_reset_tokens` collides in name only with Laravel's own stock
starter-kit migration (`email`, `token`, `created_at`). This design's version
is keyed by `account_id` FK + `token_hash` and carries `expires_at`/
`consumed_at`, matching BR-01-4's 1-hour expiry and the account-centric
model used everywhere else in this schema — **it replaces, not extends,**
the stock migration. See Decisions.

Models are shown in a flat `App\Models\*` namespace throughout — Laravel's
default and the right choice for a database-design document. If a later
architecture stage adopts module/domain namespaces (mirroring the Symfony
edition's module map), these models relocate without any change to the
schema itself; that is an `architect` decision, not a database one, and is
deliberately not pre-empted here (matching `specs/MANIFEST.md`'s own reason
for not carrying `architect-architecture.md` into this edition).

### Postgres type -> Laravel migration call

Stated once; per-table sections give the Postgres type (parallel to the
source, for easy side-by-side verification) and call out the Laravel
migration API only where it is not this mechanical mapping.

| Postgres type | Laravel `Blueprint` call |
|---|---|
| `BIGINT GENERATED BY DEFAULT AS IDENTITY` (PK) | `$table->id();` |
| `BIGINT` (FK) | `$table->foreignId('x_id')->constrained('xs')->restrictOnDelete();` (or `->cascadeOnDelete()` per FK — stated per table) |
| `BIGINT` (nullable FK) | `$table->foreignId('x_id')->nullable()->constrained('xs')->restrictOnDelete();` |
| `BIGINT` (deferred FK — target table doesn't exist yet) | `$table->unsignedBigInteger('x_id')->nullable();` in the creating migration; `$table->foreign('x_id')->references('id')->on('xs')->restrictOnDelete();` in a later `Schema::table()` migration once the target exists |
| `VARCHAR(n)` | `$table->string('col', n);` |
| `VARCHAR` + `CHECK (col IN (...))` (closed vocabulary) | `$table->enum('col', [...]);` — **Laravel's Postgres grammar compiles `enum()` to exactly `varchar(255) check ("col" in (...))`**, so this one Blueprint call reproduces the source's explicit "VARCHAR + CHECK, not native `CREATE TYPE ... ENUM`" decision with no raw SQL needed for the single-column case |
| `TEXT` | `$table->text('col');` |
| `CITEXT` | `$table->addColumn('citext', 'col');` — no native helper; requires `CREATE EXTENSION citext` (Blueprint's generic `addColumn(type, name)` escape hatch, documented Laravel API for DB-specific types) |
| `BOOLEAN` | `$table->boolean('col');` |
| `INTEGER` | `$table->integer('col');` (Postgres has no unsigned integer type, so `unsignedInteger()` buys nothing here — positivity is a CHECK, matching the source) |
| `SMALLINT` | `$table->smallInteger('col');` |
| `DATE` | `$table->date('col');` |
| `TIME` | `$table->time('col');` |
| `TIMESTAMPTZ` | `$table->timestampTz('col');` / `$table->timestampsTz();` for the `created_at`+`updated_at` pair — Laravel's plain `timestamps()` creates timezone-*naive* `timestamp` columns; this schema uses the `...Tz()` variants everywhere, matching the source's blanket `TIMESTAMPTZ` convention |
| `JSONB` | `$table->jsonb('col');` |
| `TEXT[]` (source's array columns) | **Changed to `JSONB`** — see Decisions |
| `INET` | `$table->addColumn('inet', 'col')->nullable();` |
| `CHAR(7)` (hex color) | `$table->char('col', 7);` |
| `GENERATED ALWAYS AS (expr) STORED` | `$table->someType('col')->storedAs('expr');` — Laravel's `storedAs()`/`virtualAs()` column modifiers, supported on the Postgres grammar |
| Partial/composite unique, `EXCLUDE`, cross-column `CHECK`, `DEFERRABLE` | raw `DB::statement(...)` inside the migration's `up()` — the schema builder has no fluent API for these; this is the same escape hatch this repo's own `database-designer` skill names explicitly |

### Money, tokens, enums, soft delete, capacity — unchanged from the source

- **Money**: `INTEGER` minor units (cents), columns end `_minor_units`, never
  `float`/`double`. Ported unchanged — architecture's "Stack" decision and
  the $1–$10,000 per-transaction ceiling (AC-08-5) are product facts, not
  Symfony facts.
- **Tokens**: plain `INTEGER`, never `_minor_units` — whole units by design
  (`requirements-analyst-epic-05-payments-tokens-spec.md`, "Out of MVP
  scope").
- **Enums**: `VARCHAR` + `CHECK`, mapped on the PHP side to native backed
  enums (`enum AccountRole: string { case SuperAdmin = 'super_admin'; ... }`)
  via Eloquent's built-in enum cast (`'role' => AccountRole::class` in
  `casts()`) — no custom cast class needed, per this repo's own `eloquent`
  skill ("For a native backed PHP enum, use the plain enum cast — no custom
  class needed"). This is the direct Laravel analog of the source's "native
  PHP 8.1+ backed enum..." — same layering (DB CHECK + PHP type system),
  reached through a different framework hook.
- **Soft delete**: Eloquent's `SoftDeletes` trait + `$table->softDeletes()`
  on `playlists`, `content_items`, `drill_details` only — a direct, built-in
  match for the source's `deleted_at TIMESTAMPTZ NULL` convention (AC-04-36).
- **Capacity checks**: no cached counter column anywhere in this schema,
  same as the source. `SELECT ... FOR UPDATE` on the capacity-holding row
  (`events`/`forms`) inside `DB::transaction()`, via Eloquent's
  `lockForUpdate()` query builder method, then `COUNT(*)` of confirmed child
  rows in the same transaction.
- **Array columns** (`event.skill_levels`, `event.genders`,
  `playlist.filter_skill_levels/filter_positions/filter_age_levels`,
  `content_item.tags`, `drill_detail.equipment/categories`): the source used
  native Postgres `TEXT[]`, which the source's database abstraction layer
  maps natively. **Eloquent has
  no built-in cast for native Postgres arrays** — its stock `array`/`collection`
  casts serialize to JSON and do not speak Postgres's `{a,b,c}` wire format,
  so `TEXT[]` would require eight bespoke `CastsAttributes` classes for no
  functional gain. This document stores these columns as `JSONB` instead,
  cast with Eloquent's built-in `'array'` cast — zero custom code, equivalent
  query capability via Postgres's JSONB operators (`@>`, `?`, GIN-indexable
  if ever needed), and no CHECK constraint on any of these columns in the
  source to lose. See Decisions.

### Tenancy: the five layers, re-expressed

The source's tenancy design (from the un-carried `architect-architecture.md`,
summarized in `specs/database-designer-schema.md` and
`specs/council-sharelink-tenant-resolution.md`) has five enforcement layers.
Layers 1 and 5 are pure schema/SQL and port unchanged; Layers 2–4 are
prior-framework mechanisms and are re-expressed here as their Laravel
equivalents. This document designs only as much of the mechanism as the
schema needs to be correct and usable — the full request-lifecycle
authorization architecture (route allow-lists, Super Admin administrative
scope, voters/Policies) is `architect`/`auth-scaffolding` territory, per
`specs/MANIFEST.md`'s own reasoning for not carrying the architecture
document into this edition ("the Laravel equivalents... are different
enough that translating prose would be slower and less trustworthy than
deriving them natively").

| Layer | Symfony mechanism | Laravel mechanism |
|---|---|---|
| 1. Denormalized tenancy key | `trainer_id` column on every trainer-scoped model | Same column, same rule — first column after `id`, always indexed |
| 2. Query-level filter | A query-level tenant filter | **Eloquent global scope** (`TenantScope`), added by a `BelongsToTenant` trait every trainer-scoped model uses |
| 3. Tenant resolution | `TenantResolver`, invoked from a `kernel.request` listener | **`ResolveTenant` middleware**, registered early in the HTTP middleware stack, binding a request-scoped `TenantContext` |
| 4. Post-load assertion | A framework-level post-load listener asserting the loaded row's `trainer_id` matches the resolved tenant | A `retrieved`/`saving` model-event assertion in the same `BelongsToTenant` trait (defense in depth — Layer 2 should already make a mismatch impossible to load) |
| 5. Database policy | PostgreSQL Row-Level Security | **Unchanged** — RLS is server-side SQL; Eloquent/Laravel are not in this layer at all |

**`TenantContext`** (`app/Support/Tenancy/TenantContext.php`) — a small,
request-scoped value holder, bound as a singleton per request:

```php
<?php

declare(strict_types=1);

namespace App\Support\Tenancy;

final class TenantContext
{
    private ?int $trainerId = null;

    public function resolveTo(int $trainerId): void
    {
        $this->trainerId = $trainerId;
    }

    public function clear(): void
    {
        $this->trainerId = null;
    }

    public function id(): ?int
    {
        return $this->trainerId;
    }

    public function requireId(): int
    {
        return $this->trainerId ?? throw new UnresolvedTenantException();
    }
}
```

Bound in `AppServiceProvider::register()` as `$this->app->scoped(TenantContext::class)`
(`scoped`, not `singleton` — resets per request under Octane; see the Octane
note below).

**`ResolveTenant` middleware** (`app/Http/Middleware/ResolveTenant.php`) —
the direct translation of `TenantResolver`/`kernel.request`, run early
(before route-model binding needs a resolved tenant, after the session/auth
middleware so an authenticated candidate is available):

```php
<?php

declare(strict_types=1);

namespace App\Http\Middleware;

use App\Support\Tenancy\TenantContext;
use Closure;
use Illuminate\Http\Request;
use Illuminate\Support\Facades\DB;
use Symfony\Component\HttpFoundation\Response;

final class ResolveTenant
{
    public function __construct(private readonly TenantContext $context) {}

    public function handle(Request $request, Closure $next): Response
    {
        $trainerId = $this->resolve($request);

        if ($trainerId !== null) {
            $this->context->resolveTo($trainerId);
        }

        // Unconditional on every request, resolved or not — see the Octane
        // note below for why this line cannot be skipped when unresolved.
        //
        // set_config(), NOT `SET app.current_trainer_id = ?`. PostgreSQL's SET
        // command is not a parameterizable statement: preparing it with a bind
        // placeholder fails outright with "syntax error at or near SET", so the
        // bound form would break on the very first request of every session.
        // set_config() is an ordinary function call and does accept parameters,
        // which keeps the trainer id bound rather than interpolated into SQL.
        // The third argument false makes it session-scoped, like SET and unlike
        // SET LOCAL, which would lose the tenant outside a transaction.
        // Matches architect-architecture.md, "Session variable statement".
        DB::selectOne('select set_config(?, ?, false)', [
            'app.current_trainer_id',
            (string) ($trainerId ?? 0),
        ]);

        return $next($request);
    }

    private function resolve(Request $request): ?int
    {
        // Source 2 — active impersonation session.
        if ($trainerId = app(ImpersonationTenantResolver::class)->resolve($request)) {
            return $trainerId;
        }

        // Source 5 — public code on an allow-listed route, validated against
        // the global public_tenant_codes mapping. Outranks source 3 on these
        // routes specifically (council verdict, "Precedence").
        if ($request->route()?->middleware() && in_array('tenant.public-code', $request->route()->gatherMiddleware(), true)) {
            return app(PublicCodeTenantResolver::class)->resolve($request);
        }

        // Source 3 — the session-held or route-supplied trainer context,
        // validated against the global account_trainer_links table. An
        // unvalidated candidate resolves nothing (council's amended source 3).
        return app(SessionTenantResolver::class)->resolve($request);
    }
}
```

Each `*TenantResolver` above is a small, single-purpose class — the
Laravel-idiomatic equivalent of one Symfony resolution "source". **Sources 1
(Super Admin administrative scope) and 4** are named by the (un-carried)
architecture document but never defined in any input this document has
access to; inventing their mechanics here would violate this task's own
"never invent a requirement" instruction. The pipeline above is written so
they can be added as additional resolver classes without restructuring it —
flagged in Open questions rather than guessed.

**`BelongsToTenant` trait** (`app/Models/Concerns/BelongsToTenant.php`) —
applied to every trainer-scoped model:

```php
<?php

declare(strict_types=1);

namespace App\Models\Concerns;

use App\Models\Scopes\TenantScope;
use App\Support\Tenancy\TenantContext;
use Illuminate\Database\Eloquent\Model;

trait BelongsToTenant
{
    public static function bootBelongsToTenant(): void
    {
        static::addGlobalScope(new TenantScope);

        static::creating(function (Model $model): void {
            $model->trainer_id ??= app(TenantContext::class)->requireId();
        });

        // Layer 4 — defense in depth. Layer 2 (the global scope) should
        // already make loading a foreign trainer's row impossible; this
        // catches the case where a query legitimately bypassed the scope
        // (withoutGlobalScope, a raw query result rehydrated into a model)
        // without also validating trainer_id.
        static::retrieved(function (Model $model): void {
            $tenantId = app(TenantContext::class)->id();

            if ($tenantId !== null && $model->trainer_id !== $tenantId) {
                throw new \App\Support\Tenancy\CrossTenantHydrationException($model);
            }
        });
    }
}
```

`TenantScope` (`app/Models/Scopes/TenantScope.php`) implements
`Illuminate\Database\Eloquent\Scope` and adds
`$builder->where($model->qualifyColumn('trainer_id'), app(TenantContext::class)->id())`.
Like the query-level filter it replaces, an Eloquent global scope only
affects **read** queries built through the Eloquent query builder — it has
no effect on `save()`/`update()`/`delete()` calls or raw inserts. The
write-side guarantee is Layer 5 (RLS `WITH CHECK`) plus the `creating` hook
above that fills `trainer_id` from `TenantContext` before every insert. This
split (global scope = read filter, RLS = the real write guarantee) is
identical to the source's own layering — its query-level filters have the
same SELECT-only scope, so there is no impedance mismatch here, just a
naming one.

**Two tables never get `TenantScope` and never get RLS**: `AccountTrainerLink`
and `PublicTenantCode` do not use `BelongsToTenant` — they are read by
`ResolveTenant` itself, before a tenant exists. See "Row-Level Security" for
why a policy on either would silently break every login and every
public-code route.

**The widened publication policy** (`playlists`, `content_items`): global
scopes apply uniformly across a model, but this pair's RLS SELECT policy is
deliberately wider than its INSERT/UPDATE/DELETE policies (published content
is cross-tenant readable). These two models use a dedicated
`PublishedOrOwnedScope` instead of the plain `TenantScope` — see their table
sections below.

### Laravel Octane note (a risk this design must flag that the Symfony port did not have)

The source's session-variable mechanism is explicitly justified by "the
stack carries no persistent connections and no connection pooler" — true for
traditional PHP-FPM, where each request gets a fresh process (and Laravel's
default `pgsql` connection is non-persistent). **If this application ever
runs under Laravel Octane** (or any long-lived-worker model), a single PHP
process — and its already-open PostgreSQL connection — serves many requests
in sequence. `SET app.current_trainer_id` is connection-scoped, not
request-scoped, so a stale value from a previous request's tenant would
leak into the next request on the same worker **unless every request
unconditionally re-issues the `SET`**, including the unresolved sentinel
(`'0'`) when no tenant resolves. `ResolveTenant` above already does this
unconditionally (the `DB::statement` call is outside the `if`), specifically
to close this gap — call it out explicitly rather than let a future reader
assume PHP-FPM's per-request isolation still holds. Also carried over
unchanged from the source: never place a transaction-mode connection pooler
(e.g., PgBouncer in transaction mode) in front of this database — it
silently reuses a physical connection across sessions, which breaks the
`SET`-per-connection assumption the same way Octane does, and would be far
harder to diagnose. `SET` (session-scoped), not `SET LOCAL`
(transaction-scoped), is still the right choice — not every unit of work in
a Laravel request runs inside an explicit `DB::transaction()`.

### Database roles — new infrastructure, not a port

`00-roles.sh` exists in the Symfony edition's `Task/app/docker/postgres/initdb/`;
**no equivalent exists anywhere in this Laravel repo** (`Task/app/` does not
exist). The three-role design (`pp_owner`, `pp_app`, `pp_crossing`) is a
product/security decision worth keeping, so this document originates the
Laravel-side bootstrap rather than silently assuming it:

- **`pp_owner`** — DDL owner. Runs `php artisan migrate` and nothing else;
  never the credential set the running web/queue app uses. `FORCE ROW LEVEL
  SECURITY` is deliberately never set on any table (matching the source), so
  this role's migrations and backfills are never blocked by their own
  policies.
- **`pp_app`** — the running application's only credential, subject to RLS
  on every trainer-scoped table. `config/database.php`'s default `pgsql`
  connection uses this role.
- **`pp_crossing`** — `BYPASSRLS`, `SELECT`-only, used exclusively by
  `CrossTenantReadService` (below) for genuinely cross-tenant reads (the
  "used by N trainers" figure on `content_usage`, and any future admin
  cross-tenant report). A second named connection, `pgsql_crossing`, in
  `config/database.php`.

Role and grant DDL is **cluster-level** (`CREATE ROLE`) and typically needs
superuser privileges a normal migration-runner credential does not have —
the same reason the source kept it in a provisioning script
(`00-roles.sh`), not a schema migration. This document proposes the same
separation: a plain SQL file, `database/pgsql/00_roles.sql`, run once by
whoever provisions the database (locally: `psql -f database/pgsql/00_roles.sql`;
in a managed environment: adapted to that provider's bootstrap procedure —
see Open questions, since managed Postgres (RDS, Cloud SQL) often restricts
role creation and `BYPASSRLS` grants to specific bootstrap paths that this
document cannot assume without knowing the target host).

```sql
-- database/pgsql/00_roles.sql
-- Run once, by a superuser, before the first `php artisan migrate`.

CREATE ROLE pp_owner LOGIN PASSWORD :'pp_owner_password';
CREATE ROLE pp_app   LOGIN PASSWORD :'pp_app_password';
CREATE ROLE pp_crossing LOGIN PASSWORD :'pp_crossing_password' BYPASSRLS;

GRANT ALL PRIVILEGES ON DATABASE practiceperfect TO pp_owner;
ALTER DATABASE practiceperfect OWNER TO pp_owner;

-- New tables automatically grant pp_app full DML and pp_crossing SELECT-only.
-- Individual migrations narrow this further for token_entries/audit_log_entries.
ALTER DEFAULT PRIVILEGES FOR ROLE pp_owner
    IN SCHEMA public
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO pp_app;

ALTER DEFAULT PRIVILEGES FOR ROLE pp_owner
    IN SCHEMA public
    GRANT SELECT ON TABLES TO pp_crossing;

ALTER DEFAULT PRIVILEGES FOR ROLE pp_owner
    IN SCHEMA public
    GRANT USAGE, SELECT ON SEQUENCES TO pp_app;
```

Operationally: the artisan process that runs `php artisan migrate` (deploy
step or CI) must resolve its `DB_USERNAME`/`DB_PASSWORD` to `pp_owner`; the
actual web/queue application processes resolve theirs to `pp_app`. Both
point at the same `pgsql` connection name in `config/database.php` — only
the environment's credentials differ between the migration step and the
running application, matching the source's "`pp_owner`... never used by the
running application."

**`CrossTenantReadService`** (`app/Services/CrossTenantReadService.php`) —
kept as a single, named, auditable class, same shape and same name as the
source's, using the `pgsql_crossing` connection exclusively:

```php
<?php

declare(strict_types=1);

namespace App\Services;

use Illuminate\Support\Facades\DB;

final class CrossTenantReadService
{
    public function contentItemUsageCount(int $contentItemId): int
    {
        return (int) DB::connection('pgsql_crossing')
            ->table('content_usage')
            ->where('content_item_id', $contentItemId)
            ->count();
    }
}
```

Every method on this class is a genuinely cross-tenant question answered by
a genuinely cross-tenant mechanism, matching the source's own framing for
`content_usage` — not a general-purpose escape hatch. New methods here
should be rare and reviewed.

---

## Global tables

No `trainer_id`, no `BelongsToTenant` trait, no RLS. Protected by
application-level authorization (Policies/Gates) only. Ported from
`specs/database-designer-schema.md` "Global tables" (lines 139–614)
unchanged in shape; every difference from the source is a Laravel mapping
choice, called out per table.

### `accounts` — was `account`

One row per person or Super Admin. `requirements-analyst-epic-01-user-management-spec.md`
"Data requirements" ("User", line 310); BR-01-2, BR-01-7.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `email` | CITEXT | NO | | Case-insensitive per BR-01-2. `CREATE EXTENSION citext` required |
| `password_hash` | VARCHAR(255) | NO | | BR-01-1 |
| `role` | VARCHAR(16) | NO | | `super_admin`\|`trainer`\|`coach`\|`player` — BR-01-7, "no role hierarchy" |
| `status` | VARCHAR(16) | NO | `'active'` | `active`\|`inactive`\|`deleted` — BR-01-23, BR-01-24 |
| `email_verified_at` | TIMESTAMPTZ | YES | NULL | Q-01.05 default: required before first login |
| `last_login_at` | TIMESTAMPTZ | YES | NULL | |
| `created_at` / `updated_at` | TIMESTAMPTZ | NO | | |

- Unique: `email`. Check: `role IN (...)`, `status IN (...)`.
- Indexes: `(role)` — Users tool role filter (AC-07-9); `(status)` —
  active-user counts (BR-07-7).
- GDPR deletion (BR-01-24) is a real `UPDATE`, not a soft-delete row: `email`
  becomes `deleted_<id>@example.com` (deterministic — the unique index can
  never collide), `password_hash` a random unusable value, `status` becomes
  `deleted`. `account_profiles` is nulled in the same transaction. Every FK
  pointing at this account keeps resolving.

```php
// database/migrations/2025_01_02_000001_create_accounts_table.php
<?php

declare(strict_types=1);

use Illuminate\Database\Migrations\Migration;
use Illuminate\Database\Schema\Blueprint;
use Illuminate\Support\Facades\DB;
use Illuminate\Support\Facades\Schema;

return new class extends Migration
{
    public function up(): void
    {
        Schema::create('accounts', function (Blueprint $table): void {
            $table->id();
            $table->addColumn('citext', 'email');
            $table->string('password_hash', 255);
            $table->enum('role', ['super_admin', 'trainer', 'coach', 'player']);
            $table->enum('status', ['active', 'inactive', 'deleted'])->default('active');
            $table->timestampTz('email_verified_at')->nullable();
            $table->timestampTz('last_login_at')->nullable();
            $table->timestampsTz();

            $table->unique('email');
            $table->index('role');
            $table->index('status');
        });
    }

    public function down(): void
    {
        Schema::dropIfExists('accounts');
    }
};
```

**Model** `App\Models\Account`:

```php
final class Account extends Authenticatable
{
    use HasFactory;

    protected $table = 'accounts';

    protected $fillable = ['email', 'role', 'status'];

    protected $hidden = ['password_hash'];

    protected function casts(): array
    {
        return [
            'password_hash' => 'hashed', // auto-hashes on set, Laravel 10.14+
            'role' => AccountRole::class,
            'status' => AccountStatus::class,
            'email_verified_at' => 'datetime',
            'last_login_at' => 'datetime',
        ];
    }

    // Authenticatable::getAuthPassword() defaults to the `password` column;
    // overridden because this schema names it `password_hash` (BR-01-1's
    // "securely hashed" is the requirement — the column name is a Laravel
    // mapping detail, not a product fact worth bending to match).
    public function getAuthPassword(): string
    {
        return $this->password_hash;
    }

    public function profile(): HasOne
    {
        return $this->hasOne(AccountProfile::class, 'account_id');
    }

    public function trainer(): HasOne
    {
        return $this->hasOne(Trainer::class, 'owner_account_id');
    }

    public function trainerLinks(): HasMany
    {
        return $this->hasMany(AccountTrainerLink::class, 'account_id');
    }
}
```

No RLS (global). Not `BelongsToTenant`.

### `account_profiles` — was `account_profile`

1-to-1 extension holding common profile fields. `requirements-analyst-epic-01-user-management-spec.md`
"Data requirements" ("Profile", line 311); AC-01-48.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `account_id` | BIGINT | NO | | **PK, and FK** to `accounts(id)` ON DELETE CASCADE |
| `first_name` / `last_name` | VARCHAR(100) | NO | | |
| `phone` | VARCHAR(32) | YES | NULL | AC-01-50, format-validated at the Form Request layer |
| `photo_url` | VARCHAR(2048) | YES | NULL | AC-01-49 |
| `school_or_organization` | VARCHAR(255) | YES | NULL | |
| `created_at` / `updated_at` | TIMESTAMPTZ | NO | | |

- Shared-PK 1-to-1 extension (the source declared this pairing with
  attribute-based mapping declarations on the FK column). Eloquent's
  idiomatic equivalent: a model with a non-auto-incrementing custom
  primary key.

```php
Schema::create('account_profiles', function (Blueprint $table): void {
    $table->foreignId('account_id')->primary()->constrained('accounts')->cascadeOnDelete();
    $table->string('first_name', 100);
    $table->string('last_name', 100);
    $table->string('phone', 32)->nullable();
    $table->string('photo_url', 2048)->nullable();
    $table->string('school_or_organization', 255)->nullable();
    $table->timestampsTz();
});
```

**Model**:

```php
final class AccountProfile extends Model
{
    protected $table = 'account_profiles';
    protected $primaryKey = 'account_id';
    public $incrementing = false;
    protected $keyType = 'int';

    protected $fillable = ['first_name', 'last_name', 'phone', 'photo_url', 'school_or_organization'];

    public function account(): BelongsTo
    {
        return $this->belongsTo(Account::class, 'account_id');
    }
}
```

No RLS.

### `parent_child_links` — was `parent_child_link`

`requirements-analyst-epic-01-user-management-spec.md` BR-01-16, BR-01-17,
AC-01-16, AC-01-20, AC-01-27.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `parent_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |
| `child_player_id` | BIGINT | NO | | FK → `player_profiles(id)` RESTRICT |
| `child_account_id` | BIGINT | YES | NULL | FK → `accounts(id)` RESTRICT. Set only if the child also has their own login (AC-01-20) |
| `allow_token_spending_without_approval` | BOOLEAN | NO | `false` | AC-01-27/28 |
| `created_at` | TIMESTAMPTZ | NO | | |

- Unique: `child_player_id`, `child_account_id`.
- Check: `parent_account_id <> child_account_id` — raw `DB::statement()`,
  Blueprint has no cross-column CHECK helper.
- Index: `(parent_account_id)` — "Family"/"Player Profiles" listing (AC-01-22).

**Model** `App\Models\ParentChildLink`: `$fillable = ['parent_account_id',
'child_player_id', 'child_account_id', 'allow_token_spending_without_approval']`;
casts `allow_token_spending_without_approval` => `'boolean'`; `belongsTo`
to `Account` (twice, `parentAccount()`/`childAccount()`) and `PlayerProfile`
(`childPlayer()`). No RLS.

### `player_profiles` — was `player_profile`

The person being trained. `requirements-analyst-epic-01-user-management-spec.md`
"Data requirements" ("Player profile", line 314); Q-01.02 default.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `self_account_id` | BIGINT | YES | NULL | FK → `accounts(id)` RESTRICT, UNIQUE |
| `first_name` | VARCHAR(100) | NO | | |
| `date_of_birth` | DATE | NO | | Q-01.02 default: store DoB, derive age at query time |
| `gender` | VARCHAR(32) | YES | NULL | |
| `is_child` | BOOLEAN | NO | `false` | Service-maintained, not generated — see note |
| `school_or_team` | VARCHAR(255) | YES | NULL | |
| `jersey_number` | VARCHAR(16) | YES | NULL | |
| `emergency_contact_name` | VARCHAR(255) | YES | NULL | |
| `emergency_contact_phone` | VARCHAR(32) | YES | NULL | |
| `photo_url` | VARCHAR(2048) | YES | NULL | |
| `created_at` / `updated_at` | TIMESTAMPTZ | NO | | |

- Unique: `self_account_id`.
- `is_child` cannot be a Postgres generated column (it would need to read
  `parent_child_links`, a different table — Postgres generated columns may
  only reference the same row, same restriction the source's framework hit). It is
  service-maintained: flipped `true` the moment a `ParentChildLink` model is
  created for this player by the Action that owns that write, and never
  legitimately flips back. A test asserts the two never disagree.
- Age 1–18 validation for children (AC-01-21) is a Form Request rule, not a
  CHECK — `now()` is not immutable, and the rule is about age *at creation*.
- The source's `emergency_contact_name`/`emergency_contact_phone`
  value-object mapping becomes an Eloquent **custom cast class** here — the
  real Eloquent mechanism for "combine sibling columns into one value
  object," per this repo's `eloquent` skill (`CastsAttributes`, reading
  multiple keys off the `$attributes` array passed to `get()`):

```php
final class EmergencyContact implements CastsAttributes
{
    public function get(Model $model, string $key, mixed $value, array $attributes): ?EmergencyContactValue
    {
        if ($attributes['emergency_contact_name'] === null) {
            return null;
        }

        return new EmergencyContactValue(
            $attributes['emergency_contact_name'],
            $attributes['emergency_contact_phone'],
        );
    }

    public function set(Model $model, string $key, mixed $value, array $attributes): array
    {
        return [
            'emergency_contact_name' => $value?->name,
            'emergency_contact_phone' => $value?->phone,
        ];
    }
}
```

**Model** `App\Models\PlayerProfile`: `$fillable = ['first_name',
'date_of_birth', 'gender', 'school_or_team', 'jersey_number', 'photo_url']`
(`self_account_id`, `is_child` are service-set, deliberately excluded from
mass assignment); casts: `'date_of_birth' => 'date'`, `'is_child' =>
'boolean'`, `'emergency_contact' => EmergencyContact::class`;
`belongsTo(Account::class, 'self_account_id')` as `selfAccount()`;
`hasOne(ParentChildLink::class, 'child_player_id')` as `parentLink()`;
`hasMany(PlayerTrainerMembership::class, 'player_id')`. No RLS.

### `email_verification_tokens` / `password_reset_tokens`

Same shape, different expiry. `requirements-analyst-epic-01-user-management-spec.md`
BR-01-5 (24h) / BR-01-4 (1h).

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `account_id` | BIGINT | NO | | FK → `accounts(id)` ON DELETE CASCADE |
| `token_hash` | VARCHAR(255) | NO | | SHA-256 of the emailed token — the raw token is never stored |
| `expires_at` | TIMESTAMPTZ | NO | | `created_at + 24h` (verification) / `+ 1h` (reset) |
| `consumed_at` | TIMESTAMPTZ | YES | NULL | |
| `created_at` | TIMESTAMPTZ | NO | | |

- Unique: `token_hash`. Index: `(account_id)`.
- **Deliberately replaces**, not extends, Laravel's stock
  `password_reset_tokens` migration (`email`+`token`, no expiry column, no
  FK) — see Decisions.

**Models** `App\Models\EmailVerificationToken` / `PasswordResetToken`:
`$fillable = ['account_id', 'token_hash', 'expires_at']`; casts
`'expires_at' => 'datetime'`, `'consumed_at' => 'datetime'`; `belongsTo(Account::class)`;
a local scope `scopeValid(Builder $query)` → `whereNull('consumed_at')->where('expires_at', '>', now())`.
No RLS.

### `user_deletion_records` — was `user_deletion_record`

GDPR compliance record. `requirements-analyst-epic-01-user-management-spec.md`
BR-01-24, AC-01-59.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `original_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |
| `original_email` | VARCHAR(255) | NO | | Snapshot, kept after `accounts.email` is anonymized |
| `deleted_by_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |
| `reason` | TEXT | YES | NULL | AC-01-59 |
| `deleted_at` | TIMESTAMPTZ | NO | `now()` | |

Index: `(original_account_id)`. **Model**: `$fillable = ['original_account_id',
'original_email', 'deleted_by_account_id', 'reason']`; two `belongsTo(Account::class)`
relations (`originalAccount()`, `deletedBy()`). No `updated_at` (append-only
by convention, not privilege — this table is small and not on the ledger/audit
privilege list). No RLS.

### `trainers` — was `trainer`

The tenant registry. Global because it is read pre-authentication and
cross-tenant (trainer switcher, public branding). `trainer_billing_settings`
splits off the sensitive half — see that table.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `owner_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT, UNIQUE |
| `business_name` | VARCHAR(255) | NO | | AC-01-2 |
| `slug` | VARCHAR(100) | NO | | URL-safe, referral link paths |
| `organization_address` | VARCHAR(500) | YES | NULL | |
| `organization_website` | VARCHAR(2048) | YES | NULL | |
| `organization_description` | TEXT | YES | NULL | |
| `timezone` | VARCHAR(64) | NO | `'America/New_York'` | Required by the 24h refund boundary (BR-05-10) and the 1-advance-booking-per-day rule (BR-05-14) |
| `created_at` / `updated_at` | TIMESTAMPTZ | NO | | |

- Unique: `owner_account_id`, `slug`. Index: `(slug)` — public
  form/referral-link resolution.
- No RLS — global, written only by the trainer-creation Action (BR-01-13:
  Super Admin only, no self-registration).

**Model** `App\Models\Trainer`: `$fillable = ['business_name', 'slug',
'organization_address', 'organization_website', 'organization_description',
'timezone']`; `belongsTo(Account::class, 'owner_account_id')` as `owner()`;
`hasOne(TrainerBillingSetting::class)`, `hasOne(TrainerBrandingSetting::class)`;
`hasMany` to every trainer-scoped model as needed by callers (not
enumerated here — mechanical).

### `trainer_branding_settings` — was `trainer_branding_settings`

Public by nature — rendered on unauthenticated camp forms (BR-08-6).
`requirements-analyst-epic-01-user-management-spec.md` AC-01-60/61/63.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `trainer_id` | BIGINT | NO | | **PK, and FK** to `trainers(id)` ON DELETE CASCADE |
| `logo_path` | VARCHAR(2048) | YES | NULL | AC-01-60 |
| `primary_color_hex` | CHAR(7) | YES | NULL | AC-01-61 |
| `updated_at` | TIMESTAMPTZ | NO | | AC-01-62: applied immediately, nothing cached |

Check: `primary_color_hex IS NULL OR primary_color_hex ~ '^#[0-9A-Fa-f]{6}$'`
(raw `DB::statement`). Same shared-PK pattern as `account_profiles`. Model
class named `TrainerBrandingSetting` (singular) per this document's naming
convention. No RLS.

### `account_trainer_links` — was `account_trainer_link`

**The tenant resolver's own source** — must be global; the resolver reads
it before a tenant exists.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` RESTRICT |
| `role_in_tenant` | VARCHAR(16) | NO | | `trainer`\|`coach`\|`player` — denormalized off `accounts.role` so the resolver's hot-path query never joins `accounts` |
| `status` | VARCHAR(16) | NO | `'active'` | `active`\|`inactive` |
| `linked_at` | TIMESTAMPTZ | NO | `now()` | |

- Unique: `(account_id, trainer_id)`. Indexes: `(account_id)` —
  trainer-switcher listing (AC-01-15, AC-01-32); `(trainer_id)` —
  Administration cross-trainer listings via `CrossTenantReadService`.
- **Written only by the same Action/Service that creates or transitions a
  `CoachMembership`/`PlayerTrainerMembership` row**, in the same DB
  transaction — never directly from a controller. This dual-write is real
  coupling worth an integration test, exactly as the source notes.
- **No RLS, and none must ever be added** — see "Row-Level Security".
  `App\Models\Concerns\BelongsToTenant` is never applied to this model.

```php
Schema::create('account_trainer_links', function (Blueprint $table): void {
    $table->id();
    $table->foreignId('account_id')->constrained('accounts')->restrictOnDelete();
    $table->foreignId('trainer_id')->constrained('trainers')->restrictOnDelete();
    $table->enum('role_in_tenant', ['trainer', 'coach', 'player']);
    $table->enum('status', ['active', 'inactive'])->default('active');
    $table->timestampTz('linked_at')->useCurrent();

    $table->unique(['account_id', 'trainer_id']);
    $table->index('trainer_id');
});
```

**Model** `App\Models\AccountTrainerLink` — deliberately does **not** use
`BelongsToTenant`:

```php
final class AccountTrainerLink extends Model
{
    protected $table = 'account_trainer_links';
    public $timestamps = false;

    protected $fillable = ['account_id', 'trainer_id', 'role_in_tenant', 'status', 'linked_at'];

    protected function casts(): array
    {
        return [
            'role_in_tenant' => TenantRole::class,
            'status' => LinkStatus::class,
            'linked_at' => 'datetime',
        ];
    }

    public function account(): BelongsTo
    {
        return $this->belongsTo(Account::class);
    }

    public function trainer(): BelongsTo
    {
        return $this->belongsTo(Trainer::class);
    }
}
```

### `public_tenant_codes` — was `public_tenant_code`

**The second half of pre-tenant resolution.** Where `account_trainer_links`
resolves an *authenticated* account, this resolves an *anonymous,
code-bearing* request. Added per `specs/council-sharelink-tenant-resolution.md`
"Recommendation" (option G) — the council's own note that this table's
wiring names Symfony mechanisms requiring Laravel translation is exactly
what this section does.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `code` | VARCHAR(64) | NO | | Same value as the owning `share_links.code` or `forms.shareable_slug` |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` RESTRICT |
| `kind` | VARCHAR(16) | NO | | `sharelink`\|`form` |
| `reference_id` | BIGINT | NO | | **Opaque scalar, deliberately not a real FK** — see note |
| `created_at` | TIMESTAMPTZ | NO | | |

- Unique: `code` — globally.
- Index: `(kind, reference_id)` — revocation lookup.
- **`reference_id` carries no FK on purpose**: a real FK would point at
  either `share_links(id)` or `forms(id)` depending on `kind` — the same
  "too many possible targets for one column" shape as
  `audit_log_entries.subject_id`. A real FK would also give this table a
  dependency on whichever module owns `ShareLink`/`Form`, inverting the
  intended "Platform depends on nothing" direction.
- **No RLS, ever.** See "Row-Level Security" — a policy here breaks every
  login and every public-code route.
- **Single writer**: `PublicTenantCodeRegistry::issue()`/`::revoke()`.
  `ShareLink` creation and `Form` publication each call `issue()` inside the
  same DB transaction that persists their own row.

```php
Schema::create('public_tenant_codes', function (Blueprint $table): void {
    $table->id();
    $table->string('code', 64)->unique();
    $table->foreignId('trainer_id')->constrained('trainers')->restrictOnDelete();
    $table->enum('kind', ['sharelink', 'form']);
    $table->unsignedBigInteger('reference_id'); // deliberately not a foreignId() — see note
    $table->timestampTz('created_at')->useCurrent();

    $table->index(['kind', 'reference_id']);
});
```

**`PublicTenantCodeRegistry`** (`app/Support/Tenancy/PublicTenantCodeRegistry.php`),
the single writer, translating the source's `Platform` service:

```php
final class PublicTenantCodeRegistry
{
    public function issue(string $code, int $trainerId, string $kind, int $referenceId): void
    {
        PublicTenantCode::query()->create([
            'code' => $code,
            'trainer_id' => $trainerId,
            'kind' => $kind,
            'reference_id' => $referenceId,
        ]);
    }

    public function revoke(string $code): void
    {
        PublicTenantCode::query()->where('code', $code)->delete();
    }
}
```

`PublicCodeTenantResolver` (used by `ResolveTenant`, above) is then just:
`PublicTenantCode::query()->where('code', $request->route('code'))->value('trainer_id')`
— the one query that runs with no tenant set, matching the source's
equivalent lookup method on its `PublicTenantCode` data-access class.

### `audit_log_entries` — was `audit_log_entry`

Append-only, spans tenants by design.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `occurred_at` | TIMESTAMPTZ | NO | `now()` | AC-07-31 |
| `actor_account_id` | BIGINT | YES | NULL | FK → `accounts(id)` RESTRICT. Nullable only for system-initiated entries |
| `action_type` | VARCHAR(64) | NO | | e.g. `impersonation_start`, `user_deleted`, `feature_toggled` — AC-07-30 |
| `subject_type` | VARCHAR(64) | NO | | e.g. `account`, `trainer`, `event` — deliberately **not** a real FK |
| `subject_id` | BIGINT | YES | NULL | Soft reference paired with `subject_type` |
| `related_trainer_id` | BIGINT | YES | NULL | FK → `trainers(id)` RESTRICT. Real FK, populated whenever one trainer is concerned |
| `details` | JSONB | NO | `'{}'` | Action-specific payload |
| `created_at` | TIMESTAMPTZ | NO | `now()` | |

- Check: `char_length(action_type) > 0`.
- Indexes: `(occurred_at DESC)`; `(related_trainer_id, occurred_at DESC)
  WHERE related_trainer_id IS NOT NULL`; `(action_type)`; `(actor_account_id)`.
- **`subject_id` stays a soft, untyped reference** — audit subjects span
  nearly every model in the system; a nullable FK per possible subject type
  would be dozens of always-null columns. Contrast `child_approval_requests`
  below, which has exactly three possible subject shapes and gets real FKs.
- **Append-only by database privilege, not convention**:

```php
Schema::create('audit_log_entries', function (Blueprint $table): void {
    $table->id();
    $table->timestampTz('occurred_at')->useCurrent();
    $table->foreignId('actor_account_id')->nullable()->constrained('accounts')->restrictOnDelete();
    $table->string('action_type', 64);
    $table->string('subject_type', 64);
    $table->unsignedBigInteger('subject_id')->nullable();
    $table->foreignId('related_trainer_id')->nullable()->constrained('trainers')->restrictOnDelete();
    $table->jsonb('details')->default('{}');
    $table->timestampTz('created_at')->useCurrent();

    $table->index([DB::raw('occurred_at DESC')]);
    $table->index('action_type');
    $table->index('actor_account_id');
});

DB::statement('CREATE INDEX audit_log_entries_trainer_occurred_idx ON audit_log_entries (related_trainer_id, occurred_at DESC) WHERE related_trainer_id IS NOT NULL');
DB::statement("ALTER TABLE audit_log_entries ADD CONSTRAINT audit_log_entries_action_type_check CHECK (char_length(action_type) > 0)");

// Matches I7's mechanism on token_entries — a database privilege, not a
// Model convention that "a setter survives exactly until someone adds one."
DB::statement('REVOKE UPDATE, DELETE ON audit_log_entries FROM pp_app');
```

**Model** `App\Models\AuditLogEntry`: `public $timestamps = false;`
(`created_at` only, no `updated_at` — the REVOKE makes an update impossible
regardless); `$fillable = ['actor_account_id', 'action_type', 'subject_type',
'subject_id', 'related_trainer_id', 'details']`; cast `'details' => 'array'`;
`belongsTo(Account::class, 'actor_account_id')` as `actor()`;
`belongsTo(Trainer::class, 'related_trainer_id')`. Every row written through
a single `AuditLogger` service class — the sole writer, matching the
source's "Cross-cutting services" convention. No RLS.

### `impersonation_sessions` — was `impersonation_session`

BR-01-21/22.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `admin_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |
| `target_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |
| `started_at` | TIMESTAMPTZ | NO | `now()` | |
| `ended_at` | TIMESTAMPTZ | YES | NULL | |
| `ended_reason` | VARCHAR(16) | YES | NULL | `manual`\|`expired` |

- Check: `admin_account_id <> target_account_id` (raw `DB::statement`).
  "Cannot target another Super Admin" (BR-01-21) needs `target_account_id`'s
  role — a cross-table lookup no CHECK can express — so it stays an
  application guard in the impersonation Action, covered by a test.
- Indexes: `(target_account_id)`; partial `(admin_account_id) WHERE ended_at
  IS NULL` — "is this admin already impersonating someone", the 1-hour
  auto-expiry sweep, and `ImpersonationTenantResolver`'s lookup (source 2)
  all hit this partial index.
- "Expires automatically after 1 hour" (BR-01-22) is **derived at read
  time** (`started_at + 1h < now()` and `ended_at IS NULL`) — an accessor on
  the model, not a scheduled job flipping a column, matching the source's
  general "time-based state is derived at read time" rule.

**Model**: `$fillable = ['admin_account_id', 'target_account_id']`; casts
`'ended_reason' => ImpersonationEndedReason::class`; an `isActive(): bool`
accessor computing the 1-hour rule above; two `belongsTo(Account::class)`.
No RLS.

### `platform_configurations` — was `platform_configuration`

Small, rarely-written, frequently-read singleton settings. BR-06-4; Section
B defaults for Q-05.03, Q-06.11, Q-06.12.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `key` | VARCHAR(100) | NO | | |
| `value` | JSONB | NO | | |
| `updated_at` | TIMESTAMPTZ | NO | | |
| `updated_by_account_id` | BIGINT | YES | NULL | FK → `accounts(id)` RESTRICT |

Unique: `key`. **Model**: `$fillable = ['key', 'value', 'updated_by_account_id']`;
cast `'value' => 'array'`; a static `PlatformConfiguration::valueFor(string
$key): mixed` helper. Seeded rows (application data, not schema — see
Seeders): `default_platform_fee_basis_points` = 500 (BR-05-7);
`default_platform_subscription_price_minor_units` = 1500 (BR-05-13);
`referral_reward_ratio` = `{"referrals":1,"tokens":1}` (Q-06.11 default);
`referral_attribution_window_days` = 30 (Q-06.12 default);
`referee_welcome_token` = `false`; `token_package_seed` = the epic's
illustrative packages (Q-05.03 default). No RLS.

### `feature_toggles` — was `feature_toggle`

Platform configuration *about* a trainer, but itself global. BR-07-1/2/3.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` RESTRICT — a reference, not a tenancy key; no RLS |
| `feature_name` | VARCHAR(16) | NO | | `lppp`\|`marketing`\|`camps` |
| `is_enabled` | BOOLEAN | NO | `true` | BR-07-2: defaults enabled |
| `updated_at` | TIMESTAMPTZ | NO | | |
| `updated_by_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |

Unique: `(trainer_id, feature_name)`. Index: `(trainer_id)`. Seeded with all
three rows `is_enabled = true` at trainer creation (part of the
trainer-creation Action, not a migration). **Model**: `$fillable =
['trainer_id', 'feature_name', 'is_enabled', 'updated_by_account_id']`; cast
`'feature_name' => FeatureName::class`, `'is_enabled' => 'boolean'`. No RLS
— `trainer_id` here is a plain reference column, deliberately not paired
with `BelongsToTenant`.

### `platform_subscriptions` — was `platform_subscription`

The platform's own billing relationship with a trainer. Global so Epic-07's
"active trainers" count needs no crossing read.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` RESTRICT, UNIQUE |
| `stripe_subscription_id` | VARCHAR(255) | YES | NULL | |
| `status` | VARCHAR(16) | NO | `'pending'` | `pending`\|`active`\|`past_due`\|`suspended`\|`canceled` |
| `updated_at` | TIMESTAMPTZ | NO | | |

Unique: `trainer_id`, `stripe_subscription_id`. Index: `(status)` —
BR-07-7's "active trainers" dashboard count reads this column directly.
**No revenue/payout figure stored anywhere on this table** — same earnings
boundary as `payment_records` below. **Model**: `$fillable = ['trainer_id',
'stripe_subscription_id', 'status']`; cast `'status' =>
PlatformSubscriptionStatus::class`. No RLS.

### `stripe_customer_links` — was `stripe_customer_link`

One per account, shared across all of that account's trainers. BR-05-16.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT, UNIQUE |
| `stripe_customer_id` | VARCHAR(255) | NO | | |
| `default_payment_method_ref` | VARCHAR(255) | YES | NULL | Informational only — Stripe is the source of truth (AC-05-21) |
| `created_at` | TIMESTAMPTZ | NO | | |

Unique: `account_id`, `stripe_customer_id`. **Model**: `belongsTo(Account::class)`.
No RLS.

### `stripe_event_receipts` — was `stripe_event_receipt`

Inbound webhook idempotency — "the unique violation *is* the duplicate
detection." BR-05-6, AC-05-34/35.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `stripe_event_id` | VARCHAR(255) | NO | | |
| `event_type` | VARCHAR(100) | NO | | e.g. `payment_intent.succeeded` |
| `received_at` | TIMESTAMPTZ | NO | `now()` | |
| `processed_at` | TIMESTAMPTZ | YES | NULL | |
| `processing_error` | TEXT | YES | NULL | |

- Unique: `stripe_event_id`. Inserted **before** the webhook body is parsed
  — the handler catches the unique-violation (Postgres SQLSTATE `23505`,
  surfaced by Laravel as `Illuminate\Database\UniqueConstraintViolationException`
  since Laravel 11) and returns 2xx immediately for a replay.
- Index: `(processed_at) WHERE processed_at IS NULL` — retry sweep,
  AC-05-35's exponential backoff, run as a scheduled Artisan command
  (`App\Console\Commands\RetryFailedStripeWebhooks`, `Schedule::command(...)`
  in `routes/console.php`) — the Laravel equivalent of the source's
  scheduler-driven sweep.

**Model**: `$fillable = ['stripe_event_id', 'event_type']`; a local scope
`scopeUnprocessed()`. No RLS.

---

## Trainer-scoped tables

Every model below `use`s `App\Models\Concerns\BelongsToTenant`, carries a
`trainer_id` column, and gets the standard RLS policy (full SQL in "Row-Level
Security") unless the table's own section states an exception. Ported from
`specs/database-designer-schema.md` "Trainer-scoped tables" (line 617
onward), organized by the same modules.

### Identity module

#### `trainer_billing_settings`

Split from `trainers` for exactly one reason: sensitive, never cross-tenant.
BR-05-1, AC-05-1/2/27.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `trainer_id` | BIGINT | NO | | **PK, FK, and tenant key** to `trainers(id)` ON DELETE RESTRICT |
| `stripe_connect_account_id` | VARCHAR(255) | YES | NULL | UNIQUE |
| `stripe_connect_onboarding_status` | VARCHAR(16) | NO | `'pending'` | `pending`\|`complete`\|`incomplete` |
| `platform_fee_basis_points` | SMALLINT | NO | `500` | 5% default, Super-Admin-editable per trainer |
| `monthly_subscription_price_minor_units` | INTEGER | NO | `1500` | |
| `token_price_minor_units` | INTEGER | NO | `1000` | |
| `player_subscription_price_minor_units` | INTEGER | YES | NULL | |
| `payout_schedule` | VARCHAR(16) | NO | `'monthly'` | `monthly`\|`weekly` |
| `updated_at` | TIMESTAMPTZ | NO | | |

- Same shared-PK pattern as `account_profiles`, except here the shared PK
  **is also the tenant key** — `trainer_id` is simultaneously `$primaryKey`
  and the column `BelongsToTenant`'s global scope filters on. `TenantScope`
  is written generically enough (`$model->qualifyColumn('trainer_id')`) that
  this needs no special case.
- Check: `platform_fee_basis_points BETWEEN 0 AND 10000`.
- RLS: standard policy.

**Model** `App\Models\TrainerBillingSetting`: `protected $primaryKey =
'trainer_id'; public $incrementing = false;`; `use BelongsToTenant;`
`$fillable = ['platform_fee_basis_points', 'monthly_subscription_price_minor_units',
'token_price_minor_units', 'player_subscription_price_minor_units', 'payout_schedule']`
(Stripe columns excluded — written only by the Stripe Connect onboarding
flow, not mass-assigned); casts: `'stripe_connect_onboarding_status' =>
StripeConnectOnboardingStatus::class`, `'payout_schedule' =>
PayoutSchedule::class`; `belongsTo(Trainer::class, 'trainer_id')`.

#### `coach_memberships` — was `coach_membership`

BR-01-11, BR-01-15, AC-01-39/40/41.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |
| `status` | VARCHAR(16) | NO | `'pending'` | `pending`\|`active`\|`declined` |
| `bio` / `credentials` / `certifications` | TEXT | YES | NULL | |
| `is_public_profile` | BOOLEAN | NO | `false` | AC-01-51 |
| `invited_via_share_link_id` | BIGINT | YES | NULL | FK → `share_links(id)` RESTRICT |
| `joined_at` | TIMESTAMPTZ | YES | NULL | Set on acceptance |
| `created_at` | TIMESTAMPTZ | NO | | |

- **Unique partial index: `(account_id) WHERE status = 'active'`, with no
  `trainer_id` in the key.** This is the mechanism enforcing BR-01-11 ("a
  coach cannot be active under multiple trainers simultaneously") *across*
  tenants from inside an RLS-protected table. It works because PostgreSQL
  unique-index enforcement is **not** filtered by RLS: a session scoped to
  Trainer A's tenant still gets a unique-violation activating a coach
  already `active` under Trainer B, even though that row is invisible to a
  normal `SELECT` in A's session. This is a deliberate, documented use of a
  known RLS property (sometimes called a "covert channel," because the
  error itself leaks the row's existence), unchanged from the source.
- Indexes: `(trainer_id, status)` — Coaches list (AC-01-40); `(account_id)`.

```php
Schema::create('coach_memberships', function (Blueprint $table): void {
    $table->id();
    $table->foreignId('trainer_id')->constrained('trainers');
    $table->foreignId('account_id')->constrained('accounts')->restrictOnDelete();
    $table->enum('status', ['pending', 'active', 'declined'])->default('pending');
    $table->text('bio')->nullable();
    $table->text('credentials')->nullable();
    $table->text('certifications')->nullable();
    $table->boolean('is_public_profile')->default(false);
    $table->foreignId('invited_via_share_link_id')->nullable()->constrained('share_links')->restrictOnDelete();
    $table->timestampTz('joined_at')->nullable();
    $table->timestampTz('created_at')->useCurrent();

    $table->index(['trainer_id', 'status']);
});

// Cross-tenant-safe by construction — see the note above. Do not add
// trainer_id to this index; that would silently defeat BR-01-11.
DB::statement('CREATE UNIQUE INDEX coach_memberships_one_active_per_account ON coach_memberships (account_id) WHERE status = \'active\'');
```

**Model** `App\Models\CoachMembership`: `use BelongsToTenant;` `$fillable =
['account_id', 'bio', 'credentials', 'certifications', 'is_public_profile',
'invited_via_share_link_id']`; cast `'status' => CoachMembershipStatus::class`;
`belongsTo(Account::class)`, `belongsTo(ShareLink::class, 'invited_via_share_link_id')`,
`hasMany(AvailabilityWindow::class, 'coach_membership_id')`,
`hasMany(CoachAssignment::class)`. RLS: standard policy.

#### `player_trainer_memberships` — was `player_trainer_membership`

The multi-trainer join, with a mandatory association source. BR-01-10/12;
`MembershipService` (the sole creator) is ported as an Action class,
`App\Actions\Identity\CreatePlayerTrainerMembership`.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `player_id` | BIGINT | NO | | FK → `player_profiles(id)` RESTRICT |
| `source` | VARCHAR(20) | NO | | `sharelink`\|`event_registration`\|`coach_invite`\|`camp_registration` — BR-03-1, A3 |
| `share_link_id` | BIGINT | YES | NULL | FK → `share_links(id)` RESTRICT |
| `status` | VARCHAR(16) | NO | `'active'` | `active`\|`removed` |
| `skill_level` | VARCHAR(50) | YES | NULL | Trainer-set free text — see Open questions (Q-01.01) |
| `joined_at` | TIMESTAMPTZ | NO | `now()` | |
| `removed_at` | TIMESTAMPTZ | YES | NULL | AC-01-24: soft-removal, history preserved |

- Unique: `(trainer_id, player_id)` — re-joining reactivates the same row
  rather than inserting a duplicate.
- Check: `source IN (...)`; `(source = 'sharelink') = (share_link_id IS NOT NULL)`
  (raw `DB::statement`).
- Indexes: `(player_id)`; `(trainer_id, status)` — CRM roster (BR-03-2);
  `(trainer_id, skill_level)` — segmentation filter (BR-03-13).

**Model** `App\Models\PlayerTrainerMembership`: `use BelongsToTenant;`
`$fillable = ['player_id', 'source', 'share_link_id', 'skill_level']`; casts
`'source' => MembershipSource::class`, `'status' => MembershipStatus::class`,
`'joined_at' => 'datetime'`, `'removed_at' => 'datetime'`;
`belongsTo(PlayerProfile::class, 'player_id')`, `belongsTo(ShareLink::class)`.
RLS: standard policy.

#### `share_links` — was `share_link`

BR-01-14/15/27.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `code` | VARCHAR(64) | NO | | High-entropy, URL-safe, application-generated |
| `link_type` | VARCHAR(16) | NO | | `static_player`\|`unique_coach` |
| `created_by_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |
| `target_email` | CITEXT | YES | NULL | Coach links only |
| `expires_at` | TIMESTAMPTZ | YES | NULL | Coach links: `created_at + 7d`. Static: NULL |
| `max_uses` | INTEGER | YES | NULL | Coach links: 1. Static: NULL |
| `use_count` | INTEGER | NO | `0` | BR-01-27 |
| `is_active` | BOOLEAN | NO | `true` | |
| `created_at` | TIMESTAMPTZ | NO | | |

- Unique: `code` — **globally**, not per-trainer. The unauthenticated
  cold-click lookup does **not** run against this table (it is
  RLS-protected like every other trainer-scoped table) — it runs against
  `public_tenant_codes`. `ShareLink` creation calls
  `PublicTenantCodeRegistry::issue($code, $trainerId, 'sharelink', $id)` in
  the same transaction.
- Check: `link_type IN (...)`; the two mutually-exclusive shape constraints
  tying `link_type` to `target_email`/`expires_at`/`max_uses` (raw
  `DB::statement`, cross-column).
- Indexes: `(trainer_id)`; partial `(expires_at) WHERE link_type =
  'unique_coach' AND is_active` — expiry sweep for AC-01-42's resend prompt.

**Model** `App\Models\ShareLink`: `use BelongsToTenant;` `$fillable =
['code', 'link_type', 'created_by_account_id', 'target_email', 'expires_at',
'max_uses']`; casts `'link_type' => ShareLinkType::class`, `'expires_at' =>
'datetime'`, `'is_active' => 'boolean'`; `belongsTo(Account::class,
'created_by_account_id')`; `hasMany(ShareLinkOpen::class)`. RLS: standard
policy.

#### `share_link_opens` — was `share_link_open`

Click log. BR-03-22.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `share_link_id` | BIGINT | NO | | FK → `share_links(id)` ON DELETE CASCADE |
| `opened_at` | TIMESTAMPTZ | NO | `now()` | |
| `ip_address` | INET | YES | NULL | Optional per BR-03-22 |

Index: `(share_link_id, opened_at)` — Quick View open-count (BR-03-22). This
is the trainer-scoped **write** the council doc's Finding 1b identifies on
the otherwise-anonymous `GET /join/{code}` route — the controller resolves
`trainer_id` via `ResolveTenant`'s public-code source before this insert,
there is no separate unscoped code path. **Model**: `use BelongsToTenant;`
`$fillable = ['share_link_id', 'ip_address']`; `public $timestamps = false;`
(single `opened_at` moment, not created/updated); `belongsTo(ShareLink::class)`.
RLS: standard policy.

#### `availability_windows` — was `availability_window`

"Best Times" for both coaches and players. BR-01-25; kept trainer-scoped,
not global — isolation over convenience.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `owner_type` | VARCHAR(16) | NO | | `coach`\|`player` |
| `coach_membership_id` | BIGINT | YES | NULL | FK → `coach_memberships(id)` ON DELETE CASCADE |
| `player_id` | BIGINT | YES | NULL | FK → `player_profiles(id)` ON DELETE CASCADE |
| `day_of_week` | SMALLINT | NO | | 0–6 |
| `start_time` / `end_time` | TIME | NO | | |
| `is_available` | BOOLEAN | NO | `true` | |

- Check: `owner_type IN (...)`; the two mutually-exclusive owner-column
  shape constraints; `day_of_week BETWEEN 0 AND 6`; `end_time > start_time`
  (all raw `DB::statement`).
- Indexes: `(coach_membership_id, day_of_week)`; `(player_id, day_of_week)`.
- No overlap-prevention constraint — "multiple time slots allowed per day"
  (AC-01-46) means overlaps are legitimate input, not an error.

**Model**: `use BelongsToTenant;` `$fillable = ['owner_type',
'coach_membership_id', 'player_id', 'day_of_week', 'start_time', 'end_time',
'is_available']`; cast `'owner_type' => AvailabilityOwnerType::class`;
`belongsTo(CoachMembership::class)`, `belongsTo(PlayerProfile::class)`. RLS:
standard policy.

#### `child_approval_requests` — was `child_approval_request`

Covers RSVPs and purchases alike. BR-01-18/19/20.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `child_player_id` | BIGINT | NO | | FK → `player_profiles(id)` RESTRICT |
| `parent_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |
| `action_type` | VARCHAR(20) | NO | | `rsvp`\|`token_purchase`\|`content_purchase` |
| `rsvp_id` | BIGINT | YES | NULL | FK → `rsvps(id)` ON DELETE CASCADE. Set when `action_type = 'rsvp'` |
| `requested_token_package_id` | BIGINT | YES | NULL | **Deferred FK** → `token_packages(id)` RESTRICT — target table doesn't exist until the Billing migration stage |
| `requested_playlist_id` | BIGINT | YES | NULL | **Deferred FK** → `playlists(id)` RESTRICT — target doesn't exist until the Content migration stage |
| `status` | VARCHAR(16) | NO | `'pending'` | `pending`\|`approved`\|`denied` — **no `expired` value; see note** |
| `requested_at` | TIMESTAMPTZ | NO | `now()` | |
| `responded_at` | TIMESTAMPTZ | YES | NULL | |
| `parent_note` | TEXT | YES | NULL | BR-01-20 |

- Check: `action_type IN (...)`; exactly one of the three request-target
  columns is non-null, matching `action_type`; `status IN (...)` (all raw
  `DB::statement`).
- Indexes: `(parent_account_id, status)` — parent's pending-approvals inbox
  (AC-01-26); `(child_player_id)`.
- **`status` deliberately has no `expired` value.** BR-01-18's 48-hour
  auto-expiry is derived at read time — `status = 'pending' AND
  requested_at + interval '48 hours' < now()` — computed as a model
  accessor (`isExpired(): bool`), not materialized. A scheduled Artisan
  command sends the auto-denial *notification* at that point without
  flipping the column, mirroring the source's Scheduler/worker split.
- These three request-target columns are exactly why `rsvps`, `token_packages`
  and `playlists` must exist before their FKs are fully attached — see
  "Migration ordering", deferred-FK pattern (introduced here, reused
  throughout — every later occurrence points back to this paragraph).

```php
// Creating migration (Identity stage) — nullable plain columns, no FK yet
// for the two Content/Billing targets.
Schema::create('child_approval_requests', function (Blueprint $table): void {
    $table->id();
    $table->foreignId('trainer_id')->constrained('trainers');
    $table->foreignId('child_player_id')->constrained('player_profiles')->restrictOnDelete();
    $table->foreignId('parent_account_id')->constrained('accounts')->restrictOnDelete();
    $table->enum('action_type', ['rsvp', 'token_purchase', 'content_purchase']);
    $table->foreignId('rsvp_id')->nullable()->constrained('rsvps')->cascadeOnDelete(); // attached in the Scheduling-stage migration — rsvps exists by then
    $table->unsignedBigInteger('requested_token_package_id')->nullable(); // deferred FK — attached in the Billing stage
    $table->unsignedBigInteger('requested_playlist_id')->nullable();      // deferred FK — attached in the Content stage
    $table->enum('status', ['pending', 'approved', 'denied'])->default('pending');
    $table->timestampTz('requested_at')->useCurrent();
    $table->timestampTz('responded_at')->nullable();
    $table->text('parent_note')->nullable();

    $table->index(['parent_account_id', 'status']);
    $table->index('child_player_id');
});
```

**Model**: `use BelongsToTenant;` `$fillable = ['child_player_id',
'parent_account_id', 'action_type', 'rsvp_id', 'requested_token_package_id',
'requested_playlist_id', 'parent_note']`; casts `'action_type' =>
ChildApprovalActionType::class`, `'status' => ChildApprovalStatus::class`;
`belongsTo` to `PlayerProfile`, `Account`, `Rsvp`, `TokenPackage`, `Playlist`;
accessor `isExpired(): bool`. RLS: standard policy.

### Scheduling module (Epic-02)

#### `events` — was `event`

BR-02-1 through BR-02-6, BR-02-19/20; AC-02-58 through AC-02-65.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `title` | VARCHAR(100) | NO | | BR-02-1 |
| `description` | TEXT | YES | NULL | |
| `event_type` | VARCHAR(24) | NO | | `training_session`\|`private_session`\|`small_group` |
| `starts_at` / `ends_at` | TIMESTAMPTZ | NO | | |
| `location` | VARCHAR(255) | NO | | |
| `capacity` | SMALLINT | NO | | 1–999 — BR-02-4 |
| `visibility` | VARCHAR(16) | NO | `'public'` | `public`\|`private` |
| `usd_pricing_enabled` | BOOLEAN | NO | `false` | AC-05-30: default off |
| `usd_price_minor_units` | INTEGER | NO | `0` | |
| `token_pricing_enabled` | BOOLEAN | NO | `true` | AC-05-30: default on |
| `token_price` | INTEGER | NO | `1` | AC-05-32: trainer-configurable |
| `min_age` / `max_age` | SMALLINT | YES | NULL | Eligibility — BR-02-5 |
| `skill_levels` / `genders` | JSONB | YES | NULL | Was `TEXT[]` — see Conventions |
| `status` | VARCHAR(16) | NO | `'active'` | `active`\|`canceled`\|`completed` |
| `canceled_reason` | TEXT | YES | NULL | |
| `canceled_by_account_id` | BIGINT | YES | NULL | FK → `accounts(id)` RESTRICT |
| `canceled_at` | TIMESTAMPTZ | YES | NULL | |
| `created_at` / `updated_at` | TIMESTAMPTZ | NO | | |

- Check: `event_type IN (...)`; `ends_at > starts_at`; `capacity BETWEEN 1
  AND 999`; `visibility IN (...)`; `status IN (...)`; `usd_price_minor_units
  >= 0`; `token_price >= 1`; `(usd_pricing_enabled = false) OR
  (usd_price_minor_units > 0)` (BR-02-3: a *paid* event's amount must be >
  0); `(status = 'canceled') = (canceled_reason IS NOT NULL AND canceled_at
  IS NOT NULL)` — all raw `DB::statement`.
- Indexes: `(trainer_id, starts_at)` — Training Calendar (AC-02-66);
  `(trainer_id, status, starts_at)` — upcoming/active listings;
  `(trainer_id, event_type)` — Event Master filter (AC-07-24; Super Admin's
  cross-tenant version goes through `CrossTenantReadService`, not this
  index).
- Capacity enforcement: `Event::query()->lockForUpdate()->findOrFail($id)`
  inside `DB::transaction()`, then `Rsvp::query()->where('event_id',
  $id)->where('status', 'confirmed')->count()` — Eloquent's `lockForUpdate()`
  is the direct translation of `SELECT ... FOR UPDATE`, same no-cached-counter
  rule as the source.
- "Bulk auto-creation" (AC-02-61) needs no extra schema: each generated
  occurrence is an independent `Event` row created via
  `Event::factory()->count($n)->create([...])`-shaped application code, no
  `RecurringPattern` model, no linking column — matches the source's own
  reading of BR-02-19.
- The source's `min_age`/`max_age`/`skill_levels`/`genders` and
  `usd_pricing_enabled`/`usd_price_minor_units`/`token_pricing_enabled`/
  `token_price` value-object groupings become two Eloquent custom casts,
  `EligibilityCriteria` and `DualPricing`, each combining sibling columns
  the same way `EmergencyContact` does above:

```php
final class DualPricing implements CastsAttributes
{
    public function get(Model $model, string $key, mixed $value, array $attributes): DualPricingValue
    {
        return new DualPricingValue(
            usdEnabled: $attributes['usd_pricing_enabled'],
            usdPriceMinorUnits: $attributes['usd_price_minor_units'],
            tokenEnabled: $attributes['token_pricing_enabled'],
            tokenPrice: $attributes['token_price'],
        );
    }

    public function set(Model $model, string $key, mixed $value, array $attributes): array
    {
        return [
            'usd_pricing_enabled' => $value->usdEnabled,
            'usd_price_minor_units' => $value->usdPriceMinorUnits,
            'token_pricing_enabled' => $value->tokenEnabled,
            'token_price' => $value->tokenPrice,
        ];
    }
}
```

`DualPricingValue::priceFor(PaymentMethod $method): Money|int` gives the
RSVP Action one call site instead of branching on raw columns everywhere,
matching the source's stated intent for this embeddable exactly.

**Model** `App\Models\Event`: `use BelongsToTenant;` `$fillable = ['title',
'description', 'event_type', 'starts_at', 'ends_at', 'location', 'capacity',
'visibility', 'min_age', 'max_age', 'skill_levels', 'genders']` (pricing
columns excluded from mass assignment — set via `DualPricing` explicitly);
casts: `'event_type' => EventType::class`, `'visibility' =>
EventVisibility::class`, `'status' => EventStatus::class`, `'starts_at' =>
'datetime'`, `'ends_at' => 'datetime'`, `'skill_levels' => 'array'`,
`'genders' => 'array'`, `'eligibility' => EligibilityCriteria::class`,
`'pricing' => DualPricing::class`; `hasMany(Rsvp::class)`,
`hasMany(AttendanceRecord::class)`, `hasMany(CoachAssignment::class)`,
`hasMany(EventInvitation::class)`. RLS: standard policy.

#### `rsvps` — was `rsvp`

BR-02-7 through BR-02-11.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `event_id` | BIGINT | NO | | FK → `events(id)` RESTRICT |
| `player_id` | BIGINT | NO | | FK → `player_profiles(id)` RESTRICT |
| `status` | VARCHAR(24) | NO | `'pending_payment'` | `pending_parent_approval`\|`pending_payment`\|`confirmed`\|`canceled` — see note |
| `payment_method` | VARCHAR(8) | NO | | `token`\|`usd`\|`free` |
| `payment_record_id` | BIGINT | YES | NULL | **Deferred FK** → `payment_records(id)` — attached in the Billing stage |
| `requested_at` | TIMESTAMPTZ | NO | `now()` | |
| `confirmed_at` / `canceled_at` | TIMESTAMPTZ | YES | NULL | |
| `cancellation_reason` | TEXT | YES | NULL | |

- Check: `status IN (...)`; `payment_method IN (...)`.
- Unique: `(event_id, player_id)` — BR-02-7, one RSVP per player per event.
- Indexes: `(event_id, status)` — capacity count and roster; `(player_id,
  status)` — player's upcoming RSVPs, and the 1-advance-booking-per-day
  check (joined through `entitlement_coverages`).
- **`status` intentionally excludes `attended`/`no_show`** even though the
  epic's own Data Requirements lists them as RSVP statuses — the richer,
  dedicated `AttendanceRecord` model (4-state, BR-02-17) is kept as the one
  source of truth for that fact instead of letting two vocabularies
  disagree. Recorded in Decisions.
- Free events: `payment_method = 'free'`, `payment_record_id` stays `NULL`,
  `status` goes straight to `confirmed`.

```php
Schema::create('rsvps', function (Blueprint $table): void {
    $table->id();
    $table->foreignId('trainer_id')->constrained('trainers');
    $table->foreignId('event_id')->constrained('events')->restrictOnDelete();
    $table->foreignId('player_id')->constrained('player_profiles')->restrictOnDelete();
    $table->enum('status', ['pending_parent_approval', 'pending_payment', 'confirmed', 'canceled'])->default('pending_payment');
    $table->enum('payment_method', ['token', 'usd', 'free']);
    $table->unsignedBigInteger('payment_record_id')->nullable(); // deferred FK — attached in the Billing stage
    $table->timestampTz('requested_at')->useCurrent();
    $table->timestampTz('confirmed_at')->nullable();
    $table->timestampTz('canceled_at')->nullable();
    $table->text('cancellation_reason')->nullable();

    $table->unique(['event_id', 'player_id']);
    $table->index(['event_id', 'status']);
    $table->index(['player_id', 'status']);
});
```

**Model**: `use BelongsToTenant;` `$fillable = ['event_id', 'player_id',
'payment_method']`; casts `'status' => RsvpStatus::class`, `'payment_method'
=> RsvpPaymentMethod::class`; `belongsTo(Event::class)`,
`belongsTo(PlayerProfile::class)`, `belongsTo(PaymentRecord::class)`,
`hasOne(ChildApprovalRequest::class)`, `hasOne(EntitlementCoverage::class)`.
RLS: standard policy.

#### `attendance_records` — was `attendance_record`

BR-02-16/17.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `event_id` | BIGINT | NO | | FK → `events(id)` RESTRICT |
| `player_id` | BIGINT | NO | | FK → `player_profiles(id)` RESTRICT |
| `rsvp_id` | BIGINT | NO | | FK → `rsvps(id)` RESTRICT |
| `status` | VARCHAR(16) | NO | | `present`\|`absent`\|`late`\|`excused` |
| `recorded_by_coach_membership_id` | BIGINT | NO | | FK → `coach_memberships(id)` RESTRICT |
| `recorded_at` | TIMESTAMPTZ | NO | `now()` | Only after event start — BR-02-16 |

- Check: `status IN (...)`. Unique: `(event_id, player_id)`.
- Index: `(trainer_id, player_id, status, recorded_at)` — the index the CRM
  segmentation and Top Players queries (BR-03-13/14/16) run against, using
  `toBase()` query-builder projections (see "Read-heavy screens" below),
  since RLS covers raw/query-builder SQL exactly as it covers Eloquent's.

**Model**: `use BelongsToTenant;` `$fillable = ['event_id', 'player_id',
'rsvp_id', 'recorded_by_coach_membership_id']`; cast `'status' =>
AttendanceStatus::class`; `belongsTo` to `Event`, `PlayerProfile`, `Rsvp`,
`CoachMembership`; `hasMany(AttendanceEdit::class)`. RLS: standard policy.

#### `attendance_edits` — was `attendance_edit`

History for edits inside the same-day coach edit window. BR-02-18.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `attendance_record_id` | BIGINT | NO | | FK → `attendance_records(id)` ON DELETE CASCADE |
| `old_status` / `new_status` | VARCHAR(16) | NO | | |
| `edited_by_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |
| `edited_at` | TIMESTAMPTZ | NO | `now()` | |

Check: both status columns `IN (...)`. Index: `(attendance_record_id,
edited_at)`. Ordinary history table — same-day coach edit permission
(BR-02-18) is a Form Request/Policy check comparing `attendance_record.recorded_at`'s
date to today in the trainer's timezone, not a DB constraint (the rule
changes with the clock). **Model**: `use BelongsToTenant;` `$fillable =
['attendance_record_id', 'old_status', 'new_status', 'edited_by_account_id']`;
`public $timestamps = false;` (`edited_at` only); casts on both status
columns. RLS: standard policy.

#### `coach_assignments` — was `coach_assignment`

BR-02-13/14/15.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `event_id` | BIGINT | NO | | FK → `events(id)` RESTRICT |
| `coach_membership_id` | BIGINT | NO | | FK → `coach_memberships(id)` RESTRICT |
| `status` | VARCHAR(16) | NO | `'pending'` | `pending`\|`confirmed`\|`declined` |
| `decline_reason` | TEXT | YES | NULL | |
| `confirmed_at` | TIMESTAMPTZ | YES | NULL | |
| `assigned_at` | TIMESTAMPTZ | NO | `now()` | |

Check: `status IN (...)`. Unique: `(event_id, coach_membership_id)`. Index:
`(coach_membership_id, status)` — coach's pending assignments. **Model**:
`use BelongsToTenant;` `$fillable = ['event_id', 'coach_membership_id',
'decline_reason']`; cast `'status' => CoachAssignmentStatus::class`;
`belongsTo(Event::class)`, `belongsTo(CoachMembership::class)`. RLS:
standard policy.

#### `coach_availability_overrides` — was `coach_availability_override`

Kept independent of `coach_assignments` on purpose — a re-assignment does
not erase the audited override. BR-01-26.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `event_id` | BIGINT | NO | | FK → `events(id)` RESTRICT |
| `coach_membership_id` | BIGINT | NO | | FK → `coach_memberships(id)` RESTRICT |
| `overridden_by_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |
| `reason` | TEXT | NO | | BR-01-26: required |
| `created_at` | TIMESTAMPTZ | NO | `now()` | |

Check: `char_length(reason) > 0`. Index: `(event_id, coach_membership_id)`.
**No FK to `coach_assignments`** — referencing `(event_id,
coach_membership_id)` directly keeps the audited fact intact regardless of
what the *current* assignment says, even after a reassignment. **Model**:
`use BelongsToTenant;` `$fillable = ['event_id', 'coach_membership_id',
'overridden_by_account_id', 'reason']`; `public $timestamps = false;`. RLS:
standard policy.

#### `event_invitations` — was `event_invitation`

Private-event guest list. BR-02-6.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `event_id` | BIGINT | NO | | FK → `events(id)` ON DELETE CASCADE |
| `player_id` | BIGINT | NO | | FK → `player_profiles(id)` RESTRICT |
| `invited_by_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |
| `invited_at` | TIMESTAMPTZ | NO | `now()` | |

Unique: `(event_id, player_id)`. Index: `(player_id)` — private event
visibility check. **Model**: `use BelongsToTenant;` `$fillable = ['event_id',
'player_id', 'invited_by_account_id']`; `public $timestamps = false;`. RLS:
standard policy.

#### `event_duplication_records` — was `event_duplication_record`

BR-02-19.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `original_event_id` | BIGINT | NO | | FK → `events(id)` RESTRICT |
| `new_event_id` | BIGINT | NO | | FK → `events(id)` RESTRICT, UNIQUE |
| `duplicated_by_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |
| `duplicated_at` | TIMESTAMPTZ | NO | `now()` | |

Unique: `new_event_id`. Index: `(original_event_id)`. **Model**: `use
BelongsToTenant;` `$fillable = ['original_event_id', 'new_event_id',
'duplicated_by_account_id']`; `public $timestamps = false;`;
`belongsTo(Event::class, 'original_event_id')`, `belongsTo(Event::class,
'new_event_id')`. RLS: standard policy.

### Crm module (Epic-03)

#### `labels` — was `label`

BR-03-3/4/5.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `name` | VARCHAR(50) | NO | | |
| `name_normalized` | VARCHAR(50) | NO | generated | `storedAs('lower(name)')` |
| `color_hex` | CHAR(7) | NO | | |
| `created_at` | TIMESTAMPTZ | NO | | |

- Unique: `(trainer_id, name_normalized)` — BR-03-3: case-insensitive within
  one trainer ("Elite"/"elite" collide); two different trainers' "Elite"
  labels do not (BR-03-4).
- Check: `color_hex ~ '^#[0-9A-Fa-f]{6}$'`.

```php
Schema::create('labels', function (Blueprint $table): void {
    $table->id();
    $table->foreignId('trainer_id')->constrained('trainers');
    $table->string('name', 50);
    $table->string('name_normalized', 50)->storedAs('lower(name)');
    $table->char('color_hex', 7);
    $table->timestampTz('created_at')->useCurrent();

    $table->unique(['trainer_id', 'name_normalized']);
});
```

**Model**: `use BelongsToTenant;` `$fillable = ['name', 'color_hex']`;
`public $timestamps = false;`; `name_normalized` excluded from `$fillable`
(database-generated); `hasMany(PlayerLabel::class)`. RLS: standard policy.

#### `player_labels` — was `player_label`

BR-03-4/5.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `player_id` | BIGINT | NO | | FK → `player_profiles(id)` RESTRICT |
| `label_id` | BIGINT | NO | | FK → `labels(id)` **ON DELETE CASCADE** |
| `applied_by_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |
| `applied_at` | TIMESTAMPTZ | NO | `now()` | |

- Unique: `(player_id, label_id)`. Index: `(trainer_id, label_id)` —
  segmentation by label (BR-03-13).
- `ON DELETE CASCADE` on `label_id` is deliberate: BR-03-5 states deleting a
  label removes it from every player it was applied to — the cascade *is*
  the rule, not a side effect to guard against — `$table->foreignId('label_id')->constrained('labels')->cascadeOnDelete();`.

**Model**: `use BelongsToTenant;` `$fillable = ['player_id', 'label_id',
'applied_by_account_id']`; `public $timestamps = false;`;
`belongsTo(PlayerProfile::class)`, `belongsTo(Label::class)`. RLS: standard
policy.

#### `player_flags` — was `player_flag`

The 8 fixed system flags. BR-03-6/7/8.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `player_id` | BIGINT | NO | | FK → `player_profiles(id)` RESTRICT |
| `flag_type` | VARCHAR(24) | NO | | 8-value closed set — see check |
| `applied_by_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |
| `applied_at` | TIMESTAMPTZ | NO | `now()` | |
| `note` | TEXT | YES | NULL | Optional but recommended — BR-03-7 |
| `status` | VARCHAR(16) | NO | `'active'` | `active`\|`resolved` |
| `resolved_by_account_id` | BIGINT | YES | NULL | FK → `accounts(id)` RESTRICT |
| `resolved_at` | TIMESTAMPTZ | YES | NULL | |

- Check: `flag_type IN ('behavior','high_no_show_rate','injured',
  'medical_restriction','scholarship','financial_aid','contact_priority',
  'attendance_risk')` — the exact 8 from BR-03-6, no custom flags possible
  by construction (`$table->enum('flag_type', [...])`);
  `(status = 'resolved') = (resolved_at IS NOT NULL AND
  resolved_by_account_id IS NOT NULL)` (raw `DB::statement`).
- **Unique partial index: `(player_id, flag_type) WHERE status = 'active'`**
  — at most one *active* instance of a given flag per player, while still
  allowing reapplication after resolution (BR-03-8: a second row, not an
  update).
- Index: `(trainer_id, flag_type) WHERE status = 'active'` — flag-count
  widgets and segmentation by flag.

```php
DB::statement('CREATE UNIQUE INDEX player_flags_one_active_per_type ON player_flags (player_id, flag_type) WHERE status = \'active\'');
DB::statement('CREATE INDEX player_flags_trainer_active_idx ON player_flags (trainer_id, flag_type) WHERE status = \'active\'');
```

**Model**: `use BelongsToTenant;` `$fillable = ['player_id', 'flag_type',
'applied_by_account_id', 'note']`; casts `'flag_type' =>
PlayerFlagType::class`, `'status' => PlayerFlagStatus::class`;
`belongsTo(PlayerProfile::class)`; a local scope `scopeActive(Builder
$query)`. RLS: standard policy.

#### `player_notes` — was `player_note`

BR-03-10/11/12.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `player_id` | BIGINT | NO | | FK → `player_profiles(id)` RESTRICT |
| `note_type` | VARCHAR(16) | NO | | `general`\|`session` |
| `event_id` | BIGINT | YES | NULL | FK → `events(id)` RESTRICT. Session notes only |
| `note_text` | VARCHAR(1000) | NO | | BR-03-10: 1000-char suggested limit |
| `created_by_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |
| `created_at` | TIMESTAMPTZ | NO | | |
| `updated_at` | TIMESTAMPTZ | YES | NULL | |
| `edited_by_account_id` | BIGINT | YES | NULL | FK → `accounts(id)` RESTRICT |

- Check: `note_type IN (...)`; `(note_type = 'session') = (event_id IS NOT
  NULL)`; `char_length(note_text) <= 1000` (raw `DB::statement`).
- Indexes: `(player_id, created_at DESC)` — chronological display; `(event_id)`.
- 24-hour edit window (BR-03-12) is a Policy check against `created_at`,
  same reasoning as `attendance_edits` — not a CHECK constraint.

**Model**: `use BelongsToTenant;` `$fillable = ['player_id', 'note_type',
'event_id', 'note_text', 'edited_by_account_id']`; cast `'note_type' =>
PlayerNoteType::class`; `belongsTo(PlayerProfile::class)`,
`belongsTo(Event::class)`. RLS: standard policy.

### Content module (Epic-04)

#### `playlists` — was `playlist`

Two orthogonal visibility facts. BR-04-3/4/5; A9; AC-04-41. **Carries the
widened RLS SELECT policy** — see below.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` — always the creator, never changes |
| `title` | VARCHAR(255) | NO | | |
| `description` | TEXT | YES | NULL | |
| `pillar` | VARCHAR(16) | NO | | `learn`\|`practice` |
| `is_public` | BOOLEAN | NO | `false` | Current discovery state |
| `ever_published_at` | TIMESTAMPTZ | YES | NULL | Set once, first time `is_public` becomes true; never cleared |
| `audience` | VARCHAR(24) | NO | `'players_and_coaches'` | `players_and_coaches`\|`coaches_only` — AC-04-41 |
| `filter_skill_levels` / `filter_positions` / `filter_age_levels` | JSONB | YES | NULL | Was `TEXT[]` — see Conventions |
| `created_at` / `updated_at` | TIMESTAMPTZ | NO | | |
| `deleted_at` | TIMESTAMPTZ | YES | NULL | AC-04-36 soft delete |

- Check: `pillar IN (...)`; `audience IN (...)`; `NOT (is_public AND
  audience = 'coaches_only')` — collapses two orthogonal booleans into
  exactly A9's three valid states (public / private / coach-only) by
  forbidding the fourth; `(is_public = false) OR (ever_published_at IS NOT
  NULL)` (all raw `DB::statement`).
- Indexes: `(trainer_id, deleted_at)`; partial `(ever_published_at) WHERE
  ever_published_at IS NOT NULL` — feeds the widened RLS policy and the
  public-library discovery query.
- Soft delete via Eloquent's `SoftDeletes` trait (`$table->softDeletes()`).

```php
Schema::create('playlists', function (Blueprint $table): void {
    $table->id();
    $table->foreignId('trainer_id')->constrained('trainers');
    $table->string('title', 255);
    $table->text('description')->nullable();
    $table->enum('pillar', ['learn', 'practice']);
    $table->boolean('is_public')->default(false);
    $table->timestampTz('ever_published_at')->nullable();
    $table->enum('audience', ['players_and_coaches', 'coaches_only'])->default('players_and_coaches');
    $table->jsonb('filter_skill_levels')->nullable();
    $table->jsonb('filter_positions')->nullable();
    $table->jsonb('filter_age_levels')->nullable();
    $table->timestampsTz();
    $table->softDeletesTz();

    $table->index(['trainer_id', 'deleted_at']);
});

DB::statement('CREATE INDEX playlists_ever_published_idx ON playlists (ever_published_at) WHERE ever_published_at IS NOT NULL');
DB::statement('ALTER TABLE playlists ADD CONSTRAINT playlists_not_public_coaches_only CHECK (NOT (is_public AND audience = \'coaches_only\'))');
DB::statement('ALTER TABLE playlists ADD CONSTRAINT playlists_published_once_stays_published CHECK (is_public = false OR ever_published_at IS NOT NULL)');

// Widened policy, not the standard one — see "Row-Level Security" for the
// full four-policy SQL this call runs.
$this->enableWidenedPublicationIsolation('playlists');
```

**Model** `App\Models\Playlist` — uses `PublishedOrOwnedScope` instead of
plain `TenantScope` (see "Tenancy" above):

```php
final class Playlist extends Model
{
    use SoftDeletes;

    protected $table = 'playlists';

    protected $fillable = ['title', 'description', 'pillar', 'audience',
        'filter_skill_levels', 'filter_positions', 'filter_age_levels'];

    protected static function booted(): void
    {
        static::addGlobalScope(new PublishedOrOwnedScope);

        static::creating(function (self $playlist): void {
            $playlist->trainer_id ??= app(TenantContext::class)->requireId();
        });
    }

    protected function casts(): array
    {
        return [
            'pillar' => ContentPillar::class,
            'audience' => PlaylistAudience::class,
            'is_public' => 'boolean',
            'ever_published_at' => 'datetime',
            'filter_skill_levels' => 'array',
            'filter_positions' => 'array',
            'filter_age_levels' => 'array',
        ];
    }

    public function items(): HasMany
    {
        return $this->hasMany(PlaylistItem::class);
    }
}
```

`PublishedOrOwnedScope` (`app/Models/Scopes/PublishedOrOwnedScope.php`):

```php
final class PublishedOrOwnedScope implements Scope
{
    public function apply(Builder $builder, Model $model): void
    {
        $tenantId = app(TenantContext::class)->id();

        $builder->where(function (Builder $query) use ($tenantId): void {
            $query->where($query->getModel()->qualifyColumn('trainer_id'), $tenantId)
                ->orWhereNotNull($query->getModel()->qualifyColumn('ever_published_at'));
        });
    }
}
```

This mirrors the DB-level widened policy on the Eloquent read side; writes
are still governed by the model's own `trainer_id` plus RLS `WITH CHECK`
(the global scope has no write-side effect, as noted under "Tenancy"). RLS:
**widened policy**, not standard — full SQL in "Row-Level Security".

#### `content_items` — was `content_item`

BR-04-1/2/3/4/5/11/12. Same widened-policy treatment as `playlists`.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` — original creator, ownership never transfers (BR-04-11) |
| `type` | VARCHAR(16) | NO | | `video`\|`drill` |
| `pillar` | VARCHAR(16) | NO | | `learn`\|`practice` |
| `title` | VARCHAR(100) | NO | | |
| `instructions` | TEXT | YES | NULL | ≤1000 chars |
| `youtube_url` | VARCHAR(2048) | NO | | BR-04-1: unlisted YouTube only, no file storage |
| `duration_seconds` | INTEGER | YES | NULL | Nullable — see Open questions |
| `tags` | JSONB | YES | NULL | Was `TEXT[]` — see Conventions |
| `is_public` | BOOLEAN | NO | `false` | |
| `ever_published_at` | TIMESTAMPTZ | YES | NULL | Same widening mechanism as `playlists` |
| `created_at` / `updated_at` | TIMESTAMPTZ | NO | | |
| `deleted_at` | TIMESTAMPTZ | YES | NULL | AC-04-35/36 |

- Check: `type IN (...)`; `pillar IN (...)`; `type <> 'drill' OR pillar =
  'practice'`; `char_length(title) <= 100`; `instructions IS NULL OR
  char_length(instructions) <= 1000`; `(is_public = false) OR
  (ever_published_at IS NOT NULL)` (raw `DB::statement`).
- Indexes: `(trainer_id, deleted_at)`; `(type, deleted_at) WHERE is_public
  AND deleted_at IS NULL` — public drill search/filter (AC-04-19); a GIN
  index on `tags` (`DB::statement('CREATE INDEX ... USING gin (tags)')`) is
  **not** committed here — no epic states a tag-search performance target,
  same as the source's own deferral, and JSONB's GIN option is a strict
  upgrade path if that need appears later.

**Model** `App\Models\ContentItem` — same `PublishedOrOwnedScope` pattern as
`Playlist`; `use SoftDeletes;` `$fillable = ['type', 'pillar', 'title',
'instructions', 'youtube_url', 'duration_seconds', 'tags']`; casts `'type' =>
ContentType::class`, `'pillar' => ContentPillar::class`, `'tags' => 'array'`,
`'is_public' => 'boolean'`, `'ever_published_at' => 'datetime'`;
`hasOne(DrillDetail::class, 'content_item_id')`;
`hasMany(PlaylistItem::class)`, `hasMany(ContentProgress::class)`. RLS:
**widened policy**.

#### `drill_details` — was `drill_detail`

Joined extension of `content_items`, populated only when `type = 'drill'`.
BR-04-18/19.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `content_item_id` | BIGINT | NO | | **PK, and FK** to `content_items(id)` ON DELETE CASCADE |
| `difficulty_level` | VARCHAR(16) | NO | | `beginner`\|`intermediate`\|`advanced`\|`elite` |
| `equipment` | JSONB | YES | NULL | Was `TEXT[]` |
| `space_requirement` | VARCHAR(16) | YES | NULL | `small`\|`medium`\|`large` |
| `player_count` | VARCHAR(20) | YES | NULL | Free text, e.g. "1-2" |
| `duration_min_minutes` / `duration_max_minutes` | INTEGER | YES | NULL | |
| `categories` | JSONB | YES | NULL | Was `TEXT[]` |

- Check: `difficulty_level IN (...)`; `space_requirement IS NULL OR ... IN
  (...)`; `duration_max_minutes IS NULL OR duration_min_minutes IS NULL OR
  duration_max_minutes >= duration_min_minutes` (raw `DB::statement`).
- **No `trainer_id` column of its own** — RLS is enforced through the
  relationship to `content_items`; there is no code path that queries
  `drill_details` alone without its parent. Not in the RLS table list.

The source mapped this as **joined-table inheritance**
(`Drill extends ContentItem`) — Eloquent has no native joined-table
inheritance mechanism. The idiomatic Eloquent translation is the same
shared-PK 1-to-1 extension pattern used for `account_profiles`/
`trainer_billing_settings`, not a subclass:

```php
Schema::create('drill_details', function (Blueprint $table): void {
    $table->foreignId('content_item_id')->primary()->constrained('content_items')->cascadeOnDelete();
    $table->enum('difficulty_level', ['beginner', 'intermediate', 'advanced', 'elite']);
    $table->jsonb('equipment')->nullable();
    $table->enum('space_requirement', ['small', 'medium', 'large'])->nullable();
    $table->string('player_count', 20)->nullable();
    $table->integer('duration_min_minutes')->nullable();
    $table->integer('duration_max_minutes')->nullable();
    $table->jsonb('categories')->nullable();
});
```

**Model** `App\Models\DrillDetail`:

```php
final class DrillDetail extends Model
{
    protected $table = 'drill_details';
    protected $primaryKey = 'content_item_id';
    public $incrementing = false;
    public $timestamps = false;

    protected $fillable = ['difficulty_level', 'equipment', 'space_requirement',
        'player_count', 'duration_min_minutes', 'duration_max_minutes', 'categories'];

    protected function casts(): array
    {
        return [
            'difficulty_level' => DrillDifficulty::class,
            'space_requirement' => DrillSpaceRequirement::class,
            'equipment' => 'array',
            'categories' => 'array',
        ];
    }

    public function contentItem(): BelongsTo
    {
        return $this->belongsTo(ContentItem::class, 'content_item_id');
    }
}
```

`ContentItem::drill()`/`DrillDetail::contentItem()` is a plain
`hasOne`/`belongsTo` pair on the shared key — callers that need "a drill" as
one object eager-load `ContentItem::with('drill')` rather than a
`Drill`-subtype query, which is the one genuine ergonomics loss versus
the source's joined-table inheritance (no single-class polymorphic query),
worth naming rather than glossing over. No RLS of its own — reached only
through `content_items`.

#### `playlist_items` — was `playlist_item`

BR-04-12; "same video may appear at more than one position" edge case.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` — the playlist owner's tenant, not the content's creator |
| `playlist_id` | BIGINT | NO | | FK → `playlists(id)` ON DELETE CASCADE |
| `content_item_id` | BIGINT | NO | | FK → `content_items(id)` RESTRICT |
| `sequence_order` | INTEGER | NO | | |
| `is_required` | BOOLEAN | NO | `true` | |
| `trainer_notes` | TEXT | YES | NULL | |

- Unique: `(playlist_id, sequence_order)` **DEFERRABLE INITIALLY DEFERRED**
  — lets a reorder operation update every row's `sequence_order` inside one
  transaction without a transient collision. No Blueprint helper exposes
  `DEFERRABLE`; raw SQL.
- **No unique constraint on `(playlist_id, content_item_id)`** — the same
  content may legitimately appear twice in one playlist.
- Index: `(content_item_id)` — "used in N playlists" delete-warning count
  (AC-04-35), scoped to the deleting trainer's own tenant via RLS.
- `content_item_id` is `RESTRICT`, not `CASCADE`: it never actually fires,
  because `content_items` is soft-deleted — this is what lets BR-04-12's
  "reference breaks and shows Unavailable" work without an orphaned FK.

```php
Schema::create('playlist_items', function (Blueprint $table): void {
    $table->id();
    $table->foreignId('trainer_id')->constrained('trainers');
    $table->foreignId('playlist_id')->constrained('playlists')->cascadeOnDelete();
    $table->foreignId('content_item_id')->constrained('content_items')->restrictOnDelete();
    $table->integer('sequence_order');
    $table->boolean('is_required')->default(true);
    $table->text('trainer_notes')->nullable();

    $table->index('content_item_id');
});

DB::statement('ALTER TABLE playlist_items ADD CONSTRAINT playlist_items_playlist_sequence_unique UNIQUE (playlist_id, sequence_order) DEFERRABLE INITIALLY DEFERRED');
```

**Model**: `use BelongsToTenant;` `$fillable = ['playlist_id',
'content_item_id', 'sequence_order', 'is_required', 'trainer_notes']`;
`public $timestamps = false;`; `belongsTo(Playlist::class)`,
`belongsTo(ContentItem::class)`. RLS: standard policy.

#### `playlist_assignments` — was `playlist_assignment`

BR-04-13/14/15.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `playlist_id` | BIGINT | NO | | FK → `playlists(id)` RESTRICT |
| `target_type` | VARCHAR(16) | NO | | `player`\|`label`\|`skill_level` |
| `target_player_id` | BIGINT | YES | NULL | FK → `player_profiles(id)` RESTRICT |
| `target_label_id` | BIGINT | YES | NULL | FK → `labels(id)` RESTRICT |
| `target_skill_level` | VARCHAR(50) | YES | NULL | |
| `assigned_by_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |
| `assigned_at` | TIMESTAMPTZ | NO | `now()` | |
| `due_date` | DATE | YES | NULL | |
| `note` | TEXT | YES | NULL | Visible to the player |

- Check: `target_type IN (...)`; exactly one of the three `target_*`
  columns is non-null, matching `target_type` (raw `DB::statement`).
- Indexes: `(playlist_id)`; `(target_player_id)`; `(target_label_id)`.
- **No `status` column.** An assignment can target a whole label or
  skill-level group, so one status value per row cannot represent N
  players' individual progress. Per-player status is derived at read time
  by joining `content_progress` across the assigned playlist's items for a
  specific player — a named Eloquent query method,
  `PlaylistAssignment::resolvedStatusFor(int $playerId): AssignmentStatus`,
  not a stored column. BR-04-15's "reassignment keeps progress" falls out
  of this for free, since `content_progress` never depended on the
  assignment row. Recorded in Decisions.

**Model**: `use BelongsToTenant;` `$fillable = ['playlist_id', 'target_type',
'target_player_id', 'target_label_id', 'target_skill_level',
'assigned_by_account_id', 'due_date', 'note']`; cast `'target_type' =>
PlaylistAssignmentTargetType::class`, `'due_date' => 'date'`;
`belongsTo(Playlist::class)`, `belongsTo(PlayerProfile::class,
'target_player_id')`, `belongsTo(Label::class, 'target_label_id')`. RLS:
standard policy.

#### `content_progress` — was `content_progress` (kept singular; see naming note)

Keyed per (player, content item), not per (player, playlist) — this is what
makes BR-04-17 (progress survives playlist restructuring) true by
construction. BR-04-16/17.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `player_id` | BIGINT | NO | | FK → `player_profiles(id)` RESTRICT |
| `content_item_id` | BIGINT | NO | | FK → `content_items(id)` RESTRICT |
| `status` | VARCHAR(16) | NO | `'not_started'` | `not_started`\|`in_progress`\|`completed` |
| `progress_percent` | SMALLINT | NO | `0` | 0–100 |
| `first_viewed_at` / `completed_at` | TIMESTAMPTZ | YES | NULL | |
| `watch_time_seconds` | INTEGER | NO | `0` | |
| `updated_at` | TIMESTAMPTZ | NO | | |

- Unique: `(player_id, content_item_id)`.
- Check: `status IN (...)`; `progress_percent BETWEEN 0 AND 100`; `(status =
  'completed') = (completed_at IS NOT NULL)` (raw `DB::statement`).
- Index: `(trainer_id, player_id)`.
- BR-04-16: completion is auto-set the moment playback starts — application
  logic (an Action, `App\Actions\Content\RecordPlaybackStarted`), not the
  schema; idempotent replay is a plain `->where('completed_at', null)->update([...])`
  guard.

**Model** `App\Models\ContentProgress`: `protected $table =
'content_progress';` (explicit — deliberately not auto-pluralized, see
naming convention); `use BelongsToTenant;` `$fillable = ['player_id',
'content_item_id', 'watch_time_seconds']`; casts `'status' =>
ContentProgressStatus::class`, `'first_viewed_at' => 'datetime'`,
`'completed_at' => 'datetime'`; `belongsTo(PlayerProfile::class)`,
`belongsTo(ContentItem::class)`. RLS: standard policy.

#### `playlist_access_grants` — was `playlist_access_grant`

Per-playlist one-time purchase (A8). BR-04-6/7/8/9.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `playlist_id` | BIGINT | NO | | FK → `playlists(id)` RESTRICT |
| `player_id` | BIGINT | NO | | FK → `player_profiles(id)` RESTRICT — the beneficiary child; see Open questions |
| `parent_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT — who paid |
| `payment_record_id` | BIGINT | NO | | **Deferred FK** → `payment_records(id)` — attached in the Billing stage, `NOT NULL` from the start (unlike `rsvps.payment_record_id`, which is nullable for the free-event path — every playlist purchase has a payment) |
| `granted_at` | TIMESTAMPTZ | NO | `now()` | |

- Unique: `payment_record_id` (one grant per funding payment); `(playlist_id,
  player_id)` — access "stays accessible forever" (AC-05-18), so a second
  purchase is not a new grant.
- Index: `(player_id)` — the paywall's hot read, checked on every
  locked-content render.

**Model**: `use BelongsToTenant;` `$fillable = ['playlist_id', 'player_id',
'parent_account_id']`; `public $timestamps = false;` (`granted_at` only);
`belongsTo(Playlist::class)`, `belongsTo(PlayerProfile::class)`,
`belongsTo(Account::class, 'parent_account_id')`,
`belongsTo(PaymentRecord::class)`. RLS: standard policy.

#### `content_usage` — was `content_usage` (kept singular; see naming note)

Scoped to the *reusing* trainer — "the creator's 'used by N trainers'
figure is therefore a crossing read." BR-04-12; AC-04-37.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` — the **reusing** trainer, not the creator |
| `content_item_id` | BIGINT | NO | | FK → `content_items(id)` RESTRICT — the original, possibly another trainer's, content |
| `first_used_at` | TIMESTAMPTZ | NO | `now()` | |

- Unique: `(trainer_id, content_item_id)` — upserted the first time that
  trainer adds the item to *any* of their own playlists.
- Index: `(content_item_id)` — the crossing-read query, `SELECT COUNT(*)
  FROM content_usage WHERE content_item_id = ?`, run via
  `CrossTenantReadService::contentItemUsageCount()` above over the
  `pgsql_crossing` connection. Genuinely cross-tenant question, genuinely
  cross-tenant mechanism — not a leak.
- Distinct from the same-tenant "used in N of *my own* playlists" warning on
  drill delete (AC-04-35), answered by counting `playlist_items` rows
  within the deleting trainer's own RLS-scoped view — no crossing read
  needed for that one.

**Model** `App\Models\ContentUsage`: `protected $table = 'content_usage';`
(explicit); `use BelongsToTenant;` `$fillable = ['content_item_id']`;
`public $timestamps = false;`; `belongsTo(ContentItem::class)`. RLS:
standard policy.

### Billing module (Epic-05)

See "The token and payment ledger" below for the invariant-by-invariant
reasoning behind `token_entries`/`token_balances`; this section states
columns and constraints.

#### `token_entries` — was `token_entry`

Append-only. Entry kinds and signs, invariants I1–I7.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `parent_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT — the ledger subject, paired with `trainer_id` |
| `kind` | VARCHAR(20) | NO | | `purchase`\|`gift`\|`referral_reward`\|`refund`\|`spend`\|`adjustment` |
| `amount` | INTEGER | NO | | Signed token count, never 0 |
| `beneficiary_player_id` | BIGINT | YES | NULL | FK → `player_profiles(id)` RESTRICT. **Required** for `spend`/`refund` (A7, I3) |
| `refunds_entry_id` | BIGINT | YES | NULL | FK → `token_entries(id)` RESTRICT, self-referencing. Required for `refund`, forbidden otherwise |
| `payment_record_id` | BIGINT | YES | NULL | FK → `payment_records(id)` — `payment_records` precedes `token_entries` in build order, so this FK attaches immediately, not deferred |
| `related_event_id` | BIGINT | YES | NULL | FK → `events(id)` RESTRICT |
| `related_content_item_id` | BIGINT | YES | NULL | FK → `content_items(id)` RESTRICT |
| `referral_id` | BIGINT | YES | NULL | **Deferred FK** → `referrals(id)` — attached in the Growth stage |
| `performed_by_account_id` | BIGINT | YES | NULL | FK → `accounts(id)` RESTRICT. Set for `gift`/`adjustment` |
| `description` | TEXT | NO | | Human-readable reason, always required |
| `created_at` | TIMESTAMPTZ | NO | `now()` | Append-only — no `updated_at` |

- Check: `kind IN (...)`; **sign-by-kind (I5)**:
  `(kind IN ('purchase','gift','referral_reward','refund') AND amount > 0)
  OR (kind = 'spend' AND amount < 0)
  OR (kind = 'adjustment' AND amount <> 0)`;
  **beneficiary required for spend/refund (I3, A7)**: `kind NOT IN
  ('spend','refund') OR beneficiary_player_id IS NOT NULL`; **refund must
  reference, others must not**: `(kind = 'refund') = (refunds_entry_id IS
  NOT NULL)`; `char_length(trim(description)) > 0` — all raw
  `DB::statement`, no Blueprint helper expresses cross-column CHECKs.
- Unique: **`(payment_record_id, kind) WHERE payment_record_id IS NOT
  NULL`** — this is I6, "at most one entry per (payment record, purpose)":
  outbound idempotency at the ledger level, so a replayed webhook cannot
  double-credit the same payment.
- Indexes: `(trainer_id, parent_account_id, created_at)` — I1's
  reconciliation query and the transaction-history screen (AC-05-23);
  `(beneficiary_player_id)`; `(refunds_entry_id) WHERE refunds_entry_id IS
  NOT NULL` — I4's lookup.
- **I3's second half** ("every refund inherits the beneficiary of the spend
  it references") is not a CHECK — Postgres CHECK constraints cannot read
  another row. `TokenLedgerService::refund()` (an Action class, below)
  copies `beneficiary_player_id` from the referenced spend entry at insert
  time; a test asserts they never diverge.
- **I4** ("sum of refunds against one spend never exceeds that spend") is
  enforced by locking, not a constraint, and cannot be a maintained
  running-counter column because I7 forbids `UPDATE` entirely — there is no
  row to increment. Pattern: `TokenEntry::query()->where('id',
  $spendId)->lockForUpdate()->value('id')` (locks the spend row without
  modifying it — a no-op locking `SELECT` is a legal, common serialization
  primitive, expressed in Eloquent exactly as `->lockForUpdate()` on a query
  that selects but does not update), then a plain `TokenEntry::query()->where('refunds_entry_id',
  $spendId)->sum('amount')`, then compare against the spend's original
  `amount` before inserting the refund — all inside one `DB::transaction()`.
  Locking the spend row first serializes every concurrent refund attempt
  against it.
- **I7 (append-only) is a database privilege, not a convention** — same
  `REVOKE` pattern as `audit_log_entries`.
- **I1** (balance projection = sum of entries) is not a DB constraint at
  all — it spans two tables. Maintained transactionally by
  `TokenLedgerService` (insert the entry, update `token_balances` in the
  same transaction, balance row locked first per the fixed lock order
  below) and independently verified by a scheduled reconciliation Artisan
  command plus a test.

```php
// database/migrations/2025_01_05_000001_create_token_entries_table.php
Schema::create('token_entries', function (Blueprint $table): void {
    $table->id();
    $table->foreignId('trainer_id')->constrained('trainers');
    $table->foreignId('parent_account_id')->constrained('accounts')->restrictOnDelete();
    $table->enum('kind', ['purchase', 'gift', 'referral_reward', 'refund', 'spend', 'adjustment']);
    $table->integer('amount');
    $table->foreignId('beneficiary_player_id')->nullable()->constrained('player_profiles')->restrictOnDelete();
    $table->foreignId('refunds_entry_id')->nullable()->constrained('token_entries')->restrictOnDelete();
    $table->foreignId('payment_record_id')->nullable()->constrained('payment_records')->restrictOnDelete();
    $table->foreignId('related_event_id')->nullable()->constrained('events')->restrictOnDelete();
    $table->foreignId('related_content_item_id')->nullable()->constrained('content_items')->restrictOnDelete();
    $table->unsignedBigInteger('referral_id')->nullable(); // deferred FK — attached in the Growth stage
    $table->foreignId('performed_by_account_id')->nullable()->constrained('accounts')->restrictOnDelete();
    $table->text('description');
    $table->timestampTz('created_at')->useCurrent();

    $table->index(['trainer_id', 'parent_account_id', 'created_at']);
    $table->index('beneficiary_player_id');
});

DB::statement('CREATE UNIQUE INDEX token_entries_one_per_payment_purpose ON token_entries (payment_record_id, kind) WHERE payment_record_id IS NOT NULL');
DB::statement('CREATE INDEX token_entries_refunds_entry_idx ON token_entries (refunds_entry_id) WHERE refunds_entry_id IS NOT NULL');
DB::statement("ALTER TABLE token_entries ADD CONSTRAINT token_entries_sign_by_kind CHECK (
    (kind IN ('purchase','gift','referral_reward','refund') AND amount > 0)
    OR (kind = 'spend' AND amount < 0)
    OR (kind = 'adjustment' AND amount <> 0)
)");
DB::statement("ALTER TABLE token_entries ADD CONSTRAINT token_entries_beneficiary_required CHECK (
    kind NOT IN ('spend','refund') OR beneficiary_player_id IS NOT NULL
)");
DB::statement("ALTER TABLE token_entries ADD CONSTRAINT token_entries_refund_reference CHECK (
    (kind = 'refund') = (refunds_entry_id IS NOT NULL)
)");
DB::statement("ALTER TABLE token_entries ADD CONSTRAINT token_entries_description_present CHECK (char_length(trim(description)) > 0)");

// I7 — enforced by privilege, not convention. Matches audit_log_entries.
DB::statement('REVOKE UPDATE, DELETE ON token_entries FROM pp_app');
```

**Model** `App\Models\TokenEntry`:

```php
final class TokenEntry extends Model
{
    use BelongsToTenant;

    protected $table = 'token_entries';
    public $timestamps = false; // created_at only — no updated_at, ever

    protected $fillable = ['parent_account_id', 'kind', 'amount',
        'beneficiary_player_id', 'refunds_entry_id', 'payment_record_id',
        'related_event_id', 'related_content_item_id', 'referral_id',
        'performed_by_account_id', 'description'];

    protected function casts(): array
    {
        return ['kind' => TokenEntryKind::class, 'created_at' => 'datetime'];
    }

    public function parentAccount(): BelongsTo
    {
        return $this->belongsTo(Account::class, 'parent_account_id');
    }

    public function refundedEntry(): BelongsTo
    {
        return $this->belongsTo(self::class, 'refunds_entry_id');
    }
}
```

No `update()`/`delete()` call on this model can succeed regardless — the
`REVOKE` makes it a database-level guarantee, not a model-level one (a
missing `$guarded`/observer would not reopen the hole the way it would on a
purely conventional "no setters" rule).

#### `token_balances` — was `token_balance`

The locked projection and concurrency anchor. I1, I2.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `parent_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |
| `balance` | INTEGER | NO | `0` | |
| `updated_at` | TIMESTAMPTZ | NO | | |

- Unique: `(trainer_id, parent_account_id)`.
- Check: `balance >= 0` — **I2**, the database-level backstop.
- No optimistic-locking `version` column: the fixed lock order (token
  balance row first, then event row) is pessimistic (`lockForUpdate()`) by
  explicit design — a `version` column here would be a second, redundant
  concurrency mechanism.
- Row acquired via `lockForUpdate()` as the **first** lock in any
  transaction that also touches `events` capacity (paid RSVP,
  RSVP-cancellation refund, capacity-checked content purchase) — funnelled
  through one method so every call site goes through the same order:

```php
final class TokenLedgerService
{
    public function lockBalance(int $trainerId, int $parentAccountId): TokenBalance
    {
        return TokenBalance::query()
            ->where('trainer_id', $trainerId)
            ->where('parent_account_id', $parentAccountId)
            ->lockForUpdate()
            ->firstOrFail();
    }
}
```

The source's equivalent event-locking method's Laravel translation —
`Event::query()->lockForUpdate()->findOrFail($eventId)` — is always called
**second**, only from inside a transaction that already holds the balance
lock. A test drives both a paid RSVP and an RSVP-cancellation refund
concurrently against the same parent and event and asserts no deadlock,
same as the source names explicitly.

**Model** `App\Models\TokenBalance`: `use BelongsToTenant;` `$fillable =
['parent_account_id', 'balance']`; `belongsTo(Account::class,
'parent_account_id')`. RLS: standard policy.

#### `payment_records` — was `payment_record`

Mirror of a Stripe object, kept mutable (unlike the ledger) because it
reconciles to Stripe's own lifecycle.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `type` | VARCHAR(20) | NO | | `token_purchase`\|`event_rsvp`\|`content_purchase`\|`player_subscription`\|`camp_registration` |
| `payer_account_id` | BIGINT | YES | NULL | FK → `accounts(id)` RESTRICT. **Nullable** — A3/A4: a camp payer may have no account |
| `contact_name` | VARCHAR(255) | NO | | Always-present snapshot, independent of `payer_account_id` |
| `contact_email` | CITEXT | NO | | |
| `contact_phone` | VARCHAR(32) | YES | NULL | |
| `payment_method` | VARCHAR(8) | NO | | `token`\|`card` |
| `status` | VARCHAR(16) | NO | `'pending'` | `pending`\|`completed`\|`failed`\|`refunded` |
| `amount_minor_units` | INTEGER | NO | | Positive; for a refund row, the amount refunded |
| `fee_rate_basis_points` | SMALLINT | YES | NULL | Snapshotted at creation for charge rows — see "Money and the platform fee". NULL on refund rows |
| `platform_fee_minor_units` | INTEGER | NO | `0` | Computed once at creation, never recomputed |
| `stripe_payment_intent_id` / `stripe_charge_id` / `stripe_refund_id` | VARCHAR(255) | YES | NULL | |
| `refunds_payment_record_id` | BIGINT | YES | NULL | FK → `payment_records(id)` RESTRICT, self-referencing. Set only on a refund row — refunds are new records, never mutations |
| `related_token_package_id` | BIGINT | YES | NULL | FK → `token_packages(id)` RESTRICT |
| `related_rsvp_id` | BIGINT | YES | NULL | FK → `rsvps(id)` RESTRICT — `rsvps` exists by the Billing stage (built after Scheduling) |
| `related_playlist_id` | BIGINT | YES | NULL | FK → `playlists(id)` RESTRICT — `playlists` exists by the Billing stage (built after Content) |
| `related_form_submission_id` | BIGINT | YES | NULL | **Deferred FK** → `form_submissions(id)` — attached in the Forms stage |
| `related_subscription_entitlement_id` | BIGINT | YES | NULL | FK → `subscription_entitlements(id)` RESTRICT. Backfilled after the entitlement is granted |
| `created_at` / `updated_at` | TIMESTAMPTZ | NO | | |

- Check: `type IN (...)`; `payment_method IN (...)`; `status IN (...)`;
  `amount_minor_units > 0`; `(refunds_payment_record_id IS NULL) =
  (fee_rate_basis_points IS NOT NULL)`; the type-to-related-FK mapping —
  exactly one of the four `related_*_id`/`related_form_submission_id`
  columns is non-null and matches `type` (`player_subscription` rows have
  none of the four set at creation — see note) — all raw `DB::statement`.
- Unique: `stripe_payment_intent_id`, `stripe_charge_id`,
  `stripe_refund_id` (each nullable-unique).
- Indexes: `(trainer_id, payer_account_id, created_at DESC)` — transaction
  history (AC-05-22/23), trainer-isolated by RLS (BR-05-17); `(status)
  WHERE status = 'pending'` — 7-day pending-payment sweep (BR-05-9);
  `(refunds_payment_record_id) WHERE refunds_payment_record_id IS NOT NULL`.
- **Idempotency**: `id` is assigned (Eloquent's `create()` returns the
  model with its DB-assigned `id` populated) **before** the Stripe API
  call, and the Stripe idempotency key is derived from it — a timeout retry
  reuses the key. The row exists in `pending` status ahead of the Stripe
  round-trip for every type; `status` and the `related_*`/`stripe_*`
  columns are updated (this table is **not** append-only, unlike the
  ledger) as the webhook confirms outcome.
- **No trainer earnings/net/payout column anywhere on this table** — the
  earnings boundary: `StripeReportingReader` (ported as
  `App\Services\StripeReportingReader`) is the only source of any figure
  presented as earnings/revenue/payout, read live from Stripe, never
  cached, never summed from this table.

```php
Schema::create('payment_records', function (Blueprint $table): void {
    $table->id();
    $table->foreignId('trainer_id')->constrained('trainers');
    $table->enum('type', ['token_purchase', 'event_rsvp', 'content_purchase', 'player_subscription', 'camp_registration']);
    $table->foreignId('payer_account_id')->nullable()->constrained('accounts')->restrictOnDelete();
    $table->string('contact_name', 255);
    $table->addColumn('citext', 'contact_email');
    $table->string('contact_phone', 32)->nullable();
    $table->enum('payment_method', ['token', 'card']);
    $table->enum('status', ['pending', 'completed', 'failed', 'refunded'])->default('pending');
    $table->integer('amount_minor_units');
    $table->smallInteger('fee_rate_basis_points')->nullable();
    $table->integer('platform_fee_minor_units')->default(0);
    $table->string('stripe_payment_intent_id', 255)->nullable()->unique();
    $table->string('stripe_charge_id', 255)->nullable()->unique();
    $table->string('stripe_refund_id', 255)->nullable()->unique();
    $table->foreignId('refunds_payment_record_id')->nullable()->constrained('payment_records')->restrictOnDelete();
    $table->foreignId('related_token_package_id')->nullable()->constrained('token_packages')->restrictOnDelete();
    $table->foreignId('related_rsvp_id')->nullable()->constrained('rsvps')->restrictOnDelete();
    $table->foreignId('related_playlist_id')->nullable()->constrained('playlists')->restrictOnDelete();
    $table->unsignedBigInteger('related_form_submission_id')->nullable(); // deferred FK — attached in the Forms stage
    $table->foreignId('related_subscription_entitlement_id')->nullable()->constrained('subscription_entitlements')->restrictOnDelete();
    $table->timestampsTz();

    $table->index(['trainer_id', 'payer_account_id', DB::raw('created_at DESC')]);
});

DB::statement("CREATE INDEX payment_records_pending_idx ON payment_records (status) WHERE status = 'pending'");
DB::statement('CREATE INDEX payment_records_refunds_idx ON payment_records (refunds_payment_record_id) WHERE refunds_payment_record_id IS NOT NULL');
DB::statement('ALTER TABLE payment_records ADD CONSTRAINT payment_records_amount_positive CHECK (amount_minor_units > 0)');
DB::statement('ALTER TABLE payment_records ADD CONSTRAINT payment_records_fee_rate_on_charges_only CHECK ((refunds_payment_record_id IS NULL) = (fee_rate_basis_points IS NOT NULL))');
```

**Model** `App\Models\PaymentRecord`: `use BelongsToTenant;` `$fillable =
['type', 'payer_account_id', 'contact_name', 'contact_email',
'contact_phone', 'payment_method', 'amount_minor_units',
'related_token_package_id', 'related_rsvp_id', 'related_playlist_id',
'related_form_submission_id']` (`fee_rate_basis_points`/
`platform_fee_minor_units`/Stripe columns excluded — set only by
`PlatformFeeCalculator`/the webhook handler, never mass-assigned); casts
`'type' => PaymentRecordType::class`, `'payment_method' =>
PaymentMethod::class`, `'status' => PaymentRecordStatus::class`;
`belongsTo` to `Account`, `TokenPackage`, `Rsvp`, `Playlist`,
`FormSubmission`, `SubscriptionEntitlement`; `hasMany(TokenEntry::class)`. A
local scope `scopePending(Builder $query)` for the sweep. RLS: standard
policy.

#### `token_packages` — was `token_package`

Trainer-configured bundles. BR-05-1; Q-05.03 default.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `label` | VARCHAR(100) | NO | | e.g. "10 tokens" |
| `token_count` | INTEGER | NO | | |
| `price_minor_units` | INTEGER | NO | | |
| `is_active` | BOOLEAN | NO | `true` | |
| `created_at` | TIMESTAMPTZ | NO | | |

Check: `token_count > 0`; `price_minor_units > 0`. Index: `(trainer_id,
is_active)`. Seeded per trainer at creation from `platform_configurations`'
illustrative packages (10/$90, 25/$225, 50/$450 — Q-05.03 default) —
application logic (a `TokenPackageSeeder` **Action**, run at trainer
creation, not the database `Seeder` class used for local/dev fixtures — see
"Seeders and factories" for that distinction), trainer-editable afterward.
A custom-amount purchase (AC-05-4) needs no package row — priced directly
off `trainer_billing_settings.token_price_minor_units`. **Model**: `use
BelongsToTenant;` `$fillable = ['label', 'token_count', 'price_minor_units',
'is_active']`; `public $timestamps = false;` (`created_at` only). RLS:
standard policy.

#### `subscription_entitlements` — was `subscription_entitlement`

BR-05-14. "The subscription token" — an entitlement grant, not a ledger
entry.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `parent_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |
| `activation_date` | DATE | NO | | Purchaser-chosen, may be future-dated |
| `window_ends_on` | DATE | NO | generated | `storedAs('activation_date + 30')` |
| `payment_record_id` | BIGINT | NO | | FK → `payment_records(id)` RESTRICT, UNIQUE |
| `created_at` | TIMESTAMPTZ | NO | | |

- Unique: `payment_record_id`.
- **Exclusion constraint**, not a plain unique index: no two entitlement
  windows for the same (trainer, parent) pair may overlap. Requires
  `CREATE EXTENSION btree_gist` (GiST has no native equality operator class
  for `bigint`; `btree_gist` supplies one so the `WITH =` terms can share
  the same GiST index as the `&&` range term) — created once, in the M0
  bootstrap migration, alongside `citext`.
- Index: `(trainer_id, parent_account_id, activation_date, window_ends_on)`
  — "does this parent hold an active/pending entitlement" and the
  1-per-day advance-booking check both filter on this shape.
- **No stored `status` column.** `pending_activation`/`active`/`expired`
  are derived at read time from `activation_date`/`window_ends_on` versus
  "today" in `trainers.timezone` — a model accessor, not a column.

```php
Schema::create('subscription_entitlements', function (Blueprint $table): void {
    $table->id();
    $table->foreignId('trainer_id')->constrained('trainers');
    $table->foreignId('parent_account_id')->constrained('accounts')->restrictOnDelete();
    $table->date('activation_date');
    $table->date('window_ends_on')->storedAs('activation_date + 30');
    $table->foreignId('payment_record_id')->unique()->constrained('payment_records')->restrictOnDelete();
    $table->timestampTz('created_at')->useCurrent();

    $table->index(['trainer_id', 'parent_account_id', 'activation_date', 'window_ends_on']);
});

DB::statement('ALTER TABLE subscription_entitlements ADD CONSTRAINT subscription_entitlements_no_overlap
    EXCLUDE USING gist (
        trainer_id WITH =,
        parent_account_id WITH =,
        daterange(activation_date, window_ends_on, \'[]\') WITH &&
    )');
```

**Model** `App\Models\SubscriptionEntitlement`: `use BelongsToTenant;`
`public $timestamps = false;` (`created_at` only; `window_ends_on` is
database-generated, excluded from `$fillable`); `$fillable =
['parent_account_id', 'activation_date', 'payment_record_id']`; cast
`'activation_date' => 'date'`, `'window_ends_on' => 'date'`;
`belongsTo(Account::class, 'parent_account_id')`,
`belongsTo(PaymentRecord::class)`; accessor `status(): SubscriptionEntitlementStatus`
computing pending/active/expired from the two dates and the parent
trainer's timezone. RLS: standard policy.

#### `entitlement_coverages` — was `entitlement_coverage`

Which entitlement covered which RSVP. Implements BR-05-14's "1 advance
booking per day" rule and AC-02-63.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `subscription_entitlement_id` | BIGINT | NO | | FK → `subscription_entitlements(id)` RESTRICT |
| `rsvp_id` | BIGINT | NO | | FK → `rsvps(id)` RESTRICT, UNIQUE |
| `covered_at` | TIMESTAMPTZ | NO | `now()` | |

Unique: `rsvp_id`. Index: `(subscription_entitlement_id)`. The
advance-booking check is a query, not new schema: does any
`entitlement_coverages` row exist, joined to an `rsvps` row whose
`events.starts_at` falls on the same calendar date (trainer timezone) as
the requested event, where that date is strictly after today —
`rsvp_id`'s uniqueness plus the `rsvps`→`events` join already carries this,
no new index needed. **Model**: `use BelongsToTenant;` `$fillable =
['subscription_entitlement_id', 'rsvp_id']`; `public $timestamps = false;`;
`belongsTo(SubscriptionEntitlement::class)`, `belongsTo(Rsvp::class)`. RLS:
standard policy.

### Growth module (Epic-06)

#### `referral_links` — was `referral_link`

One per (player, trainer). BR-06-2 (attribution), US-06.01.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `player_id` | BIGINT | NO | | FK → `player_profiles(id)` RESTRICT — the referrer |
| `created_at` | TIMESTAMPTZ | NO | | |

Unique: `(trainer_id, player_id)` — the link's URL is composed from
`trainers.slug` and this row's identity at render time
(`platform.com/join/{trainer-slug}/{player-id}`); no separate stored code
column, since nothing about the URL is secret (a referral link is meant to
be shared widely). **Model**: `use BelongsToTenant;` `public $timestamps =
false;` `$fillable = ['player_id']`; `belongsTo(PlayerProfile::class)`;
`hasMany(Referral::class)`. RLS: standard policy.

#### `referrals` — was `referral`

BR-06-1/2/3.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `referral_link_id` | BIGINT | NO | | FK → `referral_links(id)` RESTRICT |
| `referrer_player_id` | BIGINT | NO | | FK → `player_profiles(id)` RESTRICT |
| `referee_player_id` | BIGINT | NO | | FK → `player_profiles(id)` RESTRICT, UNIQUE |
| `clicked_at` | TIMESTAMPTZ | NO | | From the 30-day attribution cookie |
| `registered_at` | TIMESTAMPTZ | NO | | |
| `status` | VARCHAR(16) | NO | `'pending'` | `pending`\|`converted` — see note |
| `first_purchase_payment_record_id` | BIGINT | YES | NULL | FK → `payment_records(id)` RESTRICT |
| `first_purchase_at` | TIMESTAMPTZ | YES | NULL | |

- Unique: `referee_player_id` — BR-06-3: a referee is credited to exactly
  one referral ever.
- Check: `status IN (...)`; `(status = 'converted') =
  (first_purchase_payment_record_id IS NOT NULL AND first_purchase_at IS
  NOT NULL)`; `referrer_player_id <> referee_player_id` — BR-06-3
  self-referral block (all raw `DB::statement`).
- Indexes: `(referrer_player_id, trainer_id)`; `(status) WHERE status =
  'pending'`.
- **No `rewarded` status**, even though the epic's own Data Requirements
  lists Pending/Converted/Rewarded. Whether a reward was actually granted is
  a fact about the ledger (a `token_entries` row with `kind =
  'referral_reward'` and `referral_id` pointing here), not about this row —
  a redundant `rewarded` flag would be a second place that fact could drift
  from the ledger. Recorded in Decisions.

**Model**: `use BelongsToTenant;` `$fillable = ['referral_link_id',
'referrer_player_id', 'referee_player_id', 'clicked_at', 'registered_at']`;
`public $timestamps = false;`; casts `'status' => ReferralStatus::class`,
`'clicked_at' => 'datetime'`, `'registered_at' => 'datetime'`,
`'first_purchase_at' => 'datetime'`; `belongsTo(ReferralLink::class)`;
`belongsTo(PlayerProfile::class, 'referrer_player_id')`,
`belongsTo(PlayerProfile::class, 'referee_player_id')`. RLS: standard
policy.

#### `referral_assist_counts` — was `referral_assist_count`

Per (player, trainer), never global. BR-06-4/5/6/7.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `player_id` | BIGINT | NO | | FK → `player_profiles(id)` RESTRICT |
| `assist_count` | INTEGER | NO | `0` | Resets to 0 (not deleted) each time it crosses the platform threshold and a reward fires |
| `updated_at` | TIMESTAMPTZ | NO | | |

Unique: `(trainer_id, player_id)`. Check: `assist_count >= 0`. Updated
transactionally by the same Action that inserts the `referral_reward`
`token_entries` row, so the counter reset and the ledger grant never
disagree about whether a threshold was crossed. **Model**: `use
BelongsToTenant;` `$fillable = ['player_id', 'assist_count']`;
`belongsTo(PlayerProfile::class)`. RLS: standard policy.

#### `coupons` — was `coupon`

BR-06-8/9/10/11.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `code` | VARCHAR(50) | NO | | Case-sensitive — see Open questions |
| `discount_type` | VARCHAR(16) | NO | | `percentage`\|`fixed` |
| `discount_value` | INTEGER | NO | | Percentage points (1–100) or minor units, by type |
| `applies_to` | VARCHAR(16) | NO | | `events`\|`content`\|`both` |
| `usage_limit` | INTEGER | YES | NULL | |
| `usage_count` | INTEGER | NO | `0` | |
| `eligibility` | VARCHAR(24) | NO | `'any_player'` | `any_player`\|`new_players_only` — Q-06.10 default |
| `expires_at` | TIMESTAMPTZ | YES | NULL | |
| `is_active` | BOOLEAN | NO | `true` | Auto-set false when `usage_count` reaches `usage_limit` |
| `created_by_account_id` | BIGINT | NO | | FK → `accounts(id)` RESTRICT |
| `created_at` | TIMESTAMPTZ | NO | | |

- Unique: `(trainer_id, code)` — BR-06-11: scoped per trainer, not global.
- Check: `discount_type IN (...)`; `(discount_type = 'percentage' AND
  discount_value BETWEEN 1 AND 100) OR (discount_type = 'fixed' AND
  discount_value > 0)`; `applies_to IN (...)`; `eligibility IN (...)`;
  `usage_limit IS NULL OR usage_count <= usage_limit` (raw
  `DB::statement`).
- Index: `(trainer_id, code) WHERE is_active`.

**Model**: `use BelongsToTenant;` `$fillable = ['code', 'discount_type',
'discount_value', 'applies_to', 'usage_limit', 'eligibility', 'expires_at',
'created_by_account_id']`; `public $timestamps = false;` (`created_at`
only); casts `'discount_type' => CouponDiscountType::class`, `'applies_to'
=> CouponAppliesTo::class`, `'eligibility' => CouponEligibility::class`,
`'expires_at' => 'datetime'`, `'is_active' => 'boolean'`. RLS: standard
policy.

#### `coupon_redemptions` — was `coupon_redemption`

BR-06-9/10.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `coupon_id` | BIGINT | NO | | FK → `coupons(id)` RESTRICT |
| `player_id` | BIGINT | NO | | FK → `player_profiles(id)` RESTRICT |
| `payment_record_id` | BIGINT | NO | | FK → `payment_records(id)` RESTRICT, UNIQUE |
| `original_price_minor_units` | INTEGER | NO | | |
| `discount_amount_minor_units` | INTEGER | NO | | |
| `final_price_minor_units` | INTEGER | NO | | |
| `redeemed_at` | TIMESTAMPTZ | NO | `now()` | |

- Unique: `payment_record_id` — BR-06-10: only one coupon per transaction,
  enforced structurally.
- Check: `discount_amount_minor_units >= 0`; `final_price_minor_units =
  original_price_minor_units - discount_amount_minor_units`;
  `final_price_minor_units >= 0` — BR-06-9's floor-at-$0 rule, a real
  constraint, not just application logic (raw `DB::statement`).
- Index: `(coupon_id)` — usage-count reconciliation.
- The source's three `*_price_minor_units` columns as one value-object
  grouping become a `Discount` custom cast here, same pattern as
  `EmergencyContact`/`DualPricing`, exposing the floor-at-zero rule (BR-06-9)
  as a named constructor on the value object rather than three
  independently-settable properties:

```php
final class Discount implements CastsAttributes
{
    public function get(Model $model, string $key, mixed $value, array $attributes): DiscountValue
    {
        return new DiscountValue(
            originalPriceMinorUnits: $attributes['original_price_minor_units'],
            discountAmountMinorUnits: $attributes['discount_amount_minor_units'],
            finalPriceMinorUnits: $attributes['final_price_minor_units'],
        );
    }

    public function set(Model $model, string $key, mixed $value, array $attributes): array
    {
        return [
            'original_price_minor_units' => $value->originalPriceMinorUnits,
            'discount_amount_minor_units' => $value->discountAmountMinorUnits,
            'final_price_minor_units' => $value->finalPriceMinorUnits,
        ];
    }
}
```

`DiscountValue::apply(Money $original, Coupon $coupon): self` is the named
constructor that computes the floor-at-zero rule once, at redemption time —
the CHECK constraints on the three columns are the backstop, not the only
place the rule is expressed.

**Model**: `use BelongsToTenant;` `$fillable = ['coupon_id', 'player_id',
'payment_record_id']`; `public $timestamps = false;` (`redeemed_at` only);
cast `'discount' => Discount::class` (combining the three price columns);
`belongsTo(Coupon::class)`, `belongsTo(PlayerProfile::class)`,
`belongsTo(PaymentRecord::class)`. RLS: standard policy.

### Forms module (Epic-08)

#### `forms` — was `form`

Camp or evaluation, one shared shape. BR-08-1 through BR-08-5.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)` |
| `form_type` | VARCHAR(16) | NO | | `camp`\|`evaluation` |
| `name` | VARCHAR(255) | NO | | |
| `description` | TEXT | YES | NULL | |
| `price_minor_units` | INTEGER | YES | NULL | NULL/0 = free — BR-08-4 |
| `capacity_limit` | INTEGER | YES | NULL | Camps only (BR-08-3) — 1–1000, AC-08-4 |
| `is_active` | BOOLEAN | NO | `true` | Camps only — evaluations "always on" and ignore this column |
| `field_definitions` | JSONB | NO | `'[]'` | Array of `{id, type, label, required, options?}` |
| `shareable_slug` | VARCHAR(64) | NO | | High-entropy, application-generated |
| `created_at` / `updated_at` | TIMESTAMPTZ | NO | | |

- Unique: `shareable_slug` — globally. Same pre-tenant-resolution caveat as
  `share_links.code`: form publication calls
  `PublicTenantCodeRegistry::issue($shareableSlug, $trainerId, 'form',
  $id)` in the same transaction as persisting the `Form` row.
- Check: `form_type IN (...)`; `price_minor_units IS NULL OR
  price_minor_units >= 0`; `capacity_limit IS NULL OR (form_type = 'camp'
  AND capacity_limit BETWEEN 1 AND 1000)` — evaluations carry no capacity
  at all, not just an ignored one; the CHECK makes that structural (raw
  `DB::statement`).
- Index: `(trainer_id, form_type)`.
- **`field_definitions` as JSONB, not a child table** — field types are a
  closed 4-value set (BR-08-2), never queried individually anywhere in the
  epic; a normalized `form_fields` table would add joins for zero query
  benefit. Recorded in Decisions.
- Capacity enforcement mirrors `events`: `Form::query()->lockForUpdate()->findOrFail($id)`
  inside `DB::transaction()`, then `COUNT(*)` of confirmed `form_submissions`
  against `capacity_limit`.

**Model**: `use BelongsToTenant;` `$fillable = ['form_type', 'name',
'description', 'price_minor_units', 'capacity_limit', 'is_active',
'field_definitions']`; cast `'form_type' => FormType::class`,
`'field_definitions' => 'array'`, `'is_active' => 'boolean'`;
`hasMany(FormSubmission::class)`. RLS: standard policy.

#### `form_submissions` — was `form_submission`

Public, unauthenticated write — **the only one in the platform**.
Trainer-scoped, matching the source's explicit correction of the tenancy
manifest's own header comment (which listed this table as global — the
source flags the contradiction and follows the settled architecture; ported
here as settled, not reopened). BR-08-6 through BR-08-18.

| Column | Type | Null | Default | Notes |
|---|---|---|---|---|
| `id` | BIGINT IDENTITY | NO | | PK |
| `trainer_id` | BIGINT | NO | | FK → `trainers(id)`. Resolved from `forms.trainer_id` at write time via the form's `shareable_slug` — the *only* trainer-scoped row this schema lets an unauthenticated request write |
| `form_id` | BIGINT | NO | | FK → `forms(id)` RESTRICT |
| `submission_data` | JSONB | NO | | Field-id → answer map, shape driven by `forms.field_definitions` |
| `contact_email` | CITEXT | NO | | Denormalized off `submission_data` for the uniqueness check and `payment_records.contact_email` linkage |
| `payment_status` | VARCHAR(16) | NO | `'free'` | `free`\|`paid`\|`pending` |
| `payment_record_id` | BIGINT | YES | NULL | FK → `payment_records(id)` — attached in the Billing stage from the Forms side (`payment_records.related_form_submission_id` is the deferred one; this column's own FK direction attaches once `payment_records` exists, same stage) |
| `submitted_at` | TIMESTAMPTZ | NO | `now()` | |
| `converted_account_id` | BIGINT | YES | NULL | FK → `accounts(id)` RESTRICT. Set on conversion (A5) |
| `converted_at` | TIMESTAMPTZ | YES | NULL | |

- Unique: `(form_id, contact_email)` — BR-08-10: the same email cannot
  submit the same form twice.
- Check: `payment_status IN (...)`; `(converted_account_id IS NULL) =
  (converted_at IS NULL)` (raw `DB::statement`).
- Indexes: `(trainer_id, form_id, payment_status)` — the trainer's
  participant list, filterable by payment status (AC-08-18); `(form_id,
  converted_account_id)`.
- **A3/A4, made concrete**: a camp registrant who never converts leaves
  `converted_account_id NULL` forever, and their `payment_records.
  payer_account_id` is also `NULL` — fully functional (visible in the
  participant list, refundable via its payment record) with no `Account`
  anywhere in the chain. On conversion, `converted_account_id` is set and —
  per A5 — the *existing* `payment_records.payer_account_id` is additively
  updated to point at the new account (a plain `update()`, since
  `payment_records` is not append-only), so transaction history picks up
  the historical camp payment without re-creating it.
- **RLS applies to this table exactly as it does to every other
  trainer-scoped table**, including on the unauthenticated write path: the
  public form controller resolves `trainer_id` via `ResolveTenant`'s
  public-code source from `forms.shareable_slug` before the `INSERT` —
  there is no separate, unscoped code path for this table. This is the one
  route in the entire schema where `ResolveTenant`'s public-code source
  (source 5) resolves a tenant for a **write**, not just a read; the
  `tenant.public-code` middleware group covers it identically to the
  ShareLink routes.

**Model** `App\Models\FormSubmission`: `use BelongsToTenant;` `$fillable =
['form_id', 'submission_data', 'contact_email', 'converted_account_id']`;
`public $timestamps = false;` (`submitted_at` only); casts
`'submission_data' => 'array'`, `'payment_status' =>
FormSubmissionPaymentStatus::class`, `'converted_at' => 'datetime'`;
`belongsTo(Form::class)`, `belongsTo(PaymentRecord::class)`,
`belongsTo(Account::class, 'converted_account_id')`. RLS: standard policy.

---

## Row-Level Security

Applies to every table in "Trainer-scoped tables" above except `drill_details`
(no `trainer_id` of its own) and to none in "Global tables". Ported
unchanged from `specs/database-designer-schema.md` "Row-Level Security" —
this entire section is server-side PostgreSQL SQL, outside any
application framework, so there is nothing framework-specific to
re-express here. What changes is *how* the SQL gets run (a Laravel
migration helper instead of the source tool's migration) and *how* the
session variable gets set (`ResolveTenant`
middleware instead of `TenantResolver`, both already covered under
"Tenancy" above).

### Sentinel

`ResolveTenant` sets a PostgreSQL session variable on every request,
resolved or not (see the Octane note above for why the unresolved branch is
not optional):

```sql
SET app.current_trainer_id = '<trainer.id>';   -- a resolved tenant
SET app.current_trainer_id = '0';              -- unresolved — matches no real trainer, since identity PKs start at 1
```

Session-scoped `SET`, not `SET LOCAL` — not every unit of work in a Laravel
request runs inside an explicit `DB::transaction()`.

### Standard policy — reusable migration helper

Rather than repeat the same two SQL statements in 37 separate migrations,
this document defines one trait every RLS-bearing migration uses:

```php
<?php

declare(strict_types=1);

namespace Database\Migrations\Concerns;

use Illuminate\Support\Facades\DB;

trait AppliesRowLevelSecurity
{
    protected function enableTenantIsolation(string $table): void
    {
        DB::statement("ALTER TABLE {$table} ENABLE ROW LEVEL SECURITY");

        DB::statement(<<<SQL
            CREATE POLICY tenant_isolation ON {$table}
                USING (trainer_id = current_setting('app.current_trainer_id')::bigint)
                WITH CHECK (trainer_id = current_setting('app.current_trainer_id')::bigint)
            SQL);
    }

    protected function disableTenantIsolation(string $table): void
    {
        DB::statement("DROP POLICY IF EXISTS tenant_isolation ON {$table}");
    }
}
```

Every "RLS: standard policy" table above has a migration that `use`s this
trait and calls `$this->enableTenantIsolation('coach_memberships');`
(substituting its own table name) immediately after `Schema::create(...)`,
and its `down()` calls `disableTenantIsolation()` before
`Schema::dropIfExists(...)` — Postgres drops policies automatically with the
table, so this is usually a no-op, but a migration that *alters* an
RLS-bearing table rather than dropping it must not assume the same, per the
source's own rollback note. `FORCE ROW LEVEL SECURITY` is deliberately
never set, so `pp_owner` can migrate and backfill without fighting its own
policies.

### Widened policy — `playlists`, `content_items`

The one declared widening of tenant isolation: reads widen on "has ever
been published"; writes stay strictly tenant-owned. Ported unchanged —
four named policies, each with the `USING`/`WITH CHECK` combination that
operation actually needs (`WITH CHECK` governs rows being written, so it
has no SELECT/DELETE form; `USING` governs rows being read or matched, so
it has no bare-INSERT form):

```sql
ALTER TABLE playlists ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation_select ON playlists
    FOR SELECT
    USING (
        trainer_id = current_setting('app.current_trainer_id')::bigint
        OR ever_published_at IS NOT NULL
    );

CREATE POLICY tenant_isolation_write ON playlists
    FOR INSERT
    WITH CHECK (trainer_id = current_setting('app.current_trainer_id')::bigint);

CREATE POLICY tenant_isolation_update ON playlists
    FOR UPDATE
    USING (trainer_id = current_setting('app.current_trainer_id')::bigint)
    WITH CHECK (trainer_id = current_setting('app.current_trainer_id')::bigint);

CREATE POLICY tenant_isolation_delete ON playlists
    FOR DELETE
    USING (trainer_id = current_setting('app.current_trainer_id')::bigint);
```

Identical four-policy block for `content_items` (substitute the table
name). Added to the `AppliesRowLevelSecurity` migration trait as a second
method so both migrations call one line instead of inlining this block
twice:

```php
protected function enableWidenedPublicationIsolation(string $table): void
{
    DB::statement("ALTER TABLE {$table} ENABLE ROW LEVEL SECURITY");

    DB::statement(<<<SQL
        CREATE POLICY tenant_isolation_select ON {$table}
            FOR SELECT
            USING (
                trainer_id = current_setting('app.current_trainer_id')::bigint
                OR ever_published_at IS NOT NULL
            )
        SQL);

    DB::statement(<<<SQL
        CREATE POLICY tenant_isolation_write ON {$table}
            FOR INSERT
            WITH CHECK (trainer_id = current_setting('app.current_trainer_id')::bigint)
        SQL);

    DB::statement(<<<SQL
        CREATE POLICY tenant_isolation_update ON {$table}
            FOR UPDATE
            USING (trainer_id = current_setting('app.current_trainer_id')::bigint)
            WITH CHECK (trainer_id = current_setting('app.current_trainer_id')::bigint)
        SQL);

    DB::statement(<<<SQL
        CREATE POLICY tenant_isolation_delete ON {$table}
            FOR DELETE
            USING (trainer_id = current_setting('app.current_trainer_id')::bigint)
        SQL);
}
```

`drill_details` needs no policy of its own — no `trainer_id` column,
reached only through `content_items`.

### Grants

`00_roles.sql` (above) already sets `ALTER DEFAULT PRIVILEGES` so every new
table automatically grants `pp_app` full DML and `pp_crossing` `SELECT`
only. Two migrations narrow that immediately after creating their table —
already shown in full above: `token_entries` (`REVOKE UPDATE, DELETE ...
FROM pp_app`) and `audit_log_entries` (same). This is what turns I7 and
BR-07-6's implicit append-only requirement into a database privilege rather
than a code convention.

### Tenancy configuration and the startup verification gate

The source's `Task/app/config/tenancy/trainer_scoped_tables.txt` and
`resolver_global_tables.txt` become one Laravel config file,
`config/tenancy.php` — new infrastructure this document originates (no
existing entrypoint script to hook into in this repo yet; see Decisions and
Open questions for wiring it into an actual deploy/boot step):

```php
<?php

declare(strict_types=1);

return [
    // Every table here must carry the standard or widened RLS policy.
    // Checked by `php artisan tenancy:verify`.
    'trainer_scoped_tables' => [
        'trainer_billing_settings', 'coach_memberships', 'player_trainer_memberships',
        'share_links', 'share_link_opens', 'availability_windows', 'child_approval_requests',
        'events', 'rsvps', 'attendance_records', 'attendance_edits', 'coach_assignments',
        'coach_availability_overrides', 'event_invitations', 'event_duplication_records',
        'labels', 'player_labels', 'player_flags', 'player_notes',
        'playlists', 'content_items', 'playlist_items', 'playlist_assignments',
        'content_progress', 'playlist_access_grants', 'content_usage',
        'token_entries', 'token_balances', 'payment_records', 'token_packages',
        'subscription_entitlements', 'entitlement_coverages',
        'referral_links', 'referrals', 'referral_assist_counts', 'coupons', 'coupon_redemptions',
        'forms', 'form_submissions',
    ],

    // These two must NEVER carry RLS — the resolver reads them before a
    // tenant exists. A policy on either breaks every login and every
    // public-code route, silently (RLS returns zero rows, not an error).
    'resolver_global_tables' => [
        'account_trainer_links',
        'public_tenant_codes',
    ],
];
```

`php artisan tenancy:verify` (`app/Console/Commands/VerifyTenancyRls.php`)
— the direct translation of the source's container-startup Gate 4
(`docker-entrypoint.sh`), reading `pg_class`/`pg_policy` instead of parsing
a `.txt` file against the source framework's schema metadata:

```php
final class VerifyTenancyRls extends Command
{
    protected $signature = 'tenancy:verify';

    public function handle(): int
    {
        $failed = false;

        foreach (config('tenancy.trainer_scoped_tables') as $table) {
            $hasRls = DB::selectOne(
                'SELECT relrowsecurity FROM pg_class WHERE relname = ?',
                [$table],
            )?->relrowsecurity;

            if (! $hasRls) {
                $this->error("{$table} is listed as trainer-scoped but has no RLS policy enabled.");
                $failed = true;
            }
        }

        foreach (config('tenancy.resolver_global_tables') as $table) {
            $hasRls = DB::selectOne(
                'SELECT relrowsecurity FROM pg_class WHERE relname = ?',
                [$table],
            )?->relrowsecurity;

            if ($hasRls) {
                $this->error("{$table} is a resolver global table but has RLS enabled — this breaks every login.");
                $failed = true;
            }
        }

        return $failed ? self::FAILURE : self::SUCCESS;
    }
}
```

Run at deploy time (before traffic is routed to a new release) — this
document proposes the command; wiring it into a specific deploy pipeline or
health check is out of scope for a database design and is flagged in Open
questions, matching how the source's own equivalent lived in a Docker
entrypoint this repo has no equivalent of yet.

---

## The token and payment ledger

Restates invariants I1–I7 exactly as the source states them — these are
product/correctness invariants, not framework facts — pointing at where each
is enforced in this Laravel port (full mechanism under `token_entries`/
`token_balances` above; this is the index).

| Invariant | Statement | Enforced by |
|---|---|---|
| I1 | Balance projection = sum of that pair's entries | Not a DB constraint (spans two tables). `TokenLedgerService` maintains both in one `DB::transaction()`; a scheduled Artisan command (`php artisan ledger:reconcile`) plus a test verify it periodically |
| I2 | Balance never negative | `CHECK (balance >= 0)` on `token_balances` |
| I3 | Every `spend` records the beneficiary; every `refund` inherits it | CHECK requires the column on `spend`/`refund`; inheritance is `TokenLedgerService::refund()`, covered by a test |
| I4 | Sum of refunds against one spend never exceeds it | Lock the spend row (`->lockForUpdate()`, no write), then sum + compare, inside `DB::transaction()` — cannot be a running counter because I7 forbids `UPDATE` |
| I5 | Sign matches kind | CHECK on `token_entries.amount` vs. `kind` |
| I6 | At most one entry per (payment record, purpose) | Partial unique index `(payment_record_id, kind) WHERE payment_record_id IS NOT NULL` |
| I7 | No entry ever updated or deleted | `REVOKE UPDATE, DELETE ON token_entries FROM pp_app` |

### Lock ordering, as Action/Service methods

The fixed order — **token balance row, then event row** — is made
structural by funnelling every dual-lock path through two methods that can
only be called in that order (Laravel has no separate data-access layer to
mirror, so these live on the Action/Service classes that own the
transaction, per this repo's own
`.claude/skills/database-designer/SKILL.md` — "Eloquent models or the
query builder" *are* the access layer):

- `TokenLedgerService::lockBalance(trainerId, parentAccountId): TokenBalance`
  — always called first (shown in full under `token_balances` above).
- `Event::query()->lockForUpdate()->findOrFail($eventId)` — always called
  second, only from inside a transaction that already holds the balance
  lock. Wrapped as `EventLockingService::lock(eventId): Event` so every
  call site goes through one place that cannot get the order wrong, same
  reasoning as the balance lock.

Both a paid RSVP and an RSVP-cancellation refund go through
`TokenLedgerService`/an `RsvpService` Action in that order; a test drives
both concurrently against the same parent and event and asserts no
deadlock. Trainer-initiated cancellation never holds both locks at once: it
locks and commits the event cancel first, then refunds each player
asynchronously — one Laravel queued Job per RSVP, each job taking only that
one balance lock — the direct translation of the source's "fan-out... one
transaction per RSVP" worker pattern (Symfony Messenger → Laravel Queues,
per `specs/MANIFEST.md`'s own list of Laravel equivalents).

---

## Money and the platform fee

Integer minor units throughout. This section fixes the rounding rule for
the 5% platform fee, since the trainer absorbs it and it is included in the
listed price (BR-05-7).

### Formula

```
platform_fee_minor_units = FLOOR(
    (amount_minor_units * fee_rate_basis_points + 5000) / 10000
)
```

Round half up to the nearest minor unit (cent), integer arithmetic only —
no floating point anywhere near a money figure. Identical PHP either way
(this formula has no framework-specific dimension to translate):

```php
<?php

declare(strict_types=1);

namespace App\Support\Billing;

final class PlatformFeeCalculator
{
    public static function forAmount(int $amountMinorUnits, int $feeRateBasisPoints): int
    {
        return intdiv($amountMinorUnits * $feeRateBasisPoints + 5000, 10000);
    }
}
```

### Independently re-verified against both worked examples in the epic

Re-computed from scratch for this document, not copied from the source's
own verification, per this task's explicit "verify rather than trust"
instruction:

- **BR-05-7**: $100 event = 10,000 minor units, 500 bps (5%).
  `(10000 * 500 + 5000) / 10000 = (5,000,000 + 5,000) / 10000 =
  5,005,000 / 10000 = 500.5` → `floor` → **500 → $5.00 exactly**, matching
  "the $5 (5%) platform fee" stated in the epic.
- **AC-05-11**: $20 charge = 2,000 minor units, 500 bps.
  `(2000 * 500 + 5000) / 10000 = (1,000,000 + 5,000) / 10000 =
  1,005,000 / 10000 = 100.5` → `floor` → **100 → $1.00 exactly**, matching
  "a platform fee of $1 (5%)".
- **A genuine half-cent case** (10-cent amount, 5%):
  `(10 * 500 + 5000) / 10000 = (5,000 + 5,000) / 10000 = 10,000 / 10000 =
  1` exactly → an exact 0.5-cent fee rounds **up** to 1 cent, confirming
  round-half-up (away from zero), not banker's rounding — the same
  conclusion the source reaches, reproduced independently here.
- `intdiv()` truncates toward zero; for the non-negative operands this
  formula always receives (charge rows only, never negative amounts), that
  is identical to `floor()`.

### Where it runs

Computed once, at `PaymentRecord` creation, from
`trainer_billing_settings.platform_fee_basis_points` **read at that
moment** and snapshotted into `payment_records.fee_rate_basis_points` and
`payment_records.platform_fee_minor_units`. AC-05-27 changes the rate for
new transactions only — a rate read live at *display* time would
retroactively rewrite every historical figure, which is exactly what the
snapshot prevents.

`platform_fee_minor_units` is sent to Stripe as the Connect charge's
`application_fee_amount` — a request parameter the platform computes and
holds, not a reporting figure, so storing it does not conflict with the
earnings boundary below.

**Trainer net/payout is never computed or stored here.** Stripe deducts its
own processing fee independently and that figure is never mirrored
locally — `StripeReportingReader` (`app/Services/StripeReportingReader.php`)
is the only source of any earnings/payout figure shown to a trainer, read
live from the Stripe API, never cached, never derived by summing
`payment_records`.

---

## Eloquent mapping notes

Direct counterpart to the source's own mapping-notes section — everything
there that was a framework-specific fact (its underscore naming strategy,
its identity-column preference) is replaced by an equivalent Laravel fact;
everything that was a general mapping *decision* (shared-PK extension,
value-object groupings, enum encoding, DTO projections) is re-expressed in
Eloquent terms, already shown per-table above. This section is the index.

### Backed enums, one per closed vocabulary

Every `VARCHAR` + `CHECK (col IN (...))` column above maps to a native PHP
8.1+ backed enum, cast via Eloquent's built-in enum cast (`'col' =>
SomeEnum::class` in `casts()`) — no custom cast class, per this repo's
`eloquent` skill. Two full definitions, representative of the rest:

```php
<?php

declare(strict_types=1);

namespace App\Enums;

enum AccountRole: string
{
    case SuperAdmin = 'super_admin';
    case Trainer = 'trainer';
    case Coach = 'coach';
    case Player = 'player';
}

enum TokenEntryKind: string
{
    case Purchase = 'purchase';
    case Gift = 'gift';
    case ReferralReward = 'referral_reward';
    case Refund = 'refund';
    case Spend = 'spend';
    case Adjustment = 'adjustment';
}
```

The remaining ~33, same pattern, one enum class per row below (`app/Enums/*`):

| Enum | Values | Columns |
|---|---|---|
| `AccountStatus` | `active`, `inactive`, `deleted` | `accounts.status` |
| `TenantRole` | `trainer`, `coach`, `player` | `account_trainer_links.role_in_tenant` |
| `LinkStatus` | `active`, `inactive` | `account_trainer_links.status` |
| `PublicTenantCodeKind` | `sharelink`, `form` | `public_tenant_codes.kind` |
| `ImpersonationEndedReason` | `manual`, `expired` | `impersonation_sessions.ended_reason` |
| `FeatureName` | `lppp`, `marketing`, `camps` | `feature_toggles.feature_name` |
| `PlatformSubscriptionStatus` | `pending`, `active`, `past_due`, `suspended`, `canceled` | `platform_subscriptions.status` |
| `StripeConnectOnboardingStatus` | `pending`, `complete`, `incomplete` | `trainer_billing_settings.stripe_connect_onboarding_status` |
| `PayoutSchedule` | `monthly`, `weekly` | `trainer_billing_settings.payout_schedule` |
| `CoachMembershipStatus` | `pending`, `active`, `declined` | `coach_memberships.status` |
| `MembershipSource` | `sharelink`, `event_registration`, `coach_invite`, `camp_registration` | `player_trainer_memberships.source` |
| `MembershipStatus` | `active`, `removed` | `player_trainer_memberships.status` |
| `ShareLinkType` | `static_player`, `unique_coach` | `share_links.link_type` |
| `AvailabilityOwnerType` | `coach`, `player` | `availability_windows.owner_type` |
| `ChildApprovalActionType` | `rsvp`, `token_purchase`, `content_purchase` | `child_approval_requests.action_type` |
| `ChildApprovalStatus` | `pending`, `approved`, `denied` | `child_approval_requests.status` |
| `EventType` | `training_session`, `private_session`, `small_group` | `events.event_type` |
| `EventVisibility` | `public`, `private` | `events.visibility` |
| `EventStatus` | `active`, `canceled`, `completed` | `events.status` |
| `RsvpStatus` | `pending_parent_approval`, `pending_payment`, `confirmed`, `canceled` | `rsvps.status` |
| `RsvpPaymentMethod` | `token`, `usd`, `free` | `rsvps.payment_method` |
| `AttendanceStatus` | `present`, `absent`, `late`, `excused` | `attendance_records.status`, `attendance_edits.old_status`/`new_status` |
| `CoachAssignmentStatus` | `pending`, `confirmed`, `declined` | `coach_assignments.status` |
| `PlayerFlagType` | `behavior`, `high_no_show_rate`, `injured`, `medical_restriction`, `scholarship`, `financial_aid`, `contact_priority`, `attendance_risk` | `player_flags.flag_type` |
| `PlayerFlagStatus` | `active`, `resolved` | `player_flags.status` |
| `PlayerNoteType` | `general`, `session` | `player_notes.note_type` |
| `ContentPillar` | `learn`, `practice` | `playlists.pillar`, `content_items.pillar` |
| `ContentType` | `video`, `drill` | `content_items.type` |
| `PlaylistAudience` | `players_and_coaches`, `coaches_only` | `playlists.audience` |
| `DrillDifficulty` | `beginner`, `intermediate`, `advanced`, `elite` | `drill_details.difficulty_level` |
| `DrillSpaceRequirement` | `small`, `medium`, `large` | `drill_details.space_requirement` |
| `ContentProgressStatus` | `not_started`, `in_progress`, `completed` | `content_progress.status` |
| `PlaylistAssignmentTargetType` | `player`, `label`, `skill_level` | `playlist_assignments.target_type` |
| `PaymentRecordType` | `token_purchase`, `event_rsvp`, `content_purchase`, `player_subscription`, `camp_registration` | `payment_records.type` |
| `PaymentMethod` | `token`, `card` | `payment_records.payment_method` |
| `PaymentRecordStatus` | `pending`, `completed`, `failed`, `refunded` | `payment_records.status` |
| `ReferralStatus` | `pending`, `converted` | `referrals.status` |
| `CouponDiscountType` | `percentage`, `fixed` | `coupons.discount_type` |
| `CouponAppliesTo` | `events`, `content`, `both` | `coupons.applies_to` |
| `CouponEligibility` | `any_player`, `new_players_only` | `coupons.eligibility` |
| `FormType` | `camp`, `evaluation` | `forms.form_type` |
| `FormSubmissionPaymentStatus` | `free`, `paid`, `pending` | `form_submissions.payment_status` |

### Shared-PK 1-to-1 extensions

`account_profiles`, `trainer_billing_settings`, `trainer_branding_settings`,
`drill_details` all use the same pattern: `protected $primaryKey = 'x_id';
public $incrementing = false;`, `belongsTo` back to the parent. This is the
direct Eloquent equivalent of the source's own owning-side, shared-key
mapping — each still has its own model and its own lifecycle, unlike a
true embeddable, matching the source's own distinction.

### Custom casts replace the source's value-object groupings

`EmergencyContact` (`player_profiles`), `EligibilityCriteria`/`DualPricing`
(`events`), `Discount` (`coupon_redemptions`) — small, always-loaded value
groups spanning sibling columns, none independently queried, FK-referenced,
or RLS subjects. Full definitions shown per-table above. This is the one
place Eloquent's mechanism is *more* code than the source's declarative
value-object mapping (a hand-written `get()`/`set()` pair per group vs. one
declarative attribute), named explicitly rather than glossed over.

### Read-heavy screens: query-builder projections, not model hydration

Per this task's own instruction ("where a read-heavy screen should use a
query-builder projection instead of hydrating models"), the following use
`toBase()` (executes the Eloquent query but returns a plain query-builder
`Collection` of `stdClass` rows — no model hydration, no relation loading,
no cast overhead) or `DB::table()` directly, exactly mirroring the source's
"DTO projections... explicitly permitted to use native SQL... because RLS
covers native SQL just as it covers Eloquent-generated SQL":

- **CRM segmentation** (BR-03-13/14, AND-combined filters over
  `player_trainer_memberships` + `attendance_records` + `player_labels` +
  `player_flags`) —
  `App\Services\PlayerSegmentationService::search(SegmentCriteria $criteria): Collection`,
  built on `DB::table('player_trainer_memberships')->join(...)`, never
  hydrates full `PlayerProfile` graphs for a screen that shows a row of
  badges and numbers.
- **Top Players** (BR-03-16/17, 90-day attendance) —
  `App\Services\AttendanceReportingService::topPlayers(int $trainerId, int
  $limit = 10)`, a single aggregate query over the `(trainer_id, player_id,
  status, recorded_at)` index on `attendance_records`, via
  `AttendanceRecord::query()->toBase()->...`.
- **Epic-07 dashboards** (BR-07-7/8/9) —
  `App\Services\DashboardMetricsService`, entirely query-builder-projected,
  entirely live-queried (no metrics-storage table exists in this
  schema — see Open questions on the epic's own live-vs-batch
  contradiction).
- **Transaction history** (AC-05-23) —
  `PaymentRecord::query()->toBase()->where(...)->paginate(...)`, filterable,
  never hydrates `PaymentRecord` models for a list screen.
- **Event Master Tool listing** (AC-07-23/24/26) —
  `Event::query()->toBase()->...`, 50/page via `paginate(50)`.

### Named Action/Service methods worth listing now

Laravel has no separate data-access layer to name methods on — the direct
counterparts to the source's own "worth naming now" list are model query
scopes for simple filters and Action/Service classes for anything
transactional, so a later implementation stage does not invent a
differently-shaped equivalent:

- `TokenLedgerService::lockBalance(trainerId, parentAccountId): TokenBalance`
- `EventLockingService::lock(eventId): Event`
- `Event::confirmedRsvpCount(int $eventId): int` (called under the same lock)
- `TokenLedgerService::sumRefundsAgainst(int $spendEntryId): int` (after
  locking the spend row)
- `php artisan ledger:reconcile` — I1's verification query, used by both a
  scheduled run and its test
- `ContentItem::findAccessible(int $trainerId, int $id): ?ContentItem` —
  applies the widened publication predicate explicitly for the one code
  path that legitimately needs it outside RLS's own enforcement (defense in
  depth, mirroring why Layer 4 exists alongside Layer 5)
- `SubscriptionEntitlement::activeFor(trainerId, parentAccountId, onDate)`
  — local scope
- `PlaylistAssignment::resolvedStatusFor(playerId, playlistId)` — the
  read-time join described under that table
- `CoachVisibilityService::reachablePlayerIds(coachMembershipId)` —
  backing every coach-facing query above; not itself new schema
- `PublicTenantCode::query()->where('code', $code)->value('trainer_id')`
  — the one query `ResolveTenant` runs with no tenant set, backing
  `PublicTenantCodeRegistry`

---

## Migration ordering

Respects build order 01 → 02 → {03, 04} → 05 → {06, 07, 08} (A10) exactly.
Laravel runs `database/migrations/*.php` in filename order — there is no
separate "batch" abstraction the way the source's migration tool has, so
this document encodes the same M0–M7 staging directly into migration
filename timestamps, one date-bucket per stage.

Session/cache/queue tables (`sessions`, `cache`, `jobs`, `failed_jobs` —
Laravel's own equivalents of the source's PDO session handler / cache pool
/ lock store) are **out of scope for these migrations**, same as the
source — created by `php artisan session:table` / `cache:table` /
`queue:table` if the corresponding drivers are database-backed, not by
anything in this schema.

### M0 — Bootstrap (no tables)

`2025_01_01_000001_enable_postgres_extensions.php`:

```php
public function up(): void
{
    DB::statement('CREATE EXTENSION IF NOT EXISTS citext');
    DB::statement('CREATE EXTENSION IF NOT EXISTS btree_gist');
}
```

Role creation (`database/pgsql/00_roles.sql`) is run once, separately, by
whoever provisions the database — not a Laravel migration, same separation
of concerns as the source's `00-roles.sh`.

### M1 — Identity + Platform core (Epic-01). The walking skeleton.

**The four MVP roles must be able to log in as soon as this batch lands**,
before any other epic exists: `accounts`, `account_profiles`, `trainers`,
`account_trainer_links` — sufficient for Super Admin to create a Trainer
account and for both to authenticate (BR-01-13, AC-01-1 through AC-01-8).

Everything else Epic-01 needs, same date-bucket
(`2025_01_02_0000NN_create_*_table.php`, NN incrementing):
`parent_child_links`, `player_profiles`, `email_verification_tokens`,
`password_reset_tokens`, `user_deletion_records`, `trainer_branding_settings`,
`public_tenant_codes`, `coach_memberships`, `player_trainer_memberships`,
`share_links`, `share_link_opens`, `availability_windows`,
`child_approval_requests`, `impersonation_sessions`, `audit_log_entries`
(with the `REVOKE` narrowing in the same migration), `feature_toggles`,
`platform_configurations`.

`public_tenant_codes` has no dependency on `share_links` existing first
(`reference_id` carries no FK), so its migration runs before `share_links`'
in this same batch; `share_links`' own migration seeds no rows into it —
`PublicTenantCodeRegistry::issue()` is an application-runtime call from
`ShareLink` creation from here on, not a migration-time concern.

**Deferred FK attachment** (pattern introduced here, reused throughout —
every later occurrence points back to this paragraph): three columns on
`child_approval_requests` (`requested_token_package_id`,
`requested_playlist_id`) reference tables that do not exist until the
Billing/Content stages. They are created here as plain nullable
`unsignedBigInteger` columns with no FK constraint; the FK is attached by a
`Schema::table(...)->foreign(...)` call in the migration that first creates
the target table.

`feature_toggles` is seeded with all three rows (`lppp`, `marketing`,
`camps`), `is_enabled = true`, by the trainer-creation Action for every
trainer created from here on — Epic-07's own migration stage adds no
schema, only the admin screen over rows that already exist.

### M2 — Scheduling (Epic-02)

`2025_01_03_0000NN_*`: `events`, `rsvps` (with `payment_record_id` as a
deferred column — `payment_records` doesn't exist until M4),
`attendance_records`, `attendance_edits`, `coach_assignments`,
`coach_availability_overrides`, `event_invitations`,
`event_duplication_records`.

Attaches a deferred FK from M1 — **not** actually deferred by the time this
batch runs, since `rsvps` is created in this same batch and
`child_approval_requests` already exists from M1:

```php
// 2025_01_03_000009_attach_child_approval_requests_rsvp_foreign.php
Schema::table('child_approval_requests', function (Blueprint $table): void {
    $table->foreign('rsvp_id')->references('id')->on('rsvps')->cascadeOnDelete();
});
```

### M3 — Crm + Content (Epic-03, Epic-04, built in parallel per A10)

`2025_01_04_0000NN_*`, sequence continuous across both halves (no
dependency between them, so relative order within this stage does not
matter):

**Crm**: `labels`, `player_labels`, `player_flags`, `player_notes`.

**Content**: `playlists`, `content_items`, `drill_details`,
`playlist_items`, `playlist_assignments`, `content_progress`,
`playlist_access_grants` (with `payment_record_id` deferred to M4),
`content_usage`.

Attaches the deferred FK from M1: `child_approval_requests.requested_playlist_id`
→ `playlists(id)`.

### M4 — Billing (Epic-05)

`2025_01_05_0000NN_*`: `trainer_billing_settings`, `payment_records`,
`token_entries` (with the `REVOKE` narrowing in the same migration, and
`referral_id` created as a deferred column — `referrals` doesn't exist
until M5), `token_balances`, `token_packages`, `subscription_entitlements`,
`entitlement_coverages`, `platform_subscriptions`, `stripe_customer_links`,
`stripe_event_receipts`.

Attaches every FK deferred from M1–M3, now that both sides exist:
`child_approval_requests.requested_token_package_id` → `token_packages(id)`;
`rsvps.payment_record_id` → `payment_records(id)`;
`playlist_access_grants.payment_record_id` → `payment_records(id)` **NOT
NULL** (tightened from nullable-during-creation now that the column is
guaranteed populated going forward — a fresh environment has no existing
rows to backfill at this point; a populated environment adopting this
schema for the first time would need a backfill step first, noted for
completeness); `payment_records.related_rsvp_id` → `rsvps(id)`;
`payment_records.related_playlist_id` → `playlists(id)`.

### M5 — Growth (Epic-06)

`2025_01_06_0000NN_*`: `referral_links`, `referrals`,
`referral_assist_counts`, `coupons`, `coupon_redemptions`.

Attaches the deferred FK from M4: `token_entries.referral_id` →
`referrals(id)`.

### M6 — Administration (Epic-07)

**No new tables.** Every screen this epic adds — Dashboard, Users tool,
Event Master Tool, Audit Log viewer, per-trainer fee editing — reads or
writes tables that already exist by M4 (`feature_toggles` from M1,
`audit_log_entries` from M1, `events`/`rsvps` from M2,
`trainer_billing_settings` from M4). No migration file for this stage
unless a later performance pass finds an index gap — none predicted here.

### M7 — Forms (Epic-08)

`2025_01_07_0000NN_*`: `forms`, `form_submissions`.

Attaches the deferred FK from M4: `payment_records.related_form_submission_id`
→ `form_submissions(id)`.

### Rollback notes

Every migration above has a `down()` that reverses what `up()` created, in
reverse dependency order within its stage — standard Laravel practice, not
restated per file. Two things need more care than a bare
`Schema::dropIfExists()`:

- **RLS-bearing tables**: `down()` calls `$this->disableTenantIsolation($table)`
  (the trait method above) before dropping — Postgres drops policies
  automatically with the table, so this is usually a no-op, but a migration
  that narrows/widens a policy on an existing table (rather than dropping
  it outright) must not assume the same — an explicit drop-then-recreate
  pair, never a bare `ALTER POLICY` that could leave a gap mid-transaction.
- **Deferred-FK-attaching migrations**: `down()` drops the constraint, not
  the column — the column still belongs to the earlier migration that
  created it. `Schema::table($table, fn (Blueprint $t) => $t->dropForeign([$column]))`.

### Backfill

No migration in this list runs against populated data — `Task/app/` does
not exist yet in this repo. The one backfill worth flagging for whoever
eventually runs this against seeded/staging data:
`player_profiles.is_child` (service-maintained, not generated) must be
backfilled from `parent_child_links` existence before the application
starts trusting it, if any fixture or import ever inserts
`parent_child_links` rows without going through the owning Action.

---

## Seeders and factories

The source's Symfony edition used its own framework's fixture-loading
bundle; this document uses **Laravel model factories** (`database/factories/*.php`, realistic
defaults via `fake()`) for test/local fixtures and **database seeders**
(`database/seeders/*.php`) for reference data and orchestration — per this
repo's own `database-designer` skill ("Every model that needs test fixtures
should have a factory... Use seeders for reference/lookup data and local
dev fixtures; keep production seed data idempotent"). Shown for the models
needed to stand up the four MVP roles end-to-end; every other model gets a
factory following the same shape (one `definition()`, one state method per
named variant) and is not individually listed here.

### Factories

```php
// database/factories/AccountFactory.php
final class AccountFactory extends Factory
{
    protected $model = Account::class;

    public function definition(): array
    {
        return [
            'email' => fake()->unique()->safeEmail(),
            'password_hash' => 'password', // hashed by the 'hashed' cast on save
            'role' => AccountRole::Player,
            'status' => AccountStatus::Active,
            'email_verified_at' => now(),
        ];
    }

    public function superAdmin(): static
    {
        return $this->state(['role' => AccountRole::SuperAdmin]);
    }

    public function trainerRole(): static
    {
        return $this->state(['role' => AccountRole::Trainer]);
    }

    public function coachRole(): static
    {
        return $this->state(['role' => AccountRole::Coach]);
    }

    public function unverified(): static
    {
        return $this->state(['email_verified_at' => null]);
    }
}
```

```php
// database/factories/AccountProfileFactory.php
final class AccountProfileFactory extends Factory
{
    protected $model = AccountProfile::class;

    public function definition(): array
    {
        return [
            'account_id' => Account::factory(),
            'first_name' => fake()->firstName(),
            'last_name' => fake()->lastName(),
            'phone' => fake()->optional()->phoneNumber(),
        ];
    }
}
```

```php
// database/factories/TrainerFactory.php
final class TrainerFactory extends Factory
{
    protected $model = Trainer::class;

    public function definition(): array
    {
        return [
            'owner_account_id' => Account::factory()->trainerRole(),
            'business_name' => fake()->company(),
            'slug' => fake()->unique()->slug(),
            'timezone' => 'America/New_York',
        ];
    }

    // Mirrors the trainer-creation Action's own side effects (billing
    // settings, branding settings, all three feature toggles enabled) so a
    // factory-built Trainer is never missing rows the real creation path
    // always produces — the same reason the source flags this as a real,
    // testable coupling rather than an incidental one.
    public function configure(): static
    {
        return $this->afterCreating(function (Trainer $trainer): void {
            TrainerBillingSetting::factory()->for($trainer)->create();
            TrainerBrandingSetting::factory()->for($trainer)->create();

            foreach (FeatureName::cases() as $feature) {
                FeatureToggle::factory()->for($trainer)->create([
                    'feature_name' => $feature,
                    'updated_by_account_id' => $trainer->owner_account_id,
                ]);
            }
        });
    }
}
```

```php
// database/factories/CoachMembershipFactory.php
final class CoachMembershipFactory extends Factory
{
    protected $model = CoachMembership::class;

    public function definition(): array
    {
        return [
            'trainer_id' => Trainer::factory(),
            'account_id' => Account::factory()->coachRole(),
            'status' => CoachMembershipStatus::Pending,
        ];
    }

    public function active(): static
    {
        return $this->state(['status' => CoachMembershipStatus::Active, 'joined_at' => now()]);
    }
}
```

```php
// database/factories/PlayerProfileFactory.php
final class PlayerProfileFactory extends Factory
{
    protected $model = PlayerProfile::class;

    public function definition(): array
    {
        return [
            'self_account_id' => Account::factory(), // adult, self-training, by default
            'first_name' => fake()->firstName(),
            'date_of_birth' => fake()->dateTimeBetween('-45 years', '-19 years'),
            'is_child' => false,
        ];
    }

    // A child never has its own self_account_id by default (AC-01-20 makes
    // that optional) and must be paired with a ParentChildLink to make
    // is_child true correctly (service-maintained — see that column's
    // note) rather than just flipping the flag in isolation here.
    public function child(): static
    {
        return $this->state([
            'self_account_id' => null,
            'date_of_birth' => fake()->dateTimeBetween('-17 years', '-6 years'),
            'is_child' => true,
        ])->afterCreating(function (PlayerProfile $child): void {
            ParentChildLink::factory()->create(['child_player_id' => $child->id]);
        });
    }
}
```

```php
// database/factories/PlayerTrainerMembershipFactory.php
final class PlayerTrainerMembershipFactory extends Factory
{
    protected $model = PlayerTrainerMembership::class;

    public function definition(): array
    {
        return [
            'trainer_id' => Trainer::factory(),
            'player_id' => PlayerProfile::factory(),
            'source' => MembershipSource::Sharelink,
            'status' => MembershipStatus::Active,
        ];
    }
}
```

Usage, composing the four roles for a feature test:

```php
$trainer = Trainer::factory()->create();
$coach = CoachMembership::factory()->for($trainer)->active()->create();
$adultPlayer = PlayerTrainerMembership::factory()->for($trainer)->create();
$child = PlayerProfile::factory()->child()->create();
$childMembership = PlayerTrainerMembership::factory()
    ->for($trainer)
    ->create(['player_id' => $child->id]);
```

### Seeders

```php
// database/seeders/DatabaseSeeder.php — local/dev only, walking-skeleton fixture
final class DatabaseSeeder extends Seeder
{
    public function run(): void
    {
        $this->call(PlatformConfigurationSeeder::class);

        $superAdmin = Account::factory()->superAdmin()->create(['email' => 'admin@practiceperfect.test']);
        AccountProfile::factory()->for($superAdmin)->create();

        $trainer = Trainer::factory()->create(['business_name' => 'Acme Training']);
        AccountProfile::factory()->for($trainer->owner)->create();

        CoachMembership::factory()->for($trainer)->active()->count(2)->create();
        CoachMembership::factory()->for($trainer)->count(1)->create(); // one pending

        $adult = PlayerProfile::factory()->create();
        PlayerTrainerMembership::factory()->for($trainer)->create(['player_id' => $adult->id]);

        $child = PlayerProfile::factory()->child()->create();
        PlayerTrainerMembership::factory()->for($trainer)->create(['player_id' => $child->id]);
    }
}
```

```php
// database/seeders/PlatformConfigurationSeeder.php — idempotent, safe in any environment
final class PlatformConfigurationSeeder extends Seeder
{
    public function run(): void
    {
        $rows = [
            'default_platform_fee_basis_points' => 500,                    // BR-05-7
            'default_platform_subscription_price_minor_units' => 1500,      // BR-05-13
            'referral_reward_ratio' => ['referrals' => 1, 'tokens' => 1],    // Q-06.11 default
            'referral_attribution_window_days' => 30,                       // Q-06.12 default
            'referee_welcome_token' => false,                               // Q-06.11 default
            'token_package_seed' => [
                ['label' => '10 tokens', 'token_count' => 10, 'price_minor_units' => 9000],
                ['label' => '25 tokens', 'token_count' => 25, 'price_minor_units' => 22500],
                ['label' => '50 tokens', 'token_count' => 50, 'price_minor_units' => 45000],
            ], // Q-05.03 default
        ];

        foreach ($rows as $key => $value) {
            PlatformConfiguration::query()->updateOrCreate(
                ['key' => $key],
                ['value' => $value, 'updated_at' => now()],
            );
        }
    }
}
```

`updateOrCreate()` makes this seeder safe to run against an already-seeded
database (a real deploy, not just local dev) — matching the skill's own
"keep production seed data idempotent" rule. `TokenPackage` rows
materialize per-trainer from `token_package_seed` via the trainer-creation
Action (as noted under `token_packages` above), not this seeder directly.

---

## Decisions

Focused on the Laravel-porting judgment calls this document made — every
product-level decision already justified in
`specs/database-designer-schema.md`'s own "Decisions" table (money format,
platform-fee rounding, capacity-check strategy, playlist/content soft
delete, ledger invariant I4's locking mechanism, entry-immutability-by-privilege,
coach cross-tenant uniqueness via a partial index, RSVP status vocabulary,
referral "Rewarded" state, `playlist_assignments` status, form field
definitions as JSONB, `audit_log_entries.subject_id` as a soft reference,
`form_submissions` tenancy classification, the `public_tenant_codes` table,
Player/Parent modeling, content-purchase beneficiary) **carries over
unchanged** and is not re-litigated below — this table is what changed in
translation, and why.

| Decision | Chosen | Rejected | Because |
|---|---|---|---|
| Table naming | Laravel's plural convention (`trainers`, `events`), mechanically renamed from the source's singular names; every model declares `protected $table` explicitly | Keep the source's singular table names, override `$table` on every model to match | Fighting Laravel's strong naming convention on every one of 58 models buys nothing; explicit `$table` removes any dependency on how Eloquent's auto-pluralizer would resolve ambiguous words (`content_progress`, `content_usage`, `trainer_billing_settings`) rather than asserting a specific inflector behavior |
| Primary keys | `$table->id()` (Postgres `bigserial`) | A raw `GENERATED BY DEFAULT AS IDENTITY` column via `DB::statement` | Laravel's schema builder has no identity-column helper; `bigserial` is sequence-backed and behaves equivalently for this design (auto-incrementing, concurrency-safe). The *decision* this ports is "integer PK over UUID" (same reasoning as the source: public tokens are already separate high-entropy columns), not the literal DDL keyword |
| Array-typed columns (`skill_levels`, `genders`, `tags`, `equipment`, `categories`, three `filter_*` columns) | `JSONB` + Eloquent's built-in `'array'` cast | Native Postgres `TEXT[]`, matching the source exactly | Eloquent has no built-in cast for Postgres's native array wire format (`{a,b,c}`) — the stock `array` cast is JSON-based. Native arrays would need eight bespoke `CastsAttributes` classes for columns with no CHECK constraints to lose; JSONB gets identical query capability (`@>`, `?`, GIN-indexable) with zero custom code |
| Enum columns | Laravel's `$table->enum(...)` Blueprint helper | Raw `DB::statement` VARCHAR + CHECK | Laravel's Postgres grammar already compiles `enum()` to `varchar(255) check (col in (...))` — the exact encoding the source's own Conventions section specifies, reachable with a native Blueprint call instead of hand-written SQL |
| Prior framework's data-access layer | Eloquent models + query builder directly, with Action/Service classes for transactional/multi-step operations | A hand-rolled data-access class per model, mirroring the source 1:1 | Laravel has no equivalent pattern; this repo's own `database-designer` skill states Eloquent/query builder fill that role directly. A parallel data-access layer over Eloquent would be structure fighting the framework for no isolation benefit nothing here needs |
| Prior framework's query-level tenant filter (tenancy Layer 2) | Eloquent global scope (`TenantScope`) via a `BelongsToTenant` trait | A manual `where('trainer_id', ...)` added at every call site | A global scope is the named, idiomatic Laravel mechanism for "this filter must always apply," per this repo's `eloquent` skill, which names multi-tenancy as the paradigm case |
| `TenantResolver`/`kernel.request` (tenancy Layer 3) | `ResolveTenant` middleware + a request-scoped `TenantContext` binding | A trait or facade queried ad hoc from controllers | Direct translation instructed by `specs/council-sharelink-tenant-resolution.md` itself ("must become Laravel middleware and a resolver binding"); middleware is the idiomatic Laravel hook for "runs on every matching request before the controller," matching where `kernel.request` sat in the source |
| `drill_details` (was joined-table inheritance) | A shared-PK 1-to-1 Eloquent model (`ContentItem hasOne DrillDetail`), not a model subclass | Attempting to reproduce joined-table inheritance with a `Drill extends ContentItem` model and a discriminator-driven factory | Eloquent has no joined-table inheritance mechanism. The shared-PK pattern already used for `account_profiles`/`trainer_billing_settings` covers the same "1-to-1 extension keyed by the parent's PK" shape; the one acknowledged loss is a single polymorphic query across `ContentItem`/`Drill` as one class, named explicitly rather than silently dropped |
| Prior framework's value-object groupings (`EmergencyContact`, `EligibilityCriteria`, `DualPricing`, `Discount`) | Eloquent custom cast classes (`CastsAttributes`), combining sibling columns via the `$attributes` array | Plain public properties on the model for each raw column, no value object | This repo's own `eloquent` skill names exactly this pattern ("combining multiple columns into one value object") as the case for a custom cast; it is more code than the source's declarative value-object mapping, called out rather than presented as free |
| RLS policy DDL | Raw `DB::statement()`, wrapped in a small reusable `AppliesRowLevelSecurity` migration trait | Repeating the two-statement `ENABLE ROW LEVEL SECURITY`/`CREATE POLICY` block inline in all 37 migrations | The schema builder has no RLS API at all (expected — RLS is Postgres-specific and this repo's own `database-designer` skill already names raw `DB::statement()` as the escape hatch for anything Blueprint doesn't expose); the trait is a small ergonomics improvement the source tool's migrations didn't need shown, since those migrations are also just plain code that could equally have shared a helper — not a framework-forced difference, just taken here |
| `accounts` table name (not `users`) | Kept the source's `account` naming, pluralized | Rename to Laravel's conventional `users`/`User` | `account` already matches BR-01-7 ("each user has exactly one role" — Super Admin, Trainer, Coach, or Player, one identity table for all four) more precisely than Laravel's stock scaffolding assumption of a single consumer-facing `User`. This also means a starter kit's stock `users` migration/model must **not** be layered on top of this schema — flagged in Open questions, since resolving that is an `auth-scaffolding` decision, not a database one |
| `password_reset_tokens` | This design's version (`account_id` FK + `token_hash` + `expires_at`/`consumed_at`), replacing Laravel's stock starter-kit migration | Extend the stock migration (`email`+`token`) with extra columns | The column shapes conflict (account-keyed vs. email-keyed, hashed-token-with-explicit-expiry vs. plaintext-token-with-implicit-expiry); silently merging them would leave two different "how does a password reset token expire" answers in the same table. Whoever scaffolds a starter kit next must skip or delete its stock `password_reset_tokens` migration |
| Database roles/RLS bootstrap (`pp_owner`/`pp_app`/`pp_crossing`) | A plain SQL file (`database/pgsql/00_roles.sql`), run once by a superuser outside Artisan | A Laravel migration using `DB::statement('CREATE ROLE ...')` | Role/cluster-level DDL typically needs superuser privileges a migration-runner credential shouldn't have, and managed Postgres (RDS, Cloud SQL) often restricts it to provider-specific bootstrap paths a portable migration can't assume — the same reason the source kept this in `00-roles.sh`, not a schema migration. **This is new infrastructure this document originates**, not a port — no equivalent exists anywhere in this Laravel repo yet |
| Migration ordering | Filename-timestamp date-buckets, one per M0–M7 stage | An abstract "batch" grouping mirroring the source tool's own versioning | Laravel has no batch concept — it runs `database/migrations/*.php` strictly in filename order, so encoding the build order directly into timestamps is the literal mechanism, not an analogy |
| Data-access methods (source's "worth naming now" list) | Named Action/Service class methods, plus Eloquent local scopes for simple filters | A parallel data-access class per model exposing the same method names | Same reasoning as the data-access-layer decision above — Laravel's query builder and Eloquent scopes are the direct replacement, and Action/Service classes are where this repo's own conventions (`AGENTS.md`: "move multi-step business logic into Actions, Services, or the model layer") already put transactional, multi-step logic like the ledger's lock ordering |

---

## Open questions

Two kinds: product ambiguities carried over from the source (still
unsettled — this port does not resolve them, since none of these specs
touch them), and new questions specific to the Laravel translation.

### Carried over from `specs/database-designer-schema.md`, still unsettled

1. **Player/Parent modeling** — Epic-01 explicitly states it does not
   resolve how this should be modeled. This document adopts
   `player_profiles.self_account_id` + `parent_child_links` (matching the
   source's own inference) as workable, not settled — confirm before
   scaffolding depends on it.
2. **Does a playlist purchase unlock for one child or the whole family?**
   Neither Epic-04 nor Epic-05 states this. `playlist_access_grants` is
   modeled per-specific-player here, consistent with A7's
   beneficiary-recording spirit; if the client's intent is family-wide
   unlock, `playlist_access_grants.player_id` would need to become
   nullable-meaning-"whole family" or the table restructured around
   `(trainer_id, parent_account_id)`.
3. **Skill level vocabulary (Q-01.01, still open).** Plain `VARCHAR(50)` on
   `player_trainer_memberships.skill_level` and
   `playlists.filter_skill_levels`, per Section B's safe default. If the
   client wants trainer-configurable, reusable skill-level definitions, the
   upgrade path is a new trainer-scoped `skill_level_definitions` lookup
   table — additive, not a redesign.
4. **`ContentUsage` vs. "usage analytics" being out-of-MVP.** The epic
   defers "usage analytics for public content creators" to Phase 2, yet
   independently requires the "used by N trainers" figure (AC-04-37) that
   `content_usage` backs. This document builds `content_usage` per the
   settled architecture and treats AC-04-37 as the narrower in-scope thing
   the broader deferral is not — the tension is real and worth the
   client's eyes.
5. **Coupon code case-sensitivity.** BR-06 never states whether
   `coupons.code` should collide case-insensitively the way `labels.name`
   explicitly does. Left case-sensitive (plain `VARCHAR` unique index, no
   `citext`) as the more conservative default for a checkout field.
6. **`duration_seconds` nullability on `content_items`.** The epic's own
   open question — what happens if YouTube duration auto-detection fails
   at video-add time — is unresolved. Nullable, rather than inventing a
   fallback (e.g. 0) indistinguishable from a genuinely zero-length video.
7. **Whether Epic-07's dashboard metrics are ever materialized.** This
   schema assumes live queries (`toBase()` projections, no metrics-storage
   table). If AC-07-41's performance targets prove unreachable live at 500
   trainers/10,000 players, the first materialized projection needed is a
   `dashboard_metrics_snapshots` table — deliberately not built ahead of a
   measured need.

### New to this Laravel translation

8. **Tenant resolution sources 1 and 4 are named but never defined in any
   input available to this document.** The council's precedence note
   ("Sources 1 and 2 outrank everything... 3 then 4") references a Super
   Admin administrative-scope source (1) and a fourth source never
   described anywhere `specs/` carries into this Laravel edition (source 1
   is referenced only by its effect — "AdministrativeScope... requires
   ROLE_SUPER_ADMIN plus a voter" — never its concrete mechanics). This
   document implements sources 2, 3, and 5 concretely (`ResolveTenant`
   above) and leaves an extension point for 1 and 4 rather than inventing
   them — an `architect`/`auth-scaffolding` decision, not a database one.
9. **Managed Postgres provisioning for the three-role RLS bootstrap.**
   `database/pgsql/00_roles.sql` assumes a superuser can run arbitrary
   `CREATE ROLE ... BYPASSRLS`. Managed providers (RDS, Cloud SQL, Supabase)
   frequently restrict this to a specific bootstrap procedure or disallow
   `BYPASSRLS` for non-owner roles entirely — whoever picks the hosting
   target needs to confirm this script (or an equivalent) is actually
   runnable there before relying on `pp_crossing`.
10. **Whether this application will run under Laravel Octane.** Directly
    changes how much the "Octane note" under Tenancy matters in practice —
    if Octane is adopted, the unconditional per-request `SET
    app.current_trainer_id` in `ResolveTenant` (already designed for this)
    becomes load-bearing rather than defensive; if the app stays on
    traditional PHP-FPM, it is a no-cost safety margin. Not yet decided
    anywhere in this repo's specs.
11. **Wiring `php artisan tenancy:verify` into an actual deploy gate.** The
    command exists (above); *when* it runs — a CI step, a release-phase
    command (Heroku-style), a Kubernetes init container, a manual runbook
    step — is a deployment-topology decision this repo has not made yet
    (no `Task/app/` scaffold to hook into, unlike the source's Docker
    entrypoint Gate 4).
12. **Starter-kit choice and its collision with `accounts`.** This repo's
    own `auth-scaffolding` skill defaults new Laravel apps to the
    first-party Starter Kits, which scaffold a stock `User` model and
    `users` table. This schema's identity table is `accounts` (see
    Decisions) — whoever runs `auth-scaffolding` next must point
    `config/auth.php`'s `providers.users.model` at `App\Models\Account`
    and skip/delete the stock `users`/`password_reset_tokens` migrations
    rather than layering this schema on top of them. Flagged here because
    it is a direct, foreseeable integration failure if missed, even though
    resolving it is that skill's job, not this document's.
