<?php

declare(strict_types=1);

namespace App\Forms\Form;

use App\Forms\Entity\FormSubmission;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\CheckboxType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * AC-08-20: one checkbox per participant, built dynamically from the
 * form's CURRENT confirmed submission list — matching the epic's own
 * "collection form" description (api-designer-spec.md), and the same
 * "assembled at runtime via `FormFactoryInterface`, not a static class"
 * shape the public submission form uses, since the row count is never
 * known statically.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class FormAttendanceType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        /** @var list<FormSubmission> $submissions */
        $submissions = $options['submissions'];

        foreach ($submissions as $submission) {
            $builder->add(self::fieldName($submission), CheckboxType::class, [
                'label' => $submission->participantName(),
                'required' => false,
                'data' => $submission->isAttended(),
            ]);
        }

        $builder->add('submit', SubmitType::class, ['label' => 'Save Attendance']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
        $resolver->setRequired('submissions');
        $resolver->setAllowedTypes('submissions', 'array');
    }

    public static function fieldName(FormSubmission $submission): string
    {
        $id = $submission->getId();
        \assert(null !== $id);

        return 'attendance_'.$id;
    }
}
