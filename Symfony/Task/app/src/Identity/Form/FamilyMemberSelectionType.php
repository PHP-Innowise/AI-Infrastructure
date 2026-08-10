<?php

declare(strict_types=1);

namespace App\Identity\Form;

use App\Identity\Entity\PlayerProfile;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\CheckboxType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * AC-01-14: "Who will train with [New Trainer]?" — the parent ("Me") and all
 * children, multi-select.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class FamilyMemberSelectionType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder->add('includeSelf', CheckboxType::class, [
            'required' => false,
            'label' => 'Me',
        ]);

        if ([] !== $options['children']) {
            $builder->add('children', ChoiceType::class, [
                'required' => false,
                'multiple' => true,
                'expanded' => true,
                'choices' => $options['children'],
                'choice_value' => static fn (?PlayerProfile $child): string => null === $child ? '' : (string) $child->getId(),
                'choice_label' => static fn (PlayerProfile $child): string => $child->getFirstName(),
            ]);
        }

        $builder->add('submit', SubmitType::class, ['label' => 'Continue']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
        $resolver->setRequired('children');
        $resolver->setAllowedTypes('children', 'array');
    }
}
