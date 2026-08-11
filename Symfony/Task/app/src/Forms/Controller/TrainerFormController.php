<?php

declare(strict_types=1);

namespace App\Forms\Controller;

use App\Forms\Dto\FormField;
use App\Forms\Entity\Form;
use App\Forms\Exception\FormHasSubmissionsException;
use App\Forms\Form\CampFormType;
use App\Forms\Form\EvaluationFormType;
use App\Forms\Repository\FormRepository;
use App\Forms\Service\FormService;
use App\Forms\Service\PublicSubmissionFormBuilder;
use App\Forms\Voter\FormVoter;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\Form\FormError;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-08.01/08.02/08.06: trainer console for camps and evaluations.
 *
 * @see specs/api-designer-spec.md "Forms module" — trainer console route table
 */
#[IsGranted('ROLE_TRAINER')]
final class TrainerFormController extends AbstractController
{
    public function __construct(
        private readonly FormRepository $forms,
        private readonly FormService $formService,
        private readonly PublicSubmissionFormBuilder $submissionFormBuilder,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    /**
     * AC-08-27/32: every camp/evaluation, with submission counts, remaining
     * spots and shareable links.
     */
    #[Route('/trainer/forms', name: 'forms_trainer_index', methods: ['GET'])]
    public function index(): Response
    {
        return $this->render('forms/trainer_index.html.twig', [
            'forms' => $this->forms->findAllForTrainer($this->currentTrainer()),
            'formService' => $this->formService,
        ]);
    }

    /**
     * AC-08-1..5, BR-08-1..4.
     */
    #[Route('/trainer/forms/camps/new', name: 'forms_trainer_camp_create', methods: ['GET', 'POST'])]
    public function createCamp(Request $request): Response
    {
        $this->denyAccessUnlessGranted(FormVoter::FORM_CREATE, Form::TYPE_CAMP);

        $formView = $this->createForm(CampFormType::class, [
            'fields' => self::initialFieldRows(FormService::defaultTemplateFields()),
        ], ['submitLabel' => 'Create Camp']);
        $formView->handleRequest($request);

        if ($formView->isSubmitted() && $formView->isValid()) {
            /** @var array{name: string, description: ?string, capacityLimit: int, priceMinorUnits: ?int, fields: list<array<string, mixed>>} $data */
            $data = $formView->getData();

            try {
                $created = $this->formService->createCamp(
                    $this->currentTrainer(),
                    $data['name'],
                    $data['description'],
                    $data['priceMinorUnits'],
                    $data['capacityLimit'],
                    self::fieldsFromRows($data['fields']),
                );
            } catch (\InvalidArgumentException $e) {
                $formView->addError(new FormError($e->getMessage()));

                return $this->render('forms/trainer_form_edit.html.twig', ['formView' => $formView, 'mode' => 'create', 'formType' => Form::TYPE_CAMP], new Response(status: 422));
            }

            $this->addFlash('success', 'Camp created. Customize it below, then publish when ready.');

            return $this->redirectToRoute('forms_trainer_form_edit', ['form' => $created->getId()]);
        }

        return $this->render(
            'forms/trainer_form_edit.html.twig',
            ['formView' => $formView, 'mode' => 'create', 'formType' => Form::TYPE_CAMP],
            new Response(status: $formView->isSubmitted() ? 422 : 200),
        );
    }

    /**
     * AC-08-9/10.
     */
    #[Route('/trainer/forms/evaluations/new', name: 'forms_trainer_evaluation_create', methods: ['GET', 'POST'])]
    public function createEvaluation(Request $request): Response
    {
        $this->denyAccessUnlessGranted(FormVoter::FORM_CREATE, Form::TYPE_EVALUATION);

        $formView = $this->createForm(EvaluationFormType::class, [
            'fields' => self::initialFieldRows(FormService::defaultTemplateFields()),
        ], ['submitLabel' => 'Create Evaluation']);
        $formView->handleRequest($request);

        if ($formView->isSubmitted() && $formView->isValid()) {
            /** @var array{name: string, description: ?string, priceMinorUnits: ?int, fields: list<array<string, mixed>>} $data */
            $data = $formView->getData();

            try {
                $created = $this->formService->createEvaluation(
                    $this->currentTrainer(),
                    $data['name'],
                    $data['description'],
                    $data['priceMinorUnits'],
                    self::fieldsFromRows($data['fields']),
                );
            } catch (\InvalidArgumentException $e) {
                $formView->addError(new FormError($e->getMessage()));

                return $this->render('forms/trainer_form_edit.html.twig', ['formView' => $formView, 'mode' => 'create', 'formType' => Form::TYPE_EVALUATION], new Response(status: 422));
            }

            $this->addFlash('success', 'Evaluation created. Customize it below, then publish when ready.');

            return $this->redirectToRoute('forms_trainer_form_edit', ['form' => $created->getId()]);
        }

        return $this->render(
            'forms/trainer_form_edit.html.twig',
            ['formView' => $formView, 'mode' => 'create', 'formType' => Form::TYPE_EVALUATION],
            new Response(status: $formView->isSubmitted() ? 422 : 200),
        );
    }

    /**
     * AC-08-6/11: reachable pre-publish — renders the same template
     * `forms_public_show` uses, read-only.
     */
    #[Route('/trainer/forms/{form}/preview', name: 'forms_trainer_form_preview', methods: ['GET'])]
    public function preview(Form $form): Response
    {
        $this->denyAccessUnlessGranted(FormVoter::FORM_EDIT, $form);

        return $this->render('forms/public_show.html.twig', [
            'form' => $form,
            'submissionForm' => $this->submissionFormBuilder->build($form),
            'remainingSpots' => $this->formService->remainingSpotsFor($form),
            'honeypotField' => PublicSubmissionFormBuilder::HONEYPOT_FIELD,
            'preview' => true,
        ]);
    }

    /**
     * AC-08-28/30.
     */
    #[Route('/trainer/forms/{form}/edit', name: 'forms_trainer_form_edit', methods: ['GET', 'POST'])]
    public function edit(Request $request, Form $form): Response
    {
        $this->denyAccessUnlessGranted(FormVoter::FORM_EDIT, $form);

        $type = $form->isCamp() ? CampFormType::class : EvaluationFormType::class;
        $initial = [
            'name' => $form->getName(),
            'description' => $form->getDescription(),
            'priceMinorUnits' => $form->getPriceMinorUnits(),
            'fields' => self::initialFieldRows($form->getFields()),
        ];

        if ($form->isCamp()) {
            $initial['capacityLimit'] = $form->getCapacityLimit();
        }

        $formView = $this->createForm($type, $initial, ['submitLabel' => 'Save Changes']);
        $formView->handleRequest($request);

        $submissionCount = $this->formService->submissionCountFor($form);

        if ($formView->isSubmitted() && $formView->isValid()) {
            /** @var array{name: string, description: ?string, priceMinorUnits: ?int, capacityLimit?: int, fields: list<array<string, mixed>>} $data */
            $data = $formView->getData();

            try {
                $this->formService->update(
                    $form,
                    $data['name'],
                    $data['description'],
                    $data['priceMinorUnits'],
                    $data['capacityLimit'] ?? null,
                    self::fieldsFromRows($data['fields']),
                );
            } catch (\InvalidArgumentException $e) {
                $formView->addError(new FormError($e->getMessage()));

                return $this->render('forms/trainer_form_edit.html.twig', [
                    'formView' => $formView, 'mode' => 'edit', 'formType' => $form->getFormType(), 'form' => $form, 'submissionCount' => $submissionCount,
                    'isPublished' => $this->formService->isPublished($form),
                ], new Response(status: 422));
            }

            $this->addFlash('success', 'Saved.');

            return $this->redirectToRoute('forms_trainer_form_edit', ['form' => $form->getId()]);
        }

        return $this->render(
            'forms/trainer_form_edit.html.twig',
            [
                'formView' => $formView,
                'mode' => 'edit',
                'formType' => $form->getFormType(),
                'form' => $form,
                'submissionCount' => $submissionCount,
                'isPublished' => $this->formService->isPublished($form),
            ],
            new Response(status: $formView->isSubmitted() ? 422 : 200),
        );
    }

    /**
     * AC-08-7: generates the shareable link on first publish.
     */
    #[Route('/trainer/forms/{form}/publish', name: 'forms_trainer_form_publish', methods: ['POST'])]
    public function publish(Request $request, Form $form): Response
    {
        $this->denyAccessUnlessGranted(FormVoter::FORM_EDIT, $form);
        $this->assertCsrf($request, 'form-publish', $form);

        $this->formService->publish($form);
        $this->addFlash('success', 'Published! Your shareable link is ready.');

        return $this->redirectToRoute('forms_trainer_form_edit', ['form' => $form->getId()]);
    }

    /**
     * AC-08-8/29: camps only.
     */
    #[Route('/trainer/forms/{form}/toggle', name: 'forms_trainer_form_toggle', methods: ['POST'])]
    public function toggle(Request $request, Form $form): Response
    {
        $this->denyAccessUnlessGranted(FormVoter::FORM_TOGGLE, $form);
        $this->assertCsrf($request, 'form-toggle', $form);

        $form->isActiveFlag() ? $this->formService->disable($form) : $this->formService->enable($form);
        $this->addFlash('success', $form->isActiveFlag() ? 'Registration enabled.' : 'Registration disabled.');

        return $this->redirectToRoute('forms_trainer_form_edit', ['form' => $form->getId()]);
    }

    /**
     * AC-08-31.
     */
    #[Route('/trainer/forms/{form}/delete', name: 'forms_trainer_form_delete', methods: ['POST'])]
    public function delete(Request $request, Form $form): Response
    {
        $this->denyAccessUnlessGranted(FormVoter::FORM_DELETE, $form);
        $this->assertCsrf($request, 'form-delete', $form);

        try {
            $name = $form->getName();
            $this->formService->delete($form);
            $this->addFlash('success', sprintf('"%s" deleted.', $name));
        } catch (FormHasSubmissionsException $e) {
            $this->addFlash('error', $e->getMessage());

            return $this->redirectToRoute('forms_trainer_form_edit', ['form' => $form->getId()]);
        }

        return $this->redirectToRoute('forms_trainer_index');
    }

    /**
     * Pads the field list with a few blank slots so a trainer can add NEW
     * custom fields through plain HTML — `CollectionType`'s own
     * JS-driven "add another" prototype needs a Stimulus controller this
     * codebase does not yet ship (coder-frontend territory; see
     * `templates/content/trainer_learn_form.html.twig`'s identical
     * comment for the video collection this mirrors). Every row becomes a
     * plain array here (not a `FormField` instance) so a genuine template
     * field and a blank padding slot share one shape for Symfony Form's
     * property access.
     *
     * @param list<FormField> $fields
     *
     * @return list<array{id: string, type: string, label: string, required: bool, options: ?list<string>}>
     */
    private static function initialFieldRows(array $fields, int $blankSlots = 3): array
    {
        $rows = array_map(static fn (FormField $field): array => $field->toArray(), $fields);

        for ($i = 0; $i < $blankSlots; ++$i) {
            $rows[] = ['id' => '', 'type' => FormField::TYPE_TEXT, 'label' => '', 'required' => false, 'options' => null];
        }

        return $rows;
    }

    /**
     * A row left blank (no id, no label) is an unused padding slot — see
     * `initialFieldRows()` — not a field to construct.
     *
     * @param list<array<string, mixed>> $rows
     *
     * @return list<FormField>
     */
    private static function fieldsFromRows(array $rows): array
    {
        $fields = [];

        foreach ($rows as $row) {
            $id = \is_string($row['id'] ?? null) ? trim($row['id']) : '';
            $label = \is_string($row['label'] ?? null) ? trim($row['label']) : '';

            if ('' === $id && '' === $label) {
                continue;
            }

            $fields[] = FormField::fromArray($row);
        }

        return $fields;
    }

    private function assertCsrf(Request $request, string $tokenId, Form $form): void
    {
        if (!$this->isCsrfTokenValid($tokenId.$form->getId(), $request->request->getString('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }
    }

    private function currentTrainer(): Trainer
    {
        /** @var Trainer $trainer */
        $trainer = $this->entityManager->getReference(Trainer::class, $this->tenantContext->requireTrainerId());

        return $trainer;
    }
}
