---
{
  "id": "MEM-0006",
  "title": "Stack choices with a shelf life",
  "type": "architecture",
  "status": "active",
  "scope": ["platform", "infrastructure"],
  "tags": ["stack", "dependencies", "versions", "decisions"],
  "created": "2026-08-09",
  "last_verified": "2026-08-09",
  "review_after": "2027-02-09",
  "sources": [
    "specs/architect-architecture.md"
  ],
  "supersedes": [],
  "superseded_by": null
}
---

# Stack Choices with a Shelf Life

## Durable Context

Each pinned choice trades off alternatives. Review these decisions at the start of a major release or when a constraint changes (e.g., Docker Compose support ends, PHP security support changes, or a new Redis release makes single-broker configurations viable).

| Choice | Version | Over the alternative | Consequence |
|---|---|---|---|
| **Symfony** | 7.4 LTS | 8.1 (not LTS; requires PHP >= 8.4; ends Jan 2027) | Bugfixes to Nov 2028, security to Nov 2029. No Python-environment-style churn from version jumps |
| **PHP image** | 8.3 or 8.4 | 8.2 (security support ends 31 Dec 2026) | Both satisfy Symfony 7.4's floor (PHP >= 8.2). Choose 8.4 if you need the latest; 8.3 if you need stability |
| **PostgreSQL** | 16 or 17, one pinned tag | MySQL (no RLS), SQLite (single-process limit), managed services outside Docker | Row-Level Security and LISTEN/NOTIFY, both long-established and stable. Same instance backs Messenger, sessions, cache and locks. No sixth container |
| **Stripe** | `stripe/stripe-php` v21.x, pinned in `composer.lock` | The abandoned Symfony bundle or rolling the latest SDK version | Official, ~164M installs. The one established bundle's own README recommends implementing directly. Pin the lock to avoid mid-release surprises |
| **Asset pipeline** | AssetMapper | Node.js toolchain (Webpack, Vite, esbuild) | Current `--webapp` default. No Node toolchain, which matters under a Compose-only constraint. Static token layer compiles once; brand layer never enters |
| **Money** | Integer minor units (cents) | `brick/money` library or floating-point | Matches Stripe's wire format exactly. `brick/money` adds a mapping layer at a boundary that already speaks integers. Scoped to USD-only with a flat fee |
| **Async transport** | `doctrine://` using PostgreSQL | RabbitMQ, Redis, Kafka, or a dedicated broker | Uses the PostgreSQL already present; no broker container. Webhook processing requires async (Stripe 2xx before retry logic). Single-process worker under Compose limit is adequate |
| **Scheduling** | Symfony Scheduler in the worker container | Host cron or a dedicated cron container | Host cron violates the no-host-dependency constraint. A separate container adds nothing the worker cannot do. State derived at read time; scheduler only materialises side effects |
| **Session/cache/locks** | PostgreSQL PDO handlers | Filesystem (forces single-replica affinity) or Redis (sixth container) | PostgreSQL adequately handles all three at this scale. Replicas are possible; uploaded logos on a volume would need shared storage before scaling `php` |

## Consequences

### At Each Stage

**Now (MVP, Docker Compose, single PostgreSQL):**

- Symfony 7.4 and PHP 8.3 are stable, well-documented choices.
- PostgreSQL 16 or 17 are both solid; 17 is newer. Use one tag and do not drift.
- AssetMapper means no Node.js environment and no build-time color derivation.
- `doctrine://` Messenger is sufficient for webhooks, refund fan-out, and notifications.
- Scheduler in the worker handles time-based side effects (auto-denial, RSVP expiry, referral attribution).
- Sessions and cache on PostgreSQL mean no sticky sessions required and no Redis container.

**Later, if scale or availability changes:**

- Move to Symfony 8.x only after 7.4 nears end of security support (Nov 2029).
- PostgreSQL 16 → 17 → 18 are LTS releases; upgrade within the LTS window.
- `brick/money` becomes relevant if you add multi-currency or complex partial-refund allocation.
- Redis becomes relevant if a separate cache layer is needed; it does not replace PostgreSQL for locks or sessions at this architecture.
- A message broker (RabbitMQ, Kafka) becomes relevant if async work scales beyond one worker consuming all queues.

**If Compose support ends:**

- The constraint "no host dependencies" is the biggest blocker for moving to a traditional host deployment. Most choices above (Symfony, PHP, PostgreSQL) are identical; Scheduler would move to a separate container or host cron.

## Verification

Pin every version in `composer.lock` and Docker image tags. Do not rely on `latest` or floating tags. Document the upgrade calendar: when does each component's security support end? Schedule reviews ahead of those dates.

Before upgrading any pinned component:

- Check if it changes the architecture (e.g., new AssetMapper feature, PostgreSQL major release with RLS changes).
- Run the full test suite with the new version before deploying to staging.
- For PostgreSQL, test migrations and RLS behavior — schema and policy changes are the highest-risk changes in this platform.
