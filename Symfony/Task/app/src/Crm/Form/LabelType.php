<?php

declare(strict_types=1);

namespace App\Crm\Form;

use App\Crm\Entity\Label;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\Length;
use Symfony\Component\Validator\Constraints\NotBlank;
use Symfony\Component\Validator\Constraints\Regex;

/**
 * AC-03-11: name (required, max 50 chars) and a color "from a color picker
 * with presets" — a closed preset list, not a free-text swatch, so a
 * submitted value is always one of these known-good hex codes.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class LabelType extends AbstractType
{
    public const PRESETS = [
        'Green' => '#00B300',
        'Blue' => '#0066CC',
        'Red' => '#CC0000',
        'Orange' => '#FF8800',
        'Purple' => '#8800CC',
        'Yellow' => '#CCAA00',
        'Teal' => '#008080',
        'Pink' => '#CC0066',
    ];

    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('name', TextType::class, [
                'constraints' => [new NotBlank(), new Length(max: Label::MAX_NAME_LENGTH)],
            ])
            ->add('colorHex', ChoiceType::class, [
                'choices' => self::PRESETS,
                'constraints' => [new NotBlank(), new Regex('/^#[0-9A-Fa-f]{6}$/')],
            ])
            ->add('submit', SubmitType::class, ['label' => 'Save label']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
