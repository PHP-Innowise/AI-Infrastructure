<?php

declare(strict_types=1);

namespace App\Identity\Form;

use App\Platform\Entity\Trainer;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;

/**
 * AC-01-23: either a manually-entered ShareLink code, or a pick from "My
 * Trainers". Both fields are optional here; the controller requires exactly
 * one to be filled.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class AddTrainerType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder->add('code', TextType::class, [
            'required' => false,
            'label' => 'Enter a ShareLink code',
        ]);

        if ([] !== $options['myTrainers']) {
            $builder->add('trainer', ChoiceType::class, [
                'required' => false,
                'label' => 'Or select from My Trainers',
                'choices' => $options['myTrainers'],
                'choice_value' => static fn (?Trainer $trainer): string => null === $trainer ? '' : (string) $trainer->getId(),
                'choice_label' => static fn (Trainer $trainer): string => $trainer->getBusinessName(),
            ]);
        }

        $builder->add('submit', SubmitType::class, ['label' => 'Add trainer']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
        $resolver->setRequired('myTrainers');
        $resolver->setAllowedTypes('myTrainers', 'array');
    }
}
