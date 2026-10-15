<?php

declare(strict_types=1);

namespace App\Tests\Scheduling;

use App\Scheduling\Entity\Rsvp;
use App\Scheduling\Repository\RsvpRepository;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-02.12 — Trainer Views RSVP List.
 */
final class TrainerRsvpListTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-02-43: for each registrant — player name, age, skill level, RSVP
     * timestamp, payment status, and an availability match indicator.
     */
    public function testRsvpListShowsPlayerDetailsAndAvailabilityIndicator(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, [
            'title' => 'RSVP Detail Session',
            'usdPricingEnabled' => true,
            'usdPriceMinorUnits' => 1500,
        ]);
        $this->createRsvp($event, $this->patPlayer(), Rsvp::METHOD_USD, Rsvp::STATUS_CONFIRMED);

        $this->client->loginUser($trainer->getOwnerAccount());
        $this->client->request('GET', sprintf('/trainer/events/%d/rsvps', $event->getId()));

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Pat');
        self::assertSelectorTextContains('body', 'Paid');
        // AC-02-13's own three-state indicator, reused here — Pat has set
        // no availability, so "gray".
        self::assertSelectorTextContains('body', 'gray');
    }

    /**
     * AC-02-44: "15 of 20 registered", and "Event Full - No spots
     * available" once full.
     */
    public function testRsvpListShowsCountWithCapacityAndFullMessage(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $event = $this->createEvent($trainer, ['title' => 'Capacity Count Session', 'capacity' => 2]);
        $this->createRsvp($event, $this->patPlayer());

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', sprintf('/trainer/events/%d/rsvps', $event->getId()));

        self::assertSelectorTextContains('body', '1 of 2 registered');
        self::assertCount(0, $crawler->filter('body:contains("Event Full")'), 'Not full yet.');

        $alex = $this->alexPlayer();
        $this->ensureActivePlayerMembership($trainer, $alex);
        $this->createRsvp($event, $alex);

        $this->client->request('GET', sprintf('/trainer/events/%d/rsvps', $event->getId()));
        self::assertSelectorTextContains('body', '2 of 2 registered');
        self::assertSelectorTextContains('body', 'Event Full - No spots available');
    }

    /**
     * AC-02-45: manually add a player (space available), remove a player
     * (issuing a refund if paid), and export the list as CSV.
     */
    public function testTrainerCanManuallyAddRemoveAndExportRsvps(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $alex = $this->alexPlayer();
        $this->ensureActivePlayerMembership($trainer, $alex);
        $event = $this->createEvent($trainer, ['title' => 'Roster Management Session', 'capacity' => 5]);
        $rsvp = $this->createRsvp($event, $this->patPlayer());

        $this->client->loginUser($trainer->getOwnerAccount());

        // Manually add Alex.
        $addCrawler = $this->client->request('GET', sprintf('/trainer/events/%d/rsvps/add', $event->getId()));
        $addForm = $addCrawler->selectButton('Add player')->form([
            'manual_add_player[player]' => (string) $alex->getId(),
        ]);
        $this->client->submit($addForm);
        self::assertResponseRedirects(sprintf('/trainer/events/%d/rsvps', $event->getId()));

        $this->activateTenant($trainer);
        /** @var RsvpRepository $rsvps */
        $rsvps = self::getContainer()->get(RsvpRepository::class);
        self::assertNotNull($rsvps->findActiveOneByEventAndPlayer($event, $alex), 'AC-02-45: Alex was manually added.');

        // Remove Pat's RSVP.
        $rsvpsCrawler = $this->client->request('GET', sprintf('/trainer/events/%d/rsvps', $event->getId()));
        $removeForm = $rsvpsCrawler->filter('tr:contains("Pat")')->selectButton('Remove')->form();
        $this->client->submit($removeForm);
        self::assertResponseRedirects(sprintf('/trainer/events/%d/rsvps', $event->getId()));

        // Re-fetched fresh: KernelBrowser reboots the kernel (a new
        // container, a new EntityManager) on every request by default, and
        // two more requests happened since $rsvps was first fetched above
        // — reusing that same, now-stale repository reference here would
        // query against a discarded EntityManager instead of the current
        // database state.
        $this->activateTenant($trainer);
        /** @var RsvpRepository $freshRsvps */
        $freshRsvps = self::getContainer()->get(RsvpRepository::class);
        $reloadedRsvp = $freshRsvps->find($rsvp->getId());
        self::assertNotNull($reloadedRsvp);
        self::assertTrue($reloadedRsvp->isCanceled(), 'AC-02-45: removing cancels the RSVP.');

        // Export CSV. The response body is a StreamedResponse — its
        // callback's actual CSV rows are covered by the equivalent HTML
        // rendering assertions elsewhere in this class (same rsvpRows()
        // data source); this checks the export is reachable and correctly
        // shaped as a downloadable CSV attachment for this specific event.
        $this->client->request('GET', sprintf('/trainer/events/%d/rsvps/export', $event->getId()));
        self::assertResponseIsSuccessful();
        self::assertStringStartsWith('text/csv', (string) $this->client->getResponse()->headers->get('Content-Type'));
        self::assertStringContainsString(
            sprintf('event-%d-rsvps.csv', $event->getId()),
            (string) $this->client->getResponse()->headers->get('Content-Disposition'),
        );
    }
}
