<?php

declare(strict_types=1);

namespace App\Tests\Administration;

use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * "In Scope (MVP)" § "Trainer Management" — AC-07-38, AC-07-39.
 */
final class TrainersToolTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-07-38: Super Admin can view a list of all trainers.
     */
    public function testTrainersToolListsEveryTrainer(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $this->client->request('GET', '/super-admin/trainers');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Peak Performance Basketball');
        self::assertSelectorTextContains('body', 'Baseline Athletics');
    }

    /**
     * AC-07-38: the list is searchable by business name (tool-specific
     * search, matching TrainerRepository::search()'s own AC-07-38 intent).
     */
    public function testTrainersToolSearchMatchesBusinessName(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $this->client->request('GET', '/super-admin/trainers', ['q' => 'Baseline']);
        self::assertSelectorTextContains('body', 'Baseline Athletics');
        self::assertSelectorTextNotContains('body', 'Peak Performance Basketball');
    }

    /**
     * AC-07-38: view/edit trainer details, and view the trainer's
     * subscription status.
     */
    public function testTrainerShowPageDisplaysDetailsAndSubscriptionStatusWithAnEditLink(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $crawler = $this->client->request('GET', sprintf('/super-admin/trainers/%d', $trainer->getId()));

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('h1', 'Peak Performance Basketball');
        self::assertSelectorTextContains('body', 'pending', 'AC-07-38: subscription status shown (no subscription provisioned for this fixture trainer).');
        self::assertGreaterThan(0, $crawler->selectLink('View / Edit account')->count());
        self::assertSame(
            sprintf('/super-admin/users/%d', $trainer->getOwnerAccount()->getId()),
            $crawler->selectLink('View / Edit account')->attr('href'),
        );
    }

    /**
     * AC-07-39: Super Admin can deactivate and reactivate trainer accounts,
     * via the same mechanism as any other account (Epic-01 US-01.12) —
     * proven here from the Trainer show page specifically, complementing
     * AccountLifecycleTest's own generic account coverage.
     */
    public function testTrainerShowPageDeactivatesAndReactivatesTheTrainer(): void
    {
        $trainer = $this->trainer('baseline-athletics');
        $admin = $this->account('admin@practiceperfect.test');
        $this->client->loginUser($admin);

        $crawler = $this->client->request('GET', sprintf('/super-admin/trainers/%d', $trainer->getId()));
        $deactivateLink = $crawler->selectLink('Deactivate trainer');
        self::assertGreaterThan(0, $deactivateLink->count(), 'AC-07-39: a deactivate action is reachable from the trainer page.');

        $confirmCrawler = $this->client->click($deactivateLink->link());
        $this->client->submit($confirmCrawler->selectButton('Confirm deactivation')->form());
        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertSelectorTextContains('.flash--success', 'deactivated');

        $showAgain = $this->client->request('GET', sprintf('/super-admin/trainers/%d', $trainer->getId()));
        $reactivateButton = $showAgain->selectButton('Reactivate trainer');
        self::assertGreaterThan(0, $reactivateButton->count(), 'AC-07-39: reactivation is offered once deactivated.');
        $this->client->submit($reactivateButton->form());

        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertSelectorTextContains('.flash--success', 'reactivated');
    }

    /**
     * A Trainer (not Super Admin) cannot reach the Trainers tool.
     */
    public function testATrainerCannotReachTheTrainersTool(): void
    {
        $this->client->loginUser($this->trainer('peak-performance')->getOwnerAccount());
        $this->client->request('GET', '/super-admin/trainers');

        self::assertResponseStatusCodeSame(403);
    }
}
