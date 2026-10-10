<?php

declare(strict_types=1);

namespace App\Tests\Platform;

use App\Platform\Entity\FeatureToggle;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Doctrine\DBAL\Connection;
use PHPUnit\Framework\Attributes\DataProvider;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\DomCrawler\Crawler;

/**
 * The navigation has to be on every page, not on the one screen that built
 * it.
 *
 * `DashboardNavigationTest` already holds each role's link list against the
 * router — and passed throughout, because it asks the dashboard. The dead end
 * was everywhere else: 114 templates, none of them linking to the dashboard,
 * and an empty `{% block header %}` in the layout that nothing overrode. Open
 * Players, Tokens or Branding and the only way out was the browser's back
 * button.
 *
 * So these tests never look at the dashboard. They ask inner pages.
 */
final class GlobalNavigationTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * One deep page per role — the kind of screen a user actually lands on
     * after following a link and then wants to leave.
     *
     * @param non-empty-string $email
     * @param non-empty-string $path
     * @param non-empty-string $expectedOwnLink
     */
    #[DataProvider('innerPageProvider')]
    public function testAnInnerPageCarriesTheNavigationAndAWayBack(string $email, string $path, string $expectedOwnLink): void
    {
        $this->client->loginUser($this->account($email));
        $this->resolveTenantIfPlayer($email);
        $crawler = $this->client->request('GET', $path);

        self::assertResponseIsSuccessful();

        $hrefs = $this->navigationHrefs($crawler);

        self::assertNotEmpty($hrefs, sprintf('%s has no navigation at all — the defect this test exists for.', $path));
        self::assertContains('/dashboard', $hrefs, sprintf('%s offers no way back to the dashboard.', $path));
        self::assertContains($expectedOwnLink, $hrefs, sprintf('%s does not offer this role its own module links.', $path));
    }

    /**
     * @return iterable<string, array{string, string, string}>
     */
    public static function innerPageProvider(): iterable
    {
        yield 'trainer, deep in the CRM' => ['trainer@practiceperfect.test', '/trainer/players', '/trainer/events'];
        yield 'trainer, on branding settings' => ['trainer@practiceperfect.test', '/trainer/branding', '/trainer/billing'];
        yield 'player, on tokens' => ['player@practiceperfect.test', '/portal/tokens', '/portal/calendar'];
        yield 'player, on the calendar' => ['player@practiceperfect.test', '/portal/calendar', '/portal/family'];
        yield 'coach, on activities' => ['coach@practiceperfect.test', '/coach/activities', '/coach/players'];
        yield 'super admin, in the users tool' => ['admin@practiceperfect.test', '/super-admin/users', '/super-admin/audit-log'];
    }

    /**
     * A role's own links, and only their own, wherever they are standing —
     * the same rule AccessControlTest asserts on the dashboard (AC-01-68),
     * now that the nav renders in a hundred more places.
     */
    public function testAnInnerPageStillShowsOnlyTheRolesOwnLinks(): void
    {
        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->resolveTenantIfPlayer('player@practiceperfect.test');
        $crawler = $this->client->request('GET', '/portal/tokens');

        $hrefs = $this->navigationHrefs($crawler);

        foreach (['/trainer/players', '/super-admin/users', '/coach/activities'] as $foreign) {
            self::assertNotContains($foreign, $hrefs, sprintf('A player must not be offered %s.', $foreign));
        }
    }

    /**
     * BR-07-1 held on the dashboard already; it has to hold on every page the
     * nav now reaches, or a disabled feature is one click away from a 403
     * from anywhere.
     */
    public function testADisabledFeatureIsAbsentFromEveryPagesNavigation(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $before = $this->navigationHrefs($this->client->request('GET', '/trainer/players'));
        self::assertContains('/trainer/marketing/coupons', $before, 'Precondition: marketing is on.');

        $this->setFeature($trainer->getId(), FeatureToggle::FEATURE_MARKETING, false);

        try {
            $after = $this->navigationHrefs($this->client->request('GET', '/trainer/players'));

            self::assertNotContains('/trainer/marketing/coupons', $after, 'A disabled feature must vanish from the nav on inner pages too.');
            self::assertContains('/trainer/events', $after, 'Disabling one feature must not take the rest of the nav with it.');
        } finally {
            $this->removeFeatureOverride($trainer->getId(), FeatureToggle::FEATURE_MARKETING);
        }
    }

    /**
     * Nobody signed in, nothing to navigate. A public landing page belongs to
     * a prospective customer, and a nav full of portal links would be both
     * broken and confusing.
     *
     * @param non-empty-string $path
     */
    #[DataProvider('publicPageProvider')]
    public function testPublicPagesCarryNoNavigation(string $path): void
    {
        $crawler = $this->client->request('GET', $path);

        self::assertResponseIsSuccessful();
        self::assertCount(0, $crawler->filter('nav[aria-label="Main"]'), sprintf('%s is public — it has no user to navigate as.', $path));
    }

    /**
     * @return iterable<string, array{string}>
     */
    public static function publicPageProvider(): iterable
    {
        yield 'login' => ['/login'];
        yield 'trainer landing page' => ['/join/join-peak-performance'];
    }

    /**
     * A player whose account is linked to more than one trainer has no tenant
     * until they pick one, and a portal page without a tenant is a 500 — a
     * pre-existing shape this test works around exactly as the Scheduling
     * tests do, rather than pretending the choice is implicit. Which trainers
     * a fixture player has accumulated depends on what ran before, so this is
     * not optional even though it passes without it in isolation.
     */
    private function resolveTenantIfPlayer(string $email): void
    {
        if ('player@practiceperfect.test' !== $email) {
            return;
        }

        $this->switchPlayerToTrainer($this->client, $this->trainer('peak-performance'));
    }

    /**
     * @return list<string> hrefs inside the main navigation, path only
     */
    private function navigationHrefs(Crawler $crawler): array
    {
        return $crawler->filter('nav[aria-label="Main"] a')->each(
            static fn (Crawler $node): string => parse_url($node->attr('href') ?? '', \PHP_URL_PATH) ?: '',
        );
    }

    private function setFeature(?int $trainerId, string $feature, bool $enabled): void
    {
        $actorId = (int) $this->connection()->fetchOne("SELECT id FROM account WHERE role = 'super_admin' ORDER BY id LIMIT 1");

        $this->connection()->executeStatement(
            'INSERT INTO feature_toggle (trainer_id, feature_name, is_enabled, updated_at, updated_by_account_id)
             VALUES (?, ?, ?, now(), ?)
             ON CONFLICT (trainer_id, feature_name) DO UPDATE SET is_enabled = EXCLUDED.is_enabled',
            [$trainerId, $feature, $enabled ? 'true' : 'false', $actorId],
        );
    }

    /**
     * Deleted rather than set back to true, for the reason
     * DashboardNavigationTest already records: the gate defaults to enabled
     * when no row exists, so removing the override restores the original
     * state exactly instead of approximating it.
     */
    private function removeFeatureOverride(?int $trainerId, string $feature): void
    {
        $this->connection()->executeStatement(
            'DELETE FROM feature_toggle WHERE trainer_id = ? AND feature_name = ?',
            [$trainerId, $feature],
        );
    }

    private function connection(): Connection
    {
        /** @var Connection $connection */
        $connection = self::getContainer()->get('doctrine.dbal.default_connection');

        return $connection;
    }
}
