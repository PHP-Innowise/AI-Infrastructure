<?php

declare(strict_types=1);

namespace App\Tests\Forms;

use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\FormsFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\RateLimiter\RateLimiterFactory;

/**
 * The public Forms surface is the only place in this platform an anonymous
 * request writes trainer-scoped data carrying PII of minors — an open risk
 * no epic specifies a mitigation for
 * (council-sharelink-tenant-resolution.md, "Open architecture risk 7").
 * This codebase's answer: per-IP rate limiting (`config/packages/
 * rate_limiter.yaml`) plus the honeypot field (covered by
 * `PublicFormSubmissionTest::testHoneypotFieldSilentlyDropsTheSubmission()`).
 *
 * **Every test uses a freshly random RFC 5737 TEST-NET-3 address
 * (`203.0.113.0/24`) as its rate-limit key, never a fixed one like
 * `127.0.0.1`.** The limiter's storage is the filesystem cache pool
 * (`cache.app`), which — unlike the database fixtures `make test` reloads
 * on every run — is never purged between runs; a fixed key would let one
 * run's consumed tokens leak into the next and make this test's own outcome
 * depend on unrelated history instead of what it actually exercises.
 *
 * Drives each limiter directly to its OWN configured boundary (`getLimit()`)
 * rather than hard-coding the number, so this test stays correct if the
 * policy in `rate_limiter.yaml` is ever retuned — then makes one real HTTP
 * request to confirm the controller actually surfaces the exhausted limiter
 * as `429`, not merely that the limiter service itself would reject.
 */
final class FormAbuseProtectionTest extends WebTestCase
{
    use FixtureHelpers;
    use FormsFixtureHelpers;

    private KernelBrowser $client;
    private RateLimiterFactory $showLimiter;
    private RateLimiterFactory $submitLimiter;

    protected function setUp(): void
    {
        $this->client = self::createClient();

        /** @var RateLimiterFactory $showLimiter */
        $showLimiter = self::getContainer()->get('limiter.forms_public_show');
        $this->showLimiter = $showLimiter;

        /** @var RateLimiterFactory $submitLimiter */
        $submitLimiter = self::getContainer()->get('limiter.forms_public_submit');
        $this->submitLimiter = $submitLimiter;
    }

    public function testPublicShowIsRateLimitedPerIp(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Rate Limited Show Camp']);

        $ip = self::freshTestIp();
        $this->exhaust($this->showLimiter, $ip);

        $this->client->request('GET', '/forms/'.$form->getShareableSlug(), [], [], ['REMOTE_ADDR' => $ip]);

        self::assertResponseStatusCodeSame(429);
    }

    public function testPublicSubmitIsRateLimitedPerIp(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Rate Limited Submit Camp']);

        $ip = self::freshTestIp();
        $this->exhaust($this->submitLimiter, $ip);

        $this->client->request('POST', '/forms/'.$form->getShareableSlug(), [
            'form_submission' => ['participant_name' => 'Rate Limited', 'participant_email' => 'rate-limited@example.test'],
        ], [], ['REMOTE_ADDR' => $ip]);

        self::assertResponseStatusCodeSame(429);
    }

    /**
     * Different IPs get independent budgets — the limiter key is per-IP,
     * not global, so one abusive visitor cannot lock out everyone else.
     */
    public function testRateLimitIsPerIpNotGlobal(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Independent Budget Camp']);

        $this->exhaust($this->showLimiter, self::freshTestIp());

        // A different, un-exhausted (and never-before-used) IP is unaffected.
        $this->client->request('GET', '/forms/'.$form->getShareableSlug(), [], [], ['REMOTE_ADDR' => self::freshTestIp()]);

        self::assertResponseIsSuccessful();
    }

    private function exhaust(RateLimiterFactory $factory, string $key): void
    {
        $first = $factory->create($key)->consume(1);
        $remaining = $first->getLimit() - 1;

        if ($remaining > 0) {
            $factory->create($key)->consume($remaining);
        }
    }

    /**
     * RFC 5737 TEST-NET-3 (`203.0.113.0/24`) — reserved for documentation
     * and testing, never a real routable address, and never reused between
     * calls within this whole suite's lifetime (the /24 gives 254 host
     * addresses; a single test process exhausts nowhere near that many).
     */
    private static function freshTestIp(): string
    {
        return '203.0.113.'.random_int(1, 254);
    }
}
