<?php

declare(strict_types=1);

namespace App\Scheduling\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\DateType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\NotBlank;

/**
 * AC-02-61: "Every Tuesday for 3 months" — every field EventType already
 * collects describes the FIRST occurrence, the template
 * EventService::createRecurring() clones weekly; this form adds only the
 * one field a single-event form has no use for — how far the weekly
 * pattern repeats. Extends EventType via getParent() rather than
 * duplicating its ~15 fields and its POST_SUBMIT past/pricing validation
 * (both still apply, checked against the first occurrence) — see
 * EventType's own docblock for why the field set is what it is.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class RecurringEventType extends AbstractType
{
    public function getParent(): string
    {
        return EventType::class;
    }

    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        /** @var \DateTimeZone $timezone */
        $timezone = $options['timezone'];

        // Removed and re-added after repeatUntil so the submit button
        // renders last — EventType's own buildForm() (the parent in this
        // type's chain, which runs before this method) already added it.
        $builder->remove('submit');
        $builder->add('repeatUntil', DateType::class, [
            'widget' => 'single_text',
            'input' => 'datetime_immutable',
            'model_timezone' => $timezone->getName(),
            'view_timezone' => $timezone->getName(),
            'html5' => true,
            'constraints' => [new NotBlank()],
            'help' => 'Repeats weekly, on the same day and time as the first occurrence above, through this date.',
        ]);
        $builder->add('submit', SubmitType::class, ['label' => (string) $options['submitLabel']]);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['submitLabel' => 'Create recurring events']);
    }
}
