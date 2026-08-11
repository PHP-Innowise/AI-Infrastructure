<?php

declare(strict_types=1);

namespace App\Identity\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\EmailType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TelType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\NotBlank;
use Symfony\Component\Validator\Constraints\Regex;

/**
 * AC-01-1/2: business name, trainer name, email, and phone. AC-07-16 adds
 * `subscriptionTier`.
 *
 * **"Subscription tier" is not a concept this schema models anywhere** —
 * `specs/database-designer-schema.md` names exactly one platform-subscription
 * price knob, `trainer_billing_settings.monthly_subscription_price_minor_units`,
 * a single per-trainer rate with one platform-wide default (BR-05-13,
 * $15/month) and no catalog of named tiers. Rather than inventing an
 * unstated tier-catalog entity, this field is a small, fixed choice of
 * monthly price PRESETS for that one existing column — "subscription tier"
 * read as "which starting price," the only axis the settled schema actually
 * has. `TrainerCreationController` applies the chosen amount via the exact
 * same `TrainerBillingSettings::updatePricing()` Super Admin already uses
 * from `administration_trainer_fee_edit` (AC-05-27) — editable again
 * afterward the same way, tier or no tier. Recorded as a judgment call in
 * the coder's final report, not a silently invented requirement.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class CreateTrainerType extends AbstractType
{
    /**
     * BR-05-13's own $15/month platform default is kept as the "Standard"
     * choice, so a trainer created without touching this field still gets
     * exactly the pre-Epic-07 default rate.
     */
    public const TIER_STANDARD_MINOR_UNITS = 1500;
    public const TIER_GROWTH_MINOR_UNITS = 2500;
    public const TIER_PRO_MINOR_UNITS = 4000;

    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('businessName', TextType::class, ['constraints' => [new NotBlank()]])
            ->add('trainerFirstName', TextType::class, ['constraints' => [new NotBlank()]])
            ->add('trainerLastName', TextType::class, ['constraints' => [new NotBlank()]])
            ->add('email', EmailType::class, ['constraints' => [new NotBlank()]])
            ->add('phone', TelType::class, [
                'required' => false,
                // AC-01-50: phone number format validation.
                'constraints' => [new Regex(pattern: '/^[0-9()+\-.\s]{7,32}$/', message: 'Enter a valid phone number.')],
            ])
            ->add('subscriptionTier', ChoiceType::class, [
                'label' => 'Subscription tier',
                'choices' => [
                    'Standard — $15/month' => self::TIER_STANDARD_MINOR_UNITS,
                    'Growth — $25/month' => self::TIER_GROWTH_MINOR_UNITS,
                    'Pro — $40/month' => self::TIER_PRO_MINOR_UNITS,
                ],
                'data' => self::TIER_STANDARD_MINOR_UNITS,
            ])
            ->add('submit', SubmitType::class, ['label' => 'Create trainer account']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
