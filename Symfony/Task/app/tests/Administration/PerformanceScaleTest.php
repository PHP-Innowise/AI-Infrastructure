<?php

declare(strict_types=1);

namespace App\Tests\Administration;

use PHPUnit\Framework\TestCase;

/**
 * AC-07-41 — "Performance & Scale Targets": dashboard loads <2 seconds
 * (100+ trainers); every Super Admin tool loads <2 seconds; user search
 * returns in <500ms; the Event Master Tool loads <2 seconds with 1,000
 * events (paginated, up to 5,000 total); the audit log loads <1 second
 * with 10,000 entries (storing up to 100,000); the platform supports 500
 * trainers and 10,000 players.
 *
 * Every one of these is a wall-clock latency budget or a production-scale
 * figure — none is meaningfully verifiable from this test environment, for
 * the same reasons `Tests\Billing\PerformanceTargetsTest` already gives for
 * AC-05-38 (that file's own docblock, restated here per-criterion):
 *
 * - Latency budgets (<2s/<500ms/<1s) measured against this dev/test
 *   stack's own single-container Postgres and a fixture database of a
 *   handful of rows would say nothing about whether the criterion holds at
 *   the stated PRODUCTION scale (100+ trainers, 1,000-5,000 events, 10,000
 *   -100,000 audit entries) — a passing timer here would be measuring the
 *   wrong thing, and a failing one would be equally meaningless.
 * - "The platform supports 500 trainers and 10,000 players" is a capacity
 *   claim about a populated production deployment, not a behavior any
 *   single test run produces or observes.
 *
 * What IS genuinely proven, by other tests in this suite, is the
 * STRUCTURAL half of each figure — the part a functional test can actually
 * demonstrate regardless of what timings a production deployment would
 * show:
 * - Every paginated Administration screen this epic adds genuinely caps
 *   its page size and offers page navigation rather than returning
 *   unbounded result sets (`UsersToolEpic07Test::testUsersToolPaginatesAtFiftyPerPage()`,
 *   `EventMasterEpic07Test::testEventListPaginatesAtFiftyPerPage()`,
 *   `AuditLogTest` via `AuditLogEntryRepository::search()`'s own `$limit`/
 *   `$offset`).
 * - The Event Master and dashboard queries reuse `CrossTenantReadService`'s
 *   own indexed columns (`event.starts_at`, `event.trainer_id`) and the
 *   audit log reuses `audit_log_entry`'s own four indexes
 *   (`idx_audit_occurred_at`, `idx_audit_trainer_occurred_at`,
 *   `idx_audit_action_type`, `idx_audit_actor`) rather than an unindexed
 *   scan — a design property, not a timing measurement.
 *
 * Recorded honestly as unverifiable in this environment, per this task's
 * own instruction that a genuinely unverifiable criterion gets a skip with
 * an honest reason rather than a fabricated pass — never a sleep()-based
 * timing assertion dressed up as proof, and never a single-request "it
 * responded" check mislabeled as a scale proof.
 */
final class PerformanceScaleTest extends TestCase
{
    public function testPerformanceAndScaleTargetsAreNotVerifiableInThisEnvironment(): void
    {
        self::markTestSkipped(
            'AC-07-41: latency budgets (<2s/<500ms/<1s) and production-scale figures (100+ trainers, '
            .'1,000-5,000 events, 10,000-100,000 audit entries, 500 trainers/10,000 players platform-wide) are '
            .'production/operational claims, not behavior this local test stack can meaningfully prove or '
            .'disprove. The STRUCTURAL half — genuine pagination and indexed queries, never an unbounded scan — '
            .'is proven by UsersToolEpic07Test, EventMasterEpic07Test and AuditLogTest. See this test\'s own '
            .'class docblock for the full reasoning per target.',
        );
    }
}
