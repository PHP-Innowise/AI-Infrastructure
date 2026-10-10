<?php

declare(strict_types=1);

namespace App\Tests\Platform;

use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * AC-01-74, coach half, at the HTTP layer (the player/membership half is
 * TenancyIsolationTest, at the repository layer): a trainer's Coaches list
 * shows only coaches active under that trainer's own organization.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md AC-01-74
 */
final class CoachListTenancyTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    public function testATrainerSeesOnlyTheirOwnCoachesNeverAnotherTrainers(): void
    {
        // Fixtures: Casey Coach is active under Peak Performance only.
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', '/trainer/coaches');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Casey Coach');

        $this->client->loginUser($this->account('trainer-b@practiceperfect.test'));
        $this->client->request('GET', '/trainer/coaches');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextNotContains('body', 'Casey Coach', 'AC-01-74: Baseline Athletics must not see Peak Performance\'s coach.');
        self::assertSelectorTextContains('body', 'No coaches yet.');
    }
}
