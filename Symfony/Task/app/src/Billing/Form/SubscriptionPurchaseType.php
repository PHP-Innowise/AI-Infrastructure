<?php

declare(strict_types=1);

namespace App\Billing\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\DateType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\GreaterThanOrEqual;
use Symfony\Component\Validator\Constraints\NotBlank;

/**
 * AC-05-29/BR-05-14: "the trainer can activate subscription from any future
 * date" — the purchaser (player/parent) picks the date at checkout.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class SubscriptionPurchaseType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('activationDate', DateType::class, [
                'widget' => 'single_text',
                'constraints' => [new NotBlank(), new GreaterThanOrEqual('today')],
            ])
            ->add('submit', SubmitType::class, ['label' => 'Subscribe']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
