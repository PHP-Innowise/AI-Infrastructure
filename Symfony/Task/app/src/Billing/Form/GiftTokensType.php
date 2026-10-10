<?php

declare(strict_types=1);

namespace App\Billing\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\IntegerType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TextareaType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\GreaterThan;
use Symfony\Component\Validator\Constraints\NotBlank;

/**
 * AC-05-33: "trainer selects player, enters a token amount, and adds a
 * note" — the note is REQUIRED, doubling as the audit trail's own reason
 * (matching BR-05-12's "reason required" for the Super-Admin `adjustment`
 * kind, applied here to gifting too since both are manual, unbacked-by-payment
 * ledger entries a reviewer must later be able to explain).
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class GiftTokensType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('amount', IntegerType::class, ['constraints' => [new GreaterThan(0)]])
            ->add('note', TextareaType::class, ['constraints' => [new NotBlank()]])
            ->add('submit', SubmitType::class, ['label' => 'Gift tokens']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
