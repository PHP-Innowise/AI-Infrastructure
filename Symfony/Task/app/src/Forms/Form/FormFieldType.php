<?php

declare(strict_types=1);

namespace App\Forms\Form;

use App\Forms\Dto\FormField;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\CallbackTransformer;
use Symfony\Component\Form\Extension\Core\Type\CheckboxType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * One row of the field builder — BR-08-2's closed four-type vocabulary. Not
 * entity-mapped (`AbstractType<array<string, mixed>>`, matching
 * `CouponType`'s own array-backed precedent): the CONTROLLER converts each
 * submitted row into a `FormField` DTO via `FormField::fromArray()`, which
 * is where the real validation (id/label non-empty, options required for
 * dropdown/multiselect) actually lives — this type only shapes the HTTP
 * input.
 *
 * `id` is a plain visible input, not hidden-and-generated: "Basic form
 * builder (not drag-and-drop)... Simple field types only" (Out of MVP scope
 * — Simplifications) rules out inventing client-side slug generation for a
 * feature this epic explicitly keeps simple.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class FormFieldType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('id', TextType::class, ['label' => 'Field ID', 'help' => 'A short, unique identifier (letters, numbers, underscores).'])
            ->add('type', ChoiceType::class, [
                'label' => 'Type',
                'choices' => [
                    'Text' => FormField::TYPE_TEXT,
                    'Email' => FormField::TYPE_EMAIL,
                    'Dropdown' => FormField::TYPE_DROPDOWN,
                    'Multi-select' => FormField::TYPE_MULTISELECT,
                ],
            ])
            ->add('label', TextType::class, ['label' => 'Label'])
            ->add('required', CheckboxType::class, ['label' => 'Required', 'required' => false])
            ->add('options', TextType::class, [
                'label' => 'Options',
                'required' => false,
                'help' => 'Comma-separated — Dropdown/Multi-select only.',
            ]);

        // Model data is a ?list<string>; the HTTP field is one
        // comma-separated string. reverseTransform's argument is typed
        // ?string, not string: an unchecked, empty, not-required TextType
        // whose submitted value is genuinely absent from the request
        // reaches here as null, not '' — a real case, not merely
        // defensive.
        $builder->get('options')->addModelTransformer(new CallbackTransformer(
            static fn (?array $optionsList): string => null === $optionsList ? '' : implode(', ', $optionsList),
            static function (?string $raw): ?array {
                $trimmed = trim($raw ?? '');

                if ('' === $trimmed) {
                    return null;
                }

                $parts = array_map(trim(...), explode(',', $trimmed));

                return array_values(array_filter($parts, static fn (string $part): bool => '' !== $part));
            },
        ));
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => false]);
    }
}
