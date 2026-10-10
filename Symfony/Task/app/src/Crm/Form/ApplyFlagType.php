<?php

declare(strict_types=1);

namespace App\Crm\Form;

use App\Crm\Entity\PlayerFlag;
use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TextareaType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\NotBlank;

/**
 * AC-03-15: one of the 8 system-defined flags, with each choice paired with
 * its short definition (US-03.04's own "Acceptance Criteria - Apply Flag"
 * list), plus an optional explanatory note (BR-03-7: "optional though
 * recommended").
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class ApplyFlagType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('flagType', ChoiceType::class, [
                'choices' => array_flip(PlayerFlag::labelsWithDefinitions()),
                'constraints' => [new NotBlank()],
            ])
            ->add('note', TextareaType::class, ['required' => false])
            ->add('submit', SubmitType::class, ['label' => 'Add flag']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
