<?php

declare(strict_types=1);

namespace App\Growth\Form;

use App\Growth\Entity\Coupon;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\CheckboxType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\DateTimeType;
use Symfony\Component\Form\Extension\Core\Type\IntegerType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * AC-06-17/18: coupon creation — code, discount type/value, applies-to,
 * usage limit, optional expiration, status.
 *
 * Also the same class the route table names for `growth_trainer_coupon_edit`
 * (`specs/api-designer-spec.md`), but AC-06-27 scopes editing to
 * expiration/usage limit/status only — reconciled here by disabling
 * code/discountType/discountValue/appliesTo/eligibility in `isEdit` mode
 * (a disabled Symfony Form field is never bound from submitted data, so the
 * original values are structurally preserved regardless of what the
 * rendered, grayed-out inputs show) rather than maintaining two form
 * classes for one screen shape.
 *
 * @see specs/api-designer-spec.md "Growth module" (`growth_trainer_coupon_create`/`_edit`)
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class CouponType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $immutableAfterCreate = true === $options['isEdit'];

        $builder
            ->add('code', TextType::class, [
                'label' => 'Coupon code',
                'disabled' => $immutableAfterCreate,
                'help' => '4-20 characters, letters, numbers, and hyphens. Case-sensitive.',
            ])
            ->add('discountType', ChoiceType::class, [
                'label' => 'Discount type',
                'disabled' => $immutableAfterCreate,
                'choices' => ['Percentage' => Coupon::DISCOUNT_PERCENTAGE, 'Fixed amount' => Coupon::DISCOUNT_FIXED],
            ])
            ->add('discountValue', IntegerType::class, [
                'label' => 'Discount value',
                'disabled' => $immutableAfterCreate,
                'help' => 'Percentage points (1-100) if Percentage, or cents if Fixed amount.',
            ])
            ->add('appliesTo', ChoiceType::class, [
                'label' => 'Applies to',
                'disabled' => $immutableAfterCreate,
                'choices' => [
                    'Events' => Coupon::APPLIES_TO_EVENTS,
                    'Content' => Coupon::APPLIES_TO_CONTENT,
                    'Both' => Coupon::APPLIES_TO_BOTH,
                ],
            ])
            ->add('usageLimit', IntegerType::class, [
                'label' => 'Usage limit',
                'required' => false,
                'help' => 'Leave blank for unlimited.',
            ])
            ->add('eligibility', ChoiceType::class, [
                'label' => 'Player eligibility',
                'disabled' => $immutableAfterCreate,
                'choices' => [
                    'Any player' => Coupon::ELIGIBILITY_ANY_PLAYER,
                    'New players only' => Coupon::ELIGIBILITY_NEW_PLAYERS_ONLY,
                ],
            ])
            ->add('expiresAt', DateTimeType::class, [
                'label' => 'Expiration date',
                'required' => false,
                'widget' => 'single_text',
                'input' => 'datetime_immutable',
            ])
            ->add('isActive', CheckboxType::class, ['label' => 'Active', 'required' => false])
            ->add('submit', SubmitType::class, ['label' => true === $options['isEdit'] ? 'Save changes' : 'Create coupon']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true, 'isEdit' => false]);
        $resolver->setAllowedTypes('isEdit', 'bool');
    }
}
