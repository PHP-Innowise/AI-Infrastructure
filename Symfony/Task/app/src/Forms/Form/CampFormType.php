<?php

declare(strict_types=1);

namespace App\Forms\Form;

use App\Forms\Entity\Form;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\CollectionType;
use Symfony\Component\Form\Extension\Core\Type\IntegerType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TextareaType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\Length;
use Symfony\Component\Validator\Constraints\NotBlank;
use Symfony\Component\Validator\Constraints\Range;

/**
 * AC-08-1..5: name, display-only dates folded into the description (the
 * epic names no dedicated date column — "display-only dates not linked to
 * the calendar"), capacity 1-1000, optional $0/$1-10,000 price, and the
 * field builder. Shared route for both create and edit
 * (`forms_trainer_camp_create`/`forms_trainer_form_edit`), matching
 * `CouponType`'s own precedent of one class covering both screens.
 *
 * @see specs/api-designer-spec.md "Forms module" — `CampFormType`
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class CampFormType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('name', TextType::class, [
                'label' => 'Camp Name',
                'constraints' => [new NotBlank(), new Length(max: 255)],
            ])
            ->add('description', TextareaType::class, [
                'label' => 'Description',
                'required' => false,
                'help' => 'Include the camp dates — this is display-only text, not linked to the calendar.',
            ])
            ->add('capacityLimit', IntegerType::class, [
                'label' => 'Capacity',
                'help' => sprintf('Maximum participants (%d-%d).', Form::MIN_CAPACITY, Form::MAX_CAPACITY),
                'constraints' => [new Range(min: Form::MIN_CAPACITY, max: Form::MAX_CAPACITY)],
            ])
            ->add('priceMinorUnits', IntegerType::class, [
                'label' => 'Price (in cents)',
                'required' => false,
                'help' => sprintf(
                    'Leave blank or 0 for a free camp, or %d-%d ($%d.00-$%d.00).',
                    Form::MIN_PRICE_MINOR_UNITS,
                    Form::MAX_PRICE_MINOR_UNITS,
                    intdiv(Form::MIN_PRICE_MINOR_UNITS, 100),
                    intdiv(Form::MAX_PRICE_MINOR_UNITS, 100),
                ),
            ])
            ->add('fields', CollectionType::class, [
                'label' => 'Registration fields',
                'entry_type' => FormFieldType::class,
                'allow_add' => true,
                'allow_delete' => true,
                'by_reference' => false,
                'prototype' => true,
            ])
            ->add('submit', SubmitType::class, ['label' => (string) $options['submitLabel']]);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults([
            'csrf_protection' => true,
            'submitLabel' => 'Save Camp',
        ]);
    }
}
