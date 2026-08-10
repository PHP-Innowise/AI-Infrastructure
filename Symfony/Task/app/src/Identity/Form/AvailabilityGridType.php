<?php

declare(strict_types=1);

namespace App\Identity\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * AC-01-43/44: a "Best Times" grid, one row per weekday, one range per day.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class AvailabilityGridType extends AbstractType
{
    /**
     * @var list<string>
     */
    public const DAY_LABELS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];

    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        foreach (self::DAY_LABELS as $day => $label) {
            $builder->add('day'.$day, DayAvailabilityType::class, ['label' => $label]);
        }

        $builder->add('submit', SubmitType::class, ['label' => 'Save availability']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
