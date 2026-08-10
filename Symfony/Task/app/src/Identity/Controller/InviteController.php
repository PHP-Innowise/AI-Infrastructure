<?php

declare(strict_types=1);

namespace App\Identity\Controller;

use App\Identity\Exception\CoachAlreadyActiveElsewhereException;
use App\Identity\Exception\DuplicateEmailException;
use App\Identity\Form\CoachRegistrationType;
use App\Identity\Repository\ShareLinkRepository;
use App\Identity\Service\CoachRegistrationService;
use App\Identity\Service\ShareLinkService;
use App\Identity\Voter\ShareLinkVoter;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Bundle\SecurityBundle\Security;
use Symfony\Component\Form\FormError;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;

/**
 * US-01.08: the coach invitation landing flow. `/invite/{code}` is
 * allow-listed in TenantFromPublicCode, so the tenant is already resolved by
 * the time these actions run.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md US-01.08, AC-01-39..42
 */
final class InviteController extends AbstractController
{
    public function __construct(
        private readonly ShareLinkRepository $shareLinks,
        private readonly ShareLinkService $shareLinkService,
        private readonly CoachRegistrationService $coachRegistration,
        private readonly Security $security,
    ) {
    }

    #[Route('/invite/{code}', name: 'identity_invite_show', methods: ['GET'])]
    public function show(string $code): Response
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

        $form = $this->createForm(CoachRegistrationType::class);

        return $this->render('identity/invite_show.html.twig', ['shareLink' => $shareLink, 'form' => $form]);
    }

    #[Route('/invite/{code}/register', name: 'identity_invite_register', methods: ['POST'])]
    public function register(Request $request, string $code): Response
    {
        $shareLink = $this->shareLinks->findOneByCode($code) ?? throw $this->createNotFoundException();
        $this->denyAccessUnlessGranted(ShareLinkVoter::SHARELINK_RESOLVE, $shareLink);

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
}
