<?php

declare(strict_types=1);

namespace App\Tests\Billing;

use PHPUnit\Framework\TestCase;

/**
 * AC-05-38 — "Performance & Scale Targets" (specs/requirements-analyst-epic-05-payments-tokens-spec.md,
 * lines 747-761; epic-level AC "Performance", lines 890-893): token
 * balance check <100ms, token purchase redirect to Stripe <500ms, payment
 * confirmation webhook processing <2 seconds, refund processing <5
 * seconds (tokens) / <10 seconds (USD initiation); 100 concurrent
 * payments supported; 1,000 transactions/day (Year 1 estimate); 50
 * webhooks/minute; 99.9% payment processing uptime (Stripe SLA); <0.1%
 * payment data loss.
 *
 * Every one of these is either a wall-clock latency budget, a
 * production-scale throughput/concurrency figure, or a third-party SLA
 * (Stripe's own 99.9% uptime commitment). None is meaningfully verifiable
 * from this test environment:
 *
 * - Latency budgets (<100ms/<500ms/<2s/<5s/<10s) against an
 *   `InMemoryStripeClient` and a local, single-container Postgres
 *   instance would measure this dev/test stack's own incidental
 *   performance, not the production system the targets describe — a
 *   passing or failing assertion here would say nothing meaningful about
 *   whether the REAL, Stripe-backed, deployed platform meets them.
 * - Concurrency/throughput figures (100 concurrent payments, 1,000
 *   transactions/day, 50 webhooks/minute) describe PRODUCTION load
 *   patterns over sustained periods; TokenBalanceLockOrderingConcurrencyTest
 *   already proves this codebase's own lock-ordering invariant holds
 *   under contention (the CORRECTNESS half of concurrency), but "correct
 *   under 2 concurrent requests in a test" does not establish "handles
 *   100 concurrent payments in production" (the SCALE half) — those are
 *   different claims, and only the first is something a unit/integration
 *   test can actually demonstrate.
 * - 99.9% uptime is Stripe's own SLA commitment, not this codebase's
 *   behavior to test at all.
 * - <0.1% payment data loss is an operational/statistical claim about a
 *   fleet running over time, not a single test run's pass/fail condition.
 *
 * Recorded honestly as unverifiable in this environment, per this task's
 * own instruction that a genuinely unverifiable criterion gets a skip with
 * an honest reason rather than a fabricated pass — never a sleep()-based
 * timing assertion dressed up as proof, and never a single-request
 * "it responded" check mislabeled as a concurrency/scale proof.
 */
final class PerformanceTargetsTest extends TestCase
{
    public function testPerformanceAndScaleTargetsAreNotVerifiableInThisEnvironment(): void
    {
        self::markTestSkipped(
            'AC-05-38: latency budgets, concurrency/throughput figures, Stripe\'s own uptime SLA, and a '
            .'data-loss rate are production/operational claims, not behavior this local test stack can '
            .'meaningfully prove or disprove. The CORRECTNESS half of concurrent token spending (never the '
            .'raw throughput) is genuinely proven by TokenBalanceLockOrderingConcurrencyTest. See this '
            .'test\'s own class docblock for the full reasoning per target.',
        );
    }
}
