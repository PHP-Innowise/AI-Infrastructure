<?php

declare(strict_types=1);

namespace App\Tests\Identity;

use App\Identity\Entity\ChildApprovalRequest;
use App\Identity\Repository\ChildApprovalRequestRepository;
use App\Identity\Repository\ParentChildLinkRepository;
use App\Identity\Repository\PlayerProfileRepository;
use App\Identity\Service\ChildApprovalService;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\Mime\Email;

/**
 * US-01.05 — Child Purchase Requires Parent Approval.
 *
 * These tests exercise the generic decision lifecycle and toggle Epic-01
 * owns outright — `requestApproval()` directly, never through a real
 * RSVP/purchase HTTP flow (Epic-02's RsvpTest/Epic-05's TokenPurchaseTest
 * exercise those triggers end to end instead). The fixture request used
 * here is created with `ChildApprovalRequest::ACTION_TOKEN_PURCHASE` but no
 * `requestedTokenPackageId` — Epic-05's `PortalTokenController` now owns
 * real post-decision handling for that action type
 * (`ApprovalController::decide()`'s own redirect), which the two
 * decision tests below follow through to, exactly as a real browser
 * clicking the approvals-index link would; `completePurchaseAfterApproval()`
 * no-ops for a request with no package id, so the underlying decision
 * mechanics these tests actually assert on are unaffected.
 */
final class ChildApprovalTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-01-25: a request lands in "Pending Parent Approval" and the parent
     * is notified by email (in-app is the same inbox AC-01-26 renders).
     */
    public function testChildRequestIsPendingAndParentIsNotifiedByEmail(): void
    {
        $request = $this->createPendingRequest();

        self::assertTrue($request->isPending(), 'AC-01-25: the request starts Pending Parent Approval.');
        self::assertQueuedEmailCount(1);
        $email = self::getMailerMessage(0);
        self::assertInstanceOf(Email::class, $email);
        self::assertEmailAddressContains($email, 'To', 'player@practiceperfect.test');
    }

    /**
     * AC-01-26, BR-01-20: the parent approves a pending request, optionally
     * with a note, and the child is notified.
     */
    public function testParentApprovesAPendingRequest(): void
    {
        $requestId = $this->createPendingRequest()->getId();
        $parent = $this->account('player@practiceperfect.test');
        $this->client->loginUser($parent);

        $crawler = $this->client->request('GET', '/portal/approvals');
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'pending');

        // The generic route redirects to Billing's own type-specific one
        // for an ACTION_TOKEN_PURCHASE request (see this class's own
        // docblock) — followed here exactly as a real browser would.
        $this->client->request('GET', sprintf('/portal/approvals/%d/approve', $requestId));
        $approveCrawler = $this->client->followRedirect();
        $form = $approveCrawler->selectButton('Approve')->form(['approval_decision[note]' => 'Sounds good!']);
        $this->client->submit($form);

        self::assertResponseRedirects('/portal/approvals');

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var ChildApprovalRequestRepository $requests */
        $requests = self::getContainer()->get(ChildApprovalRequestRepository::class);
        $reloaded = $requests->find($requestId);
        self::assertNotNull($reloaded);
        self::assertSame(ChildApprovalRequest::STATUS_APPROVED, $reloaded->getStatus());
        self::assertSame('Sounds good!', $reloaded->getParentNote(), 'BR-01-20: notes are captured on the decision.');
    }

    /**
     * AC-01-26: the parent can deny a request, and the child is notified.
     */
    public function testParentDeniesAPendingRequest(): void
    {
        $requestId = $this->createPendingRequest()->getId();
        $parent = $this->account('player@practiceperfect.test');
        $this->client->loginUser($parent);

        // See testParentApprovesAPendingRequest()'s own comment on why this
        // follows a redirect first.
        $this->client->request('GET', sprintf('/portal/approvals/%d/deny', $requestId));
        $denyCrawler = $this->client->followRedirect();
        $form = $denyCrawler->selectButton('Deny')->form();
        $this->client->submit($form);

        self::assertResponseRedirects();

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var ChildApprovalRequestRepository $requests */
        $requests = self::getContainer()->get(ChildApprovalRequestRepository::class);
        $reloaded = $requests->find($requestId);
        self::assertNotNull($reloaded);
        self::assertSame(ChildApprovalRequest::STATUS_DENIED, $reloaded->getStatus());
    }

    /**
     * AC-01-27: default OFF — token purchases follow the approval workflow
     * unless the parent has explicitly enabled the bypass for this child.
     */
    public function testTokenApprovalBypassDefaultsToOff(): void
    {
        $parent = $this->account('player@practiceperfect.test');
        /** @var ParentChildLinkRepository $links */
        $links = self::getContainer()->get(ParentChildLinkRepository::class);
        $alexLink = current(array_filter(
            $links->findByParent($parent),
            static fn ($link) => 'Alex' === $link->getChildPlayer()->getFirstName(),
        ));
        self::assertNotFalse($alexLink);

        self::assertFalse($alexLink->allowsTokenSpendingWithoutApproval(), 'AC-01-27: default OFF.');
    }

    /**
     * AC-01-27/28: the parent can change the per-child setting at any time
     * from the child's profile settings.
     */
    public function testParentChangesTheTokenApprovalSetting(): void
    {
        $parent = $this->account('player@practiceperfect.test');
        $this->client->loginUser($parent);
        $childId = $this->findChildIdByFirstName($parent, 'Alex');

        $crawler = $this->client->request('GET', sprintf('/portal/family/children/%d/token-approval', $childId));
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Save')->form([
            'token_approval_toggle[allowed]' => true,
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();

        $reloadedParent = $this->account('player@practiceperfect.test');
        /** @var ParentChildLinkRepository $links */
        $links = self::getContainer()->get(ParentChildLinkRepository::class);
        $alexLink = current(array_filter(
            $links->findByParent($reloadedParent),
            static fn ($link) => 'Alex' === $link->getChildPlayer()->getFirstName(),
        ));
        self::assertNotFalse($alexLink);
        self::assertTrue($alexLink->allowsTokenSpendingWithoutApproval(), 'AC-01-28: the parent changed the setting.');

        // And can change it back at any time.
        $crawler = $this->client->request('GET', sprintf('/portal/family/children/%d/token-approval', $childId));
        $form = $crawler->selectButton('Save')->form([
            'token_approval_toggle[allowed]' => false,
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects();

        $links2 = self::getContainer()->get(ParentChildLinkRepository::class);
        $alexLink2 = current(array_filter(
            $links2->findByParent($this->account('player@practiceperfect.test')),
            static fn ($link) => 'Alex' === $link->getChildPlayer()->getFirstName(),
        ));
        self::assertNotFalse($alexLink2);
        self::assertFalse($alexLink2->allowsTokenSpendingWithoutApproval(), 'AC-01-28: changeable back at any time.');
    }

    private function createPendingRequest(): ChildApprovalRequest
    {
        $trainer = $this->trainer('peak-performance');
        $parent = $this->account('player@practiceperfect.test');

        /** @var ParentChildLinkRepository $links */
        $links = self::getContainer()->get(ParentChildLinkRepository::class);
        $alexLink = current(array_filter(
            $links->findByParent($parent),
            static fn ($link) => 'Alex' === $link->getChildPlayer()->getFirstName(),
        ));
        self::assertNotFalse($alexLink);

        /** @var ChildApprovalService $service */
        $service = self::getContainer()->get(ChildApprovalService::class);

        return $service->requestApproval($trainer, $alexLink->getChildPlayer(), ChildApprovalRequest::ACTION_TOKEN_PURCHASE);
    }

    private function findChildIdByFirstName(\App\Identity\Entity\Account $parent, string $firstName): int
    {
        /** @var ParentChildLinkRepository $links */
        $links = self::getContainer()->get(ParentChildLinkRepository::class);
        $link = current(array_filter(
            $links->findByParent($parent),
            static fn ($l) => $firstName === $l->getChildPlayer()->getFirstName(),
        ));
        self::assertNotFalse($link, sprintf('Expected a child named "%s".', $firstName));

        return (int) $link->getChildPlayer()->getId();
    }
}
