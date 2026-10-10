<?php

declare(strict_types=1);

namespace App\Identity\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * AC-01-46: a recurring weekly schedule, "multiple time slots allowed per
 * day" — up to 3 fixed slot rows per weekday, all rendered upfront so the
 * form works with JavaScript disabled (no dynamic add/remove needed to get
 * more than one slot).
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class CoachAvailabilityType extends AbstractType
{
    public const SLOTS_PER_DAY = 3;

    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        foreach (AvailabilityGridType::DAY_LABELS as $day => $label) {
            for ($slot = 0; $slot < self::SLOTS_PER_DAY; ++$slot) {
                $builder->add(sprintf('day%d_slot%d', $day, $slot), DayAvailabilityType::class, [
                    'label' => 0 === $slot ? $label : $label.' (additional slot)',
                ]);
            }
        }

        $builder->add('submit', SubmitType::class, ['label' => 'Save my times']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
