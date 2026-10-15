<?php

declare(strict_types=1);

namespace App\Tests\Scheduling;

use App\Scheduling\Entity\Event;
use App\Scheduling\Repository\EventRepository;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * "Dual Pricing Options" / "Flexible Token Pricing" — In Scope (MVP), Event
 * Creation & Management (Trainer).
 */
final class DualPricingTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-02-58, AC-05-30: by default, token pricing is enabled at 1 token
     * and USD pricing is disabled at $0 — verified against the real
     * Create Event form's own pre-filled defaults, not a test helper's
     * shortcut. AC-05-30 restates AC-02-58's own default from Epic-05's
     * "payment-method defaults" angle; the event-side toggle/display
     * mechanics stay Epic-02's (that criterion's own cross-epic note).
     */
    public function testNewEventDefaultsToTokenPricingOnAndUsdOff(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', '/trainer/events/new');

        self::assertTrue($crawler->filter('#event_tokenPricingEnabled')->attr('checked') !== null, 'AC-02-58: token pricing defaults ON.');
        self::assertNull($crawler->filter('#event_usdPricingEnabled')->attr('checked'), 'AC-02-58: USD pricing defaults OFF.');
        self::assertSame('1', $crawler->filter('#event_tokenPrice')->attr('value'), 'AC-02-58: default 1 token.');

        // Submitted untouched, the created event carries exactly that
        // default.
        $form = $crawler->selectButton('Create event')->form([
            'event[title]' => 'Dual Pricing Default Session',
            'event[startsAt]' => (new \DateTimeImmutable('+3 days'))->format('Y-m-d\TH:i'),
            'event[endsAt]' => (new \DateTimeImmutable('+3 days +1 hour'))->format('Y-m-d\TH:i'),
            'event[location]' => 'Court D',
            'event[capacity]' => '10',
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var EventRepository $events */
        $events = self::getContainer()->get(EventRepository::class);
        $created = current(array_filter($events->findAllForActiveTenant(), static fn (Event $e): bool => 'Dual Pricing Default Session' === $e->getTitle()));
        self::assertNotFalse($created);
        self::assertTrue($created->isTokenPricingEnabled());
        self::assertSame(1, $created->getTokenPrice());
        self::assertFalse($created->isUsdPricingEnabled());
        self::assertSame(0, $created->getUsdPriceMinorUnits());
    }

    /**
     * AC-02-58, AC-05-30: both a USD price and a token price at once
     * ("Dual Pricing") — the trainer toggles either on or off per event,
     * independently.
     */
    public function testEventCanBeConfiguredWithBothUsdAndTokenPricingAtOnce(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', '/trainer/events/new');
        $form = $crawler->selectButton('Create event')->form([
            'event[title]' => 'Dual Pricing Both On Session',
            'event[startsAt]' => (new \DateTimeImmutable('+3 days'))->format('Y-m-d\TH:i'),
            'event[endsAt]' => (new \DateTimeImmutable('+3 days +1 hour'))->format('Y-m-d\TH:i'),
            'event[location]' => 'Court E',
            'event[capacity]' => '10',
            'event[usdPricingEnabled]' => true,
            'event[usdPrice]' => '12',
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var EventRepository $events */
        $events = self::getContainer()->get(EventRepository::class);
        $created = current(array_filter($events->findAllForActiveTenant(), static fn (Event $e): bool => 'Dual Pricing Both On Session' === $e->getTitle()));
        self::assertNotFalse($created);
        self::assertTrue($created->isUsdPricingEnabled());
        self::assertTrue($created->isTokenPricingEnabled(), 'AC-02-58: token pricing (the default) stays on alongside USD.');
        self::assertSame([Event::PAYMENT_USD, Event::PAYMENT_TOKEN], $created->availablePaymentMethods());
    }

    /**
     * AC-02-59, AC-05-32: token pricing is not fixed at 1:1 — each
     * trainer configures their own per-event token cost independently,
     * e.g. a premium session at 2+ tokens.
     */
    public function testEventCanBePricedAtMoreThanOneToken(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        $this->client->loginUser($trainer->getOwnerAccount());
        $crawler = $this->client->request('GET', '/trainer/events/new');
        $form = $crawler->selectButton('Create event')->form([
            'event[title]' => 'Premium Multi Token Session',
            'event[startsAt]' => (new \DateTimeImmutable('+3 days'))->format('Y-m-d\TH:i'),
            'event[endsAt]' => (new \DateTimeImmutable('+3 days +3 hours'))->format('Y-m-d\TH:i'),
            'event[location]' => 'Court F',
            'event[capacity]' => '10',
            'event[tokenPrice]' => '3',
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var EventRepository $events */
        $events = self::getContainer()->get(EventRepository::class);
        $created = current(array_filter($events->findAllForActiveTenant(), static fn (Event $e): bool => 'Premium Multi Token Session' === $e->getTitle()));
        self::assertNotFalse($created);
        self::assertSame(3, $created->getTokenPrice(), 'AC-02-59: not fixed at 1 token.');
        self::assertSame(3, $created->priceForMethod(Event::PAYMENT_TOKEN));
    }
}
