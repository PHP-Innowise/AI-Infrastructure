<?php

declare(strict_types=1);

namespace App\Identity\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\CheckboxType;
use Symfony\Component\Form\Extension\Core\Type\TimeType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * One weekday row: available on/off, plus a start/end time when on.
 * AC-01-43: "toggle each day available/not-available or set custom time
 * ranges."
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class DayAvailabilityType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('isAvailable', CheckboxType::class, ['required' => false])
            ->add('startTime', TimeType::class, ['required' => false, 'input' => 'datetime_immutable', 'widget' => 'single_text'])
            ->add('endTime', TimeType::class, ['required' => false, 'input' => 'datetime_immutable', 'widget' => 'single_text']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => false]);
    }
}
