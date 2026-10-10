<?php

declare(strict_types=1);

namespace App\Billing\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\IntegerType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\GreaterThan;
use Symfony\Component\Validator\Constraints\PositiveOrZero;

/**
 * BR-05-1, AC-05-1/2/32: the trainer's own $/token price, and (BR-05-14) the
 * Player Subscription price, if enabled. Amounts in minor units (cents).
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class TokenPricingType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('tokenPriceMinorUnits', IntegerType::class, ['label' => 'Price per token (cents)', 'constraints' => [new GreaterThan(0)]])
            ->add('playerSubscriptionPriceMinorUnits', IntegerType::class, [
                'label' => 'Player subscription price (cents) — leave blank to disable',
                'required' => false,
                'constraints' => [new PositiveOrZero()],
            ])
            ->add('submit', SubmitType::class, ['label' => 'Save pricing']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
