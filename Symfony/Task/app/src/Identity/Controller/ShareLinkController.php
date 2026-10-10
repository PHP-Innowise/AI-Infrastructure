<?php

declare(strict_types=1);

namespace App\Identity\Controller;

use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Exception\DuplicateEmailException;
use App\Identity\Form\FamilyMemberSelectionType;
use App\Identity\Form\PlayerRegistrationType;
use App\Identity\Repository\ParentChildLinkRepository;
use App\Identity\Repository\PlayerProfileRepository;
use App\Identity\Repository\ShareLinkRepository;
use App\Identity\Service\IdentityMailer;
use App\Identity\Service\MembershipService;
use App\Identity\Service\PlayerRegistrationService;
use App\Identity\Service\ShareLinkService;
use App\Identity\Voter\ShareLinkVoter;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Bundle\SecurityBundle\Security;
use Symfony\Component\Form\FormError;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\CurrentUser;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-01.02: the player ShareLink landing flow. `/join/{code}` is
 * allow-listed in TenantFromPublicCode, so by the time these actions run the
 * tenant is already resolved and active.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md US-01.02, AC-01-9..15, AC-01-31
 */
final class ShareLinkController extends AbstractController
{
    public function __construct(
        private readonly ShareLinkRepository $shareLinks,
        private readonly ShareLinkService $shareLinkService,
        private readonly PlayerRegistrationService $playerRegistration,
        private readonly MembershipService $membershipService,
        private readonly ParentChildLinkRepository $parentChildLinks,
        private readonly PlayerProfileRepository $playerProfiles,
        private readonly IdentityMailer $mailer,
        private readonly Security $security,
    ) {
    }

    /**
     * AC-01-9: logged out -> registration prompt; logged in -> instant
     * association; logged-in child -> blocked (AC-01-31).
     */
    #[Route('/join/{code}', name: 'identity_sharelink_show', methods: ['GET'])]
    public function show(string $code, #[CurrentUser] ?Account $account): Response
    {
        $shareLink = $this->shareLinks->findOneByCode($code);

        if (null === $shareLink) {
            throw $this->createNotFoundException();
        }

        $this->shareLinkService->recordOpen($shareLink);

        if (!$shareLink->isUsable(new \DateTimeImmutable())) {
            return $this->render('identity/sharelink_unusable.html.twig', ['shareLink' => $shareLink], new Response(status: 410));
        }

        if (null === $account) {
            $form = $this->createForm(PlayerRegistrationType::class);

            return $this->render('identity/sharelink_show.html.twig', ['shareLink' => $shareLink, 'form' => $form]);
        }

        $parentLink = $this->parentChildLinks->findByChildAccount($account);

        if (null !== $parentLink) {
            // AC-01-31: a logged-in child is blocked; the parent is emailed
            // instead, and no association happens until they complete it.
            $this->mailer->sendParentReviewRegistration($parentLink->getParentAccount(), $account, $shareLink->getTrainer(), $code);

            return $this->render('identity/sharelink_child_blocked.html.twig', ['shareLink' => $shareLink]);
        }

        // AC-01-9: already logged in -> redirect straight to association.
        return $this->redirectToRoute('identity_sharelink_associate', ['code' => $code]);
    }

    /**
     * AC-01-10/11/12: new account + player profile + membership + confirmation.
     */
    #[Route('/join/{code}/register', name: 'identity_sharelink_register', methods: ['POST'])]
    public function register(Request $request, string $code): Response
    {
        $shareLink = $this->shareLinks->findOneByCode($code) ?? throw $this->createNotFoundException();
        $this->denyAccessUnlessGranted(ShareLinkVoter::SHARELINK_RESOLVE, $shareLink);

        $form = $this->createForm(PlayerRegistrationType::class);
        $form->handleRequest($request);

        if (!$form->isSubmitted() || !$form->isValid()) {
            return $this->render('identity/sharelink_show.html.twig', ['shareLink' => $shareLink, 'form' => $form]);
        }

        /** @var array{accountFirstName: string, accountLastName: string, email: string, plainPassword: string, parentPhone: ?string, playerFirstName: string, playerDateOfBirth: \DateTimeInterface, playerGender: ?string} $data */
        $data = $form->getData();

        try {
            $account = $this->playerRegistration->registerViaShareLink(
                $shareLink,
                $data['accountFirstName'],
                $data['accountLastName'],
                $data['email'],
                $data['plainPassword'],
                $data['parentPhone'],
                $data['playerFirstName'],
                \DateTimeImmutable::createFromInterface($data['playerDateOfBirth']),
                $data['playerGender'],
            );
        } catch (DuplicateEmailException $e) {
            // AC-01-8: a clear, specific error rather than a generic failure.
            $form->get('email')->addError(new FormError($e->getMessage()));

            return $this->render('identity/sharelink_show.html.twig', ['shareLink' => $shareLink, 'form' => $form]);
        }

        $this->security->login($account, 'form_login', 'main');
        $this->addFlash('success', sprintf('Welcome to %s!', $shareLink->getTrainer()->getBusinessName()));

        return $this->redirectToRoute('app_dashboard');
    }

    /**
     * AC-01-13/14: an already-authenticated player/parent accepting a
     * (possibly second) trainer's link.
     */
    #[Route('/join/{code}/associate', name: 'identity_sharelink_associate', methods: ['GET', 'POST'])]
    #[IsGranted('ROLE_PLAYER')]
    public function associate(Request $request, string $code): Response
    {
        $shareLink = $this->shareLinks->findOneByCode($code) ?? throw $this->createNotFoundException();
        $this->denyAccessUnlessGranted(ShareLinkVoter::SHARELINK_RESOLVE, $shareLink);

        /** @var Account $account */
        $account = $this->getUser();

        // AC-01-30: a child's own login cannot add a new trainer.
        if (null !== $this->parentChildLinks->findByChildAccount($account)) {
            throw $this->createAccessDeniedException();
        }

        $selfPlayer = $this->playerProfiles->findOneForSelfAccount($account);
        $children = array_map(
            static fn ($link) => $link->getChildPlayer(),
            $this->parentChildLinks->findByParent($account),
        );

        $form = $this->createForm(FamilyMemberSelectionType::class, null, ['children' => $children]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{includeSelf: bool, children?: list<\App\Identity\Entity\PlayerProfile>} $data */
            $data = $form->getData();

            if ($data['includeSelf'] && null !== $selfPlayer) {
                $this->membershipService->associatePlayer($shareLink->getTrainer(), $selfPlayer, PlayerTrainerMembership::SOURCE_SHARELINK, $shareLink);
                $this->shareLinkService->recordUse($shareLink);
            }

            foreach ($data['children'] ?? [] as $child) {
                $this->membershipService->associatePlayer($shareLink->getTrainer(), $child, PlayerTrainerMembership::SOURCE_SHARELINK, $shareLink);
                $this->shareLinkService->recordUse($shareLink);
            }

            $this->addFlash('success', sprintf('Connected with %s.', $shareLink->getTrainer()->getBusinessName()));

            return $this->redirectToRoute('app_dashboard');
        }

        return $this->render('identity/sharelink_associate.html.twig', [
            'form' => $form,
            'shareLink' => $shareLink,
            'selfPlayer' => $selfPlayer,
        ]);
    }
}
