<?php

declare(strict_types=1);

namespace App\Identity\Form;

use Symfony\Component\Form\AbstractType;
use Symfony\Component\Form\Extension\Core\Type\ChoiceType;
use Symfony\Component\Form\Extension\Core\Type\DateType;
use Symfony\Component\Form\Extension\Core\Type\SubmitType;
use Symfony\Component\Form\Extension\Core\Type\TextType;
use Symfony\Component\Form\FormBuilderInterface;
use Symfony\Component\OptionsResolver\OptionsResolver;
use Symfony\Component\Validator\Constraints\NotBlank;

/**
 * AC-01-16/21: name, age, gender required (age enforced 1-18 in the
 * service — CURRENT_DATE-relative rules cannot live in a Form constraint
 * cleanly); school is optional.
 */
/**
 * @extends AbstractType<array<string, mixed>>
 */
final class ChildProfileType extends AbstractType
{
    public function buildForm(FormBuilderInterface $builder, array $options): void
    {
        $builder
            ->add('firstName', TextType::class, ['constraints' => [new NotBlank()]])
            ->add('dateOfBirth', DateType::class, [
                'widget' => 'single_text',
                'constraints' => [new NotBlank()],
            ])
            ->add('gender', ChoiceType::class, [
                'required' => false,
                'choices' => ['Female' => 'female', 'Male' => 'male', 'Prefer not to say' => 'unspecified'],
            ])
            ->add('schoolOrTeam', TextType::class, ['required' => false, 'label' => 'School (optional)'])
            ->add('submit', SubmitType::class, ['label' => 'Save child profile']);
    }

    public function configureOptions(OptionsResolver $resolver): void
    {
        $resolver->setDefaults(['csrf_protection' => true]);
    }
}
