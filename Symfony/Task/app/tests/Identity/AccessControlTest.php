<?php

declare(strict_types=1);

namespace App\Tests\Identity;

use App\Tests\Support\FixtureHelpers;
use PHPUnit\Framework\Attributes\DataProvider;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * Epic-level authentication/authorization criteria that have no dedicated
 * user story of their own (requirements-analyst spec, "Acceptance Criteria
 * (Epic-Level)"): AC-01-68 (RBAC — correct dashboard per role), AC-01-69
 * (session management), AC-01-70 (no access outside role permissions).
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md AC-01-68, AC-01-69, AC-01-70, BR-01-3
 */
final class AccessControlTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-01-68: role-based access control is enforced so each role reaches
     * its correct dashboard — measured here as each role's dashboard
     * exposing that role's own navigation and never another role's
     * exclusive links (identity/dashboard.html.twig's role.value branches).
     */
    /**
     * @param list<string> $formerRoleOnlyLinkTexts
     */
    #[DataProvider('roleNavigationProvider')]
    public function testEachRoleSeesOnlyItsOwnDashboardNavigation(
        string $email,
        string $expectedLinkText,
        array $formerRoleOnlyLinkTexts,
    ): void {
        $this->client->loginUser($this->account($email));

        $this->client->request('GET', '/dashboard');
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', $expectedLinkText);

        foreach ($formerRoleOnlyLinkTexts as $foreignLinkText) {
            self::assertSelectorTextNotContains('nav', $foreignLinkText, sprintf(
                'AC-01-68: %s should not see the %s-only "%s" link.',
                $email,
                $foreignLinkText,
                $foreignLinkText,
            ));
        }
    }

    /**
     * @return iterable<string, array{string, string, list<string>}>
     */
    public static function roleNavigationProvider(): iterable
    {
        yield 'super admin sees Users, not role-specific tools' => [
            'admin@practiceperfect.test', 'Users', ['Coaches', 'ShareLinks', 'My Times', 'Family'],
        ];
        yield 'trainer sees Coaches/ShareLinks/Branding, not Users or Family' => [
            'trainer@practiceperfect.test', 'ShareLinks', ['Users', 'Family'],
        ];
        yield 'coach sees My Times, not Users or Family' => [
            'coach@practiceperfect.test', 'My Times', ['Users', 'Family', 'ShareLinks'],
        ];
        yield 'player sees Family, not Users or trainer tools' => [
            'player@practiceperfect.test', 'Family', ['Users', 'ShareLinks', 'Coaches'],
        ];
    }

    /**
     * AC-01-69 (login, logout): logging out actually ends the authenticated
     * session server-side — the same browser session that could reach a
     * protected page is refused afterward, not merely "forgotten" client-side.
     */
    public function testLoggingOutEndsTheAuthenticatedSession(): void
    {
        $this->client->loginUser($this->account('player@practiceperfect.test'));

        $this->client->request('GET', '/dashboard');
        self::assertResponseIsSuccessful();

        $this->client->request('GET', '/logout');
        self::assertResponseRedirects('/login');

        // Same client, same cookie jar: if the session were merely
        // client-side, this would still succeed.
        $this->client->request('GET', '/dashboard');
        self::assertResponseRedirects('/login', 302, 'AC-01-69: logout must end the session server-side, not just redirect once.');
    }

    /**
     * AC-01-69 (session expiry), documented rather than asserted with a
     * concrete duration: BR-01-3 requires sessions to "expire after a
     * reasonable period of inactivity", but Q-01.07 ("Session timeout: How
     * long should users stay logged in? (1 day, 7 days, 30 days?)") is
     * logged as an OPEN, client-owned, P2 question with no answer given.
     * Asserting a specific number here would fabricate a requirement the
     * client has not actually set. What IS verifiable without inventing a
     * number: the mechanism that would enforce whatever value is eventually
     * chosen is in place (framework.session is enabled, so PHP's own
     * inactivity-based garbage collection governs expiry today) — the gap is
     * a business decision, not missing plumbing.
     *
     * @see specs/requirements-analyst-epic-01-user-management-spec.md BR-01-3, Q-01.07
     */
    public function testSessionExpiryDurationIsAnOpenClientQuestionNotAFixedRequirement(): void
    {
        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $this->client->request('GET', '/dashboard');

        self::assertNotCount(
            0,
            $this->client->getCookieJar()->all(),
            'The session mechanism BR-01-3 depends on (framework.session: true) is actually in effect — a session cookie is set. The still-open part is only which inactivity duration Q-01.07 eventually settles on, which is not this suite\'s decision to make up.',
        );
    }

    /**
     * AC-01-70: a role can never reach another role's exclusive area, tested
     * as a full matrix rather than the single spot-checks scattered across
     * other test files — every one of the four role-gated route prefixes,
     * attempted by every role that is NOT supposed to hold it.
     */
    #[DataProvider('crossRoleAccessProvider')]
    public function testARoleCannotReachAnotherRolesExclusiveRoute(string $email, string $forbiddenPath): void
    {
        $this->client->loginUser($this->account($email));

        $this->client->request('GET', $forbiddenPath);

        self::assertResponseStatusCodeSame(403, sprintf('AC-01-70: %s must not reach %s.', $email, $forbiddenPath));
    }

    /**
     * @return iterable<string, array{string, string}>
     */
    public static function crossRoleAccessProvider(): iterable
    {
        $roles = [
            'admin@practiceperfect.test' => '/super-admin/users',
            'trainer@practiceperfect.test' => '/trainer/coaches',
            'coach@practiceperfect.test' => '/coach/availability',
            'player@practiceperfect.test' => '/portal/family',
        ];

        foreach ($roles as $ownerEmail => $exclusivePath) {
            foreach (array_keys($roles) as $otherEmail) {
                if ($otherEmail === $ownerEmail) {
                    continue;
                }

                yield sprintf('%s cannot reach %s', $otherEmail, $exclusivePath) => [$otherEmail, $exclusivePath];
            }
        }
    }
}
