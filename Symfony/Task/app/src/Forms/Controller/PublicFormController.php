<?php

declare(strict_types=1);

namespace App\Forms\Controller;

use App\Forms\Dto\FormField;
use App\Forms\Entity\Form;
use App\Forms\Entity\FormSubmission;
use App\Forms\Exception\CampFullException;
use App\Forms\Exception\DuplicateSubmissionException;
use App\Forms\Form\ConvertSubmissionToAccountType;
use App\Forms\Repository\FormRepository;
use App\Forms\Repository\FormSubmissionRepository;
use App\Forms\Service\FormService;
use App\Forms\Service\FormSubmissionConversionService;
use App\Forms\Service\FormSubmissionService;
use App\Forms\Service\PublicSubmissionFormBuilder;
use App\Forms\Service\SubmissionTokenFactory;
use App\Forms\Voter\FormSubmissionVoter;
use App\Identity\Exception\DuplicateEmailException;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Bundle\SecurityBundle\Security;
use Symfony\Component\DependencyInjection\Attribute\Autowire;
use Symfony\Component\Form\FormError;
use Symfony\Component\Form\FormInterface;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\HttpKernel\Exception\TooManyRequestsHttpException;
use Symfony\Component\RateLimiter\RateLimiterFactory;
use Symfony\Component\Routing\Attribute\Route;

/**
 * US-08.03: the public, unauthenticated camp/evaluation flow —
 * `TenantFromPublicCode`'s allow-list resolves the tenant from `{code}`
 * before any of these actions run (source 5). Every route here is
 * `PUBLIC_ACCESS` per `config/packages/security.yaml`'s `^/forms/` rule.
 *
 * @see specs/api-designer-spec.md "Forms module" — public route table
 * @see specs/council-sharelink-tenant-resolution.md
 */
final class PublicFormController extends AbstractController
{
    public function __construct(
        private readonly FormRepository $forms,
        private readonly FormSubmissionRepository $submissions,
        private readonly FormService $formService,
        private readonly FormSubmissionService $submissionService,
        private readonly FormSubmissionConversionService $conversionService,
        private readonly PublicSubmissionFormBuilder $submissionFormBuilder,
        private readonly SubmissionTokenFactory $tokens,
        private readonly Security $security,
        #[Autowire(service: 'limiter.forms_public_show')]
        private readonly RateLimiterFactory $showLimiter,
        #[Autowire(service: 'limiter.forms_public_submit')]
        private readonly RateLimiterFactory $submitLimiter,
        #[Autowire(service: 'limiter.forms_public_convert_account')]
        private readonly RateLimiterFactory $convertLimiter,
    ) {
    }

    /**
     * AC-08-13/15, BR-08-3/5/7: full form if open; "Camp Full" at capacity;
     * "Registration Closed" for a disabled camp.
     */
    #[Route('/forms/{code}', name: 'forms_public_show', methods: ['GET'])]
    public function show(string $code, Request $request): Response
    {
        $this->consume($this->showLimiter, $request);

        $form = $this->findPublishedForm($code);

        if ($this->isFull($form)) {
            return $this->render('forms/public_full.html.twig', ['form' => $form]);
        }

        if (!$this->isReachable($form)) {
            return $this->render('forms/public_closed.html.twig', ['form' => $form]);
        }

        $this->denyAccessUnlessGranted(FormSubmissionVoter::FORM_SUBMISSION_CREATE, $form);

        return $this->renderPublicForm($form, $this->submissionFormBuilder->build($form));
    }

    /**
     * AC-08-14..16, BR-08-6/9..11.
     */
    #[Route('/forms/{code}', name: 'forms_public_submit', methods: ['POST'])]
    public function submit(string $code, Request $request): Response
    {
        $this->consume($this->submitLimiter, $request);

        $form = $this->findPublishedForm($code);

        // Mirrors show()'s own explicit pre-checks rather than relying on
        // denyAccessUnlessGranted() alone: for an ANONYMOUS actor (every
        // actor on this route), Symfony's security layer turns a bare
        // AccessDeniedException into a 302-to-login redirect — sensible
        // for a gated screen, nonsensical for a public camp registration
        // page. A capacity race lost between the GET and this POST (or a
        // direct POST to a full/closed camp) must render the same
        // friendly page a GET would, never send a parent to a login form.
        if ($this->isFull($form)) {
            return $this->render('forms/public_full.html.twig', ['form' => $form]);
        }

        if (!$this->isReachable($form)) {
            return $this->render('forms/public_closed.html.twig', ['form' => $form]);
        }

        $this->denyAccessUnlessGranted(FormSubmissionVoter::FORM_SUBMISSION_CREATE, $form);

        $submissionForm = $this->submissionFormBuilder->build($form);
        $submissionForm->handleRequest($request);

        if (!$submissionForm->isSubmitted() || !$submissionForm->isValid()) {
            return $this->renderPublicForm($form, $submissionForm, 422);
        }

        // Honeypot: silently pretend success — never a validation error, so
        // the bot learns nothing (PublicSubmissionFormBuilder's own docblock).
        $honeypot = $submissionForm->get(PublicSubmissionFormBuilder::HONEYPOT_FIELD)->getData();

        if (\is_string($honeypot) && '' !== trim($honeypot)) {
            return $this->redirectToRoute('forms_public_show', ['code' => $code], Response::HTTP_SEE_OTHER);
        }

        /** @var array<string, mixed> $answers */
        $answers = $submissionForm->getData();

        try {
            $outcome = $this->submissionService->submit($form, $answers, null);
        } catch (DuplicateSubmissionException $e) {
            $submissionForm->get(FormField::FIELD_PARTICIPANT_EMAIL)->addError(new FormError($e->getMessage()));

            return $this->renderPublicForm($form, $submissionForm, 422);
        } catch (CampFullException) {
            // A capacity race lost between the GET and this POST.
            return $this->render('forms/public_full.html.twig', ['form' => $form]);
        }

        if ($outcome->requiresPayment) {
            \assert(null !== $outcome->redirectUrl);

            return $this->redirect($outcome->redirectUrl, Response::HTTP_SEE_OTHER);
        }

        return $this->redirectToRoute('forms_public_confirmation', [
            'code' => $code,
            'submission' => $this->tokens->tokenFor($outcome->submission),
        ], Response::HTTP_SEE_OTHER);
    }

    /**
     * AC-08-16.
     */
    #[Route('/forms/{code}/confirmation', name: 'forms_public_confirmation', methods: ['GET'])]
    public function confirmation(string $code, Request $request): Response
    {
        $form = $this->findPublishedForm($code);
        $submission = $this->resolveSubmission($request, $form);
        $this->denyAccessUnlessGranted(FormSubmissionVoter::FORM_SUBMISSION_CREATE, $form);

        return $this->render('forms/public_confirmation.html.twig', [
            'form' => $form,
            'submission' => $submission,
            'accountAlreadyExists' => $this->conversionService->accountAlreadyExists($submission),
        ]);
    }

    /**
     * Waypoint after a successful Stripe Checkout — the real confirmation is
     * the async webhook (BR-08-14); this page never claims the registration
     * is confirmed on its own, only that payment was received.
     */
    #[Route('/forms/{code}/checkout/success', name: 'forms_public_checkout_success', methods: ['GET'])]
    public function checkoutSuccess(string $code, Request $request): Response
    {
        $form = $this->findPublishedForm($code);
        $submission = $this->resolveSubmission($request, $form);

        if ($submission->isConfirmed()) {
            return $this->redirectToRoute('forms_public_confirmation', ['code' => $code, 'submission' => $request->query->getString('submission')]);
        }

        return $this->render('forms/public_checkout_success.html.twig', ['form' => $form, 'submission' => $submission]);
    }

    #[Route('/forms/{code}/checkout/cancel', name: 'forms_public_checkout_cancel', methods: ['GET'])]
    public function checkoutCancel(string $code, Request $request): Response
    {
        $form = $this->findPublishedForm($code);
        $submission = $this->resolveSubmission($request, $form);

        return $this->render('forms/public_checkout_cancel.html.twig', ['form' => $form, 'submission' => $submission]);
    }

    /**
     * AC-08-23..26, BR-08-15..18.
     */
    #[Route('/forms/{code}/convert-account', name: 'forms_public_convert_account', methods: ['GET', 'POST'])]
    public function convertAccount(string $code, Request $request): Response
    {
        $this->consume($this->convertLimiter, $request);

        $form = $this->findPublishedForm($code);
        $submission = $this->resolveSubmission($request, $form);

        // AC-08-26: never even renders the form for an email that already
        // has an account. Checked ahead of the voter (rather than relying
        // on FormSubmissionVoter::FORM_SUBMISSION_CONVERT's own
        // isConverted() deny) for the same reason submit() checks
        // full/closed explicitly: an anonymous actor hitting a bare
        // AccessDeniedException is redirected to /login by Symfony's
        // security layer regardless of *why* it was denied, which would
        // otherwise show no explanation at all.
        if ($submission->isConverted() || $this->conversionService->accountAlreadyExists($submission)) {
            $this->addFlash('info', 'An account already exists for this email address. Log in to continue.');

            return $this->redirectToRoute('identity_auth_login');
        }

        $this->denyAccessUnlessGranted(FormSubmissionVoter::FORM_SUBMISSION_CONVERT, $submission);

        $accountForm = $this->createForm(ConvertSubmissionToAccountType::class);
        $accountForm->handleRequest($request);

        if ($accountForm->isSubmitted() && $accountForm->isValid()) {
            /** @var array{plainPassword: string, dateOfBirth: \DateTimeInterface, gender: ?string} $data */
            $data = $accountForm->getData();

            try {
                $account = $this->conversionService->convert(
                    $submission,
                    \DateTimeImmutable::createFromInterface($data['dateOfBirth']),
                    $data['gender'],
                    $data['plainPassword'],
                );
            } catch (DuplicateEmailException) {
                $this->addFlash('info', 'An account already exists for this email address. Log in to continue.');

                return $this->redirectToRoute('identity_auth_login');
            }

            $this->security->login($account, 'form_login', 'main');

            return $this->redirectToRoute('app_dashboard');
        }

        return $this->render('forms/public_convert_account.html.twig', [
            'form' => $form,
            'submission' => $submission,
            'accountForm' => $accountForm,
        ]);
    }

    /**
     * @param FormInterface<array<string, mixed>> $submissionForm
     */
    private function renderPublicForm(Form $form, FormInterface $submissionForm, int $status = 200): Response
    {
        return $this->render('forms/public_show.html.twig', [
            'form' => $form,
            'submissionForm' => $submissionForm,
            'remainingSpots' => $this->formService->remainingSpotsFor($form),
            'honeypotField' => PublicSubmissionFormBuilder::HONEYPOT_FIELD,
        ], new Response(status: $status));
    }

    /**
     * A code that resolves to an unpublished `Form` must 404, identically
     * to a code that resolves to nothing — "an unknown code resolves
     * nothing and 404s before any trainer-scoped query." This is not
     * automatic from RLS alone: `TenantResolver`'s source 5 is the ONLY
     * source on an unpublished code (no `PublicTenantCode` row exists yet
     * to resolve from), but if the visitor also happens to be
     * authenticated as the form's OWNING trainer (e.g. previewing, then
     * separately opening the public link in the same browser session),
     * source 4 ("the trainer's own tenant") still resolves — the
     * precedence rule only says source 5 OUTRANKS 3/4 when it resolves,
     * not that an allow-listed route refuses to fall through to them when
     * it does not. Left unchecked, the form's OWNER could reach their own
     * still-unpublished camp as a live, submittable public page. Every
     * public action re-checks this explicitly rather than trusting
     * resolution alone, matching "resolution is not authorization."
     */
    private function findPublishedForm(string $code): Form
    {
        $form = $this->forms->findOneBySlug($code) ?? throw $this->createNotFoundException();

        if (!$this->formService->isPublished($form)) {
            throw $this->createNotFoundException();
        }

        return $form;
    }

    private function isFull(Form $form): bool
    {
        return $form->isFull($this->formService->submissionCountFor($form));
    }

    private function isReachable(Form $form): bool
    {
        if (!$form->isOpenForRegistration()) {
            return false;
        }

        return $this->isGranted(FormSubmissionVoter::FORM_SUBMISSION_CREATE, $form);
    }

    private function resolveSubmission(Request $request, Form $form): FormSubmission
    {
        $token = $request->query->getString('submission');
        $id = '' !== $token ? $this->tokens->decode($token) : null;
        $submission = null !== $id ? $this->submissions->find($id) : null;

        if (null === $submission || $submission->getForm()->getId() !== $form->getId()) {
            throw $this->createNotFoundException();
        }

        return $submission;
    }

    private function consume(RateLimiterFactory $limiterFactory, Request $request): void
    {
        $limit = $limiterFactory->create($request->getClientIp() ?? 'unknown')->consume();

        if (!$limit->isAccepted()) {
            throw new TooManyRequestsHttpException($limit->getRetryAfter()->getTimestamp() - time());
        }
    }
}
