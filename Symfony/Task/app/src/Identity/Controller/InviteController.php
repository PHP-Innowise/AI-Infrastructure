<?php

declare(strict_types=1);

namespace App\Identity\Controller;

use App\Identity\Entity\Account;
use App\Identity\Entity\ShareLink;
use App\Identity\Exception\CoachAlreadyActiveElsewhereException;
use App\Identity\Exception\DuplicateEmailException;
use App\Identity\Form\CoachRegistrationType;
use App\Identity\Form\PlayerRegistrationType;
use App\Identity\Repository\ParentChildLinkRepository;
use App\Identity\Repository\ShareLinkRepository;
use App\Identity\Service\CoachRegistrationService;
use App\Identity\Service\IdentityMailer;
use App\Identity\Service\ShareLinkService;
use App\Identity\Voter\ShareLinkVoter;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Bundle\SecurityBundle\Security;
use Symfony\Component\Form\FormError;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\CurrentUser;

/**
 * US-01.08: the coach invitation landing flow, AND (Epic-03) US-03.11: the
 * coach-issued player invitation landing flow — both reached through
 * `/invite/{code}`, branching on `ShareLink.type`
 * (`specs/api-designer-spec.md` "Identity module" Decisions, "`/invite/{code}`
 * reuse"). `/invite/{code}` is allow-listed in TenantFromPublicCode, so the
 * tenant is already resolved by the time these actions run.
 *
 * **The player-invite branch renders `sharelink_show.html.twig` unchanged**
 * — that template's registration form already posts to
 * `identity_sharelink_register` (`/join/{code}/register`), which
 * `ShareLinkController::register()` already handles for ANY usable
 * ShareLink regardless of type (it never inspects `linkType`). This is
 * deliberate reuse, not an oversight: it means a coach-issued player invite
 * completes registration through the exact same, already-tested Epic-01
 * code path as the trainer's own static link, with zero duplication and
 * zero risk of the two diverging. `identity_invite_register` (this
 * controller's own POST route) therefore stays coach-registration-only —
 * see `register()`'s own guard against a mismatched link type reaching it
 * directly.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md US-01.08, AC-01-39..42
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md US-03.11, AC-03-50..53
 */
final class InviteController extends AbstractController
{
    public function __construct(
        private readonly ShareLinkRepository $shareLinks,
        private readonly ShareLinkService $shareLinkService,
        private readonly CoachRegistrationService $coachRegistration,
        private readonly ParentChildLinkRepository $parentChildLinks,
        private readonly IdentityMailer $mailer,
        private readonly Security $security,
    ) {
    }

    #[Route('/invite/{code}', name: 'identity_invite_show', methods: ['GET'])]
    public function show(string $code, #[CurrentUser] ?Account $account): Response
    {
        $shareLink = $this->shareLinks->findOneByCode($code);

        if (null === $shareLink) {
            throw $this->createNotFoundException();
        }

        $this->shareLinkService->recordOpen($shareLink);

        if (!$shareLink->isUsable(new \DateTimeImmutable())) {
            // AC-01-42: a clear message. The resend action itself lives on
            // the trainer's Coaches list, not on this anonymous page.
            return $this->render('identity/sharelink_unusable.html.twig', ['shareLink' => $shareLink], new Response(status: 410));
        }

        if ($shareLink->isCoachPlayerInviteLink()) {
            return $this->showPlayerInvite($shareLink, $account);
        }

        $form = $this->createForm(CoachRegistrationType::class);

        return $this->render('identity/invite_show.html.twig', ['shareLink' => $shareLink, 'form' => $form]);
    }

    #[Route('/invite/{code}/register', name: 'identity_invite_register', methods: ['POST'])]
    public function register(Request $request, string $code): Response
    {
        $shareLink = $this->shareLinks->findOneByCode($code) ?? throw $this->createNotFoundException();
        $this->denyAccessUnlessGranted(ShareLinkVoter::SHARELINK_RESOLVE, $shareLink);

        // Coach registration only — a coach-issued player invite's own
        // registration form posts to identity_sharelink_register instead
        // (see this class's own docblock). A direct POST here with the
        // wrong link type is refused rather than silently creating a coach
        // account off a player invite.
        if ($shareLink->isCoachPlayerInviteLink()) {
            throw $this->createNotFoundException();
        }

        $form = $this->createForm(CoachRegistrationType::class);
        $form->handleRequest($request);

        if (!$form->isSubmitted() || !$form->isValid()) {
            return $this->render('identity/invite_show.html.twig', ['shareLink' => $shareLink, 'form' => $form]);
        }

        /** @var array{firstName: string, lastName: string, email: string, plainPassword: string} $data */
        $data = $form->getData();

        try {
            $account = $this->coachRegistration->registerViaInvite($shareLink, $data['firstName'], $data['lastName'], $data['email'], $data['plainPassword']);
        } catch (DuplicateEmailException $e) {
            $form->get('email')->addError(new FormError($e->getMessage()));

            return $this->render('identity/invite_show.html.twig', ['shareLink' => $shareLink, 'form' => $form]);
        } catch (CoachAlreadyActiveElsewhereException $e) {
            // AC-01-41: an error, not a second active trainer.
            $form->addError(new FormError($e->getMessage()));

            return $this->render('identity/invite_show.html.twig', ['shareLink' => $shareLink, 'form' => $form]);
        }

        $this->security->login($account, 'form_login', 'main');
        $this->addFlash('success', sprintf('Welcome to %s!', $shareLink->getTrainer()->getBusinessName()));

        return $this->redirectToRoute('app_dashboard');
    }

    /**
     * AC-03-50/51: mirrors ShareLinkController::show()'s three branches
     * (anonymous / authenticated-non-child / authenticated-child) for a
     * coach-issued player invite. Deliberately not extracted into a shared
     * base with ShareLinkController — see this class's own docblock.
     */
    private function showPlayerInvite(ShareLink $shareLink, ?Account $account): Response
    {
        if (null === $account) {
            $form = $this->createForm(PlayerRegistrationType::class);

            return $this->render('identity/sharelink_show.html.twig', ['shareLink' => $shareLink, 'form' => $form]);
        }

        $parentLink = $this->parentChildLinks->findByChildAccount($account);

        if (null !== $parentLink) {
            // AC-01-31's same rule, reused: a logged-in child is blocked; the
            // parent is emailed instead, no association happens yet.
            $this->mailer->sendParentReviewRegistration($parentLink->getParentAccount(), $account, $shareLink->getTrainer(), $shareLink->getCode());

            return $this->render('identity/sharelink_child_blocked.html.twig', ['shareLink' => $shareLink]);
        }

        // "same as identity_sharelink_join_show's authenticated branch" per
        // the settled route design (specs/api-designer-spec.md:409).
        return $this->redirectToRoute('identity_sharelink_associate', ['code' => $shareLink->getCode()]);
    }
}
