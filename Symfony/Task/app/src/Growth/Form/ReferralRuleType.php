<?php

declare(strict_types=1);

namespace App\Growth\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\CheckboxType;
use Symfony\Component\Form\Extension\Core\Type\IntegerType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\GreaterThan;

/**
 * AC-06-29: Super Admin edits the platform-wide referral reward rule —
 * "Referrals Required," "Tokens Awarded," and an optional "Reward Referee
 * Too" checkbox (Q-06.11's welcome-bonus toggle).
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class ReferralRuleType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('referralsRequired', IntegerType::class, [
                'label' => 'Referrals required',
                'constraints' => [new GreaterThan(0)],
            ])
            ->add('tokensAwarded', IntegerType::class, [
                'label' => 'Tokens awarded',
                'constraints' => [new GreaterThan(0)],
            ])
            ->add('refereeWelcomeBonus', CheckboxType::class, [
                'label' => 'Reward Referee Too (welcome token for the new player)',
                'required' => false,
            ])
            ->add('submit', SubmitType::class, ['label' => 'Save rule']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
