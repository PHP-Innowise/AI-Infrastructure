<?php

declare(strict_types=1);

namespace App\Tests\Content;

use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * The epic-level criteria this suite has not already covered as a side
 * effect of another AC — each skipped here with an honest, specific reason
 * rather than faked. See the coder's final report for the same list with
 * full context.
 *
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md "Epic-level (from 'Acceptance Criteria (Epic-Level)')"
 */
final class EpicCompletionCriteriaTest extends WebTestCase
{
    /**
     * AC-04-42: "The LP portal loads in under 2 seconds and drill-database
     * search returns results in under 1 second; YouTube videos are expected
     * to embed and play smoothly." Load/performance targets need dedicated
     * load-testing tooling (e.g. a k6/Locust/JMeter run against a
     * realistically sized dataset), not a PHPUnit functional-test assertion
     * — a wall-clock timing on a single request in a shared, unthrottled
     * test container would not be a meaningful measurement either way,
     * matching Scheduling's own EpicCompletionCriteriaTest precedent for
     * AC-02-69.
     */
    public function testPerformanceTargets(): void
    {
        self::markTestSkipped(
            'AC-04-42\'s targets are load/performance criteria requiring dedicated load-testing '.
            'tooling against a realistic dataset, not a PHPUnit functional-test assertion.',
        );
    }

    /**
     * AC-04-43 (the epic's own text: "Process gate, not product behavior"):
     * "Epic-04 is considered complete only once the demo is approved, all P0
     * open questions are resolved, public content network effects are
     * validated, and the YouTube embedding strategy is confirmed working."
     * A human sign-off/process gate, not something a test can assert.
     */
    public function testEpicApprovalIsAProcessGateNotProductBehavior(): void
    {
        self::markTestSkipped(
            'AC-04-43 is explicitly a process gate ("Process gate, not product behavior" per the '.
            'epic\'s own text) — demo approval and open-question resolution are human sign-offs, '.
            'not something a test can assert.',
        );
    }
}
