<?php

declare(strict_types=1);

namespace App\Identity\Controller;

use App\Identity\Entity\Account;
use App\Identity\Entity\ParentChildLink;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Exception\ShareLinkNotUsableException;
use App\Identity\Form\AddTrainerType;
use App\Identity\Form\ChildProfileType;
use App\Identity\Form\TokenApprovalToggleType;
use App\Identity\Repository\ParentChildLinkRepository;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Identity\Service\ChildProfileService;
use App\Identity\Voter\ChildProfileVoter;
use App\Platform\Entity\Trainer;
use App\Platform\Repository\AccountTrainerLinkRepository;
use App\Platform\Repository\TrainerRepository;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\Form\FormError;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-01.03/US-01.04: "Family"/"Player Profiles" — child creation and
 * ongoing trainer association management.
 */
#[IsGranted('ROLE_PLAYER')]
final class FamilyController extends AbstractController
{
    public function __construct(
        private readonly ParentChildLinkRepository $parentChildLinks,
        private readonly PlayerTrainerMembershipRepository $playerTrainerMemberships,
        private readonly AccountTrainerLinkRepository $accountTrainerLinks,
        private readonly TrainerRepository $trainers,
        private readonly ChildProfileService $childProfileService,
    ) {
    }

    /**
     * AC-01-22: every child with their trainer associations.
     */
    #[Route('/portal/family', name: 'identity_portal_family_index', methods: ['GET'])]
    public function index(): Response
    {
        /** @var Account $parent */
        $parent = $this->getUser();

        $links = $this->parentChildLinks->findByParent($parent);
        $memberships = [];

        foreach ($links as $link) {
            $memberships[(int) $link->getChildPlayer()->getId()] = $this->playerTrainerMemberships->findActiveForPlayer($link->getChildPlayer());
        }

        return $this->render('identity/family_index.html.twig', [
            'links' => $links,
            'now' => new \DateTimeImmutable(),
            'membershipsByChild' => $memberships,
        ]);
    }

    /**
     * AC-01-16/21: create a child profile. Trainer association is a
     * deliberately separate follow-up step (AC-01-17), not bundled into this
     * form.
     */
    #[Route('/portal/family/children/new', name: 'identity_portal_child_create', methods: ['GET', 'POST'])]
    public function create(Request $request): Response
    {
        $this->denyAccessUnlessGranted(ChildProfileVoter::CHILD_PROFILE_CREATE);

        /** @var Account $parent */
        $parent = $this->getUser();

        $form = $this->createForm(ChildProfileType::class);
        $form->handleRequest($request);
        $duplicates = [];

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{firstName: string, dateOfBirth: \DateTimeInterface, gender: ?string, schoolOrTeam: ?string} $data */
            $data = $form->getData();

            // AC-01-21: a non-blocking duplicate warning.
            $duplicates = $this->childProfileService->findPossibleDuplicates($parent, $data['firstName']);

            try {
                $this->childProfileService->createChild(
                    $parent,
                    $data['firstName'],
                    \DateTimeImmutable::createFromInterface($data['dateOfBirth']),
                    $data['gender'],
                    $data['schoolOrTeam'],
                );

                $this->addFlash('success', sprintf('%s has been added.', $data['firstName']));

                return $this->redirectToRoute('identity_portal_family_index');
            } catch (\InvalidArgumentException $e) {
                $form->addError(new FormError($e->getMessage()));
            }
        }

        return $this->render('identity/child_create.html.twig', ['form' => $form, 'duplicates' => $duplicates]);
    }

    #[Route('/portal/family/children/{child}/edit', name: 'identity_portal_child_edit', methods: ['GET', 'POST'])]
    public function edit(Request $request, PlayerProfile $child): Response
    {
        $this->denyAccessUnlessGranted(ChildProfileVoter::CHILD_PROFILE_EDIT, $child);

        $form = $this->createForm(ChildProfileType::class, [
            'firstName' => $child->getFirstName(),
            'dateOfBirth' => $child->getDateOfBirth(),
            'gender' => $child->getGender(),
            'schoolOrTeam' => $child->getSchoolOrTeam(),
        ]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{firstName: string, gender: ?string, schoolOrTeam: ?string} $data */
            $data = $form->getData();
            $child->updateProfile($data['firstName'], $data['gender'], $data['schoolOrTeam'], null, null, null);

            $this->addFlash('success', 'Saved.');

            return $this->redirectToRoute('identity_portal_family_index');
        }

        return $this->render('identity/child_edit.html.twig', ['form' => $form, 'child' => $child]);
    }

    /**
     * AC-01-23: by manually-entered code, or by picking from "My Trainers".
     */
    #[Route('/portal/family/children/{child}/trainers/add', name: 'identity_portal_child_trainer_add', methods: ['GET', 'POST'])]
    public function addTrainer(Request $request, PlayerProfile $child): Response
    {
        $this->denyAccessUnlessGranted(ChildProfileVoter::CHILD_TRAINER_ADD, $child);

        /** @var Account $parent */
        $parent = $this->getUser();
        // AC-01-23 "My Trainers": every trainer the PARENT's own account is
        // already actively linked to, regardless of the parent's role in
        // that tenant (they may themselves be a player there).
        $myTrainers = array_map(
            static fn ($link) => $link->getTrainer(),
            $this->accountTrainerLinks->findActiveFor($parent),
        );

        $form = $this->createForm(AddTrainerType::class, null, ['myTrainers' => $myTrainers]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{code: ?string, trainer: ?Trainer} $data */
            $data = $form->getData();

            try {
                if (null !== ($data['trainer'] ?? null)) {
                    $this->childProfileService->addChildToExistingTrainer($parent, $child, $data['trainer']);
                } elseif (null !== $data['code'] && '' !== trim($data['code'])) {
                    $this->childProfileService->addChildToTrainerByCode($child, $data['code']);
                } else {
                    throw new \InvalidArgumentException('Enter a code or select a trainer.');
                }

                $this->addFlash('success', sprintf('%s is now connected with this trainer.', $child->getFirstName()));

                return $this->redirectToRoute('identity_portal_family_index');
            } catch (ShareLinkNotUsableException|\InvalidArgumentException $e) {
                $form->addError(new FormError($e->getMessage()));
            }
        }

        return $this->render('identity/child_trainer_add.html.twig', ['form' => $form, 'child' => $child]);
    }

    /**
     * AC-01-24: soft-removal, with a confirmation step rendered before this
     * POST is reachable.
     */
    #[Route('/portal/family/children/{child}/trainers/{trainer}/remove', name: 'identity_portal_child_trainer_remove', methods: ['GET', 'POST'])]
    public function removeTrainer(Request $request, PlayerProfile $child, int $trainer): Response
    {
        $this->denyAccessUnlessGranted(ChildProfileVoter::CHILD_TRAINER_REMOVE, $child);

        $membership = $this->playerTrainerMemberships->findOneByTrainerAndPlayer(
            $this->trainers->find($trainer) ?? throw $this->createNotFoundException(),
            $child,
        );

        if (null === $membership || !$membership->isActive()) {
            throw $this->createNotFoundException();
        }

        if ($request->isMethod('POST')) {
            if (!$this->isCsrfTokenValid('remove-trainer'.$membership->getId(), $request->request->getString('_token'))) {
                throw $this->createAccessDeniedException('Invalid CSRF token.');
            }

            $this->childProfileService->removeChildFromTrainer($membership);
            $this->addFlash('success', sprintf('%s has been removed from this trainer.', $child->getFirstName()));

            return $this->redirectToRoute('identity_portal_family_index');
        }

        // GET renders the confirmation warning about cancelling upcoming
        // RSVPs before the destructive POST is ever reachable.
        return $this->render('identity/child_trainer_remove_confirm.html.twig', ['child' => $child, 'membership' => $membership]);
    }

    /**
     * AC-01-27/28.
     */
    #[Route('/portal/family/children/{child}/token-approval', name: 'identity_portal_child_token_approval', methods: ['GET', 'POST'])]
    public function tokenApproval(Request $request, PlayerProfile $child): Response
    {
        $this->denyAccessUnlessGranted(ChildProfileVoter::CHILD_TOKEN_APPROVAL_EDIT, $child);

        $link = $this->parentChildLinks->findByChildPlayer($child) ?? throw $this->createNotFoundException();

        $form = $this->createForm(TokenApprovalToggleType::class, ['allowed' => $link->allowsTokenSpendingWithoutApproval()]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{allowed: bool} $data */
            $data = $form->getData();
            $this->childProfileService->setTokenApprovalBypass($link, $data['allowed']);

            $this->addFlash('success', 'Saved.');

            return $this->redirectToRoute('identity_portal_family_index');
        }

        return $this->render('identity/child_token_approval.html.twig', ['form' => $form, 'child' => $child]);
    }

    /**
     * AC-01-18: the child context switcher.
     */
    #[Route('/portal/context/child/{child}', name: 'identity_portal_context_child_switch', methods: ['POST'])]
    public function switchChildContext(Request $request, PlayerProfile $child): Response
    {
        $this->denyAccessUnlessGranted(ChildProfileVoter::CHILD_PROFILE_VIEW, $child);

        if (!$this->isCsrfTokenValid('child-context'.$child->getId(), $request->request->getString('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }

        $session = $request->getSession();
        $session->set('current_player_context_id', $child->getId());

        return $this->redirect($request->headers->get('referer') ?? $this->generateUrl('app_dashboard'));
    }
}
