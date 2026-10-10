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

/**
 * AC-08-9/10: name, description, optional price, and the same field
 * builder as `CampFormType` — no capacity field at all (BR-08-3).
 *
 * @see specs/api-designer-spec.md "Forms module" — `EvaluationFormType`
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class EvaluationFormType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('name', TextType::class, [
                'label' => 'Evaluation Name',
                'constraints' => [new NotBlank(), new Length(max: 255)],
            ])
            ->add('description', TextareaType::class, ['label' => 'Description', 'required' => false])
            ->add('priceMinorUnits', IntegerType::class, [
                'label' => 'Price (in cents)',
                'required' => false,
                'help' => sprintf(
                    'Leave blank or 0 for free, or %d-%d ($%d.00-$%d.00).',
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
            'submitLabel' => 'Save Evaluation',
        ]);
    }
}
