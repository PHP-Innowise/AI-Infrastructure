<?php

declare(strict_types=1);

namespace App\Administration\Controller;

use App\Administration\Form\AdminEditAccountType;
use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\AccountStatus;
use App\Identity\Exception\DuplicateEmailException;
use App\Identity\Repository\AccountRepository;
use App\Identity\Repository\PlayerProfileRepository;
use App\Identity\Service\AccountLifecycleService;
use App\Identity\Voter\AccountVoter;
use App\Platform\Repository\AccountTrainerLinkRepository;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\Form\FormError;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * AC-01-72, AC-07-8..17: the Users tool — list/search/view/edit any account,
 * deactivate/reactivate, GDPR-delete. Routed under Administration
 * (Epic-07's console) but calling into Identity's own account services
 * underneath, per api-designer-spec's "Module boundary calls".
 */
#[IsGranted('ROLE_SUPER_ADMIN')]
final class UsersController extends AbstractController
{
    private const PER_PAGE = 50;

    public function __construct(
        private readonly AccountRepository $accounts,
        private readonly AccountLifecycleService $accountLifecycleService,
        private readonly EntityManagerInterface $entityManager,
        private readonly AccountTrainerLinkRepository $accountTrainerLinks,
        private readonly PlayerProfileRepository $playerProfiles,
    ) {
    }

    /**
     * AC-01-72/AC-07-8..10: tool-specific search and filters, not a global
     * search. AC-07-12: 50 users per page with page navigation.
     */
    #[Route('/super-admin/users', name: 'administration_users_index', methods: ['GET'])]
    public function index(Request $request): Response
    {
        $role = $request->query->get('role');
        $status = $request->query->get('status');
        $q = $request->query->get('q');
        $roleFilter = \is_string($role) && '' !== $role ? AccountRole::from($role) : null;
        $statusFilter = \is_string($status) && '' !== $status ? AccountStatus::from($status) : null;
        $queryFilter = \is_string($q) && '' !== $q ? $q : null;
        $page = max(1, (int) $request->query->get('page', 1));
        $accounts = $this->accounts->search($queryFilter, $roleFilter, $statusFilter, self::PER_PAGE, $page);

        // AC-07-10: "associated trainer (for coaches/players)" — resolved
        // per row from the global AccountTrainerLink table (the tenant
        // resolver's own source, reachable with no tenant context needed —
        // see that class's own docblock), never a trainer-scoped query.
        $associatedTrainers = [];
        foreach ($accounts as $account) {
            if (AccountRole::Coach === $account->getRole() || AccountRole::Player === $account->getRole()) {
                $names = array_map(
                    static fn ($link): string => $link->getTrainer()->getBusinessName(),
                    $this->accountTrainerLinks->findActiveFor($account),
                );
                $associatedTrainers[(int) $account->getId()] = implode(', ', $names);
            }
        }

        return $this->render('administration/users_index.html.twig', [
            'accounts' => $accounts,
            'total' => $this->accounts->countMatching($queryFilter, $roleFilter, $statusFilter),
            'page' => $page,
            'perPage' => self::PER_PAGE,
            'roles' => AccountRole::cases(),
            'statuses' => AccountStatus::cases(),
            'associatedTrainers' => $associatedTrainers,
        ]);
    }

    #[Route('/super-admin/users/{account}', name: 'administration_user_show', methods: ['GET'])]
    public function show(Account $account): Response
    {
        $this->denyAccessUnlessGranted(AccountVoter::ACCOUNT_VIEW, $account);

        return $this->render('administration/user_show.html.twig', ['account' => $account]);
    }

    /**
     * AC-01-71, AC-07-13..15.
     */
    #[Route('/super-admin/users/{account}/edit', name: 'administration_user_edit', methods: ['GET', 'POST'])]
    public function edit(Request $request, Account $account): Response
    {
        $this->denyAccessUnlessGranted(AccountVoter::ACCOUNT_EDIT, $account);

        $profile = $account->getProfile();
        // AC-07-14: "player-specific profile details... editable when the
        // user is a player" — only a self-training player's OWN account has
        // a PlayerProfile to edit at all; a parent's Account has none
        // (AC-01-30's own model: the child, not the parent, carries the
        // profile), so this naturally stays empty for a parent account.
        $playerProfile = AccountRole::Player === $account->getRole() ? $this->playerProfiles->findOneForSelfAccount($account) : null;

        $form = $this->createForm(AdminEditAccountType::class, [
            'firstName' => $profile?->getFirstName() ?? '',
            'lastName' => $profile?->getLastName() ?? '',
            'email' => $account->getEmail(),
            'phone' => $profile?->getPhone(),
            'playerGender' => $playerProfile?->getGender(),
            'playerSchoolOrTeam' => $playerProfile?->getSchoolOrTeam(),
        ], ['isPlayer' => null !== $playerProfile]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{firstName: string, lastName: string, email: string, phone: ?string, playerGender: ?string, playerSchoolOrTeam: ?string} $data */
            $data = $form->getData();

            $existing = $this->accounts->findOneByEmail($data['email']);
            if (null !== $existing && $existing->getId() !== $account->getId()) {
                // AC-01-71/BR-01-2: email stays unique across every account,
                // Super Admin edits included — surfaced as a form error
                // rather than a raw DB constraint violation.
                $form->get('email')->addError(new FormError(DuplicateEmailException::forEmail($data['email'])->getMessage()));

                return $this->render('administration/user_edit.html.twig', ['form' => $form, 'account' => $account]);
            }

            $account->changeEmail($data['email']);
            $profile?->updateCommonFields($data['firstName'], $data['lastName'], $data['phone'], $profile->getSchoolOrOrganization());

            if (null !== $playerProfile) {
                // AC-01-48's own read-only date of birth is preserved
                // unchanged — see AdminEditAccountType's own docblock.
                $playerProfile->updateProfile(
                    $data['firstName'],
                    $data['playerGender'],
                    $data['playerSchoolOrTeam'],
                    $playerProfile->getJerseyNumber(),
                    $playerProfile->getEmergencyContactName(),
                    $playerProfile->getEmergencyContactPhone(),
                );
            }

            $this->entityManager->flush();
            $this->addFlash('success', 'Account updated.');

            return $this->redirectToRoute('administration_user_show', ['account' => $account->getId()]);
        }

        return $this->render('administration/user_edit.html.twig', ['form' => $form, 'account' => $account]);
    }

    /**
     * AC-01-52/53.
     */
    #[Route('/super-admin/users/{account}/deactivate', name: 'administration_user_deactivate', methods: ['GET', 'POST'])]
    public function deactivate(Request $request, Account $account): Response
    {
        $this->denyAccessUnlessGranted(AccountVoter::ACCOUNT_DEACTIVATE, $account);

        if ($request->isMethod('POST')) {
            if (!$this->isCsrfTokenValid('deactivate'.$account->getId(), $request->request->getString('_token'))) {
                throw $this->createAccessDeniedException('Invalid CSRF token.');
            }

            /** @var Account $actor */
            $actor = $this->getUser();
            $this->accountLifecycleService->deactivate($actor, $account);
            $this->addFlash('success', 'Account deactivated. Historical data is preserved.');

            return $this->redirectToRoute('administration_user_show', ['account' => $account->getId()]);
        }

        return $this->render('administration/user_deactivate_confirm.html.twig', ['account' => $account]);
    }

    /**
     * AC-01-54.
     */
    #[Route('/super-admin/users/{account}/reactivate', name: 'administration_user_reactivate', methods: ['POST'])]
    public function reactivate(Request $request, Account $account): Response
    {
        $this->denyAccessUnlessGranted(AccountVoter::ACCOUNT_REACTIVATE, $account);

        if (!$this->isCsrfTokenValid('reactivate'.$account->getId(), $request->request->getString('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }

        /** @var Account $actor */
        $actor = $this->getUser();
        $this->accountLifecycleService->reactivate($actor, $account);
        $this->addFlash('success', 'Account reactivated.');

        return $this->redirectToRoute('administration_user_show', ['account' => $account->getId()]);
    }

    /**
     * AC-01-55..59.
     */
    #[Route('/super-admin/users/{account}/delete', name: 'administration_user_delete', methods: ['GET', 'POST'])]
    public function delete(Request $request, Account $account): Response
    {
        $this->denyAccessUnlessGranted(AccountVoter::ACCOUNT_DELETE_GDPR, $account);

        if ($request->isMethod('POST')) {
            if (!$this->isCsrfTokenValid('delete'.$account->getId(), $request->request->getString('_token'))) {
                throw $this->createAccessDeniedException('Invalid CSRF token.');
            }

            /** @var Account $actor */
            $actor = $this->getUser();
            $reason = $request->request->get('reason');
            $this->accountLifecycleService->anonymize($actor, $account, \is_string($reason) ? $reason : null);

            $this->addFlash('success', 'Account deleted. This cannot be undone.');

            return $this->redirectToRoute('administration_users_index');
        }

        return $this->render('administration/user_delete_confirm.html.twig', ['account' => $account]);
    }
}
