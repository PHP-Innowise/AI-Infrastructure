<?php

declare(strict_types=1);

namespace App\Crm\Controller;

use App\Crm\Entity\Label;
use App\Crm\Exception\DuplicateLabelNameException;
use App\Crm\Form\LabelType;
use App\Crm\Repository\LabelRepository;
use App\Crm\Service\LabelService;
use App\Crm\Voter\LabelVoter;
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
 * US-03.03: create, edit, and delete labels ("Manage Labels").
 *
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md BR-03-3/4/5, AC-03-11..14
 */
#[IsGranted('ROLE_TRAINER')]
final class TrainerLabelController extends AbstractController
{
    public function __construct(
        private readonly LabelRepository $labels,
        private readonly LabelService $labelService,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    /**
     * AC-03-14: each label shows its usage count.
     */
    #[Route('/trainer/labels', name: 'crm_trainer_labels_index', methods: ['GET'])]
    public function index(): Response
    {
        $labels = $this->labels->findAllForActiveTenant();

        return $this->render('crm/trainer_labels_index.html.twig', [
            'labels' => $labels,
            'usageCounts' => array_combine(
                array_map(static fn (Label $l): int => (int) $l->getId(), $labels),
                array_map(fn (Label $l): int => $this->labels->countPlayers($l), $labels),
            ),
        ]);
    }

    #[Route('/trainer/labels/new', name: 'crm_trainer_label_create', methods: ['GET', 'POST'])]
    public function create(Request $request): Response
    {
        $this->denyAccessUnlessGranted(LabelVoter::LABEL_MANAGE, null);

        $form = $this->createForm(LabelType::class);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{name: string, colorHex: string} $data */
            $data = $form->getData();
            $trainerId = $this->tenantContext->requireTrainerId();
            /** @var Trainer $trainer */
            $trainer = $this->entityManager->getReference(Trainer::class, $trainerId);

            try {
                $this->labelService->create($trainer, $data['name'], $data['colorHex']);
                $this->addFlash('success', 'Label created.');

                return $this->redirectToRoute('crm_trainer_labels_index');
            } catch (DuplicateLabelNameException $e) {
                $form->get('name')->addError(new FormError($e->getMessage()));
            }
        }

        return $this->render('crm/trainer_label_form.html.twig', ['form' => $form, 'mode' => 'create']);
    }

    #[Route('/trainer/labels/{label}/edit', name: 'crm_trainer_label_edit', methods: ['GET', 'POST'])]
    public function edit(Request $request, Label $label): Response
    {
        $this->denyAccessUnlessGranted(LabelVoter::LABEL_MANAGE, $label);

        $form = $this->createForm(LabelType::class, ['name' => $label->getName(), 'colorHex' => $label->getColorHex()]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{name: string, colorHex: string} $data */
            $data = $form->getData();

            try {
                $this->labelService->rename($label, $data['name'], $data['colorHex']);
                $this->addFlash('success', 'Label updated.');

                return $this->redirectToRoute('crm_trainer_labels_index');
            } catch (DuplicateLabelNameException $e) {
                $form->get('name')->addError(new FormError($e->getMessage()));
            }
        }

        return $this->render('crm/trainer_label_form.html.twig', ['form' => $form, 'mode' => 'edit', 'label' => $label]);
    }

    /**
     * AC-03-13/BR-03-5: "Remove [Label] from [N] players?" confirmation.
     */
    #[Route('/trainer/labels/{label}/delete', name: 'crm_trainer_label_delete', methods: ['GET', 'POST'])]
    public function delete(Request $request, Label $label): Response
    {
        $this->denyAccessUnlessGranted(LabelVoter::LABEL_MANAGE, $label);

        if ($request->isMethod('POST')) {
            if (!$this->isCsrfTokenValid('label-delete'.$label->getId(), $request->request->getString('_token'))) {
                throw $this->createAccessDeniedException('Invalid CSRF token.');
            }

            $affectedPlayers = $this->labels->countPlayers($label);
            $this->labelService->delete($label);
            $this->addFlash('success', sprintf('Label removed from %d player(s).', $affectedPlayers));

            return $this->redirectToRoute('crm_trainer_labels_index');
        }

        return $this->render('crm/trainer_label_delete.html.twig', [
            'label' => $label,
            'affectedPlayers' => $this->labels->countPlayers($label),
        ]);
    }
}
