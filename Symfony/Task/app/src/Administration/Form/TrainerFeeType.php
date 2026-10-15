<?php

declare(strict_types=1);

namespace App\Administration\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\IntegerType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TextareaType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\GreaterThan;
use Symfony\Component\Validator\Constraints\Range;

/**
 * AC-05-27/28: Super Admin edits a specific trainer's subscription price
 * and/or application fee rate — with an optional audit-log reason.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class TrainerFeeType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('monthlySubscriptionPriceMinorUnits', IntegerType::class, [
                'label' => 'Monthly subscription price (cents)',
                'constraints' => [new GreaterThan(0)],
            ])
            ->add('platformFeeBasisPoints', IntegerType::class, [
                'label' => 'Application fee (basis points, 500 = 5%)',
                'constraints' => [new Range(min: 0, max: 10000)],
            ])
            ->add('reason', TextareaType::class, ['required' => false, 'label' => 'Reason (optional)'])
            ->add('submit', SubmitType::class, ['label' => 'Save']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
