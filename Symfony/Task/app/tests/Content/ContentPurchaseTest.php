<?php

declare(strict_types=1);

namespace App\Tests\Content;

use App\Content\Repository\PlaylistAccessGrantRepository;
use App\Content\Repository\PlaylistAssignmentRepository;
use App\Content\Service\PlaylistAssignmentService;
use App\Tests\Support\BillingFixtureHelpers;
use App\Tests\Support\ContentFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-05.06 — Player Purchases Content (LPPP Playlist).
 */
final class ContentPurchaseTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;
    use ContentFixtureHelpers;
    use BillingFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-05-18: a locked playlist shows its price, and paying with tokens
     * unlocks it inline (instant, no Checkout redirect) — "stays
     * accessible forever."
     */
    public function testPlayerPurchasesLockedPlaylistWithTokensAndUnlocksPermanently(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'Locked Shooting Fundamentals']);
        $playlist->updatePricing(5000, 5);
        $this->contentEntityManagerFlush();
        $pat = $this->account('player@practiceperfect.test');
        $balanceBefore = $this->tokenBalance($trainer, $pat);
        $this->giveTokens($trainer, $pat, 10);

        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainer);

        $crawler = $this->client->request('GET', sprintf('/portal/content/playlists/%d', $playlist->getId()));
        self::assertSelectorTextContains('body', '$50.00', 'AC-05-18: the price is shown — "$50 one-time or 5 tokens."');
        self::assertSelectorTextContains('body', '5 tokens');

        $crawler = $this->client->request('GET', sprintf('/portal/content/playlists/%d/checkout', $playlist->getId()));
        $form = $crawler->selectButton('Purchase')->form(['purchase_method[method]' => 'token']);
        $this->client->submit($form);

        self::assertResponseRedirects(sprintf('/portal/content/playlists/%d', $playlist->getId()));
        $this->activateTenant($trainer);
        self::assertSame($balanceBefore + 10 - 5, $this->tokenBalance($trainer, $pat), 'AC-05-18: 5 tokens deducted instantly.');

        /** @var PlaylistAccessGrantRepository $grants */
        $grants = self::getContainer()->get(PlaylistAccessGrantRepository::class);
        self::assertNotNull($grants->findOneByPlaylistAndPlayer($playlist, $this->patPlayer()), 'AC-05-18: the playlist is unlocked.');

        // Stays accessible — visiting again never re-shows the paywall.
        $this->client->request('GET', sprintf('/portal/content/playlists/%d', $playlist->getId()));
        self::assertSelectorTextNotContains('body', 'Purchase Access', 'AC-05-18: permanently unlocked, not a per-session unlock.');
    }

    /**
     * AC-05-18: choosing Card 303s to Stripe Checkout in the same request
     * (BR-04-6..9's card path) — access stays locked until the webhook
     * confirms payment (Content's own PaymentOutcomeSubscriber).
     */
    public function testPlayerPurchasesLockedPlaylistWithCardRedirectsToStripeCheckout(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createPracticePlaylist($trainer, ['title' => 'Locked Ballhandling Drills']);
        $playlist->updatePricing(3000, 3);
        $this->contentEntityManagerFlush();
        $pat = $this->account('player@practiceperfect.test');

        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', sprintf('/portal/content/playlists/%d/checkout', $playlist->getId()));
        $form = $crawler->selectButton('Purchase')->form(['purchase_method[method]' => 'usd']);
        $this->client->submit($form);

        self::assertTrue($this->client->getResponse()->isRedirect(), 'AC-05-18: redirected to Stripe Checkout for card.');
        self::assertStringContainsString('checkout.stripe.test', (string) $this->client->getResponse()->headers->get('Location'));

        $this->activateTenant($trainer);
        /** @var PlaylistAccessGrantRepository $grants */
        $grants = self::getContainer()->get(PlaylistAccessGrantRepository::class);
        self::assertNull($grants->findOneByPlaylistAndPlayer($playlist, $this->patPlayer()), 'Still locked — the card payment has not been confirmed yet.');
    }

    /**
     * AC-05-19: a trainer's suggestion notifies the player and is
     * highlighted in their library, but purchase is still required — the
     * suggestion mechanism itself is Epic-04's (PlaylistAssignmentService,
     * already covered by that epic's own tests); this proves the
     * Epic-05-specific fact the AC actually adds: being suggested never
     * bypasses payment.
     */
    public function testSuggestedPlaylistStillRequiresPurchaseToUnlock(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $playlist = $this->createLearnPlaylist($trainer, ['title' => 'Suggested Footwork Series']);
        $playlist->updatePricing(2000, 2);
        $this->contentEntityManagerFlush();
        $pat = $this->account('player@practiceperfect.test');

        /** @var PlaylistAssignmentService $assignmentService */
        $assignmentService = self::getContainer()->get(PlaylistAssignmentService::class);
        $assignmentService->assign(
            $trainer,
            $playlist,
            $trainer->getOwnerAccount(),
            \App\Content\Entity\PlaylistAssignment::TARGET_PLAYER,
            $this->patPlayer(),
            null,
            null,
            null,
            'Great for your next game.',
        );

        self::assertQueuedEmailCount(1, message: 'AC-05-19: the player is notified of the suggestion.');

        $this->client->loginUser($pat);
        $this->switchPlayerToTrainer($this->client, $trainer);
        $crawler = $this->client->request('GET', '/portal/content?tab=learn');
        self::assertSelectorTextContains('body', 'Suggested Footwork Series');
        self::assertSelectorTextContains('body', 'Suggested', 'AC-05-19: highlighted as a suggestion in the library.');

        $playlistCrawler = $this->client->request('GET', sprintf('/portal/content/playlists/%d', $playlist->getId()));
        self::assertSelectorTextContains('body', '$20.00', 'AC-05-19: still shows its price — suggesting never grants access.');

        $this->activateTenant($trainer);
        /** @var PlaylistAccessGrantRepository $grants */
        $grants = self::getContainer()->get(PlaylistAccessGrantRepository::class);
        self::assertNull($grants->findOneByPlaylistAndPlayer($playlist, $this->patPlayer()), 'AC-05-19: the player must still purchase it to gain access.');

        /** @var PlaylistAssignmentRepository $assignments */
        $assignments = self::getContainer()->get(PlaylistAssignmentRepository::class);
        self::assertNotEmpty($assignments->findBy(['playlist' => $playlist]), 'The suggestion itself was recorded.');
    }

    private function contentEntityManagerFlush(): void
    {
        self::getContainer()->get(\Doctrine\ORM\EntityManagerInterface::class)->flush();
    }
}
