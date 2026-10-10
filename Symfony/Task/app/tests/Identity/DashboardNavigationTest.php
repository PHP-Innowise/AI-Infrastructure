<?php

declare(strict_types=1);

namespace App\Tests\Identity;

use App\Identity\Entity\Account;
use App\Identity\Repository\AccountRepository;
use App\Platform\Entity\FeatureToggle;
use App\Platform\Repository\TrainerRepository;
use App\Platform\Service\FeatureGate;
use PHPUnit\Framework\Attributes\DataProvider;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\DomCrawler\Crawler;

/**
 * The dashboard is the product's only navigation, which makes it the single
 * place an entire module can go missing.
 *
 * It did. Manual testing found five modules — CRM, Content, Billing, Growth
 * and Forms — fully built, migrated and tested, yet reachable only by typing a
 * URL: 81 trainer routes behind 4 links, 37 portal routes behind 5, 31
 * Super Admin routes behind 2. Every epic added routes; only two remembered to
 * add links. The automated suite missed it because tests request routes
 * directly and never have to find them.
 *
 * These tests close that gap by asserting the navigation against each module's
 * entry point, so a sixth module cannot ship invisible.
 *
 * AC-01-5, AC-01-32.
 */
final class DashboardNavigationTest extends WebTestCase
{
    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * Every module a role owns must be reachable from that role's dashboard.
     *
     * @param list<string> $expectedPaths
     */
    #[DataProvider('roleEntryPointProvider')]
    public function testEveryModuleEntryPointIsLinkedFromTheDashboard(string $email, array $expectedPaths): void
    {
        $this->client->loginUser($this->accountFor($email));
        $crawler = $this->client->request('GET', '/dashboard');

        self::assertResponseIsSuccessful();

        $linked = $this->navigationHrefs($crawler);

        foreach ($expectedPaths as $path) {
            self::assertContains(
                $path,
                $linked,
                sprintf(
                    'The dashboard for %s does not link to %s. A module reachable only by typing its URL '
                    .'is a module the user will never find.',
                    $email,
                    $path,
                ),
            );
        }
    }

    /**
     * @return iterable<string, array{string, list<string>}>
     */
    public static function roleEntryPointProvider(): iterable
    {
        yield 'trainer' => ['trainer@practiceperfect.test', [
            '/trainer/dashboard',          // Crm — Quick View
            '/trainer/players',            // Crm
            '/trainer/labels',             // Crm
            '/trainer/events',             // Scheduling
            '/trainer/coaches',            // Identity
            '/trainer/content/learn',      // Content (lppp)
            '/trainer/content/practice',   // Content (lppp)
            '/trainer/content/drills',     // Content (lppp)
            '/trainer/billing',            // Billing
            '/trainer/billing/pricing',    // Billing
            '/trainer/billing/earnings',   // Billing
            '/trainer/marketing/referrals', // Growth (marketing)
            '/trainer/marketing/coupons',  // Growth (marketing)
            '/trainer/sharelinks',         // Identity
            '/trainer/forms',              // Forms (camps)
            '/trainer/branding',           // Identity
        ]];

        yield 'player' => ['player@practiceperfect.test', [
            '/portal/calendar',            // Scheduling
            '/portal/reservations',        // Scheduling
            '/portal/availability',        // Identity
            '/portal/content',             // Content (lppp)
            '/portal/content/progress',    // Content (lppp)
            '/portal/family',              // Identity
            '/portal/approvals',           // Identity
            '/portal/tokens',              // Billing
            '/portal/transactions',        // Billing
            '/portal/referrals',           // Growth (marketing)
        ]];

        yield 'coach' => ['coach@practiceperfect.test', [
            '/coach/activities',           // Scheduling
            '/coach/players',              // Crm
            '/coach/players/invite',       // Identity
            '/coach/availability',         // Identity
        ]];

        yield 'super admin' => ['admin@practiceperfect.test', [
            '/super-admin/dashboard',              // Administration
            '/super-admin/users',                  // Administration
            '/super-admin/trainers',               // Administration
            '/super-admin/events',                 // Administration
            '/super-admin/audit-log',              // Administration
            '/super-admin/stripe',                 // Administration
            '/super-admin/crm/dashboard',          // Crm
            '/super-admin/crm/players',            // Crm
            '/super-admin/content/analytics',      // Content
            '/super-admin/growth/referral-rules',  // Growth
        ]];
    }

    /**
     * BR-07-1: a disabled feature "disappears from the trainer's UI". A link
     * that leads to a 403 is a broken feature, not a disabled one — so the
     * navigation must consult the gate, not only the controllers behind it.
     */
    public function testADisabledFeatureIsAbsentFromTheNavigationEntirely(): void
    {
        $trainers = self::getContainer()->get(TrainerRepository::class);
        $trainer = $trainers->findOneBySlug('peak-performance');
        self::assertNotNull($trainer);

        $gate = self::getContainer()->get(FeatureGate::class);
        self::assertTrue(
            $gate->isEnabled($trainer, FeatureToggle::FEATURE_MARKETING),
            'Marketing is expected on by default; this test asserts what changes when it is turned off.',
        );

        $this->client->loginUser($this->accountFor('trainer@practiceperfect.test'));
        $enabled = $this->navigationHrefs($this->client->request('GET', '/dashboard'));

        self::assertContains('/trainer/marketing/coupons', $enabled, 'Precondition: marketing is on.');

        $this->disableFeature($trainer->getId(), FeatureToggle::FEATURE_MARKETING);

        try {
            $disabled = $this->navigationHrefs($this->client->request('GET', '/dashboard'));

            self::assertNotContains(
                '/trainer/marketing/coupons',
                $disabled,
                'A disabled feature must vanish from the navigation, not offer a link into a 403.',
            );
            self::assertContains(
                '/trainer/events',
                $disabled,
                'Disabling one feature must not take unrelated navigation with it.',
            );
        } finally {
            // The suite shares one database without per-test rollback, so a
            // toggle left off here would disable marketing for every later
            // test that touches this trainer. Restored in a finally block so a
            // failed assertion above still cleans up after itself.
            $this->removeFeatureOverride($trainer->getId(), FeatureToggle::FEATURE_MARKETING);
        }
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

    private function disableFeature(?int $trainerId, string $feature): void
    {
        self::assertNotNull($trainerId);

        /** @var \Doctrine\DBAL\Connection $connection */
        $connection = self::getContainer()->get('doctrine.dbal.default_connection');

        // Written directly rather than through the admin screen: this test is
        // about the navigation's reaction, not about how the toggle is set.
        $actorId = (int) $connection->fetchOne(
            "SELECT id FROM account WHERE role = 'super_admin' ORDER BY id LIMIT 1"
        );

        $connection->executeStatement(
            'INSERT INTO feature_toggle (trainer_id, feature_name, is_enabled, updated_at, updated_by_account_id)
             VALUES (?, ?, false, now(), ?)
             ON CONFLICT (trainer_id, feature_name) DO UPDATE SET is_enabled = false',
            [$trainerId, $feature, $actorId],
        );
    }

    /**
     * Deletes the override rather than setting it back to true: FeatureGate
     * defaults to enabled when no row exists, so removing it restores the
     * original state exactly instead of merely approximating it.
     */
    private function removeFeatureOverride(?int $trainerId, string $feature): void
    {
        self::assertNotNull($trainerId);

        /** @var \Doctrine\DBAL\Connection $connection */
        $connection = self::getContainer()->get('doctrine.dbal.default_connection');

        $connection->executeStatement(
            'DELETE FROM feature_toggle WHERE trainer_id = ? AND feature_name = ?',
            [$trainerId, $feature],
        );
    }

    private function accountFor(string $email): Account
    {
        /** @var AccountRepository $repository */
        $repository = self::getContainer()->get(AccountRepository::class);
        $account = $repository->findOneByEmail($email);

        self::assertInstanceOf(Account::class, $account, sprintf('Fixture account "%s" is missing.', $email));

        return $account;
    }
}
